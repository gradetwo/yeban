//! 保存侧内嵌收集：把谱子**逐音实际触发**的样本 ＋ SFZ 定义文本收进会话 CAS 池。
//!
//! ## 口径（与渲染侧同一条，⛔ 不另写一套匹配）
//!
//! * 触发集 = [`crate::domain::sfz::triggered_sample_keys`]：`DemandLoadFilter`
//!   逐音算 ＋ `wants` 逐区域判定（力度缺省值、新音符缺省 `DEFAULT_VELOCITY = 100`、
//!   键位切换、CC 门控全部与渲染的按需装载同一口径；CC 触发的区域不在本次范围）。
//! * 映射表键 = [`crate::domain::sfz::embedded_sample_key`]（绑定目录 + `sample=`
//!   原串，词法折叠）；SFZ 本身的键 = 绑定路径原样。
//! * SFZ 文本逐字节原样进 CAS（⛔ 不重写 `sample=` 行）；`sample=*silence` 不是
//!   文件、⛔ 不进集；含 `#include` 的 SFZ 整轨跳过（展开后的片段无法用单文件键索引）。
//!
//! ## 字节来源（盘上优先，已内嵌复用，缺了响亮失败）
//!
//! * SFZ：`resolve_sampler_path`（工程目录 ⇒ 库根 ⇒ 仓库根 ⇒ cwd）找到 ⇒ 读盘；
//!   找不到 ⇒ 工程旧映射表里的副本（跨机重存时 SFZ 已不在盘上）；两者皆无 ⇒
//!   响亮失败（`samplerUnreadable`），⛔ 不跳过该轨继续存。
//! * 样本：磁盘 SFZ 腿 ⇒ `resolve_sample_path`（SFZ 所在目录 ＋ `sample=` 原串，
//!   与渲染同一条）；已内嵌 SFZ 腿 ⇒ 键的四档候选（与 `resolve_sampler_path`
//!   同一条顺序，见 [`resolve_key_disk`]）；盘上找不到 ⇒ 旧映射表复用；皆无 ⇒
//!   响亮失败。
//!
//! ## 剪枝
//!
//! 新表是一次性重建的（不是在旧表上增量改）：旧表里**新表不再引用**的哈希、且
//! 不在工程 `assets`（音频片段）里的，进 `pruned` —— 调用方据此从会话池里删，
//! 但**绝不**删 `project.assets` 引用的（那是音频片段的资产，不是采样）。
//! 池里旧表/新表/`assets` 三者都不认识的孤儿资产一律不动（⛔ 不猜）。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use yeban_model::container::ContainerLimits;
use yeban_model::{AssetHash, YebanProjectV1};

use crate::tools::ErrorCode;

use super::error::Fault;
use super::sample_roots::{library_root, repo_root};
use super::sfz::{DemandLoadFilter, resolve_sample_path, triggered_sample_keys};

/// 保存一次的内嵌产物：新的 `embedded_samples` 表 ＋ 要加进会话 CAS 池的资产
/// ＋ 旧的内嵌资产里本次不再被引用的哈希（调用方据此剪枝）。
#[derive(Debug, Default)]
pub struct EmbeddedCollection {
    /// 新的 `embedded_samples` 表（渲染根相对解析路径 → SHA-256 小写十六进制）。
    pub map: BTreeMap<String, String>,
    /// 本次新收进池的资产（哈希 → 字节；SFZ 文本 ＋ 触发样本；CAS 按内容去重）。
    pub added: BTreeMap<AssetHash, Vec<u8>>,
    /// 旧表里新表不再引用、且不在工程 `assets` 里的哈希（调用方从池里删）。
    pub pruned: Vec<AssetHash>,
}

