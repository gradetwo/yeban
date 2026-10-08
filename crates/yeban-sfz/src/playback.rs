//! 把一次 note-on 变成**可渲染的采样描述**：`yeban-engine` 接入采样播放的接口。
//!
//! 本模块补上 `crate::lib.rs` 文档里点名的缺口：「静态预分配声部池（SFZ 采样源待接入）」。
//! 它只做**纯数值换算**（音高比、步进比、线性增益、循环窗口），不做 I/O、不解码音频、
//! 不查采样字节。因此它**不依赖任何新依赖**，也不需要采样数据入库。
//!
//! 规范来源 (Normative):
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` ROAD-M2-005（SFZ 引擎）
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3.2 [ARCH-RT-001]（RT 零分配）
//!
//! 格式事实与出处（全部取自 <https://sfzformat.com/opcodes/>）：
//! - `pitch_keycenter`：采样原始音高对应的 MIDI 根音。默认 60。
//!   <https://sfzformat.com/opcodes/pitch_keycenter/>
//! - `transpose`：移调，单位半音，默认 0，范围 -127..=127。
//!   <https://sfzformat.com/opcodes/transpose/>
//! - `tune`：微调，单位音分（cent），默认 0，SFZ1 范围 -100..=100。
//!   <https://sfzformat.com/opcodes/tune/>
//! - `volume`：音量，单位 dB，默认 0，规范范围 -144..=6。
//!   <https://sfzformat.com/opcodes/volume/>
//!
//! # 换算口径
//!
//! ```text
//! 音高比 pitch_ratio = 2 ^ ( ((note - pitch_keycenter + transpose) * 100 + tune) / 1200 )
//! 步进比 rate        = pitch_ratio * (sample_hz / engine_hz)
//! 线性增益 gain      = 2 ^ ( volume_db * log2(10) / 20 )
//! ```
//!
//! `rate` 的单位是「每输出一个采样所前进的**源采样**个数」。`region.pitch_keycenter`、
//! `region.transpose`、`region.tune` 只决定音高；采样文件自身的采样率只能由解码器给出，
//! 因此它作为 [`RenderRates`] 的输入（**不是**本 crate 猜测的值）。
//!
//! # 刻意不做的换算（避免发明语义）
//!
//! - **力度 → 增益**：`amp_veltrack` / `amp_velcurve_N` 未建模（见
//!   `docs/ledger/sfz-core-notes.md` §5），因此 [`PlaybackSpec`] 只**携带** `velocity`，
//!   不把它折进 `gain`。调用方用自己的力度律。
//! - **声相定律**：`pan` 原样以百分比输出；`pan_law` 是引擎侧的 need（N4）。
//! - **循环窗口缺省**：本 crate 不解码音频，所以 `loop_mode` 缺省是
//!   [`LoopMode::NoLoop`]（见 `crate::instrument::Region` 文档），一律不循环。
//! - **无有效循环窗口的降级**：`loop_mode` 要求循环、而 `loop_end <= loop_start` 时，
//!   [`Region::loop_window`] 返回 `None`。这是本模块的工程裁决：零长度循环没有定义，
//!   返回 `None` 表示「按不循环渲染」，绝不返回一个 `start == end` 的死循环窗口。
//!
//! # 实时安全
//!
//! [`Region::playback_spec`] / [`Instrument::playback_for`] 全程**零堆分配、零锁、
//! 零 I/O**：它们只读已构造好的 region 索引并返回 `Copy` 的 [`PlaybackSpec`]。
//! 音高/增益使用 `exp2`（与 `crate::voice_pool::StealFade::gain_at` 同类的浮点原语）；
//! 跨架构逐位一致性 (ARCH-DET-002) 未验证，与 `StealFade` 登记在同一条 pending 上。

use crate::instrument::{Instrument, LoopMode, Region, RegionQuery};

/// 采样率回退值 (Hz)：输入采样率非有限或非正时使用，与
/// [`crate::voice_pool::StealFade::new`] 的回退口径一致。
pub const FALLBACK_SAMPLE_RATE: f32 = 48_000.0;

/// 一个八度内的音分数（`transpose` 半音与 `tune` 音分统一到这个单位）。
const CENTS_PER_OCTAVE: f64 = 1200.0;

