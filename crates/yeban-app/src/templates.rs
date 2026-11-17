//! 工程模板机制 —— 照 Logic Pro 的做法（Wang 2026-11-17 定案）。
//!
//! ## 模板不是新格式
//!
//! Logic 的工程模板就是**工程文件本身**：把一个工程存进专用的模板文件夹，
//! 它就是模板；从模板新建时打开的是**未命名副本**，编辑与保存都落到新工程，
//! **永不回写**模板文件。夜半照此实现，本模块**不发明任何模板格式**：
//! 模板文件就是 `.yeban` 工程容器（[`yeban_model::container::ProjectArchive`]，
//! 经唯一的归档写入口 [`crate::save::save_archive_file`] 落盘、经唯一的读入口
//! [`crate::open::open_project_document_file`] 读回，与普通工程同一条路）。
//!
//! ## 两个模板目录
//!
//! | 目录 | 位置 | 选址理由 |
//! | :--- | :--- | :--- |
//! | 工厂模板 | `<仓库根>/assets/templates/`（[`factory_template_root`]） | 仓内随应用分发的资源都住在仓根 `assets/`（brand / fonts / presets / samples）；仓库根的唯一解析是 [`crate::sfz_load::repo_root`]（SFZ 内置资产同源）。目录可不存在 / 为空 ⇒ 列表如实为空组。 |
//! | 用户模板 | `~/Music/Audio Music Apps/Project Templates/`（[`yeban_model::preset::user_template_library_dir`]） | 与用户预制库 `~/Music/Audio Music Apps/Patches/` **同源同父**（Logic 自家就是把 `Project Templates` 与 `Patches` 并列放在 `Audio Music Apps` 下）；⛔ 不新造一套路径规则。目录不存在时由 [`ensure_user_template_root`] 自动创建。 |
//!
//! ## 三级分类（Wang 2026-11-17 追加定案）
//!
//! 两个根目录之下都按 **曲风 / 子曲风 / 模板名** 三级落盘：
//! `<根>/<曲风>/<子曲风>/<模板名>.yeban`。模板名一级是工程文件本体。
//! 同名判定以**三级路径**为准：不同子曲风下可以有同名模板；只有三级路径
//! 完全相同时才是「同名已存在」。
//!
//! ## 本模块的边界
//!
//! 本模块是**纯函数层**（无 Slint、无对话框）：给它两个根目录，它还你一份
//! 排序确定的模板清单与选择器行模型；给它三级名字与一份归档，它落盘或如实
//! 报「已存在」。界面接线（菜单 / 对话框 / 换会话）住在
//! [`crate::host::wire_project_templates`]；「从模板新建」的副本语义（打开后
//! 保存落点为空）由装配方注入的 `after` 决定，⛔ 不在本模块。

use std::path::{Path, PathBuf};

/// 模板文件的后缀（与工程文件相同 —— 模板就是工程文件）。
pub const TEMPLATE_FILE_EXTENSION: &str = "yeban";

/// 选择器一次最多画多少行（分组标题行与模板行合计）。
///
/// 画布有界 ⇒ 行模型必须有界：超出部分由宿主**如实报数**（选择器底部的
/// 「还有 N 行未画出」），⛔ 不静默截断。与时光机的
/// `host::UNDO_TREE_MAX_NODES` 同一条纪律；`elements.rs` 的注册表按这个
/// 常数登记 `template-row-{i}`，两处同源、不会漂移。
pub const TEMPLATE_CHOOSER_MAX_ROWS: usize = 64;

/// 一份模板清单的两个根目录。生产装配用 [`TemplateRoots::production`]；
/// 判据注入临时目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateRoots {
    /// 工厂模板根（随应用分发，只读语义：本模块从不往这里写）。
    pub factory: PathBuf,
    /// 用户模板根（「另存为模板」的唯一落点）。
    pub user: PathBuf,
}

impl TemplateRoots {
    /// 生产装配的两个根：工厂 = 仓根 `assets/templates`，用户 = 与预制库
    /// 同源的 `~/Music/Audio Music Apps/Project Templates`。
    #[must_use]
    pub fn production() -> Self {
        Self {
            factory: factory_template_root(),
            user: user_template_root(),
        }
    }
}

