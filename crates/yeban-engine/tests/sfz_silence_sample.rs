//! ⭐ 任务单 `#217`：SFZ 内建静音伪样本 `sample=*silence`（引擎侧判据）。
//!
//! ## 量什么（对象 ＋ 单位，先说再跑）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `regions` | [`PreparedSfz::region_count`] | 区域数 |
//! | `samples` | [`PreparedSfz::sample_count`] | 去重后的样本份数 |
//! | `silent` | [`PreparedSfz::silent_region_count`] | **不产生采样输出**的区域数（缺样本 ＋ 内建静音） |
//! | `missing` | [`PreparedSfz::missing_sample_paths`] | **缺文件**路径条数（⛔ 不含 `*silence`） |
//! | `frames` | 渲染写入帧数 | 帧（0 ＝ 一帧都不写） |
//! | `energy` | 输出 `Σx²` | 无量纲（0.0 ＝ 逐位静音） |
//!
//! ## 它**不**证什么
//!
//! ⛔ 不证"人耳听到静音"（本文件只读引擎输出数组；静音在这里**恰好**可用能量判，
//! 因为判据要求的是 **0** —— 见交付报告 §8）。
//! ⛔ 不证其它 ARIA 内建波形（`*sine` / `*saw` / …）：它们**未实现**，本文件不碰。
//!
//! ## 反向对照（判据有牙）
//!
//! 每个"静音"断言都配一条**同夹具、同键、只把 `sample=*silence` 换成真样本**的腿
//! ⇒ 它**必须出声**（否则"0 帧"可能只是因为整条夹具根本不匹配音符）。

use std::sync::Arc;

use yeban_engine::sfz_instrument::{InMemorySample, PreparedRegion, PreparedSfz, SfzInstrument};
use yeban_sfz::parser::ParseLimits;

const RATE: f32 = 48_000.0;

/// 480 帧的常量样本（`0.25`）—— "真样本"那条腿用。
fn sample() -> InMemorySample {
    InMemorySample {
        pcm: Arc::new(vec![0.25; 480]),
        sample_hz: RATE,
    }
}

/// 能量 `Σx²`（单位：无量纲）。
fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|value| value * value).sum()
}

/// 一份**两区域**的乐器：`key=60` 是内建静音，`key=72` 是真样本。
const SILENCE_AND_TONE: &str = "<region> key=60 sample=*silence
<region> key=72 sample=tone.wav";

/// ⭐ 摊平期：`*silence` ⇒ `NO_SAMPLE` ＋ `silent`，⛔ 不进 `missing`；
/// 同一个 plan 里 key=72 的普通区域**照常**指向自己的样本。
#[test]
fn prepare_marks_builtin_silence_as_silent_and_never_as_missing() {
    let instrument = SfzInstrument::load(SILENCE_AND_TONE.to_owned(), &ParseLimits::default())
        .expect("夹具必须可解析");
    let plan =
        PreparedSfz::prepare_with_samples(&instrument, &[("tone.wav".to_owned(), sample())], RATE)
            .expect("摊平");

    let builtin = &plan.regions()[0];
    let tone = &plan.regions()[1];
    println!(
        "[engine/silence] regions={} samples={} silent={} missing={:?} \
         silence_index={} tone_index={} tone_has_sample={}",
        plan.region_count(),
        plan.sample_count(),
        plan.silent_region_count(),
        plan.missing_sample_paths(),
        builtin.sample_index(),
        tone.sample_index(),
        tone.has_sample(),
    );

    // 读数（每个数字都有单位）：
    assert_eq!(plan.region_count(), 2, "区域数");
    assert_eq!(plan.sample_count(), 1, "只 materialize 真样本那一份");
    assert_eq!(plan.silent_region_count(), 1, "不产生输出的区域数");
    assert!(
        plan.missing_sample_paths().is_empty(),
        "`*silence` ⛔ 不是缺文件，missing 必须空；实得 {:?}",
        plan.missing_sample_paths()
    );
    assert_eq!(
        builtin.sample_index(),
        PreparedRegion::NO_SAMPLE,
        "内建静音的区域必须 NO_SAMPLE"
    );
    assert!(
        tone.has_sample(),
        "非 `*silence` 的区域必须**不受影响**（仍指向 tone.wav）"
    );
    assert_eq!(
        plan.sample_content_label(),
        "distinct(1)+silent(1)",
        "读数形状（缺样本口径未变，只是内建静音也计入 silent）"
    );
}

