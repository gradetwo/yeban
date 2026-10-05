//! 走带推进的**运行期**零分配断言（[MUST-GATE-001] / 红线 7）。
//!
//! 与 [`rt_zero_alloc`](rt_zero_alloc.rs) 同款计数型全局分配器，但窗口里跑的是
//! **走带**：`Play` / `Stop` / `Pause` / `SeekTicks` 命令经无锁 SPSC 出队、在量子边界
//! 应用，位置按整数有理数推进，读数发布到原子镜面。
//!
//! 逐条覆盖走带**新增**的那几段代码：
//! 1. `EngineEvent::Transport` 的出队分支（命令匹配 + `Transport::apply`）；
//! 2. `Transport::advance_frames` 的整数带余除法（`u128` 中间量）；
//! 3. `synth.seek` 的声部释放路径（`SeekTicks`；`seek` 只写定长数组，不许分配）；
//! 4. 停住分支（`track_scratch.fill(0.0)` + 跳过硬件的合成）；
//! 5. `Transport::publish`（seqlock 原子写）。
//!
//! # 这条判据怎么变红（可由注入验证）
//!
//! - 在 `advance_frames` 里把整数推进换成 `format!`/`Vec` 记录轨迹 ⇒ `allocations != 0`；
//! - 把 `TransportMirror` 换成 `Mutex<Transport>`  ⇒ `Mutex` 本身不分配，但
//!   `read()` 返回 `String`/`Vec` 之类的拥有型读数就会分配（本判据会红）；
//! - 在 `render_block` 的停住分支里 `Vec::with_capacity` ⇒ 红。
//!
//! # 为什么 `harness = false`
//!
//! 理由与 `rt_zero_alloc.rs` 完全相同（该文件模块文档有实测记录）：`#[global_allocator]`
//! 每个二进制只能定义一次，而 libtest 自己会在别的线程里分配 ⇒ 必须关掉 libtest，
//! 让进程里只有主线程跑测量，数字才无歧义。`cargo test --all-targets` 仍会构建并运行它，
//! 以**退出码**判定。

use std::alloc::{GlobalAlloc, Layout, System};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_engine::meter::meter_channel;
use yeban_engine::ring::{EngineEvent, TransportCommand, event_channel};
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};
use yeban_engine::transport::TransportState;

/// 包住 [`System`] 的计数型分配器。
struct CountingAllocator;

