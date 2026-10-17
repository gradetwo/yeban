//! ⭐ 任务单 `#164` 的判据：**控制台一键给展示轨装一份 SFZ 乐器**（UI "能听"闭环的最后一格）。
//!
//! ## 这条判据要证的句子（逐条对应工单 §2）
//!
//! | §2 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 装乐器 = Δ1 | `invoke_instrument_install(0)` ⇒ `commit_count +1`、`undoable +1`；读回 `TrackV3::sampler` 非 `None` 且 `path` **逐字**等于内置清单里的仓库路径；该轨其余字段逐字段不变 | 证"点击 → 一个 `Op::SetTrackSampler` → 一步撤销" |
//! | ② 能出声 | 同一份工程装一份**带真实 WAV 样本**的夹具 SFZ ＋ 1 个音符 ⇒ 引擎渲染路径（`render_project_f32_with_assets`，走 `arm_pending_sfz`）非静音（非零样本 ＋ 峰值）；对照臂：去掉音符 ⇒ 逐位零 | 证"模型链条（sampler ⇒ 摊平表 ⇒ 引擎 SFZ 分支）真的把样本送进了输出"；⛔ 不证仓库自带 SFZ 的样本字节（见下） |
//! | ③ 投影可见 | 投影 / 界面 / 控件树三处都读到 `乐器：<路径>`，没装时是 `未装乐器` | 证"装完看得见" |
//! | ④ 换乐器 = Δ1 | 再装第 2 份 ⇒ `commit_count +1`、读回路径变了、`乐器：…` 跟着变；重复装同一份 ⇒ Δ0 | 证"换一份是**一次**新提交，同一份不制造空撤销步" |
//! | ⑤ 无展示轨 | 只有主总线的工程 ⇒ 两档可用性 `false`、原因**逐字**等于 `INSTRUMENT_INSTALL_NO_TRACK_REASON`；直接调两个回调也 Δ0；纯函数 `Err(同一个原因)` | 证"不静默禁用、也不靠界面拦" |
//! | ⑥ 预算三件套 | 不在本文件：读数在 `tests/perf_draw_budget.rs`（写点 **74** / 数组 **44**，常量与原因同处） | — |
//! | ⑦ 不退化 | `cargo test -p yeban-app --locked --no-fail-fast` 的 rc 与 passed 数 | — |
//!
//! ## ⚠ 仓库现状（本判据不假装）
//!
//! `assets/samples/**` 里**有 1398 份 `.sfz` 文本、0 个样本字节**
//! （`.wav` / `.flac` / `.ogg` 合计 0；见 `docs/ledger/phase-status.md` 的同一行读数）。
//! ⇒ 装一份**仓库自带**的 SFZ 时，`decode_sfz_plan` 把每条区域标成 `NO_SAMPLE`
//! ⇒ 引擎的 SFZ 分支**不建声部** ⇒ 输出是**静音**（本判据把这个读数打印出来，但**不**
//! 断言它，否则将来样本入库时这条判据会反向变红）。"能出声"那一条因此用**运行时夹具**
//! （临时目录里的 1 个 region ＋ 1 份 24-bit WAV）通过**同一个** `instrument_install_ops`
//! 装进模型 ⇒ 证明的是"UI 写进去的 `sampler` 真的能驱动引擎 SFZ 分支"，不是"仓库样本存在"。
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! - 摘掉 `host::wire_device_rack` 里 `on_instrument_install` 的 `commit_ops` ⇒ ①④ 的
//!   `commit_count` 差 0；
//! - 把 `instrument_install_ops` 的 `new_sampler` 改成 `None` ⇒ ① 的"读回非 `None`"红；
//! - 把 `instrument_label` 恒改成 `"未装乐器"` ⇒ ③ 红；
//! - 把 `instrument_install_ops` 的 `old_sampler` 改成 `None` ⇒ ④ 的第二次提交被模型
//!   （陈旧载荷）拒绝 ⇒ `commit_count` 差 0；
//! - 把音频夹具的 `sample=` 指向一个不存在的文件 ⇒ ② 的 `decoded_samples == 1` 红。
//!
//! ## 它**不**证什么
//!
//! ⛔ 不证"仓库自带 SFZ 今天能出声"（样本字节不在仓库，见上）；
//! ⛔ 不证人耳听到的声音（本机无音频设备会话，判据只读样本数组）；
//! ⛔ 不证"不装乐器 ⇒ 静音"—— ⭐ 恰恰相反：引擎给每个轨槽**恒备一台复音合成器**
//! （`yeban-engine/src/synth.rs:1923-1928` 的 `armed_sources = 1 + …`），所以没装采样器
//! 的轨**也会发声**。本判据把这条实测读数打印出来（`baseline_peak`），工单 §2 的反向腿
//! 与引擎事实不符，已在交付报告里如实登记。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use yeban_app::bridge::{
    BUILTIN_INSTRUMENTS, INSTRUMENT_INSTALL_NO_TRACK_REASON, ViewState, device_rack_track,
    instrument_install_ops,
};
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
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::tree::ControlTree;

