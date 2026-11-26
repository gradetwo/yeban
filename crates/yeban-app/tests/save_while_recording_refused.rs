//! T-FIX-738：**录音进行中的保存必须被拒绝**，不得静默成功、不得清零脏标记。
//!
//! ## 缺陷（实测形状）
//!
//! `wire_save` / `wire_save_with_engine` 此前没有 `recording_in_progress()` 检查；
//! 保存序列化的是**已登记**的工程与资产池，正在录的 take 还没 stop→land，不在其中。
//! 保存成功又把 `save-status` 写成"已保存"、脏标记清零 ⇒ 用户以为 take 已存，
//! 随后崩溃/退出即丢，而且退出提示因为"刚保存过"不再出现。
//!
//! ## 判据
//!
//! ① 起录成功后按保存 ⇒ `save-status` 必须是明确拒绝（含"录音"与"先停止录音"），
//! ② 目标文件**一个字节都没写**（没有被"保存成功"的假读数），
//! ③ 停录后同一路径再保存 ⇒ 成功写出文件（证明拒绝只针对录音窗口，不是把保存弄坏）。
//!
//! 红对照：删掉 `refuse_save_while_recording` 的调用（或让它恒 `false`），
//! ① 会红在"保存被拒绝"缺失、② 会红在文件已存在。

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::record_device::InputSourceFactory;
use yeban_app::record_wav::{InputSource, RecordOptions, RecordPlan};
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::EntityId;
use yeban_model::samples::filled_project;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW: u64 = 1_760_000_000_000;
const WINDOW: Size = Size::new(1440, 900);

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-738-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn options() -> RecordOptions {
    RecordOptions {
        seconds: 1.0,
        sample_rate: 48_000,
        channels: 2,
        capacity_frames: 8192,
        drain_batch: 512,
        ..RecordOptions::default()
    }
}

/// 脚本化非静音源（⛔ 不碰声卡）。
struct ToneSource {
    channels: u16,
    frame: u64,
}

impl InputSource for ToneSource {
    fn channels(&self) -> u16 {
        self.channels
    }

    fn next_block(&mut self, frames: usize, out: &mut [f32]) -> usize {
        let channels = usize::from(self.channels.max(1));
        for index in 0..frames {
            let value = ((self.frame + index as u64) % 32) as f32 / 64.0;
            for channel in 0..channels {
                out[index * channels + channel] = value;
            }
        }
        self.frame += frames as u64;
        frames
    }

    fn label(&self) -> &'static str {
        "t738-tone-source"
    }
}

struct ToneFactory;

impl InputSourceFactory for ToneFactory {
    fn open(&self, options: &RecordOptions) -> Result<Box<dyn InputSource + Send>, String> {
        Ok(Box::new(ToneSource {
            channels: options.channels.max(1),
            frame: 0,
        }))
    }

    fn label(&self) -> &'static str {
        "t738-tone-factory"
    }
}

#[test]
fn saving_while_recording_is_refused_and_writes_nothing() {
    let scratch = Scratch::new("refused");
    let save_path = scratch.dir.join("while-recording.yeban");
    let take_path = scratch.dir.join("take.wav");

    let project = filled_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();

    let port = Rc::new(UndoPort::new(
        UndoSession::open("t738", "yeban-app", project.clone(), NOW).expect("撤销会话"),
    ));

    // 录音这条腿：真的起录（线程本地 RECORDER_FOR_SWITCH 被装上）。
    host::wire_record_at(
        ui,
        Some(Rc::new(RefCell::new(project.clone()))),
        Some(Rc::clone(&port)),
        RecordPlan {
            path: take_path,
            options: options(),
        },
        Rc::new(ToneFactory),
        Rc::new(host::UiRecordOutcome),
        Rc::new(|| 0),
    );

    // 保存这条腿：给了目标路径（⇒ 不弹原生对话框），走产品同一条 dispatch_save。
    host::wire_save_with_engine(
        ui,
        host::SaveStatus {
            target: Some(save_path.clone()),
            project: Rc::clone(&port.project_shared().expect("端口有会话")),
        },
        Some(Rc::clone(&port)),
        Rc::new(yeban_app::file_picker::NoPicker),
        None,
        None,
    );

    ui.invoke_toggle_record();
    assert!(ui.get_recording(), "夹具前提：必须真的在录音");

    ui.invoke_save_project();

    let status = ui.get_save_status().to_string();
    assert!(
        status.contains("保存被拒绝") && status.contains("正在录音") && status.contains("停止录音"),
        "录音中保存必须响亮拒绝并说明请先停录，实际 status = {status:?}"
    );
    assert!(
        !save_path.exists(),
        "被拒绝的保存不得写盘（否则仍是『用户以为存了』的假读数）: {}",
        save_path.display()
    );

    // 停录后再保存：同一条路径必须真的写出文件 —— 证明拒绝只发生在录音窗口内。
    ui.invoke_toggle_record();
    assert!(!ui.get_recording(), "停录之后 UI 状态必须归零");
    std::thread::sleep(Duration::from_millis(120));

    ui.invoke_save_project();
    let status = ui.get_save_status().to_string();
    assert!(
        status.contains("已保存") && save_path.exists(),
        "停录后的保存必须照常成功，实际 status = {status:?}"
    );
}
