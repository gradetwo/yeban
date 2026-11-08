//! 端到端口径判据：只走 `yeban-dsp` 的公开面。
//!
//! 每个断言都有独立可复算的期望值（手算见各测试注释），不只断言“跑通”：
//! 单频正弦的频段能量分布、静音的 LUFS、固定谱面的张力值、粗糙度与五度圈
//! 的解析值、聚类 Onset 的精确计数、双边滤波的解析平滑值。
//!
//! 规范锚点：设计稿 `~/workspace/yeban-songmap-design.md` §3–§6、§8.2。

use yeban_dsp::song_overview::{
    AnomalyType, AudioInput, BAND_DB_FLOOR, NO_LOUDNESS_READING_LUFS, NoteEvent, ScoreInput,
    SectionMarker, analyze_song_overview, bilateral_smooth, circle_of_fifths_distance,
    roughness_of_midis,
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
    // L̂ ≈ 0.926（代理 −10.08 LUFS），D̂ 拍均 ≈ 0.187，P̂ = 0.5，
    // Ĥ ≈ 0，Ô = 0.0625 → 张力 ≈ 0.425（双边滤波后拍均）。
    let notes = vec![note(0.0, 4.0, 60, 100, 0, false)];
    let overview = analyze_song_overview(&score_with(notes), None);
    assert_eq!(overview.song_meta.total_bars, 1);
    let tension = overview.bar_series.tension[0];
    assert!(
        (0.41..0.44).contains(&tension),
        "单音 C4 全音符的张力应约 0.425，实际 {tension}"
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
        },
        SectionMarker {
            bar: 3,
            label: "Chorus".to_string(),
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
    assert_eq!(overview.schema_version, "1.0");
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
/// 目标 < 30 ms（设计稿 §9.1，realfft 口径）；本实现复用树内 `math::fft`（f64），
/// 先测实数再定是否需要 realfft。
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
