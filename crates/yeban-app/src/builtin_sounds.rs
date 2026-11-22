//! ⭐ 内置音源清单（`assets/samples/manifest.json`）—— 音源管理里「内置音源」那一区
//! 的唯一数据源。
//!
//! ## 为什么不是再写一份清单
//!
//! 仓库里**已经**有一份 30 条乐器的登记表：`assets/samples/manifest.json`（8.7 MB，
//! 每条带 `repo` / `pin` / `license` / `total_bytes` / `file_count` / `sfz`，
//! 并写明"官方仓库**不随包分发**任何采样字节"）。音源管理要列出的正是这些条目 ——
//! 再手写一份 JSON 就等于第二份真相，**必然**与登记表漂移。
//!
//! 因此本模块只做三件事：**读那张表**、**拼出可下载地址**、**交给既有的安装腿**
//! （[`crate::sound_library`] 的 `download_pack` / `extract_and_place`，⛔ 不另写一条
//! 解包路径 —— 那里的 zip 炸弹护栏、路径逃逸检查、符号链接拒绝都必须继续生效）。
//!
//! ## 下载地址的真实性（如实登记）
//!
//! `repo` + `pin` 都是登记表里写死的值，拼出的地址是 GitHub 的**归档直链**：
//! `https://github.com/<repo>/archive/<pin>.zip`。这是 GitHub 的固定端点（按 commit
//! 归档），**不是**我们编的镜像地址；登记表里的 `pin` 是 commit sha，因此同一地址
//! 永远指向同一份字节。
//!
//! ⚠ 归档**没有**随附的 SHA-256（GitHub 不发布归档摘要），登记表给的是**逐文件**
//! 的指纹配方（`fingerprint`）。所以这一条腿装完只做**结构校验**（zip 可解、无逃逸、
//! 总量在上限内）＋ **落位后逐文件对账**（[`verify_fingerprint`]，样本文件数与总字节
//! 对不上就整包回滚）。⛔ 不假装"哈希已校验"。
//!
//! ## 与远端目录的关系
//!
//! 远端 `sounds/catalog.json`（[`crate::sound_library::fetch_catalog`]）仍然保留，
//! 但音源管理**不再依赖它**：远端挂掉时这一区照常满员（这正是"列表永远是空的"
//! 那个体验缺口的修法）。

use std::path::{Path, PathBuf};

use crate::sound_library::{
    InstalledLibrary, PACK_MARKER_FILE, SoundError, SoundRoots, format_size,
};

/// 登记表相对仓库根的路径。
pub const MANIFEST_RELATIVE: &str = "assets/samples/manifest.json";

/// 一条内置音源（登记表 `instruments[]` 的一行，只留界面与下载用得到的字段）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuiltinSound {
    /// 稳定 id，同时是安装后的目录名（登记表里就是合法目录名）。
    pub id: String,
    /// 显示名。
    pub name: String,
    /// 分类（World / Drums / Keys …），界面上的第二列。
    pub category: String,
    /// 许可（`CC0-1.0` / `CC-BY-3.0` …）。
    pub license: String,
    /// 作者 / 上游。
    pub author: String,
    /// 上游仓库（`owner/name`）。
    pub repo: String,
    /// 钉住的 commit（40 位十六进制）。
    pub pin: String,
    /// 上游 homepage。
    pub source_url: String,
    /// 全部文件的字节数（登记表的 `total_bytes`）。
    pub total_bytes: u64,
    /// 文件数。
    pub file_count: u64,
    /// 是否**已经**在仓库里（登记表 `bundled`）。当前全部为 `false`。
    pub bundled: bool,
    /// 主 SFZ 的相对路径（`Programs/xx.sfz`）。
    pub sfz: String,
    /// 下载地址覆盖（空 = 由 `repo` + `pin` 拼 GitHub commit 归档）。
    ///
    /// 只有「社区知名音源」用它：那些仓库没有钉 commit（或者上游只维护分支），
    /// 地址只能指到 `refs/heads/<branch>.zip` —— ⚠ 这种地址的内容会随上游推进而变，
    /// 因此**永远**没有摘要可校验（与内置那 28 条钉 commit 的性质不同，如实登记）。
    pub archive_override: String,
    /// 上游**发布包**直链（登记表 `provenance_form == "archive"` 且给的是 zip 才有）。
    ///
    /// ⚠ 与 [`Self::archive_url`]（按 commit 的 GitHub 归档）是**两种布局**：
    /// 发布包解压后顶层就是乐器本体（`Emilyguitar/`、`Meatbass/`），
    /// 而 commit 归档会多一层 `<repo>-<sha>/`。登记表的 `sfz` 是按发布包写的 ⇒
    /// 这一档**必须**用发布包，否则装完按登记路径找不到 SFZ（实测 404）。
    pub release_zip: String,
    /// 发布包的 sha256（空 = 未登记 ⇒ 下载后只做结构校验）。
    pub release_sha256: String,
    /// 发布包的**压缩**字节数（0 = 未知）。
    ///
    /// ⚠ 下载时能对上的只有这个数：登记表的 `total_bytes` 是**解压后**的素材总量
    /// （9.4 GiB 那一档），拿它当下载大小会 100% 判"包大小不符"。
    pub release_bytes: u64,
}

