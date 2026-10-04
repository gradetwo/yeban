//! `yeban-app` 的构建脚本: 把 `ui/app.slint` 编译成 Rust。
//!
//! 规范来源 (Normative):
//! - `docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md` §8 (Slint 组件树, 11 个 `.slint`)
//! - `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md` D2 (文件清单裁决)
//!
//! API 核验 (2026-10-05):
//! - `slint_build::compile(path: impl AsRef<Path>) -> Result<(), CompileError>`
//!   <https://docs.rs/slint-build/1.18.1/slint_build/fn.compile.html>
//! - 生成的 Rust 落在 `$OUT_DIR/<root 文件 stem>.rs`, 并把绝对路径写进
//!   `cargo:rustc-env=SLINT_INCLUDE_GENERATED=...`; 由 `slint::include_modules!()` 消费。
//!   <https://docs.rs/slint-build/1.18.1/src/slint_build/lib.rs.html> (515-559 行)
//!
//! 边界: 本脚本只编译 `ui/app.slint` 这一个根文件; 根文件通过 `import` 拉进其余
//! 11 个 `.slint`(tokens / transport / …)。`slint-build` 会为依赖图里每个文件打印
//! `cargo:rerun-if-changed`, 因此不需要在这里手写依赖清单。
//!
//! 本机纪律: 这个脚本**不在本机执行** —— 它一跑就会触发 Slint 的高耗 CPU 编译,
//! 那是 `docs/DEV_WORKFLOW.md` 与 `AGENTS.md` §5 明令禁止的。编译由 CI 判定。

fn main() {
    // `.expect()` 而不是 `.unwrap()`: 失败信息里要带上"是哪个 slint 文件"。
    // `slint-build` 在出错时已经把带行列号的诊断打到 stderr 了。
    slint_build::compile("ui/app.slint").expect("slint_build::compile(ui/app.slint) 失败");
}
