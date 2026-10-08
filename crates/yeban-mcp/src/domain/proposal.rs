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
//! ## `CommitGraph` 的 API 缺口（登记，不隐藏）
//!
//! 实测 `yeban-model` 的 `CommitGraph`：
//!
//! | 想要的能力 | 现实 |
//! | :--- | :--- |
//! | 在**指定父提交**上创建**命名**分支 | **没有**。`genesis` 建根提交（`parents: []`），`append` 要求分支已存在，`fork_anonymous` 把分支名强制成 `anon-<ulid>` |
//! | 创建**多父**合并提交 | **没有**。`Commit` 有 `parents: Vec<EntityId>`，但 `append` 永远写 `parents: vec![head]` |
//!
//! 本线的处置：
//!
//! - 提案分支用 `genesis` 建成**孤立根提交** —— 这正是"隔离分支"的字面语义
//!   （它的 `ancestry()` 只有它自己，不会把提案的 op 混进主分支的撤销链）；
//! - **合并**在主分支上 `append` 一个携带单一原子 `Op::Batch` 的提交，
//!   两个方向的身份（`base_commit` / `head_commit` / `merge_commit`）由本模块的
//!   记录承担，主分支的撤销仍然是"一次 `Cmd+Z` 回退整套 AI 变更"（`ARCH-OPS-002`）。
//!
//! 两条缺口都写进 `docs/ledger/tools-domain-notes.md` 的 needs 清单。

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
    /// 提案分支的（孤立根）提交。
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