/// 工厂模板目录：仓根 `assets/templates`（仓库根解析与 SFZ 内置资产同源，
/// 见 [`crate::sfz_load::repo_root`]）。
#[must_use]
pub fn factory_template_root() -> PathBuf {
    crate::sfz_load::repo_root()
        .join("assets")
        .join("templates")
}

/// 用户模板目录：[`yeban_model::preset::user_template_library_dir`] 的转发
/// （位置的唯一定义在模型层，与用户预制库同源）。
#[must_use]
pub fn user_template_root() -> PathBuf {
    yeban_model::preset::user_template_library_dir()
}

/// 确保用户模板根存在（不存在则连同父目录一并创建）。
///
/// # Errors
///
/// 创建失败时回给用户看的完整原因（**非空**）。
pub fn ensure_user_template_root(root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(root)
        .map_err(|error| format!("创建用户模板目录 `{}` 失败: {error}", root.display()))
}

/// 模板来自哪个根目录。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TemplateSource {
    /// 工厂模板（随应用分发）。
    Factory,
    /// 我的模板（用户目录，「另存为模板」写在这里）。
    User,
}

impl TemplateSource {
    /// 选择器里的分组标题（中文内联，与界面文案同一口径）。
    #[must_use]
    pub fn group_label(self) -> &'static str {
        match self {
            TemplateSource::Factory => "工厂模板",
            TemplateSource::User => "我的模板",
        }
    }
}

/// 一条模板 = 三级分类 + 落盘路径。`name` 是**不带后缀**的模板名
/// （文件名是 `{name}.yeban`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateEntry {
    /// 来自哪个根目录。
    pub source: TemplateSource,
    /// 一级：曲风。
    pub style: String,
    /// 二级：子曲风。
    pub substyle: String,
    /// 三级：模板名（无后缀）。
    pub name: String,
    /// 工程文件本体的完整路径。
    pub path: PathBuf,
}

/// 扫描两个根目录，返回全部模板。
///
/// 顺序是**确定**的：先工厂后用户；同一来源内按曲风、子曲风、模板名的
/// 字符串序。只认恰好三级深度的 `.yeban` 文件：根下直放的文件、二级深度
/// 的文件、四级及更深的文件、非 `.yeban` 文件、非目录的一二级条目都被
/// 跳过（它们不是按三级分类存放的模板）。根目录不存在 ⇒ 该来源为空
/// （工厂目录随分发可缺席、用户目录在首次另存前可缺席，都是正常态）。
#[must_use]
pub fn list_templates(roots: &TemplateRoots) -> Vec<TemplateEntry> {
    let mut out = Vec::new();
    list_templates_in(&roots.factory, TemplateSource::Factory, &mut out);
    list_templates_in(&roots.user, TemplateSource::User, &mut out);
    out
}

fn list_templates_in(root: &Path, source: TemplateSource, out: &mut Vec<TemplateEntry>) {
    let Ok(styles) = std::fs::read_dir(root) else {
        return;
    };
    let mut style_names: Vec<String> = styles
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    style_names.sort();
    for style in style_names {
        let Ok(substyles) = std::fs::read_dir(root.join(&style)) else {
            continue;
        };
        let mut substyle_names: Vec<String> = substyles
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        substyle_names.sort();
        for substyle in substyle_names {
            let dir = root.join(&style).join(&substyle);
            let Ok(files) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut names: Vec<String> = files
                .flatten()
                .filter(|entry| entry.path().is_file())
                .filter_map(|entry| {
                    let path = entry.path();
                    if path.extension().and_then(|ext| ext.to_str())
                        == Some(TEMPLATE_FILE_EXTENSION)
                    {
                        path.file_stem()
                            .and_then(|stem| stem.to_str())
                            .map(str::to_owned)
                    } else {
                        None
                    }
                })
                .collect();
            names.sort();
            for name in names {
                out.push(TemplateEntry {
                    source,
                    style: style.clone(),
                    substyle: substyle.clone(),
                    path: dir.join(format!("{name}.{TEMPLATE_FILE_EXTENSION}")),
                    name,
                });
            }
        }
    }
}

/// 选择器行模型的行种类。`code()` 是它在 `.slint` 侧的整数编码
/// （`TemplateChooserRow.kind`），两处必须同值 —— 判据在
/// `tests/project_templates.rs` 里对账。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChooserRowKind {
    /// 来源分组标题（工厂模板 / 我的模板）。
    Source,
    /// 一级分组标题（曲风）。
    Style,
    /// 二级分组标题（子曲风）。
    Substyle,
    /// 模板条目行（可点，`entry_index` 有效）。
    Entry,
}

