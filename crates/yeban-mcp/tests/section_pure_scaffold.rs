//! **把 `yeban-mcp` 的零依赖脚手架接进 cargo 门禁**（先例：R50① 的
//! `crates/yeban-render/tests/pure_modules_scaffold.rs`）。
//!
//! ## 背景（实测缺口）
//!
//! `verify/section_pure.rs` 是 `yeban_propose_section` 的纯函数判据：用 `#[path]`
//! 引入**真实源文件**（`src/domain/section_build.rs` / `src/domain/ids.rs`），文件头
//! 登记了本机 `rustc --edition 2024 --test` 的逐字命令。但 `grep -rn 'section_pure'
//! scripts/ .github/` 只命中 `clippy-changed.sh` 的 **clippy 元数据**分支 ——
//! **没有任何门禁执行它的判据**，这些判据是**开环**的。其中"未知 `materialSource`
//! 被响亮拒绝"这一条在 CI 执行的 `tests/` 里没有等价物（实现见
//! `MaterialSource::parse`，断言只在本脚手架）。
//!
//! ## 这个文件做什么
//!
//! `#[path]` 直接引入 `verify/section_pure.rs`。本文件属于**另一个 crate**
//! （集成测试），有独立的模块树 ⇒ 被包含的 `ids.rs` / `section_build.rs` 是这个测试
//! crate 的子模块，与 `src/` 的同名模块各自独立编译（代价是几秒编译时间，换来判据
//! 进门禁）。`section_build.rs` 的模块文档明确写着**没有** `use crate::…` ⇒ 模块化后
//! 路径继续成立；`verify/` 里嵌套 `#[path]` 的相对基准是**声明它的文件所在目录**，
//! 与本文件无关。
//!
//! ⛔ 不在 `mod` 声明上加 `#[allow(dead_code)]`：被包含文件自己的第一条内层属性就是
//! 它，重复会触发 clippy 的 `duplicated attribute`（渲染侧同一先例的实测教训）。

#[path = "../verify/section_pure.rs"]
mod section_pure_scaffold;
