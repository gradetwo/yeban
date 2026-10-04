//! # Spike: 无锁 SPSC 与快照退役回收队列
//!
//! 规范 ID: `ROAD-M0-002` (docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md §3 Phase 0)
//!
//! ## 方法 (Method)
//!
//! 用 `rtrb` 的 bulk API 打通 UI/模型线程 → 音频线程的事件通道，并验证 `Arc<EngineSnapshot>`
//! 退役队列在高频拓扑交换下零阻塞、零分配。
//!
//! ## 通过判据 (Pass criterion)
//!
//! 100,000 事件/秒，零死锁、零 malloc、单事件 < 0.05ms、零丢弃。
//!
//! ## 纪律
//!
//! 本 crate 是**一次性可行性验证**，不是产品代码：它必须能在 CI 上自动跑完并给出
//! 机器可读的判据。验证结论写入 `docs/ledger/`，结论成立后其结论被汲取进正式的
//! `crates/yeban-*`，spike 本身可以按需退役 (docs/DEV_WORKFLOW.md「明确废弃」)。
//!
//! 状态: **未实现**。
//!
#![deny(missing_docs)]
