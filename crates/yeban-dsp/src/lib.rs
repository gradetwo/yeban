//! # yeban-dsp — 纯数学 DSP 原语
//!
//! 本 crate 是最底层的纯数学信号处理库：无 I/O、无分配、无锁、无时钟。
//! 所有算法必须能在实时音频回调内以固定块长运行且**零堆分配** [ARCH-RT-001]。
//!
//! 复用来源 (Reuse provenance, 必须保留 MIT 归属):
//! - 前瞻候选: `synth/crates/synth-core/src/dsp/**` + `voice.rs` + `fx_shaping.rs`
//!   (~8,000 行无依赖、无分配、带单元测试的 MIT 纯 Rust DSP)。
//!   移植时必须在文件头保留原始版权与 MIT 声明，并登记进 `THIRD_PARTY_LICENSES.md`。
//!   参见 `docs/ledger/legacy-reuse-audit.md`。
//!
//! 规范来源 (Normative):
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3/§4 (`ARCH-RT-*`, `ARCH-DSP-*`)
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` §2 (代码资产迁移处置清单)
//!
//! 状态: **scaffold（Phase 2 之前为占位骨架，尚无实现）**。
//!
#![forbid(unsafe_code)]
//!
#![deny(missing_docs)]
