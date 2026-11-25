//! 帮助系统的宿主应用层：把手册 / 速查 / 引导 / 课程四块的数据面写进
//! `MainWindow` 属性，并接住界面上报的回调。由 `host::wire_help_system`
//! 注册，命令入口（`help-manual` 等四条）也在 host 的命令分发里落到这里。
//!
//! 状态纪律：引导与课程状态以 `help_state` 的落盘文件为跨启动的事实源，
//! 每次动作都是「读 → 改 → 写」一轮；落盘失败只降级为本次会话不持久化，⛔ 不挡界面——但失败
//! 必须被用户看见（T-FIX-311）：写失败时按落点走 `report_notice` 错误级
//! 常驻提示或课程反馈区，⛔ 不许只打 stderr 就当没发生。
//!
//! T-FIX-328 修订：落盘失败时磁盘是旧的，进行中的课只在内存里；若每次
//! 动作仍从磁盘重读，`check_current_step` 会因磁盘上没有 active_lesson
//! 而直接 return，步永远推不上去。所以本模块另记一份会话态
//! （[`SESSION_STATE`]，按设置文件路径分键）：改状态的入口每次都把改后
//! 状态记进来，读状态时有缓存就以缓存为准。同一进程内 HelpState 的唯一
//! 写入口就是本模块的三处 save，磁盘不可能比缓存新（成功落盘后两者一
//! 致）；进程外改设置文件需重启才生效。UI 线程调用，不进实时音频回调。

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Mutex;

use slint::{ModelRc, SharedString, VecModel};
use yeban_model::ids::EntityId;

use crate::cli::Language;
use crate::help_state::{self, ActiveLesson, HelpState};
use crate::lessons::{self, Lesson};
use crate::manual;
use crate::ui::{HelpShortcutRow, LessonRow, MainWindow, ManualLine as UiManualLine};
use crate::undo::UndoPort;

// --------------------------------------------------------------------- 手册

/// 打开用户手册：写入章节目录并呈现当前章（首次打开是第一章）。
pub fn open_manual(ui: &MainWindow) {
    let titles: Vec<SharedString> = manual::CHAPTERS
        .iter()
        .map(|chapter| SharedString::from(chapter.title))
        .collect();
    ui.set_manual_chapters(ModelRc::new(VecModel::from(titles)));
    apply_manual_chapter(ui, ui.get_manual_chapter_index());
    ui.set_help_manual_open(true);
}

/// 呈现某一章（下标夹紧到合法范围）。
pub fn apply_manual_chapter(ui: &MainWindow, index: i32) {
    let index = usize::try_from(index)
        .unwrap_or(0)
        .min(manual::CHAPTERS.len() - 1);
    let chapter = &manual::CHAPTERS[index];
    let lines: Vec<UiManualLine> = manual::parse_chapter(chapter.markdown)
        .into_iter()
        .map(|line| UiManualLine {
            text: SharedString::from(line.text.as_str()),
            level: i32::from(line.level),
        })
        .collect();
    ui.set_manual_lines(ModelRc::new(VecModel::from(lines)));
    ui.set_manual_chapter_index(i32::try_from(index).unwrap_or(0));
}

/// 关闭用户手册。
pub fn close_manual(ui: &MainWindow) {
    ui.set_help_manual_open(false);
}

// --------------------------------------------------------------------- 速查

/// 打开快捷键速查：行数据现算自 `cli::shortcut_rows()`（与
/// `--print-shortcuts` 同一张策略表），⛔ 不手抄第二份键位表。
pub fn open_shortcuts(ui: &MainWindow) {
    let rows: Vec<HelpShortcutRow> = crate::cli::shortcut_rows()
        .iter()
        .map(|row| {
            let (keys, action) = split_shortcut_label(row.label);
            HelpShortcutRow {
                keys: SharedString::from(keys.as_str()),
                action: SharedString::from(action.as_str()),
                status: SharedString::from(if row.implemented { "" } else { "暂未落地" }),
            }
        })
        .collect();
    ui.set_help_shortcut_rows(ModelRc::new(VecModel::from(rows)));
    ui.set_help_shortcuts_open(true);
}

