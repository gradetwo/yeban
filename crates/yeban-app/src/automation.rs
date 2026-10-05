//! 自动化泳道的**投影**（纯函数、**零 Slint 依赖**）：`TrackV3::automation_lanes` → 可注入的折线。
//!
//! 规范来源 (Normative) / 上游接入点：
//! - `[MODEL-AST-002]` 泳道挂在 `TrackV3::automation_lanes`（`BTreeMap<AutomationTarget,
//!   AutomationLane>`），界面只读它；
//! - `[MODEL-AST-003]` 迭代顺序由 `BTreeMap` 的键序（`AutomationTarget::Ord`）保证确定；
//! - `[MODEL-AST-001]` / `[ARCH-DET-001]` 位置**一律**由整数 tick 经 [`crate::bridge::tick_to_px`]
//!   导出（本模块**没有**第二套 tick → 像素换算，也**没有**任何浮点累加）；
//! - `[UI-NOTE-002]` 坐标映射的投影侧一半；`[UI-TEST-001]` §12.2 语义 ID；
//! - ADR-0001 **D28**：投影层零 Slint；唯一注入点在 [`crate::host`]。
//!
//! `docs/ledger/model-automation-notes.md` §5 给了界面侧**唯一**的接入口径，本模块逐条照做：
//!
//! | 该调什么 | 本模块的落点 |
//! | :--- | :--- |
//! | `lane.unit()` 画轴标签 | [`AutomationLaneView::unit`] / `unit_symbol` / `axis_label` |
//! | `lane.effective_domain()` 定纵轴（`None` ⇒ 用曲线最值自适应） | [`AutomationLaneView::domain_min`] / `domain_max` / `domain_adaptive` |
//! | `lane.points_in_tick_order()` 画折线（相邻两点之间按左端点的 `curve` 采样 `ease`） | [`AutomationLaneView::points`]（每个采样点一个顶点）与 [`AutomationLaneView::samples`]（含缓动细采样） |
//! | `lane.read_enabled` / `lane.write_mode` 画开关与录制臂 | `read_enabled` / `write_mode` / `write_label` / `badge` |
//!
//! ## 为什么这里**没有**插值公式（本模块最重要的一条约束）
//!
//! 每一个顶点的值都来自模型层的那**一个**求值入口：
//!
//! - 采样点上的顶点：`lane.value_at(point.tick)` 逐位精确（模型文档 §2.2 的边界口径）；
//! - 采样点之间的缓动顶点：`project.automation_value_at(&target, tick)`。
//!
//! 本模块**不 import** `CurveType`、**不写** `ease`/`interpolate` 的任何形式 —— 因此
//! "app 里另写一份插值"这件事在**类型层面**就无法发生（不是靠注释承诺）。判据
//! `drawn_values_come_from_the_model_evaluation_entry` 把这条钉在数值上。
//!
//! ## 纵向映射的口径（值域 → 像素）
//!
//! [`AutomationAxis`] 把闭区间 `[min, max]` 线性映射到车道带的 `[0, height_px]`
//! （**y 向下**：值越大 y 越小）：
//!
//! ```text
//! fraction(value) = (value − min) / (max − min)        // 0.0 = 轴底, 1.0 = 轴顶
//! y_offset(value) = (1 − fraction) × height_px
//! ```
//!
//! 三个边界是**显式**的（都有判据）：
//!
//! | 情形 | fraction | 理由 |
//! | :--- | :--- | :--- |
//! | `min == max`（退化区间，例如自适应时曲线是常量） | `0.5` | 常量曲线画在带**正中**；除以 0 会得到 `NaN`/`±∞`，那是最难查的一类界面缺陷 |
//! | `value` 非有限 | `0.5` | 同上（投影不假设输入可信） |
//! | `value` 落在区间外 | 钳到 `0` / `1` | **只钳像素**：模型文档明确"`domain` 不是合法值钳位"（它只是显示域）⇒ 顶点里的 `value` 保留原始值，只有 `y` 被钉进车道带内 |
//!
//! ## 纵向布局（一条轨道上的多条泳道）
//!
//! 一条轨道可以有多个目标（音量 / 声相 / 发送 / 设备参数 / 宏），因此一条车道行里可能有
//! 多条泳道。本模块按 **`BTreeMap` 键序**把它们**等分**成多条带（`band_y` / `band_height`），
//! 于是"多泳道叠加"不是静默丢数据，而是"一条泳道一条带、顺序确定、几何可判据"。
//! 带的高度公式（**没有累加**，只有一次乘法）：
//!
//! ```text
//! usable      = 56 − 2×2                                    // 车道高 − 上下内缩
//! band_height = usable / lane_count
//! band_y      = 42 + 56×track_index + 2 + band_height × band_index
//! ```
//!
//! `42` / `56` 是 `ui/workspace/arrangement_view.slint` 里**已经在用**的车道几何
//! （`42px + 56px * track_index`）。两处各写一份是**已知的耦合**，由文本层判据
//! `lane_geometry_matches_the_arrangement_grid` 对账（口径与 `app-completion` 的
//! 音高车道 `14px * lane_index` 完全同源，本机不编译 Slint 时这是能拿到的最强证据）。
//!
//! ## 没画的（如实登记，不是静默降级）
//!
//! - **末点之后的保持段不画**：画到哪里属于视口 / 工程长度的口径（`[UI-NOTE-001]`），
//!   本模块不发明工程结束 tick；
//! - **主总线轨道的泳道不画**：编排视图没有主总线的车道行（主总线是调音台的通道条），
//!   因此 `master_bus_track_id` 那条轨道上的泳道被**跳过**（判据断言跳过是有意的）；
//! - **写入方向（拖动编辑 / 录制落点）不做**：本模块只有"模型 → 视图"一个方向（D28）。

use std::fmt::Write as _;

use yeban_model::project::{
    AutomationLane, AutomationPoint, AutomationTarget, AutomationUnit, AutomationValueDomain,
    AutomationWriteMode, TrackV3, YebanProjectV1,
};

use crate::bridge::{BridgeError, tick_to_px};

/// 编排视图里一条轨道所在车道的顶沿 y（与 `arrangement_view.slint` 的 `42px` 对齐）。
pub const TRACK_LANE_TOP_PX: f32 = 42.0;

/// 一条轨道车道的高度（与 `arrangement_view.slint` 的 `56px` 对齐）。
pub const TRACK_LANE_HEIGHT_PX: f32 = 56.0;

/// 自动化曲线带在车道内的纵向内缩（上下各一份，逻辑像素）。
///
/// 存在的理由不只是好看：带高必须是**有限正数**，否则元素几何退化成 0 高 ——
/// 而 0 面积/亚像素的元素会被上游裁剪语义过滤掉，于是"语义 ID 登记了却查不到"
/// （`docs/ledger/app-binding-notes.md` §4.1 踩过同一类问题）。
pub const AUTOMATION_BAND_INSET_PX: f32 = 2.0;

/// 相邻两个采样点之间插入多少段**缓动细采样**（段的端点是采样点本身）。
///
/// 为什么需要它：模型口径是"相邻两点之间按**左端点**的 `curve` 插值"（分段曲线，不是阶梯），
/// 而 Slint 的 `Path` 只画折线。用 `S` 段折线逼近一段曲线，误差随 `S` 下降；`S = 8` 时
/// 一条 3840 tick 的段每 480 tick 一个顶点（默认缩放下 16px 一段）。
///
/// 它**不是**插值实现：每个顶点的高度来自模型求值入口（见模块文档）。
pub const AUTOMATION_EASE_SAMPLES_PER_SEGMENT: u64 = 8;

/// 静态投影的求值 tick（"当前值"取哪一刻）。
///
/// **为什么是常量**：走带位置属**会话运行态**（`[MODEL-ISO-001]` 的第二层），
/// `YebanProjectV1` 里没有它，本线不发明。`0` = 工程起点，是一个确定且可判据的取值点；
/// 需要真实播放头时用 [`project_lanes_at_cursor`] 注入（一条后续线的接线动作）。
pub const AUTOMATION_CURSOR_TICK: u64 = 0;

