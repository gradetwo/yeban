//! 提案（Musical PR）的领域表示与**可追溯记录** [MCP-TOOL-005/006/007/009/010]。
//!
//! ## 提案不是"待办事项"，是一条真的隔离分支
//!
//! `yeban_propose_section` / `yeban_edit_notes` / `yeban_set_macro` 都**不直接改工程**：
//! 它们把一组 `Op` 放进一条隔离分支 `ai/proposal-{ulid}`，并留下一条
//! [`Proposal`] 记录（谁、什么时候、基于哪个提交、动了哪些 op、最后被合并还是被拒）。
//! 只有 `yeban_merge_proposal` 才会把 op 落到主分支上；`yeban_reject_proposal`
//! 只改记录状态，**不删除**记录 —— "拒绝"也必须可追溯。
//!
//! ## `CommitGraph` 的两条缺口：**已关闭**（就地更正，不是改写历史）
//!
//! 实测 `yeban-model` 的 `CommitGraph` 曾经缺两条能力（`docs/ledger/tools-domain-notes.md`
//! 的 **needs-5**）：
//!
//! | 曾经缺的能力 | 模型层现在有什么 |
//! | :--- | :--- |
//! | 在**指定父提交**上创建**命名**分支 | `CommitGraph::create_branch`：指向一个**已存在**的提交，分支名由调用方决定，**不**写提交、**不**动既有分支头 |
//! | 创建**多父**合并提交 | `CommitGraph::append_merge`：父集合 = `[当前分支头] ++ extra_parents`，第一父恒为当前分支头 |
//!
//! 本线的处置（**needs-5 的第二半就在这两条上关闭**）：
//!
//! - 提案分支用 `create_branch` 建在 `draft.base_commit` 上，再用 `append` 写提案提交
//!   ⇒ **分叉点是一条父边**，而不是只写在 [`Proposal::base_commit`] 里的一个字面量；
//! - **合并**用 `crate::undo_session::commit_merge` 写一条**多父合并提交**，
//!   第二父是提案分支的头 ⇒ "这次合并并了谁"由**图谱**（`history.dag`）自己承担；
//!   [`Proposal`] 记录是**另一份**可读的索引，不再是唯一的一份。
//!
//! 主干形状**没有**变：`append_merge` 的第一父仍是活跃分支头，`main` 的祖先链上
//! 不出现提案的提交，撤销仍然是"一次 `Cmd+Z` 回退整套 AI 变更"（`ARCH-OPS-002`）。

use serde_json::{Map, Value};

use yeban_model::{EntityId, Op, StampedOp};

use super::section::ops_to_value;

/// 提案的生命周期状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProposalStatus {
    /// 待审查。
    Open,
    /// 已合并进主分支。
    Merged,
    /// 已拒绝（记录保留）。
    Rejected,
}

impl ProposalStatus {
    /// 规范字符串。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Merged => "merged",
            Self::Rejected => "rejected",
        }
    }
}

/// 一次待创建的提案（由 `yeban_propose_section` / `yeban_edit_notes` / `yeban_set_macro` 产出）。
#[derive(Clone, Debug, PartialEq)]
pub struct ProposalDraft {
    /// 提案类别（`section` / `notes` / `macro`）。
    pub kind: &'static str,
    /// 一句话标题。
    pub title: String,
    /// 批次描述（进 `Op::Batch` 的 `description`）。
    pub description: String,
    /// 领域操作（顺序即施加顺序）。
    pub ops: Vec<Op>,
    /// 提案基于的主分支头提交。
    pub base_commit: EntityId,
    /// 提案基于的工程内容摘要（合并时的冲突判定依据）。
    pub base_digest: String,
}

/// 一条提案记录。
#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    /// 提案身份。
    pub id: EntityId,
    /// 隔离分支名 `ai/proposal-{ulid}`。
    pub branch: String,
    /// 提案类别。
    pub kind: &'static str,
    /// 标题。
    pub title: String,
    /// 批次描述。
    pub description: String,
    /// 状态。
    pub status: ProposalStatus,
    /// 基于的主分支头。
    pub base_commit: EntityId,
    /// 提案分支上的提交（其父提交 **=** `base_commit`，见 `CommitGraph::create_branch`）。
    pub head_commit: EntityId,
    /// 主分支上的合并提交（未合并时为 `None`）。
    pub merge_commit: Option<EntityId>,
    /// 捕获的操作日志（带 `McpProposal` 来源与戳）。
    pub ops: Vec<StampedOp>,
    /// 创建时间 (Unix 毫秒，由注入的时钟提供)。
    pub created_at: u64,
    /// 决议时间。
    pub resolved_at: Option<u64>,
    /// 决议说明（合并信息或拒绝原因）。
    pub resolution: Option<String>,
}

