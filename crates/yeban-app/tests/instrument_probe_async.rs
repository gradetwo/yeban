//! ⭐ 任务单 `#211`：装乐器的**字节探针搬到后台** ＋ **拒绝原因进界面**。
//!
//! ## 这一条判据要证的两句话（`#210` 暴露的两条用户体验缺陷）
//!
//! | 缺陷 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ⭐ "点一下界面卡 4 分钟" | 点一个**慢**乐器（128 份 × 0.5 s 样本）⇒ `invoke_instrument_install` 的墙钟 **< 100 ms**，且**立刻**可观察的界面状态是 `"检查中…：…"`，工程**一位没改**（提交还没发生） | 证"点击回调不在 UI 线程上解码"；⛔ 不证 Slint 事件循环的帧率（那需要真窗口 + GPU） |
//! | ⭐ "点了没反应" | 点缺样本的乐器 ⇒ 后台结果排空后控件树 `instrument-install-status` 的 `label` **逐字**是 `"装不了：… 缺样本（3 个文件）…"`；`commit_count` Δ0、`sampler` 仍 `None` | 证"拒绝原因进界面"（⛔ 只进 stderr）；⛔ 不证真库那 7 条的具体文件数（那要 `--probe-instruments` 的 10 分钟读数） |
//! | 结构证据 | `host.rs` 的 `on_instrument_install` 闭包里**真的**有 `std::thread::spawn`，慢调用 `probe_instrument_samples` 在**那个闭包内**；`apply_view` 里**没有**任何探针调用 | 证"探针在别的线程"＋"投影仍然不跑探针"（`#210` 的裁决） |
//! | CLI 读数 | 子进程 `--probe-instruments` 对 3 份夹具逐条打印 ok / fail（原因逐字），末行 `total=3 installable=1 missing=1 unreadable=1` | 证"一次看清哪些能装"；⛔ 不证真库 28 条（真库读数在交付报告里，实测约 10 分钟） |
//! | 撤销 | 装成功 = Δ1 提交 / Δ1 撤销步；`UiAction::Undo` 之后 `sampler` 回到 `None` | 证"后台只改变提交**时机**，不改变提交**次数**" |
//!
//! ## 为什么夹具是"真样本"而不是 mock
//!
//! 探针必须走**生产同一条**字节路径（`bridge::probe_instrument_samples` →
//! `sfz_load::decode_sfz_plan`）。因此慢夹具是 128 份真的 16-bit WAV（每份 0.5 s），
//! 缺样本夹具引用 3 个**不存在**的文件，读不出来夹具写的是**非法 UTF-8** 字节。
//! ⛔ 本文件不 mock 探针、⛔ 不改探针的判据。
//!
//! ## 它**不**证什么（如实登记）
//!
//! - ⛔ **没人耳听**：本文件只读 `commit_count` / `sampler` / 界面文本；
//! - ⛔ **GUI 鼠标点击本机不能真注入**：Tier-1 端口没有 `send_mouse_click`（上游 1.18.1
//!   不存在），本文件调的是生产同一个回调 `invoke_instrument_install`；
//! - ⛔ **不证 1920 基准帧**：那是 Tab 0，本单只动 Tab 3（读数见交付报告 §3）。

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use yeban_app::bridge::{ViewState, library_instrument_names};
use yeban_app::host::{self, InstrumentProbeState};
use yeban_app::samples::SAMPLE_LIBRARY_ENV;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_model::project::YebanProjectV1;
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::UiTestPort as _;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::tree::ControlTree;

/// 窗口尺寸（与 `track_instrument_switch.rs` / `instrument_install.rs` 同款）。
const WINDOW: Size = Size::new(1440, 1000);
/// 一次提交用的时间戳（与其余界面判据同一个值，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

// 三份夹具的**显示名**逐字取自仓库 `assets/samples/manifest.json`。
// 清单解析走的是**真实** manifest（`library_instruments` 只从库根找 SFZ），
// 因此夹具必须在库根下写这三条 manifest 路径对应的文件。
const SLOW_NAME: &str = "Cowsynth";
const MISSING_NAME: &str = "Karoryfer Pastabass (linguine)";
const UNREADABLE_NAME: &str = "Squidpipes";

