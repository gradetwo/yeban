//! **生产窗口的电平腿**端到端判据（本票补上的那一条缺口）。
//!
//! ## 缺口是什么（实测，不是转述）
//!
//! `src/main.rs` 的 `run_gui` 曾经这么写：
//!
//! ```text
//! if let Err(error) = engine.reload(&loaded.archive.project, 0) { … }
//! ```
//!
//! `Ok` 那一侧**整个被丢掉**，而 `Ok` 装的 `EngineRebuild` 里有 `collector`
//! （新引擎的电平消费端，`crates/yeban-app/src/engine_host.rs:289` 的注释写着
//! "**UI 线程必须采纳它**"）。后果：引擎侧那条 SPSC 没有消费者，而生产窗口里的
//! 电平表永远不动。
//!
//! ## 这个文件证明什么
//!
//! 生产窗口的**一跳**（`src/main.rs` 的 16 ms `slint::Timer` 体）被抽成了
//! [`yeban_app::host::ProductionLoop::tick`]。因此判据可以在**无显示器**环境里
//! 驱动那一跳：真 `EngineHost`（`EngineHost::reload`）、真 `MeterRuntime`、
//! 真 `host::apply_meters`、真 Tier-1 软件光栅化窗口。
//!
//! 1. `production_loop_start_adopts_the_engine_meter_consumer` —— **采纳消费端**那一格。
//!    真引擎每一跳推量子，读数按**量子序号**单调前进；消费端被丢掉时那个序号永远是 `None`。
//! 2. `production_tick_writes_known_levels_into_the_slint_properties` —— **注入已知帧**
//!    那一格。逐跳打印 Slint 属性里的字面 dBFS，钉住"引擎的电平真的进了界面"，
//!    而不只是"某个计数器在动"。
//! 3. `production_tick_does_not_inject_when_the_window_track_count_disagrees` ——
//!    长度契约被打破（控制面换了工程）时**只抽干、不注入**，且**不 panic**。
//!
//! ## 这个文件**不**证明什么（边界，不许当成已证）
//!
//! - ⛔ **生产窗口本身没有被执行**：`run_gui` 要求一个真显示器（`MainWindow::new()`
//!   经 `backend-winit`），本机与 CI 都没有 ⇒ `slint::ComponentHandle::run` 那一跳
//!   **没有被跑过**。这里驱动的是它调用的**同一个**库函数。
//! - ⛔ "窗口关闭 ⇒ tick 停"没有在本文件里被动态观测：停止由 `slint::Timer` 的 `Drop`
//!   完成（上游 `i-slint-core-1.18.1/timers.rs:253` 的 `impl Drop for Timer` 把槽从
//!   定时器表摘掉并 drop 回调），而本文件的窗口不进事件循环 ⇒ 定时器根本不跳。
//! - ✅ **引擎真的在合成**（轨道渲染**不再**是占位静音）⇒ 走真引擎的那条判据读到的
//!   **不只是**量子序号：它读到引擎自己发布的**非注入**电平（本机实测 8 跳，出处 =
//!   `cargo test -p yeban-app --test production_meter_leg production_loop_start -- --nocapture`
//!   的 `[meter-leg] tick #0‥#7` 行，2026-10-08：0 号轨 `-17.3 / -7.1 / -6.8 / -7.3 / -7.6 /
//!   -7.9 / -8.3 / -8.5` dBFS，主总线 `-20.4 / -13.1 / -9.8 / -10.0 / -10.6 / -10.9 /
//!   -11.1 / -11.5` dBFS）。这里原先写「引擎仍然**不发声**（轨道渲染是占位静音）⇒ 走真
//!   引擎的那条判据读到的是**量子序号**在前进，不是"真的有声音"」—— 那句话与本文件判据
//!   `production_loop_start_adopts_the_engine_meter_consumer`（断言"至少一条轨的读数必须
//!   离开显示下限"）**矛盾**，因此**就地改正**。判据 2 仍靠**注入**已知帧：它证明的是
//!   "注入的幅度原样进了界面"，与"引擎有没有声音"是两件事、各有判据。
//! - ⛔ `src/main.rs` 的 `run_gui` 装配（`ProductionLoop::start` 的调用点）是**二进制**里的
//!   代码，集成测试看不到它。本文件证明的是"`ProductionLoop` 的装配与逐跳正确"，
//!   加上 `run_gui` 只调用这一处（结构上无法再丢掉消费端）。
//!
//! ## 顺序契约（判据依赖它）
//!
//! `poll`（抽干）→ `snapshot`（对齐投影）→ `apply_meters`（写 Slint 属性）。
//! 判据在**每一次** `tick` 之后立刻读窗口属性，因此"某一跳没抽"必然表现为
//! "窗口上还是上一跳的值"。

