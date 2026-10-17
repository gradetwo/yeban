//! ⭐ **控制侧音频片段注入**（任务单 `#136`）：把 `ClipContent::Audio` 的**真实字节**
//! 解码成引擎的 [`AudioPcm`]，按 `AssetHash` 缓存，并交给
//! [`EngineSnapshot::with_audio_asset`]。
//!
//! # 它补的是哪一格
//!
//! `#135` 让实时引擎**消费**音频摆放（`yeban-engine/src/audio_clip.rs`），但那条路的
//! 分工是"快照只搬轻数据（区间 ＋ 增益 ＋ 素材哈希），重数据 PCM 由控制侧注入"。
//! `#135` 只交付了引擎侧那一半：`EngineSnapshot::from_project` 建出来的音频摆放
//! `pcm == None` ⇒ 走带**逐位静音**。本模块补控制侧：
//!
//! ```text
//! project.clip_pool 的 ClipContent::Audio{ asset }
//!   ──▶ AudioAssetSource（会话资产池，只读）        // 未命中缓存时才调用
//!   ──▶ crate::audio_clip::decode_asset_bytes       // ⭐ #134 的同一条解码器（⛔ 不新写）
//!   ──▶ 素材率 ≠ 快照率 ⇒ yeban_decode::resample_interleaved   // ⭐ #138：rubato sinc
//!   ──▶ AudioPcm::new(...)                          // 引擎形状（32 B，见 #135；率 = 快照率）
//!   ──▶ EngineSnapshot::with_audio_asset(&asset, pcm)  // 引擎仍保留拒绝（它拒绝不符是正确行为）
//! ```
//!
//! # 四个决定
//!
//! 1. **PCM 不在工程模型里**（模型只有 `AssetHash`）⇒ **每个修订都要重新注入**。
//!    因此这里有一份**按 `AssetHash` 键控的解码缓存**：同一份素材在后续修订里只做
//!    一次 `Arc` 克隆（`AudioPcm::clone`），⛔ 不重复解码。读数见 [`AudioCacheReadings`]
//!    （`decodes` / `hits` / `fetches` / `resamples`）。
//!    ⚠ 缓存值里记着**目标采样率**（注入时传进来的快照率）：工程换了采样率 ⇒ 该条目
//!    **作废并重解码＋重采样**（⛔ 不把上一工程的率当成这一工程的率）。
//! 2. **失效与上限**（⛔ 不是无限增长）：
//!    - **按需回收**：每次注入先调 [`AudioPcmCache::retain_live`]，只保留"当前工程
//!      `clip_pool` 里还引用的 `ClipContent::Audio` 哈希" ⇒ 删掉片段/换工程后旧 PCM
//!      在**下一修订**就从缓存里掉出去（快照自己还持有的那份 `Arc` 由引擎的退役队列
//!      回收，⛔ 不由本模块释放）；
//!    - **硬上限**：[`MAX_CACHED_ASSETS`] 个条目 ＋ [`MAX_CACHED_BYTES`] 字节**解码后
//!      样本**（`4 B / f32`）；超限按 `last_used`（注入修订序号）淘汰最久未用的；
//!      单个素材**自己**就超过字节上限 ⇒ 本次修订照常注入，但**不进缓存**
//!      （`oversize_uncached` 计数）⇒ 下次修订会重解码 —— 这是有意的：宁可慢，也不让
//!      一份超大素材把缓存上限撑破。
//! 3. **采样率不符 ⇒ 控制侧重采样**（任务单 `#138`）：解码出的 `sample_rate` 与快照率
//!    （`project.audio_config.sample_rate` 的投影）不一致时，在**控制线程**上用
//!    [`yeban_decode::resample_interleaved`]（`rubato` sinc，[ARCH-DSP-002] / D26 的
//!    唯一实现）转到快照率，再 `with_audio_asset` 注入。⛔ app 侧不另写第二份重采样器；
//!    ⛔ 引擎侧一个字不改（它继续拒绝不符的 PCM —— 那是它的正确行为）。
//!    输出帧数被钉到 `ceil(原帧数 × 目标率 / 源率)`（时长口径，见 [`resample_to_target`]）。
//!    ⚠ **本单只做采样率转换**：声道数 ≠ 1,2 仍**跳过**（⛔ 不悄悄丢声道）。
//! 4. **跳过必须可见**（⛔ 不静默）：没有字节来源 / 资产池里没有字节 / SHA-256 不符 /
//!    声道数 ≠ 1,2 / 解码或重采样失败 ⇒ 该素材**不注入**，一条逐字原因进
//!    [`AudioClipSkip`]，并由 `EngineHost` 打到 stderr（`[yeban-app] 音频片段注入跳过: …`）。
//!
//! # 线程
//!
//! 全部方法都在**控制线程**上（`EngineHost::reload` / `EngineHost::publish_project`）：
//! 读字节、解码、分配都允许。音频线程只读快照里已经注入的 `Arc<[f32]>`
//! [MUST-GATE-001]；本模块**不在**任何回调路径上。
//!
//! # 与离线路径的关系
//!
//! 解码**复用** [`crate::audio_clip`] 的 `decode_wav_bytes`（同一个
//! `yeban_render::wav::read_plain_wav`、同一张归一化表）；本模块只多了"按哈希缓存"、
//! "必要时重采样"与"把结果搬进引擎形状"三件事。⛔ 没有第二份 WAV 解析器、
//! ⛔ 没有第二份重采样器、⛔ 没有新依赖（`yeban-decode` 早在 `#85` 就是本 crate 的依赖）。
//!
//! ⭐ **离线导出那条支路也重采样**（任务单 `#141`）：`crate::audio_clip` 的
//! `decode_checked` 现在调用**本模块的** [`resample_to_target`]（同一个函数、同一份
//! `yeban_decode::resample_interleaved`、同一条"帧数钉到 `ceil(原帧 × 目标 / 源)`"口径），
//! ⛔ 没有第二份重采样器。因此 `#138`（实时注入侧）与 `#141`（离线导出侧）在采样率换算上
//! **是同一份实现**；离线侧仍不能处理的形态（声道无法映射 / 坏 WAV / 重采样失败）照旧
//! **响亮跳过**，⛔ 不静默少一段。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use yeban_engine::audio_clip::AudioPcm;
use yeban_model::AssetHash;

