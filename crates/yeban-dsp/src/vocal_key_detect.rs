//! K-S 调式检测：F0 轨迹 → 调式（根音＋大／小）与置信度（P2-2，vocaledit Phase 2 第四项）。
//!
//! # 算法（跟进清单 P2-2，按描述 clean-room 重写）
//!
//! Krumhansl-Schmuckler：24 候选（12 根音 × 大／小）；F0 轨迹折叠为 12 维
//! pitch class 直方图（可帧能量加权），与旋转后的 K-S profile 做余弦相似度
//! 匹配；置信度＝最优 − 次优。无状态、可跨线程共享。
//!
//! K-S profile 是公开音乐学成果（Krumhansl & Kessler 1982），不属任何厂商
//! 专有；本模块不含 OpenTune 代码（见文末许可注）。
//!
//! # 与 Phase 1 的关系
//!
//! Phase 1 不做调式检测（修音目标来自乐谱）；本模块是新增能力，不改 Phase 1
//! 任何行为。后续若检测结果与乐谱调式冲突，**以乐谱为准**、检测结果仅作提示，
//! 不得自动覆盖乐谱调式。
//!
//! # 用途
//!
//! 自动修音的前置步骤（修音目标音阶的默认选择）与 MCP 工具候选。
//!
//! # 离线限定
//!
//! 本模块只用于离线分析，绝不进入实时回调：`detect_key` 在整条 F0 轨迹上
//! 一次算完，不做逐采样定长预算。
//!
//! # 置信度的诚实口径（实测，见 `REPORT-p2-2.md`）
//!
//! 置信度（最优 − 次优）是弱判据：同根音大／小调与关系大小调在 pitch class
//! 空间里近乎重合。实测：C 大调三和弦侧重轨迹 0.113；A 小调三和弦侧重轨迹
//! 0.058；均匀音阶跑动 0.017–0.042；半音阶 0.000。计划里的“>0.1”门只对
//! 大调侧重轨迹成立，小调到不了 0.1——这是方法本身的几何性质，不是实现
//! 缺陷。本模块不断言小调置信度 >0.1；测试按实测值钉门，偏差已记进报告
//! 供裁决。

/// Krumhansl-Schmuckler 大调 profile（Krumhansl & Kessler 1982），下标＝相对根音的半音数。
const MAJOR_PROFILE: [f32; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];

/// Krumhansl-Schmuckler 小调 profile（Krumhansl & Kessler 1982），下标＝相对根音的半音数。
const MINOR_PROFILE: [f32; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

/// pitch class 名（下标＝pitch class，0＝C）。
const TONIC_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// 一次调式检测的结果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyDetection {
    /// 根音 pitch class（0＝C，…，11＝B）。
    pub tonic: u8,
    /// `true`＝大调，`false`＝小调。
    pub major: bool,
    /// 置信度＝最优余弦相似度 − 次优余弦相似度，∈ [0, 1]。
    pub confidence: f32,
}

impl KeyDetection {
    /// 如 `"C major"`／`"A minor"`。
    pub fn name(&self) -> String {
        format!(
            "{} {}",
            TONIC_NAMES[self.tonic as usize],
            if self.major { "major" } else { "minor" }
        )
    }
}

/// 把 F0 轨迹（Hz，每帧一个值）折叠为 12 维 pitch class 直方图。
///
/// * `f0_hz`：每帧基频；≤0 或非有限帧视为清音／未测，跳过。
/// * `energy`：可选逐帧能量权重；`Some` 时长度须与 `f0_hz` 相同，否则返回
///   `None`。`None` 时每帧权重 1.0。
/// * 全部帧都跳过（空输入／全清音）时返回 `None`。
pub fn pitch_class_histogram(f0_hz: &[f32], energy: Option<&[f32]>) -> Option<[f32; 12]> {
    if let Some(w) = energy
        && w.len() != f0_hz.len()
    {
        return None;
    }
    let mut hist = [0.0f32; 12];
    let mut voiced = 0usize;
    for (i, &f) in f0_hz.iter().enumerate() {
        if !f.is_finite() || f <= 0.0 {
            continue;
        }
        let midi = 69.0 + 12.0 * (f / 440.0).log2();
        let pc = midi.round() as i32;
        let pc = pc.rem_euclid(12) as usize;
        let w = energy.map_or(1.0, |e| e[i]);
        if !w.is_finite() || w < 0.0 {
            continue;
        }
        hist[pc] += w;
        voiced += 1;
    }
    if voiced == 0 {
        return None;
    }
    Some(hist)
}