use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::engine_host::EditMark;
use yeban_app::host::{self, MeterPump, ProductionLoop};
use yeban_app::scene::DemoScene;
use yeban_engine::meter::{MeterFrame, MeterPublisher, meter_channel};
use yeban_model::ids::EntityId;
use yeban_model::project::YebanProjectV1;
use yeban_ui_test_port::LivePort;
use yeban_ui_test_port::image::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// `[string]` 属性 → `Vec<String>`（**读回注入面**，与生产写入面配对）。
fn strings_of(model: &slint::ModelRc<slint::SharedString>) -> Vec<String> {
    use slint::Model as _;
    model.iter().map(|value| value.to_string()).collect()
}

/// `[length]` 属性 → `Vec<f32>`。
fn lengths_of(model: &slint::ModelRc<f32>) -> Vec<f32> {
    use slint::Model as _;
    model.iter().collect()
}

/// 一个"活窗口 + 投影"的夹具（与 `src/main.rs` 共用 `host::build_main_window`）。
struct Window {
    port: LivePort<yeban_app::ui::MainWindow>,
    view: ViewState,
}

impl Window {
    /// 默认控制台 Tab（0 = 编曲视图那一档；电平条**不在**这一档里）。
    fn new(project: &YebanProjectV1) -> Self {
        Self::with_console_tab(project, 0)
    }

    /// 指定控制台 Tab 的窗口（`1` = 混音台，**电平条住在那一档**）。
    fn with_console_tab(project: &YebanProjectV1, console_tab: i32) -> Self {
        let view = ViewState::from_project(project).expect("工程必须能投影");
        let scene = DemoScene::from_view(&view);
        let size = Size::new(scene.viewport_width, scene.viewport_height);
        let port = LivePort::new(size, Permission::ReadOnly, None, || {
            host::build_main_window_with_console_tab(&view, &scene, console_tab)
        })
        .expect("Tier-1 平台 + 主窗口");
        Self { port, view }
    }

    fn ui(&self) -> &yeban_app::ui::MainWindow {
        self.port.ui()
    }

    /// 界面**此刻**的每轨峰值 dBFS 文本（`track-meter-peaks` 属性的原文）。
    fn peak_labels(&self) -> Vec<String> {
        strings_of(&self.ui().get_track_meter_peaks())
    }

    /// 界面**此刻**的每轨柱高（`track-meter-levels` 属性的原文）。
    fn levels(&self) -> Vec<f32> {
        lengths_of(&self.ui().get_track_meter_levels())
    }

    /// 界面此刻的主控柱高（第三条独立路径）。
    fn master_level(&self) -> f32 {
        self.ui().get_master_meter_level()
    }

    /// 界面此刻的主控峰值文本（第四条独立路径）。
    fn master_peak(&self) -> String {
        self.ui().get_master_meter_peak().to_string()
    }
}

