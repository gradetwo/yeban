//! ⭐ **离线渲染的三条缺口**（任务 `#310`）：这些不是"新功能"，而是**离线与实时不一致**。
//!
//! # 为什么要有这批判据
//!
//! 一次真实的编曲事故：用 CLI `--sfz <音色> --export-wav` 渲了一首 188 s 的歌，
//! 弦乐"断气"、吉他硬切、鼓的 `kick` 尾巴莫名消失。逐层排查后落在三个**同一类**缺陷上：
//! 实时声部（[`SfzVoicePool`]）拥有的语义，离线两条路（[`SfzInstrument::render_note_into`] /
//! [`PreparedSfz::render_block_into`]）**一条都没有**。
//!
//! | # | 缺口 | 用户听到的 | 代码位置 |
//! | :--- | :--- | :--- | :--- |
//! | ① | 离线**没有 note-off** ⇒ `ampeg_release` 永不生效 | 音符在时值处被硬切，"断气"、"爆音" | `EnvRun::new(.., u64::MAX, ..)` |
//! | ② | 离线**没有轮替**（`seq_*`）⇒ 所有变体同相叠加 | 电平莫名 +6 dB、机关枪效应 | `for region in regions()` 无 occurrence 过滤 |
//! | ③ | 离线**只有一份样本** ⇒ 所有匹配区域都用它 | 力度分层/多音区共用一个采样 | `render_note_into(source)` 的单一 `source` |
//!
//! # 判据表
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | O1 | `ampeg_release` **真的生效** | `held = 2400`、`release = 0.1 s`（4800 帧）⇒ `written == 7200`，且 `t = 2400` 处仍是稳定值、`t = 3600` 处已衰减、`t = 7200` 之后逐位 0 |
//! | O2 | **旧入口逐位不变** | [`SfzInstrument::render_note_into`] 在 `release = 0.1` 下**整段恒定**（＝ 接线前"永不释放"） |
//! | O3 | `held_frames = None` ⇒ **与旧入口逐位相同** | 两条路的输出**逐 `f32` 位相等** |
//! | O4 | **没有 `ampeg_*` 的区域也响应 note-off** | `written == held`（⛔ 不是"播到样本尾"） |
//! | O5 | `loop_mode=one_shot` **忽略 note-off** | `written == pcm_len`（播满整个采样） |
//! | O6 | 轮替**互斥** | `seq_length=2` 两个变体（0.5 / 0.25）⇒ `occurrence=0` 出 `a`、`1` 出 `b`、`None` 出**两者之和** |
//! | O7 | **region 级样本** | 两个 `key=` 各自的 `sample=` ⇒ 各出各的值 |
//! | O8 | 表里**缺**该区域的样本 ⇒ **不发声** | `written == 0`（⛔ 不借用别的样本、⛔ 不回退正弦） |
//!
//! ⚠ O2／O3 是**后向兼容的护栏**：本片的全部新语义都必须由"调用方显式给参数"开启，
//! 旧入口的字节一位都不许变。

use std::collections::BTreeMap;
use std::sync::Arc;

use yeban_engine::sfz_instrument::{InMemorySample, SfzInstrument};

/// 引擎采样率（判据里的"帧"一律指它）。
const HZ: f32 = 48_000.0;

/// 一条常值 PCM（`Arc` 共享，与真实解码路径同形状）。
fn flat(value: f32, frames: usize) -> InMemorySample {
    InMemorySample {
        pcm: Arc::new(vec![value; frames]),
        sample_hz: HZ,
    }
}

/// 单区域 SFZ ＋ 一份样本表。
fn one_region(
    ops: &str,
    value: f32,
    frames: usize,
) -> (SfzInstrument, BTreeMap<String, InMemorySample>) {
    let text = format!("<region> sample=a.wav key=60 {ops}");
    let sfz = SfzInstrument::load(text, &Default::default()).expect("SFZ 必须解析成功");
    let mut table = BTreeMap::new();
    table.insert("a.wav".to_owned(), flat(value, frames));
    (sfz, table)
}

/// 旧入口的读数（同一份 SFZ、同一份样本、无 note-off）。
///
/// ⚠ `pcm_frames` 与 `out_frames` **必须分开**：`mix_span_into` 的停止条件是
/// "插值要读的下一格仍在 PCM 内"，PCM 只有 `out_frames` 长时最后一帧不会被写
/// （那是判据 O3 第一版失败的真正原因 —— 两条路的 PCM 长度不同，不是语义不同）。
fn legacy_level(sfz: &SfzInstrument, value: f32, pcm_frames: usize, out_frames: usize) -> Vec<f32> {
    let mut out = vec![0.0_f32; out_frames];
    sfz.render_note_into(&flat(value, pcm_frames), 60, 100, &mut out, HZ)
        .expect("旧入口渲染");
    out
}

