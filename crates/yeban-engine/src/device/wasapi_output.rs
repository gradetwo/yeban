//! **自研 WASAPI 输出后端**（T-FIX-409；T-FIX-225/251 链的裁决 A）。
//!
//! ## 为什么必须自己写这一条腿
//!
//! cpal 0.18.2 的 WASAPI 后端把 `IMMDeviceEnumerator` 放进**进程级** `OnceLock`
//! （`cpal-0.18.2/src/host/wasapi/device.rs:1145`，`get_enumerator()` 只有
//! `get_or_init`，**没有**失效/重建入口）。于是只要有一条活着的 `IAudioClient`
//! 跨过了 `audiosrv` 重启，这个进程里的 `IMMDevice` / `IAudioClient` 就再也
//! `Activate` 不出来 —— 实测恒报 `RPC_E_CHANGED_MODE`（`0x80010106`），只有
//! **新进程**能恢复（251 的四组对照实验：旧流活着跨重启 ✗、先丢旧流 ✗、
//! 全新 MTA 线程 ✗、新进程 ✓）。
//!
//! 本模块把输出腿整个换成自己的实现，**枚举器不缓存**：每次枚举、每次打开、
//! 每次重建都 `CoCreateInstance` 一个**新的** `IMMDeviceEnumerator`，用完即弃。
//! 于是"重建"就是把旧的丢掉、新建一个再来一遍。
//!
//! ## 实测边界（本机 Windows 真机，2026-11-25；⛔ 别把没测到的说成测到了）
//!
//! **测到的**：
//!
//! - cpal 腿在**同一天、同一台机器**上仍然是坏的：开一条流并让它活到 `audiosrv`
//!   重启之后，同进程里后续 6 次 `build_output_stream` **全部**报
//!   `RPC_E_CHANGED_MODE`（`0x80010106`），另起一条线程连
//!   `default_output_device()` 都拿不到。⇒ 本票的前提（#4109）成立。
//! - 本模块的腿在同样场景下**自己回来了**：从 `Stop-Service` 起算约 1.8 s 稳定
//!   出帧（含端点重新发布与 1 s 稳定窗口），设备列表重启前后逐台一致，
//!   端点在公开句柄上回到 `Ok`。
//!
//! **没测到的（⛔ 不写进结论）**：把 [`create_enumerator`] 改成缓存
//! （`thread_local`，cpal 进程级 `OnceLock` 的同构做法）之后，判据 B **仍然绿**
//! —— 也就是说本机实测**不能**证明"必须每次新建枚举器"。因此这里只声称
//! 「这条腿能恢复」，⛔ 不声称「缓存枚举器是唯一病因」。真要钉死病因，得把
//! `IMMDevice` / `IAudioClient` / 枚举器三层拆开逐个复用去测，超出本票范围。
//! 之所以仍然每次新建：它是安全的（枚举器很便宜），也是本票裁决 A 的形态。
//!
//! ## 与 cpal 腿的分工
//!
//! | 腿 | 谁做 | 为什么 |
//! | --- | --- | --- |
//! | 输出（Windows） | **本模块** | 需要可重建的枚举器 |
//! | 输入 | cpal（[`super::open_input`]） | 不涉及本票的失效形态；保持一致 |
//! | 输出（非 Windows） | cpal | 本票只针对 WASAPI |
//!
//! ## 线程与公寓
//!
//! 每条输出流有自己的渲染线程（`yeban-wasapi-render-*`）：它先
//! [`super::apartment::Apartment::bind`] 把公寓定成 MTA，再建流、填缓冲。
//! 枚举/查询类调用仍在 [`super::backend`] 的那条 MTA 线程上 —— 两条线程同属
//! 一个进程的 MTA，接口指针可以直接跨线程用（这与 251 的"单线程"约束不冲突：
//! 251 约束的成因是 cpal 的进程级枚举器，本模块根本没有那份缓存）。
//!
//! ## 实时约束
//!
//! 填缓冲的路径（[`pump`]）零分配、零锁、零阻塞 I/O、零日志：暂存缓冲、
//! 事件句柄、设备格式都在建流/重建时准备好；重建发生在检测到失效之后
//! （控制路径），不在稳态回调里。

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

use cpal::{SampleFormat, SupportedBufferSize, SupportedStreamConfigRange};
use windows::Win32::Foundation::{CloseHandle, HANDLE, PROPERTYKEY, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Media::Audio::{
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, DEVICE_STATE_ACTIVE, IAudioClient,
    IAudioRenderClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX,
    WAVEFORMATEXTENSIBLE, eConsole, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, STGM_READ, StructuredStorage::PropVariantClear,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::core::PCWSTR;

use super::{
    BackendErrorLog, DeviceError, DeviceInfo, EndpointHealth, EngineConfig, describe,
    negotiate_traced, pick_named_device_index,
};

/// `MMDeviceEnumerator` 的类 ID 由 windows crate 给（`MMDeviceEnumerator`）；这里只需
/// 属性键：`PKEY_Device_FriendlyName`（`{a45c254e-df1c-4efd-8020-67d146a850e0}, 14`）。
///
/// ⚠ 不引 `Win32_Devices_FunctionDiscovery` / `Win32_Devices_Properties` 两个 feature
/// 只为拿这个常量：它是一个**稳定**的 Windows 属性键，写在本地即可（值与 SDK 一致）。
const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: windows::core::GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};

/// `VT_LPWSTR`（`VARENUM` 的 31 号）：属性值的字符串形态。
const VT_LPWSTR: u16 = 31;

/// `RPC_E_CHANGED_MODE`：`0x80010106`。线程公寓被定成另一种模式，MTA 改不回来。
const RPC_E_CHANGED_MODE: u32 = 0x8001_0106;
/// `AUDCLNT_E_DEVICE_INVALIDATED`：`0x88890004`。流被后端作废（`audiosrv` 重启的现场）。
const AUDCLNT_E_DEVICE_INVALIDATED: u32 = 0x8889_0004;
/// `AUDCLNT_E_SERVICE_NOT_RUNNING`：`0x8889000A`。音频服务不在。
const AUDCLNT_E_SERVICE_NOT_RUNNING: u32 = 0x8889_000A;
/// `AUDCLNT_E_NOT_INITIALIZED`：`0x88890001`。
const AUDCLNT_E_NOT_INITIALIZED: u32 = 0x8889_0001;
/// `AUDCLNT_E_ENDPOINT_CREATE_FAILED`：`0x8889000B`。端点对象建不出来（设备刚回来时常见）。
const AUDCLNT_E_ENDPOINT_CREATE_FAILED: u32 = 0x8889_000B;
/// `ERROR_NOT_FOUND`：`0x80070490`。按 id 取设备时"元素找不到"（端点索引还没发布完）。
const ERROR_NOT_FOUND: u32 = 0x8007_0490;
/// `AUDCLNT_E_UNSUPPORTED_FORMAT`：`0x88890008`。格式不被接受 —— 重建解决不了。
const AUDCLNT_E_UNSUPPORTED_FORMAT: u32 = 0x8889_0008;

/// 首次建流失败后的等待上限：超过它就认为这次打开失败（与 cpal 腿的
/// [`super::STREAM_OPEN_TIMEOUT`] 同量级，避免界面在设备被独占时无限等）。
const OPEN_BUDGET: Duration = Duration::from_secs(5);
/// 重建时的退避起点。
const RETRY_FIRST: Duration = Duration::from_millis(120);
/// 重建时的退避上限（端点索引一般 1~3 s 回来，1 s 的采足够细）。
const RETRY_MAX: Duration = Duration::from_millis(1000);
/// 音频事件的等待上限：正常时事件每 ~20 ms 就来一次，这个值只在设备失效时兜底。
const WAIT_SLICE: Duration = Duration::from_millis(100);
/// 连续多久没有一次成功的填充就判定"流已经不出声了"（设备被拔/服务停了）。
const STALL_LIMIT: Duration = Duration::from_millis(400);
/// 共享模式请求的缓冲时长（100 ns 单位）：20 ms，与实测的设备默认周期一致。
const BUFFER_DURATION: i64 = 200_000;
/// 暂停/未启流时等命令的时间片（决定 `play`/`pause` 的响应上限）。
const IDLE_TICK: Duration = Duration::from_millis(2);

/// 一条 WASAPI 报错（HRESULT + 可读消息）。
///
/// ⚠ 不用 `cpal::Error` 直接贯穿：它拿不到 HRESULT 数值，而"这个 HRESULT 要不要
/// 重建枚举器"正是本模块的判决输入。
#[derive(Clone, Debug)]
struct WasapiError {
    code: u32,
    message: String,
}

impl WasapiError {
    fn new(code: u32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// 从 `windows` 的错误取值（`windows::core::Error` 同时给 HRESULT 与消息）。
    fn from_windows(step: &str, error: &windows::core::Error) -> Self {
        Self::new(
            error.code().0 as u32,
            format!("{step}: {}", error.message()),
        )
    }

    /// 送给读数/调用方的 `cpal::Error`（种类按 HRESULT 分类，见 [`error_kind`]）。
    fn to_cpal(&self) -> cpal::Error {
        cpal::Error::with_message(
            error_kind(self.code),
            format!("{}（HRESULT 0x{:08X}）", self.message, self.code),
        )
    }
}

/// HRESULT ⇒ `cpal::ErrorKind`（读数与端点看护共用一份分类）。
///
/// 映射口径：**会让这条流失去意义**的三类（设备变了 / 设备没了 / 流被作废）必须
/// 落在 `2|3|9` 上，否则 [`BackendErrorLog::endpoint_losses`] 读不到失效、界面侧
/// 的 1 Hz 看护就不会换流。
fn error_kind(code: u32) -> cpal::ErrorKind {
    match code {
        RPC_E_CHANGED_MODE => cpal::ErrorKind::DeviceChanged,
        AUDCLNT_E_DEVICE_INVALIDATED => cpal::ErrorKind::StreamInvalidated,
        ERROR_NOT_FOUND
        | AUDCLNT_E_SERVICE_NOT_RUNNING
        | AUDCLNT_E_NOT_INITIALIZED
        | AUDCLNT_E_ENDPOINT_CREATE_FAILED => cpal::ErrorKind::DeviceNotAvailable,
        AUDCLNT_E_UNSUPPORTED_FORMAT => cpal::ErrorKind::UnsupportedConfig,
        _ => cpal::ErrorKind::BackendError,
    }
}

/// 这个 HRESULT 的重建判决。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Retry {
    /// 值得丢掉枚举器、新建一个再来一次（端点还没回来 / 枚举器已失效 / 服务刚起）。
    Fresh,
    /// 重建解决不了（格式不支持、参数非法……）⇒ 立刻放弃，把原因交出去。
    GiveUp,
}

/// 见 [`Retry`]。**纯函数**：判决口径可逐条单测，不碰任何设备。
fn retry_kind(code: u32) -> Retry {
    match code {
        RPC_E_CHANGED_MODE
        | AUDCLNT_E_DEVICE_INVALIDATED
        | AUDCLNT_E_SERVICE_NOT_RUNNING
        | AUDCLNT_E_NOT_INITIALIZED
        | AUDCLNT_E_ENDPOINT_CREATE_FAILED
        | ERROR_NOT_FOUND => Retry::Fresh,
        _ => Retry::GiveUp,
    }
}

/// 交给渲染线程的一块**设备**缓冲格式（引擎侧恒 f32，这里是设备侧的落点）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeviceFormat {
    /// IEEE float 32（Windows 共享模式的常态）。
    Float32,
    /// 有符号 16 位 PCM。
    Pcm16,
    /// 24 位 PCM，样本紧排（每样本 3 字节）。
    Pcm24Packed,
    /// 32 位容器里的 24 位 PCM（低位补零）。
    Pcm24In32,
    /// 有符号 32 位 PCM。
    Pcm32,
}

