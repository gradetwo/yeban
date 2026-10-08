//! # yeban-sfz — SFZ v2 采样器
//!
//! 零拷贝 SFZ v2 词法 / 结构解析器 + 预分配 voice pool（默认 512 声部、可配 1024）
//! [ARCH-RT-004]。解析器是不可信输入边界：必须能承受 `cargo-fuzz` 千万次变异零崩溃
//! [MUST-GATE-011]。
//!
//! 识别的段头是 `<control>` / `<global>` / `<master>` / `<group>` / `<region>`，
//! 作用域链 `region → group → master → global`（[`Header::Master`] 是 ARIA 扩展，
//! 见 <https://sfzformat.com/headers/>）。其余段头（`<curve>` / `<effect>` / `<midi>` /
//! `<sample>`）产生 [`Warning::IgnoredHeader`] 并丢弃其 opcode。
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
//! 需要 `#include` 时先解析再解析文本（两步走，保持核心解析器是纯函数）：
//!
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

pub mod error;
pub mod instrument;
pub mod parser;
pub mod playback;
pub mod voice_pool;

pub use error::SfzError;
pub use instrument::{CcGate, Instrument, LoopMode, PlayDirection, Region, RegionQuery, SampleEnd};
pub use parser::{
    Header, IncludeResolver, OpcodeValue, ParseLimits, SfzSource, Warning, parse_f32, parse_int,
    parse_note, parse_sources, parse_text,
};
pub use playback::{
    FALLBACK_SAMPLE_RATE, LoopWindow, PlaybackSpec, RegionPlay, RenderRates, SampleSpan,
};
pub use voice_pool::{
    DEFAULT_VOICE_CAPACITY, MAX_VOICE_CAPACITY, NoteOnOutcome, SILENT_DBFS, STEAL_FADE_FLOOR,
    STEAL_FADE_MILLIS, StealFade, VoiceHandle, VoiceInfo, VoicePool, VoiceStage,
};