/// 值域 → 车道带像素的**唯一**映射（纯计算，可判据）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutomationAxis {
    /// 取值域下界（`<= max`）。
    pub min: f32,
    /// 取值域上界（`>= min`）。
    pub max: f32,
    /// 车道带的高度（逻辑像素，`> 0` 时元素才可被语义寻址）。
    pub height_px: f32,
}

impl AutomationAxis {
    /// 由取值域与带高构造。
    #[must_use]
    pub const fn new(min: f32, max: f32, height_px: f32) -> Self {
        Self {
            min,
            max,
            height_px,
        }
    }

    /// 归一化位置：`0.0` = 轴底（`min`），`1.0` = 轴顶（`max`）。
    ///
    /// 退化区间（`max − min <= 0`，含 `NaN`）与非有限值一律给 `0.5`（带正中）——
    /// 见模块文档的三条边界。
    #[must_use]
    pub fn fraction(self, value: f32) -> f32 {
        let span = self.max - self.min;
        // `!span.is_finite()` 一并挡住 `NaN` 与 `±∞`：`NaN <= 0.0` 是假，
        // 只写比较会漏掉 `NaN`（而 `NaN` 会一路污染到像素）。
        if !span.is_finite() || span <= 0.0 || !value.is_finite() {
            return 0.5;
        }
        ((value - self.min) / span).clamp(0.0, 1.0)
    }

    /// 带内相对 y（逻辑像素，**y 向下**）：值越大越靠上。
    #[must_use]
    pub fn y_offset(self, value: f32) -> f32 {
        (1.0 - self.fraction(value)) * self.height_px
    }
}

/// 折线上的一个顶点（**已经换算好的逻辑像素**，界面不做任何算术）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutomationVertex {
    /// 该顶点的 tick。
    pub tick: u64,
    /// 该 tick 处的自动化值（**原始值**，越出显示域时**不**被钳位）。
    pub value: f32,
    /// 时间轴相对 x（逻辑像素，`= tick_to_px(tick)`，与 clips / notes 同一套换算）。
    pub x: f32,
    /// 车道带内相对 y（逻辑像素，由 [`AutomationAxis::y_offset`] 得出）。
    pub y: f32,
}

/// 一条自动化泳道的投影结果。
///
/// 字段分四组：**身份**（哪些能在控件树里寻址）、**轴**（单位与值域）、
/// **开关**（读/写状态）、**几何**（折线顶点与 `Path` 指令）。
#[derive(Debug, Clone, PartialEq)]
pub struct AutomationLaneView {
    /// 视图内泳道序号（轨道序 → `AutomationTarget::Ord` 序）。
    pub index: usize,
    /// 稳定的语义元素 ID：`track-{i}-automation-{target_key}-lane`（`[UI-TEST-001]` §12.2）。
    pub element_id: String,
    /// 模型的原始目标 —— 判据用它重新调用**唯一求值入口**（`automation_value_at`）。
    pub target: AutomationTarget,
    /// 目标键（`volume` / `pan` / `send-{edge}` / `device-{slot}-{param}` / `macro-{i}`）：
    /// 元素 ID 的第三段，也是"同一轨道上多个目标不撞车"的保证。
    pub target_key: String,
    /// 目标的人类可读短名（`音量` / 声相 / 设备参数名 …）。
    pub target_label: String,
    /// 目标所属音轨在视图里的序号（= `track-{i}` 的 `i`；主总线不计入）。
    pub track_index: usize,
    /// 目标音轨的身份规范文本。
    pub track_id: String,
    /// 目标音轨的显示名。
    pub track_name: String,
    /// 单位（**由目标派生**：`AutomationLane::unit()`，不是第二份事实源）。
    pub unit: AutomationUnit,
    /// 单位符号（`dB` / 空串），进轴标签与无障碍标签。
    pub unit_symbol: String,
    /// 纵轴下界（`effective_domain().min()`，或自适应时曲线的最小值）。
    pub domain_min: f32,
    /// 纵轴上界。
    pub domain_max: f32,
    /// 是否**自适应**（`effective_domain()` 为 `None`，即按曲线最值定纵轴）。
    pub domain_adaptive: bool,
    /// 纵轴文本（如 `dB [-60.0, 12.0]` / `[200.0, 4000.0] 自适应`），界面直接画。
    pub axis_label: String,
    /// 读开关（`AutomationLane::read_enabled`）。
    pub read_enabled: bool,
    /// 写模式（`AutomationLane::write_mode`）。
    pub write_mode: AutomationWriteMode,
    /// 写模式的界面短名（`关` / `写入` / `触碰` / `锁存`）。
    pub write_label: &'static str,
    /// 角标文本（`读关` / `● 触碰` / `读关 · ● 触碰` / 空串），界面直接画。
    pub badge: String,
    /// 无障碍标签：`"{轨道} · {目标} 自动化 {值}{单位}"`（读关闭 / 空泳道有各自的分支）。
    pub label: String,
    /// 本投影的求值 tick（[`AUTOMATION_CURSOR_TICK`] 或注入值）。
    pub cursor_tick: u64,
    /// `automation_value_at(target, cursor_tick)` 的结果（`None` = 无可用自动化值）。
    ///
    /// **它一定经过模型的那一个入口**，因此它已经遵守了 `read_enabled` 与"空泳道"的语义。
    pub value_at_cursor: Option<f32>,
    /// `target.static_value(&project)`（"没有自动化值"时的静态值），失败时 `None`。
    pub static_value: Option<f32>,
    /// 目标是否在文档里真的存在（`AutomationTarget::validate_against`）。
    pub target_reconciled: bool,
    /// 车道带顶沿（画布相对 y，逻辑像素）。
    pub band_y: f32,
    /// 车道带高度（逻辑像素）。
    pub band_height: f32,
    /// **每个采样点一个**顶点，按 `(tick, point_id)` 序（`[MODEL-AST-003]` 的确定顺序）。
    ///
    /// 同一 tick 上可能有多个采样点（模型允许）⇒ `tick`/`x` 是**非递减**而不是严格递增。
    pub points: Vec<AutomationVertex>,
    /// 画出来的折线顶点：含首点之前的**保持**顶点与相邻点之间的**缓动**细采样。
    ///
    /// `tick` 严格递增（同一个 tick 上由 `(tick, point_id)` 的最大者胜出 —— 与模型
    /// 求值的同 tick 裁决完全一致）。
    pub samples: Vec<AutomationVertex>,
    /// Slint `Path` 的 `commands`（局部坐标，相对车道带左上角）。
    pub path_commands: String,
}

impl AutomationLaneView {
    /// 采样点数量（= `points.len()`，判据 ② 的分母）。
    #[must_use]
    pub fn point_count(&self) -> usize {
        self.points.len()
    }

    /// 画出折线的顶点数量（`samples.len()`）。
    #[must_use]
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }
}

/// 目标 → 元素 ID 的第三段（**唯一**的目标命名口径）。
///
/// 口径（每个 `AutomationTarget` 变体都能唯一确定一个键）：
///
/// | 变体 | 键 | 例子 |
/// | :--- | :--- | :--- |
/// | `TrackVolume` | `volume` | `track-0-automation-volume-lane` |
/// | `TrackPan` | `pan` | `track-1-automation-pan-lane` |
/// | `SendGain { edge_id }` | `send-{edge_id}` | `track-0-automation-send-01J8…-lane` |
/// | `DeviceParam { slot, param }` | `device-{slot}-{param}` | `track-0-automation-device-0-0-lane` |
/// | `Macro { macro_index }` | `macro-{index}` | `track-2-automation-macro-0-lane` |
#[must_use]
pub fn target_key(target: &AutomationTarget) -> String {
    match target {
        AutomationTarget::TrackVolume { .. } => "volume".to_owned(),
        AutomationTarget::TrackPan { .. } => "pan".to_owned(),
        AutomationTarget::SendGain { edge_id, .. } => {
            format!("send-{}", edge_id.to_canonical_string())
        }
        AutomationTarget::DeviceParam {
            slot_index,
            param_index,
            ..
        } => format!("device-{slot_index}-{param_index}"),
        AutomationTarget::Macro { macro_index, .. } => format!("macro-{macro_index}"),
    }
}

