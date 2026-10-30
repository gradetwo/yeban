//! AI 提案抽屉的**读面与写面** —— 界面侧提案唯一的出入口 (`[UI-A11Y-004]` / §10 Musical PR)。
//!
//! ## 这一格补的是哪一个洞（实测, 不是转述）
//!
//! `ui/dialogs/musical_pr_drawer.slint` 的四个数据面属性曾经**没有任何写者**：
//! 抽屉因此永远画"0 条提案 + 一句空态"。而领域侧**有**这些提案 —— 一次
//! `yeban_propose_section` / `yeban_edit_notes` / `yeban_set_macro` 就在
//! [`Domain::proposals`] 里留下一条 `Open` 记录（会话运行态, `MODEL-ISO-001` 第 2 层）。
//! 于是"AI 提了案, 界面上看不见"是**两侧都真、中间断开**的缺口。
//!
//! 本模块做两件**严格分开**的事：
//!
//! * **读**：`Domain → ProposalRowView → 抽屉属性`；
//! * **写**：`抽屉的一次选中 + 一个按钮 → ProposalDecision → ProjectAuthorityHandle::apply_host`
//!   —— 它**只构造** `HostAction` 那两臂再交给唯一的施加点, 合并 / 拒绝的语义一律住在
//!   领域侧（见下）。
//!
//! ## ⛔ 不发明第二份权威（结构性约束, 不是口号）
//!
//! * 提案记录**只**从 [`ProjectAuthorityHandle`] 的只读口 `open_proposal_rows()` 取
//!   （它经 `HttpServer::host_domain`, 签名里只有 `&Domain` —— 与
//!   `project()` / `undo_display()` / `graph()` 同一条边界, 不是新开的第二扇门）；
//! * **顺序**不在这里定义：排序口径住在 `Domain::open_proposals`（工具面的
//!   `yeban_list_proposals` 用的是**同一个**函数）, 抄第二遍就会产生"工具说第 1 条是
//!   A、界面把 A 画在第 2 行"的漂移, 而从任何一侧单独看都发现不了；
//! * **身份**不在这里造：[`ProposalRowView::id`] 是 `ProposalRow::id` 的原文,
//!   写面只把它塞进 `HostAction`。界面上没有任何一处能生成或猜测一个 `proposalId`；
//! * **合并不在这里实现**：[`ProposalSource::resolve`] 全程只有一次
//!   `apply_host` 调用, 没有 op 施加、没有提交构造、没有状态翻转（那些都在
//!   `crates/yeban-mcp/src/domain/mod.rs` 的 `merge_plan` / `reject_plan` 与
//!   `apply_plan_model` 里 —— 与工具面 `yeban_merge_proposal` /
//!   `yeban_reject_proposal` **同一个**函数, 判据
//!   `there_is_exactly_one_merge_and_one_reject_implementation` 钉住它）。
//!
//! ## ⛔ 没有数据源的读数, 一个都不给
//!
//! 抽屉原先有一格 `confidences`（每条提案一个百分比）。`confidence` 在
//! `yeban-mcp` / `yeban-model` / `yeban-theory` 三个 crate 的源码里实测 **0** 处命中
//! ⇒ 任何百分比都只能是编造的, 因此 [`ProposalRowView`] **没有**这一格。
//! 同理没有"轨道号 / 小节号"：提案承载的是 **op 批次**, 不是单条轨道上的单条剪辑,
//! 界面上那句"轨道 i · 小节 i+1"是按提案**下标**算出来的假读数 —— 现在画的是
//! `op_count`（真实条数）。
//!
//! ## 刷新点（两条, 都是事件驱动的, 没有定时器）
//!
//! | 时机 | 落点 | 为什么够用 |
//! | :--- | :--- | :--- |
//! | 用户打开抽屉 | [`crate::host::wire_proposal_drawer`] 的 `on_open_musical_pr` | "打开时看见的就是权威此刻的清单" |
//! | 会话侧改了工程 ⇒ 重投影 | `crates/yeban-app/src/reproject.rs` 的 `AuthorityMirror::sync_now` | **合并**提案会推进施加修订号（[`Plan::mutates_project`] 对 `Merge` 为真, 它真的改工作工程） |
//!
//! ⚠ **登记的边界**：`Propose` / `Reject` 按计划**不**推进施加修订号
//! （[`Plan::mutates_project`] 的口径：它们不动工作工程）, 因此"抽屉已经开着的时候
//! 又进来一条提案"不会自己刷新 —— 下一次打开抽屉或下一次重投影才会看见。这条边界写在
//! 这里而不是写在界面上（界面只报它**读到**的条数, 那句话本身始终是真话）。
//!
//! ## 写面的**拒绝**也走这一条路（为什么界面不需要"成功提示"）
//!
//! [`ProposalSource::resolve`] 的 `Err` 是领域 `Fault` 的**原文**（[`fault_message`]）。
//! 它被注入成抽屉的 `resolve-reason` 并**保持抽屉打开** ⇒ "点了没动"永远有一句
//! 来自权威的逐字解释, 而不是一个关掉的抽屉。反过来成功时**不**写理由：那一行从清单
//! 里消失（下一次读到的清单已经没有它）就是唯一的、真实的反馈 —— ⛔ 不弹"操作成功",
//! 那句话是"我请求了什么"的复述, 不是"发生了什么"的读数。

