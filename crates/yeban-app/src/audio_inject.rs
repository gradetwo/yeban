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
//!   ──▶ AudioPcm::new(...)                          // 引擎形状（32 B，见 #135）
//!   ──▶ EngineSnapshot::with_audio_asset(&asset, pcm)  // 采样率不符 ⇒ Err（引擎拒绝）
//! ```
//!
//! # 三个决定
//!
//! 1. **PCM 不在工程模型里**（模型只有 `AssetHash`）⇒ **每个修订都要重新注入**。
//!    因此这里有一份**按 `AssetHash` 键控的解码缓存**：同一份素材在后续修订里只做
//!    一次 `Arc` 克隆（`AudioPcm::clone`），⛔ 不重复解码。读数见 [`AudioCacheReadings`]
//!    （`decodes` / `hits` / `fetches`）。
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
//! 3. **跳过必须可见**（⛔ 不静默）：没有字节来源 / 资产池里没有字节 / SHA-256 不符 /
//!    声道数 ≠ 1,2 / 采样率与快照不符 ⇒ 该素材**不注入**，一条逐字原因进
//!    [`AudioClipSkip`]，并由 `EngineHost` 打到 stderr（`[yeban-app] 音频片段注入跳过: …`）。
//!    ⚠ 采样率不符时**不重采样**：app 侧够不着重采样器（唯一实现在 `yeban-decode`，
//!    与 [`crate::audio_clip`] 模块文档第 2 条同一条纪律）—— 宁可静音并如实登记，
//!    ⛔ 不改采样率标签、⛔ 不丢帧。
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
//! `yeban_render::wav::read_plain_wav`、同一张归一化表）；本模块只多了"按哈希缓存"
//! 与"把结果搬进引擎形状"两件事。⛔ 没有第二份 WAV 解析器、⛔ 没有新依赖。

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
    /// 素材帧数（`samples.len() / channels`）。
    pub frames: u64,
    /// 素材声道数（1 或 2）。
    pub channels: u16,
    /// 素材采样率 (Hz)——必等于快照采样率。
    pub sample_rate: u32,
    /// 本次修订里被接上 PCM 的摆放条数。
    pub spans_attached: usize,
    /// `true` = 命中缓存（本次修订**没有**解码）。
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
            "音频片段注入: revision {} 注入 {} 个素材 / {} 条摆放 / 跳过 {} 个；\
             缓存 decodes={} hits={} fetches={} cached_assets={} cached_bytes={}",
            self.revision,
            self.injected_assets,
            self.spans_attached,
            self.skipped.len(),
            self.cache.decodes,
            self.cache.hits,
            self.cache.fetches,
            self.cache.cached_assets,
            self.cache.cached_bytes,
        )
    }

    /// 逐素材的读数行（`frames` / `channels` / `sample_rate`；日志用）。
    #[must_use]
    pub fn asset_lines(&self) -> Vec<String> {
        self.assets
            .iter()
            .map(|asset| {
                format!(
                    "音频片段注入素材: {} frames={} channels={} sample_rate={} spans={} from_cache={}",
                    asset.asset,
                    asset.frames,
                    asset.channels,
                    asset.sample_rate,
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

/// 缓存里的值。
enum CachedAudio {
    /// 已建成引擎形状的 PCM（`Arc` 克隆只在这里发生）。
    Ready(AudioPcm),
    /// **永久形状拒绝**（声道数 ≠ 1,2 / 半帧）：缓存原因文本，不再重复解码。
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
    /// 可注入的 PCM。
    pub(crate) pcm: AudioPcm,
    /// `true` = 命中缓存。
    pub(crate) from_cache: bool,
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

    /// 取一份 PCM：命中 ⇒ `Arc` 克隆；未命中 ⇒ 走**同一条** app 侧解码器。
    ///
    /// # Errors
    ///
    /// 逐字原因（没有字节来源 / 池里没有字节 / SHA-256 不符 / WAV 解码失败 /
    /// 声道形状非法）。调用方必须把它记进 [`AudioClipSkip`]，⛔ 不许吞掉。
    pub(crate) fn lookup(
        &mut self,
        hash: &AssetHash,
        source: Option<&AudioAssetSource>,
    ) -> Result<AudioLookup, String> {
        if let Some(entry) = self.entries.get_mut(hash) {
            entry.last_used = self.epoch;
            match &entry.value {
                CachedAudio::Ready(pcm) => {
                    self.readings.hits = self.readings.hits.saturating_add(1);
                    return Ok(AudioLookup {
                        pcm: pcm.clone(),
                        from_cache: true,
                    });
                }
                CachedAudio::Rejected(reason) => return Err(reason.clone()),
            }
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
        let pcm = match AudioPcm::new(Arc::from(samples), channels, sample_rate) {
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
        self.store(hash.clone(), CachedAudio::Ready(pcm.clone()), bytes);
        Ok(AudioLookup {
            pcm,
            from_cache: false,
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
