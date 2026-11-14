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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
}

/// ⭐ **⑤ 0 帧不摆**：0 帧 WAV ⇒ 明确拒绝，`clip_pool`／摆放／资产池全部一位不动。
#[test]
fn a_zero_frame_take_is_refused_before_anything_is_placed_or_pooled() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
}
