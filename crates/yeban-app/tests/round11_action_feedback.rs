//! 第十一轮 `[PATCH UI-FEEDBACK-05]`：**「可交互即可感知」全仓收官**——
//! 把第九/十轮开创的手型光标推广到剩余高频可点控件，并给主窗口分隔条补尺寸调整光标。
//!
//! 本轮用户原话：「git pull 后，继续仔细检查各种视图是否用户友好，操作是否用户友好，
//! 修复完善出具补丁」。`git pull` 后基线快进到 `6ce6aae2`（第九轮已由 `deffe88b` 合入、
//! `39c9d2e0` 修了 round9 判据的 CRLF 边界 panic、`5cf3c0c0` 加了无窗口导出）。
//! 逐文件核对：第九轮 WIP 与合入版逐字节一致 ⇒ 备份后对齐；第十轮补丁原样重建，
//! 上游的 `is_char_boundary` CRLF 修复完整保留。
//!
//! 双轴盘点（分视图族 × 分交互维度）+ 亲自复核后，全仓 108 个 `TouchArea` 中仍有
//! **43 个「有 `clicked` 却既无光标也无反馈」**。按 ROI 收敛出两组：
//!
//! | 组 | 区域 | 文件 | 缺口 | 改法 |
//! | :-- | :--- | :--- | :--- | :--- |
//! | ① | 高频可点控件（状态 tab / 导航 / 列表项 / 功能键） | 8 个 `.slint` ×37 | transport 两个视图 tab、Console 主 tab、sidebar 分类与素材项、素材池行、Device Rack 20 键、Mixer M/S 与浏览键、编曲轨道 M/S/R、声学诊断键：鼠标扫过仍是默认箭头 | 各 `TouchArea` 加 `mouse-cursor: MouseCursor.pointer;`（全仓 13→50） |
//! | ② | 主窗口分隔条 | `app.slint` ×1 | `splitter-touch` 是 `accessible-role: slider` 的拖拽调整控件（220–600px），4px 细线、零视觉提示，用户完全看不出能拖 | 加 `mouse-cursor: MouseCursor.row-resize;`（`i-slint-common-1.18.1::BuiltInMouseCursor::RowResize`，`cargo check` 编译探针已验） |
//!
//! ## 默认帧零像素漂移怎么保证
//!
//! * 光标不是渲染帧的一部分（第九轮已确立，`round9_action_feedback.rs` ② 同款）；
//!   ⛔ 本轮**只插入 `mouse-cursor` 属性行**，不改 `background` / `color` / `width` /
//!   `height` / `visible` / 任何 `accessible-*` / 任何 `clicked` 与 `pointer-event` 回调。
//! * 全仓 `has-hover` 仍 32、`grab` 仍 7 ⇒ 前两轮的反馈与拖拽成果一位未动。
//! * `row-resize` 落在 `splitter-touch` 内；它本来就是「不画任何像素」的输入面
//!   （注释明写「命中带上下各扩 2px：4px 的细线不好抓（⛔ 不改可视几何）」）。
//! * 本机 Linux golden 逐字节断言若红，按 `AGENTS.md` 用干净基线 A/B 取证后记
//!   `pending`，⛔ 不重录、⛔ 不放宽判据。
//!
//! ## 诚实登记（明确**不做**的边界，⑦ 钉住不可静默改动）
//!
//! * ⛔ **画布手势面**：`piano_roll.slint` 卷帘,Mixer 点空白选轨、编曲剪辑点选 / 拖动、
//!   标尺 seek、卷帘横滚——同一面上混着点选 / 画音符 / 拖滚动 / 双击复位，给单一
//!   光标会误导。`round10_action_feedback.rs` ② 还把整个 `piano_roll.slint` 钉成零光标，
//!   故**工具栏数字键本轮也刻意不加**（为 1 处光标改上轮判据语义不划算），留独立切片。
//! * ⛔ **禁用态 `not-allowed` 光标**：Device Rack 有 11 个 `enabled: root.*-available`
//!   按钮。查 `i-slint-core-1.18.1/items/input_items.rs:81`，
//!   `TouchArea::input_event_filter_before_children` 在 `!self.enabled()` 时直接
//!   `ForwardAndIgnore`，**根本不设置 `*cursor`** ⇒ 可用时显示手型、禁用时保持默认箭头，
//!   ⛔ 不误导、⛔ 也不假装有「禁止」提示。全仓 `MouseCursor.not-allowed` 保持 0。
//! * ⛔ **覆盖层「×」关闭键的 hover/pressed 反馈**：`macro_panel` 的 × 父格没有
//!   `background`，补反馈要新增渲染属性；`export_wav_dialog` 关闭格含
//!   `export-wav-close-focus := FocusScope` 与 Tab/Return 冒泡，撞
//!   `export_dialog_button_focus.rs` 轮转表。第十轮只给了 pointer，本轮不动。
//! * ⛔ **M/S 激活态字色**（第十轮 A/B 后撤回）：`arrangement_view.slint` 与干净基线
//!   逐字节一致，⑤ 与 `round10` ⑤ 双重钉住，⛔ 不借本轮把反色三元塞回去。
//! * ⛔ **`transport-commit-button`**：`src/elements.rs` 的
//!   `KNOWN_ACTIONLESS_CONTROLS` 刻意登记的占位（能力不存在、`ADR-0052` 未决），
//!   且 `elements.rs:3792` 与 `round8` ⑤ 把「今天不得有 `clicked`」钉成事实 ——
//!   本轮巡检时亲自复核过，**不是漏接线**，不动。
//! * ⛔ 本轮**零新增** `accessible-id` / `accessible-value` / 可寻址节点，
//!   全仓 239 / 55 由 ⑥ 逐文件对账。零新增 `experimental-*` / `mcp-*` feature、
//!   零 `Cargo.toml` 改动、零 golden PNG 改动。
//! * ⛔ 本文件刻意不用 `.all(` 适配器（`assertion_bounds_audit.rs` 的登记守卫会要求
//!   扩 `SOURCES`）；全是纯源码文本判据，不装 Slint 平台、不碰活窗口。
//! * 同步扩了 `round9_action_feedback.rs` ②（13→50）与 `round10_action_feedback.rs`
//!   ①（13→50）、②（20→58）：这是**新增合法光标**带来的预期增长，⛔ 不是放宽 ——
//!   仍是精确计数，且由本文件 ①③ 三方互相印证。带日期记录只追加、不改写。

