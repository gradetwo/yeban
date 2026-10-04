//! # Spike: 内嵌 MCP 与 `.yeban.lock` 互斥
//!
//! 规范 ID: `ROAD-M0-007` (docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md §3 Phase 0)
//!
//! ## 方法 (Method)
//!
//! 内嵌 Streamable HTTP MCP（本地 Token）+ 独立 stdio CLI 同时抢占同一工程文件。
//!
//! ## 通过判据 (Pass criterion)
//!
//! 外部 Agent 可驱动运行中的 DAW 并触发 UI 刷新；并发打开得到 `PROJECT_LOCKED`。
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
