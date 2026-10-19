//! 旧 `.groove`（Groove Web 的 **JSON** 工程包）⇒ 新 `.yeban` 容器的**最小迁移器**
//! （任务单 `#146`，`HD-64` 的裁决 A：读旧 `.groove` ⇒ 产出新 `.yeban` 容器，
//! ⛔ 不做历史清理）。
//!
//! ## 输入是什么（实测，不是猜测）
//!
//! 旧 `.groove` **只是 JSON**，没有任何二进制容器包装。仓库外唯一的真旧工程
//! `/Users/crow/Downloads/假如时光能够倒流.groove`（19,111 字节）的顶层是
//! `{"format":"groove-project","version":1,"exportedAt":…,"appVersion":"2.34.0","project":{…}}`。
//! 另外 5 个同名扩展名的文件 `format = "groove-package"` —— 那是一种**旧实现里没有读取器**
//! 的形状（`groove/src/**` 对 `groove-package` 命中 0），因此本迁移器**响亮拒绝**它，
//! ⛔ 不猜格式、⛔ 不产半成品。
//!
//! ## 映射（有损的地方逐条登记，⛔ 不假装无损）
//!
//! | 旧 (`groove-project`) | 新 (`YebanProjectV1`) |
//! | :-- | :-- |
//! | `project.name` / `bpm` / `timeSignature` | 顶层 `title` / `bpm` / `time_signature`（唯一权威） |
//! | `project.patterns.{A,B}.tracks[*]` | 每个 `track_id` **一条** `TrackV3`（A/B 合并） |
//! | `steps[i] != 0` | 一个 `MidiNote`（`start_tick = i × 每步 tick`，时值 = 每步 tick） |
//! | `pitch[i]`（可空数组） | 音符音高；缺省时按鼓件表映射（`closed_hat` → 42 …） |
//! | `velocity[i]`（可空数组） | 音符力度；缺省 [`DEFAULT_VELOCITY`] |
//! | 每个 `(track_id, A/B)` | 一个 `clip_pool` 里的 MIDI 片段 |
//! | `sections[*].slot/bars/label/mute` | `SectionV3` ＋ 每轨一个 `ClipPlacement`（mute 落到摆放） |
//! | 旧工程没有主总线概念 | 迁移器**补**一条 `TrackKind::Master` ＋ 每轨一条 `TrackToBus` 路由 |
//!
//! **有损项**（逐条打到 stdout 的 `migrated-groove-loss:` 行，⛔ 不静默丢弃）：
//! `effectsRack` / `masterSettings` / `swing` / `genreId` / `genreName` / `drumKit` /
//! `loopRange` / `activeSlot` / `songMode` / `isFavorite` / `color` / `snapshotSummary` /
//! `extraClips` / `arrangement` / 段落 `overrides` 与 `velocityScale`。
//!
//! ## 旧文件**只读**
//!
//! [`run`] 先 `fs::read` 旧文件，迁移与保存全部完成后**再读一次**并逐字节比较；
//! 不一致 ⇒ [`MigrateError::InputMutated`]。输入与输出是同一个路径 ⇒ 解析阶段就拒绝
//! （[`crate::cli::ParseError::MigrateGrooveSamePath`]），所以本命令**不可能**原地改写旧文件。
//!
//! ## 为什么不用 `serde` 派生
//!
//! 本 crate 的直接依赖里没有 `serde`（只有 `serde_json`），而旧格式的字段有一半我们
//! 不消费。用 [`serde_json::Value`] 手读的好处是**每条错误都能点名完整路径**
//! （`project.patterns.A.tracks[3].steps`），这正好对上 `HD-64` 那句"坏文件要响亮"。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use serde_json::{Map, Value};
use yeban_model::container::ProjectArchive;
use yeban_model::ids::{EntityId, PPQ};
use yeban_model::music::{DEFAULT_VELOCITY, MidiNote};
use yeban_model::project::{
    ClipContent, ClipPlacement, ClipPoolEntry, LaunchQuantization, LoopConfig, ProjectMetadata,
    RoutingEdge, RoutingKind, SectionV3, TimeSignature, TrackKind, TrackV3, TransportConfig,
    YebanProjectV1,
};

use crate::save::{SaveError, SaveReport, save_archive_file};

/// 迁移开关的字面值（`--migrate-groove <old.groove>`，必须与 `--save-as <new.yeban>` 成对）。
pub const MIGRATE_GROOVE_SWITCH: &str = "--migrate-groove";

/// 迁移失败的全部原因（每一种都**响亮**上报，⛔ 绝不"写了个空容器也算成功"）。
#[derive(Debug)]
pub enum MigrateError {
    /// 读取旧文件失败。
    Read {
        /// 旧文件路径。
        path: PathBuf,
        /// 底层 I/O 错误。
        source: std::io::Error,
    },
    /// 旧文件不是合法 JSON。
    InvalidJson {
        /// 旧文件路径。
        path: PathBuf,
        /// `serde_json` 的原样描述（含行列号）。
        detail: String,
    },
    /// 顶层 `format` 不是本迁移器认识的 `groove-project`。
    UnsupportedFormat {
        /// 旧文件路径。
        path: PathBuf,
        /// 实际读到的 `format`。
        format: String,
    },
    /// 顶层 `version` 不在支持范围（本迁移器只认 1 / 2）。
    UnsupportedVersion {
        /// 旧文件路径。
        path: PathBuf,
        /// 实际读到的版本号。
        version: u64,
    },
    /// 旧文件缺少一个**必需**字段。
    MissingField {
        /// 旧文件路径。
        path: PathBuf,
        /// 完整字段路径（例如 `project.patterns.A.tracks`）。
        field: String,
    },
    /// 旧文件的某个字段类型不对。
    FieldType {
        /// 旧文件路径。
        path: PathBuf,
        /// 完整字段路径。
        field: String,
        /// 期望的 JSON 类型。
        expected: &'static str,
        /// 实际读到的 JSON 类型。
        found: &'static str,
    },
    /// 旧文件的某个字段取值非法（越界 / 长度不一致 / 无法解析）。
    InvalidValue {
        /// 旧文件路径。
        path: PathBuf,
        /// 完整字段路径。
        field: String,
        /// 人话原因。
        detail: String,
    },
    /// 迁移出的工程未通过 `YebanProjectV1::validate()`。
    InvalidProject {
        /// 旧文件路径。
        path: PathBuf,
        /// 模型层的原样裁决。
        detail: String,
    },
    /// 迁移自检失败（旧读数与新读数不相等 —— 这属于实现缺陷，不是输入问题）。
    CountMismatch {
        /// 旧文件路径。
        path: PathBuf,
        /// 人话原因。
        detail: String,
    },
    /// 保存成功之后重新读旧文件，字节变了 ⇒ 只读契约被破坏。
    InputMutated {
        /// 旧文件路径。
        path: PathBuf,
    },
    /// 输入与输出是同一个路径（旧文件只读，绝不原地改写）。
    OutputEqualsInput {
        /// 那个路径。
        path: PathBuf,
    },
    /// 把迁移结果写成 `.yeban` 容器失败。
    Save {
        /// 目标路径。
        path: PathBuf,
        /// 落盘 / 容器层的原样裁决。
        source: Box<SaveError>,
    },
}