/// 资产字节的**只读来源**（产品 = 会话 CAS 池；判据 = 自己的夹具表）。
///
/// 只在缓存未命中时被调用；返回 `None` = 池里没有这份字节（会进跳过报告，⛔ 不静默）。
pub type AudioAssetSource = Box<dyn Fn(&AssetHash) -> Option<Vec<u8>>>;

/// 缓存**条目数**上限（不是素材数：一个素材一个条目）。
pub const MAX_CACHED_ASSETS: usize = 64;

/// 缓存**解码后样本字节数**上限（`4 B / f32`）。
///
/// 256 MiB ≈ 6700 万帧 ≈ 48 kHz 立体声 **23 分钟**；超过它的单份素材不常驻缓存。
pub const MAX_CACHED_BYTES: u64 = 256 * 1024 * 1024;

/// 解码缓存的读数（控制线程；⛔ 不进实时线程）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AudioCacheReadings {
    /// **进入"字节 ⇒ PCM"那一步**的次数（缓存未命中且拿到了字节）。
    ///
    /// ⚠ 哈希校验失败也计一次（它确实进了那一步、只是被校验挡在解码之前）；判据
    /// `decodes == 1` 数的是"同一素材在两个摆放上只走一次这条路"。
    pub decodes: u64,
    /// 从缓存直接命中的次数（`Ready` 条目）。
    pub hits: u64,
    /// 向 [`AudioAssetSource`] 要字节的次数（只有未命中才会发生）。
    pub fetches: u64,
    /// **控制侧重采样**的次数（素材率 ≠ 快照率且真的转换成功）。
    ///
    /// 按"每素材每目标率一次"计：同一率的后续修订命中缓存 ⇒ **不增加**；
    /// 工程换了采样率 ⇒ 旧条目作废，下一次未命中会再计一次。
    pub resamples: u64,
    /// 因"当前工程不再引用它"而回收的条目数（失效）。
    pub evicted_stale: u64,
    /// 因条目数/字节数上限而淘汰的条目数（LRU）。
    pub evicted_pressure: u64,
    /// 单份素材**自身**超过 [`MAX_CACHED_BYTES`] ⇒ 本次注入但不缓存 的次数。
    pub oversize_uncached: u64,
    /// 此刻缓存的条目数。
    pub cached_assets: usize,
    /// 此刻缓存的解码后样本字节数。
    pub cached_bytes: u64,
}

