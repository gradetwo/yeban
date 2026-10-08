//! 提交图谱 (Commit DAG) 与非线性历史分叉 [ARCH-OPS-002, ROAD-M1-003]。
//!
//! 本模块把"编曲时光机"落成可判定的数据结构：
//!
//! - [`Commit`] — 一次提交：父提交集合、分支、作者、消息、时间、`rng_seed`、
//!   该提交捕获的操作日志，以及可选的**全量快照**引用。
//! - [`CommitGraph`] — 提交与分支的唯一事实源，全部集合都是 `BTreeMap`
//!   [MODEL-AST-003]。
//! - [`CommitGraph::undo`] — **跨 Commit 边界**的连续撤销：ops 用尽就自动回溯父提交，
//!   直到撤销步数用满（或到达根提交）。
//! - [`CommitGraph::fork_anonymous`] — 撤销数步后继续编辑时自动派生**匿名分支**，
//!   被撤销的操作 100% 永久保全（原分支头不动，成为只读孤岛）。
//! - [`CommitGraph::create_branch`] — 在**指定提交**上创建一条**命名**分支（不写提交）：
//!   "Musical PR" 那类隔离分支不再只能建成孤立根提交。
//! - [`CommitGraph::append_merge`] — **多父**合并提交：第一父仍是当前分支头，因此
//!   [`CommitGraph::ancestry`] 与撤销链的主干方向不变，被并进来的一侧留在
//!   [`Commit::parents`] 里，DAG 关系由本图谱自己承担。
//!
//! ## 快照策略
//!
//! 每 [`SNAPSHOT_INTERVAL`]（256）次提交保存一次全量快照，其余提交只存紧凑操作日志。
//! 快照点定义为深度 `1, 257, 513, …`：根提交必须有快照，
//! 否则"撤销到根"没有可回放的全量基线。
//!
//! ## 为什么提交身份由调用方提供
//!
//! 所有写入 API 都通过 [`CommitDraft`] 显式接收 `id`。这样图谱本身是**纯的**
//! （同输入恒同输出），测试可以用确定性 ULID 断言 DAG 形状与分支名；
//! 真实调用方用 [`EntityId::new`] 生成身份即可。
//! `CommitDraft` 同时把 8 个字段收成一个参数，避免超长参数列表
//! （`clippy::too_many_arguments` 是 `clippy::all` 的一部分）。

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::error::ModelError;
use crate::ids::{ContentHash, EntityId};
use crate::ops::StampedOp;
use crate::project::YebanProjectV1;

/// 每多少次提交保存一次全量快照 [ARCH-OPS-002, ROAD-M1-003]。
pub const SNAPSHOT_INTERVAL: u64 = 256;

/// 匿名分支名的前缀（`anon-<提交 ULID>`）。
pub const ANONYMOUS_BRANCH_PREFIX: &str = "anon-";

/// 判定某个**深度**（根提交深度为 1）是否到达快照点 [ROAD-M1-003]。
///
/// 快照点为 `1, 257, 513, …`，即相邻快照间隔恒为 [`SNAPSHOT_INTERVAL`]。
#[must_use]
pub const fn snapshot_due_at_depth(depth: u64) -> bool {
    depth >= 1 && (depth - 1).is_multiple_of(SNAPSHOT_INTERVAL)
}

/// 一次版本提交 [ARCH-OPS-002]。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Commit {
    /// 提交身份。
    pub id: EntityId,
    /// 父提交（根提交为空；合并提交可以有多个）。
    pub parents: Vec<EntityId>,
    /// 所属分支名。
    pub branch_id: String,
    /// 作者。
    pub author: String,
    /// 提交消息。
    pub message: String,
    /// 创建时间 (Unix 毫秒)。
    pub created_at: u64,
    /// 本次提交使用的确定性随机种子 [MODEL-AST-005]。
    pub rng_seed: u64,
    /// 本次提交捕获的操作日志。
    pub ops: Vec<StampedOp>,
    /// 全量快照引用；`None` 表示"相对父提交重放 ops 即可复原"
    /// [ARCH-OPS-002 的每 256 次提交一个快照策略]。
    pub snapshot_ref: Option<ContentHash>,
}

impl Commit {
    /// 本提交（位于给定深度时）是否应当携带全量快照。
    #[must_use]
    pub const fn snapshot_due(&self, depth: u64) -> bool {
        snapshot_due_at_depth(depth)
    }

    /// 是否真的携带了全量快照。
    #[must_use]
    pub const fn has_snapshot(&self) -> bool {
        self.snapshot_ref.is_some()
    }

    /// 第一个父提交（线性历史的主干方向）。
    #[must_use]
    pub fn first_parent(&self) -> Option<EntityId> {
        self.parents.first().copied()
    }

    /// 是否为**合并提交**（父提交多于一个）。
    #[must_use]
    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }
}

/// 分支头。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BranchHead {
    /// 分支名。
    pub name: String,
    /// 当前头提交。
    pub head: EntityId,
    /// 是否为**匿名分支**（由"撤销后继续编辑"自动派生）[ARCH-OPS-002]。
    pub anonymous: bool,
}

/// 待写入的提交草稿。
///
/// 所有字段都可以用链式方法覆盖，构造最小草稿只需 [`CommitDraft::new`]。
#[derive(Clone, Debug, PartialEq)]
pub struct CommitDraft {
    /// 提交身份（由调用方提供，保证图谱是纯函数）。
    pub id: EntityId,
    /// 所属分支名。
    pub branch_id: String,
    /// 作者。
    pub author: String,
    /// 提交消息。
    pub message: String,
    /// 创建时间 (Unix 毫秒)。
    pub created_at: u64,
    /// 确定性随机种子。
    pub rng_seed: u64,
    /// 本次提交捕获的操作日志。
    pub ops: Vec<StampedOp>,
    /// 调用方提供的全量快照引用（仅在快照点被采用）。
    pub snapshot_ref: Option<ContentHash>,
}

impl CommitDraft {
    /// 用身份、分支、作者与消息构造草稿（时间与种子为 0，ops 为空，无快照引用）。
    #[must_use]
    pub fn new(
        id: EntityId,
        branch_id: impl Into<String>,
        author: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            id,
            branch_id: branch_id.into(),
            author: author.into(),
            message: message.into(),
            created_at: 0,
            rng_seed: 0,
            ops: Vec::new(),
            snapshot_ref: None,
        }
    }

    /// 设置创建时间。
    #[must_use]
    pub const fn with_created_at(mut self, created_at: u64) -> Self {
        self.created_at = created_at;
        self
    }

    /// 设置确定性随机种子。
    #[must_use]
    pub const fn with_rng_seed(mut self, rng_seed: u64) -> Self {
        self.rng_seed = rng_seed;
        self
    }

    /// 设置操作日志。
    #[must_use]
    pub fn with_ops(mut self, ops: Vec<StampedOp>) -> Self {
        self.ops = ops;
        self
    }

    /// 设置快照引用（仅在快照点会被采用）。
    #[must_use]
    pub fn with_snapshot(mut self, snapshot_ref: ContentHash) -> Self {
        self.snapshot_ref = Some(snapshot_ref);
        self
    }
}

/// 连续撤销的游标：记录"已经从当前头往回撤销了多少步"。
///
/// **属于会话运行态，不是持久化文档层** [MODEL-ISO-001]：因此本类型刻意
/// **不实现** `Serialize`/`Deserialize`，从类型上杜绝它被写进工程文件。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UndoCursor {
    /// 已撤销的步数（从分支头往前数）。
    pub skip: usize,
}

impl UndoCursor {
    /// 全新的游标（一步都还没撤销）。
    #[must_use]
    pub const fn new() -> Self {
        Self { skip: 0 }
    }

    /// 已撤销步数。
    #[must_use]
    pub const fn undone(&self) -> usize {
        self.skip
    }
}

