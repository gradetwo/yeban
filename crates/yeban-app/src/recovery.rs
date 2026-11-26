//! 旁挂恢复文件（T-FIX-739）：自动保存/异常退出后的恢复入口。
//!
//! ## 它补的是哪一格
//!
//! `save.rs` 的 `ARCH-SEC-004` 只保证**一次显式保存**的原子性：旧文件不会被写坏，
//! 但自上次保存以来的编辑没有任何磁盘副本。本模块在**不动保存主路径**的前提下，
//! 按固定间隔把活会话（工程 + `history.dag` + 资产池）写成一个旁挂 `.recover`
//! 容器，启动时检测并给出恢复入口；正常保存/退出按规则删除它。
//!
//! ## 三条硬约束
//!
//! 1. **不新造第二套工程序列化**：内容由 `save::save_archive_parts_file` 写出，
//!    与用户按「保存」走的是同一条 `write_project_container_*` / 原子写实现。
//! 2. **默认不误报**：只在工程指纹相对上次基线变了才写；正常保存会 `discard`，
//!    并递增 [`note_saved`] 让自动保存重新取基线，避免刚存完又生成一个"恢复文件"。
//! 3. **失效不冒充可用**：主文件比旁挂文件新，或旁挂文件不是 ZIP 头，都返回
//!    `None` / `Invalid`，绝不把坏文件当成"可恢复"。
//!
//! ## 边界（如实）
//!
//! * 旁挂文件与主文件同目录，名字是 `<主文件>.recover`；主文件不存在（未命名工程）
//!   时没有旁挂位置，自动保存直接跳过。
//! * 启动提示走 `save-status`（屏读器可达）；恢复入口是 `yeban-app --open <recover>`
//!   —— 它就是一个合法的 `.yeban` 容器，打开后另存为原文件即可。
//! * 自动保存间隔内（默认 20 s）的编辑仍可能丢；本单不承诺零丢失。

use std::cell::{Cell, RefCell};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use slint::ComponentHandle;
use yeban_model::YebanProjectV1;

use crate::undo::UndoPort;

/// 旁挂文件的后缀（接在主文件名后，不替换 `.yeban`）。
pub const RECOVERY_SUFFIX: &str = ".recover";

/// 自动保存间隔：20 秒。录音中跳过（避免与 take 落盘抢同一份资产池）。
pub const AUTOSAVE_INTERVAL: Duration = Duration::from_millis(20_000);

/// 检测结果。`Invalid` 是**响亮**的"存在但不可用"，⛔ 不混成 `None`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryState {
    /// 没有旁挂文件。
    None,
    /// 有可用的恢复副本。
    Available {
        /// 旁挂恢复文件的路径。
        sidecar: PathBuf,
    },
    /// 有旁挂文件，但读不出/不是容器：保留现场并响亮报告。
    Invalid {
        /// 旁挂恢复文件的路径。
        sidecar: PathBuf,
        /// 为什么读不出（给用户看的完整原因）。
        reason: String,
    },
}

/// 主文件对应的旁挂文件路径（`foo.yeban` → `foo.yeban.recover`）。
#[must_use]
pub fn sidecar_path(target: &Path) -> PathBuf {
    let mut name = target.as_os_str().to_os_string();
    name.push(RECOVERY_SUFFIX);
    PathBuf::from(name)
}

/// 检查主文件旁是否有可用的恢复副本。
///
/// 规则：
/// * 旁挂文件不存在 ⇒ [`RecoveryState::None`]；
/// * 旁挂文件为空 / 不是 ZIP 头 ⇒ [`RecoveryState::Invalid`]（保留现场）；
/// * 主文件存在且不旧于旁挂文件 ⇒ 旁挂文件已失效，删除并返回 `None`；
/// * 其余 ⇒ [`RecoveryState::Available`]。
#[must_use]
pub fn detect(target: &Path) -> RecoveryState {
    let sidecar = sidecar_path(target);
    let metadata = match std::fs::metadata(&sidecar) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return RecoveryState::None;
        }
        Err(error) => {
            return RecoveryState::Invalid {
                sidecar,
                reason: format!("读不到恢复文件的元信息：{error}"),
            };
        }
    };
    if metadata.len() == 0 {
        return RecoveryState::Invalid {
            sidecar,
            reason: "恢复文件是 0 字节".to_owned(),
        };
    }
    if !has_zip_header(&sidecar) {
        return RecoveryState::Invalid {
            sidecar,
            reason: "恢复文件不是 ZIP 容器（可能写到一半就中断）".to_owned(),
        };
    }
    if let Ok(main) = std::fs::metadata(target)
        && let (Ok(main_time), Ok(sidecar_time)) = (main.modified(), metadata.modified())
        && main_time >= sidecar_time
    {
        // 主文件不比旁挂旧 ⇒ 正常保存/外部替换已经发生，旁挂失效。
        let _ = std::fs::remove_file(&sidecar);
        return RecoveryState::None;
    }
    RecoveryState::Available { sidecar }
}

/// 启动时可读的恢复提示；没有旁挂文件时返回 `None`。
///
/// 提示里同时给**恢复入口**（`--open <recover>`）与"原文件未被改写"这一事实，
/// 避免用户以为启动已经自动恢复了。
#[must_use]
pub fn startup_notice(target: &Path) -> Option<String> {
    match detect(target) {
        RecoveryState::None => None,
        RecoveryState::Available { sidecar } => Some(format!(
            "检测到未保存的恢复副本：{} · 恢复入口：运行 `yeban-app --open {}` 打开它，再另存为原文件；当前启动加载的仍是原文件，未被覆盖。",
            sidecar.display(),
            sidecar.display()
        )),
        RecoveryState::Invalid { sidecar, reason } => Some(format!(
            "检测到恢复文件但无法读取：{}（{reason}） · 已保留现场，未自动删除；请手工检查或删除它。",
            sidecar.display()
        )),
    }
}

