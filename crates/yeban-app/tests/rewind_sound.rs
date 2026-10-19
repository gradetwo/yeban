//! ⭐ **回起点之后必须真的出声**的端到端判据（任务单 `#231`）。
//!
//! ## 被钉住的缺陷（负责人逐字复验 `#227` 的结果）
//!
//! > ⭐ "两下回到 `001.01.000` 但是播放其实没有真正回到 `001.01.000` 因为没声音了"
//!
//! `#227` 只断言了**控制面**（`position_ticks == 0` ＋ 命令 `[SeekTicks(0)]`），
//! ⛔ 没有断言"之后按 `Space` 能出声"。本文件补的正是那一格：**音频面**真的开始发声。
//!
//! ## 根因（实测，`#231` 的复现矩阵）
//!
//! `EngineRuntime::render_block` 的 `SeekTicks` 分支只调
//! `SynthEngine::seek(frames)`：它搬播放头、释放声部，但**不动逐轨音符游标**。
//! 游标的纪律是"只增不减"（快照边界 `align_cursors` 用 `max`）。首次播放把游标
//! 推过起点处那个音符之后，`Stop + SeekTicks(0)` 把位置搬回 0、游标却留在"上一次
//! 播到哪" ⇒ 触发循环从旧游标往后看 ⇒ **回 0 之后一个音符都不起音**。
//! 修法见 `SynthEngine::realign_cursors` ＋ `rt.rs` 里 `if seeked` 的调用点。
//!
//! 复现矩阵（真音频腿，`demo_project`、48 kHz、120 BPM；修复**前**的读数）：
//!
//! | 格 | 键序列 | 非零样本 | 峰值 |
//! | :--- | :--- | ---: | ---: |
//! | 正对照 | 初始 → `Space` | 2048 | 0.324276 |
//! | 不定位的中间态 | 播 → `Space` 停 → `Space` 继续（`Play`，无 seek） | 2048 | 0.249923 |
//! | **本缺陷** | 播 → `Space` 停 → `Space` 回 0 → `Space` 播 | **0** | **0.000000** |
//! | 停止按钮 | 播 → 停止按钮（`Stop + SeekTicks(0)`）→ `Space` | **0** | **0.000000** |
//! | `Return`（播放中） | 播 → `Return`（`Stop + SeekTicks(0) + Play`） | **0** | **0.000000** |
//!
//! 修复后三条回 0 路径全部回到 `2048 / 0.324276`（与正对照逐位同量级）。
//!
//! ## 走的是哪条腿（第 321 条）
//!
//! 生产装配 [`ProductionLoop::start_with_audio_source`] ＋ 真 Tier-1 活窗口
//! （`host::build_main_window` / `host::wire_transport` / `host::wire_keys`）＋
//! [`EngineHost::render_into`]（内部就是 `EngineRuntime::process_quantum`）。
//! ⛔ 不是"只推控制面 `pump` 然后看 `position_ticks`"—— 那样正是 `#227` 的缺口。
//!
//! ## 边界（不许当成已证）
//!
//! - ⛔ **没有人耳听**：本机/CI 无显示器、无扬声器。判据证明的是"输出**非静音**"
//!   （峰值 > 0 ＋ 非零样本数），目标值就是"非零"（第 323 条）。
//! - ⛔ `run_gui` 的事件循环没有执行：这里驱动的是它调用的**同一个**装配。

use std::cell::RefCell;
use std::rc::Rc;

use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::engine_host::EngineHost;
use yeban_app::host::{self, ProductionLoop};
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_engine::ring::TransportCommand;
use yeban_engine::rt::EngineStats;
use yeban_engine::transport::TransportState;
use yeban_model::samples::filled_project;
use yeban_ui_test_port::LivePort;
use yeban_ui_test_port::image::Size;
use yeban_ui_test_port::port::Permission;

/// 工程声道的兜底值（`demo_project` 投影出来是 2；只有投影失败时才用它）。
const FALLBACK_CHANNELS: u16 = 2;

/// 一次渲染的读数：**音频面**的样本 ＋ **控制面**在同一瞬间的位置。
struct Rendered {
    samples: Vec<f32>,
    /// 渲染第一帧的**绝对帧号**（读自 `TransportReading::position_frames`，
    /// 即音频线程写出的原子镜面 —— ⛔ 不是测试自己数的）。
    start_frame: u64,
    /// 同一瞬间的绝对 tick（控制面读数）。
    start_ticks: u64,
    channels: u16,
}