/// [`CommitGraph`] 的**结构自洽**错误 [ARCH-OPS-002, ARCH-SEC-003]。
///
/// 与 [`HistoryDagError`] 的分工：那个说的是"这串字节根本不是一份图谱"，
/// 这里说的是"这份图谱自己不自洽"（引用了不存在的提交、深度缓存与父集合矛盾等）。
/// 图谱有两个来源：本 crate 的四个写入 API（恒自洽），以及磁盘 / 第三方归档里的
/// `history.dag`（**不可信**）。因此自洽性必须在**解读的边界**上检查一次，
/// 见 [`decode_history_dag`]。
///
/// ## 为什么必须有这一层
///
/// `commits`、`branches`、`depths` 是**分开保存**的三个集合：写入 API 负责让它们同步，
/// 反序列化不做这件事。一份"JSON 合法、但集合互相矛盾"的 `history.dag` 带来两类故障，
/// 两者都不是"数据难看"，而是**行为错**：
///
/// 1. **深度缓存与父集合矛盾** ⇒ 快照点（深度 `1, 257, 513, …`）从此算错，且
///    [`CommitGraph::depth_of`] 会报一个并非"提交不存在"的假事实；
/// 2. **父集合成环** ⇒ [`CommitGraph::ancestry`] 与 [`CommitGraph::ops_backwards`] 沿
///    第一父无限前进（两者都没有 visited 集），表现为**进程挂死**而不是报错。
///
/// 校验通过的图谱，父链上的声明深度**严格递减**，因此它必然无环：每一次遍历都保证终止。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CommitGraphError {
    /// `commits` 的键与提交内嵌的 `id` 不一致。
    #[error("commit key `{key}` does not match the embedded id `{embedded}`")]
    CommitKeyMismatch {
        /// 集合键。
        key: EntityId,
        /// 提交内嵌的身份。
        embedded: EntityId,
    },
    /// 提交引用了不在 `commits` 里的父提交。
    #[error("commit `{commit}` references unknown parent `{parent}`")]
    ParentNotFound {
        /// 引用者。
        commit: EntityId,
        /// 不存在的父提交。
        parent: EntityId,
    },
    /// 提交在深度缓存里没有条目。
    #[error("commit `{commit}` has no entry in `depths`")]
    DepthMissing {
        /// 缺少深度条目的提交。
        commit: EntityId,
    },
    /// 深度缓存里有条目，但它不对应任何提交。
    #[error("`depths` has an entry for unknown commit `{commit}`")]
    DepthOrphan {
        /// 深度缓存里多出来的键。
        commit: EntityId,
    },
    /// 声明的深度与由父提交推导出的深度不一致。
    #[error("commit `{commit}` declares depth {declared} but its parents imply {derived}")]
    DepthInconsistent {
        /// 提交身份。
        commit: EntityId,
        /// 深度缓存里声明的值。
        declared: u64,
        /// 由父提交**声明的**深度推导出的值。
        derived: u64,
    },
    /// 某父提交声明的深度是 `u64::MAX`，因此本提交的深度无法用 `u64` 表示。
    #[error("commit `{commit}` cannot be assigned a depth: parent declares {parent_depth}")]
    DepthOverflow {
        /// 提交身份。
        commit: EntityId,
        /// 父提交声明的深度。
        parent_depth: u64,
    },
    /// 分支头指向不在 `commits` 里的提交。
    #[error("branch `{name}` points at unknown commit `{head}`")]
    BranchHeadNotFound {
        /// 分支名。
        name: String,
        /// 分支头指向的身份。
        head: EntityId,
    },
    /// `branches` 的键与分支内嵌的 `name` 不一致。
    #[error("branch key `{key}` does not match the embedded name `{embedded}`")]
    BranchKeyMismatch {
        /// 集合键。
        key: String,
        /// 分支内嵌的名字。
        embedded: String,
    },
}

/// 提交图谱：提交、分支与深度缓存的唯一事实源 [ARCH-OPS-002]。
///
/// 三个集合全部是 `BTreeMap`（红线 4 / [MODEL-AST-003]），
/// 因此迭代顺序、序列化字节与跨进程行为都是确定的。
///
/// ## 三个集合全部**必需** [ADR-0001 D43]
///
/// `history.dag` 由本写入器整份写出，缺键不可能来自本写入器：它只意味着文件被截断。
/// 缺 `commits` / `branches` 会把"整条撤销历史"静默读成空图谱；缺 `depths`（深度缓存）
/// 虽然可以重新推导，但缺键会让 [`CommitGraph::depth_of`] 退化成误导性的
/// [`ModelError::CommitNotFound`] —— 要求显式写出才能得到**精确**的错误。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct CommitGraph {
    /// 提交集合，键为提交身份。
    pub commits: BTreeMap<EntityId, Commit>,
    /// 分支集合，键为分支名。
    pub branches: BTreeMap<String, BranchHead>,
    /// 深度缓存（根为 1），键为提交身份。
    ///
    /// 缓存的是 DAG 的**因果序号**，由 [`CommitGraph::genesis`] /
    /// [`CommitGraph::append`] / [`CommitGraph::fork_anonymous`] 在写入时维护，
    /// 避免每次撤销都遍历整条祖先链。
    ///
    /// **必需** [ADR-0001 D43]：本写入器总是写出它，缺键只意味着文件被截断。
    pub depths: BTreeMap<EntityId, u64>,
}

impl CommitGraph {
    /// 空图谱。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 检查三个集合互相自洽（**不可信输入的边界**，见 [`CommitGraphError`]）
    /// [ARCH-OPS-002, ARCH-SEC-003]。
    ///
    /// 逐条不变量（全部由四个写入 API 保证，因此它们产出的图谱恒通过）：
    ///
    /// 1. `commits` 的每个键等于该提交内嵌的 `id`；
    /// 2. 每个提交的 `parents` 都在 `commits` 里；
    /// 3. `commits` 与 `depths` 的键集合**完全相同**（不多不少 —— 深度缓存在写提交时
    ///    维护，缺条目会让 [`CommitGraph::depth_of`] 退化成误导性的
    ///    [`ModelError::CommitNotFound`]）；
    /// 4. 每个提交声明的深度 = `max(父提交声明的深度) + 1`（根提交恒为 1）。
    ///    沿父边声明深度**严格递减**，因此这一条本身就排除了环；
    /// 5. 每个分支头的 `head` 在 `commits` 里，且 `branches` 的键等于该分支的 `name`。
    ///
    /// 遍历是单趟、迭代、无递归、不分配的：`O(提交数 + 父边数)`。
    /// **不**要求每个提交都能从某个分支头到达（孤岛提交合法：写入 API 不产生它，
    /// 但它不构成矛盾）。
    ///
    /// # Errors
    ///
    /// 任一条不变量被破坏时返回对应的 [`CommitGraphError`]。
    pub fn validate(&self) -> Result<(), CommitGraphError> {
        for (key, commit) in &self.commits {
            if *key != commit.id {
                return Err(CommitGraphError::CommitKeyMismatch {
                    key: *key,
                    embedded: commit.id,
                });
            }
            let Some(&declared) = self.depths.get(key) else {
                return Err(CommitGraphError::DepthMissing { commit: *key });
            };
            // 只读父提交**声明的**深度（不用任何推导值），因此迭代顺序不影响结论。
            let mut deepest_parent: Option<u64> = None;
            for parent in &commit.parents {
                if !self.commits.contains_key(parent) {
                    return Err(CommitGraphError::ParentNotFound {
                        commit: *key,
                        parent: *parent,
                    });
                }
                let parent_depth = self
                    .depths
                    .get(parent)
                    .copied()
                    .ok_or(CommitGraphError::DepthMissing { commit: *parent })?;
                deepest_parent = Some(match deepest_parent {
                    Some(current) => current.max(parent_depth),
                    None => parent_depth,
                });
            }
            let derived = match deepest_parent {
                None => 1,
                Some(parent_depth) => {
                    parent_depth
                        .checked_add(1)
                        .ok_or(CommitGraphError::DepthOverflow {
                            commit: *key,
                            parent_depth,
                        })?
                }
            };
            if declared != derived {
                return Err(CommitGraphError::DepthInconsistent {
                    commit: *key,
                    declared,
                    derived,
                });
            }
        }
        // `depths` 不得有 `commits` 里没有的键（上面只检查了反方向）。
        for key in self.depths.keys() {
            if !self.commits.contains_key(key) {
                return Err(CommitGraphError::DepthOrphan { commit: *key });
            }
        }
        for (name, head) in &self.branches {
            if name != &head.name {
                return Err(CommitGraphError::BranchKeyMismatch {
                    key: name.clone(),
                    embedded: head.name.clone(),
                });
            }
            if !self.commits.contains_key(&head.head) {
                return Err(CommitGraphError::BranchHeadNotFound {
                    name: name.clone(),
                    head: head.head,
                });
            }
        }
        Ok(())
    }

    /// 创建根提交并建立命名分支 [ARCH-OPS-002]。
    ///
    /// 根提交深度恒为 1，因此**必然**是快照点：`snapshot_ref` 取草稿里的值，
    /// 缺省时按本提交的 ops 计算一个确定性内容摘要。
    ///
    /// # Errors
    ///
    /// 提交身份已存在 → [`ModelError::DuplicateEntityId`]；
    /// 分支名已存在 → [`ModelError::OpStateMismatch`]。
    pub fn genesis(&mut self, draft: CommitDraft) -> Result<EntityId, ModelError> {
        if self.commits.contains_key(&draft.id) {
            return Err(ModelError::DuplicateEntityId { id: draft.id });
        }
        if self.branches.contains_key(&draft.branch_id) {
            return Err(ModelError::OpStateMismatch { op: "genesis" });
        }
        let branch_name = draft.branch_id.clone();
        let id = self.insert_commit(draft, Vec::new(), 1);
        self.branches.insert(
            branch_name.clone(),
            BranchHead {
                name: branch_name,
                head: id,
                anonymous: false,
            },
        );
        Ok(id)
    }

