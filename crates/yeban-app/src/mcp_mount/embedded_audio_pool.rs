//! ⭐ 任务单 `#292`：**内嵌 MCP 挂载必须把会话 CAS 池带进 `Domain`**。
//!
//! ## 缺口（修前）
//!
//! [`super::InProcessMcp::start_for_project`] 经 [`super::session_dispatcher`]（生产
//! 路径的**同一个**符号）只把 `YebanProjectV1` 交给 `Domain::open_in_memory`，而那里
//! 把会话资产池建成**空 `BTreeMap`** ⇒ GUI 里导入过音频的工程经**内嵌**控制面导出母带
//! 时，片段引用的哈希在池里查不到，渲染层只能登记 `audioClips` 并当静音。
//!
//! ⚠ `audioClips` **不是**"音频不参与"：`ClipContent::Audio` 早就真的进母带
//! （解码 → 重采样 → 落位 → 门控 → PDC，见 `yeban-mcp` 的 `domain/render.rs` "真做"
//! 清单）。那个键只在"工程**声明**了资产、会话池里**没有它的字节**"时出现 ——
//! 「删掉它会把一个'渲染必然失败'的工程交出去」。
//! 因此本判据修的是**池没被带过去**，⛔ 不是退那个键。
//!
//! ## 判据
//!
//! | # | 判据 | 被什么注入破坏 |
//! | :--- | :--- | :--- |
//! | 1 | GUI 导入（[`crate::import_audio::import_audio_file`]）的字节进活会话池 | 导入不落池 |
//! | 2 | 经**内嵌装配**导出 ⇒ `unsupported == []` | 挂载不传池 |
//! | 3 | 同一导出 ⇒ `audioClipsUnrendered == 0` ＋ `bytesPresent == true` | 只写读数不渲染 |
//! | 4 | 母带真的有声（段 RMS > 0.1） | 池到了但解码/落位没接 |
//! | 5 | 正对照：**无音频片段**的工程 ⇒ 带池/不带池 `sha256` 相同 | 池改了无关工程的字节 |

use serde_json::{Value, json};

use yeban_mcp::security::Channel;
use yeban_model::{AssetHash, AssetMetadata, ClipPlacement, LoopConfig, MediaKind, Op, TrackKind};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;

use super::embedded_digest::{
    END_TICK, FRAMES, SAMPLE_RATE, add_track, empty_project, fixed, mcp_pcm_payload,
};
use super::embedded_sfz_tools::{Scratch, call_tool};
use super::session_dispatcher;
use crate::import_audio::{AudioImportOptions, import_audio_file};
use crate::undo::{UndoPort, UndoSession};

/// 注入时钟（与内嵌装配的既有夹具同一个值）。
const NOW_MS: u64 = 1_760_000_000_000;
/// 素材帧数：1 秒 @ 48 kHz（工程末端 2 秒 ⇒ 后半截是静音，可用于段能量对账）。
const ASSET_FRAMES: usize = 48_000;
/// 24-bit 量化尺度 `2^(bits-1)`。
const I24_SCALE: f64 = 8_388_608.0;

/// 现造一份**后半截反相的常量电平** WAV（左右同值，峰 0.5）。
///
/// 用 f32 WAV：`yeban-decode` 对 f32 WAV 是逐位透传 ⇒ 母带里的样本可与写进去的直接比。
fn write_asset(scratch: &Scratch) -> std::path::PathBuf {
    let path = scratch.join("take.wav");
    let samples: Vec<f32> = (0..ASSET_FRAMES)
        .flat_map(|index| {
            let value = if index < ASSET_FRAMES / 2 { 0.5 } else { -0.5 };
            [value, value]
        })
        .collect();
    write_plain_wav(
        &path,
        &PcmFormat::float(2, SAMPLE_RATE, 32),
        &PcmBuffer::Float32(samples),
    )
    .expect("写夹具 WAV");
    path
}

