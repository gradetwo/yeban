//! ⭐ **随机轮替（`lorand` / `hirand`）在实时触发与离线渲染两条路上的落地**。
//!
//! # 这条判据在修什么
//!
//! `lorand` / `hirand` 此前在全仓**零实现**：解析层不认、引擎不判，未知
//! opcode 被静默忽略 ⇒ 写了随机 round-robin 的库（规范的标准写法：四段
//! `[0, .25) / [.25, .5) / [.5, .75) / [.75, 1)` 各放一个变体）在夜半里
//! 四个 region 无条件全匹配、同相位叠加发声 —— 电平虚高、梳状滤波、
//! 轮替本身消失，而且**没有任何告警**。
//!
//! # 确定性口径
//!
//! 随机数不是真随机：[`yeban_sfz::instrument::trigger_rand`] 把它定义成
//! 触发身份 `(occurrence, note, channel)` 的纯函数（splitmix64 终混）。
//! 同一触发序列必然选出同一串变体（离线可复现），同次触发全部 region
//! 共用一个值（规范要求；逐 region 各抽是格式页点名的 ARIA 反例）。
//! 本文件的判定量因此可以**先算期望、再对实测**，不靠统计蒙。
//!
//! # 判据表
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | G1 | 实时互斥＋覆盖＋可复现 | 铺满 `[0,1)` 的两段：连发 64 次，每次 `created == 1`、首帧 ＝ 按 `trigger_rand` 期望的变体值、两段都被选到；`silence()` 后重扫 ⇒ 变体序列逐一相同 |
//! | G2 | 实时落缝静音 | 覆盖只到 `0.5` 的两段：`rand >= 0.5` 的触发 `created == 0`，其余 `created == 1` 且变体对 |
//! | G3 | 无随机门的库逐位不变 | 两条无 `lorand`/`hirand` 的区域 ⇒ `has_rand() == false`、一次触发 `created == 2`（照旧叠加） |
//! | G4 | 离线互斥 | 同一两段库走 `render_note_into_from_samples`：`occurrence = Some(k)` ⇒ 首帧恰一变体；`None` ⇒ 两者之和（旧叠加保留） |
//! | G5 | 离线与实时同选 | 同一 `k` 序列：离线首帧变体与实时变体逐一相同 |
//! | G6 | 摊平表真的带上 | `PreparedRegion::lorand()` / `hirand()` 逐字回读；`PreparedSfz::has_rand()` 对有门/无门两态 |
//! | G7 | `seq_*` × rand 组合（规范示例形状） | 8 区域（`seq_length=2` 两子集 × rand 四等分）：每次 `created == 1`，命中同时满足 `seq_position == occurrence % 2 + 1` 与本次随机门 |

use std::collections::BTreeMap;
use std::sync::Arc;

use yeban_engine::sfz_instrument::{InMemorySample, PreparedSfz, SfzInstrument, SfzVoicePool};
use yeban_sfz::instrument::trigger_rand;

/// 变体样本的常值（两条 PCM 只有这一个数不同 ⇒ 首帧可**逐值反解**是哪个变体）。
const VARIANT_A: f32 = 0.5;
const VARIANT_B: f32 = 0.25;

/// 引擎采样率。
const HZ: f32 = 48_000.0;

/// 样本帧数（远短于每次触发后的渲染长度 ⇒ 声部在下次触发前必然播完退休）。
const FRAMES: usize = 64;

/// 一条常值 PCM（`Arc` 共享，与真实解码路径同形状）。
fn flat(value: f32) -> InMemorySample {
    InMemorySample {
        pcm: Arc::new(vec![value; FRAMES]),
        sample_hz: HZ,
        loop_points: None,
    }
}

/// 摊平两段库：`a.wav` → `VARIANT_A`、`b.wav` → `VARIANT_B`。
fn prepare(text: &str) -> PreparedSfz {
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    PreparedSfz::prepare_with_samples(
        &sfz,
        &[
            ("a.wav".to_owned(), flat(VARIANT_A)),
            ("b.wav".to_owned(), flat(VARIANT_B)),
        ],
        HZ,
    )
    .expect("摊平必须成功")
}

/// 触发一次并渲染到声部播完，返回（`created`, 首帧输出）。
fn strike(pool: &mut SfzVoicePool, plan: &PreparedSfz, note: u8) -> (usize, f32) {
    let created = pool.trigger_note(plan, note, 127, u64::MAX, 1.0, None);
    let mut out = vec![0.0f32; 512];
    pool.render_into(plan, 0, &mut out);
    (created, out[0])
}

/// 近似比较（增益链的乘法允许 1e-6 的 binary32 误差）。
fn near(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() < 1e-6
}

