//! 构建脚本：编译 Tier-1 夹具 `ui/fixture.slint`。
//!
//! ## 为什么必须显式打开 debug info
//!
//! `i-slint-backend-testing` 的 `ElementHandle` 遍历依赖编译期生成的 **debug info**
//! （`item.element_count()` / `item_element_infos()`）。上游 `i-slint-compiler-1.18.1/lib.rs:282`
//! 的默认值是：
//!
//! ```text
//! let debug_info = std::env::var_os("SLINT_EMIT_DEBUG_INFO").is_some();
//! ```
//!
//! 也就是说**默认关闭**，只有 `SLINT_EMIT_DEBUG_INFO` 存在或调用
//! `CompilerConfiguration::with_debug_info(true)` 才会打开。CI 上我们控制不了环境变量，
//! 所以必须在这里显式打开 —— 否则运行时控件树是**空**的，`[UI-TEST-001]` 的语义寻址
//! 与 `[UI-MCP-002]` 的遮罩都无从谈起（实测：CI run 37221680724 的
//! `ControlTree { nodes: {} }`）。
//!
//! `with_debug_info` 的文档原文（`slint-build-1.18.1/lib.rs:229-236`）：
//! *"This is the equivalent to setting `SLINT_EMIT_DEBUG_INFO=1` and using the `slint!()` macro
//! and is primarily used by `i-slint-backend-testing`."*
//!
//! ## 为什么不用 `slint::slint!` 内联宏
//!
//! 见 `ui/fixture.slint` 的注释：宏路径只能靠环境变量，且宏的**每一条**编译器警告都会
//! 被展开成 `#[deprecated] const WARNING`，在 `-D warnings` 下变成硬错误
//! （`i-slint-compiler-1.18.1/diagnostics.rs:575-594`）。

fn main() {
    let config = slint_build::CompilerConfiguration::new().with_debug_info(true);
    slint_build::compile_with_config("ui/fixture.slint", config)
        .expect("编译 ui/fixture.slint 失败 (Tier-1 测试夹具)");
}
