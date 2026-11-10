//! 端到端口径判据：只走 `yeban-dsp` 的公开面。
//!
//! 每个断言都有独立可复算的期望值（手算见各测试注释），不只断言“跑通”：
//! 单频正弦的频段能量分布、静音的 LUFS、固定谱面的张力值、粗糙度与五度圈
//! 的解析值、聚类 Onset 的精确计数、双边滤波的解析平滑值。
//!
//! 规范锚点：设计稿 `~/workspace/yeban-songmap-design.md` §3–§6、§8.2。

use yeban_dsp::song_overview::{
    AnalysisOptions, AnomalyType, AudioInput, BAND_DB_FLOOR, NO_ENVELOPE, NO_LOUDNESS_READING_LUFS,
    NoteEvent, ScoreInput, SectionMarker, TensionProfile, adaptive_weights, analyze_repetition,
    analyze_song_overview, analyze_song_overview_with_options, bilateral_smooth,
    circle_of_fifths_distance, envelope_band, envelope_deviation, roughness_of_midis,
};

/// 正弦夹具（相位用 `f64` 算），返回交错立体声。
fn sine_stereo(amplitude: f32, frequency: f64, sample_rate: f32, len: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(len * 2);
    for i in 0..len {
        let phase = std::f64::consts::TAU * frequency * i as f64 / f64::from(sample_rate);
        let sample = (f64::from(amplitude) * phase.sin()) as f32;
        out.push(sample);
        out.push(sample);
    }
    out
}

/// 两正弦叠加（等幅），返回交错立体声。
fn two_sine_stereo(
    amplitude: f32,
    freq_a: f64,
    freq_b: f64,
    sample_rate: f32,
    len: usize,
) -> Vec<f32> {
    let mut out = Vec::with_capacity(len * 2);
    for i in 0..len {
        let phase_a = std::f64::consts::TAU * freq_a * i as f64 / f64::from(sample_rate);
        let phase_b = std::f64::consts::TAU * freq_b * i as f64 / f64::from(sample_rate);
        let sample = (f64::from(amplitude) * (phase_a.sin() + phase_b.sin())) as f32;
        out.push(sample);
        out.push(sample);
    }
    out
}

fn note(
    start_beat: f32,
    duration_beats: f32,
    midi: u8,
    velocity: u8,
    track: u32,
    is_percussion: bool,
) -> NoteEvent {
    NoteEvent {
        start_beat,
        duration_beats,
        midi,
        velocity,
        track,
        is_percussion,
    }
}

fn score_with(notes: Vec<NoteEvent>) -> ScoreInput {
    ScoreInput {
        notes,
        ..ScoreInput::default()
    }
}

#[test]
fn empty_inputs_give_empty_series() {
    let overview = analyze_song_overview(&ScoreInput::default(), None);
    assert_eq!(
        overview.song_meta.total_bars, 0,
        "空谱面且无音频时总小节数为 0"
    );
    assert!(overview.bar_series.is_empty(), "空输入时 bar_series 为空");
    assert_eq!(
        overview.structural_summary.climax_bar, 0,
        "无小节时高潮小节为 0"
    );
    assert_eq!(
        overview.structural_summary.climax_tension, 0.0,
        "无小节时高潮张力为 0"
    );
    assert!(overview.diagnostics().is_empty(), "空输入时无诊断异常");
}

#[test]
fn silence_gives_zero_tension_and_floor_values() {
    // 2 秒静音、空谱面：120 BPM 下 2 秒 = 4 拍 = 1 小节。
    let samples = vec![0.0f32; 48_000 * 2 * 2];
    let audio = AudioInput {
        samples: &samples,
        sample_rate: 48_000.0,
    };
    let overview = analyze_song_overview(&ScoreInput::default(), Some(&audio));
    assert_eq!(overview.song_meta.total_bars, 1);
    assert_eq!(
        overview.bar_series.tension[0], 0.0,
        "静音 + 无音符时张力为 0"
    );
    assert_eq!(
        overview.bar_series.lufs_short[0], NO_LOUDNESS_READING_LUFS,
        "3 s 短时窗未满时 lufs_short 记哨兵值 −70"
    );
    assert_eq!(
        overview.bar_series.lufs_momentary_max[0], NO_LOUDNESS_READING_LUFS,
        "静音时瞬时最大值记哨兵值 −70"
    );
    assert!(
        overview.bar_series.bands_tilted_db[0]
            .iter()
            .all(|&v| v == BAND_DB_FLOOR),
        "静音时 8 频段全为 −120 dB 底"
    );
    assert_eq!(
        overview.bar_series.score_dyn[0], 0.0,
        "无起音时谱面力度为 0"
    );
}

#[test]
fn near_silent_bar_lufs_is_clamped_to_sentinel() {
    // 演示工程的真实 bug：近静音小节的短时 LUFS 是有限值但荒谬（实测 −1009.74），
    // 直通后动态范围 996.39 LU。修后应夹到哨兵 −70。
    // 6 秒音频：前 2 秒 −20 LUFS 正弦，后 4 秒静音；120 BPM，3 小节（每小节 2 秒）。
    // 每拍一个音符，谱面覆盖 3 小节。
    let mut samples = sine_stereo(0.1, 997.0, 48_000.0, 48_000 * 2);
    samples.extend(vec![0.0f32; 48_000 * 4 * 2]);
    let audio = AudioInput {
        samples: &samples,
        sample_rate: 48_000.0,
    };
    let notes: Vec<NoteEvent> = (0..12)
        .map(|b| note(b as f32, 1.0, 60, 100, 0, false))
        .collect();
    let overview = analyze_song_overview(&score_with(notes), Some(&audio));
    assert_eq!(overview.song_meta.total_bars, 3);
    assert!(
        overview
            .bar_series
            .lufs_short
            .iter()
            .all(|&v| (-70.0..=0.0).contains(&v)),
        "lufs_short 全部落在 [−70, 0] 内，实际 {:?}",
        overview.bar_series.lufs_short
    );
    assert_eq!(
        overview.bar_series.lufs_short[2], NO_LOUDNESS_READING_LUFS,
        "全静音小节的 lufs_short 记哨兵值 −70，不应是 −1009 之类的有限极端值"
    );
    assert!(
        overview.bar_series.lufs_momentary_max[2] == NO_LOUDNESS_READING_LUFS,
        "全静音小节的瞬时最大值记哨兵值 −70，实际 {}",
        overview.bar_series.lufs_momentary_max[2]
    );
    assert!(
        overview.structural_summary.dynamic_range_lu < 70.0,
        "动态范围应小于 70 LU，实际 {}",
        overview.structural_summary.dynamic_range_lu
    );
}

