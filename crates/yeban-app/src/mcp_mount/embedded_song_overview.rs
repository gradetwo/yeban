//! 整曲总览图 Phase 1：真实 provider 的内嵌装配判据。
//!
//! 被测对象是**挂载路径全程**：`session_dispatcher`（注入真实 provider）→
//! `tools/call get_song_overview` → 响应 JSON。期望值走**第二条路径**：
//! 直接调 B 线管线（`yeban_render::song_overview::cached_overview`，
//! 新缓存、演示工程无采样器绑定故 provider 绝不被调用），把 dsp 的列式输出
//! 按 §8.2 的取整口径（归一化 2 位、dB 1 位）逐项换算后与 handler 的响应对照。
//! 两条路径逐位一致才算过 —— 不是"跑通了就行"。
//!
//! 演示工程的已知答案（B 线夹具，见
//! `crates/yeban-render/tests/song_overview_pipeline.rs`）：谱面 4 拍、
//! 音频 10 拍 ⇒ 3 小节；5 秒 @ 120 BPM；第一小节段落标签 `Intro`。

use serde_json::{Value, json};

use yeban_mcp::security::Channel;
use yeban_model::samples::{demo_project, demo_project_assets};
use yeban_model::{SamplerBinding, YebanProjectV1};
use yeban_render::engine_source::ArmedSfzInput;
use yeban_render::song_overview::{RenderSettings, SongOverviewCache, cached_overview};

use super::embedded_sfz_tools::{Scratch, call_tool};
use super::session_dispatcher;

/// 演示工程没有采样器绑定 ⇒ B 线管线的 provider 绝不能被调用（被调用即 panic）。
fn never_called_provider() -> impl FnMut(yeban_model::EntityId, &str) -> ArmedSfzInput {
    |_, _| panic!("演示工程没有采样器绑定，provider 不应被调用")
}

/// §8.2 取整口径：归一化指标 2 位小数。
fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// §8.2 取整口径：响度与频段能量 1 位小数。
fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

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

/// 经挂载路径调一次 `get_song_overview`，返回 `data`。
fn mounted_overview(project: &YebanProjectV1, arguments: Value) -> Value {
    let scratch = Scratch::new("song-overview");
    let mut dispatcher = session_dispatcher(
        project,
        &scratch.join("demo.yeban"),
        false,
        demo_project_assets(),
    )
    .expect("内嵌装配");
    call_tool(
        &mut dispatcher,
        Channel::Http,
        "get_song_overview",
        arguments,
    )
}

