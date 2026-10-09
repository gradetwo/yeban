//! `line/engine-2` 的判据：**引擎侧的波形选择与第二条振荡器**（缺口 R4）。
//! [ARCH-DSP-001, ROAD-M2-006, ARCH-DET-001, ARCH-RT-001]
//!
//! ## 缺口是什么、登记在哪里
//!
//! 器件（`yeban_dsp::polysynth`）早就是**双振荡器**（两条独立的相位累加器，各自的
//! 波表下标 / 电平 / 失谐），但引擎侧的临时形状 `ToneParams` 只投影了滤波器三个旋钮
//! ⇒ `PolySynthParams::new()` 的单振荡器默认音色是引擎**唯一**能发出的声音：
//! "引擎选不了波形、也开不了第二条振荡器"。这条缺口在
//! `docs/ledger/engine-mix-notes.md` §8.1 的 R4 行（2026-10-08 补记）与
//! `crates/yeban-engine/src/synth.rs` 的模块文档 §4 里都有登记。
//!
//! 本票把它接通：`ToneParams` 多了 `osc1_wave` / `osc2_wave` / `osc2_level` /
//! `osc2_detune_cents` 四个投影；波形库在**构造期**按
//! `yeban_dsp::oscillator` 的五张公开配方建起来（下标 0 = 历史默认的 `HOLLOW`）。
//!
//! ## `line/engine-5`：剩下那条"`osc1` 电平/失谐"缺口（判据 O8）
//!
//! 上一票接通四条支路参数时，`osc1` 仍被写死成"满电平、不失谐"，并在
//! `crates/yeban-engine/src/synth.rs` 与 `crates/yeban-engine/src/lib.rs` 的模块文档里
//! 登记为**部分缺口**。本票补上 `osc1_level` / `osc1_detune_cents` 两个键。
//! **默认值是历史默认**（`1.0` / `0.0`）⇒ 不写这两个键的工程与接通之前**逐位相同**；
//! O8(a) 用"逐字段重建历史投影"（`OscSettings::new(table, 1.0, 0.0)`）把这条钉住。
//!
//! ## 判据表（"怎么变红" = 注入；实测见本票报告）
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | O1 | 不给振荡器参数 ⇒ **逐位**等于显式给下标 0；五个下标两两产生**不同**的位模式 | 波形库只装一张表 / 顺序换掉 |
//! | O2 | 库有 5 张表、名字集合 == `FACTORY_RECIPES`、下标 0 = `hollow`、越界钳到最后一张 | 少建表 / 名字表与配方表脱钩 |
//! | O3 | 投影真的到了器件（读数 + 音频）：`osc2` 关 = 不给参数、开 = 不同、失谐 = 不同 | `poly_synth_params` 丢掉振荡器 |
//! | O4 | 投影规则：只有振荡器键 ⇒ 滤波器旁通、只写 `resonance` 不算来源、`bypassed` 忽略、大小写不敏感、跳过非来源设备 | `is_source` 放宽 / 规则改向 |
//! | O5 | 钳制：波形**四舍五入**再钳、非有限电平 ⇒ 回该字段的默认、越界电平钳到 `0..=1` | 截断代替四舍五入 / 去掉非有限守卫 |
//! | O6 | **产品路径**：模型设备 → 快照 → 实时音频；换波形 ⇒ 换声音 | 投影不读振荡器键 |
//! | O7 | 确定性 + 有限性：同输入两次逐位相同，满共振 + 第二条支路下无 `NaN`/`inf` | 引入熵源 / 让坏参数穿过去 |
//! | O8 | `osc1` **电平/失谐**：不写 ⇒ 逐字段等于历史投影（`1.0` / `0.0`）；写了 ⇒ 换声音；电平 `0` ⇒ 静音；只写 `osc1_level` 的设备**是**音色来源 | `with_osc1` 的默认值改掉 / `poly_synth_params` 不传 `osc1` 的两个字段 / 两个键漏出识别键 |
//!
//! 全部判据在 **`--no-default-features`（不编译 cpal）** 下运行，与
//! `synth_render.rs` / `synth_filter.rs` / `mix_render.rs` 同一条主路径。

