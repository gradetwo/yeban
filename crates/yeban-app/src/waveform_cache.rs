//! 音频片段**真实波形包络**的会话缓存（ADR-0076 B1 第二步）。
//!
//! ## 它补的是哪一格
//!
//! 第一步（`bridge::compute_waveform_peaks` ＋ `TimelineBlockData::waveform_peaks` ＋
//! `arrangement.slint` 的竖条绘制）只把**链路**接好：峰值数组空着，界面照旧画
//! 「波形待分析 · 示意」。本模块负责把那份数组填上真实数据。
//!
//! ## 三条设计约束
//!
//! 1. **字节从哪来**：峰值必须从**资产字节**算（`ClipContent::Audio.asset` 指向
//!    `assets/{sha256}`），而字节在会话的 CAS 池里。投影层是 `ViewState` 的纯函数、
//!    拿不到端口，因此**泵**（`host::pump_waveform_analysis`）在心跳里把
//!    「哈希 + 字节」成批递进来，投影层只按哈希查缓存。
//! 2. **不许阻塞界面**：解码在**后台线程**上做（一批一个线程，⛔ 不并发两批）；
//!    UI 线程只做「取结果 / 查表 / 降采样」这些内存操作。
//! 3. **算一次就够**：每份资产只解一次，结果按最高分辨率（[`ENVELOPE_BINS`] 个 bin 的
//!    `|sample|` 最大值）存住；界面要多少根竖条都从这份包络**降采样**
//!    （[`downsample`]）得出。内容寻址 ⇒ 同哈希必同字节 ⇒ 同哈希必同包络。
//!
//! ## 诚实降级（⛔ 不许画假波形）
//!
//! 池里没有这块字节 / 解码被 `yeban-decode` 拒绝（零帧、损坏、超预算 …）⇒
//! [`peaks`] 返回**空** `Vec`，界面保持「波形待分析 · 示意」。解码失败按哈希记进
//! `failed`，⛔ 不每帧重试炸 CPU（泵会跳过它）。
//!
//! ## 线程模型
//!
//! 状态是一份 `Mutex<Store>`（`OnceLock` 惰性初始化）：UI 线程读表、后台线程写表。
//! 锁里**没有** Slint 对象、没有 I/O、没有解码 ⇒ 临界区是纯内存操作，微秒级。
//! ⛔ 本模块不进入音频回调（`[ARCH-RT-001]` 的零分配红线与之无关：音频线程根本不调它）。
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::bridge::compute_waveform_peaks;

/// 包络的最高分辨率（bin 数）。
///
/// 一份资产只解码一次，结果按这个分辨率存住；界面要多少根竖条都从这份包络降采样。
/// 取 1024 是因为它比最宽的片段（`host` 的柱数上限 200）还细一档 —— 再高只是内存开销。
pub const ENVELOPE_BINS: usize = 1024;

/// 包络缓存的上限（条目数）。超了整表清空：包络是**可重算的派生量**，
/// ⛔ 不是工程内容，清掉只会让下一批重新分析一次。
const MAX_CACHED_ASSETS: usize = 512;

/// 缓存与批次状态。
#[derive(Default)]
struct Store {
    /// 已算好的包络（按资产规范文本索引，内容寻址 ⇒ 同键必同值）。
    envelopes: BTreeMap<String, Arc<Vec<f32>>>,
    /// 解码**被拒绝**的资产：⛔ 泵不再为它起批次。
    failed: BTreeSet<String>,
    /// 有批次在飞。
    inflight: bool,
    /// 上一批已结束、且**还没被泵消费**（消费一次就落回去，泵只在真变化时重投影）。
    finished: bool,
}

/// 全局状态（惰性初始化，⛔ 不用 static mut）。
fn store() -> MutexGuard<'static, Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    let mutex = STORE.get_or_init(|| Mutex::new(Store::default()));
    // 后台线程 panic 只会让锁中毒；状态本身是可重算的派生量 ⇒ 继续用，⛔ 不把
    // 一次 panic 升级成「整个界面再也画不出波形」。
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// 某个资产在 `bins` 个 bin 上的峰值（未分析 / 解不出来 ⇒ 空，界面保持「示意」）。
///
/// `bins` 由调用方按块的**像素宽度**给（`host::waveform_bins_for_width`），
/// 内部夹到 `[1, ENVELOPE_BINS]`：包络已是最高分辨率，再多要没有信息。
#[must_use]
pub fn peaks(asset: &str, bins: usize) -> Vec<f32> {
    if asset.is_empty() || bins == 0 {
        return Vec::new();
    }
    let envelope = store().envelopes.get(asset).cloned();
    match envelope {
        Some(envelope) => downsample(&envelope, bins.min(ENVELOPE_BINS)),
        None => Vec::new(),
    }
}

