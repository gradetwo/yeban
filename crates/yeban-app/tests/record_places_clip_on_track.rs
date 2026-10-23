//! ⭐⭐ **任务单 `#134` 的判据：停录之后，那段音频被**摆到轨道上**，而且**真的响**。
//!
//! ## 它补的是哪一格
//!
//! `#132` 让录音进 `clip_pool`（`Op::AddClip`），`#133` 让素材池在面板里可见；
//! 但那段音频**不在任何轨道上**（`TrackV3::clips` 为空）⇒ 渲染器从不遍历它。
//! 本片在同一次提交里补一条 `Op::AddClipPlacement`，落在**第一条音频轨**的
//! **播放头 tick** 上。
//!
//! ## 判据与工单 §4 的对应
//!
//! | §4 | 断言 | 证什么 |
//! | :--- | :--- | :--- |
//! | ① 摆放存在 | 停录后全工程只有一条摆放引用新 clip；轨道 id = 第一条音频轨；`start_tick` = 播放头；`duration_ticks` = 帧→tick | 模型里真的有这条摆放 |
//! | ② 它能响 | 用 **app 的离线渲染入口**（`render_core::render_project_f32_with_assets`）渲染 1 秒 ⇒ 非零样本数 > 0、峰值 > 0；**反向腿**：撤掉摆放再渲染 ⇒ 非零样本数 = 0、峰值 = 0 | 摆放真的进了渲染路径，不是"数据里有个字段" |
//! | ③ 帧→tick 正确 | 公式 `ceil(frames × PPQ × bpm / (rate × 60))` ＋ 实测逐字相等；纯函数另有手算对照（模型层 `time_math` 的判据） | 换算方向/单位不是猜的 |
//! | ④ 往返保真 | 保存 ⇒ `open_project_document_file` 重开 ⇒ 摆放 id／轨道／起点／长度逐字相同 | 保存没丢摆放 |
//! | ⑤ 0 帧不摆 | 0 帧 WAV 调 `land_recording_at` ⇒ `EmptyRecording`，`clip_pool` 与 `assets` 一位不动 | 空录音不会被当成一段"可播的静音" |
//!
//! ## 无头（§4 第 7 条）
//!
//! 窗口用 Tier-1 无头平台；输入源是**脚本化非静音源**（⛔ 不碰声卡）；WAV／容器落
//! `std::env::temp_dir()`，`Drop` 清理。纯函数 `frames_to_ticks_ceil` 的判据在
//! `crates/yeban-model/src/time_math.rs`，不需要窗口也不需要会话。
//!
//! ## ⚠ 听得到的边界（如实）
//!
//! 实时引擎快照**不消费** `ClipContent::Audio`（`yeban-engine/src/snapshot.rs` 的
//! `project_schedules` 第 5 条）⇒ 本判据走的是**离线渲染**那条真实路径
//! （`--export-wav` 与录音侧共用 `render_core`）。走带播放那半属引擎层，本单不改。

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use yeban_app::bridge::ViewState;
use yeban_app::export_wav::WavExportOptions;
use yeban_app::host;
use yeban_app::open::open_project_document_file;
use yeban_app::record_device::InputSourceFactory;
use yeban_app::record_wav::{InputSource, RecordOptions, RecordPlan};
use yeban_app::render_core::render_project_f32_with_assets;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::samples::filled_project;
use yeban_model::{
    AssetHash, ClipContent, ClipPlacement, EntityId, Op, OpOrigin, PPQ, TrackKind, YebanProjectV1,
    frames_to_ticks_ceil,
};
use yeban_render::dither::BitDepth;
use yeban_render::wav::read_plain_wav;

const NOW: u64 = 1_760_000_000_000;
const WINDOW: yeban_ui_test_port::Size = yeban_ui_test_port::Size::new(1440, 900);
/// 摆放起点（判据固定值；产品路径来自引擎的 `position_ticks`）。
const PLAYHEAD_TICK: u64 = 960;

