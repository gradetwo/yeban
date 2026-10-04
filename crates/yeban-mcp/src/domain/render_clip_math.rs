//! 音频片段装配的**零第三方依赖纯逻辑**：帧落位、增益合成、声道布局矩阵、
//! 重采样判定、编码器延迟裁剪区间与容器魔数嗅探。
//!
//! ## 为什么单独一个模块
//!
//! 本 crate 一旦依赖 `yeban-render`（rayon/hound/midly）与 `yeban-decode`
//! （symphonia/rubato）就不能在本机编译（`AGENTS.md` §5.2 的本机纪律），
//! 而"片段落在哪一帧、增益怎么合成、声道怎么映射、要不要重采样"这几件事
//! 完全不依赖任何第三方 crate。把它们抽到这里，就能用
//!
//! ```text
//! rustc --edition 2024 --test -D warnings crates/yeban-mcp/verify/render_pure.rs
//! ```
//!
//! 在本机**真的执行**（`#[path]` 引入本文件的**真实源码**，不是抄一份会漂移的副本）。
//!
//! ## 这一层允许什么运算
//!
//! 只用 `+ - * /`、比较与整数运算，以及从 [`super::render_math`] 复用的
//! `ticks_to_frames`（它内部的 `f64::round` 是 IEEE 754 精确指定的
//! `roundToIntegralTiesAway`，不是超越函数）。`dB → 线性` 的 `libm::powf` 留在
//! [`super::render`]，因为超越函数的位模式预算属于 [ARCH-DET-001] 的另一类
//! （见 `ADR-0001` **D32**："重采样与超越函数按类别分策"）。
//!
//! ## 这一层**不**做什么（边界，别误读）
//!
//! - [`sniff_container`] 只按**魔数**给一个"看起来是什么容器"的**报告用**结论。
//!   它**不**是支持性裁决：真正的裁决永远是 `yeban-decode`（symphonia）能不能解出来。
//!   拿嗅探结果去拒绝一个解码器其实能解的流，等于用一个猜的答案换掉一个真的答案。
//! - 重采样本身不在这一层：这里只判"要不要"（[`needs_resample`]），
//!   真正做转换的是 `yeban_decode::resample_interleaved`（rubato sinc，
//!   口径见 `docs/ledger/decode-core-notes.md` §6 与 `ADR-0001` D26）。

use super::render_math::ticks_to_frames;

/// 一个音频片段摆放的**时间轴帧区间** `[start, end)`。
///
/// 口径（与 MIDI 音符的 [`super::render_math::note_frame_span`] 一致）：
///
/// - 起点 = `ticks_to_frames(placement.start_tick)`（四舍五入到最近帧）；
/// - 终点 = `ticks_to_frames(placement.start_tick + max(duration_ticks, 1))`，
///   并且**至少比起点大 1 帧** —— 摆放的时值在模型层被校验为非零，
///   一个"非零时值却渲染出 0 帧"的片段是丢音，不是精确。
///
/// `placement.duration_ticks` 是**剪辑边界**（硬切，不做淡出）：素材比它长就截断，
/// 比它短就留静音尾巴。这个语义与 DAW 里"摆放长度 = 片段可见长度"一致。
#[must_use]
pub fn clip_frame_span(
    start_tick: u64,
    duration_ticks: u64,
    ppq: u64,
    bpm: f64,
    sample_rate: u32,
) -> (i64, i64) {
    let start = saturating_i64(ticks_to_frames(start_tick, ppq, bpm, sample_rate));
    let end_tick = start_tick.saturating_add(duration_ticks.max(1));
    let end = saturating_i64(ticks_to_frames(end_tick, ppq, bpm, sample_rate));
    (start, end.max(start.saturating_add(1)))
}