const SLOW_SFZ: &str = "karoryfer-cowsynth/cowsynth_baggy.sfz";
const MISSING_SFZ: &str = "karoryfer-pastabass/linguine.sfz";
const UNREADABLE_SFZ: &str = "karoryfer-squidpipes/Programs/01-squidpipes.sfz";

/// 慢夹具的样本数 / 每份帧数（16-bit 单声道 48 kHz）—— 128 × 24,000 帧 ≈ 6 MiB。
const SLOW_SAMPLES: usize = 128;
const SLOW_FRAMES_PER_SAMPLE: usize = 24_000;
/// 缺样本夹具引用几个**不存在**的文件。
const MISSING_FILES: usize = 3;

/// 假库根（进程内建一次）：三份 manifest 乐器 ＋ 各自的样本夹具。
///
/// ⚠ `set_var` 在 Rust 2024 是 `unsafe`（要求没有并发读者）：写发生在 `OnceLock`
/// 的初始化里，任何读都发生在本函数返回之后（与 `track_instrument_switch.rs` 同款）。
fn fake_library() -> &'static Path {
    static LIBRARY: OnceLock<PathBuf> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("yeban-211-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // ---- ① 慢：128 份**真的** WAV，每份 0.5 s。解码它们需要可测量的时间。
        let slow_sfz = dir.join(SLOW_SFZ);
        let slow_dir = slow_sfz.parent().expect("SFZ 有父目录");
        std::fs::create_dir_all(slow_dir).expect("建慢夹具目录");
        let mut slow_text = String::new();
        for index in 0..SLOW_SAMPLES {
            let name = format!("s{index:03}.wav");
            write_plain_wav(
                slow_dir.join(&name),
                &PcmFormat::integer(1, 48_000, 16),
                &PcmBuffer::Int16(vec![0_i16; SLOW_FRAMES_PER_SAMPLE]),
            )
            .expect("写慢夹具 WAV");
            use std::fmt::Write as _;
            writeln!(
                slow_text,
                "<region> sample={name} key={index} pitch_keycenter={index}"
            )
            .expect("写 SFZ 文本");
        }
        std::fs::write(slow_sfz, slow_text).expect("写慢夹具 SFZ");

        // ---- ② 缺样本：3 个**不存在**的文件 ⇒ 探针必须逐字说"缺样本（3 个文件）"。
        let missing_sfz = dir.join(MISSING_SFZ);
        std::fs::create_dir_all(missing_sfz.parent().expect("父目录")).expect("建缺样本目录");
        let mut missing_text = String::new();
        for index in 0..MISSING_FILES {
            use std::fmt::Write as _;
            writeln!(
                missing_text,
                "<region> sample=absent-{index}.wav key={} pitch_keycenter={}",
                60 + index,
                60 + index
            )
            .expect("写缺样本 SFZ 文本");
        }
        std::fs::write(missing_sfz, missing_text).expect("写缺样本 SFZ");

        // ---- ③ 读不出来：**非法 UTF-8** ⇒ `read_to_string` 失败 ⇒ UnreadableSfz。
        let unreadable_sfz = dir.join(UNREADABLE_SFZ);
        std::fs::create_dir_all(unreadable_sfz.parent().expect("父目录")).expect("建坏 SFZ 目录");
        std::fs::write(unreadable_sfz, [0xff_u8, 0xfe, 0x00, 0x80]).expect("写坏 SFZ 字节");

        // SAFETY: 见函数文档 —— 这次写发生在任何读之前（`OnceLock` 的 happens-before）。
        unsafe { std::env::set_var(SAMPLE_LIBRARY_ENV, &dir) };
        dir
    })
}

/// 一条带撤销端口的 Tab 3 活窗口（接线走**生产同一个** `host::wire_device_rack`）。
fn live_window_with_undo(
    project: &YebanProjectV1,
    tag: &str,
) -> (LivePort<MainWindow>, Rc<UndoPort>, InstrumentProbeState) {
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
    let probe = host::wire_device_rack(live.ui(), &port);
    (live, port, probe)
}

/// 运行时控件树（`[UI-DoD §3.6]` 的唯一载体）。
fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

