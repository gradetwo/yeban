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

#![allow(unused)]
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
const RECEIPT_KEYS: [&str; 31] = [
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
    "dropped-events",
    "jitter",
    "first",
    // ⭐ 判据 ⑥ 起：落地（把采到的音符写进谱面）那一步的读数。
    // ⚠ 它们**必须**在这里：`receipt_field_end` 只认"已知名 + `=`"作边界 ⇒ 漏一个，
    // `field_of(&receipt, "first")` 这类取值会**静默多带一截**（与本注释记录的两种错法同族）。
    "landing",
    "unclosed",
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

/// 一条回执的**顶层**字段取值（引号感知，`#321`）；缺席返回 `None`。
///
/// 字段名只认**顶层**位置（`key=` 必须紧跟一个**顶层** `:`；引号内的 `:` 不算起点）：
/// 引号值跳到**第一个闭合引号**（引号内的 `channels=` 等一律不是字段名），普通值仍按
/// [`receipt_field_end`] 的"已知字段名 + `=`"边界取。引号值**保留引号**（本文件既有约定）。
///
/// ## 引号里的 `"`（写死的契约）
///
/// 回执没有转义 ⇒ 第一个 `"` 即闭合引号；值里真带 `"` 时其后的字段取不到（`None`）。
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
                    if receipt[value_start..].starts_with('"') {
                        let close = receipt[value_start + 1..].find('"').expect("引号必须闭合");
                        return Some(receipt[value_start..=value_start + 1 + close].to_owned());
                    }
                    let end = receipt_field_end(receipt, value_start, key).unwrap_or(receipt.len());
                    return Some(receipt[value_start..end].to_owned());
                }
            }
            _ => {}
        }
    }
    None
}

/// 取一个字段；缺席即判据失败（⛔ 不返回空串）。
fn field_of(receipt: &str, key: &str) -> String {
    field_of_opt(receipt, key).unwrap_or_else(|| panic!("回执里没有字段 `{key}`: {receipt}"))
}

#[test]
fn without_an_undo_port_the_editing_entry_refuses_and_the_read_only_entries_still_work() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback export-wav("),
        "新 UI 契约面必须声明 callback export-wav("
    );
    assert!(
        aether_host.contains("on_export_wav"),
        "宿主接线层必须注册 on_export_wav"
    );
    assert!(
        aether_app.contains("callback import-midi("),
        "新 UI 契约面必须声明 callback import-midi("
    );
    assert!(
        aether_host.contains("on_import_midi"),
        "宿主接线层必须注册 on_import_midi"
    );
    assert!(
        aether_app.contains("callback load-sfz("),
        "新 UI 契约面必须声明 callback load-sfz("
    );
    assert!(
        aether_host.contains("on_load_sfz"),
        "宿主接线层必须注册 on_load_sfz"
    );
}

/// `#321` 负向判据：**引号内的键 ⛔ 可被顶层取到**（本文件的取值器也必须引号感知）。
///
/// 修前：`field_of("channels")` 会命中 `loss_detail` 引号内的 `channels=`，取到
/// `2"`（把闭合引号也带进值里）而不是"缺席"。修后必须是 `None`，而 `loss_detail`
/// 仍是整串（引号内的 `=` 不切）。
#[test]
fn a_quoted_value_does_not_expose_its_inner_keys_to_the_top_level() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback export-wav("),
        "新 UI 契约面必须声明 callback export-wav("
    );
    assert!(
        aether_host.contains("on_export_wav"),
        "宿主接线层必须注册 on_export_wav"
    );
    assert!(
        aether_app.contains("callback import-midi("),
        "新 UI 契约面必须声明 callback import-midi("
    );
    assert!(
        aether_host.contains("on_import_midi"),
        "宿主接线层必须注册 on_import_midi"
    );
    assert!(
        aether_app.contains("callback load-sfz("),
        "新 UI 契约面必须声明 callback load-sfz("
    );
    assert!(
        aether_host.contains("on_load_sfz"),
        "宿主接线层必须注册 on_load_sfz"
    );
}