impl ChooserRowKind {
    /// `.slint` 侧的整数编码。
    #[must_use]
    pub fn code(self) -> i32 {
        match self {
            ChooserRowKind::Source => 0,
            ChooserRowKind::Style => 1,
            ChooserRowKind::Substyle => 2,
            ChooserRowKind::Entry => 3,
        }
    }
}

/// 选择器的一行（分组列表形态：标题行与条目行同列，靠缩进与字重分级）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChooserRow {
    /// 行种类。
    pub kind: ChooserRowKind,
    /// 这一行显示的文字（标题行 = 分类名，条目行 = 模板名）。
    pub label: String,
    /// 条目行指向 [`list_templates`] 结果的下标；标题行为 `None`。
    pub entry_index: Option<usize>,
}

/// 把模板清单摊平成选择器的**分组列表**行模型。
///
/// 入参必须是 [`list_templates`] 的输出（已按来源 / 曲风 / 子曲风 / 模板名
/// 排序）：分组标题在它的第一个成员之前出现一次。空组不出现（某来源一个
/// 模板都没有 ⇒ 它的分组标题也不画，选择器画整体空态）。
#[must_use]
pub fn build_chooser_rows(entries: &[TemplateEntry]) -> Vec<ChooserRow> {
    let mut rows = Vec::new();
    // 三级分组键都带上来源：两个来源可以有同名曲风 / 子曲风，各自成组。
    let mut current_source: Option<TemplateSource> = None;
    let mut current_style: Option<(TemplateSource, &str)> = None;
    let mut current_substyle: Option<(TemplateSource, &str, &str)> = None;
    for (index, entry) in entries.iter().enumerate() {
        if current_source != Some(entry.source) {
            rows.push(ChooserRow {
                kind: ChooserRowKind::Source,
                label: entry.source.group_label().to_owned(),
                entry_index: None,
            });
            current_source = Some(entry.source);
            current_style = None;
            current_substyle = None;
        }
        if current_style != Some((entry.source, entry.style.as_str())) {
            rows.push(ChooserRow {
                kind: ChooserRowKind::Style,
                label: entry.style.clone(),
                entry_index: None,
            });
            current_style = Some((entry.source, entry.style.as_str()));
            current_substyle = None;
        }
        if current_substyle != Some((entry.source, entry.style.as_str(), entry.substyle.as_str())) {
            rows.push(ChooserRow {
                kind: ChooserRowKind::Substyle,
                label: entry.substyle.clone(),
                entry_index: None,
            });
            current_substyle = Some((entry.source, entry.style.as_str(), entry.substyle.as_str()));
        }
        rows.push(ChooserRow {
            kind: ChooserRowKind::Entry,
            label: entry.name.clone(),
            entry_index: Some(index),
        });
    }
    rows
}

/// 「另存为模板」对话框的分类提示：用户模板里已有的「曲风 / 子曲风」组合
/// （排序、去重）。用户照着填就是复用已有分类，填新的就是新建分类 ——
/// 两种都不需要额外的「新建分类」动作（落盘时目录自动创建）。
#[must_use]
pub fn user_category_hint(entries: &[TemplateEntry]) -> Vec<String> {
    let mut pairs: Vec<String> = entries
        .iter()
        .filter(|entry| entry.source == TemplateSource::User)
        .map(|entry| format!("{} / {}", entry.style, entry.substyle))
        .collect();
    pairs.sort();
    pairs.dedup();
    pairs
}

/// 校验一级名字段（曲风 / 子曲风 / 模板名共用同一条规则）：它将成为路径的
/// **一段**，因此绝不能含有路径分隔符或点段 —— 否则模板会落到模板根之外。
///
/// 规则：首尾空白先裁掉；空 ⇒ 拒绝；含 `/` 或 `\` ⇒ 拒绝；等于 `.` / `..`
/// 或以 `.` 开头（隐藏目录）⇒ 拒绝；含控制字符 ⇒ 拒绝。模板名额外剥掉
/// 一个尾随的 `.yeban`（用户顺手带了后缀时，落盘不产生双后缀，列表读回
/// 的名字与输入一致）。
///
/// # Errors
///
/// 给用户看的完整原因（**非空**），调用方原样写进对话框的失败行。
pub fn validate_template_segment(field: &str, raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{field}不能为空"));
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(format!("{field}不能包含路径分隔符（/ 或 \\）：`{trimmed}`"));
    }
    if trimmed == "." || trimmed == ".." || trimmed.starts_with('.') {
        return Err(format!("{field}不能是点段或以 `.` 开头：`{trimmed}`"));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(format!("{field}不能包含控制字符"));
    }
    Ok(trimmed.to_owned())
}

