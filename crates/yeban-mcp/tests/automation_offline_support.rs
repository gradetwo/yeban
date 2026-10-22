//! 任务单 `#291`（B）：`yeban_edit_automation` 读侧的**离线渲染落点**读数。
//!
//! ## 本文件钉住的事实
//!
//! 写侧新增两个字段（`data.appliedInOfflineRender` / `data.residualReason`），它们
//! **不是**本文件或 MCP 里的另一张表：判定来自
//! `yeban_render::automation::unsupported_lane_reason`（与 `yeban_render_master` 的
//! `AutomationReport` 同一份实现），探针来自同一份 `RenderPlan` 剪枝结果。
//!
//! 判据（与任务单 §B 的四条对应）：
//!
//! | 判据 | 测试 |
//! | :--- | :--- |
//! | 写 `TrackVolume` ⇒ `appliedInOfflineRender:true`，导出 `unsupported` 不含 `automationLanes` | [`a_track_volume_point_is_reported_as_applied_and_the_export_agrees`] |
//! | 写 `Macro` ⇒ `false` ＋ `"macro"`，导出真的登记 `automationLanes` | [`a_macro_point_is_reported_as_not_applied_and_the_export_agrees`] |
//! | 写**主总线** `TrackPan` ⇒ `false` ＋ `"masterPan"`，导出真的登记 | [`master_pan_is_reported_as_not_applied_and_the_export_agrees`] |
//! | 写**总线轨** `TrackVolume` ⇒ `false` ＋ `"busTrack"`（同一个探针的另一支） | [`a_bus_track_volume_point_is_reported_as_not_applied`] |
//! | 写**非通道条** `DeviceParam` ⇒ `false` ＋ `"deviceParamNotChannelStrip"` | [`a_non_channel_strip_device_param_is_reported_as_not_applied`] |
//!
//! 边界：本文件只证"写侧读数"与"导出读数"**同源且一致**，⛔ 不证听感。

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::samples::filled_project;
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, ParameterValue, YebanProjectV1};

/// 临时目录（`Drop` 删掉自己；工作树里不留临时文件）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-291-{tag}-{}-{}",
            std::process::id(),
            EntityId::new()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        Self { dir }
    }

    fn project(&self) -> PathBuf {
        self.dir.join("p.yeban")
    }

    fn wav(&self) -> PathBuf {
        self.dir.join("out.wav")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn dispatcher(project: YebanProjectV1, path: &Path) -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(path.to_path_buf(), project, false)
        .expect("注入规范样本");
    dispatcher
}

fn call(dispatcher: &mut Dispatcher, name: &str, arguments: Value) -> Value {
    let auth = format!("Bearer {}", dispatcher.expected_token().expose());
    let line = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
    assert_eq!(
        outcome.error_code(),
        None,
        "`{name}` 不得产出 JSON-RPC 层错误: {outcome:?}"
    );
    outcome.response.expect("响应").result.expect("result")
}

/// 样本里那条带设备的音轨（`filled_project` 的 lead）。
fn lead_track(project: &YebanProjectV1) -> EntityId {
    project
        .tracks
        .values()
        .find(|track| !track.devices.is_empty())
        .expect("样本里必须有带设备的音轨")
        .id
}

/// 往 `lane` 目标写一个点，返回工具的 `data`。
fn write_point(dispatcher: &mut Dispatcher, track: EntityId, lane: &str, extra: Value) -> Value {
    let mut arguments = json!({
        "trackId": track.to_canonical_string(),
        "lane": lane,
        "point": {"tick": 0, "value": 0.5, "curve": "Linear"},
    });
    if let (Some(map), Some(extra)) = (arguments.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            map.insert(key.clone(), value.clone());
        }
    }
    let response = call(dispatcher, "yeban_edit_automation", arguments);
    assert_eq!(response["status"], "success", "{response}");
    response["data"].clone()
}

fn render(dispatcher: &mut Dispatcher, path: &Path) -> Value {
    let response = call(
        dispatcher,
        "yeban_render_master",
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "path": path.display().to_string(),
        }),
    );
    assert_eq!(response["status"], "success", "{response}");
    response["data"].clone()
}

fn unsupported_has_automation_lanes(rendered: &Value) -> bool {
    rendered["unsupported"]
        .as_array()
        .expect("unsupported 必须是数组")
        .iter()
        .any(|value| value == "automationLanes")
}

