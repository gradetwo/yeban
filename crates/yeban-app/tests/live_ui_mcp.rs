//! **真实界面上的 UI 控制面** —— 端到端判据（`cargo test -p yeban-app --all-targets` 会跑它）。
//!
//! 这一条工作线要证明的事只有一句：**AI 能在活的 `MainWindow` 上看见界面**。
//! 本文件把这句话拆成可失败的判据：
//!
//! | 判据 | 证明 |
//! | :--- | :--- |
//! | `live_control_plane_reads_the_project_backed_window_end_to_end` | 建立真实执行面 → `ui/tree` → `ui/node`（按**语义 ID**）→ `ui/screenshot` 四步全通；节点标签里是**工程**（`filled_project()`）的轨道名；截图尺寸非零、非全黑，且与"直接从窗口抓的那一帧"**逐字节同一份** |
//! | `live_control_plane_coverage_matches_the_projected_registry` | 运行时树 ⊆ 投影注册表（`visible: false` 的分支不在树里 ⇒ 缺失非空），两个方向的差异都被 `ui/coverage` 如实报出来 |
//! | `production_plane_hard_denies_injection_on_the_live_window` | §12.3 / `ARCH-SEC-002`：生产模式下 `ui:inject` 在**真实执行面**上也是 403 硬禁（并且之后的只读调用照常可用） |
//! | `interactive_test_mode_plane_injects_into_the_live_window` | §12.4：测试模式 + `Interactive` 权限下，事件注入真的落进窗口；`ui/methods` 如实报出 `injectAllowed` |
//! | `a_tree_read_sees_a_click_that_just_happened` | **新鲜度**：一次真实点击（`toggle-sidebar`）之后，`ui/tree` / `ui/node` / `ui/property` 三条读同时看见它（节点 **85→81**、`x` **210→6**、宽度 `"240.00"→"36.00"`），再点一次全部回去 |
//! | `a_tree_regrab_costs_one_introspection_not_one_full_render` | **代价**：一次重抓 = 一次全树内省（85 节点），不是一次 1920×1080 的 Tier-1 全量渲染（两者并排打出来） |
//! | `escape_closes_the_time_machine_modal_for_real` | **`Escape` 真的关得掉全屏时光机**（`[UI-A11Y-003]` §7.3 与 `.slint` 的 "Esc 关闭" 那句）：端口读数 `true→false`、运行时树里 `undo-tree-modal` 消失、最后一条动作记录是 `close-undo-tree`，而工程与撤销栈一位未动 |
//!
//! ## 为什么判据住在两个文件里而不是这里
//!
//! 判据的**逻辑**住在 `crates/yeban-app/src/live_surface.rs`（接线）与
//! `crates/yeban-ui-mcp/src/live.rs`（`ControlPlane::probe`：树 → 节点 → 截图那条链）。
//! 本文件只做"装配 + 断言 + 留痕"，因此同一段 `probe` 逻辑在**本机**（假执行面，零 Slint）
//! 与 **CI**（真实 `MainWindow`）上跑的是同一份代码 —— 差别只有执行面。
//!
//! ## 依赖从哪来（红线 6 没有被削弱）
//!
//! `yeban-ui-mcp` / `yeban-ui-test-port` 都是 `[dev-dependencies]`：**不进入** release 构建。
//! 核验命令（零编译）：`cargo tree -p yeban-app -e normal --locked`。
//!
//! ## 每个 `#[test]` 只装一次 Tier-1 平台
//!
//! `slint::platform::set_platform` 是**线程局部**的（每个测试一个线程，libtest 默认如此），
//! 因此每个测试各建一个 `LiveUi`；同一个测试里**不能**建第二个。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
mod live;

use live::{
    LiveControlPlane, LiveWiringError, LiveWiringOptions, build_live_ui, build_live_ui_with,
};

// 判据 17（`ROAD-M4-008` 选项 (a)）：以进程内控制面会话为**唯一可变权威**的装配入口
// 与刷新点。它们只存在于非默认 feature `in-process-mcp` 下 —— 默认构建里没有
// `yeban-mcp`（`tree -p yeban-app -e normal` 命中 0），因此这里也必须 cfg。
#[cfg(feature = "in-process-mcp")]
use live::{AuthoritySync, build_live_ui_from_authority};

// 判据 17 的**同案差分像素**（`ADR-0005` S1）与横向缩放的**像素 A/B**：
// 帧差分读数在默认构建里也被用到（`ui_zoom_shortcuts_...`），因此 `Rect` / `Rgb8Image`
// 与 [`frame_diff`] **不再** cfg —— 只被 non-default feature 用到的那几个助手
// （`diff_pixels_outside` / `projected_lane_stack_box` / 容差常量）继续 cfg，否则默认构建
// 里它们是 dead_code（`-D warnings`）。
use yeban_ui_test_port::{Rect, Rgb8Image};

use yeban_app::elements::is_model_driven_family;
use yeban_app::scene::TRACK_NAMES;
use yeban_ui_mcp::live::{ProbeOptions, family_member_count};
use yeban_ui_mcp::surface::{DEFAULT_MAX_PNG_BYTES, encode_with_evidence};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 装配一条真实控制面（所有判据的唯一入口）。
fn assemble(permission: Permission) -> Result<LiveControlPlane, LiveWiringError> {
    let project = yeban_model::samples::filled_project();
    Ok(build_live_ui(&project, permission)?.into_control_plane(permission))
}

/// 把过程产物写进 `target/ui-test-port/`（`.gitignore` 已忽略；由 CI 的 artifact 步骤上传）。
fn write_artifact_bytes(name: &str, bytes: &[u8]) -> String {
    let dir = yeban_ui_test_port::artifact_dir();
    if let Err(error) = std::fs::create_dir_all(&dir) {
        report_line(&format!("artifact 目录创建失败(不影响判据): {error}"));
        return String::new();
    }
    let path = dir.join(name);
    match std::fs::write(&path, bytes) {
        Ok(()) => path.display().to_string(),
        Err(error) => {
            report_line(&format!("artifact 写入失败(不影响判据): {error}"));
            String::new()
        }
    }
}

/// 判据 1（**本工作线的核心**）：真实窗口上的四步端到端读数，且数据来自工程。
#[test]
fn live_control_plane_reads_the_project_backed_window_end_to_end() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

/// 判据 2：运行时树与**投影注册表**的双向覆盖（`[UI-TEST-001]` / `[ARCH-UI-005]`）。
#[test]
fn live_control_plane_coverage_matches_the_projected_registry() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据 3：生产模式下 `ui:inject` 在**真实执行面**上也被硬禁（403），且只读能力不受影响。
#[test]
fn production_plane_hard_denies_injection_on_the_live_window() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据 4：测试模式 + `Interactive` ⇒ 事件注入真的落进活窗口（§12.4）。
#[test]
fn interactive_test_mode_plane_injects_into_the_live_window() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

// ===========================================================================
// app-mixer 工作线：混音台通道条（投影 + 真实电平）与三个管理动作
// ===========================================================================
//
// 这一半的判据回答两个问题：
//   1. **混音台的通道条真的由工程与引擎驱动吗？**（数量 / 名字 / 数值 / 像素 / 控件树属性）
//   2. **`[UI-MCP-001]` §12.3 的 Administrative 三件事真的发生了吗？**（切视图 / 写盘 / 重建引擎）
//
// 全部判据都住在**一个**测试目标里（本文件）：`src/live_surface.rs` 用 `#[path]` 被本目标
// 装进来编译，而 `-D warnings` 下"没人用的公开方法"是硬错误 —— 一个目标 = 一份确定的使用
// 集合，多目标会让某个目标看不到某个方法（live-port 线 CI 第 1 轮就死在这条上）。

use std::path::PathBuf;

use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::host::{self, MeterPump};
use yeban_app::open::open_project_file;
use yeban_engine::meter::{MeterFrame, meter_channel};
use yeban_model::ids::EntityId;
use yeban_model::project::YebanProjectV1;
use yeban_ui_mcp::tree::UiTree;
use yeban_ui_test_port::tree::ControlTree;

/// 装配选项：判据只调三个旋钮（权限 / 控制台 Tab / 每个引擎代的量子数）。
fn options(permission: Permission, console_tab: i32) -> LiveWiringOptions {
    LiveWiringOptions {
        permission,
        console_tab,
        save_path: None,
        engine_quanta: 4,
        // 这条装配不接撤销端口（与既有判据 16 的"没有撤销会话"形态一致）。
        undo: None,
        proposal_source: yeban_app::proposals::ProposalSource::none(),
    }
}

/// 通道条族规模（`track-{i}-channel-strip`）。
fn channel_strips(tree: &ControlTree) -> usize {
    tree.with_prefix("track-")
        .filter(|node| node.id.ends_with("-channel-strip"))
        .count()
}

/// 取某个语义 ID 在**执行面**控件树（`ControlTree`）里的标签。
///
/// 两个助手对应两条**不同的**树类型：`ControlTree` 是 `LivePort` 内省出来的那棵树
/// （`LiveUi::tree_snapshot`），`UiTree` 是 `ui/tree` 把同一棵树投影成线格式的结果。
/// 判据两边都用，所以类型必须分开写 —— 混用是编译错误（CI 第一次就把这一条抓住了）。
fn label_of(tree: &ControlTree, id: &str) -> String {
    tree.find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里没有 `{id}`"))
        .label
        .clone()
}

/// 取某个语义 ID 在**控制面**投影（`ui/tree` 的 `UiTree`）里的标签。
fn label_in(tree: &UiTree, id: &str) -> String {
    tree.find(id)
        .unwrap_or_else(|| panic!("`ui/tree` 的投影里没有 `{id}`"))
        .label
        .clone()
}

/// 取某个语义 ID 在**执行面**控件树（`ControlTree`）里的 `accessible-value` 原文。
///
/// `None` = 该元素**没有声明** `accessible-value`（`[ARCH-UI-004]` 的可选状态属性；
/// 不是空串）。判据既要读值，也要读"没有值"。
fn value_of(tree: &ControlTree, id: &str) -> Option<String> {
    tree.find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里没有 `{id}`"))
        .value
        .clone()
}

/// 取某个语义 ID 在**执行面**控件树（`ControlTree`）里的 `accessible-checked`。
fn checked_of(tree: &ControlTree, id: &str) -> Option<bool> {
    tree.find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里没有 `{id}`"))
        .checked
}

/// 取某个语义 ID 在**控制面**投影（`ui/tree` 的 `UiTree`）里的 `accessible-value`。
fn value_in(tree: &UiTree, id: &str) -> Option<String> {
    tree.find(id)
        .unwrap_or_else(|| panic!("`ui/tree` 的投影里没有 `{id}`"))
        .value
        .clone()
}

/// 取某个语义 ID 在**控制面**投影（`ui/tree` 的 `UiTree`）里的 `accessible-checked`。
fn checked_in(tree: &UiTree, id: &str) -> Option<bool> {
    tree.find(id)
        .unwrap_or_else(|| panic!("`ui/tree` 的投影里没有 `{id}`"))
        .checked
}

/// 从推子的 `accessible-value`（`"-3.2 dB"`）里取出 dB 数值。
///
/// 用于把**读回来的文本**与 `TrackV3::volume_db`（模型的权威数字）对账 ——
/// 而不是与"测试自己写的期望文本"对账。`{:.1}` 的显示精度 ⇒ 容差 0.05 dB。
fn db_of_value(value: &str) -> Option<f32> {
    value.strip_suffix(" dB")?.trim().parse::<f32>().ok()
}

/// 从电平标签里取出**峰值 dBFS**（`"… 峰值 -6.0 RMS -23.4 dBFS"` ⇒ `-6.0`）。
///
/// 为什么不用字符串相等去断言电平: 电平是**新引擎真实合成**的产物, 把它写成常量会让这条判据
/// 在合成器任何一次合理调参后变红 —— 那时红的是"数字变了", 而不是"接线坏了"。
/// 这条判据要守的是**接线**: 注入值被丢弃 + 读数是新队列的真实结果。
fn dbfs_value(label: &str) -> Option<f32> {
    let peak = label.split("峰值 ").nth(1)?;
    peak.split_whitespace().next()?.parse::<f32>().ok()
}

/// 一帧像素的 FNV-1a 指纹（与 `ui/screenshot` 的指纹同算法）。
fn frame_fingerprint(image: &yeban_ui_test_port::Rgb8Image) -> String {
    let (_bytes, evidence) =
        encode_with_evidence(image, DEFAULT_MAX_PNG_BYTES).expect("Tier-1 帧必须非零且非全黑");
    format!("{:016x}", evidence.fingerprint)
}

// ---------------------------------------------------------------------------
// 判据 17 的**同案差分像素**（`ADR-0005` S1）：同一次运行的两帧对比，**不碰 golden**
// ---------------------------------------------------------------------------

/// 差分包围盒允许的**命名容差**（逻辑像素）。
///
/// 它吸收两处不可避免的取整：① 画布 → 窗口的 y 平移由投影 `band_y` 的取整标定；
/// ② 字形边缘的抗锯齿会落到相邻像素。**它不允许大到"什么都容得下"** ——
/// 上界由 [`LANE_DIFF_TOLERANCE_MAX_PX`] 与判据 `the_lane_diff_tolerance_stays_bounded`
/// 钉住（把这里放大到全屏必须让那条判据变红）。
#[cfg(feature = "in-process-mcp")]
const LANE_DIFF_TOLERANCE_PX: i32 = 2;

/// 容差的**上界**：判据钉住它，防止"把容差放大到全屏"把断言变成空转。
#[cfg(feature = "in-process-mcp")]
const LANE_DIFF_TOLERANCE_MAX_PX: i32 = 4;

/// 两帧的**差异像素**读数（逐像素比较 RGB 三字节；相等即逐字节相同）。
struct FrameDiff {
    /// 差异像素数。
    count: u64,
    /// 差异像素的整数包围盒（含边界；单位 = 像素）。
    bbox: Rect,
}