/// 窗口尺寸（与 `device_clip_creation.rs` 同款：同一块 Tab 3 面板需要高度余量）。
const WINDOW: Size = Size::new(1440, 1000);

/// 一次提交用的时间戳（与其余界面判据同一个值，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 工程采样率（= 渲染采样率；与 `YebanProjectV1::default` 同口径）。
const RATE: u32 = 48_000;

/// 夹具音符的时值：480 tick = 半拍 @ 120 BPM = 0.25 s。
const NOTE_TICKS: u64 = 480;

/// 一份**带真实样本字节**的 SFZ（临时目录；见模块文档"仓库现状"）。
struct AudioFixture {
    dir: PathBuf,
    sfz: PathBuf,
}

impl AudioFixture {
    /// 写 1 份 24-bit 单声道 WAV（0.25 s、440 Hz、幅度 0.5）＋ 1 份单 region 的 SFZ。
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-instrument-{tag}-{}-{}",
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
        write_plain_wav(
            &wav,
            &PcmFormat::integer(1, RATE, 24),
            &PcmBuffer::Int24(pcm),
        )
        .expect("写夹具 WAV");
        let sfz = dir.join("tone.sfz");
        std::fs::write(&sfz, "<region> sample=tone.wav key=60 pitch_keycenter=60\n")
            .expect("写夹具 SFZ");
        Self { dir, sfz }
    }
}

impl Drop for AudioFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 一个最小可播放工程：主总线 ＋ 一条 MIDI 轨 ＋ 一个摆好的 MIDI 片段 ＋ 1 个音符。
fn project_with_one_note(tag: &str) -> (YebanProjectV1, EntityId) {
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
            name: format!("Lead {tag}"),
            kind: TrackKind::Midi,
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
        },
    );
    project.validate().expect("夹具工程必须合法");
    (project, track_id)
}

/// 只有主总线的合法工程（判据 ⑤ 的夹具）。
fn project_with_master_only() -> YebanProjectV1 {
    let master = EntityId::new();
    let mut project = YebanProjectV1 {
        master_bus_track_id: master,
        ..YebanProjectV1::default()
    };
    project.tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    project.routing_graph.nodes = vec![master];
    project.validate().expect("只剩主总线的工程必须合法");
    project
}

/// 一条生产形态的 Tab 3 活窗口（接线走**生产同一个** `host::wire_device_rack`）。
fn live_window_with_undo(
    project: &YebanProjectV1,
    tag: &str,
) -> (LivePort<MainWindow>, Rc<UndoPort>) {
    let view = ViewState::from_project(project).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            format!("<判据 {tag}>"),
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("打开撤销会话"),
    ));
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(&view, &scene, 2)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    host::wire_device_rack(live.ui(), &port);
    (live, port)
}

/// 运行时控件树（`[UI-DoD §3.6]` 的唯一载体）。
fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

/// 渲染这条工程 0.5 s（2 声道）并交回控制侧读数。
fn render(project: &YebanProjectV1) -> yeban_app::render_core::RenderProjectOutput {
    let options = WavExportOptions {
        frames: u64::from(RATE) / 2,
        sample_rate: RATE,
        channels: 2,
        seed: 1,
        use_engine_source: true,
        bit_depth: BitDepth::Int24,
    };
    render_project_f32_with_assets(project, options, None, &[]).expect("引擎渲染路径")
}

/// `(非零样本个数, 峰值)` —— 交错样本上的两个读数。
fn stats(samples: &[f32]) -> (usize, f32) {
    let nonzero = samples.iter().filter(|sample| **sample != 0.0).count();
    let peak = samples
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    (nonzero, peak)
}

