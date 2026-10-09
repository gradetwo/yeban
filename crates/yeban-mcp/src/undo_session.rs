//! 撤销会话 —— **UI 与 MCP 共用的唯一实现** [ADR-0001 **D45**, `ARCH-OPS-001/002`, `MODEL-ISO-001`]。
//!
//! ## 为什么是一个文件被两侧同时编译
//!
//! D45 的原文是"撤销入口 **UI+MCP 两侧同接，共用同一实现**"，并且明确
//! "**不允许** UI 与 MCP 各写一份撤销逻辑（那会立刻产生第二份语义）"。
//! 本仓库里"同一份实现被两个 crate 用"的既有手法是 `#[path]` 引入**同一份源码**
//! （先例：`crates/yeban-render/verify/pure_modules.rs`、`crates/yeban-mcp/verify/section_pure.rs`、
//! `crates/yeban-app/tests/real_ui_tier1.rs`）。本文件沿用同一手法：
//!
//! | 侧 | 引入方式 | 落点 |
//! | :--- | :--- | :--- |
//! | MCP | `crates/yeban-mcp/src/lib.rs` 的 `pub mod undo_session;` | `domain::plan/apply` 的 `yeban_undo` / `yeban_redo` |
//! | UI | `crates/yeban-app/src/undo.rs` 的 `#[path = "../../yeban-mcp/src/undo_session.rs"]` | `main.rs` 的 Cmd+Z / 时光机按钮 |
//!
//! 于是"只有一份撤销实现"是**文件级事实**，不是纪律：两处 `grep` 到的是同一个文件、
//! 同一个函数、同一串字节。判据 [`tests::no_second_undo_implementation_exists_in_the_workspace`]
//! 把这件事机械钉住（它扫描生产源码，任何自写的反向应用循环都变红）。
//!
//! ## 依赖面刻意只有 `std` + `yeban_model`
//!
//! 两侧都**直接**依赖 `yeban-model`（MCP 的 `Cargo.toml` 与 app 的 `Cargo.toml` 都写了它），
//! 因此本文件不需要任何新依赖边、不动根 `Cargo.toml` / `Cargo.lock`（零新增依赖）。
//! 这也是它**不**碰 `serde_json` 的原因：`yeban-app` 没有 `serde_json` 依赖，
//! 所以"工程字节"的取法只能是模型自己的确定性容器写出
//! （[`canonical_project_bytes`]，固定 DOS 时间戳 ⇒ 逐字节稳定）。
//!
//! ## 撤销的**唯一**执行者仍然是模型
//!
//! 本文件**不构造**任何逆操作，也**不写**任何反向应用循环：
//!
//! - 撤销：[`CommitGraph::undo_with`]（模型内 `op.apply_inverse(doc)`）；
//! - 重做：把 op 按**正向** [Op::apply] 再打一次（模型自己的正向应用，不是第二份撤销）。
//!
//! 本文件只负责**会话语义**——三件事，且三件事都在模型之外无法表达：
//!
//! 1. **游标**（[`UndoCursor`]）：`Cmd+Z` 一次撤一步、连按继续往前。游标由调用方保存
//!    （模型 `undo_with` 的签名就是这么定的）；
//! 2. **活跃分支**：撤销之后继续编辑时，按模型既有的 [`CommitGraph::fork_anonymous`]
//!    在**撤销位置**派生匿名分支 —— 原分支头作为只读孤岛保全，被撤销的那几条 op
//!    因此**不可能**被再撤一次（模型文档：`ARCH-OPS-002` 的"撤销后继续编辑"）；
//! 3. **拒绝**：没有历史可撤 / 没有步骤可重做 / 撤销位置不在提交边界上 ⇒ 明确拒绝，
//!    并且**一个字节都不改**。
//!
//! ## 会话运行态绝不落盘 [MODEL-ISO-001]
//!
//! [`UndoCursor`] 刻意**不实现** `Serialize`/`Deserialize`（模型的设计），
//! [`UndoState`] 因此也不可能被写进 `YebanProjectV1` 或 `project.json`。
//! 可机械验证的推论有两条（判据钉住）：
//!
//! - 撤销前后 `history.dag` 的**字节完全相同**（撤销不动图谱，只动文档与游标）；
//! - 撤销后的 `project.json` 里既没有"游标"，也没有任何新键 ——
//!   它的键集合与"历史上的那一版工程"逐键相同。
//!
//! ## 一次提交 = 一条 op = **一步**撤销
//!
//! [`commit`] 把一批 op 包成一个 [`Op::Batch`]，整个批**原子**地施加（模型 `Op::Batch::commit`
//! 在克隆体上试跑），并且只产生一条 [`StampedOp`] ⇒ `yeban_propose_section` 产出的那批
//! （章节 + 摆放 + 声部连接）算**一步**，不是 N 步。
//!
//! ## 多父合并提交：[`commit_merge`]
//!
//! [`commit_merge`] 与 [`commit`] 是**同一个实现体**（私有 [`commit_with`]），只差一个
//! "额外父集合"参数。它把 `yeban-model` 的 [`CommitGraph::append_merge`] 用于
//! "提案分支 → 主分支"的合并，因此**合并这件事写进图谱**，而不是只写在调用方的记录里。
//! 主干的形状不变：`append_merge` 的第一父恒为当前分支头，
//! [`CommitGraph::ancestry`] 与跨提交撤销仍然只走主干。
//!
//! ## 判据怎么用本文件
//!
//! 本文件的 `#[cfg(test)]` 会**在两个 crate 里各跑一遍**（这正是"同一份实现"的证据）。
//! 两侧入口各自的端到端判据住在：
//! `crates/yeban-mcp/tests/undo_wiring.rs`（工具面）与 `crates/yeban-app/src/undo.rs` 的判据
//! + `crates/yeban-app/tests/undo_wiring_ui.rs`（界面面）。

use std::fmt;
use std::path::PathBuf;

use yeban_model::{
    AssetHash, CommitDraft, CommitGraph, EntityId, Op, OpOrigin, StampedOp, UndoCursor,
    YebanProjectV1, container,
};

/// 主分支名（与 `yeban-model` / `domain::MAIN_BRANCH` 同一个字面量，只此一处）。
pub const MAIN_BRANCH: &str = "main";

/// 撤销类操作最多一次撤多少步（防止 `steps: u32::MAX` 把会话拖死）。
pub const MAX_STEPS: usize = 10_000;

// ---------------------------------------------------------------------------
// 拒绝
// ---------------------------------------------------------------------------

/// 一次撤销 / 重做 / 提交**被拒绝**的原因。
///
/// 刻意**不**在这里写 "错误码"：错误码是 MCP 契约（`schemas/mcp-tools.schema.json`
/// 的 `ToolResponse.error.code`，ADR-0001 **D25** 的联集 20 值）的词汇，映射住在
/// [`crate::domain`]（契约侧）。界面侧只需要一句人话（[`fmt::Display`]）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoRefusal {
    /// 活跃分支在提交图谱里不存在（图谱未初始化 / 被外部改坏）。
    NoBranch {
        /// 找不到的分支名。
        branch: String,
    },
    /// 没有可撤销的历史（一次都没提交过，或已经撤到底）。
    NoHistory {
        /// 当前**可撤销**的步数（恒为 0；带上它是为了让人看清"确实撤到底了"）。
        undoable: usize,
    },
    /// 没有可重做的步骤（游标已经在头上）。
    NoRedo,
    /// 撤销位置**不在**提交边界上 —— 本实现保证每次提交恰好一条 op，因此这是
    /// 内部不变量被破坏（图谱不是本会话写出来的），按拒绝处理而不是造一个错的提交。
    NotAtCommitBoundary {
        /// 已撤销的步数（= 撤销位置）。
        undone: usize,
    },
    /// 模型层拒绝：逆操作前置条件不成立、图谱损坏等。**工程保持原样**。
    Model {
        /// 模型错误的原文（`ModelError` 的 `Display`）。
        detail: String,
    },
    /// 容器写出失败（只有 [`canonical_project_bytes`] / [`UndoSession::project_bytes`] 会产出）。
    Serialization {
        /// 容器错误的原文。
        detail: String,
    },
}

impl fmt::Display for UndoRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBranch { branch } => {
                write!(f, "提交图谱里没有分支 `{branch}`（尚未初始化）")
            }
            Self::NoHistory { undoable } => {
                write!(f, "没有可撤销的历史（可撤销步数 = {undoable}）")
            }
            Self::NoRedo => f.write_str("没有可重做的步骤"),
            Self::NotAtCommitBoundary { undone } => write!(
                f,
                "撤销位置 {undone} 不落在提交边界上 —— 图谱不是本会话写出来的（每条提交恰好一条 op）"
            ),
            Self::Model { detail } => write!(f, "模型层拒绝: {detail}"),
            Self::Serialization { detail } => write!(f, "工程容器写出失败: {detail}"),
        }
    }
}

impl std::error::Error for UndoRefusal {}

// ---------------------------------------------------------------------------
// 会话态
// ---------------------------------------------------------------------------

/// 撤销会话的**运行态**：一条游标 + 一个活跃分支名 [MODEL-ISO-001]。
///
/// 两者都**不进** `.yeban`：游标是模型刻意不实现 `Serialize` 的 [`UndoCursor`]，
/// 分支名在图谱里（`history.dag` 的 `branches`）本来就有一份权威记录 ——
/// 本结构记的只是"**当前活跃**的是哪一条"。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndoState {
    cursor: UndoCursor,
    branch: String,
    author: String,
}

