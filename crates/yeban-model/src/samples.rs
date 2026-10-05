//! 规范样本导出（跨语言契约对账的**非测试**入口）[MUST-GATE-010, TEST-SPEC-005]。
//!
//! `scripts/gates/validate_schemas.py --samples-dir <dir>` 会把本模块导出的样本
//! 交给 **Python jsonschema** 逐份对账，而样本本身由 **Rust serde** 写出 ——
//! 两个独立实现互相钉住：serde 写的字节必须被契约接受，契约漂移或实现漂移任意一边都会变红。
//!
//! ## 为什么要有这个模块，而不是"测试的副作用"
//!
//! 样本曾只能靠 `#[test]` 的副作用产生。那让"导出失败"看起来像"测试通过"，
//! 也让 CI 必须靠跑测试来生成对账输入。现在 **同一份实现** 有两个入口：
//!
//! - `crates/yeban-model/examples/export_schema_samples.rs`（可执行，`--out <dir>`）；
//! - `crates/yeban-model/src/project.rs` 的 `export_schema_samples_to_target` 测试（本机门禁用）。
//!
//! 两者都调用 [`export_all`]，因此不存在"测试里那套"和"CI 里那套"两份实现。
//!
//! ## 命名的契约
//!
//! 文件名的前缀决定用哪份 schema（`scripts/gates/validate_schemas.py` 的 `SAMPLE_SCHEMA_MAP`）：
//!
//! | 文件 | schema |
//! | :--- | :--- |
//! | `project.default.json` / `project.filled.json` | `schemas/project.schema.json` |
//! | `ops.default.json` / `ops.filled.json` | `schemas/ops.schema.json` |
//!
//! 样本里刻意使用**确定性 ULID**（`fixture_id`）而不是 `EntityId::new()`：
//! 样本因此逐字节稳定，可以被 diff、被缓存、被 CI 断言"两次导出完全一致"。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::ModelError;
use crate::ids::{AssetHash, EntityId};
use crate::music::{CurveType, MidiNote};
use crate::ops::{Op, OpOrigin, StampedOp};
use crate::project::{
    AssetMetadata, AutomationLane, AutomationPoint, AutomationTarget, AutomationValueDomain,
    AutomationWriteMode, BitDepth, BlockSize, ClipContent, ClipPlacement, ClipPoolEntry,
    DEFAULT_RNG_SEED, DEFAULT_WRITER_VERSION, DeviceDefinition, DeviceKind, LaunchQuantization,
    LoopConfig, MIN_READER_VERSION, MacroMapping, MacroParameter, MediaKind, PanLaw,
    ParameterValue, ProjectAudioConfig, ProjectMetadata, RoutingEdge, RoutingGraph, RoutingKind,
    SCHEMA_VERSION, SampleRate, SceneV3, SectionV3, TimeSignature, TrackKind, TrackV3,
    TransportConfig, YebanProjectV1,
};

/// 默认样本目录名（相对工作区 `target/`）。
pub const SAMPLES_DIR_NAME: &str = "schema-samples";

/// `project` 样本：默认（空）工程。
pub const PROJECT_DEFAULT_FILE: &str = "project.default.json";

/// `project` 样本：填满全部子类型的工程。
pub const PROJECT_FILLED_FILE: &str = "project.filled.json";

/// `ops` 样本：最小 `StampedOp` 信封。
pub const OPS_DEFAULT_FILE: &str = "ops.default.json";

/// `ops` 样本：最丰富 `StampedOp` 信封（`McpProposal` 来源 + 原子 `Batch`）。
pub const OPS_FILLED_FILE: &str = "ops.filled.json";

/// 样本里统一使用的时间戳（2025-10-05T00:00:00Z 附近的一个整数毫秒值）。
pub const SAMPLE_TIMESTAMP: u64 = 1_760_000_000_000;

