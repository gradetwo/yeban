//! `T-FIX-273` 的判据：**振幅包络接进工程参数**（`attack_s` / `decay_s` /
//! `sustain` / `release_s`）。
//! [ARCH-DSP-001, ROAD-M2-006, ARCH-DET-001, ARCH-RT-001]
//!
//! ## 缺口是什么
//!
//! 器件侧 `yeban_dsp::polysynth::PolySynthParams::with_envelope` 早就存在（并有
//! 器件自己的单元判据），但**引擎侧**的音色投影 `ToneParams` 没有
//! `attack_s` / `decay_s` / `sustain` / `release_s` 四个字段，
//! `poly_synth_params()` 也从不调用 `with_envelope` ⇒ 所有内置 PolySynth 轨道只能用
//! 器件的默认包络（`5 ms / 80 ms / 0.7 / 50 ms`），工程里写的包络参数**一个也不进声音**。
//! 本票把四条投影接通，参数名固定为 `attack_s` / `decay_s` / `sustain` / `release_s`
//! （与同批 `T-FIX-276`（滤波器包络）/ `T-FIX-277`（LFO）共用的命名口径）。
//!
//! ## 判据表（"怎么变红" = 注入；实测见交付回执）
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | E1 | **默认逐位不变**：不写四个键 ⇒ `poly_synth_params()` 与 `PolySynthParams::new()` 逐字段相同，渲染位模式相同 | 把 `TONE_DEFAULT_*` 任一个写偏 / `with_envelope` 只在"写了参数"时才调用 |
//! | E2 | **投影真的进器件**（读数）：`attack_s = 0.5` 与不写 ⇒ `ToneParams` 不同、`poly_synth_params().attack_s()` 不再是 `0.005`；四条键逐条可观测；大小写不敏感 | `from_devices` 少一个 `match` 臂 / `poly_synth_params` 丢掉 `.with_envelope` |
//! | E3 | **音频腿**：自定义 ADSR 与默认包络的渲染位模式不同，且长起振段里振幅显著更小 | 包络没进器件 / 器件 `set_params` 之后又被覆盖 |
//! | E4 | **非法值回落**：非有限/负值 ⇒ 该参数取默认（`0.0` 合法），`sustain` 钳到 `0..=1`；回落后的渲染与默认**逐位相同** | 把 `NaN` 原样存进字段 / 去掉负值守卫 |
//! | E5 | **产品路径**：模型设备 → 快照 → 实时音频；换 `attack_s` ⇒ 换声音，快照读数逐字段对账 | 投影不读包络键 / 快照丢字段 |
//!
//! 全部判据在 **`--no-default-features`（不编译 cpal）** 下运行，与
//! `synth_render.rs` / `synth_filter.rs` / `synth_osc.rs` 同一条主路径。

mod support;

use support::{NoteSpec, SynthRig, note_project, render, scheduled};
use yeban_dsp::polysynth::PolySynthParams;
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

/// 一条轨的**最大**样本绝对值（"起振段有多响"的读数）。
fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs()))
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

/// E1：**不写四个包络键 ⇒ 与接通之前逐位相同**。
///
/// 两条腿：参数层（`poly_synth_params()` 与器件默认**逐字段相同**）与行为层
/// （渲染位模式与"显式给默认包络"逐位相同）。参数层这一条同时是"默认值写偏"
/// 的探针：`TONE_DEFAULT_*` 任一个与器件默认不一致，第一个断言当场红。
#[test]
fn e1_default_envelope_is_the_historical_sound() {
    // (a) 参数层：旁通 + 默认包络 == 器件默认参数。
    assert_eq!(
        ToneParams::bypass().poly_synth_params(),
        PolySynthParams::new(),
        "不写包络键必须等于器件的默认参数（否则'既有输出逐位不变'失去构造依据）"
    );
    let default = PolySynthParams::new();
    assert_eq!(
        (
            default.attack_s(),
            default.decay_s(),
            default.sustain(),
            default.release_s()
        ),
        (0.005, 0.08, 0.7, 0.05),
        "器件默认包络的读数变了 —— 本票的 `TONE_DEFAULT_*` 必须跟着改"
    );

    // 一台"只有滤波器键"的设备（本票之前唯一能被投影出来的形状）。
    let filter_only = ToneParams::from_devices(&[device(&[
        ("cutoff_hz", 1_200.0),
        ("resonance", 0.2),
        ("drive", 0.0),
    ])]);
    assert_eq!(filter_only.attack_s(), 0.005);
    assert_eq!(filter_only.decay_s(), 0.08);
    assert_eq!(filter_only.sustain(), 0.7);
    assert_eq!(filter_only.release_s(), 0.05);

    // (b) 行为层：默认包络与"显式给默认值"逐位相同，且这个夹具真的出声。
    let implicit = render_tone(&ToneParams::new(1_200.0, 0.0, 0.0));
    let explicit =
        render_tone(&ToneParams::new(1_200.0, 0.0, 0.0).with_envelope(0.005, 0.08, 0.7, 0.05));
    assert_eq!(
        implicit, explicit,
        "不写包络与显式给默认值必须逐位相同（`with_envelope` 只能改它该改的四个字段）"
    );
    assert!(
        implicit.iter().any(|bits| *bits != 0.0f32.to_bits()),
        "夹具没有出声 —— 逐位相同是空转"
    );
}