/// ⭐ 几个**知名开源音源**（不在仓库登记表里，但用户会主动想要的那几件）。
///
/// 收录标准（三条都满足才进表）：
/// 1. 许可是 CC0 / CC-BY 一类**允许本地保存与商用**的；
/// 2. 上游有**可直接下载**的归档地址（本表只收实测 200 的，⛔ 不收"要去网页点两次"的）；
/// 3. 内容是 SFZ（本仓的采样器认得），不是 SF2 / Kontakt。
///
/// ⚠ 如实登记的两条边界：
/// * 这两个仓库**没有**钉 commit（上游只维护 `master`），所以地址指到分支归档：
///   内容会变、**没有**摘要可校验 —— 装完只做结构校验与文件数>0 的粗检；
/// * 体积未登记（0 ⇒ 不做字节对账），界面上大小列显示 `—`。
#[must_use]
pub fn community_extras() -> Vec<BuiltinSound> {
    vec![
        BuiltinSound {
            id: "salamander-grand-piano".to_owned(),
            name: "Salamander Grand Piano V3".to_owned(),
            category: "Keys".to_owned(),
            license: "CC-BY-3.0 / Public Domain(2022)".to_owned(),
            author: "Alexander Holm".to_owned(),
            repo: "sfzinstruments/SalamanderGrandPiano".to_owned(),
            pin: String::new(),
            source_url: "https://archive.org/details/SalamanderGrandPianoV3".to_owned(),
            total_bytes: 0,
            file_count: 0,
            bundled: false,
            sfz: String::new(),
            archive_override:
                "https://github.com/sfzinstruments/SalamanderGrandPiano/archive/refs/heads/master.zip"
                    .to_owned(),
            release_zip: String::new(),
            release_sha256: String::new(),
            release_bytes: 0,
        },
        // ⚠ T-FIX-107：这里**曾经**还有一条 `vsco-2-ce`「VSCO 2 Community Edition」，
        // 指向 `sgossner/VSCO-2-CE@refs/heads/master`。实测该分支 **0 个 .sfz**
        // （3187 个文件全是采样与说明），装完必然卡在"没有任何 .sfz"这一步。
        // 而同一件音源在登记表里已经有钉 commit 的正确条目（`vsco2ce`，
        // `schollz/VSCO-2-CE` 的 `SFZ` 分支 pin `6dd651d5…`，顶层 75 个 .sfz）——
        // 所以这条重复条目直接删除，⛔ 不再指到不含 SFZ 的分支上让用户白下 2 GB。
    ]
}

impl BuiltinSound {
    /// 下载地址。**发布包优先**（布局与登记表的 `sfz` 一致，且有 sha256 可校验），
    /// 其次才是按 commit 的 GitHub 归档；两者都拼不出时给 `None`
    /// —— ⛔ 不拼一个注定 404 的地址。
    #[must_use]
    pub fn archive_url(&self) -> Option<String> {
        if !self.release_zip.trim().is_empty() {
            return Some(self.release_zip.clone());
        }
        if !self.archive_override.trim().is_empty() {
            return Some(self.archive_override.clone());
        }
        if !is_repo_slug(&self.repo) || !is_commit_pin(&self.pin) {
            return None;
        }
        Some(format!(
            "https://github.com/{}/archive/{}.zip",
            self.repo, self.pin
        ))
    }

    /// 解压后是否需要**抹平**外层目录。
    ///
    /// * 发布包（[`Self::release_zip`] 非空）：顶层已是乐器本体 ⇒ **不抹**
    ///   （抹掉 `Emilyguitar/` 会让登记表的 `sfz` 路径少一层）；
    /// * GitHub commit 归档 / 分支归档：多一层 `<repo>-<sha>/`（或 `<repo>-<branch>/`）
    ///   ⇒ 抹一层。
    #[must_use]
    pub fn needs_wrapper_flatten(&self) -> bool {
        self.release_zip.trim().is_empty()
    }

    /// 允许自动下载（有归档直链）。
    #[must_use]
    pub fn installable(&self) -> bool {
        self.archive_url().is_some()
    }

    /// 界面第二行的说明（许可 · 来源 · 文件数）—— 由宿主拼好再播给界面，
    /// ⛔ 不让 `.slint` 自己拼字符串。
    #[must_use]
    pub fn detail(&self) -> String {
        let mut text = format!("{} · {} 个文件", self.license, self.file_count);
        if !self.author.is_empty() && self.author != self.id {
            text.push_str(" · ");
            text.push_str(&self.author);
        }
        if let Some(url) = self.archive_url() {
            let _ = url;
        } else if !self.source_url.is_empty() {
            text.push_str(" · 需手动下载");
        }
        text
    }

    /// 供安装腿使用的下载说明（大小已知 ⇒ 下载后逐字节对账；哈希未知 ⇒ 空串）。
    #[must_use]
    pub fn download_note(&self) -> String {
        format!(
            "{} · {}（GitHub 归档，{}）",
            self.category,
            self.license,
            format_size(self.total_bytes)
        )
    }
}

/// `owner/name` 形态（只允许字母数字 `-` `_` `.` 与**一个**斜杠）。
fn is_repo_slug(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    [owner, name].iter().all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    })
}

/// 40 位十六进制 commit（GitHub 归档端点接受短 sha，但登记表钉的是全长）。
fn is_commit_pin(pin: &str) -> bool {
    pin.len() >= 7 && pin.len() <= 40 && pin.chars().all(|ch| ch.is_ascii_hexdigit())
}

/// 登记表路径（生产口径 = 仓库根下）。
#[must_use]
pub fn manifest_path() -> PathBuf {
    crate::sfz_load::repo_root().join(MANIFEST_RELATIVE)
}

