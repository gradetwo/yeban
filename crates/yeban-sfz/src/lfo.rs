//! ⭐ **LFO 三族 + 可分配 LFO**（`amplfo_*` / `fillfo_*` / `pitchlfo_*` 与
//! `lfoN_*` 目标系；SFZ LFO 片，审计 C21／C22）。
//!
//! # 规范值（2026-11-15 逐页亲核 <https://sfzformat.com>）
//!
//! v1 三族形状相同，只有目标与 `depth` 单位／域不同：
//!
//! | opcode | Type | Default | Range | Unit |
//! | :--- | :--- | :--- | :--- | :--- |
//! | `*_delay` | float | 0 | 0..100 | seconds |
//! | `*_fade` | float | 0 | 0..100 | seconds |
//! | `*_freq` | float | 0 | 0..20 | Hz |
//! | `*_freqccN` / `*_freqchanaft` / `*_freqpolyaft` | float | 0 | -200..200 | Hz |
//! | `amplfo_depth` (+cc/aft 伴随) | float | 0 | -10..10 | dB |
//! | `fillfo_depth` / `pitchlfo_depth` (+cc/aft 伴随) | float | 0 | -1200..1200 | cents |
//!
//! 出处：<https://sfzformat.com/modulations/lfo/> 与各 opcode 页
//! （如 `amplfo_freq`、`amplfo_depth`、`pitchlfo_depth`、`fillfo_depth`）。
//! `*_depth_onccN` 是 `*_depthccN` 的 SFZ v2 别名（opcodes 索引页同行登记），
//! 本片把两者归到同一个 `(cc)` 键上（与 `ccN`/`_onccN` 别名同口径）。
//!
//! v2 可分配 LFO（`lfoN_*`）：
//!
//! | opcode | Type | Default | Range | Unit |
//! | :--- | :--- | :--- | :--- | :--- |
//! | `lfoN_wave` | integer | 1 | （见 [`LfoWave`]） | N/A |
//! | `lfoN_freq` | float | N/A | N/A | Hz |
//! | `lfoN_delay` | float | 0 | N/A | seconds |
//! | `lfoN_fade` | float | N/A | N/A | seconds |
//! | `lfoN_phase` | float | 0 | 0..1 | cycle |
//! | `lfoN_count` | integer | N/A | N/A | repetitions |
//! | `lfoN_steps` / `lfoN_stepX` | int / float | N/A | stepX: -100..100 | % |
//! | `lfoN_pitch` | — | N/A | N/A | cents（页示例 `lfo01_pitch=100` ＝ 100 音分） |
//! | `lfoN_cutoff` / `lfoN_resonance` / `lfoN_volume` | — | N/A | N/A | N/A（见下） |
//!
//! # 拿不准、不发明的语义（本片裁决）
//!
//! * **`lfoN_cutoff` / `lfoN_resonance` / `lfoN_volume` 的单位与域，规格页
//!   只写 N/A**。本片按同名静态 opcode 的单位类比消费：`cutoff` ＝音分偏移
//!   （与 `cutoff_ccN` 同单位）、`resonance` ＝ dB（与 `resonance` 同单位）、
//!   `volume` ＝ dB（与 `volume` 同单位）；解析层**不钳位**（只验有限），
//!   因为规格没有给出域，写一个域出来就是发明语义。
//! * **`lfoN_amplitude` 不做**：单位同样是 N/A，而 `amplitude` 静态 opcode
//!   是线性百分比，LFO 偏移到底是「百分点」还是「dB 式」无从类比 ⇒ 留待
//!   裁决，解析层仍按未知 opcode 忽略（与审计 C22 的其余目标分开登记）。
//! * **v1 波形**：SFZ v1 没有波形 opcode。ARIA 文档记 SFZ1 LFO 在 ARIA 里
//!   缺省 sine、其它播放器未测（`lfoN_wave` 页 Practical 注记）⇒ 本片 v1
//!   取 **sine**，并在此登记这是播放器口径、不是规格明文。
//! * **`*_chanaft` / `*_polyaft` 与 `lfoN_*_smoothccX` / `*_stepccX`**：
//!   解析层**收下并存值**（chanaft/polyaft 存标量、cc 存调制项），但本仓
//!   没有 aftertouch 探针、也没有 smooth/step 的 CC 平滑状态（与 D10 的
//!   `*_smoothccN` 不识别口径不同：那是不收，本片是收下不消费）⇒ 引擎
//!   求值只消费 `cc` 探针那一支，aft 两支是**已解析未消费**（B 类），
//!   ⛔ 不假装已生效。`smoothcc/stepcc` 伴随**不收**（名字识别直接跳过）。
//! * **`lfoN_freq_lfoX` / `lfoN_depth_lfoX`（LFO 调 LFO）不做**：反馈环的
//!   求值序规格只给了 Cakewalk 的简化注记，本片不发明 ⇒ 按未知忽略。
//! * **`lfoN_count` 走完后输出归 0**（偏移消失）：规格只说「repetitions
//!   before the LFO stops」，停在最后电平还是归零未明 ⇒ 取归零（无偏移
//!   ＝静止位），在此登记。

