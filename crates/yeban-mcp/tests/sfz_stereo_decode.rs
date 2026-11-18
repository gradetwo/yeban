//! # MCP 侧 SFZ 解码保声道判据（台账 §15.8）
//!
//! ## 缺口（修前）
//!
//! 引擎立体声通路已合入 main，但 MCP 进程内的 SFZ 装载走 `domain::sfz` 自有
//! 解码腿：WAV 走 `read_plain_wav_mono_f32`、其余走 `decode_mono_channel0`，
//! **两条腿都折声道 0**；提供表转武装样本时声道数恒填 1。后果：经
//! `yeban_generate_drum_pattern`＋`yeban_render_master` 装载的采样鼓组，
//! `has_multichannel_samples` 恒假、走单声道通路 —— 原声 6 条 loop 实测
//! side/mid 仅 0.007–0.033 且与单声道版逐位一致（源 FLAC 本身是真立体声）。
//!
//! ## 本文件钉的六件事
//!
//! | # | 判据 | 修前行为 |
//! |---|------|---------|
//! | 1 | WAV 腿保声道：立体声 WAV ⇒ `channels=2`、交错 PCM 原样、`frames`＝每声道帧数；声道 0 与单声道腿**逐位相同** | `channels` 概念不存在，PCM 只剩声道 0 |
//! | 2 | 单声道不退化：单声道 WAV ⇒ `channels=1`、PCM 与单声道腿**逐位相同**（单声道源渲染逐位不变的前提） | 同左（本判据防回归） |
//! | 3 | 通用腿保声道：立体声 FLAC ⇒ `channels=2`、交错长度＝帧数×2 | 折声道 0，长度＝帧数 |
//! | 4 | >2 声道**响亮拒绝**：3 声道 WAV ⇒ 解码报错点名声道数；装载计 `failed=1 / read=0` 且回退原因可见 | 静默折声道 0 当单声道读入 |
//! | 5 | 装载读数如实：立体声样本经 `yeban_load_sfz` ⇒ `sampleSource` 带 `channels=2`、`sampleFramesLoaded`＝每声道帧数 | 读数无声道段 |
//! | 6 | 引擎通路见证：同工程绑采样器，样本右声道有声（0.25）vs 右声道静音两版 `yeban_render_master` ⇒ 右声道窗 RMS 前者显著大于后者、左声道两版近似 | 两版右声道逐位相同（样本右声道根本没进引擎） |
//!
//! 夹具全部手写字节／`yeban_decode::testfix` 合成，⛔ 不用被测写入器自证。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use yeban_decode::testfix::{FlacSpec, flac_constant};
use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::sfz::read_sample_file;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::samples::filled_project;
use yeban_model::{SamplerBinding, YebanProjectV1};
use yeban_render::wav::read_plain_wav_mono_f32;

// ---------------------------------------------------------------------------
// 夹具基础设施（与 tests/sfz_decoded_formats.rs 同形）
// ---------------------------------------------------------------------------

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "yeban-sfz-stereo-{tag}-{}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        Self(dir)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn dispatcher_with_project(project: YebanProjectV1, path: &Path) -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(path.to_path_buf(), project, false)
        .expect("注入工程");
    dispatcher
}

fn bearer(dispatcher: &Dispatcher) -> String {
    format!("Bearer {}", dispatcher.expected_token().expose())
}

fn call_tool(dispatcher: &mut Dispatcher, name: &str, arguments: Value) -> Value {
    let auth = bearer(dispatcher);
    let line = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
    assert_eq!(
        outcome.error_code(),
        None,
        "`{name}` 不得产出 JSON-RPC 层错误: {outcome:?}"
    );
    outcome.response.expect("响应").result.expect("result")
}

/// 手写 16-bit PCM WAV（任意声道数；每声道一个常量值）。⛔ 不用被测写入器。
fn write_constant_wav(path: &Path, channel_values: &[f64], frames: usize, sample_rate: u32) {
    let channels = u16::try_from(channel_values.len()).expect("声道数");
    let mut data = Vec::with_capacity(frames * channel_values.len() * 2);
    for _ in 0..frames {
        for value in channel_values {
            let scaled = (value * f64::from(i16::MAX)).round() as i16;
            data.extend_from_slice(&scaled.to_le_bytes());
        }
    }
    let block_align = channels * 2;
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&u32::try_from(36 + data.len()).expect("小").to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * u32::from(block_align)).to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&u32::try_from(data.len()).expect("小").to_le_bytes());
    out.extend_from_slice(&data);
    fs::write(path, &out).expect("写常量 WAV 夹具");
}