/// 逐像素比较两帧；**逐字节相同** ⇒ `None`（"界面一位没动"）。
fn frame_diff(before: &Rgb8Image, after: &Rgb8Image) -> Option<FrameDiff> {
    assert_eq!(
        (before.width(), before.height()),
        (after.width(), after.height()),
        "两帧必须同尺寸（视口由装配时的 `DemoScene` 定下，重投影不改它）"
    );
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0_u32, 0_u32);
    let mut count = 0_u64;
    for y in 0..before.height() {
        for x in 0..before.width() {
            if before.pixel(x, y) != after.pixel(x, y) {
                count += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    (count > 0).then(|| FrameDiff {
        count,
        // 边界含两端 ⇒ 宽高 +1（`Rect::right()`/`bottom()` 是**不含**的右/下界）。
        bbox: Rect::new(
            min_x as i32,
            min_y as i32,
            max_x - min_x + 1,
            max_y - min_y + 1,
        ),
    })
}

/// 落在 `area` **之外**的差异像素数。`0` = "期望区域外逐字节相同"。
#[cfg(feature = "in-process-mcp")]
fn diff_pixels_outside(before: &Rgb8Image, after: &Rgb8Image, area: Rect) -> u64 {
    let mut outside = 0_u64;
    for y in 0..before.height() {
        for x in 0..before.width() {
            let inside = (x as i32) >= area.x
                && (x as i32) < area.right()
                && (y as i32) >= area.y
                && (y as i32) < area.bottom();
            if !inside && before.pixel(x, y) != after.pixel(x, y) {
                outside += 1;
            }
        }
    }
    outside
}

/// 被改轨道上**泳道带栈**在窗口像素里的期望区域。
///
/// **几何来源是投影**：`ViewState::automation_lanes` 的 `band_y` / `band_height`
/// （`ADR-0001` `D28` 的整数口径，`automation.rs:750-753` 的等分）。画布 → 窗口的 y
/// 平移由 `anchor` 标定：起点那一条泳道在 `ui/tree` 里的真实几何（`anchor.y`）减去它的
/// 投影 `band_y` —— 这不是猜，`D28` 的**唯一**注入点把同一份 `band_y` 写进这**一个**
/// 窗口，因此平移是一个常量。x 范围直接取该泳道的窗口几何（所有泳道共用同一条 x 带）。
///
/// 为什么是"带**栈**"而不是"新泳道那一条带"：见判据 17 的步 5b（加一条泳道会重新等分
/// 整条带栈，**既有**泳道的带高也会变）。
#[cfg(feature = "in-process-mcp")]
fn projected_lane_stack_box(
    views: [&ViewState; 2],
    track_index: usize,
    anchor_band_y: f32,
    anchor_bounds: Rect,
    tolerance: i32,
) -> Rect {
    let origin_y = anchor_bounds.y - anchor_band_y.round() as i32;
    let mut bands: Vec<(i32, i32)> = Vec::new();
    for view in views {
        for lane in view
            .automation_lanes
            .iter()
            .filter(|lane| lane.track_index == track_index)
        {
            let y0 = origin_y + lane.band_y.round() as i32;
            bands.push((y0, y0 + lane.band_height.round() as i32));
        }
    }
    let top = bands
        .iter()
        .map(|(y0, _)| *y0)
        .min()
        .expect("该轨道至少一条泳道");
    let bottom = bands
        .iter()
        .map(|(_, y1)| *y1)
        .max()
        .expect("该轨道至少一条泳道");
    Rect::new(
        anchor_bounds.x - tolerance,
        top - tolerance,
        anchor_bounds.width + 2 * tolerance as u32,
        (bottom - top) as u32 + 2 * tolerance as u32,
    )
}

/// 工程里的**非主总线**轨道身份（`BTreeMap` 升序 = 视图顺序）。
fn track_ids(project: &YebanProjectV1) -> Vec<EntityId> {
    project
        .tracks
        .keys()
        .copied()
        .filter(|id| *id != project.master_bus_track_id)
        .collect()
}

/// 一个一次性的临时目录（不引入 `tempfile`：它不是本 crate 的依赖）。
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-app-live-{tag}-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 判据 5（**混音台**）：通道条数 == 工程轨道数；**同一个活窗口**换工程 ⇒ 名字/数值/像素全变。
#[test]
fn mixer_channel_strips_follow_the_projected_project_on_the_same_window() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

/// 判据 7（**混音台的四项写入能力**）：推子 / 声相 / 静音 / 独奏各走**自己的**语义元素
/// 与**自己的**轨道下标，并且各自满足"拖动期只写视图态 ＋ 松手恰一次提交 ＋ `Cmd+Z` 复原"。
///
/// ## 为什么四项用四个不同的轨道下标（序号化 / 多路径探针）
///
/// 上一轮的教训：判据用**固定探针 id** 时，单独破坏某一路径**仍然绿**。
/// 本判据因此给每一项能力分配一组**互不共享**的 `(语义元素, 轨道下标, 模型字段)`：
///
/// | # | 能力 | 语义元素 | 轨道下标 | 模型字段 |
/// | :--- | :--- | :--- | :--- | :--- |
/// | ① | 音量 | `track-0-fader` | 0 | `TrackV3::volume_db` |
/// | ② | 声相 | `track-1-pan` | 1 | `TrackV3::pan` |
/// | ③ | 静音 | `track-2-mixer-mute-button` | 2 | `TrackV3::mute` |
/// | ④ | 独奏 | `track-3-mixer-solo-button` | 3 | `TrackV3::solo` |
///
/// 每一段都额外断言**别的轨道一位没变** ⇒ "写错下标"这一类缺陷会被指名抓到。
///
/// ## 三条会变红的注入（都实测过，见交付报告）
///
/// | 注入 | 位置 | 现象 |
/// | :--- | :--- | :--- |
/// | 删掉推子的 `TouchArea`（"混音台回退成无输入面"） | `ui/console/mixer_console.slint` | ① 段：注入后 `track-volumes[0]` 与注入前逐字相同 ⇒ 红 |
/// | 把提交从 `release` 挪到 `drag`（"拖动不原子"） | `src/host.rs` 的 `on_mixer_fader_drag` | ① 段：`拖动之后 undoable == 0` 与 `工程字段仍是起点值` 两条同时红 |
/// | 删掉 `wire_mixer_edit(&window, undo)` | `src/live_surface.rs` | 四段一起红（没有任何 `.slint` 回调被接到端口上） |
#[test]
fn the_mixer_strip_writes_volume_pan_mute_and_solo_back_into_the_project() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

/// 判据 7b（**主控通道条的写入能力**，本切片补的那一格）：主控**音量**与**声相**各自满足
/// "拖动期只写视图态 ＋ 松手恰一次提交 ＋ `Cmd+Z` 逐位回退 ＋ `Escape` 写回起点不提交 ＋
/// 孤立 `move`／迟到松手不再改值"。
///
/// ## 为什么主控要用**另一组无下标的回调**（本判据同时钉住这个方案）
///
/// 主总线**不在** `track-names` / `track-ids` 里（投影刻意排除它，它单独走
/// `ViewState::master: Option<TrackView>`）⇒ 通道条那八个回调的 `int` 参数里**没有**任何
/// 合法值能指到主控。本切片给主控一组**不带下标**的回调
/// （`mixer-master-fader-grab(length)` 等），身份由宿主在按下那一刻从权威工程取
/// （`host::master_track_id` ← `UndoPort::try_project().master_bus_track_id`）。
/// 于是 `.slint` 里没有身份→下标的算术，也没有哨兵下标。
///
/// ## 五行字面读数（每一项能力各走一遍，元素与模型字段互不共享）
///
/// | # | 情形 | 主控音量（`mixer-master-fader`） | 主控声相（`mixer-master-pan`） |
/// | :--- | :--- | :--- | :--- |
/// | ① | 按下 ＋ 拖动 | 文本 `0.0 → -33.0`、工程逐位未动、`undoable` 0、标志 `true` | 文本 `C → R50`、工程逐位未动、`undoable` 0、标志 `true` |
/// | ② | 松手 | 工程 `volume_db = -33.0`、`undoable` **恰 1** | 工程 `pan = +0.5`、`undoable` **恰 1** |
/// | ③ | `Cmd+Z` | `volume_db` **逐位**回退、`undoable` 0 | `pan` **逐位**回退、`undoable` 0 |
/// | ④ | `Escape` | 文本写回 `0.0`、工程逐位未动、`undoable` 0、标志 `false` | 文本写回 `C`、工程逐位未动、`undoable` 0、标志 `false` |
/// | ⑤ | 孤立 `move` ＋ 迟到松手 | 文本仍是起点、`undoable` 仍 0 | 文本仍是起点、`undoable` 仍 0 |
///
/// ④ 的两条路径**故意不同**：音量走**端口**（`ui/dispatch_key_press`，真事件源），
/// 声相走 `MainWindow::invoke_key_action`（`.slint` 的 `key-action` 那一格）——
/// 两条都坏才会绿。
///
/// ## 怎么变红（注入的字面红行见交付报告）
///
/// | 注入 | 位置 | 现象 |
/// | :--- | :--- | :--- |
/// | 摘掉主控推子的 `TouchArea`（"主控回退成不可操作"） | `ui/console/mixer_console.slint` | ① 步：注入后 `master-volume-display` 与注入前逐字相同 ⇒ 红 |
/// | 让主控的按下不接线（摘掉 `ui.on_mixer_master_fader_grab`） | `src/host.rs` 的 `wire_mixer_edit` | ① 步：标志仍 `false`、文本不动 ⇒ 红 |
/// | 把主控的提交从 `release` 挪到 `drag`（"主控拖动不原子"） | `src/host.rs` 的 `on_mixer_master_fader_drag` | ① 步：`undoable` 不再是 0 ⇒ 红 |
/// | 让主控 `Escape` 走提交 | `src/host.rs` 的 `cancel_mixer_gesture` | ④ 步：`undoable` 涨到 1、工程字段被改 ⇒ 红 |
#[test]
fn the_master_strip_writes_volume_and_pan_back_into_the_project() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

/// 判据（`ADR-0005` S1 的**第三条收尾路径**）：`Escape` **取消**混音手势（推子 / 声相），
/// 写回起点值、**不提交**，并且手势标志归位 —— 与 `cancel_track_height_drag` 同口径。
///
/// ## 这条判据为什么必须存在
///
/// 在本次改动之前，混音手势只有"松手"这一条收尾路径：`.slint` 的 `pointer-event` 把
/// `up` / `cancel` 都送到 `mixer-*-release`，宿主在 `release` 里提交。拖动中途按 `Escape`
/// **什么都不发生** —— 用户按了取消，界面上数值仍停在拖到的位置，随后松手还会提交。
///
/// ## 四条路径各自的字面读数（都在本判据里当场读出来）
///
/// | # | 情形 | 语义元素 | 轨道下标 | 模型字段 | 期望 |
/// | :--- | :--- | :--- | :--- | :--- | :--- |
/// | ① | 拖动期 | `track-0-fader` | 0 | `TrackV3::volume_db` | 文本已动、工程逐位未动、`undoable` 0、标志 `true` |
/// | ② | `Escape`（**真事件源**） | 同上 | 0 | 同上 | 文本写回起点、工程逐位未动、`undoable` 0、标志 `false` |
/// | ③ | `Escape` 后的孤立 `move` ＋ 迟到的松手 | 同上 | 0 | 同上 | 文本仍是起点、`undoable` 仍 0（手势真的结束了） |
/// | ④ | `Escape`（**回调那一格**）＋ 孤立 `move` | `track-1-pan` | 1 | `TrackV3::pan` | 文本 `R50 → C`、工程逐位未动、`undoable` 0 |
/// | ⑤ | 只松手（对照） | `track-2-fader` | 2 | `TrackV3::volume_db` | **恰一步** `undoable`、`Cmd+Z` 逐位回退 |
///
/// ## 怎么变红（注入的字面红行见交付报告）
///
/// | 注入 | 位置 | 现象 |
/// | :--- | :--- | :--- |
/// | 摘掉 `ui.on_mixer_cancel_gesture(..)` | `src/host.rs` 的 `wire_mixer_edit` | ② 步：标志仍是 `true`、文本仍是 `-33.2` ⇒ 红 |
/// | 摘掉 `cancel_mixer_gesture` 里的 `preview_target_volume` 写回 | `src/host.rs` | ② 步：文本仍是 `-33.2` ⇒ 红 |
/// | 让取消走提交（`cancel_mixer_gesture` 里 `commit_ops`） | `src/host.rs` | ② 步：`undoable` 涨到 1、工程字段被改 ⇒ 红 |
#[test]
fn escape_cancels_the_mixer_fader_and_pan_drag_without_committing() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

/// 判据（像素**命题 ②**）：一次**提交了的**推子手势真的换了画面 —— 与 `Escape` 那条判据的
/// 命题 ①（「默认帧可复现」= 确定性）**分开陈述**，两者不可互相代替。
///
/// 驱动入口是 `MainWindow` 的三个混音台回调（`.slint` 的 `TouchArea` 调的就是它们，
/// 见 `ui/console/mixer_console.slint:322-332`）—— 与真实指针事件同一条链。
/// `LiveUi::capture` 是 `ui/screenshot` 内部用的**同一个** `capture_tier1`，因此这里
/// 不需要第二套像素路径，也**不碰 golden**（比的是同一次运行的两帧）。
#[test]
fn a_committed_mixer_fader_gesture_changes_the_frame() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

/// 判据 7c（**像素命题 ②**，主控通道条）：提交一次**主控**推子手势之后，画面**真的变了** ——
/// 给差异像素数与差异包围盒（命题 ① "默认帧可复现" 是另一件事，见混音台那一对判据）。
///
/// 驱动入口是 `MainWindow` 的**主控**混音回调（`.slint` 的 `mixer-master-fader` 那个
/// `TouchArea` 调的就是它们，见 `ui/console/mixer_console.slint`）——
/// 与真实指针事件同一条链（后者由判据 7b 走端口）。
/// 比的是**同一次运行**的两帧，**不碰 golden**，也不用尺寸当证据
/// （本仓 PNG 是存储式 deflate，1920×1080 恒为 6,222,418 字节）。
#[test]
fn a_committed_master_fader_gesture_changes_the_frame() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

/// 判据 20（**编辑 ⇒ 发声**端到端，本票）：在**同一个活窗口**上做一次真实的混音台推子
/// 手势 ⇒ `UndoPort` 真的提交一步 ⇒ 控制线程心跳一跳 ⇒ **引擎快照里那一个字段**变了。
///
/// 它证明的那句话是："改一个混音参数之后，播放侧**真的**会看到新值。" 断点两侧都是**字面
/// 读数**：
///
/// | 时刻 | 读数 | 期望 |
/// | :--- | :--- | :--- |
/// | 起引擎之后 | `engine_track_volume_db(track0)` | `-3.2`（= 工程起点） |
/// | 手势之后、心跳之前 | 同上 | **仍是** `-3.2`（这就是本票修的那个断点） |
/// | 心跳之后 | 同上 | `-33.2`（= 提交进工程的那一版） |
///
/// 手势走的是 `MainWindow` 的**三个混音台回调**（`.slint` 的 `TouchArea` 调的就是它们，
/// 见 `ui/console/mixer_console.slint:324-330`）—— 与真实指针事件同一条链（后者另有判据 7）。
///
/// ## 会变红的注入（实测见交付报告）
///
/// | 注入 | 位置 | 现象 |
/// | :--- | :--- | :--- |
/// | 心跳里跳过 `publish_project`（"编辑不发布"） | `src/main.rs` / `engine_tick` | 第 3 行的读数仍是 `-3.2` ⇒ 红 |
/// | 心跳里跳过 `prune`（"发布了不回收"） | `EngineHost::heartbeat` | 第 2 跳 `released == 0` / `pending_len == 1` ⇒ 红 |
#[test]
fn a_mixer_gesture_reaches_the_engine_snapshot_on_the_live_window() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 工程里第 `index` 条非主总线轨道的混音四格 `(volume_db, pan, mute, solo)`。
///
/// 身份从**投影**取（`ViewState::tracks[index].id`），因此这里的读数与界面上的
/// `track-{index}-*` 寻址的是**同一条轨道**（不是两套下标口径）。
fn mixer_readings(
    project: &YebanProjectV1,
    view: &ViewState,
    index: usize,
) -> (f32, f32, bool, bool) {
    let id = view.tracks[index].id.clone();
    let track = project
        .tracks
        .values()
        .find(|track| track.id.to_canonical_string() == id)
        .unwrap_or_else(|| panic!("工程里必须有身份 {id} 的轨道"));
    (track.volume_db, track.pan, track.mute, track.solo)
}

/// **主总线**的 `(volume_db, pan)` —— 按 `master_bus_track_id` 从**模型**读。
///
/// 主总线不在 `view.tracks` 里（投影刻意排除它）⇒ [`mixer_readings`] 的"按下标取"口径
/// 对它**不适用**，必须按身份取。这同时是本判据的**独立基准**：它不从界面读，
/// 因此"改了没有"不依赖被测的写入面。
fn master_readings(project: &YebanProjectV1) -> (f32, f32) {
    let id = project.master_bus_track_id;
    assert!(!id.is_nil(), "工程必须有主总线身份（演示夹具）");
    let track = project
        .tracks
        .get(&id)
        .unwrap_or_else(|| panic!("主总线 {id} 必须在 tracks 里"));
    assert_eq!(
        track.kind,
        yeban_model::project::TrackKind::Master,
        "`master_bus_track_id` 指向的必须是一条 `TrackKind::Master`（与模型 `validate` 同口径）"
    );
    (track.volume_db, track.pan)
}

/// 判据 6（**电平真的被消费**）：注入一组已知 `MeterFrame` ⇒ 控件树里的 dBFS 与之一致。
///
/// 容差与理由：`MeterFrame` 的幅度是线性的，dBFS = `20·log10(a)`；界面上显示的是
/// `{:.1}` 的文本，因此"显示值 vs 真值"的偏差上界是 **0.05 dB**（半个显示步长）。
/// 判据同时钉住两边：文本**逐字**相等（`"0.0"` / `"-6.0"` / `"-120.0"`），以及
/// 未取整的浮点值与解析式相差 < 1e-3 dB。只断言其中一个都会漏掉一类错。
#[test]
fn mixer_meter_labels_match_the_injected_frames() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据 6b（**长度契约，测试端口那条腿**）：界面 `track-names` 行数与手里那份投影的
/// 轨道数不等长 ⇒ 与生产路径**同一份实现**：抽干照做、**跳过注入**、如实回报
/// `MeterPump::LengthMismatch { rows, projected }`。
///
/// ## 它钉的缺口（2026-10-08 收敛）
///
/// 本文件走的执行面（`build_live_ui*` ⇒ `LiveAdminSurface`）原先是"抽干 ＋ 应用"的
/// **第二份**实现，而它**没有**生产路径那道长度闸（`host::pump_meters`，
/// `crates/yeban-app/src/host.rs:1958`）⇒ 无头 Tier-1 判据面与产品路径跑的是**两条腿**，
/// 判据面可能因此看不见产品路径上的真实行为（缺口登记在
/// `docs/ledger/feature-alignment.md` 的"60Hz 主线程电平心跳"行）。现在两处共用
/// **同一个函数**（`LiveAdminSurface::pump_meters` 只加"重抓树"这一步）；
/// 本判据在"这条腿重新变成无牙（无条件注入）"时变红。
///
/// ## 造法（与生产判据 3 同款，但走**另一条腿**）
///
/// 窗口上装载 `filled` 工程（3 条非主总线轨道），执行面手里的投影也是 `filled`；
/// 随后把 `demo` 工程的投影（6 条）**直接注入窗口**（`host::apply_view`）——
/// 这正是"控制面换了工程、而持有循环的那一段还拿着旧投影"的形态
/// （生产那条腿的对应判据是 `tests/production_meter_leg.rs` 的判据 3）。
///
/// ## 为什么要四个字面读数
///
/// 1. **等长**那一跳：注入的已知幅度真的进控件树（前提：这条腿本来有牙）；
/// 2. **不等长**那一跳：`LengthMismatch { rows: 6, projected: 3 }`，且电平数组一位没动；
/// 3. **抽干照做**：队列容量取 **1** ⇒ "这一跳有没有抽干"可以从
///    `MeterPublisher::dropped()` 直接读出来（没抽干 ⇒ 队列还满着 ⇒ 下一条推送被丢）；
/// 4. **反证**：把长度对齐回去，面板里那帧必须重新进界面（⇒ 上面那条读数不是
///    "这条腿永远不注入"）。
#[test]
fn the_live_surface_meter_leg_reports_a_length_mismatch_and_skips_the_injection() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

/// 判据 7（**静音下限与 NaN**）：敌意电平输入在控件树里**不许**出现 `NaN`，只许出现下限。
///
/// 这里故意绕过引擎的生产侧钳位（直接构造 `NaN` 帧），因为消费侧必须自己也是安全的：
/// 引擎的防线挡不住"测试注入 / 上游回归"这两类输入。
#[test]
fn hostile_levels_never_render_as_nan_in_the_control_tree() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据 8（**管理动作 1/3**）：`ui/switch_main_view` 真的切了主视图，树跟着换。
#[test]
fn admin_switch_main_view_really_switches_the_live_view() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据 9（**管理动作 2/3**）：`ui/force_save` 真的写了盘，写出来的能被容器读回。
#[test]
fn admin_force_save_really_writes_a_readable_container() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据 21（**运行中换工程 1/2：控制面入口**）：`ui/open_project` 走**真的** JSON-RPC 文本
/// 换掉整份工程 —— 语义元素族、行几何、电平元素族与像素全部跟着换；失败路径
/// **当前工程一位不动**。
///
/// ## 它钉的缺口
///
/// 在这一条之前，产品与控制面**在进程跑起来之后**都无法换工程：唯一的打开入口是启动时的
/// CLI `--open`（`crates/yeban-app/src/main.rs` 的 `cli::load_project`），控制面 15 条
/// `ui/*` 方法里没有一条叫 `ui/open_project`。本判据证明新方法真的落到**同一个活窗口**上，
/// 而且复用**同一条**容器读取核心（`yeban_app::open` 的 `read_capped` ＋
/// `open_project_archive`；本方法用其上的 `open_project_file`）与**同一个**注入点
/// （`LiveAdminSurface::apply_project`）—— 因此它不造第二个权威。
///
/// ## 两个工程的选取（分辨力）
///
/// `filled`（3 条非主总线轨）与 `demo`（6 条）互不相同 ⇒ "轨道数真的变了"是一个有分辨力的
/// 读数；两者都带真实摆放，因此换工程后像素**必须**变。
#[test]
fn admin_open_project_swaps_the_whole_projection_through_the_control_plane() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据 21b（**运行中换工程 1b：旧工程的未保存改动**）：`saveFirst` 默认 `true` ⇒
/// 当前工程**没有落点**时 `ui/open_project` **拒绝**（既有错误码），当前工程一位不动；
/// 显式给 `saveFirst: false`（＝明确同意丢弃）时才放行。
///
/// ## 为什么这条判据有分辨力
///
/// 同一次调用在同一份装配上给出**两个相反**的结局（拒绝 / 成功），因此它证明的不是
/// "这条方法总是失败"，而是"拒绝的**原因**真的是没有落点"。
/// 默认值取自既有工具 `yeban_close_project` 的 `saveFirst`（`arg_bool(call, "saveFirst", true)`）
/// —— 不是本切片发明的口径。
#[test]
fn open_project_refuses_to_discard_unsaved_work_when_there_is_no_save_target() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据 22（**运行中换工程 2/2：窗口读数**）：换工程后**行数与电平数组等长**（没有串轨）、
/// 行几何跟着换，并且画面**真的**变了（不同像素数 + 包围盒）。
///
/// 为什么这一条不能并进判据 21：判据 21 走控制面（`ui/tree` / `ui/node` / `ui/screenshot`
/// 的**线上形态**），而"电平数组与轨道数组等长"这件事住在**活窗口的属性**上
/// （`track-names` / `track-meter-levels`），控制面没有把它们暴露成 `ui/property` 的读法。
/// `into_control_plane` 会把执行面装箱取走 ⇒ 窗口读数与控制面读数**不可兼得**，因此两条判据
/// 各走一条路（与 `live_surface.rs` 的 `dispatch_key_press` 存在同一个理由）。
///
/// ## 为什么"等长"是可判据的（而不是一句注释）
///
/// `host::apply_meters` 的第一件事就是 `debug_assert_eq!(row_count(track_names),
/// snapshot.tracks.len())`，而 `cargo test` 默认开着 debug 断言 ⇒ 电平数组与行数一旦不等长，
/// 这一跳会**直接 panic**（不是静默串轨）。本判据另外把两个长度**读出来**逐个断言，
/// 因为"没 panic"与"长度真的相等"不是同一句话。
#[test]
fn open_project_keeps_the_meter_arrays_aligned_with_the_track_rows() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

/// 判据 20（`ROAD-M4-008` 选项 (a)：**单一写者会话**的 GUI 侧）：挂在控制面上时，
/// `ui/force_save` 写出的**不是**界面缓存，而是**权威会话当前那一版**。
///
/// ## 为什么这条判据有分辨力（而不是"两条路写出同样的字节"）
///
/// 关键是让"界面缓存"与"权威工程"**故意分叉**：会话侧改一次工程，但**不**调
/// `sync_authority` ⇒ 活窗口仍画着旧那一版；随后按保存。
///
/// | 走的路 | 结果 |
/// | :--- | :--- |
/// | 界面缓存（改动前的本地路径 `save_project_file`） | 要么写旧那一版（新泳道不在文件里），要么被会话的排他锁拒（`ui/force_save` 直接失败） |
/// | 权威（`ProjectAuthorityHandle::save_to`） | 文件里**有**那条新泳道，且回执 `writtenByAuthority = true` |
///
/// ## 论证顺序
///
/// | 步 | 动作 | 读回 |
/// | :--- | :--- | :--- |
/// | 1 | 在真工程文件上以**单一写者**形态挂载，并以权威装配真实界面（带 `save_path`） | 权威可写；窗口里没有 pan 泳道 |
/// | 2 | 真环回 socket 发 `yeban_edit_automation`（建 pan 泳道）；**不**同步投影 | 权威工程里有新泳道；活窗口仍**没有** —— 界面缓存确实是旧的 |
/// | 3 | `ui/force_save` | 成功，`report.bytes > 0`、`report.writtenByAuthority = true` |
/// | 4 | 用**后续读者**读同一个文件 | 新泳道**在**文件里 ⇒ 写的是权威那一版，不是陈旧缓存 |
#[cfg(feature = "in-process-mcp")]
#[test]
fn a_gui_force_save_while_mounted_writes_through_the_authority() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据 10（**管理动作 3/3**）：`ui/reload_engine` 真的重建引擎，并把电平接回下限。
///
/// 副作用链（每一步都可观测）：注入 0.0 dBFS 的电平 ⇒ `ui/tree` 读得到 ⇒ 重建引擎 ⇒
/// **新的**电平队列被采纳、旧读数作废 ⇒ `ui/tree` 读回下限。
#[test]
fn admin_reload_engine_rebuilds_and_resets_the_meter_tap() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据 11（**管理动作的失败路径**）：引擎重建失败必须**如实报错**，不许假成功。
#[test]
fn admin_reload_engine_reports_a_real_failure_honestly() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据 12（**生产模式**）：三个管理动作在**生产模式**下按 scope 被拒，且没有副作用。
#[test]
fn production_mode_rejects_the_three_admin_actions_by_scope() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

// ===========================================================================
// `ui-mcp-dryrun-ime` 工作线：`dryRun`（ADR-0001 **D48**）与 IME 合成态
// ===========================================================================
//
// 这一半的判据必须在**真实窗口**上跑（Tier-1 光栅化 + 真控件树 + 真 `MainWindow`
// 属性），因为"dryRun 不改状态"这句话只有对着**真的会改状态**的执行面才有意义：
// 零 Slint 的假面那 10 条判据证明的是管线与词表，这里证明的是**接线**。
//
// 本机纪律禁止编译 Slint ⇒ 这 3 条**只能由 CI 判**（`docs/ledger/ui-mcp-dryrun-ime-notes.md`
// 的"本机 vs CI"一节）。本机跑过的对应判据是 `crates/yeban-ui-mcp/src/service.rs` 的那批。

/// 判据 13（D48）：`dryRun=true` 在**真实界面**上不改一个状态位，且它**预告**的事情
/// 与随后的真调用**逐字段一致**。
///
/// 观测面有四个，缺一不可：
/// - `ui/tree` 的**线上 JSON**（逐字节）；
/// - 磁盘（`ui/force_save` 的落点必须不存在）；
/// - `MainWindow.arrangement-view` 的回读（经 `ui/switch_main_view` 的回执）；
/// - 引擎代数（`ui/reload_engine` 的回执 `generation` 必须是**第一次**真调用推进的 1，而不是 2）。
#[test]
fn dry_run_leaves_the_live_window_and_the_disk_untouched() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

/// 判据 14（`[UI-A11Y-002]`）：IME 合成态在**真实界面**上可观测，且读的是
/// `LiveUi::input_context()` 交出的**那一个**状态机（不是影子变量）。
///
/// 两个方向都断言：非合成 → 合成 → 非合成（含"焦点离开文本域自动结束合成"的既有语义）。
#[test]
fn ime_composition_is_observable_on_the_live_window() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据 14b（`[UI-A11Y-002]` §7.2，**本线新增的那一半**）：合成态的**事件源**。
///
/// 判据 14 证明的是"状态机可观测"，它直接写 `input_context()` 那个对象 ——
/// 也就是说 `.slint` 一侧到底有没有事件源，它**证明不了**（这正是上一版 `transport.slint`
/// 只写着"先用 role=text-input 占位"时留下的缺口）。
///
/// 本判据从**事件源那一侧**驱动：`ui/transport.slint` 的 `bpm-input`
/// （真 `TextInput`）在 `changed preedit-text` / `changed has-focus` 里调用的两个回调，
/// 由 `host::wire_input` 接到**同一个** `InputContext`。链路因此是：
///
/// ```text
/// Slint TextInput.preedit-text --changed--> MainWindow.ime-composition-changed
///   --host::wire_input--> InputContext --ui/property--> isComposing
/// ```
///
/// ⚠ **诚实边界**：上游有信号（`preedit-text`，出处见 `ui/transport.slint` 的注释），
/// 但 Slint 1.18.1 **没有公开的 preedit 注入面**（`WindowEvent` 没有合成变体、
/// `InternalKeyEvent`/`KeyEventType` 未被再导出、`process_key_input` 是 `pub(crate)`），
/// 因此判据只能驱动到 `.slint` 回调这一格。原因与取舍写在
/// `docs/ledger/app-projection-notes.md` 的 needs 里，本判据的 `report_line` 也会打印它。
#[test]
fn ime_composition_flows_from_the_slint_event_source_into_the_control_plane() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据 15（D48）：`dryRun` 对"注定失败的真调用"**如实报错** —— 与真调用同码、同话、同 data。
///
/// 未配置保存路径（`LiveWiringOptions::save_path = None`）时 `ui/force_save` 会报
/// `-32005`（"这条能力在这个装配上没接线"）；`dryRun` 必须报**同一件事**，
/// 而不是给一份看起来成功的预览（领域侧 dryRun 的口径就是"只做参数与领域合法性校验"）。
#[test]
fn dry_run_reports_the_same_failure_as_the_real_save() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

// ===========================================================================
// `N2` 裁决 (1)：GUI 路径的**逻辑键**快捷键
// ===========================================================================
//
// `docs/ledger/open-questions.md` 问题 1 的裁决 (1)：**GUI 绑逻辑键，无头端口保留物理码判据**。
// 依据是 Slint 的公开按键事件只有 `text`（`KeyEvent { text, modifiers, repeat }`），
// 拿不到 `src/input.rs` 扫描码表要的物理键身份。
//
// 这一节判的就是裁决要求的那条判据：一个用**逻辑键**表达的快捷键，在**真实界面**上
// 真的作用到宿主状态。链路是：
//
// ```text
// ui/dispatch_key_press(KeyCode) --LivePort--> Slint WindowEvent::KeyPressed { text }
//   --ui/app.slint 的 key-handler(FocusScope).key-pressed--> callback key-action(text, 修饰位)
//   --host::wire_keys--> input::resolve_logical --> apply_action --> MainWindow 的属性
// ```
//
// 没有一处经过物理码；物理码那条路（`physical_key_of` + `InputContext::resolve`）保持原样，
// 仍由本文件其它判据与 `test_port_adapter.rs` / `undo.rs` 的判据覆盖。

/// 判据 16（`N2` 裁决 (1)）：逻辑键快捷键**真的**触发动作 —— 经端口的真事件源，也经回调那一格。
///
/// 三个观测面：
/// 1. `ui/dispatch_key_press` 注入 `"3"`（端口的 `KeyCode::Character` → `type_char` →
///    `WindowEvent::KeyPressed { text: "3" }`）⇒ `active-tool` 必须变成矩阵第 3 行；
/// 2. 同一个端口注入 `Tab` ⇒ 画布焦点下必须切视图，且**被消费**（`accept` 意味着
///    Slint 自己的 `Tab` 焦点轮转不再接管 —— 这正是 §2.2 MUST 的形态）；
/// 3. `invoke_key_action` 直接驱动 `.slint` 回调那一格：已绑定的键被消费、未绑定的键
///    与"这个装配没有撤销会话"的 `Cmd+Z` 如实 `reject`（不假装处理了）。
#[test]
fn a_logical_key_shortcut_from_the_event_source_reaches_the_host_action() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

// ---------------------------------------------------------------------------
// 判据 17：`ROAD-M4-008` 选项 (a) —— 唯一可变权威是控制面会话，界面是它的投影
// ---------------------------------------------------------------------------

/// 真环回 socket 上的一次 JSON-RPC 往返（判据 17 的**客户端**那一半）。
///
/// **它不是第二份协议实现**：协议层（405 / 411 / 413 / 431 / 401 / 200）的判据住在
/// `crates/yeban-app/tests/in_process_mcp.rs` 与 `yeban-mcp` 的单元判据里，本文件
/// **不重复**它们。这里只需要"把一条 `tools/call` 送到那个监听口、把响应体读回来"。
///
/// 为什么走 socket 而不是宿主口：这条判据要证的是"**经 MCP 会话**的改动会到界面"，
/// 因此改动必须真的从一个客户端、带令牌、过鉴权与分发器进来 —— 那是外部 AI 的路径。
#[cfg(feature = "in-process-mcp")]
fn mcp_call(address: std::net::SocketAddr, bearer: &str, body: &str) -> serde_json::Value {
    use std::io::{Read as _, Write as _};
    use std::net::TcpStream;
    use std::time::Duration;

    let timeout = Duration::from_secs(5);
    let mut stream =
        TcpStream::connect_timeout(&address, timeout).expect("连接环回控制面（真 socket）");
    stream.set_read_timeout(Some(timeout)).expect("设置读超时");
    stream.set_write_timeout(Some(timeout)).expect("设置写超时");
    let head = format!(
        "POST {} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Authorization: {bearer}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        yeban_mcp::transport::http::MCP_PATH,
        body.len()
    );
    stream.write_all(head.as_bytes()).expect("写请求头");
    stream.write_all(body.as_bytes()).expect("写请求体");
    stream.flush().expect("刷出请求");
    let mut raw = String::new();
    stream
        .read_to_string(&mut raw)
        .expect("读响应（超时即失败）");
    let body = raw
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("响应里没有头/体分隔符: {raw}"))
        .1;
    serde_json::from_str(body).unwrap_or_else(|error| panic!("响应体不是 JSON ({error}): {body}"))
}

/// 判据 17（`docs/ledger/open-questions.md` 问题 6 选项 (a)）：**经 MCP 会话改动工程 ⇒
/// 界面投影跟着变**，因为两者**只有一个**可变权威。
///
/// ## 这条判据要证的句子
///
/// `build_live_ui_from_authority` 装配出来的界面**没有**自己的工程来源：它只从
/// `ProjectAuthorityHandle`（= 控制面正在服务的那一个 `Domain`）取工程。因此一次 AI
/// 的工具调用改了会话的工程之后，`LiveUi::sync_authority` 依**施加修订号**重投影，
/// 真实 `MainWindow` 的语义元素随之改变 —— 而只读调用一位都不改。
///
/// ## 论证顺序（每一步都用上一步的实读值）
///
/// | 步 | 动作 | 读回 |
/// | :--- | :--- | :--- |
/// | 1 | 以权威装配真实界面（该入口**不接受**工程参数） | 运行时控件树里只有样本那一条泳道 `track-0-automation-volume-lane` |
/// | 2 | 一个**真客户端**带令牌在环回 socket 上发 `yeban_edit_automation`（在从没有泳道的 `TrackPan` 上写一个点） | `status = success`、`applied = true` |
/// | 3 | 从**宿主口**读同一个会话 | 施加修订号 **+1**；工程里真的多了 `track-0-automation-pan-lane` |
/// | 4 | `sync_authority` | `Reprojected { revision }`（不是 `Unchanged`） |
/// | 5 | 再读运行时控件树（同一个活窗口） | **新元素**在树里；它的 `accessible-label` 与"权威工程的投影"逐字相等 |
/// | 5b | **同案两帧差分**（`ADR-0005` S1，不碰 golden）：域侧改动前 / 后各抓一帧 | 两帧不同；差异包围盒落在被改轨道的**投影泳道带栈**内；区域外逐字节相同；指纹可复现 |
/// | 6 | 一次**只读**工具调用（`yeban_query_project`） | 修订号不动、`sync_authority = Unchanged` ⇒ 界面不是被查询刷来刷去 |
/// | 7 | `ui/tree` / `ui/node` / `ui/screenshot` 端到端 | 控制面服务的就是更新后的那一棵树（同一个窗口），且未遮罩截图与第 5b 步那一帧指纹相同 |
///
/// ## 这条判据怎么变红（负向实测见工作线报告）
///
/// - 让 `Domain::apply` 不推进 `apply_revision`（或 `Plan::mutates_project` 对
///   `EditAutomation` 恒为 `false`）⇒ 第 4 步变成 `Unchanged`，第 5 步的新元素不在树里；
/// - 让 `sync_authority` 拿**装配时**那一份工程（而不是每次从权威取）⇒ 同样在第 5 步变红；
/// - 把 `host::apply_view` 的注入摘掉 ⇒ 两帧**逐字节相同**，第 5b 步的差分断言先变红
///   （比第 5 步更早、更直接地指出"界面没跟着权威动"）；
/// - 让改动**溅到别的轨道**（例如同一次运行里再改一条别的轨道的泳道）⇒ 差异包围盒越出
///   被改轨道的投影带栈，第 5b 步变红。
#[cfg(feature = "in-process-mcp")]
#[test]
fn an_mcp_mutation_reaches_the_live_ui_projection_through_the_single_authority() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

// ---------------------------------------------------------------------------
// 判据 17b：差分**容差**的上界（`ADR-0005` S1 的"容差本身要被判据钉住"）
// ---------------------------------------------------------------------------

/// 把 [`LANE_DIFF_TOLERANCE_PX`] 放大（例如放到全屏）就必须在这里变红；把上界
/// [`LANE_DIFF_TOLERANCE_MAX_PX`] 也放大去迁就它，同样在这里变红。
///
/// 两条断言都走**局部绑定**而不是直接比较两个常量字面量：`clippy::assertions_on_constants`
/// 会把后者当成"恒真的断言"而拒绝（`-D warnings`）。
#[cfg(feature = "in-process-mcp")]
#[test]
fn the_lane_diff_tolerance_stays_bounded() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

// ---------------------------------------------------------------------------
// 判据 18 / 19（`ROAD-M4-008` 选项 (a) **第二片**）：GUI 的写入口落到唯一可变权威上
// ---------------------------------------------------------------------------

/// 工程里**全部**音符身份（`clip_pool` 键序；与 `bridge::ViewState` 的枚举同一来源）。
#[cfg(feature = "in-process-mcp")]
fn project_note_ids(project: &YebanProjectV1) -> Vec<String> {
    let mut ids = Vec::new();
    for entry in project.clip_pool.values() {
        let Some(notes) = entry.content.notes() else {
            continue;
        };
        ids.extend(notes.keys().map(|id| id.to_canonical_string()));
    }
    ids
}

/// 判据 18：**GUI 的撤销与环回会话的撤销改的是同一个 `Domain`**，因此两条写路径在
/// **同一个投影**上汇合。
///
/// ## 这条判据要证的两句话
///
/// 1. GUI 的写入口（`UndoPort`）**不再持有**自己的 `UndoSession`：它由
///    `UndoPort::from_authority` 构造，只握 `ProjectAuthorityHandle`；因此
///    第 3 步的 GUI 撤销必须推进**同一个**权威的施加修订号（`r+1 → r+2`）。
/// 2. 会话侧的写入（`yeban_edit_automation` / `yeban_redo`）与 GUI 侧的写入
///    **互相可见**：同一棵运行时控件树按同一条 `sync_authority` 重投影。
///
/// ## 论证顺序（每一步都用上一步的实读值）
///
/// | 步 | 谁写 | 读回 |
/// | :--- | :--- | :--- |
/// | 1 | 以权威装配真实界面 + 由**同一个**句柄构造端口 | 起点：`track-0-automation-pan-lane` 不在树里；端口显示态 == 权威显示态 |
/// | 2 | **会话侧**：真客户端 + 真令牌发 `yeban_edit_automation`（`TrackPan` 写一个点） | 修订号 `r → r+1`；`sync_authority = Reprojected`；泳道进树 |
/// | 3 | **GUI 侧**：`host::wire_undo` 接的真实 Slint 回调 `undo-step` | 修订号 `r+1 → r+2`（同一个权威！）；端口的 `undone == 1` |
/// | 4 | 投影跟随 | `sync_authority = Reprojected`；泳道离开运行时树 |
/// | 5 | **会话侧**：`yeban_redo` | 修订号 `r+2 → r+3`；同一条泳道回到同一投影 |
///
/// ## 怎么变红（负向实测见工作线报告）
///
/// 把 `UndoPort::from_authority(authority.clone())` 换回 `UndoPort::new(<另一份 UndoSession>)`
/// ⇒ 第 3 步的修订号停在 `r+1`（GUI 写的是**另一个**会话），第 4 步 `Unchanged`、泳道仍在树里。
#[cfg(feature = "in-process-mcp")]
#[test]
fn a_gui_action_and_a_session_action_share_one_projection_through_the_authority() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据 19：**GUI 的卷帘铅笔编辑进的是同一个权威** —— 会话侧一次 `yeban_undo` 能把它撤掉，
/// 同一棵运行时控件树随之更新。
///
/// ## 论证顺序
///
/// | 步 | 动作 | 读回 |
/// | :--- | :--- | :--- |
/// | 1 | 以权威装配真实界面；`wire_roll_edit` 接上**同一个**句柄构造的端口 | 起点音符身份集 |
/// | 2 | 用**与宿主同一条解析**（`host::pencil_op_for`）找一个真落在片段内的点击位置 | `Some(AddNote)` |
/// | 3 | **GUI 事件源**：`app.slint` 的 `clicked` 回调 → `wire_roll_edit` | 权威工程多**恰好一个**音符；修订号 `r → r+1` |
/// | 4 | `sync_authority` + 运行时树 | 新音符 `note-{ulid}-rect` 在树里 |
/// | 5 | **会话侧**：真环回 socket 发 `yeban_undo` | 音符身份集回到起点；新元素离开树 |
///
/// ## 怎么变红
///
/// 把 `wire_roll_edit` 里的 `port.commit_ops(..)` 摘掉（或让端口改回本地会话）
/// ⇒ 第 3 步权威工程里音符数不变 ⇒ 本判据立刻红。
#[cfg(feature = "in-process-mcp")]
#[test]
fn a_gui_pencil_edit_lands_in_the_same_authority_as_the_session() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// `[length]` 属性 → `Vec<f32>`（本文件只读注入面；生产侧的读回在 `host::read_lengths`）。
fn injected_lengths(model: &slint::ModelRc<f32>) -> Vec<f32> {
    use slint::Model as _;
    model.iter().collect()
}

/// `[string]` 属性 → `Vec<String>`（本文件只读注入面；生产侧的读回在 `host::read_strings`）。
fn injected_strings(model: &slint::ModelRc<slint::SharedString>) -> Vec<String> {
    use slint::Model as _;
    model.iter().map(|value| value.to_string()).collect()
}

/// 窗口上那 7 个行几何数组的快照（顺序 = `host::apply_row_geometry` 的写入顺序）。
fn row_geometry_snapshot(window: &yeban_app::ui::MainWindow) -> (Vec<Vec<f32>>, Vec<String>) {
    (
        vec![
            injected_lengths(&window.get_track_ys()),
            injected_lengths(&window.get_track_heights()),
            injected_lengths(&window.get_clip_ys()),
            injected_lengths(&window.get_clip_heights()),
            injected_lengths(&window.get_automation_lane_band_ys()),
            injected_lengths(&window.get_automation_lane_band_heights()),
        ],
        injected_strings(&window.get_automation_path_commands()),
    )
}

/// **投影**里同样那 7 个字段的快照（本判据的独立基准；不是"另一个注入写者"）。
fn projection_geometry_snapshot(view: &ViewState) -> (Vec<Vec<f32>>, Vec<String>) {
    (
        vec![
            view.track_ys(),
            view.track_heights(),
            view.clip_ys(),
            view.clip_heights(),
            view.automation_lane_band_ys(),
            view.automation_lane_band_heights(),
        ],
        view.automation_path_commands(),
    )
}

/// 轨道头 `track-0-header` 在**运行时树**里的几何原点与行高（窗口逻辑坐标）。
///
/// 判据用它取注入坐标，而不是写死像素 —— 侧栏宽度 / 顶栏高度一改，坐标跟着走。
fn header_box(plane: &mut LiveControlPlane, id: i64) -> (f64, f64, f64) {
    let bounds = header_bounds(plane, id);
    (
        bounds["x"].as_f64().expect("x"),
        bounds["y"].as_f64().expect("y"),
        bounds["height"].as_f64().expect("height"),
    )
}

/// 同 [`header_box`]，但给宽度（日志里用）。
fn header_box_width(plane: &mut LiveControlPlane, id: i64) -> f64 {
    header_bounds(plane, id)["width"].as_f64().expect("width")
}

/// `ui/node` 读 `track-0-header` 的 `bounds` —— 没有它（元素不在树里 / 没有几何）就**响亮失败**。
fn header_bounds(plane: &mut LiveControlPlane, id: i64) -> serde_json::Value {
    let node = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        id,
        "ui/node",
        Some(serde_json::json!({"elementId": "track-0-header"})),
    ));
    assert!(!node.is_error(), "轨道头必须在运行时树里: {node:?}");
    let bounds = node.result.expect("有 result")["node"]["bounds"].clone();
    assert!(!bounds.is_null(), "轨道头必须有几何包围盒");
    bounds
}

