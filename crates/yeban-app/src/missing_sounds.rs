//! ⭐ **打开工程时的"音源缺失"引导**（设计原则见 `docs/UX-错误与提醒设计原则.md`）。
//!
//! ## 它解决的是哪一个体验缺口
//!
//! 工程里某条轨绑着一份 SFZ，而那件音源没装在本地时，引擎起不来。此前这件事
//! **只在终端里**喊一句 `SFZ 摊平失败: 读 SFZ 失败: SFZ 路径 …` —— 界面照常打开，
//! 用户按播放没声，既不知道原因，也不知道该做什么。
//!
//! 本模块把"哪条轨、缺哪件、能不能一键装回来"**在进界面之前**算清楚，交给宿主
//! 播给界面：能对上内置清单的 ⇒ 直接筛出可下载项；对不上的 ⇒ 如实说"清单里没有"，
//! 并把轨道名留给用户自己挑。
//!
//! ⚠ 本模块**只读**：不写工程、不下载、不碰撤销树。所有动作由用户点一下触发。

use std::path::Path;

use crate::builtin_sounds::{BuiltinSound, load_builtin_sounds};
use crate::sfz_load::SfzRoots;

/// 一条轨缺的那件音源。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingSound {
    /// 轨道在界面里的顺序（0 起；与 `track-<i>-*` 同一口径）。
    pub track_index: i32,
    /// 轨道名（界面上就是它）。
    pub track_name: String,
    /// 模型里写下的那条路径原文。
    pub path: String,
    /// 能对上内置清单时：包 id（= 安装后的目录名，也是搜索框直接能筛到的词）。
    pub pack_id: Option<String>,
    /// 能对上内置清单时：显示名。
    pub pack_name: Option<String>,
    /// 这条轨**大概是哪种乐器**（按轨名与路径推，见 [`infer_kind`]）。
    pub kind: Kind,
    /// 可以一键换上的同类音源（至多 3 件；空 = 清单里连同类都没有）。
    pub suggestions: Vec<Suggestion>,
}

/// 乐器大类 —— 只用来**挑相近的替代品**，不参与任何音频判定。
///
/// 取值刻意与内置清单的 `category` 字段同名（`Acoustic Piano` / `Bass` …），
/// 这样"推荐"就是一次按分类筛选，而不是再造一份映射表。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// 钢琴 / 电钢 / 拇指琴这类键盘音。
    Keys,
    /// 贝斯（含合成贝斯、肉贝斯这类采样贝斯）。
    Bass,
    /// 吉他（含失真、古典、班卓）。
    Guitar,
    /// 鼓组 / 打击乐。
    Drums,
    /// 弦乐（提琴、低音提琴、二胡、竖琴、琵琶）。
    Strings,
    /// 管乐（萨克斯、笛、风笛、手风琴这类"吹"的）。
    Winds,
    /// 人声。
    Vocals,
    /// 风琴。
    Organ,
    /// 认不出来 —— 只做"按体积从轻到重"的兜底推荐。
    Other,
}

impl Kind {
    /// 对应内置清单里的 `category`（`None` = 清单里没有这个大类）。
    #[must_use]
    pub fn category(self) -> Option<&'static str> {
        match self {
            Self::Keys => Some("Acoustic Piano"),
            Self::Bass => Some("Bass"),
            Self::Guitar => Some("Guitar"),
            Self::Drums => Some("Acoustic Drums"),
            Self::Strings => Some("Orchestral"),
            Self::Winds => Some("Winds"),
            Self::Vocals => Some("Vocals"),
            Self::Organ => Some("Organ"),
            Self::Other => None,
        }
    }

    /// 界面上给用户看的大类名（"同类：贝斯"里那个词）。
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Keys => "键盘",
            Self::Bass => "贝斯",
            Self::Guitar => "吉他",
            Self::Drums => "鼓组",
            Self::Strings => "弦乐",
            Self::Winds => "管乐",
            Self::Vocals => "人声",
            Self::Organ => "风琴",
            Self::Other => "乐器",
        }
    }
}

