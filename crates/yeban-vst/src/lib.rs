//! # yeban-vst — 反向插件输出
//!
//! [v2.0.0] 把 `yeban-dsp` 反向打包成 VST3/CLAP 插件。
//! `nih-plug` 处于 **maintenance mode**（RSK-35），`vst3` feature 默认关闭
//! [AGENTS.md §2 红线 6]；`cargo-deny` 的 `unmaintained = "workspace"` 会在此依赖进入时报警。
//!
//! 规范来源 (Normative): `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §9
//!
//! 状态: **scaffold（v2.0.0 之前为占位骨架，尚无实现）**。
//!
#![deny(missing_docs)]
