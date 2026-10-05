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
//! **由模型驱动（model-bound）**：界面数据来自 [`bridge`] 对 `YebanProjectV1` 的投影，
//! 经 [`host`] 单向注入 `.slint`。`.slint` 里不再有 `for … in 6` 这类把演示数据
//! 刻进界面的写法 —— 轨道数 / 剪辑数 / 段落数由工程决定。
//! `[elements]` 的语义注册表同样由投影构造，因此它总是与当前工程一致。
//! 卷帘音符的 x / y 由 tick / 音高整数派生，轨道色标由 [`bridge::parse_hex_color`]
//! 解析（非法 / 缺失显式回退）；真实的 `.yeban` 文件经 [`open`] 的容器入口打开，
//! 容器的拒绝原因**原样**上报，绝不退化成空工程。
//!
//! **混音台通道条**（app-mixer 工作线，`[ARCH-UI-002]`）：通道条数 = 工程轨道数，
//! 名字 / 音量 / 声相 / 静音 / 独奏 / 色标来自投影（推子位置 = `TrackV3::volume_db`），
//! 而**电平**来自引擎：实时线程每量子一次批量发布到 SPSC（`yeban-engine` 的生产侧），
//! UI 线程的 [`meters::MeterRuntime`] 用 `drain_latest` 抽干取最新 → [`host::apply_meters`]
//! 写 Slint 属性；`.slint` 侧 `track-{i}-meter` 的 `accessible-label` 携带 dBFS 文本，
//! 因此"界面真的消费了电平"可以被控件树机械验证。引擎的重建（快照 / 队列 / 量子驱动）
//! 由 [`engine_host`] 负责，`.yeban` 的原子落盘（`[ARCH-SEC-004]`）由 [`save`] 负责。
//!
//! **命令行面**（app-cli 工作线）：[`cli`] 是零 Slint 依赖的解析 / 用法 / 报告 / 退出码
//! 实现，`src/main.rs` 只做分发。`--open` 经 [`open`] 打开真工程文档 —— `.yeban` 容器
//! 是**唯一**工程格式（ADR-0001 D43，裸 `project.json` 兼容读路径已删除），
//! `--save-as` 经 [`save`] **原子**落盘，`--export-elements` 把
//! [`elements`] 的注册表原子写到文件；两条路径（GUI / 无窗口）共用同一份报告实现。
//!
//! **MIDI 导出**（app-export-midi 工作线，ADR-0001 D47 的裁决：唯一出口是 app CLI
//! `--export-midi`）：[`export_midi`] 把当前工程投影成 `yeban_render::midi::MidiExport`，
//! 由**那一个** SMF 编码器写成字节后原子落盘 —— 本 crate 里**没有**第二份编码器。
//! 映射表（轨道 → MIDI 轨/通道、`MidiNote` 字段 → 事件、PPQ 口径）写在
//! [`export_midi`] 的模块文档与 `docs/ledger/app-export-midi-notes.md`。
//!
//! 仍未接线的部分（走带 / Op 归约 / 设备链 / 自动化 / 声卡宿主 / UI→模型写入）
//! 逐条记在 `docs/ledger/app-mixer-notes.md` §7 与 `docs/ledger/app-binding-notes.md`
//! 的未实现项里；命令行的边界与未实现项记在 `docs/ledger/app-cli-notes.md`。
#![deny(missing_docs)]

pub mod automation;
pub mod bridge;
pub mod cli;
pub mod elements;
pub mod engine_host;
pub mod export_midi;
pub mod host;
pub mod input;
pub mod meters;
pub mod open;
pub mod save;
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