impl UndoState {
    /// 全新会话（游标在头上，活跃分支 `main`）。
    #[must_use]
    pub fn new(author: impl Into<String>) -> Self {
        Self {
            cursor: UndoCursor::new(),
            branch: MAIN_BRANCH.to_owned(),
            author: author.into(),
        }
    }

    /// 当前活跃分支名。
    #[must_use]
    pub fn branch(&self) -> &str {
        &self.branch
    }

    /// 已撤销的步数（= 可重做的步数）。
    #[must_use]
    pub const fn undone(&self) -> usize {
        self.cursor.undone()
    }

    /// 模型游标（只读）。
    #[must_use]
    pub const fn cursor(&self) -> UndoCursor {
        self.cursor
    }

    /// 提交作者（写进 [`CommitDraft::author`]）。
    #[must_use]
    pub fn author(&self) -> &str {
        &self.author
    }

    /// 活跃分支头。**模型读数**：查的是提交图谱的分支表。
    ///
    /// # Errors
    ///
    /// 分支不存在 → [`UndoRefusal::NoBranch`]。
    pub fn head(&self, graph: &CommitGraph) -> Result<EntityId, UndoRefusal> {
        graph
            .branch_head(&self.branch)
            .map(|branch| branch.head)
            .map_err(|_| UndoRefusal::NoBranch {
                branch: self.branch.clone(),
            })
    }

    /// 从（**文档**, **图谱**）**推导**游标：头上已经有几步是"没被应用"的。
    ///
    /// ## 为什么需要它（而不是把游标写进工程）
    ///
    /// 游标是会话运行态，**不许**落盘 [MODEL-ISO-001]。但"撤销 → 保存 → 退出 →
    /// 重开"是真实用法：文档停在被撤销后的状态，而 `history.dag` 里那条提交还在。
    /// 不处理的话，重开之后按 `Cmd+Z` 会去撤销一条**本来就没应用**的 op ——
    /// 模型会如实拒绝（`OpStateMismatch`），用户看到的是"撤销坏了"。
    ///
    /// 处置不是把游标序列化，而是**推导**：从头上逐条试
    /// [`Op::invert`]（只读：构造逆操作 + 校验它在当前文档上前置条件成立），
    /// 第一个"已应用"的 op 之前就是被撤销的部分。这也顺带证明了一件事：
    /// 游标**可恢复**并不需要被持久化 —— 它是 (`文档`, `图谱`) 的纯函数。
    ///
    /// 返回推导出的已撤销步数。
    ///
    /// # Errors
    ///
    /// 分支不存在或图谱损坏 → 对应的 [`UndoRefusal`]。
    pub fn align_with(
        &mut self,
        graph: &CommitGraph,
        project: &YebanProjectV1,
    ) -> Result<usize, UndoRefusal> {
        let mut cursor = Some(self.head(graph)?);
        let mut skip = 0_usize;
        let mut guard = graph.commit_count() + 1;
        'chain: while let Some(id) = cursor {
            if guard == 0 {
                return Err(UndoRefusal::Model {
                    detail: format!("分支 `{}` 的父链出现环", self.branch),
                });
            }
            guard -= 1;
            let commit = graph.commit(&id).map_err(model_refusal)?;
            for stamped in commit.ops.iter().rev() {
                if op_is_applied(&stamped.op, project) {
                    break 'chain;
                }
                skip += 1;
            }
            cursor = commit.first_parent();
        }
        self.cursor.skip = skip;
        Ok(skip)
    }

    /// 活跃分支上**全部**可回退的 op 数（模型读数：沿 `first_parent` 走到根）。
    ///
    /// 这是"撤销栈深度"的分母；分子是 [`Self::undone`]。
    ///
    /// # Errors
    ///
    /// 分支不存在或图谱损坏 → 对应的 [`UndoRefusal`]。
    pub fn reachable_ops(&self, graph: &CommitGraph) -> Result<usize, UndoRefusal> {
        let mut total = 0_usize;
        let mut cursor = Some(self.head(graph)?);
        // 防御：正常图谱是 DAG（父链必然终止）。上限用提交总数，超了就是损坏。
        let mut guard = graph.commit_count() + 1;
        while let Some(id) = cursor {
            if guard == 0 {
                return Err(UndoRefusal::Model {
                    detail: format!("分支 `{}` 的父链出现环", self.branch),
                });
            }
            guard -= 1;
            let commit = graph.commit(&id).map_err(model_refusal)?;
            total += commit.ops.len();
            cursor = commit.first_parent();
        }
        Ok(total)
    }

    /// 还能撤销多少步（**模型读数**减去已撤销步数）。
    ///
    /// # Errors
    ///
    /// 同 [`Self::reachable_ops`]。
    pub fn undoable(&self, graph: &CommitGraph) -> Result<usize, UndoRefusal> {
        Ok(self.reachable_ops(graph)?.saturating_sub(self.cursor.skip))
    }

    /// `true` = 现在按 `Cmd+Z` 会真的改动工程。
    #[must_use]
    pub fn can_undo(&self, graph: &CommitGraph) -> bool {
        self.undoable(graph).is_ok_and(|steps| steps > 0)
    }

    /// `true` = 现在按 `Cmd+Shift+Z` 会真的改动工程。
    #[must_use]
    pub const fn can_redo(&self) -> bool {
        self.cursor.skip > 0
    }

    /// 界面 / 工具响应共用的**显示态**（全部字段来自模型读数 + 游标）。
    #[must_use]
    pub fn display(&self, graph: &CommitGraph) -> UndoDisplay {
        let head = self.head(graph).ok();
        UndoDisplay {
            branch: self.branch.clone(),
            head,
            commit_count: graph.commit_count(),
            branch_count: graph.branch_count(),
            undone: self.cursor.skip,
            undoable: self.undoable(graph).unwrap_or(0),
            can_undo: self.can_undo(graph),
            can_redo: self.can_redo(),
        }
    }
}

impl Default for UndoState {
    fn default() -> Self {
        Self::new("yeban")
    }
}

/// 撤销能力的**显示态**：界面与工具响应都从这里取数，谁都不许自己算。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndoDisplay {
    /// 活跃分支名。
    pub branch: String,
    /// 活跃分支头（图谱未初始化时为 `None`）。
    pub head: Option<EntityId>,
    /// 提交总数（模型读数）。
    pub commit_count: usize,
    /// 分支总数（含匿名分支与提案分支）。
    pub branch_count: usize,
    /// 已撤销的步数（= 可重做步数）。
    pub undone: usize,
    /// 还能撤销的步数。
    pub undoable: usize,
    /// 现在按 `Cmd+Z` 是否有效。
    pub can_undo: bool,
    /// 现在按 `Cmd+Shift+Z` 是否有效。
    pub can_redo: bool,
}

/// 一次撤销 / 重做的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndoOutcome {
    /// **实际**执行的步数（可能小于请求值：撤到底就停）。
    pub steps: usize,
    /// 执行后已撤销的总步数。
    pub undone_total: usize,
    /// 被处理的 op 变体名（`Op::name()`，顺序 = 处理顺序）。
    pub op_kinds: Vec<&'static str>,
}

impl UndoOutcome {
    /// `true` = 工程真的被改动了。
    #[must_use]
    pub const fn changed(&self) -> bool {
        self.steps > 0
    }
}

/// 一次提交的请求（把 6 个参数收成一个结构体：参数一多就是"提交这件事有太多形态"的信号）。
#[derive(Clone, Debug)]
pub struct CommitRequest {
    /// Unix 毫秒（模型不自取时钟，由调用方注入）。
    pub now_ms: u64,
    /// 操作来源（UI = `OpOrigin::UserUi`，MCP 合并 = `OpOrigin::McpProposal{..}`）。
    pub origin: OpOrigin,
    /// 提交信息（同时作为 `Op::Batch` 的 `description`）。
    pub message: String,
    /// 本次提交携带的 op（**整批算一步**）。
    pub ops: Vec<Op>,
}

/// 模型错误 → 拒绝（`Display` 原文保留，便于判据与人类读）。
fn model_refusal(error: yeban_model::ModelError) -> UndoRefusal {
    UndoRefusal::Model {
        detail: error.to_string(),
    }
}

/// 这条 op **是否已经作用在** `doc` 上（只读判定）。
///
/// 判据就是模型自己的口径：逆操作的**前置条件**成立 ⇔ 本操作已经生效
/// （见 [`Op::invert`] 的文档："`doc` 用于校验'本操作确实已经作用在这份文档上'"）。
///
/// ⚠ `Op::Batch` 必须**递归到子 op**：模型对批次刻意跳过了逆操作的前置条件检查
/// （批次整体施加是原子的，因此单看批次永远"可逆"）。不递归的话，
/// 撤销位置推导会把"整批被撤掉"误判成"整批已应用"。
fn op_is_applied(op: &Op, doc: &YebanProjectV1) -> bool {
    match op {
        Op::Batch { ops, .. } => ops.iter().all(|inner| op_is_applied(inner, doc)),
        other => other.invert(doc).is_ok(),
    }
}

// ---------------------------------------------------------------------------
// 会话动作：提交 / 撤销 / 重做
// ---------------------------------------------------------------------------

