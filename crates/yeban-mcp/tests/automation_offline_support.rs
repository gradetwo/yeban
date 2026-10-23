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
//! | 写**主总线** `TrackPan` ⇒ `true`、无残差，导出**不**登记（`#296` 起有声相落点） | [`master_pan_is_reported_as_applied_and_the_export_agrees`] |
//! | 写**不可达总线轨** `TrackVolume` ⇒ `false` ＋ `"busTrack"`（`#296` 正对照：可达总线已转 supported） | [`an_unreachable_bus_track_volume_point_is_still_reported_as_not_applied`] |
//! | 写**不可达总线轨** `TrackPan` ⇒ `false` ＋ `"busTrack"`（B2 正对照：可达总线已转 supported） | [`an_unreachable_bus_track_pan_point_is_still_reported_as_not_applied`] |
//! | 写**非通道条** `DeviceParam` ⇒ `false` ＋ `"deviceParamNotChannelStrip"` | [`a_non_channel_strip_device_param_is_reported_as_not_applied`] |
//!
//! 边界：本文件只证"写侧读数"与"导出读数"**同源且一致**，⛔ 不证听感。

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::samples::filled_project;
use yeban_model::{
    AutomationTarget, DeviceDefinition, DeviceKind, EntityId, MacroMapping, ParameterValue,
    YebanProjectV1,
};

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

/// `#297`：把样本轨的宏映射改成 `TrackVolume`、`depth = 6.0`（dB），并清掉目标泳道，
/// 让"宏位置 = 0"的基线就是静态推子。
fn volume_macro_fixture() -> (YebanProjectV1, EntityId) {
    let mut project = filled_project();
    let lead = lead_track(&project);
    let track = project.tracks.get_mut(&lead).expect("lead");
    track.automation_lanes.clear();
    track.macros[0].mappings = vec![MacroMapping {
        target: AutomationTarget::TrackVolume { track_id: lead },
        depth: 6.0,
    }];
    project.validate().expect("宏量纲夹具必须合法");
    (project, lead)
}

/// 往 `Macro` 泳道写一个 tick 0 的点。
fn write_macro_point(dispatcher: &mut Dispatcher, track: EntityId, value: f64) -> Value {
    let response = call(
        dispatcher,
        "yeban_edit_automation",
        json!({
            "trackId": track.to_canonical_string(),
            "lane": "Macro",
            "macroIndex": 0,
            "point": {"tick": 0, "value": value, "curve": "Linear"},
        }),
    );
    assert_eq!(response["status"], "success", "{response}");
    response["data"].clone()
}

fn mono_peak(path: &Path) -> f32 {
    let (samples, _rate, _description) =
        yeban_render::wav::read_plain_wav_mono_f32(path).expect("读回导出 WAV");
    samples
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
}

