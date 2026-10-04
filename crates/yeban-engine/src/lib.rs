//! # yeban-engine — 实时音频引擎
//!
//! 本 crate 是实时音频权威：cpal 流宿主、`rtrb` 无锁 SPSC、`Arc<EngineSnapshot>`
//! 原子交换与退役回收队列、内部 PDC 延迟补偿、FTZ/DAZ 浮点环境 [ARCH-RT-001..005, ARCH-PDC-001..002]。
//!
//! 本 crate **不属于** `#![forbid(unsafe_code)]` 名单（名单为 model/theory/dsp/render）：
//! FTZ/DAZ 设置与部分驱动边界需要 `unsafe`，但每一处都必须附 `// SAFETY:` 证明
//! [AGENTS.md §2 红线 8]。实时回调内禁止任何分配/释放/锁等待/阻塞 I/O [红线 7]。
//!
//! 规范来源 (Normative):
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` ROAD-M2-001 … ROAD-M2-008
//!
//! 状态: **scaffold（Phase 2 之前为占位骨架，尚无实现）**。
//!
#![deny(missing_docs)]