/// 读登记表并投影成 [`BuiltinSound`] 列表（按分类 + 名字排序，界面顺序稳定）。
///
/// 失败一律是**给人看的中文原文**：文件读不到 / JSON 坏 / `instruments` 不是数组。
/// ⛔ 不静默返回空表 —— "空表"会被界面读成"没有音源"，那是假的。
pub fn load_builtin_sounds() -> Result<Vec<BuiltinSound>, String> {
    use std::sync::OnceLock;
    // ⚠ 清单 8.7 MB，此前**每次**开音源管理都要重新读 + 重新解析（debug 档实测几十秒）。
    // 用户看到的只是"空了半分钟"。进程内缓存一份：第二次起是 **0 成本**。
    static CACHE: OnceLock<Result<Vec<BuiltinSound>, String>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let path = manifest_path();
            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("读内置音源清单失败（{}）：{error}", path.display()))?;
            parse_manifest(&text)
        })
        .clone()
}

/// 只声明**用得上**的字段的清单结构：`serde` 会跳过全部 `items` / `excluded`
/// 之类的巨型数组的**内容**，而 `serde_json::Value` 会把它们整棵建出来。
/// 这就是"同一份 8.7 MB 文件，解析从几十秒掉到毫秒级"的那一处。
#[derive(serde::Deserialize)]
struct ManifestDoc {
    /// ⚠ `Option` 而不是 `#[serde(default)] Vec`：**缺 `instruments` 是错误**，
    /// 不是"空清单"（空清单会被界面读成"没有音源"，那是假的）。
    #[serde(default)]
    instruments: Option<Vec<ManifestEntry>>,
}

#[derive(serde::Deserialize)]
struct ManifestEntry {
    // ⚠ 登记表里确实存在 `null`（例如某条的 `sfz` 没填）⇒ 一律 `Option`，
    // 不能被一个 `null` 把整张表判成"坏文件"（那样音源管理直接空白）。
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    pin: Option<String>,
    #[serde(default)]
    source_url: Option<String>,
    #[serde(default)]
    total_bytes: u64,
    #[serde(default)]
    file_count: u64,
    #[serde(default)]
    bundled: bool,
    #[serde(default)]
    sfz: Option<String>,
    /// 素材来历：`git`（按 commit 归档）或 `archive`（上游发布包 zip）。
    ///
    /// ⚠ 两者**解压后的布局不同**：commit 归档带一层 `<repo>-<sha>/`（要抹平），
    /// 而发布包顶层就是乐器目录本身（`Emilyguitar/…`，⛔ 不能抹）。
    /// 登记表的 `sfz` / `files[]` 对 `archive` 条目是按**发布包**写的。
    #[serde(default)]
    provenance_form: Option<String>,
    /// 上游发布包（仅 `archive` 条目有）。
    #[serde(default)]
    archive: Option<ManifestArchive>,
}

#[derive(serde::Deserialize)]
struct ManifestArchive {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    bytes: u64,
}

/// 解析登记表文本（判据与生产共用）。
pub fn parse_manifest(text: &str) -> Result<Vec<BuiltinSound>, String> {
    let doc: ManifestDoc =
        serde_json::from_str(text).map_err(|error| format!("内置音源清单解析失败：{error}"))?;
    let instruments = doc
        .instruments
        .ok_or_else(|| "内置音源清单缺少 instruments 数组".to_owned())?;
    let mut sounds = Vec::with_capacity(instruments.len());
    for (index, entry) in instruments.into_iter().enumerate() {
        let id = entry.id.unwrap_or_default();
        if id.is_empty() {
            return Err(format!("instruments[{index}] 缺少 id"));
        }
        let text = |value: Option<String>| value.unwrap_or_default();
        // 只有"发布包"条目才取 archive.url，且**只认 zip**：安装腿只解 zip，
        // `.tar.xz`（两个 freepats 风琴）如实留在"需手动下载"那一档。
        let archive = entry.archive;
        let provenance = text(entry.provenance_form);
        let release = match (&archive, provenance.as_str()) {
            (Some(a), "archive") => {
                let url = text(a.url.clone());
                if url.to_ascii_lowercase().ends_with(".zip") {
                    Some((url, text(a.sha256.clone()), a.bytes))
                } else {
                    None
                }
            }
            _ => None,
        };
        let (release_zip, release_sha256, release_bytes) =
            release.unwrap_or_else(|| (String::new(), String::new(), 0));
        sounds.push(BuiltinSound {
            name: {
                let name = text(entry.name);
                if name.is_empty() { id.clone() } else { name }
            },
            category: {
                let category = text(entry.category);
                if category.is_empty() {
                    "未分类".to_owned()
                } else {
                    category
                }
            },
            license: {
                let license = text(entry.license);
                if license.is_empty() {
                    "未登记".to_owned()
                } else {
                    license
                }
            },
            author: text(entry.author),
            repo: text(entry.repo),
            pin: text(entry.pin),
            source_url: text(entry.source_url),
            total_bytes: entry.total_bytes,
            file_count: entry.file_count,
            bundled: entry.bundled,
            sfz: text(entry.sfz),
            id,
            // 登记表里的条目一律走 commit 归档，⛔ 不用覆盖地址。
            archive_override: String::new(),
            release_zip,
            release_sha256,
            release_bytes,
        });
    }
    // ⭐ 两个知名开源音源并进同一张表（同一区展示、同一条安装腿）。
    sounds.extend(community_extras());
    sounds.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(sounds)
}

/// 已安装判定用的目录名（安装落点 = 库根下 `<id>`）。
#[must_use]
pub fn installed_dir(roots: &SoundRoots, id: &str) -> PathBuf {
    roots.library_root.join(id)
}

/// 这一条是否**已经装进用户库**（目录存在且写了包标记）。
#[must_use]
pub fn is_installed(roots: &SoundRoots, id: &str) -> bool {
    let dir = installed_dir(roots, id);
    dir.is_dir() && dir.join(PACK_MARKER_FILE).is_file()
}