// ===========================================================================
// 判据 ①：装乐器 = Δ1 ＋ 逐字读回
// ===========================================================================
#[test]
fn installing_an_instrument_commits_once_and_reads_back_verbatim() {
    let (project, track_id) = project_with_one_note("install");
    let before_track = project.tracks.get(&track_id).expect("轨在").clone();
    assert_eq!(before_track.sampler, None, "夹具起手必须没有乐器");

    let view = ViewState::from_project(&project).expect("投影");
    assert_eq!(view.instrument_label, "未装乐器");
    assert!(view.instrument_install_available);
    assert_eq!(view.instrument_install_reason, "");
    assert_eq!(view.instrument_names.len(), BUILTIN_INSTRUMENTS.len());

    let (live, port) = live_window_with_undo(&project, "#164 装乐器");
    let before = port.display();
    assert_eq!(live.ui().get_instrument_label().to_string(), "未装乐器");
    live.ui().invoke_instrument_install(0);

    let expected = BUILTIN_INSTRUMENTS[0];
    let after = port.display();
    assert_eq!(
        after.commit_count,
        before.commit_count + 1,
        "一次点击必须**恰好**一次 `commit_ops`（前 {}，后 {}）",
        before.commit_count,
        after.commit_count
    );
    assert_eq!(after.undoable, before.undoable + 1, "必须恰好一步可撤销");

    let written = port.project();
    let after_track = written.tracks.get(&track_id).expect("轨在");
    assert_eq!(
        after_track.sampler,
        Some(SamplerBinding {
            path: expected.path.to_owned()
        }),
        "读回的 `TrackV3::sampler` 必须**逐字**是内置清单里的仓库路径"
    );
    let mut expected_track = before_track;
    expected_track.sampler = after_track.sampler.clone();
    assert_eq!(
        after_track, &expected_track,
        "`SetTrackSampler` 只许改 `sampler`：其余字段必须逐字段不变"
    );
    written.validate().expect("装完乐器的工程必须合法");

    let label = format!("乐器：{}", expected.path);
    assert_eq!(live.ui().get_instrument_label().to_string(), label);
    let tree = runtime_tree(live.ui());
    let node = tree
        .find_by_id("instrument-label")
        .expect("控件树里必须有 `instrument-label`");
    assert_eq!(node.label, label, "控件树的读数必须逐字是那条路径");
    assert_eq!(
        tree.find_by_id("instrument-install-0-button")
            .expect("第一个装乐器按钮必须在树里")
            .label,
        format!("装乐器 {}", expected.name)
    );

    report_line(&format!(
        "[#164 ①] 装乐器：commit {}→{}、undoable {}→{}；sampler.path={:?}；label={:?}",
        before.commit_count,
        after.commit_count,
        before.undoable,
        after.undoable,
        after_track
            .sampler
            .as_ref()
            .map(|binding| binding.path.as_str()),
        label
    ));
}

// ===========================================================================
// 判据 ③④：投影可见 ＋ 换乐器 = Δ1（重复装同一份 = Δ0）
// ===========================================================================
#[test]
fn changing_the_instrument_is_one_more_step_and_the_label_follows() {
    let (project, track_id) = project_with_one_note("change");
    let (live, port) = live_window_with_undo(&project, "#164 换乐器");

    live.ui().invoke_instrument_install(0);
    let first = port.display();
    let first_path = port
        .project()
        .tracks
        .get(&track_id)
        .and_then(|track| track.sampler.clone())
        .expect("第一份必须装上");
    assert_eq!(first_path.path, BUILTIN_INSTRUMENTS[0].path);
    let first_label = live.ui().get_instrument_label().to_string();
    assert_eq!(
        first_label,
        format!("乐器：{}", BUILTIN_INSTRUMENTS[0].path)
    );

    live.ui().invoke_instrument_install(1);
    let second = port.display();
    assert_eq!(
        second.commit_count,
        first.commit_count + 1,
        "换一份别的乐器必须恰好一次新提交"
    );
    let second_path = port
        .project()
        .tracks
        .get(&track_id)
        .and_then(|track| track.sampler.clone())
        .expect("第二份必须装上");
    assert_eq!(second_path.path, BUILTIN_INSTRUMENTS[1].path);
    assert_ne!(first_path, second_path, "换乐器读回必须变");
    let second_label = live.ui().get_instrument_label().to_string();
    assert_eq!(
        second_label,
        format!("乐器：{}", BUILTIN_INSTRUMENTS[1].path)
    );
    assert_ne!(first_label, second_label);

    // 重复装**同一份** ⇒ 值没变 ⇒ 0 提交、0 撤销步。
    live.ui().invoke_instrument_install(1);
    let third = port.display();
    assert_eq!(
        third.commit_count, second.commit_count,
        "重复装同一份不得制造一步空撤销"
    );
    assert_eq!(third.undoable, second.undoable);
    assert!(
        instrument_install_ops(&port.project(), BUILTIN_INSTRUMENTS[1].path)
            .expect("纯函数")
            .is_empty()
    );

    report_line(&format!(
        "[#164 ③④] 两档读数：{:?} → {:?}；提交 {}→{}→{}；重复 = Δ0（undoable 仍 {}）",
        first_label,
        second_label,
        first.commit_count,
        second.commit_count,
        third.commit_count,
        third.undoable
    ));
}

