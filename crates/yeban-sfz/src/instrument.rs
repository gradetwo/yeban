//! 可播放的 SFZ 乐器模型：`<region>` 归约与确定性 region 选择。
//!
//! 规范来源 (Normative): `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` ROAD-M2-005 /
//! ROAD-M2-006；确定性要求见 `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` [ARCH-DET-001]
//! （同一输入必须得到同一结果，禁止随机数）。
//!
//! 语义移植来源 (learn-from, MIT): `groove/src/audio/sfz/**`（TypeScript, 2,979 行）——
//! 只借鉴 `keyswitch` / `include` / `define` / CC gate / 轮替的**语义**，Rust 重写。
//! 见 `docs/ledger/legacy-reuse-audit.md`：该目录**代码不复用**。
//!
//! 默认值全部取自 sfzformat 的机器可读 opcode 表 `_data/sfz/syntax.yml`
//! （`pitch_keycenter` 默认 60、`lokey` 0、`hikey` 127、`lovel` 0、`hivel` 127、
//! `lochan` 1、`hichan` 16、`seq_length` 1、`seq_position` 1、`tune` 0、`transpose` 0）。

use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::crossfade::{Crossfade, XfAxis, XfCurve, XfDirection, XfRange};
use crate::curve::{Curve, CurvePoint};
use crate::effect::{Effect, EffectBus};
use crate::error::SfzError;
use crate::midi::MidiSection;
use crate::parser::Warning;
use crate::parser::{OpcodeMap, OpcodeValue, parse_int};
use crate::velocity::{
    AMP_VELTRACK_DEFAULT, AMP_VELTRACK_MAX, AMP_VELTRACK_MIN, MAX_VELCURVE_AMPLITUDE,
    MAX_VELCURVE_INDEX, MIN_VELCURVE_AMPLITUDE, MIN_VELCURVE_INDEX, VelocityCurve, veltrack_gain,
};

/// 循环模式。取值与默认语义见 <https://sfzformat.com/opcodes/loop_mode/>。
///
/// 规范默认值是「文件无 loop 元数据时 `no_loop`，有 loop 时 `loop_continuous`」；
/// 本 crate 不读音频文件元数据（依赖预算：零重依赖），因此**缺省为 [`LoopMode::NoLoop`]**，
/// 该边界记在 `docs/ledger/sfz-core-notes.md`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopMode {
    /// `no_loop`：从头播到尾，或到 note-off。
    NoLoop,
    /// `one_shot`：完整播放，忽略 note-off（鼓组常用）。
    OneShot,
    /// `loop_continuous`：到 loop 点后持续循环，包括 release 阶段。
    LoopContinuous,
    /// `loop_sustain`：按住（或踩住延音踏板）时循环，release 阶段不循环。
    LoopSustain,
}

impl LoopMode {
    /// 白名单（`opcode=value` 的大小写不敏感匹配集合）。
    pub const OPTIONS: &'static [(&'static str, LoopMode)] = &[
        ("no_loop", LoopMode::NoLoop),
        ("one_shot", LoopMode::OneShot),
        ("loop_continuous", LoopMode::LoopContinuous),
        ("loop_sustain", LoopMode::LoopSustain),
    ];

    /// 用于错误信息的允许值列表。
    pub const ALLOWED: &'static str = "no_loop, one_shot, loop_continuous, loop_sustain";
}

/// 采样播放区间的终点（`end` opcode，取值**含**端点，单位采样点）。
///
/// 规范事实（<https://sfzformat.com/opcodes/end/>）：默认 `unspecified`（播到采样末尾）；
/// `end` **是含端点的**（「`end` is inclusive, so if set to 133000, the sample will play
/// all samples up to and including 133000」）；`end=-1` 时「the sample will not play」，
/// 但该 region **仍然被触发**，因此可以用 `group` / `off_by` 关掉别的 region。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleEnd {
    /// 未指定 `end`：播放到采样末尾（终点只能由解码器给出）。
    Unspecified,
    /// `end=-1`：不产生采样输出，但仍参与触发与 `group` / `off_by` 互斥。
    Silent,
    /// 显式终点（采样点，**含**该点）。
    Inclusive(u32),
}

/// 采样播放方向（`direction` opcode，SFZ v2，默认 `forward`）。
///
/// 出处：<https://sfzformat.com/opcodes/direction/>（`Options: forward, reverse`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayDirection {
    /// `forward`（默认）：从起点向终点播放。
    Forward,
    /// `reverse`：从终点向起点播放。
    Reverse,
}

impl PlayDirection {
    /// 白名单（`opcode=value` 的大小写不敏感匹配集合）。
    pub const OPTIONS: &'static [(&'static str, PlayDirection)] = &[
        ("forward", PlayDirection::Forward),
        ("reverse", PlayDirection::Reverse),
    ];

    /// 用于错误信息的允许值列表。
    pub const ALLOWED: &'static str = "forward, reverse";
}

/// region 收到的 MIDI 事件：`trigger` 门控的**事件口径**。
///
/// 本枚举只描述「发生了哪一类事件」，不含「由哪个 region 播放」的判断 ——
/// 后者是 [`Trigger::responds_to`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerEvent {
    /// 一个 note-on。
    NoteOn,
    /// 一个 note-off。
    ///
    /// `pedal_down` 报告延音踏板此刻是否踩下：规范里 `trigger=release` 的 region
    /// 「will play on note-off **or** sustain pedal off」，因此踏板踩下时它**不**发声
    /// （等踏板松开再报一次本事件）；`release_key` 则完全忽略踏板。
    /// 出处：<https://sfzformat.com/opcodes/trigger/>。
    NoteOff {
        /// 延音踏板此刻是否踩下。
        pedal_down: bool,
    },
}

/// `trigger` opcode：region 由哪一类 MIDI 事件触发。
///
/// 规范事实（<https://sfzformat.com/opcodes/trigger/>）：
/// `attack` 是缺省值；`first` / `legato` 也是 note-on 家族，但额外要求
/// 「触发时有没有**其它**音符按着」；`release` 由 note-off / 踏板松开触发；
/// `release_key`（SFZ v2）由 note-off 触发并**忽略**踏板。
///
/// 同一页还规定：`trigger=release` / `release_key` 的 region
/// 「will play as if `loop_mode` was set to `one_shot`」。该覆盖由
/// [`Region::effective_loop_mode`] 实现。
///
/// **本切片刻意不建模**（登记在 `docs/ledger/sfz-core-notes.md`）：
/// ARIA / DropZone 要求 release region 存在「对应的 attack region」才发声，
/// 并按 `rt_decay` 缩放音量；规范自己写明这一族行为
/// 「varies considerably between SFZ players」。本 crate 只做**事件门控**，
/// 不发明「对应 attack region」的判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// `attack`（缺省）：note-on 触发。
    Attack,
    /// `release`：note-off 触发；踏板踩下时推迟到踏板松开。
    Release,
    /// `release_key`（SFZ v2）：note-off 触发，忽略延音踏板。
    ReleaseKey,
    /// `first`：note-on 触发，且触发时没有其它音符按着。
    First,
    /// `legato`：note-on 触发，且触发时有其它音符按着。
    Legato,
}

impl Trigger {
    /// 白名单（`opcode=value` 的大小写不敏感匹配集合）。
    pub const OPTIONS: &'static [(&'static str, Trigger)] = &[
        ("attack", Trigger::Attack),
        ("release", Trigger::Release),
        ("release_key", Trigger::ReleaseKey),
        ("first", Trigger::First),
        ("legato", Trigger::Legato),
    ];

    /// 用于错误信息的允许值列表。
    pub const ALLOWED: &'static str = "attack, release, first, legato, release_key";

    /// 是否属于 release 家族（`release` / `release_key`）：由 note-off 触发，
    /// 且按规范强制 `loop_mode=one_shot`。
    #[must_use]
    pub fn is_release_family(self) -> bool {
        matches!(self, Trigger::Release | Trigger::ReleaseKey)
    }

    /// 该 region 是否响应这个事件。
    #[must_use]
    pub fn responds_to(self, event: TriggerEvent) -> bool {
        match self {
            Trigger::Attack | Trigger::First | Trigger::Legato => event == TriggerEvent::NoteOn,
            // 踏板踩下时 `release` 不发声，等踏板松开。
            Trigger::Release => event == TriggerEvent::NoteOff { pedal_down: false },
            // `release_key` 忽略踏板：踏板踩不踩都发声。
            Trigger::ReleaseKey => matches!(event, TriggerEvent::NoteOff { .. }),
        }
    }

    /// `first` / `legato` 的「其它按住的音符数」门控是否满足；其它取值恒 `true`。
    ///
    /// `held_notes` 是**其它**音符的当前按住数，由调用方给出。严格策略与
    /// [`RegionQuery`] 的 keyswitch / CC 门控一致：region 需要该状态而查询没有提供时
    /// 返回 `false` —— 不猜「大概没有别的音」，也不悄悄发声。
    #[must_use]
    pub fn held_notes_ok(self, held_notes: Option<u32>) -> bool {
        match self {
            Trigger::First => held_notes == Some(0),
            Trigger::Legato => held_notes.is_some_and(|held| held > 0),
            Trigger::Attack | Trigger::Release | Trigger::ReleaseKey => true,
        }
    }
}

/// `off_time` opcode 的规范缺省值，单位**秒**。
///
/// 出处：<https://sfzformat.com/opcodes/off_time/> 的表格（Type = float，Default = 0.006，
/// 无 Range）以及同页正文第一句 "When `off_mode` is set to `time`, this specifies the
/// fadeout time for regions being muted by voice-stealing"。
///
/// **它不是 [ARCH-RT-004] 的 3 ms 窃取淡出常量**：3 ms 是本 crate 在窃取路径上的工程常量
/// （见 `crate::voice_pool::StealFade`），本常量只是 [`Region::off_time`] 缺省时的回退值，
/// 由消费方决定何时使用。两者不互相改写。
pub const OFF_TIME_DEFAULT_SECONDS: f32 = 0.006;

/// `bend_up` opcode 的规范缺省值，单位**音分**。
///
/// 出处：<https://sfzformat.com/opcodes/bend_up/> 的表格行
/// （Type = integer，Default = 200，Range = -9600 to 9600，Unit = cents）。
/// 同页正文界定了语义："Pitch bend range when Bend Wheel or Joystick is moved up,
/// in cents"，并写明取值可为负："If `bend_up` is negative, then moving the pitch wheel
/// up will cause the pitch to move down."
pub const BEND_UP_DEFAULT_CENTS: i32 = 200;

/// `bend_down` opcode 的规范缺省值，单位**音分**。
///
/// 出处：<https://sfzformat.com/opcodes/bend_down/> 的表格行
/// （Type = integer，Default = -200，Range = -9600 to 9600，Unit = cents）。
/// 同页正文："Pitch bend range when Bend Wheel or Joystick is moved down, in cents"，
/// 并写明取正值的用途："Positive values of `bend_down` can be useful with instruments
/// such as zithers or guitars ... this way, moving the pitch wheel in either direction
/// will result in a realistic-sounding upwards bend." —— 因此本 crate **不**把
/// "`bend_down` 必须非正"当校验，登记语料里也确有 `bend_down=1200`（19 次）。
pub const BEND_DOWN_DEFAULT_CENTS: i32 = -200;

/// `bend_up` / `bend_down` opcode 的规范取值域端点，单位**音分**。
///
/// 两个格式页的表格 Range 都是 `-9600 to 9600`（＝ ±8 个八度）；越界是明确
/// [`SfzError::IntegerOutOfRange`]，**不**静默钳位（与 `tune` / `transpose` 同一条口径）。
pub const BEND_RANGE_MAX_CENTS: i32 = 9600;

/// 弯音轮处于中位（未弯音）的 MIDI 值。
///
/// 弯音轮是 14 位量（0..=16383），中位是 8192；本 crate 只接收已归约到
/// `0..=127` 的**单一**弯音量（见 [`Region::bend_cents`]），其中位是 [`PITCH_BEND_CENTER`]。
pub const PITCH_BEND_CENTER: u8 = 64;

/// `off_mode` opcode：region 被**关断**时声部如何结束。
///
/// 取值集合与缺省值取自登记语料的 opcode 普查（三个取值 `normal` / `fast` / `time`，
/// 出现次数为 818 / 87 / 4，合计 909），见
/// `docs/ledger/sfz-core-notes.md` 第 11 节；格式出处
/// <https://sfzformat.com/opcodes/off_mode/>。
///
/// **出处分工**：取值集合、出现次数与缺省值 `fast` 来自上面那份**登记语料普查**；
/// 三个取值各自的含义来自上面那个**格式页**。其中「`fast` ＝ 立刻结束」一条另有台账佐证：
/// 同节写明缺省值 `fast`「与现行为等价」，而现行为就是在关断时立刻结束声部。
///
/// 格式页正文第一句界定了这一族 opcode 的作用面：它决定「region 如何被 `off_by`
/// opcode 关断」，而不是 note-off 的包络释放：
///
/// - [`OffMode::Fast`]（缺省）：立刻关断声部；release 设置不起作用。
/// - [`OffMode::Normal`]：进入 release 阶段 —— 所有包络发生器进入 release，
///   声部在**放大器包络**耗尽时结束（需要包络，引擎侧）。
/// - [`OffMode::Time`]（ARIA 扩展）：用一段**与采样 release 无关**的时间关断声部，
///   时长由 [`Region::off_time`] 给出（缺省 `OFF_TIME_DEFAULT_SECONDS`）。
///   格式页写明该时长同样落在声部窃取的淡出路径上：
///   "this specifies the fadeout time for regions being muted by voice-stealing"
///   （<https://sfzformat.com/opcodes/off_time/>）。
///
/// **本 crate 只做类型化建模，不决定包络 / 淡化的实现**（那属于引擎侧）。
/// 因此 [`OffMode::Time`] 的时长不与 [ARCH-RT-004] 的 3 ms 窃取淡出冲突：
/// 后者是本 crate 在窃取路径上给出的工程常量，本 crate 不读取采样字节、也不实现淡化。
/// 消费方得到的是一条明确契约：只有 [`OffMode::Fast`] 允许立刻切断声部。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffMode {
    /// `fast`（缺省）：立刻结束声部。
    Fast,
    /// `normal`：进入 release 阶段，按放大器包络结束声部（需要包络，引擎侧）。
    Normal,
    /// `time`（ARIA 扩展）：在 [`Region::off_time`] 秒后结束声部。
    Time,
}

impl OffMode {
    /// 白名单（`opcode=value` 的大小写不敏感匹配集合）。
    pub const OPTIONS: &'static [(&'static str, OffMode)] = &[
        ("fast", OffMode::Fast),
        ("normal", OffMode::Normal),
        ("time", OffMode::Time),
    ];

    /// 用于错误信息的允许值列表。
    pub const ALLOWED: &'static str = "fast, normal, time";

    /// 是否允许**立刻**切断声部：只有 `fast`。
    ///
    /// 另外两个取值要求一段放大器包络 release（`normal`）或一段由 `off_time`
    /// 给定的淡化时长（`time`），因此调用方**不得**把它们当成立刻切断
    /// —— 这正是规范区分三者的目的。
    #[must_use]
    pub fn cuts_voice_immediately(self) -> bool {
        self == Self::Fast
    }
}

/// 一个 MIDI CC 门控：`loccN` / `hiccN` 归约成 `[lo, hi]` 闭区间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CcGate {
    /// CC 编号 (0..=127)。
    pub cc: u8,
    /// 下界（含）。
    pub lo: u8,
    /// 上界（含）。
    pub hi: u8,
}