/// `tests/undo_wiring_ui.rs` 用的同一个时间戳夹具（会话打开时刻）。
const TRACK_HEIGHT_DRAG_NOW: u64 = 1_760_000_000_000;

/// 拖拽手势判据的装配：真实界面 + **同一份工程**的撤销端口 + 生产形态的两条接线。
///
/// 返回 `(window, port, plane)`：
/// - `window` 是活窗口的强引用（`clone_strong` —— 与执行面持有的是**同一个**组件实例），
///   判据用它读注入面（`track-ys` / `track-heights` / `track-height-*`）与直接驱动回调；
/// - `plane` 是同一个窗口上的控制面（注入真实指针 / 键盘事件）。
///
/// 两条接线都是**产品路径上的既有调用**（`src/main.rs` 里逐字相同）：
/// `host::wire_track_height_drag`（手势本体）与 `host::wire_keys`（`Escape` 的取消路径
/// 需要用端口把几何重新投影回起点）。`build_live_ui` 内部已经用 `None` 接过一次
/// `wire_keys`；这里按**生产形态**（`Some(port)`）再接一次 —— 两次接的是同一个
/// `InputContext`，后一次覆盖前一次（Slint 的回调 setter 是覆盖语义）。
fn assemble_track_height_drag() -> (
    yeban_app::ui::MainWindow,
    std::rc::Rc<yeban_app::undo::UndoPort>,
    LiveControlPlane,
) {
    use std::rc::Rc;

    use yeban_app::undo::{UndoPort, UndoSession};

    let project = yeban_model::samples::filled_project();
    let ui = build_live_ui(&project, Permission::Interactive).expect("装配");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    // 端口持有的是**同一份**工程（`filled_project()` 的身份是固定夹具 ⇒ 身份逐位相同）。
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据>", "yeban-app", project, TRACK_HEIGHT_DRAG_NOW).expect("打开"),
    ));
    yeban_app::host::wire_track_height_drag(&window, &port);
    yeban_app::host::wire_keys(
        &window,
        ui.input_context(),
        Some(Rc::clone(&port)),
        None,
        None,
    );
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, port, plane)
}