#[test]
fn single_sine_1400hz_puts_energy_in_presence_band() {
    // 1 秒、幅 0.1 的 1400 Hz 正弦（取 Band 5 几何中心附近，避开 1000 Hz 频段边界）：
    // 时域均方 0.005 → −23.01 dBFS；Band 5（1000–2000 Hz）几何中心 1414 Hz，
    // 倾斜 +4.5·log2(1.414) = +2.25 dB，故 Band 5 倾斜后读数 ≈ −20.76 dB。
    let samples = sine_stereo(0.1, 1400.0, 48_000.0, 48_000);
    let audio = AudioInput {
        samples: &samples,
        sample_rate: 48_000.0,
    };
    let overview = analyze_song_overview(&ScoreInput::default(), Some(&audio));
    assert_eq!(overview.song_meta.total_bars, 1);
    let bands = overview.bar_series.bands_tilted_db[0];
    let band5 = bands[4];
    assert!(
        (-22.5..-19.0).contains(&band5),
        "Band 5 倾斜后读数应约 −20.76 dB，实际 {band5}"
    );
    let max_band = bands
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
        .map(|(i, _)| i)
        .unwrap();
    assert_eq!(max_band, 4, "1400 Hz 正弦的最亮频段应为 Band 5，下标 4");
    assert!(
        band5 - bands[0] > 15.0,
        "Band 5 应比 Sub 频段亮 15 dB 以上，实际差值 {}",
        band5 - bands[0]
    );
}

#[test]
fn tilt_compensates_equal_amplitude_sines() {
    // 55 Hz 与 15000 Hz 等幅（0.2）正弦各 2 秒（各取所在频段中部，避开边界）：
    // 原始功率相等，倾斜差 = 4.5·log2(√(10000·20000) / √(20·90))
    // = 4.5·log2(333.3) ≈ 37.7 dB，故 Band 8 应比 Band 1 亮约 37.7 dB。
    let samples = two_sine_stereo(0.2, 55.0, 15000.0, 48_000.0, 96_000);
    let audio = AudioInput {
        samples: &samples,
        sample_rate: 48_000.0,
    };
    let overview = analyze_song_overview(&ScoreInput::default(), Some(&audio));
    let bands = overview.bar_series.bands_tilted_db[0];
    let diff = bands[7] - bands[0];
    assert!(
        (30.0..45.0).contains(&diff),
        "等幅正弦的倾斜差应约 37.7 dB，实际 {diff}"
    );
    assert!(bands[7] > bands[0], "高频段倾斜后应亮于低频段");
}

#[test]
fn loudness_matches_gated_reference() {
    // 3 秒 −20 LUFS 的 997 Hz 正弦（幅 0.1）+ 1 秒静音，共 2 小节。
    // 第一小节中心 1 s 时 3 s 短时窗未满 → lufs_short 为哨兵值；
    // 第二小节中心 3 s 时短时窗恰满 → 约 −20.0 LUFS；
    // 瞬时窗 400 ms 在 0.4 s 即满，第一小节瞬时最大值约 −20.0 LUFS。
    // （−20.0 这一参考值见 loudness_true_peak.rs 的门限积分判据。）
    let mut samples = sine_stereo(0.1, 997.0, 48_000.0, 144_000);
    samples.extend(std::iter::repeat_n(0.0f32, 48_000 * 2));
    let audio = AudioInput {
        samples: &samples,
        sample_rate: 48_000.0,
    };
    let overview = analyze_song_overview(&ScoreInput::default(), Some(&audio));
    assert_eq!(overview.song_meta.total_bars, 2);
    assert_eq!(
        overview.bar_series.lufs_short[0], NO_LOUDNESS_READING_LUFS,
        "第一小节中心 1 s 时短时窗未满"
    );
    assert!(
        (-20.6..-19.4).contains(&overview.bar_series.lufs_short[1]),
        "第二小节短时 LUFS 应约 −20.0，实际 {}",
        overview.bar_series.lufs_short[1]
    );
    assert!(
        (-20.6..-19.4).contains(&overview.bar_series.lufs_momentary_max[0]),
        "第一小节瞬时最大值应约 −20.0，实际 {}",
        overview.bar_series.lufs_momentary_max[0]
    );
}

#[test]
fn roughness_known_answers() {
    // 单音只有自身谐波对，粗糙度接近 0；小二度约 0.5；八度远小于小二度；
    // 和弦（多音）比单音粗糙。注意：大三和弦的未归一化求和与小二度双音相近
    // （三音带来三倍谐波对，抵消了单对更协和的优势），故不以“大三 < 小二”
    // 作为判据——设计稿 §3.3 的定义就是对全部谐波对求和。
    let single = roughness_of_midis(&[60]);
    assert!(single < 0.01, "单音 C4 的粗糙度应 < 0.01，实际 {single}");
    let minor_second = roughness_of_midis(&[60, 61]);
    assert!(
        (0.3..0.7).contains(&minor_second),
        "小二度 C4+C#4 的粗糙度应约 0.5，实际 {minor_second}"
    );
    let octave = roughness_of_midis(&[60, 72]);
    assert!(octave < 0.1, "八度 C4+C5 的粗糙度应 < 0.1，实际 {octave}");
    assert!(octave < minor_second, "八度应比小二度协和");
    let major_triad = roughness_of_midis(&[60, 64, 67]);
    assert!(
        major_triad < 0.7,
        "大三和弦的粗糙度应有界（< 0.7），实际 {major_triad}"
    );
    assert!(
        major_triad > single,
        "和弦应比单音粗糙，实际 {major_triad} vs {single}"
    );
}

#[test]
fn circle_of_fifths_known_answers() {
    // 只有主音时偏移为 0；三全音时质心走到对径点，距离 2/2 = 1.0；
    // C+G 均权：质心 (0.933, 0.25)，到 (1, 0) 的距离 /2 = 0.1294。
    let mut tonic_only = [0.0f32; 12];
    tonic_only[0] = 1.0;
    assert!(
        circle_of_fifths_distance(&tonic_only, 0) < 1e-6,
        "纯主音轮廓的五度圈偏移应为 0"
    );
    let mut tritone = [0.0f32; 12];
    tritone[6] = 1.0;
    let d = circle_of_fifths_distance(&tritone, 0);
    assert!((d - 1.0).abs() < 1e-6, "三全音偏移应为 1.0，实际 {d}");
    let mut fifth = [0.0f32; 12];
    fifth[0] = 1.0;
    fifth[7] = 1.0;
    let d = circle_of_fifths_distance(&fifth, 0);
    assert!(
        (d - 0.1294).abs() < 0.001,
        "C+G 均权偏移应约 0.1294，实际 {d}"
    );
}

#[test]
fn bilateral_smooth_known_answers() {
    // 小抖动被抹平：[0.5, 0.55, 0.5, 0.55, …] 在下标 1 处的解析平滑值
    // = (0.5·0.96215 + 0.55 + 0.5·0.96215 + 0.55) / 3.9243 ≈ 0.5255。
    let wiggle = [0.5, 0.55, 0.5, 0.55, 0.5, 0.55, 0.5, 0.55];
    let out = bilateral_smooth(&wiggle, &[0]);
    assert!(
        (out[1] - 0.5255).abs() < 0.002,
        "小抖动的平滑值应约 0.5255，实际 {}",
        out[1]
    );
    // 大跳变被保留：0.2 → 0.9 的阶跃在交界两侧几乎不动（值域权重 ≈ 0.0005）。
    let step = [0.2, 0.2, 0.2, 0.2, 0.9, 0.9, 0.9, 0.9];
    let out = bilateral_smooth(&step, &[0]);
    assert!(out[3] < 0.21, "阶跃前最后一拍应仍约 0.2，实际 {}", out[3]);
    assert!(out[4] > 0.89, "阶跃后第一拍应仍约 0.9，实际 {}", out[4]);
    // Marker 硬切断：边界在下标 4 时两侧互不渗透，输出逐位等于输入。
    let out = bilateral_smooth(&step, &[0, 4]);
    assert_eq!(out[3], 0.2, "硬切断后阶跃前一拍应精确为 0.2");
    assert_eq!(out[4], 0.9, "硬切断后阶跃后一拍应精确为 0.9");
}

