//! ARA2 宿主（Phase B 原型）：手写 Rust FFI，进程内 VST3 companion。
//!
//! 来源：Celemony ARA API（Apache 2.0，`ARA_API/ARAInterface.h` 与
//! `ARA_API/ARAVST3.h` 原文翻译；⛔ 不可凭记忆改虚表顺序与结构体布局）。
//! VST3 侧复用一期 [`crate::vst3`]（COM 虚表、模块加载、实例化）；
//! ARA 层只新增：factory 发现（`ARA Main Factory Class` 类名配对）、
//! 5 个 controller 接口、文档绑定、分析完成门、companion 离线渲染、
//! 文档 blob 存取（CAS 资产形态：调用方以 `AssetHash` 为键存 blob 字节，
//! 本模块只产 blob、不碰工程 JSON）。
//!
//! 原型边界（与设计 §3 一致）：
//! - 一会话一音频片段、一文档、一 musical context、一 region sequence；
//!   循环在文档构建时展开成多次摆放，muted 摆放不建 region。
//! - 无编辑器（无头盒上绝不打开插件编辑器）；note 级编辑不在 v1。
//! - RT 回调不碰 ARA 对象：渲染只走离线 `process_ara` 路径。
//! - ARA 全局初始化按 factory 指针只做一次、不反初始化（原型简化：
//!   插件模块由会话持有、进程退出时随模块卸载而结束；多会话共用同一
//!   factory 时重复 `initializeARA` 会破坏 JUCE 侧的全局状态，故只初始化一次）。
//! - 会话是单线程使用的（`Send`、非 `Sync`，与 `Vst3Instance` 同款契约）：
//!   模型调用只发生在持有 `&mut self` 的调用方线程；插件分析线程只调
//!   宿主回调（audio reader／通知），回调内只碰 `AraShared` 的锁与原子量。

use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::error::HostError;
use crate::types::{Tuid, tuid};
use crate::vst3::{ProcessContext, Vst3Instance, Vst3Module};

// ---------------------------------------------------------------------------
// 常量（ARAInterface.h／ARAVST3.h 原文）
// ---------------------------------------------------------------------------

/// `ARA::IMainFactory` 的 IID（ARAVST3.h `DECLARE_CLASS_IID` 原文）。
pub const IMAIN_FACTORY_IID: Tuid = tuid(0xDB2A_1669, 0xFAFD_42A5, 0xA82F_864F, 0x7B68_72EA);
/// `ARA::IPlugInEntryPoint2` 的 IID（ARAVST3.h 原文）。
pub const IPLUGIN_ENTRY_POINT2_IID: Tuid = tuid(0xCD9A_5913, 0xC9EB_46D7, 0x96CA_53AD, 0xD1DB_89F5);

/// ARA API 世代：2.0 草案（下限，PitchNet 报告的下限即此值）。
pub const API_GENERATION_2_0_DRAFT: i32 = 3;
/// ARA API 世代：2.0 正式版（原型请求的世代；须落在插件报告的区间内）。
pub const API_GENERATION_2_0_FINAL: i32 = 4;

/// 内容类型：音符（`kARAContentTypeNotes`）。
pub const CONTENT_TYPE_NOTES: i32 = 10;
/// 内容类型：速度表（`kARAContentTypeTempoEntries`）。
pub const CONTENT_TYPE_TEMPO_ENTRIES: i32 = 20;

/// 内容等级：已检测（自动分析，未经人工复核）。
pub const CONTENT_GRADE_DETECTED: i32 = 1;
/// 内容等级：已调整（宿主提供的工程数据，如 tempo map）。
pub const CONTENT_GRADE_ADJUSTED: i32 = 2;

/// 插件实例角色：播放渲染（`kARAPlaybackRendererRole`）。
pub const ROLE_PLAYBACK_RENDERER: i32 = 1;
/// 插件实例角色：编辑渲染（`kARAEditorRendererRole`）。
pub const ROLE_EDITOR_RENDERER: i32 = 2;
/// 插件实例角色：编辑视图（`kARAEditorViewRole`）。
pub const ROLE_EDITOR_VIEW: i32 = 4;

/// ARA 布尔真（`ARABool` 是 i32，⛔ 不是 Rust `bool`）。
pub const ARA_TRUE: i32 = 1;
/// ARA 布尔假（同上）。
pub const ARA_FALSE: i32 = 0;

/// 无效音高号（纯打击乐音符等无确定音高时插件返回此值）。
pub const INVALID_PITCH_NUMBER: i32 = i32::MIN;

/// 播放变换标志：无变换（PitchNet 报告 `supportedPlaybackTransformationFlags = 0`，
/// 故原型只用 0；时间拉伸不在原型能力内，⛔ 不得凭空置位）。
pub const TRANSFORMATION_NONE: i32 = 0;

// ---------------------------------------------------------------------------
// ARA C 类型（`ARAInterface.h` 原文翻译；虚表／接口／factory 全为指针字段，
// 布局与标准 C 一致；唯属性／过滤／配置结构用 `packed(4)`，见下）。
//
// ⛔ 属性类结构一律 `packed(4)`，不是标准 C 布局。实测证据（2026-11-03，
// PitchNet 0.7.4 二进制反汇编）：MusicalContext 的 color 在偏移 20（标准布局
// 为 24）、PlaybackRegion 的 f64 组在偏移 12／28（标准布局为 16／32）。按标准
// 布局传，插件读到的 color 是填充垃圾指针，在 createMusicalContext 内段错误
// （gdb 实测）。VST3 侧结构两种布局偏移一致，不受影响。
// ---------------------------------------------------------------------------

type AraSize = usize;
type AraBool = i32;
type AraRef = *mut c_void;

/// `ARAColor`：r／g／b 三个 f32（原型只传空指针，布局备用）。
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

/// `ARAAudioSourceProperties`（2.0 Final 全字段；单声道／立体声时
/// `channel_arrangement` 可空，`channel_arrangement_data_type` 置 0）。
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

/// `ARAPlaybackRegionProperties`（2.0 全字段；`musical_context_ref` 是
/// 已废弃字段，2.0 起以 region sequence 为准，此处置空）。
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

/// `ARAContentTempoEntry`（宿主向插件提供的 tempo 表条目）。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct AraContentTempoEntry {
    time_position: f64,
    quarter_position: f64,
}

/// `ARAContentNote`（插件内容读取器返回的音符；字段顺序见头文件原文）。
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

/// `ARAStoreObjectsFilter`（存档过滤：原型存文档数据＋本会话的源与修改）。
#[repr(C, packed(4))]
struct AraStoreObjectsFilter {
    struct_size: AraSize,
    document_data: AraBool,
    audio_source_refs_count: AraSize,
    audio_source_refs: *const AraRef,
    audio_modification_refs_count: AraSize,
    audio_modification_refs: *const AraRef,
}

/// `ARARestoreObjectsFilter`（读档过滤：按 persistent ID 映射回当前图）。
#[repr(C, packed(4))]
struct AraRestoreObjectsFilter {
    struct_size: AraSize,
    document_data: AraBool,
    audio_source_ids_count: AraSize,
    audio_source_archive_ids: *const *const c_char,
    audio_source_current_ids: *const *const c_char,
    audio_modification_ids_count: AraSize,
    audio_modification_archive_ids: *const *const c_char,
    audio_modification_current_ids: *const *const c_char,
}

// ---------------------------------------------------------------------------
// 插件侧虚表（顺序见 ARAInterface.h 原文；⛔ 不可凭记忆改）
// ---------------------------------------------------------------------------

/// `ARAFactory`（64 位布局 128 字节；实测 PitchNet `structSize = 128` 一致）。
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
///
/// 全字段为函数指针＋尺寸，故 `Copy`：会话每次调用前复制一份虚表，
/// 不持有对自身的借用（避免 `&mut self` 操作与接口借用冲突）。
#[repr(C)]
#[derive(Clone, Copy)]
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
    restore_objects_from_archive:
        unsafe extern "C" fn(AraRef, AraRef, *const AraRestoreObjectsFilter) -> AraBool,
    store_objects_to_archive:
        unsafe extern "C" fn(AraRef, AraRef, *const AraStoreObjectsFilter) -> AraBool,
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

/// `ARAPlaybackRendererInterface`（仅 `add／removePlaybackRegion` 两个方法）。
#[repr(C)]
struct AraPlaybackRendererInterface {
    struct_size: AraSize,
    add_playback_region: unsafe extern "C" fn(AraRef, AraRef),
    remove_playback_region: unsafe extern "C" fn(AraRef, AraRef),
}

/// `ARAPlugInExtensionInstance`（未用的 editor 接口指针以 `*const c_void`
/// 占位，布局与原文一致；原型只调 playback renderer）。
#[repr(C)]
struct AraPlugInExtensionInstance {
    struct_size: AraSize,
    plug_in_extension_ref: AraRef,
    plug_in_extension_interface: *const c_void,
    playback_renderer_ref: AraRef,
    playback_renderer_interface: *const AraPlaybackRendererInterface,
    editor_renderer_ref: AraRef,
    editor_renderer_interface: *const c_void,
    editor_view_ref: AraRef,
    editor_view_interface: *const c_void,
}

/// `ARA::IMainFactory` 虚表（`FUnknown`＋`getFactory`）。
#[repr(C)]
struct AraMainFactoryVtbl {
    query_interface: unsafe extern "system" fn(AraRef, *const Tuid, *mut AraRef) -> i32,
    add_ref: unsafe extern "system" fn(AraRef) -> u32,
    release: unsafe extern "system" fn(AraRef) -> u32,
    get_factory: unsafe extern "system" fn(AraRef) -> *const AraFactoryRaw,
}

/// `ARA::IPlugInEntryPoint2` 虚表（`FUnknown`＋`bindToDocumentControllerWithRoles`）。
#[repr(C)]
struct AraEntryPoint2Vtbl {
    query_interface: unsafe extern "system" fn(AraRef, *const Tuid, *mut AraRef) -> i32,
    add_ref: unsafe extern "system" fn(AraRef) -> u32,
    release: unsafe extern "system" fn(AraRef) -> u32,
    bind_to_document_controller_with_roles:
        unsafe extern "system" fn(AraRef, AraRef, i32, i32) -> *const AraPlugInExtensionInstance,
}

/// 从对象指针读虚表（与 `vst3.rs` 的 `vtbl` 同款约定；ARA 侧本地复写，
/// 避免跨模块共享私有辅助）。
///
/// # Safety
///
/// 调用方保证 `obj` 是指向 COM 对象的有效指针，且首字段为对应虚表指针。
unsafe fn ara_vtbl<T>(obj: AraRef) -> &'static T {
    // SAFETY: 调用方保证对象有效；虚表指针在对象生命周期内恒有效。
    unsafe { &*(*obj.cast::<*const T>()) }
}

/// 释放一个 COM 引用（`release` 一次；返回值忽略）。
///
/// # Safety
///
/// 调用方保证 `obj` 是插件对象指针，且本次 `release` 与一次先前的
/// `createInstance`／`queryInterface` 成功配对。
unsafe fn ara_release(obj: AraRef) {
    // SAFETY: 调用方保证配对；虚表首 3 项为 `FUnknown`。
    let vtbl = unsafe { ara_vtbl::<AraMainFactoryVtbl>(obj) };
    // SAFETY: 对象指针由调用方保证有效；虚表首 3 项为 FUnknown。
    unsafe { (vtbl.release)(obj) };
}

/// 解码插件侧 UTF-8 字符串（空指针 ⇒ 空串；非法序列按丢失替换）。
fn decode_plugin_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // SAFETY: 插件保证 factory 字段字符串在模块生命周期内有效且 NUL 结尾。
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

// ---------------------------------------------------------------------------
// 宿主侧状态与 5 个 controller 接口
// ---------------------------------------------------------------------------

/// 绑定到会话的音频片段（非交错声道 PCM；reader 按声道随机读）。
#[derive(Clone, Debug)]
pub struct PcmData {
    /// 每声道一列 f32 样本（非交错；单声道人声片段为 1 列）。
    pub channels: Vec<Vec<f32>>,
    /// 采样率（Hz；audio source 属性与处理器激活共用此值，避免重采样问题）。
    pub sample_rate: f64,
}

impl PcmData {
    /// 总帧数（各声道等长；空 ⇒ 0）。
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.channels
            .first()
            .map_or(0, |channel| channel.len() as u64)
    }

    /// 声道数。
    #[must_use]
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// 片段时长（秒；帧数／采样率）。
    #[must_use]
    pub fn duration_seconds(&self) -> f64 {
        if self.sample_rate > 0.0 {
            self.frames() as f64 / self.sample_rate
        } else {
            0.0
        }
    }
}

/// 音频读取器条目（插件分析线程持有；单 reader 单线程使用，另加锁防卫）。
struct AudioReaderEntry {
    pcm: Arc<PcmData>,
    use_64_bit: bool,
    /// 读互斥（非 RT 线程专用；reader 单线程保证＋此锁＝双保险）。
    lock: Mutex<()>,
}

/// Tempo 读取器条目（宿主 tempo 表的快照；创建后不再改动，指针稳定）。
struct TempoReaderEntry {
    entries: Vec<AraContentTempoEntry>,
}

/// 归档读取器槽（store 前由会话建槽、restore 时由会话建槽；数据不可变）。
struct ArchiveReaderSlot {
    data: Vec<u8>,
    archive_id: CString,
}

/// 归档写入器槽（插件在 `storeObjectsToArchive` 调用内按位置写）。
struct ArchiveWriterSlot {
    data: Mutex<Vec<u8>>,
}

/// 宿主侧插槽标签（host ref 的不透明身份；⛔ 不用 ZST：零尺寸 Box 的
/// 地址可能重合，指针身份比较会误判，故每个槽带唯一 id、保证真实分配）。
/// 图对象 host ref 槽：64 字节堆分配，地址即 host ref 身份。
///
/// ⛔ 不可用 ZST：`Box<ZST>` 的地址可能重合，host ref 身份会撞；
/// 也不用 `id` 字段——地址本身就是身份，字段永不被读。
type Slot = [u8; 64];