/// 清单里的显示名 ⇒ 按钮下标（按钮行与清单**同序**）。
fn index_of(name: &str) -> i32 {
    let names = library_instrument_names();
    let index = names
        .iter()
        .position(|candidate| candidate == name)
        .unwrap_or_else(|| panic!("清单里必须有 `{name}`，实际 {names:?}"));
    i32::try_from(index).expect("下标在 i32 内")
}

/// 点一下 + **确定性**等后台探针跑完 + 在 UI 线程上排空。
fn click_and_settle(ui: &MainWindow, probe: &InstrumentProbeState, port: &UndoPort, index: i32) {
    ui.invoke_instrument_install(index);
    assert!(
        host::wait_for_pending_instrument_probe(probe),
        "点第 {index} 条必须起一个探针工作线程"
    );
    assert!(
        host::apply_pending_instrument_probe(ui, probe, Some(port)),
        "探针跑完之后必须有一个结果可排空（第 {index} 条）"
    );
}

/// 切成一条非主总线的展示轨（`display_track` 的选中态）。
fn select_first_track(ui: &MainWindow, project: &YebanProjectV1) -> yeban_model::EntityId {
    let track = project
        .tracks
        .values()
        .find(|track| track.id != project.master_bus_track_id)
        .expect("演示工程必须有一条非主总线轨");
    ui.set_selected_track_id(track.id.to_canonical_string().into());
    track.id
}

// ===========================================================================
// ① 不卡：点击回调立刻返回，探针在别的线程上跑
// ===========================================================================
#[test]
fn clicking_a_slow_instrument_returns_before_the_probe_finishes() {
    let library = fake_library();
    let project = yeban_app::bridge::demo_project();
    let (live, port, probe) = live_window_with_undo(&project, "#211 不卡");
    let track_id = select_first_track(live.ui(), &project);
    let index = index_of(SLOW_NAME);

    let before = port.display();
    let started = Instant::now();
    live.ui().invoke_instrument_install(index);
    let elapsed = started.elapsed();

    // ---- ⭐ 判据 1：UI 线程上的这一跳必须短（慢乐器在真库里是 253,973 ms）。
    assert!(
        elapsed < Duration::from_millis(100),
        "点击回调在 UI 线程上耗时 {elapsed:?}，必须 < 100 ms（探针必须已在别的线程上）"
    );
    // ---- ⭐ 判据 2：点击后**立刻**可观察的中间态（⛔ 不是"没反应"）。
    let immediate = live.ui().get_instrument_install_status().to_string();
    assert!(
        immediate.starts_with("检查中…：") && immediate.contains(SLOW_NAME),
        "点击后立刻可见的状态必须是 `检查中…：<名字>`，实际 {immediate:?}"
    );
    // ---- ⭐ 判据 3：此刻工程**一位没改**（提交只发生在排空那一跳）。
    assert_eq!(
        port.display().commit_count,
        before.commit_count,
        "点击返回时提交数必须还没变（探针在飞）"
    );
    assert_eq!(
        port.project().tracks.get(&track_id).expect("轨在").sampler,
        None,
        "点击返回时 `sampler` 必须还是 None（还没提交）"
    );
    // ---- ⭐ 判据 4：再点一次会**立刻**得到"上一条还在检查"的回执（⛔ 不并发、不卡）。
    assert!(probe.is_in_flight(), "点击返回时探针必须还在飞");
    live.ui().invoke_instrument_install(index);
    assert_eq!(
        live.ui().get_instrument_install_status().to_string(),
        yeban_app::bridge::INSTRUMENT_INSTALL_BUSY_STATUS,
        "第二次点击必须立刻给出逐字回执（⛔ 不排队、⛔ 不开第二个解码线程）"
    );

    // ---- 排空：后台结果回到 UI 线程，**这一次**才提交。
    assert!(
        host::wait_for_pending_instrument_probe(&probe),
        "必须有一个工作线程可 join"
    );
    assert!(
        host::apply_pending_instrument_probe(live.ui(), &probe, Some(&port)),
        "探针跑完之后必须有一个结果可排空"
    );
    let after = port.display();
    assert_eq!(
        after.commit_count,
        before.commit_count + 1,
        "装一份乐器仍必须**恰好一次** `commit_ops`"
    );
    assert_eq!(
        after.undoable,
        before.undoable + 1,
        "装一份乐器仍必须**恰好一步**可撤销"
    );
    assert_eq!(
        port.project().tracks.get(&track_id).expect("轨在").sampler,
        Some(yeban_model::SamplerBinding {
            path: format!("assets/samples/{SLOW_SFZ}")
        }),
        "读回的 `sampler.path` 必须逐字是 manifest 路径"
    );
    assert_eq!(
        live.ui().get_instrument_install_status().to_string(),
        format!("已装上：{SLOW_NAME}"),
        "排空后的终态回执"
    );

    // ---- 撤销仍对：一步回到 `None`（后台只改提交时机，不改撤销树形状）。
    assert!(
        port.perform(UiAction::Undo).changed(),
        "装乐器必须真的进了撤销栈"
    );
    assert_eq!(
        port.project().tracks.get(&track_id).expect("轨在").sampler,
        None,
        "撤销后必须回到装之前的值"
    );

    report_line(&format!(
        "[#211 不卡] 点击回调墙钟 = {:?}（预算 < 100 ms）；立刻可见状态 = {immediate:?}；\
         点击时 commit={}；排空后 commit={} undoable={}；撤销后 sampler=None；库根={}",
        elapsed,
        before.commit_count,
        after.commit_count,
        after.undoable,
        library.display()
    ));
}