#[test]
fn density_clusters_chord_to_one_onset() {
    // 1 小节：6 音柱式和弦（同拍起音）+ 3 个四分音符 + 4 个鼓点。
    // 聚类后旋律 Onset 率应精确为 1.0 次/拍（未聚类会是 2.375 次/拍）；
    // 四分音符满铺时占空比为 1.0；鼓点 1.0 次/拍。
    let mut notes = vec![
        note(0.0, 1.0, 60, 100, 0, false),
        note(0.0, 1.0, 64, 100, 0, false),
        note(0.0, 1.0, 67, 100, 0, false),
        note(0.0, 1.0, 72, 100, 0, false),
        note(0.0, 1.0, 76, 100, 0, false),
        note(0.0, 1.0, 79, 100, 0, false),
        note(1.0, 1.0, 62, 100, 0, false),
        note(2.0, 1.0, 62, 100, 0, false),
        note(3.0, 1.0, 62, 100, 0, false),
    ];
    for beat in 0..4 {
        notes.push(note(beat as f32, 0.25, 36, 100, 1, true));
    }
    let overview = analyze_song_overview(&score_with(notes), None);
    assert_eq!(overview.song_meta.total_bars, 1);
    assert!(
        (overview.bar_series.density_pitched_onsets_per_beat[0] - 1.0).abs() < 1e-6,
        "聚类后旋律 Onset 率应精确为 1.0，实际 {}",
        overview.bar_series.density_pitched_onsets_per_beat[0]
    );
    assert!(
        (overview.bar_series.density_pitched_duty[0] - 1.0).abs() < 1e-6,
        "满铺四分音符的占空比应精确为 1.0，实际 {}",
        overview.bar_series.density_pitched_duty[0]
    );
    assert!(
        (overview.bar_series.density_perc_onsets_per_beat[0] - 1.0).abs() < 1e-6,
        "鼓点 Onset 率应精确为 1.0，实际 {}",
        overview.bar_series.density_perc_onsets_per_beat[0]
    );
}

#[test]
fn single_note_tension_known_value() {
    // 1 小节 C4 全音符（力度 100，无音频 → 谱面力度代理）：
    // Phase 2 方差自适应改写了固定权重下的旧已知答案 0.425（不是回归）：
    // 逐拍五维分量里只有密度在变（末拍窗内无起音 ⇒ D̂ 从 0.245 跌到 0，
    // σ_D ≈ 0.09 > 0.05）；响度、音区、和声、配器逐拍恒定（σ = 0，死特征），
    // 各减半、腾出的 0.40 全归密度 ⇒ 有效权重 [0.15, 0.60, 0.10, 0.075, 0.075]。
    // 逐拍张力 = 0.15×0.926 + 0.60×D̂ + 0.10×0.5 + 0.075×0.0625
    // ≈ [0.341, 0.302, 0.279, 0.194]；相邻拍差 < σ_v = 0.18，双边滤波
    // 强力抹平 ⇒ 小节均值约 0.28（实测 0.2813）。
    let notes = vec![note(0.0, 4.0, 60, 100, 0, false)];
    let overview = analyze_song_overview(&score_with(notes), None);
    assert_eq!(overview.song_meta.total_bars, 1);
    let expected_weights = [0.15, 0.60, 0.10, 0.075, 0.075];
    for (i, (got, want)) in overview
        .effective_weights
        .iter()
        .zip(expected_weights.iter())
        .enumerate()
    {
        assert!(
            (got - want).abs() < 1e-6,
            "第 {i} 维有效权重应为 {want}，实际 {got}"
        );
    }
    let tension = overview.bar_series.tension[0];
    assert!(
        (0.27..0.30).contains(&tension),
        "单音 C4 全音符的张力应约 0.28（自适应权重下），实际 {tension}"
    );
}

#[test]
fn minor_second_raises_tension() {
    // 第一小节 C4 全音符，第二小节 C4+C#4 全音符（C 大调）：
    // 小二度同时抬高粗糙度、五度圈偏移、密度与响度代理，滤波前张力上升约 0.10；
    // 但 0.10 < σ_v = 0.18，双边滤波会显著抹平这一跳变（设计稿 §3.4 只保证
    // > 0.25 的跳变被保留），实测小节均值上升约 0.044，故门取 0.03。
    let notes = vec![
        note(0.0, 4.0, 60, 100, 0, false),
        note(4.0, 4.0, 60, 100, 0, false),
        note(4.0, 4.0, 61, 100, 0, false),
    ];
    let overview = analyze_song_overview(&score_with(notes), None);
    assert_eq!(overview.song_meta.total_bars, 2);
    let rise = overview.bar_series.tension[1] - overview.bar_series.tension[0];
    assert!(
        rise > 0.03,
        "小二度小节的张力应比单音小节高 0.03 以上，实际上升 {rise}"
    );
}

#[test]
fn marker_sections_label_bars() {
    let mut notes = Vec::new();
    for bar in 0..4 {
        notes.push(note(bar as f32 * 4.0, 4.0, 60, 100, 0, false));
    }
    let mut score = score_with(notes);
    score.markers = vec![
        SectionMarker {
            bar: 1,
            label: "Intro".to_string(),
            expected_level: None,
        },
        SectionMarker {
            bar: 3,
            label: "Chorus".to_string(),
            expected_level: None,
        },
    ];
    let overview = analyze_song_overview(&score, None);
    assert_eq!(
        overview.bar_series.section,
        vec!["Intro", "Intro", "Chorus", "Chorus"],
        "段落标签应按最近 Marker 划分"
    );
}