/// E2：**投影真的进器件**（读数腿）。
///
/// 每一条包络键都必须能被 `from_devices` 认下并出现在 `poly_synth_params()` 的读数里；
/// 名字大小写不敏感；没写的那几条仍是默认。
#[test]
fn e2_each_envelope_key_reaches_the_device_projection() {
    let baseline = ToneParams::from_devices(&[device(&[("cutoff_hz", 1_000.0)])]);

    // (a) 一条一条地加：每次只改自己那一格。
    let cases: [(&str, &str, f32); 4] = [
        ("attack_s", "ATTACK_S", 0.5),
        ("decay_s", "Decay_S", 0.5),
        ("sustain", "SUSTAIN", 0.5),
        ("release_s", "Release_S", 0.5),
    ];
    for (lower, mixed_case, value) in cases {
        let tone =
            ToneParams::from_devices(&[device(&[("cutoff_hz", 1_000.0), (mixed_case, value)])]);
        assert_ne!(
            tone, baseline,
            "`{lower}` 必须被投影认下（大小写不敏感：`{mixed_case}`）—— 否则 from_devices 漏了这条臂"
        );
        let params = tone.poly_synth_params();
        let read = match lower {
            "attack_s" => params.attack_s(),
            "decay_s" => params.decay_s(),
            "sustain" => params.sustain(),
            "release_s" => params.release_s(),
            other => panic!("夹具里出现了未登记的名字 {other}"),
        };
        assert_eq!(
            read, value,
            "`{lower}` 必须出现在 `poly_synth_params()` 的读数里（否则 with_envelope 被丢掉）"
        );
        // 其余三格仍是默认 ⇒ "一条键只改一格"。
        let defaults = params_defaults_except(lower);
        assert_eq!(
            params_value(&params, defaults.0),
            defaults.1,
            "`{lower}` 串到了别的包络字段"
        );
    }

    // (b) 四个键一起写：投影出的四项**全部**是写的值。
    let all = ToneParams::from_devices(&[device(&[
        ("cutoff_hz", 1_000.0),
        ("attack_s", 0.5),
        ("decay_s", 0.25),
        ("sustain", 0.3),
        ("release_s", 0.4),
    ])]);
    let params = all.poly_synth_params();
    assert_eq!(
        (
            params.attack_s(),
            params.decay_s(),
            params.sustain(),
            params.release_s()
        ),
        (0.5, 0.25, 0.3, 0.4),
        "四条键必须逐条进器件"
    );
}

/// [`e2_each_envelope_key_reaches_the_device_projection`] 的辅助：给定"被改的那一格"，
/// 返回另一格的名字与它的默认值（用来钉住"一条键只改一格"）。
fn params_defaults_except(changed: &str) -> (&'static str, f32) {
    match changed {
        "attack_s" => ("decay_s", 0.08),
        "decay_s" => ("sustain", 0.7),
        "sustain" => ("release_s", 0.05),
        _ => ("attack_s", 0.005),
    }
}

/// 按名字读 `PolySynthParams` 的一格（[`params_defaults_except`] 的配套）。
fn params_value(params: &PolySynthParams, name: &str) -> f32 {
    match name {
        "attack_s" => params.attack_s(),
        "decay_s" => params.decay_s(),
        "sustain" => params.sustain(),
        "release_s" => params.release_s(),
        other => panic!("未登记的名字 {other}"),
    }
}

/// E3：**音频腿**——自定义 ADSR 真的换声音，且长起振段里振幅显著更小。
#[test]
fn e3_custom_envelope_changes_the_audio() {
    let base = ToneParams::new(1_200.0, 0.0, 0.0);
    let mut rig = SynthRig::new(&base);
    let default_render = rig.render(&tone_schedule(69), 40);
    let default_bits = bits(&default_render);

    let long_attack = base.with_envelope(0.5, 0.2, 0.6, 0.4);
    let mut rig = SynthRig::new(&long_attack);
    let slow_render = rig.render(&tone_schedule(69), 40);
    assert_ne!(
        default_bits,
        bits(&slow_render),
        "换了包络必须换样本 —— 否则 with_envelope 没进器件"
    );

    // 长起振的读数：0.5 s 起振在 5 120 帧（≈107 ms）里只走到 ~21%，峰值必须显著更低。
    let slow_peak = peak(&slow_render);
    let default_peak = peak(&default_render);
    assert!(
        slow_peak > 0.0 && slow_peak < 0.6 * default_peak,
        "0.5 s 起振的峰值 {slow_peak} 必须远低于默认包络 {default_peak}（否则包络没生效）"
    );

    // 反过来：`sustain = 0.0` + 短衰减 ⇒ 尾段明显更轻（同一段时间窗里能量更小）。
    let dropped = base.with_envelope(0.005, 0.05, 0.0, 0.05);
    let mut rig = SynthRig::new(&dropped);
    let dropped_render = rig.render(&tone_schedule(69), 40);
    assert_ne!(
        default_bits,
        bits(&dropped_render),
        "`sustain = 0.0` 必须换声音"
    );
    assert!(
        peak(&dropped_render) <= peak(&default_render),
        "把延音电平拧到 0 不该比默认更响"
    );
}

