//! 整曲总览图 Phase 1 管线（`yeban_render::song_overview`）的集成判据。
//!
//! 用仓库内演示工程（`yeban_model::samples::demo_project`，真实引擎合成、
//! 无采样器）跑通全链路：渲染 → 映射 → 分析 → 缓存。民谣 B（`nengfou_b.yeban`）
//! 的全曲渲染因 6 条 Discord GM 采样器文件不在本机磁盘上而无法在本机跑通，
//! 见 folk 探针报告（不属于本测试文件的断言范围）。

use yeban_dsp::song_overview::{AnalysisOptions, TensionProfile};
use yeban_model::samples::demo_project;
use yeban_model::{TrackKind, TrackV3, YebanProjectV1};
use yeban_render::engine_source::ArmedSfzInput;
use yeban_render::song_overview::{
    RenderSettings, SkippedReason, SongOverviewCache, SongOverviewError, cached_overview,
    render_project_to_buffer, song_overview_hash,
};

/// 演示工程没有采样器绑定 ⇒ provider 绝不能被调用（被调用即 panic）。
fn never_called_provider() -> impl FnMut(yeban_model::EntityId, &str) -> ArmedSfzInput {
    |_, _| panic!("演示工程没有采样器绑定，provider 不应被调用")
}

#[test]
fn rendering_the_demo_project_produces_a_real_master() {
    let project = demo_project();
    let settings = RenderSettings::default();
    let rendered =
        render_project_to_buffer(&project, &settings, never_called_provider()).expect("渲染");

    // 帧数：保留节点上最晚的摆放末端是 9600 tick（音频摆放）⇒ 5 s @ 120 BPM。
    assert_eq!(rendered.frames, 240_000, "9600 tick @ 120 BPM = 5 s");
    assert_eq!(rendered.samples.len(), 480_000);
    assert_eq!(rendered.sample_rate, 48_000);
    println!("### [demo-render] peak={:.6}", rendered.peak());
    assert!(
        rendered.peak() > 0.01,
        "MIDI 轨必须出声，实测峰值 {}",
        rendered.peak()
    );

    // 两条纯音频轨（贝斯、铺底）被跳过并如实登记；其余 4 条 MIDI 轨正常渲染。
    assert_eq!(rendered.skipped_tracks.len(), 2);
    for skipped in &rendered.skipped_tracks {
        assert_eq!(skipped.reason, SkippedReason::AudioClipsOnly);
    }
    let skipped_names: Vec<&str> = rendered
        .skipped_tracks
        .iter()
        .map(|skipped| project.tracks[&skipped.track].name.as_str())
        .collect();
    assert_eq!(skipped_names, vec!["贝斯", "铺底"]);
}

#[test]
fn the_cached_pipeline_analyses_the_demo_end_to_end() {
    let project = demo_project();
    let settings = RenderSettings::default();
    let mut cache = SongOverviewCache::new();

    let (first, first_hit) = cached_overview(
        &mut cache,
        &project,
        &settings,
        &AnalysisOptions::default(),
        never_called_provider(),
    )
    .expect("首次分析");
    assert!(!first_hit, "首次必须是未命中");
    assert_eq!((cache.hits(), cache.misses()), (0, 1));

    let (second, second_hit) = cached_overview(
        &mut cache,
        &project,
        &settings,
        &AnalysisOptions::default(),
        never_called_provider(),
    )
    .expect("二次分析");
    assert!(second_hit, "第二次同一哈希必须命中，不重算");
    assert_eq!((cache.hits(), cache.misses()), (1, 1));
    assert_eq!(first, second, "缓存返回的结果必须与现算逐位相同");

    // 总览形状：谱面 4 拍、音频 10 拍 ⇒ 3 小节；各列长度一致。
    assert_eq!(first.song_meta.total_bars, 3);
    assert!((first.song_meta.duration_sec - 5.0).abs() < 1e-6);
    assert_eq!(first.song_meta.base_bpm, 120.0);
    let series = &first.bar_series;
    assert_eq!(series.len(), 3);
    for column_len in [
        series.time_sec.len(),
        series.section.len(),
        series.tension.len(),
        series.valence_hue.len(),
        series.lufs_short.len(),
        series.lufs_momentary_max.len(),
        series.score_dyn.len(),
        series.envelope_lo.len(),
        series.envelope_hi.len(),
        series.target_deviation.len(),
        series.density_pitched_onsets_per_beat.len(),
        series.density_pitched_duty.len(),
        series.density_perc_onsets_per_beat.len(),
        series.bands_tilted_db.len(),
    ] {
        assert_eq!(column_len, 3, "各列长度必须恒等于总小节数");
    }
    // 段落标签来自演示工程的 sections（Intro 起于 0 拍）。
    assert_eq!(series.section[0], "Intro");
    // 数值合理性：张力在 [0,1] 内、频段能量有限、第一小节有谱面力度。
    for &tension in &series.tension {
        assert!((0.0..=1.0).contains(&tension), "张力 {tension} 越界");
    }
    for bands in &series.bands_tilted_db {
        for &db in bands {
            assert!(db.is_finite(), "频段能量必须有限");
        }
    }
    assert!(series.score_dyn[0] > 0.0, "第一小节必须有起音");
    println!(
        "### [demo-overview] bars={} climax_bar={} climax_tension={:.3} dynamic_range_lu={:.2} lufs_short={:?} tension={:?}",
        first.song_meta.total_bars,
        first.structural_summary.climax_bar,
        first.structural_summary.climax_tension,
        first.structural_summary.dynamic_range_lu,
        series.lufs_short,
        series.tension,
    );
}

