//! **把第三个零依赖脚手架（115 条判据）接进 cargo 门禁**（裁决 R50①）。
//!
//! ## 背景（本机实测的缺口）
//!
//! `verify/pure_modules.rs` 是 `rustc --edition 2024 --test` 的手工脚手架:
//! 它把 `src/{dither,pdc,rf64,sum}.rs` 与 `../../yeban-midi/src/vlq.rs`
//! 用 `#[path]` **按同一份源码**包含进来, 本机实测 **115 条判据全通过**。
//! 但 `grep -rn 'pure_modules' scripts/ .github/` **零命中**, 且它不在
//! `src/`/`tests/`/`benches/`/`examples/` 里 ⇒ **任何门禁都不跑它**, 它的 115 条
//! 是**开环判据**。
//!
//! ## 这个文件做什么（R50① 的第 1 条路: 拆成不重复模块的 `tests/` 目标）
//!
//! `#[path]` 直接引入 `verify/pure_modules.rs`。**不会**与本 crate 的 `src/` 模块
//! 冲突: 本文件属于**另一个 crate**（集成测试），它自己有独立的模块树, 被包含的
//! `dither.rs` / `rf64.rs` … 是这个测试 crate 的子模块, 与 `src/` 的同名模块各自
//! 独立编译（代价是几秒编译时间, 换来 115 条判据进门禁）。
//!
//! `verify/pure_modules.rs` 里的嵌套 `#[path]`（`../src/…`、`../../yeban-midi/src/vlq.rs`）
//! 相对的是**声明它的文件所在目录**（`verify/`）, 与本文件无关 —— 因此路径继续成立。
//!
//! ## 它为什么能通过 `cargo clippy --all-targets -- -D warnings`
//!
//! `verify/pure_modules.rs` 的文档登记过它在本机用 `clippy-driver` 逐条等价集合
//! （工作区 `[lints]`）跑过 0 告警; 本轮唯一需要的调整是**去掉 `#[path] mod` 上重复的
//! `#[allow(dead_code)]`**（文件自己的第一条内层属性就是它, 重复属性会被 clippy 判为
//! `duplicated attribute`）。本文件因此**一行 allow 都不加**。

#[path = "../verify/pure_modules.rs"]
mod pure_modules_scaffold;