/// 判据 O1：`ampeg_release` 在离线渲染里**真的生效**。
#[test]
fn o1_release_takes_effect_offline() {
    let (sfz, table) = one_region("ampeg_release=0.1", 0.5, 96_000);
    let legacy = legacy_level(&sfz, 0.5, 96_000, 9_600);
    let base = legacy[100];
    assert!(base > 0.0, "夹具必须出声：base={base}");

    let held = 2_400_u64; // 50 ms
    let release = 4_800_u64; // ampeg_release=0.1 s @ 48 kHz
    let mut out = vec![0.0_f32; 9_600];
    let written = sfz
        .render_note_into_from_samples(&table, 60, 100, Some(held), None, &mut out, HZ)
        .expect("新入口渲染");

    assert_eq!(
        written as u64,
        held + release,
        "写入帧数必须恰好是 note-off ＋ 释放段（音源 PCM 长 96 000 帧，⛔ 不是被 PCM 截断）"
    );
    assert!(
        (out[100] - base).abs() < 1e-6,
        "起音段必须与旧入口同值：{} vs {base}",
        out[100]
    );
    assert!(
        (out[2_399] - base).abs() < 1e-6,
        "note-off 前一帧仍应满值（sustain 段）：{}",
        out[2_399]
    );
    assert!(
        out[3_600] < base * 0.8,
        "note-off 后 25 ms（释放段中点）必须已明显衰减：{} vs {base}",
        out[3_600]
    );
    assert_eq!(
        out[7_200].to_bits(),
        0.0_f32.to_bits(),
        "释放段走完（note-off + 4800 帧）之后必须逐位 0"
    );
}

/// 判据 O2：旧入口在**写了 `ampeg_release`** 的 SFZ 上仍然"永不释放"（接线前行为）。
#[test]
fn o2_legacy_entry_never_releases() {
    let (sfz, _table) = one_region("ampeg_release=0.1", 0.5, 96_000);
    let out = legacy_level(&sfz, 0.5, 96_000, 9_600);
    let base = out[100];
    assert!(base > 0.0);
    for index in [2_400, 3_600, 5_760, 9_000] {
        assert!(
            (out[index] - base).abs() < 1e-6,
            "旧入口第 {index} 帧必须恒等于 {base}（无 note-off）：{}",
            out[index]
        );
    }
}

/// 判据 O3：`held_frames = None` ⇒ 与旧入口**逐位相同**。
#[test]
fn o3_none_is_bit_identical_to_the_legacy_entry() {
    let (sfz, table) = one_region(
        "ampeg_attack=0.01 ampeg_decay=0.05 ampeg_sustain=40",
        0.5,
        96_000,
    );
    let legacy = legacy_level(&sfz, 0.5, 96_000, 9_600);
    let mut new = vec![0.0_f32; 9_600];
    let written = sfz
        .render_note_into_from_samples(&table, 60, 100, None, None, &mut new, HZ)
        .expect("新入口渲染");
    assert_eq!(written, 9_600, "两条路都必须写满缓冲");
    for (index, (a, b)) in legacy.iter().zip(new.iter()).enumerate() {
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "第 {index} 帧逐位不同：{a} vs {b}（`held = None` 必须是恒等路径）"
        );
    }
}

/// 判据 O4：**没有 `ampeg_*`** 的区域也必须响应 note-off（当场停）。
#[test]
fn o4_no_envelope_still_honours_note_off() {
    let (sfz, table) = one_region("", 0.5, 96_000);
    let held = 1_200_u64;
    let mut out = vec![0.0_f32; 9_600];
    let written = sfz
        .render_note_into_from_samples(&table, 60, 100, Some(held), None, &mut out, HZ)
        .expect("渲染");
    assert_eq!(
        written as u64, held,
        "无 `ampeg_*` ⇒ 释放段 0 帧 ⇒ 在 note-off 当帧停（⛔ 不是播到样本尾）"
    );
    assert!(out[100] > 0.0, "note-off 之前必须出声");
    assert_eq!(
        out[1_200].to_bits(),
        0.0_f32.to_bits(),
        "note-off 之后必须逐位 0"
    );
}

