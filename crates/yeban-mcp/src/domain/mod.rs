//! 领域实现层：十个 Intent 工具的**真实执行** [MCP-TOOL-001..010, ROAD-M4-003/004]。
//!
//! ## 结构：`plan`（只读）与 `apply`（可变）**分家**
//!
//! ```text
//! plan(&Domain, &ToolCall)  ->  Result<Plan, Fault>     // 只读：参数 + 领域合法性校验
//! apply(&mut Domain, Plan)  ->  Result<ToolResponse>    // 可变：真的做事
//! execute(&mut Domain, &ToolCall) = plan + apply
//! ```
//!
//! 这个切分不是风格问题，它是 `dryRun` 判据的**类型系统级**保证：
//! [`plan`] 拿到的是 `&Domain`（共享引用），**物理上不可能**改任何状态；
//! `dryRun` 走的就是 [`plan`] ⇒ "`dryRun` 不改状态"不靠自觉，靠借用检查器。
//! 运行期判据（`tests/tools_e2e.rs`）再独立地把它证明一遍：
//! 前后 `YebanProjectV1` 序列化**逐字节相同** + `CommitGraph` 提交数不变。
//!
//! ## 逆操作的唯一来源
//!
//! 本层**从不**自己写逆操作。所有会改工程的东西都表达成
//! [`yeban_model::Op`]，撤销一律由 [`Op::invert`] / [`Op::apply_inverse`] 提供
//! （判据 `proposal_ops_are_reversible_through_the_model_inverse`）。
//! 在 MCP 层再写一套 `invert` 就会有两份会漂移的真相。
//!
//! ## 模块地图
//!
//! | 模块 | 职责 |
//! | :--- | :--- |
//! | [`error`] | 领域失败 ↔ 契约错误码的**唯一**映射；实现级状况走 JSON-RPC |
//! | [`store`] | 工程文件读取、**原子落盘**（临时文件 + `fsync` + `rename`）、`.yeban.lock` |
//! | [`view`] | `yeban_query_project` 的字段选择器与分页 |
//! | [`notes`] | `yeban_edit_notes` 的 `NoteOp` → `Op` 编译 + 发声数校验 |
//! | [`section`] | `yeban_propose_section` 的**契约适配层**（`BuildFault` → `Fault`、`Op` → JSON） |
//! | [`section_build`] | `yeban_propose_section` 的**零重依赖**骨架生成器（段落 / 片段池条目 / 摆放 / 声部连接），本机可单独验证 |
//! | [`macros`] | `yeban_set_macro` 的宏与级联自动化展开 |
//! | [`proposal`] | 提案记录（Musical PR 的可追溯性） |
//! | [`render`] | `yeban_render_master` 的参数校验 **+ 真渲染**（`yeban-render` 接线、原子落盘） |
//! | [`render_math`] | 渲染的**零第三方依赖**纯逻辑（tick→帧、归一化、包络、日历），本机可单独验证 |
//! | [`render_clip_math`] | 音频片段装配的**零第三方依赖**纯逻辑（帧落位、增益合成、声道矩阵、重采样判定、延迟裁剪、魔数嗅探），本机可单独验证 |
//! | [`ids`] | 确定性夹具身份（让 `dryRun` 预览与真调用逐字节相同） |
//! | [`automation`] | `yeban_edit_automation`：泳道读（唯一求值入口）+ 写一个点（`Op`，可逆） |
//! | [`automation_audit`] | **零依赖**审计：生产代码里不许有第二份自动化求值，本机可单独验证 |
//! | [`engine_state`] | `yeban_query_engine_state`：设备链 + 引擎/会话读数（只读） |
//! | [`project_create`] | `yeban_open_project` 的 **`create: true`** 分支：从零建一个可渲染、可配器的工程（主总线在路由图里） |
//! | [`import_audio`] | `yeban_import_audio`：`yeban-decode` + `PcmBudget` + `Op::AddClip` |
//! | [`export_midi`] | `yeban_export_midi`：`yeban-midi` 共享映射 → SMF 字节（base64 回传，只读） |
//! | [`extension_pure`] | 三个扩展工具的**零第三方依赖**纯逻辑（词表 / 来源二选一 / 确定性标签），本机可单独验证 |
//! | [`extension_audit`] | **零依赖**文本守卫：写路径 / `dryRun` 入口 / 错误码词表 / 无孤儿模块，本机可单独验证 |

pub mod automation;
pub mod automation_audit;
pub mod diagnostics;
pub mod engine_state;
pub mod error;
pub mod export_midi;
pub mod extension_audit;
pub mod extension_pure;
pub mod ids;
pub mod import_audio;
pub mod lock;
pub mod macros;
pub mod notes;
pub mod project_create;
pub mod proposal;
pub mod render;
pub mod render_clip_math;
pub mod render_math;
pub mod section;
pub mod section_build;
pub mod store;
pub mod view;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use yeban_model::{
    AssetHash, CommitDraft, CommitGraph, EntityId, Op, OpOrigin, SessionRuntimeState, StampedOp,
    YebanProjectV1,
};

use crate::jsonrpc::ErrorObject;
use crate::tools::{ErrorCode, ToolCall, ToolResponse};
use crate::undo_session::{self, CommitRequest, UndoRefusal};

use error::Fault;
use proposal::{Proposal, ProposalDraft, ProposalStatus, draft_ops_value};
use store::AcquiredLock;

/// 主分支名。
/// 主分支名（唯一字面量在 [`crate::undo_session::MAIN_BRANCH`]，这里只是转发）。
pub const MAIN_BRANCH: &str = undo_session::MAIN_BRANCH;

/// 隔离提案分支名前缀（架构 §7.2：`ai/proposal-{ulid}`）。
pub const PROPOSAL_BRANCH_PREFIX: &str = "ai/proposal-";

/// 写进 `OpOrigin::McpProposal` 的代理名。
pub const AGENT_NAME: &str = "yeban-mcp";

/// 一次已经打开的工程（会话态）。
#[derive(Debug)]
struct Active {
    /// 工程文件路径。
    path: PathBuf,
    /// 是否以只读方式打开。
    read_only: bool,
    /// 内存中的权威工程状态。
    project: YebanProjectV1,
    /// 最近一次落盘（或打开）时的内容摘要，用于"未保存标记"。
    saved_digest: String,
    /// 会话 CAS 资产池（`assets/{sha256}` 的**字节**，`BTreeMap` 保证键序确定）。
    ///
    /// 打开工程时由容器里的 `assets/{sha256}` 填充；`open_in_memory` 注入的会话
    /// （夹具 / 测试）没有载荷字节，因此池为空、只有 `project.assets` 那份元数据索引。
    /// 保存时整体交给 `write_project_container` 逐条重算 SHA-256。
    assets: BTreeMap<AssetHash, Vec<u8>>,
    /// 持有的锁（内存会话没有工程文件时为 `None`）。
    ///
    /// 排他写是 [`store::LockGuard`]，共享读是包着共享建议锁的 [`AcquiredLock`] ——
    /// 两者都是 RAII：`Drop` 即释放，进程死亡由内核释放。
    lock: Option<AcquiredLock>,
}

/// 打开一个工程所需的全部会话种子（[`Domain::reset_history`] 的入参）。
///
/// 收成一个结构体而不是 8 个参数：`clippy::too_many_arguments` 是承重的信号，
/// 而不是需要 `allow` 掉的噪音 —— 参数一多就意味着"打开"这件事有太多可选形态。
#[derive(Debug)]
struct SessionSeed {
    /// 工程路径。
    path: PathBuf,
    /// 只读打开。
    read_only: bool,
    /// 权威工程状态。
    project: YebanProjectV1,
    /// 内容摘要（规范化 JSON 的 SHA-256）。
    digest: String,
    /// 已获取的锁（内存注入时为 `None`）。
    lock: Option<AcquiredLock>,
    /// `history.dag` 恢复出的提交图谱；`None` = 从根提交开始的新会话。
    history: Option<CommitGraph>,
    /// 会话 CAS 资产池。
    assets: BTreeMap<AssetHash, Vec<u8>>,
}

/// 领域会话状态：活跃工程 + 提交图谱 + 提案记录 + **撤销会话态** + 注入的时钟
/// + **第 2 层会话运行态** + 宿主注入的引擎镜像。
///
/// **刻意不实现 `Clone`**：它内涵 `.yeban.lock` 的 RAII 守卫与提交图谱，
/// 克隆会产出两个独立的写者。
#[derive(Debug)]
pub struct Domain {
    active: Option<Active>,
    graph: CommitGraph,
    proposals: BTreeMap<EntityId, Proposal>,
    /// 撤销会话态（游标 + 活跃分支）[`crate::undo_session`]。
    ///
    /// **属于会话运行态，不是持久化文档层** [MODEL-ISO-001]：打开 / 关闭工程都会把它
    /// 整体重置，它也不会进 `project.json` 或 `history.dag`（模型层把这点做成了类型事实：
    /// `UndoCursor` 不实现 `Serialize`）。
    undo: undo_session::UndoState,
    /// **第 2 层：会话运行态**（`MODEL-ISO-001`）—— 走带位置 / 播放状态 / 长任务进度 /
    /// 插件进程 / 打开的视窗。
    ///
    /// 为什么不自己造一份：这一层的**模型类型**已经存在
    /// （`yeban_model::SessionRuntimeState`，且**不实现 `Serialize`** ⇒ 结构上不可能
    /// 落盘），MCP 侧再造一个"会话态结构体"就是第二份真相。
    ///
    /// ## 关于 `session.undo_cursor`（这里**不是**第二个游标）
    ///
    /// 撤销游标的权威是 [`Domain::undo`]（`undo_session::UndoState`，它内部持有的
    /// 就是模型 `UndoCursor`）。`SessionRuntimeState::undo_cursor` 是模型层对**同一件事**
    /// 的读法，因此 [`Domain::sync_session`] 在**唯一**的可变入口
    /// [`apply`] 结束时把它单向同步成权威值 —— 两处不可能漂移（判据
    /// `session_mirror_never_drifts_from_the_undo_authority` 逐次工具调用核对）。
    session: SessionRuntimeState,
    /// 宿主注入的**引擎读数镜像**（只读快照；见 [`engine_state`] 的模块文档）。
    ///
    /// 本 crate **不依赖** `yeban-engine`（零新增依赖），因此缓冲帧数只能由宿主
    /// （形态 A 的 `yeban-app`）通过 [`Domain::set_engine_readings`] 交进来；
    /// 没有注入时读数是 `null` + `bufferSource: "unavailable"`。
    engine: Option<engine_state::EngineReadings>,
    /// 引擎读数镜像的**单调修订号**（`0` = 从未注入；每次注入 +1）。
    ///
    /// 它就是控制面上的**读数游标**（见 [`engine_state`] 的模块文档）：客户端拿它当
    /// `yeban_query_engine_state` 的 `since`，就能只取没见过的读数。单调性由
    /// [`Domain::set_engine_readings`] 这**一个**写入点保证。
    readings_revision: u64,
    /// 最近 [`engine_state::READINGS_TAIL_CAPACITY`] 条读数（含各自的修订号），**按修订递增**。
    ///
    /// `Vec` 而不是环形缓冲：容量 64，且这里**不在实时线程上**（宿主按控制面节奏注入）。
    /// 与 `engine` 一样是**会话运行态**（`MODEL-ISO-001` 第 2 层）：打开 / 关闭工程会整体重置，
    /// 不进 `project.json` 也不进 `history.dag`。
    readings_tail: Vec<(u64, engine_state::EngineReadings)>,
    now_ms: u64,
    /// **施加修订号**：每施加一个**可能改工程**的计划就 +1（只读计划不推进）。
    ///
    /// 它是**宿主投影刷新的触发口径**（`ROAD-M4-008` 选项 (a)）：形态 A 的
    /// `yeban-app` 把控制面会话当唯一可变权威，界面是它的**投影** —— 投影只在
    /// 这个号前进时重做一次，因此"AI 改模型 ⇒ 界面跟着变"不依赖宿主记住
    /// "刚才那次调用改了没有"（那正是选项 (b) 的弱点）。
    ///
    /// 单调性由 [`apply`] 这**一个**写入点保证（推进口径见 [`Plan::mutates_project`]）；
    /// 与 [`Self::readings_revision`] 是两件事：后者是**引擎读数**的游标，住在会话运行态。
    apply_revision: u64,
}

impl Default for Domain {
    fn default() -> Self {
        Self::new()
    }
}

impl Domain {
    /// 空会话（没有活跃工程，提交图谱为空）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            active: None,
            graph: CommitGraph::new(),
            proposals: BTreeMap::new(),
            undo: undo_session::UndoState::new(AGENT_NAME),
            session: SessionRuntimeState::new(),
            engine: None,
            readings_revision: 0,
            readings_tail: Vec::new(),
            now_ms: 0,
            apply_revision: 0,
        }
    }

    /// **施加修订号**（`0` = 从未施加过任何可能改工程的计划）。
    ///
    /// 宿主用它驱动界面投影的刷新（`ROAD-M4-008` 选项 (a)）：见字段文档。
    #[must_use]
    pub const fn apply_revision(&self) -> u64 {
        self.apply_revision
    }

    /// 会话运行态（只读）[`MODEL-ISO-001` 第 2 层]。
    #[must_use]
    pub const fn session(&self) -> &SessionRuntimeState {
        &self.session
    }

    /// 会话运行态的**注入口**（宿主与判据用：定位播放头、起停走带）。
    pub const fn session_mut(&mut self) -> &mut SessionRuntimeState {
        &mut self.session
    }

    /// 宿主注入的引擎读数镜像（`None` = 没注入，读数如实为 `null`）。
    #[must_use]
    pub const fn engine_readings(&self) -> Option<engine_state::EngineReadings> {
        self.engine
    }

    /// 引擎读数镜像的当前**修订号**（控制面游标；`0` = 从未注入）。
    #[must_use]
    pub const fn readings_revision(&self) -> u64 {
        self.readings_revision
    }

    /// 最近注入的读数（含修订号），**按修订递增**，最多
    /// [`engine_state::READINGS_TAIL_CAPACITY`] 条（游标段从这里取增量）。
    #[must_use]
    pub fn readings_tail(&self) -> &[(u64, engine_state::EngineReadings)] {
        &self.readings_tail
    }

    /// 注入 / 清除引擎读数镜像，并推进**读数修订号**（`[ARCH-UI-002]` 的交付侧）。
    ///
    /// **只对宿主开放**：没有任何工具的 `apply` 会碰它（判据
    /// `tools_never_write_the_engine_mirror` 用"调用前后镜像逐位相同"钉住）。
    ///
    /// 与读数一起推进的还有**有界尾部**：`since` 游标只会拿到**新注入**的读数，
    /// 且窗口之外的游标会被如实标记 `agedOut`（见 [`engine_state`] 的模块文档）。
    /// 修订号只在**注入内容与当前不同**时推进 —— 同一条读数重复注入不制造假更新。
    pub fn set_engine_readings(&mut self, readings: Option<engine_state::EngineReadings>) {
        let changed = self.engine != readings;
        self.engine = readings;
        if !changed {
            return;
        }
        self.readings_revision = self.readings_revision.saturating_add(1);
        if let Some(readings) = readings {
            self.readings_tail.push((self.readings_revision, readings));
            let excess = self
                .readings_tail
                .len()
                .saturating_sub(engine_state::READINGS_TAIL_CAPACITY);
            if excess > 0 {
                self.readings_tail.drain(..excess);
            }
        }
    }

    /// 把模型会话态里那份**撤销游标镜像**同步成权威值（见 [`Domain::session`]）。
    ///
    /// 只在 [`apply`]（唯一会改变状态的入口）结束时调用，因此不存在"某条路径忘了同步"。
    fn sync_session(&mut self) {
        self.session.undo_cursor = self.undo.cursor();
    }

    /// 注入时钟（Unix 毫秒）。
    ///
    /// 模型层刻意不自取时钟（可测试性）；MCP 工具的契约里也没有时钟参数，
    /// 因此时钟由**调用方**注入：形态 B 的二进制在启动时注入一次系统时间，
    /// 判据注入固定值以获得逐字节稳定的提交记录。
    pub const fn set_now_ms(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
    }

    /// 当前注入的时钟。
    #[must_use]
    pub const fn now_ms(&self) -> u64 {
        self.now_ms
    }

    /// 活跃工程（只读）。
    #[must_use]
    pub fn active_project(&self) -> Option<&YebanProjectV1> {
        self.active.as_ref().map(|active| &active.project)
    }

    /// 活跃工程路径。
    #[must_use]
    pub fn active_path(&self) -> Option<&Path> {
        self.active.as_ref().map(|active| active.path.as_path())
    }

    /// 是否以只读方式打开。
    #[must_use]
    pub fn is_read_only(&self) -> bool {
        self.active.as_ref().is_some_and(|active| active.read_only)
    }

    /// 当前持有的锁文件路径（内存会话时为 `None`）。
    ///
    /// 这个访问器同时承担一个结构职责：`Active::lock` 是 RAII 守卫
    /// （`Drop` 释放 `.yeban.lock` 的建议锁），**必须被持有**；通过它读一次，
    /// "锁一直是活的"这件事就有一个可观察的出口，而不是一个只写字段。
    #[must_use]
    pub fn lock_path(&self) -> Option<&Path> {
        self.active
            .as_ref()
            .and_then(|active| active.lock.as_ref())
            .map(|lock| lock.guard.path())
    }

    /// 当前持有的锁模式（内存会话时为 `None`）。
    #[must_use]
    pub fn lock_mode(&self) -> Option<store::LockMode> {
        self.active
            .as_ref()
            .and_then(|active| active.lock.as_ref())
            .map(|lock| lock.guard.mode())
    }

    /// 当前会话**持有的锁**里那份持有者元数据（内存会话时为 `None`）。
    ///
    /// 这是"谁持有这个工程"的**唯一**跨平台可靠来源：它住在守卫（内存）里，
    /// 而不是"现场去读 `.yeban.lock`"。后者在 Windows 上必然失败 —— `LockFileEx`
    /// 是**强制**字节区间锁，持锁期间连本进程的另一个句柄都读不到那个文件
    /// （见 `src/domain/lock.rs` 的平台矩阵与 `docs/ledger/store-container-notes.md`）。
    #[must_use]
    pub fn lock_holder(&self) -> Option<&store::LockMetadata> {
        self.active
            .as_ref()
            .and_then(|active| active.lock.as_ref())
            .and_then(|lock| lock.guard.holder())
    }

    /// 提交总数（`dryRun` "状态未变" 判据的一半）。
    #[must_use]
    pub fn commit_count(&self) -> usize {
        self.graph.commit_count()
    }

    /// 分支总数（含提案分支）。
    #[must_use]
    pub fn branch_count(&self) -> usize {
        self.graph.branch_count()
    }

    /// 提交图谱（只读）。
    #[must_use]
    pub const fn graph(&self) -> &CommitGraph {
        &self.graph
    }

    /// 主分支头。
    #[must_use]
    pub fn main_head(&self) -> Option<EntityId> {
        self.graph
            .branches
            .get(MAIN_BRANCH)
            .map(|branch| branch.head)
    }

    /// **活跃**分支头（撤销之后继续编辑会派生匿名分支，因此活跃头可能不在 `main` 上）。
    ///
    /// 为什么单独给一个入口：`yeban_propose_section` 的 `baseCommit` 与
    /// `yeban_merge_proposal` 的"基线是否移动过"都必须看**活跃**头 ——
    /// 盯着 `main` 会在撤销之后把提案挂到一条只读孤岛上。
    #[must_use]
    pub fn active_head(&self) -> Option<EntityId> {
        self.undo.head(&self.graph).ok()
    }

    /// 活跃分支名。
    #[must_use]
    pub fn active_branch(&self) -> &str {
        self.undo.branch()
    }

    /// 撤销会话态（只读）：可撤销 / 可重做 / 提交数等**模型读数**都在这里。
    #[must_use]
    pub const fn undo_state(&self) -> &undo_session::UndoState {
        &self.undo
    }

    /// 撤销能力的显示态（模型读数）。
    #[must_use]
    pub fn undo_display(&self) -> undo_session::UndoDisplay {
        self.undo.display(&self.graph)
    }

    /// 提案记录。
    #[must_use]
    pub fn proposal(&self, id: &EntityId) -> Option<&Proposal> {
        self.proposals.get(id)
    }

    /// 提案条数。
    #[must_use]
    pub fn proposal_count(&self) -> usize {
        self.proposals.len()
    }

    /// 提案身份清单（`BTreeMap` 键序 = 字典序，确定性）。
    #[must_use]
    pub fn proposal_ids(&self) -> Vec<EntityId> {
        self.proposals.keys().copied().collect()
    }

    /// 会话 CAS 池里的资产键（哈希升序 = `BTreeMap` 键序 = 容器内条目顺序）。
    #[must_use]
    pub fn asset_hashes(&self) -> Vec<AssetHash> {
        self.active
            .as_ref()
            .map(|active| active.assets.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// 会话 CAS 池里的资产数量。
    #[must_use]
    pub fn asset_count(&self) -> usize {
        self.active.as_ref().map_or(0, |active| active.assets.len())
    }

    /// 取一份 CAS 资产的**原始字节**（没有活跃工程或池里没有它时为 `None`）。
    #[must_use]
    pub fn asset(&self, hash: &AssetHash) -> Option<&[u8]> {
        self.active
            .as_ref()
            .and_then(|active| active.assets.get(hash))
            .map(Vec::as_slice)
    }

    /// 把一份资产放进**会话 CAS 池**（`ARCH-SEC-003` 的 `assets/{sha256}`）。
    ///
    /// 内容寻址：哈希由字节**算出**并作为返回值/池键，**不接受**调用方声明的哈希 ——
    /// 否则池里就能存在"条目名与字节不符"的资产，而容器写出时会直接拒绝它，
    /// 把矛盾推迟到保存那一刻才暴露。返回算出的 [`AssetHash`] 供调用方引用。
    ///
    /// 只改内存，不碰文件系统；持久化发生在下一次 `yeban_save_project`。
    ///
    /// # Errors
    ///
    /// 没有活跃工程 → `NO_ACTIVE_PROJECT`。
    pub fn put_asset(&mut self, bytes: Vec<u8>) -> Result<AssetHash, Fault> {
        let hash = AssetHash::of_bytes(&bytes);
        let active = self.active.as_mut().ok_or_else(no_active_project)?;
        active.assets.insert(hash.clone(), bytes);
        Ok(hash)
    }

    /// 当前工程内容的 SHA-256 摘要（没有活跃工程时为 `None`）。
    #[must_use]
    pub fn project_digest(&self) -> Option<String> {
        let active = self.active.as_ref()?;
        let json = store::serialize_project(&active.project).ok()?;
        Some(store::digest_of(json.as_bytes()))
    }

    /// 是否有未保存改动。
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        let Some(active) = self.active.as_ref() else {
            return false;
        };
        self.project_digest()
            .is_none_or(|digest| digest != active.saved_digest)
    }

    /// **不碰文件系统**地注入一个已经打开的工程。
    ///
    /// 两个用途，都是正当的：
    ///
    /// 1. 形态 A（`yeban-app` 内嵌）：应用进程自己持有 `YebanProjectV1`，
    ///    通过这个口子把它交给 MCP 层，而不是让 MCP 去读盘；
    /// 2. 判据与规范样本：需要一个确定性的"已打开"起点。
    ///
    /// # Errors
    ///
    /// 工程未通过 `validate()`，或提交图谱创建失败。
    pub fn open_in_memory(
        &mut self,
        path: impl Into<PathBuf>,
        project: YebanProjectV1,
        read_only: bool,
    ) -> Result<(), Fault> {
        project
            .check_readable()
            .map_err(|error| error::from_model("工程版本门", &error))?;
        project
            .validate()
            .map_err(|error| error::from_model("工程结构校验", &error))?;
        let json = store::serialize_project(&project)?;
        let digest = store::digest_of(json.as_bytes());
        self.reset_history(SessionSeed {
            path: path.into(),
            read_only,
            project,
            digest,
            lock: None,
            history: None,
            assets: BTreeMap::new(),
        })
    }

    /// 重建会话历史（打开工程 = 恢复 `history.dag`，或新会话的根提交）。
    ///
    /// `seed.history` 为 `Some` 时**整体采用**容器里的提交图谱（`ARCH-OPS-002` 的
    /// "编曲时光机"必须跨打开存活）；为 `None` 时按老语义建一条根提交。
    ///
    /// **撤销会话态在这里整体重置** —— 这是"撤销不越过工程打开边界"的落点：
    /// 换一个工程之后，游标与活跃分支都从头开始（判据
    /// `undo_does_not_cross_a_project_open_boundary`）。
    fn reset_history(&mut self, seed: SessionSeed) -> Result<(), Fault> {
        self.graph = CommitGraph::new();
        self.proposals.clear();
        self.undo = undo_session::UndoState::new(AGENT_NAME);
        if let Some(history) = seed.history {
            self.graph = history;
            // 采纳别人的图谱之前先确认它有一条活跃分支（`main`），否则宁可拒绝；
            // 随后按（文档, 图谱）**推导**游标 —— "撤销 → 保存 → 重开"之后
            // 重做栈仍然是对的，而游标一个字节都没落盘 [MODEL-ISO-001]。
            self.undo
                .align_with(&self.graph, &seed.project)
                .map_err(undo_refusal_to_fault)?;
        } else {
            undo_session::genesis(
                &mut self.graph,
                &mut self.undo,
                self.now_ms,
                &format!("open {}", seed.path.display()),
            )
            .map_err(undo_refusal_to_fault)?;
        }
        self.active = Some(Active {
            path: seed.path,
            read_only: seed.read_only,
            project: seed.project,
            saved_digest: seed.digest,
            assets: seed.assets,
            lock: seed.lock,
        });
        // 打开工程会重建撤销会话态（游标可能被 `align_with` 推导出来），
        // 因此模型会话态里那份镜像必须跟着走 —— 否则"打开一个带历史的工程"之后
        // 两处游标立刻不一致。
        self.sync_session();
        Ok(())
    }

    /// 释放当前工程（`yeban_close_project` 的落点）。
    fn release(&mut self) -> Option<(PathBuf, bool)> {
        let active = self.active.take()?;
        let saved = !active.read_only;
        Some((active.path, saved))
    }
}

