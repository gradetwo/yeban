//! `yeban_*` 意图工具的注册表 [MCP-TOOL-001..010 + `ADR-0001` D45/D46 的两条扩展, ROAD-M4-003]。
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
//! ## 十二个工具 = 规范表格的十个 + 两条**扩展**（ADR-0001 D45 / D46）
//!
//! `ADR-0001` **D46** 明文："十工具是**起点不是上限**；扩充时必须同步
//! `schemas/mcp-tools.schema.json`（契约是唯一权威定义）"。**D45** 则要求撤销入口
//! "UI+MCP 两侧同接，共用同一实现"。于是本注册表多出两条：
//!
//! | 工具 | 作用 | 规范 ID |
//! | :--- | :--- | :--- |
//! | `yeban_undo` | 撤销最近 N 步（默认 1），走 [`crate::undo_session`] 的**唯一**实现 | `MCP-TOOL-EXT-UNDO` |
//! | `yeban_redo` | 重做最近 N 步（默认 1），同一个实现 | `MCP-TOOL-EXT-REDO` |
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
pub const TOOL_COUNT: usize = 17;

/// **规范表格**里的工具数（`MCP-TOOL-001..010`）。
pub const DOCUMENTED_TOOL_COUNT: usize = 10;

/// 扩展工具数（`MCP-TOOL-EXT-*`）：D45 的撤销入口 2 条 + D46 的三类能力 3 条
/// + D56 的诊断包 1 条 + 本线的 SMF 导出 1 条。
pub const EXTENSION_TOOL_COUNT: usize = 7;

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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParamSpec {
    /// JSON 属性名（`camelCase`，与规范表格一致）。
    pub name: &'static str,
    /// JSON 类型：`boolean` / `string` / `integer` / `number` / `array` / `object`。
    pub json_type: &'static str,
    /// 是否必填。
    pub required: bool,
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
        doc: "只读模拟校验: 只做参数与领域合法性校验, 不改任何状态",
    },
    ParamSpec {
        name: IDEMPOTENCY_KEY_PARAM,
        json_type: "string",
        required: false,
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
        doc,
    }
}