#[cfg(feature = "in-process-mcp")]
use crate::mcp_mount::ProjectAuthorityHandle;
#[cfg(feature = "in-process-mcp")]
use yeban_mcp::domain::error::Fault;

/// 一条提案在抽屉里要画的那四格（**唯一**的界面侧提案形状）。
///
/// 它是 `crates/yeban-mcp/src/domain/proposal.rs` 的 `ProposalRow` 的**界面侧投影**：
/// 与 [`crate::undo`] 对 `UndoDisplay` 做同一件事的理由相同 —— `yeban-app` 的默认
/// 依赖图**不含** `yeban-mcp`（`MUST-GATE-009`）, 所以默认构建里能编译的形态只能是
/// 宿主自己的类型。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalRowView {
    /// 提案身份的 canonical ULID 原文 —— 写面唯一能接受的凭据（⛔ 界面不许自造）。
    pub id: String,
    /// 提案类别（`section` / `notes` / `macro`）—— 领域记录的原文。
    pub kind: String,
    /// 一句话标题 —— 领域记录的原文（⛔ 界面不拼第二份文案）。
    pub title: String,
    /// 本提案携带的 op 条数（> 0；一次提案是一个 op 批次）。
    pub op_count: usize,
}

/// 界面上一次"要处理哪条提案"的**完整**陈述 —— 只有两条臂, 各自只带身份。
///
/// ⚠ 它**故意不带**别的格子：缺省文案（提交信息 / 拒绝原因）的口径住在领域侧
/// （`merge_plan` / `reject_plan` 的 `None` 分支）, 界面侧填一个自己的默认串就会
/// 与工具面的缺省不一致 —— 那正是"同一件事两个面各说一套"的形状。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalDecision {
    /// 采纳：把这条提案的 op 批次合进主线（领域产出**多父**合并提交）。
    Merge { id: String },
    /// 放弃：只翻转记录, **绝不**删除, 也不产生提交。
    Reject { id: String },
}

impl ProposalDecision {
    /// 这次决定作用在哪条提案上。
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Merge { id } | Self::Reject { id } => id,
        }
    }
}

/// 抽屉的**来源**：有进程内权威时是那句柄, 没有时**什么都没有**。
///
/// 它存在是为了让"有没有控制面"这件事**不是**散在调用点上的 `cfg`：
/// [`Self::rows()`] 在默认构建里恒为空, [`Self::resolve()`] 恒拒绝, 因此宿主只有
/// 一条注入路径与一条写路径（⛔ 没有"feature 开着走 A、关着走 B"的两份界面行为）。
#[derive(Clone, Debug, Default)]
pub struct ProposalSource {
    #[cfg(feature = "in-process-mcp")]
    authority: Option<ProjectAuthorityHandle>,
}

/// 领域侧的一行 → 界面侧的一行（**唯一**的转换点）。
#[cfg(feature = "in-process-mcp")]
fn row_view_of(row: yeban_mcp::domain::proposal::ProposalRow) -> ProposalRowView {
    ProposalRowView {
        id: row.id,
        kind: row.kind.to_owned(),
        title: row.title,
        op_count: row.op_count,
    }
}

/// 领域 `Fault` → 界面上那一句**逐字**原因。
///
/// 与 `crate::undo` 的 `refusal_from_fault` 同一取向（那里是私有函数, 所以这里
/// **重新 match 一次**而不是把它提成公共口 —— 两种界面各自决定要上屏什么, 共享的
/// 只有"原文照传"这条规则）。`Fault` 实测只有两臂, ⛔ 没有第三份消息可拼。
#[cfg(feature = "in-process-mcp")]
fn fault_message(fault: &Fault) -> String {
    match fault {
        Fault::Domain { message, .. } => message.clone(),
        Fault::Impl { error } => format!("{error:?}"),
    }
}

