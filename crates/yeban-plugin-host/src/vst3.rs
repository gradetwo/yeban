//! VST3 宿主：COM 虚表、宿主侧对象、模块加载与实例。
//!
//! 来源：Steinberg `vst3_pluginterfaces`（MIT）原文（`vst/ivstcomponent.h`、
//! `vst/ivstaudioprocessor.h`、`vst/ivsteditcontroller.h`、`vst/ivstevents.h`、
//! `base/ibstream.h`、`pluginterfaces/vst/ihostapplication.h`）。
//!
//! 布局要点（实测自 SDK 头文件，⛔ 不可凭记忆改）：
//! - 每张虚表前 3 项都是 `FUnknown` 的 `queryInterface`／`addRef`／`release`。
//! - `String128` ＝ 128 个 UTF-16 单元 ＝ 256 字节（⛔ 不是 128 字节）：
//!   `PFactoryInfo` 772 字节，`BusInfo` 276 字节，`ParameterInfo` 792 字节。
//! - `PClassInfo` 116 字节（`category`／`name` 是单字节 `char` 串，⛔ 不是 `String128`）。
//! - `Event` 48 字节：头（`type` i32＋总线＋偏移＋`ppqPosition` f64＋`flags` u16＋补齐）
//!   28 字节＋联合 20 字节（最大成员 `NoteOnEvent` 20 字节）。
//! - `ProcessData` 80 字节，`AudioBusBuffers` 24 字节，`ProcessSetup` 24 字节。
//! - 虚表函数一律 `extern "system"`（Windows 上＝`__stdcall`，即 COM 调用约定；
//!   Unix 上＝C 调用约定，`this` 为首参，与 Itanium 虚表 ABI 一致）。
//! - `tresult` 歧义：`kResultFalse == kResultOk == 0`。通用调用“非负＝成功”；
//!   只有 `queryInterface` 要求严格等于 0（`kNoInterface` 为 -1）。
//!
//! 一期局限（写进 ADR-0064，不藏）：
//! - 宿主侧对象（`IHostApplication`、`IComponentHandler`）的 `queryInterface`
//!   只应答 `FUnknown`，其余返回 `kNoInterface`：插件按指针直调方法的常规
//!   路径不受影响；个别插件若先 `queryInterface` 再确认宿主对象类型，可能走
//!   降级分支（`initialize` 仍成功，插件按“无宿主名”继续）。
//! - 单独控制器不做 `IConnectionPoint` 连接（`connect`／`disconnect` 一期不用）：
//!   控制器拿不到处理器侧的参数变化通知，参数以构造期＋渲染前直设为准。

use std::ffi::{c_char, c_void};
use std::ptr;

use crate::dylib::Library;
use crate::error::HostError;
use crate::types::{
    FUNKNOWN_IID, IAUDIO_PROCESSOR_IID, ICOMPONENT_IID, IEDIT_CONTROLLER_IID, MidiEvent,
    MidiEventKind, ParamInfo, PluginClass, PluginInstance, PluginKind, PluginState,
    RESULT_NO_INTERFACE, RESULT_NOT_IMPLEMENTED, RESULT_OK, TUID_INVALID, Tuid,
};

// ---------------------------------------------------------------------------
// 常量（SDK 枚举原文）
// ---------------------------------------------------------------------------

/// 媒体类型：音频。
pub const MEDIA_AUDIO: i32 = 0;
/// 媒体类型：事件。
pub const MEDIA_EVENT: i32 = 1;
/// 总线方向：输入。
pub const DIR_INPUT: i32 = 0;
/// 总线方向：输出。
pub const DIR_OUTPUT: i32 = 1;
/// I/O 模式：简单（插件不区分输入输出通道数）。
pub const IO_SIMPLE: i32 = 0;
/// 处理模式：实时。
pub const PROCESS_REALTIME: i32 = 0;
/// 符号样本大小：32 位浮点。
pub const SAMPLE_32: i32 = 0;
/// 立体声扬声器排布（`SpeakerArrangement`，`kStereo = 0x3`）。
pub const ARRANGEMENT_STEREO: u64 = 0x3;
/// 事件类型：音符开。
pub const EVENT_NOTE_ON: i32 = 0;
/// 事件类型：音符关。
pub const EVENT_NOTE_OFF: i32 = 1;
/// `IBStream::seek` 模式：从头。
pub const SEEK_SET: i32 = 0;
/// `IBStream::seek` 模式：从当前位置。
pub const SEEK_CUR: i32 = 1;
/// `IBStream::seek` 模式：从尾。
pub const SEEK_END: i32 = 2;

/// 校验通用调用（非负＝成功；负值 ⇒ `CallFailed`）。
fn check(method: &'static str, code: i32) -> Result<(), HostError> {
    if code < 0 {
        Err(HostError::CallFailed { method, code })
    } else {
        Ok(())
    }
}

/// 校验 `queryInterface`（严格等于 `kResultOk` 才算拿到接口）。
fn check_query(method: &'static str, code: i32) -> Result<(), HostError> {
    if code != RESULT_OK {
        Err(HostError::CallFailed { method, code })
    } else {
        Ok(())
    }
}

/// 解码 `String128`（UTF-16，遇 NUL 截断，非法序列按 `from_utf16_lossy`）。
fn decode_string128(units: &[u16; 128]) -> String {
    let end = units.iter().position(|unit| *unit == 0).unwrap_or(128);
    String::from_utf16_lossy(&units[..end])
}

/// 解码 `char` 串（遇 NUL 截断，非 UTF-8 字节按丢失替换）。
fn decode_char_str(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

// ---------------------------------------------------------------------------
// SDK 结构体（`#[repr(C)]`，布局见模块文档）
// ---------------------------------------------------------------------------

/// `PFactoryInfo`：772 字节。
#[repr(C)]
#[derive(Clone, Copy)]
struct PFactoryInfo {
    vendor: [u16; 128],
    url: [u16; 128],
    email: [u16; 128],
    flags: i32,
}

/// `PClassInfo`：116 字节。
#[repr(C)]
#[derive(Clone, Copy)]
struct PClassInfo {
    cid: Tuid,
    cardinality: i32,
    category: [u8; 32],
    name: [u8; 64],
}

/// `BusInfo`：276 字节。
#[repr(C)]
#[derive(Clone, Copy)]
struct BusInfo {
    media_type: i32,
    direction: i32,
    channel_count: i32,
    name: [u16; 128],
    bus_type: i32,
    flags: u32,
}

/// `ParameterInfo`：792 字节。
#[repr(C)]
#[derive(Clone, Copy)]
struct ParameterInfoRaw {
    id: u32,
    title: [u16; 128],
    short_title: [u16; 128],
    units: [u16; 128],
    step_count: i32,
    default_normalized: f64,
    unit_id: i32,
    flags: i32,
}

/// `RoutingInfo`：12 字节（`getRoutingInfo` 用，一期不调，只定布局备用）。
#[repr(C)]
#[derive(Clone, Copy)]
struct RoutingInfo {
    media_type: i32,
    bus_index: i32,
    channel: i32,
}

/// `ProcessSetup`：24 字节。
#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessSetup {
    process_mode: i32,
    symbolic_sample_size: i32,
    max_samples_per_block: i32,
    _pad: i32,
    sample_rate: f64,
}

/// `AudioBusBuffers`：24 字节。
#[repr(C)]
#[derive(Clone, Copy)]
struct AudioBusBuffers {
    num_channels: i32,
    _pad: i32,
    silence_flags: u64,
    channel_buffers: *mut *mut f32,
}

