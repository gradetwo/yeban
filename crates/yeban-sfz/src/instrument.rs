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

use crate::error::SfzError;
use crate::parser::Warning;
use crate::parser::{OpcodeMap, OpcodeValue, parse_int};

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

/// `off_mode` opcode：note-off（或 `off_by` 关断）到达时，声部**如何结束**。
///
/// 取值集合与缺省值取自登记语料的 opcode 普查（三个取值 `normal` / `fast` / `time`，
/// 后者的出现次数为 818 / 87 / 4，合计 909），见
/// `docs/ledger/sfz-core-notes.md` 第 11 节；格式出处
/// <https://sfzformat.com/opcodes/off_mode/>。
///
/// **出处分工**：取值集合、出现次数与缺省值 `fast` 来自上面那份**登记语料普查**；
/// 三个取值各自的含义来自上面那个**格式页**。其中「`fast` ＝ 立刻结束」一条另有台账佐证：
/// 同节写明缺省值 `fast`「与现行为等价」，而现行为就是在 note-off 立刻结束声部。
///
/// - [`OffMode::Fast`]（缺省）：立刻结束声部。
/// - [`OffMode::Normal`]：按正常（包络）release 结束声部。
/// - [`OffMode::Time`]：在 `off_time` 秒之后结束声部。
///
/// **本 crate 只做类型化建模，不决定 release 的实现**：包络属于引擎侧；
/// 而 per-region `off_time` 与 [ARCH-RT-004] 的 3 ms 窃取淡出冲突、正等人类裁决
/// （`docs/ledger/sfz-core-notes.md` 第 6 节第 4 条），因此本切片刻意**不**读 `off_time`。
/// 消费方得到的是一条明确契约：只有 [`OffMode::Fast`] 允许立刻切断声部。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffMode {
    /// `fast`（缺省）：立刻结束声部。
    Fast,
    /// `normal`：按正常 release 结束声部（需要包络，引擎侧）。
    Normal,
    /// `time`：在 `off_time` 秒后结束声部（`off_time` 未建模，见类型文档）。
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
    /// 另外两个取值要求一段 release（`normal`）或一段 `off_time` 保持（`time`），
    /// 因此调用方**不得**把它们当成立刻切断 —— 这正是规范区分三者的目的。
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
    /// 力度下界（含）。
    pub lovel: u8,
    /// 力度上界（含）。
    pub hivel: u8,
    /// MIDI 通道下界（含，1..=16）。
    pub lochan: u8,
    /// MIDI 通道上界（含，1..=16）。
    pub hichan: u8,
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

    /// 该 region 是否需要「其它按住的音符数」这类外部状态（`trigger=first` /
    /// `legato`）。
    #[must_use]
    pub fn has_held_notes_gate(&self) -> bool {
        matches!(self.trigger, Trigger::First | Trigger::Legato)
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
    /// 按音符分桶的 region 下标（加速 `region_for`，构造后只读）。
    key_buckets: Vec<Vec<u32>>,
    warnings: Vec<Warning>,
}

impl<'a> Instrument<'a> {
    /// 从 region 列表构造乐器（并建立音符索引）。
    pub(crate) fn new(regions: Vec<Region<'a>>, warnings: Vec<Warning>) -> Self {
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
            key_buckets,
            warnings,
        }
    }

    /// 全部 region（保持文件出现顺序，确定性）。
    #[must_use]
    pub fn regions(&self) -> &[Region<'a>] {
        &self.regions
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
        let bucket = self.key_buckets.get(query.note as usize)?;

        let mut seq_length = 1u32;
        for &index in bucket {
            let region = &self.regions[index as usize];
            if region_matches(region, &query) {
                seq_length = region.seq_length.max(1);
                break;
            }
        }

        let target = (query.occurrence % u64::from(seq_length)) as u32 + 1;
        for &index in bucket {
            let region = &self.regions[index as usize];
            if region.seq_position == target && region_matches(region, &query) {
                return Some(region);
            }
        }

        for &index in bucket {
            let region = &self.regions[index as usize];
            if region_matches(region, &query) {
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
    // 刻意不读 `off_time`：它与 [ARCH-RT-004] 的 3 ms 窃取淡出的冲突待人类裁决
    // （同文件第 6 节第 4 条）。
    let off_mode = match scopes.get("off_mode") {
        Some(value) => value.as_option(OffMode::OPTIONS, OffMode::ALLOWED)?,
        None => OffMode::Fast,
    };

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

    Ok(Some(Region {
        sample,
        default_path: default_path.filter(|path| !path.is_empty()),
        lokey,
        hikey,
        pitch_keycenter,
        trigger_by_note,
        trigger,
        off_mode,
        lovel,
        hivel,
        lochan,
        hichan,
        loop_start,
        loop_end,
        loop_mode,
        offset,
        end,
        direction,
        tune,
        transpose,
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
    use crate::parser::{Header, parse_text};

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
            lovel: 0,
            hivel: 127,
            lochan: 1,
            hichan: 16,
            loop_start: 0,
            loop_end: 0,
            loop_mode: LoopMode::NoLoop,
            offset: 0,
            end: SampleEnd::Unspecified,
            direction: PlayDirection::Forward,
            tune: 0,
            transpose: 0,
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
        let instrument = Instrument::new(vec![region(60, 1, 1), region(62, 1, 1)], Vec::new());
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
}
