//! 插件宿主（`yeban-plugin-host`）：VST3 乐器的一期进程内宿主＋效果器宿主。
//!
//! 一期范围（ADR-0069）：扫描／验证／加载＋MIDI 输入＋音频渲染拉取＋
//! 参数＋状态保存恢复；VST3 先行，CLAP 随后；跨进程隔离＋shm＋看门狗为 phase 2。
//! 效果器线：实例化＋立体声音频总线＋`process_audio`＋参数＋状态往返；
//! 效果器接轨内插入点（首个非旁通带绑定效果器胜出，见渲染装配）。
//!
//! # ⚠ 崩溃隔离现状与风险声明（T-FIX-339）
//!
//! **已落地（探测期隔离）**：[`Host::scan`] 在探针二进制可用时，把「加载／类枚举／
//! 验证激活」放进独立子进程（[`probe`]），坏插件 `abort()`／SIGSEGV／挂起只杀子进程，
//! 应用继续扫下一个候选；超时按 `kill` 处理。探针由 `YEBAN_PLUGIN_PROBE_BIN` 指向，
//! 应用分发必须携带 `yeban-plugin-probe` 二进制；未部署时回退进程内原路径（旧部署兼容）。
//!
//! **未落地（ phase 2，⛔ 不当作已解决）**：
//! - 渲染期 `process` / `process_audio` 仍在调用线程直接调插件虚表，坏插件可在音频线程
//!   卡死或崩溃并带走应用；进程外实例 + shm + 音频线程外 watchdog 尚未实现。
//! - `Host::load` / `load_effect` 的验证激活仍在进程内（仅扫描路径已隔离）。
//!
//! 使用插件（尤其未经验证的第三方 VST3）前请知悉上述渲染期风险；离线导出 / 整曲预览
//! 执行插件，实时播放不执行（见 `yeban-app::plugin_panel::PLUGIN_REGISTRATION_NOTE`）。
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

pub mod ara;
pub mod dylib;
pub mod error;
pub mod host;
pub mod probe;
pub mod stub;
pub mod types;
pub mod vst3;

pub use ara::{
    AraClip, AraClipKind, AraFactoryInfo, AraNote, AraPlacement, AraRenderedAudio,
    AraRestoreOutcome, AraSession, PcmData, inspect_ara_factory,
};
pub use error::HostError;
pub use host::Host;
pub use stub::{STUB_GAIN_ID, STUB_SINE_ID, StubEffect, StubInstrument};
pub use types::{
    MidiEvent, MidiEventKind, ParamInfo, PluginClass, PluginDescriptor, PluginInstance, PluginKind,
    PluginState,
};
pub use vst3::{ProcessContext, Vst3Instance, Vst3Module};
