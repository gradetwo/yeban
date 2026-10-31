//! **把 `yeban-mcp` 的第二份零依赖脚手架接进 cargo 门禁**（先例：`tests/section_pure_scaffold.rs`
//! 与 `crates/yeban-render/tests/pure_modules_scaffold.rs`）。
//!
//! ## 背景（实测缺口）
//!
//! `verify/render_pure.rs` 是 `yeban_render_master` 纯逻辑的本机脚手架：`#[path]` 引**真实源文件**
//! （`src/domain/render_math.rs`、`src/domain/render_clip_math.rs`、`../../yeban-model/src/time_math.rs`）。
//! 它**不是 cargo 目标**，`grep -rn 'render_pure' scripts/ .github/` 只命中
//! `scripts/gates/clippy-changed.sh` 的 **clippy 元数据**分支 ⇒ 它的判据在 CI 里**一条都没被跑**。
//!
//! ## 与 section_pure 包装的**唯一差异**：`crate::PPQ`
//!
//! 两份脚手架都在根上写了 `extern crate self as …`（因为它按 crate root 编译，`render_math.rs`
//! 用的 `yeban_model::time_math` / `libm::log10f` 要解析回自己）。实测（`rustc --edition 2024`
//! 三个 20 行探针）：
//!
//! - 别名写在**模块里**时，**同一模块内**的 `libm::…` 会解析到别名（压过 extern prelude）；
//! - 别名写在**父模块**里时，**子模块看不见它**（`E0433`）—— 所以把 `render_pure.rs` 当模块引入后，
//!   其子模块里的 `yeban_model::…` / `libm::…` 会回落到**真实依赖 crate**（同一份源）；
//! - 别名写在 **crate root** 时对全 crate 可见 —— 这正是脚手架原本能跑的原因。
//!
//! 因此只需补一件东西：`time_math.rs` 的单元测试写的是 `crate::PPQ`，包装后 `crate::` 指向**本测试
//! crate 的根** ⇒ 根上用 `pub use yeban_model::PPQ;` **re-export**（⛔ 不写字面量 960，⛔ 不立第二份真相）。
//!
//! ## 代价与边界
//!
//! 被包含的三个模块在本测试 crate 里**各自独立编译一次**（几秒），换来判据进门禁。
//! `render_math.rs` 的 `#[cfg(test)]` 单元测试也随之一并执行 —— 它们本来就该在门禁里。

#![allow(dead_code)]

// `time_math.rs` 的 `#[cfg(test)]` 用 `crate::PPQ`（本 crate = 集成测试 crate）。
pub use yeban_model::PPQ;

#[path = "../verify/render_pure.rs"]
mod render_pure_scaffold;
