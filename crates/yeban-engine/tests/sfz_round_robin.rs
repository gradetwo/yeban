//! ⭐ **round-robin（`seq_position` / `seq_length`）在实时触发路径上的落地**。
//!
//! # 这条判据在修什么
//!
//! `SfzVoicePool::trigger_note` 的候选筛选只做 `accepts(note, velocity)`（键/力度掩码）。
//! 于是 SFZ 里用 `seq_length` ＋ `seq_position` 写成的**轮替组**（同一个音高上两条
//! `sample=` 不同的区域，按第 n 次触发换着用）会被当成"多匹配 ＝ 叠加"：
//! **一次 note-on 把两个变体一起发出来**。
//!
//! 后果不是"多一点点混响"，而是三件事同时发生：
//!
//! * 电平：两个不同采样叠在一起（语料里常是同一乐器的两个发音，相位/噪声不同）
//!   ⇒ 响度与音色都不是作者写的那个；
//! * 声部：每个音吃两个槽 ⇒ 有效复音数**减半** ⇒ 池满提前，窃取数翻倍；
//! * 轮替本身消失：`seq_*` 的全部意义（避免"机关枪效应"）被抹掉。
//!
//! 实测依据（`assets/samples/vsco2ce/Strings` 家族的 spiccato）：
//! `seq_length=2` ＋ `seq_position=1|2` 两条区域 ⇒ 旧行为下 `Strings · Ostinato`
//! 一支轨在 183 s 里被记到 **1084 次声部窃取**。
//!
//! # 判据表
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | R1 | 轮替组**互斥** | 两个变体、`seq_length=2` ⇒ 连发两次：`created` 各为 **1**（⛔ 不是 2）、在场 **2**、输出 **0.75**（＝ a 的 0.5 ＋ b 的 0.25；若两次都发 a 会是 1.0、都发 b 会是 0.5） |
//! | R2 | **没有** `seq_*` 的库照旧叠加（后向兼容） | 两条无 `seq_*` 的区域 ⇒ 一次触发 `created == 2`、输出 **0.75**、`has_round_robin() == false` |
//! | R3 | 轮替序列**按音高**独立 | 音高 60 连发两次后再发音高 62：62 拿到的是**第一个**变体（输出 0.5） |
//! | R4 | `silence()` 让序列**从头** | 触发两次（a,b）后 `silence()` ⇒ 再触发一次输出 **0.5**（＝ a，⛔ 不是 b）—— 重渲染的确定性靠这条 |
//! | R5 | `seq_position` **不覆盖**`1..=L` 时**不静音** | `seq_length=3` 只有 `seq_position=2` ⇒ 每次 `created == 1`（⛔ 不是 0）；`seq_length=2` 只有 `seq_position=1` ⇒ 第二次也不静音 |
//! | R6 | 摊平表**真的带上**两个字段 | `PreparedRegion::seq_position()` / `seq_length()` 逐字回读；`PreparedSfz::round_robin_region_count()` 数得对 |

use std::sync::Arc;

use yeban_engine::sfz_instrument::{InMemorySample, PreparedSfz, SfzInstrument, SfzVoicePool};

/// 变体样本的常值（两条 PCM 只有这一个数不同 ⇒ 输出可**逐值反解**是哪个变体）。
const VARIANT_A: f32 = 0.5;
const VARIANT_B: f32 = 0.25;

/// 样本帧数（远短于一次渲染的块，`end_sample = u64::MAX` ⇒ 不退休）。
const FRAMES: usize = 64;

/// 一条常值 PCM（`Arc` 共享，与真实解码路径同形状）。
fn flat(value: f32) -> InMemorySample {
    InMemorySample {
        pcm: Arc::new(vec![value; FRAMES]),
        sample_hz: 48_000.0,
    }
}

/// 摊平：`a.wav` → `VARIANT_A`、`b.wav` → `VARIANT_B`（其余路径缺样本）。
fn prepare(text: &str) -> PreparedSfz {
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    PreparedSfz::prepare_with_samples(
        &sfz,
        &[
            ("a.wav".to_owned(), flat(VARIANT_A)),
            ("b.wav".to_owned(), flat(VARIANT_B)),
        ],
        48_000.0,
    )
    .expect("摊平必须成功")
}

/// 触发一次后渲染 8 帧，返回（`created`, 首帧输出）。
fn strike(pool: &mut SfzVoicePool, plan: &PreparedSfz, note: u8) -> (usize, f32) {
    let created = pool.trigger_note(plan, note, 127, u64::MAX, 1.0, None);
    let mut out = vec![0.0f32; 8];
    pool.render_into(plan, 0, &mut out);
    (created, out[0])
}

