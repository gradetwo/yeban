//! # yeban-app — Slint 桌面主程序
//!
//! Slint 声明式 GUI 宿主 + 后台调度。UI 文件清单以 UI/UX 规范 §8 为准，见
//! `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md` 的裁决 D2。
//!
//! ## 关键约束
//!
//! - `[UI-GRID-004]` / `[ARCH-TOP-002]`：UI 线程与实时音频线程**物理隔离**，
//!   UI 负载绝不阻塞音频回调。
//! - `[ARCH-UI-003]` / `[UI-TEST-003]`：必须支持 `--headless` 与 `SLINT_BACKEND=headless`
//!   无头启动，供 CI 在无显示器环境跑。**实测结论**：Slint 1.18.1 **没有**名为 `headless`
//!   的后端（`SLINT_BACKEND` 只接受 `qt` / `winit` / `linuxkms`，可选 `-software` / `-skia`
//!   之类的渲染器后缀 —— 出处 <https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backends_and_renderers/>）。
//!   因此 `headless` 在 yeban-app 里是**自研哨兵值**：见 [`main`](crate) 所在的 `src/main.rs`。
//! - `[UI-TEST-001]`：自动化寻址只允许语义 Element ID（`track-{i}-fader` / `note-{ulid}-rect`
//!   / `clip-{ulid}-header` / `tab-{name}-button`），严禁绝对像素坐标。
//!   Rust 侧事实源见 [`elements`]。
//! - `[UI-A11Y-001]` / `[UI-A11Y-002]`：物理扫描码绑定 + IME `is_composing` 防护。
//!   策略表见 [`input`]（纯 Rust、零 Slint 依赖，因此可在任何机器上测试）。
//!
//! ## 规范来源 (Normative)
//!
//! - `docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md` §1 … §12
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §1 / §8
//!
//! ## 状态
//!
//! **scaffold**：Phase 3 之前是占位骨架 + 演示数据，尚无任何真实引擎/模型绑定。
//! 缺口与待决项逐条记录在 `docs/ledger/ui-shell-notes.md`。
#![deny(missing_docs)]

pub mod elements;
pub mod input;
pub mod scene;

/// `build.rs` 里 `slint_build::compile("ui/app.slint")` 生成的 Slint 组件类型。
///
/// 生成机制（已核验）：`slint-build` 把 `ui/app.slint` **及其 `import` 到的全部文件**
/// 编译进**一个**生成文件，写到 `$OUT_DIR/<根文件 stem>.rs`，并把绝对路径放进
/// `cargo:rustc-env=SLINT_INCLUDE_GENERATED`；`slint::include_modules!()` 就是
/// `include!(env!("SLINT_INCLUDE_GENERATED"))`。因此这里 `include!` 之后，`app.slint`
/// 导出的 `MainWindow` 就是 [`ui::MainWindow`]。
///
/// 出处：<https://docs.rs/slint-build/1.18.1/src/slint_build/lib.rs.html>（514-559 行）
/// 与 <https://docs.rs/slint/1.18.1/slint/macro.include_modules.html>。
///
/// ## 为什么包一层模块并挂一串 `allow`
///
/// 生成代码是**第三方产物**：它既不遵守本仓库的 `#![deny(missing_docs)]`，也不保证
/// 通过 `[workspace.lints] clippy::all = "deny"`。把这几个 `allow` 收在这一个模块里，
/// 好过在 crate 根放松全局 lint 策略 —— 后者会让本 crate 的**手写**代码也失去约束。
///
/// 边界：这些 `allow` 只覆盖 `slint::include_modules!()` 展开出来的那一棵子树。
#[allow(missing_docs, clippy::all, rust_2018_idioms)]
pub mod ui {
    #![allow(missing_docs, clippy::all, rust_2018_idioms)]

    slint::include_modules!();
}