/// 一个**真的接上**的素材（每素材一条）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioClipInjection {
    /// 素材哈希。
    pub asset: AssetHash,
    /// **注入 PCM** 的帧数（`samples.len() / channels`；重采样后）。
    pub frames: u64,
    /// 素材声道数（1 或 2）。
    pub channels: u16,
    /// **注入 PCM** 的采样率 (Hz)——必等于快照采样率。
    pub sample_rate: u32,
    /// 素材**容器声明**的采样率 (Hz)——重采样前（`resampled == false` 时与
    /// [`Self::sample_rate`] 相同）。
    pub source_sample_rate: u32,
    /// 重采样**前**的帧数（源率下的帧数；`resampled == false` 时等于 [`Self::frames`]）。
    pub source_frames: u64,
    /// `true` = 本素材在控制侧被重采样到快照率
    /// （`source_sample_rate != sample_rate`）。
    pub resampled: bool,
    /// 本次修订里被接上 PCM 的摆放条数。
    pub spans_attached: usize,
    /// `true` = 命中缓存（本次修订**没有**解码／重采样）。
    pub from_cache: bool,
}

/// 一个**被跳过**的素材（逐字原因；⛔ 不静默）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioClipSkip {
    /// 素材哈希（`AssetHash` 的规范文本）。
    pub asset: String,
    /// 逐字原因（含引擎错误原文 / 字节缺失 / SHA-256 不符）。
    pub reason: String,
}

/// 一次修订的**注入报告**（`EngineHost::audio_clip_report()` 交出最近一份）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioClipReport {
    /// 这份报告对应的快照修订号。
    pub revision: u64,
    /// 注入成功的素材数。
    pub injected_assets: usize,
    /// 注入成功的素材里**被重采样**的个数（`decoded.sample_rate != snapshot_rate`）。
    pub resampled_assets: usize,
    /// 接上 PCM 的摆放条数（全快照合计）。
    pub spans_attached: usize,
    /// 逐素材读数。
    pub assets: Vec<AudioClipInjection>,
    /// 逐素材跳过原因。
    pub skipped: Vec<AudioClipSkip>,
    /// 本次修订之后的缓存读数（快照，不是活引用）。
    pub cache: AudioCacheReadings,
}

impl AudioClipReport {
    /// 一行摘要（日志与判据共用同一份文本）。
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "音频片段注入: revision {} 注入 {} 个素材（其中重采样 {} 个）/ {} 条摆放 / 跳过 {} 个；\
             缓存 decodes={} hits={} fetches={} resamples={} cached_assets={} cached_bytes={}",
            self.revision,
            self.injected_assets,
            self.resampled_assets,
            self.spans_attached,
            self.skipped.len(),
            self.cache.decodes,
            self.cache.hits,
            self.cache.fetches,
            self.cache.resamples,
            self.cache.cached_assets,
            self.cache.cached_bytes,
        )
    }

    /// 逐素材的读数行（`frames` / `channels` / `sample_rate` / 重采样前后；日志用）。
    #[must_use]
    pub fn asset_lines(&self) -> Vec<String> {
        self.assets
            .iter()
            .map(|asset| {
                format!(
                    "音频片段注入素材: {} frames={} channels={} sample_rate={} \
                     source_sample_rate={} source_frames={} resampled={} spans={} from_cache={}",
                    asset.asset,
                    asset.frames,
                    asset.channels,
                    asset.sample_rate,
                    asset.source_sample_rate,
                    asset.source_frames,
                    asset.resampled,
                    asset.spans_attached,
                    asset.from_cache,
                )
            })
            .collect()
    }

    /// 什么都没发生（空工程 / 没有音频片段）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.assets.is_empty() && self.skipped.is_empty()
    }
}

