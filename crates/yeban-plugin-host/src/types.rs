//! 宿主公共类型：接口标识、事件、参数、描述符、状态容器与实例 trait。
//!
//! VST3 常量来源：Steinberg `vst3_pluginterfaces`（MIT）原文。
//! IID 的 16 字节＝4 个 u32 **大端**串联（SDK 的 `FUID` 构造口径；JUCE 的换算代码同口径）。
//! ⚠ 本文件只定常量与 trait，COM 虚表布局在 [`crate::vst3`]。

use crate::error::HostError;

/// VST3 类型唯一标识（16 字节）。
pub type Tuid = [u8; 16];

/// 由 4 个 u32 拼成 `Tuid`（大端串联，见模块文档）。
#[must_use]
pub const fn tuid(a: u32, b: u32, c: u32, d: u32) -> Tuid {
    let mut out = [0_u8; 16];
    out[0] = (a >> 24) as u8;
    out[1] = (a >> 16) as u8;
    out[2] = (a >> 8) as u8;
    out[3] = a as u8;
    out[4] = (b >> 24) as u8;
    out[5] = (b >> 16) as u8;
    out[6] = (b >> 8) as u8;
    out[7] = b as u8;
    out[8] = (c >> 24) as u8;
    out[9] = (c >> 16) as u8;
    out[10] = (c >> 8) as u8;
    out[11] = c as u8;
    out[12] = (d >> 24) as u8;
    out[13] = (d >> 16) as u8;
    out[14] = (d >> 8) as u8;
    out[15] = d as u8;
    out
}

/// `FUnknown` 的 IID（与 COM `IUnknown` 同值）。
pub const FUNKNOWN_IID: Tuid = tuid(0x0000_0000, 0x0000_0000, 0xC000_0000, 0x0000_0046);

/// `Vst::IComponent` 的 IID（`vst/ivstcomponent.h` 的 `DECLARE_CLASS_IID` 原文）。
pub const ICOMPONENT_IID: Tuid = tuid(0xE831_FF31, 0xF2D5_4301, 0x928E_BBEE, 0x2569_7802);

/// `Vst::IAudioProcessor` 的 IID（`vst/ivstaudioprocessor.h` 原文）。
pub const IAUDIO_PROCESSOR_IID: Tuid = tuid(0x4204_3F99, 0xB7DA_453C, 0xA569_E79D, 0x9AAE_C33D);

/// `Vst::IEditController` 的 IID（`vst/ivsteditcontroller.h` 原文）。
pub const IEDIT_CONTROLLER_IID: Tuid = tuid(0xDCD7_BBE3, 0x7742_448D, 0xA874_AACC, 0x979C_759E);

/// `IConnectionPoint`（`pluginterfaces/base/iconnectionpoint.h` 原文；字节取自
/// Dexed.so 的 `Steinberg::Vst::IConnectionPoint_iid`，2026-11-03 实测）。
pub const ICONNECTIONPOINT_IID: Tuid = tuid(0x70A4_156F, 0x6E6E_4026, 0x9891_48BF, 0xAA60_D8D1);

/// `tresult` 成功（`kResultOk`）。
pub const RESULT_OK: i32 = 0;

/// `tresult` 真（`kResultTrue`）。注意 SDK 原文：`kResultTrue == kResultOk == 0`，
/// 与 C 布尔惯例（真＝1）相反；真插件（JUCE）实测亦如此。
pub const RESULT_TRUE: i32 = 0;

/// `tresult` 假（`kResultFalse`）。
pub const RESULT_FALSE: i32 = 1;

/// `tresult` 无接口（`kNoInterface`）。
pub const RESULT_NO_INTERFACE: i32 = -1;

/// `tresult` 未实现（`kNotImplemented`，`0x80004001`）。
pub const RESULT_NOT_IMPLEMENTED: i32 = 0x8000_4001_u32 as i32;