/// 对一份工程做保存侧内嵌收集（⛔ 不写盘、不改工程 —— 只算出产物，调用方安装）。
///
/// 逐轨（`tracks` 的 BTreeMap 顺序）处理带 `sampler` 绑定的轨；无绑定、触发集为空
/// 的轨照常处理（SFZ 文本仍进表 —— 渲染侧凭它走内嵌腿，而不是回落磁盘）。
///
/// # Errors
///
/// SFZ 读不到 / 不是 UTF-8 / 解析失败 / 谱子触发的样本在盘上与旧表里都找不到 ⇒
/// [`Fault`]（`samplerUnreadable`，含绑定路径与缺失的键 —— 保存被拒绝，
/// ⛔ 不静默跳过该轨）。
pub fn collect(
    project: &YebanProjectV1,
    project_path: &Path,
    assets: &BTreeMap<AssetHash, Vec<u8>>,
) -> Result<EmbeddedCollection, Fault> {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    let mut added: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
    for (track_id, track) in &project.tracks {
        let Some(binding) = track.sampler.as_ref().map(|sampler| sampler.path.as_str()) else {
            continue;
        };
        // ① SFZ 文本：盘上优先，否则已内嵌复用，否则响亮失败。
        let (text, disk_path) = match super::render::resolve_sampler_path(binding, project_path) {
            Ok(disk) => {
                let bytes = std::fs::read(&disk).map_err(|error| {
                    sampler_unreadable(binding, format!("读采样器定义文件失败: {error}"))
                })?;
                let text = String::from_utf8(bytes).map_err(|error| {
                    sampler_unreadable(binding, format!("采样器定义文件不是合法 UTF-8: {error}"))
                })?;
                (text, Some(disk))
            }
            Err(_) => {
                let bytes = embedded_bytes(project, assets, binding).ok_or_else(|| {
                    sampler_unreadable(
                        binding,
                        "磁盘上找不到采样器定义文件，工程里也没有内嵌副本".to_owned(),
                    )
                })?;
                let text = String::from_utf8(bytes).map_err(|error| {
                    sampler_unreadable(
                        binding,
                        format!("内嵌采样器定义文件不是合法 UTF-8: {error}"),
                    )
                })?;
                (text, None)
            }
        };
        // ② 逐音触发集；含 `#include` ⇒ 整轨跳过（SFZ 条目也不写 —— 渲染侧
        // 凭"绑定在表里"走内嵌腿，而内嵌腿装不了含 include 的 SFZ；写了表反而会
        // 把本可回落磁盘的渲染改成响亮失败。旧条目由集差剪枝）。
        let filter = DemandLoadFilter::from_project_track(project, *track_id);
        let Some(triggered) = triggered_sample_keys(&text, disk_path.as_deref(), binding, &filter)?
        else {
            continue;
        };
        // ③ SFZ 文本本身进表（键 = 绑定原样）＋ 进池。
        let sfz_hash = AssetHash::of_bytes(text.as_bytes());
        map.insert(binding.to_owned(), sfz_hash.as_str().to_owned());
        added.insert(sfz_hash, text.as_bytes().to_vec());
        for (raw, key) in triggered {
            let bytes = resolve_sample_bytes(
                project_path,
                disk_path.as_deref(),
                &raw,
                &key,
                project,
                assets,
            )
            .ok_or_else(|| {
                sampler_unreadable(
                    binding,
                    format!("谱子触发的样本找不到: `{raw}`（键 `{key}`）"),
                )
            })?;
            let hash = AssetHash::of_bytes(&bytes);
            map.insert(key, hash.as_str().to_owned());
            added.insert(hash, bytes);
        }
    }
    // ④ 剪枝：旧表里新表不再引用、且不在工程 `assets` 里的哈希。
    let new_hashes: BTreeSet<&str> = map.values().map(String::as_str).collect();
    let pruned: Vec<AssetHash> = project
        .embedded_samples
        .values()
        .filter_map(|hash_text| AssetHash::parse(hash_text).ok())
        .filter(|hash| !new_hashes.contains(hash.as_str()) && !project.assets.contains_key(hash))
        .collect();
    Ok(EmbeddedCollection { map, added, pruned })
}

