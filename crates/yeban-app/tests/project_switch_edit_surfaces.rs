//! T-FIX-371：换工程后模块入口的缺省路径必须跟着新工程目录走。
//!
//! ## 判据按生产顺序复现
//!
//! 启动期用一个 `Rc<UndoPort>` 接 `host::wire_edit_surfaces` 与四个模块入口；换工程时
//! 只调 `UndoPort::open_project_session` 原地换会话，随后按生产 `after` 跑
//! `wire_view_callbacks` 与 `set_module_project_dir`。本判据不主动重绑编辑面，顺带证明
//! 同一 `Rc` 原地换会话后四类编辑不会写丢；生产 `after` 仍保留 T-FIX-323 的编辑面重绑，
//! 本票不改那条逻辑。
//!
//! 随后依次触发设备机架旁通、宏释放、BPM 敲入、建 MIDI 轨：
//! 四类 Op 都必须落在**换会话之后**的工程上。
//!
//! 核心断言是模块路径：启动时路径指向目录 A，换工程后 `set_module_project_dir` 原地改到
//! 目录 B，导出 WAV / 导出 MIDI / 导入 MIDI / 装载 SFZ 四条回调必须各自读 B 下的
//! 缺省文件。导出 WAV 在工作线程异步执行、回执经事件循环送回，Tier-1 测试平台
//! 无事件循环代理，因此 WAV 以落盘文件为据（轮询 B 的文件出现且大小稳定；A 先出现即红）；
//! 其余三条同步回调的回执 `path=` 逐字对上。
//!
//! ## 红对照
//!
//! - 模块回调若仍按值捕获启动期 `ModulePaths`：换目录后回执仍报 A ⇒ 红。
//! - 若每次换工程都重跑 `wire_module_actions_for_project`：会多出一套 MIDI 实时线程；
//!   判据坚持只装配一次、只原地更新路径。

use std::path::PathBuf;
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::YebanProjectV1;
use yeban_model::project::TrackKind;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW_MS: u64 = 1_700_000_000_000;