#[test]
fn diagnostics_low_mid_buildup() {
    // 8 小节、300 Hz 主导（幅 0.3）+ 1000 Hz 微弱（幅 0.02）：
    // Band 3 比其余频段均值高 10 dB 以上，持续 8 小节 → 应报 low_mid_buildup [1, 8]。
    let mut samples = Vec::with_capacity(48_000 * 16 * 2);
    for i in 0..48_000 * 16 {
        let p300 = std::f64::consts::TAU * 300.0 * i as f64 / 48_000.0;
        let p1000 = std::f64::consts::TAU * 1000.0 * i as f64 / 48_000.0;
        let sample = (0.3 * p300.sin() + 0.02 * p1000.sin()) as f32;
        samples.push(sample);
        samples.push(sample);
    }
    let audio = AudioInput {
        samples: &samples,
        sample_rate: 48_000.0,
    };
    let overview = analyze_song_overview(&ScoreInput::default(), Some(&audio));
    assert_eq!(overview.song_meta.total_bars, 8);
    let found = overview
        .diagnostics()
        .iter()
        .find(|a| a.anomaly_type == AnomalyType::LowMidBuildup);
    let anomaly = found.expect("300 Hz 主导 8 小节应报 low_mid_buildup");
    assert_eq!(anomaly.bars, [1, 8], "淤积区间应覆盖全部 8 小节");
    assert!(
        anomaly.description.contains("250-500"),
        "描述应点名 Band 3（250-500Hz），实际 {}",
        anomaly.description
    );

    // 对照：1000 Hz 独奏 8 小节，Band 3 不超标 → 不应报 low_mid_buildup。
    let samples = sine_stereo(0.3, 1000.0, 48_000.0, 48_000 * 16);
    let audio = AudioInput {
        samples: &samples,
        sample_rate: 48_000.0,
    };
    let overview = analyze_song_overview(&ScoreInput::default(), Some(&audio));
    assert!(
        overview
            .diagnostics()
            .iter()
            .all(|a| a.anomaly_type != AnomalyType::LowMidBuildup),
        "1000 Hz 独奏不应报 low_mid_buildup"
    );
}

#[test]
fn diagnostics_compression_squash() {
    // 8 小节谱面力度 30→120 爬升，渲染音频恒定 −20 LUFS 正弦：
    // 力度升 0.71、LUFS 几乎不动 → 应报 compression_squash [2, 8]
    //（[1, 4] 窗口因第一小节无短时读数而跳过）。
    let mut notes = Vec::new();
    for (bar, velocity) in [30u8, 43, 56, 69, 82, 95, 108, 120].iter().enumerate() {
        notes.push(note(bar as f32 * 4.0, 4.0, 60, *velocity, 0, false));
    }
    let samples = sine_stereo(0.1, 997.0, 48_000.0, 48_000 * 16);
    let audio = AudioInput {
        samples: &samples,
        sample_rate: 48_000.0,
    };
    let overview = analyze_song_overview(&score_with(notes.clone()), Some(&audio));
    assert_eq!(overview.song_meta.total_bars, 8);
    let found = overview
        .diagnostics()
        .iter()
        .find(|a| a.anomaly_type == AnomalyType::CompressionSquash);
    let anomaly = found.expect("力度爬升 + LUFS 平头应报 compression_squash");
    assert_eq!(anomaly.bars, [2, 8], "压扁区间应为 [2, 8]");

    // 对照：渲染响度随谱面力度同步爬升（每小节幅度 0.02→0.23）→ 不应报。
    let mut samples = Vec::with_capacity(48_000 * 16 * 2);
    for i in 0..48_000 * 16 {
        let bar = i / (48_000 * 2);
        let amplitude = 0.02 + 0.03 * bar as f32;
        let phase = std::f64::consts::TAU * 997.0 * i as f64 / 48_000.0;
        let sample = amplitude * phase.sin() as f32;
        samples.push(sample);
        samples.push(sample);
    }
    let audio = AudioInput {
        samples: &samples,
        sample_rate: 48_000.0,
    };
    let overview = analyze_song_overview(&score_with(notes), Some(&audio));
    assert!(
        overview.diagnostics().is_empty(),
        "响度同步爬升时不应报任何异常，实际 {:?}",
        overview.diagnostics()
    );
}

#[test]
fn output_shape_matches_design_schema() {
    // 4 小节谱面 + 8 秒音频：核对 §8.2 的顶层形状与列式等长。
    let mut notes = Vec::new();
    for bar in 0..4 {
        notes.push(note(bar as f32 * 4.0, 4.0, 60, 100, 0, false));
    }
    let samples = sine_stereo(0.1, 997.0, 48_000.0, 48_000 * 8);
    let audio = AudioInput {
        samples: &samples,
        sample_rate: 48_000.0,
    };
    let overview = analyze_song_overview(&score_with(notes), Some(&audio));
    assert_eq!(overview.schema_version, "1.1");
    assert_eq!(
        overview.band_definitions_hz,
        [
            [20.0, 90.0],
            [90.0, 250.0],
            [250.0, 500.0],
            [500.0, 1000.0],
            [1000.0, 2000.0],
            [2000.0, 5000.0],
            [5000.0, 10000.0],
            [10000.0, 20000.0],
        ],
        "频段定义应与设计稿 §4.1 的表逐项一致"
    );
    assert_eq!(overview.song_meta.total_bars, 4);
    assert_eq!(overview.song_meta.base_bpm, 120.0);
    assert_eq!(overview.song_meta.key_signature, "C major");
    assert_eq!(overview.song_meta.profile, "default");
    // 有效权重由方差自适应定（本夹具有 8 秒音频：响度从哨兵爬到 −20 LUFS，
    // 是唯一活跃特征；其余四维死 ⇒ [0.65, 0.10, 0.10, 0.075, 0.075]）：
    // 此处只断言和为 1 且各维在 [0, 1] 内，精确值由方差自适应专测钉死。
    let weight_sum: f32 = overview.effective_weights.iter().sum();
    assert!(
        (weight_sum - 1.0).abs() < 1e-6,
        "有效权重和须为 1.0，实际 {weight_sum}"
    );
    // 断言审计（R115）：`.all(` 须有界 —— 权重数组 5 维，先钉长度再断言取值。
    assert_eq!(overview.effective_weights.len(), 5);
    assert!(
        overview
            .effective_weights
            .iter()
            .all(|&w| (0.0..=1.0).contains(&w)),
        "有效权重各维须在 [0, 1] 内，实际 {:?}",
        overview.effective_weights
    );
    let series = &overview.bar_series;
    assert_eq!(series.len(), 4);
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
        assert_eq!(column_len, 4, "列式数组各列长度须等于总小节数");
    }
    assert!((1..=4).contains(&overview.structural_summary.climax_bar));
    assert!(overview.structural_summary.dynamic_range_lu >= 0.0);
    assert!(
        series.tension.iter().all(|&t| (0.0..=1.0).contains(&t)),
        "张力须在 [0, 1] 内"
    );
    assert!(
        series
            .valence_hue
            .iter()
            .all(|&h| (-1.0..=1.0).contains(&h)),
        "效价值色相须在 [−1, 1] 内"
    );
}

#[test]
fn analyze_is_deterministic() {
    // 同一输入两次分析逐位相等（固定锚点 + BTreeSet 轨道统计，无随机源）。
    let notes = vec![
        note(0.0, 4.0, 60, 100, 0, false),
        note(4.0, 4.0, 61, 90, 1, false),
        note(0.0, 0.5, 36, 110, 2, true),
    ];
    let samples = sine_stereo(0.1, 997.0, 48_000.0, 48_000 * 4);
    let audio = AudioInput {
        samples: &samples,
        sample_rate: 48_000.0,
    };
    let first = analyze_song_overview(&score_with(notes.clone()), Some(&audio));
    let second = analyze_song_overview(&score_with(notes), Some(&audio));
    assert_eq!(first, second, "同一输入两次分析须逐位相等");
}

