//! ⭐ 任务单 `#164`／`#165` 的判据：**一键给展示轨装一份 SFZ 乐器**，以及
//! **绝不许把一条有声的轨装成静音**（`#165` 的集成者裁决）。
//!
//! ## 这条判据要证的句子（逐条对应工单 §2）
//!
//! | 工单 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | `#165` ① 清单为空且原因可见 | `installable_builtin_instruments()` 为空；投影 `instrument_names` 为空；控件树里 `instrument-install-0-button` **不存在**；`instrument-install-reason`（纯函数层）逐字等于 `INSTRUMENT_INSTALL_NO_BUILTIN_REASON`；⭐ `#196` 起控件树那一格显示 `sample-library-status == "采样库未安装，点此下载（9.4 GiB）"` ＋ 下载按钮在场 | 证"按钮数 0 ＋ 不静默禁用 ＋ 有可操作的下一步" |
//! | `#165` ② 装不掉 | 直接调 `invoke_instrument_install(0)`（宿主回调）⇒ `commit_count` Δ0、`sampler` 仍 `None`；纯函数 `instrument_install_ops` 对两份候选都 `Err(INSTRUMENT_INSTALL_NO_SAMPLES_REASON)` | 证"界面之外的入口也拒绝" |
//! | `#165` ③ 有声未被破坏 | 没装乐器时引擎渲染 `peak ≈ 0.774…`（恒备复音）；装不掉之后再渲一次 ⇒ `digest`／`non-zero`／`peak` **逐位相同** | 证"不再有'装了却静音'的路径" |
//! | `#164` 正路 = Δ1 | 一份**带真实 WAV 样本**的夹具 SFZ ⇒ `instrument_install_ops` 恰好 1 条 `Op::SetTrackSampler`，`commit_ops` 之后 `commit_count +1`、`undoable +1`；读回 `path` 逐字；该轨其余字段不变 | 证"样本存在时那条装乐器的路仍然是**一次**提交" |
//! | `#164` 能出声 | 夹具 SFZ ＋ 1 个音符 ⇒ 引擎渲染路径（`render_project_f32_with_assets`，走 `arm_pending_sfz`）非静音；对照臂：去掉音符 ⇒ 逐位零 | 证"模型链条（sampler ⇒ 摊平表 ⇒ 引擎 SFZ 分支）真的发声" |
//! | `#164` 投影可见 | 装完之后投影 / 界面都读到 `乐器：<路径>`；没装时是 `未装乐器` | 证"装完看得见" |
//! | `#164` 换乐器 = Δ1 | 换成第 2 份夹具 ⇒ `commit_count +1`、`乐器：…` 跟着变；重复装同一份 ⇒ `ops` 为空 ⇒ Δ0 | 证"换一份是一次新提交，同一份不制造空撤销步" |
//! | `#164` 无展示轨 | 只有主总线的工程 ⇒ 可用性 `false`、原因逐字等于 `INSTRUMENT_INSTALL_NO_TRACK_REASON`；直接调回调 Δ0；纯函数 `Err(同一个原因)` | 证"不静默禁用、也不靠界面拦" |
//! | `#165` 谓词唯一 | 谓词 `sfz_samples_present` 对"1 region ＋ 1 WAV"返回 `Ok`、对"样本缺失"返回 `NO_SAMPLES`、对"SFZ 读不到"返回 `UNREADABLE`；可装清单 = 候选 × 同一谓词 | 证"过滤没有第二条真相（不另写'找 wav'的探测）" |
//!
//! ## ⚠ 仓库现状（本判据不假装）
//!
//! `assets/samples/**` 里**有 1398 份 `.sfz` 文本、0 个样本字节**
//! （`.wav` / `.flac` / `.ogg` 合计 0；`find assets/samples -name '*.sfz' | wc -l` = 1398，
//! 三个音频后缀合计 0）。⇒ `decode_sfz_plan` 把每条区域标成 `NO_SAMPLE` ⇒ 装一份仓库
//! 自带的 SFZ 会让这条轨**静音**（`#164` 实测：`regions=96 / decoded=0 / silent=96`）。
//! ⭐ `#165` 的裁决因此是：**内置清单按样本字节过滤**（今天为空 ⇒ 0 个按钮）＋
//! **装乐器入口拒绝缺样本的 SFZ**（逐字原因）。
//!
//! ⚠ 不装乐器**不是静音**：引擎给每个轨槽恒备一台复音合成器
//! （`yeban-engine/src/synth.rs` 的 `armed_sources = 1 + …`）⇒ 未装采样器的轨**也会发声**
//! （本判据实测 `baseline_peak ≈ 0.774…`）。所以"装一份全缺样本的 SFZ"是把轨从
//! **有声**变成**静音**的**倒退** —— 这正是 `#165` 要堵的路。
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! - **注入甲**：把 `installable_builtin_instruments` 的过滤恒真（保留 2 份候选）⇒
//!   ①的"0 个按钮"与③的"清单为空"红；
//! - **注入乙**：摘掉 `instrument_install_ops` 里的 `sfz_samples_present(path)?` ⇒
//!   ②的纯函数拒绝红（并且真装上了 ⇒ `sampler` 非 `None`、`digest` 变、峰值归零）；
//! - 摘掉 `host::wire_device_rack` 里 `on_instrument_install` 的 `commit_ops` ⇒ 正路的
//!   `commit_count` 差 0；
//! - 把 `instrument_install_ops` 的 `new_sampler` 改成 `None` ⇒ 正路"读回非 `None`"红；
//! - 把 `instrument_label` 恒改成 `"未装乐器"` ⇒ 投影可见红；
//! - 把 `old_sampler` 改成 `None` ⇒ 换乐器的第二次提交被模型（陈旧载荷）拒绝；
//! - 把音频夹具的 `sample=` 指向一个不存在的文件 ⇒ ② 的 `decoded_samples == 1` 红。
//!
//! ## 它**不**证什么
//!
//! ⛔ 不证"仓库自带 SFZ 今天能出声"（样本字节不在仓库，见上）；
//! ⛔ 不证"界面点一下真能装上"—— 今天可装清单为空、**没有按钮**；装乐器的正路只在
//! "样本字节存在"时可达，本判据用夹具从**同一个** `instrument_install_ops` 验证它；
//! ⛔ 不证人耳听到的声音（本机无音频设备会话，判据只读样本数组）。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use yeban_app::bridge::{
    BUILTIN_INSTRUMENTS, INSTRUMENT_INSTALL_NO_BUILTIN_REASON,
    INSTRUMENT_INSTALL_NO_SAMPLES_REASON, INSTRUMENT_INSTALL_NO_TRACK_REASON,
    INSTRUMENT_INSTALL_UNREADABLE_SFZ_REASON, ViewState, builtin_instrument, device_rack_track,
    installable_builtin_instruments, instrument_install_ops, instrument_names, sfz_samples_present,
};
use yeban_app::export_wav::WavExportOptions;
use yeban_app::host;
use yeban_app::render_core::render_project_f32_with_assets;
use yeban_app::samples::{SAMPLE_LIBRARY_ENV, SAMPLE_LIBRARY_NOT_INSTALLED_HINT};
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

