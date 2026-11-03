//! 最小 VST3 测试插件（夹具）：独立手写 COM 布局，⛔ 不 import 宿主代码。
//!
//! 用途：`tests/vst3_loader.rs` 在测试时用 `rustc` 把本文件现场编译成
//! cdylib，再由宿主加载——证明 VST3 ABI 链路（枚举／实例化／MIDI→音频／
//! 参数／状态往返），不依赖测试机上装没装商业插件。
//!
//! 布局来源与宿主同源（Steinberg `vst3_pluginterfaces` MIT 原文），但此处
//! 独立重写：两边按同一 SDK 文本对齐，测试验证的是“宿主按 SDK 行事”，
//! 而不是“宿主与夹具共享同一份（可能错的）定义”。
//!
//! 三个类：
//! - `TestSine`（组合式）：同一对象实现 `IComponent`＋`IAudioProcessor`＋
//!   `IEditController`（`getControllerClassId` 返回假，宿主走组合式分支）。
//! - `TestSineSeparate`（分离式）：组件与控制器是两个对象；控制器经工厂
//!   另建（`getControllerClassId` 返回真 CID，宿主走分离式分支）。
//!   增益共用一个堆单元：组件建实例时把单元指针记在工厂静态区，
//!   控制器建实例时取回它——这是**夹具的简写**，不是 VST3 机制
//!   （真实分离式插件靠 `IConnectionPoint` 同步，宿主一期不支持，
//!   见 ADR-0064 D9 注记；夹具只借此验证分离式建实例与直设参数的链路）。
//! - `TestGain`（组合式效果器）：音频输入 1 条＋音频输出 1 条，无事件输入；
//!   `process` 把立体声输入 × 增益写到立体声输出（直通增益 1.0 时逐位不变）。
//!   宿主探针据此定 kind＝`Effect`，效果器线经它证明 VST3 音频输入路径。
//!
//! 内存规则（夹具专用）：视图与状态对象一律泄漏不释放（`release` 只减计数，
//! 从不 `drop`）：测试进程短命，OS 回收；⛔ 杜绝悬空指针，不求零泄漏。

#![allow(dead_code)]
#![allow(non_snake_case)]

use std::ffi::{c_char, c_void};
use std::ptr;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// 常量（与 SDK 文本一致；TUID 为夹具自造，不与 Steinberg 的 IID 冲突）
// ---------------------------------------------------------------------------

type Tuid = [u8; 16];

const fn tuid(a: u32, b: u32, c: u32, d: u32) -> Tuid {
    [
        (a >> 24) as u8,
        (a >> 16) as u8,
        (a >> 8) as u8,
        a as u8,
        (b >> 24) as u8,
        (b >> 16) as u8,
        (b >> 8) as u8,
        b as u8,
        (c >> 24) as u8,
        (c >> 16) as u8,
        (c >> 8) as u8,
        c as u8,
        (d >> 24) as u8,
        (d >> 16) as u8,
        (d >> 8) as u8,
        d as u8,
    ]
}

const FUNKNOWN_IID: Tuid = tuid(0x0000_0000, 0x0000_0000, 0xC000_0000, 0x0000_0046);
const ICOMPONENT_IID: Tuid = tuid(0xE831_FF31, 0xF2D5_4301, 0x928E_BBEE, 0x2569_7802);
const IAUDIO_PROCESSOR_IID: Tuid = tuid(0x4204_3F99, 0xB7DA_453C, 0xA569_E79D, 0x9AAE_C33D);
const IEDIT_CONTROLLER_IID: Tuid = tuid(0xDCD7_BBE3, 0x7742_448D, 0xA874_AACC, 0x979C_759E);

/// 组合式类标识（夹具自造）。
const CID_COMBINED: Tuid = tuid(0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444);
/// 分离式组件类标识（夹具自造）。
const CID_SEPARATE: Tuid = tuid(0x5555_5555, 0x6666_6666, 0x7777_7777, 0x8888_8888);
/// 分离式控制器类标识（夹具自造）。
const CID_SEPARATE_CTRL: Tuid = tuid(0x9999_9999, 0xAAAA_AAAA, 0xBBBB_BBBB, 0xCCCC_CCCC);
/// 效果器类标识（夹具自造）。
const CID_EFFECT: Tuid = tuid(0xDDDD_DDDD, 0xEEEE_EEEE, 0xFFFF_FFFF, 0x1212_1212);