/// 序数化的探针行：每一次读数都带上"第几跳 + 哪一个探针 + 字面值"。
///
/// ⚠ 为什么必须序号化：一条判据如果只有一个固定探针，它会在"整体没接线"时**空转**
/// （读到默认值也当成"通过"）。这里每次 `tick` 之后把**四条**不同路径一起打出来。
fn report_tick(tick_no: usize, pump: &MeterPump, window: &Window) {
    let applied = match pump {
        MeterPump::Applied(snapshot) => format!(
            "Applied(quantum={:?}, nodes_seen={}, master_label={})",
            snapshot.quantum,
            snapshot.nodes_seen,
            snapshot.master_peak_label()
        ),
        MeterPump::NoEngine => "NoEngine".to_owned(),
        MeterPump::LengthMismatch { rows, projected } => {
            format!("LengthMismatch(rows={rows}, projected={projected})")
        }
    };
    report_line(&format!(
        "[meter-leg] tick #{tick_no} pump={applied} peaks={:?} levels={:?} master_level={} master_peak={:?}",
        window.peak_labels(),
        window.levels(),
        window.master_level(),
        window.master_peak()
    ));
}

/// 一跳：发布这一跳的帧 → 走 [`ProductionLoop::tick`] → 打印四条探针。
fn tick_with(
    production: &mut ProductionLoop,
    window: &Window,
    publisher: &mut MeterPublisher,
    tick_no: u64,
    frames: &[MeterFrame],
) -> MeterPump {
    publisher.publish(frames);
    let tick = production.tick(window.ui(), &window.view, EditMark::new(None, 0), None);
    let pump = tick.meters;
    report_tick(usize::try_from(tick_no).expect("序号"), &pump, window);
    pump
}

/// 工程里每一轨的**身份**（顺序 = 投影顺序）。
fn track_ids(project: &YebanProjectV1, view: &ViewState) -> Vec<EntityId> {
    view.tracks
        .iter()
        .map(|track| {
            project
                .tracks
                .keys()
                .find(|id| id.to_canonical_string() == track.id)
                .copied()
                .unwrap_or_else(|| panic!("工程里必须有身份 {} 的轨道", track.id))
        })
        .collect()
}

