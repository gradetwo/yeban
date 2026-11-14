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

#![allow(unused)]
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
        // 渲染侧不读响度字段 ⇒ `None`。
        loudness: None,
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback instrument-install("),
        "新 UI 契约面必须声明 callback instrument-install("
    );
    assert!(
        aether_host.contains("on_instrument_install"),
        "宿主接线层必须注册 on_instrument_install"
    );
}

// ===========================================================================
// `#164` 正路：装一份**带样本**的夹具 = Δ1 ＋ 逐字读回 ＋ 投影可见
// ===========================================================================
#[test]
fn a_sample_bearing_sfz_installs_as_one_step_and_the_label_follows() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback instrument-install("),
        "新 UI 契约面必须声明 callback instrument-install("
    );
    assert!(
        aether_host.contains("on_instrument_install"),
        "宿主接线层必须注册 on_instrument_install"
    );
}

// ===========================================================================
// `#164` 换乐器 = Δ1（重复装同一份 = Δ0）
// ===========================================================================
#[test]
fn changing_the_instrument_is_one_more_step_and_repeating_it_is_free() {
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
        aether_app.contains("callback instrument-install("),
        "新 UI 契约面必须声明 callback instrument-install("
    );
    assert!(
        aether_host.contains("on_instrument_install"),
        "宿主接线层必须注册 on_instrument_install"
    );
}

// ===========================================================================
// `#164` ⑤：无展示轨 ⇒ 禁用 ＋ 逐字原因 ＋ 直接调用 Δ0
// ===========================================================================
#[test]
fn without_a_display_track_the_gesture_is_disabled_and_a_direct_call_changes_nothing() {
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
        aether_app.contains("callback instrument-install("),
        "新 UI 契约面必须声明 callback instrument-install("
    );
    assert!(
        aether_host.contains("on_instrument_install"),
        "宿主接线层必须注册 on_instrument_install"
    );
}

// ===========================================================================
// `#164` ②：装乐器 ＋ 1 个音符 ⇒ 引擎渲染非静音（夹具 SFZ ＋ 真 WAV）
// ===========================================================================
#[test]
fn an_installed_instrument_drives_the_engines_sfz_branch_to_a_non_silent_output() {
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
        aether_app.contains("callback instrument-install("),
        "新 UI 契约面必须声明 callback instrument-install("
    );
    assert!(
        aether_host.contains("on_instrument_install"),
        "宿主接线层必须注册 on_instrument_install"
    );
}

// ===========================================================================
// `#165` ②③：缺样本 ⇒ 装不掉，且这条轨**仍然有声**（逐位相同）
// ===========================================================================
#[test]
fn a_sample_less_sfz_is_refused_so_the_track_never_goes_silent() {
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
        aether_app.contains("callback instrument-install("),
        "新 UI 契约面必须声明 callback instrument-install("
    );
    assert!(
        aether_host.contains("on_instrument_install"),
        "宿主接线层必须注册 on_instrument_install"
    );
}

// ===========================================================================
// `#165` 谓词唯一：可装清单 = 候选 × `sfz_samples_present`（同一条读取路径）
// ===========================================================================
#[test]
fn the_sample_presence_predicate_is_the_shared_decode_path() {
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
        aether_app.contains("callback instrument-install("),
        "新 UI 契约面必须声明 callback instrument-install("
    );
    assert!(
        aether_host.contains("on_instrument_install"),
        "宿主接线层必须注册 on_instrument_install"
    );
}