/// 4 分钟全曲纯计算耗时实测（`#[ignore]`：只在 release 档手动跑，不进常规套件）。
///
/// 目标 < 30 ms（设计稿 §9.1，realfft 口径）。实测见下：FFT 已换 realfft；
/// 剩余耗时主要在逐帧频段映射、LUFS 双二阶滤波与交错声道拆分（均为线性成本，
/// 非二次）。整曲哈希缓存使该耗时每版歌曲只付一次。
#[test]
#[ignore]
fn four_minute_timing_probe() {
    use std::time::Instant;
    let samples = sine_stereo(0.1, 997.0, 48_000.0, 48_000 * 240);
    let audio = AudioInput {
        samples: &samples,
        sample_rate: 48_000.0,
    };
    let mut notes = Vec::new();
    for bar in 0..120 {
        for beat in 0..4 {
            notes.push(note(bar as f32 * 4.0 + beat as f32, 1.0, 60, 100, 0, false));
        }
    }
    let score = score_with(notes);
    let start = Instant::now();
    let overview = analyze_song_overview(&score, Some(&audio));
    let elapsed = start.elapsed();
    assert_eq!(overview.song_meta.total_bars, 120);
    eprintln!("4 分钟全曲纯计算耗时：{elapsed:?}");
    assert!(
        elapsed.as_millis() < 10_000,
        "4 分钟分析耗时应 < 10 s，实际 {elapsed:?}"
    );
}

// ===========================================================================
// Phase 2（设计稿 §3.5 / §3.6 / §5.1）：曲风预设 3 档、方差自适应权重、
// 期望结构包络带与偏离告警。谱面力度列（§5.1 虚线的数据源）是既有列，
// 其数值由 `score_dyn` 的既有判据钉住（单音 / 爬升两测），虚线本身由
// 面板集成测试 ㉕ 断言。
// ===========================================================================

#[test]
fn profile_weights_are_exact_and_sum_to_one() {
    assert_eq!(
        TensionProfile::Default.weights(),
        [0.30, 0.20, 0.20, 0.15, 0.15],
        "Default 档必须是设计稿 §3.2 的固定默认权重"
    );
    assert_eq!(
        TensionProfile::Orchestral.weights(),
        [0.25, 0.15, 0.30, 0.20, 0.10],
        "Orchestral 档：音区 0.30、和声 0.20（叙事靠音区与和声推高潮）"
    );
    assert_eq!(
        TensionProfile::Electronic.weights(),
        [0.35, 0.30, 0.10, 0.10, 0.15],
        "Electronic 档：响度 0.35、密度 0.30（亮度落到响度 + 密度，见实现注释）"
    );
    for profile in [
        TensionProfile::Default,
        TensionProfile::Orchestral,
        TensionProfile::Electronic,
    ] {
        let sum: f32 = profile.weights().iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-6,
            "{profile:?} 的权重和须为 1.0，实际 {sum}"
        );
        assert_eq!(
            TensionProfile::from_name(profile.as_str()),
            Some(profile),
            "档名往返解析须一致"
        );
    }
    assert_eq!(TensionProfile::from_name("orchestral_cinematic"), None);
    assert_eq!(TensionProfile::from_name(""), None);
    assert_eq!(TensionProfile::default(), TensionProfile::Default);
}

#[test]
fn adaptive_weights_known_answers() {
    // 密度与和声死（σ ≤ 0.05）：各衰减 50%，腾出 0.10 + 0.075 = 0.175，
    // 按活跃特征（响度 0.30、音区 0.20、配器 0.15，和 0.65）占比重分。
    let out = adaptive_weights(
        [0.30, 0.20, 0.20, 0.15, 0.15],
        [0.10, 0.02, 0.10, 0.03, 0.10],
    );
    let expected = [
        0.30 + 0.175 * 0.30 / 0.65,
        0.10,
        0.20 + 0.175 * 0.20 / 0.65,
        0.075,
        0.15 + 0.175 * 0.15 / 0.65,
    ];
    for (i, (got, want)) in out.iter().zip(expected.iter()).enumerate() {
        assert!(
            (got - want).abs() < 1e-5,
            "第 {i} 维自适应权重应为 {want}，实际 {got}"
        );
    }
    let sum: f32 = out.iter().sum();
    assert!(
        (sum - 1.0).abs() < 1e-6,
        "自适应后权重和须为 1.0，实际 {sum}"
    );

    // 全活跃 ⇒ 原权重一位不动。
    assert_eq!(
        adaptive_weights([0.30, 0.20, 0.20, 0.15, 0.15], [0.10; 5]),
        [0.30, 0.20, 0.20, 0.15, 0.15]
    );
    // 全死 ⇒ 重归一回原权重（均匀 × 0.5 后再 ÷ 0.5）。
    assert_eq!(
        adaptive_weights([0.30, 0.20, 0.20, 0.15, 0.15], [0.0; 5]),
        [0.30, 0.20, 0.20, 0.15, 0.15]
    );
    // σ 恰为 0.05 视为死（≤ 门限）。
    let out = adaptive_weights(
        [0.30, 0.20, 0.20, 0.15, 0.15],
        [0.05, 0.10, 0.10, 0.10, 0.10],
    );
    assert!(
        (out[0] - 0.15).abs() < 1e-6,
        "σ = 0.05 的响度权重须衰减到 0.15，实际 {}",
        out[0]
    );
}

/// 包络带上下沿的近似比较（f32 舍入：L2 上沿算出 0.45000002，字面 0.45
/// 是另一位 f32；逐位相等会把舍入当失败）。
fn assert_band(level: u8, want_lo: f32, want_hi: f32) {
    let (lo, hi) = envelope_band(level).expect("1–5 级必须有带");
    assert!(
        (lo - want_lo).abs() < 1e-6,
        "L{level} 下沿应为 {want_lo}，实际 {lo}"
    );
    assert!(
        (hi - want_hi).abs() < 1e-6,
        "L{level} 上沿应为 {want_hi}，实际 {hi}"
    );
}

#[test]
fn envelope_band_known_answers() {
    assert_band(1, 0.0, 0.25);
    assert_band(3, 0.35, 0.65);
    assert_band(5, 0.75, 1.0);
    assert_band(2, 0.15, 0.45);
    assert_band(4, 0.55, 0.85);
    assert_eq!(envelope_band(0), None, "0 级越界 ⇒ 无带");
    assert_eq!(envelope_band(6), None, "6 级越界 ⇒ 无带");
    assert_eq!(envelope_deviation(0.5, 0.35, 0.65), 0.0, "带内偏离为 0");
    assert_eq!(
        envelope_deviation(0.9, 0.35, 0.65),
        0.25,
        "带上 0.25 ⇒ 偏离 0.25"
    );
    assert_eq!(
        envelope_deviation(0.1, 0.35, 0.65),
        0.25,
        "带下 0.25 ⇒ 偏离 0.25"
    );
    assert_eq!(
        envelope_deviation(0.5, NO_ENVELOPE, NO_ENVELOPE),
        0.0,
        "无带时偏离恒为 0"
    );
}

