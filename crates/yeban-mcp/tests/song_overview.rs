//! 整曲多层可视化总览图（`get_song_overview`，设计稿 §8，Phase 1）的端到端判据。
//!
//! 全部断言都以** stub provider 的同一份 8 小节序列**为被测对象，但期望值
//! 走本文件内**独立重算**的第二条路径：张力按设计稿 §3.2 的公式从分量表
//! 逐项重算（不读 `domain::song_overview` 的 `FIXTURE_*` 常量）；频段表与
//! 异常算术也在本文件内另抄一份并逐项对数。两条路径逐位一致才算过。
//! ⛔ 不依赖任何外部服务；不碰真实渲染音频（真实 provider 由 dsp 线落地
//! 后另行接线，本文件只钉 stub 下的注册 / schema / handler / 边界）。

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::song_overview::StubSongOverviewProvider;
use yeban_mcp::output_schema::{BAND_DEFINITIONS_HZ, SongOverview, for_tool};
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::{EXTENSION_NAMES, EXTENSION_TOOL_COUNT, TOOL_COUNT};
use yeban_model::samples::filled_project;
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
/// `T = 0.30·L̂ + 0.20·D̂ + 0.20·P̂ + 0.15·Ĥ + 0.15·Ô`。
/// 密度项 `D̂ = clamp(ln(1+d)/ln(1+16))` 里 `d` 是**每秒等效触发数**：
/// `d = (旋律起音/拍 + 打击起音/拍) × 拍/秒`（120 BPM ⇒ 2 拍/秒；速度是
/// 密度计算的时基乘数）。占空比不进密度公式（与 dsp 同口径）。
/// 本函数是**独立于 handler 的第二份实现**：分量表另抄、公式逐字照设计稿抄。
fn recompute_tension(c: &[f64; 11]) -> f64 {
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
    let t = 0.30 * l_hat + 0.20 * d_hat + 0.20 * p_hat + 0.15 * harmony + 0.15 * o_hat;
    (t * 100.0).round() / 100.0
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

// ---------------------------------------------------------------------------
// 判据
// ---------------------------------------------------------------------------

#[test]
fn stub_bar_overview_matches_independent_recomputation() {
    let mut dispatcher = dispatcher_with_project(filled_project(), true);
    let response = call_tool(&mut dispatcher, "get_song_overview", json!({}));
    let data = ok_data(&response);

    // 顶层形状（设计稿 §8.2）。
    assert_eq!(data["schema_version"], "1.0");
    assert_eq!(data["provider"], "stub");
    assert_eq!(data["resolution"], "bar");
    // `song_meta` 的键名以 `output_schema::SongMeta` 为准（无 title / bpm 别名）。
    assert_eq!(data["song_meta"]["total_bars"], 8);
    assert_eq!(data["song_meta"]["duration_sec"], 16.0);
    assert_eq!(data["song_meta"]["base_bpm"], 120.0);
    assert_eq!(data["song_meta"]["key_signature"], "D minor");
    assert_eq!(data["song_meta"]["profile"], "orchestral_cinematic");
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
    }
    assert_eq!(
        (overview.structural_summary.dynamic_range_lu * 10.0).round() / 10.0,
        overview.structural_summary.dynamic_range_lu
    );
}

#[test]
fn registry_and_schema_counts_agree() {
    // 注册表三常量（现读，不抄文档手抄数）。
    assert_eq!(TOOL_COUNT, 31);
    assert_eq!(EXTENSION_TOOL_COUNT, 21);
    assert_eq!(EXTENSION_NAMES.last(), Some(&"get_song_overview"));
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
    assert_eq!(enum_names.len(), 31);
    assert_eq!(enum_names.last().expect("末位"), "get_song_overview");
    let defs = parsed["definitions"]["ExtensionToolArguments"]["$defs"]
        .as_object()
        .expect("$defs");
    assert_eq!(defs.len(), 21);
    assert!(defs.contains_key("get_song_overview"));
}
