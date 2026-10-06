//! **用户在界面上按的"保存工程"** —— 一个入口，两条落点（`ROAD-M4-008` 选项 (a)）。
//!
//! ## 这个模块要收掉的那个缺口
//!
//! `docs/ledger/m4-008-authority-notes.md` §9.5 第 4 条如实登记过：
//!
//! > 生产 `run_gui` 今天仍不构造保存执行面（`src/live_surface.rs` 只在测试目标里
//! > `#[path]` 装入 ⇒ 产品二进制里没有 `ui/force_save` 这条 UI 命令）
//!
//! 那是一处"**保存 UI**"的缺口，不是写者边界的缺口：能力（`ProjectAuthorityHandle::save_to`、
//! [`save_project_file`]）早已判据化，只是**用户够不到**。本模块把"保存"从测试目标的装配里
//! 拿出来，变成一个**产品路径上的普通函数**（[`dispatch_save`]），由生产窗口的回调
//! （[`crate::host::wire_save`]）调用。
//!
//! ## 为什么是"一个函数"而不是"把 `live_surface.rs` 搬进产品路径"
//!
//! `src/live_surface.rs` 需要 `yeban-ui-mcp` 与 `yeban-ui-test-port`，而这两个在
//! `crates/yeban-app/Cargo.toml` 里都是 **dev-dependency**（`AGENTS.md` §2 红线 6；
//! 那个文件的文件头已写明）。要把它挪进 `[dependencies]` 就得新增**两条产品依赖边 +
//! 一个非默认 feature**，并把整套 `LivePort` / `PortAdapter` / `ControlPlane` 装配拉进
//! 发行图 —— 那是一个大重构，而它换来的能力（"按一下保存"）只需要下面这几十行。
//! 因此选**小**：产品路径自己拿 [`dispatch_save`]，落点与测试装配**逐字相同**
//! （同一个 `save_to`、同一个 [`save_project_file`]），依赖图一位不变。
//!
//! ## 三个情形的落点（这就是本模块的全部策略）
//!
//! | 情形 | 落点 | 为什么 |
//! | :--- | :--- | :--- |
//! | 挂了控制面且会话**可写**（生产 `SessionSource::WritableFile`） | `ProjectAuthorityHandle::save_to` —— 字节由那个 `Domain` 自己产出（工程 + 提交图谱 + 资产池），走唯一原子入口 | 那份会话是这份文档的**唯一**磁盘写者；宿主的本地路径在它持排他锁期间**必然**被拒，所以本地**不能**是这里的落点 |
//! | 挂了控制面但会话**只读**（`SessionSource::File` / 内存样本） | **拒绝**（[`SaveOutcome::Failed`]），**不回退**本地路径 | 回退会把"权威说不能写"偷偷换成一个仍在写的分支（选项 (c) 被拒的形态）。诚实做法是如实报出原因 |
//! | **没有**控制面（默认构建 / 运行期开关关着 / 拿不到锁没挂上） | [`save_project_file`]（本地原子写 + 取 `.yeban.lock` 的排他写建议锁） | 进程里只有它一个写者，本地路径就是那条路；`--save-as` 今天就走它 |
//!
//! 三条都由**同一个函数**给出，因此不存在"某条保存路径忘了走权威"的形态：
//! 只要 `authority` 是 `Some`，本地路径那里就**不可达**（不是"也调一下"）。
//!
//! ## 它**不**做什么（刻意的）
//!
//! - **不**碰 `apply_revision`：保存不改工程内容，宿主保存动作在 `yeban-mcp` 侧不经过
//!   `Plan` / `apply`（推进点仍只有 `apply` 一处，见 `m4-008-authority-notes.md` §9.2）；
//! - **不**新增端口 / 令牌 / 通道 / feature；`in-process-mcp` 关着时这个模块只是
//!   "本地原子写 + 一句如实回执"（默认依赖树里 `yeban-mcp` 的命中数仍为 0）；
//! - **不**吞掉任何失败：三种失败各有各的文本，界面必须把它们念出来。

use std::path::PathBuf;

use yeban_model::project::YebanProjectV1;

use crate::save::{SaveError, save_project_file};

#[cfg(feature = "in-process-mcp")]
use crate::mcp_mount::ProjectAuthorityHandle;
#[cfg(feature = "in-process-mcp")]
use yeban_mcp::domain::error::Fault;