/// 用一条根提交**开启**一个会话（图谱必须为空）。
///
/// # Errors
///
/// 图谱非空、或模型拒绝这次根提交。
pub fn genesis(
    graph: &mut CommitGraph,
    state: &mut UndoState,
    now_ms: u64,
    message: &str,
) -> Result<EntityId, UndoRefusal> {
    let draft = CommitDraft::new(
        EntityId::new(),
        MAIN_BRANCH,
        state.author.clone(),
        message.to_owned(),
    )
    .with_created_at(now_ms);
    let id = graph.genesis(draft).map_err(model_refusal)?;
    state.cursor = UndoCursor::new();
    state.branch = MAIN_BRANCH.to_owned();
    Ok(id)
}

/// 提交一批 op：**一次提交 = 一条 `StampedOp`（`Op::Batch`）= 一步撤销**。
///
/// 顺序是刻意的，目的是让"工程里有 op、日志里没有"这种最危险的状态**不可能**出现：
///
/// 1. 先在**克隆体**上施加整批（`Op::Batch` 自己也是原子试跑）⇒ 失败时原工程一位不变，
///    而且本函数**不需要**回滚 ⇒ 全仓库的反向应用只剩 `yeban_model` 那一处；
/// 2. 再写提交（可能按 [`CommitGraph::fork_anonymous`] 派生匿名分支）；
/// 3. 写提交成功之后，才把克隆体换成权威工程。
///
/// 若当前有已撤销的步骤，则在**撤销位置**派生匿名分支
/// （原分支头保留为只读孤岛）⇒ 被撤销的 op 不可能被再撤一次。
///
/// # Errors
///
/// 分支不存在、撤销位置不在提交边界上、op 前置条件不成立、图谱写入失败。
pub fn commit(
    graph: &mut CommitGraph,
    project: &mut YebanProjectV1,
    state: &mut UndoState,
    request: CommitRequest,
) -> Result<EntityId, UndoRefusal> {
    commit_with(graph, project, state, request, &[])
}

/// 同 [`commit`]，但把这次提交写成**多父合并提交**（`merge_parents` 是额外的父提交）。
///
/// [`CommitGraph::append_merge`] 的语义照搬过来，一字不改：父集合 =
/// `[当前分支头] ++ merge_parents`，**第一父恒为当前分支头**。因此：
///
/// - `CommitGraph::ancestry` 与跨提交撤销走的仍是同一条主干（被合并那一侧的提交
///   **不**出现在主干的祖先链上）⇒ "一次 `Cmd+Z` 回退整套被合并的 op"不变；
/// - 合并关系由**图谱本身**承担，而不是只写在调用方（MCP 的 `Proposal` 记录）里。
///
/// ## `merge_parents` 什么时候**不**生效
///
/// 当前有已撤销的步骤时（`state.cursor.skip > 0`），提交必须落在**撤销位置**上并派生
/// 匿名分支（见 [`commit`] 的第 2 步）—— 那是 [`CommitGraph::fork_anonymous`]，它没有
/// "额外父"这个参数。此时 `merge_parents` 被**忽略**，写出的提交是**单父**的。
///
/// 这一点是刻意**不**静默的：调用方必须从图谱里**读回**这条提交的真实父集合再上报，
/// 而不是把自己请求的父集合当成事实（`crate::domain` 的 `yeban_merge_proposal` 就是这么做的）。
///
/// # Errors
///
/// 与 [`commit`] 相同，外加 [`CommitGraph::append_merge`] 的拒绝（父集合内有重复、
/// 某个父不是已知提交）。
pub fn commit_merge(
    graph: &mut CommitGraph,
    project: &mut YebanProjectV1,
    state: &mut UndoState,
    request: CommitRequest,
    merge_parents: &[EntityId],
) -> Result<EntityId, UndoRefusal> {
    commit_with(graph, project, state, request, merge_parents)
}

/// [`commit`] 与 [`commit_merge`] 的**唯一**实现体（两者只在"额外父集合"上不同）。
fn commit_with(
    graph: &mut CommitGraph,
    project: &mut YebanProjectV1,
    state: &mut UndoState,
    request: CommitRequest,
    merge_parents: &[EntityId],
) -> Result<EntityId, UndoRefusal> {
    let batch = Op::Batch {
        ops: request.ops,
        description: request.message.clone(),
    };
    let stamped = vec![StampedOp::new(
        request.origin,
        request.now_ms,
        batch.clone(),
    )];
    // 1) 在克隆体上施加。
    let mut staged = project.clone();
    batch.apply(&mut staged).map_err(model_refusal)?;
    // 2) 写提交（可能派生匿名分支）。
    let fork_from = if state.cursor.skip > 0 {
        match fork_point(graph, state)? {
            Some(from) => Some(from),
            None => {
                return Err(UndoRefusal::NotAtCommitBoundary {
                    undone: state.cursor.skip,
                });
            }
        }
    } else {
        None
    };
    let draft = CommitDraft::new(
        EntityId::new(),
        state.branch.clone(),
        state.author.clone(),
        request.message,
    )
    .with_created_at(request.now_ms)
    .with_ops(stamped);
    // `fork_anonymous` 会用 `anon-<新提交 ULID>` 命名分支并**忽略**草稿里的分支名，
    // 因此这里把几种写法的返回值归一成"（可选的新分支名, 提交身份）"。
    //
    // ⚠ 分支顺序即语义：有已撤销的步骤时 `fork_from` 恒为 `Some`，此时多父合并**写不出来**
    // （`fork_anonymous` 没有额外父参数）⇒ `merge_parents` 被忽略，写出的是单父提交。
    // 调用方必须从图谱读回真实父集合，见 [`commit_merge`] 的文档。
    let written = match fork_from {
        Some(from) => graph
            .fork_anonymous(&from, draft)
            .map(|(branch, id)| (Some(branch), id)),
        None if graph.commit_count() == 0 => graph.genesis(draft).map(|id| (None, id)),
        None if !merge_parents.is_empty() => graph
            .append_merge(draft, merge_parents)
            .map(|id| (None, id)),
        None => graph.append(draft).map(|id| (None, id)),
    };
    // 3) 提交写成之后才替换权威工程。
    let (new_branch, id) = written.map_err(model_refusal)?;
    if let Some(branch) = new_branch {
        state.branch = branch;
    }
    state.cursor = UndoCursor::new();
    *project = staged;
    Ok(id)
}

/// "这次撤销到底会做几步"的**唯一**口径（真做与 `dryRun` 预览共用）。
///
/// 返回 `(活跃分支头, 实际步数)`。步数被夹到 `1..=MAX_STEPS` 再与可撤销深度取小值。
fn plan_undo(
    graph: &CommitGraph,
    state: &UndoState,
    steps: usize,
) -> Result<(EntityId, usize), UndoRefusal> {
    let head = state.head(graph)?;
    let available = state.undoable(graph)?;
    let steps = steps.clamp(1, MAX_STEPS).min(available);
    if steps == 0 {
        return Err(UndoRefusal::NoHistory { undoable: 0 });
    }
    Ok((head, steps))
}

/// "这次重做到底会做几步"的**唯一**口径。
fn plan_redo(
    graph: &CommitGraph,
    state: &UndoState,
    steps: usize,
) -> Result<(EntityId, usize), UndoRefusal> {
    let head = state.head(graph)?;
    let steps = steps.clamp(1, MAX_STEPS).min(state.cursor.skip);
    if steps == 0 {
        return Err(UndoRefusal::NoRedo);
    }
    Ok((head, steps))
}

/// 将要被撤销的 op 变体名（**只读**；顺序 = 处理顺序）。
///
/// # Errors
///
/// 没有可撤销的历史 ⇒ [`UndoRefusal::NoHistory`]。
pub fn undo_op_kinds(
    graph: &CommitGraph,
    state: &UndoState,
    steps: usize,
) -> Result<Vec<&'static str>, UndoRefusal> {
    let (head, steps) = plan_undo(graph, state, steps)?;
    Ok(graph
        .ops_backwards_from(&head, state.cursor.skip, steps)
        .map_err(model_refusal)?
        .iter()
        .map(|stamped| stamped.op.name())
        .collect())
}

/// 将要被重做的 op 变体名（**只读**；顺序 = 处理顺序）。
///
/// # Errors
///
/// 没有可重做的步骤 ⇒ [`UndoRefusal::NoRedo`]。
pub fn redo_op_kinds(
    graph: &CommitGraph,
    state: &UndoState,
    steps: usize,
) -> Result<Vec<&'static str>, UndoRefusal> {
    let (head, steps) = plan_redo(graph, state, steps)?;
    let mut kinds: Vec<&'static str> = Vec::with_capacity(steps);
    for offset in 0..steps {
        let index = state.cursor.skip - 1 - offset;
        let found = graph
            .ops_backwards_from(&head, index, 1)
            .map_err(model_refusal)?;
        match found.first() {
            Some(stamped) => kinds.push(stamped.op.name()),
            None => return Err(UndoRefusal::NoRedo),
        }
    }
    Ok(kinds)
}

/// `dryRun` 预览：在**克隆体**上算出"撤销 `steps` 步之后的工程"（真状态一位不动）。
///
/// 它调用的就是 [`undo`] 本身（同一个函数、同一串字节），因此"预览说的"与"真做的"
/// 不可能漂移 —— 判据 `dry_run_preview_matches_the_real_undo_byte_for_byte` 钉住这一点。
///
/// # Errors
///
/// 同 [`undo`]。
pub fn simulate_undo(
    graph: &CommitGraph,
    project: &YebanProjectV1,
    state: &UndoState,
    steps: usize,
) -> Result<YebanProjectV1, UndoRefusal> {
    let mut simulated = project.clone();
    let mut state = state.clone();
    undo(graph, &mut simulated, &mut state, steps)?;
    Ok(simulated)
}