/// 一个可播放的 `<region>`。
///
/// 字符串字段用 [`Cow`]：无 `$VAR` 宏替换时是 `Borrowed`（零拷贝借用源缓冲），
/// 只有发生文本替换时才拥有所有权。
///
/// 采样播放的数值换算（音高比 / 步进比 / 线性增益 / 循环窗口）见
/// [`crate::playback`]。
#[derive(Debug, Clone, PartialEq)]
pub struct Region<'a> {
    /// `sample` opcode 的原始取值（未拼接 `default_path`）。
    pub sample: Cow<'a, str>,
    /// `<control>default_path`：播放时拼在 `sample` 前面，见 [`Region::sample_path`]。
    pub default_path: Option<Cow<'a, str>>,
    /// 音域下界（MIDI 号）。
    pub lokey: i32,
    /// 音域上界（MIDI 号）。
    pub hikey: i32,
    /// 根音（`pitch_keycenter`，默认 60 = C4）。
    pub pitch_keycenter: i32,
    /// `false` 表示 `key=-1`：该 region 不由音符触发（例如纯 CC 触发）。
    pub trigger_by_note: bool,
    /// `trigger`：该 region 由哪一类 MIDI 事件触发（缺省 [`Trigger::Attack`]）。
    ///
    /// 与 [`Region::trigger_by_note`] 的分工：`trigger_by_note` 是 `key=-1`
    /// 的布尔投影（「是否由音符触发**这件事**」），本字段是 `trigger` opcode 的原样取值
    /// （「由 note-on 还是 note-off 触发」）。二者都成立才在对应事件上播放。
    pub trigger: Trigger,
    /// `off_mode`：note-off / `off_by` 关断到达时声部如何结束（缺省 [`OffMode::Fast`]）。
    ///
    /// 与 [`Region::effective_loop_mode`] 的分工：`one_shot` 说的是「**不理会** note-off」，
    /// `off_mode` 说的是「note-off 真的到达之后**怎么结束**」。
    pub off_mode: OffMode,
    /// `off_time`（ARIA，单位**秒**）：`off_mode=time` 关断声部时的淡化时长。
    ///
    /// `None` 表示源文件**没有**给出该 opcode；此时规范缺省是
    /// [`OFF_TIME_DEFAULT_SECONDS`]，见 [`Region::effective_off_time`]。刻意区分
    /// `None` 与 `Some(0.0)`：前者是「没说」，后者是「显式要求 0 秒」。
    ///
    /// 格式页（<https://sfzformat.com/opcodes/off_time/>）只把该时长挂在
    /// `off_mode=time` 上："When `off_mode` is set to `time`, this specifies the fadeout
    /// time for regions being muted by voice-stealing"。因此它**不**改写
    /// [ARCH-RT-004] 的 3 ms 窃取淡出常量，也**不**是 `off_mode=normal` 的包络 release。
    ///
    /// 取值域：规范表格的 Range 为空 ⇒ 只要求**有限**且**非负**
    /// （负时长的取反是明确 `Err`，不静默钳位）。
    pub off_time: Option<f32>,
    /// 力度下界（含）。
    pub lovel: u8,
    /// 力度上界（含）。
    pub hivel: u8,
    /// MIDI 通道下界（含，1..=16）。
    pub lochan: u8,
    /// MIDI 通道上界（含，1..=16）。
    pub hichan: u8,
    /// `amp_veltrack`：力度 → 振幅的跟踪量（%，规范范围 -100..=100，缺省 100）。
    ///
    /// 出处 <https://sfzformat.com/opcodes/amp_veltrack/> 的表格行
    /// （Type = float，Default = 100，Range = -100 to 100，Unit = %）。
    /// 缺省值 [`AMP_VELTRACK_DEFAULT`]；越界是明确 `Err`（不静默钳位）。
    ///
    /// 与 [`Region::velocity_curve`] 的分工：本字段是**标准曲线**的跟踪量，
    /// [`Region::velocity_curve`] 是文件**显式覆写**的点表。求值见
    /// [`Region::velocity_gain`]（显式点表优先，见那里的工程裁决）。
    pub amp_veltrack: f32,
    /// `amp_velcurve_N`：显式给出的力度 → 归一化振幅点表（规范 Range 0 to 1）。
    ///
    /// `None` 表示源文件**没有**给出任何 `amp_velcurve_N`；此时力度响应由
    /// [`Region::amp_veltrack`] 决定。刻意区分 `None` 与 `Some(曲线)`：前者是
    /// 「没说」，后者是「文件显式覆写了标准曲线」——与 [`Region::off_time`] 的
    /// `None` / `Some(0.0)` 是同一条口径。
    ///
    /// 出处 <https://sfzformat.com/opcodes/amp_velcurve_N/>：点为
    /// 「该力度处的归一化振幅 (0 to 1)」，未给出的点**线性插值**，缺省端点
    /// `amp_velcurve_0 = 0`、`amp_velcurve_127 = 1`（见
    /// [`VelocityCurve::from_points`]）。
    pub velocity_curve: Option<VelocityCurve>,
    /// 循环起点（采样点）。
    pub loop_start: u32,
    /// 循环终点（采样点）。
    pub loop_end: u32,
    /// 循环模式。
    pub loop_mode: LoopMode,
    /// 采样起始偏移（`offset`，单位采样点，规范默认 0）。
    pub offset: u32,
    /// 采样播放终点（`end`，规范默认 `unspecified`）。
    pub end: SampleEnd,
    /// 播放方向（`direction`，SFZ v2，规范默认 `forward`）。
    pub direction: PlayDirection,
    /// 微调（cents，规范范围 -100..=100）。
    pub tune: i32,
    /// 移调（半音，规范范围 -127..=127）。
    pub transpose: i32,
    /// `bend_up`：弯音轮**向上**时的弯音范围，单位音分（规范缺省
    /// [`BEND_UP_DEFAULT_CENTS`]，范围 -9600..=9600）。
    ///
    /// 出处 <https://sfzformat.com/opcodes/bend_up/> 的表格行（Type = integer，
    /// Default = 200，Range = -9600 to 9600，Unit = cents）。允许为负：规范正文写明
    /// "If `bend_up` is negative, then moving the pitch wheel up will cause the pitch
    /// to move down."
    ///
    /// **只喂给 [`Region::bend_cents`]**：本 crate 不接收弯音轮状态、不做 I/O，
    /// 因此它不改写 [`Region::pitch_ratio`]（那需要调用方给出轮值）。越界是明确
    /// [`SfzError::IntegerOutOfRange`]，不静默钳位。
    pub bend_up: i32,
    /// `bend_down`：弯音轮**向下**时的弯音范围，单位音分（规范缺省
    /// [`BEND_DOWN_DEFAULT_CENTS`]，范围 -9600..=9600）。
    ///
    /// 出处 <https://sfzformat.com/opcodes/bend_down/> 的表格行（Type = integer，
    /// Default = -200，Range = -9600 to 9600，Unit = cents）。**允许为任意符号**：
    /// 规范正文明写正值在齐特琴 / 吉他一类乐器上有用（两个方向都把音高往上弯），
    /// 登记语料里 `bend_down=1200` 出现 19 次 —— 因此本 crate 不做「必须非正」的校验。
    pub bend_down: i32,
    /// 音量（dB，规范范围 -144..=6；本实现只要求有限）。
    pub volume: f32,
    /// 声相（%，规范范围 -100..=100；本实现只要求有限）。
    pub pan: f32,
    /// 轮替序号（`seq_position`，默认 1）。
    pub seq_position: u32,
    /// 轮替长度（`seq_length`，默认 1）。
    pub seq_length: u32,
    /// 独占组（`group`）。
    pub group: u32,
    /// 被谁关掉（`off_by`）。
    pub off_by: u32,
    /// keyswitch 期望值（`sw_last`）。
    pub sw_last: Option<u8>,
    /// keyswitch 有效音域下界（`sw_lokey`）。
    pub sw_lokey: u8,
    /// keyswitch 有效音域上界（`sw_hikey`）。
    pub sw_hikey: u8,
    /// 要求按下的 keyswitch（`sw_down`）。
    pub sw_down: Option<u8>,
    /// 要求未按下的 keyswitch（`sw_up`）。
    pub sw_up: Option<u8>,
    /// MIDI CC 门控集合（按 CC 号升序，确定性）。
    pub cc_gates: Vec<CcGate>,
    /// 交叉淡化集合（`xfin_*` / `xfout_*`，见 [`crate::crossfade`]）。
    ///
    /// 顺序**固定**，因此同一输入得到同一个 `Vec`（`Eq` 与判据都依赖它）：
    /// 键盘淡入、键盘淡出、力度淡入、力度淡出，然后按 CC 号升序的 CC 淡入、
    /// 再按 CC 号升序的 CC 淡出。空集合表示该 region 没有任何 `xfin_*` / `xfout_*`，
    /// 此时 [`Region::crossfade_gain`] 恒为 1.0。
    pub crossfades: Vec<Crossfade>,
    /// 该 region 的 `<region>` 段头所在行号（1-based）。
    pub source_line: usize,
}

impl<'a> Region<'a> {
    /// 播放路径：把 `default_path` 拼到 `sample` 上（`sample` 以 `/` 开头时不拼）。
    #[must_use]
    pub fn sample_path(&self) -> Cow<'_, str> {
        match self.default_path.as_deref() {
            Some(prefix) if !prefix.is_empty() && !self.sample.starts_with('/') => {
                let separator = if prefix.ends_with('/') { "" } else { "/" };
                Cow::Owned(format!("{prefix}{separator}{}", self.sample))
            }
            _ => Cow::Borrowed(self.sample.as_ref()),
        }
    }

    /// 是否由该音符触发（`key=-1` 的 region 永远返回 `false`）。
    #[must_use]
    pub fn matches_key(&self, note: u8) -> bool {
        self.trigger_by_note && i32::from(note) >= self.lokey && i32::from(note) <= self.hikey
    }

    /// 力度是否落在 `[lovel, hivel]`。
    #[must_use]
    pub fn matches_velocity(&self, velocity: u8) -> bool {
        velocity >= self.lovel && velocity <= self.hivel
    }

    /// MIDI 通道是否落在 `[lochan, hichan]`（通道从 1 开始）。
    #[must_use]
    pub fn matches_channel(&self, channel: u8) -> bool {
        channel >= self.lochan && channel <= self.hichan
    }

    /// keyswitch 门控是否满足。
    ///
    /// `last` 是 `[sw_lokey, sw_hikey]` 范围内**最后按下**的音（调用方负责过滤），
    /// `down` 报告某个音此刻是否按下。未声明 keyswitch 的 region 恒为 `true`。
    #[must_use]
    pub fn keyswitch_ok(&self, last: Option<u8>, down: impl Fn(u8) -> bool) -> bool {
        if let Some(expected) = self.sw_last
            && last != Some(expected)
        {
            return false;
        }
        if let Some(expected) = self.sw_down
            && !down(expected)
        {
            return false;
        }
        if let Some(expected) = self.sw_up
            && down(expected)
        {
            return false;
        }
        true
    }

    /// CC 门控是否满足。`cc` 返回某个 CC 的当前值。
    #[must_use]
    pub fn cc_gates_ok(&self, cc: impl Fn(u8) -> u8) -> bool {
        self.cc_gates
            .iter()
            .all(|gate| (gate.lo..=gate.hi).contains(&cc(gate.cc)))
    }

    /// 该 region 是否声明了任何需要外部状态的 keyswitch 门控。
    #[must_use]
    pub fn has_keyswitch_gate(&self) -> bool {
        self.sw_last.is_some() || self.sw_down.is_some() || self.sw_up.is_some()
    }

    /// 生效的循环模式：`trigger=release` / `release_key` 的 region 覆盖成
    /// [`LoopMode::OneShot`]。
    ///
    /// 规范事实（<https://sfzformat.com/opcodes/trigger/>）：「Setting trigger to
    /// release or release_key will cause the region to play as if `loop_mode` was
    /// set to **one_shot**」。该覆盖是**读取语义**上的，不改写 `region.loop_mode`
    /// 存的原样取值；[`Region::loop_window`] 与 [`crate::PlaybackSpec`] 用本方法。
    #[must_use]
    pub fn effective_loop_mode(&self) -> LoopMode {
        if self.trigger.is_release_family() {
            LoopMode::OneShot
        } else {
            self.loop_mode
        }
    }

    /// 生效的 `off_time`（秒）：源文件给出则原样返回，否则返回规范缺省
    /// [`OFF_TIME_DEFAULT_SECONDS`]（0.006 s）。
    ///
    /// 该时长只在 `off_mode=time` 关断声部时使用（见
    /// <https://sfzformat.com/opcodes/off_time/>）；`off_mode=fast` / `normal` 的
    /// 关断语义不由它决定。
    ///
    /// 零分配、可在实时路径调用（读两个标量字段）。
    #[must_use]
    pub fn effective_off_time(&self) -> f32 {
        self.off_time.unwrap_or(OFF_TIME_DEFAULT_SECONDS)
    }

    /// 该 region 是否需要「其它按住的音符数」这类外部状态（`trigger=first` /
    /// `legato`）。
    #[must_use]
    pub fn has_held_notes_gate(&self) -> bool {
        matches!(self.trigger, Trigger::First | Trigger::Legato)
    }

    /// `amp_veltrack` 标准曲线的力度 → 线性振幅（[`veltrack_gain`] 的绑定版）。
    ///
    /// 只在 [`Region::velocity_curve`] 为 `None` 时参与 [`Region::velocity_gain`]；
    /// 单独暴露是为了让消费方能显式选择「忽略文件里的显式点表、只用 `amp_veltrack`」。
    ///
    /// 零分配、可在实时路径调用（读一个标量字段 + 一个 `powf`）。
    #[must_use]
    pub fn veltrack_gain(&self, velocity: u8) -> f32 {
        veltrack_gain(velocity, self.amp_veltrack)
    }

    /// 力度 → **线性振幅**（0.0 = 静音，1.0 = 满幅）。
    ///
    /// 规则（出处与工程裁决见 [`crate::velocity`] 的模块文档）：
    ///
    /// | 条件 | 结果 |
    /// | :--- | :--- |
    /// | 文件给出了 `amp_velcurve_N` | [`VelocityCurve::amplitude`]（规范线性插值 + 缺省端点） |
    /// | 否则 | [`Region::veltrack_gain`]（`amp_veltrack` 幂律，缺省 100 ⇒ `(v/127)^2`） |
    ///
    /// **工程裁决**：两套都给出时**显式点表胜**。依据是 `amp_velcurve_N` 页的表格行把
    /// 该 opcode 的 Default 写成 "Standard curve (see `amp_veltrack`)" ——
    /// 即点表的缺省**就是** `amp_veltrack` 的标准曲线，所以文件一旦给出点表，它就是
    /// 对标准曲线的**覆写**而不是叠乘。同页正文也把两者说成**替代**关系：
    /// "Both `amp_velcurve_n` and `amp_veltrack` can be used together, though there's
    /// probably more risk of confusion than benefit to doing this."
    /// （登记语料里有 21 个文件同时给出两者，所以这不是纯理论问题。）
    ///
    /// **与 [`crate::playback::PlaybackSpec::gain`] 的分工**：那个只是 `volume` 的 dB→线性
    /// 换算，**不含**力度；本方法的结果由
    /// [`crate::playback::PlaybackSpec::velocity_gain`] 单独携带，两者相乘见
    /// [`crate::playback::PlaybackSpec::total_gain`]。这样「只折 volume」的既有消费方
    /// 一个字都不用改。
    ///
    /// 零分配、无锁、无 I/O，可在实时路径调用。
    #[must_use]
    pub fn velocity_gain(&self, velocity: u8) -> f32 {
        match &self.velocity_curve {
            Some(curve) => curve.amplitude(velocity),
            None => self.veltrack_gain(velocity),
        }
    }

    /// 交叉淡化增益（`xfin_*` / `xfout_*`）：0.0 = 静音，1.0 = 满幅。
    ///
    /// 规则（出处与工程裁决见 [`crate::crossfade`]）：
    ///
    /// | 条件 | 结果 |
    /// | :--- | :--- |
    /// | 该 region 没有任何 `xfin_*` / `xfout_*` | 1.0（恒等） |
    /// | 一段区间长度非正（`high <= low`） | 该段 1.0（不改变音量） |
    /// | 键盘位置轴 | `query.note` |
    /// | 力度轴 | `query.velocity` |
    /// | CC 轴且 `query.cc` 给出了探针 | 探针读数 |
    /// | CC 轴且**未**给出探针 | 该段 1.0 |
    ///
    /// 多段（多个 CC、以及淡入与淡出同时存在）的合并方式是**相乘** ——
    /// 规范正文说同时使用 `xfin_*` 与 `xfout_*` 时各 region「all will be triggered -
    /// but some of them may play at zero volume」（<https://sfzformat.com/opcodes/xfin_loccN/>），
    /// 即各段各自给出一个 [0, 1] 的音量因子。
    ///
    /// **未提供 CC 状态时取 1.0 而不是 0.0** 是刻意的：该轴于是**不贡献**衰减，
    /// 与既有行为（本 crate 在此之前完全忽略这一族）一致，也不会把整层静音。
    /// 这与 [`Region::cc_gates_ok`] 的严格策略（未提供状态 ⇒ 不匹配）不同，
    /// 因为那条决定「region 是否发声」，这条只决定「发声的层音量多大」。
    ///
    /// 零分配、无锁、无阻塞 I/O、无日志：遍历已构造好的切片，逐段做整数减法、
    /// 一次浮点除法与（`power` 曲线时）一次 `sqrt`。可在逐样本路径调用。
    #[must_use]
    pub fn crossfade_gain(&self, query: &RegionQuery<'_>) -> f32 {
        let mut gain = 1.0f32;
        for crossfade in &self.crossfades {
            let value = match crossfade.axis {
                XfAxis::Key => query.note,
                XfAxis::Velocity => query.velocity,
                XfAxis::Cc(cc) => match query.cc {
                    Some(probe) => probe(cc),
                    None => continue,
                },
            };
            gain *= crossfade.gain_at(value);
        }
        gain
    }
}

/// 一次触发查询的条件。
///
/// **严格策略**（刻意选择，记在 notes）：region 声明了 keyswitch / CC / `first` /
/// `legato` 门控，而查询没有提供对应状态时，该 region **不**匹配。这样「未接线的门控」
/// 不会悄悄发声，也保证同一输入永远同一结果 [ARCH-DET-001]。
pub struct RegionQuery<'q> {
    /// 触发的音符（MIDI 号 0..=127）。
    pub note: u8,
    /// 力度 0..=127。
    pub velocity: u8,
    /// MIDI 通道（1..=16）。
    pub channel: u8,
    /// 第几次触发（0-based）。轮替 `seq_length` 靠它确定性选择。
    pub occurrence: u64,
    /// 触发事件：note-on（[`RegionQuery::new`] 的缺省）或 note-off。
    pub event: TriggerEvent,
    /// **其它**音符此刻的按住数（`trigger=first` / `legato` 用）。
    ///
    /// `None` 表示调用方未提供该状态；严格策略下 `first` / `legato` region 因此不匹配。
    pub held_notes: Option<u32>,
    /// `[sw_lokey, sw_hikey]` 范围内最后按下的音。
    pub last_keyswitch: Option<u8>,
    /// 某音此刻是否按下。
    pub keys_down: Option<&'q dyn Fn(u8) -> bool>,
    /// 某 CC 的当前值。
    pub cc: Option<&'q dyn Fn(u8) -> u8>,
}

impl<'q> RegionQuery<'q> {
    /// 最简查询：note-on，只有音符与力度，通道默认 1，轮替序号 0，门控状态未提供。
    #[must_use]
    pub fn new(note: u8, velocity: u8) -> Self {
        Self {
            note,
            velocity,
            channel: 1,
            occurrence: 0,
            event: TriggerEvent::NoteOn,
            held_notes: None,
            last_keyswitch: None,
            keys_down: None,
            cc: None,
        }
    }

    /// note-off 查询（`trigger=release` / `release_key` 用）。
    ///
    /// `pedal_down` 是延音踏板此刻是否踩下：踩下时 `release` region 不匹配，
    /// `release_key` 仍然匹配（见 [`TriggerEvent::NoteOff`]）。
    #[must_use]
    pub fn note_off(note: u8, velocity: u8, pedal_down: bool) -> Self {
        Self::new(note, velocity).with_event(TriggerEvent::NoteOff { pedal_down })
    }

    /// 设置触发事件。
    #[must_use]
    pub fn with_event(mut self, event: TriggerEvent) -> Self {
        self.event = event;
        self
    }

    /// 设置「其它音符的按住数」（`trigger=first` / `legato` 用）。
    #[must_use]
    pub fn with_held_notes(mut self, held_notes: u32) -> Self {
        self.held_notes = Some(held_notes);
        self
    }

    /// 设置轮替触发序号（确定性轮替）。
    #[must_use]
    pub fn with_occurrence(mut self, occurrence: u64) -> Self {
        self.occurrence = occurrence;
        self
    }

    /// 设置 MIDI 通道。
    #[must_use]
    pub fn with_channel(mut self, channel: u8) -> Self {
        self.channel = channel;
        self
    }

    /// 设置 keyswitch 状态。
    #[must_use]
    pub fn with_keyswitch(mut self, last: u8, keys_down: &'q dyn Fn(u8) -> bool) -> Self {
        self.last_keyswitch = Some(last);
        self.keys_down = Some(keys_down);
        self
    }

    /// 设置 CC 状态。
    #[must_use]
    pub fn with_cc(mut self, cc: &'q dyn Fn(u8) -> u8) -> Self {
        self.cc = Some(cc);
        self
    }
}

/// 一个已解析的 SFZ 乐器：region 列表 + 解析警告。
///
/// 一次调用完成「选 region → 可渲染的采样描述」见 [`Instrument::playback_for`]
/// （实现在 [`crate::playback`]）。
#[derive(Debug, Clone, PartialEq)]
pub struct Instrument<'a> {
    regions: Vec<Region<'a>>,
    /// `<curve>` 段定义的调制曲线（文件出现顺序，确定性；同一编号重复定义会被拒绝）。
    curves: Vec<Curve>,
    /// `<effect>` 段定义的效果器总线声明（文件出现顺序，确定性；不去重）。
    effects: Vec<Effect<'a>>,
    /// `<midi>` 段定义的 MIDI 预处理器声明（文件出现顺序，确定性；不去重、空段也登记、
    /// 段内 opcode 原样保存不解释）。
    midi_sections: Vec<MidiSection<'a>>,
    /// 按音符分桶的 region 下标（加速 `region_for`，构造后只读）。
    key_buckets: Vec<Vec<u32>>,
    warnings: Vec<Warning>,
}

