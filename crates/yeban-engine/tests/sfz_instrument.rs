//! SFZ 缺口 1 的**步 2**：引擎侧声部（[`SfzVoicePool`]）＋ 快照形状（[`SfzPlan`]）。
//!
//! 设计裁决（`docs/ledger/phase-status.md` 的 R698「甲-1」）：`TrackSlot` 增
//! `sfz: SfzVoicePool`（`[SfzVoice; 16]`、全 `Copy`）＋ 快照侧只读 [`PreparedSfz`]
//! ＋ `render_track` 加第 3 参 `sfz: Option<&PreparedSfz>`（与既有
//! `Option<&NoteSchedule>` 对称）。分派次序 ＝ **鼓机 ＞ SFZ ＞ 复音**。
//!
//! # 量什么（对象 ＋ 单位，先说再跑）
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | S1 | 引擎声部路径 == 摊平表路径 | **逐样本最大绝对差**（`max_i |flat_i − engine_i|`）＝ **0**，并配"两侧都真的出声"的反向对照 |
//! | S2 | 终点口径 ＝ 样本自然结束 | 播完那一块之后 `sfz_active_voices == 0`（退休）＋ 下一块**逐位静音**，而摊平表**每一块都从源 0 重播**（≠ 0） |
//! | S3 | 显式 `span` 终点 | 引擎在**第 `end` 帧**停、摊平表继续（已登记的口径分叉 sfz-needs-4） |
//! | S4 | `end=-1`（`SampleEnd::Silent`） | 引擎**一帧不写**、池子 0 声部；摊平表仍出声（对照 ⇒ 引擎的静音不是"夹具坏了"） |
//! | S5 | ⭐ **反向对照（关键）** | **不给 `sfz`** 时端到端输出的 **FNV-1a 指纹**（逐位）等于**接线前**实测值 |
//! | S6 | 端到端非静音 ＋ 真的换了音源 | 非零样本数 `> 0` ＋ 指纹与"不给 SFZ"**不同** |
//! | S7 | 分派次序 | 鼓机 ＋ SFZ 同轨的输出对"只有鼓机"**逐位相同**；而无鼓机时 SFZ 支路**真的出声** |
//! | S8 | 池满（> 16 条匹配区域） | `created == 16` ＋ `dropped == 1`（⛔ 不静默） |
//! | S9 | 尺寸 | `size_of` 读数 ＋ `EngineRuntime < 192 KiB`（与 `synth_rt_zero_alloc.rs:2570` 同一门槛） |
//!
//! # S5 的指纹是**接线前**的实测值
//!
//! 三个 u64 由本线在**改动任何源码之前**于 HEAD `3ad1bda7`（`aarch64/macos`、
//! 同一个夹具、同一条产品路径）测得：
//!
//! | 夹具 | 帧数 | 非零样本 | 峰值 | 指纹（FNV-1a 64，逐位） |
//! | :--- | :--- | :--- | :--- | :--- |
//! | 两个音符、无设备链 | 2560 | 5052 | 0.5478619 | `0x71dca4dc55b44df5` |
//! | 两个音符、`cutoff_hz = 800` / `resonance = 0.3` | 2560 | 5052 | 0.32756245 | `0x6eee378072730521` |
//!
//! ⚠ 这两个字面值**不是**规范常数，而是本机 libm/LLVM 的一次实现读数：它们的作用是
//! **跨提交对账**（接线前 vs 接线后同一个夹具给出同一个数），⛔ 不是"跨架构必须相等"
//! （跨架构对账是 `ARCH-DET-002` 的独立票）。形态照
//! `tests/drums_instrument.rs` 的 `check_fingerprint_on`：**同平台硬红、异平台如实跳过
//! 并点名原因**（跳过 **不是** 通过）。

mod support;