/// 近似比较（`volume` / `velocity_gain` 的乘法链允许 1e-6 的 binary32 误差）。
fn near(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() < 1e-6
}

// ⭐ R1：轮替组互斥。
#[test]
fn r1_a_round_robin_group_alternates_instead_of_stacking() {
    const TEXT: &str = "<group> key=60 seq_length=2\n\
                        <region> sample=a.wav seq_position=1 pitch_keycenter=60\n\
                        <region> sample=b.wav seq_position=2 pitch_keycenter=60\n";
    let plan = prepare(TEXT);
    assert_eq!(plan.region_count(), 2);
    assert!(
        plan.has_round_robin(),
        "两条 `seq_length=2` ⇒ 必须识别成轮替组"
    );
    assert_eq!(plan.round_robin_region_count(), 2);
    assert_eq!(plan.regions()[0].seq_position(), 1);
    assert_eq!(plan.regions()[1].seq_position(), 2);
    assert_eq!(plan.regions()[0].seq_length(), 2);

    let mut pool = SfzVoicePool::new(48_000.0);
    // ① 第一次：只发变体 a。
    let (created_a, out_a) = strike(&mut pool, &plan, 60);
    assert_eq!(
        created_a, 1,
        "⭐ 一次 note-on 只发**一个**变体（⛔ 不是 2）"
    );
    assert!(near(out_a, VARIANT_A), "第一次必须是 a（实得 {out_a}）");
    assert_eq!(pool.active_count(), 1);

    // ② 第二次：只发变体 b（旧行为是两个都发 ⇒ 输出 1.0）。
    let (created_b, out_b) = strike(&mut pool, &plan, 60);
    assert_eq!(created_b, 1);
    assert_eq!(
        pool.active_count(),
        2,
        "两次触发共两个声部（旧行为会是四个）"
    );
    assert!(
        near(out_b, VARIANT_A + VARIANT_B),
        "⭐ 第二次叠加的是 a+b ＝ {}（实得 {out_b}）；若仍是 a 会是 1.0、若是 b 会是 0.5",
        VARIANT_A + VARIANT_B
    );

    // ③ 第三次回到 a ⇒ 输出 a+b+a。
    let (created_c, out_c) = strike(&mut pool, &plan, 60);
    assert_eq!(created_c, 1);
    assert!(
        near(out_c, VARIANT_A + VARIANT_B + VARIANT_A),
        "第三次必须回到 a（实得 {out_c}）"
    );
}

// ⭐ R2：没有 `seq_*` 的库照旧叠加（后向兼容的全部）。
#[test]
fn r2_a_library_without_seq_opcodes_keeps_stacking_every_match() {
    const TEXT: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n\
                        <region> sample=b.wav key=60 pitch_keycenter=60\n";
    let plan = prepare(TEXT);
    assert!(!plan.has_round_robin(), "没有 `seq_length>1` ⇒ 不是轮替库");
    assert_eq!(plan.round_robin_region_count(), 0);

    let mut pool = SfzVoicePool::new(48_000.0);
    let (created, out) = strike(&mut pool, &plan, 60);
    assert_eq!(
        created, 2,
        "⭐ 两条无 `seq_*` 的区域必须**都**发声（layering）"
    );
    assert!(
        near(out, VARIANT_A + VARIANT_B),
        "叠加电平必须是 {}（实得 {out}）",
        VARIANT_A + VARIANT_B
    );
    let (created_again, _) = strike(&mut pool, &plan, 60);
    assert_eq!(created_again, 2, "无 `seq_*` ⇒ 每次触发都是两条");
}

// ⭐ R3：序列按音高独立。
#[test]
fn r3_the_sequence_is_per_pitch() {
    const TEXT: &str = "<group> seq_length=2\n\
                        <region> sample=a.wav key=60 seq_position=1 pitch_keycenter=60\n\
                        <region> sample=b.wav key=60 seq_position=2 pitch_keycenter=60\n\
                        <region> sample=a.wav key=62 seq_position=1 pitch_keycenter=62\n\
                        <region> sample=b.wav key=62 seq_position=2 pitch_keycenter=62\n";
    let plan = prepare(TEXT);
    let mut pool = SfzVoicePool::new(48_000.0);

    // 音高 60 走两步（a 再 b），把它的序列推到第 3 步。
    let _ = strike(&mut pool, &plan, 60);
    let _ = strike(&mut pool, &plan, 60);
    // 音高 62 **第一次**触发 ⇒ 必须是它自己的第一个变体，⛔ 不受 60 的进度影响。
    let (created, out) = strike(&mut pool, &plan, 62);
    assert_eq!(created, 1);
    assert!(
        near(out, VARIANT_A + VARIANT_B + VARIANT_A),
        "音高 62 必须从**自己的**第一个变体开始（实得 {out}）"
    );
}

