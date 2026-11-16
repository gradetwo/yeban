//! **音频片段按真实峰值画波形**（ADR-0076 B1，第一步：链路就绪）。
//!
//! 修前：`arrangement.slint` 只有 0dB 基准线 + 「波形待分析 · 示意」，
//! 注释自证「真实峰值（按 block 的离线分析缓存）尚未接入」—— 任何素材都长一样。
//!
//! 本步交付：
//! 1. `bridge::compute_waveform_peaks(pcm, bins)` 纯函数（每 bin 取 |sample| 最大值）；
//! 2. `TimelineBlockData.waveform_peaks: [float]` 字段；
//! 3. 音频片段按 peaks 画柱；**peaks 为空时仍标「示意」**（⛔ 不假装有波形）。
//!
//! ⚠ 峰值**数据填充**（从 `.yeban` 容器 `archive.assets` 解码 → 缓存）是第二步，
//! 见 ADR-0076 B1；本判据只钉「链路 + 诚实降级」。

const BRIDGE: &str = include_str!("../src/bridge.rs");
const TOKENS: &str = include_str!("../ui/tokens.slint");
const ARR: &str = include_str!("../ui/arrangement.slint");

#[test]
fn waveform_peak_function_is_per_bin_max_abs() {
    assert!(
        BRIDGE.contains("pub fn compute_waveform_peaks(pcm: &[f32], bins: usize) -> Vec<f32>"),
        "必须有按 bin 算峰值的纯函数"
    );
    assert!(
        BRIDGE.contains("acc.max(s.abs())"),
        "每 bin 必须取 |sample| 的最大值（真实包络），⛔ 不是常量"
    );
}

#[test]
fn audio_clip_renders_real_peaks_when_present() {
    assert!(
        TOKENS.contains("waveform_peaks: [float]"),
        "TimelineBlockData 必须有 waveform_peaks 字段"
    );
    assert!(
        ARR.contains("for peak_index in blk.waveform-peaks.length"),
        "音频片段必须按 peaks 长度循环画柱"
    );
    assert!(
        ARR.contains("blk.waveform-peaks.length == 0"),
        "peaks 为空时必须仍标「示意」（诚实降级）"
    );
}
