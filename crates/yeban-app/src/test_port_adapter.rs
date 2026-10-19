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
//! - 全部注册表条目能被**无损**适配成语义控件树（ID / 角色 / 标签 / 动态标记逐条对齐）；
//! - 13 个 `.slint` 真的能被 **Tier-1 软件光栅化**渲染出**非零、非全黑**的像素，
//!   且三个不同状态（默认 Arrangement / compact 断点 / Session 视图）都渲染得出差异；
//! - `[UI-TEST-001]` 的运行时控件树与静态注册表的**双向覆盖关系**被实测钉住
//!   （运行时有 ⇒ 注册表有，严格为空；注册表有 ⇒ 运行时未必有）；
//! - `[UI-MCP-002]` + `[UI-MCP-003]`：动态区遮罩吸收抖动（SSIM 恒为 1.0），
//!   而"成块 + 亮度差大"的静态回归在遮罩之后仍被检出（< 0.98）；
//! - `[UI-MCP-001]` 的 `ReadOnly` 闸门在真实端口上拒绝事件注入；
//! - **`[MODEL-AST-002]` 模型数据真的到达像素**：
//!   `project_projection_reaches_the_control_tree_and_the_pixels` 用
//!   `yeban_model::samples::filled_project()` 驱动同一个活窗口，断言控件树里的
//!   `track-{i}-header` / `section-{i}-card` / `clip-{ulid}-header` 携带的是**工程的**
//!   名称与身份（`Lead` / `Intro` / 工程的摆放 ULID），而演示夹具的名字（`鼓`/`贝斯`）
//!   一个都不出现；随后在**同一实例**上换回演示工程，断言截图逐字节变化。
//! - **app-completion 追加**：轨道色标（`track-{i}-color-swatch` 的标签 == 投影的
//!   `#RRGGBB`，缺色轨道是回退色）与卷帘音符的**位置排序**（x 随 `start_tick` 严格递增、
//!   y 随 `pitch` 严格递减 —— 索引布局会让 y 与 pitch 同向，因此这条能直接抓住回退）。
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

use std::cell::RefCell;
use std::rc::Rc;

use yeban_app::bridge::{ViewState, host_to_painted};
use yeban_app::elements::{ElementRegistry, is_model_driven_family};
use yeban_app::engine_host::{EngineHost, TransportActionRecord};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_engine::ring::TransportCommand;
use yeban_engine::transport::{TransportReading, TransportState};
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
    regenerate_golden_if_asked(name, image);
    assert_matches_golden(name, image);
}

/// `[UI-MCP-003]` / `[ROAD-M0-008]` / `[ROAD-M3-007]` 的**视觉回归判据**。
///
/// 口径：把刚渲染的帧与 `tests/golden/<platform>/<name>.png` **逐字节**比对。
/// 之所以能逐字节：本仓的 PNG 编码器是自研的 **stored-deflate**（无压缩、无时间戳、无随机化），
/// 且第 104 轮已实测「同一条 Tier-1 路径再生成 ⇒ 3/3 张 sha256 逐字节相同」。
///
/// **偏严**是有意的：动态区域（VU 表、走带光标）一旦进入这些场景，就必须改成「解码 + 遮罩 + 像素容差」，
/// 那需要把 `crates/yeban-ui-test-port/src/png.rs` 里现在位于 `#[cfg(test)] mod tests` 的解码器提到公共位置 ——
/// 这条限制写在 `tests/golden/macos/MANIFEST.txt` 与账本里，不靠"没人发现"。
///
/// **平台无基准时不许静默通过**：打印一行显式声明（"未被判定"而非"通过"），与 `golden.rs` 对
/// `PlatformTag::Other` 的既有规定一致。
fn assert_matches_golden(name: &str, image: &Rgb8Image) {
    if std::env::var("YEBAN_WRITE_GOLDEN").ok().as_deref() == Some("1") {
        return; // 刚写完基准, 不和自己比
    }
    let tag = yeban_ui_test_port::golden::PlatformTag::current();
    let path = yeban_ui_test_port::golden::golden_path(tag, name)
        .unwrap_or_else(|err| panic!("Golden 路径拼装失败: {err}"));
    if !path.exists() {
        observe(&format!(
            "[UI-MCP-003] 平台 `{}` 无基准 `{name}` ⇒ 视觉回归**未被判定**（不等于通过）",
            tag.as_str()
        ));
        return;
    }
    let want = std::fs::read(&path)
        .unwrap_or_else(|err| panic!("读基准 `{}` 失败: {err}", path.display()));
    let got = yeban_ui_test_port::png::encode_rgb8(image);
    assert!(
        got == want,
        "[UI-MCP-003] `{name}` 与基准不一致：基准 {} 字节 / 当前 {} 字节（{}）",
        want.len(),
        got.len(),
        path.display()
    );
    observe(&format!("[UI-MCP-003] `{name}` 与基准逐字节一致 ✓"));
}

/// `[UI-MCP-003]` Golden 基准的**再生成开关**。
///
/// 只有显式设 `YEBAN_WRITE_GOLDEN=1` 时才写基准 —— 否则"跑一次测试"就会把基准悄悄改成当前实现的样子，
/// 那是**最隐蔽的一种假绿**（判据永远绿，因为它每次都拿刚渲染的结果当期望值）。
fn regenerate_golden_if_asked(name: &str, image: &Rgb8Image) {
    if std::env::var("YEBAN_WRITE_GOLDEN").ok().as_deref() != Some("1") {
        return;
    }
    let tag = yeban_ui_test_port::golden::PlatformTag::current();
    let path = yeban_ui_test_port::golden::golden_path(tag, name)
        .unwrap_or_else(|err| panic!("Golden 路径拼装失败: {err}"));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap_or_else(|err| panic!("建 golden 目录失败: {err}"));
    }
    let bytes = yeban_ui_test_port::png::encode_rgb8(image);
    std::fs::write(&path, bytes).unwrap_or_else(|err| panic!("写 Golden 基准失败: {err}"));
    observe(&format!("Golden 基准(再生成): {}", path.display()));
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

/// `ui/tokens.slint` §6 的三个色值 —— **默认主题**（`--theme default` = yeban 调色板,
/// 2026-10-07 起）的那一支。
///
/// 为什么在这里写一份副本：`.slint` 的 `Tokens` global 不保证能从 Rust 侧按名取到，
/// 而"判据需要这几个背景色"不值得为它生成一层绑定代码。这与
/// `src/scene.rs` ↔ `.slint` 的常量重复是同一类 known debt（见 `scene.rs` 的模块文档）。
/// 判据本身钉住了"这些色值确实是面板背景"：下面任何一个背景色写错，
/// 该元素内部的墨迹数都会暴涨（把整块背景都算成墨迹），判据立刻变红。
///
/// **2026-10-07 更新（跟随默认主题）**：本文件的截图来自
/// `host::build_main_window` + **不调** `apply_theme` 的路径 ⇒ 渲染用的就是
/// `ThemeState.theme` 的初值 = 默认主题。默认主题从 `brand` 换成 yeban 之后，
/// 这三支必须换成 **yeban 分支**的取值（否则 `bg-panel-alt` 那一个会整块被算成墨迹:
/// yeban 的 #161e28 与 brand 的 #1b2447 在蓝通道差 31 > `INK_CHANNEL_TOLERANCE` 24）。
/// `brand`（旧默认）的取值是 #060a14 / #1b2447 / #151d38 —— 它们已经不在这里。
const TOKEN_BG_VOID: [u8; 3] = [0x0e, 0x12, 0x16];
/// 见 [`TOKEN_BG_VOID`]。
const TOKEN_BG_PANEL_ALT: [u8; 3] = [0x16, 0x1e, 0x28];
/// 见 [`TOKEN_BG_VOID`]（状态栏的面板色）。
const TOKEN_BG_PANEL: [u8; 3] = [0x13, 0x1a, 0x22];

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
/// （默认主题的 `bg-void` 是 `#0e1216`）；用"非黑"会让整块面板都算成墨迹。
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
const VISIBLE_FAMILY_MEMBERS: [&str; 12] = [
    "clip-01J8Z5Q0R7K3M9X2V4B6N8P1F9-header",
    "note-01J8Z5Q0R7K3M9X2V4B6N8P1A2-rect",
    "piano-roll-tool-select-button",
    "section-0-card",
    "sidebar-category-0-button",
    "sidebar-item-0",
    "tab-piano-roll-button",
    "track-0-header",
    "track-0-mute-button",
    // 自动化泳道（`line/app-automation-ui`）：演示工程里轨道 0 的两条带。
    // 读开的那条证明"曲线进了控件树"，读关的那条证明"读开关在树里可区分"。
    "track-0-automation-volume-lane",
    "track-0-automation-device-0-0-lane",
    "velocity-0-bar",
];

