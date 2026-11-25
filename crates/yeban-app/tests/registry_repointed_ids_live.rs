//! T-FIX-402：重指向的 3 条 A/B 键必须在活窗口里可寻址，26 条已删 id 必须不在树里。
//!
//! 背景：`elements.rs` 的注册表曾有 27 条 component 指着两个死文件
//! （`workspace/song_overview_panel.slint` 20 条、`module_actions.slint` 7 条，
//! 其中 2 条是 `SLINT_MANIFEST` 行）；两个文件从未被任何活 `.slint` import
//! （`docs/DEAD-SLINT-FILES.md` §1，无活树对应文件）。T-FIX-402 的裁决是
//! **重指向 3 + 删除 26**：
//! - 重指向：`song-overview-capture-a-button` / `-capture-b-button` /
//!   `-compare-button` —— 它们是活的，住在 `ui/app.slint` 的 A/B 对比条里
//!   （`if !root.console-maximized`，默认开启），标签与注册表逐字相同；
//! - 删除：其余 26 个 id（16 个 song-overview + 10 个 module-actions），
//!   在活树里根本不存在，没法诚实重指向。
//!
//! 本片钉的是**活窗口**那一侧（注册表自身那一侧由 `elements.rs` 里
//! `no_registry_entry_points_at_the_two_dead_files_fixed_by_t_fix_402` 钉）：
//! 3 个重指向 id 必须在运行时控件树里、标签是注册表原文在当前区域的译文；
//! 26 个已删 id 必须一个都不在树里
//! （与 T-FIX-356 重写的 `song_overview_panel.rs` 同款口径：死 id 不在树里才是对的）。
//!
//! 平台约束：`slint::platform::set_platform` 线程局部、每线程一次 ⇒ 每个 `#[test]`
//! 只建一个 `LivePort`（与 `tests/song_overview_threshold.rs` 同款）。

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::{Permission, Size, UiTestPort as _};

