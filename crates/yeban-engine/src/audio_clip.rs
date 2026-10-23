//! **实时**音频片段（`ClipContent::Audio`）的引擎侧适配层（任务单 `#135`）。
//!
//! ## 为什么需要它
//!
//! `EngineSnapshot::from_project` 的 [`crate::snapshot`] 投影此前把
//! `ClipContent::Audio` **整体跳过**（登记原文见 `snapshot.rs` 的
//! `project_schedules` 第 5 条与 `synth.rs` 模块文档 §4）⇒ 实时走带里音频摆放
//! **全程静音**。`#134` 只在**离线**渲染（`yeban-app` 的
//! `render_project_f32_with_assets` → `audio_clip::AudioClipSource`）里接通了它。
//! 本模块把同一条语义搬到**实时侧**，并**不复制**离线实现的任何代码：它只依赖
//! tick → 样本换算（[`crate::synth::samples_per_tick`] /
//! [`crate::synth::tick_to_sample`]，算式与 `round` 的唯一实现自 `#247` 起在
//! [`yeban_model::time_math::ticks_to_frames`]）与轨增益门
//! （[`crate::synth::track_gain_scale`]）。
//!
//! ## 形状（⭐ 本单的核心裁决）
//!
//! ```text
//! 控制线程（构造期，允许分配/读盘/解码）        音频线程（每量子，零分配）
//! ─────────────────────────────────────       ─────────────────────────────
//! project.tracks[].clips[]                     transport.position_frames()
//!   │  muted ⇒ 跳过                                   │  绝对帧，走带唯一时钟
//!   ├─ start/end（tick → 帧，四舍五入）               │
//!   ├─ gain = clip_gain_db × 轨增益门                 ▼
//!   └─ asset 哈希 ──► [AudioClipSpan]        mix_into(schedule, block_start, out)
//!                         │                     span.start ≤ 绝对帧 < span.end
//!                         │                       ⇒ 素材帧 = 绝对帧 − span.start
//!                         │                                   + span.source_offset
//!                         └► Option<Arc<AudioPcm>>  ⇒ out[frame] += pcm[素材帧] × gain
//! ```
//!
//! 三个决定：
//!
//! 1. **PCM 进快照**（`Arc<[f32]>`，交织 f32）：实时回调里**不读文件、不解码**
//!    ⇒ 零 I/O 与零分配都按构造成立。`AudioPcm` 自己是 32 B（`Arc<[f32]>` 16 ＋
//!    `frames` 8 ＋ `sample_rate` 4 ＋ `channels` 2 ＋ 2 填充），每个 span 只多一个
//!    **8 B 的细指针**（`Option<Arc<AudioPcm>>`，`None` 走 niche）；PCM 本体按
//!    `4 B / 帧`计，同一份素材被多个摆放共享（构造期一次 `Arc` 克隆）。
//! 2. **没有逐样本游标**：`span.start` 是**绝对帧**，播放位置由走带的绝对帧
//!    **导出** ⇒ [`crate::rt::EngineRuntime`] **一个字节都不增长**（判据见
//!    `tests/rt_new_boxed.rs` 的 180144 B 与 `tests/sfz_instrument.rs` 的 S9）。
//!    定位（`SeekTicks`）/暂停恢复/换快照因此**自动正确**：不存在会陈旧的游标。
//! 3. **单声道口径**：实时的逐轨暂存是**单声道、声相之前**
//!    （[`crate::rt`] 的 `track_scratch`）⇒ 单声道素材直读，立体声素材取
//!    `(L + R) / 2`（与离线 `StereoToMono` 同式），其余声道数在注入期**拒绝**并
//!    如实报错（⛔ 不悄悄丢声道）。
//!
//! ## 本模块**没有**做的（如实登记）
//!
//! - **不重采样**：素材采样率与快照不一致 ⇒ [`AudioClipError::SampleRateMismatch`]，
//!   注入被拒（与 `yeban-app` 离线路径"采样率不符就跳过并报告"同一条纪律；
//!   重采样是 `yeban-decode` 的活，引擎⛔ 不引它）。
//! - **不展开 `loop_config`**：一个摆放只播一遍（与 `synth.rs` 模块文档 §4 的
//!   "没有循环片段展开"同一条口径）。
//! - ⭐ ⭐ **追加修订段（任务单 `#246`，台账第 278 条：⛔ 不改上一条原文）**：上一条
//!   描述的是 `#246` 之前的事实，现仍逐字保留。自 `#246` 起循环语义已由
//!   [ADR-0010] D1–D3 落定并**下移到 `yeban-model`**（`ClipPlacement::loop_repeat` /
//!   `note_copies`）；MIDI 音符一侧已由 [`crate::snapshot`] 按它展开。⚠ **本模块的
//!   音频路径仍不展开** —— [ADR-0010] D4 的 `source_offset`（按遍指向素材内的循环
//!   区间）引擎侧不存在，音频循环是二期工作，不是本票的语义。
//! - ⭐ ⭐ **追加修订段（任务单 `#249` / `[ADR-0010 D4]`：⛔ 不改上两条原文）**：上一条
//!   描述的是 `#249` 之前的事实，现仍逐字保留。自 `#249` 起 `AudioClipSpan` 带
//!   [`AudioClipSpan::source_offset`]，[`project_audio_clips`] 用**与 MCP、app 同一份**
//!   [`yeban_model::ClipPlacement::loop_repeat`] 按遍展开，[`mix_into`] 读
//!   `素材帧 = 绝对帧 − span.start + span.source_offset` ⇒ 实时监听里音频片段**真的循环**。
//!   非循环 / `end >= duration` / `loop_config.enabled == false` 的工程仍只产生一个
//!   `source_offset == 0` 的 span ⇒ 与展开之前**逐位相同**。
//! - ✅ **声相在 `pan == 0` 时与离线路径同一条**（任务单 `#300` 修好；修前差 −3.01 dB）：
//!   离线 `yeban-app::audio_clip` 在 `pan == 0.0` 时用 `(gain, gain)`（单位增益），
//!   引擎的**逐轨混音**现在对**所有**音源（含 MIDI 与音频片段）也走
//!   [`centered_pan_gains`](crate::mixer::centered_pan_gains) ⇒ `pan == 0` 逐位 `(1, 1)`，
//!   `pan != 0` 仍走等功率曲线（与修前逐位相同）。修前的 −3.0103 dB 差由
//!   `crates/yeban-app/tests/neutral_pan_leg.rs` 量出并已在本单对齐（`#299` 登记）。
//! - **不读盘**：`from_project` 只能把摆放投影成"带资产哈希、**没有** PCM 的 span"
//!   （与 SFZ 的 `PendingSfz` 同款分工）⇒ 控制侧拿到解码后的 PCM 后调
//!   [`crate::snapshot::EngineSnapshot::with_audio_asset`] 注入。
//!
//! ## 实时约束（[MUST-GATE-001] / AGENTS.md 红线 7）
//!
//! [`mix_into`] 里只有整数比较、切片下标与乘加：**⛔ 无分配、⛔ 无释放、⛔ 无锁、
//! ⛔ 无 I/O、⛔ 无超越函数**。运行期由 `tests/rt_zero_alloc.rs` 的音频片段场景
//! （四元组读数）与 `tests/audio_clip_render.rs` 的判据钉住。