/// 一件"可以一键换上去"的候选音源。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// 清单里的包 id。
    pub id: String,
    /// 显示名（按钮上就是它）。
    pub name: String,
    /// **可存进工程**的路径（`assets/samples/<目录>/<sfz>`）。
    pub path: String,
    /// 本地是否已经有了（`true` = 点了立刻能响；`false` = 还要先下载）。
    pub installed: bool,
}

/// 按轨名 + 路径猜乐器大类。
///
/// 顺序即优先级：**轨名**比路径更可信（用户改过名的轨叫「贝斯 DI」就是贝斯，
/// 哪怕它绑的是一条随便找来的 sfz）。中英文关键词都认，因为两类都能见到。
#[must_use]
pub fn infer_kind(track_name: &str, path: &str) -> Kind {
    let probe = format!("{track_name} {}", file_stem(path)).to_lowercase();
    // ⚠ 先判"贝斯"，再判"吉他"：`贝斯`里没有 `吉他`，但英文 `bass guitar` 两词都在，
    // 而它确实是贝斯 —— 顺序在这里是语义，不是随手排的。
    const RULES: &[(Kind, &[&str])] = &[
        (Kind::Bass, &["bass", "贝斯", "贝司", "低音吉他"]),
        (
            Kind::Drums,
            &["drum", "kick", "snare", "perc", "鼓", "打击", "军鼓"],
        ),
        (
            Kind::Guitar,
            &["guitar", "banjo", "ganjo", "吉他", "吉它", "班卓"],
        ),
        (
            Kind::Keys,
            &[
                "piano", "keys", "kalimba", "celesta", "琴键", "钢琴", "电钢", "键盘",
            ],
        ),
        (
            Kind::Strings,
            &[
                "cello", "violin", "viola", "bass", "harp", "erhu", "zither", "sitar", "提琴",
                "大提", "小提", "中提", "二胡", "竖琴", "古筝", "琵琶", "弦",
            ],
        ),
        (
            Kind::Winds,
            &[
                "sax",
                "flute",
                "pipe",
                "oboe",
                "clarinet",
                "trumpet",
                "accordion",
                "brass",
                "萨克斯",
                "萨克斯风",
                "笛",
                "管",
                "小号",
                "手风琴",
                "唢呐",
            ],
        ),
        (
            Kind::Vocals,
            &[
                "vocal", "voice", "choir", "sing", "人声", "主唱", "和声", "帮腔", "合唱",
            ],
        ),
        (Kind::Organ, &["organ", "hammond", "风琴", "管风琴"]),
    ];
    for (kind, words) in RULES {
        if words.iter().any(|word| probe.contains(word)) {
            return *kind;
        }
    }
    Kind::Other
}

/// 文件名主干（`samples/bass/ebass_rel.sfz` ⇒ `ebass_rel`）。
fn file_stem(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let name = normalized.rsplit('/').next().unwrap_or_default();
    name.strip_suffix(".sfz").unwrap_or(name).to_owned()
}

/// 按"同类优先、已装优先、体积轻优先"挑至多 `limit` 件候选。
///
/// 三条排序理由都是用户侧的真实取舍：
/// 1. **同类**：换一件萨克斯去顶贝斯轨，声音就已经不是这首歌了；
/// 2. **已装**：用户此刻要的是"马上能响"，不是"再等 700 MB 下载"；
/// 3. **轻**：非要下载时，先下小的（同分类里有 1.1 GB 的也有 6 MB 的）。
#[must_use]
pub fn suggest(
    sounds: &[BuiltinSound],
    kind: Kind,
    roots: SfzRoots<'_>,
    limit: usize,
) -> Vec<Suggestion> {
    let library_root = roots
        .library_root
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(crate::samples::library_root);
    let mut candidates: Vec<(Suggestion, u64)> = sounds
        .iter()
        .filter(|sound| match kind.category() {
            Some(category) => sound.category == category,
            // 认不出乐器 ⇒ 不假装懂行，全清单参与（后面按体积从轻到重）。
            None => true,
        })
        .filter_map(|sound| {
            let (path, installed) = candidate_path(sound, &library_root, roots)?;
            Some((
                Suggestion {
                    id: sound.id.clone(),
                    name: sound.name.clone(),
                    path,
                    installed,
                },
                sound.total_bytes,
            ))
        })
        .collect();
    candidates.sort_by(|left, right| {
        right
            .0
            .installed
            .cmp(&left.0.installed)
            .then(left.1.cmp(&right.1))
            .then(left.0.id.cmp(&right.0.id))
    });
    candidates
        .into_iter()
        .take(limit)
        .map(|(suggestion, _)| suggestion)
        .collect()
}