/// 渲染采样所需的两个采样率 (Hz)。
///
/// `sample_hz` 是采样文件自身的采样率，只能由解码器给出；`engine_hz` 是工程 /
/// 引擎的运行采样率。`copy` 语义，可在实时路径构造。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderRates {
    /// 采样文件的原始采样率 (Hz)。
    pub sample_hz: f32,
    /// 工程 / 引擎采样率 (Hz)。
    pub engine_hz: f32,
}

impl RenderRates {
    /// 构造采样率对（不做校验；校验发生在 [`RenderRates::sanitized`]）。
    #[must_use]
    pub fn new(sample_hz: f32, engine_hz: f32) -> Self {
        Self {
            sample_hz,
            engine_hz,
        }
    }

    /// 把非有限 / 非正的采样率换成 [`FALLBACK_SAMPLE_RATE`]。
    ///
    /// 该回退保证 [`RenderRates`] 永远产生有限的步进比，绝不产生 `NaN` / 除零。
    #[must_use]
    pub fn sanitized(self) -> Self {
        Self {
            sample_hz: sanitize_rate(self.sample_hz),
            engine_hz: sanitize_rate(self.engine_hz),
        }
    }
}

impl Default for RenderRates {
    /// 采样率与引擎采样率都是 [`FALLBACK_SAMPLE_RATE`]（即不做采样率换算）。
    fn default() -> Self {
        Self {
            sample_hz: FALLBACK_SAMPLE_RATE,
            engine_hz: FALLBACK_SAMPLE_RATE,
        }
    }
}

fn sanitize_rate(hz: f32) -> f32 {
    if hz.is_finite() && hz > 0.0 {
        hz
    } else {
        FALLBACK_SAMPLE_RATE
    }
}

/// 一段有效的循环窗口，单位是**采样点**，半开区间 `[start, end)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopWindow {
    /// 循环起点（含）。
    pub start: u32,
    /// 循环终点（不含）。
    pub end: u32,
}

impl LoopWindow {
    /// 循环长度（采样点）。构造上恒 `>= 1`（见 [`Region::loop_window`]）。
    #[must_use]
    pub fn len(self) -> u32 {
        self.end - self.start
    }

    /// 是否为空窗口。构造上恒为 `false`；保留该方法是 `len()` 的配对 API。
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
}

/// 一次 note-on 的可渲染采样描述。
///
/// `Copy`、不含指针、不含堆数据：渲染器可以把它存进自己的预分配声部槽，
/// 逐样本路径随时读取 [ARCH-RT-001]。
///
/// **采样身份不在本结构里**：`sample` 路径与已解码的采样缓冲的对应关系是
/// **加载期**的事（引擎按 region 建立索引）。实时路径只读本结构的数值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackSpec {
    /// 触发音符（MIDI 号 0..=127）。
    pub note: u8,
    /// 触发力度 0..=127。**不**折进 [`PlaybackSpec::gain`]（见模块文档）。
    pub velocity: u8,
    /// 该 region 的根音（`pitch_keycenter`）。
    pub pitch_keycenter: i32,
    /// 该 region 的移调（半音）。
    pub transpose: i32,
    /// 该 region 的微调（音分）。
    pub tune: i32,
    /// 音高比：`2 ^ (((note - pitch_keycenter + transpose) * 100 + tune) / 1200)`。
    pub pitch_ratio: f32,
    /// 步进比：每输出一个采样前进的源采样个数（已含采样率换算）。
    pub rate: f32,
    /// 该 region 的 `volume`（dB，原样）。
    pub volume_db: f32,
    /// `volume` 换算出的**线性**增益：`10 ^ (volume_db / 20)`。
    pub gain: f32,
    /// 该 region 的 `pan`（百分比，原样；声相定律由调用方决定）。
    pub pan: f32,
    /// 循环模式（原样）。
    pub loop_mode: LoopMode,
    /// 有效循环窗口；`None` 表示按不循环渲染。
    pub loop_window: Option<LoopWindow>,
    /// 独占组（`group`）。
    pub group: u32,
    /// 被谁关掉（`off_by`）。
    pub off_by: u32,
    /// 该 region 的 `<region>` 段头所在行号（1-based，诊断用）。
    pub source_line: usize,
}

impl PlaybackSpec {
    /// 是否循环（`loop_mode` 要求循环**且**窗口有效）。
    #[must_use]
    pub fn loops(&self) -> bool {
        self.loop_window.is_some()
    }

    /// 是否忽略 note-off（`loop_mode=one_shot`，鼓组常用）。
    #[must_use]
    pub fn ignores_note_off(&self) -> bool {
        self.loop_mode == LoopMode::OneShot
    }
}