// ===========================================================================
// ② 拒绝有原因：缺样本 / SFZ 读不出来 —— 逐字进控件树，⛔ 不再"没反应"
// ===========================================================================
#[test]
fn a_refused_install_says_why_in_the_control_tree() {
    let _library = fake_library();
    let project = yeban_app::bridge::demo_project();
    let (live, port, probe) = live_window_with_undo(&project, "#211 拒绝");
    let track_id = select_first_track(live.ui(), &project);

    // ---- 缺样本：3 个文件。
    let before = port.display();
    click_and_settle(live.ui(), &probe, &port, index_of(MISSING_NAME));
    let missing_expected = format!(
        "装不了：{MISSING_NAME} 缺样本（{MISSING_FILES} 个文件）—— 点右侧“下载采样库”补齐后重试"
    );
    assert_eq!(
        live.ui().get_instrument_install_status().to_string(),
        missing_expected,
        "缺样本的逐字回执"
    );
    let tree = runtime_tree(live.ui());
    assert_eq!(
        tree.find_by_id("instrument-install-status")
            .expect("回执必须真的在控件树里（⛔ 只进 stderr）")
            .label,
        missing_expected,
        "控件树里的 `label` 必须逐字等于界面文案"
    );
    assert_eq!(
        port.display().commit_count,
        before.commit_count,
        "拒绝 ⇒ Δ0 提交"
    );
    assert_eq!(port.display().undoable, before.undoable, "拒绝 ⇒ Δ0 撤销步");
    assert_eq!(
        port.project().tracks.get(&track_id).expect("轨在").sampler,
        None,
        "拒绝之后这条轨仍然有声（sampler 仍 None）"
    );

    // ---- SFZ 读不出来：另一句逐字原因（⛔ 不与"缺样本"共用一句假话）。
    click_and_settle(live.ui(), &probe, &port, index_of(UNREADABLE_NAME));
    let unreadable_expected =
        format!("装不了：{UNREADABLE_NAME} 的 SFZ 读不出来（文件不在库里或不是合法 SFZ）");
    assert_eq!(
        live.ui().get_instrument_install_status().to_string(),
        unreadable_expected,
        "读不出来的逐字回执"
    );
    let tree = runtime_tree(live.ui());
    assert_eq!(
        tree.find_by_id("instrument-install-status")
            .expect("回执必须在控件树里")
            .label,
        unreadable_expected
    );
    assert_eq!(
        port.display().commit_count,
        before.commit_count,
        "两次拒绝都不得提交"
    );

    report_line(&format!(
        "[#211 拒绝] `{MISSING_NAME}` ⇒ {missing_expected:?}；\
         `{UNREADABLE_NAME}` ⇒ {unreadable_expected:?}；commit={}（Δ0）",
        port.display().commit_count
    ));
}

