//! **原生文件对话框的唯一能力面**（任务单 `#86` **乙**）。
//!
//! ## 为什么是 trait，而不是在调用点直接调 `rfd`
//!
//! ⭐ **可注入性是这一条的判据面**：真弹窗在 CI 上起不来（没有窗口服务器；就算有，
//! 它会**阻塞在用户输入上**直到有人点）。因此"挑一个路径"这件事在本仓**只有一处**
//! 能力面 —— [`FilePicker`]：
//!
//! | 装配 | 实现 | 谁用 |
//! | :--- | :--- | :--- |
//! | 产品 | [`NativeFilePicker`]（`rfd`，系统原生对话框） | `main.rs` 的 `run_gui` |
//! | 判据 | **测试自己写的**假实现（记下每次请求、按脚本回一个路径） | `tests/*.rs` |
//!
//! 于是判据能断言的正是本单要求的那两句：**"对话框被调用了"**（假的实现记下了请求）
//! 与**"选了路径 ⇒ 模型被提交"**（同一个假实现回一条路径，然后读模型 / 界面读数）。
//! ⛔ 判据**从不**声称"我验证了真弹窗" —— 那需要真人（见本文件的边界登记）。
//!
//! ## 目录 / 后缀这类"业务知识"住在**调用方**，不住在这里
//!
//! 本模块⛔ 不认识 `.yeban` / `.sfz` / `.wav`：那是各调用方的语义（`file_picker`
//! 只是"给我一个路径"的通道）。调用方用 [`PickerRequest`] 把标题 / 建议文件名 /
//! 后缀过滤**表达**出来，本模块只负责把它翻译成对话框。
//!
//! ## 两种结局都必须能表达
//!
//! [`FilePicker::choose_open`] / [`FilePicker::choose_save`] 返回 `Option<PathBuf>`：
//! `None` = **用户取消**（或平台起不来对话框）。取消**不是错误** ⇒ 调用方
//! **什么都不做**，⛔ 不把它写成"失败"报给用户（那是把"我不想"和"出错了"混成一句）。
//!
//! ## 三条边界（如实登记）
//!
//! 1. **本模块不验证路径**：不查存在性、不查可写、不建目录。那是 `open.rs` /
//!    `save.rs` 的各自契约（它们**已经**有那些检查与错误分类）。
//! 2. **`rfd` 的对话框是阻塞式的**（`pick_file()` 在 UI 线程上开一个**模态**原生窗口）。
//!    本仓把它接在**用户按下按钮的那一跳**上（不是在 60Hz 心跳里），因此"阻塞主循环"
//!    的语义就是"模态对话框本来就是模态的" —— ⛔ 与 `#86` 甲那条"⛔ 不阻塞主循环"
//!    是两件事（甲写的是一个属性，乙开的是一个用户主动唤起的窗口）。
//! 3. ⚠ **未被判据覆盖**：真弹窗的外观 / 平台行为 / 取消按钮的语义。
//!    判据覆盖的是**本仓这一层**（请求构造得对不对、拿到的路径有没有被提交）。

use std::path::{Path, PathBuf};

/// 一次"挑路径"的**请求** —— 调用方表达它要什么，本模块负责把它翻成对话框。
///
/// 为什么是一个结构体而不是四个参数：判据要能**逐字断言**这次请求
/// （标题对不对、后缀过滤对不对），四个裸参数会让"假实现记下了什么"变成一串位置参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerRequest {
    /// 对话框标题（用户可见）。
    pub title: String,
    /// 建议文件名（`save` 那一条用；`open` 那条通常是 `None`）。
    pub file_name: Option<String>,
    /// 后缀过滤（**不带点**，例如 `["yeban"]` / `["sfz"]`）。空 = 不过滤。
    ///
    /// ⚠ 空 `Vec` 与"没有过滤"是同一件事（不是"过滤掉所有东西"）。
    pub extensions: Vec<String>,
    /// 起始目录（`None` = 由平台决定）。
    pub directory: Option<PathBuf>,
}