/// [`Instrument::playback_for`] 的结果：选中的 region **加**它的可渲染描述。
///
/// `region` 用于加载期解析采样身份（[`Region::sample_path`]）；`spec` 用于实时路径。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionPlay<'i, 'a> {
    /// 选中的 region（借用自 [`Instrument`]）。
    pub region: &'i Region<'a>,
    /// 该 region 的可渲染采样描述。
    pub spec: PlaybackSpec,
}

impl<'a> Region<'a> {
    /// 音高比：`2 ^ (((note - pitch_keycenter + transpose) * 100 + tune) / 1200)`。
    ///
    /// 全程 `f64` 计算后收窄到 `f32`。零分配，可在实时路径调用。
    #[must_use]
    pub fn pitch_ratio(&self, note: u8) -> f32 {
        let semitones = f64::from(i32::from(note)) - f64::from(self.pitch_keycenter)
            + f64::from(self.transpose);
        let cents = semitones * 100.0 + f64::from(self.tune);
        (cents / CENTS_PER_OCTAVE).exp2() as f32
    }

    /// 步进比：`pitch_ratio(note) * sample_hz / engine_hz`。
    ///
    /// 非法采样率按 [`RenderRates::sanitized`] 回退，因此返回值恒为有限正数
    /// （`pitch_ratio` 的合法输入范围是 `2^-21.2 ..= 2^31.7`，见模块测试）。
    #[must_use]
    pub fn playback_rate(&self, note: u8, rates: RenderRates) -> f32 {
        let rates = rates.sanitized();
        self.pitch_ratio(note) * (rates.sample_hz / rates.engine_hz)
    }

    /// `volume` (dB) → 线性增益：`10 ^ (volume / 20)`。
    ///
    /// 解析路径保证 `volume` 有限（[`crate::parser::OpcodeValue::as_f32`] 拒绝
    /// `NaN` / `±Inf`），因此本函数对解析得到的 region 不会返回 `NaN`。
    /// 手工构造的 region 若把 `volume` 设为非有限值，结果是 IEEE 语义的 `NaN` / `inf`
    /// —— 本 crate **不**发明钳制策略。
    #[must_use]
    pub fn linear_gain(&self) -> f32 {
        (f64::from(self.volume) * std::f64::consts::LOG2_10 / 20.0).exp2() as f32
    }

    /// 有效循环窗口。
    ///
    /// 返回 `Some` 的条件：`loop_mode` 是 `loop_continuous` 或 `loop_sustain`，
    /// **且** `loop_end > loop_start`。`no_loop` / `one_shot` 恒返回 `None`
    /// （即使文件写了 `loop_start` / `loop_end`）。
    #[must_use]
    pub fn loop_window(&self) -> Option<LoopWindow> {
        if !matches!(
            self.loop_mode,
            LoopMode::LoopContinuous | LoopMode::LoopSustain
        ) {
            return None;
        }
        if self.loop_end > self.loop_start {
            Some(LoopWindow {
                start: self.loop_start,
                end: self.loop_end,
            })
        } else {
            None
        }
    }

    /// 生成该 region 的可渲染采样描述。零分配，可在实时路径调用。
    #[must_use]
    pub fn playback_spec(&self, note: u8, velocity: u8, rates: RenderRates) -> PlaybackSpec {
        let rates = rates.sanitized();
        PlaybackSpec {
            note,
            velocity,
            pitch_keycenter: self.pitch_keycenter,
            transpose: self.transpose,
            tune: self.tune,
            pitch_ratio: self.pitch_ratio(note),
            rate: self.playback_rate(note, rates),
            volume_db: self.volume,
            gain: self.linear_gain(),
            pan: self.pan,
            loop_mode: self.loop_mode,
            loop_window: self.loop_window(),
            group: self.group,
            off_by: self.off_by,
            source_line: self.source_line,
        }
    }
}