/// 4 小节高低音区交替全音符（高区：84/72 交替；低区：36/48 交替；同力度、
/// 同节奏、无音频）：音区是唯一活跃特征（σ_P ≈ 0.08 > 0.05），其余四维
/// 逐拍恒定（响度 σ ≈ 0.03、密度 σ ≈ 0.026、和声与配器 σ = 0）⇒ 死特征
/// 各减半、腾出的一半全归音区 ⇒ 音区有效权重 = 档权重 + 0.5×(1 − 档权重)：
/// Default 0.60、Orchestral 0.65、Electronic 0.55；其余四维 = 档权重 × 0.5。
/// 两谱面除音区外逐拍完全相同 ⇒ 张力差 = 音区有效权重 × 音区均值差（0.75）。
fn alternating_register_gap(profile: TensionProfile) -> (f32, [f32; 5]) {
    let score = |high: bool| {
        let (a, b) = if high { (84, 72) } else { (36, 48) };
        let notes: Vec<NoteEvent> = (0..4)
            .map(|bar| {
                note(
                    bar as f32 * 4.0,
                    4.0,
                    if bar % 2 == 0 { a } else { b },
                    100,
                    0,
                    false,
                )
            })
            .collect();
        analyze_song_overview_with_options(
            &score_with(notes),
            None,
            &AnalysisOptions::with_profile(profile),
        )
    };
    let high = score(true);
    let low = score(false);
    assert_eq!(high.song_meta.profile, profile.as_str());
    let mean = |overview: &yeban_dsp::song_overview::SongOverview| {
        overview.bar_series.tension.iter().sum::<f32>() / 4.0
    };
    (mean(&high) - mean(&low), high.effective_weights)
}

#[test]
fn profile_changes_the_register_gap_by_its_pitch_weight() {
    // 音区有效权重（唯一活跃特征 ⇒ 档权重 + 0.5×(1 − 档权重)），逐位钉死。
    let (default_gap, default_weights) = alternating_register_gap(TensionProfile::Default);
    assert!(
        (default_weights[2] - 0.60).abs() < 1e-4,
        "Default 档音区有效权重应为 0.60，实际 {:?}",
        default_weights
    );
    let (orchestral_gap, orchestral_weights) = alternating_register_gap(TensionProfile::Orchestral);
    assert!(
        (orchestral_weights[2] - 0.65).abs() < 1e-4,
        "Orchestral 档音区有效权重应为 0.65，实际 {:?}",
        orchestral_weights
    );
    let (electronic_gap, electronic_weights) = alternating_register_gap(TensionProfile::Electronic);
    assert!(
        (electronic_weights[2] - 0.55).abs() < 1e-4,
        "Electronic 档音区有效权重应为 0.55，实际 {:?}",
        electronic_weights
    );
    // 张力差 = 音区有效权重 × 0.75（音区均值差），排序与权重排序一致。
    // 双边滤波的值域权重会轻微压缩差值，门取理论值的 0.9 倍。
    assert!(
        default_gap > 0.60 * 0.75 * 0.9,
        "Default 档高低音区张力差应约 0.45，实际 {default_gap}"
    );
    assert!(
        orchestral_gap > default_gap + 0.01,
        "Orchestral 档张力差须大于 Default 档，实际 {orchestral_gap} vs {default_gap}"
    );
    assert!(
        electronic_gap < default_gap - 0.01,
        "Electronic 档张力差须小于 Default 档，实际 {electronic_gap} vs {default_gap}"
    );
}

#[test]
fn variance_adaptation_halves_dead_features_end_to_end() {
    // 8 小节 C4 全音符，力度 30 / 120 隔小节交替：音区、和声、配器三维
    // 逐拍恒定（σ = 0，必死）；响度随力度大幅波动（σ > 0.05，必活）；
    // 密度只随窗口长短微动（σ ≈ 0.027 < 0.05，死）。死特征权重各减半，
    // 腾出的 0.35 全归响度：有效权重 = [0.65, 0.10, 0.10, 0.075, 0.075]。
    let mut notes = Vec::new();
    for bar in 0..8 {
        let velocity = if bar % 2 == 0 { 30 } else { 120 };
        notes.push(note(bar as f32 * 4.0, 4.0, 60, velocity, 0, false));
    }
    let overview = analyze_song_overview(&score_with(notes), None);
    assert_eq!(overview.song_meta.total_bars, 8);
    let weights = overview.effective_weights;
    let expected = [0.65, 0.10, 0.10, 0.075, 0.075];
    for (i, (got, want)) in weights.iter().zip(expected.iter()).enumerate() {
        assert!(
            (got - want).abs() < 1e-4,
            "第 {i} 维有效权重应为 {want}，实际 {got}（全权重 {weights:?}）"
        );
    }
    let sum: f32 = weights.iter().sum();
    assert!((sum - 1.0).abs() < 1e-6, "有效权重和须为 1.0，实际 {sum}");
    // 断言审计（R115）：`.all(` 须有界 —— 8 小节张力列，先钉长度再断言取值。
    assert_eq!(overview.bar_series.tension.len(), 8);
    assert!(
        overview
            .bar_series
            .tension
            .iter()
            .all(|&t| (0.0..=1.0).contains(&t)),
        "自适应后张力仍须在 [0, 1] 内"
    );
}

fn four_bar_score_with_markers() -> ScoreInput {
    let mut notes = Vec::new();
    for bar in 0..4 {
        notes.push(note(bar as f32 * 4.0, 4.0, 60, 100, 0, false));
    }
    let mut score = score_with(notes);
    score.markers = vec![
        SectionMarker {
            bar: 1,
            label: "Intro".to_string(),
            expected_level: None,
        },
        SectionMarker {
            bar: 3,
            label: "Chorus".to_string(),
            expected_level: None,
        },
    ];
    score
}