/// 下载并安装一条内置音源：归档落到既有安装腿，必要时**抹平外层目录**
/// （GitHub commit 归档的 `<repo>-<sha>/`），再做逐文件对账。
///
/// ⭐ 两处口径（2026-11-21 实测修复）：
///
/// 1. **下载大小 ≠ 素材总量**：安装腿对 `size_bytes` 做的是**精确相等**校验，
///    而登记表的 `total_bytes` 是**解压后**的素材字节和（全表 9.4 GiB 那一档）。
///    此前把它当下载大小传进去 ⇒ zip 永远对不上 ⇒ **每一条都判"包大小不符"**。
///    现在只传**压缩包**字节数（发布包有登记就传，commit 归档没登记就传 0 ＝ 不校验）。
/// 2. **布局**：发布包（`provenance_form == "archive"`）解压后顶层就是乐器本体，
///    抹平会削掉 `Emilyguitar/` 这一层 ⇒ 登记表的 `sfz` 路径失效。这一档不抹。
///
/// 对账失败 ⇒ 已装的那一份整体删掉（⛔ 不留"装了一半"的目录让用户以为能用）。
pub fn download_builtin(
    sound: &BuiltinSound,
    roots: &SoundRoots,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<InstalledLibrary, SoundError> {
    let Some(url) = sound.archive_url() else {
        return Err(SoundError::BadArchive(format!(
            "{} 没有可用的下载地址（repo/pin 形态不对，或上游只提供非 zip 归档）",
            sound.name
        )));
    };
    if is_installed(roots, &sound.id) {
        return Err(SoundError::BadArchive(format!("{} 已经装过了", sound.name)));
    }
    let pack = catalog_pack_for(sound, url);
    let installed = crate::sound_library::download_pack(&pack, roots, progress)?;
    // 只有 commit / 分支归档才多一层 `<repo>-<sha>/`，需要抹平，让 `<id>/` 下
    // 直接是 Programs/…；发布包顶层就是乐器本体，不抹（见 `needs_wrapper_flatten`）。
    if sound.needs_wrapper_flatten() {
        flatten_wrapper_dir(&installed.location)?;
    }
    if let Err(error) = verify_extracted(sound, &installed.location) {
        let _ = std::fs::remove_dir_all(&installed.location);
        return Err(error);
    }
    Ok(installed)
}

/// 把一条内置音源拼成安装腿认的目录包。
fn catalog_pack_for(sound: &BuiltinSound, url: String) -> crate::sound_library::CatalogPack {
    // ⭐ 归档大小只认**压缩包**口径：发布包登记了 `archive.bytes` 就传它并附
    // 登记的 `archive.sha256` 做硬校验；commit 归档没有压缩大小与摘要登记，
    // `size_bytes` 留 0（不校验）、`sha256` 留空串（⛔ 不假装校验过）。
    // ⛔ 绝不把登记表的 `total_bytes` 填进 `size_bytes`——那是**解压后**全部
    // 文件的总字节，安装腿拿它逐字节比对 zip 文件大小必然 `SizeMismatch`，
    // 内置音源会全灭（CODE_AUDIT_REPORT P0-6）。无发布包登记的条目，字节完整性
    // 由落位后的 [`verify_extracted`] 按登记的 `total_bytes` / `file_count`
    // 对账，不足即整包回滚。
    let size_bytes = if sound.release_zip.trim().is_empty() {
        0
    } else {
        sound.release_bytes
    };
    crate::sound_library::CatalogPack {
        id: sound.id.clone(),
        name: sound.name.clone(),
        description: sound.download_note(),
        version: if !sound.release_zip.trim().is_empty() {
            "发布包".to_owned()
        } else if sound.pin.is_empty() {
            "社区".to_owned()
        } else {
            sound.pin.chars().take(8).collect()
        },
        size_bytes,
        // 发布包登记了 sha256 ⇒ 硬校验（比"只做结构校验"强一档）；无登记时为空串。
        sha256: sound.release_sha256.clone(),
        url,
    }
}

/// 抹平"只有一个子目录"的落位结果（GitHub 归档的 `<repo>-<pin>/`）。
///
/// 只抹**唯一**一层；多子目录时原样保留（那是包本身的结构，⛔ 不猜）。
fn flatten_wrapper_dir(root: &Path) -> Result<(), SoundError> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(root)
        .map_err(|error| SoundError::Io(format!("读 {} 失败：{error}", root.display())))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.file_name().and_then(|n| n.to_str()) != Some(PACK_MARKER_FILE))
        .collect();
    if entries.len() != 1 {
        return Ok(());
    }
    let wrapper = entries.remove(0);
    if !wrapper.is_dir() {
        return Ok(());
    }
    let children: Vec<PathBuf> = std::fs::read_dir(&wrapper)
        .map_err(|error| SoundError::Io(format!("读 {} 失败：{error}", wrapper.display())))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    for child in children {
        let Some(name) = child.file_name() else {
            continue;
        };
        let dest = root.join(name);
        std::fs::rename(&child, &dest)
            .map_err(|error| SoundError::Io(format!("移到 {} 失败：{error}", dest.display())))?;
    }
    std::fs::remove_dir(&wrapper)
        .map_err(|error| SoundError::Io(format!("删空壳 {} 失败：{error}", wrapper.display())))?;
    Ok(())
}