/// 这个资产的包络是否已经在缓存里（泵据此跳过重复分析）。
#[must_use]
pub fn is_cached(asset: &str) -> bool {
    store().envelopes.contains_key(asset)
}

/// 这个资产是否已经被判定解不出来（泵据此跳过重试）。
#[must_use]
pub fn is_failed(asset: &str) -> bool {
    store().failed.contains(asset)
}

/// 当前没有批次在飞。
#[must_use]
pub fn is_idle() -> bool {
    !store().inflight
}

/// 取走「上一批刚结束」这条信号（⛔ 不清空缓存）：为 true ⇒ 调用方应重投影一次。
pub fn take_finished() -> bool {
    let mut store = store();
    if store.finished {
        store.finished = false;
        true
    } else {
        false
    }
}

/// 起一批后台分析（一批一个线程；⛔ 不并发两批，泵会先问 [`is_idle`]）。
///
/// 传进来的 `jobs` 是「资产哈希 + 字节」：字节由泵从会话 CAS 池里取
/// （`UndoPort::asset_bytes`），因此本模块⛔ 不读盘、⛔ 不认识路径。
///
/// 批次结束后 [`take_finished`] 变 true、[`is_idle`] 变回 true。
pub fn spawn_batch(jobs: Vec<(String, Vec<u8>)>) {
    if jobs.is_empty() {
        return;
    }
    {
        let mut store = store();
        store.inflight = true;
        store.finished = false;
    }
    let spawned = std::thread::Builder::new()
        .name("yeban-waveform".to_owned())
        .spawn(move || {
            for (asset, bytes) in jobs {
                match decode_envelope(&bytes) {
                    Some(envelope) => {
                        let mut store = store();
                        if store.envelopes.len() >= MAX_CACHED_ASSETS {
                            store.envelopes.clear();
                        }
                        store.envelopes.insert(asset, Arc::new(envelope));
                    }
                    None => {
                        store().failed.insert(asset);
                    }
                }
            }
            let mut store = store();
            store.inflight = false;
            store.finished = true;
        });
    if spawned.is_err() {
        // 起不了线程（极端资源耗尽）⇒ 如实回到空闲：泵下一跳会再试，
        // 界面停在「示意」基准线上（⛔ 假装分析过）。
        let mut store = store();
        store.inflight = false;
        store.finished = false;
    }
}

/// 清空缓存与失败表（判据用；打开新工程时也可用 —— 包络是可重算的派生量）。
pub fn clear() {
    let mut store = store();
    store.envelopes.clear();
    store.failed.clear();
    store.inflight = false;
    store.finished = false;
}

/// 资产字节 ⇒ 最高分辨率包络（解码失败 / 空素材 ⇒ None）。
///
/// 多声道交错 PCM 先按**帧**取各声道 `|s|` 的最大值（降成单声道包络），再交给
/// [`compute_waveform_peaks`]：这样立体声素材的左右差异不会互相抵消，画出来的是
/// 「这一刻有多响」而不是「这一声道有多响」。
fn decode_envelope(bytes: &[u8]) -> Option<Vec<f32>> {
    let decoded =
        yeban_decode::decode_bytes(bytes, &yeban_decode::DecodeOptions::default()).ok()?;
    let channels = usize::from(decoded.channels()).max(1);
    let mono = fold_channels(decoded.samples(), channels);
    let peaks = compute_waveform_peaks(&mono, ENVELOPE_BINS);
    if peaks.is_empty() { None } else { Some(peaks) }
}

/// 交错 PCM ⇒ 每帧一个 `|s|` 最大值（单声道包络）。
fn fold_channels(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    samples
        .chunks_exact(channels)
        .map(|frame| {
            frame
                .iter()
                .fold(0.0_f32, |acc, sample| acc.max(sample.abs()))
        })
        .collect()
}