/// `ProcessData`：80 字节。
#[repr(C)]
struct ProcessData {
    process_mode: i32,
    symbolic_sample_size: i32,
    num_samples: i32,
    num_inputs: i32,
    num_outputs: i32,
    _pad: i32,
    inputs: *mut AudioBusBuffers,
    outputs: *mut AudioBusBuffers,
    input_events: *mut c_void,
    output_events: *mut c_void,
    input_parameter_changes: *mut c_void,
    output_parameter_changes: *mut c_void,
    process_context: *mut c_void,
}

/// `Event`：48 字节（头 28＋联合 20；联合只写 NoteOn／NoteOff，见模块文档）。
#[repr(C)]
#[derive(Clone, Copy)]
struct Event {
    event_type: i32,
    bus_index: i32,
    sample_offset: i32,
    _pad: i32,
    ppq_position: f64,
    flags: u16,
    _pad2: u16,
    data: [u8; 20],
}

impl Event {
    /// 写 NoteOn 联合体（`channel` i16＋`pitch` i16＋`velocity` f32＋
    /// `length` i32（0＝未知）＋`tuning` f32（0）＋`noteId` i32）。
    fn note_on(channel: u8, pitch: u8, velocity: f32, note_id: i32, offset: u32) -> Self {
        let mut data = [0_u8; 20];
        data[0..2].copy_from_slice(&(channel as i16).to_le_bytes());
        data[2..4].copy_from_slice(&(pitch as i16).to_le_bytes());
        data[4..8].copy_from_slice(&velocity.to_le_bytes());
        data[16..20].copy_from_slice(&note_id.to_le_bytes());
        Self {
            event_type: EVENT_NOTE_ON,
            bus_index: 0,
            sample_offset: offset as i32,
            _pad: 0,
            ppq_position: 0.0,
            flags: 0,
            _pad2: 0,
            data,
        }
    }

    /// 写 NoteOff 联合体（`channel`＋`pitch`＋`velocity`＋`noteId`＋`tuning`）。
    fn note_off(channel: u8, pitch: u8, velocity: f32, note_id: i32, offset: u32) -> Self {
        let mut data = [0_u8; 20];
        data[0..2].copy_from_slice(&(channel as i16).to_le_bytes());
        data[2..4].copy_from_slice(&(pitch as i16).to_le_bytes());
        data[4..8].copy_from_slice(&velocity.to_le_bytes());
        data[8..12].copy_from_slice(&note_id.to_le_bytes());
        Self {
            event_type: EVENT_NOTE_OFF,
            bus_index: 0,
            sample_offset: offset as i32,
            _pad: 0,
            ppq_position: 0.0,
            flags: 0,
            _pad2: 0,
            data,
        }
    }
}

// ---------------------------------------------------------------------------
// COM 虚表（顺序见模块文档；全部 `extern "system"`）
// ---------------------------------------------------------------------------

type QueryFn = unsafe extern "system" fn(*mut c_void, *const Tuid, *mut *mut c_void) -> i32;
type RefFn = unsafe extern "system" fn(*mut c_void) -> u32;

#[repr(C)]
struct FUnknownVtbl {
    query_interface: QueryFn,
    add_ref: RefFn,
    release: RefFn,
}

#[repr(C)]
struct PluginFactoryVtbl {
    base: FUnknownVtbl,
    get_factory_info: unsafe extern "system" fn(*mut c_void, *mut PFactoryInfo) -> i32,
    count_classes: unsafe extern "system" fn(*mut c_void) -> i32,
    get_class_info: unsafe extern "system" fn(*mut c_void, i32, *mut PClassInfo) -> i32,
    create_instance:
        unsafe extern "system" fn(*mut c_void, *const Tuid, *const Tuid, *mut *mut c_void) -> i32,
}

#[repr(C)]
struct PluginBaseVtbl {
    base: FUnknownVtbl,
    initialize: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    terminate: unsafe extern "system" fn(*mut c_void) -> i32,
}

#[repr(C)]
struct ComponentVtbl {
    base: PluginBaseVtbl,
    get_controller_class_id: unsafe extern "system" fn(*mut c_void, *mut Tuid) -> i32,
    set_io_mode: unsafe extern "system" fn(*mut c_void, i32) -> i32,
    get_bus_count: unsafe extern "system" fn(*mut c_void, i32, i32) -> i32,
    get_bus_info: unsafe extern "system" fn(*mut c_void, i32, i32, i32, *mut BusInfo) -> i32,
    get_routing_info:
        unsafe extern "system" fn(*mut c_void, *mut RoutingInfo, *mut RoutingInfo) -> i32,
    activate_bus: unsafe extern "system" fn(*mut c_void, i32, i32, i32, u8) -> i32,
    set_active: unsafe extern "system" fn(*mut c_void, u8) -> i32,
    set_state: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    get_state: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
}

#[repr(C)]
struct AudioProcessorVtbl {
    base: PluginBaseVtbl,
    set_bus_arrangements:
        unsafe extern "system" fn(*mut c_void, *mut u64, i32, *mut u64, i32) -> i32,
    get_bus_arrangement: unsafe extern "system" fn(*mut c_void, i32, i32, *mut u64) -> i32,
    can_process_sample_size: unsafe extern "system" fn(*mut c_void, i32) -> i32,
    get_latency_samples: unsafe extern "system" fn(*mut c_void) -> u32,
    setup_processing: unsafe extern "system" fn(*mut c_void, *const ProcessSetup) -> i32,
    set_processing: unsafe extern "system" fn(*mut c_void, u8) -> i32,
    process: unsafe extern "system" fn(*mut c_void, *mut ProcessData) -> i32,
    get_tail_samples: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct EditControllerVtbl {
    base: PluginBaseVtbl,
    set_component_state: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    set_state: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    get_state: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    get_parameter_count: unsafe extern "system" fn(*mut c_void) -> i32,
    get_parameter_info: unsafe extern "system" fn(*mut c_void, i32, *mut ParameterInfoRaw) -> i32,
    get_param_string_by_value: unsafe extern "system" fn(*mut c_void, u32, f64, *mut u16) -> i32,
    get_param_value_by_string:
        unsafe extern "system" fn(*mut c_void, u32, *const u16, *mut f64) -> i32,
    normalized_param_to_plain: unsafe extern "system" fn(*mut c_void, u32, f64) -> f64,
    plain_param_to_normalized: unsafe extern "system" fn(*mut c_void, u32, f64) -> f64,
    get_param_normalized: unsafe extern "system" fn(*mut c_void, u32) -> f64,
    set_param_normalized: unsafe extern "system" fn(*mut c_void, u32, f64) -> i32,
    set_component_handler: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    create_view: unsafe extern "system" fn(*mut c_void, *const c_char) -> *mut c_void,
}

#[repr(C)]
struct EventListVtbl {
    base: FUnknownVtbl,
    get_event_count: unsafe extern "system" fn(*mut c_void) -> i32,
    get_event: unsafe extern "system" fn(*mut c_void, i32, *mut Event) -> i32,
    add_event: unsafe extern "system" fn(*mut c_void, *const Event) -> i32,
}

#[repr(C)]
struct BStreamVtbl {
    base: FUnknownVtbl,
    read: unsafe extern "system" fn(*mut c_void, *mut c_void, i32, *mut i32) -> i32,
    write: unsafe extern "system" fn(*mut c_void, *const c_void, i32, *mut i32) -> i32,
    seek: unsafe extern "system" fn(*mut c_void, i64, i32, *mut i64) -> i32,
    tell: unsafe extern "system" fn(*mut c_void, *mut i64) -> i32,
}

#[repr(C)]
struct HostApplicationVtbl {
    base: FUnknownVtbl,
    get_name: unsafe extern "system" fn(*mut c_void, *mut u16) -> i32,
    create_instance:
        unsafe extern "system" fn(*mut c_void, *const Tuid, *const Tuid, *mut *mut c_void) -> i32,
}

#[repr(C)]
struct ComponentHandlerVtbl {
    base: FUnknownVtbl,
    begin_edit: unsafe extern "system" fn(*mut c_void, u32) -> i32,
    perform_edit: unsafe extern "system" fn(*mut c_void, u32, f64) -> i32,
    end_edit: unsafe extern "system" fn(*mut c_void, u32) -> i32,
    restart_component: unsafe extern "system" fn(*mut c_void, i32) -> i32,
}

/// 从对象指针读虚表（对象首字段恒为虚表指针，COM 约定）。
///
/// # Safety
///
/// 调用方保证 `obj` 是指向 COM 对象的有效指针，且该对象首字段为对应虚表指针。
unsafe fn vtbl<T>(obj: *mut c_void) -> &'static T {
    // SAFETY: 调用方保证对象有效；虚表指针在对象生命周期内恒有效。
    unsafe { &*(*obj.cast::<*const T>()) }
}