// ⭐ R4：`silence()` 让序列从头（重渲染确定性的支点）。
#[test]
fn r4_silence_restarts_the_sequence() {
    const TEXT: &str = "<group> key=60 seq_length=2\n\
                        <region> sample=a.wav seq_position=1 pitch_keycenter=60\n\
                        <region> sample=b.wav seq_position=2 pitch_keycenter=60\n";
    let plan = prepare(TEXT);
    let mut pool = SfzVoicePool::new(48_000.0);

    let _ = strike(&mut pool, &plan, 60);
    let (_, out_b) = strike(&mut pool, &plan, 60);
    assert!(near(out_b, VARIANT_A + VARIANT_B));

    pool.silence();
    assert_eq!(pool.active_count(), 0);
    let (created, out) = strike(&mut pool, &plan, 60);
    assert_eq!(created, 1);
    assert!(
        near(out, VARIANT_A),
        "⭐ `silence()`（`seek` / 快照重建都走它）之后必须回到第一个变体（实得 {out}）"
    );

    // 反向腿：**不清空**地再走一步 ⇒ 应当拿到 b（证明不是"永远发 a"）。
    let (_, out_next) = strike(&mut pool, &plan, 60);
    assert!(
        near(out_next, VARIANT_A + VARIANT_B),
        "计数器真的在推进（实得 {out_next}）"
    );
}

// ⭐ R5：`seq_position` 不覆盖 `1..=L` 时不静音（兜底分支）。
#[test]
fn r5_a_missing_seq_position_still_produces_sound() {
    // ① `seq_length=3` 只有 `seq_position=2` ⇒ target 会轮到 1 / 3。
    let plan =
        prepare("<region> sample=a.wav key=60 seq_length=3 seq_position=2 pitch_keycenter=60\n");
    assert!(plan.has_round_robin());
    let mut pool = SfzVoicePool::new(48_000.0);
    for step in 0..3 {
        let (created, out) = strike(&mut pool, &plan, 60);
        assert_eq!(created, 1, "⭐ 第 {step} 步必须兜底发声（⛔ 不是整组静音）");
        assert!(
            near(out, VARIANT_A * (step as f32 + 1.0)),
            "第 {step} 步输出 {out}"
        );
    }

    // ② `seq_length=2` 只有 `seq_position=1` ⇒ 第二次的 target=2 无人命中。
    let plan =
        prepare("<region> sample=a.wav key=60 seq_length=2 seq_position=1 pitch_keycenter=60\n");
    let mut pool = SfzVoicePool::new(48_000.0);
    let (first, _) = strike(&mut pool, &plan, 60);
    let (second, _) = strike(&mut pool, &plan, 60);
    assert_eq!((first, second), (1, 1), "⭐ 两拍都必须出声");
}

// ⭐ R6：摊平表的两个字段逐字回读 ＋ 计数正确。
#[test]
fn r6_the_flattened_table_carries_both_seq_fields() {
    let plan = prepare(
        "<region> sample=a.wav key=60 seq_length=4 seq_position=3 pitch_keycenter=60\n\
         <region> sample=b.wav key=62 pitch_keycenter=62\n",
    );
    assert_eq!(plan.regions()[0].seq_length(), 4);
    assert_eq!(plan.regions()[0].seq_position(), 3);
    assert_eq!(plan.regions()[1].seq_length(), 1, "缺省 `seq_length` 是 1");
    assert_eq!(
        plan.regions()[1].seq_position(),
        1,
        "缺省 `seq_position` 是 1"
    );
    assert_eq!(plan.round_robin_region_count(), 1);
    assert!(plan.has_round_robin());

    // `selects_occurrence` 的四步：3 / 4 / 1 / 2。
    let region = &plan.regions()[0];
    let picked: Vec<u32> = (0..4)
        .filter(|occurrence| region.selects_occurrence(*occurrence))
        .map(|occurrence| occurrence % 4 + 1)
        .collect();
    assert_eq!(picked, vec![3], "四步里只有第 3 步该由它发声");
    assert!(
        plan.regions()[1].selects_occurrence(0) && plan.regions()[1].selects_occurrence(7),
        "`seq_length <= 1` ⇒ 恒发声"
    );
}