/// 插件线程可触碰的共享态（全部字段 `Send + Sync`，除两个裸指针身份字段
/// 外；那两个指针只做相等比较、⛔ 永不解引用，见 `AraShared` 的 impl 注释）。
pub struct AraShared {
    /// 当前 audio source 的 PCM（`create_audio_source` 时由会话填入）。
    source_pcm: Mutex<Option<Arc<PcmData>>>,
    /// 当前 audio source 的 host ref（指针身份，供 reader 创建时校验）。
    source_host_ref: Mutex<AraRef>,
    /// 当前 musical context 的 host ref（指针身份，供内容查询时校验）。
    context_host_ref: Mutex<AraRef>,
    /// 宿主 tempo 表（恒定 BPM 的两点同步表；见 `AraSession::tempo_entries`）。
    tempo_entries: Mutex<Vec<AraContentTempoEntry>>,
    /// 存活的音频读取器（`Box` 堆地址稳定，Vec 重分配不移动条目本身；
    /// 地址已交插件作 host ref，故保留 Box，见 vec_box 豁免）。
    #[allow(clippy::vec_box)]
    audio_readers: Mutex<Vec<Box<AudioReaderEntry>>>,
    /// 存活的 tempo 读取器（同款地址稳定性）。
    #[allow(clippy::vec_box)]
    tempo_readers: Mutex<Vec<Box<TempoReaderEntry>>>,
    /// 最近一次分析进度通知（状态，值；`Completed` 由 poll 循环的
    /// `isAudioSourceContentAnalysisIncomplete` 判定，不以通知为准）。
    analysis_progress: Mutex<(i32, f32)>,
    /// 插件侧内容变化通知是否到过（诊断用；完成判定仍以轮询为准）。
    content_changed: AtomicBool,
    /// 最近一次归档进度通知（诊断用）。
    archive_progress: Mutex<f32>,
    /// 插件走带请求记录（v1 只记不执行；走带始终宿主驱动）。
    playback_requests: Mutex<Vec<(u8, f64, f64)>>,
}

// SAFETY: 两个裸指针字段只做相等比较，永不解引用；其余字段全部
// `Send + Sync`（`Mutex`／`Arc`／`AtomicBool`），故跨线程共享安全。
unsafe impl Send for AraShared {}
// SAFETY: 同上；回调只在插件的非 RT 线程上发生，锁内无 RT 调用。
unsafe impl Sync for AraShared {}

impl AraShared {
    fn new() -> Self {
        Self {
            source_pcm: Mutex::new(None),
            source_host_ref: Mutex::new(ptr::null_mut()),
            context_host_ref: Mutex::new(ptr::null_mut()),
            tempo_entries: Mutex::new(Vec::new()),
            audio_readers: Mutex::new(Vec::new()),
            tempo_readers: Mutex::new(Vec::new()),
            analysis_progress: Mutex::new((0, 0.0)),
            content_changed: AtomicBool::new(false),
            archive_progress: Mutex::new(0.0),
            playback_requests: Mutex::new(Vec::new()),
        }
    }
}

/// 从 controller host ref 取共享态（回调入口的唯一解引用点；空指针 ⇒ `None`）。
fn shared_from(controller_host_ref: AraRef) -> Option<&'static AraShared> {
    if controller_host_ref.is_null() {
        return None;
    }
    // SAFETY: controller host ref 是会话建的 `Arc<AraShared>` 指针，
    // 在全部 document controller 销毁前保持有效（会话 Drop 先销毁 controller）。
    Some(unsafe { &*controller_host_ref.cast::<AraShared>() })
}

/// `ARAAudioAccessControllerInterface`。
#[repr(C)]
struct AraAudioAccessInterface {
    struct_size: AraSize,
    create_audio_reader_for_source: unsafe extern "C" fn(AraRef, AraRef, AraBool) -> AraRef,
    read_audio_samples:
        unsafe extern "C" fn(AraRef, AraRef, i64, i64, *const *mut c_void) -> AraBool,
    destroy_audio_reader: unsafe extern "C" fn(AraRef, AraRef),
}

/// `ARAArchivingControllerInterface`。
#[repr(C)]
struct AraArchivingInterface {
    struct_size: AraSize,
    get_archive_size: unsafe extern "C" fn(AraRef, AraRef) -> AraSize,
    read_bytes_from_archive:
        unsafe extern "C" fn(AraRef, AraRef, AraSize, AraSize, *mut u8) -> AraBool,
    write_bytes_to_archive:
        unsafe extern "C" fn(AraRef, AraRef, AraSize, AraSize, *const u8) -> AraBool,
    notify_document_archiving_progress: unsafe extern "C" fn(AraRef, f32),
    notify_document_unarchiving_progress: unsafe extern "C" fn(AraRef, f32),
    get_document_archive_id: unsafe extern "C" fn(AraRef, AraRef) -> *const c_char,
}

/// `ARAContentAccessControllerInterface`。
#[repr(C)]
struct AraContentAccessInterface {
    struct_size: AraSize,
    is_musical_context_content_available: unsafe extern "C" fn(AraRef, AraRef, i32) -> AraBool,
    get_musical_context_content_grade: unsafe extern "C" fn(AraRef, AraRef, i32) -> i32,
    create_musical_context_content_reader:
        unsafe extern "C" fn(AraRef, AraRef, i32, *const AraContentTimeRange) -> AraRef,
    is_audio_source_content_available: unsafe extern "C" fn(AraRef, AraRef, i32) -> AraBool,
    get_audio_source_content_grade: unsafe extern "C" fn(AraRef, AraRef, i32) -> i32,
    create_audio_source_content_reader:
        unsafe extern "C" fn(AraRef, AraRef, i32, *const AraContentTimeRange) -> AraRef,
    get_content_reader_event_count: unsafe extern "C" fn(AraRef, AraRef) -> i32,
    get_content_reader_data_for_event: unsafe extern "C" fn(AraRef, AraRef, i32) -> *const c_void,
    destroy_content_reader: unsafe extern "C" fn(AraRef, AraRef),
}

/// `ARAModelUpdateControllerInterface`。
#[repr(C)]
struct AraModelUpdateInterface {
    struct_size: AraSize,
    notify_audio_source_analysis_progress: unsafe extern "C" fn(AraRef, AraRef, i32, f32),
    notify_audio_source_content_changed:
        unsafe extern "C" fn(AraRef, AraRef, *const AraContentTimeRange, i32),
    notify_audio_modification_content_changed:
        unsafe extern "C" fn(AraRef, AraRef, *const AraContentTimeRange, i32),
    notify_playback_region_content_changed:
        unsafe extern "C" fn(AraRef, AraRef, *const AraContentTimeRange, i32),
    notify_document_data_changed: unsafe extern "C" fn(AraRef),
}

/// `ARAPlaybackControllerInterface`（v1 只记请求、不执行走带）。
#[repr(C)]
struct AraPlaybackInterface {
    struct_size: AraSize,
    request_start_playback: unsafe extern "C" fn(AraRef),
    request_stop_playback: unsafe extern "C" fn(AraRef),
    request_set_playback_position: unsafe extern "C" fn(AraRef, f64),
    request_set_cycle_range: unsafe extern "C" fn(AraRef, f64, f64),
    request_enable_cycle: unsafe extern "C" fn(AraRef, AraBool),
}

/// `ARADocumentControllerHostInstance`（5 组 host ref＋接口指针）。
#[repr(C)]
struct AraDocumentControllerHostInstance {
    struct_size: AraSize,
    audio_access_controller_host_ref: AraRef,
    audio_access_controller_interface: *const AraAudioAccessInterface,
    archiving_controller_host_ref: AraRef,
    archiving_controller_interface: *const AraArchivingInterface,
    content_access_controller_host_ref: AraRef,
    content_access_controller_interface: *const AraContentAccessInterface,
    model_update_controller_host_ref: AraRef,
    model_update_controller_interface: *const AraModelUpdateInterface,
    playback_controller_host_ref: AraRef,
    playback_controller_interface: *const AraPlaybackInterface,
}

// --- AudioAccess 回调 -------------------------------------------------------

/// 创建音频读取器：只认会话登记的 source host ref；其余 ⇒ 空指针。
unsafe extern "C" fn ara_create_audio_reader(
    controller_host_ref: AraRef,
    audio_source_host_ref: AraRef,
    use_64_bit_samples: AraBool,
) -> AraRef {
    let Some(shared) = shared_from(controller_host_ref) else {
        return ptr::null_mut();
    };
    let expected = shared.source_host_ref.lock().ok().map(|guard| *guard);
    if expected != Some(audio_source_host_ref) {
        return ptr::null_mut();
    }
    let Some(pcm) = shared
        .source_pcm
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
    else {
        return ptr::null_mut();
    };
    let entry = Box::new(AudioReaderEntry {
        pcm,
        use_64_bit: use_64_bit_samples != ARA_FALSE,
        lock: Mutex::new(()),
    });
    let ptr = (&*entry) as *const AudioReaderEntry as AraRef;
    if let Ok(mut readers) = shared.audio_readers.lock() {
        readers.push(entry);
        ptr
    } else {
        ptr::null_mut()
    }
}

/// 读采样：越界填静音（⛔ 不算错）；`buffers` 为空或 reader 非法 ⇒ 假。
unsafe extern "C" fn ara_read_audio_samples(
    controller_host_ref: AraRef,
    audio_reader_host_ref: AraRef,
    sample_position: i64,
    samples_per_channel: i64,
    buffers: *const *mut c_void,
) -> AraBool {
    if buffers.is_null() || samples_per_channel < 0 {
        return ARA_FALSE;
    }
    let Some(shared) = shared_from(controller_host_ref) else {
        return ARA_FALSE;
    };
    // SAFETY: reader host ref 是本回调创建的条目指针；插件保证调用期间有效。
    let entry = unsafe { &*audio_reader_host_ref.cast::<AudioReaderEntry>() };
    let Ok(_guard) = entry.lock.lock() else {
        return ARA_FALSE;
    };
    let _ = shared;
    for channel in 0..entry.pcm.channel_count() {
        // SAFETY: `buffers` 是插件提供的声道指针数组；下标在声道数内。
        let dst = unsafe { *buffers.add(channel) };
        if dst.is_null() {
            continue;
        }
        let source = &entry.pcm.channels[channel];
        for index in 0..samples_per_channel {
            let at = sample_position + index;
            let value = if at >= 0 && (at as usize) < source.len() {
                source[at as usize]
            } else {
                0.0
            };
            if entry.use_64_bit {
                // SAFETY: 64 位模式下目标是 f64 数组；下标在块内。
                unsafe {
                    *(dst.cast::<f64>().add(index as usize)) = f64::from(value);
                }
            } else {
                // SAFETY: 32 位模式下目标是 f32 数组；下标在块内。
                unsafe {
                    *(dst.cast::<f32>().add(index as usize)) = value;
                }
            }
        }
    }
    ARA_TRUE
}

/// 销毁音频读取器（按指针身份移出登记；找不到 ⇒ 静默忽略）。
unsafe extern "C" fn ara_destroy_audio_reader(
    controller_host_ref: AraRef,
    audio_reader_host_ref: AraRef,
) {
    let Some(shared) = shared_from(controller_host_ref) else {
        return;
    };
    if let Ok(mut readers) = shared.audio_readers.lock()
        && let Some(index) = readers.iter().position(|entry| {
            (&**entry) as *const AudioReaderEntry as AraRef == audio_reader_host_ref
        })
    {
        readers.remove(index);
    }
}

// --- Archiving 回调 ---------------------------------------------------------

unsafe extern "C" fn ara_get_archive_size(
    _controller_host_ref: AraRef,
    archive_reader_host_ref: AraRef,
) -> AraSize {
    if archive_reader_host_ref.is_null() {
        return 0;
    }
    // SAFETY: reader host ref 是会话建的归档读取器槽指针；调用期间有效。
    unsafe { &*archive_reader_host_ref.cast::<ArchiveReaderSlot>() }
        .data
        .len()
}

unsafe extern "C" fn ara_read_bytes_from_archive(
    _controller_host_ref: AraRef,
    archive_reader_host_ref: AraRef,
    position: AraSize,
    length: AraSize,
    buffer: *mut u8,
) -> AraBool {
    if archive_reader_host_ref.is_null() || buffer.is_null() {
        return ARA_FALSE;
    }
    // SAFETY: 同上；`buffer` 是插件提供的写槽。
    let slot = unsafe { &*archive_reader_host_ref.cast::<ArchiveReaderSlot>() };
    let Some(end) = position.checked_add(length) else {
        return ARA_FALSE;
    };
    if end > slot.data.len() {
        return ARA_FALSE;
    }
    // SAFETY: 范围已校验在数据内；`buffer` 是插件提供的写槽。
    unsafe {
        ptr::copy_nonoverlapping(slot.data.as_ptr().add(position), buffer, length);
    }
    ARA_TRUE
}

unsafe extern "C" fn ara_write_bytes_to_archive(
    _controller_host_ref: AraRef,
    archive_writer_host_ref: AraRef,
    position: AraSize,
    length: AraSize,
    buffer: *const u8,
) -> AraBool {
    if archive_writer_host_ref.is_null() || buffer.is_null() {
        return ARA_FALSE;
    }
    // SAFETY: writer host ref 是会话建的归档写入器槽指针；调用期间有效。
    let slot = unsafe { &*archive_writer_host_ref.cast::<ArchiveWriterSlot>() };
    let Ok(mut data) = slot.data.lock() else {
        return ARA_FALSE;
    };
    let Some(end) = position.checked_add(length) else {
        return ARA_FALSE;
    };
    if data.len() < end {
        data.resize(end, 0);
    }
    // SAFETY: 目标区已按需扩容；`buffer` 是插件提供的读槽。
    unsafe {
        ptr::copy_nonoverlapping(buffer, data.as_mut_ptr().add(position), length);
    }
    ARA_TRUE
}

unsafe extern "C" fn ara_notify_archiving_progress(controller_host_ref: AraRef, value: f32) {
    if let Some(shared) = shared_from(controller_host_ref)
        && let Ok(mut progress) = shared.archive_progress.lock()
    {
        *progress = value;
    }
}

unsafe extern "C" fn ara_notify_unarchiving_progress(controller_host_ref: AraRef, value: f32) {
    // SAFETY: 同模块回调，参数由插件提供（见被调函数的契约）。
    unsafe { ara_notify_archiving_progress(controller_host_ref, value) };
}

unsafe extern "C" fn ara_get_document_archive_id(
    _controller_host_ref: AraRef,
    archive_reader_host_ref: AraRef,
) -> *const c_char {
    if archive_reader_host_ref.is_null() {
        return ptr::null();
    }
    // SAFETY: 同上；`CString` 在槽生命周期内有效。
    unsafe { &*archive_reader_host_ref.cast::<ArchiveReaderSlot>() }
        .archive_id
        .as_ptr()
}

// --- ContentAccess 回调 -----------------------------------------------------

unsafe extern "C" fn ara_is_musical_context_content_available(
    controller_host_ref: AraRef,
    musical_context_host_ref: AraRef,
    content_type: i32,
) -> AraBool {
    let Some(shared) = shared_from(controller_host_ref) else {
        return ARA_FALSE;
    };
    let expected = shared.context_host_ref.lock().ok().map(|guard| *guard);
    if content_type == CONTENT_TYPE_TEMPO_ENTRIES && expected == Some(musical_context_host_ref) {
        ARA_TRUE
    } else {
        ARA_FALSE
    }
}