/// 关闭快捷键速查。
pub fn close_shortcuts(ui: &MainWindow) {
    ui.set_help_shortcuts_open(false);
}

/// 把策略表的规范写法 `Cmd/Ctrl+S → 保存工程` 拆成（键位， 说明），
/// 并按当前平台折叠 `Cmd/Ctrl+`（与 cli 的 `--print-shortcuts` 渲染同口径）。
fn split_shortcut_label(label: &str) -> (String, String) {
    let single = if cfg!(target_os = "macos") {
        "Cmd+"
    } else {
        "Ctrl+"
    };
    let folded = label.replace("Cmd/Ctrl+", single);
    match folded.split_once(" → ") {
        Some((keys, action)) => (keys.trim().to_owned(), action.trim().to_owned()),
        None => (folded, String::new()),
    }
}

// --------------------------------------------------------------------- 引导

/// 导览的一步：讲哪个真实区域、切步时要不要先把界面导航过去。
struct TourStep {
    title: &'static str,
    body: &'static str,
    /// 卡片落位区（见 onboarding_overlay.slint 的 card-zone）。
    zone: i32,
    /// 切到这一步时先执行的既有命令（真导航，不做假热点）。
    command: Option<&'static str>,
    /// 是否确保左侧检查器可见（不可见才发 toggle，⛔ 不盲目翻转）。
    ensure_inspector: bool,
}

fn tour_steps() -> Vec<TourStep> {
    vec![
        TourStep {
            title: "欢迎来到夜半",
            body: "夜半是一款 AI 原生的编曲工作站：中间排轨道、底部写音符与混音、左边挑音色。这一圈导览带你把五个主要区域认一遍，每一步都会把界面切到正讲的那一区。随时可以跳过；之后想再看，从「帮助 → 新手引导」重开。",
            zone: 0,
            command: None,
            ensure_inspector: false,
        },
        TourStep {
            title: "走带栏：播放与速度",
            body: "窗口最上方是走带栏：播放 / 暂停（空格键）、停止、回到开头、循环开关都在这里，速度（BPM）的显示与敲入也在这一行。现在就可以按一下空格，听听演示工程的声音。",
            zone: 2,
            command: None,
            ensure_inspector: false,
        },
        TourStep {
            title: "轨道区：一切编排发生的地方",
            body: "中间是编曲区：一条轨道一行，片段（Clip）摆在时间轴上。拖动片段改位置，拖边缘改长度，轨道头工具行的最前面是「轨道菜单」。段落标记与小节标尺在编曲区上方。",
            zone: 1,
            command: None,
            ensure_inspector: false,
        },
        TourStep {
            title: "钢琴卷帘：写音符的地方",
            body: "底部第一个页签是钢琴卷帘（按 P 打开）：横轴是时间、纵轴是音高。数字键 2 是绘制工具，点下去就是一个音符。导览已经帮你切到了这一页——往下看就是它。",
            zone: 1,
            command: Some("tab-pianoroll"),
            ensure_inspector: false,
        },
        TourStep {
            title: "混音控制台：推子与效果",
            body: "底部第二个页签是混音控制台（按 X 打开）：每条轨道一个通道条，推子、声像、静音 / 独奏都在这里，轨道的效果链也从这里进。导览已经帮你切到了这一页。",
            zone: 1,
            command: Some("tab-mixer"),
            ensure_inspector: false,
        },
        TourStep {
            title: "检查器与音色库",
            body: "左栏是通道检查器（按 I 显隐）：选中哪条轨道，这里就显示它的通道参数；左栏下方是音色库，预置音色与 SFZ 音色从这里挑。导览已经确保检查器是打开的，看左边就是。",
            zone: 3,
            command: None,
            ensure_inspector: true,
        },
        TourStep {
            title: "导览结束",
            body: "五个区域都认完了。想系统地学，用「帮助 → 教学课程」跟着写出你的第一段旋律；忘了键位看「帮助 → 快捷键速查」；完整说明在「帮助 → 用户手册」里。",
            zone: 0,
            command: None,
            ensure_inspector: false,
        },
    ]
}