/// 一个**永远不可能被构造**的类型：`in-process-mcp` 关着时"控制面权威"不存在
/// （没有 `yeban-mcp` 依赖边）。用它把 [`dispatch_save`] 的签名收成**一份**
/// —— 不复制两套策略代码。
#[cfg(not(feature = "in-process-mcp"))]
#[derive(Debug, Clone, Copy)]
pub enum AuthorityHandle {}

/// `dispatch_save` 的权威参数类型：有 feature 时是那个句柄，没有时是上面的占位类型。
#[cfg(feature = "in-process-mcp")]
type AuthorityParam<'a> = Option<&'a ProjectAuthorityHandle>;
/// `dispatch_save` 的权威参数类型（默认构建：永远是 `None`）。
#[cfg(not(feature = "in-process-mcp"))]
type AuthorityParam<'a> = Option<&'a AuthorityHandle>;

/// 一次保存请求：**目标路径** + "若没有权威时用哪一份工程"。
///
/// 为什么目标路径是 `Option<PathBuf>` 而不是 `PathBuf`：`.yeban` 的"内存样本"
/// （`--project-sample`，或没给 `--open`）**没有磁盘对应物**，因此它没有"保存到哪"。
/// 那时如实报 [`SaveOutcome::Failed`]（"未配置保存路径"），而不是猜一个文件名。
#[derive(Debug, Clone)]
pub struct SaveRequest {
    /// 保存目标。`None` = 这个会话没有磁盘对应物。
    pub target: Option<PathBuf>,
    /// GUI 当前持有的工程 —— **只在没有权威时**使用（有权威时字节由权威自己产出）。
    pub project: YebanProjectV1,
}

impl SaveRequest {
    /// 有目标路径的请求。
    #[must_use]
    pub fn new(target: PathBuf, project: YebanProjectV1) -> Self {
        Self {
            target: Some(target),
            project,
        }
    }
}

/// 一次用户保存的结果（**界面拿它渲染状态文本**，不是内部日志）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveOutcome {
    /// 真的落盘了：`bytes` 字节写到 `path`。
    Saved {
        /// 最终落点。
        path: PathBuf,
        /// 写出的容器字节数。
        bytes: usize,
        /// `true` = 走的是控制面会话（`ProjectAuthorityHandle::save_to`）；
        /// `false` = 走的是本地原子写（[`save_project_file`]）。
        ///
        /// 这条读数存在的理由与 `ui/force_save` 回执里的 `writtenByAuthority` 相同：
        /// "这条保存走了哪条路"必须是**可断言的**，而不是注释里的声明。
        by_authority: bool,
    },
    /// 拒绝了 / 失败了，且**什么都没写**。`stage` 是本模块自己的分类（判据与界面都读它）。
    Failed {
        /// 失败发生在哪一步（本地写 / 权威会话 / 没有目标）。
        stage: SaveStage,
        /// 给用户看的完整原因。
        message: String,
    },
}

impl SaveOutcome {
    /// 这次保存真的落盘了吗。
    #[must_use]
    pub const fn succeeded(&self) -> bool {
        matches!(self, Self::Saved { .. })
    }
}

/// [`SaveOutcome::Failed`] 发生在哪一步。
///
/// 它**不是**给用户看的（用户看 `message`），而是给判据与日志用的稳定标签 ——
/// 于是"被权威的门拒"与"被 `.yeban.lock` 拒"不会在断言里混成一句字符串匹配。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveStage {
    /// 这个会话没有磁盘对应物（`.yeban` 内存样本）⇒ 不猜文件名。
    NoTarget,
    /// 权威会话拒绝了（只读形态 / 落盘失败）。
    Authority,
    /// 本地原子写拒绝了（`.yeban.lock` 被占 / 容器被拒 / I/O）。
    Local,
}

