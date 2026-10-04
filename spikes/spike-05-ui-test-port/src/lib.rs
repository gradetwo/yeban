//! # Spike: 自研无头 UI 测试端口 (Tier-3 兜底)
//!
//! 规范 ID: `ROAD-M0-005` (docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md §3 Phase 0)
//!
//! ## 方法 (Method)
//!
//! 基于 `slint::platform::Platform` + `SoftwareRenderer` 自建 Framebuffer 捕获与 HTTP JSON-RPC 端口。
//!
//! ## 通过判据 (Pass criterion)
//!
//! 无显示器环境可启动；JSON-RPC 往返 ≤ 15ms；截图导出 ≤ 50ms。
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