use std::collections::BTreeMap;
use std::sync::Arc;

use thiserror::Error;
use yeban_model::{AssetHash, ClipContent, EntityId, YebanProjectV1};

use crate::synth::{samples_per_tick, tick_to_sample, track_gain, track_gain_scale};

/// 实时音频片段的**注入/形状**错误（全部发生在**控制线程**的构造期）。
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum AudioClipError {
    /// 素材采样率与快照采样率不一致（引擎不重采样，⛔ 也不改标签）。
    #[error(
        "audio PCM sample rate {pcm_rate} Hz does not match the snapshot rate {snapshot_rate} Hz"
    )]
    SampleRateMismatch {
        /// 素材自身的采样率 (Hz)。
        pcm_rate: u32,
        /// 快照（工程）的采样率 (Hz)。
        snapshot_rate: u32,
    },
    /// 声道数不是 1（单声道）或 2（立体声）。
    #[error("audio PCM channel count {0} is neither 1 (mono) nor 2 (stereo)")]
    UnsupportedChannels(u16),
    /// 交织样本数不是声道数的整数倍（半帧 ⇒ 长度公式不成立）。
    #[error("audio PCM has {samples} interleaved samples, not a multiple of {channels} channels")]
    RaggedInterleaved {
        /// 交织样本总数。
        samples: usize,
        /// 声道数。
        channels: u16,
    },
}