/// 保存前容器上限预检。
///
/// 写容器路径本身**不**执行单条目 / 总体积 / 条目数这三道上限（读侧才查，
/// 见 `yeban-model` 的 `ContainerLimits`）—— 因此保存侧必须在这里先算、
/// 响亮失败，⛔ 不静默截断。`max_ratio` 是读侧的解压炸弹判据，保存写 `stored`
/// 条目不压缩、比率恒为 1，不在这里查。
///
/// `json_len` / `history_len` 是调用方算出的 `project.json` 与 `history.dag`
/// 字节数（容器条目 ＝ 这两条 ＋ `assets/{sha256}` 池）；`assets` 是合并后
/// 的池（旧池 ＋ 新收 − 剪枝），不是本次 `added`。
///
/// # Errors
///
/// 任一上限被超 ⇒ [`Fault`]（`IoError`，消息点名超出的上限与实测数字）。
pub fn check_container_limits(
    json_len: usize,
    history_len: usize,
    assets: &BTreeMap<AssetHash, Vec<u8>>,
    limits: &ContainerLimits,
) -> Result<(), Fault> {
    for (hash, bytes) in assets {
        let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if len > limits.max_entry_bytes {
            return Err(Fault::domain(
                ErrorCode::IoError,
                format!(
                    "内嵌资产 `assets/{hash}` 有 {len} 字节，超过单条目上限 {} 字节（容器上限；保存被拒绝，⛔ 不截断）",
                    limits.max_entry_bytes
                ),
            ));
        }
    }
    let total = u64::try_from(json_len)
        .unwrap_or(u64::MAX)
        .saturating_add(u64::try_from(history_len).unwrap_or(u64::MAX))
        .saturating_add(
            assets
                .values()
                .map(|bytes| u64::try_from(bytes.len()).unwrap_or(u64::MAX))
                .sum::<u64>(),
        );
    if total > limits.max_total_bytes {
        return Err(Fault::domain(
            ErrorCode::IoError,
            format!(
                "容器总字节数 {total} 超过上限 {} 字节（project.json {json_len} 字节 ＋ history.dag {history_len} 字节 ＋ {count} 条资产；保存被拒绝，⛔ 不截断）",
                limits.max_total_bytes,
                count = assets.len(),
            ),
        ));
    }
    let entries = 2usize.saturating_add(assets.len());
    if entries > limits.max_entries {
        return Err(Fault::domain(
            ErrorCode::IoError,
            format!(
                "容器条目数 {entries} 超过上限 {}（project.json ＋ history.dag ＋ {} 条资产；保存被拒绝，⛔ 不截断）",
                limits.max_entries,
                assets.len(),
            ),
        ));
    }
    Ok(())
}

/// 工程旧映射表里 `binding` 指的资产字节（哈希非法 / 资产不在池里 ⇒ `None`）。
fn embedded_bytes(
    project: &YebanProjectV1,
    assets: &BTreeMap<AssetHash, Vec<u8>>,
    binding: &str,
) -> Option<Vec<u8>> {
    let hash_text = project.embedded_samples.get(binding)?;
    let hash = AssetHash::parse(hash_text).ok()?;
    assets.get(&hash).cloned()
}

/// 一份触发样本的字节：盘上优先（与渲染同一条解析），旧表复用次之，皆无 ⇒ `None`
/// （调用方响亮失败）。
///
/// * 磁盘 SFZ 腿：`resolve_sample_path`（SFZ 所在目录 ＋ `sample=` 原串）。
/// * 已内嵌 SFZ 腿：键的四档候选（工程目录 ⇒ 库根 ⇒ 仓库根 ⇒ cwd，见
///   [`resolve_key_disk`]，与 `resolve_sampler_path` 同一条顺序）。
#[allow(clippy::too_many_arguments)]
fn resolve_sample_bytes(
    project_path: &Path,
    disk_sfz_path: Option<&Path>,
    raw: &str,
    key: &str,
    project: &YebanProjectV1,
    assets: &BTreeMap<AssetHash, Vec<u8>>,
) -> Option<Vec<u8>> {
    if let Some(disk) = disk_sfz_path {
        let candidate = resolve_sample_path(disk, raw);
        if candidate.is_file()
            && let Ok(bytes) = std::fs::read(&candidate)
        {
            return Some(bytes);
        }
    } else if let Some(candidate) = resolve_key_disk(key, project_path)
        && let Ok(bytes) = std::fs::read(&candidate)
    {
        return Some(bytes);
    }
    let hash_text = project.embedded_samples.get(key)?;
    let hash = AssetHash::parse(hash_text).ok()?;
    assets.get(&hash).cloned()
}