/// ⭐ **可分配 LFO 的波形**（`lfoN_wave`，SFZ v2 编号）。
///
/// 出处 <https://sfzformat.com/opcodes/lfoN_wave/>：0 triangle、1 sine、
/// 2 75% pulse、3 square (50%)、4 25% pulse、5 12.5% pulse、6 saw up、
/// 7 saw down；Default = 1。⚠ ARIA 的编号整体 +1 且 1 是 random —— 本片
/// 只认 SFZ v2 编号，ARIA 编号按未知值响亮报错处理（见解析层）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LfoWave {
    /// 0：三角波。
    Triangle,
    /// 1：正弦波（规范缺省；v1 三族也用它，见模块文档）。
    Sine,
    /// 2：75% 占空比脉冲。
    Pulse75,
    /// 3：方波（50% 占空比）。
    Square,
    /// 4：25% 占空比脉冲。
    Pulse25,
    /// 5：12.5% 占空比脉冲。
    Pulse12_5,
    /// 6：上行锯齿。
    SawUp,
    /// 7：下行锯齿。
    SawDown,
}

impl LfoWave {
    /// 规范缺省（`lfoN_wave` Default = 1 ＝ sine）。
    pub const DEFAULT: Self = Self::Sine;

    /// 按 SFZ v2 编号取波形；编号不在 0..=7 ⇒ `None`（调用方响亮报错）。
    #[must_use]
    pub const fn from_index(index: i64) -> Option<Self> {
        match index {
            0 => Some(Self::Triangle),
            1 => Some(Self::Sine),
            2 => Some(Self::Pulse75),
            3 => Some(Self::Square),
            4 => Some(Self::Pulse25),
            5 => Some(Self::Pulse12_5),
            6 => Some(Self::SawUp),
            7 => Some(Self::SawDown),
            _ => None,
        }
    }

    /// 波形在相位 `phase`（周期单位，任意实数、内部取小数部分）处的
    /// 输出（-1..=1）。脉冲波在占空比内为 +1、其余为 -1。
    #[must_use]
    pub fn sample(self, phase: f32) -> f32 {
        let p = phase.rem_euclid(1.0);
        match self {
            Self::Sine => (p * core::f32::consts::TAU).sin(),
            Self::Triangle => {
                if p < 0.25 {
                    4.0 * p
                } else if p < 0.75 {
                    2.0 - 4.0 * p
                } else {
                    4.0 * p - 4.0
                }
            }
            Self::Pulse75 => {
                if p < 0.75 {
                    1.0
                } else {
                    -1.0
                }
            }
            Self::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Self::Pulse25 => {
                if p < 0.25 {
                    1.0
                } else {
                    -1.0
                }
            }
            Self::Pulse12_5 => {
                if p < 0.125 {
                    1.0
                } else {
                    -1.0
                }
            }
            Self::SawUp => 2.0 * p - 1.0,
            Self::SawDown => 1.0 - 2.0 * p,
        }
    }
}