/// 缓存里的**已注入形状**（PCM 的采样率 = 建它时那个工程的目标率）。
struct ReadyAudio {
    /// 引擎形状的 PCM（`Arc` 克隆只在这里发生）。
    pcm: AudioPcm,
    /// 解码出的素材采样率（重采样前）。
    source_rate: u32,
    /// 解码出的帧数（重采样前）。
    source_frames: u64,
    /// 是否在控制侧重采样过。
    resampled: bool,
}

/// 缓存里的值。
enum CachedAudio {
    /// 已建成引擎形状的 PCM（`Arc` 克隆只在这里发生）。
    Ready(ReadyAudio),
    /// **永久形状拒绝**（声道数 ≠ 1,2 / 半帧）：缓存原因文本，不再重复解码。
    ///
    /// ⚠ 采样率不符**不再**进这里（`#138` 起在控制侧重采样）。
    Rejected(String),
}

/// 一个缓存条目。
struct CacheEntry {
    value: CachedAudio,
    /// 解码后样本字节数（`Rejected` = 0）。
    bytes: u64,
    /// 最后被使用的修订序号（LRU 键）。
    last_used: u64,
}

/// 一次缓存查询的结果。
pub(crate) struct AudioLookup {
    /// 可注入的 PCM（采样率 = 目标率）。
    pub(crate) pcm: AudioPcm,
    /// `true` = 命中缓存。
    pub(crate) from_cache: bool,
    /// 素材容器声明的采样率（重采样前）。
    pub(crate) source_sample_rate: u32,
    /// 重采样前的帧数。
    pub(crate) source_frames: u64,
    /// 本素材是否被重采样（`source_sample_rate != pcm.sample_rate()`）。
    pub(crate) resampled: bool,
}

/// 按 `AssetHash` 键控的解码缓存 ＋ 一套可读计数器。
#[derive(Default)]
pub(crate) struct AudioPcmCache {
    entries: BTreeMap<AssetHash, CacheEntry>,
    epoch: u64,
    readings: AudioCacheReadings,
}

impl AudioPcmCache {
    /// 缓存读数（此刻的条目数/字节数也会现读，不靠调用方自己算）。
    pub(crate) fn readings(&self) -> AudioCacheReadings {
        let mut readings = self.readings;
        readings.cached_assets = self.entries.len();
        readings.cached_bytes = self.total_bytes();
        readings
    }

    /// 开始一个新修订（`last_used` 的序号）。
    pub(crate) fn begin_revision(&mut self) {
        self.epoch = self.epoch.saturating_add(1);
    }

    /// **失效**：只保留 `live`（当前工程 `clip_pool` 引用的音频哈希）里的条目。
    pub(crate) fn retain_live(&mut self, live: &BTreeSet<AssetHash>) {
        let before = self.entries.len();
        let mut removed_bytes = 0_u64;
        self.entries.retain(|hash, entry| {
            let keep = live.contains(hash);
            if !keep {
                removed_bytes = removed_bytes.saturating_add(entry.bytes);
            }
            keep
        });
        self.readings.cached_bytes = self.readings.cached_bytes.saturating_sub(removed_bytes);
        self.readings.evicted_stale = self
            .readings
            .evicted_stale
            .saturating_add(u64::try_from(before - self.entries.len()).unwrap_or(0));
    }