// ---------------------------------------------------------------------------
// 宿主侧对象（`Box` 承载，地址在实例生命周期内稳定；引用计数为空操作）
// ---------------------------------------------------------------------------

/// `IHostApplication` 的宿主实现：只报名字 `Yeban`，不创建实例。
#[repr(C)]
struct HostApplicationObj {
    vtbl: *const HostApplicationVtbl,
}

impl HostApplicationObj {
    fn new() -> Box<Self> {
        static VTBL: HostApplicationVtbl = HostApplicationVtbl {
            base: FUnknownVtbl {
                query_interface: host_app_query,
                add_ref: host_noop_ref,
                release: host_noop_ref,
            },
            get_name: host_app_get_name,
            create_instance: host_app_create_instance,
        };
        Box::new(Self { vtbl: &VTBL })
    }
}

unsafe extern "system" fn host_app_query(
    this: *mut c_void,
    iid: *const Tuid,
    out: *mut *mut c_void,
) -> i32 {
    // SAFETY: 插件按 COM 约定传有效指针；只读 IID 做比较。
    let iid = unsafe { *iid };
    if iid == FUNKNOWN_IID {
        // SAFETY: `out` 是插件提供的写槽。
        unsafe { *out = this };
        RESULT_OK
    } else {
        RESULT_NO_INTERFACE
    }
}