/// 判据（`ADR-0004` **S1**）：轨道高度的**视图态**真的走到了注入的行几何上 ——
/// 即"每轨高度 + 全局乘子"这条能力**在真实窗口上有消费者**，而不是一个没人调用的参数。
///
/// 链路（每一步都是既有实现，本判据不新增测试专用路径）：
///
/// ```text
/// MainWindow 的 track-height-* 三个属性        ← host::set_track_height_override / _percent
///   └─ host::track_height_layout（唯一读点）
///        └─ ViewState::from_project_with_layout（唯一的 clamp：base × percent / 100，再夹紧）
///             └─ host::apply_view → track-ys / track-heights
///                  └─ 活窗口上的 ModelRc（本判据直接读）
/// ```
///
/// 驱动入口是**既有的**重投影点 `LiveUi::apply_project`（不是为判据新开的口子）：
/// 它读回窗口上的布局再投影 —— 若不读回来，"设置行高之后再重投影"会把高度静默清零，
/// 这条判据就会红。
///
/// 行高的**取值范围**是 `[16, 320]`（`MIN/MAX_TRACK_HEIGHT_PX`，`ADR-0004` Q2 点名常量、
/// 未给数字 ⇒ 本仓登记的工程选择；理由见常量文档）。这里取的 120/150% 落在区间内，
/// 因此断言的是"公式"，不是夹紧。
#[test]
fn track_height_view_state_reaches_the_injected_row_geometry() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据（`ADR-0004` **S1 的纵向入口**）：轨道头**拖拽手势**真的改行高，而且收尾干净。
///
/// ## 为什么这条判据用 `ui/dispatch_pointer_*` 而不是新加一个 `ui/set_track_height`
///
/// §12.4 的两个注入方法**已经**在方法表里（`ui/dispatch_pointer_*`），而它们驱动的是与用户
/// **一模一样**的事件源：真实指针事件 → Slint 命中测试 → `.slint` 的轨道头 `TouchArea` →
/// `host::wire_track_height_drag`。于是这条判据证明的是"**手势真的能用**"，
/// 比"存在一个把数字写进去的设置器"更强（后者证明不了 `.slint` 那一半接上了）。
///
/// ## 三条会变红的注入（每条都实测过）
///
/// | 注入 | 位置 | 现象 |
/// | :--- | :--- | :--- |
/// | 摘掉 `.slint` 里的 `track-height-grab/drag/release` 转发 | `arrangement_view.slint` | 指针注入之后行几何一位不变（本判据第一段红） |
/// | 摘掉 `host::wire_track_height_drag` 里的 `set_track_height_override` | `src/host.rs` | 同上 |
/// | 摘掉 `end_track_height_drag` 里的 `set_track_height_drag_active(false)` | `src/host.rs` | 收尾断言红（粘住的拖拽态） |
#[test]
fn the_track_header_drag_gesture_changes_the_row_geometry_and_ends_cleanly() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