/// 泳道的语义元素 ID（**唯一**的拼接口径）。
///
/// `ui/workspace/arrangement_view.slint` 用同一组字面段拼出它
/// （`"track-" + 轨道序号 + "-automation-" + 目标键 + "-lane"`）；
/// 两侧一致性由 `elements::slint_accessible_ids_and_registry_cover_each_other` 与
/// 文本层判据 `lane_element_ids_match_the_slint_template` 共同钉住。
#[must_use]
pub fn lane_element_id(track_index: usize, target_key: &str) -> String {
    format!("track-{track_index}-automation-{target_key}-lane")
}

/// 值 → 文本（**唯一**的数值格式化口径）。
///
/// dB 用一位小数（与 `TrackView::volume_display` 的 `{:.1}` 同形态，界面上不会出现
/// 两种精度）；其余单位用三位小数（`-1.000` / `0.500` / `1200.000`）。
/// 非有限值 → `"-"`（诚实输出，不"修成 0"）。
#[must_use]
pub fn value_text(unit: AutomationUnit, value: f32) -> String {
    if !value.is_finite() {
        return "-".to_owned();
    }
    let number = match unit {
        AutomationUnit::Decibels => format!("{value:.1}"),
        AutomationUnit::Normalized | AutomationUnit::Bipolar | AutomationUnit::Native => {
            format!("{value:.3}")
        }
    };
    let symbol = unit.symbol();
    if symbol.is_empty() {
        number
    } else {
        format!("{number} {symbol}")
    }
}

/// `AutomationWriteMode` → 界面短名。
#[must_use]
pub const fn write_label(mode: AutomationWriteMode) -> &'static str {
    match mode {
        AutomationWriteMode::Off => "关",
        AutomationWriteMode::Write => "写入",
        AutomationWriteMode::Touch => "触碰",
        AutomationWriteMode::Latch => "锁存",
    }
}

/// 拼一条泳道的无障碍标签（**唯一**的标签口径）。
///
/// 为什么用一个结构体而不是八个参数：`clippy::too_many_arguments` 的硬线是 7，
/// 而这条标签真的需要八个输入（少一个都会让某个分支说谎）。
struct LabelParts<'a> {
    track_name: &'a str,
    target_label: &'a str,
    unit: AutomationUnit,
    value: Option<f32>,
    static_value: Option<f32>,
    reconciled: bool,
    read_enabled: bool,
    has_points: bool,
    write_mode: AutomationWriteMode,
}

/// 标签的**状态段**（优先级：目标不存在 > 读关闭 > 有值 > 无采样点 > 无自动化值）。
///
/// `[UI-TEST-001]` 只允许语义 ID 寻址，而 `accessible-value` 不在 Slint 的无障碍属性
/// 清单里（`ui/property` 有 `accessible-value`，但投影只用 `accessible-label` 传数值 ——
/// 与色标 / 电平文本同款做法）。因此"AI/判据能读到自动化"的载体就是这一段文本。
fn label_state(parts: &LabelParts<'_>) -> String {
    if !parts.reconciled {
        return "目标不存在".to_owned();
    }
    if !parts.read_enabled {
        return match parts.static_value {
            Some(value) => format!("读关闭（静态 {}）", value_text(parts.unit, value)),
            None => "读关闭".to_owned(),
        };
    }
    if let Some(value) = parts.value {
        return value_text(parts.unit, value);
    }
    if !parts.has_points {
        return match parts.static_value {
            Some(value) => format!("无采样点（静态 {}）", value_text(parts.unit, value)),
            None => "无采样点".to_owned(),
        };
    }
    // 有点、读开 ⇒ 求值入口必然给出 `Some`；走到这里说明文档在两次读之间变了（不可能）。
    "无自动化值".to_owned()
}

/// 拼标签：`"{轨道} · {目标} 自动化 {状态}"`，录制臂另起一段。
///
/// 为什么会话态的**写模式**也进标签（而不仅是角标）：角标是画出来的像素，而读屏 / MCP
/// 的 `ReadOnly` 只能读控件树。把"录制臂开着"放进 `accessible-label`，于是
/// "AI 能读到自动化的读/写状态"不需要看截图 —— 与色标携带 `#RRGGBB` 是同一个理由。
fn lane_label(parts: &LabelParts<'_>) -> String {
    let mut text = format!(
        "{} · {} 自动化 {}",
        parts.track_name,
        parts.target_label,
        label_state(parts)
    );
    if !parts.write_mode.is_off() {
        let _ = write!(text, " · 录制臂 {}", write_label(parts.write_mode));
    }
    text
}

/// 值域：泳道显式声明的覆盖优先；否则按**曲线最值**自适应；空泳道退回中性单位区间。
///
/// 返回 `(min, max, adaptive)`。自适应只用采样点自身的值 —— 四种植入曲线
/// （`ease` 单调不减）在区间端点之间是单调的，因此**最值必然出现在采样点上**，
/// 不需要再扫一遍曲线（也就不会引入第二处求值）。
fn lane_domain(lane: &AutomationLane, ordered: &[AutomationPoint]) -> (f32, f32, bool) {
    if let Some(domain) = lane.effective_domain() {
        return (domain.min(), domain.max(), false);
    }
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    let mut seen = false;
    for point in ordered {
        if !point.value.is_finite() {
            continue;
        }
        min = min.min(point.value);
        max = max.max(point.value);
        seen = true;
    }
    if seen {
        return (min, max, true);
    }
    // 空泳道（或全部值非有限）：没有曲线可以自适应。取单位区间是**显式**的显示兜底，
    // 不影响任何顶点（没有顶点），只影响轴文本 —— 判据钉住这个取值。
    let fallback = AutomationValueDomain::new(0.0, 1.0).expect("单位区间的端点必然有限");
    (fallback.min(), fallback.max(), true)
}

/// 轴文本（`dB [-60.0, 12.0]` / `[200.0, 4000.0] 自适应`）。
fn axis_label(unit_symbol: &str, min: f32, max: f32, adaptive: bool) -> String {
    let prefix = if unit_symbol.is_empty() {
        ""
    } else {
        unit_symbol
    };
    let suffix = if adaptive { " 自适应" } else { "" };
    format!("{prefix} [{min:.1}, {max:.1}]{suffix}")
}

/// 顶点：把 `(tick, value)` 换算成带内逻辑像素。
fn vertex(
    tick: u64,
    value: f32,
    ticks_per_pixel: u64,
    axis: AutomationAxis,
) -> Result<AutomationVertex, BridgeError> {
    #[allow(clippy::cast_precision_loss)]
    let x = tick_to_px(tick, ticks_per_pixel)? as f32;
    Ok(AutomationVertex {
        tick,
        value,
        x,
        y: axis.y_offset(value),
    })
}

/// 采样点上的顶点（一个采样点一个，顺序 = `(tick, point_id)`）。
fn point_vertices(
    ordered: &[AutomationPoint],
    ticks_per_pixel: u64,
    axis: AutomationAxis,
) -> Result<Vec<AutomationVertex>, BridgeError> {
    let mut out = Vec::with_capacity(ordered.len());
    for point in ordered {
        out.push(vertex(point.tick, point.value, ticks_per_pixel, axis)?);
    }
    Ok(out)
}

