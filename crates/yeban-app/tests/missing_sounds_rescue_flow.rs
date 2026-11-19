//! 判据：**救援面板的完整闭环真的把声音接回来了**（用户口径：点一下就要能响）。
//!
//! | 腿 | 断言 | 证什么 |
//! | :--- | :--- | :--- |
//! | ① 换成同类 | 点一次「换成 X」⇒ 面板收走、工程里这条轨的 `sampler` 逐字换成那件 | 一次点击真的改了工程 |
//! | ② 能发声 | 换之前那条轨（路径不存在）渲染无输出、换之后**有声**（非零样本 + 峰值 > 0.1 + digest 变） | 不是"界面说好了"，而是**引擎真的拉起了那件音源** |
//! | ③ 红字自愈 | 换完之后 `engine-error` 被清空 | 用户截图里那条"过期红色提醒"不再挂着 |
//! | ④ 下载即换上 | 点「下载 X」（记待办）⇒ 装好那一跳（`rescue_after_pack_installed`）⇒ 自动绑到那条轨、面板收走、**不需要第二次点击** | 用户在音源管理里点的是「下载」而不是「换成」 |
//! | ⑤ 面板让位 | 点「下载」时救援面板先收起、音源管理在前台（`sound-library-open == true`） | 修掉"两个面板叠在一起 ⇒ 看不见下载进度" |
//!
//! ## 它**不**证什么
//!
//! ⛔ 不证"真从 GitHub 下载成功"（那要网络；本文件把"装好"这一跳按它**唯一的入口**
//! `host::rescue_after_pack_installed` 直接调起来 —— 与下载排空那一跳同一个函数）。
//! ⛔ 不证人耳听到的声音（只读样本数组）。

#![allow(unused)]
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::export_wav::WavExportOptions;
use yeban_app::host;
use yeban_app::render_core::render_project_f32_with_assets;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::project::{
    ClipContent, ClipPlacement, ClipPoolEntry, LoopConfig, RoutingEdge, RoutingKind, TrackKind,
    TrackV3, YebanProjectV1,
};
use yeban_model::{EntityId, MidiNote, SamplerBinding};
use yeban_render::dither::{BitDepth, PcmBuffer};
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::Size;

const WINDOW: Size = Size::new(1440, 1000);
const NOW_MS: u64 = 1_760_000_000_000;
const RATE: u32 = 48_000;
const NOTE_TICKS: u64 = 480;

/// 一条**路径不存在**的音源（= 用户打开别人工程时的那种状态）。
const MISSING_PATH: &str = "assets/samples/emily_basic/Programs/emily_basic.sfz";

/// 带真实样本字节的夹具 SFZ（写 WAV + 1 region），用来当"同类的已装候选"。
struct AudioFixture {
    dir: PathBuf,
    sfz: PathBuf,
}

impl AudioFixture {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-rescue-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let wav = dir.join("tone.wav");
        let frames = (RATE / 4) as usize;
        let mut pcm: Vec<i32> = Vec::with_capacity(frames);
        for frame in 0..frames {
            let phase = std::f64::consts::TAU * 440.0 * frame as f64 / f64::from(RATE);
            #[allow(clippy::cast_possible_truncation)]
            let value = (phase.sin() * 0.5 * 8_388_607.0).round() as i32;
            pcm.push(value);
        }
        write_plain_wav(&wav, &PcmFormat::integer(1, RATE, 24), &PcmBuffer::Int24(pcm))
            .expect("写夹具 WAV");
        let sfz = dir.join("tone.sfz");
        std::fs::write(&sfz, "<region> sample=tone.wav key=60 pitch_keycenter=60\n")
            .expect("写夹具 SFZ");
        Self { dir, sfz }
    }

    fn path(&self) -> String {
        self.sfz.to_str().expect("临时路径必须是 UTF-8").to_owned()
    }
}