/// **用户按保存**的唯一下发点：按"谁是写者"把这次请求送到唯一正确的落点。
///
/// `authority` 为 `Some` 时**只**调 `ProjectAuthorityHandle::save_to`，
/// 无论它成功还是失败都**不**回退到本地路径（见模块文档的三个情形表）。
/// 因此"界面上按保存"与"AI 发 `yeban_save_project`"在挂载时是**同一个**写会话。
///
/// 与 `live_surface.rs` 的 `LiveAdminSurface::save_now` 是同一套策略的**两个装配**
/// （那边是控制面执行面，这里是生产窗口回调）；两处的落点函数逐字相同，
/// 界面回执的字段名也刻意对齐（`bytes` / `by_authority` ↔ `bytes` / `writtenByAuthority`）。
#[must_use]
#[cfg_attr(not(feature = "in-process-mcp"), allow(unused_variables))]
pub fn dispatch_save(request: &SaveRequest, authority: AuthorityParam<'_>) -> SaveOutcome {
    let Some(target) = request.target.as_deref() else {
        return SaveOutcome::Failed {
            stage: SaveStage::NoTarget,
            message: "未配置保存路径（这个会话没有磁盘对应物；`--save-as <path>` \
                      可以把当前工程另存为一个容器）"
                .to_owned(),
        };
    };
    #[cfg(feature = "in-process-mcp")]
    if let Some(authority) = authority {
        // 只读形态在这里就**如实拒绝**，而不是先试一次再报错：`is_writable()` 与
        // `save_to` 读的是同一个 `Domain::is_read_only()`，而前者能给出更准的原因。
        // 关键是**不回退**：这条分支无论结果如何都不碰本地路径。
        if !authority.is_writable() {
            return SaveOutcome::Failed {
                stage: SaveStage::Authority,
                message: format!(
                    "保存被拒：控制面会话以**只读**形态挂载（`SessionSource::File` / 内存样本），\
                     宿主保存动作与 `yeban_save_project` 走同一个 `read_only` 门 ⇒ \
                     一个字节都没写；不会回退到本地路径（那会造出第二个写者）。目标 {}",
                    target.display()
                ),
            };
        }
        return match authority.save_to(target) {
            Ok(saved) => SaveOutcome::Saved {
                path: saved.path,
                bytes: saved.bytes,
                by_authority: true,
            },
            Err(fault) => SaveOutcome::Failed {
                stage: SaveStage::Authority,
                message: format!(
                    "保存被权威会话拒绝（目标 {}）: {}",
                    target.display(),
                    render_fault(&fault)
                ),
            },
        };
    }
    // 没有权威 ⇒ 本地原子写（唯一写者的本地形态）。它自己取 `.yeban.lock` 排他写建议锁。
    match save_project_file(&request.project, target) {
        Ok(report) => SaveOutcome::Saved {
            path: report.path,
            bytes: report.bytes,
            by_authority: false,
        },
        Err(error) => SaveOutcome::Failed {
            stage: SaveStage::Local,
            message: format!(
                "本地保存被拒（目标 {}）: {}",
                target.display(),
                render_local_error(&error)
            ),
        },
    }
}

/// 把 `.yeban.lock` 的拒绝原因原样转成人话。
///
/// **`SaveError::Locked` 必须点名"谁占着、占的是什么模式"** —— 只说"保存失败"会让
/// "另一个形态正开着这份工程"变成一个查不出的现象（这正是本判据要证的那条）。
fn render_local_error(error: &SaveError) -> String {
    match error {
        SaveError::Locked(hold) => format!(
            "`{}` 被 `.yeban.lock` 建议锁占用 ⇒ 拒绝写入（锁文件 {}，持有者 {}，模式 {}）。\
             另一个 yeban 形态正开着这份工程时这是**正确**行为：请等它退出，\
             或从那个会话里保存（挂载了控制面的 GUI 保存会走那个会话）",
            hold.path.display(),
            hold.lock_file.display(),
            hold.holder,
            hold.holder_mode
                .clone()
                .unwrap_or_else(|| "未知（元数据读不到）".to_owned())
        ),
        other => other.to_string(),
    }
}

/// `Fault` 的两种出口各自如实转达（领域失败给人话 + 契约错误码；实现级状况原样带出）。
#[cfg(feature = "in-process-mcp")]
fn render_fault(fault: &Fault) -> String {
    match fault {
        Fault::Domain {
            code,
            message,
            data,
        } => match data {
            Some(data) => format!("[{code:?}] {message} ({data})"),
            None => format!("[{code:?}] {message}"),
        },
        Fault::Impl { error } => format!("实现级状况: {error:?}"),
    }
}
