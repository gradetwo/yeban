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

#![allow(unused)]
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use slint::Model as _;

use yeban_app::bridge::ViewState;
use yeban_app::engine_host::EngineHost;
use yeban_app::export_wav::WavExportOptions;
use yeban_app::host;
use yeban_app::open::{ProjectOpenOptions, open_project_document_file};
use yeban_app::record_device::InputSourceFactory;
use yeban_app::record_landing::{RecordLandingError, RecordTarget, land_recording_at};
use yeban_app::record_wav::{InputSource, RecordOptions, RecordPlan};
use yeban_app::render_core::render_project_f32_with_assets;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::container::write_project_container;
use yeban_model::samples::filled_project;
use yeban_model::{
    AssetHash, ClipContent, ClipPlacement, EntityId, Op, OpOrigin, PPQ, TrackKind, TrackV3,
    YebanProjectV1, frames_to_ticks_ceil,
};
use yeban_render::dither::{BitDepth, PcmBuffer};
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::{read_plain_wav, write_plain_wav};

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
        Some(Rc::new(RefCell::new(project.clone()))),
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
            threads: None,
            loudness: None,
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
    let base = fixture_project();
    let (_tier1, ui, port) = assemble(base.clone());
    let scratch = Scratch::new("placement");
    let path = scratch.dir.join("take.wav");

    record_once(&ui, &port, &base, &path);

    let after = port.project();
    let before_ids: std::collections::BTreeSet<EntityId> = base.clip_pool.keys().copied().collect();
    let new_ids: Vec<EntityId> = after
        .clip_pool
        .keys()
        .copied()
        .filter(|id| !before_ids.contains(id))
        .collect();
    assert_eq!(
        new_ids.len(),
        1,
        "一次停录必须恰好新增一条 clip：{new_ids:?}"
    );
    let clip_id = new_ids[0];
    let ClipContent::Audio { asset, gain_db: _ } = &after.clip_pool[&clip_id].content else {
        panic!("录音片段必须是 ClipContent::Audio");
    };

    let placements = placements_of(&after, clip_id);
    assert_eq!(
        placements.len(),
        1,
        "新 clip 必须恰好有一条摆放：{placements:?}"
    );

    // 这一条是任务单 368 的刷新腿：模型池里有新片段还不够，GUI 的
    // clip-ulids 必须在停录返回时已经重投影；否则用户看到的是录完了但轨道上
    // 什么都没出现。T-FIX-347 已修：停录落地当帧重投影。clip-ulids 装的是
    // 摆放 id（ViewState::clip_ulids 取 placement_id），不是池里的
    // clip id —— 断言按摆放 id 核。
    let placement_id = placements[0].1.id;
    let ui_ids: Vec<String> = ui
        .get_clip_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    assert!(
        ui_ids.iter().any(|id| id == &placement_id.to_string()),
        "停录返回时 GUI 的 clip-ulids 必须包含新摆放：摆放 {placement_id}，GUI {ui_ids:?}"
    );
    let (track_id, placement) = &placements[0];
    assert_eq!(
        *track_id,
        first_audio_track(&after),
        "录音必须落在第一条音频轨"
    );
    assert_eq!(
        placement.start_tick, PLAYHEAD_TICK,
        "摆放起点必须等于停录播放头"
    );
    assert!(placement.duration_ticks > 0, "摆放松弛长度必须为正");

    let (format, buffer) = read_plain_wav(&path).expect("录音 WAV 必须可回读");
    let channels = usize::from(format.channels.max(1));
    let frames = u64::try_from(buffer.len() / channels).expect("帧数");
    let expected_ticks =
        frames_to_ticks_ceil(frames, PPQ, after.bpm, format.sample_rate).expect("帧→tick 必须可算");
    assert_eq!(
        placement.duration_ticks, expected_ticks,
        "摆放时长必须逐字等于 frames_to_ticks_ceil({frames})"
    );

    let assets = port.assets();
    let asset_bytes = assets
        .iter()
        .find(|(hash, _)| hash == asset)
        .map(|(_, bytes)| bytes.clone())
        .expect("录音资产必须进端口资产池");
    let (nonzero, peak, rendered_frames, audio_rendered) =
        render_one_second(&after, asset, &asset_bytes);
    assert!(nonzero > 0, "录音摆放必须真的响：非零样本数 = {nonzero}");
    assert!(peak > 0.0, "录音摆放峰值必须 > 0");
    assert_eq!(rendered_frames, 48_000, "离线渲染必须交付 1 秒");
    assert_eq!(audio_rendered, 1, "必须消费一条音频摆放");

    // 反向腿：撤掉摆放 ⇒ 同样的工程和资产必须渲染成整段静音。
    let mut without = after.clone();
    for track in without.tracks.values_mut() {
        track.clips.clear();
    }
    let (zero_nonzero, zero_peak, _, zero_rendered) =
        render_one_second(&without, asset, &asset_bytes);
    assert_eq!(zero_nonzero, 0, "撤掉摆放后不得有非零样本");
    assert_eq!(zero_peak, 0.0, "撤掉摆放后峰值必须为 0");
    assert_eq!(zero_rendered, 0, "撤掉摆放后不得消费音频摆放");

    // ④ 往返保真：保存容器 ⇒ 重新打开 ⇒ 摆放的轨道/起点/长度逐字相同。
    let roundtrip_path = scratch.dir.join("roundtrip.yeban");
    let assets_map: BTreeMap<AssetHash, Vec<u8>> = assets.into_iter().collect();
    let container = write_project_container(&after, b"", &assets_map).expect("写容器");
    std::fs::write(&roundtrip_path, container).expect("落容器");
    let opened = open_project_document_file(&roundtrip_path, &ProjectOpenOptions::default())
        .expect("重开容器");
    let reopened = opened.archive.project;
    let reopened_placements = placements_of(&reopened, clip_id);
    assert_eq!(
        reopened_placements.len(),
        1,
        "重开后摆放必须仍在：{reopened_placements:?}"
    );
    let (reopened_track, reopened_placement) = &reopened_placements[0];
    assert_eq!(*reopened_track, *track_id);
    assert_eq!(reopened_placement.start_tick, placement.start_tick);
    assert_eq!(reopened_placement.duration_ticks, placement.duration_ticks);
}

