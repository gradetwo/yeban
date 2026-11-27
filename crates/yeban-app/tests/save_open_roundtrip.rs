//! **T-FIX-942 判据**：含多轨、自动化、效果链的工程，走公开保存入口落盘后重开，
//! **逐字段**比对。
//!
//! 这条判据刻意不复用 `yeban-model` 的容器往返判据：容器层 `read == project` 只能
//! 证明"模型自己写自己读"；本判据要证明的是 **app 的两条公开入口**
//! `save::save_project_file` → `open::open_project_file` 接起来之后，磁盘上的字节
//! 再回到内存时没有字段被丢、被改写、被排序漂移。
//!
//! ## 比对口径
//!
//! 1. 先按 `YebanProjectV1` 的 `PartialEq` 做整份相等（最严格，含 `f32` 逐位）；
//! 2. 再把两份工程 serde 成 JSON，**扁平化到每一个叶子**（对象键、数组下标都进
//!    JSON Pointer），逐 key 比对。`Eq` 相等时第 2 步仍会打印比对叶子数 ——
//!    防止"判据只比了顶层三个字段"的空心通过。
//!
//! 任何差异都会列出全部 JSON Pointer，不只报第一个；这正是"发现不一致逐条立
//! issue 并附最小复现"的机械底座。

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::str::FromStr;

use serde_json::Value;
use yeban_app::open::{ProjectOpenOptions, open_project_archive_file};
use yeban_app::save::save_archive_file;
use yeban_model::CurveType;
use yeban_model::container::ProjectArchive;
use yeban_model::ids::{AssetHash, EntityId};
use yeban_model::project::{
    AutomationLane, AutomationPoint, AutomationTarget, AutomationValueDomain, AutomationWriteMode,
    ClipContent, DeviceDefinition, DeviceKind, ParameterValue, PluginBinding, TrackKind,
    YebanProjectV1,
};

fn id(index: u128) -> EntityId {
    EntityId::from_str(&format!("01J8ZQ{index:020}")).expect("canonical fixture ulid")
}

/// 把 JSON 扁平化成 `pointer -> 叶子值`；数组下标也进 pointer，顺序变化同样可见。
fn flatten(value: &Value, pointer: &str, out: &mut BTreeMap<String, Value>) {
    match value {
        Value::Object(object) => {
            if object.is_empty() {
                out.insert(pointer.to_owned(), value.clone());
            } else {
                for (key, child) in object {
                    let escaped = key.replace('~', "~0").replace('/', "~1");
                    let child_pointer = format!("{pointer}/{escaped}");
                    flatten(child, &child_pointer, out);
                }
            }
        }
        Value::Array(items) => {
            if items.is_empty() {
                out.insert(pointer.to_owned(), value.clone());
            } else {
                for (index, child) in items.iter().enumerate() {
                    flatten(child, &format!("{pointer}/{index}"), out);
                }
            }
        }
        _ => {
            out.insert(pointer.to_owned(), value.clone());
        }
    }
}

/// 逐字段差异：missing（只在期望里有）、extra（只在实测里有）、changed。
#[derive(Debug, Default)]
struct FieldDiff {
    missing: Vec<String>,
    extra: Vec<String>,
    changed: Vec<(String, Value, Value)>,
}

impl FieldDiff {
    fn is_empty(&self) -> bool {
        self.missing.is_empty() && self.extra.is_empty() && self.changed.is_empty()
    }

    fn len(&self) -> usize {
        self.missing.len() + self.extra.len() + self.changed.len()
    }
}

fn compare_fields(expected: &YebanProjectV1, actual: &YebanProjectV1) -> (usize, FieldDiff) {
    let expected = serde_json::to_value(expected).expect("期望工程 serde");
    let actual = serde_json::to_value(actual).expect("实测工程 serde");
    let mut expected_leaves = BTreeMap::new();
    let mut actual_leaves = BTreeMap::new();
    flatten(&expected, "", &mut expected_leaves);
    flatten(&actual, "", &mut actual_leaves);

    let mut diff = FieldDiff::default();
    for (pointer, expected_value) in &expected_leaves {
        match actual_leaves.get(pointer) {
            None => diff.missing.push(pointer.clone()),
            Some(actual_value) if actual_value != expected_value => diff.changed.push((
                pointer.clone(),
                expected_value.clone(),
                actual_value.clone(),
            )),
            Some(_) => {}
        }
    }
    for pointer in actual_leaves.keys() {
        if !expected_leaves.contains_key(pointer) {
            diff.extra.push(pointer.clone());
        }
    }
    (expected_leaves.len(), diff)
}

fn temp_project_path(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-t942-{}-{}-{tag}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir.join("roundtrip.yeban")
}

