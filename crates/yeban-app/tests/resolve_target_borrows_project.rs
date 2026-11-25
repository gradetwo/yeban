//! ⭐⭐ **T-FIX-459 的判据：`resolve_target` 只借工程、不整工程克隆。**
//!
//! ## 它补的是哪一格
//!
//! 修前：`RecordControl::resolve_target` 每次起录／停录现解析目标轨时都整工程
//! 深拷贝一次 —— 活端口走 `port.try_project()`（ owned 快照），回退走
//! `shared.borrow().clone()`。本单之后：活端口走 `port.with_project` 借用，
//! 回退走装配期那份的 `shared.borrow()` 现借现算；两条路都不再整工程克隆。
//!
//! ## 为什么\"零克隆\"只能用源码级证据
//!
//! 对纯读而言，克隆与借用在**行为上完全相同**（读到的工程内容一位不差，
//! 解析出的 `RecordTarget` 也一位不差）—— 没有任何行为判据能区分\"借着读\"
//! 还是\"克隆了再读\"；性能／分配在判据里也测不稳。所以本单的\"零克隆\"钉在
//! 源码上：把 `resolve_target` 与 `resolve_target_from` 的函数体抽出来，断言
//! 它们经 `with_project`／`shared.borrow()` 取工程、且体内没有任何
//! `.clone()`／`try_project()`。
//! 红对照：把旧的两行克隆写回去 ⇒ 本判据立刻红（见收据）。
//!
//! ## 行为部分
//!
//! 源码钉不住行为，所以另有一条行为判据：没有活会话（`undo = None`）时回退
//! 路径仍能完整走完一次起录／停录（借用不 panic、不卡住，WAV 真落盘）。
//! 端口路径的行为（待录 ＞ 选中 ＞ 单轨；多条无选择 ⇒ 响亮拒绝）
//! 已由 `record_landing_start_tick_and_target.rs` 的 ②／②'／②''／②'''／②''''
//! 覆盖，本文件不再重复。

#![allow(unused)]
use std::rc::Rc;
use std::time::Duration;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::record_device::InputSourceFactory;
use yeban_app::record_wav::{InputSource, RecordOptions, RecordPlan};
use yeban_app::scene::DemoScene;
use yeban_model::YebanProjectV1;
use yeban_model::samples::filled_project;

const WINDOW: yeban_ui_test_port::Size = yeban_ui_test_port::Size::new(1440, 900);

/// 从 `src/host.rs` 里抽出指定函数的函数体（花括号配平）。
///
/// 体内没有含花括号的字面量／注释（以 `marker` 在场为准做窗口自检：
/// 窗口切错时自检先红，⛔ 不静默放过）。
fn function_body(name: &str, marker: &str) -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/host.rs");
    let text = std::fs::read_to_string(path).expect("读 host.rs");
    let lines: Vec<&str> = text.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.contains(&format!("fn {name}(")))
        .unwrap_or_else(|| panic!("host.rs 必须有 fn {name}"));
    let mut depth = 0i32;
    let mut seen_open = false;
    let mut end = start;
    for (index, line) in lines.iter().enumerate().skip(start) {
        for c in line.chars() {
            if c == '{' {
                depth += 1;
                seen_open = true;
            } else if c == '}' {
                depth -= 1;
            }
        }
        if seen_open && depth == 0 {
            end = index;
            break;
        }
    }
    assert!(
        end > start,
        "⭐ {name} 的函数体必须能配平抽出（起 {start} 行）"
    );
    let body = lines[start..=end].join("\n");
    assert!(
        body.contains(marker),
        "⭐ 抽出的 {name} 窗口必须含 {marker}，否则窗口切错了（起 {start} 行）"
    );
    body
}

/// ⭐ **验收 ②**：`resolve_target` 两条取工程的路都不整工程克隆。
#[test]
fn resolve_target_reads_the_project_by_borrow_not_by_clone() {
    let body = function_body("resolve_target", "resolve_target_from");
    assert!(
        body.contains("with_project"),
        "⭐ 活端口必须走 `with_project` 借用（体实测无该调用）：\n{body}"
    );
    assert!(
        body.contains("shared.borrow()"),
        "⭐ 回退路径必须走 `shared.borrow()` 现借现算（体实测无该调用）：\n{body}"
    );
    assert!(
        !body.contains(".clone()"),
        "⭐ resolve_target 体内 ⛔ 不许出现整工程克隆（实测含 .clone()）：\n{body}"
    );
    assert!(
        !body.contains("try_project"),
        "⭐ resolve_target 体内 ⛔ 不许再走 `try_project()` 快照（实测含该调用）：\n{body}"
    );
    // 纯算走独立函数 `resolve_target_from`：它同样只许借着读。
    let pure = function_body("resolve_target_from", "resolve_record_target");
    assert!(
        !pure.contains(".clone()"),
        "⭐ resolve_target_from 体内 ⛔ 不许出现整工程克隆（实测含 .clone()）：\n{pure}"
    );
    assert!(
        !pure.contains("try_project"),
        "⭐ resolve_target_from 体内 ⛔ 不许再走 `try_project()` 快照（实测含该调用）：\n{pure}"
    );
}

/// ⭐ **脚本化非静音源**：每帧一个非零阶梯值（⛔ 不碰声卡，⛔ 不是静音腿）。
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
        "criterion-tone-source"
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
        "criterion-tone-factory"
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

/// 判据的工程夹具：`filled_project()`（含**一条音频轨**）＋ 清空**全部**摆放。
fn fixture_project() -> YebanProjectV1 {
    let mut project = filled_project();
    project.transport.count_in_bars = 0;
    for track in project.tracks.values_mut() {
        track.clips.clear();
    }
    project
}

/// ⭐ **验收 ①**：没有活会话（`undo = None`）时，回退路径仍能完整走完一次
/// 起录／停录 —— 借用不 panic、不卡住，WAV 真落盘（`resolve_target` 在
/// `apply_started` 里被调到；有克隆／借用冲突时这里先炸）。
#[test]
fn fallback_path_without_a_live_session_still_starts_and_stops_a_take() {
    let dir = std::env::temp_dir().join(format!("yeban-459-fallback-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let path = dir.join("take.wav");
    let _ = std::fs::remove_file(&path);

    let project = fixture_project();
    let tier1 =
        yeban_ui_test_port::render::Tier1Window::install(WINDOW).expect("装 Tier-1 无头平台");
    let view = ViewState::from_project(&project).expect("装配时投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window(&view, &scene).expect("生产窗口构造入口");
    tier1
        .resize(WINDOW)
        .expect("窗口尺寸定下来之后再注入一次布局");
    host::wire_record_at(
        &ui,
        Some(Rc::new(std::cell::RefCell::new(project))),
        None,
        RecordPlan {
            path: path.clone(),
            options: options(),
        },
        Rc::new(ToneFactory),
        Rc::new(host::UiRecordOutcome),
        Rc::new(|| 0),
    );
    ui.invoke_toggle_record();
    assert!(ui.get_recording(), "⭐ 回退路径起录必须成功");
    std::thread::sleep(Duration::from_millis(80));
    ui.invoke_toggle_record();
    assert!(!ui.get_recording(), "⭐ 回退路径停录之后必须归零");
    let bytes = std::fs::metadata(&path)
        .expect("⭐ 回退路径停录后 WAV 必须落盘")
        .len();
    assert!(
        bytes > 0,
        "⭐ 回退路径停录后 WAV ⛔ 不能是空文件（实测 {bytes} 字节）"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