impl MigrateError {
    /// 这一档失败的退出码：读/解析/校验 ⇒ [`crate::cli::EXIT_OPEN`]；
    /// 写容器 ⇒ [`crate::cli::EXIT_SAVE`]。
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Save { .. } => crate::cli::EXIT_SAVE,
            Self::Read { .. }
            | Self::InvalidJson { .. }
            | Self::UnsupportedFormat { .. }
            | Self::UnsupportedVersion { .. }
            | Self::MissingField { .. }
            | Self::FieldType { .. }
            | Self::InvalidValue { .. }
            | Self::InvalidProject { .. }
            | Self::CountMismatch { .. }
            | Self::InputMutated { .. }
            | Self::OutputEqualsInput { .. } => crate::cli::EXIT_OPEN,
        }
    }
}

impl fmt::Display for MigrateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(formatter, "读取旧工程 `{}` 失败: {source}", path.display())
            }
            Self::InvalidJson { path, detail } => write!(
                formatter,
                "旧工程 `{}` 不是合法 JSON: {detail}",
                path.display()
            ),
            Self::UnsupportedFormat { path, format } => write!(
                formatter,
                "旧工程 `{}` 的 format = `{format}`, 本迁移器只认 `groove-project` \
                 (Groove Web 的 .groove v1/v2); `groove-package` 在旧实现里没有读取器, 无法迁移",
                path.display()
            ),
            Self::UnsupportedVersion { path, version } => write!(
                formatter,
                "旧工程 `{}` 的 version = {version}, 本迁移器只接受 1 或 2 \
                 (v1 = 两套 pattern; v2 = v1 + arrangement)",
                path.display()
            ),
            Self::MissingField { path, field } => {
                write!(formatter, "旧工程 `{}` 缺少字段 `{field}`", path.display())
            }
            Self::FieldType {
                path,
                field,
                expected,
                found,
            } => write!(
                formatter,
                "旧工程 `{}` 的字段 `{field}` 需要 {expected}, 实际是 {found}",
                path.display()
            ),
            Self::InvalidValue {
                path,
                field,
                detail,
            } => write!(
                formatter,
                "旧工程 `{}` 的字段 `{field}` 非法: {detail}",
                path.display()
            ),
            Self::InvalidProject { path, detail } => write!(
                formatter,
                "迁移出的工程未通过模型校验 (输入 `{}`): {detail}",
                path.display()
            ),
            Self::CountMismatch { path, detail } => write!(
                formatter,
                "迁移自检失败 (输入 `{}`): {detail}",
                path.display()
            ),
            Self::InputMutated { path } => write!(
                formatter,
                "迁移前后旧文件 `{}` 的字节不同 —— 只读契约被破坏",
                path.display()
            ),
            Self::OutputEqualsInput { path } => write!(
                formatter,
                "`--migrate-groove` 的输入与输出是同一个路径 `{}` ⇒ 拒绝 (旧文件只读, 绝不原地改写)",
                path.display()
            ),
            Self::Save { path, source } => write!(
                formatter,
                "把迁移结果保存到 `{}` 失败: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for MigrateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Save { source, .. } => Some(source.as_ref()),
            Self::InvalidJson { .. }
            | Self::UnsupportedFormat { .. }
            | Self::UnsupportedVersion { .. }
            | Self::MissingField { .. }
            | Self::FieldType { .. }
            | Self::InvalidValue { .. }
            | Self::InvalidProject { .. }
            | Self::CountMismatch { .. }
            | Self::InputMutated { .. }
            | Self::OutputEqualsInput { .. } => None,
        }
    }
}

/// 执行一次迁移：读旧 `.groove` ⇒ 建 `YebanProjectV1` ⇒ 原子写成 `.yeban` 容器 ⇒
/// 返回**应当打印的每一行**（打印由 `main.rs` 的 [`crate::cli::emit`] 做）。
///
/// # Errors
///
/// 见 [`MigrateError`]：读 / JSON / 字段 / 模型校验 / 保存任一档失败都响亮上报，
/// 且**旧文件一个字节都不写**。
pub fn run(input: &Path, output: &Path) -> Result<Vec<String>, MigrateError> {
    let before = std::fs::read(input).map_err(|source| MigrateError::Read {
        path: input.to_path_buf(),
        source,
    })?;

    let input_canonical = std::fs::canonicalize(input).map_err(|source| MigrateError::Read {
        path: input.to_path_buf(),
        source,
    })?;
    if std::fs::canonicalize(output)
        .is_ok_and(|output_canonical| output_canonical == input_canonical)
    {
        return Err(MigrateError::OutputEqualsInput {
            path: input.to_path_buf(),
        });
    }

    let (archive, report) = migrate_bytes(&before, input)?;
    let saved = save_archive_file(&archive, output).map_err(|source| MigrateError::Save {
        path: output.to_path_buf(),
        source: Box::new(source),
    })?;

    let after = std::fs::read(input).map_err(|source| MigrateError::Read {
        path: input.to_path_buf(),
        source,
    })?;
    if after != before {
        return Err(MigrateError::InputMutated {
            path: input.to_path_buf(),
        });
    }

    Ok(report.lines(input, &saved))
}

