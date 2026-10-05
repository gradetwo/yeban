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

use std::collections::BTreeSet;
use std::fmt;

use yeban_model::ids::EntityId;
use yeban_model::music::MidiNote;
use yeban_model::project::{
    ClipContent, ClipPlacement, ClipPoolEntry, SceneV3, SectionV3, TimeSignature, TrackKind,
    TrackV3, YebanProjectV1,
};

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
/// 取值就是 `ui/tokens.slint` 的 `Tokens.line-strong`（`#2c3a63`）—— 一个中性石板色：
/// 它在深色面板上可见、又不会被误认成"某个轨道品牌色"。代价是 `.slint` 与 Rust 各写一份
/// 十六进制值，由判据 `missing_or_illegal_colors_fall_back_to_the_documented_value` 与
/// `token_drift_of_the_fallback_color_is_detected` 两侧对账（后者直接读 `tokens.slint` 原文）。
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
/// 所以一次取走一份，由本结构保证四个长度相等。
#[derive(Debug, Clone, PartialEq)]
pub struct VisibleNotes {
    /// 音符块左沿 x。
    pub positions: Vec<f32>,
    /// 音符块宽。
    pub widths: Vec<f32>,
    /// 音符块顶沿 y。
    pub ys: Vec<f32>,
    /// 音高车道索引。
    pub rows: Vec<i32>,
    /// `note-{ulid}-rect` 的 `{ulid}` 段（语义元素 ID 的一半）。
    pub ulids: Vec<String>,
    /// 归一化力度（0.0..=1.0）。
    pub velocities: Vec<f32>,
}

impl VisibleNotes {
    /// 可见音符数（四个数组长度相同，故用一个函数报告）。
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
    /// 车道序号（= [`ClipView::track_index`]，`.slint` 用它算 y）。
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
    /// 投影一个工程（默认缩放 [`DEFAULT_TICKS_PER_PIXEL`]）。
    ///
    /// # Errors
    ///
    /// 见 [`BridgeError`]。空工程**不是**错误 —— 它投影成空视图。
    pub fn from_project(project: &YebanProjectV1) -> Result<Self, BridgeError> {
        Self::from_project_with_zoom(project, DEFAULT_TICKS_PER_PIXEL)
    }

    /// 投影一个工程并指定缩放（`ticks_per_pixel`）。
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

    /// 投影一个工程、指定缩放，并指定自动化"当前值"的求值 tick。
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
        if ticks_per_pixel == 0 {
            return Err(BridgeError::ZeroTicksPerPixel);
        }

        let bar_ticks = bar_length_ticks(project.time_signature)?;

        let mut tracks: Vec<TrackView> = Vec::new();
        let mut master: Option<TrackView> = None;
        let mut clips: Vec<ClipView> = Vec::new();

