//! `T-FIX-277` 的判据：**全局 LFO 接进工程参数**（`lfo_wave` / `lfo_rate_hz` /
//! `lfo_depth` / `lfo_target`）。
//! [ARCH-DSP-001, ROAD-M2-006, ARCH-DET-001, ARCH-RT-001]
//!
//! ## 缺口是什么
//!
//! 器件侧 `yeban_dsp::polysynth::PolySynthParams::with_lfo` 早就存在（并有
//! 器件自己的单元判据 `tests/polysynth_lfo.rs`），但**引擎侧**的音色投影
//! `ToneParams` 没有 `lfo_wave` / `lfo_rate_hz` / `lfo_depth` / `lfo_target`
//! 四个字段，`poly_synth_params()` 也从不调用 `with_lfo` ⇒ 所有内置 PolySynth
//! 轨道只能用器件的默认 LFO（深度 `0` = 关），工程里写的 LFO 参数**一个也不进声音**。
//! 本票把四条投影接通，参数名固定为 `lfo_wave` / `lfo_rate_hz` / `lfo_depth` /
//! `lfo_target`（与同批 `T-FIX-273`（包络）/ `T-FIX-276`（滤波器包络）共用的命名口径）。
//!
//! ## 判据表（"怎么变红" = 注入；实测见交付回执）
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | L1 | **默认逐位不变**：不写四个键 ⇒ `poly_synth_params()` 与 `PolySynthParams::new()` 逐字段相同，渲染位模式相同 | 把 `TONE_DEFAULT_LFO_*` 任一个写偏 / `with_lfo` 只在"写了参数"时才调用 |
//! | L2 | **投影真的进器件**（读数）：每条键逐条可观测；大小写不敏感；波形/目标是下标（`0.5` 四舍五入到 `1`） | `from_devices` 少一个 `match` 臂 / `poly_synth_params` 丢掉 `.with_lfo` |
//! | L3 | **音频腿**：三个目标各自换声音（音高/振幅/截止各一条） | 某个目标没进器件 / 器件 `set_params` 之后又被覆盖 |
//! | L4 | **非法值回落 + 不是识别键**：非有限/负值 ⇒ 该参数取默认（`0.0` 合法）；回落后的渲染与默认**逐位相同**；只写 LFO 键的设备仍被跳过 | 把 `NaN` 原样存进字段 / 去掉负值守卫 / 把 LFO 键算进 `is_source` |
//! | L5 | **产品路径**：模型设备 → 快照 → 实时音频；换 `lfo_depth` ⇒ 换声音，快照读数逐字段对账 | 投影不读 LFO 键 / 快照丢字段 |
//!
//! 全部判据在 **`--no-default-features`（不编译 cpal）** 下运行，与
//! `synth_render.rs` / `synth_filter.rs` / `synth_osc.rs` / `synth_env.rs` 同一条主路径。

mod support;

use support::{NoteSpec, SynthRig, note_project, render, scheduled};
use yeban_dsp::oscillator::LfoWave;
use yeban_dsp::polysynth::{LfoTarget, PolySynthParams};
use yeban_engine::synth::{NoteSchedule, ToneParams};
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, ParameterValue};

/// 采样率（夹具固定 48 kHz）。
const SR: f32 = 48_000.0;

/// 一个长音符（96 000 帧 ≈ 2 s），够 40 个量子（5 120 帧）全程有声。
fn tone_schedule(pitch: u8) -> NoteSchedule {
    NoteSchedule::from_sorted(vec![scheduled(0, 96_000, pitch, 1.0, SR)])
}

/// 一段渲染的**位模式**（逐位比较的单位）。
fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|sample| sample.to_bits()).collect()
}

/// 用给定音色渲染 40 个量子并返回位模式。
fn render_tone(tone: &ToneParams) -> Vec<u32> {
    let mut rig = SynthRig::new(tone);
    bits(&rig.render(&tone_schedule(69), 40))
}

/// 一台 `InternalInstrument` 设备（夹具用；只改夹具，不改模型层）。
fn device(params: &[(&str, f32)]) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: "Hollow".to_owned(),
        kind: DeviceKind::InternalInstrument,
        bypassed: false,
        params: params
            .iter()
            .map(|(name, value)| ParameterValue {
                name: (*name).to_owned(),
                value: *value,
                unit: None,
            })
            .collect(),
        latency_samples: 0,
        plugin: None,
    }
}

/// 把设备链挂到夹具的那条 MIDI 轨上。
fn mount(devices: Vec<DeviceDefinition>, fixture: &mut support::Fixture) {
    let track = fixture.track;
    let entry = fixture
        .project
        .tracks
        .get_mut(&track)
        .expect("夹具里必须有那条 MIDI 轨");
    entry.devices = devices;
}