/// 一个临时目录，`Drop` 时清理（**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-134-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 建环参数（与 `record_lands_in_pools.rs` 同一条；⛔ 不另发明一套）。
fn options() -> RecordOptions {
    RecordOptions {
        seconds: 1.0,
        sample_rate: 48_000,
        channels: 2,
        capacity_frames: 8192,
        drain_batch: 512,
        ..RecordOptions::default()
    }
}

/// 判据的工程夹具：`filled_project()`（含**音频轨**）＋ 清空**全部**摆放 ＋ 预备拍清零。
///
/// 为什么清空摆放：反向腿要证"撤掉这条摆放 ⇒ 零样本"。若夹具里还留着别的摆放
/// （`filled_project` 自带一条指向假资产的音频摆放与一条 MIDI 摆放），那两条会
/// 让"零样本"这个读数永远不成立（MIDI 那条**真的会响**）。
fn fixture_project() -> YebanProjectV1 {
    let mut project = filled_project();
    project.transport.count_in_bars = 0;
    for track in project.tracks.values_mut() {
        track.clips.clear();
    }
    project
}

/// 第一条音频轨（与被测实现**同一口径**：`BTreeMap` 键序里第一条 `TrackKind::Audio`）。
fn first_audio_track(project: &YebanProjectV1) -> EntityId {
    project
        .tracks
        .iter()
        .find(|(id, track)| **id != project.master_bus_track_id && track.kind == TrackKind::Audio)
        .map(|(id, _)| *id)
        .expect("夹具必须有音频轨")
}

/// 全工程里引用 `clip_id` 的摆放：`(轨道 id, 摆放)`。
fn placements_of(project: &YebanProjectV1, clip_id: EntityId) -> Vec<(EntityId, ClipPlacement)> {
    project
        .tracks
        .iter()
        .flat_map(|(track_id, track)| {
            track
                .clips
                .values()
                .filter(|placement| placement.clip_id == clip_id)
                .map(|placement| (*track_id, *placement))
        })
        .collect()
}

/// ⭐ **脚本化非静音源**：每帧一个非零阶梯值（⛔ 不碰声卡，⛔ 不是静音腿）。
struct ToneSource {
    channels: u16,
    frame: u64,
}

impl InputSource for ToneSource {
    fn channels(&self) -> u16 {
        self.channels
    }

    fn next_block(&mut self, frames: usize, out: &mut [f32]) -> usize {
        let channels = usize::from(self.channels.max(1));
        for index in 0..frames {
            let value = ((self.frame + index as u64) % 32) as f32 / 64.0;
            for channel in 0..channels {
                out[index * channels + channel] = value;
            }
        }
        self.frame += frames as u64;
        frames
    }

    fn label(&self) -> &'static str {
        "criterion-tone-source"
    }
}

struct ToneFactory;

impl InputSourceFactory for ToneFactory {
    fn open(&self, options: &RecordOptions) -> Result<Box<dyn InputSource + Send>, String> {
        Ok(Box::new(ToneSource {
            channels: options.channels.max(1),
            frame: 0,
        }))
    }

    fn label(&self) -> &'static str {
        "criterion-tone-factory"
    }
}

/// 组装一个真的活窗口 ＋ 一个真的活会话（与 `run_gui` 同一条构造入口）。
fn assemble(
    project: YebanProjectV1,
) -> (
    yeban_ui_test_port::render::Tier1Window,
    yeban_app::ui::MainWindow,
    Rc<UndoPort>,
) {
    let tier1 =
        yeban_ui_test_port::render::Tier1Window::install(WINDOW).expect("装 Tier-1 无头平台");
    let view = ViewState::from_project(&project).expect("装配时投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window(&view, &scene).expect("生产窗口构造入口");
    tier1
        .resize(WINDOW)
        .expect("窗口尺寸定下来之后再注入一次布局");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("record-places", "yeban-app", project, NOW).expect("打开撤销会话"),
    ));
    (tier1, ui, port)
}

