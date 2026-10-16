//! 判据：**GUI 路径引擎装配链**在 Windows 主线程默认栈（≈ 1 MiB）下必须能启动。
//!
//! ## 这一条在防什么
//!
//! Windows 主线程默认栈 ≈ **1 MiB**（PE 头 `StackReserve`），而 macOS / Linux ≈ 8 MiB
//! ⇒ "本机绿"会掩盖一类**只在 Windows 红**的栈溢出（台账 R724）。`--export-wav` 那一面
//! 已由 `#72` 修掉（`Box<SynthEngine>`）；**GUI 那一面更深**：`run_gui` → `host::ProductionLoop::start`
//! → `EngineHost::reload`／`open_device` → `engine_host::fresh_runtime` → `EngineRuntime::new`
//! 是**多层按值交接同一个** `EngineRuntime`（⭐ `#107` 后为 `180144 B`；`#107` 前是 `180000 B`），在**进入设备腿之前**就已经
//! 越过 1 MiB（R725 逐帧读数：`start` `543360 B`／`reload` `744528 B`／`fresh_runtime`
//! `360992 B`）⇒ Windows 上**开界面即崩**。
//!
//! ## 被钉住的两条链（都在**子进程**里量）
//!
//! | # | 链 | 覆盖的帧 |
//! | :- | :--- | :--- |
//! | ① | **pre-device** | `ProductionLoop::start`（＋ `mark_applied` ＋ `stop`，与 `src/main.rs` 同序） |
//! | ② | **device-leg entry** | ① ＋ `ProductionLoop::open_device`（`RequireExclusive` ⇒ **无设备也真跑**，见 `device.rs` 的协商第一行） |
//!
//! ⚠ ② 用 `ShareMode::RequireExclusive` 是**确定性失败注入**：它**不需要**任何声卡，
//! 在任何机器上都真的进入 `EngineHost::open_device` → `fresh_runtime` → `device::open_output`
//! （后者的栈帧在**序言**里就分配），并在协商第一行返回 `ExclusiveModeUnsupported`
//! ⇒ 帧用量被测到、真实设备一位不碰。⛔ 因此本文件不登记任何需要真人 + 真声卡的判据。
//!
//! ⚠ 为什么必须走**子进程**：栈溢出是 `fatal runtime error: stack overflow` ＋ `abort`
//! ⇒ ⛔ 在判据进程内测不出"崩"那一侧。做法与 `#72` 的 `ulimit -s` 手法同源，只是把
//! "压系统栈限额"换成"显式 `Builder::stack_size`"，读数因此精确到 KiB（口径同 R725 第 234 条）。
//!
//! ## 为什么止于"进入设备腿"（如实边界）
//!
//! 真开窗需要显示器，真开设备需要声卡 ⇒ 本机无头环境跑不到那两步。R725 指出越限之处
//! **正是**"构造完运行时 ＋ 进入设备腿之前"那一步 ⇒ 本文件的 ① 覆盖它，② 再把设备腿
//! 的进入帧一起量进来。⛔ 本文件**不**声明"界面画出来了"或"设备真的开了"。
//!
//! ## 本机读数（`-C opt-level=0`）
//!
//! | 读法 | 修复前 | 修复后 |
//! | :--- | ---: | ---: |
//! | ① pre-device 最小可跑栈（KiB） | `1952`（@1024 红） | `368`（@1024 绿） |
//! | ② device-leg entry 最小可跑栈（KiB） | `1952`（@1024 红） | `528`（@1024 绿） |
//!
//! ⚠ ② 修复后的峰值由三项决定（逐帧见报告）：`EngineHost::open_device` 的帧里**先**留了
//! `open_output` 的 `180144 B` 按值实参位（⭐ `#107` 后；引擎侧 API，⛔ 本单不改），而**同一个帧**里
//! 又要嵌套 `fresh_runtime` → `new_boxed` → `write_into`（`176384 B`）→ `SynthEngine::new`
//! （`129952 B`）那段构造 ⇒ `180144 + 176384 + 129952 ≈ 486 KiB` ＋ 包装帧。
//!
//! ⛔ 本文件**不测**引擎内部结构，只测"这段装配链要多少栈"。