impl<'a> Instrument<'a> {
    /// 从 region 列表构造乐器（并建立音符索引）。
    pub(crate) fn new(
        regions: Vec<Region<'a>>,
        curves: Vec<Curve>,
        effects: Vec<Effect<'a>>,
        midi_sections: Vec<MidiSection<'a>>,
        warnings: Vec<Warning>,
    ) -> Self {
        let mut key_buckets: Vec<Vec<u32>> = vec![Vec::new(); 128];
        for (index, region) in regions.iter().enumerate() {
            if !region.trigger_by_note {
                continue;
            }
            let low = region.lokey.clamp(0, 127);
            let high = region.hikey.clamp(0, 127);
            if low > high {
                continue;
            }
            for note in low..=high {
                key_buckets[note as usize].push(index as u32);
            }
        }
        Self {
            regions,
            curves,
            effects,
            midi_sections,
            key_buckets,
            warnings,
        }
    }

    /// 全部 region（保持文件出现顺序，确定性）。
    #[must_use]
    pub fn regions(&self) -> &[Region<'a>] {
        &self.regions
    }

    /// 全部 `<curve>`（保持文件出现顺序，确定性；`curve_index` 不保证升序）。
    #[must_use]
    pub fn curves(&self) -> &[Curve] {
        &self.curves
    }

    /// 按 `curve_index` 取一条**文件内定义**的曲线（线性扫描，零分配）。
    ///
    /// 取不到时调用方可以退回 ARIA 内建曲线，见 [`Curve::built_in`]。
    #[must_use]
    pub fn curve(&self, index: u8) -> Option<&Curve> {
        self.curves.iter().find(|curve| curve.index() == index)
    }

    /// 全部 `<effect>` 段（保持文件出现顺序，确定性；同一个 `bus` 上可以有多条）。
    ///
    /// 只含登记到了至少一个规范 opcode 的段：空 `<effect>` 段（含「只写了非规范 opcode」
    /// 的段）不产生条目，见 [`Effect`]。
    #[must_use]
    pub fn effects(&self) -> &[Effect<'a>] {
        &self.effects
    }

    /// 全部 `<midi>` 段（保持文件出现顺序，确定性；段不去重、空段也在内）。
    ///
    /// 段内 opcode 是**原样**登记的（名字 / 取值 / 行号），本 crate **不解释**它们的语义：
    /// ARIA 的 `<midi>` opcode 词汇跨播放器不一致，且本切片无法核验规范表。
    /// 读取本切片零分配、无锁、无 I/O，可在实时路径使用。
    #[must_use]
    pub fn midi_sections(&self) -> &[MidiSection<'a>] {
        &self.midi_sections
    }

    /// 文件是否声明了 ARIA 的 MIDI 预处理器。
    ///
    /// 规范原文（<https://sfzformat.com/headers/midi/>，转引自 [`EffectBus::Midi`]）：
    /// "From ARIA v1.0.8.0+ an `<effect>` section with a `bus=midi` can be used instead."
    /// ⇒ **两种写法等价**，任一出现即为 `true`：至少一个 `<midi>` 段，或至少一条
    /// `bus=midi` 的 `<effect>` 声明。
    ///
    /// 零分配、无锁、无 I/O，可在实时路径调用。
    #[must_use]
    pub fn midi_preprocessor_declared(&self) -> bool {
        !self.midi_sections.is_empty()
            || self
                .effects
                .iter()
                .any(|effect| effect.bus() == EffectBus::Midi)
    }

    /// 求一条曲线的值：**文件内定义的优先**，否则退回 ARIA 内建曲线。
    ///
    /// 返回 `None` 表示这个编号既没有定义、也不是内建（`0..=254` 之外，或 4..=6 这类
    /// 规范只说 `Nonlinear` 而没给公式的内建曲线）。该调用零分配、无锁、无 I/O，
    /// 可在实时路径使用。
    #[must_use]
    pub fn curve_value_at(&self, index: u8, x: f32) -> Option<f32> {
        if let Some(curve) = self.curve(index) {
            return Some(curve.value_at(x));
        }
        Curve::built_in(index).map(|curve| curve.value_at(x))
    }

    /// region 数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.regions.len()
    }

    /// 是否没有任何 region。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    /// 解析期间记录的非致命警告。
    #[must_use]
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// 选择该音符 / 力度应该演奏的 region（note-on 事件，轮替序号为 0）。
    ///
    /// 等价于 `region_for_with(RegionQuery::new(note, velocity))`。
    /// note-on 事件**不会**选中 `trigger=release` / `release_key` 的 region
    /// （它们由 [`RegionQuery::note_off`] 选中）。
    #[must_use]
    pub fn region_for(&self, note: u8, velocity: u8) -> Option<&Region<'a>> {
        self.region_for_with(RegionQuery::new(note, velocity))
    }

    /// 带完整上下文（轮替序号 / 通道 / keyswitch / CC）确定性地选择 region。
    ///
    /// 轮替算法（**不使用随机数**，满足 [ARCH-DET-001]）：
    /// 1. 在候选里找第一个完整匹配的 region，取它的 `seq_length` 作为组长 `L`；
    /// 2. 目标序号 `target = (occurrence % L) + 1`；
    /// 3. 返回第一个 `seq_position == target` 且完整匹配的 region；
    /// 4. 若无人命中，退回「第一个完整匹配」。
    ///
    /// 全过程只做三次线性扫描，**零分配**，可在实时路径调用。
    #[must_use]
    pub fn region_for_with(&self, query: RegionQuery<'_>) -> Option<&Region<'a>> {
        self.region_for_query(&query)
    }

    /// [`Instrument::region_for_with`] 的**借用**版（同一个算法）。
    ///
    /// 供 [`Instrument::playback_for`] 复用同一次查询：它既要选 region，又要把
    /// `RegionQuery` 交给 [`Region::crossfade_gain`]（那需要 CC 状态），而
    /// `RegionQuery` 不实现 `Clone`。全过程只做三次线性扫描，**零分配**。
    pub(crate) fn region_for_query(&self, query: &RegionQuery<'_>) -> Option<&Region<'a>> {
        let bucket = self.key_buckets.get(query.note as usize)?;

        let mut seq_length = 1u32;
        for &index in bucket {
            let region = &self.regions[index as usize];
            if region_matches(region, query) {
                seq_length = region.seq_length.max(1);
                break;
            }
        }

        let target = (query.occurrence % u64::from(seq_length)) as u32 + 1;
        for &index in bucket {
            let region = &self.regions[index as usize];
            if region.seq_position == target && region_matches(region, query) {
                return Some(region);
            }
        }

        for &index in bucket {
            let region = &self.regions[index as usize];
            if region_matches(region, query) {
                return Some(region);
            }
        }
        None
    }
}

/// 单个 region 是否满足查询（严格门控策略见 [`RegionQuery`]）。
fn region_matches(region: &Region<'_>, query: &RegionQuery<'_>) -> bool {
    if !region.matches_key(query.note)
        || !region.matches_velocity(query.velocity)
        || !region.matches_channel(query.channel)
    {
        return false;
    }

    // `trigger` 事件门控：note-on 事件永不选中 `trigger=release` / `release_key`
    // 的 region，note-off 事件永不选中 note-on 家族的 region。
    if !region.trigger.responds_to(query.event) || !region.trigger.held_notes_ok(query.held_notes) {
        return false;
    }

    if region.has_keyswitch_gate() {
        if region.sw_last.is_some() && query.last_keyswitch.is_none() {
            return false;
        }
        if (region.sw_down.is_some() || region.sw_up.is_some()) && query.keys_down.is_none() {
            return false;
        }
        let down = |key: u8| query.keys_down.is_some_and(|probe| probe(key));
        if !region.keyswitch_ok(query.last_keyswitch, down) {
            return false;
        }
    }

    if !region.cc_gates.is_empty() {
        let Some(probe) = query.cc else {
            return false;
        };
        if !region.cc_gates_ok(probe) {
            return false;
        }
    }

    true
}

// ---------------------------------------------------------------------------
// <region> 归约
// ---------------------------------------------------------------------------

/// 按 `region → group → master → global` 的优先级读取 opcode。
///
/// 四级链的规范依据："The master header is an extra level added inbetween group and
/// global for the ARIA player." <https://sfzformat.com/headers/>
struct Scopes<'m, 'a> {
    region: &'m OpcodeMap<'a>,
    group: &'m OpcodeMap<'a>,
    master: &'m OpcodeMap<'a>,
    global: &'m OpcodeMap<'a>,
    line: usize,
}

impl<'a> Scopes<'_, 'a> {
    fn get(&self, name: &'static str) -> Option<OpcodeValue<'a>> {
        [self.region, self.group, self.master, self.global]
            .into_iter()
            .find_map(|map| map.get(name))
            .map(|value| OpcodeValue::new(name, value.clone(), self.line))
    }
}

/// 把四个作用域归约成一个可播放的 [`Region`]。
///
/// 返回 `Ok(None)` 表示该 `<region>` 没有 `sample`（例如纯 keyswitch 映射）：
/// 调用方应丢弃它并记一条 [`Warning::RegionWithoutSample`]。
///
/// **优先级**（与 ARIA 一致，见 <https://sfzformat.com/opcodes/key/>）：
/// 显式 `pitch_keycenter` 永远胜过 `key`；显式 `lokey` / `hikey` 胜过 `key` 推导值。
/// 作用域优先级是 `region → group → master → global`
/// （`<master>` 是 ARIA 在 `group` 与 `global` 之间加的一层，见
/// <https://sfzformat.com/headers/>）。
pub(crate) fn build_region<'a>(
    region_map: &OpcodeMap<'a>,
    group_map: &OpcodeMap<'a>,
    master_map: &OpcodeMap<'a>,
    global_map: &OpcodeMap<'a>,
    default_path: Option<Cow<'a, str>>,
    line: usize,
) -> Result<Option<Region<'a>>, SfzError> {
    let scopes = Scopes {
        region: region_map,
        group: group_map,
        master: master_map,
        global: global_map,
        line,
    };

    let Some(sample_value) = scopes.get("sample") else {
        return Ok(None);
    };
    if sample_value.as_str().is_empty() {
        return Ok(None);
    }
    let sample = sample_value.value;

    // ---- 触发事件（`trigger`，见 <https://sfzformat.com/opcodes/trigger/>） ----
    let trigger = match scopes.get("trigger") {
        Some(value) => value.as_option(Trigger::OPTIONS, Trigger::ALLOWED)?,
        None => Trigger::Attack,
    };

    // ---- off 语义（`off_mode`，见 <https://sfzformat.com/opcodes/off_mode/>） ----
    // 三个取值与缺省 `fast` 的出处是登记语料普查（`docs/ledger/sfz-core-notes.md` 第 11 节）。
    // `off_time` 一并读取：格式页把它定义为「`off_mode=time` 时被窃取静音的 region 的
    // 淡化时长」，与 [ARCH-RT-004] 的 3 ms 窃取淡出是**两个**量（后者是本 crate 的工程常量），
    // 因此这里只带出原样取值，不改写任何既定行为。
    let off_mode = match scopes.get("off_mode") {
        Some(value) => value.as_option(OffMode::OPTIONS, OffMode::ALLOWED)?,
        None => OffMode::Fast,
    };
    let off_time = match scopes.get("off_time") {
        // 规范表格 Range 为空、Default = 0.006（<https://sfzformat.com/opcodes/off_time/>）：
        // 只要求有限且非负；缺省记 `None`（未给出），由 `effective_off_time` 回退。
        Some(value) => {
            let seconds = value.as_f32()?;
            if seconds < 0.0 {
                return Err(SfzError::InvalidDuration {
                    line,
                    opcode: "off_time".to_string(),
                    value: seconds,
                });
            }
            Some(seconds)
        }
        None => None,
    };

    // ---- 力度 → 振幅（`amp_veltrack` / `amp_velcurve_N`，见 crate::velocity） ----
    // 规范：Default = 100，Range = -100 to 100（<https://sfzformat.com/opcodes/amp_veltrack/>）。
    let amp_veltrack = match scopes.get("amp_veltrack") {
        Some(value) => {
            let tracked = value.as_f32()?;
            if !(AMP_VELTRACK_MIN..=AMP_VELTRACK_MAX).contains(&tracked) {
                return Err(SfzError::FloatOutOfRange {
                    line,
                    opcode: "amp_veltrack".to_string(),
                    value: tracked,
                    min: AMP_VELTRACK_MIN,
                    max: AMP_VELTRACK_MAX,
                });
            }
            tracked
        }
        None => AMP_VELTRACK_DEFAULT,
    };
    let velocity_curve = read_velocity_curve(&scopes, line)?;

    // ---- 键映射 ----
    let key = match scopes.get("key") {
        Some(value) => Some(value.as_note(-1, 127)?),
        None => None,
    };
    let trigger_by_note = key != Some(-1);
    let derived = key.filter(|value| *value >= 0);
    let lokey = match scopes.get("lokey") {
        Some(value) => value.as_int(0, 127)? as i32,
        None => derived.unwrap_or(0),
    };
    let hikey = match scopes.get("hikey") {
        Some(value) => value.as_int(0, 127)? as i32,
        None => derived.unwrap_or(127),
    };
    let pitch_keycenter = match scopes.get("pitch_keycenter") {
        Some(value) => value.as_note(-127, 127)?,
        None => derived.unwrap_or(60),
    };

    // ---- 力度 / 通道 ----
    let lovel = read_u8(&scopes, "lovel", 0, 127, 0)?;
    let hivel = read_u8(&scopes, "hivel", 0, 127, 127)?;
    let lochan = read_u8(&scopes, "lochan", 1, 16, 1)?;
    let hichan = read_u8(&scopes, "hichan", 1, 16, 16)?;

    // ---- 循环 ----
    let loop_start = read_u32(&scopes, "loop_start", Some("loopstart"), 0)?;
    let loop_end = read_u32(&scopes, "loop_end", Some("loopend"), 0)?;
    let loop_mode = match scopes.get("loop_mode").or_else(|| scopes.get("loopmode")) {
        Some(value) => value.as_option(LoopMode::OPTIONS, LoopMode::ALLOWED)?,
        None => LoopMode::NoLoop,
    };

    // ---- 播放区间 / 方向（显式区间与方向，见 <https://sfzformat.com/opcodes/offset/>、
    //      <https://sfzformat.com/opcodes/end/>、<https://sfzformat.com/opcodes/direction/>） ----
    let offset = read_u32(&scopes, "offset", None, 0)?;
    let end = match scopes.get("end") {
        // 规范取值域是 `0 to 4294967296` 再加上文档化的 `-1`（不发声）哨兵。
        Some(value) => match value.as_int(-1, i64::from(u32::MAX))? {
            -1 => SampleEnd::Silent,
            end => SampleEnd::Inclusive(end as u32),
        },
        None => SampleEnd::Unspecified,
    };
    let direction = match scopes.get("direction") {
        Some(value) => value.as_option(PlayDirection::OPTIONS, PlayDirection::ALLOWED)?,
        None => PlayDirection::Forward,
    };

    // ---- 音高 / 电平 ----
    let tune = read_i32(&scopes, "tune", -100, 100, 0)?;
    let transpose = read_i32(&scopes, "transpose", -127, 127, 0)?;
    // 弯音范围：两个端点各自独立，规范缺省 200 / -200，Range 都是 ±9600 音分
    // （<https://sfzformat.com/opcodes/bend_up/>、<https://sfzformat.com/opcodes/bend_down/>）。
    let bend_up = read_i32(
        &scopes,
        "bend_up",
        -i64::from(BEND_RANGE_MAX_CENTS),
        i64::from(BEND_RANGE_MAX_CENTS),
        BEND_UP_DEFAULT_CENTS,
    )?;
    let bend_down = read_i32(
        &scopes,
        "bend_down",
        -i64::from(BEND_RANGE_MAX_CENTS),
        i64::from(BEND_RANGE_MAX_CENTS),
        BEND_DOWN_DEFAULT_CENTS,
    )?;
    let volume = match scopes.get("volume").or_else(|| scopes.get("gain")) {
        Some(value) => value.as_f32()?,
        None => 0.0,
    };
    let pan = match scopes.get("pan") {
        Some(value) => value.as_f32()?,
        None => 0.0,
    };

    // ---- 轮替 / 分组 ----
    let seq_position = read_u32(&scopes, "seq_position", None, 1)?;
    let seq_length = read_u32(&scopes, "seq_length", None, 1)?;
    let group = read_u32(&scopes, "group", Some("polyphony_group"), 0)?;
    let off_by = read_u32(&scopes, "off_by", Some("offby"), 0)?;

    // ---- keyswitch ----
    let sw_last = match scopes.get("sw_last") {
        Some(value) => Some(value.as_note(0, 127)? as u8),
        None => None,
    };
    let sw_lokey = read_u8(&scopes, "sw_lokey", 0, 127, 0)?;
    let sw_hikey = read_u8(&scopes, "sw_hikey", 0, 127, 127)?;
    let sw_down = match scopes.get("sw_down") {
        Some(value) => Some(value.as_note(0, 127)? as u8),
        None => None,
    };
    let sw_up = match scopes.get("sw_up") {
        Some(value) => Some(value.as_note(0, 127)? as u8),
        None => None,
    };

    // ---- CC 门控（global → master → group → region 覆盖） ----
    let mut gates: BTreeMap<u8, (Option<u8>, Option<u8>)> = BTreeMap::new();
    for map in [global_map, master_map, group_map, region_map] {
        for (name, value) in map {
            let Some((is_low, cc)) = parse_cc_gate_name(name.as_ref()) else {
                continue;
            };
            let entry = gates.entry(cc).or_insert((None, None));
            let parsed = parse_int(value.as_ref()).ok_or_else(|| SfzError::InvalidInteger {
                line,
                opcode: name.to_string(),
                value: value.to_string(),
            })?;
            if !(0..=127).contains(&parsed) {
                return Err(SfzError::IntegerOutOfRange {
                    line,
                    opcode: name.to_string(),
                    value: parsed,
                    min: 0,
                    max: 127,
                });
            }
            let parsed = parsed as u8;
            if is_low {
                entry.0 = Some(parsed);
            } else {
                entry.1 = Some(parsed);
            }
        }
    }
    let cc_gates = gates
        .into_iter()
        .map(|(cc, (lo, hi))| CcGate {
            cc,
            lo: lo.unwrap_or(0),
            hi: hi.unwrap_or(127),
        })
        .collect();

    // ---- 交叉淡化（xfin_* / xfout_*，见 crate::crossfade） ----
    let crossfades = read_crossfades(&scopes, line)?;

    Ok(Some(Region {
        sample,
        default_path: default_path.filter(|path| !path.is_empty()),
        lokey,
        hikey,
        pitch_keycenter,
        trigger_by_note,
        trigger,
        off_mode,
        off_time,
        lovel,
        hivel,
        lochan,
        hichan,
        amp_veltrack,
        velocity_curve,
        loop_start,
        loop_end,
        loop_mode,
        offset,
        end,
        direction,
        tune,
        transpose,
        bend_up,
        bend_down,
        volume,
        pan,
        seq_position,
        seq_length,
        group,
        off_by,
        sw_last,
        sw_lokey,
        sw_hikey,
        sw_down,
        sw_up,
        cc_gates,
        crossfades,
        source_line: line,
    }))
}