/// 母带前 1 秒的**段 RMS**（24-bit 整数码 ⇒ `[-1,1)` 尺度）。
fn segment_rms(payload: &[u8]) -> f64 {
    let mut sum = 0.0_f64;
    let mut count = 0_u64;
    // 24-bit 载荷 ⇒ 每 3 字节一个样本（`as_chunks` 让长度前提成为类型事实）。
    for chunk in payload.as_chunks::<3>().0 {
        let raw = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], 0]);
        let extended = (raw << 8) >> 8; // 24-bit 符号扩展
        let value = f64::from(extended) / I24_SCALE;
        sum += value * value;
        count += 1;
    }
    assert!(count > 0, "母带必须带载荷");
    (sum / f64::from(u32::try_from(count).expect("样本数放得进 u32"))).sqrt()
}

/// 渲染一次：**内嵌装配**（生产符号）＋ `yeban_render_master`。
fn mount_and_render(
    project: &yeban_model::YebanProjectV1,
    assets: Vec<(AssetHash, Vec<u8>)>,
    scratch: &Scratch,
    tag: &str,
) -> Value {
    let session_path = scratch.join(&format!("{tag}.session.yeban"));
    let mut dispatcher = session_dispatcher(project, &session_path, false, assets)
        .expect("内嵌会话装配（生产路径的同一个符号）");
    let out = scratch.join(&format!("{tag}.master.wav"));
    let result = call_tool(
        &mut dispatcher,
        Channel::Http,
        "yeban_render_master",
        json!({
            "format": "wav",
            "sampleRate": SAMPLE_RATE,
            "frames": FRAMES,
            "normalize": false,
            "path": out.display().to_string(),
        }),
    );
    assert_eq!(result["status"], "success", "{tag}: 渲染必须成功: {result}");
    result
}

/// ⭐ 判据 ①②③④：GUI 导入的音频 ⇒ 经内嵌 MCP 导出 ⇒ 真的进母带。
#[test]
fn the_embedded_mount_carries_the_session_asset_pool_into_the_render() {
    let scratch = Scratch::new("292-pool");
    let wav = write_asset(&scratch);

    // ① GUI 路径：空工程 ＋ 一条音频轨 ⇒ 导入（与界面同一个函数）。
    let mut project = empty_project();
    let track = add_track(&mut project, 2, TrackKind::Audio, "Audio");
    let session = UndoSession::open("<test:292>", "yeban-app", project, NOW_MS).expect("开会话");
    let port = UndoPort::new(session);
    let report = import_audio_file(
        &port,
        &wav,
        &port.project(),
        &AudioImportOptions::default(),
        NOW_MS,
    )
    .expect("GUI 导入必须成功");
    // GUI 的字节登记口（`host.rs` 保存路径读的是同一个池）。
    port.replace_assets([(report.asset.clone(), report.bytes.clone())]);
    // 摆到轨上 ＋ 在工程文档里**声明**这条资产（导入只建池条目；声明与摆放是
    // 界面那一步，与工具侧 `yeban_import_audio` 的 `AddClip + AddClipPlacement +
    // SetAsset` 同一批口径）。
    port.commit_ops(
        NOW_MS,
        "declare audio asset and place it",
        vec![
            Op::SetAsset {
                asset_hash: report.asset.clone(),
                old_asset: None,
                new_asset: AssetMetadata {
                    hash: report.asset.clone(),
                    original_path: wav.display().to_string(),
                    byte_len: u64::try_from(report.bytes.len()).expect("字节数放得进 u64"),
                    media_kind: MediaKind::Audio,
                    license: "test fixture (generated in-memory)".to_owned(),
                },
            },
            Op::AddClipPlacement {
                track_id: track,
                placement: ClipPlacement {
                    id: fixed(40),
                    clip_id: report.clip_id,
                    start_tick: 0,
                    duration_ticks: END_TICK,
                    loop_config: LoopConfig::default(),
                    muted: false,
                },
            },
        ],
    )
    .expect("声明与摆放必须被接受");

    let result = mount_and_render(&port.project(), port.assets(), &scratch, "with-pool");
    let data = &result["data"];

    // ② 不再登记 `audioClips`。
    assert_eq!(
        data["unsupported"],
        json!([]),
        "池被带进会话 ⇒ 不该再有 unsupported: {data}"
    );
    // ③ 片段真的被渲染，而不是"假装渲染"。
    assert_eq!(data["audio"]["clipsRendered"], 1, "{data}");
    assert_eq!(data["audio"]["assets"][0]["bytesPresent"], true, "{data}");
    assert_eq!(data["sources"][0]["audioClipsUnrendered"], 0, "{data}");
    assert_eq!(data["sources"][0]["audioClips"], 1, "{data}");

    // ④ 母带真的有声：前 1 秒的段 RMS ≈ 0.5，后 1 秒 ≈ 0。
    let payload = mcp_pcm_payload(&scratch.join("with-pool.master.wav"));
    let frames = payload.len() / 3 / 2;
    let head = segment_rms(&payload[..(frames.min(48_000) * 6)]);
    let tail = segment_rms(&payload[(frames.min(48_000) * 6)..]);
    println!(
        "### #292 导出读数: unsupported={} clipsRendered={} bytesPresent={} \
peakAfter={} sha256={} digest={} rms[0,1s)={head:.4} rms[1,2s)={tail:.6}",
        data["unsupported"],
        data["audio"]["clipsRendered"],
        data["audio"]["assets"][0]["bytesPresent"],
        data["peak"]["after"],
        data["sha256"],
        data["masterDigest"],
    );
    assert!(
        head > 0.1,
        "前 1 秒的段 RMS 只有 {head} —— 音频片段没进母带（只报了读数）"
    );
}