/// 打开新手引导（从第一步开始；不改动落盘状态）。
pub fn open_onboarding(ui: &MainWindow) {
    apply_onboarding_step(ui, 0);
    ui.set_onboarding_open(true);
}

/// 首次启动自动打开引导的唯一入口（仅 `main.rs` 的真实装配调用）：
/// 落盘状态说「该弹」（没点过不再显示、没走完过）才弹。返回是否弹了。
#[must_use]
pub fn show_onboarding_if_first_launch(ui: &MainWindow) -> bool {
    if !help_state::load().should_auto_show_onboarding() {
        return false;
    }
    open_onboarding(ui);
    true
}

/// 呈现导览的某一步：先按步导航界面，再写卡片读数。
pub fn apply_onboarding_step(ui: &MainWindow, index: usize) {
    let steps = tour_steps();
    let index = index.min(steps.len() - 1);
    let step = &steps[index];
    if let Some(command) = step.command {
        ui.invoke_command_execute(SharedString::from(command));
    }
    if step.ensure_inspector && !ui.get_show_left_inspector() {
        ui.invoke_command_execute(SharedString::from("toggle-inspector"));
    }
    ui.set_onboarding_step_index(i32::try_from(index).unwrap_or(0));
    ui.set_onboarding_step_count(i32::try_from(steps.len()).unwrap_or(0));
    ui.set_onboarding_step_title(SharedString::from(step.title));
    ui.set_onboarding_step_body(SharedString::from(step.body));
    ui.set_onboarding_card_zone(step.zone);
}

/// 「下一步」：走完最后一步 ⇒ 落盘「已完成」并关层（以后不再自动弹）。
pub fn onboarding_next(ui: &MainWindow) {
    let index = usize::try_from(ui.get_onboarding_step_index()).unwrap_or(0);
    if index + 1 >= tour_steps().len() {
        update_state(ui, |state| state.onboarding_finished = true);
        ui.set_onboarding_open(false);
    } else {
        apply_onboarding_step(ui, index + 1);
    }
}

/// 「上一步」。
pub fn onboarding_back(ui: &MainWindow) {
    let index = usize::try_from(ui.get_onboarding_step_index()).unwrap_or(0);
    if index > 0 {
        apply_onboarding_step(ui, index - 1);
    }
}

/// 「跳过导览」：只关层，不落盘（下次启动还会自动弹）。
pub fn onboarding_skip(ui: &MainWindow) {
    ui.set_onboarding_open(false);
}

/// 「不再显示」：落盘 dismissed 并关层。
pub fn onboarding_dismiss_forever(ui: &MainWindow) {
    update_state(ui, |state| state.onboarding_dismissed = true);
    ui.set_onboarding_open(false);
}

/// 落盘失败的事项闭集（两处调用点各一；T-FIX-447，形态同
/// `settings::SaveFailureSubject`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelpSaveFailureSubject {
    /// 新手引导设置（`update_state`）。
    OnboardingSettings,
    /// 学习进度（开课 / 完成一步）。
    LessonProgress,
}

impl HelpSaveFailureSubject {
    /// 事项在 `lang` 下的称呼（九语闭集，与 `settings.rs` 同款形态）。
    #[must_use]
    fn label_in(self, lang: Language) -> &'static str {
        use Language::{DeDe, EnUs, EsEs, FrFr, JaJp, KoKr, PtBr, RuRu, ZhCn};
        match self {
            Self::OnboardingSettings => match lang {
                ZhCn => "新手引导设置",
                EnUs => "Onboarding settings",
                FrFr => "Paramètres d'accueil",
                EsEs => "Ajustes de incorporación",
                PtBr => "Configurações de integração",
                JaJp => "オンボーディング設定",
                KoKr => "온보딩 설정",
                RuRu => "Параметры ознакомления",
                DeDe => "Onboarding-Einstellungen",
            },
            Self::LessonProgress => match lang {
                ZhCn => "学习进度",
                EnUs => "Lesson progress",
                FrFr => "Progression de la leçon",
                EsEs => "Progreso de la lección",
                PtBr => "Progresso da lição",
                JaJp => "レッスンの進捗",
                KoKr => "수업 진행 상황",
                RuRu => "Прогресс урока",
                DeDe => "Lektionsfortschritt",
            },
        }
    }
}