const RESULT_OK: i32 = 0;
const RESULT_NO_INTERFACE: i32 = -1;
const RESULT_NOT_IMPLEMENTED: i32 = 0x8000_4001_u32 as i32;

// ---------------------------------------------------------------------------
// SDK 结构体（独立重写，布局见宿主 `vst3.rs` 模块文档）
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
struct PFactoryInfo {
    vendor: [u16; 128],
    url: [u16; 128],
    email: [u16; 128],
    flags: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PClassInfo {
    cid: Tuid,
    cardinality: i32,
    category: [u8; 32],
    name: [u8; 64],
}

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

#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessSetup {
    process_mode: i32,
    symbolic_sample_size: i32,
    max_samples_per_block: i32,
    _pad: i32,
    sample_rate: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AudioBusBuffers {
    num_channels: i32,
    _pad: i32,
    silence_flags: u64,
    channel_buffers: *mut *mut f32,
}

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

fn write_string128(out: &mut [u16; 128], text: &str) {
    let units: Vec<u16> = text.encode_utf16().collect();
    let take = units.len().min(127);
    out[..take].copy_from_slice(&units[..take]);
    out[take] = 0;
}

fn write_char_str(out: &mut [u8], text: &str) {
    let bytes = text.as_bytes();
    let take = bytes.len().min(out.len() - 1);
    out[..take].copy_from_slice(&bytes[..take]);
    out[take] = 0;
}

// ---------------------------------------------------------------------------
// 虚表（顺序与 SDK 一致；全部 `extern "system"`）
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
    get_routing_info: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
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
    get_param_string_by_value:
        unsafe extern "system" fn(*mut c_void, u32, f64, *mut u16) -> i32,
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

// ---------------------------------------------------------------------------
// 插件状态与视图
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Voice {
    active: bool,
    freq_hz: f64,
    phase: f64,
    velocity: f32,
    note_id: i32,
}

impl Voice {
    const fn idle() -> Self {
        Self {
            active: false,
            freq_hz: 0.0,
            phase: 0.0,
            velocity: 0.0,
            note_id: 0,
        }
    }
}

/// 一个插件实例的内芯（组合式三视图共享一个；分离式组件与控制器各一个，
/// 但共用同一个 `gain_cell`——见文件头注记）。
struct Inner {
    refcount: u32,
    combined: bool,
    /// 是否效果器（总线排布与 `process` 语义按此分支）。
    effect: bool,
    gain_cell: *mut f32,
    sample_rate: f64,
    active: bool,
    processing: bool,
    voices: [Voice; 16],
}

impl Inner {
    fn new(combined: bool, gain_cell: *mut f32, effect: bool) -> *mut Inner {
        Box::into_raw(Box::new(Inner {
            refcount: 1,
            combined,
            effect,
            gain_cell,
            sample_rate: 48000.0,
            active: false,
            processing: false,
            voices: [Voice::idle(); 16],
        }))
    }

    fn gain(&self) -> f32 {
        unsafe { *self.gain_cell }
    }