#[test]
fn mounted_real_provider_matches_the_direct_pipeline() {
    let project = demo_project();
    let response = mounted_overview(&project, json!({}));
    let data = ok_data(&response);
    assert_eq!(data["provider"], "real", "挂载注入的必须是真实 provider");
    assert_eq!(data["resolution"], "bar");

    // 独立重算：直接跑 B 线管线（新缓存），逐项换算后对照。
    let mut cache = SongOverviewCache::new();
    let (direct, hit) = cached_overview(
        &mut cache,
        &project,
        &RenderSettings::default(),
        never_called_provider(),
    )
    .expect("B 线管线");
    assert!(!hit, "新缓存首次必须是未命中");
    assert_eq!(direct.song_meta.total_bars, 3, "已知答案：3 小节");
    assert!(
        (direct.song_meta.duration_sec - 5.0).abs() < 1e-6,
        "已知答案：5 秒"
    );

    let meta = &data["song_meta"];
    assert_eq!(meta["total_bars"], u64::from(direct.song_meta.total_bars));
    assert_eq!(
        meta["duration_sec"],
        round1(f64::from(direct.song_meta.duration_sec))
    );
    assert_eq!(
        meta["base_bpm"],
        round1(f64::from(direct.song_meta.base_bpm))
    );
    assert_eq!(meta["key_signature"], direct.song_meta.key_signature);
    assert_eq!(meta["profile"], direct.song_meta.profile);
    assert_eq!(
        data["band_definitions_hz"],
        json!([
            [20, 90],
            [90, 250],
            [250, 500],
            [500, 1000],
            [1000, 2000],
            [2000, 5000],
            [5000, 10000],
            [10000, 20000]
        ])
    );

    let series = &direct.bar_series;
    let rows = data["bar_series"].as_array().expect("bar_series");
    assert_eq!(rows.len(), series.len());
    assert_eq!(rows.len(), 3, "已知答案：3 行");
    assert_eq!(rows[0]["section"], "Intro", "已知答案：首小节 Intro");
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row["bar"], series.bar[index]);
        assert_eq!(row["time_sec"], round2(f64::from(series.time_sec[index])));
        assert_eq!(row["section"], series.section[index]);
        assert_eq!(row["tension"], round2(f64::from(series.tension[index])));
        assert_eq!(
            row["valence_hue"],
            round2(f64::from(series.valence_hue[index]))
        );
        assert_eq!(
            row["lufs_short"],
            round1(f64::from(series.lufs_short[index]))
        );
        assert_eq!(
            row["lufs_momentary_max"],
            round1(f64::from(series.lufs_momentary_max[index]))
        );
        assert_eq!(row["score_dyn"], round2(f64::from(series.score_dyn[index])));
        assert_eq!(
            row["density_pitched_onsets_per_beat"],
            round2(f64::from(series.density_pitched_onsets_per_beat[index]))
        );
        assert_eq!(
            row["density_pitched_duty"],
            round2(f64::from(series.density_pitched_duty[index]))
        );
        assert_eq!(
            row["density_perc_onsets_per_beat"],
            round2(f64::from(series.density_perc_onsets_per_beat[index]))
        );
        let bands = row["bands_tilted_db"].as_array().expect("bands");
        assert_eq!(bands.len(), 8);
        for (band, value) in bands.iter().enumerate() {
            assert_eq!(
                value.as_f64().expect("band dB"),
                round1(f64::from(series.bands_tilted_db[index][band])),
                "第 {} 小节 Band {}",
                index + 1,
                band + 1
            );
        }
        assert!(row.get("beat").is_none(), "bar 模式不带 beat 键");
    }

    let summary = &data["structural_summary"];
    assert_eq!(
        summary["climax_bar"],
        u64::from(direct.structural_summary.climax_bar)
    );
    assert_eq!(
        summary["climax_tension"],
        round2(f64::from(direct.structural_summary.climax_tension))
    );
    assert_eq!(
        summary["dynamic_range_lu"],
        round1(f64::from(direct.structural_summary.dynamic_range_lu))
    );
    let anomalies = summary["detected_anomalies"].as_array().expect("anomalies");
    assert_eq!(
        anomalies.len(),
        direct.structural_summary.detected_anomalies.len()
    );
    for (index, anomaly) in anomalies.iter().enumerate() {
        let direct_anomaly = &direct.structural_summary.detected_anomalies[index];
        assert_eq!(
            anomaly["bars"],
            json!([direct_anomaly.bars[0], direct_anomaly.bars[1]])
        );
        assert_eq!(anomaly["type"], direct_anomaly.anomaly_type.as_str());
        assert_eq!(anomaly["description"], direct_anomaly.description);
    }
}

#[test]
fn mounted_second_call_returns_the_same_json() {
    let project = demo_project();
    let scratch = Scratch::new("song-overview-cache");
    let mut dispatcher = session_dispatcher(
        &project,
        &scratch.join("demo.yeban"),
        false,
        demo_project_assets(),
    )
    .expect("内嵌装配");
    let first = call_tool(
        &mut dispatcher,
        Channel::Http,
        "get_song_overview",
        json!({}),
    );
    let second = call_tool(
        &mut dispatcher,
        Channel::Http,
        "get_song_overview",
        json!({}),
    );
    let first_data = ok_data(&first);
    let second_data = ok_data(&second);
    assert_eq!(
        first_data, second_data,
        "第二次同一调用必须与第一次逐位相同（整曲哈希缓存命中）"
    );
}

#[test]
fn mounted_beat_resolution_expands_bars() {
    let project = demo_project();
    let response = mounted_overview(&project, json!({"resolution": "beat"}));
    let data = ok_data(&response);
    assert_eq!(data["resolution"], "beat");
    let rows = data["bar_series"].as_array().expect("bar_series");
    assert_eq!(rows.len(), 12, "3 小节 × 4 拍 = 12 行");
    for (index, row) in rows.iter().enumerate() {
        let bar = (index / 4) as u64 + 1;
        let beat = (index % 4) as u64 + 1;
        assert_eq!(row["bar"], bar);
        assert_eq!(row["beat"], beat);
        assert_eq!(row["time_sec"], index as f64 * 0.5, "120 BPM ⇒ 0.5 秒一拍");
    }
}

#[test]
fn mounted_missing_sfz_is_a_loud_render_failure() {
    let mut project = demo_project();
    let first_track = *project.tracks.keys().next().expect("演示工程有轨");
    project.tracks.get_mut(&first_track).expect("轨在").sampler = Some(SamplerBinding {
        path: "assets/samples/does-not-exist/missing.sfz".to_owned(),
    });
    let response = mounted_overview(&project, json!({}));
    let (code, data) = error_data(&response);
    assert_eq!(code, "RENDER_FAILED");
    assert_eq!(data["reason"], "samplerUnreadable");
    assert_eq!(data["path"], "assets/samples/does-not-exist/missing.sfz");
}
