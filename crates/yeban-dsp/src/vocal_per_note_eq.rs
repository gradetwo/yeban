//! 逐音符 EQ 粒度：只在指定音符区间内对一个峰值频段做增益（P2-1，vocaledit Phase 2 第 6 项）。
//!
//! 本模块实现 Melodyne 式"选中一个音符、只修它的音色"的最小形态：调用方给
//! Phase 1 精修器（[`crate::vocal_refine`]）的音节结构（[`RefinedNote`]）
//! 加一条"峰值频段增益"（中心频率／增益／Q），本模块只在该音符的
//! `[pre_s, off_s]` 区间内施加这条 EQ，区间外输出与输入逐位相同。
//!
//! # 设计
//!
//! * **滤波器**：复用 [`crate::shaping`] 的 RBJ 峰值 biquad
//!   （`Biquad::peaking`，转置直接 II 型），每条 EQ 一只独立实例。
//!   滤波器**连续运转**：它对整段信号逐样本推进，状态不随区间开关而
//!   复位（复位会在边界处制造相位台阶）；开关只由包络做。
//! * **包络**：每条 EQ 一条升余弦包络：区间外恰好 `0.0`、区间内恰好
//!   `1.0`，两端各一段 [`PerNoteEqConfig::fade_s`] 的升余弦过渡
//!   （默认 5 ms，与 P1-1 的过渡淡入淡出同口径）。包络为 `0.0` 与 `1.0`
//!   的采样点走**分支直通**（`out = dry`／`out = wet`），不经过
//!   `dry * (1 - env) + wet * env` 的乘加 —— 浮点乘加在 `env = 0` 时
//!   仍可能改写 `-0.0` 的符号位，故区间外逐位相同必须靠分支保证，
//!   不能靠"乘 0"。
//! * **无 EQ 即恒等**：EQ 表为空，或某条增益为 `0.0 dB`（构造时跳过，
//!   记进收据），输出就是输入的逐位复制 —— `process` 里没有任何
//!   "默认滤波"分支。
//! * **非有限输入**：湿路入口过 [`crate::math::finite_or_zero`]
//!   （`NaN`／`±∞` 归 `0.0`），与 `shaping` 三节点同口径 —— 递归
//!   biquad 一旦吃进 `NaN` 就永久中毒；干路原样透传，区间外（含
//!   `NaN`）逐位不变。过渡区内的 `NaN` 仍会随干路混入输出（干路
//!   不消毒是逐位相同的前提）；内区（包络恰好 `1.0`）的 `NaN` 被
//!   湿路消毒，不中毒、不扩散。
//!
//! # 离线限定
//!
//! 本模块只用于离线渲染，绝不进入实时回调（与 `vocal_timemap` 同款
//! 离线限定）：`process` 本身零分配（全部状态在构造时建好），但离线
//! 入口 [`apply_per_note_eq`] 在开头分配一次输出缓冲。
//!
//! # 命名
//!
//! 增益以**分贝**（`gain_db`）为唯一输入口径；中心频率以 Hz 计；Q 值
//! 与 RBJ 定义一致（`alpha = sin(w0) / (2Q)`）。`formant_base` 式
//! 调用永不恢复 —— 本模块不做共振峰修正。
//!
//! # Clean-room
//!
//! 本模块是按 PLAN-vocaledit-phase2.md §3 第 6 项的验收描述独立写的
//! DSP（连续 biquad ＋ 包络混音），未打开、未对照、未粘贴任何 OpenTune
//! 代码。OpenTune 本体 AGPL-3.0，禁入 MIT 仓库。
//!
//! # 局限
//!
//! * 只处理单声道 take（人声 take 即单声道）；立体声须逐声道各建一个实例。
//! * 区间取自 [`RefinedNote`] 的 `pre_s..off_s`（整段音节跨度，含辅音
//!   起音）；只对**已锚定**音符生效，未锚定音符的 EQ 条目在构造时拒绝
//!   （调用方必须先排除未锚定音符，与 `refine_note` 的契约一致）。
//! * 多条 EQ 串联（按表内顺序）：后一条滤前一条的混音输出；同音符多
//!   频段即多条目。

use crate::MIN_SAMPLE_RATE;
use crate::math::finite_or_zero;
use crate::shaping::Biquad;
use crate::vocal_refine::RefinedNote;