/// ⭐ `#297` 判据 1/2/3/4（端到端）：`Macro` 泳道按 `静态值 + depth × macro_value`
/// 真的改音频；`macro_value = 0` 与"无宏泳道"逐字节相同；`macro_value = 1` 的读数
/// 落在静态值 + depth 上；写侧与导出侧都不再报 `macro`。
#[test]
fn a_supported_macro_lane_changes_the_export_in_target_units() {
    let scratch = Scratch::new("macro-dimension");
    let (project, lead) = volume_macro_fixture();

    // ① 无宏泳道基线。
    let mut baseline_dispatcher = dispatcher(project.clone(), &scratch.project());
    let baseline_out = scratch.dir.join("baseline.wav");
    let baseline = render(&mut baseline_dispatcher, &baseline_out);

    // ② 宏位置恒 0 ⇒ 正对照（逐字节等于无泳道）。
    let mut zero_dispatcher = dispatcher(project.clone(), &scratch.project());
    let zero_data = write_macro_point(&mut zero_dispatcher, lead, 0.0);
    assert_eq!(zero_data["appliedInOfflineRender"], true, "{zero_data}");
    assert!(zero_data["residualReason"].is_null(), "{zero_data}");
    let zero_out = scratch.dir.join("zero.wav");
    let zero = render(&mut zero_dispatcher, &zero_out);
    assert_eq!(
        zero["sha256"], baseline["sha256"],
        "宏位置恒 0 必须逐字节等于无宏泳道"
    );
    assert_eq!(zero["masterDigest"], baseline["masterDigest"]);

    // ③ 宏位置恒 1 ⇒ 静态推子 −3 dB + 6 dB = +3 dB，真的改音频。
    let mut one_dispatcher = dispatcher(project, &scratch.project());
    let one_data = write_macro_point(&mut one_dispatcher, lead, 1.0);
    assert_eq!(one_data["appliedInOfflineRender"], true, "{one_data}");
    assert!(one_data["residualReason"].is_null(), "{one_data}");
    let one_out = scratch.dir.join("one.wav");
    let one = render(&mut one_dispatcher, &one_out);
    assert_ne!(
        one["sha256"], baseline["sha256"],
        "宏位置 1 必须真的改文件字节"
    );
    assert_ne!(
        one["masterDigest"], baseline["masterDigest"],
        "宏位置 1 必须真的改母带样本"
    );

    // 读数：两端的峰值（+6 dB 相对静态基线）。
    let baseline_peak = mono_peak(&baseline_out);
    let zero_peak = mono_peak(&zero_out);
    let one_peak = mono_peak(&one_out);
    println!(
        "### #297 macro lane peaks: baseline={baseline_peak:.6} zero={zero_peak:.6} one={one_peak:.6} ratio={:.6}",
        one_peak / zero_peak
    );
    assert!(
        (zero_peak - baseline_peak).abs() < 1e-6,
        "正对照峰值必须相同"
    );
    assert!(one_peak > zero_peak * 1.5, "宏位置 1 必须显著更响");

    // ④ 导出读数：Macro 泳道 supported/applied，且不再登记 `automationLanes`。
    let lanes = one["automation"]["tracks"]
        .as_array()
        .expect("tracks")
        .iter()
        .flat_map(|track| track["lanes"].as_array().expect("lanes").iter())
        .collect::<Vec<_>>();
    let macro_lane = lanes
        .iter()
        .find(|lane| lane["target"] == "Macro")
        .expect("导出读数里必须有 Macro 泳道");
    assert_eq!(macro_lane["supported"], true, "{macro_lane}");
    assert_eq!(macro_lane["applied"], true, "{macro_lane}");
    assert!(
        !unsupported_has_automation_lanes(&one),
        "Macro 已被消费 ⇒ `automationLanes` 不该再登记: {one}"
    );
}

/// ⭐ 判据 3（`#296` 更新）：写**主总线** `TrackPan` ⇒ 写侧说 `true`、无残差，
/// 导出读数 `supported:true` 且**不再**登记 `automationLanes`。主总线声相现在
/// 真的有落点（静态 `pan` 走计划的输出级，自动化在执行之后乘绝对左右增益）。
#[test]
fn master_pan_is_reported_as_applied_and_the_export_agrees() {
    let scratch = Scratch::new("master-pan");
    let project = filled_project();
    let master = project.master_bus_track_id;
    let mut dispatcher = dispatcher(project, &scratch.project());

    let data = write_point(&mut dispatcher, master, "TrackPan", json!({}));
    assert_eq!(data["appliedInOfflineRender"], true, "{data}");
    assert!(data["residualReason"].is_null(), "{data}");

    let rendered = render(&mut dispatcher, &scratch.wav());
    assert!(
        !unsupported_has_automation_lanes(&rendered),
        "主总线声相真的被应用 ⇒ 导出不许再登记 automationLanes: {rendered}"
    );
    let lanes = rendered["automation"]["tracks"][0]["lanes"]
        .as_array()
        .expect("lanes");
    let pan_lane = lanes
        .iter()
        .find(|lane| lane["target"] == "TrackPan")
        .expect("导出读数里必须有主总线 TrackPan 泳道");
    assert_eq!(pan_lane["supported"], true, "{pan_lane}");
    assert_eq!(pan_lane["applied"], true, "{pan_lane}");
}

