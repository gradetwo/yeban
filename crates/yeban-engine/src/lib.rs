//! # yeban-engine — 实时音频引擎
//!
//! 本 crate 是实时音频权威：cpal 流宿主、`rtrb` 无锁 SPSC、`Arc<EngineSnapshot>`
//! 原子交换与退役回收队列、内部 PDC 延迟补偿、FTZ/DAZ 浮点环境
//! [ARCH-RT-001..005, ARCH-PDC-001..002, ROAD-M2-001..004, ROAD-M2-007..008]。
//!
//! 本 crate **不属于** `#![forbid(unsafe_code)]` 名单（名单为 model/theory/dsp/render）：
//! FTZ/DAZ 设置与快照原子指针的引用计数操作需要 `unsafe`，但每一处都必须附
//! `// SAFETY:` 证明 [AGENTS.md §2 红线 8]。实时回调内禁止任何分配/释放/锁等待/阻塞 I/O
//! [AGENTS.md §2 红线 7]。
//!
//! ## 模块地图
//!
//! | 模块 | 内容 | 规范 ID |
//! | :--- | :--- | :--- |
//! | [`block`] | 固定块长 `AudioBlock` 与栈上 `[f32; N]` 缓冲约定 | [ARCH-RT-001]、[ARCH-DET-001]、[ROAD-M2-007] |
//! | [`fpu`] | FTZ / DAZ 浮点环境开关（x86 MXCSR / aarch64 FPCR） | [ARCH-RT-003]、[ROAD-M2-003] |
//! | [`graph`] | `RoutingGraph` 拓扑排序、关键路径 `L_max`、`D_i` 分配、环形延迟线 | [ARCH-PDC-001]、[ROAD-M2-004] |
//! | [`level`] | **再导出** `yeban_dsp::meter` 的电平口径（峰值/峰值保持/RMS/平滑/dBFS/钳位/取最新/真峰值）——本 crate 不再持有实现 | [ARCH-UI-002]、[ROAD-M2-008] |
//! | [`ring`] | UI/模型 → 音频线程的批量无锁 SPSC 事件通道 | [ARCH-RT-001]、[ROAD-M2-007] |
//! | [`snapshot`] | 不可变 `EngineSnapshot`、原子交换槽、退役回收队列 | [ARCH-RT-002]、[ROAD-M2-002] |
//! | [`meter`] | VU / 峰值电平独立高容量 SPSC、每节点电平状态机、UI 60Hz 抽干 | [ARCH-UI-002]、[ROAD-M2-008] |
//! | [`rt`] | 渲染量子驱动（`EngineRuntime`），**不依赖 cpal** | [ARCH-TOP-002]、[ARCH-RT-001] |
//! | `device` | cpal 宿主、配置协商、`NullBackend`（**feature `device`**） | [ARCH-TOP-002]、[ROAD-M2-001] |
//!
//! ## Cargo features：设备 I/O 与 PDC 算法必须能分开消费
//!
//! [docs/adr/ADR-0001 D19] 裁决：PDC 算法（[`graph`]）**不得**引用 cpal，
//! 这样纯离线渲染器 `yeban-render` 可以 `default-features = false` 依赖本 crate，
//! 拿到同一份拓扑排序 + 关键路径 + 环形延迟线，而不被拖入整条声卡驱动栈。
//!
//! | feature | 默认 | 作用 |
//! | :--- | :---: | :--- |
//! | `device` | ✅ | 编译 `cpal` 与 `device` 模块（声卡宿主、配置协商、`NullBackend`） |
//!
//! 关掉 `device` 后仍然可用的公共面：`block` / `fpu` / `graph` / `ring` / `snapshot` /
//! `meter` / `rt` —— 也就是说"PDC 算法 + 快照交换 + SPSC + 渲染量子驱动"全部可用，
//! 只是没有声卡。
//!
//! **电平口径不在这条 feature 切分的两侧**：它已经上移到零重依赖的 `yeban-dsp`
//! （`yeban_dsp::meter`），本 crate 的 [`level`] 只剩 `pub use`。因此无论 `device`
//! 开或关，电平读数都是同一份实现；混音台/母带/导出也可以直接复用，不必拖入 cpal。
//!
//! ## 线程拓扑（[ARCH-TOP-002]）
//!
//! ```text
//!  [Model / UI 线程]                        [cpal 实时回调线程]
//!        │  publish(snapshot)                     │
//!        ├──────────────► SnapshotSlot.ptr ───────┤ load(Acquire) + Arc 克隆
//!        │                                       │
//!        │  EventSender ──(rtrb, 批量)──────────► EventReceiver
//!        │                                       │
//!        │                                       ├──► MeterPublisher ──► MeterCollector
//!        │                                       │        (rtrb 高容量)
//!        ◄── RetireQueue.drain() ◄───────────────┘ push(旧 Arc<EngineSnapshot>)
//!            (60Hz, 主线程 Drop)
//! ```
//!
//! ## 设计边界（本切片**没有**证明的东西）
//!
//! 1. **声部合成尚未接入**：`EngineRuntime::process_quantum` 目前只做
//!    "参数/事件出队 → 快照切换 → 渲染（占位静音）→ 逐轨/母线电平计量与发布"，
//!    真正的乐器/效果渲染留给 `yeban-sfz` / `yeban-dsp` 的后续切片。
//!    因此本 crate 现在**不能**发声：电平计算本身是真实的（口径见 [`level`]），
//!    但端到端喂进去的是占位静音 ⇒ 发布出来的电平恒为静音。
//! 2. **实时线程优先级**（[ROAD-M2-001]）未实现，理由见 `device` 模块文档与
//!    `docs/ledger/engine-rt-notes.md` §4：cpal 0.18 的 `realtime` feature 只覆盖
//!    WASAPI / AAudio / PipeWire / JACK，macOS 与 Linux-ALSA 路径没有开关，
//!    自行写 `pthread_setschedparam` 需要新的 `libc` 依赖（属于依赖图裁决）。
//! 3. **独占模式**（WASAPI Exclusive）cpal 0.18 无 API；`ShareMode::RequireExclusive`
//!    会返回明确错误而不是假装成功。
//! 4. **采样格式**只支持 `f32`（协商失败会返回 `device::DeviceError`），
//!    `i16`/`u16` 的 `FromSample` 转换路径留给后续切片。
//! 5. **PDC 的节点延迟来源**：规范 [ARCH-PDC-001] 要求
//!    `DeviceDefinition::latency_samples`，而 `yeban-model` 当前**没有**该字段，
//!    因此延迟目前由 [`graph::LatencyTable`] 显式提供（`from_tracks` 返回全零）。
//!    这是**规范缺口**，已登记在 notes 的 needs 清单。
//!
//! 规范来源 (Normative):
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3（`ARCH-RT-001..005`、
//!   `ARCH-PDC-001..002`、`ARCH-TOP-002`、`ARCH-DET-001`）
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` `ROAD-M2-001..004`、`ROAD-M2-007..008`
//! - `docs/ledger/engine-rt-notes.md`（API 核验出处与 TODO 清单）
#![deny(missing_docs)]
// 红线 8 [AGENTS.md §2] 只对 model/theory/dsp/render 强制 forbid(unsafe_code)。
// 本 crate 需要 FTZ/DAZ 的 MXCSR/FPCR 写入与快照指针的引用计数操作, 因此**不能**
// 加 forbid —— 但 guard G03 的名单里也没有它, 见 scripts/guards/policy_check.py。

pub mod block;
#[cfg(feature = "device")]
pub mod device;
pub mod fpu;
pub mod graph;
pub mod level;
pub mod meter;
pub mod ring;
pub mod rt;
pub mod snapshot;

/// 本 crate 实现的规范需求 ID（规格 → 测试映射的单一事实源，测试里逐条引用）。
///
/// 与 `docs/ledger/engine-rt-notes.md` §3 的判据表一一对应。
pub const IMPLEMENTED_SPEC_IDS: &[&str] = &[
    "ARCH-RT-001",  // 零分配/零锁/零阻塞 I/O + rtrb 批量 API
    "ARCH-RT-002",  // EngineSnapshot 原子交换 + 退役回收队列
    "ARCH-RT-003",  // FTZ/DAZ
    "ARCH-PDC-001", // 延迟上报与关键路径对齐
    "ARCH-PDC-002", // 环形延迟线（时延预算的补偿实现）
    "ARCH-TOP-002", // 线程模型与通信隔离
    "ARCH-UI-002",  // 电平独立 SPSC + 真峰值/RMS 计量 + UI 取最新
    "ARCH-DET-001", // L1：固定 128 采样块长
    "ROAD-M2-001",  // 音频调度核心（宿主部分）
    "ROAD-M2-002",  // 双缓冲快照原子交换
    "ROAD-M2-003",  // FTZ/DAZ 强制统一
    "ROAD-M2-004",  // 内部 PDC 总架构
    "ROAD-M2-007",  // 批量无锁环形队列
    "ROAD-M2-008",  // VU/峰值电平独立 SPSC + 60Hz 抽干
];