/// 旧工程一条音轨的**稳定元数据**（同名 `track_id` 在 A/B 里取第一次出现的那份）。
struct TrackMeta {
    name: String,
    instrument: String,
    volume: f32,
    pan: f32,
}

/// 旧工程的一条音符（已换算到 960 PPQ）。
struct LegacyNote {
    start_tick: u64,
    pitch: u8,
    velocity: u8,
}

/// 旧工程的一个 `(pattern, track)` 读数。
struct ParsedTrack {
    track_id: String,
    name: String,
    instrument: String,
    volume: f32,
    pan: f32,
    natural_ticks: u64,
    notes: Vec<LegacyNote>,
}

/// 旧工程的一段（来自 `sections` 或 `songChain`）。
struct SectionSpec {
    slot: String,
    bars: u64,
    label: String,
    mute: BTreeSet<String>,
    synthetic: bool,
}

/// 一次成功迁移的读数（打印行的唯一来源）。
#[derive(Debug)]
struct MigrationReport {
    input_bytes: usize,
    format: String,
    version: u64,
    app_version: String,
    exported_at: Option<u64>,
    legacy_tracks: usize,
    legacy_notes: usize,
    legacy_sections: usize,
    sections_source: &'static str,
    legacy_clips: usize,
    migrated_tracks_musical: usize,
    migrated_tracks_all: usize,
    migrated_notes: usize,
    migrated_sections: usize,
    migrated_clips: usize,
    losses: Vec<String>,
}

impl MigrationReport {
    fn lines(&self, input: &Path, saved: &SaveReport) -> Vec<String> {
        let mut lines = Vec::with_capacity(5 + self.losses.len());
        lines.push(crate::cli::HEADLESS_HANDSHAKE.to_owned());
        lines.push(format!(
            "migrated-groove: input={} input-bytes={} format={} version={} app-version={} exported-at={}",
            quoted_path(input),
            self.input_bytes,
            self.format,
            self.version,
            crate::cli::quoted(&self.app_version),
            self.exported_at
                .map_or_else(|| "none".to_owned(), |value| value.to_string()),
        ));
        lines.push(format!(
            "migrated-groove-counts: legacy-tracks={} legacy-notes={} legacy-sections={} \
             sections-source={} legacy-clips={} migrated-tracks-musical={} migrated-tracks-all={} \
             migrated-notes={} migrated-sections={} migrated-clips={}",
            self.legacy_tracks,
            self.legacy_notes,
            self.legacy_sections,
            self.sections_source,
            self.legacy_clips,
            self.migrated_tracks_musical,
            self.migrated_tracks_all,
            self.migrated_notes,
            self.migrated_sections,
            self.migrated_clips,
        ));
        lines.push(format!(
            "migrated-groove-losses: count={}",
            self.losses.len()
        ));
        for loss in &self.losses {
            lines.push(format!("migrated-groove-loss: {loss}"));
        }
        lines.push(format!(
            "migrated: path={} bytes={} temp={}",
            quoted_path(&saved.path),
            saved.bytes,
            crate::cli::quoted(&saved.temp_name),
        ));
        lines
    }
}