/// 一件清单条目在**本机**能解析到的路径（`None` = 连解析都过不了，不能推荐）。
///
/// 两步：先按"安装后应有的样子"拼（`assets/samples/<id>/<sfz>`）再验；验不过就
/// 扫库根下**目录名与 id 近似**的那一份（`virtuosity-drums-basic` 装成
/// `virtuosity-drums` 这种历史差异，⛔ 不该让推荐整条消失）。
fn candidate_path(
    sound: &BuiltinSound,
    library_root: &std::path::Path,
    roots: SfzRoots<'_>,
) -> Option<(String, bool)> {
    if sound.sfz.trim().is_empty() {
        return None;
    }
    let expected = format!(
        "assets/samples/{}/{}",
        sound.id,
        sound.sfz.replace('\\', "/")
    );
    if crate::sfz_load::resolve_sampler_path(&expected, roots).is_ok() {
        return Some((expected, true));
    }
    // 目录名与 id 不完全一致时：扫两个根，找最像的那一个目录。
    let key = squashed(&sound.id);
    let builtin_root = crate::sfz_load::repo_root().join("assets/samples");
    for root in [library_root.to_path_buf(), builtin_root] {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if squashed(&name) != key {
                continue;
            }
            let path = format!("assets/samples/{name}/{}", sound.sfz.replace('\\', "/"));
            if crate::sfz_load::resolve_sampler_path(&path, roots).is_ok() {
                return Some((path, true));
            }
        }
    }
    // 没装：仍把"装上之后会在哪儿"作为候选路径给出（点了先下载）。
    Some((expected, false))
}

/// 某个清单包**此刻**在本机能解析到的 `.sfz` 路径（`None` = 还没装 / 装得不完整）。
///
/// 用途只有一处：用户在救援面板点了「下载」，装完那一刻要**立刻**把这条轨换上
/// （见 `host::rescue_after_pack_installed`）。⛔ 它不猜"装了以后会在哪儿" ——
/// 只有 `resolve_sampler_path` 真的解得开才算装好，否则宁可不换也不给一条死路径。
#[must_use]
pub fn installed_path(pack_id: &str, roots: SfzRoots<'_>) -> Option<String> {
    let sounds = load_builtin_sounds().ok()?;
    let sound = sounds
        .iter()
        .find(|sound| sound.id == pack_id)
        .or_else(|| sounds.iter().find(|sound| squashed(&sound.id) == squashed(pack_id)))?;
    let library_root = roots
        .library_root
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(crate::samples::library_root);
    match candidate_path(sound, &library_root, roots) {
        Some((path, true)) => Some(path),
        _ => None,
    }
}

/// 每条轨最多给几个"换成这件"候选（用户在救援面板里一眼扫完的上限）。
pub const SUGGESTION_LIMIT: usize = 3;

