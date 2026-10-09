//! 声部包络：可覆盖 release 的 ADSR。[ARCH-RT-004] [ARCH-DSP-001]
//!
//! Ported from synth-core (MIT, Copyright (c) 2026 GROOVE SYNTH GS-1 contributors)
//! `src/dsp/adsr.rs`。
//!
//! 用原生 Rust 实现（来源库当年也是这个理由）：引擎需要**每个声部一个**包络实例，
//! 并且 release 必须能被**逐实例覆盖**（声部抢占要给它一段毫秒级的快速淡出），
//! 这是共享参数的包装器表达不了的。算法是"线性 attack + 单极点 decay/release"，
//! 无分配、全程单元测试覆盖。
//!
//! ## 与来源的差异
//!
//! 1. 类型改名为 [`Adsr`] / [`AdsrStage`]（来源叫 `Adsr` / `Stage`，`Stage` 太泛）；
//! 2. 显式补上规范 ID：零 sustain 自终止与抢占淡出分别对应 [ARCH-DSP-001] 与
//!    [ARCH-RT-004]，来源只有散文注释；
//! 3. 新增 [`Adsr::start_steal_fade`] 与 [`STEAL_RELEASE_SECONDS`]，把规范里
//!    "窃取瞬间施加 3ms 快速指数衰减微淡出"这一条**钉在 API 上**，而不是让
//!    每个调用点各自记住那个数字；
//! 4. 来源的 `set_sample_rate`/`set_params` 只接受裸值，夜半版本对退化输入
//!    （`NaN`/负数/`0`）做了钳制，避免系数变成 `NaN` 后整条链路静默死亡。
//!
//! ## 来源回归测试全部保留
//!
//! 尤其是"零 sustain 的声部在按键仍按住时自行结束"与"release 中途重触发要重新
//! 起 attack"——这两条是真实 bug 的化石。

/// 声部抢占时的快速淡出时间常数 [ARCH-RT-004]。
///
/// 规范原文："窃取瞬间对被终止声部强制应用 3ms 快速指数衰减微淡出包络，
/// 彻底杜绝爆音"。
pub const STEAL_RELEASE_SECONDS: f32 = 0.003;

/// 包络所处阶段。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AdsrStage {
    /// 静默（声部可回收）。
    Idle,
    /// 线性上升段。
    Attack,
    /// 指数下降到 sustain。
    Decay,
    /// 保持。
    Sustain,
    /// 指数下降到静默。
    Release,
}

/// 单声部 ADSR 包络。
///
/// 所有系数在构造期或参数变更时算好：逐样本路径上只有一次乘加或加法 [ARCH-RT-001]。
#[derive(Clone, Copy, Debug)]
pub struct Adsr {
    stage: AdsrStage,
    value: f32,
    attack_inc: f32,
    decay_coef: f32,
    release_coef: f32,
    attack_s: f32,
    decay_s: f32,
    sustain: f32,
    release_s: f32,
    sample_rate: f32,
}