impl DeviceFormat {
    /// 从 `WAVEFORMATEX`（可能是 `WAVEFORMATEXTENSIBLE`）读出格式。
    ///
    /// 不认识的组合 ⇒ `None`（调用方报 [`DeviceError::UnsupportedSampleFormat`]：
    /// 响亮拒绝，⛔ 不静默按 f32 写 —— 那会把 16 位设备喂成噪音）。
    fn from_wave(format: &WAVEFORMATEX) -> Option<Self> {
        // WAVE_FORMAT_EXTENSIBLE：真正的格式在 SubFormat 里。
        const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
        const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
        // KSDATAFORMAT_SUBTYPE_*（稳定 GUID，写在本地免得引两个只为常量的 feature）。
        const SUBTYPE_PCM: u128 = 0x0000_0001_0000_0010_8000_00aa_0038_9b71;
        const SUBTYPE_IEEE_FLOAT: u128 = 0x0000_0003_0000_0010_8000_00aa_0038_9b71;

        let (tag, bits, valid_bits) = if format.wFormatTag == WAVE_FORMAT_EXTENSIBLE {
            // SAFETY: `WAVEFORMATEXTENSIBLE` 以 `WAVEFORMATEX` 开头（Win32 契约），
            // 这一块内存来自 `GetMixFormat`/我们自己的 `extensible_for`，长度足够。
            let ext = unsafe { &*(format as *const WAVEFORMATEX).cast::<WAVEFORMATEXTENSIBLE>() };
            // SAFETY: `Samples` 是 POD 联合体，读 `wValidBitsPerSample` 不需要初始化任何东西。
            let valid = unsafe { ext.Samples.wValidBitsPerSample };
            (ext.Format.wFormatTag, ext.Format.wBitsPerSample, valid)
        } else {
            (
                format.wFormatTag,
                format.wBitsPerSample,
                format.wBitsPerSample,
            )
        };

        let subtype = if format.wFormatTag == WAVE_FORMAT_EXTENSIBLE {
            // SAFETY: 同上 —— 前面已确认这是 `WAVEFORMATEXTENSIBLE`。
            let ext = unsafe { &*(format as *const WAVEFORMATEX).cast::<WAVEFORMATEXTENSIBLE>() };
            // ⚠ `WAVEFORMATEXTENSIBLE` 是 `packed(1)`：`SubFormat` 的地址可能不对齐，
            // 所以不能取引用（E0793），只能按值读。
            // SAFETY: `ext` 已确认是 `WAVEFORMATEXTENSIBLE`（长度足够），
            // `read_unaligned` 允许任意对齐；`GUID` 是 POD，按值读不产生悬挂引用。
            unsafe { std::ptr::addr_of!(ext.SubFormat).read_unaligned() }.to_u128()
        } else if tag == WAVE_FORMAT_IEEE_FLOAT {
            SUBTYPE_IEEE_FLOAT
        } else {
            SUBTYPE_PCM
        };
        if subtype == SUBTYPE_IEEE_FLOAT {
            return (bits == 32).then_some(Self::Float32);
        }
        if subtype != SUBTYPE_PCM {
            return None;
        }
        match (bits, valid_bits) {
            (16, _) => Some(Self::Pcm16),
            (24, _) => Some(Self::Pcm24Packed),
            (32, 24) => Some(Self::Pcm24In32),
            (32, _) => Some(Self::Pcm32),
            _ => None,
        }
    }

    /// 每样本字节数。
    const fn bytes(self) -> usize {
        match self {
            Self::Float32 | Self::Pcm32 | Self::Pcm24In32 => 4,
            Self::Pcm24Packed => 3,
            Self::Pcm16 => 2,
        }
    }
}

/// 一台输出设备的混音格式（引擎侧即按它协商/转换）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MixFormat {
    channels: u16,
    sample_rate: u32,
    block_align: u16,
    format: DeviceFormat,
}

/// 打开一条流要用的**目标**（`probe` 的产物：设备身份 + 混音格式 + 可协商范围）。
pub(super) struct Target {
    /// 设备显示名（整机里可能重名，选择政策见 [`pick_named_device_index`]）。
    pub(super) device_name: String,
    /// 设备稳定标识（`"wasapi:{...}"`；读不到时 `None`）。
    pub(super) device_id: Option<String>,
    /// 混音格式的通道数。
    pub(super) channels: u16,
    /// 混音格式的采样率。
    pub(super) sample_rate: u32,
    /// 合成出来的唯一一条可协商范围（见 [`supported_range`]）。
    pub(super) supported: Vec<SupportedStreamConfigRange>,
    /// 设备侧的样本落点（引擎恒 f32，这里做最后一步换算）。
    mix: MixFormat,
}