/// `yeban_open_project` 的**已校验**打开请求（[`plan_open`] 的产物）。
///
/// 收成一个结构体而不是 9 个 `Plan::Open` 字段：打开一个容器要携带的东西是
/// "工程 + 历史 + 资产池"三类，平铺进枚举变体会让每一处 `match` 都变成
/// 一长串 `..`。结构体也让"打开时必须一起决定的事"在类型上绑在一起。
/// `yeban_open_project` 的**已校验**打开请求（[`plan_open`] 的产物）。
///
/// 收成一个结构体而不是 9 个 `Plan::Open` 字段：打开一个容器要携带的东西是
/// "工程 + 历史 + 资产池"三类，平铺进枚举变体会让每一处 `match` 都变成
/// 一长串 `..`。结构体也让"打开时必须一起决定的事"在类型上绑在一起。
///
/// `create: true` 时 [`OpenRequest::created`] 携带"这份文档是刚建出来的"
/// 以及它的落盘字节；其余路径为 `None`。
#[derive(Debug)]
pub struct OpenRequest {
    /// 目标路径。
    pub path: PathBuf,
    /// 只读打开。
    pub read_only: bool,
    /// 已经解析并校验过的工程。
    pub project: Box<YebanProjectV1>,
    /// 工程内容的规范化摘要。
    pub digest: String,
    /// 磁盘上的文件字节数（**容器字节**）。
    pub bytes: u64,
    /// 是否已经打开了同一个工程（幂等）。
    pub already_open: bool,
    /// `history.dag` 恢复出的提交图谱（空图谱为 `None`）。
    pub history: Option<Box<CommitGraph>>,
    /// `assets/{sha256}` 解出的会话 CAS 资产池。
    pub assets: BTreeMap<AssetHash, Vec<u8>>,
    /// **新建**工程的落盘字节（`create: true` 且目标路径不存在时才有值）。
    ///
    /// 为什么放在这里而不是在 [`apply_open`] 里现算：`plan` 是**只读**的
    /// （`dryRun` 走同一条路，见模块头），所以"这份文档长什么样"必须在 `plan` 里定死；
    /// `apply` 只负责把它原子落盘（`store::write_project_atomic`）。
    /// 两个相位读的是**同一份**字节，因此 `dryRun` 的预览与真调用不会漂移。
    pub created: Option<CreatedSeed>,
}

/// `create: true` 建出来的东西（文档 + 字节 + 读数）。
#[derive(Debug)]
pub struct CreatedSeed {
    /// 已通过 `validate()` 的容器字节。
    pub bytes: Vec<u8>,
    /// 主总线音轨身份（**非 nil**）。
    pub master_bus_track_id: EntityId,
    /// `data.seed` 载荷。
    pub summary: Value,
}

/// **宿主（形态 A 的 GUI）经唯一可变权威施加的一次会话动作**（`ROAD-M4-008` 选项 (a) 第二片）。
///
/// ## 为什么住在领域层而不是 GUI 层
///
/// 生产 GUI 的写入口（`Cmd+Z` / 时光机按钮 / 卷帘铅笔）过去改的是 `yeban-app` 自己的
/// `undo::UndoPort`（一份 `RefCell<UndoSession>`）。那让"界面改了工程"与"控制面会话改了工程"
/// 成为**两处**可变状态。本类型把 GUI 的动作**翻译成领域层的动作**，由
/// [`apply`]（唯一可变入口）施加到控制面正在服务的那一个 [`Domain`] 上 ——
/// 于是 GUI 与控制面读写的是同一份 `Active::project` + `CommitGraph` + `UndoState`。
///
/// ## 它不是第二个工具面
///
/// 它**不**出现在 [`crate::tools::ToolCall`] 的契约里，也**没有**对应的 `yeban_*` 工具：
/// 工具面仍然只有那十个。宿主拿到它的路径是
/// [`crate::transport::http::HttpServer::apply_host_action`] —— 与宿主读数口
/// [`crate::transport::http::HttpServer::host_domain`] 共用**同一个** `Mutex<Dispatcher>`，
/// 没有新端口、新令牌、新通道。
///
/// ## 为什么要走 `Plan` 而不是直接调 `undo_session`
///
/// [`Domain::apply_revision`] 的推进口径写在 [`Plan::mutates_project`] 里、推进动作写在
/// [`apply`] 里。宿主动作若绕过 `apply`，就会长出第二个"推进修订号"的地方。
/// 因此宿主路径与工具路径在**同一处**汇合：`apply(&mut Domain, Plan)`。
#[derive(Debug)]
pub enum HostAction {
    /// 撤销若干步（`Cmd+Z` / 时光机）。
    Undo {
        /// 请求的步数（≥ 1）。
        steps: usize,
    },
    /// 重做若干步（`Cmd+Shift+Z`）。
    Redo {
        /// 请求的步数（≥ 1）。
        steps: usize,
    },
    /// 提交一批 op（卷帘铅笔加音符 / 将来的编辑入口）。
    ///
    /// `origin` 由调用方给出：GUI 的动作是 [`OpOrigin::UserUi`]，与 MCP 采纳提案的
    /// [`OpOrigin::McpProposal`] 区分开 —— 提交血缘不能因为走了一条不同的路就变。
    Commit {
        /// Unix 毫秒（模型不自取时钟，由调用方注入）。
        now_ms: u64,
        /// 操作来源。
        origin: OpOrigin,
        /// 提交信息。
        message: String,
        /// 本次提交携带的 op（**整批算一步**）。
        ops: Vec<Op>,
    },
}

/// 一次 [`HostAction`] 施加后的**结构化读数**。
///
/// GUI 侧用它写动作日志与界面显示态，**不必**去解析工具响应 JSON：字段就在这里，
/// 与 [`Plan::Host`] 的响应是同一处定义的。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostOutcome {
    /// **实际**走的步数（撤销可能少于请求值；提交 = 1）。
    pub steps: usize,
    /// 施加后已撤销的总步数（会话运行态）。
    pub undone_total: usize,
    /// 被处理的 op 变体名（`Op::name()`，顺序 = 处理顺序）。
    pub op_kinds: Vec<String>,
    /// 提交动作的新提交身份（撤销 / 重做时为 `None`）。
    pub commit: Option<EntityId>,
    /// 施加后的**显示态**（模型读数；界面不许自己算）。
    pub display: undo_session::UndoDisplay,
}

/// 一次**宿主保存动作**（[`host_save_project`]）的结构化读数（`ROAD-M4-008` 选项 (a)：
/// 单一写者会话）。
///
/// GUI 侧用它写保存回执，**不必**解析工具响应 JSON；字段与 `yeban_save_project` 的
/// 响应同源（同一个 `apply_save` 口径），但**不经过** `Plan` / 工具分发面。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostSaveOutcome {
    /// 最终落点（调用方给的目标路径）。
    pub path: PathBuf,
    /// 写出的容器字节数。
    pub bytes: usize,
    /// 是否**跳过**了写盘（`force = false` 且内存状态与磁盘一致）。
    pub skipped: bool,
    /// 写进容器的 CAS 资产数。
    pub assets: usize,
    /// 写进 `history.dag` 的提交数。
    pub history_commits: usize,
    /// 工程内容摘要（规范化 JSON 的 SHA-256）。
    pub digest: String,
}

/// 一次已经校验过的执行计划（**只读计算的产物**）。
#[derive(Debug)]
pub enum Plan {
    /// `yeban_open_project`
    Open(Box<OpenRequest>),
    /// `yeban_export_diagnostics`
    Diagnostics {
        /// 已校验的输出目录（`plan` 的产物）。
        export: Box<diagnostics::DiagnosticsExport>,
    },
    /// `yeban_save_project`
    Save {
        /// 目标路径。
        path: PathBuf,
        /// 将要写出的**容器字节**（`ARCH-SEC-003`；由 `write_project_container` 产出）。
        bytes: Vec<u8>,
        /// 将要写出的工程内容摘要（规范化 JSON 的 SHA-256）。
        digest: String,
        /// 是否强制落盘。
        force: bool,
        /// 相比磁盘上的内容是否真有变化。
        changed: bool,
        /// 将写进容器的 CAS 资产数。
        assets: usize,
        /// 将写进 `history.dag` 的提交数。
        history_commits: usize,
    },
    /// `yeban_close_project`
    Close {
        /// 关闭前是否先保存。
        save_first: bool,
        /// 保存计划（`save_first` 且非只读时存在）。
        save: Option<Box<Self>>,
        /// 被关闭的工程路径。
        path: PathBuf,
    },
    /// `yeban_query_project`（只读；`apply` 是空操作）
    Query {
        /// 查询结果 `{project, entities, page}`。
        data: Value,
    },
    /// 三个提案类工具共用的落点
    Propose {
        /// 待创建的提案。
        draft: Box<ProposalDraft>,
        /// 响应是否回传**完整 op 载荷**（`arguments.includeOps`）`[BASELINE-006]`。
        ///
        /// `false`（缺省）⇒ 预览与提交响应只带结构化字段；`true` ⇒ 额外带逐条 `ops`。
        /// 这个标志放在 [`Plan`] 上而不是 [`ProposalDraft`] 上：它是**响应的形状**，
        /// 不是提案的领域内容（提案在两种取值下逐字节相同）。
        include_ops: bool,
    },
    /// `yeban_merge_proposal`
    Merge {
        /// 提案身份。
        proposal_id: EntityId,
        /// 合并提交信息。
        commit_message: String,
        /// 已经在那份**提案**记录里（用于预览）。
        snapshot: Box<Proposal>,
    },
    /// `yeban_reject_proposal`
    Reject {
        /// 提案身份。
        proposal_id: EntityId,
        /// 拒绝原因。
        reason: String,
        /// 预览用的记录快照。
        snapshot: Box<Proposal>,
    },
    /// `yeban_render_master`：**已经渲染好**的产物（只差落盘）
    RenderMaster {
        /// 已编码的容器字节 + 全部实测数字。
        artifact: Box<render::RenderArtifact>,
    },
    /// `yeban_undo` [ADR-0001 **D45**]
    Undo {
        /// 请求的步数（已校验 ≥ 1）。
        steps: usize,
        /// 只读规划时**模型给出**的可撤销深度（预览与真做共用同一口径）。
        undoable: usize,
        /// 将要被撤销的 op 变体名（只读）。
        op_kinds: Vec<&'static str>,
        /// 撤销前的活跃分支（若这次撤销之后接着编辑，会派生匿名分支）。
        branch: String,
    },
    /// `yeban_redo` [ADR-0001 **D45**]
    Redo {
        /// 请求的步数（已校验 ≥ 1）。
        steps: usize,
        /// 只读规划时的可重做深度。
        redoable: usize,
        /// 将要被重做的 op 变体名（只读）。
        op_kinds: Vec<&'static str>,
        /// 重做前的活跃分支。
        branch: String,
    },
    /// `yeban_edit_automation` [ADR-0001 **D46** 第 1 类能力]
    EditAutomation {
        /// 只读规划的产物（读的读数 + 将要写入的那个点）。
        edit: Box<automation::AutomationEdit>,
    },
    /// `yeban_query_engine_state` [ADR-0001 **D46** 第 2 类能力]（**只读**）
    EngineState {
        /// 已经组装好的读数（`apply` 原样返回，一位都不改）。
        data: Value,
    },
    /// `yeban_import_audio` [ADR-0001 **D46** 第 3 类能力]
    ImportAudio {
        /// 只读规划的产物（片段条目 + 需要登记进 CAS 池的字节）。
        import: Box<import_audio::AudioImport>,
    },
    /// `yeban_export_midi`（**只读**）：已经编好的 SMF 字节 + 全部实测读数
    ExportMidi {
        /// 只读规划的产物（SMF 字节 + 计数 + SHA-256）。
        export: Box<export_midi::MidiExportArtifact>,
    },
    /// **宿主动作**（形态 A 的 GUI 经唯一可变权威写入；`ROAD-M4-008` 选项 (a) 第二片）。
    ///
    /// 工具面的 [`plan`] **永远不产出**这个变体（它不是任何一个 `yeban_*` 工具）；
    /// 它只由 [`apply_host`] 构造，再交给 [`apply`] —— 因此它走的是与工具路径
    /// **同一条**施加链，`apply_revision` / `sync_session` 一位不少。
    Host(HostAction),
}