/// 纯函数部分：旧 JSON 字节 ⇒ `(归档, 读数)`。
fn migrate_bytes(
    bytes: &[u8],
    path: &Path,
) -> Result<(ProjectArchive, MigrationReport), MigrateError> {
    let root: Value = serde_json::from_slice(bytes).map_err(|error| MigrateError::InvalidJson {
        path: path.to_path_buf(),
        detail: error.to_string(),
    })?;
    let reader = Reader { path };
    let root = reader.object(&root, "$")?;

    let format = reader.string_at(root, "$", "format")?;
    if format != "groove-project" {
        return Err(MigrateError::UnsupportedFormat {
            path: path.to_path_buf(),
            format: format.to_owned(),
        });
    }
    let version = reader.u64_at(root, "$", "version")?;
    if version != 1 && version != 2 {
        return Err(MigrateError::UnsupportedVersion {
            path: path.to_path_buf(),
            version,
        });
    }
    let app_version = reader.opt_string(root, "appVersion").unwrap_or_default();
    let exported_at = reader.opt_u64(root, "exportedAt")?;

    let project_field = "project";
    let legacy_project = reader.object(reader.get(root, "$", project_field)?, project_field)?;

    let title = reader
        .string_at(legacy_project, project_field, "name")?
        .to_owned();
    let bpm = reader.f64_at(legacy_project, project_field, "bpm")?;
    if !(20.0..=999.0).contains(&bpm) {
        return Err(MigrateError::InvalidValue {
            path: path.to_path_buf(),
            field: format!("{project_field}.bpm"),
            detail: format!("{bpm} 不在 20.0..=999.0"),
        });
    }
    let time_signature = parse_time_signature(
        reader.string_at(legacy_project, project_field, "timeSignature")?,
        path,
    )?;
    let resolution = reader.string_at(legacy_project, project_field, "resolution")?;
    let step_ticks = resolution_ticks(resolution).ok_or_else(|| MigrateError::InvalidValue {
        path: path.to_path_buf(),
        field: format!("{project_field}.resolution"),
        detail: format!("`{resolution}` 不是 1/8 | 1/16 | 1/32"),
    })?;

    let patterns_field = format!("{project_field}.patterns");
    let patterns = reader.object(
        reader.get(legacy_project, project_field, "patterns")?,
        &patterns_field,
    )?;
    let pattern_a = reader.get(patterns, &patterns_field, "A")?;
    let pattern_b = reader.get(patterns, &patterns_field, "B")?;

    let mut losses: BTreeSet<String> = BTreeSet::new();
    for ignored in [
        "effectsRack",
        "masterSettings",
        "genreId",
        "genreName",
        "drumKit",
        "loopRange",
        "activeSlot",
        "songMode",
        "isFavorite",
        "color",
        "snapshotSummary",
        "extraClips",
    ] {
        if legacy_project
            .get(ignored)
            .is_some_and(|value| !value.is_null())
        {
            losses.insert(format!("`project.{ignored}` 在新模型里没有对应物, 未迁移"));
        }
    }
    if legacy_project
        .get("swing")
        .is_some_and(|value| !value.is_null())
    {
        losses.insert("`project.swing` 在新模型里没有对应物, 未迁移".to_owned());
    }
    if root
        .get("arrangement")
        .is_some_and(|value| !value.is_null())
    {
        losses.insert("`arrangement` 未迁移 (本迁移器只搬 project 的 pattern/section)".to_owned());
    }

    let parsed_patterns = [
        (
            "A",
            reader.parse_pattern("A", pattern_a, step_ticks, &mut losses)?,
        ),
        (
            "B",
            reader.parse_pattern("B", pattern_b, step_ticks, &mut losses)?,
        ),
    ];

    let mut track_order: Vec<String> = Vec::new();
    let mut track_meta: BTreeMap<String, TrackMeta> = BTreeMap::new();
    for (_, parsed_tracks) in &parsed_patterns {
        for parsed in parsed_tracks {
            if let Some(existing) = track_meta.get(&parsed.track_id) {
                if existing.name != parsed.name || existing.instrument != parsed.instrument {
                    losses.insert(format!(
                        "track `{}`: pattern A/B 的 name/instrument 不一致, 取第一次出现的那份",
                        parsed.track_id
                    ));
                }
                continue;
            }
            track_order.push(parsed.track_id.clone());
            track_meta.insert(
                parsed.track_id.clone(),
                TrackMeta {
                    name: parsed.name.clone(),
                    instrument: parsed.instrument.clone(),
                    volume: parsed.volume,
                    pan: parsed.pan,
                },
            );
        }
    }

    let step_count = reader.opt_u64(legacy_project, "stepCount")?;
    if let Some(declared) = step_count {
        for (slot, parsed_tracks) in &parsed_patterns {
            for parsed in parsed_tracks {
                let actual = parsed.natural_ticks / step_ticks;
                if actual != declared {
                    losses.insert(format!(
                        "`project.stepCount` = {declared}, 但 pattern {slot} 的 track `{}` 有 {actual} 步; 以实际数组长度为准",
                        parsed.track_id
                    ));
                }
            }
        }
    }

    let legacy_tracks = track_order.len();
    let legacy_notes: usize = parsed_patterns
        .iter()
        .flat_map(|(_, tracks)| tracks.iter())
        .map(|track| track.notes.len())
        .sum();
    let legacy_clips: usize = parsed_patterns
        .iter()
        .flat_map(|(_, tracks)| tracks.iter())
        .filter(|track| !track.notes.is_empty())
        .count();

    let (sections, legacy_sections, sections_source) =
        reader.parse_sections(legacy_project, &mut losses)?;

    let description = reader
        .opt_string(legacy_project, "notes")
        .unwrap_or_default();
    let created_at_unix_ms = reader
        .opt_u64(legacy_project, "createdAt")?
        .or(exported_at)
        .unwrap_or(0);
    let modified_at_unix_ms = reader
        .opt_u64(legacy_project, "updatedAt")?
        .or(exported_at)
        .unwrap_or(0);
    let tags = reader
        .opt_string_array(legacy_project, project_field, "tags")?
        .unwrap_or_default();
    let metronome_enabled = reader
        .opt_bool(legacy_project, "isMetronome")?
        .unwrap_or(false);
    let count_in_bars = u8::from(
        reader
            .opt_bool(legacy_project, "isCountIn")?
            .unwrap_or(false),
    );

    let mut ids = IdGen { counter: 1 };
    let project_id = ids.next();
    let mut project = YebanProjectV1 {
        schema_version: yeban_model::project::SCHEMA_VERSION,
        min_reader_version: yeban_model::project::MIN_READER_VERSION,
        id: project_id,
        title,
        bpm,
        time_signature,
        metadata: ProjectMetadata {
            description,
            created_at_unix_ms,
            modified_at_unix_ms,
            tags,
        },
        transport: TransportConfig {
            metronome_enabled,
            count_in_bars,
            launch_quantization: LaunchQuantization::Bar,
        },
        ..YebanProjectV1::default()
    };

    let master_id = ids.next();
    let master = TrackV3 {
        id: master_id,
        name: "Master".to_owned(),
        kind: TrackKind::Master,
        ..TrackV3::default()
    };
    project.tracks.insert(master_id, master);
    project.master_bus_track_id = master_id;
    project.routing_graph.nodes.push(master_id);

    let mut track_entities: BTreeMap<String, EntityId> = BTreeMap::new();
    for track_id in &track_order {
        let meta = track_meta
            .get(track_id)
            .expect("track_order 与 track_meta 同源");
        let entity_id = ids.next();
        let track = TrackV3 {
            id: entity_id,
            name: meta.name.clone(),
            volume_db: volume_to_db(meta.volume),
            pan: meta.pan.clamp(-1.0, 1.0),
            ..TrackV3::default()
        };
        project.tracks.insert(entity_id, track);
        project.routing_graph.nodes.push(entity_id);
        let edge_id = ids.next();
        project.routing_graph.edges.insert(
            edge_id,
            RoutingEdge {
                id: edge_id,
                source_node: entity_id,
                destination_node: master_id,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
        track_entities.insert(track_id.clone(), entity_id);
    }

    let mut clip_ids: BTreeMap<(String, String), EntityId> = BTreeMap::new();
    let mut natural_ticks: BTreeMap<(String, String), u64> = BTreeMap::new();
    for (slot, parsed_tracks) in &parsed_patterns {
        for parsed in parsed_tracks {
            if parsed.notes.is_empty() {
                continue;
            }
            let meta = track_meta
                .get(&parsed.track_id)
                .expect("parsed track 必在 track_meta 里");
            let clip_id = ids.next();
            let mut notes: BTreeMap<EntityId, MidiNote> = BTreeMap::new();
            for legacy in &parsed.notes {
                let note_id = ids.next();
                let mut note = MidiNote::new(note_id, legacy.start_tick, legacy.pitch, step_ticks);
                note.velocity = legacy.velocity;
                notes.insert(note_id, note);
            }
            project.clip_pool.insert(
                clip_id,
                ClipPoolEntry {
                    id: clip_id,
                    name: format!("{} · {slot}", meta.name),
                    content: ClipContent::Midi { notes },
                },
            );
            clip_ids.insert((parsed.track_id.clone(), (*slot).to_owned()), clip_id);
            natural_ticks.insert(
                (parsed.track_id.clone(), (*slot).to_owned()),
                parsed.natural_ticks,
            );
        }
    }

    let bar_ticks =
        u64::from(time_signature.numerator) * PPQ * 4 / u64::from(time_signature.denominator);
    let mut cursor = 0_u64;
    for spec in &sections {
        let duration_ticks = spec.bars * bar_ticks;
        let start_tick = cursor;
        let end_tick = start_tick + duration_ticks;
        if !spec.synthetic {
            let section_id = ids.next();
            project.sections.insert(
                section_id,
                SectionV3 {
                    id: section_id,
                    name: spec.label.clone(),
                    start_tick,
                    end_tick,
                    color: None,
                },
            );
        }
        let mut placed_any = false;
        for track_id in &track_order {
            let key = (track_id.clone(), spec.slot.clone());
            let Some(clip_id) = clip_ids.get(&key).copied() else {
                continue;
            };
            placed_any = true;
            let natural = natural_ticks.get(&key).copied().unwrap_or(duration_ticks);
            let loop_config = if duration_ticks > natural && natural > 0 {
                LoopConfig {
                    enabled: true,
                    start_tick: 0,
                    end_tick: natural,
                }
            } else {
                LoopConfig::default()
            };
            let placement_id = ids.next();
            let placement = ClipPlacement {
                id: placement_id,
                clip_id,
                start_tick,
                duration_ticks,
                loop_config,
                muted: spec.mute.contains(track_id),
            };
            if let Some(track) = project.tracks.get_mut(&track_entities[track_id]) {
                track.clips.insert(placement_id, placement);
            }
        }
        if !placed_any {
            losses.insert(format!(
                "section slot `{}` 没有对应 pattern (extraClips 未迁移)",
                spec.slot
            ));
        }
        cursor = end_tick;
    }

    project
        .validate()
        .map_err(|error| MigrateError::InvalidProject {
            path: path.to_path_buf(),
            detail: error.to_string(),
        })?;
    project
        .check_readable()
        .map_err(|error| MigrateError::InvalidProject {
            path: path.to_path_buf(),
            detail: error.to_string(),
        })?;

    let migrated_notes: usize = project
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .map(BTreeMap::len)
        .sum();
    let migrated_tracks_musical = track_order.len();
    let migrated_tracks_all = project.tracks.len();
    let migrated_sections = project.sections.len();
    let migrated_clips = project.clip_pool.len();
    if legacy_notes != migrated_notes || migrated_tracks_musical != legacy_tracks {
        return Err(MigrateError::CountMismatch {
            path: path.to_path_buf(),
            detail: format!(
                "旧读数 tracks={legacy_tracks} notes={legacy_notes} \
                 ⇒ 新读数 tracks-musical={migrated_tracks_musical} notes={migrated_notes}"
            ),
        });
    }

    let report = MigrationReport {
        input_bytes: bytes.len(),
        format: format.to_owned(),
        version,
        app_version,
        exported_at,
        legacy_tracks,
        legacy_notes,
        legacy_sections,
        sections_source,
        legacy_clips,
        migrated_tracks_musical,
        migrated_tracks_all,
        migrated_notes,
        migrated_sections,
        migrated_clips,
        losses: losses.into_iter().collect(),
    };
    Ok((
        ProjectArchive {
            project,
            history_dag: Vec::new(),
            assets: Vec::new(),
        },
        report,
    ))
}

/// 旧工程的解引用器：把"路径"带进每一条错误，⛔ 不让坏文件变成一句笼统的 "parse error"。
struct Reader<'a> {
    path: &'a Path,
}

impl Reader<'_> {
    fn get<'v>(
        &self,
        map: &'v Map<String, Value>,
        parent: &str,
        key: &str,
    ) -> Result<&'v Value, MigrateError> {
        map.get(key).ok_or_else(|| MigrateError::MissingField {
            path: self.path.to_path_buf(),
            field: format!("{parent}.{key}"),
        })
    }

    fn object<'v>(
        &self,
        value: &'v Value,
        field: &str,
    ) -> Result<&'v Map<String, Value>, MigrateError> {
        value
            .as_object()
            .ok_or_else(|| self.field_type(field, "object", value))
    }

    fn array<'v>(&self, value: &'v Value, field: &str) -> Result<&'v Vec<Value>, MigrateError> {
        value
            .as_array()
            .ok_or_else(|| self.field_type(field, "array", value))
    }

    fn string<'v>(&self, value: &'v Value, field: &str) -> Result<&'v str, MigrateError> {
        value
            .as_str()
            .ok_or_else(|| self.field_type(field, "string", value))
    }

    fn u64(&self, value: &Value, field: &str) -> Result<u64, MigrateError> {
        value.as_u64().ok_or_else(|| MigrateError::InvalidValue {
            path: self.path.to_path_buf(),
            field: field.to_owned(),
            detail: format!("需要一个非负整数, 实际是 {}", json_kind(value)),
        })
    }

    fn f64(&self, value: &Value, field: &str) -> Result<f64, MigrateError> {
        value
            .as_f64()
            .ok_or_else(|| self.field_type(field, "number", value))
    }

    fn string_at<'v>(
        &self,
        map: &'v Map<String, Value>,
        parent: &str,
        key: &str,
    ) -> Result<&'v str, MigrateError> {
        let field = format!("{parent}.{key}");
        self.string(self.get(map, parent, key)?, &field)
    }

    fn u64_at(
        &self,
        map: &Map<String, Value>,
        parent: &str,
        key: &str,
    ) -> Result<u64, MigrateError> {
        let field = format!("{parent}.{key}");
        self.u64(self.get(map, parent, key)?, &field)
    }

    fn f64_at(
        &self,
        map: &Map<String, Value>,
        parent: &str,
        key: &str,
    ) -> Result<f64, MigrateError> {
        let field = format!("{parent}.{key}");
        self.f64(self.get(map, parent, key)?, &field)
    }

    fn opt_u64(&self, map: &Map<String, Value>, key: &str) -> Result<Option<u64>, MigrateError> {
        match map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => Ok(Some(self.u64(value, key)?)),
        }
    }

    fn opt_f64(&self, map: &Map<String, Value>, key: &str) -> Result<Option<f64>, MigrateError> {
        match map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => Ok(Some(self.f64(value, key)?)),
        }
    }

    fn opt_bool(&self, map: &Map<String, Value>, key: &str) -> Result<Option<bool>, MigrateError> {
        match map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value
                .as_bool()
                .map(Some)
                .ok_or_else(|| self.field_type(key, "boolean", value)),
        }
    }

    fn opt_string(&self, map: &Map<String, Value>, key: &str) -> Option<String> {
        match map.get(key) {
            Some(Value::String(value)) => Some(value.clone()),
            _ => None,
        }
    }

    fn opt_string_array(
        &self,
        map: &Map<String, Value>,
        parent: &str,
        key: &str,
    ) -> Result<Option<Vec<String>>, MigrateError> {
        let Some(value) = map.get(key) else {
            return Ok(None);
        };
        if value.is_null() {
            return Ok(None);
        }
        let field = format!("{parent}.{key}");
        let values = self.array(value, &field)?;
        let mut out = Vec::with_capacity(values.len());
        for (index, value) in values.iter().enumerate() {
            out.push(self.string(value, &format!("{field}[{index}]"))?.to_owned());
        }
        Ok(Some(out))
    }

    fn steps(&self, value: &Value, field: &str) -> Result<Vec<f64>, MigrateError> {
        let values = self.array(value, field)?;
        let mut steps = Vec::with_capacity(values.len());
        for (index, value) in values.iter().enumerate() {
            steps.push(self.f64(value, &format!("{field}[{index}]"))?);
        }
        Ok(steps)
    }

    fn optional_u8_array(
        &self,
        map: &Map<String, Value>,
        parent: &str,
        key: &str,
        expected_len: usize,
    ) -> Result<Option<Vec<Option<u8>>>, MigrateError> {
        let Some(value) = map.get(key) else {
            return Ok(None);
        };
        if value.is_null() {
            return Ok(None);
        }
        let field = format!("{parent}.{key}");
        let values = self.array(value, &field)?;
        if values.len() != expected_len {
            return Err(MigrateError::InvalidValue {
                path: self.path.to_path_buf(),
                field: field.clone(),
                detail: format!(
                    "长度 {} 与同一 track 的 steps 长度 {expected_len} 不一致",
                    values.len()
                ),
            });
        }
        let mut out = Vec::with_capacity(values.len());
        for (index, value) in values.iter().enumerate() {
            if value.is_null() {
                out.push(None);
                continue;
            }
            let raw = self.u64(value, &format!("{field}[{index}]"))?;
            let narrowed = u8::try_from(raw).map_err(|_| MigrateError::InvalidValue {
                path: self.path.to_path_buf(),
                field: format!("{field}[{index}]"),
                detail: format!("{raw} 超过 MIDI 的 0..=127"),
            })?;
            if narrowed > 127 {
                return Err(MigrateError::InvalidValue {
                    path: self.path.to_path_buf(),
                    field: format!("{field}[{index}]"),
                    detail: format!("{narrowed} 超过 MIDI 的 0..=127"),
                });
            }
            out.push(Some(narrowed));
        }
        Ok(Some(out))
    }

    fn field_type(&self, field: &str, expected: &'static str, value: &Value) -> MigrateError {
        MigrateError::FieldType {
            path: self.path.to_path_buf(),
            field: field.to_owned(),
            expected,
            found: json_kind(value),
        }
    }

    fn parse_pattern(
        &self,
        slot: &str,
        value: &Value,
        step_ticks: u64,
        losses: &mut BTreeSet<String>,
    ) -> Result<Vec<ParsedTrack>, MigrateError> {
        let field = format!("project.patterns.{slot}");
        let pattern = self.object(value, &field)?;
        let tracks_field = format!("{field}.tracks");
        let tracks = self.array(self.get(pattern, &field, "tracks")?, &tracks_field)?;
        let mut parsed = Vec::with_capacity(tracks.len());
        for (index, track_value) in tracks.iter().enumerate() {
            let track_field = format!("{tracks_field}[{index}]");
            let track = self.object(track_value, &track_field)?;
            let track_id = self.string_at(track, &track_field, "track_id")?;
            let name = self.string_at(track, &track_field, "name")?;
            let instrument = self.string_at(track, &track_field, "instrument")?;
            let steps = self.steps(
                self.get(track, &track_field, "steps")?,
                &format!("{track_field}.steps"),
            )?;
            let velocity = self.optional_u8_array(track, &track_field, "velocity", steps.len())?;
            let pitch = self.optional_u8_array(track, &track_field, "pitch", steps.len())?;
            let volume = self.opt_f64(track, "volume")?.unwrap_or(1.0);
            let pan = self.opt_f64(track, "pan")?.unwrap_or(0.0);
            if !volume.is_finite() {
                return Err(MigrateError::InvalidValue {
                    path: self.path.to_path_buf(),
                    field: format!("{track_field}.volume"),
                    detail: "必须有限".to_owned(),
                });
            }
            if !pan.is_finite() {
                return Err(MigrateError::InvalidValue {
                    path: self.path.to_path_buf(),
                    field: format!("{track_field}.pan"),
                    detail: "必须有限".to_owned(),
                });
            }

            let mut notes = Vec::new();
            for (step_index, step) in steps.iter().enumerate() {
                if *step == 0.0 {
                    continue;
                }
                let pitch_value = pitch
                    .as_ref()
                    .and_then(|values| values[step_index])
                    .or_else(|| drum_pitch(instrument))
                    .unwrap_or_else(|| {
                        losses.insert(format!(
                            "track `{track_id}` (instrument `{instrument}`) 没有 pitch, 且不是已知鼓件 \
                             ⇒ 固定 pitch=60 (有损)"
                        ));
                        60
                    });
                let velocity_value = velocity
                    .as_ref()
                    .and_then(|values| values[step_index])
                    .unwrap_or(DEFAULT_VELOCITY)
                    .max(1);
                notes.push(LegacyNote {
                    start_tick: (step_index as u64) * step_ticks,
                    pitch: pitch_value,
                    velocity: velocity_value,
                });
            }
            parsed.push(ParsedTrack {
                track_id: track_id.to_owned(),
                name: name.to_owned(),
                instrument: instrument.to_owned(),
                volume: volume as f32,
                pan: pan as f32,
                natural_ticks: (steps.len() as u64) * step_ticks,
                notes,
            });
        }
        Ok(parsed)
    }

    fn parse_sections(
        &self,
        project: &Map<String, Value>,
        losses: &mut BTreeSet<String>,
    ) -> Result<(Vec<SectionSpec>, usize, &'static str), MigrateError> {
        if let Some(value) = project.get("sections")
            && !value.is_null()
        {
            let sections = self.array(value, "project.sections")?;
            if !sections.is_empty() {
                let mut specs = Vec::with_capacity(sections.len());
                for (index, section_value) in sections.iter().enumerate() {
                    let field = format!("project.sections[{index}]");
                    let section = self.object(section_value, &field)?;
                    let slot = self.string_at(section, &field, "slot")?.to_owned();
                    let bars = self.opt_u64(section, "bars")?.unwrap_or(1);
                    if bars == 0 {
                        return Err(MigrateError::InvalidValue {
                            path: self.path.to_path_buf(),
                            field: format!("{field}.bars"),
                            detail: "必须 ≥ 1".to_owned(),
                        });
                    }
                    let label = self
                        .opt_string(section, "label")
                        .unwrap_or_else(|| format!("Section {}", index + 1));
                    let mute = self
                        .opt_string_array(section, &field, "mute")?
                        .unwrap_or_default()
                        .into_iter()
                        .collect();
                    if section
                        .get("overrides")
                        .is_some_and(|value| !value.is_null())
                    {
                        losses.insert(
                            "section `overrides` (fill / velocityRamp …) 未迁移".to_owned(),
                        );
                    }
                    if section
                        .get("velocityScale")
                        .is_some_and(|value| !value.is_null())
                    {
                        losses.insert("section `velocityScale` 未迁移".to_owned());
                    }
                    specs.push(SectionSpec {
                        slot,
                        bars,
                        label,
                        mute,
                        synthetic: false,
                    });
                }
                return Ok((specs, sections.len(), "sections"));
            }
        }

        if let Some(chain) = self.opt_string_array(project, "project", "songChain")?
            && !chain.is_empty()
        {
            let specs = chain
                .into_iter()
                .map(|slot| SectionSpec {
                    label: slot.clone(),
                    slot,
                    bars: 1,
                    mute: BTreeSet::new(),
                    synthetic: false,
                })
                .collect();
            return Ok((specs, 0, "songChain"));
        }

        Ok((
            vec![SectionSpec {
                slot: "A".to_owned(),
                bars: 1,
                label: "Section 1".to_owned(),
                mute: BTreeSet::new(),
                synthetic: true,
            }],
            0,
            "none",
        ))
    }
}

