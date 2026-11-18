//! ⭐ **触发期随机族**（`offset_random` / `amp_random`）在实时与离线两条路上的落地。
//!
//! # 这条判据在修什么
//!
//! 登记语料里 `offset_random` 出现在 **79** 个 `.sfz` 文件（**4 901** 处）、`amp_random`
//! 出现在 **2** 个文件（**19** 处），此前解析层与播放层**一个都不认**：未知 opcode 被
//! 静默忽略 ⇒ 写这两族的库每次触发都从**同一个**固定起点、以**同一个**固定音量发声，
//! 随机变化整段消失，而且**没有任何告警**。
//!
//! # 口径（规范出处与工程裁决的完整文本在 `yeban-sfz` 的 `Region` 字段文档里）
//!
//! * `offset_random`："Random offset added to the region offset."、Range `0 to 4294967296`、
//!   "**Unipolar in ARIA, Cakewalk and rcg sfz.**" ⇒ 位移 `floor(rand × N)` ∈ `[0, N)`；
//! * `amp_random`："Random volume for the region, in decibels."、Range `0 to 24`、
//!   "In ARIA and Cakewalk, `amp_random` is **unipolar**." ⇒ 乘子 `10 ^ (−(rand × dB) / 20)`；
//! * 两者与 `lorand`/`hirand` **共用**同一个 `trigger_rand(occurrence, note, 1)` 读数
//!   （规范对三族都写 "Computed when the note is triggered"）。
//!
//! # 判据表
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | R1 | 摊平表真的带上两族读数 | `PreparedRegion::offset_random()` / `amp_random_db()` |
//! | R2 | 实时起点按 `trigger_rand` 位移 | 首帧 == `pcm[100 + floor(rand × 800)]` |
//! | R3 | 离线（`occurrence = Some(0)`）与实时**同一触发身份 ⇒ 同一个起点** | 首帧逐位相同 |
//! | R4 | `amp_random` 是单极衰减、两条路同一乘子 | 与 `amp_random=0` 的比值逐位 == `10 ^ (−(rand × 24) / 20)` |
//! | R5 | 两族都不写 ⇒ 逐位不变 | 首帧 == `pcm[0]`，且实时 == 离线 |
//! | R6 | `has_rand` 的语义扩到"需要随机数" | 三族各自 `true`、都没有 `false` |
//! | R7 | 位移进循环窗口之后**回绕**而不是越界/静音 | 首帧 == `pcm[200 + (起点 − 200) % 200] × 增益链`，且离线同身份逐位相同 |

use std::collections::BTreeMap;
use std::sync::Arc;

use yeban_engine::sfz_instrument::{InMemorySample, PreparedSfz, SfzInstrument, SfzVoicePool};
use yeban_sfz::instrument::trigger_rand;

/// 引擎采样率（与样本同率 ⇒ `pitch_keytrack` 在 key == keycenter 上速率恰 1）。
const HZ: f32 = 48_000.0;
/// 样本长度（帧）：`pcm[i] = i / 1024` ⇒ 首帧可以**反解**出读的是哪个源采样点。
const FRAMES: usize = 1_024;
/// 触发音符（= 区域根音 ⇒ 速率恰 1）。
const NOTE: u8 = 60;
/// 每次触发渲染的长度（帧）。
const OUT: usize = 64;

/// `pcm[i] = i / FRAMES` 的斜坡样本（首帧 ⇒ 源起点可反解）。
fn ramp() -> InMemorySample {
    let pcm: Vec<f32> = (0..FRAMES).map(|i| i as f32 / FRAMES as f32).collect();
    InMemorySample {
        channels: 1,
        pcm: Arc::new(pcm),
        sample_hz: HZ,
        loop_points: None,
    }
}

/// 摊平一份只引用 `a.wav` 的 SFZ。
fn prepare(text: &str) -> PreparedSfz {
    let instrument =
        SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    PreparedSfz::prepare_with_samples(&instrument, &[("a.wav".to_owned(), ramp())], HZ)
        .expect("摊平必须成功")
}

/// 实时触发一次（第一次触发 ⇒ `occurrence = 0`）并返回首帧。
fn realtime_first_frame(text: &str) -> f32 {
    let plan = prepare(text);
    let mut pool = SfzVoicePool::new(HZ);
    let created = pool.trigger_note(&plan, NOTE, 100, u64::MAX, 1.0, None);
    assert_eq!(created, 1, "单区域夹具 ⇒ 恰好建 1 个声部");
    let mut out = vec![0.0f32; OUT];
    pool.render_into(&plan, 0, &mut out);
    out[0]
}

