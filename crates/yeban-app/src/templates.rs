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
//! ## 模板快照（T-FIX-295）
//!
//! 「另存为模板」写的是**模板快照**，不是会话备份：撤销历史（`history.dag`）、
//! 未合并提案、内嵌用户预制与装载留痕都不进模板；资产池与 `project.assets`
//! 索引只保留当前工程引用到的资产（片段池里的音频 + `embedded_samples`）。
//! 于是从模板新建后撤销栈从空开始，模板体积也不随来源会话的素材池 / 撤销
//! 历史增长；片段内容本身不剥离（照 Logic 的产品边界）。收敛逻辑集中在
//! [`template_snapshot`]，所有落盘调用方都绕不过它。
//!
//! ## 两个模板目录
//!
//! | 目录 | 位置 | 选址理由 |
//! | :--- | :--- | :--- |
//! | 工厂模板 | 安装资源根下的 `assets/templates/`（[`factory_template_root`]） | 随应用分发的资源住在安装包的资源目录：macOS `.app` 为 `Contents/Resources/assets/`，Linux `.deb` 为 `/usr/share/yeban/assets/`；源码树直接运行时回退到仓根 `assets/`（[`crate::sfz_load::repo_root`]，与 SFZ 内置资产同源）。目录可不存在 / 为空 ⇒ 列表如实为空组。 |
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

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use yeban_model::AssetHash;
use yeban_model::project::ClipContent;

/// 模板文件的后缀（与工程文件相同 —— 模板就是工程文件）。
pub const TEMPLATE_FILE_EXTENSION: &str = "yeban";

/// 选择器行族在 [`crate::elements`] 注册表里登记的**代表窗口**（`template-row-{0..N}`）。
///
/// T-FIX-293 起行模型**无界**：宿主把全部行写进模型，界面用 `Flickable` 纵向滚动
/// 到达任意一行，`hidden_rows` 恒为 0（⛔ 不再按此截断）。这个常数不再限制行数，
/// 只给注册表一个枚举窗口 —— 覆盖判据按 `.slint` 的 `"template-row-" + row-index`
/// 模板匹配，与窗口大小无关；运行时第 65 行起与前 64 行共享同一模板、同一 `kind`
/// 与同一 `accessible-action-default` 落点。⛔ 残留缺口（登记在 T-FIX-293 回执）：
/// 若将来有判据对打开的、超过 64 行的选择器做运行时 `coverage_against` 断言，
/// 第 65 行起会报 `unknown_at_runtime` —— 今天没有这样的判据。
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
    ///
    /// # Errors
    ///
    /// 用户主目录不可解析时响亮拒绝，⛔ 不退化成 cwd 相对路径。
    pub fn production() -> Result<Self, String> {
        Ok(Self {
            factory: factory_template_root(),
            user: user_template_root()?,
        })
    }
}

/// 工厂模板目录：安装资源根下的 `assets/templates`。
///
/// 解析顺序（只影响工厂模板，⛔ 不改 [`crate::sfz_load::repo_root`] 的 SFZ 语义）：
///
/// 1. macOS `.app`：`<Contents>/Resources/assets/templates`（可执行文件在
///    `Contents/MacOS/`）；
/// 2. 安装目录旁：`<可执行文件目录>/assets/templates`（Linux 若把资源装在
///    `/usr/lib/yeban/assets` 也走这一条）；
/// 3. Linux 共享资源：`<prefix>/share/yeban/assets/templates`（可执行文件在
///    `<prefix>/lib/yeban/` 时，`<prefix>` 为 `/usr` 等）；
/// 4. 源码树 / 找不到安装资源：仓根 `assets/templates`（见
///    [`crate::sfz_load::repo_root`]）—— `cargo run` / `target/release` 仍可用。
///
/// 目录存在但为空时，调用方（[`crate::templates::list_templates`]）如实返回
/// 空工厂组，⛔ 不伪造模板。
#[must_use]
pub fn factory_template_root() -> PathBuf {
    match std::env::current_exe() {
        Ok(exe) => factory_template_root_for_exe(&exe),
        Err(_) => source_tree_assets_root().join("templates"),
    }
}

