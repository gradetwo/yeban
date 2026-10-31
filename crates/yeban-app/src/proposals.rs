//! AI 提案抽屉的**读面** —— 界面的提案清单唯一的来源 (`[UI-A11Y-004]` / §10 Musical PR)。
//!
//! ## 这一格补的是哪一个洞（实测, 不是转述）
//!
//! `ui/dialogs/musical_pr_drawer.slint` 的四个数据面属性今天**没有任何写者**：
//! 抽屉因此永远画"0 条提案 + 一句空态"。而领域侧**有**这些提案 —— 一次
//! `yeban_propose_section` / `yeban_edit_notes` / `yeban_set_macro` 就在
//! [`Domain::proposals`] 里留下一条 `Open` 记录（会话运行态, `MODEL-ISO-001` 第 2 层）。
//! 于是"AI 提了案, 界面上看不见"是**两侧都真、中间断开**的缺口。
//!
//! 本模块只做**读**：`Domain → ProposalRowView → 抽屉属性`。
//!
//! ## ⛔ 不发明第二份权威（结构性约束, 不是口号）
//!
//! * 提案记录**只**从 [`ProjectAuthorityHandle`] 的只读口 `open_proposal_rows()` 取
//!   （它经 `HttpServer::host_domain`, 签名里只有 `&Domain` —— 与
//!   `project()` / `undo_display()` / `graph()` 同一条边界, 不是新开的第二扇门）；
//! * **顺序**不在这里定义：排序口径住在 `Domain::open_proposals`（工具面的
//!   `yeban_list_proposals` 用的是**同一个**函数）, 抄第二遍就会产生"工具说第 1 条是
//!   A、界面把 A 画在第 2 行"的漂移, 而从任何一侧单独看都发现不了；
//! * 本模块**不**推进施加修订号、不写提案状态 —— 这里没有一个能改状态的调用。
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

#[cfg(feature = "in-process-mcp")]
use crate::mcp_mount::ProjectAuthorityHandle;

/// 一条提案在抽屉里要画的那三格（**唯一**的界面侧提案形状）。
///
/// 它是 `crates/yeban-mcp/src/domain/proposal.rs` 的 `ProposalRow` 的**界面侧投影**：
/// 与 [`crate::undo`] 对 `UndoDisplay` 做同一件事的理由相同 —— `yeban-app` 的默认
/// 依赖图**不含** `yeban-mcp`（`MUST-GATE-009`）, 所以默认构建里能编译的形态只能是
/// 宿主自己的类型。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalRowView {
    /// 提案类别（`section` / `notes` / `macro`）—— 领域记录的原文。
    pub kind: String,
    /// 一句话标题 —— 领域记录的原文（⛔ 界面不拼第二份文案）。
    pub title: String,
    /// 本提案携带的 op 条数（> 0；一次提案是一个 op 批次）。
    pub op_count: usize,
}

/// 抽屉的**来源**：有进程内权威时是那句柄, 没有时**什么都没有**。
///
/// 它存在是为了让"有没有控制面"这件事**不是**散在调用点上的 `cfg`：
/// [`Self::rows()`] 在默认构建里恒为空, 因此宿主只有一条注入路径
/// （⛔ 没有"feature 开着走 A、关着走 B"的两份界面行为）。
#[derive(Clone, Debug, Default)]
pub struct ProposalSource {
    #[cfg(feature = "in-process-mcp")]
    authority: Option<ProjectAuthorityHandle>,
}

