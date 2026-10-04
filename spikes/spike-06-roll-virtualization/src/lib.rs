//! # Spike: 10 万音符虚拟化钢琴卷帘
//!
//! 规范 ID: `ROAD-M0-006` (docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md §3 Phase 0)
//!
//! ## 方法 (Method)
//!
//! R-Tree 视口裁剪 + 批量绘制，验证 100,000 音符下的滚动/缩放帧率与内存稳定性。
//!
//! ## 通过判据 (Pass criterion)
//!
//! 稳定 120 FPS (帧耗时 ≤ 8.3ms)；无内存泄漏。
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
