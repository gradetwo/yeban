//! `yeban_*` 意图工具的注册表 [MCP-TOOL-001..010 + `ADR-0001` D45/D46 及后续的九条扩展, ROAD-M4-003]。
//!
//! ## 权威契约
//!
//! - **工具名集合 / `dryRun` / `idempotencyKey`**：`schemas/mcp-tools.schema.json`
//!   的 `definitions.ToolCall`；
//! - **每个工具的参数概要、领域语义、错误码**：`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §7.2
//!   的 `MCP-TOOL-001..010` 表格。
//!
//! 这两份契约在"错误码"上**不一致**（见 [`ErrorCode`] 的说明），本模块的处置是
//! "两份都实现、把差异钉成常量、让判据把它暴露出来"，而不是悄悄选一边。
//!
//! ## 二十四个工具 = 规范表格的十个 + 十四条**扩展**（ADR-0001 D45 / D46 + 后续线）
//!
//! 扩展段的**唯一清单**是 [`EXTENSION_NAMES`]：D45 的撤销 / 重做 2 条、D46 的三类能力 3 条、
//! D56 的诊断包 1 条、SMF 导出 1 条、SMF 导入 1 条、SFZ 装载与 SFZ 音色导出 2 条、录音导出 1 条、
//! 物理 MIDI 输入探针 1 条、未合并提案清单 1 条（`#176`）、场景启动 1 条（`#290`）= **14** 条。
//!
//! `ADR-0001` **D46** 明文："十工具是**起点不是上限**；扩充时必须同步
//! `schemas/mcp-tools.schema.json`（契约是唯一权威定义）"。**D45** 则要求撤销入口
//! "UI+MCP 两侧同接，共用同一实现"。于是本注册表多出十四条扩展：
//!
//! | 工具 | 作用 | 规范 ID |
//! | :--- | :--- | :--- |
//! | `yeban_undo` | 撤销最近 N 步（默认 1），走 [`crate::undo_session`] 的**唯一**实现 | `MCP-TOOL-EXT-UNDO` |
//! | `yeban_redo` | 重做最近 N 步（默认 1），同一个实现 | `MCP-TOOL-EXT-REDO` |
//! | `yeban_edit_automation` | 自动化泳道读（唯一求值入口）+ 写一个点 | `MCP-TOOL-EXT-AUTOMATION` |
//! | `yeban_query_engine_state` | 设备链 + 引擎/会话读数（只读） | `MCP-TOOL-EXT-ENGINE-STATE` |
//! | `yeban_import_audio` | 音频素材登记成 `clip_pool` 条目（`yeban-decode`） | `MCP-TOOL-EXT-IMPORT-AUDIO` |
//! | `yeban_export_diagnostics` | 诊断包采集（zip） | `MCP-TOOL-EXT-DIAGNOSTICS` |
//! | `yeban_export_midi` | 共享 SMF 映射导出（base64 回传，只读） | `MCP-TOOL-EXT-EXPORT-MIDI` |
//! | `yeban_import_midi` | 导入 SMF 文件到空 MIDI 音轨/片段并上报损失 | `MCP-TOOL-EXT-IMPORT-MIDI` |
//! | `yeban_load_sfz` | 装载 SFZ 乐器进会话 registry（读盘 + 解析 + 区域摘要） | `MCP-TOOL-EXT-SFZ-LOAD` |
//! | `yeban_render_sfz_wav` | 用已装载的 SFZ 区域把活跃工程的 MIDI 音符导出成 24-bit WAV | `MCP-TOOL-EXT-SFZ-WAV` |
//! | `yeban_record_wav` | `ARCH-REC-001` 三段（无锁环 + 后台抽干）录一段测试音成 24-bit WAV | `MCP-TOOL-EXT-RECORD-WAV` |
//! | `yeban_midi_input_probe` | 物理 MIDI 输入端口的一段事件窗口（`ADR-0006`，feature `midi-input` 默认关） | `MCP-TOOL-EXT-MIDI-INPUT` |
//! | `yeban_list_proposals` | **未合并**的提案清单（只读；`#176`，`#174` 坑②的发现面） | `MCP-TOOL-EXT-LIST-PROPOSALS` |
//! | `yeban_launch_scene` | 把一次**量化场景启动**交给宿主（`#290`；引擎由形态 A 的 app 经 attach 口接上） | `MCP-TOOL-EXT-SCENE-LAUNCH` |
//!
//! ### 为什么它们的 `specId` 不是 `MCP-TOOL-011/012`
//!
//! `AGENTS.md` §4.1 立过先例（`MODEL-AST-006` 在规范里缺号 ⇒ **不得凭空发明编号**）：
//! 四份规范里 `MCP-TOOL-` 族**只有** `001..010`，而本线**无权**改 `docs/YEBAN_*.md`。
//! 因此扩展工具带一个**形态上就不是编号**的 ID（`MCP-TOOL-EXT-*`，没有三位数字后缀），
//! 并在判据里钉住"规范族恰好是 001..010、扩展 ID 不得伪装成编号"。
//!
//! ## 为什么注册表是 `const` 数组而不是 `HashMap`
//!
//! 红线 4（`MODEL-AST-003` 的确定性精神）：工具集是**契约**，不是运行时数据。
//! `const` 数组的顺序即规范顺序（`MCP-TOOL-001` … `MCP-TOOL-010`，扩展追加在后），
//! `tools/list` 的输出因此逐字节稳定，跨进程、跨重启可对账。
//!
//! ## 与契约逐条对账的判据
//!
//! [`crate::tools`] 自带的单元判据 + `tests/contract.rs` 里的跨语言判据一共断言：
//!
//! - 工具名集合与 `definitions.ToolCall.properties.name.enum` **完全相等**（双向包含 + 计数）；
//! - 每个工具都声明 `dryRun` 与 `idempotencyKey`；
//! - 契约里的错误码全部被 [`ErrorCode`] 覆盖；
//! - `MCP-TOOL-001..010` 顺序与规范表格一致，扩展追在其后。
//!
//! 新增或漏掉一个工具，上述判据立刻变红。

use std::fmt;

use serde_json::{Map, Value};

use crate::security::Scope;

/// 每个工具都必须支持的"只读模拟"参数名。
pub const DRY_RUN_PARAM: &str = "dryRun";

/// 每个工具都必须支持的"幂等重放"参数名。
pub const IDEMPOTENCY_KEY_PARAM: &str = "idempotencyKey";

/// 三个提案类工具（`yeban_propose_section` / `yeban_edit_notes` / `yeban_set_macro`）
/// 的可选实参：是否在响应里回传**完整 op 载荷** `[BASELINE-006]`。
///
/// - 缺省 `false` ⇒ 响应只带**结构化字段**（`willCreate` + 提案摘要的
///   `opCount` / `opKinds`），完整 `Op` 载荷留在隔离分支与 op 日志里；
/// - 给 `true` ⇒ 响应额外回传逐条 `ops`（审查用途），代价是载荷变大。
///
/// 为什么缺省是 `false`：规范 `[BASELINE-006]` 对"生成 16 小节段落的完整 MCP
/// 工具往返载荷"的判据是**序列化 JSON 载荷 ≤ 4 KB** 且**结构化字段传输**。
/// 本 crate 实测（`tests/payload_budget.rs`）：
///
/// | 16 小节 `yeban_propose_section` 往返 | 请求 | 响应 | 合计 |
/// | :--- | ---: | ---: | ---: |
/// | 回传完整 op 载荷（旧默认） | 214 | 9,456 | **9,670** |
/// | 只回结构化字段（现默认） | 196 | 2,747 | **2,943** |
///
/// ⚠ 这条实参的缺省值**改变**了本 crate 早先的默认响应形状（那时
/// `data.proposal.ops` 恒在）。判定依据是上面那条规范判据，不是偏好；
/// 完整记账见 `src/payload.rs` 的文件头与 `tests/payload_budget.rs`。
pub const INCLUDE_OPS_PARAM: &str = "includeOps";

/// 工具集规模（规范表格的 10 个 + `ADR-0001` D45/D46 的扩展）。
pub const TOOL_COUNT: usize = 24;

/// **规范表格**里的工具数（`MCP-TOOL-001..010`）。
pub const DOCUMENTED_TOOL_COUNT: usize = 10;

/// 扩展工具数（`MCP-TOOL-EXT-*`）：D45 的撤销入口 2 条 + D46 的三类能力 3 条
/// + D56 的诊断包 1 条 + SMF 导出 1 条 + SMF 导入 1 条 + SFZ 装载 / SFZ 音色导出 2 条
/// + 录音导出 1 条 + **物理 MIDI 输入探针 1 条**（`ADR-0006`，`midi-input` 默认关）
/// + **未合并提案清单 1 条**（`#176`，`#174` 坑②）+ **场景启动 1 条**（`#290`）。
pub const EXTENSION_TOOL_COUNT: usize = 14;

/// 扩展工具的规范 ID 前缀。
///
/// ⚠ 刻意**不带**三位数字后缀：`MCP-TOOL-011` 会**伪装成**规范编号，而四份规范里
/// `MCP-TOOL-` 族只有 `001..010`（`AGENTS.md` §4.1 的 `MODEL-AST-006` 先例：
/// 不得凭空发明编号）。
pub const EXTENSION_SPEC_ID_PREFIX: &str = "MCP-TOOL-EXT-";

/// 扩展工具名，**注册顺序**（D45 的撤销入口在前，D46 的三类能力在后）。
///
/// 这是扩展段的**唯一清单**：判据、契约对账与样本导出都从它派生，
/// 因此"注册表里多了一条但清单没跟上"会立刻变红。
pub const EXTENSION_NAMES: [&str; EXTENSION_TOOL_COUNT] = [
    "yeban_undo",
    "yeban_redo",
    "yeban_edit_automation",
    "yeban_query_engine_state",
    "yeban_import_audio",
    "yeban_export_diagnostics",
    "yeban_export_midi",
    "yeban_import_midi",
    "yeban_load_sfz",
    "yeban_render_sfz_wav",
    "yeban_record_wav",
    "yeban_midi_input_probe",
    "yeban_list_proposals",
    "yeban_launch_scene",
];

/// 规范 ID 前缀。
pub const SPEC_ID_PREFIX: &str = "MCP-TOOL-";

/// 契约里的错误码 [MCP-TOOL-001..010]。
///
/// # 契约缺口与它的裁决（ADR-0001 D25）
///
/// 本线第一轮测出来的事实是：schema 的 `ToolResponse.error.code` 只列了 **7** 个值，
/// 而架构 §7.2 的表格逐工具列出的并集是 **16** 个，交集只有 3 个
/// （`PROJECT_LOCKED` / `IO_ERROR` / `PROPOSAL_NOT_FOUND`）—— 也就是 **13 个领域错误码
/// 没有家**，任何一个真实的领域失败都会产出被本 schema 判为非法的 `ToolResponse`。
///
/// 集成者按 **ADR-0001 D25** 把契约修成**联集 20 值**（原有 7 个一个不删，
/// 规范并集一个不缺），缺口因此关闭。本模块随之把 [`ErrorCode::SCHEMA_CONTRACT`]
/// 对齐成那 20 个，判据从"钉住缺口"升级成"实现集合 **==** 契约集合"。
/// 历史缺口清单保留在 `docs/ledger/mcp-core-notes.md` §3.1（作为方法论留痕）。
///
/// 联集里 schema 原有而规范表格未列的 4 个（[`ErrorCode::SCHEMA_ONLY`]）：
/// `ROUTING_CYCLE_DETECTED` / `ENTITY_NOT_FOUND` / `INVALID_PARAMETER_RANGE` /
/// `PERMISSION_DENIED`。其中 `PERMISSION_DENIED` 是 scope 强制的必需码
/// （[`crate::security::Denial`] 的语义），不是冗余。
///
/// [`ErrorCode::NotImplemented`] 是**实现级**错误码，不在任何契约 enum 里：
/// [`crate::dispatch`] 对"尚未接线"这类状况走 JSON-RPC `-32005`，
/// 绝不伪造一个契约里没有的 `ToolResponse.error.code`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ErrorCode {
    // ---- 架构 §7.2 的 16 个领域错误码（规范顺序） ----
    /// `PROJECT_LOCKED`：`.yeban.lock` 已被其他进程独占。
    ProjectLocked,
    /// `FILE_NOT_FOUND`：目标工程文件不存在。
    FileNotFound,
    /// `IO_ERROR`：落盘 / 读盘失败。
    IoError,
    /// `DISK_FULL`：磁盘写满。
    DiskFull,
    /// `NO_ACTIVE_PROJECT`：当前没有活跃工程。
    NoActiveProject,
    /// `INVALID_FIELD_SELECTOR`：稀疏视图的字段选择器非法。
    InvalidFieldSelector,
    /// `STYLE_NOT_FOUND`：风格预设不存在。
    StyleNotFound,
    /// `CYCLE_DETECTED`：声部连接产生环路。
    CycleDetected,
    /// `CLIP_NOT_FOUND`：片段不存在。
    ClipNotFound,
    /// `OUT_OF_RANGE`：音域 / 发声数越界。
    OutOfRange,
    /// `TRACK_NOT_FOUND`：音轨不存在。
    TrackNotFound,
    /// `INDEX_OUT_OF_BOUNDS`：宏索引越界。
    IndexOutOfBounds,
    /// `RENDER_FAILED`：离线渲染失败。
    RenderFailed,
    /// `BUSY`：引擎正忙（已有渲染在跑）。
    Busy,
    /// `PROPOSAL_NOT_FOUND`：提案分支不存在。
    ProposalNotFound,
    /// `CONFLICT`：合并冲突。
    Conflict,
    // ---- 4 个 schema 原有、架构 §7.2 表格未列的错误码（ADR-0001 D25 裁决保留） ----
    /// `ROUTING_CYCLE_DETECTED`：schema 里对 `CYCLE_DETECTED` 的另一种写法（同名异形）。
    RoutingCycleDetected,
    /// `ENTITY_NOT_FOUND`：schema 的通用"实体不存在"。
    EntityNotFound,
    /// `INVALID_PARAMETER_RANGE`：schema 的通用"参数越界"。
    InvalidParameterRange,
    /// `PERMISSION_DENIED`：schema 的通用"权限不足"（运行期权限，不是 token/scope）。
    PermissionDenied,
    // ---- 实现级（**不在任何契约 enum 里**） ----
    /// `NOT_IMPLEMENTED`：分发链路已就绪，领域实现尚未接线。
    NotImplemented,
}

impl ErrorCode {
    /// 全部错误码（规范顺序 + schema 独有 + 实现级），共 21 个。
    pub const ALL: [Self; 21] = [
        Self::ProjectLocked,
        Self::FileNotFound,
        Self::IoError,
        Self::DiskFull,
        Self::NoActiveProject,
        Self::InvalidFieldSelector,
        Self::StyleNotFound,
        Self::CycleDetected,
        Self::ClipNotFound,
        Self::OutOfRange,
        Self::TrackNotFound,
        Self::IndexOutOfBounds,
        Self::RenderFailed,
        Self::Busy,
        Self::ProposalNotFound,
        Self::Conflict,
        Self::RoutingCycleDetected,
        Self::EntityNotFound,
        Self::InvalidParameterRange,
        Self::PermissionDenied,
        Self::NotImplemented,
    ];

    /// `schemas/mcp-tools.schema.json` 的 `ToolResponse.error.code` enum
    /// （**联集 20 值**，按 schema 里的字母序；ADR-0001 D25）。
    ///
    /// 判据 `tests/contract.rs::implementation_error_codes_equal_the_contract_enum_exactly`
    /// 要求本常量与契约文件里的 enum **集合完全相等**（双向包含 + 计数）。
    pub const SCHEMA_CONTRACT: [Self; 20] = [
        Self::Busy,
        Self::ClipNotFound,
        Self::Conflict,
        Self::CycleDetected,
        Self::DiskFull,
        Self::EntityNotFound,
        Self::FileNotFound,
        Self::IndexOutOfBounds,
        Self::InvalidFieldSelector,
        Self::InvalidParameterRange,
        Self::IoError,
        Self::NoActiveProject,
        Self::OutOfRange,
        Self::PermissionDenied,
        Self::ProjectLocked,
        Self::ProposalNotFound,
        Self::RenderFailed,
        Self::RoutingCycleDetected,
        Self::StyleNotFound,
        Self::TrackNotFound,
    ];

    /// 契约里的 20 个减去规范表格的 16 个 = **4 个 schema 原有码**
    /// （架构 §7.2 表格未列，但 ADR-0001 D25 裁决保留）。
    pub const SCHEMA_ONLY: [Self; 4] = [
        Self::RoutingCycleDetected,
        Self::EntityNotFound,
        Self::InvalidParameterRange,
        Self::PermissionDenied,
    ];

    /// 架构 §7.2 表格里的 16 个领域错误码（表格顺序）。
    pub const DOCUMENTED_TOOL_CODES: [Self; 16] = [
        Self::ProjectLocked,
        Self::FileNotFound,
        Self::IoError,
        Self::DiskFull,
        Self::NoActiveProject,
        Self::InvalidFieldSelector,
        Self::StyleNotFound,
        Self::CycleDetected,
        Self::ClipNotFound,
        Self::OutOfRange,
        Self::TrackNotFound,
        Self::IndexOutOfBounds,
        Self::RenderFailed,
        Self::Busy,
        Self::ProposalNotFound,
        Self::Conflict,
    ];

    /// 规范字符串。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProjectLocked => "PROJECT_LOCKED",
            Self::FileNotFound => "FILE_NOT_FOUND",
            Self::IoError => "IO_ERROR",
            Self::DiskFull => "DISK_FULL",
            Self::NoActiveProject => "NO_ACTIVE_PROJECT",
            Self::InvalidFieldSelector => "INVALID_FIELD_SELECTOR",
            Self::StyleNotFound => "STYLE_NOT_FOUND",
            Self::CycleDetected => "CYCLE_DETECTED",
            Self::ClipNotFound => "CLIP_NOT_FOUND",
            Self::OutOfRange => "OUT_OF_RANGE",
            Self::TrackNotFound => "TRACK_NOT_FOUND",
            Self::IndexOutOfBounds => "INDEX_OUT_OF_BOUNDS",
            Self::RenderFailed => "RENDER_FAILED",
            Self::Busy => "BUSY",
            Self::ProposalNotFound => "PROPOSAL_NOT_FOUND",
            Self::Conflict => "CONFLICT",
            Self::RoutingCycleDetected => "ROUTING_CYCLE_DETECTED",
            Self::EntityNotFound => "ENTITY_NOT_FOUND",
            Self::InvalidParameterRange => "INVALID_PARAMETER_RANGE",
            Self::PermissionDenied => "PERMISSION_DENIED",
            Self::NotImplemented => "NOT_IMPLEMENTED",
        }
    }

    /// `true` 表示该错误码出现在 schema 的 `ToolResponse.error.code` enum 里。
    #[must_use]
    pub fn is_schema_contract(self) -> bool {
        Self::SCHEMA_CONTRACT.contains(&self)
    }

    /// `true` 表示该错误码在架构 §7.2 的工具表格里被列出。
    #[must_use]
    pub fn is_documented_tool_code(self) -> bool {
        Self::DOCUMENTED_TOOL_CODES.contains(&self)
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 工具的副作用等级（决定 `dryRun` 必须短路掉什么）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideEffect {
    /// 只读：`dryRun` 与真调用在状态上等价（仍然会做参数校验）。
    ReadOnly,
    /// 改变内存中的权威工程状态（Ops Log / 提案分支 / 宏）。
    ProjectState,
    /// 写盘：工程文件、CAS 资产或渲染产物。
    Disk,
}

impl SideEffect {
    /// 规范字符串。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::ProjectState => "project-state",
            Self::Disk => "disk",
        }
    }

    /// `true` 表示 `dryRun` 必须真的短路掉副作用。
    #[must_use]
    pub const fn is_side_effecting(self) -> bool {
        !matches!(self, Self::ReadOnly)
    }
}

