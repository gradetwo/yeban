//! # yeban-plugin-host — 崩溃隔离插件宿主
//!
//! [v2.0.0] 跨进程商业插件宿主：`clack` (CLAP) / `vst3-sys`（VST3 SDK ≥ 3.8.0，MIT）
//! + POSIX 共享内存桥接 + 崩溃看门狗与热重启。shm 往返目标 < 0.3ms。
//!
//! **红线**: 仓库内绝不允许出现 Steinberg ASIO SDK 头文件或专有代码 [MUST-GATE-013]。
//!
//! 规范来源 (Normative): `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §9 (`ARCH-PLUG-001`)
//!
//! 状态: **scaffold（v2.0.0 之前为占位骨架，尚无实现）**。
//!
#![deny(missing_docs)]