/// 一条共享模式流的**可协商范围**：只有设备当前的混音格式这一条。
///
/// 为什么只有一条：共享模式下 `Initialize` 只接受混音格式（换格式要独占模式），
/// 所以"设备支持什么"的诚实答案就是这一个点。缓冲区间报 `Unknown` ——
/// 块长由 WASAPI 的周期决定，我们**不**声称能按请求的帧数切。
fn supported_range(mix: &MixFormat) -> SupportedStreamConfigRange {
    SupportedStreamConfigRange::new(
        mix.channels,
        mix.sample_rate,
        mix.sample_rate,
        SupportedBufferSize::Unknown,
        SampleFormat::F32,
    )
}

/// 把一个设备读成 `(显示名, id, 混音格式)`。读不到混音格式 ⇒ `None`（该设备跳过）。
fn describe_device(device: &IMMDevice) -> (String, Option<String>, Option<MixFormat>) {
    let id = device_id(device).map(|id| format!("wasapi:{id}"));
    let name = friendly_name(device).unwrap_or_else(|| id.clone().unwrap_or_default());
    let mix = act(device).and_then(|client| mix_format_of(&client)).ok();
    (name, id, mix)
}

/// 对新枚举器取一台设备并 `Activate` 出 `IAudioClient`（`probe` 与 `enumerate` 共用）。
fn act(device: &IMMDevice) -> Result<IAudioClient, WasapiError> {
    // SAFETY: `Activate` 只读设备对象并写出一个接口指针；`CLSCTX_ALL` 是此处的标准
    // 取值；`pactivationparams = None` 表示不要激活参数（Win32 允许空）。
    unsafe { device.Activate::<IAudioClient>(CLSCTX_ALL, None) }
        .map_err(|error| WasapiError::from_windows("IMMDevice::Activate(IAudioClient)", &error))
}

/// 读设备的混音格式（`GetMixFormat` 的内存由 COM 分配 ⇒ 必须 `CoTaskMemFree`）。
fn mix_format_of(client: &IAudioClient) -> Result<MixFormat, WasapiError> {
    // SAFETY: 返回的指针归调用方（`CoTaskMemFree`），下面立刻拷贝出想要的字段再释放。
    let raw = unsafe { client.GetMixFormat() }
        .map_err(|error| WasapiError::from_windows("IAudioClient::GetMixFormat", &error))?;
    if raw.is_null() {
        return Err(WasapiError::new(0, "IAudioClient::GetMixFormat 返回空指针"));
    }
    // SAFETY: 非空且指向一块有效的 `WAVEFORMATEX`（Win32 契约）。
    let wave = unsafe { &*raw };
    // 先把要用的数字拷出来：`CoTaskMemFree` 之后**不能**再碰 `wave`，而错误文案
    // 恰恰要用这几个数（`GetMixFormat` 的缓冲归调用方，这是最容易写错的一处）。
    let channels = wave.nChannels;
    let sample_rate = wave.nSamplesPerSec;
    let block_align = wave.nBlockAlign;
    let bits = wave.wBitsPerSample;
    let tag = wave.wFormatTag;
    let format = DeviceFormat::from_wave(wave);
    // SAFETY: 与上面 `GetMixFormat` 的分配配对；之后再不解引用 `raw`。
    unsafe { CoTaskMemFree(Some(raw.cast())) };
    match format {
        Some(format) => Ok(MixFormat {
            channels,
            sample_rate,
            block_align,
            format,
        }),
        None => Err(WasapiError::new(
            AUDCLNT_E_UNSUPPORTED_FORMAT,
            format!(
                "设备混音格式不是可写的形态（{channels}ch @ {sample_rate} Hz, {bits} bit, tag {tag}）"
            ),
        )),
    }
}

/// 设备稳定标识（`GetId` 的内存同样归调用方）。
fn device_id(device: &IMMDevice) -> Option<String> {
    // SAFETY: 返回的宽字符串由 COM 分配，下面是"取字符串 → 释放"的标准两步。
    let raw = unsafe { device.GetId() }.ok()?;
    if raw.is_null() {
        return None;
    }
    // SAFETY: `GetId` 保证返回以 NUL 结尾的宽字符串。
    let text = unsafe { raw.to_string() }.ok();
    // SAFETY: 与 `GetId` 的分配配对。
    unsafe { CoTaskMemFree(Some(raw.0.cast())) };
    text
}

/// 设备显示名（`PKEY_Device_FriendlyName`）。读不到 ⇒ `None`（调用方回退到 id）。
fn friendly_name(device: &IMMDevice) -> Option<String> {
    // SAFETY: `OpenPropertyStore` 只要一个访问模式；返回的接口由 windows crate 管引用计数。
    let store: IPropertyStore = unsafe { device.OpenPropertyStore(STGM_READ) }.ok()?;
    // SAFETY: `GetValue` 返回一个由调用方清理的 `PROPVARIANT`（下面 `PropVariantClear`）。
    let mut value = unsafe { store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME) }.ok()?;
    // SAFETY: 读 `PROPVARIANT` 的 `vt` 判别字段：先判类型再读联合体，避免把别名当指针解引用。
    let text = unsafe {
        let inner = &*value.Anonymous.Anonymous;
        if inner.vt.0 != VT_LPWSTR {
            None
        } else {
            let ptr = inner.Anonymous.pwszVal;
            if ptr.is_null() {
                None
            } else {
                ptr.to_string().ok()
            }
        }
    };
    // SAFETY: 与上面 `GetValue` 的分配配对（释放 `pwszVal` 指向的字符串）。
    unsafe { PropVariantClear(&mut value) }.ok();
    text
}

/// 新建一个 `IMMDeviceEnumerator`（**每次调用都是新对象**，绝不缓存）。
fn create_enumerator() -> Result<IMMDeviceEnumerator, WasapiError> {
    // SAFETY: `MMDeviceEnumerator` 是进程内 COM 类的稳定 CLSID；`CLSCTX_ALL` 允许
    // 进程内/外的实现；`punkouter = None` 表示不聚合。
    unsafe { CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) }
        .map_err(|error| WasapiError::from_windows("CoCreateInstance(MMDeviceEnumerator)", &error))
}

/// 枚举当前**在场**的输出端点（`DEVICE_STATE_ACTIVE`）。
fn endpoints(enumerator: &IMMDeviceEnumerator) -> Result<Vec<IMMDevice>, WasapiError> {
    // SAFETY: 只读枚举器；返回的集合接口由 windows crate 管引用计数。
    let collection = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }
        .map_err(|error| WasapiError::from_windows("EnumAudioEndpoints", &error))?;
    // SAFETY: 同上。
    let count = unsafe { collection.GetCount() }
        .map_err(|error| WasapiError::from_windows("IMMDeviceCollection::GetCount", &error))?;
    let mut out = Vec::with_capacity(count as usize);
    for index in 0..count {
        // SAFETY: `index < GetCount()`，这是集合契约要求的合法下标。
        match unsafe { collection.Item(index) } {
            Ok(device) => out.push(device),
            // 单台设备读不出来不该让整次枚举失败（与 cpal 腿"单设备失败跳过"同口径）。
            Err(_) => continue,
        }
    }
    Ok(out)
}

/// 当前默认输出端点的 id（`"wasapi:{...}"`）。读不到 ⇒ `None`。
fn current_default_id(enumerator: &IMMDeviceEnumerator) -> Option<String> {
    // SAFETY: 只读枚举器；返回的设备接口由 windows crate 管引用计数。
    let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }.ok()?;
    device_id(&device).map(|id| format!("wasapi:{id}"))
}

/// 枚举全部输出设备（[`super::enumerate_output_devices`] 的 Windows 本体）。
pub(super) fn enumerate_outputs() -> Result<Vec<DeviceInfo>, DeviceError> {
    let enumerator = create_enumerator().map_err(backend)?;
    let default_id = current_default_id(&enumerator);
    let devices = endpoints(&enumerator).map_err(backend)?;
    let mut out = Vec::with_capacity(devices.len());
    for device in &devices {
        let (name, id, mix) = describe_device(device);
        let Some(mix) = mix else { continue };
        out.push(DeviceInfo {
            name,
            is_default: id.is_some() && id == default_id,
            id: id.unwrap_or_default(),
            channels: mix.channels,
            sample_rate: mix.sample_rate,
            // 引擎侧恒按 f32 渲染；设备侧的落点见 `DeviceFormat`（T-FIX-409）。
            sample_format: SampleFormat::F32,
            supported: describe(&[supported_range(&mix)]),
        });
    }
    Ok(out)
}