/// ⭐ 判据 1：写 `TrackVolume` ⇒ 写侧说 `true`，导出**不**登记 `automationLanes`。
#[test]
fn a_track_volume_point_is_reported_as_applied_and_the_export_agrees() {
    let scratch = Scratch::new("volume");
    let project = filled_project();
    let lead = lead_track(&project);
    let mut dispatcher = dispatcher(project, &scratch.project());

    let data = write_point(&mut dispatcher, lead, "TrackVolume", json!({}));
    assert_eq!(
        data["appliedInOfflineRender"], true,
        "TrackVolume 在源节点上真的有落点: {data}"
    );
    assert_eq!(data["residualReason"], Value::Null, "{data}");
    assert_eq!(data["offlineRenderError"], Value::Null, "{data}");

    let rendered = render(&mut dispatcher, &scratch.wav());
    assert!(
        !unsupported_has_automation_lanes(&rendered),
        "被应用的 TrackVolume 泳道不许再报 automationLanes: {rendered}"
    );
    assert_eq!(rendered["automation"]["appliedLanes"], 1, "{rendered}");
    assert_eq!(rendered["automation"]["unsupportedLanes"], 0, "{rendered}");
}

/// ⭐ 判据 2：写 `Macro` ⇒ 写侧说 `false` ＋ `"macro"`，导出**真的**登记。
#[test]
fn a_macro_point_is_reported_as_not_applied_and_the_export_agrees() {
    let scratch = Scratch::new("macro");
    let project = filled_project();
    let lead = lead_track(&project);
    let mut dispatcher = dispatcher(project, &scratch.project());

    let data = write_point(&mut dispatcher, lead, "Macro", json!({"macroIndex": 0}));
    assert_eq!(data["appliedInOfflineRender"], false, "{data}");
    assert_eq!(data["residualReason"], "macro", "{data}");

    let rendered = render(&mut dispatcher, &scratch.wav());
    assert!(
        unsupported_has_automation_lanes(&rendered),
        "写侧说 Macro 不应用 ⇒ 导出必须真的登记: {rendered}"
    );
    assert_eq!(rendered["automation"]["unsupportedLanes"], 1, "{rendered}");
    let lanes = rendered["automation"]["tracks"][0]["lanes"]
        .as_array()
        .expect("lanes");
    let macro_lane = lanes
        .iter()
        .find(|lane| lane["target"] == "Macro")
        .expect("导出读数里必须有 Macro 泳道");
    assert_eq!(macro_lane["supported"], false, "{macro_lane}");
    assert_eq!(macro_lane["applied"], false, "{macro_lane}");
}

/// ⭐ 判据 3：写**主总线** `TrackPan` ⇒ 写侧说 `false` ＋ `"masterPan"`，导出真的登记。
#[test]
fn master_pan_is_reported_as_not_applied_and_the_export_agrees() {
    let scratch = Scratch::new("master-pan");
    let project = filled_project();
    let master = project.master_bus_track_id;
    let mut dispatcher = dispatcher(project, &scratch.project());

    let data = write_point(&mut dispatcher, master, "TrackPan", json!({}));
    assert_eq!(data["appliedInOfflineRender"], false, "{data}");
    assert_eq!(data["residualReason"], "masterPan", "{data}");

    let rendered = render(&mut dispatcher, &scratch.wav());
    assert!(
        unsupported_has_automation_lanes(&rendered),
        "写侧说主总线声相不应用 ⇒ 导出必须真的登记: {rendered}"
    );
    let lanes = rendered["automation"]["tracks"][0]["lanes"]
        .as_array()
        .expect("lanes");
    let pan_lane = lanes
        .iter()
        .find(|lane| lane["target"] == "TrackPan")
        .expect("导出读数里必须有主总线 TrackPan 泳道");
    assert_eq!(pan_lane["supported"], false, "{pan_lane}");
}

/// ⭐ 同一个探针的另一支：**总线轨**（Aux 有入边 ⇒ 不是源节点）的 `TrackVolume`
/// 也没有落点 —— 写侧与导出同样一致。
#[test]
fn a_bus_track_volume_point_is_reported_as_not_applied() {
    let scratch = Scratch::new("bus-volume");
    let project = filled_project();
    let bus = project
        .tracks
        .values()
        .find(|track| track.kind == yeban_model::TrackKind::AuxReturn)
        .expect("样本里必须有 Aux 轨")
        .id;
    let mut dispatcher = dispatcher(project, &scratch.project());

    let data = write_point(&mut dispatcher, bus, "TrackVolume", json!({}));
    assert_eq!(data["appliedInOfflineRender"], false, "{data}");
    assert_eq!(data["residualReason"], "busTrack", "{data}");

    let rendered = render(&mut dispatcher, &scratch.wav());
    assert!(
        unsupported_has_automation_lanes(&rendered),
        "总线轨的音量没有源落点 ⇒ 导出必须真的登记: {rendered}"
    );
}