/// 收集"绑了 SFZ 但本地解析不到"的轨道。
///
/// 判据与渲染**同一条**：都走 [`crate::sfz_load::resolve_sampler_path`]（工程目录 ⇒
/// 库根 ⇒ 仓库根 ⇒ cwd）。⛔ 这里不另写一份"文件在不在"的判断 —— 否则会出现
/// "界面说缺、引擎能读"这种自相矛盾。
#[must_use]
pub fn collect(
    project: &yeban_model::YebanProjectV1,
    project_dir: Option<&Path>,
) -> Vec<MissingSound> {
    let roots = match project_dir {
        Some(dir) => SfzRoots::under_project(dir),
        None => SfzRoots::none(),
    };
    // 清单只读一次（8.7 MB）；挂在这个调用里，⛔ 不逐轨重读。
    let catalog: Option<Vec<BuiltinSound>> = load_builtin_sounds().ok();
    let mut missing = Vec::new();
    // ⚠ `track_index` 必须与界面**同一套顺序**：`ViewState::tracks` 的行下标来自
    // `YebanProjectV1::display_track_ids()`（主总线不进 `tracks`，也不占行）。
    // 早先这里遍历 `project.tracks.values()`（`BTreeMap` 身份升序、且含主总线）⇒
    // 主总线的身份一旦排在展示轨前面，救援行就会指到**下一条**轨上：用户点「换成 X」
    // 改的是别的轨（或在末行直接改不动）。`tests/missing_sounds_rescue_flow.rs` 用
    // "换完读回权威工程"抓的就是这一格。
    for (index, id) in project.display_track_ids().iter().enumerate() {
        let Some(track) = project.tracks.get(id) else {
            continue;
        };
        let Some(binding) = track.sampler.as_ref() else {
            continue;
        };
        if binding.path.trim().is_empty() {
            continue;
        }
        if crate::sfz_load::resolve_sampler_path(&binding.path, roots).is_ok() {
            continue;
        }
        let (pack_id, pack_name) = catalog
            .as_ref()
            .and_then(|sounds| match_by_path(sounds, &binding.path))
            .map_or((None, None), |sound| {
                (Some(sound.id.clone()), Some(sound.name.clone()))
            });
        // ⭐ 不在清单里 ⇒ **按乐器大类推荐**（用户口径：不在常见清单里也得能"点一下
        // 换个类似的"，⛔ 不是丢一句"清单里没有"就把人卡住）。
        let kind = infer_kind(&track.name, &binding.path);
        let suggestions = catalog.as_ref().map_or_else(Vec::new, |sounds| {
            suggest(sounds, kind, roots, SUGGESTION_LIMIT)
        });
        missing.push(MissingSound {
            track_index: i32::try_from(index).unwrap_or(0),
            track_name: track.name.clone(),
            path: binding.path.clone(),
            pack_id,
            pack_name,
            kind,
            suggestions,
        });
    }
    missing
}

/// 路径 ⇒ 清单条目。
///
/// 两档匹配，先精确后宽松：
/// 1. 路径里出现 `assets/samples/<id>/`（或库根下 `<id>/`）⇒ 取 `<id>` 精确比对；
/// 2. 都不中 ⇒ 拿路径最后一段（去掉 `.sfz`）与条目的 `id`/`name` 做包含匹配。
#[must_use]
pub fn match_by_path<'a>(sounds: &'a [BuiltinSound], path: &str) -> Option<&'a BuiltinSound> {
    let normalized = path.replace('\\', "/");
    let trimmed = normalized
        .strip_prefix("assets/samples/")
        .unwrap_or(&normalized);
    let first = trimmed.split('/').next().unwrap_or_default();
    // ① 相对仓库根的写法：`assets/samples/<id>/…` ⇒ 第一段就是 id。
    if let Some(found) = sounds.iter().find(|sound| sound.id == first) {
        return Some(found);
    }
    // ② 绝对路径（用户自己挑过文件 / 工程挪到别的机器）：逐段比对 id ——
    //    `…\samples\aliexpress-erhu\Programs\02.sfz` 里只有 `aliexpress-erhu` 这一段是 id。
    for segment in normalized.split('/') {
        if let Some(found) = sounds.iter().find(|sound| sound.id == segment) {
            return Some(found);
        }
    }
    // ③ 兜底：拿文件名主干与条目的 id / 显示名做**忽略分隔符**的比对
    //    （`SalamanderGrandPiano.sfz` ↔ `salamander-grand-piano` 这种写法差）。
    let stem = Path::new(&normalized)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let stem_key = squashed(stem);
    if stem_key.is_empty() {
        return None;
    }
    sounds.iter().find(|sound| {
        let id_key = squashed(&sound.id);
        let name_key = squashed(&sound.name);
        id_key.contains(&stem_key) || name_key.contains(&stem_key)
    })
}