impl<'a> Instrument<'a> {
    /// 一次调用完成「选 region → 生成可渲染描述」。
    ///
    /// 参数 `query` 的全部字段（音符 / 力度 / 通道 / 轮替序号 / keyswitch / CC）
    /// 都参与选择，语义与 [`Instrument::region_for_with`] 完全一致。
    /// 没有匹配 region 时返回 `None`。全程零分配，可在实时路径调用。
    #[must_use]
    pub fn playback_for(
        &self,
        query: RegionQuery<'_>,
        rates: RenderRates,
    ) -> Option<RegionPlay<'_, 'a>> {
        let note = query.note;
        let velocity = query.velocity;
        let region = self.region_for_with(query)?;
        Some(RegionPlay {
            region,
            spec: region.playback_spec(note, velocity, rates),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_text;

    const RATES_EQUAL: RenderRates = RenderRates {
        sample_hz: 48_000.0,
        engine_hz: 48_000.0,
    };

    fn close(left: f32, right: f32, tolerance: f32) -> bool {
        (left - right).abs() <= tolerance
    }

    #[test]
    fn pitch_ratio_follows_equal_temperament() {
        let instrument = parse_text(
            "<region>sample=a.wav pitch_keycenter=60",
            &Default::default(),
        )
        .expect("parses");
        let region = &instrument.regions()[0];
        assert_eq!(region.pitch_ratio(60), 1.0);
        assert_eq!(
            region.pitch_ratio(72),
            2.0,
            "an octave up doubles the ratio"
        );
        assert_eq!(
            region.pitch_ratio(48),
            0.5,
            "an octave down halves the ratio"
        );
        assert!(
            close(region.pitch_ratio(61), 2.0f32.powf(1.0 / 12.0), 1.0e-6),
            "one semitone must be 2^(1/12)"
        );
    }

    #[test]
    fn transpose_and_tune_shift_the_ratio() {
        let by_transpose = parse_text(
            "<region>sample=a.wav pitch_keycenter=60 transpose=12",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(by_transpose.regions()[0].pitch_ratio(60), 2.0);

        // transpose 与「同一音符高 12 个半音」必须给出同一个比值。
        let by_note = parse_text(
            "<region>sample=a.wav pitch_keycenter=60",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(
            by_transpose.regions()[0].pitch_ratio(60),
            by_note.regions()[0].pitch_ratio(72)
        );

        // tune 是音分：+1200 音分 = 一个八度。
        let by_tune = parse_text(
            "<region>sample=a.wav pitch_keycenter=60 tune=100",
            &Default::default(),
        )
        .expect("parses");
        assert!(close(
            by_tune.regions()[0].pitch_ratio(60),
            2.0f32.powf(100.0 / 1200.0),
            1.0e-6
        ));
    }

    #[test]
    fn playback_rate_converts_the_sample_rate() {
        let instrument = parse_text(
            "<region>sample=a.wav pitch_keycenter=60",
            &Default::default(),
        )
        .expect("parses");
        let region = &instrument.regions()[0];

        // 44.1 kHz 采样在 48 kHz 引擎上：每个输出采样前进 44100/48000 个源采样。
        let rates = RenderRates::new(44_100.0, 48_000.0);
        assert!(close(
            region.playback_rate(60, rates),
            44_100.0 / 48_000.0,
            1.0e-9
        ));
        // 上八度 = 2 × 步进比。
        assert!(close(
            region.playback_rate(72, rates),
            2.0 * 44_100.0 / 48_000.0,
            1.0e-9
        ));
        // 采样率相同 ⇒ 步进比就是音高比。
        assert_eq!(region.playback_rate(60, RATES_EQUAL), 1.0);
    }

    #[test]
    fn invalid_rates_fall_back_and_never_produce_nan() {
        let instrument = parse_text(
            "<region>sample=a.wav pitch_keycenter=60",
            &Default::default(),
        )
        .expect("parses");
        let region = &instrument.regions()[0];
        for rates in [
            RenderRates::new(f32::NAN, 48_000.0),
            RenderRates::new(0.0, 48_000.0),
            RenderRates::new(-44_100.0, 48_000.0),
            RenderRates::new(44_100.0, f32::INFINITY),
            RenderRates::new(44_100.0, 0.0),
        ] {
            let rate = region.playback_rate(60, rates);
            assert!(
                rate.is_finite() && rate > 0.0,
                "rate = {rate} for {rates:?}"
            );
        }
        // 两个采样率都非法 ⇒ 回退后相互抵消，步进比等于音高比。
        assert_eq!(
            region.playback_rate(72, RenderRates::new(f32::NAN, f32::NAN)),
            2.0
        );
        assert_eq!(RenderRates::default(), RATES_EQUAL);
    }

    #[test]
    fn linear_gain_is_db_amplitude() {
        let gain_of = |volume: f32| {
            parse_text(
                &format!("<region>sample=a.wav volume={volume}"),
                &Default::default(),
            )
            .expect("parses")
            .regions()[0]
                .linear_gain()
        };
        assert!(close(gain_of(0.0), 1.0, 1.0e-7));
        assert!(close(gain_of(-6.0206), 0.5, 1.0e-5));
        assert!(close(gain_of(6.0), 1.995_262_3, 1.0e-5));
        // `gain` 是 `volume` 的别名（见 instrument.rs 的 build_region）。
        let gain_alias =
            parse_text("<region>sample=a.wav gain=-6.0206", &Default::default()).expect("parses");
        assert!(close(gain_alias.regions()[0].linear_gain(), 0.5, 1.0e-5));
    }

    #[test]
    fn loop_window_requires_a_strictly_positive_length() {
        let window_of = |text: &str| {
            parse_text(text, &Default::default())
                .expect("parses")
                .regions()[0]
                .loop_window()
        };
        let ok =
            window_of("<region>sample=a.wav loop_mode=loop_continuous loop_start=10 loop_end=20");
        assert_eq!(ok, Some(LoopWindow { start: 10, end: 20 }));
        assert_eq!(ok.expect("window").len(), 10);
        assert!(!ok.expect("window").is_empty());

        // 零长度 / 反向窗口 ⇒ None（不返回死循环）。
        assert_eq!(
            window_of("<region>sample=a.wav loop_mode=loop_continuous loop_start=10 loop_end=10"),
            None
        );
        assert_eq!(
            window_of("<region>sample=a.wav loop_mode=loop_sustain loop_start=30 loop_end=20"),
            None
        );
        // no_loop / one_shot / 缺省：即使写了循环点也不循环。
        assert_eq!(
            window_of("<region>sample=a.wav loop_start=10 loop_end=20"),
            None
        );
        assert_eq!(
            window_of("<region>sample=a.wav loop_mode=one_shot loop_start=10 loop_end=20"),
            None
        );
    }

    #[test]
    fn one_shot_ignores_note_off_and_only_loop_modes_loop() {
        let spec_of = |text: &str| {
            parse_text(text, &Default::default())
                .expect("parses")
                .regions()[0]
                .playback_spec(60, 100, RATES_EQUAL)
        };
        let one_shot = spec_of("<region>sample=a.wav loop_mode=one_shot");
        assert!(one_shot.ignores_note_off());
        assert!(!one_shot.loops());

        let no_loop = spec_of("<region>sample=a.wav");
        assert!(!no_loop.ignores_note_off());
        assert!(!no_loop.loops());

        let looping =
            spec_of("<region>sample=a.wav loop_mode=loop_continuous loop_start=0 loop_end=48");
        assert!(looping.loops());
        assert!(!looping.ignores_note_off());
        assert_eq!(looping.loop_window.map(LoopWindow::len), Some(48));

        // `loop_sustain` 也必须被 `loops()` 认作循环（两种循环模式都要覆盖）。
        let sustain =
            spec_of("<region>sample=a.wav loop_mode=loop_sustain loop_start=0 loop_end=64");
        assert!(sustain.loops());
        assert!(!sustain.ignores_note_off());

        // 要求循环但窗口无效 ⇒ `loops()` 为 false（降级为播一遍）。
        let broken_window =
            spec_of("<region>sample=a.wav loop_mode=loop_continuous loop_start=9 loop_end=9");
        assert!(!broken_window.loops());
        assert_eq!(broken_window.loop_window, None);
    }

    #[test]
    fn playback_spec_carries_the_region_fields_and_is_deterministic() {
        let instrument = parse_text(
            "<region>sample=a.wav key=36 pitch_keycenter=60 transpose=-2 tune=50 \
             volume=-3 pan=-25 loop_mode=loop_sustain loop_start=5 loop_end=105 group=7 off_by=9",
            &Default::default(),
        )
        .expect("parses");
        let region = &instrument.regions()[0];
        let spec = region.playback_spec(36, 111, RATES_EQUAL);
        assert_eq!(spec.note, 36);
        assert_eq!(spec.velocity, 111);
        assert_eq!(spec.pitch_keycenter, 60);
        assert_eq!(spec.transpose, -2);
        assert_eq!(spec.tune, 50);
        assert_eq!(spec.volume_db, -3.0);
        assert_eq!(spec.pan, -25.0);
        assert_eq!(spec.loop_mode, LoopMode::LoopSustain);
        assert_eq!(spec.loop_window.map(LoopWindow::len), Some(100));
        assert_eq!(spec.group, 7);
        assert_eq!(spec.off_by, 9);
        assert_eq!(spec.source_line, region.source_line);
        assert!(close(spec.gain, 10.0f32.powf(-3.0 / 20.0), 1.0e-6));
        assert!(close(spec.rate, region.pitch_ratio(36), 1.0e-9));
        // 同一输入 ⇒ 同一描述（ARCH-DET-001）。
        assert_eq!(spec, region.playback_spec(36, 111, RATES_EQUAL));
    }

    #[test]
    fn playback_for_selects_the_same_region_as_region_for() {
        let text = "<group>key=36 seq_length=2\n\
                    <region>seq_position=1 sample=k1.wav\n\
                    <region>seq_position=2 sample=k2.wav";
        let instrument = parse_text(text, &Default::default()).expect("parses");
        let rates = RenderRates::new(44_100.0, 48_000.0);

        let first = instrument
            .playback_for(RegionQuery::new(36, 100).with_occurrence(0), rates)
            .expect("region matches");
        assert_eq!(first.region.sample, "k1.wav");
        assert_eq!(first.spec.note, 36);
        assert_eq!(first.spec.velocity, 100);

        let second = instrument
            .playback_for(RegionQuery::new(36, 100).with_occurrence(1), rates)
            .expect("region matches");
        assert_eq!(second.region.sample, "k2.wav");

        // 与 region_for_with 的选择结果逐项一致（同一选择算法）。
        for occurrence in 0..6u64 {
            let query = RegionQuery::new(36, 100).with_occurrence(occurrence);
            let chosen = instrument.region_for_with(query);
            let played = instrument
                .playback_for(RegionQuery::new(36, 100).with_occurrence(occurrence), rates);
            assert_eq!(
                chosen.map(|region| region.sample.clone()),
                played.map(|play| play.region.sample.clone()),
                "occurrence {occurrence}"
            );
            assert_eq!(played.map(|play| play.spec.velocity), Some(100));
        }

        // 没有匹配 region ⇒ None（不 panic、不 fallback 到别的 region）。
        assert!(
            instrument
                .playback_for(RegionQuery::new(61, 100), rates)
                .is_none()
        );
        // CC 门控未提供状态 ⇒ 严格策略下不匹配。
        let gated = parse_text(
            "<region>sample=a.wav locc1=64 hicc1=127",
            &Default::default(),
        )
        .expect("parses");
        assert!(
            gated
                .playback_for(RegionQuery::new(60, 100), rates)
                .is_none()
        );
    }

    #[test]
    fn pitch_ratio_is_finite_positive_over_the_parsed_field_ranges() {
        // 字段范围来自 instrument.rs 的字段声明：note 0..=127、pitch_keycenter -127..=127、
        // transpose -127..=127、tune -100..=100（解析期已强制）。
        let mut ratios = Vec::new();
        for note in [0u8, 1, 60, 64, 127] {
            for pitch_keycenter in [-127i32, 0, 60, 127] {
                for transpose in [-127i32, -1, 0, 1, 127] {
                    for tune in [-100i32, 0, 100] {
                        let region = Region {
                            sample: std::borrow::Cow::Borrowed("a.wav"),
                            default_path: None,
                            lokey: 0,
                            hikey: 127,
                            pitch_keycenter,
                            trigger_by_note: true,
                            lovel: 0,
                            hivel: 127,
                            lochan: 1,
                            hichan: 16,
                            loop_start: 0,
                            loop_end: 0,
                            loop_mode: LoopMode::NoLoop,
                            tune,
                            transpose,
                            volume: 0.0,
                            pan: 0.0,
                            seq_position: 1,
                            seq_length: 1,
                            group: 0,
                            off_by: 0,
                            sw_last: None,
                            sw_lokey: 0,
                            sw_hikey: 127,
                            sw_down: None,
                            sw_up: None,
                            cc_gates: Vec::new(),
                            source_line: 1,
                        };
                        let ratio = region.pitch_ratio(note);
                        assert!(
                            ratio.is_finite() && ratio > 0.0,
                            "ratio {ratio} for note {note} keycenter {pitch_keycenter} \
                             transpose {transpose} tune {tune}"
                        );
                        ratios.push(ratio);
                    }
                }
            }
        }
        assert_eq!(ratios.len(), 5 * 4 * 5 * 3);
        assert!(ratios.iter().all(|ratio| ratio.is_finite()));
    }
}