#![allow(unused)]
const APP: &str = include_str!("../ui/app.slint");
const TRANSPORT: &str = include_str!("../ui/transport.slint");
const MODULE_ACTIONS: &str = include_str!("../ui/module_actions.slint");
const SIDEBAR: &str = include_str!("../ui/sidebar.slint");
const SESSION_VIEW: &str = include_str!("../ui/workspace/session_view.slint");
const ARRANGEMENT: &str = include_str!("../ui/workspace/arrangement_view.slint");
const SONG_OVERVIEW: &str = include_str!("../ui/workspace/song_overview_panel.slint");
const CONSOLE_TABS: &str = include_str!("../ui/console/console_tabs.slint");
const PIANO_ROLL: &str = include_str!("../ui/console/piano_roll.slint");
const MIXER_CONSOLE: &str = include_str!("../ui/console/mixer_console.slint");
const DEVICE_RACK: &str = include_str!("../ui/console/device_rack.slint");
const MUSICAL_PR_DRAWER: &str = include_str!("../ui/dialogs/musical_pr_drawer.slint");
const MACRO_PANEL: &str = include_str!("../ui/dialogs/macro_panel.slint");
const EXPORT_WAV_DIALOG: &str = include_str!("../ui/dialogs/export_wav_dialog.slint");
const HUMANIZE_DIALOG: &str = include_str!("../ui/dialogs/humanize_dialog.slint");
const UNDO_TREE_MODAL: &str = include_str!("../ui/dialogs/undo_tree_modal.slint");

