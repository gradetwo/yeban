//! # Spike: Tier-1 软件光栅化无头截图
//!
//! 规范 ID: `ROAD-M0-008` (docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md §3 Phase 0)
//!
//! ## 方法 (Method)
//!
//! 验证 `i-slint-backend-testing` 为内部 crate、不渲染像素 (只能属性断言)，并把
//! `SoftwareRenderer` + Framebuffer 确立为视觉回归唯一合法出图路径。
//!
//! ## 通过判据 (Pass criterion)
//!
//! 无头内存中产出 1920x1080 像素级一致 PNG，且非零尺寸、非全黑 (MUST-GATE-015)。
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
