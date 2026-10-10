//! **R48：枚举型黄金表的编译期穷举探针**。
//!
//! ## 为什么需要这一份
//!
//! `assert_eq!(cases.len(), N)` 数的是**表**，不是**枚举** —— 给一个枚举加一个变体、
//! 忘了往表里加一行时，表还是 N 行、断言照样绿（"表在自比"）。本文件给每一个
//! **枚举型黄金表**配一个**无通配符 `match`** 的探针：把枚举的每个变体都写进 `match`，
//! 于是**加变体 ⇒ 本文件编译不过**（E0004），作者被迫回来决定它属于哪一类。
//!
//! ⚠ 如实登记边界：探针保证"**加变体一定编译不过**"，但**不**保证 `[Self; N]` 表的
//! 完整性 —— 这一点由每处的**长度字面值**（例如 `assert_eq!(ErrorCode::ALL.len(), 21)`）
//! 与"表里的每一项都能被探针命名"共同兜住。二者缺一都留下静默漂移的口子。
//!
//! ## 覆盖的表（本 crate 的全部枚举型金表）
//!
//! | 表 | 枚举 | 探针 |
//! | :--- | :--- | :--- |
//! | `ErrorCode::{ALL, SCHEMA_CONTRACT, SCHEMA_ONLY, DOCUMENTED_TOOL_CODES}` | `ErrorCode`（本地 21） | `code_name` 21 臂 |
//! | `Scope::ALL` | `Scope`（本地 6） | `scope_name` 6 臂 |
//! | `BuildCode::ALL` | `BuildCode`（本地 7） | `build_code_name` 7 臂 |
//! | `LANE_NAMES` | `LaneKind`（本地 5） | `lane_name` 5 臂 |
//! | `CURVE_NAMES` | `CurveType`（**模型** 4） | `curve_name` 4 臂 |
//! | `LANE_WRITE_MODES` | `AutomationWriteMode`（**模型** 4） | `write_mode` 4 臂 |
//! | `DEVICE_KINDS` | `DeviceKind`（**模型** 4） | `device_kind` 4 臂 |
//! | `StaticLane::NAMES` | `StaticLane`（本地 2） | `static_lane` 2 臂 |
//! | `TrackFlag::NAMES` | `TrackFlag`（本地 2） | `track_flag` 2 臂 |
//!
//! `Channel` / `RunMode` / `ChannelLayout` / `Violation` / `ContainerRejection` **没有**
//! 独立的表（它们的 `as_str` / `name()` 本身就是无通配符 `match`）⇒ 已有等价探针，不重复。

use std::collections::BTreeSet;

use yeban_mcp::domain::extension_pure::{CURVE_NAMES, LANE_NAMES, LaneKind};
use yeban_mcp::domain::notes::{DEVICE_KINDS, LANE_WRITE_MODES, StaticLane, TrackFlag};
use yeban_mcp::domain::section::error_code;
use yeban_mcp::domain::section_build::BuildCode;
use yeban_mcp::security::Scope;
use yeban_mcp::tools::ErrorCode;
use yeban_model::{AutomationWriteMode, CurveType, DeviceKind};

/// `ErrorCode` 的规范名 —— **无通配符**（加变体 ⇒ E0004）。
fn code_name(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::ProjectLocked => "PROJECT_LOCKED",
        ErrorCode::FileNotFound => "FILE_NOT_FOUND",
        ErrorCode::IoError => "IO_ERROR",
        ErrorCode::DiskFull => "DISK_FULL",
        ErrorCode::NoActiveProject => "NO_ACTIVE_PROJECT",
        ErrorCode::InvalidFieldSelector => "INVALID_FIELD_SELECTOR",
        ErrorCode::StyleNotFound => "STYLE_NOT_FOUND",
        ErrorCode::CycleDetected => "CYCLE_DETECTED",
        ErrorCode::ClipNotFound => "CLIP_NOT_FOUND",
        ErrorCode::OutOfRange => "OUT_OF_RANGE",
        ErrorCode::TrackNotFound => "TRACK_NOT_FOUND",
        ErrorCode::IndexOutOfBounds => "INDEX_OUT_OF_BOUNDS",
        ErrorCode::RenderFailed => "RENDER_FAILED",
        ErrorCode::Busy => "BUSY",
        ErrorCode::ProposalNotFound => "PROPOSAL_NOT_FOUND",
        ErrorCode::Conflict => "CONFLICT",
        ErrorCode::RoutingCycleDetected => "ROUTING_CYCLE_DETECTED",
        ErrorCode::EntityNotFound => "ENTITY_NOT_FOUND",
        ErrorCode::InvalidParameterRange => "INVALID_PARAMETER_RANGE",
        ErrorCode::PermissionDenied => "PERMISSION_DENIED",
        ErrorCode::NotImplemented => "NOT_IMPLEMENTED",
    }
}