/// 确定性的 fixture ULID 生成器（`01J8ZQ` + 20 位十进制序号）。
///
/// 为什么不 `EntityId::new()`：迁移同一份旧工程两次应当得到**同一条** ID 序列，
/// 判据才能逐字节比较；而 `ulid` 不是本 crate 的直接依赖，所以这里用
/// `EntityId::from_str` 构造固定文本（与 `yeban-model` 的规范样本同一个手法）。
struct IdGen {
    counter: u64,
}

impl IdGen {
    fn next(&mut self) -> EntityId {
        let index = self.counter;
        self.counter += 1;
        EntityId::from_str(&format!("01J8ZQ{index:020}")).expect("deterministic fixture ULID")
    }
}

/// 旧工程里 `resolution` 的一个步长对应多少 tick（960 PPQ）。
fn resolution_ticks(resolution: &str) -> Option<u64> {
    match resolution {
        "1/8" => Some(PPQ / 2),
        "1/16" => Some(PPQ / 4),
        "1/32" => Some(PPQ / 8),
        _ => None,
    }
}

/// `"4/4"` ⇒ [`TimeSignature`]；分母必须是规范枚举之一。
fn parse_time_signature(value: &str, path: &Path) -> Result<TimeSignature, MigrateError> {
    let Some((numerator, denominator)) = value.split_once('/') else {
        return Err(MigrateError::InvalidValue {
            path: path.to_path_buf(),
            field: "project.timeSignature".to_owned(),
            detail: format!("`{value}` 不是 `n/d`"),
        });
    };
    let numerator: u8 = numerator.parse().map_err(|_| MigrateError::InvalidValue {
        path: path.to_path_buf(),
        field: "project.timeSignature".to_owned(),
        detail: format!("`{value}` 的分子不是 u8"),
    })?;
    let denominator: u8 = denominator
        .parse()
        .map_err(|_| MigrateError::InvalidValue {
            path: path.to_path_buf(),
            field: "project.timeSignature".to_owned(),
            detail: format!("`{value}` 的分母不是 u8"),
        })?;
    let time_signature = TimeSignature {
        numerator,
        denominator,
    };
    time_signature
        .validate()
        .map_err(|error| MigrateError::InvalidValue {
            path: path.to_path_buf(),
            field: "project.timeSignature".to_owned(),
            detail: format!("`{value}` 非法: {error}"),
        })?;
    Ok(time_signature)
}