/// 读默认设备（或按名选一台）的混音格式与范围，供协商用（[`super::open_output`] 的第一步）。
pub(super) fn probe(want_name: Option<&str>) -> Result<Target, DeviceError> {
    let enumerator = create_enumerator().map_err(backend)?;
    let device = match want_name {
        None => {
            // SAFETY: 只读枚举器；`eRender/eConsole` 是"输出/控制台角色"的标准组合。
            unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }.map_err(|error| {
                let error = WasapiError::from_windows("GetDefaultAudioEndpoint", &error);
                if error.code == ERROR_NOT_FOUND {
                    DeviceError::NoDefaultOutputDevice
                } else {
                    backend(error)
                }
            })?
        }
        Some(want) => {
            let devices = endpoints(&enumerator).map_err(backend)?;
            let names: Vec<String> = devices
                .iter()
                .map(|device| describe_device(device).0)
                .collect();
            // 选择政策与 cpal 腿**同一份**（`pick_named_device_index`：精确相等、取首个）。
            let index = pick_named_device_index(&names, want).ok_or_else(|| {
                DeviceError::NamedOutputDeviceUnavailable {
                    name: want.to_owned(),
                }
            })?;
            devices.into_iter().nth(index).ok_or_else(|| {
                DeviceError::NamedOutputDeviceUnavailable {
                    name: want.to_owned(),
                }
            })?
        }
    };
    let client = act(&device).map_err(backend)?;
    let mix = mix_format_of(&client).map_err(backend)?;
    let (device_name, device_id, _) = describe_device(&device);
    Ok(Target {
        device_name,
        device_id,
        channels: mix.channels,
        sample_rate: mix.sample_rate,
        supported: vec![supported_range(&mix)],
        mix,
    })
}

/// [`super::trace_output_negotiation`] 的 Windows 本体：与打开路径同一个 `probe`，
/// 所以 trace 里写的设备与真正会被打开的设备**必然**一致。
pub(super) fn trace_output(want: &EngineConfig) -> Vec<String> {
    let mut lines = vec!["trace-output: backend wasapi (self-built, T-FIX-409)".to_owned()];
    let target = match probe(None) {
        Ok(target) => target,
        Err(error) => {
            lines.push(format!("trace-output: probe failed: {error}"));
            return lines;
        }
    };
    lines.push(format!(
        "trace-output: device {} ({} ch @ {} Hz, {} bit)",
        target.device_name,
        target.channels,
        target.sample_rate,
        target.mix.format.bytes() * 8
    ));
    let (_, trace) = negotiate_traced(&target.supported, want);
    lines.extend(trace);
    lines
}

/// 端点健康判决的 Windows 本体：与 cpal 腿同一个纯判决函数（[`super::classify_endpoint`]），
/// 只有"此刻的读数"来源换成了自己的枚举器。
pub(super) fn poll_endpoint(opened_id: Option<&str>, opened_default: bool) -> EndpointHealth {
    let Ok(enumerator) = create_enumerator() else {
        return EndpointHealth::Unknown;
    };
    if opened_default {
        let current = current_default_id(&enumerator);
        return super::classify_endpoint(opened_id, true, current.as_deref(), true, None);
    }
    let present = match opened_id {
        Some(want) => endpoints(&enumerator)
            .map(|devices| {
                devices
                    .iter()
                    .filter_map(|device| device_id(device).map(|id| format!("wasapi:{id}")))
                    .any(|id| id == want)
            })
            // 枚举被拒 ⇒ 读不出来，按"仍在"处理（与 cpal 腿 `.unwrap_or(true)` 同口径）。
            .unwrap_or(true),
        None => true,
    };
    super::classify_endpoint(opened_id, false, None, present, None)
}

/// 把 `WasapiError` 抬成 [`DeviceError::Backend`]。
fn backend(error: WasapiError) -> DeviceError {
    DeviceError::Backend(error.to_cpal())
}

/// 一条**活着的**流：`IAudioClient` + 渲染客户端 + 设备格式与暂存。
///
/// ⚠ 只在渲染线程上创建/使用/析构（COM 对象不跨线程搬运）。
struct Active {
    client: IAudioClient,
    render: IAudioRenderClient,
    buffer_frames: u32,
    format: DeviceFormat,
    channels: u16,
    /// f32 暂存（构造期分配一次，稳态零分配）。等长于 `buffer_frames × channels`。
    scratch: Vec<f32>,
}

impl Active {
    /// 用**新枚举器**建一条流（`audiosrv` 重启后的重建也走这里，见 [`StreamThread::rebuild`]）。
    fn open(target: &Target, event: HANDLE) -> Result<Self, WasapiError> {
        let enumerator = create_enumerator()?;
        let device = match &target.device_id {
            Some(id) => {
                let wide: Vec<u16> = id
                    .strip_prefix("wasapi:")
                    .unwrap_or(id)
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                // SAFETY: `wide` 是本次调用期间有效的 NUL 结尾宽字符串。
                unsafe { enumerator.GetDevice(PCWSTR(wide.as_ptr())) }.map_err(|error| {
                    WasapiError::from_windows("IMMDeviceEnumerator::GetDevice", &error)
                })?
            }
            // SAFETY: 只读枚举器。
            None => unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
                .map_err(|error| WasapiError::from_windows("GetDefaultAudioEndpoint", &error))?,
        };
        let client = act(&device)?;
        // 重建时混音格式可能变（用户把设备切到别的率）：如实报告差异，别硬按旧格式建。
        let mix = mix_format_of(&client)?;
        if mix != target.mix {
            return Err(WasapiError::new(
                AUDCLNT_E_UNSUPPORTED_FORMAT,
                format!(
                    "设备混音格式在重建前后不一致（打开时 {}ch/{}Hz/{}B，现在 {}ch/{}Hz/{}B）",
                    target.mix.channels,
                    target.mix.sample_rate,
                    target.mix.format.bytes(),
                    mix.channels,
                    mix.sample_rate,
                    mix.format.bytes()
                ),
            ));
        }
        let wave = extensible_for(&mix);
        // SAFETY: `wave` 是本地构造、按 Win32 契约填好的 `WAVEFORMATEXTENSIBLE`
        // （以 `WAVEFORMATEX` 开头）；`Initialize` 只读它。
        unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                BUFFER_DURATION,
                0,
                (&raw const wave).cast::<WAVEFORMATEX>(),
                None,
            )
        }
        .map_err(|error| WasapiError::from_windows("IAudioClient::Initialize", &error))?;
        // SAFETY: 事件句柄由本线程创建、生命周期覆盖整条流。
        unsafe { client.SetEventHandle(event) }
            .map_err(|error| WasapiError::from_windows("IAudioClient::SetEventHandle", &error))?;
        // SAFETY: 取同一间公寓里的服务接口。
        let render: IAudioRenderClient = unsafe { client.GetService() }
            .map_err(|error| WasapiError::from_windows("IAudioClient::GetService", &error))?;
        // SAFETY: 只读 client 状态。
        let buffer_frames = unsafe { client.GetBufferSize() }
            .map_err(|error| WasapiError::from_windows("IAudioClient::GetBufferSize", &error))?;
        let samples = buffer_frames as usize * usize::from(mix.channels);
        Ok(Self {
            client,
            render,
            buffer_frames,
            format: mix.format,
            channels: mix.channels,
            scratch: vec![0.0; samples],
        })
    }

    /// 填一次缓冲。返回本次写出的**帧**数（0 = 此刻没有空位）。
    ///
    /// 实时路径：零分配、零锁、零 I/O、零日志。
    fn pump(
        &mut self,
        fill: &mut dyn FnMut(&mut [f32], u16, u64),
        gap: &mut GapClock,
    ) -> Result<u32, WasapiError> {
        // SAFETY: 只读 client 状态。
        let padding = unsafe { self.client.GetCurrentPadding() }.map_err(|error| {
            WasapiError::from_windows("IAudioClient::GetCurrentPadding", &error)
        })?;
        let frames = self.buffer_frames.saturating_sub(padding);
        if frames == 0 {
            return Ok(0);
        }
        let samples = frames as usize * usize::from(self.channels);
        // SAFETY: `scratch` 的长度按 `buffer_frames × channels` 分配，`frames ≤ buffer_frames`
        // ⇒ `samples` 必定在界内。
        let slice = &mut self.scratch[..samples];
        fill(
            slice,
            self.channels,
            gap.take(samples as u64 / u64::from(self.channels), self.channels),
        );
        // SAFETY: 申请 `frames` 帧的写指针，写完立刻 `ReleaseBuffer(frames)`；
        // 指针只在本次调用内使用。
        let dst = unsafe { self.render.GetBuffer(frames) }
            .map_err(|error| WasapiError::from_windows("IAudioRenderClient::GetBuffer", &error))?;
        write_frames(slice, dst, self.format);
        // SAFETY: 与 `GetBuffer(frames)` 配对，帧数一致。
        unsafe { self.render.ReleaseBuffer(frames, 0) }.map_err(|error| {
            WasapiError::from_windows("IAudioRenderClient::ReleaseBuffer", &error)
        })?;
        Ok(frames)
    }

    /// 启流。
    fn start(&self) -> Result<(), WasapiError> {
        // SAFETY: 只写 client 状态。
        unsafe { self.client.Start() }
            .map_err(|error| WasapiError::from_windows("IAudioClient::Start", &error))
    }

    /// 停流（失败只记读数，不改变控制流）。
    fn stop(&self) -> Result<(), WasapiError> {
        // SAFETY: 只写 client 状态。
        unsafe { self.client.Stop() }
            .map_err(|error| WasapiError::from_windows("IAudioClient::Stop", &error))
    }
}

