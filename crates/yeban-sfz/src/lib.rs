//! # yeban-sfz — SFZ v2 采样器
//!
//! 零拷贝 SFZ v2 词法 / 结构解析器 + 预分配 voice pool（默认 512 声部、可配 1024）
//! [ARCH-RT-004]。解析器是不可信输入边界：必须能承受 `cargo-fuzz` 千万次变异零崩溃
//! [MUST-GATE-011]。
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
//! ## 与实时引擎的接口（**预留，Pending**）
//!
//! 本 crate 已经能解析乐器（[`parse_text`] → [`Instrument::region_for`]）并持有预分配的
//! [`VoicePool`]（默认 [`DEFAULT_VOICE_CAPACITY`] / 上限 [`MAX_VOICE_CAPACITY`]，
//! 含 [`StealFade`] 的 3 ms 淡出）。但**还没有任何调用方**把它接到音源上，
//! 因为缺少三个前提（全部登记在 `docs/ledger/engine-sound-notes.md` §5.3 与 needs N6）：
//!
//! ```text
//! 1) 采样数据解码（WAV/FLAC）—— 需要依赖裁决（symphonia/hound）或自研解码器；
//! 2) 重采样（region 的 keycenter/tune/采样率 ≠ 工程采样率）—— 需要 rubato 或自研；
//! 3) 数据本身：assets/samples/ 目前**只有** ATTRIBUTION.md，且 AGENTS.md §2 红线 9
//!    要求样本在 assets/manifest.json 登记许可证 + SHA-256。
//! ```
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
pub mod voice_pool;

pub use error::SfzError;
pub use instrument::{CcGate, Instrument, LoopMode, Region, RegionQuery};
pub use parser::{
    Header, IncludeResolver, OpcodeValue, ParseLimits, SfzSource, Warning, parse_f32, parse_int,
    parse_note, parse_sources, parse_text,
};
pub use voice_pool::{
    DEFAULT_VOICE_CAPACITY, MAX_VOICE_CAPACITY, NoteOnOutcome, SILENT_DBFS, STEAL_FADE_FLOOR,
    STEAL_FADE_MILLIS, StealFade, VoiceHandle, VoiceInfo, VoicePool, VoiceStage,
};
