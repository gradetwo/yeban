//! # Spike: 高频快照原子交换与退役队列压测
//!
//! 规范 ID: `ROAD-M0-009` (docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md §3 Phase 0)
//!
//! ## 方法 (Method)
//!
//! 每秒 10,000 次快照/拓扑变更，音频线程原子指针读取，主线程 60Hz 出队并负责 Drop。
//!
//! ## 通过判据 (Pass criterion)
//!
//! 音频线程零 dealloc、零爆音、队列不溢出、内存曲线平坦 (MUST-GATE-012)。
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