/// ⭐ **一条 CC 调制项**（LFO 专用形状：`cc` 号 + `delta`）。
///
/// 求值公式与本仓既有 CC 调制同形：`base + Σ delta × (cc_value / 127)`，
/// 同一 `(目标, cc)` 在作用域链上后者胜、归约后按 cc 升序（确定性求和）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LfoCcMod {
    /// MIDI CC 号（0..=127；解析层按既有调制口径接受 0..=255 的写法域
    /// 检查，见解析层实现）。
    pub cc: u8,
    /// 调制量（单位随目标：Hz / 音分 / dB）。
    pub delta: f32,
}

/// CC 调制求和的唯一实现（`cc == None` ⇒ 只回基准值，一项算术都不做）。
fn modulated(base: f32, mods: &[LfoCcMod], cc: Option<&dyn Fn(u8) -> u8>) -> f32 {
    let Some(probe) = cc else {
        return base;
    };
    let mut value = base;
    for item in mods {
        value += item.delta * (f32::from(probe(item.cc)) / 127.0);
    }
    value
}

/// ⭐ **v1 LFO 一族的完整定义**（`amplfo_*` / `fillfo_*` / `pitchlfo_*` 共用形状）。
///
/// 三族只有 `depth` 的单位／域不同（amp = dB ±10、fil/pitch = 音分 ±1200），
/// 形状一份、解析一份（族前缀与 depth 域作参数），⛔ 不写三套。
#[derive(Debug, Clone, PartialEq)]
pub struct V1Lfo {
    /// `*_delay`（秒；0..=100，缺省 0）：触发后先不振荡的时长。
    pub delay: f32,
    /// `*_fade`（秒；0..=100，缺省 0）：delay 之后线性淡入到满深度的时长。
    pub fade: f32,
    /// `*_freq`（Hz；0..=20，缺省 0）。
    pub freq: f32,
    /// `*_depth`（amp = dB、fil/pitch = 音分；缺省 0）。
    pub depth: f32,
    /// `*_depthccN` / `*_depth_onccN` 的调制量（按 cc 升序）。
    pub depth_cc: Vec<LfoCcMod>,
    /// `*_freqccN` 的调制量（Hz；按 cc 升序）。
    pub freq_cc: Vec<LfoCcMod>,
    /// `*_depthchanaft`（已解析未消费，见模块文档）。
    pub depth_chanaft: f32,
    /// `*_depthpolyaft`（已解析未消费，见模块文档）。
    pub depth_polyaft: f32,
    /// `*_freqchanaft`（已解析未消费，见模块文档）。
    pub freq_chanaft: f32,
    /// `*_freqpolyaft`（已解析未消费，见模块文档）。
    pub freq_polyaft: f32,
}

impl V1Lfo {
    /// 空定义（该族一个 opcode 都没出现）。
    pub const EMPTY: Self = Self {
        delay: 0.0,
        fade: 0.0,
        freq: 0.0,
        depth: 0.0,
        depth_cc: Vec::new(),
        freq_cc: Vec::new(),
        depth_chanaft: 0.0,
        depth_polyaft: 0.0,
        freq_chanaft: 0.0,
        freq_polyaft: 0.0,
    };

