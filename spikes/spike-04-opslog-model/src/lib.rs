//! # Spike: ULID + BTreeMap 可逆操作日志
//!
//! 规范 ID: `ROAD-M0-004` (docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md §3 Phase 0)
//!
//! ## 方法 (Method)
//!
//! 用 `EntityId`(ULID) 与 `BTreeMap` 实现可逆 `Op` 序列，跑 10,000 次撤销/重做。
//!
//! ## 通过判据 (Pass criterion)
//!
//! 状态 100% 还原；单步 < 0.2ms；序列化字节序完全一致。
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