/// `u64` → `i64` 的饱和转换（帧数上限远小于 `i64::MAX`，这里是防御性写法）。
#[must_use]
fn saturating_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// 一条音轨上的音频片段在**合成之后**的增益（dB）。
///
/// 两级增益都是 dB，因此"先求和再转线性"与"分别转线性再相乘"在数学上等价，
/// 但只有前者是**一次**超越函数求值 —— 后者会引入两次舍入，
/// 而 [ARCH-DET-001] 的 L1 判据比较的是位模式。
///
/// 返回 `None` 表示**静音**（不可闻：`mute` / 未 `solo`），调用方据此把增益取 0，
/// 而不是把"静音"表示成一个极小的 dB。
///
/// 非有限的 dB 值按"未上报"处理（取 0 dB），与
/// [`yeban_render::render::db_to_linear`] 对非有限输入返回 `1.0` 的口径一致 ——
/// 两处对同一个病态输入给出同一个答案，不会产生"一个说静音、一个说满增益"的分歧。
#[must_use]
pub fn combined_gain_db(clip_gain_db: f32, track_volume_db: f32, audible: bool) -> Option<f32> {
    if !audible {
        return None;
    }
    let clip = if clip_gain_db.is_finite() {
        clip_gain_db
    } else {
        0.0
    };
    let track = if track_volume_db.is_finite() {
        track_volume_db
    } else {
        0.0
    };
    Some(clip + track)
}

/// 素材声道数 → 母线声道数的映射方式。
///
/// 这张表就是"支持矩阵"里**声道那一维**的可执行形式：
/// 只有 [`Self::Identity`] / [`Self::MonoToAll`] / [`Self::StereoToMono`] 三种，
/// 其余组合（例如 6 声道素材进立体声母线）返回 `None` —— 那时**拒绝**，
/// 而不是悄悄丢声道（丢声道是不可闻的错误，正是"不许静默出错"要消灭的东西）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelLayout {
    /// 声道数相同：逐声道一一对应。
    Identity,
    /// 单声道素材：复制到每一个母线声道。
    MonoToAll,
    /// 双声道素材进单声道母线：`(L + R) / 2`。
    StereoToMono,
}

impl ChannelLayout {
    /// 进响应的规范名字。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::MonoToAll => "mono-to-all",
            Self::StereoToMono => "stereo-to-mono",
        }
    }
}

/// 判定声道布局；不支持的组合返回 `None`。
///
/// 支持：`asset == out`（恒等）、`asset == 1`（单声道复制到任意母线）、
/// `asset == 2 && out == 1`（双声道折叠）。
#[must_use]
pub fn channel_layout(asset_channels: u16, out_channels: usize) -> Option<ChannelLayout> {
    if asset_channels == 0 || out_channels == 0 {
        return None;
    }
    if usize::from(asset_channels) == out_channels {
        return Some(ChannelLayout::Identity);
    }
    if asset_channels == 1 {
        return Some(ChannelLayout::MonoToAll);
    }
    if asset_channels == 2 && out_channels == 1 {
        return Some(ChannelLayout::StereoToMono);
    }
    None
}

/// 素材采样率是否必须转换才能进这条渲染管线。
///
/// `true` 时调用方**必须**走 `yeban_decode::resample_interleaved`
/// （rubato sinc，[ARCH-DSP-002] / D26），**不得**用"改个采样率标签"或
/// "丢帧"代替 —— 前者是改时长、后者是改音高，两者都是静默的错误音频。
#[must_use]
pub const fn needs_resample(asset_rate: u32, target_rate: u32) -> bool {
    asset_rate != target_rate
}

/// 编码器前置延迟 / 尾部填充的**裁剪区间** `[from, to)`（帧）。
///
/// 口径：`from = min(delay, frames)`、`to = frames - min(padding, frames)`；
/// `from >= to` 时返回 `None`（整段都被裁掉 —— 调用方应报错，而不是渲染一段静音）。
/// 无延迟/填充（`None` 或 0）时返回 `Some((0, frames))`。
///
/// 为什么这一层能做这个裁剪：`yeban-decode` 的边界是"**记录**容器上报的
/// 编码器延迟，不裁剪样本"（见 `docs/ledger/decode-core-notes.md` §9 的 needs）。
/// 渲染侧要的是"时间轴上对齐的音频"，因此这一步落在渲染侧，
/// 并且**必须如实报告**裁了多少帧（否则时长会悄悄偏）。
#[must_use]
pub fn encoder_trim_span(
    frames: u64,
    delay: Option<u32>,
    padding: Option<u32>,
) -> Option<(u64, u64)> {
    let from = u64::from(delay.unwrap_or(0)).min(frames);
    let to = frames.saturating_sub(u64::from(padding.unwrap_or(0)));
    if from >= to { None } else { Some((from, to)) }
}

/// 容器形态的**魔数**分类（报告用，不是裁决）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerSniff {
    /// `RIFF....WAVE`。
    RiffWave,
    /// `fLaC`。
    Flac,
    /// `OggS`。
    Ogg,
    /// 魔数不认识（可能是别的容器，也可能就是坏字节）。
    Unknown,
}