fn auto_point(index: u128, tick: u64, value: f32, curve: CurveType) -> AutomationPoint {
    AutomationPoint {
        id: id(index),
        tick,
        value,
        curve,
    }
}

fn effect(index: u128, name: &str, bypassed: bool, plugin_state: &[u8]) -> DeviceDefinition {
    DeviceDefinition {
        id: id(index),
        name: name.to_owned(),
        kind: DeviceKind::ExternalEffect,
        bypassed,
        params: vec![
            ParameterValue {
                name: "mix".to_owned(),
                value: 0.375,
                unit: Some("%".to_owned()),
            },
            ParameterValue {
                name: "drive".to_owned(),
                value: -6.5,
                unit: Some("dB".to_owned()),
            },
        ],
        latency_samples: 64,
        plugin: Some(PluginBinding {
            path: "stub:t942/external-effect.vst3".to_owned(),
            class: Some("T942ExternalEffect".to_owned()),
            state: plugin_state.to_vec(),
        }),
    }
}

/// 构造任务书点名的"多轨 + 自动化 + 效果链"工程；样本本身已有多条 MIDI / 音频 /
/// 辅助 / 主总线轨与自动化泳道，这里再给首条 MIDI 轨挂两格外部效果（其中一格
/// 旁通、两格都带插件状态字节），把效果链字段也纳入往返。
fn rich_project() -> (YebanProjectV1, EntityId) {
    let mut project = yeban_model::samples::filled_project();
    assert!(project.tracks.len() >= 3, "样本必须多轨");

    // 样本的音频片段引用的是随包演示素材，字节不在这个模型夹具里；本判据只做
    // 工程文档的字段往返，不测试素材包，因此先把音频片段与对应资产声明剥掉，
    // 保留多轨 / 自动化 / 效果链（任务书点名的三样）。
    let audio_clips: BTreeSet<EntityId> = project
        .clip_pool
        .iter()
        .filter(|(_, clip)| matches!(&clip.content, ClipContent::Audio { .. }))
        .map(|(id, _)| *id)
        .collect();
    project.clip_pool.retain(|id, _| !audio_clips.contains(id));
    let keep_clips: BTreeSet<EntityId> = project.clip_pool.keys().copied().collect();
    for track in project.tracks.values_mut() {
        track
            .clips
            .retain(|_, placement| keep_clips.contains(&placement.clip_id));
    }
    project.assets.clear();

    let track_id = project
        .tracks
        .values()
        .find(|track| track.kind == TrackKind::Midi && !track.clips.is_empty())
        .expect("必须有一条带摆放的 MIDI 轨")
        .id;
    let track = project.tracks.get_mut(&track_id).expect("轨存在");
    track.devices.push(effect(
        900,
        "T942 External Compressor",
        false,
        b"YBPGST01-t942-compressor",
    ));
    track.devices.push(effect(
        901,
        "T942 External Reverb (bypassed)",
        true,
        b"YBPGST01-t942-reverb",
    ));

    // 再补两条目标不同的自动化泳道：声相（读关闭 + 显式值域 + Touch 写模式）、
    // 外部效果参数（读打开、无显式值域）。样本自带的音量泳道已有一条。
    let pan_target = AutomationTarget::TrackPan { track_id };
    track.automation_lanes.insert(
        pan_target,
        AutomationLane {
            target: pan_target,
            points: BTreeMap::from([
                (id(910), auto_point(910, 0, -0.5, CurveType::Linear)),
                (id(911), auto_point(911, 960, 0.75, CurveType::SCurve)),
            ]),
            read_enabled: false,
            write_mode: AutomationWriteMode::Touch,
            domain: Some(AutomationValueDomain::new(-1.0, 1.0).expect("合法域")),
        },
    );
    let device_target = AutomationTarget::DeviceParam {
        track_id,
        slot_index: track.devices.len() - 2,
        param_index: 0,
    };
    track.automation_lanes.insert(
        device_target,
        AutomationLane {
            target: device_target,
            points: BTreeMap::from([(id(912), auto_point(912, 480, 0.5, CurveType::Linear))]),
            read_enabled: true,
            write_mode: AutomationWriteMode::Latch,
            domain: None,
        },
    );

    (project, track_id)
}