        // `BTreeMap::values()` 是**身份升序**：跨进程、跨重启、跨机器都给出同一顺序
        // （红线 4 的确定性要求）。这里刻意不排序 —— 排序会掩盖"集合被换成 HashMap"。
        for track in project.tracks.values() {
            if track.id == project.master_bus_track_id {
                master = Some(track_view(track, 0, true));
                continue;
            }
            let index = tracks.len();
            for placement in track.clips.values() {
                clips.push(clip_view(
                    project,
                    placement,
                    index,
                    &track.name,
                    ticks_per_pixel,
                )?);
            }
            tracks.push(track_view(track, index, false));
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
            ppq: PPQ,
            ticks_per_pixel,
            bar_length_ticks: bar_ticks,
            tracks,
            master,
            clips,
            sections,
            scenes,
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
            automation_lanes: crate::automation::project_lanes_at_cursor(
                project,
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

    /// 剪辑车道序号。
    #[must_use]
    pub fn clip_lanes(&self) -> Vec<i32> {
        self.clips.iter().map(|clip| clip.lane).collect()
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
                "track index={} id={} name={} kind={} color={} color_rgb={} mute={} solo={} solo_safe={} volume_db={:.6} pan_millis={} pan={} clips={}",
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
fn track_view(track: &TrackV3, index: usize, is_master: bool) -> TrackView {
    #[allow(clippy::cast_possible_truncation)]
    let pan_millis = (f64::from(track.pan) * 1000.0).round() as i32;
    // 色标：**在这里**（唯一一处）解析 + 回退；界面与判据都只读解析结果。
    let color_rgb = track_color_or_default(track.color.as_deref());
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
        volume_db: track.volume_db,
        volume_display: format!("{:.1}", track.volume_db),
        volume_fraction: volume_fraction(track.volume_db),
        pan_millis,
        pan_display: pan_display(pan_millis),
        clip_count: track.clips.len(),
        is_master,
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
/// 用整数（千分之一）而不是 `f32`：声相文本会进 `accessible-value`，跨平台浮点格式化
/// 的 1 ulp 差异会让"同一工程两台机器给出不同文本"（`ARCH-DET-001` 禁止的正是这个）。
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
fn clip_view(
    project: &YebanProjectV1,
    placement: &ClipPlacement,
    track_index: usize,
    track_name: &str,
    ticks_per_pixel: u64,
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

/// 演示工程的实体身份：`01J8Z5Q0R7K3M9X2V4B6N8P0` + 2 字符尾段。
///
/// 前缀与 `scene.rs` 里的演示 ULID 常量同族（`…N8Pxx`），尾段用另一段命名空间
/// （前导 `0`）以免与音符 / 剪辑常量相撞。
///
/// # Panics
///
/// 尾段不是合法 Crockford Base32 时 panic（常量错误属编程错误）。
#[must_use]
fn demo_id(tail: &str) -> EntityId {
    use std::str::FromStr as _;
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P0{tail}"))
        .expect("演示夹具的 ULID 必须是合法 Crockford Base32")
}

/// **演示工程的夹具**：`src/scene.rs` 里原先那组硬编码常量的模型化形态。
///
/// 它与 `scene::TRACK_NAMES` / `NOTE_ULIDS` / `CLIP_ULIDS` / `SECTION_NAMES` /
/// `SCENE_NAMES` / `FADER_DB_LABELS` **逐字对应**，并有判据钉住
/// （`demo_projection_reproduces_the_scene_constants`）。这样"演示数据"就不再是
/// 界面里的常量，而是**一个真正的 `YebanProjectV1`** —— 换成
/// [`yeban_model::samples::filled_project`] 时走的是**同一条**代码路径。
#[must_use]
pub fn demo_project() -> YebanProjectV1 {
    use std::collections::BTreeMap;
    use std::str::FromStr as _;

    use yeban_model::music::MidiNote;
    use yeban_model::project::{ClipPoolEntry, LoopConfig, RoutingEdge, RoutingGraph, RoutingKind};

    let master_id = demo_id("M0");
    let track_ids: [EntityId; 6] = [
        demo_id("T1"),
        demo_id("T2"),
        demo_id("T3"),
        demo_id("T4"),
        demo_id("T5"),
        demo_id("T6"),
    ];
    // 与 `scene::TRACK_NAMES` 逐字一致；顺序 = 身份升序（`BTreeMap` 迭代序）。
    let track_names = ["鼓", "贝斯", "铺底", "主音", "弦乐", "打击"];
    // 与 `scene::FADER_DB_LABELS` 逐字一致（模型是权威，界面标签由投影生成）。
    let track_volumes = [-3.2_f32, -6.0, -8.4, -4.8, -12.0, -10.6];
    let track_kinds = [
        TrackKind::Midi,
        TrackKind::Audio,
        TrackKind::Midi,
        TrackKind::Midi,
        TrackKind::Midi,
        TrackKind::Audio,
    ];
    let track_colors = [
        Some("#f7e6b0"),
        None,
        Some("#22aa88"),
        None,
        None,
        Some("#3366ff"),
    ];

    let midi_clip_id = demo_id("K1");
    let audio_clip_id = demo_id("K2");

    // 六个音符的身份与 `scene::NOTE_ULIDS` 逐字一致。
    let note_ids: [EntityId; 6] = [
        "01J8Z5Q0R7K3M9X2V4B6N8P1A2",
        "01J8Z5Q0R7K3M9X2V4B6N8P1A3",
        "01J8Z5Q0R7K3M9X2V4B6N8P1B0",
        "01J8Z5Q0R7K3M9X2V4B6N8P1C7",
        "01J8Z5Q0R7K3M9X2V4B6N8P1D4",
        "01J8Z5Q0R7K3M9X2V4B6N8P1E1",
    ]
    .map(|text| EntityId::from_str(text).expect("演示音符 ULID 必须合法"));

    let mut notes: BTreeMap<EntityId, MidiNote> = BTreeMap::new();
    for (index, note_id) in note_ids.into_iter().enumerate() {
        let pitch = [60_u8, 64, 67, 72, 74, 76][index];
        let start = 480 * u64::try_from(index).unwrap_or(0);
        notes.insert(note_id, MidiNote::new(note_id, start, pitch, 480));
    }

    let mut tracks: BTreeMap<EntityId, TrackV3> = BTreeMap::new();
    tracks.insert(master_id, master_track(master_id));
    for (slot, track_id) in track_ids.into_iter().enumerate() {
        tracks.insert(
            track_id,
            TrackV3 {
                id: track_id,
                name: track_names[slot].to_owned(),
                kind: track_kinds[slot],
                volume_db: track_volumes[slot],
                pan: 0.0,
                mute: slot == 5,
                solo: slot == 0,
                solo_safe: false,
                folder_id: None,
                color: track_colors[slot].map(str::to_owned),
                devices: demo_track_devices(slot),
                macros: Vec::new(),
                automation_lanes: demo_automation_lanes(slot, track_id),
                clips: BTreeMap::new(),
            },
        );
    }

    // 三个剪辑摆放：身份与 `scene::CLIP_ULIDS` 逐字一致；一个 MIDI、两个音频。
    let placements: [(EntityId, EntityId, usize, u64, u64); 3] = [
        (
            EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8P1F9").expect("剪辑 ULID 必须合法"),
            midi_clip_id,
            0,
            0,
            3840,
        ),
        (
            EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8P1G6").expect("剪辑 ULID 必须合法"),
            audio_clip_id,
            1,
            1920,
            960,
        ),
        (
            EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8P1H3").expect("剪辑 ULID 必须合法"),
            audio_clip_id,
            2,
            5760,
            3840,
        ),
    ];
    for (placement_id, clip_id, slot, start_tick, duration_ticks) in placements {
        if let Some(track) = tracks.get_mut(&track_ids[slot]) {
            track.clips.insert(
                placement_id,
                ClipPlacement {
                    id: placement_id,
                    clip_id,
                    start_tick,
                    duration_ticks,
                    loop_config: LoopConfig::default(),
                    muted: false,
                },
            );
        }
    }

    // 四个段落 / 四个场景，与 `scene::SECTION_NAMES` / `scene::SCENE_NAMES` 逐字一致。
    let mut sections: BTreeMap<EntityId, SectionV3> = BTreeMap::new();
    for (slot, name) in ["Intro", "Verse", "Chorus", "Outro"]
        .into_iter()
        .enumerate()
    {
        let id = demo_id(["S1", "S2", "S3", "S4"][slot]);
        let start = 7680 * u64::try_from(slot).unwrap_or(0);
        sections.insert(
            id,
            SectionV3 {
                id,
                name: name.to_owned(),
                start_tick: start,
                end_tick: start + 7680,
                // `if` 而不是 `bool::then(..)`：后者会被 `clippy::unnecessary_lazy_evaluations`
                // 盯上（本仓库在 test_port_adapter.rs 里已踩过一次同类）。
                color: if slot == 0 {
                    Some("#22AA88".to_owned())
                } else {
                    None
                },
            },
        );
    }
    let mut scenes: BTreeMap<EntityId, SceneV3> = BTreeMap::new();
    for (slot, name) in ["Intro", "Verse", "Chorus", "Drop"].into_iter().enumerate() {
        let id = demo_id(["C1", "C2", "C3", "C4"][slot]);
        scenes.insert(
            id,
            SceneV3 {
                id,
                name: name.to_owned(),
                tempo: None,
                color: None,
            },
        );
    }

    // 路由：每条轨道 → 主总线（`RoutingGraph` 是声学连接的唯一真理源，红线见 MODEL-AST-004）。
    let mut edges: BTreeMap<EntityId, RoutingEdge> = BTreeMap::new();
    for (slot, track_id) in track_ids.into_iter().enumerate() {
        let edge_id = demo_id(["R1", "R2", "R3", "R4", "R5", "R6"][slot]);
        edges.insert(
            edge_id,
            RoutingEdge {
                id: edge_id,
                source_node: track_id,
                destination_node: master_id,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
    }
    let mut nodes = vec![master_id];
    nodes.extend(track_ids);

    let mut clip_pool: BTreeMap<EntityId, ClipPoolEntry> = BTreeMap::new();
    clip_pool.insert(
        midi_clip_id,
        ClipPoolEntry {
            id: midi_clip_id,
            name: "夜色铺底".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );
    clip_pool.insert(
        audio_clip_id,
        ClipPoolEntry {
            id: audio_clip_id,
            name: "909 鼓组".to_owned(),
            content: ClipContent::Audio {
                asset: yeban_model::ids::AssetHash::of_bytes(b"yeban-demo-kick"),
                gain_db: -1.5,
            },
        },
    );

    YebanProjectV1 {
        title: "夜半 Yeban".to_owned(),
        author: "Yeban Project Contributors".to_owned(),
        bpm: 120.0,
        id: demo_id("P0"),
        tracks,
        master_bus_track_id: master_id,
        routing_graph: RoutingGraph { nodes, edges },
        sections,
        scenes,
        clip_pool,
        ..YebanProjectV1::default()
    }
}

/// 演示夹具的**设备链**：只有轨道 0（`鼓`）挂一个内部合成器。
///
/// 它存在的理由是**一条泳道的值域**：`AutomationTarget::DeviceParam` 是模型里唯一
/// "固有取值域不可知"（`nominal_domain() == None`）的目标，因此只有它能让"按曲线最值
/// 自适应纵轴"这条路径被真正执行到（见 `crate::automation` 的模块文档与判据 ③）。
#[must_use]
fn demo_track_devices(slot: usize) -> Vec<yeban_model::project::DeviceDefinition> {
    if slot != 0 {
        return Vec::new();
    }
    vec![yeban_model::project::DeviceDefinition {
        id: demo_id("D1"),
        name: "Yeban PolySynth".to_owned(),
        kind: yeban_model::project::DeviceKind::InternalInstrument,
        bypassed: false,
        params: vec![yeban_model::project::ParameterValue {
            name: "cutoff".to_owned(),
            value: 1200.0,
            unit: Some("Hz".to_owned()),
        }],
        latency_samples: 0,
    }]
}

/// 演示夹具的一个自动化采样点（身份与值一起给出，避免键/身份漂移）。
fn demo_point(
    tail: &str,
    tick: u64,
    value: f32,
    curve: yeban_model::music::CurveType,
) -> (EntityId, yeban_model::project::AutomationPoint) {
    let id = demo_id(tail);
    (
        id,
        yeban_model::project::AutomationPoint {
            id,
            tick,
            value,
            curve,
        },
    )
}

/// 演示夹具的**自动化泳道**（`line/app-automation-ui` 补的那一格）。
///
/// 三条泳道刻意覆盖三种不同的目标形状，让"人工看一下"也能分辨它们：
///
/// | 轨道 | 目标 | 单位 | 取值域 | 读 / 写 | 覆盖的口径 |
/// | :--- | :--- | :--- | :--- | :--- | :--- |
/// | 0（`鼓`） | `TrackVolume` | `dB` | **固有** `[-60, 12]` | 读开 / `Touch` | 轴来自目标、录制臂角标 |
/// | 0（`鼓`） | `DeviceParam(0, 0)` | `Native` | **自适应** `[200, 4000]` | **读关** / `Off` | 值域自适应 + 读关闭可区分 + 同轨多泳道等分 |
/// | 1（`贝斯`） | `TrackPan` | `Bipolar` | 固有 `[-1, 1]` | 读开 / `Write` | 双极单位、另一条轨道 |
///
/// 它们**不是**界面常量：`YebanProjectV1` 是唯一事实源，界面只读投影
/// （`demo_projection_reproduces_the_scene_constants` 钉住这一点）。
#[must_use]
fn demo_automation_lanes(
    slot: usize,
    track_id: EntityId,
) -> std::collections::BTreeMap<
    yeban_model::project::AutomationTarget,
    yeban_model::project::AutomationLane,
> {
    use yeban_model::music::CurveType;
    use yeban_model::project::{
        AutomationLane, AutomationPoint, AutomationTarget, AutomationWriteMode,
    };

    let mut lanes = std::collections::BTreeMap::new();
    let mut insert = |target: AutomationTarget,
                      points: Vec<(EntityId, AutomationPoint)>,
                      read_enabled: bool,
                      write_mode: AutomationWriteMode| {
        lanes.insert(
            target,
            AutomationLane {
                target,
                points: points.into_iter().collect(),
                read_enabled,
                write_mode,
                domain: None,
            },
        );
    };
    match slot {
        0 => {
            insert(
                AutomationTarget::TrackVolume { track_id },
                vec![
                    demo_point("A1", 0, -3.2, CurveType::Linear),
                    demo_point("A2", 1920, -8.0, CurveType::Logarithmic),
                    demo_point("A3", 3840, -1.0, CurveType::Linear),
                ],
                true,
                AutomationWriteMode::Touch,
            );
            insert(
                AutomationTarget::DeviceParam {
                    track_id,
                    slot_index: 0,
                    param_index: 0,
                },
                vec![
                    demo_point("A4", 0, 200.0, CurveType::Exponential),
                    demo_point("A5", 960, 1200.0, CurveType::Linear),
                    demo_point("A6", 3840, 4000.0, CurveType::Linear),
                ],
                false,
                AutomationWriteMode::Off,
            );
        }
        1 => {
            insert(
                AutomationTarget::TrackPan { track_id },
                vec![
                    demo_point("A7", 0, -1.0, CurveType::Linear),
                    demo_point("A8", 960, 0.0, CurveType::SCurve),
                    demo_point("A9", 2880, 1.0, CurveType::Linear),
                ],
                true,
                AutomationWriteMode::Write,
            );
        }
        _ => {}
    }
    lanes
}

/// 最小主总线音轨夹具（`TrackKind::Master`）。
#[must_use]
fn master_track(id: EntityId) -> TrackV3 {
    TrackV3 {
        id,
        name: "Master".to_owned(),
        kind: TrackKind::Master,
        ..TrackV3::default()
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

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

    /// 判据 3: **空工程不 panic**，且投影成空视图（轨道 / 剪辑 / 段落 / 场景全空）。
    #[test]
    fn empty_project_projects_to_an_empty_view() {
        let view = ViewState::from_project(&default_project()).expect("空工程必须能投影");
        assert!(view.tracks.is_empty());
        assert!(view.master.is_none());
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
        assert_eq!(ViewState::empty().canonical_bytes(), view.canonical_bytes());
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
        // 判据: 四个平行数组**同长**, 且第 k 个可见音符的每个字段都等于源 `notes[idx[k]]` 的同名字段。
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
        let literal = strong
            .split(':')
            .nth(1)
            .expect("line-strong 有值")
            .trim()
            .trim_end_matches(';')
            .to_ascii_uppercase();
        assert_eq!(
            literal, DEFAULT_TRACK_COLOR_HEX,
            "回退色必须等于 Tokens.line-strong（两处各写一份，靠这条判据对账）"
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
        assert!(
            code.contains("x: Tokens.space-5 + root.note-positions[note_index];"),
            "音符 x 必须直接取投影数组"
        );
        assert_eq!(PITCH_LANE_COUNT, 16, "车道数必须与 .slint 的 16 条一致");
        assert_eq!(
            PITCH_LANE_HEIGHT_PX, 14.0,
            "车道高必须与 .slint 的 14px 一致"
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
}