impl ContainerSniff {
    /// 进响应的规范名字。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::RiffWave => "RIFF/WAVE",
            Self::Flac => "FLAC",
            Self::Ogg => "Ogg",
            Self::Unknown => "unknown",
        }
    }
}

/// 按魔数嗅探容器。**只用于报告**（见模块文档的边界一节）。
#[must_use]
pub fn sniff_container(bytes: &[u8]) -> ContainerSniff {
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        return ContainerSniff::RiffWave;
    }
    if bytes.starts_with(b"fLaC") {
        return ContainerSniff::Flac;
    }
    if bytes.starts_with(b"OggS") {
        return ContainerSniff::Ogg;
    }
    ContainerSniff::Unknown
}

/// `yeban-decode` 启用的容器/编解码矩阵（响应与台账**共用同一份字符串**）。
pub const SUPPORTED_ASSET_SUMMARY: &str =
    "WAV (PCM 8/16/24/32-bit 与 f32, 含 ADPCM) / FLAC / Ogg-Vorbis; 1-2 声道";
/// 明确**不**支持的容器/编解码（feature 未启用，遇到即 RENDER_FAILED）。
pub const UNSUPPORTED_ASSET_SUMMARY: &str =
    "Matroska / AIFF / CAF / ISO-MP4 / MP3 / AAC / ALAC (feature 未启用); 声道数 > 2";
/// 重采样口径（D26）的一句话说明。
pub const RESAMPLER_SUMMARY: &str = "rubato Async::<f32>::new_sinc, 窗 BlackmanHarris2, sinc_len=256, chunk=1024, \
     max_relative_ratio=1.0, 前置延迟已被上游 process_all_into_buffer 裁掉";

#[cfg(test)]
mod tests {
    use super::*;

    /// 判据 1: 帧落位在整数关系上精确（120 BPM / 960 PPQ ⇒ 1920 tick = 1 秒）。
    #[test]
    fn clip_span_maps_one_second_of_ticks_to_the_sample_rate() {
        assert_eq!(clip_frame_span(0, 1_920, 960, 120.0, 48_000), (0, 48_000));
        assert_eq!(clip_frame_span(0, 1_920, 960, 120.0, 44_100), (0, 44_100));
        // 摆放在 1 秒处、时长半秒。
        assert_eq!(
            clip_frame_span(1_920, 960, 960, 120.0, 48_000),
            (48_000, 72_000)
        );
        // 128 BPM / 3840 tick ⇒ 1.875 s。
        assert_eq!(clip_frame_span(0, 3_840, 960, 128.0, 48_000), (0, 90_000));
    }

    /// 判据 2: 病态输入不会产生反向区间或 panic。
    #[test]
    fn degenerate_tempo_yields_an_empty_but_ordered_span() {
        for (ppq, bpm, rate) in [
            (0u64, 120.0f64, 48_000u32),
            (960, 0.0, 48_000),
            (960, -120.0, 48_000),
            (960, f64::NAN, 48_000),
            (960, 120.0, 0),
        ] {
            let (start, end) = clip_frame_span(1_000, 500, ppq, bpm, rate);
            assert_eq!(start, 0, "{ppq}/{bpm}/{rate}");
            assert_eq!(end, 1, "非零时值必须至少给 1 帧");
        }
        // 时值为 0（模型层会拒绝，这里仍不得产生反向区间）。
        let (start, end) = clip_frame_span(960, 0, 960, 120.0, 48_000);
        assert!(end > start);
    }

    /// 判据 3: 增益合成 —— 静音是 `None`（而不是一个极小的 dB），
    /// 两级 dB 相加，非有限值按 0 dB 处理。
    #[test]
    fn gain_composition_reports_silence_as_none() {
        assert_eq!(combined_gain_db(-1.5, 6.0, true), Some(4.5));
        assert_eq!(combined_gain_db(-1.5, 6.0, false), None);
        assert_eq!(combined_gain_db(0.0, 0.0, true), Some(0.0));
        assert_eq!(combined_gain_db(f32::NAN, 3.0, true), Some(3.0));
        assert_eq!(combined_gain_db(3.0, f32::INFINITY, true), Some(3.0));
        assert_eq!(
            combined_gain_db(f32::INFINITY, f32::NEG_INFINITY, true),
            Some(0.0)
        );
    }

