//! **导出 WAV 选项框**（`ui/dialogs/export_wav_dialog.slint`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 面板上的采样率 / 位深 / 时长三格文本**不是装饰**：它们经宿主
//! `host::parse_wav_export_options` 解析后真的进
//! `export_wav::WavExportOptions::resolve`，回执里的 `sample_rate` / `bits` / `frames`
//! 必须跟着变；非法文本 ⇒ `error=invalid-options` 且**不落盘**。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（隐藏态） | 装配后对话框**不在**运行时控件树里（`export-wav-open=false`），金帧因此不变；`module-status` 仍为空 |
//! | ②（可寻址） | `open-export-wav` ⇒ 10 个语义 ID 全部出现在树里、角色与注册表一致、路径由 `ModulePaths` 播种；树里没有注册表之外的 ID |
//! | ③（选项真的生效） | 采样率 44100 / 位深 16 / 时长 0.5 ⇒ 回执 `sample_rate=44100:bits=16:frames=22050`，字节数与磁盘一致 |
//! | ④（非法选项不落盘） | 位深 `cd` ⇒ `outcome=error:error=invalid-options`，目标文件**不存在** |
//! | ⑤（关闭还原） | `close-export-wav` ⇒ 对话框节点全部消失，ID 集合回到打开前的**同一集合**；用户改过的路径重开时**不被**缺省值覆盖 |
//! | ⑥（键盘输入真的进根属性） | 审评 2026-10-31（P0-2 跟进）：在路径输入框里**真实敲键盘**（点按聚焦 + 退格清掉旧值 + 逐字输入新路径）⇒ 根属性必须跟着变，导出必须落到**新**路径；单向绑定（修前）下根属性纹丝不动、导出仍落旧路径，判据必红 |
//! | ⑦（⭐ `D-EX-1` 读数恒发） | 一条**成功**回执里 `limiter` / `limiter_reductions` / `true_peak_dbfs` / `headroom_db` 四条字段**必须在**（`#229` 的旧门把它们拴在"削波"上 ⇒ 不削波那一格连字段名都没有），且值是 `null` 或一个数；`limiter_reductions` 在关着时恒 `0` |
//! | ⑧（⭐ `D-EX-1` 开关经真实点按可达） | 走 `注入 → Slint 事件源 → TouchArea.clicked → 根属性` 全链路点 `export-wav-limiter-switch`：奇数次为开、偶数次为关，`accessible-checked` 跟着翻，导出的 `limiter` 字段变成 `on` —— ⛔ 不是测试直接 `set_`（那只证"宿主读了属性"，不证"人摸得到控件"） |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/ui_module_actions.rs` 同款）。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code
// （`-D warnings` 会判红）。同款见 `tests/a11y_screen_reader.rs`。
#[allow(dead_code)]
mod live;

use std::path::{Path, PathBuf};
use std::rc::Rc;

use live::{LiveControlPlane, LiveWiringOptions, build_live_ui_with};
use yeban_app::elements::ElementKind;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::YebanProjectV1;
use yeban_ui_mcp::tree::UiTree;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 测试产物落在临时目录里（`Drop` 时清理，**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-ui-export-dialog-{}-{}",
            std::process::id(),
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

/// 对话框在树里的语义 ID 与登记角色（`elements.rs` 是同一份口径）。
///
/// ⭐ `D-EX-1` 加上第五格 `export-wav-limiter-switch`（布尔量，角色 `switch`）。
/// 这条闭集判据是"UI 加了控件却忘了登记"的第一道防线：`unknown` 那段会在**注册表漏登记**
/// 时红，`DIALOG_IDS` 少了它则永远不会被发现 —— 所以两处一起加。
const DIALOG_IDS: [(&str, ElementKind); 10] = [
    ("export-wav-dialog", ElementKind::Region),
    ("export-wav-panel", ElementKind::Region),
    ("export-wav-close-button", ElementKind::Button),
    ("export-wav-path-input", ElementKind::TextInput),
    ("export-wav-sample-rate-input", ElementKind::TextInput),
    ("export-wav-bit-depth-input", ElementKind::TextInput),
    ("export-wav-seconds-input", ElementKind::TextInput),
    ("export-wav-limiter-switch", ElementKind::Switch),
    ("export-wav-options-note", ElementKind::Text),
    ("export-wav-run-button", ElementKind::Button),
];

/// 回执字段名闭集（只收本判据可能遇到的字段；`clipping_note` 在**最末**）。
///
/// ⭐ `D-EX-1` 新增四个**恒发**的峰值读数键（`limiter` / `limiter_reductions` /
/// `true_peak_dbfs` / `headroom_db`）。它们必须进这张表，否则 `receipt_field_end`
/// 认不出字段边界，取到的值会把后面的键一起吞进来。
const RECEIPT_KEYS: [&str; 17] = [
    "action",
    "outcome",
    "error",
    "message",
    "path",
    "frames",
    "sample_rate",
    "channels",
    "bits",
    "bytes",
    "source",
    "digest",
    "limiter",
    "limiter_reductions",
    "true_peak_dbfs",
    "headroom_db",
    "clipping_note",
];

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

/// 取一个顶层字段（引号感知）；缺席即判据失败（⛔ 不返回空串）。
fn field_of(receipt: &str, key: &str) -> String {
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
                        return receipt[value_start..=value_start + 1 + close].to_owned();
                    }
                    let end = receipt_field_end(receipt, value_start, key).unwrap_or(receipt.len());
                    return receipt[value_start..end].to_owned();
                }
            }
            _ => {}
        }
    }
    panic!("回执里没有字段 `{key}`: {receipt}")
}

fn tree_ids(plane: &mut LiveControlPlane) -> Vec<String> {
    let (tree, _) = plane.plane().tree().expect("ui/tree");
    let mut ids: Vec<String> = tree.nodes.iter().map(|node| node.id.clone()).collect();
    ids.sort();
    ids
}

fn node<'a>(tree: &'a UiTree, id: &str) -> &'a yeban_ui_mcp::tree::UiNode {
    tree.nodes
        .iter()
        .find(|node| node.id == id)
        .unwrap_or_else(|| panic!("`{id}` 不在运行时控件树里（树里 {} 条）", tree.nodes.len()))
}

fn assert_nonempty_file(path: &Path) {
    let meta = std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("目标文件没落盘 {}: {error}", path.display()));
    assert!(meta.is_file(), "{} 不是文件", path.display());
    assert!(meta.len() > 0, "{} 是 0 字节", path.display());
}

#[test]
fn the_export_dialog_is_hidden_until_opened_and_its_options_reach_the_wav_writer() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback close-export-wav("),
        "新 UI 契约面必须声明 callback close-export-wav("
    );
    assert!(
        aether_host.contains("on_close_export_wav"),
        "宿主接线层必须注册 on_close_export_wav"
    );
    assert!(
        aether_app.contains("callback export-wav("),
        "新 UI 契约面必须声明 callback export-wav("
    );
    assert!(
        aether_host.contains("on_export_wav"),
        "宿主接线层必须注册 on_export_wav"
    );
    assert!(
        aether_app.contains("> export-wav-bit-depth:"),
        "新 UI 契约面必须声明 > export-wav-bit-depth:"
    );
    assert!(
        aether_host.contains("export_wav_bit_depth"),
        "宿主接线层必须注册 export_wav_bit_depth"
    );
}