/// 默认过渡淡入淡出时长（秒）：5 ms，与 P1-1 的过渡淡入淡出同口径，
/// 亦落在业界常用的 5–10 ms 微淡化量级内。
pub const DEFAULT_FADE_S: f32 = 0.005;
/// 过渡淡入淡出时长上限（秒）：超过此值构造时拒绝。
pub const MAX_FADE_S: f32 = 0.1;

/// 逐音符 EQ 的配置：全部阈值与开关均可配置，默认值钉死。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerNoteEqConfig {
    /// 区间两端升余弦过渡时长（秒）。默认 `0.005`（5 ms）。
    /// `0.0` 表示硬开关（边界处有咔哒风险，仅调用方明确要求时用）。
    pub fade_s: f32,
}

impl Default for PerNoteEqConfig {
    fn default() -> Self {
        Self {
            fade_s: DEFAULT_FADE_S,
        }
    }
}

impl PerNoteEqConfig {
    /// 校验配置：非法值直接 panic（调用方错误，大声失败）。
    pub fn validate(&self) {
        assert!(
            self.fade_s.is_finite(),
            "fade_s 必须有限，实得 {}",
            self.fade_s
        );
        assert!(
            self.fade_s >= 0.0 && self.fade_s <= MAX_FADE_S,
            "fade_s 必须在 [0, {MAX_FADE_S}] 秒内，实得 {}",
            self.fade_s
        );
    }
}

/// 一条逐音符 EQ：一个音符区间里的一个峰值频段。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteEqBand {
    /// 目标音符在 `notes` 切片里的下标。
    pub note_index: usize,
    /// 峰值中心频率（Hz），必须在 `(0, sample_rate / 2)` 内。
    pub center_hz: f32,
    /// 增益（dB）。`0.0` 表示本条无操作（构造时跳过，记进收据）。
    pub gain_db: f32,
    /// Q 值（带宽），必须 `> 0`。
    pub q: f32,
}

/// 构造收据：实际建好的 EQ 条目数与跳过的零增益条目数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerNoteEqReceipt {
    /// 实际建好的 EQ 条目数。
    pub applied: usize,
    /// 增益为 `0.0 dB` 而跳过的条目数。
    pub skipped_zero_gain: usize,
}

/// 一条已建好的 EQ：采样点区间＋过渡长度＋滤波器实例。
struct Entry {
    /// 区间起点（采样点，含）。
    start: u64,
    /// 区间终点（采样点，不含）。
    end: u64,
    /// 两端过渡长度（采样点），不超过区间长度的一半。
    fade: usize,
    /// RBJ 峰值 biquad（连续运转，状态不随区间开关复位）。
    filter: Biquad,
}

impl Entry {
    /// 该采样点的包络值：区间外恰好 `0.0`、区间内恰好 `1.0`，
    /// 两端升余弦过渡。
    fn envelope(&self, pos: u64) -> f32 {
        if pos < self.start || pos >= self.end {
            return 0.0;
        }
        let p = (pos - self.start) as usize;
        let len = (self.end - self.start) as usize;
        if self.fade == 0 || (p >= self.fade && p < len - self.fade) {
            return 1.0;
        }
        // 升余弦过渡只用钉死的 libm（不是宿主 libm），故 R77 分类里
        // 本路径是可硬断言类。
        let u = if p < self.fade {
            p as f64 / self.fade as f64
        } else {
            (p - (len - self.fade)) as f64 / self.fade as f64
        };
        let c = libm::cos(std::f64::consts::PI * u);
        if p < self.fade {
            (0.5 - 0.5 * c) as f32
        } else {
            (0.5 + 0.5 * c) as f32
        }
    }
}

/// 逐音符 EQ 处理器：构造时建好全部滤波器与区间，`process` 零分配。
///
/// 单声道。`process` 按 128 帧量子或整段调用均可；位置（[`Self::position`]）
/// 跨调用连续推进，`reset` 后回到 0。
pub struct PerNoteEq {
    /// 采样率（Hz）。
    sample_rate: u32,
    /// 已建好的 EQ 条目（按表内顺序串联）。
    entries: Vec<Entry>,
    /// 已处理采样数（位置，跨调用连续；`reset` 后归零）。
    position: u64,
    /// 累计处理采样数（覆盖度自检用；`reset` **不**清零 —— 累计口径）。
    processed_samples: u64,
    /// 构造收据。
    receipt: PerNoteEqReceipt,
}