/// 一个参数的类型与必填性。
#[derive(Clone, Copy, Debug)]
pub struct ParamSpec {
    /// JSON 属性名（`camelCase`，与规范表格一致）。
    pub name: &'static str,
    /// JSON 类型：`boolean` / `string` / `integer` / `number` / `array` / `object`。
    pub json_type: &'static str,
    /// 是否必填。
    pub required: bool,
    /// `minLength`（**Unicode 码点**计数，JSON Schema 语义）；`None` = 不约束。
    pub min_len: Option<usize>,
    /// `maxLength`（**Unicode 码点**计数，JSON Schema 语义）；`None` = 不约束。
    pub max_len: Option<usize>,
    /// `array` 参数的 item schema 构造器；`None` = 不广播 `items`。
    ///
    /// 用函数指针而不是 `Value`：`ParamSpec` 必须保持 `Copy` + `const`
    /// （注册表是 `const TOOLS`），而 `serde_json::Value` 不能 const 构造。
    /// 目前只有 `yeban_edit_notes` 的 `ops` 用它（[`crate::op_schema::ops_items`]）。
    pub items: Option<fn() -> Value>,
    /// 规范语义。
    pub doc: &'static str,
}

impl ParamSpec {
    /// 该 JSON 值是否符合本参数声明的类型。
    #[must_use]
    pub fn accepts(&self, value: &Value) -> bool {
        match self.json_type {
            "boolean" => value.is_boolean(),
            "string" => value.is_string(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "number" => value.is_number(),
            "array" => value.is_array(),
            "object" => value.is_object(),
            _ => false,
        }
    }
}

/// 所有工具共有的两个参数 [MCP-TOOL-001..010, 架构 §7.2 前言]。
pub const COMMON_PARAMS: [ParamSpec; 2] = [
    ParamSpec {
        name: DRY_RUN_PARAM,
        json_type: "boolean",
        required: false,
        min_len: None,
        max_len: None,
        items: None,
        doc: "只读模拟校验: 只做参数与领域合法性校验, 不改任何状态",
    },
    ParamSpec {
        name: IDEMPOTENCY_KEY_PARAM,
        json_type: "string",
        required: false,
        // ⭐ D46 收口（实现侧）：契约 `schemas/mcp-tools.schema.json` 写着
        // `maxLength: 256` ⇒ 工具面**必须真的拒**，否则"契约说有界、实现无界"
        // 就是 R142 的"静默永不生效"。以 Unicode 码点计数（JSON Schema 语义）。
        min_len: None,
        max_len: Some(256),
        items: None,
        doc: "幂等重放键: 同一个非空键的第二次调用不执行工具, 直接返回首次结果的缓存; 该响应是 `{\"replayed\":true,\"response\":…}` 信封 (契约 `definitions.ReplayedToolResponse`), 而不是裸 `ToolResponse`; 空串按未提供处理",
    },
];

/// 一个工具的全部规范信息。
#[derive(Clone, Copy, Debug)]
pub struct ToolSpec {
    /// 规范 ID（`MCP-TOOL-001` … `MCP-TOOL-010`）。
    pub spec_id: &'static str,
    /// 工具名（`yeban_*`）。
    pub name: &'static str,
    /// 领域语义一句话。
    pub summary: &'static str,
    /// 调用该工具需要的作用域 [ARCH-SEC-002]。
    pub scope: Scope,
    /// 副作用等级。
    pub side_effect: SideEffect,
    /// 工具**特有**参数（公共的 `dryRun` / `idempotencyKey` 由 [`ToolSpec::all_params`] 追加）。
    pub params: &'static [ParamSpec],
    /// 规范表格里列出的错误码。
    pub errors: &'static [ErrorCode],
}

impl ToolSpec {
    /// 公共参数 + 工具特有参数（顺序稳定：公共在前）。
    #[must_use]
    pub fn all_params(&self) -> Vec<&'static ParamSpec> {
        COMMON_PARAMS.iter().chain(self.params.iter()).collect()
    }

    /// 按名查找参数。
    #[must_use]
    pub fn param(&self, name: &str) -> Option<&'static ParamSpec> {
        self.all_params().into_iter().find(|spec| spec.name == name)
    }

    /// 必填参数清单。
    #[must_use]
    pub fn required_params(&self) -> Vec<&'static ParamSpec> {
        self.all_params()
            .into_iter()
            .filter(|spec| spec.required)
            .collect()
    }

    /// 校验实参（**纯函数**，`dryRun` 与真调用共用同一条校验）。
    ///
    /// 规则：
    ///
    /// 1. 必填参数必须存在且类型正确；
    /// 2. 可选参数若存在，类型必须正确；
    /// 3. 未知参数被拒绝（`_` 前缀的 MCP 保留键除外）—— 静默忽略拼错的参数
    ///    会让 Agent 以为自己的意图生效了。
    ///
    /// # Errors
    ///
    /// 违反上述任一条。
    pub fn validate_arguments(&self, arguments: &Map<String, Value>) -> Result<(), ToolCallError> {
        let specs = self.all_params();
        for spec in &specs {
            match arguments.get(spec.name) {
                Some(value) => {
                    if !spec.accepts(value) {
                        return Err(ToolCallError::InvalidParam {
                            name: spec.name.to_owned(),
                            expected: spec.json_type,
                        });
                    }
                    // ⭐ D46：长度约束必须**真的执行**（契约里写着就必须成立）。
                    if spec.json_type == "string"
                        && let Some(text) = value.as_str()
                    {
                        let length = text.chars().count();
                        if spec.min_len.is_some_and(|min| length < min)
                            || spec.max_len.is_some_and(|max| length > max)
                        {
                            return Err(ToolCallError::ParamLengthOutOfRange {
                                name: spec.name.to_owned(),
                                min: spec.min_len,
                                max: spec.max_len,
                            });
                        }
                    }
                }
                None if spec.required => {
                    return Err(ToolCallError::MissingParam {
                        name: spec.name.to_owned(),
                    });
                }
                None => {}
            }
        }
        for key in arguments.keys() {
            if key.starts_with('_') {
                continue;
            }
            if !specs.iter().any(|spec| spec.name == key) {
                return Err(ToolCallError::UnknownParam { name: key.clone() });
            }
        }
        Ok(())
    }
}

/// 参数概要的简写构造（保持注册表可读）。
const fn param(
    name: &'static str,
    json_type: &'static str,
    required: bool,
    doc: &'static str,
) -> ParamSpec {
    ParamSpec {
        name,
        json_type,
        required,
        min_len: None,
        max_len: None,
        items: None,
        doc,
    }
}

/// 带**长度约束**的参数（`minLength` / `maxLength`，Unicode 码点计数）。
///
/// 为什么需要它：契约（`schemas/mcp-tools.schema.json`）是唯一权威定义，
/// 但它自己**不会**执行任何东西 —— 实现侧不照着做，约束就是 R142 说的
/// "静默永不生效"（第五批实测：schema 改完，64 KiB 的幂等键**照样**被接受）。
const fn bounded_param(
    name: &'static str,
    json_type: &'static str,
    required: bool,
    doc: &'static str,
    min_len: Option<usize>,
    max_len: Option<usize>,
) -> ParamSpec {
    ParamSpec {
        name,
        json_type,
        required,
        min_len,
        max_len,
        items: None,
        doc,
    }
}

/// `array` 参数，带一个**广播给 `tools/list` 的 item schema**。
///
/// 为什么需要它（#156）：`yeban_edit_notes` 的 `ops` 此前只声明
/// `{"type": "array"}` ⇒ 外部 Agent 只能靠一段长 description 猜 op 形状与
/// 效果器参数名。item schema 由 [`crate::op_schema::ops_items`] 生成
/// （每个 kind 一个 `oneOf` 分支，`insertDevice.device.params.items` 里是
/// 效果器参数契约）。
const fn array_param(
    name: &'static str,
    required: bool,
    doc: &'static str,
    items: fn() -> Value,
) -> ParamSpec {
    ParamSpec {
        name,
        json_type: "array",
        required,
        min_len: None,
        max_len: None,
        items: Some(items),
        doc,
    }
}

