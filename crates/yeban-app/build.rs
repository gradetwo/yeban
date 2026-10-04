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
//! ## 为什么必须显式打开 debug info (2026-10-05 补, `line/ui-test-port` 的实测发现)
//!
//! Slint 的 `ElementHandle` 遍历(语义控件树内省)依赖**编译期生成的 debug info**。
//! 上游 `i-slint-compiler-1.18.1/lib.rs:282` 的默认值是
//! `let debug_info = std::env::var_os("SLINT_EMIT_DEBUG_INFO").is_some();` —— 即**默认关闭**。
//! 没有它时 `element_count()` 返回 `None`, **运行时控件树恒为空且不报错**。
//!
//! 这意味着: 不打开它, `yeban-app` 的 13 个 `.slint` 对 `yeban-ui-test-port` 完全不可见,
//! 于是 `ARCH-UI-005` / UI/UX §12.3(属性读取) / §12.5(动态遮罩) / §12.4(事件注入的绝对坐标)
//! 在**真实界面**上一条都执行不了 —— 只能停在"编译通过"。
//! CI 上我们控制不了环境变量, 所以在这里显式打开, 而不是要求每个环境都设 `SLINT_EMIT_DEBUG_INFO=1`。
//!
//! 代价(如实登记): 生成的 Rust 会带元素树元数据 ⇒ 二进制略大、编译略慢。
//! 这是"UI 变更必须双重验证"(AGENTS.md §3 DoD 6) 的必要成本, 不是可选装饰。
//!
//! 本机纪律: 这个脚本**不在本机执行** —— 它一跑就会触发 Slint 的高耗 CPU 编译,
//! 那是 `docs/DEV_WORKFLOW.md` 与 `AGENTS.md` §5 明令禁止的。编译由 CI 判定。

fn main() {
    // `.expect()` 而不是 `.unwrap()`: 失败信息里要带上"是哪个 slint 文件"。
    // `slint-build` 在出错时已经把带行列号的诊断打到 stderr 了。
    //
    // `with_debug_info(true)`: 见文件头 —— 没有它, 语义控件树内省在本应用上是空操作。
    // 文档原文(slint-build 1.18.1): "This is the equivalent to setting SLINT_EMIT_DEBUG_INFO=1
    // … and is primarily used by `i-slint-backend-testing`."
    let config = slint_build::CompilerConfiguration::new().with_debug_info(true);
    slint_build::compile_with_config("ui/app.slint", config)
        .expect("slint_build::compile_with_config(ui/app.slint) 失败");
}