/// 判据 1（**采纳消费端**）：真引擎每一跳推量子 ⇒ 读数的**量子序号连续前进**，
/// 而且**引擎自己发布的真实电平**（不是注入的）真的到达界面。
///
/// 这一条直接钉住本票的缺口：
///
/// - 若 `ProductionLoop::start` 丢掉 `EngineRebuild::collector`（旧行为），
///   面板里一帧都没有（`quantum == None`、`nodes_seen == 0`），本判据在**第一跳**就红；
/// - 若 `ProductionLoop::tick` 不 `poll`，量子序号会停在第 1 跳的值上，**第 2 跳**就红。
///
/// 走带状态：本判据**不**调 `EngineHost::stop`（`reload` 的默认是自由跑 ⇒ `Playing`），
/// 因此引擎真的逐轨合成。生产 `run_gui` 启动时会 `stop()`（界面初始态是 `playing: false`），
/// 那时电平恒为显示下限 —— 同一条链，只是读数落在另一端。
#[test]
fn production_loop_start_adopts_the_engine_meter_consumer() {
    let project = demo_project();
    let window = Window::new(&project);
    let expected_nodes = window.view.tracks.len() + 1;

    // 与 `src/main.rs` **同一行**装配：真的建一代引擎，并由它把消费端交给 UI 线程。
    let mut production = ProductionLoop::start(&project, 0).expect("引擎重建");
    assert!(production.has_engine(), "引擎应当活着");
    assert!(
        production.meters().has_collector(),
        "ProductionLoop::start 必须把 EngineRebuild::collector 采纳进来"
    );

    let mut quanta = Vec::new();
    let mut label_rows: Vec<Vec<String>> = Vec::new();
    for tick_no in 0..8u64 {
        // 设备回调那一侧的替身：真的推**一个**量子（`process_quantum` 就是 cpal 回调
        // 会调的那一个函数，只是这里由本线程显式驱动）。
        production.engine_handle().borrow_mut().drive_audio(1);
        let tick = production.tick(window.ui(), &window.view, EditMark::new(None, 0), None);
        let MeterPump::Applied(snapshot) = &tick.meters else {
            panic!(
                "第 {tick_no} 跳必须真的把电平写进界面, 实际 {:?}",
                tick.meters
            );
        };
        report_tick(
            usize::try_from(tick_no).expect("序号"),
            &tick.meters,
            &window,
        );
        assert_eq!(
            snapshot.nodes_seen, expected_nodes,
            "第 {tick_no} 跳必须见到 `非母线轨数 + 1` 个节点 —— 这一条**只**在真的抽到帧时成立"
        );
        assert_eq!(
            tick.readings.map(|readings| readings.published),
            Some(1),
            "`reload` 已经发布过 1 次快照 (quanta = 0 ⇒ 心跳的 `published` 停在 1)"
        );
        // 读数必须**恒有限**：`NaN` 不许出现在界面上。
        for (index, label) in window.peak_labels().iter().enumerate() {
            assert!(
                !label.contains("NaN") && label.parse::<f32>().is_ok(),
                "第 {tick_no} 跳第 {index} 轨的标签 {label:?} 必须是可解析的有限值"
            );
        }
        quanta.push(snapshot.quantum);
        label_rows.push(window.peak_labels());
    }

    // 序号必须**严格递增**：任何一跳没抽都会在这里留下一个重复值 / `None`。
    let numbers: Vec<u64> = quanta
        .iter()
        .map(|quantum| quantum.expect("每一跳都必须见到帧"))
        .collect();
    assert!(
        numbers.windows(2).all(|pair| pair[1] > pair[0]),
        "8 跳的量子序号必须严格递增, 实际 {numbers:?}"
    );
    assert_eq!(
        numbers.first(),
        Some(&1),
        "第 1 跳的序号是 1（引擎从 1 起计）"
    );

    // ⭐ **真引擎发布的、非注入的**电平真的到达界面：至少有一跳的某一轨离开显示下限。
    let lit: Vec<(usize, usize, String)> = label_rows
        .iter()
        .enumerate()
        .flat_map(|(tick_no, labels)| {
            labels
                .iter()
                .enumerate()
                .filter(|(_, label)| label.as_str() != "-120.0")
                .map(move |(index, label)| (tick_no, index, label.clone()))
        })
        .collect();
    assert!(
        !lit.is_empty(),
        "引擎真的在合成 ⇒ 至少一条轨的读数必须离开显示下限; 实际 8 跳全是 -120.0: {label_rows:?}"
    );
    report_line(&format!(
        "[meter-leg] 8 跳的量子序号 = {numbers:?}; 离开下限的读数（跳号, 轨号, dBFS）= {:?}",
        &lit[..lit.len().min(6)]
    ));
}