    fn set_gain(&self, value: f32) {
        unsafe { *self.gain_cell = value.clamp(0.0, 1.0) };
    }
}

/// 分离式控制器建实例时取回的增益单元（工厂静态区；夹具简写，见文件头）。
/// 存地址（`usize`）而不用裸指针：`Mutex` 要求内容 `Send`，裸指针不是。
static LAST_SEPARATE_GAIN_CELL: Mutex<Option<usize>> = Mutex::new(None);

unsafe fn inner_of(view: *mut c_void) -> *mut Inner {
    // 视图首字段后紧跟 `inner` 指针（三个视图结构体同款布局）。
    unsafe { *(view as *mut *mut Inner).add(1) }
}

unsafe fn add_ref(view: *mut c_void) -> u32 {
    let inner = unsafe { &mut *inner_of(view) };
    inner.refcount += 1;
    inner.refcount
}

unsafe fn release(view: *mut c_void) -> u32 {
    let inner = unsafe { &mut *inner_of(view) };
    inner.refcount = inner.refcount.saturating_sub(1);
    // 泄漏不释放（见文件头内存规则）。
    inner.refcount
}

// 各视图的 query：自身接口返回自身（+1 引用），其余按组合／分离分流。
unsafe extern "system" fn query_component(
    this: *mut c_void,
    iid: *const Tuid,
    out: *mut *mut c_void,
) -> i32 {
    let want = unsafe { *iid };
    let inner = unsafe { &*inner_of(this) };
    if want == FUNKNOWN_IID || want == ICOMPONENT_IID {
        unsafe { add_ref(this) };
        unsafe { *out = this };
        RESULT_OK
    } else if want == IAUDIO_PROCESSOR_IID {
        let view = Box::into_raw(Box::new(ProcessorView {
            vtbl: &PROCESSOR_VTBL,
            inner: inner_of(this),
        }));
        unsafe { add_ref(this) };
        unsafe { *out = view as *mut c_void };
        RESULT_OK
    } else if want == IEDIT_CONTROLLER_IID && inner.combined {
        let view = Box::into_raw(Box::new(ControllerView {
            vtbl: &CONTROLLER_VTBL,
            inner: inner_of(this),
        }));
        unsafe { add_ref(this) };
        unsafe { *out = view as *mut c_void };
        RESULT_OK
    } else {
        RESULT_NO_INTERFACE
    }
}

unsafe extern "system" fn query_processor(
    this: *mut c_void,
    iid: *const Tuid,
    out: *mut *mut c_void,
) -> i32 {
    let want = unsafe { *iid };
    if want == FUNKNOWN_IID || want == IAUDIO_PROCESSOR_IID {
        unsafe { add_ref(this) };
        unsafe { *out = this };
        RESULT_OK
    } else {
        RESULT_NO_INTERFACE
    }
}

unsafe extern "system" fn query_controller(
    this: *mut c_void,
    iid: *const Tuid,
    out: *mut *mut c_void,
) -> i32 {
    let want = unsafe { *iid };
    if want == FUNKNOWN_IID || want == IEDIT_CONTROLLER_IID {
        unsafe { add_ref(this) };
        unsafe { *out = this };
        RESULT_OK
    } else {
        RESULT_NO_INTERFACE
    }
}

unsafe extern "system" fn ref_component(this: *mut c_void) -> u32 {
    unsafe { add_ref(this) }
}
unsafe extern "system" fn rel_component(this: *mut c_void) -> u32 {
    unsafe { release(this) }
}
unsafe extern "system" fn ref_processor(this: *mut c_void) -> u32 {
    unsafe { add_ref(this) }
}
unsafe extern "system" fn rel_processor(this: *mut c_void) -> u32 {
    unsafe { release(this) }
}
unsafe extern "system" fn ref_controller(this: *mut c_void) -> u32 {
    unsafe { add_ref(this) }
}
unsafe extern "system" fn rel_controller(this: *mut c_void) -> u32 {
    unsafe { release(this) }
}

#[repr(C)]
struct ComponentView {
    vtbl: *const ComponentVtbl,
    inner: *mut Inner,
}

#[repr(C)]
struct ProcessorView {
    vtbl: *const AudioProcessorVtbl,
    inner: *mut Inner,
}

#[repr(C)]
struct ControllerView {
    vtbl: *const EditControllerVtbl,
    inner: *mut Inner,
}

// --- 组件方法（组件视图与处理器视图共享 `Inner`；控制器视图只用控制器方法） ---

unsafe extern "system" fn comp_initialize(_this: *mut c_void, _ctx: *mut c_void) -> i32 {
    RESULT_OK
}
unsafe extern "system" fn comp_terminate(_this: *mut c_void) -> i32 {
    RESULT_OK
}
unsafe extern "system" fn comp_get_controller_cid(this: *mut c_void, cid: *mut Tuid) -> i32 {
    let inner = unsafe { &*inner_of(this) };
    if inner.combined {
        0 // 假：宿主走组合式分支。
    } else {
        unsafe { *cid = CID_SEPARATE_CTRL };
        1 // 真：宿主走分离式分支。
    }
}
unsafe extern "system" fn comp_set_io_mode(_this: *mut c_void, _mode: i32) -> i32 {
    RESULT_OK
}
unsafe extern "system" fn comp_get_bus_count(this: *mut c_void, media: i32, dir: i32) -> i32 {
    let inner = unsafe { &*inner_of(this) };
    if inner.effect {
        return match (media, dir) {
            (0, 0) => 1, // 音频输入 1 条。
            (0, 1) => 1, // 音频输出 1 条。
            _ => 0,
        };
    }
    match (media, dir) {
        (1, 0) => 1, // 事件输入 1 条。
        (0, 1) => 1, // 音频输出 1 条。
        _ => 0,
    }
}
unsafe extern "system" fn comp_get_bus_info(
    this: *mut c_void,
    media: i32,
    dir: i32,
    index: i32,
    info: *mut BusInfo,
) -> i32 {
    if index != 0 {
        return 0;
    }
    let inner = unsafe { &*inner_of(this) };
    let (channel_count, name) = if inner.effect {
        match (media, dir) {
            (0, 0) => (2, "Audio In"),
            (0, 1) => (2, "Audio Out"),
            _ => return 0,
        }
    } else {
        match (media, dir) {
            (1, 0) => (1, "Event In"),
            (0, 1) => (2, "Audio Out"),
            _ => return 0,
        }
    };
    let slot = unsafe { &mut *info };
    slot.media_type = media;
    slot.direction = dir;
    slot.channel_count = channel_count;
    write_string128(&mut slot.name, name);
    slot.bus_type = 0;
    slot.flags = 1;
    RESULT_OK
}
unsafe extern "system" fn comp_get_routing(_this: *mut c_void, _a: *mut c_void, _b: *mut c_void) -> i32 {
    RESULT_NOT_IMPLEMENTED
}
unsafe extern "system" fn comp_activate_bus(
    _this: *mut c_void,
    _m: i32,
    _d: i32,
    _i: i32,
    _s: u8,
) -> i32 {
    RESULT_OK
}
unsafe extern "system" fn comp_set_active(this: *mut c_void, state: u8) -> i32 {
    let inner = unsafe { &mut *inner_of(this) };
    inner.active = state != 0;
    RESULT_OK
}

unsafe fn stream_write_gain(stream: *mut c_void, gain: f32) -> i32 {
    let vtbl = unsafe { &*(*(stream as *mut *const BStreamVtbl)) };
    let bytes = gain.to_le_bytes();
    let mut written: i32 = 0;
    unsafe { (vtbl.write)(stream, bytes.as_ptr() as *const c_void, 4, &mut written) }
}

unsafe fn stream_read_gain(stream: *mut c_void) -> Option<f32> {
    let vtbl = unsafe { &*(*(stream as *mut *const BStreamVtbl)) };
    let mut buf = [0_u8; 4];
    let mut read: i32 = 0;
    let code = unsafe { (vtbl.read)(stream, buf.as_mut_ptr() as *mut c_void, 4, &mut read) };
    if code < 0 || read != 4 {
        None
    } else {
        Some(f32::from_le_bytes(buf))
    }
}

unsafe extern "system" fn comp_set_state(this: *mut c_void, stream: *mut c_void) -> i32 {
    let inner = unsafe { &*inner_of(this) };
    match unsafe { stream_read_gain(stream) } {
        Some(gain) => {
            inner.set_gain(gain);
            RESULT_OK
        }
        None => RESULT_NOT_IMPLEMENTED,
    }
}
unsafe extern "system" fn comp_get_state(this: *mut c_void, stream: *mut c_void) -> i32 {
    let inner = unsafe { &*inner_of(this) };
    unsafe { stream_write_gain(stream, inner.gain()) }
}
unsafe extern "system" fn ctrl_set_component_state(this: *mut c_void, stream: *mut c_void) -> i32 {
    unsafe { comp_set_state(this, stream) }
}
unsafe extern "system" fn ctrl_set_state(this: *mut c_void, stream: *mut c_void) -> i32 {
    unsafe { comp_set_state(this, stream) }
}
unsafe extern "system" fn ctrl_get_state(this: *mut c_void, stream: *mut c_void) -> i32 {
    unsafe { comp_get_state(this, stream) }
}

// --- 处理器方法 ---

unsafe extern "system" fn proc_set_bus_arrangements(
    _this: *mut c_void,
    inputs: *mut u64,
    num_ins: i32,
    outputs: *mut u64,
    num_outs: i32,
) -> i32 {
    if num_outs < 1 || unsafe { *outputs } != 0x3 {
        return 0;
    }
    if num_ins >= 1 && unsafe { *inputs } != 0x3 {
        return 0;
    }
    1
}
unsafe extern "system" fn proc_get_bus_arrangement(
    this: *mut c_void,
    dir: i32,
    index: i32,
    arr: *mut u64,
) -> i32 {
    let inner = unsafe { &*inner_of(this) };
    if index == 0 && ((dir == 1) || (dir == 0 && inner.effect)) {
        unsafe { *arr = 0x3 };
        RESULT_OK
    } else {
        RESULT_NOT_IMPLEMENTED
    }
}
unsafe extern "system" fn proc_can_process(_this: *mut c_void, size: i32) -> i32 {
    if size == 0 {
        1
    } else {
        0
    }
}
unsafe extern "system" fn proc_latency(_this: *mut c_void) -> u32 {
    0
}
unsafe extern "system" fn proc_setup(this: *mut c_void, setup: *const ProcessSetup) -> i32 {
    let inner = unsafe { &mut *inner_of(this) };
    inner.sample_rate = unsafe { (*setup).sample_rate };
    RESULT_OK
}
unsafe extern "system" fn proc_set_processing(this: *mut c_void, state: u8) -> i32 {
    let inner = unsafe { &mut *inner_of(this) };
    inner.processing = state != 0;
    RESULT_OK
}
unsafe extern "system" fn proc_process(this: *mut c_void, data: *mut ProcessData) -> i32 {
    let inner = unsafe { &mut *inner_of(this) };
    let data = unsafe { &mut *data };
    let frames = data.num_samples.max(0) as usize;
    // 效果器分支：立体声输入 × 增益 → 立体声输出（不读事件）。
    if inner.effect {
        if data.num_inputs < 1 || data.inputs.is_null() || data.num_outputs < 1 || data.outputs.is_null()
        {
            return RESULT_NOT_IMPLEMENTED;
        }
        let in_bus = unsafe { &*data.inputs };
        let out_bus = unsafe { &*data.outputs };
        if in_bus.num_channels < 2
            || in_bus.channel_buffers.is_null()
            || out_bus.num_channels < 2
            || out_bus.channel_buffers.is_null()
        {
            return RESULT_NOT_IMPLEMENTED;
        }
        let gain = inner.gain();
        let in_l = unsafe { *in_bus.channel_buffers };
        let in_r = unsafe { *in_bus.channel_buffers.add(1) };
        let out_l = unsafe { *out_bus.channel_buffers };
        let out_r = unsafe { *out_bus.channel_buffers.add(1) };
        for sample in 0..frames {
            unsafe {
                *out_l.add(sample) = *in_l.add(sample) * gain;
                *out_r.add(sample) = *in_r.add(sample) * gain;
            }
        }
        return RESULT_OK;
    }
    // 事件：读宿主事件列表（虚表布局与 SDK 一致），按偏移收集后逐采样应用
    // （插件负责块内偏移精度：偏移 N 的事件在采样 N 之前生效）。
    let mut pending: Vec<(u32, i32, u8, f32, i32)> = Vec::new();
    if !data.input_events.is_null() {
        let list = data.input_events;
        let vtbl = unsafe { &*(*(list as *mut *const EventListVtbl)) };
        let count = unsafe { (vtbl.get_event_count)(list) };
        for index in 0..count {
            let mut event = Event {
                event_type: 0,
                bus_index: 0,
                sample_offset: 0,
                _pad: 0,
                ppq_position: 0.0,
                flags: 0,
                _pad2: 0,
                data: [0; 20],
            };
            let code = unsafe { (vtbl.get_event)(list, index, &mut event) };
            if code < 0 {
                continue;
            }
            let pitch = i16::from_le_bytes([event.data[2], event.data[3]]) as u8;
            let velocity = f32::from_le_bytes([
                event.data[4],
                event.data[5],
                event.data[6],
                event.data[7],
            ]);
            let note_id = match event.event_type {
                0 => i32::from_le_bytes([
                    event.data[16],
                    event.data[17],
                    event.data[18],
                    event.data[19],
                ]),
                _ => i32::from_le_bytes([
                    event.data[8],
                    event.data[9],
                    event.data[10],
                    event.data[11],
                ]),
            };
            pending.push((
                event.sample_offset.max(0) as u32,
                event.event_type,
                pitch,
                velocity,
                note_id,
            ));
        }
    }
    pending.sort_by_key(|event| event.0);
    // 输出：立体声总线 0 号，双声道写同一正弦混音。
    if data.num_outputs < 1 || data.outputs.is_null() {
        return RESULT_NOT_IMPLEMENTED;
    }
    let out_bus = unsafe { &*data.outputs };
    if out_bus.num_channels < 2 || out_bus.channel_buffers.is_null() {
        return RESULT_NOT_IMPLEMENTED;
    }
    let left = unsafe { *out_bus.channel_buffers };
    let right = unsafe { *out_bus.channel_buffers.add(1) };
    let gain = inner.gain();
    let mut next = 0_usize;
    for sample in 0..frames {
        while next < pending.len() && (pending[next].0 as usize) == sample {
            let (_, event_type, pitch, velocity, note_id) = pending[next];
            if event_type == 0 {
                let freq = 440.0 * 2_f64.powf((f64::from(pitch) - 69.0) / 12.0);
                for voice in inner.voices.iter_mut() {
                    if !voice.active || voice.note_id == note_id {
                        *voice = Voice {
                            active: true,
                            freq_hz: freq,
                            phase: 0.0,
                            velocity,
                            note_id,
                        };
                        break;
                    }
                }
            } else if event_type == 1 {
                for voice in inner.voices.iter_mut() {
                    if voice.active && voice.note_id == note_id {
                        voice.active = false;
                    }
                }
            }
            next += 1;
        }
        let mut mixed = 0.0_f32;
        for voice in inner.voices.iter_mut() {
            if !voice.active {
                continue;
            }
            mixed += voice.phase.sin() as f32 * voice.velocity;
            voice.phase += 2.0 * std::f64::consts::PI * voice.freq_hz / inner.sample_rate;
            if voice.phase >= 2.0 * std::f64::consts::PI {
                voice.phase %= 2.0 * std::f64::consts::PI;
            }
        }
        let value = mixed * gain;
        unsafe {
            *left.add(sample) = value;
            *right.add(sample) = value;
        }
    }
    RESULT_OK
}
unsafe extern "system" fn proc_tail(_this: *mut c_void) -> u32 {
    0
}

// --- 控制器方法 ---

unsafe extern "system" fn ctrl_get_param_count(_this: *mut c_void) -> i32 {
    1
}
unsafe extern "system" fn ctrl_get_param_info(
    this: *mut c_void,
    index: i32,
    info: *mut ParameterInfoRaw,
) -> i32 {
    if index != 0 {
        return 0;
    }
    let inner = unsafe { &*inner_of(this) };
    let slot = unsafe { &mut *info };
    slot.id = 0;
    write_string128(&mut slot.title, "Gain");
    write_string128(&mut slot.short_title, "Gain");
    write_string128(&mut slot.units, "");
    slot.step_count = 0;
    // 缺省与工厂建实例的增益单元一致（乐器 0.8／效果器 1.0）。
    slot.default_normalized = if inner.effect { 1.0 } else { 0.8 };
    slot.unit_id = 0;
    slot.flags = 1;
    RESULT_OK
}
unsafe extern "system" fn ctrl_get_param_string(
    _this: *mut c_void,
    _id: u32,
    value: f64,
    string: *mut u16,
) -> i32 {
    let text = format!("{value:.4}");
    let units: Vec<u16> = text.encode_utf16().collect();
    let take = units.len().min(127);
    unsafe {
        ptr::copy_nonoverlapping(units.as_ptr(), string, take);
        *string.add(take) = 0;
    }
    RESULT_OK
}
unsafe extern "system" fn ctrl_get_param_value(
    _this: *mut c_void,
    _id: u32,
    _string: *const u16,
    _value: *mut f64,
) -> i32 {
    RESULT_NOT_IMPLEMENTED
}
unsafe extern "system" fn ctrl_norm_to_plain(_this: *mut c_void, _id: u32, v: f64) -> f64 {
    v
}
unsafe extern "system" fn ctrl_plain_to_norm(_this: *mut c_void, _id: u32, v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}
unsafe extern "system" fn ctrl_get_param_norm(this: *mut c_void, _id: u32) -> f64 {
    let inner = unsafe { &*inner_of(this) };
    f64::from(inner.gain())
}
unsafe extern "system" fn ctrl_set_param_norm(this: *mut c_void, _id: u32, value: f64) -> i32 {
    let inner = unsafe { &*inner_of(this) };
    inner.set_gain(value as f32);
    RESULT_OK
}
unsafe extern "system" fn ctrl_set_handler(_this: *mut c_void, _handler: *mut c_void) -> i32 {
    RESULT_OK
}
unsafe extern "system" fn ctrl_create_view(_this: *mut c_void, _name: *const c_char) -> *mut c_void {
    ptr::null_mut()
}

// --- 虚表静态量 ---

static COMPONENT_VTBL: ComponentVtbl = ComponentVtbl {
    base: PluginBaseVtbl {
        base: FUnknownVtbl {
            query_interface: query_component,
            add_ref: ref_component,
            release: rel_component,
        },
        initialize: comp_initialize,
        terminate: comp_terminate,
    },
    get_controller_class_id: comp_get_controller_cid,
    set_io_mode: comp_set_io_mode,
    get_bus_count: comp_get_bus_count,
    get_bus_info: comp_get_bus_info,
    get_routing_info: comp_get_routing,
    activate_bus: comp_activate_bus,
    set_active: comp_set_active,
    set_state: comp_set_state,
    get_state: comp_get_state,
};

static PROCESSOR_VTBL: AudioProcessorVtbl = AudioProcessorVtbl {
    base: PluginBaseVtbl {
        base: FUnknownVtbl {
            query_interface: query_processor,
            add_ref: ref_processor,
            release: rel_processor,
        },
        initialize: comp_initialize,
        terminate: comp_terminate,
    },
    set_bus_arrangements: proc_set_bus_arrangements,
    get_bus_arrangement: proc_get_bus_arrangement,
    can_process_sample_size: proc_can_process,
    get_latency_samples: proc_latency,
    setup_processing: proc_setup,
    set_processing: proc_set_processing,
    process: proc_process,
    get_tail_samples: proc_tail,
};

static CONTROLLER_VTBL: EditControllerVtbl = EditControllerVtbl {
    base: PluginBaseVtbl {
        base: FUnknownVtbl {
            query_interface: query_controller,
            add_ref: ref_controller,
            release: rel_controller,
        },
        initialize: comp_initialize,
        terminate: comp_terminate,
    },
    set_component_state: ctrl_set_component_state,
    set_state: ctrl_set_state,
    get_state: ctrl_get_state,
    get_parameter_count: ctrl_get_param_count,
    get_parameter_info: ctrl_get_param_info,
    get_param_string_by_value: ctrl_get_param_string,
    get_param_value_by_string: ctrl_get_param_value,
    normalized_param_to_plain: ctrl_norm_to_plain,
    plain_param_to_normalized: ctrl_plain_to_norm,
    get_param_normalized: ctrl_get_param_norm,
    set_param_normalized: ctrl_set_param_norm,
    set_component_handler: ctrl_set_handler,
    create_view: ctrl_create_view,
};

// --- 工厂 ---

#[repr(C)]
struct FactoryObj {
    vtbl: *const PluginFactoryVtbl,
}

// SAFETY: 工厂虚表指针指向静态虚表，永不修改；工厂对象本身无可变状态。
unsafe impl Sync for FactoryObj {}

unsafe extern "system" fn factory_query(
    this: *mut c_void,
    iid: *const Tuid,
    out: *mut *mut c_void,
) -> i32 {
    let want = unsafe { *iid };
    if want == FUNKNOWN_IID {
        unsafe { *out = this };
        RESULT_OK
    } else {
        RESULT_NO_INTERFACE
    }
}
unsafe extern "system" fn factory_ref(_this: *mut c_void) -> u32 {
    1
}
unsafe extern "system" fn factory_get_info(_this: *mut c_void, info: *mut PFactoryInfo) -> i32 {
    let slot = unsafe { &mut *info };
    write_string128(&mut slot.vendor, "Yeban Test");
    write_string128(&mut slot.url, "");
    write_string128(&mut slot.email, "");
    slot.flags = 0;
    RESULT_OK
}
unsafe extern "system" fn factory_count(_this: *mut c_void) -> i32 {
    3
}
unsafe extern "system" fn factory_get_class(_this: *mut c_void, index: i32, info: *mut PClassInfo) -> i32 {
    let slot = unsafe { &mut *info };
    slot.cardinality = 0;
    match index {
        0 => {
            slot.cid = CID_COMBINED;
            write_char_str(&mut slot.category, "Audio Module Class");
            write_char_str(&mut slot.name, "TestSine");
        }
        1 => {
            slot.cid = CID_SEPARATE;
            write_char_str(&mut slot.category, "Audio Module Class");
            write_char_str(&mut slot.name, "TestSineSeparate");
        }
        2 => {
            slot.cid = CID_EFFECT;
            write_char_str(&mut slot.category, "Audio Module Class");
            write_char_str(&mut slot.name, "TestGain");
        }
        _ => return 0,
    }
    RESULT_OK
}
unsafe extern "system" fn factory_create(
    _this: *mut c_void,
    cid: *const Tuid,
    iid: *const Tuid,
    out: *mut *mut c_void,
) -> i32 {
    let cid = unsafe { *cid };
    let iid = unsafe { *iid };
    if cid == CID_EFFECT {
        // 效果器走组合式控制器（与 TestSine 同款视图分发，只换总线与处理语义）。
        let gain_cell = Box::into_raw(Box::new(1.0_f32));
        let inner = Inner::new(true, gain_cell, true);
        if iid == ICOMPONENT_IID {
            let view = Box::into_raw(Box::new(ComponentView {
                vtbl: &COMPONENT_VTBL,
                inner,
            }));
            unsafe { *out = view as *mut c_void };
            RESULT_OK
        } else if iid == IAUDIO_PROCESSOR_IID {
            let view = Box::into_raw(Box::new(ProcessorView {
                vtbl: &PROCESSOR_VTBL,
                inner,
            }));
            unsafe { *out = view as *mut c_void };
            RESULT_OK
        } else if iid == IEDIT_CONTROLLER_IID {
            let view = Box::into_raw(Box::new(ControllerView {
                vtbl: &CONTROLLER_VTBL,
                inner,
            }));
            unsafe { *out = view as *mut c_void };
            RESULT_OK
        } else {
            RESULT_NO_INTERFACE
        }
    } else if cid == CID_COMBINED {
        let gain_cell = Box::into_raw(Box::new(0.8_f32));
        let inner = Inner::new(true, gain_cell, false);
        if iid == ICOMPONENT_IID {
            let view = Box::into_raw(Box::new(ComponentView {
                vtbl: &COMPONENT_VTBL,
                inner,
            }));
            unsafe { *out = view as *mut c_void };
            RESULT_OK
        } else if iid == IAUDIO_PROCESSOR_IID {
            let view = Box::into_raw(Box::new(ProcessorView {
                vtbl: &PROCESSOR_VTBL,
                inner,
            }));
            unsafe { *out = view as *mut c_void };
            RESULT_OK
        } else if iid == IEDIT_CONTROLLER_IID {
            let view = Box::into_raw(Box::new(ControllerView {
                vtbl: &CONTROLLER_VTBL,
                inner,
            }));
            unsafe { *out = view as *mut c_void };
            RESULT_OK
        } else {
            RESULT_NO_INTERFACE
        }
    } else if cid == CID_SEPARATE {
        if iid != ICOMPONENT_IID {
            return RESULT_NO_INTERFACE;
        }
        let gain_cell = Box::into_raw(Box::new(0.8_f32));
        if let Ok(mut guard) = LAST_SEPARATE_GAIN_CELL.lock() {
            *guard = Some(gain_cell as usize);
        }
        let inner = Inner::new(false, gain_cell, false);
        let view = Box::into_raw(Box::new(ComponentView {
            vtbl: &COMPONENT_VTBL,
            inner,
        }));
        unsafe { *out = view as *mut c_void };
        RESULT_OK
    } else if cid == CID_SEPARATE_CTRL {
        if iid != IEDIT_CONTROLLER_IID {
            return RESULT_NO_INTERFACE;
        }
        let gain_cell = match LAST_SEPARATE_GAIN_CELL.lock() {
            Ok(guard) => guard
                .map(|addr| addr as *mut f32)
                .unwrap_or_else(|| Box::into_raw(Box::new(0.8_f32))),
            Err(_) => Box::into_raw(Box::new(0.8_f32)),
        };
        let inner = Inner::new(false, gain_cell, false);
        let view = Box::into_raw(Box::new(ControllerView {
            vtbl: &CONTROLLER_VTBL,
            inner,
        }));
        unsafe { *out = view as *mut c_void };
        RESULT_OK
    } else {
        RESULT_NO_INTERFACE
    }
}

static FACTORY_VTBL: PluginFactoryVtbl = PluginFactoryVtbl {
    base: FUnknownVtbl {
        query_interface: factory_query,
        add_ref: factory_ref,
        release: factory_ref,
    },
    get_factory_info: factory_get_info,
    count_classes: factory_count,
    get_class_info: factory_get_class,
    create_instance: factory_create,
};

static FACTORY: FactoryObj = FactoryObj {
    vtbl: &FACTORY_VTBL,
};

#[no_mangle]
pub extern "system" fn GetPluginFactory() -> *mut c_void {
    &FACTORY as *const FactoryObj as *mut c_void
}