/// 画出来的折线顶点：首点之前的**保持**段 + 采样点 + 采样点之间的**缓动**细采样。
///
/// 用 `BTreeMap` 收敛同一个 tick 的多个来源（同一 tick 上的多个采样点、以及细采样与
/// 采样点撞车的情形）：键是 tick ⇒ `samples` 的 `tick` **必然**严格递增，插入顺序不影响
/// 结果（`[MODEL-AST-003]` 的确定性），并且"同 tick 由 `(tick, point_id)` 最大者胜出"
/// 与模型求值的裁决逐字一致。
fn sample_vertices(
    project: &YebanProjectV1,
    target: &AutomationTarget,
    ordered: &[AutomationPoint],
    ticks_per_pixel: u64,
    axis: AutomationAxis,
) -> Result<Vec<AutomationVertex>, BridgeError> {
    let Some(first) = ordered.first() else {
        return Ok(Vec::new());
    };
    let mut by_tick: std::collections::BTreeMap<u64, f32> = std::collections::BTreeMap::new();
    // ① 首点之前是**保持**（模型口径"不外推"）⇒ 从时间轴 0 起画一条平线。
    if first.tick > 0 {
        by_tick.insert(0, first.value);
    }
    // ② 采样点本身（同 tick 时后写者胜 = `point_id` 最大者胜）。
    for point in ordered {
        by_tick.insert(point.tick, point.value);
    }
    // ③ 相邻两点之间的缓动细采样；值来自**模型求值入口**（本模块没有插值实现）。
    for pair in ordered.windows(2) {
        let (low, high) = (&pair[0], &pair[1]);
        let Some(span) = high.tick.checked_sub(low.tick) else {
            continue;
        };
        if span <= 1 {
            continue;
        }
        for step in 1..AUTOMATION_EASE_SAMPLES_PER_SEGMENT {
            // 先乘后除的**整数**运算：不引入浮点漂移（位置一律整数派生）。
            let offset = span * step / AUTOMATION_EASE_SAMPLES_PER_SEGMENT;
            let Some(tick) = low.tick.checked_add(offset) else {
                continue;
            };
            if tick <= low.tick || tick >= high.tick {
                continue;
            }
            // 唯一的求值入口；`Err`（目标对账失败）与 `None`（无值）都不画这个顶点 ——
            // 采样点上的顶点仍然在，因此曲线不会因此消失。
            if let Ok(Some(value)) = project.automation_value_at(target, tick) {
                by_tick.insert(tick, value);
            }
        }
    }
    let mut out = Vec::with_capacity(by_tick.len());
    for (tick, value) in by_tick {
        out.push(vertex(tick, value, ticks_per_pixel, axis)?);
    }
    Ok(out)
}

/// 折线 → Slint `Path` 的 `commands`（`M x y L x y …`，**局部坐标**，`{:.2}` 定位）。
///
/// 定点格式化是刻意的：同一个投影两次必须给出**逐字节**相同的字符串（确定性判据），
/// 而 `{}` 的浮点最短表示依赖平台库。坐标不带单位 —— `Path` 的坐标就在逻辑像素里
/// （上游语言参考：SVG 命令的坐标"operate within the imaginary coordinate system"）。
fn path_commands(samples: &[AutomationVertex]) -> String {
    let mut text = String::new();
    for (index, sample) in samples.iter().enumerate() {
        let command = if index == 0 { 'M' } else { 'L' };
        let _ = write!(text, "{command} {:.2} {:.2} ", sample.x, sample.y);
    }
    while text.ends_with(' ') {
        text.pop();
    }
    text
}

/// 目标的人类可读短名（设备参数 / 宏会去文档里取**名字** —— 只取名字，永不取值）。
fn target_label(track: &TrackV3, target: &AutomationTarget) -> String {
    match target {
        AutomationTarget::TrackVolume { .. } => "音量".to_owned(),
        AutomationTarget::TrackPan { .. } => "声相".to_owned(),
        AutomationTarget::SendGain { edge_id, .. } => {
            format!("发送 {}", edge_id.to_canonical_string())
        }
        AutomationTarget::DeviceParam {
            slot_index,
            param_index,
            ..
        } => track
            .devices
            .get(*slot_index)
            .and_then(|device| device.params.get(*param_index))
            .map_or_else(
                || format!("设备 {slot_index} 参数 {param_index}"),
                |param| param.name.clone(),
            ),
        AutomationTarget::Macro { macro_index, .. } => track
            .macros
            .get(*macro_index)
            .map_or_else(|| format!("宏 {macro_index}"), |macro_| macro_.name.clone()),
    }
    .trim()
    .to_owned()
}

/// 投影**一条**泳道。
///
/// 调用者保证 `band_y` / `band_height` 已经算好（见 [`project_lanes`] 的等分口径）。
///
/// # Errors
///
/// tick 超出像素可表示范围 → [`BridgeError`]（与 clips / notes 同一条政策：
/// **不**饱和、**不**回绕、**不** panic）。
#[allow(clippy::too_many_arguments)]
fn project_lane(
    project: &YebanProjectV1,
    track: &TrackV3,
    track_index: usize,
    index: usize,
    target: &AutomationTarget,
    lane: &AutomationLane,
    band_y: f32,
    band_height: f32,
    ticks_per_pixel: u64,
    cursor_tick: u64,
) -> Result<AutomationLaneView, BridgeError> {
    let ordered = lane.points_in_tick_order();
    let (domain_min, domain_max, domain_adaptive) = lane_domain(lane, &ordered);
    let axis = AutomationAxis::new(domain_min, domain_max, band_height);
    let points = point_vertices(&ordered, ticks_per_pixel, axis)?;
    let samples = sample_vertices(project, target, &ordered, ticks_per_pixel, axis)?;
    let unit = lane.unit();
    let unit_symbol = unit.symbol().to_owned();
    let key = target_key(target);
    let reconciled = target.validate_against(project).is_ok();
    // **唯一求值入口**：它已经遵守 `read_enabled` / 空泳道 / 目标对账三条语义。
    let value_at_cursor = project
        .automation_value_at(target, cursor_tick)
        .ok()
        .flatten();
    let static_value = target.static_value(project).ok();
    let parts = LabelParts {
        track_name: &track.name,
        target_label: &target_label(track, target),
        unit,
        value: value_at_cursor,
        static_value,
        reconciled,
        read_enabled: lane.read_enabled,
        has_points: !ordered.is_empty(),
        write_mode: lane.write_mode,
    };
    let mut badge = String::new();
    if !lane.read_enabled {
        badge.push_str("读关");
    }
    if !lane.write_mode.is_off() {
        if !badge.is_empty() {
            badge.push_str(" · ");
        }
        badge.push_str("● ");
        badge.push_str(write_label(lane.write_mode));
    }
    let commands = path_commands(&samples);
    Ok(AutomationLaneView {
        index,
        element_id: lane_element_id(track_index, &key),
        target: *target,
        target_key: key,
        target_label: target_label(track, target),
        track_index,
        track_id: track.id.to_canonical_string(),
        track_name: track.name.clone(),
        unit,
        unit_symbol: unit_symbol.clone(),
        domain_min,
        domain_max,
        domain_adaptive,
        axis_label: axis_label(&unit_symbol, domain_min, domain_max, domain_adaptive),
        read_enabled: lane.read_enabled,
        write_mode: lane.write_mode,
        write_label: write_label(lane.write_mode),
        badge,
        label: lane_label(&parts),
        cursor_tick,
        value_at_cursor,
        static_value,
        target_reconciled: reconciled,
        band_y,
        band_height,
        points,
        samples,
        path_commands: commands,
    })
}

/// 投影工程里**全部**非主总线轨道上的泳道（默认求值 tick）。
///
/// # Errors
///
/// 见 [`project_lane`]。
pub fn project_lanes(
    project: &YebanProjectV1,
    ticks_per_pixel: u64,
) -> Result<Vec<AutomationLaneView>, BridgeError> {
    project_lanes_at_cursor(project, ticks_per_pixel, AUTOMATION_CURSOR_TICK)
}