/// 判据（`ADR-0004` S1 的纵向入口）：`Escape` **取消**这一拖（写回按下时的高度）并收尾。
///
/// 三条收尾路径里，`Escape` 是唯一"取消"语义的那条（松手与指针离开窗口都是"算数"）。
/// 它与另外两条共用 `host::end_track_height_drag`（手势状态的唯一清零点），
/// 但多一步"把基准高写回起点 + 重投影" —— 少了那一步，用户看到的是"按了 Esc 没反应"。
///
/// ## 怎么变红
///
/// 把 `apply_action` 里的 `Action::Cancel => cancel_track_height_drag(ui, undo)` 摘掉：
/// `Escape` 回到"未实现 ⇒ 放行"的老行为，行高**留在 116px** ⇒ 本判据红。
#[test]
fn escape_cancels_the_track_height_drag_and_restores_the_starting_height() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

/// 判据（`ADR-0004` S1 的纵向入口）：**孤立的拖拽事件一位都改不动几何** ——
/// 这是"粘住的拖拽态"那一类缺陷的机械防线。
///
/// 四种形态（每一种都对应一个真实的失败模式）：
///
/// | # | 形态 | 真实的失败模式 |
/// | :--- | :--- | :--- |
/// | ① | 没有按下的 `move` | 松在窗口外 / 事件丢失之后，下一次移动继续改高度 |
/// | ② | 没有手势的 `release` | 收尾代码把"没有手势"当成"手势结束"并写坏状态 |
/// | ③ | 下标越界的 `grab` | 索引越界时猜第一条轨道（高度被改到别的轨上） |
/// | ④ | 手势进行中、**另一个下标**发来的 `move` | 多指 / 事件串扰把高度改到别的轨上 |
#[test]
fn a_stray_drag_event_without_a_grab_never_touches_the_row_geometry() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据（`ADR-0004` S1 的**控制面入口**）：`ui/set_track_height` 真的改行高，且**两个观测面**
/// 都能回读出来。
///
/// ## 为什么这条判据有两个观测面
///
/// | # | 观测面 | 读法 | 证明的是 |
/// | :--- | :--- | :--- | :--- |
/// | 1 | 注入的投影数组 | `MainWindow.get_track_heights()` | 视图态 → 投影 → 界面的那条链真的走到了 |
/// | 2 | 控制面**自己的**运行时树 | `ui/node` 的 `bounds.height` | 执行面在改完之后**重抓过树**（否则 `ui/tree` 会一直卖旧几何给 AI） |
///
/// ## 顺带钉住的口径（每条一行断言）
///
/// - `heightPx: 0` = **取消覆盖**（回到默认基准），与 `host::set_track_height_override` 逐字相同；
/// - 幂等：同一个值再来一次 ⇒ 回执里 `changed: false`，几何一位不动；
/// - `dryRun: true` ⇒ 预览给出"将要写进去的基准高"，而**状态一位不变**；
/// - **非法参数用既有错误码**（D25）：负数是 `-32602 INVALID_PARAMS`、不是轨道头是 `-32006`；
/// - 权限层：`ui/methods` 里它的 `requiredScope` 是 `ui:inject`（`Interactive` 层）。
#[test]
fn the_set_track_height_method_reaches_the_injected_row_geometry() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据（`ADR-0004` S0/S1 的**接线纪律**）：两条注入路径写进界面的行几何**逐位等于投影**。
///
/// ## 判据用的是什么"尺"（这是本判据有牙的关键）
///
/// 它拿 **`ViewState`（投影本身）** 当独立基准，而不是拿"另一条注入路径"当基准。
/// 理由是一次真测得来的：我最初写成"窄注入 == 完整注入"，注入验证（从
/// `host::apply_row_geometry` 里删掉 `set_clip_ys`）**没有让它变红** —— 因为
/// `apply_view` 调用的就是 `apply_row_geometry`，两条路一起漏 ⇒ 两边相等。
/// 那就是一条**空转的判据**。
///
/// 现在两条路各自与投影对账：
///
/// | 注入路径 | 谁用 | 断言 |
/// | :--- | :--- | :--- |
/// | `host::apply_row_geometry`（窄，7 个数组） | 拖拽手势的每一次 move | 窗口上的 7 个数组 == `ViewState` 的对应字段 |
/// | `host::apply_view`（完整） | 换工程 / 撤销 / 打开工程 | 同上 |
///
/// 于是"漏掉任何一个行几何数组"（无论在哪条路里）都会红。
///
/// ## 怎么变红（实测）
///
/// 从 `apply_row_geometry` 里删掉 `set_clip_ys` / `set_clip_heights` ⇒ 窗口上的
/// `clip-ys` 停在旧值、投影给了新值 ⇒ 本判据红。
#[test]
fn the_row_geometry_writers_carry_the_projections_geometry_bit_for_bit() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据（`ADR-0004` S0/S1）：**默认外观一位不变** —— 没有人碰过高度时，注入的行几何逐字段
/// 等于 S0 的字面量，而且拖拽手势的四个状态位全是默认值。
///
/// 这条是"像素影响"的可判据那一半（另一half 是渲染帧 sha256 的 A/B，见交付报告）：
/// 新的 `TouchArea` **不画任何东西**，新的属性与回调**不进像素**，因此默认帧的输入
/// （`track-ys` / `track-heights` / `clip-ys` / `clip-heights` / 自动化带与折线）
/// 必须与加手势之前**逐位相同**。
#[test]
fn the_default_row_geometry_is_bit_for_bit_the_s0_literals() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据 18（**产品形态**）：`--enable-ui-mcp-http` 走的那一条装配路径
/// （`LiveUi::into_production_control_plane` + `LiveControlPlane::into_service`）。
///
/// 为什么这条判据必须存在（而不是"产品代码写好了就行"）：
///
/// 1. `src/live_surface.rs` 同时被产品库与**本测试目标**编译（`#[path]`）。这两个方法
///    只被产品路径调用 ⇒ 少了本判据，它们在测试目标的编译里是 `dead_code`
///    （本仓库 `-D warnings` ⇒ 硬错误），而且"产品形态真的按生产模式装配"这句话
///    就没有任何会变红的证据；
/// 2. 产品形态与判据形态的**唯一**差别就是运行模式：判据用 `RunMode::Test` 才能注入事件，
///    产品必须用 `RunMode::Production`（`ui:inject` 族硬禁）。这条判据把那个差别钉住：
///    在**同一条**真实执行面上，注入被硬拒而管理族照常授权。
#[test]
fn production_plane_denies_injection_and_really_switches_the_live_view() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据 18b（**跨 crate 字面值对账**）：`yeban-app::cli` 里那三个"为了默认构建也能打用法
/// 文本"而保留的副本，必须与 `yeban-ui-mcp` 的权威定义**逐字节相同**。
///
/// 为什么必须有这条：字面值有两份是实现的需要（默认构建里 `yeban-ui-mcp` 只是
/// dev-dependency，`usage_text()` 却必须打得出来），而"两份不许漂移"只能靠判据 ——
/// 与 `tests/in_process_mcp.rs` 对 `--enable-mcp-http` / `YEBAN_MCP_HTTP` 做的是同一件事。
/// 端点路径那一条还有第二个理由：`ui/*` 与领域 MCP 接**不同**的路径，
/// 接错端口时人应当能一眼看出来（`/ui-mcp` vs `/mcp`）。
#[test]
fn the_ui_mcp_http_switch_literals_match_the_ui_mcp_crate() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

// ===========================================================================
// 删除选区（`Delete`/`Backspace`）：真实事件源 → 一次可撤销提交
// ===========================================================================

/// 运行树里的**真**音符身份：`note-{26 字符 ULID}-rect` 的 `{ulid}` 段。
///
/// 不能只按 `note-` / `-rect` 前后缀取：任何同形的前缀/后缀节点都会被误收
/// （历史读数：2026-10-30 之前那块静态 AI 建议层 `note-suggestion-overlay-rect`
/// 就满足那对前后缀 —— 一棵 85 节点的树里 5 个候选、其中 1 个是叠加层；
/// 该层已按诚实性纪律删除，但这条**形态过滤**仍按 ULID 形状取，⛔ 不退回前后缀）。
fn note_ulids_in_tree(tree: &ControlTree) -> Vec<String> {
    tree.with_prefix("note-")
        .filter_map(|node| {
            node.id
                .strip_prefix("note-")
                .and_then(|rest| rest.strip_suffix("-rect"))
                .map(str::to_owned)
        })
        .filter(|ulid| ulid.len() == 26 && ulid.chars().all(|c| c.is_ascii_alphanumeric()))
        .collect()
}

/// 音符族规模 = [`note_ulids_in_tree`] 的条目数（**条目**, 不是行）。
fn note_rects(tree: &ControlTree) -> usize {
    note_ulids_in_tree(tree).len()
}

/// 工程里的音符**条目**数（片段池键序 → 音符键序；与投影同源）。
fn model_note_count(project: &YebanProjectV1) -> usize {
    project
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .map(std::collections::BTreeMap::len)
        .sum()
}

/// 判据 D1（删除选区, **默认构建**）：`Backspace` 经真实事件源删除当前选区,
/// 且**一次撤销整批回来**。
///
/// ## 三行读数（全部来自活窗口）
///
/// | 步 | 动作 | 读数 |
/// | :--- | :--- | :--- |
/// | ① | 注入前 | 运行时树里的音符元素数 = N |
/// | ② | 注入 `Backspace` | N − k（k = 选区大小） |
/// | ③ | `Cmd+Z` | 回到 N |
///
/// ## 怎么变红
///
/// 把 `apply_action` 的 `DeleteSelection` 分支摘掉（或让按键被吞却不提交）⇒ ② 步读数不变。
/// 把整批拆成 k 次 `commit_ops`（非原子）⇒ ③ 步只回来 1 个。
#[test]
fn backspace_deletes_the_selection_through_the_event_source_and_one_undo_restores_all() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

// ===========================================================================
// 复制选区（`Cmd`/`Ctrl+D`）：真实事件源 → 一次可撤销提交（右移一个吸附网格）
// ===========================================================================

/// 判据 D3（复制选区, **默认构建**）：`Cmd+D` 经真实事件源把选区里的每个音符
/// **复制到下一格**（右移一个吸附网格）, 副本拿**全新身份**; 且**一次撤销整批撤掉**;
/// 空选区**不消费**（`reject`）且一位不变。
///
/// ## 量什么 / 怎么量 / 单位
///
/// | 量 | 怎么量 | 单位 |
/// | :--- | :--- | :--- |
/// | 音符条目数 | [`model_note_count`]（**模型**侧 oracle, 走 `clip_pool`） | 条目 |
/// | 音符元素数 | [`note_rects`]（运行时控制树的 `note-{ulid}-rect`） | 条目 |
/// | 副本位置 | `(start_tick, pitch, id)` 三元组, 从 `port.project()` 读 | tick / 半音 / 身份 |
/// | 键是否被消费 | `invoke_key_action` 的**返回值**（`true` = `accept`） | 布尔 |
/// | 提交数 | `port.display().commit_count`（`CommitGraph` 的模型读数） | 次 |
/// | 画面变化 | [`frame_diff`] 的差异**像素数** + 包围盒（逐像素比 RGB） | 像素 |
///
/// ## 算术（全部整数；期望值在这里手算, **不引用被测函数**）
///
/// - 夹具（`filled_project`）：**4** 个音符, 起点 `0 / 960 / 1920 / 2880`,
///   音高 `60 / 64 / 67 / 72`, 时值各 `480`。
///   出处：`crates/yeban-model/src/samples.rs` 的 `filled_project`。
/// - 吸附网格 = **240** tick（1/16）—— 与 `host::ROLL_SNAP_GRID_TICKS` 同值, 本文件写成
///   字面量当独立 oracle。
/// - 全选（`k = 4`）⇒ 复制后 `N + k = 4 + 4 = 8` 条。
/// - 副本起点 = 原文 + 240 ⇒ `240 / 1200 / 2160 / 3120`; 音高**不变**（`60 / 64 / 67 / 72`）。
///   两个 tick 集合**不相交** ⇒ 副本与原文可区分。
/// - 一次 `Cmd+Z` ⇒ 回到 `4` 条 ⇒ 整批是**一个** `Op::Batch`（`ARCH-OPS-002`）。
/// - 一次 `Cmd+D` ⇒ `commit_count` 恰好 `+1`（不是 `+k`）。
///
/// ## 怎么变红（注入实测, 见本提交的说明）
///
/// | 注入 | 位置 | 现象 |
/// | :--- | :--- | :--- |
/// | 把 `Duplicate` 放回 `action_has_implementation` 的 `!matches!` | `src/host.rs` | ② 步音符数不变（键被 `reject`）, `cli_contract.rs` 的 B11e 也红 |
/// | 让副本复用原文的 `id` | `src/host.rs` 的 `duplicate_ops_for` | ② 步为 0：`AddNote::precondition` 以 `DuplicateEntityId` 拒绝整批 |
/// | 把整批拆成 k 次 `commit_ops` | `src/host.rs` 的 `Duplicate` 分支 | ③ 步只回来 1 条, 且 ② 步 `commit_count` 变成 `+k` |
#[test]
fn cmd_d_duplicates_the_selection_one_grid_to_the_right_and_one_undo_restores_all() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

/// 判据 D2（删除选区, `in-process-mcp`）：同一次按键的删除落在**唯一权威**上,
/// 且 `Cmd+Z` 从**同一个**权威回来（`ROAD-M4-008` 选项 (a)）。
///
/// 与判据 D1 的差别是**结构性的**：这里没有本地会话, 工程的唯一来源是控制面 `Domain`；
/// 因此"删除进的是同一个会话"由 `apply_revision` 的前进来见证, 而不是由本地端口自证。
#[cfg(feature = "in-process-mcp")]
#[test]
fn a_gui_delete_lands_in_the_same_authority_and_one_undo_restores_all() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

// =====================================================================================
// 纯视图态的四条回调（本切片）：`toggle-view` / `toggle-sidebar` / `toggle-ai-drawer`
// / `open-musical-pr` —— 判据证明"入口真的能用"，不是"控件存在"。
//
// 两半，缺一不可（两条 `#[test]`，因为 Tier-1 平台是线程局部的：
// `slint::platform::set_platform` 每个测试各装一次，同一个测试里不能装第二遍）：
//
// | 判据 | 回答的问题 |
// | :--- | :--- |
// | [`the_view_state_callbacks_really_change_the_view_state`] | **真实点击**（§12.4 指针注入）⇒ 视图态属性真的翻了 |
// | [`the_view_state_properties_drive_the_runtime_tree`] | 属性 ⇒ **运行时控件树**里 `visible` 真的换了（`ui/tree` 的元素数与互斥画布） |
// =====================================================================================

/// 一条视图态判据的装配：真实界面 + 可注入的控制面（`Interactive` 才允许指针注入）。
///
/// **不需要额外接线**：装配路径（`src/live_surface.rs` 的 `build_live_ui_with`）与产品进程
/// 调的是**同一个** `host::wire_view_callbacks`（`src/main.rs` 的 `wire_callbacks` 逐字相同），
/// 因此这里注入的点击与用户点击走同一条链。
fn assemble_view_state_callbacks() -> (yeban_app::ui::MainWindow, LiveControlPlane) {
    let project = yeban_model::samples::filled_project();
    let ui = build_live_ui(&project, Permission::Interactive).expect("装配");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, plane)
}

/// 一个语义元素的**绝对**包围盒（经 `ui/node` 读，不写死像素 —— 布局一改坐标跟着走）。
fn element_bounds(plane: &mut LiveControlPlane, seq: i64, element_id: &str) -> serde_json::Value {
    let node = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        seq,
        "ui/node",
        Some(serde_json::json!({ "elementId": element_id })),
    ));
    assert!(
        !node.is_error(),
        "`{element_id}` 必须在运行时树里: {node:?}"
    );
    let bounds = node.result.expect("有 result")["node"]["bounds"].clone();
    assert!(!bounds.is_null(), "`{element_id}` 必须有几何包围盒");
    bounds
}

