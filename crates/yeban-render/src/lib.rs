//! # yeban-render — 离线渲染与导出
//!
//! Rayon 多核并行离线母带渲染器、RF64/BW64 + BEXT 写入、TPDF 抖动与 MIDI 0/1 导出。
//! 必须与实时引擎**共用同一套 PDC 算法**，且并行汇聚必须按 `EntityId` 字典序做确定性串行归约
//! [ARCH-DET-002] —— 否则浮点非结合律会造成 LSB 漂移，L1 bit-exact 契约失效。
//!
//! 规范来源 (Normative):
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §4/§5
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` ROAD-M4-004 … ROAD-M4-007, BASELINE-001
//!
//! 状态: **scaffold（Phase 4 之前为占位骨架，尚无实现）**。
//!
#![forbid(unsafe_code)]
//!
#![deny(missing_docs)]