impl Adsr {
    /// 构造：48 kHz、A 10 ms / D 200 ms / S 0.8 / R 300 ms（一个安全的默认音色）。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            stage: AdsrStage::Idle,
            value: 0.0,
            attack_inc: 1.0,
            decay_coef: 0.0,
            release_coef: 0.0,
            attack_s: 0.01,
            decay_s: 0.2,
            sustain: 0.8,
            release_s: 0.3,
            sample_rate: 48_000.0,
        }
    }

    /// 设置采样率并重算系数。
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sample_rate = crate::math::sanitise_sample_rate(sample_rate);
        if sample_rate != self.sample_rate {
            self.sample_rate = sample_rate;
            self.recompute();
        }
    }

    /// 当前采样率。
    #[must_use]
    pub const fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// 更新四段参数（秒 / 秒 / 0..1 / 秒）。
    ///
    /// 只有值真的变化时才重算系数，因此**防御性重复调用是零成本的**。
    /// 负时间钳到 0，sustain 钳到 `0..=1`。
    pub fn set_params(&mut self, attack_s: f32, decay_s: f32, sustain: f32, release_s: f32) {
        let attack_s = if attack_s.is_finite() {
            attack_s.max(0.0)
        } else {
            0.0
        };
        let decay_s = if decay_s.is_finite() {
            decay_s.max(0.0)
        } else {
            0.0
        };
        let sustain = if sustain.is_finite() {
            sustain.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let release_s = if release_s.is_finite() {
            release_s.max(0.0)
        } else {
            0.0
        };
        if (attack_s - self.attack_s).abs() > 1e-9
            || (decay_s - self.decay_s).abs() > 1e-9
            || (sustain - self.sustain).abs() > 1e-9
            || (release_s - self.release_s).abs() > 1e-9
        {
            self.attack_s = attack_s;
            self.decay_s = decay_s;
            self.sustain = sustain;
            self.release_s = release_s;
            self.recompute();
        }
    }

    /// 只覆盖 release 时间（声部抢占路径专用；不影响 A/D/S）。
    pub fn set_release(&mut self, release_s: f32) {
        let release_s = if release_s.is_finite() {
            release_s.max(0.0)
        } else {
            0.0
        };
        if (release_s - self.release_s).abs() > 1e-9 {
            self.release_s = release_s;
            self.recompute();
        }
    }

    /// 当前 sustain 电平。
    #[must_use]
    pub const fn sustain(&self) -> f32 {
        self.sustain
    }

    /// 重算三段系数。`attack` 是线性增量，decay/release 是一极点系数。
    fn recompute(&mut self) {
        let sample_rate = self.sample_rate;
        self.attack_inc = if self.attack_s <= 1e-5 {
            1.0
        } else {
            1.0 / (self.attack_s * sample_rate)
        };
        self.decay_coef = time_coefficient(self.decay_s, sample_rate);
        self.release_coef = time_coefficient(self.release_s, sample_rate);
    }

    /// 从静默重新开始（声部回收时调用）。
    pub fn reset(&mut self) {
        self.stage = AdsrStage::Idle;
        self.value = 0.0;
    }

    /// 按键按下：从当前电平（可能是 release 中途）重新进入 attack。
    pub fn gate_on(&mut self) {
        self.stage = AdsrStage::Attack;
    }

    /// 按键松开：进入 release（已是 Idle 则不动）。
    pub fn gate_off(&mut self) {
        if self.stage != AdsrStage::Idle {
            self.stage = AdsrStage::Release;
        }
    }

    /// 声部被抢占：覆盖 release 为 [STEAL_RELEASE_SECONDS] 并立即淡出 [ARCH-RT-004]。
    ///
    /// 调用点因此不需要知道"3ms"这个数字，也不会忘记先覆盖再 gate_off 的顺序。
    pub fn start_steal_fade(&mut self) {
        self.set_release(STEAL_RELEASE_SECONDS);
        self.gate_off();
    }

    /// 是否仍在发声（可据此回收声部）。
    #[must_use]
    pub const fn is_active(&self) -> bool {
        !matches!(self.stage, AdsrStage::Idle)
    }

    /// 当前输出电平。
    #[must_use]
    pub const fn value(&self) -> f32 {
        self.value
    }

    /// 当前阶段。
    #[must_use]
    pub const fn stage(&self) -> AdsrStage {
        self.stage
    }

    /// 推进一个样本并返回新的包络值。
    ///
    /// 音符**只能**通过 [`Self::gate_on`] 开始：这防止零 sustain 的声部在按键
    /// 还按着的时候自己重新起 attack（那样会变成无限重触发）。
    #[inline]
    pub fn process(&mut self, gate: bool) -> f32 {
        if !gate && self.stage != AdsrStage::Idle {
            self.stage = AdsrStage::Release;
        }

        match self.stage {
            AdsrStage::Attack => {
                self.value += self.attack_inc;
                if self.value >= 1.0 {
                    self.value = 1.0;
                    self.stage = AdsrStage::Decay;
                }
            }
            AdsrStage::Decay => {
                self.value = self.sustain + (self.value - self.sustain) * self.decay_coef;
                if (self.value - self.sustain).abs() < 1e-4 {
                    self.value = self.sustain;
                    // 零 sustain 的声部在 decay 结束时就完成，哪怕键还按着
                    //（拨弦/铃声行为）。
                    self.stage = if self.sustain <= 1e-4 {
                        AdsrStage::Idle
                    } else {
                        AdsrStage::Sustain
                    };
                    if self.stage == AdsrStage::Idle {
                        self.value = 0.0;
                    }
                }
            }
            AdsrStage::Sustain => {
                self.value = self.sustain;
            }
            AdsrStage::Release => {
                self.value *= self.release_coef;
                if self.value < 1e-4 {
                    self.value = 0.0;
                    self.stage = AdsrStage::Idle;
                }
            }
            AdsrStage::Idle => {
                self.value = 0.0;
            }
        }
        self.value
    }
}