    /// 该族是否**写了任何** opcode（解析层测试与摊平判定用）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::EMPTY
    }

    /// **CC 可消费意义上的透明**：有效 depth 恒为 0 的静态那一半
    /// （基准 depth 与 depth 的 cc 项都缺席）⇒ 引擎不产出运行束。
    /// ⚠ aft 两支不进本判定（未消费，见模块文档）。
    #[must_use]
    pub fn is_transparent(&self) -> bool {
        self.depth == 0.0 && self.depth_cc.is_empty()
    }

    /// 给定 CC 探针下的有效 depth（aft 支不消费）。
    #[must_use]
    pub fn effective_depth(&self, cc: Option<&dyn Fn(u8) -> u8>) -> f32 {
        modulated(self.depth, &self.depth_cc, cc)
    }

    /// 给定 CC 探针下的有效 freq（Hz；负值钳到 0 —— 频率不能为负，
    /// cc 项域 -200..200 与基准相加后可为负，规格未明，取钳制并登记）。
    #[must_use]
    pub fn effective_freq(&self, cc: Option<&dyn Fn(u8) -> u8>) -> f32 {
        modulated(self.freq, &self.freq_cc, cc).max(0.0)
    }

    /// 在触发后 `t` 秒、给定 CC 探针下的**无量纲输出**（-1..=1 × fade 因子）。
    ///
    /// 语义：`t < delay` ⇒ 0；其后相位 ＝ `freq × (t − delay)`（sine 波形，
    /// 见模块文档的 v1 波形裁决）；`fade > 0` 时再乘线性淡入因子
    /// `min((t − delay) / fade, 1)`。消费方把本值乘上
    /// [`Self::effective_depth`] 即得该目标的偏移量。
    #[must_use]
    pub fn value_at(&self, t: f32, cc: Option<&dyn Fn(u8) -> u8>) -> f32 {
        let elapsed = t - self.delay;
        if elapsed < 0.0 {
            return 0.0;
        }
        let freq = self.effective_freq(cc);
        if freq == 0.0 {
            return 0.0;
        }
        let wave = LfoWave::Sine.sample(freq * elapsed);
        if self.fade > 0.0 {
            wave * (elapsed / self.fade).min(1.0)
        } else {
            wave
        }
    }
}

impl Default for V1Lfo {
    fn default() -> Self {
        Self::EMPTY.clone()
    }
}

/// ⭐ **一台可分配 LFO**（`lfoN_*`；SFZ v2）。
///
/// 目标只收本片有消费落点的四个：`pitch`（音分）、`cutoff`（音分偏移）、
/// `resonance`（dB）、`volume`（dB）—— 单位类比的依据与不做
/// `amplitude` 的理由见模块文档。各目标的 `*_onccX` 调制量与目标同单位；
/// `freq/delay/fade/phase` 的 `*_onccX` 同理。
#[derive(Debug, Clone, PartialEq)]
pub struct GenericLfo {
    /// LFO 编号（`lfo01` ⇒ 1、`lfo3` ⇒ 3；按编号升序归约，确定性）。
    pub index: u32,
    /// `lfoN_wave`（缺省 [`LfoWave::DEFAULT`]）。
    pub wave: LfoWave,
    /// `lfoN_freq`（Hz；缺省 0 —— 规格 Default 是 N/A，0 ⇒ 不振荡，
    /// 是唯一不改变既有输出的读法）。
    pub freq: f32,
    /// `lfoN_delay`（秒；缺省 0）。
    pub delay: f32,
    /// `lfoN_fade`（秒；缺省 0 —— 规格 Default 是 N/A，同 `freq` 口径）。
    pub fade: f32,
    /// `lfoN_phase`（周期；0..=1，缺省 0）。
    pub phase: f32,
    /// `lfoN_count`（重复次数；`None` ＝ 未给出 ⇒ 无限）。
    pub count: Option<u32>,
    /// `lfoN_stepX` 的电平表（百分比 -100..100；下标 ＝ X 的数值序）。
    ///
    /// 非空 ⇒ 本 LFO 是**阶梯 LFO**：输出取
    /// `steps[floor(phase × len)] / 100`，⛔ 不再读 [`Self::wave`]。
    /// `lfoN_steps` 只作声明计数、与本表长度不一致时以**实际给出的
    /// stepX 为准**（规格未明两者的优先序，在此登记）。
    pub steps: Vec<f32>,
    /// `lfoN_pitch`（音分）。
    pub pitch: f32,
    /// `lfoN_cutoff`（音分偏移；类比依据见模块文档）。
    pub cutoff: f32,
    /// `lfoN_resonance`（dB；类比依据见模块文档）。
    pub resonance: f32,
    /// `lfoN_volume`（dB；类比依据见模块文档）。
    pub volume: f32,
    /// `lfoN_pitch_onccX` 调制量（音分，按 cc 升序）。
    pub pitch_oncc: Vec<LfoCcMod>,
    /// `lfoN_cutoff_onccX` 调制量（按 cc 升序）。
    pub cutoff_oncc: Vec<LfoCcMod>,
    /// `lfoN_resonance_onccX` 调制量（按 cc 升序）。
    pub resonance_oncc: Vec<LfoCcMod>,
    /// `lfoN_volume_onccX` 调制量（按 cc 升序）。
    pub volume_oncc: Vec<LfoCcMod>,
    /// `lfoN_freq_onccX` 调制量（Hz，按 cc 升序）。
    pub freq_oncc: Vec<LfoCcMod>,
    /// `lfoN_delay_onccX` 调制量（秒，按 cc 升序）。
    pub delay_oncc: Vec<LfoCcMod>,
    /// `lfoN_fade_onccX` 调制量（秒，按 cc 升序）。
    pub fade_oncc: Vec<LfoCcMod>,
    /// `lfoN_phase_onccX` 调制量（周期，按 cc 升序）。
    pub phase_oncc: Vec<LfoCcMod>,
}