mod support;

use support::{NoteSpec, SynthRig, note_project, render, scheduled};
use yeban_dsp::polysynth::{OscSettings, PolySynthParams};
use yeban_engine::synth::{NoteSchedule, SynthEngine, ToneParams};
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

/// O1：**默认（不给振荡器参数）与"显式给下标 0"逐位相同**，而 1..=4 都是另一种声音。
///
/// 这是"既有工程输出逐位不变"的**行为**证据；它的**构造**证据是
/// `ToneParams::bypass().poly_synth_params() == PolySynthParams::new()`
/// （参数层面逐字段相同 ⇒ 器件逐样本算式一字未动）。
#[test]
fn o1_default_oscillator_params_are_the_historical_sound() {
    // (a) 参数层面：本层投影出来的器件参数与器件的默认参数**逐字段相同**。
    assert_eq!(
        ToneParams::bypass().poly_synth_params(),
        PolySynthParams::new(),
        "旁通 + 默认振荡器必须等于器件的默认参数（否则'既有输出逐位不变'失去构造依据）"
    );

    // (b) 行为层面：不给参数与显式给下标 0 逐位相同。
    let implicit = render_tone(&ToneParams::bypass());
    let explicit_zero = render_tone(&ToneParams::bypass().with_oscillators(0, 0, 0.0, 0.0));
    assert_eq!(
        implicit, explicit_zero,
        "不给振荡器参数与显式给下标 0（hollow）必须逐位相同"
    );
    // 覆盖度自检：这个夹具必须真的出声，否则"逐位相同"在全零上也成立（假绿）。
    assert!(
        implicit.iter().any(|bits| *bits != 0.0f32.to_bits()),
        "夹具没有出声 —— 逐位相同是空转"
    );

    // (c) 五个下标两两不同 ⇒ 库里真的五张表、下标 0 就是历史那一种。
    let mut seen: Vec<(u8, Vec<u32>)> = vec![(0, implicit.clone())];
    for wave in 1u8..=4 {
        let rendered = render_tone(&ToneParams::bypass().with_oscillators(wave, 0, 0.0, 0.0));
        for (other, pattern) in &seen {
            assert_ne!(
                &rendered, pattern,
                "波形 {wave} 与波形 {other} 的位模式相同 —— 库里两张表退化成了一张"
            );
        }
        seen.push((wave, rendered));
    }
    println!(
        "[engine-osc] O1 五个波形的位模式两两不同；不给参数 == 下标 0（逐位）；\
         首样本位模式={:#010x}",
        implicit[0]
    );
}

/// O2：波形库的**张数、名字、下标 0、越界口径**。
///
/// 名字集合必须与器件自己的工厂配方集合**相同**：引擎只决定顺序，
/// 不另立第二份 DSP 定义。
#[test]
fn o2_wave_library_has_five_named_tables() {
    let engine = SynthEngine::new(48_000);
    let names: Vec<&str> = (0..engine.table_count())
        .map(|table| engine.table_name(table))
        .collect();
    assert_eq!(
        names,
        vec!["hollow", "organ", "vocal", "metallic", "glass"],
        "波形库的名字/顺序是契约（下标 0 必须是历史默认的 hollow）"
    );
    let factory: Vec<&str> = yeban_dsp::oscillator::FACTORY_RECIPES
        .iter()
        .map(|(name, _)| *name)
        .collect();
    assert_eq!(
        names.len(),
        factory.len(),
        "引擎库与器件工厂配方的张数必须相同（多一张 = 凭空发明的波形）"
    );
    for name in &names {
        assert!(
            factory.contains(name),
            "库里的 {name} 不在 yeban_dsp::oscillator::FACTORY_RECIPES 里 —— 第二份 DSP 定义"
        );
    }
    assert_eq!(names[0], "hollow", "下标 0 必须是历史默认音色");
    assert_eq!(
        engine.table_name(99),
        "glass",
        "越界下标必须钳到最后一张表（与器件的 OscSettings 同口径）"
    );
    assert_eq!(
        engine.table_levels(),
        yeban_dsp::oscillator::LEVELS,
        "每张表的 mip 级数不因建了五张表而变化"
    );

    // 尺寸读数（诊断；不进结论 —— 见字段文档里记录的那两个数）。
    println!(
        "[engine-osc] O2 tables={} names={names:?} size_of: ToneParams={} SynthEngine={} EngineRuntime={}",
        engine.table_count(),
        core::mem::size_of::<ToneParams>(),
        core::mem::size_of::<SynthEngine>(),
        core::mem::size_of::<yeban_engine::rt::EngineRuntime>(),
    );
}