impl Rendered {
    fn nonzero_and_peak(&self) -> (usize, f32) {
        (
            self.samples.iter().filter(|sample| **sample != 0.0).count(),
            self.samples
                .iter()
                .fold(0.0_f32, |acc, sample| acc.max(sample.abs())),
        )
    }

    /// **音频面第一次非零样本的绝对帧号**（任一 rip 声道非零即算；单位 = 帧）。
    fn first_nonzero_frame(&self) -> Option<u64> {
        let channels = usize::from(self.channels);
        self.samples
            .chunks_exact(channels)
            .position(|frame| frame.iter().any(|sample| *sample != 0.0))
            .map(|index| self.start_frame + index as u64)
    }
}

/// 生产形状的夹具：`ProductionLoop`（真引擎）＋ 真活窗口 ＋ 产品走带/键盘接线。
struct Rig {
    /// 持有它 ⇒ `EngineRebuild::collector` 的消费端一直在（与 `main.rs` 同一装配）。
    _production: ProductionLoop,
    engine: Rc<RefCell<EngineHost>>,
    port: LivePort<MainWindow>,
}

impl Rig {
    fn new() -> Self {
        Self::with_project(demo_project())
    }

    fn with_project(project: yeban_model::YebanProjectV1) -> Self {
        let production =
            ProductionLoop::start_with_audio_source(&project, 0, None).expect("产品装配建引擎");
        assert!(production.has_engine(), "引擎必须活着");
        let engine = production.engine_handle();
        // `src/main.rs` 的同一行：新引擎默认自由跑，而界面初始态是停住 ⇒ 显式 `Stop`。
        engine.borrow_mut().stop();
        assert_eq!(
            engine.borrow().transport().state,
            TransportState::Stopped,
            "夹具前置：必须停在 Stopped（第一次 `Space` 的语义是起播）"
        );

        let view = ViewState::from_project(&project).expect("demo_project 必须能投影");
        let scene = DemoScene::from_view(&view);
        let size = Size::new(scene.viewport_width, scene.viewport_height);
        let port = LivePort::new(size, Permission::Interactive, None, || {
            host::build_main_window(&view, &scene)
        })
        .expect("Tier-1 平台 + 主窗口");

        // 与 `main.rs` 同一对装配函数：走带回调 + 键盘动作。
        host::wire_transport(port.ui(), Rc::clone(&engine), None, None);
        let context = Rc::new(RefCell::new(yeban_app::input::InputContext::new()));
        host::wire_input(port.ui(), Rc::clone(&context));
        host::wire_keys(
            port.ui(),
            context,
            None,
            Some(Rc::clone(&engine)),
            Some(Rc::new(project)),
        );
        host::apply_transport(port.ui(), engine.borrow().transport());

        Self {
            _production: production,
            engine,
            port,
        }
    }

    fn ui(&self) -> &MainWindow {
        self.port.ui()
    }

    /// 按一次裸键（`" "` = `Space`，`"\n"` = `Return`）—— 走 `.slint` 的
    /// `key-action` 回调 ⇒ `wire_keys` ⇒ `apply_action`，与 #227 的 `press()` 同一条链。
    fn press(&self, text: &str) -> bool {
        self.ui()
            .invoke_key_action(text.into(), false, false, false, false)
    }

    fn reading(&self) -> yeban_engine::transport::TransportReading {
        self.engine.borrow().transport()
    }

    fn commands_since(&self, base: usize) -> Vec<TransportCommand> {
        self.engine.borrow().transport_journal()[base..]
            .iter()
            .map(|record| record.command)
            .collect()
    }

    fn journal_len(&self) -> usize {
        self.engine.borrow().transport_journal().len()
    }

    fn stats(&self) -> EngineStats {
        self.engine
            .borrow()
            .engine_stats()
            .expect("控制腿必须有统计读数")
    }

