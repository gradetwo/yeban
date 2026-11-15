//! ⭐ **SFZ LFO 族**（v1 三族 `amplfo_*` / `fillfo_*` / `pitchlfo_*` ＋
//! 可分配 `lfoN_*` 四目标）的引擎判据。
//!
//! 解析、取值域与求值本体在 `yeban-sfz`（`lfo` 模块与 `instrument` 的
//! 模块测试覆盖）；本文件只判**引擎消费**：
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | L1 | generic 阶梯恒定 pitch ⇒ 与 `tune=1200` 参照逐位相同（速率 ×2） | 逐位 |
//! | L2 | `amplfo_delay` 内与裸 region 逐位相同；颤音真的调制恒值样本 | 逐位 ＋ 极值 |
//! | L3 | generic 阶梯恒定 cutoff ⇒ 截止 ×2 后带内正弦通过；无 `cutoff` 的 `fillfo_*` 透明 | RMS 比 ＋ 逐位 |
//! | L4 | generic 阶梯恒定 volume ⇒ 输出 ＝ 裸输出 × 10^(6/20) 逐位 | 逐位 |
//! | L5 | 三路一致：池渲染 == 离线渲染逐位（四族同时在场） | 逐位 |
//! | L6 | 池跨块延续：两块 == 一块逐位（LFO 状态写回正确） | 逐位 |
//! | L7 | CC 消费：`amplfo_depthcc1` 无探针 ⇒ 透明逐位；探针 127 ⇒ 颤音出现且池 == 离线 | 逐位 ＋ 极值 |
//! | L8 | generic 阶梯恒定 resonance ⇒ 截止处正弦被谐振抬升 | RMS 比 |
//! | L9 | `pitchlfo` 颤音真的弯音：过零数高于裸 region（音分域对称 ⇒ 均值速率 > 1） | 过零数 |
//!
//! 范围登记（见 `yeban_sfz::lfo` 模块文档）：`*_chanaft` / `*_polyaft`
//! 只解析不消费；`lfoN_amplitude` 未解析；LFO→LFO 未做；进引擎的
//! 振荡器上界 [`yeban_engine::sfz_instrument::LFO_OSC_CAP`]。

use std::collections::BTreeMap;
use std::sync::Arc;

use yeban_engine::sfz_instrument::{InMemorySample, PreparedSfz, SfzInstrument, SfzVoicePool};

/// 引擎采样率。
const HZ: f32 = 48_000.0;
/// 样本长度（帧）：0.1 s。
const PCM: usize = 4_800;
/// 输出长度（帧）：与样本同长。
const OUT: usize = 4_800;

/// 恒值样本：输出 = 恒值 × 增益链 ⇒ 增益调制可逐位对读。
fn constant() -> InMemorySample {
    InMemorySample {
        pcm: Arc::new(vec![0.5_f32; PCM]),
        sample_hz: HZ,
        loop_points: None,
    }
}

/// 指定频率的正弦样本（满幅 0.5）。
fn sine(freq: f32) -> InMemorySample {
    let pcm: Vec<f32> = (0..PCM)
        .map(|i| 0.5 * (2.0 * std::f32::consts::PI * freq * i as f32 / HZ).sin())
        .collect();
    InMemorySample {
        pcm: Arc::new(pcm),
        sample_hz: HZ,
        loop_points: None,
    }
}

fn rms(samples: &[f32]) -> f32 {
    let sum: f64 = samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|sample| sample.to_bits()).collect()
}

fn crossings(samples: &[f32]) -> usize {
    samples
        .windows(2)
        .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
        .count()
}

/// 池渲染一份 SFZ（广播样本、力度 100、整段按住），返回输出缓冲。
fn render_pool(text: &str, sample: &InMemorySample) -> Vec<f32> {
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let prepared = PreparedSfz::prepare(&sfz, sample, HZ).expect("摊平必须成功");
    let mut pool = SfzVoicePool::new(HZ);
    let created = pool.trigger_note(&prepared, 60, 100, OUT as u64, 1.0, None);
    assert_eq!(created, 1, "单区域夹具 ⇒ 恰好建 1 个声部");
    let mut out = vec![0.0_f32; OUT];
    pool.render_into(&prepared, 0, &mut out);
    out
}

