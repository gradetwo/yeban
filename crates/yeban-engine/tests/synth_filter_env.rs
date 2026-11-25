//! `T-FIX-276` 的判据：**滤波器包络接进工程参数**（`filter_env_attack_s` /
//! `filter_env_decay_s` / `filter_env_sustain` / `filter_env_release_s` /
//! `filter_env_depth`）。
//!
//! # 为什么要有它
//!
//! `T-FIX-273` 把**振幅**包络接通之后，合成器仍然只有一套包络：截止频率在音符
//! 生命期内不动，"短促的滤波打击感"（pluck）做不出来。`T-FIX-277` 的 LFO 能扫
//! 截止，但它是**全局周期**的、不随音符起落 ⇒ 也不是包络。
//!
//! 本票把**第二套包络**（滤波器包络）接进 `PolySynthParams` / `PolyVoice`：每个声部
//! 一只 `Adsr`，与振幅包络**同一帧** `gate_on / gate_off`，渲染时以
//! `cutoff × 2^(depth × 包络值)` 在**绝对位置 64 样本网格**上重配置滤波器
//! （系数重算含 `tan`，逐样本路径不含超越函数）。
//!
//! # 判据
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | E1 | 不写五个键 ⇒ 与器件默认**逐字段相同**、渲染**逐位相同** | 默认值写偏 / 去掉 `depth > 0` 开关 |
//! | E2 | 五条键逐条可观测（大小写不敏感） | `from_devices` 少一个 `match` 臂 / `poly_synth_params` 丢掉 `.with_filter_env` |
//! | E3 | 非法值回落（NaN / 负值 ⇒ 默认；深度钳到 4 八度；elev 钳到 0..1） | 把 `NaN` 原样存进字段 / 去掉守卫 |
//! | E4 | **音频腿**：`depth > 0` ⇒ 换声音；`depth = 0` ⇒ 不换声音 | 投影不读包络键 / 包络不进器件 |
//!
//! 判据 E1 的音频腿同时钉住"既有工程逐位不变"（本票的兼容性承诺）：不写这五个键
//! 的工程投影出来仍是深度 `0` ⇒ 器件的第二只包络一次也不推进。

mod support;

use support::{NoteSpec, SynthRig, note_project, render, scheduled};
use yeban_dsp::polysynth::PolySynthParams;
use yeban_engine::synth::{NoteSchedule, ToneParams};
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, ParameterValue};

/// 采样率（夹具固定 48 kHz）。
const SR: f32 = 48_000.0;

/// 一个长音符（96 000 帧 ≈ 2 s）。
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

/// 一台 `InternalInstrument` 设备（夹具用）。
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

/// 一个有滤波器（不旁通）的音色底座。
fn filtered() -> ToneParams {
    ToneParams::new(300.0, 0.3, 0.0)
}

/// **E1**：不写五个键 ⇒ 与器件默认逐字段相同、渲染逐位相同。
#[test]
fn e1_absent_filter_env_is_the_historical_sound() {
    // (a) 参数层：旁通 + 不写键 == 器件默认参数。
    assert_eq!(
        ToneParams::bypass().poly_synth_params(),
        PolySynthParams::new(),
        "不写滤波器包络键必须等于器件的默认参数"
    );
    let defaults = PolySynthParams::new();
    assert_eq!(defaults.filter_env_attack_s(), 0.0);
    assert_eq!(defaults.filter_env_decay_s(), 0.0);
    assert_eq!(defaults.filter_env_sustain(), 1.0);
    assert_eq!(defaults.filter_env_release_s(), 0.0);
    assert_eq!(defaults.filter_env_depth(), 0.0);

    // (b) 显式深度 0 与不写键，在**滤波器启用**的音色上逐位相同。
    let implicit = filtered();
    let explicit_zero = filtered().with_filter_env(0.2, 0.3, 0.4, 0.5, 0.0);
    assert_eq!(
        render_tone(&implicit),
        render_tone(&explicit_zero),
        "深度 0 必须与不写键逐位相同"
    );
}

/// **E2**：五条键逐条可观测（大小写不敏感）。
#[test]
fn e2_every_filter_env_key_reaches_the_projection() {
    // 底座：滤波器启用 + 五个键都给默认值。
    let base = [
        ("cutoff_hz", 300.0),
        ("filter_env_attack_s", 0.0),
        ("filter_env_decay_s", 0.0),
        ("filter_env_sustain", 1.0),
        ("filter_env_release_s", 0.0),
        ("filter_env_depth", 0.0),
    ];
    // 每条键注入一个**远离默认**的值；`(键, 注入值, 期望读数)`。
    type Readout = fn(&ToneParams) -> f32;
    let cases: [(&str, f32, Readout); 5] = [
        ("filter_env_attack_s", 0.25, ToneParams::filter_env_attack_s),
        ("filter_env_decay_s", 0.35, ToneParams::filter_env_decay_s),
        ("filter_env_sustain", 0.45, ToneParams::filter_env_sustain),
        (
            "filter_env_release_s",
            0.55,
            ToneParams::filter_env_release_s,
        ),
        ("filter_env_depth", 0.65, ToneParams::filter_env_depth),
    ];
    let mut checked = 0usize;
    for (name, value, read) in cases {
        let mut params: Vec<(&str, f32)> = base.to_vec();
        params.retain(|(existing, _)| *existing != name);
        params.push((name, value));
        let tone = ToneParams::from_devices(&[device(&params)]);
        assert_eq!(
            read(&tone),
            value,
            "键 {name} 必须被投影认下（大小写不敏感）"
        );
        checked += 1;
    }
    assert_eq!(checked, 5, "五条键必须全部被测到，实得 {checked}");
    // 大小写不敏感：大写名同一条臂。
    let upper =
        ToneParams::from_devices(&[device(&[("cutoff_hz", 300.0), ("FILTER_ENV_DEPTH", 1.5)])]);
    assert_eq!(upper.filter_env_depth(), 1.5);
}