    /// 取一份 PCM：命中 ⇒ `Arc` 克隆；未命中 ⇒ 走**同一条** app 侧解码器，
    /// 必要时（`sample_rate != target_rate`）再走**唯一那份**重采样器。
    ///
    /// `target_rate` 是**快照的**采样率（`EngineSnapshot::sample_rate()`）；缓存值按它
    /// 定形 —— 工程换了采样率 ⇒ 旧条目作废（见 [`ReadyAudio`]）。
    ///
    /// # Errors
    ///
    /// 逐字原因（没有字节来源 / 池里没有字节 / SHA-256 不符 / WAV 解码失败 /
    /// 声道形状非法 / 重采样失败）。调用方必须把它记进 [`AudioClipSkip`]，⛔ 不许吞掉。
    pub(crate) fn lookup(
        &mut self,
        hash: &AssetHash,
        source: Option<&AudioAssetSource>,
        target_rate: u32,
    ) -> Result<AudioLookup, String> {
        if let Some(entry) = self.entries.get(hash) {
            match &entry.value {
                CachedAudio::Rejected(reason) => return Err(reason.clone()),
                CachedAudio::Ready(ready) if ready.pcm.sample_rate() == target_rate => {
                    let lookup = AudioLookup {
                        pcm: ready.pcm.clone(),
                        from_cache: true,
                        source_sample_rate: ready.source_rate,
                        source_frames: ready.source_frames,
                        resampled: ready.resampled,
                    };
                    self.readings.hits = self.readings.hits.saturating_add(1);
                    if let Some(entry) = self.entries.get_mut(hash) {
                        entry.last_used = self.epoch;
                    }
                    return Ok(lookup);
                }
                // 目标率变了（工程换了采样率）⇒ 缓存里那份 PCM 属于**另一个工程**的
                // 目标率，⛔ 不冒充：落到下面重走「字节 ⇒ 解码 ⇒ 重采样」。
                CachedAudio::Ready(_) => {}
            }
        }
        if let Some(previous) = self.entries.remove(hash) {
            self.readings.cached_bytes = self.readings.cached_bytes.saturating_sub(previous.bytes);
        }
        let Some(source) = source else {
            return Err(
                "引擎宿主没有接上资产字节来源（`EngineHost::set_audio_asset_source` 未被调用）\
                 ⇒ 控制侧拿不到这份素材的字节"
                    .to_owned(),
            );
        };
        let Some(bytes) = source(hash) else {
            return Err(
                "会话资产池里没有这份字节（工程引用了它，但字节没有到引擎宿主）".to_owned(),
            );
        };
        self.readings.fetches = self.readings.fetches.saturating_add(1);
        self.readings.decodes = self.readings.decodes.saturating_add(1);
        let decoded = crate::audio_clip::decode_asset_bytes(hash, &bytes)
            .map_err(|reason| format!("解码失败: {reason}"))?;
        let (samples, channels, sample_rate) = decoded.into_parts();
        // ⚠ 本单只做采样率转换：>2 声道仍按引擎的形状拒绝（逐字原因同引擎）。
        if channels != 1 && channels != 2 {
            let reason =
                yeban_engine::audio_clip::AudioClipError::UnsupportedChannels(channels).to_string();
            self.store(hash.clone(), CachedAudio::Rejected(reason.clone()), 0);
            return Err(reason);
        }
        let source_frames =
            u64::try_from(samples.len() / usize::from(channels)).unwrap_or(u64::MAX);
        let resampled = sample_rate != target_rate;
        let samples = if resampled {
            let converted = resample_to_target(&samples, channels, sample_rate, target_rate)?;
            self.readings.resamples = self.readings.resamples.saturating_add(1);
            converted
        } else {
            // 同率 ⇒ **原样**搬进 `Arc`（零复制、逐位相同；⛔ 不走滤波）。
            samples
        };
        let pcm = match AudioPcm::new(Arc::from(samples), channels, target_rate) {
            Ok(pcm) => pcm,
            Err(error) => {
                let reason = error.to_string();
                self.store(hash.clone(), CachedAudio::Rejected(reason.clone()), 0);
                return Err(reason);
            }
        };
        let bytes = u64::try_from(pcm.samples().len())
            .unwrap_or(u64::MAX)
            .saturating_mul(4);
        self.store(
            hash.clone(),
            CachedAudio::Ready(ReadyAudio {
                pcm: pcm.clone(),
                source_rate: sample_rate,
                source_frames,
                resampled,
            }),
            bytes,
        );
        Ok(AudioLookup {
            pcm,
            from_cache: false,
            source_sample_rate: sample_rate,
            source_frames,
            resampled,
        })
    }

