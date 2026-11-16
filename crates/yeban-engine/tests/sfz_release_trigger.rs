//! ⭐ **释放采样真的发声**（`trigger=release` / `release_key`）＋ `rt_decay`（本片）。
//!
//! # 这条判据在修什么
//!
//! 登记语料里 `trigger=release` 出现在 **29** 个文件（117 处）、`release_key` **10** 个
//! 文件（20 处），`rt_decay` **5** 个文件（22 处）。此前引擎把释放族的**键掩码清零**
//! （`#94` 的防止"按下时打出来"）之后**没有**任何 note-off 触发路径 ⇒ 松键噪声、
//! 钢琴的 key-up、吉他的松弦声**永远不发声**，而且没有任何告警。
//!
//! # 口径（规范逐字与工程裁决见 `docs/ledger/sfz-release-samples-notes.md`）
//!
//! * `trigger=release` 由 note-off 触发；`release_key` 同族但**忽略延音踏板**；
//! * 规范：释放族"will play as if `loop_mode` was set to `one_shot`" ⇒ 播到样本尾；
//! * `rt_decay`（dB/秒）："The volume decrease (in decibels) per seconds after the
//!   corresponding attack region was triggered" ⇒ 乘子 `10 ^ (−(rt_decay × 按住秒数) / 20)`；
//! * **力度来源**：取该键最近一次 note-on 的力度（工程裁决；MIDI note-off 力度在多数
//!   DAW 里是 0，用它会让释放采样全静音）。
//!
//! # 判据表
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | T1 | 两张掩码各归各的 | 释放区域 `key_mask == [0,0]`、`release_key_mask != [0,0]`；起音相反 |
//! | T2 | note-on 不建释放声部、note-off 建 | `trigger_note` 返回 0、`trigger_release` 返回 1，且首帧是释放样本的值 |
//! | T3 | `rt_decay` 按按住秒数衰减 | 1 秒 / 6 dB / 秒 ⇒ 首帧比值 ≈ 0.501187 |
//! | T4 | 离线从松键帧起写、与实时同值 | `out[..held]` 全 0、`out[held]` == 实时首帧 |
//! | T5 | 没有释放区域 ⇒ 快路径 | `has_release_regions() == false`、`trigger_release == 0` |

use std::collections::BTreeMap;
use std::sync::Arc;

use yeban_engine::sfz_instrument::{InMemorySample, PreparedSfz, SfzInstrument, SfzVoicePool};

/// 引擎采样率。
const HZ: f32 = 48_000.0;
/// 样本帧数。
const FRAMES: usize = 4_800;
/// 触发音符。
const NOTE: u8 = 60;

/// 常值 PCM（首帧可反解是哪一个样本）。
fn flat(value: f32) -> InMemorySample {
    InMemorySample {
        pcm: Arc::new(vec![value; FRAMES]),
        sample_hz: HZ,
        loop_points: None,
    }
}

/// 摊平一份两个样本（`a.wav` / `rel.wav`）的 SFZ。
fn prepare(text: &str) -> PreparedSfz {
    let instrument =
        SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    PreparedSfz::prepare_with_samples(
        &instrument,
        &[
            ("a.wav".to_owned(), flat(0.5)),
            ("rel.wav".to_owned(), flat(0.25)),
        ],
        HZ,
    )
    .expect("摊平必须成功")
}

#[test]
fn t1_the_two_masks_belong_to_their_own_event_family() {
    let plan =
        prepare("<region> sample=a.wav key=60\n<region> sample=rel.wav key=60 trigger=release");
    let attack = &plan.regions()[0];
    let release = &plan.regions()[1];
    assert_ne!(attack.key_mask(), [0u64; 2], "起音区域进 note-on 掩码");
    assert_eq!(
        attack.release_key_mask(),
        [0u64; 2],
        "起音区域⛔ 不进释放掩码"
    );
    assert_eq!(
        release.key_mask(),
        [0u64; 2],
        "释放区域⛔ 不进 note-on 掩码"
    );
    assert_ne!(release.release_key_mask(), [0u64; 2], "释放区域进释放掩码");
    assert!(plan.has_release_regions());
}

#[test]
fn t2_note_on_does_not_build_a_release_voice_and_note_off_does() {
    let plan = prepare("<region> sample=rel.wav key=60 trigger=release");
    let mut pool = SfzVoicePool::new(HZ);
    // 按下：只有释放区域 ⇒ 一个声部都不建（掩码清零），但记账（键/力度/起点）照做。
    let created = pool.trigger_note(&plan, NOTE, 100, u64::MAX, 1.0, None);
    assert_eq!(created, 0, "note-on ⛔ 不能把释放采样打出来");
    let mut out = vec![0.0f32; 512];
    pool.render_into(&plan, 0, &mut out);
    assert!(out.iter().all(|sample| *sample == 0.0), "按下时必须是静音");
    // 松键：建 1 个释放声部，首帧是 `rel.wav` 的值 × 力度链。
    let created = pool.trigger_release(&plan, NOTE, 480, 1.0, None);
    assert_eq!(created, 1, "note-off 必须建释放声部");
    let mut out = vec![0.0f32; 512];
    pool.render_into(&plan, 0, &mut out);
    // 力度 100 的线性增益（`(100/127)^2`）。
    let gain = (100.0f32 / 127.0).powi(2);
    assert!(
        (out[0] - 0.25 * gain).abs() <= 1.0e-6,
        "释放首帧 {} != 0.25 × {gain}",
        out[0]
    );
}

