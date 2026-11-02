//! 宿主错误（手写 `Display`，本 crate 零第三方依赖，故不用 `thiserror`）。

use std::fmt;

/// 插件宿主操作可能失败的方式。
#[derive(Debug)]
pub enum HostError {
    /// 动态库打不开（`dlopen`／`LoadLibrary` 失败，含系统错误文本）。
    LibraryLoad(String),
    /// 动态库里没有要找的导出符号。
    SymbolMissing(&'static str),
    /// 动态库没有 `GetPluginFactory` 导出（不是 VST3 模块）。
    FactoryMissing,
    /// 工厂里没有请求的类（`plugin_id` 指名失败，或没有任何类）。
    ClassNotFound(String),
    /// 类存在但不是乐器（无事件输入总线，或有音频输入总线）。
    NotAnInstrument(String),
    /// 插件没有实现一期必需的接口（`IComponent`／`IAudioProcessor`）。
    InterfaceMissing(&'static str),
    /// 插件不支持 32 位浮点处理（`canProcessSampleSize(kSample32)` 非真）。
    SampleSizeUnsupported,
    /// 插件输出总线不是立体声（一期只接立体声输出）。
    OutputNotStereo,
    /// 某次 COM 调用返回负值（`tresult` 错误码）。
    ///
    /// ⚠ `tresult` 的著名歧义：`kResultFalse == kResultOk == 0`，因此“非负”才是
    /// 通用成功；只有 `queryInterface` 要求严格等于 `kResultOk`（见 `vst3.rs`）。
    CallFailed {
        /// 调用的方法名（诊断用，不含插件私有信息）。
        method: &'static str,
        /// 插件返回的 `tresult`。
        code: i32,
    },
    /// 实例未激活就被要求渲染（`process` 之前必须先 `activate`）。
    NotActivated,
    /// 插件路径非法（空串、或既非文件又非 `.vst3` 包）。
    InvalidPluginPath(String),
    /// 状态容器损坏（magic／版本／长度对不上）。
    StateCorrupt(String),
    /// 其他宿主侧失败（含系统 I/O 文本）。
    Other(String),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LibraryLoad(detail) => write!(f, "插件动态库打不开: {detail}"),
            Self::SymbolMissing(symbol) => write!(f, "插件动态库没有导出符号 `{symbol}`"),
            Self::FactoryMissing => write!(f, "动态库没有 `GetPluginFactory` 导出，不是 VST3 模块"),
            Self::ClassNotFound(detail) => write!(f, "插件类找不到: {detail}"),
            Self::NotAnInstrument(detail) => write!(f, "插件类不是乐器: {detail}"),
            Self::InterfaceMissing(interface) => {
                write!(f, "插件没有实现一期必需的接口 `{interface}`")
            }
            Self::SampleSizeUnsupported => write!(f, "插件不支持 32 位浮点处理"),
            Self::OutputNotStereo => write!(f, "插件输出总线不是立体声（一期只接立体声输出）"),
            Self::CallFailed { method, code } => {
                write!(f, "插件调用 `{method}` 失败（tresult = {code}）")
            }
            Self::NotActivated => write!(f, "插件实例未激活就被要求渲染（须先 activate）"),
            Self::InvalidPluginPath(detail) => write!(f, "插件路径非法: {detail}"),
            Self::StateCorrupt(detail) => write!(f, "插件状态容器损坏: {detail}"),
            Self::Other(detail) => write!(f, "{detail}"),
        }
    }
}

impl std::error::Error for HostError {}