/// [`factory_template_root`] 的可注入版本（判据用临时目录模拟三种安装形态）。
fn factory_template_root_for_exe(exe: &Path) -> PathBuf {
    bundled_assets_root_for_exe(exe)
        .unwrap_or_else(source_tree_assets_root)
        .join("templates")
}

/// 从可执行文件位置推断安装资源根（`.../assets`）；源码树返回 `None`。
///
/// 判据只看目录是否存在：安装包若把 `assets/templates` 漏装，目录不存在
/// ⇒ 退回源码树根；打包脚本的自验必须把“目录存在”钉住，不能让运行时
/// 用编译期路径掩盖漏装。
fn bundled_assets_root_for_exe(exe: &Path) -> Option<PathBuf> {
    let exe_dir = exe.parent()?;

    // 1. macOS `.app`：<Contents>/MacOS/yeban → <Contents>/Resources/assets
    if let Some(contents) = exe_dir.parent() {
        let mac_assets = contents.join("Resources").join("assets");
        if mac_assets.is_dir() {
            return Some(mac_assets);
        }
    }

    // 2. 应用私有安装目录：<exe_dir>/assets（Linux 若装 /usr/lib/yeban/assets）
    let local_assets = exe_dir.join("assets");
    if local_assets.is_dir() {
        return Some(local_assets);
    }

    // 3. Linux 共享资源：<prefix>/lib/yeban/yeban-app → <prefix>/share/yeban/assets
    if let Some(prefix) = exe_dir.parent().and_then(Path::parent) {
        let shared_assets = prefix.join("share").join("yeban").join("assets");
        if shared_assets.is_dir() {
            return Some(shared_assets);
        }
    }

    None
}

/// 源码树回退根：仓根 `assets`（唯一解析仍是 [`crate::sfz_load::repo_root`]）。
fn source_tree_assets_root() -> PathBuf {
    crate::sfz_load::repo_root().join("assets")
}

/// 用户模板目录：[`yeban_model::preset::user_template_library_dir`] 的转发
/// （位置的唯一定义在模型层，与用户预制库同源）。
///
/// # Errors
///
/// 用户主目录不可解析。
pub fn user_template_root() -> Result<PathBuf, String> {
    yeban_model::preset::user_template_library_dir()
}

/// 用户模板根必须是绝对路径，且位于解析出的用户主目录下。
fn validate_user_template_root(root: &Path) -> Result<(), String> {
    if !root.is_absolute() {
        return Err(format!(
            "用户模板根必须是绝对路径，实际 `{}`；拒绝退化为当前工作目录",
            root.display()
        ));
    }
    let home = yeban_model::preset::user_home_dir()?;
    if !root.starts_with(&home) {
        return Err(format!(
            "用户模板根必须位于用户主目录 `{}` 下，实际 `{}`",
            home.display(),
            root.display()
        ));
    }
    Ok(())
}