use std::process::{Command, Stdio};

use yeban_app::bridge::demo_project;
use yeban_app::engine_host::{EditMark, EngineHostError};
use yeban_app::host::ProductionLoop;
use yeban_engine::device::{EngineConfig, ShareMode};
use yeban_model::project::YebanProjectV1;

/// 探针开关：值 = 要跑的链（`chain` / `device`）。父进程不设它 ⇒ 探针入口是空判据。
const PROBE_MODE: &str = "YB_APP_GUI_STACK_PROBE";
/// 探针线程的栈大小（KiB）。
const PROBE_KIB: &str = "YB_APP_GUI_STACK_KIB";
/// 探针入口的测试名（父进程用 `--exact` 只跑它）。
const PROBE_TEST: &str = "gui_stack_probe_child";
/// Windows 主线程默认栈（PE 头 `StackReserve`，≈ 1 MiB）—— 本文件的目标平台档位。
const WINDOWS_MAIN_STACK_KIB: usize = 1024;
/// 二分的上界：4 MiB（必须够；不够说明夹具坏了，⛔ 不是栈的问题）。
const PROBE_MAX_KIB: usize = 4096;
/// 二分的下界（KiB）。
const PROBE_MIN_KIB: usize = 16;
/// 二分的粒度（KiB）。
const PROBE_STEP_KIB: usize = 16;

/// 要量的两条链。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Chain {
    /// ① 进入设备腿**之前**的 GUI 装配（`run_gui` 的引擎那一段）。
    PreDevice,
    /// ② 再把 `open_device` 的进入帧一起算上（失败注入，不碰真设备）。
    DeviceLegEntry,
}

impl Chain {
    /// 探针协议里的名字。
    const fn probe_value(self) -> &'static str {
        match self {
            Self::PreDevice => "chain",
            Self::DeviceLegEntry => "device",
        }
    }

    /// 探针协议里的名字 → 链。
    fn from_value(value: &str) -> Self {
        match value {
            "chain" => Self::PreDevice,
            "device" => Self::DeviceLegEntry,
            other => panic!("未知探针模式 {other}"),
        }
    }
}

/// ① `run_gui` 的引擎那一段：`ProductionLoop::start` ＋ `mark_applied` ＋ `stop`。
///
/// 顺序与 `src/main.rs` 逐字一致：`start` 之后先 `mark_applied`（记下开局标记），
/// 再 `stop`（让这一代停在 `Stopped`）。`#[inline(never)]` 为了让它自己占一帧、
/// 便于在反汇编里指认。
#[inline(never)]
fn pre_device_chain(project: &YebanProjectV1) {
    let production = ProductionLoop::start(project, 0).expect("引擎重建必须成功");
    if production.has_engine() {
        production
            .engine_handle()
            .borrow_mut()
            .mark_applied(EditMark::new(None, 0));
        production.engine_handle().borrow_mut().stop();
    }
}

/// ② ① ＋ `open_device` 的进入（`RequireExclusive` ⇒ **无设备也真跑**，协商第一行返回 Err）。
#[inline(never)]
fn device_leg_entry_chain(project: &YebanProjectV1) {
    let mut production = ProductionLoop::start(project, 0).expect("引擎重建必须成功");
    let config = EngineConfig {
        share_mode: ShareMode::RequireExclusive,
        ..EngineConfig::default()
    };
    let error = production
        .open_device(config)
        .expect_err("RequireExclusive 必须明确失败（cpal 0.18 没有独占 API）");
    assert!(
        matches!(error, EngineHostError::Device(_)),
        "必须是设备错误（优雅失败），实际 {error:?}"
    );
}

/// 跑一条链（在调用者**当前线程**上）。
fn run_chain(chain: Chain, project: &YebanProjectV1) {
    match chain {
        Chain::PreDevice => pre_device_chain(project),
        Chain::DeviceLegEntry => device_leg_entry_chain(project),
    }
}

