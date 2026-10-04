//! `yeban-app` ↔ `yeban-ui-test-port` 的**单向适配器**（app 侧）。
//!
//! 规范来源 (Normative)：
//! - `[UI-TEST-001]` UI/UX §12.2 语义元素寻址：控件树断言必须建立在稳定语义 ID 上；
//! - `[UI-MCP-002]` §12.5 动态区域遮罩：遮罩矩形必须来自元素树元数据；
//! - `[UI-MCP-003]` §12.5 分平台 Golden 与 SSIM ≥ 0.98；
//! - `[MUST-GATE-015]` 路线图 §5：Golden 图必须由 Tier-1 软件光栅化产出，尺寸非零且非全黑。
//!
//! ## 本文件有两个 cargo 目标（**判据源码只有这一份**）
//!
//! 1. `[[test]] name = "test_port_adapter"`（`Cargo.toml`，`required-features = ["ui-test-port"]`）
//!    —— 集成者设计的入口；`cargo test -p yeban-app --features ui-test-port` 会跑它。
//! 2. `tests/real_ui_tier1.rs` —— 用 `#[path]` 把本文件的判据**原样**装进一个自动发现的
//!    集成测试目标，并通过 `[dev-dependencies] yeban-ui-test-port` 拿到依赖。
//!
//! 为什么必须有第 2 个入口（实测，不是偏好）：第一个入口挂在默认关闭的 feature 后面，
//! 而 CI 目前没有任何一步启用该 feature（`.github/workflows/ci.yml` 里那一步被集成者
//! 按纪律暂时移除），于是**它从未被编译过** —— commit `66b002c` / run 37223586792 的
//! 实测就是 8 处编译错误（`image` 未绑定 ×6、`report_evidence` / `report_capability` 缺 `use`）。
//! dev-dependencies **不进入** release 构建，因此 `AGENTS.md` §2 红线 6 与
//! `Cargo.toml` 里"内省能力不进默认构建"的契约没有被削弱（`cargo build -p yeban-app`
//! 的依赖图不变，可用 `cargo tree -p yeban-app -e normal` 核验）。
//!
//! ## 本文件证明什么 / 不证明什么
//!
//! 证明（`cargo test -p yeban-app`，零 feature）：
//! - 184 条注册表条目能被**无损**适配成语义控件树（ID / 角色 / 标签 / 动态标记逐条对齐）；
//! - 13 个 `.slint` 真的能被 **Tier-1 软件光栅化**渲染出**非零、非全黑**的像素，
//!   且三个不同状态（默认 Arrangement / compact 断点 / Session 视图）都渲染得出差异；
//! - `[UI-TEST-001]` 的运行时控件树与静态注册表的**双向覆盖关系**被实测钉住
//!   （运行时有 ⇒ 注册表有，严格为空；注册表有 ⇒ 运行时未必有）；
//! - `[UI-MCP-002]` + `[UI-MCP-003]`：动态区遮罩吸收抖动（SSIM 恒为 1.0），
//!   而"成块 + 亮度差大"的静态回归在遮罩之后仍被检出（< 0.98）；
//! - `[UI-MCP-001]` 的 `ReadOnly` 闸门在真实端口上拒绝事件注入。
//!
//! 不证明：像素**长什么样**（没有基准图，`[UI-MCP-003]` 的分平台 Golden 需要人类先提交基准）；
//! 也不证明"UI 没有视觉缺陷" —— 它证明的是"UI 真的被渲染过、而且树和像素来自同一个实例"。
//!
//! ## 控件树的事实源与它的前置条件（按集成者裁决）
//!
//! - **权威事实源是注册表**（[`registry_to_tree`]）：它是纯 Rust、确定性、无窗口也能工作的。
//!   `dump_json()` 的默认输出与产物 `app-registry-control-tree.json` 都来自它。
//! - **运行时交叉核对是独立的能力**：它的前置条件是被内省的 `.slint` 在**编译期**打开了
//!   debug info。上游默认关闭（`i-slint-compiler-1.18.1/lib.rs:282`：
//!   `debug_info = std::env::var_os("SLINT_EMIT_DEBUG_INFO").is_some()`），没有它时
//!   `element_count()` 返回 `None` ⇒ 控件树**恒为空且不报错**。
//!   `crates/yeban-app/build.rs` 已按 ADR-0001 D22 打开它
//!   （`compile_with_config("ui/app.slint", ..with_debug_info(true))`），所以本判据现在
//!   **应当**拿到非空树。若将来它又变空（有人改回 `compile`），下面的能力分支会
//!   **出声**（`report_capability`）并且**红** —— "静默跳过"在本仓库等于假绿。
//!
//! ## `visible: false` 的子树为什么不在运行时树里（上游语义，已对源码核验）
//!
//! `visit_descendants` 只访问 `ItemRc::is_visible()` 为真的元素，而它判定的是
//! "绝对裁剪矩形 ∩ 元素几何 ≠ ∅"（`i-slint-core-1.18.1/item_tree.rs:410-419`）。
//! `visible:` 在编译期被 lower 成一个注入的 `Clip` 元素
//! （`i-slint-compiler-1.18.1/passes/visible.rs`：`clip = !visible`、`is-visibility-clip: true`），
//! 而该 pass 跑在 `default_geometry` **之后**（`passes.rs:171-198` 的顺序），
//! 所以这个包裹元素没有几何绑定 ⇒ 几何恒为 `0×0`：
//! `visible == true` 时它不 clip（子元素照常可见）、`visible == false` 时它以空矩形
//! clip 掉整棵子树。上游自己的回归测试把它钉死了
//! （`i-slint-backend-testing-1.18.1/search_api.rs:1223-1266` 的 `test_conditional`：
//! `visible-element` 与它的 `inner-element` 在 `visible: false` 时都查不到）。
//! 本文件的 [`DEFAULT_VIEW_MUST_NOT_HAVE`] 就是这条语义的判据。
//!
//! ## 边界（不编造）
//!
//! - `ElementMeta::component`（定义该 ID 的 `.slint` 文件）在 test-port 的节点模型里没有对应字段，
//!   因此断言失败时无法从 JSON 直接映射回源码文件（已登记为 needs）。
//! - 静态注册表没有几何 ⇒ `bounds` 一律 `None`，`parent` 一律 `None`（不编造层级）。

use yeban_app::elements::ElementRegistry;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_ui_test_port::port::{Permission, PortError, UiTestPort};
use yeban_ui_test_port::tree::{ControlNode, ControlTree, Role, TreeError};
use yeban_ui_test_port::{
    GoldenEvidence, LivePort, LumaImage, Rect, Rgb8Image, Size, compare_with_dynamic_masking,
    golden_evidence, mask_rects_from_tree, report_capability, report_evidence, report_line, ssim,
};