/// 识别 `loccN` / `hiccN`，返回 `(是否下界, CC 号)`。
fn parse_cc_gate_name(name: &str) -> Option<(bool, u8)> {
    let (is_low, digits) = if let Some(rest) = name.strip_prefix("locc") {
        (true, rest)
    } else {
        (false, name.strip_prefix("hicc")?)
    };
    if digits.is_empty() {
        return None;
    }
    let cc: u8 = digits.parse().ok()?;
    if cc > 127 { None } else { Some((is_low, cc)) }
}

/// 识别 `amp_velcurve_N`，返回**下标文本** `N`。
///
/// 只在 `amp_velcurve_` 之后是**全 ASCII 数字**时命中；其它形态（如
/// `amp_velcurve_foo`、`amp_velcurve_`）返回 `None`，按未知 opcode 忽略 ——
/// 这条口径与 [`parse_cc_gate_name`] 一致。
///
/// 返回文本而不是 `u8`，是为了把「数字但越界 / 溢出」的输入交给调用方报
/// [`SfzError::VelocityCurveIndexOutOfRange`] 并带上原始文本（`parse::<u8>()` 溢出时
/// 拿不到值，且 `IntegerOutOfRange` 的字段是 `i64`，装不下畸形的超长数字串）。
fn parse_velocity_curve_name(name: &str) -> Option<&str> {
    let digits = name.strip_prefix("amp_velcurve_")?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(digits)
}

/// 把四个作用域里的 `amp_velcurve_N` 归约成一条 [`VelocityCurve`]。
///
/// 优先级：`region → group → master → global`（与其它 opcode 同一条链，见
/// [`Scopes::get`]），实现方式是按 `global → master → group → region` 的顺序喂点，
/// 同一 `N` 上后者胜。
///
/// `Ok(None)` 表示四个作用域都没有给出任何 `amp_velcurve_N`：此时力度响应由
/// `amp_veltrack` 决定（见 [`Region::velocity_gain`]）。刻意区分 `None` 与
/// 「给了点但都是缺省端点」——后者是文件的显式覆写。
///
/// 取值域：`N` 必须能解析成 `0..=127`（规范正文 "N can be from 0 to 127"）；
/// 取值必须落在规范表格的 `0 to 1`。两者越界都是明确 `Err`（不静默丢弃、不钳位）。
fn read_velocity_curve(
    scopes: &Scopes<'_, '_>,
    line: usize,
) -> Result<Option<VelocityCurve>, SfzError> {
    let mut points: BTreeMap<u8, f32> = BTreeMap::new();
    for map in [scopes.global, scopes.master, scopes.group, scopes.region] {
        for (name, value) in map {
            let Some(digits) = parse_velocity_curve_name(name.as_ref()) else {
                continue;
            };
            let index: u8 = digits
                .parse()
                .ok()
                .filter(|index| (MIN_VELCURVE_INDEX..=MAX_VELCURVE_INDEX).contains(index))
                .ok_or_else(|| SfzError::VelocityCurveIndexOutOfRange {
                    line,
                    opcode: name.to_string(),
                    index: digits.to_string(),
                })?;
            let opcode = || format!("amp_velcurve_{index}");
            let amplitude = match value.as_ref().parse::<f32>() {
                Err(_) => {
                    return Err(SfzError::InvalidFloat {
                        line,
                        opcode: opcode(),
                        value: value.to_string(),
                    });
                }
                Ok(amplitude) if !amplitude.is_finite() => {
                    return Err(SfzError::NonFiniteFloat {
                        line,
                        opcode: opcode(),
                        value: value.to_string(),
                    });
                }
                Ok(amplitude)
                    if !(MIN_VELCURVE_AMPLITUDE..=MAX_VELCURVE_AMPLITUDE).contains(&amplitude) =>
                {
                    return Err(SfzError::FloatOutOfRange {
                        line,
                        opcode: opcode(),
                        value: amplitude,
                        min: MIN_VELCURVE_AMPLITUDE,
                        max: MAX_VELCURVE_AMPLITUDE,
                    });
                }
                Ok(amplitude) => amplitude,
            };
            points.insert(index, amplitude);
        }
    }
    if points.is_empty() {
        return Ok(None);
    }
    Ok(Some(VelocityCurve::from_points(
        points
            .into_iter()
            .map(|(at, value)| CurvePoint { at, value }),
    )))
}

/// 交叉淡化的一个骨架（`xfin_*` 或 `xfout_*`）在两个端点上的归约结果。
///
/// 两个端点各自是**独立**的 opcode（例如 `xfin_lovel` 与 `xfin_hivel`），
/// 因此各自记「有没有被四个作用域里的任何一个提到」。
#[derive(Debug, Default, Clone, Copy)]
struct XfEndpoints {
    low: Option<u8>,
    high: Option<u8>,
}

impl XfEndpoints {
    /// 按方向补齐缺省端点。两个端点都没被提到时返回 `None`（该骨架不产生一段交叉淡化）。
    fn resolve(self, direction: XfDirection) -> Option<XfRange> {
        if self.low.is_none() && self.high.is_none() {
            return None;
        }
        let default = direction.default_range();
        Some(XfRange {
            low: self.low.unwrap_or(default.low),
            high: self.high.unwrap_or(default.high),
        })
    }
}

