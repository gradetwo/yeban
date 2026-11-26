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
//! ## 工具总数 = [`DOCUMENTED_TOOL_COUNT`] + [`EXTENSION_TOOL_COUNT`]（= [`TOOL_COUNT`]）
//!
//! ⚠ 计数以三个常量的现读为准（本文件底部有判据把它们与 `TOOL_COUNT`、schema 的
//! `name.enum` 对齐）；⛔ 不在此处手抄数字 —— 工具数历史上多次手抄后漂移。
//!
//! 扩展段的**唯一清单**是 [`EXTENSION_NAMES`]（各项分类见下表）：D45 的撤销 / 重做、D46 的三类能力、
//! D56 的诊断包、SMF 导出 / 导入、SFZ 装载与音色导出、录音导出、物理 MIDI 输入探针、未合并提案清单
//! （`#176`）、场景启动（`#290`）、分轨导出（`ADR-0030`）、会话乐器清单（审计 `N13`）、插件扫描 +
//! 装载（`ADR-0069`）、ARA 绑定 / 查询 / 渲染、伶人 SVS、原生预制、整曲总览（设计稿 §8，Phase 1）、
//! OTCP 音高校正、移轨、快照对比（设计稿 §8.3，Phase 2）等，条数以 [`EXTENSION_TOOL_COUNT`] 为准。
//!
//! `ADR-0001` **D46** 明文："十工具是**起点不是上限**；扩充时必须同步
//! `schemas/mcp-tools.schema.json`（契约是唯一权威定义）"。**D45** 则要求撤销入口
//! "UI+MCP 两侧同接，共用同一实现"。下表列出的是扩展的一部分（全表见 [`EXTENSION_NAMES`]）：
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
//! | `yeban_export_stems` | 一次调用把每条轨弹成**一个 32-bit float WAV 分轨**（`ADR-0030`；`N + 1` 次渲染，母带 24-bit 路径逐字节不变） | `MCP-TOOL-EXT-EXPORT-STEMS` |
//! | `yeban_list_instruments` | 会话 SFZ registry 的只读枚举（审计 `N13`） | `MCP-TOOL-EXT-LIST-INSTRUMENTS` |
//! | `yeban_ara_bind` | 音频 wav 绑定到 ARA 插件进会话 registry（plan 解码、apply 打开会话） | `MCP-TOOL-EXT-ARA-BIND` |
//! | `yeban_ara_query` | ARA 会话的分析状态与内容查询（只读；内容不可用时报状态不报错） | `MCP-TOOL-EXT-ARA-QUERY` |
//! | `yeban_ara_render` | 已绑定 ARA 会话离线渲染成 WAV（原子落盘 + 文档 blob 哈希回传） | `MCP-TOOL-EXT-ARA-RENDER` |
//! | `yeban_ara_unbind` | 解绑并释放一个 ARA 会话（回收会话名额，只动会话态） | `MCP-TOOL-EXT-ARA-UNBIND` |
//!
//! | `get_song_overview` | 整曲多层总览图：小节级列式 JSON + 自动异常诊断摘要（设计稿 §8，Phase 1；只读；分析由宿主注入的 provider 提供） | `MCP-TOOL-EXT-SONG-OVERVIEW` |
//! | `yeban_correct_pitch` | 把一个音频片段提交给用户自备的 OpenTune 做离线调性 snap（`ADR-0070`；后台线程跑，立即回 `job_id`） | `MCP-TOOL-EXT-OTCP-CORRECT` |
//! | `yeban_correct_pitch_job` | 查校正任务状态；成功后经提案换资产（默认只出提案，`autoMerge` 时当场合并） | `MCP-TOOL-EXT-OTCP-JOB` |
//! | `yeban_correct_pitch_notes` | 读某片段最近一次校正的检测音符表（只读；无任务时报 `otcpNotesUnavailable`） | `MCP-TOOL-EXT-OTCP-NOTES` |
//! | `compare_song_snapshots` | 两版 Songmap 快照的 A/B Diff：逐小节差值 + 新增/删除小节 + 异常与重复增减（设计稿 §8.3，Phase 2；只读；入参的纯函数，不要求活跃工程） | `MCP-TOOL-EXT-COMPARE-SNAPSHOT` |
//! | `yeban_auto_align_tracks` | 自动相位对齐（手动层第二层）：互相关测两轨延时，补偿经提案路径写延时/极性（`apply: false` 只测量） | `MCP-TOOL-EXT-PHASE-ALIGN-AUTO` |

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

/// **成功回执**里的自述键：这次调用属于哪一种提交模型（`ADR-0032` **D1**）。
///
/// 取值恰好四个（`proposal` / `proposalAutoMerged` / `direct` / `none`），由
/// `domain::commit_model_of` 从 `Plan` 推导、在 `apply_inner` 的
/// 成功出口统一注入 —— ⛔ 任何 `apply_*` 里都不许手写这个键（那会把"易错"从
/// 调用方的记忆里搬进生产代码）。
pub const COMMIT_MODEL_KEY: &str = "commitModel";

/// 工具集规模（规范表格的 10 个 + `ADR-0001` D45/D46 的扩展 + 插件扫描／装载 2 条 + ARA 4 条 + 伶人 SVS 2 条 + 原生预制 4 条 + 整曲总览 1 条 + OTCP 音高校正 3 条 + 移轨 1 条 + 快照对比 1 条 + 自动相位对齐 1 条 + 鼓机生成 1 条）。
pub const TOOL_COUNT: usize = 46;

/// **规范表格**里的工具数（`MCP-TOOL-001..010`）。
pub const DOCUMENTED_TOOL_COUNT: usize = 10;

/// 扩展工具数（`MCP-TOOL-EXT-*`）：D45 的撤销入口 2 条 + D46 的三类能力 3 条
/// + D56 的诊断包 1 条 + SMF 导出 1 条 + SMF 导入 1 条 + SFZ 装载 / SFZ 音色导出 2 条
/// + 录音导出 1 条 + **物理 MIDI 输入探针 1 条**（`ADR-0006`，`midi-input` 默认关）
/// + **未合并提案清单 1 条**（`#176`，`#174` 坑②）+ **场景启动 1 条**（`#290`）
/// + **分轨导出 1 条**（`ADR-0030`，一次调用弹完 N 条轨）
/// + **会话乐器清单 1 条**（审计 `N13`，会话 SFZ registry 的只读枚举）
/// + **插件扫描 1 条**（`ADR-0069`，VST3 发现与验证的只读枚举）
/// + **插件装载 1 条**（`ADR-0069`，验证加载后经提案路径挂外部乐器设备）。
/// + **ARA 绑定 1 条**（ARA2 宿主原型：音频 wav 绑定到 ARA 插件进会话 registry）
/// + **ARA 查询 1 条**（ARA2 宿主原型：会话分析状态与内容查询，只读）
/// + **ARA 渲染 1 条**（ARA2 宿主原型：已绑定会话离线渲染成 WAV，写盘）。
/// + **伶人 SVS 2 条**（`synthesize_singing` 提交 + `svs_job` 查询/取回，Phase 2）。
/// + **原生预制 4 条**（`yeban_list_presets` / `yeban_apply_preset` /
///   `yeban_save_preset` / `yeban_preset_bypass`，夜半原生预制机制一期，MCP-only）。
/// + **整曲总览 1 条**（设计稿 §8，Phase 1：`get_song_overview`，小节级列式 JSON + 自动异常诊断摘要）。
/// + **OTCP 音高校正 3 条**（`ADR-0070`：`yeban_correct_pitch` 提交 +
///   `yeban_correct_pitch_job` 查询/交付 + `yeban_correct_pitch_notes` 读谱，
///   子进程驱动用户自备的 OpenTune API 分支构建，单 clip 调性 snap）。
/// + **移轨 1 条**（`yeban_move_track`，`Op::MoveTrack` 的 MCP 出口：显示顺序里的绝对位置，主总线不参与）。
/// + **快照对比 1 条**（设计稿 §8.3，Phase 2：`compare_song_snapshots`，两版快照的 A/B Diff）。
/// + **自动相位对齐 1 条**（手动层第二层：`yeban_auto_align_tracks`，互相关测两轨延时
///   + 经提案路径写延时/极性补偿；测量住在 `yeban-dsp`，宿主注入 provider）。
/// + **鼓机生成 1 条**（鼓机一期：`yeban_generate_drum_pattern`，风格预置 + 三旋钮
///   + 种子经 `yeban-theory` 生成鼓型，落成目标轨上的 MIDI 片段；轨无音源时
///     按风格族装默认鼓组——原声 SFZ / 电子合成）。
/// + **ARA 解绑 1 条**（审计 `#24.5`：`yeban_ara_unbind`，解绑并释放会话，
///   给有界会话 registry 一个显式的回收出口）。
pub const EXTENSION_TOOL_COUNT: usize = 36;

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
    "yeban_export_stems",
    "yeban_list_instruments",
    "yeban_scan_plugins",
    "yeban_load_plugin",
    "yeban_ara_bind",
    "yeban_ara_query",
    "yeban_ara_render",
    "yeban_ara_unbind",
    "synthesize_singing",
    "svs_job",
    "yeban_list_presets",
    "yeban_apply_preset",
    "yeban_save_preset",
    "yeban_preset_bypass",
    "get_song_overview",
    "yeban_correct_pitch",
    "yeban_correct_pitch_job",
    "yeban_correct_pitch_notes",
    "yeban_move_track",
    "compare_song_snapshots",
    "yeban_auto_align_tracks",
    "yeban_generate_drum_pattern",
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
    /// 外部副作用：不改工程、不落盘，但在外部服务侧产生状态
    /// （`synthesize_singing` 在 SVS 服务侧建任务）。`dryRun` 必须短路它。
    External,
}