/// ⭐ **⑤ 0 帧不摆**：0 帧 WAV ⇒ 明确拒绝，`clip_pool`／摆放／资产池全部一位不动。
#[test]
fn a_zero_frame_take_is_refused_before_anything_is_placed_or_pooled() {
    let base = fixture_project();
    let (_tier1, _ui, port) = assemble(base.clone());
    let scratch = Scratch::new("zero");
    let path = scratch.dir.join("empty.wav");
    write_plain_wav(
        &path,
        &PcmFormat::integer(2, 48_000, 24),
        &PcmBuffer::Int24(Vec::new()),
    )
    .expect("写 0 帧 WAV");

    let before_clips = port.project().clip_pool.len();
    let before_assets = port.assets().len();
    let error = land_recording_at(&port, &path, NOW, PLAYHEAD_TICK, RecordTarget::NoSelection)
        .expect_err("0 帧录音必须被拒绝");
    assert!(
        matches!(error, RecordLandingError::EmptyRecording { .. }),
        "必须是 EmptyRecording，实际 {error:?}"
    );
    let after = port.project();
    assert_eq!(after.clip_pool.len(), before_clips, "0 帧不得新增 clip");
    assert_eq!(port.assets().len(), before_assets, "0 帧不得登记资产");
}

/// 往夹具里加一条**音频**轨（身份取新 ULID ⇒ `BTreeMap` 键序在主 Bass 之后）。
fn add_audio_track(project: &mut YebanProjectV1, name: &str) -> EntityId {
    let id = EntityId::new();
    project.tracks.insert(
        id,
        TrackV3 {
            id,
            name: name.to_owned(),
            kind: TrackKind::Audio,
            ..TrackV3::default()
        },
    );
    id
}