unsafe extern "C" fn ara_get_musical_context_content_grade(
    controller_host_ref: AraRef,
    musical_context_host_ref: AraRef,
    content_type: i32,
) -> i32 {
    // SAFETY: 回调参数由插件提供；函数只读共享态，见其契约。
    if unsafe {
        ara_is_musical_context_content_available(
            controller_host_ref,
            musical_context_host_ref,
            content_type,
        )
    } == ARA_TRUE
    {
        CONTENT_GRADE_ADJUSTED
    } else {
        0
    }
}

unsafe extern "C" fn ara_create_musical_context_content_reader(
    controller_host_ref: AraRef,
    musical_context_host_ref: AraRef,
    content_type: i32,
    _range: *const AraContentTimeRange,
) -> AraRef {
    // SAFETY: 回调参数由插件提供；函数只读共享态，见其契约。
    if unsafe {
        ara_is_musical_context_content_available(
            controller_host_ref,
            musical_context_host_ref,
            content_type,
        )
    } != ARA_TRUE
    {
        return ptr::null_mut();
    }
    let Some(shared) = shared_from(controller_host_ref) else {
        return ptr::null_mut();
    };
    let entries = shared.tempo_entries.lock().ok().map(|guard| guard.clone());
    let Some(entries) = entries else {
        return ptr::null_mut();
    };
    let entry = Box::new(TempoReaderEntry { entries });
    let ptr = (&*entry) as *const TempoReaderEntry as AraRef;
    if let Ok(mut readers) = shared.tempo_readers.lock() {
        readers.push(entry);
        ptr
    } else {
        ptr::null_mut()
    }
}

unsafe extern "C" fn ara_is_audio_source_content_available(
    _controller_host_ref: AraRef,
    _audio_source_host_ref: AraRef,
    _content_type: i32,
) -> AraBool {
    // 宿主不提供 audio source 内容（插件自分析；见设计 §1.3 ContentAccess）。
    ARA_FALSE
}

unsafe extern "C" fn ara_get_audio_source_content_grade(
    _controller_host_ref: AraRef,
    _audio_source_host_ref: AraRef,
    _content_type: i32,
) -> i32 {
    0
}

unsafe extern "C" fn ara_create_audio_source_content_reader(
    _controller_host_ref: AraRef,
    _audio_source_host_ref: AraRef,
    _content_type: i32,
    _range: *const AraContentTimeRange,
) -> AraRef {
    ptr::null_mut()
}

unsafe extern "C" fn ara_get_content_reader_event_count(
    _controller_host_ref: AraRef,
    content_reader_host_ref: AraRef,
) -> i32 {
    if content_reader_host_ref.is_null() {
        return 0;
    }
    // SAFETY: reader host ref 是本回调创建的 tempo 读取器指针；调用期间有效。
    let entry = unsafe { &*content_reader_host_ref.cast::<TempoReaderEntry>() };
    entry.entries.len() as i32
}

unsafe extern "C" fn ara_get_content_reader_data_for_event(
    _controller_host_ref: AraRef,
    content_reader_host_ref: AraRef,
    event_index: i32,
) -> *const c_void {
    if content_reader_host_ref.is_null() || event_index < 0 {
        return ptr::null();
    }
    // SAFETY: 同上；条目在读取器生命周期内不变，指针稳定。
    let entry = unsafe { &*content_reader_host_ref.cast::<TempoReaderEntry>() };
    entry
        .entries
        .get(event_index as usize)
        .map_or(ptr::null(), |item| {
            item as *const AraContentTempoEntry as *const c_void
        })
}

unsafe extern "C" fn ara_destroy_content_reader(
    controller_host_ref: AraRef,
    content_reader_host_ref: AraRef,
) {
    let Some(shared) = shared_from(controller_host_ref) else {
        return;
    };
    if let Ok(mut readers) = shared.tempo_readers.lock()
        && let Some(index) = readers.iter().position(|entry| {
            (&**entry) as *const TempoReaderEntry as AraRef == content_reader_host_ref
        })
    {
        readers.remove(index);
    }
}

// --- ModelUpdate 回调 -------------------------------------------------------

unsafe extern "C" fn ara_notify_analysis_progress(
    controller_host_ref: AraRef,
    _audio_source_host_ref: AraRef,
    state: i32,
    value: f32,
) {
    if let Some(shared) = shared_from(controller_host_ref)
        && let Ok(mut progress) = shared.analysis_progress.lock()
    {
        *progress = (state, value);
    }
}

unsafe extern "C" fn ara_notify_audio_source_content_changed(
    controller_host_ref: AraRef,
    _audio_source_host_ref: AraRef,
    _range: *const AraContentTimeRange,
    _flags: i32,
) {
    if let Some(shared) = shared_from(controller_host_ref) {
        shared.content_changed.store(true, Ordering::SeqCst);
    }
}

unsafe extern "C" fn ara_notify_audio_modification_content_changed(
    controller_host_ref: AraRef,
    _audio_source_host_ref: AraRef,
    _range: *const AraContentTimeRange,
    _flags: i32,
) {
    if let Some(shared) = shared_from(controller_host_ref) {
        shared.content_changed.store(true, Ordering::SeqCst);
    }
}

unsafe extern "C" fn ara_notify_playback_region_content_changed(
    controller_host_ref: AraRef,
    _playback_region_host_ref: AraRef,
    _range: *const AraContentTimeRange,
    _flags: i32,
) {
    if let Some(shared) = shared_from(controller_host_ref) {
        shared.content_changed.store(true, Ordering::SeqCst);
    }
}

unsafe extern "C" fn ara_notify_document_data_changed(controller_host_ref: AraRef) {
    if let Some(shared) = shared_from(controller_host_ref) {
        shared.content_changed.store(true, Ordering::SeqCst);
    }
}

// --- Playback 回调（只记不执行） ---------------------------------------------

unsafe extern "C" fn ara_request_start_playback(controller_host_ref: AraRef) {
    if let Some(shared) = shared_from(controller_host_ref)
        && let Ok(mut requests) = shared.playback_requests.lock()
    {
        requests.push((0, 0.0, 0.0));
    }
}

unsafe extern "C" fn ara_request_stop_playback(controller_host_ref: AraRef) {
    if let Some(shared) = shared_from(controller_host_ref)
        && let Ok(mut requests) = shared.playback_requests.lock()
    {
        requests.push((1, 0.0, 0.0));
    }
}

unsafe extern "C" fn ara_request_set_playback_position(
    controller_host_ref: AraRef,
    time_position: f64,
) {
    if let Some(shared) = shared_from(controller_host_ref)
        && let Ok(mut requests) = shared.playback_requests.lock()
    {
        requests.push((2, time_position, 0.0));
    }
}

unsafe extern "C" fn ara_request_set_cycle_range(
    controller_host_ref: AraRef,
    start_time: f64,
    duration: f64,
) {
    if let Some(shared) = shared_from(controller_host_ref)
        && let Ok(mut requests) = shared.playback_requests.lock()
    {
        requests.push((3, start_time, duration));
    }
}

unsafe extern "C" fn ara_request_enable_cycle(controller_host_ref: AraRef, enable: AraBool) {
    if let Some(shared) = shared_from(controller_host_ref)
        && let Ok(mut requests) = shared.playback_requests.lock()
    {
        requests.push((4, f64::from(enable), 0.0));
    }
}

// --- 静态接口表（`Box` 承载的 host instance 引用它们；地址恒定） --------------

static AUDIO_ACCESS_INTERFACE: AraAudioAccessInterface = AraAudioAccessInterface {
    struct_size: size_of::<AraAudioAccessInterface>(),
    create_audio_reader_for_source: ara_create_audio_reader,
    read_audio_samples: ara_read_audio_samples,
    destroy_audio_reader: ara_destroy_audio_reader,
};

static ARCHIVING_INTERFACE: AraArchivingInterface = AraArchivingInterface {
    struct_size: size_of::<AraArchivingInterface>(),
    get_archive_size: ara_get_archive_size,
    read_bytes_from_archive: ara_read_bytes_from_archive,
    write_bytes_to_archive: ara_write_bytes_to_archive,
    notify_document_archiving_progress: ara_notify_archiving_progress,
    notify_document_unarchiving_progress: ara_notify_unarchiving_progress,
    get_document_archive_id: ara_get_document_archive_id,
};

static CONTENT_ACCESS_INTERFACE: AraContentAccessInterface = AraContentAccessInterface {
    struct_size: size_of::<AraContentAccessInterface>(),
    is_musical_context_content_available: ara_is_musical_context_content_available,
    get_musical_context_content_grade: ara_get_musical_context_content_grade,
    create_musical_context_content_reader: ara_create_musical_context_content_reader,
    is_audio_source_content_available: ara_is_audio_source_content_available,
    get_audio_source_content_grade: ara_get_audio_source_content_grade,
    create_audio_source_content_reader: ara_create_audio_source_content_reader,
    get_content_reader_event_count: ara_get_content_reader_event_count,
    get_content_reader_data_for_event: ara_get_content_reader_data_for_event,
    destroy_content_reader: ara_destroy_content_reader,
};

static MODEL_UPDATE_INTERFACE: AraModelUpdateInterface = AraModelUpdateInterface {
    struct_size: size_of::<AraModelUpdateInterface>(),
    notify_audio_source_analysis_progress: ara_notify_analysis_progress,
    notify_audio_source_content_changed: ara_notify_audio_source_content_changed,
    notify_audio_modification_content_changed: ara_notify_audio_modification_content_changed,
    notify_playback_region_content_changed: ara_notify_playback_region_content_changed,
    notify_document_data_changed: ara_notify_document_data_changed,
};

static PLAYBACK_INTERFACE: AraPlaybackInterface = AraPlaybackInterface {
    struct_size: size_of::<AraPlaybackInterface>(),
    request_start_playback: ara_request_start_playback,
    request_stop_playback: ara_request_stop_playback,
    request_set_playback_position: ara_request_set_playback_position,
    request_set_cycle_range: ara_request_set_cycle_range,
    request_enable_cycle: ara_request_enable_cycle,
};

// ---------------------------------------------------------------------------
// Factory 发现与信息
// ---------------------------------------------------------------------------

/// ARA factory 的宿主侧读数（扫描的 ARA 能力列与绑定的版本判定共用）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AraFactoryInfo {
    /// 插件名（factory `plugInName`；类名配对的另一半）。
    pub plug_in_name: String,
    /// 厂商名。
    pub manufacturer: String,
    /// 插件版本串（版本变化→重新分析的判定键）。
    pub version: String,
    /// Factory 标识（`factoryID`）。
    pub factory_id: String,
    /// 文档归档标识（`documentArchiveID`）。
    pub document_archive_id: String,
    /// 可分析内容类型（PitchNet：`[10]`＝仅音符）。
    pub analyzeable_content_types: Vec<i32>,
    /// 支持的播放变换标志（PitchNet：`0`＝无时间拉伸）。
    pub supported_playback_transformation_flags: i32,
    /// 插件报告的最低／最高 API 世代。
    pub lowest_api_generation: i32,
    /// 插件报告的最低／最高 API 世代。
    pub highest_api_generation: i32,
}

/// 从 factory 原始指针读信息（字符串当场复制；factory 在模块生命周期内有效）。
///
/// # Safety
///
/// 调用方保证 `factory` 是插件返回的有效 `ARAFactory` 指针，且
/// `struct_size >= 128`（即含 `supportsStoringAudioFileChunks` 的完整布局）。
unsafe fn factory_info_from(factory: *const AraFactoryRaw) -> AraFactoryInfo {
    // SAFETY: 调用方保证指针与布局有效；字段按 ARAFactory 原文顺序读。
    let raw = unsafe { &*factory };
    let count = raw.analyzeable_content_types_count;
    let mut types = Vec::new();
    if !raw.analyzeable_content_types.is_null() {
        for index in 0..count {
            // SAFETY: 计数与指针均来自 factory；下标在计数内。
            types.push(unsafe { *raw.analyzeable_content_types.add(index) });
        }
    }
    AraFactoryInfo {
        plug_in_name: decode_plugin_string(raw.plug_in_name),
        manufacturer: decode_plugin_string(raw.manufacturer_name),
        version: decode_plugin_string(raw.version),
        factory_id: decode_plugin_string(raw.factory_id),
        document_archive_id: decode_plugin_string(raw.document_archive_id),
        analyzeable_content_types: types,
        supported_playback_transformation_flags: raw.supported_playback_transformation_flags,
        lowest_api_generation: raw.lowest_supported_api_generation,
        highest_api_generation: raw.highest_supported_api_generation,
    }
}

/// 扫描一个模块的 ARA factory（不初始化 ARA：factory 结构体是静态的；
///
/// 探针实测 PitchNet 在未初始化时即可读 factory）。
/// 返回 `None` ⇔ 模块无 `ARA Main Factory Class` 类，或创建／读取失败。
/// 类名配对不在此做（扫描列全部 ARA 类的第一条；配对在会话打开时做）。
#[must_use]
pub fn inspect_ara_factory(module: &Vst3Module) -> Option<AraFactoryInfo> {
    let raw = module.classes_raw();
    let (cid, _name, _category) = raw
        .iter()
        .find(|(_cid, _name, category)| category == "ARA Main Factory Class")
        .cloned()?;
    let obj = module.create_instance(&cid, &IMAIN_FACTORY_IID).ok()?;
    // SAFETY: `obj` 是刚创建的 IMainFactory 对象；虚表首字段为其虚表。
    let vtbl = unsafe { ara_vtbl::<AraMainFactoryVtbl>(obj) };
    // SAFETY: 同上；`getFactory` 无参（除 this），返回静态 factory 指针。
    let factory = unsafe { (vtbl.get_factory)(obj) };
    let info = if factory.is_null() {
        None
    } else {
        // SAFETY: factory 指针来自插件；布局校验：完整 ARAFactory 为 128 字节。
        let size = unsafe { (*factory).struct_size };
        if size < 128 {
            None
        } else {
            // SAFETY: 尺寸已校验，布局完整。
            Some(unsafe { factory_info_from(factory) })
        }
    };
    // SAFETY: `obj` 与一次 `createInstance` 成功配对，释放一次。
    unsafe { ara_release(obj) };
    info
}