    /// 缓存总字节数（现算；条目最多 [`MAX_CACHED_ASSETS`] 个）。
    fn total_bytes(&self) -> u64 {
        self.entries.values().map(|entry| entry.bytes).sum()
    }

    /// 插入一个条目并执行上限淘汰。
    fn store(&mut self, hash: AssetHash, value: CachedAudio, bytes: u64) {
        if bytes > MAX_CACHED_BYTES {
            // 单份素材自己就超上限 ⇒ 本次照常用，但⛔ 不常驻（下次会重解码）。
            self.readings.oversize_uncached = self.readings.oversize_uncached.saturating_add(1);
            return;
        }
        if let Some(previous) = self.entries.insert(
            hash,
            CacheEntry {
                value,
                bytes,
                last_used: self.epoch,
            },
        ) {
            self.readings.cached_bytes = self.readings.cached_bytes.saturating_sub(previous.bytes);
        }
        self.readings.cached_bytes = self.readings.cached_bytes.saturating_add(bytes);
        while self.entries.len() > MAX_CACHED_ASSETS || self.total_bytes() > MAX_CACHED_BYTES {
            let Some(victim) = self
                .entries
                .iter()
                .map(|(hash, entry)| (hash.clone(), entry.last_used))
                .min_by(|left, right| left.1.cmp(&right.1).then(left.0.cmp(&right.0)))
                .map(|(hash, _)| hash)
            else {
                break;
            };
            if let Some(entry) = self.entries.remove(&victim) {
                self.readings.cached_bytes = self.readings.cached_bytes.saturating_sub(entry.bytes);
                self.readings.evicted_pressure = self.readings.evicted_pressure.saturating_add(1);
            }
        }
    }
}

/// 把交织 PCM 从 `source_rate` 转到 `target_rate`，并把输出帧数钉到
/// `ceil(source_frames × target_rate / source_rate)`。
///
/// 只在**两率不同**时被调用（同率素材由调用方原样搬进 `Arc`，⛔ 不复制、⛔ 不滤波）。
///
/// ⭐ 两个调用方共用这一份：实时注入侧（本模块的 `AudioPcmCache::lookup`，`#138`）与
/// 离线导出侧（[`crate::audio_clip`] 的 `decode_checked`，`#141`）。⛔ 不是复制两份。
///
/// # 为什么要把长度钉死
///
/// 引擎的摆放帧区间按**工程率**换算，而 `rubato` 的长度契约带
/// `max(0.1%, 8 帧)` 的容差（`yeban_decode::limits::resample_len_contract`）⇒ 输出可能
/// 比理想值少 1 帧。这里按**时长口径** `ceil` 补齐（末尾补零）或截掉多余帧，使
/// `frames == ceil(...)` 逐字成立 —— 与"时长照算"的既有口径一致。
///
/// # 音质代价（如实）
///
/// `rubato` `Async::new_sinc`：`sinc_len = 256`、`BlackmanHarris2` 窗、`f32` 域、
/// 固定块 1024 帧（[ARCH-DSP-002] / D26 的同一实现，与 MCP 渲染器**同一份**）。
/// 上采样在镜像频段靠该窗衰减、下采样在目标 Nyquist 之上靠该窗衰减（**不是**砖墙）；
/// 滤波器启动延迟由 `process_all_into_buffer` 裁掉（`PREFILL_FRAMES = 0`），
/// 但片段两端各有 `SINC_LEN / 2 = 128` 帧量级的过渡带。末尾补零最多 1 帧。
pub(crate) fn resample_to_target(
    samples: &[f32],
    channels: u16,
    source_rate: u32,
    target_rate: u32,
) -> Result<Vec<f32>, String> {
    let channels_usize = usize::from(channels);
    let source_frames = samples.len() / channels_usize;
    let mut converted =
        yeban_decode::resample_interleaved(samples, channels, source_rate, target_rate).map_err(
            |error| {
                format!(
                    "重采样失败（{source_rate} Hz → {target_rate} Hz，{source_frames} 帧）: {error}"
                )
            },
        )?;
    let target_frames = resampled_frames(source_frames, source_rate, target_rate)?;
    let wanted = target_frames
        .checked_mul(channels_usize)
        .ok_or_else(|| format!("重采样输出长度溢出：{target_frames} 帧 × {channels} 声道"))?;
    match converted.len().cmp(&wanted) {
        std::cmp::Ordering::Greater => converted.truncate(wanted),
        std::cmp::Ordering::Less => converted.resize(wanted, 0.0),
        std::cmp::Ordering::Equal => {}
    }
    Ok(converted)
}