/// L1：**不写四个 LFO 键 ⇒ 与接通之前逐位相同**。
///
/// 两条腿：参数层（`poly_synth_params()` 与器件默认**逐字段相同**）与行为层
/// （渲染位模式与"显式给默认 LFO"逐位相同）。参数层这一条同时是"默认值写偏"
/// 的探针：`TONE_DEFAULT_LFO_*` 任一个与器件默认不一致，第一个断言当场红。
#[test]
fn l1_default_lfo_is_the_historical_sound() {
    // (a) 参数层：旁通 + 默认 LFO == 器件默认参数。
    assert_eq!(
        ToneParams::bypass().poly_synth_params(),
        PolySynthParams::new(),
        "不写 LFO 键必须等于器件的默认参数（否则'既有输出逐位不变'失去构造依据）"
    );
    let default = PolySynthParams::new();
    assert_eq!(default.lfo_wave(), LfoWave::Sine);
    assert_eq!(default.lfo_rate_hz(), 5.0);
    assert_eq!(default.lfo_depth(), 0.0);
    assert_eq!(default.lfo_target(), LfoTarget::Pitch);

    // 一台"只有滤波器键"的设备（本票之前唯一能被投影出来的形状）。
    let filter_only = ToneParams::from_devices(&[device(&[
        ("cutoff_hz", 1_200.0),
        ("resonance", 0.2),
        ("drive", 0.0),
    ])]);
    assert_eq!(filter_only.lfo_wave(), LfoWave::Sine);
    assert_eq!(filter_only.lfo_rate_hz(), 5.0);
    assert_eq!(filter_only.lfo_depth(), 0.0);
    assert_eq!(filter_only.lfo_target(), LfoTarget::Pitch);

    // (b) 行为层：默认 LFO 与"显式给默认值"逐位相同，且这个夹具真的出声。
    let implicit = render_tone(&ToneParams::new(1_200.0, 0.0, 0.0));
    let explicit = render_tone(&ToneParams::new(1_200.0, 0.0, 0.0).with_lfo(
        LfoWave::Sine,
        5.0,
        0.0,
        LfoTarget::Pitch,
    ));
    assert_eq!(
        implicit, explicit,
        "不写 LFO 与显式给默认值必须逐位相同（`with_lfo` 只能改它该改的四个字段）"
    );
    assert!(
        implicit.iter().any(|bits| *bits != 0.0f32.to_bits()),
        "夹具没有出声 —— 逐位相同是空转"
    );
}