/// 离线渲染一份 SFZ（样本表路径），返回输出缓冲。
fn render_offline(text: &str, sample: InMemorySample) -> Vec<f32> {
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let mut table = BTreeMap::new();
    table.insert("a.wav".to_owned(), sample);
    let mut out = vec![0.0_f32; OUT];
    sfz.render_note_into_from_samples(&table, 60, 100, Some(OUT as u64), None, &mut out, HZ)
        .expect("离线渲染必须成功");
    out
}

/// 离线渲染（广播样本 ＋ CC 探针），返回输出缓冲。
fn render_offline_cc(
    text: &str,
    sample: &InMemorySample,
    cc: Option<&dyn Fn(u8) -> u8>,
) -> Vec<f32> {
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let mut out = vec![0.0_f32; OUT];
    sfz.render_note_into_with_cc(sample, 60, 100, cc, 0.0, &mut out, HZ)
        .expect("离线渲染必须成功");
    out
}

/// ⭐ L1：generic 阶梯 LFO 恒定输出 1.0 × pitch 1200 音分 ⇒ 速率恰 ×2，
/// 与静态 `tune=1200` 的裸 region 逐位相同（同一合成点、同一乘子公式）。
#[test]
fn l1_generic_step_pitch_is_a_constant_rate_multiplier() {
    let lfo = render_offline(
        "<region> sample=a.wav pitch_keytrack=0 \
         lfo01_steps=1 lfo01_step01=100 lfo01_freq=1 lfo01_pitch=1200",
        sine(440.0),
    );
    let reference = render_offline(
        "<region> sample=a.wav pitch_keytrack=0 tune=1200",
        sine(440.0),
    );
    assert!(lfo.iter().any(|s| *s != 0.0), "夹具必须真的出声");
    assert_eq!(
        bits(&lfo),
        bits(&reference),
        "⭐ 阶梯恒定 pitch LFO 与静态 tune 参照逐位相同"
    );
}

/// ⭐ L2：`amplfo_delay` 内 LFO 输出恒 0 ⇒ 增益乘子恰 1.0 ⇒ 与裸 region
/// 逐位相同；delay 之后颤音出现（恒值样本的极值被拉开）。
#[test]
fn l2_amplfo_delay_is_transparent_then_tremolo_starts() {
    let plain = render_offline("<region> sample=a.wav pitch_keytrack=0", constant());
    let lfo = render_offline(
        "<region> sample=a.wav pitch_keytrack=0 \
         amplfo_delay=0.025 amplfo_freq=10 amplfo_depth=6",
        constant(),
    );
    assert_eq!(
        bits(&lfo[..1200]),
        bits(&plain[..1200]),
        "⭐ delay 内（前 1200 帧）与裸 region 逐位相同"
    );
    let tail = &lfo[1200..];
    let max = tail.iter().fold(0.0f32, |a, s| a.max(s.abs()));
    let min = tail.iter().fold(f32::MAX, |a, s| a.min(s.abs()));
    assert!(
        max > plain[3000].abs() * 1.5 && min < plain[3000].abs() * 0.6,
        "⭐ delay 后颤音必须把恒值拉开（max {max}, min {min}, 裸值 {})",
        plain[3000].abs()
    );
}