    /// 推 `quanta` 个量子并**把真实输出取回来**（`render_into` ⇒
    /// `EngineRuntime::process_quantum`，与设备回调同一个函数体）。
    fn render(&self, quanta: usize) -> Rendered {
        let (channels, reading) = {
            let engine = self.engine.borrow();
            let channels = engine
                .current_snapshot()
                .map_or(FALLBACK_CHANNELS, |snapshot| snapshot.channels().max(1));
            (channels, engine.transport())
        };
        let block = DEFAULT_BLOCK_FRAMES * usize::from(channels);
        let mut samples = vec![0.0_f32; block * quanta];
        let driven = self
            .engine
            .borrow_mut()
            .render_into(&mut samples, channels, quanta as u64);
        assert_eq!(
            driven, quanta as u64,
            "render_into 必须推进请求的量子数（控制腿没有设备）"
        );
        Rendered {
            samples,
            start_frame: reading.position_frames,
            start_ticks: reading.position_ticks,
            channels,
        }
    }
}

/// 判据 1 ＋ 正对照 ＋ 判据 6：`Space` → `Space` → `Space`（连按回 0）→ `Space`
/// ⇒ **必须出声**，且**音频面第一次非零样本的帧号**必须落在**控制面位置**上。
///
/// 同时读根因 ①/②/③ 的三条机械读数：
/// * ① `SeekTicks(0)` 真的到了音频线程：位置镜面 = 0（写者是音频线程），命令计数 +1；
/// * ② 定位**释放声部池**：seek 前 `poly_active_voices >= 1`、seek 后 = 0；
/// * ③ 修复的是**游标**：定位后 `notes_triggered` 必须再前进（修复前它一步不动）。
#[test]
fn space_stop_space_rewind_space_plays_with_sound() {
    let rig = Rig::new();

    // ---- 正对照：初始 → Space（播）⇒ 必须出声（证明夹具本身有声音）----
    let base = rig.journal_len();
    assert!(rig.press(" "), "Space 必须被消费");
    assert_eq!(
        rig.commands_since(base),
        vec![TransportCommand::Play],
        "第一次 Space 是 Play"
    );
    let first = rig.render(8);
    let (first_nonzero, first_peak) = first.nonzero_and_peak();
    println!(
        "[#231/正对照] 初始 → Space：非零样本={first_nonzero} 峰值={first_peak:.6} \
         渲染起点={} 帧 / {} tick",
        first.start_frame, first.start_ticks
    );
    assert!(
        first_nonzero > 0 && first_peak > 0.0,
        "正对照：第一次播放必须出声（非零={first_nonzero} 峰值={first_peak}）"
    );

    // ---- Space（停）⇒ 停在中途、位置保留 ----
    let base = rig.journal_len();
    assert!(rig.press(" "), "第二次 Space 必须被消费");
    assert_eq!(
        rig.commands_since(base),
        vec![TransportCommand::Stop],
        "第二次 Space 是 Stop（位置保留）"
    );
    let stopped = rig.reading();
    assert_eq!(stopped.state, TransportState::Stopped);
    assert!(
        stopped.position_ticks > 0,
        "前置：必须停在中途（实得 {} tick）",
        stopped.position_ticks
    );
    let before_seek = rig.stats();
    println!(
        "[#231/②] seek 前：位置={} tick / {} 帧；poly_active_voices={} notes_triggered={}",
        stopped.position_ticks,
        stopped.position_frames,
        before_seek.poly_active_voices,
        before_seek.notes_triggered
    );
    assert!(
        before_seek.poly_active_voices >= 1,
        "前置见证：播放中必须至少有一个活声部（实得 {}）",
        before_seek.poly_active_voices
    );

    // ---- Space（连按第二次）⇒ 回 0：命令只有 SeekTicks(0) ----
    let base = rig.journal_len();
    assert!(rig.press(" "), "第三次 Space 必须被消费");
    assert_eq!(
        rig.commands_since(base),
        vec![TransportCommand::SeekTicks(0)],
        "连按回 0 = 只有 seek、没有 Stop/Play"
    );
    let rewound = rig.reading();
    assert_eq!(rewound.position_ticks, 0, "控制面位置必须回 0");
    assert_eq!(rewound.position_frames, 0, "控制面帧位置必须回 0");
    assert_eq!(rewound.state, TransportState::Stopped, "回 0 但仍是停住");
    let after_seek = rig.stats();
    println!(
        "[#231/①] SeekTicks(0)：命令计数 commands_applied={}（+1，写者是音频线程）；\
         位置镜面={} tick / {} 帧",
        rewound.commands_applied, rewound.position_ticks, rewound.position_frames
    );
    println!(
        "[#231/②] seek 后：poly_active_voices={} notes_triggered={}（声部池已释放）",
        after_seek.poly_active_voices, after_seek.notes_triggered
    );
    assert_eq!(
        after_seek.poly_active_voices, 0,
        "定位必须释放声部池（② 的机械读数）"
    );

    // ---- Space（播）⇒ **本缺陷的判据：必须出声** ----
    let base = rig.journal_len();
    assert!(rig.press(" "), "第四次 Space 必须被消费");
    assert_eq!(
        rig.commands_since(base),
        vec![TransportCommand::Play],
        "回 0 之后的下一次 Space 是 Play"
    );
    let replay = rig.render(8);
    let (replay_nonzero, replay_peak) = replay.nonzero_and_peak();
    let first_frame = replay.first_nonzero_frame();
    println!(
        "[#231/本缺陷] 回 0 → Space：非零样本={replay_nonzero} 峰值={replay_peak:.6} \
         渲染起点={} 帧 / {} tick；音频面第一次非零帧={first_frame:?}",
        replay.start_frame, replay.start_ticks
    );
    println!(
        "[#231/③] 定位后的音符触发：notes_triggered {} → {}（修复前这一步恒等）",
        after_seek.notes_triggered,
        rig.stats().notes_triggered
    );
    assert!(
        replay_nonzero > 0,
        "回起点之后按播放必须有声音（非零样本 = {replay_nonzero}）—— #227 的缺口"
    );
    assert!(
        replay_peak > 0.0,
        "回起点之后按播放的峰值必须 > 0（实测 {replay_peak:.6}）"
    );

    // ---- 判据 6：两面一致（控制面位置 vs 音频面第一次非零帧）----
    let first_frame = first_frame.expect("非零输出必然有第一次非零帧");
    assert!(
        first_frame >= replay.start_frame,
        "音频面不得早于控制面位置起振：第一次非零帧 {first_frame} < 渲染起点 {}",
        replay.start_frame
    );
    assert!(
        first_frame < replay.start_frame + u64::from(DEFAULT_BLOCK_FRAMES as u32),
        "音频面必须在本量子内起振：第一次非零帧 {first_frame}，渲染起点 {}（一个量子 = {} 帧）",
        replay.start_frame,
        DEFAULT_BLOCK_FRAMES
    );
    assert_eq!(
        rig.stats().notes_triggered,
        after_seek.notes_triggered + 1,
        "回 0 后必须恰好补触发起点处那一个音符（③ 的反向对照）"
    );
}

