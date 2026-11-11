//! 整曲多层可视化总览图（`get_song_overview`，设计稿 §8，Phase 1；Phase 2
//! 的 `profile` / `expected_levels` 参数与包络列也在本文件钉死）的端到端判据。
//!
//! 全部断言都以** stub provider 的同一份 8 小节序列**为被测对象，但期望值
//! 走本文件内**独立重算**的第二条路径：张力按设计稿 §3.2 的公式从分量表
//! 逐项重算（不读 `domain::song_overview` 的 `FIXTURE_*` 常量）；频段表与
//! 异常算术也在本文件内另抄一份并逐项对数。两条路径逐位一致才算过。
//! ⛔ 不依赖任何外部服务；不碰真实渲染音频（真实 provider 由 dsp 线落地
//! 后另行接线，本文件只钉 stub 下的注册 / schema / handler / 边界）。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::error::Fault;
use yeban_mcp::domain::song_overview::{
    SongOverviewProvider, SongOverviewRequest, StubSongOverviewProvider, overview_html_report,
    overview_html_report_with_title,
};
use yeban_mcp::output_schema::{
    BAND_DEFINITIONS_HZ, BarRow, DetectedAnomaly, FrequencyClashSummary, RepetitionSummary,
    SongMeta, SongOverview, StructuralSummary, for_tool,
};
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::{EXTENSION_NAMES, EXTENSION_TOOL_COUNT, TOOL_COUNT};
use yeban_model::samples::{demo_project, filled_project};
use yeban_model::{EntityId, YebanProjectV1};

// ---------------------------------------------------------------------------
// 脚手架（与 `tests/svs_tools.rs` 同款，不跨测试文件引用）
// ---------------------------------------------------------------------------

fn dispatcher_with_project(project: YebanProjectV1, with_stub: bool) -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(unique_path("fixture"), project, false)
        .expect("注入规范工程");
    if with_stub {
        dispatcher
            .domain_mut()
            .set_song_overview_provider(Arc::new(StubSongOverviewProvider));
    }
    dispatcher
}

fn unique_path(tag: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "yeban-mcp-song-overview-{}-{tag}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ))
        .join("demo.yeban")
}

fn bearer(dispatcher: &Dispatcher) -> String {
    format!("Bearer {}", dispatcher.expected_token().expose())
}

/// 一次 `tools/call` 的 `ToolResponse`（`result` 里的 JSON 对象；领域失败走带内错误）。
fn call_tool(dispatcher: &mut Dispatcher, name: &str, arguments: Value) -> Value {
    let auth = bearer(dispatcher);
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
        "`{name}` 不得产出 JSON-RPC 层错误（领域失败走 ToolResponse）: {outcome:?}"
    );
    outcome.response.expect("响应").result.expect("result")
}

// ---------------------------------------------------------------------------
// 独立重算（第二条路径）：设计稿 §3.2 的张力公式，分量表另抄一份
// ---------------------------------------------------------------------------

/// 每小节的分量（与 stub 夹具同一份序列，**本文件另抄**，不读实现侧常量）：
/// `lufs_short` / `lufs_momentary_max` / `score_dyn` /
/// `pitched_onsets_per_beat` / `pitched_duty` / `perc_onsets_per_beat` /
/// `pitch_center_midi` / `harmony` / `active_tracks` / `octave_bandwidth` /
/// `valence_hue`。
const COMPONENTS: [[f64; 11]; 8] = [
    [
        -22.4, -19.8, 0.28, 1.5, 0.65, 0.5, 57.0, 0.20, 2.0, 1.5, -0.62,
    ],
    [
        -20.1, -17.5, 0.34, 2.0, 0.68, 1.0, 59.0, 0.25, 3.0, 2.0, -0.58,
    ],
    [
        -18.6, -16.0, 0.42, 2.5, 0.70, 1.0, 60.0, 0.30, 3.0, 2.0, -0.55,
    ],
    [
        -16.2, -13.6, 0.48, 3.0, 0.72, 2.0, 62.0, 0.35, 4.0, 2.5, -0.50,
    ],
    [
        -13.8, -11.2, 0.52, 3.5, 0.75, 2.5, 64.0, 0.42, 5.0, 3.0, -0.44,
    ],
    [
        -14.0, -11.4, 0.40, 2.0, 0.60, 1.0, 60.0, 0.30, 3.0, 2.0, -0.52,
    ],
    [
        -13.6, -11.0, 0.54, 4.0, 0.78, 3.0, 65.0, 0.48, 6.0, 3.5, -0.40,
    ],
    [
        -12.4, -9.8, 0.58, 4.5, 0.80, 3.5, 67.0, 0.55, 7.0, 4.0, -0.35,
    ],
];

/// 8 频段的倾斜后能量（dB，1 位小数；**本文件另抄夹具的公布值**，不读实现侧常量）。
/// 频段是夹具数据（无公式可重算），本表钉的是"handler 原样回传、不转置、不改值"；
/// 真正的独立算术在异常判据里：Band 3（250–500 Hz）在第 5–6 小节为 −6.8，
/// 余下 6 小节的均值恰为 −12.0（超量 +5.2 dB）；第 1 行与设计稿 §8.2 的示例行逐位相同。
const BANDS: [[f64; 8]; 8] = [
    [-18.2, -14.5, -12.0, -11.4, -15.0, -19.2, -24.0, -31.5],
    [-17.0, -13.8, -12.1, -11.0, -14.6, -18.8, -23.5, -31.0],
    [-16.2, -13.2, -11.9, -10.6, -14.0, -18.0, -22.8, -30.2],
    [-15.0, -12.4, -12.2, -10.2, -13.5, -17.4, -22.0, -29.5],
    [-14.2, -11.8, -6.8, -10.0, -13.2, -17.0, -21.6, -29.0],
    [-14.6, -12.0, -6.8, -10.4, -13.6, -17.5, -22.2, -29.6],
    [-13.8, -11.2, -12.0, -9.8, -13.0, -16.8, -21.2, -28.6],
    [-13.0, -10.6, -11.8, -9.4, -12.6, -16.2, -20.8, -28.0],
];