/// `true` 时才开始计数 —— 只在被测窗口内打开。
static ARMED: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static DEALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: 每个方法都只是"计数 + 原样转发给 `System`"，不改变指针/布局语义，
// 也不持有任何跨调用状态（计数器是原子量）。`unsafe` 块的边界就是 `System` 的调用本身。
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: 由调用方保证 `layout` 合法（`GlobalAlloc` 的契约）。
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ARMED.load(Ordering::Relaxed) {
            DEALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: 由调用方保证 `ptr` 来自本分配器且 `layout` 与分配时一致。
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: 由调用方保证 `ptr`/`layout` 合法且 `new_size > 0`。
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// 在"必须零分配"的窗口内执行 `body`，返回 `(allocations, deallocations)`。
fn measure<F: FnOnce()>(label: &str, body: F) -> (usize, usize) {
    ALLOCATIONS.store(0, Ordering::SeqCst);
    DEALLOCATIONS.store(0, Ordering::SeqCst);
    ARMED.store(true, Ordering::SeqCst);
    body();
    ARMED.store(false, Ordering::SeqCst);
    let allocations = ALLOCATIONS.load(Ordering::SeqCst);
    let deallocations = DEALLOCATIONS.load(Ordering::SeqCst);
    println!(
        "[MUST-GATE-001·transport] {label}: allocations={allocations} deallocations={deallocations}"
    );
    (allocations, deallocations)
}

fn main() -> ExitCode {
    let mut failures: Vec<String> = Vec::new();

    // 夹具与通道全部在"打开设备之前"建好（回调内不许分配 —— 产品契约）。
    let project = yeban_model::samples::filled_project();
    let snapshot = EngineSnapshot::from_project(&project, 1).expect("夹具工程必须能编译成快照");
    let slot = SnapshotSlot::new(snapshot);
    let (retire, mut queue) = retire_channel(64);
    let (mut sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(4096);
    let mut runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
    let mut output = vec![0.0f32; DEFAULT_BLOCK_FRAMES * 2];

    // 预热：一次性惰性路径（FPU 武装、首次武装快照）先跑完，否则会把初始化记到实时路径上。
    runtime.process_quantum(&mut output, 2);

    // ---- 场景 1：控制侧发命令（**允许分配**，不在窗口内）+ 实时侧应用 ----
    // 命令的生产端是控制线程，因此 publish 允许分配；被测窗口**只**包含 process_quantum。
    let mut playing_quanta = 0usize;
    for round in 0..200u64 {
        let command = match round % 4 {
            0 => TransportCommand::Stop,
            1 => TransportCommand::Play,
            2 => TransportCommand::SeekTicks(round * 37),
            _ => TransportCommand::Play,
        };
        let batch = [EngineEvent::Transport { command }];
        assert_eq!(sender.publish(&batch), 1, "命令必须被通道接受");

        if matches!(command, TransportCommand::Play) {
            playing_quanta += 1;
        }
        let (allocations, deallocations) = measure("命令应用 + 1 量子", || {
            runtime.process_quantum(&mut output, 2);
        });
        if allocations != 0 {
            failures.push(format!(
                "走带命令应用 + 渲染窗口内分配了 {allocations} 次（round={round}, {command:?}）"
            ));
        }
        if deallocations != 0 {
            failures.push(format!(
                "走带命令应用 + 渲染窗口内释放了 {deallocations} 次（round={round}, {command:?}）"
            ));
        }
    }
    if playing_quanta == 0 {
        failures.push("夹具里没有任何 Play —— 这条判据会变成'什么都没跑'的假绿".to_owned());
    }

    // ---- 场景 2：稳定播放下的 10,000 个量子（含每量子一次的走带读数发布）----
    let snapshot = EngineSnapshot::from_project(&project, 2).expect("等价快照");
    slot.publish(snapshot);
    runtime.process_quantum(&mut output, 2);
    queue.drain(64);
    let _ = sender.publish(&[EngineEvent::Transport {
        command: TransportCommand::Play,
    }]);
    runtime.process_quantum(&mut output, 2);

    let (allocations, deallocations) = measure("播放中的 10_000 个量子", || {
        for _ in 0..10_000 {
            runtime.process_quantum(&mut output, 2);
        }
    });
    if allocations != 0 {
        failures.push(format!(
            "播放窗口内发生了 {allocations} 次堆分配 —— 一票否决"
        ));
    }
    if deallocations != 0 {
        failures.push(format!(
            "播放窗口内发生了 {deallocations} 次堆释放 —— 一票否决"
        ));
    }

    // ---- 场景 3：停住时的量子（必须静音、冻结时钟，且同样零分配）----
    let _ = sender.publish(&[EngineEvent::Transport {
        command: TransportCommand::Stop,
    }]);
    runtime.process_quantum(&mut output, 2);
    let frozen = runtime.position_ticks();
    let (allocations, deallocations) = measure("停住时的 1_000 个量子", || {
        for _ in 0..1_000 {
            runtime.process_quantum(&mut output, 2);
        }
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "停住窗口内 alloc={allocations} dealloc={deallocations} —— 一票否决"
        ));
    }

    // ---- 覆盖度自检（防止"窗口里其实什么都没跑"的假绿）----
    let stats = runtime.stats();
    if stats.quanta < 11_000 {
        failures.push(format!("只处理了 {} 个量子，未达到压测规模", stats.quanta));
    }
    if stats.transport_commands < 200 {
        failures.push(format!(
            "只应用了 {} 条走带命令，命令路径可能根本没被覆盖",
            stats.transport_commands
        ));
    }
    if stats.transport_state != TransportState::Stopped {
        failures.push(format!(
            "场景 3 之后状态应为 Stopped，实际 {:?}",
            stats.transport_state
        ));
    }
    if runtime.position_ticks() != frozen {
        failures.push("停住窗口内位置发生了变化 —— 判据 ⑩ 的负向断言失败".to_owned());
    }
    if stats.transport_quanta == 0 {
        failures.push("没有任何推进量子 —— 走带推进路径未被覆盖".to_owned());
    }
    println!(
        "[MUST-GATE-001·transport] 汇总: quanta={} commands={} transport_quanta={} position_ticks={}",
        stats.quanta, stats.transport_commands, stats.transport_quanta, stats.position_ticks
    );

    // `Arc` 的一份克隆在窗口之外（控制侧读数），证明镜面读数本身不分配。
    let mirror = Arc::clone(runtime.transport_mirror());
    let (allocations, deallocations) = measure("镜面读数 ×1_000", || {
        for _ in 0..1_000 {
            let reading = mirror.read();
            std::hint::black_box(reading.position_ticks);
        }
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "读数窗口内 alloc={allocations} dealloc={deallocations}"
        ));
    }

    if failures.is_empty() {
        println!(
            "[MUST-GATE-001·transport] ok: 命令应用 200 轮 + 播放 10,000 量子 + 停住 1,000 量子 + 读数 1,000 次，全程零分配零释放"
        );
        ExitCode::SUCCESS
    } else {
        for failure in &failures {
            eprintln!("[MUST-GATE-001·transport] FAIL: {failure}");
        }
        ExitCode::FAILURE
    }
}