impl PickerRequest {
    /// 一条**不过滤**的"打开"请求（调用方随后用 [`Self::with_extensions`] 补过滤）。
    #[must_use]
    pub fn open(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            file_name: None,
            extensions: Vec::new(),
            directory: None,
        }
    }

    /// 一条"保存"请求（带建议文件名）。
    #[must_use]
    pub fn save(title: impl Into<String>, file_name: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            file_name: Some(file_name.into()),
            extensions: Vec::new(),
            directory: None,
        }
    }

    /// 补上后缀过滤（**不带点**）。
    #[must_use]
    pub fn with_extensions<I, S>(mut self, extensions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.extensions = extensions.into_iter().map(Into::into).collect();
        self
    }

    /// 补上起始目录。
    #[must_use]
    pub fn in_directory(mut self, directory: impl AsRef<Path>) -> Self {
        self.directory = Some(directory.as_ref().to_path_buf());
        self
    }
}

/// ⭐ **「挑一个路径」的唯一能力面**（判据靠替换它来测"对话框被调用了"）。
///
/// 两个方法都是 `&self`（⛔ 不是 `&mut self`）：调用点从 `.slint` 的回调闭包里够到它，
/// 而那些闭包是 `Fn`（不是 `FnMut`）。需要记事的实现自己拿 `RefCell`。
pub trait FilePicker {
    /// 挑一个**已存在**的文件（打开工程 / 选 SFZ）。`None` = 用户取消。
    fn choose_open(&self, request: &PickerRequest) -> Option<PathBuf>;

    /// 挑一个**要写**的路径（保存工程 / 另存为）。`None` = 用户取消。
    fn choose_save(&self, request: &PickerRequest) -> Option<PathBuf>;
}

/// **产品实现**：`rfd`（系统原生对话框）。
///
/// ## 为什么默认 feature 就够（⛔ 不需要 `gtk3`）
///
/// 实测 `cargo tree -p yeban-app -e normal --locked`：本 crate 只多 `rfd` 自己一个包；
/// `objc2*` 族早已被 slint/winit 带进依赖图。Linux 上起的是 `xdg-portal` 那条腿
/// （默认 feature），⛔ 不拖 GTK。
///
/// ⚠ **`rfd` 的对话框是模态且阻塞的**：它跑在**调用它的那个线程**（UI 线程）上。
/// 这与"用户按下按钮"的语义一致，但意味着一件事 —— ⛔ **绝不在心跳里调它**。
#[derive(Debug, Clone, Copy, Default)]
pub struct NativeFilePicker;

impl FilePicker for NativeFilePicker {
    fn choose_open(&self, request: &PickerRequest) -> Option<PathBuf> {
        build(request).pick_file()
    }

    fn choose_save(&self, request: &PickerRequest) -> Option<PathBuf> {
        build(request).save_file()
    }
}

/// [`PickerRequest`] ⇒ `rfd::FileDialog`（**唯一**一处翻译，两个方法共用）。
fn build(request: &PickerRequest) -> rfd::FileDialog {
    let mut dialog = rfd::FileDialog::new().set_title(request.title.clone());
    if let Some(name) = request.file_name.as_deref() {
        dialog = dialog.set_file_name(name);
    }
    if let Some(directory) = request.directory.as_deref() {
        dialog = dialog.set_directory(directory);
    }
    // ⚠ 空过滤 = 不过滤（`add_filter` 传空数组会让某些平台**什么都选不了**，
    // 那是"看起来接了、其实用不了"的一种）⇒ 空的时候⛔ 不加过滤器。
    if !request.extensions.is_empty() {
        let extensions: Vec<&str> = request.extensions.iter().map(String::as_str).collect();
        dialog = dialog.add_filter(request.title.clone(), &extensions);
    }
    dialog
}