/// 读回母带 WAV（24-bit 立体声），返回每声道的**最大窗 RMS**（窗 2048 帧）。
///
/// 常量样本在音符存续期内是常量 ⇒ 窗 RMS 抓住发声段，不受文件前后静音稀释。
fn master_window_rms(path: &Path) -> (f64, f64) {
    let bytes = fs::read(path).expect("读母带");
    // 定位 data 块（母带由被测写入器产出，但解析只按 WAV 容器规范走）。
    let mut offset = 12_usize;
    let mut data: Option<&[u8]> = None;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(
            bytes[offset + 4..offset + 8]
                .try_into()
                .expect("块头 4 字节"),
        ) as usize;
        if id == b"data" {
            data = Some(&bytes[offset + 8..offset + 8 + size]);
            break;
        }
        offset += 8 + size + (size % 2);
    }
    let data = data.expect("母带必须有 data 块");
    let frame_bytes = 6_usize; // 24-bit × 2 声道
    let frames = data.len() / frame_bytes;
    assert!(frames > 4096, "母带帧数异常: {frames}");
    let sample = |frame: usize, channel: usize| -> f64 {
        let at = frame * frame_bytes + channel * 3;
        let raw =
            i32::from(data[at]) | (i32::from(data[at + 1]) << 8) | (i32::from(data[at + 2]) << 16);
        let signed = if raw & 0x80_0000 != 0 {
            raw - 0x100_0000
        } else {
            raw
        };
        f64::from(signed) / f64::from(1_i32 << 23)
    };
    let mut best = (0.0_f64, 0.0_f64);
    let window = 2048_usize;
    let mut start = 0_usize;
    while start + window <= frames {
        let mut sums = (0.0_f64, 0.0_f64);
        for frame in start..start + window {
            sums.0 += sample(frame, 0).powi(2);
            sums.1 += sample(frame, 1).powi(2);
        }
        best.0 = best.0.max((sums.0 / window as f64).sqrt());
        best.1 = best.1.max((sums.1 / window as f64).sqrt());
        start += window;
    }
    best
}

// ---------------------------------------------------------------------------
// 判据 1–3：解码腿本身
// ---------------------------------------------------------------------------

/// 判据 1：WAV 腿保声道 —— 交错原样、声道数如实、声道 0 与单声道腿逐位相同。
#[test]
fn a_stereo_wav_reads_back_interleaved_with_its_channel_count() {
    let scratch = Scratch::new("wav-stereo");
    let path = scratch.join("st.wav");
    write_constant_wav(&path, &[0.5, 0.25], 4_800, 48_000);

    let reading = read_sample_file(&path).expect("立体声 WAV 必须读得出");
    assert_eq!(reading.channels, 2, "声道数必须如实: {}", reading.describe);
    assert_eq!(
        reading.frames, 4_800,
        "frames 必须是每声道帧数: {}",
        reading.describe
    );
    assert_eq!(
        reading.pcm.len(),
        9_600,
        "交错 PCM 长度 = 帧数 × 声道数（修前折声道 0 时只有 4800）"
    );
    assert!(
        reading.describe.contains("channels=2"),
        "读数必须带声道数: {}",
        reading.describe
    );
    // 交错顺序逐位：偶数位 = 左（0.5 的 16-bit 换算），奇数位 = 右（0.25）。
    let (mono, mono_rate, _) = read_plain_wav_mono_f32(&path).expect("单声道腿参照");
    assert_eq!(mono_rate, 48_000);
    for (frame, mono_sample) in mono.iter().enumerate() {
        assert_eq!(
            reading.pcm[frame * 2].to_bits(),
            mono_sample.to_bits(),
            "声道 0 必须与单声道腿逐位相同（帧 {frame}）"
        );
        assert!(
            (f64::from(reading.pcm[frame * 2 + 1]) - 0.25).abs() < 0.001,
            "右声道必须是样本自己的 0.25（帧 {frame}）: {}",
            reading.pcm[frame * 2 + 1]
        );
    }
}