/// O3：振荡器参数**真的到了器件那一侧**（读数 + 音频两条腿）。
///
/// "关掉 = 不给"这一条是**逐位**的：`osc2_level == 0.0` 时第二条支路的波表下标与
/// 失谐都不参与输出（器件对关掉的支路一帧也不执行）。
#[test]
fn o3_oscillator_settings_reach_the_device() {
    // (a) 读数腿：武装进去的那个数就是投影进去的那个数。
    let armed = ToneParams::new(1_200.0, 0.0, 0.0).with_oscillators(2, 4, 0.5, 7.0);
    let rig = SynthRig::new(&armed);
    assert_eq!(
        rig.engine.tone_params(rig.track),
        Some(armed),
        "音色读数必须逐字段等于武装进去的参数"
    );

    // (b) 音频腿：第二条支路开/关、失谐 0/7 音分必须给出不同的位模式。
    let base = ToneParams::new(1_200.0, 0.0, 0.0);
    let osc2_off = render_tone(&base.with_oscillators(2, 4, 0.0, 7.0));
    let osc2_absent = render_tone(&base.with_oscillators(2, 0, 0.0, 0.0));
    let osc2_on = render_tone(&base.with_oscillators(2, 4, 0.5, 0.0));
    let osc2_detuned = render_tone(&base.with_oscillators(2, 4, 0.5, 7.0));

    assert_eq!(
        osc2_off, osc2_absent,
        "电平 0 的第二条支路必须与'根本不给第二条支路'逐位相同（关掉 = 一帧也不执行）"
    );
    assert_ne!(
        osc2_on, osc2_off,
        "开了第二条支路（电平 0.5）必须改变样本 —— 否则投影没到器件"
    );
    assert_ne!(
        osc2_detuned, osc2_on,
        "失谐 7 音分必须改变样本 —— 否则失谐没进器件"
    );
    assert_ne!(
        render_tone(&base.with_oscillators(1, 0, 0.0, 0.0)),
        render_tone(&base.with_oscillators(4, 0, 0.0, 0.0)),
        "两条不同的波表下标必须给出不同的音色"
    );
    println!(
        "[engine-osc] O3 armed={:?}；osc2 关==不给={}；osc2 开≠关；失谐≠不失谐",
        rig.engine.tone_params(rig.track),
        osc2_off == osc2_absent,
    );
}