/// 把 `yeban-app` 的静态语义元素注册表适配成 test-port 的控件树。
///
/// 这是一个**纯函数**：注册表 → 树，不做 I/O、不碰 Slint、没有几何
/// （静态注册表里没有包围盒，`bounds` 一律 `None` —— 不编造数字）。
fn registry_to_tree(registry: &ElementRegistry) -> Result<ControlTree, TreeError> {
    let mut tree = ControlTree::new();
    for meta in registry.iter() {
        let role = Role::parse(meta.kind.accessible_role())?;
        let node = if meta.dynamic_region {
            ControlNode::new(meta.id.clone(), role, meta.label.clone()).as_dynamic()
        } else {
            ControlNode::new(meta.id.clone(), role, meta.label.clone())
        };
        tree.insert(node)?;
    }
    Ok(tree)
}

/// 把一行"实测观察值"同时写进 CI 可见的 stderr 与 `target/ui-test-port/` 下的产物文件。
///
/// 两处都要的理由：(a) [`report_line`] 直写进程 fd 2，所以**通过的测试**也会把它留在
/// CI 日志里（`eprintln!` 会被 libtest 捕获）；(b) 文本产物跟着截图一起被
/// `upload-artifact` 上传，人复核截图时能对照同一批数字，不必翻日志。
///
/// 用 `O_APPEND` 而不是"读—改—写"：同一个测试二进制里的两个测试是**并行**跑的，
/// 读改写会互相覆盖（`report_line` 那份仍然完整，所以这只是一处artifact 卫生问题）。
fn observe(detail: &str) {
    use std::io::Write as _;

    report_line(detail);
    let path = yeban_ui_test_port::artifact_dir().join("app-introspect-observations.txt");
    if let Some(parent) = path.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        report_line(&format!("观察值文件目录创建失败(不影响判据): {err}"));
        return;
    }
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(mut handle) => {
            if let Err(err) = writeln!(handle, "{detail}") {
                report_line(&format!("观察值文件写入失败(不影响判据): {err}"));
            }
        }
        Err(err) => report_line(&format!("观察值文件打开失败(不影响判据): {err}")),
    }
}

/// 落盘一张 Tier-1 截图并打印路径。
///
/// 落盘失败**是判据失败**（不是"不影响判据"）：`[MUST-GATE-015]` 与 `AGENTS.md` §3 DoD 6
/// 要的是"人眼可复核的证据"，而截图是这个证据的载体 —— 写不出来就等于没有。
fn write_png(name: &str, image: &Rgb8Image) {
    let path = yeban_ui_test_port::write_artifact(name, image)
        .unwrap_or_else(|err| panic!("写 Tier-1 截图 `{name}` 失败: {err}"));
    observe(&format!("截图产物: {}", path.display()));
}

/// 两个矩形的交集面积（无交集或退化时为 0）。用于判断"这个候选静态区被动态遮罩盖掉了多少"。
fn overlap_area(left: Rect, right: Rect) -> u64 {
    let x0 = left.x.max(right.x);
    let y0 = left.y.max(right.y);
    let x1 = left.right().min(right.right());
    let y1 = left.bottom().min(right.bottom());
    if x1 <= x0 || y1 <= y0 {
        return 0;
    }
    (x1 - x0) as u64 * (y1 - y0) as u64
}

/// 一个矩形内的平均亮度。
///
/// 口径与 `ssim` 模块**完全一致**：直接用 [`Rgb8Image::to_luma`] 的结果（ITU-R BT.601），
/// 不在这里另写一份灰度公式 —— 两份公式就是两个口径。
fn mean_luma(luma: &LumaImage, rect: Rect) -> f64 {
    let Some(clipped) = rect.intersect(luma.size()) else {
        return 0.0;
    };
    let mut sum = 0.0_f64;
    let mut count = 0_u64;
    for y in clipped.y..clipped.bottom() {
        for x in clipped.x..clipped.right() {
            if let Some(value) = luma.at(x as u32, y as u32) {
                sum += value;
                count += 1;
            }
        }
    }
    if count == 0 { 0.0 } else { sum / count as f64 }
}

/// `ui/tokens.slint` §6 的两个色值。
///
/// 为什么在这里写一份副本：`.slint` 的 `Tokens` global 不保证能从 Rust 侧按名取到，
/// 而"判据需要这两个背景色"不值得为它生成一层绑定代码。这与
/// `src/scene.rs` ↔ `.slint` 的常量重复是同一类 known debt（见 `scene.rs` 的模块文档）。
/// 判据本身钉住了"这两个色值确实是面板背景"：下面任何一个背景色写错，
/// 该元素内部的墨迹数都会暴涨（把整块背景都算成墨迹），判据立刻变红。
const TOKEN_BG_VOID: [u8; 3] = [0x06, 0x0a, 0x14];
/// 见 [`TOKEN_BG_VOID`]。
const TOKEN_BG_PANEL_ALT: [u8; 3] = [0x1b, 0x24, 0x47];
/// 见 [`TOKEN_BG_VOID`]（状态栏的面板色）。
const TOKEN_BG_PANEL: [u8; 3] = [0x15, 0x1d, 0x38];

/// ADR-0001 D24「界面字体非 tofu」判据的硬下限：**以汉字为主的文本元素**内部的墨迹像素数。
///
/// 两侧都有实测（数字与测法见 `docs/ledger/app-introspect-notes.md` §6.2）：
/// - **没有 CJK 字体**（第 1 轮 CI 的真实截图，用 Pillow **独立解码**）：`ai-rail-diagnose-button`
///   内部只有 **24 px** 墨迹 —— 只剩 ASCII 的 `:` 与 `/`，汉字整片**什么都不画**
///   （实测不是豆腐块：连 `.notdef` 方框都没有）；
/// - **有 CJK 字体**（本机用 FreeType + 苹方/宋体按 10–11px 量同一串）：`声学诊断:` ≈ 211–239 px、
///   `掩蔽 / 相位 / 动态范围` ≈ 497–562 px，按 10px 折算合计 ≈ **590 px**。
///
/// 取 150：比"无字体"基线高 6×，比"有字体"预期低 4×，两侧都不擦边。
const MIN_CJK_INK_PIXELS: u64 = 150;

/// 通道差超过它才算"墨迹"（抗锯齿的浅色边缘也算 —— 那是真实的字形覆盖）。
const INK_CHANNEL_TOLERANCE: u8 = 24;

