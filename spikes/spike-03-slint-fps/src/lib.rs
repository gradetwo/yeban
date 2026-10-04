//! # Spike: Slint 最小宿主与 120 FPS 渲染管线
//!
//! 规范 ID: `ROAD-M0-003` (docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md §3 Phase 0)
//!
//! ## 方法 (Method)
//!
//! 搭建最小 Slint 窗口 + 走带光标动画，验证 FemtoVG/Skia/OpenGL 后端的帧率与内存。
//!
//! ## 通过判据 (Pass criterion)
//!
//! 稳定 120 FPS；常驻内存 < 25MB。
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