/// O4：**投影规则**（`ToneParams::from_devices`，纯函数）。
#[test]
fn o4_projection_rules_are_explicit() {
    // (a) 只有振荡器键、没有截止频率 ⇒ 滤波器整段旁通，波形照常生效。
    let osc_only = ToneParams::from_devices(&[device(&[("osc1_wave", 1.0)])]);
    assert!(osc_only.is_bypass(), "只有振荡器键时滤波器必须旁通");
    assert_eq!(osc_only.osc1_wave(), 1);
    assert_eq!(
        osc_only.cutoff_hz(),
        ToneParams::bypass().cutoff_hz(),
        "旁通时的截止频率是占位值，不参与声音"
    );
    assert_ne!(
        render_tone(&osc_only),
        render_tone(&ToneParams::bypass()),
        "只有振荡器键的设备必须真的改变音色（不是静默忽略）"
    );

    // (b) 截止频率 + 全部振荡器键（六个）一起生效。
    let full = ToneParams::from_devices(&[device(&[
        ("cutoff_hz", 1_500.0),
        ("resonance", 0.25),
        ("osc1_wave", 4.0),
        ("osc2_wave", 1.0),
        ("osc1_level", 0.75),
        ("osc1_detune_cents", -6.0),
        ("osc2_level", 0.5),
        ("osc2_detune_cents", 7.0),
    ])]);
    assert!(!full.is_bypass());
    assert_eq!(full.cutoff_hz(), 1_500.0);
    assert_eq!(full.resonance(), 0.25);
    assert_eq!(full.osc1_wave(), 4);
    assert_eq!(full.osc2_wave(), 1);
    assert_eq!(full.osc1_level(), 0.75);
    assert_eq!(full.osc1_detune_cents(), -6.0);
    assert_eq!(full.osc2_level(), 0.5);
    assert_eq!(full.osc2_detune_cents(), 7.0);

    // (c) 只写 resonance / drive 的设备**不是**音色来源（上移前的规则）。
    //
    // ⚠ 这一臂**单独看是没有牙的**：把 `resonance` 也算成识别键时，那台设备照样投影出
    // `bypass()`（没有截止频率 ⇒ 滤波器旁通，振荡器取默认）⇒ 两种实现的返回值逐字段相同。
    // 本票的注入 I4 实测过这一点（`5 passed` 之外的"red"只来自 O5，本臂没红）。
    // 有牙的形式是下面的 (e)：**第一台**用只写 `resonance` 的设备，断言**第二台**的
    // `osc1_wave` 必须被取到 —— 那样 I4 立刻变红。
    assert!(
        ToneParams::from_devices(&[device(&[("resonance", 0.5)])]).is_bypass(),
        "只写 resonance 不能被当成音色来源"
    );
    assert!(
        ToneParams::from_devices(&[device(&[("drive", 0.5)])]).is_bypass(),
        "只写 drive 不能被当成音色来源"
    );

    // (d) bypassed 的乐器设备整体忽略 ⇒ 与"没有设备"同解。
    let mut bypassed = device(&[("osc1_wave", 4.0), ("cutoff_hz", 400.0)]);
    bypassed.bypassed = true;
    assert_eq!(
        ToneParams::from_devices(&[bypassed]),
        ToneParams::bypass(),
        "bypassed 的设备必须整体忽略"
    );

    // (e) 跳过不是来源的设备，取**第一条**写了识别键的设备。
    //     前两台（只有 resonance / 只有 drive）必须被跳过；第三台才是音色来源。
    let skipped = ToneParams::from_devices(&[
        device(&[("resonance", 0.5)]),
        device(&[("drive", 0.5)]),
        device(&[("osc1_wave", 3.0), ("cutoff_hz", 800.0)]),
        device(&[("osc1_wave", 1.0)]),
    ]);
    assert_eq!(
        skipped.osc1_wave(),
        3,
        "必须取第一条写了识别键的设备（把 resonance 误当识别键会让这里取到默认下标 0）"
    );
    assert_eq!(skipped.cutoff_hz(), 800.0);

    // (f) 参数名大小写不敏感（含本票新增的两个键）。
    let upper = ToneParams::from_devices(&[device(&[
        ("OSC1_WAVE", 2.0),
        ("OSC1_LEVEL", 0.5),
        ("OSC1_DETUNE_CENTS", 4.0),
        ("OSC2_LEVEL", 0.5),
    ])]);
    assert_eq!(upper.osc1_wave(), 2);
    assert_eq!(upper.osc1_level(), 0.5);
    assert_eq!(upper.osc1_detune_cents(), 4.0);
    assert_eq!(upper.osc2_level(), 0.5);

    // (g) 非内置设备（外部乐器）不参与投影。
    let mut external = device(&[("osc1_wave", 4.0)]);
    external.kind = DeviceKind::ExternalInstrument;
    assert_eq!(ToneParams::from_devices(&[external]), ToneParams::bypass());
}