impl Drop for AudioFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 主总线 ＋ 一条 MIDI 轨（绑着一条**不存在**的 SFZ）＋ 一个音符。
fn project_with_missing_sampler(tag: &str) -> (YebanProjectV1, EntityId) {
    let master = EntityId::new();
    let track_id = EntityId::new();
    let clip_id = EntityId::new();
    let note = MidiNote::new(EntityId::new(), 0, 60, NOTE_TICKS);
    let mut project = YebanProjectV1 {
        master_bus_track_id: master,
        ..YebanProjectV1::default()
    };
    project.tracks.insert(
        track_id,
        TrackV3 {
            id: track_id,
            name: format!("Clean 吉他 {tag}"),
            kind: TrackKind::Midi,
            sampler: Some(SamplerBinding {
                note_octave_override: None,
                path: MISSING_PATH.to_owned(),
            }),
            ..TrackV3::default()
        },
    );
    project.tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    let mut notes = BTreeMap::new();
    notes.insert(note.id, note);
    project.clip_pool.insert(
        clip_id,
        ClipPoolEntry {
            id: clip_id,
            name: "clip".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );
    let placement = ClipPlacement {
        id: EntityId::new(),
        clip_id,
        start_tick: 0,
        duration_ticks: NOTE_TICKS,
        loop_config: LoopConfig::default(),
        muted: false,
        offset_samples: None,
    };
    project
        .tracks
        .get_mut(&track_id)
        .expect("MIDI 轨在表里")
        .clips
        .insert(placement.id, placement);
    project.routing_graph.nodes = vec![track_id, master];
    let edge = EntityId::new();
    project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: track_id,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
            source_lane: None,
            dest_lane: None,
        },
    );
    project.validate().expect("夹具工程必须合法");
    (project, track_id)
}

/// 生产形态的活窗口 ＋ 音源管理/救援面板的**生产接线**。
fn live_window(project: &YebanProjectV1, tag: &str) -> (LivePort<MainWindow>, Rc<UndoPort>) {
    let view = ViewState::from_project(project).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open(format!("<判据 {tag}>"), "yeban-app", project.clone(), NOW_MS)
            .expect("打开撤销会话"),
    ));
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(&view, &scene, 2)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    let ui = live.ui();
    host::wire_sound_library_dialog_with(ui, Some(Rc::clone(&port)));
    let rows = yeban_app::missing_sounds::collect(&port.project(), None);
    assert!(!rows.is_empty(), "夹具必须真的被判成缺音源（否则本判据是空绿）");
    host::wire_missing_sounds_dialog(ui, Rc::clone(&port), rows);
    (live, port)
}

fn render(project: &YebanProjectV1) -> Result<(usize, f32, String), String> {
    let options = WavExportOptions {
        frames: u64::from(RATE) / 2,
        sample_rate: RATE,
        channels: 2,
        seed: 1,
        use_engine_source: true,
        bit_depth: BitDepth::Int24,
        limiter: false,
        threads: None,
        loudness: None,
    };
    let output = render_project_f32_with_assets(project, options, None, &[])
        .map_err(|error| format!("{error:?}"))?;
    let nonzero = output.samples.iter().filter(|sample| **sample != 0.0).count();
    let peak = output
        .samples
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    Ok((nonzero, peak, output.digest))
}

