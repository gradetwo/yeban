//! T-FIX-719：GUI「导入 MIDI 文件...」必须先问一次原生文件选择器。
//!
//! ## 本文件要证的三句话
//!
//! 1. **选了文件 ⇒ 选中的路径进导入**：`ScriptedPicker` 递一个 2 音符夹具，
//!    回执必须 `outcome=ok` + `notes=2` + `path=` **逐字等于**递出的路径
//!    （⛔ 不是缺省路径；缺省被故意指到一个**不存在**的路径 —— 旧代码读缺省
//!    会直接失败，红控的支点就在这里）；请求原文必须标题 `导入 MIDI 文件`、
//!    后缀过滤 `["mid", "midi"]`（与 `wire_import_audio` 同一条口径）。
//! 2. **取消 ⇒ 一位不动**：`ScriptedPicker::cancelling()`，对话框被问一次，
//!    回执原文、成功标志、工程指纹、可撤销步数**四项全不变**
//!    （⛔ 不写回执、不动工程 —— `file_picker` 模块契约第 2 条：
//!    "我不想"不是错误）。
//! 3. **无头缺省仍可用**：`NoPicker` + `import_midi` 指到一个真实 1 音符夹具
//!    ⇒ 导入成功，`notes=1` + `path=` 等于缺省路径（固定路径**只**作测试 /
//!    无头缺省；这条臂修前修后都绿，钉的是被允许的回落，不钉修复本身）。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是**线程局部**的，`Tier1Window::install` 每线程
//! 只能成功一次 ⇒ 本二进制只有一个 GUI `#[test]`，三条臂各起一个线程
//! （与 `tests/import_midi_losses_ui.rs` / `tests/export_menu_repro_167.rs` 同款）。
//!
//! ## 红控
//!
//! 把 `host.rs` 里 `on_import_midi` 的取径改回「直接读 `paths.import_midi`」
//! （不问选择器）⇒ 臂 1 红（对话框一次没问 + 缺省不存在 ⇒ `outcome=error`）、
//! 臂 2 红（回执被错误回执改写）；臂 3 仍绿（见上）。

mod common;

use std::path::{Path, PathBuf};
use std::rc::Rc;

use common::ScriptedPicker;
use yeban_app::bridge::ViewState;
use yeban_app::file_picker::{FilePicker, NoPicker};
use yeban_app::host;
use yeban_app::host::ModulePaths;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::YebanProjectV1;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW_MS: u64 = 1_700_000_000_000;

/// 测试产物落在这个临时目录里（`Drop` 时清理，**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-719-picker-{}-{}-{}",
            std::process::id(),
            tag,
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 最小合法 SMF（格式 0、960 PPQ、`notes` 个 C4 起步的单音符）。
///
/// 手写字节而不是借用导出器：本判据要独立于"导出"那条路（否则来源判据会随导出器一起漂移）。
/// 与 `src/import_midi.rs` 单元测试里的 `minimal_smf` 同款写法，集成判据不能直接用它。
fn minimal_smf(notes: u8) -> Vec<u8> {
    let mut bytes: Vec<u8> = Vec::new();
    bytes.extend_from_slice(b"MThd");
    bytes.extend_from_slice(&6u32.to_be_bytes());
    bytes.extend_from_slice(&0u16.to_be_bytes()); // 格式 0
    bytes.extend_from_slice(&1u16.to_be_bytes()); // 1 条轨
    bytes.extend_from_slice(&960u16.to_be_bytes()); // 960 PPQ
    let mut track: Vec<u8> = Vec::new();
    track.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20]); // tempo 500000
    for index in 0..notes {
        let pitch = 0x3C + index * 2; // C4、D4、E4……
        track.extend_from_slice(&[0x00, 0x90, pitch, 0x64]); // note on
        track.extend_from_slice(&[0x87, 0x40, 0x80, pitch, 0x40]); // delta 960: note off
    }
    track.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]); // end of track
    bytes.extend_from_slice(b"MTrk");
    bytes.extend_from_slice(&u32::try_from(track.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&track);
    bytes
}

fn write_fixture(dir: &Path, name: &str, notes: u8) -> PathBuf {
    let bytes = minimal_smf(notes);
    let path = dir.join(name);
    std::fs::write(&path, &bytes).expect("写 SMF 夹具");
    println!(
        "[719 fixture] path={} bytes={} notes={notes}",
        path.display(),
        bytes.len()
    );
    path
}