/// 十二个工具，**规范顺序**（`MCP-TOOL-001..010` 在前，D45 的两条扩展在后）。
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
                "从零新建工程 (默认 false = 打开已存在的容器)。目标路径已存在 ⇒ `CONFLICT`, 绝不静默覆盖; 与 `readOnly:true` 同给 ⇒ `INVALID_PARAMETER_RANGE`",
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
            param("fields", "array", false, "字段选择器 (稀疏视图)"),
        ],
        errors: &[ErrorCode::InvalidFieldSelector],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-005",
        name: "yeban_propose_section",
        summary: "在隔离分支 `ai/proposal-{ulid}` 创建章节配器骨架与声部连接",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ProjectState,
        params: &[
            param("sectionName", "string", true, "章节名"),
            param("stylePreset", "string", true, "风格预设"),
            param("bars", "integer", true, "小节数"),
            param("scale", "string", false, "音阶"),
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
        summary: "在指定片段执行音符增删改, 自动进行音域与发声数合法性校验; `create:true` 时用这个身份新建一条 MIDI 片段池条目 (材料创建); `placement` 时把已有片段摆到音轨时间轴上 (材料摆放)",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ProjectState,
        params: &[
            param("trackId", "string", true, "音轨 EntityId (26 字符 ULID)"),
            param(
                "clipId",
                "string",
                true,
                "片段 EntityId; `create:true` 时是**将要新建的**片段身份 (池里已有该身份 ⇒ `CONFLICT`)",
            ),
            param(
                "ops",
                "array",
                true,
                "音符操作列表 (NoteOp): `{\"kind\":\"add\",\"note\":{id?,startTick,pitch,durationTicks,velocity?,probability?,ratchet?,microTimingTicks?}}` / `delete` / `move` / `velocity`。`note.probability` (可选, 0.0..=1.0) 是**确定性**概率触发, 由 `MidiNote::triggers(rng_seed)` 裁决 [MODEL-AST-005]; `note.ratchet` (可选, 整数 1..=16) 的连击细分与 `note.microTimingTicks` (可选, 整数 -240..=240 tick) 的起点偏移都由母带渲染器**真的**消费 (与实时引擎同一条格点公式)。`note` 里这 8 个键之外的字段一律 `INVALID_PARAMETER_RANGE` (绝不静默丢弃): 模型另有 `slide`/`pitchBendCurve`/`syllable`/`phonemes` 四个表现力字段, 工具面**还没有**通路, 渲染器会如实登记进 `unsupported`。给了 `placement` 时允许空数组 (这次调用只摆放, 一个音符都不动)",
            ),
            // `create` / `clipName` 是**可选**实参（缺省 = 逐字节等于旧行为），
            // 与 `yeban_open_project` 的 `create` 同词同义（ADR-0001 D48）。
            //
            // 为什么扩本工具而不新增工具：`ADR-0001` D46 的扩张原则是"先扩既有工具的
            // 参数，只有确实不合适才新增工具"；新增工具要同步 `schemas/mcp-tools.schema.json`
            // 的 `name.enum` + `ExtensionToolArguments.$defs` + `allOf` 三处（本线禁改
            // `schemas/**`）。§7.2 的参数表因此**一字未动**：`clipId` 仍是必填的"目标片段"。
            param(
                "create",
                "boolean",
                false,
                "用 `clipId` 新建一条 MIDI 片段池条目 (缺省 false = 编辑已存在的片段)。`create:true` 时 `ops` 只允许 `add` (它们成为新片段的初始内容): 池里已有该身份 ⇒ `CONFLICT`, 出现其他 `kind` ⇒ `INVALID_PARAMETER_RANGE`",
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
            param(
                "placement",
                "object",
                false,
                "把已有的 `clipId` 摆到 `trackId` 上 (`Op::AddClipPlacement`, 渲染**真的**消费): `{startTick?: 非负整数 (默认 0), durationTicks?: >=1 (默认 = 片段内容长度; 片段推不出长度时必填), placementId?: ULID (默认由 片段+音轨+起点 确定性派生), muted?: 布尔 (默认 false)}`。这四个键之外的键一律 `INVALID_PARAMETER_RANGE`。目标音轨上已有该摆放身份 ⇒ `CONFLICT` (内容相同的重放请用 `idempotencyKey`)。与 `create:true` 同给 ⇒ `INVALID_PARAMETER_RANGE` (先建材料, 再单独一次调用摆放)",
            ),
            param(
                "idempotencyKey",
                "string",
                false,
                "幂等重放键: 同键第二次调用返回 `{\"replayed\":true,\"response\":…}` 信封 (契约 `definitions.ReplayedToolResponse`)",
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
        summary: "触发 `yeban-render` 多核并行导出广播级 WAV 并返回哈希与路径",
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
        summary: "读某条自动化泳道的点与值 (唯一求值入口) 并写入一个点 (Op, 可逆)",
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
        summary: "把会话资产池里的哈希或磁盘音频文件登记成 clip_pool 音频条目 (走 yeban-decode + Op); 给了 trackId 就在同一次调用里摆放 (Op::AddClipPlacement)",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ProjectState,
        params: &[
            param("name", "string", true, "片段显示名"),
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
        summary: "把活跃工程经 yeban-midi 的共享 SMF 映射导出成字节 (base64 回传, 只读不落盘)",
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
];

/// 按名查找工具（`const` 数组上的线性查找；10 个元素，无需哈希表）。
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
#[must_use]
pub fn catalog() -> Value {
    let mut map = Map::new();
    map.insert(
        "tools".to_owned(),
        Value::Array(TOOLS.iter().map(ToolSpec::descriptor).collect()),
    );
    Value::Object(map)
}

impl ToolSpec {
    /// MCP 工具描述符（`tools/list` 的一项）。
    #[must_use]
    pub fn descriptor(&self) -> Value {
        let mut map = Map::new();
        map.insert("name".to_owned(), Value::from(self.name));
        map.insert("description".to_owned(), Value::from(self.summary));
        map.insert("inputSchema".to_owned(), self.input_schema());
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
            let mut property = Map::new();
            property.insert("type".to_owned(), Value::from(spec.json_type));
            property.insert("description".to_owned(), Value::from(spec.doc));
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
        assert_eq!(ErrorCode::DOCUMENTED_TOOL_CODES.len(), 16, "架构 §7.2 表格");
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
                "yeban_export_midi"
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