/// `Scope` 的规范名 —— 无通配符（6 臂）。
fn scope_name(scope: Scope) -> &'static str {
    match scope {
        Scope::UiRead => "ui:read",
        Scope::UiScreenshot => "ui:screenshot",
        Scope::UiInject => "ui:inject",
        Scope::AppSave => "app:save",
        Scope::AppReloadEngine => "app:reload-engine",
        Scope::AppAdmin => "app:admin",
    }
}

/// `BuildCode` 的名字 —— 无通配符（7 臂）。
fn build_code_name(code: BuildCode) -> &'static str {
    match code {
        BuildCode::StyleNotFound => "StyleNotFound",
        BuildCode::CycleDetected => "CycleDetected",
        BuildCode::OutOfRange => "OutOfRange",
        BuildCode::InvalidParameterRange => "InvalidParameterRange",
        BuildCode::Conflict => "Conflict",
        BuildCode::ClipNotFound => "ClipNotFound",
        BuildCode::TrackNotFound => "TrackNotFound",
    }
}

/// `LaneKind` 的规范名 —— 无通配符（5 臂）。
fn lane_kind_name(kind: LaneKind) -> &'static str {
    match kind {
        LaneKind::TrackVolume => "TrackVolume",
        LaneKind::TrackPan => "TrackPan",
        LaneKind::SendGain => "SendGain",
        LaneKind::DeviceParam => "DeviceParam",
        LaneKind::Macro => "Macro",
    }
}

/// `CurveType`（**模型**枚举）的规范名 —— 无通配符（4 臂）。
fn curve_name(curve: CurveType) -> &'static str {
    match curve {
        CurveType::Linear => "Linear",
        CurveType::Exponential => "Exponential",
        CurveType::Logarithmic => "Logarithmic",
        CurveType::SCurve => "SCurve",
    }
}

/// `AutomationWriteMode`（**模型**枚举）的规范名 —— 无通配符（4 臂）。
fn write_mode_name(mode: AutomationWriteMode) -> &'static str {
    match mode {
        AutomationWriteMode::Off => "Off",
        AutomationWriteMode::Write => "Write",
        AutomationWriteMode::Touch => "Touch",
        AutomationWriteMode::Latch => "Latch",
    }
}

/// `DeviceKind`（**模型**枚举）的规范名 —— 无通配符（4 臂）。
fn device_kind_name(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::InternalInstrument => "InternalInstrument",
        DeviceKind::InternalEffect => "InternalEffect",
        DeviceKind::ExternalInstrument => "ExternalInstrument",
        DeviceKind::ExternalEffect => "ExternalEffect",
    }
}

/// `StaticLane` 的规范名 —— 无通配符（2 臂）。
fn static_lane_name(lane: StaticLane) -> &'static str {
    match lane {
        StaticLane::TrackVolume => "TrackVolume",
        StaticLane::TrackPan => "TrackPan",
    }
}

/// `TrackFlag` 的 `kind` 名 —— 无通配符（2 臂）。
fn track_flag_kind(flag: TrackFlag) -> &'static str {
    match flag {
        TrackFlag::Mute => "setTrackMute",
        TrackFlag::Solo => "setTrackSolo",
    }
}