/// 二十三个工具，**规范顺序**（`MCP-TOOL-001..010` 在前，D45/D46 及后续的扩展在后）。
pub const TOOLS: [ToolSpec; TOOL_COUNT] = [
    ToolSpec {
        spec_id: "MCP-TOOL-001",
        name: "yeban_open_project",
        summary: "校验排他锁并打开指定 `.yeban` 工程; `create:true` 时从零建一个可渲染的工程 (主总线在路由图里)",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ProjectState,
        params: &[
            param("path", "string", true, "工程文件路径"),
            param("readOnly", "boolean", false, "以只读方式打开 (不取写锁)"),
            // `create` 与它的三个伙伴是本线新增的**可选**实参（缺省 = 逐字节等于旧行为）。
            //
            // 为什么扩 `yeban_open_project` 而不新增工具：建工程需要的三样东西
            // （路径、`.yeban.lock` 协议、"打开后成为活跃工程"）这个工具已经全有；
            // 新增工具会动 `TOOL_COUNT`、契约的 `name.enum` 与台账的计数
            // （`ADR-0001` D46：先扩参数，确实不合适才新增）。
            //
            // ⚠ 主总线身份**不是**任何 `Op` 的载荷（模型 `Op` 全集没有写
            // `master_bus_track_id` 的变体，见 `domain/project_create.rs` 模块头的逐条证据），
            // 因此模板文档由本工具定型，内容改动仍走 `Op`。
            param(
                "create",
                "boolean",
                false,
                "从零新建工程 (默认 false = 打开已存在的容器)。目标路径已存在 ⇒ `CONFLICT`, 绝不静默覆盖; 与 `readOnly:true` 同给 ⇒ `INVALID_PARAMETER_RANGE`; 不给它时再给 `title`/`bpm`/`timeSignature`/`seed` 里任何一个 ⇒ `INVALID_PARAMETER_RANGE` (`createOnlyParameter`, 绝不静默丢弃)",
            ),
            param(
                "title",
                "string",
                false,
                "`create:true` 时的工程标题 (缺省 `Untitled`)",
            ),
            param(
                "bpm",
                "number",
                false,
                "`create:true` 时的速度 (BPM, 模型区间 20..=999; 缺省 120)",
            ),
            // `timeSignature` 是**可选**实参（缺省 = 模型默认 `4/4` = 逐字节等于旧行为）。
            //
            // 为什么必须有一条通路：`YebanProjectV1::time_signature` 是模型层一等字段，
            // 且已经被四个地方消费 —— 本 crate 的 `section_build::ticks_per_bar`
            // (配器段落长度)、`export_midi` (SMF 的拍号元事件)、`engine_state` (会话读数)、
            // 以及宿主的 小节/时间码 投影。但模型 `Op` 全集里**没有**写它的变体，
            // 因此模板层 (本工具) 是唯一的落点：接线之前，工具面建出来的工程恒为 `4/4`,
            // 也就是说 `propose_section` 的 `bars`、SMF 的拍号与界面时间码都按一个
            // 谁也设不了的值走。
            param(
                "timeSignature",
                "string",
                false,
                "`create:true` 时的拍号, \"N/D\" 文本 (例 \"3/4\"、\"6/8\"; 缺省 4/4)。取值由模型层 `TimeSignature::validate` 判决: 分子 1..=32, 分母 ∈ {1,2,4,8,16,32}; 形状错误 ⇒ `INVALID_PARAMETER_RANGE`, 取值越界 ⇒ `OUT_OF_RANGE`。种子摆放的时值与 `yeban_propose_section` 的 `bars` 都按这个拍号算; 不给 `create` 时给它是 `INVALID_PARAMETER_RANGE` (`createOnlyParameter`)",
            ),
            param(
                "seed",
                "object",
                false,
                "`create:true` 时的最小内容: `{trackCount: 1..=8, clipName: string, notes: [{pitch: 0..=127, durationTicks: >=1}]}`。缺省 = 1 条 MIDI 轨 + 4 个种子音符 + 一小节摆放 (因此新建的工程立即可 `render_master`, 且 `propose_section` 有 MIDI 材料可用)",
            ),
        ],
        // `CONFLICT` 是 `ADR-0001` D25 的 20 值联集与 §7.2 表格里都有的既有码,
        // 用于"目标路径已存在, 拒绝新建覆盖"。**没有**发明新错误码。
        errors: &[
            ErrorCode::ProjectLocked,
            ErrorCode::FileNotFound,
            ErrorCode::Conflict,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-002",
        name: "yeban_save_project",
        summary: "将内存权威状态与 CAS 资产原子刷盘",
        scope: Scope::AppSave,
        side_effect: SideEffect::Disk,
        params: &[param("force", "boolean", false, "忽略未保存标记强制落盘")],
        errors: &[ErrorCode::IoError, ErrorCode::DiskFull],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-003",
        name: "yeban_close_project",
        summary: "保存并释放当前工程与 `.yeban.lock`",
        scope: Scope::AppSave,
        side_effect: SideEffect::ProjectState,
        params: &[param("saveFirst", "boolean", false, "关闭前先保存")],
        errors: &[ErrorCode::NoActiveProject],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-004",
        name: "yeban_query_project",
        summary: "分页拉取工程稀疏视图, 防止海量音符撑爆 Agent 上下文",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ReadOnly,
        params: &[
            param("limit", "integer", false, "分页大小"),
            param("offset", "integer", false, "分页偏移"),
            // ⭐ `#177` 坑⑩：`fields` 此前只声明 `{"type":"array"}` ⇒ 合法字段名**无从
            // 得知**（只能靠猜，猜错一律 `INVALID_FIELD_SELECTOR`）。`items` 的 `enum`
            // 直接引用解析器的白名单 `crate::domain::view::SELECTORS`（⛔ 不手抄第二份
            // 清单），因此"白名单增删了而广播没跟上"在结构上不可能发生。
            array_param(
                "fields",
                false,
                "字段选择器 (稀疏视图): 元素必须**恰好**是 `items.enum` 里的一个名字 (白名单 = 解析器的 `SELECTORS`; 不在里面 ⇒ `INVALID_FIELD_SELECTOR`, 带 `availableSelectors`)。去重、保序; 缺省 = 全量顶层视图",
                crate::domain::view::selector_items,
            ),
        ],
        errors: &[ErrorCode::InvalidFieldSelector],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-005",
        name: "yeban_propose_section",
        summary: "在隔离分支 `ai/proposal-{ulid}` 创建章节配器骨架与声部连接 (材料来源: 工程片段池或乐理生成)",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ProjectState,
        params: &[
            param("sectionName", "string", true, "章节名"),
            param("stylePreset", "string", true, "风格预设"),
            param("bars", "integer", true, "小节数"),
            param("scale", "string", false, "音阶"),
            param(
                "materialSource",
                "string",
                false,
                "材料来源 [ADR-0016]: \"project\" (缺省) = 从 `clip_pool` 的可用 MIDI 材料装配; \"theory\" = 由 `yeban-theory` 的走向展开 + 声部连接生成 3/4 声部骨架 (空 `clip_pool` 也可用; 给了它就不再读片段池)。其余取值 ⇒ `INVALID_PARAMETER_RANGE`",
            ),
            param(
                "progression",
                "string",
                false,
                "自定义罗马数字走向 (例 \"i-VI-III-VII\"), 只被 `materialSource:\"theory\"` 消费; 缺省 = 该流派登记的首选走向, 流派没有登记时兜底 \"I-V-vi-IV\"。解析失败或声部连接无可行解 ⇒ `INVALID_PARAMETER_RANGE` (带 `theoryStep`)",
            ),
            param(
                "includeDrums",
                "boolean",
                false,
                "是否追加鼓组声部 [ADR-0017]: 缺省 false。true = 在段落末尾追加 `\"{sectionName} · Drums\"` 音轨, 鼓点由 `GenreRule::drum_pattern` (或 `swung_styled_drum_pattern`) 生成, 映射为标准 GM 打击乐键 (底鼓 36 / 军鼓 38 / 踩镲 42 / 吊镲 51), 时值固定 120 tick, 重音 112 其余 96, 并直连主总线 (整图仍为 DAG)。只允许与 `materialSource:\"theory\"` 同给; 否则 ⇒ `INVALID_PARAMETER_RANGE` (带 `hint`)",
            ),
            param(
                "includeMelody",
                "boolean",
                false,
                "是否追加旋律声部 [ADR-0017]: 缺省 false。true = 在段落末尾追加 `\"{sectionName} · Melody\"` 音轨, 旋律由 `melody_over_chords` 在**当前段落展开的** `ChordSpan` 与流派节奏网格上生成, 音域 `MelodyConstraints::DEFAULT`, 起点/时值与网格对齐, 和弦音 112 其余 96, 并直连主总线 (整图仍为 DAG)。只允许与 `materialSource:\"theory\"` 同给; 否则 ⇒ `INVALID_PARAMETER_RANGE` (带 `hint`)",
            ),
            param(
                "drumDensity",
                "string",
                false,
                "鼓点密度档位 [ADR-0028]: 缺省 \"normal\" = 逐字节维持既有鼓组输出。\"sparse\" = 只保留 `GenreRule::drum_pattern` 里**登记为重音** (`accent`) 的击点 (不发明新节奏表); 该流派没有登记重音时 ⇒ `INVALID_PARAMETER_RANGE` (带 `hint`, 建议换 \"normal\" 或换流派)。只允许与 `includeDrums:true` 同给; 否则 ⇒ `INVALID_PARAMETER_RANGE` (带 `hint`)。其余取值 ⇒ `INVALID_PARAMETER_RANGE` (带 `accepted`)",
            ),
            param("dryRun", "boolean", false, "只读模拟校验"),
            param(
                INCLUDE_OPS_PARAM,
                "boolean",
                false,
                "是否回传完整 op 载荷: 缺省 false = 只回结构化字段 (willCreate + opCount/opKinds), true = 额外回传逐条 ops (审查用途) [BASELINE-006]",
            ),
        ],
        errors: &[ErrorCode::StyleNotFound, ErrorCode::CycleDetected],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-006",
        name: "yeban_edit_notes",
        summary: "⭐ **本调用永远写入隔离提案分支，必须 `yeban_merge_proposal` 才落到 main**（响应的 `data.pendingMerge` 恒为 `true`、`data.projectUnchanged` 恒为 `true` —— 成功只代表提案已创建, ⛔ 不代表已落盘）。⭐ 用 `yeban_list_proposals` 查未合并的提案（`proposalId` / `title` / `opCount` / `opKinds` / `createdAt`）；⭐ 效果器参数名拼错（与某个已识别拼写的编辑距离 ≤ 2）会在**解析期** `INVALID_PARAMETER_RANGE`（`unknownEffectParam`，带最近邻建议）；引擎不认但距离更远的未知名字照旧接受, 并在响应的 `data.warnings[]` 里逐条报出。⭐ `#178`：`insertDevice` 装到一条**没有任何可发声内容**的音轨上时, 同一条 `warnings[]` 会多一条 `code:\"deviceOnSilentTrack\"`（带 `trackId` / `track` / `placements` / `notes` / 逐字 `hint`），⛔ **不阻断**提案（装是合法的, 只是该设备对这条轨的渲染输出不会产生影响；渲染侧的同一读数在 `sources[].kind:\"silent\"`）。⭐ `G5`（审计 2026-10-27）: 内置**音色选择**的入口就是 `insertDevice` + `device.kind:\"InternalInstrument\"` —— 引擎在渲染时**真的求值**这些参数名: `osc1_wave` / `osc2_wave`（取值 = 波表下标 `0`=hollow / `1`=organ / `2`=vocal / `3`=metallic / `4`=glass, 越界钳到末表）、`osc1_level` / `osc1_detune_cents` / `osc2_level` / `osc2_detune_cents`、`cutoff_hz`(`cutoff`) / `resonance`(`res`) / `drive`, 以及鼓机键位 `kick_note` 等五个 `*_note` 与 `kick_*`/`snare_*`/`hihat_*`/`clap_*`/`master_level` 旋钮（权威判据: `yeban_engine::synth::is_consumed_instrument_param`）; **非采样器轨**上这些名字⛔ 不再触发 `deviceChainDsp` 登记（采样器轨的音源是 SFZ, 音源参数不生效 ⇒ 仍如实登记）。; ⭐ `ADR-0018`: 可与 `placement` 同给, 单次提案原子生成 `Op::AddClip` + `Op::AddClipPlacement`在指定片段执行音符增删改, 自动进行音域与发声数合法性校验; `create:true` 时用这个身份新建一条 MIDI 片段池条目 (材料创建); `ops[].kind:\"removeClip\"` 单独出现时把 `clipId` 这个**已有的**片段池条目取走 (`Op::RemoveClip`, 还有摆放引用它则报 `CONFLICT`); `placement.kind` 选择摆放编辑 (`add` 把已有片段摆到音轨时间轴上 / `move` 平移一条已有摆放 / `remove` 取走一条已有摆放); `ops[].kind` 的五个音轨级形态写 `trackId` 那条轨的静态混音值、通道条开关、自动化泳道属性与自动化点 (`setParam` 音量 / 声相 · `setTrackMute` · `setTrackSolo` 三者都可用 `ops[].trackId` 逐条点名另一条音轨, 单次提案即可批量跨轨静音/独奏/调音 · `setAutomationLane` 读开关 / 写模式 / 取值域 / 取走泳道 · `removeAutomationPoint` 取走一个点); `ops[].kind:\"setRoutingGain\"` 写 `edgeId` 那条路由边的静态增益 (`Op::SetRoutingGain`; `value:null` = 单位增益, 与 `0.0` 可区分); `ops[].kind:\"disconnectRouting\"` 把 `edgeId` 那条路由边**取走** (`Op::DisconnectRouting`; 撤销载荷 `previous_edge` 从当前文档读); `ops[].kind:\"removeRoutingNode\"` 把 `nodeId` 那个路由**节点**从 `routing_graph.nodes` 取走 (`Op::RemoveRoutingNode`; 载荷只有节点身份, 主总线节点被响亮拒绝); `ops[].kind:\"removeSection\"` 把 `sectionId` 那个**曲式段落**从工程里取走 (`Op::RemoveSection`, `sectionId` 是本操作自带的寻址; 撤销载荷 `previousSection` 从当前文档读, 段落不存在报 `ENTITY_NOT_FOUND` (reason `sectionNotFound`)); `ops[].kind:\"removeTrack\"` 把 `trackId` 那条**音轨本身**从工程里取走 (`Op::RemoveTrack`, `trackId` 是本操作自带的寻址, 顶层 `trackId`/`clipId` 对本形态无意义; 撤销载荷 `previousTrack` 从当前文档读 (整条音轨, 含它的摆放与自动化泳道), 音轨不存在报 `ENTITY_NOT_FOUND` (reason `trackNotFound`), 主总线被响亮拒绝 (reason `masterBusTrackCannotBeRemoved`); 提案里在它**之前**排上该轨在路由图里的逐条断边与移除节点 (`disconnectRouting` → `removeRoutingNode`, 顺序承重), 一次 `yeban_undo` 把边、节点与音轨一起复原; 本形态只能**单独**出现, 不能与别的 `kind` 或 `placement` 同给); `ops[].kind:\"removeScene\"` 把 `sceneId` 那个**场景**从工程里取走 (`Op::RemoveScene`, `sceneId` 是本操作自带的寻址; 撤销载荷 `previousScene` 从当前文档读, 场景不存在报 `ENTITY_NOT_FOUND` (reason `sceneNotFound`)); `ops[].kind:\"setScene\"` 写 `scene.sceneId` 那个**场景** (`Op::SetScene`; `scene.create:true` = 新建 (身份必须**不在**工程里, 否则 `CONFLICT` (reason `sceneAlreadyExists`), 且 `name` 必填), 缺省 = 更新 (身份必须**在**工程里, 否则 `ENTITY_NOT_FOUND` (reason `sceneNotFound`); 三个属性 `name`/`tempo`/`color` 按**合并**语义施加 (缺省 = 保留现值), 显式 `null` 清空 `tempo`/`color`); 撤销载荷 `oldScene` 从当前文档读 (新建时为空, 更新时是整条现值 → 一次 `yeban_undo` 就能把这个场景整条取走)",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ProjectState,
        params: &[
            param(
                "trackId",
                "string",
                true,
                "音轨 EntityId (26 字符 ULID); 音符 / 摆放 / 音轨属性级形态的**缺省**目标音轨 (`setParam` / `setTrackMute` / `setTrackSolo` 可用 `ops[].trackId` 逐条覆盖, 批内每条都自备 `trackId` 时顶层身份接受但**不读** —— 可给占位身份)。自带寻址的形态 (removeTrack / insertDevice / removeDevice) 不读它, 但仍按本工具既有入口口径要求它是合法身份",
            ),
            param(
                "clipId",
                "string",
                false,
                "片段 EntityId; ⭐ **按 `ops[].kind` 条件必需**（`#177` 坑④，此前是无条件必填）: 只有**读**它的形态需要 —— 六个音符形态 (`add` / `delete` / `move` / `velocity` / `setNoteTiming` / `setNoteExpression`)、池级 `removeClip`，以及 `placement` 在场 / `create:true` 时。其余形态 (`addTrack` / 设备 / 路由 / 段落 / 场景 / 资产 / 音轨属性 / 文档级) **不读**它: 省略 = 用占位身份 (一个字节都不会被读, 也不会查片段池)，给了 = **接受但忽略**。需要而缺 ⇒ `INVALID_PARAMETER_RANGE` (`reason: \"clipIdRequired\"`, 带 `requiredByKinds`); 不读它的形态即使给了非法 ULID 也不解析 (同样忽略)。逐 kind 的口径同时广播在 `ops.items` 每个分支的 `x-envelope.clipId` 与分支描述里。`create:true` 时它是**将要新建的**片段身份 (池里已有该身份 ⇒ `CONFLICT`)",
            ),
            array_param(
                "ops",
                true,
                "操作列表 (NoteOp): `{\"kind\":\"add\",\"note\":{id?,startTick,pitch,durationTicks,velocity?,channel?,probability?,ratchet?,microTimingTicks?,pitchBendCurve?,slide?,syllable?,phonemes?}}` / `delete` / `move` / `velocity` / `setNoteTiming` / `setNotePitchShape` / `setPitchBendPoint` / `insertDevice` / `removeDevice` / `setParam` / `setTrackMute` / `setTrackSolo` / `setTrackName` / `setClipName` / `setClipGain` / `setDeviceName` / `setDeviceBypassed` / `setClipPlacementMuted` / `setClipPlacementLoop` / `setTrackColor` / `setTrackSampler` / `setClipPlacementDuration` / `setTransportConfig` / `setTempo` / `setAuthor` / `setTitle` / `setTimeSignature` / `setKeySignature` / `setTempoPoint` / `setTimeSignaturePoint` / `setAutomationLane` / `removeAutomationPoint` / `removeClip` / `removeAsset` / `setAsset` / `setRoutingGain` / `disconnectRouting` / `connectRouting` / `removeRoutingNode` / `addRoutingNode` / `addTrack` / `removeSection` / `setSection` / `removeTrack` / `removeScene` / `setScene`。`note.channel` (可选, 整数 0..=15) 为 MIDI 通道 (缺省遵循所属轨道默认通道); `note.syllable` (可选, 字符串) 为歌词音节; `note.phonemes` (可选, 字符串数组, 缺省 = 空数组) 为该音符的音素序列 (每个音素必须是字符串)。`note.probability` (可选, 0.0..=1.0) 是**确定性**概率触发, 由 `MidiNote::triggers(rng_seed)` 裁决 [MODEL-AST-005]; `note.ratchet` (可选, 整数 1..=16) 的连击细分与 `note.microTimingTicks` (可选, 整数 -240..=240 tick) 的起点偏移都由母带渲染器**真的**消费 (与实时引擎同一条格点公式)。`note.pitchBendCurve` (可选) 与 `note.slide` (可选) 也已接线: `pitchBendCurve` 是 `[tick, cents]` 二元组数组 (tick 是**音符局部**坐标、严格升序; cents 相对 `pitch` 且闭区间 ±1200; 首点可为任意 tick ≥ 0, 形状/值域由模型 `MidiNote::validate` 把关, 见 `ADR-0012`), `slide` 是 `{durationTicks, targetPitch, curve?}` (`durationTicks` 必须非零、`targetPitch` 是 0..=127, 两者**必填**; `curve` 只认 `Linear`/`Exponential`/`Logarithmic`/`SCurve`, 缺省 `Linear`); 两者都由引擎与母带渲染器**真的求值** (同时存在时**相加**, `ADR-0012` D9), 母带里听得到。`note` 里这 13 个键之外的字段一律 `INVALID_PARAMETER_RANGE` (绝不静默丢弃): 模型的表现力字段 (`slide` / `pitchBendCurve` / `channel` / `syllable` / `phonemes`) 现已**全部**接上工具面通路 ⇒ 拒绝载荷里的 `unsupportedNoteFields` 是空集。⚠ `phonemes` 写进文档但本渲染器**没有歌声合成** —— 母带里听不到, 渲染器仍把「母带不会唱」如实登记为 `noteLyrics` (⛔ 不再登记 `noteSlide`/`notePitchBend` —— `#311b` 起它们真的被求值)。`{\"kind\":\"setNoteTiming\",\"noteId\":ULID,\"startTick\":N,\"durationTicks\":N}` 把**已有**音符的起点/时值设为**绝对值** (`Op::SetNoteTiming`): ⭐ 补的是 `move` 只按增量平移起点、时值**没有任何字段级通路**的缺口 —— 在它之前改时值只能删 + 加, 且要把音高/力度/概率/连击/微时序等表现力字段全部抄一遍 (漏抄一个就是静默丢失); 两个旧值由服务端从**当前文档**读 (陈旧载荷 ⇒ `CONFLICT`), `durationTicks` 必须 ≥ 1 (模型的 `ZeroDuration`), 操作对象只认 `kind`/`noteId`/`startTick`/`durationTicks` 四个键 (`unknownSetNoteTimingField`)。`{\"kind\":\"setNoteExpression\",\"noteId\":ULID,\"probability\":0.0..=1.0|null,\"ratchet\":1..=16|null,\"microTimingTicks\":-240..=240|null}` 把**已有**音符的三个表现力字段**整体替换**为绝对值 (`Op::SetNoteExpression`): ⭐ 补的是 `probability`/`ratchet`/`microTimingTicks` 只在 `add` 时能给的缺口 —— 在它之前改一个概率只能删 + 加, 而那会换掉音符身份 (`MidiNote::triggers` 的确定性哈希只用 `id`)。⚠ 三个键**全部必填**、`null` = 清空 = 模型的 `None` (必然触发/等价于 1/无偏移); ⛔ 缺键不等于 `null` (`missingField`)。三个旧值由服务端从**当前文档**读 (陈旧载荷 ⇒ `CONFLICT`), `probability` 的比较是**逐位**的; 值域照抄模型 (`0.0..=1.0` 且有限 / `1..=16` / `-240..=240`), 越界 ⇒ `OUT_OF_RANGE`。操作对象只认 `kind`/`noteId`/`probability`/`ratchet`/`microTimingTicks` 五个键 (`unknownSetNoteExpressionField`)。`{\"kind\":\"setNotePitchShape\",\"noteId\":ULID,\"newPitchBendCurve\":[[tick,cents],...],\"newSlide\":{...}|null}` 把**已有**音符的**音高形状**（弯音曲线 ＋ 滑音）**整体替换** (`Op::SetNotePitchShape`): ⭐ 补的是 `pitchBendCurve`/`slide` 只在 `add` 时能给的缺口 —— 在它之前改形状只能删 + 加, 而那会换掉音符身份 (`MidiNote::triggers` 的确定性哈希只用 `id`)。⚠ 载荷**只带新值**: `newPitchBendCurve` 与 `newSlide` **全部必填**; 旧值由服务端从**当前文档**读 (与 `setNoteExpression` 同一条纪律), 把 `oldPitchBendCurve`/`oldSlide` 写进载荷是**未知键**并响亮拒绝 (那正是 `oldProbability` 的待遇)。\"无弯音\"**只有一种拼法 — 空数组 `[]`** (模型的 `pitch_bend_curve` 是 `Vec`, ⛔ 无 `null`); `newSlide:null` = 清空滑音; 两者同时存在时由引擎/渲染器**相加** (`ADR-0012` D9), 改完的母带真的变调。值域照抄模型 (`SlideConfig::validate` + `MidiNote::validate`, 弯音 tick 升序先于 cents ±1200)。操作对象只认 `kind`/`noteId`/`newPitchBendCurve`/`newSlide` 四个键 (`unknownSetNotePitchShapeField`)。五个**音轨级**形态 (不读不写任何音符, 目标缺省是顶层 `trackId`; `setParam` / `setTrackMute` / `setTrackSolo` 三个形态可给可选的 `ops[].trackId` **逐条点名**另一条音轨, 批内每条都自备目标时顶层 `trackId` 接受但**不读**): `{\"kind\":\"setParam\",\"lane\":\"TrackVolume\"|\"TrackPan\",\"value\":number}` 写 `TrackV3::volume_db` / `pan` (`Op::SetParam`), `{\"kind\":\"setParam\",\"lane\":\"DeviceParam\",\"slotIndex\":N,\"paramIndex\":N,\"value\":number}` 写 `track.devices[slotIndex].params[paramIndex].value`; `lane` 只认这三个规范名 (`SendGain` 走 `setRoutingGain`, 与 `Macro`、未知名字一样都 `INVALID_PARAMETER_RANGE`), `DeviceParam` 的两个下标必填且越界时 `data.reason` 用 `slotOutOfRange`/`paramIndexOutOfRange`, 参数名不在离线渲染器消费的白名单里时 `paramNotSupported` (⛔ 不静默写入一个 DSP 不认的参数), 值域 (声相 -1.0..=1.0, 有限值) 由模型层 `validate_param_value` 判; `{\"kind\":\"setTrackMute\",\"value\":bool}` / `{\"kind\":\"setTrackSolo\",\"value\":bool}` 写 `TrackV3::mute` / `solo`; `{\"kind\":\"setTrackName\",\"trackId\":ULID,\"name\":\"<字符串>\"}` 写 `TrackV3::name` (`Op::SetTrackName`)：`name` **必填**但**可为空串** (模型的 `Op::SetTrackName` 只有「陈旧载荷」一条校验, `TrackV3::validate` **不查**空名 ⇒ 空串 = 显式清空名字; 与 `addTrack` 的 `name` 必须非空**不同口径**), 目标音轨必须在 `project.tracks` 里 (否则 `ENTITY_NOT_FOUND` / `trackNotFound`), 撤销载荷 `oldName` 由服务端从**当前文档**读 (`project.tracks[trackId].name`), 比较是**逐字节**的; ⭐ 本形态补的是「建轨后改不了名」—— 此前只能删轨重建, 那会换掉 `trackId` 并丢掉设备链/自动化/路由边; `setClipName` 写 `ClipPoolEntry::name` (`Op::SetClipName`)：载荷 `{\"kind\":\"setClipName\",\"clipId\":ULID,\"name\":\"<字符串>\"}`，`name` **必填**但**可为空串** (模型只有「陈旧载荷」一条校验, `ClipPoolEntry` 的校验**不查**空名 — 与 `addClip` 的必需标签不是同一条口径), 目标片段必须在 `project.clip_pool` 里 (否则 `CLIP_NOT_FOUND`, 模型不带 `data.reason`), 撤销载荷 `oldName` 由服务端从**当前文档**读 (`project.clip_pool[clipId].name`), 比较是**逐字节**的; ⭐ 本形态补的是「建片段后改不了名」—— 此前只能删 + 加, 而 `removeClip` 还要求先把该片段的每一处摆放拆掉; {\"kind\":\"setClipGain\",\"clipId\":ULID,\"gainDb\":number} 写**音频片段**的 `ClipContent::Audio::gain_db` (`Op::SetClipGain`)：⭐ 本形态补的是「音频片段建得出、增益改不了」—— `gain_db` 此前只在 `yeban_import_audio` 创建时由 `gainDb` 写一次。目标 `clipId` **在操作对象里** (⛔ 不读工具顶层 `clipId`)，且目标**必须是音频片段** (MIDI 片段 ⇒ `CLIP_NOT_FOUND`，文案逐字 \"不是音频片段, 没有 gain_db 可改\" —— 因为 MIDI 没有 `gain_db` 可读、撤销载荷 `old_gain_db` 无从取得)；撤销载荷 `old_gain_db` 由服务端从**当前文档**读 (⛔ 不信调用方)，比较是**逐位**的。⚠ `gainDb` 的**值域照抄现状**：模型只查有限性 (`clip.audio.gain_db`)，**没有** dB 上下限 ⇒ 收任意**有限** `f32`；非数字 ⇒ `valueMustBeNumber`，收窄到 `f32` 后非有限 (如 `1e300` ⇒ `inf`) ⇒ `nonFiniteValue`。操作对象只认 `kind`/`clipId`/`gainDb` 三个键 (`unknownSetClipGainField`)。⚠ 身份是条目自己的 `id` (即 `clip_pool` 的键) ⇒ 改 `name` **不动键**、不悬空任何摆放 (`Op::SetTrackMute` / `Op::SetTrackSolo`, 母带渲染器的 `audible` 判定真的读它们), `value` 只收 JSON 布尔 (不做真假值强转), 开关对象里 `kind`/`value`/`trackId` 之外的键一律 `INVALID_PARAMETER_RANGE` (`trackId` **可选**: 省略 = 目标是顶层 `trackId`, 给了 = 逐条点名另一条音轨); `{\"kind\":\"setTempo\",\"bpm\":number}` 写**工程速度** (`Op::SetTempo`): `bpm` **必填**、必须**有限**并在 **20.0..=999.0** 内 (越界或非有限 ⇒ `bpmOutOfRange`)。撤销载荷 `oldBpm` 由服务端从**当前文档**读 (`project.bpm`), ⛔ 不信调用方声明; 模型的比较是**逐位**的 (`-0.0` 与 `0.0` 位不同 ⇒ 陈旧载荷一律 `OpStateMismatch`)。⚠ **JSON 没有 NaN / Infinity 字面量** —— 它们在线上是 `null`, 因此这一类在**解析期**就报 `valueMustBeNumber`; 工具面**没有**非有限值的可表达入口, 这一半如实说清。操作对象只认 `kind`/`bpm` 两个键 (`unknownSetTempoField`)。 `{\"kind\":\"setAuthor\",\"author\":\"<字符串>\"}` 写**工程署名** (`Op::SetAuthor`): `author` **必填**, 且**照抄模型口径收任意字符串** —— 模型只有「陈旧载荷」这一条校验, 而 `project.schema.json` 的 `author` 是裸 `{\"type\":\"string\"}` (非必需、无 `minLength`、无 `pattern`) ⇒ ⛔ 本层**不发明**「非空」或长度上限。撤销载荷 `oldAuthor` 由服务端从**当前文档**读 (`project.author`), 比较是**逐字节**的; 操作对象只认 `kind`/`author` 两个键 (`unknownSetAuthorField`)。 `{\"kind\":\"setTitle\",\"title\":\"<字符串>\"}` 写**工程标题** (`Op::SetTitle`): `title` **必填**, 且**照抄模型口径收任意字符串** —— 模型只有「陈旧载荷」这一条校验, 而 `project.schema.json` 的 `title` 是裸 `{\"type\":\"string\"}` (**必需**、无 `minLength`、无 `pattern`) ⇒ ⛔ 本层**不发明**「非空」或长度上限。撤销载荷 `oldTitle` 由服务端从**当前文档**读 (`project.title`), 比较是**逐字节**的; 操作对象只认 `kind`/`title` 两个键 (`unknownSetTitleField`)。⭐ 本形态补的是「建得出、改不了」—— 此前 `project.title` 唯一的**结构写**点是 `yeban_create_project` (`domain/project_create.rs:370`)。 `setTimeSignature` 形态写**工程拍号** (`Op::SetTimeSignature`): 载荷是 `kind` + `timeSignature` 两个键, `timeSignature` **必填** —— 形状与 `yeban_open_project` 的 `create:true` 实参**逐字同词同义** (`N/D` 文本, 例 `3/4`、`6/8`), 且**复用同一个解析器** (`parse_time_signature`) ⇒ 接受集合逐点相同 (形状错误 ⇒ `INVALID_PARAMETER_RANGE`, 取值越界 ⇒ `OUT_OF_RANGE`; 值域的唯一判定入口是模型 `TimeSignature::validate`, ⛔ 本层不抄那张表)。⛔ 载荷**只带新值**: 旧值由服务端从**当前文档**读 (`project.time_signature`), 把 `oldTimeSignature` 写进载荷是**未知键**并响亮拒绝 (`unknownSetTimeSignatureField`)。⭐ 本形态补的是「建得出、改不了」—— 此前 `project.time_signature` 唯一的**结构写**点是 `yeban_open_project` 的 `timeSignature` 实参。⚠ 本渲染器是 **tick 基**的: 改拍号**不会**改变任何已渲染时值 (离线母带逐字节不变); 它驱动的是**栅格几何** —— 引擎节拍器的 `ticks_per_beat` (= `PPQ` × 4 ÷ 分母) 与 `beats_per_bar` (= 分子)、小节长度 (`ticks_per_bar`) 与 SMF 拍号元事件。 `setKeySignature` 形态写**工程调号** (`Op::SetKeySignature`): 载荷是 `kind` + `keySignature` 两个键, `keySignature` **必填** (可为 `null` 清空调号，或包含 `sharpsFlats` 整数 -7..=7 与 `isMinor` 布尔值的对象; 值域由模型 `KeySignature::validate` 判 ⇒ 越界报 `OUT_OF_RANGE`)。旧值由服务端从**当前文档**读 (`project.key_signature`), 把 `oldKeySignature` 写进载荷是**未知键**并响亮拒绝 (`unknownSetKeySignatureField`)。 `setTempoPoint` 形态写 `project.tempo_map` 里的一个**速度映射点** (`Op::SetTempoPoint`): 载荷是 `kind` + `tick` + `bpm` 三个键, `tick` **必填**且必须 **≥ 1** (tick 0 的时间点是**工程速度本身**, 归 `setTempo`; 模型对 `tempo_map[0]` 报 `TempoMapZeroTick` ⇒ 本层同理由 `tempoMapZeroTick` 拒)。⚠ `bpm` **必填** —— 数字 = 新增/修改该点 (区间与有限性**同 `setTempo` 一个口径**, `20.0..=999.0`, 越界 ⇒ `bpmOutOfRange`), `null` = **删除**该映射点; ⛔ **缺键不等于删除** (删除的唯一拼法是显式 `null`)。删除一个**不存在**的点 ⇒ `ENTITY_NOT_FOUND` (`tempoPointNotFound`, 带 `tick`)。撤销载荷 `old_bpm` 由服务端从**当前文档**读 (`project.tempo_map[tick]`, ⛔ 不信调用方), 模型的比较是**逐位**的。操作对象只认 `kind`/`tick`/`bpm` 三个键 (`unknownSetTempoPointField`)。⭐ 本形态补的是「速度**曲线**改不了」—— 在它之前 `project.tempo_map` 在工具面**没有任何**结构写点。 `setTimeSignaturePoint` 形态写 `project.time_signature_map` 里的一个**拍号映射点** (`Op::SetTimeSignaturePoint`): 载荷是 `kind` + `tick` + `timeSignature` 三个键, `tick` **必填**且必须 **≥ 1** (同上, `tempoMapZeroTick`); `timeSignature` **必填** —— `\"N/D\"` 文本与 `setTimeSignature` **逐字同词同义**且**复用同一个解析器** (`parse_time_signature`, 唯一值域判定 = 模型 `TimeSignature::validate`; 形状错误 ⇒ `INVALID_PARAMETER_RANGE`, 取值越界 ⇒ `OUT_OF_RANGE`), `null` = **删除**该映射点 (⛔ 缺键不等于删除, 显式 `null` 是唯一拼法)。删除一个**不存在**的点 ⇒ `ENTITY_NOT_FOUND` (`timeSignaturePointNotFound`, 带 `tick`)。撤销载荷 `old_time_signature` 由服务端从**当前文档**读 (`project.time_signature_map[tick]`)。操作对象只认 `kind`/`tick`/`timeSignature` 三个键 (`unknownSetTimeSignaturePointField`)。⭐ 本形态补的是「拍号**曲线**改不了」—— 在它之前 `project.time_signature_map` 在工具面**没有任何**结构写点。 ⛔ 两个映射点形态都**不触碰**顶层 `project.bpm` / `project.time_signature`; 与 `setTimeSignature` 同一条事实: 本渲染器是 **tick 基**的, 映射点驱动的是引擎节拍器的**栅格几何**与 SMF 元事件, ⛔ 不移动任何音符/摆放的 tick。 八个形态都是**顶层文档字段** (与走带配置同族): 不读不写音符, 也不寻址音轨。 `{\"kind\":\"setAutomationLane\",\"lane\":{...}}` 写**自动化泳道自己**的属性 (`Op::SetAutomationLane` / `Op::RemoveAutomationLane`): `lane.lane` 是目标变体名 (`TrackVolume`/`TrackPan`/`SendGain`+`edgeId`/`DeviceParam`+`slotIndex`+`paramIndex`/`Macro`+`macroIndex`, 与 `yeban_edit_automation` 同一份词表), 属性 `readEnabled` (bool) / `writeMode` (`Off`|`Write`|`Touch`|`Latch`) / `domain` (`{min,max}` 或 `null` 清掉) **各自可选** (缺省 = 保留文档现值, 是**合并**不是整体替换), `remove:true` 把整条泳道取走 (与属性键同给即 `INVALID_PARAMETER_RANGE`), 同一目标在一次调用里出现两次也 `INVALID_PARAMETER_RANGE`; `readEnabled:false` 会让唯一求值入口 `automation_value_at` 返回「无自动化值」⇒ 该泳道在走带与宿主求值里**不生效**; `{\"kind\":\"removeAutomationPoint\",\"point\":{...}}` 取走一个**自动化点** (`Op::RemoveAutomationPoint`): `point` 是自包含对象 —— 泳道寻址 (`lane` 与上一条同一份词表, `SendGain` 仍必须给 `edgeId`) 加**恰好一个**点寻址: `tick` (文档里**恰好一个**落在该 tick 上的点; 0 个 `ENTITY_NOT_FOUND`, 同 tick 多个 `INVALID_PARAMETER_RANGE` 并给出 `candidatePointIds`) 或 `pointId` (`yeban_edit_automation` 读侧 `lane.points[].id` 报的身份), 两个同给 (`pointAddressIsAmbiguous`) 或都不给 (`pointAddressRequired`) 都是 `INVALID_PARAMETER_RANGE`; `point` 里的泳道属性键 (`readEnabled`/`writeMode`/`domain`/`remove`) 与操作对象顶层的 `tick`/`pointId` 都被响亮拒绝 (寻址在 `point` 对象里); 撤销载荷 `previous_point` 从当前文档读; `{\"kind\":\"setAsset\",\"assetHash\":\"<SHA-256>\",\"license\":\"<许可>\"|\"originalPath\":\"<路径>\"}` **改**一条已存在的资产索引项 (`Op::SetAsset`, `old_asset` 从当前文档读; ⭐ 补的是「建得出、改不了」)。⚠ **只可改两个字段**: `license` 与 `originalPath` —— ⛔ `hash`/`byteLen`/`mediaKind` **不可改** (它们是**内容身份或内容事实**; 给它们会 `unknownSetAssetField`)。⭐ 新 `license` **同样过步 ③ 的许可判定** (白名单外 ⇒ `licenseNotAllowed`) ⇒ 本通路⛔ **不绕过**它。至少给一个可改字段 (否则 `assetPatchIsEmpty`)。该哈希**在** `project.assets` 里 (否则 `assetNotFound`)。⚠ 它与 `yeban_import_audio` 的 `assetDeclaredDifferently` 冲突**分工不同**: 那个拒绝「靠重导**静默**改写许可留痕」, 本形态给出一条**正门** (用户明说要改)。 `{\"kind\":\"removeAsset\",\"assetHash\":\"<SHA-256>\"}` 取走**工程 assets 索引里的一条资产** (`Op::RemoveAsset`, 撤销载荷 `previousAsset` 从当前文档读)。⭐ 它是资产**唯一的显式删除通路** —— 在它之前 `Op::RemoveAsset` 只由模型的结构逆产出 (用户只能靠撤销那次导入取走索引项)。⚠ **自带寻址** (本工具没有顶层 `assetHash` 实参), 操作对象只认 `kind`/`assetHash` 两个键 (`unknownRemoveAssetField`)。两条前置: 该哈希**在** `project.assets` 里 (否则 `ENTITY_NOT_FOUND` / `assetNotFound`); ⚠ **没有任何 `clip_pool` 条目引用它** (否则 `CONFLICT` / `assetInUse`, 带 `clipCount`/`clipIds`) —— 与 `removeClip` 要求先取走摆放是同一条顺序纪律 (先取走引用它的片段, 再取走这条索引项)。⚠ 它**只**删工程里的索引留痕, ⛔ **不碰** CAS 池里的字节 (字节住在容器层, `needs-2` 仍开着)。 `{\"kind\":\"removeClip\"}` 取走**顶层 `clipId` 那条已有的片段池条目** (`Op::RemoveClip`, 撤销载荷 `previous_clip` 从当前文档读) —— 这是唯一的**池级**形态, 操作对象**只认 `kind` 一个键** (嵌套 `clipId` 等一律 `INVALID_PARAMETER_RANGE` / `unknownRemoveClipField`), 且必须**单独**出现在 `ops` 里 (`removeClipTakesNoOtherOps`), 与 `placement` 同给即 `removeClipIsNotPlacement`; 池里没有该身份 `CLIP_NOT_FOUND`, 还有**任何**摆放引用它则 `CONFLICT` (模型自己的 `ClipInUse`), 因此顺序是「先取走摆放, 再取走材料」; 它**不要求片段是 MIDI** (音频池条目同样取得走), 但**不回收** CAS 资产字节 (模型 `Op` 全集没有资产变体)。`{\"kind\":\"setRoutingGain\",\"edgeId\":ULID,\"value\":数字|null}` 是唯一的**路由级**形态: 写 `edgeId` 那条**已有**路由边的静态增益 (`Op::SetRoutingGain`), 撤销载荷 `old_gain_db` 从当前文档读**原样**的 `Option<f32>` (不经过会把 `None` 折算成 `0.0` 的静态值入口, 因为模型的 `same_gain` 是逐位比较)。`value` 是 `Option<f32>` 的字面形态: `null` = `None` = **单位增益**, 数字 = `Some` (收窄到 `f32` 后必须有限, 否则 `nonFiniteValue`); 缺 `value` 或给了布尔/字符串/对象都是 `INVALID_PARAMETER_RANGE`; 操作对象只认 `kind`/`edgeId`/`value` 三个键 (`unknownRoutingGainField`, 嵌套的 `trackId`/`gainDb` 不会被读取); 工程里没有该边 ⇒ `ENTITY_NOT_FOUND` (`routingEdgeNotFound`); 它与顶层 `trackId`/`clipId` 都无关, 目标由自带的 `edgeId` 给出。`{\"kind\":\"connectRouting\",\"edgeId\":ULID,\"sourceNode\":ULID,\"destinationNode\":ULID,\"edgeKind\":\"TrackToBus\"|\"BusToMaster\"|\"SendToAux\"|\"Sidechain\",\"gainDb\":数字|null}` 是**建立路由边**形态 (`Op::ConnectRouting`): 把一条边加进 `routing_graph.edges`。它是 `disconnectRouting` 的**逆**, 也是**路由图可用性的最后一环** —— 在它之前工具面**建得起节点、连不起边** (边只在 `yeban_create_project` 与 `yeban_propose_section` 里发生)。⚠ **平铺** (⛔ 不嵌套 `edge: {...}` 对象), 与 `addRoutingNode`/`disconnectRouting` 同一形状。`edgeId`/`sourceNode`/`destinationNode`/`edgeKind` **四个必填**; `gainDb` **可选** —— ⛔ 与 `setRoutingGain` 的 `value` **同口径** (不造第二套): `null` = `None` = **单位增益**, 数字 = `Some` (收窄到 `f32` 后必须有限, 否则 `nonFiniteValue`), **整键省略与 `null` 同义** (模型 `RoutingEdge::gain_db` 带 `#[serde(default)]`)。模型的三条前置条件: 两端都必须在 `routing_graph.nodes` 里 (`routingNodeNotFound` —— 先用 `addRoutingNode` 把节点加进图), `edgeId` 不在 `edges` 里 (`routingEdgeAlreadyExists`, **幂等拒绝**), 增益有限。⚠ 模型**没有**「节点类型与 `edgeKind` 必须匹配」这条约束 ⇒ 工具面**也不**发明它。操作对象只认那六个键 (`unknownConnectRoutingField`)。 `{\"kind\":\"disconnectRouting\",\"edgeId\":ULID}` 是**另一个**路由级形态: 把 `edgeId` 那条**已有**路由边**取走** (`Op::DisconnectRouting`), 撤销载荷 `previous_edge` 从当前文档读**整条**边 (`RoutingEdge` 逐字段比较, 因此调用方声明的旧状态一律不被采信)。它的载荷是**空**的: 操作对象只认 `kind`/`edgeId` 两个键 (`unknownDisconnectRoutingField`, 嵌套的 `trackId`/`value`/`gainDb` 都不会被读取); 工程里没有该边 ⇒ `ENTITY_NOT_FOUND` (`routingEdgeNotFound`); `routing_graph.nodes` 一个字都不动 (取走**节点**是**另一个**形态 `removeRoutingNode` 的事, 它要求没有任何边引用它 —— 先断开每一条引用边, 再单独一次调用取走节点)。`{\"kind\":\"addRoutingNode\",\"nodeId\":ULID}` 是**第四个**路由级形态 (`Op::AddRoutingNode`): 把 `nodeId` 那个身份**加进** `routing_graph.nodes`。它与 `removeRoutingNode` 严格**互为逆操作**, 因此载荷也**是空的**: 操作对象只认 `kind`/`nodeId` 两个键 (`unknownAddRoutingNodeField`), 嵌套的 `edgeId`/`trackId` 都不会被读取 (节点与边是两种实体)。⚠ 这个身份**必须不在** `nodes` 里, 否则 `CONFLICT` (`routingNodeAlreadyExists`) —— 本形态是**幂等拒绝**的。⚠ 它**不要求**这个身份是一条真实音轨: 模型只做数据层校验 (`RoutingGraph::validate` 查节点重复、以及每条边的两端都在 `nodes` 里), **不**要求节点能对应到 `tracks` 的条目 (同族先例: `addTrack` 只把音轨加进 `tracks`, **不**自动把身份加进 `nodes`)。**它是「让一条新音轨成为路由节点」的唯一通路** —— 在此之前工具面只有 `disconnectRouting`/`setRoutingGain`/`removeRoutingNode`, **没有**任何形态能**建立**一条路由边或**加入**一个节点 (那条通路此前只在 `yeban_propose_section` 的建批里)。 `{\"kind\":\"removeRoutingNode\",\"nodeId\":ULID}` 是**第三个**路由级形态: 把 `nodeId` 那个**已有**路由节点从 `routing_graph.nodes` **取走** (`Op::RemoveRoutingNode`)。它的载荷是**空**的: 模型 `RemoveRoutingNode` 只有节点身份, 而且它与 `AddRoutingNode` 互为逆操作 ⇒ **没有撤销载荷要读** (前两个路由级形态的撤销载荷都从当前文档读, 本形态不需要也不接受调用方声明旧状态)。操作对象只认 `kind`/`nodeId` 两个键 (`unknownRemoveRoutingNodeField`, 嵌套的 `trackId`/`edgeId`/`value` 都不会被读取 —— `edgeId` 是**边**的寻址, 节点与边是两种实体); `routing_graph.nodes` 里没有该身份 ⇒ `ENTITY_NOT_FOUND` (`routingNodeNotFound`); 这个身份是**主总线** ⇒ `CONFLICT` (`masterBusNodeCannotBeRemoved`: 模型要求有音轨时主总线留在 `nodes` 里, 那是 `yeban-engine` 的 `PdcPlan::compute` 找得到唯一声学出口的前提); 还有**任何**路由边引用它 ⇒ `CONFLICT` (模型自己的 `RoutingNodeInUse`, 带 `edge_count`)。`{\"kind\":\"setSection\",\"section\":{...}}` 是**段落级写入**形态 (`Op::SetSection`): 载荷是**自包含**的 `section` 对象 (只认 `create`/`sectionId`/`name`/`startTick`/`endTick`/`color` 六个键, 其余一律 `INVALID_PARAMETER_RANGE` / `unknownSetSectionField`; 操作对象顶层只认 `kind` 与 `section`, 嵌套的 `sectionId` 放在顶层是 `unknownSetSectionOpField`)。它是 `removeSection` 的**对偶**: 在它之前工具面**删得掉段落、⛔ 改不了也加不了**, 而 `Op::SetSection` 的**唯一**构造点是 `yeban_propose_section` 的内部建批 ⇒ 一个**已存在的**工程无法改段落名/区间/色标, 也无法逐个小段落地加段落。`sectionId` (ULID) **必填**; `section.create:true` = **新建**: 该身份必须**不在** `project.sections` 里 (否则 `CONFLICT` / `sectionAlreadyExists`), 且 `name` **必填** (模型的 `SectionV3::name` 没有校验, 空名会静默落盘 ⇒ `sectionNameRequiredWhenCreating`); 缺省 = **更新**: 该身份必须**在**工程里 (否则 `ENTITY_NOT_FOUND` / `sectionNotFound`)。四个属性**各自可选**、按**合并**语义施加 (缺省 = 保留文档现值, 不是整体替换): `name` (非空字符串), `startTick`/`endTick` (非负整数, 否则 `sectionTickMustBeUnsignedInteger`), `color` (非空字符串或 `null` = 清空)。⚠ 区间约束 (`endTick > startTick`) 与色标/名字一样**不在这里**复制: 模型 `SectionV3::validate` 是唯一事实源 (`ZeroDuration`)。撤销载荷 `oldSection` 由服务端从**当前文档**读 (新建时为空 ⇒ 逆操作是 `RemoveSection`), 因此一次合并不可能把段落改成调用方声明的样子。 `{\"kind\":\"removeSection\",\"sectionId\":ULID}` 是**唯一的段落级**形态 (`Op::RemoveSection`): 把 `sectionId` 那个**已有的**曲式段落从 `project.sections` **取走**。它的载荷是**空**的: 操作对象只认 `kind`/`sectionId` 两个键 (`unknownRemoveSectionField`, 嵌套的 `trackId`/`nodeId`/`edgeId`/`clipId` 都不会被读取 —— 段落与音轨/节点/边/片段是四种不同的实体); 撤销载荷 `previousSection` 由服务端从**当前文档**读**整条**段落 (`SectionV3` 逐字段比较, 因此调用方声明的旧状态一律不被采信); 工程里没有该段落 ⇒ `ENTITY_NOT_FOUND` (`sectionNotFound`, 段落身份由 `yeban_query_project` 的 `entities[]` 里 `kind == \"section\"` 的条目报出)。它不读不写任何音符 (因此不要求片段是 MIDI), 也不碰音轨、片段池与路由图; 与 `removeClip` 一样, 顶层 `trackId`/`clipId` 仍是本工具签名要求的必填实参, 但它们不是本形态的目标。`{\"kind\":\"addTrack\",\"trackId\":ULID,\"name\":\"<非空>\",\"trackKind\":\"Midi\"|\"Audio\"|\"AuxReturn\"|\"Master\"}` 是**新增音轨**形态 (`Op::AddTrack`): 把一条**空形状**的音轨加进 `project.tracks`。它补的是「删得到、加不到」的不对称 —— 加音轨此前只在 `yeban_create_project` 与 `yeban_propose_section` 里发生, 所以一个**已存在的**工程无法被逐步扩展。⚠⭐ **四个集合 (`devices`/`macros`/`automationLanes`/`clips`) 与 `folderId`/`color`/五个混音标量 (`volumeDb`/`pan`/`mute`/`solo`/`soloSafe`) 都**不在**载荷里** —— 那是**决定**, 理由 = **新实体无内容可清**: `ADR-0001 D43` 的必需性立法理由是「缺键 = 静默清空**既有**内容」, 而创建时没有既有内容 (同族先例: `project_create.rs:381` 的 `TrackV3 { id, name, kind, ..Default::default() }` 与 `setScene` 的 create 分支)。因此新建空轨**不会**改动混音 (`volume_db = 0.0`、`pan = 0.0`、三个开关 `false`); 操作对象只认 `kind`/`trackId`/`name`/`trackKind` 四个键 (`unknownAddTrackField`), 多写一个集合或混音标量都被响亮拒绝。`trackId` 已在 `tracks` 里 ⇒ `CONFLICT` (`trackAlreadyExists`, **幂等拒绝**); `name` 必须非空 (`valueMustBeNonEmptyString` —— 模型的 `TrackV3::validate` 不查空名); `trackKind` 必须是四个规范名之一 (`unknownTrackKind`)。⚠ 它**不**自动把身份加进 `routing_graph.nodes`、也**不**建边 —— 那要另调 `addRoutingNode`。 `{\"kind\":\"removeTrack\",\"trackId\":ULID}` 是**唯一的音轨实体级**形态 (`Op::RemoveTrack`): 把 `trackId` 那条**已有的**音轨**本身**从 `project.tracks` 取走 (连带它自己的 `clips` 摆放与 `automation_lanes` —— 那是音轨的子结构)。它的载荷是**空**的: 操作对象只认 `kind`/`trackId` 两个键 (`unknownRemoveTrackField`, 嵌套的 `clipId`/`sceneId`/`sectionId`/`nodeId`/`edgeId` 都不会被读取 —— 音轨与片段/场景/段落/节点/边是五种不同的实体); 撤销载荷 `previousTrack` 由服务端从**当前文档**读**整条**音轨 (`TrackV3` 逐字段比较, 因此调用方声明的旧状态一律不被采信); 工程里没有该音轨 ⇒ `ENTITY_NOT_FOUND` (`trackNotFound`, 音轨身份由 `yeban_query_project` 的 `tracks` 字段报出); 该身份是**主总线** ⇒ `CONFLICT` (`masterBusTrackCannotBeRemoved`: 模型要求有音轨时主总线留在 `tracks` 里且 `kind == Master`, 那也让 `master_bus_track_id` 不悬空)。⚠ 它与工具顶层 `trackId` **不是同一个东西**: 后者是「目标片段所在的音轨」(本工具的签名要求它必填), 本形态的寻址在操作对象**自带**的 `trackId` 上 —— 因此 `removeTrack` 单独出现时顶层 `trackId`/`clipId` 两个实参的内容不被读取。本形态必须**单独**出现在 `ops` 里 (`removeTrackTakesNoOtherOps`), 与 `placement` 同给即 `removeTrackIsNotPlacement` (摘掉一条音轨与在它上面摆片段是同一批里互相矛盾的两件事)。它不读不写任何音符 (因此不要求片段是 MIDI), 也不碰片段池 (池里的材料留着, 可能仍被别的音轨引用)。⭐ 它**连带**清掉该轨在路由图里的边与节点: 提案里排在 `RemoveTrack` **之前**的是每一条端点命中该轨的边的 `Op::DisconnectRouting` (撤销载荷 `previous_edge` 从当前文档读) 与 `Op::RemoveRoutingNode` —— 留着孤儿节点会让 `yeban_render_master` **整体失败** (G2 / `ADR-0023`)。该轨不在图里时不产生这些 op。`{\"kind\":\"removeScene\",\"sceneId\":ULID}` 是**唯一的场景级**形态 (`Op::RemoveScene`): 把 `sceneId` 那个**已有的**场景从 `project.scenes` **取走**。它的载荷是**空**的: 操作对象只认 `kind`/`sceneId` 两个键 (`unknownRemoveSceneField`, 嵌套的 `trackId`/`sectionId`/`nodeId`/`edgeId`/`clipId` 都不会被读取 —— 场景与段落/音轨/节点/边/片段是五种不同的实体); 撤销载荷 `previousScene` 由服务端从**当前文档**读**整条**场景 (`SceneV3` 逐字段比较, 因此调用方声明的旧状态一律不被采信); 工程里没有该场景 ⇒ `ENTITY_NOT_FOUND` (`sceneNotFound`, 场景身份由 `yeban_query_project` 的 `entities[]` 里 `kind == \"scene\"` 的条目报出)。⚠ 与段落形态不同, 写侧此前**一处都没有**: `Op::SetScene` 与 `Op::RemoveScene` 在 MCP 侧实测构造点都是 0 (同一模式对 `Op::SetSection` 读 2 ⇒ 模式有效), 因此本形态关的是场景实体上**无自由度**的那一半 (删除; 创建/更新要决定 `sceneId`/`name`/`tempo`/`color` 的形状, 留在它自己的一票)。它不读不写任何音符 (因此不要求片段是 MIDI), 也不碰音轨、片段池、路由图与曲式段落。`{\"kind\":\"setScene\",\"scene\":{...}}` 是**场景级写入**形态 (`Op::SetScene`): 载荷是**自包含**的 `scene` 对象 (只认 `create`/`sceneId`/`name`/`tempo`/`color` 五个键, 其余一律 `INVALID_PARAMETER_RANGE` / `unknownSetSceneField`; 操作对象顶层只认 `kind` 与 `scene`, 嵌套的 `sceneId` 放在顶层是 `unknownSetSceneOpField`)。`sceneId` (ULID) **必填** —— 本工具面的身份一律由调用方给出, 且 `ops[]` 信封没有回传「服务端替你生成的身份」的通道。`scene.create:true` = **新建**: 该身份必须**不在** `project.scenes` 里 (否则 `CONFLICT` / `sceneAlreadyExists`, 与 `create:true` 的 `clipAlreadyExists` 同口径), 且 `name` **必填** (模型的 `SceneV3::name` 没有校验, 空名会静默落盘); 缺省 = **更新**: 该身份必须**在**工程里 (否则 `ENTITY_NOT_FOUND` / `sceneNotFound`)。三个属性**各自可选**、按**合并**语义施加 (缺省 = 保留文档现值, 不是整体替换): `name` (非空字符串; 模型里不是可空字段, 给 `null` 响亮拒绝), `tempo` (数字或 `null`; `null` = 清空覆盖 = 跟随工程速度, 值域 20.0..=999.0 与有限性由模型 `SceneV3::validate` 判 → `OUT_OF_RANGE` / `INVALID_PARAMETER_RANGE`), `color` (非空字符串或 `null`; `null` = 清空)。撤销载荷 `oldScene` 由服务端从**当前文档**读 (新建时为空 ⇒ 逆操作是 `RemoveScene`; 更新时是整条现值), 因此一次合并不可能把场景改成调用方声明的样子。同一个场景在一次调用里出现两次 ⇒ `INVALID_PARAMETER_RANGE` (`duplicateSceneTarget`)。十三个形态都走与本工具音符编辑**相同**的提案路径; 除 `removeRoutingNode` 外, 其余形态的撤销载荷都从当前文档读 (`removeRoutingNode` **没有**撤销载荷 —— 它与 `AddRoutingNode` 互为逆操作)。`{\"kind\":\"setTrackSampler\",\"trackId\":ULID,\"sampler\":\"<路径>\"|null}` 给一条音轨**绑定采样器定义文件** (`Op::SetTrackSampler` ⇒ `TrackV3::sampler`): `sampler` **必填** —— 非空字符串 = 绑定 (路径**原样留痕**, ⛔ 不查存在性/不规范化/不查扩展名), `null` = **解绑** (缺键 ⛔ 不等于解绑); 撤销载荷 `oldSampler` 由服务端从当前文档读。⭐ `yeban_render_master` **真的**消费它 (与 app 的 `--track-sampler`/`--export-wav` **同一条音源路径**: 区域选择/播放速率/总增益/样本文件都走 `yeban-sfz` 的同一份读数), 并把逐轨读数 `sfz(tracks=1 regions=… samples=… silent=… missing=[…])` 写进 `data.sources[].sampler.label`; 路径读不到 ⇒ `RENDER_FAILED` / `samplerUnreadable` (⛔ 不静默回退正弦)。给了 `placement` 时允许空数组 (这次调用只摆放, 一个音符都不动) ⭐ **逐 kind 的 JSON Schema 见本参数的 `items`** (oneOf, 每个 `kind` 一个分支); EQ / 混响 / 卷积混响的参数名 / 单位 / 值域 / 缺省值见 `insertDevice` 分支的 `device.params.items` (名字大小写不敏感; ⛔ 不认 `reverb_mix` / `reverb_width`); 建发送的边形状见 `connectRouting` 的 `edgeKind:\"SendToAux\"`。⭐ `#177` 坑④: 顶层 `clipId` 只在**读**它的形态上是必需的 (五个音符形态 / 池级 `removeClip` / `placement` / `create:true`), 其余形态省略即可 (给了也接受但忽略); 逐 kind 的 `x-envelope.clipId` 见每个 `oneOf` 分支。",
                crate::op_schema::ops_items,
            ),
            // `create` / `clipName` 是**可选**实参（缺省 = 逐字节等于旧行为），
            // 与 `yeban_open_project` 的 `create` 同词同义（ADR-0001 D48）。
            //
            // 为什么扩本工具而不新增工具：`ADR-0001` D46 的扩张原则是"先扩既有工具的
            // 参数，只有确实不合适才新增工具"；新增工具要同步 `schemas/mcp-tools.schema.json`
            // 的 `name.enum` + `ExtensionToolArguments.$defs` + `allOf` 三处（本线禁改
            // `schemas/**`）。§7.2 的参数表因此**一字未动**；`clipId` 在 `#177` 之前是
            // 无条件必填，`#177` 把它改成**按 `ops[].kind` 条件必需**（见下面那条
            // `clipId` 的 `required: false` 与 `crate::domain::notes::ENVELOPE_CLIP_KINDS`）：
            // 只有五个音符形态 / 池级 `removeClip` / `placement` / `create:true` 读它。
            param(
                "create",
                "boolean",
                false,
                "用 `clipId` 新建一条 MIDI 片段池条目 (缺省 false = 编辑已存在的片段)。`create:true` 时 `ops` 只允许 `add` (它们成为新片段的初始内容): 池里已有该身份 ⇒ `CONFLICT`, 出现其他 `kind` ⇒ `INVALID_PARAMETER_RANGE`。⭐ `ADR-0018`: `create:true` 可与 `placement` 同给 —— 单次提案同时生成 `Op::AddClip` 与 `Op::AddClipPlacement`; 此时 `durationTicks` 缺省由新片段的全部音符推导",
            ),
            param(
                "clipName",
                "string",
                false,
                "`create:true` 时的片段名 (缺省 `Clip`; 只是给人看的标签, 不参与身份)",
            ),
            // `placement` 是**可选**实参（缺省 = 不摆放 = 逐字节等于旧行为）。
            // 它关闭 `docs/ledger/mcp-tools-expansion-notes.md` §6 的 needs-6：
            // 渲染器只遍历 `track.clips`，池子里没被摆放的片段一帧都不出声，
            // 而此前**没有任何工具**能把已有片段摆到音轨上（`create:true` 建出来的
            // MIDI 材料正是这种"在池子里但发不出声"的状态）。
            //
            // `kind`（缺省 `add`）让同一参数承载三种摆放编辑：`add` 之后，
            // `move` / `remove` 分别编译成 `Op::MoveClipPlacement` /
            // `Op::RemoveClipPlacement` —— 那两个模型变体与渲染器的出片路径
            // 早已存在，此前工具面够不着。
            param(
                "placement",
                "object",
                false,
                "摆放编辑, 三种形态由 `kind` 选择 (缺省 `add`): `add` = `{startTick?: 非负整数 (默认 0), durationTicks?: >=1 (默认 = 片段内容长度; 片段推不出长度时必填), placementId?: ULID (默认由 片段+音轨+起点 确定性派生), muted?: 布尔 (默认 false), loopEnabled?: 布尔 (默认 false), loopStartTick?: 非负整数 (默认 0), loopEndTick?: 非负整数 (启用时默认 = 片段内容长度)}` 把 `clipId` 摆到 `trackId` 上 (`Op::AddClipPlacement`); ⭐ `#241`: `loopEnabled: true` 且 `loopStartTick < loopEndTick < durationTicks` 时, 渲染器把循环区间**真的展开** N = ceil((durationTicks - loopStartTick) / (loopEndTick - loopStartTick)) 遍 (母带里真的有 N 遍音符/音频); 想重复一个 4 小节 loop 就把它摆成 N×4 小节并把 `loopEndTick` 设成 4 小节, ⛔ 不用手写 N 遍音符; `move` = `{kind:\"move\", placementId: ULID (必填), startTick: 非负整数 (必填, 新起点)}` 平移一条已有摆放 (`Op::MoveClipPlacement`, 现值从工程读取); `remove` = `{kind:\"remove\", placementId: ULID (必填)}` 取走一条已有摆放 (`Op::RemoveClipPlacement`, 撤销载荷从工程读取)。三者的键之外的键一律 `INVALID_PARAMETER_RANGE`; 形态不适用但别处合法的键报 `placementFieldNotApplicable` (不静默丢弃)。`move`/`remove` 的 `clipId` 必须等于文档里那条摆放引用的片段, 否则 `INVALID_PARAMETER_RANGE` (`placementClipMismatch`); 该音轨上没有这条摆放 ⇒ `ENTITY_NOT_FOUND`; `move` 到原起点 ⇒ `CONFLICT` (`placementAlreadyAtStartTick`)。`add` 时目标音轨上已有该摆放身份 ⇒ `CONFLICT` (内容相同的重放请用 `idempotencyKey`)。⭐ `ADR-0018`: 与 `create:true` 同给时, 单次提案原子生成 `Op::AddClip` + `Op::AddClipPlacement` (此时 `placement.kind` 只能是 `add`, `move`/`remove` 报 `INVALID_PARAMETER_RANGE` / `createPlacementKindMustBeAdd`)。",
            ),
            // ⭐ 第八批实测：这一条**遮蔽**了公共那份（`ParamSpec` 同名两份、界不同：
            // 公共 `(None, Some(256))` vs 本条 `(None, None)`）⇒ `input_schema()` 合并前
            // 会**丢掉约束**、而判据 `every_string_params_bound_status_is_registered`
            // 会报"同一参数名在不同工具里的界不一致"。这里补上与契约**逐值相同**的界。
            bounded_param(
                "idempotencyKey",
                "string",
                false,
                "幂等重放键: 同键第二次调用返回 `{\"replayed\":true,\"response\":…}` 信封 (契约 `definitions.ReplayedToolResponse`)",
                None,
                Some(256),
            ),
            param(
                INCLUDE_OPS_PARAM,
                "boolean",
                false,
                "是否回传完整 op 载荷: 缺省 false = 只回结构化字段 (willCreate + opCount/opKinds), true = 额外回传逐条 ops (审查用途) [BASELINE-006]",
            ),
        ],
        // `CONFLICT` 是 §7.2 表格 16 个里的既有码（`MCP-TOOL-001` 的 `create` 同款语义：
        // "目标已存在就响亮拒绝"，见 `domain/project_create.rs`）。`INVALID_PARAMETER_RANGE`
        // 是本工具**本来就**在产的 schema 独有码（`parse_ops` 的缺字段 / 未知 `kind`），
        // 按既有口径不列进文档工具的错误码表（那条表只收 §7.2 的 16 个）。
        errors: &[
            ErrorCode::ClipNotFound,
            ErrorCode::OutOfRange,
            ErrorCode::Conflict,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-007",
        name: "yeban_set_macro",
        summary: "调节乐器宏旋钮, 触发级联平滑自动化展开",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ProjectState,
        params: &[
            param("trackId", "string", true, "音轨 EntityId"),
            param("macroIndex", "integer", true, "宏索引"),
            param("value", "number", true, "目标值"),
            param(
                INCLUDE_OPS_PARAM,
                "boolean",
                false,
                "是否回传完整 op 载荷: 缺省 false = 只回结构化字段 (willCreate + opCount/opKinds), true = 额外回传逐条 ops (审查用途) [BASELINE-006]",
            ),
        ],
        errors: &[ErrorCode::TrackNotFound, ErrorCode::IndexOutOfBounds],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-008",
        name: "yeban_render_master",
        summary: "触发 `yeban-render` 多核并行导出广播级 WAV 并返回哈希与路径; 响应带实测削波读数: `data.clippedSamples` (会被 24-bit 量化器钳住的样本数, 整数) / `data.truePeakDbfs` (16x 多相过采样的真峰值, 0 = 满量程, >0 = 过载) / `data.headroomDb` (= -truePeakDbfs, 负值 = 已过载)。⭐ 削波时 `data.warnings[]` 里**恰好一条** `code:\"exportClipping\"`: 带 `clippedSamples` / `truePeakDbfs` / `suggestedTrimDb` (= -(ceil(truePeakDbfs * 2) / 2), 向上取整到 0.5 dB) / `loudestSource` (最响的源: `node` / `track` / `peakDbfs`) / `hint`; ⭐ 可闻、排了内容、却 `peak == 0.0` (源节点一个非零样本都没出) 的源另由**一条** `code:\"silentSources\"` 逐条点名 (`count` / `sources[]` / `hint`, 非阻断、不自动修); 两条来源都不出现时 `warnings` 是**空数组**。⛔ 本工具**不自动改增益** (那会静默改用户意图) —— 只是读数 + 建议; 逐源峰值见 `data.sources[].peak` / `.peakDbfs` (源节点输出, ⛔ 不含该轨插入链与路由边增益)。⚠ `data.peak.before` / `data.peak.after` 是**归一化前 / 归一化后的样本峰值** (`max |sample|`), 作用在 TPDF 抖动与量化**之前**的浮点母带上 —— ⛔ 不是写盘文件的峰值, 也⛔ 不是真峰值。⭐ `#252`: 可选 `frames` (整数) / `durationSeconds` (秒) 显式指定总长度 —— 缺省仍按工程末端推导 (与加这个实参之前逐字节相同), 用途是把本工具与 CLI `--export-wav --wav-frames` 的帧数**逐帧**对齐。⭐ `ADR-0020`: 可选 `contributions: true` 让响应额外带逐轨贡献读数 (`data.contributions` / `data.contributionsBasis`, 见该实参), ⛔ 不改写盘字节",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::Disk,
        params: &[
            param(
                "format",
                "string",
                true,
                "输出容器/编码 (wav / rf64 / bw64)",
            ),
            param(
                "sampleRate",
                "integer",
                true,
                "输出采样率 (Hz); 与工程采样率不一致时由 rubato sinc 重采样 (ARCH-DSP-002)",
            ),
            param(
                "normalize",
                "boolean",
                false,
                "是否做峰值归一化 (目标满量程)",
            ),
            param(
                "targetLufs",
                "number",
                false,
                "响度目标 (LUFS, ITU-R BS.1770-4 门限积分; 闭区间 [−70, 0])。给了就按 ±0.5 LU 容差判定: 未达标返回 RENDER_FAILED; 不给则只报实测读数 (默认路径)",
            ),
            param(
                "path",
                "string",
                false,
                "输出路径; 缺省为 <工程文件 stem>.master.<format> (与工程同目录)",
            ),
            param(
                "frames",
                "integer",
                false,
                "⭐ 显式总帧数 (整数, 1..=172800000 即 1 小时 @ 48 kHz)。缺省 = 按工程末端推导 (默认路径, 与加这个实参之前逐字节相同)。⭐ 用途: 把本工具与 CLI `--export-wav --wav-frames` 的帧数**逐帧**对齐 (帧数是 `#250` 的 `C1` 判据\"两侧同 masterDigest\"的六个前置条件之一)。与 `durationSeconds` 二选一: 同给必须指向同一个帧数, 否则 INVALID_PARAMETER_RANGE。大于工程末端 ⇒ 末尾补静音; 小于 ⇒ 裁掉末端",
            ),
            param(
                "durationSeconds",
                "number",
                false,
                "⭐ 显式总时长 (秒, 有限且 > 0)。帧数 = round(秒 × sampleRate) (请求的采样率, ⛔ 不是工程采样率)。与 `frames` 二选一 (同给必须一致); 换算后 0 帧或超过 172800000 帧 ⇒ 显式拒绝",
            ),
            param(
                "contributions",
                "boolean",
                false,
                "⭐ `ADR-0020`: 逐轨贡献读数。缺省 false = 不额外渲染 (`data.contributions` 不出现, 写盘字节与加这个实参之前逐字节相同)。true = 对每条非 master 轨各渲染一次\"只静音该轨\", 在 `data.contributions[]` 报 `{trackId, name, alreadyMuted, deltaRmsDbfs, deltaPeakDbfs}` (两次**未归一化**母带逐样本差的 RMS / 峰值, dBFS; 逐样本无差异或渲染前已静音 ⇒ null), 口径见 `data.contributionsBasis`。⛔ 不改写盘字节; 成本 = 1 + 未静音轨数 次额外渲染",
            ),
        ],
        // 这里声明的仍然是**规范表格那一行**的错误码。实现还会真实产出
        // `NO_ACTIVE_PROJECT` / `INVALID_PARAMETER_RANGE`（参数与路径护栏）/
        // `IO_ERROR`（落盘失败）—— 它们全在 ADR-0001 D25 的 20 值联集内,
        // 由 `docs/ledger/tools-domain-notes.md` 的 boundary-7 与本线台账登记,
        // 不塞进这一列（这一列的口径是"表格里列了什么"）。
        //
        // 本线新增的 `targetLufs` **没有**改变这一列：它产生的两条出口里,
        // `RENDER_FAILED`（未达标 / 测不出）本来就在列内；另一条
        // `INVALID_PARAMETER_RANGE`（目标越出 [−70, 0] 或非有限数）**必须**留在列外
        // —— 判据 `tools::tests::documented_tools_error_codes_cover_the_table_exactly`
        // 明文要求文档十工具声明的每一个码都属于架构 §7.2 的表格并集, 往这一列塞
        // 一个表格外的码会**当场变红**（本线第一版就是这么红的）。
        // ⇒ 口径与上面那段注释一致：**这一列是"表格里列了什么"，不是"实现能产出什么"**。
        // `BUSY` 仍不可达（领域状态单线程同步），见 `domain/render.rs` 模块头。
        errors: &[ErrorCode::RenderFailed, ErrorCode::Busy],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-009",
        name: "yeban_merge_proposal",
        summary: "将审查通过的 AI 提案分支以原子 `Op::Batch` 合并至主分支并广播重绘",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ProjectState,
        params: &[
            param("proposalId", "string", true, "提案分支 EntityId"),
            param("commitMessage", "string", true, "合并提交信息"),
        ],
        errors: &[ErrorCode::ProposalNotFound, ErrorCode::Conflict],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-010",
        name: "yeban_reject_proposal",
        summary: "拒绝并归档指定提案分支, 释放无用内存快照",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ProjectState,
        params: &[
            param("proposalId", "string", true, "提案分支 EntityId"),
            param("reason", "string", true, "拒绝原因"),
        ],
        errors: &[ErrorCode::ProposalNotFound],
    },
    // ---- ADR-0001 D45/D46 的两条扩展（规范表格里没有，契约 enum 里有） ----
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-UNDO",
        name: "yeban_undo",
        summary: "撤销最近 N 步领域操作 (默认 1; 一次提交算一步, 与 UI 的 Cmd+Z 同一实现)",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ProjectState,
        params: &[param(
            "steps",
            "integer",
            false,
            "撤销步数 (默认 1; 一次提交算一步)",
        )],
        // `INDEX_OUT_OF_BOUNDS` 承担"没有可撤销的历史"（ADR-0001 D25 的联集里没有
        // 专门的 `NO_HISTORY`，且**不许发明新码**；语义就是"请求的步数超出可回退深度"）。
        // `NO_ACTIVE_PROJECT` 是没有活跃工程时的领域失败。
        errors: &[ErrorCode::IndexOutOfBounds, ErrorCode::NoActiveProject],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-REDO",
        name: "yeban_redo",
        summary: "重做最近 N 步被撤销的领域操作 (默认 1; 与 UI 的 Cmd+Shift+Z 同一实现)",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ProjectState,
        params: &[param("steps", "integer", false, "重做步数 (默认 1)")],
        errors: &[ErrorCode::IndexOutOfBounds, ErrorCode::NoActiveProject],
    },
    // ---- ADR-0001 D46 的三类扩展能力（每类一个工具，全部**真做事**） ----
    //
    // 三个工具的错误码都落在 **D25 的 20 值联集**里（判据
    // `tests/contract.rs::extension_tools_only_declare_codes_inside_the_d25_union`）：
    // 既有十工具的"表格 16 码"口径**没有**被放宽 —— 这一列的口径是"本工具真的会产出哪些码"。
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-AUTOMATION",
        name: "yeban_edit_automation",
        summary: "读某条自动化泳道的点与值 (唯一求值入口) 并写入一个点 (Op, 可逆)。⭐ 给 `point` 时**本调用直接提交**到当前文档 (`OpOrigin::McpEdit`): ⛔ 不创建提案、没有 `proposalId`、不需要 `yeban_merge_proposal`; 成功响应 `data.applied` 为 `true`。",
        scope: Scope::AppAdmin,
        // 按**最坏情况**声明：给了 `point` 就改工程状态。只读调用在响应里如实标
        // `readOnly: true`（不给 `point` 时一位都不改）。
        side_effect: SideEffect::ProjectState,
        params: &[
            param(
                "trackId",
                "string",
                true,
                "目标音轨 EntityId (26 字符 ULID)",
            ),
            param(
                "lane",
                "string",
                true,
                "自动化目标变体名: TrackVolume | TrackPan | SendGain | DeviceParam | Macro (与 project.json 同一份词汇表; 不接受别名)",
            ),
            param("edgeId", "string", false, "SendGain 必需: 路由边 EntityId"),
            param(
                "slotIndex",
                "integer",
                false,
                "DeviceParam: 设备链插槽下标 (默认 0)",
            ),
            param(
                "paramIndex",
                "integer",
                false,
                "DeviceParam: 参数下标 (默认 0)",
            ),
            param("macroIndex", "integer", false, "Macro: 宏下标 (默认 0)"),
            param(
                "ticks",
                "array",
                false,
                "要读取自动化值的 tick 列表 (保持给定顺序; 最多 256 个)",
            ),
            param(
                "point",
                "object",
                false,
                "要写入的一个点 {tick, value, curve?}; 缺省 = 只读调用 (一位都不改)",
            ),
            param(
                "pointId",
                "string",
                false,
                "点的显式 EntityId; 缺省由 (目标, tick) 确定性派生 (同一 tick 重复写 = 更新同一点)",
            ),
        ],
        errors: &[
            ErrorCode::NoActiveProject,
            ErrorCode::TrackNotFound,
            ErrorCode::EntityNotFound,
            ErrorCode::IndexOutOfBounds,
            ErrorCode::OutOfRange,
            ErrorCode::InvalidParameterRange,
            ErrorCode::Conflict,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-ENGINE-STATE",
        name: "yeban_query_engine_state",
        summary: "查询某轨的设备链与引擎/会话读数 (采样率/缓冲/走带位置/是否播放 + 宿主注入的响度读数)",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ReadOnly,
        params: &[
            param(
                "trackId",
                "string",
                false,
                "要展开设备链的音轨 EntityId; 缺省只报引擎与会话读数",
            ),
            // 读数游标（`docs/ledger/open-questions.md` 问题 2 选 (a)）：本次传输的既有形态是
            // **一请求一响应**（无 keep-alive / 无 chunked），因此"服务端主动推"表达不了；
            // 这里给的是最小的诚实替代 —— 客户端带上次的 `engine.readingsRevision`，
            // 就只取没见过的读数。缺省（不带）时响应形状与从前逐字段相同。
            param(
                "since",
                "integer",
                false,
                "读数游标: 只回传修订号大于它的引擎读数 (缺省 = 不带增量段; 取上一次响应的 engine.readingsRevision)",
            ),
        ],
        errors: &[ErrorCode::NoActiveProject, ErrorCode::TrackNotFound],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-IMPORT-AUDIO",
        name: "yeban_import_audio",
        summary: "把会话资产池里的哈希或磁盘音频文件登记成 clip_pool 音频条目, 并在**同一个 Op::Batch** 里用 Op::SetAsset 声明工程 assets 索引 (许可/原路径/字节数, 由 yeban-decode 的 asset_index 产出); 给了 trackId 就在同一次调用里摆放 (Op::AddClipPlacement)",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ProjectState,
        params: &[
            // ⭐ D46 收口（实现侧）：契约写着 `minLength: 1` ⇒ 空名字必须被拒
            // （第五批实测：空串会**真的落进** `clip_pool`）。
            bounded_param(
                "name",
                "string",
                true,
                "片段显示名 (非空, minLength 1)",
                Some(1),
                None,
            ),
            param(
                "assetHash",
                "string",
                false,
                "已在会话 CAS 池 (assets/{sha256}) 里的资产哈希; 与 `path` 恰好给一个",
            ),
            param(
                "path",
                "string",
                false,
                "磁盘音频文件路径 (WAV/FLAC/OGG); 与 `assetHash` 恰好给一个",
            ),
            // ⭐ `MUST-GATE-014` 要求资产许可**逐条可查**，而
            // `yeban_decode::asset_index` 的契约明文写着"本 crate 不会凭空编一个许可
            // 字符串" ⇒ 这条值只能由调用方给 ⇒ **必填**且非空。
            // ⛔ 本工具**不发明**默认许可（那会让每条导入都留一个假许可）。
            bounded_param(
                "license",
                "string",
                true,
                "资产许可标识 (非空, 逐条可查); ⚠ 必须在白名单里 \
                 (现读 assets/samples/manifest.json 的 licence_whitelist)， \
                 否则结构化拒绝 (licenseNotAllowed); 清单读不到也拒绝 \
                 (licenseWhitelistUnavailable, ⛔ 绝不放行)",
                Some(1),
                None,
            ),
            param("gainDb", "number", false, "片段增益 (dB); 默认 0.0"),
            param(
                "clipId",
                "string",
                false,
                "片段的显式 EntityId; 缺省由 (来源, 名字, 增益) 确定性派生",
            ),
            param(
                "trackId",
                "string",
                false,
                "目标音轨 EntityId (26 字符 ULID); 给了就摆放, 不给就只登记 (片段在母带里不会出现)",
            ),
            param(
                "startTick",
                "integer",
                false,
                "摆放起点 (tick); 默认 0 (只在给了 trackId 时有效)",
            ),
            param(
                "durationTicks",
                "integer",
                false,
                "摆放时值 (tick, >= 1, 剪辑边界); 缺省由素材全长换算 ceil(frames * PPQ * bpm / (sampleRate * 60)) (只在给了 trackId 时有效)",
            ),
            param(
                "placementId",
                "string",
                false,
                "摆放的显式 EntityId; 缺省由 (片段, 音轨, 起点) 确定性派生 (只在给了 trackId 时有效)",
            ),
            param(
                "muted",
                "boolean",
                false,
                "摆放是否静音; 默认 false (只在给了 trackId 时有效)",
            ),
        ],
        errors: &[
            ErrorCode::NoActiveProject,
            ErrorCode::EntityNotFound,
            ErrorCode::FileNotFound,
            ErrorCode::IoError,
            ErrorCode::InvalidParameterRange,
            ErrorCode::RenderFailed,
            ErrorCode::Conflict,
            // 摆放新增的一条出口：目标音轨不存在。它本来就是 D25 联集里的既有码
            // （`yeban_edit_automation` 已声明同一个码），**不是**新码。
            ErrorCode::TrackNotFound,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-DIAGNOSTICS",
        name: "yeban_export_diagnostics",
        summary: "把调试信息与相关文件采集并导出成 zip 诊断包 (人工触发, 供复现排查)",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ReadOnly,
        params: &[param(
            "outDir",
            "string",
            false,
            "输出目录; 缺省为系统临时目录 (响应里给出完整路径)",
        )],
        errors: &[ErrorCode::IoError, ErrorCode::InvalidParameterRange],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-EXPORT-MIDI",
        name: "yeban_export_midi",
        summary: "把活跃工程经 yeban-midi 的共享 SMF 映射导出成字节 (base64 回传, 只读不落盘)。⭐ `#312`: `note.syllable`（非空）真的写成 `FF 05` 歌词元事件, 条数是响应里的 `data.lyrics`（与编码器同一个判定, ⛔ 不是「想写几条」）; ⚠ `phonemes` 没有 SMF 载体（写进去等于把 SVS 对齐数据伪装成歌词）⇒ **只在工程真带 phonemes 时**进 `data.unsupported` / `data.unsupportedCounts`（键 `phonemes`, 计数 = 带 phonemes 的音符数）",
        scope: Scope::AppAdmin,
        // 只读：字节作为工具结果回传，工程状态一位都不改（落盘出口仍是 app CLI，D47）。
        side_effect: SideEffect::ReadOnly,
        // 映射层没有任何可调参数：输出的 SMF 只由工程内容决定（ARCH-DET-* 的确定性口径）。
        params: &[],
        // 错误码全部取自 `ADR-0001` D25 的 20 值联集（判据
        // `tests/contract.rs::extension_tools_only_declare_codes_inside_the_d25_union`）：
        // 没有可导出内容 / 编码器拒绝 → RENDER_FAILED；悬空片段 → CLIP_NOT_FOUND；
        // 拍号或 PPQ 装不进 SMF → INVALID_PARAMETER_RANGE；PPQ 漂移 → CONFLICT。
        errors: &[
            ErrorCode::NoActiveProject,
            ErrorCode::ClipNotFound,
            ErrorCode::InvalidParameterRange,
            ErrorCode::Conflict,
            ErrorCode::RenderFailed,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-IMPORT-MIDI",
        name: "yeban_import_midi",
        summary: "从磁盘读取 Standard MIDI File (.mid) 解析并导入进指定的空 MIDI 片段，采纳单值头元数据并原子提交到撤销历史。⭐ [ARCH-EXT-002]：响应 `data` 逐条回报采纳结果 —— `adoptedTempoBpm` / `adoptedTimeSignature` / `adoptedTempoMap`（tick > 0 的变速映射点，形如 `{ \"<tick>\": bpm }`，与工程 `tempo_map` 同口径）/ `adoptedTimeSignatureMap`（tick > 0 的拍号映射点，形如 `{ \"<tick>\": { \"numerator\": u8, \"denominator\": u8 } }`，与工程 `time_signature_map` 同口径）/ `adoptedKeySignature`（`FF 59` 唯一 tick 0 调号 ⇒ `project.key_signature`，形如 `{ \"sharpsFlats\": i8, \"isMinor\": bool }`）/ `adoptedTrackName` / `adoptedTitle`（导体轨名或 `FF 01` 文本 ⇒ `project.title`）/ `adoptedCopyright`（`FF 02` ⇒ `project.author`）；未采纳的一律为 `null`，⛔ 不拿没做的事填个值；未采纳的原因在 `data.losses.aux_events` 对应类别的 `reason`。多轨拆分 (`splitTracks: true`) 时另报 `createdTracks`（多轨拆分新建音轨列表，每项含 `trackId`、`name`、`clipId`；未拆轨或仅单轨时为空数组）",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ProjectState,
        params: &[
            param("path", "string", true, "源标准 MIDI 文件 (.mid) 磁盘路径"),
            param(
                "trackId",
                "string",
                false,
                "可选的目标音轨 EntityId (26 字符 ULID)；缺省由 destination 选择首条含 MIDI 片段的音轨",
            ),
            param(
                "clipId",
                "string",
                false,
                "可选的目标片段 EntityId (26 字符 ULID)；缺省由 destination 选择目标音轨上的首个 MIDI 片段",
            ),
            param(
                "splitTracks",
                "boolean",
                false,
                "可选的目标行为：是否将多轨 SMF 中的后续音符轨道拆分到新轨道，缺省为 true",
            ),
        ],
        errors: &[
            ErrorCode::NoActiveProject,
            ErrorCode::FileNotFound,
            ErrorCode::IoError,
            ErrorCode::InvalidParameterRange,
            ErrorCode::TrackNotFound,
            ErrorCode::ClipNotFound,
            ErrorCode::Conflict,
        ],
    },
    // ---- 本线的 SFZ 两条扩展（`crates/yeban-mcp/src/domain/sfz.rs`） ----
    //
    // 缺口是**实测**出来的，不是从词面推的（`AGENTS.md` §6.3）：`grep` 全仓
    // `yeban-mcp` 当时的全部工具 ⇒ `yeban_load_sfz` / `yeban_render_sfz_wav` 一个都不存在，
    // 而当时 `yeban_render_master` 的 `unsupported` 里**明写**采样器绑定这个键
    // （`domain/render.rs` 模块头："本模型版本（`DeviceKind`）里没有 SFZ 设备变体"）。
    // ⚠ 那个键已在 `#221` **退役**（采样器现在真的被渲染）：现行全集以
    // `domain/render.rs` 的 `KNOWN_UNSUPPORTED_KEYS` 为**唯一事实源**，里面没有它。
    // 因此"用 SFZ 音色导出 WAV"在工具面**没有通路** —— 这两条补的就是它。
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-SFZ-LOAD",
        name: "yeban_load_sfz",
        summary: "装载一份 SFZ v2 乐器 (走 yeban-sfz 的唯一解析实现) 并返回区域数与会话内 instrumentId; ⭐ G4: 样本文件读不到时 status 仍是 success (文本确实解析了), 但 data.warnings[] 会点名 sfzSamplesUnreadable —— 采样器轨没有正弦回退, 绑轨即整轨静音, 采样须按 assets/samples/manifest.json 登记式外取",
        scope: Scope::AppAdmin,
        // 只读工程、不落盘：装载只把"文本 + 元数据"放进会话态（`MODEL-ISO-001` 第 2 层），
        // 打开 / 关闭工程会整体重置它。`Op` 一位都不写 ⇒ 提交图谱不动。
        side_effect: SideEffect::ReadOnly,
        params: &[
            // ⚠ 必须与 `schemas/mcp-tools.schema.json` 的 `$defs.yeban_load_sfz` 逐字段一致
            // （判据 `tests/contract.rs::extension_argument_constraints_match_the_registry`
            // 双向核对属性集合 / 类型 / 必填集合）。
            // ⚠ `path` 的长度界**必须**与注册表里其他工具的 `path` 一致（都是无界）：
            // `tests/param_bounds_guard.rs::every_string_params_bound_status_is_registered`
            // 要求同名参数在不同工具里的界**逐值相同**（`seen.len() == 1`），而既有的
            // `yeban_open_project` / `yeban_render_master` / `yeban_import_audio` 都声明
            // `BoundStatus::Unbounded`。空串由**领域层**拒绝（`domain/sfz.rs` 的
            // `parse_load_arguments`），这条拒绝不体现在 `ParamSpec` 上。
            param(
                "path",
                "string",
                true,
                "磁盘上的 .sfz 文件路径 (空串/目录/不存在 ⇒ `IO_ERROR` 或 `INVALID_PARAMETER_RANGE`; 联集里没有专门的 SFZ 错误码, 且不许发明)",
            ),
            param(
                "sampleHz",
                "number",
                false,
                "样本侧采样率 (Hz); 缺省 48000。它只进报告里的播放速率比, 本仓不读采样文件",
            ),
        ],
        errors: &[
            // `IO_ERROR` 承担"读文件失败 / 路径是目录"；`INVALID_PARAMETER_RANGE` 承担
            // 空串、非有限采样率、以及"这个路径根本不是音频样本"以外的形状错误。
            // SFZ 文本非法 ⇒ 同样是 `INVALID_PARAMETER_RANGE`（联集里没有 `PARSE_ERROR`，
            // 不许发明），并在 `data.reason = "sfzParseFailed"` 里如实说明来源。
            ErrorCode::IoError,
            ErrorCode::InvalidParameterRange,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-SFZ-WAV",
        name: "yeban_render_sfz_wav",
        summary: "用已装载的 SFZ 乐器把活跃工程的 MIDI 音符离线渲染成 24-bit WAV 并原子落盘",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::Disk,
        params: &[
            param(
                "instrumentId",
                "integer",
                true,
                "`yeban_load_sfz` 返回的会话内 instrumentId; 未知取值 ⇒ `INVALID_PARAMETER_RANGE` (`sfzInstrumentNotFound`)",
            ),
            param(
                "velocity",
                "integer",
                false,
                "整段使用的力度 (0..=127); 缺省 100",
            ),
            param(
                "frames",
                "integer",
                true,
                "输出总帧数 (> 0; 上限 172800000 帧 = 1 小时 @48 kHz); 它就是输出时长, 不是估算",
            ),
            param(
                "sampleRate",
                "integer",
                false,
                "输出采样率 (Hz); 缺省 48000",
            ),
            param(
                "toneHz",
                "number",
                false,
                "内存正弦样本的频率 (Hz); 缺省 440。样本数据来自它（⚠ 仅当区域没有可用采样文件时才走这条内存正弦回退；调用方给了 samples 时按 SFZ 的 sample= 读文件）",
            ),
            // ⚠ 同 `yeban_load_sfz` 的 `path`：界必须与既有工具逐值相同（无界）。
            // 空串与"撞工程/锁文件"由领域层拒绝（`sfz::validate` + `render::guard_output_path`）。
            param(
                "path",
                "string",
                true,
                "输出 WAV 路径 (空串/撞工程文件/撞 .yeban.lock ⇒ `INVALID_PARAMETER_RANGE`)",
            ),
            param(
                "note",
                "integer",
                false,
                "只渲染这一个 MIDI 音高 (0..=127); 不给 = 渲染活跃工程 MIDI 片段里的全部音符",
            ),
        ],
        errors: &[
            ErrorCode::NoActiveProject,
            ErrorCode::InvalidParameterRange,
            ErrorCode::IoError,
            ErrorCode::RenderFailed,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-RECORD-WAV",
        name: "yeban_record_wav",
        summary: "按 ARCH-REC-001 的三段 (预分配无锁环 + 整帧原子推送 + 后台抽干) 录一段内存测试音成 24-bit WAV",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::Disk,
        params: &[
            // ⚠ 与 `schemas/mcp-tools.schema.json` 的 `$defs.yeban_record_wav` 逐字段一致。
            param(
                "path",
                "string",
                true,
                "输出 WAV 路径 (空串/撞工程文件/撞 .yeban.lock ⇒ `INVALID_PARAMETER_RANGE`; 长度界与其他工具的 path 一致 = 无界)",
            ),
            param(
                "seconds",
                "number",
                false,
                "录制秒数 (> 0; 上限 3600 秒); 缺省 2.0。实际写出帧数由环与抽干决定, 响应分列 `requestedFrames` / `framesWritten` / `dropped`",
            ),
            param(
                "sampleRate",
                "integer",
                false,
                "采样率 (Hz, > 0); 缺省 48000",
            ),
            param(
                "channels",
                "integer",
                false,
                "声道数: 只接受 1 (单声道) 或 2 (立体声); 缺省 2",
            ),
            param(
                "capacityFrames",
                "integer",
                false,
                "无锁环容量 (帧, > 0); 缺省 = 采样率 × 1000ms / 1000 (即 1 秒的环)。故意给小的值可观测到 `dropped`",
            ),
            param(
                "drainBatch",
                "integer",
                false,
                "抽取批大小 (帧/次 `pump`, > 0); 缺省 4096。生产与消费按它**交错**",
            ),
            param(
                "toneHz",
                "number",
                false,
                "内存测试音的频率 (Hz, > 0); 缺省 440。⛔ 不是声卡输入",
            ),
        ],
        errors: &[
            // 路径护栏 / 参数越界 ⇒ `INVALID_PARAMETER_RANGE`；抽干 I/O ⇒ `IO_ERROR`；
            // 容器编码被拒 ⇒ `RENDER_FAILED`。全部取自 D25 的 20 值联集, ⛔ 不发明新码。
            ErrorCode::InvalidParameterRange,
            ErrorCode::IoError,
            ErrorCode::RenderFailed,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-MIDI-INPUT",
        name: "yeban_midi_input_probe",
        summary: "打开一个物理 MIDI 输入端口并收一段事件窗口 (只读; 未启用 `midi-input` 或无端口时如实报结构化不可用)",
        scope: Scope::AppAdmin,
        // 只读：读硬件端口、不改工程、不落盘（`Plan::MidiInputProbe` 的 `apply` 原样回传）。
        side_effect: SideEffect::ReadOnly,
        params: &[
            // ⚠ 与 `schemas/mcp-tools.schema.json` 的 name enum 同名登记一致（契约只冻结工具名）。
            param(
                "port",
                "string",
                false,
                "端口选择: 全十进制字符串 (如 \"0\") 按端口下标; 否则按**大小写不敏感的名字子串**匹配。缺省仅在**恰好一个**端口时合法 (两个同型号设备时\"默认挑第一个\"会让调用方以为在跟另一台说话) ⇒ 否则 `INVALID_PARAMETER_RANGE` (reason: midiInputPortRequired / midiInputNoSuchPortIndex / midiInputUnknownPort / midiInputAmbiguousPort); 空串按歧义拒绝",
            ),
            param(
                "seconds",
                "number",
                false,
                "收取窗口秒数 (0 < seconds <= 60); 缺省 3。⚠ 本工具在请求线程上**真的等**这段窗口",
            ),
        ],
        errors: &[
            // 不可用 (feature 未启用 / 零端口 / 宿主不可用 / 连接失败) ⇒ `CONFLICT`;
            // 端口选择与 `seconds` 越界 ⇒ `INVALID_PARAMETER_RANGE`。全部取自 D25 的 20 值联集。
            ErrorCode::Conflict,
            ErrorCode::InvalidParameterRange,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-LIST-PROPOSALS",
        name: "yeban_list_proposals",
        summary: "列出当前**未合并**的提案（只读）: 每条给 `proposalId` / `title` / `opCount` / `opKinds` / `baseCommit` / `headCommit` / `branch` / `createdAt`（外加 1 起的 `index`）。没有未合并的提案 ⇒ `proposals: []` ＋ 一句 `note`（⛔ 不报错）。⚠ 提案记录是**会话运行态**（不落 `project.json` / `history.dag`）⇒ 进程重启后这里为空; 用它可以在提案还没 merge 时重新找到 `proposalId`，但**不能**跨进程恢复提案。",
        scope: Scope::AppAdmin,
        // 只读：读 `domain.proposals`，不改工程、不落盘（`Plan::ListProposals` 的 `apply` 原样回传）。
        side_effect: SideEffect::ReadOnly,
        params: &[param(
            "kind",
            "string",
            false,
            "只看某一类提案: `section` / `notes` / `macro`; 其余取值 ⇒ `INVALID_PARAMETER_RANGE` (⛔ 不静默返回全部)",
        )],
        errors: &[ErrorCode::InvalidParameterRange],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-SCENE-LAUNCH",
        name: "yeban_launch_scene",
        summary: "把一次**量化场景启动**交给宿主：`sceneId` 必须是当前工程里的场景；成功响应的 `data.queued` 只代表**已交给宿主**（宿主在量化点调用引擎），⛔ 不代表已生效；`data.applied` 恒为 `false`。生效读数用 `yeban_query_engine_state` 的 `engine.sceneLaunchesApplied`。⚠ 形态 B（独立 stdio 进程）不托管引擎 ⇒ 响亮报 `CONFLICT`（`reason:\"engineUnavailable\"`），⛔ 不静默成功；本 crate 不依赖 `yeban-engine`（依赖方向门 `FORBIDDEN`），引擎由形态 A 的 `yeban-app` 经宿主 attach 口接上。",
        scope: Scope::AppAdmin,
        // 最坏情况声明：它不改工程文档（`Plan::mutates_project = false`），但会改
        // 会话/引擎运行态 ⇒ `dryRun` 必须短路。标成 `ProjectState` 与
        // `yeban_edit_automation`（同一条纪律）一致。
        side_effect: SideEffect::ProjectState,
        params: &[param(
            "sceneId",
            "string",
            true,
            "场景身份（26 字符 ULID；与 yeban_query_project 的 entities[] 里 kind == \"scene\" 的 id 同形）。合法但不在当前工程里 ⇒ ENTITY_NOT_FOUND (reason sceneNotFound, 带 sceneCount)；⛔ 不回落第一个场景",
        )],
        errors: &[
            ErrorCode::NoActiveProject,
            ErrorCode::EntityNotFound,
            ErrorCode::InvalidParameterRange,
            ErrorCode::OutOfRange,
            ErrorCode::Conflict,
            ErrorCode::Busy,
        ],
    },
];

/// 按名查找工具（`const` 数组上的线性查找；24 个元素，无需哈希表）。
#[must_use]
pub fn tool(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|spec| spec.name == name)
}

/// 按规范 ID 查找工具。
#[must_use]
pub fn tool_by_spec_id(spec_id: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|spec| spec.spec_id == spec_id)
}

/// 工具名清单，规范顺序。
#[must_use]
pub fn tool_names() -> Vec<&'static str> {
    TOOLS.iter().map(|spec| spec.name).collect()
}

/// 规范 ID 清单，规范顺序。
#[must_use]
pub fn spec_ids() -> Vec<&'static str> {
    TOOLS.iter().map(|spec| spec.spec_id).collect()
}

/// 某个错误码在工具表格里被哪些工具声明。
#[must_use]
pub fn tools_declaring(error: ErrorCode) -> Vec<&'static str> {
    TOOLS
        .iter()
        .filter(|spec| spec.errors.contains(&error))
        .map(|spec| spec.name)
        .collect()
}

/// MCP `tools/list` 的结果：`{"tools":[…]}`（顺序 = 规范顺序，逐字节稳定）。
///
/// 结果缓存（[`OnceLock`]）：`TOOLS` 是编译期常量，描述符逐字节稳定，
/// 每次重建只是重复分配。调用方只把返回值当作只读响应体使用，不修改。
#[must_use]
pub fn catalog() -> Value {
    use std::sync::OnceLock;
    static CACHED: OnceLock<Value> = OnceLock::new();
    CACHED
        .get_or_init(|| {
            let mut map = Map::new();
            map.insert(
                "tools".to_owned(),
                Value::Array(TOOLS.iter().map(ToolSpec::descriptor).collect()),
            );
            Value::Object(map)
        })
        .clone()
}

impl ToolSpec {
    /// MCP 工具描述符（`tools/list` 的一项）。
    #[must_use]
    pub fn descriptor(&self) -> Value {
        let mut map = Map::new();
        map.insert("name".to_owned(), Value::from(self.name));
        map.insert("description".to_owned(), Value::from(self.summary));
        map.insert("inputSchema".to_owned(), self.input_schema());
        // ⭐ #176（#174 坑⑥）：成功响应的 `data` 形状由 `crate::output_schema`
        // 派生。没声明形状的工具**不广播**这个键（⛔ 不拿空对象冒充）。
        if let Some(schema) = crate::output_schema::for_tool(self.name) {
            map.insert("outputSchema".to_owned(), schema);
        }
        let mut annotations = Map::new();
        annotations.insert("specId".to_owned(), Value::from(self.spec_id));
        annotations.insert("requiredScope".to_owned(), Value::from(self.scope.as_str()));
        annotations.insert(
            "sideEffect".to_owned(),
            Value::from(self.side_effect.as_str()),
        );
        annotations.insert("dryRunSupported".to_owned(), Value::from(true));
        annotations.insert(
            "sideEffecting".to_owned(),
            Value::from(self.side_effect.is_side_effecting()),
        );
        annotations.insert("idempotent".to_owned(), Value::from(true));
        annotations.insert(
            "errorCodes".to_owned(),
            Value::Array(
                self.errors
                    .iter()
                    .map(|code| Value::from(code.as_str()))
                    .collect(),
            ),
        );
        map.insert("annotations".to_owned(), Value::Object(annotations));
        Value::Object(map)
    }

    /// 由注册表**派生**的 JSON Schema（`type: object` + `properties` + `required`）。
    ///
    /// 契约里的 `definitions.ToolCall.properties.arguments` 只声明了 `dryRun` /
    /// `idempotencyKey` 两个公共键；各工具的真实参数在架构 §7.2 的表格里。
    /// 这里把两处合成一份 schema，作为 `tools/list` 的 `inputSchema` ——
    /// **注册表即契约的唯一实现**，不存在第二份手写 schema 会漂移。
    #[must_use]
    pub fn input_schema(&self) -> Value {
        let mut properties = Map::new();
        let mut required: Vec<Value> = Vec::new();
        for spec in self.all_params() {
            // ⚠ 有的工具（如 `yeban_edit_notes`）**自己**又声明了一份 `idempotencyKey`
            // （描述文本与公共那份不同）⇒ `properties.insert` 会**覆盖**掉公共那份，
            // 于是公共参数的 `maxLength` 在广播面上**消失**（而校验面仍然生效 ⇒ 两边不一致）。
            // 处置：**合并** —— 描述文本取后者，长度约束**谁有取谁**。
            let mut property = properties
                .get(spec.name)
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            for key in ["minLength", "maxLength"] {
                if let Some(value) = property.get(key).cloned() {
                    property.insert(key.to_owned(), value);
                }
            }
            property.insert("type".to_owned(), Value::from(spec.json_type));
            property.insert("description".to_owned(), Value::from(spec.doc));
            // ⭐ D46：注册表即契约的唯一实现 ⇒ 约束也要广播出去（否则 schema 与
            // `tools/list` 各说各话）。
            if let Some(min) = spec.min_len {
                property.insert("minLength".to_owned(), Value::from(min as u64));
            }
            if let Some(max) = spec.max_len {
                property.insert("maxLength".to_owned(), Value::from(max as u64));
            }
            // ⭐ #156：`array` 参数的 item schema（每个 op 的形状 + 效果器参数契约）。
            // 这是"外部 agent 能发现怎么写 EQ / 混响 / 发送"的那一半 —— 只挂在
            // `params` 的声明上还不够，形状必须能**逐字段读到**。
            if let Some(items) = spec.items {
                property.insert("items".to_owned(), items());
            }
            properties.insert(spec.name.to_owned(), Value::Object(property));
            if spec.required
                && !required
                    .iter()
                    .any(|value| value.as_str() == Some(spec.name))
            {
                required.push(Value::from(spec.name));
            }
        }
        let mut schema = Map::new();
        schema.insert("type".to_owned(), Value::from("object"));
        schema.insert("properties".to_owned(), Value::Object(properties));
        if !required.is_empty() {
            schema.insert("required".to_owned(), Value::Array(required));
        }
        schema.insert("additionalProperties".to_owned(), Value::from(false));
        Value::Object(schema)
    }
}

/// `tools/call` 解析 / 校验失败的形状。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ToolCallError {
    /// `params` 不是对象。
    #[error("`params` 必须是对象, 形如 {{\"name\": \"yeban_*\", \"arguments\": {{...}}}}")]
    ParamsNotAnObject,
    /// 缺少 `name`。
    #[error("缺少字符串字段 `name`")]
    MissingName,
    /// `arguments` 缺失（契约 `definitions.ToolCall` 把它列为 required）。
    #[error("缺少对象字段 `arguments` (契约 ToolCall 的必填项)")]
    MissingArguments,
    /// `arguments` 不是对象。
    #[error("`arguments` 必须是 JSON 对象")]
    ArgumentsNotAnObject,
    /// 工具名不在契约枚举里。
    #[error("工具 `{name}` 不在契约工具集中")]
    UnknownTool {
        /// 传入的工具名。
        name: String,
    },
    /// 缺少必填参数。
    #[error("缺少必填参数 `{name}`")]
    MissingParam {
        /// 参数名。
        name: String,
    },
    /// 参数类型不对。
    #[error("参数 `{name}` 类型非法, 期望 `{expected}`")]
    InvalidParam {
        /// 参数名。
        name: String,
        /// 期望的 JSON 类型。
        expected: &'static str,
    },
    /// 字符串参数**长度越界**（`minLength` / `maxLength`，Unicode 码点计数）。
    #[error("参数 `{name}` 长度越界 (minLength = {min:?}, maxLength = {max:?})")]
    ParamLengthOutOfRange {
        /// 参数名。
        name: String,
        /// 契约声明的下界。
        min: Option<usize>,
        /// 契约声明的上界。
        max: Option<usize>,
    },
    /// 未知参数。
    #[error("未知参数 `{name}`")]
    UnknownParam {
        /// 参数名。
        name: String,
    },
}