impl Plan {
    /// 计划的工具名（用于预览信封）。
    #[must_use]
    pub const fn op(&self) -> &'static str {
        match self {
            Self::Open(..) => "open",
            Self::Save { .. } => "save",
            Self::Close { .. } => "close",
            Self::Query { .. } => "query",
            Self::Diagnostics { .. } => "diagnostics",
            Self::Propose { .. } => "propose",
            Self::Merge { .. } => "merge",
            Self::Reject { .. } => "reject",
            Self::RenderMaster { .. } => "render",
            Self::Undo { .. } => "undo",
            Self::Redo { .. } => "redo",
            Self::EditAutomation { .. } => "edit_automation",
            Self::EngineState { .. } => "engine_state",
            Self::ImportAudio { .. } => "import_audio",
            Self::ExportMidi { .. } => "export_midi",
            // 宿主动作不是工具（`plan` 不产出它）；名字只用于留痕与判据点名。
            Self::Host(_) => "host_action",
        }
    }

    /// 该计划会向提交图谱添加的提交数（`dryRun` 的"提交数不变"判据用它预测）。
    ///
    /// `Open` 的增量是**变量**（恢复的 `history.dag` 有多少条提交就装多少条），
    /// 因此它的预测值由 [`Plan::planned_commit_count`] 单独给出，不在这里。
    #[must_use]
    pub fn commit_delta(&self) -> usize {
        match self {
            Self::Propose { .. } | Self::Merge { .. } => 1,
            // 只写一个点的自动化编辑 = 一条提交；只读调用（没有 `point`）= 0。
            Self::EditAutomation { edit } => usize::from(edit.write.is_some()),
            // 真登记才提交；幂等命中（内容已存在）一位都不改。
            Self::ImportAudio { import } => usize::from(!import.ops.is_empty()),
            // 宿主动作：提交恰好一条 `Op::Batch`；撤销 / 重做不动图谱。
            Self::Host(action) => match action {
                HostAction::Commit { ops, .. } => usize::from(!ops.is_empty()),
                HostAction::Undo { .. } | HostAction::Redo { .. } => 0,
            },
            Self::Open(..)
            | Self::Save { .. }
            | Self::Close { .. }
            | Self::Query { .. }
            | Self::Diagnostics { .. }
            | Self::Reject { .. }
            | Self::RenderMaster { .. }
            // 引擎/会话读数是**只读**的。
            | Self::EngineState { .. }
            // SMF 导出也是**只读**的（字节只回传，不落盘、不改工程）。
            | Self::ExportMidi { .. }
            // 撤销 / 重做**不动提交图谱**（只动文档与游标）⇒ 提交数不变。
            | Self::Undo { .. }
            | Self::Redo { .. } => 0,
        }
    }

    /// 施加后提交图谱里的提交数（`dryRun` 预览的**精确**预测，不是 `+delta` 的近似）。
    ///
    /// `Open` 会**整体替换**图谱（恢复 `history.dag` 或建一条根提交），所以
    /// `current + 1` 在"打开一个带历史的容器"时是错的读数 —— 这条判据存在的意义
    /// 就是不让预览报一个自己都知道不对的数。
    #[must_use]
    pub fn planned_commit_count(&self, domain: &Domain) -> usize {
        match self {
            Self::Open(request) => request
                .history
                .as_ref()
                .map_or(1, |graph| graph.commit_count()),
            other => domain.commit_count() + other.commit_delta(),
        }
    }

    /// 这个计划**可能改动工程文档**吗（只读变体 = `false`）。
    ///
    /// 与 [`Self::op`] / [`Self::commit_delta`] 同一位置、同一风格：它是
    /// [`Domain::apply_revision`] 的**推进口径**，因此"一次查询把界面刷新了一遍"
    /// 在结构上不会发生。三处容易搞错的地方都写在这里，而不是散在 `apply_inner` 里：
    ///
    /// | 变体 | 为什么是这个答案 |
    /// | :--- | :--- |
    /// | `Propose` / `Reject` | 只动提案记录与提案分支 —— 工作工程（`Active::project`）一位不变，因此界面**不需要**重投影 |
    /// | `Save` / `Close` | `Save` 只改"未保存标记"；`Close` 把工程整体放掉（从"有工程"变成"没有工程"），投影确实会失效 ⇒ 算**可能改**，由调用方如实处理"没有活跃工程"这一档 |
    /// | `Undo` / `Redo` | 不动提交图谱（`commit_delta` = 0），但**真的动工程字节** ⇒ 必须算 |
    #[must_use]
    pub fn mutates_project(&self) -> bool {
        match self {
            Self::Open(..) | Self::Close { .. } | Self::Merge { .. } => true,
            // 撤销 / 重做只移动游标，但工程字节真的回退/前进。
            Self::Undo { .. } | Self::Redo { .. } => true,
            // 只写一个点的自动化编辑改工程；只读调用（没有 `point`）一位都不改。
            Self::EditAutomation { edit } => edit.write.is_some(),
            // 真登记才改工程；幂等命中（内容已存在）一位都不改。
            Self::ImportAudio { import } => !import.ops.is_empty(),
            // 宿主动作：撤销 / 重做真的动工程字节；提交在携带 op 时才算（空批不改文档）。
            Self::Host(action) => match action {
                HostAction::Commit { ops, .. } => !ops.is_empty(),
                HostAction::Undo { .. } | HostAction::Redo { .. } => true,
            },
            Self::Save { .. }
            | Self::Query { .. }
            | Self::Diagnostics { .. }
            | Self::Propose { .. }
            | Self::Reject { .. }
            | Self::RenderMaster { .. }
            // 引擎/会话读数是**只读**的。
            | Self::EngineState { .. }
            // SMF 导出也是**只读**的（字节只回传，不落盘、不改工程）。
            | Self::ExportMidi { .. } => false,
        }
    }

    /// 施加后工程的形态（`None` 表示"工程内容不变"）。
    ///
    /// 这是 `dryRun` 的**差异预览**：把"将要发生什么"算出来给调用方看，
    /// 但**不改**真实状态（`Merge` 与撤销 / 重做都在克隆体上模拟）。
    ///
    /// 撤销 / 重做的模拟走的就是 [`crate::undo_session`] 里的那个 `undo` / `redo`
    /// （在克隆体上跑），因此预览与真做**不可能**漂移。
    ///
    /// # Errors
    ///
    /// `Merge` 的 op 无法整体施加（即真的会冲突）→ `CONFLICT`；
    /// 撤销 / 重做没有可动的东西 → 对应领域失败（`INDEX_OUT_OF_BOUNDS`）。
    pub fn project_after(&self, domain: &Domain) -> Result<Option<YebanProjectV1>, Fault> {
        match self {
            Self::Open(request) => Ok(Some((*request.project).clone())),
            Self::Merge { snapshot, .. } => {
                let current = domain.active_project().ok_or_else(no_active_project)?;
                let mut simulated = current.clone();
                merge_batch(snapshot, "dryRun")
                    .apply(&mut simulated)
                    .map_err(|failure| {
                        Fault::domain_with_data(
                            ErrorCode::Conflict,
                            format!("提案无法合并到当前工程: {failure}"),
                            serde_json::json!({ "model": format!("{failure:?}") }),
                        )
                    })?;
                Ok(Some(simulated))
            }
            Self::Undo { steps, .. } => {
                let current = domain.active_project().ok_or_else(no_active_project)?;
                undo_session::simulate_undo(domain.graph(), current, domain.undo_state(), *steps)
                    .map(Some)
                    .map_err(undo_refusal_to_fault)
            }
            Self::Redo { steps, .. } => {
                let current = domain.active_project().ok_or_else(no_active_project)?;
                undo_session::simulate_redo(domain.graph(), current, domain.undo_state(), *steps)
                    .map(Some)
                    .map_err(undo_refusal_to_fault)
            }
            // 差异预览的**模拟**：写入一个自动化点 / 登记一个音频片段。
            // 两者都只走 `Op::apply`（模型自己的实现），因此预览不可能与真做漂移；
            // 只读调用（没有点 / 幂等命中）返回 `None` ⇒ "工程内容不变"。
            Self::EditAutomation { edit } => {
                let ops = edit.ops();
                if ops.is_empty() {
                    return Ok(None);
                }
                let current = domain.active_project().ok_or_else(no_active_project)?;
                let mut simulated = current.clone();
                Op::Batch {
                    ops,
                    description: "dryRun edit_automation".to_owned(),
                }
                .apply(&mut simulated)
                .map_err(|failure| error::from_model("自动化写入模拟", &failure))?;
                Ok(Some(simulated))
            }
            Self::ImportAudio { import } => {
                if import.ops.is_empty() {
                    return Ok(None);
                }
                let current = domain.active_project().ok_or_else(no_active_project)?;
                let mut simulated = current.clone();
                // 与 `undo_session::commit` 同一口径：`ops` 是**顺序**施加的
                // （`commit` 把它们包成 `Op::Batch`，而模型层的 `Batch` 就是顺序 + 原子）。
                for op in &import.ops {
                    op.apply(&mut simulated)
                        .map_err(|failure| error::from_model("音频登记模拟", &failure))?;
                }
                Ok(Some(simulated))
            }
            // 宿主动作的差异预览：与真做**共用**同一个 `undo_session` / `Op::apply`
            // （尽管 `plan` 永不产出这个变体，这里的口径仍与真做一致）。
            Self::Host(action) => match action {
                HostAction::Undo { steps } => {
                    let current = domain.active_project().ok_or_else(no_active_project)?;
                    undo_session::simulate_undo(
                        domain.graph(),
                        current,
                        domain.undo_state(),
                        *steps,
                    )
                    .map(Some)
                    .map_err(undo_refusal_to_fault)
                }
                HostAction::Redo { steps } => {
                    let current = domain.active_project().ok_or_else(no_active_project)?;
                    undo_session::simulate_redo(
                        domain.graph(),
                        current,
                        domain.undo_state(),
                        *steps,
                    )
                    .map(Some)
                    .map_err(undo_refusal_to_fault)
                }
                HostAction::Commit { ops, message, .. } => {
                    if ops.is_empty() {
                        return Ok(None);
                    }
                    let current = domain.active_project().ok_or_else(no_active_project)?;
                    let mut simulated = current.clone();
                    Op::Batch {
                        ops: ops.clone(),
                        description: message.clone(),
                    }
                    .apply(&mut simulated)
                    .map_err(|failure| error::from_model("宿主动作提交模拟", &failure))?;
                    Ok(Some(simulated))
                }
            },
            Self::Save { .. }
            | Self::Close { .. }
            | Self::Query { .. }
            | Self::Diagnostics { .. }
            | Self::Propose { .. }
            | Self::Reject { .. }
            | Self::RenderMaster { .. }
            | Self::EngineState { .. }
            | Self::ExportMidi { .. } => Ok(None),
        }
    }

    /// `dryRun` 的预览载荷（**只读**）。
    ///
    /// # Errors
    ///
    /// 差异预览本身需要模拟合并且模拟失败时返回 `CONFLICT`。
    pub fn describe(&self, domain: &Domain) -> Result<Value, Fault> {
        let mut preview = Map::new();
        preview.insert("plan".to_owned(), Value::from(self.op()));
        match self {
            Self::Open(request) => {
                preview.insert(
                    "path".to_owned(),
                    Value::from(request.path.display().to_string()),
                );
                preview.insert("readOnly".to_owned(), Value::from(request.read_only));
                preview.insert("bytes".to_owned(), Value::from(request.bytes));
                // `format` 只有一个取值（`ADR-0001 D43`）：常量，不是枚举。
                preview.insert("format".to_owned(), Value::from(store::DOCUMENT_FORMAT));
                preview.insert(
                    "historyCommits".to_owned(),
                    Value::from(
                        request
                            .history
                            .as_ref()
                            .map_or(0, |graph| graph.commit_count()),
                    ),
                );
                preview.insert("assets".to_owned(), Value::from(request.assets.len()));
                preview.insert(
                    "projectDigest".to_owned(),
                    Value::from(request.digest.clone()),
                );
                preview.insert("alreadyOpen".to_owned(), Value::from(request.already_open));
                preview.insert("summary".to_owned(), project_summary(&request.project));
            }
            Self::Save {
                path,
                bytes,
                digest,
                force,
                changed,
                assets,
                history_commits,
            } => {
                preview.insert("path".to_owned(), Value::from(path.display().to_string()));
                preview.insert("bytes".to_owned(), Value::from(bytes.len()));
                preview.insert("format".to_owned(), Value::from(store::DOCUMENT_FORMAT));
                preview.insert("containerEntries".to_owned(), Value::from(2 + assets));
                preview.insert("assets".to_owned(), Value::from(*assets));
                preview.insert("historyCommits".to_owned(), Value::from(*history_commits));
                preview.insert("projectDigest".to_owned(), Value::from(digest.clone()));
                preview.insert("force".to_owned(), Value::from(*force));
                preview.insert("changed".to_owned(), Value::from(*changed));
                preview.insert("atomic".to_owned(), Value::from(true));
                preview.insert(
                    "strategy".to_owned(),
                    Value::from(
                        "容器字节 (ARCH-SEC-003: project.json + history.dag + assets/{sha256}) \
                         + 同目录临时文件 + fsync + rename (ARCH-SEC-004)",
                    ),
                );
                preview.insert("wouldSkip".to_owned(), Value::from(!force && !changed));
            }
            Self::Close {
                save_first,
                save,
                path,
            } => {
                preview.insert("path".to_owned(), Value::from(path.display().to_string()));
                preview.insert("saveFirst".to_owned(), Value::from(*save_first));
                preview.insert("willSave".to_owned(), Value::from(save.is_some()));
                preview.insert(
                    "willReleaseLock".to_owned(),
                    Value::from(domain.active_path().is_some() && !domain.is_read_only()),
                );
            }
            // 诊断导出没有工程差异可预览；顶部的 `plan` 键已足够。
            Self::Diagnostics { .. } => {}
            Self::Query { data } => {
                preview.insert("readOnly".to_owned(), Value::from(true));
                preview.insert(
                    "result".to_owned(),
                    serde_json::json!({
                        "project": data.get("project").cloned().unwrap_or(Value::Null),
                        "page": data.get("page").cloned().unwrap_or(Value::Null),
                        "entityCount": data
                            .get("entities")
                            .and_then(Value::as_array)
                            .map_or(0, Vec::len),
                    }),
                );
            }
            Self::Propose { draft, include_ops } => {
                preview.insert("kind".to_owned(), Value::from(draft.kind));
                preview.insert("title".to_owned(), Value::from(draft.title.clone()));
                preview.insert(
                    "branchPrefix".to_owned(),
                    Value::from(PROPOSAL_BRANCH_PREFIX),
                );
                preview.insert(
                    "baseCommit".to_owned(),
                    Value::from(draft.base_commit.to_canonical_string()),
                );
                preview.insert("opCount".to_owned(), Value::from(draft.ops.len()));
                // 完整 op 载荷是**可选**的（`arguments.includeOps`，缺省 false）：
                // `[BASELINE-006]` 要求 16 小节段落生成的往返 JSON ≤ 4 KB 且
                // "结构化字段传输"，而逐条 op 载荷正是把这条判据顶破的那一块。
                if *include_ops {
                    preview.insert("ops".to_owned(), draft_ops_value(draft));
                }
                // "将要做什么"的**派生**清单（从同一份 `ops` 数出来, 因此不可能漂移）。
                preview.insert(
                    "willCreate".to_owned(),
                    section_build::summarize_ops(&draft.ops),
                );
            }
            Self::Merge {
                proposal_id,
                commit_message,
                snapshot,
            } => {
                preview.insert(
                    "proposalId".to_owned(),
                    Value::from(proposal_id.to_canonical_string()),
                );
                preview.insert(
                    "commitMessage".to_owned(),
                    Value::from(commit_message.clone()),
                );
                preview.insert("proposal".to_owned(), snapshot.summary());
                preview.insert("batchOpCount".to_owned(), Value::from(snapshot.ops.len()));
            }
            Self::Reject {
                proposal_id,
                reason,
                snapshot,
            } => {
                preview.insert(
                    "proposalId".to_owned(),
                    Value::from(proposal_id.to_canonical_string()),
                );
                preview.insert("reason".to_owned(), Value::from(reason.clone()));
                preview.insert("proposal".to_owned(), snapshot.summary());
            }
            Self::RenderMaster { artifact } => {
                // 渲染在 `plan`（只读）里就完成了: 因此预览里的字节数、SHA-256、帧数
                // 都是**实测值**而不是估算。落盘只发生在 `apply`。
                if let Value::Object(details) = artifact.preview() {
                    for (key, value) in details {
                        preview.insert(key, value);
                    }
                }
            }
            Self::Undo {
                steps,
                undoable,
                op_kinds,
                branch,
            } => {
                preview.insert("branch".to_owned(), Value::from(branch.clone()));
                preview.insert("requestedSteps".to_owned(), Value::from(*steps));
                preview.insert("undoableSteps".to_owned(), Value::from(*undoable));
                preview.insert(
                    "willUndoSteps".to_owned(),
                    Value::from((*steps).min(*undoable)),
                );
                preview.insert(
                    "opKinds".to_owned(),
                    Value::Array(op_kinds.iter().map(|kind| Value::from(*kind)).collect()),
                );
                preview.insert(
                    "undoneBefore".to_owned(),
                    Value::from(domain.undo_state().undone()),
                );
                preview.insert(
                    "undoneAfter".to_owned(),
                    Value::from(domain.undo_state().undone() + (*steps).min(*undoable)),
                );
            }
            Self::Redo {
                steps,
                redoable,
                op_kinds,
                branch,
            } => {
                preview.insert("branch".to_owned(), Value::from(branch.clone()));
                preview.insert("requestedSteps".to_owned(), Value::from(*steps));
                preview.insert("redoableSteps".to_owned(), Value::from(*redoable));
                preview.insert(
                    "willRedoSteps".to_owned(),
                    Value::from((*steps).min(*redoable)),
                );
                preview.insert(
                    "opKinds".to_owned(),
                    Value::Array(op_kinds.iter().map(|kind| Value::from(*kind)).collect()),
                );
                preview.insert(
                    "undoneBefore".to_owned(),
                    Value::from(domain.undo_state().undone()),
                );
                preview.insert(
                    "undoneAfter".to_owned(),
                    Value::from(
                        domain
                            .undo_state()
                            .undone()
                            .saturating_sub((*steps).min(*redoable)),
                    ),
                );
            }
            // 自动化编辑：预览与真做**共用** `AutomationEdit::data()`，
            // 因此"预览说的"与"真做的"逐字段相同（判据在 `tests/extension_tools.rs`）。
            Self::EditAutomation { edit } => {
                let data = edit.data()?;
                if let Value::Object(fields) = data {
                    for (key, value) in fields {
                        preview.insert(key, value);
                    }
                }
                preview.insert("wouldApply".to_owned(), Value::from(edit.write.is_some()));
            }
            // 引擎/会话读数：预览 = 真做（只读工具没有任何差异可预览）。
            // 与上面两个计划**同一个做法**：把 `data` 的键合并进预览，因此
            // "预览与真做逐字段一致"这条判据能用同一段代码对三个工具成立。
            Self::EngineState { data } => {
                if let Value::Object(fields) = data.clone() {
                    for (key, value) in fields {
                        preview.insert(key, value);
                    }
                }
                preview.insert("readOnly".to_owned(), Value::from(true));
                preview.insert("result".to_owned(), data.clone());
            }
            // 音频导入：与自动化编辑同一个做法（共用 `AudioImport::data()`）。
            Self::ImportAudio { import } => {
                let data = import.data()?;
                if let Value::Object(fields) = data {
                    for (key, value) in fields {
                        preview.insert(key, value);
                    }
                }
                preview.insert("wouldApply".to_owned(), Value::from(!import.ops.is_empty()));
            }
            // SMF 导出：**只读**且不做差异模拟 —— 与引擎读数同一个做法
            // （共用 `MidiExportArtifact::data()`，于是预览与真做逐字段一致）。
            Self::ExportMidi { export } => {
                let data = export.data();
                if let Value::Object(fields) = data.clone() {
                    for (key, value) in fields {
                        preview.insert(key, value);
                    }
                }
                preview.insert("readOnly".to_owned(), Value::from(true));
                preview.insert("result".to_owned(), data);
            }
            // 宿主动作没有工具信封可预览；顶部的 `plan` 键 + 下面的工程摘要已足够。
            // （`plan` 永不产出这个变体，因此这条分支在实践中不可达。）
            Self::Host(_) => {}
        }

        // 差异预览的公共部分：工程内容摘要 + 提交数（**预测**，不是实测）。
        let before = domain.project_digest();
        let after = self.project_after(domain)?;
        let after_digest = match &after {
            Some(project) => Some(store::digest_of(
                store::serialize_project(project)?.as_bytes(),
            )),
            None => before.clone(),
        };
        preview.insert(
            "projectDigestBefore".to_owned(),
            before.clone().map_or(Value::Null, Value::from),
        );
        preview.insert(
            "projectDigestAfter".to_owned(),
            after_digest.clone().map_or(Value::Null, Value::from),
        );
        preview.insert(
            "projectChanges".to_owned(),
            Value::from(after_digest != before || after.is_some()),
        );
        preview.insert(
            "commitCountBefore".to_owned(),
            Value::from(domain.commit_count()),
        );
        preview.insert(
            "commitCountAfter".to_owned(),
            Value::from(self.planned_commit_count(domain)),
        );
        // 计划自己的口径优先：只读调用（不给 `point` / 幂等命中）在各自的臂里已经写了
        // `wouldApply: false` —— 无条件覆盖成 `true` 会让预览对**只读调用**撒谎。
        preview
            .entry("wouldApply".to_owned())
            .or_insert(Value::from(true));
        Ok(Value::Object(preview))
    }
}

/// `NO_ACTIVE_PROJECT` 的简写。
fn no_active_project() -> Fault {
    Fault::domain(ErrorCode::NoActiveProject, "当前没有活跃工程")
}

/// [`UndoRefusal`] → 契约错误码的**唯一**映射 [ADR-0001 **D25**：联集 20 值，不发明新码]。
///
/// | 拒绝 | 契约码 | 为什么是它 |
/// | :--- | :--- | :--- |
/// | `NoHistory` | `INDEX_OUT_OF_BOUNDS` | D25 的联集里没有 `NO_HISTORY`；"请求的步数超出可回退深度"就是索引/计数越界 |
/// | `NoRedo` | `INDEX_OUT_OF_BOUNDS` | 同上（重做游标已在头上） |
/// | `NoBranch` / `NotAtCommitBoundary` / `Model` | `CONFLICT` | 会话状态与操作日志不一致 ⇒ 状态冲突 |
/// | `Serialization` | `IO_ERROR` | 容器写出失败是 I/O 面的失败 |
fn undo_refusal_to_fault(refusal: UndoRefusal) -> Fault {
    match refusal {
        UndoRefusal::NoHistory { undoable } => Fault::domain_with_data(
            ErrorCode::IndexOutOfBounds,
            "没有可撤销的历史",
            serde_json::json!({ "undoable": undoable, "reason": "no-history" }),
        ),
        UndoRefusal::NoRedo => Fault::domain_with_data(
            ErrorCode::IndexOutOfBounds,
            "没有可重做的步骤",
            serde_json::json!({ "redoable": 0, "reason": "no-redo" }),
        ),
        UndoRefusal::NoBranch { branch } => Fault::domain_with_data(
            ErrorCode::Conflict,
            format!("提交图谱里没有分支 `{branch}`"),
            serde_json::json!({ "branch": branch }),
        ),
        UndoRefusal::NotAtCommitBoundary { undone } => Fault::domain_with_data(
            ErrorCode::Conflict,
            format!("撤销位置 {undone} 不落在提交边界上"),
            serde_json::json!({ "undone": undone }),
        ),
        UndoRefusal::Model { detail } => Fault::domain_with_data(
            ErrorCode::Conflict,
            format!("模型层拒绝: {detail}"),
            serde_json::json!({ "model": detail }),
        ),
        UndoRefusal::Serialization { detail } => {
            Fault::domain(ErrorCode::IoError, format!("工程容器写出失败: {detail}"))
        }
    }
}