/// 设计稿 §3.2 字面公式（固定锚点，不做全曲 Min-Max）：
/// `T = w_L·L̂ + w_D·D̂ + w_P·P̂ + w_H·Ĥ + w_O·Ô`。
/// 密度项 `D̂ = clamp(ln(1+d)/ln(1+16))` 里 `d` 是**每秒等效触发数**：
/// `d = (旋律起音/拍 + 打击起音/拍) × 拍/秒`（120 BPM ⇒ 2 拍/秒；速度是
/// 密度计算的时基乘数）。占空比不进密度公式（与 dsp 同口径）。
/// 本函数是**独立于 handler 的第二份实现**：分量表另抄、公式逐字照设计稿抄。
/// 权重按曲风档取（Phase 2，设计稿 §3.6；与 dsp 的 `TensionProfile::weights`
/// 逐位一致）：Default [0.30, 0.20, 0.20, 0.15, 0.15]、
/// Orchestral [0.25, 0.15, 0.30, 0.20, 0.10]、
/// Electronic [0.35, 0.30, 0.10, 0.10, 0.15]。
fn recompute_tension_with_weights(c: &[f64; 11], weights: [f64; 5]) -> f64 {
    let lufs = c[0];
    let pitched_onsets = c[3];
    let perc_onsets = c[5];
    let pitch_center = c[6];
    let harmony = c[7];
    let active_tracks = c[8];
    let octave_bw = c[9];
    let l_hat = ((lufs + 36.0) / 28.0).clamp(0.0, 1.0);
    let notes_per_sec = (pitched_onsets + perc_onsets) * 2.0;
    let d_hat = ((1.0 + notes_per_sec).ln() / 17.0_f64.ln()).clamp(0.0, 1.0);
    let p_hat = ((pitch_center - 36.0) / 48.0).clamp(0.0, 1.0);
    // `Ô`：配器宽度（活跃轨数 / 8 与八度带宽 / 5 各占一半）。
    let o_hat = 0.5 * (active_tracks / 8.0).min(1.0) + 0.5 * (octave_bw / 5.0).min(1.0);
    let t = weights[0] * l_hat
        + weights[1] * d_hat
        + weights[2] * p_hat
        + weights[3] * harmony
        + weights[4] * o_hat;
    (t * 100.0).round() / 100.0
}

