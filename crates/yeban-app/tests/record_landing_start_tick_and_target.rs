//! ⭐⭐ **T-FIX-239 的判据：录音落到起录 tick ＋ 选中的／待录的音频轨。**
//!
//! ## 它补的是哪一格
//!
//! 修前：停录分支读**停录时**的播放头当摆放起点（走带在录音期间往前走了 ⇒
//! 片段整体右移一段自己的长度）；目标轨恒为**第一条音频轨**（BTreeMap 键序）。
//! 本单之后：起点是起录时捕获的 `起录播放头 + 预备拍 tick`
//! （`RecordReport::start_tick`）；目标轨是起录时解析的 `RecordTarget`
//! （待录 ＞ 选中 ＞ 工程只有一条音频轨；多条且无选择 ⇒ 响亮地拒绝）。
//!
//! ## 判据与验收标准的对应
//!
//! | 验收 | 断言 | 证什么 |
//! | :--- | :--- | :--- |
//! | ① 起点是起录 tick | 走带在录音期间前进 ≥ 1 小节；停录后唯一新摆放的 `start_tick == 起录 tick` 且 `!= 停录 tick` | 落点不被走带拖走 |
//! | ①' 预备拍 | `count_in_bars = 1` 时 `start_tick == 起录 tick + 预备拍 tick` | 第一帧真正被推入的位置才是起点 |
//! | ② 选中轨 | 两条音频轨，选中第二条 ⇒ 摆放在第二条（⛔ 不是键序第一条） | 选中真的决定目标轨 |
//! | ②' 待录轨（生产 `track-arms`） | 两条音频轨，待录 Vox 行 ⇒ 摆放在那条轨 | 生产待录真的决定目标轨 |
//! | ②'' 待录轨（旧式 `TrackData.armed`） | 无头装配窗口里待录行 ⇒ 摆放在那条轨 | 旧式待录也被落点方认 |
//! | ②''' 无选择 | 两条音频轨，既不选也不待录 ⇒ 响亮的错误（报告含"没有进工程"＋"被选中"），工程一位不动 | ⛔ 不静默落到第一条 |
//! | ②'''' 单轨 | 只有一条音频轨，不选不待录 ⇒ 仍落到那条轨 | "没得选"不是"静默猜" |
//!
//! ## 无头
//!
//! 窗口用 Tier-1 无头平台；输入源是**脚本化非静音源**（⛔ 不碰声卡）；WAV 落
//! `std::env::temp_dir()`，`Drop` 清理。播放头是 `Rc<Cell<u64>>`（判据在起录与
//! 停录之间拨动它 ⇒ 走带前进可复现）。

#![allow(unused)]
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use slint::Model as _;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::record_device::InputSourceFactory;
use yeban_app::record_wav::{InputSource, RecordOptions, RecordPlan};
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::samples::filled_project;
use yeban_model::{ClipPlacement, EntityId, TrackKind, YebanProjectV1};

const NOW: u64 = 1_760_000_000_000;
const WINDOW: yeban_ui_test_port::Size = yeban_ui_test_port::Size::new(1440, 900);
/// 起录时的播放头（判据固定值；产品路径来自引擎的 `position_ticks`）。
const PRESS_TICK: u64 = 960;
/// 一小节 4/4 的 tick（PPQ 960）。
const ONE_BAR_TICKS: u64 = 3840;

/// 一个临时目录，`Drop` 时清理（**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-239-{tag}-{}-{}",
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

/// 建环参数（与 `record_places_clip_on_track.rs` 同一条；⛔ 不另发明一套）。
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

/// 判据的工程夹具：`filled_project()`（含**一条音频轨**）＋ 清空**全部**摆放 ＋
/// 预备拍清零（`count_in_bars = 0` 的那几条用它；预备拍那条另外改）。
fn fixture_project() -> YebanProjectV1 {
    let mut project = filled_project();
    project.transport.count_in_bars = 0;
    for track in project.tracks.values_mut() {
        track.clips.clear();
    }
    project
}

