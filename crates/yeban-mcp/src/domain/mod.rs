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
//! | [`section`] | `yeban_propose_section` 的章节骨架 + 环路判定 |
//! | [`macros`] | `yeban_set_macro` 的宏与级联自动化展开 |
//! | [`proposal`] | 提案记录（Musical PR 的可追溯性） |
//! | [`render`] | `yeban_render_master` 的参数校验（渲染本体未接线） |
//! | [`ids`] | 确定性夹具身份（让 `dryRun` 预览与真调用逐字节相同） |

pub mod error;
pub mod ids;
pub mod lock;
pub mod macros;
pub mod notes;
pub mod proposal;
pub mod render;
pub mod section;
pub mod store;
pub mod view;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use yeban_model::{
    AssetHash, CommitDraft, CommitGraph, EntityId, Op, OpOrigin, StampedOp, YebanProjectV1,
};

use crate::jsonrpc::ErrorObject;
use crate::tools::{ErrorCode, ToolCall, ToolResponse};

use error::{Fault, not_wired};
use proposal::{Proposal, ProposalDraft, ProposalStatus, draft_ops_value};
use store::AcquiredLock;

/// 主分支名。
pub const MAIN_BRANCH: &str = "main";

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
    /// 打开容器时由 `assets/{sha256}` 填充；打开裸 JSON 兼容路径时为空
    /// （裸 JSON 里**没有**资产字节，只有 `project.assets` 这一份元数据索引）。
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

/// 领域会话状态：活跃工程 + 提交图谱 + 提案记录 + 注入的时钟。
///
/// **刻意不实现 `Clone`**：它内涵 `.yeban.lock` 的 RAII 守卫与提交图谱，
/// 克隆会产出两个独立的写者。
#[derive(Debug)]
pub struct Domain {
    active: Option<Active>,
    graph: CommitGraph,
    proposals: BTreeMap<EntityId, Proposal>,
    now_ms: u64,
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
            now_ms: 0,
        }
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
    fn reset_history(&mut self, seed: SessionSeed) -> Result<(), Fault> {
        self.graph = CommitGraph::new();
        self.proposals.clear();
        if let Some(history) = seed.history {
            self.graph = history;
        } else {
            let draft = CommitDraft::new(
                EntityId::new(),
                MAIN_BRANCH,
                AGENT_NAME,
                format!("open {}", seed.path.display()),
            )
            .with_created_at(self.now_ms);
            self.graph
                .genesis(draft)
                .map_err(|failure| error::from_model("提交图谱根", &failure))?;
        }
        self.active = Some(Active {
            path: seed.path,
            read_only: seed.read_only,
            project: seed.project,
            saved_digest: seed.digest,
            assets: seed.assets,
            lock: seed.lock,
        });
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
/// "工程 + 形态 + 历史 + 资产池"四类，平铺进枚举变体会让每一处 `match` 都变成
/// 一长串 `..`。结构体也让"打开时必须一起决定的事"在类型上绑在一起。
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
    /// 磁盘上的文件字节数（容器形态下是容器字节数）。
    pub bytes: u64,
    /// 是否已经打开了同一个工程（幂等）。
    pub already_open: bool,
    /// 磁盘形态（容器 / 裸 JSON 兼容路径）。
    pub format: store::ProjectFormat,
    /// `history.dag` 恢复出的提交图谱（裸 JSON / 空图谱为 `None`）。
    pub history: Option<Box<CommitGraph>>,
    /// `assets/{sha256}` 解出的会话 CAS 资产池。
    pub assets: BTreeMap<AssetHash, Vec<u8>>,
}