/// 判据 2（**已知帧进界面**）：逐跳注入已知幅度 ⇒ Slint 属性里的字面 dBFS 与之一致。
///
/// 本条判据**注入**已知幅度，而不是等引擎自己发声。为什么要注入：它要证明的是
/// "**给定的**那一帧幅度原样进了界面"（`{:.1}` 的显示步长内），而不是"引擎此刻在响什么"——
/// 后者由判据 1 单独证明（真引擎**真的在合成**，见文件头 ✅ 那一条，本机实测 8 跳的
/// 非下限读数）。两件事各有判据，互不代替。
///
/// 注入走的是**真** `meter_channel` 与**真** `ProductionLoop::tick`：唯一被替换的是消费端的
/// 来源（引擎的交出物换成判据自己建的那一条），tick 的调用序列一位没变。
///
/// ⚠ 原文写「真引擎不发声，因此"有数值"这一半只能靠注入」—— 那是**同一条假话**的第三处
/// （另两处在文件头 `:39-49`，**上一线**已就地改正）：引擎**在**合成（`project_schedules`
/// 真的按摆放生成调度表，`synth.render_track` 真的渲染），"有数值"这一半因此**不是**
/// 只能靠注入 —— 判据 1 读到的就是引擎自己发布的非注入读数。本条判据保留注入，
/// 理由是**分辨力**（已知幅度 ⇒ 逐位对账），不是"引擎不发声"。就地改正。
#[test]
fn production_tick_writes_known_levels_into_the_slint_properties() {
    let project = demo_project();
    let window = Window::new(&project);
    assert!(
        window.view.tracks.len() >= 3,
        "演示工程至少 3 条非主总线轨道, 否则本条判据没有分辨力"
    );
    let ids = track_ids(&project, &window.view);
    let master_id = project.master_bus_track_id;

    let mut production = ProductionLoop::start(&project, 0).expect("引擎重建");
    let (mut publisher, collector) = meter_channel(64);
    production.meters_mut().adopt(collector);

    // ---- 第 0 跳：一条帧都没有 ⇒ 全部下限（"无"的那一侧） -------------------
    let pump = tick_with(&mut production, &window, &mut publisher, 0, &[]);
    assert!(matches!(pump, MeterPump::Applied(_)));
    assert_eq!(
        window.peak_labels(),
        vec!["-120.0".to_owned(); window.view.tracks.len()],
        "没有帧 ⇒ 每一轨都是显示下限（有限值, 不是空串也不是 NaN）"
    );
    assert!(
        window.levels().iter().all(|level| *level == 0.0),
        "没有帧 ⇒ 柱高全 0"
    );
    assert_eq!(window.master_level(), 0.0);

    // ---- 第 1 跳：注入三条已知幅度 + 母线 ⇒ "从无到有" ----------------------
    let pump = tick_with(
        &mut production,
        &window,
        &mut publisher,
        1,
        &[
            // 满幅 ⇒ 20·log10(1.0) = 0.0 dBFS
            MeterFrame::new(ids[0], 5, 1.0, 1.0),
            // 半幅 ⇒ 20·log10(0.5) = -6.0206 ⇒ 显示 "-6.0"
            MeterFrame::new(ids[1], 5, 0.5, 0.25),
            // 静音 ⇒ 下限
            MeterFrame::new(ids[2], 5, 0.0, 0.0),
            // 母线单独一条
            MeterFrame::new(master_id, 5, 1.0, 1.0),
        ],
    );
    let MeterPump::Applied(snapshot) = pump else {
        panic!("第 1 跳必须写进界面, 实际 {pump:?}");
    };
    assert_eq!(snapshot.quantum, Some(5));
    assert_eq!(snapshot.nodes_seen, 4);
    let peaks = window.peak_labels();
    assert_eq!(
        &peaks[..3],
        ["0.0", "-6.0", "-120.0"],
        "字面 dBFS 必须与注入的幅度一一对应（`{{:.1}}` 的显示步长是 0.05 dB）"
    );
    assert!(
        (window.levels()[1] - (1.0 - 6.020_6 / 120.0)).abs() < 1e-3,
        "半幅的柱高必须≈0.9498, 实际 {}",
        window.levels()[1]
    );
    assert_eq!(window.master_level(), 1.0);
    assert_eq!(window.master_peak(), "0.0");
    report_line(&format!(
        "[meter-leg] 第 1 跳: 注入 [1.0, 0.5, 0.0] ⇒ 界面 peaks={:?} levels={:?}",
        &peaks[..3],
        &window.levels()[..3]
    ));

    // ---- 第 2 跳：更**新**的量子把旧值换掉 ⇒ 界面必须跟着动 ---------------
    let pump = tick_with(
        &mut production,
        &window,
        &mut publisher,
        2,
        &[
            // 四分之一幅 ⇒ 20·log10(0.25) = -12.0412 ⇒ 显示 "-12.0"
            MeterFrame::new(ids[0], 6, 0.25, 0.0625),
            MeterFrame::new(ids[1], 6, 1.0, 1.0),
            MeterFrame::new(ids[2], 6, 0.5, 0.25),
            MeterFrame::new(master_id, 6, 0.5, 0.25),
        ],
    );
    let MeterPump::Applied(snapshot) = pump else {
        panic!("第 2 跳必须写进界面, 实际 {pump:?}");
    };
    assert_eq!(snapshot.quantum, Some(6));
    assert_eq!(
        &window.peak_labels()[..3],
        ["-12.0", "0.0", "-6.0"],
        "第 2 跳的值必须覆盖第 1 跳（任何一跳不抽都会留下上一跳的值）"
    );
    assert!(
        (window.master_level() - (1.0 - 6.020_6 / 120.0)).abs() < 1e-3,
        "母线半幅 ⇒ 柱高≈0.9498, 实际 {}",
        window.master_level()
    );

    // ---- 第 3 跳：迟到的**旧**量子不许把界面顶回去 --------------------------
    let pump = tick_with(
        &mut production,
        &window,
        &mut publisher,
        3,
        &[
            MeterFrame::new(ids[0], 2, 0.001, 0.001),
            MeterFrame::new(ids[1], 2, 0.001, 0.001),
            MeterFrame::new(ids[2], 2, 0.001, 0.001),
        ],
    );
    let MeterPump::Applied(snapshot) = pump else {
        panic!("第 3 跳必须写进界面, 实际 {pump:?}");
    };
    assert_eq!(
        &window.peak_labels()[..3],
        ["-12.0", "0.0", "-6.0"],
        "迟到的旧帧永远顶不回新值（`level::supersedes` 的消费侧形态）"
    );
    report_line(&format!(
        "[meter-leg] 第 3 跳（注入 quantum=2 的旧帧）: 界面仍是 {:?}",
        &window.peak_labels()[..3]
    ));

    // 注入的轨道身份与投影逐个对齐（按下标取到别的轨道会让上面每一条断言失去意义）。
    assert_eq!(
        snapshot.tracks.len(),
        window.view.tracks.len(),
        "快照的通道条数必须与投影等长"
    );
    for (index, track) in snapshot.tracks.iter().enumerate() {
        assert_eq!(
            track.node_id, window.view.tracks[index].id,
            "第 {index} 条的身份必须来自投影"
        );
        assert_eq!(track.name, window.view.tracks[index].name);
    }
}