/// 缺省档（Default）的张力重算（旧测试的简写）。
fn recompute_tension(c: &[f64; 11]) -> f64 {
    recompute_tension_with_weights(c, [0.30, 0.20, 0.20, 0.15, 0.15])
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// `data` 必须是成功响应：`status == "success"`（`ToolResponse` 的成功字面量，
/// 见 `tests/extension_tools.rs` 的同款断言），且含 `data.song_meta`。
fn ok_data(response: &Value) -> &Value {
    assert_eq!(response["status"], "success", "{response}");
    &response["data"]
}

fn error_data(response: &Value) -> (&str, &Value) {
    assert_eq!(response["status"], "error", "{response}");
    (
        response["error"]["code"].as_str().expect("code"),
        &response["error"]["data"],
    )
}

/// 同上，但**不拆包**：协议层（JSON-RPC）的拒绝也要能被断言。
///
/// 需要它是因为注册表类型校验先于领域层：`expected_levels` 不是对象时，
/// 请求走不到 handler（`-32602`），于是"参数问题"在这里有**两个合法的落点**
/// （口径先例：`tests/export_stems.rs` 的 `call_raw`）。
fn call_raw(dispatcher: &mut Dispatcher, arguments: Value) -> yeban_mcp::Response {
    let auth = bearer(dispatcher);
    let line = json!({
        "jsonrpc": "2.0",
        "id": "t",
        "method": "tools/call",
        "params": {"name": "get_song_overview", "arguments": arguments}
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
    outcome.response.expect("必须有响应")
}

// ---------------------------------------------------------------------------
// 判据
// ---------------------------------------------------------------------------

#[test]
fn stub_bar_overview_matches_independent_recomputation() {
    let mut dispatcher = dispatcher_with_project(filled_project(), true);
    let response = call_tool(&mut dispatcher, "get_song_overview", json!({}));
    let data = ok_data(&response);

    // 顶层形状（设计稿 §8.2；schema 1.1 起加 `repetition`）。
    assert_eq!(data["schema_version"], "1.2");
    assert_eq!(data["provider"], "stub");
    assert_eq!(data["resolution"], "bar");
    // stub 不算重复检测：重复摘要为空（形状完整、值为空）。
    assert_eq!(data["repetition"]["best_matches"], json!([]));
    assert_eq!(data["repetition"]["mechanical_index"], 0.0);
    assert_eq!(data["repetition"]["repeated_passages"], json!([]));
    // stub 不算打架检测：重叠摘要为空（形状完整、值为空）。
    assert_eq!(data["frequency_clash"]["total_groups"], 0);
    assert_eq!(data["frequency_clash"]["overall_overlap"], 0.0);
    assert_eq!(data["frequency_clash"]["bar_overlaps"], json!([]));
    assert_eq!(data["frequency_clash"]["entries"], json!([]));
    // `song_meta` 的键名以 `output_schema::SongMeta` 为准（无 title / bpm 别名）。
    assert_eq!(data["song_meta"]["total_bars"], 8);
    assert_eq!(data["song_meta"]["duration_sec"], 16.0);
    assert_eq!(data["song_meta"]["base_bpm"], 120.0);
    assert_eq!(data["song_meta"]["key_signature"], "D minor");
    // 缺省档 = `default`（Phase 2：stub 的 profile 现在跟请求走，不再写死）。
    assert_eq!(data["song_meta"]["profile"], "default");
    assert_eq!(data["band_definitions_hz"], json!(BAND_DEFINITIONS_HZ));
    // 缺省不给 `include_png_path` 时两个键根本不出席（不是 null 占位）。
    assert!(data.get("pngPath").is_none(), "缺省不给 png_path");
    assert!(
        data.get("pngNote").is_none(),
        "缺省不给 png_path 时不带说明"
    );

    let series = data["bar_series"].as_array().expect("bar_series");
    assert_eq!(series.len(), 8);
    for (i, row) in series.iter().enumerate() {
        let c = &COMPONENTS[i];
        let bar = i as u64 + 1;
        assert_eq!(row["bar"], bar, "第 {} 行的 bar 键", bar);
        assert_eq!(row["time_sec"], (bar - 1) as f64 * 2.0);
        let sections = [
            "Intro", "Intro", "Verse", "Verse", "Chorus", "Chorus", "Chorus", "Outro",
        ];
        assert_eq!(row["section"], sections[i], "第 {bar} 小节的段落标签");
        // 张力：独立重算（第二条路径）逐位一致。
        let expected_tension = recompute_tension(c);
        assert_eq!(
            row["tension"].as_f64().expect("tension"),
            expected_tension,
            "第 {bar} 小节张力：handler 输出与独立重算不一致"
        );
        assert_eq!(row["valence_hue"], c[10]);
        assert_eq!(row["lufs_short"], c[0]);
        assert_eq!(row["lufs_momentary_max"], c[1]);
        assert_eq!(row["score_dyn"], c[2]);
        assert_eq!(row["density_pitched_onsets_per_beat"], c[3]);
        assert_eq!(row["density_pitched_duty"], c[4]);
        assert_eq!(row["density_perc_onsets_per_beat"], c[5]);
        // 频段：另抄的表逐位一致（第 1 行与设计稿 §8.2 的示例行逐位相同）。
        let bands = row["bands_tilted_db"].as_array().expect("bands");
        assert_eq!(bands.len(), 8);
        for (j, b) in bands.iter().enumerate() {
            assert_eq!(
                b.as_f64().expect("band dB"),
                BANDS[i][j],
                "第 {bar} 小节 Band {}",
                j + 1
            );
        }
        assert!(row.get("beat").is_none(), "bar 模式不带 beat 键");
        // 无期望等级表时包络列缺席、偏离为 0.0（Phase 2：`None` 不序列化）。
        assert!(row.get("envelope_lo").is_none(), "无标注时不带 envelope_lo");
        assert!(row.get("envelope_hi").is_none(), "无标注时不带 envelope_hi");
        assert_eq!(row["target_deviation"], 0.0, "无标注时偏离为 0.0");
    }

    // 结构摘要：高潮在第 8 小节（张力 0.79），动态范围 10.0 LU。
    let summary = &data["structural_summary"];
    assert_eq!(summary["climax_bar"], 8);
    assert_eq!(summary["climax_tension"], 0.79);
    assert_eq!(summary["dynamic_range_lu"], 10.0);
    // 独立重算的张力列（Python 复核同一公式亦得此列）：[0.42, 0.50, 0.54, 0.62, 0.69, 0.58, 0.73, 0.79]。
    let tensions: Vec<f64> = (0..8).map(|i| recompute_tension(&COMPONENTS[i])).collect();
    assert_eq!(
        tensions,
        vec![0.42, 0.50, 0.54, 0.62, 0.69, 0.58, 0.73, 0.79]
    );
}

#[test]
fn anomalies_point_at_real_anomalies() {
    let mut dispatcher = dispatcher_with_project(filled_project(), true);
    let response = call_tool(&mut dispatcher, "get_song_overview", json!({}));
    let data = ok_data(&response);
    let anomalies = data["structural_summary"]["detected_anomalies"]
        .as_array()
        .expect("detected_anomalies");
    assert_eq!(anomalies.len(), 2, "stub 序列恰好两条真实异常");

    // 低频中段淤积：第 5–6 小节 Band 3 为 −6.8，比其余小节的约 −12.0 高 +5.2 dB。
    assert_eq!(anomalies[0]["type"], "low_mid_buildup");
    assert_eq!(anomalies[0]["bars"], json!([5, 6]));
    let band3_buildup = BANDS[4][2];
    let band3_elsewhere: Vec<f64> = (0..8)
        .filter(|&i| i != 4 && i != 5)
        .map(|i| BANDS[i][2])
        .collect();
    let band3_typical = band3_elsewhere.iter().sum::<f64>() / band3_elsewhere.len() as f64;
    assert_eq!(round2(band3_buildup - band3_typical), 5.2);
    assert!(
        anomalies[0]["description"]
            .as_str()
            .expect("description")
            .contains("+5.2 dB"),
        "{}",
        anomalies[0]["description"]
    );

    // 总线压缩压扁：第 6→7 小节谱面力度 0.40→0.54（+35.0%），渲染 LUFS 只 −14.0→−13.6（+0.4 LU）。
    assert_eq!(anomalies[1]["type"], "compression_squash");
    assert_eq!(anomalies[1]["bars"], json!([7, 8]));
    let score_rise = (COMPONENTS[6][2] - COMPONENTS[5][2]) / COMPONENTS[5][2];
    assert_eq!(round2(score_rise * 100.0), 35.0);
    let lufs_rise = COMPONENTS[6][0] - COMPONENTS[5][0];
    assert_eq!(round2(lufs_rise), 0.4);
    assert!(
        anomalies[1]["description"]
            .as_str()
            .expect("description")
            .contains("+35%"),
        "{}",
        anomalies[1]["description"]
    );
}

#[test]
fn beat_resolution_expands_each_bar_to_four_beats() {
    let mut dispatcher = dispatcher_with_project(filled_project(), true);
    let response = call_tool(
        &mut dispatcher,
        "get_song_overview",
        json!({"resolution": "beat"}),
    );
    let data = ok_data(&response);
    assert_eq!(data["resolution"], "beat");
    let series = data["bar_series"].as_array().expect("bar_series");
    assert_eq!(series.len(), 32, "8 小节 × 4 拍 = 32 行");
    for (i, row) in series.iter().enumerate() {
        let bar = (i / 4) as u64 + 1;
        let beat = (i % 4) as u64 + 1;
        assert_eq!(row["bar"], bar);
        assert_eq!(row["beat"], beat, "每小节内 beat 1–4 循环");
        assert_eq!(row["time_sec"], i as f64 * 0.5, "0.5 秒一拍");
        // 拍级行继承所属小节的值（stub 口径；真实 provider 按拍重算）。
        assert_eq!(
            row["tension"].as_f64().expect("tension"),
            recompute_tension(&COMPONENTS[(bar - 1) as usize])
        );
        assert_eq!(row["lufs_short"], COMPONENTS[(bar - 1) as usize][0]);
    }
    // 结构摘要仍是小节级（高潮第 8 小节）。
    assert_eq!(data["structural_summary"]["climax_bar"], 8);
}

#[test]
fn include_png_path_returns_null_with_phase2_note() {
    let mut dispatcher = dispatcher_with_project(filled_project(), true);
    let response = call_tool(
        &mut dispatcher,
        "get_song_overview",
        json!({"include_png_path": true}),
    );
    let data = ok_data(&response);
    assert_eq!(data["pngPath"], Value::Null, "Phase 1 不产 PNG");
    let note = data["pngNote"].as_str().expect("pngNote");
    assert!(note.contains("Phase 2"), "{note}");
    assert!(note.contains("Slint"), "{note}");
}

#[test]
fn invalid_resolution_is_invalid_parameter_range() {
    let mut dispatcher = dispatcher_with_project(filled_project(), true);
    let response = call_tool(
        &mut dispatcher,
        "get_song_overview",
        json!({"resolution": "hour"}),
    );
    let (code, data) = error_data(&response);
    assert_eq!(code, "INVALID_PARAMETER_RANGE");
    assert_eq!(data["reason"], "unknownResolution");
    assert_eq!(data["accepted"], json!(["bar", "beat"]));
}

#[test]
fn no_provider_is_conflict_with_provider_unavailable() {
    // 故意不注入 stub：宿主没有提供分析能力 ⇒ 响亮拒绝，不返回空总览。
    let mut dispatcher = dispatcher_with_project(filled_project(), false);
    let response = call_tool(&mut dispatcher, "get_song_overview", json!({}));
    let (code, data) = error_data(&response);
    assert_eq!(code, "CONFLICT");
    assert_eq!(data["reason"], "providerUnavailable");
    assert!(
        data["hint"].as_str().expect("hint").contains("yeban-dsp"),
        "{}",
        data["hint"]
    );
}

#[test]
fn no_active_project_is_no_active_project() {
    // 工程未装载（`require_active` 先于 provider 检查）。
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher
        .domain_mut()
        .set_song_overview_provider(Arc::new(StubSongOverviewProvider));
    let response = call_tool(&mut dispatcher, "get_song_overview", json!({}));
    let (code, _) = error_data(&response);
    assert_eq!(code, "NO_ACTIVE_PROJECT");
}

#[test]
fn dry_run_does_not_change_state() {
    let mut dispatcher = dispatcher_with_project(filled_project(), true);
    let revision_before = dispatcher.domain().apply_revision();
    let commits_before = dispatcher.domain().commit_count();
    let first = call_tool(
        &mut dispatcher,
        "get_song_overview",
        json!({"dryRun": true}),
    );
    let second = call_tool(&mut dispatcher, "get_song_overview", json!({}));
    // dryRun 不返回同一信封：它返回预览信封（`data.preview.result` = 总览本体，
    // 另有 `stateUnchanged` / `wouldChangeState` / 前后摘要）。比较本体，不比较信封。
    let preview_result = &first["data"]["preview"]["result"];
    let mut normal_data = second["data"].clone();
    normal_data
        .as_object_mut()
        .expect("data 是对象")
        .remove("commitModel");
    assert_eq!(
        preview_result, &normal_data,
        "dryRun 预览的总览本体必须与正式调用逐位相同（只读工具无写副作用）"
    );
    assert_eq!(first["data"]["stateUnchanged"], true);
    assert_eq!(first["data"]["wouldChangeState"], false);
    // `projectChanges` 与前后摘要住在 `preview` 里，不在信封顶层（见 `domain::preview`）。
    assert_eq!(first["data"]["preview"]["projectChanges"], false);
    assert_eq!(
        first["data"]["preview"]["projectDigestBefore"],
        first["data"]["preview"]["projectDigestAfter"],
        "dryRun 前后工程摘要必须相同"
    );
    assert_eq!(dispatcher.domain().apply_revision(), revision_before);
    assert_eq!(dispatcher.domain().commit_count(), commits_before);
}

#[test]
fn struct_round_trip_and_rounding() {
    let mut dispatcher = dispatcher_with_project(filled_project(), true);
    let response = call_tool(&mut dispatcher, "get_song_overview", json!({}));
    let data = ok_data(&response);
    // 输出 JSON 能反序列化回 §8.2 的结构（字段名与形状由 structs 钉住）。
    let overview: SongOverview =
        serde_json::from_value(data.clone()).expect("SongOverview 反序列化");
    assert_eq!(overview.bar_series.len(), 8);
    assert_eq!(overview.structural_summary.climax_bar, 8);
    assert_eq!(overview.structural_summary.detected_anomalies.len(), 2);
    // 精度口径：归一化指标 2 位小数，响度与频段能量 1 位小数。
    for row in &overview.bar_series {
        assert_eq!(round2(row.tension), row.tension);
        assert_eq!(round2(row.valence_hue), row.valence_hue);
        assert_eq!((row.lufs_short * 10.0).round() / 10.0, row.lufs_short);
        assert_eq!(
            (row.lufs_momentary_max * 10.0).round() / 10.0,
            row.lufs_momentary_max
        );
        for b in &row.bands_tilted_db {
            assert_eq!((*b * 10.0).round() / 10.0, *b);
        }
        // 无期望等级表时包络列缺席、偏离为 0.0（Phase 2 的反序列化口径）。
        assert_eq!(row.envelope_lo, None);
        assert_eq!(row.envelope_hi, None);
        assert_eq!(row.target_deviation, 0.0);
    }
    assert_eq!(
        (overview.structural_summary.dynamic_range_lu * 10.0).round() / 10.0,
        overview.structural_summary.dynamic_range_lu
    );
}

#[test]
fn registry_and_schema_counts_agree() {
    // 注册表三常量（现读，不抄文档手抄数）。
    assert_eq!(TOOL_COUNT, 42);
    assert_eq!(EXTENSION_TOOL_COUNT, 32);
    assert_eq!(EXTENSION_NAMES.last(), Some(&"yeban_move_track"));
    assert!(EXTENSION_NAMES.contains(&"get_song_overview"));
    // §8.2 的形状由 structs + 本文件钉住，不进 `COVERED_TOOLS` 的广播
    // （见 `output_schema.rs` 的决策说明：不改他人的十工具广播测试）。
    assert_eq!(for_tool("get_song_overview"), None);
    // 契约 schema 的 name enum 含新工具（与注册表同序、同数）。
    let schema = include_str!("../../../schemas/mcp-tools.schema.json");
    let parsed: Value = serde_json::from_str(schema).expect("schema JSON");
    let enum_names = parsed["definitions"]["ToolCall"]["properties"]["name"]["enum"]
        .as_array()
        .expect("name enum");
    assert_eq!(enum_names.len(), 42);
    assert_eq!(enum_names.last().expect("末位"), "yeban_move_track");
    let defs = parsed["definitions"]["ExtensionToolArguments"]["$defs"]
        .as_object()
        .expect("$defs");
    assert_eq!(defs.len(), 32);
    assert!(defs.contains_key("get_song_overview"));
}

// ---------------------------------------------------------------------------
// 真实 provider 的接线口径（round 2）：`Domain` 把**当前**活跃工程传给 provider
// ---------------------------------------------------------------------------

/// 回显 provider：记下 handler 传进来的工程，再把调用交给 stub。
///
/// 用于证明 trait 的工程参数就是 `Domain` 当前的活跃工程（不是挂载时的快照、
/// 不是共享 cell 里上一次刷新的版本）。真实 provider（`yeban-app` 的
/// `RealSongOverviewProvider`）用同一条参数拿工程，见
/// `crates/yeban-mcp/src/domain/song_overview.rs` 的模块文档。
#[derive(Debug, Default)]
struct ProjectEchoProvider {
    seen: Arc<Mutex<Option<YebanProjectV1>>>,
}

impl SongOverviewProvider for ProjectEchoProvider {
    fn name(&self) -> &'static str {
        "echo"
    }

    fn overview(
        &self,
        project: &YebanProjectV1,
        request: &SongOverviewRequest,
    ) -> Result<SongOverview, Fault> {
        *self.seen.lock().expect("echo 锁未被毒化") = Some(project.clone());
        StubSongOverviewProvider.overview(project, request)
    }
}

fn dispatcher_with_echo(project: YebanProjectV1) -> (Dispatcher, Arc<ProjectEchoProvider>) {
    let echo = Arc::new(ProjectEchoProvider::default());
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(unique_path("echo"), project, false)
        .expect("注入规范工程");
    let provider: Arc<dyn SongOverviewProvider> = Arc::<ProjectEchoProvider>::clone(&echo);
    dispatcher.domain_mut().set_song_overview_provider(provider);
    (dispatcher, echo)
}

#[test]
fn provider_receives_the_live_active_project() {
    let project = filled_project();
    let (mut dispatcher, echo) = dispatcher_with_echo(project.clone());
    let response = call_tool(&mut dispatcher, "get_song_overview", json!({}));
    let data = ok_data(&response);
    assert_eq!(data["provider"], "echo");
    let seen = echo.seen.lock().expect("echo 锁").clone();
    assert_eq!(
        seen,
        Some(project),
        "provider 拿到的必须是 Domain 当前的活跃工程（逐位相同）"
    );
}

#[test]
fn provider_sees_the_project_after_it_is_replaced() {
    let first = filled_project();
    let (mut dispatcher, echo) = dispatcher_with_echo(first);
    let _ = call_tool(&mut dispatcher, "get_song_overview", json!({}));
    // 同一个 Domain 换一份工程再调：provider 必须看到新工程，不是挂载时的快照。
    // `open_in_memory` 经 `reset_history` 换活跃工程，但不碰已注入的 provider。
    let second = demo_project();
    dispatcher
        .domain_mut()
        .open_in_memory(unique_path("second"), second.clone(), false)
        .expect("换工程");
    let response = call_tool(&mut dispatcher, "get_song_overview", json!({}));
    assert_eq!(ok_data(&response)["provider"], "echo");
    let seen = echo.seen.lock().expect("echo 锁").clone();
    assert_eq!(
        seen,
        Some(second),
        "换工程后 provider 必须看到新工程（快照接线会在这里红）"
    );
}

// ---------------------------------------------------------------------------
// Phase 2（设计稿 §3.5 / §3.6）：曲风预设与期望等级表
// ---------------------------------------------------------------------------

#[test]
fn orchestral_profile_changes_tension_and_meta_profile() {
    let mut dispatcher = dispatcher_with_project(filled_project(), true);
    let response = call_tool(
        &mut dispatcher,
        "get_song_overview",
        json!({"profile": "orchestral"}),
    );
    let data = ok_data(&response);
    assert_eq!(data["song_meta"]["profile"], "orchestral");
    // 张力按管弦权重独立重算（第二条路径）：逐位一致，且第 8 小节
    // 0.75 ≠ 缺省档 0.79（档位真的换了权重，不只是改档名）。
    let series = data["bar_series"].as_array().expect("bar_series");
    assert_eq!(series.len(), 8);
    for (i, row) in series.iter().enumerate() {
        let expected =
            recompute_tension_with_weights(&COMPONENTS[i], [0.25, 0.15, 0.30, 0.20, 0.10]);
        assert_eq!(
            row["tension"].as_f64().expect("tension"),
            expected,
            "第 {} 小节管弦档张力",
            i + 1
        );
    }
    assert_eq!(series[7]["tension"], 0.75);
    assert_eq!(recompute_tension(&COMPONENTS[7]), 0.79);
}

#[test]
fn electronic_profile_changes_tension() {
    let mut dispatcher = dispatcher_with_project(filled_project(), true);
    let response = call_tool(
        &mut dispatcher,
        "get_song_overview",
        json!({"profile": "electronic"}),
    );
    let data = ok_data(&response);
    assert_eq!(data["song_meta"]["profile"], "electronic");
    let series = data["bar_series"].as_array().expect("bar_series");
    for (i, row) in series.iter().enumerate() {
        let expected =
            recompute_tension_with_weights(&COMPONENTS[i], [0.35, 0.30, 0.10, 0.10, 0.15]);
        assert_eq!(
            row["tension"].as_f64().expect("tension"),
            expected,
            "第 {} 小节电子档张力",
            i + 1
        );
    }
    // 第 8 小节电子档 0.84 ≠ 缺省档 0.79。
    assert_eq!(series[7]["tension"], 0.84);
}

#[test]
fn unknown_profile_is_invalid_parameter_range() {
    let mut dispatcher = dispatcher_with_project(filled_project(), true);
    let response = call_tool(
        &mut dispatcher,
        "get_song_overview",
        json!({"profile": "rock"}),
    );
    let (code, data) = error_data(&response);
    assert_eq!(code, "INVALID_PARAMETER_RANGE");
    assert_eq!(data["reason"], "unknownProfile");
    assert_eq!(
        data["accepted"],
        json!(["default", "orchestral", "electronic"])
    );
}

#[test]
fn expected_levels_fill_envelope_band_and_flag_deviation() {
    let mut dispatcher = dispatcher_with_project(filled_project(), true);
    let response = call_tool(
        &mut dispatcher,
        "get_song_overview",
        json!({"expected_levels": {"Chorus": 5, "Intro": 1}}),
    );
    let data = ok_data(&response);
    let series = data["bar_series"].as_array().expect("bar_series");
    assert_eq!(series.len(), 8);

    // Intro（L1，带 [0, 0.25]）：第 1 小节张力 0.42 偏离 0.17，
    // 第 2 小节张力 0.50 偏离 0.25，两小节都超 0.15 告警阈值。
    assert_eq!(series[0]["envelope_lo"], 0.0);
    assert_eq!(series[0]["envelope_hi"], 0.25);
    assert_eq!(series[0]["target_deviation"], 0.17);
    assert_eq!(series[1]["envelope_lo"], 0.0);
    assert_eq!(series[1]["envelope_hi"], 0.25);
    assert_eq!(series[1]["target_deviation"], 0.25);
    // Chorus（L5，带 [0.75, 1.0]）：第 5 小节张力 0.69 偏离 0.06（带内告警线下），
    // 第 6 小节张力 0.58 偏离 0.17（超阈），第 7 小节张力 0.73 偏离 0.02。
    assert_eq!(series[4]["envelope_lo"], 0.75);
    assert_eq!(series[4]["envelope_hi"], 1.0);
    assert_eq!(series[4]["target_deviation"], 0.06);
    assert_eq!(series[5]["target_deviation"], 0.17);
    assert_eq!(series[6]["target_deviation"], 0.02);
    // Verse 无标注：包络列缺席、偏离为 0.0。
    assert!(series[2].get("envelope_lo").is_none());
    assert!(series[2].get("envelope_hi").is_none());
    assert_eq!(series[2]["target_deviation"], 0.0);

    // 异常：2 条夹具异常 + 2 条包络偏离异常（Intro [1,2] 与 Chorus [6,6]，
    // 连续超阈小节合并，单小节也成条）。
    let anomalies = data["structural_summary"]["detected_anomalies"]
        .as_array()
        .expect("detected_anomalies");
    assert_eq!(anomalies.len(), 4, "2 条夹具异常 + 2 条包络偏离异常");
    assert_eq!(anomalies[2]["type"], "envelope_deviation");
    assert_eq!(anomalies[2]["bars"], json!([1, 2]));
    assert!(
        anomalies[2]["description"]
            .as_str()
            .expect("description")
            .contains("Intro"),
        "{}",
        anomalies[2]["description"]
    );
    assert_eq!(anomalies[3]["type"], "envelope_deviation");
    assert_eq!(anomalies[3]["bars"], json!([6, 6]));
    assert!(
        anomalies[3]["description"]
            .as_str()
            .expect("description")
            .contains("Chorus"),
        "{}",
        anomalies[3]["description"]
    );

    // 包络列反序列化回 structs：有等级的行带值，无等级的行缺席。
    let overview: SongOverview =
        serde_json::from_value(data.clone()).expect("SongOverview 反序列化");
    assert_eq!(overview.bar_series[0].envelope_lo, Some(0.0));
    assert_eq!(overview.bar_series[0].envelope_hi, Some(0.25));
    assert_eq!(overview.bar_series[0].target_deviation, 0.17);
    assert_eq!(overview.bar_series[2].envelope_lo, None);
    assert_eq!(overview.bar_series[2].envelope_hi, None);
    assert_eq!(overview.bar_series[2].target_deviation, 0.0);
}

#[test]
fn invalid_expected_levels_are_rejected() {
    let mut dispatcher = dispatcher_with_project(filled_project(), true);
    // 非对象 ⇒ 协议层先拒（`-32602`，注册表类型校验；请求走不到领域层）。
    let response = call_raw(&mut dispatcher, json!({"expected_levels": 5}));
    assert!(response.result.is_none(), "类型非法不得同时带 result");
    let error = response.error.expect("非对象必须是 JSON-RPC 层错误");
    assert_eq!(error.code, -32602, "类型非法是 INVALID_PARAMS");
    let detail = serde_json::to_string(&error.data).expect("错误 data 必须可序列化");
    assert!(
        detail.contains("expected_levels"),
        "报文必须点名是哪个参数: {detail}"
    );
    // 对象内等级 0、等级 6、非整数、字符串值 ⇒ 领域层响亮拒绝。
    for arguments in [
        json!({"expected_levels": {"Chorus": 0}}),
        json!({"expected_levels": {"Chorus": 6}}),
        json!({"expected_levels": {"Chorus": 3.5}}),
        json!({"expected_levels": {"Chorus": "high"}}),
    ] {
        let response = call_tool(&mut dispatcher, "get_song_overview", arguments);
        let (code, data) = error_data(&response);
        assert_eq!(code, "INVALID_PARAMETER_RANGE");
        assert_eq!(data["reason"], "invalidExpectedLevels");
    }
    // 空对象合法 = 无标注（与缺省调用逐位相同）。
    let empty = call_tool(
        &mut dispatcher,
        "get_song_overview",
        json!({"expected_levels": {}}),
    );
    let plain = call_tool(&mut dispatcher, "get_song_overview", json!({}));
    assert_eq!(ok_data(&empty), ok_data(&plain));
}

// ---------------------------------------------------------------------------
// `format` 参数（HTML 报告）：缺省 json 一字不动，html 走唯一渲染器
// ---------------------------------------------------------------------------

/// `format` 缺省前（`{}`）的金标：2026-11-11 抓自改动前代码（临时抓取测试已删），
/// 规范序 JSON（`serde_json::to_string` 的键序）逐字节钉死。
/// 分轨频率打架线合入时同步更新：schema 1.1 → 1.2，`frequency_clash` 为空摘要
/// （stub 不算打架；真实值见 `frequency_clash` 判据）。
const GOLDEN_JSON: &str = r#"{"band_definitions_hz":[[20,90],[90,250],[250,500],[500,1000],[1000,2000],[2000,5000],[5000,10000],[10000,20000]],"bar_series":[{"bands_tilted_db":[-18.2,-14.5,-12.0,-11.4,-15.0,-19.2,-24.0,-31.5],"bar":1,"density_perc_onsets_per_beat":0.5,"density_pitched_duty":0.65,"density_pitched_onsets_per_beat":1.5,"lufs_momentary_max":-19.8,"lufs_short":-22.4,"score_dyn":0.28,"section":"Intro","target_deviation":0.0,"tension":0.42,"time_sec":0.0,"valence_hue":-0.62},{"bands_tilted_db":[-17.0,-13.8,-12.1,-11.0,-14.6,-18.8,-23.5,-31.0],"bar":2,"density_perc_onsets_per_beat":1.0,"density_pitched_duty":0.68,"density_pitched_onsets_per_beat":2.0,"lufs_momentary_max":-17.5,"lufs_short":-20.1,"score_dyn":0.34,"section":"Intro","target_deviation":0.0,"tension":0.5,"time_sec":2.0,"valence_hue":-0.58},{"bands_tilted_db":[-16.2,-13.2,-11.9,-10.6,-14.0,-18.0,-22.8,-30.2],"bar":3,"density_perc_onsets_per_beat":1.0,"density_pitched_duty":0.7,"density_pitched_onsets_per_beat":2.5,"lufs_momentary_max":-16.0,"lufs_short":-18.6,"score_dyn":0.42,"section":"Verse","target_deviation":0.0,"tension":0.54,"time_sec":4.0,"valence_hue":-0.55},{"bands_tilted_db":[-15.0,-12.4,-12.2,-10.2,-13.5,-17.4,-22.0,-29.5],"bar":4,"density_perc_onsets_per_beat":2.0,"density_pitched_duty":0.72,"density_pitched_onsets_per_beat":3.0,"lufs_momentary_max":-13.6,"lufs_short":-16.2,"score_dyn":0.48,"section":"Verse","target_deviation":0.0,"tension":0.62,"time_sec":6.0,"valence_hue":-0.5},{"bands_tilted_db":[-14.2,-11.8,-6.8,-10.0,-13.2,-17.0,-21.6,-29.0],"bar":5,"density_perc_onsets_per_beat":2.5,"density_pitched_duty":0.75,"density_pitched_onsets_per_beat":3.5,"lufs_momentary_max":-11.2,"lufs_short":-13.8,"score_dyn":0.52,"section":"Chorus","target_deviation":0.0,"tension":0.69,"time_sec":8.0,"valence_hue":-0.44},{"bands_tilted_db":[-14.6,-12.0,-6.8,-10.4,-13.6,-17.5,-22.2,-29.6],"bar":6,"density_perc_onsets_per_beat":1.0,"density_pitched_duty":0.6,"density_pitched_onsets_per_beat":2.0,"lufs_momentary_max":-11.4,"lufs_short":-14.0,"score_dyn":0.4,"section":"Chorus","target_deviation":0.0,"tension":0.58,"time_sec":10.0,"valence_hue":-0.52},{"bands_tilted_db":[-13.8,-11.2,-12.0,-9.8,-13.0,-16.8,-21.2,-28.6],"bar":7,"density_perc_onsets_per_beat":3.0,"density_pitched_duty":0.78,"density_pitched_onsets_per_beat":4.0,"lufs_momentary_max":-11.0,"lufs_short":-13.6,"score_dyn":0.54,"section":"Chorus","target_deviation":0.0,"tension":0.73,"time_sec":12.0,"valence_hue":-0.4},{"bands_tilted_db":[-13.0,-10.6,-11.8,-9.4,-12.6,-16.2,-20.8,-28.0],"bar":8,"density_perc_onsets_per_beat":3.5,"density_pitched_duty":0.8,"density_pitched_onsets_per_beat":4.5,"lufs_momentary_max":-9.8,"lufs_short":-12.4,"score_dyn":0.58,"section":"Outro","target_deviation":0.0,"tension":0.79,"time_sec":14.0,"valence_hue":-0.35}],"commitModel":"none","frequency_clash":{"bar_overlaps":[],"entries":[],"overall_overlap":0.0,"total_groups":0},"provider":"stub","repetition":{"best_matches":[],"mechanical_index":0.0,"repeated_passages":[]},"resolution":"bar","schema_version":"1.2","song_meta":{"base_bpm":120.0,"duration_sec":16.0,"key_signature":"D minor","profile":"default","total_bars":8},"structural_summary":{"climax_bar":8,"climax_tension":0.79,"detected_anomalies":[{"bars":[5,6],"description":"Band 3 (250-500Hz) exceeds tilted balance by +5.2 dB","type":"low_mid_buildup"},{"bars":[7,8],"description":"Score velocity rises +35% but rendered LUFS changes only +0.4 LU","type":"compression_squash"}],"dynamic_range_lu":10.0}}"#;

#[test]
fn default_json_is_byte_identical_to_pre_change_golden() {
    let mut dispatcher = dispatcher_with_project(filled_project(), true);
    for arguments in [json!({}), json!({"format": "json"})] {
        let response = call_tool(&mut dispatcher, "get_song_overview", arguments);
        let data = ok_data(&response);
        let canonical = serde_json::to_string(data).expect("data 可序列化");
        assert_eq!(canonical, GOLDEN_JSON, "缺省 json 与改前逐字节一致");
        assert!(data.get("format").is_none(), "响应里不回显 format 参数");
    }
}

#[test]
fn format_html_returns_the_same_report_the_renderer_produces() {
    let project = filled_project();
    let mut dispatcher = dispatcher_with_project(project.clone(), true);
    let json_response = call_tool(&mut dispatcher, "get_song_overview", json!({}));
    let overview: SongOverview =
        serde_json::from_value(ok_data(&json_response).clone()).expect("json data 反序列化");
    let html_response = call_tool(
        &mut dispatcher,
        "get_song_overview",
        json!({"format": "html"}),
    );
    let html = ok_data(&html_response)
        .as_str()
        .expect("html data 是字符串");
    assert_eq!(
        html,
        overview_html_report_with_title(&overview, &project.title),
        "handler 与渲染器同一份输出（同一工程标题）"
    );
    assert!(html.starts_with("<!doctype html>"));
    assert!(!html.contains("<script"), "报告里不含脚本");
    // 报告头带当前工程标题（`filled_project` 的 title）。
    assert!(
        html.contains("<h1>Yeban Model Core Sample</h1>"),
        "h1 是当前工程标题"
    );
    assert!(
        html.contains("<title>Yeban Model Core Sample · "),
        "title 是当前工程标题"
    );
}

#[test]
fn format_html_escapes_the_project_title() {
    let mut project = filled_project();
    project.title = "试听 <>&\"' 版".to_owned();
    let mut dispatcher = dispatcher_with_project(project, true);
    let html_response = call_tool(
        &mut dispatcher,
        "get_song_overview",
        json!({"format": "html"}),
    );
    let html = ok_data(&html_response)
        .as_str()
        .expect("html data 是字符串");
    let escaped = "试听 &lt;&gt;&amp;&quot;&#39; 版";
    assert!(
        html.contains(&format!("<h1>{escaped}</h1>")),
        "h1 里的工程标题必须转义"
    );
    assert!(
        html.contains(&format!("<title>{escaped} · ")),
        "title 里的工程标题必须转义"
    );
    assert!(!html.contains("试听 <>&\"' 版"), "原始标题不得原样出现");
}

#[test]
fn format_unknown_value_is_rejected() {
    let mut dispatcher = dispatcher_with_project(filled_project(), true);
    let response = call_tool(
        &mut dispatcher,
        "get_song_overview",
        json!({"format": "yaml"}),
    );
    let (code, data) = error_data(&response);
    assert_eq!(code, "INVALID_PARAMETER_RANGE");
    assert_eq!(data["reason"], "unknownFormat");
    assert_eq!(data["accepted"], json!(["json", "html"]));
}

#[test]
fn format_non_string_is_protocol_error() {
    let mut dispatcher = dispatcher_with_project(filled_project(), true);
    let response = call_raw(&mut dispatcher, json!({"format": 42}));
    assert!(response.result.is_none(), "类型非法不得同时带 result");
    let error = response.error.expect("非字符串必须是 JSON-RPC 层错误");
    assert_eq!(error.code, -32602, "类型非法是 INVALID_PARAMS");
    let detail = serde_json::to_string(&error.data).expect("错误 data 必须可序列化");
    assert!(
        detail.contains("format"),
        "报文必须点名是哪个参数: {detail}"
    );
}

#[test]
fn format_html_with_png_path_is_rejected() {
    let mut dispatcher = dispatcher_with_project(filled_project(), true);
    let response = call_tool(
        &mut dispatcher,
        "get_song_overview",
        json!({"format": "html", "include_png_path": true}),
    );
    let (code, data) = error_data(&response);
    assert_eq!(code, "INVALID_PARAMETER_RANGE");
    assert_eq!(data["reason"], "includePngPathRequiresJson");
}

/// 手写 2 小节总览：拍行去重 + 无期望带哨兵。
fn two_bar_overview_with_beat_duplicate() -> SongOverview {
    let row = |bar: u64, beat: Option<u64>, time_sec: f64| BarRow {
        bar,
        beat,
        time_sec,
        section: if bar == 1 { "Intro" } else { "Verse" }.to_owned(),
        tension: if bar == 1 { 0.42 } else { 0.5 },
        valence_hue: -0.6,
        lufs_short: -22.4,
        lufs_momentary_max: -19.8,
        score_dyn: 0.28,
        envelope_lo: None,
        envelope_hi: None,
        target_deviation: 0.0,
        density_pitched_onsets_per_beat: 1.0,
        density_pitched_duty: 0.65,
        density_perc_onsets_per_beat: 2.0,
        bands_tilted_db: [-18.2, -14.5, -12.0, -11.4, -15.0, -19.2, -24.0, -31.5],
    };
    SongOverview {
        schema_version: "1.1".to_owned(),
        song_meta: SongMeta {
            total_bars: 2,
            duration_sec: 4.0,
            base_bpm: 120.0,
            key_signature: "D minor".to_owned(),
            profile: "default".to_owned(),
        },
        band_definitions_hz: BAND_DEFINITIONS_HZ,
        structural_summary: StructuralSummary {
            climax_bar: 2,
            climax_tension: 0.5,
            dynamic_range_lu: 10.0,
            detected_anomalies: vec![],
        },
        repetition: RepetitionSummary {
            best_matches: vec![],
            mechanical_index: 0.0,
            repeated_passages: vec![],
        },
        // 夹具不声称重叠与否，字段只保形状完整，值为空摘要。
        frequency_clash: FrequencyClashSummary {
            total_groups: 0,
            overall_overlap: 0.0,
            bar_overlaps: vec![],
            entries: vec![],
        },
        bar_series: vec![
            row(1, Some(1), 0.0),
            row(1, Some(2), 1.5),
            row(2, None, 3.0),
        ],
    }
}

#[test]
fn overview_html_report_dedupes_beat_rows_and_shows_missing_envelope() {
    let html = overview_html_report(&two_bar_overview_with_beat_duplicate());
    assert!(
        !html.contains(">1.50<"),
        "去重后第 2 拍行（时间 1.5）不进报告"
    );
    assert!(html.contains(">3.00<"), "第 2 小节（时间 3.0）在报告里");
    assert!(html.contains("—"), "无期望带的小节显示 —（哨兵）");
}

#[test]
fn html_report_keeps_unknown_anomaly_text() {
    let mut overview = two_bar_overview_with_beat_duplicate();
    overview.structural_summary.detected_anomalies = vec![DetectedAnomaly {
        bars: [1, 2],
        anomaly_type: "phantom_type".to_owned(),
        description: "phantom 异常：描述原样保留".to_owned(),
    }];
    let html = overview_html_report(&overview);
    assert!(
        html.contains("phantom 异常：描述原样保留"),
        "未知异常类型的描述文本原样保留"
    );
    assert!(
        html.contains("low_mid_buildup"),
        "未知类型兜底成 low_mid_buildup（契约外值不可达，见转置文档）"
    );
}