// ===========================================================================
// 判据 ⑤：无展示轨 ⇒ 禁用 ＋ 逐字原因 ＋ 直接调用 Δ0
// ===========================================================================
#[test]
fn without_a_display_track_the_gesture_is_disabled_and_a_direct_call_changes_nothing() {
    let project = project_with_master_only();
    let view = ViewState::from_project(&project).expect("投影");
    assert!(device_rack_track(&project).is_none());
    assert!(!view.instrument_install_available);
    assert_eq!(
        view.instrument_install_reason,
        INSTRUMENT_INSTALL_NO_TRACK_REASON
    );
    assert_eq!(view.instrument_label, "未装乐器");
    assert_eq!(
        view.instrument_names,
        BUILTIN_INSTRUMENTS
            .iter()
            .map(|instrument| instrument.name.to_owned())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        instrument_install_ops(&project, BUILTIN_INSTRUMENTS[0].path),
        Err(INSTRUMENT_INSTALL_NO_TRACK_REASON)
    );

    let registry = yeban_app::elements::ElementRegistry::from_view(&view);
    for id in [
        "instrument-label",
        "instrument-install-reason",
        "instrument-install-0-button",
        "instrument-install-1-button",
    ] {
        assert!(registry.contains(id), "注册表必须登记 `{id}`");
    }

    let (live, port) = live_window_with_undo(&project, "#164 空态");
    assert!(!live.ui().get_instrument_install_available());
    assert_eq!(
        live.ui().get_instrument_install_reason().to_string(),
        INSTRUMENT_INSTALL_NO_TRACK_REASON
    );
    let before = port.display();
    live.ui().invoke_instrument_install(0);
    live.ui().invoke_instrument_install(1);
    assert_eq!(
        port.display().commit_count,
        before.commit_count,
        "没有展示轨时两个手势都必须 Δ0（界面禁用之外，宿主也拒绝）"
    );
    assert_eq!(port.project().tracks.len(), 1);
    assert!(
        port.project()
            .tracks
            .values()
            .all(|track| track.sampler.is_none())
    );

    report_line(&format!(
        "[#164 ⑤] 空态：available=false · reason={:?} · 直接调用两次 commit {}→{}",
        INSTRUMENT_INSTALL_NO_TRACK_REASON,
        before.commit_count,
        port.display().commit_count
    ));
}

