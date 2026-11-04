//! **判据共享的测试替身**（`tests/` 下的 `mod` 目录不会自成测试目标）。
//!
//! ## 为什么放在这里（而不是每个判据各写一份）
//!
//! [`ScriptedPicker`] 是任务单 `#86` **乙** 建的**假文件选择器**：它记下每一次请求、
//! 按脚本回一个路径。两句话在两个判据文件里都要断：
//!
//! | 判据 | 它断的是 |
//! | :--- | :--- |
//! | `tests/file_picker_dialogs.rs`（`#86` 乙） | 三处**接线**：保存 / 选 SFZ / `open_project_via_picker` |
//! | `tests/open_project_ui.rs`（`#93`） | **运行期「打开工程」的可见入口**（控制台 Tab 1 的按钮） |
//!
//! ⇒ 替身只留**一份**，⛔ 不复制第二份（复制出来的那一份迟早与这一份漂移，而"对话框被问了
//! 几次"这种读数一旦分叉，两条判据就会各说各话）。
//!
//! ## 它⛔ 不碰文件系统、⛔ 不开窗口
//!
//! 这正是"可注入"那句话的落点：判据能断言"**对话框被调用了 N 次**"与"**选了路径 ⇒ 那一份
//! 真的被打开**"，而 ⛔ **不**声称验证过真弹窗的外观 / 平台行为 / 取消按钮的系统语义
//! （那是 `yeban_app::file_picker::NativeFilePicker` 的边界）。
//!
//! ## 本模块的第二位住户（`D-EX-1`）
//!
//! [`wav_export_receipt`] —— `export-wav` 成功回执的**逐字形状**。两个 GUI 导出判据
//! （`tests/gui_export_clipping.rs` / `tests/gui_export_limiter.rs`）都要重建整条回执
//! 做等值断言；形状只留一份，改字段时不会出现"一个文件跟着改、另一个忘改"。

// 每个测试目标用到的成员不同（有的只用 `answering_open`，有的还要 `answering_save`）
// ⇒ 未用到的不是死代码，是**另一个目标**的入口。
#![allow(dead_code)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use yeban_app::export_wav::WavExportReport;
use yeban_app::file_picker::{FilePicker, PickerRequest};

/// ⭐ `D-EX-1`：一条 `export-wav` **成功**回执的逐字形状（**不含** `clipping_note`）。
///
/// ## 为什么它在判据这一侧，还放在共享模块里
///
/// 宿主 `src/host.rs` 的 `fields` 拼装是**第一份**形状；判据必须能**独立**说出"应该
/// 长什么样"，否则它只是在打印宿主的话（那样连字段顺序写反都测不出来）。所以这里
/// 是**第二份** —— 不可避。
///
/// 但只允许有这一份第二份：`tests/gui_export_clipping.rs` 与
/// `tests/gui_export_limiter.rs` 各自都要重建整条回执，各写一份的话，改形状时会
/// 出现"一个文件跟着改、另一个忘改"的漂移（同 `ScriptedPicker` 的立项理由）。
///
/// ## 每个数字都来自报告，⛔ 没有一个是手写的
///
/// 值一律取自同一次运行里**直接调用** `export_wav` 得到的 [`WavExportReport`]
/// （`frames` / `digest` / `true_peak_dbfs` / `limiter_reductions` …）。绝对 `sha256`
/// 与绝对 dB 值都**不**钉常量（机器相关，第 313 条）。
#[must_use]
pub fn wav_export_receipt(report: &WavExportReport, path: &Path, limiter: bool) -> String {
    format!(
        "module:action=export-wav:outcome=ok:path={}:frames={}:sample_rate={}:channels={}:bits={}:bytes={}:source={}:digest={}:limiter={}:limiter_reductions={}:true_peak_dbfs={}:headroom_db={}",
        path.display(),
        report.frames,
        report.sample_rate,
        report.channels,
        report.bits,
        report.bytes,
        report.source,
        report.digest,
        if limiter { "on" } else { "off" },
        report.limiter_reductions,
        report
            .true_peak_dbfs
            .map_or_else(|| "null".to_owned(), |v| format!("{v:.6}")),
        report
            .headroom_db
            .map_or_else(|| "null".to_owned(), |v| format!("{v:.6}")),
    )
}

/// ⭐ **假的选择器**：记下每一次请求，并按脚本回答（`None` = 用户取消）。
pub struct ScriptedPicker {
    /// 每一次请求（`open` 与 `save` 都在这里，按发生顺序）。
    calls: RefCell<Vec<(&'static str, PickerRequest)>>,
    /// `choose_open` 的脚本答案。
    open: RefCell<Option<PathBuf>>,
    /// `choose_save` 的脚本答案。
    save: RefCell<Option<PathBuf>>,
}

impl ScriptedPicker {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            calls: RefCell::new(Vec::new()),
            open: RefCell::new(None),
            save: RefCell::new(None),
        })
    }

    /// 脚本答案 = 这个路径（`choose_open`）。
    pub fn answering_open(path: impl Into<PathBuf>) -> Rc<Self> {
        let picker = Self::new();
        *picker.open.borrow_mut() = Some(path.into());
        picker
    }

    /// 脚本答案 = 这个路径（`choose_save`）。
    pub fn answering_save(path: impl Into<PathBuf>) -> Rc<Self> {
        let picker = Self::new();
        *picker.save.borrow_mut() = Some(path.into());
        picker
    }

    /// **取消**（两个方法都回 `None`）—— 语义就是"用户在对话框里按了取消"。
    pub fn cancelling() -> Rc<Self> {
        Self::new()
    }

    /// 某一类请求被问了几次（`"open"` / `"save"`）。
    pub fn call_count(&self, kind: &str) -> usize {
        self.calls
            .borrow()
            .iter()
            .filter(|(which, _)| *which == kind)
            .count()
    }

    /// 最近一次某一类请求的**原文**（标题 / 建议文件名 / 后缀过滤 / 起始目录）。
    pub fn last_request(&self, kind: &str) -> Option<PickerRequest> {
        self.calls
            .borrow()
            .iter()
            .rev()
            .find(|(which, _)| *which == kind)
            .map(|(_, request)| request.clone())
    }
}

impl FilePicker for ScriptedPicker {
    fn choose_open(&self, request: &PickerRequest) -> Option<PathBuf> {
        self.calls.borrow_mut().push(("open", request.clone()));
        self.open.borrow().clone()
    }

    fn choose_save(&self, request: &PickerRequest) -> Option<PathBuf> {
        self.calls.borrow_mut().push(("save", request.clone()));
        self.save.borrow().clone()
    }
}