/// ⭐ `#205`：把采样库根固定到**不存在**的路径（= CI 的形态）。
///
/// 本文件的判据写的是"**仓库内** 0 个样本字节 ⇒ 内置清单一律不可装"（`#165`）。
/// `#196` 起装乐器谓词**也认采样库根** ⇒ 一台已经下载过库的开发机会把两份内置候选
/// 判成可装（那是产品想要的，但**不是**本文件要测的那一档）。所以判据开始前先把
/// 库根隔离掉，让它无论在哪种机器上都测同一件事。
///
/// ⚠ 用 [`std::sync::Once`]：`set_var` 在 Rust 2024 是 `unsafe`（要求没有并发读者）。
/// 本文件**每个**判据的第一行都调本函数 ⇒ 写发生在任何一次读之前（`Once` 的
/// happens-before），进程内只有这一次写，之后不再改。
fn isolate_repo_only_library() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let path = std::env::temp_dir().join("yeban-instrument-install-no-library");
        // SAFETY: 见函数文档 —— 唯一一次写，且所有读都发生在本函数返回之后。
        unsafe { std::env::set_var(SAMPLE_LIBRARY_ENV, path) };
    });
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
        limiter: false,
        threads: None,
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

/// 把一份夹具乐器按**宿主同一条** `Op` 路径提交（`instrument_install_ops` + `commit_ops`）。
fn install_fixture(port: &UndoPort, fixture: &AudioFixture, message: &str) -> String {
    let path = fixture
        .sfz
        .to_str()
        .expect("临时路径必须是 UTF-8")
        .to_owned();
    let ops = instrument_install_ops(&port.project(), &path).expect("带样本的夹具必须可装");
    assert_eq!(ops.len(), 1, "装一份新乐器恰好一条 `SetTrackSampler`");
    port.commit_ops(NOW_MS + 1, message, ops)
        .expect("提交必须被模型接受");
    path
}

