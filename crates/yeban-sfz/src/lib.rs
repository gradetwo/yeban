//! # yeban-sfz — SFZ v2 采样器
//!
//! 零拷贝 SFZ v2 词法 / 结构解析器 + 预分配 voice pool（默认 512 声部、可配 1024）
//! [ARCH-RT-004]。解析器是不可信输入边界：必须能承受 `cargo-fuzz` 千万次变异零崩溃
//! [MUST-GATE-011]。
//!
//! 识别的段头是 `<control>` / `<global>` / `<master>` / `<group>` / `<region>`、
//! `<curve>`、`<effect>` 与 `<midi>`，作用域链 `region → group → master → global`
//! （[`Header::Master`] 是 ARIA 扩展，见 <https://sfzformat.com/headers/>）。
//! `<curve>` 是**定义段**：它的
//! `curve_index` 与 `v000..v127` 只进 [`Curve`]（经 [`Instrument::curves`] /
//! [`Instrument::curve_value_at`] 读取），不进继承链、也不清空继承链。`<effect>` 同为
//! **定义段**：它的 `bus` / `type` / `param_offset` / `dsp_order` / `effect1`..`effect4`
//! 只进 [`Effect`]（经 [`Instrument::effects`] 读取），同样不进继承链、也不清空继承链。
//! `<midi>` 也是**定义段**：它的 opcode 只进 [`MidiSection`]（经
//! [`Instrument::midi_sections`] 读取），**原样登记、不解释语义**（ARIA 的 `<midi>`
//! opcode 词汇跨播放器不一致）。仍未建模的段头（`<sample>`）产生
//! [`Warning::IgnoredHeader`] 并丢弃其 opcode。
//!
//! 关断语义由 `off_mode`（[`OffMode`]）与 `off_time`（[`Region::off_time`]，缺省
//! [`OFF_TIME_DEFAULT_SECONDS`]）成对带出；后者只在 `off_mode=time` 时生效
//! （<https://sfzformat.com/opcodes/off_time/>），不改写 [ARCH-RT-004] 的 3 ms
//! 窃取淡出常量。
//!
//! 同音同时发声数限制由 `note_polyphony` 与 `note_selfmask`（规范缺省 `on`，缺省值即
//! [`NotePolyphony::UNLIMITED`]）成对带出，归约进 [`Region::note_polyphony`]
//! 与 [`PlaybackSpec::note_polyphony`]。检查的键是 polyphony group
//! （[`Region::group`]，缺省 0）加音高（<https://sfzformat.com/opcodes/note_polyphony/>）；
//! 判定函数是 [`VoicePool::apply_note_polyphony`]，它把让位的声部置为
//! [`VoiceInfo::retiring`] 并进入既有 3 ms 指数淡出路径，全程零分配。
//! `limit = 0` 读作「不限制」、以及对 `note_polyphony=1` 之外取值的推广规则
//! 都是工程裁决，逐条登记在 [`NotePolyphony`] 的文档里。
//!
//! 力度 → 振幅由 `amp_veltrack`（[`Region::amp_veltrack`]，缺省
//! [`AMP_VELTRACK_DEFAULT`]）与 `amp_velcurve_N`（[`Region::velocity_curve`]）成对带出；
//! 求值见 [`Region::velocity_gain`]，合并进 [`PlaybackSpec::total_gain`]。
//! **显式点表优先于 `amp_veltrack`** 是本 crate 的工程裁决，理由与规范出处见
//! [`velocity`]（该模块的文档同时登记了 `amp_veltrack` 非缺省取值的待裁决项）。
//!
//! 弯音范围由 `bend_up`（[`Region::bend_up`]，缺省 [`BEND_UP_DEFAULT_CENTS`]）与
//! `bend_down`（[`Region::bend_down`]，缺省 [`BEND_DOWN_DEFAULT_CENTS`]）成对带出，
//! 单位都是音分、范围都是 `±BEND_RANGE_MAX_CENTS`；求值见 [`Region::bend_cents`] /
//! [`Region::bend_ratio`]，字段也进 [`PlaybackSpec`]。轮值本身由调用方提供
//! （本 crate 不接收 MIDI 输入）。
//!
//! 交叉淡化由 `xfin_*` / `xfout_*` 三族（键盘位置 / 力度 / MIDI CC，每族两个方向，
//! 外加 `xf_keycurve` / `xf_velcurve` / `xf_cccurve` 三条曲线）带出，归约进
//! [`Region::crossfades`]，求值见 [`Region::crossfade_gain`]，合并进
//! [`PlaybackSpec::total_gain`] 的第三段 [`PlaybackSpec::crossfade_gain`]。
//! `xfout_loccN` 的规范 Default 与同族其余 opcode 冲突、`power` 曲线的形状规范未定
//! —— 两条取舍都登记在 [`crossfade`] 的模块文档里。改动前这一族被完全忽略；在登记的
//! 1267 个可解析音色上它命中 68 个文件、7812 个 region（共 9259 段）。
//!
//! 标签族（`sw_label` / `label_ccN` / `region_label` / `group_label` / `master_label` /
//! `global_label`）是 ARIA 的 GUI 元数据：它**不改变**任何 region 选择或渲染结果。
//! 归约进 [`Region::labels`]（[`Labels`]），文件级 `<control>` 里的 `label_ccN` 另见
//! [`Instrument::cc_labels`]，「当前选中的 keyswitch 名字」见
//! [`Instrument::keyswitch_label`]。取值全是 [`std::borrow::Cow`]，无宏替换时零拷贝。
//! 这一族的规范出处、`label_ccN` 下标上界与 `scope_label` 优先序两条工程裁决
//! 都登记在 [`label`] 的模块文档里。
//!
//! 规范来源 (Normative):
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` ROAD-M2-005 / ROAD-M2-006
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3.2 ARCH-RT-001 / ARCH-RT-004
//!
//! 移植语义来源 (learn-from, MIT): `groove/src/audio/sfz/**`（TypeScript, 2,979 行）——
//! 只借鉴 keyswitch / include / define / CC gate / 轮替的**语义**，代码用 Rust 重写，
//! 不做逐行转译（见 `docs/ledger/legacy-reuse-audit.md`）。
//!
//! 格式事实与出处 URL、显式上限口径、未实现清单、待人类裁决的歧义见
//! `docs/ledger/sfz-core-notes.md`。
//!
//! ## 快速开始
//!
//! ```
//! use yeban_sfz::{ParseLimits, parse_text};
//!
//! let instrument = parse_text(
//!     "<region>lokey=36 hikey=36 sample=kick.wav",
//!     &ParseLimits::default(),
//! )?;
//! let region = instrument.region_for(36, 100).expect("region covers note 36");
//! assert_eq!(region.sample, "kick.wav");
//! # Ok::<(), yeban_sfz::SfzError>(())
//! ```
//!
//! 把一次 note-on 变成**可渲染的采样描述**（音高比 / 步进比 / 线性增益 / 循环窗口）：
//!
//! ```
//! use yeban_sfz::{ParseLimits, RegionQuery, RenderRates, parse_text};
//!
//! let instrument = parse_text(
//!     "<group>key=36 seq_length=2\n\
//!      <region>seq_position=1 sample=k1.wav pitch_keycenter=48 volume=-3\n\
//!      <region>seq_position=2 sample=k2.wav pitch_keycenter=48 volume=-3",
//!     &ParseLimits::default(),
//! )?;
//! let rates = RenderRates::new(44_100.0, 48_000.0);
//! let play = instrument
//!     .playback_for(RegionQuery::new(36, 100).with_occurrence(0), rates)
//!     .expect("region covers note 36");
//! assert_eq!(play.region.sample, "k1.wav");
//! assert_eq!(play.spec.pitch_ratio, 0.5); // 36 比根音 48 低一个八度
//! assert!(play.spec.rate < 0.5); // 再乘上 44100/48000 的采样率换算
//! # Ok::<(), yeban_sfz::SfzError>(())
//! ```
//!
//! `<curve>` 段定义 MIDI CC 调制曲线（`curve_index` 7..=254，点 `v000..v127`；
//! 点之间按规范线性插值，`v000` / `v127` 缺省为 0 与 1）：
//!
//! ```
//! use yeban_sfz::{ParseLimits, parse_text};
//!
//! let instrument = parse_text(
//!     "<curve>curve_index=7\nv000=0\nv095=1\nv127=1",
//!     &ParseLimits::default(),
//! )?;
//! assert_eq!(instrument.curve_value_at(7, 95.0), Some(1.0));
//! assert_eq!(instrument.curve_value_at(7, 47.5), Some(0.5)); // 0 与 95 的中点
//! // 0..=6 是 ARIA 内建曲线（不可覆写）：编号 1 是 -1 → 1 的双极直线。
//! assert_eq!(instrument.curve_value_at(1, 0.0), Some(-1.0));
//! # Ok::<(), yeban_sfz::SfzError>(())
//! ```
//!
//! `<effect>` 段定义一条效果器总线声明（`bus` / `type` / `param_offset` / `dsp_order` /
//! `effect1`..`effect4`；定义段，不进继承链）：
//!
//! ```
//! use yeban_sfz::{EffectBus, ParseLimits, parse_text};
//!
//! let instrument = parse_text(
//!     "<effect>bus=aux1 type=com.mda.Limiter param_offset=400 dsp_order=2 effect1=50\n\
//!      <region>sample=kick.wav",
//!     &ParseLimits::default(),
//! )?;
//! let effect = &instrument.effects()[0];
//! assert_eq!(effect.bus(), EffectBus::Aux(1));
//! assert_eq!(effect.type_name(), Some("com.mda.Limiter"));
//! assert_eq!(effect.param_offset(), Some(400));
//! assert_eq!(effect.dsp_order(), Some(2));
//! assert_eq!(effect.sends(), &[50.0, 0.0, 0.0, 0.0]); // effect2..4 缺省 0
//! assert_eq!(instrument.len(), 1, "the region after the effect survives");
//! // 规范原文："If not set, or any other value is set, this goes to the main output."
//! assert_eq!(EffectBus::from_value("aux99"), EffectBus::Main);
//! # Ok::<(), yeban_sfz::SfzError>(())
//! ```
//!
//! `<midi>` 段声明 ARIA 的 MIDI 预处理器（定义段，不进继承链）。段内 opcode 的
//! **语义**本 crate 不解释（词汇跨播放器不一致），只**原样**登记名字 / 取值 / 行号；
//! 规范把 `<effect>bus=midi` 说成它的替代写法，两者对
//! [`Instrument::midi_preprocessor_declared`] 等价：
//!
//! ```
//! use yeban_sfz::{ParseLimits, parse_text};
//!
//! let instrument = parse_text(
//!     "<midi>cc1=64 curve_index=7\n<effect>bus=midi\n<region>sample=kick.wav",
//!     &ParseLimits::default(),
//! )?;
//! assert_eq!(instrument.len(), 1, "the region after the declarations survives");
//! let section = &instrument.midi_sections()[0];
//! assert_eq!(section.line(), 1);
//! assert_eq!(section.len(), 2);
//! assert_eq!(section.opcode("cc1"), Some("64"));
//! assert_eq!(section.opcodes()[1].name(), "curve_index");
//! assert_eq!(section.opcodes()[1].value(), "7");
//! assert!(instrument.midi_preprocessor_declared());
//! # Ok::<(), yeban_sfz::SfzError>(())
//! ```
//!
//! 力度 → 振幅：`amp_veltrack`（规范缺省 100 ⇒ `(v/127)^2`）与
//! `amp_velcurve_N`（归一化振幅点表，未给出的点线性插值，缺省端点 0 与 1）：
//!
//! ```
//! use yeban_sfz::{ParseLimits, RegionQuery, RenderRates, parse_text};
//!
//! let instrument = parse_text(
//!     "<region>key=36 sample=kick.wav amp_velcurve_1=0.2 amp_velcurve_3=0.3",
//!     &ParseLimits::default(),
//! )?;
//! let region = &instrument.regions()[0];
//! // 规范原文算例：amp_velcurve_1=0.2 / amp_velcurve_3=0.3 ⇒ amp_velcurve_2 是 0.25。
//! assert_eq!(region.velocity_gain(2), 0.25);
//! assert_eq!(region.velocity_gain(0), 0.0); // 缺省端点
//! assert_eq!(region.velocity_gain(127), 1.0); // 缺省端点
//!
//! let standard = parse_text(
//!     "<region>key=36 sample=kick.wav",
//!     &ParseLimits::default(),
//! )?;
//! // 没有点表 ⇒ amp_veltrack 的规范缺省 100 ⇒ (v/127)^2。
//! assert_eq!(standard.regions()[0].velocity_gain(127), 1.0);
//! let play = standard
//!     .playback_for(RegionQuery::new(36, 127), RenderRates::default())
//!     .expect("region covers note 36");
//! assert_eq!(play.spec.velocity_gain, 1.0);
//! assert_eq!(play.spec.total_gain(), play.spec.gain);
//! # Ok::<(), yeban_sfz::SfzError>(())
//! ```
//!
//! 弯音范围：`bend_up` / `bend_down` 是**两个独立**的音分数（规范缺省 200 / -200，
//! 范围都是 ±9600），线性插值见 [`Region::bend_cents`]：
//!
//! ```
//! use yeban_sfz::{ParseLimits, PITCH_BEND_CENTER, RegionQuery, RenderRates, parse_text};
//!
//! let instrument = parse_text(
//!     "<region>key=36 sample=kick.wav pitch_keycenter=60 bend_up=1200 bend_down=-1200",
//!     &ParseLimits::default(),
//! )?;
//! let region = &instrument.regions()[0];
//! assert_eq!(region.bend_up, 1200);
//! assert_eq!(region.bend_down, -1200);
//! // 中位轮值不弯音 ⇒ 与不含弯音的音高比逐位相同。
//! assert_eq!(region.bend_cents(PITCH_BEND_CENTER), 0);
//! assert_eq!(region.bend_ratio(36, PITCH_BEND_CENTER), region.pitch_ratio(36));
//! // 轮子推到底（127）正好是整个 bend_up 范围；音高比是它乘上 2^(12/12) = 2。
//! assert_eq!(region.bend_cents(127), 1200);
//! assert_eq!(region.bend_ratio(36, 127), region.pitch_ratio(36) * 2.0);
//! // 轮子拉到 0 正好是整个 bend_down 范围（负值 ⇒ 音高下降）。
//! assert_eq!(region.bend_cents(0), -1200);
//!
//! let play = instrument
//!     .playback_for(RegionQuery::new(36, 100), RenderRates::default())
//!     .expect("region covers note 36");
//! assert_eq!(play.spec.bend_up, 1200);
//! assert_eq!(play.spec.bend_down, -1200);
//! # Ok::<(), yeban_sfz::SfzError>(())
//! ```
//!
//! 交叉淡化：`xfin_*` / `xfout_*` 按键盘位置、力度或 MIDI CC 缩放音量
//! （规范缺省曲线 `power` 取等功率的 `sqrt`，见 [`crossfade`]）：
//!
//! ```
//! use yeban_sfz::{ParseLimits, RegionQuery, RenderRates, XfAxis, XfDirection, parse_text};
//!
//! let instrument = parse_text(
//!     "<region>key=36 sample=loud.wav xfout_locc1=64\n\
//!      <region>key=36 sample=soft.wav xfin_hicc1=64",
//!     &ParseLimits::default(),
//! )?;
//! let soft = &instrument.regions()[1];
//! // 只给了一个端点 ⇒ 另一个取该族的规范缺省：淡入是 [0, 64]、淡出是 [64, 127]。
//! assert_eq!(soft.crossfades[0].axis, XfAxis::Cc(1));
//! assert_eq!(soft.crossfades[0].direction, XfDirection::In);
//! let quiet = |_: u8| 0u8;
//! let loud = |_: u8| 127u8;
//! assert_eq!(soft.crossfade_gain(&RegionQuery::new(36, 100).with_cc(&quiet)), 0.0);
//! assert_eq!(soft.crossfade_gain(&RegionQuery::new(36, 100).with_cc(&loud)), 1.0);
//! // 选择只返回第一个匹配的 region：调制轮推满时它（大声层）已被淡出到 0，
//! // 而小声层在同一个 CC 上正好是满幅 —— 两层随调制轮交叉。
//! let play = instrument
//!     .playback_for(RegionQuery::new(36, 100).with_cc(&loud), RenderRates::default())
//!     .expect("region covers note 36");
//! assert_eq!(play.region.sample, "loud.wav");
//! assert_eq!(play.spec.crossfade_gain, 0.0);
//! # Ok::<(), yeban_sfz::SfzError>(())
//! ```
//!
//! 需要 `#include` 时先解析再解析文本（两步走，保持核心解析器是纯函数）：
//! ```no_run
//! use yeban_sfz::{IncludeResolver, ParseLimits, parse_sources};
//!
//! let limits = ParseLimits::default();
//! let resolver = IncludeResolver::new("/path/to/library", limits)?;
//! let sources = resolver.resolve("instrument.sfz")?;
//! let instrument = parse_sources(&sources, &limits)?;
//! # Ok::<(), yeban_sfz::SfzError>(())
//! ```
//!
//! ## 硬性约束
//!
//! 1. **零 GUI / 零重依赖** [ARCH-TOP-003]：本 crate 不引入 slint / cpal / symphonia / rayon，
//!    仅依赖 `thiserror`。由 `scripts/guards/policy_check.py` 机械校验。
//! 2. **内存安全**：本 crate `#![forbid(unsafe_code)]`。
//! 3. **不可信输入不 panic**：解析路径全部 `Result`，无 `unwrap` / `expect`。
//! 4. **实时路径零分配** [ARCH-RT-001]：`VoicePool::note_on` / `process` / `retire` 不触碰堆。
//!
//! ## 与实时引擎的接口
//!
//! 本 crate 已经能解析乐器（[`parse_text`] → [`Instrument::region_for`]）并持有预分配的
//! [`VoicePool`]（默认 [`DEFAULT_VOICE_CAPACITY`] / 上限 [`MAX_VOICE_CAPACITY`]，
//! 含 [`StealFade`] 的 3 ms 淡出）。
//!
//! **纯数值的桥已经就位**：[`Instrument::playback_for`] 一次调用完成
//! 「选 region → 可渲染的采样描述」（[`PlaybackSpec`]：音高比 / 步进比 / 线性增益 /
//! 循环窗口 / 独占组）。该调用零堆分配，可在实时路径使用。它只做换算，
//! **不**做 I/O、不解码音频、不猜采样率。
//!
//! 真正发声仍缺两个前提（登记在 `docs/ledger/engine-sound-notes.md` §5.3 与 needs N6）：
//!
//! ```text
//! 1) 采样数据解码（WAV/FLAC）—— 需要依赖裁决（symphonia/hound）或自研解码器。
//!    解码器还必须提供采样文件自身的采样率（[`RenderRates::sample_hz`]）与真实循环点；
//! 2) 数据本身：`assets/samples/` 是**登记式(registry-only)**的 —— `manifest.json` 已登记
//!    30 款乐器 / 20 594 个文件的许可 + SHA-256（每条 `optional: true`，**字节不入库**），
//!    `ATTRIBUTION.md` 逐条署名。要让本 crate 真正发声，仍需按清单的 `repo` + `pin`
//!    拉取采样字节（拉取与校验步骤见该清单 §5 与 `docs/ledger/samples-attribution-notes.md`）。
//!    注意 `MUST-GATE-014` 的规范目标是 323 款，当前登记 30 款，差额 293 款。
//! ```
//!
//! 重采样本身**不再需要**新依赖来做决策：region 的 `pitch_keycenter` / `transpose` /
//! `tune` / 采样率差异已经归约成一个 [`PlaybackSpec::rate`]（每输出采样前进多少源采样）。
//! 引擎可以用它驱动任意插值器。
//!
//! [`Region::sample_path`] 给出采样身份，供**加载期**把 region 映射到已解码缓冲；
//! 实时路径只读 [`PlaybackSpec`]。
//!
//! **引擎侧的接入点已经就位**：`yeban-engine` 的 `SynthEngine::trigger` 是"选一个声部并
//! 初始化它"的唯一位置（`docs/ledger/engine-sound-notes.md` §5.3 有精确说明）。
//! 把"内置波表 + 定点相位"换成"region → 采样播放"只需在那里分派，
//! 逐样本循环、电平、母线汇流与零分配约束都不需要改。

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod crossfade;
pub mod curve;
pub mod effect;
pub mod error;
pub mod instrument;
pub mod label;
pub mod midi;
pub mod parser;
pub mod playback;
pub mod velocity;
pub mod voice_pool;

pub use crossfade::{Crossfade, XfAxis, XfCurve, XfDirection, XfRange, fade_in, fade_out};
pub use curve::{Curve, CurvePoint, MAX_BUILT_IN_CURVE_INDEX, MAX_CURVE_INDEX};
pub use effect::{Effect, EffectBus, MAX_AUX_BUS, MAX_DSP_ORDER, MAX_FX_BUS, SEND_COUNT};
pub use error::SfzError;
pub use instrument::{
    BEND_DOWN_DEFAULT_CENTS, BEND_RANGE_MAX_CENTS, BEND_UP_DEFAULT_CENTS, CcGate, Instrument,
    LoopMode, NotePolyphony, OFF_TIME_DEFAULT_SECONDS, OffMode, PITCH_BEND_CENTER, PlayDirection,
    Region, RegionQuery, SampleEnd, Trigger, TriggerEvent,
};
pub use label::{Labels, MAX_CC_LABEL_INDEX, parse_cc_label_name};
pub use midi::{MidiOpcode, MidiSection};
pub use parser::{
    Header, IncludeResolver, OpcodeValue, ParseLimits, SfzSource, Warning, parse_f32, parse_int,
    parse_note, parse_sources, parse_text,
};
pub use playback::{
    FALLBACK_SAMPLE_RATE, LoopWindow, PlaybackSpec, RegionPlay, RenderRates, SampleSpan,
};
pub use velocity::{
    AMP_VELTRACK_DEFAULT, AMP_VELTRACK_MAX, AMP_VELTRACK_MIN, MAX_VELCURVE_AMPLITUDE,
    MAX_VELCURVE_INDEX, MIN_VELCURVE_AMPLITUDE, MIN_VELCURVE_INDEX, VelocityCurve, veltrack_gain,
};
pub use voice_pool::{
    DEFAULT_VOICE_CAPACITY, MAX_VOICE_CAPACITY, NoteOnOutcome, SILENT_DBFS, STEAL_FADE_FLOOR,
    STEAL_FADE_MILLIS, StealFade, VoiceHandle, VoiceInfo, VoicePool, VoiceStage,
};