/// `ceil(source_frames × target_rate / source_rate)`（0 帧 ⇒ 0 帧）。
fn resampled_frames(
    source_frames: usize,
    source_rate: u32,
    target_rate: u32,
) -> Result<usize, String> {
    if source_frames == 0 {
        return Ok(0);
    }
    let ideal = u128::from(u64::try_from(source_frames).unwrap_or(u64::MAX))
        .saturating_mul(u128::from(target_rate))
        .div_ceil(u128::from(source_rate));
    usize::try_from(ideal).map_err(|_| format!("重采样输出帧数溢出：{ideal} 帧"))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use yeban_render::dither::PcmBuffer;
    use yeban_render::rf64::PcmFormat;
    use yeban_render::wav::write_plain_wav;

    use super::*;

    /// 写一份**真 WAV** 字节（与 `--import-audio` / `#136` 判据同一个写入器）。
    fn wav_bytes(tag: &str, rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "yeban-138-unit-{tag}-{}-{serial}.wav",
            std::process::id()
        ));
        write_plain_wav(
            &path,
            &PcmFormat::integer(channels, rate, 16),
            &PcmBuffer::Int16(samples.to_vec()),
        )
        .expect("写夹具 WAV");
        let bytes = std::fs::read(&path).expect("读夹具 WAV");
        std::fs::remove_file(&path).expect("删夹具 WAV");
        bytes
    }

    fn source(bytes: Vec<u8>) -> AudioAssetSource {
        Box::new(move |_hash| Some(bytes.clone()))
    }

    /// 同率 ⇒ **不重采样**，逐样本位模式与解码结果相同（判据 §4③ 的单元级证据）。
    #[test]
    fn a_same_rate_lookup_returns_the_decoded_samples_bit_for_bit() {
        let frames = 100_usize;
        let raw: Vec<i16> = (0..frames)
            .map(|index| i16::try_from(index as i32 * 97 - 4_800).unwrap())
            .collect();
        let bytes = wav_bytes("same-rate", 48_000, 1, &raw);
        let hash = AssetHash::of_bytes(&bytes);
        let mut cache = AudioPcmCache::default();
        let lookup = cache
            .lookup(&hash, Some(&source(bytes.clone())), 48_000)
            .expect("同率必须命中");
        assert!(!lookup.resampled, "同率素材⛔ 不许走重采样");
        assert_eq!(lookup.source_sample_rate, 48_000);
        assert_eq!(lookup.source_frames, frames as u64);
        assert_eq!(lookup.pcm.frames(), frames as u64);
        assert_eq!(lookup.pcm.sample_rate(), 48_000);
        assert_eq!(cache.readings().resamples, 0);
        assert_eq!(cache.readings().decodes, 1);

        let decoded = crate::audio_clip::decode_asset_bytes(&hash, &bytes).expect("解码");
        let (expected, channels, rate) = decoded.into_parts();
        assert_eq!(channels, 1);
        assert_eq!(rate, 48_000);
        let same_bits = expected.iter().map(|sample| sample.to_bits()).eq(lookup
            .pcm
            .samples()
            .iter()
            .map(|sample| sample.to_bits()));
        assert!(
            same_bits,
            "同率路径必须逐位返回解码样本（⛔ 不复制、⛔ 不滤波）"
        );

        let again = cache
            .lookup(&hash, None, 48_000)
            .expect("第二次必须命中缓存");
        assert!(again.from_cache);
        assert!(!again.resampled);
        assert_eq!(cache.readings().decodes, 1);
        assert_eq!(cache.readings().resamples, 0);
        assert_eq!(cache.readings().hits, 1);
    }

    /// 44.1k ⇒ 48k：帧数 = `ceil(44101 × 48000 / 44100)`；同率后续修订不再重采样；
    /// 工程换率 ⇒ 缓存条目作废并重新解码（⛔ 不把旧率冒充新率）。
    #[test]
    fn a_rate_mismatch_resamples_and_invalidates_when_the_project_rate_changes() {
        let frames = 44_101_usize;
        let raw = vec![8_192_i16; frames];
        let bytes = wav_bytes("mismatch", 44_100, 1, &raw);
        let hash = AssetHash::of_bytes(&bytes);
        let mut cache = AudioPcmCache::default();

        let lookup = cache
            .lookup(&hash, Some(&source(bytes.clone())), 48_000)
            .expect("44.1k 素材必须被重采样而不是跳过");
        assert!(lookup.resampled);
        assert_eq!(lookup.source_sample_rate, 44_100);
        assert_eq!(lookup.source_frames, frames as u64);
        assert_eq!(lookup.pcm.sample_rate(), 48_000);
        let expected = (frames as u64 * 48_000).div_ceil(44_100);
        assert_eq!(
            lookup.pcm.frames(),
            expected,
            "帧数必须是 ceil(原帧数 × 比例)"
        );
        assert_eq!(cache.readings().resamples, 1);
        assert_eq!(cache.readings().decodes, 1);

        let again = cache
            .lookup(&hash, None, 48_000)
            .expect("第二个修订必须命中缓存");
        assert!(again.from_cache);
        assert!(again.resampled);
        assert_eq!(cache.readings().resamples, 1, "命中缓存 ⇒ ⛔ 不重采样");
        assert_eq!(cache.readings().decodes, 1);

        // 工程换成 44.1 kHz：上一份 48 kHz 的 PCM 不能冒充。
        let changed = cache
            .lookup(&hash, Some(&source(bytes)), 44_100)
            .expect("换率后必须重走解码");
        assert!(!changed.from_cache);
        assert!(!changed.resampled, "目标率 == 素材率 ⇒ 不重采样");
        assert_eq!(changed.pcm.frames(), frames as u64);
        assert_eq!(changed.pcm.sample_rate(), 44_100);
        assert_eq!(cache.readings().decodes, 2, "换率必须重新解码");
        assert_eq!(cache.readings().resamples, 1);
    }

    /// 0 帧 / 单帧：不 panic，帧数按 `ceil` 口径给读数。
    #[test]
    fn degenerate_lengths_stay_exact_and_do_not_panic() {
        let empty = wav_bytes("empty", 44_100, 1, &[]);
        let empty_hash = AssetHash::of_bytes(&empty);
        let mut cache = AudioPcmCache::default();
        let zero = cache
            .lookup(&empty_hash, Some(&source(empty)), 48_000)
            .expect("0 帧素材必须能注入");
        assert_eq!(zero.source_frames, 0);
        assert_eq!(zero.pcm.frames(), 0);

        let one = wav_bytes("one", 44_100, 1, &[16_384]);
        let one_hash = AssetHash::of_bytes(&one);
        let single = cache
            .lookup(&one_hash, Some(&source(one)), 48_000)
            .expect("单帧素材必须能注入");
        assert_eq!(single.source_frames, 1);
        assert_eq!(single.pcm.frames(), 2, "ceil(1 × 48000 / 44100) = 2");
    }
}