use support::{MixSpec, NoteSpec, note_project, render, render_snapshot, tuned_project};
use yeban_engine::drums::DrumsParams;
use yeban_engine::sfz_instrument::{
    InMemorySample, PreparedSfz, SFZ_VOICES_PER_TRACK, SfzInstrument, SfzPlan, SfzVoice,
    SfzVoicePool,
};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_engine::synth::{NoteSchedule, SynthEngine};
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, ParameterValue};

/// S5 夹具：两个音符（1 tick = 25 样本）。⭐ 本夹具与**接线前**测指纹时**一字不差**。
const SFZ_NOTES: [NoteSpec; 2] = [
    NoteSpec::at(0, 480, 60, 100),
    NoteSpec::at(960, 480, 64, 90),
];

/// S6（端到端）夹具：两个起音都落在 20 个量子的窗口内（⇒ 两条区域都会发声）。
const E2E_NOTES: [NoteSpec; 2] = [NoteSpec::at(0, 480, 60, 100), NoteSpec::at(48, 480, 64, 90)];

/// S6 的 SFZ：两条区域各管一个音高（⇒ 两个起音各触发一条）。
const E2E_SFZ: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n\
                       <region> sample=b.wav key=64 pitch_keycenter=60 volume=-6\n";

/// S5 夹具的渲染量子数（20 × 128 = 2560 帧，覆盖两个起音）。
const SFZ_QUANTA: usize = 20;

/// **接线前**实测：无设备链（见模块头）。
const BASELINE_PLAIN: u64 = 8_204_613_887_399_841_269;
/// **接线前**实测：`cutoff_hz = 800` / `resonance = 0.3`（见模块头）。
const BASELINE_TUNED: u64 = 7_993_387_413_444_691_233;

/// 指纹的**测量平台**（`[ARCH-DET-001]` 的 L1 只承诺"相同 OS ＋ CPU 架构 ＋
/// 纯 Rust libm"下的逐位一致）。
const MEASURED_PLATFORM: &str = "aarch64/macos";

/// 本机（**编译期事实**）是否就是那个测量平台。
const ON_MEASURED_PLATFORM: bool = cfg!(target_arch = "aarch64") && cfg!(target_os = "macos");

/// 同平台**硬红** / 异平台**如实跳过并点名原因**（跳过 **不是** 通过）。
///
/// 形态照 `tests/drums_instrument.rs` 的 `check_fingerprint_on`（那条判据的
/// 同一条理由：字面值是某台机器上的实现读数，不是规范常数）。
fn check_fingerprint(label: &str, actual: u64, expected: u64) -> bool {
    if ON_MEASURED_PLATFORM {
        assert_eq!(
            actual, expected,
            "{label}: 同平台（本机 == {MEASURED_PLATFORM}）⇒ 指纹必须逐位等于**接线前**\
             实测值 0x{expected:016x}（实得 0x{actual:016x}）；不同就是真缺陷"
        );
        return true;
    }
    println!(
        "[engine-sfz/S5] SKIP（未判定，不是通过）：{label} 的字面值 0x{expected:016x} \
         实测于 {MEASURED_PLATFORM}；跨架构的位级差异属 L2（max|Δ| < 1e-6），不是本判据的失败"
    );
    false
}

/// 步 1 的"多匹配"夹具：**两条 `key=60` 的区域都匹配**（第二条另有 `volume=-6`），
/// 外加一条 `key=72` 的不命中区域（反向对照）。
const TWO_MATCH_ONE_MISS: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n\
                                  <region> sample=b.wav key=60 pitch_keycenter=60 volume=-6\n\
                                  <region> sample=c.wav key=72 pitch_keycenter=60\n";

/// 摊平用的内存样本长度（帧）：极短 ⇒ 一块 512 帧内就播完。
const SAMPLE_FRAMES: usize = 480;

/// 块长（帧）：**比样本长** ⇒ 一块之内就能观察到"样本自然结束 ⇒ 退休"。
const BLOCK: usize = 512;