/// L2：**投影真的进器件**（读数腿）。
///
/// 每一条 LFO 键都必须能被 `from_devices` 认下并出现在 `poly_synth_params()` 的读数里；
/// 名字大小写不敏感；波形/目标是下标（`f32` 先四舍五入再钳到 `u8`，`0.5` ⇒ `1`）；
/// 没写的那几条仍是默认。
#[test]
fn l2_each_lfo_key_reaches_the_device_projection() {
    let baseline = ToneParams::from_devices(&[device(&[("cutoff_hz", 1_000.0)])]);

    // (a) 一条一条地加：每次只改自己那一格。
    let tone = ToneParams::from_devices(&[device(&[("cutoff_hz", 1_000.0), ("LFO_WAVE", 1.0)])]);
    assert_ne!(
        tone, baseline,
        "`lfo_wave` 必须被投影认下（大小写不敏感：`LFO_WAVE`）—— 否则 from_devices 漏了这条臂"
    );
    let params = tone.poly_synth_params();
    assert_eq!(params.lfo_wave(), LfoWave::Triangle);
    assert_eq!(params.lfo_rate_hz(), 5.0, "`lfo_wave` 串到了速率格");
    assert_eq!(params.lfo_depth(), 0.0, "`lfo_wave` 串到了深度格");
    assert_eq!(
        params.lfo_target(),
        LfoTarget::Pitch,
        "`lfo_wave` 串到了目标格"
    );

    let tone = ToneParams::from_devices(&[device(&[("cutoff_hz", 1_000.0), ("Lfo_Rate_Hz", 7.5)])]);
    assert_ne!(tone, baseline, "`lfo_rate_hz` 必须被投影认下");
    let params = tone.poly_synth_params();
    assert_eq!(params.lfo_rate_hz(), 7.5);
    assert_eq!(params.lfo_wave(), LfoWave::Sine);
    assert_eq!(params.lfo_depth(), 0.0);
    assert_eq!(params.lfo_target(), LfoTarget::Pitch);

    let tone = ToneParams::from_devices(&[device(&[("cutoff_hz", 1_000.0), ("LFO_DEPTH", 100.0)])]);
    assert_ne!(tone, baseline, "`lfo_depth` 必须被投影认下");
    let params = tone.poly_synth_params();
    assert_eq!(params.lfo_depth(), 100.0);
    assert_eq!(params.lfo_wave(), LfoWave::Sine);
    assert_eq!(params.lfo_rate_hz(), 5.0);
    assert_eq!(params.lfo_target(), LfoTarget::Pitch);

    let tone = ToneParams::from_devices(&[device(&[("cutoff_hz", 1_000.0), ("Lfo_Target", 2.0)])]);
    assert_ne!(tone, baseline, "`lfo_target` 必须被投影认下");
    let params = tone.poly_synth_params();
    assert_eq!(params.lfo_target(), LfoTarget::Cutoff);
    assert_eq!(params.lfo_wave(), LfoWave::Sine);
    assert_eq!(params.lfo_rate_hz(), 5.0);
    assert_eq!(params.lfo_depth(), 0.0);

    // (b) 四个键一起写：投影出的四项**全部**是写的值。
    // 注：`lfo_wave = 0.5` 四舍五入到 `1`（三角），`lfo_target = 1.5` 四舍五入到 `2`（截止）——
    // 这是 `wave_index` 的取整口径，不是本票的bug。
    let all = ToneParams::from_devices(&[device(&[
        ("cutoff_hz", 1_000.0),
        ("lfo_wave", 0.5),
        ("lfo_rate_hz", 7.5),
        ("lfo_depth", 100.0),
        ("lfo_target", 1.5),
    ])]);
    let params = all.poly_synth_params();
    assert_eq!(
        (
            params.lfo_wave(),
            params.lfo_rate_hz(),
            params.lfo_depth(),
            params.lfo_target()
        ),
        (LfoWave::Triangle, 7.5, 100.0, LfoTarget::Cutoff),
        "四条键必须逐条进器件"
    );
}

/// L3：**音频腿**——三个目标各自真的换声音。
#[test]
fn l3_each_lfo_target_changes_the_audio() {
    let base = ToneParams::new(1_200.0, 0.0, 0.0);
    let default_bits = render_tone(&base);
    assert!(
        default_bits.iter().any(|bits| *bits != 0.0f32.to_bits()),
        "夹具没有出声 —— '换声音'的比较是空转"
    );

    // 音高目标：100 音分、5 Hz 正弦 ⇒ 40 个量子（≈107 ms）里相位走 0 → 0.53，
    // 弯音最高到 ~100 音分，位模式必须变。
    let pitch = base.with_lfo(LfoWave::Sine, 5.0, 100.0, LfoTarget::Pitch);
    assert_ne!(
        render_tone(&pitch),
        default_bits,
        "音高目标的 LFO 必须换声音 —— 否则 with_lfo 没进器件"
    );

    // 振幅目标：深度 0.5 ⇒ 同一段里增益 1 → 1.5 → 1，位模式必须变。
    let amplitude = base.with_lfo(LfoWave::Sine, 5.0, 0.5, LfoTarget::Amplitude);
    assert_ne!(
        render_tone(&amplitude),
        default_bits,
        "振幅目标的 LFO 必须换声音 —— 否则 with_lfo 没进器件"
    );

    // 截止目标：滤波器启用（`ToneParams::new` 本来就不旁通）+ 深度 2 八度 ⇒
    // 截止在 1200..4800 Hz 之间摆动，位模式必须变。
    let cutoff = base.with_lfo(LfoWave::Sine, 5.0, 2.0, LfoTarget::Cutoff);
    assert_ne!(
        render_tone(&cutoff),
        default_bits,
        "截止目标的 LFO 必须换声音 —— 否则 with_lfo 没进器件"
    );
}

