//! # yeban-app — Slint 桌面主程序
//!
//! Slint 声明式 GUI 宿主 + 后台调度。UI 文件清单以 UI/UX 规范 §8 为准（11 个 `.slint`，
//! 见 `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md` 的裁决）。
//!
//! 关键约束:
//! - UI 线程与实时音频线程**物理隔离**，UI 负载绝不阻塞音频回调 [UI-GRID-004]。
//! - 必须支持 `SLINT_BACKEND=headless` 无头启动，供 CI 做控件树断言与截图回归 [UI-TEST-003]。
//! - 自动化寻址只允许语义 Element ID（`track-{i}-fader` / `note-{ulid}-rect` / `clip-{ulid}-header`
//!   / `tab-{name}-button`），严禁绝对像素坐标 [UI-TEST-001]。
//!
//! 规范来源 (Normative):
//! - `docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md` §1 … §12
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §1/§8
//!
//! 状态: **scaffold（Phase 3 之前为占位骨架，尚无实现）**。
//!
#![deny(missing_docs)]