    /// 在当前分支头上追加一次提交 [ARCH-OPS-002]。
    ///
    /// 深度 = 父深度 + 1；当 [`snapshot_due_at_depth`] 为真时 `snapshot_ref`
    /// 取草稿里的值（缺省则按本提交 ops 计算确定性摘要），否则一律为 `None`
    /// （"其余时间重放 ops"）。
    ///
    /// # Errors
    ///
    /// 分支不存在 → [`ModelError::BranchNotFound`]；提交身份已存在 →
    /// [`ModelError::DuplicateEntityId`]。
    pub fn append(&mut self, draft: CommitDraft) -> Result<EntityId, ModelError> {
        if self.commits.contains_key(&draft.id) {
            return Err(ModelError::DuplicateEntityId { id: draft.id });
        }
        let head = self.branch_head(&draft.branch_id)?.head;
        let depth = self.depth_of(&head)? + 1;
        let branch_name = draft.branch_id.clone();
        let id = self.insert_commit(draft, vec![head], depth);
        if let Some(branch) = self.branches.get_mut(&branch_name) {
            branch.head = id;
        }
        Ok(id)
    }

    /// 在**指定提交**上创建一条**命名**分支（不写入任何提交）[ARCH-OPS-002]。
    ///
    /// 与 [`CommitGraph::genesis`]（建根提交）和 [`CommitGraph::fork_anonymous`]
    /// （把分支名强制成 `anon-<提交 ULID>`）互补：隔离分支可以直接指向一个**已存在**
    /// 的提交，分支名由调用方决定，且本方法**不**产生提交、**不**动任何既有分支头。
    ///
    /// # Errors
    ///
    /// `from` 不是已知提交 → [`ModelError::CommitNotFound`]；
    /// 分支名已被占用（含匿名分支）→ [`ModelError::OpStateMismatch`]。
    pub fn create_branch(
        &mut self,
        name: impl Into<String>,
        from: &EntityId,
    ) -> Result<BranchHead, ModelError> {
        if !self.commits.contains_key(from) {
            return Err(ModelError::CommitNotFound { id: *from });
        }
        let name = name.into();
        if self.branches.contains_key(&name) {
            return Err(ModelError::OpStateMismatch {
                op: "create_branch",
            });
        }
        let head = BranchHead {
            name: name.clone(),
            head: *from,
            anonymous: false,
        };
        self.branches.insert(name, head.clone());
        Ok(head)
    }

    /// 在当前分支头上追加一次**多父合并提交** [ARCH-OPS-002]。
    ///
    /// 父集合 = `[当前分支头] ++ extra_parents`。**第一父恒为当前分支头**，
    /// 因此 [`CommitGraph::ancestry`] 与跨提交撤销的主干方向**不变**：
    /// `main` 的祖先链上不会出现被合并那一侧的提交，撤销仍然是
    /// "一次回退整套被合并的操作"。
    ///
    /// 深度 = 所有父的最大深度 + 1（DAG 的**因果序号**）；快照策略与
    /// [`CommitGraph::append`] 相同。
    ///
    /// # Errors
    ///
    /// `extra_parents` 为空 → [`ModelError::OpStateMismatch`]（单父提交请用
    /// [`CommitGraph::append`]）；父集合内有重复 → 同上；任一父不是已知提交 →
    /// [`ModelError::CommitNotFound`]；分支不存在 → [`ModelError::BranchNotFound`]；
    /// 提交身份已存在 → [`ModelError::DuplicateEntityId`]。
    pub fn append_merge(
        &mut self,
        draft: CommitDraft,
        extra_parents: &[EntityId],
    ) -> Result<EntityId, ModelError> {
        if self.commits.contains_key(&draft.id) {
            return Err(ModelError::DuplicateEntityId { id: draft.id });
        }
        if extra_parents.is_empty() {
            return Err(ModelError::OpStateMismatch { op: "append_merge" });
        }
        let head = self.branch_head(&draft.branch_id)?.head;
        let mut parents = Vec::with_capacity(extra_parents.len() + 1);
        parents.push(head);
        parents.extend_from_slice(extra_parents);
        let mut depth = 0_u64;
        for (index, parent) in parents.iter().enumerate() {
            if parents[..index].contains(parent) {
                return Err(ModelError::OpStateMismatch { op: "append_merge" });
            }
            depth = depth.max(self.depth_of(parent)?);
        }
        let branch_name = draft.branch_id.clone();
        let id = self.insert_commit(draft, parents, depth + 1);
        if let Some(branch) = self.branches.get_mut(&branch_name) {
            branch.head = id;
        }
        Ok(id)
    }

    /// 从 `from` 派生**匿名分支**并追加一次提交 [ARCH-OPS-002]。
    ///
    /// 撤销到 `from` 之后继续编辑时调用：原分支头**不动**（作为只读孤岛永久保全），
    /// 新分支名为 `anon-<新提交 ULID>`，父提交是 `from`。
    ///
    /// # Errors
    ///
    /// `from` 不是已知提交 → [`ModelError::CommitNotFound`]；提交身份已存在 →
    /// [`ModelError::DuplicateEntityId`]。
    pub fn fork_anonymous(
        &mut self,
        from: &EntityId,
        draft: CommitDraft,
    ) -> Result<(String, EntityId), ModelError> {
        if self.commits.contains_key(&draft.id) {
            return Err(ModelError::DuplicateEntityId { id: draft.id });
        }
        let depth = self.depth_of(from)? + 1;
        let branch_name = format!(
            "{ANONYMOUS_BRANCH_PREFIX}{}",
            draft.id.to_canonical_string()
        );
        let mut draft = draft;
        draft.branch_id.clone_from(&branch_name);
        let id = self.insert_commit(draft, vec![*from], depth);
        self.branches.insert(
            branch_name.clone(),
            BranchHead {
                name: branch_name.clone(),
                head: id,
                anonymous: true,
            },
        );
        Ok((branch_name, id))
    }

    /// 四个写提交的 API 共用的下半段：按快照策略构造 [`Commit`]，写入
    /// `commits` 与 `depths` 两个集合，返回提交身份。
    ///
    /// 上半段（校验身份/分支、算父集合与深度、更新分支头）各调用点不同，
    /// 因此留在各自的 `pub fn` 里；这一段在四处完全相同，只有它被收进本函数。
    fn insert_commit(
        &mut self,
        draft: CommitDraft,
        parents: Vec<EntityId>,
        depth: u64,
    ) -> EntityId {
        let snapshot_ref = if snapshot_due_at_depth(depth) {
            Some(
                draft
                    .snapshot_ref
                    .unwrap_or_else(|| default_snapshot_hash(&draft.ops)),
            )
        } else {
            None
        };
        let id = draft.id;
        self.commits.insert(
            id,
            Commit {
                id,
                parents,
                branch_id: draft.branch_id,
                author: draft.author,
                message: draft.message,
                created_at: draft.created_at,
                rng_seed: draft.rng_seed,
                ops: draft.ops,
                snapshot_ref,
            },
        );
        self.depths.insert(id, depth);
        id
    }

    /// 只读读取提交。
    ///
    /// # Errors
    ///
    /// 提交不存在 → [`ModelError::CommitNotFound`]。
    pub fn commit(&self, id: &EntityId) -> Result<&Commit, ModelError> {
        self.commits
            .get(id)
            .ok_or(ModelError::CommitNotFound { id: *id })
    }

    /// 只读读取分支头。
    ///
    /// # Errors
    ///
    /// 分支不存在 → [`ModelError::BranchNotFound`]。
    pub fn branch_head(&self, name: &str) -> Result<&BranchHead, ModelError> {
        self.branches
            .get(name)
            .ok_or_else(|| ModelError::BranchNotFound {
                name: name.to_owned(),
            })
    }

    /// 读取提交深度（根为 1）。
    ///
    /// # Errors
    ///
    /// 提交不存在或深度缓存缺失 → [`ModelError::CommitNotFound`]。
    pub fn depth_of(&self, id: &EntityId) -> Result<u64, ModelError> {
        self.depths
            .get(id)
            .copied()
            .ok_or(ModelError::CommitNotFound { id: *id })
    }

    /// 沿第一父提交回溯到根，返回 `[id, parent, …, root]`。
    ///
    /// # Errors
    ///
    /// 链上出现未知提交 → [`ModelError::CommitNotFound`]。
    pub fn ancestry(&self, id: &EntityId) -> Result<Vec<EntityId>, ModelError> {
        let mut chain = Vec::new();
        let mut cursor = Some(*id);
        while let Some(current) = cursor {
            let commit = self.commit(&current)?;
            chain.push(current);
            cursor = commit.first_parent();
        }
        Ok(chain)
    }