/// 判据 3（**长度契约**）：界面轨道数与手里那份投影不等长 ⇒ 只抽干、不注入、不 panic。
///
/// 造法：窗口上装载 **filled** 工程（轨道数与演示工程不同），再用**演示**工程的投影驱动
/// tick。这正是"控制面 `yeban_open_project` 换了工程、而持有 tick 的那一段还拿着旧投影"
/// 的形态。`apply_meters` 的长度契约（`.slint` 按下标取）因此不能被违反。
#[test]
fn production_tick_does_not_inject_when_the_window_track_count_disagrees() {
    let filled = yeban_model::samples::filled_project();
    let demo = demo_project();
    assert_ne!(
        filled.tracks.len(),
        demo.tracks.len(),
        "两个样本的轨道数必须不同, 否则本条判据没有分辨力"
    );
    let window = Window::new(&filled);
    let before = window.peak_labels();
    assert_eq!(
        before.len(),
        window.view.tracks.len(),
        "窗口上应当是 filled 工程的通道条（`view.tracks` **不含**主总线）"
    );
    assert_eq!(before.len(), 3, "filled 工程有 3 条非主总线轨道");

    let mut production = ProductionLoop::start(&demo, 0).expect("引擎重建");
    let (mut publisher, collector) = meter_channel(64);
    production.meters_mut().adopt(collector);
    publisher.publish(&[MeterFrame::new(demo.master_bus_track_id, 3, 1.0, 1.0)]);

    // ⚠ 关键的错位：窗口上是 **filled** 的 3 条轨道，而这一跳手里是 **demo** 的投影
    // （6 条轨道）。这正是"控制面换了工程、tick 还拿着旧投影"的形态。
    let stale_view = ViewState::from_project(&demo).expect("演示工程投影");
    assert_eq!(stale_view.tracks.len(), 6);
    let tick = production.tick(window.ui(), &stale_view, EditMark::new(None, 0), None);
    let MeterPump::LengthMismatch { rows, projected } = tick.meters else {
        panic!("长度不等长时必须跳过注入, 实际 {:?}", tick.meters);
    };
    assert_eq!(rows, 3, "回报的是界面此刻的长度（filled 的 3 条轨道）");
    assert_eq!(
        projected, 6,
        "回报的是手里那份投影的长度（演示工程 6 条轨道）"
    );
    assert_eq!(
        window.peak_labels(),
        before,
        "长度不等长 ⇒ 界面必须一位没动（注入会按下标串到别的轨道）"
    );
    report_line(&format!(
        "[meter-leg] 长度不等长: rows={rows} projected={projected} ⇒ 界面仍是 {:?}",
        &before[..3]
    ));

    // 队列**仍然**被抽干 ⇒ 音频侧不会因为界面换了工程而积压。
    // 这一条走 `MeterPump` 之外的第二条路径（直接问运行时）。
    assert!(
        production.meters().board().latest_quantum() == Some(3),
        "即使不注入, 这一跳也必须真的抽干队列（板上有最新量子）, 实际 {:?}",
        production.meters().board().latest_quantum()
    );
}