/// 离线渲染一次（`occurrence = Some(k)`）并返回首帧。
fn offline_first_frame(text: &str, occurrence: Option<u32>) -> f32 {
    let instrument =
        SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let mut samples = BTreeMap::new();
    samples.insert("a.wav".to_owned(), ramp());
    let mut out = vec![0.0f32; OUT];
    instrument
        .render_note_into_from_samples(&samples, NOTE, 100, None, occurrence, &mut out, HZ)
        .expect("离线渲染必须成功");
    out[0]
}

/// 源采样点 `index` 在斜坡样本上的值。
fn pcm_at(index: usize) -> f32 {
    index as f32 / FRAMES as f32
}

#[test]
fn r1_the_flattened_table_carries_both_readings() {
    let plan = prepare("<region>sample=a.wav key=60 offset=100 offset_random=800 amp_random=24");
    let region = &plan.regions()[0];
    assert_eq!(region.offset_random(), 800);
    assert_eq!(region.amp_random_db(), 24.0);
    // 缺省：两族都不在场 ⇒ 读数恰为规范缺省。
    let plain = prepare("<region>sample=a.wav key=60");
    assert_eq!(plain.regions()[0].offset_random(), 0);
    assert_eq!(plain.regions()[0].amp_random_db(), 0.0);
}

#[test]
fn r2_the_realtime_start_is_shifted_by_the_trigger_identity() {
    let text = "<region>sample=a.wav key=60 offset=100 offset_random=800";
    // 第一次触发 ⇒ `occurrence = 0` ⇒ 与离线 `Some(0)` 同一个随机数。
    let rand = trigger_rand(0, NOTE, 1);
    let shift = (f64::from(rand) * 800.0) as usize;
    assert!(shift < 800, "单极位移必须 < 800，实得 {shift}");
    // 增益链（力度 × 音量 × …）与随机位移无关 ⇒ 用无位移那一档当**同一份**参照。
    let plain = realtime_first_frame("<region>sample=a.wav key=60 offset=100");
    let scale = plain / pcm_at(100);
    let expected = pcm_at(100 + shift) * scale;
    let actual = realtime_first_frame(text);
    assert!(
        (actual - expected).abs() <= 1.0e-7,
        "实时首帧 {actual} != pcm[{}] × {scale} = {expected}（rand = {rand}）",
        100 + shift
    );
}

#[test]
fn r3_offline_and_realtime_pick_the_same_start_for_the_same_identity() {
    let text = "<region>sample=a.wav key=60 offset=100 offset_random=800";
    let realtime = realtime_first_frame(text);
    let offline = offline_first_frame(text, Some(0));
    assert_eq!(
        realtime.to_bits(),
        offline.to_bits(),
        "同一触发身份 ⇒ 同一个起点（实时 {realtime}／离线 {offline}）"
    );
    // 不同身份**可以**给出不同起点（不是常数位移）：扫 16 个身份，至少两个不同。
    let mut starts = BTreeMap::new();
    for occurrence in 0..16u32 {
        starts.insert(
            offline_first_frame(text, Some(occurrence)).to_bits(),
            occurrence,
        );
    }
    assert!(
        starts.len() >= 2,
        "16 个触发身份只给出一种起点 ⇒ 位移没在变"
    );
    // `occurrence = None`（旧入口）⇒ 位移恰 0 ⇒ 与"不写 `offset_random`"逐位相同。
    let deterministic = offline_first_frame(text, None);
    let plain = offline_first_frame("<region>sample=a.wav key=60 offset=100", None);
    assert_eq!(deterministic.to_bits(), plain.to_bits());
}