/// 摊平一条 SFZ 文本（控制侧；**允许分配**）。
fn prepare(text: &str) -> PreparedSfz {
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let sample = InMemorySample::sine(440.0, 48_000.0, SAMPLE_FRAMES);
    PreparedSfz::prepare(&sfz, &sample, 48_000.0).expect("摊平必须成功")
}

/// 造一台已经武装好 `track` 的合成器（48 kHz）。
fn engine_with_track(track: EntityId) -> SynthEngine {
    let mut engine = SynthEngine::new(48_000);
    engine.begin_snapshot(48_000, &[track], [], []);
    engine
}

/// 两个声道的位模式（"逐位相同"的判定量）。
fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|sample| sample.to_bits()).collect()
}

/// S1：**引擎声部路径与摊平表逐样本差 ＝ 0**，且两侧都真的出声。
///
/// ⚠ 这条 ==0 的**强度**在于接线（触发哪些区域／速率／增益／终点），⛔ 不在于"两段
/// 逐样本循环"：那一段是**同一个函数**（`sfz_instrument::mix_span_into`），
/// 因为本仓的机械纪律是"全仓不许两份实现"（两份在 binary32 下无法维持逐位相同）。
#[test]
fn s1_engine_voices_match_the_flattened_table_sample_by_sample() {
    let prepared = prepare(TWO_MATCH_ONE_MISS);
    assert_eq!(prepared.region_count(), 3, "三条区域都要摊平进来");
    let track = EntityId::new();
    let mut engine = engine_with_track(track);
    // 力度 127（`support::scheduled` 的口径）⇒ 与下面摊平表的 `(60, 127)` 同参数。
    let schedule =
        NoteSchedule::from_sorted(vec![support::scheduled(0, 12_000, 60, 1.0, 48_000.0)]);

    let mut engine_out = vec![0.0f32; BLOCK];
    engine.render_track(track, Some(&schedule), Some(&prepared), &mut engine_out);

    let mut flat = vec![0.0f32; BLOCK];
    let flat_written = prepared.render_block_into(60, 127, &mut flat);

    let max_abs_diff = engine_out
        .iter()
        .zip(flat.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    println!(
        "[engine-sfz/S1] 逐样本最大绝对差 = {max_abs_diff:e}；摊平写入 {flat_written} 帧；\
         引擎非零 = {}；摊平非零 = {}；引擎活声部 = {:?}；丢弃 = {}",
        engine_out.iter().filter(|s| **s != 0.0).count(),
        flat.iter().filter(|s| **s != 0.0).count(),
        engine.sfz_active_voices(track),
        engine.sfz_voice_drops(),
    );
    assert_eq!(
        max_abs_diff, 0.0,
        "⭐ 引擎声部路径必须与 `PreparedSfz::render_block_into` **逐样本相同**\
         （两条 `key=60` 的区域都匹配 ⇒ 两侧都必须叠加两条）"
    );
    // 反向对照：两侧都必须真的出声（⛔ 不能"两边都全零"也算一致）。
    assert!(
        engine_out.iter().any(|s| *s != 0.0),
        "引擎侧必须真的出声（否则上面的等号是空转）"
    );
    assert!(flat.iter().any(|s| *s != 0.0), "摊平侧必须真的出声");
}

/// S2：**终点口径 ＝ 样本自然结束**（⛔ 无包络）。
///
/// 一块 512 帧 > 样本 480 帧 ⇒ 这一块内源采样就播完了：声部**退休**
/// （`active == 0`），下一块**逐位静音**。
///
/// 反向对照（关键）：摊平表的 `render_block_into` 是**无状态**的，它每一块都从源
/// 位置 `0.0` 重播 ⇒ 第二块**不是**静音。这条对照把"第二块静音"从"整条链没接线"
/// 里区分出来。
#[test]
fn s2_the_voice_retires_when_the_sample_runs_out() {
    let prepared = prepare(TWO_MATCH_ONE_MISS);
    let track = EntityId::new();
    let mut engine = engine_with_track(track);
    let schedule =
        NoteSchedule::from_sorted(vec![support::scheduled(0, 12_000, 60, 1.0, 48_000.0)]);

    let mut first = vec![0.0f32; BLOCK];
    engine.render_track(track, Some(&schedule), Some(&prepared), &mut first);
    let after_first = engine.sfz_active_voices(track);

    engine.advance(BLOCK);
    let mut second = vec![0.0f32; BLOCK];
    engine.render_track(track, Some(&schedule), Some(&prepared), &mut second);

    let mut flat_again = vec![0.0f32; BLOCK];
    let flat_written = prepared.render_block_into(60, 127, &mut flat_again);

    println!(
        "[engine-sfz/S2] 第一块非零 = {}；播完后活声部 = {after_first:?}；第二块非零 = {}；\
         摊平表第二块写入 = {flat_written}（非零 = {}）",
        first.iter().filter(|s| **s != 0.0).count(),
        second.iter().filter(|s| **s != 0.0).count(),
        flat_again.iter().filter(|s| **s != 0.0).count(),
    );
    assert!(first.iter().any(|s| *s != 0.0), "第一块必须出声");
    assert_eq!(
        after_first,
        Some(0),
        "⭐ 样本播完 ⇒ 声部必须退休（`active == 0`）；`min(end_sample, 样本播完)` 的\
         诚实占位就是这一条"
    );
    assert!(
        second.iter().all(|s| *s == 0.0),
        "退休之后下一块必须**逐位静音**"
    );
    assert!(
        flat_again.iter().any(|s| *s != 0.0),
        "反向对照：摊平表是无状态的一次性调用 ⇒ 第二块**仍然出声**（否则'第二块静音'\
         可能只是整条链没接线）"
    );
}

/// S3：**显式 `span` 终点**在引擎侧生效（摊平表尚未应用它 ⇒ 已登记的口径分叉）。
///
/// `end=200`（含端点）⇒ 半开区间 `[0, 201)` ⇒ 引擎写到 `floor(pos) + 1 >= 201` 为止
/// ＝ **200 帧**；摊平表继续到样本末尾（478 帧）。
#[test]
fn s3_an_explicit_span_end_stops_the_voice_earlier_than_the_sample_end() {
    let prepared = prepare("<region> sample=a.wav key=60 pitch_keycenter=60 end=200\n");
    let span = prepared.regions()[0].span().expect("span 必须已摊平");
    assert_eq!(
        prepared.regions()[0].source_end(SAMPLE_FRAMES),
        Some(201),
        "`end=200`（含端点）⇒ 半开 [0, 201)"
    );
    assert_eq!(span.end, Some(201), "摊平表的口径（步 1）");

    let track = EntityId::new();
    let mut engine = engine_with_track(track);
    let schedule =
        NoteSchedule::from_sorted(vec![support::scheduled(0, 12_000, 60, 1.0, 48_000.0)]);
    let mut engine_out = vec![0.0f32; BLOCK];
    engine.render_track(track, Some(&schedule), Some(&prepared), &mut engine_out);

    let mut flat = vec![0.0f32; BLOCK];
    prepared.render_block_into(60, 127, &mut flat);

    let engine_written = engine_out.iter().filter(|s| **s != 0.0).count();
    let flat_written = flat.iter().filter(|s| **s != 0.0).count();
    // ⭐ 判定量是**逐样本**而不是"数非零"：正弦在 0 相位处就是 0.0（夹具的第 0 个样本），
    // 数非零会把"写了 200 帧"读成 199。
    let head_diff = engine_out[..200]
        .iter()
        .zip(flat[..200].iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    println!(
        "[engine-sfz/S3] 引擎非零 = {engine_written}；摊平非零 = {flat_written}（479 帧里 478 个非零）；\
         span 内逐样本最大绝对差 = {head_diff:e}"
    );
    assert_eq!(
        head_diff, 0.0,
        "⭐ span 终点**之前**两侧必须逐样本相同（引擎的口径就来自摊平表）"
    );
    assert!(
        engine_out[..200].iter().any(|s| *s != 0.0),
        "引擎在 span 内必须真的出声"
    );
    assert!(
        engine_out[200..].iter().all(|s| *s == 0.0),
        "⭐ 引擎必须在 span 终点（第 200 帧）停：之后**一帧不出**"
    );
    assert!(
        flat[200..].iter().any(|s| *s != 0.0),
        "⚠ 反向对照（**已登记的口径分叉** sfz-needs-4）：摊平表还没应用 span ⇒ 它继续出声"
    );
}

/// S4：`end=-1`（`SampleEnd::Silent`）的区域**不产生采样输出** ⇒ 引擎⛔ 不建声部。
///
/// 反向对照：同一份摊平表用 `render_block_into` 仍然出声（它还没应用 `span`）
/// ⇒ 引擎的静音只能来自"该区域不产生输出"这一条口径，⛔ 不是夹具整体坏了。
#[test]
fn s4_a_silent_span_never_gets_a_voice() {
    let prepared = prepare("<region> sample=a.wav key=60 pitch_keycenter=60 end=-1\n");
    assert_eq!(
        prepared.regions()[0].span(),
        None,
        "`end=-1` ⇒ `SampleEnd::Silent` ⇒ 不产生采样输出"
    );
    assert_eq!(prepared.regions()[0].source_end(SAMPLE_FRAMES), None);

    let track = EntityId::new();
    let mut engine = engine_with_track(track);
    let schedule =
        NoteSchedule::from_sorted(vec![support::scheduled(0, 12_000, 60, 1.0, 48_000.0)]);
    let mut engine_out = vec![0.0f32; BLOCK];
    engine.render_track(track, Some(&schedule), Some(&prepared), &mut engine_out);

    let mut flat = vec![0.0f32; BLOCK];
    prepared.render_block_into(60, 127, &mut flat);

    println!(
        "[engine-sfz/S4] 引擎非零 = {}；活声部 = {:?}；摊平非零 = {}（对照：仍出声）",
        engine_out.iter().filter(|s| **s != 0.0).count(),
        engine.sfz_active_voices(track),
        flat.iter().filter(|s| **s != 0.0).count(),
    );
    assert!(
        engine_out.iter().all(|s| *s == 0.0),
        "⭐ `end=-1` 的区域不得产生任何样本"
    );
    assert_eq!(
        engine.sfz_active_voices(track),
        Some(0),
        "⭐ 一条声部都不该建出来"
    );
    assert!(
        flat.iter().any(|s| *s != 0.0),
        "反向对照：摊平表（尚未应用 span）仍然出声 ⇒ 引擎的静音有解释"
    );
}

/// S5：⭐ **反向对照（关键）**：**不给 `sfz`** 时端到端输出与**接线前**逐位相同。
///
/// 两条臂（无设备链 / 带音色）的指纹都必须等于**改动源码之前**实测的字面值。
/// 同平台硬红、异平台如实跳过（见 [`check_fingerprint`]）。
#[test]
fn s5_without_sfz_the_output_is_bit_identical_to_before_the_wiring() {
    let plain = note_project(&SFZ_NOTES);
    let rendered = render(&plain.project, SFZ_QUANTA);
    println!(
        "[engine-sfz/S5] 无设备链: 帧={} 非零={} 峰值={} 指纹={} (0x{:016x})",
        rendered.frames(),
        rendered.nonzero(),
        rendered.peak(),
        rendered.fingerprint(),
        rendered.fingerprint(),
    );
    assert!(rendered.nonzero() > 0, "夹具必须真的出声（否则等号是空转）");
    let plain_checked = check_fingerprint("无设备链", rendered.fingerprint(), BASELINE_PLAIN);

    let tuned = tuned_project(&SFZ_NOTES, MixSpec::tone(800.0, 0.3));
    let rendered_tuned = render(&tuned.project, SFZ_QUANTA);
    println!(
        "[engine-sfz/S5] 音色(800, 0.3): 帧={} 非零={} 峰值={} 指纹={} (0x{:016x})",
        rendered_tuned.frames(),
        rendered_tuned.nonzero(),
        rendered_tuned.peak(),
        rendered_tuned.fingerprint(),
        rendered_tuned.fingerprint(),
    );
    let tuned_checked = check_fingerprint(
        "音色(800, 0.3)",
        rendered_tuned.fingerprint(),
        BASELINE_TUNED,
    );

    // ⚠ 两条臂必须在**同一平台判定**下：不能一条硬红一条跳过（那说明平台判定漂了）。
    assert_eq!(
        plain_checked, tuned_checked,
        "两条臂的平台判定必须一致（否则本判据的读数不可比）"
    );
}

/// S6：`with_sfz` 是**唯一**的绑定入口（模型⛔ 不给），且端到端真的非静音、
/// 真的换了音源（指纹与"不给 SFZ"不同）。
#[test]
fn s6_the_snapshot_only_carries_the_sfz_a_caller_injected_and_it_is_audible() {
    let fixture = note_project(&E2E_NOTES);
    let track = fixture.track;

    // ① `from_project` **不**绑 SFZ（模型层今天没有轨↔SFZ 绑定）。
    let bare = EngineSnapshot::from_project(&fixture.project, 1).expect("快照");
    assert!(
        bare.sfz().is_empty(),
        "⛔ `from_project` 不得凭空绑一份 SFZ（模型语义不由引擎发明）"
    );
    assert!(bare.sfz_plan(&track).is_none());

    // ② 同一份快照 + 显式注入 ⇒ 表里正好一条，且只有那条轨。
    let plan = SfzPlan::prepare(
        &SfzInstrument::load(E2E_SFZ.to_owned(), &Default::default()).expect("SFZ 必须解析成功"),
        &InMemorySample::sine(440.0, 48_000.0, SAMPLE_FRAMES),
        48_000.0,
    )
    .expect("摊平必须成功");
    println!(
        "[engine-sfz/S6] 摊平表：区域 {}、内存 {} 字节",
        plan.region_count(),
        plan.memory_bytes()
    );
    let armed = EngineSnapshot::from_project(&fixture.project, 1)
        .expect("快照")
        .with_sfz(track, plan);
    assert_eq!(armed.sfz().len(), 1, "注入一次 ⇒ 表里一条");
    assert!(armed.sfz_plan(&track).is_some(), "注入的那条轨必须在表里");
    assert!(
        armed.sfz_plan(&fixture.master).is_none(),
        "⛔ 母线上不该有 SFZ"
    );

    // ③ 端到端（产品路径：快照槽 → 渲染量子 → 声相 → 母线限制器）：
    //    SFZ 臂必须非静音，且指纹与"不给 SFZ"的**不同**（真的换了音源）。
    let with_sfz = render_snapshot(armed, SFZ_QUANTA);
    let without = render(&fixture.project, SFZ_QUANTA);
    println!(
        "[engine-sfz/S6] SFZ 臂: 非零={} 峰值={} 指纹=0x{:016x}；对照臂: 非零={} 峰值={} 指纹=0x{:016x}",
        with_sfz.nonzero(),
        with_sfz.peak(),
        with_sfz.fingerprint(),
        without.nonzero(),
        without.peak(),
        without.fingerprint(),
    );
    assert!(
        with_sfz.nonzero() > 0 && with_sfz.peak() > 0.0,
        "⭐ 端到端必须**非静音**（非零 = {}）",
        with_sfz.nonzero()
    );
    assert_ne!(
        with_sfz.fingerprint(),
        without.fingerprint(),
        "⭐ 注入 SFZ 之后指纹必须与对照臂不同（否则 SFZ 支路根本没接上）"
    );
}

/// S7：分派次序 ＝ **鼓机 ＞ SFZ ＞ 复音**（一条轨至多一件音源）。
///
/// ① 鼓机 ＋ SFZ 同轨 ⇒ 输出对"只有鼓机"**逐位相同**（SFZ 池被释放、⛔ 不叠加）；
/// ② 反向对照：同一份 SFZ 在没有鼓机的轨上**真的出声**（⇒ ① 的等号不是"两边都哑"）。
/// ③ "SFZ ＞ 复音"由 S1 覆盖：那台槽的复音合成器本来会出声，而 S1 的逐样本差为 0
///    ⇒ 复音**没有**被叠加进去。
#[test]
fn s7_drums_win_over_sfz_which_wins_over_polyphony() {
    let prepared = prepare(TWO_MATCH_ONE_MISS);
    let track = EntityId::new();
    // 键位映射里**包含** key 60 ⇒ 同一条音符在鼓机分支里也有鼓件可打。
    let device = DeviceDefinition {
        id: EntityId::new(),
        name: "Yeban Drums".to_owned(),
        kind: DeviceKind::InternalInstrument,
        bypassed: false,
        params: [
            ("kick_note", 60.0),
            ("snare_note", 38.0),
            ("closed_hat_note", 42.0),
            ("open_hat_note", 46.0),
            ("clap_note", 39.0),
        ]
        .iter()
        .map(|(name, value)| ParameterValue {
            name: (*name).to_owned(),
            value: *value,
            unit: None,
        })
        .collect(),
        latency_samples: 0,
    };
    let params = DrumsParams::from_devices(&[device]).expect("五个键位名写全 ⇒ 识别为鼓机");
    let schedule =
        NoteSchedule::from_sorted(vec![support::scheduled(0, 12_000, 60, 1.0, 48_000.0)]);

    let mut with_sfz = SynthEngine::new(48_000);
    let mut only_drums = SynthEngine::new(48_000);
    let mut only_sfz = SynthEngine::new(48_000);
    with_sfz.begin_snapshot(48_000, &[track], [], [(&track, &params)]);
    only_drums.begin_snapshot(48_000, &[track], [], [(&track, &params)]);
    only_sfz.begin_snapshot(48_000, &[track], [], []);

    let mut a = vec![0.0f32; 128];
    let mut b = vec![0.0f32; 128];
    let mut c = vec![0.0f32; 128];
    with_sfz.render_track(track, Some(&schedule), Some(&prepared), &mut a);
    only_drums.render_track(track, Some(&schedule), None, &mut b);
    only_sfz.render_track(track, Some(&schedule), Some(&prepared), &mut c);

    println!(
        "[engine-sfz/S7] 鼓机+SFZ 非零 = {}；只有鼓机 非零 = {}；只有 SFZ 非零 = {}；\
         鼓机臂的 SFZ 活声部 = {:?}",
        a.iter().filter(|s| **s != 0.0).count(),
        b.iter().filter(|s| **s != 0.0).count(),
        c.iter().filter(|s| **s != 0.0).count(),
        with_sfz.sfz_active_voices(track),
    );
    assert!(b.iter().any(|s| *s != 0.0), "鼓机臂必须真的出声");
    assert_eq!(
        bits(&a),
        bits(&b),
        "⭐ 鼓机 ＞ SFZ：同轨两件音源时，输出必须与'只有鼓机'**逐位相同**\
         （⛔ SFZ 不得叠加进去）"
    );
    assert_eq!(
        with_sfz.sfz_active_voices(track),
        Some(0),
        "鼓机分支必须**释放** SFZ 池（⛔ 不留上一件音源的尾巴）"
    );
    assert!(
        c.iter().any(|s| *s != 0.0),
        "反向对照：没有鼓机的同一份 SFZ 必须真的出声（否则上面的等号可能只是两边都哑）"
    );
}

/// S8：池满（> [`SFZ_VOICES_PER_TRACK`] 条区域同时匹配）⇒ **丢弃 ＋ 计数**，
/// ⛔ 不 panic、⛔ 不扩容、⛔ 不发明窃取口径（已登记为 sfz-needs-2）。
#[test]
fn s8_a_full_pool_counts_drops_instead_of_silently_dropping_them() {
    let mut text = String::new();
    for _ in 0..=SFZ_VOICES_PER_TRACK {
        text.push_str("<region> sample=a.wav key=60 pitch_keycenter=60\n");
    }
    let prepared = prepare(&text);
    assert_eq!(
        prepared.region_count(),
        SFZ_VOICES_PER_TRACK + 1,
        "夹具必须是 16 + 1 条**全部匹配**的区域"
    );

    let mut pool = SfzVoicePool::empty();
    let created = pool.trigger_note(&prepared, 60, 100, 12_000);
    println!(
        "[engine-sfz/S8] 建出声部 = {created}；活声部 = {}；丢弃 = {}",
        pool.active_count(),
        pool.dropped(),
    );
    assert_eq!(created, SFZ_VOICES_PER_TRACK, "池只能装 16 个");
    assert_eq!(pool.active_count(), SFZ_VOICES_PER_TRACK);
    assert_eq!(
        pool.dropped(),
        1,
        "⭐ 第 17 条区域必须被**计数**（⛔ 不静默丢弃）"
    );
    // 反向对照：15 条区域时**不**该有丢弃（否则上面那个 1 可能是常数）。
    let mut under = String::new();
    for _ in 0..(SFZ_VOICES_PER_TRACK - 1) {
        under.push_str("<region> sample=a.wav key=60 pitch_keycenter=60\n");
    }
    let under_prepared = prepare(&under);
    let mut under_pool = SfzVoicePool::empty();
    assert_eq!(
        under_pool.trigger_note(&under_prepared, 60, 100, 12_000),
        SFZ_VOICES_PER_TRACK - 1
    );
    assert_eq!(under_pool.dropped(), 0, "未满时不得计数");
}

/// S9：**尺寸照打** ＋ `EngineRuntime` 的 192 KiB 门槛（与
/// `tests/synth_rt_zero_alloc.rs:2570` 同一口径）。
///
/// 定长声部池的代价必须落在尺寸读数上：`SynthEngine` / `EngineRuntime` 都是
/// **按值搬进回调**的对象 ⇒ 这道门槛是"有没有把大件塞进实时对象"的机械防线。
#[test]
fn s9_sizes_are_printed_and_the_runtime_stays_under_192_kib() {
    let voice = core::mem::size_of::<SfzVoice>();
    let pool = core::mem::size_of::<SfzVoicePool>();
    let runtime = core::mem::size_of::<yeban_engine::rt::EngineRuntime>();
    println!(
        "[engine-sfz/S9] size_of: SfzVoice={voice} SfzVoicePool={pool} \
         SfzPlan={} PreparedSfz={} SynthEngine={} EngineSnapshot={} EngineRuntime={runtime} \
         （门槛 192 KiB = {}）",
        core::mem::size_of::<SfzPlan>(),
        core::mem::size_of::<PreparedSfz>(),
        core::mem::size_of::<SynthEngine>(),
        core::mem::size_of::<EngineSnapshot>(),
        192 * 1024,
    );
    // ⚠ 钉住声部的**定长**形状：16 通道 × 32 字节 ＋ 8 字节累计丢弃计数。
    // （`SfzVoice` 只含 `f32`/`u8`/`bool`/`u64`，⛔ 无指针、⛔ 无 `Vec`。）
    assert_eq!(voice, 32, "一个 SFZ 声部 = 32 字节（定长、全 `Copy`）");
    assert_eq!(
        pool,
        SFZ_VOICES_PER_TRACK * 32 + 8,
        "声部池 = 16 × 声部 ＋ 累计丢弃计数"
    );
    assert!(
        runtime < 192 * 1024,
        "`EngineRuntime` = {runtime} 字节 ≥ 192 KiB（{})—— 定长声部池把实时对象撑爆了",
        192 * 1024
    );
}
