//! T-FIX-44 判据：`read_project_container` 装载大资产时的**峰值内存**。
//!
//! 旧实现借用迭代归档条目 + 逐条 `clone()`，装载期间归档与结果同时持有
//! 全部资产字节，峰值约为载荷的 2 倍。修复后按值消费归档、逐条移动，
//! 每份字节只有一个所有者。本判据用一个计数型全局分配器实测装载窗口内
//! 的存活字节峰值：载荷 16 MiB 时，峰值增量必须 < 1.5 × 载荷（旧实现
//! ≥ 2 × 载荷，判据能把两种实现分开）。
//!
//! ⚠ 本文件**只有一个测试**：计数器是进程全局状态，同一二进制内并行
//! 的其他测试会污染读数。单测独占二进制是这条判据成立的前提。

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use yeban_model::container::{ContainerLimits, read_project_container, write_project_container};
use yeban_model::{AssetHash, YebanProjectV1};

/// 当前存活字节数（alloc 累加、dealloc 累减）。
static LIVE: AtomicUsize = AtomicUsize::new(0);
/// 本次测量窗口内的存活字节峰值。
static PEAK: AtomicUsize = AtomicUsize::new(0);

struct TrackingAllocator;

// SAFETY: 本实现只把 `GlobalAlloc` 的每个方法原样转发给系统分配器，
// 记账发生在分配成功之后 / 释放之前，不改变指针与布局的配对关系，
// 因此系统分配器的安全契约被完整保持。
unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: 直接转发系统分配器，仅在成功后记账。
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let now = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: ptr/layout 与 alloc 的返回原样配对，转发系统分配器释放。
        unsafe { System.dealloc(ptr, layout) };
    }
}

#[global_allocator]
static GLOBAL: TrackingAllocator = TrackingAllocator;

/// 载荷大小：16 MiB 的单个音频资产（资产是容器载荷的主体）。
const ASSET_LEN: usize = 16 * 1024 * 1024;

#[test]
fn loading_a_large_asset_does_not_hold_two_copies_at_peak() {
    // 确定性伪随机字节（splitmix64），与 container_roundtrip 同一手法。
    let mut state = 0x1234_5678_9ABC_DEF0u64;
    let mut asset = Vec::with_capacity(ASSET_LEN);
    while asset.len() < ASSET_LEN {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        asset.extend_from_slice(&(z ^ (z >> 31)).to_le_bytes());
    }
    asset.truncate(ASSET_LEN);

    let project = YebanProjectV1::default();
    let mut assets = BTreeMap::new();
    assets.insert(AssetHash::of_bytes(&asset), asset);
    let bytes = write_project_container(&project, b"load-memory-dag", &assets).expect("写出容器");
    // 资产原件随 map 交给写侧后仍在 `bytes` 之外存活一份（测试夹具自身），
    // 基线在它之后量取 —— 它不属于装载窗口的增量。
    drop(assets);

    // 测量窗口：基线 = 容器字节已在内存、尚未装载时的存活量；
    // 窗口内只发生 read_project_container 一次装载。
    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let loaded = read_project_container(&bytes, &ContainerLimits::default()).expect("装载必须成功");
    let peak_delta = PEAK.load(Ordering::Relaxed) - baseline;

    // 功能等价：装载结果与写入逐字节相同。
    assert_eq!(loaded.assets.len(), 1, "必须装载出唯一的资产");
    assert_eq!(loaded.assets[0].1.len(), ASSET_LEN);
    assert_eq!(
        AssetHash::of_bytes(&loaded.assets[0].1),
        loaded.assets[0].0,
        "资产内容必须与写入一致（哈希自洽）"
    );
    assert_eq!(loaded.history_dag, b"load-memory-dag");
    assert_eq!(loaded.project, project);

    // 内存判据：峰值增量 < 1.5 × 载荷。修复后增量 ≈ 载荷本体（条目字节
    // 移动进结果）+ 工程 AST 与零碎开销；旧实现还要再加一整份 clone，
    // 增量 ≥ 2 × 载荷，必然越线。
    assert!(
        peak_delta < ASSET_LEN + ASSET_LEN / 2,
        "装载峰值增量 {peak_delta} 字节 ≥ 1.5 × 载荷 {ASSET_LEN}：\
         归档与结果仍在同时持有资产副本（T-FIX-44 回归）"
    );
}