/// 全零 TUID（`FUID::isValid` 的否定：全零＝无效类标识）。
pub const TUID_INVALID: Tuid = [0_u8; 16];

/// 把 `Tuid` 写成 32 位十六进制（小写，无分隔符；诊断与 `plugin_id` 的 CID 拼法用）。
#[must_use]
pub fn tuid_hex(id: &Tuid) -> String {
    id.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 一期宿主发给插件的 MIDI 事件（只含音符 on／off；CC 一期不发，见 ADR-0068 D7）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MidiEvent {
    /// 块内偏移（帧；`0..frames`）。
    pub sample_offset: u32,
    /// 事件体。
    pub kind: MidiEventKind,
}

/// 音符事件体。
///
/// ⚠ VST3 的力度是 **f32（0.0..=1.0）**，不是 MIDI 的 0..=127：渲染层在构造事件时
/// 除以 127.0，宿主层不再换算第二遍。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MidiEventKind {
    /// 音符开：`note_id` 在一次渲染里唯一（调度下标），off 必须带同一个 `note_id`。
    NoteOn {
        /// MIDI 通道（0..=15）。
        channel: u8,
        /// MIDI 音高（0..=127）。
        pitch: u8,
        /// 力度（0.0..=1.0）。
        velocity: f32,
        /// 音符标识（调度下标）。
        note_id: i32,
    },
    /// 音符关。
    NoteOff {
        /// MIDI 通道（0..=15）。
        channel: u8,
        /// MIDI 音高（0..=127）。
        pitch: u8,
        /// 释音力度（0.0..=1.0；多数插件忽略）。
        velocity: f32,
        /// 与对应 NoteOn 相同的 `note_id`。
        note_id: i32,
    },
}

/// 插件参数的一条描述（来自控制器 `getParameterInfo`）。
#[derive(Clone, Debug, PartialEq)]
pub struct ParamInfo {
    /// 参数标识（`ParamID`，u32）。
    pub id: u32,
    /// 标题（UTF-16 解码，丢失字符按 `from_utf16_lossy`）。
    pub title: String,
    /// 短标题。
    pub short_title: String,
    /// 单位串（可为空）。
    pub units: String,
    /// 离散步数（0＝连续）。
    pub step_count: i32,
    /// 缺省归一化值（0.0..=1.0）。
    pub default_normalized: f64,
    /// 标志（`ParameterFlags`；bit 0＝`kCanAutomate`）。
    pub flags: i32,
}

impl ParamInfo {
    /// 是否可自动化（`kCanAutomate`，bit 0）。
    #[must_use]
    pub const fn can_automate(&self) -> bool {
        self.flags & 1 != 0
    }
}

/// 插件类的粗分类（扫描与加载时的乐器判定用）。
///
/// 判定口径（ADR-0068 D12）：乐器 ⟺ 有事件输入总线且**无**音频输入总线；
/// 效果器 ⟺ 有音频输入总线。枚举失败（实例化失败）时为 `Unknown`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginKind {
    /// 乐器（可当轨音源）。
    Instrument,
    /// 效果器（一期不接链，只列出）。
    Effect,
    /// 未能判定（实例化失败等）。
    Unknown,
}

/// 工厂里的一个类（扫描枚举结果）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginClass {
    /// 类标识（`PClassInfo.cid`）。
    pub id: Tuid,
    /// 类名（`PClassInfo.name`，`char` 串）。
    pub name: String,
    /// 类别（`PClassInfo.category`，恒为 `"Audio Module Class"` 的插件类）。
    pub category: String,
    /// 粗分类（实例化探针的结论；模块级枚举失败时为 `Unknown`）。
    pub kind: PluginKind,
    /// 该类实例化／探针失败的原因（`None`＝探针通过）。
    pub error: Option<String>,
}

