//! 显示诚实性与 UI 一致性的源码层看守（细节实测 / 控件逐一体检 @ `3ff6914`）。
//!
//! 两份报告发现的核心问题是"画出来但不是真的"：波形、EQ CURVE、BUS COMP、
//! 设备机架卡片、混音台通道条 EQ 缩略这 5 处全是写死的几何路径，不接任何真实
//! 数据；此外还有主题链路断掉、BUFFER 读数被裁、chip 箭头字形不渲染、空态
//! 提示被右缘裁断、Mac 键位写法等一致性缺口。
//!
//! `fabricated_state_honesty` 只钉住两处旧的编造态。本文件把这两份报告的
//! 修复**钉在源码层**：任一处被静默改回去（假曲线画回来、`Theme` global
//! 又只看 `persona`、BUFFER 盒子又没了显式宽度……）对应判据立即变红。
//!
//! 与 `fabricated_state_honesty` 同款：本文件**不编译 Slint**，只读 `.slint`
//! 源码文本（`include_str!`）——文件被删 / 改名在编译期就红，判据不会
//! "读不到就跳过"。

#![allow(unused)]

const ARRANGEMENT: &str = include_str!("../ui/arrangement.slint");
const LEFT_PANEL: &str = include_str!("../ui/left_panel.slint");
const LOWER_DOCK: &str = include_str!("../ui/lower_dock.slint");
const TRANSPORT_BAR: &str = include_str!("../ui/transport_bar.slint");
const TOKENS: &str = include_str!("../ui/tokens.slint");
const MODALS: &str = include_str!("../ui/modals.slint");