/// 宿主对象的引用计数：空操作（`Box` 拥有所有权，插件的 `release` 不删对象）。
unsafe extern "system" fn host_noop_ref(_this: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn host_app_get_name(_this: *mut c_void, name: *mut u16) -> i32 {
    // SAFETY: `name` 是插件提供的 `String128` 写槽（128 个 u16）。
    let units: Vec<u16> = "Yeban".encode_utf16().collect();
    // SAFETY: `name` 是插件提供的 String128 写槽（128 个 u16）；写入 5 个单元＋NUL，不越界。
    unsafe {
        ptr::copy_nonoverlapping(units.as_ptr(), name, units.len());
        *name.add(units.len()) = 0;
    }
    RESULT_OK
}

unsafe extern "system" fn host_app_create_instance(
    _this: *mut c_void,
    _cid: *const Tuid,
    _iid: *const Tuid,
    _out: *mut *mut c_void,
) -> i32 {
    // 一期宿主不提供可创建对象（见模块文档的局限说明）。
    RESULT_NO_INTERFACE
}

/// `IComponentHandler` 的宿主实现：一期无自动化队列，编辑通知全部空操作成功。
#[repr(C)]
struct ComponentHandlerObj {
    vtbl: *const ComponentHandlerVtbl,
}

impl ComponentHandlerObj {
    fn new() -> Box<Self> {
        static VTBL: ComponentHandlerVtbl = ComponentHandlerVtbl {
            base: FUnknownVtbl {
                query_interface: host_app_query,
                add_ref: host_noop_ref,
                release: host_noop_ref,
            },
            begin_edit: handler_noop_edit,
            perform_edit: handler_noop_perform,
            end_edit: handler_noop_edit,
            restart_component: handler_noop_restart,
        };
        Box::new(Self { vtbl: &VTBL })
    }
}

unsafe extern "system" fn handler_noop_edit(_this: *mut c_void, _id: u32) -> i32 {
    RESULT_OK
}

unsafe extern "system" fn handler_noop_perform(_this: *mut c_void, _id: u32, _value: f64) -> i32 {
    // 一期无参数队列：插件侧的 `performEdit` 被接受但不记录（参数以直设为准）。
    RESULT_OK
}

unsafe extern "system" fn handler_noop_restart(_this: *mut c_void, _flags: i32) -> i32 {
    RESULT_OK
}

/// 宿主侧事件列表（栈上构造，只在一次 `process` 调用内有效）。
/// `events` 指向调用帧的 `Vec<Event>`，插件在 `process` 返回前只读。
#[repr(C)]
struct HostEventList {
    vtbl: *const EventListVtbl,
    events: *const Event,
    count: i32,
}

impl HostEventList {
    fn new(events: &[Event]) -> Self {
        static VTBL: EventListVtbl = EventListVtbl {
            base: FUnknownVtbl {
                query_interface: host_app_query,
                add_ref: host_noop_ref,
                release: host_noop_ref,
            },
            get_event_count: event_list_count,
            get_event: event_list_get,
            add_event: event_list_add,
        };
        Self {
            vtbl: &VTBL,
            events: events.as_ptr(),
            count: events.len() as i32,
        }
    }
}

unsafe extern "system" fn event_list_count(this: *mut c_void) -> i32 {
    // SAFETY: `this` 是调用帧内的 `HostEventList`，只读。
    unsafe { (*this.cast::<HostEventList>()).count }
}

unsafe extern "system" fn event_list_get(this: *mut c_void, index: i32, out: *mut Event) -> i32 {
    // SAFETY: `this` 是调用帧内的 `HostEventList`；`out` 是插件提供的写槽。
    let list = unsafe { &*this.cast::<HostEventList>() };
    if index < 0 || index >= list.count {
        return 0; // `kResultFalse`：越界＝无此事件（与 `kResultOk` 同值，语义不同）。
    }
    // SAFETY: 下标已在上文越界检查后；`out` 是插件提供的 Event 写槽。
    unsafe {
        ptr::copy_nonoverlapping(list.events.add(index as usize), out, 1);
    }
    RESULT_OK
}

unsafe extern "system" fn event_list_add(_this: *mut c_void, _event: *const Event) -> i32 {
    // 宿主事件列表只读（输入方向）：插件写输出事件一期不支持。
    RESULT_NOT_IMPLEMENTED
}

/// 内存流（`IBStream` 的宿主实现；栈上构造，只在一次 `getState`／`setState` 内有效）。
#[repr(C)]
struct MemoryStream {
    vtbl: *const BStreamVtbl,
    buf: Vec<u8>,
    pos: usize,
}

impl MemoryStream {
    /// 写捕获流（`getState` 用：空缓冲，插件写）。
    fn capture() -> Self {
        static VTBL: BStreamVtbl = BStreamVtbl {
            base: FUnknownVtbl {
                query_interface: host_app_query,
                add_ref: host_noop_ref,
                release: host_noop_ref,
            },
            read: stream_read,
            write: stream_write,
            seek: stream_seek,
            tell: stream_tell,
        };
        Self {
            vtbl: &VTBL,
            buf: Vec::new(),
            pos: 0,
        }
    }

    /// 读恢复流（`setState` 用：`buf` 为状态字节，插件读）。
    fn restore(bytes: &[u8]) -> Self {
        let mut stream = Self::capture();
        stream.buf = bytes.to_vec();
        stream
    }
}

unsafe extern "system" fn stream_read(
    this: *mut c_void,
    buffer: *mut c_void,
    num_bytes: i32,
    out: *mut i32,
) -> i32 {
    // SAFETY: `this` 是调用帧内的 `MemoryStream`；`buffer` 是插件提供的写槽。
    let stream = unsafe { &mut *this.cast::<MemoryStream>() };
    let want = num_bytes.max(0) as usize;
    let have = stream.buf.len().saturating_sub(stream.pos);
    let take = want.min(have);
    // SAFETY: `take` ≤ 剩余字节；`buffer` 是插件提供的写槽。
    unsafe {
        ptr::copy_nonoverlapping(
            stream.buf.as_ptr().add(stream.pos),
            buffer.cast::<u8>(),
            take,
        );
        *out = take as i32;
    }
    stream.pos += take;
    RESULT_OK
}

unsafe extern "system" fn stream_write(
    this: *mut c_void,
    buffer: *const c_void,
    num_bytes: i32,
    out: *mut i32,
) -> i32 {
    // SAFETY: `this` 是调用帧内的 `MemoryStream`；`buffer` 是插件提供的读槽。
    let stream = unsafe { &mut *this.cast::<MemoryStream>() };
    let want = num_bytes.max(0) as usize;
    if stream.pos > stream.buf.len() {
        stream.pos = stream.buf.len();
    }
    if stream.pos + want > stream.buf.len() {
        stream.buf.resize(stream.pos + want, 0);
    }
    // SAFETY: 目标区已按需扩容；`buffer` 是插件提供的读槽。
    unsafe {
        ptr::copy_nonoverlapping(
            buffer.cast::<u8>(),
            stream.buf.as_mut_ptr().add(stream.pos),
            want,
        );
        *out = want as i32;
    }
    stream.pos += want;
    RESULT_OK
}

unsafe extern "system" fn stream_seek(
    this: *mut c_void,
    pos: i64,
    mode: i32,
    out: *mut i64,
) -> i32 {
    // SAFETY: `this` 是调用帧内的 `MemoryStream`；`out` 是插件提供的写槽。
    let stream = unsafe { &mut *this.cast::<MemoryStream>() };
    let base: i64 = match mode {
        SEEK_SET => 0,
        SEEK_CUR => stream.pos as i64,
        SEEK_END => stream.buf.len() as i64,
        _ => return RESULT_NOT_IMPLEMENTED,
    };
    let target = base.saturating_add(pos).clamp(0, stream.buf.len() as i64);
    stream.pos = target as usize;
    // SAFETY: `out` 是插件提供的 i64 写槽。
    unsafe { *out = target };
    RESULT_OK
}

unsafe extern "system" fn stream_tell(this: *mut c_void, out: *mut i64) -> i32 {
    // SAFETY: `this` 是调用帧内的 `MemoryStream`；`out` 是插件提供的写槽。
    let stream = unsafe { &*this.cast::<MemoryStream>() };
    // SAFETY: `out` 是插件提供的 i64 写槽。
    unsafe { *out = stream.pos as i64 };
    RESULT_OK
}

// ---------------------------------------------------------------------------
// 模块与实例
// ---------------------------------------------------------------------------

/// `GetPluginFactory` 的函数类型（`SMTG_EXPORT_SYMBOL` 导出，返回工厂指针）。
type GetPluginFactoryFn = unsafe extern "system" fn() -> *mut c_void;

/// 模块内芯（`Arc` 共享：扫描探针要从同一个模块开多个类实例，
///
/// 每个实例各持一个 `Arc`，库在最后一个实例释放后才关闭）。
struct Vst3ModuleInner {
    // 仅为保活持有：`Library` 的 `Drop` 在内芯释放时关闭动态库；
    // 工厂指针在库生命周期内有效，实例持 `Arc` 故库活得比实例久。
    #[allow(dead_code)]
    lib: Library,
    factory: *mut c_void,
}

// SAFETY: `lib` 单主拥有（`Arc` 只共享所有权，不共享可变访问）；
// 工厂指针在库生命周期内有效，而实例持 `Arc` 故库活得比实例久。
unsafe impl Send for Vst3ModuleInner {}
// SAFETY: 内芯无可变共享状态（`lib` 只在 `Arc` 释放时关闭；`factory` 只读）。
unsafe impl Sync for Vst3ModuleInner {}

/// 一个已打开的 VST3 模块（工厂指针向库借用，⛔ 永不 `release`）。
pub struct Vst3Module {
    inner: std::sync::Arc<Vst3ModuleInner>,
}

impl Vst3Module {
    /// 打开模块（`dlopen` → `GetPluginFactory` → 工厂指针）。
    ///
    /// # Errors
    ///
    /// 打不开、无导出、工厂为空 ⇒ [`HostError`]。
    pub fn load(path: &str) -> Result<Self, HostError> {
        let lib = Library::open(path)?;
        let symbol = lib
            .symbol("GetPluginFactory")
            .map_err(|_| HostError::FactoryMissing)?;
        // SAFETY: 符号已证实存在；按 SDK 签名调用（无参，返回工厂指针）。
        let factory_fn: GetPluginFactoryFn = unsafe { std::mem::transmute(symbol) };
        // SAFETY: 工厂函数指针来自已验证的 GetPluginFactory 符号；无参调用，返回工厂指针。
        let factory = unsafe { factory_fn() };
        if factory.is_null() {
            return Err(HostError::FactoryMissing);
        }
        Ok(Self {
            inner: std::sync::Arc::new(Vst3ModuleInner { lib, factory }),
        })
    }

    /// 读工厂信息（厂商名；失败 ⇒ 空串，不阻塞扫描）。
    fn factory_vendor(&self) -> String {
        // SAFETY: `factory` 是 `load` 证实非空的指针；虚表首字段为工厂虚表。
        let vtbl = unsafe { vtbl::<PluginFactoryVtbl>(self.inner.factory) };
        let mut info = PFactoryInfo {
            vendor: [0; 128],
            url: [0; 128],
            email: [0; 128],
            flags: 0,
        };
        // SAFETY: `info` 是栈上写槽；返回值忽略（厂商名读不到不算失败）。
        let code = unsafe { (vtbl.get_factory_info)(self.inner.factory, &mut info) };
        if code < 0 {
            return String::new();
        }
        decode_string128(&info.vendor)
    }

    /// 枚举全部类（`countClasses`＋`getClassInfo`；任一失败 ⇒ 空表）。
    pub(crate) fn classes_raw(&self) -> Vec<(Tuid, String, String)> {
        // SAFETY: `factory` 有效；虚表调用按 SDK 签名。
        let vtbl = unsafe { vtbl::<PluginFactoryVtbl>(self.inner.factory) };
        // SAFETY: 工厂指针有效；虚表调用按 SDK 签名。
        let count = unsafe { (vtbl.count_classes)(self.inner.factory) };
        if count < 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        for index in 0..count {
            let mut info = PClassInfo {
                cid: TUID_INVALID,
                cardinality: 0,
                category: [0; 32],
                name: [0; 64],
            };
            // SAFETY: 工厂指针有效；虚表调用按 SDK 签名。
            let code = unsafe { (vtbl.get_class_info)(self.inner.factory, index, &mut info) };
            if code < 0 {
                continue;
            }
            out.push((
                info.cid,
                decode_char_str(&info.name),
                decode_char_str(&info.category),
            ));
        }
        out
    }

    /// 创建一个类的实例（`createInstance`，IID 由调用方给）。
    ///
    /// # Errors
    ///
    /// 工厂拒绝 ⇒ `CallFailed`。
    fn create_instance(&self, cid: &Tuid, iid: &Tuid) -> Result<*mut c_void, HostError> {
        // SAFETY: `factory` 有效；`cid`／`iid` 是调用方提供的 16 字节标识。
        let vtbl = unsafe { vtbl::<PluginFactoryVtbl>(self.inner.factory) };
        let mut obj: *mut c_void = ptr::null_mut();
        // SAFETY: 工厂指针有效；虚表调用按 SDK 签名。
        let code = unsafe { (vtbl.create_instance)(self.inner.factory, cid, iid, &mut obj) };
        check_query("createInstance", code)?;
        if obj.is_null() {
            return Err(HostError::CallFailed {
                method: "createInstance",
                code,
            });
        }
        Ok(obj)
    }

    /// `queryInterface`（对象 → 目标接口；严格等于 `kResultOk` 才算拿到）。
    ///
    /// # Errors
    ///
    /// 插件不实现该接口 ⇒ `CallFailed`（`kNoInterface`）。
    fn query_interface(obj: *mut c_void, iid: &Tuid) -> Result<*mut c_void, HostError> {
        // SAFETY: `obj` 是插件对象指针；虚表首 3 项为 `FUnknown`。
        let vtbl = unsafe { vtbl::<FUnknownVtbl>(obj) };
        let mut out: *mut c_void = ptr::null_mut();
        // SAFETY: 对象指针由调用方保证有效；虚表首 3 项为 FUnknown。
        let code = unsafe { (vtbl.query_interface)(obj, iid, &mut out) };
        check_query("queryInterface", code)?;
        if out.is_null() {
            return Err(HostError::CallFailed {
                method: "queryInterface",
                code,
            });
        }
        Ok(out)
    }

    /// 释放一个 COM 引用（`release` 一次；返回值忽略）。
    ///
    /// # Safety
    ///
    /// 调用方保证 `obj` 是插件对象指针，且本次 `release` 与一次先前的
    /// `createInstance`／`queryInterface` 成功配对（不多不少一次）。
    unsafe fn release(obj: *mut c_void) {
        // SAFETY: 调用方保证配对；虚表首 3 项为 `FUnknown`。
        let vtbl = unsafe { vtbl::<FUnknownVtbl>(obj) };
        // SAFETY: 对象指针由调用方保证有效；虚表首 3 项为 FUnknown。
        unsafe { (vtbl.release)(obj) };
    }
}

/// `getBusCount` 包装（`component` 指针有效时调用；负值 ⇒ `CallFailed`）。
fn bus_count(component: *mut c_void, media: i32, dir: i32) -> Result<i32, HostError> {
    // SAFETY: 调用方保证 `component` 是组件对象指针。
    let vtbl = unsafe { vtbl::<ComponentVtbl>(component) };
    // SAFETY: `component` 是组件对象指针；虚表调用按 SDK 签名。
    let count = unsafe { (vtbl.get_bus_count)(component, media, dir) };
    if count < 0 {
        return Err(HostError::CallFailed {
            method: "getBusCount",
            code: count,
        });
    }
    Ok(count)
}

/// VST3 实例（`PluginInstance` 的 VST3 实现）。
///
/// 所有权规则（⛔ 不可错）：
/// - `module` 是 `Arc`：库在最后一个实例释放后才关闭；`factory` 指针向库借用，
///   **永不** `release`。
/// - `component`（`createInstance` 得来）、`processor`（`queryInterface` 得来）、
///   `controller`（工厂另建或 `queryInterface` 得来）各持一个引用，
///   `Drop` 时各 `release` **一次**（组合式对象三指针相同，三个引用仍各算一次）。
/// - `handler`／`host_app` 是实例拥有的 `Box`：插件在 `initialize`／
///   `setComponentHandler` 时拿到裸指针，`Box` 在实例生命周期内不替换，
///   故指针地址稳定；插件按 SDK 语义不在调用外持有宿主对象（`initialize` 的
///   context 只在调用内有效，`setComponentHandler` 的指针由实例全程保活）。
pub struct Vst3Instance {
    // 仅为保活持有：`Arc` 让动态库活得比实例久（工厂指针借用它）。
    #[allow(dead_code)]
    module: std::sync::Arc<Vst3ModuleInner>,
    component: *mut c_void,
    processor: *mut c_void,
    controller: Option<*mut c_void>,
    // 仅为保活持有：插件存裸指针，`Box` 在实例生命周期内不替换，
    // 故指针地址稳定；代码不读它们，只保证它们活着。
    #[allow(dead_code)]
    handler: Box<ComponentHandlerObj>,
    #[allow(dead_code)]
    host_app: Box<HostApplicationObj>,
    name: String,
    kind: PluginKind,
    active: bool,
    latency: u32,
    sample_rate: f64,
    max_block: usize,
}

// SAFETY: 实例单主持有；全部插件调用都在持有 `&mut self` 的调用方线程上发生；
// `Drop` 的 `release` 调用线程安全（COM 引用计数本身是原子的）；
// 宿主对象 `Box` 的地址在实例生命周期内稳定。
unsafe impl Send for Vst3Instance {}

// SAFETY: `Sync` 契约（见 `PluginInstance` trait 文档）：进插件的方法一律
// `&mut self`（`render_block` 持 `&mut` 调用，`&` 与 `&mut` 不共存）；
// `&self` 方法（`name`／`is_active`／`latency_samples`）只读 Rust 侧缓存字段，
// ⛔ 不进插件；故跨线程共享 `&` 时插件状态不可能被并发触碰。
unsafe impl Sync for Vst3Instance {}

impl Vst3Instance {
    /// 打开一个类并初始化到“可探针”状态（⛔ 不激活：激活要采样率，渲染时才定）。
    ///
    /// 顺序（SDK 要求）：`createInstance(IComponent)` → `setIoMode(kSimple)` →
    /// `initialize(hostApp)` → `queryInterface(IAudioProcessor)` →
    /// 总线探针（定 `kind`）→ 控制器解析。
    ///
    /// ⛔ 本函数**不**因“不是乐器”失败：扫描要列出效果器类并标 `kind`；
    /// “必须是乐器”的拒绝由调用方（`host.rs` 的 `load`）按 `kind` 做。
    ///
    /// # Errors
    ///
    /// 任一步失败 ⇒ [`HostError`]（已建的对象在返回前按配对规则释放，
    /// ⛔ 不留半初始化实例）。
    pub fn open(module: &Vst3Module, class_id: &Tuid, class_name: &str) -> Result<Self, HostError> {
        let handler = ComponentHandlerObj::new();
        let host_app = HostApplicationObj::new();
        let handler_ptr = (&*handler) as *const ComponentHandlerObj as *mut c_void;
        let host_ptr = (&*host_app) as *const HostApplicationObj as *mut c_void;
        let mut component: *mut c_void = ptr::null_mut();
        let mut processor: *mut c_void = ptr::null_mut();
        let mut controller: Option<*mut c_void> = None;
        let result = (|| -> Result<PluginKind, HostError> {
            component = module.create_instance(class_id, &ICOMPONENT_IID)?;
            // SAFETY: `component` 刚创建有效；虚表为组件虚表。
            let comp_vtbl = unsafe { vtbl::<ComponentVtbl>(component) };
            // `setIoMode` 必须在 `initialize` 之前；个别插件返回
            // `kNotImplemented` 也继续（SDK 允许缺省简单模式）。
            // SAFETY: `component` 是刚创建的组件对象；虚表调用按 SDK 签名。
            let _ = unsafe { (comp_vtbl.set_io_mode)(component, IO_SIMPLE) };
            // SAFETY: `component` 是刚创建的组件对象；虚表调用按 SDK 签名。
            let code = unsafe { (comp_vtbl.base.initialize)(component, host_ptr) };
            check("initialize", code)?;
            processor = Vst3Module::query_interface(component, &IAUDIO_PROCESSOR_IID)
                .map_err(|_| HostError::InterfaceMissing("IAudioProcessor"))?;
            // 总线探针定 `kind`：乐器 ⟺ 事件输入 ≥1 且无音频输入；
            // 有音频输入 ⟺ 效果器；其余 ⟺ 未知。
            let event_in = bus_count(component, MEDIA_EVENT, DIR_INPUT)?;
            let audio_in = bus_count(component, MEDIA_AUDIO, DIR_INPUT)?;
            let kind = if event_in >= 1 && audio_in == 0 {
                PluginKind::Instrument
            } else if audio_in >= 1 {
                PluginKind::Effect
            } else {
                PluginKind::Unknown
            };
            // 控制器解析：先问分离式（`getControllerClassId` 真值＋非零 CID），
            // 否则问组合式（`queryInterface(IEditController)`），都没有也允许
            // （无控制器插件：参数为空集，状态只有处理器段）。
            let mut controller_cid = TUID_INVALID;
            // SAFETY: `component` 是刚创建的组件对象；虚表调用按 SDK 签名。
            let code =
                unsafe { (comp_vtbl.get_controller_class_id)(component, &mut controller_cid) };
            if code == 1 && controller_cid != TUID_INVALID {
                controller = Some(module.create_instance(&controller_cid, &IEDIT_CONTROLLER_IID)?);
            } else {
                controller = Vst3Module::query_interface(component, &IEDIT_CONTROLLER_IID).ok();
            }
            // 控制器拿到后挂实例拥有的 handler（失败忽略：handler 只是通知通道）。
            if let Some(ctrl) = controller {
                // SAFETY: `ctrl` 是刚解析的控制器指针。
                let ctrl_vtbl = unsafe { vtbl::<EditControllerVtbl>(ctrl) };
                // SAFETY: `ctrl` 是刚解析的控制器指针；handler 指针由实例拥有，地址稳定。
                let _ = unsafe { (ctrl_vtbl.set_component_handler)(ctrl, handler_ptr) };
            }
            Ok(kind)
        })();
        match result {
            Ok(kind) => Ok(Self {
                module: std::sync::Arc::clone(&module.inner),
                component,
                processor,
                controller,
                handler,
                host_app,
                name: class_name.to_owned(),
                kind,
                active: false,
                latency: 0,
                sample_rate: 0.0,
                max_block: 0,
            }),
            Err(error) => {
                // SAFETY: 配对释放（已建的才放；空指针不放）。
                unsafe {
                    if let Some(ctrl) = controller {
                        Vst3Module::release(ctrl);
                    }
                    if !processor.is_null() {
                        Vst3Module::release(processor);
                    }
                    if !component.is_null() {
                        Vst3Module::release(component);
                    }
                }
                Err(error)
            }
        }
    }

    /// 总线探针定的粗分类（扫描与加载的乐器判定用）。
    #[must_use]
    pub fn kind(&self) -> PluginKind {
        self.kind
    }

    /// 停用（`setProcessing(false)`＋`setActive(false)`；未激活时为空操作）。
    fn deactivate_inner(&mut self) {
        if !self.active {
            return;
        }
        // SAFETY: `processor`／`component` 是本实例的有效指针。
        unsafe {
            let proc_vtbl = vtbl::<AudioProcessorVtbl>(self.processor);
            let _ = (proc_vtbl.set_processing)(self.processor, 0);
            let comp_vtbl = vtbl::<ComponentVtbl>(self.component);
            let _ = (comp_vtbl.set_active)(self.component, 0);
        }
        self.active = false;
    }
}

impl Drop for Vst3Instance {
    fn drop(&mut self) {
        self.deactivate_inner();
        // SAFETY: 三指针的配对关系见结构体文档；各 `release` 一次。
        // `terminate` 在 `release` 之前（SDK 顺序）。
        unsafe {
            let comp_vtbl = vtbl::<ComponentVtbl>(self.component);
            let _ = (comp_vtbl.base.terminate)(self.component);
            if let Some(ctrl) = self.controller {
                Vst3Module::release(ctrl);
            }
            Vst3Module::release(self.processor);
            Vst3Module::release(self.component);
        }
    }
}

impl PluginInstance for Vst3Instance {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn activate(&mut self, sample_rate: f64, max_block: usize) -> Result<(), HostError> {
        // 已激活：先停用，再按新参数重建（采样率／块上限可变）。
        self.deactivate_inner();
        // SAFETY: `processor`／`component` 是本实例的有效指针。
        unsafe {
            let proc_vtbl = vtbl::<AudioProcessorVtbl>(self.processor);
            // 32 位浮点必须被插件接受（`kResultTrue`＝1；0＝不支持）。
            let code = (proc_vtbl.can_process_sample_size)(self.processor, SAMPLE_32);
            if code != 1 {
                return Err(HostError::SampleSizeUnsupported);
            }
            let setup = ProcessSetup {
                process_mode: PROCESS_REALTIME,
                symbolic_sample_size: SAMPLE_32,
                max_samples_per_block: max_block as i32,
                _pad: 0,
                sample_rate,
            };
            check(
                "setupProcessing",
                (proc_vtbl.setup_processing)(self.processor, &setup),
            )?;
            // 总线排布按 kind 分支（效果器线新增）：一期只接立体声。
            // `setBusArrangements` 的返回值语义是“接受（1）／拒绝（0）”，
            // 不是 ok／error；接受与否以事后 `getBusArrangement` 的实测排布为准。
            // 未知 kind（既无事件输入又无音频输入）无法定排布 ⇒ 响亮失败。
            let mut out_arr = [ARRANGEMENT_STEREO];
            let mut actual: u64 = 0;
            match self.kind {
                PluginKind::Instrument => {
                    let _ = (proc_vtbl.set_bus_arrangements)(
                        self.processor,
                        ptr::null_mut(),
                        0,
                        out_arr.as_mut_ptr(),
                        1,
                    );
                    check(
                        "getBusArrangement",
                        (proc_vtbl.get_bus_arrangement)(self.processor, DIR_OUTPUT, 0, &mut actual),
                    )?;
                    if actual != ARRANGEMENT_STEREO {
                        return Err(HostError::OutputNotStereo);
                    }
                }
                PluginKind::Effect => {
                    let mut in_arr = [ARRANGEMENT_STEREO];
                    let _ = (proc_vtbl.set_bus_arrangements)(
                        self.processor,
                        in_arr.as_mut_ptr(),
                        1,
                        out_arr.as_mut_ptr(),
                        1,
                    );
                    check(
                        "getBusArrangement",
                        (proc_vtbl.get_bus_arrangement)(self.processor, DIR_INPUT, 0, &mut actual),
                    )?;
                    if actual != ARRANGEMENT_STEREO {
                        return Err(HostError::InputNotStereo);
                    }
                    check(
                        "getBusArrangement",
                        (proc_vtbl.get_bus_arrangement)(self.processor, DIR_OUTPUT, 0, &mut actual),
                    )?;
                    if actual != ARRANGEMENT_STEREO {
                        return Err(HostError::OutputNotStereo);
                    }
                }
                PluginKind::Unknown => {
                    return Err(HostError::Other(
                        "插件类未能判定 kind（既无事件输入又无音频输入），无法激活".to_owned(),
                    ));
                }
            }
            let comp_vtbl = vtbl::<ComponentVtbl>(self.component);
            // 主总线激活（失败 ⇒ 响亮失败）：乐器＝事件输入 0 号＋音频输出 0 号；
            // 效果器＝音频输入 0 号＋音频输出 0 号（事件总线一期不接）。
            match self.kind {
                PluginKind::Instrument => {
                    check(
                        "activateBus",
                        (comp_vtbl.activate_bus)(self.component, MEDIA_EVENT, DIR_INPUT, 0, 1),
                    )?;
                }
                PluginKind::Effect => {
                    check(
                        "activateBus",
                        (comp_vtbl.activate_bus)(self.component, MEDIA_AUDIO, DIR_INPUT, 0, 1),
                    )?;
                }
                PluginKind::Unknown => {
                    return Err(HostError::Other(
                        "插件类未能判定 kind，无法激活总线".to_owned(),
                    ));
                }
            }
            check(
                "activateBus",
                (comp_vtbl.activate_bus)(self.component, MEDIA_AUDIO, DIR_OUTPUT, 0, 1),
            )?;
            check("setActive", (comp_vtbl.set_active)(self.component, 1))?;
            check(
                "setProcessing",
                (proc_vtbl.set_processing)(self.processor, 1),
            )?;
            // 延迟在 `setActive(true)` 之后读（SDK 注记：此前读到的可能是旧值）。
            self.latency = (proc_vtbl.get_latency_samples)(self.processor);
        }
        self.active = true;
        self.sample_rate = sample_rate;
        self.max_block = max_block;
        Ok(())
    }

    fn deactivate(&mut self) {
        self.deactivate_inner();
    }

    fn latency_samples(&self) -> u32 {
        self.latency
    }

    fn parameters(&mut self) -> Result<Vec<ParamInfo>, HostError> {
        let Some(ctrl) = self.controller else {
            return Ok(Vec::new());
        };
        // SAFETY: `ctrl` 是本实例的控制器指针。
        unsafe {
            let vtbl = vtbl::<EditControllerVtbl>(ctrl);
            let count = (vtbl.get_parameter_count)(ctrl);
            if count < 0 {
                return Err(HostError::CallFailed {
                    method: "getParameterCount",
                    code: count,
                });
            }
            let mut out = Vec::new();
            for index in 0..count {
                let mut raw = ParameterInfoRaw {
                    id: 0,
                    title: [0; 128],
                    short_title: [0; 128],
                    units: [0; 128],
                    step_count: 0,
                    default_normalized: 0.0,
                    unit_id: 0,
                    flags: 0,
                };
                check(
                    "getParameterInfo",
                    (vtbl.get_parameter_info)(ctrl, index, &mut raw),
                )?;
                out.push(ParamInfo {
                    id: raw.id,
                    title: decode_string128(&raw.title),
                    short_title: decode_string128(&raw.short_title),
                    units: decode_string128(&raw.units),
                    step_count: raw.step_count,
                    default_normalized: raw.default_normalized,
                    flags: raw.flags,
                });
            }
            Ok(out)
        }
    }

    fn parameter_value(&mut self, id: u32) -> Option<f64> {
        let ctrl = self.controller?;
        // SAFETY: `ctrl` 是本实例的控制器指针；`getParamNormalized` 无错误通道。
        unsafe {
            let vtbl = vtbl::<EditControllerVtbl>(ctrl);
            Some((vtbl.get_param_normalized)(ctrl, id))
        }
    }

    fn set_parameter(&mut self, id: u32, value: f64) -> Result<(), HostError> {
        let Some(ctrl) = self.controller else {
            return Err(HostError::InterfaceMissing("IEditController"));
        };
        // SAFETY: `ctrl` 是本实例的控制器指针。
        unsafe {
            let vtbl = vtbl::<EditControllerVtbl>(ctrl);
            check(
                "setParamNormalized",
                (vtbl.set_param_normalized)(ctrl, id, value),
            )
        }
    }

    fn save_state(&mut self) -> Result<PluginState, HostError> {
        let mut proc_stream = MemoryStream::capture();
        // SAFETY: 流对象在调用帧内；插件在 `getState` 返回前只用它。
        let proc_ptr = (&mut proc_stream) as *mut MemoryStream as *mut c_void;
        // SAFETY: `component` 是本实例的组件指针；流对象在调用帧内，插件只在 getState 返回前用它。
        unsafe {
            let comp_vtbl = vtbl::<ComponentVtbl>(self.component);
            check("getState", (comp_vtbl.get_state)(self.component, proc_ptr))?;
        }
        let processor = proc_stream.buf.clone();
        let mut controller = Vec::new();
        if let Some(ctrl) = self.controller {
            let mut ctrl_stream = MemoryStream::capture();
            let ctrl_ptr = (&mut ctrl_stream) as *mut MemoryStream as *mut c_void;
            // SAFETY: `ctrl` 是本实例的控制器指针；流对象在调用帧内。
            unsafe {
                let ctrl_vtbl = vtbl::<EditControllerVtbl>(ctrl);
                check("getState", (ctrl_vtbl.get_state)(ctrl, ctrl_ptr))?;
            }
            controller = ctrl_stream.buf.clone();
        }
        Ok(PluginState {
            processor,
            controller,
        })
    }

    fn restore_state(&mut self, state: &PluginState) -> Result<(), HostError> {
        let mut proc_stream = MemoryStream::restore(&state.processor);
        let proc_ptr = (&mut proc_stream) as *mut MemoryStream as *mut c_void;
        // SAFETY: `component` 是本实例的组件指针；流对象在调用帧内，插件只在 setState 返回前用它。
        unsafe {
            let comp_vtbl = vtbl::<ComponentVtbl>(self.component);
            check("setState", (comp_vtbl.set_state)(self.component, proc_ptr))?;
        }
        if let Some(ctrl) = self.controller {
            // 顺序（SDK 预置加载流）：控制器先拿处理器态同步参数，
            // 再恢复控制器自己的状态；控制器态为空时跳过 `setState`
            // （无物可恢复，个别控制器对空流返回失败）。
            let mut comp_stream = MemoryStream::restore(&state.processor);
            let comp_ptr = (&mut comp_stream) as *mut MemoryStream as *mut c_void;
            // SAFETY: `ctrl` 是本实例的控制器指针；流对象在调用帧内。
            unsafe {
                let ctrl_vtbl = vtbl::<EditControllerVtbl>(ctrl);
                let _ = (ctrl_vtbl.set_component_state)(ctrl, comp_ptr);
                if !state.controller.is_empty() {
                    let mut ctrl_stream = MemoryStream::restore(&state.controller);
                    let ctrl_ptr = (&mut ctrl_stream) as *mut MemoryStream as *mut c_void;
                    check("setState", (ctrl_vtbl.set_state)(ctrl, ctrl_ptr))?;
                }
            }
        }
        Ok(())
    }

    fn process(
        &mut self,
        events: &[MidiEvent],
        frames: usize,
        out_l: &mut [f32],
        out_r: &mut [f32],
    ) -> Result<(), HostError> {
        if !self.active {
            return Err(HostError::NotActivated);
        }
        if self.kind == PluginKind::Effect {
            return Err(HostError::Other(
                "效果器实例不能走乐器 process 路径（须用 process_audio）".to_owned(),
            ));
        }
        if out_l.len() != frames || out_r.len() != frames {
            return Err(HostError::Other(format!(
                "输出片长（{}／{}）≠ 帧数 {frames}",
                out_l.len(),
                out_r.len()
            )));
        }
        if frames > self.max_block {
            return Err(HostError::Other(format!(
                "帧数 {frames} ＞ 激活块上限 {}",
                self.max_block
            )));
        }
        // 事件转换（按块重建，离线路径专用，⛔ 不进 RT）。
        let mut vst_events = Vec::with_capacity(events.len());
        for event in events {
            if (event.sample_offset as usize) >= frames {
                continue; // 越界事件丢弃（调用方应保证窗口内；不响亮失败是为容错）。
            }
            vst_events.push(match event.kind {
                MidiEventKind::NoteOn {
                    channel,
                    pitch,
                    velocity,
                    note_id,
                } => Event::note_on(channel, pitch, velocity, note_id, event.sample_offset),
                MidiEventKind::NoteOff {
                    channel,
                    pitch,
                    velocity,
                    note_id,
                } => Event::note_off(channel, pitch, velocity, note_id, event.sample_offset),
            });
        }
        // 同块同偏移先 off 后 on（ADR-0064 D7：off 优先，避免同音符粘连）；
        // 稳定排序：同偏移同类型保持调度顺序。
        vst_events.sort_by_key(|event| {
            (
                event.sample_offset,
                if event.event_type == EVENT_NOTE_OFF {
                    0
                } else {
                    1
                },
            )
        });
        let mut event_list = HostEventList::new(&vst_events);
        let event_ptr = (&mut event_list) as *mut HostEventList as *mut c_void;
        let mut channel_ptrs: [*mut f32; 2] = [out_l.as_mut_ptr(), out_r.as_mut_ptr()];
        let mut out_bus = AudioBusBuffers {
            num_channels: 2,
            _pad: 0,
            silence_flags: 0,
            channel_buffers: channel_ptrs.as_mut_ptr(),
        };
        let mut data = ProcessData {
            process_mode: PROCESS_REALTIME,
            symbolic_sample_size: SAMPLE_32,
            num_samples: frames as i32,
            num_inputs: 0,
            num_outputs: 1,
            _pad: 0,
            inputs: ptr::null_mut(),
            outputs: &mut out_bus,
            input_events: event_ptr,
            output_events: ptr::null_mut(),
            input_parameter_changes: ptr::null_mut(),
            output_parameter_changes: ptr::null_mut(),
            process_context: ptr::null_mut(),
        };
        // SAFETY: `processor` 是本实例的有效指针；`data` 指向调用帧内的
        // 总线与事件列表，插件在 `process` 返回前只用它们。
        unsafe {
            let proc_vtbl = vtbl::<AudioProcessorVtbl>(self.processor);
            check("process", (proc_vtbl.process)(self.processor, &mut data))
        }
    }

    fn process_audio(
        &mut self,
        in_l: &[f32],
        in_r: &[f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
    ) -> Result<(), HostError> {
        if !self.active {
            return Err(HostError::NotActivated);
        }
        if self.kind != PluginKind::Effect {
            return Err(HostError::NotAnEffect(
                "乐器实例没有音频输入总线，不能走效果器 process_audio 路径".to_owned(),
            ));
        }
        let frames = out_l.len();
        if in_l.len() != frames || in_r.len() != frames || out_r.len() != frames {
            return Err(HostError::Other(format!(
                "效果器片长不对（入 {}／{}，出 {}／{}）",
                in_l.len(),
                in_r.len(),
                out_l.len(),
                out_r.len()
            )));
        }
        if frames > self.max_block {
            return Err(HostError::Other(format!(
                "帧数 {frames} ＞ 激活块上限 {}",
                self.max_block
            )));
        }
        // 输入指针：VST3 契约里输入与输出片分离时插件 ⛔ 不得写输入；
        // 本方法只在输入／输出片分离的前提下被调用（见 trait 文档）。
        // SAFETY: `in_l`／`in_r` 在调用帧内有效；插件在 `process` 返回前只读它们。
        let mut in_ptrs: [*mut f32; 2] = [in_l.as_ptr() as *mut f32, in_r.as_ptr() as *mut f32];
        let mut in_bus = AudioBusBuffers {
            num_channels: 2,
            _pad: 0,
            silence_flags: 0,
            channel_buffers: in_ptrs.as_mut_ptr(),
        };
        let mut out_ptrs: [*mut f32; 2] = [out_l.as_mut_ptr(), out_r.as_mut_ptr()];
        let mut out_bus = AudioBusBuffers {
            num_channels: 2,
            _pad: 0,
            silence_flags: 0,
            channel_buffers: out_ptrs.as_mut_ptr(),
        };
        let mut data = ProcessData {
            process_mode: PROCESS_REALTIME,
            symbolic_sample_size: SAMPLE_32,
            num_samples: frames as i32,
            num_inputs: 1,
            num_outputs: 1,
            _pad: 0,
            inputs: &mut in_bus,
            outputs: &mut out_bus,
            input_events: ptr::null_mut(),
            output_events: ptr::null_mut(),
            input_parameter_changes: ptr::null_mut(),
            output_parameter_changes: ptr::null_mut(),
            process_context: ptr::null_mut(),
        };
        // SAFETY: `processor` 是本实例的有效指针；`data` 指向调用帧内的
        // 总线，插件在 `process` 返回前只用它们。
        unsafe {
            let proc_vtbl = vtbl::<AudioProcessorVtbl>(self.processor);
            check("process", (proc_vtbl.process)(self.processor, &mut data))
        }
    }
}

/// 扫描一个模块：枚举类，并逐个实例化探针定 `kind`（失败的类记 `error`，不中断）。
///
/// # Errors
///
/// 模块打不开／无工厂／无类 ⇒ [`HostError`]（模块级失败）。
pub(crate) fn inspect_module(path: &str) -> Result<(String, Vec<PluginClass>), HostError> {
    let module = Vst3Module::load(path)?;
    let vendor = module.factory_vendor();
    let raw = module.classes_raw();
    if raw.is_empty() {
        return Err(HostError::ClassNotFound("工厂没有登记任何类".to_owned()));
    }
    let mut classes = Vec::new();
    for (cid, name, category) in raw {
        match Vst3Instance::open(&module, &cid, &name) {
            Ok(instance) => classes.push(PluginClass {
                id: cid,
                name,
                category,
                kind: instance.kind(),
                error: None,
            }),
            Err(error) => classes.push(PluginClass {
                id: cid,
                name,
                category,
                kind: PluginKind::Unknown,
                error: Some(error.to_string()),
            }),
        }
    }
    Ok((vendor, classes))
}
