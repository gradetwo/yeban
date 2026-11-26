//! ⭐ **SFZ v1 `count`**（T-FIX-645）。
//!
//! # 这条判据在修什么
//!
//! SFZ v1 的 `count` 没有实现：写 `count=3` 只会报 `UnknownOpcode`，
//! 样本只播一次。规范逐字（<https://sfzformat.com/opcodes/count/>）：
//!
//! > "The number of times the sample will be played. If this opcode is
//! > specified, the sample will play as many times as defined. Envelope
//! > generators will not be retriggered on sample restart. When this opcode
//! > is defined, loop_mode is automatically set to **one_shot**."
//! > "count=1 will play the sample once, so it's effectively the same as
//! > setting loop_mode=one_shot."
//!
//! `count=0` 取 **ARIA 口径**（等价于未给出，`loop_mode` 照旧生效；同页
//! "Practical Considerations" 逐字 —— ARIA 与 sfz.dll 把 `count=0` 当作
//! 未设置，Rapture / DropZone 把它当作 `count=1`；本仓既有 ARIA 口径
//! 惯例（`cutoff=0` 等），故取前者；见回执）。
//!
//! # 夹具与算数
//!
//! 斜坡样本 `pcm[i] = i / 1024`（`rate = 1` ⇒ 输出值可**反解**成源下标），
//! 样本长 9600 帧。无 note-off（`held_frames = None`，K6 除外）。
//! 一遍播 `9599` 帧（最后一帧差一格的既有语义：`floor(position) + 1 >=
//! source_end` 时停，见 `sfz_loop_count_crossfade.rs` 的 L3；
//! `count=N` ⇒ 共 `N × 9599` 帧）。
//!
//! | 判据 | 场景 | 期望 |
//! | :--- | :--- | :--- |
//! | K1 | `count=3`（离线） | 写 `28797` 帧；三遍边界反解逐点对得上 |
//! | K2 | 不写 `count`（离线基线） | 写 `9599` 帧 |
//! | K3 | `count=1`（离线） | 与 `loop_mode=one_shot` **逐位相同**、写 `9599` 帧 |
//! | K4 | `count=0` ＋ `loop_continuous` ＋窗口（离线） | 写满输出（`30000`）—— ARIA 口径：`0` ＝ 未给出 |
//! | K5 | `count=3` ＋ `loop_continuous` ＋ `loop_count=7` ＋窗口（离线） | `count` 赢：写 `28797` 帧（无窗口、无尾巴） |
//! | K6 | `count=3` ＋ `held_frames=Some(100)`（离线） | 仍写 `28797` 帧（`one_shot` 忽略 note-off）；对照组（不写 `count`）在释放段停住 |
//! | K7 | 非法 `count` 值 | `load` 响亮 `Err`（`-1` / `4294967296` / `1.5`） |
//! | K8 | `direction=reverse` ＋ `count=2`（离线） | 写 `19198` 帧；两遍都覆盖源 `9599…1` |
//! | K9 | 实时池（单声道）＋ `count=3` | 最后一帧非零下标恰为 `28796` |
//! | K10 | 实时池（立体声）＋ `count=3` | 左右声道**逐位相同** ＋ 最后一帧非零下标 `28796` |
use std::collections::BTreeMap;
use std::sync::Arc;

use yeban_engine::sfz_instrument::{InMemorySample, PreparedSfz, SfzInstrument, SfzVoicePool};

/// 引擎采样率。
const HZ: f32 = 48_000.0;
/// 样本长度（帧）：0.2 s。
const PCM: usize = 9_600;
/// 输出长度（帧）：0.625 s（> K1 的 28797，< 无限循环的写满量）。
const OUT: usize = 30_000;
/// 块长（池侧判据用）。
const BLOCK: usize = 512;
/// 一遍播出的帧数（`PCM − 1`，最后一帧差一格的既有语义）。
const ONE_PLAY: usize = PCM - 1;

/// 斜坡样本：`pcm[i] = i / 1024` ⇒ 输出值可反解下标。
fn ramp() -> InMemorySample {
    let pcm: Vec<f32> = (0..PCM).map(|i| i as f32 / 1024.0).collect();
    InMemorySample {
        channels: 1,
        pcm: Arc::new(pcm),
        sample_hz: HZ,
        loop_points: None,
    }
}