/// 判据 2：**停止按钮**回 0（`Stop + SeekTicks(0)`）后 `Space` ⇒ 必须出声。
#[test]
fn stop_button_rewind_then_space_plays_with_sound() {
    let rig = Rig::new();

    assert!(rig.press(" "), "先起播");
    let warmup = rig.render(8);
    assert!(
        warmup.nonzero_and_peak().0 > 0,
        "前置：起播必须出声（否则下面的'停止按钮后有声'是空转）"
    );

    let base = rig.journal_len();
    rig.ui().invoke_stop();
    assert_eq!(
        rig.commands_since(base),
        vec![TransportCommand::Stop, TransportCommand::SeekTicks(0)],
        "停止按钮 = Stop + SeekTicks(0)"
    );
    assert_eq!(rig.reading().position_ticks, 0, "停止按钮必须回 0");

    let base = rig.journal_len();
    assert!(rig.press(" "), "停止按钮之后 Space 必须被消费");
    assert_eq!(rig.commands_since(base), vec![TransportCommand::Play]);
    let replayed = rig.render(8);
    let (nonzero, peak) = replayed.nonzero_and_peak();
    println!(
        "[#231/停止按钮] 停止按钮回 0 → Space：非零样本={nonzero} 峰值={peak:.6} \
         渲染起点={} 帧 / {} tick；第一次非零帧={:?}",
        replayed.start_frame,
        replayed.start_ticks,
        replayed.first_nonzero_frame()
    );
    assert!(nonzero > 0, "停止按钮回 0 后必须出声（非零 = {nonzero}）");
    assert!(
        peak > 0.0,
        "停止按钮回 0 后的峰值必须 > 0（实测 {peak:.6}）"
    );
}