impl GenericLfo {
    /// 一台只有编号的空 LFO（全缺省、无目标）。
    #[must_use]
    pub fn empty(index: u32) -> Self {
        Self {
            index,
            wave: LfoWave::DEFAULT,
            freq: 0.0,
            delay: 0.0,
            fade: 0.0,
            phase: 0.0,
            count: None,
            steps: Vec::new(),
            pitch: 0.0,
            cutoff: 0.0,
            resonance: 0.0,
            volume: 0.0,
            pitch_oncc: Vec::new(),
            cutoff_oncc: Vec::new(),
            resonance_oncc: Vec::new(),
            volume_oncc: Vec::new(),
            freq_oncc: Vec::new(),
            delay_oncc: Vec::new(),
            fade_oncc: Vec::new(),
            phase_oncc: Vec::new(),
        }
    }

    /// **可消费意义上的透明**：四个目标的基准与 oncc 项全缺席
    /// ⇒ 引擎不产出运行束（即使它调制别的 LFO 也不在本片，见模块文档）。
    #[must_use]
    pub fn is_transparent(&self) -> bool {
        self.pitch == 0.0
            && self.cutoff == 0.0
            && self.resonance == 0.0
            && self.volume == 0.0
            && self.pitch_oncc.is_empty()
            && self.cutoff_oncc.is_empty()
            && self.resonance_oncc.is_empty()
            && self.volume_oncc.is_empty()
    }

    /// 给定 CC 探针下的有效 freq（Hz，钳到 ≥ 0，口径同 [`V1Lfo::effective_freq`]）。
    #[must_use]
    pub fn effective_freq(&self, cc: Option<&dyn Fn(u8) -> u8>) -> f32 {
        modulated(self.freq, &self.freq_oncc, cc).max(0.0)
    }

    /// 给定 CC 探针下的有效 delay（秒，钳到 ≥ 0）。
    #[must_use]
    pub fn effective_delay(&self, cc: Option<&dyn Fn(u8) -> u8>) -> f32 {
        modulated(self.delay, &self.delay_oncc, cc).max(0.0)
    }

    /// 给定 CC 探针下的有效 fade（秒，钳到 ≥ 0）。
    #[must_use]
    pub fn effective_fade(&self, cc: Option<&dyn Fn(u8) -> u8>) -> f32 {
        modulated(self.fade, &self.fade_oncc, cc).max(0.0)
    }

    /// 给定 CC 探针下的有效 phase（周期，不钳位 —— 波形求值内部取小数部分）。
    #[must_use]
    pub fn effective_phase(&self, cc: Option<&dyn Fn(u8) -> u8>) -> f32 {
        modulated(self.phase, &self.phase_oncc, cc)
    }

    /// 四个目标在给定 CC 探针下的有效深度：`(pitch 音分, cutoff 音分, resonance dB, volume dB)`。
    #[must_use]
    pub fn effective_targets(&self, cc: Option<&dyn Fn(u8) -> u8>) -> (f32, f32, f32, f32) {
        (
            modulated(self.pitch, &self.pitch_oncc, cc),
            modulated(self.cutoff, &self.cutoff_oncc, cc),
            modulated(self.resonance, &self.resonance_oncc, cc),
            modulated(self.volume, &self.volume_oncc, cc),
        )
    }