/// **本单的硬要求判据**：两条链都必须落在 Windows 主线程默认栈（1 MiB）之内。
///
/// 它给出三个读数：① 1 MiB 那一档的**红/绿**；② 二分出的**最小可跑栈**；③ 与目标档位的余量。
#[test]
fn gui_path_fits_in_a_one_mib_main_thread_stack() {
    if std::env::var(PROBE_MODE).is_ok() {
        // 探针子进程里 `--exact` 只跑 `gui_stack_probe_child`；本判据不会被选中。
        return;
    }
    let mut readings = Vec::new();
    for chain in [Chain::PreDevice, Chain::DeviceLegEntry] {
        let at_target = probe(chain, WINDOWS_MAIN_STACK_KIB);
        let threshold = threshold_kib(chain);
        println!(
            "[gui-stack] {:?}: @{WINDOWS_MAIN_STACK_KIB} KiB = {} ; 最小可跑栈 = {threshold} KiB ; \
             余量 = {} KiB",
            chain,
            reading(at_target),
            WINDOWS_MAIN_STACK_KIB.saturating_sub(threshold)
        );
        readings.push((chain, at_target, threshold));
    }
    for (chain, at_target, threshold) in readings {
        assert!(
            at_target,
            "{chain:?} 必须在 Windows 主线程默认栈（{WINDOWS_MAIN_STACK_KIB} KiB）下跑完"
        );
        assert!(
            threshold <= WINDOWS_MAIN_STACK_KIB,
            "{chain:?} 的最小可跑栈（{threshold} KiB）必须落在目标平台默认栈之内"
        );
    }
}

/// 探针入口：**只**在父进程设了 [`PROBE_MODE`] 时干活（否则父进程里它是一个空判据）。
///
/// 它在 `kib` KiB 的线程上跑**整条**链 ⇒ 这个线程的栈需求就是那条链的需求。
#[test]
fn gui_stack_probe_child() {
    let Ok(mode) = std::env::var(PROBE_MODE) else {
        return;
    };
    let kib: usize = std::env::var(PROBE_KIB)
        .expect("探针必须给出栈大小")
        .parse()
        .expect("栈大小必须是整数 KiB");
    let chain = Chain::from_value(&mode);
    // 夹具在**大栈**上建好（它是输入，不是被测对象）。
    let project = demo_project();
    let handle = std::thread::Builder::new()
        .stack_size(kib * 1024)
        .spawn(move || run_chain(chain, &project))
        .expect("小栈线程必须能启动");
    handle.join().expect("小栈线程里的装配链不许 panic");
}

/// 二分出"整条链能跑完"的**最小**栈（KiB，[`PROBE_STEP_KIB`] 粒度）。
fn threshold_kib(chain: Chain) -> usize {
    let (mut lo, mut hi) = (PROBE_MIN_KIB, PROBE_MAX_KIB);
    assert!(
        probe(chain, hi),
        "{chain:?} 在 {hi} KiB 下都跑不完 ⇒ 夹具/采样有问题，⛔ 不是栈阈值"
    );
    while hi - lo > PROBE_STEP_KIB {
        let mid = (lo + hi) / 2 / PROBE_STEP_KIB * PROBE_STEP_KIB;
        if probe(chain, mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

/// 在**子进程**里用 `kib` KiB 的栈跑一次探针；返回它是否跑完。
///
/// ⚠ 栈溢出会 abort 整个进程 ⇒ 只能这么测；`status.success() == false` 既涵盖
/// 段错误也涵盖判据失败，两者对"这个栈够不够"是同一个答案。
/// ⚠ 子进程的 stdout/stderr 一律**丢掉**：否则十几次探针的输出会混进判据日志
/// （"按测试二进制分组对账"要的是干净的 `test result:` 行）。
fn probe(chain: Chain, kib: usize) -> bool {
    let exe = std::env::current_exe().expect("current_exe 必须可读");
    let status = Command::new(exe)
        .args(["--exact", PROBE_TEST])
        .env(PROBE_MODE, chain.probe_value())
        .env(PROBE_KIB, kib.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("探针子进程必须能启动");
    status.success()
}

/// 把"跑完了没有"翻译成人读的读数。
const fn reading(ok: bool) -> &'static str {
    if ok {
        "绿（整条链跑完）"
    } else {
        "红（栈溢出/abort）"
    }
}