// ===========================================================================
// 判据 ②：装乐器 ＋ 1 个音符 ⇒ 引擎渲染非静音（夹具 SFZ ＋ 真 WAV）
// ===========================================================================
#[test]
fn an_installed_instrument_drives_the_engines_sfz_branch_to_a_non_silent_output() {
    let (mut project, track_id) = project_with_one_note("audio");
    let fixture = AudioFixture::new("audio");
    let fixture_path = fixture
        .sfz
        .to_str()
        .expect("临时路径必须是 UTF-8")
        .to_owned();

    // ---- 对照臂 A：没装乐器 ⇒ 引擎**仍然**有声（恒备的复音合成器），而不是工单说的静音。
    let baseline = render(&project);
    assert!(baseline.sfz_decoded.is_empty(), "没装采样器就不得有摊平表");
    let (baseline_nonzero, baseline_peak) = stats(&baseline.samples);
    assert!(
        baseline_nonzero > 0 && baseline_peak > 0.0,
        "引擎给每个轨槽恒备一台复音合成器 ⇒ 没装乐器也会发声（实测 non-zero={baseline_nonzero}, \
         peak={baseline_peak}）；工单 §2 的\"不装乐器 ⇒ 静音\"与引擎事实不符"
    );

    // ---- 正路：把夹具 SFZ（真 WAV）装进模型，再走**引擎渲染入口**。
    let ops = instrument_install_ops(&project, &fixture_path).expect("装夹具乐器");
    assert_eq!(ops.len(), 1);
    for op in &ops {
        op.apply(&mut project).expect("SetTrackSampler 必须被接受");
    }
    let rendered = render(&project);
    assert_eq!(
        rendered.sfz_decoded.len(),
        1,
        "模型链条必须为这条轨摊平一份 SFZ"
    );
    let (_, report) = &rendered.sfz_decoded[0];
    assert_eq!(report.regions, 1, "夹具 SFZ 必须恰好 1 个 region");
    assert_eq!(report.decoded_samples, 1, "夹具的 WAV 必须真的解码 1 份");
    assert_eq!(report.silent_region_count, 0, "不该有缺样本的静音区域");
    assert!(report.missing_sample_paths.is_empty());
    let (nonzero, peak) = stats(&rendered.samples);
    assert!(
        nonzero > 0,
        "装完乐器必须非静音（实测 {nonzero} 个非零样本）"
    );
    assert!(peak > 0.05, "峰值必须可闻（实测 {peak}）");
    assert_ne!(
        rendered.digest, baseline.digest,
        "装乐器之后输出必须真的不同（不是同一条复音尾巴）"
    );
    assert_eq!(
        project
            .tracks
            .get(&track_id)
            .and_then(|track| track.sampler.clone()),
        Some(SamplerBinding {
            path: fixture_path.clone()
        })
    );

    // ---- 对照臂 B：同一份装了乐器的工程去掉音符 ⇒ 逐位零（证明"非静音"不是夹具噪声）。
    let silent = {
        let mut project = project.clone();
        for entry in project.clip_pool.values_mut() {
            if let ClipContent::Midi { notes } = &mut entry.content {
                notes.clear();
            }
        }
        render(&project)
    };
    let (silent_nonzero, silent_peak) = stats(&silent.samples);
    assert_eq!(silent_peak, 0.0, "没有音符时输出必须逐位零");
    assert_eq!(silent_nonzero, 0);

    // ---- 诊断（⛔ 不断言）：装**仓库自带**的 SFZ 时，样本字节不在仓库 ⇒ 摊平表全静音。
    // ⚠ `cargo test` 的工作目录是**crate 目录**（不是仓库根）⇒ 这里用 `CARGO_MANIFEST_DIR`
    // 拼绝对路径读**同一份**文件。界面上写进模型的是仓库相对路径（工单要求"路径逐字"）；
    // 从别的 cwd 启动时它读不到 —— 这是本切片如实登记的边界（交付报告 §8）。
    let repo_sfz = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(BUILTIN_INSTRUMENTS[0].path);
    let mut repo_project = project.clone();
    let repo_ops = instrument_install_ops(
        &repo_project,
        repo_sfz.to_str().expect("仓库路径必须是 UTF-8"),
    )
    .expect("换仓库乐器");
    for op in &repo_ops {
        op.apply(&mut repo_project).expect("SetTrackSampler");
    }
    let repo_rendered = render(&repo_project);
    let (_, repo_report) = &repo_rendered.sfz_decoded[0];
    let (repo_nonzero, repo_peak) = stats(&repo_rendered.samples);

    report_line(&format!(
        "[#164 ②] 对照A（无乐器）non-zero={baseline_nonzero} peak={baseline_peak:.6} \
         digest={}；正路（夹具 SFZ）non-zero={nonzero} peak={peak:.6} digest={} \
         decoded={} silent={}；对照B（无音符）non-zero={silent_nonzero} peak={silent_peak:.6}；\
         仓库 SFZ（{}）regions={} decoded={} silent={} → non-zero={repo_nonzero} peak={repo_peak:.6}",
        baseline.digest,
        rendered.digest,
        report.decoded_samples,
        report.silent_region_count,
        BUILTIN_INSTRUMENTS[0].path,
        repo_report.regions,
        repo_report.decoded_samples,
        repo_report.silent_region_count,
    ));
}