    /// 在触发后 `t` 秒、给定 CC 探针下的**无量纲输出**（-1..=1 × fade 因子）。
    ///
    /// 语义：`t < delay` ⇒ 0；相位 ＝ `phase + freq × (t − delay)`；
    /// `count` 给出且已走满 ⇒ 0（模块文档的归零裁决）；阶梯表非空时
    /// 按阶梯取电平、否则按 [`Self::wave`] 取波形；`fade` 同 [`V1Lfo::value_at`]。
    #[must_use]
    pub fn value_at(&self, t: f32, cc: Option<&dyn Fn(u8) -> u8>) -> f32 {
        let delay = self.effective_delay(cc);
        let elapsed = t - delay;
        if elapsed < 0.0 {
            return 0.0;
        }
        let freq = self.effective_freq(cc);
        if freq == 0.0 {
            return 0.0;
        }
        let cycles = freq * elapsed;
        if let Some(count) = self.count {
            if cycles >= count as f32 {
                return 0.0;
            }
        }
        let phase = self.effective_phase(cc) + cycles;
        let wave = if self.steps.is_empty() {
            self.wave.sample(phase)
        } else {
            let slot = (phase.rem_euclid(1.0) * self.steps.len() as f32) as usize;
            self.steps[slot.min(self.steps.len() - 1)] / 100.0
        };
        let fade = self.effective_fade(cc);
        if fade > 0.0 {
            wave * (elapsed / fade).min(1.0)
        } else {
            wave
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wave_shapes_hit_their_landmarks() {
        assert_eq!(LfoWave::Sine.sample(0.0), 0.0);
        assert!((LfoWave::Sine.sample(0.25) - 1.0).abs() < 1e-6);
        assert_eq!(LfoWave::Triangle.sample(0.25), 1.0);
        assert_eq!(LfoWave::Triangle.sample(0.75), -1.0);
        assert_eq!(LfoWave::Square.sample(0.25), 1.0);
        assert_eq!(LfoWave::Square.sample(0.75), -1.0);
        assert_eq!(LfoWave::SawUp.sample(0.0), -1.0);
        assert_eq!(LfoWave::SawDown.sample(0.0), 1.0);
        assert_eq!(LfoWave::from_index(1), Some(LfoWave::Sine));
        assert_eq!(LfoWave::from_index(8), None);
    }

    #[test]
    fn v1_value_at_respects_delay_and_fade() {
        let lfo = V1Lfo {
            delay: 1.0,
            fade: 1.0,
            freq: 1.0,
            depth: 100.0,
            ..V1Lfo::EMPTY.clone()
        };
        assert_eq!(lfo.value_at(0.5, None), 0.0, "delay 内恒 0");
        assert_eq!(lfo.value_at(1.0, None), 0.0, "delay 终点相位 0 ⇒ sine 0");
        // t=1.25：相位 0.25 ⇒ sine=1；fade 因子 0.25。
        assert!((lfo.value_at(1.25, None) - 0.25).abs() < 1e-6);
        assert_eq!(lfo.effective_depth(None), 100.0);
        assert!(lfo.is_transparent() == false);
        assert!(V1Lfo::EMPTY.is_transparent());
    }

    #[test]
    fn generic_steps_and_count() {
        let mut lfo = GenericLfo::empty(1);
        lfo.freq = 1.0;
        lfo.steps = vec![100.0, -100.0];
        lfo.pitch = 100.0;
        assert_eq!(lfo.value_at(0.25, None), 1.0);
        assert_eq!(lfo.value_at(0.75, None), -1.0);
        lfo.count = Some(1);
        assert_eq!(lfo.value_at(1.25, None), 0.0, "count 走满 ⇒ 归零");
        assert_eq!(lfo.effective_targets(None), (100.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn cc_modulation_scales_by_probe() {
        let mut lfo = V1Lfo::EMPTY.clone();
        lfo.depth_cc = vec![LfoCcMod {
            cc: 1,
            delta: 100.0,
        }];
        let full = |_: u8| 127u8;
        let half = |_: u8| 0u8;
        assert_eq!(lfo.effective_depth(Some(&full)), 100.0);
        assert_eq!(lfo.effective_depth(Some(&half)), 0.0);
        assert_eq!(lfo.effective_depth(None), 0.0);
    }
}
