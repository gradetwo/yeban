//! 判据：`EngineRuntime` 的**装箱构造**（[`EngineRuntime::new_boxed`]，任务单 `#73`）。
//!
//! ## 这一条在防什么
//!
//! Windows 主线程默认栈 ≈ **1 MiB**（PE 头 `StackReserve`），而 macOS / Linux ≈ 8 MiB
//! ⇒ "本机绿"会掩盖一类**只在新平台红**的栈溢出。`EngineRuntime` 的值尺寸是
//! `180144 B`（⭐ 任务单 `#107` 的 CC 表 ＋144 B；本文件也有这条读数），而 `EngineRuntime::new` **按值返回** ⇒ 每个调用点
//! 都要在自己的栈帧里留一份 180144 B 的返回值位；`yeban-app` 的 GUI 那条链
//! （`host::ProductionLoop::start` → `EngineHost::reload`／`open_device` →
//! `engine_host::fresh_runtime` → `EngineRuntime::new`）在**打开设备之前**就已越过
//! 1 MiB ⇒ 开界面即崩（台账 R725 的逐帧读数）。
//!
//! ## 三条判据
//!
//! | # | 判据 | 口径 |
//! | :- | :--- | :--- |
//! | ① | **1 MiB 栈直构** | `std::thread::Builder::stack_size(1 << 20)` 上构造 ＋ 渲染 4 个量子 ⇒ 必须成功 |
//! | ② | **崩溃阈值（二分）** | 两条构造路径各在**子进程**里二分"最小可构造栈"⇒ `new_boxed` 必须**严格低于** `new`，且 ≤ 1024 KiB |
//! | ③ | **等价性** | 同一夹具、同一量子数 ⇒ `new_boxed` 与按值构造的左右声道**逐位相同**（指纹相等） |
//!
//! ⚠ 为什么 ② 必须走子进程：栈溢出是 `abort`（`fatal runtime error: stack overflow`）
//! ⇒ ⛔ 在判据进程内测不出"崩"那一侧。做法与 `#72` 的 `ulimit -s` 手法同源，只是把
//! "压系统栈限额"换成"显式 `stack_size`"，读数因此精确到 KiB。
//!
//! ⚠ 两条路在判据里是**对称**的：各有一只同形的 `fresh_runtime_*`（对应
//! `engine_host::fresh_runtime`）与一只 `render_*`，唯一差别是"按值还是装箱"。
//! ⛔ 不给出"按值那条多背一份载体"的不对称夹具 —— 那会把阈值差读成假的。
//!
//! ## 本机读数（`-C opt-level=0`，判据二进制实测）
//!
//! | 读法 | `new`（按值） | `new_boxed`（装箱） |
//! | :--- | ---: | ---: |
//! | 自身栈帧（`objdump` 读序言 `sub sp`） | **`224 B`**（改动前 `359312 B`） | **`304 B`** |
//! | 构造期峰值（＋ `write_into` `176416 B` ＋ 器件帧 `129984 B`） | ≈ `306928 B` | ≈ `306704 B` |
//! | **调用者**还要为返回值位留一份 | `180144 B` | ⛔ **0**（只有一个 `Box` 指针） |
//! | 最小可构造栈（本文件 ② 二分） | **`512 KiB`** | **`336 KiB`** |
//!
//! ⇒ ⭐ 装箱把"构造 ＋ 一个量子"的栈阈值压下去 **`176 KiB`**（＝那 180144 B 的返回值位所在的
//! 16 KiB 档；⭐ 任务单 `#107` 的 ＋144 B **没有跨档** ⇒ 本读数不变）。
//! ⚠ 本夹具是**单层**按值交接 ⇒ `new` 在 1 MiB 下**不崩**（阈值 512 KiB）；产物里
//! 真正越过 1 MiB 的是 `yeban-app` 那条**多层**按值交接的 GUI 链（台账 R725 实测逐帧：
//! `start` `543360 B`／`reload` `744528 B`／`fresh_runtime` `360992 B`）——
//! 那半边是**下一单**（app 侧装箱）的事，本单只交引擎侧那一半。
//!
//! ⚠ **修订（任务单 `#173`）**：`EngineRuntime` 的值尺寸现在是 **181 936 B**
//! （`#107` 的 180144 ＋ 主总线插入槽位/计数 **＋1792**）⇒ 本文档表格里的
//! `180144` 是 `#73`/`#107` 当时的实测读数（⛔ 不删）；**阈值差（176 KiB）未重测**，
//! 本文件的可执行判据里没有钉那个差值（只钉 `new_boxed < new` 与 ≤ 1024 KiB）。
//!
//! ⛔ 本文件**不测** `new_boxed` 与 `new` 的内部结构，只测它们的**栈用量**与**输出**。