/// 一极点系数：该段在 `time_s` 秒内走完自身跨度的约 99.8%
///（因此"release = 0.3 s"听起来就是 0.3 s，而不是 3 s）。
///
/// 指数走纯 Rust `libm::expf`，**不用** `f32::exp`：后者的实现来自宿主 libm，
/// 换架构/OS 末位可能不同，会把 L1 的逐位对账变成跨平台假红 `[ARCH-DET-001]`。
fn time_coefficient(time_s: f32, sample_rate: f32) -> f32 {
    /// 6 个时间常数 → `1 − e⁻⁶ ≈ 99.75%`。
    const TIME_CONSTANTS: f32 = 6.0;
    if time_s <= 1e-5 {
        0.0
    } else {
        libm::expf(-TIME_CONSTANTS / (time_s * sample_rate))
    }
}

impl Default for Adsr {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **判据（新写，可红）**：`reset` 之后的包络与**全新构造的同参数实例**在同样
    /// 的门信号下逐位一致。
    ///
    /// 量什么：128 个输出样本（`f32` 位型）、`value()`、`stage()`。
    ///
    /// `reset` 清两处（阶段与电平）。注入实测：去掉 `self.value = 0.0;`
    /// ⇒ 既有全量判据**全绿** ⇒ 复位后的电平没有被守住（下一次 attack 从旧电平起跳，
    /// 起振曲线与全新实例不同）。
    #[test]
    fn reset_reproduces_a_freshly_built_envelope_bit_for_bit() {
        let build = || {
            let mut envelope = Adsr::new();
            // ⚠ 夹具刻意用**非默认**参数：`Adsr::new()` 的三个系数是占位值
            //（`attack_inc = 1.0`、`decay_coef = release_coef = 0.0`），要等一次
            // **真的**参数变更才由 `recompute` 算出来；参数与构造器里存的四个秒值
            // 完全相等时那次重算会被去抖门限跳过。本判据测的是复位等价性，
            // 因此绕开那条待裁决的路径（见本票报告）。
            envelope.set_params(0.012, 0.22, 0.75, 0.33);
            envelope
        };
        let drive = |envelope: &mut Adsr| -> (Vec<f32>, f32, AdsrStage) {
            envelope.gate_on();
            let mut out: Vec<f32> = (0..64).map(|_| envelope.process(true)).collect();
            envelope.gate_off();
            out.extend((0..64).map(|_| envelope.process(false)));
            (out, envelope.value(), envelope.stage())
        };
        let mut used = build();
        let warmup = drive(&mut used);
        assert!(warmup.1 > 0.0, "夹具必须真的把包络推离初态");
        used.reset();
        let after = drive(&mut used);
        let fresh = drive(&mut build());
        assert_eq!(after.0, fresh.0, "复位后的输出不一致");
        assert_eq!(after.1.to_bits(), fresh.1.to_bits(), "复位后的电平不一致");
        assert_eq!(after.2, fresh.2, "复位后的阶段不一致");
    }

    fn render(envelope: &mut Adsr, gate: bool, samples: usize) -> f32 {
        let mut last = 0.0;
        for _ in 0..samples {
            last = envelope.process(gate);
        }
        last
    }

    #[test]
    fn attack_reaches_peak_then_decays_to_sustain() {
        let mut env = Adsr::new();
        env.set_sample_rate(48000.0);
        env.set_params(0.01, 0.05, 0.5, 0.1);
        env.gate_on();
        let peak = render(&mut env, true, 480);
        assert!(peak >= 0.99, "attack did not reach peak: {peak}");
        let sustain = render(&mut env, true, 48000 / 4);
        assert!((sustain - 0.5).abs() < 0.01, "sustain wrong: {sustain}");
    }

    #[test]
    fn release_decays_to_idle_and_reports_inactive() {
        let mut env = Adsr::new();
        env.set_sample_rate(48000.0);
        env.set_params(0.001, 0.01, 0.8, 0.02);
        env.gate_on();
        render(&mut env, true, 4800);
        env.gate_off();
        render(&mut env, false, 48000);
        assert!(!env.is_active());
        assert_eq!(env.value(), 0.0);
    }

    #[test]
    fn zero_sustain_voice_finishes_while_key_held() {
        let mut env = Adsr::new();
        env.set_sample_rate(48000.0);
        env.set_params(0.001, 0.02, 0.0, 0.1);
        env.gate_on();
        render(&mut env, true, 48000);
        assert!(!env.is_active(), "pluck should finish after decay");
        assert_eq!(env.value(), 0.0);
        // 按住的键不得悄悄重启包络。
        assert_eq!(render(&mut env, true, 128), 0.0);
    }

    #[test]
    fn retrigger_during_release_restarts_attack() {
        let mut env = Adsr::new();
        env.set_sample_rate(48000.0);
        env.set_params(0.01, 0.1, 0.6, 0.5);
        env.gate_on();
        render(&mut env, true, 2400);
        env.gate_off();
        render(&mut env, false, 1200);
        let mid_release = env.value();
        assert!(mid_release > 0.0);
        // 显式重触发（note-on）重启 attack。
        env.gate_on();
        let after = render(&mut env, true, 240);
        assert!(after > mid_release, "retrigger should climb again");
    }