/// ⭐ L3：generic 阶梯恒定 cutoff +1200 音分 ⇒ 截止 1000 → 2000 Hz，
/// 1500 Hz 正弦由被压到通过；`fillfo_*` 写在无 `cutoff` 的区域上透明。
#[test]
fn l3_generic_step_cutoff_sweeps_and_fillfo_needs_cutoff() {
    let plain = render_offline(
        "<region> sample=a.wav pitch_keytrack=0 cutoff=1000 fil_type=lpf_2p",
        sine(1500.0),
    );
    let lfo = render_offline(
        "<region> sample=a.wav pitch_keytrack=0 cutoff=1000 fil_type=lpf_2p \
         lfo01_steps=1 lfo01_step01=100 lfo01_freq=1 lfo01_cutoff=1200",
        sine(1500.0),
    );
    assert!(
        rms(&lfo) > rms(&plain) * 1.6,
        "⭐ 截止 ×2 后 1500 Hz 必须通过（lfo {}, plain {}）",
        rms(&lfo),
        rms(&plain)
    );
    let no_cutoff_plain = render_offline("<region> sample=a.wav pitch_keytrack=0", sine(440.0));
    let no_cutoff_lfo = render_offline(
        "<region> sample=a.wav pitch_keytrack=0 fillfo_freq=5 fillfo_depth=1200",
        sine(440.0),
    );
    assert_eq!(
        bits(&no_cutoff_lfo),
        bits(&no_cutoff_plain),
        "⭐ 无 cutoff ⇒ fillfo 整体透明、逐位相同"
    );
}

/// ⭐ L4：generic 阶梯恒定 volume +6 dB ⇒ 每帧增益恰 ×10^(6/20)，
/// 输出 ＝ 裸输出 × 同一乘子（同一乘法链 ⇒ 逐位）。
#[test]
fn l4_generic_step_volume_is_a_constant_gain() {
    let plain = render_offline("<region> sample=a.wav pitch_keytrack=0", constant());
    let lfo = render_offline(
        "<region> sample=a.wav pitch_keytrack=0 \
         lfo01_steps=1 lfo01_step01=100 lfo01_freq=1 lfo01_volume=6",
        constant(),
    );
    let gain = 10.0f32.powf(6.0 / 20.0);
    let expected: Vec<f32> = plain.iter().map(|s| s * gain).collect();
    assert_eq!(
        bits(&lfo),
        bits(&expected),
        "⭐ 阶梯恒定 volume 与裸输出 × 常量增益逐位相同"
    );
}

/// ⭐ L5：四族同时在场，池渲染 == 离线渲染逐位（同一逐样本路）。
#[test]
fn l5_pool_matches_offline_with_all_families() {
    let text = "<region> sample=a.wav pitch_keytrack=0 cutoff=800 fil_type=lpf_2p \
                pitchlfo_freq=5 pitchlfo_depth=100 \
                amplfo_freq=3 amplfo_depth=2 \
                fillfo_freq=2 fillfo_depth=600 \
                lfo01_wave=6 lfo01_freq=1 lfo01_pitch=50 lfo01_volume=1 \
                lfo01_cutoff=-300 lfo01_resonance=6";
    let pooled = render_pool(text, &sine(440.0));
    let offline = render_offline(text, sine(440.0));
    assert!(pooled.iter().any(|s| *s != 0.0), "夹具必须真的出声");
    assert_eq!(
        bits(&pooled),
        bits(&offline),
        "⭐ 池与离线走同一逐样本路 ⇒ 逐位相同"
    );
    // 摊平表带出：v1 三台 ＋ generic 一台。
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("解析");
    let prepared = PreparedSfz::prepare(&sfz, &sine(440.0), HZ).expect("摊平");
    assert_eq!(prepared.regions()[0].lfo_defs(&prepared).len(), 4);
}

/// ⭐ L6：池跨块延续：两块渲染 == 一块逐位（共享计数写回正确，
/// 第二块的 LFO 相位不许从 0 重来）。
#[test]
fn l6_pool_block_boundaries_continue_lfo_phase() {
    let text = "<region> sample=a.wav pitch_keytrack=0 \
                pitchlfo_freq=5 pitchlfo_depth=300 amplfo_freq=7 amplfo_depth=3";
    let one_block = render_pool(text, &sine(440.0));
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let prepared = PreparedSfz::prepare(&sfz, &sine(440.0), HZ).expect("摊平必须成功");
    let mut pool = SfzVoicePool::new(HZ);
    assert_eq!(
        pool.trigger_note(&prepared, 60, 100, OUT as u64, 1.0, None),
        1
    );
    let mut two_blocks = vec![0.0_f32; OUT];
    pool.render_into(&prepared, 0, &mut two_blocks[..OUT / 2]);
    pool.render_into(&prepared, (OUT / 2) as u64, &mut two_blocks[OUT / 2..]);
    assert_eq!(
        bits(&one_block),
        bits(&two_blocks),
        "⭐ LFO 状态跨块写回 ⇒ 两块 == 一块逐位"
    );
}