/// L4：**非法值回落**（非有限/负值 ⇒ 默认；`0.0` 合法）+ **只写 LFO 键的设备不算音源**。
///
/// 回落之后整段渲染必须与默认**逐位相同** —— 这是"坏参数不会悄悄换音色"的证据。
#[test]
fn l4_illegal_lfo_values_fall_back_to_defaults() {
    let base = ToneParams::new(1_200.0, 0.0, 0.0);

    // (a) NaN / 负值 / inf 一律回落默认；`0.0` 是**合法值**（冻结/关），不是"没写"。
    let illegal = base.with_lfo(LfoWave::Sine, f32::NAN, -1.0, LfoTarget::Pitch);
    assert_eq!(illegal.lfo_rate_hz(), 5.0, "NaN 速率必须回落默认");
    assert_eq!(illegal.lfo_depth(), 0.0, "负值深度必须回落默认");
    assert_eq!(
        render_tone(&illegal),
        render_tone(&base),
        "回落之后必须与默认 LFO 逐位相同"
    );

    // 越界值钳到上限。
    let clamped = base.with_lfo(LfoWave::Sine, 500.0, 5000.0, LfoTarget::Pitch);
    assert_eq!(clamped.lfo_rate_hz(), 100.0, "速率必须钳到 100 Hz");
    assert_eq!(clamped.lfo_depth(), 1200.0, "深度必须钳到 1200");

    // `0.0` 速率是合法值（冻结），`0.0` 深度是合法值（关）—— 读数必须如实保留。
    let frozen = base.with_lfo(LfoWave::Sine, 0.0, 0.0, LfoTarget::Pitch);
    assert_eq!(frozen.lfo_rate_hz(), 0.0);
    assert_eq!(frozen.lfo_depth(), 0.0);

    // (b) 从设备链投影的那条路也走同一份兜底（`unwrap_or` + 本层钳制）。
    let nan_device = ToneParams::from_devices(&[device(&[
        ("cutoff_hz", 1_000.0),
        ("lfo_rate_hz", f32::NAN),
        ("lfo_depth", -2.0),
    ])]);
    assert_eq!(nan_device.lfo_rate_hz(), 5.0);
    assert_eq!(nan_device.lfo_depth(), 0.0);

    // (c) **只写 LFO 键的设备不算音源**（识别键规则没有放宽）。
    let lfo_only = ToneParams::from_devices(&[device(&[("lfo_depth", 100.0)])]);
    assert!(
        lfo_only.is_bypass(),
        "只写 LFO 键的设备必须仍被跳过（LFO 不是识别键）"
    );
    assert_eq!(lfo_only.lfo_depth(), 0.0, "被跳过的设备连 LFO 也不得带进来");
}

/// L5：**产品路径**（模型设备 → 快照 → 实时音频）里换 `lfo_depth` 真的换声音。
#[test]
fn l5_the_product_path_applies_the_lfo() {
    let mut fixture = note_project(&[NoteSpec::quarter(69)]);
    {
        let track = fixture.track;
        fixture
            .project
            .tracks
            .get_mut(&track)
            .expect("夹具里必须有那条 MIDI 轨")
            .volume_db = -6.0;
    }
    mount(
        vec![device(&[
            ("cutoff_hz", 1_200.0),
            ("lfo_wave", 0.0),
            ("lfo_rate_hz", 5.0),
            ("lfo_depth", 100.0),
            ("lfo_target", 0.0),
        ])],
        &mut fixture,
    );
    let with_lfo = render(&fixture.project, 60);
    assert!(with_lfo.nonzero() > 0, "夹具必须真的出声");

    // 快照侧读数：投影真的进了快照。
    let snapshot = yeban_engine::snapshot::EngineSnapshot::from_project(&fixture.project, 1)
        .expect("夹具工程必须能编译成快照");
    let tone = *snapshot.tone(&fixture.track).expect("该轨必须有音色投影");
    assert_eq!(tone.lfo_wave(), LfoWave::Sine, "快照必须带着 `lfo_wave`");
    assert_eq!(tone.lfo_rate_hz(), 5.0);
    assert_eq!(tone.lfo_depth(), 100.0);
    assert_eq!(tone.lfo_target(), LfoTarget::Pitch);

    // 把深度关到 0（短音符整段都在 LFO 的第一个周期里）⇒ 同一条产品路径给出不同的位模式。
    mount(
        vec![device(&[
            ("cutoff_hz", 1_200.0),
            ("lfo_wave", 0.0),
            ("lfo_rate_hz", 5.0),
            ("lfo_depth", 0.0),
            ("lfo_target", 0.0),
        ])],
        &mut fixture,
    );
    let without_lfo = render(&fixture.project, 60);
    assert!(
        without_lfo.nonzero() > 0,
        "关掉 LFO 的短音符仍必须出声（否则'听不见'被误当成'生效'）"
    );
    assert_ne!(
        with_lfo.fingerprint(),
        without_lfo.fingerprint(),
        "`lfo_depth` 必须经产品路径改变声音"
    );
    println!(
        "[engine-lfo] L5 depth=100 指纹={:#018x} depth=0 指纹={:#018x}",
        with_lfo.fingerprint(),
        without_lfo.fingerprint()
    );
}
