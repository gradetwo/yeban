//! **把两个"本机零依赖验证脚手架"接进 cargo 门禁**（形态 D 第六批的发现之一）。
//!
//! ## 为什么要这个文件（本机实测的缺口）
//!
//! `examples/support/l1_receipt_tests.rs`（594 行, 18 条判据）与
//! `examples/support/l1_digest_record_tests.rs`（647 行, 20 条判据）在本轮之前
//! **不是任何 cargo 目标**, 也不是任何 `scripts/**` 或 `.github/**` 的输入:
//!
//! ```text
//! grep -rn 'l1_receipt_tests\|l1_digest_record_tests' scripts/ .github/   -> 0 命中
//! grep -rn '#\[path' crates/yeban-render/{tests,examples}/*.rs            -> 只有 export_pipeline
//! ```
//!
//! 它们的文档写明"用 `rustc --edition 2024 --test` 手动跑"。本机实测两者都通过
//! （18 + 20 条）, 但**没有任何门禁会跑它们** ⇒ 它们是**开环判据**: 改坏了不会被 CI 抓到。
//!
//! ## 这个文件做什么
//!
//! 用 `#[path]` 把两份脚手架**原样**作为子模块引入。两份文件的设计前提就是
//! "自己是 crate root"（它们用 `#[path]` 引入各自的产线助手, 并且没有 `crate::` / `super::`
//! 引用）, 因此作为子模块引入只需它们各自的 `#[path]` 相对路径继续成立 —— 那两条路径
//! 相对的是**声明它的文件所在目录**（`examples/support/`）, 与本文件无关。
//!
//! `#![allow(dead_code)]` 那两条内层属性在"模块文件"的位置上合法（它现在修饰的是子模块）。
//!
//! ## 边界（如实声明）
//!
//! - 本文件**不**修改两份脚手架的任何一行, 也不修改它们引入的产线助手;
//! - `verify/pure_modules.rs`（115 条判据）**仍不在门禁内** —— 它把 `src/` 的模块源码
//!   作为自己的模块树再引入一遍, 接进本 crate 的测试目标会与 `src/` 的同名模块重复编译,
//!   因此本轮只登记、不接线（见报告）。

// 两份脚手架的 `#[path]` 相对路径以**本文件**所在目录为基准解析,
// 因此这里指向它们真实所在的 `examples/support/`。
//
// ⚠️ 两个 `mod` 上**不能**再写 `#[allow(dead_code)]`: 两份文件各自的第一条内层属性
// 就是 `#![allow(dead_code)]`, 而它现在修饰的是子模块 —— 重复属性会被
// `cargo clippy --all-targets -- -D warnings` 判为 `duplicated attribute`（本机实测）。
#[path = "../examples/support/l1_receipt_tests.rs"]
mod l1_receipt_scaffold;

#[path = "../examples/support/l1_digest_record_tests.rs"]
mod l1_digest_record_scaffold;