impl Proposal {
    /// 响应里用的**可追溯**摘要（含 op 类别清单，不含完整载荷）。
    #[must_use]
    pub fn summary(&self) -> Value {
        let mut map = Map::new();
        map.insert(
            "proposalId".to_owned(),
            Value::from(self.id.to_canonical_string()),
        );
        map.insert("branch".to_owned(), Value::from(self.branch.clone()));
        map.insert("kind".to_owned(), Value::from(self.kind));
        map.insert("title".to_owned(), Value::from(self.title.clone()));
        map.insert(
            "description".to_owned(),
            Value::from(self.description.clone()),
        );
        map.insert("status".to_owned(), Value::from(self.status.as_str()));
        map.insert(
            "baseCommit".to_owned(),
            Value::from(self.base_commit.to_canonical_string()),
        );
        map.insert(
            "headCommit".to_owned(),
            Value::from(self.head_commit.to_canonical_string()),
        );
        map.insert(
            "mergeCommit".to_owned(),
            self.merge_commit
                .map_or(Value::Null, |id| Value::from(id.to_canonical_string())),
        );
        map.insert("createdAt".to_owned(), Value::from(self.created_at));
        map.insert(
            "resolvedAt".to_owned(),
            self.resolved_at.map_or(Value::Null, Value::from),
        );
        map.insert(
            "resolution".to_owned(),
            self.resolution
                .as_ref()
                .map_or(Value::Null, |text| Value::from(text.clone())),
        );
        map.insert("opCount".to_owned(), Value::from(self.ops.len()));
        map.insert(
            "opKinds".to_owned(),
            Value::Array(
                self.ops
                    .iter()
                    .map(|stamped| Value::from(stamped.op.name()))
                    .collect(),
            ),
        );
        Value::Object(map)
    }

    /// 提案的**完整**载荷（op 逐条，供审查者比对；`ARCH-OPS-002` 的 Musical PR 审查）。
    ///
    /// ⚠ 它**不是**提案类工具的缺省回传形状 `[BASELINE-006]`：回传完整 op 载荷
    /// （16 小节段落生成，`ops` 数组本身实测 6,702 字节）会把往返 JSON 顶到
    /// 9,670 字节，是规范 4 KB 上限的 2.36 倍。缺省回传的是 [`Self::summary`]
    /// （结构化字段），完整载荷由 `arguments.includeOps: true` 显式索取 —— 见
    /// [`crate::tools::INCLUDE_OPS_PARAM`] 与 [`crate::payload`]。
    #[must_use]
    pub fn detail(&self) -> Value {
        let mut map = self.summary().as_object().cloned().unwrap_or_default();
        map.insert(
            "ops".to_owned(),
            Value::Array(
                self.ops
                    .iter()
                    .map(|stamped| {
                        serde_json::json!({
                            "origin": stamped.origin,
                            "timestamp": stamped.timestamp,
                            "op": serde_json::to_value(&stamped.op).unwrap_or(Value::Null),
                        })
                    })
                    .collect(),
            ),
        );
        Value::Object(map)
    }
}

/// 提案里承载的 op 载荷（预览与提交共用同一份形状）。
#[must_use]
pub fn draft_ops_value(draft: &ProposalDraft) -> Value {
    ops_to_value(&draft.ops)
}