/// 把活会话写进旁挂恢复文件。序列化完全复用 [`crate::save::save_archive_parts_file`]。
///
/// # Errors
///
/// 返回给用户看的原因（容器写出失败、资产哈希不匹配、I/O/锁失败）。
pub fn write_snapshot(
    target: &Path,
    project: &YebanProjectV1,
    history_dag: &[u8],
    assets: &[(yeban_model::AssetHash, Vec<u8>)],
) -> Result<PathBuf, String> {
    let sidecar = sidecar_path(target);
    crate::save::save_archive_parts_file(project, history_dag, assets, None, &[], &sidecar)
        .map(|_report| sidecar)
        .map_err(|error| format!("恢复快照写入失败：{error}"))
}

/// 删除旁挂文件（正常保存/正常退出）。不存在不算错。
pub fn discard(target: &Path) {
    let sidecar = sidecar_path(target);
    match std::fs::remove_file(&sidecar) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            // 清理失败必须响亮：留下的旧快照会在下次启动误报。
            eprintln!(
                "yeban-app: 正常保存后未能删除恢复文件 {}（{error}）；下次启动可能仍提示恢复",
                sidecar.display()
            );
        }
    }
}

thread_local! {
    /// 成功保存的代际。自动保存每跳比对它：变了 ⇒ 只重取基线，⛔ 不立刻再写旁挂。
    static SAVE_GENERATION: Cell<u64> = const { Cell::new(0) };
    /// 自动保存定时器（进程级一条；窗口销毁后回调自行退出）。
    static AUTOSAVE_TIMER: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
}

/// 任何一次**成功保存**后调用：让自动保存下一跳重取基线，并删除旁挂文件。
pub fn note_saved(target: &Path) {
    SAVE_GENERATION.with(|generation| generation.set(generation.get() + 1));
    discard(target);
}

/// 启动自动保存：固定间隔检查活工程指纹，变了才写旁挂文件。
///
/// 目标路径优先取「另存为」记住的路径，其次取装配期目标；两者都没有（未命名工程）
/// 时跳过。录音中跳过。写失败把原因留在 `save-status`，⛔ 不静默。
pub fn start_autosave(
    ui: &crate::ui::MainWindow,
    live_port: Option<Rc<UndoPort>>,
    chosen_target: Rc<RefCell<Option<PathBuf>>>,
    startup_target: Option<PathBuf>,
    project_fallback: Rc<RefCell<YebanProjectV1>>,
) {
    let weak = ui.as_weak();
    let baseline = Rc::new(RefCell::new(current_fingerprint(
        live_port.as_ref(),
        &project_fallback,
    )));
    let last_generation = Rc::new(Cell::new(SAVE_GENERATION.with(Cell::get)));
    AUTOSAVE_TIMER.with(|slot| {
        if slot.borrow().is_some() {
            return;
        }
        let timer = slint::Timer::default();
        timer.start(slint::TimerMode::Repeated, AUTOSAVE_INTERVAL, move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            // 录音中不写：take 还在录音器里，资产池随时会变；保存路径本身
            // 在 T-FIX-738 已拒绝，自动保存也不该制造半个 take。
            if crate::host::recording_in_progress() {
                return;
            }
            let target = chosen_target
                .borrow()
                .clone()
                .or_else(|| startup_target.clone());
            let Some(target) = target else {
                return;
            };
            let generation = SAVE_GENERATION.with(Cell::get);
            let Some((project, fingerprint)) =
                current_project(live_port.as_ref(), &project_fallback)
            else {
                return;
            };
            if generation != last_generation.get() {
                // 刚成功保存过：磁盘上就是这份工程，重取基线即可。
                *baseline.borrow_mut() = Some(fingerprint);
                last_generation.set(generation);
                return;
            }
            if baseline.borrow().as_deref() == Some(fingerprint.as_str()) {
                return;
            }
            let Some(live) = live_port.as_ref() else {
                return;
            };
            let graph = live.graph();
            let history_dag = yeban_model::commit::encode_history_dag(&graph);
            let assets = live.assets();
            match write_snapshot(&target, &project, &history_dag, &assets) {
                Ok(_) => {
                    *baseline.borrow_mut() = Some(fingerprint);
                }
                Err(message) => {
                    ui.set_save_status(message.into());
                    ui.set_save_succeeded(false);
                }
            }
        });
        *slot.borrow_mut() = Some(timer);
    });
}

fn current_project(
    live_port: Option<&Rc<UndoPort>>,
    fallback: &Rc<RefCell<YebanProjectV1>>,
) -> Option<(YebanProjectV1, String)> {
    let project = match live_port.and_then(|live| live.try_project()) {
        Some(project) => project,
        None => fallback.borrow().clone(),
    };
    let fingerprint = crate::undo::project_fingerprint(&project).ok()?;
    Some((project, fingerprint))
}

fn current_fingerprint(
    live_port: Option<&Rc<UndoPort>>,
    fallback: &Rc<RefCell<YebanProjectV1>>,
) -> Option<String> {
    current_project(live_port, fallback).map(|(_, fingerprint)| fingerprint)
}

fn has_zip_header(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; 4];
    file.read_exact(&mut header).is_ok() && header == [b'P', b'K', 3, 4]
}