/// 一次已解析并校验过的工具调用。
#[derive(Clone, Debug)]
pub struct ToolCall {
    /// 命中的工具规范。
    pub tool: &'static ToolSpec,
    /// 实参。
    pub arguments: Map<String, Value>,
}

impl ToolCall {
    /// 由 `tools/call` 的 `params` 解析并**校验**。
    ///
    /// # Errors
    ///
    /// 见 [`ToolCallError`]。
    pub fn from_params(params: Option<&Value>) -> Result<Self, ToolCallError> {
        let Some(Value::Object(object)) = params else {
            return Err(ToolCallError::ParamsNotAnObject);
        };
        let name = match object.get("name") {
            Some(Value::String(name)) => name.as_str(),
            _ => return Err(ToolCallError::MissingName),
        };
        let arguments = match object.get("arguments") {
            Some(Value::Object(arguments)) => arguments.clone(),
            Some(_) => return Err(ToolCallError::ArgumentsNotAnObject),
            None => return Err(ToolCallError::MissingArguments),
        };
        let Some(tool) = tool(name) else {
            return Err(ToolCallError::UnknownTool {
                name: name.to_owned(),
            });
        };
        tool.validate_arguments(&arguments)?;
        Ok(Self { tool, arguments })
    }

    /// `dryRun` 是否为真。
    #[must_use]
    pub fn is_dry_run(&self) -> bool {
        self.arguments
            .get(DRY_RUN_PARAM)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// 幂等键（空字符串按"未提供"处理，理由见 [`crate::dispatch`]）。
    #[must_use]
    pub fn idempotency_key(&self) -> Option<&str> {
        self.arguments
            .get(IDEMPOTENCY_KEY_PARAM)
            .and_then(Value::as_str)
            .filter(|key| !key.is_empty())
    }

    /// 去掉公共参数后的领域实参（`dryRun` / `idempotencyKey` 不属于领域语义）。
    #[must_use]
    pub fn domain_arguments(&self) -> Map<String, Value> {
        let mut map = self.arguments.clone();
        map.remove(DRY_RUN_PARAM);
        map.remove(IDEMPOTENCY_KEY_PARAM);
        map
    }
}

/// 工具调用的结果，形状对齐 `schemas/mcp-tools.schema.json` 的 `definitions.ToolResponse`。
#[derive(Clone, Debug, PartialEq)]
pub enum ToolResponse {
    /// `{"status":"success","data":{...}}`
    Success {
        /// 领域数据（**必须是对象**，契约要求 `data` 是 object）。
        data: Value,
    },
    /// `{"status":"error","error":{"code":...,"message":...}}`
    Failure {
        /// 领域错误码。
        code: ErrorCode,
        /// 人话信息。
        message: String,
        /// 结构化补充。
        data: Option<Value>,
    },
}

impl ToolResponse {
    /// 成功。
    #[must_use]
    pub fn success(data: Value) -> Self {
        Self::Success { data }
    }

