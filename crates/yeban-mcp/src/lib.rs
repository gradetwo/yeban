//! # yeban-mcp — 领域 MCP / Intent API v2
//!
//! 两个形态共享同一套工具实现：
//! - **库形态**：内嵌进 `yeban-app` 进程，供 AI Agent 在运行中的 DAW 上工作。
//! - **二进制形态**：`yeban-mcp` stdio 批处理，冷启动 ≤ 20ms，持 `.yeban.lock` 独占。
//!
//! 安全红线 [ARCH-SEC-002, MUST-GATE-009]: 网络监听**默认关闭**（`mcp-http` feature 且显式
//! `--enable-mcp`）；只允许绑定 `127.0.0.1`（端口 0 动态分配）；256-bit Bearer Token 落在
//! `~/.yeban/session.token` 且权限 0600；生产构建硬封 `ui:inject`。
//!
//! 规范来源 (Normative):
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §7
//! - `schemas/mcp-tools.schema.json`（10 个 `yeban_*` 工具的权威定义）
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` ROAD-M4-001 … ROAD-M4-003
//!
//! 状态: **scaffold（Phase 4 之前为占位骨架，尚无实现）**。
//!
#![deny(missing_docs)]
