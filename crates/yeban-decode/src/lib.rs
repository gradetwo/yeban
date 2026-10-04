//! # yeban-decode — 音频解码与重采样
//!
//! symphonia 全格式解码 + rubato 多相重采样（44.1k/96k ↔ 48k）
//! [ARCH-DSP-002]。解码在后台池线程执行，结果进入不可变资产，绝不进入实时回调。
//!
//! 规范来源 (Normative): `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3/§4
//!
//! 状态: **scaffold（Phase 2 之前为占位骨架，尚无实现）**。
//!
#![deny(missing_docs)]
