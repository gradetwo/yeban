//! # yeban-theory — 乐理与流派规则引擎
//!
//! 本 crate 承载 `yeban_propose_section` 的领域逻辑：音阶/和弦构造、和弦走向展开、
//! 声部连接 (voice leading) 与 159 种流派规则库。全部为纯函数与纯数据，不触碰音频设备，
//! 也不读取系统时钟 —— 随机性一律由调用方传入的 `rng_seed` 驱动，以保证 [ARCH-DET-001] 的确定性契约。
//!
//! 规范来源 (Normative):
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §8 (`yeban-theory`)、§7 (`yeban_propose_section`)
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` ROAD-M4-003
//!
//! 状态: **scaffold（Phase 1 之前的占位骨架，尚无实现）**。
//!
#![forbid(unsafe_code)]
//!
#![deny(missing_docs)]