    /// 判据 4: 声道布局矩阵 —— 支持的三格 + 其余一律 `None`。
    #[test]
    fn channel_layout_matrix_is_explicit() {
        assert_eq!(channel_layout(2, 2), Some(ChannelLayout::Identity));
        assert_eq!(channel_layout(1, 2), Some(ChannelLayout::MonoToAll));
        assert_eq!(channel_layout(2, 1), Some(ChannelLayout::StereoToMono));
        assert_eq!(channel_layout(1, 1), Some(ChannelLayout::Identity));
        // 不支持：多声道素材既不能丢声道也不能瞎混；0 声道/0 母线一律拒绝。
        for (asset, out) in [
            (3u16, 2usize),
            (6, 2),
            (4, 2),
            (2, 4),
            (0, 2),
            (2, 0),
            (0, 0),
        ] {
            assert_eq!(channel_layout(asset, out), None, "{asset} -> {out}");
        }
        // 恰好相等时恒等（哪怕声道数很多）—— 那是"原样搬"，不是"混音"。
        assert_eq!(channel_layout(8, 8), Some(ChannelLayout::Identity));
        assert_eq!(ChannelLayout::MonoToAll.name(), "mono-to-all");
        assert_eq!(ChannelLayout::Identity.name(), "identity");
        assert_eq!(ChannelLayout::StereoToMono.name(), "stereo-to-mono");
    }

    /// 判据 5: 重采样判定只看"两个率是否相等"，与数值大小无关。
    #[test]
    fn resample_decision_is_about_inequality_only() {
        assert!(!needs_resample(48_000, 48_000));
        assert!(needs_resample(44_100, 48_000));
        assert!(needs_resample(48_000, 44_100));
        assert!(needs_resample(48_000, 96_000));
        assert!(!needs_resample(0, 0));
        assert!(needs_resample(0, 48_000));
    }

    /// 判据 6: 编码器延迟/填充的裁剪区间（含"整段被裁掉"与"越界声明"）。
    #[test]
    fn encoder_trim_span_handles_delay_padding_and_overclaims() {
        assert_eq!(encoder_trim_span(1_000, None, None), Some((0, 1_000)));
        assert_eq!(encoder_trim_span(1_000, Some(0), Some(0)), Some((0, 1_000)));
        assert_eq!(
            encoder_trim_span(1_000, Some(64), Some(32)),
            Some((64, 968))
        );
        // 声明比总帧数还大: 夹住, 不 panic, 也不产生反向区间。
        assert_eq!(encoder_trim_span(100, Some(1_000), None), None);
        assert_eq!(encoder_trim_span(100, Some(40), Some(1_000)), None);
        assert_eq!(encoder_trim_span(0, None, None), None);
        // 恰好剩 1 帧仍然保留。
        assert_eq!(encoder_trim_span(100, Some(99), Some(0)), Some((99, 100)));
    }

    /// 判据 7: 魔数嗅探只认明确的签名，其余一律 `Unknown`（不做猜测）。
    #[test]
    fn container_sniff_requires_an_exact_signature() {
        let mut riff = vec![0u8; 32];
        riff[0..4].copy_from_slice(b"RIFF");
        riff[8..12].copy_from_slice(b"WAVE");
        assert_eq!(sniff_container(&riff), ContainerSniff::RiffWave);
        // `RIFF` 但不是 `WAVE`（例如 AVI）不许被认成 WAVE。
        let mut avi = riff.clone();
        avi[8..12].copy_from_slice(b"AVI ");
        assert_eq!(sniff_container(&avi), ContainerSniff::Unknown);
        assert_eq!(
            sniff_container(b"fLaC\x00\x00\x00\x22"),
            ContainerSniff::Flac
        );
        assert_eq!(sniff_container(b"OggS\x00\x02"), ContainerSniff::Ogg);
        assert_eq!(sniff_container(b"ID3\x04\x00"), ContainerSniff::Unknown);
        assert_eq!(sniff_container(b""), ContainerSniff::Unknown);
        assert_eq!(sniff_container(b"RIFF"), ContainerSniff::Unknown);
        assert_eq!(ContainerSniff::Flac.name(), "FLAC");
        assert_eq!(ContainerSniff::Unknown.name(), "unknown");
    }
}