/// 落盘失败的可见文案（与 [`crate::host::report_notice`] 的错误级口径一致）：
/// 本次已生效、下次启动可能回退——用户必须看见这句话，⛔ 不许只打 stderr。
///
/// T-FIX-447：旧实现拼了一条硬编码中文串，英文态下提示条 / 课程反馈区照样
/// 显示中文。文案是常驻 UI 显示（错误级常驻提示与课程反馈区），按
/// `docs/app-i18n-notes.md` §4 收尾补记（2026-11-17 裁定）须译；形态与
/// T-FIX-432 的设置警告同款——事项与模板按当前语言渲染
/// （[`save_failure_message_in`]），九语模板表住在本文件，由单测逐语看守；
/// 不走 Slint 的 `@tr`/`.po`（`@tr` 是 Slint 编译期宏，Rust 代码调不了；
/// `.po` 里加一条没有 `@tr` 调用点的词条会被 `i18n_coverage` 的词表精确
/// 相等判据当孤儿条打红）。错误原文按 §4 口径不译，原样进占位。
fn save_failure_message(
    subject: HelpSaveFailureSubject,
    error: &help_state::HelpStateError,
) -> String {
    save_failure_message_in(subject, error, crate::host::current_language())
}

/// 在 `lang` 下的完整展示文案。中文态与 T-FIX-311 原文逐字相同（单测钉死）；
/// 错误原文不译，原样进占位。
#[must_use]
fn save_failure_message_in(
    subject: HelpSaveFailureSubject,
    error: &help_state::HelpStateError,
    lang: Language,
) -> String {
    use Language::{DeDe, EnUs, EsEs, FrFr, JaJp, KoKr, PtBr, RuRu, ZhCn};
    let what = subject.label_in(lang);
    match lang {
        ZhCn => format!("{what}未能保存：本次已生效，但下次启动可能回退（{error}）"),
        EnUs => format!(
            "{what} failed to save: it applies to this session, but may roll back on next launch ({error})"
        ),
        FrFr => format!(
            "Échec de l'enregistrement de {what} : appliqué pour cette session, mais il risque de revenir en arrière au prochain démarrage ({error})"
        ),
        EsEs => format!(
            "No se pudo guardar {what}: se aplica en esta sesión, pero podría revertirse en el próximo inicio ({error})"
        ),
        PtBr => format!(
            "Não foi possível salvar {what}: vale para esta sessão, mas poderá ser revertido na próxima inicialização ({error})"
        ),
        JaJp => format!(
            "{what}を保存できませんでした：このセッションでは有効ですが、次回起動時に元に戻る可能性があります（{error}）"
        ),
        KoKr => format!(
            "{what}을(를) 저장하지 못했습니다: 이번 세션에는 적용되지만, 다음 시작 시 되돌려질 수 있습니다({error})"
        ),
        RuRu => format!(
            "Не удалось сохранить {what}: действует в этом сеансе, но при следующем запуске может быть отменено ({error})"
        ),
        DeDe => format!(
            "{what} konnte nicht gespeichert werden: gilt für diese Sitzung, kann beim nächsten Start aber zurückgesetzt werden ({error})"
        ),
    }
}

/// T-FIX-328：本会话内最后一次改过的 [`HelpState`]（按设置文件路径分键）。
/// 见模块头的状态纪律修订：改状态的入口每次都把改后状态记进来。
static SESSION_STATE: Mutex<BTreeMap<PathBuf, HelpState>> = Mutex::new(BTreeMap::new());

fn remember_session_state(state: &HelpState) {
    if let Ok(mut guard) = SESSION_STATE.lock() {
        guard.insert(crate::settings::settings_path(), state.clone());
    }
}

/// 取本会话的 [`HelpState`]：有缓存就以缓存为准（见 [`SESSION_STATE`]），
/// 否则从磁盘读。锁坏了就退回磁盘读，不挡界面。
fn load_session_state() -> HelpState {
    if let Ok(guard) = SESSION_STATE.lock()
        && let Some(cached) = guard.get(&crate::settings::settings_path())
    {
        return cached.clone();
    }
    help_state::load()
}