/// 一个插件模块的扫描描述（`yeban_scan_plugins` 的一行）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginDescriptor {
    /// 模块路径（`.vst3` 包路径或动态库路径；`stub:` 桩不在扫描里出现）。
    pub path: String,
    /// 厂商名（`PFactoryInfo.vendor`，UTF-16 解码；读不到为空串）。
    pub vendor: String,
    /// 工厂里的类表。
    pub classes: Vec<PluginClass>,
    /// 模块级失败原因（打不开／无工厂／无类时 `Some`，此时 `classes` 为空）。
    pub error: Option<String>,
}

impl PluginDescriptor {
    /// 该模块是否可用（模块级无失败，且至少有一个类探针通过）。
    #[must_use]
    pub fn usable(&self) -> bool {
        self.error.is_none() && self.classes.iter().any(|class| class.error.is_none())
    }
}

/// 插件状态容器（一期格式，见 ADR-0068 D9）。
///
/// 布局（全部小端）：magic `b"YBPGST01"`（8 字节）＋版本 u32（＝1）＋
/// 处理器 blob 长度 u32＋处理器 blob＋控制器 blob 长度 u32＋控制器 blob。
/// 控制器缺席时控制器 blob 为空（长度 0），⛔ 不是“缺字段”。
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PluginState {
    /// 处理器状态字节（`IComponent::getState` 原样）。
    pub processor: Vec<u8>,
    /// 控制器状态字节（`IEditController::getState` 原样；无控制器时为空）。
    pub controller: Vec<u8>,
}

impl PluginState {
    /// 容器 magic。
    pub const MAGIC: [u8; 8] = *b"YBPGST01";
    /// 容器版本。
    pub const VERSION: u32 = 1;

    /// 编码成容器字节。
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.processor.len() + self.controller.len());
        out.extend_from_slice(&Self::MAGIC);
        out.extend_from_slice(&Self::VERSION.to_le_bytes());
        out.extend_from_slice(&(self.processor.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.processor);
        out.extend_from_slice(&(self.controller.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.controller);
        out
    }

    /// 从容器字节解码（长度／magic／版本逐项校验，⛔ 不信任输入长度）。
    ///
    /// # Errors
    ///
    /// magic、版本或长度对不上 ⇒ [`HostError::StateCorrupt`]。
    pub fn decode(bytes: &[u8]) -> Result<Self, HostError> {
        let read_u32 = |at: usize| -> Result<u32, HostError> {
            bytes
                .get(at..at + 4)
                .and_then(|slice| slice.try_into().ok())
                .map(u32::from_le_bytes)
                .ok_or_else(|| {
                    HostError::StateCorrupt(format!(
                        "容器在偏移 {at} 处截断（总长 {}）",
                        bytes.len()
                    ))
                })
        };
        if bytes.len() < 16 || bytes[..8] != Self::MAGIC {
            return Err(HostError::StateCorrupt("magic 不是 YBPGST01".to_owned()));
        }
        let version = read_u32(8)?;
        if version != Self::VERSION {
            return Err(HostError::StateCorrupt(format!("版本 {version} 不是 1")));
        }
        let proc_len = read_u32(12)? as usize;
        let proc_start: usize = 16;
        let proc_end = proc_start.saturating_add(proc_len);
        let ctrl_len_at = proc_end;
        let ctrl_len = read_u32(ctrl_len_at)? as usize;
        let ctrl_start = ctrl_len_at + 4;
        let ctrl_end = ctrl_start.saturating_add(ctrl_len);
        if ctrl_end != bytes.len() {
            return Err(HostError::StateCorrupt(format!(
                "长度对不上（控制器段尾 {ctrl_end} ≠ 总长 {}）",
                bytes.len()
            )));
        }
        Ok(Self {
            processor: bytes[proc_start..proc_end].to_vec(),
            controller: bytes[ctrl_start..ctrl_end].to_vec(),
        })
    }
}