    /// **跨 Commit 边界**逆序收集最近 `steps` 条 op（最新的在前）。
    ///
    /// 当祖先链上的 op 用尽（到达根提交）时提前停止，因此返回条数可能少于
    /// `steps` —— 根提交之前没有可回放的操作，继续"撤销"只能靠加载快照，
    /// 那属于存储引擎的职责。
    ///
    /// # Errors
    ///
    /// 链上出现未知提交 → [`ModelError::CommitNotFound`]。
    pub fn ops_backwards(
        &self,
        from: &EntityId,
        steps: usize,
    ) -> Result<Vec<StampedOp>, ModelError> {
        self.ops_backwards_from(from, 0, steps)
    }

    /// 同 [`CommitGraph::ops_backwards`]，但先跳过已经处理过的 `skip` 条 op。
    ///
    /// `skip` 正是 [`UndoCursor::skip`]：连续 `Cmd+Z` 时每次都在同一个头提交上
    /// 重新取数，靠 skip 前移，因此图谱无需保存任何可变游标状态。
    ///
    /// # Errors
    ///
    /// 链上出现未知提交 → [`ModelError::CommitNotFound`]。
    pub fn ops_backwards_from(
        &self,
        from: &EntityId,
        skip: usize,
        steps: usize,
    ) -> Result<Vec<StampedOp>, ModelError> {
        let mut collected: Vec<StampedOp> = Vec::new();
        let mut skipped = 0_usize;
        let mut cursor = Some(*from);
        while collected.len() < steps {
            let Some(current) = cursor else {
                break;
            };
            let commit = self.commit(&current)?;
            for op in commit.ops.iter().rev() {
                if collected.len() >= steps {
                    break;
                }
                if skipped < skip {
                    skipped += 1;
                    continue;
                }
                collected.push(op.clone());
            }
            cursor = commit.first_parent();
        }
        Ok(collected)
    }

    /// 跨 Commit 边界连续撤销：对 `doc` 逆序应用最近 `steps` 条 op 的逆操作。
    ///
    /// 返回**实际**撤销的步数（可能小于 `steps`，见 [`CommitGraph::ops_backwards`]）。
    ///
    /// # Errors
    ///
    /// 任一步的逆操作不可构造或不可应用 → 返回对应 [`ModelError`]，
    /// 且此前已成功的步骤**不会被回滚**（撤销逐步提交，符合 `Cmd+Z` 语义）。
    pub fn undo(
        &self,
        doc: &mut YebanProjectV1,
        head: &EntityId,
        steps: usize,
    ) -> Result<usize, ModelError> {
        self.undo_with(doc, head, &mut UndoCursor::new(), steps)
    }

    /// 用显式游标连续撤销（一次 `Cmd+Z` 调一次，游标由调用方保存）。
    ///
    /// 与 [`CommitGraph::undo`] 的唯一区别是"从哪儿继续"：`cursor.skip`
    /// 记录已经撤销的步数，因此连续调用会**继续往前**而不是重复撤销同一步。
    /// 游标前移在每一步成功之后进行，因此中途失败不会让游标与文档状态错位。
    ///
    /// # Errors
    ///
    /// 任一步的逆操作不可构造或不可应用 → 返回对应 [`ModelError`]。
    pub fn undo_with(
        &self,
        doc: &mut YebanProjectV1,
        head: &EntityId,
        cursor: &mut UndoCursor,
        steps: usize,
    ) -> Result<usize, ModelError> {
        let ops = self.ops_backwards_from(head, cursor.skip, steps)?;
        let mut undone = 0_usize;
        for op in &ops {
            op.apply_inverse(doc)?;
            undone += 1;
            cursor.skip += 1;
        }
        Ok(undone)
    }

    /// 分支总数（含匿名分支）。
    #[must_use]
    pub fn branch_count(&self) -> usize {
        self.branches.len()
    }

    /// 提交总数。
    #[must_use]
    pub fn commit_count(&self) -> usize {
        self.commits.len()
    }
}

/// `history.dag`（规范 §5.3 的提交树条目）的**编解码**错误 [ARCH-OPS-002, ARCH-SEC-003]。
///
/// 只有"读"会失败：`history.dag` 来自磁盘或第三方归档，属不可信输入。
/// "写"的那一侧见 [`encode_history_dag`] —— 它是全函数，不产生错误。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HistoryDagError {
    /// 字节不是合法的 `CommitGraph` JSON（serde 层：语法错、缺必需键、类型不符）。
    #[error("`history.dag` is not a valid CommitGraph JSON: {detail}")]
    InvalidJson {
        /// `serde_json` 的错误文本（含行列位置）。
        detail: String,
    },
    /// JSON 合法、但图谱**自己不自洽**（见 [`CommitGraphError`]）[ARCH-OPS-002]。
    ///
    /// 与 [`HistoryDagError::InvalidJson`] 分开是刻意的：这条错误的主语是**图谱**，
    /// 不是字节的语法。它同样**绝不**降级成"空历史" —— 那等于把用户的历史悄悄丢掉。
    #[error("`history.dag` is not a self-consistent CommitGraph: {detail}")]
    InconsistentGraph {
        /// [`CommitGraph::validate`] 的错误文本。
        detail: String,
    },
}

/// `history.dag` 的**唯一**编码口径 [ARCH-OPS-002, ARCH-SEC-003]。
///
/// ## 为什么这一份口径住在 `yeban-model`
///
/// 条目载荷就是 [`CommitGraph`]。口径若由每个消费者各写一遍就会漂移：
/// `crates/yeban-mcp/src/domain/store.rs` 已有私有的 `decode_history_dag` 与内联的
/// `serde_json::to_vec(graph)`，而 `crates/yeban-app` 不依赖 `serde_json`，于是它
/// **写不出**真实的提交图谱（`force_save` 落的是空字节 `history.dag`）。两条跨 crate 的
/// needs（`docs/ledger/undo-wiring-notes.md` §9 needs-2、`docs/ledger/app-mixer-notes.md`
/// §7 第 7 条）都指向本 crate 提供这一公开面。
///
/// ## 格式
///
/// `serde_json::to_vec(CommitGraph)` —— 紧凑 JSON、无结尾换行，与 `yeban-mcp` 已经在写的
/// 字节**逐字节相同**（判据 `mcp_caliber_bytes_are_accepted`）。键序由 `BTreeMap` 决定
/// （[MODEL-AST-003]），因此同一图谱恒同字节。
///
/// ## 返回值为什么不是 `Result`
///
/// `CommitGraph` 的 JSON 编码是**全函数**：`serde_json` 只有两条失败路径 —— "JSON 对象键
/// 不是字符串"与"自定义 `Serialize` 返回错误" —— 而本类型两者都不存在（对象键只来自
/// [`EntityId`] 与 `String`，两者都序列化成字符串；没有手写的 `Serialize`）。非有限浮点
/// 被 `serde_json` 写成语义等价的 `null`，不是错误。判据
/// `non_finite_op_payload_still_encodes_and_decode_rejects_it` 钉住这一条。
///
/// # Panics
///
/// 仅当上面那条不变量被将来的改动破坏时才会 panic；本函数**不**接收不可信输入。
#[must_use]
pub fn encode_history_dag(graph: &CommitGraph) -> Vec<u8> {
    serde_json::to_vec(graph).expect("CommitGraph 的 JSON 编码不会失败（见函数文档的不变量）")
}