/// 一次真的录音：起录 ⇒ 等非静音源送进一批帧 ⇒ 停录（停录内同步落地）。
fn record_once(
    ui: &yeban_app::ui::MainWindow,
    port: &Rc<UndoPort>,
    project: &YebanProjectV1,
    path: &std::path::Path,
) {
    let _ = std::fs::remove_file(path);
    host::wire_record_at(
        ui,
        Some(Rc::new(project.clone())),
        Some(Rc::clone(port)),
        RecordPlan {
            path: path.to_path_buf(),
            options: options(),
        },
        Rc::new(ToneFactory),
        Rc::new(host::UiRecordOutcome),
        Rc::new(|| PLAYHEAD_TICK),
    );
    ui.invoke_toggle_record();
    assert!(ui.get_recording(), "起录必须成功（工厂能开）");
    std::thread::sleep(Duration::from_millis(80));
    ui.invoke_toggle_record();
    assert!(!ui.get_recording(), "停录之后必须归零");
}

/// 渲染 1 秒，返回 `(非零样本数, 峰值, 帧数, 音频摆放渲染数)`。
fn render_one_second(
    project: &YebanProjectV1,
    asset: &AssetHash,
    bytes: &[u8],
) -> (usize, f32, u64, u64) {
    let output = render_project_f32_with_assets(
        project,
        WavExportOptions {
            frames: 48_000,
            sample_rate: 48_000,
            channels: 2,
            seed: 1,
            use_engine_source: true,
            bit_depth: BitDepth::Int24,
            limiter: false,
        },
        None,
        &[(asset.clone(), bytes.to_vec())],
    )
    .expect("离线渲染必须成功");
    let nonzero = output
        .samples
        .iter()
        .filter(|sample| **sample != 0.0)
        .count();
    let peak = output
        .samples
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    assert!(
        output.audio_clips_skipped.is_empty(),
        "采样率一致 ⇒ 不许跳过音频摆放: {:?}",
        output.audio_clips_skipped
    );
    (nonzero, peak, output.frames, output.audio_clips_rendered)
}