/// 立体声斜坡样本：左右声道同一份斜坡（交错）。
fn stereo_ramp() -> InMemorySample {
    let mut pcm = Vec::with_capacity(PCM * 2);
    for i in 0..PCM {
        let v = i as f32 / 1024.0;
        pcm.push(v);
        pcm.push(v);
    }
    InMemorySample {
        channels: 2,
        pcm: Arc::new(pcm),
        sample_hz: HZ,
        loop_points: None,
    }
}

/// 离线渲染一份 SFZ（单区域），返回（写入帧数, 输出缓冲）。
///
/// ⚠ 力度用 127：`velocity_gain = (vel/127)²`（`amp_veltrack` 缺省 100）⇒
/// 只有 127 的增益恰为 `1.0`，输出值才能**绝对反解**成源下标。
/// `held_frames`：`None` ＝ 无 note-off；`Some(n)` ＝ 第 `n` 帧进释放段。
fn render(ops: &str, held_frames: Option<u64>) -> (usize, Vec<f32>) {
    let text = format!("<region> sample=a.wav key=60 pitch_keycenter=60 {ops}");
    let sfz = SfzInstrument::load(text, &Default::default()).expect("SFZ 必须解析成功");
    let mut table = BTreeMap::new();
    table.insert("a.wav".to_owned(), ramp());
    let mut out = vec![0.0_f32; OUT];
    let written = sfz
        .render_note_into_from_samples(&table, 60, 127, held_frames, None, &mut out, HZ)
        .expect("渲染");
    (written, out)
}

/// 把输出帧 `t` 的值反解成源下标（斜坡夹具：`out[t] == src / 1024`）。
fn src_at(out: &[f32], t: usize) -> i32 {
    (out[t] * 1024.0).round() as i32
}

/// 断言 `t` 处输出的源下标（失败信息里同时报输出值）。
fn assert_src(out: &[f32], t: usize, expected: usize, what: &str) {
    assert_eq!(
        src_at(out, t),
        expected as i32,
        "⭐ {what}：输出帧 {t} 必须是源 {expected}，实得源 {}（值 {}）",
        src_at(out, t),
        out[t],
    );
}

/// ⭐ K1：`count=3` ⇒ 整段播 3 次（`3 × 9599 = 28797` 帧），三遍边界逐点对得上。
#[test]
fn k1_count_3_plays_the_whole_region_three_times() {
    let (written, out) = render("count=3", None);
    let expected = 3 * ONE_PLAY;
    assert_eq!(
        written, expected,
        "⭐ `count=3` ⇒ 3×{ONE_PLAY} ＝ {expected} 帧；实得 {written} 帧"
    );
    assert_src(&out, 0, 0, "第一遍首帧");
    assert_src(&out, ONE_PLAY - 1, PCM - 2, "第一遍末帧");
    assert_src(&out, ONE_PLAY, 0, "第二遍首帧（第一次重播）");
    assert_src(&out, 2 * ONE_PLAY - 1, PCM - 2, "第二遍末帧");
    assert_src(&out, 2 * ONE_PLAY, 0, "第三遍首帧（第二次重播）");
    assert_src(&out, 3 * ONE_PLAY - 1, PCM - 2, "第三遍末帧（最后一帧）");
}

/// ⭐ K2：不写 `count` ⇒ 只播一次（`9599` 帧）—— K1 的"多播"是 `count` 给的。
#[test]
fn k2_without_count_the_region_plays_once() {
    let (written, _) = render("", None);
    assert_eq!(
        written, ONE_PLAY,
        "⭐ 不写 `count` ⇒ 只播一次（{ONE_PLAY} 帧）；实得 {written} 帧"
    );
}

/// ⭐ K3：`count=1` ≡ `loop_mode=one_shot`（逐位相同、写 `9599` 帧）。
#[test]
fn k3_count_1_is_one_shot() {
    let (written, out) = render("count=1", None);
    let (reference_written, reference) = render("loop_mode=one_shot", None);
    assert_eq!(
        written, ONE_PLAY,
        "⭐ `count=1` 播一次（{ONE_PLAY} 帧）；实得 {written} 帧"
    );
    assert_eq!(written, reference_written, "写入帧数必须相同");
    assert_eq!(
        out, reference,
        "⭐ `count=1` ⇒ 与 `loop_mode=one_shot` **逐位相同**"
    );
}