/// ⭐ 判据 ⑤：**无音频片段**的工程 ⇒ 带池与不带池的 `sha256` 相同。
///
/// 正对照的对象是"池的传递"这件事本身：池里有字节，但没有任何片段引用它 ⇒
/// 母带必须与空池逐字节相同（⛔ 池不是隐性输入）。
#[test]
fn a_project_without_audio_clips_is_byte_identical_with_and_without_the_pool() {
    let scratch = Scratch::new("292-control");
    // 池里**有**字节（就是上面那份夹具 WAV），但工程里没有任何片段引用它。
    let wav = write_asset(&scratch);
    let bytes = std::fs::read(&wav).expect("读夹具 WAV");
    let hash = AssetHash::of_bytes(&bytes);
    let mut project = empty_project();
    let _track = add_track(&mut project, 2, TrackKind::Audio, "Audio");
    project.validate().expect("夹具合法");

    let carried = mount_and_render(&project, vec![(hash, bytes)], &scratch, "carried");
    // 同一个工程、同一条装配路径，任务单 `#292` 之前的形态（空池）：
    // 这里用 `Domain::open_in_memory` 直接造，⛔ 不复制装配代码。
    let mut dispatcher = yeban_mcp::Dispatcher::new(
        yeban_mcp::security::BearerToken::generate().token,
        yeban_mcp::security::ScopeSet::all(),
        yeban_mcp::security::RunMode::Production,
    );
    dispatcher
        .domain_mut()
        .open_in_memory(scratch.join("empty.session.yeban"), project, false)
        .expect("空池会话");
    let out = scratch.join("empty.master.wav");
    let empty = call_tool(
        &mut dispatcher,
        Channel::Http,
        "yeban_render_master",
        json!({
            "format": "wav",
            "sampleRate": SAMPLE_RATE,
            "frames": FRAMES,
            "normalize": false,
            "path": out.display().to_string(),
        }),
    );
    assert_eq!(empty["status"], "success", "{empty}");

    let carried = &carried["data"];
    let empty = &empty["data"];
    assert_eq!(carried["unsupported"], json!([]), "{carried}");
    assert_eq!(
        carried["sha256"], empty["sha256"],
        "没有音频片段的工程不该因为池里有字节就改变母带字节"
    );
    assert_eq!(carried["masterDigest"], empty["masterDigest"]);
}