#[test]
fn the_error_code_tables_are_probed_without_a_wildcard() {
    assert_eq!(ErrorCode::ALL.len(), 21, "全部错误码");
    assert_eq!(ErrorCode::SCHEMA_CONTRACT.len(), 20, "契约 enum");
    assert_eq!(ErrorCode::SCHEMA_ONLY.len(), 4);
    assert_eq!(ErrorCode::DOCUMENTED_TOOL_CODES.len(), 16);
    // 探针命名的名字必须与 `as_str` 一致（21 个都要走一遍）。
    let mut names = BTreeSet::new();
    for code in ErrorCode::ALL {
        assert_eq!(code_name(code), code.as_str(), "{code:?} 的规范名");
        assert!(names.insert(code_name(code)), "名字重复: {code:?}");
    }
    assert_eq!(names.len(), 21);
    // 三张表都必须是"探针命名得出的集合"的子集；`ALL` 必须等于 `SCHEMA_CONTRACT ∪ {NOT_IMPLEMENTED}`。
    let contract: BTreeSet<&str> = ErrorCode::SCHEMA_CONTRACT
        .iter()
        .map(|c| code_name(*c))
        .collect();
    let schema_only: BTreeSet<&str> = ErrorCode::SCHEMA_ONLY
        .iter()
        .map(|c| code_name(*c))
        .collect();
    let documented: BTreeSet<&str> = ErrorCode::DOCUMENTED_TOOL_CODES
        .iter()
        .map(|c| code_name(*c))
        .collect();
    assert_eq!(contract.len(), 20, "表内不得重复");
    assert_eq!(schema_only.len(), 4);
    assert_eq!(documented.len(), 16);
    assert!(
        schema_only.is_subset(&contract),
        "SCHEMA_ONLY ⊆ SCHEMA_CONTRACT"
    );
    assert!(
        documented.is_subset(&contract),
        "DOCUMENTED ⊆ SCHEMA_CONTRACT"
    );
    let mut expected = contract.clone();
    expected.insert("NOT_IMPLEMENTED");
    assert_eq!(
        names, expected,
        "ALL == SCHEMA_CONTRACT ∪ {{NOT_IMPLEMENTED}}"
    );
    // 分类函数与探针一致（20 个 true + 1 个 false）。
    for code in ErrorCode::ALL {
        assert_eq!(
            code.is_schema_contract(),
            ErrorCode::SCHEMA_CONTRACT.contains(&code),
            "{code:?} 的 `is_schema_contract`"
        );
    }
}

#[test]
fn the_scope_table_is_probed_without_a_wildcard() {
    assert_eq!(Scope::ALL.len(), 6);
    let mut names = BTreeSet::new();
    for scope in Scope::ALL {
        assert!(names.insert(scope_name(scope)), "作用域名重复: {scope:?}");
        assert_eq!(scope.as_str(), scope_name(scope), "{scope:?} 的规范名");
    }
    assert_eq!(names.len(), 6);
    // 六个作用域两两不同（表自比查不出这件事）。
    assert_eq!(
        names,
        [
            "app:admin",
            "app:reload-engine",
            "app:save",
            "ui:inject",
            "ui:read",
            "ui:screenshot"
        ]
        .into_iter()
        .collect()
    );
}

#[test]
fn the_build_code_table_is_probed_without_a_wildcard() {
    assert_eq!(BuildCode::ALL.len(), 7);
    let mut names = BTreeSet::new();
    for code in BuildCode::ALL {
        assert!(names.insert(build_code_name(code)), "类别重复: {code:?}");
        // 每个类别都必须落在**契约** enum 里（探针把 7 个臂都点到）。
        assert!(
            error_code(code).is_schema_contract(),
            "{code:?} 映射到了契约外的码"
        );
    }
    assert_eq!(names.len(), 7);
    assert_eq!(
        names,
        [
            "ClipNotFound",
            "Conflict",
            "CycleDetected",
            "InvalidParameterRange",
            "OutOfRange",
            "StyleNotFound",
            "TrackNotFound"
        ]
        .into_iter()
        .collect()
    );
}