mod support;

use std::process::{Command, Stdio};
use std::sync::Arc;

use support::{Fixture, NoteSpec};
use yeban_engine::meter::{MeterPublisher, meter_channel};
use yeban_engine::ring::{EventReceiver, event_channel};
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{EngineSnapshot, RetireProducer, SnapshotSlot, retire_channel};
use yeban_model::YebanProjectV1;

/// 子进程探针的模式开关（父进程不设它 ⇒ 探针入口在父进程里是一个空判据）。
const PROBE_MODE: &str = "YB_ENGINE_CTOR_STACK_PROBE";
/// 探针要用的栈大小（KiB）。
const PROBE_KIB: &str = "YB_ENGINE_CTOR_STACK_KIB";
/// 探针入口的测试名（父进程用 `--exact` 只跑它）。
const PROBE_TEST: &str = "ctor_stack_probe_child";
/// Windows 主线程默认栈（PE 头 `StackReserve`，≈ 1 MiB）—— 本文件的目标平台档位。
const WINDOWS_MAIN_STACK_KIB: usize = 1024;
/// 二分的上界：4 MiB。它必须**够**（不够说明夹具坏了，而不是栈的问题）。
const PROBE_MAX_KIB: usize = 4096;
/// 二分的粒度（KiB）。
const PROBE_STEP_KIB: usize = 16;

/// 两条构造路径。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ctor {
    /// 既有 API：**按值返回** ⇒ 调用点要留 `180144 B` 的返回值位。
    ByValue,
    /// 本单新增：**直接在堆上**构造 ⇒ 栈上只有一个 `Box` 指针。
    Boxed,
}

impl Ctor {
    /// 探针协议里的名字。
    const fn probe_value(self) -> &'static str {
        match self {
            Self::ByValue => "value",
            Self::Boxed => "boxed",
        }
    }
}

/// 与 `yeban-app` 的 `engine_host::fresh_runtime` **同形**：按值交出一份运行时。
#[inline(never)]
fn fresh_runtime_value(
    slot: &Arc<SnapshotSlot>,
    retire: RetireProducer,
    events: EventReceiver,
    meters: MeterPublisher,
) -> EngineRuntime {
    EngineRuntime::new(slot, retire, events, meters)
}

/// 同形的一只，但**装箱**交出（本单新增的那条路）。
#[inline(never)]
fn fresh_runtime_boxed(
    slot: &Arc<SnapshotSlot>,
    retire: RetireProducer,
    events: EventReceiver,
    meters: MeterPublisher,
) -> Box<EngineRuntime> {
    EngineRuntime::new_boxed(slot, retire, events, meters)
}

/// 按值那条路：构造 ＋ 渲染，并在这只函数的**帧里**把值管到底（⛔ 不让它逃出去）。
#[inline(never)]
fn render_by_value(
    slot: &Arc<SnapshotSlot>,
    retire: RetireProducer,
    events: EventReceiver,
    meters: MeterPublisher,
    quanta: usize,
) -> support::Render {
    let mut runtime = fresh_runtime_value(slot, retire, events, meters);
    render_quanta(&mut runtime, quanta)
}

/// 装箱那条路（与 [`render_by_value`] 逐字同形，只差构造入口）。
#[inline(never)]
fn render_boxed(
    slot: &Arc<SnapshotSlot>,
    retire: RetireProducer,
    events: EventReceiver,
    meters: MeterPublisher,
    quanta: usize,
) -> support::Render {
    let mut runtime = fresh_runtime_boxed(slot, retire, events, meters);
    render_quanta(&mut runtime, quanta)
}

/// 推 `quanta` 个量子并收下读数（两只 `render_*` 共用；只接引用 ⇒ 栈上不搬大值）。
fn render_quanta(runtime: &mut EngineRuntime, quanta: usize) -> support::Render {
    let mut left = Vec::with_capacity(quanta * 128);
    let mut right = Vec::with_capacity(quanta * 128);
    let mut out = vec![0.0f32; 128 * 2];
    for _ in 0..quanta {
        out.fill(0.0);
        runtime.process_quantum(&mut out, 2);
        for frame in 0..128 {
            left.push(out[frame * 2]);
            right.push(out[frame * 2 + 1]);
        }
    }
    support::Render {
        left,
        right,
        stats: runtime.stats(),
        revision: runtime.revision(),
    }
}