/// 第 `k` 次触发（音高 60、通道 1）按口径应选的变体值（铺满两段库）。
fn expected_tiling(k: u64) -> f32 {
    if trigger_rand(k, 60, 1) < 0.25 {
        VARIANT_A
    } else {
        VARIANT_B
    }
}

// ⭐ G1：实时互斥＋覆盖＋可复现。
#[test]
fn g1_a_tiling_random_group_picks_exactly_one_variant_per_note_on() {
    const TEXT: &str = "<region> sample=a.wav key=60 pitch_keycenter=60 hirand=0.25\n\
                        <region> sample=b.wav key=60 pitch_keycenter=60 lorand=0.25\n";
    let plan = prepare(TEXT);
    assert!(plan.has_rand(), "写窄了的 rand 区间 ⇒ 必须识别成随机轮替库");
    assert!(!plan.has_round_robin(), "没有 `seq_*` ⇒ 不是序列轮替库");

    let mut pool = SfzVoicePool::new(HZ);
    let mut first_sweep = Vec::new();
    for k in 0..64u64 {
        let (created, out) = strike(&mut pool, &plan, 60);
        assert_eq!(created, 1, "第 {k} 次触发必须恰好建一个声部（⛔ 不是叠加）");
        assert_eq!(pool.active_count(), 0, "样本播完后声部应退休");
        let expected = expected_tiling(k);
        assert!(
            near(out, expected),
            "第 {k} 次触发应选 {expected}（rand = {}），实得 {out}",
            trigger_rand(k, 60, 1)
        );
        first_sweep.push(out);
    }
    assert!(
        first_sweep.iter().any(|&v| near(v, VARIANT_A))
            && first_sweep.iter().any(|&v| near(v, VARIANT_B)),
        "64 次触发两段都应被选到"
    );

    // 可复现：`silence()` 把每键计数归零 ⇒ 同一串触发逐一重选同一变体。
    pool.silence();
    for (k, &before) in first_sweep.iter().enumerate() {
        let (created, out) = strike(&mut pool, &plan, 60);
        assert_eq!(created, 1);
        assert!(near(out, before), "重扫第 {k} 次应与首扫逐一相同");
    }
}

// ⭐ G2：实时落缝静音（规范明示，⛔ 不兜底改选）。
#[test]
fn g2_a_gap_in_rand_coverage_is_silence_for_that_trigger() {
    const TEXT: &str = "<region> sample=a.wav key=60 pitch_keycenter=60 hirand=0.25\n\
                        <region> sample=b.wav key=60 pitch_keycenter=60 lorand=0.25 hirand=0.5\n";
    let plan = prepare(TEXT);
    let mut pool = SfzVoicePool::new(HZ);
    let mut sounded = 0usize;
    let mut silent = 0usize;
    for k in 0..64u64 {
        let (created, out) = strike(&mut pool, &plan, 60);
        let rand = trigger_rand(k, 60, 1);
        if rand >= 0.5 {
            silent += 1;
            assert_eq!(
                created, 0,
                "第 {k} 次触发 rand={rand} 落进覆盖缝 ⇒ 不建声部"
            );
            assert!(near(out, 0.0), "落缝触发必须静音，实得 {out}");
        } else {
            sounded += 1;
            assert_eq!(created, 1, "第 {k} 次触发 rand={rand} 应恰好一个声部");
            let expected = if rand < 0.25 { VARIANT_A } else { VARIANT_B };
            assert!(
                near(out, expected),
                "第 {k} 次触发变体应为 {expected}，实得 {out}"
            );
        }
    }
    assert!(
        sounded > 0 && silent > 0,
        "64 次里两支都应出现：发声 {sounded}、落缝 {silent}"
    );
}

// ⭐ G3：无随机门的库逐位不变（后向兼容的全部）。
#[test]
fn g3_a_library_without_rand_opcodes_keeps_stacking() {
    const TEXT: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n\
                        <region> sample=b.wav key=60 pitch_keycenter=60\n";
    let plan = prepare(TEXT);
    assert!(!plan.has_rand(), "没有 `lorand`/`hirand` ⇒ 不是随机轮替库");

    let mut pool = SfzVoicePool::new(HZ);
    let (created, out) = strike(&mut pool, &plan, 60);
    assert_eq!(created, 2, "无门区域照旧叠加（两个声部）");
    assert!(
        near(out, VARIANT_A + VARIANT_B),
        "输出应是两变体之和，实得 {out}"
    );
}