/// ⭐ 同一个探针的另一支（**`#296` 正对照**）：样本里的 Aux 只有 `lead → aux`、
/// **没有 `aux → master`** ⇒ 它到不了母带、被计划剪枝 ⇒ `TrackVolume` 仍**没有**
/// 落点（`busTrack`）。⚠ 有 `aux → master` 的**可达**总线轨已转成 `supported:true`
/// （见 `render_master.rs` 的 `a_reachable_bus_volume_lane_is_applied_at_the_output_stage`）。
#[test]
fn an_unreachable_bus_track_volume_point_is_still_reported_as_not_applied() {
    let scratch = Scratch::new("bus-volume");
    let project = filled_project();
    let bus = project
        .tracks
        .values()
        .find(|track| track.kind == yeban_model::TrackKind::AuxReturn)
        .expect("样本里必须有 Aux 轨")
        .id;
    let mut dispatcher = dispatcher(project, &scratch.project());

    // 正对照的机械锚：写泳道**之前**的导出字节。
    let baseline = render(&mut dispatcher, &scratch.wav());
    let baseline_sha = baseline["sha256"].clone();

    let data = write_point(&mut dispatcher, bus, "TrackVolume", json!({}));
    assert_eq!(data["appliedInOfflineRender"], false, "{data}");
    assert_eq!(data["residualReason"], "busTrack", "{data}");

    let rendered = render(&mut dispatcher, &scratch.wav());
    assert_eq!(
        rendered["sha256"], baseline_sha,
        "不可达 Aux 的泳道没有落点 ⇒ 一个字节都不许变: {rendered}"
    );
    assert!(
        unsupported_has_automation_lanes(&rendered),
        "总线轨的音量没有源落点 ⇒ 导出必须真的登记: {rendered}"
    );
}

/// ⭐ B2 正对照：**不可达** Aux 的 `TrackPan` 泳道仍报 `busTrack`。
///
/// B2 把 `TrackPan` 的判据从"源节点"改成"保留子图里的任何节点" ⇒ 可达总线转
/// `supported:true`；但**不可达**的总线（样本里的 Aux 没有 `aux → master`）不在
/// `rendered_nodes` 里，必须继续如实报 `busTrack`，且一个字节都不许变。
#[test]
fn an_unreachable_bus_track_pan_point_is_still_reported_as_not_applied() {
    let scratch = Scratch::new("bus-pan");
    let project = filled_project();
    let bus = project
        .tracks
        .values()
        .find(|track| track.kind == yeban_model::TrackKind::AuxReturn)
        .expect("样本里必须有 Aux 轨")
        .id;
    let mut dispatcher = dispatcher(project, &scratch.project());

    // 正对照的机械锚：写泳道**之前**的导出字节。
    let baseline = render(&mut dispatcher, &scratch.wav());
    let baseline_sha = baseline["sha256"].clone();

    let data = write_point(&mut dispatcher, bus, "TrackPan", json!({}));
    assert_eq!(data["appliedInOfflineRender"], false, "{data}");
    assert_eq!(data["residualReason"], "busTrack", "{data}");

    let rendered = render(&mut dispatcher, &scratch.wav());
    assert_eq!(
        rendered["sha256"], baseline_sha,
        "不可达 Aux 的声相泳道没有落点 ⇒ 一个字节都不许变: {rendered}"
    );
    assert!(
        unsupported_has_automation_lanes(&rendered),
        "总线轨的声相没有落点 ⇒ 导出必须真的登记: {rendered}"
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

/// ⭐ `#293`/`#304` 判据 4（`residualReason` 同步）：混响与压缩泳道**都不再**报
/// `deviceParamNotChannelStrip`；同一份工程里**只有**卷积 IR 形状继续报同一个值
/// （它仍是“插入链没有这个落点”）。
#[test]
fn a_reverb_and_a_compressor_land_while_ir_decay_keeps_the_residual() {
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
    assert_eq!(
        compressor["appliedInOfflineRender"], true,
        "⭐ #304：压缩阈值泳道有落点: {compressor}"
    );
    assert_eq!(compressor["residualReason"], Value::Null, "{compressor}");

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
        "### #304 residualReason: reverb={} compressor={} conv_ir_decay={}",
        reverb["residualReason"], compressor["residualReason"], convolution["residualReason"]
    );

    let rendered = render(&mut dispatcher, &scratch.wav());
    assert!(
        unsupported_has_automation_lanes(&rendered),
        "混响/压缩泳道已落点，但 IR 形状仍不可落点 ⇒ 导出仍应登记 automationLanes: {rendered}"
    );
}