impl PerNoteEq {
    /// 构造：校验采样率、配置与每条 EQ，建好滤波器与区间。
    ///
    /// # Panics
    ///
    /// * 采样率低于 [`MIN_SAMPLE_RATE`]；
    /// * 配置非法（见 [`PerNoteEqConfig::validate`]）；
    /// * `note_index` 越界、目标音符未锚定；
    /// * `center_hz` 不在 `(0, sample_rate / 2)` 内、`q <= 0`、
    ///   `gain_db` 非有限。
    pub fn new(
        sample_rate: u32,
        notes: &[RefinedNote],
        bands: &[NoteEqBand],
        config: PerNoteEqConfig,
    ) -> Self {
        assert!(
            sample_rate >= MIN_SAMPLE_RATE as u32,
            "sample_rate {sample_rate} is below {}",
            MIN_SAMPLE_RATE as u32
        );
        config.validate();
        let sr_f64 = f64::from(sample_rate);
        let fade_samples = (f64::from(config.fade_s) * sr_f64).round().max(0.0) as usize;
        let mut entries = Vec::new();
        let mut applied = 0usize;
        let mut skipped_zero_gain = 0usize;
        for band in bands {
            assert!(
                band.note_index < notes.len(),
                "note_index {} 越界（notes 只有 {} 条）",
                band.note_index,
                notes.len()
            );
            let note = &notes[band.note_index];
            assert!(
                note.anchored,
                "note_index {} 的音符未锚定 —— 调用方必须先排除未锚定音符",
                band.note_index
            );
            assert!(
                band.center_hz.is_finite()
                    && band.center_hz > 0.0
                    && band.center_hz < sample_rate as f32 / 2.0,
                "center_hz 必须在 (0, {}) 内，实得 {}",
                sample_rate as f32 / 2.0,
                band.center_hz
            );
            assert!(
                band.q.is_finite() && band.q > 0.0,
                "q 必须有限且 > 0，实得 {}",
                band.q
            );
            assert!(
                band.gain_db.is_finite(),
                "gain_db 必须有限，实得 {}",
                band.gain_db
            );
            if band.gain_db == 0.0 {
                skipped_zero_gain += 1;
                continue;
            }
            let start = (note.pre_s * sr_f64).round().max(0.0) as u64;
            let end = (note.off_s * sr_f64).round().max(0.0) as u64;
            assert!(
                end > start,
                "note_index {} 的区间为空（pre_s {} / off_s {}）",
                band.note_index,
                note.pre_s,
                note.off_s
            );
            let len = (end - start) as usize;
            let fade = fade_samples.min(len / 2);
            let mut filter = Biquad::identity();
            filter.peaking(sample_rate as f32, band.center_hz, band.q, band.gain_db);
            entries.push(Entry {
                start,
                end,
                fade,
                filter,
            });
            applied += 1;
        }
        Self {
            sample_rate,
            entries,
            position: 0,
            processed_samples: 0,
            receipt: PerNoteEqReceipt {
                applied,
                skipped_zero_gain,
            },
        }
    }

    /// 处理一程：`input` 与 `output` 取较短长度，返回处理采样数。
    ///
    /// 零分配：本函数内无 `Vec`／`Box`／堆操作；全部状态在构造时建好。
    /// 位置跨调用连续推进；输出长度不足输入时只处理前 `output.len()` 个采样。
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> usize {
        let n = input.len().min(output.len());
        for i in 0..n {
            let pos = self.position + i as u64;
            let x = input[i];
            let mut y = x;
            for entry in &mut self.entries {
                // 湿路入口过 finite_or_zero（干路原样透传）：递归 biquad
                // 一旦吃进 NaN 就永久中毒；干路的 NaN 在区间外逐位保留。
                let wet = entry.filter.process_sample(finite_or_zero(y));
                let env = entry.envelope(pos);
                if env == 0.0 {
                    // 区间外：分支直通，⛔ 不走乘加（乘 0 会改写 -0.0 的符号位）。
                } else if env == 1.0 {
                    y = wet;
                } else {
                    y = y * (1.0 - env) + wet * env;
                }
            }
            output[i] = y;
        }
        self.position += n as u64;
        self.processed_samples += n as u64;
        n
    }

    /// 复位：位置归零，全部滤波器状态清零（系数保留）。
    /// `processed_samples` 是累计口径，**不**清零。
    pub fn reset(&mut self) {
        for entry in &mut self.entries {
            entry.filter.clear_state();
        }
        self.position = 0;
    }