/// O5：**构造期钳制**（越界、非有限、小数下标），绝不产生 `NaN`。
#[test]
fn o5_oscillator_params_are_clamped_at_construction() {
    // (a) 波形下标：四舍五入，再钳到 0..=255；非有限/负值 ⇒ 0。
    let wave =
        |value: f32| ToneParams::from_devices(&[device(&[("osc1_wave", value)])]).osc1_wave();
    assert_eq!(wave(2.6), 3, "2.6 必须四舍五入成 3（截断会给出 2）");
    assert_eq!(wave(1.4), 1);
    assert_eq!(wave(-3.0), 0);
    assert_eq!(wave(f32::NAN), 0);
    assert_eq!(wave(f32::INFINITY), 0);
    assert_eq!(wave(1.0e9), 255, "越界下标钳到 255，再由器件钳到最后一张表");

    // (b) osc2 电平：越界钳到 0..=1；非有限 ⇒ 0（**本层的**默认 = 关）。
    let level = |value: f32| {
        ToneParams::bypass()
            .with_oscillators(0, 1, value, 0.0)
            .osc2_level()
    };
    assert_eq!(level(5.0), 1.0);
    assert_eq!(level(-2.0), 0.0);
    assert_eq!(level(f32::NAN), 0.0, "非有限电平必须退回本层的默认值（关）");
    assert_eq!(level(f32::INFINITY), 0.0);

    // (c) 失谐：非有限 ⇒ 0；有限值原样保留（量程由器件再钳一次）。
    let detune = |value: f32| {
        ToneParams::bypass()
            .with_oscillators(0, 1, 0.5, value)
            .osc2_detune_cents()
    };
    assert_eq!(detune(f32::NAN), 0.0);
    assert_eq!(detune(7.0), 7.0);

    // (c2) **`osc1` 的电平/失谐**（本票）：钳制口径与 `osc1` 自己的历史默认一致
    //      —— 电平的非有限值退回 `1.0`（满），**不是** `osc2` 的"关"。
    let osc1_level = |value: f32| ToneParams::bypass().with_osc1(value, 0.0).osc1_level();
    assert_eq!(osc1_level(1.5), 1.0, "越界电平钳到 1");
    assert_eq!(osc1_level(-0.5), 0.0, "负电平钳到 0");
    assert_eq!(
        osc1_level(f32::NAN),
        1.0,
        "非有限电平必须退回 `osc1` 的历史默认（满），而不是 `osc2` 的关"
    );
    assert_eq!(osc1_level(0.25), 0.25);
    let osc1_detune = |value: f32| {
        ToneParams::bypass()
            .with_osc1(1.0, value)
            .osc1_detune_cents()
    };
    assert_eq!(osc1_detune(f32::NAN), 0.0);
    assert_eq!(osc1_detune(-9.0), -9.0);
    assert_eq!(
        ToneParams::bypass().osc1_level(),
        1.0,
        "旁通的默认是历史默认：第一条支路满电平"
    );
    assert_eq!(ToneParams::bypass().osc1_detune_cents(), 0.0);
    assert_eq!(ToneParams::new(900.0, 0.0, 0.0).osc1_level(), 1.0);
    assert_eq!(ToneParams::new(900.0, 0.0, 0.0).osc1_detune_cents(), 0.0);

    // (c3) 从设备链投影出来的非有限 `osc1` 参数也走同一条兜底（不经 `with_osc1` 的
    //      公共入口，走 `from_projection` 的 `unwrap_or`）。
    let nan_device = ToneParams::from_devices(&[device(&[
        ("osc1_wave", 1.0),
        ("osc1_level", f32::NAN),
        ("osc1_detune_cents", f32::NAN),
    ])]);
    assert_eq!(nan_device.osc1_level(), 1.0);
    assert_eq!(nan_device.osc1_detune_cents(), 0.0);

    // (d) 退化参数不得把输出变成非有限值，也不得弄成静音。
    for tone in [
        ToneParams::bypass().with_oscillators(255, 255, 1.0, 100_000.0),
        ToneParams::bypass().with_oscillators(9, 9, f32::NAN, f32::NAN),
    ] {
        let mut rig = SynthRig::new(&tone);
        let rendered = rig.render(&tone_schedule(69), 20);
        assert!(
            rendered.iter().all(|sample| sample.is_finite()),
            "退化振荡器参数 {tone:?} 产生了非有限输出"
        );
        assert!(
            rendered.iter().any(|sample| *sample != 0.0),
            "退化振荡器参数 {tone:?} 把链路弄成静音（钳制过头）"
        );
    }
}