/// 两条音频轨的夹具工程：`fixture_project()` ＋ 新加一条音频轨 "Vox"。
/// 返回 `(工程, 键序第一条音频轨, 新加的那条)`。
fn two_audio_tracks_project() -> (YebanProjectV1, EntityId, EntityId) {
    let mut project = fixture_project();
    let first = project
        .tracks
        .iter()
        .find(|(id, track)| **id != project.master_bus_track_id && track.kind == TrackKind::Audio)
        .map(|(id, _)| *id)
        .expect("夹具必须有音频轨");
    let second = EntityId::new();
    project.tracks.insert(
        second,
        yeban_model::TrackV3 {
            id: second,
            name: "Vox".to_owned(),
            kind: TrackKind::Audio,
            ..yeban_model::TrackV3::default()
        },
    );
    (project, first, second)
}

/// 全工程的摆放：`(轨道 id, 摆放)`。
fn all_placements(project: &YebanProjectV1) -> Vec<(EntityId, ClipPlacement)> {
    project
        .tracks
        .iter()
        .flat_map(|(track_id, track)| {
            track
                .clips
                .values()
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

/// 观察端口：记下产品落点收到的每一条文本（内部**仍**转调产品落点）。
#[derive(Debug, Default)]
struct SpyOutcome {
    reports: std::cell::RefCell<Vec<Option<String>>>,
}

impl host::RecordOutcomePort for SpyOutcome {
    fn report(&self, ui: &yeban_app::ui::MainWindow, error: Option<String>) {
        self.reports.borrow_mut().push(error.clone());
        host::UiRecordOutcome.report(ui, error);
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
        UndoSession::open("record-landing-239", "yeban-app", project, NOW).expect("打开撤销会话"),
    ));
    (tier1, ui, port)
}

/// 一次真的录音：起录（播放头在 `press`）⇒ 拨动播放头到 `stop_at`
/// （模拟走带在录音期间前进）⇒ 停录（停录内同步落地）。
///
/// `outcome` 为 `None` ⇒ 产品落点；`Some` ⇒ 观察端口。
fn record_once(
    ui: &yeban_app::ui::MainWindow,
    port: &Rc<UndoPort>,
    project: &YebanProjectV1,
    path: &std::path::Path,
    press: u64,
    stop_at: u64,
    outcome: Rc<dyn host::RecordOutcomePort>,
) {
    let _ = std::fs::remove_file(path);
    let playhead = Rc::new(Cell::new(press));
    let playhead_for_closure = Rc::clone(&playhead);
    host::wire_record_at(
        ui,
        Some(Rc::new(std::cell::RefCell::new(project.clone()))),
        Some(Rc::clone(port)),
        RecordPlan {
            path: path.to_path_buf(),
            options: options(),
        },
        Rc::new(ToneFactory),
        outcome,
        Rc::new(move || playhead_for_closure.get()),
    );
    ui.invoke_toggle_record();
    assert!(ui.get_recording(), "起录必须成功（工厂能开）");
    std::thread::sleep(Duration::from_millis(80));
    // ⭐ 走带在录音期间前进：停录时播放头已经不在起录的位置了。
    playhead.set(stop_at);
    std::thread::sleep(Duration::from_millis(80));
    ui.invoke_toggle_record();
    assert!(!ui.get_recording(), "停录之后必须归零");
}

/// ⭐ **验收 ①**：走带在录音期间前进 ≥ 1 小节；停录后摆放的起点是**起录 tick**，
/// ⛔ 不是停录 tick。
#[test]
fn start_tick_is_the_press_time_playhead_not_the_stop_time_playhead() {
    let scratch = Scratch::new("start-tick");
    let project = fixture_project();
    let only_audio = project
        .tracks
        .iter()
        .find(|(id, track)| **id != project.master_bus_track_id && track.kind == TrackKind::Audio)
        .map(|(id, _)| *id)
        .expect("夹具必须有音频轨");
    let (_tier1, ui, port) = assemble(project.clone());
    let path = scratch.dir.join("take.wav");
    let stop_tick = PRESS_TICK + ONE_BAR_TICKS;
    record_once(
        &ui,
        &port,
        &project,
        &path,
        PRESS_TICK,
        stop_tick,
        Rc::new(host::UiRecordOutcome),
    );

    let landed = port.try_project().expect("会话在场");
    let placements = all_placements(&landed);
    assert_eq!(
        placements.len(),
        1,
        "⭐ 停录后全工程必须恰好一条新摆放（实测 {} 条）",
        placements.len()
    );
    let (track_id, placement) = placements[0];
    assert_eq!(track_id, only_audio, "⭐ 单音频轨工程 ⇒ 落到那条音频轨");
    assert_eq!(
        placement.start_tick, PRESS_TICK,
        "⭐ 摆放起点必须是**起录 tick** {PRESS_TICK}（实测 {}）",
        placement.start_tick
    );
    assert_ne!(
        placement.start_tick, stop_tick,
        "⭐ 摆放起点 ⛔ 不能是停录 tick {stop_tick}"
    );
}

/// ⭐ **验收 ①'**：`count_in_bars = 1` 时，起点 = 起录 tick ＋ 预备拍 tick。
#[test]
fn count_in_ticks_are_added_to_the_press_time_playhead() {
    let scratch = Scratch::new("count-in-tick");
    let mut project = fixture_project();
    project.transport.count_in_bars = 1;
    project.bpm = 480.0;
    // 1 小节 4/4 @480bpm = 0.5 s 真实时间；等 1 s ⇒ 预备拍必定走完。
    let (_tier1, ui, port) = assemble(project.clone());
    let path = scratch.dir.join("take.wav");
    let _ = std::fs::remove_file(&path);
    let playhead = Rc::new(Cell::new(PRESS_TICK));
    let playhead_for_closure = Rc::clone(&playhead);
    host::wire_record_at(
        &ui,
        Some(Rc::new(std::cell::RefCell::new(project.clone()))),
        Some(Rc::clone(&port)),
        RecordPlan {
            path: path.to_path_buf(),
            options: options(),
        },
        Rc::new(ToneFactory),
        Rc::new(host::UiRecordOutcome),
        Rc::new(move || playhead_for_closure.get()),
    );
    ui.invoke_toggle_record();
    assert!(ui.get_recording(), "起录必须成功（工厂能开）");
    std::thread::sleep(Duration::from_millis(1000));
    ui.invoke_toggle_record();
    assert!(!ui.get_recording(), "停录之后必须归零");

    let landed = port.try_project().expect("会话在场");
    let placements = all_placements(&landed);
    assert_eq!(
        placements.len(),
        1,
        "⭐ 停录后全工程必须恰好一条新摆放（实测 {} 条）",
        placements.len()
    );
    let (_, placement) = placements[0];
    assert_eq!(
        placement.start_tick,
        PRESS_TICK + ONE_BAR_TICKS,
        "⭐ 预备拍 1 小节 ⇒ 起点 = 起录 tick {PRESS_TICK} ＋ 3840（实测 {}）",
        placement.start_tick
    );
}

/// ⭐ **验收 ②**：两条音频轨，选中第二条 ⇒ 落到第二条（⛔ 不是键序第一条）。
#[test]
fn landing_goes_to_the_selected_audio_track_not_the_first() {
    let scratch = Scratch::new("selected-track");
    let (project, first, second) = two_audio_tracks_project();
    assert_ne!(first, second, "夹具必须有两条不同的音频轨");
    let (_tier1, ui, port) = assemble(project.clone());
    ui.set_selected_track_id(second.to_canonical_string().into());
    let path = scratch.dir.join("take.wav");
    record_once(
        &ui,
        &port,
        &project,
        &path,
        PRESS_TICK,
        PRESS_TICK + ONE_BAR_TICKS,
        Rc::new(host::UiRecordOutcome),
    );

    let landed = port.try_project().expect("会话在场");
    let placements = all_placements(&landed);
    assert_eq!(
        placements.len(),
        1,
        "⭐ 停录后全工程必须恰好一条新摆放（实测 {} 条）",
        placements.len()
    );
    let (track_id, _) = placements[0];
    assert_eq!(
        track_id, second,
        "⭐ 选中第二条音频轨 ⇒ 必须落到第二条（实测落到 {track_id}，第一条是 {first}）"
    );
}

/// 按名字 "Vox" 在界面轨道行里找到那一行，并核对"行序 → EntityId"映射
/// 与 `display_track_ids()` 一致。返回行号（0-based）。
fn find_vox_row(
    ui: &yeban_app::ui::MainWindow,
    project: &YebanProjectV1,
    second: EntityId,
) -> usize {
    let display_ids = project.display_track_ids();
    let tracks_model = ui.get_tracks();
    assert_eq!(
        tracks_model.row_count(),
        display_ids.len(),
        "⭐ 界面轨道行数必须与显示顺序一致（实测 {} 行，显示顺序 {} 条）",
        tracks_model.row_count(),
        display_ids.len()
    );
    for (index, id) in display_ids.iter().enumerate() {
        let row = tracks_model.row_data(index).expect("行必须可读");
        if row.name == "Vox" {
            assert_eq!(
                row.kind, "AUDIO",
                "⭐ Vox 行必须是 AUDIO（实测 {}）",
                row.kind
            );
            assert_eq!(
                *id, second,
                "⭐ 行序到 EntityId 的映射必须与 display_track_ids 一致"
            );
            return index;
        }
    }
    panic!("界面里必须有一行叫 Vox");
}

/// ⭐ **验收 ②'（生产路径）**：两条音频轨，待录 Vox 行 ⇒ 落到 Vox 轨。
///
/// 生产装配里待录按钮写的是 `track-arms`（`wire_track_arm` 维护；
/// `wire_aether_actions` 把 `track-toggle-arm` 转发到它）。本判据走
/// 同一条生产链：`wire_track_arm` ＋ `invoke_track_arm_toggle`。
#[test]
fn landing_goes_to_the_armed_audio_track_not_the_first() {
    let scratch = Scratch::new("armed-track");
    let (project, _, second) = two_audio_tracks_project();
    let (_tier1, ui, port) = assemble(project.clone());
    host::wire_track_arm(&ui);
    let vox_row = find_vox_row(&ui, &project, second);
    ui.invoke_track_arm_toggle(i32::try_from(vox_row).expect("行号"));
    assert!(
        ui.get_track_arms()
            .row_data(vox_row)
            .expect("track-arms 必须可读"),
        "⭐ 点待录之后 track-arms 那一位必须真的 true"
    );
    let path = scratch.dir.join("take.wav");
    record_once(
        &ui,
        &port,
        &project,
        &path,
        PRESS_TICK,
        PRESS_TICK + ONE_BAR_TICKS,
        Rc::new(host::UiRecordOutcome),
    );

    let landed = port.try_project().expect("会话在场");
    let placements = all_placements(&landed);
    assert_eq!(
        placements.len(),
        1,
        "⭐ 停录后全工程必须恰好一条新摆放（实测 {} 条）",
        placements.len()
    );
    let (track_id, _) = placements[0];
    assert_eq!(
        track_id, second,
        "⭐ 待录 Vox 轨 ⇒ 必须落到 Vox（实测落到 {track_id}）"
    );
}

/// ⭐ **验收 ②''（旧式路径）**：无头装配／判据窗口里 `wire_aether_actions`
/// 没接管时，待录按钮翻的是 `TrackData.armed`（`wire_aether_callbacks` 的
/// 旧处理器）；落点方同样认它。
///
/// ⚠ 旧处理器会**整个替换**轨道模型 ⇒ 断言必须重新 `ui.get_tracks()` 取新
/// 模型（⛔ 不能拿点待录之前的那一份句柄重读）。
#[test]
fn landing_goes_to_the_row_armed_audio_track_in_a_bare_window() {
    let scratch = Scratch::new("armed-track-legacy");
    let (project, _, second) = two_audio_tracks_project();
    let (_tier1, ui, port) = assemble(project.clone());
    let vox_row = find_vox_row(&ui, &project, second);
    ui.invoke_track_toggle_arm(i32::try_from(vox_row + 1).expect("行号"));
    assert!(
        ui.get_tracks().row_data(vox_row).expect("行必须可读").armed,
        "⭐ 点待录之后那一行必须真的 armed"
    );
    let path = scratch.dir.join("take.wav");
    record_once(
        &ui,
        &port,
        &project,
        &path,
        PRESS_TICK,
        PRESS_TICK + ONE_BAR_TICKS,
        Rc::new(host::UiRecordOutcome),
    );

    let landed = port.try_project().expect("会话在场");
    let placements = all_placements(&landed);
    assert_eq!(
        placements.len(),
        1,
        "⭐ 停录后全工程必须恰好一条新摆放（实测 {} 条）",
        placements.len()
    );
    let (track_id, _) = placements[0];
    assert_eq!(
        track_id, second,
        "⭐ 待录 Vox 轨 ⇒ 必须落到 Vox（实测落到 {track_id}）"
    );
}

/// ⭐ **验收 ②'''**：两条音频轨，既不选也不待录 ⇒ 响亮地拒绝，工程一位不动。
#[test]
fn no_selected_or_armed_track_with_multiple_audio_tracks_is_a_loud_error() {
    let scratch = Scratch::new("no-selection");
    let (project, _, _) = two_audio_tracks_project();
    let (_tier1, ui, port) = assemble(project.clone());
    let before = port.try_project().expect("会话在场");
    let clip_pool_before = before.clip_pool.len();
    let outcome = Rc::new(SpyOutcome::default());
    let path = scratch.dir.join("take.wav");
    record_once(
        &ui,
        &port,
        &project,
        &path,
        PRESS_TICK,
        PRESS_TICK + ONE_BAR_TICKS,
        outcome.clone(),
    );

    let reports = outcome.reports.borrow();
    let errors: Vec<&String> = reports
        .iter()
        .filter_map(|report| report.as_ref())
        .collect();
    assert!(
        !errors.is_empty(),
        "⭐ 没有目标轨 ⇒ 产品落点必须收到一条**非空错误**（实测 reports={reports:?}）"
    );
    let error = errors.last().expect("至少一条错误").as_str();
    assert!(
        error.contains("没有进工程"),
        "⭐ 错误必须说清\"录音已落盘但没有进工程\"（实测 {error}）"
    );
    assert!(
        error.contains("被选中"),
        "⭐ 错误必须说清\"没有一条被选中／待录\"（实测 {error}）"
    );
    assert!(
        std::path::Path::new(&path).exists(),
        "⭐ WAV 必须还在磁盘上（⛔ 不丢）"
    );
    let landed = port.try_project().expect("会话在场");
    assert_eq!(
        all_placements(&landed).len(),
        0,
        "⭐ 拒绝时 ⛔ 不许有任何摆放"
    );
    assert_eq!(
        landed.clip_pool.len(),
        clip_pool_before,
        "⭐ 拒绝时 clip_pool 一位不动（⛔ 不登记片段）"
    );
}

/// ⭐ **验收 ②''''**：只有一条音频轨，不选不待录 ⇒ 仍落到那条轨。
#[test]
fn single_audio_track_without_selection_still_lands() {
    let scratch = Scratch::new("single-track");
    let project = fixture_project();
    let only_audio = project
        .tracks
        .iter()
        .find(|(id, track)| **id != project.master_bus_track_id && track.kind == TrackKind::Audio)
        .map(|(id, _)| *id)
        .expect("夹具必须有音频轨");
    let (_tier1, ui, port) = assemble(project.clone());
    let path = scratch.dir.join("take.wav");
    record_once(
        &ui,
        &port,
        &project,
        &path,
        PRESS_TICK,
        PRESS_TICK + ONE_BAR_TICKS,
        Rc::new(host::UiRecordOutcome),
    );

    let landed = port.try_project().expect("会话在场");
    let placements = all_placements(&landed);
    assert_eq!(
        placements.len(),
        1,
        "⭐ 停录后全工程必须恰好一条新摆放（实测 {} 条）",
        placements.len()
    );
    let (track_id, placement) = placements[0];
    assert_eq!(track_id, only_audio, "⭐ 单音频轨工程 ⇒ 落到那条音频轨");
    assert_eq!(
        placement.start_tick, PRESS_TICK,
        "⭐ 摆放起点必须是起录 tick {PRESS_TICK}（实测 {}）",
        placement.start_tick
    );
}
