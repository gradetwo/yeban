//! 最小 ARA 测试插件（夹具）：纯 Rust `cdylib`，独立手写布局，⛔ 不 import 宿主代码。
//!
//! 用途：宿主 `ara.rs` 的夹具测试在测试时用 `rustc` 把本文件现场编译成
//! cdylib，再由宿主加载——验证 Phase B ② 的完整通路（绑定→分析完成→
//! 内容查询返回音符），不依赖测试机上装没装真插件（CI 机器上无真插件）。
//!
//! 布局来源与宿主同源（Celemony ARA API Apache 2.0 原文、Steinberg
//! `vst3_pluginterfaces` MIT 原文），但此处独立重写：两边按同一 SDK 文本
//! 对齐，测试验证的是“宿主按 SDK 行事”，而不是“宿主与夹具共享同一份
//! （可能错的）定义”。
//!
//! 夹具提供的类（VST3 工厂枚举两个类，类名配对是宿主 `open()` 的要求）：
//! - `TestAra`（`ARA Main Factory Class`）：实现 `ARA::IMainFactory`，
//!   `getFactory` 返回静态 `ARAFactory`（128 字节，世代 3–4，可分析类型仅音符）。
//! - `TestAra`（`Audio Module Class`）：类名配对占位符，实例只实现 `FUnknown`；
//!   经它查询任何音频接口都响亮返回 `NO_INTERFACE`。② 通路不需要处理器，
//!   渲染（③）已由 PitchNet 真插件验证，不在本夹具范围。
//!
//! 内容模型（⛔ 如实说明，不得夸大）：夹具发布的是**固定 8 音符集**，不是
//! 音频分析结果。固定集与宿主测试的阶梯正弦代用信号同音高（MIDI 57、60、
//! 62、64、65、67、69、72，各 0.5 秒），测试断言查询结果与固定集逐项一致。
//! 音频通路并非完全旁路：分析请求时夹具经宿主 AudioAccess 接口实读音频，
//! 读不到或全静默 ⇒ 内容永不可用 ⇒ 宿主等待响亮超时（fail-closed），
//! 不静默发布假音符。
//!
//! 内存规则（夹具专用，与 `fixture_plugin.rs` 同款）：对象一律泄漏不释放
//! （`release` 只减计数，从不 `drop`）：测试进程短命，OS 回收；
//! ⛔ 杜绝悬空指针，不求零泄漏。`destroy_document_controller` 会把宿主侧
//! 指针清零，此后任何调用都 fail-closed。
//!
//! 边界（设计 §7.6）：自洽测试（宿主＋自家夹具）不能证明对真实插件的 ABI
//! 正确。夹具只验证机制，不能替代真插件放行，真插件门仍敞开。② 的
//! PitchNet 失败仍记为插件限制，不因夹具通过而改写。

#![allow(dead_code)]
#![allow(non_snake_case)]

use std::ffi::{c_char, c_void};
use std::ptr;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// 常量（与 SDK 文本一致；TUID／CID 为夹具自造，不与 Steinberg 的 IID 冲突）
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
/// `ARA::IMainFactory` 的 IID（ARAVST3.h `DECLARE_CLASS_IID` 原文）。
const IMAIN_FACTORY_IID: Tuid = tuid(0xDB2A_1669, 0xFAFD_42A5, 0xA82F_864F, 0x7B68_72EA);

/// ARA 主工厂类标识（夹具自造）。
const CID_ARA_MAIN: Tuid = tuid(0xA1A1_A1A1, 0xB2B2_B2B2, 0xC3C3_C3C3, 0xD4D4_D4D4);
/// 配对音频模块类标识（夹具自造，与 ARA 类同名 `TestAra`）。
const CID_PROC: Tuid = tuid(0xE5E5_E5E5, 0xF6F6_F6F6, 0x1717_1717, 0x2828_2828);

const RESULT_OK: i32 = 0;
const RESULT_NO_INTERFACE: i32 = -1;

/// ARA API 世代：2.0 草案（下限）。
const API_GENERATION_2_0_DRAFT: i32 = 3;
/// ARA API 世代：2.0 正式版。
const API_GENERATION_2_0_FINAL: i32 = 4;
/// 内容类型：音符（`kARAContentTypeNotes`）。
const CONTENT_TYPE_NOTES: i32 = 10;
/// 内容等级：已检测（自动分析，未经人工复核）。
const CONTENT_GRADE_DETECTED: i32 = 1;

type AraSize = usize;
type AraBool = i32;
type AraRef = *mut c_void;

const ARA_TRUE: AraBool = 1;
const ARA_FALSE: AraBool = 0;

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
// VST3 工厂侧（独立重写，布局见宿主 `vst3.rs` 模块文档）
// ---------------------------------------------------------------------------

#[repr(C)]
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
struct FactoryObj {
    vtbl: &'static PluginFactoryVtbl,
}

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
    2
}

unsafe extern "system" fn factory_get_class(
    _this: *mut c_void,
    index: i32,
    info: *mut PClassInfo,
) -> i32 {
    let slot = unsafe { &mut *info };
    slot.cardinality = 0;
    match index {
        0 => {
            slot.cid = CID_ARA_MAIN;
            write_char_str(&mut slot.category, "ARA Main Factory Class");
            write_char_str(&mut slot.name, "TestAra");
        }
        1 => {
            slot.cid = CID_PROC;
            write_char_str(&mut slot.category, "Audio Module Class");
            write_char_str(&mut slot.name, "TestAra");
        }
        _ => return RESULT_NO_INTERFACE,
    }
    RESULT_OK
}

/// 配对占位处理器对象：只实现 `FUnknown`，任何音频接口查询都响亮失败。
#[repr(C)]
struct PlaceholderObj {
    vtbl: &'static FUnknownVtbl,
}

