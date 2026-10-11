//! **四个模块入口：没有撤销会话时的那一支** —— 与
//! `tests/ui_module_actions.rs` 同一件事的另一个装配形态。
//!
//! ## 为什么单独一个测试文件
//!
//! `slint::platform::set_platform` 是**线程局部**的，而 `Tier1Window::install` 每线程只能
//! 成功一次。要驱动"没有 `UndoPort`"的装配，必须**再建一个活窗口** ⇒ 只能换一个进程
//! （每个 `tests/*.rs` 是一个独立的测试二进制）。
//!
//! ## 这条判据要证的句子
//!
//! `host::wire_module_actions` 的 `port: Option<Rc<UndoPort>>` 为 `None` 时：
//!
//! - **改工程**的那条（`import-midi`）**如实拒绝**，错误码 `no-undo-port`
//!   —— ⛔ 不静默开一个临时会话把编辑吞掉（那会让"可撤销"变成假话）；
//! - **读工程**的那条（`export-wav`）同样如实拒绝（错误码 `no-project`），
//!   ⛔ 不猜一份工程、也不写一个空文件；
//! - **不碰工程**的那两条（`load-sfz` / `record-wav`）照常可用 —— 于是"四条入口一起坏掉"
//!   这种假绿不可能出现。
//!
//! 全程**不 panic**（这就是错误路径的判据），并且失败时**不留半成品文件**。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的装配入口 ⇒ 其余项在本测试目标里是 dead_code
// （`-D warnings` 会判红）。同款见 `tests/a11y_screen_reader.rs`。
#[allow(dead_code)]
mod live;

use std::path::PathBuf;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_model::YebanProjectV1;
use yeban_ui_test_port::port::Permission;

/// 测试产物落在临时目录里（`Drop` 时清理，**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-ui-module-actions-no-port-{}-{}",
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

/// 一份最小但合法的 SFZ（一个区域）；`sample=` 只是路径字符串，不需要真实 wav。
const MINIMAL_SFZ: &str = "<region> sample=tone.wav lokey=0 hikey=127 pitch_keycenter=60\n";

/// 回执的字段名闭集（与 `tests/ui_module_actions.rs` 同一个口径；本文件只用到几个）。
const RECEIPT_KEYS: [&str; 28] = [
    "action",
    "outcome",
    "error",
    "message",
    "path",
    "notes",
    "track",
    "clip",
    "commit",
    "origin",
    "frames",
    "pushed",
    "dropped",
    "sample_rate",
    "channels",
    "bits",
    "bytes",
    "regions",
    "sample_source",
    "source",
    "wired_to_export",
    "probe",
    // ⭐ 硬件 MIDI 输入那一条自己的字段（`midi-in` 回执）。
    "port",
    "seconds",
    "events",
    "unsupported",
    "jitter",
    "first",
];

/// 见 `tests/ui_module_actions.rs` 的 `receipt_field_end` —— 值里带冒号（`probe=1:2:3:4`）
/// 与"按期望字段名划界"这两种错法都在那边有实测记录。
fn receipt_field_end(receipt: &str, at: usize, skip: &str) -> Option<usize> {
    let bytes = receipt.as_bytes();
    let mut index = at;
    while index < bytes.len() {
        if bytes[index] == b':' {
            for key in RECEIPT_KEYS {
                if key == skip {
                    continue;
                }
                let rest = &receipt[index + 1..];
                if rest.starts_with(key) && rest[key.len()..].starts_with('=') {
                    return Some(index);
                }
            }
        }
        index += 1;
    }
    None
}

/// 取一个字段；缺席即判据失败（⛔ 不返回空串）。
fn field_of(receipt: &str, key: &str) -> String {
    let needle = format!("{key}=");
    let start = receipt
        .find(&needle)
        .unwrap_or_else(|| panic!("回执里没有字段 `{key}`: {receipt}"))
        + needle.len();
    let end = receipt_field_end(receipt, start, key).unwrap_or(receipt.len());
    receipt[start..end].to_owned()
}