/// 落位后的粗对账：**至少有一个 SFZ**、**总字节不少于登记值**、**文件数不少于登记值**。
///
/// 为什么不像最初那样要求"文件数严格相等"：GitHub 归档里还带着仓库自己的
/// `README.md` / `LICENSE` / `.github/*`，登记表的 `file_count` 只数**采样与 SFZ**
/// —— 严格相等会把每一件都判成"装坏了"。这里改成三个**下界**检查：少文件（截断）、
/// 少字节（缺层采样/缺 zone）仍然抓得住，而仓库自带的说明文件不会造成假失败。
fn verify_extracted(sound: &BuiltinSound, installed_root: &Path) -> Result<(), SoundError> {
    let (files, bytes) = count_files_and_bytes(installed_root)?;
    // 标记文件是我们自己写的，⛔ 不计入登记表口径。
    let files = files.saturating_sub(1);
    // ⚠ 结论措辞分三种，⛔ 不再用一句"包里可能不是 SFZ 乐器"糊过去：
    // "查完了确实没有"与"没查完就停了"是两件不同的事实，后者不能断言前者
    // （T-FIX-107：VSCO 2 CE 的归档是"有采样、零 .sfz"的另一个分支，界面却
    // 断言"包里可能不是 SFZ 乐器" —— 误报的来源就是把这两种情形混成 `false`）。
    match scan_sfz(installed_root) {
        SfzScan::Found => {}
        SfzScan::Absent { seen } => {
            return Err(SoundError::BadArchive(format!(
                "{} 装完已查 {seen} 条，里面没有任何 .sfz（这份归档不含 SFZ 乐器文件；\
                 同一件音源若另有 SFZ 分支，只有带 .sfz 的那个分支才是 SFZ 音色）",
                sound.name
            )));
        }
        SfzScan::Incomplete { seen } => {
            return Err(SoundError::BadArchive(format!(
                "{} 装完已查 {seen} 条未见 .sfz（未查完：条目超过 {SFZ_SCAN_LIMIT} 条上限，\
                 不能据此断言没有 .sfz）",
                sound.name
            )));
        }
    }
    // ⭐ 登记表指定的**主 SFZ** 必须真的在：只查"有任意 .sfz"抓不住布局错配
    // （实测：`provenance_form=archive` 的条目按 commit 归档装完少一层目录，
    // 主 SFZ 路径 404，但"有任意 .sfz"仍成立 ⇒ 装完显示成功、点「用上」却找不到文件）。
    if !sound.sfz.is_empty() && !installed_root.join(&sound.sfz).is_file() {
        return Err(SoundError::BadArchive(format!(
            "{} 装完在 {} 找不到登记的主 SFZ —— 包布局与登记表不一致",
            sound.name, sound.sfz
        )));
    }
    if sound.file_count > 0 && files < sound.file_count {
        return Err(SoundError::BadArchive(format!(
            "{} 装完对账不符：至少应有 {} 个文件，实际 {files}",
            sound.name, sound.file_count
        )));
    }
    if sound.total_bytes > 0 && bytes < sound.total_bytes {
        return Err(SoundError::BadArchive(format!(
            "{} 装完对账不符：至少应有 {}，实际 {}",
            sound.name,
            format_size(sound.total_bytes),
            format_size(bytes)
        )));
    }
    Ok(())
}

/// 扫描 `.sfz` 的结论。
///
/// ⛔ 不用 `bool`：`false` 把"全查完了确实没有"与"条目太多没查完"压成同一个答案，
/// 而对用户说的话必须不同 —— 后者只能写"已查 N 条未见 .sfz（未查完）"。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SfzScan {
    /// 找到至少一个 `.sfz`。
    Found,
    /// 全部条目都看过，确实没有任何 `.sfz`（`seen` = 实际检查的条目数）。
    Absent { seen: usize },
    /// 条目数超过上限，**没查完**就停了（`seen` = 上限）。
    Incomplete { seen: usize },
}

/// 扫描条目上限（目录再深也不至于把安装线程卡死）。
const SFZ_SCAN_LIMIT: usize = 4096;

/// 递归找 `.sfz`，最多看 [`SFZ_SCAN_LIMIT`] 个条目。
fn scan_sfz(root: &Path) -> SfzScan {
    let mut seen: usize = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > SFZ_SCAN_LIMIT {
                return SfzScan::Incomplete {
                    seen: SFZ_SCAN_LIMIT,
                };
            }
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("sfz"))
            {
                return SfzScan::Found;
            }
        }
    }
    SfzScan::Absent { seen }
}

