//! T-FIX-886：实时设备回调体（`render_callback`，不是离线 `render_into`）
//! 必须让内置演示在整个走带范围都有非零输出。
//!
//! 背景（ISS-MWYG6XT1-B22656）：上报机在 007 小节采到端点全 0。实机复现证明
//! **起始处是有声的**（端点峰值 0.608666）；无声只发生在出场编排的内容末端之后：
//! `demo_project` 只有前 1.5 s 的 MIDI，之后没有任何可闻内容；`filled_project`
//! 的 Lead 循环摆放时值只有 1 小节（`loop_end == duration` ⇒ 单遍）。
//!
//! 判据走**设备回调同一个函数** `yeban_engine::device::render_callback`，因此
//! 覆盖的是实时路径主张本身；离线 `render_project_f32_with_assets` 的既有判据
//! `demo_sample_audio.rs` 不动。

use yeban_app::cli::Sample;
use yeban_app::engine_host::EngineHost;
use yeban_engine::device::render_callback;
use yeban_engine::ring::TransportCommand;

const RATE: usize = 48_000;
const BLOCK: usize = 128;
const CHANNELS: usize = 2;
const BAR_TICKS: u64 = 3_840;

fn host(sample: Sample) -> EngineHost {
    let project = sample.project();
    let assets = sample.assets();
    let mut host = EngineHost::new();
    host.set_audio_asset_source(Box::new(move |hash| {
        assets
            .iter()
            .find(|(candidate, _)| candidate == hash)
            .map(|(_, bytes)| bytes.clone())
    }));
    host.reload(&project, 0).expect("内置样例投影必须成功");
    host
}

/// 经设备回调体渲染 `seconds` 秒，返回交错缓冲。
fn render_via_callback(host: &mut EngineHost, seconds: usize) -> Vec<f32> {
    let runtime = host
        .runtime_for_criteria()
        .expect("控制驱动形态下必须有运行时");
    let quanta = seconds * RATE / BLOCK;
    let mut out = vec![0.0f32; BLOCK * CHANNELS * quanta];
    for quantum in 0..quanta {
        let start = quantum * BLOCK * CHANNELS;
        render_callback(runtime, &mut out[start..start + BLOCK * CHANNELS], 2);
    }
    out
}

fn window_peak(samples: &[f32], start_second: usize, end_second: usize) -> f32 {
    let start = start_second * RATE * CHANNELS;
    let end = (end_second * RATE * CHANNELS).min(samples.len());
    samples[start..end]
        .iter()
        .fold(0.0f32, |acc, sample| acc.max(sample.abs()))
}

/// 默认内置演示：起始 1 s 与 007 小节（7–8 s）都必须有声。
///
/// 修前实测 `t=0s peak=0.622127`、`t=1s peak=0.411560`、`t=2s..9s peak=0.000000`
/// ⇒ 7–8 s 断言当场变红。
#[test]
fn builtin_demo_is_audible_at_start_and_bar_seven_via_device_callback() {
    let mut host = host(Sample::Default);
    let reading = host.send_transport(&[TransportCommand::Play]);
    assert!(reading.state.is_running(), "必须真的在播放: {reading:?}");
    let out = render_via_callback(&mut host, 10);

    let at_start = window_peak(&out, 0, 1);
    let at_bar_seven = window_peak(&out, 7, 8);
    println!("[T-FIX-886/default] 0–1 s peak={at_start:.6} 7–8 s peak={at_bar_seven:.6}");
    assert!(at_start > 0.05, "起始必须可闻，实测 {at_start}");
    assert!(
        at_bar_seven > 0.05,
        "007 小节必须仍可闻（修前实测 0），实测 {at_bar_seven}"
    );
}

/// filled 演示：定位到 029 小节再走设备回调体，必须有声；对应上报机
/// `demo-filled` 在 029/160 采到全 0 的同一断点。
#[test]
fn filled_demo_is_audible_at_bar_twenty_nine_via_device_callback() {
    let mut host = host(Sample::Filled);
    let seek = 29 * BAR_TICKS;
    let reading = host.send_transport(&[
        TransportCommand::Stop,
        TransportCommand::SeekTicks(seek),
        TransportCommand::Play,
    ]);
    assert!(reading.state.is_running(), "必须真的在播放: {reading:?}");
    assert!(
        reading.position_ticks >= seek,
        "定位必须落在 029 小节（命令泵进一个量子，允许 128 帧的块内前进）: {reading:?}"
    );
    let out = render_via_callback(&mut host, 1);
    let peak = out.iter().fold(0.0f32, |acc, sample| acc.max(sample.abs()));
    println!("[T-FIX-886/filled@029] peak={peak:.6}");
    assert!(peak > 0.05, "029 小节必须可闻（修前实测 0），实测 {peak}");
}
