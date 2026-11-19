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
}

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
    for (index, track) in project.tracks.values().enumerate() {
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
        missing.push(MissingSound {
            track_index: i32::try_from(index).unwrap_or(0),
            track_name: track.name.clone(),
            path: binding.path.clone(),
            pack_id,
            pack_name,
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
}