/// **永远不挑**的实现（两个方法都回 `None`）—— 语义就是**用户每次都取消**。
///
/// 谁用它：
/// * 既有的装配/判据里"保存路径在**装配期**就定好了"的那些（`--open` / `--save`）——
///   它们一次都不该弹窗，本实现让那句承诺**可执行**（弹了就会拿到 `None`，行为不变）；
/// * 无头装配（没有窗口服务器时"打开对话框"只能取消）。
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPicker;

impl FilePicker for NoPicker {
    fn choose_open(&self, _request: &PickerRequest) -> Option<PathBuf> {
        None
    }

    fn choose_save(&self, _request: &PickerRequest) -> Option<PathBuf> {
        None
    }
}

/// 用户挑的路径**没有后缀**时补上 `extension`（带不带点都行）。
///
/// 为什么需要它（实测的平台差异）：`rfd` 的 `add_filter` 在 macOS 上会替用户补后缀，
/// 在别的平台**不一定** ⇒ "用户在对话框里敲了 `demo`、结果写出一个没有后缀的文件"
/// 会变成一条只在某些平台上出现的缺陷。本函数把这条口径**钉在一个地方**，
/// 于是"挑出来的路径 ⇒ 写盘"在任何平台上都是同一个形状。
///
/// ⛔ **已经有后缀就一位不改**（用户敲了 `.txt` 就写 `.txt` —— 那是他的选择，
/// 不是本层该纠正的错）。
#[must_use]
pub fn with_default_extension(path: PathBuf, extension: &str) -> PathBuf {
    if path.extension().is_some() {
        return path;
    }
    let extension = extension.trim_start_matches('.');
    path.with_extension(extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 请求构造必须**逐字**保留调用方说的话（标题 / 建议名 / 后缀 / 目录）。
    ///
    /// ⚠ 本判据**不碰** `rfd`：它只钉住"我们这层把请求拼对了"。
    /// "真弹窗长什么样"是 [`NativeFilePicker`] 的边界（见模块文档的第 3 条）。
    #[test]
    fn a_request_keeps_what_the_caller_said() {
        let open = PickerRequest::open("打开工程")
            .with_extensions(["yeban"])
            .in_directory("/tmp");
        assert_eq!(open.title, "打开工程");
        assert_eq!(open.file_name, None, "打开那一条不给建议文件名");
        assert_eq!(open.extensions, vec!["yeban".to_owned()]);
        assert_eq!(open.directory.as_deref(), Some(Path::new("/tmp")));

        let save = PickerRequest::save("保存工程", "demo.yeban");
        assert_eq!(save.file_name.as_deref(), Some("demo.yeban"));
        assert!(
            save.extensions.is_empty(),
            "没点名过滤就是**不过滤**（⛔ 不是「过滤掉所有东西」）"
        );
        assert!(save.directory.is_none(), "没点名目录就交给平台");
    }

    /// ⭐ [`NativeFilePicker`] 必须**真的存在并且实现了 [`FilePicker`]** ——
    /// 这条是"产品腿接上了 `rfd`"的编译期证据（`fn` 项与 `fn` 指针同款用法）。
    ///
    /// ⚠ 它**不**开对话框（那会阻塞在用户输入上）⇒ 判据只取它的函数项。
    #[test]
    fn the_native_picker_is_a_file_picker() {
        let picker = NativeFilePicker;
        let _open: fn(&NativeFilePicker, &PickerRequest) -> Option<PathBuf> =
            <NativeFilePicker as FilePicker>::choose_open;
        let _save: fn(&NativeFilePicker, &PickerRequest) -> Option<PathBuf> =
            <NativeFilePicker as FilePicker>::choose_save;
        // 只证明"这一层能被当作 `dyn FilePicker` 交出去"（调用点就是这么拿它的）。
        let _dynamic: &dyn FilePicker = &picker;
    }
}