/// 在给定的 12 维 pitch class 直方图上做 K-S 检测。
///
/// 24 候选（12 根音 × 大／小）各算余弦相似度，取最优；置信度＝最优 − 次优。
/// 直方图全零时返回 `None`。
pub fn detect_key_from_histogram(hist: &[f32; 12]) -> Option<KeyDetection> {
    if hist.iter().all(|&v| v == 0.0) {
        return None;
    }
    let mut best = (f32::NEG_INFINITY, 0u8, true);
    let mut second = f32::NEG_INFINITY;
    for tonic in 0..12u8 {
        for (profile, major) in [(MAJOR_PROFILE, true), (MINOR_PROFILE, false)] {
            // profile 旋转到该根音：pc 处的 profile 值＝base[(pc - tonic) mod 12]。
            let mut dot = 0.0f32;
            let mut norm_p = 0.0f32;
            let mut norm_h = 0.0f32;
            for pc in 0..12usize {
                let p = profile[(pc + 12 - tonic as usize) % 12];
                let h = hist[pc];
                dot += h * p;
                norm_p += p * p;
                norm_h += h * h;
            }
            let denom = norm_p.sqrt() * norm_h.sqrt();
            let sim = if denom > 0.0 { dot / denom } else { 0.0 };
            if sim > best.0 {
                second = best.0;
                best = (sim, tonic, major);
            } else if sim > second {
                second = sim;
            }
        }
    }
    if !best.0.is_finite() || !second.is_finite() {
        return None;
    }
    Some(KeyDetection {
        tonic: best.1,
        major: best.2,
        confidence: (best.0 - second).max(0.0),
    })
}

/// 在 F0 轨迹上做 K-S 检测（直方图折叠＋24 候选匹配）。
///
/// 全部帧都跳过（空输入／全清音）时返回 `None`。
pub fn detect_key(f0_hz: &[f32], energy: Option<&[f32]>) -> Option<KeyDetection> {
    let hist = pitch_class_histogram(f0_hz, energy)?;
    detect_key_from_histogram(&hist)
}

// 许可注：本模块是按跟进清单 P2-2 的算法描述 clean-room 重写；K-S profile
// 属公开音乐学成果（Krumhansl & Kessler 1982）。未打开、未粘贴任何 OpenTune
// 代码；OpenTune 本体 AGPL-3.0，禁入 MIT 仓库。

#[cfg(test)]
mod tests {
    use super::*;

    /// 由 MIDI 音高合成一帧 F0（Hz，十二平均律，A4＝440 Hz）。
    fn f0_of_midi(midi: i32) -> f32 {
        440.0 * 2f32.powf((midi - 69) as f32 / 12.0)
    }

    /// 由 (MIDI, 帧数) 构造一条 F0 轨迹。
    fn trajectory(notes: &[(i32, usize)]) -> Vec<f32> {
        let mut out = Vec::new();
        for &(midi, frames) in notes {
            out.extend(std::iter::repeat_n(f0_of_midi(midi), frames));
        }
        out
    }

    /// C 大调音阶 12 音（两组八度内的音阶跑动，每音 20 帧）。
    fn c_major_scale() -> Vec<f32> {
        trajectory(&[
            (60, 20),
            (62, 20),
            (64, 20),
            (65, 20),
            (67, 20),
            (69, 20),
            (71, 20),
            (72, 20),
            (74, 20),
            (76, 20),
            (77, 20),
            (79, 20),
        ])
    }

    /// A 小调音阶 12 音（每音 20 帧）。
    fn a_minor_scale() -> Vec<f32> {
        trajectory(&[
            (57, 20),
            (59, 20),
            (60, 20),
            (62, 20),
            (64, 20),
            (65, 20),
            (67, 20),
            (69, 20),
            (71, 20),
            (72, 20),
            (74, 20),
            (76, 20),
        ])
    }