/// 判据 4（**像素，命题一**）：默认外观（编曲视图那一帧）逐字节可复现，
/// 而且**不随电平腿变化** —— 因为电平条根本不在这一档里。
///
/// ⚠ **两个命题必须分开**（本仓库明文警告过这一类误读）：
///
/// - "默认帧逐字节可复现" 说的是**外观的确定性**；
/// - "默认帧不随电平变" 说的是**这一档画面里没有电平条** —— 它**不**蕴含"电平没接上"。
///   恰恰相反：电平条住在**混音台那一档**（`ui/console/mixer_console.slint:251` 的
///   `100px * root.track-meter-levels[track_index]`，只有控制台 Tab = 1 才在控件树里）。
///   要判"接上了没有"，靠的是本文件前三条判据，以及下一条**混音台档**的像素判据。
#[test]
fn the_default_frame_is_byte_reproducible_and_unaffected_by_the_meter_leg() {
    use yeban_ui_test_port::png::encode_rgb8;

    let project = demo_project();
    let window = Window::new(&project);

    let first = window.port.window().capture().expect("Tier-1 截图");
    let second = window.port.window().capture().expect("第二次截图");
    let before = encode_rgb8(&first);
    let before_again = encode_rgb8(&second);
    // ⚠ 用 `assert!` 而不是 `assert_eq!`：后者失败时会把 6 MB 的 PNG 字节全打出来，
    // 把真正的失败行淹掉（实测踩过）。
    assert!(
        before == before_again,
        "同一窗口两次抓帧必须逐字节相同（未注入任何播放/电平变化）: {} vs {} 字节",
        before.len(),
        before_again.len()
    );
    assert_eq!(
        before.len(),
        6_222_418,
        "1920×1080 的存储式 deflate 帧恒为这个长度（长度相等不含内容信息, 所以还要看哈希）"
    );
    maybe_write_frame("default", &before);
    report_line(&format!(
        "[meter-leg] 默认帧（编曲视图）: {} 字节, 两次抓帧逐字节相同",
        before.len()
    ));

    // 驱动 8 跳（真引擎, 自由跑 ⇒ 电平读数真的离开下限）之后再抓一帧。
    let mut production = ProductionLoop::start(&project, 0).expect("引擎重建");
    let mut lit = 0usize;
    for _ in 0..8 {
        production.engine_handle().borrow_mut().drive_audio(1);
        let tick = production.tick(window.ui(), &window.view, EditMark::new(None, 0), None);
        if let MeterPump::Applied(snapshot) = &tick.meters
            && snapshot
                .peak_labels()
                .iter()
                .any(|label| label.as_str() != "-120.0")
        {
            lit += 1;
        }
    }
    let after_image = window.port.window().capture().expect("Tier-1 截图");
    let after = encode_rgb8(&after_image);
    let (changed, bbox) = pixel_diff(&first, &after_image);
    report_line(&format!(
        "[meter-leg] 默认帧在 {} 跳离开下限之后: {} 字节, 不同像素 {} 个, 包围盒 {:?}",
        lit,
        after.len(),
        changed,
        bbox
    ));
    assert!(
        lit > 0,
        "这 8 跳里至少有一跳的电平必须离开下限（否则本条没有搭建出前提）"
    );
    assert!(
        before == after,
        "编曲视图那一档不含电平条 ⇒ 这一帧**不该**随电平变化（变了说明有别的写入者）: 不同像素 {changed} 个"
    );
}