/// 一次**真实**点击：按下 → 移到同一点 → 松手。
///
/// 三步都走 §12.4 的注入方法（真实指针事件 → Slint 命中测试 → `.slint` 的 `TouchArea`），
/// 因此它命中的是**用户点的那一下**，而不是"直接调 Rust 回调"
/// （与 `the_track_header_drag_gesture_changes_the_row_geometry_and_ends_cleanly` 同手法）。
fn click_element(plane: &mut LiveControlPlane, seq: i64, element_id: &str) {
    use serde_json::json;
    use yeban_ui_mcp::methods::{METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE};

    let bounds = element_bounds(plane, seq, element_id);
    let x_offset = bounds["width"].as_f64().expect("width") / 2.0;
    let y_offset = bounds["height"].as_f64().expect("height") / 2.0;
    let absolute_x = bounds["x"].as_f64().expect("x") + x_offset;
    let absolute_y = bounds["y"].as_f64().expect("y") + y_offset;

    let down = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        seq + 1,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": element_id,
            "xOffset": x_offset,
            "yOffset": y_offset,
            "button": "left"
        })),
    ));
    assert!(
        !down.is_error(),
        "`{element_id}` 按下必须落到真实窗口: {down:?}"
    );
    let moved = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        seq + 2,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({ "x": absolute_x, "y": absolute_y })),
    ));
    assert!(
        !moved.is_error(),
        "`{element_id}` 移动必须落到真实窗口: {moved:?}"
    );
    let up = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        seq + 3,
        "ui/dispatch_pointer_up",
        Some(json!({ "button": "left" })),
    ));
    assert!(
        !up.is_error(),
        "`{element_id}` 松手必须落到真实窗口: {up:?}"
    );
}

/// `ui/property` 读一个元素的属性（读数一律经控制面）。
fn read_property(
    plane: &mut LiveControlPlane,
    seq: i64,
    element_id: &str,
    name: &str,
) -> serde_json::Value {
    let line = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        seq,
        "ui/property",
        Some(serde_json::json!({ "elementId": element_id, "name": name })),
    ));
    assert!(!line.is_error(), "`{element_id}.{name}` 必须可读: {line:?}");
    line.result.expect("有 result")["value"].clone()
}

/// 一条视图态属性在**活窗口**上的字面读数（与判据 16 读 `arrangement-view` 同一手法）。
fn view_state_reading(window: &yeban_app::ui::MainWindow) -> String {
    format!(
        "arrangement-view={} sidebar-collapsed={} ai-drawer-open={} musical-pr-open={}",
        window.get_arrangement_view(),
        window.get_sidebar_collapsed(),
        window.get_ai_drawer_open(),
        window.get_musical_pr_open(),
    )
}

/// 判据（`ADR-0004` S1 同款口径）：**四条纯视图态回调真的被真实点击驱动**。
///
/// ## 每一段都钉住"注入前 ≠ 注入后"的字面读数
///
/// | 段 | 注入的元素 | 读回来的东西 | 期望 |
/// | :--- | :--- | :--- | :--- |
/// | `toggle-view` | `transport-view-toggle-button` | `arrangement-view` + 帧指纹 | `true` → `false`；指纹必须变 |
/// | `toggle-sidebar` | `sidebar-collapse-button` | `ui/property` 读 `sidebar.width` + `sidebar-collapsed` | 240.00 → 36.00 |
/// | `toggle-ai-drawer` | `transport-ai-drawer-button` | `ai-drawer-open` + 帧指纹（`compact=true`） | `false` → `true`；指纹必须变 |
/// | `open-musical-pr` | `ai-rail-musical-pr-button` | `musical-pr-open` + 帧指纹 | `false` → `true`；指纹必须变 |
///
/// ## 两条会变红的注入（都实测过，见账本当轮条目）
///
/// | 注入 | 位置 | 现象 |
/// | :--- | :--- | :--- |
/// | 摘掉 `host::wire_view_callbacks(&window);` | `src/live_surface.rs` | 四段的"注入后"读数与"注入前"逐字相同 ⇒ 四段一起红 |
/// | 把 `root.arrangement-view = !root.arrangement-view;` 加回转发块 | `ui/app.slint` | `.slint` 与宿主各翻转一次 ⇒ 互相抵消 ⇒ `toggle-view` 段红 |
///
/// ## 快照语义**已经取消**（本轮更正 `Round 431` 登记的那条边界）
///
/// `Round 431` 在这里写过一条"如实报告的边界"：`ui/tree` 是**快照**、`LiveAdminSurface`
/// 只在三个动作点重抓树 ⇒"点击改了可见性不会让下一次 `ui/tree` 变新"（那一轮把
/// `树节点 85→85` 原样打了出来）。**那条边界现在不成立了**：读方法在返回前会调
/// `UiSurface::refresh_runtime_tree`（`crates/yeban-ui-mcp/src/service.rs` 的
/// `refresh_before_read`），而 `LiveAdminSurface` 覆写它去重抓活窗口的树。因此判据 1
/// 的 `树节点 N→N` 读数**已经换成**真的变化（见判据
/// `a_tree_read_sees_a_click_that_just_happened` 的字面读数：点击 `toggle-sidebar` ⇒
/// 节点 **85→81**、`sidebar-search-field` 离开树、折叠按钮 `x` **210→6**）。
///
/// ⛔ 这里仍然**不**断言"两次读之间树必须不变"：那是把某一种缓存语义钉成契约。
/// 钉住的是**新鲜度** —— 读必须看见刚发生的变化。
///
/// ## 为什么不断言像素
///
/// 这四条回调**只有点击之后**才改属性；默认外观（未注入任何点击）的帧必须逐字节不变，
/// 那一条是渲染帧 sha256 的 A/B（`live-port-filled-project-unmasked-1920x1080.png`，
/// 6,222,418 字节），见账本当轮条目的"像素影响"。
#[test]
fn the_view_state_callbacks_really_change_the_view_state() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据：**属性 ⇒ 运行时控件树**（`visible` 真的跟着那四个属性走）。
///
/// 这一条与上一条合起来才是完整的链：**真实点击 ⇒ 属性 ⇒ 可见性 ⇒ 树**。
/// 上一条证明前半段（点击 ⇒ 属性），这一条证明后半段（属性 ⇒ 树）。
///
/// ## 它为什么不是"自己写进去再自己读回来"的空转
///
/// 断言的**方向**是 `.slint` 的 `visible:` 绑定，不是宿主的能力：
/// `ArrangementView` / `SessionView` 的 `visible` 互斥、两个 `ai-rail` 实例的
/// `visible: root.compact && root.ai-drawer-open`、`MusicalPrDrawer` 的
/// `visible: root.musical-pr-open`。把任何一处 `visible:` 写坏（或改成常量）⇒ 本条红。
///
/// ## 树为什么要显式重抓
///
/// `ui/tree` 是快照，重抓点是既有的几个动作（见上一条判据的边界说明）。
/// 这里用 `LiveUi::pump_meters()`（既有的"电平轮询腿"：抽干 → 注入 → **重抓树**），
/// 因为它是**唯一**不需要 `UndoPort`、也不改工程的重抓触发点。
#[test]
fn the_view_state_properties_drive_the_runtime_tree() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

// =====================================================================================
// 判据：**抽屉不说谎** —— 没有提案的抽屉里没有编造的数据，只有一句用户可见的真话
//
// 这是"AI 提案抽屉"的诚实性契约的**运行时那一半**；文本层与"唯一注入路径"那一半在
// `crates/yeban-app/src/elements.rs` 的 `musical_pr_drawer_declares_no_demo_proposals`
// 与 `the_proposal_drawer_writes_its_readings_from_one_host_path`。两条线共用**同一份**假字面量
// （`yeban_app::elements::DEMO_PROPOSAL_LITERALS`），因此不存在"两份清单漂移"。
// =====================================================================================

/// 判据：没接权威的装配 ⇒ 运行时树里**零条**提案、零条编造置信度，且有一句可见的空态。
///
/// | 路径 | 量什么 | 期望 |
/// | :--- | :--- | :--- |
/// | 属性 → 树 | `musical-pr-drawer` 节点的 `value`（= `accessible-value` 原文） | `"0"` |
/// | 树（ID） | `musical-pr-proposal-*-item` 的条数 | `0` |
/// | 树（标签） | 任何节点的 `accessible-label` / `accessible-value` 命中 6 条假字面量 | `0` 处 |
/// | 树（空态） | `musical-pr-empty-state` 的 `label` | 逐字等于宿主注入的 `NO_SOURCE_REASON` |
/// | 树（假控件） | `musical-pr-accept-button` / `-reject-button` | 都不在树里 |
/// | 像素 | 打开抽屉前后两帧的差异像素数与包围盒 | 必须变（不碰 golden） |
///
/// **为什么 `value` 这一格是"属性 ⇒ 树"**：`accessible-value` 是抽屉从 `proposal-count`
/// 算出来的文本，运行时树读的是**活控件**（`crates/yeban-ui-test-port/src/inspect.rs:186`）。
/// 把 `proposal-count` 改回演示常量 ⇒ 这一格立刻变成 `"3"`。
///
/// ⚠ 本判据走的是 `build_live_ui`（**没有**进程内权威）⇒ 那句空态必须是
/// `NO_SOURCE_REASON`（"这个装配没有提案来源"）而不是 `NO_OPEN_PROPOSAL_REASON`
/// （"控制面此刻没有未合并的提案"）。两句话**必须**能分开，否则"看不见提案"与
/// "确实没有提案"在界面上就是同一件事 —— 那一句 `src/proposals.rs` 的单元判据
/// `empty_reason_distinguishes_no_source_from_no_proposal` 钉住，这里钉的是它在真树里的那一半。
#[test]
fn the_musical_pr_drawer_shows_an_honest_empty_state() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

/// 判据（**读面的运行时那一半**）：AI 经控制面提的一次案，抽屉里**真看得见**，
/// 而那一行的每一个读数都是从 `Domain.proposals` **逐字**来的。
///
/// 它与上一条 `the_musical_pr_drawer_shows_an_honest_empty_state` 是**同一枚硬币的两面**：
/// 上一条证"零提案时界面不许编"，这一条证"有提案时界面不许看不见"。两条缺一条，
/// "抽屉如实"这句就只是半句 —— 只写上一条的抽屉可以永远画 0 条而全绿。
///
/// ## 量什么（每一步都读**活窗口**，不是属性回显）
///
/// | 路径 | 量什么 | 期望 |
/// | :--- | :--- | :--- |
/// | 起点 | `authority.open_proposal_rows()` | 空（否则 0→1 不成立） |
/// | 回调 → 树 | `invoke_open_musical_pr()` 后 `musical-pr-empty-state` 的 `label` | 逐字 `NO_OPEN_PROPOSAL_REASON`（**有**来源那句，⛔ 不是 `NO_SOURCE_REASON`） |
/// | socket → 域 | 真客户端 `yeban_propose_section`（不 `autoMerge`） | `pendingMerge = true`；权威里恰好 1 条 `Open` |
/// | 域 → 树（条数） | `musical-pr-drawer` 的 `accessible-value` | `"1"` |
/// | 域 → 树（标题） | `musical-pr-proposal-0-item` 的 `label` | 逐字 `{kind}: {title}`（`title` 是领域原文） |
/// | 域 → 树（op 数） | `musical-pr-proposal-0-ops` 的 `label` | 逐字 `本提案携带 {op_count} 条 op` |
/// | 树（假控件） | `musical-pr-accept-button` / `-reject-button` / `musical-pr-empty-state` | 都不在树里 |
/// | 树（真话） | `musical-pr-resolve-unavailable` 的 `label` | 逐字 `NO_SELECTION_REASON`（有来源、**没点选** ⇒ 亮不起来那句） |
/// | 像素 | 0 条那一帧 vs 1 条那一帧 | 必须变，且差异伸进右起 360px 那条带 |
///
/// ## 为什么打开走 `invoke_open_musical_pr()` 而不是 `set_musical_pr_open(true)`
///
/// `set_musical_pr_open(true)` 只翻可见位，**绕过**宿主那条唯一读权威的路径
/// （`host::wire_proposal_drawer` 的 `on_open_musical_pr`）—— 那正是本判据要证的机制。
/// 因此这里**只**调回调：用户点徽章 / 点 AI 栏按钮落的就是它。
///
/// ## 这条怎么变红
///
/// - 把 `build_live_ui_from_authority` 的 `proposal_source` 覆盖摘掉 ⇒ 第二格仍是
///   `NO_SOURCE_REASON`、第四格仍是 `"0"`；
/// - 把 `wire_proposal_drawer` 里的"打开时重读"删掉（只在装配时读一次）⇒ 第四格停在 `"0"`；
/// - 让界面自己拼标题（不用领域的 `title`）⇒ 第五格逐字不等；
/// - 把 `resolve-available` 写成 `true` ⇒ 最后两格反向变红（假按钮回到树里、原因卡消失）。
#[cfg(feature = "in-process-mcp")]
#[test]
fn a_real_proposal_from_the_authority_is_visible_in_the_opened_drawer() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

// =====================================================================================
// 判据（任务单 #29 的运行时那一半）：点选一行 ⇒ 按钮亮起 ⇒「采纳」/`Shift+Enter`
// **真的**合并那条提案；被权威拒绝时 ⇒ 权威那句话逐字上屏且一位都不动
// =====================================================================================

/// 判据：在**接了权威的活窗口**上走完整条写路径 —— 点一行 ⇒ 亮起 ⇒ 采纳 ⇒ 领域真的变了
/// ⇒ 那一行消失；再走一遍**键盘**那一条（`Shift+Enter`）⇒ 同样真的合并。
///
/// ## 补的是哪一个洞（实测，不是转述）
///
/// 上一条判据只走到"看得见"。`resolve-available` 亮起、按钮按下之后**发生了什么**，
/// 全仓此前没有任何运行时判据量过：抽屉的写入口 `HostAction` 当时只有
/// `Undo` / `Redo` / `Commit` 三臂，领域侧的 `yeban_merge_proposal` 早就把合并做完了，
/// 而界面上那两个按钮按下去只关抽屉。今天两臂接上了（`MergeProposal` / `RejectProposal`
/// 复用**同一个** `Plan::Merge` / `Plan::Reject`），本判据钉的就是"接上之后不许漂移"。
///
/// 领域侧那九条（`crates/yeban-mcp/tests/host_proposal_resolve.rs`）量的是**动作本身**
/// （预览==真落、多父提交、幂等零步、拒绝码逐字相同）；本判据量的是**界面到动作那一段**
/// —— 选中身份、闸门、按钮/键盘两条落点、以及按下之后读数的去向。两边不重复，缺任一边
/// 都留着一个洞：只有领域侧 ⇒ 界面可能传错身份；只有本侧 ⇒ 合并语义本身可能漂移。
///
/// ## 量什么（每一步都是活窗口 / 唯一权威上的实读值）
///
/// | 步 | 量什么 | 怎么量 | 期望 |
/// | :--- | :--- | :--- | :--- |
/// | ① | 未合并提案条数 | `authority.open_proposal_rows()` | 提案前 0，提案后 1 |
/// | ② | 选中身份 | `MainWindow.musical-pr-selected-id` | 点选前 `""`，点选后 = 那一行的 `id` |
/// | ③ | 闸门 | `MainWindow.musical-pr-resolve-available` | `false` → `true` → 合并后回 `false` |
/// | ④ | 控件 | 树里 `musical-pr-accept-button` / `-reject-button` / `-resolve-unavailable` | 亮起时**两个按钮在、原因卡没有** |
/// | ⑤ | 领域 | `apply_revision` / `undo_display().commit_count` / `project()` | 采纳后修订号 +1、恰好 +1 条提交、工程变了 |
/// | ⑥ | 清单 | `open_proposal_rows()` / `proposal-count` / 树里的 `-item` | 采纳后 0 条 / `0` / 条目消失 |
/// | ⑦ | 夹紧 | `musical-pr-selected-id` | 合并后回 `""`（⛔ 不留陈旧身份） |
/// | ⑧ | 重投影 | `sync_authority()` | `Reprojected { revision }`（合并推进了修订号 ⇒ 界面跟得上） |
/// | ⑨ | 键盘 | `invoke_key_action("\n", shift=true)` | **`true`**（被消费）且领域 +1 条提交 |
/// | ⑩ | 负向对照 | 同一键在**没有点选**时 | `false` ⇒ ⑨ 不是"恒消费" |
///
/// ## 抽屉此刻**不关**（这是设计，不是遗漏）
///
/// 成功时唯一的反馈是"那一行从清单里没了"（重新读权威得到 0 条 ⇒ 空态那句话
/// `NO_OPEN_PROPOSAL_REASON` 回到屏上）。把抽屉关掉会把"还剩几条"这个读数一起关掉，
/// 而用户可能在决定下一条。
///
/// ## 这条怎么变红
///
/// - 「采纳」的落点退回"只关抽屉"（`on_accept_ai_proposal` 不再施加 `HostAction`）⇒
///   第⑤步三项读数全不变、第⑥步那一行还在；
/// - 选中身份写的是**下标**而不是领域的 `id` ⇒ 第②步不等（`apply_host` 会因非法身份
///   拒绝，第⑤步同样红）；
/// - 把闸门改成恒 `true` ⇒ 第④步在**未点选**时就红（假按钮进树）；
/// - 键盘臂不看前置条件 ⇒ 第⑩步红（没点选也消费）；
/// - 合并后不夹紧选中 ⇒ 第⑦步红（陈旧身份留着，下一次按下就打向一条不存在的提案）。
#[cfg(feature = "in-process-mcp")]
#[test]
fn selecting_a_row_lights_up_accept_and_it_really_merges_the_proposal() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