/// 判据 3：`Return` 的**两条路**（停住时 / 播放中）⇒ 都必须出声。
///
/// * 停住时：`GoToBeginning` ⇒ `on_stop` ⇒ `[Stop, SeekTicks(0)]`；
/// * 播放中：`GoToBeginning` ⇒ `on_stop` ＋ `on_toggle_play` ⇒
///   `[Stop, SeekTicks(0), Play]`（`#227` 的逐字读数）。
#[test]
fn both_return_paths_play_with_sound() {
    let rig = Rig::new();

    // ---- 停住时按 Return ----
    let base = rig.journal_len();
    assert!(rig.press("\n"), "停住时的 Return 必须被消费");
    assert_eq!(
        rig.commands_since(base),
        vec![TransportCommand::Stop, TransportCommand::SeekTicks(0)],
        "停住时 Return = Stop + SeekTicks(0)"
    );
    assert_eq!(rig.reading().position_ticks, 0);
    assert!(rig.press(" "), "Return 之后 Space 必须被消费");
    let stopped_path = rig.render(8);
    let (nonzero, peak) = stopped_path.nonzero_and_peak();
    println!(
        "[#231/Return(停住)] 非零样本={nonzero} 峰值={peak:.6} 渲染起点={} 帧 / {} tick",
        stopped_path.start_frame, stopped_path.start_ticks
    );
    assert!(
        nonzero > 0 && peak > 0.0,
        "Return（停住时）之后必须出声（非零={nonzero} 峰值={peak}）"
    );

    // ---- 播放中按 Return（回 0 并继续播）----
    // 上一格已经把它们留在"播放中"（`Return`（停住）之后那一次 `Space` 起播了）。
    assert_eq!(
        rig.reading().state,
        TransportState::Playing,
        "前置：进播放中 Return 这一格时必须正在播"
    );
    assert!(rig.ui().get_playing(), "前置：界面显示态也必须正在播");
    let base = rig.journal_len();
    assert!(rig.press("\n"), "播放中的 Return 必须被消费");
    assert_eq!(
        rig.commands_since(base),
        vec![
            TransportCommand::Stop,
            TransportCommand::SeekTicks(0),
            TransportCommand::Play
        ],
        "播放中 Return = Stop + SeekTicks(0) + Play"
    );
    let after = rig.reading();
    // `[Stop, SeekTicks(0), Play]` 在**同一个量子边界**按 FIFO 生效 ⇒ 那个量子本身
    // 照常推进 128 帧（播放语义）⇒ 回 0 之后的位置恰好是"一个运行时量子"。
    // ⛔ 不是"没回 0"：`position_frames` 从 seek 后的 0 起算。
    assert_eq!(
        after.position_frames,
        u64::from(DEFAULT_BLOCK_FRAMES as u32),
        "Return 必须回到起点（本量子推进 128 帧：{after:?}）"
    );
    assert_eq!(
        after.state,
        TransportState::Playing,
        "播放中的 Return 回 0 后**继续播放**"
    );
    let playing_path = rig.render(8);
    let (nonzero, peak) = playing_path.nonzero_and_peak();
    println!(
        "[#231/Return(播放中)] 非零样本={nonzero} 峰值={peak:.6} 渲染起点={} 帧 / {} tick",
        playing_path.start_frame, playing_path.start_ticks
    );
    assert!(
        nonzero > 0 && peak > 0.0,
        "Return（播放中）之后必须出声（非零={nonzero} 峰值={peak}）"
    );
}