/// ① ② ③：点一次「换成 X」⇒ 工程改了、引擎有声了、红字收了。
#[test]
fn a_swap_from_the_rescue_panel_reaches_the_engine_and_becomes_audible() {
    let (project, track_id) = project_with_missing_sampler("swap");
    let fixture = AudioFixture::new("swap");
    // 换之前：那条轨的音源路径根本不存在 ⇒ 离线渲染**直接失败**（真实现场，不是"静音"）。
    let before = render(&project);
    eprintln!("[rescue/swap] 换之前 => {before:?}");

    let (live, port) = live_window(&project, "swap");
    let ui = live.ui();
    assert!(ui.get_missing_sounds_open(), "打开工程就该弹出救援面板");

    ui.invoke_missing_swap_suggested(0, fixture.path().into(), "Fixture".into());

    eprintln!(
        "[rescue/swap] toast={:?} rows={} panel={} error={:?}",
        ui.get_toast_message().to_string(),
        ui.get_missing_row_count(),
        ui.get_missing_sounds_open(),
        ui.get_engine_error().to_string()
    );

    // ① 工程真的改了（读权威工程，不读界面文案）。
    let after_project = port.project();
    let sampler = after_project
        .tracks
        .get(&track_id)
        .and_then(|track| track.sampler.clone())
        .expect("换完之后必须绑着一件音源");
    assert_eq!(sampler.path, fixture.path(), "绑定的必须正是用户点的那一件");

    // ③ 红字自愈：那条"音源没装好"的横幅不再成立。
    assert_eq!(
        ui.get_engine_error().to_string(),
        "",
        "换好之后引擎横幅必须收掉（否则就是一条过期警报）"
    );
    assert!(!ui.get_missing_sounds_open(), "全换好后面板自己走人");
    assert_eq!(ui.get_missing_row_count(), 0, "行数必须归零");

    // ② 能发声：同一条工程、同一个渲染入口，换之前（渲染失败）vs 换之后（有声）。
    let after = render(&after_project).expect("换好之后渲染必须能跑通");
    eprintln!(
        "[rescue/swap] 换之后 nonzero={} peak={} digest={}",
        after.0, after.1, after.2
    );
    assert!(
        after.0 > 0 && after.1 > 0.1,
        "换完之后必须真的有声：nonzero={} peak={}",
        after.0,
        after.1
    );
    assert!(
        before.is_err() || before.as_ref().is_ok_and(|b| b.2 != after.2),
        "换之前就渲染不出来 / 或换了音源必然是另一份输出：before={before:?} after={after:?}"
    );
}

/// ④ ⑤：点「下载 X」→ 装好那一跳 ⇒ 自动换上，**不需要第二次点击**。
#[test]
fn a_pack_finishing_after_the_rescue_button_binds_the_track_without_a_second_click() {
    // 一件**本机确实装好**的内置音源（用清单一遍扫出来，⛔ 不写死 id）。
    let Some((pack_id, pack_name, pack_path)) = first_installed_pack() else {
        // 这台机器一件内置音源都没有 ⇒ 本判据无从谈起（如实跳过，⛔ 不假装通过）。
        eprintln!("[rescue/download] 本机没有已装的内置音源，跳过");
        return;
    };
    eprintln!("[rescue/download] 用 {pack_id}（{pack_name}）⇒ {pack_path}");

    let (project, track_id) = project_with_missing_sampler("download");
    let (live, port) = live_window(&project, "download");
    let ui = live.ui();
    assert!(ui.get_missing_sounds_open());

    // ⑤ 点「下载」⇒ 救援面板让位，音源管理在前台（否则用户看不见下载进度）。
    ui.invoke_missing_open_pack(0, pack_id.as_str().into(), pack_name.as_str().into());
    assert!(
        !ui.get_missing_sounds_open(),
        "去下载时救援面板必须先收起来"
    );
    assert!(ui.get_sound_library_open(), "音源管理必须在前面");

    // ④ 装好那一跳（与下载排空**同一个**函数）⇒ 自动换上。
    host::rescue_after_pack_installed(ui, pack_id.as_str());

    let after_project = port.project();
    let sampler = after_project
        .tracks
        .get(&track_id)
        .and_then(|track| track.sampler.clone())
        .expect("装好之后必须已经绑上");
    assert_eq!(sampler.path, pack_path, "绑的必须正是刚装好的那一件");
    assert!(!ui.get_missing_sounds_open(), "都解决了 ⇒ 面板不回来");
    assert_eq!(ui.get_engine_error().to_string(), "", "红字同样要收掉");
}

/// 清单一遍扫：第一件**本机解析得到 .sfz** 的内置音源。
fn first_installed_pack() -> Option<(String, String, String)> {
    let sounds = yeban_app::builtin_sounds::load_builtin_sounds().ok()?;
    for sound in sounds {
        if let Some(path) =
            yeban_app::missing_sounds::installed_path(&sound.id, yeban_app::sfz_load::SfzRoots::none())
        {
            return Some((sound.id, sound.name, path));
        }
    }
    None
}