/// ⭐⭐ **① 摆放存在 ＋ ② 能响 ＋ ③ 帧→tick ＋ ④ 往返保真**（一条真实停录路径）。
#[test]
fn a_finished_recording_is_placed_on_the_first_audio_track_and_renders_audibly() {
    let scratch = Scratch::new("main");
    let wav = scratch.dir.join("take.wav");
    let container = scratch.dir.join("placed.yeban");
    let project = fixture_project();
    let audio_track = first_audio_track(&project);
    let clips_before = project.clip_pool.len();

    let (_tier1, ui, port) = assemble(project.clone());
    record_once(&ui, &port, &project, &wav);

    // ---- ① 摆放存在：新 clip 恰好一条摆放 ----
    let reading = port.project();
    assert_eq!(
        reading.clip_pool.len(),
        clips_before + 1,
        "停录必须让 clip_pool +1"
    );
    let wav_bytes = std::fs::read(&wav).expect("停录必须落盘");
    let wav_sha = AssetHash::of_bytes(&wav_bytes);
    let new_clip = reading
        .clip_pool
        .values()
        .find(
            |entry| matches!(&entry.content, ClipContent::Audio { asset, .. } if *asset == wav_sha),
        )
        .expect("新片段必须是 Audio 且指向 WAV 的 sha256");
    let ClipContent::Audio { asset, gain_db } = &new_clip.content else {
        unreachable!("上面已经匹配 Audio")
    };
    assert_eq!(*asset, wav_sha);
    assert_eq!(*gain_db, 0.0);

    let placed = placements_of(&reading, new_clip.id);
    assert_eq!(
        placed.len(),
        1,
        "停录之后必须恰好一条摆放引用那条 clip，实得 {}",
        placed.len()
    );
    let (track_id, placement) = &placed[0];
    assert_eq!(*track_id, audio_track, "必须落在第一条音频轨上");
    assert_eq!(placement.start_tick, PLAYHEAD_TICK, "起点 = 播放头 tick");
    assert!(!placement.id.is_nil(), "摆放身份必须非空");

    // ---- ③ 帧→tick：公式与实测逐字相等 ----
    let (format, buffer) = read_plain_wav(&wav).expect("读回停录产物");
    #[allow(clippy::cast_possible_truncation)]
    let frames = (buffer.len() as u64) / u64::from(format.channels.max(1));
    let expected_ticks = frames_to_ticks_ceil(frames, PPQ, reading.bpm, format.sample_rate)
        .expect("48 kHz / 120 BPM 必然能换算");
    assert_eq!(
        placement.duration_ticks, expected_ticks,
        "时长必须逐字等于帧→tick（frames={frames}, rate={}, bpm={}）",
        format.sample_rate, reading.bpm
    );
    let bpm = reading.bpm;
    let rate = format.sample_rate;
    println!(
        "[#134] ① 摆放: track={} placement={} clip={} start-tick={} duration-ticks={} \
         (frames={frames} rate={rate} bpm={bpm} ticks=ceil({frames} × {PPQ} × {bpm} / \
         ({rate} × 60))={expected_ticks})",
        track_id, placement.id, new_clip.id, placement.start_tick, placement.duration_ticks,
    );

    // ---- ② 能响：真实离线渲染 ⇒ 非零样本 ----
    let (nonzero, peak, frames_rendered, clips_rendered) =
        render_one_second(&reading, &wav_sha, &wav_bytes);
    assert!(
        nonzero > 0,
        "摆放必须真的进渲染路径（非零样本数 = {nonzero}）"
    );
    assert!(peak > 0.0, "峰值必须大于 0（实得 {peak}）");
    assert_eq!(clips_rendered, 1, "必须恰好渲染 1 个音频摆放");
    println!(
        "[#134] ② 正向渲染: frames={frames_rendered} nonzero-samples={nonzero} peak={peak} \
         audio-clips-rendered={clips_rendered}"
    );

    // ---- ④ 往返保真：保存 ⇒ 重开 ⇒ 摆放逐字相同 ----
    host::wire_save(
        &ui,
        host::SaveStatus {
            target: Some(container.clone()),
            // 刻意的**装配时刻**快照：保存必须从活会话现读，⛔ 不是退回它。
            project: project.clone(),
        },
        Some(Rc::clone(&port)),
        Rc::new(yeban_app::file_picker::NoPicker),
        None,
    );
    ui.invoke_save_project();
    let status = ui.get_save_status().to_string();
    assert!(status.contains("已保存"), "保存必须真的落盘: `{status}`");
    let reopened =
        open_project_document_file(&container, &Default::default()).expect("重新打开容器");
    let reopened_placed = placements_of(&reopened.archive.project, new_clip.id);
    assert_eq!(
        reopened_placed.len(),
        1,
        "重新打开之后那条摆放必须还在（实得 {}）",
        reopened_placed.len()
    );
    assert_eq!(
        reopened_placed[0], placed[0],
        "往返之后轨道 id 与摆放载荷必须逐字相同"
    );
    assert_eq!(
        reopened_placed[0].1.start_tick, placement.start_tick,
        "起点 tick 必须逐字相同"
    );
    assert_eq!(
        reopened_placed[0].1.duration_ticks, placement.duration_ticks,
        "长度 tick 必须逐字相同"
    );
    println!(
        "[#134] ④ 往返: container={} bytes={} placement={} start={} duration={}",
        container.display(),
        std::fs::metadata(&container).expect("容器").len(),
        reopened_placed[0].1.id,
        reopened_placed[0].1.start_tick,
        reopened_placed[0].1.duration_ticks,
    );

    // ---- ② 反向腿：撤掉摆放 ⇒ 零样本 ----
    port.commit_ops_with_origin(
        NOW + 1,
        OpOrigin::UserUi,
        "criterion: remove the only placement",
        vec![Op::RemoveClipPlacement {
            track_id: *track_id,
            placement_id: placement.id,
            previous_placement: *placement,
        }],
    )
    .expect("撤掉摆放必须被接受");
    let after_removal = port.project();
    assert!(
        placements_of(&after_removal, new_clip.id).is_empty(),
        "反向腿的前提：引用该 clip 的摆放必须为 0 条"
    );
    let (nonzero_removed, peak_removed, _, clips_removed) =
        render_one_second(&after_removal, &wav_sha, &wav_bytes);
    assert_eq!(
        nonzero_removed, 0,
        "去掉摆放 ⇒ 输出必须零样本（实得 {nonzero_removed}）"
    );
    assert_eq!(peak_removed, 0.0, "去掉摆放 ⇒ 峰值必须 0");
    assert_eq!(clips_removed, 0, "去掉摆放 ⇒ 渲染 0 个音频摆放");
    println!(
        "[#134] ② 反向渲染: nonzero-samples={nonzero_removed} peak={peak_removed} \
         audio-clips-rendered={clips_removed}"
    );
}