/// O6：**产品路径**（模型设备 → 快照 → 实时音频）里换波形真的换声音。
#[test]
fn o6_the_product_path_selects_the_waveform() {
    let mut fixture = note_project(&[NoteSpec::quarter(69)]);
    // 音量 −6 dB：让整段留在母线限制器的透明区，"换了波形"不与被限制器压过混在一起。
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
            ("osc1_wave", 1.0),
            ("osc2_wave", 4.0),
            ("osc2_level", 0.5),
            ("osc2_detune_cents", 7.0),
        ])],
        &mut fixture,
    );

    let hollow_like = render(&fixture.project, 60);
    assert!(hollow_like.nonzero() > 0, "夹具必须真的出声");
    assert_eq!(
        hollow_like.stats.limiter_gain_reductions, 0,
        "夹具必须整体低于限制器阈值（否则读数被测的不是波形）"
    );

    // 快照侧读数：投影真的进了快照。
    let snapshot = yeban_engine::snapshot::EngineSnapshot::from_project(&fixture.project, 1)
        .expect("夹具工程必须能编译成快照");
    let tone = *snapshot.tone(&fixture.track).expect("该轨必须有音色投影");
    assert!(tone.is_bypass(), "只有振荡器键 ⇒ 滤波器旁通");
    assert_eq!(tone.osc1_wave(), 1);
    assert_eq!(tone.osc2_wave(), 4);
    assert_eq!(tone.osc2_level(), 0.5);
    assert_eq!(tone.osc2_detune_cents(), 7.0);

    // 换一个波形下标 ⇒ 同一条产品路径给出**不同**的位模式。
    mount(
        vec![device(&[
            ("osc1_wave", 4.0),
            ("osc2_wave", 4.0),
            ("osc2_level", 0.5),
            ("osc2_detune_cents", 7.0),
        ])],
        &mut fixture,
    );
    let glass = render(&fixture.project, 60);
    assert_ne!(
        hollow_like.fingerprint(),
        glass.fingerprint(),
        "换波形下标必须换声音（产品路径）"
    );
    println!(
        "[engine-osc] O6 hollow(1)+osc2 指纹={:#018x} glass(4)+osc2 指纹={:#018x}",
        hollow_like.fingerprint(),
        glass.fingerprint()
    );
}

/// O7：**确定性 + 有限性**（第二条支路开着也一样）。
#[test]
fn o7_oscillator_path_is_deterministic_and_finite() {
    let tone = ToneParams::new(800.0, 0.9, 0.0).with_oscillators(1, 4, 0.7, -13.0);
    let first = render_tone(&tone);
    let second = render_tone(&tone);
    assert_eq!(first, second, "同一输入两次渲染必须逐位相同");
    assert!(
        first.iter().all(|bits| f32::from_bits(*bits).is_finite()),
        "第二条支路开着时输出必须全部有限"
    );
}