/// 判据：权威**拒绝**这一次采纳时，界面上屏的是权威那句**逐字**原话，抽屉不关、闸门打回
/// `false`、两个按钮退场，而且**一位都不动**。
///
/// ## 为什么要造一个"界面上的清单已经陈旧"的时机
///
/// 正常流程里被点的行必然在权威里是 `Open` ⇒ 不会失败。要量"被拒"这一跳，就得让**别的
/// 客户端**在点选之后、按下之前把那条提案处理掉（本判据经真 socket 放弃它）。这不是
/// 编出来的边角：控制面与界面共用同一个权威，两侧都可以动它，而界面**不**在每次按下前
/// 重新问一遍"这条还在不在"—— 它把身份交进去，让权威自己裁决。
///
/// ## 量什么
///
/// | 步 | 量什么 | 期望 |
/// | :--- | :--- | :--- |
/// | ① | 点选后、按下前经 socket 放弃那条 | 权威的未合并清单 1→0 |
/// | ② | 按下「采纳」后树里 `musical-pr-resolve-unavailable` 的 `label` | **逐字等于**同一次工具调用失败回执里的 `error.message` |
/// | ③ | 那句 vs 宿主的四条自述文案 | **一个都不相等** ⇒ 它是权威的话, 不是宿主替它写的话 |
/// | ④ | `musical-pr-open` / `resolve-available` / 两个按钮 | 仍开着 / `false` / 都不在树里 |
/// | ⑤ | 修订号、提交数、工程、未合并清单 | 与按下前**完全相同**（被拒的写一位不动） |
/// | ⑥ | 再打开一次抽屉 | 那句拒绝**消失** ⇒ 陈旧解释不会被读成"现在也用不了" |
///
/// ## 这条怎么变红
///
/// - 把拒绝理由吞掉（`resolve_proposal` 的 `Err` 臂只重注入不带 `refusal`）⇒ 第②步变成
///   `NO_SELECTION_REASON` 那句自述话，第③步红；
/// - 界面自己拼一句"操作失败"⇒ 第②步逐字不等；
/// - 被拒时关抽屉 ⇒ 第④步红（等于把拒绝理由一起关掉）；
/// - 被拒时还让按钮亮着 ⇒ 第④步红（用户可以对着同一个失败再按一次）；
/// - 拒绝没夹紧 ⇒ 第⑥步红（上一句解释赖在屏上）。
#[cfg(feature = "in-process-mcp")]
#[test]
fn a_refused_accept_puts_the_authoritys_own_sentence_on_the_drawer() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

// =====================================================================================
// 判据（诚实性审计 2026-10-08）：时光机画**真实提交链**；AI 徽章不再声称假状态
//
// 两条判据的"文本层 + 写者层"那一半住在新文件 `tests/undo_tree_honesty.rs`（本机不
// 渲染 Slint 也能跑）。这里只做运行时那一半：控件树 + 属性 + 真实点击 + 像素。
// =====================================================================================

/// 探针：运行时树的 `accessible-label` / `accessible-value` 里一个假历史节点名都没有。
///
/// 返回命中的字面量（空 = 干净）。**两个字段都探** —— 把假标签从 `label` 挪到
/// `value` 是一样糟的假数据。
fn demo_undo_literal_hits(tree: &ControlTree) -> Vec<String> {
    let mut hits = Vec::new();
    for node in tree.iter() {
        for literal in yeban_app::elements::DEMO_UNDO_TREE_LITERALS {
            if node.label.contains(literal) {
                hits.push(format!(
                    "`{}` 的 accessible-label 命中 `{literal}`",
                    node.id
                ));
            }
            if node
                .value
                .as_deref()
                .is_some_and(|value| value.contains(literal))
            {
                hits.push(format!(
                    "`{}` 的 accessible-value 命中 `{literal}`",
                    node.id
                ));
            }
        }
    }
    hits
}

/// 判据（**甲：真接**）：时光机画的节点 = **权威提交图谱**里的真实提交，一条不多一条不少。
///
/// ## 端到端链（每一步都是字面读数）
///
/// | 步 | 动作 | 断言 |
/// | :--- | :--- | :--- |
/// | ① | 打开真实 `UndoSession`（根提交消息 = `open <label>`）→ 装配活界面 → `host::apply_undo` | `undo-node-labels` == `["open <判据>"]`（**1** 条，不是 6 条）；`undo-node-count` == **1** |
/// | ② | 打开时光机 → 读运行时树 | 树里**有** `undo-tree-node-0`，其 `accessible-label` **逐字**等于那条真实提交消息；`undo-tree-node-1` **不在**树里；空态节点不在树里 |
/// | ③ | 经端口再提交 **7** 次真 op | 链条长 **8** ⇒ 画出最近 **6** 个 + `undo-node-hidden` == **2**；`undo-tree-node-6` 不在树里；截断报数节点在树里，其标签里的数字 == `2` |
/// | ④ | 假字面量探针 | 运行时树的 `label` / `value` 一处都不命中 [`DEMO_UNDO_TREE_LITERALS`] |
///
/// 为什么"节点数 == 图谱读数"是**算术**而不是巧合：`node-count` 由
/// `chain.len().min(UNDO_TREE_MAX_NODES)` 得到，`node-hidden` 由同一长度的
/// `saturating_sub` 得到 ⇒ 两个读数的和必须**恰好**等于 `CommitGraph::ancestry` 的长度。
#[test]
fn the_undo_tree_shows_the_real_commit_chain() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

/// 判据（**乙：如实表达"没有"**）：没有宿主写者时，时光机画**空态**，不画任何节点。
///
/// 这一条装配**没有**撤销端口（`LiveWiringOptions::undo = None`）—— `host::wire_undo` /
/// `host::apply_undo` 因此从未跑过 ⇒ 界面拿到的就是 `node-count` / `node-labels` /
/// `node-hidden` 的**属性默认值**（`0` / `[]` / `0`）。此时打开时光机，运行时树里必须有
/// `undo-tree-empty-state`，且**一个** `undo-tree-node-*` 都没有。
///
/// 改动前这里画着 **6** 个编造的版本名（含 `"c5 AI 提案"`）—— 那是**没有任何写者**的
/// 属性默认值，也就是用户看到的东西。
///
/// ## 像素
///
/// 命题①（**默认帧**，时光机关着）的读数是 PNG 字节的 sha256（本仓 PNG 是存储式
/// deflate，1920×1080 恒为 6,222,418 字节 ⇒ 尺寸证明不了内容）。命题②（打开时光机之后）
/// 是同一帧的差异像素数与包围盒 —— 两个命题分开陈述，不可互相代替。
#[test]
fn unwritten_undo_tree_shows_an_honest_empty_state_at_runtime() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

/// 判据（**乙的运行时那一半**）：AI 徽章不再声称"有 1 条待审查提案"，且它**真的能点**。
///
/// 改动前这一处是三连假：`accessible-role: button` 没有点击源；`accessible-value: "1"`
/// 与可见文案「AI提案 (待审查)」声称有 1 条待审查提案，而**没有任何宿主写者**
/// （`git grep -n 'ai-proposal-badge'` 只命中语义注册表与测试）。三件事各自独立：
///
/// 1. 运行时树的 `accessible-label` 是**动作**描述（不是状态声明），`accessible-value`
///    **不存在**（`null`，改前是 `"1"`）；
/// 2. `label` / `value` 都不含假状态字面量「待审查」；
/// 3. 一次**真实指针点击**（按下 → 移动 → 松手 → Slint 命中测试 → `.slint` 的 `TouchArea`
///    → `open-ai-proposals` → `app.slint` → `open-musical-pr` → 宿主写 `musical-pr-open`）
///    真的打开提案抽屉 —— 因此 `accessible-role: button` 不再是空承诺。
#[test]
fn the_ai_badge_has_a_real_action_and_no_fake_state_at_runtime() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

// =====================================================================================
// 判据：钢琴卷帘的工具矩阵**点得动**（台账 R16）
// =====================================================================================

/// 判据：五个工具按钮各自被一次**真实指针注入**点中之后，`active-tool` 必须变成那一行的行号。
///
/// ## 缺陷的形状（为什么这条判据有判别力）
///
/// 改动前 `ui/console/piano_roll.slint` 的五个工具按钮是纯 `Rectangle` + `UiText`：
/// 声明了 `accessible-role: button` 与 `accessible-checked`，但**没有** `TouchArea`、
/// 也**没有**回调 ⇒ 用户在界面上点它**没有任何反应**，`active-tool` 只由键盘写。
/// 本判据走的是 §12.4 的真实指针注入（按下 → 移动 → 松手 → Slint 命中测试 →
/// `.slint` 的输入面 → 回调 → 宿主 `Action::SelectTool` 臂写属性），
/// 与"用户点那一下"是同一条链。因此把 `TouchArea` 换回 `Rectangle`（或删掉那条回调）
/// 一定会红 —— 那时点击落到卷帘的滚动手势面上，`active-tool` 一位不变。
///
/// ## 为什么五个都点（而不是只点一个）
///
/// 只点一个的话，"行号 → 工具"的换算错了（例如 `tool_index` 与 `tool_index + 1` 混用）
/// 有一半的组合看不出来。五行逐个点 ⇒ 行号口径与 `Tool::digit`（`1`..`5`）的对账是完整的。
#[test]
fn the_piano_roll_tool_buttons_change_the_active_tool_at_runtime() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

// =====================================================================================
// 判据：工具矩阵的勾选态必须与**真正生效的工具**逐格一致（台账 R372）
// =====================================================================================

/// 读回工具矩阵五格的 `accessible-checked`（`ui/tree` 的活控件投影）。
///
/// 返回顺序 = `scene::TOOL_NAMES`（= `Tool::digit()` 的 `1`..`5`），每项是
/// `(语义 ID, checked)`；`None` = 该元素没声明勾选态（由调用方判红，不静默放过）。
fn tool_matrix_checked(plane: &mut LiveControlPlane) -> Vec<(String, Option<bool>)> {
    let (tree, _raw) = plane.plane().tree().expect("ui/tree");
    yeban_app::scene::TOOL_NAMES
        .iter()
        .map(|name| {
            let id = format!("piano-roll-tool-{name}-button");
            let checked = checked_in(&tree, &id);
            (id, checked)
        })
        .collect()
}

/// 判据：工具矩阵的**勾选态**必须跟**真正生效的工具**一格不差 —— 五个数字口径全查，含启动默认。
///
/// ## 缺陷的形状（为什么这条判据有判别力）
///
/// `active-tool` 是 **1 基**（`Tool::digit()` 的 `1`..`5`；`app.slint` 的默认值是 `1`），
/// 而矩阵的 `for tool_index in 5` 是 **0 基**。改动前三条绑定（底色 / `accessible-checked` /
/// 字色）都写 `root.active-tool == tool_index`：启动时 `active-tool == 1` 勾的是下标 `1`
/// 的 **「2 铅笔」**（真正生效的是箭头），而按 `5` 选橡皮时 `active-tool == 5` 对不上
/// 任何下标 ⇒ **一格都不勾**。把错口径写回三条绑定里任意一条，本条都会红。
///
/// ## 证明什么 / 不证明什么
///
/// 证明：逻辑键注入（`ui/dispatch_key_press`，真事件源）→ 宿主 `Action::SelectTool` 臂写
/// `active-tool` → `.slint` 绑定 → **活控件**的 `accessible-checked`（经 `ui/tree` 读回
/// `UiNode::checked`）这一整条链对 `1`..`5` 逐个成立，且**恰好一格**为 `true`。
/// 不证明：像素长什么样 —— 底色 / 字色那两条绑定与 `accessible-checked` 共用同一个
/// `tool_index` 比较，但本机不重录分平台 Golden（像素判据在 Linux 的 `real_ui_tier1.rs` 上跑）。
#[test]
fn the_tool_matrix_checked_state_tracks_the_active_tool_for_every_digit() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

// =====================================================================================
// 判据：在**编曲视图**点静音 ⇒ 撤销树的节点标签必须如实（台账 R17）
// =====================================================================================

/// 判据：编曲视图轨道头的静音开关点一下之后，撤销树里多出来的那条提交消息**不得**声称
/// 动作来自"混音台"。
///
/// ## 缺陷的形状（为什么这条判据有判别力）
///
/// 编曲视图的轨道头开关与调音台通道条的开关**共用同一个**宿主面
/// （`ui/workspace/arrangement_view.slint` 的 `track-mute-toggle` → `app.slint` →
/// `MainWindow.mixer-mute-toggle` → `host::wire_mixer_switch`）。那条 `commit_ops` 的
/// `message` 就是撤销树的**节点标签**（`Commit::message`）。改动前它写死
/// `"mixer: toggle track mute"` ⇒ 用户在编曲视图点一下，历史里却写着"混音台"。
///
/// 本判据走的是 §12.4 的真实指针注入（按下 → 移动 → 松手 → Slint 命中测试 → `.slint`
/// 的 `TouchArea` → 回调 → `Op::SetTrackMute` → `commit_ops`），因此它读到的标签是**真的
/// 那一条**，不是判据自己拼的字符串。
///
/// ## 为什么同时断言"工程真的变了"
///
/// 只看标签的话，"点击根本没生效"也能让标签断言通过（历史里那条消息还是旧提交的）。
/// 工程侧读数（模型里的 `TrackV3::mute`）证明这一击真的落了地 ⇒ 标签确实是这一击写下的。
#[test]
fn a_mute_click_in_the_arrangement_view_writes_an_honest_history_label() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

// =====================================================================================
// 判据：**读方法看见当下的界面**（`ui/tree` / `ui/node` / `ui/property` 的新鲜度一致）
//
// 背景：`ui/tree` / `ui/node` 读的是执行面持有的**运行时树缓存**，而 `ui/property`
// 与 `ui/screenshot` 读的是**活窗口**。两种读法的"新鲜度"因此不一致 ⇒
// 「人点一下按钮、AI 立刻读 `ui/tree`」看不见那一击。本判据把这条链钉住：
// **注入前**的读数（那时刻树是旧的）与**点击后**的读数必须一起报出来。
// =====================================================================================

/// 判据：一次**真实点击**之后，下一次 `ui/tree` / `ui/node` / `ui/property` 必须看见它。
///
/// | 段 | 做什么 | 期望 |
/// | :--- | :--- | :--- |
/// | ① 改前 | 注入前读 `ui/property {"sidebar","width"}` / `ui/tree` / `ui/node` | `"240.00"`；树里**有** `sidebar-search-field`；折叠按钮 `bounds.x = 210` |
/// | ② 点击 | `ui/dispatch_pointer_*` 点 `sidebar-collapse-button` | 活窗口 `sidebar-collapsed=false→true`（属性读数证明那一击真的生效） |
/// | ③ 改后 | 读同样三件 | `"36.00"`；树里**没有** `sidebar-search-field`；折叠按钮 `bounds.x = 6` |
///
/// ## 判别力从哪来（为什么不是"我写进去所以我说它对"）
///
/// ① 与 ③ 的三件读数全部**经控制面**取回（`ui/property` / `ui/tree` / `ui/node`），
/// 没有任何一处直接读 `MainWindow` 的 Rust getter。② 走的是 §12.4 的真实指针注入
/// （按下 → 移动 → 松手 → Slint 命中测试 → `.slint` 的 `TouchArea` → 回调 → 宿主写属性），
/// 与用户点那一下是同一条链。
#[test]
fn a_tree_read_sees_a_click_that_just_happened() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据：**读元数据不得渲染一帧**（`ui/tree` / `ui/node` / `ui/property` /
/// `ui/dynamic_regions` / `ui/coverage`），而 `ui/screenshot` **恰好**渲染一帧。
///
/// ## 这条判据在防什么（不是防"读变慢"，是防"读悄悄开始渲染"）
///
/// `refresh_tree` 曾经（`bd9fbb6`）在重抓前先 `capture()` 一次 —— 那是 1920×1080 的
/// Tier-1 全量软件光栅化。读路径现在**每次读**都重抓，因此那一步一旦被加回来，
/// 每一次 `ui/tree` 都会白渲染一张没人看的 6 MB 帧，而**没有任何断言会红**。
/// 本判据把这条不变量钉住：探针是 `Tier1Window::rendered_frames()`
/// （窗口上真的跑过渲染回调的次数），判据读的是 [`LiveControlPlane::rendered_frames`]。
///
/// ⚠ 断言**不是**计时（墙钟随机器浮动，不适合做判据）：它数的是**事件次数**，
/// 因此是确定性的。
#[test]
fn a_tree_read_does_not_rasterize_a_frame() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