/// 一份**已解码、已归一化**的实时素材 PCM（交织 `f32`）。
///
/// 由**控制线程**在构造期建好，经
/// [`EngineSnapshot::with_audio_asset`](crate::snapshot::EngineSnapshot::with_audio_asset)
/// 注入快照；音频线程只**读**它（`Arc<[f32]>` 的 `Deref`，不改引用计数 ⇒ 零分配）。
#[derive(Clone, Debug, PartialEq)]
pub struct AudioPcm {
    /// 交织样本（`[-1, 1]` 归一化；声道布局见 [`Self::channels`]）。
    samples: Arc<[f32]>,
    /// 素材原始声道数（本模块只接受 1 / 2，见 [`AudioClipError::UnsupportedChannels`]）。
    channels: u16,
    /// 素材采样率 (Hz)——必须等于快照采样率（[`AudioClipError::SampleRateMismatch`]）。
    sample_rate: u32,
    /// 帧数（＝ `samples.len() / channels`，构造期算一次）。
    frames: u64,
}

impl AudioPcm {
    /// 包装一份交织 PCM。
    ///
    /// # Errors
    ///
    /// - [`AudioClipError::UnsupportedChannels`]：`channels` 不是 1 或 2；
    /// - [`AudioClipError::RaggedInterleaved`]：`samples.len()` 不是 `channels` 的整数倍。
    ///
    /// ⚠ **不**在构造期比对采样率（那需要快照的采样率）—— 那条比对在
    /// [`EngineSnapshot::with_audio_asset`](crate::snapshot::EngineSnapshot::with_audio_asset)
    /// 里做。
    pub fn new(
        samples: Arc<[f32]>,
        channels: u16,
        sample_rate: u32,
    ) -> Result<Self, AudioClipError> {
        if channels != 1 && channels != 2 {
            return Err(AudioClipError::UnsupportedChannels(channels));
        }
        let channels_usize = usize::from(channels);
        if !samples.len().is_multiple_of(channels_usize) {
            return Err(AudioClipError::RaggedInterleaved {
                samples: samples.len(),
                channels,
            });
        }
        let frames = u64::try_from(samples.len() / channels_usize).unwrap_or(u64::MAX);
        Ok(Self {
            samples,
            channels,
            sample_rate,
            frames,
        })
    }

    /// 交织样本（音频线程只读它）。
    #[must_use]
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// 素材声道数（1 或 2）。
    #[must_use]
    pub const fn channels(&self) -> u16 {
        self.channels
    }

    /// 素材采样率 (Hz)。
    #[must_use]
    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// 帧数（`samples.len() / channels`）。
    #[must_use]
    pub const fn frames(&self) -> u64 {
        self.frames
    }
}

/// 一个音频摆放投影成的**绝对帧区间 + 增益 + 素材**。
///
/// `pcm` 为 `None` 表示"控制侧还没注入这份素材的 PCM" ⇒ 实时侧**跳过**该 span
/// （与 SFZ 的 `PendingSfz` 同一个分工：快照携带事实，重数据由控制侧补）。
#[derive(Clone, Debug, PartialEq)]
pub struct AudioClipSpan {
    /// 起始帧（含；绝对帧 = 走带 `position_frames` 的同一根时钟）。
    start: u64,
    /// 结束帧（不含；恒 `> start`）。
    end: u64,
    /// 单声道增益（`clip_gain_db → 线性` × 轨增益门；声相由逐轨混音的后续一级施加）。
    gain: f32,
    /// 内容寻址的素材哈希（注入时按它匹配）。
    asset: AssetHash,
    /// 已注入的 PCM（共享；`None` ＝ 尚未注入 ⇒ 静音）。
    pcm: Option<Arc<AudioPcm>>,
    /// 本 span 从素材的**第几帧**开始读（`#249` / `[ADR-0010 D4]`）。
    ///
    /// 非循环摆放恒为 `0` ⇒ 与加循环展开之前**逐位相同**；循环展开后每一遍都从
    /// 循环区间起点对应的素材帧读（见 [`project_audio_clips`]）。
    source_offset: u64,
}