fn update_state(ui: &MainWindow, edit: impl FnOnce(&mut HelpState)) {
    let mut state = load_session_state();
    edit(&mut state);
    remember_session_state(&state);
    // T-FIX-311：写失败走 report_notice 错误级常驻提示（引导层随即关闭，
    // 课程反馈区看不见，只能走提示条 + toast）；本次仍按改后状态走。
    if let Err(error) = help_state::save(&state) {
        eprintln!("[yeban-app] 新手引导设置落盘失败（下次启动可能回退）: {error}");
        crate::host::report_notice(
            ui,
            &save_failure_message(HelpSaveFailureSubject::OnboardingSettings, &error),
            true,
        );
    }
}

// --------------------------------------------------------------------- 课程

/// 打开教学课程抽屉：刷新课程列表与（若有）进行中课程的当前步。
pub fn open_lessons(ui: &MainWindow, port: &Rc<UndoPort>) {
    refresh_lessons(ui, port);
    ui.set_lessons_open(true);
}

/// 关闭教学课程抽屉（进行中的课与进度都已落盘，关层不丢）。
pub fn close_lessons(ui: &MainWindow) {
    ui.set_lessons_open(false);
}

/// 开始 / 继续一节课。新开课时把当前轨道 id 集合记为判定基准落盘；
/// 继续同一节课沿用原基准（基准一换，之前的活就白做了 ⇒ ⛔ 不重记）。
/// 已完成的课重开 = 从第一步重新来，进度清零、基准重记。
pub fn start_lesson(ui: &MainWindow, port: &Rc<UndoPort>, lesson_id: &str) {
    let Some(lesson) = lessons::lesson(lesson_id) else {
        return;
    };
    let mut state = load_session_state();
    let resuming = state
        .active_lesson
        .as_ref()
        .is_some_and(|active| active.lesson_id == lesson.id)
        && !state.lesson_progress(&lesson.id).finished;
    if !resuming {
        let baseline: Vec<String> = port
            .with_project(|project| {
                project
                    .tracks
                    .keys()
                    .map(EntityId::to_canonical_string)
                    .collect()
            })
            .unwrap_or_default();
        state.lessons.remove(&lesson.id);
        state.active_lesson = Some(ActiveLesson {
            lesson_id: lesson.id.clone(),
            baseline_track_ids: baseline,
            current_step: 0,
        });
    }
    // T-FIX-328：先记会话态——落盘失败时磁盘仍是旧的，后面的刷新与
    // check_current_step 都靠这份内存态找进行中的课。
    remember_session_state(&state);
    // T-FIX-311：写失败不静默——课程反馈区写明本次已生效、下次启动可能
    // 回退；开课本身不失败（进行中课程按内存状态走）。
    match help_state::save(&state) {
        Ok(()) => ui.set_lesson_feedback(SharedString::from("")),
        Err(error) => {
            eprintln!("[yeban-app] 学习进度落盘失败（下次启动可能回退）: {error}");
            ui.set_lesson_feedback(SharedString::from(save_failure_message(
                HelpSaveFailureSubject::LessonProgress,
                &error,
            )));
        }
    }
    refresh_lessons_with_state(ui, port, &state);
}