/// ⭐ **⑤ 0 帧不摆**：0 帧 WAV ⇒ 明确拒绝，`clip_pool`／摆放／资产池全部一位不动。
#[test]
fn a_zero_frame_take_is_refused_before_anything_is_placed_or_pooled() {
    let scratch = Scratch::new("zero");
    let wav = scratch.dir.join("zero.wav");
    // 0 帧的**合法** WAV：记录器拒绝 `seconds == 0`（那是对的），因此直接用本仓唯一
    // 写出器落一个空 data 段 —— 与 `#130` 停录 0 帧时落盘的头是同一个形状。
    yeban_render::wav::write_plain_wav(
        &wav,
        &yeban_render::rf64::PcmFormat::integer(2, 48_000, 16),
        &yeban_render::dither::PcmBuffer::Int16(Vec::new()),
    )
    .expect("落一条 0 帧 WAV");
    let (format, buffer) = read_plain_wav(&wav).expect("0 帧 WAV 仍是合法容器");
    assert_eq!(
        buffer.len(),
        0,
        "夹具必须是 0 帧（rate={}）",
        format.sample_rate
    );

    let project = fixture_project();
    let clips_before = project.clip_pool.len();
    let placements_before: usize = project.tracks.values().map(|track| track.clips.len()).sum();
    let session = UndoSession::open("<test:134-zero>", "yeban-app", project, NOW).expect("开会话");
    let port = UndoPort::new(session);

    let error = yeban_app::record_landing::land_recording_at(&port, &wav, NOW, PLAYHEAD_TICK)
        .expect_err("0 帧必须被拒绝");
    assert!(
        matches!(
            error,
            yeban_app::record_landing::RecordLandingError::EmptyRecording { .. }
        ),
        "必须是 EmptyRecording: {error}"
    );
    let reading = port.project();
    assert_eq!(
        reading.clip_pool.len(),
        clips_before,
        "0 帧 ⛔ 不许 AddClip"
    );
    let placements_after: usize = reading.tracks.values().map(|track| track.clips.len()).sum();
    assert_eq!(
        placements_after, placements_before,
        "0 帧 ⛔ 不许摆放（前 {placements_before}，后 {placements_after}）"
    );
    assert!(
        port.assets().is_empty(),
        "0 帧 ⛔ 不许登记资产，实得 {} 条",
        port.assets().len()
    );
    println!(
        "[#134] ⑤ 0 帧: clips={} placements={} assets={} error=`{error}`",
        reading.clip_pool.len(),
        placements_after,
        port.assets().len()
    );
}