/// 一次已经校验过的执行计划（**只读计算的产物**）。
#[derive(Debug)]
pub enum Plan {
    /// `yeban_open_project`
    Open(Box<OpenRequest>),
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
    /// `yeban_render_master`：参数已校验，但渲染器未接线
    RenderMaster {
        /// 已校验的请求。
        request: render::RenderRequest,
    },
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
            Self::Propose { .. } => "propose",
            Self::Merge { .. } => "merge",
            Self::Reject { .. } => "reject",
            Self::RenderMaster { .. } => "render",
        }
    }

    /// 该计划会向提交图谱添加的提交数（`dryRun` 的"提交数不变"判据用它预测）。
    ///
    /// `Open` 的增量是**变量**（恢复的 `history.dag` 有多少条提交就装多少条），
    /// 因此它的预测值由 [`Plan::planned_commit_count`] 单独给出，不在这里。
    #[must_use]
    pub const fn commit_delta(&self) -> usize {
        match self {
            Self::Propose { .. } | Self::Merge { .. } => 1,
            Self::Open(..)
            | Self::Save { .. }
            | Self::Close { .. }
            | Self::Query { .. }
            | Self::Reject { .. }
            | Self::RenderMaster { .. } => 0,
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

    /// 施加后工程的形态（`None` 表示"工程内容不变"）。
    ///
    /// 这是 `dryRun` 的**差异预览**：把"将要发生什么"算出来给调用方看，
    /// 但**不改**真实状态（`Merge` 在克隆体上模拟）。
    ///
    /// # Errors
    ///
    /// `Merge` 的 op 无法整体施加（即真的会冲突）→ `CONFLICT`。
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
            Self::Save { .. }
            | Self::Close { .. }
            | Self::Query { .. }
            | Self::Propose { .. }
            | Self::Reject { .. }
            | Self::RenderMaster { .. } => Ok(None),
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
                preview.insert("format".to_owned(), Value::from(request.format.as_str()));
                preview.insert(
                    "container".to_owned(),
                    Value::from(request.format.is_container()),
                );
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
                preview.insert(
                    "format".to_owned(),
                    Value::from(store::ProjectFormat::Container.as_str()),
                );
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
            Self::Propose { draft } => {
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
                preview.insert("ops".to_owned(), draft_ops_value(draft));
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
            Self::RenderMaster { request } => {
                preview.insert("request".to_owned(), request.preview());
            }
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
        preview.insert("wouldApply".to_owned(), Value::from(true));
        Ok(Value::Object(preview))
    }
}

/// `NO_ACTIVE_PROJECT` 的简写。
fn no_active_project() -> Fault {
    Fault::domain(ErrorCode::NoActiveProject, "当前没有活跃工程")
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

/// 只读规划：参数校验 + 领域合法性校验。**改不了任何状态**（拿到的是 `&Domain`）。
///
/// # Errors
///
/// 见各工具的领域语义；实现级状况（如渲染器未接线）不在这里返回
/// （它在 `apply` 之后才知道，"参数校验先于未接线"是判据）。
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
        // `ToolCall::from_params` 已按契约枚举把关, 因此这里不可达;
        // 用 CONFLICT 而不是 panic: 未知工具名不该让服务进程倒下。
        other => Err(Fault::domain(
            ErrorCode::Conflict,
            format!("未接线到领域实现的工具 `{other}`"),
        )),
    }
}