/// 「完成本步（检查）」：对真实工程跑判定。通过才推进并落盘；
/// 不通过只写反馈（还差什么，数字来自判定器的实测进度），步不推进。
pub fn check_current_step(ui: &MainWindow, port: &Rc<UndoPort>) {
    let mut state = load_session_state();
    let Some(active) = state.active_lesson.clone() else {
        return;
    };
    let Some(lesson) = lessons::lesson(&active.lesson_id) else {
        return;
    };
    let step_index = usize::try_from(active.current_step).unwrap_or(0);
    let Some(step) = lesson.steps.get(step_index) else {
        return;
    };
    let baseline = baseline_set(&active);
    let verdict = port.with_project(|project| lessons::evaluate(&step.check, &baseline, project));
    let Some(verdict) = verdict else {
        ui.set_lesson_feedback(SharedString::from("当前没有打开的工程，无法检查。"));
        return;
    };
    if verdict.done {
        let total = u32::try_from(lesson.steps.len()).unwrap_or(0);
        let next = active.current_step + 1;
        let progress = state.lessons.entry(lesson.id.clone()).or_default();
        progress.steps_done = next.min(total);
        progress.finished = next >= total;
        if next >= total {
            state.active_lesson = None;
        } else if let Some(active) = state.active_lesson.as_mut() {
            active.current_step = next;
        }
        // T-FIX-328：步推进了就记会话态——落盘失败时下一次检查仍靠这份
        // 内存态找进行中的课与当前步。
        remember_session_state(&state);
        // T-FIX-311：写失败不静默——课程反馈区写明本次已生效、下次启动可能
        // 回退；步已在内存里推进（判定过了），不因落盘失败回滚。
        match help_state::save(&state) {
            Ok(()) => ui.set_lesson_feedback(SharedString::from("")),
            Err(error) => {
                eprintln!("[yeban-app] 学习进度落盘失败（下次启动可能回退）: {error}");
                ui.set_lesson_feedback(SharedString::from(save_failure_message(
                    HelpSaveFailureSubject::LessonProgress,
                    &error,
                )));
            }
        }
    } else {
        ui.set_lesson_step_progress(SharedString::from(verdict.progress.as_str()));
        ui.set_lesson_feedback(SharedString::from(format!(
            "还没达到这一步的要求（{}）。照正文再做一做，然后重新检查。",
            verdict.progress
        )));
        return;
    }
    refresh_lessons_with_state(ui, port, &state);
}

/// 返回课程列表：进行中的课保持落盘状态，列表里显示「进行中」。
pub fn exit_lesson(ui: &MainWindow) {
    refresh_lesson_rows(ui, &load_session_state());
    ui.set_lesson_active(false);
}

fn refresh_lessons(ui: &MainWindow, port: &Rc<UndoPort>) {
    refresh_lessons_with_state(ui, port, &load_session_state());
}

/// 与 [`refresh_lessons`] 同款，但以调用方手里的内存态为准，不从磁盘重读。
/// T-FIX-328：落盘失败时磁盘上没有新进度，重读会把刚开的课 / 刚推进的步
/// 在界面上又关掉；start_lesson / check_current_step 必须按内存态刷新。
fn refresh_lessons_with_state(ui: &MainWindow, port: &Rc<UndoPort>, state: &HelpState) {
    refresh_lesson_rows(ui, state);
    match &state.active_lesson {
        Some(active) => match lessons::lesson(&active.lesson_id) {
            Some(lesson) => apply_active_lesson(ui, port, &lesson, active),
            None => ui.set_lesson_active(false),
        },
        None => ui.set_lesson_active(false),
    }
}

fn refresh_lesson_rows(ui: &MainWindow, state: &HelpState) {
    let rows: Vec<LessonRow> = lessons::catalog()
        .iter()
        .map(|lesson| {
            let progress = state.lesson_progress(&lesson.id);
            let status = if progress.finished {
                "已完成".to_owned()
            } else if let Some(active) = &state.active_lesson {
                if active.lesson_id == lesson.id {
                    format!("进行中 · 第 {} 步", active.current_step + 1)
                } else if progress.steps_done > 0 {
                    format!("已过 {} 步", progress.steps_done)
                } else {
                    "未开始".to_owned()
                }
            } else if progress.steps_done > 0 {
                format!("已过 {} 步", progress.steps_done)
            } else {
                "未开始".to_owned()
            };
            LessonRow {
                lesson_id: SharedString::from(lesson.id.as_str()),
                title: SharedString::from(lesson.title.as_str()),
                summary: SharedString::from(lesson.summary.as_str()),
                status: SharedString::from(status.as_str()),
            }
        })
        .collect();
    ui.set_lesson_rows(ModelRc::new(VecModel::from(rows)));
}