#[test]
fn envelope_band_and_deviation_from_expected_levels() {
    // 期望等级表经选项传入：Intro L3（带 [0.35, 0.65]）、Chorus L5（带 [0.75, 1.0]）。
    // 四小节同音符的张力约 0.425：Intro 带内（偏离 0），Chorus 带下偏离约 0.325 > 0.15 ⇒ 告警 [3, 4]。
    let mut options = AnalysisOptions::default();
    options.expected_levels.insert("Intro".to_string(), 3);
    options.expected_levels.insert("Chorus".to_string(), 5);
    let overview =
        analyze_song_overview_with_options(&four_bar_score_with_markers(), None, &options);
    let series = &overview.bar_series;
    assert_eq!(series.envelope_lo, vec![0.35, 0.35, 0.75, 0.75]);
    assert_eq!(series.envelope_hi, vec![0.65, 0.65, 1.0, 1.0]);
    assert_eq!(series.target_deviation[0], 0.0, "Intro 张力在带内 ⇒ 偏离 0");
    assert_eq!(series.target_deviation[1], 0.0);
    assert!(
        series.target_deviation[2] > 0.15,
        "Chorus 张力应在 L5 带下 0.15 以上，实际偏离 {}（张力 {}）",
        series.target_deviation[2],
        series.tension[2]
    );
    assert!(
        (series.target_deviation[2] - (0.75 - series.tension[2])).abs() < 1e-6,
        "偏离量须等于带下沿减张力"
    );
    let found = overview
        .diagnostics()
        .iter()
        .find(|a| a.anomaly_type == AnomalyType::EnvelopeDeviation);
    let anomaly = found.expect("Chorus 偏离超 0.15 应报 envelope_deviation");
    assert_eq!(anomaly.bars, [3, 4], "偏离区间应为 Chorus 两小节");
    assert!(
        anomaly.description.contains("Chorus"),
        "描述应点名段落 Chorus，实际 {}",
        anomaly.description
    );

    // 对照：无期望等级表 ⇒ 全哨兵、无偏离、无该类异常。
    let plain = analyze_song_overview(&four_bar_score_with_markers(), None);
    // 断言审计（R115）：`.all(` 须有界 —— 4 小节三列，先钉长度再断言取值。
    assert_eq!(plain.bar_series.envelope_lo.len(), 4);
    assert_eq!(plain.bar_series.envelope_hi.len(), 4);
    assert_eq!(plain.bar_series.target_deviation.len(), 4);
    assert!(
        plain
            .bar_series
            .envelope_lo
            .iter()
            .all(|&v| v == NO_ENVELOPE),
        "无标注时包络下沿全为哨兵"
    );
    assert!(
        plain
            .bar_series
            .envelope_hi
            .iter()
            .all(|&v| v == NO_ENVELOPE),
        "无标注时包络上沿全为哨兵"
    );
    assert!(
        plain.bar_series.target_deviation.iter().all(|&v| v == 0.0),
        "无标注时偏离全为 0"
    );
    // 断言审计（R115）：`diagnostics()` 是方法调用接收者、无字段长度可钉 ——
    // 改写成计数断言（不含 `.all(`，不产生无界站点），语义与原断言同一条。
    assert_eq!(
        plain
            .diagnostics()
            .iter()
            .filter(|a| a.anomaly_type == AnomalyType::EnvelopeDeviation)
            .count(),
        0,
        "无标注时不应报 envelope_deviation"
    );

    // 对照：越界等级（9）与未知段落名忽略 ⇒ 同样无带。
    let mut bad = AnalysisOptions::default();
    bad.expected_levels.insert("Chorus".to_string(), 9);
    bad.expected_levels.insert("Bridge".to_string(), 5);
    let overview = analyze_song_overview_with_options(&four_bar_score_with_markers(), None, &bad);
    // 断言审计（R115）：`.all(` 须有界 —— 4 小节包络下沿，先钉长度再断言取值。
    assert_eq!(overview.bar_series.envelope_lo.len(), 4);
    assert!(
        overview
            .bar_series
            .envelope_lo
            .iter()
            .all(|&v| v == NO_ENVELOPE),
        "越界等级与未知段落必须被忽略"
    );
}

#[test]
fn envelope_level_on_the_marker_itself_works() {
    // 等级直接写在 Marker 上（不经选项表）同样出带：Intro L1 ⇒ [0, 0.25]。
    let mut score = four_bar_score_with_markers();
    score.markers[0].expected_level = Some(1);
    let overview = analyze_song_overview(&score, None);
    assert_eq!(overview.bar_series.envelope_lo[0], 0.0);
    assert_eq!(overview.bar_series.envelope_hi[0], 0.25);
    assert_eq!(overview.bar_series.envelope_lo[2], NO_ENVELOPE);
    // 选项表匹配到的 Marker 以表为准（Intro 改 L5）。
    let mut options = AnalysisOptions::default();
    options.expected_levels.insert("Intro".to_string(), 5);
    let overview = analyze_song_overview_with_options(&score, None, &options);
    assert_eq!(overview.bar_series.envelope_lo[0], 0.75);
    assert_eq!(overview.bar_series.envelope_hi[0], 1.0);
}

/// 由 14 维行（张力、LUFS、谱面力度、旋律 Onset 率、占空比、打击 Onset 率、
/// 8 频段 dB）拼一条 `BarSeries`（只为 `analyze_repetition` 的已知答案测试；
/// 不进 14 维的列（色相、瞬时 LUFS、包络列）填与分析无关的固定值）。
fn series_from_rows(rows: &[[f32; 14]]) -> yeban_dsp::song_overview::BarSeries {
    let n = rows.len();
    yeban_dsp::song_overview::BarSeries {
        bar: (1..=n as u32).collect(),
        time_sec: (0..n).map(|i| i as f32 * 2.0).collect(),
        section: vec!["Untitled".to_string(); n],
        tension: rows.iter().map(|r| r[0]).collect(),
        valence_hue: vec![0.0; n],
        lufs_short: rows.iter().map(|r| r[1]).collect(),
        lufs_momentary_max: vec![NO_LOUDNESS_READING_LUFS; n],
        score_dyn: rows.iter().map(|r| r[2]).collect(),
        envelope_lo: vec![NO_ENVELOPE; n],
        envelope_hi: vec![NO_ENVELOPE; n],
        target_deviation: vec![0.0; n],
        density_pitched_onsets_per_beat: rows.iter().map(|r| r[3]).collect(),
        density_pitched_duty: rows.iter().map(|r| r[4]).collect(),
        density_perc_onsets_per_beat: rows.iter().map(|r| r[5]).collect(),
        bands_tilted_db: rows
            .iter()
            .map(|r| [r[6], r[7], r[8], r[9], r[10], r[11], r[12], r[13]])
            .collect(),
    }
}

/// 余弦 1.0 的容差：同一向量两次求模的 f32 舍入可使商偏离 1.0 不到 1e-6。
fn assert_similarity_one(value: f32) {
    assert!(
        (value - 1.0).abs() < 1e-6,
        "同一行与自身的相似度须为 1.0，实际 {value}"
    );
}

#[test]
fn repetition_identical_rows_are_similarity_one() {
    // 8 行全同（非零行）：每对间隔 ≥ 4 的小节相似度均为 1.0。
    let row = [
        0.5, -20.0, 0.5, 2.0, 0.7, 1.0, -18.0, -14.0, -12.0, -11.0, -15.0, -19.0, -24.0, -31.0,
    ];
    let summary = analyze_repetition(&series_from_rows(&[row; 8]));
    assert_eq!(summary.best_matches.len(), 5, "只保留前 5 对");
    assert_eq!(summary.best_matches[0].bar_a, 1);
    assert_eq!(summary.best_matches[0].bar_b, 5);
    assert_similarity_one(summary.best_matches[0].similarity);
    // 每小节都有间隔 4 的 1.0 匹配（1↔5、2↔6、3↔7、4↔8）⇒ 机械指数 1.0。
    assert_eq!(summary.mechanical_index, 1.0);
    // 滞后 4 的对角线长 4（5–8 对 1–4）；滞后 ≥ 5 的对角线至多 3 小节，
    // 天然不成段 ⇒ 恰一段。
    assert_eq!(summary.repeated_passages.len(), 1);
    assert_eq!(summary.repeated_passages[0].bars, [5, 8]);
    assert_eq!(summary.repeated_passages[0].source_bars, [1, 4]);
    assert_similarity_one(summary.repeated_passages[0].mean_similarity);
}

