//! `YebanProjectV1` → 视图状态的**投影层**（纯函数，零 Slint 依赖）。
//!
//! 规范来源 (Normative):
//! - `[MODEL-AST-001]` 960 PPQ 整数时钟：位置**一律**由整数 tick 导出，
//!   投影里没有任何一处用浮点累加位置（`ARCH-DET-001` 的 L1/L2 确定性契约）。
//! - `[MODEL-AST-002]` `YebanProjectV1` 是唯一权威持久化结构，界面只读它。
//! - `[UI-GRID-001]` / `[UI-GRID-002]` 网格与断点（断点判定仍在 [`crate::scene`]）。
//! - `[UI-TEST-001]` §12.2 语义 Element ID 的 `{ulid}` 段必须来自**真实实体身份**，
//!   不能是手写常量 —— 本模块把 `EntityId` 的 26 字符规范文本直接交给 `.slint`。
//! - `[MODEL-ISO-001]` 三层状态物理隔离：走带位置 / 当前分支属**会话运行态**，
//!   不在 `YebanProjectV1` 里，因此 [`crate::scene::DemoScene`] 明确标注它们是占位。
//!
//! ## 为什么这一层要存在，而且要零 Slint 依赖
//!
//! 在这个模块出现之前，`src/scene.rs` 的 `demo()` 是界面的**唯一数据源** ——
//! 一组 `&'static str` 常量。界面因此"好看但装不进工程"。投影层把
//! "模型字段 → 视图字段"的映射收敛成**一个纯函数**，带来三件事：
//!
//! 1. 界面侧只剩"取字段"与"画像素"，没有业务判断；
//! 2. 这一层不含 Slint，于是它**能在本机用 `rustc --edition 2024 --test` 真跑判据**
//!    （本机纪律禁止编译 Slint，见 `AGENTS.md` §5）；
//! 3. `.slint` 里的 `for … in 6` 变成 `for … in root.tracks` —— 轨道数 / 剪辑数 /
//!    段落数由工程决定，而不是由界面里的字面量决定。
//!
//! ## 位置为什么必须是整数运算
//!
//! `tick → 像素` 走**整数除法**（[`tick_to_px`]），`像素 → tick` 走
//! **`checked_mul`**（[`px_to_tick`]）。两者互为往返：对任意像素 `p`，
//! `tick_to_px(px_to_tick(p)?)? == p` 恒成立。用浮点累加（`x += dt / tpp`）会让
//! `tpp = 30` 这种除不尽的比例产生 1e-16 量级的漂移，累积到第 N 个剪辑就变成
//! "同一工程在两台机器上导出不同的 x" —— 这正是 `ARCH-DET-001` 禁止的东西。
//!
//! ## 溢出策略（不 panic，也不静默回绕）
//!
//! `u64` 的 tick 与 `duration_ticks` 相加、`u32` 的像素乘以 `ticks_per_pixel`
//! 都可能越界。投影**返回 `Result`**，越界即 [`BridgeError`]，绝不 wrap 或饱和后假装正常。
//! 空工程是合法输入（[`YebanProjectV1::default`]），投影成空视图而不 panic。
//!
//! ## app-completion 工作线补的两件事（见 `docs/ledger/app-completion-notes.md`）
//!
//! 1. **卷帘音符位置**：`MidiNote::start_tick` / `pitch` 经 [`tick_to_px`] 与
//!    [`pitch_lane`] 的**整数**变换给出 `x` / `y` / `width` / `row`（[`NoteView`]）——
//!    界面不再用"第 i 个音符"的索引布局（`[UI-NOTE-002]`）。
//! 2. **轨道色标**：`TrackV3::color` 在**这一层**（唯一一处）解析成 [`RgbColor`]，
//!    非法 / 缺失回退到 [`DEFAULT_TRACK_COLOR`]；规范化的 `#RRGGBB` 文本进 `.slint`
//!    与语义注册表，于是"投影 ↔ 控件树"两侧可以对账。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

// 账本第 314-318 轮：夹具家族（demo_*）下移到 `yeban-model::samples`；这里再导出, 调用方不变。
pub use yeban_model::samples::{demo_id, demo_project};

// 账本第 261 轮：这两个已**下移**到 `yeban-model`（MCP 与 UI 共用同一实现, 且不违反依赖方向）。
use yeban_model::music::MidiNote;
pub use yeban_model::note_plan::{NotePlan, plan_to_add_note};
use yeban_model::project::{
    ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition, DeviceKind, LoopConfig,
    ParameterValue, RoutingEdge, RoutingKind, SceneV3, SectionV3, TimeSignature, TrackKind,
    TrackV3, YebanProjectV1,
};
use yeban_model::{EntityId, Op, SamplerBinding};

/// 960 PPQ 整数时钟（`[MODEL-AST-001]`）—— 从模型层再导出，避免两处各写一个字面量。
pub use yeban_model::PPQ;

/// 默认缩放：**每 30 tick 一个逻辑像素**。
///
/// 为什么是 30 而不是 32：
/// - 30 让 4/4 的一小节（3840 tick）落在 128px，与 UI/UX 规范 §4.2 的标尺密度同量级；
/// - 30 **不是** 2 的幂 ⇒ `1.0 / 30.0` 在二进制浮点里不精确。因此只要有人把
///   [`tick_to_px`] 改成浮点实现，`tick_to_px(30, 30)` 就会算出 0 而不是 1，
///   往返判据立即变红。这个数字本身就是"不许用浮点算位置"的**活判据**（不是巧合）。
pub const DEFAULT_TICKS_PER_PIXEL: u64 = 30;

/// 横向缩放的**下界**（`ticks_per_pixel` 的最小合法值）：1 tick / 逻辑像素 = 最大放大。
///
/// 数字**不是新发明的**：判据 `tick_to_pixel_round_trips_in_both_directions`
/// （`bridge.rs` 的 `#[cfg(test)]` 模块）已经在 `ticks_per_pixel = 1` 上把两个方向
/// 的往返逐项断言过，极端值 `960`（= 最小放大，一个四分音符 1px）同属那一族。
/// 因此夹紧区间落在**投影已被判据覆盖**的范围之内，不引入没测过的档位。
pub const MIN_TICKS_PER_PIXEL: u64 = 1;

/// 横向缩放的**上界**（`ticks_per_pixel` 的最大合法值）：960 tick / 逻辑像素。
///
/// 960 = [`PPQ`] ⇒ 一个四分音符恰好 1 逻辑像素（最小放大）。
/// 出处同 [`MIN_TICKS_PER_PIXEL`]：同一个往返判据覆盖的档位之一。
pub const MAX_TICKS_PER_PIXEL: u64 = 960;

/// 把任意 `ticks_per_pixel` 夹进 [`MIN_TICKS_PER_PIXEL`]..=[`MAX_TICKS_PER_PIXEL`]。
///
/// **这是缩放的唯一夹紧点**（`ADR-0004` Q2 对"有意义的上下界放投影（纯函数）"的同一条取向）。
/// `0` 也走这里：它被夹成 `1`（最小 = 1，所以"0"没有第二条解释）。
/// 投影本体仍然对 `0` 报 [`BridgeError::ZeroTicksPerPixel`] —— 那个错误码与它的语义不变，
/// 本函数只是保证**宿主算出来的**值永远不落在那里。
#[must_use]
pub fn clamp_ticks_per_pixel(ticks_per_pixel: u64) -> u64 {
    ticks_per_pixel.clamp(MIN_TICKS_PER_PIXEL, MAX_TICKS_PER_PIXEL)
}

/// 把"要撑满视口的 tick 跨度"换算成夹紧后的 `ticks_per_pixel`（**纯函数**，整数口径）。
///
/// 口径（"撑满"= 跨度**装得下**且尽可能大）：
///
/// ```text
/// target = clamp( ceil(span_ticks / viewport_px) )
/// ```
///
/// - **向上取整**而不是向下：`tick_to_px` 是整数除法，向下取整会让画出的宽度**超过**视口；
///   向上取整保证画出的跨度 `<= viewport_px`（"装得下"），同时取到最小的合法 `ticks_per_pixel`
///   （= 尽量放大）。
/// - 返回 `None` 的两种情形都是**决定**：跨度为 0（选区里没有音符 / 工程没有任何内容）
///   与视口宽度为 0（没有可撑满的画布）。调用方据此**不消费**这一键 ——
///   与 `Action::DeleteSelection`（空选区返回 `false`）、`Action::Cancel`（没有手势返回
///   `false`）同一取向：一个没有可作用对象的键不该被吞掉。
/// - 夹紧的后果**如实登记**：若 `ceil(span / px) > ` [`MAX_TICKS_PER_PIXEL`]，
///   跨度在最小放大下仍然装不下 —— 那时本函数给出上界值，而不是假装装得下。
#[must_use]
pub fn ticks_per_pixel_to_fit(span_ticks: u64, viewport_px: u32) -> Option<u64> {
    if span_ticks == 0 || viewport_px == 0 {
        return None;
    }
    Some(clamp_ticks_per_pixel(
        span_ticks.div_ceil(u64::from(viewport_px)),
    ))
}

/// 一个剪辑 / 段落在时间轴上的最小可见宽度（逻辑像素）。
///
/// 亚像素宽的块会被 Slint 的裁剪语义过滤掉（`i-slint-core` 的
/// `absolute_clip_rect_and_geometry`），从而让"语义 ID 存在但运行时树里查不到" ——
/// 那会把 `[UI-TEST-001]` 的寻址变成概率事件。因此投影给一个**显式地板**，
/// 并在判据里如实断言（`width == max(1, …)`），而不是靠夹具的时值足够大来回避。
pub const MIN_BLOCK_WIDTH_PX: f32 = 1.0;

/// 标尺至少画出这么多小节线（规范 §4.2 的最小可视密度；工程更长时按需增加）。
pub const MIN_BAR_COUNT: usize = 16;

// ---------------------------------------------------------------------------
// 卷帘音高映射（**整数**，[UI-NOTE-002] 坐标双向映射的投影侧一半）
// ---------------------------------------------------------------------------

/// 卷帘可见的音高车道数。
///
/// 它必须与 `ui/console/piano_roll.slint` 里画出的车道数一致 —— 投影只给出
/// **车道索引**，车道→像素的乘法与 clips 的 `lane` 同一形态。两者的对账由
/// `pitch_lane_geometry_matches_the_slint_grid` 这条文本层判据钉住（本机不编译 Slint，
/// 所以这条耦合只能以"读 `.slint` 原文"的形式断言，见 `docs/ledger/app-completion-notes.md` §4）。
pub const PITCH_LANE_COUNT: i32 = 16;

/// 车道窗口下界的 MIDI 音高：`C4 = 60`。
///
/// 落在窗口之外的音高**钳制**进窗口（不 panic、不绕回）：真正的视口滚动 /
/// 裁剪属于 `[UI-NOTE-001]`，仍未实现，见 `docs/ledger/app-completion-notes.md` 的未实现项。
pub const PITCH_LANE_BASE: u8 = 60;

/// 一条车道在逻辑像素里的高度（与 `piano_roll.slint` 的 `14px * lane_index` 对齐）。
pub const PITCH_LANE_HEIGHT_PX: f32 = 14.0;

/// 音符块在车道内的纵向内缩（与 `piano_roll.slint` 的 `+ 6px` 对齐）。
pub const NOTE_INSET_Y_PX: f32 = 6.0;

/// 卷帘左侧琴键列的宽度（逻辑像素）—— 与 `ui/console/piano_roll.slint` 里琴键列的
/// `width: 56px` 及网格画布的 `x: 56px` 同一读数。
pub const ROLL_KEY_COLUMN_WIDTH_PX: f32 = 56.0;

/// 音符层在网格画布内的**横向**内缩（逻辑像素）—— 与 `piano_roll.slint` 音符的
/// `x: Tokens.space-5 + …`（`Tokens.space-5` = 12px）同一读数。
pub const NOTE_INSET_X_PX: f32 = 12.0;

/// 卷帘表头（工具行）高度（逻辑像素）—— 与 `piano_roll.slint` 里琴键列 / 网格画布的
/// `y: 28px` 同一读数。
pub const ROLL_HEADER_HEIGHT_PX: f32 = 28.0;

/// 卷帘**绘制坐标 → 宿主坐标**的**唯一**换算点。
///
/// 界面回调报上来的 `(x, y)` 是**相对 `PianoRoll` 根**的绘制坐标（`piano_roll.slint` 里
/// 覆盖全组件的拖动手势面报 `self.mouse-x` / `self.mouse-y`）：音符层画在琴键列
/// （[`ROLL_KEY_COLUMN_WIDTH_PX`]）之后、再内缩 [`NOTE_INSET_X_PX`]，并且整体在
/// [`ROLL_HEADER_HEIGHT_PX`] 的表头之下。而 `hit_test_visible` / `pencil_op_for` /
/// `lane_at_y` 用的是**网格画布局部**坐标（tick 0 = x 0，泳道顶 = y 0）。两者只差一个固定原点。
///
/// 本函数是这条原点换算的**单一真相源**：宿主侧只调用它一次（`host::install_roll_click`），
/// 判据侧注入指针时调用它的反向 [`host_to_painted`] —— ⛔ 任何调用点都不得自己再写一次
/// `- 56 - 12` / `- 28`（那正是本票的缺陷：宿主从不减这个偏移 ⇒ 人手点得到的音符矩形与
/// 命中测试差 `(68, 28)`，纵向 28px 超过音符高 12px ⇒ 点可见音符恒不中）。
#[must_use]
pub fn painted_to_host(x: f32, y: f32) -> (f32, f32) {
    (
        x - ROLL_KEY_COLUMN_WIDTH_PX - NOTE_INSET_X_PX,
        y - ROLL_HEADER_HEIGHT_PX,
    )
}

/// [`painted_to_host`] 的反向：宿主坐标 → 绘制坐标。
///
/// 判据要用**人看得见**的矩形坐标注入指针，就必须是它的反向；同一份常量、同一条换算，
/// 免得判据自己写 `+ 68` / `+ 28` 造出第三个数。
#[must_use]
pub fn host_to_painted(x: f32, y: f32) -> (f32, f32) {
    (
        x + ROLL_KEY_COLUMN_WIDTH_PX + NOTE_INSET_X_PX,
        y + ROLL_HEADER_HEIGHT_PX,
    )
}

/// 音高 → 车道索引：**整数**、钳制、上界 `0`（高音在上）。
///
/// `pitch < PITCH_LANE_BASE` 落到最下面一条车道（索引 `PITCH_LANE_COUNT - 1`），
/// `pitch >= PITCH_LANE_BASE + PITCH_LANE_COUNT` 落到最上面一条（索引 `0`）。
/// 单调性：`pitch` 越大 ⇒ 车道索引越小（屏幕上越靠上），在窗口内**严格**递减。
#[must_use]
pub fn pitch_lane(pitch: u8) -> i32 {
    let offset = i32::from(pitch.saturating_sub(PITCH_LANE_BASE)).min(PITCH_LANE_COUNT - 1);
    PITCH_LANE_COUNT - 1 - offset
}

/// `[UI-NOTE-002/003]` **泳道 → 音高**（[`pitch_lane`] 的反向）。
///
/// 铅笔要"在某个位置画出音符"就得先回答"这一行是哪个音高"。由 `pitch_lane` 的定义反推：
/// `offset = PITCH_LANE_COUNT - 1 - lane`，`pitch = PITCH_LANE_BASE + offset`。
/// 最下面一条泳道对应 `PITCH_LANE_BASE + 15`；比它更低的音高**画不出来**（车道只有 16 条）——
/// 这不是缺陷而是当前截图的事实, 判据里以"往返"为准。
#[must_use]
pub fn pitch_for_lane(lane: i32) -> Option<u8> {
    // 用 `Range::contains` 而不是手写比较 —— clippy 的 `manual_range_contains` 正是这么要求的,
    // 而我上一轮把门禁与提交串在一条命令里 ⇒ 它红着就推出去了（本会话第 172 轮记过这条规矩）。
    if !(0..PITCH_LANE_COUNT).contains(&lane) {
        return None;
    }
    let offset = PITCH_LANE_COUNT - 1 - lane;
    let base = i32::from(PITCH_LANE_BASE);
    u8::try_from(base + offset).ok()
}

/// `[UI-NOTE-002/003]` **y → 泳道**：`pitch_lane_y` 的反向, 供铅笔/力度工具把点击落到某一行。
///
/// y 是相对卷帘顶边的逻辑像素；落在 16 条泳道之外（含负数）返回 `None`。
#[must_use]
pub fn lane_at_y(y: f32) -> Option<i32> {
    if !y.is_finite() {
        return None;
    }
    let lane = ((y - NOTE_INSET_Y_PX) / PITCH_LANE_HEIGHT_PX).floor();
    if lane < 0.0 || lane >= PITCH_LANE_COUNT as f32 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation)]
    Some(lane as i32)
}

/// 音高 → 音符块顶边的相对 y（逻辑像素）。整数车道索引经**一次**乘法得到，不做累加。
#[must_use]
pub fn pitch_lane_y(pitch: u8) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let lane = pitch_lane(pitch) as f32;
    PITCH_LANE_HEIGHT_PX * lane + NOTE_INSET_Y_PX
}

// ---------------------------------------------------------------------------
// 界面色标（`TrackV3::color` 的消费方式：**Rust 侧解析**，解析失败必须可判据化）
// ---------------------------------------------------------------------------

/// 没有色标（`TrackV3::color == None`）或色标非法时的回退色。
///
/// 取值就是 `ui/tokens.slint` 的 `Tokens.line-strong` 的 **brand 分支**（`#2c3a63`）
/// —— 一个中性石板色：它在深色面板上可见、又不会被误认成"某个轨道品牌色"。代价是
/// `.slint` 与 Rust 各写一份十六进制值，由判据
/// `missing_or_illegal_colors_fall_back_to_the_documented_value` 与
/// `token_drift_of_the_fallback_color_is_detected` 两侧对账（后者直接读 `tokens.slint` 原文）。
/// 注意它绑的是**品牌分支**：2026-10-07 起默认主题是 yeban，`line-strong` 在默认主题下
/// 取 #2c3948，与这个回退色不同 —— 这条耦合守的是"回退色 = 品牌分支"，不是"回退色 = 默认外观"。
pub const DEFAULT_TRACK_COLOR_HEX: &str = "#2C3A63";

/// 一个已经解析成功（或已回退）的 RGB 色标。
///
/// 存在的意义：`.slint` 的 `[color]` 数组需要 `Color`，而投影层**零 Slint 依赖**，
/// 所以投影只交出不透明 `u8` 三元组，由唯一的注入点 [`crate::host`] 转成 `slint::Color`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbColor {
    /// 红通道。
    pub red: u8,
    /// 绿通道。
    pub green: u8,
    /// 蓝通道。
    pub blue: u8,
}

impl RgbColor {
    /// 构造一个不透明色。
    #[must_use]
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    /// 规范化的 `#RRGGBB` 文本（大写十六进制）—— 进判据、进控件树标签、进 `.slint`。
    ///
    /// 之所以要**规范化**：模型里同一个颜色可能写成 `#f7e6b0` 或 `#F7E6B0`，
    /// 而"控件树标签 == 投影字段"这条判据需要一个唯一的文本形态。
    #[must_use]
    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.red, self.green, self.blue)
    }
}

/// 解析 `TrackV3::color` 的十六进制色标。
///
/// ## 语法（**规范缺口** —— 模型只写"界面色标"，没有规定格式）
///
/// 接受：可选的 `#` + 3 或 6 个 ASCII 十六进制字符（大小写不敏感）。
/// 3 位短写按 CSS 规则展开（`#abc` → `#aabbcc`）。
/// 拒绝：空串、含空白、长度不是 3/6、含非十六进制字符、`#RRGGBBAA`、`rgb(...)` 等一切其它形态
/// （返回 `None`，由调用方回退到 [`DEFAULT_TRACK_COLOR_HEX`]）。
///
/// 这条缺口已登记为 needs（建议提升为 ADR 级裁决）；在裁决下来之前，**拒绝的比接受的宽**
/// 是本实现的取向：宁可回退到中性色，也不猜一个可能画错的颜色。
#[must_use]
pub fn parse_hex_color(raw: &str) -> Option<RgbColor> {
    let digits = raw.strip_prefix('#').unwrap_or(raw);
    if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let nibble = |byte: u8| -> u8 {
        match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            _ => byte - b'A' + 10,
        }
    };
    let bytes = digits.as_bytes();
    match bytes.len() {
        3 => {
            // `#abc` → `#aabbcc`：每一位复制成两位（CSS Color 3 的短写规则）。
            let value = |index: usize| -> u8 {
                let high = nibble(bytes[index]);
                high * 16 + high
            };
            Some(RgbColor::new(value(0), value(1), value(2)))
        }
        6 => {
            let pair = |index: usize| -> u8 {
                nibble(bytes[index * 2]) * 16 + nibble(bytes[index * 2 + 1])
            };
            Some(RgbColor::new(pair(0), pair(1), pair(2)))
        }
        _ => None,
    }
}

/// [`DEFAULT_TRACK_COLOR_HEX`] 的 `u8` 三元组形态（两者由判据逐字对账，不靠人眼）。
pub const DEFAULT_TRACK_COLOR: RgbColor = RgbColor::new(0x2c, 0x3a, 0x63);

/// 解析色标，失败即回退到 [`DEFAULT_TRACK_COLOR`]。
///
/// 这是 `.slint` 与控件树**唯一**的色标事实源（不存在"另一处再解析一遍"）。
#[must_use]
pub fn track_color_or_default(raw: Option<&str>) -> RgbColor {
    raw.and_then(parse_hex_color).unwrap_or(DEFAULT_TRACK_COLOR)
}

/// 投影过程中可恢复的输入问题。
///
/// 刻意**不**用 `panic!`：工程文档来自磁盘 / 归档，属不可信输入
/// （与 `yeban-model` 的 `validate()` 同一条纪律）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeError {
    /// `ticks_per_pixel == 0`：除零，且没有任何合法解释。
    ZeroTicksPerPixel,
    /// 起始 tick + `duration_ticks` 越过 `u64::MAX`（剪辑摆放与 MIDI 音符共用这一条）。
    TickOverflow {
        /// 溢出发生的起始 tick。
        start_tick: u64,
    },
    /// tick 换算出的像素数超出 `u32`。
    PixelOverflow {
        /// 越界的 tick。
        tick: u64,
    },
    /// 像素换算回 tick 时越过 `u64::MAX`。
    TickBackOverflow {
        /// 越界的像素。
        px: u32,
    },
    /// 拍号的分母为 0（小节长度无法定义）。
    ZeroTimeSignatureDenominator,
    /// 小节长度计算溢出。
    BarLengthOverflow,
}

impl fmt::Display for BridgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroTicksPerPixel => formatter.write_str("ticks_per_pixel 不能为 0"),
            Self::TickOverflow { start_tick } => write!(
                formatter,
                "起始 tick {start_tick} 与 duration_ticks 相加越过 u64::MAX"
            ),
            Self::PixelOverflow { tick } => {
                write!(formatter, "tick {tick} 换算出的像素数超出 u32 范围")
            }
            Self::TickBackOverflow { px } => {
                write!(formatter, "像素 {px} 换算回 tick 时越过 u64::MAX")
            }
            Self::ZeroTimeSignatureDenominator => formatter.write_str("拍号分母不能为 0"),
            Self::BarLengthOverflow => formatter.write_str("小节长度计算溢出"),
        }
    }
}

impl std::error::Error for BridgeError {}

/// `tick → 逻辑像素`：**整数**除法（向下取整）。
///
/// # Errors
///
/// `ticks_per_pixel == 0` → [`BridgeError::ZeroTicksPerPixel`]；
/// 商超出 `u32` → [`BridgeError::PixelOverflow`]。
pub fn tick_to_px(tick: u64, ticks_per_pixel: u64) -> Result<u32, BridgeError> {
    if ticks_per_pixel == 0 {
        return Err(BridgeError::ZeroTicksPerPixel);
    }
    u32::try_from(tick / ticks_per_pixel).map_err(|_| BridgeError::PixelOverflow { tick })
}

/// `[UI-NOTE-003]`（第 234 轮的决定）**点击 tick 落在哪个片段**。
///
/// 只接收**摆放切片**（`ClipPlacement`），因此不假定摆放住在工程的哪一层 —— 调用方传 `track.clips` 的值即可。
/// 区间取**半开** `[start_tick, start_tick + duration_ticks)`：正好落在末尾**不算**在内（与"时长"的直觉一致）。
/// 多个摆放重叠时返回**迭代顺序里最后一个**（重叠不是常规情形, 但必须有确定行为而不是随机的）。
/// 没有任何片段包含它 ⇒ `None`，调用方据此**拒绝编辑**而不是凭空造一个片段。
#[must_use]
pub fn clip_at_tick(
    placements: &[(u64, u64, yeban_model::ids::EntityId)],
    tick: u64,
) -> Option<yeban_model::ids::EntityId> {
    let mut hit = None;
    for (start_tick, duration_ticks, clip_id) in placements {
        let end = start_tick.saturating_add(*duration_ticks);
        if tick >= *start_tick && tick < end {
            hit = Some(*clip_id);
        }
    }
    hit
}

/// `[UI-NOTE-003]` 力度车道容器的高度（`.slint` 的 `y: parent.height - 44px; height: 44px;`）。
///
/// 与 [`PITCH_LANE_HEIGHT_PX`] 一样, 这是"界面画多少 / 宿主算多少"的**两处真相**候选 ⇒
/// 值集中在这里, 并由 `slint_text_contracts_for_colors_and_pitch_lanes` 对账（改任一处都变红）。
pub const VELOCITY_LANE_HEIGHT_PX: f32 = 44.0;

/// `[UI-NOTE-003]` 满力度（127）时的力度柱高（`.slint` 的 `28px * velocity` 里的 28）。
pub const VELOCITY_BAR_MAX_HEIGHT_PX: f32 = 28.0;

/// `[UI-NOTE-003]` 力度车道底沿留白（`.slint` 的 `parent.height - 4px - …` 里的 4）。
///
/// 口径：**力度 0** 的柱底沿就在 `车道高 - 4`，力度 127 的柱顶沿在 `车道高 - 4 - 28`
/// （见 [`VelocityLaneGeometry::velocity_at_lane_y`]）。
pub const VELOCITY_LANE_BOTTOM_MARGIN_PX: f32 = 4.0;

/// `[UI-NOTE-003]` 力度柱宽（`.slint` 的 `width: 6px`）。
pub const VELOCITY_BAR_WIDTH_PX: f32 = 6.0;

/// `[UI-NOTE-003]` 力度柱左沿相对"琴键列 + 内缩"的偏移（`.slint` 的 `+ 30px`）。
///
/// ⚠ 这个 `30` 此前**两处都没有名字**（`bridge.rs` 的自判据里是手抄值，`.slint` 里是字面量）；
/// 本切片给它一个名字，宿主侧因此不再需要手抄。
pub const VELOCITY_BAR_OFFSET_X_PX: f32 = 30.0;

/// `[UI-NOTE-003]` 力度车道的**几何**（界面侧的值集中在这里, 免得命中测试去猜）。
///
/// 数值来自 `piano_roll.slint` 的柱体：`x: 56px + Tokens.space-5 + note-positions[i] + 30px`、
/// `width: 6px`、`y: parent.height - 4px - 28px * velocity`、`height: 28px * velocity`。
/// 与 `NOTE_HEIGHT_PX` 一样, 这是"两处真相"的候选 ⇒ 改任一处都应由守卫钉住（账本第 196/487/501 轮）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VelocityLaneGeometry {
    /// 车道容器高度（`.slint` 的 `parent.height`，实测 44px）。
    pub lane_height: f32,
    /// 柱体左沿相对车道左沿的偏移（56 + `Tokens.space-5` + 30）。
    pub offset_x: f32,
    /// 柱体宽（6px）。
    pub bar_width: f32,
    /// 满力度时的柱高（28px）。
    pub bar_max_height: f32,
}

impl VelocityLaneGeometry {
    /// `.slint` 的**唯一**一份力度车道几何（生产唯一构造点；⛔ 调用方不得各写一份字面量）。
    #[must_use]
    pub const fn roll_velocity_lane() -> Self {
        Self {
            lane_height: VELOCITY_LANE_HEIGHT_PX,
            offset_x: ROLL_KEY_COLUMN_WIDTH_PX + NOTE_INSET_X_PX + VELOCITY_BAR_OFFSET_X_PX,
            bar_width: VELOCITY_BAR_WIDTH_PX,
            bar_max_height: VELOCITY_BAR_MAX_HEIGHT_PX,
        }
    }

    /// `[UI-NOTE-003]` **车道内 y → 绝对力度**（`0..=127`）—— `.slint` 画柱的那个映射的反函数。
    ///
    /// 口径（三条都是决定）：
    /// - **绝对**：指针的 y 决定力度本身，⛔ 不是"相对起点的增量"（那正是 `[UI-NOTE-003]`
    ///   力度臂的子裁决 ③：力度车道就是这个意思）；
    /// - 柱底沿（`lane_height - bottom_margin`）= 0，往上 `bar_max_height` 像素 = 127；
    ///   区间**外**一律**夹紧**（拖到车道上方 ⇒ 127，下方 ⇒ 0）；
    /// - 非有限 y ⇒ `None`（调用方**什么都不做** —— 与 `note_move_delta_for` 对非有限输入的取向相同，
    ///   ⛔ 不把 NaN 静默当成 0）。
    #[must_use]
    pub fn velocity_at_lane_y(self, lane_y: f32) -> Option<u8> {
        if !lane_y.is_finite() {
            return None;
        }
        let bottom = self.lane_height - VELOCITY_LANE_BOTTOM_MARGIN_PX;
        let fraction = ((bottom - lane_y) / self.bar_max_height).clamp(0.0, 1.0);
        // 127 是 `yeban_model::music::MIDI_VELOCITY_MAX`（模型层唯一真相源）。
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let velocity = (fraction * f32::from(yeban_model::music::MIDI_VELOCITY_MAX)).round() as u8;
        Some(velocity.min(yeban_model::music::MIDI_VELOCITY_MAX))
    }
}

impl ViewState {
    /// `[UI-NOTE-003]` **铅笔的决策**：在视口坐标 `(x, y)` 处画音符, 应当落在哪个 **(tick, pitch)**。
    ///
    /// 只做**决策**（纯函数, 可判据）：真正的模型突变要经过撤销与 MCP, 属于下一层。
    /// `None` 表示"这里画不了"（y 落在 16 条泳道之外, 或 x 出界）。tick 已按 `grid_ticks` 吸附。
    #[must_use]
    pub fn pencil_plan(&self, scroll_x: f32, x: f32, y: f32, grid_ticks: u64) -> Option<NotePlan> {
        let lane = lane_at_y(y)?;
        let pitch = pitch_for_lane(lane)?;
        let start_tick = self.snapped_tick_at(scroll_x, x, grid_ticks);
        Some(NotePlan {
            start_tick,
            pitch,
            // 规范 `[UI-NOTE-003]`：双击创建"**默认 1 拍**"音符 ⇒ 时值 = 工程的 `ppq`（不是硬编码 960）。
            duration_ticks: self.ppq,
        })
    }

    /// `[UI-NOTE-003]` **力度柱命中**：车道内的点落在哪个音符的力度柱上。
    ///
    /// 力度工具的左键单击是"选中对应音符的底部力度柱" ⇒ 先要知道点到的是哪根柱。
    /// 与 [`Self::hit_test_visible`] 同一取舍: 重叠时返回**最后绘制**（下标最大）的那个。
    /// `geometry` 由调用方给出（值取自 `.slint`）, 见 [`VelocityLaneGeometry`]。
    #[must_use]
    pub fn velocity_bar_hit_test(
        &self,
        scroll_x: f32,
        viewport_width: f32,
        x: f32,
        y: f32,
        geometry: VelocityLaneGeometry,
    ) -> Option<usize> {
        let mut hit: Option<usize> = None;
        // 力度车道与音符同宽 ⇒ 用**同一**可见窗口（而不是整条时间轴）。
        for index in self.notes_visible_in(scroll_x, viewport_width) {
            let note = &self.notes[index];
            let left = geometry.offset_x + note.x - scroll_x.max(0.0);
            let top =
                geometry.lane_height - 4.0 - geometry.bar_max_height * note.velocity_normalized;
            let height = geometry.bar_max_height * note.velocity_normalized;
            if x >= left && x <= left + geometry.bar_width && y >= top && y <= top + height {
                hit = Some(index);
            }
        }
        hit
    }

    /// `[UI-NOTE-002/003]` **视口坐标 → 吸附后的 tick**：编辑工具问"这一点落在哪个网格"的答案。
    ///
    /// 铅笔（"在吸附网格处画出音符"）、剪刀（"沿网格竖线切分"）与选择工具的移动都以此为前提。
    /// 口径：先用**投影自己的** `ticks_per_pixel` 把视口 x 换算成 tick（加上滚动偏移），再吸附；
    /// 换算出界（非有限、超出 `u32`）时返回 `0` —— 与"视口外"同义, 而不是 panic。
    #[must_use]
    pub fn snapped_tick_at(&self, scroll_x: f32, x: f32, grid_ticks: u64) -> u64 {
        let px = scroll_x.max(0.0) + x.max(0.0);
        if !px.is_finite() || px > u32::MAX as f32 {
            return 0;
        }
        let tick = px_to_tick(px as u32, self.ticks_per_pixel).unwrap_or(0);
        snap_tick(tick, grid_ticks)
    }
}

/// `[UI-NOTE-002/003]` 把 tick **吸附**到网格：四舍五入到最近的 `grid_ticks` 倍数。
///
/// 规范要求"960 PPQ 吸附对齐"，且铅笔（"在吸附网格处画出音符"）与剪刀（"沿网格竖线切分"）都以它为前提。
///
/// 口径（刻意写明, 因为"四舍五入"在平局处有两种合理读法）：`grid_ticks` 为偶数时存在**正中间**的情况,
/// 本函数**向更晚的时间（向上）**取整。`grid_ticks == 0` 表示**不吸附**，原样返回（且不除零）。
/// 结果始终是 `grid_ticks` 的倍数, 且与输入的差不超过半个网格 —— 这两条就是判据里的定义性断言。
#[must_use]
pub fn snap_tick(tick: u64, grid_ticks: u64) -> u64 {
    if grid_ticks == 0 {
        return tick;
    }
    let remainder = tick % grid_ticks;
    if remainder == 0 {
        return tick;
    }
    let down = tick - remainder;
    // 判据写成 `rem >= grid - rem`（等价于 `2*rem >= grid`）而**不是** `rem > grid/2`：
    // ① `grid/2` 在平局时让"向上"落空 —— 实测 `snap_tick(480, 960)` 得 0, 与本函数文档不符（判据抓到的真错）;
    // ② 用减法而不是 `rem * 2` 可避免 `grid` 很大时的乘法溢出。
    // 奇数网格下 `rem == grid - rem` 不可能成立, 所以"更近的一侧"自动正确。
    if remainder >= grid_ticks - remainder {
        down.saturating_add(grid_ticks)
    } else {
        down
    }
}

/// `逻辑像素 → tick`：整数乘法，**用 `checked_mul`**。
///
/// # Errors
///
/// `ticks_per_pixel == 0` → [`BridgeError::ZeroTicksPerPixel`]；
/// 乘积越过 `u64::MAX` → [`BridgeError::TickBackOverflow`]。
pub fn px_to_tick(px: u32, ticks_per_pixel: u64) -> Result<u64, BridgeError> {
    if ticks_per_pixel == 0 {
        return Err(BridgeError::ZeroTicksPerPixel);
    }
    u64::from(px)
        .checked_mul(ticks_per_pixel)
        .ok_or(BridgeError::TickBackOverflow { px })
}

/// `[MODEL-AST-001]` 一小节有多少 tick（960 PPQ × 分子 × 4 ÷ 分母）。
///
/// # Errors
///
/// 分母为 0、或乘法溢出 → [`BridgeError`]。
pub fn bar_length_ticks(time_signature: TimeSignature) -> Result<u64, BridgeError> {
    if time_signature.denominator == 0 {
        return Err(BridgeError::ZeroTimeSignatureDenominator);
    }
    PPQ.checked_mul(u64::from(time_signature.numerator))
        .and_then(|value| value.checked_mul(4))
        .map(|value| value / u64::from(time_signature.denominator))
        .ok_or(BridgeError::BarLengthOverflow)
}

/// 时间码的**格式化网格**（拍号 → 每拍 / 每小节的 tick 数）。
///
/// `[MODEL-ISO-001]` 把走带位置划给**会话运行态**（`SessionRuntimeState`），
/// 因此 `tick → 小节.拍.tick` 是"会话读数 × 工程拍号"的乘积：位置来自引擎，
/// 拍号来自工程，**换算只有这一处**（本模块零 Slint ⇒ 判据能在本机真跑）。
///
/// 拍号 → 每小节 tick 的算术**不再重写一遍**：直接调用模型层的唯一权威
/// [`yeban_model::SessionRuntimeState::ticks_per_bar`]（`[MODEL-AST-001]`）。
/// 模型拒绝的拍号（分子/分母为 0，或分母不整除全音符）在这里同样是 `None` ——
/// 于是"引擎给了 tick、但没有合法的拍号网格"这种情况会**如实**退回 tick 文本
/// （[`timecode_for_ticks`]），而不是按 4/4 编一个看起来正常的假读数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimecodeGrid {
    /// 一拍（一个分母音符）的 tick 数。
    ticks_per_beat: u64,
    /// 一小节的 tick 数（`ticks_per_beat × 分子`，**精确**，见 [`Self::from_time_signature`]）。
    ticks_per_bar: u64,
}

impl TimecodeGrid {
    /// 从拍号（分子 / 分母）推出网格；模型层拒绝该拍号时返回 `None`。
    ///
    /// `SessionRuntimeState::ticks_per_bar` 已经校验过 `(4 × PPQ) % 分母 == 0`，
    /// 因此 `ticks_per_bar / 分子` **整除**（一拍就是一个分母音符）：不取整、
    /// 不做浮点，拍与拍之间等宽。
    #[must_use]
    pub fn from_time_signature(numerator: u8, denominator: u8) -> Option<Self> {
        let ticks_per_bar =
            yeban_model::SessionRuntimeState::ticks_per_bar(numerator, denominator)?;
        // `numerator != 0` 由上面的 `Some` 保证（模型对 0 分子返回 `None`）。
        let ticks_per_beat = ticks_per_bar.checked_div(u64::from(numerator))?;
        if ticks_per_beat == 0 {
            return None;
        }
        Some(Self {
            ticks_per_beat,
            ticks_per_bar,
        })
    }

    /// 从**已经投影出来的两个整数**重建网格。
    ///
    /// 存在的理由：注入面（Slint 的 `int` 属性）只搬整数，而唯一的格式化实现要的是
    /// [`Self`]。畸形输入（0 / 负数 / 一拍比一小节还长）一律 `None` ⇒ 退回 tick 文本，
    /// 不 panic、不 wrap（`[ARCH-UI-002]`：UI 侧不做算术，只搬运投影算好的数）。
    #[must_use]
    pub fn from_injected(ticks_per_beat: i32, ticks_per_bar: i32) -> Option<Self> {
        let ticks_per_beat = u64::try_from(ticks_per_beat).ok()?;
        let ticks_per_bar = u64::try_from(ticks_per_bar).ok()?;
        if ticks_per_beat == 0 || ticks_per_bar < ticks_per_beat {
            return None;
        }
        Some(Self {
            ticks_per_beat,
            ticks_per_bar,
        })
    }

    /// 一拍的 tick 数。
    #[must_use]
    pub const fn ticks_per_beat(self) -> u64 {
        self.ticks_per_beat
    }

    /// 一小节的 tick 数。
    #[must_use]
    pub const fn ticks_per_bar(self) -> u64 {
        self.ticks_per_bar
    }

    /// tick → 时间码文本（`BBB.BB.TTT`：小节.**拍**.拍内 tick，全部 1 起 / 0 起，
    /// 与 `SESSION_TIMECODE` 的口径一致）。
    ///
    /// 纯整数除法（`[MODEL-AST-001]`）：`ticks_per_beat != 0` 与
    /// `ticks_per_bar >= ticks_per_beat` 由两个构造器保证，因此这里不可能除零，
    /// 也不需要 `Result`。
    #[must_use]
    pub fn timecode_at(self, ticks: u64) -> String {
        let bar = ticks / self.ticks_per_bar + 1;
        let in_bar = ticks % self.ticks_per_bar;
        let beat = in_bar / self.ticks_per_beat + 1;
        let in_beat = in_bar % self.ticks_per_beat;
        format!("{bar:03}.{beat:02}.{in_beat:03}")
    }
}

/// `[MODEL-AST-001]` tick → 时间码文本的**唯一实现**（网格来自投影，见 [`TimecodeGrid`]）。
///
/// `grid` 为 `None`（= 模型层拒绝这个拍号）时**如实**退回 tick 文本：宁可显示
/// `tick 8000`，也不按写死的 4/4 编一个"看起来对"的小节读数 —— 那正是本线修掉的
/// 缺陷（上一版 `src/host.rs` 把 `4` 刻在常量 `BEATS_PER_BAR` 里）。
#[must_use]
pub fn timecode_for_ticks(grid: Option<TimecodeGrid>, ticks: u64) -> String {
    match grid {
        Some(grid) => grid.timecode_at(ticks),
        None => format!("tick {ticks}"),
    }
}

/// `[ROAD-M3-002]` 裁剪后的**全部平行数组**（**同一索引集**）。
///
/// 平行数组各裁各的会让下标错位（位置取第 3 个、音高取第 5 个 ⇒ 画出错误的音符），
/// 所以一次取走一份，由本结构保证各数组长度相同。
#[derive(Debug, Clone, PartialEq)]
pub struct VisibleNotes {
    /// 音符块左沿 x。
    pub positions: Vec<f32>,
    /// 音符块宽。
    pub widths: Vec<f32>,
    /// 音符块顶沿 y。
    pub ys: Vec<f32>,
    /// 音高车道索引。
    ///
    /// ⚠ **追加修订段（任务单 `#127`；台账第 283 条）—— ⛔ 只针对 `rows` 这一支**：
    /// 上面这行与 [`VisibleNotes`] 的字段集合**不删不改**；被修订的是"六个字段同索引集
    /// ⇒ 六个一起注入宿主"这一支里的 **`rows` 注入面**。实测（`#127`）：宿主
    /// `apply_view` 从 `visible_notes` 注入的是 **5** 个数组（`ulids` / `velocities` /
    /// `positions` / `widths` / `ys`），第 6 条注入是 `note-selected`（由
    /// `flags_for_ids` 从选区现算，⛔ 不是 `rows`）；`rows` 的**宿主生产读点 = 0**
    /// （只有测试读点与访问器 `ViewState::note_rows` 读它）⇒ 按第 283 条**登记为
    /// 有意不消费**（⛔ 不是"待接线"、⛔ 不是静默降级）。
    ///
    /// **为什么不给它消费者**（代价读数，⛔ 不是判断）：`ys` 已经是同一事实的像素形态
    /// （`pitch_lane` 一次乘法），宿主与 `.slint` 的绘制/命中路径都只读 `ys`；要真消费
    /// `rows` 就得新增 `note-rows` 属性 ＋ `apply_view` 里多一个写入点 ⇒ 会动
    /// `tests/perf_draw_budget.rs` 的 `APPLY_VIEW_WRITE_SITE_BUDGET`（现 **81**）与
    /// `PER_FRAME_ARRAY_REWRITE_BUDGET`（现 **48**）⇒ ⛔ 本单禁改那两个读数。
    ///
    /// **判据**：`crates/yeban-app/tests/roll_injection_registration.rs` 从 `host.rs`
    /// 源码**派生**"注入面恰是那 5 个名字 ＋ `rows` 不在其中"（⛔ 不读判据自己所在的文件，
    /// 第 293 条）。
    pub rows: Vec<i32>,
    /// `note-{ulid}-rect` 的 `{ulid}` 段（语义元素 ID 的一半）。
    pub ulids: Vec<String>,
    /// 归一化力度（0.0..=1.0）。
    pub velocities: Vec<f32>,
}

impl VisibleNotes {
    /// 可见音符数（各数组长度相同，故用一个函数报告）。
    #[must_use]
    pub fn len(&self) -> usize {
        self.positions.len()
    }

    /// 是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }
}

/// 编排车道行的纵向几何（**投影算一次**，前缀和；所有纵向消费者读同一份）。
///
/// `ADR-0004` S0：行 `y` 与行高从 `.slint` 的 `42px + 56px * track_index` 搬进投影，
/// `.slint` 侧从此**零行算术** —— 判据
/// `automation::tests::lane_element_ids_match_the_slint_template` 把这条钉在文本上。
///
/// `y` 是**前缀和**（不是乘法）：`y(0) = TRACK_LANE_TOP_PX`、
/// `y(i+1) = y(i) + stride`。S0 里 `stride` 恒为
/// [`crate::automation::TRACK_LANE_HEIGHT_PX`]，因此前缀和与 `top + height × i`
/// **逐位相等**（两者都是同一批小整数，`f32` 在 2^24 以内精确表示）；但形状已经是
/// S1「每轨高度」需要的那个形状 —— 那时 `stride` 逐行不同，乘法形式不再成立。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowGeometry {
    /// 行顶沿 y（逻辑像素）。第 0 行 = [`crate::automation::TRACK_LANE_TOP_PX`]。
    pub y: f32,
    /// 行槽高（相邻两行顶沿之差）。默认布局下 = [`crate::automation::TRACK_LANE_HEIGHT_PX`]。
    pub stride: f32,
    /// 这一行的**整数**有效行高（逻辑像素）—— `RowGeometry` 的**整数真相**。
    ///
    /// `ADR-0004` S1（Q1-C / Q2-B）：有效高由 [`TrackHeightLayout::effective_px`] 在投影里
    /// 一次算出（整数），`stride` 只是它加宽成 `.slint` 需要的 `length` 的**渲染形态**。
    /// 判据 `effective_height_is_clamped_inside_the_named_bounds_and_stays_integer`
    /// 钉住 `stride == height_px as f32`（逐位），
    /// 所以两者不会各说各话。
    pub height_px: u32,
}

/// 包头 / 车道矩形相对行槽高的上下留白（逻辑像素）。
///
/// 取值来自旧 `.slint` 的两个字面量之差：`56px`（行槽高）− `54px`（矩形高）。
pub const TRACK_ROW_GAP_PX: f32 = 2.0;

/// 剪辑矩形相对车道行的上下内缩（逻辑像素）。
///
/// 取值来自旧 `.slint` 的 `+ 4px` 与 `46px`（`54px − 2×4px`）。
pub const CLIP_ROW_INSET_PX: f32 = 4.0;

/// 编排视图一条车道的**默认**行高（整数逻辑像素）—— `ADR-0004` Q2 点名的
/// `DEFAULT_TRACK_HEIGHT_PX = 56`。
///
/// 来源与"数字不变"的保证：S0 的行距常量 [`crate::automation::TRACK_LANE_HEIGHT_PX`]
/// 是 `56.0`，S1 把它**照抄**成整数默认值。默认布局（无每轨覆盖、乘子 100）下
/// 有效高 = `clamp(56 × 100 / 100)` = `56`，`as f32` 逐位等于 `56.0`
/// ⇒ 默认帧的几何一位不变（判据 `default_layout_geometry_is_bit_for_bit_the_s0_geometry`
/// 同时断言 `DEFAULT_TRACK_HEIGHT_PX as f32 == TRACK_LANE_HEIGHT_PX`）。
pub const DEFAULT_TRACK_HEIGHT_PX: u32 = 56;

/// 有效行高的**下界**（整数逻辑像素）。
///
/// `ADR-0004` Q2 点名了这个常量但**没有给数字**（规范/ADR 在这一点上是沉默的 ⇒
/// 本仓登记为工程选择）。取值理由（全部是**几何退化**约束，不是手感）：
///
/// - 剪辑矩形的可画高 = `stride − TRACK_ROW_GAP_PX − 2 × CLIP_ROW_INSET_PX`
///   = `stride − 10`；`stride ≤ 10` 时它是 0 或负 —— 而 0 面积/亚像素的元素会被
///   上游裁剪语义过滤掉，于是"语义 ID 登记了却查不到"（`AUTOMATION_BAND_INSET_PX`
///   的注释与 `app-binding-notes.md` §4.1 记录了同一类问题）；
/// - 自动化带高 = `(stride − 2 × AUTOMATION_BAND_INSET_PX) / 泳道数`；下界 16 让
///   `stride − 4 = 12`，即**12 条泳道以内**的轨道带高 ≥ 1px
///   （`app-automation-ui-notes.md:466` 登记了 ">52 条泳道时带高 <1px" 的既有风险；
///   那条风险由**泳道数**决定，任何行高下界都消不掉它，这里只是不让下界自己制造它）。
///
/// 11 是纯算术地板（`stride − 10 > 0`），16 取在其上并留出 6px 的剪辑矩形余量。
pub const MIN_TRACK_HEIGHT_PX: u32 = 16;

/// 有效行高的**上界**（整数逻辑像素）。
///
/// 同 [`MIN_TRACK_HEIGHT_PX`]：`ADR-0004` 点名常量、未给数字（登记为工程选择）。
/// 取值理由：320 约等于 1080 逻辑像素高窗口的三分之一 —— 一条车道再高就不再是
/// "车道"（一条轨就吃掉整个编排视图），也让前缀和在 `f32` 里远离 2^24 的精确表示边界。
pub const MAX_TRACK_HEIGHT_PX: u32 = 320;

/// 全局高度缩放级的**默认**百分比（`100` = 不缩放）。
pub const DEFAULT_TRACK_HEIGHT_PERCENT: u32 = 100;

/// 一次投影里**唯一**的有效行高计算（纯函数，本机可判据）—— `ADR-0004` Q2-B。
///
/// ```text
/// effective(base_px, percent) = clamp(base_px × percent / 100, MIN, MAX)
/// ```
///
/// 三条口径（各有判据，逐条可被注入打红）：
///
/// 1. **整数**：`base_px` / `percent` 都是 `u32`，中间量用 `saturating_mul`（不 panic、
///    不回绕），除法是整数除法（向下取整）。返回值是 `u32` ⇒ "结果是整数"是**类型**保证；
///    投影另外断言加宽成 `f32` 之后逐位等于该整数（`f32` 在 2^24 以内精确表示整数）。
/// 2. **乘子在基准之后**：先乘再除**再**夹紧。夹紧只发生一次、且在最后 ——
///    于是"每轨差异"（`base_px`）与"全局缩放"（`percent`）作用在**同一个**量上，
///    优先级写死在这一处，界面没有第二处（`ADR-0004` Q1-C 的代价条款）。
///    模型侧**不设**布局边界（`Q2-C` 已否决）：`from_project` 不调用 `validate()`，
///    边界放模型会留下"投影不设防"的第二条路径。
/// 3. **全函数**：任意 `u32` 输入都返回 `[MIN, MAX]` 内的值；`percent = 0`（或窗口属性
///    上的负数折算出的 0）表示"缩到最小"，**不是**"这一行消失" —— 行高 0 是损坏值，
///    不可能由这条公式产生。
///
/// 退化值 `0` 的处置见 [`TrackHeightLayout::set_track_px`]（视图侧拒绝）与
/// `ADR-0004` Q2 的"模型只拒绝退化值 0"（那一半要 `height_px` 进工程才成立，
/// 属 Q1 的负责人裁决，见 [`TrackHeightLayout`] 的文档）。
#[must_use]
pub fn effective_track_height_px(base_px: u32, percent: u32) -> u32 {
    (base_px.saturating_mul(percent) / 100).clamp(MIN_TRACK_HEIGHT_PX, MAX_TRACK_HEIGHT_PX)
}

impl RowGeometry {
    /// 包头 / 车道矩形的可画高（逻辑像素）= 行槽高 − 上下留白。
    #[must_use]
    pub fn drawn_height(self) -> f32 {
        self.stride - TRACK_ROW_GAP_PX
    }

    /// 剪辑矩形的顶沿 y（逻辑像素）= 行顶沿 + 上下内缩。
    #[must_use]
    pub fn clip_y(self) -> f32 {
        self.y + CLIP_ROW_INSET_PX
    }

    /// 剪辑矩形的可画高（逻辑像素）= 车道矩形高 − 上下内缩。
    #[must_use]
    pub fn clip_height(self) -> f32 {
        self.drawn_height() - 2.0 * CLIP_ROW_INSET_PX
    }
}

/// 轨道高度的**视图状态**（`ADR-0004` Q1-C 的"视图那一半" + Q4-A 第 2 层）。
///
/// ## 两个旋钮、一个量、一处公式
///
/// | 字段 | 是什么 | 住哪一层 | 默认 |
/// | :--- | :--- | :--- | :--- |
/// | `per_track_px` | **每轨基准行高**（整数逻辑像素） | 本片：**视图态**（`Q1` 若裁"进工程"则改为 `TrackV3::height_px`） | 无覆盖 ⇒ [`DEFAULT_TRACK_HEIGHT_PX`] |
/// | `percent` | **全局高度缩放级**（乘子，百分比） | 视图态（`Q4-A`） | [`DEFAULT_TRACK_HEIGHT_PERCENT`] = 100 |
///
/// 优先级写在**唯一**一处：`effective = clamp(base × percent / 100, MIN, MAX)`
/// （[`effective_track_height_px`]）。`base` 来自每轨覆盖，`percent` 乘在它**之后**、
/// 夹紧发生在**最后** —— 于是"这条鼓轨比那条总线高"（每轨值）与"整体放大一档"
/// （全局乘子）不会被压成一个旋钮（`ADR-0004` Q1 否决单一全局缩放的理由）。
///
/// ## 为什么 key 是**身份**而不是下标
///
/// `per_track_px` 的键是轨道的 26 字符 `EntityId` 规范文本（与 [`TrackView::id`] 同口径，
/// 也是 `[UI-TEST-001]` 语义寻址用的那个身份）。用下标做键的话，插入/删除一条轨道会让
/// 所有后续轨道的高度**静默错配**到别的轨道上 —— 判据
/// `per_track_height_is_keyed_by_identity_not_by_index` 钉住这一点。
///
/// ## 零 schema（本片），以及"重启丢什么"
///
/// 它**不进** `YebanProjectV1`（不进 `.yeban` 容器、不动 223 条冻结键路径、不动
/// `canonical_lines`/schema）、**也不进** `local_config`。它作为**会话/视图态**住在
/// `MainWindow` 的属性里（`host::track_height_layout` 是唯一读入口）。
/// 因此进程退出后（含 GUI 重启 / `yeban-app` 重启）：
///
/// - 每一条每轨高度覆盖**全部丢失** ⇒ 回到 [`DEFAULT_TRACK_HEIGHT_PX`]；
/// - 全局乘子**丢失** ⇒ 回到 100%。
///
/// 这是 `ADR-0004` Q4-A 明确的代价（"第 2 层按构造不能持久化"，`model_isolation.rs:539`
/// 机械断言 `session.rs` 无 serde 字样）。`Q1` 的负责人裁决若把 `height_px` 放进工程，
/// **投影一行都不用改**：变的只是这个 map 的**来源**（模型字段 → 视图态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackHeightLayout {
    percent: u32,
    per_track_px: BTreeMap<String, u32>,
}

impl Default for TrackHeightLayout {
    /// 默认布局：乘子 100、无每轨覆盖 —— 即 S0 的几何（判据钉住逐位相等）。
    fn default() -> Self {
        Self {
            percent: DEFAULT_TRACK_HEIGHT_PERCENT,
            per_track_px: BTreeMap::new(),
        }
    }
}

impl TrackHeightLayout {
    /// 一个每轨高度覆盖的键（轨道身份的 26 字符规范文本）是否合法。
    ///
    /// 只拒绝空串：投影**不**解析 `EntityId`（那会让视图态依赖模型的解析错误），
    /// 它只是把键当不透明身份；不在工程里的身份自然匹配不到任何行（无害）。
    #[must_use]
    fn is_valid_key(track_id: &str) -> bool {
        !track_id.is_empty()
    }

    /// 全局高度缩放级的百分比（100 = 不缩放）。
    #[must_use]
    pub fn percent(&self) -> u32 {
        self.percent
    }

    /// 每轨基准高的覆盖表（身份 → 整数逻辑像素）。
    #[must_use]
    pub fn overrides(&self) -> &BTreeMap<String, u32> {
        &self.per_track_px
    }

    /// 本布局是否就是默认布局（乘子 100 且无覆盖）—— 默认路径的快速判据。
    #[must_use]
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    /// 设置全局高度缩放级；返回"是否真的变了"（同一个值 ⇒ `false`，调用方不必重投影）。
    pub fn set_percent(&mut self, percent: u32) -> bool {
        if self.percent == percent {
            return false;
        }
        self.percent = percent;
        true
    }

    /// 去掉一条轨道的每轨覆盖（回到 [`DEFAULT_TRACK_HEIGHT_PX`]）；返回是否真的变了。
    pub fn remove_track_px(&mut self, track_id: &str) -> bool {
        self.per_track_px.remove(track_id).is_some()
    }

    /// 设置/替换一条轨道的**基准**高（整数逻辑像素，乘子尚未作用）。
    ///
    /// `px == 0` 是**退化值**（`ADR-0004` Q2：损坏，不是布局选择），这里按"删除覆盖"
    /// 处置并返回 `false`（视图态不是文档，不 panic；但 0 也**不会**被当成一个高度）。
    /// 空身份同样拒绝（`false`）。
    ///
    /// 边界**不在这里**：本函数不夹紧 `px`（哪怕 `px > MAX_TRACK_HEIGHT_PX`），
    /// 夹紧是 [`effective_track_height_px`] 的事 —— 于是"边界只有一处"（`Q2-B`）；
    /// 把上界也写进 setter 会造出第二个边界（`Q2-C` 被否决的同一个理由）。
    /// 返回 `true` 表示布局真的变了。
    pub fn set_track_px(&mut self, track_id: &str, px: u32) -> bool {
        if px == 0 {
            return self.remove_track_px(track_id);
        }
        if !Self::is_valid_key(track_id) {
            return false;
        }
        self.per_track_px.insert(track_id.to_owned(), px) != Some(px)
    }

    /// 一条轨道的**基准**高：有覆盖用覆盖，否则用 [`DEFAULT_TRACK_HEIGHT_PX`]。
    #[must_use]
    pub fn base_px(&self, track_id: &str) -> u32 {
        self.per_track_px
            .get(track_id)
            .copied()
            .unwrap_or(DEFAULT_TRACK_HEIGHT_PX)
    }

    /// 一条轨道的**有效**行高（整数逻辑像素）—— 投影里唯一的那次计算。
    #[must_use]
    pub fn effective_px(&self, track_id: &str) -> u32 {
        effective_track_height_px(self.base_px(track_id), self.percent)
    }

    /// 从宿主窗口的**三个**视图态属性构造布局（纯函数；`host::track_height_layout` 是唯一调用点）。
    ///
    /// - `ids` / `pxs` 是**按身份配对**的两个平行数组（与 `selected-ulids` 同款形状）：
    ///   逐下标 zip（长度不等时按短的那个截断，多出来的尾部**不会**被当成高度）；
    /// - `px` 是 Slint 的 `length`（`f32`），这里 `round` 到最近整数（`D28` 的整数口径）：
    ///   非有限值（`NaN`/`±∞`）与 `≤ 0` 一律视为"没有覆盖"（视图态不 panic，
    ///   但也**不会**把坏值变成布局选择 —— 它退回默认行高）；
    /// - `percent` 是 Slint 的 `int`：负数/0 折算成 `0` ⇒ [`effective_track_height_px`]
    ///   把它夹到 [`MIN_TRACK_HEIGHT_PX`]（"缩到最小"，不是"这一行消失"）。
    #[must_use]
    pub fn from_view_state(ids: &[String], pxs: &[f32], percent: i32) -> Self {
        let mut layout = Self {
            percent: u32::try_from(percent).unwrap_or(0),
            per_track_px: BTreeMap::new(),
        };
        for (id, px) in ids.iter().zip(pxs.iter()) {
            if !Self::is_valid_key(id) || !px.is_finite() {
                continue;
            }
            let rounded = px.round();
            if rounded <= 0.0 {
                continue;
            }
            // `f32 → u32`：`round` 之后仍在 `u32` 范围外的（> u32::MAX / NaN 已排除）
            // 视为没有覆盖，而不是饱和成一个巨大的行高。
            if let Ok(px) = u32::try_from(rounded as i64) {
                layout.per_track_px.insert(id.clone(), px);
            }
        }
        layout
    }
}

/// 投影**全部非主总线轨道**的行几何（`BTreeMap` 键序 ⇒ 与 [`TrackView::index`] 同序），
/// 行高来自给定的[视图态布局](TrackHeightLayout)。
///
/// 这是编排视图纵向布局的**唯一**事实源：包头 / 车道（[`TrackView`]）、剪辑（[`ClipView`]）
/// 与自动化带（[`crate::automation::project_lanes_with_rows`]）全部由它派生 ——
/// 旧版「`.slint` 算一遍 `42px + 56px * i`、`automation.rs` 再算一遍
/// `TRACK_LANE_HEIGHT_PX * i`」的两份事实源在 S0 合并成这一处。
///
/// `y` 仍是**前缀和**（`ADR-0004` S0 的形状）：S0 里 `stride` 恒 56 ⇒ 前缀和与闭式
/// `top + 56 × i` 逐位相等；S1 起 `stride` 逐行不同，**闭式不再成立**，前缀和是唯一的算法。
#[must_use]
pub fn track_rows_with_layout(
    project: &YebanProjectV1,
    layout: &TrackHeightLayout,
) -> Vec<RowGeometry> {
    let mut rows = Vec::new();
    let mut y = crate::automation::TRACK_LANE_TOP_PX;
    for track in project.tracks.values() {
        if track.id == project.master_bus_track_id {
            continue;
        }
        let height_px = layout.effective_px(&track.id.to_canonical_string());
        #[allow(clippy::cast_precision_loss)] // ≤ MAX_TRACK_HEIGHT_PX = 320 ≪ 2^24 ⇒ 精确
        let stride = height_px as f32;
        rows.push(RowGeometry {
            y,
            stride,
            height_px,
        });
        y += stride;
    }
    rows
}

/// 投影**全部非主总线轨道**的行几何，行高用**默认布局**（无每轨覆盖、乘子 100）。
///
/// 等价于 [`track_rows_with_layout`] 传入 [`TrackHeightLayout::default`]：默认路径的几何
/// 因此**逐位**等于 S0 的几何（判据 `default_layout_geometry_is_bit_for_bit_the_s0_geometry`）。
#[must_use]
pub fn track_rows(project: &YebanProjectV1) -> Vec<RowGeometry> {
    track_rows_with_layout(project, &TrackHeightLayout::default())
}

/// 视图里的一个轨道（**不是**模型实体 —— 它是投影结果，可以带界面派生字段）。
#[derive(Debug, Clone, PartialEq)]
pub struct TrackView {
    /// 视图内的轨道序号（0 起，**已排除**主总线）。进 `track-{i}-*` 的语义 ID。
    pub index: usize,
    /// `EntityId` 的 26 字符规范文本。
    pub id: String,
    /// 显示名（`TrackV3::name`）。
    pub name: String,
    /// 轨道类型（`midi` / `audio` / `aux-return` / `master`），`.slint` 用它选图标位。
    pub kind: &'static str,
    /// 界面色标（`TrackV3::color`，`None` = 用主题默认）。
    pub color: Option<String>,
    /// 已解析（或已回退）的色标 RGB —— `.slint` 的 `[color]` 数组由它构造。
    pub color_rgb: RgbColor,
    /// 规范化的 `#RRGGBB` 文本（回退色也在这里），进控件树标签与判据。
    pub color_hex: String,
    /// 静音（`TrackV3::mute`）。
    pub mute: bool,
    /// 独奏（`TrackV3::solo`）。
    pub solo: bool,
    /// 独奏安全（`TrackV3::solo_safe`）。
    pub solo_safe: bool,
    /// ⭐ SFZ 采样器绑定的**路径原文**（`TrackV3::sampler`，`#76` 的模型字段）。
    ///
    /// `None` 是**一等状态**（"这条轨不用采样器"），⛔ 不是"空字符串"：`Clear` 落到它，
    /// 而 `.slint` 的文本输入面把 `None` 显示成空串（见 [`ViewState::track_sampler_paths`]）。
    ///
    /// ⚠ 投影**不**查文件存在性、⛔ 不规范化、⛔ 不判扩展名 —— 与模型的字段口径逐字相同
    /// （路径是留痕字符串，不是"已装载的乐器"）。"这份 SFZ 读不到"是**发布点**的事
    /// （[`crate::engine_host::EngineHost::publish_project`] 明确失败，⛔ 不静默回退）。
    pub sampler_path: Option<String>,
    /// 音量 (dB)，原样保留模型值。
    pub volume_db: f32,
    /// 音量的显示文本（`{:.1}` dB）。
    pub volume_display: String,
    /// 推子位置（0.0–1.0），由 [`volume_fraction`] 从 `volume_db` 派生。
    ///
    /// 界面的推子帽位置只读它 —— 与**电平柱高**（`track-meter-levels`，来自引擎）是
    /// 两个不同的量：推子说"我把它设到多大声"，电平说"它现在多大声"。
    pub volume_fraction: f32,
    /// 声相，模型是 `-1.0..=1.0` 的 `f32`；投影成**整数千分之一**以避免在界面层碰浮点。
    pub pan_millis: i32,
    /// 声相的显示文本（`"C"` / `"L50"` / `"R30"`）—— `.slint` 直接画它，界面不做任何算术。
    pub pan_display: String,
    /// 该轨道上的摆放数量（`TrackV3::clips` 的长度）。
    pub clip_count: usize,
    /// 是否为 `master_bus_track_id` 指向的主总线。
    pub is_master: bool,
    /// 编排车道的**行顶沿 y**（逻辑像素）—— 由 [`track_rows`] 的前缀和给出。
    ///
    /// `.slint` 直接画它，不再算 `42px + 56px * track_index`（`ADR-0004` S0）。
    /// 主总线在编排视图**没有行**（它是调音台的通道条）⇒ 恒 `0.0`。
    pub y: f32,
    /// 编排车道的**行矩形高**（逻辑像素）= 行槽高 − [`TRACK_ROW_GAP_PX`]。
    ///
    /// 主总线没有行 ⇒ 恒 `0.0`。旧 `.slint` 里这是包头 / 车道的 `height: 54px`。
    pub height: f32,
    /// 这一行的**有效行高**（整数逻辑像素，`ADR-0004` S1）—— 投影的**整数真相**：
    /// `height` = `row_height_px as f32 − TRACK_ROW_GAP_PX`（判据逐位断言）。
    ///
    /// 它由 [`TrackHeightLayout::effective_px`] 从"每轨基准 × 全局乘子"一次算出
    /// （唯一的夹紧点）；界面今天不读它，但它是**下一次拖拽的起点**（拖拽必须从
    /// "当前有多高"开始，而不是从 `.slint` 里猜），也是"整数结果"这条判据的读点。
    /// 主总线没有行 ⇒ 恒 `0`。
    pub row_height_px: u32,
}

/// 视图里的一个 MIDI 音符（`MidiNote` + 它落在哪个片段池条目上 + 投影算出的位置）。
///
/// **位置是投影算出来的**（不是界面数出来的）：`x` 来自 `start_tick` 经 [`tick_to_px`]
/// 的**整数除法**，`width` 来自 `end_tick - start_tick` 的整数像素差（下限
/// [`MIN_BLOCK_WIDTH_PX`]），`row` / `y` 来自 `pitch` 经 [`pitch_lane`] 的**整数**车道映射。
/// 因此"同一个音符在不同 PPQ 缩放下位置单调"与"越界 tick 不 panic"都是可判据的事实，
/// 而不是注释里的承诺（见 `docs/ledger/app-completion-notes.md` §3）。
#[derive(Debug, Clone, PartialEq)]
pub struct NoteView {
    /// 视图内序号（片段池键序 → 音符键序，去重保序）。
    pub index: usize,
    /// `MidiNote::id` 的 26 字符规范文本 —— `note-{ulid}-rect` 的 `{ulid}` 段。
    pub id: String,
    /// 来自哪个片段池条目（`ClipPoolEntry::id`），让音符身份可回溯。
    pub clip_id: String,
    /// 起始 tick（`MidiNote::start_tick`）。
    pub start_tick: u64,
    /// 结束 tick（`start_tick + duration_ticks`，`checked_add`）。
    pub end_tick: u64,
    /// 时值（tick）。
    pub duration_ticks: u64,
    /// 音高 0..=127。
    pub pitch: u8,
    /// 力度 0..=127（模型原值）。
    pub velocity: u8,
    /// 力度归一化到 0.0–1.0（力度泳道按比例画柱高）。
    pub velocity_normalized: f32,
    /// 音符块左沿相对时间轴 0 的 x（逻辑像素，`tick_to_px(start_tick)`）。
    pub x: f32,
    /// 音符块宽（逻辑像素，下限 [`MIN_BLOCK_WIDTH_PX`]）。
    pub width: f32,
    /// 音高车道索引（0 = 最上面一条；由 [`pitch_lane`] 整数映射）。
    pub row: i32,
    /// 音符块顶沿的相对 y（逻辑像素，= `PITCH_LANE_HEIGHT_PX × row + NOTE_INSET_Y_PX`）。
    pub y: f32,
}

/// 视图里的一个剪辑摆放（`ClipPlacement` + 它落在哪条轨 / 哪个片段池条目上）。
#[derive(Debug, Clone, PartialEq)]
pub struct ClipView {
    /// 视图内的剪辑序号（0 起，按"轨道序 → 摆放身份"排序）。
    pub index: usize,
    /// 摆放身份（`ClipPlacement::id`）的规范文本 —— `[UI-TEST-001]` 的 `clip-{ulid}-header`。
    pub placement_id: String,
    /// 片段池身份（`ClipPlacement::clip_id`），用于显示名与内容类型。
    pub clip_id: String,
    /// 落在哪条视图轨道上（[`TrackView::index`]）。
    pub track_index: usize,
    /// 该轨道的显示名。
    pub track_name: String,
    /// 片段池条目的显示名（`ClipPoolEntry::name`）。
    pub clip_name: String,
    /// 片段内容类型：`midi` / `audio`。
    pub content: &'static str,
    /// 界面标签：`"{track_name} · {clip_name}"`（`.slint` 直接画它）。
    pub label: String,
    /// 起始 tick（`ClipPlacement::start_tick`）。
    pub start_tick: u64,
    /// 结束 tick（`start_tick + duration_ticks`，`checked_add`）。
    pub end_tick: u64,
    /// 摆放时值 (tick)。
    pub duration_ticks: u64,
    /// 时间轴相对 x（逻辑像素，0 = tick 0）。
    pub x: f32,
    /// 块宽（逻辑像素，下限 [`MIN_BLOCK_WIDTH_PX`]）。
    pub width: f32,
    /// 块顶沿相对 y（逻辑像素）= 所在行的 [`RowGeometry::clip_y`]。
    ///
    /// 旧 `.slint` 算的是 `42px + 56px * root.clip-lanes[clip_index] + 4px` —— 现在由
    /// 投影从**同一份**行几何给出（`ADR-0004` S0）。
    pub y: f32,
    /// 块高（逻辑像素）= 所在行的 [`RowGeometry::clip_height`]（旧 `.slint` 的 `46px`）。
    pub height: f32,
    /// 车道序号（= [`ClipView::track_index`]，进 `canonical_lines` 的 `lane=`）。
    pub lane: i32,
    /// 是否静音（`ClipPlacement::muted`）。
    pub muted: bool,
}

/// 视图里的一个曲式段落（`SectionV3`）。
#[derive(Debug, Clone, PartialEq)]
pub struct SectionView {
    /// 视图内序号（`BTreeMap` 键序 ⇒ 身份升序 ⇒ 跨进程稳定）。
    pub index: usize,
    /// 段落身份规范文本。
    pub id: String,
    /// 段落名（`Intro` / `Verse` / …）。
    pub name: String,
    /// 起始 tick。
    pub start_tick: u64,
    /// 结束 tick。
    pub end_tick: u64,
    /// 时间轴相对 x（逻辑像素）。
    pub x: f32,
    /// 卡片宽（逻辑像素，下限 [`MIN_BLOCK_WIDTH_PX`]）。
    pub width: f32,
    /// 界面色标。
    pub color: Option<String>,
}

/// 素材池面板一行里展示的**资产哈希前缀长度**（64 位十六进制的前 8 位）。
///
/// 面板只需"这一行指向哪份字节"的可辨认前缀；完整哈希属于工程文档，⛔ 不为它另造
/// 一份数据面。判据读的是同一份投影（`AssetPoolRowView::detail`），不是这个常量。
pub const ASSET_PREFIX_CHARS: usize = 8;

/// 视图里的一个**素材池条目**（`YebanProjectV1::clip_pool` 的一行）。
///
/// ⭐ 任务单 `#133`：`clip_pool` 在 `.slint` 里此前是 **0 处**投影（本单之前
/// `grep -rn clip_pool --include=*.slint crates/ | wc -l` = 0）⇒ 录音经 `#132`
/// 落进 `clip_pool` 之后，用户在界面上**看不到它**。本结构是素材池的**唯一**
/// 投影产物：面板的三个平行数组（[`ViewState::asset_pool_names`] /
/// [`ViewState::asset_pool_kinds`] / [`ViewState::asset_pool_details`]）都由它派生。
///
/// 字段**只**来自工程文档：`ClipPoolEntry` ＋ `YebanProjectV1::assets` 的
/// `AssetMetadata::byte_len`。`assets` 索引里没有登记元数据时，`detail` 如实写
/// 「字节数未知」—— ⛔ 不按 WAV 头 / 帧数**推算**一个字节数（那会是第二个真相源）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetPoolRowView {
    /// 片段池身份规范文本（`ClipPoolEntry::id`）。
    pub clip_id: String,
    /// 显示名（`ClipPoolEntry::name`）。
    pub name: String,
    /// 内容类型（`"音频"` / `"MIDI"`）。
    pub kind: String,
    /// 该行**拿得到**的字段：音频 = 字节数／资产前缀／增益；MIDI = 音符条数。
    pub detail: String,
    /// 一整行无障碍标签（`"{name} · {kind} · {detail}"`）。
    pub label: String,
}

/// 设备机架里的一格设备 —— **当前展示轨** `TrackV3::devices` 的一条。
///
/// ⭐ 任务单 `#157`：这是设备链在界面上的**唯一**数据形态。字段全部来自
/// `DeviceDefinition`（`id` / `name` / `bypassed` / `params`）；"已识别 / 未识别"
/// 的判定走 [`crate::effect_params::recognise`]（与 MCP 公布的是**同一份**契约）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRowView {
    /// 在 `TrackV3::devices` 里的下标（`Op::InsertDevice` / `RemoveDevice` 的寻址单位）。
    ///
    /// ⚠ 它是 `Vec` 下标而不是身份：本字段只用于**显示顺序**与语义 ID
    /// （`device-{slot}-card`），提交时宿主按**身份**重新寻址（`device_id`）。
    pub slot_index: usize,
    /// 设备身份规范文本（`DeviceDefinition::id`）。
    pub device_id: String,
    /// 显示名（`DeviceDefinition::name`）。
    pub name: String,
    /// 是否旁通（`DeviceDefinition::bypassed`）。
    pub bypassed: bool,
    /// 已识别参数的**逐字读数**：`"已识别 2 · EQ：eq_low_gain=6 dB, eq_low_freq=200 Hz"`；
    /// 一个都没有时是空串（界面据此不画这一行）。
    pub recognised: String,
    /// 未识别参数的逐字读数：`"未识别 1 · cutoff=1200 Hz"`；一个都没有时是空串。
    ///
    /// "未识别"= 引擎的 `insert.rs` 不认这个拼写 ⇒ 写进设备也**不会**生效
    /// （`effect_params` 的模块文档逐字写着这一点）。
    pub unrecognised: String,
}

impl DeviceRowView {
    /// 一整行无障碍标签：`"{name} · 已旁通/生效"`。
    #[must_use]
    pub fn label(&self) -> String {
        format!(
            "{} · {}",
            self.name,
            if self.bypassed { "已旁通" } else { "生效" }
        )
    }
}

/// 设备机架上那一格**可编辑**的 EQ 低架增益（`eq_low_gain`）。
///
/// ⭐ 任务单 `#157` 的"最小可编辑"：本切片只有这一格参数可写。值域（[`Self::min_db`] /
/// [`Self::max_db`]）来自 [`crate::effect_params::recognise`] 的 `min`/`max` ——
/// ⛔ 界面层**没有**第二份范围表（见 `lib.rs` 里 `effect_params` 的模块注释）。
///
/// 寻址用 `slot_index` + `param_index`（`AutomationTarget::DeviceParam` 的两个下标），
/// 宿主在提交时**重新**从权威工程求一次目标，⛔ 不信界面上的陈旧下标。
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceEqView {
    /// 设备链下标。
    pub slot_index: usize,
    /// 该设备 `params` 里的下标。
    pub param_index: usize,
    /// 设备显示名（界面标题用）。
    pub device_name: String,
    /// 当前值 (dB)，模型原值。
    pub value_db: f32,
    /// 有效值域下界 (dB)，来自契约。
    pub min_db: f32,
    /// 有效值域上界 (dB)，来自契约。
    pub max_db: f32,
}

/// 发送条里的一行 —— **展示轨**上的一条 `RoutingKind::SendToAux` 边。
///
/// ⭐ 任务单 `#159`：这是发送在界面上的**唯一**数据形态。四个字段全部来自
/// `RoutingGraph::edges`（边身份 / 目的地 / `gain_db`）；目的地名字只来自
/// `YebanProjectV1::tracks`，⛔ 界面层不持有第二份发送列表，也⛔ 不编造目的地。
#[derive(Debug, Clone, PartialEq)]
pub struct SendRowView {
    /// 路由边身份规范文本（`RoutingEdge::id`，26 字符 ULID）—— 三个手势的寻址单位。
    pub edge_id: String,
    /// 目的地音轨显示名（`RoutingEdge::destination_node` 指向的 `TrackV3::name`）。
    pub destination_name: String,
    /// 边增益（`RoutingEdge::gain_db`；`None` = 单位增益）。
    pub gain_db: Option<f32>,
    /// 增益的显示文本：`Some(v)` ⇒ `"{v} dB"`，`None` ⇒ `"单位增益"`。
    pub gain_display: String,
}

impl SendRowView {
    /// 一整行无障碍标签：`"{目的地} · {增益}"`。
    #[must_use]
    pub fn label(&self) -> String {
        format!("{} · {}", self.destination_name, self.gain_display)
    }
}

/// 发送增益的显示文本（**唯一**的格式化实现）。
///
/// `None` **不是** `0 dB`：它是模型里的"单位增益"一等状态（`RoutingEdge::gain_db` 的
/// `Option` 语义，见 `Op::SetRoutingGain` 的文档）⇒ 界面上必须逐字写成 `"单位增益"`，
/// ⛔ 不许悄悄画成 `0 dB`。
fn send_gain_display(gain_db: Option<f32>) -> String {
    gain_db.map_or_else(|| "单位增益".to_owned(), |gain_db| format!("{gain_db} dB"))
}

/// "新建发送"不可用时逐字写出的原因：工程里**没有非主总线音轨**可作发送源。
pub const SEND_CREATE_NO_TRACK_REASON: &str = "没有可显示的音轨，无法新建发送";

/// "新建发送"不可用时逐字写出的原因：工程里**没有** `TrackKind::AuxReturn` 轨。
///
/// ⭐ 任务单 `#160` 之后界面**能**建轨（`NewTrackKind::AuxReturn`）⇒ 这句话指路，
/// 而不是像 `#159` 那样把"界面还没有新建轨的入口"写死成限制。
pub const SEND_CREATE_NO_AUX_REASON: &str =
    "没有辅助返回轨（AuxReturn），无法新建发送（用上方「+ 辅助返回」建一条）";

/// "新建发送"不可用时逐字写出的原因：展示轨到那条 aux **已经有**一条
/// `SendToAux` 边（同源同目的地）。
///
/// ⭐ 任务单 `#160`：`#159` 的"连点会各建一条新边"在这里被关掉 —— 判定读的是
/// `routing_graph.edges`（权威），⛔ 不是界面上的行数。
pub const SEND_CREATE_DUPLICATE_REASON: &str = "该发送已存在（源与目的地相同），无法重复新建";

/// 发送增益的取用值域 (dB) —— **模型给的**，⛔ 不是界面里新写的一份范围表。
///
/// 来源：`yeban-model` 的 `AutomationTarget::SendGain{..}.nominal_domain()`
/// （`crates/yeban-model/src/automation.rs` 以 [`yeban_model::NOMINAL_GAIN_MIN_DB`] /
/// [`yeban_model::NOMINAL_GAIN_MAX_DB`] 为发送增益的固有取值域）。
#[must_use]
pub fn send_gain_range() -> (f32, f32) {
    (
        yeban_model::NOMINAL_GAIN_MIN_DB,
        yeban_model::NOMINAL_GAIN_MAX_DB,
    )
}

/// 发送条展示的那条轨 —— 与设备机架**同一条**规则（[`device_rack_track`]：
/// 身份序第一条非主总线轨）。
///
/// ⚠ 界面没有"选中轨"状态（台账第 236 轮："the current track does not exist"）
/// ⇒ 发送区与上方设备链永远指同一条轨；换工程之后两者一起随 `BTreeMap` 键序变化，
/// 不需要宿主再同步一份下标。
#[must_use]
pub fn send_strip_track(project: &YebanProjectV1) -> Option<&TrackV3> {
    device_rack_track(project)
}

/// "新建发送"的目的地 —— 身份序（`BTreeMap` 键序）第一条 `TrackKind::AuxReturn` 轨。
///
/// ⛔ 不按名字猜（"Reverb" 之类的词不构成契约）：类型是唯一判据。
#[must_use]
pub fn send_aux_target(project: &YebanProjectV1) -> Option<&TrackV3> {
    project
        .tracks
        .values()
        .find(|track| track.kind == TrackKind::AuxReturn)
}

/// 发送条 + "新建发送"可用性的投影本体（[`ViewState`] 与宿主提交路径**共用**这一份口径）。
///
/// 返回 `(发送行, 能否新建, 不能新建时的逐字原因)`。发送行来自展示轨的
/// `SendToAux` 出边（`BTreeMap` 边键序 ⇒ 跨进程确定）；目的地名字来自 `tracks`。
#[must_use]
fn send_strip_projection(
    project: &YebanProjectV1,
    track: Option<&TrackV3>,
) -> (Vec<SendRowView>, bool, String) {
    let Some(track) = track else {
        return (Vec::new(), false, SEND_CREATE_NO_TRACK_REASON.to_owned());
    };
    let sends = project
        .routing_graph
        .edges
        .values()
        .filter(|edge| edge.kind == RoutingKind::SendToAux && edge.source_node == track.id)
        .map(|edge| SendRowView {
            edge_id: edge.id.to_canonical_string(),
            destination_name: project.tracks.get(&edge.destination_node).map_or_else(
                || format!("未知节点 {}", edge.destination_node.to_canonical_string()),
                |destination| destination.name.clone(),
            ),
            gain_db: edge.gain_db,
            gain_display: send_gain_display(edge.gain_db),
        })
        .collect();
    let (available, reason) = send_create_availability(project);
    (sends, available, reason)
}

/// "新建发送"的可用性 ＋ 不可用时的**逐字**原因（[`ViewState`] 与宿主提交路径共用的
/// **唯一**口径）。
///
/// 三条不可用分支，按优先级：
///
/// 1. 没有展示轨 ⇒ [`SEND_CREATE_NO_TRACK_REASON`]；
/// 2. 没有 `AuxReturn` 轨 ⇒ [`SEND_CREATE_NO_AUX_REASON`]；
/// 3. 展示轨到那条 aux **已经**有一条 `SendToAux` 边 ⇒
///    [`SEND_CREATE_DUPLICATE_REASON`]（⭐ 任务单 `#160`：`#159` 登记过"连点会各建
///    一条新边"）。
///
/// ⛔ 不按名字猜目的地（类型是唯一判据），⛔ 不看界面上的行数（边是唯一事实源）。
#[must_use]
pub fn send_create_availability(project: &YebanProjectV1) -> (bool, String) {
    let Some(track) = send_strip_track(project) else {
        return (false, SEND_CREATE_NO_TRACK_REASON.to_owned());
    };
    let Some(aux) = send_aux_target(project) else {
        return (false, SEND_CREATE_NO_AUX_REASON.to_owned());
    };
    let duplicated = project.routing_graph.edges.values().any(|edge| {
        edge.kind == RoutingKind::SendToAux
            && edge.source_node == track.id
            && edge.destination_node == aux.id
    });
    if duplicated {
        return (false, SEND_CREATE_DUPLICATE_REASON.to_owned());
    }
    (true, String::new())
}

/// 发送增益手势的**权威寻址**（宿主提交路径用它，⛔ 不信界面上的行下标）。
///
/// 返回 `(当前增益, 值域下界, 值域上界)`。只认**展示轨**上的一条 `SendToAux` 边：
/// 边不存在 / 类型不对 / 源不是展示轨 ⇒ `None`（调用方据此放弃提交）。
#[must_use]
pub fn send_gain_target(
    project: &YebanProjectV1,
    edge_id: yeban_model::ids::EntityId,
) -> Option<(Option<f32>, f32, f32)> {
    let track = send_strip_track(project)?;
    let edge = project.routing_graph.edges.get(&edge_id)?;
    if edge.kind != RoutingKind::SendToAux || edge.source_node != track.id {
        return None;
    }
    let (min_db, max_db) = send_gain_range();
    Some((edge.gain_db, min_db, max_db))
}

// ---------------------------------------------------------------------------
// ⭐ 任务单 `#160`：轨 / 路由的 CRUD（建轨、删展示轨、删发送）
// ---------------------------------------------------------------------------

/// 界面可**新建**的三种轨。
///
/// ⛔ 不含 [`TrackKind::Master`]：主总线只能有一个，而模型的 `Op` 全集**没有**写
/// `master_bus_track_id` 的变体（见 `yeban-mcp` 的 `project_create` 模块头）⇒ 界面
/// 建不出第二条主总线，也不该让这个状态在类型上可表达。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewTrackKind {
    /// MIDI / 乐器轨（[`TrackKind::Midi`]）。
    Midi,
    /// 音频轨（[`TrackKind::Audio`]）。
    Audio,
    /// 辅助返回轨（[`TrackKind::AuxReturn`]）—— 建完之后"新建发送"才可能可用。
    AuxReturn,
}

impl NewTrackKind {
    /// 落进 `TrackV3::kind` 的模型枚举。
    #[must_use]
    pub const fn track_kind(self) -> TrackKind {
        match self {
            Self::Midi => TrackKind::Midi,
            Self::Audio => TrackKind::Audio,
            Self::AuxReturn => TrackKind::AuxReturn,
        }
    }

    /// 新轨到主总线那条边的 [`RoutingKind`]（语义见 `yeban-engine` 的 `routing.rs`：
    /// `TrackToBus` = 音轨 → 总线，`BusToMaster` = 总线 → 主输出）。
    ///
    /// ⇒ `Midi` / `Audio` 是**音轨** ⇒ [`RoutingKind::TrackToBus`]；
    /// `AuxReturn` 是**总线** ⇒ [`RoutingKind::BusToMaster`]。⛔ 两个方向不能互换。
    #[must_use]
    pub const fn routing_kind(self) -> RoutingKind {
        match self {
            Self::Midi | Self::Audio => RoutingKind::TrackToBus,
            Self::AuxReturn => RoutingKind::BusToMaster,
        }
    }

    /// 显示名前缀（**名字不是身份**：身份是 `EntityId`，见 [`track_create_ops`]）。
    const fn name_prefix(self) -> &'static str {
        match self {
            Self::Midi => "MIDI",
            Self::Audio => "Audio",
            Self::AuxReturn => "Aux",
        }
    }
}

/// 界面建轨失败时逐字写出的原因：工程里**没有主总线**。
///
/// 模型的 `validate()` 要求"有音轨时 `master_bus_track_id` 必须指向一条 `Master` 轨"，
/// 而 `Op` 全集**没有**写 `master_bus_track_id` 的变体 ⇒ 在**没有主总线**的工程里建轨只会
/// 写出一份非法文档。这里**响亮拒绝**，⛔ 不写半份。
///
/// ⚠ `#161` 之后"新建工程"（`new_project_archive()` → `samples::default_project()`）
/// **自带主总线** ⇒ 它不再触发这条拒绝。本常量仍然保留：`YebanProjectV1::default()`
/// 这类 0 轨／nil 主总线的工程（以及将来任何造出 0-master 工程的路径）依旧是非法起点。
pub const TRACK_CREATE_NO_MASTER_REASON: &str =
    "工程没有主总线，无法建轨（0 轨空工程请先打开一个带主总线的工程）";

/// 建轨是否可用 —— **唯一**判据是"工程有一条 `TrackKind::Master` 轨，且
/// `master_bus_track_id` 指着它"。
///
/// ⛔ 不分配身份、⛔ 不改工程：它只是 [`track_create_ops`] 的前置条件读出来的读数。
#[must_use]
pub fn track_create_available(project: &YebanProjectV1) -> bool {
    project
        .tracks
        .get(&project.master_bus_track_id)
        .is_some_and(|track| track.kind == TrackKind::Master)
}

/// 轨 / 路由段里那一行主总线说明（**投影给界面看**，不是界面自己拼的）。
///
/// - 有主总线 ⇒ `"主总线：{name}（不可删除）"`；
/// - 没有主总线（`master_bus_track_id` 为 nil 或指向非 `Master` 轨）⇒
///   [`TRACK_CREATE_NO_MASTER_REASON`]（同一行兼作建轨不可用的**逐字**原因）。
#[must_use]
pub fn master_bus_label(project: &YebanProjectV1) -> String {
    match project
        .tracks
        .get(&project.master_bus_track_id)
        .filter(|track| track.kind == TrackKind::Master)
    {
        Some(master) => format!("主总线：{}（不可删除）", master.name),
        None => TRACK_CREATE_NO_MASTER_REASON.to_owned(),
    }
}

/// ⭐ 任务单 `#180`：主总线设备链那一栏的标题（**投影给界面看**，不是界面自己拼的）。
///
/// - 有主总线 ⇒ `"主总线 {name} · 设备链"`；
/// - 没有主总线 ⇒ [`MASTER_DEVICE_INSERT_NO_MASTER_REASON`]（同一格兼作"不能装"的
///   **逐字**原因 ⇒ 界面不会出现"空白标题 + 禁用按钮但不说为什么"）。
#[must_use]
pub fn master_device_track_label(project: &YebanProjectV1) -> String {
    match master_device_track(project) {
        Some(master) => format!("主总线 {} · 设备链", master.name),
        None => MASTER_DEVICE_INSERT_NO_MASTER_REASON.to_owned(),
    }
}

/// 界面删轨失败时逐字写出的原因：工程里**没有可显示的音轨**。
pub const TRACK_DELETE_NO_TRACK_REASON: &str = "工程里没有可显示的音轨，无法删除";

/// 界面删轨失败时逐字写出的原因：唯一的音轨是**主总线**（`master_bus_track_id`）。
pub const TRACK_DELETE_MASTER_REASON: &str = "主总线（Master）不可删除";

/// 新轨的显示名：`"{前缀} {同类现有轨数 + 1}"`（如 `"MIDI 5"` / `"Aux 1"`）。
///
/// ⚠ 名字**不是身份**：`tracks` 的键是 `EntityId`（[`EntityId::new`] 分配）。因此
/// 删掉再建可能重名，模型允许（`TrackV3::validate` 不查空名，也不查重名）。
#[must_use]
pub fn new_track_name(project: &YebanProjectV1, kind: NewTrackKind) -> String {
    let existing = project
        .tracks
        .values()
        .filter(|track| track.kind == kind.track_kind())
        .count();
    format!("{} {}", kind.name_prefix(), existing + 1)
}

/// 建轨的**完整** `Op` 批次（⛔ 不是"只 `AddTrack`"）。
///
/// 返回 `[AddTrack, AddRoutingNode(新轨), ConnectRouting(新轨 → master)]`（主总线
/// 不在 `nodes` 里时先补它的 `AddRoutingNode`）。三条形状与 `yeban-mcp` 的
/// `project_create::build` **同一份**（`AddTrack` ＋ `AddRoutingNode` ＋
/// `ConnectRouting{kind: TrackToBus, gain_db: None}`）⇒ 新轨在路由图里**有一条显式的
/// 到主总线的边**，而不是靠引擎"没有出边 ⇒ 默认直连 master"这条回退。
///
/// ⚠ 引擎的回退确实存在（`yeban-engine/src/routing.rs` 模块头）：没有音频出边的节点默认
/// 直连 master。本函数仍然显式建边，因为 ① 演示 / 填充 / MCP 建工程三条既有路径都建了；
/// ② 判据要求"有条到 master 的边"；③ 图是唯一声学路由真理源，不把可听性寄托在回退上。
///
/// # Errors
///
/// - 没有主总线（或 `master_bus_track_id` 指向的轨不是 `Master`）⇒
///   [`TRACK_CREATE_NO_MASTER_REASON`]。
pub fn track_create_ops(
    project: &YebanProjectV1,
    kind: NewTrackKind,
) -> Result<Vec<Op>, &'static str> {
    let master = project
        .tracks
        .get(&project.master_bus_track_id)
        .filter(|track| track.kind == TrackKind::Master)
        .ok_or(TRACK_CREATE_NO_MASTER_REASON)?;
    let track_id = EntityId::new();
    let track = TrackV3 {
        id: track_id,
        name: new_track_name(project, kind),
        kind: kind.track_kind(),
        ..TrackV3::default()
    };
    let edge = RoutingEdge {
        id: EntityId::new(),
        source_node: track_id,
        destination_node: master.id,
        kind: kind.routing_kind(),
        gain_db: None,
    };
    let mut ops = Vec::with_capacity(4);
    ops.push(Op::AddTrack { track });
    ops.push(Op::AddRoutingNode { node: track_id });
    // 主总线**通常**已经在 `nodes` 里（`validate()` 要求）；缺失时补一格，否则
    // `ConnectRouting` 的前置条件（两端都在 `nodes` 里）会拒掉整批。
    if !project.routing_graph.nodes.contains(&master.id) {
        ops.push(Op::AddRoutingNode { node: master.id });
    }
    ops.push(Op::ConnectRouting { edge });
    Ok(ops)
}

/// 删轨的目标 —— 与设备机架 / 发送条**同一条**展示轨（[`device_rack_track`]）。
#[must_use]
pub fn track_delete_target(project: &YebanProjectV1) -> Option<&TrackV3> {
    device_rack_track(project)
}

/// 删轨的可用性 ＋ 不可用时的**逐字**原因。
///
/// 唯一的目标是展示轨（身份序第一条非主总线轨）⇒ 主总线**永远**不是目标；
/// 当工程里只剩主总线时给出 [`TRACK_DELETE_MASTER_REASON`]，空工程给出
/// [`TRACK_DELETE_NO_TRACK_REASON`]。
#[must_use]
pub fn track_delete_availability(project: &YebanProjectV1) -> (bool, String) {
    let reason = track_delete_reason(project);
    (reason.is_empty(), reason.to_owned())
}

/// 删轨不可用时的**逐字**原因（可用时是空串）—— [`track_delete_availability`] 的
/// `&'static str` 形态，让 [`track_delete_ops`] 不必为一句静态文案分配。
#[must_use]
pub fn track_delete_reason(project: &YebanProjectV1) -> &'static str {
    if track_delete_target(project).is_some() {
        return "";
    }
    if project.tracks.is_empty() {
        TRACK_DELETE_NO_TRACK_REASON
    } else {
        TRACK_DELETE_MASTER_REASON
    }
}

/// 删展示轨的**完整** `Op` 批次：先断开**每一条**引用它的边，再取走它的路由节点，
/// 最后取走音轨本身。
///
/// 顺序是承重的：`RemoveRoutingNode` 的前置条件是"没有任何边引用它"
/// （`ModelError::RoutingNodeInUse`）⇒ 边必须先走。整批由 `commit_ops` 包成**一个**
/// `Op::Batch`（原子）⇒ 一次手势 = **一步**可撤销的编辑。
///
/// ⛔ 主总线不可删：模型自己的 `Op::RemoveTrack` 前置条件就拒绝（`OpStateMismatch`），
/// 这里在**更早**的地方用 [`TRACK_DELETE_MASTER_REASON`] 给出人话原因。
///
/// # Errors
///
/// - 没有展示轨 ⇒ [`TRACK_DELETE_MASTER_REASON`]（只剩主总线）或
///   [`TRACK_DELETE_NO_TRACK_REASON`]（空工程）。
pub fn track_delete_ops(project: &YebanProjectV1) -> Result<Vec<Op>, &'static str> {
    let Some(track) = track_delete_target(project) else {
        return Err(track_delete_reason(project));
    };
    // 防御：展示轨按定义不是主总线；万一将来规则变了，这里也不会把主总线删掉。
    if track.id == project.master_bus_track_id {
        return Err(TRACK_DELETE_MASTER_REASON);
    }
    let mut ops = Vec::new();
    for edge in project.routing_graph.edges.values() {
        if edge.source_node == track.id || edge.destination_node == track.id {
            ops.push(Op::DisconnectRouting {
                edge_id: edge.id,
                previous_edge: *edge,
            });
        }
    }
    if project.routing_graph.nodes.contains(&track.id) {
        ops.push(Op::RemoveRoutingNode { node: track.id });
    }
    ops.push(Op::RemoveTrack {
        track_id: track.id,
        previous_track: track.clone(),
    });
    Ok(ops)
}

// ---------------------------------------------------------------------------
// ⭐ 任务单 `#163`：装设备 / 删设备 / 建空的 MIDI 片段（控制台 Tab 3 的同一面板）
// ---------------------------------------------------------------------------

/// 界面能**一键装上**的内置效果器（`DeviceKind::InternalEffect`）。
///
/// 参数名 / 单位 / 缺省值 / 值域**全部**来自 [`crate::effect_params::EFFECT_GROUPS`]
/// —— 那份契约是 `#[path]` 共享的 `yeban-mcp/src/effect_params.rs`（见 `src/lib.rs`），
/// ⛔ 本模块不写第二份参数名表，也不硬编码任何缺省数字。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinEffect {
    /// 三段塑形 EQ（契约分组 `eq`）—— 含 `eq_low_gain` ⇒ 装上之后 `#157` 的滑块立刻可用。
    Eq,
    /// 插入式 Freeverb（契约分组 `reverb`）—— 参数是 `reverb_*`。
    Reverb,
}

impl BuiltinEffect {
    /// 契约里的分组 id（[`crate::effect_params::EFFECT_GROUPS`] 的 `id` 字段）。
    #[must_use]
    pub const fn group_id(self) -> &'static str {
        match self {
            Self::Eq => "eq",
            Self::Reverb => "reverb",
        }
    }

    /// 写进 `DeviceDefinition::name` 的显示名。
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Eq => "EQ",
            Self::Reverb => "Reverb",
        }
    }
}

/// "装设备"不可用时的**逐字**原因：工程里没有可显示的音轨（展示轨不存在）。
pub const DEVICE_INSERT_NO_TRACK_REASON: &str = "没有可显示的音轨，无法装设备";

/// "装设备"不可用时的**逐字**原因：共享参数契约里找不到该分组（契约被改坏时的响亮失败）。
pub const DEVICE_INSERT_NO_CONTRACT_REASON: &str = "效果器契约里没有这一组参数，无法装设备";

/// "删设备"不可用时的**逐字**原因：工程里没有可显示的音轨。
pub const DEVICE_REMOVE_NO_TRACK_REASON: &str = "没有可显示的音轨，无法删除设备";

/// "删设备"不可用时的**逐字**原因：界面报的设备身份已经不在展示轨的设备链里。
pub const DEVICE_REMOVE_NOT_FOUND_REASON: &str = "这台设备已不在工程里，无法删除";

/// ⭐ 任务单 `#180`："给**主总线**装设备"不可用时的**逐字**原因：工程里没有主总线
/// （`master_bus_track_id` 为 nil，或它指向的轨不是 `TrackKind::Master`）。
pub const MASTER_DEVICE_INSERT_NO_MASTER_REASON: &str = "工程没有主总线，无法给主总线装设备";

/// ⭐ 任务单 `#180`："删**主总线**上的设备"不可用时的**逐字**原因：工程里没有主总线。
pub const MASTER_DEVICE_REMOVE_NO_MASTER_REASON: &str = "工程没有主总线，无法删除主总线上的设备";

/// ⭐ 任务单 `#180`："删**主总线**上的设备"不可用时的**逐字**原因：界面报的设备身份
/// 已经不在主总线的设备链里。
pub const MASTER_DEVICE_REMOVE_NOT_FOUND_REASON: &str = "这台设备已不在主总线上，无法删除";

/// "建片段"不可用时的**逐字**原因：工程里没有可显示的音轨（片段要摆在它上面）。
pub const CLIP_CREATE_NO_TRACK_REASON: &str = "没有可显示的音轨，无法建片段";

/// "建片段"不可用时的**逐字**原因：拍号分母为 0 / 小节长度溢出（算不出 4 小节）。
pub const CLIP_CREATE_BAD_TIME_SIGNATURE_REASON: &str = "拍号非法，无法计算片段长度";

/// 一键建出的空 MIDI 片段的**小节数**（4 小节；tick 数由工程的拍号算出，⛔ 不手抄）。
pub const CLIP_CREATE_BARS: u64 = 4;

/// "装设备"的可用性 ＋ 不可用时的**逐字**原因（可用时是空串）。
///
/// 唯一前置条件与投影**同一份**口径：存在展示轨（[`device_rack_track`]）。
#[must_use]
pub fn device_insert_availability(project: &YebanProjectV1) -> (bool, String) {
    let reason = if device_rack_track(project).is_some() {
        ""
    } else {
        DEVICE_INSERT_NO_TRACK_REASON
    };
    (reason.is_empty(), reason.to_owned())
}

/// "建片段"的可用性 ＋ 不可用时的**逐字**原因（可用时是空串）。
///
/// 片段摆在展示轨上 ⇒ 唯一前置条件与 [`device_insert_availability`] 相同。
#[must_use]
pub fn clip_create_availability(project: &YebanProjectV1) -> (bool, String) {
    let reason = if device_rack_track(project).is_some() {
        ""
    } else {
        CLIP_CREATE_NO_TRACK_REASON
    };
    (reason.is_empty(), reason.to_owned())
}

/// ⭐ 任务单 `#180`："给**主总线**装设备"的可用性 ＋ 不可用时的**逐字**原因。
///
/// 唯一前置条件与主总线投影**同一份**口径：存在主总线（[`master_device_track`]）。
/// ⛔ 与展示轨的 [`device_insert_availability`] **分开**两格 —— 两条轨的可用性
/// 互不蕴含（有展示轨不等于有主总线，反之亦然）。
#[must_use]
pub fn master_device_insert_availability(project: &YebanProjectV1) -> (bool, String) {
    let reason = if master_device_track(project).is_some() {
        ""
    } else {
        MASTER_DEVICE_INSERT_NO_MASTER_REASON
    };
    (reason.is_empty(), reason.to_owned())
}

/// 按效果器契约构造一台内置效果器：参数逐条取 `spec.default` / `spec.unit`。
///
/// 返回 `None` = 契约里没有这个分组（⛔ 不落一台空参数设备冒充成功）。
/// `latency_samples` 写 `0`：契约不公布内置效果器的延迟，`0` 与
/// `DeviceDefinition` 的"零延迟上报值"同义（⛔ 不编一个数字）。
#[must_use]
pub fn builtin_effect_device(effect: BuiltinEffect) -> Option<DeviceDefinition> {
    let group = crate::effect_params::EFFECT_GROUPS
        .iter()
        .find(|group| group.id == effect.group_id())?;
    let params: Vec<ParameterValue> = group
        .params
        .iter()
        .map(|spec| {
            #[allow(clippy::cast_possible_truncation)] // 契约里的缺省值是 f64 字面量
            let value = spec.default as f32;
            ParameterValue {
                name: spec.name.to_owned(),
                value,
                unit: if spec.unit.is_empty() {
                    None
                } else {
                    Some(spec.unit.to_owned())
                },
            }
        })
        .collect();
    Some(DeviceDefinition {
        id: EntityId::new(),
        name: effect.display_name().to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params,
        latency_samples: 0,
    })
}

/// 装一台内置效果器的**完整** `Op`：追加到展示轨设备链的**末尾**。
///
/// `slot_index = track.devices.len()`（`InsertDevice` 的文档：等于 `len` 表示追加）。
/// 设备身份来自 [`EntityId::new`]，参数来自 [`builtin_effect_device`]。
///
/// ⭐ 任务单 `#180`：本函数**只**认展示轨（[`device_rack_track`]）。主总线走
/// [`master_device_insert_ops`] —— 两条腿共用同一个 [`insert_device_op_on`]，因此
/// 参数 / 追加位置的规则不会在两处漂移，而**目标轨**由调用方**显式**选择。
///
/// # Errors
///
/// - 没有展示轨 ⇒ [`DEVICE_INSERT_NO_TRACK_REASON`]；
/// - 契约里没有该分组 ⇒ [`DEVICE_INSERT_NO_CONTRACT_REASON`]。
pub fn device_insert_ops(
    project: &YebanProjectV1,
    effect: BuiltinEffect,
) -> Result<Vec<Op>, &'static str> {
    let track = device_rack_track(project).ok_or(DEVICE_INSERT_NO_TRACK_REASON)?;
    Ok(vec![insert_device_op_on(track, effect)?])
}

/// ⭐ 任务单 `#180`：装一台内置效果器到**主总线**设备链的**末尾**。
///
/// 与 [`device_insert_ops`] 的差别**只有主语**：`track_id = master_bus_track_id`，
/// `slot_index = master.devices.len()`。⛔ 绝不复用 [`device_rack_track`]（那条规则
/// **排除**主总线 ⇒ 对 master 增删会作用到展示轨上 —— 那正是本单要关掉的洞）。
///
/// # Errors
///
/// - 没有主总线 ⇒ [`MASTER_DEVICE_INSERT_NO_MASTER_REASON`]；
/// - 契约里没有该分组 ⇒ [`DEVICE_INSERT_NO_CONTRACT_REASON`]。
pub fn master_device_insert_ops(
    project: &YebanProjectV1,
    effect: BuiltinEffect,
) -> Result<Vec<Op>, &'static str> {
    let track = master_device_track(project).ok_or(MASTER_DEVICE_INSERT_NO_MASTER_REASON)?;
    Ok(vec![insert_device_op_on(track, effect)?])
}

/// 设备插入 Op 的**唯一**构造点（展示轨与主总线共用；`track` 是唯一自变量）。
///
/// `slot_index = track.devices.len()` 表示追加到链尾；设备身份 / 参数缺省值来自
/// [`builtin_effect_device`]（契约是唯一事实源）。
fn insert_device_op_on(track: &TrackV3, effect: BuiltinEffect) -> Result<Op, &'static str> {
    let device = builtin_effect_device(effect).ok_or(DEVICE_INSERT_NO_CONTRACT_REASON)?;
    Ok(Op::InsertDevice {
        track_id: track.id,
        slot_index: track.devices.len(),
        device,
    })
}

/// 删一台设备的**完整** `Op`：按**设备身份**在展示轨里重新寻址。
///
/// 界面报的是 `DeviceDefinition::id` 的 26 字符文本（⛔ 不是行下标 —— 下标会随设备链
/// 增删漂移）。`previous_device` 逐字来自权威工程，供撤销复原。
///
/// ⭐ 任务单 `#180`：本函数**只**认展示轨；主总线走 [`master_device_remove_ops`]。
///
/// # Errors
///
/// - 没有展示轨 ⇒ [`DEVICE_REMOVE_NO_TRACK_REASON`]；
/// - 身份不在这条轨的设备链里 ⇒ [`DEVICE_REMOVE_NOT_FOUND_REASON`]。
pub fn device_remove_ops(
    project: &YebanProjectV1,
    device_id: EntityId,
) -> Result<Vec<Op>, &'static str> {
    let track = device_rack_track(project).ok_or(DEVICE_REMOVE_NO_TRACK_REASON)?;
    Ok(vec![remove_device_op_on(
        track,
        device_id,
        DEVICE_REMOVE_NOT_FOUND_REASON,
    )?])
}

/// ⭐ 任务单 `#180`：按**设备身份**删一台**主总线**上的设备。
///
/// 与 [`device_remove_ops`] 的差别**只有主语**：`track_id = master_bus_track_id`，
/// `slot_index` 在**主总线**的设备链里现查（⛔ 不信任界面上可能过期的下标）。
/// `previous_device` 逐字来自权威工程，供撤销复原。
///
/// # Errors
///
/// - 没有主总线 ⇒ [`MASTER_DEVICE_REMOVE_NO_MASTER_REASON`]；
/// - 身份不在主总线的设备链里 ⇒ [`MASTER_DEVICE_REMOVE_NOT_FOUND_REASON`]。
pub fn master_device_remove_ops(
    project: &YebanProjectV1,
    device_id: EntityId,
) -> Result<Vec<Op>, &'static str> {
    let track = master_device_track(project).ok_or(MASTER_DEVICE_REMOVE_NO_MASTER_REASON)?;
    Ok(vec![remove_device_op_on(
        track,
        device_id,
        MASTER_DEVICE_REMOVE_NOT_FOUND_REASON,
    )?])
}

/// 设备删除 Op 的**唯一**构造点（展示轨与主总线共用；`track` / `not_found_reason`
/// 是唯一自变量）。寻址单位是**设备身份**，`slot_index` 在这里现查。
fn remove_device_op_on(
    track: &TrackV3,
    device_id: EntityId,
    not_found_reason: &'static str,
) -> Result<Op, &'static str> {
    let slot_index = track
        .devices
        .iter()
        .position(|device| device.id == device_id)
        .ok_or(not_found_reason)?;
    let previous_device = track.devices[slot_index].clone();
    Ok(Op::RemoveDevice {
        track_id: track.id,
        slot_index,
        previous_device,
    })
}

/// 新建空 MIDI 片段的显示名：`"Clip {现有 clip_pool 条目数 + 1}"`。
///
/// 名字**不是身份**（身份是 [`EntityId::new`]），因此删掉再建可能重名 ——
/// 与 [`new_track_name`] 同一条口径。
#[must_use]
pub fn new_clip_name(project: &YebanProjectV1) -> String {
    format!("Clip {}", project.clip_pool.len() + 1)
}

/// 建一个**空的 MIDI 片段**并把它摆到展示轨的 tick 0：一次提交里两条 `Op`。
///
/// 形状（`Op::AddClipPlacement` 要求片段**已经在** `clip_pool` 里 ⇒ 顺序承重）：
///
/// 1. `Op::AddClip { clip: ClipPoolEntry { content: Midi { notes: {} } } }`；
/// 2. `Op::AddClipPlacement { start_tick: 0, duration_ticks: 4 小节 }`。
///
/// 时长**不写死**：`4 × bar_length_ticks(project.time_signature)`。4/4 下
/// `bar = 4 × 960 = 3840` ⇒ `duration_ticks = 15360`（判据同时断言这个真值）。
///
/// # Errors
///
/// - 没有展示轨 ⇒ [`CLIP_CREATE_NO_TRACK_REASON`]；
/// - 拍号分母为 0 / 小节长度溢出 ⇒ [`CLIP_CREATE_BAD_TIME_SIGNATURE_REASON`]。
pub fn clip_create_ops(project: &YebanProjectV1) -> Result<Vec<Op>, &'static str> {
    let track = device_rack_track(project).ok_or(CLIP_CREATE_NO_TRACK_REASON)?;
    let bar_ticks = bar_length_ticks(project.time_signature)
        .map_err(|_| CLIP_CREATE_BAD_TIME_SIGNATURE_REASON)?;
    let duration_ticks = bar_ticks
        .checked_mul(CLIP_CREATE_BARS)
        .ok_or(CLIP_CREATE_BAD_TIME_SIGNATURE_REASON)?;
    let clip_id = EntityId::new();
    let clip = ClipPoolEntry {
        id: clip_id,
        name: new_clip_name(project),
        content: ClipContent::Midi {
            notes: BTreeMap::new(),
        },
    };
    let placement = ClipPlacement {
        id: EntityId::new(),
        clip_id,
        start_tick: 0,
        duration_ticks,
        loop_config: LoopConfig::default(),
        muted: false,
    };
    Ok(vec![
        Op::AddClip { clip },
        Op::AddClipPlacement {
            track_id: track.id,
            placement,
        },
    ])
}

// ---------------------------------------------------------------------------
// ⭐ 任务单 `#164`：给展示轨**一键装乐器**（`TrackV3::sampler` ⇒ `Op::SetTrackSampler`）
// ---------------------------------------------------------------------------

/// 界面能**一键装上**的一份 SFZ 乐器。
///
/// `path` 是仓库里**真实存在**的 SFZ 文本（相对仓库根，逐字写进 `TrackV3::sampler.path`）。
/// ⚠ 模型把路径当**留痕字符串**：⛔ 不查存在性、⛔ 不规范化、⛔ 不解析
/// （与 [`SamplerBinding`] 的字段文档同一口径）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuiltinInstrument {
    /// 按钮上看得到的短名（⛔ 不是身份；身份仍是 [`TrackV3::sampler`] 的路径）。
    pub name: &'static str,
    /// 仓库内真实存在的 SFZ 路径（相对仓库根，逐字进模型）。
    pub path: &'static str,
}

/// 界面一键可装的 SFZ 乐器**候选**清单（⛔ 不是按钮行的事实源）。
///
/// 两份都在 `assets/samples/karoryfer-cowsynth/` 下、都覆盖中央 C（60）⇒ 同一份工程
/// 换一份乐器时事件网格不用改。⚠ 本清单⛔ 不包含"已解析乐器"：装载永远走
/// `crate::sfz_load`（`#79`／`#80` 的同一条管线），本模块只写路径。
///
/// ⭐ ⭐ **任务单 `#165`：候选 ≠ 可装。** 真正进按钮行的是
/// [`installable_builtin_instruments`] —— 它把候选按"**样本字节是否存在**"过滤一遍，
/// 判据只有 [`sfz_samples_present`]（`#164` 已有的读取路径）。
///
/// ⚠ **今天过滤结果是空**：仓内 `assets/samples/**` 有 **1398 份 `.sfz` 文本、0 个
/// 样本字节** ⇒ 两份候选都没法解码出任何 PCM ⇒ 一份都不可装（⛔ 不显示按钮）。
/// 这正是 `#165` 要的：⛔ 不许让"装乐器"把一条本来有声（恒备复音）的轨变成静音。
pub const BUILTIN_INSTRUMENTS: [BuiltinInstrument; 2] = [
    BuiltinInstrument {
        name: "Shifty Pluck",
        path: "assets/samples/karoryfer-cowsynth/cowsynth_shifty_pluck.sfz",
    },
    BuiltinInstrument {
        name: "Lonely Organ",
        path: "assets/samples/karoryfer-cowsynth/cowsynth_lonely_organ.sfz",
    },
];

/// "装乐器"不可用时的**逐字**原因：工程里没有可显示的音轨（展示轨不存在）。
pub const INSTRUMENT_INSTALL_NO_TRACK_REASON: &str = "没有可显示的音轨，无法装乐器";

/// ⭐ **本单的核心逐字原因**：目标 SFZ 的样本字节不在仓库 ⇒ 装了会让本轨**静音**。
///
/// 引擎对没装采样器的轨槽**恒备一台复音合成器**（`yeban-engine/src/synth.rs` 的
/// `armed_sources = 1 + …`）⇒ 不装乐器**有声**；而装一份全缺样本的 SFZ 会让
/// `decoded_samples == 0`、每个区域 `NO_SAMPLE` ⇒ **静音**。所以这条路必须**拒绝**，
/// 并把理由逐字说出来（⛔ 不静默禁用、⛔ 不装完再报"没声音"）。
pub const INSTRUMENT_INSTALL_NO_SAMPLES_REASON: &str =
    "该 SFZ 的样本文件不在仓库（0 个样本字节），装了会让本轨静音；请用自带样本的 SFZ";

/// 目标 SFZ 文本本身读不到 / 不是合法 SFZ 时的**逐字**原因。
///
/// 与 [`INSTRUMENT_INSTALL_NO_SAMPLES_REASON`] 分开：一个是"文件在、样本不在"，
/// 一个是"连 SFZ 都打不开"—— ⛔ 两种失败不共用一句假话。
pub const INSTRUMENT_INSTALL_UNREADABLE_SFZ_REASON: &str =
    "该 SFZ 读不出来（文件不在仓库或不是合法 SFZ），装了会让本轨静音；请用自带样本的 SFZ";

/// 内置候选**一份都不可装**时的**逐字**原因（今天就是这一档：0 个样本字节）。
pub const INSTRUMENT_INSTALL_NO_BUILTIN_REASON: &str =
    "仓内没有带样本的 SFZ（内置清单为空）；请用自带样本的 SFZ（Tab 1 的 SFZ 路径 ＋ SET）";

/// 判"样本字节是否存在"时交给 [`crate::sfz_load::decode_sfz_plan`] 的引擎采样率。
///
/// 它只影响 `InMemorySample` 的播放速率换算，⛔ 不影响"解没解出 PCM"这个判据；
/// 与界面批量入口（`host::on_load_sfz` 的 `48_000.0`）同值。
const INSTRUMENT_PROBE_ENGINE_HZ: f32 = 48_000.0;

/// ⭐ **本模块不再有第二份路径解析**（任务单 `#168`）：探测与渲染都调
/// [`crate::sfz_load::resolve_sampler_path`]。
///
/// [`BuiltinInstrument::path`] 的契约是"相对仓库根"（模型里逐字存这个字符串）⇒
/// 探测**不依赖进程 cwd**（`cargo test` 的 cwd 是 crate 目录，GUI 从仓根启动），
/// 而"从别的目录启动"也不再改变解释。相对路径在候选根下都找不到时，本函数返回
/// [`INSTRUMENT_INSTALL_UNREADABLE_SFZ_REASON`]（⛔ 不静默把它当成"有样本"）。
///
/// # Errors
///
/// * 样本缺失 / 一份都解不出 ⇒ [`INSTRUMENT_INSTALL_NO_SAMPLES_REASON`]；
/// * SFZ 文本本身读不到、路径哪都找不到、或文本非法 ⇒
///   [`INSTRUMENT_INSTALL_UNREADABLE_SFZ_REASON`]。
pub fn sfz_samples_present(path: &str) -> Result<(), &'static str> {
    let Ok(resolved) =
        crate::sfz_load::resolve_sampler_path(path, crate::sfz_load::SfzRoots::none())
    else {
        return Err(INSTRUMENT_INSTALL_UNREADABLE_SFZ_REASON);
    };
    match crate::sfz_load::decode_sfz_plan(&resolved, INSTRUMENT_PROBE_ENGINE_HZ) {
        Ok((_plan, report))
            if report.decoded_samples > 0
                && report.silent_region_count == 0
                && report.missing_sample_paths.is_empty() =>
        {
            Ok(())
        }
        Ok(_) => Err(INSTRUMENT_INSTALL_NO_SAMPLES_REASON),
        Err(_) => Err(INSTRUMENT_INSTALL_UNREADABLE_SFZ_REASON),
    }
}

/// ⭐ 内置候选里**真的可装**的那些（按样本字节过滤；顺序与 [`BUILTIN_INSTRUMENTS`] 一致）。
///
/// ⚠ **进程内只探一次**（`OnceLock`）：`ViewState::from_project` 每次完整重投影都会
/// 读它，而探测要读 SFZ 文本 ＋ 逐个样本读盘 ⇒ 每帧重探会把投影变成 I/O 热点。
/// 今天结果是**空切片**（仓内 0 个样本字节）；样本入库后无需改代码，下一进程自动出现。
#[must_use]
pub fn installable_builtin_instruments() -> &'static [BuiltinInstrument] {
    fn probe() -> Vec<BuiltinInstrument> {
        BUILTIN_INSTRUMENTS
            .iter()
            .copied()
            .filter(|instrument| sfz_samples_present(instrument.path).is_ok())
            .collect()
    }
    static INSTALLABLE: std::sync::OnceLock<Vec<BuiltinInstrument>> = std::sync::OnceLock::new();
    INSTALLABLE.get_or_init(probe).as_slice()
}

/// 第 `index` 份**可装**的内置乐器（越界 ⇒ `None`，⛔ 不 panic、⛔ 不猜一份）。
///
/// ⚠ 下标指的是 [`installable_builtin_instruments`] 的顺序（= 界面按钮行），
/// ⛔ 不是 [`BUILTIN_INSTRUMENTS`] 的候选顺序（今天前者为空 ⇒ 恒 `None`）。
#[must_use]
pub fn builtin_instrument(index: usize) -> Option<BuiltinInstrument> {
    installable_builtin_instruments().get(index).copied()
}

/// 内置乐器的**显示名**（与 [`installable_builtin_instruments`] 同序、同长度）。
#[must_use]
pub fn instrument_names() -> Vec<String> {
    installable_builtin_instruments()
        .iter()
        .map(|instrument| instrument.name.to_owned())
        .collect()
}

/// "装乐器"的可用性 ＋ 不可用时的**逐字**原因（可用时是空串）。
///
/// 两个前置条件，顺序即优先级：
/// 1. 存在展示轨（[`device_rack_track`]）—— 否则 [`INSTRUMENT_INSTALL_NO_TRACK_REASON`]；
/// 2. 内置清单里有**可装**的乐器（[`installable_builtin_instruments`] 非空）——
///    否则 [`INSTRUMENT_INSTALL_NO_BUILTIN_REASON`]（今天就是这一档）。
///
/// ⚠ 第 2 条是本单新增：清单空时 **0 个按钮** ⇒ 界面必须看得见原因（⛔ 不静默禁用）。
#[must_use]
pub fn instrument_install_availability(project: &YebanProjectV1) -> (bool, String) {
    let reason = if device_rack_track(project).is_none() {
        INSTRUMENT_INSTALL_NO_TRACK_REASON
    } else if installable_builtin_instruments().is_empty() {
        INSTRUMENT_INSTALL_NO_BUILTIN_REASON
    } else {
        ""
    };
    (reason.is_empty(), reason.to_owned())
}

/// 展示轨当前乐器的**显示文本**：`"乐器：<sfz 路径>"` / `"未装乐器"`。
///
/// 路径**逐字**来自 `TrackV3::sampler.path`（⛔ 不取文件名、⛔ 不截断）—— 判据读到的
/// 字符串与模型里的一模一样。
#[must_use]
pub fn instrument_label(track: Option<&TrackV3>) -> String {
    match track.and_then(|track| track.sampler.as_ref()) {
        Some(binding) => format!("乐器：{}", binding.path),
        None => "未装乐器".to_owned(),
    }
}

/// 给展示轨装一份 SFZ 乐器的**完整** `Op`（`Op::SetTrackSampler`）。
///
/// - `old_sampler` **逐字**来自权威工程 ⇒ 陈旧载荷会被模型逐字节拒掉；
/// - 值没变 ⇒ 返回**空 `Vec`**（宿主据此**不提交**，⛔ 不在撤销树上留一步空编辑）；
/// - ⭐ **本单新增（`#165`）**：装之前先用 [`sfz_samples_present`] 判"样本字节是否存在"，
///   不满足就**拒绝** —— 否则这条轨会从"恒备复音 ⇒ 有声"变成"区域 `NO_SAMPLE` ⇒ 静音"。
///
/// ⚠ 本函数因此**读盘**（SFZ 文本 ＋ 样本解码）：只跑在**控制侧**的一次点击上
/// （⛔ 不在音频回调里，红线 7 不涉及）。
///
/// ⚠ `path` 原样写进 [`SamplerBinding::path`]（⛔ 不规范化、⛔ 不改写）；相对路径的解析
/// 与渲染共用 [`crate::sfz_load::resolve_sampler_path`]（绝对原样 ⇒ 工程目录 ⇒ 仓库根
/// ⇒ cwd，见任务单 `#168`），⛔ 本模块不另写一份。
///
/// # Errors
///
/// 1. 没有展示轨 ⇒ [`INSTRUMENT_INSTALL_NO_TRACK_REASON`]（先判，保持 `#164` 的口径）；
/// 2. 样本字节不存在 ⇒ [`INSTRUMENT_INSTALL_NO_SAMPLES_REASON`]；
/// 3. SFZ 文本读不到 / 非法 ⇒ [`INSTRUMENT_INSTALL_UNREADABLE_SFZ_REASON`]。
pub fn instrument_install_ops(
    project: &YebanProjectV1,
    path: &str,
) -> Result<Vec<Op>, &'static str> {
    let track = device_rack_track(project).ok_or(INSTRUMENT_INSTALL_NO_TRACK_REASON)?;
    // ⭐ `#165`：⛔ 不许把一条有声的轨装成静音 —— 拒绝发生在**构造 `Op` 之前**。
    sfz_samples_present(path)?;
    let new_sampler = Some(SamplerBinding {
        path: path.to_owned(),
    });
    if track.sampler == new_sampler {
        return Ok(Vec::new());
    }
    Ok(vec![Op::SetTrackSampler {
        track_id: track.id,
        old_sampler: track.sampler.clone(),
        new_sampler,
    }])
}

/// 视图里的一个场景（`SceneV3`，Session View 的行）。
#[derive(Debug, Clone, PartialEq)]
pub struct SceneView {
    /// 视图内序号。
    pub index: usize,
    /// 场景身份规范文本。
    pub id: String,
    /// 场景名。
    pub name: String,
    /// 速度覆盖（`None` = 跟随工程速度）。
    pub tempo: Option<f64>,
    /// 界面色标。
    pub color: Option<String>,
}

/// 状态栏「音频配置」那一格的**整串文本**（台账 R9）—— 唯一的格式化实现。
///
/// 两个读数都来自 `YebanProjectV1::audio_config`（`[MODEL-AST-002]` 的
/// [`yeban_model::project::ProjectAudioConfig`]），即**引擎据此向声卡协商的那一份配置**：
/// `crates/yeban-engine/src/device.rs` 的 `EngineConfig::from_project` 就是取
/// `project.audio_config.sample_rate.hz()` 与 `block_size.frames()`。
///
/// ⇒ 它**不是**声卡协商**回来**的读数（那一个在 `DeviceOpening.sample_rate` /
/// `NegotiatedConfig`，只有设备腿真的打开时才存在，且生产路径只把它打到 CLI）。
/// `.slint` 侧不做字符串拼接：整串由这里产出，因为"哪几个数、什么单位、什么顺序"
/// 是一条语义，不是布局。
///
/// `"目标 120 FPS"` 是 `[UI-GRID-004]` 的**目标值**，文本自带"目标"二字 ⇒ 不是实测读数。
/// **没有** DSP / CPU 负载那一格：app 里没有任何真实负载读数。口径（先说单位再量）：对象 =
/// `crates/yeban-app/src` 下全部 `.rs` **去掉 `//` 注释之后**的代码文本，单位 = 命中行数，
/// `dsp_load` / `cpu_load` / `load_percent` 三个模式合计命中 **0**
/// （判据 `tests/status_bar_honesty.rs` 的 `no_dsp_load_reading_exists_in_the_app_source`）。
/// ⚠ 复核时必须**先去掉注释**：本行注释自身就含这三个词，不过滤注释的 grep 会得到 1。
/// 宁可少一格也不编。
#[must_use]
pub fn audio_config_display(
    sample_rate: yeban_model::project::SampleRate,
    bit_depth: yeban_model::project::BitDepth,
) -> String {
    format!(
        "{} / {} · 目标 120 FPS",
        sample_rate_display(sample_rate),
        bit_depth_display(bit_depth)
    )
}

/// 采样率的显示形态（`44100` ⇒ `"44.1 kHz"`，`48000` ⇒ `"48 kHz"`）。
///
/// 只做整数除法与取余：不引浮点。`SampleRate::ALL` 的 5 个取值都是 100 的整数倍
/// （44100 / 48000 / 88200 / 96000 / 192000）⇒ `hz % 1000 / 100` 就是 kHz 的**十分位**，
/// 于是只有"十分位为 0"与"十分位非 0"两种情形，不需要四舍五入也不需要格式化浮点。
#[must_use]
fn sample_rate_display(sample_rate: yeban_model::project::SampleRate) -> String {
    let hz = sample_rate.hz();
    let tenths = (hz % 1_000) / 100;
    if tenths == 0 {
        format!("{} kHz", hz / 1_000)
    } else {
        format!("{}.{} kHz", hz / 1_000, tenths)
    }
}

/// 位深的显示形态（`[MODEL-AST-002]` 的三档）。
///
/// 用**穷尽** `match` 而不是 `{:?}`：派生的 `Debug` 会在有人给枚举加变体时静默变成
/// 一个用户可见的英文标识符（`"Int24"`），那时没有任何判据会红。
#[must_use]
fn bit_depth_display(bit_depth: yeban_model::project::BitDepth) -> String {
    use yeban_model::project::BitDepth;
    match bit_depth {
        BitDepth::Int16 => "16-bit int".to_owned(),
        BitDepth::Int24 => "24-bit int".to_owned(),
        BitDepth::Float32 => "32-bit float".to_owned(),
    }
}

/// 一次投影的完整结果：界面侧**唯一**的数据来源。
#[derive(Debug, Clone, PartialEq)]
pub struct ViewState {
    /// 工程身份规范文本。
    pub project_id: String,
    /// 窗口标题（`YebanProjectV1::title`）。
    pub title: String,
    /// 工程速度（`YebanProjectV1::bpm`）。
    pub bpm: f64,
    /// 速度显示文本（`{:.2}`，与 `.slint` 的 `bpm-display` 对齐）。
    pub bpm_display: String,
    /// 速度的整数千分之一（给需要整数的判据 / 无障碍值用，避免界面层碰浮点）。
    pub bpm_millis: u32,
    /// 拍号分子。
    pub time_signature_numerator: u8,
    /// 拍号分母。
    pub time_signature_denominator: u8,
    /// 拍号显示文本（`"4/4"`）。
    pub time_signature_display: String,
    /// 状态栏「音频配置」那一格的整串文本（台账 R9）。
    ///
    /// 由 [`audio_config_display`] 从 `project.audio_config` 算出，**在投影期算一次**
    /// ⇒ 每一帧的状态栏读数是这份投影的读数，宿主不再自己拼第二份。
    pub audio_config_display: String,
    /// 每四分音符 tick 数（恒为 [`PPQ`]）。
    pub ppq: u64,
    /// 缩放：每逻辑像素多少 tick。
    pub ticks_per_pixel: u64,
    /// 一小节的 tick 数。
    pub bar_length_ticks: u64,
    /// 非主总线轨道（`BTreeMap` 键序 ⇒ 身份升序）。
    pub tracks: Vec<TrackView>,
    /// 主总线轨道（`master_bus_track_id`；工程无轨道时为 `None`）。
    pub master: Option<TrackView>,
    /// 全部剪辑摆放（轨道序 → 摆放身份序）。
    pub clips: Vec<ClipView>,
    /// 全部段落（身份升序）。
    pub sections: Vec<SectionView>,
    /// 全部场景（身份升序）。
    pub scenes: Vec<SceneView>,
    /// 素材池（`clip_pool`）的行投影（身份升序 = `BTreeMap` 键序）。
    ///
    /// ⭐ 任务单 `#133`：这是素材池面板的唯一事实源。面板按
    /// [`Self::asset_pool_names`] / [`Self::asset_pool_kinds`] /
    /// [`Self::asset_pool_details`] 三个平行数组取数（同索引集）；空池投影成空 `Vec`
    /// ⇒ 界面显示空态，而不是一片空白。
    pub asset_pool: Vec<AssetPoolRowView>,
    /// 设备机架展示的是**哪条轨**（`"轨 鼓 · 设备链"`；没有非主总线轨时是
    /// `"没有可显示的音轨"`）。
    ///
    /// ⚠ 今天**没有**"选中轨"这一状态（`grep -rn 'selected_track\|active-track' crates/yeban-app`
    /// 命中 0；台账第 236 轮已裁决"the current track does not exist"）⇒ 本投影把
    /// [`device_rack_track`] 的**唯一规则**（身份序第一条非主总线轨）写成可见的标题，
    /// ⛔ 不假装那是用户选中的轨道。
    pub device_track_label: String,
    /// 当前展示轨的设备链（`TrackV3::devices` 的逐条投影）—— 设备机架的**唯一**事实源。
    ///
    /// ⭐ 任务单 `#157`：空链投影成空 `Vec` ⇒ 界面显示空态（`"无设备（点上方「装 EQ」或「装混响」）"`，
    /// `#163` 起这句提示指向真实的装置按钮），而不是旧版那三条编造的设备名。
    pub devices: Vec<DeviceRowView>,
    /// ⭐ 任务单 `#173`：**主总线**（`master_bus_track_id`）上的设备链投影 —— 设备架
    /// 面板的**第二段（只读）**事实源。
    ///
    /// 为什么单独一段而不是并进 [`Self::devices`]：可编辑面（旁通 / EQ 滑块 /
    /// `InsertDevice` / `RemoveDevice`）的寻址单位是"展示轨的 `devices[slot_index]`"
    /// （[`device_rack_track`] **排除**主总线）⇒ 把 master 的设备并进同一个数组会让
    /// 那些 Op 指错轨。本字段只让面板**看得见**"主总线上有什么、已识别什么"。
    ///
    /// ⚠ 引擎侧（`rt.rs` 的 3a⁗⁗）与离线侧（`yeban-render` 的 `render_node`）在
    /// `#173` 起都**真的处理**主总线的设备 ⇒ 这里列出的每一台都不是装饰。
    ///
    /// ## 追加修订段（任务单 `#180`，2026-10-18）—— 只修订"只读"这一句
    ///
    /// 上面两句（"第二段（只读）事实源" / "本字段只让面板看得见"）是 `#173` 的**历史
    /// 状态**，原文保留。`#180` 起主总线这一段**可增删**：装 / 删走
    /// [`master_device_insert_ops`] / [`master_device_remove_ops`]（`track_id =
    /// master_bus_track_id`，**按身份**寻址），界面手势是 `master-device-insert-eq` /
    /// `master-device-insert-reverb` / `master-device-remove`。
    ///
    /// ⛔ **仍然分开两段**的原因没有变（这是本条修订的边界）：两段的寻址单位是**各自的
    /// 那条轨** —— 第一段是 [`device_rack_track`]（排除主总线），这一段是
    /// [`master_device_track`]。把两段并成一个数组才会让 `slot_index` 指错轨；
    /// "可编辑"不等于"可以共用一个数组"。旁通 / EQ 滑块**仍只属于第一段**。
    pub master_devices: Vec<DeviceRowView>,
    /// ⭐ 任务单 `#180`：主总线设备链那一栏的标题（`"主总线 Master · 设备链"`；没有主总线时
    /// 是 [`MASTER_DEVICE_INSERT_NO_MASTER_REASON`] 的**逐字**原因）。
    ///
    /// 与 [`Self::device_track_label`] 的差别只有主语：那一段是展示轨，这一段是
    /// `master_bus_track_id` 指的主总线轨。⛔ 界面不拼第二份标题。
    pub master_device_label: String,
    /// ⭐ 任务单 `#180`："给主总线装 EQ / 装混响"此刻能不能用（工程里得有一条主总线）。
    ///
    /// `false` ⇒ 两个按钮禁用并逐字显示 [`Self::master_device_insert_reason`]，⛔ 不静默。
    /// ⚠ 它与 [`Self::device_insert_available`] **分开**两格：有展示轨不代表有主总线。
    pub master_device_insert_available: bool,
    /// ⭐ 任务单 `#180`："给主总线装设备"不可用时**逐字**写出的原因（可用时是空串）。
    ///
    /// 取值来自 [`MASTER_DEVICE_INSERT_NO_MASTER_REASON`]。
    pub master_device_insert_reason: String,
    /// 当前展示轨上**第一格** `eq_low_gain`（值域来自契约）；没有 ⇒ `None`。
    ///
    /// ⚠ 这是本切片**唯一**可编辑的设备参数（"最小可编辑"）。`None` 时界面禁用滑块
    /// 并如实写出"本轨没有 `eq_low_gain`"，⛔ 不落一个假的可编辑控件。
    pub eq_low_gain: Option<DeviceEqView>,
    /// "装 EQ / 装混响"此刻能不能用（工程里得有一条展示轨）。
    ///
    /// ⭐ 任务单 `#163`：两个按钮共用这一格 —— 前置条件逐字相同（展示轨存在）。
    /// `false` ⇒ 按钮禁用并逐字显示 [`Self::device_insert_reason`]，⛔ 不静默。
    pub device_insert_available: bool,
    /// "装设备"不可用时**逐字**写出的原因（可用时是空串）。
    ///
    /// 取值来自 [`DEVICE_INSERT_NO_TRACK_REASON`]。
    pub device_insert_reason: String,
    /// "建空的 MIDI 片段"此刻能不能用（工程里得有一条展示轨）。
    ///
    /// `false` ⇒ 按钮禁用并逐字显示 [`Self::clip_create_reason`]，⛔ 不静默。
    pub clip_create_available: bool,
    /// "建片段"不可用时**逐字**写出的原因（可用时是空串）。
    ///
    /// 取值来自 [`CLIP_CREATE_NO_TRACK_REASON`]。
    pub clip_create_reason: String,
    /// ⭐ 任务单 `#164`：展示轨当前乐器的显示文本 —— `"乐器：<sfz 路径>"` / `"未装乐器"`。
    ///
    /// 路径逐字来自 `TrackV3::sampler.path`（[`instrument_label`] 是唯一格式化实现）。
    pub instrument_label: String,
    /// 界面一键可装的 SFZ 乐器**显示名**（与 [`installable_builtin_instruments`]
    /// 同序、同长度）。
    ///
    /// ⭐ 它是按钮行的唯一事实源；点第 `i` 个按钮只上报下标，路径由宿主从
    /// [`builtin_instrument`] 求 —— ⛔ 界面不持有第二份路径表。
    /// ⚠ 任务单 `#165` 起这里是**按样本字节过滤后**的清单：今天为空（0 个样本字节）
    /// ⇒ 控件树里 `instrument-install-*-button` 一个都不存在。
    pub instrument_names: Vec<String>,
    /// "装乐器"此刻能不能用（工程里得有展示轨 **且** 内置清单里有可装乐器）。
    ///
    /// `false` ⇒ 按钮禁用并逐字显示 [`Self::instrument_install_reason`]，⛔ 不静默。
    pub instrument_install_available: bool,
    /// "装乐器"不可用时**逐字**写出的原因（可用时是空串）。
    ///
    /// 取值来自 [`INSTRUMENT_INSTALL_NO_TRACK_REASON`]（无展示轨）或
    /// [`INSTRUMENT_INSTALL_NO_BUILTIN_REASON`]（清单为空 —— 今天这一档）。
    pub instrument_install_reason: String,
    /// 当前展示轨（[`send_strip_track`]，与设备机架**同一条轨**）的 `SendToAux` 边逐条投影
    /// —— 发送条的**唯一**事实源。
    ///
    /// ⭐ 任务单 `#159`：空 ⇒ 界面显示空态（`"无发送"`）＋ 条目数 `0`，⛔ 不是一片空白，
    /// 也⛔ 不是编造的发送。
    pub sends: Vec<SendRowView>,
    /// "新建发送"此刻能不能用：展示轨存在**且**工程里有一条 `TrackKind::AuxReturn` 轨
    /// **且**展示轨到那条 aux 还没有同源同目的地的 `SendToAux` 边。
    ///
    /// `false` ⇒ 界面禁用按钮并逐字显示 [`Self::send_create_reason`]，⛔ 不静默。
    pub send_create_available: bool,
    /// "新建发送"不可用时**逐字**写出的原因（可用时是空串）。
    ///
    /// 三个取值来自 [`SEND_CREATE_NO_TRACK_REASON`] / [`SEND_CREATE_NO_AUX_REASON`] /
    /// [`SEND_CREATE_DUPLICATE_REASON`]。
    pub send_create_reason: String,
    /// 工程轨数（**含主总线**）的显示文本：`"共 {n} 条轨"`。
    ///
    /// ⭐ 任务单 `#160`：它是"建轨 / 删轨真的发生了"在**同一面板**里的可见读数
    /// —— 新轨按 `EntityId` 排序通常落在末尾，展示轨（身份序第一条非主总线轨）可能
    /// 不变，所以只靠 `device_track_label` 看不出轨数变了。
    pub track_count_label: String,
    /// 建轨此刻能不能用（工程里得有一条 `TrackKind::Master` 轨）。
    ///
    /// `false` ⇒ 三个建轨按钮禁用，且 [`Self::master_bus_label`] **就是**逐字原因。
    pub track_create_available: bool,
    /// 主总线说明（`"主总线：Master（不可删除）"`；没有主总线时是建轨的逐字原因）。
    pub master_bus_label: String,
    /// "删除展示轨"此刻能不能用（展示轨存在）。
    ///
    /// ⭐ 任务单 `#160`：目标**只**是展示轨（身份序第一条非主总线轨）⇒ 主总线不可删；
    /// `false` ⇒ 界面禁用按钮并逐字显示 [`Self::track_delete_reason`]。
    pub track_delete_available: bool,
    /// "删除展示轨"不可用时**逐字**写出的原因（可用时是空串）。
    ///
    /// 两个取值来自 [`TRACK_DELETE_NO_TRACK_REASON`] / [`TRACK_DELETE_MASTER_REASON`]。
    pub track_delete_reason: String,
    /// 全部 MIDI 音符（片段池键序 → 音符键序，去重保序），**含**投影算出的 x / y / 宽 / 车道。
    ///
    /// 这是音符族唯一的权威投影产物；[`Self::note_ulids`] / [`Self::note_velocities`] 与
    /// [`Self::note_positions`] 等平行数组都由它派生（判据
    /// `note_parallel_arrays_agree_with_the_rich_projection` 逐项对账）。
    pub notes: Vec<NoteView>,
    /// `[UI-NOTE-001]` 步骤 ②：按 `x` 升序排列的 `notes` 下标 —— 让视口查询走**二分**而不是全表扫描。
    ///
    /// 只按**水平轴**建索引：视口只在 tick/x 上过滤（纵向是固定 16 条泳道, 见账本第 202/203 轮）。
    /// 这**不是** R-Tree；换真正的二维索引时应**替换**本字段而不是并存（同一契约下判据不变）。
    pub note_x_order: Vec<u32>,
    /// 片段池里全部 MIDI 音符的身份（片段序 → 音符身份序，去重保序）。
    ///
    /// 它进 `.slint` 的 `note-{ulid}-rect`：这些 ID 必须来自**工程里的音符实体**，
    /// 不能是手写常量。
    pub note_ulids: Vec<String>,
    /// 与 [`Self::note_ulids`] **逐个对齐**的音符力度（0.0–1.0 归一化）。
    ///
    /// 原始值是 `MidiNote::velocity`（0–127 整数，`MODEL-AST-005`）；归一化是**显示**需要的
    /// 形态（力度泳道按比例画柱高），量化与吸附仍归模型层。
    pub note_velocities: Vec<f32>,
    /// 标尺的小节线 x 位置（逻辑像素），至少 [`MIN_BAR_COUNT`] 条。
    pub bar_positions: Vec<f32>,
    /// 全部自动化泳道（非主总线轨道 → `AutomationTarget::Ord` 序），**含**折线顶点与
    /// `Path` 指令（见 [`crate::automation::AutomationLaneView`]）。
    ///
    /// 这是自动化族唯一的权威投影产物；注入 `.slint` 的 9 个平行数组全部由它派生
    /// （判据 `automation::vertices_follow_the_points_in_tick_order` 逐项对账）。
    /// 每一个顶点的值都来自模型的那一个求值入口 —— 本层没有插值实现
    /// （模块文档与判据 ⑧）。
    pub automation_lanes: Vec<crate::automation::AutomationLaneView>,
}

impl ViewState {
    /// 投影一个工程（默认缩放 [`DEFAULT_TICKS_PER_PIXEL`]、**默认**布局）。
    ///
    /// # Errors
    ///
    /// 见 [`BridgeError`]。空工程**不是**错误 —— 它投影成空视图。
    pub fn from_project(project: &YebanProjectV1) -> Result<Self, BridgeError> {
        Self::from_project_with_zoom(project, DEFAULT_TICKS_PER_PIXEL)
    }

    /// 投影一个工程并指定缩放（`ticks_per_pixel`），布局取默认。
    ///
    /// # Errors
    ///
    /// 见 [`BridgeError`]。
    pub fn from_project_with_zoom(
        project: &YebanProjectV1,
        ticks_per_pixel: u64,
    ) -> Result<Self, BridgeError> {
        Self::from_project_with_zoom_and_cursor(
            project,
            ticks_per_pixel,
            crate::automation::AUTOMATION_CURSOR_TICK,
        )
    }

    /// 投影一个工程、指定缩放，并指定自动化"当前值"的求值 tick（布局取默认）。
    ///
    /// 走带位置属**会话运行态**（`[MODEL-ISO-001]` 的第二层），不在 `YebanProjectV1` 里，
    /// 因此静态投影固定用 [`crate::automation::AUTOMATION_CURSOR_TICK`]；需要真实播放头时
    /// 走这个入口（一条后续线的接线动作，本线只提供可被调用的形状）。
    ///
    /// # Errors
    ///
    /// 见 [`BridgeError`]。
    pub fn from_project_with_zoom_and_cursor(
        project: &YebanProjectV1,
        ticks_per_pixel: u64,
        cursor_tick: u64,
    ) -> Result<Self, BridgeError> {
        Self::from_project_with_zoom_cursor_and_layout(
            project,
            ticks_per_pixel,
            cursor_tick,
            &TrackHeightLayout::default(),
        )
    }

    /// 投影一个工程并给出**轨道高度布局**（每轨基准 + 全局乘子），缩放取默认
    /// [`DEFAULT_TICKS_PER_PIXEL`]。
    ///
    /// 这是 S1 的生产入口：宿主（`host::track_height_layout` 是唯一读点）从会话/视图态
    /// 造出布局，投影在这里把 `clamp(base × percent / 100)` 算成每行 `stride`
    /// （**唯一**的夹紧点）。
    ///
    /// # Errors
    ///
    /// 见 [`BridgeError`]。
    pub fn from_project_with_layout(
        project: &YebanProjectV1,
        layout: &TrackHeightLayout,
    ) -> Result<Self, BridgeError> {
        Self::from_project_with_zoom_cursor_and_layout(
            project,
            DEFAULT_TICKS_PER_PIXEL,
            crate::automation::AUTOMATION_CURSOR_TICK,
            layout,
        )
    }

    /// 投影一个工程、给出**横向缩放**与**轨道高度布局**（光标 tick 取默认）。
    ///
    /// 它与 [`Self::from_project_with_layout`] 是同一件事的两个参数面：那个用默认缩放
    /// （[`DEFAULT_TICKS_PER_PIXEL`]），这个让调用方给缩放。两者都转发到
    /// [`Self::from_project_with_zoom_cursor_and_layout`] —— **不存在第三条投影实现**。
    ///
    /// 生产调用点是宿主：`host::project_with_view_state`（窗口上的
    /// `roll-ticks-per-pixel` + `track-height-*` 三个视图态属性 → 一次投影）。
    ///
    /// # Errors
    ///
    /// 见 [`BridgeError`]。
    pub fn from_project_with_zoom_and_layout(
        project: &YebanProjectV1,
        ticks_per_pixel: u64,
        layout: &TrackHeightLayout,
    ) -> Result<Self, BridgeError> {
        Self::from_project_with_zoom_cursor_and_layout(
            project,
            ticks_per_pixel,
            crate::automation::AUTOMATION_CURSOR_TICK,
            layout,
        )
    }

    /// 投影的本体：缩放 + 光标 tick + 轨道高度布局**全部**显式给全。
    ///
    /// 另外三个入口（[`Self::from_project`] / [`Self::from_project_with_zoom`] /
    /// [`Self::from_project_with_zoom_and_cursor`]）都转发到这里并传**默认布局**，
    /// 因此"默认路径"与"自定义布局路径"走的是同一段代码 —— 不存在第二条投影实现。
    ///
    /// # Errors
    ///
    /// 见 [`BridgeError`]。
    pub fn from_project_with_zoom_cursor_and_layout(
        project: &YebanProjectV1,
        ticks_per_pixel: u64,
        cursor_tick: u64,
        layout: &TrackHeightLayout,
    ) -> Result<Self, BridgeError> {
        if ticks_per_pixel == 0 {
            return Err(BridgeError::ZeroTicksPerPixel);
        }

        let bar_ticks = bar_length_ticks(project.time_signature)?;

        // 行几何：**投影算一次**（前缀和），包头 / 车道 / 剪辑 / 自动化带全部读它。
        // `ADR-0004` S0 把这三处收敛到一条 `track_rows`；S1 让行高来自布局参数
        // （`clamp(每轨基准 × 全局乘子 / 100)`，唯一的夹紧点就在这里的那一次调用里）。
        let rows = track_rows_with_layout(project, layout);

        let mut tracks: Vec<TrackView> = Vec::new();
        let mut master: Option<TrackView> = None;
        let mut clips: Vec<ClipView> = Vec::new();

        // `BTreeMap::values()` 是**身份升序**：跨进程、跨重启、跨机器都给出同一顺序
        // （红线 4 的确定性要求）。这里刻意不排序 —— 排序会掩盖"集合被换成 HashMap"。
        for track in project.tracks.values() {
            if track.id == project.master_bus_track_id {
                master = Some(track_view(track, 0, true, None));
                continue;
            }
            let index = tracks.len();
            let row = rows.get(index).copied();
            for placement in track.clips.values() {
                clips.push(clip_view(
                    project,
                    placement,
                    index,
                    &track.name,
                    ticks_per_pixel,
                    row,
                )?);
            }
            tracks.push(track_view(track, index, false, row));
        }
        for (index, clip) in clips.iter_mut().enumerate() {
            clip.index = index;
        }

        let mut sections = Vec::with_capacity(project.sections.len());
        for (index, section) in project.sections.values().enumerate() {
            sections.push(section_view(section, index, ticks_per_pixel)?);
        }

        let mut scenes = Vec::with_capacity(project.scenes.len());
        for (index, scene) in project.scenes.values().enumerate() {
            scenes.push(scene_view(scene, index));
        }

        // 素材池：`clip_pool` 的**逐条**投影（身份升序）。⛔ 不在这里筛掉 MIDI 条目 ——
        // 面板的"类型"一列要把两种内容都如实标出来（音频 / MIDI）。
        let asset_pool = asset_pool_rows(project);

        // 设备机架（任务单 `#157`）：展示轨由 [`device_rack_track`] **唯一**决定
        // （身份序第一条非主总线轨 —— 与 `tracks[0]` 是同一条轨，判据逐项对账）。
        // 设备逐条来自 `TrackV3::devices`，"已识别/未识别"走 `effect_params::recognise`。
        let (device_track_label, devices, eq_low_gain) =
            device_rack_projection(device_rack_track(project));
        // ⭐ 任务单 `#173`：主总线的设备链 —— **只读**的第二段（可编辑面仍只属于展示轨，
        // 见 `ViewState::master_devices` 的文档）。
        let master_devices = master_device_rows(project);
        // ⭐ 任务单 `#180`：主总线设备链的标题 ＋ "装设备"的可用性 / 逐字原因。
        // 三者与上面那段投影**同一份**主语（`master_bus_track_id`）⇒ 标题、列表、
        // 可用性不可能互相指错轨。
        let master_device_label = master_device_track_label(project);
        let (master_device_insert_available, master_device_insert_reason) =
            master_device_insert_availability(project);

        // ⭐ 任务单 `#163`：设备增删 ＋ 建空片段的可用性（与投影**同一份**口径）。
        let (device_insert_available, device_insert_reason) = device_insert_availability(project);
        let (clip_create_available, clip_create_reason) = clip_create_availability(project);

        // ⭐ 任务单 `#164`：一键装乐器的投影（与设备机架**同一条**展示轨）。
        // 可用性 / 逐字原因与手势共用同一份口径；`instrument_names` 是按钮行的唯一清单。
        let (instrument_install_available, instrument_install_reason) =
            instrument_install_availability(project);
        let instrument_label = instrument_label(device_rack_track(project));
        let instrument_names = instrument_names();

        // 发送条（任务单 `#159`）：与设备机架**同一条**展示轨（[`send_strip_track`]）。
        // 条目逐条来自 `RoutingGraph::edges` 的 `SendToAux` 出边；目的地名字来自 `tracks`；
        // "新建发送"能不能用由 [`send_create_availability`] **唯一**决定（任务单 `#160`
        // 起还要求"源 → 目的地"没有重复的 `SendToAux` 边）。
        let (sends, send_create_available, send_create_reason) =
            send_strip_projection(project, send_strip_track(project));

        // 轨 / 路由（任务单 `#160`）：轨数读数 ＋ 建轨 / "删除展示轨"的可用性。
        // 删轨的目标只认展示轨（[`track_delete_target`] = 身份序第一条非主总线轨）
        // ⇒ 主总线永远不可删。
        let (track_delete_available, track_delete_reason) = track_delete_availability(project);
        let track_create_available = track_create_available(project);
        let master_bus_label = master_bus_label(project);

        // MIDI 音符：按片段池键序 → 音符键序，去重但保持首次出现顺序。
        // 身份进 `note-{ulid}-rect`；力度进力度泳道；**位置**由 start_tick / pitch 经整数
        // 变换得到（复用 `tick_to_px` 与 `pitch_lane`，不在别处再写一套换算）。
        let mut notes: Vec<NoteView> = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for entry in project.clip_pool.values() {
            let Some(entry_notes) = entry.content.notes() else {
                continue;
            };
            for note in entry_notes.values() {
                let text = note.id.to_canonical_string();
                if !seen.insert(text.clone()) {
                    continue;
                }
                let index = notes.len();
                notes.push(note_view(entry, note, index, ticks_per_pixel)?);
            }
        }
        // 平行数组一律由 `notes` 派生 —— 只有一个事实源，判据再钉住它们逐项一致。
        let note_ulids: Vec<String> = notes.iter().map(|note| note.id.clone()).collect();
        let note_velocities: Vec<f32> = notes.iter().map(|note| note.velocity_normalized).collect();

        // 标尺：覆盖工程实际长度，至少 `MIN_BAR_COUNT` 条。
        let mut end_tick = 0_u64;
        for clip in &clips {
            end_tick = end_tick.max(clip.end_tick);
        }
        for section in &sections {
            end_tick = end_tick.max(section.end_tick);
        }
        let bars_in_project = end_tick.div_ceil(bar_ticks.max(1));
        let bar_count =
            usize::try_from(bars_in_project.max(MIN_BAR_COUNT as u64)).unwrap_or(MIN_BAR_COUNT);
        let mut bar_positions = Vec::with_capacity(bar_count);
        for bar in 0..bar_count {
            let tick = u64::try_from(bar)
                .ok()
                .and_then(|bar| bar.checked_mul(bar_ticks))
                .ok_or(BridgeError::BarLengthOverflow)?;
            bar_positions.push(as_px(tick_to_px(tick, ticks_per_pixel)?));
        }

        Ok(Self {
            project_id: project.id.to_canonical_string(),
            title: project.title.clone(),
            bpm: project.bpm,
            bpm_display: format!("{:.2}", project.bpm),
            bpm_millis: bpm_millis(project.bpm),
            time_signature_numerator: project.time_signature.numerator,
            time_signature_denominator: project.time_signature.denominator,
            time_signature_display: format!(
                "{}/{}",
                project.time_signature.numerator, project.time_signature.denominator
            ),
            audio_config_display: self::audio_config_display(
                project.audio_config.sample_rate,
                project.audio_config.bit_depth,
            ),
            ppq: PPQ,
            ticks_per_pixel,
            bar_length_ticks: bar_ticks,
            tracks,
            master,
            clips,
            sections,
            scenes,
            asset_pool,
            device_track_label,
            devices,
            master_devices,
            master_device_label,
            master_device_insert_available,
            master_device_insert_reason,
            eq_low_gain,
            device_insert_available,
            device_insert_reason,
            clip_create_available,
            clip_create_reason,
            instrument_label,
            instrument_names,
            instrument_install_available,
            instrument_install_reason,
            sends,
            send_create_available,
            send_create_reason,
            track_count_label: format!("共 {} 条轨", project.tracks.len()),
            track_create_available,
            master_bus_label,
            track_delete_available,
            track_delete_reason,
            note_x_order: {
                // 一次性建索引：按 `x` 升序。`total_cmp` 对 NaN 也有全序, 不会 panic。
                let mut order: Vec<u32> = (0..notes.len() as u32).collect();
                order.sort_by(|a, b| notes[*a as usize].x.total_cmp(&notes[*b as usize].x));
                order
            },
            notes,
            note_ulids,
            note_velocities,
            bar_positions,
            automation_lanes: crate::automation::project_lanes_with_rows(
                project,
                &rows,
                ticks_per_pixel,
                cursor_tick,
            )?,
        })
    }

    /// 演示工程（`src/scene.rs` 的夹具事实源）的投影。
    ///
    /// 这个夹具**就是**一个 `YebanProjectV1` —— 界面不再有第二份硬编码数据。
    ///
    /// # Panics
    ///
    /// 仅当本文件里的演示夹具被改成非法时 panic（夹具是编译期常量，属编程错误）。
    #[must_use]
    pub fn demo() -> Self {
        Self::from_project(&demo_project()).expect("演示夹具必须能投影")
    }

    /// 空工程的投影：零轨道、零剪辑、零段落、零场景。
    ///
    /// UI 侧在"工程投影失败"时的兜底值 —— 画一个空界面，而不是画上一次的残留。
    ///
    /// # Panics
    ///
    /// 不会 panic（空工程是合法输入，见 `empty_project_projects_to_an_empty_view`）。
    #[must_use]
    pub fn empty() -> Self {
        Self::from_project(&YebanProjectV1::default()).expect("空工程必须能投影")
    }

    /// 走带时间码的格式化网格（由**工程拍号**推出的 `[TimecodeGrid]`）。
    ///
    /// 这是"tick → `小节.拍.tick`"的唯一语义入口：引擎给位置，工程给拍号，
    /// 换算在 [`TimecodeGrid`] 里。若模型层拒绝这个拍号 ⇒ `None`
    /// （[`timecode_for_ticks`] 会如实退回 tick 文本，不按 4/4 编造读数）。
    #[must_use]
    pub fn timecode_grid(&self) -> Option<TimecodeGrid> {
        TimecodeGrid::from_time_signature(
            self.time_signature_numerator,
            self.time_signature_denominator,
        )
    }

    /// 走带位置（tick）在这份投影下的时间码读数 —— 判据与宿主共用同一个入口，
    /// 因此"界面上显示的读数"与"投影算出的读数"不可能各说各话。
    #[must_use]
    pub fn timecode_at(&self, ticks: u64) -> String {
        timecode_for_ticks(self.timecode_grid(), ticks)
    }

    /// 非主总线轨道的名称（`[UI-GRID-001]` 轨道包头列的文本源）。
    #[must_use]
    pub fn track_names(&self) -> Vec<String> {
        self.tracks.iter().map(|track| track.name.clone()).collect()
    }

    /// 每条非主总线轨道的**身份**（[`TrackView::id`]，26 字符 `EntityId` 规范文本）——
    /// 与 `track-names` / `track-ys` / `track-heights` **同索引集**。
    ///
    /// 存在的理由只有一个：每轨高度覆盖按**身份**键控（`ADR-0004` S1 的
    /// [`TrackHeightLayout`]），而界面上的拖拽手势只能报告**下标**
    /// （`arrangement_view.slint` 的 `track-height-grab(track_index, …)`）⇒ 宿主需要一格
    /// "下标 → 身份"的查表。
    ///
    /// 宿主**不**拿别的快照去猜身份：这个数组与行几何由**同一个** `host::apply_view`
    /// 一次性注入，因此换工程之后与行几何同步更新（不是一份会过期的副本）。
    #[must_use]
    pub fn track_ids(&self) -> Vec<String> {
        self.tracks.iter().map(|track| track.id.clone()).collect()
    }

    /// 每条非主总线轨道的**行顶沿 y**（逻辑像素，前缀和）—— `.slint` 直接画它。
    ///
    /// 与 `track-names` **同索引集**（都来自 `self.tracks`）：`.slint` 的
    /// `for track_name[track_index] in root.tracks` 与 `for _[lane_index] in root.tracks`
    /// 用的就是这个下标。判据 `automation::tests::lane_element_ids_match_the_slint_template`
    /// 断言 `.slint` 读的是这两个数组、且**不再**做任何行算术。
    #[must_use]
    pub fn track_ys(&self) -> Vec<f32> {
        self.tracks.iter().map(|track| track.y).collect()
    }

    /// 每条非主总线轨道的**行矩形高**（逻辑像素）—— `.slint` 直接画它。
    #[must_use]
    pub fn track_heights(&self) -> Vec<f32> {
        self.tracks.iter().map(|track| track.height).collect()
    }

    /// 非主总线轨道的音量显示文本（`TrackV3::volume_db` 的 `{:.1}` 形态）。
    #[must_use]
    pub fn track_volumes(&self) -> Vec<String> {
        self.tracks
            .iter()
            .map(|track| track.volume_display.clone())
            .collect()
    }

    /// 非主总线轨道的静音位（进 `accessible-checked`）。
    #[must_use]
    pub fn track_mutes(&self) -> Vec<bool> {
        self.tracks.iter().map(|track| track.mute).collect()
    }

    /// 非主总线轨道的独奏位（进 `accessible-checked`）。
    #[must_use]
    pub fn track_solos(&self) -> Vec<bool> {
        self.tracks.iter().map(|track| track.solo).collect()
    }

    /// ⭐ 非主总线轨道的 **SFZ 采样器路径文本**（`track-sampler-paths` 的注入源）。
    ///
    /// 口径（一条纯映射，判据 `track_sampler_paths_maps_none_to_the_empty_text` 钉住）：
    ///
    /// | `TrackV3::sampler` | 这里给 `.slint` 的文本 | 含义 |
    /// | :--- | :--- | :--- |
    /// | `None` | `""` | 这条轨**不用**采样器（一等状态；界面上是空输入框） |
    /// | `Some(SamplerBinding { path })` | `path` 原文 | 用户/模型写下的那条路径 |
    ///
    /// ⚠ **`None` 与 `Some("")` 在这里落到同一个文本**（都是 `""`），这是刻意的：
    /// 界面上的文本输入面只有"文本"这一维，而 `Clear`（`None`）与"输入框被清空但没提交"
    /// 在视觉上本来就该一样。两者的区别住在**模型**里（`Option`），⛔ 不在文本里 ——
    /// 因此 `Set` 收到空文本时**不提交**（见 `host::commit_sampler_binding`），
    /// 解绑只有一个入口：`Clear`。
    #[must_use]
    pub fn track_sampler_paths(&self) -> Vec<String> {
        self.tracks
            .iter()
            .map(|track| track.sampler_path.clone().unwrap_or_default())
            .collect()
    }

    /// 段落名（章节卡片文本源）。
    #[must_use]
    pub fn section_names(&self) -> Vec<String> {
        self.sections
            .iter()
            .map(|section| section.name.clone())
            .collect()
    }

    /// 场景名（Session 视图文本源）。
    #[must_use]
    pub fn scene_names(&self) -> Vec<String> {
        self.scenes.iter().map(|scene| scene.name.clone()).collect()
    }

    /// 素材池每一行的显示名（与下面两个数组**同索引集**）。
    ///
    /// ⭐ 任务单 `#133`：`.slint` 面板的 `asset-pool-names` 就是它。
    #[must_use]
    pub fn asset_pool_names(&self) -> Vec<String> {
        self.asset_pool.iter().map(|row| row.name.clone()).collect()
    }

    /// 素材池每一行的内容类型（`"音频"` / `"MIDI"`）。
    #[must_use]
    pub fn asset_pool_kinds(&self) -> Vec<String> {
        self.asset_pool.iter().map(|row| row.kind.clone()).collect()
    }

    /// 素材池每一行**拿得到**的字段（音频：字节数／资产前缀／增益；MIDI：音符条数）。
    ///
    /// ⛔ 这一格不补算任何读数：字节数只来自 `project.assets` 的 `byte_len`，
    /// 没有登记就写「字节数未知」（见 [`AssetPoolRowView`]）。
    #[must_use]
    pub fn asset_pool_details(&self) -> Vec<String> {
        self.asset_pool
            .iter()
            .map(|row| row.detail.clone())
            .collect()
    }

    /// 素材池每一行的整串无障碍标签（注册表与运行时两侧共用）。
    #[must_use]
    pub fn asset_pool_labels(&self) -> Vec<String> {
        self.asset_pool
            .iter()
            .map(|row| row.label.clone())
            .collect()
    }

    /// 设备机架每一格的显示名（与下面三个数组**同索引集**）。
    ///
    /// ⭐ 任务单 `#157`：`.slint` 的 `device-names` 就是它 —— 旧版那三条
    /// `"EQ 4 段"/"Compressor"/"Space Reverb"` 已删除（见 `device_rack.slint` 的修订段）。
    #[must_use]
    pub fn device_names(&self) -> Vec<String> {
        self.devices.iter().map(|row| row.name.clone()).collect()
    }

    /// 设备机架每一格的**设备身份**（26 字符 `EntityId` 文本，与上面数组**同索引集**）。
    ///
    /// ⭐ 任务单 `#163`：删除按钮用它寻址 —— 与发送条的 `send-edge-ids` 同一条纪律
    /// （身份不会随行下标漂移），宿主在提交前按身份重新从权威工程查一次。
    #[must_use]
    pub fn device_ids(&self) -> Vec<String> {
        self.devices
            .iter()
            .map(|row| row.device_id.clone())
            .collect()
    }

    /// 设备机架每一格的旁通位（`DeviceDefinition::bypassed`，⛔ 不是界面状态）。
    #[must_use]
    pub fn device_bypassed(&self) -> Vec<bool> {
        self.devices.iter().map(|row| row.bypassed).collect()
    }

    /// 设备机架每一格的**已识别**参数读数（空串 = 这一台没有已识别参数）。
    #[must_use]
    pub fn device_recognised(&self) -> Vec<String> {
        self.devices
            .iter()
            .map(|row| row.recognised.clone())
            .collect()
    }

    /// 设备机架每一格的**未识别**参数读数（空串 = 全部参数都被引擎认）。
    #[must_use]
    pub fn device_unrecognised(&self) -> Vec<String> {
        self.devices
            .iter()
            .map(|row| row.unrecognised.clone())
            .collect()
    }

    /// ⭐ 任务单 `#180`：**主总线**设备链每一格的显示名（与下面三个数组**同索引集**）。
    ///
    /// 口径与 [`Self::device_names`] **逐字相同**，只是主语换成 [`Self::master_devices`]
    /// ⇒ 两段面板不会各说一套（同一份 [`device_rack_projection`]）。
    #[must_use]
    pub fn master_device_names(&self) -> Vec<String> {
        self.master_devices
            .iter()
            .map(|row| row.name.clone())
            .collect()
    }

    /// ⭐ 任务单 `#180`：主总线每一格的**设备身份**（26 字符文本）—— 删除按钮只报它，
    /// ⛔ 不报行下标（下标会随设备链增删漂移，身份不会）。
    #[must_use]
    pub fn master_device_ids(&self) -> Vec<String> {
        self.master_devices
            .iter()
            .map(|row| row.device_id.clone())
            .collect()
    }

    /// ⭐ 任务单 `#180`：主总线每一格的**已识别**参数读数（空串 = 这一台没有）。
    #[must_use]
    pub fn master_device_recognised(&self) -> Vec<String> {
        self.master_devices
            .iter()
            .map(|row| row.recognised.clone())
            .collect()
    }

    /// ⭐ 任务单 `#180`：主总线每一格的**未识别**参数读数（空串 = 全部参数都被引擎认）。
    #[must_use]
    pub fn master_device_unrecognised(&self) -> Vec<String> {
        self.master_devices
            .iter()
            .map(|row| row.unrecognised.clone())
            .collect()
    }

    /// 本轨**有没有**可编辑的 `eq_low_gain`（`.slint` 的 `device-eq-available`）。
    ///
    /// `false` ⇒ 滑块禁用并如实写"本轨没有 eq_low_gain"，⛔ 不画一个假的可用控件。
    #[must_use]
    pub fn device_eq_available(&self) -> bool {
        self.eq_low_gain.is_some()
    }

    /// 滑块当前值 (dB)；没有目标时给 `0.0`（配合 [`Self::device_eq_available`] 读）。
    #[must_use]
    pub fn device_eq_gain_db(&self) -> f32 {
        self.eq_low_gain.as_ref().map_or(0.0, |eq| eq.value_db)
    }

    /// 滑块值域下界 (dB)；没有目标时给 `0.0`。
    #[must_use]
    pub fn device_eq_min_db(&self) -> f32 {
        self.eq_low_gain.as_ref().map_or(0.0, |eq| eq.min_db)
    }

    /// 滑块值域上界 (dB)；没有目标时给 `0.0`。
    #[must_use]
    pub fn device_eq_max_db(&self) -> f32 {
        self.eq_low_gain.as_ref().map_or(0.0, |eq| eq.max_db)
    }

    /// 发送条每一行的**路由边身份**（与下面三个数组**同索引集**）。
    ///
    /// ⭐ 任务单 `#159`：`.slint` 的 `send-edge-ids` 就是它。三个手势都用**身份**
    /// 寻址（宿主重新从权威工程查边），⛔ 不用行下标当第二份身份。
    #[must_use]
    pub fn send_edge_ids(&self) -> Vec<String> {
        self.sends.iter().map(|row| row.edge_id.clone()).collect()
    }

    /// 发送条每一行的目的地显示名。
    #[must_use]
    pub fn send_destination_names(&self) -> Vec<String> {
        self.sends
            .iter()
            .map(|row| row.destination_name.clone())
            .collect()
    }

    /// 发送条每一行的增益显示文本（`"−12 dB"` / `"单位增益"`）。
    #[must_use]
    pub fn send_gain_labels(&self) -> Vec<String> {
        self.sends
            .iter()
            .map(|row| row.gain_display.clone())
            .collect()
    }

    /// 发送条每一行的滑块位置 (dB) —— `None`（单位增益）在这里落到 **0 dB**。
    ///
    /// ⚠ 这只是**显示几何**：`None` 与 `Some(0.0)` 的区别住在 [`Self::sends`]
    /// （`gain_db`），⛔ 不在这个 `f32` 数组里。宿主提交时读的是模型的那一份。
    #[must_use]
    pub fn send_gain_dbs(&self) -> Vec<f32> {
        self.sends
            .iter()
            .map(|row| row.gain_db.unwrap_or(0.0))
            .collect()
    }

    /// 发送增益的值域下界 (dB) —— **模型给的**（[`send_gain_range`]）。
    #[must_use]
    pub fn send_gain_min_db(&self) -> f32 {
        send_gain_range().0
    }

    /// 发送增益的值域上界 (dB) —— **模型给的**（[`send_gain_range`]）。
    #[must_use]
    pub fn send_gain_max_db(&self) -> f32 {
        send_gain_range().1
    }

    /// 剪辑摆放身份（`clip-{ulid}-header` 的 `{ulid}` 段）。
    #[must_use]
    pub fn clip_ulids(&self) -> Vec<String> {
        self.clips
            .iter()
            .map(|clip| clip.placement_id.clone())
            .collect()
    }

    /// 剪辑标签（`"{轨道} · {片段}"`）。
    #[must_use]
    pub fn clip_labels(&self) -> Vec<String> {
        self.clips.iter().map(|clip| clip.label.clone()).collect()
    }

    /// 剪辑相对 x（逻辑像素）。
    #[must_use]
    pub fn clip_positions(&self) -> Vec<f32> {
        self.clips.iter().map(|clip| clip.x).collect()
    }

    /// 剪辑宽度（逻辑像素）。
    #[must_use]
    pub fn clip_widths(&self) -> Vec<f32> {
        self.clips.iter().map(|clip| clip.width).collect()
    }

    /// 剪辑块顶沿的相对 y（逻辑像素）—— 由所在行的 [`RowGeometry::clip_y`] 给出。
    #[must_use]
    pub fn clip_ys(&self) -> Vec<f32> {
        self.clips.iter().map(|clip| clip.y).collect()
    }

    /// 剪辑块高（逻辑像素）—— 由所在行的 [`RowGeometry::clip_height`] 给出。
    #[must_use]
    pub fn clip_heights(&self) -> Vec<f32> {
        self.clips.iter().map(|clip| clip.height).collect()
    }

    // ------------------------------------------------------------------
    // 卷帘音符的位置（x / y / 宽 / 车道）—— 全部由 tick / 音高整数派生
    // ------------------------------------------------------------------

    /// 音符块左沿 x（逻辑像素，`tick_to_px(start_tick)`）。
    #[must_use]
    pub fn note_positions(&self) -> Vec<f32> {
        self.notes.iter().map(|note| note.x).collect()
    }

    /// `[ROAD-M3-002 / BASELINE-003 第一刀]` 只返回**可见水平窗口**内的音符下标。
    ///
    /// 为什么先做这一刀：`BASELINE-003` 要的是 10 万音符滚动下的帧率，而 `piano_roll.slint` 明文
    /// 「视口裁剪、R-Tree … 仍未实现」—— 于是视图会把**全部**音符物化成元素。这里提供
    /// "按窗口裁剪"的**唯一实现**：宿主注入 `note-*` 数组时只消费它的结果。
    ///
    /// 包含性口径：与窗口左右沿**相接**的块算可见（露出一像素也算），窗口外的一个不多。
    /// 索引顺序由 `self.notes` 决定，属既有契约，本判据不重排。
    #[must_use]
    pub fn notes_visible_in(&self, scroll_x: f32, viewport_width: f32) -> Vec<usize> {
        let left = scroll_x.max(0.0);
        let right = left + viewport_width.max(0.0);
        // `[UI-NOTE-001]` 步骤 ②：**二分**定位候选（`x <= right`），再逐个验第二个条件。
        //
        // 为什么这样仍然正确：`x <= right` 是"与该窗口相交"的**必要**条件, 所以按 x 排序后
        // 第一个不满足它的下标之后都不可能命中 —— 这正是二分能用的原因。
        // 第二个条件（`x + width >= left`）在 x 上**不单调**（宽度各异）, 故只能对候选逐个判。
        let end = self
            .note_x_order
            .partition_point(|index| self.notes[*index as usize].x <= right);
        let mut hits: Vec<usize> = Vec::with_capacity(end.min(1024));
        for &index in &self.note_x_order[..end] {
            let note = &self.notes[index as usize];
            if note.x + note.width >= left {
                hits.push(index as usize);
            }
        }
        // 原实现按 `notes` 下标升序返回；这里必须保持**同一顺序**, 否则既有判据（逐项比对）会红。
        hits.sort_unstable();
        hits
    }

    /// `[UI-NOTE-001]` 步骤 ①（纵向）：当前视口的 **音高范围**。
    ///
    /// 卷帘目前**固定画 16 条泳道**（`piano_roll.slint` 的 `for lane_index in 16`），所以纵向窗口就是
    /// "泳道号落在 `[0, lane_count)` 的那些音高"。这与"工程里出现过的音高"**不是**同一个量 ——
    /// 后者会把视口之外的音高也算进来，正是账本第 202 轮拒绝发布的那种错值。
    ///
    /// 换算法是**扫描 0..=127** 找泳道在范围内的音高（`pitch_lane` 没有现成的反函数）；
    /// 无匹配时返回 `(0, 0)`，与"没有可见音高"同义。
    #[must_use]
    pub fn visible_pitch_range(&self, lane_count: i32) -> (u8, u8) {
        let mut min: Option<u8> = None;
        let mut max: Option<u8> = None;
        for pitch in 0..=u8::MAX {
            let lane = pitch_lane(pitch);
            if lane >= 0 && lane < lane_count {
                min = Some(min.map_or(pitch, |m: u8| m.min(pitch)));
                max = Some(max.map_or(pitch, |m: u8| m.max(pitch)));
            }
        }
        (min.unwrap_or(0), max.unwrap_or(0))
    }

    /// `[UI-NOTE-003]` **命中测试**：可见窗口内某个逻辑像素点落在哪个音符上。
    ///
    /// 选择 / 铅笔 / 橡皮擦三个工具都以它为前置（"选中音符"、"在网格处画出"、"删除光标下的音符"）。
    /// 它落在 **Rust 侧**（而不是给每个音符加一个 `TouchArea`）：后者会让元素数翻倍, 与规范
    /// 第 3.1 节"批量绘制"的方向相反; 由 Rust 命中, 界面只报告坐标。
    ///
    /// 重叠时的取舍**写明**：返回**绘制顺序最后**（下标最大）的那个 —— 后画的在上层, 点到的就是它。
    /// 参数 `note_height` 由调用方给出（与 `.slint` 里音符框的高度一致）, 以免这里**猜**一个高度。
    ///
    /// **帧口径**：`x` / `y` 是**宿主帧**（网格画布局部：tick 0 = x 0，泳道顶 = y 0，见
    /// [`visible_notes`]）。界面回调报的是**绘制帧**（琴键列 + `Tokens.space-5` + 表头之后的
    /// 坐标）⇒ 调用方必须先经 [`painted_to_host`] 换算（唯一派发器 `host::install_roll_click`
    /// 就是这么做的；不减这个原点就是"点可见音符恒不中"的缺陷）。
    #[must_use]
    pub fn hit_test_visible(
        &self,
        scroll_x: f32,
        viewport_width: f32,
        x: f32,
        y: f32,
        note_height: f32,
    ) -> Option<usize> {
        let mut hit: Option<usize> = None;
        for index in self.notes_visible_in(scroll_x, viewport_width) {
            let note = &self.notes[index];
            // 位置是**相对视口**的（与注入的数组同一口径, 见 `visible_notes`）。
            let left = note.x - scroll_x.max(0.0);
            let right = left + note.width;
            if x >= left && x <= right && y >= note.y && y <= note.y + note_height {
                hit = Some(index);
            }
        }
        hit
    }

    /// `[UI-NOTE-001]` 步骤 ①：当前视口的 **tick 范围**（`[min_tick, max_tick]`）。
    ///
    /// 规范第 3.1 节要求视口以 `min_tick` / `max_tick`（以及音高上下界）暴露给裁剪核心；
    /// 本函数给出水平那一对。它必须与 [`Self::notes_visible_in`] **口径一致** —— 否则"索引说可见、
    /// 范围说不在"这种自相矛盾会一直藏着（判据就是查这个）。
    /// 换算失败（`ticks_per_pixel` 为 0 或像素超 `u32`）时退化为 `(0, 0)`，与"没有可见范围"同义。
    #[must_use]
    pub fn visible_tick_range(&self, scroll_x: f32, viewport_width: f32) -> (u64, u64) {
        let left = scroll_x.max(0.0);
        let right = left + viewport_width.max(0.0);
        let to_u32 = |px: f32| -> Option<u32> {
            if px.is_finite() && px >= 0.0 && px <= u32::MAX as f32 {
                Some(px as u32)
            } else {
                None
            }
        };
        let (Some(left_px), Some(right_px)) = (to_u32(left), to_u32(right)) else {
            return (0, 0);
        };
        let min_tick = px_to_tick(left_px, self.ticks_per_pixel).unwrap_or(0);
        let max_tick = px_to_tick(right_px, self.ticks_per_pixel).unwrap_or(0);
        (min_tick, max_tick)
    }

    /// 取可见窗口内的音符（**六个**平行数组共用**同一**索引集）。
    ///
    /// `positions` 是**相对视口**的（已减去 `scroll_x`），因为 `.slint` 契约规定它不做位置算术；
    /// 与窗口左沿相交但起始更早的音符会得到**负** x（应部分可见），这是正确的。
    #[must_use]
    pub fn visible_notes(&self, scroll_x: f32, viewport_width: f32) -> VisibleNotes {
        let left = scroll_x.max(0.0);
        let indices = self.notes_visible_in(scroll_x, viewport_width);
        let mut out = VisibleNotes {
            positions: Vec::with_capacity(indices.len()),
            widths: Vec::with_capacity(indices.len()),
            ys: Vec::with_capacity(indices.len()),
            rows: Vec::with_capacity(indices.len()),
            ulids: Vec::with_capacity(indices.len()),
            velocities: Vec::with_capacity(indices.len()),
        };
        for index in indices {
            let note = &self.notes[index];
            // 相对视口（`scroll_x` 已 clamp 到 >= 0，与裁剪用同一个 left）。
            out.positions.push(note.x - left);
            out.widths.push(note.width);
            out.ys.push(note.y);
            out.rows.push(note.row);
            out.ulids.push(note.id.clone());
            out.velocities.push(note.velocity_normalized);
        }
        out
    }

    /// 音符块宽（逻辑像素，下限 [`MIN_BLOCK_WIDTH_PX`]）。
    #[must_use]
    pub fn note_widths(&self) -> Vec<f32> {
        self.notes.iter().map(|note| note.width).collect()
    }

    /// 选区（`MainWindow` 的 `selected-ulids`）在**这份投影里**解析出的 tick 跨度。
    ///
    /// 返回 `(min start_tick, max end_tick)`：**不是一个音符的跨度**，而是整个选区并集的
    /// 跨度（多选时要全部装进视口）。身份解析走**投影自己的** `notes`
    /// （[`NoteView::id`]）—— 于是"选中的身份"与"画面上的音符"是同一份事实，
    /// 不存在第二张 `ulid → tick` 表。
    ///
    /// 返回 `None` = 选区为空，或选中的身份一个都不在工程里：两种情形都**没有可作用的
    /// 对象**，调用方据此不消费按键（与 [`crate::host`] 里 `DeleteSelection` 的取向相同）。
    #[must_use]
    pub fn selection_tick_span(&self, selected_ulids: &[String]) -> Option<(u64, u64)> {
        let mut span: Option<(u64, u64)> = None;
        for id in selected_ulids {
            for note in &self.notes {
                if &note.id != id {
                    continue;
                }
                span = Some(match span {
                    None => (note.start_tick, note.end_tick),
                    Some((start, end)) => (start.min(note.start_tick), end.max(note.end_tick)),
                });
                break;
            }
        }
        span
    }

    /// 工程的**内容末端 tick**：全部剪辑摆放与全部段落的 `end_tick` 的最大值。
    ///
    /// 它与投影内部用来铺标尺的那一次 `max` **同一口径**（`from_project_with_zoom_cursor_and_layout`
    /// 里的 `end_tick` 循环）；这里把那个读数发布出来，供"全曲总览"当输入，
    /// 而不是在宿主里再遍历一遍工程（那会是第二份几何）。
    ///
    /// `0` = 工程没有任何可看的内容（空工程 / 没有任何摆放与段落）—— 调用方据此不消费按键。
    #[must_use]
    pub fn content_end_tick(&self) -> u64 {
        let mut end = 0_u64;
        for clip in &self.clips {
            end = end.max(clip.end_tick);
        }
        for section in &self.sections {
            end = end.max(section.end_tick);
        }
        end
    }

    /// 音符块顶沿 y（逻辑像素，由 [`pitch_lane`] 的车道索引一次乘法得到）。
    #[must_use]
    pub fn note_ys(&self) -> Vec<f32> {
        self.notes.iter().map(|note| note.y).collect()
    }

    /// 音符的音高车道索引（0 = 最上面一条）。
    #[must_use]
    pub fn note_rows(&self) -> Vec<i32> {
        self.notes.iter().map(|note| note.row).collect()
    }

    // ------------------------------------------------------------------
    // 轨道色标（`TrackV3::color` 的消费形态）
    // ------------------------------------------------------------------

    /// 非主总线轨道的色标 RGB（非法 / 缺失已回退到 [`DEFAULT_TRACK_COLOR`]）。
    #[must_use]
    pub fn track_colors(&self) -> Vec<RgbColor> {
        self.tracks.iter().map(|track| track.color_rgb).collect()
    }

    /// 非主总线轨道的声相显示文本（`"C"` / `"L50"` / `"R30"`）。
    ///
    /// 混音台通道条的声相文本源：界面只做"取数组下标"，不做任何算术或格式化。
    #[must_use]
    pub fn track_pans(&self) -> Vec<String> {
        self.tracks
            .iter()
            .map(|track| track.pan_display.clone())
            .collect()
    }

    /// 非主总线轨道的推子位置（0.0–1.0，由 `TrackV3::volume_db` 派生）。
    #[must_use]
    pub fn track_volume_fractions(&self) -> Vec<f32> {
        self.tracks
            .iter()
            .map(|track| track.volume_fraction)
            .collect()
    }

    /// 非主总线轨道的规范化色标文本（`#RRGGBB`，回退色也在内）。
    ///
    /// 这一份同时进 `.slint` 的 `track-color-labels` 与语义注册表的标签 ——
    /// 于是"投影 ↔ 控件树"的色标一致性可以被纯 Rust 判据 + Tier-1 判据**两侧**钉住。
    #[must_use]
    pub fn track_color_labels(&self) -> Vec<String> {
        self.tracks
            .iter()
            .map(|track| track.color_hex.clone())
            .collect()
    }

    /// 段落相对 x（逻辑像素）。
    #[must_use]
    pub fn section_positions(&self) -> Vec<f32> {
        self.sections.iter().map(|section| section.x).collect()
    }

    /// 段落卡片宽（逻辑像素）。
    #[must_use]
    pub fn section_widths(&self) -> Vec<f32> {
        self.sections.iter().map(|section| section.width).collect()
    }

    // ------------------------------------------------------------------
    // 自动化泳道（`TrackV3::automation_lanes` 的消费形态）
    // ------------------------------------------------------------------
    //
    // 9 个平行数组全部由 `self.automation_lanes` 派生 —— 只有一个事实源。
    // 它们与 `.slint` 的属性**同名**（kebab-case），由 `host::apply_view` 单向注入；
    // 界面只做"取数组下标"，不做任何算术（位置与值都已经是逻辑像素 / 文本）。

    /// 自动化泳道的目标键（`volume` / `pan` / `send-{edge}` / `device-{slot}-{param}` /
    /// `macro-{i}`）—— 元素 ID 的第三段，也是 `.slint` 循环的驱动数组。
    #[must_use]
    pub fn automation_lane_target_keys(&self) -> Vec<String> {
        self.automation_lanes
            .iter()
            .map(|lane| lane.target_key.clone())
            .collect()
    }

    /// 自动化泳道所属的音轨序号（进 `track-{i}-automation-{key}-lane` 的 `{i}` 段）。
    #[must_use]
    pub fn automation_lane_track_indexes(&self) -> Vec<i32> {
        self.automation_lanes
            .iter()
            .map(|lane| {
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                let index = lane.track_index as i32;
                index
            })
            .collect()
    }

    /// 泳道的语义元素 ID（`track-{i}-automation-{key}-lane`）。
    ///
    /// `.slint` 用三段字面量拼出同一个字符串（`[UI-TEST-001]` 的模板形态要求如此，
    /// 见 `automation::tests::lane_element_ids_match_the_slint_template`）；这份数组是
    /// 判据与 `--headless` 导出用的事实源。
    #[must_use]
    pub fn automation_lane_element_ids(&self) -> Vec<String> {
        self.automation_lanes
            .iter()
            .map(|lane| lane.element_id.clone())
            .collect()
    }

    /// 泳道的无障碍标签（携带单位与当前值，例如 `Lead · 音量 自动化 -6.0 dB`）。
    #[must_use]
    pub fn automation_lane_labels(&self) -> Vec<String> {
        self.automation_lanes
            .iter()
            .map(|lane| lane.label.clone())
            .collect()
    }

    /// 泳道的轴文本（单位 + 量程，例如 `dB [-60.0, 12.0]`；自适应时带 ` 自适应`）。
    #[must_use]
    pub fn automation_lane_axis_labels(&self) -> Vec<String> {
        self.automation_lanes
            .iter()
            .map(|lane| lane.axis_label.clone())
            .collect()
    }

    /// 泳道带的顶沿 y（逻辑像素，画布相对）。
    #[must_use]
    pub fn automation_lane_band_ys(&self) -> Vec<f32> {
        self.automation_lanes
            .iter()
            .map(|lane| lane.band_y)
            .collect()
    }

    /// 泳道带的高度（逻辑像素）。
    #[must_use]
    pub fn automation_lane_band_heights(&self) -> Vec<f32> {
        self.automation_lanes
            .iter()
            .map(|lane| lane.band_height)
            .collect()
    }

    /// 泳道的读开关（`AutomationLane::read_enabled`）—— 界面据此换描边颜色。
    #[must_use]
    pub fn automation_lane_read_enabled(&self) -> Vec<bool> {
        self.automation_lanes
            .iter()
            .map(|lane| lane.read_enabled)
            .collect()
    }

    /// 泳道的角标文本（`读关` / `● 触碰` / 组合 / 空串）。
    #[must_use]
    pub fn automation_lane_badges(&self) -> Vec<String> {
        self.automation_lanes
            .iter()
            .map(|lane| lane.badge.clone())
            .collect()
    }

    /// 折线的 Slint `Path` 指令（局部坐标，`M x y L x y …`；空泳道是空串）。
    #[must_use]
    pub fn automation_path_commands(&self) -> Vec<String> {
        self.automation_lanes
            .iter()
            .map(|lane| lane.path_commands.clone())
            .collect()
    }

    /// 规范行协议：逐字节稳定的投影快照（`BTreeMap` 顺序 + 定点浮点格式化）。
    ///
    /// 存在的意义是把"确定性"变成**可比较的字节**：同一工程的两次投影必须给出
    /// 完全相同的行；而任何算术改动（浮点位置、顺序漂移、字段丢失）都会改掉字节。
    #[must_use]
    pub fn canonical_lines(&self) -> Vec<String> {
        let mut lines = Vec::with_capacity(
            6 + self.tracks.len()
                + self.clips.len()
                + self.sections.len()
                + self.scenes.len()
                + self.notes.len()
                + self.bar_positions.len(),
        );
        lines.push(format!(
            "project id={} title={} bpm={:.6} ts={}/{} ppq={} tpp={} bar_ticks={}",
            self.project_id,
            self.title,
            self.bpm,
            self.time_signature_numerator,
            self.time_signature_denominator,
            self.ppq,
            self.ticks_per_pixel,
            self.bar_length_ticks,
        ));
        if let Some(master) = &self.master {
            lines.push(format!(
                "master id={} name={} kind={} mute={} solo={} volume_db={:.6} pan_millis={} clips={}",
                master.id,
                master.name,
                master.kind,
                master.mute,
                master.solo,
                master.volume_db,
                master.pan_millis,
                master.clip_count,
            ));
        }
        for track in &self.tracks {
            lines.push(format!(
                "track index={} id={} name={} kind={} color={} color_rgb={} mute={} solo={} solo_safe={} volume_db={:.6} pan_millis={} pan={} clips={} sampler={}",
                track.index,
                track.id,
                track.name,
                track.kind,
                track.color.as_deref().unwrap_or("-"),
                track.color_hex,
                track.mute,
                track.solo,
                track.solo_safe,
                track.volume_db,
                track.pan_millis,
                track.pan_display,
                track.clip_count,
                // ⭐ `#82`：投影里**第一次**有采样器绑定的可读读数。`None` 用 `-`
                // （与 `color` 同一款占位），因此"解绑"与"绑了一条空路径"在文本上可分。
                track.sampler_path.as_deref().unwrap_or("-"),
            ));
        }
        for clip in &self.clips {
            lines.push(format!(
                "clip index={} placement={} clip={} track={} name={} start={} end={} dur={} x={:.6} width={:.6} lane={} muted={}",
                clip.index,
                clip.placement_id,
                clip.clip_id,
                clip.track_index,
                clip.clip_name,
                clip.start_tick,
                clip.end_tick,
                clip.duration_ticks,
                clip.x,
                clip.width,
                clip.lane,
                clip.muted,
            ));
        }
        for section in &self.sections {
            lines.push(format!(
                "section index={} id={} name={} start={} end={} x={:.6} width={:.6} color={}",
                section.index,
                section.id,
                section.name,
                section.start_tick,
                section.end_tick,
                section.x,
                section.width,
                section.color.as_deref().unwrap_or("-"),
            ));
        }
        for scene in &self.scenes {
            lines.push(format!(
                "scene index={} id={} name={} tempo={} color={}",
                scene.index,
                scene.id,
                scene.name,
                scene
                    .tempo
                    .map_or_else(|| "-".to_owned(), |tempo| format!("{tempo:.6}")),
                scene.color.as_deref().unwrap_or("-"),
            ));
        }
        for note in &self.notes {
            lines.push(format!(
                "note index={} id={} clip={} start={} end={} dur={} pitch={} velocity={} norm={:.6} x={:.6} y={:.6} width={:.6} row={}",
                note.index,
                note.id,
                note.clip_id,
                note.start_tick,
                note.end_tick,
                note.duration_ticks,
                note.pitch,
                note.velocity,
                note.velocity_normalized,
                note.x,
                note.y,
                note.width,
                note.row,
            ));
        }
        for (index, x) in self.bar_positions.iter().enumerate() {
            lines.push(format!("bar index={index} x={x:.6}"));
        }
        for lane in &self.automation_lanes {
            lines.push(format!(
                "automation-lane index={} id={} target={} track={} unit={:?} domain={:.6}..{:.6} adaptive={} read={} write={:?} cursor={} value={} static={} points={} samples={} band={:.6}+{:.6} path={}",
                lane.index,
                lane.element_id,
                lane.target_key,
                lane.track_index,
                lane.unit,
                lane.domain_min,
                lane.domain_max,
                lane.domain_adaptive,
                lane.read_enabled,
                lane.write_mode,
                lane.cursor_tick,
                lane.value_at_cursor
                    .map_or_else(|| "-".to_owned(), |value| format!("{value:.6}")),
                lane.static_value
                    .map_or_else(|| "-".to_owned(), |value| format!("{value:.6}")),
                lane.point_count(),
                lane.sample_count(),
                lane.band_y,
                lane.band_height,
                lane.path_commands,
            ));
            for vertex in &lane.samples {
                lines.push(format!(
                    "automation-vertex lane={} tick={} value={:.6} x={:.6} y={:.6}",
                    lane.element_id, vertex.tick, vertex.value, vertex.x, vertex.y
                ));
            }
        }
        lines
    }

    /// [`Self::canonical_lines`] 的字节形态（以 `\n` 结尾，便于 diff 与哈希）。
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut text = self.canonical_lines().join("\n");
        text.push('\n');
        text.into_bytes()
    }
}

/// `u32` 像素 → `f32`（显式转换点，集中在这里便于审查）。
///
/// 只做一次转换、不做任何累加，因此不引入 `ARCH-DET-001` 关注的漂移。
fn as_px(value: u32) -> f32 {
    value as f32
}

/// `MidiNote::velocity`（0–127）→ 力度泳道用的 0.0–1.0。
///
/// 越界值被夹到闭区间（模型 `validate()` 已拒绝 >127，但投影不假设输入可信）。
fn velocity_normalized(velocity: u8) -> f32 {
    let clamped = velocity.min(yeban_model::music::MIDI_VELOCITY_MAX);
    f32::from(clamped) / f32::from(yeban_model::music::MIDI_VELOCITY_MAX)
}

/// BPM → 整数千分之一（`120.005` → `120005`）；非有限值或负值归 0。
fn bpm_millis(bpm: f64) -> u32 {
    if !bpm.is_finite() || bpm < 0.0 {
        return 0;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let millis = (bpm * 1000.0).round() as u64;
    u32::try_from(millis).unwrap_or(u32::MAX)
}

/// 轨道类型 → `.slint` 用的短名。
fn kind_name(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Midi => "midi",
        TrackKind::Audio => "audio",
        TrackKind::AuxReturn => "aux-return",
        TrackKind::Master => "master",
    }
}

/// 投影一条轨道。
///
/// `row` 是这条轨道在编排视图里的行几何（`None` = 主总线，它没有编排行）。
fn track_view(
    track: &TrackV3,
    index: usize,
    is_master: bool,
    row: Option<RowGeometry>,
) -> TrackView {
    #[allow(clippy::cast_possible_truncation)]
    let pan_millis = (f64::from(track.pan) * 1000.0).round() as i32;
    // 色标：**在这里**（唯一一处）解析 + 回退；界面与判据都只读解析结果。
    let color_rgb = track_color_or_default(track.color.as_deref());
    // 编排行几何：主总线没有行 ⇒ 宽高与整数行高恒 0（界面也不会画它 ——
    // `track-names` 不含主总线）。整数真相（`row_height_px`）与 `height` 同源，
    // 后者是前一者减去上下留白。
    let (y, height, row_height_px) = row.map_or((0.0, 0.0, 0), |row| {
        (row.y, row.drawn_height(), row.height_px)
    });
    TrackView {
        index,
        id: track.id.to_canonical_string(),
        name: track.name.clone(),
        kind: kind_name(track.kind),
        color: track.color.clone(),
        color_rgb,
        color_hex: color_rgb.to_hex(),
        mute: track.mute,
        solo: track.solo,
        solo_safe: track.solo_safe,
        sampler_path: track.sampler.as_ref().map(|binding| binding.path.clone()),
        volume_db: track.volume_db,
        volume_display: format!("{:.1}", track.volume_db),
        volume_fraction: volume_fraction(track.volume_db),
        pan_millis,
        pan_display: pan_display(pan_millis),
        clip_count: track.clips.len(),
        is_master,
        y,
        height,
        row_height_px,
    }
}

/// 推子的 dB 量程下界（与 `ui/console/mixer_console.slint` 的 `accessible-value-minimum` 一致）。
pub const FADER_MIN_DB: f32 = -60.0;

/// 推子的 dB 量程上界（与 `.slint` 的 `accessible-value-maximum` 一致）。
pub const FADER_MAX_DB: f32 = 6.0;

/// 音量 dB → 推子位置（0.0–1.0）。
///
/// 线性映射到 `[`FADER_MIN_DB`, `FADER_MAX_DB`]` 并夹紧；`NaN`/`±∞` 归 `0.0`
/// （推子几何里出现 `NaN` 会让 Slint 的布局算出非有限值 —— 那是最难查的一类界面 bug）。
///
/// **与电平无关**：电平柱高来自引擎的 `MeterFrame`（[`crate::meters`]），推子位置来自工程的
/// `TrackV3::volume_db`。把两者混在一个数组里（旧版 `FADER_LEVELS` 就是这么干的）会让
/// "推子动了"与"有声音了"在界面上无法区分。
#[must_use]
pub fn volume_fraction(volume_db: f32) -> f32 {
    if !volume_db.is_finite() {
        return 0.0;
    }
    ((volume_db - FADER_MIN_DB) / (FADER_MAX_DB - FADER_MIN_DB)).clamp(0.0, 1.0)
}

/// 声相千分之一 → 显示文本（**唯一**的声相文本实现）。///
/// 口径（一条纯函数，判据 `pan_display_is_centred_around_zero` 钉住）：
///
/// | `pan_millis` | 文本 | 含义 |
/// | :--- | :--- | :--- |
/// | `0` | `"C"` | 居中 |
/// | `-500` | `"L50"` | 左 50% |
/// | `-5` | `"L0"` | 左偏但不足 1%（**不设死区**：读数是"偏了"，就该显示"偏了"） |
/// | `1000` | `"R100"` | 右满 |
/// | 越界值 | 先夹到 `-1000..=1000` | 模型 `validate()` 已拦，但投影不假设输入可信 |
///
/// 用整数（千分之一）而不是 `f32`：这一层的**交付物是文本**（`.slint` 只画
/// `pan_display` 的结果 —— `mixer_console.slint` 里通道条与主总线的 `UiMonoText.text`
/// 分别是 `"PAN " + root.track-pans[…]` / `"PAN " + root.master-pan`），而它同时进投影的
/// 规范行协议（`canonical_lines` 的 `pan_millis={}`，见 [`track_view`] →
/// [`ViewState::canonical_lines`]，由判据
/// `two_projections_of_the_same_project_are_byte_identical` 钉住逐字节可复现）。
/// 于是把模型 `f32` 的量化**一次**做在投影边界（[`track_view`] 的 `round(f64 * 1000)`），
/// 此后文本只用整数除法（`abs / 10`）：1% 档位与"不足 1% 也不设死区"都是精确的整数事实，
/// 规范行里也永远不会出现 `-0` 或浮点尾数。
///
/// **更正（本次）**：这里原写「声相文本会进 `accessible-value`，跨平台浮点格式化的
/// 1 ulp 差异会让"同一工程两台机器给出不同文本"（`ARCH-DET-001` 禁止的正是这个）」。
/// 两句都不成立，故删去：① 全史没有任何 `.slint` 把 `pan` 绑到 `accessible-value`
/// （`git log -S` 无命中；`mixer_console.slint` 只把它写进 `UiMonoText.text`），
/// 而 `docs/ledger/feature-alignment.md` 与 `docs/ledger/app-mixer-notes.md` 也一直把
/// 「声相读不到」当作现状在册；② Rust 的浮点格式化是纯 Rust、与平台无关，且同一条
/// `canonical_lines` 本来就用 `{:.6}` 格式化 `volume_db`。`ARCH-DET-001` 管的是
/// PCM 采样的位级一致，不是文本。整数千分之一的真实理由见上段；表示法本身**不动**
/// （它不在 schema 里 —— `schemas/project.schema.json` 的 `pan` 仍是 `-1.0..=1.0`
/// 的 number —— 改类型只会无谓地挪动规范行的字节）。
#[must_use]
pub fn pan_display(pan_millis: i32) -> String {
    let clamped = pan_millis.clamp(-1000, 1000);
    if clamped == 0 {
        return "C".to_owned();
    }
    // 千分之一 → 百分数：整除 10（`-505` ⇒ `L50`，与 DAW 常见的整数百分比显示一致）。
    let percent = (clamped.abs() / 10) as u32;
    let side = if clamped < 0 { 'L' } else { 'R' };
    format!("{side}{percent}")
}

/// 投影一个 MIDI 音符：身份 / 力度来自模型，**位置**由 tick / 音高整数派生。
///
/// # Errors
///
/// `start_tick + duration_ticks` 越过 `u64::MAX` → [`BridgeError::TickOverflow`]；
/// tick 换算出的像素超出 `u32` → [`BridgeError::PixelOverflow`]。
/// 越界音高**不是**错误（钳制进可见车道，见 [`pitch_lane`]）。
fn note_view(
    entry: &ClipPoolEntry,
    note: &MidiNote,
    index: usize,
    ticks_per_pixel: u64,
) -> Result<NoteView, BridgeError> {
    let start_tick = note.start_tick;
    let end_tick = start_tick
        .checked_add(note.duration_ticks)
        .ok_or(BridgeError::TickOverflow { start_tick })?;
    let x = as_px(tick_to_px(start_tick, ticks_per_pixel)?);
    let x_end = as_px(tick_to_px(end_tick, ticks_per_pixel)?);
    Ok(NoteView {
        index,
        id: note.id.to_canonical_string(),
        clip_id: entry.id.to_canonical_string(),
        start_tick,
        end_tick,
        duration_ticks: note.duration_ticks,
        pitch: note.pitch,
        velocity: note.velocity,
        velocity_normalized: velocity_normalized(note.velocity),
        x,
        width: (x_end - x).max(MIN_BLOCK_WIDTH_PX),
        row: pitch_lane(note.pitch),
        y: pitch_lane_y(note.pitch),
    })
}

/// 投影一个剪辑摆放（`index` 由调用方在收集完成后统一编号）。
///
/// `row` 是这条轨道在编排视图里的行几何（同一条轨道上的所有剪辑读**同一份**行几何）；
/// `None` 只可能来自越界的 `track_index`（构造上不会发生）⇒ 宽高退化为 0 而不是 panic。
fn clip_view(
    project: &YebanProjectV1,
    placement: &ClipPlacement,
    track_index: usize,
    track_name: &str,
    ticks_per_pixel: u64,
    row: Option<RowGeometry>,
) -> Result<ClipView, BridgeError> {
    let start_tick = placement.start_tick;
    let end_tick = start_tick
        .checked_add(placement.duration_ticks)
        .ok_or(BridgeError::TickOverflow { start_tick })?;
    let x = as_px(tick_to_px(start_tick, ticks_per_pixel)?);
    let x_end = as_px(tick_to_px(end_tick, ticks_per_pixel)?);
    let entry = project.clip_pool.get(&placement.clip_id);
    let clip_name = entry.map_or_else(String::new, |entry| entry.name.clone());
    let content = entry.map_or("unknown", |entry| match entry.content {
        ClipContent::Midi { .. } => "midi",
        ClipContent::Audio { .. } => "audio",
    });
    let label = if clip_name.is_empty() {
        track_name.to_owned()
    } else {
        format!("{track_name} · {clip_name}")
    };
    #[allow(clippy::cast_possible_truncation)]
    let lane = track_index as i32;
    let (y, height) = row.map_or((0.0, 0.0), |row| (row.clip_y(), row.clip_height()));
    Ok(ClipView {
        index: 0,
        placement_id: placement.id.to_canonical_string(),
        clip_id: placement.clip_id.to_canonical_string(),
        track_index,
        track_name: track_name.to_owned(),
        clip_name,
        content,
        label,
        start_tick,
        end_tick,
        duration_ticks: placement.duration_ticks,
        x,
        width: (x_end - x).max(MIN_BLOCK_WIDTH_PX),
        y,
        height,
        lane,
        muted: placement.muted,
    })
}

/// 投影一个段落。
fn section_view(
    section: &SectionV3,
    index: usize,
    ticks_per_pixel: u64,
) -> Result<SectionView, BridgeError> {
    let x = as_px(tick_to_px(section.start_tick, ticks_per_pixel)?);
    let x_end = as_px(tick_to_px(section.end_tick, ticks_per_pixel)?);
    Ok(SectionView {
        index,
        id: section.id.to_canonical_string(),
        name: section.name.clone(),
        start_tick: section.start_tick,
        end_tick: section.end_tick,
        x,
        width: (x_end - x).max(MIN_BLOCK_WIDTH_PX),
        color: section.color.clone(),
    })
}

/// 投影一个场景。
fn scene_view(scene: &SceneV3, index: usize) -> SceneView {
    SceneView {
        index,
        id: scene.id.to_canonical_string(),
        name: scene.name.clone(),
        tempo: scene.tempo,
        color: scene.color.clone(),
    }
}

/// 把 `clip_pool` 投影成素材池面板的行（`BTreeMap` 键序 = 身份升序）。
///
/// 每一行的字段都**只**来自工程文档：
///
/// * 音频：`ClipContent::Audio { asset, gain_db }`；字节数取
///   `YebanProjectV1::assets[asset].byte_len`（⭐ `#132` 的录音只提交 `Op::AddClip`，
///   **不**写 `assets` 索引 ⇒ 新录的那条如实显示「字节数未知」）；
/// * MIDI：`ClipContent::Midi` 的音符条数。
///
/// ⛔ 这里不读盘、不打开容器、不按 WAV 头推算字节数 —— 投影的输入只有 `project`。
fn asset_pool_rows(project: &YebanProjectV1) -> Vec<AssetPoolRowView> {
    project
        .clip_pool
        .values()
        .map(|entry| {
            let (kind, detail) = match &entry.content {
                ClipContent::Audio { asset, gain_db } => {
                    let prefix: String =
                        asset.to_string().chars().take(ASSET_PREFIX_CHARS).collect();
                    let bytes = project.assets.get(asset).map_or_else(
                        || "字节数未知".to_owned(),
                        |meta| format!("{} 字节", meta.byte_len),
                    );
                    (
                        "音频".to_owned(),
                        format!("{bytes} · {prefix}… · {gain_db:.1} dB"),
                    )
                }
                ClipContent::Midi { notes } => {
                    ("MIDI".to_owned(), format!("音符 {} 条", notes.len()))
                }
            };
            let label = format!("{} · {} · {}", entry.name, kind, detail);
            AssetPoolRowView {
                clip_id: entry.id.to_canonical_string(),
                name: entry.name.clone(),
                kind,
                detail,
                label,
            }
        })
        .collect()
}

/// 设备机架展示的那条轨 —— **唯一**规则：身份序（`BTreeMap` 键序）第一条**非主总线**轨。
///
/// ⚠ 为什么是这条规则（而不是"选中轨"）：界面里**没有**选中轨这一状态
/// （`grep -rn 'selected_track\|active-track\|current_track' crates/yeban-app` 命中 0；
/// 台账第 236 轮逐字裁决 "the current track does not exist"）。本切片不起一套新的选择
/// 状态机，而是把**唯一**的确定性规则写进投影，并让标题把它**显示出来**
/// （[`ViewState::device_track_label`]）⇒ 用户看到的是"轨 鼓"，不是一句"当前轨"。
/// 换工程之后它随 `BTreeMap` 键序变化，不需要宿主再同步一份下标。
#[must_use]
pub fn device_rack_track(project: &YebanProjectV1) -> Option<&TrackV3> {
    project
        .tracks
        .values()
        .find(|track| track.id != project.master_bus_track_id)
}

/// ⭐ 任务单 `#180`：**主总线**那条轨 —— 设备增删的第二个（也是最后一个）目标。
///
/// 口径与 [`track_create_available`] / [`master_bus_label`] / [`master_device_rows`]
/// **同一份**：`tracks.get(master_bus_track_id)` **且** `kind == TrackKind::Master`。
/// ⛔ 不用 [`device_rack_track`]（它**排除**主总线）—— 那是"指错轨"的唯一来源。
///
/// 返回 `None` = 工程没有主总线（`master_bus_track_id` 为 nil 或指向非 `Master` 轨）
/// ⇒ 三个主总线设备手势都以**逐字原因**响亮拒绝，⛔ 不静默改到别的轨上。
#[must_use]
pub fn master_device_track(project: &YebanProjectV1) -> Option<&TrackV3> {
    project
        .tracks
        .get(&project.master_bus_track_id)
        .filter(|track| track.kind == TrackKind::Master)
}

/// 设备链 + EQ 旋钮的投影本体（[`ViewState`] 与宿主提交路径**共用**这一份口径）。
///
/// 返回 `(展示标题, 设备行, 第一格 eq_low_gain)`。
#[must_use]
fn device_rack_projection(
    track: Option<&TrackV3>,
) -> (String, Vec<DeviceRowView>, Option<DeviceEqView>) {
    let Some(track) = track else {
        return ("没有可显示的音轨".to_owned(), Vec::new(), None);
    };
    let mut devices = Vec::with_capacity(track.devices.len());
    let mut eq_low_gain: Option<DeviceEqView> = None;
    for (slot_index, device) in track.devices.iter().enumerate() {
        let mut recognised_parts: Vec<String> = Vec::new();
        let mut recognised_groups: Vec<&'static str> = Vec::new();
        let mut unrecognised_parts: Vec<String> = Vec::new();
        for (param_index, param) in device.params.iter().enumerate() {
            match crate::effect_params::recognise(&param.name) {
                Some((group, spec)) => {
                    if !recognised_groups.contains(&group.title) {
                        recognised_groups.push(group.title);
                    }
                    let unit = if spec.unit.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", spec.unit)
                    };
                    recognised_parts.push(format!(
                        "{}= {}{}",
                        param.name,
                        format_param_number(param.value),
                        unit
                    ));
                    // 最小可编辑的那一格：**第一台**带 eq_low_gain 的设备、它的第一个
                    // 该名字的参数。值域**只**来自契约（`spec.min` / `spec.max`）。
                    if spec.name == "eq_low_gain" && eq_low_gain.is_none() {
                        #[allow(clippy::cast_possible_truncation)]
                        let (min_db, max_db) = (spec.min as f32, spec.max as f32);
                        eq_low_gain = Some(DeviceEqView {
                            slot_index,
                            param_index,
                            device_name: device.name.clone(),
                            value_db: param.value,
                            min_db,
                            max_db,
                        });
                    }
                }
                None => {
                    let unit = param
                        .unit
                        .as_deref()
                        .map_or(String::new(), |unit| format!(" {unit}"));
                    unrecognised_parts.push(format!(
                        "{}= {}{}",
                        param.name,
                        format_param_number(param.value),
                        unit
                    ));
                }
            }
        }
        let recognised = if recognised_parts.is_empty() {
            String::new()
        } else {
            format!(
                "已识别 {} · {}：{}",
                recognised_parts.len(),
                recognised_groups.join("/"),
                recognised_parts.join(", ")
            )
        };
        let unrecognised = if unrecognised_parts.is_empty() {
            String::new()
        } else {
            format!(
                "未识别 {} · {}",
                unrecognised_parts.len(),
                unrecognised_parts.join(", ")
            )
        };
        devices.push(DeviceRowView {
            slot_index,
            device_id: device.id.to_canonical_string(),
            name: device.name.clone(),
            bypassed: device.bypassed,
            recognised,
            unrecognised,
        });
    }
    (format!("轨 {} · 设备链", track.name), devices, eq_low_gain)
}

/// ⭐ 任务单 `#173`：**主总线**上的设备链投影（只读；设备架面板的第二段事实源）。
///
/// 口径与 [`Self::devices`]（原名 `devices`）**逐字相同** —— 同一份
/// [`device_rack_projection`]、同一份 `effect_params::recognise` 的"已识别/未识别"
/// 分档 ⇒ 两段不会各说一套。唯一的差别是**主语**：那一段是"展示轨"，这一段是
/// `master_bus_track_id` 指的主总线轨。
///
/// ⚠ 主总线**不在** [`device_rack_track`] 的候选里（那条规则排除了它）⇒ 这个函数
/// 是主总线设备进入 UI 投影的**唯一**入口；`#173` 之前它根本不存在（主总线的设备
/// 在面板上不可见，即使引擎侧现在已经会处理它）。
#[must_use]
pub fn master_device_rows(project: &YebanProjectV1) -> Vec<DeviceRowView> {
    project
        .tracks
        .get(&project.master_bus_track_id)
        .map_or_else(Vec::new, |track| device_rack_projection(Some(track)).1)
}

/// 参数值的显示文本（**只有这一处**格式化）。
///
/// `f32` 的 `Display` 对整数值给出 `6`（⛔ 不是 `6.0`）、对小数给出最短可回读形态
/// （`0.35`）⇒ 界面与判据读到的是同一个字符串。
fn format_param_number(value: f32) -> String {
    format!("{value}")
}

/// 设备机架上 `eq_low_gain` 那一格的**权威寻址**（宿主提交路径用它）。
///
/// 返回 `(轨身份, 设备链下标, 参数下标, 当前值, 值域下界, 值域上界)`。
/// ⚠ 这是**唯一**的查找实现：投影（[`device_rack_projection`]）与宿主
/// （`host::wire_device_rack`）都从同一条规则出发 ⇒ 界面显示的那一格与提交改的那一格
/// 不可能漂移。值域同样**只**来自 [`crate::effect_params`]。
#[must_use]
pub fn device_eq_low_gain_target(
    project: &YebanProjectV1,
) -> Option<(yeban_model::ids::EntityId, usize, usize, f32, f32, f32)> {
    let track = device_rack_track(project)?;
    for (slot_index, device) in track.devices.iter().enumerate() {
        for (param_index, param) in device.params.iter().enumerate() {
            let Some((_, spec)) = crate::effect_params::recognise(&param.name) else {
                continue;
            };
            if spec.name != "eq_low_gain" {
                continue;
            }
            #[allow(clippy::cast_possible_truncation)]
            let (min_db, max_db) = (spec.min as f32, spec.max as f32);
            return Some((
                track.id,
                slot_index,
                param_index,
                param.value,
                min_db,
                max_db,
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;
    use yeban_model::ids::EntityId;

    use super::*;
    use yeban_model::samples::{default_project, filled_project};

    /// 把 `[&str; N]` 常量提升成可与 `Vec<String>` 比较的形态。
    fn owned(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    /// 判据 1b（`[MODEL-AST-001]`）：时间码的**唯一实现**在多种拍号下的读数。
    ///
    /// 期望值是**手算表**（不是拿实现算一遍再和实现比）：一拍 = 一个分母音符
    /// （`4 × 960 ÷ 分母`），一小节 = 分子 × 一拍。三种拍号 + 三个位置覆盖
    /// "小节进位 / 拍进位 / 拍内 tick"三件不同的事。
    #[test]
    fn timecode_is_formatted_from_the_time_signature_not_from_four_four() {
        let cases: &[(u8, u8, u64, &str)] = &[
            // 4/4：一拍 960，一小节 3840
            (4, 4, 0, "001.01.000"),
            (4, 4, 960, "001.02.000"),
            (4, 4, 3_840, "002.01.000"),
            (4, 4, 8_000, "003.01.320"),
            // 3/4：一拍 960，一小节 2880（同一 tick 的读数与 4/4 **不同**）
            (3, 4, 960, "001.02.000"),
            (3, 4, 3_840, "002.02.000"),
            (3, 4, 8_000, "003.03.320"),
            // 6/8：一拍 480（八分音符），一小节 2880
            (6, 8, 960, "001.03.000"),
            (6, 8, 3_840, "002.03.000"),
            (6, 8, 8_000, "003.05.320"),
            // 7/8：一拍 480，一小节 3360
            (7, 8, 3_359, "001.07.479"),
            (7, 8, 3_360, "002.01.000"),
        ];
        for &(numerator, denominator, ticks, expected) in cases {
            let grid = TimecodeGrid::from_time_signature(numerator, denominator)
                .unwrap_or_else(|| panic!("{numerator}/{denominator} 必须能推出网格"));
            assert_eq!(
                grid.timecode_at(ticks),
                expected,
                "{numerator}/{denominator} 的 tick {ticks}"
            );
            assert_eq!(
                timecode_for_ticks(Some(grid), ticks),
                expected,
                "唯一实现与网格的读数必须一致"
            );
        }

        // 网格的两个整数与模型层的权威算术逐项相等（不重写模型算术的直接证据）。
        for (numerator, denominator) in [(4, 4), (3, 4), (6, 8), (7, 8), (5, 16), (2, 2)] {
            let grid = TimecodeGrid::from_time_signature(numerator, denominator).expect("网格");
            let model_bar = yeban_model::SessionRuntimeState::ticks_per_bar(numerator, denominator)
                .expect("模型必须也接受这个拍号");
            assert_eq!(grid.ticks_per_bar(), model_bar);
            assert_eq!(grid.ticks_per_beat() * u64::from(numerator), model_bar);
        }
    }

    /// 判据 1c：**同一个 tick 位置在两种拍号下的读数必须不同**（两个方向）——
    /// 这是"时间码真的读了拍号"的最小可判别形式。
    #[test]
    fn the_reading_at_one_tick_depends_on_the_time_signature_in_both_directions() {
        let four_four = TimecodeGrid::from_time_signature(4, 4).expect("4/4");
        let three_four = TimecodeGrid::from_time_signature(3, 4).expect("3/4");
        let tick = 3_840_u64;
        assert_eq!(four_four.timecode_at(tick), "002.01.000");
        assert_eq!(three_four.timecode_at(tick), "002.02.000");
        assert_ne!(four_four.timecode_at(tick), three_four.timecode_at(tick));
        // 反向：把两个网格对调，差异必须仍然成立（不是单向的巧合）。
        assert_ne!(three_four.timecode_at(tick), four_four.timecode_at(tick));
    }

    /// 判据 1d：**投影的读数**（`ViewState::timecode_at`）与工程拍号一致，
    /// 且模型层拒绝的拍号**如实**退回 tick 文本，不谎报 4/4。
    #[test]
    fn the_projection_reading_follows_the_project_time_signature_and_degrades_honestly() {
        for (numerator, denominator, ticks, expected) in [
            (4_u8, 4_u8, 8_000_u64, "003.01.320"),
            (3, 4, 8_000, "003.03.320"),
            (6, 8, 8_000, "003.05.320"),
        ] {
            let mut project = filled_project();
            project.time_signature = TimeSignature {
                numerator,
                denominator,
            };
            let view = ViewState::from_project(&project).expect("投影");
            assert_eq!(view.timecode_at(ticks), expected);
        }

        // 退化拍号（分母不能整除全音符 ⇒ 模型层 `ticks_per_bar` 拒绝）：
        // `ViewState` 仍然能投影（`bar_length_ticks` 只拒绝 0 分母），
        // 但时间码**退回 tick 文本** —— 不按写死的 4/4 编一个假的小节读数。
        let mut degenerate = filled_project();
        degenerate.time_signature = TimeSignature {
            numerator: 4,
            denominator: 7,
        };
        let view = ViewState::from_project(&degenerate).expect("退化拍号仍能投影");
        assert_eq!(view.timecode_grid(), None, "模型拒绝的拍号必须如实报 None");
        assert_eq!(view.timecode_at(8_000), "tick 8000");
        assert_eq!(timecode_for_ticks(None, 8_000), "tick 8000");
        // 反向对照：4/4 下**不是**这个文本（否则上面的断言可能因为恒等而空过）。
        assert_ne!(
            timecode_for_ticks(TimecodeGrid::from_time_signature(4, 4), 8_000),
            "tick 8000"
        );
    }

    /// 判据 1e：注入面（两个 `int`）的往返 —— 投影 → 窗口 → 重建 → 同一个读数。
    ///
    /// `from_injected` 是 `host::apply_transport` 唯一的重建入口，因此它的
    /// 边界（0 / 负数 / 一拍比一小节还长）必须**退化成 `None`**（⇒ tick 文本），
    /// 而不是 panic 或 wrap。
    #[test]
    fn the_injected_grid_round_trips_and_degenerates_safely() {
        let grid = TimecodeGrid::from_time_signature(3, 4).expect("3/4");
        let rebuilt = TimecodeGrid::from_injected(
            i32::try_from(grid.ticks_per_beat()).expect("i32"),
            i32::try_from(grid.ticks_per_bar()).expect("i32"),
        )
        .expect("往返必须成功");
        assert_eq!(rebuilt, grid);
        assert_eq!(rebuilt.timecode_at(3_840), "002.02.000");

        // 0（= 没有网格 / 模型拒绝该拍号）与畸形值一律退化。
        assert_eq!(TimecodeGrid::from_injected(0, 0), None);
        assert_eq!(TimecodeGrid::from_injected(960, 0), None);
        assert_eq!(TimecodeGrid::from_injected(-960, 3_840), None);
        assert_eq!(
            TimecodeGrid::from_injected(4_800, 3_840),
            None,
            "一拍不能比一小节还长"
        );
    }

    /// 判据 1: **同一工程两次投影逐字节相同**（`ARCH-DET-001` 的界面侧版本）。
    ///
    /// 这条判据抓三类漂移：浮点位置累加、`BTreeMap` 被换成 `HashMap`、
    /// 以及任何"看起来一样但字节不同"的格式化改动。
    #[test]
    fn two_projections_of_the_same_project_are_byte_identical() {
        for project in [demo_project(), filled_project(), default_project()] {
            let first = ViewState::from_project(&project).expect("投影");
            let second = ViewState::from_project(&project).expect("投影");
            assert_eq!(
                first.canonical_bytes(),
                second.canonical_bytes(),
                "同一工程的两次投影必须逐字节相同"
            );
            assert!(first.canonical_bytes().ends_with(b"\n"));
            // 中间夹一次别的投影 —— 顺序不得影响结果。
            let _other = ViewState::from_project(&filled_project()).expect("投影");
            let third = ViewState::from_project(&project).expect("投影");
            assert_eq!(first.canonical_lines(), third.canonical_lines());
        }
    }

    /// 判据 2: tick ↔ 像素**往返**精确（两个方向都测）。
    ///
    /// 这是"位置一律用整数 tick 计算"的可测形态：任何浮点实现都会在
    /// `ticks_per_pixel = 30` 上露馅（`tick_to_px(30, 30)` 会算成 0）。
    #[test]
    fn tick_to_pixel_round_trips_in_both_directions() {
        for tpp in [1_u64, 5, 30, 32, 60, 120, 960] {
            // 像素 → tick → 像素：对任意像素恒等。
            for px in [0_u32, 1, 2, 29, 30, 31, 1000, 65_535] {
                let tick = px_to_tick(px, tpp).expect("像素 → tick");
                assert_eq!(
                    tick_to_px(tick, tpp).expect("tick → 像素"),
                    px,
                    "px={px} tpp={tpp} 往返失败"
                );
            }
            // tick 是 tpp 的整数倍 → tick → 像素 → tick：恒等。
            for step in [0_u64, 1, 2, 17, 128, 4096] {
                let tick = step * tpp;
                let px = tick_to_px(tick, tpp).expect("tick → 像素");
                assert_eq!(
                    px_to_tick(px, tpp).expect("像素 → tick"),
                    tick,
                    "tick={tick} tpp={tpp} 往返失败"
                );
            }
        }
        // 30 不是 2 的幂 ⇒ 这条断言是"不许用浮点算位置"的探针。
        assert_eq!(tick_to_px(30, 30), Ok(1));
        assert_eq!(px_to_tick(1, 30), Ok(30));
    }

    /// 判据（`Z` / `Shift+Z` 的**算术**那一半）：`ticks_per_pixel_to_fit` 向上取整 + 夹紧。
    ///
    /// 期望值是**手算表**（不是拿实现算一遍再和实现比）：
    ///
    /// | 跨度 tick | 视口 px | `ceil(跨度/px)` | 夹紧后 | 备注 |
    /// | ---: | ---: | ---: | ---: | :--- |
    /// | 3840 | 1920 | 2 | 2 | 整除（4/4 一小节正好一屏的一半） |
    /// | 3841 | 1920 | 3 | 3 | 多 1 tick ⇒ 向上取整进一位（"装得下"） |
    /// | 1 | 1920 | 1 | 1 | 下界 |
    /// | 100000000 | 1920 | 52084 | 960 | 上界（夹紧的后果如实登记：仍装不下） |
    #[test]
    fn zooms_to_fit_are_ceiling_rounded_and_clamped() {
        assert_eq!(ticks_per_pixel_to_fit(3_840, 1_920), Some(2));
        assert_eq!(ticks_per_pixel_to_fit(3_841, 1_920), Some(3));
        assert_eq!(ticks_per_pixel_to_fit(1, 1_920), Some(1));
        assert_eq!(
            ticks_per_pixel_to_fit(100_000_000, 1_920),
            Some(MAX_TICKS_PER_PIXEL)
        );
        // 没有可作用对象 / 没有画布 ⇒ `None`（调用方据此**不消费**按键）。
        assert_eq!(
            ticks_per_pixel_to_fit(0, 1_920),
            None,
            "跨度为 0 = 没有对象"
        );
        assert_eq!(
            ticks_per_pixel_to_fit(3_840, 0),
            None,
            "视口宽 0 = 没有画布"
        );
        // 夹紧：0 与越界值都被收进 [MIN, MAX]（投影本体仍独立拒绝 0）。
        assert_eq!(clamp_ticks_per_pixel(0), MIN_TICKS_PER_PIXEL);
        assert_eq!(clamp_ticks_per_pixel(u64::MAX), MAX_TICKS_PER_PIXEL);
        assert_eq!(clamp_ticks_per_pixel(MIN_TICKS_PER_PIXEL), 1);
        assert_eq!(clamp_ticks_per_pixel(MAX_TICKS_PER_PIXEL), 960);
    }

    /// 判据：`ViewState` 发布的**选区跨度**与**内容末端**与模型读数一致。
    ///
    /// 口径是"投影读自己的音符"：跨度取选中身份的并集（`min 起点` / `max 终点`）。
    /// 这里对账的是**同一批**音符的两个来源：投影的 `notes` 与模型的 `clip_pool`。
    #[test]
    fn view_state_publishes_the_selection_span_and_the_content_end() {
        let project = filled_project();
        let view = ViewState::from_project(&project).expect("投影");
        assert!(view.notes.len() >= 2, "夹具必须有多个音符");
        let first = view.notes.first().expect("音符").clone();
        let last = view.notes.last().expect("音符").clone();
        assert_eq!(
            view.selection_tick_span(std::slice::from_ref(&first.id)),
            Some((first.start_tick, first.end_tick)),
            "单个音符的跨度就是它自己的区间"
        );
        assert_eq!(
            view.selection_tick_span(&[first.id.clone(), last.id.clone()]),
            Some((
                first.start_tick.min(last.start_tick),
                first.end_tick.max(last.end_tick)
            )),
            "多选 = 整个选区的并集跨度（不是第一个音符的跨度）"
        );
        assert_eq!(view.selection_tick_span(&[]), None, "空选区 ⇒ None");
        assert_eq!(
            view.selection_tick_span(&["01J8Z5Q0R7K3M9X2V4B6N8P0PZ".to_owned()]),
            None,
            "选中的身份不在工程里 ⇒ None"
        );
        // 内容末端：模型侧的独立口径（剪辑 `start + duration` 与段落 `end_tick` 的最大值）。
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
        assert_eq!(
            view.content_end_tick(),
            clip_end.max(section_end),
            "内容末端 = 剪辑末端 ∪ 段落末端的最大值（与铺标尺那次 max 同口径）"
        );
        assert!(view.content_end_tick() > 0, "filled 夹具必须有内容");
        // 空工程：没有任何可看的内容 ⇒ 0（`Shift+Z` 据此不消费）。
        assert_eq!(
            ViewState::from_project(&default_project())
                .expect("空工程")
                .content_end_tick(),
            0
        );
    }

    /// 判据 3: **0 音轨空工程不 panic**，且投影成"只有主总线"的视图
    /// （音轨行 / 剪辑 / 段落 / 场景全空，`master` 有且只有一条）。
    ///
    /// `#161` 起 `default_project()` 出厂自带 1 条主总线 ⇒ `tracks`（音轨行）仍为空，
    /// 但 `master` 是 `Some`。这正是"新建工程 → 加第一条轨"能走通的前提。
    #[test]
    fn empty_project_projects_to_a_master_only_view() {
        let view = ViewState::from_project(&default_project()).expect("空工程必须能投影");
        assert!(view.tracks.is_empty());
        let master = view
            .master
            .as_ref()
            .expect("`#161` 起空工程必须投影出主总线");
        assert!(master.is_master, "`master` 那一格必须标记为主总线");
        assert_eq!(master.name, "Master");
        assert_eq!(master.kind, "master");
        assert_eq!(master.index, 0);
        assert_eq!(master.row_height_px, 0, "主总线不占音轨行（没有行几何）");
        assert!(view.clips.is_empty());
        assert!(view.sections.is_empty());
        assert!(view.scenes.is_empty());
        assert!(view.note_ulids.is_empty());
        assert!(view.notes.is_empty());
        assert!(view.note_positions().is_empty());
        assert!(view.note_rows().is_empty());
        assert_eq!(view.ppq, 960);
        assert_eq!(view.time_signature_display, "4/4");
        assert_eq!(view.bpm_display, "120.00");
        // 有主总线 ≠ 空视图：`ViewState::empty()` 连主总线都没有（那是另一种状态）。
        assert_ne!(
            ViewState::empty().canonical_bytes(),
            view.canonical_bytes(),
            "空工程（0 音轨 ＋ 1 主总线）不得退化成完全空的视图"
        );
        // 标尺仍有 16 条（规范 §4.2 的最小可视密度）。
        assert_eq!(view.bar_positions.len(), MIN_BAR_COUNT);
        assert_eq!(view.bar_positions[0], 0.0);
    }

    /// 判据 4: **超长工程不溢出** —— 越界的 tick 让投影返回 `Err` 而不是 wrap / panic。
    #[test]
    fn absurd_tick_ranges_error_instead_of_overflowing() {
        let mut project = default_project();
        let track_id = demo_id("T9");
        let mut track = TrackV3 {
            id: track_id,
            name: "Overflow".to_owned(),
            ..TrackV3::default()
        };
        let clip_id = demo_id("K9");
        project.clip_pool.insert(
            clip_id,
            yeban_model::project::ClipPoolEntry {
                id: clip_id,
                name: "clip".to_owned(),
                content: ClipContent::default(),
            },
        );
        let placement_id = demo_id("Z9");
        track.clips.insert(
            placement_id,
            ClipPlacement {
                id: placement_id,
                clip_id,
                start_tick: u64::MAX - 1,
                duration_ticks: u64::MAX,
                ..ClipPlacement::default()
            },
        );
        project.tracks.insert(track_id, track);
        assert_eq!(
            ViewState::from_project(&project),
            Err(BridgeError::TickOverflow {
                start_tick: u64::MAX - 1
            }),
            "越界 tick 必须返回错误, 不得 wrap"
        );

        // 像素侧：像素 × tpp 越界。
        assert_eq!(
            px_to_tick(u32::MAX, u64::MAX),
            Err(BridgeError::TickBackOverflow { px: u32::MAX })
        );
        // tick 侧：商超出 u32。
        assert_eq!(
            tick_to_px(u64::MAX, 1),
            Err(BridgeError::PixelOverflow { tick: u64::MAX })
        );
        // 除零。
        assert_eq!(tick_to_px(0, 0), Err(BridgeError::ZeroTicksPerPixel));
        assert_eq!(px_to_tick(0, 0), Err(BridgeError::ZeroTicksPerPixel));
        assert_eq!(
            ViewState::from_project_with_zoom(&default_project(), 0),
            Err(BridgeError::ZeroTicksPerPixel)
        );
        // 拍号分母为 0。
        let mut broken = default_project();
        broken.time_signature.denominator = 0;
        assert_eq!(
            bar_length_ticks(broken.time_signature),
            Err(BridgeError::ZeroTimeSignatureDenominator)
        );
    }

    /// 判据 5: 演示工程是**合法工程**，且它的投影逐字复现 `src/scene.rs` 的常量。
    ///
    /// 这条是"演示数据已由投影层产出"的机械证明：改投影或改夹具都会变红。
    #[test]
    fn demo_projection_reproduces_the_scene_constants() {
        let project = demo_project();
        assert_eq!(project.validate(), Ok(()), "演示夹具必须是合法工程");
        assert_eq!(project.check_readable(), Ok(()));

        let view = ViewState::from_project(&project).expect("投影");
        assert_eq!(view.track_names(), owned(&crate::scene::TRACK_NAMES));
        assert_eq!(view.note_ulids, owned(&crate::scene::NOTE_ULIDS));
        assert_eq!(view.clip_ulids(), owned(&crate::scene::CLIP_ULIDS));
        assert_eq!(view.section_names(), owned(&crate::scene::SECTION_NAMES));
        assert_eq!(view.scene_names(), owned(&crate::scene::SCENE_NAMES));
        assert_eq!(
            view.tracks
                .iter()
                .map(|track| track.volume_display.clone())
                .collect::<Vec<_>>(),
            owned(&crate::scene::FADER_DB_LABELS)
        );
        assert_eq!(view.tracks.len(), crate::scene::TRACK_COUNT);
        assert_eq!(view.clips.len(), crate::scene::CLIP_COUNT);
        assert_eq!(view.note_ulids.len(), crate::scene::NOTE_COUNT);
        assert_eq!(view.sections.len(), crate::scene::SCENE_COUNT);
        assert_eq!(view.scenes.len(), crate::scene::SCENE_COUNT);
        assert_eq!(view.bpm_display, "120.00");
    }

    /// 判据（`ADR-0004` S0）：行几何是投影的**前缀和**，且 `.slint` 要读的四个数组
    /// 与它逐项一致 —— 界面因此不需要（也不许）自己做 `42px + 56px * i`。
    ///
    /// "数字不变"在这里是**逐位**断言：前缀和必须等于旧 `.slint` 的闭式
    /// `TRACK_LANE_TOP_PX + TRACK_LANE_HEIGHT_PX × i`，而旧 `.slint` 的 `54px` / `46px`
    /// / `+ 4px` 三个字面量分别等于投影的 `drawn_height()` / `clip_height()` / `clip_y()`
    /// 偏移。任何一处漂移都会让默认帧变字节 ⇒ 这条判据就是"逐字节不变"的投影侧证据。
    #[test]
    fn row_geometry_is_a_prefix_sum_that_the_slint_arrays_carry() {
        let project = demo_project();
        let view = ViewState::from_project(&project).expect("投影");
        let rows = track_rows(&project);
        assert_eq!(rows.len(), view.tracks.len());
        assert!(!rows.is_empty(), "演示工程必须有轨道");
        assert_eq!(view.track_ys().len(), view.tracks.len());
        assert_eq!(view.track_heights().len(), view.tracks.len());

        for (index, row) in rows.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let closed_form = crate::automation::TRACK_LANE_TOP_PX
                + crate::automation::TRACK_LANE_HEIGHT_PX * index as f32;
            assert_eq!(row.y, closed_form, "前缀和必须逐位等于旧 `.slint` 的闭式");
            assert_eq!(row.stride, crate::automation::TRACK_LANE_HEIGHT_PX);
            assert_eq!(view.track_ys()[index], row.y);
            assert_eq!(view.track_heights()[index], row.drawn_height());
            // 旧 `.slint` 包头 / 车道的 `height: 54px`（= 56 − 2）。
            assert_eq!(row.drawn_height(), 54.0);
        }
        // 行只能往下走（S1 的每轨高度依赖这条单调性）。
        assert!(rows.windows(2).all(|pair| pair[0].y < pair[1].y));

        // 剪辑：旧 `.slint` 的 `+ 4px` 与 `height: 46px`。
        assert!(!view.clips.is_empty(), "演示工程必须有剪辑");
        assert_eq!(view.clip_ys().len(), view.clips.len());
        assert_eq!(view.clip_heights().len(), view.clips.len());
        for clip in &view.clips {
            assert_eq!(clip.y, rows[clip.track_index].y + CLIP_ROW_INSET_PX);
            assert_eq!(clip.height, 46.0);
        }
        // 主总线在编排视图没有行 —— 几何必须显式为 0（不是编造一行）。
        if let Some(master) = &view.master {
            assert_eq!(master.y, 0.0);
            assert_eq!(master.height, 0.0);
        }
    }

    /// 判据 6: 字段映射 —— `filled_project()` 的每一个被投影用到的模型字段都进了视图。
    #[test]
    fn filled_project_maps_every_model_field_the_view_consumes() {
        let project = filled_project();
        let view = ViewState::from_project(&project).expect("投影");

        assert_eq!(view.title, project.title);
        assert_eq!(view.bpm_display, format!("{:.2}", project.bpm));
        assert_eq!(view.bpm_millis, 128_000);
        assert_eq!(view.time_signature_display, "4/4");
        assert_eq!(view.project_id, project.id.to_canonical_string());

        // 主总线被单独挑出，不进 `tracks`。
        let master = view.master.as_ref().expect("filled_project 有主总线");
        assert!(master.is_master);
        assert_eq!(master.kind, "master");
        assert_eq!(view.tracks.len(), project.tracks.len() - 1);

        // 逐字段对账：名称 / 类型 / 颜色 / 静音 / 独奏 / 音量 / 声相 / 摆放数。
        let model_tracks: Vec<&TrackV3> = project
            .tracks
            .values()
            .filter(|track| track.id != project.master_bus_track_id)
            .collect();
        for (view_track, model_track) in view.tracks.iter().zip(model_tracks) {
            assert_eq!(view_track.id, model_track.id.to_canonical_string());
            assert_eq!(view_track.name, model_track.name);
            assert_eq!(view_track.kind, kind_name(model_track.kind));
            assert_eq!(view_track.color, model_track.color);
            assert_eq!(view_track.mute, model_track.mute);
            assert_eq!(view_track.solo, model_track.solo);
            assert_eq!(view_track.volume_db, model_track.volume_db);
            assert_eq!(
                view_track.volume_display,
                format!("{:.1}", model_track.volume_db)
            );
            assert_eq!(view_track.clip_count, model_track.clips.len());
            #[allow(clippy::cast_possible_truncation)]
            let pan_millis = (f64::from(model_track.pan) * 1000.0).round() as i32;
            assert_eq!(view_track.pan_millis, pan_millis);
        }

        // 剪辑：起止 tick、身份、标签、内容类型、车道全部来自模型。
        assert_eq!(view.clips.len(), 2);
        let lead = view
            .tracks
            .iter()
            .find(|track| track.name == "Lead")
            .expect("filled_project 有 Lead 轨");
        let lead_id = EntityId::from_str(&lead.id).expect("视图 ID 必须是合法 ULID");
        let placement = project
            .tracks
            .get(&lead_id)
            .and_then(|track| track.clips.values().next())
            .expect("Lead 轨有一个摆放");
        let clip = view
            .clips
            .iter()
            .find(|clip| clip.placement_id == placement.id.to_canonical_string())
            .expect("摆放必须进视图");
        assert_eq!(clip.start_tick, placement.start_tick);
        assert_eq!(
            clip.end_tick,
            placement.start_tick + placement.duration_ticks
        );
        assert_eq!(clip.track_index, lead.index);
        assert_eq!(clip.track_name, "Lead");
        assert_eq!(clip.content, "midi");
        assert_eq!(clip.clip_name, "Clip");
        assert_eq!(clip.label, "Lead · Clip");
        assert_eq!(clip.lane, i32::try_from(lead.index).unwrap());

        // 段落 / 场景 / 音符身份逐条来自模型。
        assert_eq!(view.sections.len(), project.sections.len());
        assert_eq!(view.sections[0].name, "Intro");
        assert_eq!(view.sections[0].color.as_deref(), Some("#22AA88"));
        assert_eq!(view.scenes.len(), project.scenes.len());
        assert_eq!(view.scenes[0].name, "Scene 1");
        assert_eq!(view.scenes[0].tempo, Some(128.0));
        assert_eq!(view.note_ulids.len(), 4);
        assert_eq!(
            view.note_velocities.len(),
            view.note_ulids.len(),
            "力度必须与音符身份逐个对齐"
        );
        for velocity in &view.note_velocities {
            assert!((0.0..=1.0).contains(velocity), "归一化力度越界: {velocity}");
            assert!(
                (velocity - 100.0_f32 / 127.0).abs() < 1e-6,
                "filled_project 的音符力度是 100/127, 实测 {velocity}"
            );
        }
        for ulid in &view.note_ulids {
            assert!(
                crate::scene::is_ulid_text(ulid),
                "`{ulid}` 不是合法 ULID 文本"
            );
        }
    }

    /// 判据 7: 位置是**整数派生**的 —— 与独立算出的 `tick / tpp` 逐个相等，
    /// 且小节线严格等距、段落 x 恰为前面宽度之和（不存在累加漂移）。
    #[test]
    fn positions_are_integer_derived_and_bars_are_equidistant() {
        let project = demo_project();
        let view = ViewState::from_project_with_zoom(&project, 30).expect("投影");

        for clip in &view.clips {
            let expected = tick_to_px(clip.start_tick, 30).expect("tick → 像素");
            assert_eq!(clip.x, as_px(expected), "剪辑 x 必须是 tick/tpp 的整数商");
            let expected_end = tick_to_px(clip.end_tick, 30).expect("tick → 像素");
            assert_eq!(
                clip.width,
                as_px(expected_end - expected).max(MIN_BLOCK_WIDTH_PX)
            );
        }
        for section in &view.sections {
            let expected = tick_to_px(section.start_tick, 30).expect("tick → 像素");
            assert_eq!(section.x, as_px(expected));
        }

        // 小节线等距：相邻差恒等于 bar_length_ticks / tpp。
        let bar_px = as_px(u32::try_from(view.bar_length_ticks / view.ticks_per_pixel).unwrap());
        for window in view.bar_positions.windows(2) {
            assert_eq!(window[1] - window[0], bar_px);
        }
        // 一节的 x 恰好是前几节宽度之和（整数派生 ⇒ 可加）。
        let mut cursor = 0.0_f32;
        for section in &view.sections {
            assert_eq!(section.x, cursor, "段落 x 必须等于前面宽度的整数和");
            cursor += section.width;
        }
    }

    /// 判据 8: 音符身份来自 `clip_pool`，顺序确定（片段键序 → 音符键序）且去重；
    /// 音频片段不贡献音符。
    #[test]
    fn note_ulids_come_from_the_clip_pool_in_a_deterministic_order() {
        let view = ViewState::from_project(&demo_project()).expect("投影");
        let mut sorted = view.note_ulids.clone();
        sorted.sort();
        assert_eq!(view.note_ulids, sorted, "音符身份必须按键序给出");
        let unique: BTreeSet<&String> = view.note_ulids.iter().collect();
        assert_eq!(unique.len(), view.note_ulids.len(), "音符身份不得重复");
        // 音频片段不贡献音符：filled_project 的两条音频摆放与四个 MIDI 音符。
        let filled = ViewState::from_project(&filled_project()).expect("投影");
        assert_eq!(filled.note_ulids.len(), 4);
        assert_eq!(filled.clips.len(), 2);
    }

    /// 判据 9: 速度 / 拍号 / 小节长度的投影（含边界与非法输入）。
    #[test]
    fn tempo_and_time_signature_projection_covers_the_boundaries() {
        let mut project = default_project();
        project.bpm = 999.0;
        project.time_signature = TimeSignature {
            numerator: 7,
            denominator: 8,
        };
        let view = ViewState::from_project(&project).expect("投影");
        assert_eq!(view.bpm_display, "999.00");
        assert_eq!(view.bpm_millis, 999_000);
        assert_eq!(view.time_signature_display, "7/8");
        // 7/8: 960 × 7 × 4 ÷ 8 = 3360。
        assert_eq!(view.bar_length_ticks, 3360);

        project.bpm = 20.0;
        let view = ViewState::from_project(&project).expect("投影");
        assert_eq!(view.bpm_display, "20.00");
        assert_eq!(view.bpm_millis, 20_000);

        // 非有限值不 panic（模型 `validate()` 会拒绝它，但投影本身也必须稳健）。
        project.bpm = f64::NAN;
        let view = ViewState::from_project(&project).expect("投影");
        assert_eq!(view.bpm_millis, 0);
    }

    // =====================================================================
    // app-completion 工作线新增判据（①②③）
    // =====================================================================

    /// 判据 10: **音符 tick → 像素是整数口径**（`[UI-NOTE-002]` 的投影侧一半）。
    ///
    /// 逐音符断言 `x == tick_to_px(start_tick)`、`width == max(1, tick_to_px(end) - x)`。
    /// `ticks_per_pixel` 覆盖 **非 2 的幂**（3 / 7 / 30）与极端缩放（1 / 960）：
    /// 这一族数字就是"不许用浮点算位置"的探针（30 与 7 都除不尽）。
    #[test]
    fn scrolling_never_selects_an_empty_window_across_the_gates_600_frames() {
        // 门禁用例每帧把滚动推进 `viewport_width/120` px（1 屏/秒）。若某个窗口被裁空,
        // 那一帧的见证断言（`min_evidence > 0`）会红 —— 这条判据在**投影层**先把它挡掉,
        // 不必等 27 分钟的 CI 作业。
        let project = yeban_model::samples::project_with_notes(100_000);
        let view = ViewState::from_project_with_zoom(&project, 120).expect("投影");
        let width = 1920.0_f32;
        for frame in 0..600 {
            let scroll = frame as f32 * (width / 120.0);
            assert!(
                !view.visible_notes(scroll, width).is_empty(),
                "第 {frame} 帧的窗口被裁空了 (scroll={scroll}) —— 门禁的见证断言会红"
            );
        }
    }

    #[test]
    fn clipping_selects_a_small_fraction_of_the_100k_scene() {
        // 判据: 在**真实滚动场景**下, 1920px 窗口只应选中极小比例的音符。
        // 为什么这条必须有: 第 184 轮发现早先的夹具把 10 万音符挤在约 128px 内 ⇒ 裁剪一个都裁不掉,
        // 量到的是病态场景。夹具改成铺开在长时轴（每 240 tick 一个）之后, 这条才成立。
        let project = yeban_model::samples::project_with_notes(100_000);
        let view = ViewState::from_project_with_zoom(&project, 120).expect("投影");
        let total = view.note_positions().len();
        assert_eq!(total, 100_000, "场景必须是 10 万音符");
        let vis = view.visible_notes(0.0, 1920.0);
        assert!(!vis.is_empty(), "首个窗口必须有音符");
        assert!(
            vis.len() * 20 < total,
            "1920px 窗口应只选中不到 5%: 可见 {} / 总 {}",
            vis.len(),
            total
        );
    }

    #[test]
    fn clip_at_tick_is_half_open_and_refuses_gaps() {
        // 判据（第 234 轮规则的直接后果）: 半开区间; 末尾**不算**在内; 空隙 ⇒ None;
        // 空摆放 ⇒ None（调用方据此**拒绝**编辑, 而不是造片段）。
        let id_a = yeban_model::ids::EntityId::new();
        let id_b = yeban_model::ids::EntityId::new();
        let placements = vec![(960_u64, 960_u64, id_a), (4_800, 480, id_b)];
        assert_eq!(clip_at_tick(&placements, 960), Some(id_a), "起点算在内");
        assert_eq!(
            clip_at_tick(&placements, 1_919),
            Some(id_a),
            "末端前一 tick 在内"
        );
        assert_eq!(
            clip_at_tick(&placements, 1_920),
            None,
            "末端**不算**在内（半开）"
        );
        assert_eq!(clip_at_tick(&placements, 0), None, "第一个片段之前是空隙");
        assert_eq!(clip_at_tick(&placements, 3_000), None, "两段之间是空隙");
        assert_eq!(clip_at_tick(&placements, 4_800), Some(id_b));
        assert_eq!(clip_at_tick(&placements, u64::MAX), None);
        assert_eq!(clip_at_tick(&[], 960), None, "空摆放必须拒绝而不是猜");
    }

    #[test]
    fn planning_a_note_produces_an_operation_that_inserts_exactly_one() {
        // 判据（第 232 轮第 1 步）：`NotePlan` 变出的 op **应用后该片段音符数恰 +1**,
        // 且新增音符的 tick/pitch/时值与 plan **逐项相等** —— 这是"决策"与"模型改动"之间的接口。
        let mut project = yeban_model::samples::filled_project();
        let view = ViewState::from_project(&project).expect("投影");
        let plan = NotePlan {
            start_tick: 1_440,
            pitch: 64,
            duration_ticks: view.ppq,
        };
        // track/clip 身份取自**工程本身**, 不编造。
        let track_id = *project.tracks.keys().next().expect("工程必须有轨道");
        let clip_id = *project.clip_pool.keys().next().expect("工程必须有片段");
        let note_id = yeban_model::ids::EntityId::new();
        let notes_before = project
            .clip_pool
            .get(&clip_id)
            .and_then(|entry| entry.content.notes())
            .map_or(0, std::collections::BTreeMap::len);

        let op = plan_to_add_note(plan, track_id, clip_id, note_id);
        op.apply(&mut project).expect("插入必须成功");

        let notes_after = project
            .clip_pool
            .get(&clip_id)
            .and_then(|entry| entry.content.notes())
            .map_or(0, std::collections::BTreeMap::len);
        assert_eq!(notes_after, notes_before + 1, "应用后音符数必须恰 +1");
        let inserted = project
            .clip_pool
            .get(&clip_id)
            .and_then(|entry| entry.content.notes())
            .and_then(|notes| notes.get(&note_id))
            .expect("新音符必须可按键取回");
        assert_eq!(inserted.start_tick, plan.start_tick);
        assert_eq!(inserted.pitch, plan.pitch);
        assert_eq!(inserted.duration_ticks, plan.duration_ticks);
    }

    #[test]
    fn lane_and_pitch_are_inverses_and_the_pencil_plan_agrees_with_both() {
        // 判据: ① `pitch_for_lane(pitch_lane(p))` 回到**同一泳道**（在 16 条内）;
        // ② `lane_at_y(pitch_lane_y(p))` 也回到同一泳道 —— 两条反向路径必须一致;
        // ③ 泳道外 ⇒ None; ④ 铅笔规划出的音高, 其泳道必须正是点击 y 所在的那条;
        // ⑤ 规划出的 tick 必须是网格倍数。
        for pitch in 0..=u8::MAX {
            let lane = pitch_lane(pitch);
            if let Some(back) = pitch_for_lane(lane) {
                assert_eq!(pitch_lane(back), lane, "泳道往返不一致: pitch={pitch}");
            }
            assert_eq!(
                lane_at_y(pitch_lane_y(pitch)),
                Some(lane),
                "y 反推的泳道与 pitch_lane 不一致: pitch={pitch}"
            );
        }
        assert_eq!(pitch_for_lane(-1), None);
        assert_eq!(pitch_for_lane(PITCH_LANE_COUNT), None);
        assert_eq!(lane_at_y(-100.0), None);
        assert_eq!(lane_at_y(10_000.0), None);

        let view = ViewState::from_project_with_zoom(&filled_project(), 120).expect("投影");
        let grid = 240_u64;
        let mut planned = 0_usize;
        for lane in 0..PITCH_LANE_COUNT {
            let y = pitch_lane_y(pitch_for_lane(lane).expect("车道有效")) + 1.0;
            let plan = view
                .pencil_plan(0.0, 480.0, y, grid)
                .expect("车道内必能规划");
            assert_eq!(
                pitch_lane(plan.pitch),
                lane,
                "规划出的音高必须落在被点的那条泳道"
            );
            assert_eq!(plan.start_tick % grid, 0, "规划的 tick 必须是网格倍数");
            planned += 1;
        }
        assert_eq!(planned, PITCH_LANE_COUNT as usize, "16 条泳道都要能规划");
    }

    #[test]
    fn velocity_bar_hit_test_finds_the_bar_and_ignores_zero_velocity() {
        // 判据: ① 落在某根柱上 ⇒ 返回该音符; ② 车道内空白 ⇒ None;
        // ③ 力度 0 的音符**没有可见柱**（高 0）⇒ 点在它该在的横坐标上也不得命中;
        // ④ 与音符命中同一口径: 重叠取最后绘制者（此处只断言"返回可见集合里的下标"）;
        // ⑤ 本切片：柱高映射的**反函数** `velocity_at_lane_y` 与投影口径互逆。
        let view = ViewState::from_project_with_zoom(&filled_project(), 120).expect("投影");
        // 几何走**生产唯一构造点**（`30.0` 现在有名字了：`VELOCITY_BAR_OFFSET_X_PX`）。
        let geometry = VelocityLaneGeometry::roll_velocity_lane();
        let width = 1920.0_f32;
        let mut hit_count = 0_usize;
        let mut zero_velocity_probed = 0_usize;
        for index in view.notes_visible_in(0.0, width) {
            let note = &view.notes[index];
            let v = note.velocity_normalized;
            let left = geometry.offset_x + note.x + geometry.bar_width / 2.0;
            let top =
                geometry.lane_height - VELOCITY_LANE_BOTTOM_MARGIN_PX - geometry.bar_max_height * v;
            if v <= 0.0 {
                // ③ 零力度：柱高为 0，而命中测试的**两端都是闭区间** ⇒ 只有恰好落在 `top`
                //    这**一条线**上才命中。⚠ 夹具（`filled_project`）里**没有**零力度音符
                //    ⇒ 本分支从不执行（`zero_velocity_probed` 把它机械地报出来，而不是让读者
                //    以为这条路径被覆盖了）。生产侧因此**显式排除**力度 0
                //    （`host::velocity_bar_at`），⛔ 不依赖这条 1px 线。
                zero_velocity_probed += 1;
                let got = view.velocity_bar_hit_test(0.0, width, left, top, geometry);
                assert_eq!(
                    got,
                    Some(index),
                    "零力度柱的命中区间退化成 `y == top` 这一条线（闭区间）—— 这里如实钉住它"
                );
                continue;
            }
            let got = view.velocity_bar_hit_test(0.0, width, left, top + 1.0, geometry);
            assert!(got.is_some(), "柱内一点必须命中某根柱");
            let got = got.expect("上面已断言");
            assert!(
                view.notes_visible_in(0.0, width).contains(&got),
                "命中必须落在可见集合内"
            );
            hit_count += 1;
        }
        assert!(
            hit_count > 0,
            "夹具至少要有一个非零力度的可见音符, 否则判据空转"
        );
        assert_eq!(
            zero_velocity_probed, 0,
            "本夹具**没有**零力度音符 ⇒ 上面那条分支不执行；若有人往夹具里加了零力度音符，\
             这里会立刻说出来（那条断言钉的是既有的 1px 线行为）"
        );
        // ② 车道底部空白（所有柱都在其上）⇒ 不命中。
        assert_eq!(
            view.velocity_bar_hit_test(0.0, width, geometry.offset_x, 999.0, geometry),
            None
        );
        // ⑤ 柱高映射的反函数（力度臂用的就是它）。
        let bottom = geometry.lane_height - VELOCITY_LANE_BOTTOM_MARGIN_PX;
        assert_eq!(geometry.velocity_at_lane_y(bottom), Some(0), "柱底沿 = 0");
        assert_eq!(
            geometry.velocity_at_lane_y(bottom - geometry.bar_max_height),
            Some(127),
            "满柱顶沿 = 127"
        );
        assert_eq!(
            geometry.velocity_at_lane_y(-999.0),
            Some(127),
            "上方越界夹到 127"
        );
        assert_eq!(
            geometry.velocity_at_lane_y(999.0),
            Some(0),
            "下方越界夹到 0"
        );
        assert_eq!(
            geometry.velocity_at_lane_y(f32::NAN),
            None,
            "非有限 y ⇒ None（⛔ 不静默当成 0）"
        );
        for index in view.notes_visible_in(0.0, width) {
            let note = &view.notes[index];
            let top = bottom - geometry.bar_max_height * note.velocity_normalized;
            assert_eq!(
                geometry.velocity_at_lane_y(top),
                Some(note.velocity),
                "每个音符自己那根柱的顶沿必须反解回它自己的力度（与投影的归一化口径互逆）"
            );
        }
    }

    #[test]
    fn snapped_tick_at_maps_the_viewport_to_grid_multiples() {
        // 判据: ① 结果是网格倍数（grid>0）; ② 同一 x 在滚动前后**相差**滚动量对应的 tick;
        // ③ grid=0 ⇒ 不吸附（与 `snap_tick` 同一口径）; ④ 出界不 panic 且给 0。
        let view = ViewState::from_project_with_zoom(&filled_project(), 120).expect("投影");
        for grid in [0_u64, 120, 240, 960] {
            for x in [0.0_f32, 37.0, 480.0, 1920.0] {
                let tick = view.snapped_tick_at(0.0, x, grid);
                if grid > 0 {
                    assert_eq!(tick % grid, 0, "grid={grid} x={x} ⇒ 不是网格倍数");
                }
            }
        }
        // 滚动把同一个 x 推到**更晚**的 tick（单调, 且差值等于滚动像素对应的 tick）。
        let a = view.snapped_tick_at(0.0, 100.0, 960);
        let b = view.snapped_tick_at(1920.0, 100.0, 960);
        assert!(b > a, "滚动后同一 x 必须对应更晚的 tick: {a} vs {b}");
        let delta_px = 1920_u64;
        let expected = snap_tick(
            px_to_tick(100, view.ticks_per_pixel).unwrap()
                + px_to_tick(u32::try_from(delta_px).unwrap(), view.ticks_per_pixel).unwrap(),
            960,
        );
        assert_eq!(b, expected, "滚动量的换算必须与投影同一口径");
        // grid=0 ⇒ 与不吸附一致。
        assert_eq!(
            view.snapped_tick_at(0.0, 100.0, 0),
            px_to_tick(100, view.ticks_per_pixel).unwrap()
        );
        // 出界: 非有限或超出 u32 ⇒ 0（不 panic）。
        assert_eq!(view.snapped_tick_at(0.0, f32::INFINITY, 960), 0);
        assert_eq!(view.snapped_tick_at(0.0, f32::NAN, 960), 0);
    }

    #[test]
    fn hit_test_finds_the_note_under_a_point_and_only_that_one() {
        // 判据: ① 音符中心必命中它自己; ② 空白必不命中; ③ 窗口外必不命中;
        // ④ 重叠时返回**最后绘制**的那个（口径写明在函数文档里）。
        let view = ViewState::from_project_with_zoom(&filled_project(), 120).expect("投影");
        let width = 1920.0_f32;
        let height = 6.0_f32;
        let visible = view.visible_notes(0.0, width);
        assert!(!visible.is_empty(), "夹具在首个窗口内必须有音符");

        // ① 每个可见音符的中心都必须命中它自己 —— 除非它与**更后**的音符重叠（重叠时以下是"最后者胜"）。
        let mut checked = 0_usize;
        for (slot, index) in view.notes_visible_in(0.0, width).iter().enumerate() {
            let note = &view.notes[*index];
            let cx = note.x + note.width / 2.0;
            let cy = note.y + height / 2.0;
            let got = view
                .hit_test_visible(0.0, width, cx, cy, height)
                .expect("中心必命中");
            let later_overlaps = view.notes[got].x >= note.x && got != *index;
            assert!(
                got == *index || later_overlaps,
                "第 {slot} 个可见音符的中心命中了 {got}, 既不是它自己也没有更后的音符重叠"
            );
            checked += 1;
        }
        assert!(checked > 0, "至少检验一个音符");

        // ② 明显空白（远在音符上方/下方）⇒ 不命中。
        assert_eq!(view.hit_test_visible(0.0, width, 5.0, -50.0, height), None);
        assert_eq!(
            view.hit_test_visible(0.0, width, 5.0, 9_999.0, height),
            None
        );
        // ③ 窗口之外 ⇒ 不命中（第 4 参数意义不大, 重点是 x 超出可见集合）。
        let far = view.notes_visible_in(0.0, width).last().copied();
        if let Some(last) = far {
            let beyond = view.notes[last].x + 10_000.0;
            assert_eq!(
                view.hit_test_visible(0.0, width, beyond, view.notes[last].y + 1.0, height),
                None
            );
        }
    }

    #[test]
    fn snap_tick_is_a_multiple_within_half_a_grid_and_rounds_ties_up() {
        // 定义性判据（最强的一条）：结果**必然是网格倍数**, 且与输入的差**不超过半个网格**。
        for grid in [1_u64, 120, 240, 480, 960] {
            for tick in [
                0_u64, 1, 119, 120, 121, 239, 240, 479, 480, 481, 959, 960, 961, 12_345,
            ] {
                let snapped = snap_tick(tick, grid);
                assert_eq!(snapped % grid, 0, "tick={tick} grid={grid} ⇒ 结果不是倍数");
                let diff = tick.abs_diff(snapped);
                assert!(
                    diff <= grid / 2,
                    "tick={tick} grid={grid} ⇒ 偏差 {diff} 超过半个网格"
                );
            }
        }
        // 已在网格上 ⇒ 不动。
        assert_eq!(snap_tick(1920, 960), 1920);
        // 平局 ⇒ **向上**（更晚）—— 这是口径, 不是巧合。
        assert_eq!(snap_tick(480, 960), 960);
        // 贴近前一个网格 ⇒ 向下。
        assert_eq!(snap_tick(100, 960), 0);
        assert_eq!(snap_tick(860, 960), 960);
        // 不吸附: 原样返回, 且不除零。
        assert_eq!(snap_tick(12_345, 0), 12_345);
        // 大 tick 不回绕。
        assert!(snap_tick(u64::MAX, 960) >= u64::MAX - 960);
        // 单调不减: 吸附不能把后面的 tick 拉到前面的前面。
        let mut previous = 0_u64;
        for tick in (0..10_000).step_by(37) {
            let snapped = snap_tick(tick, 240);
            assert!(snapped >= previous, "tick={tick} ⇒ 非单调");
            previous = snapped;
        }
    }

    #[test]
    fn visible_pitch_range_covers_every_pitch_the_roll_can_draw() {
        // 纵向类比（第 459 轮 tick 判据的纵向版）：泳道号落在 `[0, lane_count)` 的音高,
        // **必须**落在返回的音高范围内 —— 否则卷帘画得出的音符会跑到"视口范围"之外。
        let view = ViewState::from_project_with_zoom(&filled_project(), 120).expect("投影");
        let lane_count = 16; // 与 `piano_roll.slint` 的 `for lane_index in 16` 一致
        let (min_pitch, max_pitch) = view.visible_pitch_range(lane_count);
        assert!(
            min_pitch <= max_pitch,
            "范围必须有序: {min_pitch}..{max_pitch}"
        );
        let mut covered = 0_usize;
        for pitch in 0..=u8::MAX {
            let lane = pitch_lane(pitch);
            if lane >= 0 && lane < lane_count {
                assert!(
                    pitch >= min_pitch && pitch <= max_pitch,
                    "音高 {pitch}（泳道 {lane}）应可见, 却不在范围 {min_pitch}..{max_pitch} 内"
                );
                covered += 1;
            }
        }
        assert!(covered > 0, "16 条泳道必须覆盖至少一个音高, 否则判据空转");
        // 反向: 范围之外**不得**有可见泳道（否则范围偏大, 等于没裁）。
        for pitch in 0..=u8::MAX {
            if pitch < min_pitch || pitch > max_pitch {
                let lane = pitch_lane(pitch);
                assert!(
                    lane < 0 || lane >= lane_count,
                    "音高 {pitch} 在范围外, 却落在可见泳道 {lane}"
                );
            }
        }
    }

    #[test]
    fn visible_tick_range_agrees_with_the_clipping_index_set() {
        // `[UI-NOTE-001]` 步骤 ① 的口径判据：索引集说"可见"的音符, 必须与 tick 范围**重叠**。
        // 在像素空间比对会退化成同义反复（裁剪本来就是像素比较）, 所以这里比 tick。
        let view = ViewState::from_project_with_zoom(&filled_project(), 120).expect("投影");
        let width = 1920.0_f32;
        let mut checked = 0_usize;
        for scroll in [0.0_f32, 500.0, 2000.0] {
            let (min_tick, max_tick) = view.visible_tick_range(scroll, width);
            assert!(min_tick <= max_tick, "范围必须有序: {min_tick}..{max_tick}");
            for index in view.notes_visible_in(scroll, width) {
                let note = &view.notes[index];
                assert!(
                    note.end_tick >= min_tick && note.start_tick <= max_tick,
                    "音符 {index} ({start}..{end}) 被判可见, 却与 tick 范围 {min}..{max} 不相交 (scroll={scroll})",
                    start = note.start_tick,
                    end = note.end_tick,
                    min = min_tick,
                    max = max_tick,
                );
                checked += 1;
            }
        }
        assert!(
            checked > 0,
            "三个窗口合计必须至少检验到一个音符, 否则判据空转"
        );
    }

    #[test]
    fn visible_notes_keeps_the_parallel_arrays_aligned() {
        // 判据: 六个平行数组**同长**, 且第 k 个可见音符的每个字段都等于源 `notes[idx[k]]` 的同名字段。
        // 若有人改成"每个数组各自裁剪", 下标错位会让这条红。
        let view = ViewState::from_project_with_zoom(&filled_project(), 120).expect("投影");
        // 窗口取**首个音符的位置**起 400px：保证非空, 否则判据无从检验（实测 (200,400) 在该夹具下是空的）。
        let first = view.note_positions()[0];
        let idx = view.notes_visible_in(first, 400.0);
        let vis = view.visible_notes(first, 400.0);
        assert!(!idx.is_empty(), "该窗口应有音符, 否则本判据没有意义");
        assert_eq!(vis.len(), idx.len(), "可见数必须等于索引集长度");
        assert_eq!(vis.positions.len(), vis.widths.len());
        assert_eq!(vis.positions.len(), vis.ys.len());
        assert_eq!(vis.positions.len(), vis.rows.len());
        for (k, i) in idx.iter().enumerate() {
            // 位置是**相对视口**的 ⇒ 期望值要减去 left（见 `visible_notes` 文档）。
            assert_eq!(vis.positions[k], view.notes[*i].x - first);
            assert_eq!(vis.widths[k], view.notes[*i].width);
            assert_eq!(vis.ys[k], view.notes[*i].y);
            assert_eq!(vis.rows[k], view.notes[*i].row);
            assert_eq!(vis.ulids[k], view.notes[*i].id);
            assert_eq!(vis.velocities[k], view.notes[*i].velocity_normalized);
        }
        // 六个平行数组**同长**（第 181 轮更正：不是四个）。
        assert_eq!(vis.positions.len(), vis.ulids.len());
        assert_eq!(vis.positions.len(), vis.velocities.len());
    }

    #[test]
    fn notes_visible_in_matches_a_brute_force_window_and_actually_clips() {
        // 判据 1: 与**暴力过滤**逐项相同（不是"大约"）。
        // 判据 2（牙）: 窄窗口必须真的裁掉东西 —— 若实现退化成"全返回", 这一条红。
        let view = ViewState::from_project_with_zoom(&filled_project(), 120).expect("投影");
        let xs = view.note_positions();
        let ws = view.note_widths();
        let total = xs.len();
        assert!(total > 0, "夹具必须有音符");

        for (scroll, width) in [
            (0.0_f32, 100.0_f32),
            (50.0, 200.0),
            (1000.0, 300.0),
            (0.0, 10_000.0),
        ] {
            let want: Vec<usize> = (0..total)
                .filter(|&i| {
                    let left = scroll.max(0.0);
                    let right = left + width.max(0.0);
                    xs[i] + ws[i] >= left && xs[i] <= right
                })
                .collect();
            assert_eq!(
                view.notes_visible_in(scroll, width),
                want,
                "窗口 scroll={scroll} width={width} 的裁剪结果与暴力过滤不符"
            );
        }

        let narrow = view.notes_visible_in(0.0, 1.0);
        assert!(
            narrow.len() < total,
            "1 像素窗口返回了全部 {total} 个音符 ⇒ 裁剪没有生效"
        );
    }

    #[test]
    fn note_positions_are_integer_derived_from_ticks() {
        for project in [demo_project(), filled_project()] {
            for tpp in [1_u64, 3, 7, 30, 32, 120, 960] {
                let view = ViewState::from_project_with_zoom(&project, tpp).expect("投影");
                assert!(!view.notes.is_empty(), "夹具必须有音符");
                for note in &view.notes {
                    let start_px = tick_to_px(note.start_tick, tpp).expect("tick → 像素");
                    assert_eq!(
                        note.x,
                        as_px(start_px),
                        "音符 x 必须是 tick / tpp 的整数商 (tpp={tpp})"
                    );
                    let end_px = tick_to_px(note.end_tick, tpp).expect("tick → 像素");
                    assert_eq!(
                        note.width,
                        as_px(end_px - start_px).max(MIN_BLOCK_WIDTH_PX),
                        "音符宽必须是整数像素差 + 显式地板 (tpp={tpp})"
                    );
                    assert!(note.width >= MIN_BLOCK_WIDTH_PX);
                }
            }
        }
        // 非 2 的幂的缩放上，整数除法的**精确**结果（浮点实现会在这里露馅）。
        assert_eq!(tick_to_px(30, 30), Ok(1));
        assert_eq!(tick_to_px(7, 7), Ok(1));
        assert_eq!(tick_to_px(6, 7), Ok(0));
    }

    /// 判据 11: 音高 → 车道是**整数、单调、钳制**的映射，任何 `u8` 都不 panic。
    ///
    /// 单调性口径：音高越大 ⇒ 车道索引越小（屏幕上越靠上）；窗口内严格递减，
    /// 窗口外（`< 60` 或 `>= 76`）钳制到两端的车道。
    #[test]
    fn note_rows_follow_pitch_monotonically_with_clamping() {
        for pitch in 0..=u8::MAX {
            let row = pitch_lane(pitch);
            assert!(
                (0..PITCH_LANE_COUNT).contains(&row),
                "pitch={pitch} 的车道 {row} 越界"
            );
            assert!((0.0..).contains(&pitch_lane_y(pitch)));
        }
        // 窗口内严格递减 + 每一档恰好差一条车道。
        for pitch in
            PITCH_LANE_BASE..(PITCH_LANE_BASE + u8::try_from(PITCH_LANE_COUNT).unwrap() - 1)
        {
            assert_eq!(
                pitch_lane(pitch + 1),
                pitch_lane(pitch) - 1,
                "pitch={pitch}"
            );
        }
        // 两端钳制（不绕回、不 panic）：这是"越界音高"的**明确**语义。
        assert_eq!(pitch_lane(0), PITCH_LANE_COUNT - 1);
        assert_eq!(pitch_lane(PITCH_LANE_BASE - 1), PITCH_LANE_COUNT - 1);
        assert_eq!(pitch_lane(u8::MAX), 0);
        assert_eq!(
            pitch_lane(PITCH_LANE_BASE + u8::try_from(PITCH_LANE_COUNT).unwrap()),
            0
        );
        // 落在窗口内的音高：车道与 y 逐音符与投影一致。
        let view = ViewState::from_project(&filled_project()).expect("投影");
        for note in &view.notes {
            assert_eq!(note.row, pitch_lane(note.pitch));
            assert_eq!(note.y, pitch_lane_y(note.pitch));
        }
    }

    /// 判据 12: **不同 PPQ 缩放下位置单调且往返一致**（任务书对①的直接要求）。
    ///
    /// 两个方向：
    /// - 缩放变大（`tpp` 增）⇒ 同一个音符的 `x` 不增（单调）；
    /// - 像素 → tick 的**区间包含**关系成立：`px_to_tick(x) ≤ start_tick < px_to_tick(x+1)`。
    ///   浮点位置实现会在第二条上变红（`tick_to_px(30, 30)` 会算成 0）。
    #[test]
    fn note_positions_are_monotone_and_round_trip_across_zoom_levels() {
        let project = filled_project();
        let zooms = [1_u64, 3, 7, 30, 32, 120, 960];
        for (index, note) in ViewState::from_project(&project)
            .expect("投影")
            .notes
            .iter()
            .enumerate()
        {
            let mut previous: Option<(u64, f32)> = None;
            for tpp in zooms {
                let view = ViewState::from_project_with_zoom(&project, tpp).expect("缩放投影");
                let current = &view.notes[index];
                assert_eq!(
                    current.start_tick, note.start_tick,
                    "音符顺序必须与缩放无关"
                );
                let px = tick_to_px(current.start_tick, tpp).expect("tick → 像素");
                assert_eq!(current.x, as_px(px));
                // 像素 → tick 的区间往回包住起始 tick（整数除法的定义）。
                assert!(
                    px_to_tick(px, tpp).expect("像素 → tick") <= current.start_tick,
                    "tpp={tpp}: px_to_tick(x) 必须 ≤ start_tick"
                );
                assert!(
                    current.start_tick < px_to_tick(px + 1, tpp).expect("像素 → tick"),
                    "tpp={tpp}: start_tick 必须落在像素 {px} 的 tick 区间内（浮点位置会破坏这一条）"
                );
                if let Some((previous_tpp, previous_x)) = previous {
                    assert!(
                        previous_tpp < tpp && current.x <= previous_x,
                        "缩放 {previous_tpp} → {tpp} 时 x 必须单调不增（{previous_x} → {}）",
                        current.x
                    );
                }
                previous = Some((tpp, current.x));
            }
        }
    }

    /// 判据 13: **越界 tick / 音高不 panic**：越界 tick 返回 `Err`，越界音高钳制。
    #[test]
    fn absurd_note_ticks_error_and_out_of_range_pitches_do_not_panic() {
        use std::collections::BTreeMap;

        let mut project = default_project();
        let track_id = demo_id("T9");
        let mut track = TrackV3 {
            id: track_id,
            name: "NoteOverflow".to_owned(),
            ..TrackV3::default()
        };
        let clip_id = demo_id("K9");
        let note_id = EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8P1Z9").expect("ULID");
        let mut notes: BTreeMap<EntityId, MidiNote> = BTreeMap::new();
        notes.insert(
            note_id,
            MidiNote::new(note_id, u64::MAX - 1, u8::MAX, u64::MAX),
        );
        project.clip_pool.insert(
            clip_id,
            ClipPoolEntry {
                id: clip_id,
                name: "clip".to_owned(),
                content: ClipContent::Midi { notes },
            },
        );
        let placement_id = demo_id("Z9");
        track.clips.insert(
            placement_id,
            ClipPlacement {
                id: placement_id,
                clip_id,
                start_tick: 0,
                duration_ticks: 960,
                ..ClipPlacement::default()
            },
        );
        project.tracks.insert(track_id, track);
        assert_eq!(
            ViewState::from_project(&project),
            Err(BridgeError::TickOverflow {
                start_tick: u64::MAX - 1
            }),
            "越界音符 tick 必须返回错误, 不得 wrap / panic"
        );

        // 越界音高（0 / 255）与 0 时值都不 panic：车道钳制、宽有地板。
        let mut project = default_project();
        let clip_id = demo_id("K8");
        let mut notes: BTreeMap<EntityId, MidiNote> = BTreeMap::new();
        for (index, pitch) in [0_u8, 255].into_iter().enumerate() {
            let id =
                EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P1{index}Y")).expect("ULID");
            notes.insert(id, MidiNote::new(id, 0, pitch, 0));
        }
        project.clip_pool.insert(
            clip_id,
            ClipPoolEntry {
                id: clip_id,
                name: "edge".to_owned(),
                content: ClipContent::Midi { notes },
            },
        );
        let new_track_id = demo_id("T8");
        let mut new_track = TrackV3 {
            id: new_track_id,
            name: "Edge".to_owned(),
            ..TrackV3::default()
        };
        let placement_id = demo_id("Z8");
        new_track.clips.insert(
            placement_id,
            ClipPlacement {
                id: placement_id,
                clip_id,
                start_tick: 0,
                duration_ticks: 960,
                ..ClipPlacement::default()
            },
        );
        project.tracks.insert(new_track_id, new_track);
        let view = ViewState::from_project(&project).expect("越界音高不是错误");
        assert_eq!(view.notes.len(), 2);
        for note in &view.notes {
            assert_eq!(note.width, MIN_BLOCK_WIDTH_PX, "0 时值必须落到像素地板");
            assert!((0..PITCH_LANE_COUNT).contains(&note.row));
        }
    }

    /// 判据 14: 平行数组（身份 / 力度 / x / y / 宽 / 车道）与富投影**逐项一致**。
    ///
    /// 它们都由 [`ViewState::notes`] 派生；这条判据把"派生"钉死，防止有人单独改一处。
    #[test]
    fn note_parallel_arrays_agree_with_the_rich_projection() {
        for project in [demo_project(), filled_project()] {
            let view = ViewState::from_project(&project).expect("投影");
            assert_eq!(view.note_ulids.len(), view.notes.len());
            assert_eq!(view.note_velocities.len(), view.notes.len());
            assert_eq!(view.note_positions().len(), view.notes.len());
            assert_eq!(view.note_widths().len(), view.notes.len());
            assert_eq!(view.note_ys().len(), view.notes.len());
            assert_eq!(view.note_rows().len(), view.notes.len());
            for (index, note) in view.notes.iter().enumerate() {
                assert_eq!(note.index, index);
                assert_eq!(view.note_ulids[index], note.id);
                assert_eq!(view.note_velocities[index], note.velocity_normalized);
                assert_eq!(view.note_positions()[index], note.x);
                assert_eq!(view.note_widths()[index], note.width);
                assert_eq!(view.note_ys()[index], note.y);
                assert_eq!(view.note_rows()[index], note.row);
                assert!(crate::scene::is_ulid_text(&note.id));
            }
        }
    }

    /// 判据 15: 色标按 `#RGB` / `#RRGGBB` 解析；**非法 / 缺失一律回退**到文档常量。
    #[test]
    fn track_colors_parse_and_missing_or_illegal_colors_fall_back() {
        assert_eq!(
            parse_hex_color("#f7e6b0"),
            Some(RgbColor::new(0xf7, 0xe6, 0xb0))
        );
        assert_eq!(
            parse_hex_color("#F7E6B0"),
            Some(RgbColor::new(0xf7, 0xe6, 0xb0))
        );
        assert_eq!(
            parse_hex_color("22aa88"),
            Some(RgbColor::new(0x22, 0xaa, 0x88))
        );
        // CSS 短写：每一位复制成两位。
        assert_eq!(
            parse_hex_color("#abc"),
            Some(RgbColor::new(0xaa, 0xbb, 0xcc))
        );
        assert_eq!(
            parse_hex_color("#FFF"),
            Some(RgbColor::new(0xff, 0xff, 0xff))
        );
        // 非法形态一律拒绝（而不是猜一个颜色）。
        for illegal in [
            "",
            "#",
            "#12",
            "#12345",
            "#1234567",
            "#12345678",
            "#gggggg",
            "# f7e6b0",
            " #f7e6b0",
            "#f7e6b0 ",
            "rgb(1,2,3)",
            "红色",
            "#f7e6b-",
        ] {
            assert_eq!(parse_hex_color(illegal), None, "`{illegal}` 必须被拒绝");
            assert_eq!(
                track_color_or_default(Some(illegal)),
                DEFAULT_TRACK_COLOR,
                "`{illegal}` 必须回退到文档常量"
            );
        }
        assert_eq!(track_color_or_default(None), DEFAULT_TRACK_COLOR);
        // 文本与 u8 三元组是同一个颜色（两处表示不得漂移）。
        assert_eq!(DEFAULT_TRACK_COLOR.to_hex(), DEFAULT_TRACK_COLOR_HEX);
        assert_eq!(
            parse_hex_color(DEFAULT_TRACK_COLOR_HEX),
            Some(DEFAULT_TRACK_COLOR)
        );

        // 投影侧：合法色保留，缺失 / 非法回退。
        let mut project = demo_project();
        let ids: Vec<EntityId> = project
            .tracks
            .values()
            .map(|track| track.id)
            .filter(|id| *id != project.master_bus_track_id)
            .collect();
        if let Some(track) = project.tracks.get_mut(&ids[0]) {
            track.color = Some("#AbC".to_owned());
        }
        if let Some(track) = project.tracks.get_mut(&ids[1]) {
            track.color = Some("不是颜色".to_owned());
        }
        let view = ViewState::from_project(&project).expect("投影");
        assert_eq!(view.tracks[0].color_rgb, RgbColor::new(0xaa, 0xbb, 0xcc));
        assert_eq!(view.tracks[0].color_hex, "#AABBCC");
        assert_eq!(view.tracks[1].color_rgb, DEFAULT_TRACK_COLOR);
        assert_eq!(view.tracks[1].color_hex, DEFAULT_TRACK_COLOR_HEX);
        assert_eq!(view.track_colors().len(), view.tracks.len());
        assert_eq!(
            view.track_color_labels(),
            view.track_colors()
                .iter()
                .map(|color| color.to_hex())
                .collect::<Vec<_>>()
        );
        // 演示夹具里 `#22aa88` 与两个 `None` 分别给出保留 / 回退。
        let demo = ViewState::demo();
        assert_eq!(demo.tracks[0].color_hex, "#F7E6B0");
        assert_eq!(demo.tracks[1].color_hex, DEFAULT_TRACK_COLOR_HEX);
        assert_eq!(demo.tracks[2].color_hex, "#22AA88");
    }

    /// 判据 16: 色标与 `.slint` / `tokens.slint` 的**文本层耦合**（本机不编译 Slint）。
    ///
    /// 两条耦合各自都要有机械检查，否则"回退色"和"车道几何"会在无编译条件下静默漂移：
    /// - 回退色必须就是 `Tokens.line-strong` 的字面值；
    /// - 车道高度 / 车道数必须与 `piano_roll.slint` 画出的网格一致；
    /// - 音符位置必须由注入数组给出 —— `.slint` 里不得再出现"第 i 个音符"的索引布局。
    #[test]
    fn slint_text_contracts_for_colors_and_pitch_lanes() {
        let ui = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
        let tokens = std::fs::read_to_string(ui.join("tokens.slint")).expect("读 tokens.slint");
        let strong = tokens
            .lines()
            .find(|line| line.contains("line-strong:"))
            .expect("tokens.slint 必须有 line-strong");
        // 2026-10-06（主题特性，`--theme`）：`line-strong` 不再是一个光秃秃的字面量，而是
        //     out property <color> line-strong: ThemeState.theme == YebanTheme.brand
        //                                        ? #2c3a63 : (… yeban / inkmoor / plume / Palette.border);
        // （2026-10-07 起是四支自绘调色板 + `Palette` 的多选一，第一分支仍是品牌字面量。）
        // 这条对账要守的性质**没有变**，只是现在要指名表达式的**第一分支**：回退色必须等于
        // `ThemeState.theme == YebanTheme.brand` 那一支的字面量 —— 那是**品牌调色板**
        // (`--theme brand`) 的取值。2026-10-07 之前品牌色就是默认主题，所以当时这句读作
        // "默认主题就是 Linux golden 基线钉住的那一屏"; 现在默认主题是 yeban（`--theme
        // default`），这条耦合因此**只**再保证"回退色与品牌分支一致"。
        // 因此这里**要求**表达式是主题多选一（少了 `?` 就是主题特性被拿掉了，应该红），
        // 而不是退回到"整行必须只有一个字面量"的旧读法。
        let after_colon = strong.split(':').nth(1).expect("line-strong 有值");
        let (_condition, rest) = after_colon
            .split_once('?')
            .expect("line-strong 必须是主题多选一表达式（第一分支 = 品牌字面量）");
        let literal = rest
            .split(':')
            .next()
            .unwrap_or(rest)
            .trim()
            .trim_end_matches(';')
            .to_ascii_uppercase();
        assert_eq!(
            literal, DEFAULT_TRACK_COLOR_HEX,
            "回退色必须等于 Tokens.line-strong 的**默认分支**（两处各写一份，靠这条判据对账）"
        );

        let roll = std::fs::read_to_string(ui.join("console/piano_roll.slint"))
            .expect("读 piano_roll.slint");
        for required in [
            "for key_index in 16",
            "for lane_index in 16",
            "14px * lane_index",
            "root.note-positions[note_index]",
            "root.note-ys[note_index]",
            "root.note-widths[note_index]",
            "root.note-positions[velocity_index]",
        ] {
            assert!(
                roll.contains(required),
                "piano_roll.slint 必须包含 `{required}`（车道几何 / 音符位置来自投影）"
            );
        }
        // 负向断言只看**非注释行**：注释里可以（也应该）提到旧写法作为历史，
        // 但真正的代码不得再退回"第 i 个音符"的索引布局。
        let code: String = roll
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !code.contains("76px * note_index"),
            "音符位置不得再退回索引布局（`76px * note_index`）"
        );
        // `[UI-NOTE-003]` 拖动期的**草稿预览**给音符 x / y 各加了一个位移项（由宿主算好注入的
        // 逻辑像素；没有手势时恒为 0）。因此这里从"整行必须逐字相等"改成三条**合起来更强**的
        // 断言：① x / y **必须从投影数组起算**；② 那条赋值**唯一的**加项只能是草稿位移属性；
        // ③ 那条赋值里**不得**出现下标算术（`* note_index`）。
        // ⛔ 原本要挡的东西仍然是红的：`76px * note_index` 那一类索引布局过不了 ① 与 ③
        // （上面逐字那条负向断言也照旧在）。
        //
        // ⚠ `.slint` 的一条赋值可以**跨行**（本切片的草稿位移就写在续行上）⇒ 先把整条
        // 语句拼到分号为止再判，而不是只看一行。
        for (axis, projection, draft) in [
            (
                "x",
                "x: Tokens.space-5 + root.note-positions[note_index]",
                "root.note-drag-preview-dx",
            ),
            (
                "y",
                "y: root.note-ys[note_index]",
                "root.note-drag-preview-dy",
            ),
        ] {
            let start = code
                .lines()
                .position(|line| line.trim_start().starts_with(projection))
                .unwrap_or_else(|| panic!("音符 {axis} 必须**从投影数组起算**（`{projection}`）"));
            let mut statement = String::new();
            for line in code.lines().skip(start) {
                statement.push_str(line);
                statement.push('\n');
                if line.contains(';') {
                    break;
                }
            }
            assert!(
                statement.contains(draft),
                "音符 {axis} 上唯一的加项必须是拖动草稿位移 `{draft}`；实际那条赋值是 `{statement}`"
            );
            assert!(
                !statement.contains("* note_index"),
                "音符 {axis} 不得出现下标算术（`* note_index`）：`{statement}`"
            );
        }
        assert_eq!(PITCH_LANE_COUNT, 16, "车道数必须与 .slint 的 16 条一致");
        assert_eq!(
            PITCH_LANE_HEIGHT_PX, 14.0,
            "车道高必须与 .slint 的 14px 一致"
        );
        // `[UI-NOTE-003]` **力度车道几何**：宿主侧五个常量（本切片给它们命名）必须与 `.slint`
        // 逐值一致 —— 力度臂用它们把指针 y 反解成**绝对**力度，两处一旦分叉，
        // "拖到哪里 = 多少力度"就与画出来的柱各说各话（与帧原点同一类缺陷）。
        assert_eq!(
            VELOCITY_LANE_HEIGHT_PX, 44.0,
            "力度车道高必须与 .slint 的 44px 一致"
        );
        assert_eq!(
            VELOCITY_BAR_MAX_HEIGHT_PX, 28.0,
            "满力度柱高必须与 .slint 的 28px 一致"
        );
        assert_eq!(
            VELOCITY_LANE_BOTTOM_MARGIN_PX, 4.0,
            "力度车道底沿留白必须与 .slint 的 4px 一致"
        );
        assert_eq!(
            VELOCITY_BAR_WIDTH_PX, 6.0,
            "力度柱宽必须与 .slint 的 6px 一致"
        );
        assert_eq!(
            VELOCITY_BAR_OFFSET_X_PX, 30.0,
            "力度柱左沿偏移必须与 .slint 的 30px 一致（此前两处都是手抄值）"
        );
        for literal in [
            "velocity-lane := Rectangle {",
            "y: parent.height - 44px;",
            "height: 44px;",
            "y: parent.height - 4px - 28px * velocity_reading;",
            "height: 28px * velocity_reading;",
            "56px + Tokens.space-5 + root.note-positions[velocity_index] + 30px",
            "width: 6px;",
            "out property <length> velocity-lane-top: velocity-lane.y;",
        ] {
            assert!(
                code.contains(literal),
                "`piano_roll.slint` 必须包含 `{literal}`（力度车道几何 / 顶沿上报是宿主反解\
                 绝对力度的唯一依据）"
            );
        }
        // 帧原点（`painted_to_host` / `host_to_painted`）：琴键列宽 + 音符横向内缩 + 表头高
        // 必须与 `.slint` 画出的几何逐值一致 —— 否则宿主命中测试又会与人看得见的矩形分叉
        // （本票的缺陷：纵向 28px 偏移超过音符高 12px ⇒ 点可见音符恒不中）。
        assert_eq!(
            ROLL_KEY_COLUMN_WIDTH_PX, 56.0,
            "琴键列宽必须与 .slint 的 `width: 56px` / 网格 `x: 56px` 一致"
        );
        assert!(
            code.contains("x: 56px;") && code.contains("width: parent.width - 56px;"),
            "琴键列 / 网格必须仍从 x = 56px 起（`x: 56px;` 且网格宽 `parent.width - 56px`）"
        );
        assert_eq!(
            ROLL_HEADER_HEIGHT_PX, 28.0,
            "表头高必须与 .slint 的 `y: 28px` 一致"
        );
        assert!(
            code.matches("y: 28px;").count() >= 2,
            "琴键列与网格画布都必须仍在表头之下（`y: 28px;` 至少两处）"
        );
        let space5 = tokens
            .lines()
            .find(|line| line.contains("space-5:"))
            .expect("tokens.slint 必须有 space-5");
        let space5_px: f32 = space5
            .split_once("space-5:")
            .expect("space-5 有值")
            .1
            .trim()
            .trim_end_matches(';')
            .trim_end_matches("px")
            .trim()
            .parse()
            .expect("space-5 必须是 px 字面量");
        assert_eq!(
            NOTE_INSET_X_PX, space5_px,
            "音符横向内缩必须等于 `Tokens.space-5`（.slint 的音符 x 从它起算）"
        );
        // 唯一条换算的正反两向必须互逆：判据注入的"绘制坐标"与宿主读到的是同一点。
        let (round_trip_x, round_trip_y) = host_to_painted(123.0, 456.0);
        assert_eq!(
            painted_to_host(round_trip_x, round_trip_y),
            (123.0, 456.0),
            "`painted_to_host` 必须是 `host_to_painted` 的反向"
        );
    }

    /// 判据：声相文本是**整数千分之一**派生的纯函数（居中 / 左右 / 越界夹紧）。
    ///
    /// 混音台通道条的声相文本 (`track-pans[i]`) 与判据都读这一个函数，因此
    /// "界面显示的声相"与"工程里的 `TrackV3::pan`"不可能各说各话。
    #[test]
    fn pan_display_is_centred_around_zero() {
        assert_eq!(pan_display(0), "C");
        assert_eq!(pan_display(-500), "L50");
        assert_eq!(pan_display(500), "R50");
        assert_eq!(pan_display(1000), "R100");
        assert_eq!(pan_display(-1000), "L100");
        // 越界：夹紧（不绕回、不 panic），并且**不出现负号**（符号进了 `L`/`R` 前缀）。
        assert_eq!(pan_display(4321), "R100");
        assert_eq!(pan_display(-4321), "L100");
        assert_eq!(pan_display(-5), "L0", "不足 1% 的偏移仍如实显示方向");
        assert_eq!(pan_display(5), "R0");
        for millis in [-1000, -999, -1, 0, 1, 999, 1000] {
            let text = pan_display(millis);
            assert!(
                text == "C" || text.starts_with('L') || text.starts_with('R'),
                "`{text}` 不符口径"
            );
            assert!(!text.contains('-'), "`{text}` 不许带负号");
        }
        // 投影出来的每一轨都有文本，且与 `pan_millis` 同源。
        let view = ViewState::from_project(&filled_project()).expect("投影");
        let pans = view.track_pans();
        assert_eq!(pans.len(), view.tracks.len());
        for (track, text) in view.tracks.iter().zip(&pans) {
            assert_eq!(*text, pan_display(track.pan_millis));
        }

        // ---- 推子位置：与**电平**无关，只由 `volume_db` 派生 ----
        assert_eq!(volume_fraction(FADER_MAX_DB), 1.0);
        assert_eq!(volume_fraction(FADER_MIN_DB), 0.0);
        assert_eq!(volume_fraction(1_000.0), 1.0, "越界向上夹紧");
        assert_eq!(volume_fraction(-1_000.0), 0.0, "越界向下夹紧");
        assert_eq!(volume_fraction(f32::NAN), 0.0, "NaN 不许进几何");
        assert_eq!(volume_fraction(f32::INFINITY), 0.0);
        let fractions = view.track_volume_fractions();
        assert_eq!(fractions.len(), view.tracks.len());
        for (track, fraction) in view.tracks.iter().zip(&fractions) {
            assert_eq!(*fraction, volume_fraction(track.volume_db));
            assert!((0.0..=1.0).contains(fraction), "推子位置必须归一化");
        }
    }

    // ------------------------------------------------------------------
    // ADR-0004 **S1**：每轨高度 + 全局高度乘子
    //
    // 判据分组（每条都在提交说明里对应一次注入）：
    // 1. 默认路径的几何**逐位**不变（byte-identical geometry）；
    // 2. 有效行高的夹紧边界（上下界）+ 整数口径（floor 除法，不是四舍五入）；
    // 3. 全局乘子与每轨基准的交互（先乘、再除、**最后**夹紧）；
    // 4. 每轨覆盖按**身份**而不是下标生效；
    // 5. 自动化带跟随每轨高度（Q3 的"同一份行几何"）；
    // 6. 视图态 ↔ 布局的转换契约（退化值 0 的处置、变更检测）。
    // ------------------------------------------------------------------

    /// 第 `index` 条**非主总线**轨道（投影下标序）的身份规范文本。
    fn track_id_at(project: &YebanProjectV1, index: usize) -> String {
        project
            .tracks
            .values()
            .filter(|track| track.id != project.master_bus_track_id)
            .nth(index)
            .expect("夹具必须有这么多非主总线轨道")
            .id
            .to_canonical_string()
    }

    /// 非主总线轨道数（`rows` / `track-*` 数组的长度）。
    fn arrangement_track_count(project: &YebanProjectV1) -> usize {
        project
            .tracks
            .values()
            .filter(|track| track.id != project.master_bus_track_id)
            .count()
    }

    /// S1 判据 ①（**默认渲染的保证**）：默认布局（无每轨覆盖、乘子 100）的几何必须
    /// 与 S0 **逐位**相同 —— 不是"看起来一样"，而是 f32 的位模式相同。
    ///
    /// 它替 `ADR-0004` 判据 14（5 张 Linux Tier-1 Golden）在本机做投影侧的等价断言：
    /// 本机 macOS **判不了** golden（`test_port_adapter.rs:174` 打印"未被判定"并 return），
    /// 因此"默认帧没变"只能靠投影侧的逐位断言 + 注入（把 `DEFAULT_TRACK_HEIGHT_PX`
    /// 改成 57 这条判据立刻变红）。
    #[test]
    fn default_layout_geometry_is_bit_for_bit_the_s0_geometry() {
        // 整数默认高与 S0 的 f32 行距必须是**同一个数**（否则默认帧必变）。
        assert_eq!(
            DEFAULT_TRACK_HEIGHT_PX as f32,
            crate::automation::TRACK_LANE_HEIGHT_PX,
            "S1 的整数默认行高必须逐位等于 S0 的 f32 行距"
        );
        for project in [
            demo_project(),
            filled_project(),
            default_project(),
            YebanProjectV1::default(),
        ] {
            // ⓪ 两个入口（有/无布局参数）必须给出**同一个** `ViewState`
            //（`PartialEq` 逐字段比较 ⇒ 所有 f32 几何逐位比较）。
            let view = ViewState::from_project(&project).expect("投影");
            let via_layout =
                ViewState::from_project_with_layout(&project, &TrackHeightLayout::default())
                    .expect("投影");
            assert_eq!(
                view, via_layout,
                "默认布局必须与默认路径是同一个视图（逐字段、逐位）"
            );

            let rows = track_rows(&project);
            assert_eq!(rows.len(), view.tracks.len());
            assert_eq!(view.track_ys().len(), view.tracks.len());
            assert_eq!(view.track_heights().len(), view.tracks.len());
            for (index, row) in rows.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let closed_form = crate::automation::TRACK_LANE_TOP_PX
                    + crate::automation::TRACK_LANE_HEIGHT_PX * index as f32;
                assert_eq!(
                    row.y.to_bits(),
                    closed_form.to_bits(),
                    "第 {index} 行的前缀和必须逐位等于 S0 的闭式 top + 56 × i"
                );
                assert_eq!(
                    row.stride.to_bits(),
                    crate::automation::TRACK_LANE_HEIGHT_PX.to_bits()
                );
                assert_eq!(row.height_px, DEFAULT_TRACK_HEIGHT_PX);
                assert_eq!(view.tracks[index].row_height_px, DEFAULT_TRACK_HEIGHT_PX);
                assert_eq!(view.track_ys()[index].to_bits(), row.y.to_bits());
                assert_eq!(
                    view.track_heights()[index].to_bits(),
                    row.drawn_height().to_bits()
                );
                // 旧 `.slint` 的 `height: 54px`（= 56 − 2）。
                assert_eq!(row.drawn_height().to_bits(), 54.0_f32.to_bits());
            }
            // 剪辑的行几何同源（旧 `.slint` 的 `+ 4px` / `height: 46px`）。
            for clip in &view.clips {
                let row = rows[clip.track_index];
                assert_eq!(clip.y.to_bits(), row.clip_y().to_bits());
                assert_eq!(clip.height.to_bits(), row.clip_height().to_bits());
                assert_eq!(row.clip_height().to_bits(), 46.0_f32.to_bits());
            }
            // 主总线在编排视图没有行 —— 整数行高必须显式为 0（不是编造一行）。
            if let Some(master) = &view.master {
                assert_eq!(master.y, 0.0);
                assert_eq!(master.height, 0.0);
                assert_eq!(master.row_height_px, 0);
            }
        }
    }

    /// S1 判据 ②：有效行高的**夹紧边界**（`MIN_TRACK_HEIGHT_PX` / `MAX_TRACK_HEIGHT_PX`）
    /// 与**整数**口径（整数除法 = floor，不是四舍五入），以及投影侧"加宽成 f32 之后
    /// 仍是精确整数"。
    ///
    /// 边界数字的权威：`ADR-0004` Q2 **点名了这两个常量但没给数字**（规范与 ADR 在此沉默），
    /// 因此取值是工程选择，理由写在两个常量的文档里（几何退化约束：剪辑矩形 `stride − 10`、
    /// 自动化带 `(stride − 4) / 泳道数`）。这条判据把"选择"钉成"事实"。
    #[test]
    fn effective_height_is_clamped_inside_the_named_bounds_and_stays_integer() {
        // 下界：0、1、MIN 本身与 MIN 之下全部落到 MIN。
        assert_eq!(effective_track_height_px(0, 100), MIN_TRACK_HEIGHT_PX);
        assert_eq!(effective_track_height_px(1, 100), MIN_TRACK_HEIGHT_PX);
        assert_eq!(
            effective_track_height_px(MIN_TRACK_HEIGHT_PX - 1, 100),
            MIN_TRACK_HEIGHT_PX
        );
        assert_eq!(
            effective_track_height_px(MIN_TRACK_HEIGHT_PX, 100),
            MIN_TRACK_HEIGHT_PX
        );
        // 默认点：56 × 100% = 56（S1 的"数字不变"承诺）。
        assert_eq!(
            effective_track_height_px(DEFAULT_TRACK_HEIGHT_PX, DEFAULT_TRACK_HEIGHT_PERCENT),
            DEFAULT_TRACK_HEIGHT_PX
        );
        // 上界：MAX 本身、MAX 之上、以及放大到荒谬的乘子全部落到 MAX。
        assert_eq!(
            effective_track_height_px(MAX_TRACK_HEIGHT_PX, 100),
            MAX_TRACK_HEIGHT_PX
        );
        assert_eq!(
            effective_track_height_px(MAX_TRACK_HEIGHT_PX + 1, 100),
            MAX_TRACK_HEIGHT_PX
        );
        assert_eq!(
            effective_track_height_px(DEFAULT_TRACK_HEIGHT_PX, 1000),
            MAX_TRACK_HEIGHT_PX
        );
        assert_eq!(
            effective_track_height_px(u32::MAX, u32::MAX),
            MAX_TRACK_HEIGHT_PX,
            "饱和乘法 + 最后夹紧：荒谬输入不 panic、不回绕"
        );
        // 乘子为 0（窗口属性上的负数折算而来）⇒ 下界，而不是"这一行消失"。
        assert_eq!(
            effective_track_height_px(DEFAULT_TRACK_HEIGHT_PX, 0),
            MIN_TRACK_HEIGHT_PX
        );
        // **整数除法（floor）**，不是四舍五入：56 × 35 = 1960 → 1960 / 100 = **19**
        //（19.6 四舍五入是 20 ⇒ 用 `round` 的实现会在这里变红）。
        assert_eq!(effective_track_height_px(56, 35), 19);
        assert_eq!(effective_track_height_px(56, 33), 18);
        assert_eq!(effective_track_height_px(56, 150), 84);
        assert_eq!(effective_track_height_px(120, 25), 30);
        // 14 = 56 × 25% 会**低于**下界 ⇒ 被夹到 16（下界管着"缩到最小"）。
        assert_eq!(effective_track_height_px(56, 25), MIN_TRACK_HEIGHT_PX);

        // 全量扫描：结果恒在 [MIN, MAX]、对两个输入都单调（非严格）。
        let bases = [
            0_u32,
            1,
            15,
            16,
            17,
            55,
            56,
            57,
            100,
            319,
            320,
            321,
            10_000,
            u32::MAX,
        ];
        let percents = [0_u32, 1, 33, 50, 99, 100, 101, 150, 200, 1000, u32::MAX];
        for base in bases {
            let mut previous = 0_u32;
            for (index, percent) in percents.iter().enumerate() {
                let px = effective_track_height_px(base, *percent);
                assert!(
                    (MIN_TRACK_HEIGHT_PX..=MAX_TRACK_HEIGHT_PX).contains(&px),
                    "base={base} percent={percent} ⇒ {px} 掉出了命名边界"
                );
                if index > 0 {
                    assert!(px >= previous, "乘子变大 ⇒ 有效高不得变小");
                }
                previous = px;
            }
        }
        for percent in percents {
            let mut previous = 0_u32;
            for (index, base) in bases.iter().enumerate() {
                let px = effective_track_height_px(*base, percent);
                if index > 0 {
                    assert!(px >= previous, "基准变大 ⇒ 有效高不得变小");
                }
                previous = px;
            }
        }

        // 投影侧：每一个 `stride` 都是**精确整数**（f32 加宽无损），而且与整数真相同源。
        let project = demo_project();
        let id0 = track_id_at(&project, 0);
        let id1 = track_id_at(&project, 1);
        let mut tall = TrackHeightLayout::default();
        assert!(tall.set_track_px(&id0, MAX_TRACK_HEIGHT_PX + 5));
        assert!(tall.set_track_px(&id1, 3));
        let mut tiny = TrackHeightLayout::default();
        assert!(tiny.set_percent(1));
        for layout in [
            TrackHeightLayout::default(),
            tall,
            tiny,
            TrackHeightLayout::from_view_state(&[], &[], -1),
        ] {
            let rows = track_rows_with_layout(&project, &layout);
            let view = ViewState::from_project_with_layout(&project, &layout).expect("投影");
            for (index, row) in rows.iter().enumerate() {
                assert_eq!(row.stride, row.height_px as f32, "stride 必须是精确整数");
                assert_eq!(row.stride.fract(), 0.0, "行高不许有小数部分");
                assert!((MIN_TRACK_HEIGHT_PX..=MAX_TRACK_HEIGHT_PX).contains(&row.height_px));
                assert_eq!(view.tracks[index].row_height_px, row.height_px);
                assert_eq!(
                    view.tracks[index].height,
                    row.height_px as f32 - TRACK_ROW_GAP_PX
                );
                assert_eq!(view.track_ys()[index], row.y);
            }
        }
    }

    /// S1 判据 ③：**全局乘子与每轨基准的交互** —— 公式只有一处、顺序写死为
    /// "先乘（每轨基准 × 乘子）、再整除、**最后**夹紧"。
    ///
    /// 后果两条（都可注入打红）：① 每轨差异在**任何**乘子下都保持（同一乘子下基准大的行不矮）；
    /// ② 夹紧发生在乘子**之后** —— 1000% 的 56 变成 320（MAX）而不是"56 先是 56、乘子被丢掉"。
    #[test]
    fn percent_multiplies_each_per_track_height_and_the_clamp_comes_last() {
        // 先乘再除：50% / 200% 逐位可算。
        assert_eq!(effective_track_height_px(56, 200), 112);
        assert_eq!(effective_track_height_px(120, 50), 60);
        assert_eq!(effective_track_height_px(56, 100), 56);
        // 夹紧在**最后**：这条把"先夹基准再乘"（一个容易写出的顺序错误）分辨出来。
        // 基准 2×MAX（视图态允许存任意正整数，见 `set_track_px` 的文档）：
        //   先乘后夹（正确）= clamp(640 × 25 / 100) = clamp(160) = 160；
        //   先夹后乘（错误）= clamp(320) × 25 / 100 = 80。
        assert_eq!(
            effective_track_height_px(MAX_TRACK_HEIGHT_PX * 2, 25),
            160,
            "夹紧必须发生在乘子**之后**（先夹基准会得到 80）"
        );
        assert_eq!(
            effective_track_height_px(MAX_TRACK_HEIGHT_PX, 200),
            MAX_TRACK_HEIGHT_PX
        );
        assert_eq!(
            effective_track_height_px(DEFAULT_TRACK_HEIGHT_PX, 1000),
            MAX_TRACK_HEIGHT_PX,
            "乘子把默认行放大到荒谬时，夹紧给出 MAX（而不是回绕/饱和出一个坏几何）"
        );
        for percent in [50_u32, 75, 100, 125, 150, 200, 1000] {
            assert!(
                effective_track_height_px(120, percent) >= effective_track_height_px(56, percent),
                "同一乘子下，基准更高的行不得更矮（每轨差异必须活下来）"
            );
        }
        // 单调（乘子）在**夹紧区内**也必须成立：56 的 100% → 150% → 200%。
        assert!(effective_track_height_px(56, 100) < effective_track_height_px(56, 150));
        assert!(effective_track_height_px(56, 150) < effective_track_height_px(56, 200));

        // 端到端：两轨不同基准 + 150% 乘子 ⇒ 行高、前缀和、剪辑几何全部跟着走。
        let project = demo_project();
        let id0 = track_id_at(&project, 0);
        let id1 = track_id_at(&project, 1);
        let mut layout = TrackHeightLayout::default();
        assert!(layout.set_track_px(&id0, 100));
        assert!(layout.set_track_px(&id1, 40));
        assert!(layout.set_percent(150));
        let rows = track_rows_with_layout(&project, &layout);
        assert_eq!(rows[0].height_px, 150, "100 × 150%");
        assert_eq!(rows[1].height_px, 60, "40 × 150%");
        for row in &rows[2..] {
            // 没有每轨覆盖的行用**默认基准** 56，乘子照样作用在它上面。
            assert_eq!(row.height_px, 84, "56 × 150%");
        }
        assert_eq!(rows[1].y, rows[0].y + 150.0, "前缀和必须用逐行行高");
        assert_eq!(rows[2].y, rows[1].y + 60.0);
        let view = ViewState::from_project_with_layout(&project, &layout).expect("投影");
        for clip in &view.clips {
            let row = rows[clip.track_index];
            assert_eq!(clip.y, row.clip_y());
            assert_eq!(clip.height, row.clip_height());
        }
        // 乘子把**整份**几何放大：默认视图与 150% 视图的行高不相等。
        let default_view = ViewState::from_project(&project).expect("投影");
        assert_ne!(view.track_heights(), default_view.track_heights());
    }

    /// S1 判据 ④：每轨高度按**身份**（26 字符 `EntityId` 文本）生效，**不是**按下标。
    ///
    /// 这条的代价是真实的：按下标存高度，插入/删除一条轨道会把所有后续轨道的高度
    /// **静默错配**到别的轨道上（而 `[UI-TEST-001]` 的语义寻址正是按身份）。
    #[test]
    fn per_track_height_is_keyed_by_identity_not_by_index() {
        let project = demo_project();
        let count = arrangement_track_count(&project);
        assert!(count >= 3, "这条判据需要至少 3 条非主总线轨道");
        let first = track_id_at(&project, 0);
        let last = track_id_at(&project, count - 1);

        // (a) 视图态数组的**顺序不决定行**：反序输入的覆盖仍落到正确的身份上。
        let layout = TrackHeightLayout::from_view_state(
            &[last.clone(), first.clone()],
            &[200.0, 100.0],
            100,
        );
        assert_eq!(layout.effective_px(&first), 100);
        assert_eq!(layout.effective_px(&last), 200);
        let rows = track_rows_with_layout(&project, &layout);
        assert_eq!(rows[0].height_px, 100, "身份 `first` 在第 0 行");
        assert_eq!(
            rows[count - 1].height_px,
            200,
            "身份 `last` 在最后一行（而不是数组里的第 0 位）"
        );
        for (index, row) in rows.iter().enumerate().take(count - 1).skip(1) {
            assert_eq!(
                row.height_px, DEFAULT_TRACK_HEIGHT_PX,
                "第 {index} 行没有被覆盖 ⇒ 默认高"
            );
        }
        // (b) 不存在的身份**不匹配任何行**（而不是匹配到第 0 行）。
        assert_eq!(
            layout.effective_px("01ARZ3NDEKTSV4RRFFQ69G5FAV"),
            DEFAULT_TRACK_HEIGHT_PX
        );
        // (c) 删掉键序最前的**非主总线**轨道 ⇒ 所有下标前移一位，
        //     但"同一条身份的行高"一位不变（下标寻址会在这里错配）。
        let mut shrunk = project.clone();
        shrunk
            .tracks
            .remove(&EntityId::from_str(&first).expect("夹具身份必须可解析"));
        assert_eq!(arrangement_track_count(&shrunk), count - 1);
        let shrunk_rows = track_rows_with_layout(&shrunk, &layout);
        assert_eq!(shrunk_rows.len(), count - 1);
        for (index, row) in shrunk_rows.iter().enumerate() {
            let id = track_id_at(&shrunk, index);
            assert_eq!(
                row.height_px,
                layout.effective_px(&id),
                "第 {index} 行（身份 {id}）的行高必须跟着身份走"
            );
        }
    }

    /// S1 判据 ⑤（`ADR-0004` Q3 的纵向那一半）：自动化带与剪辑车道读**同一份**行几何 ——
    /// 变高之后每条带仍落在它所**属行**的 `[row.y, row.y + row.stride]` 之内，且真的变高了。
    #[test]
    fn automation_bands_follow_the_per_track_row_heights() {
        let project = demo_project();
        let id0 = track_id_at(&project, 0);
        let mut layout = TrackHeightLayout::default();
        assert!(layout.set_track_px(&id0, 200));
        let rows = track_rows_with_layout(&project, &layout);
        let view = ViewState::from_project_with_layout(&project, &layout).expect("投影");
        let default_view = ViewState::from_project(&project).expect("投影");
        assert_eq!(rows[0].height_px, 200);
        assert!(!view.automation_lanes.is_empty(), "演示工程必须有泳道");
        assert_eq!(
            view.automation_lanes.len(),
            default_view.automation_lanes.len(),
            "变行高**不**增删泳道"
        );
        let mut seen_taller_band = false;
        for (lane, default_lane) in view
            .automation_lanes
            .iter()
            .zip(default_view.automation_lanes.iter())
        {
            let row = rows[lane.track_index];
            assert!(
                lane.band_y >= row.y,
                "泳道 `{}` 的带顶掉出了所属行",
                lane.element_id
            );
            assert!(
                lane.band_y + lane.band_height <= row.y + row.stride,
                "泳道 `{}` 的带底掉出了所属行",
                lane.element_id
            );
            if lane.track_index == 0 {
                assert!(
                    lane.band_height > default_lane.band_height,
                    "第 0 条轨道变高之后它的带必须跟着变高（否则带与行是两份几何）"
                );
                seen_taller_band = true;
            }
        }
        assert!(seen_taller_band, "演示工程第 0 条轨道必须有自动化带");
    }

    /// S1 判据 ⑥：**视图态 ↔ 布局**的转换契约 —— 平行数组按身份配对、长度不等按短的截断、
    /// 坏值（空身份 / `NaN` / 非正）不算覆盖、退化值 0 与"变更检测"的返回值语义。
    #[test]
    fn view_state_layout_conversion_rejects_degenerate_values() {
        let id = "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_owned();
        // ⓪ 读写往返：身份 → 整数，长度不等按短的截断。
        let layout = TrackHeightLayout::from_view_state(
            &[
                id.clone(),
                String::new(),
                "other".to_owned(),
                "truncated".to_owned(),
            ],
            &[120.0, 300.0, f32::NAN],
            150,
        );
        assert_eq!(layout.percent(), 150);
        assert_eq!(
            layout.overrides().len(),
            1,
            "空身份 / NaN / 长度不等时被截断的尾部都不算覆盖"
        );
        assert_eq!(layout.base_px(&id), 120);
        assert_eq!(
            layout.effective_px(&id),
            effective_track_height_px(120, 150)
        );
        assert_eq!(layout.overrides()[&id], 120);

        // ① `set_track_px`：0 是退化值（**不是**高度），按"删除覆盖"处置。
        let mut layout = TrackHeightLayout::default();
        assert!(!layout.set_track_px(&id, 0), "0 不是布局选择 ⇒ 布局没变");
        assert_eq!(layout.base_px(&id), DEFAULT_TRACK_HEIGHT_PX);
        assert!(layout.set_track_px(&id, 120));
        assert!(!layout.set_track_px(&id, 120), "同一个值 ⇒ 没变");
        assert!(!layout.is_default());
        assert_eq!(layout.effective_px(&id), 120);
        assert!(!layout.set_track_px("", 120), "空身份不是键");
        assert!(!layout.remove_track_px("nobody"));
        assert!(layout.set_track_px(&id, 0), "删掉已有覆盖 ⇒ 变了");
        assert!(layout.is_default());

        // ② 边界**不**在 setter 里（只有投影夹紧）：设一个大得荒谬的基准也照存，
        //    有效值由投影夹到 MAX —— 这一条把"两处边界"挡在门外。
        assert!(layout.set_track_px(&id, 100_000));
        assert_eq!(layout.base_px(&id), 100_000);
        assert_eq!(layout.effective_px(&id), MAX_TRACK_HEIGHT_PX);

        // ③ `percent`：窗口属性是 `int` ⇒ 负数折算成 0 ⇒ 投影夹到下界（全函数，不 panic）。
        let zero = TrackHeightLayout::from_view_state(&[], &[], -5);
        assert_eq!(zero.percent(), 0);
        assert_eq!(zero.effective_px(&id), MIN_TRACK_HEIGHT_PX);
        assert_eq!(
            TrackHeightLayout::from_view_state(&[], &[], i32::MIN).percent(),
            0
        );
        // ④ 变更检测：`set_percent` 的返回值就是"要不要重投影"。
        let mut layout = TrackHeightLayout::default();
        assert!(layout.is_default());
        assert!(!layout.set_percent(DEFAULT_TRACK_HEIGHT_PERCENT));
        assert!(layout.set_percent(200));
        assert!(!layout.set_percent(200));
        assert!(!layout.is_default());
    }
}