impl ProposalSource {
    /// **没有**来源（默认构建、判据装配、没挂控制面的产品运行）。
    #[must_use]
    pub fn none() -> Self {
        Self {
            #[cfg(feature = "in-process-mcp")]
            authority: None,
        }
    }

    /// 接在唯一可变权威上。
    #[cfg(feature = "in-process-mcp")]
    #[must_use]
    pub fn from_authority(authority: &ProjectAuthorityHandle) -> Self {
        Self {
            authority: Some(authority.clone()),
        }
    }

    /// 当前**未合并**的提案清单（权威顺序；没有来源 ⇒ 空）。
    #[must_use]
    pub fn rows(&self) -> Vec<ProposalRowView> {
        #[cfg(feature = "in-process-mcp")]
        {
            self.authority
                .as_ref()
                .map(|authority| {
                    authority
                        .open_proposal_rows()
                        .into_iter()
                        .map(row_view_of)
                        .collect()
                })
                .unwrap_or_default()
        }
        #[cfg(not(feature = "in-process-mcp"))]
        {
            Vec::new()
        }
    }

    /// 这个装配**有没有**提案的来源（决定空态该说哪句话, 也决定按钮画不画）。
    ///
    /// `true` 而清单为空 = "控制面活着, 但此刻确实没有未合并的提案"（真话）；
    /// `false` = "这个构建/装配根本没有提案来源"（另一句真话）。两者**不能**共用
    /// 一句话, 否则"没有提案"与"看不见提案"在界面上就成了同一件事。
    #[must_use]
    pub fn has_source(&self) -> bool {
        #[cfg(feature = "in-process-mcp")]
        {
            self.authority.is_some()
        }
        #[cfg(not(feature = "in-process-mcp"))]
        {
            false
        }
    }

    /// **写面**：把一次采纳 / 放弃交给唯一可变权威。
    ///
    /// 这里没有一个字是"实现"：唯一的动作是把身份装进 `HostAction` 的对应臂, 然后
    /// 交给 [`ProjectAuthorityHandle::apply_host`] —— 那一个调用点就是界面上**唯一**
    /// 能改变领域状态的地方（与 `Undo` / `Redo` / `Commit` 同一个门、同一个
    /// `apply_revision` 推进点）。
    ///
    /// # Errors
    ///
    /// 返回领域权威的**逐字**拒绝理由（提案不存在 / 已拒绝 / 已合并 / 非法身份 /
    /// op 施加失败）。没有来源的装配返回 [`NO_WRITE_SOURCE_REASON`] —— 它**不是**
    /// 一个被吞掉的按键, 而是一条会重新注入到抽屉上的陈述。
    #[cfg(feature = "in-process-mcp")]
    pub fn resolve(&self, decision: &ProposalDecision) -> Result<(), String> {
        use yeban_mcp::domain::HostAction;

        let Some(authority) = self.authority.as_ref() else {
            return Err(NO_WRITE_SOURCE_REASON.to_owned());
        };
        let action = match decision {
            ProposalDecision::Merge { id } => HostAction::MergeProposal {
                proposal_id: id.clone(),
                // `None` ⇒ 领域自己的缺省口径（与 `yeban_merge_proposal` 同一个 `merge_plan`）。
                commit_message: None,
            },
            ProposalDecision::Reject { id } => HostAction::RejectProposal {
                proposal_id: id.clone(),
                reason: None,
            },
        };
        // `HostOutcome` 的读数**不上屏**（本 crate 不接日志面, 且 `steps` 可能是 0 而
        // 这不是失败 —— 判据 `a_second_host_merge_is_an_idempotent_no_step`）：界面上
        // 唯一的反馈是"重新读到的清单"，那一行没了就是它真的没了。
        match authority.apply_host(action) {
            Ok(_) => Ok(()),
            Err(fault) => Err(fault_message(&fault)),
        }
    }

    /// 没有来源的装配**永远**拒绝写入（默认构建的腿）。
    #[cfg(not(feature = "in-process-mcp"))]
    pub fn resolve(&self, _decision: &ProposalDecision) -> Result<(), String> {
        Err(NO_WRITE_SOURCE_REASON.to_owned())
    }
}