/// 判据 O5：`loop_mode=one_shot` 忽略 note-off（播满整个采样）。
#[test]
fn o5_one_shot_ignores_note_off() {
    let (sfz, table) = one_region("loop_mode=one_shot", 0.5, 4_096);
    let mut out = vec![0.0_f32; 9_600];
    let written = sfz
        .render_note_into_from_samples(&table, 60, 100, Some(1_200), None, &mut out, HZ)
        .expect("渲染");
    // 线性插值要求"下一格"仍在 PCM 内 ⇒ 最后一帧不写（与 `mix_span_into` 的停止条件同口径）。
    assert!(
        written >= 4_094,
        "one_shot 必须播到样本尾（PCM 4096 帧）：written={written}"
    );
    assert!(out[2_000] > 0.0, "note-off 之后仍在响");
}

/// 判据 O6：轮替**互斥**，且 `None` 保持"全部叠加"的旧行为。
#[test]
fn o6_round_robin_is_mutually_exclusive() {
    let sfz = SfzInstrument::load(
        "<group>key=60 seq_length=2\n\
         <region>seq_position=1 sample=a.wav\n\
         <region>seq_position=2 sample=b.wav"
            .to_owned(),
        &Default::default(),
    )
    .expect("解析");
    let mut table = BTreeMap::new();
    table.insert("a.wav".to_owned(), flat(0.5, 4_096));
    table.insert("b.wav".to_owned(), flat(0.25, 4_096));

    let level = |occurrence: Option<u32>| {
        let mut out = vec![0.0_f32; 256];
        sfz.render_note_into_from_samples(&table, 60, 100, None, occurrence, &mut out, HZ)
            .expect("渲染");
        out[32]
    };
    let a = level(Some(0));
    let b = level(Some(1));
    let both = level(None);
    assert!(a > 0.0 && b > 0.0, "两个变体都必须出声：a={a} b={b}");
    assert!(
        (both - (a + b)).abs() < 1e-5,
        "`occurrence = None` 必须退回'全部叠加'的旧行为：{both} vs {}",
        a + b
    );
    assert!(
        (a - 2.0 * b).abs() < 1e-5,
        "两个变体的样本是 0.5 / 0.25 ⇒ 输出也应成 2:1：{a} vs {b}"
    );
}

/// 判据 O7：两个区域各用自己的 `sample=`。
#[test]
fn o7_each_region_uses_its_own_sample() {
    let sfz = SfzInstrument::load(
        "<region> sample=low.wav key=60\n<region> sample=high.wav key=64".to_owned(),
        &Default::default(),
    )
    .expect("解析");
    let mut table = BTreeMap::new();
    table.insert("low.wav".to_owned(), flat(0.5, 4_096));
    table.insert("high.wav".to_owned(), flat(0.25, 4_096));

    let level = |note: u8| {
        let mut out = vec![0.0_f32; 256];
        sfz.render_note_into_from_samples(&table, note, 100, None, None, &mut out, HZ)
            .expect("渲染");
        out[32]
    };
    let low = level(60);
    let high = level(64);
    assert!(low > 0.0 && high > 0.0, "两条区域都必须出声");
    assert!(
        (low - 2.0 * high).abs() < 1e-5,
        "音高 60 / 64 必须各取自己的样本（0.5 / 0.25）：{low} vs {high}"
    );
}

/// 判据 O8：表里**缺**该区域的样本 ⇒ 该区域不发声（⛔ 不借用、⛔ 不回退正弦）。
#[test]
fn o8_missing_sample_silences_that_region_only() {
    let sfz = SfzInstrument::load(
        "<region> sample=low.wav key=60\n<region> sample=high.wav key=64".to_owned(),
        &Default::default(),
    )
    .expect("解析");
    let mut table = BTreeMap::new();
    table.insert("low.wav".to_owned(), flat(0.5, 4_096));
    // `high.wav` 故意不给。

    let mut out = vec![0.0_f32; 256];
    let written = sfz
        .render_note_into_from_samples(&table, 64, 100, None, None, &mut out, HZ)
        .expect("渲染");
    assert_eq!(written, 0, "缺样本的区域必须一帧都不写");
    assert!(
        out.iter().all(|v| v.to_bits() == 0.0_f32.to_bits()),
        "缺样本 ⇒ 全零（⛔ 不是内存正弦）"
    );
}