/// ⭐ L7：`amplfo_depthcc1` 的消费：无探针 ⇒ 有效 depth 0 ⇒ 与裸 region
/// 逐位相同；探针 cc1=127 ⇒ 颤音出现，且池路与离线路逐位一致。
#[test]
fn l7_amplfo_depth_cc_is_consumed_from_the_probe() {
    let text = "<region> sample=a.wav pitch_keytrack=0 \
                amplfo_freq=10 amplfo_depthcc1=6";
    let plain = render_offline_cc("<region> sample=a.wav pitch_keytrack=0", &constant(), None);
    let no_cc = render_offline_cc(text, &constant(), None);
    assert_eq!(
        bits(&no_cc),
        bits(&plain),
        "⭐ 无探针 ⇒ depthcc 不生效、逐位相同"
    );
    let probe = |cc: u8| if cc == 1 { 127 } else { 0 };
    let with_cc = render_offline_cc(text, &constant(), Some(&probe));
    let max = with_cc.iter().fold(0.0f32, |a, s| a.max(s.abs()));
    assert!(
        max > plain[3000].abs() * 1.5,
        "⭐ 探针 127 ⇒ 颤音必须出现（max {max}）"
    );
    // 池路同一探针 ⇒ 与离线逐位一致。
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let prepared = PreparedSfz::prepare(&sfz, &constant(), HZ).expect("摊平必须成功");
    let mut pool = SfzVoicePool::new(HZ);
    assert_eq!(
        pool.trigger_note(&prepared, 60, 100, OUT as u64, 1.0, None),
        1
    );
    let mut pooled = vec![0.0_f32; OUT];
    pool.render_into_with_cc(&prepared, 0, &mut pooled, Some(&probe));
    assert_eq!(
        bits(&pooled),
        bits(&with_cc),
        "⭐ 池与离线在同一探针下逐位相同"
    );
}

/// ⭐ L8：generic 阶梯恒定 resonance +24 dB ⇒ 截止处（1000 Hz）正弦被
/// 谐振峰抬升（同截止、同输入，只差 resonance 目标）。
#[test]
fn l8_generic_step_resonance_lifts_the_cutoff_peak() {
    let plain = render_offline(
        "<region> sample=a.wav pitch_keytrack=0 cutoff=1000 fil_type=lpf_2p",
        sine(1000.0),
    );
    let lfo = render_offline(
        "<region> sample=a.wav pitch_keytrack=0 cutoff=1000 fil_type=lpf_2p \
         lfo01_steps=1 lfo01_step01=100 lfo01_freq=1 lfo01_resonance=24",
        sine(1000.0),
    );
    assert!(
        rms(&lfo) > rms(&plain) * 1.3,
        "⭐ 谐振 +24 dB 必须抬升截止处（lfo {}, plain {}）",
        rms(&lfo),
        rms(&plain)
    );
}

/// ⭐ L9：`pitchlfo_depth=1200`（±1 八度）颤音：音分域对称摆动经 exp2
/// 后均值速率 > 1 ⇒ 0.1 s（恰 1 个 LFO 周期）内过零数高于裸 region。
#[test]
fn l9_pitchlfo_vibrato_raises_the_zero_crossing_rate() {
    let plain = render_offline("<region> sample=a.wav pitch_keytrack=0", sine(440.0));
    let lfo = render_offline(
        "<region> sample=a.wav pitch_keytrack=0 pitchlfo_freq=10 pitchlfo_depth=1200",
        sine(440.0),
    );
    assert!(
        crossings(&lfo) > crossings(&plain) + 3,
        "⭐ 颤音均值速率必须更高（lfo {}, plain {}）",
        crossings(&lfo),
        crossings(&plain)
    );
}