/// 判据：**一次重抓的代价 = 一次全树内省，不是一次全量渲染**。
///
/// 这条判据存在的原因：`refresh_tree` 曾经（`bd9fbb6`）在重抓前先 `capture()` 一次，
/// 而读路径现在**每次读**都要重抓 ⇒ 那一步会把 1920×1080 的 Tier-1 全量软件光栅化
/// 挂在每一次 `ui/tree` 上。本判据把两个数字并排打出来，让"重抓很便宜、渲染很贵"
/// 这件事有可复跑的读数（**只报不判时间**：墙钟是机器相关的代理指标，不适合做断言；
/// 断言只落在"两条路都真的产出了东西"上）。
///
/// | 量什么 | 怎么量 | 单位 |
/// | :--- | :--- | :--- |
/// | Tier-1 全量渲染一帧 | `LiveUi::capture()` 一次（先预热一次剔除首次初始化） | ms（墙钟） |
/// | 重抓一次运行时树 | `LiveUi::pump_meters()` 一次（抽干 → 注入 → 全树内省） | ms（墙钟） |
/// | 树规模 | `tree_snapshot().len()` | 节点数 |
#[test]
fn a_tree_regrab_costs_one_introspection_not_one_full_render() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

// ===========================================================================
// 横向缩放（`Z` / `Shift+Z`）：真实事件源 → 投影参数 → 像素
// ===========================================================================

/// 从**模型**算选区的 tick 跨度（`min start_tick`, `max start_tick + duration_ticks`）。
///
/// 这是**独立 oracle**：它走 `clip_pool` 的模型数据，不走 `ViewState`。
/// 于是"投影算错跨度"会在判据里显形（拿投影自己的读数当 oracle 是自我确认）。
/// 返回 `None` = 选中的身份一个都不在工程里（空选区 / 陈旧身份）。
fn model_span_of(project: &YebanProjectV1, selected_ulids: &[String]) -> Option<(u64, u64)> {
    let wanted: std::collections::BTreeSet<&str> =
        selected_ulids.iter().map(String::as_str).collect();
    let mut span: Option<(u64, u64)> = None;
    for entry in project.clip_pool.values() {
        let Some(notes) = entry.content.notes() else {
            continue;
        };
        for note in notes.values() {
            let id = note.id.to_canonical_string();
            if !wanted.contains(id.as_str()) {
                continue;
            }
            let end = note.start_tick + note.duration_ticks;
            span = Some(match span {
                None => (note.start_tick, end),
                Some((start, best_end)) => (start.min(note.start_tick), best_end.max(end)),
            });
        }
    }
    span
}

/// 从**模型**算工程的**内容末端 tick**：全部剪辑摆放 `start + duration` 与全部段落
/// `end_tick` 的最大值（`Shift+Z` 的独立 oracle）。
fn model_content_end(project: &YebanProjectV1) -> u64 {
    let clip_end = project
        .tracks
        .values()
        .flat_map(|track| track.clips.values())
        .map(|placement| placement.start_tick + placement.duration_ticks)
        .max()
        .unwrap_or(0);
    let section_end = project
        .sections
        .values()
        .map(|section| section.end_tick)
        .max()
        .unwrap_or(0);
    clip_end.max(section_end)
}

/// 把一批音符身份写进选区的**唯一事实源** `selected-ulids`（并把计数同步过去）。
fn select_ulids(window: &yeban_app::ui::MainWindow, ids: &[String]) {
    window.set_selected_ulids(slint::ModelRc::new(slint::VecModel::from(
        ids.iter()
            .map(|id| slint::SharedString::from(id.as_str()))
            .collect::<Vec<_>>(),
    )));
    window.set_selected_note_count(i32::try_from(ids.len()).unwrap_or(i32::MAX));
}

/// 判据 Z1（横向缩放，**默认构建**）：`Z` ⇒ 选区撑满视口；`Shift+Z` ⇒ 全曲总览；
/// **空选区按 `Z` ⇒ 没有变化且键未被消费**；缩放**不进撤销栈**。
///
/// ## 量什么 / 怎么量 / 单位
///
/// | 量 | 怎么量 | 单位 |
/// | :--- | :--- | :--- |
/// | 当前缩放 | `MainWindow.roll-ticks-per-pixel`（`host::roll_ticks_per_pixel` 是唯一读点） | tick / 逻辑像素 |
/// | 滚动偏移 | `MainWindow.roll-scroll-x` | 逻辑像素 |
/// | 选区跨度 | [`model_span_of`]（**模型**侧 oracle） | tick |
/// | 内容末端 | [`model_content_end`]（**模型**侧 oracle） | tick |
/// | 视口宽 | `slint::Window::size().width`（与 `apply_view` / 裁剪同一个读数） | 逻辑像素 |
/// | 画面变化 | [`frame_diff`] 的差异**像素数** + 包围盒（逐像素比 RGB） | 像素 |
///
/// ## 算术（全部整数、**向上取整**；期望值在这里手算，不引用被测函数）
///
/// - 夹具（`filled_project`）：4 个音符，起点 `0 / 960 / 1920 / 2880`，
///   时值各 `480` ⇒ 终点 `480 / 1440 / 2400 / 3360`。工程内容末端 **`15360`**
///   （两个段落 `Intro = 0..7680`、`Drop = 7680..15360` 的较大者；剪辑末端 `3840` 更短）
///   —— 出处：`crates/yeban-model/src/samples.rs` 的 `filled_project`。
/// - 视口 `1920` px。夹紧上下界是 `1` / `960`。
/// - 全选（并集跨度 `3360 − 0 = 3360`）⇒ `ceil(3360 / 1920) = 2`，滚动 `0 / 2 = 0`。
/// - 只选**最后一个**音符（跨度 `3360 − 2880 = 480`）⇒ `ceil(480 / 1920) = 1`，
///   滚动 `2880 / 1 = 2880`。
/// - `Shift+Z`（内容末端 `15360`）⇒ `ceil(15360 / 1920) = 8`，滚动 `0`。
///
/// ## 怎么变红（每条都实测过）
///
/// | 注入 | 位置 | 现象 |
/// | :--- | :--- | :--- |
/// | 把 `ZoomToSelection` / `ZoomToFit` 放回 `action_has_implementation` 的 `!matches!` | `src/host.rs` | 注入之后缩放一位不变（本判据第一段红），`cli_contract.rs` 的 B11d 也红 |
/// | 跨度只取**一个**音符的区间（不取并集） | `src/bridge.rs` 的 `selection_tick_span` | 全选那一步的期望值 `2` 变成 `1` ⇒ 红 |
/// | 换算改成**向下取整**（`span / px`） | `src/bridge.rs` 的 `ticks_per_pixel_to_fit` | 全选那一步 `3360 / 1920` 的向下取整 = `1`，期望 `2` ⇒ 红（**这一步是"向上取整"的牙**；`3841 / 1920` 那条边界由 `bridge` 的单元判据钉住） |
/// | 选区总览不写 `scroll-x` | `src/host.rs` 的 `zoom_action` | 最后一步 `roll-scroll-x` 期望 `2880`、实得 `0` ⇒ 红 |
#[test]
fn zoom_shortcuts_reach_the_projection_and_an_empty_selection_is_not_consumed() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

// =====================================================================================
// AI 建议采纳（`Shift+Enter`）与试听主线（`[` / `]`）：本装配里没有可作用的对象
// ⇒ 如实拒绝且一位不变
// =====================================================================================

/// 判据 B11f 的**行为侧**（`[UI-A11Y-001]` / `[ARCH-RT-005]`）：`Shift+Enter` 与 `[`
/// 在**真实界面**上被如实拒绝，而且**一位不变**（不是"吞掉键却什么都不做"）。
///
/// ## ⭐ 这两条的**原因不同**（2026-10-31 起），别把它们读成同一件事
///
/// - `[` / `]`（试听主线 / 提案）：**能力本身缺失** —— `[ARCH-RT-005]` 要的"A/B 并发渲染
///   ＋ 30ms 等功率瞬切 ＋ 2048 采样预滚"在仓库里一处都不存在 ⇒
///   `host::action_has_implementation` 返回 `false`，**任何**装配都不消费。
/// - `Shift+Enter`（采纳）：能力**已经落地**（`HostAction::MergeProposal` ⇒ 与 MCP 工具
///   复用同一个 `Plan::Merge`）。它在这一条里仍然被拒绝, 是因为**本装配的前置条件不满足**
///   （`proposal_source: ProposalSource::none()` ⇒ 没有写来源, 且没点选任何一行 ⇒
///   `musical-pr-resolve-available` 是 `false`）。"接上权威 + 点选一行 ⇒ 这个键真的改领域"
///   由 `tests/live_ui_mcp.rs` 的 `selecting_a_row_lights_up_accept_and_it_really_merges_the_proposal` 那一条量 —— ⛔ 不要在这里把它当"仍然不可用"的证据。
///
/// ## 为什么要有这一条（它不是"什么都没做"的空判据）
///
/// 它证明的是**否定命题**："这些动作在这一格状态下没有可作用的对象" —— 这一点只能靠"按键
/// 之后没有任何可观测变化"来证，而"没有任何变化"必须先证明**探针能看见变化**，否则断言
/// 是空的。因此本判据自带一个**阳性对照**：注入已落地且已绑定的 `B`（箭头 ⇄ 铅笔）⇒
/// 同一个读数面必须真的变。阳性对照不过 ⇒ 本判据的"一位不变"不算证据。
///
/// ## 量什么 / 怎么量 / 单位
///
/// | 步 | 量什么 | 怎么量 | 单位 |
/// | :--- | :--- | :--- | :--- |
/// | ① | 工具矩阵行号 | `MainWindow.active-tool` | 行号（1–5） |
/// | ② | 视图档 | `MainWindow.arrangement-view` | bool |
/// | ③ | 走带显示态 | `MainWindow.playing` | bool |
/// | ④ | 横向缩放 / 滚动 | `MainWindow.roll-ticks-per-pixel` / `roll-scroll-x` | tick/px、tick |
/// | ⑤ | 工程音符条目数 | `model_note_count(UndoPort::project())` | 条目 |
/// | ⑥ | 可撤销步数 / 提交数 | `UndoPort::display().undoable` / `.commit_count` | 步、次 |
/// | ⑦ | 帧内容 | `LiveControlPlane::capture()` 的**差异像素数**与 **PNG 字节的 sha256** | 像素、哈希 |
///
/// ## 怎么变红
///
/// - 把 `Action::AuditionMain` / `Action::AuditionProposal` 从
///   `host::action_has_implementation` 的那一条挡下臂移出而没真接上能力 ⇒ ①②…⑦ 里
///   至少一项变化（键被消费 ⇒ `invoke_key_action` 返回 `true`）。
/// - 把 `[` / `]`（或 `Shift+Enter`）从 `input.rs` 的策略表里删掉 ⇒
///   `invoke_key_action` 仍然返回 `false`，但 `tests/cli_contract.rs` 的判据 B11f 会红
///   （它要求键仍绑定, 且只有 `[` 那一条带 `(未实现)` 标记）。
/// - 把 `Shift+Enter` 那一臂改成"不看前置条件就 invoke"（删掉 `resolve-available` 那道
///   闸）⇒ 本判据的第一格变红（没有写来源的装配里键被消费, 而什么都没发生）。
/// - 把阳性对照用的 `B` 摘掉 ⇒ 本判据的阳性对照步变红（说明探针失灵，而不是"引擎安静"）。
#[test]
fn the_ai_suggestion_and_audition_keys_are_rejected_without_any_state_change() {
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
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
}

// =====================================================================================
// 判据：`Escape` 关闭**全屏时光机**（`[UI-A11Y-003]` §7.3 / `.slint` 的 "Esc 关闭" 那句）
// =====================================================================================

/// 判据：时光机开着时，`Escape` 经**真事件源**注入必须把它关掉，并且**不碰工程**。
///
/// ## 缺陷的形状（为什么这条判据有判别力）
///
/// `ui/dialogs/undo_tree_modal.slint` 的底部提示行写着 `… · Esc 关闭`，而
/// `host::apply_action` 的 `Action::Cancel` 分支此前**只**问两条拖拽手势
/// （`cancel_track_height_drag` 与窗口回调 `mixer-cancel-gesture`）。时光机开着时
/// 两条都不在手 ⇒ 返回 `false`（`reject`）⇒ 弹窗**关不掉**，那句话是假的。
/// 更强的证据：全仓**没有**任何调用点读 `UiAction::CloseUndoTree`
/// （改动前只有 `undo.rs` 的变体定义、它的 `name()` 与它自己的自判据）。
///
/// ## 为什么关弹窗**必须**走 `UndoPort`
///
/// `undo-tree-open` 的**唯一写者**是 `host::apply_undo`，它每一跳都从
/// `port.undo_tree_open()` 回写 ⇒ 直接 `window.set_undo_tree_open(false)` 会在下一次
/// 回写时被宿主的读数覆盖。因此这条判据断言的是"端口读数 + 运行时树"两处同时变化，
/// 而不是只断言一个可以被覆盖的属性。
///
/// ## 端到端链（每一步都是字面读数）
///
/// | 步 | 动作 | 断言 |
/// | :--- | :--- | :--- |
/// | ① | 经端口打开时光机 → `apply_undo` | 端口读数 `true`；运行时树里**有** `undo-tree-modal` |
/// | ② | `ui/dispatch_key_press` 注入 `Escape` | 注入成功；端口读数 `false`（弹窗关掉） |
/// | ③ | 读运行时树 | `undo-tree-modal` **不在**树里（`.slint` 的 `visible` 归位） |
/// | ④ | 动作日志 + 工程读数 | 最后一条记录的动作名 == `"close-undo-tree"`；撤销栈与提交数一位未动 |
///
/// ## 注入（负向实测）
///
/// 把 `host.rs` 的 `Action::Cancel` 分支里那段 `CloseUndoTree` 落点删掉（或把
/// `undo_tree_open()` 的判定改成恒 `false`）⇒ ② 的端口读数与 ③ 的树读数**同时**变红。
#[test]
fn escape_closes_the_time_machine_modal_for_real() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

// ==================================================================== 混音台**双击复位**
//
// 文本层那四段接线（命中面 → 回调 → 两级转发 → 宿主四臂）在 `tests/mixer_fader_double_click.rs`；
// 这里证的是**动作**：双击真的改了工程、只改被点那一格、已经在中性位就⛔ 不提交，
// 而且带着在手的拖拽去双击不会把 `mixer-drag-active` 黏住。
//
// ⚠ 为什么驱动的是 `MainWindow` 回调而不是真实指针：`double-clicked` 要 `click_count == 2`，
// 而注入面今天只有 `down` / `move` / `up`（`METHOD_DISPATCH_POINTER_DOWN` / `..._MOVE`），
// 没有"第二下带计数"的入口。`.slint` 的四条 `double-clicked` 臂调的**就是**这里 invoke 的
// 那四个回调（`ui/console/mixer_console.slint` 的四面），因此走的仍是同一条宿主链；
// "臂确实挂在命中面上"那半由文本层判据钉 —— 两半缺一个都会漏掉一类缺陷。

/// 判据：双击推子 / 声相 ⇒ 工程与界面**同时**回中性（`0 dB` / 中央），并且
/// ③ 已经在中性位时**不产生**任何提交、⑤ 复位与拖动共用**同一条**撤销栈。
#[test]
fn a_double_click_reset_moves_the_fader_and_pan_to_neutral() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}

/// 判据（手势交错）：双击的第二下会先来一次 `grab` ⇒ 复位臂**必须先把在手的拖拽清零**。
/// 两条交错都跑：清零之后那一次孤立的 `release` 既不提交、也不许把标志留在 `true`。
/// 同一窗口里另证**主控**那两条（寻址按身份、视图态是两个标量属性）。
#[test]
fn a_double_click_reset_leaves_no_sticky_gesture() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
}