/// 把矩形向内收 `margin` 像素（用于排除面板的 1px 边框）。退化时返回空矩形。
fn inset_rect(rect: Rect, margin: u32) -> Rect {
    let shrink = margin.saturating_mul(2);
    if rect.width <= shrink || rect.height <= shrink {
        return Rect::new(rect.x, rect.y, 0, 0);
    }
    Rect::new(
        rect.x.saturating_add(margin as i32),
        rect.y.saturating_add(margin as i32),
        rect.width - shrink,
        rect.height - shrink,
    )
}

/// 一张图里某个矩形内的"墨迹"统计：`(墨迹像素数, 墨迹包围盒, 不同颜色数)`。
///
/// "墨迹" = 任一通道与**该元素的背景色**相差超过 [`INK_CHANNEL_TOLERANCE`] 的像素。
/// 用"与该元素自己的背景色比较"而不是"非黑"，因为 DAW 面板本身就不是黑的
/// （`bg-void` 是 `#060a14`）；用"非黑"会让整块面板都算成墨迹。
fn ink_stats(image: &Rgb8Image, rect: Rect, background: [u8; 3]) -> (u64, Option<Rect>, usize) {
    let Some(area) = rect.intersect(image.size()) else {
        return (0, None, 0);
    };
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    let mut ink = 0_u64;
    let mut colors = std::collections::BTreeSet::new();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let Some(pixel) = image.pixel(x as u32, y as u32) else {
                continue;
            };
            colors.insert(pixel);
            let differs = (0..3).any(|channel| {
                pixel[channel].abs_diff(background[channel]) > INK_CHANNEL_TOLERANCE
            });
            if differs {
                ink += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    // `if` 而不是 `bool::then(..)`：后者会被 `clippy::unnecessary_lazy_evaluations` 盯上
    // （闭包里只是算术，没有副作用），而 CI 的 `-D warnings` 不接受"善意提示"。
    let bbox = if ink > 0 {
        Some(Rect::new(
            min_x,
            min_y,
            (max_x - min_x + 1) as u32,
            (max_y - min_y + 1) as u32,
        ))
    } else {
        None
    };
    (ink, bbox, colors.len())
}

/// 为"遮罩不得把界面变成盲区"这条判据挑一个改动对象：**面积 ≥5% 画面**、
/// 且**不与动态遮罩区大面积重叠**的静态节点，并按它的**实测**平均亮度决定改向。
///
/// 为什么要算而不是写死坐标（ADR-0001 D23 的实测结论）：SSIM（7×7 均匀窗）只对
/// "成块 + 亮度差足够大"的变化敏感；DAW 界面是大量纯色面板，一个写死的
/// `Rect::new(400, 300, 400, 300)` 完全可能落在本来就接近纯黑的面板里 ——
/// 那样判据看起来在测视觉回归，实际什么都没测。这里改为：
/// ① 候选必须 ≥5% 画面（SSIM 是逐窗均值，改动区太小无论如何都拉不下 0.98）；
/// ② 动态区盖掉候选的 <10% 面积（否则回归会被遮罩吸收）；
/// ③ 平均亮度 >127.5 ⇒ 往黑改，否则往白改 ⇒ **保证**是"亮度差大"的方向。
///
/// 5% 这个门槛不是随手定的：本机实测（用 `ssim.rs` 的真实实现 + 合成画面）
/// **0.48% 画面的改动未遮罩 SSIM = 0.994871 ⇒ 检不出**（≥0.98），
/// 而 ≥5% 画面 + 亮度差大的改动实测 0.62 量级 ⇒ 检得出。
/// 数字与依据见 `docs/ledger/app-introspect-notes.md` §5.1；口径见 ADR-0001 D23。
fn pick_regression_target(
    tree: &ControlTree,
    luma: &LumaImage,
    size: Size,
    masks: &[Rect],
) -> Option<(Rect, f64, [u8; 3])> {
    let floor = (size.pixel_count() / 20).max(1);
    let mut best: Option<(Rect, u64)> = None;
    for node in tree.iter() {
        if node.dynamic_region {
            continue;
        }
        let Some(bounds) = node.bounds.and_then(|rect| rect.intersect(size)) else {
            continue;
        };
        let area = bounds.area();
        if area < floor {
            continue;
        }
        let masked: u64 = masks.iter().map(|mask| overlap_area(bounds, *mask)).sum();
        if masked * 10 >= area {
            continue;
        }
        if best.as_ref().is_none_or(|(_, best_area)| area > *best_area) {
            best = Some((bounds, area));
        }
    }
    let (bounds, _area) = best?;
    let value = mean_luma(luma, bounds);
    let fill = if value > 127.5 {
        [0x00, 0x00, 0x00]
    } else {
        [0xff, 0xff, 0xff]
    };
    Some((bounds, value, fill))
}

/// 默认演示视图下**必然可见**的语义 ID 最小集合（`arrangement-view = true`、`compact = false`、
/// `console-tab = 0`、所有抽屉与模态关闭）。
///
/// 上游 Testing Backend 的 `visit_descendants` 只访问**几何可见**的元素
/// （见文件头"`visible: false` 的子树为什么不在运行时树里"），所以运行时树是
/// "当前可见元素"的集合，而注册表是"UI 声明过的全部元素"的集合。
/// 这两个集合本来就不该相等 —— 本判据只钉住"默认视图的关键部件真的在树里"。
///
/// 这一份是**硬断言**的核心清单：每一条都在默认视图里既非 `visible: false` 又无裁剪风险。
const DEFAULT_VIEW_MUST_HAVE: [&str; 16] = [
    "ai-rail",
    "arrangement-playhead",
    "arrangement-ruler",
    "console-tab-rail",
    "piano-roll",
    "piano-roll-grid",
    "sidebar",
    "splitter-workspace-console",
    "status-bar",
    "status-bar-selection",
    "transport-bpm-field",
    "transport-play-button",
    "transport-record-button",
    "transport-stop-button",
    "transport-timecode",
    "workspace-arrangement-canvas",
];

/// 默认视图下**可见的**、且**在 `.slint` 里只声明一次**的全部语义 ID（覆盖率分母）。
///
/// 为什么只收非重复节点：`for` 循环展开的元素能否逐个被语义 ID 寻址，取决于上游
/// `ElementHandle::accessible_id()` 对 `element_index != 0` 返回 `None`
/// （`i-slint-backend-testing-1.18.1/search_api.rs:745-748`）是否会影响重复实例。
/// 那件事在**编译之前**无法核验，因此重复族只出现在
/// [`VISIBLE_FAMILY_MEMBERS`]（"至少一个成员可寻址"）与观察值里，不进这个分母。
///
/// 覆盖率判据允许少量缺失（`< MIN_SINGLETON_COVERAGE_PERCENT` 才红），因为某一块面板
/// 被邻居裁剪掉 1px 属于 SSIM/几何口径问题；而系统性失效（树为空、整列不见）会立刻掉到 0%。
const VISIBLE_SINGLETONS: [&str; 39] = [
    "ai-rail",
    "ai-rail-diagnose-button",
    "ai-rail-intent-button",
    "ai-rail-musical-pr-button",
    "arrangement-loop-brace",
    "arrangement-playhead",
    "arrangement-ruler",
    "console-maximize-button",
    "console-tab-rail",
    "note-suggestion-overlay-rect",
    "piano-roll",
    "piano-roll-grid",
    "piano-roll-keys",
    "piano-roll-playhead",
    "piano-roll-status",
    "piano-roll-velocity-lane",
    "sidebar",
    "sidebar-collapse-button",
    "sidebar-search-field",
    "splitter-workspace-console",
    "status-bar",
    "status-bar-chord",
    "status-bar-device",
    "status-bar-selection",
    "status-bar-shortcut-tip",
    "transport-ai-drawer-button",
    "transport-ai-proposal-badge",
    "transport-bpm-field",
    "transport-branch-button",
    "transport-commit-button",
    "transport-play-button",
    "transport-record-button",
    "transport-revert-button",
    "transport-stop-button",
    "transport-timecode",
    "transport-view-arrangement-button",
    "transport-view-session-button",
    "transport-view-toggle-button",
    "workspace-arrangement-canvas",
];

/// 单例覆盖率的硬下限（百分比）。
const MIN_SINGLETON_COVERAGE_PERCENT: usize = 90;

/// `[UI-TEST-001]` 点名的语义 ID 族：默认视图下**每个可见族至少要有一个成员**能被语义 ID 寻址。
///
/// 这是"不管上游怎么展开 `for`，语义寻址必须至少可用"的最低要求 —— 比"实例数必须等于 N"
/// 弱，比"整族不判"强。实测计数（含全部实例）作为观察值打印，不进断言。
const VISIBLE_FAMILY_MEMBERS: [&str; 10] = [
    "clip-01J8Z5Q0R7K3M9X2V4B6N8P1F9-header",
    "note-01J8Z5Q0R7K3M9X2V4B6N8P1A2-rect",
    "piano-roll-tool-select-button",
    "section-0-card",
    "sidebar-category-0-button",
    "sidebar-item-0",
    "tab-piano-roll-button",
    "track-0-header",
    "track-0-mute-button",
    "velocity-0-bar",
];

/// 默认视图下**不可见**的语义 ID 样本：它们不得出现在运行时树里 ——
/// 否则说明 `visible:` 的语义没有被尊重（见文件头对上游 `visible.rs` 的核验）。
const DEFAULT_VIEW_MUST_NOT_HAVE: [&str; 7] = [
    "workspace-session-canvas",
    "session-back-to-arrangement-button",
    "scene-launch-column-header",
    "mixer-console",
    "device-rack",
    "musical-pr-drawer",
    "undo-tree-modal",
];

fn demo_registry() -> ElementRegistry {
    ElementRegistry::demo()
}

/// 判据 1: 适配必须是**无损的双射** —— 条目数、ID、角色、标签、动态标记逐条对齐。
#[test]
fn adapter_mirrors_the_registry_one_to_one() {
    let registry = demo_registry();
    let tree = registry_to_tree(&registry).expect("注册表必须能适配");

    assert_eq!(tree.len(), registry.len(), "适配不得丢条目/造条目");
    assert!(
        registry.len() >= 100,
        "注册表条目数异常偏少: {}",
        registry.len()
    );

    for meta in registry.iter() {
        let node = tree
            .find_by_id(&meta.id)
            .unwrap_or_else(|| panic!("丢失 ID {}", meta.id));
        assert_eq!(
            node.role.as_str(),
            meta.kind.accessible_role(),
            "角色漂移: {}",
            meta.id
        );
        assert_eq!(node.label, meta.label, "标签漂移: {}", meta.id);
        assert_eq!(
            node.dynamic_region, meta.dynamic_region,
            "动态标记漂移: {}",
            meta.id
        );
        assert_eq!(node.bounds, None, "静态注册表不该有几何: {}", meta.id);
        assert_eq!(node.parent, None, "静态注册表没有层级信息: {}", meta.id);
    }

    let dynamic_in_tree = tree
        .dynamic_regions()
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    let dynamic_in_registry = registry
        .dynamic_regions()
        .map(|meta| meta.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        dynamic_in_tree, dynamic_in_registry,
        "动态区清单必须逐条一致"
    );
    assert!(
        dynamic_in_tree.len() >= 10,
        "动态区只有 {} 条, 疑似漏登记",
        dynamic_in_tree.len()
    );
}

/// 判据 2: 语义 ID 的格式规则在两侧是**同一条规则**（依赖方向不允许复用实现，因此逐条对账）。
#[test]
fn id_format_rule_matches_yeban_app() {
    let registry = demo_registry();
    for meta in registry.iter() {
        assert_eq!(
            yeban_app::elements::is_well_formed_id(&meta.id),
            yeban_ui_test_port::tree::is_well_formed_id(&meta.id),
            "两份 ID 格式实现出现分歧: {}",
            meta.id
        );
        assert!(
            yeban_app::elements::is_well_formed_id(&meta.id),
            "注册表 ID 必须格式良好"
        );
    }
    for probe in [
        "",
        "-x",
        "x-",
        "a--b",
        "track_0",
        "track 0",
        "音符-0",
        "note-01J8Z-rect",
    ] {
        assert_eq!(
            yeban_app::elements::is_well_formed_id(probe),
            yeban_ui_test_port::tree::is_well_formed_id(probe),
            "两份实现分歧: `{probe}`"
        );
    }
}

/// 判据 3: 注册表用到的**每一个** `ElementKind` 都必须落在 Slint `AccessibleRole` 的全集里
/// （拼错一个角色就会让运行时控件树构建整条失败）。
#[test]
fn every_registry_role_is_a_slint_accessible_role() {
    let registry = demo_registry();
    let mut seen = std::collections::BTreeSet::new();
    for meta in registry.iter() {
        let raw = meta.kind.accessible_role();
        Role::parse(raw).unwrap_or_else(|err| panic!("`{raw}` 不是合法角色: {err}"));
        seen.insert(raw);
    }
    assert!(
        seen.len() >= 15,
        "注册表只用到 {} 种角色, 覆盖异常",
        seen.len()
    );
    assert!(
        seen.contains("progress-indicator"),
        "动态区必须有 progress-indicator 一类的角色"
    );
}

/// 判据 4: 静态树**没有几何**，因此 `[UI-MCP-002]` 的遮罩在它上面必须**显式报错**，
/// 而不是"遮 0 个矩形"然后静默通过。
///
/// 这条判据是"动态标记 + 几何必须合并"的证明：几何只能来自运行时树（`LivePort`），
/// 动态标记只能来自注册表（本文件），任何一侧单独用都不够。
#[test]
fn static_tree_cannot_mask_without_geometry() {
    let tree = registry_to_tree(&demo_registry()).expect("适配");
    let err = mask_rects_from_tree(&tree).expect_err("静态树没有几何, 必须报错");
    assert!(
        matches!(err, TreeError::DynamicRegionWithoutBounds { .. }),
        "错误类型必须指明原因, 实际 {err:?}"
    );
    assert!(tree.mask_rects().is_err(), "静态树的 mask_rects 必须报错");
}

/// 判据 5: 控件树 JSON 必须逐字节稳定、可被反序列化回等价模型（CI 断言脚本读的就是它）。
#[test]
fn control_tree_json_is_stable_and_round_trips() {
    let registry = demo_registry();
    let first = registry_to_tree(&registry).expect("适配");
    let second = registry_to_tree(&registry).expect("适配");
    let json = first.dump_json();
    assert_eq!(json, second.dump_json(), "同一注册表两次导出必须逐字节相同");
    assert_eq!(ControlTree::from_json(&json).expect("往返"), first);
    assert!(json.ends_with("}\n"));
    assert!(json.contains("\"track-0-fader\"") || json.contains("track-0-"));
}

/// 判据 6: 本文件里四份 ID 清单的每一条都必须在注册表里**真的存在**。
///
/// 没有这条，"不可见的 `X` 不得出现在运行时树里"就退化成一句空话 —— 一个拼错的 ID
/// （例如把 `scene-launch-column-header` 写成 `scene-launch-column-head`）永远满足
/// `!contains(id)`：判据看起来绿，实际什么都没测。
/// 这条判据是**纯注册表**的，因此在本机也能核验（本机不许编译 Slint）。
#[test]
fn criteria_id_lists_are_all_real_registry_entries() {
    let registry = demo_registry();
    for id in DEFAULT_VIEW_MUST_HAVE
        .iter()
        .chain(VISIBLE_SINGLETONS.iter())
        .chain(VISIBLE_FAMILY_MEMBERS.iter())
        .chain(DEFAULT_VIEW_MUST_NOT_HAVE.iter())
    {
        assert!(
            registry.contains(id),
            "判据清单里的 `{id}` 不在注册表里 —— 该判据要么拼错了, 要么已经过期"
        );
    }
    for id in DEFAULT_VIEW_MUST_HAVE {
        assert!(
            !DEFAULT_VIEW_MUST_NOT_HAVE.contains(&id),
            "`{id}` 同时出现在「必须可见」与「必须不可见」两份清单里"
        );
    }
    let unique: std::collections::BTreeSet<&str> = VISIBLE_SINGLETONS.into_iter().collect();
    assert_eq!(
        unique.len(),
        VISIBLE_SINGLETONS.len(),
        "单例清单不得有重复项"
    );
}

/// 构造注入同一组演示数据的主窗口（与 `src/main.rs` 的 GUI 路径一致）。
fn build_demo_main_window(scene: &DemoScene) -> Result<MainWindow, slint::PlatformError> {
    let ui = MainWindow::new()?;
    ui.set_timecode(scene.timecode.into());
    ui.set_bpm_display(scene.bpm_display.into());
    ui.set_branch_name(scene.branch_name.into());
    ui.set_arrangement_view(scene.arrangement_by_default);
    ui.set_playing(false);
    ui.set_console_tab(0);
    ui.set_compact(scene.compact());
    Ok(ui)
}

/// **本文件的核心里程碑（像素那一半）**：让 13 个 `.slint` 真的被 Tier-1 软件光栅化渲染一次，
/// 并断言 `[MUST-GATE-015]` 的两条门槛（尺寸非零、非全黑）+ `[UI-MCP-001]` 的权限闸门。
///
/// 这一条**不依赖运行时控件树**（因此也不依赖编译期 debug info），所以它总是可执行的 ——
/// 它就是 ADR-0001 D18 里那句"当前界面只是编译通过，从未被渲染器或人眼看过"的关闭动作。
///
/// 三个状态各截一张（都是同一个活窗口实例，尺寸都是规范的 ≥1920 全展开网格 1920×1080）：
/// A 默认演示视图（Arrangement + 全展开）、B `compact` 断点、C Session 视图。
/// 只截一张的话，"13 个 `.slint` 被渲染过"这句话对 `session_view.slint` 与
/// `sidebar.slint` 的折叠分支仍然是未验证的。
#[test]
fn live_main_window_renders_tier1_pixels_and_enforces_permissions() {
    let scene = DemoScene::demo();
    let registry = demo_registry();
    let static_tree = registry_to_tree(&registry).expect("注册表适配");
    let size = Size::new(scene.viewport_width, scene.viewport_height);

    let mut port = LivePort::new(size, Permission::ReadOnly, Some(&static_tree), || {
        build_demo_main_window(&scene)
    })
    .expect("Tier-1 平台 + MainWindow + 运行时控件树");

    // ---- A: 默认演示视图（Arrangement / 全展开） ----
    let first = port.window().capture().expect("Tier-1 截图");
    let second = port.window().capture().expect("第二次截图");
    assert_eq!(
        first.pixels(),
        second.pixels(),
        "静态界面两次截图必须逐字节相同（NewBuffer 语义下的全量重绘）"
    );
    let evidence: GoldenEvidence =
        golden_evidence(&first).expect("[MUST-GATE-015] 尺寸非零且非全黑");
    assert_eq!(evidence.size, size, "截图尺寸必须等于窗口尺寸");
    assert!(evidence.non_black_pixels > 0, "全黑截图: {evidence:?}");
    assert!(
        evidence.distinct_colors >= 8,
        "颜色过少, 疑似只画了背景: {evidence:?}"
    );
    assert_eq!(
        evidence.png_bytes,
        yeban_ui_test_port::png::encoded_len(&first),
        "证据里的 PNG 字节数必须与编码器一致"
    );
    report_evidence(&evidence);
    observe(&format!(
        "状态 A (Arrangement / 全展开 {}x{}): {}",
        size.width,
        size.height,
        evidence.summary()
    ));
    observe(&format!(
        "状态 A 两次截图逐字节相同: {}",
        first.pixels() == second.pixels()
    ));
    write_png("app-main-window-arrangement-full-1920x1080", &first);

    // ---- B: `compact` 断点（左栏收成 36px 导轨 / 右栏改抽屉） ----
    port.ui().set_compact(true);
    let compact_image = port.window().capture().expect("compact 断点截图");
    let compact_evidence =
        golden_evidence(&compact_image).expect("[MUST-GATE-015] compact 截图非零且非全黑");
    assert!(
        compact_evidence.distinct_colors >= 8,
        "compact 截图颜色过少: {compact_evidence:?}"
    );
    assert_ne!(
        compact_image.pixels(),
        first.pixels(),
        "compact 断点必须真的改变像素（否则 [UI-GRID-002] 的折叠分支没有被渲染过）"
    );
    report_evidence(&compact_evidence);
    observe(&format!(
        "状态 B (Arrangement / compact): {}",
        compact_evidence.summary()
    ));
    write_png(
        "app-main-window-arrangement-compact-1920x1080",
        &compact_image,
    );
    port.ui().set_compact(scene.compact());

    // ---- C: Session 视图（另一条可见性分支：session_view.slint 首次被渲染） ----
    port.ui().set_arrangement_view(false);
    let session_image = port.window().capture().expect("Session 视图截图");
    let session_evidence =
        golden_evidence(&session_image).expect("[MUST-GATE-015] Session 截图非零且非全黑");
    assert!(
        session_evidence.distinct_colors >= 8,
        "Session 截图颜色过少: {session_evidence:?}"
    );
    assert_ne!(
        session_image.pixels(),
        first.pixels(),
        "切换视图必须真的改变像素（否则 session_view.slint 从未被渲染过）"
    );
    report_evidence(&session_evidence);
    observe(&format!(
        "状态 C (Session / 全展开): {}",
        session_evidence.summary()
    ));
    write_png("app-main-window-session-full-1920x1080", &session_image);

    // ---- 控件树的**权威事实源是注册表**（无窗口也能工作、确定性、由构造方注入） ----
    let json_path = yeban_ui_test_port::artifact_dir().join("app-registry-control-tree.json");
    std::fs::write(&json_path, static_tree.dump_json())
        .unwrap_or_else(|err| panic!("写注册表控件树 JSON 失败 {}: {err}", json_path.display()));
    let runtime_json = yeban_ui_test_port::artifact_dir().join("app-runtime-control-tree.json");
    std::fs::write(&runtime_json, port.tree().dump_json())
        .unwrap_or_else(|err| panic!("写运行时控件树 JSON 失败 {}: {err}", runtime_json.display()));
    observe(&format!(
        "控件树 JSON: 注册表 {} 条 -> {}; 运行时(默认视图) {} 条 -> {}",
        static_tree.len(),
        json_path.display(),
        port.tree().len(),
        runtime_json.display()
    ));

    // ---- `[UI-MCP-001]` 真实端口上的权限行为 ----
    assert_eq!(port.permission(), Permission::ReadOnly);
    assert!(matches!(
        port.dispatch_pointer_down(
            "transport-play-button",
            1.0,
            1.0,
            yeban_ui_test_port::PointerButton::Left
        ),
        Err(PortError::PermissionDenied { .. })
    ));
    assert!(matches!(
        port.dispatch_key_press(yeban_ui_test_port::KeyCode::Tab),
        Err(PortError::PermissionDenied { .. })
    ));
    // ReadOnly 允许的截图（不依赖运行时控件树）。
    let png = port.capture_png().expect("只读允许截图");
    assert!(
        png.starts_with(&yeban_ui_test_port::png::PNG_SIGNATURE),
        "capture_png 必须返回真 PNG"
    );
}

/// `[UI-TEST-001]` 的**首次真实断言**：运行时控件树 vs 静态注册表。
///
/// 结果先打印成实测数字，判据再建立在这些数字上 —— 失败时日志/产物里已经有全部观察值，
/// 不需要"再跑一轮才知道发生了什么"。
#[test]
fn runtime_control_tree_cross_check_against_the_registry() {
    let scene = DemoScene::demo();
    let registry = demo_registry();
    let static_tree = registry_to_tree(&registry).expect("注册表适配");
    let size = Size::new(scene.viewport_width, scene.viewport_height);
    // 注意：这条判据只用只读方法（`tree()` / `read_property()` / `capture()`），
    // 因此**不能**声明成 `mut` —— `unused_mut` 在 CI 的 `-D warnings` 下是硬错误。
    let port = LivePort::new(size, Permission::ReadOnly, Some(&static_tree), || {
        build_demo_main_window(&scene)
    })
    .expect("Tier-1 平台 + MainWindow");

    // ---- 能力前置条件：`ElementHandle` 的遍历依赖**编译期 debug info**（ADR-0001 D22） ----
    if port.tree().is_empty() {
        report_capability(
            "unavailable: crates/yeban-app 的 .slint 没有编译期 debug info ⇒ ElementHandle \
             遍历拿到一棵空树 (见 docs/ledger/ui-test-port-notes.md §2 #27 / §10 needs 0)",
        );
        panic!(
            "[UI-TEST-001] 运行时控件树为空 ⇒ 前置条件不满足: `crates/yeban-app` 的 .slint 没有编译期 debug info。\n\
             根因: i-slint-compiler-1.18.1/lib.rs:282 `debug_info = std::env::var_os(\"SLINT_EMIT_DEBUG_INFO\").is_some()` 默认关闭;\n\
                   `crates/yeban-app/build.rs` 必须用 `compile_with_config(\"ui/app.slint\", ..with_debug_info(true))`\n\
                   (ADR-0001 D22 已记录该裁决)。\n\
             影响: §12.2 语义寻址 / §12.3 属性读取 / §12.5 动态遮罩在**真实界面**上都还无法执行。\n\
             详见 docs/ledger/app-introspect-notes.md。"
        );
    }

    let runtime = port.tree().clone();
    let coverage = runtime.coverage_against(&static_tree);

    // =====================================================================
    // 第 1 步：把**实测数据**全部打印出来（判据在第 2 步才建立在这些数字上）
    // =====================================================================
    observe(&format!(
        "控件树计数: 注册表 {} 条 / 运行时 {} 条 / 运行时有而注册表无 {} 条 / 注册表有而运行时无 {} 条",
        static_tree.len(),
        runtime.len(),
        coverage.unknown_at_runtime.len(),
        coverage.missing_at_runtime.len()
    ));
    observe(&format!(
        "运行时有而注册表无(必须为空): {:?}",
        coverage.unknown_at_runtime
    ));
    observe(&format!(
        "注册表有而运行时无({} 条, 全部): {:?}",
        coverage.missing_at_runtime.len(),
        coverage.missing_at_runtime
    ));

    let present = VISIBLE_SINGLETONS
        .iter()
        .filter(|id| runtime.contains(id))
        .count();
    let coverage_percent = present * 100 / VISIBLE_SINGLETONS.len();
    let singletons_missing: Vec<&str> = VISIBLE_SINGLETONS
        .iter()
        .copied()
        .filter(|id| !runtime.contains(id))
        .collect();
    observe(&format!(
        "关键单例覆盖率: {present}/{} = {coverage_percent}%（硬下限 {MIN_SINGLETON_COVERAGE_PERCENT}%）缺: {singletons_missing:?}",
        VISIBLE_SINGLETONS.len()
    ));

    let family = |prefix: &str, suffix: &str| -> usize {
        runtime
            .with_prefix(prefix)
            .filter(|node| node.id.ends_with(suffix))
            .count()
    };
    let prefix_count = |prefix: &str| -> usize { runtime.with_prefix(prefix).count() };
    observe(&format!(
        "重复族实测计数(观察值, 非断言): track-*-header={}, track-*-fader={}, track-*-meter={}, \
         note-*-rect={}, clip-*-header={}, velocity-*-bar={}, section-*-card={}, slot-*-cell={}, \
         tab-*-button={}, sidebar-item-*={}, piano-roll-tool-*-button={}",
        family("track-", "-header"),
        family("track-", "-fader"),
        family("track-", "-meter"),
        family("note-", "-rect"),
        family("clip-", "-header"),
        family("velocity-", "-bar"),
        family("section-", "-card"),
        family("slot-", "-cell"),
        family("tab-", "-button"),
        prefix_count("sidebar-item-"),
        family("piano-roll-tool-", "-button"),
    ));
    let family_present: Vec<&str> = VISIBLE_FAMILY_MEMBERS
        .iter()
        .copied()
        .filter(|id| runtime.contains(id))
        .collect();
    observe(&format!(
        "可见重复族里可被语义 ID 寻址的样本: {}/{} {:?}",
        family_present.len(),
        VISIBLE_FAMILY_MEMBERS.len(),
        family_present
    ));

    // ---- `[UI-MCP-002]` + `[UI-MCP-003]` 的实测数字（先算，后判） ----
    let image = port.window().capture().expect("Tier-1 截图");
    let mask_rects = mask_rects_from_tree(&runtime).expect("运行时动态区必须有几何");
    let targets: Vec<Rect> = mask_rects
        .iter()
        .copied()
        .filter(|rect| !rect.is_empty() && rect.intersect(size).is_some())
        .collect();
    let mask_area: u64 = targets.iter().map(|rect| rect.area()).sum();
    observe(&format!(
        "运行时动态区: {} 个（在画面内 {} 个）, 总面积 {mask_area} px = 画面的 {:.4}%",
        mask_rects.len(),
        targets.len(),
        mask_area as f64 * 100.0 / size.pixel_count() as f64
    ));

    // 抖动：把所有动态区整块涂绿（DAW 里真实发生的事：VU 表跳变 / 时间码跳字）。
    let mut jittered = image.clone();
    for rect in &targets {
        jittered.fill_rect(*rect, [0x00, 0xff, 0x00]);
    }
    let jitter_changed = image.pixels() != jittered.pixels();
    let jitter_unmasked = ssim::ssim(&image, &jittered).expect("同尺寸可算");
    let jitter_masked =
        compare_with_dynamic_masking(&image, &jittered, &runtime).expect("遮罩比对");

    // 静态回归：按实测像素挑"成块 + 亮度差大"的改动对象（ADR-0001 D23）。
    let luma = image.to_luma();
    let (target_rect, target_luma, fill) = pick_regression_target(&runtime, &luma, size, &targets)
        .expect("默认视图里必须有 ≥5% 画面、且不被动态区盖掉的静态节点");
    let mut regressed = image.clone();
    regressed.fill_rect(target_rect, fill);
    let regression_changed = image.pixels() != regressed.pixels();
    let regression_unmasked = ssim::ssim(&image, &regressed).expect("同尺寸可算");
    let regression_masked =
        compare_with_dynamic_masking(&image, &regressed, &runtime).expect("遮罩比对");
    observe(&format!(
        "[UI-MCP-003] 动态抖动未遮罩 SSIM={jitter_unmasked:.6} / 遮罩后 SSIM={:.6}(passed={}); \
         静态回归 {}x{}@({},{}) 平均亮度 {target_luma:.2} 填 {fill:?}: 未遮罩 SSIM={regression_unmasked:.6}, \
         遮罩后 SSIM={:.6}(passed={})",
        jitter_masked.score,
        jitter_masked.passed,
        target_rect.width,
        target_rect.height,
        target_rect.x,
        target_rect.y,
        regression_masked.score,
        regression_masked.passed,
    ));

    // =====================================================================
    // 第 2 步：判据
    // =====================================================================

    // ---- 方向 1（强）：运行时有 ⇒ 注册表有。UI 里出现未登记的 accessible-id 就是契约破裂。 ----
    assert!(
        coverage.unknown_at_runtime.is_empty(),
        "运行时树里出现了注册表没登记的语义 ID（双向契约被破坏）: {:?}",
        coverage.unknown_at_runtime
    );
    // ---- 方向 2（弱，按事实写）：注册表有 ⇒ 运行时未必有。 ----
    // 上游遍历是**几何可见性 + visible 语义**过滤后的集合，所以"注册表有而运行时无"必然非空：
    // 默认视图下 Session 视图整棵、Mixer / DeviceRack 两个 tab、两个模态都不在可见分支里。
    // 反过来若它为空，说明可见性过滤没生效（`visible: false` 的分支也在树里）。
    assert!(
        !coverage.missing_at_runtime.is_empty(),
        "运行时树包含了注册表全部 {} 条 —— 说明可见性/裁剪没有被尊重（`visible: false` 的分支也在树里）",
        static_tree.len()
    );
    assert!(
        coverage.missing_at_runtime.len() < static_tree.len(),
        "运行时树一条都没抓到 (缺 {}/{}), 布局或 debug info 疑似没生效",
        coverage.missing_at_runtime.len(),
        static_tree.len()
    );
    assert!(
        runtime.len() >= 40,
        "运行时树只有 {} 个节点, 远少于默认视图应有的量, 布局疑似没有生效",
        runtime.len()
    );

    for id in DEFAULT_VIEW_MUST_HAVE {
        assert!(
            runtime.contains(id),
            "默认视图缺少必然可见的 `{id}` ({coverage})"
        );
    }
    assert!(
        coverage_percent >= MIN_SINGLETON_COVERAGE_PERCENT,
        "可见单例覆盖率 {coverage_percent}% < {MIN_SINGLETON_COVERAGE_PERCENT}%（缺 {singletons_missing:?}）"
    );
    for id in VISIBLE_FAMILY_MEMBERS {
        assert!(
            runtime.contains(id),
            "`[UI-TEST-001]` 要求可见族的成员可被语义 ID 寻址, 但 `{id}` 不在运行时树里 ({coverage})"
        );
    }
    for id in DEFAULT_VIEW_MUST_NOT_HAVE {
        assert!(
            !runtime.contains(id),
            "不可见的 `{id}` 混进了运行时树 —— `visible: false` 的语义没有被尊重"
        );
    }

    // ---- `[UI-MCP-002]`：遮罩必须完全吸收动态抖动 ----
    assert!(jitter_changed, "抖动必须真的改到像素");
    assert!(
        !targets.is_empty(),
        "运行时树里没有任何**在画面内**的动态区 —— [UI-MCP-002] 无从执行: {mask_rects:?}"
    );
    assert!(jitter_masked.passed, "遮罩后必须通过: {jitter_masked}");
    assert!(
        (jitter_masked.score - 1.0).abs() <= f64::EPSILON,
        "遮罩区内的差异必须被完全吸收（逐字节相同 ⇒ SSIM 恒等于 1.0）, 实测 {}",
        jitter_masked.score
    );
    // "未遮罩时必须被检出"只在动态区**占画面足够大**时成立：SSIM 是逐窗均值，
    // 1px 宽的走带光标改 10^2 个像素时均值仍然是 1.000000 —— 那不是判据失效，是口径。
    if mask_area * 100 >= size.pixel_count() * 5 {
        assert!(
            !ssim::Verdict::with_default_threshold(jitter_unmasked).passed,
            "动态区占画面 ≥5% 时未遮罩必须被检出, 实测 SSIM {jitter_unmasked:.6}"
        );
    } else {
        observe(&format!(
            "说明: 动态区只占画面 {:.4}% ⇒ 未遮罩 SSIM={jitter_unmasked:.6} 拉不下 0.98（SSIM 口径, 非判据失效）; \
             '未遮罩的大块亮度变化必须被检出'由下面的静态回归判据承担",
            mask_area as f64 * 100.0 / size.pixel_count() as f64
        ));
    }

    // ---- `[UI-MCP-003]`：遮罩不得把整个界面变成盲区 ----
    assert!(regression_changed, "静态回归的改动必须真的改到像素");
    assert!(
        !ssim::Verdict::with_default_threshold(regression_unmasked).passed,
        "D23: 成块({}x{}) + 亮度差大的静态回归在**未遮罩**时必须被检出, 实测 SSIM {regression_unmasked:.6}",
        target_rect.width,
        target_rect.height
    );
    assert!(
        !regression_masked.passed,
        "静态回归在**遮罩之后**仍然必须被检出（遮罩不能把界面变成盲区）, 实测 SSIM {}",
        regression_masked.score
    );

    // ---- ADR-0001 D24：界面字体**非 tofu**（"汉字真的被栅格化"的量化版本） ----
    //
    // D24 把这条判据明确划给本线（原文建议"可用字符包围盒非零或与已知 tofu 图样比对"）。
    // 这里用墨迹量化：`ai-rail-diagnose-button` 的内容是两行汉字（12 个汉字 + `:`/`/` 三个 ASCII），
    // 对照是状态栏的 `status-bar-chord`（2 个汉字 + 5 个 ASCII，同为 font-size-xs）。
    let cjk_card = runtime
        .find_by_id("ai-rail-diagnose-button")
        .and_then(|node| node.bounds)
        .expect("AI 协作栏的『声学诊断』卡片必须可见（D24 的汉字判据挂在它上面）");
    let (cjk_ink, cjk_bbox, cjk_colors) = ink_stats(&image, inset_rect(cjk_card, 4), TOKEN_BG_VOID);
    let reference_card = runtime
        .find_by_id("status-bar-chord")
        .and_then(|node| node.bounds)
        .expect("状态栏的『和弦』文本必须可见（D24 的对照项）");
    let (reference_ink, reference_bbox, reference_colors) =
        ink_stats(&image, inset_rect(reference_card, 4), TOKEN_BG_PANEL);
    let (intent_ink, _, _) = ink_stats(
        &image,
        inset_rect(
            runtime
                .find_by_id("ai-rail-intent-button")
                .and_then(|node| node.bounds)
                .expect("AI 协作栏的『意图生成』卡片必须可见"),
            4,
        ),
        TOKEN_BG_VOID,
    );
    observe(&format!(
        "[D24] 汉字墨迹: 声学诊断卡(12 汉字+3 ASCII) {cjk_ink} px (包围盒 {cjk_bbox:?}, 颜色 {cjk_colors}); \
         对照 `status-bar-chord`(2 汉字+5 ASCII) {reference_ink} px (包围盒 {reference_bbox:?}, 颜色 {reference_colors}); \
         意图生成卡(14 汉字+2 ASCII) {intent_ink} px; 下限 {MIN_CJK_INK_PIXELS} px"
    ));
    assert!(
        cjk_ink >= MIN_CJK_INK_PIXELS,
        "D24「界面字体非 tofu」: `ai-rail-diagnose-button` 内部几乎没有墨迹（实测 {cjk_ink} px, 下限 {MIN_CJK_INK_PIXELS}）\
         ⇒ 汉字没有被栅格化。\n\
         已知两侧实测: 没有 CJK 字体的 runner 上该卡片只有 24 px（只剩 ASCII 的 `:` 与 `/`）; \
         有 CJK 字体时应为数百 px（本机 FreeType 按 10px 折算 ≈ 590 px）。\n\
         排查顺序: ① 该 job 的 apt 步骤是否装了 `fonts-noto-cjk`（ADR-0001 D24 的运行时环境依赖, 不是仓库资产）; \
         ② Slint 的字体回退链（`ui/tokens.slint` 的 `font-ui`）是否命中它。\n\
         限制（明说）: 本判据证明「汉字字形真的被画出来了」, 但**不能**逐字形比对 —— \
         那需要一份人类批准的参考图样（见 docs/ledger/app-introspect-notes.md §8）。"
    );
    assert!(
        cjk_ink > reference_ink,
        "D24: 以汉字为主的卡片({cjk_ink} px) 的墨迹必须多于以 ASCII 为主的对照卡({reference_ink} px) —— \
         没有 CJK 字体时实测是 24/119 = 0.20, 有字体时约为 3–5"
    );

    // ---- `[UI-MCP-001]` ReadOnly 的属性读取（需要运行时元素） ----
    let role = port
        .read_property("transport-play-button", "role")
        .expect("role 可读");
    assert_eq!(role, "button", "运行时角色必须与注册表一致");
    let width = port
        .read_property("transport-play-button", "width")
        .expect("width 可读");
    assert!(
        width.parse::<f64>().is_ok_and(|value| value > 0.0),
        "实测 width={width}"
    );
    assert!(matches!(
        port.read_property("transport-play-button", "no-such-property"),
        Err(PortError::Rejected { .. })
    ));
}