// ===========================================================================
// ③ 结构证据：探针在 `thread::spawn` 的闭包里；投影里没有探针
// ===========================================================================
#[test]
fn the_probe_lives_on_a_worker_thread_and_never_in_apply_view() {
    // 本文件所有判据都先初始化同一个假库根（`OnceLock` 的写读 happens-before），
    // 免得本进程里"设环境变量 / 读环境变量"并发（Rust 2024 的 `set_var` 是 unsafe）。
    let _library = fake_library();
    let source = include_str!("../src/host.rs").replace("\r\n", "\n");

    // ---- ③a：`on_instrument_install` 的闭包里真的有一个 `thread::spawn`，
    //          而且慢调用 `probe_instrument_samples` 在**那个** spawn 之后。
    let handler = source
        .find("ui.on_instrument_install({")
        .expect("host.rs 里必须有 on_instrument_install");
    let handler_block = &source[handler..];
    let spawn = handler_block
        .find("std::thread::spawn(move || {")
        .expect("装乐器的点击闭包里必须有 thread::spawn");
    let probe_call = handler_block
        .find("crate::bridge::probe_instrument_samples(&path)")
        .expect("慢探针必须被调用");
    assert!(
        probe_call > spawn,
        "`probe_instrument_samples` 必须出现在 `thread::spawn` **之后**（⛔ UI 线程）"
    );
    // 排空请求必须经 `upgrade_in_event_loop`（唯一跨线程的一跳）。
    assert!(
        handler_block.contains("upgrade_in_event_loop"),
        "工作线程必须请求 UI 线程排空（`upgrade_in_event_loop`）"
    );

    // ---- ③b：`apply_view`（投影 ⇒ 界面）里没有探针：`#210` 的裁决不动。
    let apply_start = source.find("pub fn apply_view(").expect("apply_view");
    let rest = &source[apply_start..];
    let open = rest.find('{').expect("函数体") + 1;
    let body = &rest[open..];
    let end = body.find("\n}\n").expect("函数体以行首 } 结束");
    let apply_view = &body[..end];
    for forbidden in ["probe_instrument_samples", "sfz_samples_present"] {
        assert!(
            !apply_view.contains(forbidden),
            "`apply_view` 里不得出现 `{forbidden}`（投影跑探针会冻结界面 —— `#210` 的裁决）"
        );
    }

    // ---- ③c：结构证据的行号（报告里逐字给出）。
    let handler_line = source[..handler].lines().count();
    let spawn_line = source[..handler + spawn].lines().count();
    let probe_line = source[..handler + probe_call].lines().count();
    report_line(&format!(
        "[#211 结构] host.rs: on_instrument_install 起于第 {handler_line} 行；\
         `std::thread::spawn` 第 {spawn_line} 行；`probe_instrument_samples` 第 {probe_line} 行；\
         apply_view 内 probe/sfz_samples_present 命中 0"
    ));
}

// ===========================================================================
// ④ CLI：一次看清"哪些能装"（逐条原因 ＋ 末行汇总）
// ===========================================================================
#[test]
fn the_cli_probe_instruments_readout_lists_ok_and_fail_with_reasons() {
    let library = fake_library();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_yeban-app"))
        .arg("--probe-instruments")
        .env(SAMPLE_LIBRARY_ENV, library)
        .output()
        .expect("真二进制必须能启动");
    assert_eq!(
        output.status.code(),
        Some(0),
        "`--probe-instruments` 必须退出 0；stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!("lib: {}", library.display())),
        "第一行必须声明读的是哪个库；实际 stdout=\n{stdout}"
    );
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.starts_with("instrument-ok:"))
            .count(),
        1,
        "3 份夹具里可装的必须恰好 1 条；实际 stdout=\n{stdout}"
    );
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.starts_with("instrument-fail:"))
            .count(),
        2,
        "3 份夹具里不可装的必须恰好 2 条；实际 stdout=\n{stdout}"
    );
    assert!(
        stdout.contains("缺样本（3 个文件）"),
        "缺样本那一条必须带**文件数**；实际 stdout=\n{stdout}"
    );
    assert!(
        stdout.contains("SFZ 读不出来"),
        "坏 SFZ 那一条必须与缺样本**分开说**；实际 stdout=\n{stdout}"
    );
    let summary = stdout
        .lines()
        .find(|line| line.starts_with("instrument-total:"))
        .expect("末行必须是 `instrument-total:`");
    assert_eq!(
        summary, "instrument-total: total=3 installable=1 missing=1 unreadable=1",
        "汇总行的逐字契约"
    );

    // ---- 独立命令：与别的开关组合 = 用法错误（⛔ 不静默丢掉别的开关）。
    let conflict = std::process::Command::new(env!("CARGO_BIN_EXE_yeban-app"))
        .args(["--probe-instruments", "--headless"])
        .env(SAMPLE_LIBRARY_ENV, library)
        .output()
        .expect("真二进制必须能启动");
    assert_eq!(
        conflict.status.code(),
        Some(2),
        "组合必须是用法错误（退出码 2）"
    );
    assert!(
        String::from_utf8_lossy(&conflict.stderr).contains("不能与"),
        "stderr 必须逐字说不能组合：{}",
        String::from_utf8_lossy(&conflict.stderr)
    );

    report_line(&format!(
        "[#211 CLI] `--probe-instruments` ok=1 fail=2；{summary}；组合退出码=2"
    ));
}