/// 一期宿主的插件实例（格式无关的 trait；VST3 实例与测试桩都实现它）。
///
/// 线程规则：实例是**单主**的（同一时刻只被一个调用方持有）；`Send` 的
/// SAFETY 论证在各实现里写（VST3 实例见 `vst3.rs`）。
/// `Sync` 的契约（各实现必须遵守，⛔ 不是自动获得的）：
/// 凡进插件的方法一律 `&mut self`（渲染层 `render_block` 持 `&mut` 调用，
/// 同一实例不存在 `&` 与 `&mut` 共存）；`&self` 方法只读 Rust 侧缓存字段，
/// ⛔ 不进插件。于是跨线程共享 `&` 时插件状态不可能被并发触碰。
/// 渲染层 `AudioSource: Send + Sync` 要求本 trait 同为 `Send + Sync`，
/// 否则插件源进不了渲染计划。
pub trait PluginInstance: Send + Sync {
    /// 显示名（VST3＝类名；桩＝`stub:` 标识）。
    fn name(&self) -> &str;

    /// 是否已激活（`activate` 之后为真，`deactivate` 之后为假）。
    fn is_active(&self) -> bool;

    /// 激活：`setupProcessing`＋`setActive(true)`＋`setProcessing(true)`。
    /// 已激活时先停用再按新参数重建（采样率／块上限可变）。
    ///
    /// # Errors
    ///
    /// 插件拒绝处理配置（采样率／样本格式／总线排布）⇒ [`HostError`]。
    fn activate(&mut self, sample_rate: f64, max_block: usize) -> Result<(), HostError>;

    /// 停用：`setProcessing(false)`＋`setActive(false)`（未激活时为空操作）。
    fn deactivate(&mut self);

    /// 插件上报的处理延迟（采样点；`getLatencySamples`，激活后读）。
    /// 未激活时为 0（一期 PDC 不消费它，见 ADR-0068 D11）。
    fn latency_samples(&self) -> u32;

    /// 参数表（控制器枚举；无控制器时为空）。
    ///
    /// # Errors
    ///
    /// 控制器枚举失败 ⇒ [`HostError`]。
    fn parameters(&mut self) -> Result<Vec<ParamInfo>, HostError>;

    /// 读一个参数的归一化值（无控制器时为 `None`；`getParamNormalized` 本身
    /// 无错误通道，控制器存在时恒返回 `Some`——“参数不存在”不可观测）。
    fn parameter_value(&mut self, id: u32) -> Option<f64>;

    /// 直接设一个参数的归一化值（一期唯一的参数写入口，见 ADR-0068 D8）。
    ///
    /// # Errors
    ///
    /// 控制器缺席或插件拒绝 ⇒ [`HostError`]。
    fn set_parameter(&mut self, id: u32, value: f64) -> Result<(), HostError>;

    /// 保存状态（两段 `getState`，见 ADR-0068 D9）。
    ///
    /// # Errors
    ///
    /// 插件取状态失败 ⇒ [`HostError`]。
    fn save_state(&mut self) -> Result<PluginState, HostError>;

    /// 恢复状态（处理器 `setState` → 控制器 `setComponentState` → 控制器 `setState`）。
    ///
    /// # Errors
    ///
    /// 插件拒状态 ⇒ [`HostError`]。
    fn restore_state(&mut self, state: &PluginState) -> Result<(), HostError>;

    /// 渲染一块：事件按块内偏移发给插件，输出写进左右声道片（长度必须等于 `frames`）。
    ///
    /// ⚠ 本方法**分配**（事件表按块重建）：只走离线路径，⛔ 不进 RT 回调。
    ///
    /// # Errors
    ///
    /// 未激活、片长不对、插件 `process` 返回负值 ⇒ [`HostError`]。
    fn process(
        &mut self,
        events: &[MidiEvent],
        frames: usize,
        out_l: &mut [f32],
        out_r: &mut [f32],
    ) -> Result<(), HostError>;
}