// ⭐ G4＋G5：离线互斥，且与实时同选。
#[test]
fn g4_g5_offline_render_picks_the_same_single_variant_as_realtime() {
    const TEXT: &str = "<region> sample=a.wav key=60 pitch_keycenter=60 hirand=0.25\n\
                        <region> sample=b.wav key=60 pitch_keycenter=60 lorand=0.25\n";
    let sfz = SfzInstrument::load(TEXT.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let mut table = BTreeMap::new();
    table.insert("a.wav".to_owned(), flat(VARIANT_A));
    table.insert("b.wav".to_owned(), flat(VARIANT_B));

    for k in 0..32u32 {
        let mut out = vec![0.0f32; 512];
        sfz.render_note_into_from_samples(&table, 60, 127, Some(64), Some(k), &mut out, HZ)
            .expect("离线渲染必须成功");
        let expected = expected_tiling(u64::from(k));
        assert!(
            near(out[0], expected),
            "离线第 {k} 次应选 {expected}（与实时同一公式），实得 {}",
            out[0]
        );
    }

    // `occurrence = None` ⇒ 随机互斥整段跳过 ⇒ 两变体照旧叠加（旧行为保留）。
    let mut out = vec![0.0f32; 512];
    sfz.render_note_into_from_samples(&table, 60, 127, Some(64), None, &mut out, HZ)
        .expect("离线渲染必须成功");
    assert!(
        near(out[0], VARIANT_A + VARIANT_B),
        "`None` 必须叠加（实得 {}）",
        out[0]
    );
}

// ⭐ G6：摊平表真的带上两个字段。
#[test]
fn g6_the_flattened_table_carries_the_rand_bounds() {
    const TEXT: &str = "<region> sample=a.wav key=60 lorand=0.1 hirand=0.4\n\
                        <region> sample=b.wav key=60\n";
    let plan = prepare(TEXT);
    assert!(plan.has_rand());
    assert_eq!(plan.regions()[0].lorand(), 0.1);
    assert_eq!(plan.regions()[0].hirand(), 0.4);
    assert!(plan.regions()[0].has_rand_gate());
    // 缺省对 (0, 1) 原样搬运、不是门。
    assert_eq!(plan.regions()[1].lorand(), 0.0);
    assert_eq!(plan.regions()[1].hirand(), 1.0);
    assert!(!plan.regions()[1].has_rand_gate());
    assert!(plan.regions()[1].selects_rand(0.999));
}

// ⭐ G7：`seq_*` × rand 组合 ＝ 规范示例形状（8 区域）。
#[test]
fn g7_seq_subsets_and_rand_quarters_compose() {
    const TEXT: &str = "\
<region> sample=rr1.wav key=60 pitch_keycenter=60 seq_length=2 seq_position=1 hirand=0.25\n\
<region> sample=rr2.wav key=60 pitch_keycenter=60 seq_length=2 seq_position=1 lorand=0.25 hirand=0.5\n\
<region> sample=rr3.wav key=60 pitch_keycenter=60 seq_length=2 seq_position=1 lorand=0.5 hirand=0.75\n\
<region> sample=rr4.wav key=60 pitch_keycenter=60 seq_length=2 seq_position=1 lorand=0.75\n\
<region> sample=rr5.wav key=60 pitch_keycenter=60 seq_length=2 seq_position=2 hirand=0.25\n\
<region> sample=rr6.wav key=60 pitch_keycenter=60 seq_length=2 seq_position=2 lorand=0.25 hirand=0.5\n\
<region> sample=rr7.wav key=60 pitch_keycenter=60 seq_length=2 seq_position=2 lorand=0.5 hirand=0.75\n\
<region> sample=rr8.wav key=60 pitch_keycenter=60 seq_length=2 seq_position=2 lorand=0.75\n";
    let sfz = SfzInstrument::load(TEXT.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    // 八个变体用互不相同的常值（0.05 × 序号），首帧逐值反解命中哪一条。
    let samples: Vec<(String, InMemorySample)> = (1..=8)
        .map(|i| (format!("rr{i}.wav"), flat(0.05 * i as f32)))
        .collect();
    let plan = PreparedSfz::prepare_with_samples(&sfz, &samples, HZ).expect("摊平必须成功");
    assert!(plan.has_round_robin() && plan.has_rand());

    let mut pool = SfzVoicePool::new(HZ);
    for k in 0..32u64 {
        let (created, out) = strike(&mut pool, &plan, 60);
        assert_eq!(created, 1, "第 {k} 次触发必须恰好一个声部");
        // 反解命中的区域序号（1-based，与 sample 名一致）。
        let hit = (1..=8)
            .find(|&i| near(out, 0.05 * i as f32))
            .unwrap_or_else(|| panic!("第 {k} 次触发首帧 {out} 不对应任何变体"));
        let region = &plan.regions()[hit - 1];
        assert_eq!(
            region.seq_position(),
            (k % 2) as u32 + 1,
            "第 {k} 次触发的子集应由 seq 定（命中 rr{hit}）"
        );
        assert!(
            region.selects_rand(trigger_rand(k, 60, 1)),
            "第 {k} 次触发命中 rr{hit} 必须通过本次随机门"
        );
    }
}