/// 空态文案①：**没有来源**（默认构建 / 没挂控制面）。
///
/// 逐字写在这里, 界面上那句可见文案与 `accessible-label` 都取它 —— ⛔ 不在 `.slint`
/// 里再抄一份（抄一份就会漂移）。
pub const NO_SOURCE_REASON: &str = "这个装配没有接进程内的提案来源，因此界面看不到 AI 提案。";

/// 空态文案②：**有来源, 但此刻没有未合并的提案**。
pub const NO_OPEN_PROPOSAL_REASON: &str = "控制面此刻没有未合并的 AI 提案。";

/// 写面**用不了**的原因①：这个装配没有能改领域的门。
///
/// 与 [`NO_SOURCE_REASON`] 是两句话（读不到 vs 写不动）, 但在这种装配里**同时**成立
/// —— 所以 `resolve-reason` 拿到的就是这句, 而按钮由 `resolve-available` 挡住不画。
pub const NO_WRITE_SOURCE_REASON: &str =
    "这个装配没有接进程内的提案来源，界面上没有采纳 / 放弃的写入口。";

/// 写面**用不了**的原因②：有来源, 但还没点选要处理哪一行。
///
/// ⚠ 领域的 `yeban_merge_proposal` / `HostAction::MergeProposal` 都要一个明确的
/// `proposalId`；"把清单里所有提案一次采纳"不是一个被实现、被省略的功能, 而是一个
/// **从未被提出**的语义（合并顺序、冲突处理都要人裁决）⇒ 界面不许替用户猜一个身份。
pub const NO_SELECTION_REASON: &str =
    "先点选一行提案，「采纳 / 放弃」才会亮起：领域的合并与拒绝都要一个明确的 proposalId。";

/// 一次注入的**结果**：抽屉数据面九个属性的值, 从清单 + 选中 + 上一次拒绝算出来。
///
/// 把它做成一个函数而不是让宿主各写各的，是为了让"四个数组**等长**"与"选中身份必须
/// 在当前清单里"这两件事只有一处实现（下面循环按下标同时读数组, 长度不等就会画到数组
/// 外面去；陈旧选中不夹紧就会把 `proposalId` 写给一个已经不存在的提案）。
///
/// * `selection` —— 抽屉此刻的选中身份（界面属性读回来的原文）；
/// * `refusal` —— 上一次写入被权威拒绝的**逐字**理由（没有就传 `None`）。
#[must_use]
pub fn drawer_values(
    rows: &[ProposalRowView],
    has_source: bool,
    selection: &str,
    refusal: Option<&str>,
) -> DrawerValues {
    // ⭐ 夹紧：选中身份**必须**是当前清单里某一行的 `id`, 否则写面拿到的 `proposalId`
    // 指向一条看不见 / 已处理的提案。与 `wire_clip_scope` / 侧栏分类的夹紧同一取向
    // （界面给的身份, 宿主负责验, 验不过就当没选）。
    let selected = if rows.iter().any(|row| row.id == selection) {
        selection.to_owned()
    } else {
        String::new()
    };
    let resolve_available = has_source && !selected.is_empty() && refusal.is_none();
    let resolve_reason = match refusal {
        // 拒绝理由**优先**：那是权威刚刚说过的话, 一个字都不许改。
        Some(message) => message.to_owned(),
        None if !has_source => NO_WRITE_SOURCE_REASON.to_owned(),
        None if selected.is_empty() => NO_SELECTION_REASON.to_owned(),
        None => String::new(),
    };
    DrawerValues {
        count: i32::try_from(rows.len()).unwrap_or(i32::MAX),
        ids: rows.iter().map(|row| row.id.clone()).collect(),
        kinds: rows.iter().map(|row| row.kind.clone()).collect(),
        titles: rows.iter().map(|row| row.title.clone()).collect(),
        op_counts: rows.iter().map(|row| i32_from(row.op_count)).collect(),
        selected,
        empty_reason: (if has_source {
            NO_OPEN_PROPOSAL_REASON
        } else {
            NO_SOURCE_REASON
        })
        .to_owned(),
        resolve_available,
        resolve_reason,
    }
}