#[test]
fn the_hash_changes_when_the_project_or_the_settings_change() {
    let project = demo_project();
    let settings = RenderSettings::default();
    let base = song_overview_hash(&project, &settings, &AnalysisOptions::default());
    assert_eq!(
        base,
        song_overview_hash(&project, &settings, &AnalysisOptions::default()),
        "同一输入同一哈希"
    );

    let mut faster = project.clone();
    faster.bpm = 121.0;
    assert_ne!(
        base,
        song_overview_hash(&faster, &settings, &AnalysisOptions::default()),
        "工程内容变化 ⇒ 哈希变 ⇒ 缓存失效"
    );

    let mut other_seed = settings;
    other_seed.seed = 1;
    assert_ne!(
        base,
        song_overview_hash(&project, &other_seed, &AnalysisOptions::default()),
        "渲染设置变化 ⇒ 哈希变 ⇒ 缓存失效"
    );

    // Phase 2：分析选项（曲风档 / 期望等级表）同样进哈希 —— 同一工程换档
    // 必须是不命中的另一条缓存项，⛔ 不许返回 Default 档的旧结果。
    let orchestral = AnalysisOptions::with_profile(TensionProfile::Orchestral);
    assert_ne!(
        base,
        song_overview_hash(&project, &settings, &orchestral),
        "曲风档变化 ⇒ 哈希变 ⇒ 缓存失效"
    );
    let mut levels = AnalysisOptions::default();
    levels.expected_levels.insert("Intro".to_string(), 5);
    assert_ne!(
        base,
        song_overview_hash(&project, &settings, &levels),
        "期望等级表变化 ⇒ 哈希变 ⇒ 缓存失效"
    );
}

#[test]
fn profile_and_expected_levels_flow_through_the_cache() {
    let project = demo_project();
    let settings = RenderSettings::default();
    let mut cache = SongOverviewCache::new();

    let (defaulted, _) = cached_overview(
        &mut cache,
        &project,
        &settings,
        &AnalysisOptions::default(),
        never_called_provider(),
    )
    .expect("Default 档分析");
    assert_eq!(defaulted.song_meta.profile, "default");

    // 换 Orchestral 档：未命中（新哈希）、档名进 meta、至少一小节张力变了。
    let (orchestral, hit) = cached_overview(
        &mut cache,
        &project,
        &settings,
        &AnalysisOptions::with_profile(TensionProfile::Orchestral),
        never_called_provider(),
    )
    .expect("Orchestral 档分析");
    assert!(!hit, "换档必须是未命中");
    assert_eq!(orchestral.song_meta.profile, "orchestral");
    assert_ne!(
        defaulted.bar_series.tension, orchestral.bar_series.tension,
        "换档后张力列必须变（否则档位没生效）"
    );

    // 同档再调：命中，逐位相同。
    let (again, hit) = cached_overview(
        &mut cache,
        &project,
        &settings,
        &AnalysisOptions::with_profile(TensionProfile::Orchestral),
        never_called_provider(),
    )
    .expect("同档二次分析");
    assert!(hit, "同档二次必须是命中");
    assert_eq!(orchestral, again);

    // 期望等级表：Intro L5（带 [0.75, 1.0]）⇒ 第一小节有带，且张力低于带下沿
    // 时偏离 > 0（演示工程第一小节张力 < 0.75，见打印值）。
    let mut levels = AnalysisOptions::default();
    levels.expected_levels.insert("Intro".to_string(), 5);
    let (targeted, hit) = cached_overview(
        &mut cache,
        &project,
        &settings,
        &levels,
        never_called_provider(),
    )
    .expect("带期望等级的分析");
    assert!(!hit, "新期望等级表必须是未命中");
    assert_eq!(targeted.bar_series.envelope_lo[0], 0.75);
    assert_eq!(targeted.bar_series.envelope_hi[0], 1.0);
    assert!(
        targeted.bar_series.target_deviation[0] > 0.0,
        "第一小节张力 {} 应在 L5 带之下、偏离 > 0",
        targeted.bar_series.tension[0]
    );
}

#[test]
fn sample_rate_mismatch_is_a_loud_error() {
    let project = demo_project();
    let settings = RenderSettings::new(44_100, 0);
    let error = render_project_to_buffer(&project, &settings, never_called_provider())
        .expect_err("采样率不一致必须响亮失败");
    assert_eq!(
        error,
        SongOverviewError::SampleRateMismatch {
            expected_hz: 48_000,
            got_hz: 44_100,
        }
    );
}

#[test]
fn an_empty_project_is_a_loud_error_not_silence() {
    let master = yeban_model::EntityId::default();
    let mut project = YebanProjectV1::default();
    project.tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    project.routing_graph.nodes.push(master);
    let error = render_project_to_buffer(
        &project,
        &RenderSettings::default(),
        never_called_provider(),
    )
    .expect_err("无内容工程必须响亮失败");
    assert_eq!(error, SongOverviewError::EmptyProject);
}