/// E4：**非法值回落**（非有限/负值 ⇒ 默认；`0.0` 合法；`sustain` 钳到 `0..=1`）。
///
/// 回落之后整段渲染必须与默认**逐位相同** —— 这是"坏参数不会悄悄换音色"的证据。
#[test]
fn e4_illegal_envelope_values_fall_back_to_defaults() {
    let base = ToneParams::new(1_200.0, 0.0, 0.0);

    // (a) NaN / 负值 / inf 一律回落默认；`0.0` 是**合法值**（瞬时），不是"没写"。
    let illegal = base.with_envelope(f32::NAN, -1.0, f32::NAN, f32::INFINITY);
    assert_eq!(illegal.attack_s(), 0.005, "NaN 起振必须回落默认");
    assert_eq!(illegal.decay_s(), 0.08, "负值衰减必须回落默认");
    assert_eq!(illegal.sustain(), 0.7, "NaN 延音必须回落默认");
    assert_eq!(illegal.release_s(), 0.05, "inf 释放必须回落默认");
    assert_eq!(
        render_tone(&illegal),
        render_tone(&base),
        "回落之后必须与默认包络逐位相同"
    );

    assert_eq!(base.with_envelope(0.0, 0.0, 0.0, 0.0).attack_s(), 0.0);
    assert_eq!(base.with_envelope(0.0, 0.0, 0.0, 0.0).sustain(), 0.0);
    assert_ne!(
        render_tone(&base.with_envelope(0.0, 0.0, 0.0, 0.0)),
        render_tone(&base),
        "`0.0` 必须真的生效（瞬间起振/瞬间释放是另一种声音）"
    );

    // (b) `sustain` 的钳制。
    assert_eq!(base.with_envelope(0.01, 0.01, 5.0, 0.01).sustain(), 1.0);
    assert_eq!(base.with_envelope(0.01, 0.01, -3.0, 0.01).sustain(), 0.0);

    // (c) 从设备链投影的那条路也走同一份兜底（`unwrap_or` + 本层钳制）。
    let nan_device = ToneParams::from_devices(&[device(&[
        ("cutoff_hz", 1_000.0),
        ("attack_s", f32::NAN),
        ("decay_s", -2.0),
        ("sustain", f32::INFINITY),
        ("release_s", -0.5),
    ])]);
    assert_eq!(nan_device.attack_s(), 0.005);
    assert_eq!(nan_device.decay_s(), 0.08);
    assert_eq!(nan_device.sustain(), 0.7);
    assert_eq!(nan_device.release_s(), 0.05);

    // (d) **只写包络键的设备不算音源**（识别键规则没有放宽）。
    let envelope_only = ToneParams::from_devices(&[device(&[("attack_s", 0.5)])]);
    assert!(
        envelope_only.is_bypass(),
        "只写包络键的设备必须仍被跳过（包络不是识别键）"
    );
    assert_eq!(
        envelope_only.attack_s(),
        0.005,
        "被跳过的设备连包络也不得带进来"
    );
}

/// E5：**产品路径**（模型设备 → 快照 → 实时音频）里换 `attack_s` 真的换声音。
#[test]
fn e5_the_product_path_applies_the_envelope() {
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
        vec![device(&[("cutoff_hz", 1_200.0), ("attack_s", 0.02)])],
        &mut fixture,
    );
    let quick = render(&fixture.project, 60);
    assert!(quick.nonzero() > 0, "夹具必须真的出声");

    // 快照侧读数：投影真的进了快照。
    let snapshot = yeban_engine::snapshot::EngineSnapshot::from_project(&fixture.project, 1)
        .expect("夹具工程必须能编译成快照");
    let tone = *snapshot.tone(&fixture.track).expect("该轨必须有音色投影");
    assert_eq!(tone.attack_s(), 0.02, "快照必须带着 `attack_s`");
    assert_eq!(tone.decay_s(), 0.08);
    assert_eq!(tone.sustain(), 0.7);
    assert_eq!(tone.release_s(), 0.05);

    // 把起振拉长到 0.5 s（短音符整段都在起振段）⇒ 同一条产品路径给出不同的位模式。
    mount(
        vec![device(&[("cutoff_hz", 1_200.0), ("attack_s", 0.5)])],
        &mut fixture,
    );
    let slow = render(&fixture.project, 60);
    assert!(
        slow.nonzero() > 0,
        "0.5 s 起振的短音符仍必须出声（否则'听不见'被误当成'生效'）"
    );
    assert_ne!(
        quick.fingerprint(),
        slow.fingerprint(),
        "`attack_s` 必须经产品路径改变声音"
    );
    println!(
        "[engine-env] E5 0.02s 指纹={:#018x} 0.5s 指纹={:#018x}",
        quick.fingerprint(),
        slow.fingerprint()
    );
}