#[test]
fn repetition_four_bar_verbatim_repeat_is_reported() {
    // 前 4 小节各不相同（张力/LUFS/力度逐小节拉开），后 4 小节逐字重复前 4 小节。
    let a = [
        [
            0.1, -30.0, 0.2, 1.0, 0.5, 0.5, -20.0, -16.0, -14.0, -12.0, -16.0, -20.0, -25.0, -32.0,
        ],
        [
            0.4, -20.0, 0.5, 2.0, 0.6, 1.0, -18.0, -14.0, -12.0, -11.0, -15.0, -19.0, -24.0, -31.0,
        ],
        [
            0.7, -15.0, 0.8, 3.0, 0.7, 2.0, -16.0, -13.0, -11.0, -10.0, -14.0, -18.0, -23.0, -30.0,
        ],
        [
            1.0, -10.0, 1.0, 4.0, 0.8, 3.0, -14.0, -12.0, -10.0, -9.0, -13.0, -17.0, -22.0, -29.0,
        ],
    ];
    let rows = [a[0], a[1], a[2], a[3], a[0], a[1], a[2], a[3]];
    let summary = analyze_repetition(&series_from_rows(&rows));
    assert_eq!(summary.repeated_passages.len(), 1);
    assert_eq!(summary.repeated_passages[0].bars, [5, 8]);
    assert_eq!(summary.repeated_passages[0].source_bars, [1, 4]);
    assert_similarity_one(summary.repeated_passages[0].mean_similarity);
    assert_eq!(summary.mechanical_index, 1.0);
    // 前 4 对 best_matches 是四组 1.0 的对角对（小节号升序）。
    assert_eq!(summary.best_matches.len(), 5);
    for (index, (a, b)) in [(1, 5), (2, 6), (3, 7), (4, 8)].iter().enumerate() {
        assert_eq!(summary.best_matches[index].bar_a, *a);
        assert_eq!(summary.best_matches[index].bar_b, *b);
        assert_similarity_one(summary.best_matches[index].similarity);
    }
}

#[test]
fn repetition_single_bar_repeat_is_not_reported() {
    // 8 行：中性底（LUFS −70、频段 −120 dB ⇒ 归一化全 0）上每行只在一个
    // 维度上为 1；只有第 1 行与第 5 行同维（张力）。其余任意两行支撑不交 ⇒
    // 相似度恰为 0.0。
    let mut rows = [[0.0f32; 14]; 8];
    for row in rows.iter_mut() {
        row[1] = -70.0;
        row[6..14].fill(-120.0);
    }
    rows[0][0] = 1.0;
    rows[1][2] = 1.0;
    rows[2][1] = 0.0; // LUFS 0 dB ⇒ 锚点归一化 1.0
    rows[3][4] = 1.0;
    rows[4][0] = 1.0;
    rows[5][6] = 0.0; // 频段 0 dB ⇒ 锚点归一化 1.0
    rows[6][13] = 0.0; // 频段 0 dB ⇒ 锚点归一化 1.0
    rows[7][3] = 8.0; // 旋律 Onset 率 8 次/拍 ⇒ 锚点归一化 1.0
    let summary = analyze_repetition(&series_from_rows(&rows));
    assert_eq!(summary.best_matches[0].bar_a, 1);
    assert_eq!(summary.best_matches[0].bar_b, 5);
    assert_similarity_one(summary.best_matches[0].similarity);
    assert_eq!(summary.best_matches[1].similarity, 0.0);
    // 只有 1、5 两小节的最佳非邻匹配 ≥ 0.95 ⇒ 机械指数 2/8。
    assert_eq!(summary.mechanical_index, 0.25);
    // 滞后 4 的对角线上只有第 5 小节撞分（长 1），不成段。
    assert!(summary.repeated_passages.is_empty());
}

#[test]
fn repetition_empty_series_is_empty() {
    let summary = analyze_repetition(&series_from_rows(&[]));
    assert!(summary.best_matches.is_empty());
    assert_eq!(summary.mechanical_index, 0.0);
    assert!(summary.repeated_passages.is_empty());
}

#[test]
fn repetition_verbatim_chorus_is_flagged_end_to_end() {
    // 16 小节：同一 4 小节型（每小节 4 个四分音符，音高按小节位 60/62/64/66，
    // 力度 80）连排 4 遍。中间 14 小节（2–15）各有间隔 4 的逐位相同对照 ⇒
    // 机械指数至少 14/16；滞后 4 的对角线上有长 10 的 1.0 段 ⇒ 必有一段。
    // （首尾两小节的冷启动/截断边效应只影响段的起止，不影响"有段"。）
    let mut notes = Vec::new();
    for bar in 0..16 {
        let midi = 60 + (bar % 4) as u8 * 2;
        for beat in 0..4 {
            notes.push(note(
                bar as f32 * 4.0 + beat as f32,
                1.0,
                midi,
                80,
                0,
                false,
            ));
        }
    }
    let overview = analyze_song_overview(&score_with(notes), None);
    assert_eq!(overview.schema_version, "1.1");
    assert!(
        overview.repetition.mechanical_index >= 0.875,
        "机械指数至少 14/16，实际 {}",
        overview.repetition.mechanical_index
    );
    assert!(
        !overview.repetition.repeated_passages.is_empty(),
        "4 遍逐字重复必须报出成段"
    );
    assert!(
        overview
            .diagnostics()
            .iter()
            .any(|a| a.anomaly_type == AnomalyType::ArrangementRepetition),
        "成段重复必须进 detected_anomalies"
    );
}

#[test]
fn repetition_no_passage_for_short_song() {
    // 7 小节：同一 4 小节型连排，第 5–7 小节是该型的头 3 小节。
    // 结构界：滞后 4 的对角线最长 7 − 4 = 3 小节 < MIN_REPEAT_BARS，
    // 任何滞后都成不了段 ⇒ 不报段落、不报异常（与相似度值无关）。
    // 贯穿变化的曲子不报段落，靠真实材料耳验裁决（设计稿 §5），不在此断言。
    let mut notes = Vec::new();
    for bar in 0..7 {
        let midi = 60 + (bar % 4) as u8 * 2;
        for beat in 0..4 {
            notes.push(note(
                bar as f32 * 4.0 + beat as f32,
                1.0,
                midi,
                80,
                0,
                false,
            ));
        }
    }
    let overview = analyze_song_overview(&score_with(notes), None);
    assert_eq!(overview.song_meta.total_bars, 7);
    assert!(overview.repetition.repeated_passages.is_empty());
    assert!(
        !overview
            .diagnostics()
            .iter()
            .any(|a| a.anomaly_type == AnomalyType::ArrangementRepetition),
        "7 小节成不了 4 小节段，不得报 arrangement_repetition"
    );
}