/// 滤掉**整行注释**之后剩下的源码行（与 `fabricated_state_honesty` 同一口径：
/// 注释里解释"为什么删"必然提到被删的标识符，整份 `contains` 会误判）。
fn code_lines(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 五处（七个路径串）写死的假几何，逐字抄自 `3ff6914` 实测报告。
const FABRICATED_COMMANDS: [&str; 7] = [
    "M 0 12 L 6 4 L 12 14 L 18 3 L 26 15 L 34 2", // 波形上半
    "M 0 12 L 6 18 L 12 13 L 18 20 L 26 13 L 34 21", // 波形下半镜像
    "M 0 28 Q 20 25, 35 12 T 70 18 T 100 24",     // 检查器 EQ CURVE
    "M 0 20 Q 35 18, 55 12 T 100 16",             // 检查器 BUS COMP
    "M 0 36 Q 40 12, 80 34 T 160 18 T 240 32",    // 设备机架第一张卡
    "M 0 48 Q 55 42, 110 20 T 220 34",            // 设备机架其余卡
    "M 0 18 Q 20 8, 38 16 T 55 12 T 72 20",       // 混音台通道条 EQ 缩略
];

/// **判据 ①**：五处假几何不再被画回来（报告 §3 / 控件体检 §4）。
#[test]
fn no_fabricated_waveform_or_eq_curves_are_drawn() {
    let sources = [ARRANGEMENT, LEFT_PANEL, LOWER_DOCK];
    for cmd in FABRICATED_COMMANDS {
        for (i, src) in sources.iter().enumerate() {
            assert!(
                !code_lines(src).contains(cmd),
                "假几何路径又被画回来了：源文件 #{i} 的代码行含 `{cmd}`"
            );
        }
    }
}

/// **判据 ②**：**未接真实数据**的替代方案必须是**明确标注的占位**，而不是又一条假曲线。
/// `EQ CURVE` 卡片已接 `bridge::calculate_eq_curve_svg` 的真实曲线（0dB 缺省仍是
/// 基准线）⇒ 它不再属于占位，改由 `ui_unfixed_list_regression.rs` 钉住实数据链路；
/// 设备机架卡片（C2）已接 `bridge::device_curve_svgs`（逐台 EQ 频响 / 压缩传递曲线），
/// 非 EQ/压缩设备仍保留显式占位；波形 / BUS COMP / 通道条 EQ 仍未接，继续要求「示意」标注。
#[test]
fn placeholders_are_explicitly_labelled_as_schematic() {
    assert!(code_lines(ARRANGEMENT).contains("波形待分析 · 示意"));
    // EQ CURVE 已接真实 eq-curve-svg（bridge::calculate_eq_curve_svg），不再是示意占位。
    assert!(code_lines(LEFT_PANEL).contains("commands: root.eq-curve-svg"));
    assert!(!code_lines(LEFT_PANEL).contains("EQ CURVE · 示意"));
    assert!(code_lines(LEFT_PANEL).contains("BUS COMP · 示意"));
    // 设备卡片的真实曲线必须由投影属性驱动；占位文案只作为空串兜底。
    assert!(
        code_lines(LOWER_DOCK).contains("root.device-curve-svgs[idx]"),
        "设备卡片必须消费逐台真实曲线属性"
    );
    assert!(code_lines(LOWER_DOCK).contains("曲线示意 · 未连接参数"));
    assert!(code_lines(LOWER_DOCK).contains("EQ · 示意"));
}

/// **判据 ③**：`--theme` 的调色板真的流到窗口绘制。
///
/// 报告 §6 的三组对比 0 像素差异，根因是 `tokens.slint` 的 `export global Theme`
/// 只按 `persona` 取色、完全不读 `ThemeState.theme`，而 `--theme` 改的正是
/// `ThemeState.theme`。默认 `yeban` 仍走原表达式（保护 5 张 golden 基准），
/// 其余主题改读主题感知的 `Tokens.*` 角色令牌。
#[test]
fn theme_global_actually_reads_theme_state() {
    let after = TOKENS
        .split_once("export global Theme {")
        .expect("tokens.slint 必须有 export global Theme")
        .1;
    assert!(
        after.contains("ThemeState.theme"),
        "`Theme` global 必须读 ThemeState.theme，否则 --theme 对像素无效"
    );
    assert!(after.contains("Tokens.bg-void"));
    assert!(after.contains("Tokens.accent"));
    assert!(after.contains("YebanTheme.yeban"));
}

/// **判据 ④**：BUFFER 读数盒子有显式宽度（细节实测 §4.6 / 控件体检 §5）。
#[test]
fn buffer_readout_has_explicit_width() {
    let src = code_lines(TRANSPORT_BAR);
    assert!(src.contains("text: \"BUFFER:\""));
    assert!(
        src.contains("width: 128px;"),
        "BUFFER 盒子必须显式定宽，否则挂在 alignment:end 右岛里会被裁成 BUFF"
    );
}

/// **判据 ⑤**：chip 用能渲染的中文标签，不再依赖缺失的 ▸ / ▾ 字形。
#[test]
fn chips_use_renderable_labels_instead_of_missing_arrow_glyphs() {
    let src = code_lines(ARRANGEMENT);
    assert!(!src.contains("▾"), "不渲染的 ▾ 不应再出现在代码行");
    assert!(!src.contains("▸"), "不渲染的 ▸ 不应再出现在代码行");
    assert!(src.contains("和弦收起"));
    assert!(src.contains("和弦展开"));
    assert!(src.contains("轨道菜单"));
    assert!(!src.contains("·收起") && !src.contains("·展开") && !src.contains("·菜单"));
}

/// **判据 ⑥**：空编排区提示按视口居中且可换行（细节实测 §4.8）。
#[test]
fn empty_state_hint_is_centered_on_viewport() {
    let src = code_lines(ARRANGEMENT);
    assert!(src.contains("在此区域拖拽音频、MIDI 文件或双击以新增轨道"));
    assert!(
        src.contains("wrap: word-wrap"),
        "空态提示必须按视口宽度换行，否则整句被内容网格推到右缘只剩半句"
    );
}

/// **判据 ⑦**：界面文案不再用 Mac-only 的 ⌘ / Cmd 写法（报告 §5.11 / §2.5）。
#[test]
fn mac_only_key_labels_are_gone() {
    for src in [ARRANGEMENT, MODALS, TRANSPORT_BAR] {
        let code = code_lines(src);
        assert!(!code.contains("⌘K"), "⌘K 应统一为 Ctrl+K");
        assert!(!code.contains("[Cmd+T]"), "[Cmd+T] 应统一为 [Ctrl+T]");
    }
}
