//! 噪声门 / 扩展器 / 侧链闪避器 集成判据 [ARCH-RT-001] [ARCH-DSP-001]

use yeban_dsp::gate::{GateDetector, GateMode, NoiseGate, NoiseGateParams};
use yeban_dsp::math::db_to_gain;

#[test]
fn noise_gate_contract_gate_opens_and_closes() {
    let params = NoiseGateParams {
        threshold_db: -30.0,
        range_db: -70.0,
        attack_s: 0.001,
        hold_s: 0.010,
        release_s: 0.020,
        hysteresis_db: 2.0,
        mode: GateMode::Gate,
        detector: GateDetector::Peak,
        ..Default::default()
    };
    let mut gate = NoiseGate::new(params, 48000.0);

    // 1. 静音输入下保持关门并给出最大衰减
    let mut silence = vec![0.0f32; 480];
    gate.process_mono(&mut silence);
    assert!(!gate.is_open());
    assert!(gate.current_gain_db() <= -69.0);

    // 2. 注入突发大信号（-6 dBFS）触发开门
    let mut burst = vec![db_to_gain(-6.0); 480];
    gate.process_mono(&mut burst);
    assert!(gate.is_open());
    assert!(gate.current_gain_db() > -0.1);

    // 3. 信号切断后，Hold 时间内依然保持开门
    let mut gap = vec![0.0f32; 240]; // 5 ms (小于 10 ms hold)
    gate.process_mono(&mut gap);
    assert!(gate.is_open(), "在 hold 时间内应保持开门");

    // 4. 超过 hold + release 后彻底关闭
    let mut long_tail = vec![0.0f32; 4800]; // 100 ms
    gate.process_mono(&mut long_tail);
    assert!(!gate.is_open());
    assert!(gate.current_gain_db() <= -65.0);
}

#[test]
fn noise_gate_contract_ducker_mode_attenuates_on_key() {
    let params = NoiseGateParams {
        threshold_db: -20.0,
        range_db: -18.0,
        attack_s: 0.001,
        hold_s: 0.020,
        release_s: 0.050,
        mode: GateMode::Ducker,
        ..Default::default()
    };
    let mut ducker = NoiseGate::new(params, 48000.0);

    let mut prog_l = vec![db_to_gain(-3.0); 2400];
    let mut prog_r = vec![db_to_gain(-3.0); 2400];
    let key_quiet = vec![db_to_gain(-40.0); 2400];

    // 键信号安静时：节目不衰减 (0 dB)
    ducker.process_stereo_with_key(&mut prog_l, &mut prog_r, &key_quiet, &key_quiet);
    assert!(!ducker.is_open());
    assert!(ducker.current_gain_db() > -0.1);

    // 键信号爆发（-6 dBFS，高于 -20 dB 阈值）：节目信号被压低 18 dB
    let key_loud = vec![db_to_gain(-6.0); 2400];
    ducker.process_stereo_with_key(&mut prog_l, &mut prog_r, &key_loud, &key_loud);
    assert!(ducker.is_open());
    let g = ducker.current_gain_db();
    assert!(
        (g - -18.0).abs() < 0.5,
        "闪避衰减应达到 -18 dB，实际: {g} dB"
    );
}

#[test]
fn noise_gate_contract_key_filters_isolate_triggers() {
    let params = NoiseGateParams {
        threshold_db: -20.0,
        hpf_hz: 200.0,  // 过滤 200 Hz 以下
        lpf_hz: 1500.0, // 过滤 1500 Hz 以上
        mode: GateMode::Gate,
        ..Default::default()
    };
    let mut gate = NoiseGate::new(params, 48000.0);

    // 1. 低频次声 (50 Hz) 键信号被 HPF 过滤，不触发开门
    let mut low_key = Vec::with_capacity(2400);
    for i in 0..2400 {
        let s = db_to_gain(-3.0)
            * libm::sinf(2.0 * core::f32::consts::PI * 50.0 * (i as f32) / 48000.0);
        low_key.push(s);
    }
    let mut prog = vec![0.5f32; 2400];
    gate.process_mono_with_key(&mut prog, &low_key);
    assert!(!gate.is_open(), "50 Hz 次声应被 HPF 过滤，不误开门");

    // 2. 高频杂音 (8000 Hz) 键信号被 LPF 过滤，不触发开门
    let mut high_key = Vec::with_capacity(2400);
    for i in 0..2400 {
        let s = db_to_gain(-3.0)
            * libm::sinf(2.0 * core::f32::consts::PI * 8000.0 * (i as f32) / 48000.0);
        high_key.push(s);
    }
    gate.process_mono_with_key(&mut prog, &high_key);
    assert!(!gate.is_open(), "8000 Hz 高频杂音应被 LPF 过滤，不误开门");

    // 3. 通带内信号 (600 Hz) 正常通过滤波器，触发开门
    let mut mid_key = Vec::with_capacity(2400);
    for i in 0..2400 {
        let s = db_to_gain(-6.0)
            * libm::sinf(2.0 * core::f32::consts::PI * 600.0 * (i as f32) / 48000.0);
        mid_key.push(s);
    }
    gate.process_mono_with_key(&mut prog, &mid_key);
    assert!(gate.is_open(), "600 Hz 通带信号应正常触发开门");
}