/// `yeban_open_project`。
///
/// 读盘一律走 [`store::load_project`]（容器优先，裸 JSON 兼容路径），
/// 工程**形态 / 历史 / CAS 资产池**一并进入 [`OpenRequest`]。
fn plan_open(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let path =
        PathBuf::from(arg_str(call, "path").ok_or_else(|| {
            Fault::domain(ErrorCode::InvalidParameterRange, "`path` 必须是字符串")
        })?);
    let read_only = arg_bool(call, "readOnly", false);
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
        format: loaded.format,
        history: loaded.graph.map(Box::new),
        assets: loaded.assets,
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
    let json = store::serialize_project(&active.project)?;
    let digest = store::digest_of(json.as_bytes());
    // 落盘字节 = `ARCH-SEC-003` 的容器（`project.json` + `history.dag` + `assets/{sha256}`）。
    // 与"工程内容摘要"是**两个量**：前者含提交图谱与资产，后者只描述工程文档。
    let bytes =
        store::container_bytes(&active.path, &active.project, &domain.graph, &active.assets)?;
    Ok(Plan::Save {
        path: active.path.clone(),
        changed: digest != active.saved_digest,
        assets: active.assets.len(),
        history_commits: domain.graph.commit_count(),
        bytes,
        digest,
        force: arg_bool(call, "force", false),
    })
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
fn propose_draft(
    domain: &Domain,
    project: &YebanProjectV1,
    kind: &'static str,
    title: String,
    description: String,
    ops: Vec<Op>,
) -> Result<Plan, Fault> {
    let base_commit = domain.main_head().ok_or_else(|| {
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
    )
}

/// `yeban_edit_notes`。
fn plan_edit_notes(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let project = require_active(domain)?;
    let track_id = arg_id(call, "trackId")?;
    let clip_id = arg_id(call, "clipId")?;
    let raw_ops = call
        .arguments
        .get("ops")
        .ok_or_else(|| Fault::domain(ErrorCode::InvalidParameterRange, "缺少 `ops`"))?;
    let note_ops = notes::parse_ops(raw_ops)?;
    let compiled = notes::compile(project, &track_id, &clip_id, &note_ops)?;
    notes::check_polyphony(project, &clip_id, &compiled)?;
    propose_draft(
        domain,
        project,
        "notes",
        format!("音符编辑: {} 步", note_ops.len()),
        format!("edit_notes {clip_id}"),
        compiled,
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
    )
}

/// `yeban_render_master`。
fn plan_render_master(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    require_active(domain)?;
    let request = render::validate(&call.arguments)?;
    Ok(Plan::RenderMaster { request })
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
// apply：可变
// ---------------------------------------------------------------------------

/// 施加一个计划。**这是唯一会改变状态的入口。**
///
/// # Errors
///
/// 领域失败 → [`Fault::Domain`]（走 `ToolResponse`）；
/// 实现级状况 → [`Fault::Impl`]（走 JSON-RPC 错误对象）。
pub fn apply(domain: &mut Domain, plan: Plan, call: &ToolCall) -> Result<ToolResponse, Fault> {
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
        Plan::Propose { draft } => apply_propose(domain, *draft),
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
        Plan::RenderMaster { request } => Err(render_unwired(call, request)),
    }
}

/// `yeban_open_project` 的施加。
///
/// 容器形态会**恢复** `history.dag`（提交图谱）与 `assets/{sha256}`（CAS 池）；
/// 裸 JSON 兼容路径两者都为空（那份文件里没有它们）。
fn apply_open(domain: &mut Domain, request: OpenRequest) -> Result<ToolResponse, Fault> {
    let OpenRequest {
        path,
        read_only,
        project,
        digest,
        bytes,
        already_open,
        format,
        history,
        assets,
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
            "format": format.as_str(),
            "historyRestored": false,
            "assets": asset_count,
            "projectDigest": digest,
            "project": summary,
        })));
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
    Ok(ToolResponse::success(serde_json::json!({
        "opened": true,
        "alreadyOpen": false,
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
        // 磁盘形态 + 容器里另外两类条目的实际装载量（如实上报，不假装）。
        "format": format.as_str(),
        "historyRestored": history_commits > 0,
        "historyCommits": domain.commit_count(),
        "assets": domain.asset_count(),
        "projectDigest": digest,
        "project": domain
            .active_project()
            .map_or(Value::Null, project_summary),
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
            "format": store::ProjectFormat::Container.as_str(),
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
        "format": store::ProjectFormat::Container.as_str(),
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
fn apply_propose(domain: &mut Domain, draft: ProposalDraft) -> Result<ToolResponse, Fault> {
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
    let head_commit = domain
        .graph
        .genesis(
            CommitDraft::new(
                EntityId::new(),
                branch.clone(),
                AGENT_NAME,
                draft.title.clone(),
            )
            .with_created_at(now_ms)
            .with_ops(stamped.clone()),
        )
        .map_err(|failure| error::from_model("提案分支", &failure))?;
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
    let detail = record.detail();
    domain.proposals.insert(proposal_id, record);

    // 提案**不**改工程内容 —— 这两行是给调用方的证据, 不是顺手加的字段。
    let project_digest = store::digest_of(store::serialize_project(&project)?.as_bytes());
    let unwired = draft_unwired(&detail);
    Ok(ToolResponse::success(serde_json::json!({
        "created": true,
        "projectUnchanged": true,
        "projectDigest": project_digest,
        "unwired": unwired,
        "proposal": detail,
    })))
}

/// 提案类工具里**明确没接线**的那部分（如实上报，不藏在错误码后面）。
fn draft_unwired(detail: &Value) -> Vec<&'static str> {
    match detail.get("kind").and_then(Value::as_str) {
        Some("section") => vec!["clipPoolEntries", "routingEdges"],
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
    let base_moved = domain.main_head() != Some(snapshot.base_commit);
    let batch = merge_batch(snapshot, commit_message);
    let mut merged = current;
    batch.apply(&mut merged).map_err(|failure| {
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
    merged
        .validate()
        .map_err(|failure| error::from_model("合并结果校验", &failure))?;

    let merge_commit = domain
        .graph
        .append(
            CommitDraft::new(
                EntityId::new(),
                MAIN_BRANCH,
                AGENT_NAME,
                commit_message.to_owned(),
            )
            .with_created_at(now_ms)
            .with_ops(vec![StampedOp::new(
                OpOrigin::McpProposal {
                    proposal_id,
                    agent_name: AGENT_NAME.to_owned(),
                },
                now_ms,
                batch,
            )]),
        )
        .map_err(|failure| error::from_model("合并提交", &failure))?;

    let new_digest = store::digest_of(store::serialize_project(&merged)?.as_bytes());
    if let Some(active) = domain.active.as_mut() {
        active.project = merged;
    }
    if let Some(record) = domain.proposals.get_mut(&proposal_id) {
        record.status = ProposalStatus::Merged;
        record.merge_commit = Some(merge_commit);
        record.resolved_at = Some(now_ms);
        record.resolution = Some(commit_message.to_owned());
    }
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
            "branch": MAIN_BRANCH,
            "message": commit_message,
            "atomicBatch": true,
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

/// 渲染器的实现级状况：**参数已经校验通过**，只是这一半没接线。
fn render_unwired(call: &ToolCall, request: render::RenderRequest) -> Fault {
    let mut data = Map::new();
    data.insert("validated".to_owned(), Value::from(true));
    data.insert("request".to_owned(), request.preview());
    data.insert(
        "reason".to_owned(),
        Value::from("离线渲染属于 line/render-master; 本线只接线参数校验与 dryRun"),
    );
    not_wired(
        call.tool.name,
        call.tool.spec_id,
        "渲染器未接线: 参数校验已通过",
        data,
    )
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
    match apply(domain, planned, call) {
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
                serde_json::json!({"sectionName": "Chorus", "stylePreset": "lofi-beats", "bars": 8}),
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
    fn render_master_validates_before_it_reports_not_wired() {
        let mut domain = domain();
        // 坏参数: 领域错误码 (带内 ToolResponse)。
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

        // 好参数: 通过校验 ⇒ 实现级 -32005。
        let good = execute(
            &mut domain,
            &call(
                "yeban_render_master",
                serde_json::json!({"format": "wav", "sampleRate": 48000}),
            ),
        )
        .expect_err("渲染未接线");
        assert_eq!(good.code, crate::jsonrpc::NOT_IMPLEMENTED);
        assert_eq!(good.data.as_ref().expect("data")["validated"], true);
        assert_eq!(good.data.as_ref().expect("data")["request"]["wired"], false);
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
        let arguments = serde_json::json!({
            "trackId": track.to_canonical_string(),
            "macroIndex": 0,
            "value": 0.3,
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
}