fn live_window(project: &YebanProjectV1) -> LivePort<MainWindow> {
    let view = ViewState::from_project(project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口")
}

fn undo_port(project: &YebanProjectV1) -> Rc<UndoPort> {
    Rc::new(UndoPort::new(
        UndoSession::open("<判据 old>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ))
}

/// 展示轨里第一台带设备的身份（旁通判据用）。
fn first_device_id(
    project: &YebanProjectV1,
    track_id: yeban_model::EntityId,
) -> yeban_model::EntityId {
    project.tracks[&track_id]
        .devices
        .first()
        .map(|device| device.id)
        .expect("展示轨必须至少有一台设备")
}

/// 临时目录（`Drop` 时清理，绝不污染仓库）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-fix371-{tag}-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
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

/// 一份最小但合法的 SMF：一个 note_on（delta 0）+ note_off（delta 480），480 PPQ。
fn minimal_smf() -> Vec<u8> {
    let events: &[u8] = &[
        0x00, 0x90, 60, 100, // note_on
        0x83, 0x60, 0x80, 60, 0x00, // delta 480, note_off
        0x00, 0xFF, 0x2F, 0x00, // end of track
    ];
    let mut track = Vec::new();
    track.extend_from_slice(b"MTrk");
    track.extend_from_slice(&u32::try_from(events.len()).expect("小片段").to_be_bytes());
    track.extend_from_slice(events);

    let mut file = Vec::new();
    file.extend_from_slice(b"MThd");
    file.extend_from_slice(&6u32.to_be_bytes());
    file.extend_from_slice(&0u16.to_be_bytes());
    file.extend_from_slice(&1u16.to_be_bytes());
    file.extend_from_slice(&480u16.to_be_bytes());
    file.extend_from_slice(&track);
    file
}

/// 一份最小但合法的 SFZ（一个区域，覆盖全部音高；样本缺失时实现如实回退）。
const MINIMAL_SFZ: &str = "<region> sample=tone.wav lokey=0 hikey=127 pitch_keycenter=60\n";

#[test]
fn project_switch_module_paths_follow_the_new_project_dir() {
    let old_project = yeban_model::samples::filled_project();
    let new_project = yeban_model::samples::filled_project();
    let live = live_window(&old_project);
    let ui = live.ui();

    // 启动装配：**唯一**端口 + **唯一**模块路径状态（与 `main.rs` 生产装配同形）。
    let port = undo_port(&old_project);
    let dir_a = Scratch::new("a");
    let dir_b = Scratch::new("b");
    let module_paths = host::module_paths_for_project(Some(dir_a.dir.as_path()));
    host::wire_edit_surfaces(ui, &port);
    let _midi_state = host::wire_module_actions_for_project_shared(
        ui,
        Some(Rc::clone(&port)),
        Rc::clone(&module_paths),
    );

    // 换工程：同一个 `Rc<UndoPort>` 原地换会话；then 生产 `after` 的 `wire_view_callbacks`。
    // 本判据不主动重绑编辑面；生产的 ③-c 仍保留 T-FIX-323 那一跳。
    port.open_project_session("<判据 new>", new_project.clone(), Vec::new(), NOW_MS)
        .expect("换会话必须成功");
    host::wire_view_callbacks(ui);

    // 模块路径改到目录 B（生产 `after` 的 ③-d）。
    host::set_module_project_dir(&module_paths, Some(dir_b.dir.as_path()));

    // ① 设备机架旁通：必须落在换会话之后的工程。
    let device_track = port
        .try_project()
        .expect("新工程")
        .tracks
        .values()
        .find(|track| track.kind != TrackKind::Master && !track.devices.is_empty())
        .expect("新工程必须有一条带设备的展示轨")
        .id;
    let device_id = first_device_id(&port.try_project().expect("新工程"), device_track);
    ui.set_selected_track_id(device_track.to_string().into());
    ui.invoke_device_bypass_toggle(device_id.to_string().into());
    assert!(
        port.try_project().expect("新工程").tracks[&device_track].devices[0].bypassed,
        "换工程后设备旁通必须写进新会话"
    );

    // ② 宏释放：同一轨的宏 0 做一次真实 grab → drag → release。
    let macro_track = port
        .try_project()
        .expect("新工程")
        .tracks
        .values()
        .find(|track| track.kind != TrackKind::Master && !track.macros.is_empty())
        .expect("新工程必须有一条带宏的展示轨")
        .id;
    ui.set_selected_track_id(macro_track.to_string().into());
    let old_macro = port.try_project().expect("新工程").tracks[&macro_track].macros[0].value;
    ui.invoke_macro_grab(0, 0.0);
    ui.invoke_macro_drag(0, 1.0);
    ui.invoke_macro_release(0);
    assert_ne!(
        port.try_project().expect("新工程").tracks[&macro_track].macros[0].value,
        old_macro,
        "换工程后宏释放必须写进新会话"
    );

    // ③ BPM 敲入。
    let old_bpm = port.try_project().expect("新工程").bpm;
    ui.invoke_bpm_commit("140".into());
    assert!(
        (port.try_project().expect("新工程").bpm - 140.0).abs() < f64::EPSILON,
        "换工程后 BPM 敲入必须写进新会话"
    );
    assert_ne!(old_bpm, 140.0, "判据要求 BPM 真变化，而不是同值早退");

    // ④ 建 MIDI 轨。
    let before_tracks = port.try_project().expect("新工程").tracks.len();
    ui.invoke_track_create_midi();
    assert_eq!(
        port.try_project().expect("新工程").tracks.len(),
        before_tracks + 1,
        "换工程后建 MIDI 轨必须写进新会话"
    );

    // ---------------------------------------------------------------- 模块路径
    let dir_b_canonical = std::fs::canonicalize(&dir_b.dir).expect("规范目录 B");
    let expected_export = dir_b_canonical.join("ui-export.mid");
    let expected_export_wav = dir_b_canonical.join("ui-export.wav");
    let expected_import = dir_b_canonical.join("ui-import.mid");
    let expected_sfz = dir_b_canonical.join("ui-instrument.sfz");

    // 目录 B 里放好导入 MIDI 与 SFZ 的缺省文件。
    std::fs::write(&expected_import, minimal_smf()).expect("写判据 SMF");
    std::fs::write(&expected_sfz, MINIMAL_SFZ).expect("写判据 SFZ");

    // 导出 WAV：必须落到 B 的缺省路径，而不是启动时的 A。
    // 导出在工作线程异步执行，回执经事件循环送回；Tier-1 无事件循环代理，
    // 因此以落盘文件为据：轮询 B 的文件出现且大小稳定；A 先出现即判红。
    let dir_a_canonical = std::fs::canonicalize(&dir_a.dir).expect("规范目录 A");
    let startup_export_wav = dir_a_canonical.join("ui-export.wav");
    ui.invoke_export_wav();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    let mut last_size: Option<u64> = None;
    loop {
        assert!(
            !startup_export_wav.is_file(),
            "导出 WAV 落到了启动时的目录 A（{}），模块路径没有跟着新工程目录走",
            startup_export_wav.display()
        );
        if let Ok(meta) = std::fs::metadata(&expected_export_wav) {
            let size = meta.len();
            if size > 0 && last_size == Some(size) {
                break;
            }
            last_size = Some(size);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "导出 WAV 超时未落盘到 B（{}）",
            expected_export_wav.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(expected_export_wav.is_file(), "导出 WAV 必须真的落盘");

    // 导出 MIDI：必须落到 B 的缺省路径，而不是启动时的 A。
    ui.invoke_export_midi();
    let receipt = ui.get_module_status().to_string();
    assert!(
        ui.get_module_status_succeeded(),
        "导出 MIDI 必须成功: {receipt}"
    );
    assert!(
        receipt.contains(&format!("path={}", expected_export.display())),
        "导出 MIDI 必须按新工程目录解析（期望 {}）: {receipt}",
        expected_export.display()
    );
    assert!(expected_export.is_file(), "导出 MIDI 必须真的落盘");

    // 导入 MIDI：必须从 B 的缺省路径读。
    ui.invoke_import_midi();
    let receipt = ui.get_module_status().to_string();
    assert!(
        ui.get_module_status_succeeded(),
        "导入 MIDI 必须成功: {receipt}"
    );
    assert!(
        receipt.contains(&format!("path={}", expected_import.display())),
        "导入 MIDI 必须按新工程目录解析（期望 {}）: {receipt}",
        expected_import.display()
    );
    assert!(
        receipt.contains("notes=1"),
        "判据 SMF 必须导入 1 条音符: {receipt}"
    );

    // 装载 SFZ：必须从 B 的缺省路径读。
    ui.invoke_load_sfz();
    let receipt = ui.get_module_status().to_string();
    assert!(
        ui.get_module_status_succeeded(),
        "装载 SFZ 必须成功: {receipt}"
    );
    assert!(
        receipt.contains(&format!("path={}", expected_sfz.display())),
        "装载 SFZ 必须按新工程目录解析（期望 {}）: {receipt}",
        expected_sfz.display()
    );

    // 共享状态本身也必须是"改一处、所有回调看见"的那一份。
    assert_eq!(
        module_paths.borrow().export_wav,
        expected_export_wav,
        "共享 ModulePaths 的 export_wav 必须已改到目录 B"
    );
    assert_eq!(
        module_paths.borrow().export_midi,
        expected_export,
        "共享 ModulePaths 的 export_midi 必须已改到目录 B"
    );
    assert_eq!(
        module_paths.borrow().import_midi,
        expected_import,
        "共享 ModulePaths 的 import_midi 必须已改到目录 B"
    );
    assert_eq!(
        module_paths.borrow().load_sfz,
        expected_sfz,
        "共享 ModulePaths 的 load_sfz 必须已改到目录 B"
    );
}