/// 按 [`MixFormat`] 造一个 `WAVEFORMATEXTENSIBLE`（`Initialize` 的入参形态）。
///
/// 共享模式下用 `WAVEFORMATEXTENSIBLE` 最稳：设备报回来的混音格式本来就是它，
/// 直接按同样的形态递回去，`Initialize` 不需要做"最接近格式"的替换。
fn extensible_for(mix: &MixFormat) -> WAVEFORMATEXTENSIBLE {
    const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
    const SUBTYPE_PCM: u128 = 0x0000_0001_0000_0010_8000_00aa_0038_9b71;
    const SUBTYPE_IEEE_FLOAT: u128 = 0x0000_0003_0000_0010_8000_00aa_0038_9b71;

    let bytes = mix.format.bytes() as u16;
    let bits = bytes * 8;
    let valid_bits = match mix.format {
        DeviceFormat::Float32 | DeviceFormat::Pcm32 => bits,
        // 32 位容器里的 24 位：容器 32、有效位 24。
        DeviceFormat::Pcm24In32 => 24,
        // 紧排的 24 位：容器就是 24。
        DeviceFormat::Pcm24Packed => 24,
        DeviceFormat::Pcm16 => 16,
    };
    let block_align = bytes.saturating_mul(mix.channels);
    let subtype = if mix.format == DeviceFormat::Float32 {
        SUBTYPE_IEEE_FLOAT
    } else {
        SUBTYPE_PCM
    };
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE,
            nChannels: mix.channels,
            nSamplesPerSec: mix.sample_rate,
            nAvgBytesPerSec: mix.sample_rate.saturating_mul(u32::from(block_align)),
            nBlockAlign: block_align,
            wBitsPerSample: bits,
            cbSize: 22,
        },
        Samples: windows::Win32::Media::Audio::WAVEFORMATEXTENSIBLE_0 {
            wValidBitsPerSample: valid_bits,
        },
        dwChannelMask: 0,
        SubFormat: windows::core::GUID::from_u128(subtype),
    }
}

/// 把 f32 交错样本写成设备格式（含钳位）。
///
/// 这是"引擎恒 f32、设备可能是 16/24/32 位"的唯一换算点：⛔ 别在别处再写一份。
fn write_frames(samples: &[f32], dst: *mut u8, format: DeviceFormat) {
    #[inline]
    fn clamp(value: f32) -> f32 {
        if value.is_nan() {
            0.0
        } else {
            value.clamp(-1.0, 1.0)
        }
    }
    match format {
        DeviceFormat::Float32 => {
            // SAFETY: WASAPI 交给我们的缓冲按 `nBlockAlign` 对齐（≥ 4 字节），
            // 长度不小于 `samples.len() × 4`；两边都是 f32 且不重叠。
            unsafe {
                std::ptr::copy_nonoverlapping(samples.as_ptr(), dst.cast::<f32>(), samples.len())
            };
        }
        DeviceFormat::Pcm16 => {
            for (index, sample) in samples.iter().enumerate() {
                let value = (clamp(*sample) * 32_767.0).round() as i16;
                // SAFETY: 缓冲区按样本紧排、足够放下 `samples.len()` 个 i16。
                unsafe { dst.cast::<i16>().add(index).write_unaligned(value) };
            }
        }
        DeviceFormat::Pcm24In32 => {
            for (index, sample) in samples.iter().enumerate() {
                let value = ((clamp(*sample) * 8_388_607.0).round() as i32) << 8;
                // SAFETY: 同上，按 i32 槽写（低位补零 = 24 位有效）。
                unsafe { dst.cast::<i32>().add(index).write_unaligned(value) };
            }
        }
        DeviceFormat::Pcm32 => {
            for (index, sample) in samples.iter().enumerate() {
                let value = (f64::from(clamp(*sample)) * 2_147_483_647.0).round() as i32;
                // SAFETY: 同上。
                unsafe { dst.cast::<i32>().add(index).write_unaligned(value) };
            }
        }
        DeviceFormat::Pcm24Packed => {
            for (index, sample) in samples.iter().enumerate() {
                let value = (clamp(*sample) * 8_388_607.0).round() as i32;
                let bytes = value.to_le_bytes();
                // SAFETY: 每样本 3 字节紧排，缓冲区足够。
                unsafe {
                    let slot = dst.add(index * 3);
                    slot.write(bytes[0]);
                    slot.add(1).write(bytes[1]);
                    slot.add(2).write(bytes[2]);
                }
            }
        }
    }
}

/// 回调间隔的观测器（等价的"上一次填充时刻"）。
///
/// 为什么不用 [`super::CallbackGapObserver`]：那个观测器吃 `cpal::StreamInstant`，
/// 而自研后端没有 cpal 的时间戳对象可造。跳跃的**口径**仍走同一个纯函数
/// （[`super::callback_gap_frames_for`]），这里只补"从墙上时钟取间隔"这一跳。
struct GapClock {
    last: Option<Instant>,
    sample_rate: u32,
}

impl GapClock {
    const fn new(sample_rate: u32) -> Self {
        Self {
            last: None,
            sample_rate,
        }
    }

    /// 记一次填充，返回"这次比上次该到的时刻晚了多少帧"。
    fn take(&mut self, frames: u64, channels: u16) -> u64 {
        let _ = channels;
        let now = Instant::now();
        let elapsed = self.last.map(|last| now.saturating_duration_since(last));
        self.last = Some(now);
        super::callback_gap_frames_for(elapsed, frames as usize, self.sample_rate)
    }

    /// 重建之后重新起算（否则跨重建的那段静音会被记成一次巨大的 xrun）。
    fn reset(&mut self) {
        self.last = None;
    }
}

/// 渲染线程收到的命令。
enum Command {
    /// 启流 + 回执。
    Play(Sender<Result<(), DeviceError>>),
    /// 停流 + 回执。
    Pause(Sender<Result<(), DeviceError>>),
    /// 收尾：线程就地析构流后退出。
    Shutdown,
}