impl SideEffect {
    /// 规范字符串。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::ProjectState => "project-state",
            Self::Disk => "disk",
            Self::External => "external",
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
        doc: "幂等重放键: 同一个非空键的第二次调用不执行工具, 直接返回首次结果的缓存 (响应是 `{\"replayed\":true,\"response\":…}` 信封, 不是裸 `ToolResponse`); 空串按未提供处理; 作用域 = 工具名 + 键 ⇒ 不同工具复用同一个键不会互相命中",
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
                    // T-FIX-709: `placement.add` 自带的 `trackId` 与顶层同权 ——
                    // 只有它给了目标轨时，顶层 `trackId` 才可以省略。
                    if spec.name == "trackId" && placement_carries_its_own_track(arguments) {
                        continue;
                    }
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

/// `arguments` 的 `placement.add` 是否自带目标轨（T-FIX-709）。
///
/// 只有 `kind` 缺省或等于 `add`、且 `placement.trackId` 在场时才成立：`move` /
/// `remove` 的 `trackId` 属于本形态不适用，不能拿来豁免顶层实参。
fn placement_carries_its_own_track(arguments: &Map<String, Value>) -> bool {
    arguments
        .get("placement")
        .and_then(Value::as_object)
        .is_some_and(|object| {
            let kind = object.get("kind").and_then(Value::as_str).unwrap_or("add");
            kind == "add" && object.get("trackId").is_some()
        })
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

/// `beatGrouping` 的 item schema（`ADR-0046` D5）：逐项 = 该组的拍数，正整数。
///
/// ⛔ 写 `maximum`、⛔ 写数组级 `maxItems`：两者的上界都是**该拍号的拍数**
/// （`felt_beats_per_bar(meter)`），写死会对窄拍号报出"契约说能、本层拒"的矛盾。
#[must_use]
pub fn beat_grouping_items() -> Value {
    serde_json::json!({ "type": "integer", "minimum": 1 })
}

/// `materialClipIds`/`partTracks` 这类**身份数组**的 item schema
/// （审计 `N2`/`N9`）：逐项 = 实体身份（26 字符 ULID 字符串）。
/// ⛔ 写数组级 `minItems`/`maxItems` —— 合法长度是**该流派的声部数**
/// （3 或 4），写死会对另一档声部数报出"契约说能、本层拒"的矛盾
/// （与 `beatGrouping` 的纪律同一条）。
#[must_use]
pub fn entity_id_items() -> Value {
    serde_json::json!({ "type": "string" })
}

/// `paths` 这类**自由字符串数组**的 item schema：逐项 = 一条文件系统路径
/// （⛔ 不是实体身份，别复用 `entity_id_items` —— 那名字会让人以为逐项是 ULID）。
#[must_use]
pub fn string_items() -> Value {
    serde_json::json!({ "type": "string" })
}

/// 三十五个工具，**规范顺序**（`MCP-TOOL-001..010` 在前，D45/D46 及后续的扩展在后）。
pub const TOOLS: [ToolSpec; TOOL_COUNT] = [
    ToolSpec {
        spec_id: "MCP-TOOL-001",
        name: "yeban_open_project",
        summary: "校验排他锁并打开指定 `.yeban` 工程; `sample` 可打开内置样本的可写副本; `create:true` 时从零建一个可渲染的工程 (主总线在路由图里)",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ProjectState,
        params: &[
            param(
                "path",
                "string",
                false,
                "工程文件路径 (打开已有容器时必填; 给 `sample` 时可省略, 此时落到临时目录的唯一新文件并回填实际路径)",
            ),
            param(
                "readOnly",
                "boolean",
                false,
                "以只读方式打开 (不取写锁); 与 `sample` 同给 ⇒ `INVALID_PARAMETER_RANGE` (样本总是打开为可写副本)",
            ),
            param(
                "sample",
                "string",
                false,
                "打开内置样本为**可写副本**: `default` (6 轨演示夹具), `filled` (丰富样本), `empty` (空工程 + 主总线); `demo` 是 `default` 的别名。与 `path` 同给 ⇒ 物化到该新路径; 不给 `path` ⇒ 物化到临时目录的唯一新文件 (回执 `path` 报明实际路径), 该临时副本在 `yeban_close_project` 时自动回收; 要保留请用 `yeban_save_project` 的 `path` 另存。未知名字 ⇒ `INVALID_PARAMETER_RANGE` 并回传 `available` 列表; 目标已存在 ⇒ `CONFLICT`, 绝不覆盖",
            ),
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
                "`create:true` 时的拍号, \"N/D\" 文本 (例 \"3/4\"、\"6/8\"; 缺省 4/4)。取值由模型层 `TimeSignature::validate` 判决: 分子 1..=32, 分母 ∈ {1,2,4,8,16,32}; 形状错误 ⇒ `INVALID_PARAMETER_RANGE`, 取值越界 ⇒ `OUT_OF_RANGE`。不给 `create` 时给它 ⇒ `INVALID_PARAMETER_RANGE` (`createOnlyParameter`)",
            ),
            param(
                "seed",
                "object",
                false,
                "`create:true` 时的最小内容: `{trackCount: 1..=8, clipName: string, notes: [{pitch: 0..=127, durationTicks: >=1}]}`。`sampler` 是 SFZ 路径，绑到第一条种子轨；缺省不绑。缺省 = 1 条 MIDI 轨 + 4 个种子音符 + 一小节摆放 (因此新建的工程立即可 `render_master`, 且 `propose_section` 有 MIDI 材料可用)",
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
        params: &[
            param("force", "boolean", false, "忽略未保存标记强制落盘"),
            param(
                "path",
                "string",
                false,
                "可选**另存为**目标: 给一个与活跃工程不同的新路径时, 内容写到该文件并把活跃工程切换过去 (后续保存落新文件); 目标已存在 ⇒ `CONFLICT`。只读工程不给 `path` 会被拒, 错误 hint 指向本参数",
            ),
            param(
                "includeUserPresets",
                "boolean",
                false,
                "导出时是否带出用户预制（设计稿 §5；缺省 true = 物化进包，条目名 `presets/user/<id>.json`，一律 stored；false = 不内嵌，接收方打开缺用户预制会响亮报错）。收据 `embeddedPresets` 如实报内嵌预制数、字节数与本取值",
            ),
        ],
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
        summary: "分页拉取工程稀疏视图（clip_pool 默认只给摘要；全量音符需显式字段 clip_pool.full），防止海量音符撑爆 Agent 上下文",
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
                "字段选择器 (稀疏视图): 元素必须**恰好**是 `items.enum` 里的一个名字 (白名单 = 解析器的 `SELECTORS`; 不在里面 ⇒ `INVALID_FIELD_SELECTOR`, 带 `availableSelectors`)。去重、保序; 缺省 = 稀疏顶层视图 (`clip_pool` 只给摘要; 全量音符需显式 `clip_pool.full`)",
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
                "自定义走向 (例 \"i-VI-III-VII\"), 只被 `materialSource:\"theory\"` 消费; 缺省 = 该流派登记的首选走向, 无登记时兜底 \"I-V-vi-IV\"。接受两种文本: 罗马数字级数 (在目标调式上展开) 与绝对和弦符号 (例 \"Dm7-G7-Cmaj7\", 符号自带根音/性质、不依赖调式); 整体先罗马、失败再试符号, 混合写法 (\"I-Dm7\") 两边都拒。都失败 ⇒ `INVALID_PARAMETER_RANGE` (带 `romanError`/`symbolError`); 声部连接无可行解 ⇒ 同码 (带 `theoryStep`)",
            ),
            param(
                "includeDrums",
                "boolean",
                false,
                "是否追加鼓组声部 [ADR-0017]: 缺省 false。true = 在段落末尾追加 `\"{sectionName} · Drums\"` 音轨: 鼓点由流派 `GenreRule` 生成, 映射 GM 打击乐键 (底鼓 36 / 军鼓 38 / 踩镲 42 / 吊镲 51), 时值固定 120 tick, 重音取 `velocityMax` 其余取 `velocityMin`, 并直连主总线。两条材料源都支持 (`'project'` 源上 `swingPermille`/`drumBackbeat`/`drumStyle`/`beatGrouping`/`onsetsPerBar`/`drumDensity`/`velocityMin`/`velocityMax` 同样被消费)",
            ),
            param(
                "includeMelody",
                "boolean",
                false,
                "是否追加旋律声部 [ADR-0017]: 缺省 false。true = 在段落末尾追加 `\"{sectionName} · Melody\"` 音轨: 旋律由 `melody_over_chords` 在当前段落展开的 `ChordSpan` 与流派节奏网格上生成, 音域缺省 `48..=84` (`melodyLower`/`melodyUpper` [ADR-0038]), 跳进上界缺省 12 半音 (`melodyMaxLeap` [ADR-0039]), 取音种子缺省由段落 `EntityId` 派生 (`melodySeed` [ADR-0045]), 和弦音取 `velocityMax` 其余取 `velocityMin`, 并直连主总线。`'project'` 源上和声跨度逐小节从材料判出 (判不出 ⇒ `INVALID_PARAMETER_RANGE` 带 `bar`+`hint`), 且要求 `scale` 显式指名",
            ),
            param(
                "drumDensity",
                "string",
                false,
                "鼓点密度档位 [ADR-0028]: 缺省 \"normal\"。\"sparse\" = 只保留流派鼓型里登记为重音 (`accent`) 的击点; 该流派没有登记重音时 ⇒ `INVALID_PARAMETER_RANGE` (带 `hint`)。只允许与 `includeDrums:true` 同给, 否则同码 (带 `hint`); 其余取值同码 (带 `accepted`)",
            ),
            param(
                "voiceSubdivision",
                "integer",
                false,
                "和声声部的节奏切分 [ADR-0033]: 单位 = **每个和弦跨度切成几条** (⛔ 不是每小节击点数, 勿与 `onsetsPerBar` 混同)。取值 1..=16, 缺省 1。N>1 时每个 `ChordSpan` 均匀切成 N 条同音高音符 (力度取 `velocityMin` 档), 最后一条吸收余数 ⇒ 总时值守恒。只作用于 `materialSource:\"theory\"` 的和声声部, 只允许与它同给; 否则或越界/非整数 ⇒ `INVALID_PARAMETER_RANGE` (带 `hint`/`range`)",
            ),
            param(
                "voiceSubdivisions",
                "object",
                false,
                "按**声部名**给值的切分覆盖表 [ADR-0035]: `{\"Bass\": 8, \"Soprano\": 1}`。单位与 `voiceSubdivision` 一致 (每个和弦跨度切成几条, 1..=16)。组合规则: 列出的声部用表里的值, **没列出的沿用 `voiceSubdivision`** (缺省 1)。键必须是该风格预设的和声声部名 (含 `\"Drums\"`/`\"Melody\"` 在内的非声部名 ⇒ `INVALID_PARAMETER_RANGE` 带 `part`+`allowedParts`, ⛔ 静默忽略); 非整数/越界同码。空对象 `{}` 合法 = 无覆盖。只允许与 `materialSource:\"theory\"` 同给",
            ),
            param(
                "onsetsPerBar",
                "integer",
                false,
                "鼓组与旋律的**每小节**音头数 [ADR-0036]: 单位 = 每小节 (⛔ 与 `voiceSubdivision` 的\"每个和弦跨度\"不是同一个量)。缺省 = 该流派拍号一小节的 16 分格位数 (4/4⇒16, 3/4⇒12, 2/4⇒8, 6/8⇒12, 7/8⇒14)。下界 1; 上界 = 该拍号格位数 (由 `yeban-theory` 判定), 超过 ⇒ `INVALID_PARAMETER_RANGE` (带 `maxOnsetsPerBar`+`hint`, ⛔ 静默夹紧)。只作用于鼓组与旋律, 要求 `includeDrums` 或 `includeMelody` 至少一条为 true",
            ),
            param(
                "velocityMin",
                "integer",
                false,
                "生成音符的力度**下档** [ADR-0037]: 单位 = MIDI velocity。取值 0..=127, 缺省 96。消费点: 和声声部全部音符 + 鼓的非重音 + 旋律的非和弦音。只作用于 `materialSource:\"theory\"` 生成路径, 只允许与它同给; 否则/越界 ⇒ `INVALID_PARAMETER_RANGE`。与 `velocityMax` 的相对界 (`min <= max`) 同在此判定 (⛔ 不支持反重音)。⛔ 不改音符身份: 音符 id 与不传时逐条对齐",
            ),
            param(
                "velocityMax",
                "integer",
                false,
                "生成音符的力度**上档** (重音档) [ADR-0037]: 单位 = MIDI velocity。取值 0..=127, 缺省 112 = 逐字节维持既有输出。消费点: 鼓 `hit.accent==true` 与旋律 `chord_tone==true` (即既有\"重音 112 其余 96\"里的那个 112)。约束与拒绝形状同 `velocityMin` (含 `min <= max`); 与 `velocityMin` 相等 ⇒ 全曲单档 (合法, 不视作错误)",
            ),
            param(
                "melodyLower",
                "integer",
                false,
                "旋律声部音高窗口的**下端** [ADR-0038]: 单位 = **MIDI 音高值** (⛔ 不是八度号、⛔ 不是半音偏移)。取值 0..=127, 缺省 48。消费点: 仅旋律生成路径 (`melody_over_chords`); 要求与 `includeMelody:true` 同给, 否则 ⇒ `INVALID_PARAMETER_RANGE` (带 `hint`)。越界同码; 相对界 (`melodyLower <= melodyUpper`) 同在此判定。窗口内没有该调式的音 ⇒ 由 `yeban-theory` 判定并原样转发 (带 `theoryStep:\"melody_over_chords\"`)。⛔ 不筛点、⛔ 不改时值: 只有音高变。跳进上界是另一个键 `melodyMaxLeap` [ADR-0039]",
            ),
            param(
                "melodyUpper",
                "integer",
                false,
                "旋律声部音高窗口的**上端** [ADR-0038]: 单位 = MIDI 音高值。取值 0..=127, 缺省 84 (= theory 的 `MELODY_UPPER_BOUND`) ⇒ 逐字节维持既有输出。约束与拒绝形状同 `melodyLower` (含 `melodyLower <= melodyUpper`); 与 `melodyLower` 相等 = 单音音域 (合法: theory 实测把整条旋律压成那一个音高, onset 数不变)",
            ),
            param(
                "melodyMaxLeap",
                "integer",
                false,
                "旋律**相邻两音的跳进上界** [ADR-0039]: 单位 = **半音距离** (⛔ 不是 MIDI 音高值、⛔ 不是八度号)。取值 0..=127, 缺省 12。`0` 合法 = 只允许同音反复。消费点与 `melodyLower`/`melodyUpper` 同一个 (只被 `melody_over_chords` 读取), 要求与 `includeMelody:true` 同给, 否则 ⇒ `INVALID_PARAMETER_RANGE` (带 `hint`); 越界同码。⛔ 不筛点、⛔ 不改时值: 只有音高变; 窗口里没有满足上界的音阶音时由 `yeban-theory` 判定并原样转发 (带 `theoryStep`)",
            ),
            param(
                "melodySeed",
                "integer",
                false,
                "旋律**取音种子** [ADR-0045]: 单位 = 无单位整数 (u64)。同一个数 + 同一组其余参数 ⇒ 同一条旋律 (确定性)。缺省不给 = 沿用由段落 `EntityId` 派生的种子 (**每个段落不同**) ⇒ 契约里不写 `default`/`maximum`。消费点: 仅旋律生成路径, 要求 `includeMelody:true` 同给, 否则 ⇒ `INVALID_PARAMETER_RANGE` (带 `hint`); 非整数/负数/浮点同码。⚠ 换种子改的是**音高选择**与随之的力度档 ⇒ 起点/时值/音符 id 逐条不变; 可行性随种子移动, 无解由 `yeban-theory` 判定并原样转发 (带 `theoryStep`), ⛔ 静默放宽或换种子重试",
            ),
            param(
                "harmonySeed",
                "integer",
                false,
                "**和声编排种子** [审计 N3]: 单位 = `u64` 整数。缺省不给 = 走向取流派首选、音阶取 `primary_scale`。给了 = `scale` 缺席时音阶改由种子抽取、走向改由种子抽取 (与 `melodySeed` 同一支\"换个数再来一版\"语义, 消费点不同)。要求 `materialSource:\"theory\"` 同给, 否则 ⇒ `INVALID_PARAMETER_RANGE` (带 `hint`); `progression` 与 `scale` 都显式给了时无槽位可消费, 同码。⚠ 本键不兼作鼓种子 (theory 的鼓生成没有种子入参)",
            ),
            param(
                "harmonyMaxVoiceJump",
                "integer",
                false,
                "和声声部**相邻两个和弦之间**的跳进上界 [ADR-0040]: 单位 = 半音距离 (同一声部在前后两个和弦里的音高差, ⛔ 不是 MIDI 音高)。取值 0..=127; 缺省不给 = 不覆盖 theory 出厂值 (3/4 声部同为 12)。消费点: 仅和声生成路径 (`voice_leading::realize`, 3 与 4 声部都接线), 只要求与 `materialSource:\"theory\"` 同给。越界 ⇒ `INVALID_PARAMETER_RANGE` (带 `field`+`value`+`range`)。`0` 合法 (和弦不动); 某上界在该进行里可不可行随进行移动, 由 `yeban-theory` 判定并原样转发 (带 `theoryStep`), ⛔ 静默放宽。⛔ 不筛和弦、⛔ 不改时值: 只有和声声部音高变。与 `melodyMaxLeap` 是两个键 (那个管旋律线相邻两音)",
            ),
            param(
                "swingPermille",
                "integer",
                false,
                "鼓网格的**摇摆比例** [ADR-0041]: 单位 = **千分比** (每对相邻格位里第二个 onset 向后推移的比例; 500 = 平直不推, 1000 = 推到半个格位)。取值 500..=1000。⛔ 没有固定缺省值: 不给 = 不覆盖该流派登记的摇摆值 (随流派移动) ⇒ 契约里不写 `default`。消费点: 仅鼓轨网格, 要求与 `includeDrums:true` 同给, 否则 ⇒ `INVALID_PARAMETER_RANGE` (带 `hint`); 越界同码。⛔ 不筛点、⛔ 不改音色/力度/时值: 只改 `tick`。与 `onsetsPerBar`、`drumDensity` 是不同的轴",
            ),
            param(
                "drumBackbeat",
                "integer",
                false,
                "鼓的**军鼓落点** [ADR-0043]: 单位 = **拍序号** (第 0 基; 军鼓打在能被它整除的那些**拍的起点**上; `1` = 每一拍都打, ⛔ 不是\"不打军鼓\"; 3/4 上 `2` = 第 0、2 拍)。取值 1..=该流派拍号的拍数 (上界随拍号移动 ⇒ 契约里不写 `maximum`/`default`)。缺省不给 = 不覆盖 theory 的派生值。消费点: 仅鼓轨, 要求与 `includeDrums:true` 同给, 否则 ⇒ `INVALID_PARAMETER_RANGE` (带 `hint`); 越界由本层先拒 (带 `field`+`value`+`range`)。⚠ 本键会加/筛击点 (与 `swingPermille` 只推 tick 不同)。该落点在此拍号下产不产得出鼓点由 `yeban-theory` 判定并原样转发 (带 `theoryStep`)",
            ),
            param(
                "drumStyle",
                "string",
                false,
                "鼓的**底鼓口径** [ADR-0044]: 单位 = 档位名 (⛔ 数字、⛔ 枚举下标)。`\"metric\"` = 底鼓落在**组起点**; `\"four-on-the-floor\"` = 底鼓落在**每一个拍的起点**。候选 = theory 的 `DrumStyle` 词表 (契约 `enum` 与错误载荷 `accepted` 同源生成)。⛔ 没有固定缺省值: 不给 = 不覆盖该流派登记的口径 ⇒ 契约里不写 `default`。消费点: 仅鼓轨, 要求与 `includeDrums:true` 同给, 否则 ⇒ `INVALID_PARAMETER_RANGE` (带 `hint`); 未知取值同码 (带 `accepted`), ⛔ 静默降级成 `metric`。⚠ 本键只动底鼓。与 `drumBackbeat`/`swingPermille`/`onsetsPerBar`/`drumDensity` 是不同的轴",
            ),
            array_param(
                "beatGrouping",
                false,
                "鼓网格的**拍内分组** [ADR-0046]: 单位 = 拍数序列, 逐项 = 该组的拍数 (⛔ tick、⛔ \"每小节打几下\"); item = 正整数。缺省不给 = 不覆盖拍号自身的度量层级 ⇒ 契约里不写 `default`。合法形状随拍号移动: 逐项 >= 1 且总和 == 该拍号的拍数 (4/4 ⇒ `[4]`/`[2,2]`/`[1,1,1,1]`), 唯一判者是 theory 的 `BeatGrouping::new` ⇒ 契约里不写 `maximum`; 不合法 ⇒ `INVALID_PARAMETER_RANGE` (带 `field`+`value`+`beatsPerBar`), ⛔ 夹紧、⛔ 静默降级。消费点: 仅鼓轨, 要求与 `includeDrums:true` 同给。⚠ 本键改分派与重量 (组起点决定底鼓与吊镲落点、重音档跟着变), ⛔ 不改网格",
                crate::tools::beat_grouping_items,
            ),
            array_param(
                "materialClipIds",
                false,
                "**指名材料表**: 单位 = 片段池条目身份数组 (ULID 字符串, 第 `i` 项喂第 `i` 个声部); item = 实体身份字符串。缺省不给 = 可用材料按 `clip_pool` 顺序轮询分配。长度必须**恰等于**该流派的**和声**声部数 (3 或 4); 每一项必须在 `clip_pool` 里存在且是可用 MIDI 材料 (至少一个音符), 重复项允许。只允许与 `materialSource:\"project\"` (或缺省) 同给, 与 `progression`/`melodySeed` 等 theory 键互斥 (同给必有一边被静默忽略 ⇒ 响亮拒)。长度不符 ⇒ `INVALID_PARAMETER_RANGE`; 条目不存在或不是可用材料 ⇒ `CLIP_NOT_FOUND`",
                crate::tools::entity_id_items,
            ),
            array_param(
                "partTracks",
                false,
                "**复用既有音轨**: 单位 = 音轨身份数组 (ULID 字符串, 第 `i` 项是第 `i` 个声部的目标音轨); item = 实体身份字符串。缺省不给 = 每个声部新开一条确定性派生轨。长度必须**恰等于**声部总数 (含 `includeDrums`/`includeMelody` 追加的声部); 每一项必须在 `project.tracks` 里存在且是 `TrackKind::Midi`; **不允许重复项**。复用的轨不改名不改电平, 路由按现状补齐 (缺节点/缺边才补)。长度不符/重复 ⇒ `INVALID_PARAMETER_RANGE`; 轨不存在或不是 MIDI 轨 ⇒ `TRACK_NOT_FOUND`。与 `materialClipIds` 正交, 与 `materialSource` 两条路径都可同给",
                crate::tools::entity_id_items,
            ),
            param(
                "autoMerge",
                "boolean",
                false,
                "[ADR-0029]: 缺省 false = 只建提案 (`pendingMerge:true`)。true = 同一次调用内**背靠背执行既有的** `yeban_merge_proposal` 合并实现, 响应 = 提案回执 ⊕ 合并回执 (`autoMerged:true` / `pendingMerge:false` / `projectUnchanged:false` / `merge:{...}`)。第二步失败 ⇒ 提案**保持 Open**, 错误 `data` 带 `proposalCreated:true` + `proposalId` + `proposalRemainsOpen:true` (⛔ 不假装回滚)",
            ),
            param(
                "mergeCommitMessage",
                "string",
                false,
                "[ADR-0029]: `autoMerge` 合并提交的信息, 只随 `autoMerge:true` 生效; 缺省同手动合并的既有缺省 `\"merge proposal {id}\"`",
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
        summary: "⭐ **本调用永远写入隔离提案分支，必须 `yeban_merge_proposal` 才落到 main** (缺省响应 `data.pendingMerge:true`、`data.projectUnchanged:true`, 成功只代表提案已创建; 给 `autoMerge:true` 时本次调用内执行同一条合并实现, 响应改为 `pendingMerge:false` + `autoMerged:true` + `merge:{…}`)。用 `yeban_list_proposals` 查未合并提案。⭐ 易错点: 效果器参数名拼错 (与已识别拼写编辑距离 ≤ 2) 在解析期 `INVALID_PARAMETER_RANGE` (`unknownEffectParam`, 带最近邻建议); 更远的未知名字会被接受但不生效, 逐条报在 `data.warnings[]`。`insertDevice` 装到无可发声内容的源轨上时 `warnings[]` 另报 `deviceOnSilentTrack` (不阻断)。内置音色选择也是 `insertDevice` + `device.kind:\"InternalInstrument\"` (参数名见 `ops.items` 的 `insertDevice` 分支); 鼓机五个 `*_note` 键位只写一部分 ⇒ 解析期 `INVALID_PARAMETER_RANGE` (`incompleteDrumNoteMap`)。可与 `placement` 同给 (单次提案原子生成片段 + 摆放); `create:true` 时以 `clipId` 新建片段池条目；同轨第 2 台及之后通道条设备参数被遮挡时 `warnings[]` 报 `ignoredStripDevices`。",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ProjectState,
        params: &[
            param(
                "trackId",
                "string",
                false,
                "音轨 EntityId (26 字符 ULID); **按 `ops[].kind` 条件必需**（T-FIX-652/T-FIX-708: 顶层与逐条寻址统一 —— 批内**每条**要么自带 `ops[].trackId`、要么是 `removeTrack` / `addTrack` / 路由级（`addRoutingNode` / `connectRouting` / `disconnectRouting` / `setRoutingGain` / `removeRoutingNode`，寻址全在 `nodeId` / `edgeId`）这类不需要顶层音轨的形态时, 顶层 `trackId` 可省略；`setParam` / `setTrackMute` / `setTrackSolo` / `setTrackPolarity` / `setTrackDelay` / `setTrackTune` / `insertDevice` / `removeDevice` 都可用 `ops[].trackId` 逐条点名, 省缺时回退顶层 `trackId`）。需要而缺 ⇒ `INVALID_PARAMETER_RANGE` (`reason: \"trackIdRequired\"`): 设备形态两处都缺时错误点名 `ops[i].trackId` 并给可修正的 hint; 其余形态点名顶层 `trackId` 与 `requiredByKinds`。顶层与逐条同时给出且不一致时**以逐条值为准**（顶层不是额外必填, 也不是冲突源）",
            ),
            param(
                "clipId",
                "string",
                false,
                "片段 EntityId; ⭐ **按 `ops[].kind` 条件必需**: 只有**读**它的形态需要 —— 音符形态 (清单以 `domain::notes::ENVELOPE_CLIP_KINDS` 为准)、池级 `removeClip`, 以及 `placement` 在场 / `create:true` 时。其余形态 (音轨/设备/路由/段落/场景/资产/文档级) **不读**它: 省略即可, 给了也接受但忽略 (不查片段池)。需要而缺 ⇒ `INVALID_PARAMETER_RANGE` (`reason:\"clipIdRequired\"`, 带 `requiredByKinds`)。逐 kind 口径见 `ops.items` 各分支的 `x-envelope.clipId` 与分支描述。`create:true` 时它是**将要新建的**片段身份 (池里已有 ⇒ `CONFLICT`)",
            ),
            array_param(
                "ops",
                true,
                "操作列表 (NoteOp 数组): 逐 `kind` 的形状、必填键与封闭性见本参数 `items` 的 `oneOf` 分支 (与解析器逐臂对账); `insertDevice` 的效果器参数名/单位/值域见该分支的 `device.params.items`。封闭形态上多余的键一律 `INVALID_PARAMETER_RANGE` (⛔ 静默丢弃)。顶层 `clipId` 只在读它的形态上必需 (见 `clipId` 参数)。⭐ 最小示例: `[{\"kind\":\"add\",\"note\":{\"startTick\":0,\"pitch\":60,\"durationTicks\":480}}]`",
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
            // 只有**读**它的音符形态（清单 = `ENVELOPE_CLIP_KINDS` 去掉 `removeClip`）/ 池级
            // `removeClip` / `placement` / `create:true` 读它。
            param(
                "create",
                "boolean",
                false,
                "用 `clipId` 新建一条 MIDI 片段池条目 (缺省 false = 编辑已存在的片段)。`create:true` 时 `ops` 允许 `add` (它们成为新片段的初始内容) 与纯音轨元数据 `setTrackName` / `setTrackColor` (不依赖新片段, 编译成既有 Op 并与 `Op::AddClip` 同批原子施加); 池里已有该身份 ⇒ `CONFLICT`, 出现其他 `kind` ⇒ `INVALID_PARAMETER_RANGE` (含音符级/片段级/摆放级/设备级/自动化级与 `removeClip`); `setTrackName`/`setTrackColor` 指向不存在的轨 ⇒ 整批拒绝且不建片段。可与 `placement` 同给 —— 单次提案同时生成 `Op::AddClip` 与 `Op::AddClipPlacement`; 此时 `durationTicks` 缺省由新片段的全部音符推导",
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
                "摆放编辑, 三种形态由 `kind` 选择 (缺省 `add`): `add` = 把 `clipId` 摆到 `trackId` 上 —— 目标轨可写在 `placement.trackId?`（给出时它就是目标；与顶层 `trackId` 同时给出且不一致 ⇒ `INVALID_PARAMETER_RANGE`/`placementTrackIdMismatch`，点名两处身份），也可以省略 `placement.trackId` 用顶层身份 (键: `startTick?` 缺省 0 / `durationTicks?` 缺省 = 片段内容长度, 推不出时必填 / `placementId?` 缺省确定性派生 / `muted?`、`loopEnabled?` 缺省 false / `loopStartTick?`、`loopEndTick?` / `fadeInTicks?`、`fadeOutTicks?` 仅音频片段, MIDI 给了响亮拒绝, 之和不得超过 `durationTicks`); `loopEnabled:true` 且循环区间短于 `durationTicks` 时渲染器真的展开 N 遍。`move` = `{kind:\"move\", placementId, startTick}` 平移已有摆放; `remove` = `{kind:\"remove\", placementId}` 取走已有摆放。 add 另接受 sourceStartSamples? (素材内起点, 帧; 仅音频片段, 改从素材第几帧开始读而不移动时间轴); sourceStart 形态 = kind+placementId+sourceStartSamples, 改一条已有音频摆放的素材内起点 (摆放身份不变)。多余的键一律 `INVALID_PARAMETER_RANGE`; `move`/`remove` 的 `clipId` 必须等于该摆放引用的片段 (`placementClipMismatch`); 轨上没有该摆放 ⇒ `ENTITY_NOT_FOUND`; 与 `create:true` 同给时只能是 `add`",
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
            param(
                "autoMerge",
                "boolean",
                false,
                "[ADR-0029]: 缺省 false = 只建提案 (`pendingMerge:true`)。true = 同一次调用内**背靠背执行既有的** `yeban_merge_proposal` 合并实现, 响应 = 提案回执 ⊕ 合并回执 (`autoMerged:true` / `pendingMerge:false` / `projectUnchanged:false` / `merge:{...}`)。第二步失败 ⇒ 提案**保持 Open**, 错误 `data` 带 `proposalCreated:true` + `proposalId` + `proposalRemainsOpen:true` (⛔ 不假装回滚)",
            ),
            param(
                "mergeCommitMessage",
                "string",
                false,
                "[ADR-0029]: `autoMerge` 合并提交的信息, 只随 `autoMerge:true` 生效; 缺省同手动合并的既有缺省 `\"merge proposal {id}\"`",
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
                "mappings",
                "array",
                false,
                "整体替换该宏的映射列表: 每项 = {\"target\": <AutomationTarget 同形载荷>, \"depth\": <目标单位振幅>}。提供时在值调整的级联展开之后追加 `Op::SetMacroMappings` (old = 文档现值, 陈旧载荷在模型层逐项相等校验; 空数组 = 清空全部映射); 缺省 / null = 不动映射",
            ),
            param(
                INCLUDE_OPS_PARAM,
                "boolean",
                false,
                "是否回传完整 op 载荷: 缺省 false = 只回结构化字段 (willCreate + opCount/opKinds), true = 额外回传逐条 ops (审查用途) [BASELINE-006]",
            ),
            param(
                "autoMerge",
                "boolean",
                false,
                "[ADR-0029]: 缺省 false = 只建提案 (`pendingMerge:true`)。true = 同一次调用内**背靠背执行既有的** `yeban_merge_proposal` 合并实现, 响应 = 提案回执 ⊕ 合并回执 (`autoMerged:true` / `pendingMerge:false` / `projectUnchanged:false` / `merge:{...}`)。第二步失败 ⇒ 提案**保持 Open**, 错误 `data` 带 `proposalCreated:true` + `proposalId` + `proposalRemainsOpen:true` (⛔ 不假装回滚)",
            ),
            param(
                "mergeCommitMessage",
                "string",
                false,
                "[ADR-0029]: `autoMerge` 合并提交的信息, 只随 `autoMerge:true` 生效; 缺省同手动合并的既有缺省 `\"merge proposal {id}\"`",
            ),
        ],
        errors: &[ErrorCode::TrackNotFound, ErrorCode::IndexOutOfBounds],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-008",
        name: "yeban_render_master",
        summary: "触发 `yeban-render` 离线导出广播级 WAV 并返回哈希与路径 (缺省多线程; `threads` 可指定线程数, 逐字节相同)。响应带实测读数: `data.clippedSamples` (会被 24-bit 量化器钳住的样本数) / `data.truePeakDbfs` (真峰值, 0 = 满量程, >0 = 过载) / `data.headroomDb` (= −truePeakDbfs)。⭐ 削波时 `data.warnings[]` 恰好一条 `code:\"exportClipping\"` (带 `suggestedTrimDb` 与最响的源); 可闻却 `peak == 0.0` 的源另由一条 `code:\"silentSources\"` 点名; 鼓机键位没写全的设备另由一条 `code:\"incompleteDrumNoteMap\"` 点名; 都不出现时 `warnings` 是空数组。⛔ 读数与预警不自动改增益; 唯一会改增益的是显式 `targetLufs`。逐源峰值见 `data.sources[]` (源节点输出, ⛔ 不含插入链与路由边增益); `data.peak.before/after` 是归一化前/后的浮点母带峰值 (⛔ 不是写盘文件峰值)。可选 `frames`/`durationSeconds` 指定总长度, `startTick`/`endTick` 圈定部分渲染区间, `contributions:true` 附逐轨贡献读数。⭐ `sampleRate` 可省略, 缺省 = 工程采样率 (原样导出不必写死)",

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
                false,
                "输出采样率 (Hz); ⭐ 缺省 = 工程采样率 (`audio_config.sample_rate`), 原样导出不必给、也不重采样; 显式给出且与工程不一致时由 rubato sinc 重采样 (ARCH-DSP-002); 非法值 (非正/非有限/超范围) 仍显式拒绝",
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
                "响度目标 (LUFS, ITU-R BS.1770-4 门限积分; 闭区间 [−70, 0])。给了就**施加增益**去够它: 与 CLI `--loudness-target` 同一份伺服 (增益 = 目标 − 实测, 真峰值上限 −1.0 dBTP), 施加后按 ±0.5 LU 容差复测判定; 结果在 `data.loudness`。已达标时增益恒为 0、产物逐字节不变。施加后仍未达标 ⇒ `RENDER_FAILED` (带原因分类与诊断)。不给则只报实测读数。先降 crest 再补增益见 `masteringClipper`",
            ),
            param(
                "masteringClipper",
                "object",
                false,
                "软削波段 (响度目标施加级的增能一级): 只与 `targetLufs` 同给时生效 (单独给 ⇒ `INVALID_PARAMETER_RANGE`)。流程 = clipper 先降 crest → 增益伺服补剩余增益 → 真峰值上限 −1.0 dBTP 兜底 (与母带链/CLI 同一条伺服)。子键全可选, 缺省 = 器件缺省: `driveDb` [0, 24] 缺省 6 / `ceilingDb` [−6, 0] 缺省 −1 / `kneeDb` [0, 6] 缺省 3 / `mix` [0, 1] 缺省 1 (⚠ mix<1 时干声绕过削波器, `targetLufs` 可能物理上到不了 —— 可达上限见回执 `data.loudness.reachableIntegratedLufs`) / `oversample` 1|2|4|8 缺省 4。未知子键/非数字/越界 ⇒ `INVALID_PARAMETER_RANGE` 并点名子键",
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
                "显式总帧数 (整数, 1..=172800000)。缺省 = 按工程末端推导。用途: 与 CLI `--export-wav --wav-frames` 的帧数逐帧对齐。与 `durationSeconds` 二选一: 同给必须指向同一个帧数, 否则 `INVALID_PARAMETER_RANGE`。大于工程末端 ⇒ 末尾补静音; 小于 ⇒ 裁掉末端",
            ),
            param(
                "durationSeconds",
                "number",
                false,
                "⭐ 显式总时长 (秒, 有限且 > 0)。帧数 = round(秒 × sampleRate) (请求的采样率, ⛔ 不是工程采样率)。与 `frames` 二选一 (同给必须一致); 换算后 0 帧或超过 172800000 帧 ⇒ 显式拒绝",
            ),
            param(
                "startTick",
                "integer",
                false,
                "部分渲染区间起点 (tick, ≥ 0, 左闭)。与 `endTick` 一起圈定区间: 真区间渲染 (只按区间编译、音源 seek 到起点起渲, ⛔ 不是先渲全程再切)。跨起点的音符由引擎音源预滚追补、与全程同段一致; 起点前已结束内容的尾音不带入。缺省 = 工程起点; 只给一端时另一端取工程首/尾。统计与 `targetLufs` 只算区间。负数/小数/`startTick >= endTick`/与 `contributions` 同给 ⇒ `INVALID_PARAMETER_RANGE`; 换算成帧后为空或超出工程总长 ⇒ `RENDER_FAILED` 且不落盘",
            ),
            param(
                "endTick",
                "integer",
                false,
                "⭐ 部分渲染区间终点 (tick, ≥ 0, 右开)。语义与校验见 `startTick`; 缺省 = 本次全程渲染的末端",
            ),
            param(
                "contributions",
                "boolean",
                false,
                "逐轨贡献读数: 缺省 false = 不额外渲染 (`data.contributions` 不出现, 写盘字节不变)。true = 对每条非 master 轨各渲染一次\"只静音该轨\", 在 `data.contributions[]` 报 `{trackId, name, alreadyMuted, deltaRmsDbfs, deltaPeakDbfs}` (两次未归一化母带逐样本差的 RMS/峰值; 无差异或渲染前已静音 ⇒ null), 口径见 `data.contributionsBasis`。⛔ 不改写盘字节; 成本 = 1 + 未静音轨数次额外渲染。与 `startTick`/`endTick` 同给 ⇒ `INVALID_PARAMETER_RANGE`",
            ),
            param(
                "threads",
                "integer",
                false,
                "⭐ 审评 2026-10-31（P1-1）: 显式渲染线程数（正整数）。缺省 = 多线程默认路径（可用并行度；Nightfall v1）。给了就透传给渲染器；输出与单线程逐字节相同（固定顺序汇聚，与线程数无关）。0 / 负数 / 非整数 ⇒ INVALID_PARAMETER_RANGE。`data.request.threads` 回显请求值（缺省为 null）",
            ),
            param(
                "maxSampleFiles",
                "integer",
                false,
                "每条采样器轨最多读多少个**不同**样本文件 (去重后); 缺省 1024, `0` = 不限。与 `yeban_load_sfz` 的同名上限同一份解析实现与缺省值。逐轨记账 (已解码 PCM 跨轨共享)。截断如实进 `data.sources[].sampler` 的记账字段, 预算在 `data.request.sampleLimits` 回显",
            ),
            param(
                "maxSampleFrames",
                "integer",
                false,
                "每条采样器轨最多读入多少帧 (合计, 按解码后规模算); 缺省 134217728, `0` = 不限。缺省取 2^27: 过单文件字节门的文件必过本门, 本门只约束同轨多文件合计。⚠ 采样器轨没有正弦回退: 被本门截断的音区就是数字静音, 截断如实记账在 `data.sources[].sampler`。逐轨记账; 已解码 PCM 由一次渲染内的共享池复用",
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
            param(
                "commitMessage",
                "string",
                false,
                "合并提交信息; 缺省 = `\"merge proposal {proposalId}\"` [ADR-0029] (建议显式书写, 但不再必填 —— 代码口径 `plan_merge` 本来就接受缺省)",
            ),
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
        summary: "读某条自动化泳道的点与值 (唯一求值入口) 并写入一个点 (Op, 可逆)。⭐ 给 `point` 时**本调用直接提交**到当前文档 (`OpOrigin::McpEdit`): ⛔ 不创建提案、没有 `proposalId`、不需要 `yeban_merge_proposal`; 成功响应 `data.applied` 为 `true`。⭐ 遮蔽: 有自动化值时用自动化值, 静态值被遮蔽 (不叠加); 无自动化值时退回静态值。⚠ 序列化形状: 工程文件里泳道的 `points` 是「点 id → 点对象」的**对象映射** (⛔ 不是数组) 且是必需字段; 本工具的**读响应**则把点报成 `lane.points[]` 数组 —— 两种形状不要互相套用",
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
                "自动化目标变体名: TrackVolume | TrackPan | SendGain | DeviceParam | Macro | MidiCc (与 project.json 同一份词汇表; 不接受别名)。固有值域: `TrackVolume`/`SendGain` 为 [-60, +12] dB, `TrackPan` 为 [-1, 1], `Macro` 为 [0, 1], `MidiCc` 点值为 0..=127; `DeviceParam` 无固有值域 (由目标设备参数自身决定)",
            ),
            param("edgeId", "string", false, "SendGain 必需: 路由边 EntityId"),
            param(
                "controller",
                "integer",
                false,
                "MidiCc 必需: MIDI 控制器号 0..=127 (128..=255 由模型对账拒绝, >255 形状拒绝); 点值域为 0..=127",
            ),
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
            param(
                "paramName",
                "string",
                false,
                "DeviceParam: 按参数名寻址 (与 paramIndex 二选一或并存; 并存必须指向同一个参数, 不一致响亮拒绝; 名字不存在时报错并列出该设备可用名全集)。落盘目标仍是下标形态; 成功响应以 resolvedParamName 回显落点规范名",
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
        summary: "把活跃工程经 yeban-midi 的共享 SMF 映射导出成字节 (base64 回传, 只读不落盘)。`note.syllable` (非空) 真的写成 `FF 05` 歌词元事件, 条数是响应里的 `data.lyrics`; ⚠ `phonemes` 没有 SMF 载体 ⇒ 只在工程真带 phonemes 时进 `data.unsupported`/`data.unsupportedCounts` (计数 = 带 phonemes 的音符数)",
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
        summary: "从磁盘读取 Standard MIDI File (.mid) 解析并导入进指定的空 MIDI 片段, 采纳单值头元数据并原子提交到撤销历史。响应 `data` 逐条回报采纳结果: `adoptedTempoBpm` / `adoptedTimeSignature` / `adoptedTempoMap` / `adoptedTimeSignatureMap` / `adoptedKeySignature` / `adoptedTrackName` / `adoptedTitle` / `adoptedCopyright`; 未采纳的一律为 `null`, 原因在 `data.losses.aux_events`。多轨拆分 (`splitTracks:true`) 时另报 `createdTracks`, 新建轨自动连 master (单位增益) 即可出声。歌词五桶 `data.lyricCounts`: `read` / `attached` / `dropped` (恒有 read == attached + dropped) + `nonUtf8` / `empty`",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ProjectState,
        params: &[
            param("path", "string", true, "源标准 MIDI 文件 (.mid) 磁盘路径"),
            param(
                "trackId",
                "string",
                false,
                "可选的目标音轨 EntityId (26 字符 ULID)；缺省由 destination 选择第一条**带 MIDI 片段**的非 master 音轨（与 app CLI 的 --import-midi 同一条规则；音频片段不会成为目标）",
            ),
            param(
                "clipId",
                "string",
                false,
                "可选的目标片段 EntityId (26 字符 ULID)；缺省取目标音轨上的第一个 **MIDI** 摆放（音频摆放跳过）；该摆放的片段必须是空的 MIDI 片段（音频片段 ⇒ 拒绝，非空 MIDI ⇒ CONFLICT）",
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
        summary: "装载一份 SFZ v2 乐器 (走 yeban-sfz 的唯一解析实现) 并返回区域数与会话内 instrumentId。`data.silentRegionCount` 是引擎摊平口径的静音区域数 (缺样本 + 内建静音), `builtinSilentRegionCount` 是其中的 *silence 子集。⭐ 样本文件读不到时 status 仍是 success, 但 `data.warnings[]` 点名 `sfzSamplesUnreadable` —— 采样器轨没有正弦回退, 绑轨即整轨静音。⭐ CC 门控普查 `data.ccGateCensus`: 生效种子下不可发声的区域分计, 非 0 时 `warnings[]` 另点名 `sfzCcGatedSilence` (需注入 MidiCc 泳道校准)。⭐ 样本被预算上限跳过时 `data.sampleTruncation` 结构化记账 + `warnings[]` 点名 `sfzSampleBudgetTruncated`。可选 `initialCc` 并入同一种子表播种 (覆盖文件 `set_ccN`)。⭐ SFZ 规范的 23 个 `fil_type` 值全部可装载; 本仓尚未实现的 8 个类型在 `data.unsupportedFilterTypes[]` 点名, 这些区域滤波直通",
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
                "磁盘上的 .sfz 文件路径 (空串/目录/不存在 ⇒ `IO_ERROR` 或 `INVALID_PARAMETER_RANGE`; 联集里没有专门的 SFZ 错误码, 且不许发明)。同一份内容已登记 ⇒ 复用登记项, 响应回报**登记时**的路径 (本次路径只用于读文件算内容摘要, 不改变登记项)",
            ),
            param(
                "sampleHz",
                "number",
                false,
                "样本侧采样率 (Hz); 只进报告 (响应里的 `sampleHz` 回显, dryRun 预览同样回显), 本仓不读采样文件、也不计算播放速率比; 缺省 48000。同一份内容已登记 ⇒ 复用登记项, 本次的取值不生效 (回报登记时的值)",
            ),
            param(
                "maxSampleFiles",
                "integer",
                false,
                "本次装载最多读多少个**不同**样本文件 (去重后); 缺省 1024, `0` = 不限。真实大库一件乐器就有上百份不同样本, 缺省太小会把音区/力度层砍掉 —— 采样器轨没有正弦回退, 砍掉的音区就是数字静音。截断响亮报告 (`sampleTruncation` 记账 + `sfzSampleBudgetTruncated` 警告 + 逐条 `sampleFailures`), 预算在响应 `sampleLimits` 回显",
            ),
            param(
                "maxSampleFrames",
                "integer",
                false,
                "本次装载最多读入多少帧 (所有文件合计, 按**解码后**规模算); 缺省 134217728, `0` = 不限。旧缺省 8000000 帧只够 181 s 单声道, 而 VSCO-2-CE 的 FHornSus 一件就要 1266 万帧 ⇒ 读数 `regions=29 samples=17 silent=12`。缺省于 2026-11-18 升到 2^27 (= 256 MiB 单文件门以 16-bit 单声道计的帧数), 任何过单文件门的文件必过本门, 本门只约束同轨多文件合计",
            ),
            param(
                "initialCc",
                "object",
                false,
                "用户初始 CC (控制器号 → 值, 如 `{\"1\": 64}`): 与文件 `<control> set_ccN` 并入同一张 CC 种子表 (用户值覆盖文件值), 装载时播种。控制器号与值合法域均为 0..=127, 越界响亮拒绝、⛔ 不钳位。给了它时 `data.ccGateCensus` 按生效种子算, 响应回显 `data.initialCc`",
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
                "整段统一的力度覆盖 (0..=127): 给了 ⇒ 每个音符改用这个力度（区域选择 matches_velocity 与增益都随之改变）; 不给 ⇒ 用工程里每个音符自己的力度",
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
                "只在**活跃工程已排程的 MIDI 音符**里筛出这一个音高 (0..=127); 不给 = 不过滤。⛔ 它不是单音试听器: 工程里没有该音高的音符 ⇒ 仍写出合法 WAV 但全部为零帧, 回执会带 `notesEmpty.reason`",
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
        summary: "列出当前**未合并**的提案 (只读): 每条给 `proposalId` / `branch` / `kind` / `title` / `status` / `opCount` / `opKinds` 等 (键全集见 outputSchema)。没有未合并的提案 ⇒ `proposals: []` + 一句 `note` (⛔ 不报错)。⭐ 未合并提案随工程容器落盘: 保存后重开工程 (或进程重启后重新打开) 会恢复它们, `proposalId` 不变、可继续 merge / reject。用它在提案还没 merge 时重新找到 `proposalId`",
        scope: Scope::AppAdmin,
        // 只读：读 `domain.proposals`，不改工程、不落盘（`Plan::ListProposals` 的 `apply` 原样回传）。
        side_effect: SideEffect::ReadOnly,
        params: &[param(
            "kind",
            "string",
            false,
            "只看某一类提案: `section` / `notes` / `macro` / `plugin` (T-FIX-419: `yeban_load_plugin` 的提案类别); 其余取值 ⇒ `INVALID_PARAMETER_RANGE` (⛔ 不静默返回全部)",
        )],
        errors: &[ErrorCode::InvalidParameterRange],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-SCENE-LAUNCH",
        name: "yeban_launch_scene",
        summary: "把一次**量化场景启动**交给宿主: `sceneId` 必须是当前工程里的场景; 成功响应的 `data.queued` 只代表**已交给宿主** (宿主在量化点调用引擎), ⛔ 不代表已生效; `data.applied` 恒为 `false`。生效读数用 `yeban_query_engine_state` 的 `engine.sceneLaunchesApplied`。⚠ 独立 stdio 进程形态不托管引擎 ⇒ 响亮报 `CONFLICT` (`reason:\"engineUnavailable\"`), ⛔ 不静默成功",
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
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-EXPORT-STEMS",
        name: "yeban_export_stems",
        summary: "一次调用把活跃工程的**每条源节点轨**弹成一个独立的 32-bit IEEE float WAV 分轨 (母带那条 24-bit 整数 + TPDF 路径不变)。实现 = 克隆工程 → 只留目标轨可闻 → 复用母带渲染器 ⇒ 代价是 **N + 1** 次整工程长度渲染。每条 stem: 无抖动、不重定目标响度、`normalize` 恒 false (保轨间电平关系)。文件名由轨名清洗生成, 同一工程两次导出逐字节同名。⭐ 试听状态 (mute/solo) **不参与**批次选取, 每条如实回报 `sourceMute`/`sourceSolo`。⚠ 目标文件已在盘上 ⇒ 整批拒绝 (`CONFLICT` + `conflictingPaths`), 不隐式改名不覆盖; 主总线不出文件 (在 `data.skippedTracks[]` 点名); 无通路/无内容的轨同样点名。⚠ 写盘中途失败是响亮的 `IO_ERROR` (带 `completedStems`/`failedTrackId`/`orphanedFiles`/`resumeFrom`)。某轨峰值超过 0 dBFS 时该轨回执带 `peakWarning` (float 文件不削波, 但转 24-bit 会硬削平)。成功时在 `outDir` 写 `stems-manifest.json` 供核对与续跑; `trackIds` 给空数组 ⇒ `INVALID_PARAMETER_RANGE` (省略才是全部)。`dryRun` 只给测量读数, 一个字节都不写；同轨多台通道条设备遮挡时该轨行条件带 `stripWarning` (`ignoredStripDevices`)。",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::Disk,
        params: &[
            param(
                "outDir",
                "string",
                true,
                "分轨落盘的目录。不存在时在**写盘时**创建 (`validate` 只读不建, `dryRun` 不创建); 已存在但不是目录 ⇒ `INVALID_PARAMETER_RANGE` 带 `notADirectory`",
            ),
            param(
                "trackIds",
                "array",
                false,
                "可选: 只导这些轨 (26 字符 ULID 数组, 顺序无关, 重复会被去重)。缺省 = 工程里每一条源节点轨。⚠ 点名**不改变**帧数基线 (公共帧数仍来自整工程基线渲染) ⇒ 子集与全集的每条 stem 逐字节相同; 未知身份 ⇒ `TRACK_NOT_FOUND` 带 `unknownTrackIds`",
            ),
        ],
        errors: &[
            ErrorCode::NoActiveProject,
            ErrorCode::InvalidParameterRange,
            ErrorCode::Conflict,
            ErrorCode::TrackNotFound,
            ErrorCode::RenderFailed,
            ErrorCode::IoError,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-LIST-INSTRUMENTS",
        name: "yeban_list_instruments",
        summary: "列出**本会话已装载**的 SFZ 乐器。响应 `data.instruments[]`: 每条给 `instrumentId` / `path` / `regions` / `silentRegionCount` / `builtinSilentRegionCount` / `sha256` / `sampleSource` 等装载读数; 空 registry ⇒ `instruments: []` + 一句 `note` (⛔ 不报错)。⚠ registry 是**会话运行态** (不落盘): 打开/关闭工程或重启进程后为空, `instrumentId` 只能当本会话句柄。装乐器走 `yeban_load_sfz`; 绑轨走 `yeban_edit_notes` 的 `insertDevice` (sampler)。⭐ 目录模式: 给 `directory` 时改扫磁盘目录 (⛔ 不查 registry、⛔ 不装载): 递归列出每件 `.sfz` 的区域数与样本在缺计数, 解析失败的条目带 `parseError` 点名, 根目录有 `manifest.json` 时附逐文件对账; 目录不存在 ⇒ `IO_ERROR`; 空目录 ⇒ `count: 0` + `note` (⛔ 不报错)。⭐ 装前发现面: includeEffectParams:true 或 effectGroup:<id> 时同一响应另给 effectParams[] (规范名/别名/单位/值域/缺省/分组/consumed) 与 effectGroups[] —— 一次调用拿全表, ⛔ 不用翻 schema 的 oneOf 标题",
        scope: Scope::AppAdmin,
        // 只读：枚举 `domain.sfz_instruments` 或扫磁盘目录，不改工程、不装载、不落盘。
        side_effect: SideEffect::ReadOnly,
        params: &[
            // ⚠ 必须与 `schemas/mcp-tools.schema.json` 的 `$defs.yeban_list_instruments` 逐字段一致
            // （判据 `tests/contract.rs::extension_argument_constraints_match_the_registry`
            // 双向核对属性集合 / 类型 / 必填集合）。
            param(
                "directory",
                "string",
                false,
                "可选: 目录模式的根目录（磁盘路径）；给出时递归扫描该目录的 SFZ 与 manifest，不查会话 registry；不存在/不是目录 ⇒ `IO_ERROR`；不给 = 会话模式（既有行为）",
            ),
            param(
                "includeEffectParams",
                "boolean",
                false,
                "可选: true 时在同一次会话模式响应里返回引擎认的**全部效果器参数**（`data.effectParams[]`: 规范名 / canonicalName / aliases / unit / min / max / default / doc / group / consumed）与分组表 `data.effectGroups[]`；数据逐条取自 effect_params.rs::EFFECT_GROUPS（与 insertDevice 校验同一份词表）。consumed:true = 引擎真的消费；未列出的名字只存储不生效（由 insertDevice 的 warnings 点名）。与 directory 同给 ⇒ INVALID_PARAMETER_RANGE",
            ),
            param(
                "effectGroup",
                "string",
                false,
                "可选: 只返回某个器件分组的参数（id 逐字取自 effect_params.rs::EFFECT_GROUPS，如 reverb / eq / channelStrip）；未知分组 ⇒ INVALID_PARAMETER_RANGE（unknownEffectGroup，带 availableGroups）。给出时隐含 includeEffectParams:true",
            ),
        ],
        // 会话模式没有业务失败面（空 registry 是合法状态）；目录模式的
        // 路径失败走 `IO_ERROR`、空串走 `InvalidParameterRange`。
        errors: &[ErrorCode::InvalidParameterRange, ErrorCode::IoError],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-SCAN-PLUGINS",
        name: "yeban_scan_plugins",
        summary: "枚举本机可用的 VST3 插件: 扫描 `YEBAN_VST3_PATHS` 给出的目录与平台标准路径, 另加 `paths` 给出的额外目录; 每个候选都尝试打开、枚举类、逐个实例化探针, 失败逐个记原因、不中断扫描、不进可用列表。响应 `data.plugins[]` 每条给 path / className / classId / kind / usable / error (失败时)。只读: 不改工程、不落盘。扫描只证明插件能打开、能实例化、能通过验证激活, 不证明它在真实工程里出声",
        scope: Scope::AppAdmin,
        // 只读枚举：Host::scan 只读目录、打开库、实例化探针，不写工程、不落盘。
        side_effect: SideEffect::ReadOnly,
        params: &[
            // ⚠ 必须与 `schemas/mcp-tools.schema.json` 的 `$defs.yeban_scan_plugins` 逐字段一致
            // （判据 `tests/contract.rs::extension_argument_constraints_match_the_registry`
            // 双向核对属性集合 / 类型 / 必填集合）。
            array_param(
                "paths",
                false,
                "额外扫描目录（数组，逐项为路径字符串）；不给则只扫环境变量与平台标准路径；不存在的目录静默跳过，不报错",
                crate::tools::string_items,
            ),
        ],
        // 扫描本身没有业务失败面（坏插件是数据，不是错误）；仍声明
        // `InvalidParameterRange`：契约的实参形状校验（多余键等）走这个码
        // —— 判据要求每个工具至少声明一个 D25 联集内的码，⛔ 留空。
        errors: &[ErrorCode::InvalidParameterRange],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-LOAD-PLUGIN",
        name: "yeban_load_plugin",
        summary: "验证加载一个 VST3 插件并经提案路径把它挂到指定轨道上: 先用宿主打开 `pluginPath`、按 `pluginId` 指名类 (不给则按 `kind` 取首个同类)、做验证激活, 失败则整个调用失败、不写提案; 成功后以缺省提案挂一台外部设备 (追加到该轨 devices 末尾)。`kind` 缺省 `instrument` (挂 ExternalInstrument); `kind:\"effect\"` 挂 ExternalEffect。缺省建提案; `autoMerge` 为真时当场落 main, 与 `yeban_edit_notes` 同一条提案纪律。带绑定但加载失败的轨在渲染时响亮失败。测试桩 `stub:sine`/`stub:gain` 可用 (仅测试夹具)",
        scope: Scope::AppAdmin,
        // 提案路径：建提案（ProjectState），autoMerge 时同一调用内合并；⛔ 不直写 main。
        side_effect: SideEffect::ProjectState,
        params: &[
            // ⚠ 必须与 `schemas/mcp-tools.schema.json` 的 `$defs.yeban_load_plugin` 逐字段一致
            // （判据 `tests/contract.rs::extension_argument_constraints_match_the_registry`
            // 双向核对属性集合 / 类型 / 必填集合）。
            param(
                "trackId",
                "string",
                true,
                "目标音轨 EntityId（26 字符 ULID）；设备追加到该轨 devices 末尾；音轨不存在 ⇒ `TRACK_NOT_FOUND`",
            ),
            param(
                "pluginPath",
                "string",
                true,
                "插件路径：.vst3 包路径、散动态库路径，或测试桩 stub:sine（仅测试夹具）；打不开 ⇒ `IO_ERROR`",
            ),
            param(
                "pluginId",
                "string",
                false,
                "类名或类 CID 十六进制；不给则按 `kind` 取首个同类；指名类不存在或 kind 不对 ⇒ `INVALID_PARAMETER_RANGE`",
            ),
            param(
                "kind",
                "string",
                false,
                "插件种类：`instrument`（缺省，挂 ExternalInstrument）或 `effect`（挂 ExternalEffect）；其它值 ⇒ `INVALID_PARAMETER_RANGE`",
            ),
            param(
                "autoMerge",
                "boolean",
                false,
                "[ADR-0029]：缺省 false = 只建提案；true = 同一次调用内背靠背执行既有的合并实现（与 `yeban_edit_notes` 同一条纪律）",
            ),
            param(
                "mergeCommitMessage",
                "string",
                false,
                "[ADR-0029]：`autoMerge` 合并提交的信息，只随 `autoMerge:true` 生效；缺省同手动合并的既有缺省",
            ),
        ],
        errors: &[
            ErrorCode::TrackNotFound,
            ErrorCode::IoError,
            ErrorCode::InvalidParameterRange,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-ARA-BIND",
        name: "yeban_ara_bind",
        summary: "把一段音频 wav 绑定到 ARA 插件 (ARA2 宿主原型): plan 只做实参校验 + wav 解码, apply 打开 ARA 会话并以 sessionId 登记进会话 registry。绑定不改工程、不落盘 (会话运行态; 打开/关闭工程整体重置); sessionId 只在本会话有效。pluginId 不给则取首个 ARA 类; tempoBpm 缺省 120; loopIterations 缺省 1; muted 缺省 false。MIDI 轨不能挂 ARA (构造期响亮失败, 须先渲染成音频)",
        scope: Scope::AppAdmin,
        // 只动会话态 registry（与 load_sfz 同款口径）：不改工程、不落盘。
        side_effect: SideEffect::ReadOnly,
        params: &[
            // ⚠ 必须与 `schemas/mcp-tools.schema.json` 的 `$defs.yeban_ara_bind` 逐字段一致
            // （判据 `tests/contract.rs::extension_argument_constraints_match_the_registry`
            // 双向核对属性集合 / 类型 / 必填集合）。
            param(
                "pluginPath",
                "string",
                true,
                "插件路径：.vst3 包路径或散动态库路径；打不开 ⇒ `IO_ERROR`",
            ),
            param(
                "pluginId",
                "string",
                false,
                "ARA 类名；不给则取首个 ARA 类；指名类不存在或无同名 Audio Module 类 ⇒ `INVALID_PARAMETER_RANGE`",
            ),
            param(
                "wavPath",
                "string",
                true,
                "片段 wav 路径；解码失败 ⇒ `IO_ERROR`",
            ),
            param(
                "tempoBpm",
                "number",
                false,
                "片段速度（BPM）；缺省 120；须大于 0",
            ),
            param(
                "loopIterations",
                "integer",
                false,
                "循环迭代数；缺省 1；须 ≥ 1 且 ≤ 4096（上限与宿主 MAX_LOOP_ITERATIONS 同源）",
            ),
            param(
                "muted",
                "boolean",
                false,
                "静音摆放；缺省 false；静音时不建 playback region",
            ),
        ],
        errors: &[ErrorCode::InvalidParameterRange, ErrorCode::IoError],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-ARA-QUERY",
        name: "yeban_ara_query",
        summary: "ARA 会话的分析状态与内容查询（只读）：回 sessionId、factory 名与版本、analysisAvailable、regionCount、head/tail 秒数、notes（可用时）与 notesError（不可用时）。会话不存在 ⇒ `INVALID_PARAMETER_RANGE` + `araSessionNotFound`；内容不可用时不报错（状态查询工具报状态，不把未完成当失败 —— PitchNet 无头流程从不发布 Notes 内容）。",
        scope: Scope::AppAdmin,
        // 纯只读：读会话 registry 与插件内容查询，不写任何状态。
        side_effect: SideEffect::ReadOnly,
        params: &[
            // ⚠ 必须与 `schemas/mcp-tools.schema.json` 的 `$defs.yeban_ara_query` 逐字段一致。
            param(
                "sessionId",
                "string",
                true,
                "会话 id（`yeban_ara_bind` 的响应）；不存在 ⇒ `INVALID_PARAMETER_RANGE`",
            ),
        ],
        errors: &[ErrorCode::InvalidParameterRange],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-ARA-RENDER",
        name: "yeban_ara_render",
        summary: "把已绑定 ARA 会话离线渲染成 WAV (ARA2 宿主原型): 按块调处理器渲染整段, 编成 32-bit float WAV 后原子落盘。回帧数、峰值、非零帧数、字节数与文档 blob 的字节数与 SHA-256 (blob 字节本身不进工程 JSON)。渲染仅支持 44100Hz 片段, 非 44100 片段响亮失败、不静默变调或截断。会话不存在 ⇒ `INVALID_PARAMETER_RANGE` + `araSessionNotFound`",
        scope: Scope::AppAdmin,
        // 写盘（渲染 WAV）；不改工程文档。
        side_effect: SideEffect::Disk,
        params: &[
            // ⚠ 必须与 `schemas/mcp-tools.schema.json` 的 `$defs.yeban_ara_render` 逐字段一致。
            param(
                "sessionId",
                "string",
                true,
                "会话 id（`yeban_ara_bind` 的响应）；不存在 ⇒ `INVALID_PARAMETER_RANGE`",
            ),
            param(
                "outPath",
                "string",
                true,
                "输出 wav 路径；父目录不存在 ⇒ `IO_ERROR`",
            ),
            param(
                "blockFrames",
                "integer",
                false,
                "渲染块大小；缺省 1024；须在 1..=4096 内",
            ),
        ],
        errors: &[ErrorCode::InvalidParameterRange, ErrorCode::IoError],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-ARA-UNBIND",
        name: "yeban_ara_unbind",
        summary: "解绑并释放一个已绑定的 ARA 会话: 从会话 registry 移除该 sessionId, 插件实例的资源随之释放。只动会话态, 不改工程、不落盘; 解绑后该 id 立即失效。会话数达上限时用本工具回收旧会话即可继续绑定。会话不存在 ⇒ `INVALID_PARAMETER_RANGE` + `araSessionNotFound`",
        scope: Scope::AppAdmin,
        // 只动会话态 registry（与 ara_bind 同款）：不改工程、不落盘。
        side_effect: SideEffect::ReadOnly,
        params: &[
            // ⚠ 必须与 `schemas/mcp-tools.schema.json` 的 `$defs.yeban_ara_unbind` 逐字段一致。
            param(
                "sessionId",
                "string",
                true,
                "要解绑的会话 id（`yeban_ara_bind` 的响应）；不存在 ⇒ `INVALID_PARAMETER_RANGE`",
            ),
        ],
        errors: &[ErrorCode::InvalidParameterRange],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-SVS-SUBMIT",
        name: "synthesize_singing",
        summary: "把乐谱提交到伶人 SVS 服务 (只提交、不含推理): `parts` 数组每项 `{label, score, voice_id, model?, variant?, takes?, seeds?, options?}`; `score = {bpm, language?, notes: [{lyric, midi, start_s, dur_s, tie?, phoneme?}]}`, `lyric` 为 `''` 或 `'<SP>'` 表示休止; `takes` 缺省 1 (1..=8)。返回 `{jobId, status, partsTotal, takesTotal, serviceUrl}`; 提交失败直接报 `IO_ERROR`, ⛔ 不返回空任务。配置: 服务地址 = 实参 `service_url` > 环境变量 `YEBAN_SVS_URL` > 缺省 `http://127.0.0.1:8000`; API key **只**从环境变量 `YEBAN_SVS_API_KEY` 读 (⛔ 不是工具参数、不进工程文件、不出现在响应里)。只支持 `http` (`https` 在配置解析时响亮拒绝)。`dryRun` 只校验参数、不提交。⚠ 工具名不带 `yeban_` 前缀 (跨线契约名)",
        scope: Scope::AppAdmin,
        // 外部副作用（服务端建任务）：不改工程、不落盘，但 `dryRun` 必须短路它。
        side_effect: SideEffect::External,
        params: &[
            param(
                "parts",
                "array",
                true,
                "乐谱 part 数组 (至少 1 项), 每项 `{label, score, voice_id, model?, variant?, takes?, seeds?, options?}`; `score = {bpm, language?, notes: [{lyric, midi, start_s, dur_s, tie?, phoneme?}]}`; `lyric` 为 `''` 或 `'<SP>'` = 休止; `takes` 缺省 1 (1..=8); 未知 `model` 由服务端响亮拒绝",
            ),
            param(
                "service_url",
                "string",
                false,
                "SVS 服务地址（只支持 http）；缺省读 `YEBAN_SVS_URL` 环境变量，再缺省 `http://127.0.0.1:8000`",
            ),
        ],
        errors: &[ErrorCode::InvalidParameterRange, ErrorCode::IoError],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-SVS-JOB",
        name: "svs_job",
        summary: "查 SVS 任务状态、取音频与回执 (`want` 缺省 `status`)。`status`: 返回任务进度字段, `failed` 时 `error` 原文照带。`receipt`: 返回服务端回执全文。`audio`: 按 `part` + `take` 取一遍 wav (⛔ 不替用户混音); 取前先查状态: 任务失败 ⇒ `IO_ERROR` + 服务端 `error` 原文 (⛔ 不返回静音 wav), 未完成 ⇒ `IO_ERROR` (`jobNotCompleted`)。取回的 wav 写稳定暂存路径; 给了 `trackId` 就按放置语义落轨 (走 `yeban_import_audio` 同一机制: `license` 此时必填且须在白名单内, `gainDb` 缺省 0.0, `startTick` 缺省 0), 不给只返回暂存路径。放置语义: SoulX 按乐谱时钟贴轨 (回执实测偏差只作校验, 超 100 ms 进 `deviationWarnings`, ⛔ 不自动移位); VocalRender 按回执实测起点贴; 休止条目放置时忽略。取回的 wav 作为**普通音频片段**走现有音频片段机制。⚠ 工具名不带 `yeban_` 前缀 (跨线契约名)",
        scope: Scope::AppAdmin,
        // 按最坏情况：`want=audio` + `trackId` 时改工程（与 `yeban_edit_automation` 同款口径）。
        side_effect: SideEffect::ProjectState,
        params: &[
            param(
                "job_id",
                "string",
                true,
                "SVS 任务 id（服务端生成的 16 位十六进制；本层只接受字母 / 数字 / `_` / `-`，防 URL 路径与暂存文件名越界）",
            ),
            param(
                "want",
                "string",
                false,
                "`status` | `audio` | `receipt`，缺省 `status`",
            ),
            param(
                "part",
                "string",
                false,
                "`want=audio` 时必填：part label（服务端允许多遍歧义回落，但本工具⛔ 不替调用方猜是哪一遍）",
            ),
            param(
                "take",
                "integer",
                false,
                "`want=audio` 时必填：遍号（0 起）",
            ),
            param(
                "trackId",
                "string",
                false,
                "`want=audio` 时可选：给了就把取回的 wav 落到该轨（26 字符 ULID）；不给只返回暂存路径",
            ),
            param(
                "startTick",
                "integer",
                false,
                "落轨时乐谱起点 tick（缺省 0；只在给了 `trackId` 时有效；SoulX 按此贴轨，VocalRender 在此基础上按实测首音节移位）",
            ),
            bounded_param(
                "license",
                "string",
                false,
                "落轨时必填：资产许可标识（须在 `assets/samples/manifest.json` 白名单内，如 `CC0-1.0`；与 `yeban_import_audio` 同一条 MUST-GATE-014 判定，本工具不发明默认值）",
                Some(1),
                None,
            ),
            param(
                "gainDb",
                "number",
                false,
                "落轨时片段增益（dB，缺省 0.0；只在给了 `trackId` 时有效）",
            ),
            param(
                "service_url",
                "string",
                false,
                "SVS 服务地址（只支持 http）；缺省读 `YEBAN_SVS_URL` 环境变量，再缺省 `http://127.0.0.1:8000`",
            ),
        ],
        errors: &[
            ErrorCode::InvalidParameterRange,
            ErrorCode::IoError,
            ErrorCode::EntityNotFound,
            ErrorCode::NoActiveProject,
            ErrorCode::TrackNotFound,
            ErrorCode::Conflict,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-LIST-PRESETS",
        name: "yeban_list_presets",
        summary: "列出可用的预制: 系统预制 (发行表) + 用户预制库。只读, 不要求活跃工程; `kind` 缺省 `all`; `instrument` 按乐器 slug 过滤。响应 `presets[]` 每条给 `id` / `name` / `kind` / `instrument` / `scope` / `default` (该乐器的默认预制) 等; 坏文件逐条进 `skipped`, 不拦整个列表",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ReadOnly,
        params: &[
            param(
                "kind",
                "string",
                false,
                "`system` | `user` | `all`（缺省 `all`）；其它值 ⇒ `INVALID_PARAMETER_RANGE`",
            ),
            param(
                "instrument",
                "string",
                false,
                "乐器 slug 过滤（如 `salamander-grand`）；不给则不过滤",
            ),
        ],
        errors: &[ErrorCode::InvalidParameterRange],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-APPLY-PRESET",
        name: "yeban_apply_preset",
        summary: "把一份预制应用到指定音轨 (提案路径): 通道条与四件落成 `Preset: …` 设备 (确定性身份), 发送只改已有边的增益 (不增边、不删边、不建 Aux), 宏只调已有的宏位 (超出跳过并报告), 乐器绑定变化时写 `SetTrackSampler`, 最后写 `track.preset`。`categories` 缺省全开 (instrument/strip/effects/sends/macros), 未知类名 ⇒ `INVALID_PARAMETER_RANGE`。跳过的发送/宏写进提案描述。缺省建提案; `autoMerge` 为真时当场落 main, 与 `yeban_edit_notes` 同一条提案纪律。预制查不到 ⇒ `ENTITY_NOT_FOUND`。预制带 `cc` 段且本次含 instrument 类别时, 落地后按 `yeban_load_sfz` 同一条路登记乐器并播种初始 CC, 回执带 `initialCcSeeded`",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ProjectState,
        params: &[
            param(
                "trackId",
                "string",
                true,
                "目标音轨 EntityId（26 字符 ULID）",
            ),
            param(
                "presetId",
                "string",
                true,
                "预制标识（`sys:<乐器>:<名>` 或 `user:<ulid>`）；查不到 ⇒ `ENTITY_NOT_FOUND`",
            ),
            array_param(
                "categories",
                false,
                "应用类别（缺省 = 全部五类）：`instrument` / `strip` / `effects` / `sends` / `macros`；未知类名 ⇒ `INVALID_PARAMETER_RANGE`",
                crate::tools::string_items,
            ),
            param(
                INCLUDE_OPS_PARAM,
                "boolean",
                false,
                "是否回传完整 op 载荷: 缺省 false = 只回结构化字段 (willCreate + opCount/opKinds), true = 额外回传逐条 ops (审查用途) [BASELINE-006]",
            ),
            param(
                "autoMerge",
                "boolean",
                false,
                "[ADR-0029]: 缺省 false = 只建提案 (`pendingMerge:true`)。true = 同一次调用内**背靠背执行既有的** `yeban_merge_proposal` 合并实现, 响应 = 提案回执 ⊕ 合并回执 (`autoMerged:true` / `pendingMerge:false` / `projectUnchanged:false` / `merge:{...}`)。第二步失败 ⇒ 提案**保持 Open**, 错误 `data` 带 `proposalCreated:true` + `proposalId` + `proposalRemainsOpen:true` (⛔ 不假装回滚)",
            ),
            param(
                "mergeCommitMessage",
                "string",
                false,
                "[ADR-0029]: `autoMerge` 合并提交的信息, 只随 `autoMerge:true` 生效; 缺省同手动合并的既有缺省 `\\\"merge proposal {id}\\\"`",
            ),
        ],
        errors: &[
            ErrorCode::TrackNotFound,
            ErrorCode::EntityNotFound,
            ErrorCode::InvalidParameterRange,
            ErrorCode::NoActiveProject,
            ErrorCode::Conflict,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-SAVE-PRESET",
        name: "yeban_save_preset",
        summary: "把当前轨状态另存为一份用户预制: 从轨上的 `Preset:` 设备收通道条与四件、从 `SendToAux` 边收发送、取轨宏数组, 写进用户预制库, 标识 `user:<ulid>`。不改工程、不动提交图谱; 要进工程包须保存工程。名重复 ⇒ `CONFLICT` (`overwrite:true` 才覆盖)。单份超 64 KiB ⇒ 拒绝保存。轨未绑定已知乐器 ⇒ `INVALID_PARAMETER_RANGE`。本会话登记过该轨乐器装载且带 `initialCc` 时, 初始 CC 一并捕获进预制的 `cc` 段",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::Disk,
        params: &[
            param(
                "trackId",
                "string",
                true,
                "来源音轨 EntityId（26 字符 ULID）",
            ),
            // ⭐ D46 收口（实现侧）：与 `yeban_import_audio` 的 `name` 同一条纪律 ——
            // 契约 `minLength: 1`，空名字必须在参数层被拒（`build_save_preset` 在
            // plan 层同样拒空名；两道口径一致）。
            bounded_param(
                "name",
                "string",
                true,
                "预制显示名（非空，用户库内不得重名，除非 `overwrite: true`）",
                Some(1),
                None,
            ),
            param(
                "scope",
                "string",
                false,
                "`instrument`（乐器调优，缺省）| `track`（整轨快照，另记 mute/solo）；其它值 ⇒ `INVALID_PARAMETER_RANGE`",
            ),
            param(
                "overwrite",
                "boolean",
                false,
                "同名已存在时是否覆盖同一标识的文件（缺省 false = 响亮拒绝，不静默覆盖）",
            ),
        ],
        errors: &[
            ErrorCode::TrackNotFound,
            ErrorCode::Conflict,
            ErrorCode::InvalidParameterRange,
            ErrorCode::NoActiveProject,
            ErrorCode::IoError,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-PRESET-BYPASS",
        name: "yeban_preset_bypass",
        summary: "开关一条轨的预制旁通 (提案路径): `bypassed:true` = 恢复 `previous` 快照 (通道条、四件、发送、宏), 只旁通处理、不换乐器; 没有快照 ⇒ 响亮拒绝。`bypassed:false` = 按当前 `preset_id` 重新应用该预制 (带 `cc` 段时重播初始 CC 种子)。缺省建提案; `autoMerge` 为真时当场落 main",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ProjectState,
        params: &[
            param(
                "trackId",
                "string",
                true,
                "目标音轨 EntityId（26 字符 ULID）",
            ),
            param(
                "bypassed",
                "boolean",
                true,
                "true = 旁通（恢复应用前快照，含乐器绑定：应用前没绑采样器就清掉），false = 取消旁通（重新应用当前预制）",
            ),
            param(
                INCLUDE_OPS_PARAM,
                "boolean",
                false,
                "是否回传完整 op 载荷: 缺省 false = 只回结构化字段 (willCreate + opCount/opKinds), true = 额外回传逐条 ops (审查用途) [BASELINE-006]",
            ),
            param(
                "autoMerge",
                "boolean",
                false,
                "[ADR-0029]: 缺省 false = 只建提案 (`pendingMerge:true`)。true = 同一次调用内**背靠背执行既有的** `yeban_merge_proposal` 合并实现, 响应 = 提案回执 ⊕ 合并回执 (`autoMerged:true` / `pendingMerge:false` / `projectUnchanged:false` / `merge:{...}`)。第二步失败 ⇒ 提案**保持 Open**, 错误 `data` 带 `proposalCreated:true` + `proposalId` + `proposalRemainsOpen:true` (⛔ 不假装回滚)",
            ),
            param(
                "mergeCommitMessage",
                "string",
                false,
                "[ADR-0029]: `autoMerge` 合并提交的信息, 只随 `autoMerge:true` 生效; 缺省同手动合并的既有缺省 `\\\"merge proposal {id}\\\"`",
            ),
        ],
        errors: &[
            ErrorCode::TrackNotFound,
            ErrorCode::EntityNotFound,
            ErrorCode::InvalidParameterRange,
            ErrorCode::NoActiveProject,
            ErrorCode::Conflict,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-SONG-OVERVIEW",
        name: "get_song_overview",
        summary: "返回整曲多层可视化总览图的小节级列式 JSON 与自动异常诊断摘要 (字段集见 outputSchema; 归一化指标保留 2 位小数, 响度与频段能量保留 1 位小数)。`resolution` 缺省 `\"bar\"` (一小节一行), 给 `\"beat\"` 则一拍一行; `profile` = 曲风预设 (缺省 `default`), `expected_levels` = 段落期望张力等级表; `format` 缺省 `\"json\"`, 给 `\"html\"` 返回单文件自包含 HTML 报告字符串 (同一份数)。共享缓存: 同一会话内同工程同参数的重复调用只算一次, JSON 响应带 `cacheHit`。只读: 不改工程、不落盘; 需要活跃工程与宿主注入的 provider (未注入 ⇒ `CONFLICT` + `providerUnavailable`, ⛔ 不返回空总览)。`scoreOnly:true` 只要谱面那一份: 跳过整曲渲染与音频面分析、不要求 provider。⚠ 工具名不带 `yeban_` 前缀 (跨线契约名)",
        scope: Scope::AppAdmin,
        // 只读：取 provider 已算好的总览做映射，不改工程、不落盘。
        side_effect: SideEffect::ReadOnly,
        params: &[
            // ⚠ 必须与 `schemas/mcp-tools.schema.json` 的
            // `definitions.ExtensionToolArguments.$defs.get_song_overview` 逐字段一致
            // （判据 `tests/contract.rs::extension_argument_constraints_match_the_registry`
            // 双向核对属性集合 / 类型 / 必填集合）。
            param(
                "resolution",
                "string",
                false,
                "时间分辨率：\"bar\"（缺省，一小节一行）或 \"beat\"（一拍一行）；其它值 ⇒ `INVALID_PARAMETER_RANGE`",
            ),
            param(
                "profile",
                "string",
                false,
                "曲风预设档: 缺省 `default` (均衡流行摇滚); `orchestral` (管弦/原声叙事: 音区与和声权重调高); `electronic` (电子/律动: 响度与密度权重调高)。方差自适应: 某特征全曲标准差 σ ≤ 0.05 时该权重衰减 50% 并按比例重分配。其它值 ⇒ `INVALID_PARAMETER_RANGE` (`reason:\"unknownProfile\"`, `accepted:[\"default\",\"orchestral\",\"electronic\"]`)",
            ),
            param(
                "expected_levels",
                "object",
                false,
                "段落期望张力等级表: 对象, 键 = 段落名 (如 `Chorus`), 值 = 期望张力等级 1–5 的整数 (例 `{\"Chorus\": 5}`)。实际张力偏离期望带过远的小节进 `envelope_deviation` 异常。空对象 `{}` 合法 = 无标注。非对象 ⇒ 协议层拒绝; 对象内非整数值/越界等级 ⇒ `INVALID_PARAMETER_RANGE` (`reason:\"invalidExpectedLevels\"`)。等级只活在本次分析请求里, 不写工程",
            ),
            param(
                "include_png_path",
                "boolean",
                false,
                "是否返回 PNG 路径（缺省 false）。Phase 1 只接受参数、不产 PNG：给 true 时响应带 `pngPath: null` + 一句 Phase 2 说明（设计稿 §8.1）",
            ),
            param(
                "format",
                "string",
                false,
                "响应形态: `\"json\"` (缺省) 或 `\"html\"` (单文件自包含 HTML 报告字符串, 与 JSON 同一份数); 其它值 ⇒ `INVALID_PARAMETER_RANGE` (`reason:\"unknownFormat\"`); 与 `include_png_path:true` 同给 ⇒ 同码 (`reason:\"includePngPathRequiresJson\"`)",
            ),
            param(
                "scoreOnly",
                "boolean",
                false,
                "只算谱面总览 (缺省 false, 与 app CLI 的 `--overview-score-only` 同一模式): true 时跳过整曲渲染与音频面分析, 只由谱面算出同构总览 —— 不要求宿主注入 provider。响应带 `scoreOnly:true`, `cacheHit` 恒 `false` (不走共享缓存)。`profile`/`expected_levels` 本模式下不参与计算 (仍被校验, 非法值照样响亮拒绝)",
            ),
        ],
        errors: &[
            ErrorCode::NoActiveProject,
            ErrorCode::InvalidParameterRange,
            ErrorCode::Conflict,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-OTCP-CORRECT",
        name: "yeban_correct_pitch",
        summary: "把一个音频片段提交给用户自备的 OpenTune API 分支构建做离线调性 snap (`ADR-0070`; 单 clip; 只改音高、不改时值)。提交后立即回 `{job_id, status:\"running\", server, notes_before, receipt_path}`, 校正在后台线程跑; 用 `yeban_correct_pitch_job` 轮询。源音频须是单声道 (立体声 ⇒ `otcpStereoUnsupported`); 轨道已挂 OpenTune 外部效果绑定 ⇒ `otcpDoubleCorrection` (二选一, ⛔ 不双重校正)。`mode` 缺省 `snapNotes`, v1 只执行它 (其余给了即响亮拒绝)。`autoMerge` 缺省 false = 只出提案; `predictOnly` (缺省 false) = 真跑一遍只返回预测音符表, ⛔ 不导出、不交付。失败时 `RENDER_FAILED` 大声失败、什么都不写、⛔ 不回落干声",
        scope: Scope::AppAdmin,
        // 外部副作用（启动 OpenTune 子进程建任务）：不改工程、不落盘，但 `dryRun` 必须短路它。
        side_effect: SideEffect::External,
        params: &[
            // ⚠ 必须与 `schemas/mcp-tools.schema.json` 的
            // `definitions.ExtensionToolArguments.$defs.yeban_correct_pitch` 逐字段一致
            // （判据 `tests/contract.rs::extension_argument_constraints_match_the_registry`
            // 双向核对属性集合 / 类型 / 必填集合）。
            param("trackId", "string", true, "目标音轨 id（26 字符 ULID）"),
            param(
                "clipId",
                "string",
                true,
                "目标音频片段 id（26 字符 ULID，`clip_pool` 的键）",
            ),
            param(
                "keyRoot",
                "integer",
                true,
                "目标调性根音 0–11（C=0）；越界 ⇒ `INVALID_PARAMETER_RANGE`",
            ),
            param(
                "scale",
                "string",
                true,
                "音阶名：`major` | `minor` | `chromatic` | `harmonicMinor` | `dorian` | `mixolydian` | `pentatonicMajor` | `pentatonicMinor`；其它值 ⇒ `INVALID_PARAMETER_RANGE`",
            ),
            param(
                "mode",
                "string",
                false,
                "校正模式：`snapNotes`（缺省，v1 只执行它）；`regenerate` / `notesOnly` 须先探针验证，给了即 `INVALID_PARAMETER_RANGE`",
            ),
            param("tuningHz", "number", false, "A4 频率 Hz（缺省 440）"),
            param(
                "autoMerge",
                "boolean",
                false,
                "交付时是否立即合并提案（缺省 false：只出提案；true 时当场合并，像 `yeban_load_plugin`）",
            ),
            param(
                "predictOnly",
                "boolean",
                false,
                "只预测不交付（缺省 false）：真跑一遍导入→F0→调音→读谱，只返回预测音符表，不导出、不交付；与通用 `dryRun` 不同（`dryRun` 只校验参数、不提交）",
            ),
            param(
                "noteOps",
                "array",
                false,
                "`content.editNotes` 的 op 列表（JSON 原样透传；空 = 不做；干跑时只预测）",
            ),
        ],
        errors: &[
            ErrorCode::InvalidParameterRange,
            ErrorCode::IoError,
            ErrorCode::NoActiveProject,
            ErrorCode::TrackNotFound,
            ErrorCode::ClipNotFound,
            ErrorCode::Conflict,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-OTCP-JOB",
        name: "yeban_correct_pitch_job",
        summary: "查音高校正任务状态; 成功后经提案换资产。`running`: 回 `{job_id, status, notes_before, receipt_path}`。`succeeded`: 回音符表与测量读数 (`measurement` / `asset` / `server`), 同时按提交时的 `autoMerge` 出提案 (false 只出提案, true 当场合并), 响应带 `proposal` 块。`failed`: 回 `{job_id, status, error:{code, detail, …}}`, `code` 是固定 `reason`; `RENDER_FAILED` 时什么都没写、源片段原样。任务收据住在控制根的任务目录下, 进程重启后仍可查",
        scope: Scope::AppAdmin,
        // 按最坏情况：成功后交付走提案路径改工程（与 `svs_job` 同款口径）。
        side_effect: SideEffect::ProjectState,
        params: &[param(
            "job_id",
            "string",
            true,
            "校正任务 id（提交时返回的 `job_id`）",
        )],
        errors: &[
            ErrorCode::InvalidParameterRange,
            ErrorCode::IoError,
            ErrorCode::EntityNotFound,
            ErrorCode::NoActiveProject,
            ErrorCode::Conflict,
            ErrorCode::RenderFailed,
            ErrorCode::Busy,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-OTCP-NOTES",
        name: "yeban_correct_pitch_notes",
        summary: "读某片段最近一次校正的检测音符表 (只读)。给 `trackId` + `clipId` 取该片段最近一次校正任务的音符表; 给 `contentId` 取该 OpenTune 内容的任务音符表。返回 `{contentId, tuningHz, totalCount, notes:[…]}`。⚠ 这是 OpenTune **检测到**的音符, 不是夜半乐谱 (探针会过分割, 含短碎片); 无任务时报 `otcpNotesUnavailable`, ⛔ 不返回空表。v1 不现算音符 (须已有任务收据)",
        scope: Scope::AppAdmin,
        // 只读：读任务收据里的音符表，不改工程、不落盘、不启动服务端。
        side_effect: SideEffect::ReadOnly,
        params: &[
            param(
                "trackId",
                "string",
                false,
                "片段所属音轨 id（26 字符 ULID；与 `clipId` 成对给）",
            ),
            param(
                "clipId",
                "string",
                false,
                "目标片段 id（26 字符 ULID，`clip_pool` 的键；与 `trackId` 成对给）",
            ),
            param(
                "contentId",
                "string",
                false,
                "OpenTune 内容 id（在途任务的 `contentId`；与 `trackId`/`clipId` 二选一）",
            ),
        ],
        errors: &[
            ErrorCode::InvalidParameterRange,
            ErrorCode::IoError,
            ErrorCode::EntityNotFound,
            ErrorCode::NoActiveProject,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-MOVE-TRACK",
        name: "yeban_move_track",
        summary: "移动音轨在显示顺序里的位置（`Op::MoveTrack` 的 MCP 出口；提案路径，`autoMerge` 可一次调用落 main）。`newIndex` 是显示顺序里的绝对位置（`display_track_ids`，不含主总线）；`oldIndex` 由服务端从当前文档读，调用方不传（陈旧的旧值在模型层必被拒绝）。等于现位置即 `INVALID_PARAMETER_RANGE`（无操作不建提案），主总线即 `CONFLICT`，越界即 `INDEX_OUT_OF_BOUNDS`。",
        scope: Scope::AppAdmin,
        side_effect: SideEffect::ProjectState,
        params: &[
            param(
                "trackId",
                "string",
                true,
                "被移动的音轨 EntityId（26 字符 ULID）",
            ),
            param(
                "newIndex",
                "integer",
                true,
                "移动后在显示顺序里的位置（0..显示轨数，不含主总线）；等于现位置即 INVALID_PARAMETER_RANGE，越界即 INDEX_OUT_OF_BOUNDS",
            ),
            param(
                INCLUDE_OPS_PARAM,
                "boolean",
                false,
                "是否回传完整 op 载荷: 缺省 false = 只回结构化字段 (willCreate + opCount/opKinds), true = 额外回传逐条 ops (审查用途) [BASELINE-006]",
            ),
            param(
                "autoMerge",
                "boolean",
                false,
                "[ADR-0029]: 缺省 false = 只建提案 (`pendingMerge:true`)。true = 同一次调用内**背靠背执行既有的** `yeban_merge_proposal` 合并实现, 响应 = 提案回执 ⊕ 合并回执 (`autoMerged:true` / `pendingMerge:false` / `projectUnchanged:false` / `merge:{...}`)。第二步失败 ⇒ 提案**保持 Open**, 错误 `data` 带 `proposalCreated:true` + `proposalId` + `proposalRemainsOpen:true` (⛔ 不假装回滚)",
            ),
            param(
                "mergeCommitMessage",
                "string",
                false,
                "[ADR-0029]: `autoMerge` 合并提交的信息, 只随 `autoMerge:true` 生效; 缺省同手动合并的既有缺省 `\\\"merge proposal {id}\\\"`",
            ),
        ],
        errors: &[
            ErrorCode::TrackNotFound,
            ErrorCode::IndexOutOfBounds,
            ErrorCode::InvalidParameterRange,
            ErrorCode::Conflict,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-COMPARE-SNAPSHOT",
        name: "compare_song_snapshots",
        summary: "返回两版 Songmap 快照的 A/B Diff: 逐小节差值 (Δ = B − A, 小数口径与 `get_song_overview` 同) + 新增/删除小节 + 异常与重复条目的增减 + 结构摘要变化。快照 = `get_song_overview` 的 `data` 原样: 先取修改前快照 A, 改谱后再取当前版 B, 两份原样传进来。只比小节级: 任一快照含拍级行 ⇒ `INVALID_PARAMETER_RANGE` (`reason:\"snapshotMismatch\"`); 两版 `schema_version`/`band_definitions_hz` 不等 ⇒ 同码; 快照形状非法 ⇒ 同码 (`reason:\"invalidSnapshot\"`)。只读: 纯函数, 不要求活跃工程、不改工程、不落盘。⚠ 工具名不带 `yeban_` 前缀 (跨线契约名)",
        scope: Scope::AppAdmin,
        // 只读：对两份入参快照做纯算术，不改工程、不落盘。
        side_effect: SideEffect::ReadOnly,
        params: &[
            // ⚠ 必须与 `schemas/mcp-tools.schema.json` 的
            // `definitions.ExtensionToolArguments.$defs.compare_song_snapshots` 逐字段一致
            // （判据 `tests/contract.rs::extension_argument_constraints_match_the_registry`
            // 双向核对属性集合 / 类型 / 必填集合）。
            param(
                "snapshot_a",
                "object",
                true,
                "修改前的快照：`get_song_overview` 的 `data`（`SongOverview`，schema 1.1）原样；非对象 ⇒ 协议层 `-32602`（注册表类型先拒），形状非法 ⇒ `INVALID_PARAMETER_RANGE`（`reason: \"invalidSnapshot\"`）",
            ),
            param(
                "snapshot_b",
                "object",
                true,
                "当前版的快照：`get_song_overview` 的 `data`（`SongOverview`，schema 1.1）原样；与 `snapshot_a` 同口径（版本 / 频段定义 / 小节级）",
            ),
        ],
        errors: &[ErrorCode::InvalidParameterRange],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-PHASE-ALIGN-AUTO",
        name: "yeban_auto_align_tracks",
        summary: "自动相位对齐: 对两条源轨 solo 渲染后做归一化互相关, 测目标轨相对基准轨的延时 (`lagSamples`, 正 = 目标轨晚到), 再把补偿经提案路径写进已有的手动延时机制 (只延时先到的轨, 正延时, 无前视; 极性反转时同时翻目标轨 `invert_polarity`)。`apply` 缺省 true = 建提案; false = 只测量。置信度 |ρ| < 0.3 响亮拒绝应用 (`lowCorrelation`); 新延时超出 ±4800 采样响亮拒绝 (`compensationOutOfRange`), ⛔ 不部分应用。`autoMerge` 缺省 false; true 时交付当场合并。测量由宿主注入的 provider 提供 (未注入 ⇒ `CONFLICT` + `providerUnavailable`, ⛔ 不返回假测量)",
        scope: Scope::AppAdmin,
        // 提案路径：建提案（ProjectState），autoMerge 时同一调用内合并；⛔ 不直写 main。
        side_effect: SideEffect::ProjectState,
        params: &[
            // ⚠ 必须与 `schemas/mcp-tools.schema.json` 的 `$defs.yeban_auto_align_tracks` 逐字段一致
            // （判据 `tests/contract.rs::extension_argument_constraints_match_the_registry`
            // 双向核对属性集合 / 类型 / 必填集合）。
            param(
                "referenceTrackId",
                "string",
                true,
                "基准轨 EntityId（26 字符 ULID）；时间锚点：补偿只延时先到的轨，基准轨是被对齐的参照",
            ),
            param(
                "targetTrackId",
                "string",
                true,
                "目标轨 EntityId（26 字符 ULID）；须与基准轨不同，相同 ⇒ `INVALID_PARAMETER_RANGE` + `sameReferenceAndTarget`；音轨不存在 ⇒ `TRACK_NOT_FOUND`",
            ),
            param(
                "maxLagMs",
                "number",
                false,
                "最大搜索延时（毫秒，缺省 50；须 > 0 且折算 ≤ 4800 采样；越界 ⇒ `INVALID_PARAMETER_RANGE` + `maxLagOutOfRange`）",
            ),
            param(
                "apply",
                "boolean",
                false,
                "缺省 true = 有补偿时建提案；false = 只测量、不建提案（响应仍带完整测量字段，`applied: false`）",
            ),
            param(
                "autoMerge",
                "boolean",
                false,
                "[ADR-0029]：缺省 false = 只建提案；true = 同一次调用内背靠背执行既有的合并实现（与 `yeban_edit_notes` 同一条纪律）",
            ),
            param(
                "mergeCommitMessage",
                "string",
                false,
                "[ADR-0029]：`autoMerge` 合并提交的信息，只随 `autoMerge:true` 生效；缺省同手动合并的既有缺省",
            ),
            param(
                INCLUDE_OPS_PARAM,
                "boolean",
                false,
                "是否回传完整 op 载荷: 缺省 false = 只回结构化字段 (willCreate + opCount/opKinds), true = 额外回传逐条 ops (审查用途) [BASELINE-006]",
            ),
        ],
        errors: &[
            ErrorCode::NoActiveProject,
            ErrorCode::InvalidParameterRange,
            ErrorCode::TrackNotFound,
            ErrorCode::Conflict,
            ErrorCode::RenderFailed,
        ],
    },
    ToolSpec {
        spec_id: "MCP-TOOL-EXT-DRUM-GENERATE",
        name: "yeban_generate_drum_pattern",
        summary: "鼓机一期: 按风格预置生成鼓型并落成目标轨上的 MIDI 片段 (提案路径, `autoMerge` 可一次调用落 main)。风格 = 出厂 14 预置 (trap/drill/phonk/lofi/boombap/house/techno/edm/pop/rock/jazz/funk/rnb/hiphop); 三旋钮 complexity (密度/鬼音) / intensity (力度) / swingPermille (摇摆, 缺省取风格定值) + seed (同种子同参数逐字节可复现)。`trackId` 缺省 ⇒ 新建鼓轨; 目标轨尚无音源时按风格族装默认鼓组, 已有音源则沿用。ratchet/hat roll 在音符事件层展开。风格不存在 ⇒ `STYLE_NOT_FOUND` (回执带 `availableStyles`)",
        scope: Scope::AppAdmin,
        // 提案路径：建提案（ProjectState），autoMerge 时同一调用内合并；⛔ 不直写 main。
        side_effect: SideEffect::ProjectState,
        params: &[
            // ⚠ 必须与 `schemas/mcp-tools.schema.json` 的 `$defs.yeban_generate_drum_pattern` 逐字段一致
            // （判据 `tests/contract.rs::extension_argument_constraints_match_the_registry`
            // 双向核对属性集合 / 类型 / 必填集合）。
            param(
                "style",
                "string",
                true,
                "风格预置 id（出厂 14 个：trap/drill/phonk/lofi/boombap/house/techno/edm/pop/rock/jazz/funk/rnb/hiphop）；不存在 ⇒ STYLE_NOT_FOUND",
            ),
            param(
                "trackId",
                "string",
                false,
                "目标音轨 EntityId（26 字符 ULID）；缺省 = 新建一条鼓轨（含到主总线的路由）；音轨不存在 ⇒ TRACK_NOT_FOUND",
            ),
            param(
                "complexity",
                "integer",
                false,
                "复杂度 0..=100（缺省 50）：<34 只出核心层、34..=66 加标准层、>=67 全层 + 军鼓鬼音（种子驱动）；越界 ⇒ INVALID_PARAMETER_RANGE",
            ),
            param(
                "intensity",
                "integer",
                false,
                "强度 0..=100（缺省 80）：力度缩放（100 = 预置原力度，0 = 压到 55%）；越界 ⇒ INVALID_PARAMETER_RANGE",
            ),
            param(
                "swingPermille",
                "integer",
                false,
                "摇摆千分比 500..=1000（500 = 平直）；缺省 = 风格预置定值；越界 ⇒ INVALID_PARAMETER_RANGE",
            ),
            param(
                "seed",
                "integer",
                false,
                "生成种子（缺省 0）：同种子 + 同参数 ⇒ 片段逐字节一致；驱动鬼音层",
            ),
            param(
                "bars",
                "integer",
                false,
                "生成小节数 1..=16（缺省 4；预置 pattern 循环取用）；越界 ⇒ INVALID_PARAMETER_RANGE",
            ),
            param(
                "fill",
                "boolean",
                false,
                "缺省 true = 最后一小节落 fill（风格带 fill 时）；false = 全程主 pattern",
            ),
            param("startTick", "integer", false, "摆放起始 tick（缺省 0）"),
            param(
                INCLUDE_OPS_PARAM,
                "boolean",
                false,
                "是否回传完整 op 载荷: 缺省 false = 只回结构化字段 (willCreate + opCount/opKinds), true = 额外回传逐条 ops (审查用途) [BASELINE-006]",
            ),
            param(
                "autoMerge",
                "boolean",
                false,
                "[ADR-0029]: 缺省 false = 逐字节不变 (只建提案, `pendingMerge:true`)。true = 同一次调用内背靠背执行既有的 `yeban_merge_proposal` 合并实现, 响应 = 提案回执 ⊕ 合并回执",
            ),
            param(
                "mergeCommitMessage",
                "string",
                false,
                "[ADR-0029]: `autoMerge` 合并提交的信息, 只随 `autoMerge:true` 生效; 缺省同手动合并的既有缺省 `\\\"merge proposal {id}\\\"`",
            ),
        ],
        errors: &[
            ErrorCode::NoActiveProject,
            ErrorCode::StyleNotFound,
            ErrorCode::TrackNotFound,
            ErrorCode::InvalidParameterRange,
            ErrorCode::Conflict,
        ],
    },
];

/// 按名查找工具（`const` 数组上的线性查找；45 个元素，无需哈希表）。
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

    /// 往成功回执的 `data` 里写入 [`COMMIT_MODEL_KEY`]（`ADR-0032` **D2** 的唯一注入点）。
    ///
    /// 三条口径：
    /// 1. **只作用于 `Success`** —— 失败回执没有"这次属于哪种提交模型"可说（它一位都没施加），
    ///    原样返回；
    /// 2. `data` 不是对象时**不强行造对象**（契约要求成功 `data` 恒为 object，那是别的判据
    ///    守着的事；本函数⛔ 不为了让一个键存在而篡改形状）；
    /// 3. **覆盖式写入** —— 生产代码里⛔ 任何人都不该手写这个键，真出现覆盖就是漂移，
    ///    由 `tests/commit_model.rs` 的取值判据把它抓出来。
    #[must_use]
    pub fn with_commit_model(self, model: &'static str) -> Self {
        match self {
            Self::Success { data } => {
                let mut data = data;
                if let Some(object) = data.as_object_mut() {
                    object.insert(COMMIT_MODEL_KEY.to_owned(), Value::from(model));
                }
                Self::Success { data }
            }
            failure @ Self::Failure { .. } => failure,
        }
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
    fn render_master_sample_rate_is_optional_and_schema_follows() {
        let spec = tool("yeban_render_master").expect("render_master 已注册");
        let sample_rate = spec.param("sampleRate").expect("sampleRate 参数仍在");
        assert!(
            !sample_rate.required,
            "sampleRate 必须可选（缺省 = 工程采样率）"
        );
        assert!(
            sample_rate.doc.contains("工程采样率"),
            "参数文案必须写明缺省 = 工程采样率: {}",
            sample_rate.doc
        );
        let minimal = serde_json::json!({"format": "wav"});
        spec.validate_arguments(minimal.as_object().expect("object"))
            .expect("省略 sampleRate 必须通过参数校验");
        let required: Vec<&str> = spec
            .required_params()
            .iter()
            .map(|param| param.name)
            .collect();
        assert!(
            !required.contains(&"sampleRate"),
            "required 清单不应再含 sampleRate: {required:?}"
        );
        // tools/list 的 inputSchema 由注册表派生，required 必须同步。
        let schema = spec.input_schema();
        let schema_required = schema["required"].as_array().expect("required 数组");
        assert!(
            !schema_required.iter().any(|value| value == "sampleRate"),
            "inputSchema.required 不应再含 sampleRate: {schema_required:?}"
        );
    }

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
        // ⭐ 规格名例外（跨线契约名，不带 `yeban_` 前缀）：`synthesize_singing` /
        // `svs_job` 的名字由 SVS Phase 1 规格定（服务文档与调用方都按此名），
        // 是 `yeban_` 族之外的第一个工具族；`get_song_overview` 的名字由整曲
        // 总览设计稿 §8.1/§12 定（MCP 工具名就叫它），是第二个；
        // `compare_song_snapshots` 的名字由同一设计稿 §11 定，是第三个。
        // 例外**只此四条**：新增工具仍须 `yeban_` 前缀；对齐守卫的 `TABLE_TOOL_RE`
        // 同步放了这四条（见该脚本注释）。
        const NON_YEBAN_PREFIX_NAMES: [&str; 4] = [
            "synthesize_singing",
            "svs_job",
            "get_song_overview",
            "compare_song_snapshots",
        ];
        for spec in &TOOLS {
            assert!(
                spec.name.starts_with("yeban_") || NON_YEBAN_PREFIX_NAMES.contains(&spec.name),
                "工具名必须是 yeban_*（SVS 两条与 get_song_overview 除外）: {}",
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
                "yeban_list_proposals",
                // 审计 `N13`：会话 SFZ registry 枚举 —— 只读 `domain.sfz_instruments`。
                "yeban_list_instruments",
                // 插件扫描：只枚举路径并逐个实例化探针，不改工程、不落盘 ⇒ 读-only。
                "yeban_scan_plugins",
                // ARA 绑定：只动**会话态** registry（与 load_sfz 同款口径），
                // 打开插件是 apply 的事，但既不改工程也不落盘 ⇒ 读-only。
                "yeban_ara_bind",
                // ARA 查询：纯只读（读会话 registry 与插件内容查询）。
                "yeban_ara_query",
                // ARA 解绑：只释放会话态 registry 里的插件实例（与 bind 同款口径），
                // 不改工程、不落盘 ⇒ 读-only。
                "yeban_ara_unbind",
                // 原生预制清单：只读发行表 + 用户库，不改工程、不落盘 ⇒ 读-only。
                "yeban_list_presets",
                // 整曲总览：只取 provider 已算好的总览做映射，不改工程、不落盘 ⇒ 只读。
                "get_song_overview",
                // OTCP 音符表：只读任务收据（不派生服务端、不改工程、不落盘）⇒ 只读。
                "yeban_correct_pitch_notes",
                // 快照对比：对两份入参快照做纯算术，不读工程、不落盘 ⇒ 只读。
                "compare_song_snapshots"
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