/// 走**产品路径**渲染 `quanta` 个量子（与 `support::render` 同口径，只把构造入口参数化）。
#[must_use]
fn render_via(ctor: Ctor, project: &YebanProjectV1, quanta: usize) -> support::Render {
    let snapshot = EngineSnapshot::from_project(project, 1).expect("夹具必须能编译成快照");
    let slot = SnapshotSlot::new(snapshot);
    let (retire, _queue) = retire_channel(64);
    let (_sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(4096);
    match ctor {
        Ctor::ByValue => render_by_value(&slot, retire, receiver, publisher, quanta),
        Ctor::Boxed => render_boxed(&slot, retire, receiver, publisher, quanta),
    }
}

/// 判据夹具：主总线 ＋ 一条 MIDI 轨 ＋ 三个**不重叠**的音符。
///
/// 三个音符是为了让"逐位相同"这条判据真的覆盖合成器与电平：全零输出也能指纹相等。
fn fixture() -> Fixture {
    support::note_project(&[
        NoteSpec::at(0, 960, 60, 100),
        NoteSpec::at(960, 960, 64, 90),
        NoteSpec::at(1920, 480, 67, 80),
    ])
}

/// ① Windows 主线程默认栈（1 MiB）下，`new_boxed` 必须能构造**并**渲染。
///
/// ⚠ 这条判据就是"把 Windows 才崩变成本机可判"：`stack_size(1 << 20)` 直接给出
/// 目标平台的那一档，不需要 `ulimit`。
#[test]
fn new_boxed_constructs_on_a_one_mib_stack() {
    let fixture = fixture();
    let quanta = 4;
    let handle = std::thread::Builder::new()
        .stack_size(WINDOWS_MAIN_STACK_KIB * 1024)
        .spawn(move || {
            let render = render_via(Ctor::Boxed, &fixture.project, quanta);
            (render.fingerprint(), render.nonzero(), render.stats.quanta)
        })
        .expect("1 MiB 线程必须能启动");
    let (fingerprint, nonzero, quanta_done) = handle
        .join()
        .expect("1 MiB 栈上构造 new_boxed 不许崩（那正是本单要修的东西）");
    println!(
        "[rt-boxed/1mib] new_boxed @{WINDOWS_MAIN_STACK_KIB} KiB: ok  \
         fingerprint={fingerprint:#018x} nonzero={nonzero} quanta={quanta_done}"
    );
    assert_eq!(quanta_done, quanta as u64, "四个量子必须真的跑完");
    assert!(nonzero > 0, "夹具必须真的出声（否则指纹相等没有牙齿）");
}

/// ② 崩溃阈值（子进程二分）＋ 1 MiB 那一档的直读。
#[test]
fn boxed_constructor_lowers_the_crash_threshold() {
    if std::env::var(PROBE_MODE).is_ok() {
        // 探针子进程里 `--exact` 只跑 `ctor_stack_probe_child`；本判据不会被选中。
        return;
    }
    let by_value = threshold_kib(Ctor::ByValue);
    let boxed = threshold_kib(Ctor::Boxed);
    let by_value_at_default = probe(Ctor::ByValue, WINDOWS_MAIN_STACK_KIB);
    let boxed_at_default = probe(Ctor::Boxed, WINDOWS_MAIN_STACK_KIB);
    println!(
        "[rt-boxed/stack] 最小可构造栈（构造 ＋ 一个量子）：new={by_value} KiB / \
         new_boxed={boxed} KiB ⇒ 下降 {} KiB",
        by_value.saturating_sub(boxed)
    );
    println!(
        "[rt-boxed/stack] @{WINDOWS_MAIN_STACK_KIB} KiB（Windows 主线程默认）: \
         new={} / new_boxed={}",
        reading(by_value_at_default),
        reading(boxed_at_default)
    );
    assert!(
        boxed_at_default,
        "new_boxed 必须能在 Windows 主线程默认栈（{WINDOWS_MAIN_STACK_KIB} KiB）下构造"
    );
    assert!(
        boxed < by_value,
        "装箱构造必须把栈阈值压下来：new={by_value} KiB, new_boxed={boxed} KiB"
    );
    assert!(
        boxed <= WINDOWS_MAIN_STACK_KIB,
        "new_boxed 的阈值（{boxed} KiB）必须落在目标平台默认栈之内"
    );
}

/// ③ 等价性（逐位）＋ 尺寸照打。
#[test]
fn boxed_constructor_is_bit_identical_to_by_value() {
    let fixture = fixture();
    let quanta = 64;
    let by_value = render_via(Ctor::ByValue, &fixture.project, quanta);
    let boxed = render_via(Ctor::Boxed, &fixture.project, quanta);
    println!(
        "[rt-boxed/equiv] size_of EngineRuntime={} SynthEngine={} quanta={quanta} \
         new={:#018x} new_boxed={:#018x}",
        core::mem::size_of::<EngineRuntime>(),
        core::mem::size_of::<yeban_engine::synth::SynthEngine>(),
        by_value.fingerprint(),
        boxed.fingerprint(),
    );
    assert_eq!(
        core::mem::size_of::<EngineRuntime>(),
        186_736,
        "装箱构造不该改布局：`EngineRuntime` 的现读尺寸是 186736 B\
         （`#107` 的 CC 表 ＋144 → 180144；`#173` 的主总线插入槽位 ＋1792 → 181936；\
         `#261` 的逐边发送量表/平滑器/重放副本 ＋4784 → 186720；\
         ⭐ **S1 第一片**的两个 `u64` 事件计数 ＋16 → 186736；\
         门槛仍是 192 KiB = 196608）"
    );
    assert_eq!(
        boxed.fingerprint(),
        by_value.fingerprint(),
        "两条构造路径的左右声道必须逐位相同"
    );
    assert_eq!(boxed.left_bits(), by_value.left_bits());
    assert_eq!(boxed.frames(), by_value.frames());
    assert_eq!(boxed.nonzero(), by_value.nonzero());
    assert_eq!(boxed.revision, by_value.revision);
    assert_eq!(boxed.stats.quanta, by_value.stats.quanta);
    assert_eq!(boxed.stats.meter_frames, by_value.stats.meter_frames);
    assert_eq!(
        boxed.stats.insert_convolution_rejects,
        by_value.stats.insert_convolution_rejects
    );
    assert!(boxed.nonzero() > 0, "夹具必须真的出声");
}

/// 探针入口：**只**在父进程设了 [`PROBE_MODE`] 时干活（否则父进程里它是一个空判据）。
///
/// 它把"夹具 ＋ 通道"在**大栈**上建好，再在 `kib` KiB 的线程上跑
/// **构造 ＋ 一个量子** —— 于是这个线程的栈需求就是那两件事的需求。
#[test]
fn ctor_stack_probe_child() {
    let Ok(mode) = std::env::var(PROBE_MODE) else {
        return;
    };
    let kib: usize = std::env::var(PROBE_KIB)
        .expect("探针必须给出栈大小")
        .parse()
        .expect("栈大小必须是整数 KiB");
    let ctor = match mode.as_str() {
        "value" => Ctor::ByValue,
        "boxed" => Ctor::Boxed,
        other => panic!("未知探针模式 {other}"),
    };
    let fixture = fixture();
    let snapshot = EngineSnapshot::from_project(&fixture.project, 1).expect("夹具必须能编译成快照");
    let slot = SnapshotSlot::new(snapshot);
    let (retire, queue) = retire_channel(64);
    let (_sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(4096);
    let handle = std::thread::Builder::new()
        .stack_size(kib * 1024)
        .spawn(move || {
            let render = match ctor {
                Ctor::ByValue => render_by_value(&slot, retire, receiver, publisher, 1),
                Ctor::Boxed => render_boxed(&slot, retire, receiver, publisher, 1),
            };
            render.stats.quanta
        })
        .expect("小栈线程必须能启动");
    let quanta = handle.join().expect("小栈线程里的构造不许 panic");
    assert_eq!(quanta, 1, "一个量子必须真的跑完");
    drop(queue);
    println!("[rt-boxed/probe] {mode} @{kib} KiB: ok");
}

/// 二分出"构造 ＋ 一个量子"能跑完的**最小**栈（KiB，[`PROBE_STEP_KIB`] 粒度）。
fn threshold_kib(ctor: Ctor) -> usize {
    let (mut lo, mut hi) = (PROBE_STEP_KIB, PROBE_MAX_KIB);
    assert!(
        probe(ctor, hi),
        "{ctor:?} 在 {hi} KiB 下都跑不完 ⇒ 夹具/采样有问题，⛔ 不是栈阈值"
    );
    while hi - lo > PROBE_STEP_KIB {
        let mid = (lo + hi) / 2 / PROBE_STEP_KIB * PROBE_STEP_KIB;
        if probe(ctor, mid) {
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
/// ⚠ 子进程的 stdout/stderr 一律**丢掉**：否则十六次探针的输出会混进判据日志
/// （"按测试二进制分组对账"要的是干净的 `test result:` 行）。
fn probe(ctor: Ctor, kib: usize) -> bool {
    let exe = std::env::current_exe().expect("current_exe 必须可读");
    let status = Command::new(exe)
        .args(["--exact", PROBE_TEST])
        .env(PROBE_MODE, ctor.probe_value())
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
        "绿（构造＋一个量子跑完）"
    } else {
        "红（栈溢出/abort）"
    }
}