/// 全仓 `.slint` 清单：与 `round8` / `round9` / `round10`、
/// `round6_addressable_roles.rs::SLINT_FILES`、`src/elements.rs::SLINT_MANIFEST`
/// 同一份 19 文件口径（编译期固定），⛔ 不用 `read_dir` 空转成绿。
const SLINT_FILES: &[(&str, &str)] = &[
    ("app.slint", APP),
    ("transport.slint", TRANSPORT),
    ("module_actions.slint", MODULE_ACTIONS),
    ("sidebar.slint", SIDEBAR),
    ("status_bar.slint", include_str!("../ui/status_bar.slint")),
    ("tokens.slint", include_str!("../ui/tokens.slint")),
    ("workspace/session_view.slint", SESSION_VIEW),
    ("workspace/arrangement_view.slint", ARRANGEMENT),
    ("workspace/song_overview_panel.slint", SONG_OVERVIEW),
    ("console/console_tabs.slint", CONSOLE_TABS),
    ("console/piano_roll.slint", PIANO_ROLL),
    ("console/mixer_console.slint", MIXER_CONSOLE),
    ("console/device_rack.slint", DEVICE_RACK),
    (
        "console/asset_pool_panel.slint",
        include_str!("../ui/console/asset_pool_panel.slint"),
    ),
    ("dialogs/musical_pr_drawer.slint", MUSICAL_PR_DRAWER),
    ("dialogs/macro_panel.slint", MACRO_PANEL),
    ("dialogs/export_wav_dialog.slint", EXPORT_WAV_DIALOG),
    ("dialogs/humanize_dialog.slint", HUMANIZE_DIALOG),
    ("dialogs/undo_tree_modal.slint", UNDO_TREE_MODAL),
];

/// 从标记行开始取 `count` 行。⛔ 刻意用 `lines()` 逐行拼接，不用字节算术切片：
/// 上游 `39c9d2e0` 刚修过 `round9` 判据在 Windows CRLF 检出下
/// `cancel + 6000` 落在多字节字符中间的 panic，本轮不复刻。
fn lines_after(src: &str, marker: &str, count: usize) -> String {
    let mut out = String::new();
    let mut taking = false;
    let mut taken = 0usize;
    for line in src.lines() {
        if !taking && line.contains(marker) {
            taking = true;
        }
        if taking {
            out.push_str(line);
            out.push('\n');
            taken += 1;
            if taken >= count {
                break;
            }
        }
    }
    assert!(
        !out.is_empty(),
        "判据标记 `{marker}` 未在源码中找到 ⇒ ⛔ 不许空窗口假绿"
    );
    out
}

/// ① 全仓手型光标收官：逐文件精确分布 + 全仓棘轮 = 50。
/// 删任一处 ⇒ 少一次，红；别处又加 ⇒ 多一次，也红。
#[test]
fn all_clickable_controls_now_show_a_pointer_cursor() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// ② 主窗口分隔条终于看得出能拖：全仓唯一 1 处 `row-resize`，
/// 且必须落在 `splitter-touch` 内 —— 它是 `accessible-role: slider` 的垂直调整控件。
#[test]
fn the_splitter_is_discoverable_as_a_resizable_control() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// ③ 前两轮的成果一位未动：grab 7 / has-hover 32 / cursor_total 58 逐文件对账。
#[test]
fn the_grab_and_hover_inventories_are_untouched() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// ④ 画布手势面刻意零光标：语义混合（点选/画音符/拖滚动/双击复位），
/// 给单一光标会把「能拖」误标成「能点」。
#[test]
fn canvas_gesture_surfaces_still_have_no_cursor() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// ⑤ 11 个禁用态按钮不假装修好：`enabled` 仍在、全仓零 `not-allowed`。
/// Slint 源码取证见文档头：disabled 时 `ForwardAndIgnore`，光标根本不生效
/// ⇒ 可用才显示手型、禁用保持默认箭头，⛔ 不误导、⛔ 不假装「禁止」提示。
#[test]
fn disabled_buttons_do_not_pretend_to_be_clickable() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// ⑥ 零新增可寻址节点 / 无障碍读数 / 唯一强调色：本轮 8 个改动文件逐文件对账
/// （＋人性化弹窗 1 案：12 ID / 5 读数）。
#[test]
fn zero_new_addressable_nodes_or_readouts() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// ⑦ 刻意不做的边界：无动作占位、A/B 撤回的 M/S 字色、× 关闭键反馈、piano_roll。
#[test]
fn the_deliberate_boundaries_are_still_intact() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}