/// 一条回执的**顶层**字段取值（引号感知）；缺席返回 `None`。
///
/// 口径与 `tests/import_midi_losses_ui.rs` 的 `field_of_opt` 相同：回执语法是
/// `module:action=…:key=value:…`（`host.rs::module_receipt`）；字段名只认**顶层**位置：
/// `key=` 必须紧跟一个**顶层** `:`（引号内的 `:` 不算起点）。
fn field_of_opt(receipt: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    let mut in_quotes = false;
    for (index, byte) in receipt.bytes().enumerate() {
        match byte {
            b'"' => in_quotes = !in_quotes,
            b':' if !in_quotes => {
                let start = index + 1;
                if receipt[start..].starts_with(&needle) {
                    let value_start = start + needle.len();
                    let rest = &receipt[value_start..];
                    if let Some(quoted) = rest.strip_prefix('"') {
                        let end = quoted.find('"').expect("引号必须闭合");
                        return Some(quoted[..end].to_owned());
                    }
                    let end = rest.find(':').unwrap_or(rest.len());
                    return Some(rest[..end].to_owned());
                }
            }
            _ => {}
        }
    }
    None
}

fn field_of(receipt: &str, key: &str) -> String {
    field_of_opt(receipt, key).unwrap_or_else(|| panic!("回执里没有字段 `{key}`: {receipt}"))
}