impl AudioClipSpan {
    /// 起始帧（含）。
    #[must_use]
    pub const fn start(&self) -> u64 {
        self.start
    }

    /// 结束帧（不含）。
    #[must_use]
    pub const fn end(&self) -> u64 {
        self.end
    }

    /// 单声道增益（声相之前）。
    #[must_use]
    pub const fn gain(&self) -> f32 {
        self.gain
    }

    /// 引用的素材哈希。
    #[must_use]
    pub const fn asset(&self) -> &AssetHash {
        &self.asset
    }

    /// 本 span 从素材的第几帧开始读（`#249`；非循环恒为 `0`）。
    #[must_use]
    pub const fn source_offset(&self) -> u64 {
        self.source_offset
    }

    /// 已注入的 PCM（`None` ＝ 尚未注入）。
    #[must_use]
    pub fn pcm(&self) -> Option<&AudioPcm> {
        self.pcm.as_deref()
    }

    /// 本 span 是否有 PCM（判据用来数"真的可发声的摆放"）。
    #[must_use]
    pub const fn has_pcm(&self) -> bool {
        self.pcm.is_some()
    }
}

/// 一条轨道的音频摆放表（`BTreeMap` 的值；⛔ 不含路由/电平语义）。
///
/// `spans` 已按 `(start, end, gain 位型)` 全序排好（[ARCH-DET-002]：浮点加法不满足
/// 结合律 ⇒ 同一素材的多个摆放必须有**确定的**累加顺序，与离线路径同一条口径）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioClipSchedule {
    /// span（一个摆放可以展开成多遍；非循环摆放是 1:1）。
    spans: Vec<AudioClipSpan>,
    /// 产生这些 span 的**摆放数**（`#249` 的读数口径，与 [`Self::len`] 分开）。
    ///
    /// 循环展开后一个摆放 ⇒ N 个 span；报告里的"摆放数"仍读这个字段，
    /// ⛔ 不能拿 `spans.len()` 冒充（`#241` 在 MCP 的 `audioClips` 上踩过同一坑）。
    placements: usize,
}

impl AudioClipSchedule {
    /// 全部 span（全序）。
    #[must_use]
    pub fn spans(&self) -> &[AudioClipSpan] {
        &self.spans
    }

    /// **span 条数**（循环展开后可能多于摆放数）。
    ///
    /// ⚠ 与 [`Self::placements`] 是两个口径：前者数的是搬进混音的区间，
    /// 后者数的是工程里的摆放。
    #[must_use]
    pub fn len(&self) -> usize {
        self.spans.len()
    }

    /// 产生本表全部 span 的**摆放数**（`#249`；⛔ 不随循环展开增长）。
    #[must_use]
    pub const fn placements(&self) -> usize {
        self.placements
    }

    /// 是否一个摆放都没有。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// 已注入 PCM 的摆放条数。
    #[must_use]
    pub fn pcm_spans(&self) -> usize {
        self.spans.iter().filter(|span| span.has_pcm()).count()
    }

    /// 把 `asset` 匹配的摆放接上 `pcm`（**控制线程**；`Arc` 克隆只在这里发生）。
    ///
    /// 返回**真的接上**的摆放条数（判据/报告读它，⛔ 不静默）。
    pub fn attach_pcm(&mut self, asset: &AssetHash, pcm: &Arc<AudioPcm>) -> usize {
        let mut attached = 0;
        for span in &mut self.spans {
            if span.asset == *asset {
                span.pcm = Some(Arc::clone(pcm));
                attached += 1;
            }
        }
        attached
    }
}