#[test]
fn r4_amp_random_is_a_unipolar_attenuation_shared_by_both_paths() {
    // 两档都带 `offset=100`：首帧非零 ⇒ 比值（＝随机乘子）可求。
    let plain = "<region>sample=a.wav key=60 offset=100";
    let random = "<region>sample=a.wav key=60 offset=100 amp_random=24";
    let baseline_rt = realtime_first_frame(plain);
    let attenuated_rt = realtime_first_frame(random);
    let baseline_off = offline_first_frame(plain, Some(0));
    let attenuated_off = offline_first_frame(random, Some(0));
    // 两条路对同一触发身份给出**同一个**乘子。
    let rand = trigger_rand(0, NOTE, 1);
    let expected = 10.0f32.powf(-(rand * 24.0) / 20.0);
    let ratio_rt = attenuated_rt / baseline_rt;
    let ratio_off = attenuated_off / baseline_off;
    assert!(
        (ratio_rt - expected).abs() <= 1.0e-6,
        "实时乘子 {ratio_rt} != {expected}（rand = {rand}）"
    );
    assert!(
        (ratio_off - expected).abs() <= 1.0e-6,
        "离线乘子 {ratio_off} != {expected}（rand = {rand}）"
    );
    assert!(
        (ratio_rt - ratio_off).abs() <= 1.0e-6,
        "两条路的乘子必须同值（实时 {ratio_rt}／离线 {ratio_off}）"
    );
    // 单极 ⇒ 乘子 ∈ (0, 1]：⛔ 不会把音量放大。
    assert!(ratio_rt > 0.0 && ratio_rt <= 1.0, "实得 {ratio_rt}");
}

#[test]
fn r5_absent_opcodes_leave_the_output_bit_identical() {
    let text = "<region>sample=a.wav key=60";
    let realtime = realtime_first_frame(text);
    let offline = offline_first_frame(text, Some(0));
    // 两种口径都与"无位移、无随机音量"的确定性读数逐位相同。
    assert_eq!(realtime.to_bits(), pcm_at(0).to_bits());
    assert_eq!(realtime.to_bits(), offline.to_bits());
    // 显式写出缺省值（`offset_random=0 amp_random=0`）也必须与不写逐位相同。
    let explicit = "<region>sample=a.wav key=60 offset_random=0 amp_random=0";
    assert_eq!(
        realtime_first_frame(explicit).to_bits(),
        realtime.to_bits(),
        "显式缺省值必须与不写逐位相同"
    );
}

#[test]
fn r6_has_rand_means_this_trigger_needs_a_random_number() {
    assert!(!prepare("<region>sample=a.wav key=60").has_rand());
    assert!(prepare("<region>sample=a.wav key=60 offset_random=1").has_rand());
    assert!(prepare("<region>sample=a.wav key=60 amp_random=1").has_rand());
    // 随机门仍在（语义被**扩**，⛔ 不是被替换）。
    assert!(prepare("<region>sample=a.wav key=60 lorand=0.5").has_rand());
}

#[test]
fn r7_offset_random_inside_a_loop_window_wraps_instead_of_reading_past_the_end() {
    // 规范页把"loop 样本从随机起点起播"列为 `offset_random` 的用途之一 ⇒ 起点被
    // 推进到 `loop_end` 之后时必须**回绕**，⛔ 不能越界读、⛔ 不能静音。
    let text = "<region>sample=a.wav key=60 offset=100 offset_random=1000 \
                loop_mode=loop_continuous loop_start=200 loop_end=400";
    let plan = prepare(text);
    let mut pool = SfzVoicePool::new(HZ);
    assert_eq!(pool.trigger_note(&plan, NOTE, 100, u64::MAX, 1.0, None), 1);
    let mut out = vec![0.0f32; 64];
    pool.render_into(&plan, 0, &mut out);
    assert!(out.iter().all(|sample| sample.is_finite()), "全是有限值");
    // `rand = trigger_rand(0) = 0.5592167` ⇒ 位移 559 ⇒ 起点 659，越过 `loop_end=400`
    // ⇒ 首帧按 `[200, 400)` 的窗口回绕到 `200 + (659 - 200) % 200 = 259`。
    let rand = trigger_rand(0, NOTE, 1);
    let shift = (f64::from(rand) * 1_000.0) as usize;
    let start = 100 + shift;
    let wrapped = 200 + (start - 200) % 200;
    let plain = realtime_first_frame("<region>sample=a.wav key=60 offset=100");
    let scale = plain / pcm_at(100);
    let expected = pcm_at(wrapped) * scale;
    assert!(
        (out[0] - expected).abs() <= 1.0e-7,
        "首帧 {} != pcm[{wrapped}] × {scale} = {expected}（起点 {start} 回绕）",
        out[0]
    );
    // 离线同身份 ⇒ 同一首帧。
    assert_eq!(
        out[0].to_bits(),
        offline_first_frame(text, Some(0)).to_bits(),
        "回绕腿也必须三条路同口径"
    );
}