fn apply_active_lesson(
    ui: &MainWindow,
    port: &Rc<UndoPort>,
    lesson: &Lesson,
    active: &ActiveLesson,
) {
    let step_index = usize::try_from(active.current_step)
        .unwrap_or(0)
        .min(lesson.steps.len() - 1);
    let step = &lesson.steps[step_index];
    ui.set_lesson_active(true);
    ui.set_lesson_title(SharedString::from(lesson.title.as_str()));
    ui.set_lesson_step_index(i32::try_from(step_index).unwrap_or(0));
    ui.set_lesson_step_count(i32::try_from(lesson.steps.len()).unwrap_or(0));
    ui.set_lesson_step_title(SharedString::from(step.title.as_str()));
    ui.set_lesson_step_body(SharedString::from(step.body.as_str()));
    let baseline = baseline_set(active);
    let progress = port
        .with_project(|project| lessons::evaluate(&step.check, &baseline, project).progress)
        .unwrap_or_else(|| "（当前没有打开的工程）".to_owned());
    ui.set_lesson_step_progress(SharedString::from(progress.as_str()));
}

fn baseline_set(active: &ActiveLesson) -> BTreeSet<EntityId> {
    active
        .baseline_track_ids
        .iter()
        .filter_map(|text| text.parse::<EntityId>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Language;

    /// 固定测试错误：io 原文 ASCII，便于断言错误原文原样进占位。
    fn test_error() -> help_state::HelpStateError {
        help_state::HelpStateError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "disk full (test)",
        ))
    }

    /// 中文态与 T-FIX-311 原文逐字相同（两处调用点各一）。
    #[test]
    fn save_failure_message_zh_cn_is_byte_identical_to_311() {
        let error = test_error();
        assert_eq!(
            save_failure_message_in(
                HelpSaveFailureSubject::OnboardingSettings,
                &error,
                Language::ZhCn
            ),
            "新手引导设置未能保存：本次已生效，但下次启动可能回退（帮助状态落盘失败：disk full (test)）"
        );
        assert_eq!(
            save_failure_message_in(
                HelpSaveFailureSubject::LessonProgress,
                &error,
                Language::ZhCn
            ),
            "学习进度未能保存：本次已生效，但下次启动可能回退（帮助状态落盘失败：disk full (test)）"
        );
    }

    /// 英文态三场景逐条钉死：两事项的模板 + 错误原文原样进占位。
    #[test]
    fn save_failure_message_en_us_renders_english() {
        let error = test_error();
        let onboarding = save_failure_message_in(
            HelpSaveFailureSubject::OnboardingSettings,
            &error,
            Language::EnUs,
        );
        assert_eq!(
            onboarding,
            "Onboarding settings failed to save: it applies to this session, but may roll back on next launch (帮助状态落盘失败：disk full (test))"
        );
        let lesson = save_failure_message_in(
            HelpSaveFailureSubject::LessonProgress,
            &error,
            Language::EnUs,
        );
        assert_eq!(
            lesson,
            "Lesson progress failed to save: it applies to this session, but may roll back on next launch (帮助状态落盘失败：disk full (test))"
        );
    }

    /// 九语全覆盖：每语两事项均有文案；非中文语的模板不含中文模板残片
    /// （「未能保存」「回退」）；错误原文（ASCII 记号）每语原样进占位。
    #[test]
    fn save_failure_message_all_languages_have_no_chinese_template_residue() {
        let error = test_error();
        for lang in Language::ALL {
            for subject in [
                HelpSaveFailureSubject::OnboardingSettings,
                HelpSaveFailureSubject::LessonProgress,
            ] {
                let rendered = save_failure_message_in(subject, &error, lang);
                assert!(
                    rendered.contains(subject.label_in(lang)),
                    "{lang:?} 渲染必须含事项称呼，实际：{rendered}"
                );
                assert!(
                    rendered.contains("disk full (test)"),
                    "{lang:?} 渲染必须原样保留错误原文，实际：{rendered}"
                );
                if lang != Language::ZhCn {
                    assert!(
                        !rendered.contains("未能保存") && !rendered.contains("回退"),
                        "{lang:?} 模板不许含中文残片，实际：{rendered}"
                    );
                }
            }
        }
    }
}