/// 已初始化过的 factory 指针集（原型简化：只初始化一次、不反初始化）。
static INITIALIZED_FACTORIES: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// 按需初始化 ARA（factory 指针首次出现时调一次；已初始化过 ⇒ 跳过）。
///
/// # Errors
///
/// 工厂无初始化入口 ⇒ [`HostError::InterfaceMissing`]。
fn ensure_ara_initialized(factory: *const AraFactoryRaw) -> Result<(), HostError> {
    // SAFETY: 调用方保证 factory 指针有效。
    let raw = unsafe { &*factory };
    let Some(initialize) = raw.initialize_ara_with_configuration else {
        return Err(HostError::InterfaceMissing("ARAFactory::initializeARA"));
    };
    let key = factory as usize;
    let mut initialized = INITIALIZED_FACTORIES
        .lock()
        .map_err(|_| HostError::Other("ARA 初始化登记锁已毒化".to_owned()))?;
    if initialized.contains(&key) {
        return Ok(());
    }
    let config = AraInterfaceConfiguration {
        struct_size: size_of::<AraInterfaceConfiguration>(),
        // 选插件报告的最高世代（须 ≥ 2.0 草案，否则 region sequence 不可用）。
        desired_api_generation: raw.highest_supported_api_generation,
        assert_function_address: ptr::null(),
    };
    if raw.highest_supported_api_generation < API_GENERATION_2_0_DRAFT {
        return Err(HostError::Other(format!(
            "插件 ARA 世代最高为 {}，低于 2.0 草案（region sequence 不可用）",
            raw.highest_supported_api_generation
        )));
    }
    // SAFETY: `config` 在调用帧内；factory 指针来自已验证的模块。
    unsafe { initialize(&config) };
    initialized.push(key);
    Ok(())
}

// ---------------------------------------------------------------------------
// 会话输入与输出类型
// ---------------------------------------------------------------------------

/// 绑定到 ARA 会话的片段（原型只接受音频；MIDI 在构造期响亮失败）。
#[derive(Clone, Debug)]
pub enum AraClipKind {
    /// 音频片段（CAS 资产形态的 PCM；`persistent_id`＝资产哈希 hex）。
    Audio,
    /// MIDI 片段（⛔ v1 不做：ARA 是音频优先，见设计 §2 映射表）。
    Midi,
}

/// 会话打开参数（调用方提供已解码的 PCM；解码本身不在宿主层做）。
#[derive(Clone, Debug)]
pub struct AraClip {
    /// 片段名（audio source／modification 名的底本）。
    pub name: String,
    /// 片段 persistent ID（文档内唯一；原型用 CAS 资产哈希 hex）。
    pub persistent_id: String,
    /// 片段种类（`Midi` ⇒ 构造期响亮失败）。
    pub kind: AraClipKind,
    /// PCM（非交错声道；采样率与处理器激活共用）。
    pub pcm: PcmData,
    /// 工程速度（BPM；恒定 tempo 表的唯一速度值）。
    pub tempo_bpm: f64,
}

/// 循环／静音摆放（原型会话的单片段摆放形态；多片段是后续期）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AraPlacement {
    /// 循环迭代数（≥1；每次迭代各建一个 playback region）。
    pub loop_iterations: usize,
    /// 静音（true ⇒ 不建 region；取消静音时重建，不触发重新分析）。
    pub muted: bool,
}

impl Default for AraPlacement {
    fn default() -> Self {
        Self {
            loop_iterations: 1,
            muted: false,
        }
    }
}

/// 展开后的单个 playback region 规格（纯函数，供单测与会话共用）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AraRegionSpec {
    /// 播放时间起点（秒；自文档起点）。
    pub start_in_playback_time: f64,
    /// 播放时长（秒；＝修改时长，原型无时间拉伸）。
    pub duration_in_playback_time: f64,
    /// 修改时间起点（秒；原型恒为 0，即整段片段）。
    pub start_in_modification_time: f64,
    /// 修改时长（秒；＝片段全长）。
    pub duration_in_modification_time: f64,
}

/// 循环／静音展开：muted ⇒ 0 个 region；否则每次迭代一个 region，
///
/// 播放起点按迭代累进、修改范围恒为整段片段（ghost 语义：同 modification
/// 的多次摆放，改一处全改；独立编辑须先 clone，见设计 §2.2／§2.3）。
#[must_use]
pub fn expand_placement(placement: AraPlacement, clip_duration_seconds: f64) -> Vec<AraRegionSpec> {
    if placement.muted || placement.loop_iterations == 0 {
        return Vec::new();
    }
    (0..placement.loop_iterations)
        .map(|iteration| AraRegionSpec {
            start_in_playback_time: iteration as f64 * clip_duration_seconds,
            duration_in_playback_time: clip_duration_seconds,
            start_in_modification_time: 0.0,
            duration_in_modification_time: clip_duration_seconds,
        })
        .collect()
}

/// 内容查询返回的音符（`ARAContentNote` 的安全复制；插件侧指针不外泄）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AraNote {
    /// 平均频率（Hz；无确定音高时为 0）。
    pub frequency: f32,
    /// 量化音高（MIDI 号；无确定音高时为 `INVALID_PITCH_NUMBER`）。
    pub pitch_number: i32,
    /// 归一化力度（0.0..=1.0）。
    pub volume: f32,
    /// 起音时间（秒；相对片段起点）。
    pub start_position: f64,
    /// 起音锚点偏移（秒；相对起音）。
    pub attack_duration: f64,
    /// 音符时长（秒；相对起音）。
    pub note_duration: f64,
    /// 信号总时长（秒；相对起音，含释放尾）。
    pub signal_duration: f64,
}

/// 离线渲染结果（立体声；`left`／`right` 等长，帧数＝请求帧数）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AraRenderedAudio {
    /// 左声道。
    pub left: Vec<f32>,
    /// 右声道。
    pub right: Vec<f32>,
}

impl AraRenderedAudio {
    /// 峰值（两声道绝对值最大；空 ⇒ 0）。
    #[must_use]
    pub fn peak(&self) -> f32 {
        self.left
            .iter()
            .chain(self.right.iter())
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
    }

    /// 非零帧数（任一声道绝对值 ＞ 0 的帧；出声判定的量）。
    #[must_use]
    pub fn nonzero_frames(&self) -> usize {
        self.left
            .iter()
            .zip(self.right.iter())
            .filter(|(left, right)| **left != 0.0 || **right != 0.0)
            .count()
    }
}

/// 读档结果（版本变化时不读档、改走重新分析，见设计 §1.8）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AraRestoreOutcome {
    /// 读档成功（分析态已恢复，无需重新分析）。
    Restored,
    /// 版本变化：旧 blob 已丢弃、重新分析完成，附警告文本（进渲染收据）。
    ReanalyzedDueToVersion {
        /// 警告文本（ stored 版本 → 当前版本）。
        warning: String,
    },
}

/// 绑定后的处理器（VST3 实例＋ARA 扩展；实例拥有，扩展指针向实例借用）。
struct AraProcessor {
    instance: Vst3Instance,
    playback_renderer_ref: AraRef,
    playback_renderer_interface: *const AraPlaybackRendererInterface,
}

// SAFETY: 实例单主持有（与 `Vst3Instance` 同款契约）；扩展指针在实例
// 生命周期内有效；渲染调用只发生在持有 `&mut` 的调用方线程。
unsafe impl Send for AraProcessor {}

// ---------------------------------------------------------------------------
// 会话
// ---------------------------------------------------------------------------

/// ARA 会话（一文档一片段；见模块文档的原型边界）。
///
/// 所有权规则（⛔ 不可错）：
/// - `module` 拥有动态库：factory 指针与全部插件对象向它借用，库活得比
///   会话久（`Vst3Instance` 另持 `Arc`，故处理器实例也不先于库释放）。
/// - `shared`（`Arc`）的指针是 5 组 controller host ref：会话先销毁
///   controller 再释放 `shared`（见 `teardown` 顺序），回调永不悬空。
/// - 插槽 `Box` 的堆地址是 host ref：会话结构体移动不移动堆对象，
///   故 host ref 在会话生命周期内稳定。
/// - 销毁顺序：处理器 → 图对象（region→modification→source→sequence→
///   context，包在编辑事务内）→ document controller。
pub struct AraSession {
    module: Vst3Module,
    factory_info: AraFactoryInfo,
    shared: Arc<AraShared>,
    // 仅持有：地址在 open() 时交插件作 host instance 身份，
    // 之后不再被宿主解引用，但必须活到 controller 销毁之后。
    #[allow(dead_code)]
    host_instance: Box<AraDocumentControllerHostInstance>,
    controller_instance: *const AraDocumentControllerInstance,
    controller_ref: AraRef,
    controller_iface: *const AraDocumentControllerInterface,
    context_slot: Box<Slot>,
    source_slot: Box<Slot>,
    modification_slot: Box<Slot>,
    sequence_slot: Box<Slot>,
    // Box 保地址稳定：region 槽地址已交插件作 host ref，
    // Vec 扩容不得移动它们（故保留 Box，见 clippy::vec_box 豁免理由）。
    #[allow(clippy::vec_box)]
    region_slots: Vec<Box<Slot>>,
    context_ref: AraRef,
    source_ref: AraRef,
    modification_ref: AraRef,
    sequence_ref: AraRef,
    region_refs: Vec<AraRef>,
    clip_name: String,
    clip_persistent_id: String,
    modification_persistent_id: String,
    clip_duration_seconds: f64,
    clip_sample_rate: f64,
    /// 片段总帧数（`open` 时由 PCM 定；MCP 渲染用它定总帧数，不再反推）。
    clip_frames: u64,
    tempo_bpm: f64,
    processor_class_id: Tuid,
    processor_class_name: String,
    placement: AraPlacement,
    processor: Option<AraProcessor>,
    analysis_requested: bool,
    closed: bool,
}

// SAFETY: 会话单主持有（与 `Vst3Instance` 同款契约）：模型调用只发生在
// 持有 `&mut self` 的调用方线程；插件分析线程只调宿主回调（回调只碰
// `AraShared` 的锁与原子量）；裸指针字段是插件对象借用，随模块／会话
// 生命周期走，不跨会话共享。
unsafe impl Send for AraSession {}