#[test]
fn the_lane_kind_table_is_probed_without_a_wildcard() {
    assert_eq!(LANE_NAMES.len(), 5);
    let mut names = BTreeSet::new();
    for kind in [
        LaneKind::TrackVolume,
        LaneKind::TrackPan,
        LaneKind::SendGain,
        LaneKind::DeviceParam,
        LaneKind::Macro,
    ] {
        let name = lane_kind_name(kind);
        assert!(names.insert(name), "泳道名重复: {kind:?}");
        assert_eq!(kind.as_str(), name);
        assert!(LANE_NAMES.contains(&name), "`{name}` 必须在 LANE_NAMES 里");
        // 每个名字都能被 `parse` 认回来（表与解析器同一份真相）。
        assert_eq!(LaneKind::parse(name), Some(kind));
    }
    assert_eq!(names.len(), LANE_NAMES.len());
    assert_eq!(names, LANE_NAMES.into_iter().collect());
}

#[test]
fn the_curve_table_is_probed_without_a_wildcard() {
    assert_eq!(CURVE_NAMES.len(), 4);
    let mut names = BTreeSet::new();
    for curve in [
        CurveType::Linear,
        CurveType::Exponential,
        CurveType::Logarithmic,
        CurveType::SCurve,
    ] {
        assert!(names.insert(curve_name(curve)), "曲线名重复: {curve:?}");
        // 表里的名字必须与模型 serde 的名字**逐字**相同（跨层同一份真相）。
        assert_eq!(
            serde_json::to_value(curve).expect("枚举序列化"),
            serde_json::json!(curve_name(curve))
        );
    }
    assert_eq!(names, CURVE_NAMES.into_iter().collect());
}

#[test]
fn the_write_mode_table_is_probed_without_a_wildcard() {
    assert_eq!(LANE_WRITE_MODES.len(), 4);
    let mut names = BTreeSet::new();
    for mode in [
        AutomationWriteMode::Off,
        AutomationWriteMode::Write,
        AutomationWriteMode::Touch,
        AutomationWriteMode::Latch,
    ] {
        assert!(
            names.insert(write_mode_name(mode)),
            "写模式名重复: {mode:?}"
        );
        assert_eq!(
            serde_json::to_value(mode).expect("枚举序列化"),
            serde_json::json!(write_mode_name(mode))
        );
    }
    assert_eq!(names, LANE_WRITE_MODES.into_iter().collect());
}

#[test]
fn the_device_kind_table_is_probed_without_a_wildcard() {
    assert_eq!(DEVICE_KINDS.len(), 4);
    let mut names = BTreeSet::new();
    for kind in [
        DeviceKind::InternalInstrument,
        DeviceKind::InternalEffect,
        DeviceKind::ExternalInstrument,
        DeviceKind::ExternalEffect,
    ] {
        assert!(
            names.insert(device_kind_name(kind)),
            "设备类型名重复: {kind:?}"
        );
        assert_eq!(
            serde_json::to_value(kind).expect("枚举序列化"),
            serde_json::json!(device_kind_name(kind))
        );
    }
    assert_eq!(names, DEVICE_KINDS.into_iter().collect());
}

#[test]
fn the_small_local_tables_are_probed_without_a_wildcard() {
    assert_eq!(StaticLane::NAMES.len(), 2);
    let statics: BTreeSet<&str> = [StaticLane::TrackVolume, StaticLane::TrackPan]
        .into_iter()
        .map(static_lane_name)
        .collect();
    assert_eq!(statics, StaticLane::NAMES.into_iter().collect());
    assert_eq!(TrackFlag::NAMES.len(), 2);
    let flags: BTreeSet<&str> = [TrackFlag::Mute, TrackFlag::Solo]
        .into_iter()
        .map(track_flag_kind)
        .collect();
    assert_eq!(flags, TrackFlag::NAMES.into_iter().collect());
}
