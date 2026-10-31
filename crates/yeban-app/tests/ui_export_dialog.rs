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
//! | ②（可寻址） | `open-export-wav` ⇒ 9 个语义 ID 全部出现在树里、角色与注册表一致、路径由 `ModulePaths` 播种；树里没有注册表之外的 ID |
//! | ③（选项真的生效） | 采样率 44100 / 位深 16 / 时长 0.5 ⇒ 回执 `sample_rate=44100:bits=16:frames=22050`，字节数与磁盘一致 |
//! | ④（非法选项不落盘） | 位深 `cd` ⇒ `outcome=error:error=invalid-options`，目标文件**不存在** |
//! | ⑤（关闭还原） | `close-export-wav` ⇒ 对话框节点全部消失，ID 集合回到打开前的**同一集合**；用户改过的路径重开时**不被**缺省值覆盖 |
//! | ⑥（键盘输入真的进根属性） | 审评 2026-10-31（P0-2 跟进）：在路径输入框里**真实敲键盘**（点按聚焦 + 退格清掉旧值 + 逐字输入新路径）⇒ 根属性必须跟着变，导出必须落到**新**路径；单向绑定（修前）下根属性纹丝不动、导出仍落旧路径，判据必红 |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/ui_module_actions.rs` 同款）。

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

/// 对话框在树里的九个语义 ID 与登记角色（`elements.rs` 是同一份口径）。
const DIALOG_IDS: [(&str, ElementKind); 9] = [
    ("export-wav-dialog", ElementKind::Region),
    ("export-wav-panel", ElementKind::Region),
    ("export-wav-close-button", ElementKind::Button),
    ("export-wav-path-input", ElementKind::TextInput),
    ("export-wav-sample-rate-input", ElementKind::TextInput),
    ("export-wav-bit-depth-input", ElementKind::TextInput),
    ("export-wav-seconds-input", ElementKind::TextInput),
    ("export-wav-options-note", ElementKind::Text),
    ("export-wav-run-button", ElementKind::Button),
];

