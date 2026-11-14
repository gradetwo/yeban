//! ⭐ **SFZ 滤波静态族**（`fil_type` / `cutoff` / `resonance` /
//! `fil_keytrack` / `fil_keycenter` / `fil_veltrack`）的引擎判据。
//!
//! 解析与取值域在 `yeban-sfz`（其模块测试覆盖）；本文件只判**引擎消费**：
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | F1 | 无 `cutoff` ⇒ **逐位不变**（哪怕写了 `fil_type`/`resonance`/`fil_keytrack`） | 与裸 region 的池渲染逐位相同 |
//! | F2 | `cutoff=0` ⇒ 与无 `cutoff` **逐位相同**（ARIA 口径，见 `yeban-sfz` 的 `Region::cutoff`） | 逐位 |
//! | F3 | `lpf_2p` 真的滤波：Nyquist 内容被压掉；`hpf_2p` 压掉低频正弦 | RMS 比 |
//! | F4 | 三条路同值：带滤波时**池渲染 == 离线渲染**逐位 | 逐位 |
//! | F5 | `fil_keytrack` 生效：高键的截止被推高 ⇒ 高频内容回来 | RMS 比（同 region、两键对照） |
//! | F6 | 池跨块延续：两块渲染 == 一块渲染逐位（滤波递推状态写回正确） | 逐位 |
//! | F7 | 窃取/提升路径不 panic 且出声（提升期现场配置滤波） | 有限、非静音 |
//! | F8 | 摊平表原样带出六个标量 | `PreparedRegion` 访问器 |
//!
//! 夹具 PCM 是 **Nyquist 交替**（`+0.5, −0.5, …`）：全部能量在最高频，
//! 低通开/关的差别最大；`pitch_keytrack=0` 让所有键的速率恰为 1
//! （交替形态在任何键上不被重采样破坏）。

use std::collections::BTreeMap;
use std::sync::Arc;

use yeban_engine::sfz_instrument::{InMemorySample, PreparedSfz, SfzInstrument, SfzVoicePool};
use yeban_sfz::FilterType;

/// 引擎采样率。
const HZ: f32 = 48_000.0;
/// 样本长度（帧）：0.1 s。
const PCM: usize = 4_800;
/// 输出长度（帧）：与样本同长。
const OUT: usize = 4_800;

/// Nyquist 交替样本（全部能量在 Nyquist）。
fn nyquist() -> InMemorySample {
    let pcm: Vec<f32> = (0..PCM)
        .map(|i| if i % 2 == 0 { 0.5 } else { -0.5 })
        .collect();
    InMemorySample {
        pcm: Arc::new(pcm),
        sample_hz: HZ,
        loop_points: None,
    }
}

/// 低频正弦样本（100 Hz）。
fn low_sine() -> InMemorySample {
    InMemorySample::sine(100.0, HZ, PCM)
}

fn rms(samples: &[f32]) -> f32 {
    let sum: f64 = samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|sample| sample.to_bits()).collect()
}

/// 池渲染一份 SFZ（广播样本），返回输出缓冲。
fn render_pool(text: &str, sample: &InMemorySample, note: u8) -> Vec<f32> {
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let prepared = PreparedSfz::prepare(&sfz, sample, HZ).expect("摊平必须成功");
    let mut pool = SfzVoicePool::new(HZ);
    let created = pool.trigger_note(&prepared, note, 100, OUT as u64, 1.0, None);
    assert_eq!(created, 1, "单区域夹具 ⇒ 恰好建 1 个声部");
    let mut out = vec![0.0_f32; OUT];
    pool.render_into(&prepared, 0, &mut out);
    out
}

/// 离线渲染一份 SFZ（样本表路径），返回输出缓冲。
fn render_offline(text: &str, sample: InMemorySample, note: u8) -> Vec<f32> {
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let mut table = BTreeMap::new();
    table.insert("a.wav".to_owned(), sample);
    let mut out = vec![0.0_f32; OUT];
    sfz.render_note_into_from_samples(&table, note, 100, Some(OUT as u64), None, &mut out, HZ)
        .expect("离线渲染必须成功");
    out
}

const REGION: &str = "<region> sample=a.wav pitch_keytrack=0";