/// 键（渲染根相对路径）的四档磁盘候选 —— 与 `resolve_sampler_path` 同一条顺序
/// （工程目录 ⇒ 库根 ⇒ 仓库根 ⇒ cwd），只是锚点是**键**而不是绑定路径。
///
/// 库内布局剥掉 `assets/samples/` 前缀（与 `resolve_sampler_path` 及 app 的
/// 下载脚本 `--dest` 同一条规则）。
fn resolve_key_disk(key: &str, project_path: &Path) -> Option<PathBuf> {
    let candidate = Path::new(key);
    let mut tried: Vec<PathBuf> = Vec::new();
    if candidate.is_absolute() {
        tried.push(candidate.to_path_buf());
    } else {
        if let Some(dir) = project_path.parent() {
            tried.push(dir.join(candidate));
        }
        let library_relative = candidate
            .strip_prefix("assets/samples")
            .unwrap_or(candidate);
        tried.push(library_root().join(library_relative));
        tried.push(repo_root().join(candidate));
        tried.push(candidate.to_path_buf());
    }
    tried.into_iter().find(|path| path.is_file())
}

/// 内嵌保存失败：`samplerUnreadable` ＋ 绑定路径 ＋ 人话原因（与渲染侧同一条 `reason`）。
fn sampler_unreadable(binding: &str, detail: String) -> Fault {
    Fault::domain_with_data(
        ErrorCode::IoError,
        format!("保存内嵌采样失败（绑定 `{binding}`）: {detail}"),
        serde_json::json!({
            "reason": "samplerUnreadable",
            "path": binding,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_model::{ClipContent, ClipPoolEntry, EntityId, MidiNote, TrackV3};

    fn uid(n: u128) -> EntityId {
        use std::str::FromStr as _;
        EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8{n:04}")).expect("合法 ULID")
    }

    /// 最小工程：一轨 ＋ 一条 MIDI 片段（给定音高/力度）＋ 一条 SFZ 绑定。
    fn project_with_notes(track_id: EntityId, notes: &[(u8, u8)], binding: &str) -> YebanProjectV1 {
        let mut project = YebanProjectV1::default();
        let clip_id = uid(9001);
        let mut midi_notes = BTreeMap::new();
        for (i, &(pitch, velocity)) in notes.iter().enumerate() {
            let id = uid(9100 + i as u128);
            let mut note = MidiNote::new(id, (i as u64) * 960, pitch, 960);
            note.velocity = velocity;
            midi_notes.insert(id, note);
        }
        project.clip_pool.insert(
            clip_id,
            ClipPoolEntry {
                id: clip_id,
                name: "t".to_owned(),
                content: ClipContent::Midi { notes: midi_notes },
            },
        );
        let mut track = TrackV3 {
            id: track_id,
            ..TrackV3::default()
        };
        track.sampler = Some(yeban_model::SamplerBinding {
            path: binding.to_owned(),
        });
        let placement_id = uid(9200);
        track.clips.insert(
            placement_id,
            yeban_model::ClipPlacement {
                id: placement_id,
                clip_id,
                start_tick: 0,
                duration_ticks: 960 * notes.len().max(1) as u64,
                loop_config: yeban_model::LoopConfig::default(),
                muted: false,
                offset_samples: None,
            },
        );
        project.tracks.insert(track_id, track);
        project
    }

    /// 写一份最小 WAV（8 帧单声道 16 位）＋ 一份 SFZ 到临时目录，返回 SFZ 路径。
    ///
    /// 布局模拟真实工程：工程在 `dir/proj.yeban`，绑定是
    /// `assets/samples/kit/001.sfz` ⇒ SFZ 与样本都在 `dir/assets/samples/kit/` 下
    ///（`resolve_sampler_path` 候选 ① 工程目录命中）。
    fn write_fixture(dir: &Path, sfz_text: &str, wavs: &[(&str, &[u8])]) -> PathBuf {
        let kit = dir.join("assets/samples/kit");
        std::fs::create_dir_all(&kit).expect("建临时目录");
        for (name, bytes) in wavs {
            std::fs::write(kit.join(name), bytes).expect("写 WAV 夹具");
        }
        let sfz_path = kit.join("001.sfz");
        std::fs::write(&sfz_path, sfz_text).expect("写 SFZ 夹具");
        sfz_path
    }

    /// 8 帧单声道 16 位 WAV（手写 44 字节头 ＋ 8 帧采样，44.1kHz，样本值 0..8；
    /// ⛔ 不引入 `hound` —— 本 crate 没有这条直接依赖）。
    fn tiny_wav() -> Vec<u8> {
        let mut bytes = Vec::with_capacity(60);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&52u32.to_le_bytes()); // 36 + 16 字节数据
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&1u16.to_le_bytes()); // 单声道
        bytes.extend_from_slice(&44_100u32.to_le_bytes());
        bytes.extend_from_slice(&88_200u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes()); // 块对齐
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        for sample in 0..8i16 {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    /// 收集只收触发到的层：72 号音两个力度层，只触发低层（力度 32）。
    #[test]
    fn collect_embeds_only_triggered_layers() {
        let dir = std::env::temp_dir().join(format!(
            "yeban-embed-collect-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        let wav_low = tiny_wav();
        let mut wav_high = tiny_wav();
        wav_high[44] = 7; // 改第一帧采样的低字节 ⇒ 内容不同 ⇒ 哈希不同（⛔ 不能靠同名混过去）。
        let sfz_text = "<region> sample=low.wav key=72 lovel=0 hivel=63\n\
                        <region> sample=high.wav key=72 lovel=64 hivel=127\n\
                        <region> sample=unhit.wav key=60\n";
        write_fixture(
            &dir,
            sfz_text,
            &[
                ("low.wav", &wav_low),
                ("high.wav", &wav_high),
                ("unhit.wav", &tiny_wav()),
            ],
        );
        let binding = "assets/samples/kit/001.sfz";
        let project = project_with_notes(uid(1012), &[(72, 32)], binding);
        // 工程路径放在临时目录下 ⇒ `resolve_sampler_path` 候选 ①（工程目录）命中。
        let project_path = dir.join("proj.yeban");
        let collected = collect(&project, &project_path, &BTreeMap::new()).expect("收集");
        assert_eq!(
            collected.map.len(),
            2,
            "表里只有 SFZ 本身 ＋ 低力度层：{map:?}",
            map = collected.map
        );
        assert_eq!(
            collected.map.get(binding).map(String::as_str),
            Some(AssetHash::of_bytes(sfz_text.as_bytes()).as_str()),
            "SFZ 键 = 绑定原样，值 = 文本哈希"
        );
        assert_eq!(
            collected
                .map
                .get("assets/samples/kit/low.wav")
                .map(String::as_str),
            Some(AssetHash::of_bytes(&wav_low).as_str()),
            "低力度层必须在表里"
        );
        assert!(
            !collected.map.contains_key("assets/samples/kit/high.wav"),
            "高力度层（力度 64..127）未被触发，⛔ 不许收"
        );
        assert!(
            !collected.map.contains_key("assets/samples/kit/unhit.wav"),
            "无音符的区域 ⛔ 不许收"
        );
        assert_eq!(collected.added.len(), 2, "池里新增 SFZ 文本 ＋ 低层样本");
        assert!(collected.pruned.is_empty(), "旧表为空 ⇒ 无剪枝");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 缺触发样本 ⇒ 保存被拒绝（响亮失败，不静默跳过该轨）。
    #[test]
    fn collect_fails_loudly_on_missing_triggered_sample() {
        let dir = std::env::temp_dir().join(format!(
            "yeban-embed-collect-missing-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        let sfz_text = "<region> sample=gone.wav key=60\n";
        write_fixture(&dir, sfz_text, &[]);
        let binding = "assets/samples/kit/001.sfz";
        let project = project_with_notes(uid(1013), &[(60, 100)], binding);
        let project_path = dir.join("proj.yeban");
        let error = collect(&project, &project_path, &BTreeMap::new()).expect_err("缺样本必须失败");
        let Fault::Domain { message, data, .. } = error else {
            panic!("必须是领域失败");
        };
        assert!(
            message.contains("gone.wav"),
            "失败消息必须点名缺失的样本: {message}"
        );
        assert_eq!(
            data.as_ref()
                .and_then(|value| value.get("reason"))
                .and_then(|value| value.as_str()),
            Some("samplerUnreadable"),
            "reason 必须复用 samplerUnreadable（不新增 fault 词汇）"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 含 `#include` 的 SFZ ⇒ 整轨跳过（旧条目由集差剪枝），不失败。
    #[test]
    fn collect_skips_sfz_with_includes_and_prunes_old_entries() {
        let dir = std::env::temp_dir().join(format!(
            "yeban-embed-collect-inc-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        let sfz_text = "<region> sample=a.wav key=60\n#include \"sub.sfz\"\n";
        let sfz_path = write_fixture(&dir, sfz_text, &[("a.wav", &tiny_wav())]);
        std::fs::write(
            sfz_path
                .parent()
                .expect("SFZ 在 kit 目录下")
                .join("sub.sfz"),
            "<region> sample=b.wav key=61\n",
        )
        .expect("写 include 目标");
        let binding = "assets/samples/kit/001.sfz";
        let mut project = project_with_notes(uid(1014), &[(60, 100)], binding);
        // 旧表里有一条上一轮收的样本（本轮 SFZ 含 include ⇒ 跳过 ⇒ 该条被剪枝）。
        let old_hash = AssetHash::of_bytes(b"old-bytes");
        project.embedded_samples.insert(
            "assets/samples/kit/old.wav".to_owned(),
            old_hash.as_str().to_owned(),
        );
        let project_path = dir.join("proj.yeban");
        let collected = collect(&project, &project_path, &BTreeMap::new()).expect("收集");
        assert!(
            collected.map.is_empty(),
            "含 include 的轨整轨跳过，表里什么都不留: {map:?}",
            map = collected.map
        );
        assert_eq!(collected.pruned, vec![old_hash], "旧条目必须被剪枝");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 剪枝不碰工程 `assets`（音频片段）引用的哈希。
    #[test]
    fn collect_pruning_keeps_audio_assets() {
        let dir = std::env::temp_dir().join(format!(
            "yeban-embed-collect-prune-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        let sfz_text = "<region> sample=a.wav key=60\n";
        write_fixture(&dir, sfz_text, &[("a.wav", &tiny_wav())]);
        let binding = "assets/samples/kit/001.sfz";
        let mut project = project_with_notes(uid(1015), &[(60, 100)], binding);
        // 音频资产：既在旧表里，也在工程 `assets` 里 ⇒ 即使新表不再引用，也不剪。
        let audio_hash = AssetHash::of_bytes(b"audio-bytes");
        project.embedded_samples.insert(
            "assets/samples/kit/audio.wav".to_owned(),
            audio_hash.as_str().to_owned(),
        );
        project.assets.insert(
            audio_hash.clone(),
            yeban_model::AssetMetadata {
                hash: audio_hash.clone(),
                original_path: "audio.wav".to_owned(),
                byte_len: 11,
                media_kind: yeban_model::MediaKind::Audio,
                license: String::new(),
            },
        );
        let project_path = dir.join("proj.yeban");
        let collected = collect(&project, &project_path, &BTreeMap::new()).expect("收集");
        assert!(
            !collected.pruned.contains(&audio_hash),
            "工程 assets 引用的哈希 ⛔ 不许剪枝"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 上限预检：小阈值注入 ⇒ 单条目 / 总体积 / 条目数三道都响亮失败。
    #[test]
    fn check_container_limits_rejects_each_limit_with_tiny_thresholds() {
        let mut assets: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
        let hash = AssetHash::of_bytes(b"0123456789");
        assets.insert(hash, b"0123456789".to_vec());
        // 单条目超限（10 字节 > 5 字节）。
        let limits = ContainerLimits {
            max_entry_bytes: 5,
            ..ContainerLimits::default()
        };
        assert!(
            check_container_limits(10, 10, &assets, &limits).is_err(),
            "单条目超限必须失败"
        );
        // 总体积超限（10 + 10 + 10 = 30 > 25）。
        let limits = ContainerLimits {
            max_total_bytes: 25,
            ..ContainerLimits::default()
        };
        assert!(
            check_container_limits(10, 10, &assets, &limits).is_err(),
            "总体积超限必须失败"
        );
        // 条目数超限（2 + 1 = 3 > 2）。
        let limits = ContainerLimits {
            max_entries: 2,
            ..ContainerLimits::default()
        };
        assert!(
            check_container_limits(10, 10, &assets, &limits).is_err(),
            "条目数超限必须失败"
        );
        // 默认上限下通过。
        assert!(
            check_container_limits(10, 10, &assets, &ContainerLimits::default()).is_ok(),
            "默认上限下必须通过"
        );
    }
}