unsafe extern "system" fn placeholder_query(
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

unsafe extern "system" fn placeholder_ref(_this: *mut c_void) -> u32 {
    1
}

static PLACEHOLDER_VTBL: FUnknownVtbl = FUnknownVtbl {
    query_interface: placeholder_query,
    add_ref: placeholder_ref,
    release: placeholder_ref,
};

unsafe extern "system" fn factory_create(
    _this: *mut c_void,
    cid: *const Tuid,
    iid: *const Tuid,
    out: *mut *mut c_void,
) -> i32 {
    let cid = unsafe { *cid };
    let iid = unsafe { *iid };
    if cid == CID_ARA_MAIN && iid == IMAIN_FACTORY_IID {
        let view = Box::into_raw(Box::new(MainFactoryObj {
            vtbl: &MAIN_FACTORY_VTBL,
        }));
        unsafe { *out = view as *mut c_void };
        RESULT_OK
    } else if cid == CID_PROC && iid == FUNKNOWN_IID {
        let view = Box::into_raw(Box::new(PlaceholderObj {
            vtbl: &PLACEHOLDER_VTBL,
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

// ---------------------------------------------------------------------------
// ARA 结构体（独立重写；属性类一律 `packed(4)`，与宿主 `ara.rs` 的实测一致：
// 标准 C 布局会把 MusicalContext 的 color 读成填充垃圾指针导致段错误）
// ---------------------------------------------------------------------------

/// `ARAColor`：r／g／b 三个 f32（夹具只收空指针，布局备用）。
#[repr(C)]
#[derive(Clone, Copy)]
struct AraColor {
    red: f32,
    green: f32,
    blue: f32,
}

/// `ARADocumentProperties`。
#[repr(C, packed(4))]
struct AraDocumentProperties {
    struct_size: AraSize,
    name: *const c_char,
}

/// `ARAMusicalContextProperties`（2.0 全字段；`name` 可空）。
#[repr(C, packed(4))]
struct AraMusicalContextProperties {
    struct_size: AraSize,
    name: *const c_char,
    order_index: i32,
    color: *const AraColor,
}

/// `ARARegionSequenceProperties`。
#[repr(C, packed(4))]
struct AraRegionSequenceProperties {
    struct_size: AraSize,
    name: *const c_char,
    order_index: i32,
    musical_context_ref: AraRef,
    color: *const AraColor,
}

/// `ARAAudioSourceProperties`（2.0 Final 全字段）。
#[repr(C, packed(4))]
struct AraAudioSourceProperties {
    struct_size: AraSize,
    name: *const c_char,
    persistent_id: *const c_char,
    sample_count: i64,
    sample_rate: f64,
    channel_count: i32,
    merits_64_bit_samples: AraBool,
    channel_arrangement_data_type: i32,
    channel_arrangement: *const c_void,
}

/// `ARAAudioModificationProperties`。
#[repr(C, packed(4))]
struct AraAudioModificationProperties {
    struct_size: AraSize,
    name: *const c_char,
    persistent_id: *const c_char,
}

/// `ARAPlaybackRegionProperties`（2.0 全字段）。
#[repr(C, packed(4))]
struct AraPlaybackRegionProperties {
    struct_size: AraSize,
    transformation_flags: i32,
    start_in_modification_time: f64,
    duration_in_modification_time: f64,
    start_in_playback_time: f64,
    duration_in_playback_time: f64,
    musical_context_ref: AraRef,
    region_sequence_ref: AraRef,
    name: *const c_char,
    color: *const AraColor,
}

/// `ARAContentNote`（内容读取器返回的音符；字段顺序见头文件原文）。
#[repr(C)]
#[derive(Clone, Copy)]
struct AraContentNoteRaw {
    frequency: f32,
    pitch_number: i32,
    volume: f32,
    start_position: f64,
    attack_duration: f64,
    note_duration: f64,
    signal_duration: f64,
}

/// `ARAContentTimeRange`（内容读取范围；空指针＝全部）。
#[repr(C)]
struct AraContentTimeRange {
    start: f64,
    duration: f64,
}

/// `ARAInterfaceConfiguration`。
#[repr(C, packed(4))]
struct AraInterfaceConfiguration {
    struct_size: AraSize,
    desired_api_generation: i32,
    assert_function_address: *const c_void,
}

/// `ARAFactory`（64 位布局 128 字节；字段顺序见 ARAInterface.h 原文）。
#[repr(C)]
struct AraFactoryRaw {
    struct_size: AraSize,
    lowest_supported_api_generation: i32,
    highest_supported_api_generation: i32,
    factory_id: *const c_char,
    initialize_ara_with_configuration:
        Option<unsafe extern "C" fn(*const AraInterfaceConfiguration)>,
    uninitialize_ara: Option<unsafe extern "C" fn()>,
    plug_in_name: *const c_char,
    manufacturer_name: *const c_char,
    information_url: *const c_char,
    version: *const c_char,
    create_document_controller_with_document: Option<
        unsafe extern "C" fn(
            *const AraDocumentControllerHostInstance,
            *const AraDocumentProperties,
        ) -> *const AraDocumentControllerInstance,
    >,
    document_archive_id: *const c_char,
    compatible_document_archive_ids_count: AraSize,
    compatible_document_archive_ids: *const *const c_char,
    analyzeable_content_types_count: AraSize,
    analyzeable_content_types: *const i32,
    supported_playback_transformation_flags: i32,
    supports_storing_audio_file_chunks: AraBool,
}

/// `ARADocumentControllerInstance`（controller ref＋接口指针）。
#[repr(C)]
struct AraDocumentControllerInstance {
    struct_size: AraSize,
    document_controller_ref: AraRef,
    document_controller_interface: *const AraDocumentControllerInterface,
}

/// `ARADocumentControllerInterface`（完整顺序，见 ARAInterface.h；
/// 未调用的槽位也按原文签名占位，⛔ 只填布局、不省略）。
#[repr(C)]
struct AraDocumentControllerInterface {
    struct_size: AraSize,
    destroy_document_controller: unsafe extern "C" fn(AraRef),
    get_factory: unsafe extern "C" fn(AraRef) -> *const AraFactoryRaw,
    begin_editing: unsafe extern "C" fn(AraRef),
    end_editing: unsafe extern "C" fn(AraRef),
    notify_model_updates: unsafe extern "C" fn(AraRef),
    begin_restoring_document_from_archive: unsafe extern "C" fn(AraRef, AraRef) -> AraBool,
    end_restoring_document_from_archive: unsafe extern "C" fn(AraRef, AraRef) -> AraBool,
    store_document_to_archive: unsafe extern "C" fn(AraRef, AraRef) -> AraBool,
    update_document_properties: unsafe extern "C" fn(AraRef, *const AraDocumentProperties),
    create_musical_context:
        unsafe extern "C" fn(AraRef, AraRef, *const AraMusicalContextProperties) -> AraRef,
    update_musical_context_properties:
        unsafe extern "C" fn(AraRef, AraRef, *const AraMusicalContextProperties),
    update_musical_context_content:
        unsafe extern "C" fn(AraRef, AraRef, *const AraContentTimeRange, i32),
    destroy_musical_context: unsafe extern "C" fn(AraRef, AraRef),
    create_audio_source:
        unsafe extern "C" fn(AraRef, AraRef, *const AraAudioSourceProperties) -> AraRef,
    update_audio_source_properties:
        unsafe extern "C" fn(AraRef, AraRef, *const AraAudioSourceProperties),
    update_audio_source_content:
        unsafe extern "C" fn(AraRef, AraRef, *const AraContentTimeRange, i32),
    enable_audio_source_samples_access: unsafe extern "C" fn(AraRef, AraRef, AraBool),
    deactivate_audio_source_for_undo_history: unsafe extern "C" fn(AraRef, AraRef, AraBool),
    destroy_audio_source: unsafe extern "C" fn(AraRef, AraRef),
    create_audio_modification: unsafe extern "C" fn(
        AraRef,
        AraRef,
        AraRef,
        *const AraAudioModificationProperties,
    ) -> AraRef,
    clone_audio_modification: unsafe extern "C" fn(
        AraRef,
        AraRef,
        AraRef,
        *const AraAudioModificationProperties,
    ) -> AraRef,
    update_audio_modification_properties:
        unsafe extern "C" fn(AraRef, AraRef, *const AraAudioModificationProperties),
    deactivate_audio_modification_for_undo_history: unsafe extern "C" fn(AraRef, AraRef, AraBool),
    destroy_audio_modification: unsafe extern "C" fn(AraRef, AraRef),
    create_playback_region:
        unsafe extern "C" fn(AraRef, AraRef, AraRef, *const AraPlaybackRegionProperties) -> AraRef,
    update_playback_region_properties:
        unsafe extern "C" fn(AraRef, AraRef, *const AraPlaybackRegionProperties),
    destroy_playback_region: unsafe extern "C" fn(AraRef, AraRef),
    is_audio_source_content_available: unsafe extern "C" fn(AraRef, AraRef, i32) -> AraBool,
    is_audio_source_content_analysis_incomplete:
        unsafe extern "C" fn(AraRef, AraRef, i32) -> AraBool,
    request_audio_source_content_analysis:
        unsafe extern "C" fn(AraRef, AraRef, AraSize, *const i32),
    get_audio_source_content_grade: unsafe extern "C" fn(AraRef, AraRef, i32) -> i32,
    create_audio_source_content_reader:
        unsafe extern "C" fn(AraRef, AraRef, i32, *const AraContentTimeRange) -> AraRef,
    is_audio_modification_content_available: unsafe extern "C" fn(AraRef, AraRef, i32) -> AraBool,
    get_audio_modification_content_grade: unsafe extern "C" fn(AraRef, AraRef, i32) -> i32,
    create_audio_modification_content_reader:
        unsafe extern "C" fn(AraRef, AraRef, i32, *const AraContentTimeRange) -> AraRef,
    is_playback_region_content_available: unsafe extern "C" fn(AraRef, AraRef, i32) -> AraBool,
    get_playback_region_content_grade: unsafe extern "C" fn(AraRef, AraRef, i32) -> i32,
    create_playback_region_content_reader:
        unsafe extern "C" fn(AraRef, AraRef, i32, *const AraContentTimeRange) -> AraRef,
    get_content_reader_event_count: unsafe extern "C" fn(AraRef, AraRef) -> i32,
    get_content_reader_data_for_event: unsafe extern "C" fn(AraRef, AraRef, i32) -> *const c_void,
    destroy_content_reader: unsafe extern "C" fn(AraRef, AraRef),
    create_region_sequence:
        unsafe extern "C" fn(AraRef, AraRef, *const AraRegionSequenceProperties) -> AraRef,
    update_region_sequence_properties:
        unsafe extern "C" fn(AraRef, AraRef, *const AraRegionSequenceProperties),
    destroy_region_sequence: unsafe extern "C" fn(AraRef, AraRef),
    get_playback_region_head_and_tail_time:
        unsafe extern "C" fn(AraRef, AraRef, *mut f64, *mut f64),
    restore_objects_from_archive: unsafe extern "C" fn(AraRef, AraRef, *const c_void) -> AraBool,
    store_objects_to_archive: unsafe extern "C" fn(AraRef, AraRef, *const c_void) -> AraBool,
    get_processing_algorithms_count: unsafe extern "C" fn(AraRef) -> i32,
    get_processing_algorithm_properties: unsafe extern "C" fn(AraRef, i32) -> *const c_void,
    get_processing_algorithm_for_audio_source: unsafe extern "C" fn(AraRef, AraRef) -> i32,
    request_processing_algorithm_for_audio_source: unsafe extern "C" fn(AraRef, AraRef, i32),
    is_licensed_for_capabilities:
        unsafe extern "C" fn(AraRef, AraBool, AraSize, *const i32, i32) -> AraBool,
    store_audio_source_to_audio_file_chunk:
        unsafe extern "C" fn(AraRef, AraRef, AraRef, *mut *const c_char, *mut AraBool) -> AraBool,
    is_audio_modification_preserving_audio_source_signal:
        unsafe extern "C" fn(AraRef, AraRef) -> AraBool,
}

/// `ARAAudioAccessControllerInterface`（宿主侧；夹具只调读音频三件）。
#[repr(C)]
struct AraAudioAccessInterface {
    struct_size: AraSize,
    create_audio_reader_for_source: unsafe extern "C" fn(AraRef, AraRef, AraBool) -> AraRef,
    read_audio_samples:
        unsafe extern "C" fn(AraRef, AraRef, i64, i64, *const *mut c_void) -> AraBool,
    destroy_audio_reader: unsafe extern "C" fn(AraRef, AraRef),
}

/// `ARADocumentControllerHostInstance`（5 组 host ref＋接口指针；夹具只读
/// audio_access 一组，其余字段只占布局、不解引用）。
#[repr(C)]
struct AraDocumentControllerHostInstance {
    struct_size: AraSize,
    audio_access_controller_host_ref: AraRef,
    audio_access_controller_interface: *const AraAudioAccessInterface,
    archiving_controller_host_ref: AraRef,
    archiving_controller_interface: *const c_void,
    content_access_controller_host_ref: AraRef,
    content_access_controller_interface: *const c_void,
    model_update_controller_host_ref: AraRef,
    model_update_controller_interface: *const c_void,
    playback_controller_host_ref: AraRef,
    playback_controller_interface: *const c_void,
}

/// `ARA::IMainFactory` 虚表（`FUnknown`＋`getFactory`）。
#[repr(C)]
struct AraMainFactoryVtbl {
    query_interface: QueryFn,
    add_ref: RefFn,
    release: RefFn,
    get_factory: unsafe extern "system" fn(AraRef) -> *const AraFactoryRaw,
}

#[repr(C)]
struct MainFactoryObj {
    vtbl: &'static AraMainFactoryVtbl,
}

// SAFETY: `AraFactoryRaw` 的指针字段全部指向静态字节串／静态数组／
// 函数指针，永不突变；静态共享只读访问是安全的。
unsafe impl Sync for AraFactoryRaw {}

// ---------------------------------------------------------------------------
// 夹具控制器状态与固定音符集
// ---------------------------------------------------------------------------

/// 控制器内部状态（只在宿主测试线程上访问；Mutex 防跨线程误用，不跨
/// 宿主回调持有——探针先把参数拷出锁外再调宿主接口，⛔ 不持锁进回调）。
struct ControllerInner {
    /// 宿主实例指针（会话期有效；`destroy_document_controller` 后清零）。
    host_instance: *const AraDocumentControllerHostInstance,
    /// `create_audio_source` 时宿主传来的 source host ref（音频探针的键）。
    source_host_ref: AraRef,
    sample_count: i64,
    sample_rate: f64,
    channel_count: i32,
    analysis_requested: bool,
    available: bool,
}

struct ControllerState {
    inner: Mutex<ControllerInner>,
}

impl ControllerState {
    fn new(host_instance: *const AraDocumentControllerHostInstance) -> Self {
        Self {
            inner: Mutex::new(ControllerInner {
                host_instance,
                source_host_ref: ptr::null_mut(),
                sample_count: 0,
                sample_rate: 0.0,
                channel_count: 0,
                analysis_requested: false,
                available: false,
            }),
        }
    }
}

/// 哑对象：宿主只校验创建返回值非空、之后原样传回，永不解引用。
struct DummyObj;

/// 固定音符集（⛔ 不是分析结果）：与宿主测试阶梯正弦同音高，每音 0.5 秒。
fn fixed_notes() -> Vec<AraContentNoteRaw> {
    let midi_notes = [57, 60, 62, 64, 65, 67, 69, 72];
    midi_notes
        .iter()
        .enumerate()
        .map(|(index, note)| {
            let frequency = 440.0 * 2.0_f64.powf((f64::from(*note) - 69.0) / 12.0);
            AraContentNoteRaw {
                frequency: frequency as f32,
                pitch_number: *note,
                volume: 1.0,
                start_position: index as f64 * 0.5,
                attack_duration: 0.02,
                note_duration: 0.5,
                signal_duration: 0.5,
            }
        })
        .collect()
}

/// 内容读取器状态（创建时按范围过滤，之后只读）。
struct ReaderState {
    notes: Vec<AraContentNoteRaw>,
}

fn controller_state(controller: AraRef) -> &'static ControllerState {
    // SAFETY: controller ref 由夹具创建调用返回，会话期内有效（泄漏模型）。
    unsafe { &*(controller as *const ControllerState) }
}

fn dummy_ref() -> AraRef {
    Box::into_raw(Box::new(DummyObj)) as AraRef
}

/// 音频探针：经宿主 AudioAccess 接口实读首段音频；读不到或全静默 ⇒ 假。
/// 调用方保证不持控制器锁（探针内调宿主回调，⛔ 不持锁进回调）。
unsafe fn probe_audio(
    host_instance: *const AraDocumentControllerHostInstance,
    source_host_ref: AraRef,
    sample_count: i64,
    channel_count: i32,
) -> bool {
    if host_instance.is_null() || source_host_ref.is_null() {
        return false;
    }
    if sample_count <= 0 || channel_count <= 0 {
        return false;
    }
    // SAFETY: host instance 在会话期内有效；audio_access 一组按原文布局读。
    let (host_ref, iface) = unsafe {
        (
            (*host_instance).audio_access_controller_host_ref,
            (*host_instance).audio_access_controller_interface,
        )
    };
    if iface.is_null() {
        return false;
    }
    // SAFETY: 接口指针来自宿主实例；reader 创建失败 ⇒ 空指针。
    let reader = unsafe { ((*iface).create_audio_reader_for_source)(host_ref, source_host_ref, 0) };
    if reader.is_null() {
        return false;
    }
    let frames = sample_count.min(8192) as usize;
    let channels = channel_count as usize;
    let mut bufs: Vec<Vec<f32>> = vec![vec![0.0; frames]; channels];
    let ptrs: Vec<*mut c_void> = bufs
        .iter_mut()
        .map(|buf| buf.as_mut_ptr() as *mut c_void)
        .collect();
    // SAFETY: 缓冲区按声道数配齐；f32 模式（use64bit = 0）与宿主约定一致。
    let ok =
        unsafe { ((*iface).read_audio_samples)(host_ref, reader, 0, frames as i64, ptrs.as_ptr()) };
    // SAFETY: reader 由上调用创建；销毁与创建配对。
    unsafe { ((*iface).destroy_audio_reader)(host_ref, reader) };
    if ok == ARA_FALSE {
        return false;
    }
    bufs.iter().flatten().any(|value| value.abs() > 1e-6)
}

// ---------------------------------------------------------------------------
// 控制器方法（虚表顺序与宿主 `AraDocumentControllerInterface` 严格一致）
// ---------------------------------------------------------------------------

unsafe extern "C" fn ctrl_destroy(controller: AraRef) {
    // 泄漏模型：不释放状态，只把宿主侧指针清零，此后调用一律 fail-closed。
    let state = controller_state(controller);
    if let Ok(mut inner) = state.inner.lock() {
        inner.host_instance = ptr::null();
        inner.source_host_ref = ptr::null_mut();
        inner.available = false;
    }
}

unsafe extern "C" fn ctrl_get_factory(_controller: AraRef) -> *const AraFactoryRaw {
    &ARA_FACTORY
}

unsafe extern "C" fn ctrl_begin_editing(_controller: AraRef) {}

unsafe extern "C" fn ctrl_end_editing(_controller: AraRef) {}

unsafe extern "C" fn ctrl_notify_model_updates(_controller: AraRef) {}

unsafe extern "C" fn ctrl_begin_restore(_controller: AraRef, _archive: AraRef) -> AraBool {
    ARA_FALSE
}

unsafe extern "C" fn ctrl_end_restore(_controller: AraRef, _archive: AraRef) -> AraBool {
    ARA_FALSE
}

unsafe extern "C" fn ctrl_store_to_archive(_controller: AraRef, _archive: AraRef) -> AraBool {
    ARA_FALSE
}

unsafe extern "C" fn ctrl_update_doc_props(
    _controller: AraRef,
    _props: *const AraDocumentProperties,
) {
}

unsafe extern "C" fn ctrl_create_musical_context(
    _controller: AraRef,
    _host_ref: AraRef,
    _props: *const AraMusicalContextProperties,
) -> AraRef {
    dummy_ref()
}

unsafe extern "C" fn ctrl_update_musical_context_props(
    _controller: AraRef,
    _context: AraRef,
    _props: *const AraMusicalContextProperties,
) {
}

unsafe extern "C" fn ctrl_update_musical_context_content(
    _controller: AraRef,
    _context: AraRef,
    _range: *const AraContentTimeRange,
    _type: i32,
) {
}

unsafe extern "C" fn ctrl_destroy_musical_context(_controller: AraRef, _context: AraRef) {}

unsafe extern "C" fn ctrl_create_audio_source(
    controller: AraRef,
    host_ref: AraRef,
    props: *const AraAudioSourceProperties,
) -> AraRef {
    let state = controller_state(controller);
    if let Ok(mut inner) = state.inner.lock() {
        inner.source_host_ref = host_ref;
        if !props.is_null() {
            // SAFETY: 属性指针由宿主在调用帧内提供；packed(4) 布局按字段名读。
            let props = unsafe { &*props };
            inner.sample_count = props.sample_count;
            inner.sample_rate = props.sample_rate;
            inner.channel_count = props.channel_count;
        }
    }
    dummy_ref()
}

unsafe extern "C" fn ctrl_update_audio_source_props(
    _controller: AraRef,
    _source: AraRef,
    _props: *const AraAudioSourceProperties,
) {
}

unsafe extern "C" fn ctrl_update_audio_source_content(
    _controller: AraRef,
    _source: AraRef,
    _range: *const AraContentTimeRange,
    _type: i32,
) {
}

unsafe extern "C" fn ctrl_enable_samples_access(
    _controller: AraRef,
    _source: AraRef,
    _enable: AraBool,
) {
}

unsafe extern "C" fn ctrl_deactivate_source_undo(
    _controller: AraRef,
    _source: AraRef,
    _enable: AraBool,
) {
}

unsafe extern "C" fn ctrl_destroy_audio_source(_controller: AraRef, _source: AraRef) {}

unsafe extern "C" fn ctrl_create_audio_modification(
    _controller: AraRef,
    _source: AraRef,
    _host_ref: AraRef,
    _props: *const AraAudioModificationProperties,
) -> AraRef {
    dummy_ref()
}

unsafe extern "C" fn ctrl_clone_audio_modification(
    _controller: AraRef,
    _source: AraRef,
    _host_ref: AraRef,
    _props: *const AraAudioModificationProperties,
) -> AraRef {
    dummy_ref()
}

unsafe extern "C" fn ctrl_update_audio_modification_props(
    _controller: AraRef,
    _modification: AraRef,
    _props: *const AraAudioModificationProperties,
) {
}

unsafe extern "C" fn ctrl_deactivate_modification_undo(
    _controller: AraRef,
    _modification: AraRef,
    _enable: AraBool,
) {
}

unsafe extern "C" fn ctrl_destroy_audio_modification(_controller: AraRef, _modification: AraRef) {}

unsafe extern "C" fn ctrl_create_playback_region(
    _controller: AraRef,
    _modification: AraRef,
    _host_ref: AraRef,
    _props: *const AraPlaybackRegionProperties,
) -> AraRef {
    dummy_ref()
}

unsafe extern "C" fn ctrl_update_playback_region_props(
    _controller: AraRef,
    _region: AraRef,
    _props: *const AraPlaybackRegionProperties,
) {
}

unsafe extern "C" fn ctrl_destroy_playback_region(_controller: AraRef, _region: AraRef) {}

unsafe extern "C" fn ctrl_source_content_available(
    controller: AraRef,
    _source: AraRef,
    content_type: i32,
) -> AraBool {
    let state = controller_state(controller);
    let available = state
        .inner
        .lock()
        .map(|inner| inner.available)
        .unwrap_or(false);
    if content_type == CONTENT_TYPE_NOTES && available {
        ARA_TRUE
    } else {
        ARA_FALSE
    }
}

unsafe extern "C" fn ctrl_source_content_incomplete(
    controller: AraRef,
    _source: AraRef,
    content_type: i32,
) -> AraBool {
    let state = controller_state(controller);
    let (requested, available) = state
        .inner
        .lock()
        .map(|inner| (inner.analysis_requested, inner.available))
        .unwrap_or((false, false));
    if content_type == CONTENT_TYPE_NOTES && requested && !available {
        ARA_TRUE
    } else {
        ARA_FALSE
    }
}

unsafe extern "C" fn ctrl_request_source_analysis(
    controller: AraRef,
    _source: AraRef,
    count: AraSize,
    types: *const i32,
) {
    // 先把探针参数拷出锁外，再调宿主回调（⛔ 不持锁进回调）。
    let (host_instance, source_host_ref, sample_count, channel_count) = {
        let state = controller_state(controller);
        match state.inner.lock() {
            Ok(mut inner) => {
                inner.analysis_requested = true;
                if inner.available {
                    return;
                }
                (
                    inner.host_instance,
                    inner.source_host_ref,
                    inner.sample_count,
                    inner.channel_count,
                )
            }
            Err(_) => return,
        }
    };
    if types.is_null() {
        return;
    }
    let mut wants_notes = false;
    for index in 0..count {
        // SAFETY: 类型数组由宿主在调用帧内提供；下标在计数内。
        let entry = unsafe { *types.add(index) };
        if entry == CONTENT_TYPE_NOTES {
            wants_notes = true;
        }
    }
    if !wants_notes {
        return;
    }
    // SAFETY: 参数已拷出锁；探针内调宿主 AudioAccess 回调。
    let ok = unsafe { probe_audio(host_instance, source_host_ref, sample_count, channel_count) };
    if ok {
        let state = controller_state(controller);
        if let Ok(mut inner) = state.inner.lock() {
            inner.available = true;
        }
    }
}

unsafe extern "C" fn ctrl_source_content_grade(
    controller: AraRef,
    _source: AraRef,
    content_type: i32,
) -> i32 {
    let state = controller_state(controller);
    let available = state
        .inner
        .lock()
        .map(|inner| inner.available)
        .unwrap_or(false);
    if content_type == CONTENT_TYPE_NOTES && available {
        CONTENT_GRADE_DETECTED
    } else {
        0
    }
}

unsafe extern "C" fn ctrl_create_source_content_reader(
    controller: AraRef,
    _source: AraRef,
    content_type: i32,
    range: *const AraContentTimeRange,
) -> AraRef {
    let state = controller_state(controller);
    let available = state
        .inner
        .lock()
        .map(|inner| inner.available)
        .unwrap_or(false);
    if content_type != CONTENT_TYPE_NOTES || !available {
        return ptr::null_mut();
    }
    let mut notes = fixed_notes();
    if !range.is_null() {
        // SAFETY: 范围指针由宿主在调用帧内提供。
        let range = unsafe { &*range };
        let end = range.start + range.duration;
        notes.retain(|note| note.start_position >= range.start && note.start_position < end);
    }
    Box::into_raw(Box::new(ReaderState { notes })) as AraRef
}

unsafe extern "C" fn ctrl_modification_content_available(
    _controller: AraRef,
    _modification: AraRef,
    _content_type: i32,
) -> AraBool {
    // 夹具只在 audio source 级发布音符；modification 级内容不在 ② 范围。
    ARA_FALSE
}

unsafe extern "C" fn ctrl_modification_content_grade(
    _controller: AraRef,
    _modification: AraRef,
    _content_type: i32,
) -> i32 {
    0
}

unsafe extern "C" fn ctrl_create_modification_content_reader(
    _controller: AraRef,
    _modification: AraRef,
    _content_type: i32,
    _range: *const AraContentTimeRange,
) -> AraRef {
    ptr::null_mut()
}

unsafe extern "C" fn ctrl_region_content_available(
    _controller: AraRef,
    _region: AraRef,
    _content_type: i32,
) -> AraBool {
    ARA_FALSE
}

unsafe extern "C" fn ctrl_region_content_grade(
    _controller: AraRef,
    _region: AraRef,
    _content_type: i32,
) -> i32 {
    0
}

unsafe extern "C" fn ctrl_create_region_content_reader(
    _controller: AraRef,
    _region: AraRef,
    _content_type: i32,
    _range: *const AraContentTimeRange,
) -> AraRef {
    ptr::null_mut()
}

unsafe extern "C" fn ctrl_reader_event_count(_controller: AraRef, reader: AraRef) -> i32 {
    if reader.is_null() {
        return 0;
    }
    // SAFETY: reader 由夹具创建调用返回；调用期内有效（泄漏模型）。
    let state = unsafe { &*(reader as *const ReaderState) };
    state.notes.len() as i32
}

unsafe extern "C" fn ctrl_reader_data_for_event(
    _controller: AraRef,
    reader: AraRef,
    index: i32,
) -> *const c_void {
    if reader.is_null() || index < 0 {
        return ptr::null();
    }
    // SAFETY: reader 由夹具创建调用返回；下标已校验在计数内（越界 ⇒ 空指针，
    // 宿主侧响亮失败，不静默）。
    let state = unsafe { &*(reader as *const ReaderState) };
    match state.notes.get(index as usize) {
        Some(note) => note as *const AraContentNoteRaw as *const c_void,
        None => ptr::null(),
    }
}

unsafe extern "C" fn ctrl_destroy_reader(_controller: AraRef, _reader: AraRef) {
    // 泄漏模型：不释放，只丢弃句柄。
}

unsafe extern "C" fn ctrl_create_region_sequence(
    _controller: AraRef,
    _host_ref: AraRef,
    _props: *const AraRegionSequenceProperties,
) -> AraRef {
    dummy_ref()
}

unsafe extern "C" fn ctrl_update_region_sequence_props(
    _controller: AraRef,
    _sequence: AraRef,
    _props: *const AraRegionSequenceProperties,
) {
}

unsafe extern "C" fn ctrl_destroy_region_sequence(_controller: AraRef, _sequence: AraRef) {}

unsafe extern "C" fn ctrl_region_head_and_tail(
    controller: AraRef,
    _region: AraRef,
    head: *mut f64,
    tail: *mut f64,
) {
    let state = controller_state(controller);
    let (sample_count, sample_rate) = state
        .inner
        .lock()
        .map(|inner| (inner.sample_count, inner.sample_rate))
        .unwrap_or((0, 0.0));
    let duration = if sample_rate > 0.0 {
        sample_count as f64 / sample_rate
    } else {
        0.0
    };
    if !head.is_null() {
        // SAFETY: 指针由宿主提供；调用帧内有效。
        unsafe { *head = 0.0 };
    }
    if !tail.is_null() {
        // SAFETY: 同上。
        unsafe { *tail = duration };
    }
}

unsafe extern "C" fn ctrl_restore_objects(
    _controller: AraRef,
    _archive: AraRef,
    _filter: *const c_void,
) -> AraBool {
    // 夹具不支持归档（④ 已由 PitchNet 真插件验证，不在本夹具范围）。
    ARA_FALSE
}

unsafe extern "C" fn ctrl_store_objects(
    _controller: AraRef,
    _archive: AraRef,
    _filter: *const c_void,
) -> AraBool {
    ARA_FALSE
}

unsafe extern "C" fn ctrl_processing_algorithms_count(_controller: AraRef) -> i32 {
    0
}

unsafe extern "C" fn ctrl_processing_algorithm_props(
    _controller: AraRef,
    _index: i32,
) -> *const c_void {
    ptr::null()
}

unsafe extern "C" fn ctrl_processing_algorithm_for_source(
    _controller: AraRef,
    _source: AraRef,
) -> i32 {
    0
}

unsafe extern "C" fn ctrl_request_processing_algorithm(
    _controller: AraRef,
    _source: AraRef,
    _index: i32,
) {
}

unsafe extern "C" fn ctrl_is_licensed(
    _controller: AraRef,
    _capability: AraBool,
    _count: AraSize,
    _types: *const i32,
    _flags: i32,
) -> AraBool {
    ARA_TRUE
}

unsafe extern "C" fn ctrl_store_source_to_chunk(
    _controller: AraRef,
    _source: AraRef,
    _modification: AraRef,
    chunk: *mut *const c_char,
    _is_chunk_modified: *mut AraBool,
) -> AraBool {
    if !chunk.is_null() {
        // SAFETY: 指针由宿主提供；调用帧内有效。
        unsafe { *chunk = ptr::null() };
    }
    ARA_FALSE
}

unsafe extern "C" fn ctrl_modification_preserves_signal(
    _controller: AraRef,
    _modification: AraRef,
) -> AraBool {
    ARA_TRUE
}

static CONTROLLER_VTBL: AraDocumentControllerInterface = AraDocumentControllerInterface {
    struct_size: size_of::<AraDocumentControllerInterface>(),
    destroy_document_controller: ctrl_destroy,
    get_factory: ctrl_get_factory,
    begin_editing: ctrl_begin_editing,
    end_editing: ctrl_end_editing,
    notify_model_updates: ctrl_notify_model_updates,
    begin_restoring_document_from_archive: ctrl_begin_restore,
    end_restoring_document_from_archive: ctrl_end_restore,
    store_document_to_archive: ctrl_store_to_archive,
    update_document_properties: ctrl_update_doc_props,
    create_musical_context: ctrl_create_musical_context,
    update_musical_context_properties: ctrl_update_musical_context_props,
    update_musical_context_content: ctrl_update_musical_context_content,
    destroy_musical_context: ctrl_destroy_musical_context,
    create_audio_source: ctrl_create_audio_source,
    update_audio_source_properties: ctrl_update_audio_source_props,
    update_audio_source_content: ctrl_update_audio_source_content,
    enable_audio_source_samples_access: ctrl_enable_samples_access,
    deactivate_audio_source_for_undo_history: ctrl_deactivate_source_undo,
    destroy_audio_source: ctrl_destroy_audio_source,
    create_audio_modification: ctrl_create_audio_modification,
    clone_audio_modification: ctrl_clone_audio_modification,
    update_audio_modification_properties: ctrl_update_audio_modification_props,
    deactivate_audio_modification_for_undo_history: ctrl_deactivate_modification_undo,
    destroy_audio_modification: ctrl_destroy_audio_modification,
    create_playback_region: ctrl_create_playback_region,
    update_playback_region_properties: ctrl_update_playback_region_props,
    destroy_playback_region: ctrl_destroy_playback_region,
    is_audio_source_content_available: ctrl_source_content_available,
    is_audio_source_content_analysis_incomplete: ctrl_source_content_incomplete,
    request_audio_source_content_analysis: ctrl_request_source_analysis,
    get_audio_source_content_grade: ctrl_source_content_grade,
    create_audio_source_content_reader: ctrl_create_source_content_reader,
    is_audio_modification_content_available: ctrl_modification_content_available,
    get_audio_modification_content_grade: ctrl_modification_content_grade,
    create_audio_modification_content_reader: ctrl_create_modification_content_reader,
    is_playback_region_content_available: ctrl_region_content_available,
    get_playback_region_content_grade: ctrl_region_content_grade,
    create_playback_region_content_reader: ctrl_create_region_content_reader,
    get_content_reader_event_count: ctrl_reader_event_count,
    get_content_reader_data_for_event: ctrl_reader_data_for_event,
    destroy_content_reader: ctrl_destroy_reader,
    create_region_sequence: ctrl_create_region_sequence,
    update_region_sequence_properties: ctrl_update_region_sequence_props,
    destroy_region_sequence: ctrl_destroy_region_sequence,
    get_playback_region_head_and_tail_time: ctrl_region_head_and_tail,
    restore_objects_from_archive: ctrl_restore_objects,
    store_objects_to_archive: ctrl_store_objects,
    get_processing_algorithms_count: ctrl_processing_algorithms_count,
    get_processing_algorithm_properties: ctrl_processing_algorithm_props,
    get_processing_algorithm_for_audio_source: ctrl_processing_algorithm_for_source,
    request_processing_algorithm_for_audio_source: ctrl_request_processing_algorithm,
    is_licensed_for_capabilities: ctrl_is_licensed,
    store_audio_source_to_audio_file_chunk: ctrl_store_source_to_chunk,
    is_audio_modification_preserving_audio_source_signal: ctrl_modification_preserves_signal,
};

// ---------------------------------------------------------------------------
// ARA 工厂与主工厂对象
// ---------------------------------------------------------------------------

static FACTORY_ID: &[u8] = b"com.yeban.testara.arafactory.0.1.0\0";
static PLUG_IN_NAME: &[u8] = b"TestAra\0";
static MANUFACTURER_NAME: &[u8] = b"Yeban Test\0";
static INFORMATION_URL: &[u8] = b"\0";
static VERSION: &[u8] = b"0.1.0\0";
static DOCUMENT_ARCHIVE_ID: &[u8] = b"com.yeban.testara.archive.0.1.0\0";
static ANALYZEABLE_TYPES: [i32; 1] = [CONTENT_TYPE_NOTES];

unsafe extern "C" fn fixture_initialize_ara(_config: *const AraInterfaceConfiguration) {}

unsafe extern "C" fn fixture_uninitialize_ara() {}

unsafe extern "C" fn fixture_create_controller(
    host_instance: *const AraDocumentControllerHostInstance,
    _props: *const AraDocumentProperties,
) -> *const AraDocumentControllerInstance {
    let state = Box::into_raw(Box::new(ControllerState::new(host_instance)));
    let instance = Box::into_raw(Box::new(AraDocumentControllerInstance {
        struct_size: size_of::<AraDocumentControllerInstance>(),
        document_controller_ref: state as AraRef,
        document_controller_interface: &CONTROLLER_VTBL,
    }));
    instance
}

static ARA_FACTORY: AraFactoryRaw = AraFactoryRaw {
    struct_size: size_of::<AraFactoryRaw>(),
    lowest_supported_api_generation: API_GENERATION_2_0_DRAFT,
    highest_supported_api_generation: API_GENERATION_2_0_FINAL,
    factory_id: FACTORY_ID.as_ptr() as *const c_char,
    initialize_ara_with_configuration: Some(fixture_initialize_ara),
    uninitialize_ara: Some(fixture_uninitialize_ara),
    plug_in_name: PLUG_IN_NAME.as_ptr() as *const c_char,
    manufacturer_name: MANUFACTURER_NAME.as_ptr() as *const c_char,
    information_url: INFORMATION_URL.as_ptr() as *const c_char,
    version: VERSION.as_ptr() as *const c_char,
    create_document_controller_with_document: Some(fixture_create_controller),
    document_archive_id: DOCUMENT_ARCHIVE_ID.as_ptr() as *const c_char,
    compatible_document_archive_ids_count: 0,
    compatible_document_archive_ids: ptr::null(),
    analyzeable_content_types_count: ANALYZEABLE_TYPES.len(),
    analyzeable_content_types: ANALYZEABLE_TYPES.as_ptr(),
    supported_playback_transformation_flags: 0,
    supports_storing_audio_file_chunks: ARA_FALSE,
};

unsafe extern "system" fn main_factory_query(
    this: *mut c_void,
    iid: *const Tuid,
    out: *mut *mut c_void,
) -> i32 {
    let want = unsafe { *iid };
    if want == FUNKNOWN_IID || want == IMAIN_FACTORY_IID {
        unsafe { *out = this };
        RESULT_OK
    } else {
        RESULT_NO_INTERFACE
    }
}

unsafe extern "system" fn main_factory_ref(_this: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn main_factory_get_factory(_this: AraRef) -> *const AraFactoryRaw {
    &ARA_FACTORY
}

static MAIN_FACTORY_VTBL: AraMainFactoryVtbl = AraMainFactoryVtbl {
    query_interface: main_factory_query,
    add_ref: main_factory_ref,
    release: main_factory_ref,
    get_factory: main_factory_get_factory,
};