/// 默认视图下**不可见**的语义 ID 样本：它们不得出现在运行时树里 ——
/// 否则说明 `visible:` 的语义没有被尊重（见文件头对上游 `visible.rs` 的核验）。
///
/// ⭐ **`engine-error-banner`（任务单 `#86` 甲）**：它与上面六个
/// （会话视图 / 混音台 / 设备链 / 两个模态）**不是同一类**不可见 —— 上面那些是
/// "**这个视图此刻没被选中**"，而横幅是"**此刻没有错误**"（`visible: text != ""`）。
/// 两者都走同一条上游过滤，因此都该在这里；但它的**判据面**比这一条强得多：
/// `tests/engine_error_banner.rs` 会**造一个坏路径**把同一个 ID 变成可见的、再断言
/// 它的 `accessible-value` 含那段错误原文 ⇒ 覆盖"不显示 ⇄ 显示"的**两个方向**。
///
/// ⭐ **`module-export-midi-button`（任务单 `#90` 甲）在最后一位**：它是**新的第六条模块
/// 入口**，但物理地址在控制台 **Tab 1（调音台）**里 ⇒ 默认帧（Tab 0 卷帘）中不可见。
/// 它在这里不是"顺便加一条"，而是本节要证的那句话本身：**默认帧里没有它**
/// ⇒ `[UI-MCP-003]` 的 5 张 Linux 基准逐位不变（本机 macOS 无法重录）。
/// 反方向（Tab 1 里**有**它、且能被事件注入触发、且回执可读）在
/// `tests/export_midi_ui.rs` 里断 —— 与 `engine-error-banner` 的两个方向同款。
///
/// ⭐ **`module-open-project-button`（任务单 `#93`）紧随其后**：同一条纪律、同一个物理位置
/// （控制台 **Tab 1 = 调音台**，与 `#90` 那条并排）。它是**运行期「打开工程」**的可见入口，
/// 而在**默认帧（Tab 0 卷帘）里一个像素都不画** ⇒ 5 张 Linux 基准逐位不变。
/// 反方向（Tab 1 里**有**它、且注入它的默认无障碍动作**真的换掉工程**、取消不动一位、
/// 坏路径响亮且不毁工程）在 `tests/open_project_ui.rs` 里断。
///
/// ⭐ **`module-new-project-button`（任务单 `#129`）紧随其后**：同一个「工程」面板、
/// 「打开工程」**正下方**（控制台 **Tab 1 = 调音台**）⇒ **默认帧里一个像素都不画**。
/// 反方向（Tab 1 里**有**它、注入默认无障碍动作 ⇒ 问一次保存框 ⇒ 落一个
/// 1148 B／0 音轨 ＋ 1 主总线容器（`#215`：创建时刻已写进文件）⇒ 就地换成新工程；
/// 取消 / 没有对话框两条腿）在
/// `tests/new_project_ui.rs` 里断。
///
/// ⭐ **`module-clipping-warning`（任务单 `#230`）是最后一位**：它是右栏模块入口条里
/// 的**母带削波警示行**（`visible: 属性 != ""`），默认文本是空串 ⇒ 默认帧里不在运行时
/// 树、一个像素都不画。反方向（削波导出之后它在树里、文本逐字、几何在右栏内）在
/// `tests/gui_clipping_warning.rs` 里断。
const DEFAULT_VIEW_MUST_NOT_HAVE: [&str; 13] = [
    "workspace-session-canvas",
    "session-back-to-arrangement-button",
    "scene-launch-column-header",
    "mixer-console",
    "device-rack",
    // ⭐ 任务单 `#180`：主总线设备链是 `DeviceRack` 里面的第二段 ⇒ 默认帧（Tab 0）
    // 同样一个像素都不画。加这一条是把"面板还在 Tab 3"这件事从推断变成机械读数。
    "master-device-rack",
    "musical-pr-drawer",
    "undo-tree-modal",
    "engine-error-banner",
    "module-export-midi-button",
    "module-open-project-button",
    "module-new-project-button",
    "module-clipping-warning",
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

/// 构造注入演示投影的主窗口。
///
/// **与 `src/main.rs` 的 GUI 路径共用 [`host::build_main_window`]** —— 判据断言的界面
/// 就是命令行启动的那个界面，不存在第二份注入实现（`AGENTS.md` §3 DoD 6 的前提）。
fn build_demo_main_window(
    view: &ViewState,
    scene: &DemoScene,
) -> Result<MainWindow, slint::PlatformError> {
    host::build_main_window(view, scene)
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
    let view = ViewState::demo();
    let scene = DemoScene::from_view(&view);
    let registry = demo_registry();
    let static_tree = registry_to_tree(&registry).expect("注册表适配");
    let size = Size::new(scene.viewport_width, scene.viewport_height);

    let mut port = LivePort::new(size, Permission::ReadOnly, Some(&static_tree), || {
        build_demo_main_window(&view, &scene)
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
    let view = ViewState::demo();
    let scene = DemoScene::from_view(&view);
    let registry = demo_registry();
    let static_tree = registry_to_tree(&registry).expect("注册表适配");
    let size = Size::new(scene.viewport_width, scene.viewport_height);
    // 注意：这条判据只用只读方法（`tree()` / `read_property()` / `capture()`），
    // 因此**不能**声明成 `mut` —— `unused_mut` 在 CI 的 `-D warnings` 下是硬错误。
    let port = LivePort::new(size, Permission::ReadOnly, Some(&static_tree), || {
        build_demo_main_window(&view, &scene)
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
         tab-*-button={}, sidebar-item-*={}, piano-roll-tool-*-button={}, \
         track-*-automation-*-lane={} (工程里 {} 条)",
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
        runtime
            .with_prefix("track-")
            .filter(|node| node.id.contains("-automation-") && node.id.ends_with("-lane"))
            .count(),
        view.automation_lanes.len(),
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

    // ---- 自动化泳道：演示工程的**每一条**泳道都在控件树里，且标签携带单位与当前值 ----
    //
    // 这是 `line/app-automation-ui` 要补的那一格："AI 能读到自动化"的机械证据 ——
    // 不是"画了几条线"，而是"每条曲线都有稳定 ID、标签里是模型入口给出的值"。
    // 判据的口径（三条独立证据）：
    //   ① 泳道元素数 == 工程里的泳道数（`automation_lanes` 的条目数）；
    //   ② 每个元素的 `label` 与投影的 `label` **逐字相等**（单位、读/写状态都在里面）；
    //   ③ 标签里的数值 == `automation_value_at(target, 投影的求值 tick)` 的返回值 ——
    //      判据自己调模型的那**一个**入口，因此"第二份插值实现"会当场露馅。
    {
        let demo_project = yeban_app::bridge::demo_project();
        let lanes_in_tree = runtime
            .with_prefix("track-")
            .filter(|node| node.id.contains("-automation-") && node.id.ends_with("-lane"))
            .count();
        assert_eq!(
            lanes_in_tree,
            view.automation_lanes.len(),
            "控件树里的自动化泳道数必须等于工程里的泳道数"
        );
        assert!(
            !view.automation_lanes.is_empty(),
            "演示工程必须有自动化泳道，否则本判据无从执行"
        );
        for lane in &view.automation_lanes {
            let node = runtime.find_by_id(&lane.element_id).unwrap_or_else(|| {
                panic!(
                    "自动化泳道 `{}` 不在运行时树里（树里的泳道: {:?}）",
                    lane.element_id,
                    runtime
                        .with_prefix("track-")
                        .filter(|node| node.id.contains("-automation-"))
                        .map(|node| node.id.clone())
                        .collect::<Vec<_>>()
                )
            });
            assert_eq!(
                node.role.as_str(),
                "image",
                "泳道角色漂移: {}",
                lane.element_id
            );
            assert_eq!(
                node.label, lane.label,
                "控件树标签必须与投影逐字相等（否则 AI 读到的是第二种事实源）"
            );
            // ③ 数值来自**唯一求值入口**（判据自己调模型，而不是读投影的字段）。
            let entry = demo_project
                .automation_value_at(&lane.target, lane.cursor_tick)
                .expect("目标的音轨 / 设备 / 宏必须存在");
            assert_eq!(
                entry, lane.value_at_cursor,
                "`automation_value_at` 与投影字段必须一致: {}",
                lane.element_id
            );
            match entry {
                Some(value) => {
                    if lane.unit == yeban_model::project::AutomationUnit::Decibels {
                        assert!(
                            node.label.contains(&format!("{value:.1} dB")),
                            "dB 泳道的标签必须携带求值入口给出的值: {} vs {:?}",
                            value,
                            node.label
                        );
                    } else {
                        assert!(
                            node.label
                                .contains(&yeban_app::automation::value_text(lane.unit, value)),
                            "标签必须携带求值入口给出的值: {} vs {:?}",
                            value,
                            node.label
                        );
                    }
                }
                None => assert!(
                    node.label.contains("读关闭") || node.label.contains("无采样点"),
                    "求值入口说「没有可用自动化值」时，标签必须显式说明: {:?}",
                    node.label
                ),
            }
            if !lane.read_enabled {
                assert!(
                    node.label.contains("读关闭"),
                    "读关的泳道必须在控件树里可区分: {}= {:?}",
                    lane.element_id,
                    node.label
                );
            }
            if !lane.write_mode.is_off() {
                assert!(
                    node.label.contains(lane.write_label),
                    "录制臂必须在控件树里可读: {}= {:?}",
                    lane.element_id,
                    node.label
                );
            }
            observe(&format!(
                "[automation] 工程泳道 {} -> 控件树 {}.label={:?}",
                lane.element_id, lane.element_id, node.label
            ));
        }
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
    // 对照是状态栏的 `status-bar-chord`（2 个汉字 + 6 个 ASCII，同为 font-size-xs）。
    //
    // ⚠ **2026-10-08（`line/app-r9pix`，台账 R9）**：对照文本由「和弦 Cmaj7」
    //   改成「和弦 (none)」—— `Cmaj7` 是一个**编造的和弦读数**（app 没有和弦识别能力，
    //   `yeban-theory` 不在 `crates/yeban-app/Cargo.toml` 的依赖图上）。
    //   本判据的**断言一个字未改**，改的只是对照样本的文本；ASCII 由 5 个带墨迹的字形
    //   变成 6 个 ⇒ 对照侧墨迹**不减少**，`24/119 = 0.20` 那条"没有字体"的实测读数
    //   仍然抓得到（判据方向与灵敏度都不变）。
    //   ⚠ 对照文本**必须**含 ASCII：若改成纯汉字，在没有 CJK 字体的 runner 上
    //   两侧都会塌到 0，这条判据会从"抓得到"变成"永远绿"。
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
    // 第三块：4 个汉字 + 12 个 ASCII —— 它在"没有 CJK 字体"的环境里**仍然**有墨迹（ASCII 那 12 个），
    // 因此它是"墨迹数不等于汉字数"的对照，也是 §6.2 那张表里唯一两侧都很大的样本。
    let (mixed_ink, _, _) = ink_stats(
        &image,
        inset_rect(
            runtime
                .find_by_id("ai-rail-musical-pr-button")
                .and_then(|node| node.bounds)
                .expect("AI 协作栏的『Musical PR』卡片必须可见"),
            4,
        ),
        TOKEN_BG_PANEL_ALT,
    );
    observe(&format!(
        "[D24] 汉字墨迹: 声学诊断卡(12 汉字+3 ASCII) {cjk_ink} px (包围盒 {cjk_bbox:?}, 颜色 {cjk_colors}); \
         对照 `status-bar-chord`(2 汉字+6 ASCII, 2026-10-08 起对照文本为「和弦 (none)」) {reference_ink} px (包围盒 {reference_bbox:?}, 颜色 {reference_colors}); \
         意图生成卡(14 汉字+2 ASCII) {intent_ink} px; \
         混合卡 Musical PR(4 汉字+12 ASCII) {mixed_ink} px; 下限 {MIN_CJK_INK_PIXELS} px"
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
         没有 CJK 字体时实测是 24/119 = 0.20, 有字体时约为 3–5。\
         ⚠ 2026-10-08（台账 R9）：对照文本由「和弦 Cmaj7」改为「和弦 (none)」，\
         带墨迹的 ASCII 字形由 5 个变成 6 个 ⇒ 上面那对历史实测值（24 / 119）在新文本下\
         的分母**只增不减**：无 CJK 字体时比值更小、**更**容易判红。判据方向不变，\
         也没有放宽。"
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

/// 判据（任务单 `#90` **乙**）：顶栏**保存按钮**不再被 **AI 提案徽章**遮挡。
///
/// ## 判定量（每个数字都有单位）
///
/// | 量 | 对象 | 单位 |
/// | :--- | :--- | :--- |
/// | `bounds` | 运行时控件树里该节点的几何包围盒 | 逻辑像素（x / y / 宽 / 高） |
/// | `overlap_area` | 两个包围盒的交集面积（无交集 ⇒ `0`） | 平方逻辑像素（px²） |
/// | 遮挡宽度 | 交集矩形的宽 | 逻辑像素 |
///
/// ## 为什么这两个 ID（而不是"看着像被挡住"）
///
/// `transport-save-button`（`ui/transport.slint`，`x: 580px`、`w: 56px` ⇒ `580..636`）
/// 与 `transport-ai-proposal-badge`（同文件**声明在后**，后画的盖在上面）在历史版本上
/// 曾经真的相交。`docs/adr/ADR-0003-save-button-top-bar-slot.md` §(c) 记下了那一刻的
/// 机械读数：**56 px 中有 48 px 被遮挡（85.7%）**，徽章占 `x 588..708`。那份读数**就是**
/// 本判据负方向的正对照（"修复前长什么样"）；`9c8d49d2`（"ADR-0003 落地"）把徽章右移到
/// `x: 640px` 之后，交集必须归零。
///
/// ## 它怎么变红（注入读数见交付报告）
///
/// 把 `ui/transport.slint` 里徽章的 `x: 640px` 改回 `x: 588px` ⇒ 交集
/// `x 588..636 × y 8..40` = **48 × 32 = 1536 px²** ⇒ 本判据立刻红。
/// 因此它测的是**同一件事**，不是一个恒真的空断言。
///
/// ⚠ 本判据**只读**运行时树（`LivePort::tree()`，几何来自 `ElementHandle` 的
/// `absolute_position()` + `size()`），⛔ 不解析 `.slint` 源码、⛔ 不看截图猜。
#[test]
fn the_save_button_is_not_occluded_by_the_ai_badge() {
    let view = ViewState::demo();
    let scene = DemoScene::from_view(&view);
    let registry = demo_registry();
    let static_tree = registry_to_tree(&registry).expect("注册表适配");
    let size = Size::new(scene.viewport_width, scene.viewport_height);
    let port = LivePort::new(size, Permission::ReadOnly, Some(&static_tree), || {
        build_demo_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + MainWindow + 运行时控件树");

    let runtime = port.tree();
    let bounds_of = |id: &str| -> Rect {
        runtime
            .find_by_id(id)
            .and_then(|node| node.bounds)
            .unwrap_or_else(|| {
                panic!(
                    "默认帧里 `{id}` 必须有几何（它在 `VISIBLE_SINGLETONS` 里，且是顶栏可见控件）"
                )
            })
    };
    let save = bounds_of("transport-save-button");
    let badge = bounds_of("transport-ai-proposal-badge");
    let area = overlap_area(save, badge);
    let intersection = {
        let x0 = save.x.max(badge.x);
        let y0 = save.y.max(badge.y);
        let x1 = save.right().min(badge.right());
        let y1 = save.bottom().min(badge.bottom());
        if x1 > x0 && y1 > y0 {
            Some(Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32))
        } else {
            None
        }
    };
    observe(&format!(
        "[#90 乙] 保存按钮 bounds={save:?}；AI 提案徽章 bounds={badge:?}；相交={intersection:?}；\
         相交面积={area} px²（历史读数（`9c8d49d2` 之前，徽章 x=588）: 48 × 32 = 1536 px²，遮挡 48/56 px）"
    ));

    assert!(
        save.right() <= badge.x,
        "⛔ 保存按钮（右边缘 {}）必须**完全**在 AI 提案徽章（左边缘 {}）之左 —— \
         后者在 `ui/transport.slint` 里**声明在后**，任何相交都会盖住前者。\
         实测保存按钮 {save:?} / 徽章 {badge:?} / 相交 {intersection:?}。\
         负方向的正对照（`docs/adr/ADR-0003-save-button-top-bar-slot.md` §(c)，徽章在 x=588 时）：\
         遮挡 48/56 px、面积 48 × 32 = 1536 px²。",
        save.right(),
        badge.x
    );
    assert_eq!(
        area, 0,
        "⛔ 保存按钮与 AI 提案徽章的包围盒交集面积必须是 **0 px²**（实得 {area}，相交 {intersection:?}）"
    );
}

/// **本工作线最核心的判据**：模型数据真的从 `YebanProjectV1` 流到了像素。
///
/// 规范来源 (Normative): `[MODEL-AST-002]`（界面只读工程）、`[UI-TEST-001]` §12.2
/// （语义 ID 的 `{ulid}` 段来自实体身份）、`[MUST-GATE-015]`（Golden 必须尺寸非零、非全黑）、
/// `[UI-GRID-001]`（轨道数 / 剪辑数 / 段落数由工程决定）。
///
/// ## 它证明什么（逐条）
///
/// 1. **控件树里出现工程数据**：`track-{i}-header` 的 `accessible-label` 含
///    `filled_project()` 的轨道名（`Lead` / `Bass` / `Aux Reverb`），段落卡片含 `Intro` / `Drop`，
///    剪辑包头 ID 含**工程的摆放身份**（不是演示 ULID）；
/// 2. **演示数据一个都不出现**：整棵树里没有任何标签含 `鼓` / `贝斯`（演示夹具的轨道名）——
///    这一条把"界面到底读的是工程还是 `demo()`"变成可判定的；
/// 3. **像素真的变了**：在**同一个活窗口**上把投影换回演示工程，截图逐字节不同；
/// 4. **两个方向的树都跟着变**：换回演示工程后 `track-5-header` 出现、`track-3-header` 消失。
///
/// ## 为什么在同一个 `LivePort` 上换工程（而不是开两个）
///
/// `slint::platform::set_platform` 每个**线程**只能成功一次（上游 `i-slint-core` 的
/// `GLOBAL_CONTEXT` 是 thread-local `OnceCell`），而 libtest 默认一个测试一个线程。
/// 在同一个活窗口上先注入工程 A、截图、再注入工程 B、再截图，既省一次平台安装，
/// 又让"像素差异"这一条断言**排除了字体/后端等环境差异**（同一进程、同一后端、同一字体）。
/// `[BASELINE-003]` 帧率判据：**10 万音符**滚动下的单帧耗时分布（p50 / p99 / max）+ 非平凡见证。
///
/// **口径**：`ci.yml` **不**断言帧率（托管 runner 读数不算, 见 `spikes/README.md` 第 36 行）；
/// 判决走**手动档 `fps`**, 依 `HD-45`。
// 为什么 `#[ignore]`：本用例跑 **600 帧 × 10 万音符** 的软光栅化循环。CI 的 workspace 腿是 **debug** 构建，
// 实测在托管 runner 上 20 分钟没跑完（run 37348640469 长时间 in_progress）⇒ 它会拖垮整条腿、
// 而它**本来就不该在 CI 上判定帧率**（`spikes/README.md` 第 36 行：托管 runner 读数不算数）。
// 所以：CI 默认跳过（`#[ignore]`），判决由**手动档 `fps`** 用 `--release --ignored --nocapture` 显式运行并收集数字，
// 依 `HD-45`（接受参考机上自适应刷新率的读数）。
#[test]
fn a_synthetic_click_selects_the_note_under_it_and_clears_on_empty_space() {
    // `[UI-NOTE-003]` 选择工具的单击语义, 端到端: 界面报坐标 ⇒ 宿主命中 ⇒ 选区 ⇒ 可观测计数。
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let size = Size::new(scene.viewport_width, scene.viewport_height);
    let registry = registry_to_tree(&ElementRegistry::from_view(&view)).expect("注册表必须能适配");
    let port = LivePort::new(size, Permission::ReadOnly, Some(&registry), || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台");
    let ui = port.ui();

    assert_eq!(ui.get_selected_note_count(), 0, "初始必须无选中");

    // 取一个可见音符, 在它的中心合成一次点击。`invoke_clicked` 绕过 Slint 的坐标解析,
    // 直接送进**与产品同一条** `on_clicked` ⇒ 送的必须是界面回调报的**绘制帧**坐标
    // （`host_to_painted` 是唯一换算；宿主收到后由 `painted_to_host` 搬回）。
    let scroll = ui.get_roll_scroll_x();
    let width = slint::ComponentHandle::window(ui).size().width as f32;
    let visible = view.visible_notes(scroll, width);
    assert!(!visible.is_empty(), "夹具在初始窗口内必须有音符");
    let (cx, cy) = host_to_painted(
        visible.positions[0] + visible.widths[0] / 2.0,
        visible.ys[0] + 3.0,
    );
    ui.invoke_clicked(cx, cy);
    assert_eq!(
        ui.get_selected_note_count(),
        1,
        "点在音符上必须选中它（绘制帧坐标 {cx},{cy}）"
    );

    // 牙: 点在音符**下半截**（相对顶部 +11px）也必须命中 —— 若宿主的高度常量小于 `.slint` 的
    // `height: 12px`, 这一条会红（第 501 轮我第一次写 6.0, 只有这条能抓到）。
    let bottom = view.visible_notes(scroll, width);
    let (bx, by) = host_to_painted(
        bottom.positions[0] + bottom.widths[0] / 2.0,
        bottom.ys[0] + 11.0,
    );
    ui.invoke_clicked(bx, by);
    assert_eq!(
        ui.get_selected_note_count(),
        1,
        "音符下半截必须可命中（高度常量须与 .slint 的 12px 一致）"
    );

    // 被点击的那个音符的 ULID —— 两次点击（中心与下半截）点的都是可见集里的第一个。
    let clicked_ulid = visible.ulids[0].clone();
    // 标志数组必须**恰有一个 true**, 且落在被点音符的下标上（与注入的可见集同一索引集）。
    {
        use slint::Model as _;
        let flags: Vec<bool> = ui.get_note_selected().iter().collect();
        assert_eq!(
            flags.iter().filter(|f| **f).count(),
            1,
            "选中后标志数组必须恰好一个 true"
        );
        assert_eq!(
            flags.len(),
            visible.positions.len(),
            "标志必须与注入数组同长"
        );
        let hit_index = flags.iter().position(|f| *f).expect("恰有一个");
        let visible_now = view.visible_notes(scroll, width);
        assert_eq!(
            visible_now.ulids[hit_index], clicked_ulid,
            "true 的位置必须是被点击的那个音符"
        );
    }

    // 点明显空白 ⇒ 清空。
    ui.invoke_clicked(cx, cy - 500.0);
    assert_eq!(ui.get_selected_note_count(), 0, "点空白必须清空选区");
}

#[test]
fn the_active_tool_is_readable_from_the_host() {
    // `[UI-NOTE-003]` 第 258 轮：工具状态镜像到窗口后, **宿主必须能读回**它 —— 否则"该执行哪个工具"
    // 无从判断（这正是把镜像当成前置的原因）。判据只断言镜像这一环: 写入后读回同一个值。
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let size = Size::new(scene.viewport_width, scene.viewport_height);
    let registry = registry_to_tree(&ElementRegistry::from_view(&view)).expect("注册表必须能适配");
    let port = LivePort::new(size, Permission::ReadOnly, Some(&registry), || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台");
    let ui = port.ui();

    assert_eq!(
        ui.get_active_tool(),
        1,
        "默认必须是选择工具（规范矩阵第 1 行）"
    );
    for tool in 1..=5 {
        ui.set_active_tool(tool);
        assert_eq!(ui.get_active_tool(), tool, "工具 {tool} 未能在宿主侧读回");
    }
}

#[test]
fn published_viewport_bounds_agree_with_the_window_they_came_from() {
    // `[UI-NOTE-001]` 步骤 ① 的**接线**判据（补第 204 轮记的缺口：属性"写了但没人验证"）。
    // 平台设置必须与同文件其它判据一致：`MainWindow::new()` 在裸环境下会在依赖内部 panic
    // （实测：第一次写成直接 `build_main_window` ⇒ panic 落在 slint 的 registry 源码里）。
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let size = Size::new(scene.viewport_width, scene.viewport_height);
    let registry = registry_to_tree(&ElementRegistry::from_view(&view)).expect("注册表必须能适配");
    let port = LivePort::new(size, Permission::ReadOnly, Some(&registry), || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台");
    let ui = port.ui();
    let width = slint::ComponentHandle::window(ui).size().width as f32;

    let (min_tick, max_tick) = view.visible_tick_range(0.0, width);
    let expect_min = min_tick.min(i32::MAX as u64) as i32;
    let expect_max = max_tick.min(i32::MAX as u64) as i32;
    assert_eq!(ui.get_roll_min_tick(), expect_min, "下界必须来自同一窗口");
    assert_eq!(ui.get_roll_max_tick(), expect_max, "上界必须来自同一窗口");
    assert!(
        ui.get_roll_min_tick() <= ui.get_roll_max_tick(),
        "区间必须有序"
    );

    let (min_pitch, max_pitch) = view.visible_pitch_range(16);
    assert_eq!(ui.get_roll_min_pitch(), i32::from(min_pitch));
    assert_eq!(ui.get_roll_max_pitch(), i32::from(max_pitch));
    assert!(
        ui.get_roll_min_pitch() <= ui.get_roll_max_pitch(),
        "音高区间必须有序"
    );
}

#[test]
#[ignore = "600 帧 × 10 万音符的软光栅化循环：CI debug 构建跑不动, 且帧率判决归手动档 fps (HD-45)"]
fn frame_time_under_one_hundred_thousand_notes_is_measured_with_a_witness() {
    const FRAMES: usize = 600;
    let project = yeban_model::samples::project_with_notes(100_000);
    let project_view = ViewState::from_project(&project).expect("10 万音符工程必须能投影");
    let scene = DemoScene::from_view(&project_view);
    // 口径行（账本第 206 轮）：数字必须与**它的 backend 口径**一起被读, 否则无法判断量的是软件还是 GPU 路径。
    // `SLINT_BACKEND` 未设置时打印 `<unset>` —— 这是事实; "未设置 ⇒ 默认后端"是推论, 由读的人结合 runner 判断。
    // 公开 API 里**没有**查询当前 backend/renderer 名字的入口（本轮查过, 未找到）, 所以这里只报环境, 不假装知道更多。
    println!(
        "BASELINE-003 口径: slint_backend={} 尺寸={}x{}",
        std::env::var("SLINT_BACKEND").unwrap_or_else(|_| "<unset>".to_string()),
        scene.viewport_width,
        scene.viewport_height,
    );
    let size = Size::new(scene.viewport_width, scene.viewport_height);
    let registry =
        registry_to_tree(&ElementRegistry::from_view(&project_view)).expect("注册表必须能适配");
    let port = LivePort::new(size, Permission::ReadOnly, Some(&registry), || {
        host::build_main_window(&project_view, &scene)
    })
    .expect("Tier-1 平台 + 10 万音符主窗口");

    let note_count: usize = project
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .map(std::collections::BTreeMap::len)
        .sum();

    let mut samples: Vec<f64> = Vec::with_capacity(FRAMES);
    let mut min_evidence = usize::MAX;
    // 规格（账本第 136 轮）要求 600 帧里**每帧把滚动推进 1/120 秒**。
    // 口径：把"1/120 秒"折算成**一屏宽的 1/120**（即滚动速度 = 1 屏/秒 = 1920 px/s ⇒ 每帧 16 px）。
    // 注入**不计入**计时区间 —— 计时区间只含 request_redraw + capture（渲染 + 回读）。
    let viewport_width = slint::ComponentHandle::window(port.ui()).size().width as f32;
    for frame in 0..FRAMES {
        let scroll_x = frame as f32 * (viewport_width / 120.0);
        host::apply_view(port.ui(), &project_view, viewport_width, scroll_x);
        let start = std::time::Instant::now();
        port.window().request_redraw();
        let image = port.window().capture().expect("每帧都应能抓到像素");
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        let evidence = golden_evidence(&image).expect("见证");
        min_evidence = min_evidence.min(evidence.summary().len());
    }
    samples.sort_by(|a, b| a.partial_cmp(b).expect("无 NaN"));
    let pct = |q: f64| samples[((samples.len() as f64 - 1.0) * q).round() as usize];
    eprintln!(
        "BASELINE-003(10万音符) 帧数={FRAMES} 音符={note_count} p50={:.3}ms p99={:.3}ms max={:.3}ms 见证字符数下限={min_evidence}",
        pct(0.50),
        pct(0.99),
        samples[samples.len() - 1]
    );

    assert_eq!(samples.len(), FRAMES, "必须采满 {FRAMES} 帧");
    assert_eq!(note_count, 100_000, "场景必须是 10 万音符, 不是近似值");
    assert!(min_evidence > 0, "每帧见证都必须有内容");
}

#[test]
fn project_projection_reaches_the_control_tree_and_the_pixels() {
    let project = yeban_model::samples::filled_project();
    let project_view = ViewState::from_project(&project).expect("filled_project 必须能投影");
    let demo_view = ViewState::demo();
    let scene = DemoScene::from_view(&project_view);
    let size = Size::new(scene.viewport_width, scene.viewport_height);
    let registry = registry_to_tree(&ElementRegistry::from_view(&project_view))
        .expect("由工程投影构造的注册表必须能适配");

    let mut port = LivePort::new(size, Permission::ReadOnly, Some(&registry), || {
        host::build_main_window(&project_view, &scene)
    })
    .expect("Tier-1 平台 + 由 YebanProjectV1 驱动的主窗口");

    if port.tree().is_empty() {
        report_capability(
            "unavailable: crates/yeban-app 的 .slint 没有编译期 debug info ⇒ 控件树为空",
        );
        panic!("[UI-TEST-001] 运行时控件树为空 ⇒ 前置条件不满足（ADR-0001 D22）");
    }

    // =====================================================================
    // 第 1 步：把**实测数据**全部打印出来（判据建立在这些数字上）
    // =====================================================================
    let runtime = port.tree().clone();
    observe(&format!(
        "[model-binding] filled_project 投影: 轨道 {} 条 / 段落 {} 条 / 场景 {} 条 / 剪辑 {} 条 / 音符 {} 个 / bpm {} / 拍号 {}",
        project_view.tracks.len(),
        project_view.sections.len(),
        project_view.scenes.len(),
        project_view.clips.len(),
        project_view.note_ulids.len(),
        project_view.bpm_display,
        project_view.time_signature_display,
    ));
    observe(&format!(
        "[model-binding] 运行时控件树 {} 条; track-*-header={}, section-*-card={}, clip-*-header={}",
        runtime.len(),
        runtime
            .with_prefix("track-")
            .filter(|node| node.id.ends_with("-header"))
            .count(),
        runtime
            .with_prefix("section-")
            .filter(|node| node.id.ends_with("-card"))
            .count(),
        runtime
            .with_prefix("clip-")
            .filter(|node| node.id.ends_with("-header"))
            .count(),
    ));
    for (index, track) in project_view.tracks.iter().enumerate() {
        if let Some(node) = runtime.find_by_id(&format!("track-{index}-header")) {
            observe(&format!(
                "[model-binding] 工程字段 TrackV3::name[{}]={:?} -> 控件树 track-{}-header.label={:?}",
                index, track.name, index, node.label
            ));
        }
    }

    // =====================================================================
    // 第 2 步：判据
    // =====================================================================

    // ---- 方向 1: 工程字段 -> 控件树标签（数据真的到了树里） ----
    assert!(
        !project_view.tracks.is_empty(),
        "filled_project 必须有非主总线轨道, 否则本判据无从执行"
    );
    for (index, track) in project_view.tracks.iter().enumerate() {
        let id = format!("track-{index}-header");
        let node = runtime.find_by_id(&id).unwrap_or_else(|| {
            panic!("缺少 `{id}`（工程有 {} 条轨道）", project_view.tracks.len())
        });
        assert!(
            node.label.contains(&track.name),
            "`{id}` 的标签 {:?} 必须含工程的轨道名 {:?}",
            node.label,
            track.name
        );
        assert_eq!(node.role.as_str(), "list-item", "轨道包头角色漂移: {id}");
    }
    // ---- 方向 1b: **轨道色标**同时在控件树里可读（app-completion ②） ----
    //
    // `track-{i}-color-swatch` 的标签携带投影解析后的规范化 `#RRGGBB`：
    // 所以"界面画的颜色"与"自动化读到的色标"是同一个值，而不是两处各写一份。
    for (index, track) in project_view.tracks.iter().enumerate() {
        let id = format!("track-{index}-color-swatch");
        let node = runtime
            .find_by_id(&id)
            .unwrap_or_else(|| panic!("缺少轨道色标 `{id}`"));
        assert_eq!(node.role.as_str(), "image", "色标角色漂移: {id}");
        assert!(
            node.label.contains(&track.color_hex),
            "`{id}` 的标签 {:?} 必须含投影的规范化色标 {:?}",
            node.label,
            track.color_hex
        );
        observe(&format!(
            "[model-binding] 工程字段 TrackV3::color[{}]={:?} -> 控件树 {id}.label={:?}",
            index, track.color, node.label
        ));
    }

    // ---- 方向 1c: **音符位置**来自 tick / 音高，而不是循环下标（app-completion ①） ----
    //
    // 这条断言的口径是**排序关系**而不是绝对像素（绝对像素由纯 Rust 判据钉住）：
    // - x 随 `start_tick` 严格递增（`tick_to_px` 的作用）；
    // - y 随 `pitch` 严格**递减**（音高越高越靠上）。
    // 旧的索引布局（`y: 14px * note_index + 6px`）会让 y 与 pitch **同向**，
    // 因此这条判据能直接抓住"退回索引布局"。
    {
        let mut rows: Vec<(u64, u8, i32, i32)> = Vec::new();
        for note in &project_view.notes {
            let id = format!("note-{}-rect", note.id);
            let node = runtime
                .find_by_id(&id)
                .unwrap_or_else(|| panic!("缺少音符 `{id}`"));
            let bounds = node
                .bounds
                .unwrap_or_else(|| panic!("运行时的 `{id}` 必须有几何包围盒"));
            rows.push((note.start_tick, note.pitch, bounds.x, bounds.y));
            observe(&format!(
                "[model-binding] 工程字段 MidiNote(start={}, pitch={}) -> 控件树 {id} bounds=({}, {})",
                note.start_tick, note.pitch, bounds.x, bounds.y
            ));
        }
        assert!(rows.len() >= 2, "filled_project 必须有多个音符才能判排序");
        let mut by_tick = rows.clone();
        by_tick.sort_by_key(|row| row.0);
        for pair in by_tick.windows(2) {
            assert!(
                pair[0].2 < pair[1].2,
                "音符 x 必须随 start_tick 严格递增（{} → {}）",
                pair[0].2,
                pair[1].2
            );
        }
        let mut by_pitch = rows;
        by_pitch.sort_by_key(|row| row.1);
        for pair in by_pitch.windows(2) {
            assert!(
                pair[0].3 > pair[1].3,
                "音符 y 必须随音高严格递减（音高 {} → {} 的 y: {} → {}）—— 索引布局会相反",
                pair[0].1,
                pair[1].1,
                pair[0].3,
                pair[1].3
            );
        }
    }
    for (index, section) in project_view.sections.iter().enumerate() {
        let id = format!("section-{index}-card");
        let node = runtime
            .find_by_id(&id)
            .unwrap_or_else(|| panic!("缺少 `{id}`"));
        assert!(
            node.label.contains(&section.name),
            "`{id}` 的标签 {:?} 必须含工程的段落名 {:?}",
            node.label,
            section.name
        );
    }
    for clip in &project_view.clips {
        let id = format!("clip-{}-header", clip.placement_id);
        assert!(
            runtime.contains(&id),
            "[UI-TEST-001] 工程的摆放身份 `{}` 必须在控件树里可寻址（找到的 clip-* 有 {:?}）",
            clip.placement_id,
            runtime
                .with_prefix("clip-")
                .map(|node| node.id.clone())
                .collect::<Vec<_>>()
        );
    }
    // 钢琴卷帘（默认可见的控制台页）里的音符身份也必须来自工程 ——
    // 这是"界面里不再有任何内联演示 ULID"的正面断言。
    for ulid in &project_view.note_ulids {
        assert!(
            runtime.contains(&format!("note-{ulid}-rect")),
            "工程的音符身份 `{ulid}` 必须出现在卷帘控件树里"
        );
    }
    for index in 0..project_view.note_velocities.len() {
        assert!(
            runtime.contains(&format!("velocity-{index}-bar")),
            "力度条 `velocity-{index}-bar` 必须与工程的音符逐个对齐"
        );
    }

    // ---- 方向 1e: **自动化泳道**（`line/app-automation-ui`）真的到达控件树 ----
    //
    // 口径与默认视图那条判据完全同源（见 `runtime_control_tree_cross_check_against_the_registry`），
    // 但这里跑的是 `filled_project()`：它只有 **1** 条音量泳道（`Lead`，显式取值域
    // `[-60, 12]`，`SCurve`，录制臂 `Touch`），因此它是"换工程 ⇒ 换泳道"的对照物。
    // 三条断言：
    //   ① 泳道元素数 == 工程的泳道数；
    //   ② 标签与投影逐字相等，且携带单位（`dB`）与**模型入口**在该 tick 的值；
    //   ③ 曲线真的改到了像素（曲线只在自动化泳道 + Path 上画；见下面的截图指纹）。
    {
        let lanes_in_tree = runtime
            .with_prefix("track-")
            .filter(|node| node.id.contains("-automation-") && node.id.ends_with("-lane"))
            .count();
        assert_eq!(
            lanes_in_tree,
            project_view.automation_lanes.len(),
            "控件树里的自动化泳道数必须等于工程里的泳道数"
        );
        assert_eq!(
            project_view.automation_lanes.len(),
            1,
            "`filled_project` 有 1 条音量泳道 —— 这条数字是「换工程」判据的对照物"
        );
        for lane in &project_view.automation_lanes {
            let node = runtime
                .find_by_id(&lane.element_id)
                .unwrap_or_else(|| panic!("缺少自动化泳道 `{}`", lane.element_id));
            assert_eq!(node.role.as_str(), "image");
            assert_eq!(node.label, lane.label);
            assert!(
                node.label.contains("dB"),
                "音量泳道的标签必须携带单位: {:?}",
                node.label
            );
            let entry = project
                .automation_value_at(&lane.target, lane.cursor_tick)
                .expect("Lead 轨必须存在")
                .expect("读开且有点 ⇒ 有值");
            assert_eq!(Some(entry), lane.value_at_cursor);
            assert!(
                node.label.contains(&format!("{entry:.1} dB")),
                "标签里的值必须等于 `automation_value_at` 的返回值: {} vs {:?}",
                entry,
                node.label
            );
            assert!(
                node.label.contains("录制臂 触碰"),
                "`AutomationWriteMode::Touch` 必须进控件树标签: {:?}",
                node.label
            );
            observe(&format!(
                "[automation] filled_project 泳道 {} -> 控件树 label={:?}",
                lane.element_id, node.label
            ));
        }
    }

    // ---- 方向 2: 由**投影驱动**的族里不许出现演示夹具的数据 ----
    //
    // 负向断言**只**覆盖"应当由工程驱动"的语义 ID 族。侧栏资源库（`Sub Bass 低频` /
    // `Night Pad 夜色铺底` …）、混音台通道条、设备机架与两个对话框目前仍是静态标签
    // （见 `docs/ledger/app-binding-notes.md` 的未实现项），对它们做全局断言会**假红** ——
    // 这正是"判据要按事实写、不能为了让数字好看而放宽/收紧"的实例。
    for node in runtime.iter() {
        if !is_model_driven_family(&node.id) {
            continue;
        }
        for name in yeban_app::scene::TRACK_NAMES {
            assert!(
                !node.label.contains(name),
                "由工程驱动的 `{}` 的标签里出现了演示夹具的名字 `{name}`: {:?}",
                node.id,
                node.label
            );
        }
    }
    for ulid in yeban_app::scene::NOTE_ULIDS {
        assert!(
            !runtime.contains(&format!("note-{ulid}-rect")),
            "演示音符 `{ulid}` 不该出现在工程驱动的界面里"
        );
    }

    // ---- 方向 3: 生成的标量属性同样来自工程（像素就是从这些属性画的） ----
    assert_eq!(
        port.ui().get_bpm_display(),
        "128.00",
        "BPM 显示值必须来自 YebanProjectV1::bpm={}",
        project.bpm
    );
    assert_eq!(
        port.ui().get_window_title(),
        "Yeban Model Core Sample",
        "窗口标题必须来自 YebanProjectV1::title={:?}",
        project.title
    );

    // ---- 方向 4: 像素层（[MUST-GATE-015]） ----
    let project_image = port.window().capture().expect("Tier-1 截图");
    let evidence: GoldenEvidence =
        golden_evidence(&project_image).expect("[MUST-GATE-015] 尺寸非零且非全黑");
    assert_eq!(evidence.size, size, "截图尺寸必须等于窗口尺寸");
    assert!(evidence.non_black_pixels > 0, "全黑截图: {evidence:?}");
    assert!(
        evidence.distinct_colors >= 8,
        "颜色过少, 疑似只画了背景: {evidence:?}"
    );
    report_evidence(&evidence);
    observe(&format!(
        "[model-binding] 状态 A (由 filled_project 驱动 / Arrangement): {}",
        evidence.summary()
    ));
    write_png("app-model-driven-filled-project-1920x1080", &project_image);

    // ---- 方向 5: 在**同一个活窗口**上换回演示工程 ⇒ 树与像素都真的变 ----
    host::apply_view(
        port.ui(),
        &demo_view,
        slint::ComponentHandle::window(port.ui()).size().width as f32,
        0.0,
    );
    let demo_image = port.window().capture().expect("演示投影截图");
    assert_ne!(
        demo_image.pixels(),
        project_image.pixels(),
        "换工程必须真的改变像素（否则界面画的不是工程数据）"
    );
    let demo_evidence = golden_evidence(&demo_image).expect("[MUST-GATE-015] 演示投影截图");
    observe(&format!(
        "[model-binding] 状态 B (由 demo_project 驱动 / Arrangement): {}",
        demo_evidence.summary()
    ));
    write_png("app-model-driven-demo-project-1920x1080", &demo_image);

    let demo_registry = registry_to_tree(&ElementRegistry::from_view(&demo_view)).expect("适配");
    let demo_runtime = {
        let tree = port
            .refresh_tree(Some(&demo_registry))
            .expect("切换工程后重新抓运行时不变量");
        tree.clone()
    };
    assert!(
        demo_runtime.contains("track-5-header"),
        "演示工程有 6 条非主总线轨道 ⇒ `track-5-header` 必须出现（工程驱动的树里它不该出现）"
    );
    for node in demo_runtime.iter() {
        if !is_model_driven_family(&node.id) {
            continue;
        }
        assert!(
            !node.label.contains("Lead"),
            "切回演示工程后仍能看到工程轨道名 `Lead`: {}={:?}",
            node.id,
            node.label
        );
    }
    // 两个工程驱动的三族计数必须各自等于**自己那个工程**的规模：
    // 工程 A（filled_project）3/2/2，工程 B（demo_project）6/4/3。
    // 这一对断言把"轨道数/剪辑数/段落数来自工程"从文字变成数字 ——
    // 曾经在这里写反过一次（把"演示树里不该有 track-3-header"当成断言，
    // 而演示工程有 6 条轨道，它**本来就有** track-3-header），
    // CI run 37229239490 把它抓成红。判据是可失败的，这次是它自己犯了错。
    let family_count = |tree: &ControlTree, prefix: &str, suffix: &str| -> usize {
        tree.with_prefix(prefix)
            .filter(|node| node.id.ends_with(suffix))
            .count()
    };
    for (tree, label, tracks, sections, clips) in [
        (
            &runtime,
            "filled_project",
            project_view.tracks.len(),
            project_view.sections.len(),
            project_view.clips.len(),
        ),
        (
            &demo_runtime,
            "demo_project",
            demo_view.tracks.len(),
            demo_view.sections.len(),
            demo_view.clips.len(),
        ),
    ] {
        assert_eq!(
            family_count(tree, "track-", "-header"),
            tracks,
            "{label}: 控件树里的轨道包头数必须等于工程的轨道数"
        );
        assert_eq!(
            family_count(tree, "section-", "-card"),
            sections,
            "{label}: 章节卡片数必须等于工程的段落数"
        );
        assert_eq!(
            family_count(tree, "clip-", "-header"),
            clips,
            "{label}: 剪辑包头数必须等于工程的剪辑摆放数"
        );
    }
    assert_eq!(project_view.tracks.len(), 3);
    assert_eq!(demo_view.tracks.len(), 6);
    // 色标族规模 == 工程轨道规模（两侧都判），且**缺色**轨道的标签是回退色。
    //
    // ⚠ 只看 **arrangement** 的色标：app-mixer 之后调音台也有 `track-{i}-mixer-color-swatch`
    // （同一个投影字段，不同的语义 ID）。它在**注册表**里恒存在，而"`visible: false` 的分支
    // 是否进运行时树"是上游几何遍历的结论 —— 本判据不假设哪一种，只把计数口径收窄到
    // arrangement（与 `elements.rs` 的注册表版本一致）。
    let arrangement_swatches = |tree: &ControlTree| -> usize {
        tree.with_prefix("track-")
            .filter(|node| node.id.ends_with("-color-swatch"))
            .filter(|node| !node.id.contains("-mixer-"))
            .count()
    };
    for (tree, view, label) in [
        (&runtime, &project_view, "filled_project"),
        (&demo_runtime, &demo_view, "demo_project"),
    ] {
        assert_eq!(
            arrangement_swatches(tree),
            view.tracks.len(),
            "{label}: 色标元素数必须等于工程的轨道数"
        );
        for (index, track) in view.tracks.iter().enumerate() {
            let id = format!("track-{index}-color-swatch");
            let node = tree
                .find_by_id(&id)
                .unwrap_or_else(|| panic!("{label}: 缺少 `{id}`"));
            assert!(
                node.label.contains(&track.color_hex),
                "{label}: `{id}` 的标签 {:?} 必须含 {:?}",
                node.label,
                track.color_hex
            );
            if track.color.is_none() {
                assert!(
                    node.label
                        .contains(yeban_app::bridge::DEFAULT_TRACK_COLOR_HEX),
                    "{label}: 缺色轨道 `{id}` 必须用文档回退色"
                );
            }
        }
    }
    assert_eq!(
        demo_view
            .tracks
            .iter()
            .filter(|track| track.color.is_none())
            .count(),
        3,
        "演示工程有 3 条缺色轨道 —— 回退路径必须真的被执行到"
    );
    assert!(
        !runtime.contains("track-3-header"),
        "filled_project 只有 3 条非主总线轨道 ⇒ 工程驱动的树里不该有 `track-3-header`"
    );
    assert!(
        demo_runtime.contains("track-3-header"),
        "演示工程有 6 条轨道 ⇒ 演示驱动的树里**必须**有 `track-3-header`"
    );
    // ---- 自动化泳道也随工程切换（同一活窗口；判据 ⑥ 的运行时侧） ----
    let lanes_in = |tree: &ControlTree| -> usize {
        tree.with_prefix("track-")
            .filter(|node| node.id.contains("-automation-") && node.id.ends_with("-lane"))
            .count()
    };
    assert_eq!(
        lanes_in(&runtime),
        project_view.automation_lanes.len(),
        "工程 A（filled_project）的泳道数"
    );
    assert_eq!(
        lanes_in(&demo_runtime),
        demo_view.automation_lanes.len(),
        "工程 B（demo_project）的泳道数 —— 同一活窗口上换工程必须换泳道"
    );
    assert_ne!(
        lanes_in(&demo_runtime),
        lanes_in(&runtime),
        "两个工程的泳道数必须有差别（1 vs 3），否则「泳道跟着工程走」没有被测到"
    );
    // 读关的设备参数泳道**属于**演示工程（`demo_project` 的轨道 0 有两条带），
    // 而 `filled_project` 只有一条音量泳道 —— 两个方向都断言。
    // 实测教训：这条断言的第一版把两个树写反了（对 `demo_runtime` 断言"不得出现"），
    // CI run 37249203359 当场把它抓成红（`test_port_adapter.rs:1700`）。
    assert!(
        demo_runtime.contains("track-0-automation-device-0-0-lane"),
        "演示工程有读关的设备参数泳道 ⇒ 必须在演示树里"
    );
    assert!(
        !runtime.contains("track-0-automation-device-0-0-lane"),
        "`filled_project` 只有一条音量泳道 ⇒ 设备参数泳道不得出现在它的树里"
    );
    observe(&format!(
        "[model-binding] 切换后运行时控件树 {} 条; 工程驱动的树 {} 条 —— 两者必须不同",
        demo_runtime.len(),
        runtime.len()
    ));
    assert_ne!(
        demo_runtime.ids().collect::<Vec<_>>(),
        runtime.ids().collect::<Vec<_>>(),
        "两个工程驱动的语义 ID 集合必须不同"
    );
}

// ===========================================================================
// 走带接线（`line/transport-engine`）：**回调真的驱动引擎**的判据
// ===========================================================================
//
// 这一节回答的是 `docs/ledger/feature-alignment.md` 错位 7 登记的那个方法论问题：
// 「控件存在 ≠ 回调接线」。「`transport-play-button` 在控件树里」**不是**证据 ——
// 上一版 `main.rs` 的 9 个回调全部指向只打 stderr 的 `trace()`，控件树、截图、
// `ui/coverage` 四个视角看都"有"，点下去什么都不动。
//
// 因此这里的证据是**动作记录**：一条真的改变过引擎状态、并且被引擎自己的读数
// （状态 + 位置）确认的命令。判据同时给出**负向对照**（一个没接线的窗口：
// 同样的 `invoke_toggle_play()` 之后引擎与显示态都必须一位不动）——
// 没有这个对照，"引擎变了"可能只是夹具自己在动。

/// 走带判据的夹具：真的建一代引擎 + 一个真的活窗口，并把两者按产品路径接起来。
struct TransportHarness {
    /// 活窗口（Tier-1 软件光栅化平台，与 `main.rs` 共用 `host::build_main_window`）。
    port: LivePort<MainWindow>,
    /// 引擎宿主（UI 线程持有；与 `main.rs` 的 GUI 路径同款）。
    engine: Rc<RefCell<EngineHost>>,
    /// 活窗口上那一份**投影**（判据要在它身上对时间码读数：投影是唯一事实源）。
    view: ViewState,
}

impl TransportHarness {
    /// 建夹具：`filled_project` 投影 → 引擎重建（0 量子）→ 活窗口 → 接线。
    fn new() -> Self {
        let project = yeban_model::samples::filled_project();
        let view = ViewState::from_project(&project).expect("filled_project 必须能投影");
        let scene = DemoScene::from_view(&view);
        let size = Size::new(scene.viewport_width, scene.viewport_height);

        let mut engine = EngineHost::new();
        engine.reload(&project, 0).expect("引擎重建");
        // `reload` **不**改走带状态（它沿用"自由跑"默认值，契约见 `EngineRebuild::transport`）；
        // "加载即停住"是控制面的显式动作 —— `main.rs` 的 GUI 路径做的是同一件事。
        assert_eq!(
            engine.transport().state,
            TransportState::Playing,
            "新引擎沿用自由跑的默认值（reload 不碰走带）"
        );
        engine.stop();
        assert_eq!(
            engine.transport().state,
            TransportState::Stopped,
            "显式 Stop 之后必须停在 Stopped, 否则第一次点播放的语义是\"暂停\""
        );
        let engine = Rc::new(RefCell::new(engine));

        let port = LivePort::new(size, Permission::Interactive, None, || {
            host::build_main_window(&view, &scene)
        })
        .expect("Tier-1 平台 + 主窗口");

        host::wire_transport(port.ui(), Rc::clone(&engine), None, None);
        // 接线之后先把引擎读数注入一次界面（`main.rs` 的 GUI 路径做的是同一件事）。
        host::apply_transport(port.ui(), engine.borrow().transport());
        Self { port, engine, view }
    }

    /// 引擎当前读数。
    fn reading(&self) -> TransportReading {
        self.engine.borrow().transport()
    }

    /// 引擎的走带动作日志。
    fn journal(&self) -> Vec<TransportActionRecord> {
        self.engine.borrow().transport_journal().to_vec()
    }

    /// 把**键盘事件源**接上（`N2` 裁决 (1) 的 GUI 路径）。
    ///
    /// 它调的是产品路径（`main.rs` 的 `wire_input` + `wire_keys`）与判据装配
    /// （`live_surface.rs` 的 `build_live_ui_with`）调的**同一对**函数，因此经
    /// `ui/dispatch_key_press` 注入的 `WindowEvent::KeyPressed` 走的就是用户按键那条链。
    ///
    /// 返回值是那个 `InputContext`：判据必须**先断言**前置条件（主画布焦点、非合成态）
    /// 再注入按键 —— 否则"这一键被输入法吞掉"会被误读成"走带接线坏了"。
    ///
    /// ⚠ 撤销端口传 `None`：走带不经过撤销族（`playing` 是显示态，不是工程内容），
    /// 因此本夹具不需要撤销会话。
    fn wire_keyboard(&self) -> Rc<RefCell<yeban_app::input::InputContext>> {
        let context = Rc::new(RefCell::new(yeban_app::input::InputContext::new()));
        host::wire_input(self.port.ui(), Rc::clone(&context));
        host::wire_keys(self.port.ui(), Rc::clone(&context), None, None, None);
        context
    }

    /// 把引擎定位到 `ticks` 并**重新注入**一次走带读数（产品路径同款：
    /// `host::apply_transport` 是唯一写 `timecode` 的地方）。
    fn seek_and_inject(&mut self, ticks: u64) -> TransportReading {
        self.engine.borrow_mut().seek(ticks);
        let reading = self.reading();
        host::apply_transport(self.port.ui(), reading);
        reading
    }

    /// 把一份**同工程、不同拍号**的投影重新注入同一个活窗口（拍号改变的唯一路径：
    /// `host::apply_view` —— 换工程 / 撤销走的就是它）。
    ///
    /// 返回值是重新投影出来的 `ViewState`：拍号改变的**两方向**断言要拿它算期望读数。
    fn apply_time_signature(&mut self, numerator: u8, denominator: u8) -> ViewState {
        let mut project = yeban_model::samples::filled_project();
        project.time_signature = yeban_model::project::TimeSignature {
            numerator,
            denominator,
        };
        let view = ViewState::from_project(&project).expect("改拍号后的工程必须能投影");
        host::apply_view(
            self.port.ui(),
            &view,
            slint::ComponentHandle::window(self.port.ui()).size().width as f32,
            0.0,
        );
        view
    }
}

/// 判据 ⑧ + ⑨（**本切片的核心里程碑**）：`toggle-play` / `stop` 两个回调
/// **真的**驱动引擎走带，而且 `playing` 显示态**双向**跟着引擎走。
#[test]
fn transport_callbacks_really_drive_the_engine_and_the_display_follows_it() {
    let harness = TransportHarness::new();
    let ui = harness.port.ui();

    // ---- 方向 A：界面回调 ⇒ 引擎状态变化（不是"控件树里有这个元素"）----
    assert!(!ui.get_playing(), "初始显示态是停住");
    assert_eq!(harness.reading().state, TransportState::Stopped);

    ui.invoke_toggle_play();
    let after_play = harness.reading();
    assert_eq!(
        after_play.state,
        TransportState::Playing,
        "`toggle-play` 必须真的把引擎推进到 Playing"
    );
    assert!(ui.get_playing(), "显示态必须跟着引擎变成\"播放中\"");
    assert!(after_play.position_ticks > 0, "播放必须真的推进 tick");
    assert_eq!(
        ui.get_timecode(),
        harness.view.timecode_at(after_play.position_ticks),
        "时间码必须等于**投影**在这个 tick 上的读数（不是界面自己算的一份）"
    );

    // **动作记录**（注入点）：命令、命令后的引擎状态与位置、推了几个量子。
    //
    // ⚠ 就地改正（2026-10-08，走带票）：本行原文写「第 0 条是 `EngineHost::reload`
    // 末尾**自动**发的那条 `Stop`」—— **与事实不符**。`reload` 的第 5 步明文
    // 「**刻意不做**：`reload` 不碰走带状态」（`src/engine_host.rs` 的 `reload` 文档，
    // 原因写在那里：顺手发 `Stop` 会多推一个量子，`ui/reload_engine` 的 `quanta` 读数当场错）。
    // 第 0 条其实是 `TransportHarness::new` 在 `reload` 之后**显式**调的那次
    // `engine.stop()`（"加载即停住"是控制面的显式动作）。条数断言（2 / 3 / 5）本来就是对的，
    // 错的只有归属；本票新增的判据要按"日志的真实组成"算增量，因此这里一并说清。
    let journal = harness.journal();
    assert_eq!(
        journal.len(),
        2,
        "`TransportHarness::new` 的显式 Stop + 本次点击的 Play"
    );
    assert_eq!(journal[0].command, TransportCommand::Stop);
    assert_eq!(journal[0].state_after, TransportState::Stopped);
    assert_eq!(journal[1].command, TransportCommand::Play);
    assert_eq!(journal[1].state_after, TransportState::Playing);
    assert_eq!(journal[1].position_ticks_after, after_play.position_ticks);
    assert_eq!(journal[1].quanta_pumped, 1, "命令必须在量子边界被应用");
    observe(&format!(
        "[transport] toggle-play ⇒ 记录[Play] 状态={:?} tick={} 推量子={} 显示态={} 时间码={}",
        journal[1].state_after,
        journal[1].position_ticks_after,
        journal[1].quanta_pumped,
        ui.get_playing(),
        ui.get_timecode(),
    ));

    // ---- 再点一次 ⇒ 停住（位置保留）----
    let playing_position = after_play.position_ticks;
    ui.invoke_toggle_play();
    let after_stop = harness.reading();
    assert_eq!(
        after_stop.state,
        TransportState::Stopped,
        "再点一次必须停住"
    );
    assert!(!ui.get_playing(), "显示态必须跟着回到停住");
    assert_eq!(
        after_stop.position_ticks, playing_position,
        "引擎的 `Stop` 保留位置（\"回到起点\"是界面停止按钮的语义, 见下一条判据）"
    );
    assert_eq!(
        harness.journal().len(),
        3,
        "夹具的显式 Stop + Play + 本次 Stop"
    );

    // ---- `stop` 回调：停住 + 回到起点（与 `transport.slint` 的 accessible-label 一致）----
    ui.invoke_stop();
    let rewound = harness.reading();
    assert_eq!(rewound.state, TransportState::Stopped);
    assert_eq!(rewound.position_ticks, 0, "停止按钮必须回到 tick 0");
    assert_eq!(ui.get_timecode(), "001.01.000");
    let journal = harness.journal();
    assert_eq!(journal.len(), 5, "`stop_and_rewind` 是批量里的两条命令");
    assert_eq!(journal[3].command, TransportCommand::Stop);
    assert_eq!(journal[4].command, TransportCommand::SeekTicks(0));
    assert_eq!(journal[4].position_ticks_after, 0);
    assert_eq!(
        journal[3].quanta_pumped, 1,
        "两条命令在**同一个**量子边界一起生效（一次批量推一个量子）"
    );

    // ---- 方向 B：直接改引擎状态 ⇒ 显示态跟着变（显示态没有自己的状态机）----
    harness.engine.borrow_mut().play();
    host::apply_transport(ui, harness.reading());
    assert!(
        ui.get_playing(),
        "引擎进入 Playing ⇒ 显示态必须跟着变（bound 方向的反向）"
    );
    harness.engine.borrow_mut().stop_and_rewind();
    host::apply_transport(ui, harness.reading());
    assert!(!ui.get_playing(), "引擎回到 Stopped ⇒ 显示态必须跟着回来");
    assert_eq!(ui.get_timecode(), "001.01.000");

    // 走带推进会让**截图像素**变化（两次截图不同 ⇒ 这条链真的到了像素一侧）。
    let stopped_shot = harness.port.window().capture().expect("Tier-1 截图");
    harness.engine.borrow_mut().seek(8_000);
    host::apply_transport(ui, harness.reading());
    assert_eq!(ui.get_timecode(), harness.view.timecode_at(8_000));
    let moved_shot = harness.port.window().capture().expect("Tier-1 截图");
    assert_ne!(
        stopped_shot.pixels(),
        moved_shot.pixels(),
        "走带位置到了 8000 tick ⇒ 时间码液晶屏的像素必须变化"
    );
    observe(&format!(
        "[transport] 时间码 0 -> {} 使截图像素变化 ({} 字节 vs {} 字节)",
        harness.view.timecode_at(8_000),
        stopped_shot.pixels().len(),
        moved_shot.pixels().len(),
    ));
}

/// 判据 ⑩（`[MODEL-AST-001]` 时间码 × 工程拍号）：**三种拍号**下同一批 tick 位置的
/// `小节.拍.tick` 读数正确，且**读数等于投影算出的读数**（投影是唯一事实源）。
///
/// 这条判据的判别力来自"两个独立来源必须一致"：
/// - 期望值是一张**手算的表**（4/4 / 3/4 / 6/8 各三个位置，见下面的字面量）；
/// - 实测值来自活窗口的 `timecode` 属性，而它是由 `apply_view` 注入的两个整数
///   （`timecode-ticks-{beat,bar}`）经 `bridge::timecode_for_ticks` 算出来的。
///
/// 若界面（host.rs）自己按写死的 4/4 算一份，3/4 与 6/8 的断言立刻变红
/// （本线实测过：把 `timecode_grid_of` 换成写死 4/4 之后，本判据在
/// `002.02.000` / `003.05.320` 两处失败 —— 见 ledger 的注入记录）。
#[test]
fn timecode_reads_the_projected_time_signature_for_three_signatures() {
    let mut harness = TransportHarness::new();
    // 手算表：(分子, 分母, tick, 期望读数)
    //   4/4: 一拍 960, 一小节 3840  ⇒ 3840 = 002.01.000；8000 = 003.01.320；960 = 001.02.000
    //   3/4: 一拍 960, 一小节 2880  ⇒ 3840 = 002.02.000；8000 = 003.03.320；960 = 001.02.000
    //   6/8: 一拍 480, 一小节 2880  ⇒ 3840 = 002.03.000；8000 = 003.05.320；960 = 001.03.000
    let cases: &[(u8, u8, u64, &str)] = &[
        (4, 4, 960, "001.02.000"),
        (4, 4, 3_840, "002.01.000"),
        (4, 4, 8_000, "003.01.320"),
        (3, 4, 960, "001.02.000"),
        (3, 4, 3_840, "002.02.000"),
        (3, 4, 8_000, "003.03.320"),
        (6, 8, 960, "001.03.000"),
        (6, 8, 3_840, "002.03.000"),
        (6, 8, 8_000, "003.05.320"),
    ];

    for &(numerator, denominator, ticks, expected) in cases {
        let view = harness.apply_time_signature(numerator, denominator);
        harness.seek_and_inject(ticks);
        let displayed = harness.port.ui().get_timecode();
        assert_eq!(
            displayed, expected,
            "{numerator}/{denominator} 的 tick {ticks} 读数必须是 {expected}（手算表）"
        );
        assert_eq!(
            displayed,
            view.timecode_at(ticks),
            "{numerator}/{denominator} 的 tick {ticks}: 界面读数必须等于**投影**读数"
        );
        // 注入面本身也要对得上：窗口上那两个整数就是投影算出的网格。
        assert_eq!(
            harness.port.ui().get_timecode_ticks_bar(),
            i32::try_from(view.bar_length_ticks).expect("小节长度落在 i32"),
            "注入的小节 tick 数必须来自投影"
        );
        assert_eq!(
            harness.port.ui().get_timecode_ticks_beat(),
            i32::try_from(view.bar_length_ticks / u64::from(numerator)).expect("一拍 tick 数"),
            "注入的每拍 tick 数必须来自投影（= 小节长度 ÷ 分子）"
        );
        observe(&format!(
            "[timecode] {numerator}/{denominator} tick={ticks} ⇒ 界面={displayed} 投影={} 小节={} 拍={}",
            view.timecode_at(ticks),
            view.bar_length_ticks,
            view.bar_length_ticks / u64::from(numerator),
        ));
    }
}

/// 判据 ⑪（**拍号改变 ⇒ 同一个 tick 的读数随之变**，两个方向都断）：
/// 引擎位置一位不动，只把 `3/4` 的投影重新注入同一个活窗口，读数必须变；
/// 再注入回 `4/4`，读数必须变回来。
#[test]
fn changing_the_time_signature_changes_the_reading_at_the_same_tick_both_ways() {
    let mut harness = TransportHarness::new();
    let tick = 3_840_u64;

    let four_four = harness.view.timecode_at(tick);
    harness.seek_and_inject(tick);
    let displayed_four_four = harness.port.ui().get_timecode();
    assert_eq!(displayed_four_four, four_four);
    assert_eq!(displayed_four_four, "002.01.000");

    // 方向 A：4/4 → 3/4（**位置不变**）
    let three_four_view = harness.apply_time_signature(3, 4);
    let reading_after = harness.seek_and_inject(tick);
    assert_eq!(
        reading_after.position_ticks, tick,
        "换拍号**不许**动引擎位置（拍号不是位置）"
    );
    let displayed_three_four = harness.port.ui().get_timecode();
    assert_eq!(displayed_three_four, three_four_view.timecode_at(tick));
    assert_eq!(displayed_three_four, "002.02.000");
    assert_ne!(
        displayed_three_four, displayed_four_four,
        "同一个 tick 在 3/4 与 4/4 下的读数必须不同（否则时间码根本没读拍号）"
    );

    // 方向 B：3/4 → 4/4（回到原投影，读数必须回到原值）
    let back = harness.apply_time_signature(4, 4);
    harness.seek_and_inject(tick);
    let displayed_back = harness.port.ui().get_timecode();
    assert_eq!(displayed_back, back.timecode_at(tick));
    assert_eq!(
        displayed_back, displayed_four_four,
        "方向 B：读数必须变回来"
    );
    assert_ne!(displayed_back, displayed_three_four);
    observe(&format!(
        "[timecode] 同一 tick {tick}: 4/4={displayed_four_four} 3/4={displayed_three_four} 回到 4/4={displayed_back}（引擎位置始终 {tick}）"
    ));
}

/// 判据 ⑫（`[UI-A11Y-002]` §7.2）：IME 事件源（`.slint` 的 `TextInput` 回调）
/// **真的**驱动那个唯一的状态机 —— 两态可区分，而且被吞掉的按键是**行为**断言。
///
/// 这里注入的是 `.slint` 声明的回调（`invoke_ime_composition_changed` /
/// `invoke_ime_focus_changed`），也就是 `ui/transport.slint` 的 `bpm-input` 在
/// `changed preedit-text` / `changed has-focus` 里调的那两个入口 ——
/// 判据不打桩、不写状态机，只驱动事件源那一侧。
#[test]
fn ime_event_source_drives_the_input_context_and_swallows_bare_shortcuts() {
    let view = ViewState::from_project(&yeban_model::samples::filled_project()).expect("投影");
    let scene = DemoScene::from_view(&view);
    let size = Size::new(scene.viewport_width, scene.viewport_height);
    let port = LivePort::new(size, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 主窗口");
    let context = Rc::new(RefCell::new(yeban_app::input::InputContext::new()));
    host::wire_input(port.ui(), Rc::clone(&context));
    let ui = port.ui();

    // 起点：画布聚焦、非合成 ⇒ Space 是走带播放/暂停。
    assert!(!context.borrow().is_composing());
    assert_eq!(
        context.borrow().resolve(
            yeban_app::input::PhysicalKey::Space,
            yeban_app::input::Modifiers::none()
        ),
        yeban_app::input::Resolution::Action(yeban_app::input::Action::PlayPause),
        "非合成态下 Space 必须命中走带"
    );

    // 焦点进入敲入控件（`bpm-input` 的 `changed has-focus`）。
    ui.invoke_ime_focus_changed(true);
    assert_eq!(context.borrow().focus(), yeban_app::input::Focus::TextInput);

    // 合成态置位（`changed preedit-text`：候选词非空）。
    ui.invoke_ime_composition_changed(true);
    assert!(
        context.borrow().is_composing(),
        "preedit 非空 ⇒ 状态机必须进入合成态（false → true）"
    );
    // **行为**断言：合成态下这一键被输入法吞掉，绝不冒泡成 DAW 快捷键。
    assert_eq!(
        context.borrow().resolve(
            yeban_app::input::PhysicalKey::Space,
            yeban_app::input::Modifiers::none()
        ),
        yeban_app::input::Resolution::ConsumedByIme,
        "§7.2 MUST：合成态下 Space 必须被吞掉（不是\"控件存在\"）"
    );

    // 上屏 / 取消（preedit 清空）⇒ 合成态清零。焦点**仍在**敲入控件，因此裸快捷键
    // 按 `input.rs` 的既有语义交给文本控件（"打字"而不是"DAW 动作"）。
    ui.invoke_ime_composition_changed(false);
    assert!(
        !context.borrow().is_composing(),
        "preedit 清空 ⇒ 状态机必须离开合成态（true → false）"
    );
    assert_eq!(
        context.borrow().resolve(
            yeban_app::input::PhysicalKey::Space,
            yeban_app::input::Modifiers::none()
        ),
        yeban_app::input::Resolution::PassThrough,
        "焦点在敲入控件上时 Space 必须交给文本控件（非合成态也是打字）"
    );

    // 失焦 ⇒ 焦点回画布；**合成中直接失焦**也必须清零（`set_focus` 的既有语义兜底）。
    ui.invoke_ime_focus_changed(false);
    assert_eq!(
        context.borrow().focus(),
        yeban_app::input::Focus::MainCanvas
    );
    assert_eq!(
        context.borrow().resolve(
            yeban_app::input::PhysicalKey::Space,
            yeban_app::input::Modifiers::none()
        ),
        yeban_app::input::Resolution::Action(yeban_app::input::Action::PlayPause),
        "焦点回画布之后 Space 必须重新命中走带（否则守卫会永久吞键）"
    );
    ui.invoke_ime_composition_changed(true);
    assert!(context.borrow().is_composing());
    ui.invoke_ime_focus_changed(false);
    assert!(
        !context.borrow().is_composing(),
        "合成中失焦必须结束合成态（否则一次丢事件会把单键热键永久吞掉）"
    );
    observe(&format!(
        "[ime] 事件源(Slint 回调) → InputContext: 合成 false→true→false 全部对得上; \
         合成态下 Space={:?} 焦点在敲入控件时 Space={:?} 焦点回画布后 Space={:?}",
        yeban_app::input::Resolution::ConsumedByIme,
        yeban_app::input::Resolution::PassThrough,
        yeban_app::input::Resolution::Action(yeban_app::input::Action::PlayPause),
    ));
}

/// 判据 ⑬（**如实 SKIP，不静默通过**）：没有接线时事件源一位都改不动状态机；
/// 接上之后同一个调用立刻生效 —— 因此判据 ⑫ 的绿**只能**来自 `host::wire_input`。
///
/// ## 这条判据同时打印本线**做不到**的那一半（SKIP 原因）
///
/// 上游（Slint 1.18.1）**有** IME 合成信号，但没有**公开的注入面**：
/// - `WindowEvent` 是 `#[non_exhaustive]` 的公开枚举，**没有**合成变体；
/// - 真正携带 preedit 的 `i_slint_core::input::InternalKeyEvent` / `KeyEventType`
///   **没有**被 `slint` 或 `i-slint-backend-testing` 再导出（`slint` 的
///   `private_unstable_api::re_exports` 只导出了 `input::{FocusEvent, KeyEvent, …}`，
///   其中不含这两个类型），而 `WindowInner::process_key_input` 是 `pub(crate)`；
/// - 因此判据**无法**在 Tier-1 平台上合成一次 `Ime::Preedit`，只能驱动到
///   `.slint` 回调这一层（= 事件源的下游一格）。
///
/// 本线**不**为此加依赖（`i-slint-core` 不是本 crate 的依赖，加它就是"新增依赖"，
/// 违反本线的约束）。这条限制如实登记在
/// `docs/ledger/app-projection-notes.md` 的 needs 里，并被下面的 `observe` 打印出来
/// —— 而不是让一条"元素存在"的断言冒充端到端证据。
#[test]
fn an_unwired_ime_event_source_changes_nothing_and_the_skip_is_reported() {
    let view = ViewState::from_project(&yeban_model::samples::filled_project()).expect("投影");
    let scene = DemoScene::from_view(&view);
    let size = Size::new(scene.viewport_width, scene.viewport_height);
    let port = LivePort::new(size, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 主窗口");
    let context = Rc::new(RefCell::new(yeban_app::input::InputContext::new()));
    let ui = port.ui();

    // ---- 负向对照：没有 `wire_input` 时，事件源**一位都改不动** ----
    ui.invoke_ime_focus_changed(true);
    ui.invoke_ime_composition_changed(true);
    assert!(
        !context.borrow().is_composing(),
        "没有接线时合成态必须**不动**（否则判据 ⑫ 的绿可能来自别处）"
    );
    assert_eq!(
        context.borrow().focus(),
        yeban_app::input::Focus::MainCanvas,
        "没有接线时焦点也必须不动"
    );

    // ---- 同一对象接上之后，同一个调用立刻生效 ----
    host::wire_input(ui, Rc::clone(&context));
    ui.invoke_ime_focus_changed(true);
    ui.invoke_ime_composition_changed(true);
    assert!(
        context.borrow().is_composing(),
        "接线之后同一个 Slint 回调必须驱动状态机 ⇒ `wire_input` 是唯一边"
    );

    // ---- SKIP：平台级 preedit 注入在本仓库的依赖集下做不到（原因见文档注释）----
    observe(
        "[ime-skip] 平台级 `Ime::Preedit` 注入在 Slint 1.18.1 上没有公开入口 \
         (`WindowEvent` 无合成变体; `InternalKeyEvent`/`KeyEventType` 未被再导出; \
         `WindowInner::process_key_input` 是 pub(crate); 加 `i-slint-core` 依赖被本线约束禁止) \
         ⇒ 判据覆盖到 `.slint` 回调这一格; 上游信号本身的接线证据 = \
         ui/transport.slint 的 `changed preedit-text`（本线源码级核验），平台级注入记为 needs",
    );
}

/// **负向对照**（判据 ⑧ 的判别力证明）：一个**没有接线**的活窗口上，
/// 同样的 `invoke_toggle_play()` 既不改引擎、也不改显示态。
///
/// 这正是上一版 `main.rs` 的形态（`on_toggle_play(|| trace("toggle-play"))`）
/// 加上 `app.slint` 里那句 `root.playing = !root.playing;` 的合并后果 ——
/// 界面看起来会"变"，但引擎一位不动。删掉自翻转之后，两个方向都必须**不动**：
/// 显示态不再是界面自造的，引擎也不会被不存在的手连接线推动。
#[test]
fn an_unwired_window_changes_neither_the_engine_nor_the_display() {
    let harness = TransportHarness::new();
    let engine = Rc::clone(&harness.engine);
    let before = engine.borrow().transport();
    let journal_before = engine.borrow().transport_journal().len();

    let unwired_view =
        ViewState::from_project(&yeban_model::samples::filled_project()).expect("投影");
    let unwired_scene = DemoScene::from_view(&unwired_view);
    let unwired = host::build_main_window(&unwired_view, &unwired_scene)
        .expect("Tier-1 平台已装好 ⇒ 第二个窗口必须能建");
    assert!(!unwired.get_playing());
    unwired.invoke_toggle_play();
    unwired.invoke_stop();

    assert_eq!(
        engine.borrow().transport(),
        before,
        "未接线的窗口不得改变引擎读数"
    );
    assert_eq!(
        engine.borrow().transport_journal().len(),
        journal_before,
        "未接线的窗口不得产生任何动作记录"
    );
    assert!(
        !unwired.get_playing(),
        "显示态不再由界面自翻转 ⇒ 没接线时它必须保持停住"
    );
}

// ===========================================================================
// 走带 / 播放 / 录音（2026-10-08，负责人优先级：走带、播放、录音先完成）
// ===========================================================================
//
// 上一节（判据 ⑧ + ⑨）证明的是**回调那一格**接线正确（`invoke_toggle_play()` /
// `invoke_stop()`）。本节把注入点往**上游**推一格，证明**事件源**那一格也通 ——
// 用户按下键 / 点下按钮的那条链，而不是宿主直接调自己的回调。
//
// | 情形 | 本节的注入点 | 中间层 |
// | :--- | :--- | :--- |
// | `Space` | `ui/dispatch_key_press(KeyCode::Space)`（真 `WindowEvent`） | `app.slint` 的 `key-handler` → `key-action` → `host::wire_keys` → `input::resolve_logical` → `apply_action` → `MainWindow.toggle-play` → `host::wire_transport` |
// | `Shift+Space` | `MainWindow.invoke_key_action(text, shift, …)`（端口**表达不了** chord —— 见判据 ② 的文档） | 同上，去掉端口那两格 |
// | `stop` | `ui/dispatch_pointer_down/up("transport-stop-button")`（真指针事件） | Slint 命中测试 → `stop_area.clicked` → `root.stop()` → `MainWindow.stop` → `host::wire_transport` |
// | 录音 | `ui/dispatch_pointer_down/up("transport-record-button")`（真指针事件） | `rec_area` **没有** `clicked` ⇒ 什么都不发生（今天的事实） |
//
// **本节的判据不重复既有判据的断言**：既有判据已经钉住"回调 → 引擎"，
// 本节的牙咬在"事件源 → 回调"这一格（破坏它就是"按键/点击没反应"，而控件树照样"有"）。

/// 判据（**本票的端到端 ①**）：`Space` 经真键盘事件真的切换播放态；停止按钮经真指针事件真的回起点。
///
/// ## 量什么 / 怎么量 / 单位
///
/// | 量 | 怎么量 | 单位 |
/// | :--- | :--- | :--- |
/// | 引擎走带状态 | `EngineHost::transport().state` | 枚举（`Stopped` / `Playing` / `Recording`） |
/// | 引擎位置 | `EngineHost::transport().position_ticks` | tick（960 PPQ） |
/// | 引擎命令日志 | `EngineHost::transport_journal()`（每条 = 命令 + 命令后状态/位置 + 推了几个量子） | 条 |
/// | 界面显示态 | `MainWindow.get_playing()` | 布尔 |
/// | 时间码 | `MainWindow.get_timecode()` | `BBB.BB.BBB` 字符串 |
///
/// ## 算术（整数；期望值手算，**不引用被测函数**）
///
/// 起点 = `TransportHarness::new()` 之后：日志 **1** 条（夹具的显式 `Stop`）、位置 `0`、`Stopped`。
/// - 一次 `Space` ⇒ 日志 **+1** 条，命令 `[Play]`，状态 `Playing`，位置 `> 0`；
/// - 再一次 `Space` ⇒ 日志 **+1** 条，命令 `[Stop]`，位置**等于**上一次（`Stop` 保留位置）；
/// - 定位到 `3 × 960 = 2880` tick 后点停止 ⇒ 日志 **+2** 条，命令 `[Stop, SeekTicks(0)]`，
///   位置 `0`（`stop_and_rewind` 是**一个批量里的两条命令**）。
///
/// ## 怎么变红（注入实测见交付报告）
///
/// | 注入 | 位置 | 现象 |
/// | :--- | :--- | :--- |
/// | 把 `PlayPause` 从 `action_has_implementation` 的名单里去掉（改成不消费） | `src/host.rs` | ① 步状态仍是 `Stopped`、日志 **+0** |
/// | 把 `key-action` 的返回永远取反（`reject`） | `ui/app.slint` | ① 步状态仍是 `Stopped`（Slint 不再把键交给宿主） |
/// | 让 `stop_area.clicked` 调 `root.toggle-play()` | `ui/transport.slint` | ③ 步位置停在 `2880`（不回到 `0`） |
#[test]
fn space_from_the_real_key_event_source_toggles_playback_and_the_stop_button_rewinds() {
    let mut harness = TransportHarness::new();
    let context = harness.wire_keyboard();
    // 强句柄：下面要用 `&mut harness.port` 注入事件，不能同时借出 `port.ui()` 的 `&`。
    let ui = slint::ComponentHandle::clone_strong(harness.port.ui());

    // ---- 前置条件（**先证明注入源是活的**；否则"没反应"会被误读成"接线坏了"）----
    assert!(
        !context.borrow().is_composing(),
        "注入前必须是非合成态，否则 §7.2 会把 Space 吞掉"
    );
    assert_eq!(
        context.borrow().focus(),
        yeban_app::input::Focus::MainCanvas,
        "注入前焦点必须在主画布，否则 Space 归文本控件"
    );
    let start = harness.reading();
    assert_eq!(start.state, TransportState::Stopped, "起点必须是停住");
    assert_eq!(start.position_ticks, 0, "起点必须在 tick 0");
    assert!(!ui.get_playing(), "起点显示态必须是停住");
    let journal_at_start = harness.journal().len();

    // ---- ① `Space`（真 `WindowEvent::KeyPressed`）⇒ 引擎真的开始播 ----
    harness
        .port
        .dispatch_key_press(yeban_ui_test_port::KeyCode::Space)
        .expect("注入 Space 必须被端口接受（Interactive 权限）");
    let after_space = harness.reading();
    assert_eq!(
        after_space.state,
        TransportState::Playing,
        "`Space` 必须真的把引擎推进到 `Playing`（不是\"控件树里有这个按钮\"）"
    );
    assert!(after_space.position_ticks > 0, "播放必须真的推进 tick");
    assert!(ui.get_playing(), "显示态必须跟着引擎走");
    assert_eq!(
        ui.get_timecode(),
        harness.view.timecode_at(after_space.position_ticks),
        "时间码必须等于**投影**在这个 tick 上的读数"
    );
    let journal_after_space = harness.journal();
    assert_eq!(
        journal_after_space.len(),
        journal_at_start + 1,
        "一次 `Space` 恰好产生一条引擎命令"
    );
    let play_record = journal_after_space.last().expect("非空");
    assert_eq!(play_record.command, TransportCommand::Play);
    assert_eq!(play_record.state_after, TransportState::Playing);
    assert_eq!(play_record.position_ticks_after, after_space.position_ticks);
    assert_eq!(play_record.quanta_pumped, 1, "命令必须在量子边界被应用");
    observe(&format!(
        "[transport-e2e] ① 注入 `ui/dispatch_key_press(Space)` ⇒ 命令[Play] 状态={:?} \
         位置={} tick 推量子={} 显示态={} 时间码={}（日志 {} → {} 条）",
        play_record.state_after,
        play_record.position_ticks_after,
        play_record.quanta_pumped,
        ui.get_playing(),
        ui.get_timecode(),
        journal_at_start,
        journal_after_space.len(),
    ));

    // ---- ② 再按一次 `Space` ⇒ 停住，且**位置保留**（回到起点是停止按钮的语义）----
    harness
        .port
        .dispatch_key_press(yeban_ui_test_port::KeyCode::Space)
        .expect("再次注入 Space");
    let after_second_space = harness.reading();
    assert_eq!(
        after_second_space.state,
        TransportState::Stopped,
        "第二次 `Space` 必须停住"
    );
    assert_eq!(
        after_second_space.position_ticks, after_space.position_ticks,
        "`Space` 的停住**保留位置** —— 位置必须逐 tick 相等（差一位就是语义变了）"
    );
    assert!(!ui.get_playing(), "显示态必须跟着回到停住");
    let journal_after_second = harness.journal();
    assert_eq!(
        journal_after_second.len(),
        journal_at_start + 2,
        "第二次 `Space` 恰好再加一条命令"
    );
    assert_eq!(
        journal_after_second.last().expect("非空").command,
        TransportCommand::Stop
    );
    observe(&format!(
        "[transport-e2e] ② 再注入一次 `Space` ⇒ 命令[Stop] 状态={:?} 位置={} tick\
         （与 ① 后逐 tick 相等 ⇒ 位置保留）显示态={}",
        after_second_space.state,
        after_second_space.position_ticks,
        ui.get_playing()
    ));

    // ---- ③ 停止按钮（真指针事件）⇒ 停住 **并回到 tick 0** ----
    // 先把位置挪到非零，否则"回到起点"与"位置保留"读数相同 ⇒ 判据没有牙。
    let moved = harness.seek_and_inject(3 * 960);
    assert_eq!(moved.position_ticks, 3 * 960, "定位必须真的改位置");
    assert_eq!(ui.get_timecode(), harness.view.timecode_at(3 * 960));
    let journal_before_stop = harness.journal().len();
    harness
        .port
        .dispatch_pointer_down(
            "transport-stop-button",
            16.0,
            16.0,
            yeban_ui_test_port::PointerButton::Left,
        )
        .expect("真实指针按下停止按钮");
    harness
        .port
        .dispatch_pointer_up(yeban_ui_test_port::PointerButton::Left)
        .expect("真实指针释放停止按钮");
    let after_stop = harness.reading();
    assert_eq!(after_stop.state, TransportState::Stopped);
    assert_eq!(
        after_stop.position_ticks, 0,
        "停止按钮必须回到 tick 0（`accessible-label` 的\"回到起始点\"）"
    );
    assert_eq!(
        ui.get_timecode(),
        "001.01.000",
        "回到起点后时间码必须是第 1 小节第 1 拍"
    );
    let journal_after_stop = harness.journal();
    assert_eq!(
        journal_after_stop.len(),
        journal_before_stop + 2,
        "`stop_and_rewind` 是**一个批量里的两条命令**"
    );
    assert_eq!(
        journal_after_stop[journal_before_stop].command,
        TransportCommand::Stop
    );
    assert_eq!(
        journal_after_stop[journal_before_stop + 1].command,
        TransportCommand::SeekTicks(0)
    );
    assert_eq!(
        journal_after_stop[journal_before_stop].quanta_pumped, 1,
        "两条命令在**同一个**量子边界一起生效（一次批量推一个量子）"
    );
    observe(&format!(
        "[transport-e2e] ③ 真指针点击停止按钮（注入前位置=2880 tick）⇒ 命令[Stop, SeekTicks(0)] \
         状态={:?} 位置={} tick 时间码={}（日志 {} → {} 条）",
        after_stop.state,
        after_stop.position_ticks,
        ui.get_timecode(),
        journal_before_stop,
        journal_after_stop.len(),
    ));
}

/// 判据（**本票的端到端 ②：如实登记"今天无法区分"**）：`Shift+Space` 与 `Space` 走**同一条**走带路径。
///
/// `input.rs` 把两键解析成**两个不同**的 `Action`（`Space` → `PlayPause`、
/// `Shift+Space` → `ResumeFromCursor`），`cli.rs` 的快捷表也把它们当成两条规范快捷键
/// （两条的 `implemented` 都是 `true`，`--print-shortcuts` 都不打 `(未实现)`）。
/// 但宿主的落点是**一个** match 臂 —— `src/host.rs` 的
/// `Action::PlayPause | Action::ResumeFromCursor => ui.invoke_toggle_play()` ——
/// 因此两键的引擎读数**逐位相同**：今天**没有任何可观测差异**。
///
/// ⚠ 本判据**不假装**它们有区别。它把"今天不可区分"钉成机械事实：
/// 将来谁真的做出"从光标续播"的独立语义（例如 `PlayPause` 也回到起点、
/// 或 `ResumeFromCursor` 带上光标定位），本判据会红，作者必须一并更新本判据
/// 与 `cli.rs` 的表述。**这不是永久禁令，是今天的状态。**
///
/// ## 注入面（为什么 `Shift+Space` 不走端口）
///
/// `[UI-TEST-002]` 的 `ui/dispatch_key_press(keyCode)` 只携带 `text`：
/// `slint::platform::WindowEvent::KeyPressed { text }` **没有**修饰位字段
/// （`crates/yeban-ui-test-port/src/render.rs` 的 `Tier1Window::key_press` 逐字只发 `text`），
/// 端口的 `KeyCode` 也没有 `ShiftSpace`。因此 chord 只能注入到 `.slint` 回调那一格
/// `MainWindow.invoke_key_action(text, shift, ctrl, alt, meta)` —— 它就是
/// `ui/app.slint` 的 `key-handler` 在 `key-pressed` 里调的那**一个**回调
/// （`event.modifiers.shift` 由 Slint 逐位传下来）。仓库既有的 `Cmd+Z` 判据用的是同一格
/// （`crates/yeban-app/tests/live_ui_mcp.rs` 的注释逐字写着"`ui/dispatch_key_press`
/// 表达不了修饰键 chord"）。
///
/// ## 怎么变红
///
/// | 注入 | 位置 | 现象 |
/// | :--- | :--- | :--- |
/// | 把 `ResumeFromCursor` 从那个 match 臂里拿掉 | `src/host.rs` | ② 步命令序列变 `[]`（不消费）⇒ 与 ① 不等 |
/// | 让 `ResumeFromCursor` 改走 `stop_and_rewind` 再播 | `src/host.rs` | ② 步位置仍是起点 `0` 或命令序列不同 ⇒ 与 ① 不等 |
#[test]
fn shift_space_and_space_are_the_same_transport_path_today() {
    /// 把夹具复位到"停住 @ tick 0"。两次注入的起点必须逐位相同，否则比较不成立。
    /// 返回复位后的日志长度（增量测量的基线）。
    fn rewind(harness: &mut TransportHarness) -> usize {
        harness.engine.borrow_mut().stop_and_rewind();
        let reading = harness.reading();
        host::apply_transport(harness.port.ui(), reading);
        assert_eq!(reading.state, TransportState::Stopped, "复位后必须停住");
        assert_eq!(reading.position_ticks, 0, "复位后必须在 tick 0");
        harness.journal().len()
    }

    let mut harness = TransportHarness::new();
    let context = harness.wire_keyboard();
    let ui = slint::ComponentHandle::clone_strong(harness.port.ui());

    // ---- ① `Space`：**端口**注入一个真 `WindowEvent` ----
    assert_eq!(
        context.borrow().focus(),
        yeban_app::input::Focus::MainCanvas,
        "注入前焦点必须在主画布"
    );
    let base_key = rewind(&mut harness);
    harness
        .port
        .dispatch_key_press(yeban_ui_test_port::KeyCode::Space)
        .expect("注入 Space");
    let after_key = harness.reading();
    let key_commands: Vec<TransportCommand> = harness.journal()[base_key..]
        .iter()
        .map(|record| record.command)
        .collect();

    // ---- ② `Shift+Space`：端口表达不了 chord ⇒ 注入到 `.slint` 回调那一格 ----
    let base_shift = rewind(&mut harness);
    let consumed = ui.invoke_key_action(" ".into(), true, false, false, false);
    let after_shift = harness.reading();
    let shift_commands: Vec<TransportCommand> = harness.journal()[base_shift..]
        .iter()
        .map(|record| record.command)
        .collect();

    // ---- ③ 读数的**逐位相等**（这就是"今天无法区分"的字面证据）----
    assert!(
        consumed,
        "`Shift+Space` 今天必须被消费 —— `cli.rs` 的快捷表把它标成 `implemented: true`"
    );
    assert!(
        ui.invoke_key_action(" ".into(), false, false, false, false),
        "对照：无修饰的 `Space` 也必须被消费"
    );
    assert_eq!(key_commands, vec![TransportCommand::Play], "① 的命令序列");
    assert_eq!(
        shift_commands, key_commands,
        "② 的命令序列必须与 ① 逐条相同 —— 不同就说明两键的落点已经分叉，\
         本判据与 `cli.rs` 的表述都要一起更新"
    );
    assert_eq!(
        after_shift.state, after_key.state,
        "两键后的引擎状态必须相同"
    );
    assert_eq!(
        after_shift.position_ticks, after_key.position_ticks,
        "两键后的位置必须逐 tick 相同（今天没有任何可观测差异）"
    );
    observe(&format!(
        "[transport-e2e] ④ `Space`（端口真事件）⇒ 状态={:?} 位置={} tick 命令={:?}；\
         `Shift+Space`（`.slint` 回调那一格）⇒ 状态={:?} 位置={} tick 命令={:?} \
         ⇒ **两键今天不可区分**（`host.rs` 的 `Action::PlayPause | Action::ResumeFromCursor` \
         是同一个 match 臂）",
        after_key.state,
        after_key.position_ticks,
        key_commands,
        after_shift.state,
        after_shift.position_ticks,
        shift_commands,
    ));
}

/// 判据（⭐ 任务单 `#212` 的**无头复现**）：`Space` 在**生产接线**（`wire_keys` 拿到引擎）
/// 下不得重入 `EngineHost` 的借用。
///
/// ## 复现的是什么
///
/// `main.rs` 的 `wire_keys` 传的是 `Some(engine_handle)` ＋ `Some(project)`（与
/// `wire_transport` 拿的是**同一个** `Rc<RefCell<EngineHost>>`）。按键那条链是：
///
/// ```text
/// key-action → wire_keys 闭包 → host.try_borrow_mut()   （⭐ 借在这里就活着）
///            → apply_action → Action::PlayPause => ui.invoke_toggle_play()
///            → wire_transport 的 on_toggle_play → toggle.borrow_mut()  ← 同一个 RefCell
/// ```
///
/// 于是 `Space` 在真机上直接 panic（`RefCell already borrowed`），而它落在 `key_down`
/// 这个 FFI 边界里 ⇒ **非 unwinding panic ⇒ 进程 abort**。
///
/// ## 为什么既有判据没抓住
///
/// [`TransportHarness::wire_keyboard`] 传的是 `wire_keys(…, None, None, None)`（判据装配
/// 的既有形态：没有撤销会话、没有引擎上下文）⇒ 那条臂根本不借引擎 ⇒ 同一键在判据里绿。
/// 本判据把生产的那两个 `Some(..)` **照抄**进来，因此它与真机是**同一形状**。
#[test]
fn space_with_the_production_engine_wiring_does_not_re_enter_the_engine_borrow() {
    let mut harness = TransportHarness::new();
    let context = Rc::new(RefCell::new(yeban_app::input::InputContext::new()));
    host::wire_input(harness.port.ui(), Rc::clone(&context));
    // ⭐ 生产形状：引擎上下文 ＋ 当前工程都**在场**（`main.rs` 的 `wire_keys` 调用同款）。
    host::wire_keys(
        harness.port.ui(),
        Rc::clone(&context),
        None,
        Some(Rc::clone(&harness.engine)),
        Some(Rc::new(yeban_model::samples::filled_project())),
    );
    assert_eq!(
        context.borrow().focus(),
        yeban_app::input::Focus::MainCanvas,
        "注入前焦点必须在主画布"
    );
    assert!(
        !context.borrow().is_composing(),
        "注入前不得处于输入法合成态（否则 Space 会被吞掉）"
    );
    let before = harness.reading();
    assert_eq!(
        before.state,
        TransportState::Stopped,
        "夹具起点必须是停住（否则 Space 的语义是暂停）"
    );
    let base = harness.journal().len();

    // ⭐ 这一行在修好之前会 panic（`RefCell already borrowed`，位置是 `host.rs` 的
    // `on_toggle_play`）—— panic 在 Slint 回调里，产品二进制里它就是**进程 abort**。
    harness
        .port
        .dispatch_key_press(yeban_ui_test_port::KeyCode::Space)
        .expect("注入 Space");

    let after = harness.reading();
    assert_eq!(
        after.state,
        TransportState::Playing,
        "`Space` 必须真的起播（引擎读数）"
    );
    assert_eq!(
        harness.journal()[base..]
            .iter()
            .map(|record| record.command)
            .collect::<Vec<_>>(),
        vec![TransportCommand::Play],
        "一次 `Space` = 一条 `Play`"
    );
    let ui = harness.port.ui();
    assert_eq!(
        ui.get_playing(),
        after.state.is_running(),
        "显示态 `playing` 必须跟着引擎读数走"
    );
    observe(&format!(
        "[transport-e2e] ⑤ 生产接线（`wire_keys` 拿到引擎）下 `Space` ⇒ 状态={:?} \
         位置={} tick 命令={:?} 显示 playing={}（⭐ 不 panic；修好之前这一行到不了）",
        after.state,
        after.position_ticks,
        vec!["Play"],
        ui.get_playing(),
    ));
}

/// 判据（⭐ `#212` 判据 ⑤：走带**仍正确**）：生产接线（`wire_keys` 拿到引擎）下，
/// `Space` 播放 → `Space` 暂停（位置保留）→ 停止按钮回起点 → 播放按钮再起播。
///
/// 这条与 [`space_with_the_production_engine_wiring_does_not_re_enter_the_engine_borrow`]
/// 的分工：那条证明"不崩"，这条证明"**修完之后语义一位没变**"（状态机 ＋ `apply_transport`
/// 写下的时间码都跟着引擎读数走）。
#[test]
fn space_pause_and_stop_stay_correct_on_the_production_wiring() {
    let mut harness = TransportHarness::new();
    let context = Rc::new(RefCell::new(yeban_app::input::InputContext::new()));
    host::wire_input(harness.port.ui(), Rc::clone(&context));
    host::wire_keys(
        harness.port.ui(),
        Rc::clone(&context),
        None,
        Some(Rc::clone(&harness.engine)),
        Some(Rc::new(yeban_model::samples::filled_project())),
    );
    let ui = slint::ComponentHandle::clone_strong(harness.port.ui());
    assert!(!ui.get_playing(), "初始显示态是停住");

    // ① `Space` ⇒ 播放（真端口事件：`WindowEvent::KeyPressed` → `.slint` 的 `key-handler`）。
    harness
        .port
        .dispatch_key_press(yeban_ui_test_port::KeyCode::Space)
        .expect("注入 Space");
    let played = harness.reading();
    assert_eq!(played.state, TransportState::Playing, "① Space 必须起播");
    assert!(played.position_ticks > 0, "播放必须真的推进 tick");
    assert!(ui.get_playing(), "显示态必须跟着引擎");
    assert_eq!(
        ui.get_timecode(),
        harness.view.timecode_at(played.position_ticks),
        "时间码必须等于投影在这个 tick 上的读数（`apply_transport` 的唯一写点）"
    );

    // ② 再按 `Space` ⇒ 暂停（引擎的 `Stop`，位置保留）。
    harness
        .port
        .dispatch_key_press(yeban_ui_test_port::KeyCode::Space)
        .expect("注入 Space");
    let paused = harness.reading();
    assert_eq!(paused.state, TransportState::Stopped, "② Space 必须暂停");
    assert_eq!(
        paused.position_ticks, played.position_ticks,
        "暂停保留位置（\"回到起点\"是停止按钮的语义）"
    );
    assert!(!ui.get_playing(), "显示态必须跟着回到停住");

    // ③ 停止按钮 ⇒ 停住 ＋ 回到 tick 0（`on_stop` 的 `stop_and_rewind`）。
    harness.seek_and_inject(2880);
    assert_eq!(
        ui.get_timecode(),
        harness.view.timecode_at(2880),
        "定位后时间码跟着走"
    );
    ui.invoke_stop();
    let rewound = harness.reading();
    assert_eq!(rewound.state, TransportState::Stopped);
    assert_eq!(rewound.position_ticks, 0, "停止按钮必须回到 tick 0");
    assert_eq!(
        ui.get_timecode(),
        "001.01.000",
        "回到起点后是第 1 小节第 1 拍"
    );
    assert!(!ui.get_playing());

    // ④ 播放按钮 ⇒ 再起播（回调链在 ③ 之后仍然通）。
    ui.invoke_toggle_play();
    let replayed = harness.reading();
    assert_eq!(
        replayed.state,
        TransportState::Playing,
        "④ 播放按钮必须起播"
    );
    assert!(ui.get_playing());
    observe(&format!(
        "[transport-e2e] ⑨ 生产接线下的走带全链：① Space→{:?}@{} tick ② Space→{:?}@{}\
         （暂停保留位置）③ 停止按钮→{:?}@{}（回起点，时间码 001.01.000）④ 播放按钮→{:?}@{}",
        played.state,
        played.position_ticks,
        paused.state,
        paused.position_ticks,
        rewound.state,
        rewound.position_ticks,
        replayed.state,
        replayed.position_ticks,
    ));
}

/// 判据（⭐ `#212` 的**同类扫描 ①**：键盘全键）：`wire_keys` 里**每一个**快捷键在
/// **生产接线**（引擎上下文 ＋ 工程都在场）下各过一遍。
///
/// ## 两条断言为什么正好覆盖 `#212`
///
/// 崩法的共同形状是"按键回调**持借**期间调界面" ⇒ 被调的界面回调 `borrow_mut()` 重入
/// ⇒ `RefCell already borrowed`，而它落在 `key_down`（FFI 边界）⇒ **进程 abort**。
///
/// 1. **不 panic**：每一键都真的调一次 `key-action`（`.slint` 的 `key-handler` 调的就是
///    这**一个**回调），并且顺序覆盖**全部**策略表分支（`input.rs` 的
///    `resolve_logical` 第 1–6 步）；
/// 2. **借还回来了**：每次返回之后 `try_borrow_mut()` 必须成功 —— 这就是"没有长借留在
///    回调外"的机械读数。任何新增的"持借跨界面调用"都在这里红，而不是在真机上崩。
///
/// ⚠ 带 `Cmd/Ctrl` 的组合经 `invoke_key_action` 注入（端口表达不了 chord，仓库既有的
/// `Cmd+Z` 判据用的是同一格）；裸键那条链由 [`TransportHarness::wire_keyboard`] 的
/// 端口真事件路覆盖（见 `shift_space_and_space_are_the_same_transport_path_today`）。
#[test]
fn every_wired_key_returns_without_panicking_and_without_keeping_the_engine_borrow() {
    let harness = TransportHarness::new();
    let context = Rc::new(RefCell::new(yeban_app::input::InputContext::new()));
    host::wire_input(harness.port.ui(), Rc::clone(&context));
    host::wire_keys(
        harness.port.ui(),
        Rc::clone(&context),
        None,
        Some(Rc::clone(&harness.engine)),
        Some(Rc::new(yeban_model::samples::filled_project())),
    );
    let ui = slint::ComponentHandle::clone_strong(harness.port.ui());

    // (`text`, shift, ctrl, alt, meta, 标签) —— 顺序就是 `input.rs` 策略表的顺序：
    // 直达视图 → Cmd/Ctrl 组合 → Alt 组合 → Shift 组合 → 无修饰单键 → 未绑定键。
    let keys: &[(&str, bool, bool, bool, bool, &str)] = &[
        ("\u{f708}", false, false, false, false, "F5"),
        ("\u{f709}", false, false, false, false, "F6"),
        ("z", false, true, false, false, "Cmd+Z"),
        ("z", true, true, false, false, "Cmd+Shift+Z"),
        ("h", true, true, false, false, "Cmd+Shift+H"),
        ("d", false, true, false, false, "Cmd+D"),
        ("b", false, true, true, false, "Cmd+Alt+B"),
        ("m", false, true, true, false, "Cmd+Alt+M"),
        ("n", false, true, true, false, "Cmd+Alt+N（节拍器）"),
        ("d", false, true, true, false, "Cmd+Alt+D（预备拍）"),
        ("1", false, false, true, false, "Alt+1"),
        ("2", false, false, true, false, "Alt+2"),
        ("\u{f702}", false, false, true, false, "Alt+Left"),
        ("\u{f703}", false, false, true, false, "Alt+Right"),
        (" ", true, false, false, false, "Shift+Space"),
        ("\n", true, false, false, false, "Shift+Enter"),
        ("z", true, false, false, false, "Shift+Z"),
        ("\u{f700}", true, false, false, false, "Shift+Up"),
        ("\u{f701}", true, false, false, false, "Shift+Down"),
        ("\t", false, false, false, false, "Tab"),
        (" ", false, false, false, false, "Space"),
        // ⭐ 任务单 `#227`：裸 `Return` / `Enter` = 回到起点（界面回调族，借外执行）。
        ("\n", false, false, false, false, "Return（回起点）"),
        ("\u{1b}", false, false, false, false, "Escape"),
        ("\u{7f}", false, false, false, false, "Delete"),
        ("\u{8}", false, false, false, false, "Backspace"),
        ("\u{f702}", false, false, false, false, "Left"),
        ("\u{f703}", false, false, false, false, "Right"),
        ("\u{f700}", false, false, false, false, "Up"),
        ("\u{f701}", false, false, false, false, "Down"),
        ("b", false, false, false, false, "B"),
        ("z", false, false, false, false, "Z"),
        ("[", false, false, false, false, "["),
        ("]", false, false, false, false, "]"),
        ("1", false, false, false, false, "1"),
        ("2", false, false, false, false, "2"),
        ("3", false, false, false, false, "3"),
        ("4", false, false, false, false, "4"),
        ("5", false, false, false, false, "5"),
        ("q", false, false, false, false, "Q（未绑定）"),
        ("hello", false, false, false, false, "多字符文本"),
    ];

    let mut consumed = Vec::new();
    for (text, shift, ctrl, alt, meta, label) in keys {
        let accepted = ui.invoke_key_action((*text).into(), *shift, *ctrl, *alt, *meta);
        if accepted {
            consumed.push(*label);
        }
        assert!(
            harness.engine.try_borrow_mut().is_ok(),
            "`{label}` 返回之后引擎**仍被借用** ⇒ 有一条长借跨过了界面调用；\
             这正是 `#212` 的崩法（下一个 `borrow_mut()` 会 panic，在 `key_down` 里 = abort）"
        );
    }
    assert!(
        consumed.len() >= 8,
        "策略表里被消费的键不可能这么少（只数到 {} 条：{consumed:?}）—— \
         这条判据本身可能没接上生产接线",
        consumed.len()
    );
    observe(&format!(
        "[transport-e2e] ⑥ 键盘全键 × {} 条 ⇒ 全部返回，且每次返回后引擎借用可再取 \
         （0 次 `RefCell already borrowed`）；被消费的键={consumed:?}",
        keys.len()
    ));
}

/// 判据（⭐ `#212` 的**同类扫描 ②**：`try_borrow` 兜底）：**故意持借**时调走带回调 ⇒
/// **降级**（打一行 ＋ 一位不动），⛔ 不是 panic、⛔ 更不是 abort。
///
/// ## 这条判据造的是**真机那个形状**
///
/// `#212` 的崩溃是"进入 `on_toggle_play` 时 `engine` 已经被借走且没释放"。修根因
/// （`wire_keys` 只给"要动引擎"的动作上借）之后这条路径不该再出现；但按键回调跑在
/// `winit` 的 `key_down`（FFI 边界，**不能 unwind**）⇒ 一旦回归，代价是**整个进程**。
/// 因此那道边界上还有第二道防线：`try_borrow_mut()` 失败 ⇒ 打一行 ＋ 直接返回。
///
/// 本判据**手工持借**（不经过 `wire_keys`）来钉住那道防线：它必须给出**读数**
/// （引擎一 tick 没动、一条命令没收到、界面一位没变），而不是把测试进程打死。
#[test]
fn toggle_play_and_stop_degrade_instead_of_aborting_when_the_engine_is_already_borrowed() {
    let harness = TransportHarness::new();
    let ui = slint::ComponentHandle::clone_strong(harness.port.ui());
    assert!(!ui.get_playing(), "初始显示态是停住");
    let base = harness.journal().len();

    {
        // ⭐ **故意持借** = `#212` 的重入形状（持借期间调界面回调）。
        let engine = Rc::clone(&harness.engine);
        let _guard = engine.borrow_mut();
        // ⛔ 修好之前这两行各自会 `RefCell already borrowed`（`host.rs` 的
        // `on_toggle_play` / `on_stop`），在真机上就是**进程 abort**。
        ui.invoke_toggle_play();
        ui.invoke_stop();
    }

    let after = harness.reading();
    assert_eq!(
        after.state,
        TransportState::Stopped,
        "被降级的两次回调**不得**改动引擎状态"
    );
    assert_eq!(
        after.position_ticks, 0,
        "被降级的两次回调**不得**推动播放头"
    );
    assert!(
        !ui.get_playing(),
        "被降级的两次回调**不得**改动显示态（⛔ 不假装切换过）"
    );
    assert_eq!(
        harness.journal().len(),
        base,
        "被降级的两次回调**不得**向引擎发任何命令"
    );
    assert!(
        harness.engine.try_borrow_mut().is_ok(),
        "回调返回之后借用必须已经放掉（降级路径也不许把守卫带走）"
    );
    observe(&format!(
        "[transport-e2e] ⑦ 故意持借 ⇒ `invoke_toggle_play()` ＋ `invoke_stop()` 都降级：\
         状态={:?} 位置={} tick 命令增量={} 显示 playing={}，且没有 panic / abort",
        after.state,
        after.position_ticks,
        harness.journal().len() - base,
        ui.get_playing(),
    ));
}

// ===========================================================================
// 任务单 `#227`：**Logic 的播放头语义**（停止 / 连按回起点 / `Return`）
// ===========================================================================
//
// 负责人报的缺陷：「现在 GUI 播放我按停止后没法回到从头开始播放」；裁决「用 Logic 方案」。
//
// 这一节的判据一律走**生产装配**（第 321 条）：`wire_transport`（引擎句柄）＋
// `wire_keys(…, Some(engine), Some(project))` ＋ `.slint` 的 `key-action` 回调 ——
// 与 `main.rs` 的 `run_gui` 是同一对函数、同一份 `Rc<RefCell<EngineHost>>`。
// 因此"按键 → 状态机 → 引擎命令 → `apply_transport` → 时间码"整条链都在判据里，
// ⛔ 不是把某个内部函数单独拎出来测。
//
// ## 语义表（任务单 §1 逐行）
//
// | # | 当前状态 | 按键 | 期望 |
// | :--- | :--- | :--- | :--- |
// | 1 | 停止（在起点） | `Space` | 开始播放 |
// | 2 | 播放中 | `Space` | 停止、**停在中途**（位置保留） |
// | 3 | 停止（在中途，无待配对停住） | `Space`（第一次） | 从停止处继续 |
// | 4 | 停止（在中途，上一次 `Space` 停住在 1.5 s 内） | `Space`（连按第二次） | 回到起点 |
// | 5 | 任意停住 | `Return` / `Enter` | 回到起点 |
// | 6 | 播放中 | `Return` / `Enter` | 回到起点 ＋ **继续播放** |

/// `#227` 的夹具：生产形状的键盘接线（与 `main.rs` 的 `wire_input` + `wire_keys` 同款）。
///
/// `wire_keys` 拿到 **`Some(engine)` ＋ `Some(project)`** —— 判据 5（第 321 条）要求的
/// 生产装配；若引擎上下文缺席，`wire_keys` 会走"借外"分支，那就测不到真实的借用形状。
fn logic_head_harness() -> (TransportHarness, MainWindow) {
    let harness = TransportHarness::new();
    let context = Rc::new(RefCell::new(yeban_app::input::InputContext::new()));
    host::wire_input(harness.port.ui(), Rc::clone(&context));
    host::wire_keys(
        harness.port.ui(),
        context,
        None,
        Some(Rc::clone(&harness.engine)),
        Some(Rc::new(yeban_model::samples::filled_project())),
    );
    let ui = slint::ComponentHandle::clone_strong(harness.port.ui());
    (harness, ui)
}

/// 按一次**裸键**：`text` 是 Slint `KeyEvent.text` 的词表（`" "` / `"\n"` / `"\t"`…）。
fn press(ui: &MainWindow, text: &str) -> bool {
    ui.invoke_key_action(text.into(), false, false, false, false)
}

/// 自 `base`（调用前的日志长度）起，引擎真的收到的命令序列。
fn commands_since(harness: &TransportHarness, base: usize) -> Vec<TransportCommand> {
    harness.journal()[base..]
        .iter()
        .map(|record| record.command)
        .collect()
}

/// 逐状态对账：**显示的时间码 / `playing`** 必须等于**引擎位置 / 状态**的投影。
///
/// 这是根因 ③（"显示回 0、引擎没回"）的机械否定：`apply_transport` 是唯一写者，
/// 它只读 [`TransportReading`] ⇒ 两个方向都必须一致，没有第二份状态。
fn assert_display_follows_engine(harness: &TransportHarness, ui: &MainWindow, label: &str) {
    let reading = harness.reading();
    assert_eq!(
        ui.get_timecode(),
        harness.view.timecode_at(reading.position_ticks),
        "{label}: 显示时间码必须等于引擎位置（{} tick）在投影上的读数 —— 显示与状态分叉了",
        reading.position_ticks,
    );
    assert_eq!(
        ui.get_playing(),
        reading.state.is_running(),
        "{label}: 显示 `playing` 必须等于引擎状态 {:?} —— 显示与状态分叉了",
        reading.state,
    );
}

/// 根因 ①②③ 的**逐状态读数**（任务单 §2 第 1 条 ＋ §1 的三条可能根因）。
///
/// 判据形状：每换一次状态，就把**显示时间码**与**引擎 `position_ticks`** 并排读出来，
/// 断言两者一致；最后把全部状态里的 `(显示时间码, 引擎位置)` 收集起来，断言
/// **不存在**"显示 `001.01.000` 而引擎位置 ≠ 0"的那一格（根因 ③ 的反例）。
#[test]
fn head_root_causes_display_and_engine_positions_are_measured_in_every_state() {
    let (mut harness, ui) = logic_head_harness();
    let mut readings: Vec<(String, u64, String, bool)> = Vec::new();

    fn record(
        harness: &TransportHarness,
        ui: &MainWindow,
        label: &str,
        out: &mut Vec<(String, u64, String, bool)>,
    ) {
        assert_display_follows_engine(harness, ui, label);
        let reading = harness.reading();
        let timecode = ui.get_timecode().to_string();
        out.push((
            label.to_string(),
            reading.position_ticks,
            timecode.clone(),
            ui.get_playing(),
        ));
        observe(&format!(
            "[logic-head] {label}: 显示时间码={timecode} 引擎位置={} tick 显示 playing={} \
             引擎状态={:?}",
            reading.position_ticks,
            ui.get_playing(),
            reading.state,
        ));
    }

    // ① 初始：停止、在起点。
    record(
        &harness,
        &ui,
        "根因读数 1/7 初始（停止，在起点）",
        &mut readings,
    );
    assert_eq!(harness.reading().position_ticks, 0);
    assert_eq!(ui.get_timecode(), "001.01.000");

    // ② `Space` ⇒ 播放中（引擎真的推进了 tick）。
    let base = harness.journal().len();
    assert!(press(&ui, " "), "`Space` 必须被消费");
    record(&harness, &ui, "根因读数 2/7 Space ⇒ 播放中", &mut readings);
    let playing_at = harness.reading().position_ticks;
    assert!(playing_at > 0, "播放必须真的推进 tick");
    assert_eq!(
        commands_since(&harness, base),
        vec![TransportCommand::Play],
        "根因读数：起播只发 `Play`"
    );

    // ③ 再按 `Space` ⇒ 停住。**根因 ② 的证据**：位置保留（`stop()` 是暂停语义）。
    let base = harness.journal().len();
    assert!(press(&ui, " "), "`Space` 必须被消费");
    record(
        &harness,
        &ui,
        "根因读数 3/7 Space ⇒ 停住（位置保留）",
        &mut readings,
    );
    let stopped = harness.reading();
    assert_eq!(stopped.state, TransportState::Stopped);
    assert_eq!(
        stopped.position_ticks, playing_at,
        "根因 ② 的证据：引擎 `stop()` **保留**位置（它是暂停语义，不是停止）"
    );
    assert_ne!(
        ui.get_timecode(),
        "001.01.000",
        "根因 ③ 的反证：停在中途时显示**不是** 0（它等于引擎位置）"
    );
    assert_eq!(
        commands_since(&harness, base),
        vec![TransportCommand::Stop],
        "根因读数：停住只发 `Stop`（不带 `SeekTicks`）"
    );

    // ④ 停止按钮。**根因 ① 的证据**：它发 `Stop` + `SeekTicks(0)`，⛔ 不是只调 `stop()`。
    let base = harness.journal().len();
    ui.invoke_stop();
    record(
        &harness,
        &ui,
        "根因读数 4/7 停止按钮 ⇒ 停止 ＋ 回起点",
        &mut readings,
    );
    assert_eq!(harness.reading().position_ticks, 0);
    assert_eq!(ui.get_timecode(), "001.01.000");
    assert_eq!(
        commands_since(&harness, base),
        vec![TransportCommand::Stop, TransportCommand::SeekTicks(0)],
        "根因 ① 的反证：停止按钮的回调是 `stop_and_rewind()`（两条命令），⛔ 不是只调 `stop()`"
    );

    // ⑤ 外部定位（拖播放头 / 打开工程那种"停在中途"）⇒ 显示跟着引擎。
    harness.seek_and_inject(2880);
    record(
        &harness,
        &ui,
        "根因读数 5/7 seek(2880) ⇒ 停在中途",
        &mut readings,
    );
    assert_eq!(harness.reading().position_ticks, 2880);
    assert_ne!(ui.get_timecode(), "001.01.000");

    // ⑥ `Return` ⇒ 回到起点（新键映射）。
    assert!(press(&ui, "\n"), "`Return` 必须被消费");
    record(
        &harness,
        &ui,
        "根因读数 6/7 Return ⇒ 回到起点",
        &mut readings,
    );
    assert_eq!(harness.reading().position_ticks, 0);
    assert_eq!(ui.get_timecode(), "001.01.000");

    // ⑦ `Space` 起播 ⇒ `Return`（播放中）⇒ 回起点并继续。
    assert!(press(&ui, " "), "`Space` 必须被消费");
    assert!(press(&ui, "\n"), "`Return` 必须被消费");
    record(
        &harness,
        &ui,
        "根因读数 7/7 播放中 Return ⇒ 回起点 ＋ 继续播放",
        &mut readings,
    );

    // ③ 的**反例扫描**：所有状态里都不存在"显示 0 而引擎非 0"。
    for (label, position, timecode, _) in &readings {
        assert!(
            !(timecode.as_str() == "001.01.000" && *position != 0),
            "根因 ③ 成立（应当没有这一格）：{label} 显示 `001.01.000` 而引擎位置 = {position} tick \
             —— 显示与状态不一致"
        );
    }
    observe(&format!(
        "[logic-head] ③ 反例扫描：{} 个状态全部满足「显示时间码 == timecode_at(引擎 position_ticks)」；\
         『显示 001.01.000 而引擎非 0』出现 0 次",
        readings.len()
    ));
}

/// 任务单 §1 的**状态机全表**（六行逐行一条读数：状态 / 位置 / 命令序列）。
///
/// 行 1 / 2 / 4 必须在**同一条时间线**上（"连按"是两次真实的 `Space` 相隔几百毫秒，
/// 而不是两个互不相干的夹具）；行 3 用一条**全新**的装配（没有待配对的停住时刻 = "第一次"）；
/// 行 5 / 6 回到同一条时间线。
#[test]
fn head_state_machine_full_table_is_measured_on_the_production_wiring() {
    let (mut harness, ui) = logic_head_harness();

    // ---- 行 1：停止（在起点）＋ `Space` ⇒ 开始播放 ----
    let base = harness.journal().len();
    assert!(press(&ui, " "), "`Space` 必须被消费");
    let row1 = harness.reading();
    assert_eq!(row1.state, TransportState::Playing, "行 1：必须开始播放");
    assert!(row1.position_ticks > 0, "行 1：必须真的推进 tick");
    assert_eq!(commands_since(&harness, base), vec![TransportCommand::Play]);
    observe(&format!(
        "[logic-head] 行1 状态=停止@0 ＋ Space ⇒ 状态={:?} 位置={} tick 命令={:?} 显示 playing={} \
         时间码={}",
        row1.state,
        row1.position_ticks,
        commands_since(&harness, base),
        ui.get_playing(),
        ui.get_timecode(),
    ));

    // ---- 行 2：播放中 ＋ `Space` ⇒ 停止、**停在中途**（位置保留）----
    let playing_at = row1.position_ticks;
    let base = harness.journal().len();
    assert!(press(&ui, " "), "`Space` 必须被消费");
    let row2 = harness.reading();
    assert_eq!(row2.state, TransportState::Stopped, "行 2：必须停住");
    assert_eq!(
        row2.position_ticks, playing_at,
        "行 2：位置必须保留（停在中途）"
    );
    assert_eq!(commands_since(&harness, base), vec![TransportCommand::Stop]);
    observe(&format!(
        "[logic-head] 行2 状态=播放中@{} ＋ Space ⇒ 状态={:?} 位置={} tick（== 停住前）命令={:?} \
         显示 playing={} 时间码={}",
        playing_at,
        row2.state,
        row2.position_ticks,
        commands_since(&harness, base),
        ui.get_playing(),
        ui.get_timecode(),
    ));

    // ---- 行 4：停止（在中途）＋ `Space`（连按第二次，**无时间窗等待**）⇒ 回到起点 ----
    // ⚠ 顺序：行 4 必须先于"行 3 的继续"—— 行 2 的 `Stop` 刚刚写下了停住时刻，
    // 紧接着的这一下就是任务单说的"连按第二次"（1.5 s 窗口内，见 [`HEAD_DOUBLE_TAP_WINDOW`]）。
    let base = harness.journal().len();
    assert!(press(&ui, " "), "`Space` 必须被消费");
    let row4 = harness.reading();
    assert_eq!(row4.state, TransportState::Stopped, "行 4：仍是停住");
    assert_eq!(row4.position_ticks, 0, "行 4：必须回到起点");
    assert_eq!(
        commands_since(&harness, base),
        vec![TransportCommand::SeekTicks(0)],
        "行 4：已经停住 ⇒ 只需定位（`SeekTicks(0)`）"
    );
    assert_eq!(ui.get_timecode(), "001.01.000");
    observe(&format!(
        "[logic-head] 行4 状态=停止@{} ＋ Space（连按第二次，1.5 s 内）⇒ 状态={:?} 位置={} tick \
         命令={:?} 时间码={}",
        playing_at,
        row4.state,
        row4.position_ticks,
        commands_since(&harness, base),
        ui.get_timecode(),
    ));

    // ---- 行 1（复验）：回起点之后的下一下 `Space` 必须是**新的第一次**（起播）----
    // 若实现忘了在回起点后清掉停住时刻，这一下会被困在"回起点"里 ⇒ 下面立刻红。
    let base = harness.journal().len();
    assert!(press(&ui, " "), "`Space` 必须被消费");
    let replay = harness.reading();
    assert_eq!(
        replay.state,
        TransportState::Playing,
        "行 1（复验）：回起点之后 `Space` 必须起播（不能被连按状态困住）"
    );
    assert_eq!(commands_since(&harness, base), vec![TransportCommand::Play]);
    observe(&format!(
        "[logic-head] 行1 复验 状态=停止@0 ＋ Space ⇒ 状态={:?} 位置={} tick 命令={:?}",
        replay.state,
        replay.position_ticks,
        commands_since(&harness, base),
    ));

    // ---- 行 3：停止（在中途，**没有**待配对的停住时刻）＋ `Space`（第一次）⇒ 从停止处继续 ----
    // 到达这个状态的方式：**停止按钮**（它回起点，并清掉待配对的停住时刻）＋ `seek(2880)`
    // —— 这正是"打开工程 / 拖播放头之后停在中途"的形态。
    //
    // ⚠ 本测试**只有一个活窗口**：Tier-1 平台是**线程级**的一次性安装（同一条测试线程上
    //   第二次 `set_platform` 会 `PlatformUnavailable`）⇒ 行 3 不能另起一个
    //   `TransportHarness`（第一版实测正是这样红的：`The platform has already been
    //   initialized`）。状态的**形状**才是判据的对象，不是"怎么走到那里"。
    ui.invoke_stop();
    assert_eq!(harness.reading().position_ticks, 0, "前置：停止按钮回起点");
    harness.seek_and_inject(2880);
    assert_eq!(harness.reading().state, TransportState::Stopped);
    let base = harness.journal().len();
    assert!(press(&ui, " "), "`Space` 必须被消费");
    let row3 = harness.reading();
    assert_eq!(row3.state, TransportState::Playing, "行 3：必须起播");
    assert!(
        row3.position_ticks >= 2880,
        "行 3：必须从停止处（2880）继续，而不是从 0 —— 实际 {} tick",
        row3.position_ticks
    );
    assert_eq!(commands_since(&harness, base), vec![TransportCommand::Play]);
    observe(&format!(
        "[logic-head] 行3 状态=停止@2880（无待配对停住）＋ Space（第一次）⇒ 状态={:?} \
         位置={} tick 命令={:?} 时间码={}",
        row3.state,
        row3.position_ticks,
        commands_since(&harness, base),
        ui.get_timecode(),
    ));

    // ---- 行 5：任意停住 ＋ `Return` ⇒ 回到起点 ----
    // 先把上面那条时间线停住并挪到中途（拖动播放头 / 打开工程的形态）。
    assert!(press(&ui, " "), "前置：停住");
    assert_eq!(harness.reading().state, TransportState::Stopped);
    harness.seek_and_inject(2880);
    let base = harness.journal().len();
    assert!(press(&ui, "\n"), "`Return` 必须被消费");
    let row5 = harness.reading();
    assert_eq!(row5.state, TransportState::Stopped, "行 5：停止态保持不变");
    assert_eq!(row5.position_ticks, 0, "行 5：必须回到起点");
    assert_eq!(
        commands_since(&harness, base),
        vec![TransportCommand::Stop, TransportCommand::SeekTicks(0)],
        "行 5：`Return` 落到 `on_stop`（停止 ＋ 回起点）"
    );
    assert_eq!(ui.get_timecode(), "001.01.000");
    observe(&format!(
        "[logic-head] 行5 状态=停止@2880 ＋ Return ⇒ 状态={:?} 位置={} tick 命令={:?} 时间码={}",
        row5.state,
        row5.position_ticks,
        commands_since(&harness, base),
        ui.get_timecode(),
    ));

    // ---- 行 6：播放中 ＋ `Return` ⇒ 回到起点 ＋ **继续播放** ----
    // 先把播放头推远（`drive_audio` = 控制面驱动，与判据夹具的"每次动作 1 个量子"同一条
    // `process_quantum`），证明 `Return` **不是**"从当前播放位置附近继续"。
    assert!(press(&ui, " "), "`Space` 必须被消费");
    harness.engine.borrow_mut().drive_audio(200);
    let before_return = harness.reading();
    assert_eq!(before_return.state, TransportState::Playing);
    assert!(
        before_return.position_ticks > 1_000,
        "前置：播放头必须已经远离起点"
    );
    let base = harness.journal().len();
    assert!(press(&ui, "\n"), "`Return` 必须被消费");
    let row6 = harness.reading();
    assert_eq!(row6.state, TransportState::Playing, "行 6：必须继续播放");
    assert!(
        row6.position_ticks < before_return.position_ticks,
        "行 6：必须从**起点**继续（{} tick），而不是停在原处（{} tick）",
        row6.position_ticks,
        before_return.position_ticks,
    );
    assert!(
        row6.position_ticks < 1_000,
        "行 6：回到起点后只应推进一个量子（实际 {} tick）",
        row6.position_ticks
    );
    assert_eq!(
        commands_since(&harness, base),
        vec![
            TransportCommand::Stop,
            TransportCommand::SeekTicks(0),
            TransportCommand::Play
        ],
        "行 6：`Stop` + `SeekTicks(0)` + `Play`"
    );
    assert_eq!(
        ui.get_timecode(),
        harness.view.timecode_at(row6.position_ticks),
        "行 6：时间码必须跟着新位置走"
    );
    observe(&format!(
        "[logic-head] 行6 状态=播放中@{} ＋ Return ⇒ 状态={:?} 位置={} tick 命令={:?} 时间码={}",
        before_return.position_ticks,
        row6.state,
        row6.position_ticks,
        commands_since(&harness, base),
        ui.get_timecode(),
    ));
}

/// 任务单 §2 第 4 条（a）：**连按窗口的边界** —— 纯函数在 1.5 s 前后逐点读数。
///
/// 判据不必睡 1.5 s 就能把边界钉死：`head_command` 是纯函数，时间戳是入参。
#[test]
fn head_double_tap_window_boundary_is_measured_on_the_pure_function() {
    use std::time::{Duration, Instant};
    use yeban_app::host::{HEAD_DOUBLE_TAP_WINDOW, HeadCommand, head_command};

    // ---- 纯函数：1.5 s 窗口的两侧与边界（单位 = 毫秒）----
    let now = Instant::now();
    let at = |ms: u64| {
        now.checked_sub(Duration::from_millis(ms))
            .expect("合成时间戳")
    };
    assert_eq!(
        HEAD_DOUBLE_TAP_WINDOW,
        Duration::from_millis(1_500),
        "窗口必须钉在 1.5 s（任务单举例值；改它要重新量边界）"
    );
    assert_eq!(
        head_command(true, None, now),
        HeadCommand::Stop,
        "播放中 ⇒ 停住"
    );
    assert_eq!(
        head_command(true, Some(at(0)), now),
        HeadCommand::Stop,
        "播放中优先于连按判定（第一次 Space 永远是停住）"
    );
    assert_eq!(
        head_command(false, None, now),
        HeadCommand::Play,
        "没有停住时刻 ⇒ 继续"
    );
    assert_eq!(
        head_command(false, Some(at(0)), now),
        HeadCommand::RewindToStart,
        "刚停住 ⇒ 连按第二次 ⇒ 回起点"
    );
    assert_eq!(
        head_command(false, Some(at(1_499)), now),
        HeadCommand::RewindToStart,
        "1.499 s ⇒ 仍在窗内"
    );
    assert_eq!(
        head_command(false, Some(at(1_500)), now),
        HeadCommand::RewindToStart,
        "1.500 s ⇒ 边界含（<=）"
    );
    assert_eq!(
        head_command(false, Some(at(1_501)), now),
        HeadCommand::Play,
        "1.501 s ⇒ 窗外 ⇒ 新的第一次 ⇒ 继续"
    );
    assert_eq!(
        head_command(
            false,
            Some(now.checked_add(Duration::from_millis(5)).expect("未来时刻")),
            now
        ),
        HeadCommand::RewindToStart,
        "时钟异常（停住时刻在未来）⇒ 保守按刚停住处理，⛔ 不 panic"
    );
    observe(&format!(
        "[logic-head] 连按窗口（纯函数，单位 ms）：0/1499/1500 ⇒ RewindToStart；1501/None ⇒ Play；\
         窗口={} ms",
        HEAD_DOUBLE_TAP_WINDOW.as_millis()
    ));
}

/// 任务单 §2 第 4 条（b）：**生产装配**里窗口的两侧读数（同一时间线、一个活窗口）。
///
/// 窗内：`Space` 停住 → 紧接着 `Space` ⇒ 回起点（`SeekTicks(0)`）。
/// 窗外：真实等待 1.55 s → `Space` ⇒ 从停住处继续（`Play`，位置不回退）。
///
/// ⚠ 一个测试只建一个活窗口（Tier-1 平台是**线程级**的一次性安装）。
#[test]
fn head_double_tap_is_measured_inside_and_outside_the_window_on_the_production_wiring() {
    use std::time::Duration;
    use yeban_app::host::HEAD_DOUBLE_TAP_WINDOW;

    // ---- 窗内连按（行 2 的停住 → 紧接着 Space）⇒ 回起点 ----
    let (harness, ui) = logic_head_harness();
    assert!(press(&ui, " "), "起播");
    assert!(press(&ui, " "), "停住（写下停住时刻）");
    let stopped_at = harness.reading().position_ticks;
    assert!(stopped_at > 0, "前置：必须停在中途");
    let base = harness.journal().len();
    assert!(press(&ui, " "), "窗内第二次");
    assert_eq!(harness.reading().position_ticks, 0, "窗内 ⇒ 回起点");
    assert_eq!(
        commands_since(&harness, base),
        vec![TransportCommand::SeekTicks(0)]
    );
    observe(&format!(
        "[logic-head] 窗内连按（生产装配）：停住@{} ⇒ 第二次 Space ⇒ 位置={} tick 命令={:?}",
        stopped_at,
        harness.reading().position_ticks,
        commands_since(&harness, base),
    ));

    // ---- 窗外（真实等待 1.55 s）⇒ 继续（**位置不回退**）----
    // 上一步停在起点（回起点之后状态是停住）；先起播、再停住（写下新的停住时刻），
    // 然后等窗口真的过期 —— 用的是生产回调里的 `Instant::now()`，⛔ 不是合成时间戳。
    assert!(press(&ui, " "), "起播");
    assert!(press(&ui, " "), "停住（写下停住时刻）");
    let stopped_at = harness.reading().position_ticks;
    assert!(stopped_at > 0, "前置：必须停在中途");
    std::thread::sleep(HEAD_DOUBLE_TAP_WINDOW + Duration::from_millis(50));
    let base = harness.journal().len();
    assert!(press(&ui, " "), "窗外");
    let resumed = harness.reading();
    assert_eq!(
        resumed.state,
        TransportState::Playing,
        "窗外 ⇒ 从停住处继续"
    );
    assert!(
        resumed.position_ticks >= stopped_at,
        "窗外 ⇒ 必须从停住处（{} tick）继续，实际 {} tick",
        stopped_at,
        resumed.position_ticks
    );
    assert_eq!(commands_since(&harness, base), vec![TransportCommand::Play]);
    observe(&format!(
        "[logic-head] 窗外连按（生产装配，真实等待 {} ms）：停住@{} ⇒ Space ⇒ 状态={:?} 位置={} tick \
         命令={:?}",
        HEAD_DOUBLE_TAP_WINDOW.as_millis() + 50,
        stopped_at,
        resumed.state,
        resumed.position_ticks,
        commands_since(&harness, base),
    ));
}

/// 任务单 §2 第 3 条：**`Return` 的新键映射** ＋ "它**不**借引擎"的证据。
///
/// 三层证据，彼此独立：
/// 1. **解析层**：`input.rs` 的裸 `Enter` 解析成 `Action::GoToBeginning`；
/// 2. **分类层**：`host::action_needs_engine(GoToBeginning) == false`（⛔ 不在借内）；
/// 3. **运行层**：**故意持借**引擎时按 `Return` —— 它仍被消费、不 panic、借用没被带走；
///    放掉借用之后再按一次，引擎真的回 0（落点是 `on_stop` 的 `Stop` + `SeekTicks(0)`）。
#[test]
fn return_is_mapped_to_go_to_beginning_and_does_not_take_the_engine_borrow() {
    use yeban_app::input::{
        Action as InputAction, InputContext, LogicalKey, Modifiers as InputModifiers, Resolution,
    };

    // ① 解析层。
    let context = InputContext::new();
    assert_eq!(
        context.resolve_logical(LogicalKey::Enter, InputModifiers::none()),
        Resolution::Action(InputAction::GoToBeginning),
        "裸 `Enter` / `Return` 必须解析成 `GoToBeginning`"
    );
    assert_eq!(
        context.resolve_logical(LogicalKey::Enter, InputModifiers::shift()),
        Resolution::Action(InputAction::AcceptAiSuggestion),
        "`Shift+Enter` 仍是 AI 采纳（未实现 ⇒ 不消费），⛔ 不得被 `Return` 抢走"
    );

    // ② 分类层：它**不在**借宿主的集合里。
    assert!(
        !host::action_needs_engine(InputAction::GoToBeginning),
        "`GoToBeginning` 的落点会调 `ui.invoke_stop()` / `ui.invoke_toggle_play()` ⇒ 必须在借外"
    );

    // ③ 运行层：故意持借 ⇒ 降级（不 panic、不带走守卫），放借后可用。
    let (mut harness, ui) = logic_head_harness();
    harness.seek_and_inject(2880);
    {
        let engine = Rc::clone(&harness.engine);
        let _guard = engine.borrow_mut();
        // ⛔ 若 `GoToBeginning` 在借内被分发（或 `wire_keys` 先借后判），这一行就是
        // `#212` 的重入 ⇒ 在真机上 = `key_down` 里的进程 abort。
        assert!(
            ui.invoke_key_action("\n".into(), false, false, false, false),
            "持借时的 `Return` 仍必须被消费（它的落点在借外）"
        );
    }
    // ⚠ 读数必须在守卫**作用域之外**：`TransportHarness::reading()` 自己也要 `borrow()`，
    // 持借期间读它就是 `RefCell already mutably borrowed`（第一版实测踩过）。
    assert_eq!(
        harness.reading().position_ticks,
        2880,
        "持借时 `on_stop` 降级 ⇒ 引擎一位不动（⛔ 不假装回起点）"
    );
    assert!(
        harness.engine.try_borrow_mut().is_ok(),
        "`Return` 返回之后借用必须已经放掉"
    );
    let base = harness.journal().len();
    assert!(press(&ui, "\n"), "放借后 `Return` 必须被消费");
    assert_eq!(
        harness.reading().position_ticks,
        0,
        "放借后必须真的回到起点"
    );
    assert_eq!(
        commands_since(&harness, base),
        vec![TransportCommand::Stop, TransportCommand::SeekTicks(0)],
        "`Return` 落到 `on_stop`（停止 ＋ 回起点）"
    );
    observe(&format!(
        "[logic-head] Return 映射：解析={:?} needs_engine=false；持借时消费=true 且引擎一位不动 \
         （命令增量 0）；放借后位置 2880 → {} tick 命令={:?}",
        Resolution::Action(InputAction::GoToBeginning),
        harness.reading().position_ticks,
        commands_since(&harness, base),
    ));
}

/// 把 **Windows 检出**（`core.autocrlf=true` ⇒ 每行 CRLF）的行尾归一到 LF。
///
/// ⚠ **第 299 条**（`#216` 的根因）：`include_str!` 把工作树里的字节**原样**编进来 ⇒
/// Windows 腿里同一份 `host.rs` 每行是 CRLF。跨行 `contains` / `find` 的 needle 若写死
/// `\n`，`\r` 会夹在中间 ⇒ **永远匹配不上**。凡按源码文本做结构对账，**入口先归一**。
///
/// ⛔ 只动 `\r\n`：裸 `\r` 是文件损坏，不该被归一掩盖（与 `yeban-mcp/tests/op_reachability.rs`
/// 的 R51 纪律同一条）。
fn lf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// 反过来：把 LF 源码换成 **Windows 检出**的字节（防线判据用，幂等）。
fn as_crlf(text: &str) -> String {
    lf(text).replace('\n', "\r\n")
}

/// 取 `host.rs` 里一个顶层 `pub fn {name}(` 的**代码区**（剥掉 `//` 注释），
/// 用于源码级对账。⭐ **入口先归一**（`#216`）⇒ LF / CRLF 检出得到同一份文本。
fn host_function_code(source: &str, name: &str) -> String {
    let source = lf(source);
    let marker = format!("pub fn {name}(");
    let start = source
        .find(&marker)
        .unwrap_or_else(|| panic!("`host.rs` 里必须有 `pub fn {name}(`"));
    let rest = &source[start..];
    let end = rest
        .find("\n}\n")
        .unwrap_or_else(|| panic!("`pub fn {name}` 必须以 `\\n}}\\n` 结束"));
    rest[..end]
        .lines()
        .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 函数体里出现过的 `Action::<变体>` 名字（去重、排序）。
fn action_variants_in(code: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut rest = code;
    while let Some(index) = rest.find("Action::") {
        let tail = &rest[index + "Action::".len()..];
        let name: String = tail
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
            .collect();
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
        rest = tail;
    }
    names.sort();
    names
}

/// 借宿主 / 引擎命令两个动作集的全部断言 —— LF 与 CRLF 两条腿**共用这一份**，
/// 因此"CRLF 下仍绿"证明的是**同一套探针**，不是另写一套更松的。
fn engine_action_sets(source: &str, label: &str) -> (Vec<String>, Vec<String>) {
    let borrow_predicate = action_variants_in(&host_function_code(source, "action_needs_engine"));
    let engine_command = action_variants_in(&host_function_code(source, "apply_engine_command"));
    assert_eq!(
        borrow_predicate, engine_command,
        "`action_needs_engine`（决定**借不借**宿主）与 `apply_engine_command`（决定**借来\
         做什么**）必须处理同一组 `Action`：一边改了、另一边没改 ⇒ 要么引擎命令拿不到\
         上下文（静默不生效），要么一条会调界面的动作被塞进了借内（`#212` 的崩法）\
         （{label} 源码）"
    );
    assert!(
        !borrow_predicate.is_empty(),
        "两个动作集不可能都是空的 —— 节拍器 / 预备拍今天就在这条链上（{label} 源码）"
    );
    // ⭐ 安全属性：落点里有 `ui.invoke_*` 的动作**一律**不在借内。
    // ⭐ 任务单 `#227` 追加 `GoToBeginning`：它的落点是 `ui.invoke_stop()` /
    // `ui.invoke_toggle_play()`（两个界面回调）⇒ 与上面三条同族，必须在借外。
    for ui_calling in ["PlayPause", "ResumeFromCursor", "Cancel", "GoToBeginning"] {
        assert!(
            !borrow_predicate.iter().any(|name| name == ui_calling),
            "`Action::{ui_calling}` 的落点会调界面回调（`ui.invoke_*`）⇒ 它**必须**在\
             借外执行；把它加进 `action_needs_engine` 就是 `#212` 的重入（{label} 源码）"
        );
    }
    (borrow_predicate, engine_command)
}

/// 判据（⭐ `#212` 的**结构耦合**）：**借宿主**的动作集必须与**引擎命令**的动作集逐条相同。
///
/// ## 为什么要有这条（而不是只靠运行时）
///
/// `wire_keys` 的 `if !action_needs_engine(action)` 与 `apply_engine_command` 的 `match`
/// 是**两处**必须同步的名单：前者决定"借不借"，后者决定"借来做什么"。若有人把一条
/// **会调界面**的新动作加进前者（或把一条引擎命令只加进后者），运行时判据不一定当场红，
/// 但 `#212` 的崩法会以另一种形式回来。
///
/// 本判据从**源码文本**取两个函数体里的 `Action::*` 名字集合，要求**逐条相等**；
/// 并钉住"界面回调族不在借内"这条安全属性（它们各自在 `host.rs` 的代码里只出现一次，
/// 见 `src/elements.rs` 的 `space_and_shift_space_share_one_host_arm_today`）。
/// ⭐ 任务单 `#227` 追加 `GoToBeginning`（`Return` 的落点会调 `ui.invoke_stop()` /
/// `ui.invoke_toggle_play()`）。
#[test]
fn only_engine_commands_take_the_engine_borrow() {
    let (borrow_predicate, engine_command) = engine_action_sets(include_str!("host.rs"), "LF");
    observe(&format!(
        "[transport-e2e] ⑧ 借宿主的动作集={borrow_predicate:?} == 引擎命令集={engine_command:?}；\
         界面回调族（PlayPause / ResumeFromCursor / Cancel / GoToBeginning）均不在借内"
    ));
}

/// 判据（`#216` 的**防线**）：源码被 Windows 检出成 **CRLF** 时，同一套源码探针必须仍绿。
///
/// ## 为什么要有这条（它守的是第 299 条）
///
/// `#212` 加的 [`only_engine_commands_take_the_engine_borrow`] 在 `#212` / `#213` / `#214`
/// 三条提交的 `windows` 腿上**连续三次红**：`find("\n}\n")` 的 needle 里是 LF，而
/// `core.autocrlf=true` 的检出把 `host.rs` 变成 CRLF ⇒ `\r` 夹在 `}` 与 `\n` 之间，
/// 锚点永远找不到（实测 panic 见报告）。修法是在探针**入口**做 [`lf`] 归一。
///
/// 本判据把**同一份 `host.rs`** 人工换成 CRLF，喂给**同一个** [`engine_action_sets`]
/// （不是另写一条更松的比较），要求结果与 LF 时逐条相同 ⇒ 将来谁再在探针里写死 `\n`，
/// 这条判据先红。⛔ 它不碰被测函数、不碰装配夹具：动的只是**文本输入**。
#[test]
fn source_probes_hold_on_a_crlf_checkout() {
    let lf_source = lf(include_str!("host.rs"));
    let crlf_source = as_crlf(&lf_source);

    assert!(!lf_source.contains('\r'), "仓库里的 `host.rs` 必须是 LF");
    assert!(crlf_source.contains("\r\n"), "防线夹具必须真的带 CRLF");
    assert_eq!(
        crlf_source.matches("\r\n").count(),
        lf_source.matches('\n').count(),
        "每一行都必须被换成 CRLF（⛔ 不许只换一半，否则这条防线只测到局部）"
    );

    let lf_sets = engine_action_sets(&lf_source, "LF");
    let crlf_sets = engine_action_sets(&crlf_source, "CRLF");
    assert_eq!(
        crlf_sets, lf_sets,
        "CRLF 检出下的探针结果必须与 LF 逐条相同 —— 若这里红，说明某个源码探针又写死了 `\\n`"
    );
    // 防空：两个集合都空时上面的相等是恒真，不能算数。
    assert!(
        !lf_sets.0.is_empty() && !lf_sets.1.is_empty(),
        "LF 探针必须真的抽到动作名（防空，⛔ 不是恒真）"
    );
    observe(&format!(
        "[#216] 同一份 `host.rs` 的 CRLF 副本 ⇒ 借宿主/引擎命令集与 LF 逐条相同：{:?}",
        lf_sets.0
    ));
}

/// 判据（**本票的端到端 ③：录音键真的接线**）：真指针点录音键 ⇒ **起录**（`recording` 置位、
/// 落盘路径被创建）⇒ 再点一次 ⇒ **停录**（`recording` 归零）；而**引擎读数一位不动**。
///
/// ## 为什么这条判据从"什么都不发生"改成了"真的接线"
///
/// 它的上一版（`pressing_the_record_button_today_changes_nothing`）逐字写着：**"谁真接了录音，
/// 就把这行连同该判据一并改成真实状态源"**（`ui/transport.slint` 里那段注释也同款）。
/// 本票按那句话的指示**改名 ＋ 反转断言**：⛔ 不是"把判据放松"，而是**能力真的到了**。
///
/// ## 仍然成立的诚实边界（⛔ 不假装硬件已验证）
///
/// 引擎的 `TransportState::Recording` 仍**不可达**（本票⛔ 不碰 `yeban-engine`）：录音走的是
/// [`crate::record_wav::Recorder`] 的**独立会话**，⛔ 不是引擎走带的一个档。
///
/// ⭐ **任务单 `#88` 起**：产品的默认源是**真设备输入流**（`host::wire_record` ＋
/// `record_device::default_input_factory`），"按下去有没有声音"由 `tests/record_device_wiring.rs`
/// 的逐码判据 ＋ `#[ignore]` 的真设备腿负责。本判据**不碰设备**：它把源**从代码缝**换成
/// 显式静音腿（见函数体），因此这条链在任何机器（含无声卡的 CI）上**确定性地**可判。
///
/// ## 怎么变红
///
/// | 注入 | 位置 | 现象 |
/// | :--- | :--- | :--- |
/// | 去掉 `rec_area` 的 `clicked` | `ui/transport.slint` | 本判据的"起录后 `checked` 必须为 `Some(true)`"红 ＋ 源码判据红 |
/// | 去掉 `host::wire_record` 的 `on_toggle_record` | `host.rs` | 起录后 `recording` 恒假 ⇒ 红 |
/// | 让引擎真的能进 `Recording` | `crates/yeban-engine/**` | ② 步的"从没有 `Recording`"红 |
#[test]
fn pressing_the_record_button_starts_and_stops_a_session() {
    let mut harness = TransportHarness::new();
    let ui = slint::ComponentHandle::clone_strong(harness.port.ui());

    // ⭐⭐ **任务单 `#88`：源从代码缝注入**（⛔ 判据既不读也不写环境变量，⛔ 也绝不在判据里
    // 打开真麦克风）。`build_live_ui` 装的是**产品**接线（`host::wire_transport` ⇒
    // `default_input_factory()` = 真设备源）；本判据的对象是"按钮 ⇒ 会话 ⇒ 落盘"这条
    // **界面接线**，与"源从哪来"正交 ⇒ 这里用同一个 `host::wire_record` **再装一次**，
    // 注入**显式静音腿**（确定性、零延迟、无硬件）。
    // ⚠ 为什么必须再装一次而不是改 `live_surface.rs`：那条文件属另一条工作线（本单⛔ 不碰）；
    // 而 Slint 的 `on_toggle_record` 是**设值**语义 ⇒ 后装的回调就是那一个。
    let record_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp/mod-app-record-button")
        .join(format!("take-{}.wav", std::process::id()));
    std::fs::create_dir_all(record_path.parent().expect("有父目录")).expect("建判据目录");
    let _ = std::fs::remove_file(&record_path);
    host::wire_record(
        &ui,
        None,
        // ⭐ `#132`：本判据只读按钮／引擎读数，不接工程端口。
        None,
        yeban_app::record_wav::RecordPlan {
            path: record_path.clone(),
            options: yeban_app::record_wav::RecordOptions::default(),
        },
        Rc::new(yeban_app::record_device::SilenceFactory),
        Rc::new(host::UiRecordOutcome),
    );

    // ---- 起点读数：引擎 + 日志 + 控件**自己**的勾选位 ----
    let before = harness.reading();
    let journal_before = harness.journal().len();
    // ⚠ `tree()` 是**快照** ⇒ 读数前必须 `refresh_tree`（否则永远读到构造时那一份，
    // 那会让"必须有状态"的判据永远看 false）。
    harness.port.refresh_tree(None).expect("刷树");
    let checked_before = harness
        .port
        .tree()
        .find_by_id("transport-record-button")
        .expect("录音键必须在运行时控件树里")
        .checked;
    assert_eq!(checked_before, Some(false), "起录之前必须是未置位");

    // ---- ① 真实指针事件：按下 + 抬起录音键 ⇒ **起录** ----
    harness
        .port
        .dispatch_pointer_down(
            "transport-record-button",
            16.0,
            16.0,
            yeban_ui_test_port::PointerButton::Left,
        )
        .expect("真实指针按下录音键");
    harness
        .port
        .dispatch_pointer_up(yeban_ui_test_port::PointerButton::Left)
        .expect("真实指针释放录音键");

    harness.port.refresh_tree(None).expect("刷树");
    let recording_live = ui.get_recording();
    let checked_recording = harness
        .port
        .tree()
        .find_by_id("transport-record-button")
        .expect("录音键仍在树里")
        .checked;
    assert!(
        recording_live,
        "⭐ 按下之后根属性 `recording` 必须为真（宿主是唯一写者）"
    );
    assert_eq!(
        checked_recording,
        Some(true),
        "⭐ 按下之后必须**置位** —— 宿主是 `recording` 的唯一写者"
    );

    // ---- ② 再按一次 ⇒ **停录**，且**落盘**了一个文件 ----
    harness
        .port
        .dispatch_pointer_down(
            "transport-record-button",
            16.0,
            16.0,
            yeban_ui_test_port::PointerButton::Left,
        )
        .expect("第二次按下");
    harness
        .port
        .dispatch_pointer_up(yeban_ui_test_port::PointerButton::Left)
        .expect("第二次释放");

    harness.port.refresh_tree(None).expect("刷树");
    assert!(
        !ui.get_recording(),
        "⭐ 停录之后根属性 `recording` 必须为假"
    );
    let checked_after = harness
        .port
        .tree()
        .find_by_id("transport-record-button")
        .expect("录音键仍在树里")
        .checked;
    assert_eq!(checked_after, Some(false), "⭐ 停录之后必须**归零**");
    assert!(
        record_path.is_file(),
        "⭐ 停录必须**真的**在 {} 落一个文件",
        record_path.display()
    );
    let bytes = std::fs::metadata(&record_path).expect("读文件").len();
    assert!(
        bytes > 44,
        "落下的必须是一个有样本的 WAV（> 44 字节头），实测 {bytes}"
    );

    // ---- ③ 诚实边界：**引擎读数一位不动**（录音⛔ 不是引擎走带的一个档）----
    let after = harness.reading();
    assert_eq!(
        after, before,
        "录音键**不得**改变引擎读数（录音走独立会话，⛔ 不是 TransportState::Recording）"
    );
    assert_eq!(
        harness.journal().len(),
        journal_before,
        "录音键**不得**产生任何引擎走带命令"
    );
    assert!(!ui.get_playing(), "录音键不得顺手改播放态");
    let observed: Vec<(&str, TransportState)> = {
        let mut engine = harness.engine.borrow_mut();
        vec![
            ("play", engine.play().state),
            ("stop", engine.stop().state),
            ("stop_and_rewind", engine.stop_and_rewind().state),
            ("seek(960)", engine.seek(960).state),
        ]
    };
    assert!(
        observed
            .iter()
            .all(|(_, state)| *state != TransportState::Recording),
        "`TransportState::Recording` 今天必须**不可达**（本票⛔ 不碰引擎）：{observed:?}"
    );
    observe(&format!(
        "[transport-e2e] ③ 真指针点录音键 ⇒ 起录/停录各一次（`checked`: {:?} ⇒ {:?} ⇒ {:?}）；\
         落盘 {} 字节于 {}；引擎读数一位未变（状态={:?}，日志恒 {} 条）\
         —— 录音走独立会话，`TransportState::Recording` 仍不可达；本判据的源是**注入的显式静音腿**\
         （`host::wire_record` 的代码缝），产品的真设备源由 `tests/record_device_wiring.rs` 负责",
        checked_before,
        checked_recording,
        checked_after,
        bytes,
        record_path.display(),
        after.state,
        journal_before,
    ));
    let _ = std::fs::remove_file(&record_path);
}

/// 判据（`[UI-A11Y-003]` §7.3 第 4 条 ① / `TEST-SPEC-005` 用例 ①）：
/// **真控件树里每一个交互控件都必须有一个非空的无障碍名称。**
///
/// 规范原文（`docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md` §7.3 第 4 条）：
/// "CI 流水线中必须包含至少 3 个屏幕阅读器集成测试用例：① 遍历主界面所有交互控件并断言
/// 无障碍名称非空"。本条就是 ①：遍历的是**活的运行时树**（真实 `MainWindow` + Tier-1
/// 平台），不是注册表 —— 因此它抓得到"动态标签算出来是空串"这一类注册表看不见的缺陷。
///
/// ## 两个读数都要非空（**缺一不可**）
///
/// 1. **树里的** `label`（`ui/tree` / `ui/node` 的读数，控制面消费者看这个）；
/// 2. **活元素上的** `accessible-label`（`ui/property id label`，屏读器看这个）。
///
/// 为什么两个都要（**实测教训, 不是偏好**）：把静态注册表合并进运行时树的那一步会**填**
/// 空标签 —— `yeban_ui_test_port::tree::ControlTree::merge_dynamic_flags_from` 的
/// `if node.label.is_empty() && !source.label.is_empty() { node.label.clone_from(&source.label) }`。
/// 于是"活元素上的标签是空串"会被注册表里的合成标签（`sidebar-item-{i}` 是
/// `"资源条目 {i}"`）**掩盖**：只读树的话，注入实验（把标签改回
/// `root.compact ? "" : …`）**不会变红** —— 本判据的第一版就是这样漏掉它的。
/// 第 2 个读数直接问活元素，因此注册表补不了它。
///
/// ## 为什么查**两个断点**
///
/// `[UI-GRID-002]` §1.2 的 1366–1919 断点把左栏收成 36px 图标导轨：折叠是**视觉**选择，
/// 但它曾经连带把 `sidebar-item-{i}` 的 `accessible-label` 写成 `""` ⇒ 折叠断点下 8 个
/// 可交互条目在无障碍树里全变成**无名**节点（屏读器读到 8 个一模一样的空项目）。
/// 只查默认断点会漏掉它，所以本判据在 `compact = false` 与 `compact = true` 下各遍历一次。
///
/// ## 与文本层判据的分工
///
/// `src/elements.rs` 的 `no_accessible_name_may_be_the_empty_literal` 抓"源码里写了 `""`"
/// （本机可跑、零 Slint）；本条抓"**运行时**这个节点真的没有名字"（只有 CI 能跑）。
/// 两半都在，才既早又真。
///
/// ## 注入证明（破坏 ⇒ 抓字面红行）
///
/// 把 `ui/sidebar.slint` 的 `accessible-label` 改回 `root.compact ? "" : …` ⇒ 第二遍
/// （`compact = true`）立刻红，红行里点名 `sidebar-item-0`（第 2 个读数）。
/// 见交付报告的注入实测。
#[test]
fn every_interactive_node_in_the_live_tree_carries_a_non_empty_accessible_name() {
    /// "交互控件"的角色族（§7.3 第 4 条 ① 的对象）。
    ///
    /// `region` / `main` / `complementary` / `groupbox` 是**容器**（它们也接指针事件,
    /// 但那是画布手势, 不是"控件"）；`text` / `image` 是读数与装饰。
    const INTERACTIVE_ROLES: [&str; 9] = [
        "button",
        "tab",
        "switch",
        "checkbox",
        "radio-button",
        "combobox",
        "slider",
        "text-input",
        "list-item",
    ];

    let view = ViewState::demo();
    let scene = DemoScene::from_view(&view);
    let registry = demo_registry();
    let static_tree = registry_to_tree(&registry).expect("注册表适配");
    let size = Size::new(scene.viewport_width, scene.viewport_height);
    let mut port = LivePort::new(size, Permission::ReadOnly, Some(&static_tree), || {
        build_demo_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + MainWindow");

    if port.tree().is_empty() {
        report_capability(
            "unavailable: crates/yeban-app 的 .slint 没有编译期 debug info ⇒ ElementHandle \
             遍历拿到一棵空树 (见 docs/ledger/ui-test-port-notes.md §2 #27 / §10 needs 0)",
        );
        panic!(
            "[UI-A11Y-003] 运行时控件树为空 ⇒ 判据的前置条件不满足（.slint 缺编译期 debug info）。\n\
             详见 test_port_adapter.rs 的 runtime_control_tree_cross_check_against_the_registry。"
        );
    }

    let mut total = 0_usize;
    for compact in [false, true] {
        port.ui().set_compact(compact);
        // `refresh_tree` 借的是 `port` 的**可变**借用；下一段还要用 `read_property`
        // 读活元素（不可变借用）⇒ 先把树拷下来，借用当场结束（本判据只看这一帧的快照）。
        let tree = port
            .refresh_tree(Some(&static_tree))
            .expect("UI 状态变了之后重抓控件树")
            .clone();
        let mut interactive = 0_usize;
        for node in tree.iter() {
            if !INTERACTIVE_ROLES.contains(&node.role.as_str()) {
                continue;
            }
            interactive += 1;
            assert!(
                !node.label.trim().is_empty(),
                "[UI-A11Y-003] §7.3 第 4 条 ①: `compact = {compact}` 下交互控件 `{}` \
                 (角色 `{}`) 的树内无障碍名称是空的 —— 屏读器读不出它是什么, 用户无法在 \
                 一组无名控件里做出选择。",
                node.id,
                node.role
            );
            // 第 2 个读数: **活元素**上的 `accessible-label`（注册表补不了它, 见文件头）。
            let live = port
                .read_property(&node.id, "label")
                .unwrap_or_else(|err| panic!("读活元素的 `{}`.label 失败: {err}", node.id));
            assert!(
                !live.trim().is_empty(),
                "[UI-A11Y-003] §7.3 第 4 条 ①: `compact = {compact}` 下交互控件 `{}` \
                 (角色 `{}`) 的**活元素**无障碍名称是空串 ⇒ 屏读器读到的是一个无名控件。\n\
                 ⚠ 树里的标签非空但活元素上是空的 ⇒ 那是静态注册表在填充 \
                 (`ControlTree::merge_dynamic_flags_from`), 不是真的有人写了名字。",
                node.id,
                node.role
            );
        }
        observe(&format!(
            "[a11y-①] compact = {compact}: 运行时树 {} 条, 其中交互控件 {interactive} 个, \
             树内标签与活元素上的 accessible-label 都非空",
            tree.len()
        ));
        assert!(
            interactive >= 20,
            "[a11y-①] compact = {compact} 下只找到 {interactive} 个交互控件 ⇒ 判据可能空转 \
             （角色表或树的可见性口径漂了）"
        );
        total += interactive;
    }
    observe(&format!(
        "[a11y-①] 两个断点合计遍历交互控件 {total} 个（`[UI-A11Y-003]` §7.3 第 4 条 ①）"
    ));
}
