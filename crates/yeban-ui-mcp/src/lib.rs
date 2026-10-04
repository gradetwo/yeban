//! # yeban-ui-mcp — UI 测试 MCP 适配层
//!
//! 在 `yeban-ui-test-port` 之上暴露 JSON-RPC：语义控件树遍历、属性断言、
//! `dispatch_pointer_*` / `dispatch_key_press` 事件注入、Framebuffer 截图。
//! 三级权限 ReadOnly（默认）/ Interactive / Administrative [UI-MCP-001]，
//! Release 默认关闭且只绑 `127.0.0.1`。
//!
//! 规范来源 (Normative): `docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md` §12.3
//!
//! 状态: **scaffold（Phase 0 Spike 5 之前为占位骨架，尚无实现）**。
//!
#![deny(missing_docs)]