/// 判据 2：单声道不退化 —— PCM 与改前的单声道腿逐位相同。
#[test]
fn a_mono_wav_reads_back_bit_identical_to_the_old_mono_leg() {
    let scratch = Scratch::new("wav-mono");
    let path = scratch.join("mono.wav");
    write_constant_wav(&path, &[0.4], 4_800, 44_100);

    let reading = read_sample_file(&path).expect("单声道 WAV 必须读得出");
    let (mono, _, _) = read_plain_wav_mono_f32(&path).expect("单声道腿参照");
    assert_eq!(reading.channels, 1);
    assert_eq!(reading.frames, 4_800);
    assert_eq!(reading.pcm.len(), mono.len());
    for (index, (now, before)) in reading.pcm.iter().zip(mono.iter()).enumerate() {
        assert_eq!(
            now.to_bits(),
            before.to_bits(),
            "单声道 PCM 必须逐位不变（样本 {index}）"
        );
    }
}

/// 判据 3：通用腿保声道 —— 立体声 FLAC 交错读出，不再折声道 0。
#[test]
fn a_stereo_flac_reads_back_interleaved_via_the_generic_leg() {
    let scratch = Scratch::new("flac-stereo");
    let path = scratch.join("st.flac");
    let spec = FlacSpec {
        channels: 2,
        ..FlacSpec::default()
    };
    fs::write(&path, flac_constant(&spec, 2, 500)).expect("写立体声 FLAC 夹具");

    let reading = read_sample_file(&path).expect("立体声 FLAC 必须读得出");
    assert_eq!(reading.channels, 2, "声道数必须如实: {}", reading.describe);
    assert_eq!(
        reading.frames, 512,
        "frames 是每声道帧数: {}",
        reading.describe
    );
    assert_eq!(
        reading.pcm.len(),
        1_024,
        "交错 PCM 长度 = 帧数 × 声道数（修前折声道 0 时只有 512）"
    );
    assert!(
        reading.describe.contains("channels=2"),
        "读数必须带声道数: {}",
        reading.describe
    );
}

// ---------------------------------------------------------------------------
// 判据 4–5：响亮拒绝与装载读数
// ---------------------------------------------------------------------------

fn load_sfz(scratch: &Scratch, tag: &str, sfz_body: &str) -> Value {
    let dir = scratch.join(tag);
    fs::create_dir_all(&dir).expect("建装载目录");
    let sfz = dir.join("kit.sfz");
    fs::write(&sfz, sfz_body).expect("写 SFZ 夹具");
    let project_path = scratch.join(&format!("{tag}.yeban"));
    let mut dispatcher = dispatcher_with_project(filled_project(), &project_path);
    call_tool(
        &mut dispatcher,
        "yeban_load_sfz",
        json!({ "path": sfz.display().to_string() }),
    )
}

/// 判据 4：>2 声道响亮拒绝 —— ⛔ 不许静默折声道当单声道读入。
#[test]
fn a_three_channel_sample_is_rejected_loudly_not_folded() {
    let scratch = Scratch::new("three-channel");
    // 样本与 SFZ 同目录（装载按 SFZ 所在目录解析相对样本路径）。
    let dir = scratch.join("tri-load");
    fs::create_dir_all(&dir).expect("建目录");
    let sample = dir.join("tri.wav");
    write_constant_wav(&sample, &[0.5, 0.25, 0.125], 4_800, 48_000);

    let error = read_sample_file(&sample).expect_err("3 声道必须被拒");
    assert!(
        error.contains("unsupported channel count 3"),
        "拒绝必须点名声道数: {error}"
    );

    let loaded = load_sfz(
        &scratch,
        "tri-load",
        "<region> sample=tri.wav key=60 pitch_keycenter=60\n",
    );
    assert_eq!(loaded["status"], "success", "{loaded}");
    assert_eq!(
        loaded["data"]["sampleFilesReadCount"], 0,
        "3 声道样本不得被读入: {loaded}"
    );
    assert_eq!(
        loaded["data"]["sampleFilesFailedCount"], 1,
        "失败必须计数: {loaded}"
    );
    let source = loaded["data"]["sampleSource"]
        .as_str()
        .expect("sampleSource");
    assert!(
        source.contains("unsupported channel count 3"),
        "回退原因必须点名声道数（不许静默）: {source}"
    );
}