/// 把工程的 `ClipContent::Audio` 摆放投影成"每轨一份绝对帧区间表"（**控制线程**）。
///
/// 口径逐条对齐 `yeban-app::audio_clip` 的离线实现：
///
/// | 维度 | 口径 |
/// | :--- | :--- |
/// | 帧区间 | `[tick_to_sample(start), tick_to_sample(start + max(duration, 1)))`，四舍五入，终点至少 `start + 1` |
/// | 循环展开 | 用**模型层唯一那份** [`yeban_model::ClipPlacement::loop_repeat`]（`[ADR-0010 D4]`）：一个摆放展开成 pickup（若有）＋ N 遍循环区间，每遍的 [`AudioClipSpan::source_offset`] 指向素材里的循环区间起点 |
/// | 硬切 | 摆放时值之外不加（素材更长就截断、更短就留静音尾巴）；每一遍裁到循环区间末端 |
/// | 增益 | `track_gain(clip.gain_db) × track_gain_scale(mute, solo, solo_safe, any_solo, volume_db)`（**单一来源**：与声部槽同一函数） |
/// | 静音摆放 | `placement.muted` ⇒ 不进表（与离线一致） |
/// | 排序 | `(start, end, source_offset, gain 位型)` 全序 ⇒ 累加顺序确定 [ARCH-DET-002] |
/// | 声道 | 单声道直读；立体声 `(L+R)/2`（见 [`mix_into`]） |
/// | 读数 | [`AudioClipSchedule::len`] = span 数；[`AudioClipSchedule::placements`] = **摆放数**（⛔ 两个口径不混） |
///
/// ⚠ **只收录非空的轨**：一条音频摆放都没有的工程得到**空表** ⇒ 实时侧整段跳过
/// ⇒ 输出与接线前**逐位相同**。
#[must_use]
pub fn project_audio_clips(project: &YebanProjectV1) -> BTreeMap<EntityId, AudioClipSchedule> {
    let sample_rate = project.audio_config.sample_rate.hz();
    let per_tick = samples_per_tick(project.bpm, sample_rate);
    let any_solo = project.tracks.values().any(|track| track.solo);
    let mut schedules: BTreeMap<EntityId, AudioClipSchedule> = BTreeMap::new();

    for (id, track) in &project.tracks {
        let track_gain_scale = track_gain_scale(
            track.mute,
            track.solo,
            track.solo_safe,
            any_solo,
            track.volume_db,
        );
        let mut spans: Vec<AudioClipSpan> = Vec::new();
        let mut placements = 0usize;
        for placement in track.clips.values() {
            if placement.muted {
                continue;
            }
            let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
                continue;
            };
            let ClipContent::Audio { asset, gain_db } = &entry.content else {
                continue;
            };
            let Some(start) = tick_to_sample(placement.start_tick, per_tick) else {
                continue;
            };
            let end_tick = placement
                .start_tick
                .saturating_add(placement.duration_ticks.max(1));
            let Some(end) = tick_to_sample(end_tick, per_tick) else {
                continue;
            };
            // 终点至少 `start + 1`：与离线 `clip_frame_span` 同一条守卫。
            let end = end.max(start.saturating_add(1));
            // 非有限增益按 `0` 处理（[`track_gain`] 的口径）。⚠ 已校验的工程走不到这里：
            // `ClipContent::validate` 拒绝非有限增益 —— 这条只是"不 panic、不发疯"的兜底。
            let gain = track_gain(*gain_db) * track_gain_scale;
            let push =
                |spans: &mut Vec<AudioClipSpan>, start: u64, end: u64, source_offset: u64| {
                    spans.push(AudioClipSpan {
                        start,
                        end,
                        gain,
                        asset: asset.clone(),
                        pcm: None,
                        source_offset,
                    });
                };
            match placement.loop_repeat() {
                // `None` ＝ 不循环（含 `end >= duration` / 关掉 / 非法区间）⇒
                // 单 span、`source_offset = 0` ⇒ 逐位等于加循环展开之前。
                None => push(&mut spans, start, end, 0),
                Some(repeat) => {
                    // `[ADR-0010 D4]`：与 MCP 的 `render.rs` 逐条同式 —— 第一遍之前
                    // 的内容（pickup）原样播一遍；循环区间 `[start, end)` 的每一遍
                    // 都从素材的 `region_start - base_start` 帧开始读。
                    let region_start_tick = placement.start_tick.saturating_add(repeat.start_tick);
                    let region_end_tick = placement.start_tick.saturating_add(repeat.end_tick);
                    let Some(region_start) = tick_to_sample(region_start_tick, per_tick) else {
                        push(&mut spans, start, end, 0);
                        placements = placements.saturating_add(1);
                        continue;
                    };
                    let Some(region_end) = tick_to_sample(region_end_tick, per_tick) else {
                        push(&mut spans, start, end, 0);
                        placements = placements.saturating_add(1);
                        continue;
                    };
                    let period = region_end.saturating_sub(region_start);
                    let base_end = tick_to_sample(
                        placement
                            .start_tick
                            .saturating_add(placement.duration_ticks),
                        per_tick,
                    )
                    .unwrap_or(end);
                    if period == 0 {
                        // tick 太短、帧换算后不足一帧 ⇒ 没有可重复的区间，
                        // 按单遍播放（不静默丢内容；与 MCP 同一条守卫）。
                        push(&mut spans, start, end, 0);
                    } else {
                        if region_start > start {
                            // 区间之前的内容只播一遍（素材从 0 开始）。
                            push(&mut spans, start, region_start, 0);
                        }
                        let source_offset = region_start.saturating_sub(start);
                        let mut iteration = 0u64;
                        loop {
                            let span_start =
                                region_start.saturating_add(iteration.saturating_mul(period));
                            if span_start >= base_end || iteration >= repeat.iterations {
                                break;
                            }
                            let span_end = span_start.saturating_add(period).min(base_end);
                            push(&mut spans, span_start, span_end, source_offset);
                            iteration = iteration.saturating_add(1);
                        }
                    }
                }
            }
            placements = placements.saturating_add(1);
        }
        if spans.is_empty() {
            continue;
        }
        // [ARCH-DET-002] 固定全序（与离线 `sort_spans` 同一个 key 序列）。
        spans.sort_by(|a, b| {
            a.start
                .cmp(&b.start)
                .then(a.end.cmp(&b.end))
                .then(a.source_offset.cmp(&b.source_offset))
                .then(a.gain.to_bits().cmp(&b.gain.to_bits()))
        });
        schedules.insert(*id, AudioClipSchedule { spans, placements });
    }

    schedules
}