/// 确保用户模板根存在（不存在则连同父目录一并创建）。
///
/// # Errors
///
/// 根不是绝对路径、不在用户主目录下，或创建失败时回给用户看的完整原因
/// （**非空**）。
pub fn ensure_user_template_root(root: &Path) -> Result<(), String> {
    validate_user_template_root(root)?;
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
    /// 选择器里的分组标题（源语言 zh-CN 原文）。
    #[must_use]
    pub fn group_label(self) -> &'static str {
        match self {
            TemplateSource::Factory => "工厂模板",
            TemplateSource::User => "我的模板",
        }
    }

    /// 选择器里的分组标题（当前界面语言下的显示名，九语闭集）。
    ///
    /// 这是宿主写入行模型的闭集常驻显示标签（与总览档名
    /// `song_overview::profile_display_name_for`、状态栏选区格同一口径）：
    /// 写点 [`build_chooser_rows`] 按调用方给的语言取名。行模型在打开
    /// 选择器那一刻按当时的界面语言构建，语言切换后重新打开即是新语言
    /// （选择器是模态弹窗，不在 `host::apply_language` 的常驻重标面内）。
    #[must_use]
    pub fn group_label_for(self, lang: crate::cli::Language) -> &'static str {
        use crate::cli::Language::{DeDe, EnUs, EsEs, FrFr, JaJp, KoKr, PtBr, RuRu, ZhCn};
        match (self, lang) {
            (_, ZhCn) => self.group_label(),
            (TemplateSource::Factory, EnUs) => "Factory Templates",
            (TemplateSource::User, EnUs) => "My Templates",
            (TemplateSource::Factory, FrFr) => "Modèles d'usine",
            (TemplateSource::User, FrFr) => "Mes modèles",
            (TemplateSource::Factory, EsEs) => "Plantillas de fábrica",
            (TemplateSource::User, EsEs) => "Mis plantillas",
            (TemplateSource::Factory, PtBr) => "Modelos de fábrica",
            (TemplateSource::User, PtBr) => "Meus modelos",
            (TemplateSource::Factory, JaJp) => "ファクトリーテンプレート",
            (TemplateSource::User, JaJp) => "マイテンプレート",
            (TemplateSource::Factory, KoKr) => "팩토리 템플릿",
            (TemplateSource::User, KoKr) => "내 템플릿",
            (TemplateSource::Factory, RuRu) => "Заводские шаблоны",
            (TemplateSource::User, RuRu) => "Мои шаблоны",
            (TemplateSource::Factory, DeDe) => "Werksvorlagen",
            (TemplateSource::User, DeDe) => "Meine Vorlagen",
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
/// 模板都没有 ⇒ 它的分组标题也不画，选择器画整体空态）。来源分组标题是
/// 闭集标签，按 `lang` 取名（[`TemplateSource::group_label_for`]）；曲风 /
/// 子曲风 / 模板名是用户内容，原样显示、不翻译。
#[must_use]
pub fn build_chooser_rows(
    entries: &[TemplateEntry],
    lang: crate::cli::Language,
) -> Vec<ChooserRow> {
    build_chooser_rows_matching(entries, lang, "")
}

/// 一条模板是否命中搜索串（T-FIX-293：选择器按来源 / 曲风 / 子曲风 / 模板名过滤）。
///
/// 空串命中全部。非空时不区分大小写、子串匹配「来源标签 曲风 子曲风 模板名」
/// 拼成的整句（各字段本身也是它的子串；多词查询如「流行 摇滚」按整句拼法命中）。
#[must_use]
pub fn entry_matches_query(entry: &TemplateEntry, lang: crate::cli::Language, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    let haystack = format!(
        "{} {} {} {}",
        entry.source.group_label_for(lang),
        entry.style,
        entry.substyle,
        entry.name
    );
    haystack.to_lowercase().contains(&query)
}

/// 把模板清单摊平成选择器的分组列表行模型（带搜索过滤；T-FIX-293）。
///
/// 与 [`build_chooser_rows`] 同一条分组逻辑，区别是只放命中 `query` 的条目：
/// 分组标题只在它名下出现第一条命中条目时才出现；`entry_index` 仍是
/// **全量清单**的下标（宿主按它打开文件），⛔ 不因过滤重排。
#[must_use]
pub fn build_chooser_rows_matching(
    entries: &[TemplateEntry],
    lang: crate::cli::Language,
    query: &str,
) -> Vec<ChooserRow> {
    let mut rows = Vec::new();
    // 三级分组键都带上来源：两个来源可以有同名曲风 / 子曲风，各自成组。
    let mut current_source: Option<TemplateSource> = None;
    let mut current_style: Option<(TemplateSource, &str)> = None;
    let mut current_substyle: Option<(TemplateSource, &str, &str)> = None;
    for (index, entry) in entries.iter().enumerate() {
        if !entry_matches_query(entry, lang, query) {
            continue;
        }
        if current_source != Some(entry.source) {
            rows.push(ChooserRow {
                kind: ChooserRowKind::Source,
                label: entry.source.group_label_for(lang).to_owned(),
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

/// 一次成功「另存为模板」的读数。
///
/// `bytes` 来自唯一归档写入口的 [`crate::save::SaveReport::bytes`]（不是事后量文件），
/// `assets` / `undo_entries` 描述**真正写进模板的那份快照**：模板剥离撤销历史
/// （`history.dag`）、未合并提案与内嵌用户预制，只保留当前工程引用到的资产。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateSaveReport {
    /// 模板落盘的三级路径。
    pub path: PathBuf,
    /// 写出的 `.yeban` 容器字节数。
    pub bytes: usize,
    /// 模板内保留的资产字节条目数（只含当前工程引用到的资产）。
    pub assets: usize,
    /// 模板内撤销历史条目数（模板快照恒为 0）。
    pub undo_entries: usize,
}

/// 把一份**当前工程归档**收敛成模板快照。
///
/// 模板不是会话备份：从模板新建后必须是一份干净的起点，撤销栈从空开始。
/// 因此本函数做四件事，并把取舍钉在类型边界上（任何调用方都绕不过）：
///
/// 1. `history_dag` ⇒ 空字节（模板不携带来源会话的撤销/重做历史）；
/// 2. `proposals` ⇒ `None`；`embedded_presets` ⇒ 空表（未合并提案与用户预制
///    不随模板走）；
/// 3. 资产池只保留**当前工程**引用到的哈希：`clip_pool` 的 `ClipContent::Audio`
///    与 `project.embedded_samples` 的值（SFZ / 样本内嵌索引）。被删片段留在
///    会话池里的未引用字节、以及只在历史里出现的旧资产，都不进模板；
/// 4. `project.assets` 索引同步裁到同一组哈希，避免“模板里有元数据、没有字节”
///    的半截索引。
///
/// `load_notes` 也清空：它是装载留痕，不是工程内容；新模板应当从干净状态开始。
#[must_use]
pub fn template_snapshot(
    archive: &yeban_model::container::ProjectArchive,
) -> yeban_model::container::ProjectArchive {
    let mut referenced: BTreeSet<AssetHash> = BTreeSet::new();
    for clip in archive.project.clip_pool.values() {
        if let ClipContent::Audio { asset, .. } = &clip.content {
            referenced.insert(asset.clone());
        }
    }
    for hash in archive.project.embedded_samples.values() {
        // `embedded_samples` 的值按模型契约是 64 位小写十六进制；解析失败的值
        // 不属于任何可寻址资产，不能因此把整池字节带进模板。
        if let Ok(hash) = AssetHash::parse(hash.clone()) {
            referenced.insert(hash);
        }
    }

    let mut project = archive.project.clone();
    project
        .assets
        .retain(|hash, _metadata| referenced.contains(hash));

    yeban_model::container::ProjectArchive {
        project,
        history_dag: Vec::new(),
        proposals: None,
        assets: archive
            .assets
            .iter()
            .filter(|(hash, _bytes)| referenced.contains(hash))
            .cloned()
            .collect(),
        embedded_presets: Vec::new(),
        load_notes: Vec::new(),
    }
}

/// 把一份归档存成用户模板（三级路径自动建目录）。
///
/// 落盘走**唯一**的归档写入口 [`crate::save::save_archive_file`]（与「保存
/// 工程」/「新建工程」同一条原子写 + 同一把 `.yeban.lock`）⇒ 模板文件与
/// 工程文件逐字节同构，⛔ 没有第二份容器写入。
///
/// **写出的不是调用方给的归档原样**：函数先经 [`template_snapshot`] 收敛成
/// 模板快照 —— 剥离撤销历史 / 未合并提案 / 内嵌用户预制，只保留当前工程
/// 引用到的资产；从模板新建后撤销栈从空开始。片段内容本身不剥离（照 Logic：
/// 不自动剥离片段内容），这是模板与“只留配置”的产品边界。
///
/// `overwrite = false` 且三级路径已有文件 ⇒ [`SaveTemplateError::Exists`]
/// （一个字节都不动）。`overwrite = true` ⇒ 原子替换。
///
/// # Errors
///
/// 见 [`SaveTemplateError`]。成功时返回 [`TemplateSaveReport`] —— 字节数来自
/// 写入口的回执，资产数 / 撤销条目数来自**真正写出的快照**，不是入参。
pub fn save_template_archive(
    user_root: &Path,
    style: &str,
    substyle: &str,
    name: &str,
    archive: &yeban_model::container::ProjectArchive,
    overwrite: bool,
) -> Result<TemplateSaveReport, SaveTemplateError> {
    validate_user_template_root(user_root).map_err(SaveTemplateError::Invalid)?;
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
    let snapshot = template_snapshot(archive);
    let report = crate::save::save_archive_file(&snapshot, &target)
        .map_err(|error| SaveTemplateError::Io(format!("模板落盘失败: {error}")))?;
    Ok(TemplateSaveReport {
        path: target,
        bytes: report.bytes,
        assets: snapshot.assets.len(),
        undo_entries: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::TemplateSource;
    use crate::cli::Language::{self, DeDe, EnUs, EsEs, FrFr, JaJp, KoKr, PtBr, RuRu, ZhCn};
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 唯一临时根：并行测试不互相踩目录；只用于“安装形态 → 路径”的纯路径判据。
    fn scratch_root(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时钟必须晚于 Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "yeban-templates-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("创建临时根失败");
        root
    }

    fn cleanup(root: &Path) {
        std::fs::remove_dir_all(root).expect("删除临时根失败");
    }

    fn fake_exe(path: &Path) {
        std::fs::create_dir_all(path.parent().expect("测试可执行文件必须有父目录"))
            .expect("创建可执行文件父目录失败");
        std::fs::write(path, b"").expect("写测试可执行文件失败");
    }

    /// T-FIX-298：macOS `.app` 的工厂模板必须读 `Contents/Resources/assets/templates`，
    /// 不再从编译期 `CARGO_MANIFEST_DIR` 推出源码树路径。
    #[test]
    fn factory_root_prefers_macos_bundle_resources() {
        let root = scratch_root("mac");
        let contents = root.join("Yeban.app/Contents");
        let exe = contents.join("MacOS/yeban");
        fake_exe(&exe);
        let expected = contents.join("Resources/assets/templates");
        std::fs::create_dir_all(&expected).expect("创建 Resources 模板目录失败");

        assert_eq!(super::factory_template_root_for_exe(&exe), expected);
        cleanup(&root);
    }

    /// T-FIX-298：Linux `.deb` 的标准共享资源路径是 `<prefix>/share/yeban/assets`；
    /// 可执行文件在 `<prefix>/lib/yeban/` 时必须能找到它。
    #[test]
    fn factory_root_uses_linux_share_prefix() {
        let root = scratch_root("linux-share");
        let prefix = root.join("usr");
        let exe = prefix.join("lib/yeban/yeban-app");
        fake_exe(&exe);
        let expected = prefix.join("share/yeban/assets/templates");
        std::fs::create_dir_all(&expected).expect("创建 share/yeban 模板目录失败");

        assert_eq!(super::factory_template_root_for_exe(&exe), expected);
        cleanup(&root);
    }

    /// T-FIX-298：若发行包把资源放在可执行文件旁（`<exe_dir>/assets`），也要读到；
    /// 这条是 Linux 安装布局的兼容腿。
    #[test]
    fn factory_root_reads_app_local_assets() {
        let root = scratch_root("app-local");
        let exe_dir = root.join("opt/yeban");
        let exe = exe_dir.join("yeban-app");
        fake_exe(&exe);
        let expected = exe_dir.join("assets/templates");
        std::fs::create_dir_all(&expected).expect("创建应用内模板目录失败");

        assert_eq!(super::factory_template_root_for_exe(&exe), expected);
        cleanup(&root);
    }

    /// T-FIX-298：源码树直跑（`target/release/yeban-app`）没有安装资源目录时，
    /// 必须回退到仓根 `assets/templates`，否则 `cargo run` 的工厂组会凭空消失。
    #[test]
    fn factory_root_falls_back_to_source_tree_assets() {
        let root = scratch_root("source");
        let exe = root.join("target/release/yeban-app");
        fake_exe(&exe);

        assert_eq!(
            super::factory_template_root_for_exe(&exe),
            super::source_tree_assets_root().join("templates")
        );
        cleanup(&root);
    }

    /// T-FIX-298：按安装包形态解析出的工厂根必须真的被选择器扫描到；
    /// 只断言路径存在还不够，这里放一份三级 `.yeban` 走 [`super::list_templates`]。
    #[test]
    fn installed_factory_root_is_read_by_template_scanner() {
        let root = scratch_root("scan");
        let contents = root.join("Yeban.app/Contents");
        let exe = contents.join("MacOS/yeban");
        fake_exe(&exe);
        let factory = contents.join("Resources/assets/templates");
        let template_dir = factory.join("电子/氛围");
        std::fs::create_dir_all(&template_dir).expect("创建三级模板目录失败");
        std::fs::write(template_dir.join("夜色.yeban"), b"").expect("写模板占位失败");
        let user = root.join("user-templates");
        std::fs::create_dir_all(&user).expect("创建用户模板根失败");

        let entries = super::list_templates(&super::TemplateRoots { factory, user });
        assert_eq!(entries.len(), 1, "工厂根下唯一的三级 .yeban 必须被扫到");
        assert_eq!(entries[0].source, super::TemplateSource::Factory);
        assert_eq!(entries[0].style, "电子");
        assert_eq!(entries[0].substyle, "氛围");
        assert_eq!(entries[0].name, "夜色");
        cleanup(&root);
    }

    /// 分组标题九语闭集逐条钉死：任一语言缺译 / 误译当场红（与总览档名
    /// `profile_display_name_for` 的九语单测同一口径）。
    #[test]
    fn group_labels_cover_all_nine_languages() {
        let expected: [(Language, &str, &str); 9] = [
            (ZhCn, "工厂模板", "我的模板"),
            (EnUs, "Factory Templates", "My Templates"),
            (FrFr, "Modèles d'usine", "Mes modèles"),
            (EsEs, "Plantillas de fábrica", "Mis plantillas"),
            (PtBr, "Modelos de fábrica", "Meus modelos"),
            (JaJp, "ファクトリーテンプレート", "マイテンプレート"),
            (KoKr, "팩토리 템플릿", "내 템플릿"),
            (RuRu, "Заводские шаблоны", "Мои шаблоны"),
            (DeDe, "Werksvorlagen", "Meine Vorlagen"),
        ];
        assert_eq!(expected.len(), Language::ALL.len(), "九语必须全覆盖");
        for (lang, factory, user) in expected {
            assert_eq!(TemplateSource::Factory.group_label_for(lang), factory);
            assert_eq!(TemplateSource::User.group_label_for(lang), user);
        }
    }

    /// 构造一条测试用模板条目（路径只占位，匹配不看路径）。
    fn entry(
        source: TemplateSource,
        style: &str,
        substyle: &str,
        name: &str,
    ) -> super::TemplateEntry {
        super::TemplateEntry {
            source,
            style: style.to_owned(),
            substyle: substyle.to_owned(),
            name: name.to_owned(),
            path: std::path::PathBuf::from(format!("{style}/{substyle}/{name}.yeban")),
        }
    }

    /// T-FIX-293：搜索按来源 / 曲风 / 子曲风 / 模板名命中；空串与纯空白命中
    /// 全部；不区分大小写；多词按整句拼法命中。
    #[test]
    fn entry_matches_query_covers_source_style_substyle_and_name() {
        let fixture = entry(TemplateSource::User, "流行", "摇滚", "出发");
        // 空串 / 纯空白命中全部。
        assert!(super::entry_matches_query(&fixture, ZhCn, ""));
        assert!(super::entry_matches_query(&fixture, ZhCn, "   "));
        // 四个字段各自命中。
        assert!(super::entry_matches_query(&fixture, ZhCn, "我的模板"));
        assert!(super::entry_matches_query(&fixture, ZhCn, "流行"));
        assert!(super::entry_matches_query(&fixture, ZhCn, "摇滚"));
        assert!(super::entry_matches_query(&fixture, ZhCn, "出发"));
        // 多词按整句拼法命中；子串命中。
        assert!(super::entry_matches_query(&fixture, ZhCn, "流行 摇滚"));
        assert!(super::entry_matches_query(&fixture, ZhCn, "出"));
        // 不命中：别的来源标签、别的曲风、别的模板名。
        assert!(!super::entry_matches_query(&fixture, ZhCn, "工厂模板"));
        assert!(!super::entry_matches_query(&fixture, ZhCn, "古典"));
        assert!(!super::entry_matches_query(&fixture, ZhCn, "归来"));
        // 拉丁字母不区分大小写。
        let latin = entry(TemplateSource::Factory, "Pop", "Rock", "Sunrise");
        assert!(super::entry_matches_query(&latin, EnUs, "pop"));
        assert!(super::entry_matches_query(&latin, EnUs, "SUNRISE"));
        assert!(super::entry_matches_query(
            &latin,
            EnUs,
            "Factory Templates"
        ));
    }

    /// T-FIX-293：过滤不重排 `entry_index`（宿主按它打开文件），分组标题只在
    /// 它名下出现第一条命中条目时才出现；空串与不过滤同输出。
    #[test]
    fn build_chooser_rows_matching_keeps_full_list_entry_index() {
        let entries = vec![
            entry(TemplateSource::User, "流行", "摇滚", "出发"),
            entry(TemplateSource::User, "流行", "摇滚", "归来"),
            entry(TemplateSource::User, "古典", "巴洛克", "赋格"),
        ];
        // 空串 = 不过滤。
        assert_eq!(
            super::build_chooser_rows_matching(&entries, ZhCn, ""),
            super::build_chooser_rows(&entries, ZhCn)
        );
        // 只命中第二条：标题只剩它那一组，`entry_index` 仍是全量下标 1。
        let rows = super::build_chooser_rows_matching(&entries, ZhCn, "归来");
        let shape: Vec<(super::ChooserRowKind, String, Option<usize>)> = rows
            .iter()
            .map(|row| (row.kind, row.label.clone(), row.entry_index))
            .collect();
        assert_eq!(
            shape,
            vec![
                (super::ChooserRowKind::Source, "我的模板".to_owned(), None),
                (super::ChooserRowKind::Style, "流行".to_owned(), None),
                (super::ChooserRowKind::Substyle, "摇滚".to_owned(), None),
                (super::ChooserRowKind::Entry, "归来".to_owned(), Some(1)),
            ],
            "过滤后的条目下标必须是全量清单下标，⛔ 不重排"
        );
        // 无命中 ⇒ 空模型（不是截断计数）。
        assert!(
            super::build_chooser_rows_matching(&entries, ZhCn, "不存在的名字").is_empty(),
            "无命中时行模型为空"
        );
    }

    /// T-FIX-293：70 条同组模板的行模型 = 3 个标题行 + 70 条目行 —— 纯函数层
    /// 没有 64 上限（旧口径的截断只在宿主，如今已移除，回执里登记）。
    #[test]
    fn build_chooser_rows_has_no_sixty_four_cap() {
        let entries: Vec<super::TemplateEntry> = (0..70)
            .map(|index| {
                entry(
                    TemplateSource::User,
                    "流行",
                    "摇滚",
                    &format!("模板 {index:02}"),
                )
            })
            .collect();
        let rows = super::build_chooser_rows_matching(&entries, ZhCn, "");
        assert_eq!(
            rows.iter()
                .filter(|row| row.kind == super::ChooserRowKind::Entry)
                .count(),
            70,
            "70 条模板必须全进模型"
        );
        assert_eq!(rows.len(), 73, "70 条目行 + 来源/曲风/子曲风 3 个标题行");
        assert_eq!(
            rows.iter()
                .rfind(|row| row.kind == super::ChooserRowKind::Entry)
                .and_then(|row| row.entry_index),
            Some(69),
            "最后一条的下标必须是全量下标 69"
        );
    }
}