/// 样本导出失败。
#[derive(Debug, thiserror::Error)]
pub enum SampleExportError {
    /// 文件系统错误。
    #[error("写入样本失败: {0}")]
    Io(#[from] std::io::Error),
    /// JSON 序列化错误。
    #[error("序列化样本失败: {0}")]
    Serialize(#[from] serde_json::Error),
    /// 工程样本未通过模型层结构校验。
    #[error("样本 `{file}` 未通过结构校验: {source}")]
    InvalidProject {
        /// 文件名。
        file: String,
        /// 模型层错误。
        source: ModelError,
    },
    /// `StampedOp` 的 `op` 信封形状不合法（必须恰好一个变体键）。
    #[error("样本 `{file}` 的 op 信封非法: {detail}")]
    InvalidOpEnvelope {
        /// 文件名。
        file: String,
        /// 人话解释。
        detail: String,
    },
}

/// 构造确定性的规范 ULID 文本（前 6 位固定，余下 20 位十进制序号）。
///
/// 用固定 ULID 而非 `EntityId::new()`，样本因此逐字节可复现。
pub(crate) fn fixture_id(index: u128) -> EntityId {
    use std::str::FromStr as _;
    EntityId::from_str(&format!("01J8ZQ{index:020}")).expect("canonical fixture ulid")
}

/// 最小主总线音轨夹具。
pub(crate) fn master_track(id: EntityId) -> TrackV3 {
    TrackV3 {
        id,
        name: "Master".to_owned(),
        kind: TrackKind::Master,
        ..TrackV3::default()
    }
}

/// 最小 MIDI 音轨夹具。
pub(crate) fn midi_track(id: EntityId) -> TrackV3 {
    TrackV3 {
        id,
        name: "Lead".to_owned(),
        ..TrackV3::default()
    }
}

/// 空 MIDI 片段夹具。
pub(crate) fn midi_clip(id: EntityId) -> ClipPoolEntry {
    ClipPoolEntry {
        id,
        name: "Clip".to_owned(),
        content: ClipContent::default(),
    }
}

/// 默认工程样本（合法、可读、无音轨）。
#[must_use]
pub fn default_project() -> YebanProjectV1 {
    YebanProjectV1::default()
}

/// 构造一个把全部子类型都填满的规范样本（样本 JSON 必须逐字节稳定）。
pub fn filled_project() -> YebanProjectV1 {
    let master_id = fixture_id(1);
    let lead_id = fixture_id(2);
    let bass_id = fixture_id(3);
    let aux_id = fixture_id(4);

    let mut lead = midi_track(lead_id);
    lead.volume_db = -3.0;
    lead.pan = -0.25;
    lead.color = Some("#FF8800".to_owned());
    lead.devices = vec![DeviceDefinition {
        id: fixture_id(20),
        name: "Yeban PolySynth".to_owned(),
        kind: DeviceKind::InternalInstrument,
        bypassed: false,
        params: vec![
            ParameterValue {
                name: "cutoff".to_owned(),
                value: 1200.0,
                unit: Some("Hz".to_owned()),
            },
            ParameterValue {
                name: "resonance".to_owned(),
                value: 0.35,
                unit: Some("%".to_owned()),
            },
        ],
        // 非零延迟: 让 ARCH-PDC-001 的字段在规范样本里被真正覆盖 (0 表示"未上报")。
        // 32 采样点 = 内部合成器的块处理延迟, 会被关键路径分析用于插入延迟线。
        latency_samples: 32,
    }];
    lead.macros = vec![MacroParameter {
        name: "Brightness".to_owned(),
        value: 0.5,
        mappings: vec![MacroMapping {
            target: AutomationTarget::DeviceParam {
                track_id: lead_id,
                slot_index: 0,
                param_index: 0,
            },
            depth: 0.8,
        }],
    }];
    lead.automation_lanes.insert(
        AutomationTarget::TrackVolume { track_id: lead_id },
        AutomationLane {
            target: AutomationTarget::TrackVolume { track_id: lead_id },
            points: BTreeMap::from([
                (
                    fixture_id(40),
                    AutomationPoint {
                        id: fixture_id(40),
                        tick: 0,
                        value: -6.0,
                        curve: CurveType::Linear,
                    },
                ),
                (
                    fixture_id(41),
                    AutomationPoint {
                        id: fixture_id(41),
                        tick: 3840,
                        value: 0.0,
                        curve: CurveType::SCurve,
                    },
                ),
            ]),
            // 规范样本刻意**显式**给出三个字段：它们都是"自动化泳道"规范形状的一部分
            // （读开关 / 写模式 / 取值域）。ADR-0001 D43 之后 `read_enabled` / `write_mode`
            // / `points` 是**必需**的（缺键即 `missing field`），只有 `domain` 是
            // `Option<T>` 语义默认。严格性判据见 `tests/no_compat.rs`。
            read_enabled: true,
            write_mode: AutomationWriteMode::Touch,
            domain: Some(AutomationValueDomain::new(-60.0, 12.0).expect("音量取值域端点必然有限")),
        },
    );

    let clip_id = fixture_id(10);
    let mut clip = midi_clip(clip_id);
    if let Some(notes) = clip.content.notes_mut() {
        for (index, (start_tick, pitch)) in [(0_u64, 60_u8), (960, 64), (1920, 67), (2880, 72)]
            .into_iter()
            .enumerate()
        {
            let note_id = fixture_id(100 + index as u128);
            notes.insert(
                note_id,
                MidiNote {
                    probability: if index == 3 { Some(0.75) } else { None },
                    ratchet: if index == 1 { Some(2) } else { None },
                    micro_timing_ticks: if index == 2 { Some(-12) } else { None },
                    syllable: Some(["do", "re", "mi", "fa"][index].to_owned()),
                    ..MidiNote::new(note_id, start_tick, pitch, 480)
                },
            );
        }
    }

    let audio_clip_id = fixture_id(11);
    let audio_clip = ClipPoolEntry {
        id: audio_clip_id,
        name: "Kick".to_owned(),
        content: ClipContent::Audio {
            asset: AssetHash::of_bytes(b"yeban-kick-sample"),
            gain_db: -1.5,
        },
    };

    let placement_id = fixture_id(50);
    lead.clips.insert(
        placement_id,
        ClipPlacement {
            id: placement_id,
            clip_id,
            start_tick: 0,
            duration_ticks: 3840,
            loop_config: LoopConfig {
                enabled: true,
                start_tick: 0,
                end_tick: 3840,
            },
            muted: false,
        },
    );
    let audio_placement_id = fixture_id(51);
    let mut bass = TrackV3 {
        id: bass_id,
        name: "Bass".to_owned(),
        kind: TrackKind::Audio,
        volume_db: -6.0,
        pan: 0.0,
        mute: false,
        solo: false,
        solo_safe: true,
        folder_id: None,
        color: Some("#3366FF".to_owned()),
        ..TrackV3::default()
    };
    bass.clips.insert(
        audio_placement_id,
        ClipPlacement {
            id: audio_placement_id,
            clip_id: audio_clip_id,
            start_tick: 1920,
            duration_ticks: 960,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );

    let aux = TrackV3 {
        id: aux_id,
        name: "Aux Reverb".to_owned(),
        kind: TrackKind::AuxReturn,
        volume_db: -9.0,
        ..TrackV3::default()
    };

    let lead_to_master = fixture_id(60);
    let bass_to_master = fixture_id(61);
    let lead_to_aux = fixture_id(62);
    let mut edges = BTreeMap::new();
    for (id, source, destination, kind, gain_db) in [
        (
            lead_to_master,
            lead_id,
            master_id,
            RoutingKind::TrackToBus,
            None,
        ),
        (
            bass_to_master,
            bass_id,
            master_id,
            RoutingKind::TrackToBus,
            None,
        ),
        (
            lead_to_aux,
            lead_id,
            aux_id,
            RoutingKind::SendToAux,
            Some(-12.0),
        ),
    ] {
        edges.insert(
            id,
            RoutingEdge {
                id,
                source_node: source,
                destination_node: destination,
                kind,
                gain_db,
            },
        );
    }

    let asset_hash = AssetHash::of_bytes(b"yeban-kick-sample");
    YebanProjectV1 {
        schema_version: SCHEMA_VERSION,
        min_reader_version: MIN_READER_VERSION,
        writer_version: DEFAULT_WRITER_VERSION.to_owned(),
        id: fixture_id(999),
        title: "Yeban Model Core Sample".to_owned(),
        author: "Yeban Project Contributors".to_owned(),
        bpm: 128.0,
        time_signature: TimeSignature {
            numerator: 4,
            denominator: 4,
        },
        audio_config: ProjectAudioConfig {
            sample_rate: SampleRate::Hz48000,
            block_size: BlockSize::Frames256,
            bit_depth: BitDepth::Float32,
            pan_law: PanLaw::ConstantPowerMinus3dB,
        },
        rng_seed: DEFAULT_RNG_SEED,
        metadata: ProjectMetadata {
            description: "Cross-implementation schema sample".to_owned(),
            created_at_unix_ms: 1_760_000_000_000,
            modified_at_unix_ms: 1_760_000_000_000,
            tags: vec!["sample".to_owned(), "schema".to_owned()],
        },
        transport: TransportConfig {
            metronome_enabled: false,
            count_in_bars: 2,
            launch_quantization: LaunchQuantization::Bar,
        },
        sections: BTreeMap::from([
            (
                fixture_id(70),
                SectionV3 {
                    id: fixture_id(70),
                    name: "Intro".to_owned(),
                    start_tick: 0,
                    end_tick: 7680,
                    color: Some("#22AA88".to_owned()),
                },
            ),
            (
                fixture_id(71),
                SectionV3 {
                    id: fixture_id(71),
                    name: "Drop".to_owned(),
                    start_tick: 7680,
                    end_tick: 15360,
                    color: None,
                },
            ),
        ]),
        tracks: BTreeMap::from([
            (master_id, master_track(master_id)),
            (lead_id, lead),
            (bass_id, bass),
            (aux_id, aux),
        ]),
        master_bus_track_id: master_id,
        routing_graph: RoutingGraph {
            nodes: vec![lead_id, bass_id, aux_id, master_id],
            edges,
        },
        scenes: BTreeMap::from([(
            fixture_id(80),
            SceneV3 {
                id: fixture_id(80),
                name: "Scene 1".to_owned(),
                tempo: Some(128.0),
                color: None,
            },
        )]),
        clip_pool: BTreeMap::from([(clip_id, clip), (audio_clip_id, audio_clip)]),
        assets: BTreeMap::from([(
            asset_hash.clone(),
            AssetMetadata {
                hash: asset_hash,
                original_path: "samples/kick.wav".to_owned(),
                byte_len: 44_100,
                media_kind: MediaKind::Audio,
                license: "CC0-1.0".to_owned(),
            },
        )]),
    }
}

/// `ops` 样本：最小信封 —— `UserUi` 来源 + 一个自足的 `SetSection`。
///
/// 该 `Op` **可以**直接作用在 [`default_project`] 上（新建一个段落），
/// 因此"默认样本"这一对是自洽的，有测试钉住。
#[must_use]
pub fn default_stamped_op() -> StampedOp {
    let section_id = fixture_id(70);
    StampedOp::new(
        OpOrigin::UserUi,
        SAMPLE_TIMESTAMP,
        Op::SetSection {
            section_id,
            old_section: None,
            new_section: SectionV3 {
                id: section_id,
                name: "Intro".to_owned(),
                start_tick: 0,
                end_tick: 7680,
                color: Some("#22AA88".to_owned()),
            },
        },
    )
}

/// `ops` 样本：最丰富信封 —— `McpProposal` 来源 + 原子 `Batch`（AI 提案一键撤销的形状）。
///
/// 子操作**全部**可以按顺序作用在 [`filled_project`] 上，因此"填充样本"这一对也是自洽的：
/// 它同时演示了"外部标签对象形式的 origin"（ADR-0001 D13）与"批量的逆是一个批量"。
#[must_use]
pub fn filled_stamped_op() -> StampedOp {
    let lead_id = fixture_id(2);
    let aux_id = fixture_id(4);
    let clip_id = fixture_id(10);
    let volume_target = AutomationTarget::TrackVolume { track_id: lead_id };
    let existing_point = AutomationPoint {
        id: fixture_id(40),
        tick: 0,
        value: -6.0,
        curve: CurveType::Linear,
    };
    StampedOp::new(
        OpOrigin::McpProposal {
            proposal_id: fixture_id(9001),
            agent_name: "yeban-agent".to_owned(),
        },
        SAMPLE_TIMESTAMP,
        Op::Batch {
            ops: vec![
                Op::AddNote {
                    track_id: lead_id,
                    clip_id,
                    note: MidiNote {
                        probability: Some(0.75),
                        ratchet: Some(2),
                        micro_timing_ticks: Some(-12),
                        syllable: Some("la".to_owned()),
                        ..MidiNote::new(fixture_id(104), 3840, 74, 480)
                    },
                },
                Op::SetAutomationPoint {
                    target: volume_target,
                    point_id: existing_point.id,
                    old_point: Some(existing_point),
                    new_point: AutomationPoint {
                        id: existing_point.id,
                        tick: 3840,
                        value: 0.0,
                        curve: CurveType::SCurve,
                    },
                },
                Op::SetRoutingGain {
                    edge_id: fixture_id(60),
                    old_gain_db: None,
                    new_gain_db: Some(-3.0),
                },
                Op::ConnectRouting {
                    edge: RoutingEdge {
                        // 60/61/62 已被 `filled_project` 占用，这里必须用新的边身份。
                        id: fixture_id(63),
                        source_node: lead_id,
                        destination_node: aux_id,
                        kind: RoutingKind::SendToAux,
                        gain_db: Some(-12.0),
                    },
                },
            ],
            description: "AI 提案: 副歌加一层八度".to_owned(),
        },
    )
}

/// 默认样本目录：`<repo>/target/schema-samples`。
///
/// 遵循 Cargo 的 `CARGO_TARGET_DIR`（若设置），否则用
/// `<crate>/../../target`（即工作区 target）。
#[must_use]
pub fn default_out_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(
        || manifest_dir.join("..").join("..").join("target"),
        PathBuf::from,
    );
    target.join(SAMPLES_DIR_NAME)
}

/// 把四份规范样本写到 `out_dir`，返回实际写出的文件路径（顺序固定）。
///
/// 写之前先做 **Rust 侧结构校验**（工程样本 `validate()` + `check_readable()`，
/// op 样本检查"恰好一个变体键"），因此磁盘上永远不会出现一份连自己都不合法的样本。
///
/// # Errors
///
/// 目录创建/写入失败、序列化失败，或任一样本未通过结构校验。
pub fn export_all(out_dir: &Path) -> Result<Vec<PathBuf>, SampleExportError> {
    std::fs::create_dir_all(out_dir)?;
    let written = vec![
        write_project(out_dir, PROJECT_DEFAULT_FILE, &default_project())?,
        write_project(out_dir, PROJECT_FILLED_FILE, &filled_project())?,
        write_stamped_op(out_dir, OPS_DEFAULT_FILE, &default_stamped_op())?,
        write_stamped_op(out_dir, OPS_FILLED_FILE, &filled_stamped_op())?,
    ];
    Ok(written)
}

/// 把四份规范样本写到 [`default_out_dir`]。
///
/// # Errors
///
/// 同 [`export_all`]。
pub fn export_to_default_dir() -> Result<Vec<PathBuf>, SampleExportError> {
    export_all(&default_out_dir())
}

/// 校验并写出一个工程样本。
fn write_project(
    out_dir: &Path,
    file: &str,
    document: &YebanProjectV1,
) -> Result<PathBuf, SampleExportError> {
    document
        .check_readable()
        .map_err(|source| SampleExportError::InvalidProject {
            file: file.to_owned(),
            source,
        })?;
    document
        .validate()
        .map_err(|source| SampleExportError::InvalidProject {
            file: file.to_owned(),
            source,
        })?;
    write_json(out_dir, file, document)
}

/// 校验并写出一个 `StampedOp` 样本。
fn write_stamped_op(
    out_dir: &Path,
    file: &str,
    stamped: &StampedOp,
) -> Result<PathBuf, SampleExportError> {
    check_op_envelope(stamped).map_err(|detail| SampleExportError::InvalidOpEnvelope {
        file: file.to_owned(),
        detail,
    })?;
    write_json(out_dir, file, stamped)
}

/// 检查 `StampedOp` 的 `op` 是"恰好一个变体键"的外部标签对象。
///
/// `schemas/ops.schema.json` 的 `op.oneOf` 要求**恰好匹配一个**分支；
/// 由于每个分支都 `required` 一个互不相同的键，这在 JSON 层面等价于
/// "`op` 对象恰好有 1 个键，且键名就是变体名"。这里把这条语义落成 Rust 侧判据。
///
/// # Errors
///
/// 返回人话解释的错误描述。
pub fn check_op_envelope(stamped: &StampedOp) -> Result<(), String> {
    let value = serde_json::to_value(&stamped.op).map_err(|error| error.to_string())?;
    let object = value
        .as_object()
        .ok_or_else(|| format!("op 必须是 JSON 对象, 实际 {}", json_kind(&value)))?;
    match object.len() {
        1 => {}
        other => return Err(format!("op 必须恰好有 1 个变体键, 实际 {other} 个")),
    }
    let (key, _) = object.iter().next().ok_or("op 不能为空对象")?;
    if key != stamped.op.name() {
        return Err(format!(
            "op 的 JSON 键 `{key}` 与 Op::name() `{}` 不一致",
            stamped.op.name()
        ));
    }
    Ok(())
}

/// JSON 值的类型名（用于错误信息）。
fn json_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// 以"美化 JSON + 结尾换行"写出一个样本。
fn write_json<T: Serialize>(
    out_dir: &Path,
    file: &str,
    value: &T,
) -> Result<PathBuf, SampleExportError> {
    let mut json = serde_json::to_string_pretty(value)?;
    json.push('\n');
    let path = out_dir.join(file);
    std::fs::write(&path, json)?;
    Ok(path)
}

/// `[BASELINE-003]` 造一个含 `note_count` 个音符的工程，供 10 万音符帧率场景使用。
///
/// **确定性**：音符 id 走 [`EntityId::new`]（Ulid 单调），所以两次调用的规范化 JSON 不一定逐字节相同 ——
/// 判据因此只断言"同一进程内音符数恰好为 `note_count`"与"两次调用的音符数一致"，
/// 而**不**谎称逐字节可复现（id 是本机生成的）。这条口径写在测试里，免得后来者误判。
#[must_use]
pub fn project_with_notes(note_count: usize) -> YebanProjectV1 {
    let mut project = filled_project();
    // 找一个带 MIDI 音符的 clip（`filled_project` 里 lead 轨必有）。
    // 用显式循环而不是 `find`：`Iterator::find` 会把 `&mut` 再套一层引用，
    // `entry.content.notes_mut()` 因此借不到可变（实测 E0596）。
    let mut target: Option<&mut crate::project::ClipPoolEntry> = None;
    for entry in project.clip_pool.values_mut() {
        if entry.content.notes().is_some() {
            target = Some(entry);
            break;
        }
    }
    let clip = target.expect("filled_project 必须含至少一个 MIDI 片段");
    let notes = clip.content.notes_mut().expect("上面已判定该片段带音符");
    notes.clear();
    for i in 0..note_count {
        let id = EntityId::new();
        // 铺开在 4 小节网格上, 音高在 36..=84 之间循环, 力度交替。
        // 铺开在**长时轴**上：每 240 tick 一个音符 ⇒ 10 万音符跨 2400 万 tick。
        // 为什么必须这样（第 184 轮的发现）：早先写成 `(i % 3840) * 4`，10 万音符全挤在 3840 tick 内,
        // 在 120 tpp 下只有约 128 px 宽 ⇒ **视口裁剪一个都裁不掉**, 而"帧率"量到的是挤成一团的病态场景。
        // 真实滚动场景必须让音符在时间轴上铺开, 裁剪才有意义。
        let start_tick = i as u64 * 240;
        let pitch = 36 + (i % 49) as u8;
        notes.insert(id, crate::music::MidiNote::new(id, start_tick, pitch, 240));
    }
    project
}

/// **演示工程的夹具**：`src/scene.rs` 里原先那组硬编码常量的模型化形态。
///
/// 它与 `scene::TRACK_NAMES` / `NOTE_ULIDS` / `CLIP_ULIDS` / `SECTION_NAMES` /
/// `SCENE_NAMES` / `FADER_DB_LABELS` **逐字对应**，并有判据钉住
/// （`demo_projection_reproduces_the_scene_constants`）。这样"演示数据"就不再是
/// 界面里的常量，而是**一个真正的 `YebanProjectV1`** —— 换成
/// [`crate::samples::filled_project`] 时走的是**同一条**代码路径。
#[must_use]
pub fn demo_project() -> YebanProjectV1 {
    use std::collections::BTreeMap;
    use std::str::FromStr as _;

    use crate::music::MidiNote;
    use crate::project::{ClipPoolEntry, LoopConfig, RoutingEdge, RoutingGraph, RoutingKind};

    let master_id = demo_id("M0");
    let track_ids: [EntityId; 6] = [
        demo_id("T1"),
        demo_id("T2"),
        demo_id("T3"),
        demo_id("T4"),
        demo_id("T5"),
        demo_id("T6"),
    ];
    // 与 `scene::TRACK_NAMES` 逐字一致；顺序 = 身份升序（`BTreeMap` 迭代序）。
    let track_names = ["鼓", "贝斯", "铺底", "主音", "弦乐", "打击"];
    // 与 `scene::FADER_DB_LABELS` 逐字一致（模型是权威，界面标签由投影生成）。
    let track_volumes = [-3.2_f32, -6.0, -8.4, -4.8, -12.0, -10.6];
    let track_kinds = [
        TrackKind::Midi,
        TrackKind::Audio,
        TrackKind::Midi,
        TrackKind::Midi,
        TrackKind::Midi,
        TrackKind::Audio,
    ];
    let track_colors = [
        Some("#f7e6b0"),
        None,
        Some("#22aa88"),
        None,
        None,
        Some("#3366ff"),
    ];

    let midi_clip_id = demo_id("K1");
    let audio_clip_id = demo_id("K2");

    // 六个音符的身份与 `scene::NOTE_ULIDS` 逐字一致。
    let note_ids: [EntityId; 6] = [
        "01J8Z5Q0R7K3M9X2V4B6N8P1A2",
        "01J8Z5Q0R7K3M9X2V4B6N8P1A3",
        "01J8Z5Q0R7K3M9X2V4B6N8P1B0",
        "01J8Z5Q0R7K3M9X2V4B6N8P1C7",
        "01J8Z5Q0R7K3M9X2V4B6N8P1D4",
        "01J8Z5Q0R7K3M9X2V4B6N8P1E1",
    ]
    .map(|text| EntityId::from_str(text).expect("演示音符 ULID 必须合法"));

    let mut notes: BTreeMap<EntityId, MidiNote> = BTreeMap::new();
    for (index, note_id) in note_ids.into_iter().enumerate() {
        let pitch = [60_u8, 64, 67, 72, 74, 76][index];
        let start = 480 * u64::try_from(index).unwrap_or(0);
        notes.insert(note_id, MidiNote::new(note_id, start, pitch, 480));
    }

    let mut tracks: BTreeMap<EntityId, TrackV3> = BTreeMap::new();
    tracks.insert(master_id, master_track(master_id));
    for (slot, track_id) in track_ids.into_iter().enumerate() {
        tracks.insert(
            track_id,
            TrackV3 {
                id: track_id,
                name: track_names[slot].to_owned(),
                kind: track_kinds[slot],
                volume_db: track_volumes[slot],
                pan: 0.0,
                mute: slot == 5,
                solo: slot == 0,
                solo_safe: false,
                folder_id: None,
                color: track_colors[slot].map(str::to_owned),
                devices: demo_track_devices(slot),
                macros: Vec::new(),
                automation_lanes: demo_automation_lanes(slot, track_id),
                clips: BTreeMap::new(),
            },
        );
    }

    // 三个剪辑摆放：身份与 `scene::CLIP_ULIDS` 逐字一致；一个 MIDI、两个音频。
    let placements: [(EntityId, EntityId, usize, u64, u64); 3] = [
        (
            EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8P1F9").expect("剪辑 ULID 必须合法"),
            midi_clip_id,
            0,
            0,
            3840,
        ),
        (
            EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8P1G6").expect("剪辑 ULID 必须合法"),
            audio_clip_id,
            1,
            1920,
            960,
        ),
        (
            EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8P1H3").expect("剪辑 ULID 必须合法"),
            audio_clip_id,
            2,
            5760,
            3840,
        ),
    ];
    for (placement_id, clip_id, slot, start_tick, duration_ticks) in placements {
        if let Some(track) = tracks.get_mut(&track_ids[slot]) {
            track.clips.insert(
                placement_id,
                ClipPlacement {
                    id: placement_id,
                    clip_id,
                    start_tick,
                    duration_ticks,
                    loop_config: LoopConfig::default(),
                    muted: false,
                },
            );
        }
    }

    // 四个段落 / 四个场景，与 `scene::SECTION_NAMES` / `scene::SCENE_NAMES` 逐字一致。
    let mut sections: BTreeMap<EntityId, SectionV3> = BTreeMap::new();
    for (slot, name) in ["Intro", "Verse", "Chorus", "Outro"]
        .into_iter()
        .enumerate()
    {
        let id = demo_id(["S1", "S2", "S3", "S4"][slot]);
        let start = 7680 * u64::try_from(slot).unwrap_or(0);
        sections.insert(
            id,
            SectionV3 {
                id,
                name: name.to_owned(),
                start_tick: start,
                end_tick: start + 7680,
                // `if` 而不是 `bool::then(..)`：后者会被 `clippy::unnecessary_lazy_evaluations`
                // 盯上（本仓库在 test_port_adapter.rs 里已踩过一次同类）。
                color: if slot == 0 {
                    Some("#22AA88".to_owned())
                } else {
                    None
                },
            },
        );
    }
    let mut scenes: BTreeMap<EntityId, SceneV3> = BTreeMap::new();
    for (slot, name) in ["Intro", "Verse", "Chorus", "Drop"].into_iter().enumerate() {
        let id = demo_id(["C1", "C2", "C3", "C4"][slot]);
        scenes.insert(
            id,
            SceneV3 {
                id,
                name: name.to_owned(),
                tempo: None,
                color: None,
            },
        );
    }

    // 路由：每条轨道 → 主总线（`RoutingGraph` 是声学连接的唯一真理源，红线见 MODEL-AST-004）。
    let mut edges: BTreeMap<EntityId, RoutingEdge> = BTreeMap::new();
    for (slot, track_id) in track_ids.into_iter().enumerate() {
        let edge_id = demo_id(["R1", "R2", "R3", "R4", "R5", "R6"][slot]);
        edges.insert(
            edge_id,
            RoutingEdge {
                id: edge_id,
                source_node: track_id,
                destination_node: master_id,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
    }
    let mut nodes = vec![master_id];
    nodes.extend(track_ids);

    let mut clip_pool: BTreeMap<EntityId, ClipPoolEntry> = BTreeMap::new();
    clip_pool.insert(
        midi_clip_id,
        ClipPoolEntry {
            id: midi_clip_id,
            name: "夜色铺底".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );
    clip_pool.insert(
        audio_clip_id,
        ClipPoolEntry {
            id: audio_clip_id,
            name: "909 鼓组".to_owned(),
            content: ClipContent::Audio {
                asset: crate::ids::AssetHash::of_bytes(b"yeban-demo-kick"),
                gain_db: -1.5,
            },
        },
    );

    YebanProjectV1 {
        title: "夜半 Yeban".to_owned(),
        author: "Yeban Project Contributors".to_owned(),
        bpm: 120.0,
        id: demo_id("P0"),
        tracks,
        master_bus_track_id: master_id,
        routing_graph: RoutingGraph { nodes, edges },
        sections,
        scenes,
        clip_pool,
        ..YebanProjectV1::default()
    }
}

/// 演示工程的实体身份：`01J8Z5Q0R7K3M9X2V4B6N8P0` + 2 字符尾段。
///
/// 前缀与 `scene.rs` 里的演示 ULID 常量同族（`…N8Pxx`），尾段用另一段命名空间
/// （前导 `0`）以免与音符 / 剪辑常量相撞。
///
/// # Panics
///
/// 尾段不是合法 Crockford Base32 时 panic（常量错误属编程错误）。
#[must_use]
pub fn demo_id(tail: &str) -> EntityId {
    use std::str::FromStr as _;
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P0{tail}"))
        .expect("演示夹具的 ULID 必须是合法 Crockford Base32")
}

/// 演示夹具的一个自动化采样点（身份与值一起给出，避免键/身份漂移）。
pub fn demo_point(
    tail: &str,
    tick: u64,
    value: f32,
    curve: crate::music::CurveType,
) -> (EntityId, crate::project::AutomationPoint) {
    let id = demo_id(tail);
    (
        id,
        crate::project::AutomationPoint {
            id,
            tick,
            value,
            curve,
        },
    )
}

/// 演示夹具的**设备链**：只有轨道 0（`鼓`）挂一个内部合成器。
///
/// 它存在的理由是**一条泳道的值域**：`AutomationTarget::DeviceParam` 是模型里唯一
/// "固有取值域不可知"（`nominal_domain() == None`）的目标，因此只有它能让"按曲线最值
/// 自适应纵轴"这条路径被真正执行到（见 `crate::automation` 的模块文档与判据 ③）。
#[must_use]
pub fn demo_track_devices(slot: usize) -> Vec<crate::project::DeviceDefinition> {
    if slot != 0 {
        return Vec::new();
    }
    vec![crate::project::DeviceDefinition {
        id: demo_id("D1"),
        name: "Yeban PolySynth".to_owned(),
        kind: crate::project::DeviceKind::InternalInstrument,
        bypassed: false,
        params: vec![crate::project::ParameterValue {
            name: "cutoff".to_owned(),
            value: 1200.0,
            unit: Some("Hz".to_owned()),
        }],
        latency_samples: 0,
    }]
}

/// 演示夹具的**自动化泳道**（`line/app-automation-ui` 补的那一格）。
///
/// 三条泳道刻意覆盖三种不同的目标形状，让"人工看一下"也能分辨它们：
///
/// | 轨道 | 目标 | 单位 | 取值域 | 读 / 写 | 覆盖的口径 |
/// | :--- | :--- | :--- | :--- | :--- | :--- |
/// | 0（`鼓`） | `TrackVolume` | `dB` | **固有** `[-60, 12]` | 读开 / `Touch` | 轴来自目标、录制臂角标 |
/// | 0（`鼓`） | `DeviceParam(0, 0)` | `Native` | **自适应** `[200, 4000]` | **读关** / `Off` | 值域自适应 + 读关闭可区分 + 同轨多泳道等分 |
/// | 1（`贝斯`） | `TrackPan` | `Bipolar` | 固有 `[-1, 1]` | 读开 / `Write` | 双极单位、另一条轨道 |
///
/// 它们**不是**界面常量：`YebanProjectV1` 是唯一事实源，界面只读投影
/// （`demo_projection_reproduces_the_scene_constants` 钉住这一点）。
#[must_use]
pub fn demo_automation_lanes(
    slot: usize,
    track_id: EntityId,
) -> std::collections::BTreeMap<crate::project::AutomationTarget, crate::project::AutomationLane> {
    use crate::music::CurveType;
    use crate::project::{AutomationLane, AutomationPoint, AutomationTarget, AutomationWriteMode};

    let mut lanes = std::collections::BTreeMap::new();
    let mut insert = |target: AutomationTarget,
                      points: Vec<(EntityId, AutomationPoint)>,
                      read_enabled: bool,
                      write_mode: AutomationWriteMode| {
        lanes.insert(
            target,
            AutomationLane {
                target,
                points: points.into_iter().collect(),
                read_enabled,
                write_mode,
                domain: None,
            },
        );
    };
    match slot {
        0 => {
            insert(
                AutomationTarget::TrackVolume { track_id },
                vec![
                    demo_point("A1", 0, -3.2, CurveType::Linear),
                    demo_point("A2", 1920, -8.0, CurveType::Logarithmic),
                    demo_point("A3", 3840, -1.0, CurveType::Linear),
                ],
                true,
                AutomationWriteMode::Touch,
            );
            insert(
                AutomationTarget::DeviceParam {
                    track_id,
                    slot_index: 0,
                    param_index: 0,
                },
                vec![
                    demo_point("A4", 0, 200.0, CurveType::Exponential),
                    demo_point("A5", 960, 1200.0, CurveType::Linear),
                    demo_point("A6", 3840, 4000.0, CurveType::Linear),
                ],
                false,
                AutomationWriteMode::Off,
            );
        }
        1 => {
            insert(
                AutomationTarget::TrackPan { track_id },
                vec![
                    demo_point("A7", 0, -1.0, CurveType::Linear),
                    demo_point("A8", 960, 0.0, CurveType::SCurve),
                    demo_point("A9", 2880, 1.0, CurveType::Linear),
                ],
                true,
                AutomationWriteMode::Write,
            );
        }
        _ => {}
    }
    lanes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本 crate 的 `schemas/` 目录（`<crate>/../../schemas`）。
    fn schema_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("schemas")
            .join(name)
    }

    /// 读取 `schemas/ops.schema.json` 里 `op.oneOf[*].required[0]` 的键名集合。
    fn schema_op_variant_names() -> Vec<String> {
        let text = std::fs::read_to_string(schema_path("ops.schema.json")).expect("read schema");
        let schema: serde_json::Value = serde_json::from_str(&text).expect("schema json");
        schema["properties"]["op"]["oneOf"]
            .as_array()
            .expect("op.oneOf 必须是数组")
            .iter()
            .map(|branch| {
                branch["required"][0]
                    .as_str()
                    .expect("每个分支必须 required 一个变体键")
                    .to_owned()
            })
            .collect()
    }

    /// 读取 `schemas/ops.schema.json` 里 `origin.oneOf[0].enum` 的单元变体名。
    fn schema_origin_unit_names() -> Vec<String> {
        let text = std::fs::read_to_string(schema_path("ops.schema.json")).expect("read schema");
        let schema: serde_json::Value = serde_json::from_str(&text).expect("schema json");
        schema["properties"]["origin"]["oneOf"][0]["enum"]
            .as_array()
            .expect("origin.oneOf[0].enum 必须是数组")
            .iter()
            .map(|name| name.as_str().expect("enum 元素是字符串").to_owned())
            .collect()
    }

    #[test]
    fn default_project_sample_is_legal() {
        let project = default_project();
        assert_eq!(project.validate(), Ok(()));
        assert_eq!(project.check_readable(), Ok(()));
        assert_eq!(project.schema_version, SCHEMA_VERSION);
        assert_eq!(project.writer_version, DEFAULT_WRITER_VERSION);
        assert_eq!(project.min_reader_version, MIN_READER_VERSION);
    }

    #[test]
    fn filled_project_sample_is_legal_and_rich() {
        let project = filled_project();
        assert_eq!(project.validate(), Ok(()), "{:?}", project.validate());
        assert_eq!(project.check_readable(), Ok(()));
        assert_eq!(project.tracks.len(), 4);
        assert_eq!(project.clip_pool.len(), 2);
        assert_eq!(project.routing_graph.edges.len(), 3);
        assert_eq!(project.sections.len(), 2);
        assert_eq!(project.scenes.len(), 1);
        assert_eq!(project.assets.len(), 1);
        // 样本必须覆盖表现力字段（它们落在不同音符上，因此分开断言）。
        let notes: Vec<&MidiNote> = project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .flat_map(BTreeMap::values)
            .collect();
        assert!(!notes.is_empty(), "填充样本必须含 MIDI 音符");
        for field in ["probability", "ratchet", "micro_timing_ticks", "syllable"] {
            let present = notes.iter().any(|note| match field {
                "probability" => note.probability.is_some(),
                "ratchet" => note.ratchet.is_some(),
                "micro_timing_ticks" => note.micro_timing_ticks.is_some(),
                _ => note.syllable.is_some(),
            });
            assert!(present, "填充样本必须覆盖 `{field}`");
        }
    }

    #[test]
    fn project_with_notes_has_exactly_the_requested_count() {
        // 判据: **恰好** 10 万个音符, 不是"大约"。
        let project = project_with_notes(100_000);
        let total: usize = project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .map(std::collections::BTreeMap::len)
            .sum();
        assert_eq!(total, 100_000, "音符总数必须恰好等于请求值");
        // 跨度断言：铺在长时轴上（否则裁剪无从谈起, 见函数文档）。
        let span_ticks: u64 = 100_000 * 240;
        let max_start = project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .flat_map(std::collections::BTreeMap::values)
            .map(|note| note.start_tick)
            .max()
            .expect("音符");
        assert!(
            max_start > span_ticks / 2,
            "音符必须铺在长时轴上: 最大 start_tick={max_start} 应超过 {}",
            span_ticks / 2
        );
        // 两次调用的音符数一致（id 由本机生成 ⇒ 不谎称逐字节可复现, 见函数文档）。
        let again: usize = project_with_notes(100_000)
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .map(std::collections::BTreeMap::len)
            .sum();
        assert_eq!(again, 100_000);
    }

    #[test]
    fn default_ops_sample_applies_to_the_default_project() {
        let mut project = default_project();
        default_stamped_op()
            .apply(&mut project)
            .expect("默认 op 必须能作用在默认工程上");
        assert_eq!(project.validate(), Ok(()));
        assert_eq!(project.sections.len(), 1);
    }

    #[test]
    fn filled_ops_sample_applies_to_the_filled_project() {
        let mut project = filled_project();
        filled_stamped_op()
            .apply(&mut project)
            .expect("填充 op 必须能作用在填充工程上");
        assert_eq!(project.validate(), Ok(()));
        assert_eq!(
            project.routing_graph.edges.len(),
            4,
            "样例里的 ConnectRouting 必须生效"
        );
        // 撤销必须精确还原：样本对本身就是一组可逆的领域操作。
        filled_stamped_op()
            .apply_inverse(&mut project)
            .expect("填充 op 必须可逆");
        assert_eq!(project, filled_project());
    }

    #[test]
    fn ops_samples_match_the_ops_contract_variant_list() {
        let contract: Vec<String> = schema_op_variant_names();
        assert_eq!(
            contract.len(),
            29,
            "ops.schema.json 的 op.oneOf 必须覆盖 29 个变体, 实际 {}",
            contract.len()
        );
        for stamped in [default_stamped_op(), filled_stamped_op()] {
            check_op_envelope(&stamped).expect("op 信封必须合法");
            let key = stamped.op.name().to_owned();
            assert!(
                contract.contains(&key),
                "样本里的 `{key}` 不在 ops.schema.json 的变体清单中"
            );
        }
    }

    #[test]
    fn ops_origin_shapes_match_the_origin_one_of() {
        let unit_names = schema_origin_unit_names();
        assert_eq!(unit_names.len(), 6);

        // 单元变体：纯字符串，且必须出现在契约的 enum 里。
        let default_origin =
            serde_json::to_value(&default_stamped_op().origin).expect("serialize origin");
        let as_str = default_origin.as_str().expect("单元变体必须序列化为字符串");
        assert!(unit_names.contains(&as_str.to_owned()));

        // McpProposal：外部标签对象，且**只**带 McpProposal 一个键。
        let origin = serde_json::to_value(&filled_stamped_op().origin).expect("serialize origin");
        let object = origin.as_object().expect("McpProposal 必须是对象");
        assert_eq!(object.len(), 1);
        let proposal = object
            .get("McpProposal")
            .and_then(serde_json::Value::as_object)
            .expect("McpProposal 是对象");
        for key in ["proposal_id", "agent_name"] {
            assert!(proposal.contains_key(key), "McpProposal 缺键 {key}");
        }
    }

    #[test]
    fn every_unit_origin_variant_is_in_the_contract_enum() {
        let contract = schema_origin_unit_names();
        for origin in [
            OpOrigin::UserUi,
            OpOrigin::MidiInput,
            OpOrigin::UndoRedo,
            OpOrigin::AutomationRecord,
            OpOrigin::Import,
            OpOrigin::Migration,
        ] {
            let value = serde_json::to_value(&origin).expect("serialize origin");
            let name = value.as_str().expect("单元变体必须是字符串").to_owned();
            assert!(contract.contains(&name), "`{name}` 不在契约 enum 中");
            let back: OpOrigin = serde_json::from_value(value).expect("deserialize origin");
            assert_eq!(back, origin);
        }
    }

    #[test]
    fn export_all_writes_four_byte_stable_samples() {
        let dir = std::env::temp_dir().join("yeban-model-schema-samples-test");
        let written = export_all(&dir).expect("导出样本");
        assert_eq!(written.len(), 4, "必须写出 4 份样本: {written:?}");
        let names: Vec<String> = written
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec![
                PROJECT_DEFAULT_FILE.to_owned(),
                PROJECT_FILLED_FILE.to_owned(),
                OPS_DEFAULT_FILE.to_owned(),
                OPS_FILLED_FILE.to_owned(),
            ]
        );

        let before: Vec<String> = written
            .iter()
            .map(|path| std::fs::read_to_string(path).expect("read sample"))
            .collect();
        let again = export_all(&dir).expect("再次导出样本");
        let after: Vec<String> = again
            .iter()
            .map(|path| std::fs::read_to_string(path).expect("read sample"))
            .collect();
        assert_eq!(before, after, "样本导出必须逐字节稳定");

        // 每份样本都必须以换行结尾（便于 git diff 与文本工具）。
        for content in &before {
            assert!(content.ends_with('\n'));
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn default_out_dir_is_the_workspace_target() {
        let dir = default_out_dir();
        assert!(
            dir.ends_with(SAMPLES_DIR_NAME),
            "默认样本目录必须以 {SAMPLES_DIR_NAME} 结尾: {}",
            dir.display()
        );
    }
}