/// ⭐ K4：`count=0` 取 ARIA 口径（＝ 未给出）⇒ `loop_continuous` 照旧无限循环、写满输出。
#[test]
fn k4_count_0_is_absent_and_respects_loop_mode() {
    let (written, _) = render(
        "count=0 loop_mode=loop_continuous loop_start=1000 loop_end=5000",
        None,
    );
    assert_eq!(
        written, OUT,
        "⭐ `count=0`（ARIA 口径 ＝ 未给出）＋ `loop_continuous` ⇒ 必须写满输出；实得 {written} 帧"
    );
}

/// ⭐ K5：`count` 与显式 `loop_mode` / `loop_count` 冲突时 `count` 赢
/// （强制 `one_shot`、窗口恒 `None` ⇒ 写 `28797` 帧，无窗口、无尾巴）。
#[test]
fn k5_count_wins_over_loop_mode_and_loop_count() {
    let (written, out) = render(
        "count=3 loop_mode=loop_continuous loop_start=1000 loop_end=5000 loop_count=7",
        None,
    );
    let expected = 3 * ONE_PLAY;
    assert_eq!(
        written, expected,
        "⭐ `count=3` 赢 ⇒ 整段播 3 次（{expected} 帧），⛔ 不是窗口循环；实得 {written} 帧"
    );
    // 窗口若还生效，输出帧 1000 处会是源 1000（窗口起点）而不是源 1000 的
    // 整段第二遍 —— 整段重播的第二遍首帧在 9599，不在 1000；直接钉边界：
    assert_src(&out, ONE_PLAY, 0, "第二遍首帧（整段重播，⛔ 不是窗口回绕）");
    assert_src(&out, 2 * ONE_PLAY, 0, "第三遍首帧");
}

/// ⭐ K6：`count=3` ＋ note-off（`held_frames=Some(100)`）⇒ 仍播满 3 遍
/// （`one_shot` 忽略 note-off）；对照组（不写 `count`）在释放段停住。
#[test]
fn k6_count_ignores_note_off() {
    let (written, out) = render("count=3", Some(100));
    let expected = 3 * ONE_PLAY;
    assert_eq!(
        written, expected,
        "⭐ `count=3` ＋ note-off ⇒ 仍播满 3 遍（{expected} 帧）；实得 {written} 帧"
    );
    assert_src(&out, 2 * ONE_PLAY, 0, "第三遍首帧（note-off 之后仍重播）");
    // 对照组：不写 `count` ⇒ 第 100 帧进释放段（`ampeg_release` 缺省 0 ⇒
    // 释放段零长 ⇒ 立即退休），写入帧数远小于一遍。
    let (control_written, _) = render("", Some(100));
    assert_eq!(
        control_written, 100,
        "⭐ 对照组（不写 `count`）必须在释放段停住（100 帧）；实得 {control_written} 帧"
    );
}

/// ⭐ K7：非法 `count` 值响亮 `Err`（负数 / 超出 `u32` / 非整数）。
#[test]
fn k7_illegal_count_values_fail_loudly() {
    for bad in ["count=-1", "count=4294967296", "count=1.5", "count=abc"] {
        let text = format!("<region> sample=a.wav key=60 pitch_keycenter=60 {bad}");
        assert!(
            SfzInstrument::load(text, &Default::default()).is_err(),
            "⭐ 非法 `{bad}` 必须响亮 Err（⛔ 不许静默忽略、⛔ 不许当作未给出）"
        );
    }
}

/// ⭐ K8：反向 ＋ `count=2` ⇒ 整段反向播 2 次（`2 × 9599 = 19198` 帧），
/// 两遍都覆盖源 `9599…1`（与 C10 同一条反向起止：含 `PCM-1`、不含源 0）。
#[test]
fn k8_reverse_count_2_plays_backwards_twice() {
    let (written, out) = render("direction=reverse count=2", None);
    let expected = 2 * ONE_PLAY;
    assert_eq!(
        written, expected,
        "⭐ 反向 `count=2` ⇒ 2×{ONE_PLAY} ＝ {expected} 帧；实得 {written} 帧"
    );
    assert_src(&out, 0, PCM - 1, "第一遍首帧（反向起点）");
    assert_src(&out, ONE_PLAY - 1, 1, "第一遍末帧");
    assert_src(&out, ONE_PLAY, PCM - 1, "第二遍首帧（反向重播起点）");
    assert_src(&out, 2 * ONE_PLAY - 1, 1, "第二遍末帧（最后一帧）");
}