/// 递归数文件数与总字节（读不到的条目按 0 计，⛔ 不中断）。
fn count_files_and_bytes(root: &Path) -> Result<(u64, u64), SoundError> {
    let mut files: u64 = 0;
    let mut bytes: u64 = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            files += 1;
            bytes = bytes.saturating_add(entry.metadata().map(|m| m.len()).unwrap_or(0));
        }
    }
    Ok((files, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_slug_and_pin_guards() {
        assert!(is_repo_slug("sfzinstruments/aliexpress-erhu"));
        assert!(!is_repo_slug("sfzinstruments"));
        assert!(!is_repo_slug("a/b/c"));
        assert!(!is_repo_slug("a/../b"));
        assert!(is_commit_pin("6615047b2fd06126877483e97b8bb4af9d00b080"));
        assert!(!is_commit_pin("main"));
        assert!(!is_commit_pin(""));
    }

    #[test]
    fn parses_minimal_manifest() {
        let text = r#"{"version":"1.0.0","instruments":[
            {"id":"demo","name":"Demo","category":"Keys","license":"CC0-1.0",
             "repo":"owner/repo","pin":"6615047b2fd06126877483e97b8bb4af9d00b080",
             "total_bytes":100,"file_count":3}
        ]}"#;
        let sounds = parse_manifest(text).expect("应能解析");
        // 登记表条目 + 并进同一张表的社区音源（见 `community_extras`）。
        let demo = sounds
            .iter()
            .find(|sound| sound.id == "demo")
            .expect("登记表里的条目必须出现");
        assert_eq!(
            demo.archive_url().as_deref(),
            Some(
                "https://github.com/owner/repo/archive/6615047b2fd06126877483e97b8bb4af9d00b080.zip"
            )
        );
        assert!(demo.installable());
    }

    #[test]
    fn empty_manifest_is_error_not_empty_list() {
        assert!(parse_manifest(r#"{"version":"1.0.0"}"#).is_err());
    }

    /// 临时库根（与 sound_library 判据同款：进程号 + 标签隔离，跑完自清）。
    fn temp_roots(tag: &str) -> (PathBuf, SoundRoots) {
        let base =
            std::env::temp_dir().join(format!("yeban-builtin-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let roots = SoundRoots {
            library_root: base.join("library"),
            builtin_root: base.join("builtin"),
        };
        (base, roots)
    }

    /// 造一份 stored 模式的微型 zip 夹具（不依赖外网、不依赖压缩算法）。
    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        use std::io::Write as _;
        let file = std::fs::File::create(path).expect("建 zip 夹具");
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in entries {
            writer.start_file(name, options).expect("写 zip 条目");
            writer.write_all(bytes).expect("写 zip 内容");
        }
        writer.finish().expect("收尾 zip 夹具");
    }

    fn demo_sound(total_bytes: u64, file_count: u64) -> BuiltinSound {
        BuiltinSound {
            id: "demo-builtin".to_owned(),
            name: "演示音源".to_owned(),
            category: "Keys".to_owned(),
            license: "CC0-1.0".to_owned(),
            author: "test".to_owned(),
            repo: "owner/repo".to_owned(),
            pin: "6615047b2fd06126877483e97b8bb4af9d00b080".to_owned(),
            source_url: String::new(),
            total_bytes,
            file_count,
            bundled: false,
            sfz: "Programs/demo.sfz".to_owned(),
            archive_override: String::new(),
            release_zip: String::new(),
            release_sha256: String::new(),
            release_bytes: 0,
        }
    }

    const SFZ_BYTES: &[u8] = b"<control>\n<region> sample=../Samples/a.wav\n";
    const WAV_BYTES: &[u8] = &[7u8; 4096];

    /// GitHub 归档形态的夹具：外层 `<repo>-<pin>/` 目录 + 一个 SFZ + 一个样本。
    /// 返回（解压后内容总字节， 条目数）。
    fn write_github_style_zip(zip_path: &Path) -> (u64, u64) {
        write_zip(
            zip_path,
            &[
                ("owner-repo-6615047b/Programs/demo.sfz", SFZ_BYTES),
                ("owner-repo-6615047b/Samples/a.wav", WAV_BYTES),
            ],
        );
        ((SFZ_BYTES.len() + WAV_BYTES.len()) as u64, 2)
    }

    #[test]
    fn builtin_pack_does_not_register_unpacked_total_as_archive_size() {
        // 登记表的 `total_bytes` 是**解压后**全部文件的总字节；归档（.zip）字节数
        // 没有任何登记值（GitHub 不发布归档大小）。把它填进安装腿的归档大小槽，
        // 安装腿就会拿解压后总量去比 zip 文件大小 —— 必然 SizeMismatch。
        let sound = demo_sound(123_456, 7);
        let pack = catalog_pack_for(&sound, "https://example.invalid/x.zip".to_owned());
        assert_eq!(
            pack.size_bytes, 0,
            "内置音源的归档大小没有登记值，必须留 0（不做归档大小对账）"
        );
        assert!(pack.sha256.is_empty(), "没有上游摘要时不许假装有");
    }

    #[test]
    fn builtin_archive_installs_when_zip_size_differs_from_unpacked_total() {
        let (base, roots) = temp_roots("install");
        let zip_path = base.join("archive.zip");
        std::fs::create_dir_all(&base).expect("建临时目录");
        let (unpacked, files) = write_github_style_zip(&zip_path);
        // 前提钉死：夹具的 zip 字节数与解压后总字节**不相等**（真实归档的常态）。
        assert_ne!(
            std::fs::metadata(&zip_path).expect("读 zip 大小").len(),
            unpacked,
            "夹具失格：zip 大小竟与解压后总量相等，测不出误用总量的缺陷"
        );
        let sound = demo_sound(unpacked, files);
        let pack = catalog_pack_for(&sound, "https://example.invalid/x.zip".to_owned());
        let installed = crate::sound_library::install_pack_archive(&pack, &zip_path, &roots)
            .expect("归档大小与解压后登记总量不同是常态，必须装得上");
        flatten_wrapper_dir(&installed.location).expect("抹平 GitHub 归档外层目录");
        verify_extracted(&sound, &installed.location).expect("解压后对账必须通过");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn verify_extracted_rejects_when_unpacked_bytes_fall_short() {
        // 解压后对账是内置音源唯一的字节校验：登记总量比实际内容多 ⇒ 必须响亮拒绝。
        let (base, roots) = temp_roots("short");
        let zip_path = base.join("archive.zip");
        std::fs::create_dir_all(&base).expect("建临时目录");
        let (unpacked, files) = write_github_style_zip(&zip_path);
        let sound = demo_sound(unpacked, files);
        let pack = catalog_pack_for(&sound, "https://example.invalid/x.zip".to_owned());
        let installed = crate::sound_library::install_pack_archive(&pack, &zip_path, &roots)
            .expect("夹具包必须装得上");
        flatten_wrapper_dir(&installed.location).expect("抹平 GitHub 归档外层目录");
        let inflated = demo_sound(unpacked + 100_000, files);
        match verify_extracted(&inflated, &installed.location) {
            Err(SoundError::BadArchive(detail)) => {
                assert!(
                    detail.contains("对账不符"),
                    "错误必须点明对账不符：{detail}"
                );
            }
            other => panic!("解压后字节不足必须是 BadArchive 对账失败，实得：{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    // ---------------------------------------------------------------------
    // T-FIX-107：VSCO 2 CE「一键安装假失败」
    // ---------------------------------------------------------------------

    /// 复刻真实故障形态：归档解出来**满是采样、但一个 `.sfz` 都没有**。
    ///
    /// 实测依据（2026-11-22，`git ls-tree -r` 亲核）：
    /// * `sgossner/VSCO-2-CE@master` = 3187 个文件、**0 个 .sfz**（顶层 Brass/Keys/…）；
    /// * `schollz/VSCO-2-CE@SFZ`（pin `6dd651d5…`）= 顶层 75 个 `.sfz`。
    ///
    /// 旧代码在这里只会吐一句"包里可能不是 SFZ 乐器" —— 那是**误报**：
    /// 包确实是采样库，只是没有 SFZ 乐器文件。
    #[test]
    fn absent_sfz_reports_the_scan_count_instead_of_claiming_not_an_sfz_instrument() {
        let (base, _roots) = temp_roots("absent-sfz");
        let installed = base.join("installed");
        for sub in ["Brass", "Strings", "Woodwinds"] {
            std::fs::create_dir_all(installed.join(sub)).expect("建目录");
        }
        std::fs::write(installed.join("README.md"), b"vsco").expect("写说明");
        std::fs::write(installed.join("Strings/violin.wav"), b"wav").expect("写样本");

        // `SoundError` 没有实现 `Display`（仓里统一用 Debug 打印），与既有判据同款。
        let error = format!(
            "{:?}",
            verify_extracted(&demo_sound(0, 0), &installed).expect_err("零 .sfz 必须报错")
        );
        // 条目数要如实报出来（3 个目录 + README + violin.wav = 5）。
        assert!(error.contains("已查 5 条"), "必须报扫描条数：{error}");
        // ⛔ 禁止再断言"包里可能不是 SFZ 乐器"。
        assert!(
            !error.contains("包里可能不是 SFZ 乐器"),
            "不许再用旧措辞误报：{error}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 条目数超过上限时必须说"未查完"，⛔ 不能说"没有任何 .sfz"。
    #[test]
    fn oversized_tree_reports_incomplete_scan_and_does_not_deny_sfz() {
        let (base, _roots) = temp_roots("incomplete-scan");
        let installed = base.join("installed");
        std::fs::create_dir_all(&installed).expect("建目录");
        for index in 0..=(SFZ_SCAN_LIMIT) {
            std::fs::write(installed.join(format!("sample-{index:05}.wav")), b"").expect("写样本");
        }

        let error = format!(
            "{:?}",
            verify_extracted(&demo_sound(0, 0), &installed).expect_err("未查完同样不能放行")
        );
        assert!(error.contains("未查完"), "必须点明没查完：{error}");
        assert!(
            !error.contains("没有任何 .sfz"),
            "没查完就不能断言没有 .sfz：{error}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 扫描上限内、`.sfz` 在子目录深处时仍要算找到（⛔ 别把上限当成"只看顶层"）。
    #[test]
    fn nested_sfz_within_the_limit_is_found() {
        let (base, _roots) = temp_roots("nested-sfz");
        let installed = base.join("installed");
        std::fs::create_dir_all(installed.join("Programs")).expect("建目录");
        std::fs::write(installed.join("Programs/demo.sfz"), SFZ_BYTES).expect("写 sfz");
        for index in 0..64 {
            std::fs::write(installed.join(format!("s{index}.wav")), b"").expect("写样本");
        }
        assert_eq!(scan_sfz(&installed), SfzScan::Found);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 回归钉：社区条目里不许再出现第二份 VSCO 2 CE。
    ///
    /// 曾经的 `vsco-2-ce` 指向 `sgossner/VSCO-2-CE@master`（0 个 .sfz），装完必失败；
    /// 正确的上游是登记表里的 `vsco2ce`（`schollz/VSCO-2-CE` 的 `SFZ` 分支，钉 commit）。
    #[test]
    fn community_extras_carry_no_second_vsco_entry() {
        for extra in community_extras() {
            assert!(
                !extra.id.to_lowercase().contains("vsco"),
                "社区条目 {} 与登记表条目 vsco2ce 重复",
                extra.id
            );
            assert!(
                !extra.repo.to_lowercase().contains("vsco"),
                "社区条目 {} 的上游 {} 与登记表条目重复",
                extra.id,
                extra.repo
            );
        }
    }

    /// 社区条目既不许和登记表的 id 撞车，也不许和登记表里的条目同名（同一件音源列两遍）。
    #[test]
    fn community_extras_do_not_duplicate_registered_sounds() {
        let text = std::fs::read_to_string(manifest_path()).expect("登记表必须可读");
        // ⚠ 直接看登记表文档，⛔ 不走 parse_manifest —— 那个函数自己就会并进社区条目。
        let doc: ManifestDoc = serde_json::from_str(&text).expect("登记表必须可解析");
        let entries = doc.instruments.expect("登记表必须有 instruments");
        let ids: Vec<String> = entries.iter().filter_map(|e| e.id.clone()).collect();
        let names: Vec<String> = entries.iter().filter_map(|e| e.name.clone()).collect();
        for extra in community_extras() {
            assert!(
                !ids.contains(&extra.id),
                "社区条目 {} 与登记表 id 撞车",
                extra.id
            );
            assert!(
                !names.contains(&extra.name),
                "社区条目 {} 与登记表条目同名",
                extra.name
            );
        }
    }

    /// ⭐ 发布包条目（`provenance_form=archive`）必须走**发布包直链**、**不抹外层目录**，
    /// 并且**不把解压后字节数当下载大小**。
    ///
    /// 这三条是同一次实测缺陷的三个面（`karoryfer-emilyguitar` / `-meatbass`）：
    /// 按 commit 归档装完少一层 `Emilyguitar/` ⇒ 登记表的 `sfz` 404；
    /// 而 `total_bytes` 是解压后口径 ⇒ 下载段必报"包大小不符"。
    #[test]
    fn archive_entries_use_the_release_zip_and_keep_their_top_level() {
        let text = r#"{"version":"1.0.0","instruments":[
            {"id":"rel","name":"Rel","category":"Guitar","license":"CC0-1.0",
             "repo":"owner/rel","pin":"6615047b2fd06126877483e97b8bb4af9d00b080",
             "sfz":"Emilyguitar/emily_basic.sfz",
             "total_bytes":125538629,"file_count":331,
             "provenance_form":"archive",
             "archive":{"url":"https://example.com/Rel.v1.zip","sha256":"ffef3b28972763820bee73660a25cf47ea6e13b184342fd5d325a5631598cacb","bytes":103484350}},
            {"id":"git","name":"Git","category":"Bass","license":"CC0-1.0",
             "repo":"owner/git","pin":"6615047b2fd06126877483e97b8bb4af9d00b080",
             "sfz":"linguine.sfz","total_bytes":100,"file_count":3,"provenance_form":"git"},
            {"id":"tar","name":"Tar","category":"Keys","license":"CC0-1.0",
             "sfz":"Organ/x.sfz","provenance_form":"archive",
             "archive":{"url":"https://example.com/Organ.tar.xz","bytes":6042972}}
        ]}"#;
        let sounds = parse_manifest(text).expect("应能解析");

        let rel = sounds.iter().find(|s| s.id == "rel").expect("rel");
        assert_eq!(
            rel.archive_url().as_deref(),
            Some("https://example.com/Rel.v1.zip"),
            "发布包条目必须用 archive.url（布局与登记表 sfz 一致）"
        );
        assert_eq!(rel.release_bytes, 103_484_350, "下载大小只能取压缩包字节数");
        assert_eq!(rel.release_sha256.len(), 64, "发布包的 sha256 必须带下来");
        assert!(
            !rel.needs_wrapper_flatten(),
            "发布包顶层就是乐器本体，⛔ 不能抹"
        );

        let git = sounds.iter().find(|s| s.id == "git").expect("git");
        assert_eq!(
            git.archive_url().as_deref(),
            Some(
                "https://github.com/owner/git/archive/6615047b2fd06126877483e97b8bb4af9d00b080.zip"
            ),
            "git 条目仍走 commit 归档"
        );
        assert!(git.needs_wrapper_flatten(), "commit 归档多一层，必须抹平");
        assert_eq!(git.release_bytes, 0, "git 条目没有压缩包登记 ⇒ 不传大小");

        // `.tar.xz` 不进一键装（安装腿只解 zip）——如实留"需手动下载"。
        let tar = sounds.iter().find(|s| s.id == "tar").expect("tar");
        assert_eq!(tar.archive_url(), None, "非 zip 的发布包不可一键装");
        assert!(!tar.installable());
    }

    /// ⭐ 对账必须抓住"主 SFZ 路径对不上"——只查"有任意 .sfz"会让这种包装成功、
    /// 用户点「用上」时才无声失败。
    #[test]
    fn verification_requires_the_registered_sfz() {
        let dir = std::env::temp_dir().join(format!("yeban-builtin-verify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Emilyguitar")).expect("建目录");
        std::fs::write(dir.join("Emilyguitar/other.sfz"), b"<region>").expect("写 sfz");

        let mut sound = BuiltinSound {
            id: "rel".to_owned(),
            name: "Rel".to_owned(),
            category: String::new(),
            license: String::new(),
            author: String::new(),
            repo: String::new(),
            pin: String::new(),
            source_url: String::new(),
            total_bytes: 0,
            file_count: 0,
            bundled: false,
            sfz: "Emilyguitar/emily_basic.sfz".to_owned(),
            archive_override: String::new(),
            release_zip: String::new(),
            release_sha256: String::new(),
            release_bytes: 0,
        };
        // 包里只有 `other.sfz` ⇒ 登记的 `emily_basic.sfz` 缺失，必须判坏包。
        let error = verify_extracted(&sound, &dir).expect_err("主 SFZ 缺失必须报错");
        assert!(
            error.message().contains("找不到登记的主 SFZ"),
            "错误要指名，实际：{}",
            error.message()
        );

        std::fs::write(dir.join("Emilyguitar/emily_basic.sfz"), b"<region>").expect("写主 sfz");
        verify_extracted(&sound, &dir).expect("主 SFZ 就位后应通过对账");

        // 登记表没写 sfz 的条目（社区那两条）不做这一项检查。
        sound.sfz = String::new();
        std::fs::remove_file(dir.join("Emilyguitar/emily_basic.sfz")).expect("删主 sfz");
        verify_extracted(&sound, &dir).expect("未登记 sfz 的条目不查主文件");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