/// 工程的**摘要**（预览与打开响应共用；不含音符，避免把上下文撑爆）。
fn project_summary(project: &YebanProjectV1) -> Value {
    serde_json::json!({
        "id": project.id.to_canonical_string(),
        "title": project.title,
        "schemaVersion": project.schema_version,
        "minReaderVersion": project.min_reader_version,
        "bpm": project.bpm,
        "timeSignature": project.time_signature,
        "sampleRate": project.sample_rate().hz(),
        "trackCount": project.tracks.len(),
        "sectionCount": project.sections.len(),
        "sceneCount": project.scenes.len(),
        "clipCount": project.clip_pool.len(),
        "noteCount": project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .map(std::collections::BTreeMap::len)
            .sum::<usize>(),
        "routingNodeCount": project.routing_graph.nodes.len(),
        "routingEdgeCount": project.routing_graph.edges.len(),
    })
}

// ---------------------------------------------------------------------------
// 实参读取
// ---------------------------------------------------------------------------

/// 读字符串实参。
fn arg_str<'a>(call: &'a ToolCall, name: &str) -> Option<&'a str> {
    call.arguments.get(name).and_then(Value::as_str)
}

/// 读布尔实参（缺省值由调用方给）。
fn arg_bool(call: &ToolCall, name: &str, default: bool) -> bool {
    call.arguments
        .get(name)
        .and_then(Value::as_bool)
        .unwrap_or(default)
}

/// 读非负整数实参（缺省 `None`）。
fn arg_u64(call: &ToolCall, name: &str) -> Result<Option<u64>, Fault> {
    match call.arguments.get(name) {
        None => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("`{name}` 必须是非负整数, 实际收到 {value}"),
            )
        }),
    }
}

/// 读 `EntityId` 实参。
fn arg_id(call: &ToolCall, name: &str) -> Result<EntityId, Fault> {
    use std::str::FromStr as _;
    let text = arg_str(call, name).ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{name}` 必须是 26 字符 ULID 字符串"),
        )
    })?;
    EntityId::from_str(text).map_err(|error| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{name}` 不是合法 ULID: {error}"),
        )
    })
}

/// 取活跃工程，否则 `NO_ACTIVE_PROJECT`。
fn require_active(domain: &Domain) -> Result<&YebanProjectV1, Fault> {
    domain.active_project().ok_or_else(no_active_project)
}

/// 提案的 `Op::Batch`（合并与预览共用同一份构造）。
fn merge_batch(proposal: &Proposal, description: &str) -> Op {
    Op::Batch {
        ops: proposal
            .ops
            .iter()
            .map(|stamped| stamped.op.clone())
            .collect(),
        description: description.to_owned(),
    }
}

// ---------------------------------------------------------------------------
// plan：只读
// ---------------------------------------------------------------------------

/// 只读规划：参数校验 + 领域合法性校验（`yeban_render_master` 连渲染都在这里做完）。
/// **改不了任何状态**（拿到的是 `&Domain`），也**不碰文件系统**。
///
/// # Errors
///
/// 见各工具的领域语义。目前所有工具的实现级状况都不在 `plan` 里产生。
pub fn plan(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    match call.tool.name {
        "yeban_open_project" => plan_open(domain, call),
        "yeban_save_project" => plan_save(domain, call),
        "yeban_close_project" => plan_close(domain, call),
        "yeban_query_project" => plan_query(domain, call),
        "yeban_propose_section" => plan_propose_section(domain, call),
        "yeban_edit_notes" => plan_edit_notes(domain, call),
        "yeban_set_macro" => plan_set_macro(domain, call),
        "yeban_render_master" => plan_render_master(domain, call),
        "yeban_merge_proposal" => plan_merge(domain, call),
        "yeban_reject_proposal" => plan_reject(domain, call),
        "yeban_undo" => plan_undo(domain, call),
        "yeban_redo" => plan_redo(domain, call),
        "yeban_edit_automation" => plan_edit_automation(domain, call),
        "yeban_query_engine_state" => plan_query_engine_state(domain, call),
        "yeban_import_audio" => plan_import_audio(domain, call),
        "yeban_export_diagnostics" => plan_export_diagnostics(domain, call),
        "yeban_export_midi" => plan_export_midi(domain, call),
        // `ToolCall::from_params` 已按契约枚举把关, 因此这里不可达;
        // 用 CONFLICT 而不是 panic: 未知工具名不该让服务进程倒下。
        other => Err(Fault::domain(
            ErrorCode::Conflict,
            format!("未接线到领域实现的工具 `{other}`"),
        )),
    }
}

/// 撤销 / 重做的 `steps` 实参（缺省 1；`0` 是非法的**显式**取值）。
///
/// 为什么 `0` 不给"等价于 1"的宽容：Agent 传 0 时最可能的意思是"我不想改任何东西"，
/// 而"一次 Cmd+Z 至少撤一步"是界面侧的语义。含糊地替它决定一步，不如明确拒绝。
fn parse_steps(call: &ToolCall) -> Result<usize, Fault> {
    let raw = arg_u64(call, "steps")?.unwrap_or(1);
    if raw == 0 {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`steps` 必须 ≥ 1（缺省 = 1）",
        ));
    }
    usize::try_from(raw).map_err(|_| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`steps` 超出 usize 表示范围: {raw}"),
        )
    })
}

/// `yeban_undo` [ADR-0001 **D45**]。
///
/// **只读**规划：确认有活跃工程、实参合法、并且**真的有东西可撤**
/// （否则在这里就报 `INDEX_OUT_OF_BOUNDS`，`dryRun` 因此也能如实回答"撤不动"）。
/// 真正的逆操作由 [`crate::undo_session::undo`] 里的模型入口施加。
fn plan_undo(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    require_active(domain)?;
    let steps = parse_steps(call)?;
    let undoable = domain
        .undo_state()
        .undoable(domain.graph())
        .map_err(undo_refusal_to_fault)?;
    let op_kinds = undo_session::undo_op_kinds(domain.graph(), domain.undo_state(), steps)
        .map_err(undo_refusal_to_fault)?;
    Ok(Plan::Undo {
        steps,
        undoable,
        op_kinds,
        branch: domain.active_branch().to_owned(),
    })
}

/// `yeban_redo` [ADR-0001 **D45**]。
fn plan_redo(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    require_active(domain)?;
    let steps = parse_steps(call)?;
    let redoable = domain.undo_state().undone();
    let op_kinds = undo_session::redo_op_kinds(domain.graph(), domain.undo_state(), steps)
        .map_err(undo_refusal_to_fault)?;
    Ok(Plan::Redo {
        steps,
        redoable,
        op_kinds,
        branch: domain.active_branch().to_owned(),
    })
}

/// `yeban_open_project` 里**只对 `create: true` 有意义**的实参。
///
/// 没有 `create` 时给它们一律**响亮拒绝**（`INVALID_PARAMETER_RANGE` +
/// `data.reason = "createOnlyParameter"`），绝不静默丢弃 —— 与
/// `yeban_import_audio` 的 `placementWithoutTrack` 同一条纪律：调用方给了键，
/// 就必须知道那个键到底被读了没有。"`timeSignature` 给了但没 `create` ⇒ 工程仍是 `4/4`"
/// 正是这条纪律要拦住的那种"看起来有"。
const CREATE_ONLY_PARAMS: [&str; 4] =
    ["title", "bpm", project_create::TIME_SIGNATURE_PARAM, "seed"];