/// 同 [`project_lanes`]，但显式指定"当前值"的求值 tick（走带位置的接线点）。
///
/// 顺序（**确定**）：轨道按 `YebanProjectV1::tracks` 的键序遍历、跳过主总线，
/// 轨道内按 `automation_lanes` 的键序（`AutomationTarget::Ord`）遍历。
///
/// # Errors
///
/// 见 [`project_lane`]。
pub fn project_lanes_at_cursor(
    project: &YebanProjectV1,
    ticks_per_pixel: u64,
    cursor_tick: u64,
) -> Result<Vec<AutomationLaneView>, BridgeError> {
    let mut lanes = Vec::new();
    for (track_index, track) in project
        .tracks
        .values()
        .filter(|track| track.id != project.master_bus_track_id)
        .enumerate()
    {
        let lane_count = track.automation_lanes.len();
        if lane_count == 0 {
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let band_height =
            (TRACK_LANE_HEIGHT_PX - 2.0 * AUTOMATION_BAND_INSET_PX) / lane_count as f32;
        #[allow(clippy::cast_precision_loss)]
        let track_offset = TRACK_LANE_HEIGHT_PX * track_index as f32;
        for (band_index, (target, lane)) in track.automation_lanes.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let band_y = TRACK_LANE_TOP_PX
                + track_offset
                + AUTOMATION_BAND_INSET_PX
                + band_height * band_index as f32;
            lanes.push(project_lane(
                project,
                track,
                track_index,
                lanes.len(),
                target,
                lane,
                band_y,
                band_height,
                ticks_per_pixel,
                cursor_tick,
            )?);
        }
    }
    Ok(lanes)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::str::FromStr as _;

    use yeban_model::ids::EntityId;
    use yeban_model::music::CurveType;
    use yeban_model::project::AutomationPoint;

    use super::*;
    use crate::bridge::{DEFAULT_TICKS_PER_PIXEL, ViewState, demo_project, tick_to_px};

    fn id(text: &str) -> EntityId {
        EntityId::from_str(text).expect("判据夹具的 ULID 必须合法")
    }

    /// 判据夹具的 ULID：与演示夹具同一个 24 字符前缀 + 两字符尾（规范 26 字符）。
    fn tid(tail: &str) -> EntityId {
        id(&format!("01J8Z5Q0R7K3M9X2V4B6N8P0{tail}"))
    }

    /// 一个最小工程的夹具：一条音量泳道 + 一条设备参数泳道（自适应值域）。
    ///
    /// 它**不用** `yeban_model::samples`（那是上游的规范样本，本线只读），而是本模块
    /// 自己造的边界夹具：两个采样点、一段 `SCurve`、一条 `read_enabled = false`。
    fn fixture_project() -> (YebanProjectV1, AutomationTarget, AutomationTarget) {
        use yeban_model::project::{DeviceDefinition, DeviceKind, ParameterValue};

        let mut project = demo_project();
        let track_id = *project
            .tracks
            .keys()
            .find(|id| **id != project.master_bus_track_id)
            .expect("演示工程必须有非主总线轨道");
        let volume = AutomationTarget::TrackVolume { track_id };
        let device_target = AutomationTarget::DeviceParam {
            track_id,
            slot_index: 0,
            param_index: 0,
        };
        let track = project.tracks.get_mut(&track_id).expect("轨道存在");
        track.devices = vec![DeviceDefinition {
            id: tid("Y1"),
            name: "Fixture Synth".to_owned(),
            kind: DeviceKind::InternalInstrument,
            bypassed: false,
            params: vec![ParameterValue {
                name: "cutoff".to_owned(),
                value: 1200.0,
                unit: Some("Hz".to_owned()),
            }],
            latency_samples: 0,
        }];
        track.automation_lanes.clear();
        let mut points: BTreeMap<EntityId, AutomationPoint> = BTreeMap::new();
        for (text, tick, value, curve) in [
            ("Y2", 0_u64, -60.0_f32, CurveType::Linear),
            ("Y3", 3840, 12.0, CurveType::SCurve),
        ] {
            let point_id = tid(text);
            points.insert(
                point_id,
                AutomationPoint {
                    id: point_id,
                    tick,
                    value,
                    curve,
                },
            );
        }
        track.automation_lanes.insert(
            volume,
            AutomationLane {
                target: volume,
                points,
                read_enabled: true,
                write_mode: AutomationWriteMode::Touch,
                domain: None,
            },
        );
        let mut device_points: BTreeMap<EntityId, AutomationPoint> = BTreeMap::new();
        let point_id = tid("Y4");
        device_points.insert(
            point_id,
            AutomationPoint {
                id: point_id,
                tick: 0,
                value: 200.0,
                curve: CurveType::Exponential,
            },
        );
        track.automation_lanes.insert(
            device_target,
            AutomationLane {
                target: device_target,
                points: device_points,
                read_enabled: false,
                write_mode: AutomationWriteMode::Off,
                domain: None,
            },
        );
        (project, volume, device_target)
    }

    /// 判据夹具用：某条泳道在 `tick = 0` 处由**模型入口**给出的值。
    fn entry_here(
        lane: &AutomationLaneView,
        project: &YebanProjectV1,
        target: &AutomationTarget,
    ) -> f32 {
        assert_eq!(lane.cursor_tick, AUTOMATION_CURSOR_TICK);
        project
            .automation_value_at(target, lane.cursor_tick)
            .expect("目标存在")
            .expect("读开且有点 ⇒ 有值")
    }

    /// 判据 ①：**泳道元素数 == 工程里的泳道数**（纯 Rust 侧；运行时的那一半在 CI 的
    /// `project_projection_reaches_the_control_tree_and_the_pixels`）。
    #[test]
    fn lane_count_follows_the_project() {
        let filled =
            ViewState::from_project(&yeban_model::samples::filled_project()).expect("投影");
        let filled_lanes: usize = yeban_model::samples::filled_project()
            .tracks
            .values()
            .map(|track| track.automation_lanes.len())
            .sum();
        assert_eq!(
            filled.automation_lanes.len(),
            filled_lanes,
            "泳道数必须等于工程里 `automation_lanes` 的条目总数"
        );
        assert_eq!(filled_lanes, 1, "规范样本 `filled_project` 有 1 条音量泳道");

        let demo = ViewState::demo();
        let demo_project_total: usize = demo_project()
            .tracks
            .values()
            .map(|track| track.automation_lanes.len())
            .sum();
        assert_eq!(demo.automation_lanes.len(), demo_project_total);
        assert!(
            demo.automation_lanes.len() > filled.automation_lanes.len(),
            "演示工程必须比规范样本有更多泳道（判据 ⑥ 的换工程证据）"
        );

        // 元素 ID 唯一、格式良好、且每一族都能在注册表里找到。
        let mut ids: Vec<&str> = demo
            .automation_lanes
            .iter()
            .map(|lane| lane.element_id.as_str())
            .collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "泳道元素 ID 不得重复: {ids:?}");
        for lane in &demo.automation_lanes {
            assert!(
                crate::elements::is_well_formed_id(&lane.element_id),
                "泳道元素 ID 必须格式良好: {}",
                lane.element_id
            );
            assert_eq!(
                lane.element_id,
                lane_element_id(lane.track_index, &lane.target_key),
                "元素 ID 必须由 (轨道序号, 目标键) 唯一决定"
            );
            assert!(lane.element_id.ends_with("-lane"));
        }
        // 空工程：一条泳道都没有，也不 panic。
        assert!(ViewState::empty().automation_lanes.is_empty());
    }

    /// 判据 ②：**顶点数与采样点数一致且 tick 有序**。
    ///
    /// - `points.len() == lane.points.len()`（采样点一个不多一个不少）；
    /// - `points` 的 tick 非递减（`(tick, point_id)` 序，模型允许同 tick 多点）；
    /// - `samples`（真正画出来的折线）的 tick **严格**递增；
    /// - 每个采样点的 tick 都在 `samples` 里出现（曲线穿过每一个采样点）；
    /// - `x` 与 `tick_to_px(tick)` 逐位相等（**复用了同一个换算**，不是第二套布局）。
    #[test]
    fn vertices_follow_the_points_in_tick_order() {
        let (project, volume, _) = fixture_project();
        let lanes = project_lanes(&project, DEFAULT_TICKS_PER_PIXEL).expect("投影");
        let lane = lanes
            .iter()
            .find(|lane| lane.target == volume)
            .expect("音量泳道必须被投影");
        let model_points = project
            .automation_lane(&volume)
            .expect("泳道存在")
            .points_in_tick_order();
        assert_eq!(lane.point_count(), model_points.len());
        assert_eq!(
            lane.points.iter().map(|v| v.tick).collect::<Vec<_>>(),
            model_points.iter().map(|p| p.tick).collect::<Vec<_>>()
        );
        assert!(
            lane.points
                .windows(2)
                .all(|pair| pair[0].tick <= pair[1].tick),
            "采样点顶点必须按 tick 非递减"
        );
        assert!(
            lane.samples
                .windows(2)
                .all(|pair| pair[0].tick < pair[1].tick),
            "折线顶点的 tick 必须严格递增（同一 tick 由 point_id 最大者胜出）"
        );
        assert!(
            lane.samples.len() > lane.points.len(),
            "SCurve 段必须被细采样"
        );
        for point in &model_points {
            assert!(
                lane.samples.iter().any(|vertex| vertex.tick == point.tick),
                "采样点 tick={} 必须出现在折线上",
                point.tick
            );
        }
        for lane in &ViewState::demo().automation_lanes {
            for vertex in lane.points.iter().chain(lane.samples.iter()) {
                #[allow(clippy::cast_precision_loss)]
                let expected = tick_to_px(vertex.tick, DEFAULT_TICKS_PER_PIXEL)
                    .expect("演示夹具的 tick 不越界") as f32;
                assert_eq!(
                    vertex.x, expected,
                    "x 必须等于 `bridge::tick_to_px(tick)`（不得另写一套换算）: tick={}",
                    vertex.tick
                );
            }
        }
    }

    /// 判据 ③：**纵轴映射正确**（两个已知点 → y 的像素关系），含**值域自适应**那一条。
    #[test]
    fn value_to_pixel_mapping_is_exact_and_adapts_when_the_domain_is_unknown() {
        let axis = AutomationAxis::new(-60.0, 12.0, 52.0);
        // 端点精确：值域下界 ⇒ 带底（y = 高），上界 ⇒ 带顶（y = 0）。
        assert_eq!(axis.fraction(-60.0), 0.0);
        assert_eq!(axis.y_offset(-60.0), 52.0);
        assert_eq!(axis.fraction(12.0), 1.0);
        assert_eq!(axis.y_offset(12.0), 0.0);
        // 中点与四分之一点：线性关系（y 随值**递减**）。
        assert!((axis.fraction(-24.0) - 0.5).abs() < 1e-6);
        assert!((axis.y_offset(-24.0) - 26.0).abs() < 1e-6);
        assert!((axis.y_offset(-42.0) - 39.0).abs() < 1e-6);
        // 值域外的值：**像素**被钉在带内，但顶点里的 `value` 不钳位。
        assert_eq!(axis.fraction(-120.0), 0.0);
        assert_eq!(axis.fraction(60.0), 1.0);
        let vertex = super::vertex(0, 60.0, DEFAULT_TICKS_PER_PIXEL, axis).expect("顶点");
        assert_eq!(vertex.value, 60.0, "模型值不得被显示域钳位");
        assert_eq!(vertex.y, 0.0, "像素必须被钉进车道带内");
        // 退化区间（自适应时曲线是常量）：带正中，绝不 NaN。
        let flat = AutomationAxis::new(0.0, 0.0, 52.0);
        assert_eq!(flat.fraction(0.0), 0.5);
        assert_eq!(flat.y_offset(123.0), 26.0);
        assert!(flat.y_offset(f32::NAN).is_finite());

        // ---- 自适应那一条：设备参数没有固有值域 ⇒ 按曲线最值定纵轴 ----
        let (project, _, device_target) = fixture_project();
        let lanes = project_lanes(&project, DEFAULT_TICKS_PER_PIXEL).expect("投影");
        let lane = lanes
            .iter()
            .find(|lane| lane.target == device_target)
            .expect("设备参数泳道必须被投影");
        assert!(lane.domain_adaptive, "没有固有值域的泳道必须自适应");
        assert!(lane.target_reconciled, "设备参数的目标必须能在文档里对账");
        assert_eq!(lane.unit, AutomationUnit::Native);
        assert_eq!(lane.unit_symbol, "");
        assert!((lane.domain_min - 200.0).abs() < f32::EPSILON);
        assert!((lane.domain_max - 200.0).abs() < f32::EPSILON);
        assert!((lane.domain_min - lane.domain_max).abs() < f32::EPSILON);
        assert!(
            lane.axis_label.contains("[200.0, 200.0]") && lane.axis_label.contains("自适应"),
            "轴文本必须同时说明量程与「自适应」: {}",
            lane.axis_label
        );

        // 显式取值域（音量泳道：`filled_project` 的 `[-60, 12]`）优先于自适应。
        let filled =
            ViewState::from_project(&yeban_model::samples::filled_project()).expect("投影");
        let volume = &filled.automation_lanes[0];
        assert!(!volume.domain_adaptive, "显式取值域不得被当成自适应");
        assert!((volume.domain_min + 60.0).abs() < f32::EPSILON);
        assert!((volume.domain_max - 12.0).abs() < f32::EPSILON);
        assert_eq!(volume.unit_symbol, "dB");
        assert!(volume.axis_label.starts_with("dB [-60.0, 12.0]"));

        // 单调性（跨整个演示工程）：值越大 ⇒ y 越小 或 相等（不同泳道的轴不同，
        // 因此只在**同一条泳道**里比较）。
        for lane in &ViewState::demo().automation_lanes {
            for pair in lane.samples.windows(2) {
                if pair[0].value > pair[1].value {
                    assert!(
                        pair[0].y <= pair[1].y + 1e-6,
                        "同一条泳道里值更大 ⇒ y 不得更大（{} → {}）",
                        pair[0].value,
                        pair[1].value
                    );
                }
            }
        }
    }

    /// 判据 ④：**空泳道 / 单点泳道 / 越界 tick** 的行为是明确的（不 panic）。
    #[test]
    fn empty_single_point_and_absurd_lanes_behave_explicitly() {
        // ---- 空泳道 ----
        let mut project = demo_project();
        let track_id = *project
            .tracks
            .keys()
            .find(|id| **id != project.master_bus_track_id)
            .expect("非主总线轨道");
        let target = AutomationTarget::TrackVolume { track_id };
        let track = project.tracks.get_mut(&track_id).expect("轨道存在");
        track.automation_lanes.clear();
        track.automation_lanes.insert(
            target,
            AutomationLane {
                target,
                points: BTreeMap::new(),
                read_enabled: true,
                write_mode: AutomationWriteMode::Off,
                domain: None,
            },
        );
        let lanes = project_lanes(&project, DEFAULT_TICKS_PER_PIXEL).expect("空泳道必须能投影");
        let lane = &lanes[0];
        assert_eq!(lane.point_count(), 0);
        assert_eq!(lane.sample_count(), 0);
        assert_eq!(lane.path_commands, "", "空泳道不得画出任何折线");
        assert_eq!(lane.value_at_cursor, None, "空泳道没有自动化值");
        assert!(lane.read_enabled, "空泳道仍然可以是「读开」的");
        assert!(
            lane.label.contains("无采样点"),
            "空泳道的标签必须显式说明: {}",
            lane.label
        );
        assert!(lane.target_reconciled, "目标存在，只是没有点");

        // ---- 单点泳道：处处保持（模型口径），因此折线只有一个顶点 ----
        let point_id = tid("Y5");
        {
            let track = project.tracks.get_mut(&track_id).expect("轨道存在");
            if let Some(lane) = track.automation_lanes.get_mut(&target) {
                lane.points.insert(
                    point_id,
                    AutomationPoint {
                        id: point_id,
                        tick: 1920,
                        value: -7.5,
                        curve: CurveType::Linear,
                    },
                );
            }
        }
        let lanes = project_lanes(&project, DEFAULT_TICKS_PER_PIXEL).expect("单点泳道必须能投影");
        let lane = &lanes[0];
        assert_eq!(lane.point_count(), 1);
        assert_eq!(
            lane.sample_count(),
            2,
            "单点泳道多一个「从 0 起保持」的顶点"
        );
        assert_eq!(lane.samples[0].tick, 0);
        assert_eq!(lane.samples[0].value, -7.5);
        assert_eq!(lane.samples[1].tick, 1920);
        assert_eq!(lane.samples[1].value, -7.5);
        assert_eq!(lane.value_at_cursor, Some(-7.5), "单点泳道处处保持");
        assert_eq!(
            project
                .automation_value_at(&target, u64::MAX)
                .expect("求值"),
            Some(-7.5)
        );
        assert!(lane.path_commands.starts_with("M "));

        // ---- 越界 tick：报错而不是 panic / 饱和（与 clips 同一条政策） ----
        {
            let track = project.tracks.get_mut(&track_id).expect("轨道存在");
            if let Some(lane) = track.automation_lanes.get_mut(&target)
                && let Some(point) = lane.points.get_mut(&point_id)
            {
                point.tick = u64::MAX;
            }
        }
        assert!(matches!(
            project_lanes(&project, DEFAULT_TICKS_PER_PIXEL),
            Err(BridgeError::PixelOverflow { .. })
        ));
        // `ticks_per_pixel = 0` 同样是显式错误（不是静默的除零）。
        assert!(matches!(
            project_lanes(&demo_project(), 0),
            Err(BridgeError::ZeroTicksPerPixel)
        ));
    }

    /// 判据 ⑤：**`read_enabled = false` 的泳道在控件树里可区分**（纯 Rust 侧）。
    ///
    /// 三条独立证据：标签带 `读关闭`、角标带 `读关`、`value_at_cursor` 是 `None`
    /// （读关的泳道不参与"当前值"—— 那条语义由模型入口决定，不是界面自己判断）。
    #[test]
    fn read_disabled_lanes_are_distinguishable() {
        let (project, volume, device_target) = fixture_project();
        let lanes = project_lanes(&project, DEFAULT_TICKS_PER_PIXEL).expect("投影");
        let enabled = lanes
            .iter()
            .find(|lane| lane.target == volume)
            .expect("音量泳道");
        let disabled = lanes
            .iter()
            .find(|lane| lane.target == device_target)
            .expect("设备参数泳道");
        assert!(enabled.read_enabled && !disabled.read_enabled);
        assert!(disabled.label.contains("读关闭"), "{}", disabled.label);
        assert!(disabled.badge.contains("读关"), "{}", disabled.badge);
        assert!(!enabled.label.contains("读关闭"));
        // 角标：读开的泳道只显示录制臂（写模式）；读关且 `Off` 的只显示 `读关`。
        assert_eq!(enabled.badge, "● 触碰", "读开的泳道角标只带录制臂");
        assert!(
            !disabled.badge.contains("●"),
            "写模式为 `Off` 时不得出现录制臂角标"
        );
        assert!(
            !enabled.label.contains("读关闭") && !enabled.badge.contains("读关"),
            "读开的泳道不得出现任何读关闭标记"
        );
        assert_eq!(disabled.value_at_cursor, None);
        // 读关时求值入口同样返回 `None`（界面**没有**自己判断读开关）。
        assert_eq!(
            project
                .automation_value_at(&device_target, 0)
                .expect("目标存在"),
            None
        );
        // 但曲线本身仍然被画出来（关掉的是"应用"，不是"显示"）。
        assert_eq!(disabled.point_count(), 1);
        assert!(!disabled.path_commands.is_empty());
        assert!(disabled.static_value.is_some());

        // 写模式角标（`filled_project` 的 Touch）。
        let filled =
            ViewState::from_project(&yeban_model::samples::filled_project()).expect("投影");
        let lane = &filled.automation_lanes[0];
        assert_eq!(lane.write_mode, AutomationWriteMode::Touch);
        assert_eq!(lane.write_label, "触碰");
        assert!(lane.badge.contains("触碰"), "{}", lane.badge);
        assert!(
            lane.label.contains("录制臂 触碰"),
            "写模式必须进无障碍标签（读屏/MCP 只能读控件树）: {}",
            lane.label
        );
    }

    /// 判据 ⑥：**换工程 ⇒ 泳道随之变化**（同一份投影代码，两个工程）。
    #[test]
    fn switching_the_project_changes_the_lanes() {
        let demo = ViewState::demo();
        let filled =
            ViewState::from_project(&yeban_model::samples::filled_project()).expect("投影");
        let demo_ids: Vec<&str> = demo
            .automation_lanes
            .iter()
            .map(|lane| lane.element_id.as_str())
            .collect();
        let filled_ids: Vec<&str> = filled
            .automation_lanes
            .iter()
            .map(|lane| lane.element_id.as_str())
            .collect();
        assert_ne!(demo_ids, filled_ids);
        assert_eq!(filled_ids, vec!["track-0-automation-volume-lane"]);
        assert!(demo_ids.contains(&"track-0-automation-volume-lane"));
        assert!(
            demo_ids.iter().any(|id| id.contains("-device-")),
            "演示工程必须有一条自适应值域的泳道: {demo_ids:?}"
        );
        // 空工程：投影成空数组（不是"上一次的残留"）。
        assert!(ViewState::empty().automation_lanes.is_empty());
    }

    /// 判据 ⑦（纯 Rust 侧）：注册表与 `.slint` 的泳道族**双向覆盖**。
    ///
    /// 运行时的"未登记 0"在 CI 的 `runtime_control_tree_cross_check_against_the_registry`
    /// 里断言；本机不编译 Slint，因此这里断言的是注册表 ↔ `.slint` 文本模板的覆盖关系
    /// （由 `elements::slint_accessible_ids_and_registry_cover_each_other` 承担主体，
    /// 这里再加一条"泳道族 == 泳道数"的正面断言）。
    #[test]
    fn registry_carries_one_element_per_lane() {
        let demo = ViewState::demo();
        let registry = crate::elements::ElementRegistry::from_view(&demo);
        let lane_ids: Vec<&str> = registry
            .iter()
            .filter(|meta| meta.id.ends_with("-lane") && meta.id.contains("-automation-"))
            .map(|meta| meta.id.as_str())
            .collect();
        assert_eq!(lane_ids.len(), demo.automation_lanes.len());
        for lane in &demo.automation_lanes {
            let meta = registry
                .get(&lane.element_id)
                .unwrap_or_else(|| panic!("注册表缺少泳道 {}", lane.element_id));
            assert_eq!(meta.component, "workspace/arrangement_view.slint");
            assert_eq!(meta.kind, crate::elements::ElementKind::Image);
            assert_eq!(meta.label, lane.label, "注册表标签必须与投影逐字一致");
            assert!(!meta.dynamic_region, "自动化曲线不是每帧跳变的区域");
        }
        // 注册表里的泳道 ID 全部属于"模型驱动族"（负向断言因此覆盖它们）。
        for id in lane_ids {
            assert!(crate::elements::is_model_driven_family(id), "{id}");
        }
    }

    /// 判据 ⑧：**画出来的每一个值都来自模型的那一个求值入口**。
    ///
    /// 这是"没有第二份插值实现"的数值证据：
    /// 1. 每个采样点顶点 == `lane.value_at(point.tick)`（逐位）；
    /// 2. 每个细采样顶点 == `project.automation_value_at(&target, tick)`（逐位）；
    /// 3. 一个**已知 tick**（`SCurve` 段的中点）上的折线值 == 入口的返回值，
    ///    且**不等于**手写线性插值的结果 —— 后一条是"忽略 `curve`"注入的靶子。
    /// 4. `path_commands` 的顶点数与 `samples` 一致，且坐标逐位可复原。
    #[test]
    fn drawn_values_come_from_the_model_evaluation_entry() {
        let (project, volume, _) = fixture_project();
        let lane = project_lanes(&project, DEFAULT_TICKS_PER_PIXEL)
            .expect("投影")
            .into_iter()
            .find(|lane| lane.target == volume)
            .expect("音量泳道");
        let model_lane = project.automation_lane(&volume).expect("泳道");

        for vertex in &lane.points {
            assert_eq!(
                Some(vertex.value),
                model_lane.value_at(vertex.tick),
                "采样点顶点 tick={} 必须逐位等于 `lane.value_at`",
                vertex.tick
            );
        }
        for vertex in &lane.samples {
            assert_eq!(
                project
                    .automation_value_at(&volume, vertex.tick)
                    .expect("目标存在"),
                Some(vertex.value),
                "折线顶点 tick={} 必须逐位等于 `automation_value_at`",
                vertex.tick
            );
        }

        // 已知 tick：`[0, -60.0] → [3840, +12.0]`，左端点曲线是 `Linear`，
        // 因此 tick=1920 的值 = -24.0（模型口径）。
        let known = lane
            .samples
            .iter()
            .find(|vertex| vertex.tick == 1920)
            .expect("tick=1920 必须被采样");
        let entry = project
            .automation_value_at(&volume, 1920)
            .expect("目标存在")
            .expect("读开且有点 ⇒ 有值");
        assert_eq!(known.value, entry);
        assert!((known.value + 24.0).abs() < 1e-6, "实测 {}", known.value);
        // 中点高度：`(-24 - (-60)) / (12 - (-60)) = 0.5` ⇒ 带高的正中。
        assert!((known.y - lane.band_height / 2.0).abs() < 1e-4);
        assert!(
            lane.label.contains("-60.0 dB"),
            "标签必须携带**求值 tick** 处的当前值（静态投影的 tick 是 0）: {}",
            lane.label
        );
        assert!(
            lane.label
                .contains(&format!("{:.1} dB", entry_here(&lane, &project, &volume))),
            "标签里的数值必须来自 `automation_value_at` 的返回值: {}",
            lane.label
        );

        // ---- 反例：`SCurve` 段上的中点**不**等于线性插值 ----
        // 造一条 `[0, 0.0] --SCurve--> [1000, 1.0]` 的泳道：tick=500 时
        // `ease(0.5) = 0.5` 恰好与线性相同，因此取四分之一点（`u(0.25) = 0.15625`）。
        //
        // 目标刻意用 `DeviceParam`（模型里**唯一**没有固有取值域的目标）：
        // 于是这条判据同时覆盖"自适应纵轴"——`min/max` 来自曲线最值 `[0, 1]`，
        // 而不是目标的固有值域（TrackVolume 的固有值域是 `[-60, 12]`，用它就测不到自适应）。
        let mut project = demo_project();
        let track_id = *project
            .tracks
            .keys()
            .find(|id| **id != project.master_bus_track_id)
            .expect("非主总线轨道");
        let target = AutomationTarget::DeviceParam {
            track_id,
            slot_index: 0,
            param_index: 0,
        };
        let track = project.tracks.get_mut(&track_id).expect("轨道存在");
        track.automation_lanes.clear();
        let mut points: BTreeMap<EntityId, AutomationPoint> = BTreeMap::new();
        for (text, tick, value, curve) in [
            ("Y6", 0_u64, 0.0_f32, CurveType::SCurve),
            ("Y7", 1000, 1.0, CurveType::Linear),
        ] {
            let point_id = tid(text);
            points.insert(
                point_id,
                AutomationPoint {
                    id: point_id,
                    tick,
                    value,
                    curve,
                },
            );
        }
        track.automation_lanes.insert(
            target,
            AutomationLane {
                target,
                points,
                read_enabled: true,
                write_mode: AutomationWriteMode::Off,
                domain: None,
            },
        );
        let curved = project_lanes(&project, DEFAULT_TICKS_PER_PIXEL)
            .expect("投影")
            .into_iter()
            .find(|lane| lane.target == target)
            .expect("泳道");
        let quarter = curved
            .samples
            .iter()
            .find(|vertex| vertex.tick == 250)
            .expect("四分之一点必须被采样");
        let model_value = project
            .automation_value_at(&target, 250)
            .expect("目标存在")
            .expect("有值");
        assert_eq!(quarter.value, model_value);
        let linear_value = 0.0 + (1.0 - 0.0) * (250.0 / 1000.0);
        assert!(
            (quarter.value - 0.156_25).abs() < 1e-6,
            "实测 {}",
            quarter.value
        );
        assert!(
            (quarter.value - linear_value).abs() > 0.05,
            "SCurve 的四分之一点必须显著偏离线性插值（{quarter_value} vs {linear_value}）—— \
             否则「忽略 curve」的注入不会被抓住",
            quarter_value = quarter.value,
            linear_value = linear_value
        );
        // 自适应值域 [0, 1]：四分之一点的高度 = (1 − 0.15625) × 带高。
        assert!(
            curved.domain_adaptive,
            "`DeviceParam` 没有固有值域 ⇒ 必须自适应"
        );
        assert!(curved.domain_min.abs() < f32::EPSILON);
        assert!((curved.domain_max - 1.0).abs() < f32::EPSILON);
        let expected_y = (1.0 - 0.156_25) * curved.band_height;
        assert!(
            (quarter.y - expected_y).abs() < 1e-4,
            "自适应量程 [0, 1] 下 y 必须等于 (1 − u(0.25)) × 带高: {} vs {}",
            quarter.y,
            expected_y
        );

        // ---- `path_commands` 与 `samples` 一致（界面画的就是这些顶点） ----
        let parsed: Vec<(f32, f32)> = curved
            .path_commands
            .split(' ')
            .collect::<Vec<_>>()
            .chunks(3)
            .map(|chunk| {
                assert!(matches!(chunk[0], "M" | "L"), "指令: {:?}", chunk[0]);
                (
                    chunk[1].parse::<f32>().expect("x 必须可解析"),
                    chunk[2].parse::<f32>().expect("y 必须可解析"),
                )
            })
            .collect();
        assert_eq!(parsed.len(), curved.sample_count());
        for (index, (x, y)) in parsed.iter().enumerate() {
            let vertex = curved.samples[index];
            // `{:.2}` 的舍入误差上界是 0.005（闭区间端点），因此容差取 0.0051。
            assert!(
                (x - vertex.x).abs() <= 0.0051,
                "第 {index} 个顶点的 x 不可复原: 命令 {x} vs 投影 {}",
                vertex.x
            );
            assert!(
                (y - vertex.y).abs() <= 0.0051,
                "第 {index} 个顶点的 y 不可复原: 命令 {y} vs 投影 {}",
                vertex.y
            );
        }
        assert!(curved.path_commands.starts_with("M "), "首指令必须是 M");
    }

    /// 判据（文本层）：`.slint` 里的泳道元素 ID 模板与投影的拼接口径**一致**。
    ///
    /// 本机不编译 Slint，因此"我写对了 accessible-id 的三段字面量"只能以读原文的方式
    /// 断言（与 `app-completion` 的 `slint_text_contracts_for_colors_and_pitch_lanes`
    /// 同源）。真正的判决是 CI 的 `cargo build`。
    #[test]
    fn lane_element_ids_match_the_slint_template() {
        let ui = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
        let source = std::fs::read_to_string(ui.join("workspace/arrangement_view.slint"))
            .expect("读 arrangement_view.slint");
        let code: String = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            code.contains("\"track-\" + root.automation-lane-track-indexes[lane_index]"),
            "泳道 ID 的第一段必须来自投影的轨道序号"
        );
        assert!(
            code.contains(
                "\"-automation-\" + root.automation-lane-target-keys[lane_index] + \"-lane\""
            ),
            "泳道 ID 的第二/三段必须是 `-automation-` + 目标键 + `-lane`"
        );
        for forbidden in [
            "automatic-lane",
            "automation-lane-target-ke\"",
            "note-",
            "clip-",
        ] {
            assert!(
                !code.contains(&format!("accessible-id: \"track-\" + root.automation-lane-track-indexes[lane_index] + \"{forbidden}")),
                "泳道 ID 模板里出现了可疑字面量 `{forbidden}`"
            );
        }
        // 几何耦合：`.slint` 的车道网格必须与投影的 42 / 56 常量一致。
        assert!(
            code.contains("42px + 56px * track_index"),
            "投影的 TRACK_LANE_TOP_PX / TRACK_LANE_HEIGHT_PX 与 .slint 的网格脱节"
        );
        // 折线必须是 Path + 注入的 commands（不是界面里内联的折线）。
        assert!(code.contains("commands: root.automation-path-commands[lane_index]"));
        assert!(code.contains("viewbox-width: parent.width / 1px"));
        assert!(code.contains("fit: ImageFit.fill"));
    }
}