/// 识别 `xfin_loccN` / `xfin_hiccN` / `xfout_loccN` / `xfout_hiccN`，
/// 返回 `(方向, 是否下界, CC 号)`。
///
/// 只在 `locc` / `hicc` 之后是**全 ASCII 数字**且 CC 号 ≤ 127 时命中；其它形态
/// （`xfin_hicc`、`xfin_hiccfoo`、`xfin_hicc131`）返回 `None`，按未知 opcode 忽略 ——
/// 这条口径与 [`parse_cc_gate_name`] 一致。
fn parse_cc_crossfade_name(name: &str) -> Option<(XfDirection, bool, u8)> {
    let (direction, rest) = if let Some(rest) = name.strip_prefix("xfin_") {
        (XfDirection::In, rest)
    } else {
        (XfDirection::Out, name.strip_prefix("xfout_")?)
    };
    let (is_low, digits) = if let Some(rest) = rest.strip_prefix("locc") {
        (true, rest)
    } else {
        (false, rest.strip_prefix("hicc")?)
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let cc: u8 = digits.parse().ok()?;
    if cc > 127 {
        None
    } else {
        Some((direction, is_low, cc))
    }
}

/// 读一个交叉淡化端点（规范表格 Range = `0 to 127`）。
///
/// 非法整数与越界都是明确 `Err`（不静默钳位、不丢弃），与 [`read_velocity_curve`]
/// 对 `amp_velcurve_N` 取值的口径一致。
fn read_xf_endpoint(name: &str, value: &str, line: usize) -> Result<u8, SfzError> {
    let parsed = parse_int(value).ok_or_else(|| SfzError::InvalidInteger {
        line,
        opcode: name.to_string(),
        value: value.to_string(),
    })?;
    if !(0..=127).contains(&parsed) {
        return Err(SfzError::IntegerOutOfRange {
            line,
            opcode: name.to_string(),
            value: parsed,
            min: 0,
            max: 127,
        });
    }
    Ok(parsed as u8)
}

/// 把四个作用域里的 `xfin_*` / `xfout_*` 归约成一段交叉淡化集合。
///
/// 优先级是 `region → group → master → global`（与其它 opcode 同一条链，
/// 见 [`Scopes::get`]），实现方式是按 `global → master → group → region` 的顺序喂入，
/// 同一端点上后者胜。
///
/// 返回的 `Vec` 顺序固定：键盘淡入、键盘淡出、力度淡入、力度淡出、按 CC 号升序的
/// CC 淡入、按 CC 号升序的 CC 淡出 —— 保证同一输入得到同一个 `Vec`（ARCH-DET-001）。
///
/// 曲线（`xf_keycurve` / `xf_velcurve` / `xf_cccurve`）在同一个作用域链上归约：
/// **每个维度一条曲线**，作用在该维度的所有段上（与 sfizz 的
/// `crossfadeKeyCurve` / `crossfadeVelCurve` / `crossfadeCCCurve` 三个字段一致）。
fn read_crossfades(scopes: &Scopes<'_, '_>, line: usize) -> Result<Vec<Crossfade>, SfzError> {
    let mut key_in = XfEndpoints::default();
    let mut key_out = XfEndpoints::default();
    let mut vel_in = XfEndpoints::default();
    let mut vel_out = XfEndpoints::default();
    let mut cc_in: BTreeMap<u8, XfEndpoints> = BTreeMap::new();
    let mut cc_out: BTreeMap<u8, XfEndpoints> = BTreeMap::new();
    let mut key_curve = XfCurve::DEFAULT;
    let mut vel_curve = XfCurve::DEFAULT;
    let mut cc_curve = XfCurve::DEFAULT;

    for map in [scopes.global, scopes.master, scopes.group, scopes.region] {
        for (name, value) in map {
            let name = name.as_ref();
            match name {
                "xf_keycurve" => {
                    key_curve = OpcodeValue::new("xf_keycurve", value.clone(), line)
                        .as_option(XfCurve::OPTIONS, XfCurve::ALLOWED)?;
                    continue;
                }
                "xf_velcurve" => {
                    vel_curve = OpcodeValue::new("xf_velcurve", value.clone(), line)
                        .as_option(XfCurve::OPTIONS, XfCurve::ALLOWED)?;
                    continue;
                }
                "xf_cccurve" => {
                    cc_curve = OpcodeValue::new("xf_cccurve", value.clone(), line)
                        .as_option(XfCurve::OPTIONS, XfCurve::ALLOWED)?;
                    continue;
                }
                _ => {}
            }

            let endpoint = match name {
                "xfin_lokey" => Some((&mut key_in, true)),
                "xfin_hikey" => Some((&mut key_in, false)),
                "xfout_lokey" => Some((&mut key_out, true)),
                "xfout_hikey" => Some((&mut key_out, false)),
                "xfin_lovel" => Some((&mut vel_in, true)),
                "xfin_hivel" => Some((&mut vel_in, false)),
                "xfout_lovel" => Some((&mut vel_out, true)),
                "xfout_hivel" => Some((&mut vel_out, false)),
                _ => None,
            };
            if let Some((slot, is_low)) = endpoint {
                let parsed = read_xf_endpoint(name, value.as_ref(), line)?;
                if is_low {
                    slot.low = Some(parsed);
                } else {
                    slot.high = Some(parsed);
                }
                continue;
            }

            if let Some((direction, is_low, cc)) = parse_cc_crossfade_name(name) {
                let parsed = read_xf_endpoint(name, value.as_ref(), line)?;
                let slot = match direction {
                    XfDirection::In => cc_in.entry(cc).or_default(),
                    XfDirection::Out => cc_out.entry(cc).or_default(),
                };
                if is_low {
                    slot.low = Some(parsed);
                } else {
                    slot.high = Some(parsed);
                }
            }
        }
    }

    let mut crossfades = Vec::new();
    for (axis, direction, endpoints, curve) in [
        (XfAxis::Key, XfDirection::In, key_in, key_curve),
        (XfAxis::Key, XfDirection::Out, key_out, key_curve),
        (XfAxis::Velocity, XfDirection::In, vel_in, vel_curve),
        (XfAxis::Velocity, XfDirection::Out, vel_out, vel_curve),
    ] {
        if let Some(range) = endpoints.resolve(direction) {
            crossfades.push(Crossfade::new(axis, direction, range, curve));
        }
    }
    for (cc, endpoints) in cc_in {
        if let Some(range) = endpoints.resolve(XfDirection::In) {
            crossfades.push(Crossfade::new(
                XfAxis::Cc(cc),
                XfDirection::In,
                range,
                cc_curve,
            ));
        }
    }
    for (cc, endpoints) in cc_out {
        if let Some(range) = endpoints.resolve(XfDirection::Out) {
            crossfades.push(Crossfade::new(
                XfAxis::Cc(cc),
                XfDirection::Out,
                range,
                cc_curve,
            ));
        }
    }
    Ok(crossfades)
}

fn read_u8(
    scopes: &Scopes<'_, '_>,
    name: &'static str,
    min: i64,
    max: i64,
    fallback: u8,
) -> Result<u8, SfzError> {
    match scopes.get(name) {
        Some(value) => Ok(value.as_int(min, max)? as u8),
        None => Ok(fallback),
    }
}

fn read_i32(
    scopes: &Scopes<'_, '_>,
    name: &'static str,
    min: i64,
    max: i64,
    fallback: i32,
) -> Result<i32, SfzError> {
    match scopes.get(name) {
        Some(value) => Ok(value.as_int(min, max)? as i32),
        None => Ok(fallback),
    }
}

fn read_u32(
    scopes: &Scopes<'_, '_>,
    name: &'static str,
    alias: Option<&'static str>,
    fallback: u32,
) -> Result<u32, SfzError> {
    let value = scopes
        .get(name)
        .or_else(|| alias.and_then(|alias| scopes.get(alias)));
    match value {
        Some(value) => Ok(value.as_int(0, i64::from(u32::MAX))? as u32),
        None => Ok(fallback),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::CurvePoint;
    use crate::midi::MidiOpcode;
    use crate::parser::{Header, ParseLimits, parse_text};

    fn region(note: u8, seq_position: u32, seq_length: u32) -> Region<'static> {
        Region {
            sample: Cow::Borrowed("x.wav"),
            default_path: None,
            lokey: i32::from(note),
            hikey: i32::from(note),
            pitch_keycenter: i32::from(note),
            trigger_by_note: true,
            trigger: Trigger::Attack,
            off_mode: OffMode::Fast,
            off_time: None,
            lovel: 0,
            hivel: 127,
            lochan: 1,
            hichan: 16,
            amp_veltrack: AMP_VELTRACK_DEFAULT,
            velocity_curve: None,
            loop_start: 0,
            loop_end: 0,
            loop_mode: LoopMode::NoLoop,
            offset: 0,
            end: SampleEnd::Unspecified,
            direction: PlayDirection::Forward,
            tune: 0,
            transpose: 0,
            bend_up: BEND_UP_DEFAULT_CENTS,
            bend_down: BEND_DOWN_DEFAULT_CENTS,
            volume: 0.0,
            pan: 0.0,
            seq_position,
            seq_length,
            group: 0,
            off_by: 0,
            sw_last: None,
            sw_lokey: 0,
            sw_hikey: 127,
            sw_down: None,
            sw_up: None,
            cc_gates: Vec::new(),
            crossfades: Vec::new(),
            source_line: 1,
        }
    }

    #[test]
    fn key_and_pitch_keycenter_precedence_follows_aria() {
        let instrument = parse_text(
            "<region>key=72 pitch_keycenter=60 sample=a.wav",
            &Default::default(),
        )
        .expect("parses");
        let region = &instrument.regions()[0];
        assert_eq!(region.lokey, 72);
        assert_eq!(region.hikey, 72);
        // 显式 pitch_keycenter 永远胜过 key（ARIA 行为）。
        assert_eq!(region.pitch_keycenter, 60);
    }

    #[test]
    fn key_minus_one_disables_note_trigger() {
        let instrument =
            parse_text("<region>key=-1 sample=a.wav", &Default::default()).expect("parses");
        let region = &instrument.regions()[0];
        assert!(!region.trigger_by_note);
        assert!(instrument.region_for(60, 100).is_none());
    }

    #[test]
    fn sample_path_joins_default_path() {
        let instrument = parse_text(
            "<control>\ndefault_path=Samples/\n<region>sample=kick.wav\n",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(instrument.len(), 1);
        assert_eq!(instrument.regions()[0].sample_path(), "Samples/kick.wav");
        // 以 `/` 开头的 sample 不拼 default_path。
        let absolute = parse_text(
            "<control>\ndefault_path=Samples/\n<region>sample=/abs/kick.wav\n",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(absolute.regions()[0].sample_path(), "/abs/kick.wav");
    }

    #[test]
    fn region_inherits_group_values_even_when_a_later_group_header_intervenes() {
        // 回归判据：`<region>` 的归约必须发生在**离开 region 作用域时**，
        // 而不是等下一个 `<region>` 段头 —— 否则中途的 `<group>` 清空 group 表，
        // 会让尚未归约的 region 丢掉继承值。
        let instrument = parse_text(
            "<group>key=36\n<region>sample=a.wav\n<group>key=48\n<region>sample=b.wav\n",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(instrument.len(), 2);
        assert_eq!(instrument.regions()[0].lokey, 36);
        assert_eq!(instrument.regions()[0].hikey, 36);
        assert_eq!(instrument.regions()[1].lokey, 48);
        assert_eq!(instrument.regions()[1].hikey, 48);
    }

    #[test]
    fn master_values_are_looked_up_between_group_and_global() {
        // 四级作用域链：region → group → master → global。
        // 每一行都比上一行少一个更内层的作用域，因此 `volume` 的读数逐级外退。
        let volume_of = |text: &str| {
            parse_text(text, &Default::default())
                .expect("parses")
                .regions()[0]
                .volume
        };
        let all_four = volume_of(
            "<global>volume=-1\n\
             <master>volume=-2\n\
             <group>volume=-3\n\
             <region>sample=a.wav volume=-4",
        );
        assert_eq!(all_four, -4.0, "region wins over group/master/global");

        let without_region = volume_of(
            "<global>volume=-1\n\
             <master>volume=-2\n\
             <group>volume=-3\n\
             <region>sample=a.wav",
        );
        assert_eq!(without_region, -3.0, "group wins over master/global");

        let without_group = volume_of(
            "<global>volume=-1\n\
             <master>volume=-2\n\
             <region>sample=a.wav",
        );
        assert_eq!(without_group, -2.0, "master wins over global");

        let only_global = volume_of("<global>volume=-1\n<region>sample=a.wav");
        assert_eq!(
            only_global, -1.0,
            "global applies when no inner scope sets it"
        );
    }

    #[test]
    fn master_values_drive_region_matching_and_pitch() {
        // 价值主张：`<master>` 层写 `key` 的库现在能被正确触发。
        // 改动前 `<master>` 被忽略 ⇒ lokey/hikey 保持 0/127，note 90 也会命中。
        let instrument = parse_text("<master>key=36\n<region>sample=a.wav", &Default::default())
            .expect("parses");
        let region = &instrument.regions()[0];
        assert_eq!((region.lokey, region.hikey), (36, 36));
        assert_eq!(region.pitch_keycenter, 36);
        assert!(instrument.region_for(36, 100).is_some());
        assert!(instrument.region_for(90, 100).is_none());
    }

    #[test]
    fn master_section_persists_across_group_headers() {
        // 规范示例的形状（<https://sfzformat.com/headers/master/>）：一个 `<master>`
        // 段里可以出现多个 `<group>`，master 层取值跨这些 `<group>` 保持有效。
        let instrument = parse_text(
            "<global>volume=0\n\
             <master>volume=-6\n\
             <group>key=36\n\
             <region>sample=a.wav\n\
             <region>sample=b.wav\n\
             <group>key=38\n\
             <region>sample=c.wav",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(instrument.len(), 3);
        for region in instrument.regions() {
            assert_eq!(region.volume, -6.0, "master value must survive <group>");
        }
        assert_eq!(instrument.regions()[0].lokey, 36);
        assert_eq!(instrument.regions()[1].lokey, 36);
        assert_eq!(instrument.regions()[2].lokey, 38);
    }

    #[test]
    fn a_new_master_header_resets_the_previous_group() {
        // 工程裁决：新 `<master>` 清空 `<group>`（与「新 `<global>` 清空 master+group」一致）。
        // 若不清空，第二个 master 的 region 会继承第一个 master 的 `group` 取值。
        let instrument = parse_text(
            "<master>volume=-1\n\
             <group>key=36\n\
             <region>sample=a.wav\n\
             <master>volume=-2\n\
             <region>sample=b.wav",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(instrument.len(), 2);
        assert_eq!(
            (instrument.regions()[0].lokey, instrument.regions()[0].hikey),
            (36, 36)
        );
        assert_eq!(instrument.regions()[0].volume, -1.0);
        // 第二个 region 不再看到任何 group：回到 0..=127 默认音域。
        assert_eq!(
            (instrument.regions()[1].lokey, instrument.regions()[1].hikey),
            (0, 127)
        );
        assert_eq!(instrument.regions()[1].volume, -2.0);
    }

    #[test]
    fn master_header_is_recognized_instead_of_ignored() {
        let instrument = parse_text("<master>key=36", &Default::default()).expect("parses");
        assert!(
            !instrument.warnings().iter().any(
                |warning| matches!(warning, Warning::IgnoredHeader { name, .. } if name == "master")
            ),
            "master must not be reported as an ignored header"
        );
        assert_eq!(Header::from_name("master"), Some(Header::Master));
        assert_eq!(Header::from_name("MASTER"), Some(Header::Master));
        assert_eq!(Header::Master.name(), "master");
    }

    #[test]
    fn master_cc_gates_override_global_and_yield_to_group_and_region() {
        // CC 门控走的是与普通 opcode 不同的合并代码路径，必须单独证明。
        let gate_of = |text: &str| {
            parse_text(text, &Default::default())
                .expect("parses")
                .regions()[0]
                .cc_gates
                .clone()
        };
        assert_eq!(
            gate_of(
                "<global>locc1=0 hicc1=10\n<master>locc1=20\n<group>hicc1=30\n<region>sample=a.wav"
            ),
            vec![CcGate {
                cc: 1,
                lo: 20,
                hi: 30
            }],
            "master overrides global; group overrides master"
        );
        assert_eq!(
            gate_of("<global>locc1=0\n<master>locc1=20\n<group>locc1=40\n<region>sample=a.wav"),
            vec![CcGate {
                cc: 1,
                lo: 40,
                hi: 127
            }],
            "group beats master"
        );
        assert_eq!(
            gate_of("<global>locc1=0\n<master>locc1=20\n<region>sample=a.wav locc1=60"),
            vec![CcGate {
                cc: 1,
                lo: 60,
                hi: 127
            }],
            "region beats master"
        );
    }

    #[test]
    fn region_inherits_default_path_across_a_later_control_header() {
        let instrument = parse_text(
            "<control>\ndefault_path=A/\n<region>sample=a.wav\n<control>\ndefault_path=B/\n<region>sample=b.wav\n",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(instrument.len(), 2);
        assert_eq!(instrument.regions()[0].sample_path(), "A/a.wav");
        assert_eq!(instrument.regions()[1].sample_path(), "B/b.wav");
    }

    #[test]
    fn header_may_be_followed_by_opcodes_on_the_same_line() {
        let instrument = parse_text(
            "<region> lokey=36 hikey=38 sample=a.wav",
            &Default::default(),
        )
        .expect("parses");
        let region = &instrument.regions()[0];
        assert_eq!((region.lokey, region.hikey), (36, 38));
        assert_eq!(region.sample, "a.wav");
    }

    #[test]
    fn region_without_sample_is_skipped_with_warning() {
        let instrument = parse_text("<region>key=36", &Default::default()).expect("parses");
        assert!(instrument.is_empty());
        assert!(
            instrument
                .warnings()
                .iter()
                .any(|w| matches!(w, Warning::RegionWithoutSample { .. }))
        );
    }

    #[test]
    fn cc_gates_are_merged_and_enforced() {
        let instrument = parse_text(
            "<region>sample=a.wav locc1=64 hicc1=127",
            &Default::default(),
        )
        .expect("parses");
        let region = &instrument.regions()[0];
        assert_eq!(
            region.cc_gates,
            vec![CcGate {
                cc: 1,
                lo: 64,
                hi: 127
            }]
        );
        // 未提供 CC 状态 → 严格策略下不匹配。
        assert!(instrument.region_for(60, 100).is_none());
        let cc = |number: u8| if number == 1 { 100 } else { 0 };
        assert!(
            instrument
                .region_for_with(RegionQuery::new(60, 100).with_cc(&cc))
                .is_some()
        );
        let low = |_number: u8| 0;
        assert!(
            instrument
                .region_for_with(RegionQuery::new(60, 100).with_cc(&low))
                .is_none()
        );
    }

    #[test]
    fn round_robin_selection_is_deterministic_and_cycles() {
        let text = "<group>key=36 seq_length=3\n\
                    <region>seq_position=1 sample=k1.wav\n\
                    <region>seq_position=2 sample=k2.wav\n\
                    <region>seq_position=3 sample=k3.wav";
        let instrument = parse_text(text, &Default::default()).expect("parses");
        assert_eq!(instrument.len(), 3);
        let pick = |occurrence: u64| {
            instrument
                .region_for_with(RegionQuery::new(36, 100).with_occurrence(occurrence))
                .map(|region| region.sample.to_string())
        };
        assert_eq!(pick(0).as_deref(), Some("k1.wav"));
        assert_eq!(pick(1).as_deref(), Some("k2.wav"));
        assert_eq!(pick(2).as_deref(), Some("k3.wav"));
        assert_eq!(pick(3).as_deref(), Some("k1.wav"));
    }

    #[test]
    fn keyswitch_gate_requires_state() {
        let instrument =
            parse_text("<region>sample=a.wav sw_last=36", &Default::default()).expect("parses");
        assert!(instrument.region_for(60, 100).is_none());
        let down = |_key: u8| true;
        assert!(
            instrument
                .region_for_with(RegionQuery::new(60, 100).with_keyswitch(36, &down))
                .is_some()
        );
        let down_other = |_key: u8| true;
        assert!(
            instrument
                .region_for_with(RegionQuery::new(60, 100).with_keyswitch(40, &down_other))
                .is_none()
        );
    }

    #[test]
    fn instrument_region_lookup_uses_key_range() {
        let instrument = Instrument::new(
            vec![region(60, 1, 1), region(62, 1, 1)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(instrument.region_for(60, 100).map(|r| r.lokey), Some(60));
        assert_eq!(instrument.region_for(62, 100).map(|r| r.lokey), Some(62));
        assert!(instrument.region_for(61, 100).is_none());
    }

    #[test]
    fn offset_end_and_direction_default_to_the_spec_values() {
        // 规范默认值：`offset` 0、`end` unspecified、`direction` forward。
        // 出处 <https://sfzformat.com/opcodes/offset/>、`.../end/`、`.../direction/`。
        let instrument = parse_text("<region>sample=a.wav", &Default::default()).expect("parses");
        let region = &instrument.regions()[0];
        assert_eq!(region.offset, 0);
        assert_eq!(region.end, SampleEnd::Unspecified);
        assert_eq!(region.direction, PlayDirection::Forward);
    }

    #[test]
    fn offset_end_and_direction_are_read_from_the_four_scope_chain() {
        let instrument = parse_text(
            "<global>offset=1 end=99\n\
             <master>direction=reverse\n\
             <group>offset=10\n\
             <region>sample=a.wav end=19",
            &Default::default(),
        )
        .expect("parses");
        let region = &instrument.regions()[0];
        assert_eq!(region.offset, 10, "group beats global");
        assert_eq!(region.end, SampleEnd::Inclusive(19), "region beats global");
        assert_eq!(region.direction, PlayDirection::Reverse, "master applies");
        // `direction` 的取值匹配大小写不敏感（与 `loop_mode` 同一条 `as_option` 路径）。
        let upper = parse_text(
            "<region>sample=a.wav direction=REVERSE",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(upper.regions()[0].direction, PlayDirection::Reverse);
    }

    #[test]
    fn end_minus_one_keeps_a_silent_region_that_still_triggers() {
        // 规范：`end=-1` 时采样不发声，但 region「is still triggered」，
        // 于是可以用 `group` / `off_by` 关掉别的 region。
        // <https://sfzformat.com/opcodes/end/>
        let instrument = parse_text(
            "<region>sample=silence.wav end=-1 group=3 off_by=4",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(instrument.len(), 1, "silent region must not be dropped");
        let region = &instrument.regions()[0];
        assert_eq!(region.end, SampleEnd::Silent);
        assert_eq!((region.group, region.off_by), (3, 4));
        assert!(
            instrument.region_for(60, 100).is_some(),
            "a silent region still triggers"
        );
    }

    #[test]
    fn out_of_range_offset_end_and_direction_are_err_not_panic() {
        // `end` 的文档化取值域是 `-1`（哨兵）加 `0..=4294967295`。
        assert!(matches!(
            parse_text("<region>sample=a.wav end=-2", &Default::default()),
            Err(SfzError::IntegerOutOfRange { .. })
        ));
        assert!(matches!(
            parse_text(
                "<region>sample=a.wav offset=4294967296",
                &Default::default()
            ),
            Err(SfzError::IntegerOutOfRange { .. })
        ));
        assert!(matches!(
            parse_text("<region>sample=a.wav end=abc", &Default::default()),
            Err(SfzError::InvalidInteger { .. })
        ));
        assert!(matches!(
            parse_text(
                "<region>sample=a.wav direction=sideways",
                &Default::default()
            ),
            Err(SfzError::InvalidOption { .. })
        ));
        // 上界本身必须被接受（规范 Range `0 to 4294967296`）。
        let max = parse_text(
            "<region>sample=a.wav offset=4294967295 end=4294967295",
            &Default::default(),
        )
        .expect("the documented upper bound is in range");
        assert_eq!(max.regions()[0].offset, u32::MAX);
        assert_eq!(max.regions()[0].end, SampleEnd::Inclusive(u32::MAX));
    }

    // -----------------------------------------------------------------------
    // `trigger`（<https://sfzformat.com/opcodes/trigger/>）
    // -----------------------------------------------------------------------

    #[test]
    fn trigger_defaults_to_attack_and_accepts_the_five_documented_values() {
        // 规范表：默认 `attack`，Options `attack, release, first, legato`（SFZ v1）
        // 加 `release_key`（SFZ v2）。五个取值逐一读回。
        let default = parse_text("<region>sample=a.wav", &Default::default()).expect("parses");
        assert_eq!(default.regions()[0].trigger, Trigger::Attack);

        for (text, expected) in [
            ("attack", Trigger::Attack),
            ("release", Trigger::Release),
            ("release_key", Trigger::ReleaseKey),
            ("first", Trigger::First),
            ("legato", Trigger::Legato),
            // 取值匹配大小写不敏感（与 `loop_mode` 同一条 `as_option` 路径）。
            ("RELEASE", Trigger::Release),
        ] {
            let source = format!("<region>sample=a.wav trigger={text}");
            let instrument = parse_text(&source, &Default::default()).expect("parses");
            assert_eq!(instrument.regions()[0].trigger, expected, "trigger={text}");
        }

        assert!(matches!(
            parse_text("<region>sample=a.wav trigger=sustain", &Default::default()),
            Err(SfzError::InvalidOption { .. })
        ));
    }

    #[test]
    fn trigger_is_read_from_the_four_scope_chain() {
        // 与其它 opcode 同一条 `region → group → master → global` 查找链：
        // region 覆盖 group，group 覆盖 master。
        let inherited = parse_text(
            "<master>trigger=release\n<region>sample=a.wav",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(inherited.regions()[0].trigger, Trigger::Release);

        let overridden = parse_text(
            "<global>trigger=release\n<group>trigger=legato\n\
             <region>sample=a.wav trigger=first",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(overridden.regions()[0].trigger, Trigger::First);
    }

    // -----------------------------------------------------------------------
    // `off_mode`（<https://sfzformat.com/opcodes/off_mode/>）
    // -----------------------------------------------------------------------

    #[test]
    fn off_mode_defaults_to_fast_and_accepts_the_three_corpus_values() {
        // 三个取值与缺省 `fast` 的出处是登记语料普查（`docs/ledger/sfz-core-notes.md`
        // 第 11 节）：normal 818 / fast 87 / time 4，合计 909 次，无集合外取值。
        let default = parse_text("<region>sample=a.wav", &Default::default()).expect("parses");
        assert_eq!(default.regions()[0].off_mode, OffMode::Fast);

        for (text, expected) in [
            ("fast", OffMode::Fast),
            ("normal", OffMode::Normal),
            ("time", OffMode::Time),
            // 取值匹配大小写不敏感（与 `trigger` / `loop_mode` 同一条 `as_option` 路径）。
            ("NORMAL", OffMode::Normal),
        ] {
            let source = format!("<region>sample=a.wav off_mode={text}");
            let instrument = parse_text(&source, &Default::default()).expect("parses");
            assert_eq!(
                instrument.regions()[0].off_mode,
                expected,
                "off_mode={text}"
            );
        }
    }

    #[test]
    fn an_out_of_set_off_mode_is_an_error_not_a_silent_default() {
        // 改动前：`off_mode` 根本不读，任意字面量都被静默忽略。
        // 改动后：集合外取值走 `as_option`，返回明确的 `Err` —— 不可信输入不许静默降级。
        for bad in ["slow", "0", "fastest"] {
            let source = format!("<region>sample=a.wav off_mode={bad}");
            assert!(
                matches!(
                    parse_text(&source, &Default::default()),
                    Err(SfzError::InvalidOption { .. })
                ),
                "off_mode={bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn off_mode_is_read_from_the_four_scope_chain() {
        // 与其它 opcode 同一条 `region → group → master → global` 查找链。
        let inherited = parse_text(
            "<master>off_mode=normal\n<region>sample=a.wav",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(inherited.regions()[0].off_mode, OffMode::Normal);

        let overridden = parse_text(
            "<global>off_mode=normal\n<group>off_mode=time\n\
             <region>sample=a.wav off_mode=fast",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(overridden.regions()[0].off_mode, OffMode::Fast);
    }

    #[test]
    fn off_mode_does_not_gate_region_selection() {
        // `off_mode` 只描述 note-off 之后怎么结束，**不**参与 region 选择：
        // 三个取值下 note-on 都选中同一个 region。
        for text in ["fast", "normal", "time"] {
            let source = format!("<region>sample=a.wav off_mode={text}");
            let instrument = parse_text(&source, &Default::default()).expect("parses");
            assert_eq!(instrument.len(), 1, "off_mode={text}");
            assert!(
                instrument.region_for(60, 100).is_some(),
                "off_mode={text} must not gate region selection"
            );
        }
    }

    #[test]
    fn off_mode_is_independent_of_the_one_shot_loop_override() {
        // `trigger=release` 把 `loop_mode` 覆盖成 `one_shot`；`off_mode` 是另一个 opcode，
        // 保持自己的取值（`one_shot` 说「不理会 note-off」，`off_mode` 说「note-off 后怎么结束」）。
        let instrument = parse_text(
            "<region>sample=a.wav trigger=release off_mode=normal",
            &Default::default(),
        )
        .expect("parses");
        let region = &instrument.regions()[0];
        assert_eq!(region.effective_loop_mode(), LoopMode::OneShot);
        assert_eq!(region.off_mode, OffMode::Normal);
    }

    // -----------------------------------------------------------------------
    // `off_time`（<https://sfzformat.com/opcodes/off_time/>）
    // -----------------------------------------------------------------------
    //
    // 观测方式：`assert_eq!` 直接比较 f32 值，**不**用 `abs() < eps` 容差比较 ——
    // 判目标字面量就是 f32 字面量本身，不需要放宽。`off_time` 只有两条产生路径
    // （解析期十进制字面量取最近 f32、缺省常量），没有超越函数参与，逐位确定。

    #[test]
    fn off_time_defaults_to_the_spec_value_and_is_not_folded_into_a_zero() {
        // 规范表格 Default = 0.006 秒（<https://sfzformat.com/opcodes/off_time/>）。
        // 缺省必须是「未给出」（`None`）+ 生效值 0.006，**不是** 0.0：
        // 0.0 会让 `off_mode=time` 变成"立刻切断"，与 fast 混淆。
        let default = parse_text("<region>sample=a.wav", &Default::default()).expect("parses");
        let region = &default.regions()[0];
        assert_eq!(region.off_time, None);
        assert_eq!(region.effective_off_time(), 0.006);
        assert_ne!(region.effective_off_time(), 0.0);

        // 扫描语料实测（1398 个登记 `.sfz` 里 `off_time=` 的取值分布，
        // 重复次数：0.05×8、0.5×2、0.25×2、0.2×2、0.4×1、0.3×1）都落在这个回退之上：
        // 没有一个取值是 0，因此把缺省读成 0.0 会让整批 region 静默变成立刻切断。
        let explicit =
            parse_text("<region>sample=a.wav off_time=0.0", &Default::default()).expect("parses");
        assert_eq!(explicit.regions()[0].off_time, Some(0.0));
        assert_eq!(
            explicit.regions()[0].effective_off_time(),
            0.0,
            "an explicit 0 must survive as 0 (it is not the same request as `unspecified`)"
        );
    }

    #[test]
    fn off_time_reads_every_distinct_value_found_in_the_registered_corpus() {
        // 出处：对 `git ls-files` 的 1398 个登记 `.sfz` 逐文件取
        // `off_time=<literal>`（探针命令见本票报告）；不同取值共 6 个，这里是全部 6 个。
        for text in ["0.05", "0.5", "0.25", "0.2", "0.4", "0.3"] {
            let source = format!("<region>sample=a.wav off_mode=time off_time={text}");
            let instrument = parse_text(&source, &Default::default()).expect("parses");
            let region = &instrument.regions()[0];
            let expected: f32 = text.parse().expect("literal parses as f32");
            assert_eq!(region.off_time, Some(expected), "off_time={text}");
            assert_eq!(region.effective_off_time(), expected, "off_time={text}");
        }
    }

    #[test]
    fn off_time_is_read_from_the_four_scope_chain() {
        // 与其它 opcode 同一条 `region → group → master → global` 查找链。
        let inherited = parse_text(
            "<master>off_time=0.25\n<region>sample=a.wav",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(inherited.regions()[0].off_time, Some(0.25));

        let overridden = parse_text(
            "<global>off_time=0.4\n<group>off_time=0.3\n\
             <region>sample=a.wav off_time=0.05",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(overridden.regions()[0].off_time, Some(0.05));
    }

    #[test]
    fn a_negative_off_time_is_an_error_not_a_silent_zero() {
        // 规范表格的 Range 为空，但时长取反没有定义：明确 `Err`，不静默钳位成 0。
        assert!(
            matches!(
                parse_text("<region>sample=a.wav off_time=-0.5", &Default::default()),
                Err(SfzError::InvalidDuration { value: -0.5, .. })
            ),
            "a negative off_time must be rejected"
        );
    }

    #[test]
    fn a_non_finite_or_non_numeric_off_time_is_an_error() {
        for bad in ["nan", "inf", "-inf", "abc", ""] {
            let source = format!("<region>sample=a.wav off_time={bad}");
            let outcome = parse_text(&source, &Default::default());
            assert!(
                matches!(
                    outcome,
                    Err(SfzError::NonFiniteFloat { .. } | SfzError::InvalidFloat { .. })
                ),
                "off_time={bad:?} must be an explicit error, got {outcome:?}"
            );
        }
    }

    #[test]
    fn bend_up_and_bend_down_default_to_the_spec_values() {
        // 规范表格（<https://sfzformat.com/opcodes/bend_up/>、
        // <https://sfzformat.com/opcodes/bend_down/>）的 Default 列分别是 200 与 -200。
        // 来源是字段声明，因此两个缺省必须**成对**出现在每一个没有写这两个 opcode 的 region 上。
        let default = parse_text("<region>sample=a.wav", &Default::default()).expect("parses");
        let region = &default.regions()[0];
        assert_eq!(region.bend_up, 200);
        assert_eq!(region.bend_down, -200);
        assert_eq!(region.bend_up, BEND_UP_DEFAULT_CENTS);
        assert_eq!(region.bend_down, BEND_DOWN_DEFAULT_CENTS);

        // 显式 0 是「弯音轮不改变音高」，与缺省 200 / -200 不是一回事：
        // 缺省轮子推到底会弯 200 音分，显式 0 则一个音分都不弯。
        let zero = parse_text(
            "<region>sample=a.wav bend_up=0 bend_down=0",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(zero.regions()[0].bend_up, 0);
        assert_eq!(zero.regions()[0].bend_down, 0);
        assert_eq!(zero.regions()[0].bend_cents(127), 0);
        assert_eq!(zero.regions()[0].bend_cents(0), 0);
    }

    #[test]
    fn bend_opcodes_read_every_distinct_value_found_in_the_registered_corpus() {
        // 出处：对 `git ls-files` 引入的 1398 个登记 `.sfz` 逐文件剥掉 `//` 注释后取
        // `bend_up=<literal>` / `bend_down=<literal>`（探针命令见本票报告）。
        // 两个 opcode 各出现 708 次（合计 1416 次赋值，71 个文件），不同取值共 13 个，
        // 这里是全部 13 个。注意 `bend_up=0` / `bend_down=0` 与 `bend_down=1200` 都在其中：
        // 前者要求「显式 0 ≠ 缺省」，后者要求「`bend_down` 不强制非正」。
        for (opcode, literals) in [
            ("bend_up", &["0", "300", "500", "1200", "2400"][..]),
            (
                "bend_down",
                &[
                    "-3600", "-2400", "-1200", "-700", "-500", "-400", "0", "1200",
                ][..],
            ),
        ] {
            for literal in literals {
                let source = format!("<region>sample=a.wav {opcode}={literal}");
                let instrument = parse_text(&source, &Default::default()).expect("parses");
                let region = &instrument.regions()[0];
                let expected: i32 = literal.parse().expect("literal parses as i32");
                let parsed = if opcode == "bend_up" {
                    region.bend_up
                } else {
                    region.bend_down
                };
                assert_eq!(parsed, expected, "{opcode}={literal}");
            }
        }

        // 语料里两个 opcode 的极值（1200 / -3600）都落在规范 Range ±9600 之内，
        // 因此上面那条循环**不会**因为越界而失败 —— 这一点是测量结论，不是假设。
        let extremes = parse_text(
            "<region>sample=a.wav bend_up=2400 bend_down=-3600",
            &Default::default(),
        )
        .expect("parses");
        let region = &extremes.regions()[0];
        assert!(region.bend_up.abs() <= BEND_RANGE_MAX_CENTS);
        assert!(region.bend_down.abs() <= BEND_RANGE_MAX_CENTS);
        assert_eq!(region.bend_up, 2400);
        assert_eq!(region.bend_down, -3600);
    }

    #[test]
    fn bend_opcodes_are_read_from_the_four_scope_chain() {
        // 与其它 opcode 同一条 `region → group → master → global` 查找链。
        // 语料实测：1416 次赋值里 58 次在 `<global>`、647 次在 `<group>`、1 次在 `<master>`，
        // 因此这条链的每一级都必须能带出取值。
        let inherited = parse_text(
            "<global>bend_up=1200\n<master>bend_down=1200\n<region>sample=a.wav",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(inherited.regions()[0].bend_up, 1200);
        assert_eq!(inherited.regions()[0].bend_down, 1200);

        let by_group = parse_text(
            "<group>bend_up=500\n<region>sample=a.wav",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(by_group.regions()[0].bend_up, 500);

        let overridden = parse_text(
            "<global>bend_up=300\n<group>bend_up=500\n<region>sample=a.wav bend_up=1200",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(overridden.regions()[0].bend_up, 1200);
    }

    #[test]
    fn an_out_of_range_bend_range_is_an_error_not_a_silent_clamp() {
        // 两个格式页的表格 Range 都是 -9600 to 9600。越界是明确 `Err`。
        for source in [
            "<region>sample=a.wav bend_up=9601",
            "<region>sample=a.wav bend_up=-9601",
            "<region>sample=a.wav bend_down=9601",
            "<region>sample=a.wav bend_down=-9601",
        ] {
            let outcome = parse_text(source, &Default::default());
            assert!(
                matches!(
                    outcome,
                    Err(SfzError::IntegerOutOfRange {
                        min: -9600,
                        max: 9600,
                        ..
                    })
                ),
                "{source} must be rejected, got {outcome:?}"
            );
        }
        // 两个端点本身是合法取值（Range 是闭区间）。
        let edges = parse_text(
            "<region>sample=a.wav bend_up=9600 bend_down=-9600",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(edges.regions()[0].bend_up, 9600);
        assert_eq!(edges.regions()[0].bend_down, -9600);
    }

    #[test]
    fn a_non_integer_bend_range_is_an_error() {
        for bad in ["", "abc", "12.5", "1e3"] {
            for opcode in ["bend_up", "bend_down"] {
                let source = format!("<region>sample=a.wav {opcode}={bad}");
                let outcome = parse_text(&source, &Default::default());
                assert!(
                    matches!(outcome, Err(SfzError::InvalidInteger { .. })),
                    "{opcode}={bad:?} must be an explicit error, got {outcome:?}"
                );
            }
        }
    }

    #[test]
    fn bend_opcodes_do_not_change_region_selection_or_the_static_pitch_ratio() {
        // 弯音范围**不**参与 region 选择，也不改写 `pitch_ratio` / `transpose` / `tune`：
        // 中位轮值下的音高比必须和没有这两个 opcode 时逐位相同。
        let plain = parse_text(
            "<region>sample=a.wav pitch_keycenter=60 transpose=2 tune=50",
            &Default::default(),
        )
        .expect("parses");
        let bent = parse_text(
            "<region>sample=a.wav pitch_keycenter=60 transpose=2 tune=50 \
             bend_up=1200 bend_down=-1200",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(bent.len(), 1);
        assert_eq!(
            plain.regions()[0].pitch_ratio(36),
            bent.regions()[0].pitch_ratio(36)
        );
        assert_eq!(
            bent.regions()[0].bend_ratio(36, PITCH_BEND_CENTER),
            plain.regions()[0].pitch_ratio(36)
        );
        assert!(
            bent.region_for(60, 100).is_some(),
            "a bend range must not gate region selection"
        );
    }

    #[test]
    fn off_time_does_not_change_region_selection_or_the_loop_override() {
        // `off_time` 只喂给关断时长，**不**参与 region 选择，也不碰 `loop_mode`。
        let source = "<region>sample=a.wav off_mode=time off_time=0.25 trigger=release";
        let instrument = parse_text(source, &Default::default()).expect("parses");
        let region = &instrument.regions()[0];
        assert_eq!(region.off_time, Some(0.25));
        assert_eq!(region.effective_loop_mode(), LoopMode::OneShot);
        assert!(
            instrument.region_for(60, 100).is_none(),
            "a trigger=release region must stay out of note-on selection"
        );
    }

    #[test]
    fn a_release_region_is_not_selected_by_a_note_on() {
        // 缺口修复判据：改动前 `trigger=release` 的 region 会在 note-on 被选中
        // （把 release 采样当成 attack 采样播放）。改动后 note-on 永不选中它。
        let instrument = parse_text(
            "<region>sample=release.wav trigger=release",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(instrument.len(), 1, "the region is still parsed");
        assert!(
            instrument.region_for(60, 100).is_none(),
            "a trigger=release region must not be selected by a note-on"
        );
        // 它仍然由 note-off（踏板松开）选中。
        let picked = instrument
            .region_for_with(RegionQuery::note_off(60, 100, false))
            .expect("note-off selects the release region");
        assert_eq!(picked.sample, "release.wav");
    }

    #[test]
    fn release_and_release_key_differ_on_the_sustain_pedal() {
        // 规范：「`release` will play on note-off **or** sustain pedal off」，
        // 「`release_key` will play on note-off. Ignores sustain pedal.」
        let release = parse_text("<region>sample=r.wav trigger=release", &Default::default())
            .expect("parses");
        assert!(
            release
                .region_for_with(RegionQuery::note_off(60, 100, false))
                .is_some(),
            "release plays when the pedal is up"
        );
        assert!(
            release
                .region_for_with(RegionQuery::note_off(60, 100, true))
                .is_none(),
            "release waits for the pedal to come up"
        );

        let key = parse_text(
            "<region>sample=k.wav trigger=release_key",
            &Default::default(),
        )
        .expect("parses");
        assert!(
            key.region_for_with(RegionQuery::note_off(60, 100, true))
                .is_some(),
            "release_key ignores the pedal"
        );
        assert!(
            key.region_for_with(RegionQuery::note_off(60, 100, false))
                .is_some(),
            "release_key also plays with the pedal up"
        );
        assert!(
            key.region_for(60, 100).is_none(),
            "release_key is still not a note-on trigger"
        );
    }

    #[test]
    fn attack_regions_are_not_selected_by_a_note_off() {
        // 反向门控：note-off 事件永不选中 note-on 家族的 region。
        let instrument = parse_text(
            "<region>sample=a.wav\n<region>sample=b.wav trigger=first\n\
             <region>sample=c.wav trigger=legato",
            &Default::default(),
        )
        .expect("parses");
        for pedal_down in [false, true] {
            assert!(
                instrument
                    .region_for_with(RegionQuery::note_off(60, 100, pedal_down))
                    .is_none(),
                "note-off must not select note-on regions (pedal_down={pedal_down})"
            );
        }
    }

    #[test]
    fn a_mixed_instrument_switches_region_family_with_the_event() {
        let instrument = parse_text(
            "<region>sample=attack.wav trigger=attack\n\
             <region>sample=release.wav trigger=release",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(
            instrument.region_for(60, 100).map(|r| r.sample.as_ref()),
            Some("attack.wav")
        );
        assert_eq!(
            instrument
                .region_for_with(RegionQuery::note_off(60, 100, false))
                .map(|r| r.sample.as_ref()),
            Some("release.wav")
        );
    }

    #[test]
    fn first_and_legato_require_the_held_note_count() {
        // `first`：note-on 且没有**其它**音符按着；`legato`：note-on 且有其它音符按着。
        // 严格策略：调用方不给按住数 ⇒ 两者都不匹配（不猜「大概没有别的音」）。
        let first =
            parse_text("<region>sample=f.wav trigger=first", &Default::default()).expect("parses");
        assert!(first.region_for(60, 100).is_none(), "state not provided");
        assert!(
            first
                .region_for_with(RegionQuery::new(60, 100).with_held_notes(0))
                .is_some(),
            "first plays when no other note is held"
        );
        assert!(
            first
                .region_for_with(RegionQuery::new(60, 100).with_held_notes(1))
                .is_none(),
            "first must not play inside a legato phrase"
        );

        let legato =
            parse_text("<region>sample=l.wav trigger=legato", &Default::default()).expect("parses");
        assert!(legato.region_for(60, 100).is_none(), "state not provided");
        assert!(
            legato
                .region_for_with(RegionQuery::new(60, 100).with_held_notes(1))
                .is_some(),
            "legato plays when another note is held"
        );
        assert!(
            legato
                .region_for_with(RegionQuery::new(60, 100).with_held_notes(0))
                .is_none(),
            "legato must not play on the first note"
        );

        assert!(first.regions()[0].has_held_notes_gate());
        assert!(
            !parse_text("<region>sample=a.wav", &Default::default())
                .unwrap()
                .regions()[0]
                .has_held_notes_gate()
        );
    }

    #[test]
    fn key_minus_one_release_regions_never_trigger() {
        // `key=-1`（`trigger_by_note == false`）与 `trigger` 是两道独立的门：
        // 两道都过才播放。
        let instrument = parse_text(
            "<region>key=-1 sample=r.wav trigger=release",
            &Default::default(),
        )
        .expect("parses");
        assert!(!instrument.regions()[0].trigger_by_note);
        assert!(
            instrument
                .region_for_with(RegionQuery::note_off(60, 100, false))
                .is_none(),
            "key=-1 blocks note-off triggering too"
        );
    }

    #[test]
    fn release_regions_force_one_shot_loop_mode() {
        // 规范：「Setting trigger to release or release_key will cause the region to
        // play as if `loop_mode` was set to one_shot」。
        let instrument = parse_text(
            "<region>sample=r.wav trigger=release loop_mode=loop_continuous \
             loop_start=10 loop_end=20",
            &Default::default(),
        )
        .expect("parses");
        let region = &instrument.regions()[0];
        assert_eq!(
            region.loop_mode,
            LoopMode::LoopContinuous,
            "stored as written"
        );
        assert_eq!(
            region.effective_loop_mode(),
            LoopMode::OneShot,
            "overridden"
        );
        assert_eq!(region.loop_window(), None, "one_shot does not loop");

        let attack = parse_text(
            "<region>sample=a.wav loop_mode=loop_continuous loop_start=10 loop_end=20",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(
            attack.regions()[0].effective_loop_mode(),
            LoopMode::LoopContinuous
        );
        assert!(attack.regions()[0].loop_window().is_some());
    }

    #[test]
    fn trigger_event_matching_table_is_total() {
        // 事件口径逐条固定（4 条规则），避免以后被"顺手"改宽。
        let cases = [
            (Trigger::Attack, TriggerEvent::NoteOn, true),
            (
                Trigger::Attack,
                TriggerEvent::NoteOff { pedal_down: false },
                false,
            ),
            (Trigger::Release, TriggerEvent::NoteOn, false),
            (
                Trigger::Release,
                TriggerEvent::NoteOff { pedal_down: false },
                true,
            ),
            (
                Trigger::Release,
                TriggerEvent::NoteOff { pedal_down: true },
                false,
            ),
            (Trigger::ReleaseKey, TriggerEvent::NoteOn, false),
            (
                Trigger::ReleaseKey,
                TriggerEvent::NoteOff { pedal_down: false },
                true,
            ),
            (
                Trigger::ReleaseKey,
                TriggerEvent::NoteOff { pedal_down: true },
                true,
            ),
            (Trigger::First, TriggerEvent::NoteOn, true),
            (
                Trigger::First,
                TriggerEvent::NoteOff { pedal_down: false },
                false,
            ),
            (Trigger::Legato, TriggerEvent::NoteOn, true),
            (
                Trigger::Legato,
                TriggerEvent::NoteOff { pedal_down: true },
                false,
            ),
        ];
        for (trigger, event, expected) in cases {
            assert_eq!(
                trigger.responds_to(event),
                expected,
                "{trigger:?} vs {event:?}"
            );
        }
        assert!(Trigger::Release.is_release_family());
        assert!(Trigger::ReleaseKey.is_release_family());
        assert!(!Trigger::Attack.is_release_family());
        assert!(!Trigger::First.is_release_family());
        assert!(!Trigger::Legato.is_release_family());
    }

    // ------------------------------------------------------------------
    // `<curve>` 头（规范 <https://sfzformat.com/headers/curve/>）
    // ------------------------------------------------------------------

    /// 登记语料里的真实曲线块（`assets/samples/aliexpress-erhu/…/curves.sfz` 的形状）。
    const CORPUS_CURVES: &str = "\
<curve>curve_index=7
v000=0
v095=1
v127=1
<curve>curve_index=8
v000=0
v095=0.5
v127=1
<region>sample=a.wav";

    #[test]
    fn curve_blocks_are_reduced_in_file_order_with_the_spec_defaults() {
        let instrument = parse_text(CORPUS_CURVES, &Default::default()).expect("parses");
        assert_eq!(instrument.len(), 1, "the region after the curves survives");
        let indices: Vec<u8> = instrument.curves().iter().map(Curve::index).collect();
        assert_eq!(indices, vec![7, 8], "file order, deterministic");
        assert_eq!(
            instrument.curve(7).map(Curve::points),
            Some(
                &[
                    CurvePoint { at: 0, value: 0.0 },
                    CurvePoint { at: 95, value: 1.0 },
                    CurvePoint {
                        at: 127,
                        value: 1.0
                    },
                ][..]
            )
        );
        // 未显式给出的 v000 / v127 用规范缺省补齐（0 与 1）。
        let sparse =
            parse_text("<curve>curve_index=9\nv064=0.25", &Default::default()).expect("parses");
        assert_eq!(
            sparse.curve(9).map(Curve::points),
            Some(
                &[
                    CurvePoint { at: 0, value: 0.0 },
                    CurvePoint {
                        at: 64,
                        value: 0.25
                    },
                    CurvePoint {
                        at: 127,
                        value: 1.0
                    },
                ][..]
            )
        );
        // 未定义的编号：既不是文件里的，也不是 0..=3 内建 ⇒ None。
        assert_eq!(instrument.curve_value_at(11, 64.0), None);
        assert_eq!(instrument.curve(9), None);
    }

    #[test]
    fn curve_values_are_not_clamped_and_keep_their_sign() {
        // 内建 `Bipolar` 是 -1..1，语料里也曾出现 v000=-1（例如 `docs/ledger` 第 11 节的
        // 普查口径），因此取值不钳位。
        let instrument = parse_text("<curve>curve_index=7\nv000=-1\nv127=1", &Default::default())
            .expect("parses");
        let curve = instrument.curve(7).expect("defined");
        assert_eq!(curve.value_at(0.0), -1.0);
        assert_eq!(curve.value_at(63.5), 0.0);
        assert_eq!(curve.value_at(127.0), 1.0);
    }

    #[test]
    fn curve_opcodes_do_not_leak_into_the_inheritance_chain() {
        // `<curve>` 是定义段：段内 opcode 不得写进 group/global，也不得变成 region。
        let instrument = parse_text(
            "<group>key=36 volume=-3\n\
             <region>sample=a.wav\n\
             <curve>curve_index=7\nsample=b.wav\nvolume=99\nkey=48\nv000=0\n\
             <region>sample=c.wav",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(instrument.len(), 2, "only two real regions");
        assert_eq!(instrument.regions()[0].sample, "a.wav");
        assert_eq!(instrument.regions()[1].sample, "c.wav");
        for region in instrument.regions() {
            assert_eq!(
                region.volume, -3.0,
                "the curve block must not change volume"
            );
            assert_eq!((region.lokey, region.hikey), (36, 36));
        }
        assert_eq!(instrument.curves().len(), 1);
        assert_eq!(
            instrument
                .curve(7)
                .map(Curve::points)
                .map(<[CurvePoint]>::len),
            Some(2)
        );
    }

    #[test]
    fn a_curve_block_without_points_or_index_is_accepted_and_empty() {
        let instrument = parse_text("<curve>", &Default::default()).expect("parses");
        assert!(instrument.curves().is_empty());
        assert!(instrument.warnings().is_empty(), "nothing was dropped");
        // 只有 `curve_index` 没有点：规范缺省就是 0 → 1 的直线。
        let identity = parse_text("<curve>curve_index=7", &Default::default()).expect("parses");
        assert_eq!(identity.curve_value_at(7, 63.5), Some(0.5));
    }

    #[test]
    fn a_curve_block_with_points_but_no_index_is_an_error() {
        let error = parse_text("<curve>\nv000=0", &Default::default())
            .expect_err("points without curve_index are ambiguous");
        assert!(
            matches!(error, SfzError::CurveWithoutIndex { line: 1 }),
            "unexpected verdict: {error:?}"
        );
    }

    #[test]
    fn a_reserved_built_in_curve_index_is_an_error_not_a_silent_overwrite() {
        // 规范原文："These cannot be overwritten. Use `curve_index` numbers of 7 and above
        // for custom curves." ⇒ 0..=6 一律 Err。
        for index in 0u8..=6 {
            let source = format!("<curve>curve_index={index}\nv000=0\nv127=1");
            let error = parse_text(&source, &Default::default()).expect_err("reserved index");
            assert!(
                matches!(error, SfzError::ReservedCurveIndex { line: 1, index: got } if got == index),
                "unexpected verdict for {index}: {error:?}"
            );
        }
        // 7 与 254 是合法自定义编号；255 越界成显式 Err（不是 panic）。
        for index in [7u8, 254] {
            let source = format!("<curve>curve_index={index}\nv000=0\nv127=1");
            let instrument = parse_text(&source, &Default::default()).expect("custom index");
            assert_eq!(instrument.curve(index).map(Curve::index), Some(index));
        }
        let error = parse_text("<curve>curve_index=255\nv000=0", &Default::default())
            .expect_err("255 is above the ARIA ceiling");
        assert!(
            matches!(
                error,
                SfzError::IntegerOutOfRange {
                    line: 1,
                    value: 255,
                    min: 0,
                    max: 254,
                    ..
                }
            ),
            "unexpected verdict: {error:?}"
        );
    }

    #[test]
    fn a_duplicate_curve_index_is_an_error_not_a_guess() {
        let error = parse_text(
            "<curve>curve_index=7\nv000=0\n<curve>curve_index=7\nv000=1",
            &Default::default(),
        )
        .expect_err("duplicate index has no normative resolution");
        assert!(
            matches!(error, SfzError::DuplicateCurveIndex { line: 3, index: 7 }),
            "unexpected verdict: {error:?}"
        );
    }

    #[test]
    fn a_curve_point_above_v127_is_an_error_and_a_malformed_point_is_ignored() {
        let error = parse_text("<curve>curve_index=7\nv128=0", &Default::default())
            .expect_err("v128 is outside v000..=v127");
        assert!(
            matches!(
                error,
                SfzError::IntegerOutOfRange {
                    line: 1,
                    value: 128,
                    min: 0,
                    max: 127,
                    ..
                }
            ),
            "unexpected verdict: {error:?}"
        );
        // 不是 `vNNN` 形态的名字（`v5` / `v0000` / 未知 opcode）与全文件口径一致地忽略。
        let instrument = parse_text(
            "<curve>curve_index=7\nv5=0.5\nv0000=0.5\nv096=0.5",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(
            instrument.curve(7).map(Curve::points),
            Some(
                &[
                    CurvePoint { at: 0, value: 0.0 },
                    CurvePoint { at: 96, value: 0.5 },
                    CurvePoint {
                        at: 127,
                        value: 1.0
                    },
                ][..]
            )
        );
    }

    #[test]
    fn a_repeated_point_in_one_block_keeps_the_last_value() {
        // 与其它作用域「后者覆盖」的 `BTreeMap` 口径一致，也保证点表没有重复 `at`
        // （重复 `at` 会让插值分母为 0）。
        let instrument = parse_text(
            "<curve>curve_index=7\nv064=1\nv064=0.25",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(instrument.curve_value_at(7, 64.0), Some(0.25));
    }

    #[test]
    fn a_non_finite_curve_value_is_an_error() {
        for bad in ["nan", "inf", "-inf", "1e999"] {
            let source = format!("<curve>curve_index=7\nv000={bad}");
            let error = parse_text(&source, &Default::default()).expect_err("non-finite value");
            assert!(
                matches!(
                    error,
                    SfzError::NonFiniteFloat { .. } | SfzError::InvalidFloat { .. }
                ),
                "unexpected verdict for {bad}: {error:?}"
            );
        }
    }

    #[test]
    fn too_many_curves_hits_the_explicit_limit() {
        let limits = ParseLimits {
            max_curves: 1,
            ..ParseLimits::default()
        };
        let error = parse_text(
            "<curve>curve_index=7\nv000=0\n<curve>curve_index=8\nv000=0",
            &limits,
        )
        .expect_err("second curve exceeds max_curves");
        assert!(
            matches!(error, SfzError::TooManyCurves { limit: 1 }),
            "unexpected verdict: {error:?}"
        );
    }

    #[test]
    fn too_many_opcodes_in_a_curve_block_hits_the_explicit_limit() {
        let limits = ParseLimits {
            max_opcodes_per_header: 2,
            ..ParseLimits::default()
        };
        let error = parse_text("<curve>curve_index=7\nv000=0\nv001=0", &limits)
            .expect_err("third opcode exceeds the per-header limit");
        assert!(
            matches!(
                error,
                SfzError::TooManyOpcodes {
                    scope: "curve",
                    limit: 2
                }
            ),
            "unexpected verdict: {error:?}"
        );
    }

    #[test]
    fn curve_lookup_prefers_file_data_over_the_built_in_table() {
        // 内建曲线 1 是 -1 → 1；文件里的 40 是 0 → 1（语料里 `*_curveccN=40` 出现过 76 次）。
        let instrument = parse_text("<curve>curve_index=40\nv000=0\nv127=1", &Default::default())
            .expect("parses");
        assert_eq!(instrument.curve_value_at(40, 0.0), Some(0.0));
        assert_eq!(instrument.curve_value_at(1, 0.0), Some(-1.0), "built-in 1");
        assert_eq!(instrument.curve_value_at(1, 127.0), Some(1.0));
        assert_eq!(
            instrument.curve_value_at(5, 0.0),
            None,
            "4..=6 have no formula"
        );
        assert_eq!(
            instrument.curve_value_at(200, 0.0),
            None,
            "undefined and not built in"
        );
    }

    #[test]
    fn curve_reduction_is_deterministic_across_independent_parses() {
        let limits: ParseLimits = Default::default();
        let first = parse_text(CORPUS_CURVES, &limits).expect("parses");
        let second = parse_text(CORPUS_CURVES, &limits).expect("parses");
        assert_eq!(first.curves(), second.curves());
        let samples: Vec<f32> = (0..=127)
            .map(|x| {
                first
                    .curve_value_at(8, f32::from(x as u8))
                    .expect("curve 8")
            })
            .collect();
        let again: Vec<f32> = (0..=127)
            .map(|x| {
                second
                    .curve_value_at(8, f32::from(x as u8))
                    .expect("curve 8")
            })
            .collect();
        assert_eq!(samples, again);
    }

    // ------------------------------------------------------------------
    // `<effect>` 头（规范 <https://sfzformat.com/headers/effect/>）
    // ------------------------------------------------------------------

    /// 登记语料里的真实 `<effect>` 形状（`assets/samples/karoryfer-big-rusty-drums/Programs/`
    /// 下 8 个**可解析**文件里的那两行；`param_offset` + ARIA MDA `type`）。
    const CORPUS_EFFECT: &str = "\
<effect>
param_offset=400
type=com.mda.Limiter

<region>sample=a.wav";

    #[test]
    fn corpus_effect_block_is_reduced_with_the_specification_defaults() {
        let instrument = parse_text(CORPUS_EFFECT, &Default::default()).expect("parses");
        assert_eq!(instrument.len(), 1, "the region after the effect survives");
        let effects = instrument.effects();
        assert_eq!(effects.len(), 1);
        let effect = &effects[0];
        // `bus` 未给出 ⇒ 规范缺省 `main`。
        assert_eq!(effect.bus(), crate::effect::EffectBus::Main);
        assert_eq!(effect.type_name(), Some("com.mda.Limiter"));
        assert_eq!(effect.param_offset(), Some(400));
        assert_eq!(effect.dsp_order(), None, "dsp_order not given");
        assert_eq!(
            effect.sends(),
            &[0.0, 0.0, 0.0, 0.0],
            "effect1..4 default 0"
        );
    }

    #[test]
    fn effect_reduction_is_deterministic_across_independent_parses() {
        let limits: ParseLimits = Default::default();
        let first = parse_text(CORPUS_EFFECT, &limits).expect("parses");
        let second = parse_text(CORPUS_EFFECT, &limits).expect("parses");
        assert_eq!(first.effects(), second.effects());
        assert_eq!(
            format!("{:?}", first.effects()),
            format!("{:?}", second.effects())
        );
    }

    #[test]
    fn too_many_opcodes_in_an_effect_block_hits_the_explicit_limit() {
        let limits = ParseLimits {
            max_opcodes_per_header: 2,
            ..ParseLimits::default()
        };
        let error = parse_text("<effect>bus=aux1\ntype=comp\nparam_offset=1", &limits)
            .expect_err("third opcode exceeds the per-header limit");
        assert!(
            matches!(
                error,
                SfzError::TooManyOpcodes {
                    scope: "effect",
                    limit: 2
                }
            ),
            "unexpected verdict: {error:?}"
        );
    }

    #[test]
    fn an_effect_header_between_two_regions_keeps_both_regions() {
        // 定义段不得吞掉相邻 region，也不得让前一个 region 丢掉继承值。
        let instrument = parse_text(
            "<global>volume=-6\n<region>sample=a.wav\n<effect>bus=aux1\n<region>sample=b.wav",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(instrument.len(), 2);
        assert_eq!(instrument.regions()[0].sample, "a.wav");
        assert_eq!(instrument.regions()[1].sample, "b.wav");
        assert_eq!(instrument.regions()[0].volume, -6.0);
        assert_eq!(instrument.regions()[1].volume, -6.0);
        assert_eq!(instrument.effects().len(), 1);
    }

    // ------------------------------------------------------------------
    // `<midi>` 头（ARIA；规范 <https://sfzformat.com/headers/midi/>）
    // ------------------------------------------------------------------

    #[test]
    fn midi_opcodes_are_registered_verbatim_in_file_order_with_their_own_lines() {
        let instrument = parse_text(
            "<group>key=36\n\
             <midi>cc1=64\n\
             curve_index=7\n\
             cc1=1\n\
             <region>sample=a.wav",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(instrument.len(), 1, "the region after the <midi> survives");
        let sections = instrument.midi_sections();
        assert_eq!(sections.len(), 1);
        let section = &sections[0];
        assert_eq!(section.line(), 2, "the <midi> header line");
        assert_eq!(section.len(), 3);
        // 原样登记：顺序 = 文件顺序，同名不被归并（与继承链的「后者胜」口径相反）。
        let names: Vec<&str> = section.opcodes().iter().map(MidiOpcode::name).collect();
        assert_eq!(names, vec!["cc1", "curve_index", "cc1"]);
        let values: Vec<&str> = section.opcodes().iter().map(MidiOpcode::value).collect();
        assert_eq!(values, vec!["64", "7", "1"]);
        // 每个 opcode 记的是**它自己**的行号，不是段头行。
        let lines: Vec<usize> = section.opcodes().iter().map(MidiOpcode::line).collect();
        assert_eq!(lines, vec![2, 3, 4]);
        assert_eq!(section.opcode("cc1"), Some("64"), "first match wins");
        assert_eq!(section.opcode("curve_index"), Some("7"));
        assert_eq!(section.opcode("CC1"), None, "lookup does not fold case");
        // 关键回归：`<midi>` 是定义段，**不**清空继承链。
        assert_eq!(instrument.regions()[0].lokey, 36, "group key survives");
        assert_eq!(instrument.regions()[0].hikey, 36);
    }

    #[test]
    fn an_empty_midi_section_is_registered_unlike_curve_and_effect() {
        // `<midi>` 段本身就是声明（规范把 `bus=midi` 的 `<effect>` 说成它的替代写法），
        // 所以空段也登记；`<curve>` / `<effect>` 的空段没有数据可丢，不产生条目。
        let instrument = parse_text(
            "<midi>\n<curve>\n<effect>\n<region>sample=a.wav",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(instrument.midi_sections().len(), 1);
        assert!(instrument.midi_sections()[0].is_empty());
        assert_eq!(instrument.midi_sections()[0].line(), 1);
        assert!(instrument.curves().is_empty(), "empty <curve> has no data");
        assert!(
            instrument.effects().is_empty(),
            "empty <effect> has no data"
        );
    }

    #[test]
    fn midi_preprocessor_declaration_covers_both_equivalent_spellings() {
        // 规范原文（<https://sfzformat.com/headers/midi/>，转引自 `EffectBus::Midi`）：
        // "From ARIA v1.0.8.0+ an `<effect>` section with a `bus=midi` can be used instead."
        let none = parse_text("<region>sample=a.wav", &Default::default()).expect("parses");
        assert!(!none.midi_preprocessor_declared());
        let header =
            parse_text("<midi>\n<region>sample=a.wav", &Default::default()).expect("parses");
        assert!(header.midi_preprocessor_declared(), "<midi> declaration");
        let alternative = parse_text(
            "<effect>bus=midi\n<region>sample=a.wav",
            &Default::default(),
        )
        .expect("parses");
        assert!(
            alternative.midi_preprocessor_declared(),
            "<effect>bus=midi is the specification's alternative spelling"
        );
        assert!(
            alternative.midi_sections().is_empty(),
            "the alternative spelling does not invent a <midi> section"
        );
        let other_bus = parse_text(
            "<effect>bus=aux1\n<region>sample=a.wav",
            &Default::default(),
        )
        .expect("parses");
        assert!(!other_bus.midi_preprocessor_declared());
    }

    #[test]
    fn a_midi_section_between_two_regions_keeps_both_regions() {
        let instrument = parse_text(
            "<global>volume=-6\n<region>sample=a.wav\n<midi>cc1=64\n<region>sample=b.wav",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(instrument.len(), 2);
        assert_eq!(instrument.regions()[0].volume, -6.0);
        assert_eq!(instrument.regions()[1].volume, -6.0);
        assert_eq!(instrument.midi_sections().len(), 1);
    }

    #[test]
    fn midi_reduction_is_deterministic_across_independent_parses() {
        let text = "<midi>cc1=64\ncurve_index=7\n<midi>\n<region>sample=a.wav";
        let limits: ParseLimits = Default::default();
        let first = parse_text(text, &limits).expect("parses");
        let second = parse_text(text, &limits).expect("parses");
        assert_eq!(first.midi_sections(), second.midi_sections());
        assert_eq!(
            format!("{:?}", first.midi_sections()),
            format!("{:?}", second.midi_sections())
        );
    }

    #[test]
    fn too_many_midi_sections_hits_the_explicit_limit() {
        // 显式小上限：`DEFAULT_MAX_MIDI_SECTIONS` 调大也抓不到这条（见报告里的「没红的注入」）。
        let limits = ParseLimits {
            max_midi_sections: 1,
            ..ParseLimits::default()
        };
        let error = parse_text("<midi>\n<midi>\n<region>sample=a.wav", &limits)
            .expect_err("second section exceeds max_midi_sections");
        assert!(
            matches!(error, SfzError::TooManyMidiSections { limit: 1 }),
            "unexpected verdict: {error:?}"
        );
    }

    #[test]
    fn too_many_opcodes_in_a_midi_block_hits_the_per_header_limit() {
        let limits = ParseLimits {
            max_opcodes_per_header: 2,
            ..ParseLimits::default()
        };
        let error = parse_text("<midi>cc1=64\ncurve_index=7\ncc2=1", &limits)
            .expect_err("third opcode exceeds the per-header limit");
        assert!(
            matches!(
                error,
                SfzError::TooManyOpcodes {
                    scope: "midi",
                    limit: 2
                }
            ),
            "unexpected verdict: {error:?}"
        );
    }

    #[test]
    fn the_total_midi_opcode_budget_is_enforced_across_sections() {
        // 两个段各 1 条：单段上限（默认 4096）抓不到，只有**总**预算能抓到。
        let limits = ParseLimits {
            max_midi_opcodes: 1,
            ..ParseLimits::default()
        };
        let error = parse_text("<midi>cc1=64\n<midi>cc2=1", &limits)
            .expect_err("second entry exceeds the total budget");
        assert!(
            matches!(error, SfzError::TooManyMidiOpcodes { limit: 1 }),
            "unexpected verdict: {error:?}"
        );
        // 空段不消耗预算：4 个空段在 `max_midi_opcodes: 0` 下仍然全是合法声明。
        let limits = ParseLimits {
            max_midi_opcodes: 0,
            ..ParseLimits::default()
        };
        let empty = parse_text("<midi>\n<midi>\n<midi>\n<midi>", &limits).expect("parses");
        assert_eq!(empty.midi_sections().len(), 4);
    }

    #[test]
    fn a_midi_opcode_is_registered_verbatim_and_never_typed() {
        // 原样登记口径：本 crate 不做类型化读取，所以「不是数字」「超长」「重复」的取值
        // 都只是字符串，不产生 Err（与继承链里的未知 opcode 同一条「未知不报错」口径）。
        let instrument = parse_text(
            "<midi>cc1=not-a-number\ncurve_index=99999999999999999999\ncc1=\ncc3=\u{00e9}",
            &Default::default(),
        )
        .expect("arbitrary text is registered verbatim, not typed");
        let section = &instrument.midi_sections()[0];
        assert_eq!(section.len(), 4);
        assert_eq!(section.opcodes()[0].value(), "not-a-number");
        assert_eq!(section.opcodes()[1].value(), "99999999999999999999");
        assert_eq!(section.opcodes()[2].value(), "");
        assert_eq!(section.opcodes()[3].value(), "\u{00e9}");
    }

    // ------------------------------------------------------------------
    // amp_veltrack / amp_velcurve_N（力度 → 振幅）
    // ------------------------------------------------------------------

    /// 解析出第一个 region，便于逐条判据读字段。
    fn first_region(text: &str) -> Region<'_> {
        parse_text(text, &Default::default())
            .expect("parses")
            .regions()
            .first()
            .cloned()
            .expect("one region")
    }

    #[test]
    fn amp_veltrack_defaults_to_the_specification_value_and_gives_identity_at_127() {
        // 规范表格：Default = 100（<https://sfzformat.com/opcodes/amp_veltrack/>）。
        let absent = first_region("<region>sample=a.wav");
        assert_eq!(absent.amp_veltrack, AMP_VELTRACK_DEFAULT);
        assert_eq!(absent.velocity_curve, None, "no amp_velcurve_N given");
        assert_eq!(absent.velocity_gain(127), 1.0);
        // 缺省 100 ⇒ 规范公式 (v/127)^2。
        let expected = (64.0f32 / 127.0) * (64.0 / 127.0);
        assert!((absent.velocity_gain(64) - expected).abs() <= 1.0e-6);

        // 显式 0 ⇒ 力度不改变振幅（教程：dynamics 由别的东西控制时设 0）。
        let off = first_region("<region>sample=a.wav amp_veltrack=0");
        assert_eq!(off.amp_veltrack, 0.0);
        assert_eq!(off.velocity_gain(64), 1.0);
    }

    #[test]
    fn amp_veltrack_out_of_range_is_an_explicit_error() {
        for value in ["101", "-100.5", "1e9"] {
            let error = parse_text(
                &format!("<region>sample=a.wav amp_veltrack={value}"),
                &Default::default(),
            )
            .expect_err("outside the -100..=100 range");
            assert!(
                matches!(error, SfzError::FloatOutOfRange { .. }),
                "value {value} gave {error:?}"
            );
        }
        // 边界值（含端点）合法。
        for value in ["-100", "100"] {
            parse_text(
                &format!("<region>sample=a.wav amp_veltrack={value}"),
                &Default::default(),
            )
            .expect("endpoints are inside the range");
        }
    }

    #[test]
    fn amp_velcurve_points_are_read_with_the_specification_endpoint_defaults() {
        let region = first_region("<region>sample=a.wav amp_velcurve_1=0.2 amp_velcurve_3=0.3");
        let curve = region.velocity_curve.as_ref().expect("explicit curve");
        assert_eq!(curve.points().first().map(|p| p.at), Some(0));
        assert_eq!(curve.points().first().map(|p| p.value), Some(0.0));
        assert_eq!(curve.points().last().map(|p| p.at), Some(127));
        assert_eq!(curve.points().last().map(|p| p.value), Some(1.0));
        // 规范原文算例：amp_velcurve_2 是 0.25。
        assert_eq!(region.velocity_gain(2), 0.25);
    }

    #[test]
    fn amp_velcurve_wins_over_amp_veltrack_when_both_are_given() {
        // 工程裁决：显式点表是「覆写标准曲线」而不是叠乘（出处见 velocity 模块文档）。
        // 登记语料里有 21 个文件同时给出两者，所以这条不是纯理论。
        let region = first_region("<region>sample=a.wav amp_veltrack=0 amp_velcurve_1=0.5");
        assert_eq!(region.amp_veltrack, 0.0, "the field is still carried");
        assert_eq!(
            region.velocity_gain(1),
            0.5,
            "the explicit table must win over amp_veltrack=0"
        );
        // 对照：同一条 region 去掉点表后，amp_veltrack=0 给出恒等。
        let without_curve = first_region("<region>sample=a.wav amp_veltrack=0");
        assert_eq!(without_curve.velocity_gain(1), 1.0);
    }

    #[test]
    fn amp_velcurve_follows_the_four_level_scope_chain() {
        // 优先级 region → group → master → global；同一 N 上内层胜。
        let region = first_region(
            "<global>amp_velcurve_64=0.1\n\
             <master>amp_velcurve_64=0.2 amp_velcurve_32=0.7\n\
             <group>amp_velcurve_64=0.3\n\
             <region>sample=a.wav amp_velcurve_64=0.4",
        );
        assert!(region.velocity_curve.is_some(), "explicit curve");
        assert_eq!(
            region.velocity_gain(64),
            0.4,
            "region wins over the other three"
        );
        assert_eq!(region.velocity_gain(32), 0.7, "master value survives");
    }

    #[test]
    fn amp_velcurve_index_and_value_ranges_are_enforced() {
        let index = parse_text(
            "<region>sample=a.wav amp_velcurve_128=0.5",
            &Default::default(),
        )
        .expect_err("N must be 0..=127");
        assert!(
            matches!(
                index,
                SfzError::VelocityCurveIndexOutOfRange { ref index, .. } if index == "128"
            ),
            "unexpected verdict: {index:?}"
        );

        // 数字但超出 u8：解析溢出也必须报同一个错误，而不是 panic 或静默丢弃。
        let overflow = parse_text(
            "<region>sample=a.wav amp_velcurve_99999999999999999999=0.5",
            &Default::default(),
        )
        .expect_err("a 20-digit index cannot be a u8");
        assert!(
            matches!(overflow, SfzError::VelocityCurveIndexOutOfRange { .. }),
            "unexpected verdict: {overflow:?}"
        );

        let value = parse_text(
            "<region>sample=a.wav amp_velcurve_1=1.5",
            &Default::default(),
        )
        .expect_err("amplitude must be 0..=1");
        assert!(
            matches!(
                value,
                SfzError::FloatOutOfRange {
                    min: 0.0,
                    max: 1.0,
                    ..
                }
            ),
            "unexpected verdict: {value:?}"
        );

        // 端点（含）合法。
        parse_text(
            "<region>sample=a.wav amp_velcurve_1=0 amp_velcurve_2=1",
            &Default::default(),
        )
        .expect("0 and 1 are inside the range");
    }

    #[test]
    fn amp_velcurve_non_numeric_value_uses_the_shared_float_errors() {
        let invalid = parse_text("<region>sample=a.wav amp_velcurve_1=x", &Default::default())
            .expect_err("not a number");
        assert!(
            matches!(invalid, SfzError::InvalidFloat { .. }),
            "unexpected verdict: {invalid:?}"
        );
        let non_finite = parse_text(
            "<region>sample=a.wav amp_velcurve_1=inf",
            &Default::default(),
        )
        .expect_err("not finite");
        assert!(
            matches!(non_finite, SfzError::NonFiniteFloat { .. }),
            "unexpected verdict: {non_finite:?}"
        );
    }

    #[test]
    fn names_that_only_look_like_amp_velcurve_are_ignored_like_any_unknown_opcode() {
        // 「未知不报错」口径与 `loccN` / `hiccN` 的识别函数一致：只有
        // `amp_velcurve_` + 全数字才按下标处理。
        let region = first_region(
            "<region>sample=a.wav amp_velcurve_foo=0.5 amp_velcurve_=0.5 amp_velcurve_1x=0.5",
        );
        assert_eq!(region.velocity_curve, None);
        assert_eq!(region.velocity_gain(64), region.veltrack_gain(64));
    }

    #[test]
    fn velocity_gain_is_deterministic_and_never_panics_over_the_whole_domain() {
        // 叶子 crate 红线：任意已解析输入都不得 panic，也不得产生 NaN / inf。
        let instrument = parse_text(
            "<region>sample=a.wav amp_veltrack=-100\n\
             <region>sample=b.wav amp_velcurve_0=0 amp_velcurve_127=1\n\
             <region>sample=c.wav amp_velcurve_1=0.4 amp_velcurve_63=1",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(instrument.len(), 3);
        for region in instrument.regions() {
            for velocity in 0u8..=127 {
                let gain = region.velocity_gain(velocity);
                assert!(
                    gain.is_finite(),
                    "velocity {velocity} on {} gave {gain}",
                    region.sample
                );
                assert_eq!(gain, region.velocity_gain(velocity), "not deterministic");
            }
        }
    }

    // ------------------------------------------------------------------
    // xfin_* / xfout_*（交叉淡化 → 振幅）
    // ------------------------------------------------------------------

    /// 固定 CC 读数的探针（判据用；`cc()` 是零分配的闭包）。
    fn cc_value(value: u8) -> impl Fn(u8) -> u8 {
        move |_| value
    }

    #[test]
    fn a_region_without_any_xf_opcode_has_no_crossfade_and_identity_gain() {
        let region = first_region("<region>sample=a.wav volume=-6");
        assert!(region.crossfades.is_empty());
        let query = RegionQuery::new(60, 100);
        assert_eq!(region.crossfade_gain(&query), 1.0);
    }

    #[test]
    fn a_lone_xfin_hicc_is_a_real_fade_in_from_zero() {
        // 语料里的主导写法（只给上界）：下界取 `xfin_loccN` 的规范 Default 0。
        let region = first_region("<region>sample=a.wav xfin_hicc1=100");
        assert_eq!(
            region.crossfades,
            vec![Crossfade::new(
                XfAxis::Cc(1),
                XfDirection::In,
                XfRange { low: 0, high: 100 },
                XfCurve::Power,
            )]
        );
        let probe = cc_value(0);
        assert_eq!(
            region.crossfade_gain(&RegionQuery::new(60, 100).with_cc(&probe)),
            0.0
        );
        let probe = cc_value(100);
        assert_eq!(
            region.crossfade_gain(&RegionQuery::new(60, 100).with_cc(&probe)),
            1.0
        );
        // 中点是等功率曲线 sqrt(0.5)，不是线性档的 0.5。
        let probe = cc_value(50);
        let half = region.crossfade_gain(&RegionQuery::new(60, 100).with_cc(&probe));
        assert!((half - 0.5f32.sqrt()).abs() <= 1.0e-6, "{half}");
    }

    #[test]
    fn a_lone_xfout_locc_is_a_real_fade_out_to_127() {
        // `xfout_locc1` 的 Default 取 127（整个 xfout 族一致），理由见 crate::crossfade。
        let region = first_region("<region>sample=a.wav xfout_locc1=64");
        assert_eq!(
            region.crossfades,
            vec![Crossfade::new(
                XfAxis::Cc(1),
                XfDirection::Out,
                XfRange { low: 64, high: 127 },
                XfCurve::Power,
            )]
        );
        let probe = cc_value(64);
        assert_eq!(
            region.crossfade_gain(&RegionQuery::new(60, 100).with_cc(&probe)),
            1.0
        );
        let probe = cc_value(127);
        assert_eq!(
            region.crossfade_gain(&RegionQuery::new(60, 100).with_cc(&probe)),
            0.0
        );
    }

    #[test]
    fn the_velocity_and_key_axes_use_the_query_fields() {
        // 力度轴用 `xfin_lovel` / `xfin_hivel` 的显式端点（语料里的动态层写法）。
        let velocity_fade =
            first_region("<region>sample=a.wav xfin_lovel=45 xfin_hivel=65 amp_veltrack=0");
        let query = RegionQuery::new(60, 45);
        assert_eq!(velocity_fade.crossfade_gain(&query), 0.0);
        let query = RegionQuery::new(60, 65);
        assert_eq!(velocity_fade.crossfade_gain(&query), 1.0);

        // 键盘轴用触发音号（`xfin_lokey` / `xfin_hikey`，语料里 0 次 ⇒ 规范完备性）。
        let key_fade =
            first_region("<region>sample=a.wav lokey=0 hikey=127 xfin_lokey=60 xfin_hikey=72");
        assert_eq!(key_fade.crossfade_gain(&RegionQuery::new(60, 100)), 0.0);
        assert_eq!(key_fade.crossfade_gain(&RegionQuery::new(72, 100)), 1.0);
        assert_eq!(key_fade.crossfade_gain(&RegionQuery::new(40, 100)), 0.0);
        assert_eq!(key_fade.crossfade_gain(&RegionQuery::new(100, 100)), 1.0);
    }

    #[test]
    fn a_missing_cc_state_leaves_the_cc_axis_at_identity() {
        // 与 CC 门控的严格策略不同：未提供状态只让该轴不贡献衰减，绝不静音整层。
        let region = first_region("<region>sample=a.wav xfin_hicc1=100");
        assert_eq!(region.crossfade_gain(&RegionQuery::new(60, 100)), 1.0);
    }

    #[test]
    fn several_crossfades_multiply_in_a_fixed_order() {
        // 顺序固定（键盘 in / out、力度 in / out、CC 升序 in、CC 升序 out），
        // 因此同一个体得到同一个 `Vec`（ARCH-DET-001）。
        let region = first_region(
            "<region>sample=a.wav xfout_hicc2=100 xfin_locc2=0 xfin_hicc1=100 \
             xfout_locc1=64 xfin_lovel=0 xfin_hivel=64 xfin_lokey=0 xfin_hikey=60",
        );
        let axes: Vec<(XfAxis, XfDirection)> = region
            .crossfades
            .iter()
            .map(|crossfade| (crossfade.axis, crossfade.direction))
            .collect();
        assert_eq!(
            axes,
            vec![
                (XfAxis::Key, XfDirection::In),
                (XfAxis::Velocity, XfDirection::In),
                (XfAxis::Cc(1), XfDirection::In),
                (XfAxis::Cc(2), XfDirection::In),
                (XfAxis::Cc(1), XfDirection::Out),
                (XfAxis::Cc(2), XfDirection::Out),
            ]
        );
        // 两段都生效时是**相乘**：CC1 的淡出段在 127 处归零，整条因此是 0；
        // 在 50 处只有淡入段在动，读数是等功率曲线的 sqrt(0.5)。
        let probe = cc_value(127);
        assert_eq!(
            region.crossfade_gain(&RegionQuery::new(60, 100).with_cc(&probe)),
            0.0
        );
        let probe = cc_value(50);
        let gain = region.crossfade_gain(&RegionQuery::new(60, 100).with_cc(&probe));
        assert!((gain - 0.5f32.sqrt()).abs() <= 1.0e-6, "{gain}");
    }

    #[test]
    fn the_curve_opcodes_are_per_axis_and_case_insensitive() {
        // `xf_cccurve` 落在 CC 轴上；缺省 `power` 是 sqrt。
        let linear =
            first_region("<region>sample=a.wav xfin_locc1=0 xfin_hicc1=100 xf_cccurve=GAIN");
        let probe = cc_value(25);
        let gain = linear.crossfade_gain(&RegionQuery::new(60, 100).with_cc(&probe));
        assert!((gain - 0.25).abs() <= 1.0e-6, "{gain}");

        // 力度轴**不**读 `xf_cccurve`：同一段在 `xf_velcurve=power`（缺省）下是 sqrt。
        let velocity =
            first_region("<region>sample=a.wav xfin_lovel=0 xfin_hivel=100 xf_cccurve=gain");
        let gain = velocity.crossfade_gain(&RegionQuery::new(60, 25));
        assert!((gain - 0.25f32.sqrt()).abs() <= 1.0e-6, "{gain}");
    }

    #[test]
    fn the_crossfade_opcodes_follow_the_four_level_scope_chain() {
        let instrument = parse_text(
            "<global>xfin_hicc1=100 xf_cccurve=gain\n\
             <master>xfin_hicc1=80\n\
             <group>xfin_locc1=20\n\
             <region>sample=a.wav xfin_hicc1=60",
            &Default::default(),
        )
        .expect("parses");
        let region = &instrument.regions()[0];
        assert_eq!(
            region.crossfades,
            vec![Crossfade::new(
                XfAxis::Cc(1),
                XfDirection::In,
                XfRange { low: 20, high: 60 },
                XfCurve::Gain,
            )],
            "region wins for the high endpoint, group supplies the low one, \
             and the global curve survives"
        );
    }

    #[test]
    fn a_curve_value_outside_the_whitelist_is_an_explicit_error() {
        let outcome = parse_text(
            "<region>sample=a.wav xfin_hicc1=100 xf_cccurve=linear",
            &Default::default(),
        );
        assert!(
            matches!(outcome, Err(SfzError::InvalidOption { ref opcode, .. }) if opcode == "xf_cccurve"),
            "unexpected verdict: {outcome:?}"
        );
    }

    #[test]
    fn xf_endpoints_and_cc_indices_are_range_checked_explicitly() {
        // 端点越界：明确 Err，不静默钳位。
        let outcome = parse_text("<region>sample=a.wav xfin_hicc1=128", &Default::default());
        assert!(
            matches!(
                outcome,
                Err(SfzError::IntegerOutOfRange {
                    ref opcode,
                    value: 128,
                    min: 0,
                    max: 127,
                    ..
                }) if opcode == "xfin_hicc1"
            ),
            "unexpected verdict: {outcome:?}"
        );
        let outcome = parse_text("<region>sample=a.wav xfout_lovel=-1", &Default::default());
        assert!(
            matches!(outcome, Err(SfzError::IntegerOutOfRange { .. })),
            "unexpected verdict: {outcome:?}"
        );
        // 非数字取值：明确 Err。
        let outcome = parse_text("<region>sample=a.wav xfin_hivel=loud", &Default::default());
        assert!(
            matches!(outcome, Err(SfzError::InvalidInteger { .. })),
            "unexpected verdict: {outcome:?}"
        );
        // CC 号 > 127 与畸形名字按未知 opcode 忽略（与 `loccN` / `hiccN` 同一条口径）；
        // 但**认得出来的**名字配空取值是明确 Err（与 `locc1=` 同一条口径）。
        let ignored =
            first_region("<region>sample=a.wav xfin_hicc131=10 xfin_hicc=10 xfin_hiccfoo=10");
        assert!(ignored.crossfades.is_empty(), "{:?}", ignored.crossfades);
        let outcome = parse_text("<region>sample=a.wav xfin_hicc1=", &Default::default());
        assert!(
            matches!(outcome, Err(SfzError::InvalidInteger { ref opcode, .. }) if opcode == "xfin_hicc1"),
            "unexpected verdict: {outcome:?}"
        );
    }

    #[test]
    fn a_degenerate_crossfade_range_is_preserved_and_stays_inactive() {
        // 两端同值：区间长度 0 ⇒ 不生效（不 panic、不除零、不改音量）。
        let region = first_region("<region>sample=a.wav xfout_locc1=64 xfout_hicc1=64");
        assert_eq!(
            region.crossfades,
            vec![Crossfade::new(
                XfAxis::Cc(1),
                XfDirection::Out,
                XfRange { low: 64, high: 64 },
                XfCurve::Power,
            )]
        );
        let probe = cc_value(64);
        assert_eq!(
            region.crossfade_gain(&RegionQuery::new(60, 100).with_cc(&probe)),
            1.0
        );
    }

    #[test]
    fn crossfade_gain_is_deterministic_and_never_panics_over_the_whole_domain() {
        // 叶子 crate 红线：任意已解析输入都不得 panic，也不得产生 NaN / inf。
        let instrument = parse_text(
            "<region>sample=a.wav xfin_lovel=0 xfin_hivel=127 xfin_hicc1=64\n\
             <region>sample=b.wav xfout_lovel=64 xfout_hivel=64 xfout_locc1=100\n\
             <region>sample=c.wav xfin_lokey=60 xfin_hikey=60",
            &Default::default(),
        )
        .expect("parses");
        for region in instrument.regions() {
            for note in [0u8, 1, 60, 127] {
                for velocity in [0u8, 1, 64, 127] {
                    for cc in [0u8, 1, 64, 127] {
                        let probe = cc_value(cc);
                        let query = RegionQuery::new(note, velocity).with_cc(&probe);
                        let gain = region.crossfade_gain(&query);
                        assert!(
                            gain.is_finite() && (0.0..=1.0).contains(&gain),
                            "{} gave {gain} at note {note} velocity {velocity} cc {cc}",
                            region.sample
                        );
                        assert_eq!(
                            gain.to_bits(),
                            region.crossfade_gain(&query).to_bits(),
                            "not deterministic"
                        );
                    }
                }
            }
        }
    }
}