/// `yeban_open_project`。
///
/// 读盘一律走 [`store::load_project`]（**只接受 `.yeban` 容器**，`ADR-0001 D43`），
/// 工程**历史 / CAS 资产池**一并进入 [`OpenRequest`]。
fn plan_open(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let path =
        PathBuf::from(arg_str(call, "path").ok_or_else(|| {
            Fault::domain(ErrorCode::InvalidParameterRange, "`path` 必须是字符串")
        })?);
    let read_only = arg_bool(call, "readOnly", false);
    // `create: true` 走**建工程**分支：从零构造一份可渲染、可配器的文档。
    // 它不读盘（路径本来就不存在），因此不会撞 `FILE_NOT_FOUND` ——
    // 这正是本能力要关的那个缺口（旧行为：不存在的路径 ⇒ FILE_NOT_FOUND，
    // 而没有任何工具能建工程）。
    if arg_bool(call, "create", false) {
        return plan_create(domain, path, read_only, call);
    }
    // 只对 `create: true` 有意义的实参：在没有 `create` 时**响亮拒绝**，
    // 而不是静默丢弃（`title` / `bpm` / `seed` 在加这条之前就是被静默忽略的）。
    let create_only: Vec<&str> = CREATE_ONLY_PARAMS
        .into_iter()
        .filter(|name| call.arguments.contains_key(*name))
        .collect();
    if !create_only.is_empty() {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{}` 只在 `create: true` 时有效 (本次是打开已有工程)",
                create_only.join("`, `")
            ),
            serde_json::json!({
                "reason": "createOnlyParameter",
                "parameters": create_only,
            }),
        ));
    }
    let loaded = store::load_project(&path)?;
    let json = store::serialize_project(&loaded.project)?;
    let digest = store::digest_of(json.as_bytes());
    let already_open = domain.active_path().is_some_and(|open| open == path);
    let conflict_open = domain.active_path().is_some_and(|open| open != path);
    if conflict_open {
        return Err(Fault::domain_with_data(
            ErrorCode::Conflict,
            format!(
                "已有另一个活跃工程 `{}`, 请先关闭它",
                domain
                    .active_path()
                    .map_or_else(String::new, |open| open.display().to_string())
            ),
            serde_json::json!({ "activePath": domain.active_path().map(|open| open.display().to_string()) }),
        ));
    }
    // 幂等分支只在**模式相同**时成立。
    //
    // 本会话只读持有共享建议锁时，再来一个 `readOnly: false` 是**模式升级**：
    // 放行它等于让一个写者绕过共享锁（那正是 `MUST-GATE-008` 要拦的）。
    // 必须让底层建议锁仲裁 —— 复用当前句柄是拿不到升级的，直接报 `PROJECT_LOCKED`。
    if already_open && read_only != domain.is_read_only() {
        return Err(store::locked_fault(&path));
    }
    Ok(Plan::Open(Box::new(OpenRequest {
        path,
        read_only,
        project: Box::new(loaded.project),
        digest,
        bytes: loaded.bytes,
        already_open,
        history: loaded.graph.map(Box::new),
        assets: loaded.assets,
        created: None,
    })))
}

/// `yeban_open_project` 的 **`create: true`** 分支。
///
/// 三条判据（顺序即错误码优先级）：
///
/// 1. `readOnly: true` 同给 ⇒ `INVALID_PARAMETER_RANGE`（只读地新建一个文件
///    是自相矛盾的要求，静默忽略只读位会让调用方以为"没写盘"）；
/// 2. 目标路径**已存在** ⇒ `CONFLICT` + `data.reason = "projectAlreadyExists"`
///    （**绝不**静默覆盖；`yeban_save_project` 的语义是"刷盘到当前工程路径"，
///    与本工具的"写一个新文件"是两件事，因此这里不复用它）；
/// 3. 已有另一个活跃工程 ⇒ `CONFLICT`（与普通打开分支同一口径）。
fn plan_create(
    domain: &Domain,
    path: PathBuf,
    read_only: bool,
    call: &ToolCall,
) -> Result<Plan, Fault> {
    if read_only {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "`create: true` 与 `readOnly: true` 相互矛盾: 新建工程必须写盘",
            serde_json::json!({ "reason": "createIsNotReadOnly" }),
        ));
    }
    if path.exists() {
        return Err(Fault::domain_with_data(
            ErrorCode::Conflict,
            format!(
                "工程已存在, 拒绝新建覆盖: {} (要打开它就不要给 `create`)",
                path.display()
            ),
            serde_json::json!({
                "reason": "projectAlreadyExists",
                "path": path.display().to_string(),
            }),
        ));
    }
    if domain.active_path().is_some_and(|open| open != path) {
        return Err(Fault::domain_with_data(
            ErrorCode::Conflict,
            format!(
                "已有另一个活跃工程 `{}`, 请先关闭它",
                domain
                    .active_path()
                    .map_or_else(String::new, |open| open.display().to_string())
            ),
            serde_json::json!({ "activePath": domain.active_path().map(|open| open.display().to_string()) }),
        ));
    }
    let seed = call.arguments.get("seed");
    let config = project_create::parse_config(
        call.arguments.get("title"),
        call.arguments.get("bpm"),
        call.arguments.get(project_create::TIME_SIGNATURE_PARAM),
        seed.and_then(|seed| seed.get("trackCount")),
        seed.and_then(|seed| seed.get("clipName")),
        seed.and_then(|seed| seed.get("notes")),
    )?;
    let created = project_create::build(&config, &path)?;
    let json = store::serialize_project(&created.project)?;
    let digest = store::digest_of(json.as_bytes());
    // `history.dag`：新工程从**空图谱**开始（与 `yeban-app --save-as` 的
    // 内置样本同一条路径：样本本来就没有归档历史层）。
    let history = serde_json::to_vec(&CommitGraph::new())
        .map_err(|error| Fault::domain(ErrorCode::IoError, format!("空图谱序列化失败: {error}")))?;
    let bytes = yeban_model::container::write_project_container(
        &created.project,
        &history,
        &BTreeMap::new(),
    )
    .map_err(|error| {
        Fault::domain(
            ErrorCode::IoError,
            format!("新建工程的容器写出被拒: {error}"),
        )
    })?;
    let summary = project_create::seed_summary(&created);
    let byte_count = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    Ok(Plan::Open(Box::new(OpenRequest {
        path,
        read_only: false,
        project: Box::new(created.project),
        digest,
        bytes: byte_count,
        already_open: false,
        history: None,
        assets: BTreeMap::new(),
        created: Some(CreatedSeed {
            bytes,
            master_bus_track_id: created.master_bus_track_id,
            summary,
        }),
    })))
}

/// `yeban_save_project`。
fn plan_save(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let active = domain.active.as_ref().ok_or_else(no_active_project)?;
    if active.read_only {
        return Err(Fault::domain(
            ErrorCode::IoError,
            format!("工程 `{}` 是只读打开的, 拒绝落盘", active.path.display()),
        ));
    }
    let (bytes, digest, assets, history_commits) = save_material(domain)?;
    Ok(Plan::Save {
        path: active.path.clone(),
        changed: digest != active.saved_digest,
        assets,
        history_commits,
        bytes,
        digest,
        force: arg_bool(call, "force", false),
    })
}

/// "这份工程现在会写出哪些字节 / 摘要 / 条目数" —— `yeban_save_project` 与
/// **宿主保存动作** [`host_save_project`] 的**唯一**口径。
///
/// 为什么不各算一遍：容器字节的口径（`ARCH-SEC-003`：工程 + 提交图谱 + CAS 资产池）
/// 一旦有两份实现，迟早会出现"工具保存写进去的东西与宿主保存写进去的不一样"。
///
/// # Errors
///
/// 没有活跃工程、工程 / 图谱序列化失败、或容器层拒绝。
fn save_material(domain: &Domain) -> Result<(Vec<u8>, String, usize, usize), Fault> {
    let active = domain.active.as_ref().ok_or_else(no_active_project)?;
    let json = store::serialize_project(&active.project)?;
    let digest = store::digest_of(json.as_bytes());
    // 落盘字节 = `ARCH-SEC-003` 的容器（`project.json` + `history.dag` + `assets/{sha256}`）。
    // 与"工程内容摘要"是**两个量**：前者含提交图谱与资产，后者只描述工程文档。
    let bytes =
        store::container_bytes(&active.path, &active.project, &domain.graph, &active.assets)?;
    Ok((
        bytes,
        digest,
        active.assets.len(),
        domain.graph.commit_count(),
    ))
}

/// `yeban_close_project`。
fn plan_close(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let active = domain.active.as_ref().ok_or_else(no_active_project)?;
    let save_first = arg_bool(call, "saveFirst", true);
    let save = if save_first && !active.read_only {
        let json = store::serialize_project(&active.project)?;
        let digest = store::digest_of(json.as_bytes());
        let bytes =
            store::container_bytes(&active.path, &active.project, &domain.graph, &active.assets)?;
        Some(Box::new(Plan::Save {
            path: active.path.clone(),
            bytes,
            digest,
            force: false,
            changed: true,
            assets: active.assets.len(),
            history_commits: domain.graph.commit_count(),
        }))
    } else {
        None
    };
    Ok(Plan::Close {
        save_first,
        save,
        path: active.path.clone(),
    })
}

/// `yeban_query_project`。
fn plan_query(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let project = require_active(domain)?;
    let query = view::parse(&call.arguments)?;
    let data = view::data(project, &query)?;
    Ok(Plan::Query { data })
}

/// 三个提案类工具共用的收尾：先模拟，再打包成 [`Plan::Propose`]。
///
/// `include_ops` 直接来自 `arguments.includeOps`（缺省 `false`）—— 它只影响
/// **响应的形状**，不影响提案内容：两种取值下被创建的隔离分支、op 日志与提交
/// 逐字节相同（判据 `tests/payload_budget.rs::include_ops_only_changes_the_response_shape`）。
fn propose_draft(
    domain: &Domain,
    project: &YebanProjectV1,
    kind: &'static str,
    title: String,
    description: String,
    ops: Vec<Op>,
    include_ops: bool,
) -> Result<Plan, Fault> {
    let base_commit = domain.active_head().ok_or_else(|| {
        Fault::domain(ErrorCode::Conflict, "主分支没有头提交（提交图谱未初始化）")
    })?;
    // 模拟：整批 op 必须能在**当前**工程上干净地施加，否则这个提案不该被创建。
    let mut simulated = project.clone();
    Op::Batch {
        ops: ops.clone(),
        description: description.clone(),
    }
    .apply(&mut simulated)
    .map_err(|failure| error::from_model("提案模拟", &failure))?;
    simulated
        .validate()
        .map_err(|failure| error::from_model("提案结果校验", &failure))?;
    let base_digest = store::digest_of(store::serialize_project(project)?.as_bytes());
    Ok(Plan::Propose {
        draft: Box::new(ProposalDraft {
            kind,
            title,
            description,
            ops,
            base_commit,
            base_digest,
        }),
        include_ops,
    })
}

/// `yeban_propose_section`。
fn plan_propose_section(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let project = require_active(domain)?;
    let section_name = arg_str(call, "sectionName").ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`sectionName` 必须是字符串",
        )
    })?;
    let style_preset = arg_str(call, "stylePreset").ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`stylePreset` 必须是字符串",
        )
    })?;
    let bars = arg_u64(call, "bars")?.unwrap_or(0);
    let scale = arg_str(call, "scale");
    let section_plan = section::plan(project, section_name, style_preset, bars, scale)?;
    propose_draft(
        domain,
        project,
        "section",
        format!("章节骨架: {section_name} ({style_preset}, {bars} 小节)"),
        format!("propose_section {section_name}"),
        section_plan.ops,
        include_ops(call),
    )
}

/// `yeban_edit_notes`。
///
/// 五个形态（同一个工具、同一份 `NoteOp` 解析器、同一个发声数上限）：
///
/// - **编辑**（缺省，`create: false` 且无 `placement`）：`clipId` 必须已经在
///   `clip_pool` 里，每条 `NoteOp` 编译成一条 `Op` —— **缺省路径逐字节不变**；
/// - **创建材料**（`create: true`）：`clipId` 是**将要新建的**片段身份，
///   `ops` 只允许 `add`，整批折成**一条** `Op::AddClip` ⇒ 池子里多一条
///   "MIDI 且至少一个音符"的材料（关闭 `docs/ledger/tools-domain-notes.md:283`
///   的 needs-8：空池工程从此能做 `yeban_propose_section`）；
/// - **取走材料**（`ops[].kind == "removeClip"`，必须单独出现）：把顶层 `clipId`
///   这个**片段池条目**取走（`Op::RemoveClip`，撤销载荷 `previous_clip` 从当前文档读）
///   ⇒ 上一形态的镜像；此前池里的条目**没有任何工具**能取走（`yeban_query_project`
///   的实体索引却在报它们的身份）。与 `create: true` / `placement` / 其它任何
///   `kind` 同给都是**响亮失败**（`removeClipTakesNoOtherOps` /
///   `removeClipIsNotPlacement` / `createRequiresAddOps`）；还有摆放引用它时由模型
///   报 `ClipInUse` ⇒ `CONFLICT`；
/// - **摆放材料**（`placement` 在场）：在音符那一半之外**追加**一条
///   `Op::AddClipPlacement`，把已有的 `clipId` 摆到 `trackId` 的 `startTick` 上
///   （关闭 `docs/ledger/mcp-tools-expansion-notes.md` §6 的 needs-6：
///   池子里的片段此前**没有任何工具**能摆上时间轴 ⇒ 渲染器一帧都不出声）。
///   此时 `ops` 允许是**空数组** —— "这次调用做什么"由 `placement` 承载。
/// - **音轨级编辑**（`ops[].kind` ∈ `setParam` / `setTrackMute` / `setTrackSolo` /
///   `setAutomationLane` / `removeAutomationPoint`）：分别写 `trackId` 那条轨的
///   `TrackV3::volume_db` / `pan`（`Op::SetParam`）、`TrackV3::mute` / `solo`
///   （`Op::SetTrackMute` / `Op::SetTrackSolo`）、自动化泳道自己的属性
///   （`Op::SetAutomationLane` / `Op::RemoveAutomationLane`）与泳道里的**一个**点
///   （`Op::RemoveAutomationPoint`，按文档上的 `tick` 或点的显式身份寻址）
///   —— 关闭"17 个工具没有一个能写静态混音值 / 通道条开关 / 泳道属性 /
///   取走一个自动化点"这条缺口。
///   它们都是**音轨级**的：`compile` 的"片段必须是 MIDI"断言只在真的有音符操作时
///   成立（见 `notes::NoteOp::is_note_level`）。
/// - **路由级编辑**（`ops[].kind == "setRoutingGain"`）：写操作对象**自带的**
///   `edgeId` 那条路由边的静态增益（`Op::SetRoutingGain`，载荷是 `Option<f32>`：
///   `value: null` = 单位增益）。目标**不在**顶层 `trackId` / `clipId` 上 ——
///   与音轨级形态一样，它与片段内容无关（见 `notes::NoteOp::is_routing_level`），
///   因此提案标题按实际内容报成"路由级编辑"，不冒充音轨级。
/// - **路由级取走**（`ops[].kind == "disconnectRouting"`）：把操作对象**自带的**
///   `edgeId` 那条路由边**取走**（`Op::DisconnectRouting`，撤销载荷 `previous_edge`
///   从当前文档读整条边）。同一个"路由级"分类（`notes::NoteOp::is_routing_level`），
///   但改的是边**本身**而不是边上的一个值 —— 关闭"工具面造得出的边取不走"这条
///   缺口（`yeban_propose_section` 的建批是 `Op::ConnectRouting` 在 MCP 侧唯一的
///   构造点，而 `yeban_query_project` 的实体索引一直在报那些边的身份）。
///
/// `placement` 与 `create: true` **同给**是响亮失败（`placementIsNotCreation`）：
/// 先建材料、再摆材料，两步各自成一个可审查的提案，而不是把两件事塞进一次提交。
fn plan_edit_notes(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let project = require_active(domain)?;
    let track_id = arg_id(call, "trackId")?;
    let clip_id = arg_id(call, "clipId")?;
    let creating = arg_bool(call, notes::CREATE_PARAM, false);
    let placement_raw = call.arguments.get(notes::PLACEMENT_FIELD);
    if creating && placement_raw.is_some() {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "`placement` 与 `create: true` 不能同给: 前者摆**已有**片段, 后者建**新**片段",
            serde_json::json!({
                "reason": "placementIsNotCreation",
                "hint": "先 `create: true` 建材料, 再单独一次调用给 `placement` 摆它",
            }),
        ));
    }
    let raw_ops = call
        .arguments
        .get("ops")
        .ok_or_else(|| Fault::domain(ErrorCode::InvalidParameterRange, "缺少 `ops`"))?;
    if creating {
        let note_ops = notes::parse_ops(raw_ops)?;
        let clip_name = arg_str(call, notes::CLIP_NAME_PARAM)
            .filter(|name| !name.is_empty())
            .unwrap_or(notes::DEFAULT_NEW_CLIP_NAME);
        let compiled = notes::compile_create(project, &track_id, &clip_id, clip_name, &note_ops)?;
        notes::check_polyphony(project, &clip_id, &compiled)?;
        return propose_draft(
            domain,
            project,
            "notes",
            format!("新建 MIDI 片段: {clip_name} ({} 个音符)", note_ops.len()),
            format!("edit_notes create {clip_id}"),
            compiled,
            include_ops(call),
        );
    }
    // 摆放那一半先解析（它会拒绝未知形态、未知键、越界与"推不出长度"，
    // 也拒绝已被占用的摆放身份，以及 `move`/`remove` 找不到的摆放身份）。
    let placement = notes::parse_placement_edit(project, &track_id, &clip_id, &call.arguments)?;
    // 空 `ops` 只在摆放在场时成立；否则仍走 `parse_ops` 的空数组守卫
    // （"空操作不是一次编辑请求"这条口径没有放松）。
    let note_ops =
        if placement.is_some() && raw_ops.as_array().is_some_and(|items| items.is_empty()) {
            Vec::new()
        } else {
            notes::parse_ops(raw_ops)?
        };
    // 池级形态（`removeClip`）**单独成一路**：它把顶层 `clipId` 那个**片段池条目**
    // 取走，既不读不写音符，也不碰摆放 ⇒ 与另外三路互斥。两条排他性规则住在
    // `notes::reject_remove_clip_conflicts`（那里能被本机探针真的执行到），
    // 判定时点是**编译之前**（绝不让一个自相矛盾的批去撞一个更难懂的错误）。
    let removing_clip = matches!(note_ops.as_slice(), [notes::NoteOp::RemoveClip]);
    notes::reject_remove_clip_conflicts(&note_ops, placement.is_some())?;
    // 顺序 = 施加顺序: 先改音符, 再施加（此刻内容已确定的）摆放编辑。
    let mut compiled = if note_ops.is_empty() {
        Vec::new()
    } else {
        // 同一个泳道在一次调用里只能被写一次：批内第二条的 `old_lane` 与文档现值
        // 必然不符（模型会报 `OpStateMismatch`），因此在建提案之前就响亮拒绝。
        notes::reject_duplicate_lane_targets(&track_id, &note_ops)?;
        notes::compile(project, &track_id, &clip_id, &note_ops)?
    };
    let placement_description = match placement {
        Some(notes::PlacementEdit::Add(placement)) => {
            compiled.push(Op::AddClipPlacement {
                track_id,
                placement,
            });
            format!("摆放片段: {clip_id} → 音轨 {track_id}")
        }
        Some(notes::PlacementEdit::Move {
            placement_id,
            previous_start_tick,
            new_start_tick,
        }) => {
            compiled.push(Op::MoveClipPlacement {
                track_id,
                placement_id,
                old_start_tick: previous_start_tick,
                new_start_tick,
            });
            format!("平移摆放: {placement_id} → tick {new_start_tick}")
        }
        Some(notes::PlacementEdit::Remove {
            placement_id,
            previous_placement,
        }) => {
            compiled.push(Op::RemoveClipPlacement {
                track_id,
                placement_id,
                previous_placement,
            });
            format!("取走摆放: {placement_id}")
        }
        None => String::new(),
    };
    // 池级取走**跳过发声数检查**：那项检查量的是"这条片段在施加 ops 之后的音符重叠"，
    // 而池级取走一个音符都不读、取走之后池里也没有这条片段可量（旧代码会在克隆体上
    // 白跑一遍全文档模拟，然后读到一个已被取走的身份）。模型的 `ClipInUse` 前置条件
    // 仍由 `propose_draft` 的整批模拟把关 —— 那一步没有被跳过。
    if !removing_clip {
        notes::check_polyphony(project, &clip_id, &compiled)?;
    }
    // 描述按**实际内容**报（不把一次纯音轨级写入说成"音符编辑"，把池级取走说成
    // "音轨级编辑"，也不把一次纯路由边增益写入说成"音轨级编辑" —— 那是四个不同的
    // 对象）。三个非音符的桶各自计数，混合调用只报**真的出现过**的那些桶。
    let note_level = note_ops.iter().filter(|op| op.is_note_level()).count();
    let routing_level = note_ops.iter().filter(|op| op.is_routing_level()).count();
    let track_level = note_ops.len() - note_level - routing_level;
    let description = if removing_clip {
        format!("取走片段池条目: {clip_id}")
    } else if note_ops.is_empty() {
        placement_description
    } else if routing_level == note_ops.len() {
        format!("路由级编辑: {routing_level} 步")
    } else if note_level == 0 && routing_level == 0 {
        format!("音轨级编辑: {track_level} 步")
    } else if track_level == 0 && routing_level == 0 {
        format!("音符编辑: {note_level} 步")
    } else {
        let mut parts: Vec<String> = Vec::with_capacity(3);
        if note_level > 0 {
            parts.push(format!("音符编辑: {note_level} 步"));
        }
        if track_level > 0 {
            parts.push(format!("音轨级编辑: {track_level} 步"));
        }
        if routing_level > 0 {
            parts.push(format!("路由级编辑: {routing_level} 步"));
        }
        parts.join(" + ")
    };
    propose_draft(
        domain,
        project,
        "notes",
        description,
        format!("edit_notes {clip_id}"),
        compiled,
        include_ops(call),
    )
}

/// `yeban_set_macro`。
fn plan_set_macro(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let project = require_active(domain)?;
    let track_id = arg_id(call, "trackId")?;
    let macro_index = arg_u64(call, "macroIndex")?.unwrap_or(0);
    let macro_index = usize::try_from(macro_index).map_err(|_| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`macroIndex` 超出 usize 表示范围",
        )
    })?;
    let value = call
        .arguments
        .get("value")
        .and_then(Value::as_f64)
        .ok_or_else(|| Fault::domain(ErrorCode::InvalidParameterRange, "`value` 必须是数字"))?;
    // f64 → f32: 值域校验在 macros::plan 里, 这里只做宽度转换。
    #[allow(clippy::cast_possible_truncation)]
    let value = value as f32;
    let macro_plan = macros::plan(project, &track_id, macro_index, value)?;
    propose_draft(
        domain,
        project,
        "macro",
        format!("宏调整: 音轨 {track_id} 宏 #{macro_index} → {value}"),
        format!("set_macro {track_id}#{macro_index}"),
        macro_plan.ops,
        include_ops(call),
    )
}

/// `arguments.includeOps`（缺省 `false`）—— 三个提案类工具共用的**响应形状**开关
/// `[BASELINE-006]`，见 [`crate::tools::INCLUDE_OPS_PARAM`]。
fn include_ops(call: &ToolCall) -> bool {
    arg_bool(call, crate::tools::INCLUDE_OPS_PARAM, false)
}

/// `yeban_render_master`。
///
/// **只读**：参数校验 + 从活跃工程构造渲染计划、执行渲染、编码容器字节。
/// 渲染产物只放在 [`Plan::RenderMaster`] 里，落盘是 `apply` 的事 ——
/// 因此 `dryRun` 拿到的是**实测**的帧数/字节数/SHA-256，且一个字节都不写盘。
fn plan_render_master(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let project = require_active(domain)?;
    let project_path = domain.active_path().ok_or_else(no_active_project)?;
    let request = render::validate(&call.arguments)?;
    // 资产字节的唯一来源是会话 CAS 池（`assets/{sha256}`）；`Domain` 自己实现
    // `render::AssetStore`，因此渲染层不需要认识 `Domain` 的任何内部结构。
    let artifact = render::build(project, project_path, &request, domain.now_ms(), domain)?;
    Ok(Plan::RenderMaster {
        artifact: Box::new(artifact),
    })
}

/// `yeban_merge_proposal`。
fn plan_merge(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let proposal_id = arg_id(call, "proposalId")?;
    let snapshot = domain
        .proposal(&proposal_id)
        .ok_or_else(|| proposal_not_found(&proposal_id))?;
    let commit_message = arg_str(call, "commitMessage")
        .map_or_else(|| format!("merge proposal {proposal_id}"), str::to_owned);
    Ok(Plan::Merge {
        proposal_id,
        commit_message,
        snapshot: Box::new(snapshot.clone()),
    })
}

/// `yeban_reject_proposal`。
fn plan_reject(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let proposal_id = arg_id(call, "proposalId")?;
    let snapshot = domain
        .proposal(&proposal_id)
        .ok_or_else(|| proposal_not_found(&proposal_id))?;
    let reason =
        arg_str(call, "reason").map_or_else(|| String::from("(未提供原因)"), str::to_owned);
    Ok(Plan::Reject {
        proposal_id,
        reason,
        snapshot: Box::new(snapshot.clone()),
    })
}

/// `PROPOSAL_NOT_FOUND` 的载荷（带上已知提案，便于调用方自纠）。
fn proposal_not_found(id: &EntityId) -> Fault {
    Fault::domain_with_data(
        ErrorCode::ProposalNotFound,
        format!("提案不存在: {id}"),
        serde_json::json!({ "proposalId": id.to_canonical_string() }),
    )
}

// ---------------------------------------------------------------------------
// ADR-0001 D46 的三类扩展能力的 plan（只读）
// ---------------------------------------------------------------------------

/// `yeban_edit_automation` [ADR-0001 **D46** 第 1 类能力]。
///
/// 读的一半走**唯一求值入口**（`automation_value_at`），写的一半走
/// [`Op::SetAutomationPoint`]；两者都在 [`automation::plan`] 里完成（因此 `dryRun`
/// 拿到的是**同一份**规划数据）。
fn plan_edit_automation(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let project = require_active(domain)?;
    let track_id = arg_id(call, "trackId")?;
    let edit = automation::plan(project, &call.arguments, track_id)?;
    Ok(Plan::EditAutomation {
        edit: Box::new(edit),
    })
}

/// `yeban_query_engine_state` [ADR-0001 **D46** 第 2 类能力]（**只读**）。
///
/// 三份状态的来源见 [`engine_state`] 的模块文档：采样率读工程、走带读
/// `SessionRuntimeState`、缓冲读宿主注入的镜像。本函数**不**碰文件系统、**不**改状态。
///
/// 可选的 `since` 是**读数游标**（客户端上一个见过的 `readingsRevision`）：给了它，
/// 响应里就多一段 `readingsStream`（只含游标之后注入的读数）。缺省 ⇒ 形状不变。
fn plan_query_engine_state(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let project = require_active(domain)?;
    let track_id = match call.arguments.get("trackId") {
        Some(_) => Some(arg_id(call, "trackId")?),
        None => None,
    };
    let since = arg_u64(call, "since")?;
    let data = engine_state::snapshot(
        project,
        domain.session(),
        domain.engine_readings(),
        domain.undo_state().undone(),
        track_id,
        engine_state::ReadingsCursor {
            revision: domain.readings_revision(),
            tail: domain.readings_tail(),
            since,
        },
    )?;
    Ok(Plan::EngineState { data })
}

/// `yeban_import_audio` [ADR-0001 **D46** 第 3 类能力]。
///
/// 资产池的读法**复用** [`render::AssetStore`]（[`Domain`] 自己实现它），
/// 因此"池里有什么字节"只有一个事实源。
/// `yeban_export_diagnostics`：**不**要求活跃工程 —— 出问题时常常连工程都打不开，
/// 而日志与环境信息恰恰最该能采。
fn plan_export_diagnostics(_domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let export = diagnostics::plan(&call.arguments)?;
    Ok(Plan::Diagnostics {
        export: Box::new(export),
    })
}

fn plan_import_audio(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let project = require_active(domain)?;
    let import = import_audio::plan(project, domain, &call.arguments)?;
    Ok(Plan::ImportAudio {
        import: Box::new(import),
    })
}

/// `yeban_export_midi`（**只读**）：把活跃工程交给 `yeban-midi` 的**共享**映射层。
///
/// 本函数只拿 `&Domain`（借用检查器保证 `dryRun` 改不了任何状态），
/// 且**不碰文件系统**：字节留在 [`Plan::ExportMidi`] 里，由 `apply` 原样回传。
fn plan_export_midi(domain: &Domain, _call: &ToolCall) -> Result<Plan, Fault> {
    let project = require_active(domain)?;
    let export = export_midi::plan(project)?;
    Ok(Plan::ExportMidi {
        export: Box::new(export),
    })
}

// ---------------------------------------------------------------------------
// apply：可变
// ---------------------------------------------------------------------------

/// 施加一个计划。**这是唯一会改变状态的入口。**
///
/// 签名里没有 `&ToolCall`：十个工具的执行参数全部在 `plan` 阶段就被消化成
/// [`Plan`] 的字段（`RenderMaster` 连渲染都做完了），因此 `apply` 不需要再看原始实参 ——
/// 少一个参数就少一条"`plan` 与 `apply` 读的实参不一致"的漂移路径。
///
/// # Errors
///
/// 领域失败 → [`Fault::Domain`]（走 `ToolResponse`）；
/// 实现级状况 → [`Fault::Impl`]（走 JSON-RPC 错误对象）。
pub fn apply(domain: &mut Domain, plan: Plan) -> Result<ToolResponse, Fault> {
    // **施加修订号**在进 `apply_inner` 之前就定：`plan` 的所有权要交给 `apply_inner`，
    // 而"这个计划可不可能改工程"是**计划自己的**事实（[`Plan::mutates_project`]），
    // 不需要看施加结果。失败的计划不推进（下面只在 `Ok` 时提交这个读数）。
    let mutates = plan.mutates_project();
    let outcome = apply_inner(domain, plan);
    if mutates && outcome.is_ok() {
        domain.apply_revision = domain.apply_revision.saturating_add(1);
    }
    // 会话运行态里那份**撤销游标镜像**在唯一可变入口处同步（见 `Domain::session`）。
    // 放在这里而不是每个 `apply_*` 里：漏一处就会漂移，而这里是**唯一**的入口。
    domain.sync_session();
    outcome
}

/// **宿主（形态 A 的 GUI）的写入口** —— `ROAD-M4-008` 选项 (a) 第二片。
///
/// 它做的事只有三件：① 从 `&Domain` 只读预读"这次会处理哪些 op"（与真做同一口径：
/// [`undo_session::undo_op_kinds`] / [`undo_session::redo_op_kinds`]）；
/// ② 把动作包成 [`Plan::Host`] 交给 [`apply`]（**唯一可变入口** —— 施加、推进
/// [`Domain::apply_revision`]、[`Domain::sync_session`] 全都在那里发生，本函数
/// 不重复任何一步）；③ 读施加后的显示态并组装 [`HostOutcome`]。
///
/// # Errors
///
/// 与 [`apply`] 同：没有活跃工程、没有可撤销/可重做的历史、op 施加失败，
/// 或提交不带任何 op。
pub fn apply_host_action(domain: &mut Domain, action: HostAction) -> Result<HostOutcome, Fault> {
    let before = domain.undo_display();
    let committing = matches!(&action, HostAction::Commit { .. });
    let planned_op_kinds = match &action {
        HostAction::Undo { steps } => {
            undo_session::undo_op_kinds(domain.graph(), domain.undo_state(), *steps)
                .map_err(undo_refusal_to_fault)?
        }
        HostAction::Redo { steps } => {
            undo_session::redo_op_kinds(domain.graph(), domain.undo_state(), *steps)
                .map_err(undo_refusal_to_fault)?
        }
        HostAction::Commit { ops, .. } => {
            if ops.is_empty() {
                return Err(Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    "宿主动作提交必须携带至少一个 op（空批不改工程）",
                ));
            }
            // 提交在会话里恰好包成一条 `Op::Batch`（见 [`undo_session::commit`]）——
            // 名字取自模型自己的 `Op::name()`，不手抄字符串。
            let batch = Op::Batch {
                ops: Vec::new(),
                description: String::new(),
            };
            vec![batch.name()]
        }
    };
    // **唯一可变入口。**
    apply(domain, Plan::Host(action))?;
    let after = domain.undo_display();
    Ok(HostOutcome {
        steps: if committing {
            1
        } else {
            before.undone.abs_diff(after.undone)
        },
        undone_total: after.undone,
        op_kinds: planned_op_kinds.into_iter().map(str::to_owned).collect(),
        commit: if committing {
            domain.active_head()
        } else {
            None
        },
        display: after,
    })
}

/// **宿主的保存动作**（`ROAD-M4-008` 选项 (a)：单一写者会话）。
///
/// 它要证的那句话是：**控制面会话是唯一写者时，宿主的保存按钮也落到这同一个会话上。**
/// 因此它只做三件事：① 只读预读"这份工程现在会写出哪些字节"（[`save_material`]，与
/// `yeban_save_project` **同一个口径**）；② 在**同一个原子落盘入口**
/// [`store::write_project_atomic`] 上写出；③ 读回结构化读数 [`HostSaveOutcome`]。
///
/// ## 它不制造第二个写者，也不制造第二个推进点
///
/// - **不落回 `Plan` / `apply`**：保存**不改工程内容**，因此
///   [`Domain::apply_revision`] 在这里**不推进**（推进点仍然只有 [`apply`] 一处）。
///   它也不出现在 [`crate::tools::ToolCall`] 的契约里 —— 对外 JSON-RPC 面一位没变；
/// - **写入口与工具路径同一个**：字节由 [`save_material`] 产出、落盘由
///   [`store::write_project_atomic`] 完成 —— "写工程文档"仍然只有一份实现；
/// - **只对写会话开放**：`read_only` 是"这个会话可不可以写盘"的**唯一**事实源。
///   只读会话（内存样本 / 只读挂载）在这里同样被拒 —— 宿主保存**不是**绕过
///   `read_only` 的后门，它是"写会话的宿主落点"。
///
/// ## 目标路径可以不是会话路径
///
/// `--save-as` 语义（另存到别处）由此可行：容器字节来自**会话**（权威工程 + 图谱 +
/// 资产池），写出的目标由调用方给。只有目标与会话路径**逐字相同**时才更新内存里的
/// `saved_digest`（否则"未保存标记"会错误地声称另一个文件与内存一致）。
///
/// # Errors
///
/// 没有活跃工程、只读会话、工程 / 图谱序列化失败、容器层拒绝，或落盘 I/O 失败
/// （原样携带领域错误码，与工具路径同源）。
pub fn host_save_project(
    domain: &mut Domain,
    target: &Path,
    force: bool,
) -> Result<HostSaveOutcome, Fault> {
    let (bytes, digest, assets, history_commits, changed, session_path) = {
        let active = domain.active.as_ref().ok_or_else(no_active_project)?;
        if active.read_only {
            return Err(Fault::domain(
                ErrorCode::IoError,
                format!(
                    "工程 `{}` 是只读打开的, 宿主保存动作拒绝落盘",
                    active.path.display()
                ),
            ));
        }
        let (bytes, digest, assets, history_commits) = save_material(domain)?;
        let changed = digest != active.saved_digest;
        (
            bytes,
            digest,
            assets,
            history_commits,
            changed,
            active.path.clone(),
        )
    };
    if !force && !changed {
        return Ok(HostSaveOutcome {
            path: target.to_path_buf(),
            bytes: bytes.len(),
            skipped: true,
            assets,
            history_commits,
            digest,
        });
    }
    store::write_project_atomic(target, &bytes)?;
    if session_path.as_path() == target
        && let Some(active) = domain.active.as_mut()
    {
        active.saved_digest.clone_from(&digest);
    }
    Ok(HostSaveOutcome {
        path: target.to_path_buf(),
        bytes: bytes.len(),
        skipped: false,
        assets,
        history_commits,
        digest,
    })
}

/// [`Plan::Host`] 的施加：**复用**工具路径的那两个函数（撤销 / 重做），
/// 以及本文件里唯一的宿主动作提交实现。
fn apply_host_plan(domain: &mut Domain, action: HostAction) -> Result<ToolResponse, Fault> {
    match action {
        // 与 `yeban_undo` / `yeban_redo` **同一个**施加函数、同一份响应形状。
        HostAction::Undo { steps } => apply_undo(domain, steps),
        HostAction::Redo { steps } => apply_redo(domain, steps),
        HostAction::Commit {
            now_ms,
            origin,
            message,
            ops,
        } => apply_host_commit(domain, now_ms, origin, message, ops),
    }
}

/// 宿主动作的一次提交：**恰好**一条 `Op::Batch` 提交（= 一步撤销），与
/// [`undo_session::commit`] 同一条路 —— 逆操作仍由模型提供，本层不写第二份。
fn apply_host_commit(
    domain: &mut Domain,
    now_ms: u64,
    origin: OpOrigin,
    message: String,
    ops: Vec<Op>,
) -> Result<ToolResponse, Fault> {
    if ops.is_empty() {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            "宿主动作提交必须携带至少一个 op（空批不改工程）",
        ));
    }
    let id = {
        let Domain {
            active,
            graph,
            undo,
            ..
        } = domain;
        let active = active.as_mut().ok_or_else(no_active_project)?;
        undo_session::commit(
            graph,
            &mut active.project,
            undo,
            CommitRequest {
                now_ms,
                origin,
                message,
                ops,
            },
        )
        .map_err(undo_refusal_to_fault)?
    };
    let display = domain.undo_display();
    Ok(ToolResponse::success(serde_json::json!({
        "committed": true,
        "commit": id.to_canonical_string(),
        "cursorPersisted": false,
        "after": undo_display_value(&display),
    })))
}

/// [`apply`] 的本体（同步会话态的那一步在外面，见上）。
fn apply_inner(domain: &mut Domain, plan: Plan) -> Result<ToolResponse, Fault> {
    match plan {
        Plan::Open(request) => apply_open(domain, *request),
        Plan::Save {
            path,
            bytes,
            digest,
            force,
            changed,
            ..
        } => apply_save(domain, &path, &bytes, digest, force, changed),
        Plan::Close {
            save_first,
            save,
            path,
        } => apply_close(domain, save_first, save, path),
        Plan::Query { data } => Ok(ToolResponse::success(data)),
        Plan::Propose { draft, include_ops } => apply_propose(domain, *draft, include_ops),
        Plan::Merge {
            proposal_id,
            commit_message,
            snapshot,
        } => apply_merge(domain, proposal_id, &commit_message, &snapshot),
        Plan::Reject {
            proposal_id,
            reason,
            snapshot,
        } => apply_reject(domain, proposal_id, &reason, &snapshot),
        Plan::RenderMaster { artifact } => apply_render(*artifact),
        Plan::Undo { steps, .. } => apply_undo(domain, steps),
        Plan::Redo { steps, .. } => apply_redo(domain, steps),
        // 宿主动作（`ROAD-M4-008` 选项 (a) 第二片）：`plan` 永不产出它，只有
        // [`apply_host_action`] 会构造它 —— 施加走的就是这条同一的链。
        Plan::Host(action) => apply_host_plan(domain, action),
        Plan::EditAutomation { edit } => automation::apply(domain, &edit),
        // 只读：`plan` 已经把读数组装好了，`apply` 原样返回（一位都不改）。
        Plan::EngineState { data } => Ok(ToolResponse::success(data)),
        Plan::ImportAudio { import } => import_audio::apply(domain, &import),
        Plan::Diagnostics { export } => diagnostics::apply(domain, &export),
        // 只读：字节在 `plan` 里就编好了，`apply` 原样回传（一位都不改、不落盘）。
        Plan::ExportMidi { export } => Ok(ToolResponse::success(export.data())),
    }
}

/// 撤销 / 重做响应里那份**模型读数**（界面与工具同源：`Domain::undo_display`）。
fn undo_display_value(display: &undo_session::UndoDisplay) -> Value {
    serde_json::json!({
        "branch": display.branch,
        "head": display.head.map(|head| head.to_canonical_string()),
        "commitCount": display.commit_count,
        "branchCount": display.branch_count,
        "undone": display.undone,
        "undoable": display.undoable,
        "canUndo": display.can_undo,
        "canRedo": display.can_redo,
    })
}

/// `yeban_undo` 的施加 [ADR-0001 **D45**]。
///
/// 逆操作的施加**只有一条路**：`crate::undo_session::undo` → `CommitGraph::undo_with`。
/// 游标是会话运行态，因此响应里明确标 `cursorPersisted: false`
/// （判据 `the_cursor_never_reaches_the_project_container` 用逐字节证据钉住它）。
fn apply_undo(domain: &mut Domain, steps: usize) -> Result<ToolResponse, Fault> {
    let display_before = domain.undo_display();
    let outcome = {
        let Domain {
            active,
            graph,
            undo,
            ..
        } = domain;
        let active = active.as_mut().ok_or_else(no_active_project)?;
        undo_session::undo(graph, &mut active.project, undo, steps)
            .map_err(undo_refusal_to_fault)?
    };
    let display_after = domain.undo_display();
    let digest = domain.active_project().map_or(Value::Null, |project| {
        store::serialize_project(project)
            .ok()
            .map_or(Value::Null, |json| {
                Value::from(store::digest_of(json.as_bytes()))
            })
    });
    Ok(ToolResponse::success(serde_json::json!({
        "undone": true,
        "requestedSteps": steps,
        "steps": outcome.steps,
        "undoneTotal": outcome.undone_total,
        "opKinds": outcome.op_kinds,
        "branch": display_after.branch,
        "projectDigest": digest,
        "cursorPersisted": false,
        "before": undo_display_value(&display_before),
        "after": undo_display_value(&display_after),
    })))
}

/// `yeban_redo` 的施加 [ADR-0001 **D45**]：按**正向** `Op::apply` 把刚撤销的 op 再打一次。
fn apply_redo(domain: &mut Domain, steps: usize) -> Result<ToolResponse, Fault> {
    let display_before = domain.undo_display();
    let outcome = {
        let Domain {
            active,
            graph,
            undo,
            ..
        } = domain;
        let active = active.as_mut().ok_or_else(no_active_project)?;
        undo_session::redo(graph, &mut active.project, undo, steps)
            .map_err(undo_refusal_to_fault)?
    };
    let display_after = domain.undo_display();
    let digest = domain.active_project().map_or(Value::Null, |project| {
        store::serialize_project(project)
            .ok()
            .map_or(Value::Null, |json| {
                Value::from(store::digest_of(json.as_bytes()))
            })
    });
    Ok(ToolResponse::success(serde_json::json!({
        "redone": true,
        "requestedSteps": steps,
        "steps": outcome.steps,
        "undoneTotal": outcome.undone_total,
        "opKinds": outcome.op_kinds,
        "branch": display_after.branch,
        "projectDigest": digest,
        "cursorPersisted": false,
        "before": undo_display_value(&display_before),
        "after": undo_display_value(&display_after),
    })))
}

/// `yeban_render_master` 的施加：把**已经渲染好**的容器字节原子落盘。
///
/// 渲染本身在 `plan` 里完成（只读），这里只有一次 [`store::write_project_atomic`] ——
/// 即 `ARCH-SEC-004` 的同目录临时文件 + `fsync` + `rename`。失败时原文件不受影响、
/// 临时文件被清理，**不会留下半个母带**。
fn apply_render(artifact: render::RenderArtifact) -> Result<ToolResponse, Fault> {
    store::write_project_atomic(&artifact.path, &artifact.bytes)?;
    Ok(ToolResponse::success(artifact.response_data()))
}

/// `yeban_open_project` 的施加。
///
/// 打开一份 `.yeban` 容器会**恢复** `history.dag`（提交图谱）与
/// `assets/{sha256}`（CAS 池）—— `ADR-0001 D43` 之后这是唯一的读路径。
fn apply_open(domain: &mut Domain, request: OpenRequest) -> Result<ToolResponse, Fault> {
    let OpenRequest {
        path,
        read_only,
        project,
        digest,
        bytes,
        already_open,
        history,
        assets,
        created,
    } = request;
    let asset_count = assets.len();
    let history_commits = history.as_ref().map_or(0, |graph| graph.commit_count());
    if already_open {
        // 幂等: 同一个工程重复打开不是错误, 但也不重建历史。
        let summary = domain.active_project().map_or(Value::Null, project_summary);
        return Ok(ToolResponse::success(serde_json::json!({
            "opened": true,
            "alreadyOpen": true,
            "path": path.display().to_string(),
            "readOnly": domain.is_read_only(),
            "lockMode": lock_mode(domain.is_read_only()).as_str(),
            "locked": true,
            "advisoryLock": true,
            "tookOverStaleLock": false,
            "bytes": bytes,
            "format": store::DOCUMENT_FORMAT,
            "historyRestored": false,
            "assets": asset_count,
            "projectDigest": digest,
            "project": summary,
        })));
    }
    // ---- `create: true`：先把新文档原子落盘, 再取锁并成为活跃工程 ----
    //
    // 顺序是刻意的: 落盘在**取锁之前**。若反过来（先取锁再写盘），一次写盘失败
    // 会留下一个 `.yeban.lock` 与"没有工程"的状态；而先写盘时，写失败只是
    // "什么都没发生"（临时文件在 `write_project_atomic` 里被清理）。
    //
    // 覆盖保护在这里**再查一次**：`plan` 与 `apply` 之间有一段时间窗，
    // 期间另一个进程可能把文件放进来。第二次检查把"已存在的文件被静默覆盖"
    // 压缩到"两次 `exists()` 之间的纳秒级窗口"，并且写盘本身走的是
    // 同目录临时文件 + `rename`（`ARCH-SEC-004`），目标文件在 `rename` 之前
    // 一直保持原样。
    if created.is_some() && path.exists() {
        return Err(Fault::domain_with_data(
            ErrorCode::Conflict,
            format!("工程在新建落盘前已出现, 拒绝覆盖: {}", path.display()),
            serde_json::json!({
                "reason": "projectAlreadyExists",
                "path": path.display().to_string(),
            }),
        ));
    }
    if let Some(seed) = created.as_ref() {
        store::write_project_atomic(&path, &seed.bytes)?;
    }
    let lock = store::lock(&path, lock_mode(read_only))?;
    let took_over_stale_lock = lock.took_over_stale_lock();
    domain.reset_history(SessionSeed {
        path: path.clone(),
        read_only,
        project: *project,
        digest: digest.clone(),
        lock: Some(lock),
        history: history.map(|graph| *graph),
        assets,
    })?;
    let seed_summary = created.as_ref().map(|seed| seed.summary.clone());
    Ok(ToolResponse::success(serde_json::json!({
        "opened": true,
        "alreadyOpen": false,
        "created": created.is_some(),
        "path": path.display().to_string(),
        "readOnly": read_only,
        // 排他写 = 独占; 共享读 = 与其他读者共存 (ARCH-SEC-001 第 3 条)。
        "lockMode": lock_mode(read_only).as_str(),
        "locked": true,
        "advisoryLock": true,
        // 崩溃遗留的陈旧锁被本次打开接管并重写 —— 这是 MUST-GATE-008
        // "不留下永久锁"的可观察证据。
        "tookOverStaleLock": took_over_stale_lock,
        "lockFile": store::lock_path(&path).display().to_string(),
        "bytes": bytes,
        // 形态（唯一取值）+ 容器里另外两类条目的实际装载量（如实上报，不假装）。
        "format": store::DOCUMENT_FORMAT,
        "historyRestored": history_commits > 0,
        "historyCommits": domain.commit_count(),
        "assets": domain.asset_count(),
        "projectDigest": digest,
        "project": domain
            .active_project()
            .map_or(Value::Null, project_summary),
        "seed": seed_summary.unwrap_or(Value::Null),
    })))
}

/// `readOnly` 参数 → 锁模式（`ARCH-SEC-001` 第 3 条的双模式）。
fn lock_mode(read_only: bool) -> store::LockMode {
    if read_only {
        store::LockMode::SharedRead
    } else {
        store::LockMode::ExclusiveWrite
    }
}

/// `yeban_save_project` 的施加。
///
/// `bytes` 是**容器字节**（`ARCH-SEC-003`），落盘协议仍是 `ARCH-SEC-004` 的
/// 同目录临时文件 + `fsync` + `rename`（唯一入口 [`store::write_project_atomic`]）。
fn apply_save(
    domain: &mut Domain,
    path: &Path,
    bytes: &[u8],
    digest: String,
    force: bool,
    changed: bool,
) -> Result<ToolResponse, Fault> {
    if !force && !changed {
        return Ok(ToolResponse::success(serde_json::json!({
            "saved": false,
            "skipped": true,
            "reason": "内存状态与磁盘一致; 传 force: true 可强制落盘",
            "path": path.display().to_string(),
            "bytes": bytes.len(),
            "format": store::DOCUMENT_FORMAT,
            "projectDigest": digest,
        })));
    }
    store::write_project_atomic(path, bytes)?;
    if let Some(active) = domain.active.as_mut() {
        active.saved_digest.clone_from(&digest);
    }
    Ok(ToolResponse::success(serde_json::json!({
        "saved": true,
        "skipped": false,
        "atomic": true,
        // 落盘形态与容器里的条目数（2 = project.json + history.dag，其余是资产）。
        "format": store::DOCUMENT_FORMAT,
        "path": path.display().to_string(),
        "bytes": bytes.len(),
        "assets": domain.asset_count(),
        "historyCommits": domain.commit_count(),
        "projectDigest": digest,
        "forced": force,
    })))
}

/// `yeban_close_project` 的施加。
fn apply_close(
    domain: &mut Domain,
    save_first: bool,
    save: Option<Box<Plan>>,
    path: PathBuf,
) -> Result<ToolResponse, Fault> {
    let mut saved = false;
    if let Some(Plan::Save {
        path: save_path,
        bytes,
        digest,
        force,
        changed,
        ..
    }) = save.map(|boxed| *boxed)
    {
        apply_save(domain, &save_path, &bytes, digest, force, changed)?;
        saved = true;
    }
    let was_read_only = domain.is_read_only();
    let lock_file = domain.lock_path().map(|lock| lock.display().to_string());
    let released = domain.release().is_some();
    Ok(ToolResponse::success(serde_json::json!({
        "closed": true,
        "path": path.display().to_string(),
        "saveFirst": save_first,
        "saved": saved,
        "releasedLock": released && !was_read_only,
        "releasedLockFile": lock_file,
        "commitCount": domain.commit_count(),
    })))
}

/// 提案类工具：创建隔离分支 + 提交 + 记录。
///
/// `include_ops` 只选择**回传哪一份记录投影**：`true` ⇒ [`Proposal::detail`]（含逐条
/// op 载荷，审查用途）；`false`（缺省）⇒ [`Proposal::summary`]（结构化字段）
/// `[BASELINE-006]`。两种取值下**被创建的提案逐字节相同** —— 同一份 `record` 先插进
/// `domain.proposals`，只有回传的那一份 JSON 不同。
fn apply_propose(
    domain: &mut Domain,
    draft: ProposalDraft,
    include_ops: bool,
) -> Result<ToolResponse, Fault> {
    let project = domain
        .active_project()
        .cloned()
        .ok_or_else(no_active_project)?;
    let now_ms = domain.now_ms();
    let proposal_id = EntityId::new();
    let branch = format!(
        "{PROPOSAL_BRANCH_PREFIX}{}",
        proposal_id.to_canonical_string()
    );
    // "将要新建哪些实体"的派生清单：与 `dryRun` 预览同源（同一个 `summarize_ops`），
    // 因此"预览说的"与"提交的"不可能漂移。
    let will_create = section_build::summarize_ops(&draft.ops);
    let stamped: Vec<StampedOp> = draft
        .ops
        .iter()
        .map(|op| {
            StampedOp::new(
                OpOrigin::McpProposal {
                    proposal_id,
                    agent_name: AGENT_NAME.to_owned(),
                },
                now_ms,
                op.clone(),
            )
        })
        .collect();
    let head_commit = {
        // 隔离分支建在**提案的基线提交**上（`CommitGraph::create_branch`），再在上面写
        // 提案提交（`CommitGraph::append`）。于是"提案基于哪个提交"是**图谱里的一条父边**，
        // 而不是只写在 `Proposal` 记录里的一个字面量。
        //
        // 为什么不是 `genesis`（孤立根提交）：孤立根与基线之间没有任何父边 ⇒ 只看
        // `history.dag` 无法说出这个提案是从哪儿分出来的。两处的差别只在图谱写什么，
        // 主分支的头、工程字节与 op 日志都不动（`create_branch` **不**写提交，
        // 且**不**动任何既有分支头）。
        domain
            .graph
            .create_branch(branch.clone(), &draft.base_commit)
            .map_err(|failure| error::from_model("提案分支", &failure))?;
        domain
            .graph
            .append(
                CommitDraft::new(
                    EntityId::new(),
                    branch.clone(),
                    AGENT_NAME,
                    draft.title.clone(),
                )
                .with_created_at(now_ms)
                .with_ops(stamped.clone()),
            )
            .map_err(|failure| error::from_model("提案分支", &failure))?
    };
    let record = Proposal {
        id: proposal_id,
        branch: branch.clone(),
        kind: draft.kind,
        title: draft.title.clone(),
        description: draft.description.clone(),
        status: ProposalStatus::Open,
        base_commit: draft.base_commit,
        head_commit,
        merge_commit: None,
        ops: stamped,
        created_at: now_ms,
        resolved_at: None,
        resolution: None,
    };
    // 回传哪一份记录投影：缺省只回结构化字段；`includeOps: true` 才回逐条 op 载荷。
    let wire_projection = if include_ops {
        record.detail()
    } else {
        record.summary()
    };
    domain.proposals.insert(proposal_id, record);

    // 提案**不**改工程内容 —— 这两行是给调用方的证据, 不是顺手加的字段。
    let project_digest = store::digest_of(store::serialize_project(&project)?.as_bytes());
    let unwired = draft_unwired(&wire_projection);
    Ok(ToolResponse::success(serde_json::json!({
        "created": true,
        "projectUnchanged": true,
        "projectDigest": project_digest,
        "unwired": unwired,
        "willCreate": will_create,
        "proposal": wire_projection,
    })))
}

/// 提案类工具里**明确没接线**的那部分（如实上报，不藏在错误码后面）。
///
/// ## 它是**推导**出来的，不是硬编码的声明
///
/// `yeban_propose_section` 曾经在这里写死 `["clipPoolEntries","routingEdges"]`，
/// 理由是"`Op` 全集没有 `AddClip`/`AddRoutingNode`"。那个理由在 `ADR-0001` **D27**
/// （`Op` 23 → 27，2026-10-04 追认）之后**已经不成立** —— 写死的声明于是变成了
/// 一条**过期的自我限制**：Agent 会相信它，然后绕开 `Op` 日志（丢掉撤销语义）。
///
/// 现在改成从提案**真实的 `opKinds`** 推导
/// （[`section_build::unwired_for_section_op_kinds`]）：代码真的生成了片段与声部连接，
/// 响应就报空；哪一天某条相位被删掉，响应会**自己**把对应的键报回来。
fn draft_unwired(detail: &Value) -> Vec<&'static str> {
    match detail.get("kind").and_then(Value::as_str) {
        Some("section") => {
            let kinds: Vec<&str> = detail
                .get("opKinds")
                .and_then(Value::as_array)
                .map(|kinds| kinds.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            section_build::unwired_for_section_op_kinds(&kinds)
        }
        _ => Vec::new(),
    }
}

/// `yeban_merge_proposal` 的施加。
fn apply_merge(
    domain: &mut Domain,
    proposal_id: EntityId,
    commit_message: &str,
    snapshot: &Proposal,
) -> Result<ToolResponse, Fault> {
    // 幂等: 已合并的提案再合并一次, 返回**同一条**合并提交, 不重复施加。
    if snapshot.status == ProposalStatus::Merged {
        return Ok(ToolResponse::success(serde_json::json!({
            "merged": true,
            "alreadyMerged": true,
            "appliedOps": 0,
            "proposal": snapshot.summary(),
        })));
    }
    if snapshot.status == ProposalStatus::Rejected {
        return Err(Fault::domain_with_data(
            ErrorCode::Conflict,
            format!("提案 {proposal_id} 已被拒绝, 不能再合并"),
            serde_json::json!({ "proposalId": proposal_id.to_canonical_string() }),
        ));
    }
    let now_ms = domain.now_ms();
    let current = domain
        .active_project()
        .cloned()
        .ok_or_else(no_active_project)?;
    // "基线移动过没有"必须看**活跃**头：撤销之后继续合并会派生匿名分支，
    // 盯着 `main` 会把提案挂到一条只读孤岛上（见 `Domain::active_head`）。
    let base_moved = domain.active_head() != Some(snapshot.base_commit);
    let ops: Vec<Op> = snapshot
        .ops
        .iter()
        .map(|stamped| stamped.op.clone())
        .collect();
    // 预演一遍（保持既有错误语义：CONFLICT + 结构化 data）。
    // 真正的施加与提交都交给 `crate::undo_session::commit`（唯一实现）。
    let mut probe = current;
    merge_batch(snapshot, commit_message)
        .apply(&mut probe)
        .map_err(|failure| {
            Fault::domain_with_data(
                ErrorCode::Conflict,
                format!("提案无法合并到当前工程: {failure}"),
                serde_json::json!({
                    "proposalId": proposal_id.to_canonical_string(),
                    "model": format!("{failure:?}"),
                    "baseCommitMoved": base_moved,
                }),
            )
        })?;
    probe
        .validate()
        .map_err(|failure| error::from_model("合并结果校验", &failure))?;

    let merge_commit = {
        let Domain {
            active,
            graph,
            undo,
            ..
        } = domain;
        let active = active.as_mut().ok_or_else(no_active_project)?;
        // **多父合并提交**（`docs/ledger/tools-domain-notes.md` 的 needs-5 第二半）：
        // 额外父 = 提案分支的头提交 ⇒ `history.dag` 自己就说得清"这次合并并了谁"。
        // 第一父仍是活跃分支头（`append_merge` 的既定语义）⇒ 主干方向与撤销不变。
        undo_session::commit_merge(
            graph,
            &mut active.project,
            undo,
            CommitRequest {
                now_ms,
                origin: OpOrigin::McpProposal {
                    proposal_id,
                    agent_name: AGENT_NAME.to_owned(),
                },
                message: commit_message.to_owned(),
                ops,
            },
            &[snapshot.head_commit],
        )
        .map_err(undo_refusal_to_fault)?
    };

    // 父集合从**图谱里读回**，不把"我请求的父"当成事实：有已撤销的步骤时提交落在
    // `fork_anonymous` 上（单父），此时上报一条 `isMerge: false` 才是真话。
    let commit_record = domain
        .graph()
        .commit(&merge_commit)
        .map_err(|failure| error::from_model("合并提交读取", &failure))?;
    let commit_parents: Vec<Value> = commit_record
        .parents
        .iter()
        .map(|parent| Value::from(parent.to_canonical_string()))
        .collect();
    let commit_is_merge = commit_record.is_merge();
    // 深度缓存与提交集合的键集**恒**相同（模型 `CommitGraph::validate` 的第 3 条不变量）
    // ⇒ 这里读不到深度是图谱损坏，按领域失败上报，绝不静默记成 0。
    let commit_depth = domain
        .graph()
        .depth_of(&merge_commit)
        .map_err(|failure| error::from_model("合并提交深度读取", &failure))?;

    let new_digest = domain
        .active_project()
        .map(store::serialize_project)
        .transpose()?
        .map_or(Value::Null, |json| {
            Value::from(store::digest_of(json.as_bytes()))
        });
    if let Some(record) = domain.proposals.get_mut(&proposal_id) {
        record.status = ProposalStatus::Merged;
        record.merge_commit = Some(merge_commit);
        record.resolved_at = Some(now_ms);
        record.resolution = Some(commit_message.to_owned());
    }
    let branch = domain.active_branch().to_owned();
    let detail = domain
        .proposal(&proposal_id)
        .map(Proposal::detail)
        .unwrap_or(Value::Null);
    Ok(ToolResponse::success(serde_json::json!({
        "merged": true,
        "alreadyMerged": false,
        "appliedOps": snapshot.ops.len(),
        "baseCommitMoved": base_moved,
        "projectDigest": new_digest,
        "commit": {
            "id": merge_commit.to_canonical_string(),
            // 撤销之后继续合并会落在 `anon-<ulid>` 上（模型的 `fork_anonymous`），
            // 因此这里**如实**报活跃分支，而不是永远写 `main`。
            "branch": branch,
            "message": commit_message,
            "atomicBatch": true,
            "parents": commit_parents,
            "parentCount": commit_parents.len(),
            "isMerge": commit_is_merge,
            "depth": commit_depth,
        },
        "proposal": detail,
    })))
}

/// `yeban_reject_proposal` 的施加（只改记录，**绝不**删除）。
fn apply_reject(
    domain: &mut Domain,
    proposal_id: EntityId,
    reason: &str,
    snapshot: &Proposal,
) -> Result<ToolResponse, Fault> {
    if snapshot.status == ProposalStatus::Rejected {
        return Ok(ToolResponse::success(serde_json::json!({
            "rejected": true,
            "alreadyRejected": true,
            "proposal": snapshot.summary(),
        })));
    }
    if snapshot.status == ProposalStatus::Merged {
        return Err(Fault::domain_with_data(
            ErrorCode::Conflict,
            format!("提案 {proposal_id} 已合并, 不能再拒绝"),
            serde_json::json!({ "proposalId": proposal_id.to_canonical_string() }),
        ));
    }
    let now_ms = domain.now_ms();
    if let Some(record) = domain.proposals.get_mut(&proposal_id) {
        record.status = ProposalStatus::Rejected;
        record.resolved_at = Some(now_ms);
        record.resolution = Some(reason.to_owned());
    }
    let detail = domain
        .proposal(&proposal_id)
        .map(Proposal::detail)
        .unwrap_or(Value::Null);
    Ok(ToolResponse::success(serde_json::json!({
        "rejected": true,
        "alreadyRejected": false,
        "archived": true,
        "reason": reason,
        "proposal": detail,
    })))
}

// ---------------------------------------------------------------------------
// 对外的两个入口
// ---------------------------------------------------------------------------

/// 真调用：`plan` + `apply`。
///
/// # Errors
///
/// 只有**实现级**状况才返回 `Err`（JSON-RPC 错误对象）；领域失败一律
/// `Ok(ToolResponse{status:"error"})` —— 契约要求领域失败带内传递。
pub fn execute(domain: &mut Domain, call: &ToolCall) -> Result<Value, ErrorObject> {
    let planned = match plan(domain, call) {
        Ok(planned) => planned,
        Err(fault) => return fault.into_result(),
    };
    match apply(domain, planned) {
        Ok(response) => Ok(response.to_value()),
        Err(fault) => fault.into_result(),
    }
}

/// `dryRun` 的领域侧：**只读**差异预览载荷。
///
/// 返回的 `Value` 直接进 `ToolResponse.data.preview`；信封里的
/// `dryRun` / `tool` / `specId` / `sideEffect` 等字段由 [`crate::dispatch`] 从注册表派生。
///
/// # Errors
///
/// 领域失败（参数/领域合法性）与实现级状况都通过 [`Fault`] 返回，
/// 由调用方用 [`Fault::into_result`] 分流。
pub fn preview(domain: &Domain, call: &ToolCall) -> Result<Value, Fault> {
    plan(domain, call).and_then(|planned| planned.describe(domain))
}

impl render::AssetStore for Domain {
    /// 会话 CAS 池的只读投影（`ARCH-SEC-003` 的 `assets/{sha256}`）。
    ///
    /// 语义与 [`Domain::asset`] 完全一致 —— 这里不复制一份查找逻辑，
    /// 而是直接转发，避免"池的读法"出现第二个事实源。
    fn asset(&self, hash: &AssetHash) -> Option<&[u8]> {
        Domain::asset(self, hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr as _;
    use std::sync::OnceLock;

    use crate::tools::ToolCall;

    /// 一个**保证不存在**的工程路径（父目录也没建）。
    ///
    /// 每个测试二进制只算一次，因此同一测试里的多次调用拿到同一个路径
    /// （`yeban_open_project` 的幂等分支需要它）。绝不污染仓库。
    fn unique_path() -> PathBuf {
        static PATH: OnceLock<PathBuf> = OnceLock::new();
        PATH.get_or_init(|| {
            std::env::temp_dir()
                .join(format!(
                    "yeban-mcp-unit-{}-{}",
                    std::process::id(),
                    EntityId::new().to_canonical_string()
                ))
                .join("demo.yeban")
        })
        .clone()
    }

    fn call(name: &str, arguments: Value) -> ToolCall {
        ToolCall::from_params(Some(&serde_json::json!({
            "name": name,
            "arguments": arguments,
        })))
        .expect("合法的工具调用")
    }

    /// 一份**真 WAV**（16-bit 单声道 48 kHz, 4 帧）—— `yeban_import_audio` 的规划会真的解码它。
    ///
    /// ⚠ 用**自己的独占临时目录**（`unique_path()` 之外的新目录）。CI 实测教训：
    /// 第一版把它放进了 `unique_path()` 的父目录并 `create_dir_all` —— 于是
    /// `save_without_changes_is_skipped_unless_forced` 的**前提**（"那个父目录不存在 ⇒
    /// `force: true` 会撞 `IO_ERROR`"）被本夹具悄悄改掉了，该判据在并行执行下随机变红。
    /// **夹具不得改动别的判据依赖的路径前提。**
    fn wav_fixture() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-unit-wav-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("fixture.wav");
        let samples: [i16; 4] = [0, 1_000, -1_000, 0];
        let data: Vec<u8> = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&u32::try_from(36 + data.len()).expect("小").to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&1_u16.to_le_bytes()); // 单声道
        bytes.extend_from_slice(&48_000_u32.to_le_bytes());
        bytes.extend_from_slice(&(48_000_u32 * 2).to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&u32::try_from(data.len()).expect("小").to_le_bytes());
        bytes.extend_from_slice(&data);
        std::fs::write(&path, &bytes).expect("写 WAV 夹具");
        path
    }

    fn domain() -> Domain {
        let mut domain = Domain::new();
        domain.set_now_ms(1_760_000_000_000);
        domain
            .open_in_memory(unique_path(), yeban_model::samples::filled_project(), false)
            .expect("注入工程");
        domain
    }

    fn fixture_track(domain: &Domain) -> EntityId {
        domain
            .active_project()
            .expect("工程")
            .tracks
            .values()
            .find(|track| !track.macros.is_empty())
            .expect("带宏的音轨")
            .id
    }

    #[test]
    fn every_tool_has_a_plan_arm_and_no_not_implemented_report() {
        // 十个工具都必须能进 `plan`（不再一律 -32005）。
        let domain = domain();
        let track = fixture_track(&domain);
        let cases: Vec<(&str, Value)> = vec![
            (
                "yeban_open_project",
                serde_json::json!({"path": unique_path()}),
            ),
            ("yeban_save_project", serde_json::json!({})),
            ("yeban_close_project", serde_json::json!({})),
            ("yeban_query_project", serde_json::json!({})),
            (
                "yeban_propose_section",
                serde_json::json!({"sectionName": "Chorus", "stylePreset": "lo_fi_hip_hop", "bars": 8}),
            ),
            (
                "yeban_edit_notes",
                serde_json::json!({
                    "trackId": track.to_canonical_string(),
                    "clipId": clip_id(&domain).to_canonical_string(),
                    "ops": [{"kind": "velocity", "noteId": note_id(&domain).to_canonical_string(), "velocity": 64}]
                }),
            ),
            (
                "yeban_set_macro",
                serde_json::json!({"trackId": track.to_canonical_string(), "macroIndex": 0, "value": 0.75}),
            ),
            (
                "yeban_render_master",
                serde_json::json!({"format": "wav", "sampleRate": 48000}),
            ),
            (
                "yeban_merge_proposal",
                serde_json::json!({"proposalId": EntityId::new().to_canonical_string(), "commitMessage": "m"}),
            ),
            (
                "yeban_reject_proposal",
                serde_json::json!({"proposalId": EntityId::new().to_canonical_string(), "reason": "r"}),
            ),
            // D45 的两条扩展: 刚打开的会话一条 op 都没提交 ⇒ 规划阶段就该报"撤不动"。
            ("yeban_undo", serde_json::json!({})),
            ("yeban_redo", serde_json::json!({})),
            // D46 的三类扩展（`ADR-0001` D46）: 三者都必须**规划成功**。
            // 自动化那条只读（没有 `point`）; 引擎读数那条只读;
            // 音频导入那条指向一份**真 WAV**（`plan` 会真的解码 + 过 `PcmBudget`）。
            (
                "yeban_edit_automation",
                serde_json::json!({
                    "trackId": track.to_canonical_string(),
                    "lane": "TrackVolume",
                    "ticks": [0, 1920],
                }),
            ),
            (
                "yeban_query_engine_state",
                serde_json::json!({"trackId": track.to_canonical_string()}),
            ),
            (
                "yeban_import_audio",
                serde_json::json!({
                    "name": "Kick",
                    "path": wav_fixture().display().to_string(),
                }),
            ),
            // [D56] 诊断导出：空参数合法（缺省写到当前目录），因此走 `_` 分支的"规划必须成功"。
            ("yeban_export_diagnostics", serde_json::json!({})),
            // SMF 导出：**只读**且没有可调参数 —— `filled_project` 有 MIDI 内容 ⇒ 规划成功。
            ("yeban_export_midi", serde_json::json!({})),
        ];
        for spec in &crate::tools::TOOLS {
            assert!(
                cases.iter().any(|(name, _)| *name == spec.name),
                "少了 {} 的用例",
                spec.name
            );
        }
        for (name, arguments) in cases {
            let call = call(name, arguments);
            let outcome = plan(&domain, &call);
            // 每个工具在这个夹具下的**预期**结果:
            // - merge / reject 用的是随机提案身份 ⇒ PROPOSAL_NOT_FOUND;
            // - open 指向一个磁盘上不存在的路径 ⇒ FILE_NOT_FOUND
            //   （规划阶段真的读盘 —— 那正是"打开"应当做的事）;
            // - undo / redo 在新会话上没有历史 ⇒ INDEX_OUT_OF_BOUNDS
            //   （D25 的联集里没有 NO_HISTORY, 且不许发明新码）;
            // - 其余工具必须规划成功 ⇒ 再没有"一律 -32005"这回事。
            match name {
                "yeban_merge_proposal" | "yeban_reject_proposal" => {
                    let fault = outcome.expect_err("未知提案");
                    assert_eq!(fault.domain_code(), Some(ErrorCode::ProposalNotFound));
                }
                "yeban_open_project" => {
                    let fault = outcome.expect_err("路径不存在");
                    assert_eq!(fault.domain_code(), Some(ErrorCode::FileNotFound));
                }
                "yeban_undo" | "yeban_redo" => {
                    let fault = outcome.expect_err("没有历史");
                    assert_eq!(fault.domain_code(), Some(ErrorCode::IndexOutOfBounds));
                }
                _ => {
                    outcome.unwrap_or_else(|fault| panic!("{name} 规划失败: {fault:?}"));
                }
            }
        }
    }

    fn clip_id(domain: &Domain) -> EntityId {
        domain
            .active_project()
            .expect("工程")
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_some())
            .expect("MIDI 片段")
            .id
    }

    fn note_id(domain: &Domain) -> EntityId {
        domain
            .active_project()
            .expect("工程")
            .clip_pool
            .get(&clip_id(domain))
            .and_then(|entry| entry.content.notes())
            .expect("音符集合")
            .keys()
            .next()
            .copied()
            .expect("至少一个音符")
    }

    #[test]
    fn plan_never_mutates_state_and_preview_says_so() {
        let domain = domain();
        let before_digest = domain.project_digest();
        let before_commits = domain.commit_count();
        let call = call(
            "yeban_set_macro",
            serde_json::json!({
                "trackId": fixture_track(&domain).to_canonical_string(),
                "macroIndex": 0,
                "value": 0.9,
            }),
        );
        let preview = preview(&domain, &call).expect("预览");
        assert_eq!(domain.project_digest(), before_digest, "工程摘要不得改变");
        assert_eq!(domain.commit_count(), before_commits, "提交数不得改变");
        assert_eq!(preview["plan"], "propose");
        assert_eq!(preview["commitCountBefore"], 1);
        assert_eq!(preview["commitCountAfter"], 2);
        assert_eq!(preview["wouldApply"], true);
        assert_eq!(preview["projectChanges"], false, "提案不改工程内容");
    }

    #[test]
    fn save_without_changes_is_skipped_unless_forced() {
        let mut domain = domain();
        let no_force = execute(
            &mut domain,
            &call("yeban_save_project", serde_json::json!({})),
        )
        .expect("执行");
        assert_eq!(no_force["status"], "success");

        // `force` 走真落盘路径 → 路径不存在 ⇒ IO_ERROR（领域失败, 带内）。
        let forced = execute(
            &mut domain,
            &call("yeban_save_project", serde_json::json!({"force": true})),
        )
        .expect("执行");
        assert_eq!(forced["status"], "error");
        assert_eq!(forced["error"]["code"], "IO_ERROR");
    }

    #[test]
    fn render_master_validates_first_then_really_writes_a_master() {
        let mut domain = domain();
        // 坏参数: 领域错误码 (带内 ToolResponse) —— 校验仍然先于渲染。
        let bad = execute(
            &mut domain,
            &call(
                "yeban_render_master",
                serde_json::json!({"format": "mp3", "sampleRate": 48000}),
            ),
        )
        .expect("带内");
        assert_eq!(bad["status"], "error");
        assert_eq!(bad["error"]["code"], "INVALID_PARAMETER_RANGE");

        // 好参数: 真的渲染 + 真的落盘。
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-render-unit-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let out = dir.join("master.wav");
        let good = execute(
            &mut domain,
            &call(
                "yeban_render_master",
                serde_json::json!({
                    "format": "wav",
                    "sampleRate": 48000,
                    "path": out.display().to_string(),
                }),
            ),
        )
        .expect("带内");
        assert_eq!(good["status"], "success", "{good}");
        assert_eq!(good["data"]["rendered"], true);
        assert_eq!(good["data"]["path"], out.display().to_string());
        let written = std::fs::read(&out).expect("产物必须真的存在");
        assert_eq!(
            written.len(),
            usize::try_from(good["data"]["bytes"].as_u64().expect("bytes")).expect("小尺寸")
        );
        // 128 BPM / 960 PPQ / 3840 tick ⇒ 1.875 s ⇒ 90 000 帧 @ 48 kHz。
        assert_eq!(good["data"]["frames"], 90_000);
        assert_eq!(good["data"]["channels"], 2);
        assert_eq!(good["data"]["bitDepth"], 24);
        assert_eq!(
            good["data"]["sha256"].as_str().expect("sha256").len(),
            64,
            "整份文件的 SHA-256"
        );
        assert!(
            good["data"]["unsupported"]
                .as_array()
                .expect("unsupported 必须是数组")
                .iter()
                .any(|key| key == "audioClips"),
            "规范样本工程含音频片段, 必须如实登记未渲染: {good}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn render_master_without_a_project_is_no_active_project() {
        let mut domain = Domain::new();
        let value = execute(
            &mut domain,
            &call(
                "yeban_render_master",
                serde_json::json!({"format": "wav", "sampleRate": 48000}),
            ),
        )
        .expect("带内");
        assert_eq!(value["error"]["code"], "NO_ACTIVE_PROJECT");
    }

    #[test]
    fn proposal_lifecycle_is_traceable_and_idempotent() {
        let mut domain = domain();
        let track = fixture_track(&domain);
        let created = execute(
            &mut domain,
            &call(
                "yeban_set_macro",
                serde_json::json!({
                    "trackId": track.to_canonical_string(),
                    "macroIndex": 0,
                    "value": 0.25,
                }),
            ),
        )
        .expect("创建提案");
        assert_eq!(created["status"], "success");
        assert_eq!(created["data"]["projectUnchanged"], true);
        let proposal_id = created["data"]["proposal"]["proposalId"]
            .as_str()
            .expect("proposalId")
            .to_owned();
        assert_eq!(domain.proposal_count(), 1);

        // 合并。
        let merged = execute(
            &mut domain,
            &call(
                "yeban_merge_proposal",
                serde_json::json!({"proposalId": proposal_id, "commitMessage": "接入宏"}),
            ),
        )
        .expect("合并");
        assert_eq!(merged["data"]["merged"], true);
        assert_eq!(merged["data"]["alreadyMerged"], false);
        assert_eq!(domain.commit_count(), 3, "根提交 + 提案提交 + 合并提交");

        // 幂等: 再合并一次不重复施加。
        let again = execute(
            &mut domain,
            &call(
                "yeban_merge_proposal",
                serde_json::json!({"proposalId": proposal_id, "commitMessage": "接入宏"}),
            ),
        )
        .expect("再合并");
        assert_eq!(again["data"]["alreadyMerged"], true);
        assert_eq!(again["data"]["appliedOps"], 0);
        assert_eq!(domain.commit_count(), 3, "不得新增提交");

        // 已合并的提案不能再拒绝。
        let rejected = execute(
            &mut domain,
            &call(
                "yeban_reject_proposal",
                serde_json::json!({"proposalId": proposal_id, "reason": "反悔"}),
            ),
        )
        .expect("带内");
        assert_eq!(rejected["error"]["code"], "CONFLICT");
    }

    /// 判据：合并提案写出的是一条**多父合并提交**，且"提案基于哪个提交"是图谱里的一条父边。
    ///
    /// 关闭 `docs/ledger/tools-domain-notes.md` 的 **needs-5** 第二半（模型层已提供
    /// `CommitGraph::append_merge` 与 `CommitGraph::create_branch`）。
    ///
    /// 三个量各自钉住一件事：
    ///
    /// 1. 合并提交的父集合**逐位**是 `[合并前的活跃头, 提案头]` ⇒ "并了谁"写进图谱；
    /// 2. `CommitGraph::ancestry` 只走第一父 ⇒ 提案的提交**不**进主干祖先链，
    ///    因此撤销仍然是"一次回退整套 AI 变更"；
    /// 3. 提案分支头的第一父 = 提案的 `baseCommit` ⇒ 分叉点也在图谱里。
    #[test]
    fn a_merged_proposal_is_a_two_parent_commit_and_the_base_is_a_real_parent() {
        let mut domain = domain();
        let track = fixture_track(&domain);
        let base_commit = domain.active_head().expect("基线头");

        let created = execute(
            &mut domain,
            &call(
                "yeban_set_macro",
                serde_json::json!({
                    "trackId": track.to_canonical_string(),
                    "macroIndex": 0,
                    "value": 0.25,
                }),
            ),
        )
        .expect("创建提案");
        let proposal_id = created["data"]["proposal"]["proposalId"]
            .as_str()
            .expect("proposalId")
            .to_owned();
        let proposal_head = created["data"]["proposal"]["headCommit"]
            .as_str()
            .expect("headCommit")
            .to_owned();

        // ① 提案分支头是**基线提交的子**，不再是孤立根提交。
        let proposal_head_id = EntityId::from_str(&proposal_head).expect("提案头是 ULID");
        let proposal_commit = domain
            .graph()
            .commit(&proposal_head_id)
            .expect("提案提交在图谱里");
        assert_eq!(
            proposal_commit.parents,
            vec![base_commit],
            "提案提交的父必须恰是它的 baseCommit"
        );

        // ② 创建提案**不动**主分支头，也不动工程字节。
        assert_eq!(domain.active_head(), Some(base_commit), "主分支头不得前移");

        let merged = execute(
            &mut domain,
            &call(
                "yeban_merge_proposal",
                serde_json::json!({"proposalId": proposal_id, "commitMessage": "接入宏"}),
            ),
        )
        .expect("合并");
        assert_eq!(merged["data"]["merged"], true);
        assert_eq!(domain.commit_count(), 3, "根提交 + 提案提交 + 合并提交");

        // ③ 合并提交：响应与图谱**逐位一致**，父集合是 `[基线, 提案头]`。
        let merge_commit = EntityId::from_str(
            merged["data"]["commit"]["id"]
                .as_str()
                .expect("合并提交身份"),
        )
        .expect("合并身份是 ULID");
        let expected_parents = vec![base_commit, proposal_head_id];
        assert_eq!(
            domain
                .graph()
                .commit(&merge_commit)
                .expect("合并提交")
                .parents,
            expected_parents,
            "第一父恒为合并前的活跃头, 第二父是提案头"
        );
        assert_eq!(
            merged["data"]["commit"]["parents"],
            serde_json::json!([base_commit.to_canonical_string(), proposal_head]),
            "响应里的父集合必须与图谱逐位相同"
        );
        assert_eq!(merged["data"]["commit"]["isMerge"], true);
        assert_eq!(merged["data"]["commit"]["parentCount"], 2);
        assert_eq!(merged["data"]["commit"]["depth"], 3, "根=1 ⇒ 合并提交=3");
        assert_eq!(domain.graph().validate(), Ok(()), "多父合并后图谱必须自洽");

        // ④ 主干祖先链只走第一父：提案提交**不**在 `main` 的祖先链上。
        let ancestry = domain.graph().ancestry(&merge_commit).expect("祖先链");
        assert_eq!(ancestry, vec![merge_commit, base_commit], "主干只有两跳");
        assert!(!ancestry.contains(&proposal_head_id), "提案提交不得进主干");

        // ⑤ 合并仍然算**一步**撤销。
        let undone =
            execute(&mut domain, &call("yeban_undo", serde_json::json!({}))).expect("撤销合并");
        assert_eq!(undone["data"]["steps"], 1);
    }

    /// 判据：撤销之后继续合并落在**匿名分支**上时，响应必须**如实**报"这不是一次合并"。
    ///
    /// 这一条钉住的是"父集合从图谱读回、而不是把请求当成事实"这个决定。
    /// `undo_session::commit_merge` 在"撤销位置"上只能走 `CommitGraph::fork_anonymous`
    /// （它没有额外父参数）⇒ 写出的提交是**单父**的。若这里改成把请求的父集合
    /// （`[活跃头, 提案头]`）当成事实写进响应，本条判据变红。
    #[test]
    fn merging_after_an_undo_reports_the_single_parent_it_actually_wrote() {
        let mut domain = domain();
        let track = fixture_track(&domain);
        let root_commit = domain.active_head().expect("根提交");

        // 第一条提案：合并 => main 头前移一格。
        let first = execute(
            &mut domain,
            &call(
                "yeban_set_macro",
                serde_json::json!({
                    "trackId": track.to_canonical_string(),
                    "macroIndex": 0,
                    "value": 0.25,
                }),
            ),
        )
        .expect("第一条提案");
        execute(
            &mut domain,
            &call(
                "yeban_merge_proposal",
                serde_json::json!({
                    "proposalId": first["data"]["proposal"]["proposalId"],
                    "commitMessage": "第一次",
                }),
            ),
        )
        .expect("第一次合并");

        // 撤销一步 ⇒ 下一步编辑（这里是一次合并）必须落在撤销位置上的匿名分支。
        let undone =
            execute(&mut domain, &call("yeban_undo", serde_json::json!({}))).expect("撤销");
        assert_eq!(undone["data"]["steps"], 1);

        let second = execute(
            &mut domain,
            &call(
                "yeban_set_macro",
                serde_json::json!({
                    "trackId": track.to_canonical_string(),
                    "macroIndex": 0,
                    "value": 0.5,
                }),
            ),
        )
        .expect("第二条提案");
        let merged = execute(
            &mut domain,
            &call(
                "yeban_merge_proposal",
                serde_json::json!({
                    "proposalId": second["data"]["proposal"]["proposalId"],
                    "commitMessage": "撤销之后",
                }),
            ),
        )
        .expect("撤销之后合并");

        assert_eq!(merged["data"]["merged"], true);
        // `baseCommitMoved` 量的是**活跃分支头**：撤销不动分支头，因此这里是 `false`。
        // 撤销留下的痕迹体现在别处 —— 提交落在一条匿名分支上（见下）。
        assert_eq!(merged["data"]["baseCommitMoved"], false);
        assert!(
            merged["data"]["commit"]["branch"]
                .as_str()
                .expect("分支名")
                .starts_with("anon-"),
            "撤销位置上的提交必须落在匿名分支上: {merged}"
        );
        assert_eq!(
            merged["data"]["commit"]["parents"],
            serde_json::json!([root_commit.to_canonical_string()]),
            "写出来的就是单父提交 ⇒ 响应不得声称它有两个父"
        );
        assert_eq!(merged["data"]["commit"]["isMerge"], false);
        assert_eq!(merged["data"]["commit"]["parentCount"], 1);
    }

    #[test]
    fn rejecting_a_proposal_keeps_the_record() {
        let mut domain = domain();
        let track = fixture_track(&domain);
        let created = execute(
            &mut domain,
            &call(
                "yeban_set_macro",
                serde_json::json!({
                    "trackId": track.to_canonical_string(),
                    "macroIndex": 0,
                    "value": 0.6,
                }),
            ),
        )
        .expect("创建");
        let proposal_id = created["data"]["proposal"]["proposalId"]
            .as_str()
            .expect("proposalId")
            .to_owned();
        let digest_before = domain.project_digest();
        let rejected = execute(
            &mut domain,
            &call(
                "yeban_reject_proposal",
                serde_json::json!({"proposalId": proposal_id, "reason": "织体过密"}),
            ),
        )
        .expect("拒绝");
        assert_eq!(rejected["data"]["rejected"], true);
        assert_eq!(rejected["data"]["archived"], true);
        assert_eq!(rejected["data"]["proposal"]["status"], "rejected");
        assert_eq!(
            rejected["data"]["proposal"]["resolution"], "织体过密",
            "拒绝原因必须留痕"
        );
        assert_eq!(domain.project_digest(), digest_before, "拒绝不得改工程");
        // 记录仍在。
        assert_eq!(domain.proposal_count(), 1);
        // 再拒一次是幂等的。
        let again = execute(
            &mut domain,
            &call(
                "yeban_reject_proposal",
                serde_json::json!({"proposalId": proposal_id, "reason": "织体过密"}),
            ),
        )
        .expect("再拒绝");
        assert_eq!(again["data"]["alreadyRejected"], true);
    }

    #[test]
    fn edit_notes_produces_a_reversible_proposal() {
        let mut domain = domain();
        let project = domain.active_project().cloned().expect("工程");
        let track = fixture_track(&domain);
        let clip = clip_id(&domain);
        let note = note_id(&domain);
        let created = execute(
            &mut domain,
            &call(
                "yeban_edit_notes",
                serde_json::json!({
                    "trackId": track.to_canonical_string(),
                    "clipId": clip.to_canonical_string(),
                    "ops": [{"kind": "velocity", "noteId": note.to_canonical_string(), "velocity": 42}],
                }),
            ),
        )
        .expect("提案");
        assert_eq!(created["status"], "success");
        let proposal_id = created["data"]["proposal"]["proposalId"]
            .as_str()
            .expect("id")
            .to_owned();
        let id = EntityId::from_str(&proposal_id).expect("ULID");
        let proposal = domain.proposal(&id).expect("记录").clone();
        assert_eq!(proposal.kind, "notes");

        // 合并 → 音符力度真的变了 → 逆操作能回到原样。
        execute(
            &mut domain,
            &call(
                "yeban_merge_proposal",
                serde_json::json!({"proposalId": proposal_id, "commitMessage": "力度"}),
            ),
        )
        .expect("合并");
        let after = domain.active_project().cloned().expect("工程");
        assert_ne!(after, project, "合并必须真的改了工程");
        let mut undone = after;
        for stamped in proposal.ops.iter().rev() {
            stamped.apply_inverse(&mut undone).expect("逆操作");
        }
        assert_eq!(undone, project, "model 的 invert 必须能回到合并前的字节");
    }

    #[test]
    fn query_and_close_behave_on_the_session() {
        let mut domain = domain();
        let queried = execute(
            &mut domain,
            &call(
                "yeban_query_project",
                serde_json::json!({"limit": 2, "fields": ["bpm", "tracks.name"]}),
            ),
        )
        .expect("查询");
        assert_eq!(queried["data"]["page"]["returned"], 2);
        assert!(queried["data"]["project"].get("title").is_none());

        let fault = execute(
            &mut domain,
            &call(
                "yeban_query_project",
                serde_json::json!({"fields": ["nope"]}),
            ),
        )
        .expect("带内");
        assert_eq!(fault["error"]["code"], "INVALID_FIELD_SELECTOR");

        let closed = execute(
            &mut domain,
            &call(
                "yeban_close_project",
                serde_json::json!({"saveFirst": false}),
            ),
        )
        .expect("关闭");
        assert_eq!(closed["data"]["closed"], true);
        assert_eq!(closed["data"]["saved"], false);

        // 关闭之后所有需要工程的工具都是 NO_ACTIVE_PROJECT。
        let after = execute(
            &mut domain,
            &call("yeban_query_project", serde_json::json!({})),
        )
        .expect("带内");
        assert_eq!(after["error"]["code"], "NO_ACTIVE_PROJECT");
        let close_again = execute(
            &mut domain,
            &call("yeban_close_project", serde_json::json!({})),
        )
        .expect("带内");
        assert_eq!(close_again["error"]["code"], "NO_ACTIVE_PROJECT");
    }

    #[test]
    fn dry_run_preview_ops_equal_the_ops_actually_committed() {
        let mut domain = domain();
        let track = fixture_track(&domain);
        // `includeOps: true`：这条判据比的是**完整 op 载荷**，所以两侧都显式索取它
        // （缺省形状只回结构化字段, 见 `crate::tools::INCLUDE_OPS_PARAM`）。
        let arguments = serde_json::json!({
            "trackId": track.to_canonical_string(),
            "macroIndex": 0,
            "value": 0.3,
            "includeOps": true,
        });
        let call = call("yeban_set_macro", arguments);
        let preview = preview(&domain, &call).expect("预览");
        let planned_ops = preview["ops"].clone();

        let created = execute(&mut domain, &call).expect("执行");
        let proposal_id = created["data"]["proposal"]["proposalId"]
            .as_str()
            .expect("id");
        let record = domain
            .proposal(&EntityId::from_str(proposal_id).expect("ULID"))
            .expect("记录");
        let committed: Vec<Value> = record
            .ops
            .iter()
            .map(|stamped| serde_json::to_value(&stamped.op).expect("序列化"))
            .collect();
        assert_eq!(
            planned_ops,
            Value::Array(committed),
            "预览里的 op 必须与真实提交的 op 逐字节相同 (确定性身份)"
        );
    }

    /// **施加修订号的推进口径**（`ROAD-M4-008` 选项 (a) 的触发口径）。
    ///
    /// 这条判据要证的句子：**只有真的可能改工程的计划才推进修订号**。
    /// 两侧都用**真工具调用**（不是手搓 `Plan`）：只读一侧是 `yeban_query_project`
    /// 与 `yeban_export_midi`；写一侧是 `yeban_edit_automation`（在一个从没有泳道的
    /// `TrackPan` 目标上写一个点）。
    ///
    /// 两处交叉核对都做，因为两种错法都会让界面出错，而且方向相反：
    /// - "修订号动了但工程没动" ⇒ 界面被**白刷**（因此只读一侧断言字节逐字不变）；
    /// - "工程动了而修订号没动" ⇒ 界面**不跟**（因此写一侧断言字节真的变了）。
    #[test]
    fn only_plans_that_can_change_the_project_advance_the_apply_revision() {
        let project_json = |domain: &Domain| -> String {
            crate::domain::store::serialize_project(domain.active_project().expect("活跃工程"))
                .expect("序列化")
        };
        let mut domain = domain();
        let revision = domain.apply_revision();
        let bytes_before = project_json(&domain);

        // ---- 只读一侧：修订号与工程字节都必须一位不动 ----
        for (name, arguments) in [
            ("yeban_query_project", serde_json::json!({"limit": 3})),
            ("yeban_export_midi", serde_json::json!({})),
        ] {
            let tool_call = call(name, arguments);
            let planned = plan(&domain, &tool_call).expect("只读工具必须规划成功");
            assert!(
                !planned.mutates_project(),
                "`{name}` 不得被算作「可能改工程」"
            );
            execute(&mut domain, &tool_call).expect("只读工具必须执行成功");
            assert_eq!(
                domain.apply_revision(),
                revision,
                "`{name}` 之后修订号不得动"
            );
            assert_eq!(
                project_json(&domain),
                bytes_before,
                "`{name}` 之后工程字节必须逐字不变"
            );
        }

        // ---- 写一侧：恰好推进一次，且工程字节真的变了 ----
        let track = fixture_track(&domain);
        let write = call(
            "yeban_edit_automation",
            serde_json::json!({
                "trackId": track.to_canonical_string(),
                "lane": "TrackPan",
                "point": {"tick": 0, "value": 0.25, "curve": "Linear"},
            }),
        );
        assert!(
            plan(&domain, &write)
                .expect("写类工具必须规划成功")
                .mutates_project(),
            "在从没有泳道的目标上写一个点必须被算作「可能改工程」"
        );
        execute(&mut domain, &write).expect("施加");
        assert_eq!(
            domain.apply_revision(),
            revision + 1,
            "改工程的施加必须恰好推进一个修订号"
        );
        assert_ne!(
            project_json(&domain),
            bytes_before,
            "修订号动了 ⇒ 工程字节必须真的变了"
        );
    }
}