/// 领域侧的一行 → 界面侧的一行（**唯一**的转换点）。
#[cfg(feature = "in-process-mcp")]
fn row_view_of(row: yeban_mcp::domain::proposal::ProposalRow) -> ProposalRowView {
    ProposalRowView {
        kind: row.kind.to_owned(),
        title: row.title,
        op_count: row.op_count,
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

    /// 这个装配**有没有**提案的来源（决定空态该说哪句话）。
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
}

/// 空态文案①：**没有来源**（默认构建 / 没挂控制面）。
///
/// 逐字写在这里, 界面上那句可见文案与 `accessible-label` 都取它 —— ⛔ 不在 `.slint`
/// 里再抄一份（抄一份就会漂移）。
pub const NO_SOURCE_REASON: &str = "这个装配没有接进程内的提案来源，因此界面看不到 AI 提案。";

/// 空态文案②：**有来源, 但此刻没有未合并的提案**。
pub const NO_OPEN_PROPOSAL_REASON: &str = "控制面此刻没有未合并的 AI 提案。";

/// 「采纳 / 放弃」为什么**画不出来**的逐字原因（`resolve-available == false` 时上屏）。
///
/// 这条边界不是"忘了接"：领域侧的 `yeban_merge_proposal` / `yeban_reject_proposal`
/// 要 `proposalId`, 而 `HostAction` 今天只有 `Undo` / `Redo` / `Commit` 三臂 —— 界面
/// 侧**没有**合并提案的写入口。同一件事的键盘面（`Shift+Enter → 采纳 AI 建议`）在
/// `src/cli.rs` 的快捷表里也是 `implemented: false`。
/// 因此这里给的是**一句真话**, 而不是一个按下去只关抽屉的假按钮。
pub const RESOLVE_NOT_WIRED_REASON: &str = "提案已列出，但「采纳 / 放弃」尚未接线：界面上没有合并提案的写入口（领域的 `yeban_merge_proposal` 要 proposalId）。";

/// 一次注入的**结果**：抽屉数据面六个属性的值, 从清单算出来。
///
/// 把它做成一个函数而不是让宿主各写各的，是为了让"条数与三个数组**等长**"这件事
/// 只有一处实现（下面循环按下标同时读三个数组, 长度不等就会画到数组外面去）。
#[must_use]
pub fn drawer_values(rows: &[ProposalRowView], has_source: bool) -> DrawerValues {
    DrawerValues {
        count: i32::try_from(rows.len()).unwrap_or(i32::MAX),
        kinds: rows.iter().map(|row| row.kind.clone()).collect(),
        titles: rows.iter().map(|row| row.title.clone()).collect(),
        op_counts: rows.iter().map(|row| i32_from(row.op_count)).collect(),
        empty_reason: (if has_source {
            NO_OPEN_PROPOSAL_REASON
        } else {
            NO_SOURCE_REASON
        })
        .to_owned(),
        resolve_available: false,
        resolve_reason: RESOLVE_NOT_WIRED_REASON.to_owned(),
    }
}

/// `usize → i32` 的**唯一**夹紧点（Slint 的数组模型用 `i32` 长度）。
#[must_use]
fn i32_from(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// 抽屉数据面的一次完整读数（宿主**唯一**的写者拿它写六个属性）。
#[derive(Clone, Debug)]
pub struct DrawerValues {
    /// 提案条数（= 三个数组的长度）。
    pub count: i32,
    /// 每条的类别。
    pub kinds: Vec<String>,
    /// 每条的标题。
    pub titles: Vec<String>,
    /// 每条携带的 op 条数（与上面两个数组**等长**）。
    pub op_counts: Vec<i32>,
    /// 空态那句话（`has_source` 决定是哪一句）。
    pub empty_reason: String,
    /// 「采纳 / 放弃」此刻**能不能**用。
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
                kind: "section".to_owned(),
                title: "来一段 16 小节的副歌".to_owned(),
                op_count: 7,
            },
            ProposalRowView {
                kind: "notes".to_owned(),
                title: "加一层八度".to_owned(),
                op_count: 1,
            },
        ]
    }

    /// **等长是硬约束**：抽屉的循环按下标同时读三个数组, 少一格就会画到数组外面。
    #[test]
    fn drawer_values_keeps_the_three_arrays_the_same_length() {
        let values = drawer_values(&rows(), true);
        assert_eq!(values.count, 2);
        assert_eq!(values.kinds.len(), 2);
        assert_eq!(values.titles.len(), 2);
        assert_eq!(values.op_counts.len(), 2);
        assert_eq!(
            values.op_counts,
            vec![7, 1],
            "op 条数必须逐条对应, 不许合并"
        );
        assert_eq!(values.kinds[0], "section");
        assert_eq!(values.titles[1], "加一层八度");
    }

    /// 空态是**两句**真话, 不是一句：「没有来源」与「有来源但没有提案」在界面上必须分开。
    #[test]
    fn empty_reason_distinguishes_no_source_from_no_proposal() {
        assert_eq!(drawer_values(&[], false).empty_reason, NO_SOURCE_REASON);
        assert_eq!(
            drawer_values(&[], true).empty_reason,
            NO_OPEN_PROPOSAL_REASON
        );
        assert_ne!(NO_SOURCE_REASON, NO_OPEN_PROPOSAL_REASON);
        // 有提案时空态**不上屏**（`visible` 由条数决定）, 但读数本身仍然如实给。
        assert_eq!(
            drawer_values(&rows(), true).empty_reason,
            NO_OPEN_PROPOSAL_REASON
        );
    }

    /// 「采纳 / 放弃」今天**必须**是 `false` + 一句逐字原因。
    ///
    /// 这条断言钉的是"不许假装实现"：如果哪天有人把 `resolve_available` 写成 `true`
    /// 却没有真的接上合并, 这里先红。（真接上时改这条判据, 与那次改动一起进同一个提交。）
    #[test]
    fn resolve_stays_unavailable_until_a_write_path_exists() {
        let values = drawer_values(&rows(), true);
        assert!(
            !values.resolve_available,
            "`HostAction` 今天只有 Undo / Redo / Commit 三臂, 界面侧没有合并提案的写入口"
        );
        assert_eq!(values.resolve_reason, RESOLVE_NOT_WIRED_REASON);
        assert!(
            RESOLVE_NOT_WIRED_REASON.contains("未接线"),
            "原因必须是一句用户读得懂的话, 不是内部术语"
        );
    }

    /// 默认构建里**没有**来源 ⇒ `rows()` 恒空（这一条在两种 feature 下都必须成立，
    /// 而 feature 打开时它读的是权威的实时清单, 判据另有 `live_surface` 一条）。
    #[test]
    fn a_source_without_an_authority_reads_empty() {
        let source = ProposalSource::none();
        assert!(!source.has_source());
        assert!(source.rows().is_empty());
    }

    /// 界面侧的提案形状**只有**这三格：任何"置信度 / 轨道号 / 小节号"都是领域里
    /// 不存在的读数（`confidence` 在 `yeban-mcp` / `yeban-model` / `yeban-theory`
    /// 实测 0 处命中）, 出现在这里就是编造。
    #[test]
    fn a_row_carries_exactly_the_three_drawn_fields() {
        let row = ProposalRowView {
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