/// 生产窗口的唯一构造入口（`run_gui` 用的同一条；与
/// `tests/song_overview_threshold.rs` 同款）。
fn build(size: Size) -> LivePort<MainWindow> {
    let view = ViewState::demo();
    let scene = DemoScene::from_view(&view);
    LivePort::new(size, Permission::ReadOnly, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口")
}

/// 抓一帧再重读树：379 的教训 —— `set_console_tab` 换档后树要等一帧才同步；
/// A/B 条的 `if !root.console-maximized` 同理，先渲染再断言。
fn fresh_tree(port: &mut LivePort<MainWindow>) {
    port.window().capture().expect("必须能重绘一帧");
    port.refresh_tree(None).expect("必须能重抓树");
}

/// 3 个重指向 id 必须在树里；26 个已删 id 必须不在树里。
const REPOINTED: [(&str, &str); 3] = [
    ("song-overview-capture-a-button", "存 A"),
    ("song-overview-capture-b-button", "存 B"),
    ("song-overview-compare-button", "对比 A→B"),
];

const DELETED: [&str; 26] = [
    "song-overview-ab-status",
    "song-overview-panel",
    "song-overview-canvas",
    "song-overview-playhead",
    "song-overview-viewport",
    "song-overview-viewport-resize",
    "song-overview-hover-readout",
    "song-overview-density-strip",
    "song-overview-layer-tension-pill",
    "song-overview-layer-energy-pill",
    "song-overview-layer-bands-pill",
    "song-overview-layer-density-pill",
    "song-overview-collapse-button",
    "song-overview-fullscreen-button",
    "song-overview-profile-button",
    "song-overview-band-solo-outline",
    "module-export-wav-button",
    "module-import-midi-button",
    "module-load-sfz-button",
    "module-record-wav-button",
    "module-midi-in-button",
    "module-actions-title",
    "module-midi-in-status",
    "module-last-status",
    "module-clipping-warning",
    "module-import-losses",
];

fn assert_tree_shape(port: &mut LivePort<MainWindow>, label: &str) {
    // A/B 条的门：`console-maximized` 默认 false，否则三键根本不进树。
    assert!(
        !port.ui().get_console_maximized(),
        "[{label}] 新窗口 console-maximized 必须默认 false（否则 A/B 条不进树）"
    );
    fresh_tree(port);
    let tree = port.tree();
    for (id, expected_label) in REPOINTED {
        let node = tree
            .find_by_id(id)
            .unwrap_or_else(|| panic!("[{label}] 重指向的 `{id}` 必须在活窗口树里"));
        // 注册表存的是 @tr 的中文原文，运行时标签是它的译文 —— 默认区域（en_US）
        // 下逐字比中文原文必红（T-FIX-171 同款坑）；按当前区域取对应译文再比
        // （与 `tests/console_maximize.rs` 的同名 helper 同款口径）。
        assert_eq!(
            node.label,
            expected_label_for_current_locale(expected_label),
            "[{label}] `{id}` 的活树标签必须是注册表原文在当前区域的译文"
        );
        report_line(&format!(
            "[T-FIX-402 {label}] `{id}` 在树里 label={:?}",
            node.label
        ));
    }
    for id in DELETED {
        assert!(
            !tree.contains(id),
            "[{label}] 已删的 `{id}` 不许再出现在活窗口树里"
        );
    }
    report_line(&format!(
        "[T-FIX-402 {label}] 3 个重指向在树、26 个已删不在树"
    ));
}

#[test]
fn repointed_ab_buttons_are_addressable_at_1440x900() {
    let mut port = build(Size::new(1440, 900));
    assert_tree_shape(&mut port, "1440x900");
}

#[test]
fn repointed_ab_buttons_are_addressable_at_1920x1080() {
    let mut port = build(Size::new(1920, 1080));
    assert_tree_shape(&mut port, "1920x1080");
}

/// 注册表标签（`@tr` 的中文原文）在当前区域应有的样子。
///
/// 与 `tests/console_maximize.rs` 的同名 helper 同款口径：窗口的 `@tr` 绑定由
/// Slint 首帧按系统 locale 自动选词；`yeban_app::settings::detect_system_language`
/// 是那次自动选择的 Rust 侧同义复述，故它算出的语言就是窗口实际用的语言。
/// 源语言 zh-CN 没有词表目录，期望值就是原文；其余八语从
/// `translations/<lang>/LC_MESSAGES/yeban-app.po` 取 msgstr（解析口径与
/// `tests/i18n_coverage.rs` 的 `parse_po` 同款）。
fn expected_label_for_current_locale(registry_label: &str) -> String {
    let lang = yeban_app::settings::detect_system_language();
    if lang == yeban_app::cli::Language::ZhCn {
        return registry_label.to_string();
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "translations/{}/LC_MESSAGES/yeban-app.po",
        lang.name()
    ));
    let po = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读 {} 失败: {e}", path.display()));
    po_translation(&po, registry_label).unwrap_or_else(|| {
        panic!(
            "词表 {} 里找不到 msgid {registry_label:?} 的译文",
            path.display()
        )
    })
}

/// 从 .po 文本里取一条 msgid 的 msgstr（`msgid "..."` / `msgstr "..."` 加
/// `"..."` 续行；行首 `#` 注释与空行断开续行——与 `i18n_coverage.rs` 同款）。
fn po_translation(po: &str, wanted: &str) -> Option<String> {
    let mut current_id: Option<String> = None;
    let mut current_str: Option<String> = None;
    let mut mode: Option<&str> = None;
    for raw_line in po.lines() {
        let line = raw_line.trim();
        if let Some(rest) = line.strip_prefix("msgid ") {
            if current_id.as_deref() == Some(wanted) {
                return current_str;
            }
            current_id = Some(po_string_value(rest));
            current_str = None;
            mode = Some("id");
        } else if let Some(rest) = line.strip_prefix("msgstr ") {
            current_str = Some(po_string_value(rest));
            mode = Some("str");
        } else if line.starts_with('"') {
            let value = po_string_value(line);
            match mode {
                Some("id") => {
                    if let Some(id) = &mut current_id {
                        id.push_str(&value);
                    }
                }
                Some("str") => {
                    if let Some(s) = &mut current_str {
                        s.push_str(&value);
                    }
                }
                _ => {}
            }
        } else if line.is_empty() || line.starts_with('#') {
            mode = None;
        }
    }
    if current_id.as_deref() == Some(wanted) {
        return current_str;
    }
    None
}

/// 取一行 PO 字符串字面量（`"..."`）的解析值（与 `i18n_coverage.rs` 同款）。
fn po_string_value(text: &str) -> String {
    let text = text.trim();
    if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
        po_unescape(&text[1..text.len() - 1])
    } else {
        String::new()
    }
}

/// 还原 PO 转义序列（与 `i18n_coverage.rs` 的 `unescape` 同款）。
fn po_unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}