    /// 失败。
    #[must_use]
    pub fn failure(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::Failure {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// `status` 字段值。
    #[must_use]
    pub const fn status(&self) -> &'static str {
        match self {
            Self::Success { .. } => "success",
            Self::Failure { .. } => "error",
        }
    }

    /// 错误码（成功时为 `None`）。
    #[must_use]
    pub const fn error_code(&self) -> Option<ErrorCode> {
        match self {
            Self::Success { .. } => None,
            Self::Failure { code, .. } => Some(*code),
        }
    }

    /// 契约形状的 JSON 值。
    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("status".to_owned(), Value::from(self.status()));
        match self {
            Self::Success { data } => {
                map.insert("data".to_owned(), data.clone());
            }
            Self::Failure {
                code,
                message,
                data,
            } => {
                let mut error = Map::new();
                error.insert("code".to_owned(), Value::from(code.as_str()));
                error.insert("message".to_owned(), Value::from(message.clone()));
                if let Some(data) = data {
                    error.insert("data".to_owned(), data.clone());
                }
                map.insert("error".to_owned(), Value::Object(error));
            }
        }
        Value::Object(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_the_documented_tools_then_the_extensions() {
        assert_eq!(TOOLS.len(), TOOL_COUNT);
        // 前 10 个必须是规范编号（顺序即契约顺序）；其后全是 ADR-0001 D45/D46 的扩展。
        assert_eq!(DOCUMENTED_TOOL_COUNT, 10);
        assert_eq!(
            TOOL_COUNT,
            DOCUMENTED_TOOL_COUNT + EXTENSION_TOOL_COUNT,
            "规格表十个 + D45/D46 的扩展"
        );
        assert_eq!(
            EXTENSION_TOOL_COUNT,
            EXTENSION_NAMES.len(),
            "扩展清单必须与常量同步"
        );
        let expected_ids: Vec<String> = (1..=DOCUMENTED_TOOL_COUNT)
            .map(|index| format!("{SPEC_ID_PREFIX}{index:03}"))
            .chain(EXTENSION_NAMES.iter().map(|name| {
                let spec = tool(name).expect("扩展工具必须注册");
                spec.spec_id.to_owned()
            }))
            .collect();
        let actual_ids: Vec<String> = spec_ids().iter().map(|id| (*id).to_owned()).collect();
        assert_eq!(
            actual_ids, expected_ids,
            "规范 ID 必须是 MCP-TOOL-001..010 顺序，扩展追在其后"
        );
        // 扩展 ID **不许**伪装成编号（`MCP-TOOL-011` 是凭空发明的规范编号）。
        for spec in TOOLS.iter().skip(DOCUMENTED_TOOL_COUNT) {
            assert!(
                spec.spec_id.starts_with(EXTENSION_SPEC_ID_PREFIX),
                "扩展工具的 specId 必须带扩展前缀: {}",
                spec.spec_id
            );
            assert!(
                !spec
                    .spec_id
                    .trim_start_matches(EXTENSION_SPEC_ID_PREFIX)
                    .chars()
                    .all(|c| c.is_ascii_digit()),
                "扩展 ID 不得是纯数字后缀（那会伪装成规范编号）: {}",
                spec.spec_id
            );
        }
        let mut names = tool_names();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), TOOL_COUNT, "工具名不得重复");
        for spec in &TOOLS {
            assert!(
                spec.name.starts_with("yeban_"),
                "工具名必须是 yeban_* : {}",
                spec.name
            );
            assert!(!spec.summary.is_empty());
        }
    }