#[test]
fn project_save_open_round_trip_is_field_exact_with_tracks_automation_and_effects() {
    let (project, track_id) = rich_project();
    assert_eq!(project.validate(), Ok(()), "夹具必须是合法工程");

    let automation_lanes: usize = project
        .tracks
        .values()
        .map(|track| track.automation_lanes.len())
        .sum();
    let devices: usize = project
        .tracks
        .values()
        .map(|track| track.devices.len())
        .sum();
    let external_effects = project
        .tracks
        .values()
        .flat_map(|track| &track.devices)
        .filter(|device| device.kind == DeviceKind::ExternalEffect)
        .count();
    assert!(automation_lanes > 0, "夹具必须含自动化");
    assert!(devices >= 2, "夹具必须含效果链");
    assert!(external_effects >= 2, "夹具必须含外部效果");

    let path = temp_project_path("field-exact");
    // `filled_project` 含音频片段，`save_project_file` 按 API 契约拒绝；任务书的
    // "保存后重开"因此走全保真归档入口：工程 + history.dag + 资产池一起落盘。
    let history = b"t942-history-dag".to_vec();
    let asset = b"t942-audio-asset".to_vec();
    let asset_hash = AssetHash::of_bytes(&asset);
    let archive = ProjectArchive {
        project: project.clone(),
        history_dag: history.clone(),
        proposals: None,
        assets: vec![(asset_hash.clone(), asset.clone())],
        embedded_presets: Vec::new(),
        load_notes: Vec::new(),
    };
    let report = save_archive_file(&archive, &path).expect("公开保存入口必须成功");
    assert!(report.bytes > 0, "保存必须真的写出字节");

    let opened = open_project_archive_file(&path, &ProjectOpenOptions::default())
        .expect("公开打开入口必须读回刚保存的工程");
    assert_eq!(opened.history_dag, history, "history.dag 字节必须原样保真");
    assert_eq!(
        opened.assets,
        vec![(asset_hash, asset)],
        "资产池与 CAS 键必须原样保真"
    );
    assert_eq!(opened.proposals, None, "未带提案段时不得凭空出现提案");
    assert!(
        opened.embedded_presets.is_empty(),
        "未带内嵌预制时不得凭空出现预制"
    );
    assert!(
        opened.load_notes.is_empty(),
        "装载不得对合法工程做静默规范化: {:?}",
        opened.load_notes
    );
    let reopened = opened.project;
    assert_eq!(
        reopened, project,
        "保存→重开后 `YebanProjectV1` 必须整份相等（含 f32 逐位）"
    );

    let (leaf_count, diff) = compare_fields(&project, &reopened);
    eprintln!(
        "T942 往返比对：{} 轨 / {} 条自动化泳道 / {} 个设备（{} 个外部效果）/ {} 个 JSON 叶子",
        project.tracks.len(),
        automation_lanes,
        devices,
        external_effects,
        leaf_count
    );
    assert!(
        diff.is_empty(),
        "保存→重开必须逐字段相等，实测 {} 处差异：missing={:?} extra={:?} changed={:#?}",
        diff.len(),
        diff.missing,
        diff.extra,
        diff.changed
    );

    // 效果链不是只在"设备个数"上对上：旁通位、延迟、参数与插件状态字节都要逐项回来。
    let before = project.tracks.get(&track_id).expect("原轨");
    let after = reopened.tracks.get(&track_id).expect("重开轨");
    let before_effects: Vec<_> = before
        .devices
        .iter()
        .filter(|device| device.kind == DeviceKind::ExternalEffect)
        .collect();
    let after_effects: Vec<_> = after
        .devices
        .iter()
        .filter(|device| device.kind == DeviceKind::ExternalEffect)
        .collect();
    assert_eq!(
        before_effects.len(),
        after_effects.len(),
        "效果链条数必须一致"
    );
    for (index, (before, after)) in before_effects.iter().zip(&after_effects).enumerate() {
        assert_eq!(before.id, after.id, "效果 {index} 身份");
        assert_eq!(before.name, after.name, "效果 {index} 名称");
        assert_eq!(before.bypassed, after.bypassed, "效果 {index} 旁通位");
        assert_eq!(
            before.latency_samples, after.latency_samples,
            "效果 {index} 处理延迟"
        );
        assert_eq!(before.params, after.params, "效果 {index} 参数");
        assert_eq!(
            before.plugin, after.plugin,
            "效果 {index} 插件绑定/状态字节"
        );
    }

    // 自动化泳道不是只数"非空"：目标集合、点集合与曲线类型都必须在同一份文档里。
    let before_targets: Vec<AutomationTarget> = before.automation_lanes.keys().copied().collect();
    let after_targets: Vec<AutomationTarget> = after.automation_lanes.keys().copied().collect();
    assert_eq!(before_targets, after_targets, "自动化目标集合");
    assert_eq!(
        before.automation_lanes, after.automation_lanes,
        "自动化泳道/点/曲线必须整份一致"
    );

    let _ = std::fs::remove_dir_all(path.parent().expect("临时目录"));
}