/// 只留字母数字并转小写：`SalamanderGrandPiano` ⇒ `salamandergrandpiano`，
/// `salamander-grand-piano` ⇒ 同一个键 —— 用来吃掉横线/下划线/空格造成的写法差。
fn squashed(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_sound(id: &str, name: &str) -> BuiltinSound {
        BuiltinSound {
            id: id.to_owned(),
            name: name.to_owned(),
            category: "Keys".to_owned(),
            license: "CC0-1.0".to_owned(),
            author: String::new(),
            repo: "owner/repo".to_owned(),
            pin: "6615047b2fd06126877483e97b8bb4af9d00b080".to_owned(),
            source_url: String::new(),
            total_bytes: 0,
            file_count: 0,
            bundled: false,
            sfz: String::new(),
            archive_override: String::new(),
        }
    }

    #[test]
    fn matches_by_assets_samples_prefix() {
        let sounds = vec![sample_sound("aliexpress-erhu", "AliExpress erhu")];
        assert_eq!(
            match_by_path(&sounds, "assets/samples/aliexpress-erhu/Programs/02.sfz")
                .map(|s| s.id.as_str()),
            Some("aliexpress-erhu")
        );
        assert_eq!(
            match_by_path(
                &sounds,
                "C:\\Users\\me\\samples\\aliexpress-erhu\\Programs\\02.sfz"
            )
            .map(|s| s.id.as_str()),
            Some("aliexpress-erhu")
        );
    }

    #[test]
    fn matches_by_stem_when_prefix_absent() {
        let sounds = vec![sample_sound(
            "salamander-grand-piano",
            "Salamander Grand Piano V3",
        )];
        assert_eq!(
            match_by_path(&sounds, "piano/SalamanderGrandPiano.sfz").map(|s| s.id.as_str()),
            Some("salamander-grand-piano")
        );
        assert!(match_by_path(&sounds, "unknown/thing.sfz").is_none());
    }

    /// 判据：**轨名**是主要线索（用户改过名的轨就该按名字认），路径只是补充。
    #[test]
    fn infers_kind_from_track_name_first() {
        assert_eq!(
            infer_kind("贝斯 DI（线条主角）", "samples/bass/ebass_rel.sfz"),
            Kind::Bass
        );
        assert_eq!(
            infer_kind("Clean 吉他", "samples/gt/clean.sfz"),
            Kind::Guitar
        );
        assert_eq!(
            infer_kind("Fuzz 吉他", "assets/samples/fuzz.sfz"),
            Kind::Guitar
        );
        assert_eq!(infer_kind("主唱", "x.sfz"), Kind::Vocals);
        assert_eq!(infer_kind("帮腔", "x.sfz"), Kind::Vocals);
        assert_eq!(infer_kind("弦乐组", "x.sfz"), Kind::Strings);
        assert_eq!(infer_kind("大提琴", "x.sfz"), Kind::Strings);
        // 贝斯优先于"吉他"：`bass guitar` 两词都在，但它确实是贝斯。
        assert_eq!(infer_kind("Bass Guitar", "x.sfz"), Kind::Bass);
        // 认不出来 ⇒ Other（⛔ 不硬塞一个类别，让推荐走"从轻到重"的兜底）。
        assert_eq!(infer_kind("Track 02", "samples/zz.sfz"), Kind::Other);
    }

    /// 判据：同类优先、体积轻优先（已装优先由 `candidate_path` 决定，这里不碰磁盘）。
    #[test]
    fn suggestions_prefer_same_category_then_smaller() {
        let mut bass_big = sample_sound("bass-big", "Big Bass");
        bass_big.category = "Bass".to_owned();
        bass_big.total_bytes = 1_100_000_000;
        bass_big.sfz = "Big/Big.sfz".to_owned();
        let mut bass_small = sample_sound("bass-small", "Small Bass");
        bass_small.category = "Bass".to_owned();
        bass_small.total_bytes = 6_000_000;
        bass_small.sfz = "Small/Small.sfz".to_owned();
        let mut guitar = sample_sound("guitar", "A Guitar");
        guitar.category = "Guitar".to_owned();
        guitar.total_bytes = 5_000_000;
        guitar.sfz = "G/G.sfz".to_owned();
        let sounds = vec![bass_big, bass_small, guitar];

        let picked = suggest(&sounds, Kind::Bass, SfzRoots::none(), 2);
        assert_eq!(
            picked.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["bass-small", "bass-big"],
            "同类里应先给体积小的"
        );
        assert!(picked.iter().all(|s| s.id.starts_with("bass")));

        // 认不出乐器 ⇒ 全清单参与，仍按体积从轻到重。
        let picked = suggest(&sounds, Kind::Other, SfzRoots::none(), 3);
        assert_eq!(
            picked.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["guitar", "bass-small", "bass-big"]
        );
    }
}