    #[test]
    fn every_tool_declares_dry_run_and_idempotency_key() {
        for spec in &TOOLS {
            for name in [DRY_RUN_PARAM, IDEMPOTENCY_KEY_PARAM] {
                let param = spec
                    .param(name)
                    .unwrap_or_else(|| panic!("工具 {} 缺少契约要求的参数 `{name}`", spec.name));
                assert!(!param.required, "{name} 必须是可选参数");
                assert_eq!(
                    param.json_type,
                    if name == DRY_RUN_PARAM {
                        "boolean"
                    } else {
                        "string"
                    }
                );
            }
        }
    }

    #[test]
    fn documented_tools_error_codes_cover_the_table_exactly() {
        // ⚠ 口径：这一条只管**规范表格的十个工具**。它们声明的错误码必须是
        // "架构 §7.2 表格 16 个"里的，且那 16 个**全部**至少被一个文档工具声明（不多不少）。
        // 扩展工具（D45/D46）另有口径：只要求落在 D25 的 20 值联集里 ——
        // 见下一条判据与 `tests/contract.rs`。
        let documented_tools: Vec<&ToolSpec> = TOOLS.iter().take(DOCUMENTED_TOOL_COUNT).collect();
        for spec in &documented_tools {
            assert!(!spec.errors.is_empty(), "{} 必须声明错误码", spec.name);
            for code in spec.errors {
                assert!(
                    code.is_documented_tool_code(),
                    "{} 声明了表格之外的错误码 {code}",
                    spec.name
                );
            }
        }
        // 16 个文档错误码**全部**至少被一个文档工具声明（不多不少）。
        for code in ErrorCode::DOCUMENTED_TOOL_CODES {
            assert!(
                documented_tools
                    .iter()
                    .any(|spec| spec.errors.contains(&code)),
                "文档错误码 {code} 没有任何文档工具声明 —— 说明契约里有一条没落地"
            );
        }
        let mut declared: Vec<&str> = documented_tools
            .iter()
            .flat_map(|spec| spec.errors.iter().map(|code| code.as_str()))
            .collect();
        declared.sort_unstable();
        declared.dedup();
        let mut documented: Vec<&str> = ErrorCode::DOCUMENTED_TOOL_CODES
            .iter()
            .map(|code| code.as_str())
            .collect();
        documented.sort_unstable();
        assert_eq!(
            declared, documented,
            "文档工具声明的错误码集合必须与表格完全一致"
        );
    }