/// 判据 5（**像素，命题二**）：**混音台那一档**的画面真的随电平腿变化。
///
/// 这是"电平落在像素上"的判据：同一份投影、同一个窗口，先抓一帧，再驱动 8 跳
/// （真引擎），再抓一帧 —— 两帧必须不同，并给出不同像素数与包围盒。
#[test]
fn the_mixer_console_frame_changes_when_the_meter_leg_runs() {
    use yeban_ui_test_port::png::encode_rgb8;

    let project = demo_project();
    // 控制台 Tab = 1（混音台）——电平条只在这一档里。
    let window = Window::with_console_tab(&project, 1);
    let before_image = window.port.window().capture().expect("Tier-1 截图");
    let before = encode_rgb8(&before_image);
    maybe_write_frame("mixer-before", &before);

    let mut production = ProductionLoop::start(&project, 0).expect("引擎重建");
    for _ in 0..8 {
        production.engine_handle().borrow_mut().drive_audio(1);
        let _ = production.tick(window.ui(), &window.view, EditMark::new(None, 0), None);
    }
    let after_image = window.port.window().capture().expect("Tier-1 截图");
    let after = encode_rgb8(&after_image);
    maybe_write_frame("mixer-after", &after);

    let (changed, bbox) = pixel_diff(&before_image, &after_image);
    report_line(&format!(
        "[meter-leg] 混音台档: 默认 {} 字节 / 8 跳之后 {} 字节, 不同像素 {} 个, 包围盒 {:?}",
        before.len(),
        after.len(),
        changed,
        bbox
    ));
    assert!(before != after, "混音台档的画面必须随电平变化");
    assert!(changed > 0, "两条路径必须都报出非零差异");
}

/// 逐像素差异计数 + 包围盒 `(x0, y0, x1, y1)`（`None` = 两帧相同）。
fn pixel_diff(
    left: &yeban_ui_test_port::Rgb8Image,
    right: &yeban_ui_test_port::Rgb8Image,
) -> (usize, Option<(u32, u32, u32, u32)>) {
    assert_eq!(
        (left.width(), left.height()),
        (right.width(), right.height())
    );
    let (lhs, rhs) = (left.pixels(), right.pixels());
    let mut changed = 0usize;
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for y in 0..left.height() {
        for x in 0..left.width() {
            let offset = (y as usize * left.stride()) + (x as usize * 3);
            if lhs[offset..offset + 3] != rhs[offset..offset + 3] {
                changed += 1;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if changed == 0 {
        (0, None)
    } else {
        (changed, Some((x0, y0, x1, y1)))
    }
}

/// 判据运行的产物转储（**只在** `YEBAN_METER_LEG_FRAMES=<前缀>` 时写盘）。
///
/// 为什么要它：sha256 只能由 shell 的 `shasum` 算（本 crate 没有、也不许新增哈希依赖），
/// 而"从 PNG 字节读哈希"要求那一帧真的落成文件。默认**不写**（12 MB 不该在每次
/// `cargo test` 里产生）。
fn maybe_write_frame(label: &str, png: &[u8]) {
    let Ok(prefix) = std::env::var("YEBAN_METER_LEG_FRAMES") else {
        return;
    };
    let path = format!("{prefix}-{label}.png");
    std::fs::write(&path, png).expect("写帧");
    report_line(&format!("[meter-leg] 帧已写 {path}"));
}