/// 判据 5：装载读数如实带声道数，帧数记账仍是每声道帧数。
#[test]
fn a_stereo_sample_load_reports_channels_and_per_channel_frames() {
    let scratch = Scratch::new("load-stereo");
    let dir = scratch.join("kit2");
    fs::create_dir_all(&dir).expect("建目录");
    write_constant_wav(&dir.join("st.wav"), &[0.5, 0.25], 4_800, 48_000);

    let loaded = load_sfz(
        &scratch,
        "kit2",
        "<region> sample=st.wav key=60 pitch_keycenter=60\n",
    );
    assert_eq!(loaded["status"], "success", "{loaded}");
    assert_eq!(loaded["data"]["sampleFilesReadCount"], 1, "{loaded}");
    let source = loaded["data"]["sampleSource"]
        .as_str()
        .expect("sampleSource");
    assert!(
        source.contains("frames=4800 channels=2 rate=48000"),
        "读数必须带每声道帧数与声道数: {source}"
    );
    assert_eq!(
        loaded["data"]["sampleFramesLoaded"], 4_800,
        "帧数记账必须是每声道帧数（与改前同一口径）: {loaded}"
    );
}

// ---------------------------------------------------------------------------
// 判据 6：引擎通路见证（右声道真的到达母带）
// ---------------------------------------------------------------------------

/// 同一工程、同一 SFZ，只换样本的右声道（有声 0.25 vs 静音 0.0）渲染两次。
#[test]
fn the_right_channel_of_a_stereo_sample_reaches_the_master() {
    let scratch = Scratch::new("master-witness");
    let kit = scratch.join("kit3");
    fs::create_dir_all(&kit).expect("建目录");
    fs::write(
        kit.join("kit.sfz"),
        "<region> sample=st.wav key=60 pitch_keycenter=60\n",
    )
    .expect("写 SFZ");

    let render_with = |tag: &str, right: f64| -> (f64, f64) {
        write_constant_wav(&kit.join("st.wav"), &[0.5, right], 48_000, 48_000);
        let mut project = filled_project();
        let master = project.master_bus_track_id;
        for (id, track) in project.tracks.iter_mut() {
            if *id != master {
                track.sampler = Some(SamplerBinding {
                    note_octave_override: None,
                    path: kit.join("kit.sfz").display().to_string(),
                });
            }
        }
        let project_path = scratch.join(&format!("{tag}.yeban"));
        let mut dispatcher = dispatcher_with_project(project, &project_path);
        let out = scratch.join(&format!("{tag}.wav"));
        let rendered = call_tool(
            &mut dispatcher,
            "yeban_render_master",
            json!({ "format": "wav", "sampleRate": 48000, "path": out.display().to_string() }),
        );
        assert_eq!(rendered["status"], "success", "{tag}: {rendered}");
        master_window_rms(&out)
    };

    let (left_live, right_live) = render_with("live", 0.25);
    let (left_mute, right_mute) = render_with("mute", 0.0);
    println!("### 右声道有声版 窗 RMS: L={left_live:.6} R={right_live:.6}");
    println!("### 右声道静音版 窗 RMS: L={left_mute:.6} R={right_mute:.6}");

    assert!(
        right_live > 0.01,
        "右声道必须有真实能量（修前恒为折叠后的同值）: R={right_live}"
    );
    assert!(
        right_live > right_mute * 10.0,
        "右声道能量必须来自样本右声道本身: 有声 {right_live} vs 静音 {right_mute}"
    );
    assert!(
        (left_live - left_mute).abs() <= left_mute * 0.1 + 1e-6,
        "左声道两版必须近似（只换了右声道）: {left_live} vs {left_mute}"
    );
}
