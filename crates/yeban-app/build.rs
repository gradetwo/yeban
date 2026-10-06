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
//!
//! ## 风格 (style) 的选择 —— 2026-10-06, 负责人要求「提供切换这几个主题的功能」
//!
//! **实测结论 (先量后写)**: Slint 1.18.1 选内建风格**只有编译期一条路**, 一共两个入口:
//!
//! 1. `SLINT_STYLE` 环境变量 —— 由 `i-slint-compiler-1.18.1/lib.rs:296`
//!    (`std::env::var("SLINT_STYLE")`) 读进 `CompilerConfiguration::style`;
//! 2. `slint_build::CompilerConfiguration::with_style(String)` —— 本脚本调用的这个
//!    (slint-build-1.18.1 `lib.rs:156-160`), 就是第 1 条的**程序化等价物**。
//!
//! 这个 crate 的 `slint` 依赖里**没有** `material` / `fluent` / `cupertino` / `native`
//! 这样的 cargo feature —— 它们在整个 slint 1.18.1 的 `[features]` 表里不存在
//! (已逐条核验 slint / slint-build / i-slint-backend-selector 三份清单)。风格是
//! **源文件的选择**: 编译器把 `widgets/<style>/std-widgets.slint` 编进来。
//!
//! **运行时选风格在本版本不存在**: `slint::select_built_in_style` 在 slint-1.18.1、
//! i-slint-core-1.18.1、i-slint-backend-selector-1.18.1 与 i-slint-compiler-1.18.1 的
//! 全文里**一次都没有出现**; 官方 Widget Styles 文档也只写 "The widget style is
//! determined at your project's compile time." 因此本仓库不承诺运行期换**风格**,
//! 只承诺运行期换**调色板** (见 `ui/tokens.slint` 的 `ThemeState` 与 `src/cli.rs`
//! 的 `--theme` + `--print-theme`)。
//!
//! 为什么这里**显式**把风格传给 `with_style` 而不是只靠环境变量透传:
//!   - 取值在**编译期**就被点名校验 (见 [`KNOWN_STYLES`]), 错的风格名会得到一句
//!     中文的、能指出可用集合的构建错误, 而不是编译器那句英文诊断;
//!   - 解析结果用 `cargo:rustc-env=YEBAN_SLINT_STYLE=…` 交给 `src/cli.rs`, 于是
//!     `yeban-app --print-theme` 能把"这个二进制**实际**编的是哪个风格"如实打出来。
//!
//! **默认一位未改**: `SLINT_STYLE` 没设时取 [`DEFAULT_STYLE`] = `"fluent"`, 那正是
//! `i-slint-compiler` 在 `style = None` 时的 `unwrap_or_else(|| "fluent".into())`。
//! 所以默认构建与本次改动之前的构建**是同一个风格** —— 这是"今天的默认外观不变"的
//! 机械保证, `tests/golden/linux/**` 因此不需要重生成。

/// Slint 1.18.1 认得的**全部**内建风格名。
///
/// 取值来源 (2026-10-06, 逐条核验源码, 不是记忆):
/// - 具体风格 = `i-slint-compiler-1.18.1/widgets/` 下带 `std-widgets.slint` 的目录:
///   `material` / `fluent` / `cupertino` / `cosmic` / `qt`;
/// - `-light` / `-dark` 变体 = `i-slint-compiler-1.18.1/fileaccess.rs:104-113` 的 `ALIASES` 表;
/// - `native` = `i-slint-compiler-1.18.1/typeloader.rs:978` 额外追加的别名 (按目标平台解析:
///   macOS → `cupertino`, Windows → `fluent`, Android → `material`, Linux/BSD → 有 Qt 则 `qt` 否则 `fluent`)。
const KNOWN_STYLES: &[&str] = &[
    "fluent",
    "fluent-light",
    "fluent-dark",
    "material",
    "material-light",
    "material-dark",
    "cupertino",
    "cupertino-light",
    "cupertino-dark",
    "cosmic",
    "cosmic-light",
    "cosmic-dark",
    "qt",
    "native",
];

/// `SLINT_STYLE` 没设时用的风格 —— 与 Slint 自己的默认值**逐字相同**。
///
/// 出处: `i-slint-compiler-1.18.1/typeloader.rs:957`
/// (`compiler_config.style.clone().unwrap_or_else(|| "fluent".into())`)。
/// 改这个常量 = 改默认外观 = 必须重生成 Linux golden 基线, 因此**不许**顺手改。
const DEFAULT_STYLE: &str = "fluent";

fn main() {
    // `.expect()` 而不是 `.unwrap()`: 失败信息里要带上"是哪个 slint 文件"。
    // `slint-build` 在出错时已经把带行列号的诊断打到 stderr 了。
    //
    // `with_debug_info(true)`: 见文件头 —— 没有它, 语义控件树内省在本应用上是空操作。
    // 文档原文(slint-build 1.18.1): "This is the equivalent to setting SLINT_EMIT_DEBUG_INFO=1
    // … and is primarily used by `i-slint-backend-testing`."
    //
    // `SLINT_STYLE` 的取值**在这里**被拒绝 (而不是留给编译器): 见文件头。
    let style = std::env::var("SLINT_STYLE")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_STYLE.to_owned());
    if !KNOWN_STYLES.contains(&style.as_str()) {
        panic!(
            "SLINT_STYLE=`{style}` 不是 Slint 1.18.1 的内建风格名。可用: {}\n\
             (默认 = `{DEFAULT_STYLE}`; 取值表出处见 build.rs 头部的注释)",
            KNOWN_STYLES.join(" | ")
        );
    }
    // 让 `src/cli.rs` 的 `--print-theme` 能说出这个二进制**实际**编的是哪个风格。
    println!("cargo:rustc-env=YEBAN_SLINT_STYLE={style}");
    // `slint-build` 自己也会打印这一条; 重复打印是幂等的, 这里保留是为了让本脚本
    // **独立可读**: 只看 build.rs 就知道改环境变量会触发重编。
    println!("cargo:rerun-if-env-changed=SLINT_STYLE");

    let config = slint_build::CompilerConfiguration::new()
        .with_debug_info(true)
        .with_style(style);
    slint_build::compile_with_config("ui/app.slint", config)
        .expect("slint_build::compile_with_config(ui/app.slint) 失败");
}