    #[test]
    fn extension_tools_only_declare_codes_inside_the_d25_union() {
        // ADR-0001 D25：错误码只允许取自 20 值联集，**不许发明新码**。
        // 这条判据对本 crate 的**每一个**工具成立（含文档十工具），因此将来谁在
        // 扩展工具上写一个 `NO_HISTORY` 这种码，这里立刻红。
        for spec in &TOOLS {
            assert!(!spec.errors.is_empty(), "{} 必须声明错误码", spec.name);
            for code in spec.errors {
                assert!(
                    code.is_schema_contract(),
                    "{} 声明了 D25 联集之外的错误码 {code}",
                    spec.name
                );
            }
        }
        // 扩展工具确实用到了 schema 独有码（说明它们不是"照抄十工具那一列"）。
        let extension_codes: Vec<ErrorCode> = TOOLS
            .iter()
            .skip(DOCUMENTED_TOOL_COUNT)
            .flat_map(|spec| spec.errors.iter().copied())
            .collect();
        assert!(
            extension_codes
                .iter()
                .any(|code| ErrorCode::SCHEMA_ONLY.contains(code)),
            "扩展工具应当用到 schema 独有码 (ENTITY_NOT_FOUND / INVALID_PARAMETER_RANGE)"
        );
    }

    #[test]
    fn error_code_catalog_covers_the_union_contract_exactly() {
        // 21 = 契约联集 20 + 1 个实现级 (NOT_IMPLEMENTED)。
        assert_eq!(ErrorCode::ALL.len(), 21);
        assert_eq!(
            ErrorCode::SCHEMA_CONTRACT.len(),
            20,
            "ADR-0001 D25 的联集 20 值"
        );
        // ⚠ 这条只是"计数（防空）"：它数的是**常量自己**，⛔ 不读文档 ⇒
        // 文档 §7.2 少一个码它**照样绿**。真正与文档对账的是
        // `documented_tool_codes_match_the_architecture_section_seven_two`
        // （它**现读** `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §7.2 并双向断言）。
        assert_eq!(
            ErrorCode::DOCUMENTED_TOOL_CODES.len(),
            16,
            "计数（防空）：常量自己的长度，文档对账见 `documented_tool_codes_match_…`"
        );
        assert_eq!(ErrorCode::SCHEMA_ONLY.len(), 4);

        for code in ErrorCode::SCHEMA_CONTRACT {
            assert!(
                ErrorCode::ALL.contains(&code),
                "契约错误码 {code} 未被实现覆盖"
            );
            assert!(code.is_schema_contract());
        }
        // 规范表格的 16 个**全部**落在契约里 (缺口已由 D25 关闭)。
        for code in ErrorCode::DOCUMENTED_TOOL_CODES {
            assert!(ErrorCode::ALL.contains(&code));
            assert!(code.is_documented_tool_code());
            assert!(
                code.is_schema_contract(),
                "表格错误码 {code} 仍然不在契约里 —— 缺口又回来了"
            );
        }
        // 契约减去表格 == 正好那 4 个 schema 原有码。
        let schema_only: Vec<&str> = ErrorCode::SCHEMA_CONTRACT
            .iter()
            .filter(|code| !code.is_documented_tool_code())
            .map(|code| code.as_str())
            .collect();
        assert_eq!(
            schema_only,
            vec![
                "ENTITY_NOT_FOUND",
                "INVALID_PARAMETER_RANGE",
                "PERMISSION_DENIED",
                "ROUTING_CYCLE_DETECTED",
            ],
            "schema 原有码清单是**实测**的"
        );
        // 实现级错误码不在任何契约里。
        assert!(!ErrorCode::NotImplemented.is_schema_contract());
        assert!(!ErrorCode::NotImplemented.is_documented_tool_code());
        // 单射: 字符串不重复。
        let mut texts: Vec<&str> = ErrorCode::ALL.iter().map(|code| code.as_str()).collect();
        texts.sort_unstable();
        texts.dedup();
        assert_eq!(texts.len(), ErrorCode::ALL.len());
    }