/// 把一个量子内的音频摆放**累加**进单声道逐轨暂存（**实时路径**）。
///
/// `block_start` 是本量子第一帧的**绝对帧**（走带 `position_frames`，推进之前读）。
/// `out` 是**已经清过/已含 MIDI 基座**的单声道缓冲（本函数只做 `+=`）。
///
/// # 实时约束
///
/// 只有整数比较、切片下标与乘加：**⛔ 无分配、⛔ 无释放、⛔ 无锁、⛔ 无 I/O、
/// ⛔ 无超越函数** [MUST-GATE-001]。没有注入 PCM 的 span 与块外的 span **整段跳过**。
///
/// # 声道口径
///
/// * 单声道素材：逐帧直读；
/// * 立体声素材：`(L + R) × 0.5`（与离线 `StereoToMono` 同式；实时的逐轨暂存是单声道）。
pub fn mix_into(schedule: &AudioClipSchedule, block_start: u64, out: &mut [f32]) {
    if schedule.spans.is_empty() || out.is_empty() {
        return;
    }
    let Ok(block_len) = u64::try_from(out.len()) else {
        return;
    };
    let block_end = block_start.saturating_add(block_len);
    for span in &schedule.spans {
        let Some(pcm) = span.pcm.as_deref() else {
            continue;
        };
        if span.end <= block_start || span.start >= block_end {
            continue;
        }
        let frames = pcm.frames();
        if frames == 0 {
            continue;
        }
        let from = span.start.max(block_start);
        let to = span.end.min(block_end);
        let channels = usize::from(pcm.channels);
        let samples = pcm.samples();
        for frame in from..to {
            // `#249` / `[ADR-0010 D4]`：循环展开后每一遍从素材的 `source_offset`
            // 开始读；非循环 span 的该字段恒为 `0` ⇒ 与展开之前逐位相同。
            let asset_frame = frame.saturating_sub(span.start) + span.source_offset;
            if asset_frame >= frames {
                // 素材比摆放区间短 ⇒ 之后的帧是静音尾巴（不是"继续读越界"）。
                break;
            }
            let Ok(asset_frame) = usize::try_from(asset_frame) else {
                break;
            };
            let index = asset_frame.saturating_mul(channels);
            let value = if channels == 1 {
                samples[index]
            } else {
                (samples[index] + samples[index + 1]) * 0.5
            };
            let offset = frame - block_start;
            let Ok(offset) = usize::try_from(offset) else {
                break;
            };
            out[offset] += value * span.gain;
        }
    }
}