/// O8：**第一条支路的电平/失谐**（`line/engine-5` 补的那条投影）。
///
/// 三条腿：
/// - **(a) 逐字段重建历史投影**：不写 `osc1_level` / `osc1_detune_cents` 的设备，
///   它的 `poly_synth_params()` 必须**逐字段等于**接通之前那一份（`osc1` 支路是
///   `OscSettings::new(下标, 1.0, 0.0)`）。这是"接线不改变既有输出"的**构造**证据：
///   器件的逐样本算式一字未动，投影出来的参数也一字未动。
/// - **(b) 音频腿**：写了电平/失谐 ⇒ 换声音；电平 `0.0` ⇒ 整段**静音**（第一条支路
///   在器件里"一帧也不执行"）。
/// - **(c) 产品路径 + 识别键**：只写 `osc1_level` 的设备**必须是**音色来源，并且
///   `osc1_level` 经 模型 → 快照 → 实时音频 真的生效。
#[test]
fn o8_first_oscillator_level_and_detune_are_projected() {
    // (a) 历史投影的逐字段重建（唯一一处"手抄旧投影"的地方）。
    let old_keys = ToneParams::from_devices(&[device(&[
        ("osc1_wave", 2.0),
        ("osc2_wave", 4.0),
        ("osc2_level", 0.5),
        ("osc2_detune_cents", 7.0),
    ])]);
    assert_eq!(old_keys.osc1_level(), 1.0, "不写电平 ⇒ 历史默认（满）");
    assert_eq!(
        old_keys.osc1_detune_cents(),
        0.0,
        "不写失谐 ⇒ 历史默认（0）"
    );
    let historical = PolySynthParams::new()
        .with_oscillators(OscSettings::new(2, 1.0, 0.0), OscSettings::new(4, 0.5, 7.0))
        .with_filter(ToneParams::bypass().cutoff_hz(), 0.0, 0.0, true);
    assert_eq!(
        old_keys.poly_synth_params(),
        historical,
        "不写两个新键 ⇒ 投影出来的器件参数必须逐字段等于接通之前那一份"
    );
    assert_eq!(
        render_tone(&old_keys),
        render_tone(&ToneParams::bypass().with_oscillators(2, 4, 0.5, 7.0)),
        "同上，行为层：逐位相同"
    );

    // (b) 音频腿：电平与失谐各自可观测；电平 0.0 ⇒ 静音。
    let base = ToneParams::new(1_200.0, 0.0, 0.0);
    let full = render_tone(&base);
    let trimmed = render_tone(&base.with_osc1(0.5, 0.0));
    let detuned = render_tone(&base.with_osc1(0.5, 7.0));
    assert_ne!(
        trimmed, full,
        "`osc1_level = 0.5` 必须改变样本 —— 否则电平没进器件"
    );
    assert_ne!(
        detuned, trimmed,
        "`osc1_detune_cents = 7` 必须改变样本 —— 否则失谐没进器件"
    );
    let silent = render_tone(&base.with_osc1(0.0, 0.0));
    assert!(
        silent.iter().all(|bits| *bits == 0),
        "`osc1_level = 0.0`（第二条支路本来就关）必须给出精确静音"
    );
    assert!(
        full.iter().any(|bits| *bits != 0),
        "对照组必须真的出声（否则上面的静音判据是空真）"
    );

    // (c) 产品路径：只写 `osc1_level` 的设备是音色来源，且这个数真的生效。
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
    mount(vec![device(&[("osc1_level", 0.5)])], &mut fixture);
    let snapshot = yeban_engine::snapshot::EngineSnapshot::from_project(&fixture.project, 1)
        .expect("夹具工程必须能编译成快照");
    let tone = *snapshot.tone(&fixture.track).expect("该轨必须有音色投影");
    assert!(
        tone.is_bypass(),
        "只有 `osc1_level` ⇒ 没有截止频率 ⇒ 滤波器旁通"
    );
    assert_eq!(tone.osc1_level(), 0.5, "投影必须把电平带进快照");
    assert_eq!(tone.osc1_detune_cents(), 0.0);
    let half = render(&fixture.project, 60);
    assert!(half.nonzero() > 0, "夹具必须真的出声");

    mount(vec![device(&[("osc1_level", 1.0)])], &mut fixture);
    let loud = render(&fixture.project, 60);
    assert_ne!(
        half.fingerprint(),
        loud.fingerprint(),
        "`osc1_level` 必须经产品路径改变声音"
    );
    println!(
        "[engine-osc] O8 historical==rebuilt={}；0.5≠1.0={}；静音={}；产品路径指纹 {:#018x}≠{:#018x}",
        old_keys.poly_synth_params() == historical,
        trimmed != full,
        silent.iter().all(|bits| *bits == 0),
        half.fingerprint(),
        loud.fingerprint()
    );
}