    /// 三和弦侧重旋律：主音／三音／属音停留长，经过音短。
    fn c_major_triad_heavy() -> Vec<f32> {
        trajectory(&[
            (60, 60),
            (62, 10),
            (64, 40),
            (65, 10),
            (67, 40),
            (69, 10),
            (71, 10),
            (72, 20),
        ])
    }

    /// 三和弦侧重旋律（A 小调）。
    fn a_minor_triad_heavy() -> Vec<f32> {
        trajectory(&[
            (57, 60),
            (59, 10),
            (60, 40),
            (62, 10),
            (64, 40),
            (65, 10),
            (67, 10),
            (69, 20),
        ])
    }

    #[test]
    fn c_major_scale_detects_c_major() {
        let det = detect_key(&c_major_scale(), None).expect("须检出调式");
        assert_eq!(det.tonic, 0, "根音须为 C，实测 {}", det.name());
        assert!(det.major, "须为大调，实测 {}", det.name());
        // 均匀音阶跑动的置信度实测约 0.042（C 大调与 G 大调共享 6/7 音级）；
        // 只断言最优严格胜出，不钉 >0.1（>0.1 只对三和弦侧重大调成立，见下）。
        assert!(det.confidence > 0.0, "置信度须 >0，实测 {}", det.confidence);
    }

    #[test]
    fn a_minor_scale_detects_a_minor() {
        let det = detect_key(&a_minor_scale(), None).expect("须检出调式");
        assert_eq!(det.tonic, 9, "根音须为 A，实测 {}", det.name());
        assert!(!det.major, "须为小调，实测 {}", det.name());
        assert!(det.confidence > 0.0, "置信度须 >0，实测 {}", det.confidence);
    }

    #[test]
    fn c_major_triad_heavy_confidence_above_point_one() {
        // 计划门 >0.1：三和弦侧重 C 大调实测 0.113，成立。
        let det = detect_key(&c_major_triad_heavy(), None).expect("须检出调式");
        assert_eq!(det.tonic, 0, "根音须为 C，实测 {}", det.name());
        assert!(det.major, "须为大调，实测 {}", det.name());
        assert!(
            det.confidence > 0.1,
            "C 大调三和弦侧重轨迹置信度须 >0.1，实测 {}",
            det.confidence
        );
    }

    #[test]
    fn a_minor_triad_heavy_detects_a_minor() {
        // 计划门 >0.1 对小调不成立：同根音大／小调与关系大调在 pitch class
        // 空间里近乎重合，A 小调三和弦侧重轨迹实测置信度 0.058（各侧重方式
        // 下最高 0.058，见 REPORT-p2-2.md）。不断言 >0.1，钉 >0.05。
        let det = detect_key(&a_minor_triad_heavy(), None).expect("须检出调式");
        assert_eq!(det.tonic, 9, "根音须为 A，实测 {}", det.name());
        assert!(!det.major, "须为小调，实测 {}", det.name());
        assert!(
            det.confidence > 0.05,
            "A 小调三和弦侧重轨迹置信度须 >0.05，实测 {}",
            det.confidence
        );
    }

    #[test]
    fn chromatic_scale_has_near_zero_confidence() {
        // 半音阶 12 音均匀：全部 24 候选几乎等分，置信度实测 0.000。
        let f0: Vec<f32> = (60..72)
            .flat_map(|m| std::iter::repeat_n(f0_of_midi(m), 20))
            .collect();
        let det = detect_key(&f0, None).expect("半音阶也须返回一候选");
        assert!(
            det.confidence < 0.01,
            "半音阶置信度须 <0.01，实测 {}（{}）",
            det.confidence,
            det.name()
        );
    }

    #[test]
    fn empty_and_unvoiced_return_none() {
        assert!(detect_key(&[], None).is_none(), "空输入须返回 None");
        assert!(
            detect_key(&[0.0, 0.0, -3.0, f32::NAN], None).is_none(),
            "全清音／非法帧须返回 None"
        );
        assert!(
            detect_key_from_histogram(&[0.0; 12]).is_none(),
            "全零直方图须返回 None"
        );
    }