#[test]
fn without_an_undo_port_the_editing_entry_refuses_and_the_read_only_entries_still_work() {
    let scratch = Scratch::new();
    let sfz = scratch.dir.join("ui-instrument.sfz");
    let record = scratch.dir.join("ui-record.wav");
    let export = scratch.dir.join("ui-export.wav");
    std::fs::write(&sfz, MINIMAL_SFZ).expect("写最小 SFZ");

    // 同一条产品路径（`ModulePaths::from_env`）；`undo: None` 是唯一的差别。
    // SAFETY: 本测试二进制只有一个 `#[test]`（每个 `tests/*.rs` 是一个独立进程）
    // ⇒ 不存在第二个线程读这些变量；设置发生在本函数内部、早于任何装配。
    // Rust 2024 起 `set_var` 是 `unsafe fn`（"改环境变量会和 `getenv` 竞态"）。
    unsafe {
        std::env::set_var("YEBAN_UI_LOAD_SFZ_PATH", &sfz);
        std::env::set_var("YEBAN_UI_RECORD_WAV_PATH", &record);
        std::env::set_var("YEBAN_UI_EXPORT_WAV_PATH", &export);
        std::env::set_var("YEBAN_UI_RECORD_SECONDS", "0.5");
    }

    let project: YebanProjectV1 = yeban_model::samples::filled_project();
    let options = LiveWiringOptions {
        permission: Permission::Interactive,
        undo: None,
        ..LiveWiringOptions::default()
    };
    // 与 `tests/ui_module_actions.rs` 同款：本判据只需要活窗口
    // （`into_control_plane` 会消费 `LiveUi`，而这里用不到控制面）。
    let live_ui = build_live_ui_with(&project, &options).expect("Tier-1 平台 + 活窗口 + 四条接线");
    let ui = live_ui.ui();

    // ---- ① 改工程的那条：如实拒绝 ----
    ui.invoke_import_midi();
    let receipt = ui.get_module_status().to_string();
    assert!(
        !ui.get_module_status_succeeded(),
        "没有撤销会话时导入必须失败: {receipt}"
    );
    assert_eq!(field_of(&receipt, "action"), "import-midi", "{receipt}");
    assert_eq!(field_of(&receipt, "outcome"), "error", "{receipt}");
    assert_eq!(field_of(&receipt, "error"), "no-undo-port", "{receipt}");

    // ---- ② 读工程的那条：同样拒绝（不猜一份工程） ----
    ui.invoke_export_wav();
    let receipt = ui.get_module_status().to_string();
    assert!(
        !ui.get_module_status_succeeded(),
        "没有工程读数时导出必须失败: {receipt}"
    );
    assert_eq!(field_of(&receipt, "action"), "export-wav", "{receipt}");
    assert_eq!(field_of(&receipt, "outcome"), "error", "{receipt}");
    assert_eq!(field_of(&receipt, "error"), "no-project", "{receipt}");
    assert!(!export.exists(), "拒绝了却写出了文件: {}", export.display());

    // ---- ③ 不碰工程的那两条：照常可用（"一起坏掉"是假绿） ----
    ui.invoke_load_sfz();
    let receipt = ui.get_module_status().to_string();
    assert!(
        ui.get_module_status_succeeded(),
        "装载 SFZ 不该需要撤销会话: {receipt}"
    );
    assert_eq!(field_of(&receipt, "action"), "load-sfz", "{receipt}");
    assert_eq!(field_of(&receipt, "outcome"), "ok", "{receipt}");

    ui.invoke_record_wav();
    let receipt = ui.get_module_status().to_string();
    assert!(
        ui.get_module_status_succeeded(),
        "录音不该需要撤销会话: {receipt}"
    );
    assert_eq!(field_of(&receipt, "action"), "record-wav", "{receipt}");
    assert_eq!(field_of(&receipt, "outcome"), "ok", "{receipt}");
    assert!(
        std::fs::metadata(&record).expect("录音文件").len() > 0,
        "{} 是 0 字节",
        record.display()
    );

    // SAFETY: 同上 —— 单 `#[test]` 的进程，且此刻没有任何后台线程。
    unsafe {
        for key in [
            "YEBAN_UI_LOAD_SFZ_PATH",
            "YEBAN_UI_RECORD_WAV_PATH",
            "YEBAN_UI_EXPORT_WAV_PATH",
            "YEBAN_UI_RECORD_SECONDS",
        ] {
            std::env::remove_var(key);
        }
    }
}