/// `dryRun` 预览：在**克隆体**上算出"重做 `steps` 步之后的工程"。
///
/// # Errors
///
/// 同 [`redo`]。
pub fn simulate_redo(
    graph: &CommitGraph,
    project: &YebanProjectV1,
    state: &UndoState,
    steps: usize,
) -> Result<YebanProjectV1, UndoRefusal> {
    let mut simulated = project.clone();
    let mut state = state.clone();
    redo(graph, &mut simulated, &mut state, steps)?;
    Ok(simulated)
}

/// 撤销 `steps` 步（一次 `Cmd+Z` 传 1）。
///
/// - 步数超过可撤销深度 ⇒ 撤到底并如实返回**实际**步数；
/// - 一步都撤不动 ⇒ [`UndoRefusal::NoHistory`]，**工程与游标都不动**；
/// - 逆操作由模型 [`CommitGraph::undo_with`] 施加，本函数不自己构造逆操作。
///
/// # Errors
///
/// 见 [`UndoRefusal`]。
pub fn undo(
    graph: &CommitGraph,
    project: &mut YebanProjectV1,
    state: &mut UndoState,
    steps: usize,
) -> Result<UndoOutcome, UndoRefusal> {
    let (head, steps) = plan_undo(graph, state, steps)?;
    // 将要被撤销的 op（只读预览；真正的执行在下一行的模型入口里）。
    let op_kinds = undo_op_kinds(graph, state, steps)?;
    let applied = graph
        .undo_with(project, &head, &mut state.cursor, steps)
        .map_err(model_refusal)?;
    Ok(UndoOutcome {
        steps: applied,
        undone_total: state.cursor.skip,
        op_kinds,
    })
}

/// 重做 `steps` 步（一次 `Cmd+Shift+Z` 传 1）。
///
/// 重做 = 把**刚刚被撤销的那条 op** 按**正向** [`Op::apply`] 再打一次 ——
/// 正向应用是模型自己的入口，本函数依然不写逆操作。
///
/// # Errors
///
/// 没有可重做的步骤 ⇒ [`UndoRefusal::NoRedo`]（工程与游标都不动）。
pub fn redo(
    graph: &CommitGraph,
    project: &mut YebanProjectV1,
    state: &mut UndoState,
    steps: usize,
) -> Result<UndoOutcome, UndoRefusal> {
    let (head, steps) = plan_redo(graph, state, steps)?;
    let op_kinds = redo_op_kinds(graph, state, steps)?;
    for _ in 0..steps {
        let index = state.cursor.skip - 1;
        let found = graph
            .ops_backwards_from(&head, index, 1)
            .map_err(model_refusal)?;
        match found.first() {
            Some(stamped) => stamped.op.apply(project).map_err(model_refusal)?,
            None => return Err(UndoRefusal::NoRedo),
        }
        state.cursor.skip -= 1;
    }
    Ok(UndoOutcome {
        steps,
        undone_total: state.cursor.skip,
        op_kinds,
    })
}

/// 撤销位置对应的**提交**（`None` = 位置落在某条提交内部）。
///
/// 记 `k = cursor.skip`（已撤销的步数）、`newer(c)` = **比 `c` 新**的提交携带的 op 总数。
/// 撤销 `k` 步之后，文档状态 = "所有 op 减去最新的 `k` 条" ⇒ 只有当 `newer(c) == k`
/// 时，文档状态才恰好等于"提交 `c` 的状态"（`c` 及其祖先的 op 全在、更新的全不在）。
/// 因此上溯时**先判后加**：命中 `newer == k` 的提交就是派生点。
///
/// 本实现保证每次提交恰好一条 op，因此正常路径上**必然**命中
/// （撤销到底时会命中那条 0 op 的根提交）。
fn fork_point(graph: &CommitGraph, state: &UndoState) -> Result<Option<EntityId>, UndoRefusal> {
    let target = state.cursor.skip;
    if target == 0 {
        return Ok(None);
    }
    let mut newer = 0_usize;
    let mut cursor = Some(state.head(graph)?);
    let mut guard = graph.commit_count() + 1;
    while let Some(id) = cursor {
        if guard == 0 {
            return Err(UndoRefusal::Model {
                detail: format!("分支 `{}` 的父链出现环", state.branch),
            });
        }
        guard -= 1;
        if newer == target {
            return Ok(Some(id));
        }
        let commit = graph.commit(&id).map_err(model_refusal)?;
        newer += commit.ops.len();
        if newer > target {
            return Ok(None);
        }
        cursor = commit.first_parent();
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// 会话（拥有工程 + 图谱 + 会话态）
// ---------------------------------------------------------------------------

/// **界面侧**用的完整会话：工程 + 提交图谱 + 会话态 + 打开来源。
///
/// MCP 侧的 `Domain` 不持有它（工程住在 `Active` 里、图谱住在 `Domain` 上，
/// 两者与锁/资产池的生命周期绑在一起），而是直接调用本模块的**自由函数**
/// （[`genesis`] / [`commit`] / [`undo`] / [`redo`]）——
/// 因此会话语义仍然只有一份，不存在"Domain 那一套"与"Session 那一套"。
#[derive(Clone, Debug)]
pub struct UndoSession {
    project: YebanProjectV1,
    graph: CommitGraph,
    state: UndoState,
    label: String,
}

impl UndoSession {
    /// 打开一个工程（**新会话**：撤销不跨越打开边界）。图谱从一条根提交开始。
    ///
    /// # Errors
    ///
    /// 根提交被模型拒绝。
    pub fn open(
        label: impl Into<String>,
        author: &str,
        project: YebanProjectV1,
        now_ms: u64,
    ) -> Result<Self, UndoRefusal> {
        let label = label.into();
        let mut graph = CommitGraph::new();
        let mut state = UndoState::new(author);
        genesis(&mut graph, &mut state, now_ms, &format!("open {label}"))?;
        Ok(Self {
            project,
            graph,
            state,
            label,
        })
    }

    /// 打开一个工程并**采用**容器里恢复出来的提交图谱（`history.dag`）。
    ///
    /// 空图谱等价于 [`Self::open`]。图谱必须带 `main` 分支，否则拒绝
    /// （宁可承认"读不懂"，也不假装有一条可撤销的历史）。
    ///
    /// 游标按 [`UndoState::align_with`] 从（文档, 图谱）**推导**：
    /// "撤销 → 保存 → 重开"之后，重做栈因此仍然是对的，而游标一个字节都没落盘。
    ///
    /// # Errors
    ///
    /// 图谱非空却没有 `main` 分支。
    pub fn open_with_graph(
        label: impl Into<String>,
        author: &str,
        project: YebanProjectV1,
        graph: CommitGraph,
        now_ms: u64,
    ) -> Result<Self, UndoRefusal> {
        if graph.commit_count() == 0 {
            return Self::open(label, author, project, now_ms);
        }
        let mut session = Self {
            project,
            graph,
            state: UndoState::new(author),
            label: label.into(),
        };
        session.state.align_with(&session.graph, &session.project)?;
        Ok(session)
    }

    /// 权威工程（只读）。
    #[must_use]
    pub const fn project(&self) -> &YebanProjectV1 {
        &self.project
    }

    /// 提交图谱（只读）。
    #[must_use]
    pub const fn graph(&self) -> &CommitGraph {
        &self.graph
    }

    /// 会话态（只读）。
    #[must_use]
    pub const fn state(&self) -> &UndoState {
        &self.state
    }

    /// 打开来源（路径 / 样本名）。
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// 显示态（模型读数）。
    #[must_use]
    pub fn display(&self) -> UndoDisplay {
        self.state.display(&self.graph)
    }

    /// 提交一批 op（见自由函数 [`commit`]）。
    ///
    /// # Errors
    ///
    /// 见 [`commit`]。
    pub fn commit(&mut self, request: CommitRequest) -> Result<EntityId, UndoRefusal> {
        commit(&mut self.graph, &mut self.project, &mut self.state, request)
    }

    /// 撤销（见自由函数 [`undo`]）。
    ///
    /// # Errors
    ///
    /// 见 [`undo`]。
    pub fn undo_steps(&mut self, steps: usize) -> Result<UndoOutcome, UndoRefusal> {
        undo(&self.graph, &mut self.project, &mut self.state, steps)
    }

    /// 重做（见自由函数 [`redo`]）。
    ///
    /// # Errors
    ///
    /// 见 [`redo`]。
    pub fn redo_steps(&mut self, steps: usize) -> Result<UndoOutcome, UndoRefusal> {
        redo(&self.graph, &mut self.project, &mut self.state, steps)
    }

    /// 工程内容的**确定性容器字节**（判据的逐字节指纹）。
    ///
    /// # Errors
    ///
    /// 容器写出失败。
    pub fn project_bytes(&self) -> Result<Vec<u8>, UndoRefusal> {
        canonical_project_bytes(&self.project)
    }

    /// 工程内容的 SHA-256（十六进制小写）——比整段字节更好放进日志与断言。
    ///
    /// # Errors
    ///
    /// 同 [`Self::project_bytes`]。
    pub fn fingerprint(&self) -> Result<String, UndoRefusal> {
        Ok(AssetHash::of_bytes(&self.project_bytes()?).to_string())
    }

    /// **判据专用**的可变工程入口：模拟"有人绕开 Ops Log 直接改了工程"，
    /// 用来证明逆操作前置条件不成立时撤销会**拒绝**并且不动工程。
    ///
    /// 只在测试构建里存在（`#[cfg(test)]`），生产二进制里没有这个口子。
    #[cfg(test)]
    pub fn project_mut_for_tests(&mut self) -> &mut YebanProjectV1 {
        &mut self.project
    }
}

/// 工程 → **确定性**容器字节（`project.json` + 空 `history.dag` + 空资产池）。
///
/// 为什么用它当"逐字节"的标尺：容器写出用**固定 DOS 时间戳**（`1980-01-01`），
/// 因此同一份工程两次写出的字节完全相同 —— 它是 `serde_json::to_vec(工程)` 的一个
/// 确定性外壳，而且**不需要** `yeban-app` 引入 `serde_json`。
///
/// # Errors
///
/// 容器写出失败。
pub fn canonical_project_bytes(project: &YebanProjectV1) -> Result<Vec<u8>, UndoRefusal> {
    container::write_project_container(project, &[], &std::collections::BTreeMap::new()).map_err(
        |error| UndoRefusal::Serialization {
            detail: error.to_string(),
        },
    )
}

/// 工程内容的 SHA-256（十六进制小写）。
///
/// # Errors
///
/// 同 [`canonical_project_bytes`]。
pub fn project_fingerprint(project: &YebanProjectV1) -> Result<String, UndoRefusal> {
    Ok(AssetHash::of_bytes(&canonical_project_bytes(project)?).to_string())
}

// ---------------------------------------------------------------------------
// 判据夹具：两侧共用的**确定性编辑脚本**
// ---------------------------------------------------------------------------

/// 一次"改一个音符力度"的确定性编辑脚本（两侧接线判据共用）。
///
/// 为什么它住在**生产文件**里而不是某个 `tests/` 里：UI 路径与 MCP 路径必须撤
/// **同一串 op**，否则"两侧逐字节相同"就退化成"两个夹具恰好一致"。
/// 夹具放一处（本文件），MCP 侧用它构造 `yeban_edit_notes` 的实参，
/// 界面侧用它构造同一批 `Op` —— 于是两侧的起点、编辑、终点都同源。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WiringFixture {
    /// 承载该片段的音轨。
    pub track_id: EntityId,
    /// 片段池条目身份。
    pub clip_id: EntityId,
    /// 音符身份。
    pub note_id: EntityId,
    /// 修改前力度（来自工程本身）。
    pub old_velocity: u8,
    /// 修改后力度（判据固定值，避开边界值 0/127）。
    pub new_velocity: u8,
}

impl WiringFixture {
    /// 判据固定的目标力度。
    pub const TARGET_VELOCITY: u8 = 42;

    /// 脚本对应的 op（与 `yeban_edit_notes` 的 `velocity` 编译结果同形）。
    #[must_use]
    pub fn op(&self) -> Op {
        Op::ModifyNoteVelocity {
            track_id: self.track_id,
            clip_id: self.clip_id,
            note_id: self.note_id,
            old_vel: self.old_velocity,
            new_vel: self.new_velocity,
        }
    }
}

/// 从一份工程里挑出"有一个 MIDI 音符的音轨 / 片段 / 音符"，构造编辑脚本。
///
/// 选法完全确定（`BTreeMap` 迭代序 + 取第一个有音符的片段），因此两侧拿到同一组身份。
#[must_use]
pub fn wiring_fixture(project: &YebanProjectV1) -> Option<WiringFixture> {
    for track in project.tracks.values() {
        for placement in track.clips.values() {
            let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
                continue;
            };
            let Some(notes) = entry.content.notes() else {
                continue;
            };
            let Some(note) = notes.values().next() else {
                continue;
            };
            return Some(WiringFixture {
                track_id: track.id,
                clip_id: entry.id,
                note_id: note.id,
                old_velocity: note.velocity,
                new_velocity: WiringFixture::TARGET_VELOCITY,
            });
        }
    }
    None
}