/// ⭐ F1：没有 `cutoff` 时，其余滤波 opcode 写了也不激活任何处理。
#[test]
fn f1_no_cutoff_means_bit_exact_bypass() {
    let plain = render_pool(REGION, &nyquist(), 60);
    let dressed = render_pool(
        "<region> sample=a.wav pitch_keytrack=0 fil_type=hpf_2p resonance=12 fil_keytrack=100 fil_veltrack=1200",
        &nyquist(),
        60,
    );
    assert!(plain.iter().any(|s| *s != 0.0), "夹具必须真的出声");
    assert_eq!(
        bits(&plain),
        bits(&dressed),
        "⭐ 无 `cutoff` ⇒ 滤波禁用 ⇒ 与裸 region 逐位相同"
    );
}

/// ⭐ F2：`cutoff=0` 与无 `cutoff` 逐位相同（ARIA 口径）。
#[test]
fn f2_cutoff_zero_is_the_same_as_no_cutoff() {
    let plain = render_pool(REGION, &nyquist(), 60);
    let zero = render_pool(
        "<region> sample=a.wav pitch_keytrack=0 cutoff=0 fil_type=lpf_2p",
        &nyquist(),
        60,
    );
    assert_eq!(
        bits(&plain),
        bits(&zero),
        "⭐ `cutoff=0` 按 ARIA 口径 ＝ filter disabled ⇒ 逐位相同"
    );
}

/// ⭐ F3：低通真的压掉 Nyquist 内容；高通真的压掉低频正弦。
#[test]
fn f3_filters_actually_filter() {
    let raw = render_pool(REGION, &nyquist(), 60);
    let lowpassed = render_pool(
        "<region> sample=a.wav pitch_keytrack=0 cutoff=200 fil_type=lpf_2p",
        &nyquist(),
        60,
    );
    // 跳过起振的最初几帧（递推状态自零起，第一帧是台阶）。
    let ratio = rms(&lowpassed[64..]) / rms(&raw[64..]);
    assert!(
        ratio < 0.05,
        "⭐ lpf_2p@200Hz 应把 Nyquist 内容压到 5% 以下，实测比 {ratio}"
    );
    // 反向对照：低频正弦过同一只低通近乎无损、过高通被压掉。
    let sine_raw = render_pool(REGION, &low_sine(), 60);
    let sine_lp = render_pool(
        "<region> sample=a.wav pitch_keytrack=0 cutoff=2000 fil_type=lpf_2p",
        &low_sine(),
        60,
    );
    assert!(
        rms(&sine_lp[64..]) > 0.9 * rms(&sine_raw[64..]),
        "100 Hz 正弦过 lpf@2000 应近乎无损"
    );
    let sine_hp = render_pool(
        "<region> sample=a.wav pitch_keytrack=0 cutoff=2000 fil_type=hpf_2p",
        &low_sine(),
        60,
    );
    assert!(
        rms(&sine_hp[64..]) < 0.05 * rms(&sine_raw[64..]),
        "100 Hz 正弦过 hpf@2000 应被压到 5% 以下"
    );
}

/// ⭐ F4：带滤波时池渲染与离线渲染逐位相同（三条路同一份折算/系数）。
#[test]
fn f4_pool_and_offline_agree_bit_exactly_with_a_filter() {
    // note 66：fil_keytrack=50 × 6 键 = 300 音分 ⇒ 两条路都必须把截止
    // 折到 800 × 2^0.25（折算分叉会直接变成逐位分叉）。
    let text = "<region> sample=a.wav pitch_keytrack=0 cutoff=800 fil_type=lpf_2p resonance=6 fil_keytrack=50";
    let pooled = render_pool(text, &nyquist(), 66);
    let offline = render_offline(text, nyquist(), 66);
    assert!(pooled.iter().any(|s| *s != 0.0), "夹具必须真的出声");
    assert_eq!(
        bits(&pooled),
        bits(&offline),
        "⭐ 同一 region、同一 (note, velocity)：池与离线必须逐位相同"
    );
}

/// ⭐ F5：`fil_keytrack` 把高键的截止推高 ⇒ 高频内容回来。
#[test]
fn f5_fil_keytrack_opens_the_filter_for_high_notes() {
    // 夹具用 12 kHz 正弦（Nyquist 以下）：⛔ 不能用 Nyquist 交替 —— 任何低通在
    // Nyquist 处都有零点，「截止推满」也放不回来（那是零点，不是没推够）。
    // cutoff=200、fil_keytrack=1200/键、锚点 60：note 60 ⇒ 200 Hz（压死）；
    // note 72 ⇒ 200 × 2^12 Hz（钳到 0.495×sr ⇒ 12 kHz 近乎全通）。
    let hf = InMemorySample::sine(12_000.0, HZ, PCM);
    let text = "<region> sample=a.wav pitch_keytrack=0 cutoff=200 fil_keytrack=1200 fil_keycenter=60 fil_type=lpf_2p";
    let low_note = render_pool(text, &hf, 60);
    let high_note = render_pool(text, &hf, 72);
    let raw = render_pool(REGION, &hf, 72);
    assert!(
        rms(&low_note[64..]) < 0.05 * rms(&raw[64..]),
        "锚点键上截止 200 Hz ⇒ 12 kHz 内容应被压死"
    );
    assert!(
        rms(&high_note[64..]) > 0.8 * rms(&raw[64..]),
        "高 12 键 ⇒ 截止推高 12 个八度 ⇒ 内容应基本回来，实测 RMS {} vs {}",
        rms(&high_note[64..]),
        rms(&raw[64..])
    );
}

