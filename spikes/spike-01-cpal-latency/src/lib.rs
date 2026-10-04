//! # Spike: cpal 低时延跨平台驱动验证
//!
//! 规范 ID: `ROAD-M0-001` (docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md §3 Phase 0)
//!
//! ## 方法 (Method)
//!
//! 在三平台 (macOS CoreAudio / Windows WASAPI 独占 / Linux ALSA+PipeWire) 上以 64–128 采样点
//! 跑通 cpal 输出流，测量真实硬件往返时延。
//!
//! ## 通过判据 (Pass criterion)
//!
//! 30 分钟连续播放零 underrun/xrun；硬件往返 ≤ 5.0ms (64 采样点)。
//!
//! ## 纪律
//!
//! 本 crate 是**一次性可行性验证**，不是产品代码：它必须能在 CI 上自动跑完并给出
//! 机器可读的判据。验证结论写入 `docs/ledger/`，结论成立后其结论被汲取进正式的
//! `crates/yeban-*`，spike 本身可以按需退役 (docs/DEV_WORKFLOW.md「明确废弃」)。
//!
//! 状态: **未实现**。
//!
#![deny(missing_docs)]