/// ⭐ **Arm 目标轨**：录音必须落在用户 arm 的那条音频轨，而不是永远第一条。
#[test]
fn a_recording_lands_on_the_armed_audio_track() {
    let mut base = fixture_project();
    let armed_id = add_audio_track(&mut base, "Armed Audio");
    let view = ViewState::from_project(&base).expect("夹具投影");
    let armed_index = view
        .tracks
        .iter()
        .position(|track| track.id == armed_id.to_string())
        .expect("新音频轨必须在视图里");

    let (_tier1, ui, port) = assemble(base.clone());
    host::wire_track_arm(&ui);
    ui.invoke_track_arm_toggle(i32::try_from(armed_index).expect("轨索引"));
    assert!(
        ui.get_track_arms()
            .row_data(armed_index)
            .expect("arm 数组必须覆盖全部轨道"),
        "前置：目标轨必须已 arm"
    );

    let scratch = Scratch::new("armed");
    record_once(&ui, &port, &base, &scratch.dir.join("armed.wav"));

    let after = port.project();
    let before_ids: std::collections::BTreeSet<EntityId> = base.clip_pool.keys().copied().collect();
    let new_ids: Vec<EntityId> = after
        .clip_pool
        .keys()
        .copied()
        .filter(|id| !before_ids.contains(id))
        .collect();
    assert_eq!(new_ids.len(), 1, "一次停录必须恰好新增一条 clip");
    let placements = placements_of(&after, new_ids[0]);
    assert_eq!(placements.len(), 1, "arm 轨录音必须恰好有一条摆放");
    assert_eq!(
        placements[0].0, armed_id,
        "录音必须落在 arm 的目标轨，而不是 first_audio_track"
    );
}

/// ⭐ T-FIX-347：落地成功后**当帧**重投影，新片段不等到下一次无关操作。
///
/// 旧代码只提交 `Op::AddClipPlacement`、不调 `apply_view` ⇒ `timeline-blocks`
/// 仍是录音前那一份（本夹具清空过全部摆放，初值 0）；修后停录返回时就必须 > 0。
#[test]
fn a_landed_recording_reprojects_the_timeline_without_a_later_action() {
    let scratch = Scratch::new("t347-reproject");
    let path = scratch.dir.join("t347-take.wav");
    let project = fixture_project();
    let (_tier1, ui, port) = assemble(project.clone());

    assert_eq!(
        ui.get_timeline_blocks().row_count(),
        0,
        "夹具清空摆放 ⇒ 录音前时间线必须是空的"
    );

    record_once(&ui, &port, &project, &path);

    let placements = port
        .project()
        .tracks
        .values()
        .flat_map(|track| track.clips.values())
        .count();
    assert_eq!(placements, 1, "一次停录必须只落一条摆放");
    assert!(
        ui.get_timeline_blocks().row_count() > 0,
        "停录落地后时间线必须立刻出现新片段，而不是等下一次选轨/编辑"
    );
}
/// ⭐ **无音频轨**：录完了仍要保留片段与资产，但**不硬塞**到 MIDI/Aux/Master 上。
#[test]
fn a_recording_without_an_audio_track_is_pooled_but_not_placed() {
    let mut base = fixture_project();
    base.tracks
        .retain(|_, track| track.kind != TrackKind::Audio);
    assert!(
        !base
            .tracks
            .values()
            .any(|track| track.kind == TrackKind::Audio),
        "前置：夹具必须没有音频轨"
    );

    let (_tier1, ui, port) = assemble(base.clone());
    let scratch = Scratch::new("no-audio");
    let path = scratch.dir.join("pool-only.wav");
    record_once(&ui, &port, &base, &path);

    let after = port.project();
    let before_ids: std::collections::BTreeSet<EntityId> = base.clip_pool.keys().copied().collect();
    let new_ids: Vec<EntityId> = after
        .clip_pool
        .keys()
        .copied()
        .filter(|id| !before_ids.contains(id))
        .collect();
    assert_eq!(new_ids.len(), 1, "无音频轨时录音仍必须进 clip_pool");
    assert!(
        placements_of(&after, new_ids[0]).is_empty(),
        "无音频轨时不得硬塞到任何轨道"
    );
    let clip_id = new_ids[0];
    let ClipContent::Audio { asset, .. } = &after.clip_pool[&clip_id].content else {
        panic!("录音片段必须是 ClipContent::Audio");
    };
    assert!(
        port.assets().iter().any(|(hash, _)| hash == asset),
        "无音频轨时资产仍必须进端口资产池"
    );
}
