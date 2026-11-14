//! ⭐ **SFZ 包络族**（`ampeg_delay` / `ampeg_start` 补全 ＋ `fileg_*` ／
//! `pitcheg_*` 全族）的引擎判据。
//!
//! 解析与取值域在 `yeban-sfz`（`eg` 模块与 `instrument` 的模块测试覆盖）；
//! 本文件只判**引擎消费**：
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | E1 | `ampeg_delay`：延迟期内**恰好静音**，延迟后与无延迟渲染逐位对齐（恒值样本） | 逐位 |
//! | E2 | `ampeg_start`：起音自 start 电平起步；`start=100` ⇒ 与 `attack=0` 逐位相同（规范注记的等价式） | 逐位 ＋ 比例 |
//! | E3 | `fileg` 真的扫频：attack 内截止自静态值升向 ×2^(depth/1200)，8 kHz 正弦由灭到通 | RMS 比 |
//! | E4 | 无 `cutoff` ⇒ `fileg_*` 写了也透明（与裸 region 逐位相同） | 逐位 |
//! | E5 | `pitcheg` 真的弯音：attack 内音高升向 ×2^(depth/1200)；恒定满电平 ⇒ 与 2 倍速率参照逐位相同 | 过零数 ＋ 逐位 |
//! | E6 | `vel2` 折算：`*_vel2*` 按力度线性折算（delay 随力度变长、depth 折算值命中公式） | 帧下标 ＋ 读数 |
//! | E7 | 三路一致：池渲染 == 离线渲染逐位（三族同时在场） | 逐位 |
//! | E8 | 池跨块延续 ＋ 窃取/提升路径不 panic 且出声（EG 状态写回正确） | 逐位 ＋ 有限/非静音 |
//! | E9 | 解析错误响亮：depth 小数/越界、负时间、sustain 越界 ⇒ `Err` | `is_err` |
//! | E10 | 摊平表带出：`fileg_def` / `pitcheg_def` 与 `ampeg_base(Delay/Start)` 读数命中 | 访问器 |
//!
//! 范围裁决（见 `yeban_sfz::eg`）：本族只做**静态参 ＋ `vel2`**；`*_ccN` /
//! `*_onccN` / `*_curveccN` ⛔ 不进 `ModTarget`。`fileg` / `pitcheg` 的释放段
//! 回到 **start 电平**（静止位偏移），与 `ampeg` 释放归零的既有口径不同。

use std::collections::BTreeMap;
use std::sync::Arc;

use yeban_engine::sfz_instrument::{InMemorySample, PreparedSfz, SfzInstrument, SfzVoicePool};
use yeban_sfz::eg::{EgParam, EgParams};
use yeban_sfz::modulation::AmpegParam;

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