/// 提案的**只读投影响应**——界面（`crates/yeban-app/ui/dialogs/musical_pr_drawer.slint`）
/// 的抽屉数据面唯一的来源形状。
///
/// ## 为什么另立一个类型, 而不是把 [`Proposal`] 直接交给界面
///
/// [`Proposal`] 里有 `Vec<StampedOp>`（一次 16 小节段落提案实测 6,702 字节, 见
/// [`Proposal::detail`] 的警告）与提交身份 / 时间戳 / 决议说明。界面要画的只有
/// **类别、标题、条数、op 条数、状态**这五格 —— 把整条记录搬过 `host_domain` 的边界
/// 就等于让界面去复制一份它不读的大对象。
///
/// ## 这个形状里**没有**的字段（这一句是本切片的关键）
///
/// 抽屉原先有一格 `confidences`（"每条提案一个百分比置信度"）。⛔ 本类型**刻意不**给
/// 它一个字段：`grep -rni confidence crates/yeban-mcp/src crates/yeban-model/src
/// crates/yeban-theory/src` 的实测读数是 **0** 处命中（本段自述是唯一例外）—— 提案记录里
/// 从来没存在过置信度, 任何百分比都只能是**编造的**。同理没有"轨道号 / 小节号"两格：
/// 提案承载的是**op 批次**（[`Proposal::ops`]）, 不是单条轨道上的单条剪辑, 界面按提案
/// 下标算出来的"轨道 i · 小节 i+1"是**下标推导出的假读数**, 这里也不给它字段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalRow {
    /// 提案身份的规范文本（`EntityId::to_canonical_string`）。
    ///
    /// ⭐ 这就是 `docs/ledger/feature-alignment.md` 那行「最小代价 ①」要的
    /// **"当前待采纳的提案身份"的只读来源**：`yeban_merge_proposal` /
    /// `yeban_reject_proposal` 要的 `proposalId` 从这里读, 界面不许自己造。
    pub id: String,
    /// 提案类别（`section` / `notes` / `macro` —— [`Proposal::kind`] 原文）。
    pub kind: &'static str,
    /// 一句话标题（[`Proposal::title`] 原文, ⛔ 不是界面拼的第二份文案）。
    pub title: String,
    /// 生命周期状态的规范文本（[`ProposalStatus::as_str`]）。
    pub status: &'static str,
    /// 本提案携带的 op 条数（[`Proposal::ops`] 的长度）。
    pub op_count: usize,
}

impl ProposalRow {
    /// 从一条提案记录投影（**唯一**的投影实现）。
    #[must_use]
    pub fn new(record: &Proposal) -> Self {
        Self {
            id: record.id.to_canonical_string(),
            kind: record.kind,
            title: record.title.clone(),
            status: record.status.as_str(),
            op_count: record.ops.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::deterministic_id;

    fn sample_proposal() -> Proposal {
        let id = deterministic_id("proposal");
        let base = deterministic_id("base");
        let head = deterministic_id("head");
        Proposal {
            id,
            branch: format!("ai/proposal-{}", id.to_canonical_string()),
            kind: "notes",
            title: "加一层八度".to_owned(),
            description: "edit_notes".to_owned(),
            status: ProposalStatus::Merged,
            base_commit: base,
            head_commit: head,
            merge_commit: Some(head),
            ops: Vec::new(),
            created_at: 1,
            resolved_at: Some(2),
            resolution: Some("merged".to_owned()),
        }
    }

    #[test]
    fn summary_is_traceable_and_deterministic() {
        let proposal = sample_proposal();
        let summary = proposal.summary();
        assert_eq!(summary["status"], "merged");
        assert!(
            summary["branch"]
                .as_str()
                .expect("branch")
                .starts_with("ai/proposal-")
        );
        assert_eq!(summary["opCount"], 0);
        assert_eq!(
            summary["mergeCommit"],
            proposal.head_commit.to_canonical_string()
        );
        assert_eq!(summary, sample_proposal().summary(), "摘要必须逐字节稳定");
    }

    #[test]
    fn status_strings_are_stable() {
        assert_eq!(ProposalStatus::Open.as_str(), "open");
        assert_eq!(ProposalStatus::Merged.as_str(), "merged");
        assert_eq!(ProposalStatus::Rejected.as_str(), "rejected");
    }

    #[test]
    fn detail_carries_full_op_payloads() {
        let mut proposal = sample_proposal();
        proposal.ops.push(StampedOp::new(
            yeban_model::OpOrigin::McpProposal {
                proposal_id: proposal.id,
                agent_name: "yeban-mcp".to_owned(),
            },
            7,
            Op::SetMacro {
                track_id: deterministic_id("track"),
                macro_index: 0,
                old_val: 0.0,
                new_val: 1.0,
            },
        ));
        let detail = proposal.detail();
        assert_eq!(detail["opCount"], 1);
        assert_eq!(detail["ops"][0]["timestamp"], 7);
        assert_eq!(detail["ops"][0]["op"]["SetMacro"]["macro_index"], 0);
        assert_eq!(
            detail["ops"][0]["origin"]["McpProposal"]["agent_name"],
            "yeban-mcp"
        );
    }
}