/// 解析 `history.dag` 的字节 [ARCH-OPS-002, ARCH-SEC-003]。
///
/// ## 口径（与 `yeban-mcp` 既有私有实现一致的部分）
///
/// - 空字节 ⇒ `Ok(None)`（容器条目存在，但还没有历史）；
/// - 零提交图谱（`{"commits":{},"branches":{},"depths":{}}`）⇒ `Ok(None)`（同上）；
/// - 合法且**自洽**的图谱 ⇒ `Ok(Some(graph))`；
/// - 非法 JSON，或 JSON 合法但形状不符（例如缺 `commits` / `branches` / `depths`
///   这三个必需键 [ADR-0001 D43]）⇒ [`HistoryDagError::InvalidJson`]；
/// - JSON 合法、但图谱自己不自洽（引用不存在的父提交、深度缓存与父集合矛盾、
///   父集合成环、分支头悬空 …）⇒ [`HistoryDagError::InconsistentGraph`]，
///   判据就是 [`CommitGraph::validate`]。
///
/// **绝不**静默降级成"空历史"：那等于把用户的历史悄悄丢掉。
///
/// ## 为什么自洽性属于本层
///
/// 这是**不可信输入**进入模型层的那一格。放行一份父集合成环的图谱会让
/// [`CommitGraph::ancestry`] / [`CommitGraph::ops_backwards`] 无限前进（挂死），
/// 放行一份深度缓存被改过的图谱会让快照点从此算错 —— 两者都不会在读取时发出任何信号，
/// 因此必须在**读取的那一刻**拒绝。容器层（`read_project_container`）刻意**不**做这件事：
/// 它按规范只搬运字节、不解读 `history.dag`（判据
/// `history_dag_bytes_land_in_the_named_entry` 把"任意字节都逐字节往返"钉死），
/// 所以边界只能在这里。
///
/// ## 刻意**不**在这一层做的判定
///
/// 分支命名策略（`yeban-mcp` 要求非空图谱必须有 `main` 分支）留在 MCP 的工具面 ——
/// 那是工具的行为契约，不是 `history.dag` 的格式契约。判据
/// `decode_does_not_impose_branch_naming_policy` 把这条分工钉住。
///
/// # Errors
///
/// 字节不是合法的 `CommitGraph` JSON 时返回 [`HistoryDagError::InvalidJson`]；
/// 语法合法但图谱不自洽时返回 [`HistoryDagError::InconsistentGraph`]。
pub fn decode_history_dag(raw: &[u8]) -> Result<Option<CommitGraph>, HistoryDagError> {
    if raw.is_empty() {
        return Ok(None);
    }
    let graph: CommitGraph =
        serde_json::from_slice(raw).map_err(|error| HistoryDagError::InvalidJson {
            detail: error.to_string(),
        })?;
    // 自洽性**先于**"零提交 ⇒ None"：一份声明了分支但没有任何提交的图谱不是
    // "还没有历史"，而是自相矛盾，必须报错而不是被静默读成空历史。
    graph
        .validate()
        .map_err(|error| HistoryDagError::InconsistentGraph {
            detail: error.to_string(),
        })?;
    if graph.commits.is_empty() {
        return Ok(None);
    }
    Ok(Some(graph))
}