/// ⭐ F6：跨块延续——两块渲染与一块渲染逐位相同（递推状态写回正确）。
#[test]
fn f6_filter_state_survives_block_boundaries() {
    let text = "<region> sample=a.wav pitch_keytrack=0 cutoff=800 fil_type=lpf_2p resonance=6";
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let prepared = PreparedSfz::prepare(&sfz, &nyquist(), HZ).expect("摊平必须成功");
    let one_block = render_pool(text, &nyquist(), 60);
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
        "⭐ 同一输入分两块渲染必须与一块逐位相同（滤波状态跨块延续）"
    );
}

/// ⭐ F7：池满窃取 ⇒ 挂起者提升时现场配置滤波；整段渲染有限且出声。
#[test]
fn f7_steal_promotion_configures_the_filter() {
    // 17 条相同区域（全带低通）⇒ 第 17 条走软窃取、淡出后提升。
    // 夹具 PCM 用 440 Hz 正弦、cutoff=2000：滤波后仍响亮出声（⛔ 不能用
    // Nyquist 交替＋低截止 —— 那种组合滤完本就接近静音，测不出「提升后出声」）。
    let mut text = String::new();
    for _ in 0..17 {
        text.push_str(
            "<region> sample=a.wav key=60 pitch_keycenter=60 cutoff=2000 fil_type=lpf_2p\n",
        );
    }
    let sfz = SfzInstrument::load(text, &Default::default()).expect("SFZ 必须解析成功");
    let sample = InMemorySample::sine(440.0, HZ, PCM);
    let prepared = PreparedSfz::prepare(&sfz, &sample, HZ).expect("摊平必须成功");
    let mut pool = SfzVoicePool::new(HZ);
    let created = pool.trigger_note(&prepared, 60, 100, OUT as u64, 1.0, None);
    assert_eq!(created, 16, "池容量 16 ⇒ 17 条里 16 条直接落槽、1 条窃取");
    assert_eq!(pool.stolen(), 1, "第 17 条必须真的走了窃取路");
    // 分块渲染到淡出（3 ms = 144 帧）走完、提升发生之后。
    let mut out = vec![0.0_f32; OUT];
    let block = 512;
    let mut position = 0u64;
    while (position as usize) < OUT {
        let end = ((position as usize) + block).min(OUT);
        pool.render_into(&prepared, position, &mut out[position as usize..end]);
        position = end as u64;
    }
    assert!(
        out.iter().all(|s| s.is_finite()),
        "⭐ 窃取/提升全程输出必须有限（无 NaN/Inf）"
    );
    assert!(
        rms(&out[OUT / 2..]) > 0.05,
        "⭐ 提升后的声部必须真的出声（滤波已配置、不是静音槽），实测 RMS {}",
        rms(&out[OUT / 2..])
    );
}

/// ⭐ F8：摊平表原样带出滤波六标量。
#[test]
fn f8_prepared_region_carries_the_filter_scalars() {
    let sfz = SfzInstrument::load(
        "<region> sample=a.wav cutoff=1000 resonance=6 fil_type=hpf_4p \
         fil_keytrack=100 fil_keycenter=52 fil_veltrack=-1200"
            .to_owned(),
        &Default::default(),
    )
    .expect("SFZ 必须解析成功");
    let prepared = PreparedSfz::prepare(&sfz, &nyquist(), HZ).expect("摊平必须成功");
    let region = &prepared.regions()[0];
    assert_eq!(region.fil_type(), FilterType::Highpass4p);
    assert_eq!(region.cutoff(), Some(1000.0));
    assert_eq!(region.resonance(), 6.0);
    assert_eq!(region.fil_keytrack(), 100);
    assert_eq!(region.fil_keycenter(), 52);
    assert_eq!(region.fil_veltrack(), -1200);
}