/// 恒值样本：输出 = 恒值 × 包络 × 增益 ⇒ 包络形状可逐位对读。
fn constant() -> InMemorySample {
    InMemorySample {
        pcm: Arc::new(vec![0.5_f32; PCM]),
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

/// 池渲染一份 SFZ（广播样本、力度 100、整段按住），返回输出缓冲。
fn render_pool(text: &str, sample: &InMemorySample) -> Vec<f32> {
    render_pool_vel(text, sample, 100)
}

/// 池渲染（指定力度）。
fn render_pool_vel(text: &str, sample: &InMemorySample, velocity: u8) -> Vec<f32> {
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let prepared = PreparedSfz::prepare(&sfz, sample, HZ).expect("摊平必须成功");
    let mut pool = SfzVoicePool::new(HZ);
    let created = pool.trigger_note(&prepared, 60, velocity, OUT as u64, 1.0, None);
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

/// ⭐ E1：`ampeg_delay=0.05`（2400 帧）⇒ 前 2400 帧恰好为零；延迟结束后
/// 与无延迟渲染的**稳态段**逐位相同（恒值样本 ⇒ 只剩包络可比；⛔ 不拿
/// 无延迟渲染的**开头**比 —— 声部淡入在那一段里，延迟声部的淡入则被
/// 延迟段吃掉，两边的开头本就不同形）。
#[test]
fn e1_ampeg_delay_silences_then_matches_bit_exact() {
    let plain = render_pool("<region> sample=a.wav pitch_keytrack=0", &constant());
    let delayed = render_pool(
        "<region> sample=a.wav pitch_keytrack=0 ampeg_delay=0.05",
        &constant(),
    );
    assert!(
        delayed[..2400].iter().all(|s| *s == 0.0),
        "⭐ 延迟期内输出必须恰好为零（电平 = start = 0）"
    );
    assert_eq!(
        bits(&delayed[2400..]),
        bits(&plain[2400..]),
        "⭐ 延迟结束后两边都进稳态（电平 1、无淡入）⇒ 逐位相同"
    );
}

/// ⭐ E2：`ampeg_start` 的两条腿。
#[test]
fn e2_ampeg_start_level_and_full_start_equivalence() {
    // (a) start=100 ＋ attack=0.1 ⇒ 与 attack=0 逐位相同（规范页注记：
    //     「start=100 等价于 attack=0 且 attack 并入 hold」—— 本仓的梯形里
    //     attack 段跨度 = 1 − start = 0 ⇒ 电平恒 1，两者同形）。
    let full = render_pool(
        "<region> sample=a.wav pitch_keytrack=0 ampeg_start=100 ampeg_attack=0.1",
        &constant(),
    );
    let instant = render_pool("<region> sample=a.wav pitch_keytrack=0", &constant());
    assert_eq!(
        bits(&full),
        bits(&instant),
        "⭐ start=100 ⇒ 起音段跨度为零 ⇒ 与无 attack 逐位相同"
    );
    // (b) start=50、attack 覆盖整段（4800 帧）：中点电平 = 0.5 + 0.5×0.5 = 0.75，
    //     对照 start=0 的中点 0.5 ⇒ 同帧比值 1.5（容差 1e-5，容忍帧量化）。
    let half = render_pool(
        "<region> sample=a.wav pitch_keytrack=0 ampeg_start=50 ampeg_attack=0.1",
        &constant(),
    );
    let zero = render_pool(
        "<region> sample=a.wav pitch_keytrack=0 ampeg_attack=0.1",
        &constant(),
    );
    let ratio = half[2400] / zero[2400];
    assert!(
        (ratio - 1.5).abs() < 1e-4,
        "⭐ attack 中点电平比应为 0.75 / 0.5 = 1.5，实测 {ratio}"
    );
    assert!(half[0] != 0.0, "⭐ start=50 ⇒ 第 0 帧电平 = 0.5 ≠ 0");
}

/// ⭐ E3：`fileg` 扫频 —— 8 kHz 正弦在 attack 前被 200 Hz 低通压灭、
/// attack 后截止扫到钳制上限（200 × 2^10 ⇒ 23760 Hz）而放行。
/// ⚠ 夹具⛔ 不能用 Nyquist 交替：RBJ 低通在 Nyquist 处恒有双零点，
/// 任何截止下稳态增益都是 0 —— 那测的是零点、不是扫频（第一轮实测
/// 踩过：扫到高段输出仍为 0，探针逐块 RMS 证实中段只有系数时变的泄漏）。
#[test]
fn e3_fileg_sweeps_the_cutoff() {
    let sine8k = InMemorySample::sine(8_000.0, HZ, PCM);
    let swept = render_pool(
        "<region> sample=a.wav pitch_keytrack=0 cutoff=200 fil_type=lpf_2p \
         fileg_depth=12000 fileg_attack=0.05 fileg_sustain=100",
        &sine8k,
    );
    let early = rms(&swept[..480]);
    let late = rms(&swept[4320..]);
    let plain = render_pool("<region> sample=a.wav pitch_keytrack=0", &sine8k);
    let plain_rms = rms(&plain[4320..]);
    assert!(
        late > 10.0 * early.max(1e-6),
        "⭐ fileg attack 后 8 kHz 必须回来：early {early} → late {late}"
    );
    assert!(
        late > 0.8 * plain_rms,
        "⭐ 扫到钳制上限后应近乎直通：late {late} vs 无滤波 {plain_rms}"
    );
    // 对照：无 fileg 的静态低通全程压灭（early/late 同低）。
    let statik = render_pool(
        "<region> sample=a.wav pitch_keytrack=0 cutoff=200 fil_type=lpf_2p",
        &sine8k,
    );
    assert!(
        rms(&statik[4320..]) < 0.2 * plain_rms,
        "对照组：静态低通必须全程压灭 8 kHz"
    );
}

/// ⭐ E4：没有 `cutoff` ⇒ `fileg_*` 写了也不激活（透明口径同滤波静态族）。
#[test]
fn e4_fileg_without_cutoff_is_transparent() {
    let plain = render_pool("<region> sample=a.wav pitch_keytrack=0", &nyquist());
    let dressed = render_pool(
        "<region> sample=a.wav pitch_keytrack=0 fileg_depth=12000 fileg_attack=0.05 fileg_sustain=100",
        &nyquist(),
    );
    assert_eq!(
        bits(&plain),
        bits(&dressed),
        "⭐ 无 cutoff ⇒ fileg 透明 ⇒ 逐位相同"
    );
}

/// ⭐ E5：`pitcheg` 弯音 —— 过零数随后半段变密；恒定满电平与 2 倍速率参照逐位同。
/// ⚠ 样本必须比 `输出 × 最大速率` 更长：×2 速率下 4800 帧输出要吃掉
/// 9600 帧样本，样本只有 4800 帧时后半段是**播完后的静音**、不是弯音
/// （第一轮实测踩过：探针显示声部在约 3000 帧处把样本播穿）。
#[test]
fn e5_pitcheg_bends_pitch() {
    let sine = InMemorySample::sine(440.0, HZ, 19_200);
    let bent = render_pool(
        "<region> sample=a.wav pitch_keytrack=0 pitcheg_depth=1200 pitcheg_attack=0.05 pitcheg_sustain=100",
        &sine,
    );
    let crossings = |span: &[f32]| {
        span.windows(2)
            .filter(|pair| pair[0].signum() != pair[1].signum() && pair[1] != 0.0)
            .count()
    };
    let first = crossings(&bent[..2400]);
    let second = crossings(&bent[2400..]);
    assert!(
        (second as f32) > (first as f32) * 1.2,
        "⭐ pitcheg attack 内音高上行 ⇒ 后半过零数必须显著变多：{first} → {second}"
    );
    // 恒定满电平（attack=0、sustain=100）⇒ 速率恒 ×2 ⇒ 与 pitch_keycenter
    // 低 12 键（速率 ×2）的参照渲染逐位相同。
    let const_bent = render_pool(
        "<region> sample=a.wav pitch_keycenter=60 pitcheg_depth=1200 pitcheg_sustain=100",
        &sine,
    );
    let reference = render_pool("<region> sample=a.wav pitch_keycenter=48", &sine);
    assert_eq!(
        bits(&const_bent),
        bits(&reference),
        "⭐ pitcheg 满电平恒定 ⇒ 位置步进恒 ×2 ⇒ 与速率 ×2 参照逐位相同"
    );
}

/// ⭐ E6：`vel2` 折算 —— 解析读数命中公式 ＋ 力度改变 delay 的渲染长度。
#[test]
fn e6_vel2_folds_by_velocity() {
    // (a) 折算读数：depth 基准 6000、vel2depth −6000。
    let sfz = SfzInstrument::load(
        "<region> sample=a.wav fileg_depth=6000 fileg_vel2depth=-6000 \
         pitcheg_depth=-1200 pitcheg_vel2attack=0.5"
            .to_owned(),
        &Default::default(),
    )
    .expect("SFZ 必须解析成功");
    sfz.with_instrument(|parsed| {
        let region = &parsed.regions()[0];
        assert_eq!(region.fileg.value(EgParam::Depth, 127), 0.0);
        assert_eq!(region.fileg.value(EgParam::Depth, 0), 6000.0);
        assert_eq!(region.pitcheg.params.base(EgParam::Attack), 0.0);
        assert_eq!(region.pitcheg.vel2[EgParam::Attack.index()], Some(0.5));
        let params: EgParams = region.fileg.params;
        assert_eq!(params.base(EgParam::Depth), 6000.0);
    })
    .expect("重解析");
    // (b) 渲染：ampeg_delay=0 + ampeg_vel2delay=0.05 ⇒ 力度 127 时延迟
    //     ≈ 2400 帧（首帧非零下标 ≥ 2000），力度 1 时 ≈ 19 帧（< 200）。
    let text = "<region> sample=a.wav pitch_keytrack=0 ampeg_vel2delay=0.05";
    let loud = render_pool_vel(text, &constant(), 127);
    let soft = render_pool_vel(text, &constant(), 1);
    let first_loud = loud.iter().position(|s| *s != 0.0).expect("必须出声");
    let first_soft = soft.iter().position(|s| *s != 0.0).expect("必须出声");
    assert!(
        first_loud >= 2000,
        "⭐ 力度 127 ⇒ delay ≈ 2400 帧，实测首个非零帧 {first_loud}"
    );
    assert!(
        first_soft < 200,
        "⭐ 力度 1 ⇒ delay ≈ 19 帧，实测首个非零帧 {first_soft}"
    );
}

/// ⭐ E7：三族同时在场时池渲染 == 离线渲染逐位（同一 `mix_span_into`）。
#[test]
fn e7_pool_matches_offline_bit_exact_with_all_families() {
    let text = "<region> sample=a.wav pitch_keytrack=0 cutoff=800 fil_type=lpf_2p \
                ampeg_delay=0.01 ampeg_start=20 ampeg_attack=0.02 \
                fileg_depth=2400 fileg_attack=0.05 fileg_sustain=80 \
                pitcheg_depth=-600 pitcheg_attack=0.03 pitcheg_sustain=100";
    let pooled = render_pool(text, &nyquist());
    let offline = render_offline(text, nyquist());
    assert!(pooled.iter().any(|s| *s != 0.0), "夹具必须真的出声");
    assert_eq!(
        bits(&pooled),
        bits(&offline),
        "⭐ 池与离线走同一逐样本路 ⇒ 逐位相同"
    );
}

/// ⭐ E8：跨块延续逐位 ＋ 窃取/提升不 panic 且出声。
#[test]
fn e8_block_boundaries_and_steal_promotion() {
    let text = "<region> sample=a.wav pitch_keytrack=0 cutoff=800 fil_type=lpf_2p \
                fileg_depth=2400 fileg_attack=0.08 fileg_sustain=90 \
                pitcheg_depth=300 pitcheg_attack=0.08 pitcheg_sustain=100";
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let prepared = PreparedSfz::prepare(&sfz, &nyquist(), HZ).expect("摊平必须成功");
    let one_block = render_pool(text, &nyquist());
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
        "⭐ EG 状态跨块写回 ⇒ 两块 == 一块逐位"
    );

    // 窃取/提升：17 条相同区域（全带 fileg）⇒ 第 17 条软窃取后提升。
    let mut many = String::new();
    for _ in 0..17 {
        many.push_str(
            "<region> sample=a.wav key=60 pitch_keycenter=60 cutoff=2000 fil_type=lpf_2p \
             fileg_depth=1200 fileg_attack=0.05 fileg_sustain=100\n",
        );
    }
    let sfz = SfzInstrument::load(many, &Default::default()).expect("SFZ 必须解析成功");
    let sample = InMemorySample::sine(440.0, HZ, PCM);
    let prepared = PreparedSfz::prepare(&sfz, &sample, HZ).expect("摊平必须成功");
    let mut pool = SfzVoicePool::new(HZ);
    let created = pool.trigger_note(&prepared, 60, 100, OUT as u64, 1.0, None);
    assert_eq!(created, 16, "池容量 16 ⇒ 16 条直接落槽、1 条窃取");
    assert_eq!(pool.stolen(), 1, "第 17 条必须真的走了窃取路");
    let mut out = vec![0.0_f32; OUT];
    let block = 512;
    let mut position = 0u64;
    while (position as usize) < OUT {
        let end = ((position as usize) + block).min(OUT);
        pool.render_into(&prepared, position, &mut out[position as usize..end]);
        position = end as u64;
    }
    assert!(out.iter().all(|s| s.is_finite()), "⭐ 全程输出必须有限");
    assert!(
        rms(&out[OUT / 2..]) > 0.05,
        "⭐ 提升后的声部必须真的出声，实测 RMS {}",
        rms(&out[OUT / 2..])
    );
}

/// ⭐ E9：解析错误必须响亮（⛔ 不静默钳位、不落缺省）。
#[test]
fn e9_parse_errors_are_loud() {
    for bad in [
        "fileg_depth=1.5",       // depth 是整数（音分）
        "fileg_depth=12001",     // depth 越界
        "pitcheg_depth=-12001",  // depth 越界（负）
        "fileg_attack=-1",       // 负时间
        "pitcheg_sustain=101",   // sustain 越界
        "fileg_vel2depth=12001", // vel2 depth 越界
        "ampeg_delay=101",       // 时间越界
        "ampeg_start=101",       // 百分比越界
        "ampeg_vel2delay=-101",  // vel2 时间越界
        "ampeg_vel2start=101",   // vel2 百分比越界
    ] {
        let text = format!("<region> sample=a.wav {bad}");
        assert!(
            SfzInstrument::load(text, &Default::default()).is_err(),
            "⭐ {bad} 必须响亮 Err"
        );
    }
}

/// ⭐ E10：摊平表带出两族定义与 ampeg 两新参的基准读数。
#[test]
fn e10_prepared_region_carries_eg_definitions() {
    let sfz = SfzInstrument::load(
        "<region> sample=a.wav ampeg_delay=0.02 ampeg_start=30 \
         fileg_depth=2400 fileg_attack=0.05 pitcheg_depth=-600"
            .to_owned(),
        &Default::default(),
    )
    .expect("SFZ 必须解析成功");
    let prepared = PreparedSfz::prepare(&sfz, &nyquist(), HZ).expect("摊平必须成功");
    let region = &prepared.regions()[0];
    assert_eq!(region.ampeg_base(AmpegParam::Delay), 0.02);
    assert_eq!(region.ampeg_base(AmpegParam::Start), 30.0);
    let fileg = region.fileg_def(&prepared).expect("fileg 必须带出");
    assert_eq!(fileg.params.base(EgParam::Depth), 2400.0);
    assert_eq!(fileg.params.base(EgParam::Attack), 0.05);
    let pitcheg = region.pitcheg_def(&prepared).expect("pitcheg 必须带出");
    assert_eq!(pitcheg.params.base(EgParam::Depth), -600.0);
    // 无 EG 的 region ⇒ None（透明不占侧表）。
    let plain = SfzInstrument::load("<region> sample=a.wav".to_owned(), &Default::default())
        .expect("SFZ 必须解析成功");
    let prepared = PreparedSfz::prepare(&plain, &nyquist(), HZ).expect("摊平必须成功");
    assert!(prepared.regions()[0].fileg_def(&prepared).is_none());
    assert!(prepared.regions()[0].pitcheg_def(&prepared).is_none());
}