/// 旧工程的音量是 **0..1 的线性增益**，新模型是 dB。
///
/// ⭐ `#234`：`log10` 走纯 Rust `libm::log10f`（⛔ 不是宿主 `f32::log10`）。它虽
/// **不进音频字节**，但迁移结果会**落盘成 `.yeban` 容器** ⇒ 同一份旧工程必须在每个
/// 平台迁移出同一串字节（[ARCH-DET-001]）；`libm` 已因最后两条音频路径成为本 crate
/// 的依赖 ⇒ 顺手收口，不留第二套口径。
fn volume_to_db(volume: f32) -> f32 {
    if volume <= 0.0 {
        -60.0
    } else {
        (20.0 * libm::log10f(volume)).clamp(-60.0, 24.0)
    }
}

/// 旧工程的鼓件名 ⇒ General MIDI 打击乐音高。
fn drum_pitch(instrument: &str) -> Option<u8> {
    match instrument {
        "acoustic_kick" | "kick" => Some(36),
        "rimshot" | "side_stick" => Some(37),
        "snare" | "acoustic_snare" => Some(38),
        "clap" | "hand_clap" => Some(39),
        "closed_hat" | "hihat" | "closed_hihat" => Some(42),
        "open_hat" | "open_hihat" => Some(46),
        "tom" | "low_tom" => Some(45),
        "crash" => Some(49),
        "ride" => Some(51),
        "rim_shaker" | "shaker" => Some(82),
        _ => None,
    }
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn quoted_path(path: &Path) -> String {
    crate::cli::quoted(&path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一个最小的 v1 工程包：2 条轨 × 2 套 pattern × 16 步。
    const FIXTURE: &str = r#"{
        "format": "groove-project",
        "version": 1,
        "exportedAt": 1700000000000,
        "appVersion": "2.34.0",
        "project": {
            "id": "proj_fixture",
            "name": "迁移夹具",
            "genreId": "neo-soul",
            "genreName": "Neo-Soul",
            "bpm": 84,
            "swing": 30,
            "timeSignature": "4/4",
            "resolution": "1/16",
            "stepCount": 16,
            "patterns": {
                "A": {
                    "genre_id": "neo-soul",
                    "bpm": 84,
                    "scale": "F minor",
                    "swing": 45,
                    "tracks": [
                        {"track_id": "kick", "name": "Kick", "instrument": "acoustic_kick",
                         "steps": [1,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0],
                         "velocity": [120,0,0,0,0,0,0,0,105,0,0,0,0,0,0,0],
                         "volume": 0.9, "pan": 0.0},
                        {"track_id": "bass", "name": "Bass", "instrument": "finger_bass",
                         "steps": [1,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0],
                         "pitch": [41,null,44,null,null,null,null,null,null,null,null,null,null,null,null,null],
                         "volume": 0.9, "pan": 0.0}
                    ]
                },
                "B": {
                    "genre_id": "neo-soul",
                    "bpm": 84,
                    "scale": "F minor",
                    "swing": 45,
                    "tracks": [
                        {"track_id": "kick", "name": "Kick", "instrument": "acoustic_kick",
                         "steps": [1,0,0,0,1,0,0,0,1,0,0,0,0,0,0,0],
                         "velocity": [120,0,0,0,110,0,0,0,105,0,0,0,0,0,0,0],
                         "volume": 0.9, "pan": 0.0},
                        {"track_id": "bass", "name": "Bass", "instrument": "finger_bass",
                         "steps": [1,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0],
                         "pitch": [41,null,null,null,null,null,44,null,null,null,null,null,null,null,null,null],
                         "volume": 0.9, "pan": 0.0}
                    ]
                }
            },
            "activeSlot": "A",
            "songMode": true,
            "songChain": ["A", "B"],
            "sections": [
                {"id": "sec-1", "slot": "A", "bars": 1, "label": "intro"},
                {"id": "sec-2", "slot": "B", "bars": 2, "label": "chorus"}
            ],
            "loopRange": null,
            "effectsRack": {"filterEnabled": false},
            "drumKit": "acoustic",
            "isMetronome": false,
            "isCountIn": false,
            "tags": ["fixture"],
            "isFavorite": false,
            "notes": "迁移夹具说明",
            "createdAt": 1700000000000,
            "updatedAt": 1700000000001
        }
    }"#;

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yeban-migrate-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    #[test]
    fn fixture_migrates_one_to_one_on_tracks_and_notes() {
        let dir = scratch_dir("unit");
        let input = dir.join("old.groove");
        let output = dir.join("new.yeban");
        std::fs::write(&input, FIXTURE).expect("写夹具");
        let lines = run(&input, &output).expect("迁移成功");
        let counts = lines
            .iter()
            .find(|line| line.starts_with("migrated-groove-counts:"))
            .expect("必须有计数行");
        assert!(counts.contains("legacy-tracks=2"), "{counts}");
        assert!(counts.contains("migrated-tracks-musical=2"), "{counts}");
        assert!(counts.contains("legacy-notes=9"), "{counts}");
        assert!(counts.contains("migrated-notes=9"), "{counts}");
        assert!(lines.iter().any(|line| line.starts_with("migrated: path=")));
        assert!(output.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_field_is_reported_with_its_full_path() {
        let path = Path::new("fixture.groove");
        let broken = FIXTURE.replace("\"patterns\":", "\"patternz\":");
        let error = migrate_bytes(broken.as_bytes(), path).expect_err("缺 patterns 必须失败");
        let text = error.to_string();
        assert!(
            text.contains("缺少字段 `project.patterns`"),
            "错误必须点名完整字段路径: {text}"
        );
    }

    #[test]
    fn groove_package_is_rejected_loudly() {
        let path = Path::new("fixture.groove");
        let broken = r#"{"format":"groove-package","name":"x"}"#;
        let error = migrate_bytes(broken.as_bytes(), path).expect_err("未知格式必须失败");
        let text = error.to_string();
        assert!(text.contains("groove-package"), "{text}");
        assert!(text.contains("没有读取器"), "{text}");
    }

    #[test]
    fn the_real_legacy_file_migrates_when_present() {
        let path = Path::new("/Users/crow/Downloads/假如时光能够倒流.groove");
        if !path.exists() {
            eprintln!("skip: 本机没有真旧工程 {path:?}");
            return;
        }
        let dir = scratch_dir("real");
        let output = dir.join("real.yeban");
        let lines = run(path, &output).expect("真旧工程迁移成功");
        let counts = lines
            .iter()
            .find(|line| line.starts_with("migrated-groove-counts:"))
            .expect("计数行");
        assert!(counts.contains("legacy-tracks=8"), "{counts}");
        assert!(counts.contains("legacy-notes=59"), "{counts}");
        assert!(counts.contains("migrated-tracks-musical=8"), "{counts}");
        assert!(counts.contains("migrated-tracks-all=9"), "{counts}");
        assert!(counts.contains("migrated-notes=59"), "{counts}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