// ===========================================================================
// ⑤ 第 315 条：1920 基准帧**逐字节不动**（本单只加 Tab 3 的运行期回执）
// ===========================================================================
/// FNV-1a 64（只为报告里给一个紧凑的内容指纹；判定用的是**完整 PNG 字节**相等）。
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// ⭐ 任务单 `#315` 口径：**先算"会不会动 1920 基准帧"**。
///
/// 结论：**不动**。两条证据：
/// 1. 本单只新增 `instrument-install-status` 这个**运行期**属性，它的唯一读者是
///    控制台 **Tab 3** 的 `DeviceRack`；5 张 `golden/linux/*-1920x1080.png` 都是
///    **Tab 0**（编曲 / Session）⇒ 结构上不经过它；
/// 2. 本判据把这条路径**真的跑一遍**：在 1920×1080 的默认帧上，先把回执写成空串抓一帧，
///    再写成最坏情况的长拒绝文案抓一帧 ⇒ 两份 PNG **逐字节相同**（`assert_eq!`）。
///
/// ⚠ 它**不**替代 `tests/golden/linux/**` 的平台基准（本机是 macOS，那 5 张是 Linux
/// 托管 runner 的产物）。它证的是"新属性不可能改变默认帧的像素"这一条**因果**。
#[test]
fn the_1920_default_frame_is_byte_identical_with_and_without_the_status() {
    // 同文件其余判据共用一个假库根；先初始化它，避免并发 set_var/读 env。
    let _library = fake_library();
    let project = yeban_app::bridge::demo_project();
    let view = ViewState::from_project(&project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let port = LivePort::new(Size::new(1920, 1080), Permission::ReadOnly, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 1920×1080 默认窗口");
    host::apply_view(port.ui(), &view, 1920.0, 0.0);

    let empty_png = port.capture_png().expect("空回执那一帧");
    port.ui().set_instrument_install_status(
        "装不了：Big Rusty Drums 缺样本（4466 个文件）—— 点右侧“下载采样库”补齐后重试".into(),
    );
    let set_png = port.capture_png().expect("长回执那一帧");

    // 落盘两份产物，供报告里用 `shasum -a 256` 做体外复核（engine 目录 = target/）。
    let artifacts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-test-port");
    std::fs::create_dir_all(&artifacts).expect("建产物目录");
    let empty_path = artifacts.join("task-211-1920-status-empty.png");
    let set_path = artifacts.join("task-211-1920-status-set.png");
    std::fs::write(empty_path.as_path(), empty_png.as_slice()).expect("写空回执帧");
    std::fs::write(set_path.as_path(), set_png.as_slice()).expect("写长回执帧");

    assert_eq!(
        empty_png, set_png,
        "1920 默认帧（Tab 0）不得因为 `instrument-install-status` 而改变一个字节"
    );
    report_line(&format!(
        "[#315 1920] 默认帧 PNG 逐字节相同：len={} fnv1a={:#018x}；空回执=`{}` 长回执=`{}`",
        set_png.len(),
        fnv1a(&set_png),
        empty_path.display(),
        set_path.display()
    ));
}
