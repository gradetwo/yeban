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

// 每个测试目标用到的成员不同（有的只用 `answering_open`，有的还要 `answering_save`）
// ⇒ 未用到的不是死代码，是**另一个目标**的入口。
#![allow(dead_code)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use yeban_app::file_picker::{FilePicker, PickerRequest};

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