/// 一条输出流在调用方手里的形态：命令通道 + 渲染线程句柄。
///
/// `Send`：句柄只有通道与 `JoinHandle`，COM 对象全在渲染线程上。
pub(super) struct StreamHandle {
    commands: Sender<Command>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl StreamHandle {
    /// 启流。
    pub(super) fn play(&self) -> Result<(), DeviceError> {
        self.request(Command::Play)
    }

    /// 停流。
    pub(super) fn pause(&self) -> Result<(), DeviceError> {
        self.request(Command::Pause)
    }

    fn request(
        &self,
        make: fn(Sender<Result<(), DeviceError>>) -> Command,
    ) -> Result<(), DeviceError> {
        let (ack_tx, ack_rx) = mpsc::channel();
        self.commands
            .send(make(ack_tx))
            .map_err(|_| DeviceError::OwnerThread {
                reason: "WASAPI 渲染线程已退出（命令发不出去）".to_owned(),
            })?;
        ack_rx.recv().map_err(|_| DeviceError::OwnerThread {
            reason: "WASAPI 渲染线程在回执前退出".to_owned(),
        })?
    }
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        // 渲染线程的阻塞点最长是一次 `WAIT_SLICE`（或重建退避），join 有界。
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// 回调的填充函数：`(缓冲, 通道数, 跳跃帧数)`。
pub(super) type Fill = Box<dyn FnMut(&mut [f32], u16, u64) + Send>;

/// 打开一条共享模式输出流（Windows 输出腿的唯一入口）。
///
/// 首次建流在**渲染线程**上完成：失败时把错误原样回执给调用方（⛔ 不返回一个
/// "看起来打开了、其实没声"的句柄）。
pub(super) fn open(
    target: Target,
    fill: Fill,
    errors: Arc<BackendErrorLog>,
) -> Result<StreamHandle, DeviceError> {
    let (commands_tx, commands_rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), DeviceError>>();
    let worker = std::thread::Builder::new()
        .name("yeban-wasapi-render".to_owned())
        .spawn(move || run(target, fill, errors, commands_rx, ready_tx))
        .map_err(|error| DeviceError::OwnerThread {
            reason: format!("WASAPI 渲染线程起不来：{error}"),
        })?;
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(StreamHandle {
            commands: commands_tx,
            worker: Some(worker),
        }),
        Ok(Err(error)) => {
            let _ = worker.join();
            Err(error)
        }
        Err(_) => {
            let _ = worker.join();
            Err(DeviceError::OwnerThread {
                reason: "WASAPI 渲染线程在报告结果前退出".to_owned(),
            })
        }
    }
}

/// 渲染线程主体：定公寓 → 建流 → 回执 → 循环（命令 / 填充 / 重建）。
fn run(
    target: Target,
    mut fill: Fill,
    errors: Arc<BackendErrorLog>,
    commands: Receiver<Command>,
    ready: Sender<Result<(), DeviceError>>,
) {
    let _apartment = match super::apartment::Apartment::bind() {
        Ok(guard) => guard,
        Err(reason) => {
            let _ = ready.send(Err(DeviceError::OwnerThread { reason }));
            return;
        }
    };
    // SAFETY: 创建一条本线程独占、手动不重置的自动事件；名字为空。
    let event = match unsafe { CreateEventW(None, false, false, None) } {
        Ok(event) => event,
        Err(error) => {
            let error = WasapiError::from_windows("CreateEventW", &error);
            let _ = ready.send(Err(backend(error)));
            return;
        }
    };
    let mut thread = StreamThread {
        target,
        event,
        active: None,
        started: false,
        stopping: false,
        errors,
    };
    // 首次建流：等待上限 5 s（与 cpal 腿的 `STREAM_OPEN_TIMEOUT` 同量级）。
    match thread.open_with_retry(Some(Instant::now() + OPEN_BUDGET), &commands) {
        Ok(()) => {
            let _ = ready.send(Ok(()));
        }
        Err(error) => {
            let _ = ready.send(Err(backend(error)));
            // SAFETY: 与上面的 `CreateEventW` 配对。
            unsafe { CloseHandle(event) }.ok();
            return;
        }
    }
    thread.serve(&mut fill, &commands);
    // SAFETY: 与上面的 `CreateEventW` 配对。
    unsafe { CloseHandle(event) }.ok();
}

/// 渲染线程的运行状态（不含线程本身，便于把"重建"写成可读的一段）。
struct StreamThread {
    target: Target,
    event: HANDLE,
    active: Option<Active>,
    started: bool,
    /// 收到 `Shutdown`（或命令通道断了）⇒ 主循环必须退出。
    ///
    /// ⚠ 这个字段存在的唯一原因：`Shutdown` 可能落在**重建的退避循环里**被消费掉
    /// （那时主循环还没轮到 `drain_commands`）。没有它，`StreamHandle::drop` 的
    /// `join` 会一直等到端点回来为止 —— 实测就是这么挂住 60 s 以上的。
    stopping: bool,
    errors: Arc<BackendErrorLog>,
}

impl StreamThread {
    /// 主循环：先排命令，再按"已启流"决定是等音频事件还是等命令。
    fn serve(&mut self, fill: &mut Fill, commands: &Receiver<Command>) {
        let mut gap = GapClock::new(self.target.sample_rate);
        let mut last_fill = Instant::now();
        loop {
            if self.stopping {
                break;
            }
            match self.drain_commands(commands) {
                Control::Continue => {}
                Control::Stop => break,
            }
            if !self.started || self.active.is_none() {
                // 未启流 / 正在等端点回来：把 CPU 让出去，命令的响应上限 = IDLE_TICK。
                match commands.recv_timeout(IDLE_TICK) {
                    Ok(command) => {
                        if let Control::Stop = self.handle_command(command) {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
                continue;
            }
            // SAFETY: `event` 由本线程创建、尚未关闭。
            let wait = unsafe { WaitForSingleObject(self.event, WAIT_SLICE.as_millis() as u32) };
            if wait == WAIT_TIMEOUT {
                if last_fill.elapsed() >= STALL_LIMIT {
                    // 事件不再来了：设备被拔 / 服务停了 / 流被作废。
                    self.on_lost(commands, "音频事件长时间不来（设备失效）");
                    if self.stopping {
                        break;
                    }
                    if self.started {
                        gap.reset();
                        last_fill = Instant::now();
                    }
                }
                continue;
            }
            if wait != WAIT_OBJECT_0 {
                self.on_lost(commands, "WaitForSingleObject 失败（事件句柄失效）");
                if self.stopping {
                    break;
                }
                gap.reset();
                last_fill = Instant::now();
                continue;
            }
            let Some(active) = self.active.as_mut() else {
                continue;
            };
            match active.pump(fill.as_mut(), &mut gap) {
                Ok(0) => {}
                Ok(_) => last_fill = Instant::now(),
                Err(error) => {
                    self.errors.record(&error.to_cpal());
                    let text = format!(
                        "填充失败：{}（HRESULT 0x{:08X}）",
                        error.message, error.code
                    );
                    self.on_lost(commands, &text);
                    if self.stopping {
                        break;
                    }
                    gap.reset();
                    last_fill = Instant::now();
                }
            }
        }
        self.shutdown();
    }

    /// 把排着的命令执行掉；`Stop` = 收到 `Shutdown` 或通道断开。
    fn drain_commands(&mut self, commands: &Receiver<Command>) -> Control {
        loop {
            match commands.try_recv() {
                Ok(command) => {
                    if let Control::Stop = self.handle_command(command) {
                        return Control::Stop;
                    }
                }
                Err(TryRecvError::Empty) => return Control::Continue,
                Err(TryRecvError::Disconnected) => return Control::Stop,
            }
        }
    }

    /// 执行一条命令（启流/停流在"正在重建"时只改意图，等重建成功后再兑现）。
    fn handle_command(&mut self, command: Command) -> Control {
        match command {
            Command::Play(ack) => {
                self.started = true;
                self.errors.begin_run();
                let result = match self.active.as_ref() {
                    Some(active) => active.start().map_err(backend),
                    // 正在重建：意图已记下，重建成功那一刻会自动 `Start`。
                    None => Ok(()),
                };
                let _ = ack.send(result);
                Control::Continue
            }
            Command::Pause(ack) => {
                self.started = false;
                self.errors.begin_stop();
                let result = match self.active.as_ref() {
                    Some(active) => active.stop().map_err(backend),
                    None => Ok(()),
                };
                let _ = ack.send(result);
                Control::Continue
            }
            Command::Shutdown => {
                self.stopping = true;
                Control::Stop
            }
        }
    }

    /// 检测到失效：丢掉旧流，用**新枚举器**重建，直到端点回来（或格式变了 ⇒ 交回界面）。
    fn on_lost(&mut self, commands: &Receiver<Command>, reason: &str) {
        if let Some(active) = self.active.as_mut() {
            active.stop().ok();
        }
        self.active = None;
        // 失效这一条按"流被作废"记账：界面侧 1 Hz 看护据此知道这条流出过事。
        self.errors
            .record(&WasapiError::new(AUDCLNT_E_DEVICE_INVALIDATED, reason.to_owned()).to_cpal());
        if !self.started {
            // 没在播就没什么可重建的：下次 `play` 时再看端点。
            return;
        }
        // 重建：无预算上限（等到端点回来为止），但可被 `Shutdown` 打断。
        if let Err(error) = self.open_with_retry(None, commands) {
            self.started = false;
            if !self.stopping {
                self.errors.record(&error.to_cpal());
            }
        }
    }

    /// 建流 + 重试。`budget = None` ⇒ 一直重试（重建路径）；`Some(deadline)` ⇒ 首次打开。
    fn open_with_retry(
        &mut self,
        budget: Option<Instant>,
        commands: &Receiver<Command>,
    ) -> Result<(), WasapiError> {
        let mut pause = RETRY_FIRST;
        loop {
            match Active::open(&self.target, self.event) {
                Ok(active) => {
                    if self.started {
                        active.start()?;
                    }
                    self.active = Some(active);
                    // 这条流已经被证明能在这台端点上出声 ⇒ 之前记的"失效"读数过期，
                    // 清掉它，否则界面侧 1 Hz 看护会把这条活流当成死的、每秒重建一次。
                    self.errors.clear_endpoint_loss();
                    return Ok(());
                }
                Err(error) => {
                    self.errors.record(&error.to_cpal());
                    if retry_kind(error.code) == Retry::GiveUp {
                        return Err(error);
                    }
                    if budget.is_some_and(|deadline| Instant::now() >= deadline) {
                        return Err(error);
                    }
                    // 退避期间也要能收到 `Shutdown`/`Pause`，并把它们执行掉。
                    let until = Instant::now() + pause;
                    while Instant::now() < until {
                        match commands.try_recv() {
                            Ok(Command::Shutdown) => {
                                self.stopping = true;
                                return Err(WasapiError::new(0, "重建期间收到 Shutdown"));
                            }
                            Ok(command) => {
                                self.handle_command(command);
                                if !self.started {
                                    return Err(WasapiError::new(0, "重建期间被暂停"));
                                }
                            }
                            Err(TryRecvError::Empty) => {
                                std::thread::sleep(Duration::from_millis(10));
                            }
                            Err(TryRecvError::Disconnected) => {
                                self.stopping = true;
                                return Err(WasapiError::new(0, "句柄已丢弃"));
                            }
                        }
                    }
                    pause = (pause * 2).min(RETRY_MAX);
                }
            }
        }
    }

    /// 收尾：停流并释放设备对象（COM 对象在**本线程**析构）。
    fn shutdown(&mut self) {
        if let Some(active) = self.active.as_ref() {
            active.stop().ok();
        }
        self.active = None;
    }
}

/// 命令处理的走向。
enum Control {
    Continue,
    Stop,
}

/// **测试用**：停再启 `audiosrv`，返回这次重启本身花了多久。
///
/// 为什么由判据自己动手：这一票的判据是"**服务重启**之后流能不能自己回来"，
/// 重启时刻必须与被观察的流在同一个进程/同一条时间线上，否则"多久恢复"这个数
/// 就没有分母。251 的实验夹具靠人手 `touch` 标记文件，那种方式在
/// `cargo test -- --ignored` 里没法自动重跑；这里改成判据自己执行
/// （真机判据本来就是 `#[ignore]`，需要管理员权限这一点写在文档注释里）。
///
/// `Stop-Service -Force` 会顺带停掉依赖它的东西；`audiosrv` 的依赖方向是
/// `Audiosrv → AudioEndpointBuilder`，停它**不**会带走别的服务。等它真到
/// `Stopped` 再启（`Start-Service` 在"还在停"时会直接失败，这是最常见的坑）。
#[cfg(test)]
pub(super) fn restart_audio_service_for_tests() -> Result<Duration, String> {
    const SCRIPT: &str = "Stop-Service audiosrv -Force -ErrorAction Stop; \
         $deadline = (Get-Date).AddSeconds(20); \
         while ((Get-Service audiosrv).Status -ne 'Stopped' -and (Get-Date) -lt $deadline) { \
           Start-Sleep -Milliseconds 100 }; \
         Start-Sleep -Milliseconds 500; \
         Start-Service audiosrv -ErrorAction Stop";
    let began = std::time::Instant::now();
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .output()
        .map_err(|error| format!("起不了 powershell：{error}"))?;
    let elapsed = began.elapsed();
    if !output.status.success() {
        return Err(format!(
            "重启 audiosrv 失败（退出码 {:?}，需要管理员权限）：{}{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(elapsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// 重启之后多久算"恢复"—— 只用来给判据一个上限，不是产品口径。
    ///
    /// 为何是 30 s：251 实测的对照是"**579 s**（23 次连续失败后 Windows 侧内部
    /// 超时）"，本票的判据是"不用等那个超时"。取 30 s 是留足端点重新发布的时间
    ///（实测 ~2.4 s）又离 579 s 一个数量级，⛔ 不是"挑一个好看的数字"。
    const RECOVERY_BUDGET: Duration = Duration::from_secs(30);

    /// 一条会数帧的填充函数：判据靠"帧数有没有继续涨"判断**播放继续**。
    fn counting_fill(counter: Arc<AtomicU64>) -> Fill {
        Box::new(move |data: &mut [f32], _channels: u16, _gap: u64| {
            // 写静音即可：判据量的是"设备还在向我们要数据"，不是波形内容。
            data.fill(0.0);
            counter.fetch_add(data.len() as u64, Ordering::Relaxed);
        })
    }

    /// 在**测试线程**上绑 MTA。
    ///
    /// [`probe`] / [`enumerate_outputs`] / [`poll_endpoint`] 在生产里恒跑在
    /// [`super::super::backend`] 的 MTA 线程上（见本模块文档"线程与公寓"）；
    /// 判据直接调它们就必须自己先把公寓定下来，否则 `CoCreateInstance` 报
    /// `CO_E_NOTINITIALIZED`（`0x800401F0`）—— 这不是被测代码的缺陷。
    fn bind_mta() -> super::super::apartment::Apartment {
        super::super::apartment::Apartment::bind().expect("测试线程必须能绑上 MTA")
    }

    /// 等到帧数超过 `floor`，或超时。
    fn wait_for_frames(counter: &AtomicU64, floor: u64, budget: Duration) -> Option<u64> {
        let deadline = Instant::now() + budget;
        while Instant::now() < deadline {
            let now = counter.load(Ordering::Relaxed);
            if now > floor {
                return Some(now);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }

    /// **判据 A（headline）**：live 流在 `audiosrv` 重启后**自己**重建、播放继续，
    /// 且恢复时间离"Windows 内部超时"有一个数量级的距离。
    ///
    /// 红对照：把 [`StreamThread::on_lost`] 里的重建分支掐掉（`active = None` 之后
    /// 直接 `return`）⇒ 帧数在重启后**再也不涨**，本判据在 30 s 上限处失败。
    ///
    /// 真机判据（`#[ignore]`，需要管理员权限来停/启服务）：
    /// `cargo test -p yeban-engine --lib live_stream_keeps_playing_across_an_audio_service_restart -- --ignored --nocapture`
    #[test]
    #[ignore = "真机判据：需要管理员权限停/启 audiosrv"]
    fn live_stream_keeps_playing_across_an_audio_service_restart() {
        let _apartment = bind_mta();
        let target = match probe(None) {
            Ok(target) => target,
            Err(error) => {
                eprintln!("SKIP：读不到默认输出设备（{error}）");
                return;
            }
        };
        let counter = Arc::new(AtomicU64::new(0));
        let errors = Arc::new(BackendErrorLog::new());
        let stream = open(
            target,
            counting_fill(Arc::clone(&counter)),
            Arc::clone(&errors),
        )
        .expect("首次建流必须成功");
        stream.play().expect("启流必须成功");
        let before = wait_for_frames(&counter, 0, Duration::from_secs(5))
            .expect("启流后 5 s 内必须开始出帧");
        eprintln!("PRE-RESTART frames={before}");

        let restart = restart_audio_service_for_tests().expect("重启 audiosrv 必须成功");
        let t0 = Instant::now();
        eprintln!("RESTARTED in {} ms", restart.as_millis());
        // 「恢复」的口径 = **连续 1 s 帧数在涨，且失效读数是 `(0, 0)`**。
        //
        // ⚠ 只看"帧数涨了"会误判（本判据的第一版就是这么错的）：服务启动过程中
        // 刚重建好的流会被**再打一次**，实测 t≈1.5 s 时帧数停在 100032、
        // 读数涨到 `(6, 9)`，直到 t≈2.3 s 才真正稳住。要求"连续 1 s 稳定"把
        // 这段过渡期排除在外，报出来的数才是用户能听出来的那一刻。
        const STABLE: Duration = Duration::from_secs(1);
        let mut stable_since: Option<Instant> = None;
        let mut previous = counter.load(Ordering::Relaxed);
        let mut recovered: Option<Duration> = None;
        while t0.elapsed() < RECOVERY_BUDGET {
            std::thread::sleep(Duration::from_millis(50));
            let now = counter.load(Ordering::Relaxed);
            let growing = now > previous;
            previous = now;
            if growing && errors.endpoint_losses() == (0, 0) {
                let since = *stable_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= STABLE {
                    recovered = Some(t0.elapsed());
                    break;
                }
            } else {
                stable_since = None;
            }
        }
        let after = counter.load(Ordering::Relaxed);
        eprintln!(
            "RECOVERED after {:?} ms, frames={after}（重启前 {before}），endpoint_losses={:?}",
            recovered.map(|elapsed| elapsed.as_millis()),
            errors.endpoint_losses()
        );

        let elapsed =
            recovered.expect("重启后 30 s 内必须稳定恢复出帧（红对照：掐掉重建逻辑 ⇒ 这里失败）");
        assert!(after > before, "帧数必须真的继续涨：{before} -> {after}");
        assert!(
            elapsed < RECOVERY_BUDGET,
            "恢复耗时 {} ms 必须远小于 Windows 内部超时（约 579000 ms）",
            elapsed.as_millis()
        );
        // 重建成功那一刻会清掉"失效"读数；不清的话界面侧 1 Hz 看护会把这条
        // 正在出声的流当成死的、每秒重建一次。
        assert_eq!(
            errors.endpoint_losses(),
            (0, 0),
            "稳定之后端点失效读数必须是 (0, 0)（否则看护会反复重建活流）"
        );
        drop(stream);
    }

    /// **判据 B**：**一条活着的流跨过重启**之后，一个**新建的**枚举器仍然能
    /// `Activate`，且读到的是同一台设备、同一个混音格式。
    ///
    /// ⚠ 判据里必须先开一条流并让它活到重启之后 —— 这正是 cpal 腿失效的那个场景
    /// （251 的四组对照：只有"活着跨重启"才会把进程打进 `RPC_E_CHANGED_MODE`
    /// 状态；没有活流跨重启时，连缓存的枚举器都还能用）。本条第一版漏了这一步，
    /// 于是"缓存枚举器"这个红对照根本不红 —— 与 251 那次错红对照同一类错误。
    ///
    /// 红对照：**没有**。"把 [`create_enumerator`] 改成缓存"这条实测**不红**
    /// （见模块头「实测边界」）—— 所以本条只作为"恢复"的证据，⛔ 不作为
    /// "缓存即病因"的证据。真正咬合的红对照在判据 A（掐掉重建 ⇒ 帧数不再涨）。
    ///
    /// `cargo test -p yeban-engine --lib enumerator_rebuilt_after_service_restart_still_activates -- --ignored --nocapture`
    #[test]
    #[ignore = "真机判据：需要管理员权限停/启 audiosrv"]
    fn enumerator_rebuilt_after_service_restart_still_activates() {
        let _apartment = bind_mta();
        let before = match probe(None) {
            Ok(target) => target,
            Err(error) => {
                eprintln!("SKIP：读不到默认输出设备（{error}）");
                return;
            }
        };
        eprintln!(
            "BEFORE device={} id={:?} {}ch@{}Hz {}B",
            before.device_name,
            before.device_id,
            before.channels,
            before.sample_rate,
            before.mix.format.bytes()
        );
        // 先把一条流播起来，并让它**活着**跨过重启：这才是会把进程打进
        // `RPC_E_CHANGED_MODE` 的那个状态（没有活流跨重启时，什么都还能用）。
        let before_id = before.device_id.clone();
        let before_shape = (before.channels, before.sample_rate, before.mix.format);
        let live_counter = Arc::new(AtomicU64::new(0));
        let live_errors = Arc::new(BackendErrorLog::new());
        let live_stream = open(
            before,
            counting_fill(Arc::clone(&live_counter)),
            live_errors,
        )
        .expect("毒化场景需要一条活流：首次建流必须成功");
        live_stream.play().expect("启流必须成功");
        let live_before = wait_for_frames(&live_counter, 0, Duration::from_secs(5))
            .expect("启流后 5 s 内必须开始出帧");
        eprintln!("LIVE stream frames={live_before}（这条流要活着跨重启）");
        restart_audio_service_for_tests().expect("重启 audiosrv 必须成功");
        let t0 = Instant::now();
        let mut last = String::new();
        // 端点索引重新发布要时间（实测 ~2.4 s），所以允许重试；但**每次重试都是
        // 一个全新的枚举器**，这正是被测的行为。
        while t0.elapsed() < RECOVERY_BUDGET {
            match probe(None) {
                Ok(after) => {
                    let elapsed = t0.elapsed();
                    eprintln!(
                        "AFTER {} ms device={} id={:?} {}ch@{}Hz {}B",
                        elapsed.as_millis(),
                        after.device_name,
                        after.device_id,
                        after.channels,
                        after.sample_rate,
                        after.mix.format.bytes()
                    );
                    assert_eq!(after.device_id, before_id, "重启前后必须是同一台默认端点");
                    assert_eq!(
                        (after.channels, after.sample_rate, after.mix.format),
                        before_shape,
                        "重启前后混音格式必须一致"
                    );
                    // Activate 成功之后还能真的建流 —— `probe` 只到 `IAudioClient`，
                    // 这一句才算"能出声"。
                    let counter = Arc::new(AtomicU64::new(0));
                    let errors = Arc::new(BackendErrorLog::new());
                    let stream = open(after, counting_fill(Arc::clone(&counter)), errors)
                        .expect("重启后必须能建流");
                    stream.play().expect("重启后必须能启流");
                    let frames = wait_for_frames(&counter, 0, Duration::from_secs(5))
                        .expect("重启后建好的流 5 s 内必须出帧");
                    eprintln!(
                        "AFTER frames={frames}（另一条老流仍在，frames={}）",
                        live_counter.load(Ordering::Relaxed)
                    );
                    drop(stream);
                    drop(live_stream);
                    return;
                }
                Err(error) => {
                    last = error.to_string();
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
        drop(live_stream);
        panic!("重启后 30 s 内枚举器没能重新 Activate，最后一次错误：{last}");
    }

    /// **判据 C**：重启前后**设备列表一致**（id / 名字 / 混音格式三项逐台相同）。
    ///
    /// 红对照：**没有**（"缓存枚举器"这条实测不红，理由同判据 B）。本条的用处是
    /// 钉住"重启不会让设备清单变形"这一条用户可见的事实，而不是钉病因。
    ///
    /// `cargo test -p yeban-engine --lib device_list_is_identical_across_an_audio_service_restart -- --ignored --nocapture`
    #[test]
    #[ignore = "真机判据：需要管理员权限停/启 audiosrv"]
    fn device_list_is_identical_across_an_audio_service_restart() {
        let _apartment = bind_mta();
        let before = match enumerate_outputs() {
            Ok(list) => list,
            Err(error) => {
                eprintln!("SKIP：枚举不出输出设备（{error}）");
                return;
            }
        };
        let shape = |list: &[DeviceInfo]| -> Vec<(String, String, u16, u32)> {
            list.iter()
                .map(|device| {
                    (
                        device.id.clone(),
                        device.name.clone(),
                        device.channels,
                        device.sample_rate,
                    )
                })
                .collect()
        };
        let wanted = shape(&before);
        eprintln!("BEFORE {} devices: {wanted:?}", wanted.len());
        restart_audio_service_for_tests().expect("重启 audiosrv 必须成功");
        let t0 = Instant::now();
        let mut last = String::new();
        while t0.elapsed() < RECOVERY_BUDGET {
            match enumerate_outputs() {
                Ok(after) => {
                    let got = shape(&after);
                    if got == wanted {
                        eprintln!("AFTER {} ms identical: {got:?}", t0.elapsed().as_millis());
                        return;
                    }
                    last = format!("列表与重启前不同：{got:?} ≠ {wanted:?}");
                }
                Err(error) => last = error.to_string(),
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("重启后 30 s 内设备列表没能回到重启前的样子：{last}");
    }
}