/// ⭐ 播放期（摊平表）：内建静音的区域写 **0 帧 / 能量 0**；同夹具的普通区域**出声**。
#[test]
fn the_flattened_plan_writes_nothing_for_builtin_silence_and_still_plays_other_regions() {
    let instrument = SfzInstrument::load(SILENCE_AND_TONE.to_owned(), &ParseLimits::default())
        .expect("夹具必须可解析");
    let plan =
        PreparedSfz::prepare_with_samples(&instrument, &[("tone.wav".to_owned(), sample())], RATE)
            .expect("摊平");

    let mut silence_out = vec![0.0_f32; 128];
    let silence_frames = plan.render_block_into(60, 100, &mut silence_out);
    let mut tone_out = vec![0.0_f32; 128];
    let tone_frames = plan.render_block_into(72, 100, &mut tone_out);
    let silence_energy = energy(&silence_out);
    let tone_energy = energy(&tone_out);
    println!(
        "[engine/silence/flat] silence_frames={silence_frames} silence_energy={silence_energy:.9} \
         tone_frames={tone_frames} tone_energy={tone_energy:.9}"
    );

    assert_eq!(silence_frames, 0, "内建静音：写入帧数必须 0");
    assert_eq!(silence_energy, 0.0, "内建静音：输出能量必须 0");
    assert!(tone_frames > 0, "反向对照：普通区域必须出声");
    assert!(tone_energy > 0.0, "反向对照：普通区域的能量必须 > 0");
}

/// ⭐ 播放期（离线 `render_note_into`）：即使调用方给的是**非空正弦**，
/// `*silence` 的 region 也**一帧不写** —— 这条正是 `probe` 能量能判"静音"的依据。
#[test]
fn offline_render_skips_builtin_silence_even_when_the_source_is_a_nonempty_sine() {
    let instrument = SfzInstrument::load(
        "<region> key=60 sample=*silence".to_owned(),
        &ParseLimits::default(),
    )
    .expect("夹具必须可解析");
    let sine = InMemorySample::sine(440.0, RATE, 4_800);
    let mut out = vec![0.0_f32; 4_800];
    let written = instrument
        .render_note_into(&sine, 60, 100, &mut out, RATE)
        .expect("离线渲染");
    let silence_energy = energy(&out);
    println!(
        "[engine/silence/offline] frames={written} energy={silence_energy:.9} \
         source_pcm_len={}",
        sine.pcm.len()
    );
    assert_eq!(written, 0, "`*silence` ⇒ 一帧都不写");
    assert_eq!(silence_energy, 0.0, "`*silence` ⇒ 输出能量 0");

    // 反向对照：把同一份音符合成到一条**普通**区域上，同样的 sine 必须出声。
    let audible = SfzInstrument::load(
        "<region> key=60 sample=tone.wav".to_owned(),
        &ParseLimits::default(),
    )
    .expect("解析");
    let mut tone_out = vec![0.0_f32; 4_800];
    let tone_frames = audible
        .render_note_into(&sine, 60, 100, &mut tone_out, RATE)
        .expect("离线渲染");
    println!(
        "[engine/silence/offline-control] frames={tone_frames} energy={:.9}",
        energy(&tone_out)
    );
    assert!(tone_frames > 0, "反向对照：普通区域必须出声");
    assert!(energy(&tone_out) > 0.0, "反向对照：普通区域能量必须 > 0");
}

/// ⭐ 广播重载（[`PreparedSfz::prepare`]）也⛔ 不把内建静音指向 `samples[0]`。
#[test]
fn the_broadcast_overload_also_keeps_builtin_silence_silent() {
    let instrument = SfzInstrument::load(SILENCE_AND_TONE.to_owned(), &ParseLimits::default())
        .expect("夹具必须可解析");
    let source = sample();
    let plan = PreparedSfz::prepare(&instrument, &source, RATE).expect("广播摊平");
    println!(
        "[engine/silence/broadcast] broadcast={} silent={} missing={:?} \
         silence_index={} tone_index={}",
        plan.is_broadcast(),
        plan.silent_region_count(),
        plan.missing_sample_paths(),
        plan.regions()[0].sample_index(),
        plan.regions()[1].sample_index(),
    );
    assert!(plan.is_broadcast(), "夹具走的是广播重载");
    assert_eq!(plan.regions()[0].sample_index(), PreparedRegion::NO_SAMPLE);
    assert_eq!(plan.silent_region_count(), 1);
    assert!(plan.missing_sample_paths().is_empty());
    let mut out = vec![0.0_f32; 128];
    assert_eq!(plan.render_block_into(60, 100, &mut out), 0);
    assert_eq!(energy(&out), 0.0);
}