/// 把最高分辨率包络**降采样**到 `bins` 根竖条（每根取源区间的最大值）。
///
/// `bins >= envelope.len()` 时原样返回：⛔ 不插值 —— 插值会造出素材里不存在的峰，
/// 而「缩小时间轴不该让峰值凭空长高」正是判据要钉的性质。
fn downsample(envelope: &[f32], bins: usize) -> Vec<f32> {
    if envelope.is_empty() || bins == 0 {
        return Vec::new();
    }
    if bins >= envelope.len() {
        return envelope.to_vec();
    }
    let mut out = Vec::with_capacity(bins);
    for index in 0..bins {
        let start = index * envelope.len() / bins;
        let end = ((index + 1) * envelope.len() / bins)
            .max(start + 1)
            .min(envelope.len());
        let peak = envelope[start..end]
            .iter()
            .fold(0.0_f32, |acc, value| acc.max(*value));
        out.push(peak.clamp(0.0, 1.0));
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use std::time::{Duration, Instant};

    use super::{
        clear, downsample, fold_channels, is_cached, is_failed, is_idle, peaks, spawn_batch,
        take_finished,
    };

    /// 判据之间共享同一份全局状态 ⇒ 串行执行（⛔ 不靠"应该不会撞"）。
    fn serial() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        match LOCK.get_or_init(|| Mutex::new(())).lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// 最小的合法 PCM16 单声道 WAV（44 字节头 + 样本）。
    fn wav_pcm16_mono(samples: &[i16]) -> Vec<u8> {
        let channels: u16 = 1;
        let rate: u32 = 44_100;
        let bits: u16 = 16;
        let block_align = channels * (bits / 8);
        let data_len = u32::try_from(samples.len() * 2).expect("夹具不会超 u32");
        let byte_rate = rate * u32::from(block_align);
        let mut out = Vec::with_capacity(44 + samples.len() * 2);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&byte_rate.to_le_bytes());
        out.extend_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples {
            out.extend_from_slice(&sample.to_le_bytes());
        }
        out
    }

    /// 等一批分析结束（最多 5 s）；超时返回 false ⇒ 判据报「没跑完」而不是「没分析」。
    fn wait_finished() -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if take_finished() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn fold_channels_takes_peak_across_channels() {
        // L/R 反相：单声道化必须留下**幅度**，⛔ 不是相加抵消成 0。
        let interleaved = [0.5_f32, -0.5, 0.25, 0.0, -0.75, 0.1];
        assert_eq!(fold_channels(&interleaved, 2), vec![0.5, 0.25, 0.75]);
    }

    #[test]
    fn fold_channels_is_identity_for_mono() {
        let mono = [0.1_f32, -0.9, 0.3];
        assert_eq!(fold_channels(&mono, 1), mono.to_vec());
    }

    #[test]
    fn fold_channels_ignores_a_ragged_tail() {
        // 交错样本数不是声道数的整数倍（损坏 / 截断素材）⇒ 尾部残余不参与
        // （chunks_exact），⛔ 不 panic。
        let interleaved = [0.2_f32, -0.4, 0.7];
        assert_eq!(fold_channels(&interleaved, 2), vec![0.4]);
    }

    #[test]
    fn downsample_keeps_the_loudest_source_bin() {
        let envelope = vec![0.0_f32, 0.25, 0.0, 0.0, 0.9, 0.0, 0.0, 0.1];
        // 8 → 2：前半（0..4）峰值 0.25，后半（4..8）峰值 0.9。
        assert_eq!(downsample(&envelope, 2), vec![0.25, 0.9]);
        // 要的比源还多 ⇒ 原样返回（⛔ 插值造峰）。
        assert_eq!(downsample(&envelope, 32), envelope);
    }

    #[test]
    fn spawn_batch_analyzes_bytes_and_reports_when_done() {
        let _guard = serial();
        clear();
        // 前 32 个样本满幅、后 32 个静音：包络必然是「前响后静」。
        let mut samples = vec![30_000_i16; 32];
        samples.extend(std::iter::repeat_n(0_i16, 32));
        let wav = wav_pcm16_mono(&samples);
        let hash = "a".repeat(64);
        spawn_batch(vec![(hash.clone(), wav)]);
        assert!(!is_idle(), "刚起的批次必须报「在飞」");
        assert!(wait_finished(), "批次 5 s 内必须结束");
        assert!(is_cached(&hash), "解成功必须进缓存");
        let bins = peaks(&hash, 4);
        assert_eq!(bins.len(), 4, "要几根给几根");
        assert!(bins[0] > 0.8, "前半段该是满幅：{bins:?}");
        assert_eq!(bins[3], 0.0, "后半段是静音：{bins:?}");
        clear();
    }

    #[test]
    fn undecodable_bytes_are_marked_failed_not_retried() {
        let _guard = serial();
        clear();
        let hash = "b".repeat(64);
        spawn_batch(vec![(hash.clone(), b"not a wav at all".to_vec())]);
        assert!(wait_finished(), "批次 5 s 内必须结束（失败也要结束）");
        assert!(is_failed(&hash), "解不出来必须记账");
        assert!(!is_cached(&hash));
        assert!(peaks(&hash, 8).is_empty(), "⛔ 不许画假波形");
        clear();
    }

    #[test]
    fn unknown_asset_yields_no_peaks() {
        let _guard = serial();
        clear();
        let hash = "c".repeat(64);
        assert!(peaks(&hash, 8).is_empty());
        assert!(!is_cached(&hash));
        assert!(!is_failed(&hash));
    }
}