    /// 采样率（Hz）。
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// 当前位置（已处理采样数，自构造或 `reset` 以来）。
    pub fn position(&self) -> u64 {
        self.position
    }

    /// 累计处理采样数（覆盖度自检用；`reset` 不清零）。
    pub fn processed_samples(&self) -> u64 {
        self.processed_samples
    }

    /// 构造收据（实际建好的条目数／跳过的零增益条目数）。
    pub fn receipt(&self) -> PerNoteEqReceipt {
        self.receipt
    }
}

/// 离线入口：对整段 take 施加逐音符 EQ，返回处理后的全新缓冲与收据。
///
/// 只在开头分配一次输出缓冲；无 EQ 条目（或全为零增益）时输出与输入逐位相同。
pub fn apply_per_note_eq(
    source: &[f32],
    sample_rate: u32,
    notes: &[RefinedNote],
    bands: &[NoteEqBand],
    config: PerNoteEqConfig,
) -> (Vec<f32>, PerNoteEqReceipt) {
    let mut processor = PerNoteEq::new(sample_rate, notes, bands, config);
    let mut out = vec![0.0_f32; source.len()];
    let processed = processor.process(source, &mut out);
    debug_assert_eq!(processed, source.len());
    (out, processor.receipt())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f64::consts::TAU;

    /// 判据用采样率（Hz）。
    const SR: u32 = 48_000;

    /// 两音测试信号：A 音 440 Hz 正弦 0.5–1.0 s，B 音 440 Hz 正弦 1.5–2.0 s，
    /// 幅值 0.5；其余静音。两音同频 —— EQ 若漏到 B 音会立刻现形。
    fn two_note_signal() -> Vec<f32> {
        let mut signal = vec![0.0_f32; SR as usize * 5 / 2];
        for (i, sample) in signal.iter_mut().enumerate() {
            let t = i as f64 / f64::from(SR);
            let in_a = matches!(t, 0.5..1.0);
            let in_b = matches!(t, 1.5..2.0);
            if in_a || in_b {
                let local = if in_a { t - 0.5 } else { t - 1.5 };
                *sample = (0.5 * (TAU * 440.0 * local).sin()) as f32;
            }
        }
        signal
    }

    /// 两音的精修音节结构（与信号同界）。
    fn two_notes() -> Vec<RefinedNote> {
        vec![
            RefinedNote {
                pre_s: 0.5,
                vop_s: 0.55,
                off_s: 1.0,
                anchored: true,
                confidence: 0.9,
                f0_median_hz: 440.0,
                hnr_db: 10.0,
            },
            RefinedNote {
                pre_s: 1.5,
                vop_s: 1.55,
                off_s: 2.0,
                anchored: true,
                confidence: 0.9,
                f0_median_hz: 440.0,
                hnr_db: 10.0,
            },
        ]
    }

    /// 区间 RMS（线性）。
    fn rms(signal: &[f32], range: core::ops::Range<usize>) -> f64 {
        let slice = &signal[range];
        let mut sum = 0.0_f64;
        for sample in slice {
            let v = f64::from(*sample);
            sum += v * v;
        }
        (sum / slice.len() as f64).sqrt()
    }

    /// 两信号在同一区间的 RMS 比（dB）：`20·log10(rms_a / rms_b)`。
    fn rms_ratio_db(a: &[f32], b: &[f32], range: core::ops::Range<usize>) -> f32 {
        let ra = rms(a, range.clone());
        let rb = rms(b, range);
        (20.0 * (ra / rb).log10()) as f32
    }

    /// 逐位差异数（`to_bits` 比较；`-0.0` 与 `0.0` 算不同）。
    fn differing(a: &[f32], b: &[f32]) -> usize {
        assert_eq!(a.len(), b.len(), "夹具前提：两缓冲等长");
        let mut n = 0usize;
        for (x, y) in a.iter().zip(b.iter()) {
            if x.to_bits() != y.to_bits() {
                n += 1;
            }
        }
        n
    }

    /// 量什么：两音测试（验收门）—— A 音 440 Hz 衰减 -12 dB 只在 A 音区间，
    /// B 音区间频响不变（±1 dB）且逐位相同。
    ///
    /// 判据：A 音内区 `[0.6, 0.9)` 实测 -12 ± 1 dB；B 音内区 `[1.6, 1.9)`
    /// 实测 0 ± 1 dB 且逐位差异数为 0。内区避开 5 ms 过渡与滤波器建立
    /// （Q=1、440 Hz 的时间常数约 0.7 ms）。
    #[test]
    fn attenuation_applies_only_inside_the_target_note() {
        let src = two_note_signal();
        let notes = two_notes();
        let bands = [NoteEqBand {
            note_index: 0,
            center_hz: 440.0,
            gain_db: -12.0,
            q: 1.0,
        }];
        let (out, receipt) =
            apply_per_note_eq(&src, SR, &notes, &bands, PerNoteEqConfig::default());
        assert_eq!(receipt.applied, 1);
        assert_eq!(receipt.skipped_zero_gain, 0);
        let db_a = rms_ratio_db(&out, &src, 28_800..43_200);
        assert!(
            (db_a - (-12.0)).abs() <= 1.0,
            "A 音内区实测 {db_a} dB，须为 -12 ± 1 dB"
        );
        let db_b = rms_ratio_db(&out, &src, 76_800..91_200);
        assert!(db_b.abs() <= 1.0, "B 音内区实测 {db_b} dB，须为 0 ± 1 dB");
        assert_eq!(
            differing(&out[76_800..91_200], &src[76_800..91_200]),
            0,
            "B 音内区必须逐位相同（包络恰好为 0 ⇒ 分支直通）"
        );
        eprintln!(
            "[yeban-dsp/P2-1] 两音验收：A 内区 {db_a:.3} dB（目标 -12 ± 1）、B 内区 {db_b:.3} dB（目标 0 ± 1）、B 不同采样 0"
        );
    }

    /// 量什么：无 EQ 条目时输出与输入逐位相同（验收门）。
    #[test]
    fn no_bands_output_is_bit_identical() {
        let src = two_note_signal();
        let (out, receipt) =
            apply_per_note_eq(&src, SR, &two_notes(), &[], PerNoteEqConfig::default());
        assert_eq!(receipt.applied, 0);
        assert_eq!(receipt.skipped_zero_gain, 0);
        assert_eq!(differing(&out, &src), 0, "无 EQ 条目 ⇒ process 是逐位复制");
    }

    /// 量什么：增益为 `0.0 dB` 的条目在构造时跳过（记进收据），输出仍逐位相同。
    #[test]
    fn zero_gain_band_is_skipped_and_identity() {
        let src = two_note_signal();
        let bands = [NoteEqBand {
            note_index: 0,
            center_hz: 440.0,
            gain_db: 0.0,
            q: 1.0,
        }];
        let (out, receipt) =
            apply_per_note_eq(&src, SR, &two_notes(), &bands, PerNoteEqConfig::default());
        assert_eq!(receipt.applied, 0);
        assert_eq!(receipt.skipped_zero_gain, 1);
        assert_eq!(differing(&out, &src), 0, "零增益条目被跳过 ⇒ 无滤波分支");
    }

    /// 量什么：包络端点恰好为 0／1（直接测 `Entry::envelope`）。
    ///
    /// 判据：区间外、区间首采样、区间末采样之后均为 `0.0`（逐位）；
    /// 过渡结束与区间中部均为 `1.0`（逐位）；淡入／淡出中点约为 `0.5`
    /// （升余弦对称点，`cos(π/2)` 的浮点余量内）。
    #[test]
    fn envelope_is_exactly_zero_outside_and_one_inside() {
        let entry = Entry {
            start: 1000,
            end: 2000,
            fade: 240,
            filter: Biquad::identity(),
        };
        assert_eq!(entry.envelope(999).to_bits(), 0.0f32.to_bits());
        assert_eq!(entry.envelope(1000).to_bits(), 0.0f32.to_bits());
        assert_eq!(entry.envelope(2000).to_bits(), 0.0f32.to_bits());
        assert_eq!(entry.envelope(1240).to_bits(), 1.0f32.to_bits());
        assert_eq!(entry.envelope(1500).to_bits(), 1.0f32.to_bits());
        assert!(
            (entry.envelope(1120) - 0.5).abs() < 1e-6,
            "淡入中点须约为 0.5，实得 {}",
            entry.envelope(1120)
        );
        assert!(
            (entry.envelope(1880) - 0.5).abs() < 1e-6,
            "淡出中点须约为 0.5，实得 {}",
            entry.envelope(1880)
        );
    }

    /// 量什么：区间边界采样逐位等于干声（包络恰好为 0 ⇒ 分支直通）。
    ///
    /// 夹具：437 Hz 正弦 0.4–1.1 s（区间起点 0.5 s 处 local 相位 43.7 周期，
    /// 采样值约 -0.475，非零 —— 零值夹具测不出"直通还是乘 0"）。
    #[test]
    fn interval_edges_are_exactly_dry() {
        let mut signal = vec![0.0_f32; SR as usize * 5 / 2];
        for (i, sample) in signal.iter_mut().enumerate() {
            let t = i as f64 / f64::from(SR);
            if matches!(t, 0.4..1.1) {
                *sample = (0.5 * (TAU * 437.0 * (t - 0.4)).sin()) as f32;
            }
        }
        assert!(
            signal[24_000].abs() > 0.1,
            "夹具前提：区间起点采样必须非零，实得 {}",
            signal[24_000]
        );
        let notes = two_notes();
        let bands = [NoteEqBand {
            note_index: 0,
            center_hz: 437.0,
            gain_db: -12.0,
            q: 1.0,
        }];
        let (out, _) = apply_per_note_eq(&signal, SR, &notes, &bands, PerNoteEqConfig::default());
        assert_eq!(
            out[24_000].to_bits(),
            signal[24_000].to_bits(),
            "区间首采样包络为 0 ⇒ 干声逐位相同"
        );
        assert_eq!(
            out[23_999].to_bits(),
            signal[23_999].to_bits(),
            "区间前一采样干声逐位相同"
        );
        assert_eq!(
            out[48_000].to_bits(),
            signal[48_000].to_bits(),
            "区间末采样之后包络为 0 ⇒ 干声逐位相同"
        );
    }

    /// 量什么：同一输入两次运行逐位相同；`reset` 后重跑与首遍逐位相同；
    /// `processed_samples` 为累计口径（`reset` 不清零）。
    #[test]
    fn process_is_deterministic_and_reset_restores() {
        let src = two_note_signal();
        let notes = two_notes();
        let bands = [NoteEqBand {
            note_index: 0,
            center_hz: 440.0,
            gain_db: -12.0,
            q: 1.0,
        }];
        let (run1, _) = apply_per_note_eq(&src, SR, &notes, &bands, PerNoteEqConfig::default());
        let (run2, _) = apply_per_note_eq(&src, SR, &notes, &bands, PerNoteEqConfig::default());
        assert_eq!(differing(&run1, &run2), 0, "同一输入两次运行须逐位相同");
        let mut processor = PerNoteEq::new(SR, &notes, &bands, PerNoteEqConfig::default());
        let mut first = vec![0.0_f32; src.len()];
        let mut second = vec![0.0_f32; src.len()];
        let n1 = processor.process(&src, &mut first);
        processor.reset();
        let n2 = processor.process(&src, &mut second);
        assert_eq!(n1, src.len());
        assert_eq!(n2, src.len());
        assert_eq!(
            differing(&first, &second),
            0,
            "reset 后重跑须与首遍逐位相同"
        );
        assert_eq!(
            processor.processed_samples(),
            (src.len() * 2) as u64,
            "processed_samples 为累计口径"
        );
        assert_eq!(processor.position(), src.len() as u64);
    }

    /// 量什么：默认配置钉死淡入淡出 5 ms（位型比较）。
    #[test]
    fn default_config_pins_the_fade() {
        assert_eq!(
            PerNoteEqConfig::default().fade_s.to_bits(),
            0.005f32.to_bits()
        );
    }

    /// 量什么：收据如实计数（两条不同音符的 EQ 各一条 ⇒ applied 为 2）。
    #[test]
    fn receipt_counts_applied_bands() {
        let bands = [
            NoteEqBand {
                note_index: 0,
                center_hz: 440.0,
                gain_db: -12.0,
                q: 1.0,
            },
            NoteEqBand {
                note_index: 1,
                center_hz: 440.0,
                gain_db: -6.0,
                q: 2.0,
            },
        ];
        let (_, receipt) = apply_per_note_eq(
            &two_note_signal(),
            SR,
            &two_notes(),
            &bands,
            PerNoteEqConfig::default(),
        );
        assert_eq!(receipt.applied, 2);
        assert_eq!(receipt.skipped_zero_gain, 0);
    }

    /// 量什么：内区 `NaN` 爆发不毒化滤波器状态（与 `shaping` 的
    /// "该样本换成 `0.0` 的对照运行逐位相同"同口径）。
    ///
    /// 判据：输出处处有限；与"把 `NaN` 换成 `0.0` 的对照运行"逐位相同。
    /// `NaN` 爆发放在 A 音内区 `[0.7, 0.8)`（包络恰好 `1.0`，干路不参与）。
    #[test]
    fn nan_inside_the_interval_does_not_poison_state() {
        let mut src = two_note_signal();
        for sample in &mut src[33_600..38_400] {
            *sample = f32::NAN;
        }
        let notes = two_notes();
        let bands = [NoteEqBand {
            note_index: 0,
            center_hz: 440.0,
            gain_db: -12.0,
            q: 1.0,
        }];
        let (out, _) = apply_per_note_eq(&src, SR, &notes, &bands, PerNoteEqConfig::default());
        let mut non_finite = 0usize;
        for sample in &out {
            if !sample.is_finite() {
                non_finite += 1;
            }
        }
        assert_eq!(non_finite, 0, "内区 NaN 经湿路消毒后输出须处处有限");
        let mut clean = two_note_signal();
        for sample in &mut clean[33_600..38_400] {
            *sample = 0.0;
        }
        let (clean_out, _) =
            apply_per_note_eq(&clean, SR, &notes, &bands, PerNoteEqConfig::default());
        assert_eq!(
            differing(&out, &clean_out),
            0,
            "NaN 运行须与'该样本换成 0.0'的对照运行逐位相同"
        );
    }

    /// 量什么：非法输入大声失败（panic），不静默改写。
    #[test]
    #[should_panic(expected = "sample_rate 0 is below")]
    fn sample_rate_below_minimum_panics() {
        let _ = PerNoteEq::new(0, &two_notes(), &[], PerNoteEqConfig::default());
    }

    /// 量什么：中心频率为 0 时 panic。
    #[test]
    #[should_panic(expected = "center_hz 必须在")]
    fn center_at_zero_panics() {
        let bands = [NoteEqBand {
            note_index: 0,
            center_hz: 0.0,
            gain_db: -12.0,
            q: 1.0,
        }];
        let _ = PerNoteEq::new(SR, &two_notes(), &bands, PerNoteEqConfig::default());
    }

    /// 量什么：中心频率在 Nyquist（`sr / 2`）时 panic。
    #[test]
    #[should_panic(expected = "center_hz 必须在")]
    fn center_at_nyquist_panics() {
        let bands = [NoteEqBand {
            note_index: 0,
            center_hz: 24_000.0,
            gain_db: -12.0,
            q: 1.0,
        }];
        let _ = PerNoteEq::new(SR, &two_notes(), &bands, PerNoteEqConfig::default());
    }

    /// 量什么：`q <= 0` 时 panic。
    #[test]
    #[should_panic(expected = "q 必须有限且 > 0")]
    fn nonpositive_q_panics() {
        let bands = [NoteEqBand {
            note_index: 0,
            center_hz: 440.0,
            gain_db: -12.0,
            q: 0.0,
        }];
        let _ = PerNoteEq::new(SR, &two_notes(), &bands, PerNoteEqConfig::default());
    }

    /// 量什么：`note_index` 越界时 panic。
    #[test]
    #[should_panic(expected = "note_index 2 越界")]
    fn note_index_out_of_range_panics() {
        let bands = [NoteEqBand {
            note_index: 2,
            center_hz: 440.0,
            gain_db: -12.0,
            q: 1.0,
        }];
        let _ = PerNoteEq::new(SR, &two_notes(), &bands, PerNoteEqConfig::default());
    }

    /// 量什么：目标音符未锚定时 panic（调用方必须先排除未锚定音符）。
    #[test]
    #[should_panic(expected = "未锚定")]
    fn unanchored_note_panics() {
        let mut notes = two_notes();
        notes[0].anchored = false;
        let bands = [NoteEqBand {
            note_index: 0,
            center_hz: 440.0,
            gain_db: -12.0,
            q: 1.0,
        }];
        let _ = PerNoteEq::new(SR, &notes, &bands, PerNoteEqConfig::default());
    }

    /// 量什么：`fade_s` 超出 `[0, 0.1]` 时 panic。
    #[test]
    #[should_panic(expected = "fade_s 必须在")]
    fn fade_out_of_range_panics() {
        let config = PerNoteEqConfig { fade_s: 0.2 };
        let _ = PerNoteEq::new(SR, &two_notes(), &[], config);
    }
}
