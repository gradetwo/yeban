//! 插件宿主（`yeban-plugin-host`）：VST3 乐器的一期进程内宿主。
//!
//! 一期范围（ADR-0069）：扫描／验证／加载＋MIDI 输入＋音频渲染拉取＋
//! 参数＋状态保存恢复；VST3 先行，CLAP 随后；跨进程隔离＋shm＋看门狗为 phase 2。
//!
//! 设计要点：
//! - 零新增第三方依赖：只用 std＋手写 FFI（`dlopen`／`LoadLibrary`＋COM 虚表）。
//! - `unsafe` 只活在本 crate 内（FFI 边界），且每处附 `SAFETY` 注释；
//!   调用方（`yeban-render` 等）只见安全 API。
//! - 渲染层不直接依赖 VST3 细节：它只见 [`PluginInstance`] trait；
//!   VST3 实例与测试桩都实现该 trait。
//! - 本 crate 不在仓库 `#![forbid(unsafe_code)]` 的 crate 之列（model／theory／
//!   dsp／render 才禁 unsafe）；但 unsafe 仍只出现在 FFI 边界，不扩散到业务逻辑。
//!
//! **红线**: 仓库内绝不允许出现 Steinberg ASIO SDK 头文件或专有代码 [MUST-GATE-013]。
//!
//! 规范来源 (Normative): `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §9 (`ARCH-PLUG-001`)；
//! 一期分期裁决见 `docs/adr/ADR-0069-plugin-host-v2-phase1.md`（Proposed）。

#![deny(missing_docs)]

pub mod dylib;
pub mod error;
pub mod host;
pub mod stub;
pub mod types;
pub mod vst3;

pub use error::HostError;
pub use host::Host;
pub use stub::{STUB_SINE_ID, StubInstrument};
pub use types::{
    MidiEvent, MidiEventKind, ParamInfo, PluginClass, PluginDescriptor, PluginInstance, PluginKind,
    PluginState,
};
pub use vst3::{Vst3Instance, Vst3Module};