    /// `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §7.2 的表格 ↔
    /// [`ErrorCode::DOCUMENTED_TOOL_CODES`] 的**双向**对账。
    ///
    /// ## 为什么必须有它（第 44 条：手抄清单必须对账）
    ///
    /// `DOCUMENTED_TOOL_CODES` 是 §7.2 那张表里错误码的**手抄副本**。在它之前，
    /// 唯一"看着像"守卫的是 `assert_eq!(…len(), 16, "架构 §7.2 表格")` —— ⚠
    /// 那条**只数常量自己的长度**、⛔ **不读文档**：谁把 §7.2 的 `MCP-TOOL-008`
    /// 那格从 `RENDER_FAILED, BUSY` 改成只留 `RENDER_FAILED`，
    /// **长度 16 照样绿**，而规格表已经少了一个码。文档**加**一个码同理不会被发现。
    ///
    /// ## 抽取器的两条已知陷阱（⚠ 都是**实测**踩过的，⛔ 不许重犯）
    ///
    /// 1. `git grep -E` 用的是 **POSIX ERE**，⛔ **不支持 `\b`** —— 靠它写第一版
    ///    扫描器会**全 0**（看起来像"文档里一个码都没有"）。
    /// 2. **用长度过滤**（例如 `[A-Z_]{4,}`）会漏掉 4 个字符的 `BUSY` ⇒ 一度报出
    ///    **假缺口**"文档缺 BUSY"。
    ///
    /// ⭐ **正解**：直接拿**实际存在的 code 名**去匹配（名字从 [`ErrorCode::ALL`]
    /// **现读**，用 `as_str()` 取逐字文本），⛔ **不用长度、也不假设它在哪一列**。
    ///
    /// ## "抽不到东西"必须响亮失败（第 41 条）
    ///
    /// 一个**失灵的抽取器**（章节标题变了、文件被移动）会让"集合相等"变成
    /// "空 == 空" ⇒ 判据**永远绿**。因此这里对**三个**空集各自断言：
    /// 文档侧抽到 0 个码、常量侧为 0、以及 §7.2 段落找不到，都必须炸。
    #[test]
    fn documented_tool_codes_match_the_architecture_section_seven_two() {
        // ---- ① 常量侧：`DOCUMENTED_TOOL_CODES` 的逐字 code 文本 ----
        let constant: std::collections::BTreeSet<String> = ErrorCode::DOCUMENTED_TOOL_CODES
            .iter()
            .map(|code| code.as_str().to_owned())
            .collect();
        assert!(
            !constant.is_empty(),
            "`DOCUMENTED_TOOL_CODES` 抽出了 0 个码 —— ⛔ 不许静默通过"
        );

        // ---- ② 从**实际存在的** code 名里挑，用它们在文档里找（正解） ----
        let doc = std::fs::read_to_string(architecture_doc_path())
            .expect("读 docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md");
        let section = architecture_section(&doc, "### 7.2 ", "### 7.3 ")
            .expect("文档里必须找得到 §7.2 段落（标题 `### 7.2 ` 到 `### 7.3 ` 之间）");

        // ⚠ 名字来源是**实现自己的** 21 个码（⛔ 不是任何手抄清单）；
        // 直接子串匹配，⛔ 不用长度过滤、⛔ 不假设它在表格第几列。
        let from_doc: std::collections::BTreeSet<String> = ErrorCode::ALL
            .iter()
            .map(|code| code.as_str().to_owned())
            .filter(|name| section.contains(name.as_str()))
            .collect();
        assert!(
            !from_doc.is_empty(),
            "从 §7.2 抽出了 0 个 code —— 抽取路径已失灵, ⛔ 不许静默通过"
        );

        // ---- ③ 双向断言 ----
        let missing_in_doc: Vec<&String> = constant.difference(&from_doc).collect();
        assert!(
            missing_in_doc.is_empty(),
            "`DOCUMENTED_TOOL_CODES` 里这些码**没有出现在** §7.2 表格里: \
             {missing_in_doc:?} ⇒ 要么补文档, 要么把常量里那一条删掉 \
             (⛔ 不许把本判据放宽成单向)"
        );
        let missing_in_constant: Vec<&String> = from_doc.difference(&constant).collect();
        assert!(
            missing_in_constant.is_empty(),
            "§7.2 表格里这些码**不在** `DOCUMENTED_TOOL_CODES` 里: \
             {missing_in_constant:?} ⇒ 文档加了码而常量没跟 \
             (这正是本判据要抓的那一半)"
        );

        // 逐条读数：双向相等 ⇒ 数量也必须相等。
        assert_eq!(
            constant.len(),
            from_doc.len(),
            "双向相等 ⇒ 数量必须相等 (现读: 常量 {} 个 / 文档 §7.2 {} 个)",
            constant.len(),
            from_doc.len()
        );
    }

    /// `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` 的路径。
    ///
    /// 与 `tests/contract.rs` 的 `schema_path()` 同一条相对路径规则
    /// （`<crate>/../../docs/...`）。
    fn architecture_doc_path() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("docs")
            .join("YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md")
    }

    /// 取 `start` 标题与 `end` 标题之间的那段文本（⛔ 不假设行号）。
    ///
    /// `None` ⇒ 两个标题之一找不到（调用方**必须**把它当失败，⛔ 不许当"没有内容"）。
    fn architecture_section<'a>(doc: &'a str, start: &str, end: &str) -> Option<&'a str> {
        let from = doc.find(start)?;
        let rest = &doc[from + start.len()..];
        let to = rest.find(end)?;
        Some(&rest[..to])
    }

    #[test]
    fn tool_scopes_are_least_privilege_domain_scopes() {
        for spec in &TOOLS {
            assert!(
                spec.scope.is_app(),
                "{} 不得要求 ui:* 作用域 (界面能力属于 yeban-ui-mcp)",
                spec.name
            );
        }
        assert_eq!(
            tool("yeban_save_project").expect("存在").scope,
            Scope::AppSave
        );
        assert_eq!(
            tool("yeban_close_project").expect("存在").scope,
            Scope::AppSave
        );
        for spec in &TOOLS {
            if !matches!(spec.name, "yeban_save_project" | "yeban_close_project") {
                assert_eq!(
                    spec.scope,
                    Scope::AppAdmin,
                    "{} 是领域意图操作, 需要 app:admin",
                    spec.name
                );
            }
        }
    }

    #[test]
    fn side_effect_classification_is_explicit() {
        let read_only: Vec<&str> = TOOLS
            .iter()
            .filter(|spec| !spec.side_effect.is_side_effecting())
            .map(|spec| spec.name)
            .collect();
        assert_eq!(
            read_only,
            // ⚠ 顺序 = `TOOLS` 数组顺序（规范 10 个在前, 扩展在后）,
            // 因为 `assert_eq!` 比较 Vec 顺序。`yeban_export_diagnostics` 与
            // `yeban_export_midi` 都是扩展, 所以排在文档工具**之后**。
            vec![
                "yeban_query_project",
                "yeban_query_engine_state",
                "yeban_export_diagnostics",
                "yeban_export_midi",
                // 本线的 SFZ 装载：只把"文本 + 元数据"放进**会话态** registry，
                // 不改工程、不落盘、不进提交图谱 ⇒ 显式分类为 read-only。
                "yeban_load_sfz",
                // 物理 MIDI 输入探针：只读硬件端口的事件窗口；连活跃工程都不要求。
                "yeban_midi_input_probe",
                // #176：未合并提案清单 —— 只读 `domain.proposals`，不动工程、不动提案记录。
                "yeban_list_proposals"
            ],
            "只读工具必须显式分类为 read-only"
        );
        assert_eq!(
            tool("yeban_edit_automation").expect("存在").side_effect,
            SideEffect::ProjectState,
            "自动化编辑会写一个点 (按最坏情况声明)"
        );
        assert_eq!(
            tool("yeban_import_audio").expect("存在").side_effect,
            SideEffect::ProjectState
        );
        assert_eq!(
            tool("yeban_render_master").expect("存在").side_effect,
            SideEffect::Disk
        );
        assert_eq!(
            tool("yeban_save_project").expect("存在").side_effect,
            SideEffect::Disk
        );
        assert_eq!(
            tool("yeban_edit_notes").expect("存在").side_effect,
            SideEffect::ProjectState
        );
        assert!(SideEffect::Disk.is_side_effecting());
        assert_eq!(SideEffect::ReadOnly.as_str(), "read-only");
    }

    #[test]
    fn tool_call_parsing_rejects_contract_violations() {
        // 契约 ToolCall 的 required 是 name + arguments。
        assert_eq!(
            ToolCall::from_params(Some(&serde_json::json!({"name": "yeban_query_project"})))
                .unwrap_err(),
            ToolCallError::MissingArguments
        );
        assert_eq!(
            ToolCall::from_params(Some(&serde_json::json!({"arguments": {}}))).unwrap_err(),
            ToolCallError::MissingName
        );
        assert!(matches!(
            ToolCall::from_params(Some(
                &serde_json::json!({"name": "yeban_nope", "arguments": {}})
            )),
            Err(ToolCallError::UnknownTool { .. })
        ));
        assert_eq!(
            ToolCall::from_params(Some(&Value::Null)).unwrap_err(),
            ToolCallError::ParamsNotAnObject
        );
        assert_eq!(
            ToolCall::from_params(Some(
                &serde_json::json!({"name": "yeban_query_project", "arguments": []})
            ))
            .unwrap_err(),
            ToolCallError::ArgumentsNotAnObject
        );
        // 必填参数 + 类型 + 未知参数。
        assert_eq!(
            ToolCall::from_params(Some(&serde_json::json!({
                "name": "yeban_set_macro",
                "arguments": {"trackId": "01J8ZQ00000000000000000001", "macroIndex": 0}
            })))
            .unwrap_err(),
            ToolCallError::MissingParam {
                name: "value".to_owned()
            }
        );
        assert_eq!(
            ToolCall::from_params(Some(&serde_json::json!({
                "name": "yeban_set_macro",
                "arguments": {
                    "trackId": "01J8ZQ00000000000000000001",
                    "macroIndex": 1.5,
                    "value": 0.5
                }
            })))
            .unwrap_err(),
            ToolCallError::InvalidParam {
                name: "macroIndex".to_owned(),
                expected: "integer"
            }
        );
        assert_eq!(
            ToolCall::from_params(Some(&serde_json::json!({
                "name": "yeban_set_macro",
                "arguments": {
                    "trackId": "01J8ZQ00000000000000000001",
                    "macroIndex": 0,
                    "value": 0.5,
                    "dryrun": true
                }
            })))
            .unwrap_err(),
            ToolCallError::UnknownParam {
                name: "dryrun".to_owned()
            },
            "拼错的参数必须被拒绝, 不能静默忽略"
        );
    }

    #[test]
    fn tool_call_reads_common_params_and_strips_them() {
        let call = ToolCall::from_params(Some(&serde_json::json!({
            "name": "yeban_query_project",
            "arguments": {"limit": 10, "dryRun": true, "idempotencyKey": "k1", "_meta": {"x": 1}}
        })))
        .expect("合法调用");
        assert!(call.is_dry_run());
        assert_eq!(call.idempotency_key(), Some("k1"));
        let domain = call.domain_arguments();
        assert!(domain.contains_key("limit"));
        assert!(!domain.contains_key(DRY_RUN_PARAM));
        assert!(!domain.contains_key(IDEMPOTENCY_KEY_PARAM));

        // 空字符串按"未提供"处理。
        let call = ToolCall::from_params(Some(&serde_json::json!({
            "name": "yeban_query_project",
            "arguments": {"idempotencyKey": ""}
        })))
        .expect("合法调用");
        assert_eq!(call.idempotency_key(), None);
        assert!(!call.is_dry_run(), "dryRun 默认 false");
    }

    #[test]
    fn tool_response_shape_matches_the_contract() {
        let ok = ToolResponse::success(serde_json::json!({"tracks": 3}));
        assert_eq!(ok.status(), "success");
        assert_eq!(ok.error_code(), None);
        let value = ok.to_value();
        assert_eq!(value["status"], "success");
        assert!(value["data"]["tracks"].is_number(), "data 必须是对象");
        assert!(value.get("error").is_none());

        let failed = ToolResponse::failure(ErrorCode::ProjectLocked, "锁被占用");
        assert_eq!(failed.status(), "error");
        assert_eq!(failed.error_code(), Some(ErrorCode::ProjectLocked));
        let value = failed.to_value();
        assert_eq!(value["status"], "error");
        assert_eq!(value["error"]["code"], "PROJECT_LOCKED");
        assert_eq!(value["error"]["message"], "锁被占用");
        assert!(value.get("data").is_none());
    }

    #[test]
    fn common_params_are_shared_by_every_tool_and_declared_once() {
        assert_eq!(COMMON_PARAMS.len(), 2);
        for spec in &TOOLS {
            let all = spec.all_params();
            assert_eq!(
                all.len(),
                spec.params.len() + 2,
                "{} 的参数清单必须恰好是特有参数 + 2 个公共参数",
                spec.name
            );
            assert_eq!(all[0].name, DRY_RUN_PARAM);
            assert_eq!(all[1].name, IDEMPOTENCY_KEY_PARAM);
            // yeban_propose_section 的表格里显式写了 dryRun / yeban_edit_notes 写了 idempotencyKey ——
            // 作为**特有**参数重复声明时必须与公共声明同型 (否则两份声明会漂移)。
            for extra in spec.params {
                if extra.name == DRY_RUN_PARAM || extra.name == IDEMPOTENCY_KEY_PARAM {
                    let common = spec.param(extra.name).expect("公共参数");
                    assert_eq!(extra.json_type, common.json_type);
                    assert!(!extra.required);
                }
            }
        }
    }
}