/// 手写 `Debug`：会话含裸指针与插件对象，派生 `Debug` 会要求全部字段
/// 实现 `Debug`（指针类型没有有意义的调试输出）；这里只印可对账的摘要
/// （factory、片段名、时长、采样率、region 数、关闭态），不印任何指针。
impl std::fmt::Debug for AraSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AraSession")
            .field("factory", &self.factory_info)
            .field("clip_name", &self.clip_name)
            .field("clip_duration_seconds", &self.clip_duration_seconds)
            .field("clip_sample_rate", &self.clip_sample_rate)
            .field("region_count", &self.region_refs.len())
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl AraSession {
    /// 打开会话并建图（不触发分析；分析由 [`Self::wait_for_analysis`] 触发）。
    ///
    /// 顺序：种类校验（MIDI ⇒ 构造期响亮失败）→ 模块加载 → 类名配对 →
    /// ARA 初始化 → document controller 创建 → 编辑事务内建图。
    ///
    /// # Errors
    ///
    /// MIDI 片段、模块打不开、无 ARA 类、类名配对不上、ARA 世代过低、
    /// controller 创建失败、建图任一步空指针 ⇒ [`HostError`]。
    pub fn open(
        module_path: &str,
        plugin_name: Option<&str>,
        clip: AraClip,
        placement: AraPlacement,
    ) -> Result<Self, HostError> {
        if matches!(clip.kind, AraClipKind::Midi) {
            return Err(HostError::Other(
                "MIDI 轨不能挂 ARA 效果器（ARA 是音频优先；须先渲染成音频再绑 ARA）".to_owned(),
            ));
        }
        let resolved = crate::host::resolve_module_path(std::path::Path::new(module_path))
            .ok_or_else(|| {
                HostError::InvalidPluginPath(format!("插件路径非法或找不到模块: {module_path}"))
            })?;
        let resolved_string = resolved.to_string_lossy().into_owned();
        let module = Vst3Module::load(&resolved_string)?;
        let raw = module.classes_raw();
        let ara_classes: Vec<(Tuid, String)> = raw
            .iter()
            .filter(|(_cid, _name, category)| category == "ARA Main Factory Class")
            .map(|(cid, name, _category)| (*cid, name.clone()))
            .collect();
        if ara_classes.is_empty() {
            return Err(HostError::InterfaceMissing("ARA Main Factory Class"));
        }
        let (ara_cid, ara_name) = match plugin_name {
            Some(want) => ara_classes
                .iter()
                .find(|(_cid, name)| name == want)
                .cloned()
                .ok_or_else(|| HostError::ClassNotFound(format!("没有 ARA 类叫 `{want}`")))?,
            None => ara_classes
                .first()
                .cloned()
                .ok_or_else(|| HostError::ClassNotFound("没有 ARA 类".to_owned()))?,
        };
        let (proc_cid, proc_name) = raw
            .iter()
            .find(|(_cid, name, category)| category == "Audio Module Class" && *name == ara_name)
            .map(|(cid, name, _category)| (*cid, name.clone()))
            .ok_or_else(|| {
                HostError::ClassNotFound(format!(
                    "没有与 ARA 类 `{ara_name}` 同名的 Audio Module 类（类名配对失败）"
                ))
            })?;
        let main_factory_obj = module.create_instance(&ara_cid, &IMAIN_FACTORY_IID)?;
        // SAFETY: 刚创建的 IMainFactory 对象；虚表首字段为其虚表。
        let main_vtbl = unsafe { ara_vtbl::<AraMainFactoryVtbl>(main_factory_obj) };
        // SAFETY: 同上；`getFactory` 返回静态 factory 指针。
        let factory_ptr = unsafe { (main_vtbl.get_factory)(main_factory_obj) };
        if factory_ptr.is_null() {
            // SAFETY: 配对释放。
            unsafe { ara_release(main_factory_obj) };
            return Err(HostError::InterfaceMissing("ARAFactory"));
        }
        // SAFETY: factory 指针来自插件；尺寸校验在读信息前做。
        let factory_size = unsafe { (*factory_ptr).struct_size };
        if factory_size < 128 {
            // SAFETY: 配对释放。
            unsafe { ara_release(main_factory_obj) };
            return Err(HostError::Other(format!(
                "ARA factory 结构体过小（structSize = {factory_size}，完整布局须 128）"
            )));
        }
        // SAFETY: 尺寸已校验，布局完整。
        let factory_info = unsafe { factory_info_from(factory_ptr) };
        ensure_ara_initialized(factory_ptr)?;
        // SAFETY: 配对释放（factory 指针是静态的，不随对象释放而失效）。
        unsafe { ara_release(main_factory_obj) };

        let shared = Arc::new(AraShared::new());
        let shared_ptr = Arc::as_ptr(&shared) as AraRef;
        let host_instance = Box::new(AraDocumentControllerHostInstance {
            struct_size: size_of::<AraDocumentControllerHostInstance>(),
            audio_access_controller_host_ref: shared_ptr,
            audio_access_controller_interface: &AUDIO_ACCESS_INTERFACE,
            archiving_controller_host_ref: shared_ptr,
            archiving_controller_interface: &ARCHIVING_INTERFACE,
            content_access_controller_host_ref: shared_ptr,
            content_access_controller_interface: &CONTENT_ACCESS_INTERFACE,
            model_update_controller_host_ref: shared_ptr,
            model_update_controller_interface: &MODEL_UPDATE_INTERFACE,
            playback_controller_host_ref: shared_ptr,
            playback_controller_interface: &PLAYBACK_INTERFACE,
        });
        let doc_name = CString::new("Yeban ARA document").unwrap_or_default();
        let doc_props = AraDocumentProperties {
            struct_size: size_of::<AraDocumentProperties>(),
            name: doc_name.as_ptr(),
        };
        // SAFETY: factory 指针有效；host instance 与属性在调用帧内。
        let factory = unsafe { &*factory_ptr };
        let Some(create_controller) = factory.create_document_controller_with_document else {
            return Err(HostError::InterfaceMissing(
                "ARAFactory::createDocumentController",
            ));
        };
        // SAFETY: 同上；返回的 instance 指针归插件所有、会话负责销毁。
        let controller_instance = unsafe {
            create_controller(
                &*host_instance as *const AraDocumentControllerHostInstance,
                &doc_props,
            )
        };
        if controller_instance.is_null() {
            return Err(HostError::Other(
                "ARA document controller 创建失败（返回空指针）".to_owned(),
            ));
        }
        // SAFETY: instance 指针刚创建有效；字段按原文顺序读。
        let (controller_ref, controller_iface) = unsafe {
            (
                (*controller_instance).document_controller_ref,
                (*controller_instance).document_controller_interface,
            )
        };
        if controller_ref.is_null() || controller_iface.is_null() {
            return Err(HostError::Other(
                "ARA document controller 实例字段为空".to_owned(),
            ));
        }
        let clip_duration_seconds = clip.pcm.duration_seconds();
        let clip_sample_rate = clip.pcm.sample_rate;
        let clip_frames = clip.pcm.frames();
        let mut session = Self {
            module,
            factory_info,
            shared,
            host_instance,
            controller_instance,
            controller_ref,
            controller_iface,
            context_slot: Box::new([0u8; 64]),
            source_slot: Box::new([0u8; 64]),
            modification_slot: Box::new([0u8; 64]),
            sequence_slot: Box::new([0u8; 64]),
            region_slots: Vec::new(),
            context_ref: ptr::null_mut(),
            source_ref: ptr::null_mut(),
            modification_ref: ptr::null_mut(),
            sequence_ref: ptr::null_mut(),
            region_refs: Vec::new(),
            clip_name: clip.name.clone(),
            clip_persistent_id: clip.persistent_id.clone(),
            modification_persistent_id: format!("{}:mod", clip.persistent_id),
            clip_duration_seconds,
            clip_sample_rate,
            clip_frames,
            tempo_bpm: clip.tempo_bpm,
            processor_class_id: proc_cid,
            processor_class_name: proc_name,
            placement,
            processor: None,
            analysis_requested: false,
            closed: false,
        };
        if let Err(error) = session.build_graph(&clip) {
            session.teardown();
            return Err(error);
        }
        Ok(session)
    }

    /// 编辑事务内建图（context→source→modification→sequence→regions）。
    fn build_graph(&mut self, clip: &AraClip) -> Result<(), HostError> {
        let iface = self.iface()?;
        let controller = self.controller_ref;
        // SAFETY: controller 与接口指针在会话生命周期内有效；事务内调用。
        unsafe { (iface.begin_editing)(controller) };
        let result = self.build_graph_inner(clip);
        // SAFETY: 同上；`end_editing` 与 `begin_editing` 配对（失败也闭合事务）。
        unsafe { (iface.end_editing)(controller) };
        // SAFETY: 事务外通知插件发送待处理模型更新。
        unsafe { (iface.notify_model_updates)(controller) };
        result
    }

    fn build_graph_inner(&mut self, clip: &AraClip) -> Result<(), HostError> {
        let iface = self.iface()?;
        let controller = self.controller_ref;
        // Musical context（tempo 表先填共享态：插件在 endEditing 内外
        // 都可能经 ContentAccess 拉取，须在事务结束前就绪）。
        let context_name = CString::new("musical context").unwrap_or_default();
        let context_props = AraMusicalContextProperties {
            struct_size: size_of::<AraMusicalContextProperties>(),
            name: context_name.as_ptr(),
            order_index: 0,
            color: ptr::null(),
        };
        let context_host_ref = self.context_slot.as_ptr() as AraRef;
        // SAFETY: controller 有效；host ref 指向会话拥有的插槽；属性在调用帧内。
        let context_ref =
            unsafe { (iface.create_musical_context)(controller, context_host_ref, &context_props) };
        if context_ref.is_null() {
            return Err(HostError::Other("ARA musical context 创建失败".to_owned()));
        }
        self.context_ref = context_ref;
        let duration_quarters = self.clip_duration_seconds * self.tempo_bpm / 60.0;
        if let Ok(mut tempo) = self.shared.tempo_entries.lock() {
            *tempo = vec![
                AraContentTempoEntry {
                    time_position: 0.0,
                    quarter_position: 0.0,
                },
                AraContentTempoEntry {
                    time_position: self.clip_duration_seconds,
                    quarter_position: duration_quarters,
                },
            ];
        }
        if let Ok(mut slot) = self.shared.context_host_ref.lock() {
            *slot = context_host_ref;
        }
        // SAFETY: 同上；范围空指针＝全部内容。
        unsafe {
            (iface.update_musical_context_content)(controller, context_ref, ptr::null(), 0);
        }
        // Audio source（PCM 与 host ref 先填共享态：插件可能在创建调用内
        // 就建 reader，须先就绪）。
        let pcm = Arc::new(clip.pcm.clone());
        if let Ok(mut slot) = self.shared.source_pcm.lock() {
            *slot = Some(Arc::clone(&pcm));
        }
        let source_host_ref = self.source_slot.as_ptr() as AraRef;
        if let Ok(mut slot) = self.shared.source_host_ref.lock() {
            *slot = source_host_ref;
        }
        let source_name = CString::new(clip.name.clone()).unwrap_or_default();
        let source_id = CString::new(clip.persistent_id.clone()).unwrap_or_default();
        let source_props = AraAudioSourceProperties {
            struct_size: size_of::<AraAudioSourceProperties>(),
            name: source_name.as_ptr(),
            persistent_id: source_id.as_ptr(),
            sample_count: pcm.frames() as i64,
            sample_rate: pcm.sample_rate,
            channel_count: pcm.channel_count() as i32,
            merits_64_bit_samples: ARA_FALSE,
            channel_arrangement_data_type: 0,
            channel_arrangement: ptr::null(),
        };
        // SAFETY: 同上；属性在调用帧内。
        let source_ref =
            unsafe { (iface.create_audio_source)(controller, source_host_ref, &source_props) };
        if source_ref.is_null() {
            return Err(HostError::Other("ARA audio source 创建失败".to_owned()));
        }
        self.source_ref = source_ref;
        // 样本访问须在 reader 创建前开启：PitchNet 实测，绑定后才开则
        // JUCE 侧永不创建 reader，raw 渲染恒静默（盒上 DC／正弦探针证实）。
        // SAFETY: controller 与 source ref 有效。
        unsafe {
            (iface.enable_audio_source_samples_access)(controller, source_ref, ARA_TRUE);
        }
        // Audio modification。
        let mod_name = CString::new(format!("{} modification", clip.name)).unwrap_or_default();
        let mod_id = CString::new(self.modification_persistent_id.clone()).unwrap_or_default();
        let mod_props = AraAudioModificationProperties {
            struct_size: size_of::<AraAudioModificationProperties>(),
            name: mod_name.as_ptr(),
            persistent_id: mod_id.as_ptr(),
        };
        let mod_host_ref = self.modification_slot.as_ptr() as AraRef;
        // SAFETY: 同上。
        let mod_ref = unsafe {
            (iface.create_audio_modification)(controller, source_ref, mod_host_ref, &mod_props)
        };
        if mod_ref.is_null() {
            return Err(HostError::Other(
                "ARA audio modification 创建失败".to_owned(),
            ));
        }
        self.modification_ref = mod_ref;
        // Region sequence。
        let seq_name = CString::new(format!("{} track", clip.name)).unwrap_or_default();
        let seq_props = AraRegionSequenceProperties {
            struct_size: size_of::<AraRegionSequenceProperties>(),
            name: seq_name.as_ptr(),
            order_index: 0,
            musical_context_ref: context_ref,
            color: ptr::null(),
        };
        let seq_host_ref = self.sequence_slot.as_ptr() as AraRef;
        // SAFETY: 同上。
        let seq_ref =
            unsafe { (iface.create_region_sequence)(controller, seq_host_ref, &seq_props) };
        if seq_ref.is_null() {
            return Err(HostError::Other("ARA region sequence 创建失败".to_owned()));
        }
        self.sequence_ref = seq_ref;
        self.create_regions()?;
        Ok(())
    }

    /// 按当前摆放建全部 playback region（调用方须在编辑事务内调用，
    /// 或由 [`Self::set_placement`] 的事务包装调用）。
    fn create_regions(&mut self) -> Result<(), HostError> {
        let specs = expand_placement(self.placement, self.clip_duration_seconds);
        let iface = self.iface()?;
        let controller = self.controller_ref;
        for (index, spec) in specs.iter().enumerate() {
            let region_name =
                CString::new(format!("{} region {index}", self.clip_name)).unwrap_or_default();
            let props = AraPlaybackRegionProperties {
                struct_size: size_of::<AraPlaybackRegionProperties>(),
                transformation_flags: TRANSFORMATION_NONE,
                start_in_modification_time: spec.start_in_modification_time,
                duration_in_modification_time: spec.duration_in_modification_time,
                start_in_playback_time: spec.start_in_playback_time,
                duration_in_playback_time: spec.duration_in_playback_time,
                musical_context_ref: ptr::null_mut(),
                region_sequence_ref: self.sequence_ref,
                name: region_name.as_ptr(),
                color: ptr::null(),
            };
            let slot = Box::new([0u8; 64]);
            let host_ref = slot.as_ptr() as AraRef;
            // SAFETY: controller 有效；host ref 指向新插槽；属性在调用帧内。
            let region_ref = unsafe {
                (iface.create_playback_region)(controller, self.modification_ref, host_ref, &props)
            };
            if region_ref.is_null() {
                return Err(HostError::Other(format!(
                    "ARA playback region {index} 创建失败"
                )));
            }
            self.region_slots.push(slot);
            self.region_refs.push(region_ref);
        }
        Ok(())
    }

    /// 销毁全部 playback region（调用方须在编辑事务内调用）。
    fn destroy_regions(&mut self) {
        if self.controller_iface.is_null() || self.controller_ref.is_null() {
            self.region_slots.clear();
            self.region_refs.clear();
            return;
        }
        // SAFETY: 指针在会话生命周期内有效；销毁顺序 region 先于 modification。
        let iface = unsafe { &*self.controller_iface };
        let controller = self.controller_ref;
        for region_ref in self.region_refs.drain(..) {
            // SAFETY: region ref 由创建调用返回，尚未销毁。
            unsafe { (iface.destroy_playback_region)(controller, region_ref) };
        }
        self.region_slots.clear();
    }

    /// 取 controller 接口（按值复制；已关闭 ⇒ 响亮失败，⛔ 不返回悬空指针）。
    fn iface(&self) -> Result<AraDocumentControllerInterface, HostError> {
        if self.closed || self.controller_iface.is_null() {
            return Err(HostError::Other("ARA 会话已关闭".to_owned()));
        }
        // SAFETY: 指针在会话生命周期内有效；关闭前不释放；虚表按值复制。
        Ok(unsafe { *self.controller_iface })
    }

    /// Factory 信息（扫描列与版本判定共用）。
    #[must_use]
    pub fn factory_info(&self) -> &AraFactoryInfo {
        &self.factory_info
    }

    /// 当前摆放。
    #[must_use]
    pub const fn placement(&self) -> AraPlacement {
        self.placement
    }

    /// 当前 playback region 数（循环展开后；muted ⇒ 0）。
    #[must_use]
    pub fn region_count(&self) -> usize {
        self.region_refs.len()
    }

    /// 片段时长（秒）。
    #[must_use]
    pub const fn clip_duration_seconds(&self) -> f64 {
        self.clip_duration_seconds
    }

    /// 片段采样率（Hz；渲染与处理器激活共用）。
    #[must_use]
    pub const fn clip_sample_rate(&self) -> f64 {
        self.clip_sample_rate
    }

    /// 片段总帧数（`open` 时由 PCM 定）。
    #[must_use]
    pub const fn clip_frames(&self) -> u64 {
        self.clip_frames
    }

    /// 最近一次分析进度通知（状态，值；完成判定以轮询为准）。
    #[must_use]
    pub fn analysis_progress(&self) -> (i32, f32) {
        self.shared
            .analysis_progress
            .lock()
            .ok()
            .map(|guard| *guard)
            .unwrap_or((0, 0.0))
    }

    /// 插件侧内容变化通知是否到过。
    #[must_use]
    pub fn content_changed(&self) -> bool {
        self.shared.content_changed.load(Ordering::SeqCst)
    }

    /// 插件走带请求记录（v1 只记不执行）。
    #[must_use]
    pub fn playback_requests(&self) -> Vec<(u8, f64, f64)> {
        self.shared
            .playback_requests
            .lock()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// 换摆放（循环数／静音）：处理器已绑定时先停用、摘 region、重建、
    /// 重挂、重激活；未绑定时只重建文档 region。静音切换不触发重新分析
    /// （audio source 未变）；返回重建后的 region 数。
    ///
    /// # Errors
    ///
    /// 会话已关闭、重建失败 ⇒ [`HostError`]（失败时旧 region 已销毁，
    /// 调用方应以 [`Self::region_count`] 为准，不假设旧摆放仍在）。
    pub fn set_placement(&mut self, placement: AraPlacement) -> Result<usize, HostError> {
        let iface = self.iface()?;
        let controller = self.controller_ref;
        let mut processor = self.processor.take();
        if let Some(bound) = processor.as_mut() {
            use crate::types::PluginInstance;
            bound.instance.deactivate();
            if !bound.playback_renderer_interface.is_null() {
                // SAFETY: 扩展指针在实例生命周期内有效；region ref 有效。
                let renderer_iface = unsafe { &*bound.playback_renderer_interface };
                for region_ref in &self.region_refs {
                    // SAFETY: 同上；摘 region 须在非激活态（已停用）。
                    unsafe {
                        (renderer_iface.remove_playback_region)(
                            bound.playback_renderer_ref,
                            *region_ref,
                        );
                    }
                }
            }
        }
        // SAFETY: 事务配对；销毁与重建都在事务内。
        unsafe { (iface.begin_editing)(controller) };
        self.destroy_regions();
        self.placement = placement;
        let result = self.create_regions();
        // SAFETY: 同上。
        unsafe { (iface.end_editing)(controller) };
        // SAFETY: 事务外通知。
        unsafe { (iface.notify_model_updates)(controller) };
        result?;
        if let Some(mut bound) = processor {
            if !bound.playback_renderer_interface.is_null() {
                // SAFETY: 同上；重挂须在激活前。
                let renderer_iface = unsafe { &*bound.playback_renderer_interface };
                for region_ref in &self.region_refs {
                    // SAFETY: 同上。
                    unsafe {
                        (renderer_iface.add_playback_region)(
                            bound.playback_renderer_ref,
                            *region_ref,
                        );
                    }
                }
            }
            use crate::types::PluginInstance;
            bound
                .instance
                .activate(self.clip_sample_rate, 4096)
                .map_err(|error| HostError::Other(format!("ARA 处理器重激活失败: {error}")))?;
            self.processor = Some(bound);
        }
        Ok(self.region_refs.len())
    }

    /// 分析是否未完成（音符内容；轮询判定，不以通知为准）。
    ///
    /// # Errors
    ///
    /// 会话已关闭 ⇒ [`HostError`]。
    pub fn analysis_incomplete(&self) -> Result<bool, HostError> {
        let iface = self.iface()?;
        // SAFETY: controller 与 source ref 有效。
        let incomplete = unsafe {
            (iface.is_audio_source_content_analysis_incomplete)(
                self.controller_ref,
                self.source_ref,
                CONTENT_TYPE_NOTES,
            )
        };
        Ok(incomplete != ARA_FALSE)
    }

    /// 音符内容是否可用（分析完成前通常为假；完成判定仍以轮询为准）。
    pub fn analysis_available(&self) -> Result<bool, HostError> {
        let iface = self.iface()?;
        // SAFETY: 同上。
        let available = unsafe {
            (iface.is_audio_source_content_available)(
                self.controller_ref,
                self.source_ref,
                CONTENT_TYPE_NOTES,
            )
        };
        Ok(available != ARA_FALSE)
    }

    /// 触发分析并等完成：首次调用时请求音符分析，之后每 100ms
    /// `notifyModelUpdates`＋轮询内容可用性（⛔ 不得以
    /// `isAudioSourceContentAnalysisIncomplete == false` 为完成证据——
    /// 从未开始分析的插件该标志恒为假，旧语义会把“未开始”误报为
    // “已完成”；完成判定只认内容可用）。
    /// 超时 ⇒ 响亮失败（⛔ 不得静默渲染半分析态）。
    ///
    /// # Errors
    ///
    /// 会话已关闭、分析超时 ⇒ [`HostError`]。
    pub fn wait_for_analysis(&mut self, timeout: Duration) -> Result<Duration, HostError> {
        let iface = self.iface()?;
        let controller = self.controller_ref;
        if !self.analysis_requested {
            let types = [CONTENT_TYPE_NOTES];
            // SAFETY: controller 与 source ref 有效；类型数组在调用帧内。
            unsafe {
                (iface.request_audio_source_content_analysis)(
                    controller,
                    self.source_ref,
                    types.len(),
                    types.as_ptr(),
                );
            }
            self.analysis_requested = true;
        }
        let started = Instant::now();
        loop {
            // SAFETY: 事务外定期通知（插件借此投递模型更新回调）。
            unsafe { (iface.notify_model_updates)(controller) };
            if self.analysis_available()? {
                return Ok(started.elapsed());
            }
            if started.elapsed() >= timeout {
                let (state, value) = self.analysis_progress();
                return Err(HostError::Other(format!(
                    "ARA 分析超时（{}ms 未完成；最后进度 state = {state} value = {value}）",
                    timeout.as_millis()
                )));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// 内容查询：读全部音符（只读通道；插件侧指针当场复制，不外泄）。
    ///
    /// # Errors
    ///
    /// 会话已关闭、内容不可用、读取器创建／事件指针异常 ⇒ [`HostError`]。
    pub fn notes(&self) -> Result<Vec<AraNote>, HostError> {
        let iface = self.iface()?;
        let controller = self.controller_ref;
        if !self.analysis_available()? {
            return Err(HostError::Other(
                "ARA 音符内容不可用（分析未完成或插件不提供）".to_owned(),
            ));
        }
        // SAFETY: controller 与 source ref 有效；范围空指针＝全部。
        let reader = unsafe {
            (iface.create_audio_source_content_reader)(
                controller,
                self.source_ref,
                CONTENT_TYPE_NOTES,
                ptr::null(),
            )
        };
        if reader.is_null() {
            return Err(HostError::Other("ARA 音符读取器创建失败".to_owned()));
        }
        // SAFETY: reader 由上调用返回；事件指针归插件所有、当场复制。
        let result = (|| -> Result<Vec<AraNote>, HostError> {
            // SAFETY: controller 与 reader 由上调用返回、调用内有效。
            let count = unsafe { (iface.get_content_reader_event_count)(controller, reader) };
            if count < 0 {
                return Err(HostError::Other(format!("ARA 音符事件数异常: {count}")));
            }
            let mut out = Vec::with_capacity(count as usize);
            for index in 0..count {
                // SAFETY: index 已在 count 范围内校验；数据当场复制。
                let data =
                    unsafe { (iface.get_content_reader_data_for_event)(controller, reader, index) };
                if data.is_null() {
                    return Err(HostError::Other(format!(
                        "ARA 音符事件 {index} 数据指针为空"
                    )));
                }
                // SAFETY: 数据指针是插件的 `ARAContentNote`，当场复制。
                let raw = unsafe { &*data.cast::<AraContentNoteRaw>() };
                out.push(AraNote {
                    frequency: raw.frequency,
                    pitch_number: raw.pitch_number,
                    volume: raw.volume,
                    start_position: raw.start_position,
                    attack_duration: raw.attack_duration,
                    note_duration: raw.note_duration,
                    signal_duration: raw.signal_duration,
                });
            }
            Ok(out)
        })();
        // SAFETY: reader 由上调用返回；销毁与创建配对。
        unsafe { (iface.destroy_content_reader)(controller, reader) };
        result
    }

    /// 首个 region 的 head／tail 时间（秒；无 region ⇒ 0／0）。
    pub fn head_tail(&self) -> Result<(f64, f64), HostError> {
        let Some(region_ref) = self.region_refs.first().copied() else {
            return Ok((0.0, 0.0));
        };
        let iface = self.iface()?;
        let mut head = 0.0;
        let mut tail = 0.0;
        // SAFETY: controller 与 region ref 有效；写槽在调用帧内。
        unsafe {
            (iface.get_playback_region_head_and_tail_time)(
                self.controller_ref,
                region_ref,
                &mut head,
                &mut tail,
            );
        }
        Ok((head, tail))
    }

    /// 绑定处理器（companion API）：类名配对的 Audio Module 类实例化 →
    /// `IPlugInEntryPoint2` 绑定（known＝播放＋编辑渲染＋编辑视图，
    /// assigned＝仅播放渲染）→ 挂 region → 激活。
    ///
    /// # Errors
    ///
    /// 实例化失败、非效果器类、无 EntryPoint2、绑定空指针、渲染器缺失、
    /// 激活失败 ⇒ [`HostError`]。
    pub fn bind_processor(&mut self) -> Result<(), HostError> {
        if self.processor.is_some() {
            return Ok(());
        }
        let mut instance = Vst3Instance::open(
            &self.module,
            &self.processor_class_id,
            &self.processor_class_name,
        )?;
        use crate::types::{PluginInstance, PluginKind};
        if instance.kind() != PluginKind::Effect {
            return Err(HostError::Other(format!(
                "ARA 配对类 `{}` 不是效果器（kind＝{:?}）",
                self.processor_class_name,
                instance.kind()
            )));
        }
        let component = instance.component_ptr();
        let entry_obj = Vst3Module::query_interface(component, &IPLUGIN_ENTRY_POINT2_IID)
            .map_err(|_| HostError::InterfaceMissing("ARA::IPlugInEntryPoint2"))?;
        // SAFETY: entry 对象由 queryInterface 返回；虚表首字段为其虚表。
        let entry_vtbl = unsafe { ara_vtbl::<AraEntryPoint2Vtbl>(entry_obj) };
        // SAFETY: 同上；controller ref 在会话生命周期内有效。
        let extension = unsafe {
            (entry_vtbl.bind_to_document_controller_with_roles)(
                entry_obj,
                self.controller_ref,
                ROLE_PLAYBACK_RENDERER | ROLE_EDITOR_RENDERER | ROLE_EDITOR_VIEW,
                ROLE_PLAYBACK_RENDERER,
            )
        };
        // SAFETY: entry 对象与一次 queryInterface 成功配对，释放一次。
        unsafe { ara_release(entry_obj) };
        if extension.is_null() {
            return Err(HostError::Other(
                "ARA bindToDocumentControllerWithRoles 返回空指针".to_owned(),
            ));
        }
        // SAFETY: 扩展指针由绑定调用返回；在实例生命周期内有效。
        let (renderer_ref, renderer_iface) = unsafe {
            (
                (*extension).playback_renderer_ref,
                (*extension).playback_renderer_interface,
            )
        };
        if renderer_ref.is_null() || renderer_iface.is_null() {
            return Err(HostError::Other(
                "ARA 播放渲染器缺失（未分配播放渲染角色）".to_owned(),
            ));
        }
        // SAFETY: 渲染器指针在实例生命周期内有效；挂 region 须在非激活态。
        let renderer = unsafe { &*renderer_iface };
        for region_ref in &self.region_refs {
            // SAFETY: 同上。
            unsafe { (renderer.add_playback_region)(renderer_ref, *region_ref) };
        }
        instance
            .activate(self.clip_sample_rate, 4096)
            .map_err(|error| HostError::Other(format!("ARA 处理器激活失败: {error}")))?;
        self.processor = Some(AraProcessor {
            instance,
            playback_renderer_ref: renderer_ref,
            playback_renderer_interface: renderer_iface,
        });
        Ok(())
    }

    /// 离线渲染整段（从第 0 帧起完整一遍；每次调用都从头渲染，
    /// 故两次调用结果逐位相同即确定性证据）。
    ///
    /// 走带：每块的 `ProcessContext` 置 playing＋tempo／拍号／音乐位置有效、
    /// `projectTimeSamples`＝块起点（ARA 渲染器据此定位 region 内音频）。
    ///
    /// 渲染率（原型限制，实测结论）：PitchNet 的 ARA 渲染器在本宿主流程
    /// 下固定 44100Hz——setup 48000／32000 与二次激活均不改变（盒上正弦
    /// 探针：48000 下首秒过零 959 ≈478.9Hz、32000 下 640 ≈319Hz，且
    /// 输出在 88200  playhead 采样处截断）。非 44100 片段 ⇒ 响亮失败，
    /// ⛔ 不得静默输出变调／截断音频；非 44100 宿主重采样是后续工作。
    ///
    /// 样本位宽（实测结论）：PitchNet 的 JUCE ARA 路径恒写 f32——盒上
    /// DC 探针：f64 缓冲按字节重解释为 f32 后现 0.5 直流，f32 路径输出
    /// 与之一致；与协商的 64 位 setup 无关。ARA 渲染一律走 f32。
    ///
    /// # Errors
    ///
    /// 会话已关闭、绑定／激活失败、块超限、非 44100 片段、渲染调用失败
    /// ⇒ [`HostError`]。
    pub fn render(
        &mut self,
        total_frames: u64,
        block_frames: usize,
    ) -> Result<AraRenderedAudio, HostError> {
        if block_frames == 0 || block_frames > 4096 {
            return Err(HostError::Other(format!(
                "ARA 渲染块大小非法: {block_frames}（须在 1..=4096 内，与激活上限一致）"
            )));
        }
        if (self.clip_sample_rate - 44_100.0).abs() >= 0.5 {
            return Err(HostError::Other(format!(
                "ARA 渲染仅支持 44100Hz 片段（当前 {}Hz；PitchNet 渲染器固定 44100Hz）",
                self.clip_sample_rate
            )));
        }
        self.bind_processor()?;
        let Some(bound) = self.processor.as_mut() else {
            return Err(HostError::Other("ARA 处理器未绑定".to_owned()));
        };
        let mut out = AraRenderedAudio::default();
        let mut position: u64 = 0;
        while position < total_frames {
            let frames = (total_frames - position).min(block_frames as u64) as usize;
            let quarters = position as f64 / self.clip_sample_rate * self.tempo_bpm / 60.0;
            let context = ProcessContext {
                _pad: 0,
                state: ProcessContext::STATE_PLAYING
                    | ProcessContext::STATE_TEMPO_VALID
                    | ProcessContext::STATE_TIME_SIG_VALID
                    | ProcessContext::STATE_PROJECT_TIME_MUSIC_VALID
                    | ProcessContext::STATE_BAR_POSITION_VALID,
                sample_rate: self.clip_sample_rate,
                project_time_samples: position as i64,
                system_time: 0,
                continuous_time_samples: position as i64,
                project_time_music: quarters,
                bar_position_music: quarters,
                cycle_start_music: 0.0,
                cycle_end_music: 0.0,
                tempo: self.tempo_bpm,
                time_sig_numerator: 4,
                time_sig_denominator: 4,
                chord_key: 0,
                chord_root: 0,
                chord_mask: 0,
                smpte_offset_subframes: 0,
                frame_rate_numerator: 0,
                frame_rate_denominator: 0,
                samples_to_next_clock: 0,
                _pad2: 0,
            };
            let mut left = vec![0.0_f32; frames];
            let mut right = vec![0.0_f32; frames];
            bound
                .instance
                .process_ara(frames, &mut left, &mut right, &context)
                .map_err(|error| HostError::Other(format!("ARA 渲染块失败: {error}")))?;
            out.left.extend_from_slice(&left);
            out.right.extend_from_slice(&right);
            position += frames as u64;
        }
        Ok(out)
    }

    /// 存档：`storeObjectsToArchive` 写文档 blob（文档数据＋本会话源与修改）。
    /// blob 不进工程 JSON；调用方以内容哈希为键进 CAS，工程只存哈希。
    ///
    /// # Errors
    ///
    /// 会话已关闭、存档调用失败 ⇒ [`HostError`]。
    pub fn store_blob(&self) -> Result<Vec<u8>, HostError> {
        let iface = self.iface()?;
        let writer = Box::new(ArchiveWriterSlot {
            data: Mutex::new(Vec::new()),
        });
        let writer_ref = (&*writer) as *const ArchiveWriterSlot as AraRef;
        let source_refs = [self.source_ref];
        let mod_refs = [self.modification_ref];
        let filter = AraStoreObjectsFilter {
            struct_size: size_of::<AraStoreObjectsFilter>(),
            document_data: ARA_TRUE,
            audio_source_refs_count: source_refs.len(),
            audio_source_refs: source_refs.as_ptr(),
            audio_modification_refs_count: mod_refs.len(),
            audio_modification_refs: mod_refs.as_ptr(),
        };
        // SAFETY: controller 有效；writer 槽在调用内有效；过滤器在调用帧内。
        let ok =
            unsafe { (iface.store_objects_to_archive)(self.controller_ref, writer_ref, &filter) };
        if ok == ARA_FALSE {
            return Err(HostError::Other(
                "ARA storeObjectsToArchive 失败".to_owned(),
            ));
        }
        writer
            .data
            .lock()
            .ok()
            .map(|guard| guard.clone())
            .ok_or_else(|| HostError::Other("ARA 存档缓冲锁已毒化".to_owned()))
    }

    /// 读档：版本一致时 `restoreObjectsFromArchive` 恢复分析态；版本变化时
    /// 丢弃旧 blob 并返回警告（⛔ 不得静默用旧分析，见设计 §1.8）。
    ///
    /// 调用方须保证本会话的图以与存档时相同的 persistent ID 建好
    /// （同一片段同一资产哈希）。版本变化路径不等待内容可用：重分析由
    /// 插件驱动，且无头 PitchNet 根本不发布 Notes 内容——以内容可用为门
    /// 会使每次版本变化都误报失败；收据警告本身就是该路径的交付物。
    /// 同理，同版本读档成功只认 archive 调用结果，不以 Notes 可用为门
    /// （恢复有效性由调用方以渲染同一性验证）。
    ///
    /// # Errors
    ///
    /// 会话已关闭、读档失败 ⇒ [`HostError`]。
    pub fn restore_blob(
        &mut self,
        blob: &[u8],
        stored_plugin_version: &str,
        _analysis_timeout: Duration,
    ) -> Result<AraRestoreOutcome, HostError> {
        let current_version = self.factory_info.version.clone();
        if stored_plugin_version != current_version {
            let warning = format!(
                "ARA 插件版本变化（{stored_plugin_version} → {current_version}）：旧文档 blob 已丢弃，重新分析"
            );
            return Ok(AraRestoreOutcome::ReanalyzedDueToVersion { warning });
        }
        let iface = self.iface()?;
        let controller = self.controller_ref;
        let archive_id =
            CString::new(self.factory_info.document_archive_id.clone()).unwrap_or_default();
        let reader = Box::new(ArchiveReaderSlot {
            data: blob.to_vec(),
            archive_id,
        });
        let reader_ref = (&*reader) as *const ArchiveReaderSlot as AraRef;
        let source_id = CString::new(self.clip_persistent_id.clone()).unwrap_or_default();
        let mod_id = CString::new(self.modification_persistent_id.clone()).unwrap_or_default();
        let source_ids = [source_id.as_ptr()];
        let mod_ids = [mod_id.as_ptr()];
        let filter = AraRestoreObjectsFilter {
            struct_size: size_of::<AraRestoreObjectsFilter>(),
            document_data: ARA_TRUE,
            audio_source_ids_count: source_ids.len(),
            audio_source_archive_ids: source_ids.as_ptr(),
            audio_source_current_ids: ptr::null(),
            audio_modification_ids_count: mod_ids.len(),
            audio_modification_archive_ids: mod_ids.as_ptr(),
            audio_modification_current_ids: ptr::null(),
        };
        // SAFETY: controller 有效；reader 槽在调用内有效；过滤器在调用帧内。
        unsafe { (iface.begin_editing)(controller) };
        // SAFETY: controller 与 reader 槽有效；过滤器指针在调用帧内。
        let ok = unsafe { (iface.restore_objects_from_archive)(controller, reader_ref, &filter) };
        // SAFETY: 事务配对。
        unsafe { (iface.end_editing)(controller) };
        // SAFETY: 事务外通知。
        unsafe { (iface.notify_model_updates)(controller) };
        if ok == ARA_FALSE {
            return Err(HostError::Other(
                "ARA restoreObjectsFromArchive 失败（blob 与当前插件／图不兼容）".to_owned(),
            ));
        }
        drop(reader);
        Ok(AraRestoreOutcome::Restored)
    }

    /// 显式关闭（幂等；`Drop` 同款顺序，错误静默——关闭路径不抛错）。
    pub fn close(&mut self) {
        self.teardown();
    }

    /// 销毁顺序：处理器 → 图对象（编辑事务内）→ document controller。
    /// 调用后全部插件指针置空；重复调用为空操作。
    fn teardown(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        // 处理器先释放（实例 Drop 做停用＋terminate＋release）。
        self.processor.take();
        if !self.controller_iface.is_null() && !self.controller_ref.is_null() {
            // SAFETY: 指针在会话生命周期内有效；销毁顺序见结构体文档。
            let iface = unsafe { &*self.controller_iface };
            let controller = self.controller_ref;
            // SAFETY: 事务配对（失败也闭合；销毁调用本身无错误通道）。
            unsafe { (iface.begin_editing)(controller) };
            self.destroy_regions();
            if !self.modification_ref.is_null() {
                // SAFETY: modification ref 由创建调用返回，尚未销毁。
                unsafe {
                    (iface.destroy_audio_modification)(controller, self.modification_ref);
                }
            }
            if !self.source_ref.is_null() {
                // SAFETY: 同上。
                unsafe { (iface.destroy_audio_source)(controller, self.source_ref) };
            }
            if !self.sequence_ref.is_null() {
                // SAFETY: 同上。
                unsafe { (iface.destroy_region_sequence)(controller, self.sequence_ref) };
            }
            if !self.context_ref.is_null() {
                // SAFETY: 同上。
                unsafe { (iface.destroy_musical_context)(controller, self.context_ref) };
            }
            // SAFETY: 同上。
            unsafe { (iface.end_editing)(controller) };
            // SAFETY: controller ref 由创建调用返回；销毁后指针不再使用。
            unsafe { (iface.destroy_document_controller)(controller) };
        }
        self.controller_instance = ptr::null();
        self.controller_ref = ptr::null_mut();
        self.controller_iface = ptr::null();
        self.context_ref = ptr::null_mut();
        self.source_ref = ptr::null_mut();
        self.modification_ref = ptr::null_mut();
        self.sequence_ref = ptr::null_mut();
        self.region_refs.clear();
        self.region_slots.clear();
    }
}

impl Drop for AraSession {
    fn drop(&mut self) {
        self.teardown();
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_expansion_loop_and_mute() {
        // 循环展开：3 次迭代各一个 region，播放起点累进、修改范围恒全段。
        let regions = expand_placement(
            AraPlacement {
                loop_iterations: 3,
                muted: false,
            },
            2.0,
        );
        assert_eq!(regions.len(), 3);
        assert_eq!(regions[0].start_in_playback_time, 0.0);
        assert_eq!(regions[1].start_in_playback_time, 2.0);
        assert_eq!(regions[2].start_in_playback_time, 4.0);
        for region in &regions {
            assert_eq!(region.duration_in_playback_time, 2.0);
            assert_eq!(region.start_in_modification_time, 0.0);
            assert_eq!(region.duration_in_modification_time, 2.0);
        }
        // 静音 ⇒ 0 个 region（摆放仍在工程里，取消静音可恢复）。
        assert!(
            expand_placement(
                AraPlacement {
                    loop_iterations: 3,
                    muted: true,
                },
                2.0,
            )
            .is_empty()
        );
        // 0 次迭代 ⇒ 0 个 region（防御：调用方不得传 0，但展开不 panic）。
        assert!(
            expand_placement(
                AraPlacement {
                    loop_iterations: 0,
                    muted: false,
                },
                2.0,
            )
            .is_empty()
        );
    }

    #[test]
    fn factory_layout_matches_probe() {
        // C 探针实测 PitchNet factory structSize = 128；Rust 布局须一致，
        // 否则 factory 字段读数全部错位。
        assert_eq!(size_of::<AraFactoryRaw>(), 128);
        assert_eq!(size_of::<AraDocumentControllerHostInstance>(), 88);
        assert_eq!(size_of::<AraPlaybackRendererInterface>(), 24);
        assert_eq!(size_of::<AraPlugInExtensionInstance>(), 72);
    }

    #[test]
    fn pcm_data_duration_and_frames() {
        let pcm = PcmData {
            channels: vec![vec![0.0; 48_000], vec![0.0; 48_000]],
            sample_rate: 48_000.0,
        };
        assert_eq!(pcm.frames(), 48_000);
        assert_eq!(pcm.channel_count(), 2);
        assert_eq!(pcm.duration_seconds(), 1.0);
    }

    #[test]
    fn rendered_audio_peak_and_nonzero() {
        let rendered = AraRenderedAudio {
            left: vec![0.0, 0.5, -0.25],
            right: vec![0.0, 0.0, 0.0],
        };
        assert_eq!(rendered.peak(), 0.5);
        assert_eq!(rendered.nonzero_frames(), 2);
    }
}

/// 真插件集成测试（门控：无 `YEBAN_ARA_TEST_PLUGIN` 时跳过，⛔ 不静默失败）。
///
/// 环境变量：
/// - `YEBAN_ARA_TEST_PLUGIN`：PitchNet `.vst3` 包路径或 `.so` 路径。
/// - `YEBAN_ARA_TEST_WAV`（可选）：测试用人声 wav 路径（16 位 PCM 或
///   f32，单声道／立体声）；不给则用内置阶梯正弦（F0 可检的代用信号，
///   ⛔ 不是人声，结论里须注明）。
/// - `YEBAN_ARA_TEST_TIMEOUT_SECS`（可选）：分析超时秒数，缺省 600。
/// - `YEBAN_ARA_TEST_BLOCK`（可选）：渲染块帧数，缺省 1024。
#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::fs::File;
    use std::io::Read;

    fn plugin_path() -> Option<String> {
        std::env::var("YEBAN_ARA_TEST_PLUGIN").ok()
    }

    /// 最小 wav 读取（RIFF PCM16／PCM24／f32；只取 data 块；⛔ 测试专用）。
    fn read_wav(path: &str) -> Result<PcmData, String> {
        let mut bytes = Vec::new();
        File::open(path)
            .and_then(|mut file| file.read_to_end(&mut bytes))
            .map_err(|error| format!("wav 打开失败: {error}"))?;
        if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err("不是 RIFF/WAVE".to_owned());
        }
        let mut offset = 12;
        let mut channels = 0_u16;
        let mut sample_rate = 0_u32;
        let mut bits = 0_u16;
        let mut format = 0_u16;
        let mut data: Option<(usize, usize)> = None;
        while offset + 8 <= bytes.len() {
            let id = &bytes[offset..offset + 4];
            let size =
                u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
            let body = offset + 8;
            if id == b"fmt " && size >= 16 {
                format = u16::from_le_bytes(bytes[body..body + 2].try_into().unwrap());
                channels = u16::from_le_bytes(bytes[body + 2..body + 4].try_into().unwrap());
                sample_rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().unwrap());
                bits = u16::from_le_bytes(bytes[body + 14..body + 16].try_into().unwrap());
            } else if id == b"data" {
                data = Some((body, size));
            }
            offset = body + size + (size % 2);
        }
        let (start, size) = data.ok_or("wav 无 data 块")?;
        if channels == 0 || sample_rate == 0 {
            return Err("wav fmt 块缺失".to_owned());
        }
        let bytes_per_sample = (bits / 8) as usize;
        if bytes_per_sample == 0 {
            return Err(format!("wav 位深不支持: {bits}"));
        }
        let frames = size / bytes_per_sample / channels as usize;
        let mut out: Vec<Vec<f32>> = vec![Vec::with_capacity(frames); channels as usize];
        for frame in 0..frames {
            for (channel, buf) in out.iter_mut().enumerate() {
                let at = start + (frame * channels as usize + channel) * bytes_per_sample;
                let sample = match (format, bits) {
                    (1, 16) => {
                        i16::from_le_bytes(bytes[at..at + 2].try_into().unwrap()) as f32 / 32768.0
                    }
                    (1, 24) => {
                        let raw = (bytes[at] as i32)
                            | ((bytes[at + 1] as i32) << 8)
                            | ((bytes[at + 2] as i32) << 16);
                        let signed = if raw & 0x80_0000 != 0 {
                            raw - 0x1_000_000
                        } else {
                            raw
                        };
                        signed as f32 / 8_388_608.0
                    }
                    (3, 32) => f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()),
                    _ => {
                        return Err(format!("wav 格式不支持: format = {format} bits = {bits}"));
                    }
                };
                buf.push(sample);
            }
        }
        Ok(PcmData {
            channels: out,
            sample_rate: f64::from(sample_rate),
        })
    }

    /// 阶梯正弦代用信号（8 个音阶音，每音 0.5 秒，48kHz 单声道，-12dBFS）：
    /// F0 轨迹明确，插件 F0 检测应能检出音符；⛔ 不是人声，结论须注明。
    fn synthetic_melody() -> PcmData {
        // 44100Hz：PitchNet 渲染器固定该速率（见 render() 注释），
        // 代用信号须与渲染率一致，否则输出变调／截断。
        let sample_rate = 44_100.0;
        let midi_notes = [57, 60, 62, 64, 65, 67, 69, 72];
        let mut samples = Vec::new();
        for note in midi_notes {
            let frequency = 440.0 * 2.0_f64.powf((f64::from(note) - 69.0) / 12.0);
            for index in 0..22_050 {
                let time = index as f64 / sample_rate;
                // 基频＋二次谐波（近人声的谐波结构），包络首尾各 20ms 淡入淡出。
                let envelope = (time / 0.02).min(1.0).min((0.5 - time) / 0.02);
                let value = (2.0 * std::f64::consts::PI * frequency * time).sin()
                    + 0.3 * (2.0 * std::f64::consts::PI * frequency * 2.0 * time).sin();
                samples.push((value * 0.25 * envelope.max(0.0)) as f32);
            }
        }
        PcmData {
            channels: vec![samples],
            sample_rate,
        }
    }

    fn test_clip() -> PcmData {
        match std::env::var("YEBAN_ARA_TEST_WAV") {
            Ok(path) => {
                read_wav(&path).unwrap_or_else(|error| panic!("测试 wav 读取失败: {error}"))
            }
            Err(_) => synthetic_melody(),
        }
    }

    fn timeout() -> Duration {
        std::env::var("YEBAN_ARA_TEST_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(600))
    }

    fn block_frames() -> usize {
        std::env::var("YEBAN_ARA_TEST_BLOCK")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1024)
    }

    fn open_session(placement: AraPlacement) -> AraSession {
        let Some(path) = plugin_path() else {
            panic!("YEBAN_ARA_TEST_PLUGIN 未设置，集成测试无法运行");
        };
        let pcm = test_clip();
        let clip = AraClip {
            name: "ara-test-clip".to_owned(),
            persistent_id: "ara-test-clip-asset".to_owned(),
            kind: AraClipKind::Audio,
            pcm,
            tempo_bpm: 120.0,
        };
        AraSession::open(&path, Some("PitchNet"), clip, placement)
            .unwrap_or_else(|error| panic!("ARA 会话打开失败: {error}"))
    }

    #[test]
    fn scan_finds_ara_factory() {
        let Some(path) = plugin_path() else {
            return;
        };
        let resolved = crate::host::resolve_module_path(std::path::Path::new(&path))
            .expect("测试插件路径解析失败");
        let module = Vst3Module::load(&resolved.to_string_lossy())
            .unwrap_or_else(|error| panic!("测试插件模块加载失败: {error}"));
        let info = inspect_ara_factory(&module).expect("ARA factory 未发现");
        assert_eq!(info.plug_in_name, "PitchNet");
        assert_eq!(info.version, "0.7.4");
        assert!(info.analyzeable_content_types.contains(&CONTENT_TYPE_NOTES));
    }

    #[test]
    fn midi_clip_fails_at_construction() {
        let Some(path) = plugin_path() else {
            return;
        };
        let clip = AraClip {
            name: "midi-clip".to_owned(),
            persistent_id: "midi-clip-asset".to_owned(),
            kind: AraClipKind::Midi,
            pcm: synthetic_melody(),
            tempo_bpm: 120.0,
        };
        let result = AraSession::open(&path, Some("PitchNet"), clip, AraPlacement::default());
        assert!(result.is_err(), "MIDI 轨挂 ARA 须在构造期响亮失败");
    }

    #[test]
    fn analysis_timeout_is_loud() {
        let Some(_path) = plugin_path() else {
            return;
        };
        let mut session = open_session(AraPlacement::default());
        // 1ms 超时：ONNX 分析不可能在此时间内完成（若完成则测试本身失效，
        // 须报失败而非静默通过——此处断言 Err 即是 fail-closed）。
        let result = session.wait_for_analysis(Duration::from_millis(1));
        assert!(result.is_err(), "极小超时须响亮失败，不得静默渲染半分析态");
        session.close();
    }
    // ② 证据性测试（⛔ 不是验收通过）：PitchNet 在无头流程中从不发布
    // Notes 内容——绑定后轮询 60 秒仍 avail=0、grade=0、incomplete=0，
    // 源码亦无 ContentReader／ARAContentNote 实现，分析只在编辑器存在
    // 时触发。故 ②（内容查询返回音符）在本插件上未通过；本测试断言
    // 等待与查询均响亮失败（fail-closed），作为 ② 未通过的证据。
    #[test]
    fn acceptance_02_notes_not_published_headless() {
        let Some(_path) = plugin_path() else {
            return;
        };
        let mut session = open_session(AraPlacement::default());
        let result = session.wait_for_analysis(Duration::from_secs(5));
        assert!(
            result.is_err(),
            "PitchNet 无头应始终无 Notes 内容可用（等待须超时失败）"
        );
        assert!(session.notes().is_err(), "PitchNet 无头音符查询须响亮失败");
        session.close();
    }

    #[test]
    fn full_acceptance_flow() {
        let Some(_path) = plugin_path() else {
            return;
        };
        // ② 见 acceptance_02_notes_not_published_headless（未通过）；
        // 本流程自 ③ 起跑 ③④⑤⑥（渲染不依赖 Notes 内容，raw 路径）。
        let mut session = open_session(AraPlacement::default());
        // ③ 离线渲染出声＋两次逐位相同。
        let total_frames = (session.clip_duration_seconds() * session.clip_sample_rate()) as u64;
        let first = session
            .render(total_frames, block_frames())
            .unwrap_or_else(|error| panic!("ARA 渲染失败: {error}"));
        println!(
            "ARA 渲染 peak={} nonzero={} frames={}",
            first.peak(),
            first.nonzero_frames(),
            total_frames
        );
        assert!(first.peak() > 0.0, "ARA 渲染静默（peak = 0）");
        let second = session
            .render(total_frames, block_frames())
            .unwrap_or_else(|error| panic!("ARA 二次渲染失败: {error}"));
        assert_eq!(first.left, second.left, "两次渲染左声道逐位不同");
        assert_eq!(first.right, second.right, "两次渲染右声道逐位不同");
        // ④ blob 存取往返＋新会话恢复后渲染逐位相同。
        let blob = session
            .store_blob()
            .unwrap_or_else(|error| panic!("ARA 存档失败: {error}"));
        println!("ARA 文档 blob 字节数={}", blob.len());
        assert!(!blob.is_empty(), "ARA 文档 blob 为空");
        let version = session.factory_info().version.clone();
        session.close();
        let mut restored = open_session(AraPlacement::default());
        let outcome = restored
            .restore_blob(&blob, &version, timeout())
            .unwrap_or_else(|error| panic!("ARA 读档失败: {error}"));
        assert_eq!(outcome, AraRestoreOutcome::Restored);
        let third = restored
            .render(total_frames, block_frames())
            .unwrap_or_else(|error| panic!("ARA 恢复后渲染失败: {error}"));
        assert_eq!(first.left, third.left, "恢复后渲染左声道与保存前逐位不同");
        assert_eq!(first.right, third.right, "恢复后渲染右声道与保存前逐位不同");
        // ⑤ 循环展开＋静音（同一会话内换摆放，不重新分析）。
        let count = restored
            .set_placement(AraPlacement {
                loop_iterations: 3,
                muted: false,
            })
            .unwrap_or_else(|error| panic!("ARA 循环摆放失败: {error}"));
        assert_eq!(count, 3, "3 次循环迭代须建 3 个 region");
        let looped = restored
            .render(total_frames * 3, block_frames())
            .unwrap_or_else(|error| panic!("ARA 循环渲染失败: {error}"));
        assert!(looped.peak() > 0.0, "ARA 循环渲染静默");
        let count = restored
            .set_placement(AraPlacement {
                loop_iterations: 3,
                muted: true,
            })
            .unwrap_or_else(|error| panic!("ARA 静音摆放失败: {error}"));
        assert_eq!(count, 0, "静音摆放须建 0 个 region");
        let silent = restored
            .render(total_frames * 3, block_frames())
            .unwrap_or_else(|error| panic!("ARA 静音渲染失败: {error}"));
        assert_eq!(silent.peak(), 0.0, "静音摆放须不出声");
        let count = restored
            .set_placement(AraPlacement {
                loop_iterations: 1,
                muted: false,
            })
            .unwrap_or_else(|error| panic!("ARA 取消静音失败: {error}"));
        assert_eq!(count, 1, "取消静音须恢复 1 个 region");
        let resumed = restored
            .render(total_frames, block_frames())
            .unwrap_or_else(|error| panic!("ARA 取消静音后渲染失败: {error}"));
        assert_eq!(first.left, resumed.left, "取消静音后渲染须与静音前逐位相同");
        assert_eq!(
            first.right, resumed.right,
            "取消静音后渲染须与静音前逐位相同"
        );
        // ⑥ 版本变化→重新分析＋警告（版本串变化模拟：传入篡改后的 stored 版本）。
        let mut versioned = open_session(AraPlacement::default());
        let outcome = versioned
            .restore_blob(&blob, "0.0.0-tampered", timeout())
            .unwrap_or_else(|error| panic!("ARA 版本变化重分析失败: {error}"));
        assert!(
            matches!(outcome, AraRestoreOutcome::ReanalyzedDueToVersion { .. }),
            "版本变化须走重新分析，实际: {outcome:?}"
        );
        versioned.close();
        restored.close();
    }

    // --- D 夹具测试（设计 §6／§7.6）-------------------------------------------
    // 夹具 `tests/fixtures/fixture_ara_plugin.rs` 现场编译，不依赖真插件，
    // 故本测试不设 `YEBAN_ARA_TEST_PLUGIN` 门（CI 机器上无真插件也能跑）。
    // 夹具发布固定 8 音符集（⛔ 不是分析结果）；断言查询结果与固定集逐项一致。
    // 边界：夹具只验证机制，不能替代真插件放行，真插件门仍敞开；② 的
    // PitchNet 失败仍记为插件限制，不因夹具通过而改写。

    #[cfg(target_os = "windows")]
    const ARA_FIXTURE_EXT: &str = "dll";
    #[cfg(target_os = "macos")]
    const ARA_FIXTURE_EXT: &str = "dylib";
    #[cfg(all(unix, not(target_os = "macos")))]
    const ARA_FIXTURE_EXT: &str = "so";

    static ARA_FIXTURE_COUNTER: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    /// D 夹具专用的临时目录（并行测试互不干扰：pid＋单调计数）。
    fn ara_fixture_dir() -> std::path::PathBuf {
        use std::sync::atomic::Ordering;
        let id = ARA_FIXTURE_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "yeban-ara-fixture-test-{}-{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("建 D 夹具测试临时目录");
        dir
    }

    /// 用 `rustc` 把 D 夹具源码现场编译成 cdylib，返回产物路径。
    fn compile_ara_fixture(dir: &std::path::Path) -> std::path::PathBuf {
        let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/fixture_ara_plugin.rs");
        assert!(source.is_file(), "D 夹具源码不在 {source:?}");
        let out = dir.join(format!("fixture_ara_plugin.{ARA_FIXTURE_EXT}"));
        let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
        let output = std::process::Command::new(&rustc)
            .args([
                "--crate-type",
                "cdylib",
                "--edition",
                "2021",
                "-o",
                out.to_str().expect("产物路径可转 UTF-8"),
                source.to_str().expect("夹具路径可转 UTF-8"),
            ])
            .output()
            .unwrap_or_else(|error| panic!("启动 {rustc} 失败: {error}"));
        assert!(
            output.status.success(),
            "D 夹具编译失败（{}）：\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(out.is_file(), "D 夹具产物不在 {out:?}");
        out
    }

    /// 在 D 夹具上开会话（阶梯正弦代用信号，与固定音符集同音高）。
    fn open_fixture_session(dir: &std::path::Path) -> (AraSession, std::path::PathBuf) {
        let plugin = compile_ara_fixture(dir);
        let clip = AraClip {
            name: "ara-fixture-clip".to_owned(),
            persistent_id: "ara-fixture-clip-asset".to_owned(),
            kind: AraClipKind::Audio,
            pcm: synthetic_melody(),
            tempo_bpm: 120.0,
        };
        let session = AraSession::open(
            plugin.to_str().expect("夹具路径可转 UTF-8"),
            Some("TestAra"),
            clip,
            AraPlacement::default(),
        )
        .unwrap_or_else(|error| panic!("D 夹具 ARA 会话打开失败: {error}"));
        (session, plugin)
    }

    /// 固定音符集的期望值（与夹具 `fixed_notes()` 同口径：音高、起音、时长）。
    fn expected_fixture_notes() -> Vec<(i32, f64, f64, f64, f64, f32)> {
        let midi_notes = [57, 60, 62, 64, 65, 67, 69, 72];
        midi_notes
            .iter()
            .enumerate()
            .map(|(index, note)| {
                let frequency = 440.0 * 2.0_f64.powf((f64::from(*note) - 69.0) / 12.0);
                (*note, index as f64 * 0.5, 0.02, 0.5, 0.5, frequency as f32)
            })
            .collect()
    }

    #[test]
    fn fixture_ara_scan_finds_factory() {
        let dir = ara_fixture_dir();
        let plugin = compile_ara_fixture(&dir);
        let resolved = crate::host::resolve_module_path(std::path::Path::new(&plugin))
            .expect("D 夹具路径解析失败");
        let module = Vst3Module::load(&resolved.to_string_lossy())
            .unwrap_or_else(|error| panic!("D 夹具模块加载失败: {error}"));
        let info = inspect_ara_factory(&module).expect("D 夹具 ARA factory 未发现");
        assert_eq!(info.plug_in_name, "TestAra");
        assert_eq!(info.version, "0.1.0");
        assert!(info.analyzeable_content_types.contains(&CONTENT_TYPE_NOTES));
    }

    #[test]
    fn fixture_ara_notes_query_returns_published_notes() {
        let dir = ara_fixture_dir();
        let (mut session, _plugin) = open_fixture_session(&dir);
        assert_eq!(session.factory_info().plug_in_name, "TestAra");
        // 分析前查询须响亮失败（fail-closed），不得返回空集冒充完成。
        assert!(
            session.notes().is_err(),
            "分析前音符查询须响亮失败，不得静默返回"
        );
        session
            .wait_for_analysis(Duration::from_secs(30))
            .unwrap_or_else(|error| panic!("D 夹具分析等待失败: {error}"));
        assert!(
            session
                .analysis_available()
                .expect("D 夹具分析可用性查询失败"),
            "D 夹具分析完成后内容须可用"
        );
        assert!(
            !session
                .analysis_incomplete()
                .expect("D 夹具分析未完成查询失败"),
            "D 夹具分析完成后未完成标志须为假"
        );
        let notes = session
            .notes()
            .unwrap_or_else(|error| panic!("D 夹具音符查询失败: {error}"));
        let expected = expected_fixture_notes();
        assert_eq!(notes.len(), expected.len(), "D 夹具音符数须为 8");
        for (note, (pitch, start, attack, duration, signal, frequency)) in
            notes.iter().zip(expected.iter())
        {
            assert_eq!(note.pitch_number, *pitch, "音高号须与固定集一致");
            assert_eq!(note.start_position, *start, "起音须与固定集一致");
            assert_eq!(note.attack_duration, *attack, "起音锚点须与固定集一致");
            assert_eq!(note.note_duration, *duration, "音符时长须与固定集一致");
            assert_eq!(note.signal_duration, *signal, "信号时长须与固定集一致");
            assert_eq!(note.volume, 1.0, "力度须为 1.0");
            assert!(
                (note.frequency - frequency).abs() < 1e-3,
                "频率须与 MIDI 音高对应（{frequency} Hz 附近），实际 {}",
                note.frequency
            );
        }
        session.close();
    }
}