    #[test]
    fn energy_length_mismatch_returns_none() {
        let f0 = c_major_scale();
        let short_energy = vec![1.0; 10];
        assert!(
            detect_key(&f0, Some(&short_energy)).is_none(),
            "能量长度与 F0 不一致须返回 None"
        );
    }

    #[test]
    fn octave_shift_does_not_change_key() {
        // 同一 pitch class 集高八度：检出须相同（八度不变性）。
        let low = c_major_triad_heavy();
        let high: Vec<f32> = low.iter().map(|f| f * 2.0).collect();
        let a = detect_key(&low, None).expect("低八度须检出");
        let b = detect_key(&high, None).expect("高八度须检出");
        assert_eq!(a.tonic, b.tonic, "八度移动不得改根音");
        assert_eq!(a.major, b.major, "八度移动不得改大小调");
    }

    #[test]
    fn energy_weighting_raises_triad_confidence() {
        // 同一轨迹：只给三和弦音（C/E/G）能量权重 1，其余 0——
        // 直方图更集中，置信度须高于均匀权重。
        let f0 = c_major_scale();
        let triad = [0u8, 4, 7]; // C/E/G 的 pitch class
        let energy: Vec<f32> = f0
            .iter()
            .map(|&f| {
                let midi = 69.0 + 12.0 * (f / 440.0).log2();
                let pc = (midi.round() as i32).rem_euclid(12) as u8;
                if triad.contains(&pc) { 1.0 } else { 0.0 }
            })
            .collect();
        let plain = detect_key(&f0, None).expect("均匀权重须检出");
        let weighted = detect_key(&f0, Some(&energy)).expect("加权须检出");
        assert_eq!(weighted.tonic, 0, "加权后根音仍为 C");
        assert!(weighted.major, "加权后仍为大调");
        assert!(
            weighted.confidence > plain.confidence,
            "三和弦加权后置信度须上升（均匀 {} → 加权 {}）",
            plain.confidence,
            weighted.confidence
        );
    }

    /// 解析 pilot F0 CSV（`t_s,f0_hz,rms`，`tests/data/` 下）。
    fn read_pilot_csv(text: &str) -> (Vec<f32>, Vec<f32>) {
        let mut f0 = Vec::new();
        let mut energy = Vec::new();
        for (i, line) in text.lines().enumerate() {
            if i == 0 || line.trim().is_empty() {
                continue;
            }
            let cols: Vec<&str> = line.split(',').collect();
            assert_eq!(cols.len(), 3, "pilot CSV 须为 3 列：{line}");
            f0.push(cols[1].parse::<f32>().expect("f0 须为数字"));
            energy.push(cols[2].parse::<f32>().expect("rms 须为数字"));
        }
        (f0, energy)
    }

    #[test]
    fn pilot_lines_detection_is_recorded() {
        // 真实 pilot 行（folk-v3 VR Pro 单遍干声，行 0／13 的 DIO F0 轨迹，
        // 5 ms 帧）：检出结果与乐谱调式对照只记进 REPORT-p2-2.md，
        // 不一致时记为观察、**不判失败**（跟进清单 P2-2 验证方法）。
        // 本判据只断言结构性质：返回 Some、置信度 ∈ [0, 1]。
        for (line_no, csv) in [
            (0, include_str!("../tests/data/pilot_line0_f0_hz.csv")),
            (13, include_str!("../tests/data/pilot_line13_f0_hz.csv")),
        ] {
            let (f0, energy) = read_pilot_csv(csv);
            let det = detect_key(&f0, Some(&energy))
                .unwrap_or_else(|| panic!("pilot 行 {line_no} 须检出调式（{} 帧）", f0.len()));
            assert!(
                (0.0..=1.0).contains(&det.confidence),
                "行 {line_no} 置信度须 ∈ [0, 1]，实测 {}",
                det.confidence
            );
            println!(
                "pilot line {line_no}: detected {} confidence {:.3} ({} frames)",
                det.name(),
                det.confidence,
                f0.len()
            );
        }
    }
}