/// 回执字段名闭集（只收本判据可能遇到的字段；`clipping_note` 在 `digest` 之后）。
const RECEIPT_KEYS: [&str; 13] = [
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
    let scratch = Scratch::new();
    let export = scratch.join("ui-export-options.wav");
    let invalid = scratch.join("ui-export-invalid.wav");

    let project: YebanProjectV1 = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据: 导出选项框>", "yeban-app", project.clone(), 1)
            .expect("打开撤销会话"),
    ));

    // SAFETY: 本测试二进制只有一个 `#[test]` ⇒ 不存在第二个线程读这些变量；
    // 设置发生在本函数内部、早于任何装配与后台线程。
    unsafe {
        std::env::set_var("YEBAN_UI_EXPORT_WAV_PATH", &export);
    }

    let options = LiveWiringOptions {
        permission: Permission::Interactive,
        undo: Some(Rc::clone(&port)),
        ..LiveWiringOptions::default()
    };
    let live_ui = build_live_ui_with(&project, &options).expect("Tier-1 平台 + 活窗口 + 模块接线");
    let window = slint::ComponentHandle::clone_strong(live_ui.ui());
    let mut plane = live_ui.into_control_plane(Permission::Interactive);
    let ui = &window;

    // ---- ① 隐藏态 ----
    let before = tree_ids(&mut plane);
    assert!(
        !before.iter().any(|id| id == "export-wav-dialog"),
        "未打开时对话框不该在运行时控件树里（默认金帧因此不变）"
    );
    assert!(!ui.get_export_wav_open(), "初始必须是关闭态");
    assert_eq!(ui.get_module_status(), "", "刚装配的状态行必须是空的");

    // ---- ② 打开 ⇒ 九个 ID 可寻址，路径由 ModulePaths 播种 ----
    ui.invoke_open_export_wav();
    assert!(ui.get_export_wav_open(), "open-export-wav 必须打开对话框");
    assert_eq!(
        ui.get_export_wav_path().to_string(),
        export.display().to_string(),
        "空路径必须由 ModulePaths 播种为产品缺省"
    );
    let (tree, _) = plane.plane().tree().expect("ui/tree");
    for (id, kind) in DIALOG_IDS {
        let node = node(&tree, id);
        assert_eq!(
            node.role,
            kind.accessible_role(),
            "`{id}` 的角色必须与注册表一致"
        );
        assert!(
            !node.label.trim().is_empty(),
            "`{id}` 可见时必须有无障碍名称"
        );
        assert!(
            plane.registry().contains(id),
            "`{id}` 不在静态注册表里 —— UI 加了 ID 却忘了登记（[UI-TEST-001]）"
        );
    }
    let unknown: Vec<&str> = tree
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .filter(|id| !plane.registry().contains(id))
        .collect();
    assert!(
        unknown.is_empty(),
        "运行时树里出现了注册表没登记的 ID: {unknown:?}"
    );

    // ---- ③ 选项真的进写出器 ----
    ui.set_export_wav_sample_rate("44100".into());
    ui.set_export_wav_bit_depth("16".into());
    ui.set_export_wav_seconds("0.5".into());
    ui.invoke_export_wav();
    let receipt = ui.get_module_status().to_string();
    assert!(
        ui.get_module_status_succeeded(),
        "按当前选项导出应当成功: {receipt}"
    );
    assert_eq!(field_of(&receipt, "action"), "export-wav", "{receipt}");
    assert_eq!(field_of(&receipt, "outcome"), "ok", "{receipt}");
    assert_eq!(field_of(&receipt, "sample_rate"), "44100", "{receipt}");
    assert_eq!(field_of(&receipt, "bits"), "16", "{receipt}");
    assert_eq!(
        field_of(&receipt, "frames"),
        "22050",
        "0.5 秒 @ 44100 Hz（四舍五入）: {receipt}"
    );
    assert_nonempty_file(&export);
    assert_eq!(
        std::fs::metadata(&export)
            .expect("导出文件")
            .len()
            .to_string(),
        field_of(&receipt, "bytes"),
        "回执报的字节数必须等于磁盘上的字节数"
    );

    // ---- ④ 非法位深 ⇒ invalid-options 且不落盘 ----
    ui.set_export_wav_path(invalid.display().to_string().into());
    ui.set_export_wav_bit_depth("cd".into());
    ui.invoke_export_wav();
    let receipt = ui.get_module_status().to_string();
    assert!(
        !ui.get_module_status_succeeded(),
        "非法位深必须失败: {receipt}"
    );
    assert_eq!(field_of(&receipt, "outcome"), "error", "{receipt}");
    assert_eq!(field_of(&receipt, "error"), "invalid-options", "{receipt}");
    assert!(
        !invalid.exists(),
        "解析失败的导出不许留下文件: {}",
        invalid.display()
    );

    // ---- ⑤ 关闭还原 + 用户路径不被缺省覆盖 ----
    ui.set_export_wav_bit_depth("24".into());
    let kept = scratch.join("kept-by-user.wav");
    ui.set_export_wav_path(kept.display().to_string().into());
    ui.invoke_close_export_wav();
    assert!(!ui.get_export_wav_open(), "close-export-wav 必须关闭对话框");
    let closed = tree_ids(&mut plane);
    assert!(
        !closed.iter().any(|id| id == "export-wav-dialog"),
        "关闭之后对话框节点必须消失"
    );
    assert_eq!(closed, before, "关闭之后寻址面必须回到打开前的同一集合");
    ui.invoke_open_export_wav();
    assert_eq!(
        ui.get_export_wav_path().to_string(),
        kept.display().to_string(),
        "用户改过的路径重开时不许被缺省值覆盖"
    );

    // ---- ⑥ 键盘输入真的进根属性（审评 2026-10-31，P0-2 的跟进判据） ----
    //
    // 修前（单向 `text: root.x`）的失效形态：框里敲的字只改输入框自己的显示，
    // 根属性不动 ⇒ 导出仍落到播种/旧路径。修后（双向 `text <=> root.x`）：
    // 敲的字必须先落到根属性，导出才跟得上。
    //
    // 驱动方式：控制面 JSON-RPC（与 MCP `ui/dispatch_*` 同一条注入路）——
    // 点按路径输入框右缘（光标落到末尾）→ 退格清掉旧值 → 逐字敲新路径。
    // 采样率 / 位深 / 时长三格与路径格是同一处绑定改法（`export_wav_dialog.slint`
    // 四处同款），故只钉路径格这一处真实失效过的格子。
    let (tree, _) = plane.plane().tree().expect("ui/tree");
    let path_node = node(&tree, "export-wav-path-input");
    let bounds = path_node.bounds.expect("路径输入框必须有几何");
    let click_x = f64::from(bounds.width.saturating_sub(2));
    let click_y = f64::from(bounds.height) / 2.0;
    plane
        .plane()
        .call(
            yeban_ui_mcp::methods::METHOD_DISPATCH_POINTER_DOWN,
            Some(serde_json::json!({
                "elementId": "export-wav-path-input",
                "xOffset": click_x,
                "yOffset": click_y,
                "button": "left",
            })),
        )
        .expect("点按路径输入框必须送达");
    plane
        .plane()
        .call(
            yeban_ui_mcp::methods::METHOD_DISPATCH_POINTER_UP,
            Some(serde_json::json!({ "button": "left" })),
        )
        .expect("松开鼠标必须送达");
    // 旧值 = ⑤ 留下的 kept 路径（ASCII），逐字退格清掉。
    let old_len = kept.display().to_string().chars().count();
    for _ in 0..old_len {
        plane
            .plane()
            .call(
                yeban_ui_mcp::methods::METHOD_DISPATCH_KEY_PRESS,
                Some(serde_json::json!({ "keyCode": "Backspace" })),
            )
            .expect("退格必须送达");
    }
    let typed = scratch.join("typed-by-keyboard.wav");
    let typed_text = typed.display().to_string();
    for ch in typed_text.chars() {
        plane
            .plane()
            .call(
                yeban_ui_mcp::methods::METHOD_DISPATCH_KEY_PRESS,
                Some(serde_json::json!({ "keyCode": ch.to_string() })),
            )
            .unwrap_or_else(|error| panic!("输入 `{ch}` 必须送达: {error:?}"));
    }
    assert_eq!(
        ui.get_export_wav_path().to_string(),
        typed_text,
        "键盘敲进路径框的字必须先落到根属性（单向绑定下这里还是旧值）"
    );
    ui.invoke_export_wav();
    let receipt = ui.get_module_status().to_string();
    assert!(
        ui.get_module_status_succeeded(),
        "按键盘输入的路径导出应当成功: {receipt}"
    );
    assert_eq!(field_of(&receipt, "action"), "export-wav", "{receipt}");
    assert_eq!(field_of(&receipt, "outcome"), "ok", "{receipt}");
    assert_nonempty_file(&typed);
    assert!(
        !kept.exists(),
        "导出必须落到键盘输入的新路径，不许仍写到旧路径: {}",
        kept.display()
    );

    // SAFETY: 同上 —— 单 `#[test]` 的进程。
    unsafe {
        std::env::remove_var("YEBAN_UI_EXPORT_WAV_PATH");
    }
}