// ---------------------------------------------------------------------------
// "只有一个撤销实现"的机械守卫
// ---------------------------------------------------------------------------

/// 一份待扫描的源码（路径 + 内容）。
pub type SourceFile = (String, String);

/// 这个路径是不是**本共享实现自己**（两个 crate、两个平台下都成立）。
///
/// ## 为什么必须平台无关（CI 抓出来的两个真红点）
///
/// Windows 的 `Path::display()` 用 `\` 作分隔符，Linux/macOS 用 `/`。任何
/// `path.ends_with("src/undo_session.rs")` 形态的判断在 Windows 上都会**漏判自己**：
/// 守卫于是把自己的两处生产命中（只读推导用的 `invert(`、唯一执行者 `.undo_with(`）
/// 当成"第二份撤销实现"（CI run 37268427827 的 windows 腿）。
///
/// ## 为什么只此一处口径
///
/// "这是不是共享实现"这个判断有三个调用点：扫描器自己、共享实现的判据、以及
/// `crates/yeban-mcp/tests/undo_wiring.rs` 的源码级判据。**三处都调本函数** ——
/// 第一版把它们各写了一遍，于是修好了一处、漏了另一处
/// （CI run 37268901708 的 windows 腿又红在同一条判据上）。
///
/// 判据：分隔符归一化之后，路径尾部恰好是 `crates/yeban-mcp/src/undo_session.rs`
/// （两个 crate 反推出来的路径都落在这一个尾部上），并且带一条**反向**回归 ——
/// 别的 crate 下的同名文件**不得**被当成自己（否则守卫会空转）。
#[must_use]
pub fn is_the_shared_session(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    // 归一化后仍可能是 `…/crates/yeban-mcp/../../crates/yeban-mcp/src/undo_session.rs`
    // （`production_source_roots` 会拼出 `../..`），`ends_with` 对此天然成立。
    normalized.ends_with("crates/yeban-mcp/src/undo_session.rs")
        || std::path::Path::new(path).ends_with(
            std::path::Path::new("crates")
                .join("yeban-mcp")
                .join("src")
                .join("undo_session.rs"),
        )
}

