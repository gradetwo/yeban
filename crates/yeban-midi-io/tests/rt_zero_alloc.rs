//! ⭐ **零分配窗口**：⭐ `midir` 回调体的内核在**构造之后**的推送路径上
//! **⛔ 零分配／⛔ 零释放**（[`ARCH-RT-001`]，⭐ 红线 7）。
//!
//! # 为什么这个判据在 `tests/` 而不在 `src/`
//!
//! ⚠ ⭐ 本 crate 的 `src/lib.rs` 有 **`#![forbid(unsafe_code)]`**，而
//! `#[global_allocator]` 的实现**必须**写 `unsafe impl GlobalAlloc` ⇒
//! ⭐ `forbid` **不允许**在 crate 内用 `allow` 破例（那是 `forbid` 的定义）
//! ⇒ ⭐ 计数型全局分配器只能放在**另一个编译单元**（集成测试目标）里。
//! ⭐ 这与 `yeban-engine/tests/rt_zero_alloc.rs` 的处置同源。
//!
//! ⚠ ⭐ **按线程武装**：⭐ 测试框架自己会分配 ⇒ ⭐ 只在**武装窗口内**、且只在
//! **本线程**上计数（⭐ `thread_local!` ＋ `const` 初始化 ⇒ ⛔ 计数本身不分配）。
//!
//! [`ARCH-RT-001`]: ../../docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md
//!
//! # ⚠ ⭐ 为什么本探针只统计**两项**（⭐ 不是漏测）
//!
//! ⭐ 引擎侧的零分配探针报**四元组**（⭐ `allocations`／`deallocations`／`lock_blocking`／`io_ops`）。
//! ⚠ ⭐ **本 crate 的推送路径结构上不含互斥锁与 I/O 调用** —— ⭐ 它只有一次
//! `rtrb::Producer::push` ＋ ⭐ 一次 `saturating_add`（⭐ 见 `MidiEventSender::push` 与
//! `MidiEventSender::handle_message`）⇒ ⭐ **那两个计数器在这里没有对应对象**。
//! ⭐ 故本探针只统计**分配/释放**两项：⭐ **这不是漏测，⭐ 而是被测对象里没有那两类操作**。
//! ⛔ 本文件**刻意不**放一个恒为 0 的 `lock_blocking` 计数器 —— ⭐ 没有真的 instrument
//! 锁的话，⭐ 那就是**假证据**。
//!
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use yeban_midi_io::{MidiEvent, event_channel};

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
    static DEALLOCS: Cell<u64> = const { Cell::new(0) };
}

/// ⭐ 计数型分配器：⭐ 只把统计转发给 [`System`]（⛔ 不做任何指针运算）。
struct CountingAllocator;

// SAFETY: 本实现**不改变** `System` 的任何行为 —— 它只在窗口内先自增计数，
// 然后把**原样的** `layout` 交给 `System`，并原样返回它的指针/执行它的释放。
// ⛔ 不缓存、⛔ 不重排、⛔ 不改对齐。
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.with(Cell::get) {
            ALLOCS.with(|count| count.set(count.get().saturating_add(1)));
        }
        // SAFETY: `layout` 由调用方按 `GlobalAlloc` 契约给出，本实现**原样**转交
        // `System.alloc`（⛔ 不修改、⛔ 不缓存），并把它的返回值**原样**透传。
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ARMED.with(Cell::get) {
            DEALLOCS.with(|count| count.set(count.get().saturating_add(1)));
        }
        // SAFETY: `(ptr, layout)` 由调用方按 `GlobalAlloc` 契约给出（⭐ 必是
        // `System.alloc` 先前返回的那一对），本实现**原样**转交 `System.dealloc`。
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// ⭐ 窗口内实测的 `(allocations, deallocations)`。
fn measure_window<F: FnOnce() -> u64>(body: F) -> (u64, u64, u64) {
    // ⭐ 先把计数清零**再**武装（⭐ 清零动作本身在窗口之外）。
    ALLOCS.with(|c| c.set(0));
    DEALLOCS.with(|c| c.set(0));
    ARMED.with(|c| c.set(true));
    let produced = body();
    ARMED.with(|c| c.set(false));
    (ALLOCS.with(Cell::get), DEALLOCS.with(Cell::get), produced)
}

/// 判据 ①：⭐ **模拟回调体 10_000 次 ⇒ 窗口内零分配、零释放**。
#[test]
fn the_callback_body_never_allocates_inside_the_window() {
    // ⚠ 构造（环的分配）**必须在窗口之外** —— 这正是"构造期预分配"的含义。
    let (mut sender, mut receiver) = event_channel(1024);
    let window = 10_000u64;

    let (allocs, deallocs, produced) = measure_window(|| {
        for step in 0..window {
            // ⭐ 三字节 NoteOn，⭐ 时间戳全用参数传入（⛔ 不用 `Instant::now()`）。
            sender.handle_message(1_000_000 + step * 62, &[0x90, 60, 100]);
        }
        window
    });

    assert_eq!(
        allocs, 0,
        "推送路径分配了 {allocs} 次（红线 7）（⭐ 红色原因提示：推送路径只有一次 rtrb 推送 ＋ 一次 saturating_add ⇒ 不含锁与 I/O ⇒ 本探针只统计分配/释放两项，⭐ 见模块文档）"
    );
    assert_eq!(
        deallocs, 0,
        "推送路径释放了 {deallocs} 次（红线 7）（⭐ 红色原因提示：推送路径只有一次 rtrb 推送 ＋ 一次 saturating_add ⇒ 不含锁与 I/O ⇒ 本探针只统计分配/释放两项，⭐ 见模块文档）"
    );
    assert_eq!(produced, window);
    // ⭐ 反向对照：⭐ 窗口**真的跑了**（⭐ 环里有事件、⭐ 读者能读到完整事件）。
    assert!(receiver.pending() > 0, "窗口必须真的推进了环");
    let first = receiver.pop().expect("至少一条完整事件");
    assert_eq!(first.data1, 60);
}

/// 判据 ②：⭐ **环满时**的丢弃路径也**零分配**（⭐ 且**立刻返回**，⭐ ⛔ 不阻塞）。
#[test]
fn the_full_ring_drop_path_never_allocates() {
    let (mut sender, _receiver) = event_channel(4);
    let event = MidiEvent {
        timestamp_us: 7,
        status: 0x90,
        data1: 1,
        data2: 2,
    };
    // ⚠ 先把环填满（**窗口之外**）。
    for _ in 0..4 {
        assert!(sender.push(event));
    }
    let (allocs, deallocs, dropped_before) = measure_window(|| {
        // ⭐ 这 100 次全部走"环满 ⇒ 计数丢弃"的分支。
        for _ in 0..100 {
            assert!(!sender.push(event), "环满 ⇒ 必须返回 false（⛔ 不阻塞）");
        }
        sender.dropped_events()
    });
    assert_eq!(allocs, 0, "丢弃路径分配了 {allocs} 次");
    assert_eq!(deallocs, 0, "丢弃路径释放了 {deallocs} 次");
    // ⭐ **只数失败的那 100 次**：⭐ 前 4 次是**被接受**的推送，⛔ 不是丢弃
    // （⭐ 我第一版写 104 是**我自己算错**；⭐ 这条断言当场把它抓住了）。
    assert_eq!(
        dropped_before, 100,
        "容量 4 ⇒ 满环之后那 100 次按事件计丢弃"
    );
}