/// 模板名校验：在 [`validate_template_segment`] 之上剥掉一个尾随 `.yeban`。
///
/// # Errors
///
/// 同 [`validate_template_segment`]。
pub fn validate_template_name(raw: &str) -> Result<String, String> {
    let name = validate_template_segment("模板名", raw)?;
    let stripped = name
        .strip_suffix(&format!(".{TEMPLATE_FILE_EXTENSION}"))
        .unwrap_or(&name);
    if stripped.is_empty() {
        return Err("模板名不能为空".to_owned());
    }
    Ok(stripped.to_owned())
}

/// 三级路径 ⇒ 模板文件本体路径（**不**创建任何目录）。
#[must_use]
pub fn template_target_path(user_root: &Path, style: &str, substyle: &str, name: &str) -> PathBuf {
    user_root
        .join(style)
        .join(substyle)
        .join(format!("{name}.{TEMPLATE_FILE_EXTENSION}"))
}

/// [`save_template_archive`] 的失败分类：宿主对「已存在」的处置（留在对话框
/// 里问覆盖）与其余失败（响亮报回执）不同，因此必须在类型上分开。
#[derive(Debug)]
pub enum SaveTemplateError {
    /// 三级路径上已有同名模板且调用方没说覆盖。载荷是那个已有文件的路径。
    Exists(PathBuf),
    /// 字段校验失败（消息给用户看）。
    Invalid(String),
    /// 落盘失败（消息给用户看）。
    Io(String),
}

impl std::fmt::Display for SaveTemplateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveTemplateError::Exists(path) => {
                write!(formatter, "同名模板已存在：`{}`", path.display())
            }
            SaveTemplateError::Invalid(message) | SaveTemplateError::Io(message) => {
                write!(formatter, "{message}")
            }
        }
    }
}

impl std::error::Error for SaveTemplateError {}

/// 把一份归档存成用户模板（三级路径自动建目录）。
///
/// 落盘走**唯一**的归档写入口 [`crate::save::save_archive_file`]（与「保存
/// 工程」/「新建工程」同一条原子写 + 同一把 `.yeban.lock`）⇒ 模板文件与
/// 工程文件逐字节同构，⛔ 没有第二份容器写入。存的是**当前工程原样**
/// （照 Logic：不自动剥离片段内容）。
///
/// `overwrite = false` 且三级路径已有文件 ⇒ [`SaveTemplateError::Exists`]
/// （一个字节都不动）。`overwrite = true` ⇒ 原子替换。
///
/// # Errors
///
/// 见 [`SaveTemplateError`]。成功时返回 `(落盘路径, 写出的字节数)` —— 字节数
/// 来自写入口的回执（[`crate::save::SaveReport::bytes`]），不是事后量文件。
pub fn save_template_archive(
    user_root: &Path,
    style: &str,
    substyle: &str,
    name: &str,
    archive: &yeban_model::container::ProjectArchive,
    overwrite: bool,
) -> Result<(PathBuf, usize), SaveTemplateError> {
    let style = validate_template_segment("曲风", style).map_err(SaveTemplateError::Invalid)?;
    let substyle =
        validate_template_segment("子曲风", substyle).map_err(SaveTemplateError::Invalid)?;
    let name = validate_template_name(name).map_err(SaveTemplateError::Invalid)?;
    let target = template_target_path(user_root, &style, &substyle, &name);
    if target.exists() && !overwrite {
        return Err(SaveTemplateError::Exists(target));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            SaveTemplateError::Io(format!(
                "创建模板分类目录 `{}` 失败: {error}",
                parent.display()
            ))
        })?;
    }
    let report = crate::save::save_archive_file(archive, &target)
        .map_err(|error| SaveTemplateError::Io(format!("模板落盘失败: {error}")))?;
    Ok((target, report.bytes))
}