/// 起一个活窗口 + 只接模块入口（`invoke_import_midi` 直接触发 `on_import_midi`
/// 回调，不经过 `wire_aether_actions` 的命令分派 —— 与
/// `tests/import_midi_losses_ui.rs` 同款；本文件测的是模块入口那条回调本身）。
fn build_window(
    project: &YebanProjectV1,
    port: Rc<UndoPort>,
    paths: ModulePaths,
    picker: Rc<dyn FilePicker>,
) -> LivePort<yeban_app::ui::MainWindow> {
    let view = ViewState::from_project(project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    host::wire_module_actions_with(live.ui(), Some(Rc::clone(&port)), paths, picker);
    live
}

fn open_port(project: &YebanProjectV1, tag: &str) -> Rc<UndoPort> {
    Rc::new(UndoPort::new(
        UndoSession::open(tag, "yeban-app", project.clone(), NOW_MS).expect("撤销会话必须能打开"),
    ))
}

#[test]
fn import_midi_goes_through_the_file_picker() {
    picker_chosen_path_arm();
    // ⭐ 其余两臂各起一个线程（平台是线程局部的，每线程只能装一次）。
    let handle = std::thread::spawn(cancel_arm);
    handle.join().expect("取消臂的线程必须正常结束");
    let handle = std::thread::spawn(nopicker_fallback_arm);
    handle.join().expect("无头回落臂的线程必须正常结束");
}

/// 臂 1：选了文件 ⇒ 选中的路径进导入（2 音符夹具；缺省故意不存在）。
fn picker_chosen_path_arm() {
    let scratch = Scratch::new("chosen");
    let chosen = write_fixture(&scratch.dir, "chosen-two-notes.mid", 2);
    // ⛔ 故意不存在：旧代码读缺省会直接失败（红控的支点）。
    let unread_default = scratch.join("must-not-be-read.mid");
    assert!(!unread_default.exists(), "缺省路径必须不存在");

    let project: YebanProjectV1 = yeban_model::samples::filled_project();
    let port = open_port(&project, "<判据:719 选中的路径>");
    let fingerprint_before = port.fingerprint();
    let picker = ScriptedPicker::answering_open(chosen.clone());
    let paths = ModulePaths {
        import_midi: unread_default,
        ..ModulePaths::default()
    };
    let live = build_window(
        &project,
        Rc::clone(&port),
        paths,
        Rc::clone(&picker) as Rc<dyn FilePicker>,
    );
    let window = live.ui();

    window.invoke_import_midi();

    // ① 对话框真的被问了一次，且请求原文就是"导入 MIDI 文件" + mid/midi 过滤。
    assert_eq!(picker.call_count("open"), 1, "导入必须先问一次文件选择器");
    let request = picker.last_request("open").expect("必须记下这次 open 请求");
    assert_eq!(
        request.title, "导入 MIDI 文件",
        "对话框标题必须与菜单项一致"
    );
    assert_eq!(
        request.extensions,
        vec!["mid".to_owned(), "midi".to_owned()],
        "后缀过滤必须是 mid + midi"
    );

    // ② 回执证明**选中的**路径进了导入（不是缺省）：notes=2 是夹具的音符数。
    let succeeded = window.get_module_status_succeeded();
    let receipt = window.get_module_status().to_string();
    println!("[719 chosen] succeeded={succeeded} module-status={receipt}");
    assert!(succeeded, "GUI 导入必须成功: {receipt}");
    assert_eq!(field_of(&receipt, "action"), "import-midi", "{receipt}");
    assert_eq!(field_of(&receipt, "outcome"), "ok", "{receipt}");
    assert_eq!(field_of(&receipt, "notes"), "2", "{receipt}");
    assert_eq!(
        field_of(&receipt, "path"),
        chosen.display().to_string(),
        "回执 path 必须是选中的路径: {receipt}"
    );

    // ③ 导入真的落进了工程（指纹变了 —— 不只是回执写得好看）。
    let fingerprint_after = port.fingerprint();
    assert_ne!(
        fingerprint_after, fingerprint_before,
        "导入提交后工程指纹必须变化"
    );
}

/// 臂 2：取消 ⇒ 一位不动（对话框问一次，回执 / 成功标志 / 指纹 / 可撤销步数全不变）。
fn cancel_arm() {
    let project: YebanProjectV1 = yeban_model::samples::filled_project();
    let port = open_port(&project, "<判据:719 取消>");
    let picker = ScriptedPicker::cancelling();
    let live = build_window(
        &project,
        Rc::clone(&port),
        ModulePaths::default(),
        Rc::clone(&picker) as Rc<dyn FilePicker>,
    );
    let window = live.ui();

    let status_before = window.get_module_status().to_string();
    let succeeded_before = window.get_module_status_succeeded();
    let fingerprint_before = port.fingerprint();
    let undoable_before = port.display().undoable;

    window.invoke_import_midi();

    assert_eq!(picker.call_count("open"), 1, "取消臂也必须先问一次选择器");
    assert_eq!(
        window.get_module_status().to_string(),
        status_before,
        "取消后回执原文必须一位不动"
    );
    assert_eq!(
        window.get_module_status_succeeded(),
        succeeded_before,
        "取消后成功标志必须一位不动"
    );
    assert_eq!(
        port.fingerprint(),
        fingerprint_before,
        "取消后工程指纹必须一位不动"
    );
    assert_eq!(
        port.display().undoable,
        undoable_before,
        "取消后可撤销步数必须一位不动"
    );
    println!("[719 cancel] status unchanged; fingerprint unchanged; undoable={undoable_before}");
}

/// 臂 3：`NoPicker`（无头 / 装配期已定路径）⇒ 回落到缺省路径，导入照常成功。
///
/// 这条臂修前修后都绿 —— 它钉的是被允许的回落（固定路径只作测试 / 无头缺省），
/// 不钉修复本身。
fn nopicker_fallback_arm() {
    let scratch = Scratch::new("fallback");
    let fallback = write_fixture(&scratch.dir, "fallback-one-note.mid", 1);

    let project: YebanProjectV1 = yeban_model::samples::filled_project();
    let port = open_port(&project, "<判据:719 无头回落>");
    let paths = ModulePaths {
        import_midi: fallback.clone(),
        ..ModulePaths::default()
    };
    let live = build_window(&project, Rc::clone(&port), paths, Rc::new(NoPicker));
    let window = live.ui();

    window.invoke_import_midi();

    let succeeded = window.get_module_status_succeeded();
    let receipt = window.get_module_status().to_string();
    println!("[719 fallback] succeeded={succeeded} module-status={receipt}");
    assert!(succeeded, "无头缺省导入必须成功: {receipt}");
    assert_eq!(field_of(&receipt, "action"), "import-midi", "{receipt}");
    assert_eq!(field_of(&receipt, "outcome"), "ok", "{receipt}");
    assert_eq!(field_of(&receipt, "notes"), "1", "{receipt}");
    assert_eq!(
        field_of(&receipt, "path"),
        fallback.display().to_string(),
        "无头回落的回执 path 必须是缺省路径: {receipt}"
    );
}