/// 摊平一条 SFZ 文本（控制侧；允许分配）。
fn prepare(text: &str, sample: &InMemorySample) -> PreparedSfz {
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    PreparedSfz::prepare(&sfz, sample, HZ).expect("摊平必须成功")
}

/// 池侧渲染：按 `BLOCK` 逐块写满 `OUT` 帧，返回整段输出。
fn render_pool_mono(plan: &PreparedSfz) -> Vec<f32> {
    let mut pool = SfzVoicePool::empty();
    // ⚠ 力度 127（增益恰 1.0，见 `render` 的注释）。
    let created = pool.trigger_note(plan, 60, 127, u64::MAX, 1.0, None);
    assert_eq!(created, 1, "必须建起一个声部");
    let mut out = vec![0.0f32; OUT];
    let mut position = 0u64;
    for chunk in out.chunks_mut(BLOCK) {
        chunk.fill(0.0);
        pool.render_into(plan, position, chunk);
        position += BLOCK as u64;
    }
    assert_eq!(
        pool.active_count(),
        0,
        "⭐ 播完 3 遍后声部必须退休（遍数耗尽 ＋ 样本播完）"
    );
    out
}

/// ⭐ K9：实时池（单声道）与离线路同一份 `count` 语义。
#[test]
fn k9_realtime_pool_honours_count() {
    let plan = prepare(
        "<region> sample=a.wav key=60 pitch_keycenter=60 count=3",
        &ramp(),
    );
    let out = render_pool_mono(&plan);
    let last_nonzero = out.iter().rposition(|v| *v != 0.0).expect("必须出声");
    assert_eq!(
        last_nonzero,
        3 * ONE_PLAY - 1,
        "⭐ 最后一帧非零下标必须是第三遍末帧；实得 {last_nonzero}"
    );
    assert_src(&out, 0, 0, "第一遍首帧");
    assert_src(&out, ONE_PLAY, 0, "第二遍首帧");
    assert_src(&out, 2 * ONE_PLAY, 0, "第三遍首帧");
    assert_src(&out, 3 * ONE_PLAY - 1, PCM - 2, "第三遍末帧");
}

/// ⭐ K10：实时池（立体声双跑）左右声道逐位相同，且 `count` 语义与 K9 一致。
#[test]
fn k10_stereo_pool_left_and_right_agree_on_count() {
    let plan = prepare(
        "<region> sample=a.wav key=60 pitch_keycenter=60 count=3",
        &stereo_ramp(),
    );
    let mut pool = SfzVoicePool::empty();
    // ⚠ 力度 127（增益恰 1.0，见 `render` 的注释）。
    let created = pool.trigger_note(&plan, 60, 127, u64::MAX, 1.0, None);
    assert_eq!(created, 1, "必须建起一个声部");
    let mut left = vec![0.0f32; OUT];
    let mut right = vec![0.0f32; OUT];
    let mut position = 0u64;
    for (lchunk, rchunk) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)) {
        pool.render_into_stereo_with_probes(&plan, position, lchunk, rchunk, None, None);
        position += BLOCK as u64;
    }
    // ⭐ 双跑快照必须带已播遍数：左遍推进的遍数若漏进右遍的起点，两声道会在
    // 重播点分家（右声道提前停播）。`pan=0` ＋ 左右同源 ⇒ 两声道逐位相同。
    assert_eq!(
        left, right,
        "⭐ 立体声双跑的左右声道必须**逐位相同**（快照带已播遍数）"
    );
    let last_nonzero = left.iter().rposition(|v| *v != 0.0).expect("必须出声");
    assert_eq!(
        last_nonzero,
        3 * ONE_PLAY - 1,
        "⭐ 最后一帧非零下标必须是第三遍末帧；实得 {last_nonzero}"
    );
    assert_src(&left, ONE_PLAY, 0, "第二遍首帧");
    assert_src(&left, 2 * ONE_PLAY, 0, "第三遍首帧");
}