/// `usize → i32` 的**唯一**夹紧点（Slint 的数组模型用 `i32` 长度）。
#[must_use]
fn i32_from(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// 抽屉数据面的一次完整读数（宿主**唯一**的写者拿它写九个属性）。
#[derive(Clone, Debug)]
pub struct DrawerValues {
    /// 提案条数（= 下面四个数组的长度）。
    pub count: i32,
    /// 每条的身份（canonical ULID）—— 写面唯一接受的 `proposalId` 的来源。
    pub ids: Vec<String>,
    /// 每条的类别。
    pub kinds: Vec<String>,
    /// 每条的标题。
    pub titles: Vec<String>,
    /// 每条携带的 op 条数（与上面三个数组**等长**）。
    pub op_counts: Vec<i32>,
    /// 夹紧后的选中身份（不在当前清单里 ⇒ 空串, ⛔ 不保留陈旧身份）。
    pub selected: String,
    /// 空态那句话（`has_source` 决定是哪一句）。
    pub empty_reason: String,
    /// 「采纳 / 放弃」此刻**能不能**用（有来源 ∧ 有选中 ∧ 上一次没被拒）。
    pub resolve_available: bool,
    /// 不能用时**逐字**写出的原因（可用时界面上不画它）。
    pub resolve_reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<ProposalRowView> {
        vec![
            ProposalRowView {
                id: "01J000000000000000000000AA".to_owned(),
                kind: "section".to_owned(),
                title: "来一段 16 小节的副歌".to_owned(),
                op_count: 7,
            },
            ProposalRowView {
                id: "01J000000000000000000000BB".to_owned(),
                kind: "notes".to_owned(),
                title: "加一层八度".to_owned(),
                op_count: 1,
            },
        ]
    }

    /// **等长是硬约束**：抽屉的循环按下标同时读四个数组, 少一格就会画到数组外面。
    #[test]
    fn drawer_values_keeps_the_four_arrays_the_same_length() {
        let values = drawer_values(&rows(), true, "", None);
        assert_eq!(values.count, 2);
        assert_eq!(values.ids.len(), 2);
        assert_eq!(values.kinds.len(), 2);
        assert_eq!(values.titles.len(), 2);
        assert_eq!(values.op_counts.len(), 2);
        assert_eq!(
            values.op_counts,
            vec![7, 1],
            "op 条数必须逐条对应, 不许合并"
        );
        assert_eq!(values.ids[0], "01J000000000000000000000AA");
        assert_eq!(values.kinds[0], "section");
        assert_eq!(values.titles[1], "加一层八度");
    }

    /// 空态是**两句**真话, 不是一句：「没有来源」与「有来源但没有提案」在界面上必须分开。
    #[test]
    fn empty_reason_distinguishes_no_source_from_no_proposal() {
        assert_eq!(
            drawer_values(&[], false, "", None).empty_reason,
            NO_SOURCE_REASON
        );
        assert_eq!(
            drawer_values(&[], true, "", None).empty_reason,
            NO_OPEN_PROPOSAL_REASON
        );
        assert_ne!(NO_SOURCE_REASON, NO_OPEN_PROPOSAL_REASON);
        // 有提案时空态**不上屏**（`visible` 由条数决定）, 但读数本身仍然如实给。
        assert_eq!(
            drawer_values(&rows(), true, "", None).empty_reason,
            NO_OPEN_PROPOSAL_REASON
        );
    }

    /// 「采纳 / 放弃」要**同时**满足三件事才亮：有来源 ∧ 选中当前清单里的一行 ∧ 没有未解释的拒绝。
    #[test]
    fn resolve_available_needs_a_source_and_a_row_of_the_current_list() {
        let all = rows();
        // ① 有来源但没选 ⇒ 不亮, 并给出"先点选"那句。
        let unselected = drawer_values(&all, true, "", None);
        assert!(!unselected.resolve_available);
        assert_eq!(unselected.resolve_reason, NO_SELECTION_REASON);
        // ② 选中当前清单的一行 ⇒ 亮, 且**不**留任何理由文案。
        let picked = drawer_values(&all, true, &all[1].id, None);
        assert!(picked.resolve_available);
        assert_eq!(picked.selected, all[1].id);
        assert_eq!(picked.resolve_reason, "");
        // ③ 没有来源 ⇒ 永远不亮, 说的是"没有写入口"那句。
        let no_source = drawer_values(&all, false, &all[0].id, None);
        assert!(!no_source.resolve_available);
        assert_eq!(no_source.resolve_reason, NO_WRITE_SOURCE_REASON);
        assert_ne!(NO_WRITE_SOURCE_REASON, NO_SELECTION_REASON);
    }

    /// **陈旧选中必须夹成"没选"**：采纳之后那一行会从清单里消失, 而属性里的旧身份还在。
    ///
    /// 不夹紧的后果是"下一次按键把一个已经不存在的 `proposalId` 写给权威" —— 那既不是
    /// 用户看到的行, 也不是权威里的提案, 只会换来一句用户无法对账的拒绝。
    #[test]
    fn a_stale_selection_is_clamped_to_none_and_never_written() {
        let all = rows();
        let stale = drawer_values(&all[..1], true, &all[1].id, None);
        assert_eq!(
            stale.selected, "",
            "选中身份不在当前清单里就必须当没选, ⛔ 原样保留"
        );
        assert!(!stale.resolve_available);
        assert_eq!(stale.resolve_reason, NO_SELECTION_REASON);
        // 空串本身也不是一个身份：没选时不许把空串写给权威。
        let blank = vec![ProposalRowView {
            id: String::new(),
            kind: "macro".to_owned(),
            title: "把宏 0 推到 0.6".to_owned(),
            op_count: 1,
        }];
        assert!(
            !drawer_values(&blank, true, "", None).resolve_available,
            "没选中的时候, 空串不许被当成一个可写的身份"
        );
    }

    /// 三种"不能用"各自说自己的话 —— 不许共用一句, 否则用户分不出自己缺了哪一步。
    #[test]
    fn each_refusal_says_its_own_thing() {
        let all = rows();
        let reasons = [
            drawer_values(&all, false, "", None).resolve_reason,
            drawer_values(&all, true, "", None).resolve_reason,
            drawer_values(&all, true, &all[0].id, Some("提案不存在")).resolve_reason,
        ];
        for (i, a) in reasons.iter().enumerate() {
            for (j, b) in reasons.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "第 {i} 与第 {j} 种拒绝说了同一句话");
                }
            }
        }
        assert_eq!(
            reasons[2], "提案不存在",
            "权威的逐字拒绝必须原样上屏, ⛔ 不改写不缩写"
        );
    }

    /// 默认构建里**没有**来源 ⇒ 读为空 **且** 写被拒（两个面各说一次同一件事）。
    #[test]
    fn a_source_without_an_authority_reads_empty_and_refuses_writes() {
        let source = ProposalSource::none();
        assert!(!source.has_source());
        assert!(source.rows().is_empty());
        let decision = ProposalDecision::Merge {
            id: "01J000000000000000000000AA".to_owned(),
        };
        let error = source
            .resolve(&decision)
            .expect_err("没有写入口的装配不许返回成功");
        assert_eq!(error, NO_WRITE_SOURCE_REASON);
    }

    /// 决定**只带身份**：缺省文案的口径住在领域侧, 界面侧多一个字段就会与工具面漂移。
    #[test]
    fn a_decision_carries_only_the_identity() {
        let merge = ProposalDecision::Merge {
            id: "01J000000000000000000000AA".to_owned(),
        };
        assert_eq!(merge.id(), "01J000000000000000000000AA");
        let reject = ProposalDecision::Reject {
            id: "01J000000000000000000000BB".to_owned(),
        };
        assert_eq!(reject.id(), "01J000000000000000000000BB");
        let rendered = format!("{merge:?}");
        for forbidden in ["message", "reason"] {
            assert!(
                !rendered.to_lowercase().contains(forbidden),
                "决定里出现了界面自造的缺省文案字段 `{forbidden}`: {rendered}"
            );
        }
    }

    /// 界面侧的提案形状**只有**这四格：任何"置信度 / 轨道号 / 小节号"都是领域里
    /// 不存在的读数（`confidence` 在 `yeban-mcp` / `yeban-model` / `yeban-theory`
    /// 实测 0 处命中）, 出现在这里就是编造。
    #[test]
    fn a_row_carries_exactly_the_four_drawn_fields() {
        let row = ProposalRowView {
            id: "01J000000000000000000000CC".to_owned(),
            kind: "macro".to_owned(),
            title: "把宏 0 推到 0.6".to_owned(),
            op_count: 2,
        };
        let rendered = format!("{row:?}");
        for forbidden in ["%", "confidence", "置信度", "track", "measure"] {
            assert!(
                !rendered.to_lowercase().contains(forbidden),
                "界面侧的提案形状里出现了领域不存在的读数 `{forbidden}`: {rendered}"
            );
        }
        assert_eq!(row.op_count, 2, "op 条数是**真实条数**, 不是下标推出来的数");
    }
}
