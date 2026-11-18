//! 夜半 TUI（`yeban-tui`）：夜半的终端界面，与 Slint GUI 共用同一份工程模型、
//! 引擎宿主、快捷键策略表与解析器，只换呈现层（ratatui 平铺布局）。
//!
//! - 接缝路线：依赖 `yeban-app` 库，只调其无 Slint 窗口的部件
//!   （`EngineHost`、`input` 解析器、`cli::shortcut_rows` 策略表、
//!   `open`/`save`/`render_core`/`export_wav` 生产路径）。Slint 被链接但从不初始化。
//! - 键位口径：共享动作从 GUI（同一张表、同一个解析器），Vi 模态键只加在
//!   导航与模式层；完整映射表与冲突清单见 crate 根目录 `README.md`。
//! - 主题：真彩色取样自 GUI `ui/tokens.slint` 的 flat「渔火」主题，映射表见
//!   [`theme`] 模块文档。

pub mod app;
pub mod keymap;
pub mod theme;
pub mod ui;