/// 判据 5 的**反向对照**：不定位的"停 → 继续"必须**本来就**有声（修复前也是）。
///
/// 它把"音频腿根本不发声"这个假设排除掉：同一夹具、同一渲染函数，差的只有
/// `SeekTicks` 这一条命令。修复前的实测：这一格 `2048 / 0.249923`，而三条回 0
/// 路径全是 `0 / 0.000000` ⇒ **seek 是破坏者**，不是引擎不出声。
#[test]
fn stopping_and_resuming_without_a_seek_was_already_audible() {
    let rig = Rig::new();
    assert!(rig.press(" "), "起播");
    assert!(
        rig.render(8).nonzero_and_peak().0 > 0,
        "前置：起播必须有声（否则本格是空转）"
    );
    assert!(rig.press(" "), "停（位置保留）");
    let stopped = rig.reading();
    assert_eq!(stopped.state, TransportState::Stopped);
    assert!(stopped.position_ticks > 0, "必须停在中途");

    // `Space` 的第二次在 1.5 s 窗内 ⇒ 它是"回起点"（`[SeekTicks(0)]`）。
    // 本格要的是**不经 seek** 的继续 ⇒ 直接发引擎的 `play()`（命令形状与 `#227`
    // 的"窗外"那一格逐字相同：`[Play]`，位置不回退）。
    let base = rig.journal_len();
    let reading = rig.engine.borrow_mut().play();
    assert_eq!(
        rig.commands_since(base),
        vec![TransportCommand::Play],
        "继续 = 只有 Play（⛔ 没有 SeekTicks）"
    );
    assert_eq!(reading.state, TransportState::Playing);
    assert!(
        reading.position_ticks >= stopped.position_ticks,
        "继续不得回退位置：{} → {}",
        stopped.position_ticks,
        reading.position_ticks
    );
    let resumed = rig.render(8);
    let (nonzero, peak) = resumed.nonzero_and_peak();
    println!(
        "[#231/不定位的中间态] 停 → 继续（Play，无 SeekTicks）：非零样本={nonzero} \
         峰值={peak:.6} 渲染起点={} 帧 / {} tick",
        resumed.start_frame, resumed.start_ticks
    );
    assert!(
        nonzero > 0 && peak > 0.0,
        "不定位的继续必须出声（非零={nonzero} 峰值={peak}）"
    );
}

/// 判据 5 的**方向对照**：前向 `seek` **本来就**有声（游标在目标之前，触发循环
/// 自然扫到目标音符）；**后向** `seek`（目标在游标之后 / 回 0）在修复前静音。
///
/// 这条把根因钉到"**游标只增不减**"而不是"`SeekTicks` 本身坏了"，也不是
/// "0 这个边界的特殊判定"：
///
/// | 夹具 | 方向 | 目标 | 修复前 | 修复后 |
/// | :--- | :--- | :--- | ---: | ---: |
/// | `filled_project` | 前向 | tick 1920 | 2048 / 0.336889（有声） | 2048 / 0.336889 |
/// | `filled_project` | **后向** | tick 960（先播过 2880） | **0 / 0.000000** | 2048 / 0.341896 |
///
/// ⚠ 为什么不用 tick 2880 当"中间位置"：`filled_project` 第 4 个音符的
/// `probability = 0.75` 在**确定性判定**下被丢弃 —— 从 0 播过 72000..76800 帧
/// 实测非零 = **0** ⇒ 那个位置本来就没声，拿它当证据会误导。
#[test]
fn forward_seek_plays_and_backward_seek_now_also_plays() {
    let rig = Rig::with_project(filled_project());

    // ---- 前向 seek(1920)（第三个音符）⇒ 修复前后都必须有声 ----
    rig.engine.borrow_mut().play();
    let warmup = rig.render(8);
    assert!(warmup.nonzero_and_peak().0 > 0, "前置：起播有声");
    rig.engine.borrow_mut().stop();
    rig.engine.borrow_mut().seek(1_920);
    rig.engine.borrow_mut().play();
    let forward = rig.render(8);
    let (nonzero, peak) = forward.nonzero_and_peak();
    println!(
        "[#231/方向对照] 前向 seek(1920) → 播：非零样本={nonzero} 峰值={peak:.6} \
         渲染起点={} 帧 / {} tick",
        forward.start_frame, forward.start_ticks
    );
    assert!(
        nonzero > 0 && peak > 0.0,
        "前向 seek 必须落到目标音符上（非零={nonzero} 峰值={peak}）"
    );

    // ---- 先播过 2880，再**后向** seek 回 960 ⇒ 修复前静音、修复后有声 ----
    rig.render(600);
    let before_backward = rig.reading().position_ticks;
    assert!(
        before_backward > 1_920,
        "前置：必须已经播过目标（实得 {before_backward} tick）"
    );
    rig.engine.borrow_mut().stop();
    rig.engine.borrow_mut().seek(960);
    rig.engine.borrow_mut().play();
    let backward = rig.render(8);
    let (nonzero, peak) = backward.nonzero_and_peak();
    println!(
        "[#231/方向对照] 后向 seek(960)（自 {before_backward} tick）→ 播：非零样本={nonzero} \
         峰值={peak:.6} 渲染起点={} 帧 / {} tick",
        backward.start_frame, backward.start_ticks
    );
    assert!(
        nonzero > 0 && peak > 0.0,
        "后向 seek 也必须出声（非零={nonzero} 峰值={peak}）—— 修复前这里是 0 / 0.000000"
    );
}