    #[test]
    fn release_override_is_used_for_stealing() {
        let mut env = Adsr::new();
        env.set_sample_rate(48000.0);
        env.set_params(0.001, 0.01, 1.0, 5.0);
        env.gate_on();
        render(&mut env, true, 2400);
        env.set_release(0.004);
        env.gate_off();
        render(&mut env, false, 2400);
        assert!(!env.is_active(), "short release should finish quickly");
    }

    /// **判据（新写，可红）**：抢占淡出在规范时限内到静默，且**无瞬时跳变**。
    ///
    /// [ARCH-RT-004] 要求"3ms 快速指数衰减微淡出"，[ARCH-DSP-001] 要求"杜绝爆音"：
    /// 两者在这里被同时断言——时限（≤5 ms 到静默）与单样本位移上界
    ///（`1 − release_coef` 量级）。
    ///
    /// 若把 `time_coefficient` 的返回值改成 0（即"瞬间归零"），单样本位移会变成
    /// 满幅 1.0，本测试立即变红。
    #[test]
    fn steal_fade_finishes_in_time_and_never_steps() {
        let mut env = Adsr::new();
        env.set_sample_rate(48_000.0);
        env.set_params(0.001, 0.01, 1.0, 5.0);
        env.gate_on();
        render(&mut env, true, 4800);
        assert_eq!(env.value(), 1.0, "must start the fade from full level");

        env.start_steal_fade();
        // 3 ms 的 release 用 6 个时间常数 → 18 ms 走完 99.75%；
        // 规范要的是"3ms 量级的微淡出"，这里给 5 ms 的硬窗口。
        let window_samples = (0.005 * 48_000.0) as usize;
        // 单样本位移上界 = 段首电平 × (1 − release_coef)。
        let release_coef = (-6.0f32 / (STEAL_RELEASE_SECONDS * 48_000.0)).exp();
        let bound = (1.0 - release_coef) + 1e-6;
        let mut previous = env.value();
        let mut settled_at = None;
        for sample in 0..window_samples {
            let value = env.process(false);
            let step = (previous - value).abs();
            assert!(
                step <= bound,
                "sample {sample}: steal fade stepped {step}, bound {bound}"
            );
            previous = value;
            if !env.is_active() && settled_at.is_none() {
                settled_at = Some(sample + 1);
            }
        }
        let settled_at = settled_at.expect("steal fade never reached silence within 5 ms");
        assert!(settled_at <= window_samples, "settled late: {settled_at}");
        assert_eq!(env.value(), 0.0, "fade must end exactly at silence");
        assert_eq!(env.stage(), AdsrStage::Idle);
    }

    #[test]
    fn degenerate_parameters_are_clamped_instead_of_producing_nan() {
        let mut env = Adsr::new();
        env.set_sample_rate(f32::NAN);
        assert_eq!(env.sample_rate(), crate::MIN_SAMPLE_RATE);
        env.set_params(f32::NAN, -1.0, 5.0, f32::NAN);
        assert_eq!(env.sustain(), 1.0);
        env.gate_on();
        for _ in 0..2048 {
            assert!(env.process(true).is_finite());
        }
        env.gate_off();
        for _ in 0..4096 {
            assert!(env.process(false).is_finite());
        }
        // 零长度 attack/decay/release 不得把包络卡死。
        let mut env = Adsr::new();
        env.set_sample_rate(48_000.0);
        env.set_params(0.0, 0.0, 0.0, 0.0);
        env.gate_on();
        for _ in 0..64 {
            assert!(env.process(true).is_finite());
        }
        assert!(
            !env.is_active(),
            "zero-sustain zero-decay must finish at once"
        );
    }

    #[test]
    fn repeated_parameter_updates_are_idempotent() {
        let mut env = Adsr::new();
        env.set_sample_rate(48_000.0);
        env.set_params(0.01, 0.05, 0.5, 0.1);
        env.gate_on();
        // 重复设置同样的值不得改变轨迹（防御性调用的成本必须是零）。
        for _ in 0..16 {
            env.set_params(0.01, 0.05, 0.5, 0.1);
        }
        let value = render(&mut env, true, 240);

        let mut reference = Adsr::new();
        reference.set_sample_rate(48_000.0);
        reference.set_params(0.01, 0.05, 0.5, 0.1);
        reference.gate_on();
        let expected = render(&mut reference, true, 240);
        assert_eq!(value, expected, "重复设参改变了包络轨迹");
        assert_eq!(env.stage(), reference.stage());
    }
}
