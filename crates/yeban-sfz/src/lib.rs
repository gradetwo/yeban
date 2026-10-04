//! # yeban-sfz — SFZ v2 采样器
//!
//! 零拷贝 SFZ v2 词法解析器 + 预分配 voice pool（默认 512 声部、可配 1024）
//! [ARCH-RT-004]。解析器是不可信输入边界：必须能承受 `cargo-fuzz` 千万次变异零崩溃
//! [MUST-GATE-011]。
//!
//! 规范来源 (Normative): `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` ROAD-M2-005 … ROAD-M2-006
//! 移植语义来源 (learn-from, MIT): `groove/src/audio/sfz/**` (TypeScript, 2,979 行) —— 只借鉴
//! keyswitch / include / define 的语义与边界处理，代码用 Rust 重写，不做逐行转译。
//!
//! 状态: **scaffold（Phase 2 之前为占位骨架，尚无实现）**。
//!
#![deny(missing_docs)]