/// ⭐ 非通道条参数（样本 lead 的 `cutoff`）⇒ `"deviceParamNotChannelStrip"`。
#[test]
fn a_non_channel_strip_device_param_is_reported_as_not_applied() {
    let scratch = Scratch::new("device-param");
    let project = filled_project();
    let lead = lead_track(&project);
    let mut dispatcher = dispatcher(project, &scratch.project());

    let data = write_point(
        &mut dispatcher,
        lead,
        "DeviceParam",
        json!({"slotIndex": 0, "paramIndex": 0}),
    );
    assert_eq!(data["appliedInOfflineRender"], false, "{data}");
    assert_eq!(
        data["residualReason"], "deviceParamNotChannelStrip",
        "{data}"
    );

    let rendered = render(&mut dispatcher, &scratch.wav());
    assert!(
        unsupported_has_automation_lanes(&rendered),
        "非通道条设备参数没有落点 ⇒ 导出必须真的登记: {rendered}"
    );
}

/// 给一条轨追加一台内置效果器，返回它的 `slot_index`。
fn append_effect(
    project: &mut YebanProjectV1,
    track_id: EntityId,
    name: &str,
    params: &[(&str, f32)],
) -> usize {
    let track = project.tracks.get_mut(&track_id).expect("目标音轨存在");
    let slot = track.devices.len();
    track.devices.push(DeviceDefinition {
        id: EntityId::new(),
        name: name.to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: params
            .iter()
            .map(|(name, value)| ParameterValue {
                name: (*name).to_owned(),
                value: *value,
                unit: None,
            })
            .collect(),
        latency_samples: 0,
    });
    slot
}

/// ⭐ `#293` 判据 4（`residualReason` 同步）：混响泳道**不再**报
/// `deviceParamNotChannelStrip`；同一份工程里压缩器与卷积 IR 形状**继续**报同一个值
/// （它们仍是“插入链没有这个落点”）。
#[test]
fn a_reverb_lane_lands_while_compressor_and_ir_decay_keep_the_residual() {
    let scratch = Scratch::new("residual-sync");
    let mut project = filled_project();
    let lead = lead_track(&project);
    let reverb_slot = append_effect(&mut project, lead, "Reverb", &[("reverb_wet", 0.0)]);
    let compressor_slot =
        append_effect(&mut project, lead, "Compressor", &[("threshold_db", -20.0)]);
    let conv_slot = append_effect(
        &mut project,
        lead,
        "Convolution",
        &[("conv_ir_decay_s", 0.35)],
    );
    let mut dispatcher = dispatcher(project, &scratch.project());

    let reverb = write_point(
        &mut dispatcher,
        lead,
        "DeviceParam",
        json!({"slotIndex": reverb_slot, "paramIndex": 0}),
    );
    assert_eq!(reverb["appliedInOfflineRender"], true, "{reverb}");
    assert_eq!(reverb["residualReason"], Value::Null, "{reverb}");

    let compressor = write_point(
        &mut dispatcher,
        lead,
        "DeviceParam",
        json!({"slotIndex": compressor_slot, "paramIndex": 0}),
    );
    assert_eq!(compressor["appliedInOfflineRender"], false, "{compressor}");
    assert_eq!(
        compressor["residualReason"], "deviceParamNotChannelStrip",
        "{compressor}"
    );

    let convolution = write_point(
        &mut dispatcher,
        lead,
        "DeviceParam",
        json!({"slotIndex": conv_slot, "paramIndex": 0}),
    );
    assert_eq!(
        convolution["appliedInOfflineRender"], false,
        "{convolution}"
    );
    assert_eq!(
        convolution["residualReason"], "deviceParamNotChannelStrip",
        "{convolution}"
    );
    println!(
        "### #293 residualReason: reverb={} compressor={} conv_ir_decay={}",
        reverb["residualReason"], compressor["residualReason"], convolution["residualReason"]
    );

    let rendered = render(&mut dispatcher, &scratch.wav());
    assert!(
        unsupported_has_automation_lanes(&rendered),
        "混响泳道已落点，但压缩/IR 形状仍不可落点 ⇒ 导出仍应登记 automationLanes: {rendered}"
    );
}