/// **E3**：非法值回落 + 钳制。
#[test]
fn e3_illegal_values_fall_back_and_clamp() {
    let illegal = ToneParams::from_devices(&[device(&[
        ("cutoff_hz", 300.0),
        ("filter_env_attack_s", f32::NAN),
        ("filter_env_decay_s", -1.0),
        ("filter_env_sustain", f32::INFINITY),
        ("filter_env_release_s", f32::NEG_INFINITY),
        ("filter_env_depth", f32::NAN),
    ])]);
    assert_eq!(illegal.filter_env_attack_s(), 0.0);
    assert_eq!(illegal.filter_env_decay_s(), 0.0);
    assert_eq!(illegal.filter_env_sustain(), 1.0);
    assert_eq!(illegal.filter_env_release_s(), 0.0);
    assert_eq!(illegal.filter_env_depth(), 0.0);

    let clamped = ToneParams::from_devices(&[device(&[
        ("cutoff_hz", 300.0),
        ("filter_env_sustain", 2.0),
        ("filter_env_depth", 9.0),
    ])]);
    assert_eq!(clamped.filter_env_sustain(), 1.0);
    assert_eq!(clamped.filter_env_depth(), 4.0);
    // 钳制后的参数进器件也是钳制值（两道防线同解）。
    assert_eq!(
        clamped.poly_synth_params().filter_env_depth(),
        4.0,
        "器件层必须与引擎层同解"
    );
}

/// **E4**：音频腿 —— `depth > 0` 换声音，`depth = 0` 不换。
#[test]
fn e4_depth_changes_the_sound_only_when_positive() {
    let base = render_tone(&filtered());
    let zero = render_tone(&filtered().with_filter_env(0.05, 0.1, 0.1, 0.2, 0.0));
    assert_eq!(base, zero, "深度 0 不得改变任何样本位");
    let opened = render_tone(&filtered().with_filter_env(0.05, 0.1, 0.1, 0.2, 3.0));
    assert_ne!(base, opened, "深度 3 八度必须改变声音（否则包络没进器件）");
    assert_eq!(opened.len(), base.len());
}

/// **E5**：产品路径（模型设备 → 快照 → 渲染）：五条键真的进了快照与声音。
#[test]
fn e5_the_product_path_applies_the_filter_envelope() {
    let mut fixture = note_project(&[NoteSpec::quarter(69)]);
    let track = fixture.track;
    fixture
        .project
        .tracks
        .get_mut(&track)
        .expect("夹具里必须有那条 MIDI 轨")
        .devices = vec![device(&[
        ("cutoff_hz", 1_200.0),
        ("filter_env_attack_s", 0.05),
        ("filter_env_decay_s", 0.1),
        ("filter_env_sustain", 0.1),
        ("filter_env_release_s", 0.2),
        ("filter_env_depth", 3.0),
    ])];
    let opened = render(&fixture.project, 60);
    assert!(opened.nonzero() > 0, "夹具必须真的出声");

    let snapshot = yeban_engine::snapshot::EngineSnapshot::from_project(&fixture.project, 1)
        .expect("夹具工程必须能编译成快照");
    let tone = *snapshot.tone(&track).expect("该轨必须有音色投影");
    assert_eq!(tone.filter_env_attack_s(), 0.05);
    assert_eq!(tone.filter_env_decay_s(), 0.1);
    assert_eq!(tone.filter_env_sustain(), 0.1);
    assert_eq!(tone.filter_env_release_s(), 0.2);
    assert_eq!(tone.filter_env_depth(), 3.0);

    // 深度关到 0 ⇒ 同一条产品路径给出不同的位模式。
    fixture
        .project
        .tracks
        .get_mut(&track)
        .expect("夹具里必须有那条 MIDI 轨")
        .devices = vec![device(&[
        ("cutoff_hz", 1_200.0),
        ("filter_env_attack_s", 0.05),
        ("filter_env_decay_s", 0.1),
        ("filter_env_sustain", 0.1),
        ("filter_env_release_s", 0.2),
        ("filter_env_depth", 0.0),
    ])];
    let closed = render(&fixture.project, 60);
    assert_ne!(
        render_bits(&opened),
        render_bits(&closed),
        "深度 3 与深度 0 必须在产品路径上换声音"
    );
}

/// `Render` 的逐位读入口（左右声道拼起来）。
fn render_bits(render: &support::Render) -> Vec<u32> {
    render
        .left
        .iter()
        .chain(render.right.iter())
        .map(|sample| sample.to_bits())
        .collect()
}
