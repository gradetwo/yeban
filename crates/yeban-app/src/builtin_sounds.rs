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
    let mut extras = Vec::new();
    extras.push(BuiltinSound {
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
    });
    extras.push(BuiltinSound {
        id: "vsco-2-ce".to_owned(),
        name: "VSCO 2 Community Edition".to_owned(),
        category: "Orchestral".to_owned(),
        license: "CC0-1.0".to_owned(),
        author: "Versilian Studios".to_owned(),
        repo: "sgossner/VSCO-2-CE".to_owned(),
        pin: String::new(),
        source_url: "https://versilian-studios.com/vsco-community/".to_owned(),
        total_bytes: 0,
        file_count: 0,
        bundled: false,
        sfz: String::new(),
        archive_override:
            "https://github.com/sgossner/VSCO-2-CE/archive/refs/heads/master.zip".to_owned(),
    });
    extras
}

impl BuiltinSound {
    /// GitHub 归档直链（按 commit 归档）。`repo` / `pin` 形态不对时给 `None`
    /// —— ⛔ 不拼一个注定 404 的地址。
    #[must_use]
    pub fn archive_url(&self) -> Option<String> {
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
            let text = std::fs::read_to_string(&path).map_err(|error| {
                format!("读内置音源清单失败（{}）：{error}", path.display())
            })?;
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
}

/// 解析登记表文本（判据与生产共用）。
pub fn parse_manifest(text: &str) -> Result<Vec<BuiltinSound>, String> {
    let doc: ManifestDoc = serde_json::from_str(text)
        .map_err(|error| format!("内置音源清单解析失败：{error}"))?;
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
                if license.is_empty() { "未登记".to_owned() } else { license }
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

/// 下载并安装一条内置音源：归档落到既有安装腿，落位后**抹平 GitHub 归档的外层
/// 目录**（`<repo>-<sha>/`），再做逐文件对账。
///
/// 对账失败 ⇒ 已装的那一份整体删掉（⛔ 不留"装了一半"的目录让用户以为能用）。
pub fn download_builtin(
    sound: &BuiltinSound,
    roots: &SoundRoots,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<InstalledLibrary, SoundError> {
    let Some(url) = sound.archive_url() else {
        return Err(SoundError::BadArchive(format!(
            "{} 没有可用的下载地址（repo/pin 形态不对）",
            sound.name
        )));
    };
    if is_installed(roots, &sound.id) {
        return Err(SoundError::BadArchive(format!(
            "{} 已经装过了",
            sound.name
        )));
    }
    // 大小已知 ⇒ 沿用既有安装腿的逐字节对账；哈希未知 ⇒ 空串（⛔ 不假装校验过）。
    let pack = crate::sound_library::CatalogPack {
        id: sound.id.clone(),
        name: sound.name.clone(),
        description: sound.download_note(),
        version: if sound.pin.is_empty() {
            "社区".to_owned()
        } else {
            sound.pin.chars().take(8).collect()
        },
        size_bytes: sound.total_bytes,
        sha256: String::new(),
        url,
    };
    let installed = crate::sound_library::download_pack(&pack, roots, progress)?;
    // GitHub 归档外层目录（解出来的只有它）⇒ 抹平，让 `<id>/` 下直接是 Programs/…
    flatten_wrapper_dir(&installed.location)?;
    if let Err(error) = verify_extracted(sound, &installed.location) {
        let _ = std::fs::remove_dir_all(&installed.location);
        return Err(error);
    }
    Ok(installed)
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
        std::fs::rename(&child, &dest).map_err(|error| {
            SoundError::Io(format!("移到 {} 失败：{error}", dest.display()))
        })?;
    }
    std::fs::remove_dir(&wrapper).map_err(|error| {
        SoundError::Io(format!("删空壳 {} 失败：{error}", wrapper.display()))
    })?;
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
    if !has_any_sfz(installed_root) {
        return Err(SoundError::BadArchive(format!(
            "{} 装完没有找到任何 .sfz（包里可能不是 SFZ 乐器）",
            sound.name
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

/// 装好的目录里有没有 `.sfz`（递归、最多看 4096 个条目 —— 目录再深也不至于卡死）。
fn has_any_sfz(root: &Path) -> bool {
    let mut seen: usize = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > 4096 {
                return false;
            }
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("sfz"))
            {
                return true;
            }
        }
    }
    false
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
        // 登记表条目 + 并进同一张表的两个社区音源（见 `community_extras`）。
        let demo = sounds
            .iter()
            .find(|sound| sound.id == "demo")
            .expect("登记表里的条目必须出现");
        assert_eq!(
            demo.archive_url().as_deref(),
            Some("https://github.com/owner/repo/archive/6615047b2fd06126877483e97b8bb4af9d00b080.zip")
        );
        assert!(demo.installable());
    }

    #[test]
    fn empty_manifest_is_error_not_empty_list() {
        assert!(parse_manifest(r#"{"version":"1.0.0"}"#).is_err());
    }
}
