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
        let id = draft.id;
        let snapshot_ref = Some(
            draft
                .snapshot_ref
                .unwrap_or_else(|| default_snapshot_hash(&draft.ops)),
        );
        self.commits.insert(
            id,
            Commit {
                id,
                parents: Vec::new(),
                branch_id: draft.branch_id.clone(),
                author: draft.author,
                message: draft.message,
                created_at: draft.created_at,
                rng_seed: draft.rng_seed,
                ops: draft.ops,
                snapshot_ref,
            },
        );
        self.depths.insert(id, 1);
        self.branches.insert(
            draft.branch_id.clone(),
            BranchHead {
                name: draft.branch_id,
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
                parents: vec![head],
                branch_id: draft.branch_id.clone(),
                author: draft.author,
                message: draft.message,
                created_at: draft.created_at,
                rng_seed: draft.rng_seed,
                ops: draft.ops,
                snapshot_ref,
            },
        );
        self.depths.insert(id, depth);
        if let Some(branch) = self.branches.get_mut(&draft.branch_id) {
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
                parents: vec![*from],
                branch_id: branch_name.clone(),
                author: draft.author,
                message: draft.message,
                created_at: draft.created_at,
                rng_seed: draft.rng_seed,
                ops: draft.ops,
                snapshot_ref,
            },
        );
        self.depths.insert(id, depth);
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
}
