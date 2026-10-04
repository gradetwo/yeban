//! # yeban-ui-test-port — 无头 UI 测试端口
//!
//! Tier-3 自研兜底层：不依赖 Slint 内部 crate，基于 `slint::platform::Platform` +
//! `SoftwareRenderer` 在内存 Framebuffer 上光栅化，产出 PNG 供视觉回归
//! [ARCH-SLINT-001, MUST-GATE-015]。**Golden 图绝不允许由 `i-slint-backend-testing` 产出**
//! （它不渲染像素，只做属性断言）。
//!
//! 规范来源 (Normative):
//! - `docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md` §12
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §0.4
//!
//! 状态: **scaffold（Phase 0 Spike 5 之前为占位骨架，尚无实现）**。
//!
#![deny(missing_docs)]
