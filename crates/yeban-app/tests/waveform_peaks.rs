//! **音频片段按真实峰值画波形**（ADR-0076 B1）。
//!
//! 修前：`arrangement.slint` 只有 0dB 基准线 + 「波形待分析 · 示意」，
//! 注释自证「真实峰值（按 block 的离线分析缓存）尚未接入」—— 任何素材都长一样。
//!
//! 第一步（链路）：
//! 1. `bridge::compute_waveform_peaks(pcm, bins)` 纯函数（每 bin 取 |sample| 最大值）；
//! 2. `TimelineBlockData.waveform_peaks: [float]` 字段；
//! 3. 音频片段按 peaks 画柱；**peaks 为空时仍标「示意」**（⛔ 不假装有波形）。
//!
//! 第二步（数据）：
//! 4. `ClipView.asset_hash` 把 `ClipContent::Audio.asset` 带出投影（投影层不解码、不读盘）；
//! 5. `waveform_cache` 在后台线程上把字节解成 1024 bin 包络并按哈希缓存，
//!    `host::pump_waveform_analysis` 在心跳里驱动（⛔ 不阻塞 UI 线程）；
//! 6. `host::waveform_peaks_for` 按「未分析 ⇒ 空」的诚实口径取柱数。
//!
//! 本判据钉**源码契约**（`yeban-app` 依赖 Slint/cpal，属 `AGENTS.md` `5 的重依赖，
//! 本地不编译；行为判据在 `src/waveform_cache.rs` 的单元测试里，由 CI 跑）。

const BRIDGE: &str = include_str!("../src/bridge.rs");
const TOKENS: &str = include_str!("../ui/tokens.slint");
const ARR: &str = include_str!("../ui/arrangement.slint");
const CACHE: &str = include_str!("../src/waveform_cache.rs");
const HOST: &str = include_str!("../src/host.rs");
const MAIN: &str = include_str!("../src/main.rs");

use std::f32::consts::TAU;

/// T-FIX-80 pin: a known 1 kHz sine must produce per-bin maxima at the
/// analytic amplitude, and a silent tail must stay zero (not a fake constant).
#[test]
fn known_sine_peaks_match_the_analytic_amplitude_per_bin() {
    let rate = 48_000_usize;
    let frequency = 1_000.0_f32;
    let amplitude = 0.75_f32;
    let samples: Vec<f32> = (0..rate / 10)
        .map(|n| amplitude * (TAU * frequency * n as f32 / rate as f32).sin())
        .collect();
    let peaks = yeban_app::bridge::compute_waveform_peaks(&samples, 8);
    assert_eq!(peaks.len(), 8, "八个可见 bin 必须各有一个峰值");
    for (index, peak) in peaks.iter().enumerate() {
        assert!(
            (peak - amplitude).abs() < 1e-3,
            "bin {index} 的峰值应等于正弦幅度 {amplitude}，实际 {peak}"
        );
    }

    let mut with_silent_tail = samples;
    for sample in &mut with_silent_tail[rate / 20..] {
        *sample = 0.0;
    }
    let peaks = yeban_app::bridge::compute_waveform_peaks(&with_silent_tail, 4);
    assert_eq!(peaks.len(), 4, "四个可见 bin 必须各有一个峰值");
    assert!(
        peaks[0] > 0.7 && peaks[1] > 0.7,
        "前半段应保留正弦峰值，实际 {peaks:?}"
    );
    assert_eq!(
        &peaks[2..],
        &[0.0, 0.0],
        "后半段静音必须如实为 0，⛔ 不许被常量波形填满"
    );
}

/// T-FIX-80 pin: the projection must hand the cached peaks to the UI model,
/// not a constant placeholder.
#[test]
fn projected_timeline_block_writes_real_peaks_into_the_model() {
    assert!(
        HOST.contains("waveform_peaks: ModelRc::new(VecModel::from(waveform_peaks_for(clip)))"),
        "TimelineBlockData 必须从 waveform_peaks_for(clip) 取真实峰值，⛔ 不许写常量"
    );
}

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

#[test]
fn projection_carries_the_audio_asset_hash() {
    assert!(
        BRIDGE.contains("pub asset_hash: Option<String>"),
        "ClipView 必须把音频片段的资产哈希带出投影（峰值缓存的键）"
    );
    assert!(
        BRIDGE.contains(
            "Some(ClipContent::Audio { asset, .. }) => (\"audio\", Some(asset.to_string()))"
        ),
        "asset_hash 必须来自 ClipContent::Audio.asset，⛔ 不编一个哈希"
    );
}

#[test]
fn cache_decodes_off_the_ui_thread_and_reports_completion() {
    assert!(
        CACHE.contains("std::thread::Builder::new()"),
        "解码必须在后台线程（⛔ 不阻塞 UI 线程）"
    );
    assert!(
        CACHE.contains("pub fn spawn_batch(jobs: Vec<(String, Vec<u8>)>)"),
        "泵要能成批递「哈希 + 字节」"
    );
    assert!(
        CACHE.contains("pub fn take_finished() -> bool"),
        "批次结束要能被泵消费（否则画面永不变）"
    );
    assert!(
        CACHE.contains("store().failed.insert(asset)"),
        "解不出来的资产必须记账（⛔ 不每帧重试）"
    );
}

#[test]
fn pump_runs_on_the_heartbeat_not_on_open() {
    assert!(
        HOST.contains("pub fn pump_waveform_analysis(ui: &MainWindow, port: &Rc<UndoPort>)"),
        "泵必须由心跳驱动（录音 / 导入也会在运行期加字节）"
    );
    assert!(
        MAIN.contains("host::pump_waveform_analysis(&ui_tick, &port);"),
        "心跳里必须真的调了泵（⛔ 只定义不调用）"
    );
    assert!(
        HOST.contains("crate::waveform_cache::peaks(hash, waveform_bins_for_width(clip.width))"),
        "时间轴块必须按资产哈希 + 像素宽度取峰值"
    );
}

#[test]
fn unavailable_peaks_degrade_to_empty_not_to_a_fake_shape() {
    assert!(
        HOST.contains("// 缓存未就绪 / 字节不在资产池 / 解码失败 ⇒ 空数组 ⇒ 界面画回"),
        "取不到峰值时必须如实退回「示意」口径"
    );
    assert!(
        CACHE.contains("None => Vec::new(),"),
        "缓存未命中必须给空数组（⛔ 不编一个形状）"
    );
}