#[test]
fn t3_rt_decay_scales_the_release_voice_by_the_held_time() {
    let plan = prepare("<region> sample=rel.wav key=60 trigger=release rt_decay=6");
    let strike = |held_frames: u64| {
        let mut pool = SfzVoicePool::new(HZ);
        assert_eq!(pool.trigger_note(&plan, NOTE, 127, u64::MAX, 1.0, None), 0);
        assert_eq!(pool.trigger_release(&plan, NOTE, held_frames, 1.0, None), 1);
        let mut out = vec![0.0f32; 16];
        pool.render_into(&plan, 0, &mut out);
        out[0]
    };
    let instant = strike(0);
    let after_one_second = strike(HZ as u64);
    let ratio = after_one_second / instant;
    let expected = 10.0f32.powf(-6.0 / 20.0);
    assert!(
        (ratio - expected).abs() <= 1.0e-5,
        "1 秒 / 6 dB 每秒 ⇒ {expected}，实测 {ratio}"
    );
    // 没有 `rt_decay` 的同一份 SFZ ⇒ 比值恰 1.0。
    let plain = prepare("<region> sample=rel.wav key=60 trigger=release");
    let strike_plain = |held_frames: u64| {
        let mut pool = SfzVoicePool::new(HZ);
        let _ = pool.trigger_note(&plain, NOTE, 127, u64::MAX, 1.0, None);
        assert_eq!(
            pool.trigger_release(&plain, NOTE, held_frames, 1.0, None),
            1
        );
        let mut out = vec![0.0f32; 16];
        pool.render_into(&plain, 0, &mut out);
        out[0]
    };
    assert_eq!(
        strike_plain(HZ as u64),
        strike_plain(0),
        "缺省 rt_decay ⇒ 不衰减"
    );
}

#[test]
fn t4_offline_writes_the_release_from_the_key_up_frame() {
    let text = "<region> sample=rel.wav key=60 trigger=release";
    let instrument = SfzInstrument::load(text.to_owned(), &Default::default()).expect("解析");
    let mut samples = BTreeMap::new();
    samples.insert("rel.wav".to_owned(), flat(0.25));
    let held = 64u64;
    let mut out = vec![0.0f32; 128];
    // note-on 那一段：释放区域被事件门控排除 ⇒ 一帧都不写（缓冲保持 0）。
    let note_on = instrument
        .render_note_into_from_samples(&samples, NOTE, 100, None, None, &mut out, HZ)
        .expect("离线");
    assert_eq!(note_on, 0, "note-on ⛔ 不写释放区域");
    assert!(out.iter().all(|sample| *sample == 0.0));
    // 释放段：从第 64 帧起写。
    let released = instrument
        .render_release_into_from_samples(&samples, NOTE, 100, held, None, &mut out, HZ)
        .expect("离线释放");
    assert!(released > 0);
    let gain = (100.0f32 / 127.0).powi(2);
    assert!(
        out[..usize::try_from(held).expect("小")]
            .iter()
            .all(|sample| *sample == 0.0),
        "松键之前必须保持原样"
    );
    assert!(
        (out[usize::try_from(held).expect("小")] - 0.25 * gain).abs() <= 1.0e-6,
        "释放首帧 {} != 0.25 × {gain}",
        out[usize::try_from(held).expect("小")]
    );
    // 与实时声部同值。
    let plan = prepare(text);
    let mut pool = SfzVoicePool::new(HZ);
    let _ = pool.trigger_note(&plan, NOTE, 100, u64::MAX, 1.0, None);
    assert_eq!(pool.trigger_release(&plan, NOTE, held, 1.0, None), 1);
    let mut realtime = vec![0.0f32; 128];
    pool.render_into(&plan, 0, &mut realtime);
    assert_eq!(
        out[usize::try_from(held).expect("小")].to_bits(),
        realtime[0].to_bits(),
        "离线从松键帧起的首帧必须与实时首帧逐位相同"
    );
}

#[test]
fn t5_without_release_regions_the_note_off_path_is_a_no_op() {
    let plan = prepare("<region> sample=a.wav key=60");
    assert!(!plan.has_release_regions());
    let mut pool = SfzVoicePool::new(HZ);
    assert_eq!(pool.trigger_note(&plan, NOTE, 100, u64::MAX, 1.0, None), 1);
    assert_eq!(pool.trigger_release(&plan, NOTE, 480, 1.0, None), 0);
}