/// 按 ops 计算确定性内容摘要（缺省快照引用）。
///
/// 真实的全量快照哈希由存储引擎在落盘时计算（`ARCH-OPS-002`）；
/// 这里给出一个**确定性**的模型层替身，使快照策略本身可被测试与断言。
#[must_use]
fn default_snapshot_hash(ops: &[StampedOp]) -> ContentHash {
    let mut buffer = String::new();
    for op in ops {
        // 模型层的全部字段都可序列化，`write!` 到 `String` 不会失败；
        // 即便失败也只是少写一段文本，摘要仍然是确定的。
        let _ = write!(buffer, "{op:?}");
    }
    ContentHash::of_bytes(buffer.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{Op, OpOrigin};
    use crate::project::{SectionV3, TrackKind, TrackV3, YebanProjectV1};
    use std::str::FromStr;

    fn fixture_id(index: u128) -> EntityId {
        EntityId::from_str(&format!("01J8ZQ{index:020}")).expect("canonical fixture ulid")
    }

    fn stamped(op: Op) -> StampedOp {
        StampedOp::new(OpOrigin::UserUi, 1_760_000_000_000, op)
    }

    /// 一个只新增曲式段落的操作（可逆：逆操作是 `RemoveSection`）。
    fn add_section_op(index: u128) -> StampedOp {
        let id = fixture_id(index);
        stamped(Op::SetSection {
            section_id: id,
            old_section: None,
            new_section: SectionV3 {
                id,
                name: format!("Section {index}"),
                start_tick: 0,
                end_tick: 960,
                color: None,
            },
        })
    }

    /// 一个带主总线的文档（新增音轨用它才合法）。
    fn project_with_master() -> YebanProjectV1 {
        let master_id = fixture_id(1);
        YebanProjectV1 {
            tracks: BTreeMap::from([(
                master_id,
                TrackV3 {
                    id: master_id,
                    name: "Master".to_owned(),
                    kind: TrackKind::Master,
                    ..TrackV3::default()
                },
            )]),
            master_bus_track_id: master_id,
            ..YebanProjectV1::default()
        }
    }

    /// 正向应用最近 `steps` 条 op（把文档推到某个提交的状态）。
    fn forward(doc: &mut YebanProjectV1, graph: &CommitGraph, head: &EntityId, steps: usize) {
        for op in graph.ops_backwards(head, steps).expect("ops").iter().rev() {
            op.apply(doc).expect("apply forwards");
        }
    }

    #[test]
    fn snapshot_points_are_one_and_every_256_commits() {
        assert!(snapshot_due_at_depth(1));
        assert!(snapshot_due_at_depth(257));
        assert!(snapshot_due_at_depth(513));
        assert!(!snapshot_due_at_depth(0));
        assert!(!snapshot_due_at_depth(2));
        assert!(!snapshot_due_at_depth(256));
        assert!(!snapshot_due_at_depth(258));
        assert_eq!(SNAPSHOT_INTERVAL, 256);
    }

    #[test]
    fn only_snapshot_depths_carry_a_snapshot_ref() {
        let mut graph = CommitGraph::new();
        let mut last = graph
            .genesis(
                CommitDraft::new(fixture_id(1), "main", "agent", "genesis")
                    .with_ops(vec![add_section_op(100)]),
            )
            .expect("genesis");
        assert!(graph.commit(&last).expect("commit").has_snapshot());
        assert_eq!(graph.depth_of(&last).expect("depth"), 1);
        assert!(snapshot_due_at_depth(1));

        for depth in 2..=300_u128 {
            let current = graph
                .append(
                    CommitDraft::new(fixture_id(depth), "main", "agent", "step")
                        .with_created_at(u64::try_from(depth).expect("fits"))
                        .with_rng_seed(7)
                        .with_ops(vec![add_section_op(1000 + depth)]),
                )
                .expect("append");
            assert_eq!(
                graph.depth_of(&current).expect("depth"),
                u64::try_from(depth).expect("fits")
            );
            let commit = graph.commit(&current).expect("commit");
            // 只有深度 257 是快照点（深度 1 是根提交，已在前面断言）。
            assert_eq!(
                commit.has_snapshot(),
                depth == 257,
                "深度 {depth} 的快照判定错误"
            );
            assert_eq!(
                commit.snapshot_due(u64::try_from(depth).expect("fits")),
                depth == 257
            );
            last = current;
        }
        assert_eq!(graph.commit_count(), 300);
        assert_eq!(graph.branch_count(), 1);
        assert_eq!(graph.depth_of(&last).expect("depth"), 300);
    }

    #[test]
    fn genesis_and_append_reject_duplicates_and_unknown_branches() {
        let mut graph = CommitGraph::new();
        let genesis = fixture_id(1);
        graph
            .genesis(CommitDraft::new(genesis, "main", "agent", "genesis"))
            .expect("genesis");
        assert_eq!(
            graph.append(CommitDraft::new(fixture_id(2), "nope", "a", "m")),
            Err(ModelError::BranchNotFound {
                name: "nope".to_owned()
            })
        );
        assert_eq!(
            graph.append(CommitDraft::new(genesis, "main", "a", "m")),
            Err(ModelError::DuplicateEntityId { id: genesis })
        );
        assert_eq!(
            graph.genesis(CommitDraft::new(genesis, "other", "a", "m")),
            Err(ModelError::DuplicateEntityId { id: genesis })
        );
        assert_eq!(
            graph.genesis(CommitDraft::new(fixture_id(9), "main", "a", "m")),
            Err(ModelError::OpStateMismatch { op: "genesis" })
        );
    }

    #[test]
    fn undo_crosses_commit_boundaries() {
        let mut graph = CommitGraph::new();
        let mut doc = YebanProjectV1::default();
        let initial = doc.clone();

        graph
            .genesis(
                CommitDraft::new(fixture_id(1), "main", "agent", "c1")
                    .with_ops(vec![add_section_op(11), add_section_op(12)]),
            )
            .expect("genesis");
        for (index, ops) in [
            (2_u128, vec![add_section_op(13), add_section_op(14)]),
            (3, vec![add_section_op(15), add_section_op(16)]),
        ] {
            graph
                .append(CommitDraft::new(fixture_id(index), "main", "agent", "c").with_ops(ops))
                .expect("append");
        }
        let head = graph.branch_head("main").expect("main").head;
        assert_eq!(graph.depth_of(&head).expect("depth"), 3);

        forward(&mut doc, &graph, &head, 6);
        assert_eq!(doc.sections.len(), 6);

        // 跨三个提交连续撤销 5 步：c3 的两步、c2 的两步、c1 的最后一步。
        let mut cursor = UndoCursor::new();
        let undone = graph
            .undo_with(&mut doc, &head, &mut cursor, 5)
            .expect("undo");
        assert_eq!(undone, 5);
        assert_eq!(cursor.undone(), 5);
        assert_eq!(doc.sections.len(), 1);
        assert!(doc.sections.contains_key(&fixture_id(11)));

        // 再撤销一步就回到初始状态（跨过 c1 的另一步，游标继续前移）。
        assert_eq!(
            graph
                .undo_with(&mut doc, &head, &mut cursor, 1)
                .expect("undo"),
            1
        );
        assert_eq!(cursor.undone(), 6);
        assert_eq!(doc, initial);

        // 游标已经到根提交之前，继续撤销只返回 0，绝不报错。
        assert_eq!(
            graph
                .undo_with(&mut doc, &head, &mut cursor, 1)
                .expect("undo"),
            0
        );
        assert_eq!(doc, initial);
    }

    #[test]
    fn undo_stops_when_the_root_commit_runs_out_of_ops() {
        let mut graph = CommitGraph::new();
        let mut doc = YebanProjectV1::default();
        let head = graph
            .genesis(
                CommitDraft::new(fixture_id(1), "main", "agent", "c1")
                    .with_ops(vec![add_section_op(11)]),
            )
            .expect("genesis");
        forward(&mut doc, &graph, &head, 1);
        assert_eq!(doc.sections.len(), 1);

        let mut cursor = UndoCursor::new();
        assert_eq!(
            graph
                .undo_with(&mut doc, &head, &mut cursor, 100)
                .expect("undo"),
            1
        );
        assert!(doc.sections.is_empty());
        assert_eq!(cursor.undone(), 1);
        // 已经到根提交、没有更多 op 可撤销（返回 0 而不是报错）。
        assert_eq!(
            graph
                .undo_with(&mut doc, &head, &mut cursor, 100)
                .expect("undo"),
            0
        );
    }

    #[test]
    fn anonymous_fork_preserves_the_original_branch_island() {
        let mut graph = CommitGraph::new();
        let mut doc = project_with_master();
        let rollback_point = graph
            .genesis(
                CommitDraft::new(fixture_id(1), "main", "agent", "c1")
                    .with_ops(vec![add_section_op(11)]),
            )
            .expect("genesis");
        let head = graph
            .append(
                CommitDraft::new(fixture_id(2), "main", "agent", "c2")
                    .with_ops(vec![add_section_op(12)]),
            )
            .expect("append");

        forward(&mut doc, &graph, &head, 2);
        assert_eq!(doc.sections.len(), 2);
        // Cmd+Z 一步 → 回到 `rollback_point` 的状态。
        assert_eq!(graph.undo(&mut doc, &head, 1).expect("undo"), 1);
        assert_eq!(doc.sections.len(), 1);
        assert_eq!(graph.depth_of(&rollback_point).expect("depth"), 1);

        // 撤销后继续编辑 → 从回退点自动派生匿名分支。
        let new_track = fixture_id(500);
        let (branch_name, fork_commit) = graph
            .fork_anonymous(
                &rollback_point,
                CommitDraft::new(fixture_id(3), "", "agent", "post-undo edit").with_ops(vec![
                    stamped(Op::AddTrack {
                        track: TrackV3 {
                            id: new_track,
                            name: "New".to_owned(),
                            ..TrackV3::default()
                        },
                    }),
                ]),
            )
            .expect("fork");
        for op in &graph.commit(&fork_commit).expect("commit").ops {
            op.apply(&mut doc).expect("apply fork op");
        }
        assert!(doc.tracks.contains_key(&new_track));

        // 原分叉作为只读孤岛永久保全：main 的头仍然是 c2。
        assert_eq!(graph.branch_head("main").expect("main").head, head);
        assert!(!graph.branch_head("main").expect("main").anonymous);
        let anonymous = graph.branch_head(&branch_name).expect("anon");
        assert!(anonymous.anonymous);
        assert_eq!(anonymous.head, fork_commit);
        assert!(branch_name.starts_with(ANONYMOUS_BRANCH_PREFIX));
        assert_eq!(
            graph.commit(&fork_commit).expect("commit").parents,
            vec![rollback_point]
        );
        assert_eq!(graph.depth_of(&fork_commit).expect("depth"), 2);
        assert_eq!(graph.branch_count(), 2);
        assert_eq!(graph.commit_count(), 3);

        // 匿名提交自身也可以被撤销。
        assert_eq!(graph.undo(&mut doc, &fork_commit, 1).expect("undo"), 1);
        assert!(!doc.tracks.contains_key(&new_track));
    }

    #[test]
    fn create_branch_points_at_an_existing_commit_without_writing_one() {
        let mut graph = CommitGraph::new();
        let base = graph
            .genesis(CommitDraft::new(fixture_id(1), "main", "agent", "genesis"))
            .expect("genesis");
        let head = graph
            .append(CommitDraft::new(fixture_id(2), "main", "agent", "second"))
            .expect("append");
        let commits_before = graph.commit_count();

        // 分支指向 `base`（不是最新头）——命名分支不必等于"当前头"。
        let branch = graph.create_branch("ai/proposal-1", &base).expect("branch");
        assert_eq!(branch.name, "ai/proposal-1");
        assert_eq!(branch.head, base);
        assert!(!branch.anonymous, "create_branch 只建命名分支");
        assert_eq!(graph.branch_count(), 2);
        assert_eq!(
            graph.commit_count(),
            commits_before,
            "create_branch 不产生提交"
        );
        assert_eq!(
            graph.branch_head("main").expect("main").head,
            head,
            "既有分支头不动"
        );
        assert_eq!(graph.branch_head("ai/proposal-1").expect("p1").head, base);

        // 新分支上可以继续 `append`，且不影响 main。
        let proposal_head = graph
            .append(CommitDraft::new(
                fixture_id(3),
                "ai/proposal-1",
                "agent",
                "proposal commit",
            ))
            .expect("append on the new branch");
        assert_eq!(
            graph.commit(&proposal_head).expect("commit").parents,
            vec![base],
            "提案分支从 base 长出去，而不是从 main 的头长出去"
        );
        assert_eq!(graph.depth_of(&proposal_head).expect("depth"), 2);
        assert_eq!(graph.branch_head("main").expect("main").head, head);
    }

    #[test]
    fn create_branch_rejects_unknown_commits_and_taken_names() {
        let mut graph = CommitGraph::new();
        let genesis = fixture_id(1);
        graph
            .genesis(CommitDraft::new(genesis, "main", "agent", "genesis"))
            .expect("genesis");

        let ghost = fixture_id(77);
        assert_eq!(
            graph.create_branch("ghost-branch", &ghost),
            Err(ModelError::CommitNotFound { id: ghost })
        );
        assert_eq!(
            graph.create_branch("main", &genesis),
            Err(ModelError::OpStateMismatch {
                op: "create_branch"
            }),
            "分支名已被占用（含既有命名分支）必须拒绝"
        );

        // 匿名分支也占用名字空间：`fork_anonymous` 派生出的名字不能再被命名分支重用。
        let (anon, _) = graph
            .fork_anonymous(
                &genesis,
                CommitDraft::new(fixture_id(3), "", "agent", "fork"),
            )
            .expect("fork");
        assert_eq!(
            graph.create_branch(anon.clone(), &genesis),
            Err(ModelError::OpStateMismatch {
                op: "create_branch"
            })
        );
        assert_eq!(graph.branch_count(), 2, "两次被拒的调用不得留下分支");
    }

    #[test]
    fn merge_commit_records_every_parent_and_keeps_the_main_line_on_the_first() {
        let mut graph = CommitGraph::new();
        let mut doc = project_with_master();
        let base = graph
            .genesis(
                CommitDraft::new(fixture_id(1), "main", "agent", "c1")
                    .with_ops(vec![add_section_op(11)]),
            )
            .expect("genesis");
        let main_head = graph
            .append(
                CommitDraft::new(fixture_id(2), "main", "agent", "c2")
                    .with_ops(vec![add_section_op(12)]),
            )
            .expect("append");

        // 隔离的提案分支：从 `base` 派生命名分支，长出**两条**提交
        // ⇒ 提案侧比 main 侧更深，`深度 = 最大父深度 + 1` 才有判别力
        // （只取第一父深度的实现会得到 3，而正确值是 4）。
        graph.create_branch("ai/proposal-1", &base).expect("branch");
        let proposal_first = graph
            .append(
                CommitDraft::new(fixture_id(3), "ai/proposal-1", "agent", "proposal 1")
                    .with_ops(vec![add_section_op(13)]),
            )
            .expect("append proposal 1");
        let proposal_head = graph
            .append(CommitDraft::new(
                fixture_id(5),
                "ai/proposal-1",
                "agent",
                "proposal 2",
            ))
            .expect("append proposal 2");
        assert_eq!(graph.depth_of(&proposal_first).expect("depth"), 2);
        assert_eq!(graph.depth_of(&proposal_head).expect("depth"), 3);

        // 合并：第一父 = main 的头（深度 2），第二父 = 提案分支头（深度 3）。
        let merge = graph
            .append_merge(
                CommitDraft::new(fixture_id(4), "main", "agent", "merge proposal")
                    .with_ops(vec![add_section_op(14)]),
                &[proposal_head],
            )
            .expect("merge");
        let merged = graph.commit(&merge).expect("merge commit");
        assert_eq!(merged.parents, vec![main_head, proposal_head]);
        assert!(merged.is_merge());
        assert_eq!(merged.first_parent(), Some(main_head));
        assert_eq!(merged.branch_id, "main");
        assert_eq!(
            graph.depth_of(&merge).expect("depth"),
            4,
            "深度 = 最大父深度 + 1（第一父 2，第二父 3）"
        );
        assert_ne!(
            graph.depth_of(&merge).expect("depth"),
            graph.depth_of(&main_head).expect("depth") + 1,
            "深度不得只看第一父"
        );
        assert_eq!(graph.branch_head("main").expect("main").head, merge);
        assert_eq!(
            graph.branch_head("ai/proposal-1").expect("p1").head,
            proposal_head,
            "被合并的一侧不动"
        );

        // 主干方向不变：ancestry 沿第一父走，提案提交**不在** main 的祖先链上。
        assert_eq!(
            graph.ancestry(&merge).expect("ancestry"),
            vec![merge, main_head, base]
        );
        assert!(
            !graph
                .ancestry(&merge)
                .expect("ancestry")
                .contains(&proposal_head)
        );

        // 撤销链同样只含第一父那一侧：提案分支的 op 不混进主分支。
        let undoable = graph.ops_backwards(&merge, 10).expect("ops");
        assert_eq!(undoable.len(), 3, "merge 自己的 op + main 两个提交的 op");
        let touched: Vec<EntityId> = undoable
            .iter()
            .filter_map(|stamped| match &stamped.op {
                Op::SetSection { section_id, .. } => Some(*section_id),
                _ => None,
            })
            .collect();
        assert_eq!(
            touched,
            vec![fixture_id(14), fixture_id(12), fixture_id(11)]
        );
        assert!(
            !touched.contains(&fixture_id(13)),
            "提案分支的 op 不得进入主分支的撤销链"
        );

        // 合并提交与普通提交一样可逆：一次撤销回退它携带的 op。
        forward(&mut doc, &graph, &merge, 2);
        assert!(doc.sections.contains_key(&fixture_id(12)));
        assert!(doc.sections.contains_key(&fixture_id(14)));
        assert!(!doc.sections.contains_key(&fixture_id(13)));
        assert_eq!(graph.undo(&mut doc, &merge, 1).expect("undo"), 1);
        assert!(!doc.sections.contains_key(&fixture_id(14)));
        assert!(doc.sections.contains_key(&fixture_id(12)));
    }

    #[test]
    fn append_merge_rejects_degenerate_parent_sets() {
        let mut graph = CommitGraph::new();
        let base = graph
            .genesis(CommitDraft::new(fixture_id(1), "main", "agent", "genesis"))
            .expect("genesis");
        let head = graph
            .append(CommitDraft::new(fixture_id(2), "main", "agent", "second"))
            .expect("append");

        // 空 `extra_parents` ⇒ 单父提交，请走 `append`。
        assert_eq!(
            graph.append_merge(
                CommitDraft::new(fixture_id(3), "main", "agent", "not a merge"),
                &[],
            ),
            Err(ModelError::OpStateMismatch { op: "append_merge" })
        );
        // 重复父（当前头自己在 `extra_parents` 里）。
        assert_eq!(
            graph.append_merge(
                CommitDraft::new(fixture_id(3), "main", "agent", "self parent"),
                &[head],
            ),
            Err(ModelError::OpStateMismatch { op: "append_merge" })
        );
        // 重复父（`extra_parents` 内部重复）。
        assert_eq!(
            graph.append_merge(
                CommitDraft::new(fixture_id(3), "main", "agent", "twice"),
                &[base, base],
            ),
            Err(ModelError::OpStateMismatch { op: "append_merge" })
        );
        // 未知父。
        let ghost = fixture_id(77);
        assert_eq!(
            graph.append_merge(
                CommitDraft::new(fixture_id(3), "main", "agent", "ghost"),
                &[ghost],
            ),
            Err(ModelError::CommitNotFound { id: ghost })
        );
        // 未知分支。
        assert_eq!(
            graph.append_merge(
                CommitDraft::new(fixture_id(3), "ghost", "agent", "branch"),
                &[base],
            ),
            Err(ModelError::BranchNotFound {
                name: "ghost".to_owned()
            })
        );
        // 提交身份已存在。
        assert_eq!(
            graph.append_merge(
                CommitDraft::new(fixture_id(2), "main", "agent", "duplicate id"),
                &[base],
            ),
            Err(ModelError::DuplicateEntityId { id: fixture_id(2) })
        );
        assert_eq!(graph.commit_count(), 2, "五次被拒的调用不得留下提交");
        assert_eq!(graph.branch_count(), 1);
        assert_eq!(
            graph.branch_head("main").expect("main").head,
            head,
            "被拒的合并不得推进分支头"
        );
    }

    #[test]
    fn ancestry_and_error_paths() {
        let mut graph = CommitGraph::new();
        let genesis = fixture_id(1);
        graph
            .genesis(CommitDraft::new(genesis, "main", "a", "m"))
            .expect("genesis");
        let head = graph
            .append(CommitDraft::new(fixture_id(2), "main", "a", "m"))
            .expect("append");
        assert_eq!(
            graph.ancestry(&head).expect("ancestry"),
            vec![head, genesis]
        );

        let ghost = fixture_id(77);
        assert_eq!(
            graph.commit(&ghost),
            Err(ModelError::CommitNotFound { id: ghost })
        );
        assert_eq!(
            graph.depth_of(&ghost),
            Err(ModelError::CommitNotFound { id: ghost })
        );
        assert_eq!(
            graph.branch_head("ghost"),
            Err(ModelError::BranchNotFound {
                name: "ghost".to_owned()
            })
        );
        assert_eq!(
            graph.ops_backwards(&ghost, 1),
            Err(ModelError::CommitNotFound { id: ghost })
        );
        assert!(graph.ancestry(&ghost).is_err());
    }

    #[test]
    fn commit_graph_serde_round_trip() {
        let mut graph = CommitGraph::new();
        graph
            .genesis(
                CommitDraft::new(fixture_id(1), "main", "agent", "genesis")
                    .with_rng_seed(3)
                    .with_ops(vec![add_section_op(11)]),
            )
            .expect("genesis");
        graph
            .append(
                CommitDraft::new(fixture_id(2), "main", "agent", "second")
                    .with_ops(vec![add_section_op(12)]),
            )
            .expect("append");
        let json = serde_json::to_string(&graph).expect("serialize");
        let back: CommitGraph = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, graph);
        assert_eq!(back.ops_backwards(&fixture_id(2), 2).expect("ops").len(), 2);
        assert_eq!(back.branch_head("main").expect("main").head, fixture_id(2));
    }

    #[test]
    fn default_snapshot_hash_is_deterministic_and_content_sensitive() {
        let ops = vec![add_section_op(11), add_section_op(12)];
        let first = default_snapshot_hash(&ops);
        let second = default_snapshot_hash(&ops.clone());
        assert_eq!(first, second, "同一 ops 必须得到同一摘要");
        let mut different = ops.clone();
        different.push(add_section_op(13));
        assert_ne!(first, default_snapshot_hash(&different));
        assert_eq!(first.as_str().len(), 64);
    }

    /// 一个有形状的图谱：根提交 + 一次追加 + 一条匿名分支。
    ///
    /// 三个集合（`commits` / `branches` / `depths`）都非空，且同时含命名分支与匿名分支
    /// ⇒ 编码漏写任一处都会让字节比较变红。
    fn fixture_graph() -> CommitGraph {
        let mut graph = CommitGraph::new();
        graph
            .genesis(
                CommitDraft::new(fixture_id(1), "main", "agent", "genesis")
                    .with_created_at(1_760_000_000_000)
                    .with_rng_seed(3)
                    .with_ops(vec![add_section_op(11)]),
            )
            .expect("genesis");
        graph
            .append(
                CommitDraft::new(fixture_id(2), "main", "agent", "second")
                    .with_created_at(1_760_000_000_001)
                    .with_rng_seed(4)
                    .with_ops(vec![add_section_op(12)]),
            )
            .expect("append");
        graph
            .fork_anonymous(
                &fixture_id(1),
                CommitDraft::new(fixture_id(3), "anon-placeholder", "agent", "fork")
                    .with_created_at(1_760_000_000_002)
                    .with_rng_seed(5)
                    .with_ops(vec![add_section_op(13)]),
            )
            .expect("fork");
        graph
    }

    #[test]
    fn history_dag_round_trips_and_is_byte_stable() {
        let graph = fixture_graph();
        let bytes = encode_history_dag(&graph);
        assert!(!bytes.is_empty());
        assert_eq!(
            decode_history_dag(&bytes).expect("decode"),
            Some(graph.clone()),
            "编码后必须逐字段读回同一图谱"
        );
        assert_eq!(bytes, encode_history_dag(&graph), "同一图谱必须恒同字节");
        assert_eq!(
            bytes,
            serde_json::to_vec(&graph).expect("serde_json"),
            "口径 = serde_json::to_vec(CommitGraph)，即 yeban-mcp 已在写的同一份字节"
        );
    }

    #[test]
    fn empty_or_zero_commit_history_dag_is_none() {
        assert_eq!(decode_history_dag(b"").expect("空条目"), None);
        let zero = CommitGraph::new();
        assert_eq!(zero.commit_count(), 0);
        assert_eq!(
            decode_history_dag(&encode_history_dag(&zero)).expect("零提交图谱"),
            None,
            "零提交图谱与空条目同义：都表示`还没有历史`"
        );
    }

    #[test]
    fn history_dag_rejects_broken_bytes_without_panicking() {
        let graph = fixture_graph();
        let bytes = encode_history_dag(&graph);
        let cases: [&[u8]; 6] = [
            b"not json",
            b"[]",
            b"null",
            b"{}",
            b"{\"commits\":{},\"branches\":{}}",
            &bytes[..bytes.len() / 2],
        ];
        for raw in cases {
            let error = decode_history_dag(raw).expect_err("必须拒绝，不得静默降级成空历史");
            assert!(
                matches!(error, HistoryDagError::InvalidJson { .. }),
                "字节 {raw:?} ⇒ {error}"
            );
        }
    }

    #[test]
    fn decode_does_not_impose_branch_naming_policy() {
        // 只有 `trunk`、没有 `main`：格式层接受；"非空图谱必须有 main" 是 MCP 工具面的策略。
        let mut graph = CommitGraph::new();
        graph
            .genesis(CommitDraft::new(fixture_id(1), "trunk", "agent", "genesis"))
            .expect("genesis");
        assert!(!graph.branches.contains_key("main"));
        assert_eq!(
            decode_history_dag(&encode_history_dag(&graph)).expect("decode"),
            Some(graph)
        );
    }

    #[test]
    fn non_finite_op_payload_still_encodes_and_decode_rejects_it() {
        // 人工构造的非有限载荷。正常写入路径不可达（`Op::apply` 先报 `NonFiniteValue`），
        // 但它钉住两件事：`encode_history_dag` 是全函数（`serde_json` 把非有限浮点写成
        // `null`，不是错误），而 `null` 回到 `f32` 字段会被解码拒绝。
        let mut graph = CommitGraph::new();
        graph
            .genesis(
                CommitDraft::new(fixture_id(1), "main", "agent", "genesis").with_ops(vec![
                    stamped(Op::SetParam {
                        target: crate::project::AutomationTarget::TrackVolume {
                            track_id: fixture_id(1),
                        },
                        old_val: 0.0,
                        new_val: f32::NAN,
                    }),
                ]),
            )
            .expect("genesis");
        let bytes = encode_history_dag(&graph);
        assert!(
            String::from_utf8_lossy(&bytes).contains("null"),
            "非有限浮点必须被写成 null"
        );
        assert!(matches!(
            decode_history_dag(&bytes),
            Err(HistoryDagError::InvalidJson { .. })
        ));
    }

    /// 直接构造一个提交（不经过写入 API）—— 对抗性判据需要它。
    fn bare_commit(id: EntityId, parents: Vec<EntityId>) -> Commit {
        Commit {
            id,
            parents,
            branch_id: "main".to_owned(),
            author: "agent".to_owned(),
            message: "m".to_owned(),
            created_at: 0,
            rng_seed: 0,
            ops: Vec::new(),
            snapshot_ref: None,
        }
    }

    /// 一个只有 `main` 分支、指向 `head` 的最小图谱（三个集合都非空）。
    fn bare_graph(
        commits: Vec<Commit>,
        head: EntityId,
        depths: Vec<(EntityId, u64)>,
    ) -> CommitGraph {
        let mut graph = CommitGraph::new();
        for commit in commits {
            graph.commits.insert(commit.id, commit);
        }
        for (id, depth) in depths {
            graph.depths.insert(id, depth);
        }
        graph.branches.insert(
            "main".to_owned(),
            BranchHead {
                name: "main".to_owned(),
                head,
                anonymous: false,
            },
        );
        graph
    }

    #[test]
    fn validate_accepts_every_graph_the_write_apis_produce() {
        let mut graph = fixture_graph();
        graph.validate().expect("genesis / append / fork_anonymous");

        // `create_branch` 不写提交，只在既有提交上挂一个命名分支头。
        graph
            .create_branch("proposal", &fixture_id(1))
            .expect("create_branch");
        graph.validate().expect("create_branch");

        // 多父合并：第一父是 `main` 的头，另一父取匿名分支的头。
        let anon_name = graph
            .branches
            .keys()
            .find(|name| name.starts_with(ANONYMOUS_BRANCH_PREFIX))
            .cloned()
            .expect("fork_anonymous 必须派生匿名分支");
        let anon_head = graph.branch_head(&anon_name).expect("anon").head;
        let merge = graph
            .append_merge(
                CommitDraft::new(fixture_id(9), "main", "agent", "merge")
                    .with_ops(vec![add_section_op(19)]),
                &[anon_head],
            )
            .expect("append_merge");
        assert!(graph.commit(&merge).expect("merge").is_merge());
        assert_eq!(graph.depth_of(&merge).expect("depth"), 3);
        graph.validate().expect("append_merge");

        // 深度跨过快照间隔（257）的长链同样必须自洽。
        let mut deep = CommitGraph::new();
        deep.genesis(CommitDraft::new(fixture_id(1), "main", "agent", "genesis"))
            .expect("genesis");
        for index in 2..=258_u128 {
            deep.append(CommitDraft::new(fixture_id(index), "main", "agent", "step"))
                .expect("append");
        }
        let head = deep.branch_head("main").expect("main").head;
        assert_eq!(deep.depth_of(&head).expect("depth"), 258);
        assert!(
            deep.commit(&fixture_id(257))
                .expect("depth 257")
                .has_snapshot(),
            "深度 257 的提交必须带全量快照"
        );
        deep.validate().expect("258 条提交的链");
    }

    #[test]
    fn validate_rejects_a_cycle_and_names_the_depth_contradiction() {
        // 两个提交互为父提交：两个身份都存在，父边都存在，因此"引用完整性"查不出它。
        // 只有深度递推能抓住它 —— 也正是这条递推保证了解码后的遍历必然终止。
        let a = fixture_id(1);
        let b = fixture_id(2);
        let graph = bare_graph(
            vec![bare_commit(a, vec![b]), bare_commit(b, vec![a])],
            a,
            vec![(a, 2), (b, 2)],
        );
        let error = graph.validate().expect_err("环必须被拒绝");
        assert!(
            matches!(error, CommitGraphError::DepthInconsistent { .. }),
            "环应表现为深度矛盾，实测：{error}"
        );
        assert!(
            error.to_string().contains("declares depth"),
            "错误文本必须点名深度矛盾，实测：{error}"
        );
        // 同一个图谱经"编解码"（= 不可信输入的真实形状）同样被拒。
        assert!(
            matches!(
                decode_history_dag(&encode_history_dag(&graph)),
                Err(HistoryDagError::InconsistentGraph { .. })
            ),
            "解码边界必须拒绝它，否则 ancestry / ops_backwards 会挂死"
        );
    }

    #[test]
    fn a_poisoned_depth_cache_is_rejected_before_it_can_shift_the_snapshot_grid() {
        // 三提交链，深度缓存被改成"全是 1"。放行它会让下一次 append 得到深度 2，
        // 于是快照点（1, 257, 513, …）从此算错；这里必须在读取时拒绝。
        let a = fixture_id(1);
        let b = fixture_id(2);
        let c = fixture_id(3);
        let graph = bare_graph(
            vec![
                bare_commit(a, Vec::new()),
                bare_commit(b, vec![a]),
                bare_commit(c, vec![b]),
            ],
            c,
            vec![(a, 1), (b, 1), (c, 1)],
        );
        let error = graph.validate().expect_err("被改过的深度缓存必须被拒绝");
        match error {
            CommitGraphError::DepthInconsistent {
                commit,
                declared,
                derived,
            } => {
                assert_eq!(commit, b, "先被查出的应当是链上第二个提交");
                assert_eq!(declared, 1);
                assert_eq!(derived, 2);
            }
            other => panic!("期望 DepthInconsistent，实测 {other}"),
        }
        // 对照：同一形状但深度自洽（1/2/3）必须通过。
        let sound = bare_graph(
            vec![
                bare_commit(a, Vec::new()),
                bare_commit(b, vec![a]),
                bare_commit(c, vec![b]),
            ],
            c,
            vec![(a, 1), (b, 2), (c, 3)],
        );
        sound.validate().expect("自洽的深度缓存必须通过");
    }
}