/// 取一份源码的**生产区**（`#[cfg(test)]` 属性**行**之前的全部内容）。
///
/// 判据必须只看生产代码：本仓库的判据都写在文件尾部的 `#[cfg(test)] mod tests` 里，
/// 而为了对账在测试里调一次 `apply_inverse` 是正当的。
/// ⚠ 用"整行以 `#[cfg(test)]` 开头"而不是 `find("#[cfg(test)]")`：后者会把**文档注释里
/// 提到这个词**的地方当成测试区起点（本文件与 `domain/mod.rs` 的文档都提到过）。
///
/// `pub`：`domain::automation_audit` / `domain::extension_audit` 各有一份**同口径**的
/// 拷贝（那两份要能被**裸 `rustc` 独立跑**，因此不能依赖 crate 内任何东西）；集成判据
/// `tests/extension_tools.rs::production_region_agrees_across_the_three_copies`
/// 断言三份实现在真实源码上逐字节相同 —— 于是"三份拷贝"不会变成三种口径。
/// （与 [`read_rust_sources`] / [`production_source_roots`] 一样，为判据而公开。）
pub fn production_region(text: &str) -> String {
    text.lines()
        .take_while(|line| !line.trim_start().starts_with("#[cfg(test)]"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 扫描生产源码里是否出现**第二份撤销实现**（纯函数，返回违规清单，空 = 干净）。
///
/// 判据是 grep 级 + 结构级：
///
/// 1. 除本文件外，任何**生产**代码里出现 `apply_inverse` / `structural_inverse` /
///    `invert(` ⇒ 有人在自写反向应用（那就是第二份撤销语义）；
/// 2. 除本文件外，任何生产代码里出现 `.undo_with(` / `.undo(` ⇒ 绕开共享实现直接调模型
///    （撤销的会话语义会分叉）。
///
/// `#[cfg(test)]` 之后的区域**不算生产代码**（本仓库的判据都写在文件尾部），
/// 因此测试里为了对账而调用 `apply_inverse` 不会被误伤。
#[must_use]
pub fn scan_second_undo_implementations(sources: &[SourceFile]) -> Vec<String> {
    let mut violations = Vec::new();
    for (path, text) in sources {
        if is_the_shared_session(path) {
            continue;
        }
        let production = production_region(text);
        for (lineno, line) in production.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            for needle in ["apply_inverse", "structural_inverse", "invert("] {
                if line.contains(needle) {
                    violations.push(format!(
                        "{path}:{} 出现 `{needle}` —— 生产代码里不许有第二份反向应用实现",
                        lineno + 1
                    ));
                }
            }
            for needle in [".undo_with(", ".undo("] {
                if line.contains(needle) {
                    violations.push(format!(
                        "{path}:{} 出现 `{needle}` —— 生产代码只允许调用 undo_session 的入口",
                        lineno + 1
                    ));
                }
            }
        }
    }
    violations
}

/// 本仓库里"生产源码"的根（两个 crate 的 `src/`）。
///
/// 由 `CARGO_MANIFEST_DIR` 反推：本文件被 `crates/yeban-mcp` 与 `crates/yeban-app`
/// 两侧编译，两者的清单目录都在 `crates/<name>` ⇒ `../..` 恒为仓库根。
#[must_use]
pub fn production_source_roots() -> Vec<PathBuf> {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    vec![
        repo.join("crates").join("yeban-mcp").join("src"),
        repo.join("crates").join("yeban-app").join("src"),
    ]
}

/// 读出一批 `.rs` 源文件（递归）。
#[must_use]
pub fn read_rust_sources(roots: &[PathBuf]) -> Vec<SourceFile> {
    let mut files = Vec::new();
    let mut stack: Vec<PathBuf> = roots.to_vec();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(std::ffi::OsStr::to_str) != Some("rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            files.push((path.display().to_string(), text));
        }
    }
    files
}

// ---------------------------------------------------------------------------
// 判据
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_model::samples::filled_project;

    const NOW: u64 = 1_760_000_000_000;

    fn session() -> (UndoSession, WiringFixture) {
        let project = filled_project();
        let fixture = wiring_fixture(&project).expect("规范样本工程里有 MIDI 音符");
        let session = UndoSession::open("<fixture>", "yeban-app", project, NOW).expect("打开");
        (session, fixture)
    }

    fn commit_fixture(session: &mut UndoSession, fixture: &WiringFixture) {
        session
            .commit(CommitRequest {
                now_ms: NOW + 1,
                origin: OpOrigin::UserUi,
                message: "判据夹具: 改力度".to_owned(),
                ops: vec![fixture.op()],
            })
            .expect("提交");
    }

    /// 当前音符力度（用来构造"前置条件成立"的下一条 op）。
    fn current_velocity(session: &UndoSession, fixture: &WiringFixture) -> u8 {
        session
            .project()
            .clip_pool
            .get(&fixture.clip_id)
            .and_then(|entry| entry.content.notes())
            .and_then(|notes| notes.get(&fixture.note_id))
            .expect("音符")
            .velocity
    }

    /// 构造一条"以当前力度为 old_vel"的力度修改 op。
    fn velocity_op(session: &UndoSession, fixture: &WiringFixture, new_vel: u8) -> Op {
        Op::ModifyNoteVelocity {
            track_id: fixture.track_id,
            clip_id: fixture.clip_id,
            note_id: fixture.note_id,
            old_vel: current_velocity(session, fixture),
            new_vel,
        }
    }

    /// 判据 ①（模型侧）：单步撤销**逐字节**回到上一版。
    #[test]
    fn single_undo_returns_to_the_previous_bytes() {
        let (mut session, fixture) = session();
        let before = session.project_bytes().expect("字节");
        commit_fixture(&mut session, &fixture);
        let edited = session.project_bytes().expect("字节");
        assert_ne!(before, edited, "夹具必须真的改了工程");

        let outcome = session.undo_steps(1).expect("撤销");
        assert_eq!(outcome.steps, 1);
        assert_eq!(outcome.undone_total, 1);
        // 一次提交 = 一条 `Op::Batch`（整批算一步）⇒ 被撤销的 op 就是那个批次。
        assert_eq!(outcome.op_kinds, vec!["Batch"]);
        assert_eq!(
            session.project_bytes().expect("字节"),
            before,
            "逐字节回到上一版"
        );
        assert_eq!(
            session.project().tracks.len(),
            filled_project().tracks.len()
        );
    }

    /// 判据 ②：连续 N 步撤销**逐步**回退（每一步都逐字节可比）。
    #[test]
    fn consecutive_undos_step_back_byte_by_byte() {
        let (mut session, fixture) = session();
        let v0 = session.project_bytes().expect("字节");
        let mut states = vec![v0.clone()];
        for step in 0..3_u8 {
            let op = velocity_op(&session, &fixture, 30 + step);
            session
                .commit(CommitRequest {
                    now_ms: NOW + u64::from(step) + 1,
                    origin: OpOrigin::UserUi,
                    message: format!("判据夹具 #{step}"),
                    ops: vec![op],
                })
                .expect("提交");
            states.push(session.project_bytes().expect("字节"));
        }
        assert_eq!(session.state().undoable(&session.graph).expect("可撤销"), 3);
        for step in (0..3).rev() {
            session.undo_steps(1).expect("撤销");
            assert_eq!(
                session.project_bytes().expect("字节"),
                states[step],
                "第 {step} 步快照必须逐字节可回到"
            );
        }
        assert_eq!(
            session.undo_steps(1).unwrap_err(),
            UndoRefusal::NoHistory { undoable: 0 }
        );
    }

    /// 判据 ③：重做**逐字节**回到撤销前。
    #[test]
    fn redo_returns_to_the_bytes_before_the_undo() {
        let (mut session, fixture) = session();
        commit_fixture(&mut session, &fixture);
        let edited = session.project_bytes().expect("字节");
        let restored = session.undo_steps(1).expect("撤销");
        assert_eq!(restored.steps, 1);

        let redone = session.redo_steps(1).expect("重做");
        assert_eq!(redone.steps, 1);
        assert_eq!(redone.undone_total, 0);
        assert_eq!(session.project_bytes().expect("字节"), edited);
        assert_eq!(session.redo_steps(1).unwrap_err(), UndoRefusal::NoRedo);
    }

    /// 判据 ⑥（模型侧）：`dryRun` 式的**只读模拟**与真做逐字节一致，且模拟不改任何状态。
    #[test]
    fn the_read_only_simulation_matches_the_real_undo_and_redo() {
        let (mut session, fixture) = session();
        commit_fixture(&mut session, &fixture);
        let before = session.project_bytes().expect("字节");
        let state = session.state().clone();

        // 模拟撤销：只读，真状态一位不动。
        let simulated = simulate_undo(session.graph(), session.project(), &state, 1).expect("模拟");
        assert_eq!(
            session.project_bytes().expect("字节"),
            before,
            "模拟不得改真状态"
        );
        assert_eq!(session.state(), &state, "模拟不得改会话态");
        assert_eq!(
            undo_op_kinds(session.graph(), &state, 1).expect("预览 op"),
            vec!["Batch"]
        );

        // 真做：与模拟逐字节相同。
        let outcome = session.undo_steps(1).expect("真做");
        assert_eq!(outcome.steps, 1);
        assert_eq!(
            canonical_project_bytes(&simulated).expect("模拟字节"),
            session.project_bytes().expect("真做字节"),
            "预览与真做必须逐字节相同"
        );

        // 模拟重做：同样只读、同样逐字节一致。
        let undone_state = session.state().clone();
        let undone_bytes = session.project_bytes().expect("字节");
        assert_eq!(
            redo_op_kinds(session.graph(), &undone_state, 1).expect("预览 op"),
            vec!["Batch"]
        );
        let simulated_redo =
            simulate_redo(session.graph(), session.project(), &undone_state, 1).expect("模拟");
        assert_eq!(session.project_bytes().expect("字节"), undone_bytes);
        session.redo_steps(1).expect("真做");
        assert_eq!(
            canonical_project_bytes(&simulated_redo).expect("模拟字节"),
            session.project_bytes().expect("真做字节")
        );
        assert_eq!(session.project_bytes().expect("字节"), before);
    }

    /// 判据 ④（模型侧）：`Op::Batch` 的多条子 op 算**一步**，不是 N 步。
    #[test]
    fn a_batch_counts_as_exactly_one_step() {
        let (mut session, fixture) = session();
        let before = session.project_bytes().expect("字节");
        // 三条子 op（力度 42 → 99 → 回到原值），整批必须原子地一步回退。
        let old = current_velocity(&session, &fixture);
        let first = velocity_op(&session, &fixture, 42);
        let second = Op::ModifyNoteVelocity {
            track_id: fixture.track_id,
            clip_id: fixture.clip_id,
            note_id: fixture.note_id,
            old_vel: 42,
            new_vel: 99,
        };
        let third = Op::ModifyNoteVelocity {
            track_id: fixture.track_id,
            clip_id: fixture.clip_id,
            note_id: fixture.note_id,
            old_vel: 99,
            new_vel: old,
        };
        session
            .commit(CommitRequest {
                now_ms: NOW + 1,
                origin: OpOrigin::UserUi,
                message: "判据夹具: 批量".to_owned(),
                ops: vec![first, second, third],
            })
            .expect("提交");
        assert_eq!(
            session.state().undoable(&session.graph).expect("可撤销"),
            1,
            "三条子 op 的一条提交 = 一步"
        );
        assert_eq!(current_velocity(&session, &fixture), old);
        let outcome = session.undo_steps(1).expect("撤销");
        assert_eq!(outcome.steps, 1);
        assert_eq!(outcome.op_kinds, vec!["Batch"]);
        assert_eq!(session.project_bytes().expect("字节"), before);
    }

    /// 判据 ⑤：无历史可撤 ⇒ 明确拒绝 + **工程一位不变**。
    #[test]
    fn undo_without_history_refuses_and_leaves_the_project_untouched() {
        let (mut session, _fixture) = session();
        let before = session.project_bytes().expect("字节");
        let fingerprint = session.fingerprint().expect("指纹");
        let head = session.state().head(session.graph()).expect("头");
        assert_eq!(
            session.undo_steps(1).unwrap_err(),
            UndoRefusal::NoHistory { undoable: 0 }
        );
        assert_eq!(session.project_bytes().expect("字节"), before);
        assert_eq!(session.fingerprint().expect("指纹"), fingerprint);
        assert_eq!(session.state().undone(), 0);
        assert_eq!(session.state().head(session.graph()).expect("头"), head);
        assert!(!session.state().can_undo(session.graph()));
    }

    /// 判据 ⑦：游标**不落盘** —— 撤销不改动提交图谱一个字段，
    /// 且工程字节里不含任何"游标"痕迹。
    ///
    /// ⚠ 这里刻意**不**用 `serde_json` 序列化图谱：本文件会被 `yeban-app` 一并编译，
    /// 而那个 crate 没有 `serde_json` 依赖（零新增依赖）。字节级的那一半
    /// （`history.dag` 逐字节不变）住在 `crates/yeban-mcp/tests/undo_wiring.rs`。
    #[test]
    fn the_cursor_is_never_persisted() {
        let (mut session, fixture) = session();
        commit_fixture(&mut session, &fixture);
        let graph_before = session.graph().clone();
        let bytes_before = session.project_bytes().expect("字节");
        session.undo_steps(1).expect("撤销");
        assert_eq!(
            session.graph(),
            &graph_before,
            "撤销不得改动提交图谱（游标不在图谱里）"
        );
        let bytes_after = session.project_bytes().expect("字节");
        assert_ne!(bytes_after, bytes_before);
        // 游标是**会话态**：类型上就没有 Serialize（模型刻意如此）。
        let text = String::from_utf8_lossy(&bytes_after).into_owned();
        for needle in ["cursor", "Cursor", "undone", "skip"] {
            assert!(
                !text.contains(needle),
                "工程字节里不该出现 `{needle}`：{text}"
            );
        }
    }

    /// 判据 ⑫：撤销**不越过**"工程打开"边界。
    #[test]
    fn undo_never_crosses_a_project_open_boundary() {
        let (mut first, fixture) = session();
        commit_fixture(&mut first, &fixture);
        assert_eq!(first.state().undoable(first.graph()).expect("可撤销"), 1);

        let second_project = filled_project();
        let mut second =
            UndoSession::open("另一个工程", "yeban-app", second_project, NOW).expect("打开");
        assert_eq!(second.state().undoable(second.graph()).expect("可撤销"), 0);
        assert_eq!(second.graph().commit_count(), 1, "新会话只有一条根提交");
        let before = second.project_bytes().expect("字节");
        assert_eq!(
            second.undo_steps(1).unwrap_err(),
            UndoRefusal::NoHistory { undoable: 0 },
            "打开新工程之后不能撤到上一个工程的状态"
        );
        assert_eq!(second.project_bytes().expect("字节"), before);
    }

    /// 判据 ⑬：幂等 / 边界 —— 撤到底之后再撤不改变任何一位；
    /// 撤销后再提交 ⇒ 按模型既有的 `fork_anonymous` 派生匿名分支，被撤销的 op 不可再撤。
    #[test]
    fn undo_then_commit_forks_so_discarded_ops_cannot_be_undone_again() {
        let (mut session, fixture) = session();
        let v0 = session.project_bytes().expect("字节");
        commit_fixture(&mut session, &fixture);
        let v1 = session.project_bytes().expect("字节");

        assert_eq!(session.undo_steps(1).expect("撤销").steps, 1);
        assert_eq!(session.project_bytes().expect("字节"), v0);
        assert_eq!(
            session.undo_steps(1).unwrap_err(),
            UndoRefusal::NoHistory { undoable: 0 }
        );

        // 撤销之后继续编辑：模型在**撤销位置**派生匿名分支（原头保留为孤岛）。
        let mut op = fixture.op();
        if let Op::ModifyNoteVelocity { new_vel, .. } = &mut op {
            *new_vel = 77;
        }
        session
            .commit(CommitRequest {
                now_ms: NOW + 2,
                origin: OpOrigin::UserUi,
                message: "撤销后继续编辑".to_owned(),
                ops: vec![op],
            })
            .expect("提交");
        assert!(
            session
                .state()
                .branch()
                .starts_with(yeban_model::ANONYMOUS_BRANCH_PREFIX),
            "撤销后继续编辑必须派生匿名分支: {}",
            session.state().branch()
        );
        assert_eq!(session.state().undone(), 0, "新头之上没有已撤销的步骤");
        assert_eq!(
            session.state().undoable(session.graph()).expect("可撤销"),
            1
        );
        let v2 = session.project_bytes().expect("字节");
        assert_ne!(v2, v1);
        // 再撤一步回到 v0（而不是回到被丢弃的 v1）。
        assert_eq!(session.undo_steps(1).expect("撤销").steps, 1);
        assert_eq!(session.project_bytes().expect("字节"), v0);
        assert_eq!(
            session.undo_steps(1).unwrap_err(),
            UndoRefusal::NoHistory { undoable: 0 },
            "被丢弃的那条 op 不可能被再撤一次"
        );
        assert_eq!(session.graph().branch_count(), 2, "孤岛 + 匿名分支");
    }

    /// 判据 ⑩：撤销**不破坏**既有不变量（`validate()` 仍通过）。
    #[test]
    fn the_project_still_validates_after_undo_and_redo() {
        let (mut session, fixture) = session();
        commit_fixture(&mut session, &fixture);
        session.project().validate().expect("提交后合法");
        session.undo_steps(1).expect("撤销");
        session.project().validate().expect("撤销后合法");
        session.redo_steps(1).expect("重做");
        session.project().validate().expect("重做后合法");
    }

    /// 判据 ⑪（模型侧）：撤销后再保存、再打开 ⇒ 与撤销后的内存态逐字节一致。
    ///
    /// 容器往返用**真实的**容器读写（唯一工程格式，ADR-0001 D43）；
    /// `history.dag` 的**字节**在这一条里是占位（图谱对象本身在内存里直接传回
    /// [`UndoSession::open_with_graph`]）—— 字节级的序列化往返（`serde_json`）
    /// 住在 `crates/yeban-mcp/tests/undo_wiring.rs`。
    #[test]
    fn save_and_reopen_after_an_undo_matches_the_in_memory_state() {
        let (mut session, fixture) = session();
        let v0 = session.project_bytes().expect("字节");
        commit_fixture(&mut session, &fixture);
        let edited_bytes = session.project_bytes().expect("字节");
        session.undo_steps(1).expect("撤销");
        assert_eq!(session.project_bytes().expect("字节"), v0);

        // "保存" = 模型的容器写出（`history.dag` 用容器要求的原始字节）。
        let bytes = container::write_project_container(
            session.project(),
            b"fixture-history-dag",
            &std::collections::BTreeMap::new(),
        )
        .expect("容器写出");
        // "再打开" = 模型容器读回。
        let archive =
            container::read_project_container(&bytes, &container::ContainerLimits::default())
                .expect("容器读回");
        assert_eq!(archive.history_dag, b"fixture-history-dag");
        assert_eq!(
            container::write_project_container(
                &archive.project,
                &archive.history_dag,
                &std::collections::BTreeMap::new()
            )
            .expect("再写出"),
            bytes,
            "容器往返必须逐字节稳定"
        );
        let mut reopened = UndoSession::open_with_graph(
            "<reopen>",
            "yeban-app",
            archive.project,
            session.graph().clone(),
            NOW,
        )
        .expect("再打开");
        assert_eq!(
            reopened.project_bytes().expect("字节"),
            session.project_bytes().expect("字节"),
            "再打开之后的工程 = 撤销后的内存态"
        );
        assert_eq!(
            reopened.state().head(reopened.graph()).expect("头"),
            session.state().head(session.graph()).expect("头"),
            "图谱整份恢复（分支头一致）"
        );
        // 游标**没有被持久化**，但它可以从（文档, 图谱）**推导**出来：
        // 文档停在被撤销后的状态 ⇒ 推导出"已撤销 1 步"，于是重做栈仍然是对的。
        assert_eq!(reopened.state().undone(), 1, "游标是推导出来的");
        assert_eq!(
            reopened.state().undoable(reopened.graph()).expect("可撤销"),
            0
        );
        assert!(reopened.state().can_redo());
        reopened.redo_steps(1).expect("重做被撤销的那一步");
        assert_eq!(
            reopened.project_bytes().expect("字节"),
            edited_bytes,
            "重开之后重做 ⇒ 回到保存前编辑过的状态"
        );
    }

    /// 判据 ⑦ 的另一半：游标**可从（文档, 图谱）推导**，因此不需要持久化。
    ///
    /// 这是"撤销 → 保存 → 退出 → 重开"能继续工作的**机制性**保证：
    /// 推导出来的游标与真游标行为一致（再撤 / 再重做的字节都相同）。
    #[test]
    fn the_cursor_is_derivable_from_the_document_and_the_graph() {
        let (mut session, fixture) = session();
        for target in [30_u8, 40, 50] {
            let op = velocity_op(&session, &fixture, target);
            session
                .commit(CommitRequest {
                    now_ms: NOW + u64::from(target),
                    origin: OpOrigin::UserUi,
                    message: format!("判据夹具 {target}"),
                    ops: vec![op],
                })
                .expect("提交");
        }
        session.undo_steps(2).expect("撤两步");
        assert_eq!(session.state().undone(), 2);

        // —— 丢掉游标，仅凭（文档, 图谱）重新推导 ——
        let project = session.project().clone();
        let graph = session.graph().clone();
        let mut derived = UndoState::new("yeban-app");
        assert_eq!(
            derived.align_with(&graph, &project).expect("推导"),
            2,
            "推导出的已撤销步数必须等于真游标"
        );

        // 推导出来的游标行为一致：再撤一步 / 再重做一步都落在同一个字节上。
        let mut from_derived_project = project.clone();
        let mut from_derived_state = derived.clone();
        undo(
            &graph,
            &mut from_derived_project,
            &mut from_derived_state,
            1,
        )
        .expect("推导态撤销");
        session.undo_steps(1).expect("真游标撤销");
        assert_eq!(
            canonical_project_bytes(&from_derived_project).expect("字节"),
            session.project_bytes().expect("字节"),
            "推导出的游标与真游标必须给出同一串字节"
        );

        // 已经全部撤掉时，推导出"撤到底"（而不是 0）。
        // 此刻 `session.project()` 就是三条 op 全不应用的那个文档状态。
        let mut derived_again = UndoState::new("yeban-app");
        assert_eq!(
            derived_again
                .align_with(&graph, session.project())
                .expect("再推导"),
            3,
            "全部撤掉 ⇒ 推导出 3 步"
        );
        assert_eq!(
            derived_again.undoable(&graph).expect("可撤销"),
            0,
            "撤到底之后没有可撤销的步骤"
        );
    }

    /// 模型层拒绝时**不得**改动工程，也不得让游标与文档错位。
    #[test]
    fn a_model_refusal_leaves_the_project_and_the_cursor_alone() {
        let (mut session, fixture) = session();
        commit_fixture(&mut session, &fixture);
        // 绕过日志直接改工程（模拟"外部把状态写坏了"）。
        {
            let project = session.project_mut_for_tests();
            let entry = project.clip_pool.get_mut(&fixture.clip_id).expect("片段");
            let notes = entry.content.notes_mut().expect("MIDI 片段");
            notes.get_mut(&fixture.note_id).expect("音符").velocity = 5;
        }
        let tampered = session.project_bytes().expect("字节");
        let refusal = session.undo_steps(1).unwrap_err();
        assert!(
            matches!(refusal, UndoRefusal::Model { .. }),
            "逆操作前置条件不成立必须变成 Model 拒绝: {refusal:?}"
        );
        assert_eq!(
            session.project_bytes().expect("字节"),
            tampered,
            "拒绝路径不得改动工程"
        );
        assert_eq!(session.state().undone(), 0, "拒绝路径不得前移游标");
    }

    /// 判据：**生产代码里只有一个撤销实现**（grep 级 + 结构级）。
    #[test]
    fn no_second_undo_implementation_exists_in_the_workspace() {
        let roots = production_source_roots();
        if roots.iter().any(|root| !root.is_dir()) {
            eprintln!(
                "SKIP[no_second_undo_implementation_exists_in_the_workspace]: \
                 找不到 {roots:?}（本判据必须在仓库内跑）"
            );
            return;
        }
        let sources = read_rust_sources(&roots);
        assert!(
            sources.len() >= 10,
            "应当扫到两个 crate 的源码: {}",
            sources.len()
        );
        let violations = scan_second_undo_implementations(&sources);
        assert!(
            violations.is_empty(),
            "生产代码里出现了第二份撤销实现:\n{}",
            violations.join("\n")
        );
        // 反向确认"本文件确实调用了模型的那个 API"——否则上一条会空转。
        // 只看**生产区**（`#[cfg(test)]` 之前的文本）：判据自己的字符串字面量也在文件里。
        //
        // ⚠ 不能写 `CARGO_MANIFEST_DIR/src/undo_session.rs`：本文件被 `yeban-app` 用
        // `#[path]` 引入，那时清单目录是 `crates/yeban-app`（共享实现在隔壁）。因此从
        // **扫描结果**里找自己 —— 两个 crate 下都成立。
        let me = sources
            .iter()
            .find(|(path, _)| is_the_shared_session(path))
            .map(|(_, text)| text.clone())
            .expect("共享实现必须在扫描集合里");
        let production = production_region(&me);
        assert!(
            production.contains(".undo_with("),
            "本文件必须真的调用模型入口 `CommitGraph::undo_with`"
        );
        assert_eq!(
            production.matches(".undo_with(").count(),
            1,
            "撤销入口只能出现在一处（`undo` 函数里；文档里写的是 `::undo_with`）"
        );
        assert!(
            !production
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .any(|line| line.contains("apply_inverse")),
            "共享实现自己不得构造逆操作（文档里可以解释为什么）"
        );
    }

    /// 注入判据的**纯函数**面：把一份"看似等价的独立实现"喂给守卫 ⇒ 必须报违规。
    ///
    /// 这条判据让"注入 ⇒ 变红"可被**本机**复现（真注入的记录见台账 §判据）。
    #[test]
    fn the_guard_flags_a_look_alike_independent_implementation() {
        let fake = vec![(
            "crates/yeban-mcp/src/domain/undo_alt.rs".to_owned(),
            "pub fn undo_alt(doc: &mut YebanProjectV1, ops: &[StampedOp]) {\n    for op in ops.iter().rev() {\n        op.apply_inverse(doc).expect(\"x\");\n    }\n}\n".to_owned(),
        )];
        let violations = scan_second_undo_implementations(&fake);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(violations[0].contains("apply_inverse"));

        // 反例 2: 绕开共享入口直接调模型撤销。
        let bypass = vec![(
            "crates/yeban-app/src/main.rs".to_owned(),
            "let n = graph.undo_with(&mut project, &head, &mut cursor, 1).unwrap();\n".to_owned(),
        )];
        assert_eq!(scan_second_undo_implementations(&bypass).len(), 1);

        // 反例 3: 测试区里的 `apply_inverse` **不算**生产代码（本仓库判据写在文件尾部）。
        let in_tests = vec![(
            "crates/yeban-mcp/src/domain/mod.rs".to_owned(),
            "pub fn ok() {}\n#[cfg(test)]\nmod tests {\n    fn t() { stamped.apply_inverse(&mut doc); }\n}\n".to_owned(),
        )];
        assert!(scan_second_undo_implementations(&in_tests).is_empty());

        // 反例 4（路径形态）：**两种平台的两种拼法**下都必须认出本文件自己。
        // 这是 CI 的 windows 腿两次抓到的真实缺陷
        // （run 37268427827 与 37268901708：`ends_with("src/undo_session.rs")` 在
        //  `…\\src\\undo_session.rs` 上为假）。
        for self_path in [
            "/repo/crates/yeban-mcp/src/undo_session.rs",
            "/repo/crates/yeban-mcp/../../crates/yeban-mcp/src/undo_session.rs",
            "D:\\a\\yeban\\yeban\\crates\\yeban-mcp\\src\\undo_session.rs",
            "D:\\a\\yeban\\yeban\\crates\\yeban-mcp\\..\\..\\crates\\yeban-mcp\\src\\undo_session.rs",
            "\\\\?\\D:\\a\\yeban\\yeban\\crates\\yeban-mcp\\src\\undo_session.rs",
        ] {
            assert!(
                is_the_shared_session(self_path),
                "必须认出本共享实现: {self_path}"
            );
            let sources: Vec<SourceFile> = vec![(
                self_path.to_owned(),
                "let n = graph.undo_with(&mut project, &head, &mut cursor, 1).unwrap();\n"
                    .to_owned(),
            )];
            assert!(
                scan_second_undo_implementations(&sources).is_empty(),
                "本共享实现不得被判成第二份实现: {self_path}"
            );
        }
        // 反向：别的 crate 下的同名文件**不得**被当成自己（否则守卫会空转）。
        for other in [
            "/repo/crates/yeban-model/src/undo_session.rs",
            "D:\\a\\yeban\\yeban\\crates\\yeban-model\\src\\undo_session.rs",
        ] {
            assert!(!is_the_shared_session(other), "不得误伤: {other}");
        }

        // 反例 5: 注释里提到这些词不算违规（文档要能解释为什么不许写）。
        let comments = vec![(
            "crates/yeban-app/src/undo.rs".to_owned(),
            "//! 本模块不写 apply_inverse，撤销一律走 undo_session\n".to_owned(),
        )];
        assert!(scan_second_undo_implementations(&comments).is_empty());
    }

    /// 判据：夹具必须在**两侧**都能构造出来（否则接线判据会空转）。
    #[test]
    fn the_wiring_fixture_exists_in_the_canonical_sample() {
        let project = filled_project();
        let fixture = wiring_fixture(&project).expect("夹具");
        assert_ne!(fixture.old_velocity, fixture.new_velocity);
        assert!(project.tracks.contains_key(&fixture.track_id));
        assert!(project.clip_pool.contains_key(&fixture.clip_id));
        assert_eq!(fixture.op().name(), "ModifyNoteVelocity");
    }

    /// 判据：打开一个带图谱的会话时，`main` 分支必须存在（否则拒绝而不是假装）。
    #[test]
    fn a_graph_without_main_is_refused_on_open() {
        let mut graph = CommitGraph::new();
        graph
            .genesis(CommitDraft::new(
                EntityId::new(),
                "not-main",
                "yeban-app",
                "nope",
            ))
            .expect("根提交");
        let refusal =
            UndoSession::open_with_graph("<x>", "yeban-app", filled_project(), graph, NOW)
                .unwrap_err();
        assert_eq!(
            refusal,
            UndoRefusal::NoBranch {
                branch: MAIN_BRANCH.to_owned()
            }
        );
    }
}