// ===========================================================================
// `#165` ①：清单为空 ＋ 原因可见 ＋ 按钮数 0
// ===========================================================================
#[test]
fn the_builtin_list_is_empty_and_the_reason_is_visible_with_zero_buttons() {
    isolate_repo_only_library();
    let (project, track_id) = project_with_one_note("empty");
    let view = ViewState::from_project(&project).expect("投影");

    // ---- 纯函数层：候选 2 份，可装 0 份（仓内 0 个样本字节）。
    assert_eq!(BUILTIN_INSTRUMENTS.len(), 2, "候选清单仍是 2 份");
    assert!(
        installable_builtin_instruments().is_empty(),
        "仓内 1398 份 SFZ / 0 个样本字节 ⇒ 可装清单必须为空（实测 {} 份）",
        installable_builtin_instruments().len()
    );
    assert_eq!(builtin_instrument(0), None, "清单为空 ⇒ 下标 0 也取不到");
    assert_eq!(instrument_names(), Vec::<String>::new());

    // ---- 投影层：0 个名字 ＋ 不可用 ＋ 逐字原因。
    assert_eq!(view.instrument_label, "未装乐器");
    assert!(view.instrument_names.is_empty());
    assert!(!view.instrument_install_available);
    assert_eq!(
        view.instrument_install_reason,
        INSTRUMENT_INSTALL_NO_BUILTIN_REASON
    );

    // ---- 控件树：0 个装乐器按钮；⭐ `#196` 起那一格显示"采样库未安装"＋下载按钮。
    let (live, port) = live_window_with_undo(&project, "#165 空清单");
    assert!(!live.ui().get_instrument_install_available());
    assert_eq!(
        live.ui().get_instrument_install_reason().to_string(),
        INSTRUMENT_INSTALL_NO_BUILTIN_REASON
    );
    let tree = runtime_tree(live.ui());
    for index in 0..BUILTIN_INSTRUMENTS.len() {
        assert!(
            tree.find_by_id(&format!("instrument-install-{index}-button"))
                .is_none(),
            "可装清单为空 ⇒ 第 {index} 个装乐器按钮不得存在"
        );
    }
    // ⭐ `#196`：清单为空时界面不再"只说不做"——同一行给出可下载的入口与逐字提示。
    assert_eq!(
        tree.find_by_id("sample-library-status")
            .expect("未安装时必须在树里逐字说'采样库未安装'")
            .label,
        SAMPLE_LIBRARY_NOT_INSTALLED_HINT
    );
    let download = tree
        .find_by_id("sample-library-download-button")
        .expect("未安装时下载按钮必须在场");
    assert_eq!(download.label, "下载采样库");

    // ---- 直接调回调也 Δ0（界面之外还有宿主这道门）。
    let before = port.display();
    live.ui().invoke_instrument_install(0);
    live.ui().invoke_instrument_install(1);
    let after = port.display();
    assert_eq!(after.commit_count, before.commit_count, "拒绝 ⇒ 0 次提交");
    assert_eq!(after.undoable, before.undoable, "拒绝 ⇒ 0 步撤销");
    assert_eq!(
        port.project().tracks.get(&track_id).expect("轨在").sampler,
        None,
        "拒绝之后 `sampler` 必须仍是 `None`（这条轨仍然有声）"
    );

    report_line(&format!(
        "[#165 ①] 候选={} 可装={} · names={:?} · available=false · reason={:?} · buttons=0 · \
         直接调回调 commit {}→{} · sampler=None",
        BUILTIN_INSTRUMENTS.len(),
        installable_builtin_instruments().len(),
        view.instrument_names,
        view.instrument_install_reason,
        before.commit_count,
        after.commit_count,
    ));
}

// ===========================================================================
// `#164` 正路：装一份**带样本**的夹具 = Δ1 ＋ 逐字读回 ＋ 投影可见
// ===========================================================================
#[test]
fn a_sample_bearing_sfz_installs_as_one_step_and_the_label_follows() {
    isolate_repo_only_library();
    let (project, track_id) = project_with_one_note("install");
    let before_track = project.tracks.get(&track_id).expect("轨在").clone();
    assert_eq!(before_track.sampler, None, "夹具起手必须没有乐器");

    let fixture = AudioFixture::new("install");
    let (live, port) = live_window_with_undo(&project, "#164 装乐器");

    let before = port.display();
    let path = install_fixture(&port, &fixture, "install instrument");
    let after = port.display();
    assert_eq!(
        after.commit_count,
        before.commit_count + 1,
        "装一份乐器必须**恰好**一次 `commit_ops`（前 {}，后 {}）",
        before.commit_count,
        after.commit_count
    );
    assert_eq!(after.undoable, before.undoable + 1, "必须恰好一步可撤销");

    let written = port.project();
    let after_track = written.tracks.get(&track_id).expect("轨在");
    assert_eq!(
        after_track.sampler,
        Some(SamplerBinding { path: path.clone() }),
        "读回的 `TrackV3::sampler` 必须**逐字**是那份 SFZ 路径"
    );
    let mut expected_track = before_track;
    expected_track.sampler = after_track.sampler.clone();
    assert_eq!(
        after_track, &expected_track,
        "`SetTrackSampler` 只许改 `sampler`：其余字段必须逐字段不变"
    );
    written.validate().expect("装完乐器的工程必须合法");

    // ---- 投影 / 界面：装完看得见（`host::apply_view` 是生产同一个注入点）。
    let view = ViewState::from_project(&written).expect("重投影");
    let label = format!("乐器：{path}");
    assert_eq!(view.instrument_label, label);
    host::apply_view(live.ui(), &view, WINDOW.width as f32, 0.0);
    assert_eq!(live.ui().get_instrument_label().to_string(), label);
    let tree = runtime_tree(live.ui());
    let node = tree
        .find_by_id("instrument-label")
        .expect("控件树里必须有 `instrument-label`");
    assert_eq!(node.label, label, "控件树的读数必须逐字是那条路径");

    report_line(&format!(
        "[#164 正路] 装乐器：commit {}→{}、undoable {}→{}；sampler.path={:?}；label={:?}",
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
// `#164` 换乐器 = Δ1（重复装同一份 = Δ0）
// ===========================================================================
#[test]
fn changing_the_instrument_is_one_more_step_and_repeating_it_is_free() {
    isolate_repo_only_library();
    let (project, track_id) = project_with_one_note("change");
    let (live, port) = live_window_with_undo(&project, "#164 换乐器");

    let first_fixture = AudioFixture::new("change-a");
    let first = port.display();
    let first_path = install_fixture(&port, &first_fixture, "install instrument");
    let second = port.display();
    assert_eq!(second.commit_count, first.commit_count + 1);

    let second_fixture = AudioFixture::new("change-b");
    let second_path = install_fixture(&port, &second_fixture, "install instrument");
    let third = port.display();
    assert_eq!(
        third.commit_count,
        second.commit_count + 1,
        "换一份别的乐器必须恰好一次新提交"
    );
    assert_ne!(first_path, second_path, "换乐器读回必须变");

    // ---- 投影跟着变。
    let view = ViewState::from_project(&port.project()).expect("重投影");
    let label = format!("乐器：{second_path}");
    assert_eq!(view.instrument_label, label);
    host::apply_view(live.ui(), &view, WINDOW.width as f32, 0.0);
    assert_eq!(live.ui().get_instrument_label().to_string(), label);

    // ---- 重复装**同一份** ⇒ 值没变 ⇒ `ops` 为空 ⇒ 0 提交、0 撤销步。
    assert!(
        instrument_install_ops(&port.project(), &second_path)
            .expect("纯函数")
            .is_empty()
    );
    let fourth = port.display();
    assert_eq!(fourth.commit_count, third.commit_count);
    assert_eq!(fourth.undoable, third.undoable);
    assert_eq!(
        port.project()
            .tracks
            .get(&track_id)
            .and_then(|track| track.sampler.clone()),
        Some(SamplerBinding {
            path: second_path.clone()
        })
    );

    report_line(&format!(
        "[#164 换乐器] {:?} → {:?}；提交 {}→{}→{}；重复 = Δ0（undoable 仍 {}）",
        first_path,
        second_path,
        first.commit_count,
        second.commit_count,
        third.commit_count,
        third.undoable
    ));
}

// ===========================================================================
// `#164` ⑤：无展示轨 ⇒ 禁用 ＋ 逐字原因 ＋ 直接调用 Δ0
// ===========================================================================
#[test]
fn without_a_display_track_the_gesture_is_disabled_and_a_direct_call_changes_nothing() {
    isolate_repo_only_library();
    let project = project_with_master_only();
    let view = ViewState::from_project(&project).expect("投影");
    assert!(device_rack_track(&project).is_none());
    assert!(!view.instrument_install_available);
    assert_eq!(
        view.instrument_install_reason,
        INSTRUMENT_INSTALL_NO_TRACK_REASON
    );
    assert_eq!(view.instrument_label, "未装乐器");
    assert!(view.instrument_names.is_empty());
    assert_eq!(
        instrument_install_ops(&project, BUILTIN_INSTRUMENTS[0].path),
        Err(INSTRUMENT_INSTALL_NO_TRACK_REASON),
        "无展示轨的原因**优先于**样本缺失（顺序即优先级）"
    );

    let registry = yeban_app::elements::ElementRegistry::from_view(&view);
    for id in ["instrument-label", "instrument-install-reason"] {
        assert!(registry.contains(id), "注册表必须登记 `{id}`");
    }
    assert!(
        !registry.contains("instrument-install-0-button"),
        "可装清单为空 ⇒ 注册表不得登记装乐器按钮"
    );

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
// `#164` ②：装乐器 ＋ 1 个音符 ⇒ 引擎渲染非静音（夹具 SFZ ＋ 真 WAV）
// ===========================================================================
#[test]
fn an_installed_instrument_drives_the_engines_sfz_branch_to_a_non_silent_output() {
    isolate_repo_only_library();
    let (mut project, track_id) = project_with_one_note("audio");
    let fixture = AudioFixture::new("audio");
    let fixture_path = fixture
        .sfz
        .to_str()
        .expect("临时路径必须是 UTF-8")
        .to_owned();

    // ---- 对照臂 A：没装乐器 ⇒ 引擎**仍然**有声（恒备的复音合成器），而不是静音。
    let baseline = render(&project);
    assert!(baseline.sfz_decoded.is_empty(), "没装采样器就不得有摊平表");
    let (baseline_nonzero, baseline_peak) = stats(&baseline.samples);
    assert!(
        baseline_nonzero > 0 && baseline_peak > 0.0,
        "引擎给每个轨槽恒备一台复音合成器 ⇒ 没装乐器也会发声（实测 non-zero={baseline_nonzero}, \
         peak={baseline_peak}）"
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

    // ---- ⭐ `#165` 的拒绝（断言，不是诊断）：装**仓库自带**的 SFZ ⇒ 入口必须拒绝。
    let repo_sfz = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(BUILTIN_INSTRUMENTS[0].path);
    let (_repo_plan, repo_report) =
        yeban_app::sfz_load::decode_sfz_plan(&repo_sfz, 48_000.0).expect("读仓库 SFZ");
    assert_eq!(repo_report.regions, 96, "仓库 SFZ 的区域数（#164 实测 96）");
    assert_eq!(repo_report.decoded_samples, 0, "仓内 0 个样本字节");
    assert_eq!(repo_report.silent_region_count, 96);
    assert_eq!(
        instrument_install_ops(&project, BUILTIN_INSTRUMENTS[0].path),
        Err(INSTRUMENT_INSTALL_NO_SAMPLES_REASON),
        "缺样本的仓库 SFZ 必须被拒绝（⛔ 不许把这条轨装成静音）"
    );

    report_line(&format!(
        "[#164 ②] 对照A（无乐器）non-zero={baseline_nonzero} peak={baseline_peak:.6} \
         digest={}；正路（夹具 SFZ）non-zero={nonzero} peak={peak:.6} digest={} \
         decoded={} silent={}；对照B（无音符）non-zero={silent_nonzero} peak={silent_peak:.6}；\
         仓库 SFZ（{}）regions={} decoded={} silent={} ⇒ 装乐器入口=Err(NO_SAMPLES)",
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

// ===========================================================================
// `#165` ②③：缺样本 ⇒ 装不掉，且这条轨**仍然有声**（逐位相同）
// ===========================================================================
#[test]
fn a_sample_less_sfz_is_refused_so_the_track_never_goes_silent() {
    isolate_repo_only_library();
    let (project, track_id) = project_with_one_note("reject");

    // ---- 装之前：这条轨有声（恒备复音），读数留档。
    let baseline = render(&project);
    let (baseline_nonzero, baseline_peak) = stats(&baseline.samples);
    assert!(
        baseline_nonzero > 0 && baseline_peak > 0.5,
        "不装乐器时引擎恒备复音必须有声（实测 non-zero={baseline_nonzero}, peak={baseline_peak}）"
    );

    // ---- 直接调宿主回调（下标 0 / 1）⇒ Δ0 ＋ `sampler` 仍 `None`。
    let (live, port) = live_window_with_undo(&project, "#165 拒绝");
    let before = port.display();
    live.ui().invoke_instrument_install(0);
    live.ui().invoke_instrument_install(1);
    let after = port.display();
    assert_eq!(after.commit_count, before.commit_count, "拒绝 ⇒ Δ0 提交");
    assert_eq!(after.undoable, before.undoable, "拒绝 ⇒ Δ0 撤销步");
    assert_eq!(
        port.project().tracks.get(&track_id).expect("轨在").sampler,
        None,
        "拒绝之后 `sampler` 必须仍是 `None`"
    );

    // ---- 纯函数层：两份候选都拒绝，原因**逐字**。
    for candidate in BUILTIN_INSTRUMENTS {
        assert_eq!(
            sfz_samples_present(candidate.path),
            Err(INSTRUMENT_INSTALL_NO_SAMPLES_REASON),
            "`{}` 的样本字节不在仓库",
            candidate.path
        );
        assert_eq!(
            instrument_install_ops(&project, candidate.path),
            Err(INSTRUMENT_INSTALL_NO_SAMPLES_REASON),
            "`{}` 必须装不掉",
            candidate.path
        );
    }

    // ---- 装不掉之后：这条轨仍然有声，且与装之前**逐位相同**。
    let after_render = render(&port.project());
    let (after_nonzero, after_peak) = stats(&after_render.samples);
    assert_eq!(
        after_render.digest, baseline.digest,
        "拒绝之后工程一位没动 ⇒ 渲染摘要必须逐位相同"
    );
    assert_eq!(after_nonzero, baseline_nonzero);
    assert_eq!(after_peak, baseline_peak);
    assert_eq!(
        port.project().tracks.get(&track_id).expect("轨在").sampler,
        None
    );

    report_line(&format!(
        "[#165 ②③] 装之前 non-zero={baseline_nonzero} peak={baseline_peak:.6} digest={}；\
         回调 Δ0（commit {}→{}）· sampler=None · 纯函数=Err({:?})；\
         装不掉之后 non-zero={after_nonzero} peak={after_peak:.6} digest={}（逐位相同）",
        baseline.digest,
        before.commit_count,
        after.commit_count,
        INSTRUMENT_INSTALL_NO_SAMPLES_REASON,
        after_render.digest,
    ));
}

// ===========================================================================
// `#165` 谓词唯一：可装清单 = 候选 × `sfz_samples_present`（同一条读取路径）
// ===========================================================================
#[test]
fn the_sample_presence_predicate_is_the_shared_decode_path() {
    isolate_repo_only_library();
    let fixture = AudioFixture::new("predicate");
    let fixture_path = fixture.sfz.to_str().expect("临时路径必须是 UTF-8");
    assert_eq!(
        sfz_samples_present(fixture_path),
        Ok(()),
        "1 region ＋ 1 份 24-bit WAV ⇒ 样本字节存在"
    );

    // 样本缺失的 SFZ（文件在、`sample=` 指向的 WAV 不在）⇒ 逐字原因。
    let missing = fixture.dir.join("missing-sample.sfz");
    std::fs::write(
        &missing,
        "<region> sample=nope.wav key=60 pitch_keycenter=60\n",
    )
    .expect("写缺样本 SFZ");
    assert_eq!(
        sfz_samples_present(missing.to_str().expect("UTF-8")),
        Err(INSTRUMENT_INSTALL_NO_SAMPLES_REASON)
    );

    // SFZ 文本本身读不到 ⇒ 另一句逐字原因（⛔ 不与"样本缺失"共用假话）。
    assert_eq!(
        sfz_samples_present("/nonexistent/yeban-165.sfz"),
        Err(INSTRUMENT_INSTALL_UNREADABLE_SFZ_REASON)
    );

    // 可装清单 = 候选 × 同一个谓词（今天结果为空）。
    let expected: Vec<_> = BUILTIN_INSTRUMENTS
        .iter()
        .copied()
        .filter(|candidate| sfz_samples_present(candidate.path).is_ok())
        .collect();
    assert_eq!(installable_builtin_instruments(), expected);
    assert_eq!(
        instrument_names(),
        expected
            .iter()
            .map(|candidate| candidate.name.to_owned())
            .collect::<Vec<_>>()
    );
    assert_eq!(builtin_instrument(0), expected.first().copied());

    report_line(&format!(
        "[#165 谓词] 夹具={:?}；缺样本=Err(NO_SAMPLES)；读不到=Err(UNREADABLE)；\
         候选={} 可装={}（= 同一谓词过滤）",
        sfz_samples_present(fixture_path),
        BUILTIN_INSTRUMENTS.len(),
        expected.len(),
    ));
}
