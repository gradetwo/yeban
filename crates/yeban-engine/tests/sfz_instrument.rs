//! SFZ 缺口 1 的**步 2／步 3**：引擎侧声部（[`SfzVoicePool`]）＋ 快照形状（[`SfzPlan`]）
//! ＋ ⭐ **多采样 ＋ 绑定注入 API**。
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
//! | S1b | ⭐ **逐块拼接的整音符**（`#66` 补做 2） | 8 × 128 帧拼接 vs 摊平表**一次性 1024 帧**：逐样本最大绝对差 ＝ **0**，且"有声块 ≥ 2"（跨块见证；⛔ 挡住"提前退休"注入） |
//! | S2 | 终点口径 ＝ 样本自然结束 | 播完那一块之后 `sfz_active_voices == 0`（退休）＋ 下一块**逐位静音**，而摊平表**每一块都从源 0 重播**（≠ 0） |
//! | S3 | ⭐ **显式 `span` 终点**（步 3 后两侧同口径） | 引擎与摊平表**逐样本 0**、都写在 `end` 处停（**写入帧数 == 190/200**）；反向对照：改 `end` ⇒ 帧数变 |
//! | S4 | ⭐ `end=-1`（`SampleEnd::Silent`） | 引擎**一帧不写**、池子 0 声部；摊平表**也**一帧不写（两侧同口径）；反向对照：去掉 `end=-1` ⇒ 摊平表出声 |
//! | S5 | ⭐ **反向对照（关键）** | **不给 `sfz`** 时端到端输出的 **FNV-1a 指纹**（逐位）等于**接线前**实测值 |
//! | S6 | 端到端非静音 ＋ 真的换了音源 | 非零样本数 `> 0` ＋ 指纹与"不给 SFZ"**不同** |
//! | S7 | 分派次序 | 鼓机 ＋ SFZ 同轨的输出对"只有鼓机"**逐位相同**；而无鼓机时 SFZ 支路**真的出声** |
//! | S8 | 池满（> 16 条匹配区域） | **窃取**（⛔ 不丢弃）：`created == 16` ＋ `stolen == 1` ＋ `dropped == 0`；33 条 ⇒ 挂起者被换掉 ＝ **真丢弃**（`stolen == 17` ＋ `dropped == 16`）；未满 ⇒ 两个计数都 0 |
//! | S9 | 尺寸 | `size_of` 读数 ＋ `EngineRuntime < 192 KiB`（与 `synth_rt_zero_alloc.rs:2570` 同一门槛） |
//! | S10 | ⭐ **换音源 / 定位必须释放 SFZ 池**（`#66` 补做 2） | `sfz_active_voices`：触发后 **2** → 换音源后/`seek` 后 **0**；换音源那一块**复音非零 > 0**；切回 SFZ 那一块**非零 ＝ 0** |
//! | ⭐ S11 | **多采样各播自己**（步 3） | 两条区域两条不同 `.wav` ⇒ 两条各自与"单独一条 + 它自己那份样本"的**逐位相同**，且两条之间**指纹不同**；反向对照：**对调**提供表 ⇒ 指纹变；广播版在 key=64 上播的是 a 的内容（**内容错**的机械证据） |
//! | ⭐ S12 | **同一样本引两次只解码一次**（步 3） | `sample_count() == 1` ＋ `pcm_bytes() == 1 × 480 × 4`；反向对照：两条**不同**路径 ⇒ `2` / `2 × 1920`；提供表同一路径两份 ⇒ 首个为准（内容仍是第一份） |
//! | ⭐ S13 | **`start`/`end` 真正应用**（步 3） | 离线 vs 摊平**逐样本 0**；`offset=10` ⇒ 输出第 0 帧 **逐位等于** `源[10] × gain`；写入帧数 **190**；反向对照：`offset=50` ⇒ 指纹与波形都变 |
//! | ⭐ S14 | **单样本重载的兼容性**（步 3） | `prepare`（广播）在 span-free 夹具上的 FNV-1a 指纹 ＝ **步 2 实测** `0xa933ddab286bdf5e`；`is_broadcast()` / `sample_content_label() == "broadcast(1)"` |
//! | ⭐ S15 | **缺样本 ⇒ 静音 ＋ 计数**（⭐ 本单改写；归属裁决 `R711` 的 `needs-7`） | 缺 `missing.wav`（提供表 `[a.wav]`）⇒ `silent_region_count() == 1`、`missing_sample_paths() == ["missing.wav"]`、`sample_content = distinct(1)+silent(1)`、`sample_index == NO_SAMPLE`、该键**一帧不写**（⛔ 不再回退到 `samples[0]`）；反向对照①：补上 ⇒ 该键真的出声且**逐位**是自己的内容；反向对照②：key=60 那条区域**照常出声**（静音作用域只到缺样本的区域）；空提供表 ⇒ `silent = 2`、写入 **0** 帧、`missing = ["a.wav","missing.wav"]`（排序去重） |
//! | ⭐ S16 | **绑定注入 API 经公开路径可用（多采样）**（步 3） | `SfzPlan::prepare_with_samples` → `EngineSnapshot::with_sfz` → `render_snapshot`：非零 > 0、指纹 ≠ 不给 SFZ；反向对照：广播 plan 的端到端指纹与多采样版**不同** |
//! | ⭐ S17 | ⭐ **轨推子变化 N dB ⇒ SFZ 与鼓机输出同比例变**（本单新增；`needs-1`） | 两臂（同一份工程、只差音源）`peak(−6 dB)/peak(0 dB)` 都等于 `track_gain(−6.0)`（相对误差 ≤ 1e-5）且**互相**相等；`mute = true` ⇒ 两臂**逐位** 0；另一条轨独奏 ⇒ 两臂**逐位** 0。⚠ 量的是 `render_track` 输出（母线限制器前）|
//! | ⭐ S18 | ⭐ **引擎层⛔ 不乘第二次力度**（本单新增；`needs-1` 的防双重计腿） | `v ∈ {0,32,64,100,127}`：引擎声部路径与**乐器层**一次性渲染**逐位相同**（手工调度表的 `gain` 显式写成 `velocity_gain(v)`）；反向对照：`velocity_gain(127) == 1.0`（恒等）⇒ S1 的 127 那条腿**抓不住**双重计 |
//! | ⭐ S19 | ⭐ **池满 ⇒ 窃取 ＋ 3 ms 淡出**（本单新增；`sfz-needs-2`） | 池满后再触发：`stolen == 1`、`dropped == 0`、`fade_remaining == 144`（3 ms @ 48 kHz）；淡出窗口里槽位仍宣称**被窃取者**、走完 144 帧后就地换成新声部（`note` 变）；输出读数：淡出臂的第 143 帧 / 第 0 帧 ＝ `FLOOR^(143/144)`；**硬窃取对照臂**（`steal_fade_frames = 0`）⇒ 即时换人、指纹不同 |
//! | ⭐ S20 | ⭐ **三件同轨的可读信号**（本单新增；`sfz-needs-3`） | 鼓机 ＋ SFZ ＋ 复音同轨 ⇒ `armed_sources == 3` ＋ `ignored_sources == 2`（输出仍对"只有鼓机"**逐位相同**）；反向对照：鼓机＋复音 ⇒ `2 / 1`、只有复音 ⇒ `1 / 0` |
//! | ⭐ S21 | ⭐ **反向对照（任务单 `#78` 的核心）** | `TrackV3.sampler = None` ⇒ 端到端 **FNV-1a 指纹**仍逐位等于**接线前** [`BASELINE_PLAIN`]／[`BASELINE_TUNED`]，且 `sfz_pending()` **空** |
//! | ⭐ S22 | ⭐ **正向（`#78`）** | `sampler = Some(路径)` ⇒ `from_project` 的快照里出现**该轨**的待摊平项 `PendingSfz`（路径**逐字**相同、条数 1）＋ ⛔ **摊平表 `sfz()` 仍空**（引擎⛔ 不凭空摊平） |
//! | ⭐ S23 | ⭐ **待摊平项是纯事实（`#78`）** | 路径**磁盘上不存在**也不报错、⛔ 不改渲染（同夹具 `None`／`Some` 指纹**逐位相同**）；`with_sfz` 注入摊平表 ⇒ 待摊平项被**取代**（表清空、`sfz()` 1 条、指纹变） |
//! | ⭐ S24 | ⭐ **缺样本口径不变，且经新接线可达（`#78`）** | 由 `sfz_pending()` 给出的路径 → 控制侧解码（**只给 `a.wav`**）⇒ `silent_region_count() == 1`、`missing_sample_paths() == ["missing.wav"]`、`sample_index == NO_SAMPLE`、`render_track` 该键**不建声部**（`sfz_active_voices == 0`、逐位 0） |
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
    InMemorySample, PendingSfz, PreparedRegion, PreparedSfz, SFZ_VOICES_PER_TRACK, SfzInstrument,
    SfzPlan, SfzVoice, SfzVoicePool,
};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_engine::synth::{NoteSchedule, ScheduledNote, SynthEngine, track_gain, velocity_gain};
use yeban_model::{
    DeviceDefinition, DeviceKind, EntityId, ParameterValue, SamplerBinding, TrackKind, TrackV3,
    YebanProjectV1,
};

/// ⭐ 步 2 **实测**（本机 `aarch64/macos`、HEAD `6ae70ef8` 的 `prepare` 广播路径）：
/// span-free 夹具 [`TWO_MATCH_ONE_MISS`] 在 `(60, 127)`、512 帧上的 FNV-1a 指纹。
/// ⚠ 它的作用是**跨提交对账**（步 2 vs 步 3 的同一条广播路径逐位不变），⛔ 不是规范常数。
const BROADCAST_STEP2_FINGERPRINT: u64 = 0xa933_ddab_286b_df5e;

/// FNV-1a 64（逐位；把每个 `f32` 的 4 个字节按小端卷进哈希）。
fn fnv1a_64(samples: &[f32]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for sample in samples {
        for byte in sample.to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

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

/// ⭐ S1 第二条腿用的**量子长**（帧；= `block::DEFAULT_BLOCK_FRAMES`）。
const QUANTUM: usize = 128;

/// ⭐ S1 第二条腿用的**量子数**：`8 × 128 = 1024` 帧 > 样本 480 帧 ⇒ 真的跨块。
const MULTI_BLOCKS: usize = 8;

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

    // --- ⭐ S1 第二条腿（任务单 `#66` 补做 2）：**逐块拼接的整音符** ---
    //
    // 为什么必须补（注入实测）：上面那条腿只跨**一块**（512 帧 ≥ 样本 480 帧）
    // ⇒ `written == remaining` 永不发生 ⇒ "声部是否跨量子延续"**没有被量到**。
    // 实测：把 `written < remaining` 改成 `<=`（提前退休）时上一条腿**逐位不变**，
    // 只有把尺度**跨过"样本长"**才看得见 —— 那种注入会让输出腰斩
    // （端到端非零 1716 → 508、㉙ 总能量 2999.164 → 780.073）而**无人知**。
    //
    // 判定量：8 个 128 帧量子（= 1024 帧）拼起来的整段，与摊平表**一次性 1024 帧**
    // 调用的**逐样本最大绝对差**；外加"真的有 ≥ 2 个块出声"（跨块见证，⛔ 防空转）。
    let mut engine_multi = engine_with_track(track);
    let mut joined = vec![0.0f32; QUANTUM * MULTI_BLOCKS];
    let mut blocks_with_sound = 0usize;
    for block in 0..MULTI_BLOCKS {
        let mut out = vec![0.0f32; QUANTUM];
        engine_multi.render_track(track, Some(&schedule), Some(&prepared), &mut out);
        if out.iter().any(|s| *s != 0.0) {
            blocks_with_sound += 1;
        }
        joined[block * QUANTUM..(block + 1) * QUANTUM].copy_from_slice(&out);
        engine_multi.advance(QUANTUM);
    }
    let mut flat_joined = vec![0.0f32; QUANTUM * MULTI_BLOCKS];
    let flat_joined_written = prepared.render_block_into(60, 127, &mut flat_joined);
    let joined_diff = joined
        .iter()
        .zip(flat_joined.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    println!(
        "[engine-sfz/S1b] 逐块拼接（{MULTI_BLOCKS} × {QUANTUM} = {} 帧）：逐样本最大绝对差 = \
         {joined_diff:e}；有声块 = {blocks_with_sound}；摊平一次性写入 {flat_joined_written} 帧；\
         引擎非零 = {}；摊平非零 = {}",
        QUANTUM * MULTI_BLOCKS,
        joined.iter().filter(|s| **s != 0.0).count(),
        flat_joined.iter().filter(|s| **s != 0.0).count(),
    );
    assert_eq!(
        joined_diff, 0.0,
        "⭐ 逐块拼接的整音符必须与摊平表的一次性调用**逐样本相同**\
         （声部跨量子延续、且样本只播**一遍**）"
    );
    assert!(
        blocks_with_sound >= 2,
        "⭐ 夹具必须真的**跨块**（有声块 = {blocks_with_sound}）—— 否则这条腿量不到'跨量子延续'\
         （进程内自检：注入'提前退休'时必须在这里变红）"
    );
    assert!(
        flat_joined_written > 0,
        "摊平表的整段调用必须真的写入（否则等号是空转）"
    );
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

/// S3：⭐ **显式 `span` 终点两侧同口径**（步 3 把离线/摊平侧补齐 ⇒ sfz-needs-4 **闭合**）。
///
/// `end=200`（含端点）⇒ 半开区间 `[0, 201)` ⇒ 三条路都写到 `floor(pos) + 1 >= 201`
/// 为止 ＝ **200 帧**；摊平表**不再**继续到样本末尾（步 2 时它继续 ⇒ 那是已登记的口径分叉）。
///
/// 反向对照：把 `end` 改成 `400` ⇒ **写入帧数变**（证明不是"恒定截断在 200"这种假口径）。
#[test]
fn s3_an_explicit_span_end_stops_the_voice_at_the_same_frame_on_both_sides() {
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
    let flat_frames = prepared.render_block_into(60, 127, &mut flat);

    let engine_written = engine_out.iter().filter(|s| **s != 0.0).count();
    let flat_written = flat.iter().filter(|s| **s != 0.0).count();
    // ⭐ 判定量是**逐样本**而不是"数非零"：正弦在 0 相位处就是 0.0（夹具的第 0 个样本），
    // 数非零会把"写了 200 帧"读成 199。
    let max_abs_diff = engine_out
        .iter()
        .zip(flat.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    println!(
        "[engine-sfz/S3] 引擎非零 = {engine_written}；摊平非零 = {flat_written}；\
         摊平写入帧数 = {flat_frames}；整块逐样本最大绝对差 = {max_abs_diff:e}"
    );
    assert_eq!(
        max_abs_diff, 0.0,
        "⭐ 步 3 之后：`span` 起点/终点两侧**逐样本相同**（不再是'只覆盖触发量子'）"
    );
    assert_eq!(
        flat_frames, 200,
        "⭐ `end=200`（半开 [0,201)）⇒ 摊平表必须也停在 **200** 帧（线性插值要两个样本）"
    );
    assert!(
        engine_out[..200].iter().any(|s| *s != 0.0),
        "两侧在 span 内必须真的出声"
    );
    assert!(
        engine_out[200..].iter().all(|s| *s == 0.0),
        "⭐ 引擎必须在 span 终点（第 200 帧）停：之后**一帧不出**"
    );
    assert!(
        flat[200..].iter().all(|s| *s == 0.0),
        "⭐ 摊平表**也**必须停在同一帧（步 2 时它继续出声 ⇒ 口径分叉已闭合）"
    );

    // --- 反向对照：改 `end` ⇒ 帧数必须跟着变（⛔ 挡"恒定截断"的假口径） ---
    let longer = prepare("<region> sample=a.wav key=60 pitch_keycenter=60 end=400\n");
    let mut longer_out = vec![0.0f32; BLOCK];
    let longer_frames = longer.render_block_into(60, 127, &mut longer_out);
    println!("[engine-sfz/S3] 反向对照 `end=400`：摊平写入帧数 = {longer_frames}");
    assert_eq!(longer_frames, 400, "`end=400` ⇒ 写 400 帧（半开 [0,401)）");
    assert_ne!(
        longer_frames, flat_frames,
        "⭐ 两个不同的 `end` 必须给出不同的写入帧数（否则上面的 200 可能是常数）"
    );
}

/// S4：⭐ `end=-1`（`SampleEnd::Silent`）的区域**不产生采样输出**（三条路同口径）。
///
/// 引擎⛔ 不建声部、一帧不写；⭐ **摊平表也一帧不写**（步 3 补齐；步 2 时它仍出声 —— 那是
/// 已登记的口径分叉）。
///
/// 反向对照（关键）：去掉 `end=-1` 之后**同一份夹具**必须出声 ⇒ 上面的静音不是"夹具坏了"。
#[test]
fn s4_a_silent_span_never_gets_a_voice_nor_any_sample() {
    let prepared = prepare("<region> sample=a.wav key=60 pitch_keycenter=60 end=-1\n");
    assert_eq!(
        prepared.regions()[0].span(),
        None,
        "`end=-1` ⇒ `SampleEnd::Silent` ⇒ 不产生采样输出"
    );
    assert_eq!(prepared.regions()[0].source_end(SAMPLE_FRAMES), None);
    assert_eq!(prepared.regions()[0].source_start(SAMPLE_FRAMES), None);

    let track = EntityId::new();
    let mut engine = engine_with_track(track);
    let schedule =
        NoteSchedule::from_sorted(vec![support::scheduled(0, 12_000, 60, 1.0, 48_000.0)]);
    let mut engine_out = vec![0.0f32; BLOCK];
    engine.render_track(track, Some(&schedule), Some(&prepared), &mut engine_out);

    let mut flat = vec![0.0f32; BLOCK];
    let flat_frames = prepared.render_block_into(60, 127, &mut flat);

    println!(
        "[engine-sfz/S4] 引擎非零 = {}；活声部 = {:?}；摊平非零 = {}；摊平写入 = {flat_frames}",
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
        flat.iter().all(|s| *s == 0.0),
        "⭐ 摊平表**也**必须静音（步 3：三条路共用 `reduce_span` 的 `None` 口径）"
    );
    assert_eq!(flat_frames, 0, "摊平表写入帧数必须是 0");

    // --- 反向对照：去掉 `end=-1` ⇒ 同一夹具必须出声 ---
    let normal = prepare("<region> sample=a.wav key=60 pitch_keycenter=60\n");
    let mut normal_out = vec![0.0f32; BLOCK];
    let normal_frames = normal.render_block_into(60, 127, &mut normal_out);
    println!("[engine-sfz/S4] 反向对照（无 `end=-1`）：摊平写入帧数 = {normal_frames}");
    assert!(
        normal_frames > 0 && normal_out.iter().any(|s| *s != 0.0),
        "⭐ 反向对照必须出声（否则上面的静音可能只是夹具坏了）"
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

/// S8：池满（> [`SFZ_VOICES_PER_TRACK`] 条区域同时匹配）⇒ ⭐ **窃取**（⛔ 不是丢弃）。
///
/// # 判定量（`sfz-needs-2` 的落地）
///
/// * 17 条**全部匹配** ⇒ `created == 16`（窃取占的是**别人**的槽，⛔ 不计入"新建"）、
///   `stolen == 1`、`dropped == 0`（⭐ 池满不再是丢弃）；
/// * 33 条 ⇒ 挂起者还没轮到发声就被**再一次**换掉 ⇒ 那是**真丢弃**：
///   `stolen == 17` ＋ `dropped == 16`（⛔ 不静默 —— 这条腿保住"丢弃计数"的语义）；
/// * 反向对照：15 条（未满）⇒ `stolen == 0` ＋ `dropped == 0`。
///
/// ⚠ 本判据的**旧期望**是 `dropped == 1`（丢弃口径）；本单把它改成窃取口径
/// —— 注入"改回丢弃"时它当场变红（读数见提交报告）。
#[test]
fn s8_a_full_pool_steals_instead_of_silently_dropping_the_overflow() {
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
    // ⭐ 轨增益门 = 1.0（单位）：本判据量的是**池容量**，⛔ 不掺轨增益。
    let created = pool.trigger_note(&prepared, 60, 100, 12_000, 1.0);
    println!(
        "[engine-sfz/S8] 17 条：建出声部 = {created}；活声部 = {}；窃取 = {}；真丢弃 = {}；挂起 = {}",
        pool.active_count(),
        pool.stolen(),
        pool.dropped(),
        pool.pending_count(),
    );
    assert_eq!(created, SFZ_VOICES_PER_TRACK, "池只能装 16 个空槽");
    assert_eq!(pool.active_count(), SFZ_VOICES_PER_TRACK);
    assert_eq!(
        pool.stolen(),
        1,
        "⭐ 第 17 条区域必须**窃取**一个槽（⛔ 不是丢弃）"
    );
    assert_eq!(pool.dropped(), 0, "⭐ 窃取**不是**丢弃");
    assert_eq!(pool.pending_count(), 1, "⭐ 新声部挂在 victim 的槽里等接管");

    // ⭐ 33 条：挂起者被**再一次**窃取换掉 ⇒ 真丢弃（计数仍在，⛔ 不静默）。
    let mut many = String::new();
    for _ in 0..(2 * SFZ_VOICES_PER_TRACK + 1) {
        many.push_str("<region> sample=a.wav key=60 pitch_keycenter=60\n");
    }
    let many_prepared = prepare(&many);
    let mut many_pool = SfzVoicePool::empty();
    let many_created = many_pool.trigger_note(&many_prepared, 60, 100, 12_000, 1.0);
    println!(
        "[engine-sfz/S8] 33 条：建出声部 = {many_created}；窃取 = {}；真丢弃 = {}；挂起 = {}",
        many_pool.stolen(),
        many_pool.dropped(),
        many_pool.pending_count(),
    );
    assert_eq!(many_created, SFZ_VOICES_PER_TRACK);
    assert_eq!(many_pool.active_count(), SFZ_VOICES_PER_TRACK);
    assert_eq!(
        many_pool.stolen(),
        17,
        "⭐ 33 条 ⇒ 17 次窃取（第 17..33 条各一次）"
    );
    assert_eq!(
        many_pool.dropped(),
        16,
        "⭐ 被换掉的挂起者（第 17..32 条）是真丢弃 ⇒ 必须计数（⛔ 不静默）"
    );

    // 反向对照：15 条区域时**不**该有窃取或丢弃（否则上面那些数可能是常数）。
    let mut under = String::new();
    for _ in 0..(SFZ_VOICES_PER_TRACK - 1) {
        under.push_str("<region> sample=a.wav key=60 pitch_keycenter=60\n");
    }
    let under_prepared = prepare(&under);
    let mut under_pool = SfzVoicePool::empty();
    assert_eq!(
        under_pool.trigger_note(&under_prepared, 60, 100, 12_000, 1.0),
        SFZ_VOICES_PER_TRACK - 1
    );
    assert_eq!(under_pool.dropped(), 0, "未满时不得计数");
    assert_eq!(under_pool.stolen(), 0, "未满时不得窃取");
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
    // ⚠ 钉住声部的**定长**形状：`SfzVoice` 只含 `f32`/`u8`/`bool`/`u16`/`u64`，
    // ⛔ 无指针、⛔ 无 `Vec`。
    // ⭐ 步 3 的代价：`sample_index: u16` 进了声部 ⇒ 32 B → **40 B**（对齐到 8）。
    // ⭐ **本单（`sfz-needs-2`）的代价**：池里多了 16 槽 ×（挂起声部 40 ＋ 淡出 16 ＋
    // 触发序号 8）＋ 挂起标志 16 ＋ 两个计数器（丢弃/窃取/序号舵）⇒ 池 648 → **1712**、
    // `SynthEngine` 91984 → **109152**、`EngineRuntime` 162832 → **180000**
    // （仍 < 192 KiB，余量 16608 字节）。
    assert_eq!(voice, 40, "一个 SFZ 声部 = 40 字节（定长、全 `Copy`）");
    assert_eq!(
        pool, 1712,
        "声部池 = 16 槽 ×（声部 40 ＋ 挂起 40 ＋ 淡出 16 ＋ 序号 8）＋ 挂起标志 16 \
         ＋ 计数器 28（序号舵 8 ＋ 丢弃 8 ＋ 窃取 8 ＋ 淡出帧数 4）⇒ 对齐到 1712"
    );
    assert!(
        runtime < 192 * 1024,
        "`EngineRuntime` = {runtime} 字节 ≥ 192 KiB（{})—— 定长声部池把实时对象撑爆了",
        192 * 1024
    );
}

/// S10（任务单 `#66` **补做 2**）：**换音源 / 定位必须释放 SFZ 池**。
///
/// # 为什么补它（注入实测的缺口，逐字读数见补做报告）
///
/// `slot.sfz.silence()` 的**三处调用**（`synth.rs:1483` 的 `seek`／`:1831` 的鼓机分支／
/// `:1841` 的复音分支）在 S1~S9 里**零覆盖**：把三处全删掉之后，`S1~S9` 与
/// `rt_zero_alloc` ㉙ **全绿**（9 passed / 73 通过），而**临时探针**量到注入确实活着 ——
/// 换音源后 `sfz_active_voices` 仍是 `Some(1)`、`seek` 后仍是 `Some(1)`，
/// 且**切回 SFZ 时陈旧声部从旧源位置复活、写出 128 个非零样本**（还原态三处全是
/// `Some(0)` / `0`）。那是**用户可见的错**：换掉音源（或定位）之后仍听到上一件音源的尾巴。
///
/// # 判定量
///
/// * `SynthEngine::sfz_active_voices`（声部数）：触发后必须 **2**（前置见证）、
///   换音源后与 `seek` 后必须 **0**；
/// * 换音源那一个量子的**复音非零样本数** > 0（证明那个量子**真的**走了复音分支，
///   ⛔ 不是"整条链没跑"）；
/// * 切回 SFZ 那一个量子的**非零样本数** ＝ **0**（逐位静音，⛔ 不得复活）。
#[test]
fn s10_changing_the_source_or_seeking_releases_the_sfz_pool() {
    let prepared = prepare(TWO_MATCH_ONE_MISS);
    let track = EntityId::new();
    // 音符 A：key 60（两条区域都匹配）⇒ 第一量子建 **2** 个 SFZ 声部；
    // 音符 B：在**第二量子**起音、音高 67（三条区域都不匹配）⇒ 换音源那一个量子由
    // **复音**发声 ⇒ 那个量子是"真的走了复音分支"的见证。
    let schedule = NoteSchedule::from_sorted(vec![
        support::scheduled(0, 12_000, 60, 1.0, 48_000.0),
        support::scheduled(QUANTUM as u64, 3_000, 67, 1.0, 48_000.0),
    ]);

    // --- 臂 A：换音源（`sfz = None` ⇒ 复音分支）必须释放池 ---
    let mut engine = engine_with_track(track);
    let mut out = vec![0.0f32; QUANTUM];
    engine.render_track(track, Some(&schedule), Some(&prepared), &mut out);
    let after_sfz = engine.sfz_active_voices(track);
    engine.advance(QUANTUM);
    engine.render_track(track, Some(&schedule), None, &mut out);
    let poly_nonzero = out.iter().filter(|s| **s != 0.0).count();
    let after_swap = engine.sfz_active_voices(track);
    engine.advance(QUANTUM);
    let mut back = vec![0.0f32; QUANTUM];
    engine.render_track(track, Some(&schedule), Some(&prepared), &mut back);
    let back_nonzero = back.iter().filter(|s| **s != 0.0).count();

    // --- 臂 B：`seek`（定位）也必须释放池 ---
    let mut engine_seek = engine_with_track(track);
    engine_seek.render_track(track, Some(&schedule), Some(&prepared), &mut out);
    let before_seek = engine_seek.sfz_active_voices(track);
    engine_seek.seek(0);
    let after_seek = engine_seek.sfz_active_voices(track);

    println!(
        "[engine-sfz/S10] 触发后活声部 = {after_sfz:?}；换音源那一块复音非零 = {poly_nonzero}；\
         换音源后活声部 = {after_swap:?}；切回 SFZ 的非零 = {back_nonzero}；\
         seek 前 = {before_seek:?}、seek 后 = {after_seek:?}"
    );
    assert_eq!(
        after_sfz,
        Some(2),
        "⭐ 前置见证：两条 `key=60` 的区域都匹配 ⇒ 触发后必须有 **2** 个声部在飞\
         （否则下面的 `Some(0)` 是空转）"
    );
    assert!(
        poly_nonzero > 0,
        "⭐ 换音源那一个量子必须由**复音**发声（非零 = {poly_nonzero}）—— 这是'真的换了音源'的见证"
    );
    assert_eq!(
        after_swap,
        Some(0),
        "⭐ 换音源（`sfz = None`）⇒ SFZ 池必须被**释放**（`slot.sfz.silence()`）"
    );
    assert_eq!(
        back_nonzero, 0,
        "⭐ 切回 SFZ 时陈旧声部⛔ 不得复活（该量子必须逐位静音）—— 注入实测：不释放时会\
         从旧源位置写出 128 个非零样本"
    );
    assert_eq!(
        before_seek,
        Some(2),
        "⭐ 前置见证：`seek` 前必须也有 2 个声部在飞"
    );
    assert_eq!(
        after_seek,
        Some(0),
        "⭐ `seek`（定位）⇒ SFZ 池必须被释放（⛔ 不留上一段采样）"
    );
}

/// 用**一条区域**的 SFZ 离线渲染 `note`（参照臂：证明"某区域 + 某份样本"该出什么）。
///
/// ⚠ 它**解析**（⇒ 分配）⇒ 只用于判据的离线参照臂（与 `PreparedSfz` 的构造期同一条边）。
fn offline_one(path: &str, key: u8, sample: &InMemorySample, note: u8, out: &mut [f32]) -> usize {
    let sfz = SfzInstrument::load(
        format!("<region> sample={path} key={key} pitch_keycenter=60\n"),
        &Default::default(),
    )
    .expect("单区域 SFZ 必须解析成功");
    sfz.render_note_into(sample, note, 127, out, 48_000.0)
        .expect("离线渲染不得报错")
}

/// S11（步 3）：⭐ **多采样 —— 两条区域各播自己那份内容**。
///
/// # 判定量
///
/// * 每条区域的摊平输出与"**单独一条 SFZ ＋ 它自己那份样本**"的离线渲染**逐位相同**；
/// * 两条之间**指纹不同**（⛔ 不是只断言"非静音"）；
/// * 反向对照 1：把提供表里两份样本**对调** ⇒ 指纹变（证明真的按 `sample_path` 取内容）；
/// * 反向对照 2：**广播**重载（`prepare`）在 key=64 上播的是 `a.wav` 的内容
///   （与"`a.wav` 的 key=64 离线渲染"逐位相同）⇒ 这就是 `sample_content=broadcast(1)` 的
///   **音色内容错**的机械证据。
///
/// ⚠ 本判据里的 "WAV" 是**已解码**的 [`InMemorySample`]（两条不同频率的正弦 ⇒ 内容不同）：
/// 真读 PCM WAV 文件的那一步在控制侧（`crates/yeban-app/src/sfz_load.rs`），⛔ 引擎不做 I/O。
#[test]
fn s11_two_regions_with_two_samples_each_play_their_own_content() {
    const TWO_KEYS: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n\
                            <region> sample=b.wav key=64 pitch_keycenter=60\n";
    let sfz = SfzInstrument::load(TWO_KEYS.to_owned(), &Default::default()).expect("解析必须成功");
    let a = InMemorySample::sine(440.0, 48_000.0, SAMPLE_FRAMES);
    let b = InMemorySample::sine(660.0, 48_000.0, SAMPLE_FRAMES);
    let provided = vec![
        ("a.wav".to_owned(), a.clone()),
        ("b.wav".to_owned(), b.clone()),
    ];
    let prepared =
        PreparedSfz::prepare_with_samples(&sfz, &provided, 48_000.0).expect("多采样摊平必须成功");

    assert_eq!(prepared.region_count(), 2, "两条区域都要摊平进来");
    assert!(!prepared.is_broadcast(), "多采样路径⛔ 不是广播");
    assert_eq!(prepared.sample_count(), 2, "两条不同路径 ⇒ 两份样本");
    assert_eq!(prepared.regions()[0].sample_index(), 0, "`a.wav` ⇒ 第 0 份");
    assert_eq!(prepared.regions()[1].sample_index(), 1, "`b.wav` ⇒ 第 1 份");

    // ① 各播自己：摊平输出 == "单独一条 + 它自己那份样本"的离线渲染（逐位）
    let mut flat60 = vec![0.0f32; BLOCK];
    prepared.render_block_into(60, 127, &mut flat60);
    let mut ref60 = vec![0.0f32; BLOCK];
    let ref60_frames = offline_one("a.wav", 60, &a, 60, &mut ref60);

    let mut flat64 = vec![0.0f32; BLOCK];
    prepared.render_block_into(64, 127, &mut flat64);
    let mut ref64 = vec![0.0f32; BLOCK];
    let ref64_frames = offline_one("b.wav", 64, &b, 64, &mut ref64);

    println!(
        "[engine-sfz/S11] key=60: 摊平指纹 = 0x{:016x}／离线(a) 指纹 = 0x{:016x}（帧 {ref60_frames}）；\
         key=64: 摊平指纹 = 0x{:016x}／离线(b) 指纹 = 0x{:016x}（帧 {ref64_frames}）",
        fnv1a_64(&flat60),
        fnv1a_64(&ref60),
        fnv1a_64(&flat64),
        fnv1a_64(&ref64),
    );
    assert_eq!(
        bits(&flat60),
        bits(&ref60),
        "⭐ `key=60` 的区域必须播 `a.wav` 那份内容（与离线参照**逐位相同**）"
    );
    assert_eq!(
        bits(&flat64),
        bits(&ref64),
        "⭐ `key=64` 的区域必须播 `b.wav` 那份内容（与离线参照**逐位相同**）"
    );
    assert!(flat60.iter().any(|s| *s != 0.0), "key=60 必须真的出声");
    assert!(flat64.iter().any(|s| *s != 0.0), "key=64 必须真的出声");
    assert_ne!(
        fnv1a_64(&flat60),
        fnv1a_64(&flat64),
        "⭐ 两份不同内容 ⇒ 两条区域的指纹必须**不同**（⛔ 不是只断言'非静音'）"
    );

    // ② 反向对照 1：对调提供表 ⇒ 内容跟着换（指纹变），且等于对调后的离线参照
    let swapped = vec![
        ("a.wav".to_owned(), b.clone()),
        ("b.wav".to_owned(), a.clone()),
    ];
    let swapped_plan = PreparedSfz::prepare_with_samples(&sfz, &swapped, 48_000.0).expect("摊平");
    let mut swapped60 = vec![0.0f32; BLOCK];
    swapped_plan.render_block_into(60, 127, &mut swapped60);
    let mut ref_swapped60 = vec![0.0f32; BLOCK];
    offline_one("a.wav", 60, &b, 60, &mut ref_swapped60);
    println!(
        "[engine-sfz/S11] 对调提供表：key=60 指纹 = 0x{:016x}（原 = 0x{:016x}）",
        fnv1a_64(&swapped60),
        fnv1a_64(&flat60),
    );
    assert_eq!(
        bits(&swapped60),
        bits(&ref_swapped60),
        "对调之后 `a.wav` 那份槽位里放的是 `b` 的内容 ⇒ 输出必须跟着换"
    );
    assert_ne!(
        fnv1a_64(&swapped60),
        fnv1a_64(&flat60),
        "⭐ 对调提供表必须改变输出（否则'各播自己'可能是空转）"
    );

    // ③ 反向对照 2：**广播**重载在 key=64 上播的是 `a.wav` 的内容（内容错的机械证据）
    let broadcast = PreparedSfz::prepare(&sfz, &a, 48_000.0).expect("广播摊平");
    assert!(broadcast.is_broadcast());
    assert_eq!(broadcast.sample_content_label(), "broadcast(1)");
    assert_eq!(
        broadcast.regions()[1].sample_index(),
        0,
        "广播 ⇒ 全指第 0 份"
    );
    let mut bcast64 = vec![0.0f32; BLOCK];
    broadcast.render_block_into(64, 127, &mut bcast64);
    let mut ref_a_at_64 = vec![0.0f32; BLOCK];
    offline_one("a.wav", 64, &a, 64, &mut ref_a_at_64);
    println!(
        "[engine-sfz/S11] 广播在 key=64：指纹 = 0x{:016x}（多采样 = 0x{:016x}）",
        fnv1a_64(&bcast64),
        fnv1a_64(&flat64),
    );
    assert_eq!(
        bits(&bcast64),
        bits(&ref_a_at_64),
        "⚠ 广播路径在 key=64 上播的是 `a.wav`（**音色内容错**）—— 这正是已登记的局限"
    );
    assert_ne!(
        fnv1a_64(&bcast64),
        fnv1a_64(&flat64),
        "⭐ 广播与多采样在 key=64 上必须**不同**（否则多采样没生效）"
    );
}

/// S12（步 3）：⭐ **同一样本引两次只 materialize／解码一次**。
///
/// # 判定量
///
/// * `sample_count()`（＝摊平期的样本 clone 次数，"解码次数"的机械读数）与
///   `pcm_bytes()`（＝ PCM 字节数）；
/// * 反向对照：两条**不同**路径 ⇒ 计数与字节都翻倍；
/// * 提供表里同一路径出现**两份** ⇒ 首个为准（内容仍是第一份）。
#[test]
fn s12_the_same_sample_referenced_twice_is_materialized_once() {
    const SAME_TWICE: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n\
                              <region> sample=a.wav key=64 pitch_keycenter=60\n";
    const TWO_PATHS: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n\
                             <region> sample=c.wav key=64 pitch_keycenter=60\n";
    let a = InMemorySample::sine(440.0, 48_000.0, SAMPLE_FRAMES);
    let c = InMemorySample::sine(660.0, 48_000.0, SAMPLE_FRAMES);
    let provided = vec![("a.wav".to_owned(), a.clone())];
    let once = SfzInstrument::load(SAME_TWICE.to_owned(), &Default::default()).expect("解析");
    let twice = SfzInstrument::load(TWO_PATHS.to_owned(), &Default::default()).expect("解析");

    let deduped =
        PreparedSfz::prepare_with_samples(&once, &provided, 48_000.0).expect("摊平必须成功");
    let two_provided = vec![
        ("a.wav".to_owned(), a.clone()),
        ("c.wav".to_owned(), c.clone()),
    ];
    let doubled =
        PreparedSfz::prepare_with_samples(&twice, &two_provided, 48_000.0).expect("摊平必须成功");

    let one_copy = SAMPLE_FRAMES * core::mem::size_of::<f32>();
    println!(
        "[engine-sfz/S12] 同一路径引两次：sample_count = {}、pcm_bytes = {}（一份 = {one_copy}）；\
         两条不同路径：sample_count = {}、pcm_bytes = {}",
        deduped.sample_count(),
        deduped.pcm_bytes(),
        doubled.sample_count(),
        doubled.pcm_bytes(),
    );
    assert_eq!(
        deduped.region_count(),
        2,
        "两条区域都在（去重的是**样本**，⛔ 不是区域）"
    );
    assert_eq!(deduped.regions()[0].sample_index(), 0);
    assert_eq!(
        deduped.regions()[1].sample_index(),
        0,
        "⭐ 同一路径引两次 ⇒ 两条区域都指第 0 份（**只解码一次**）"
    );
    assert_eq!(deduped.sample_count(), 1, "⭐ 解码次数 ＝ 1");
    assert_eq!(
        deduped.pcm_bytes(),
        one_copy,
        "⭐ PCM 字节 ＝ **一份**（480 × 4 = {one_copy}）"
    );
    assert_eq!(doubled.sample_count(), 2, "反向对照：两条不同路径 ⇒ 2");
    assert_eq!(doubled.pcm_bytes(), 2 * one_copy, "反向对照：字节翻倍");
    assert_ne!(doubled.sample_count(), deduped.sample_count());

    // 提供表里同一路径两份（**内容不同**）⇒ 首个为准（⛔ 不静默合并内容）
    let second_a = InMemorySample::sine(880.0, 48_000.0, SAMPLE_FRAMES);
    let duplicated = vec![
        ("a.wav".to_owned(), a.clone()),
        ("a.wav".to_owned(), second_a),
    ];
    let first_wins =
        PreparedSfz::prepare_with_samples(&once, &duplicated, 48_000.0).expect("摊平必须成功");
    assert_eq!(
        first_wins.sample_count(),
        1,
        "同一路径两份 ⇒ 仍只 materialize 一份"
    );
    let mut out = vec![0.0f32; BLOCK];
    first_wins.render_block_into(60, 127, &mut out);
    let mut reference = vec![0.0f32; BLOCK];
    offline_one("a.wav", 60, &a, 60, &mut reference);
    assert_eq!(
        bits(&out),
        bits(&reference),
        "⭐ 提供表里同一路径出现两份时**首个为准**（内容不是第二份）"
    );
    // ⭐ 反向对照：第二份内容**确实不同**（否则上面的"首个为准"是空转）
    let mut second_reference = vec![0.0f32; BLOCK];
    offline_one(
        "a.wav",
        60,
        &InMemorySample::sine(880.0, 48_000.0, SAMPLE_FRAMES),
        60,
        &mut second_reference,
    );
    assert_ne!(
        fnv1a_64(&reference),
        fnv1a_64(&second_reference),
        "两份提供样本的内容必须真的不同（否则本条无鉴别力）"
    );
}

/// S13（步 3）：⭐ **`start`（`offset`）与 `end` 真正应用** —— 离线/摊平/实时三条路逐样本 0。
///
/// # 判定量
///
/// * 离线 vs 摊平**整块逐样本最大绝对差 ＝ 0**；
/// * `offset=10 end=200` ⇒ 写入帧数 **190**（半开 [10,201)），且输出第 0 帧
///   **逐位等于** `源[10] × gain_at(60,127)`；
/// * 反向对照：`offset=50` ⇒ 第 0 帧变成 `源[50] × gain` 且指纹不同（⛔ 证明起点真的进了循环）。
#[test]
fn s13_offset_and_end_are_applied_on_all_three_paths() {
    let a = InMemorySample::sine(440.0, 48_000.0, SAMPLE_FRAMES);
    let provided = vec![("a.wav".to_owned(), a.clone())];
    let span_sfz = SfzInstrument::load(
        "<region> sample=a.wav key=60 pitch_keycenter=60 offset=10 end=200\n".to_owned(),
        &Default::default(),
    )
    .expect("解析必须成功");
    let prepared =
        PreparedSfz::prepare_with_samples(&span_sfz, &provided, 48_000.0).expect("摊平必须成功");
    let region = &prepared.regions()[0];
    let gain = region.gain_at(60, 127);

    assert_eq!(
        region.source_start(SAMPLE_FRAMES),
        Some(10.0),
        "起点＝`offset`"
    );
    assert_eq!(
        region.source_end(SAMPLE_FRAMES),
        Some(201),
        "终点＝半开 [10,201)"
    );

    // ① 离线 vs 摊平：整块逐样本 0
    let mut offline = vec![0.0f32; BLOCK];
    span_sfz
        .render_note_into(&a, 60, 127, &mut offline, 48_000.0)
        .expect("离线渲染不得报错");
    let mut flat = vec![0.0f32; BLOCK];
    let flat_frames = prepared.render_block_into(60, 127, &mut flat);
    let diff = offline
        .iter()
        .zip(flat.iter())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max);

    // ② 实时声部也要在同一帧停、且与摊平逐样本 0
    let track = EntityId::new();
    let mut engine = engine_with_track(track);
    let schedule =
        NoteSchedule::from_sorted(vec![support::scheduled(0, 12_000, 60, 1.0, 48_000.0)]);
    let mut engine_out = vec![0.0f32; BLOCK];
    engine.render_track(track, Some(&schedule), Some(&prepared), &mut engine_out);
    let engine_diff = engine_out
        .iter()
        .zip(flat.iter())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max);

    let expected_first = a.pcm[10] * gain;
    println!(
        "[engine-sfz/S13] offset=10 end=200：摊平写入 = {flat_frames} 帧；离线 vs 摊平差 = {diff:e}；\
         引擎 vs 摊平差 = {engine_diff:e}；输出[0] = {}（源[10]×gain = {expected_first}）",
        flat[0]
    );
    assert_eq!(diff, 0.0, "⭐ 离线 vs 摊平必须逐样本相同（`span` 同口径）");
    assert_eq!(engine_diff, 0.0, "⭐ 引擎声部 vs 摊平必须逐样本相同");
    assert_eq!(flat_frames, 190, "半开 [10,201) ⇒ 190 帧");
    assert_eq!(
        flat[0].to_bits(),
        expected_first.to_bits(),
        "⭐ **起点真正应用**：输出第 0 帧必须逐位等于 `源[10] × gain`"
    );
    assert!(
        flat[190..].iter().all(|s| *s == 0.0),
        "⭐ `end` 真正应用：第 190 帧之后一帧不出"
    );

    // ③ 反向对照：换一个 `offset` ⇒ 起点与指纹都变
    let moved = SfzInstrument::load(
        "<region> sample=a.wav key=60 pitch_keycenter=60 offset=50 end=200\n".to_owned(),
        &Default::default(),
    )
    .expect("解析必须成功");
    let moved_plan =
        PreparedSfz::prepare_with_samples(&moved, &provided, 48_000.0).expect("摊平必须成功");
    let mut moved_out = vec![0.0f32; BLOCK];
    let moved_frames = moved_plan.render_block_into(60, 127, &mut moved_out);
    let expected_moved = a.pcm[50] * moved_plan.regions()[0].gain_at(60, 127);
    println!(
        "[engine-sfz/S13] 反向对照 offset=50：写入 = {moved_frames} 帧、输出[0] = {}（期望 {expected_moved}）",
        moved_out[0]
    );
    assert_eq!(
        moved_frames, 150,
        "半开 [50,201) ⇒ 151 个源采样 − 1（线性插值要两个样本）＝ 150 帧"
    );
    assert_eq!(
        moved_out[0].to_bits(),
        expected_moved.to_bits(),
        "⭐ `offset=50` ⇒ 起点必须跟着换"
    );
    assert_ne!(
        fnv1a_64(&moved_out),
        fnv1a_64(&flat),
        "⭐ 两个不同的 `offset` 必须给出不同波形（否则起点没进循环）"
    );
}

/// S14（步 3）：⭐ **单样本重载（广播）与步 2 逐位相同**。
///
/// 判定量：span-free 夹具 [`TWO_MATCH_ONE_MISS`] 的 `prepare`（广播）输出在
/// `(60, 127)`、512 帧上的 **FNV-1a 指纹** ＝ **步 2 实测** [`BROADCAST_STEP2_FINGERPRINT`]
/// （本机 `aarch64/macos` 上硬判；异平台如实跳过并点名原因）。
///
/// ⭐ 另附读数：`is_broadcast()` / `sample_content_label() == "broadcast(1)"`
/// ⇒ 报告里的 `sample_content=broadcast(1)` 有机械根据。
#[test]
fn s14_the_broadcast_overload_is_bit_identical_to_step_two() {
    let prepared = prepare(TWO_MATCH_ONE_MISS);
    let mut out = vec![0.0f32; BLOCK];
    let frames = prepared.render_block_into(60, 127, &mut out);
    println!(
        "[engine-sfz/S14] 广播路径 512 帧 @ (60,127)：写入 = {frames}、非零 = {}、\
         指纹 = 0x{:016x}（步 2 实测 = 0x{:016x}）、sample_content = {}",
        out.iter().filter(|s| **s != 0.0).count(),
        fnv1a_64(&out),
        BROADCAST_STEP2_FINGERPRINT,
        prepared.sample_content_label(),
    );
    assert!(out.iter().any(|s| *s != 0.0), "夹具必须真的出声");
    let _checked = check_fingerprint(
        "步 3 的广播重载 vs 步 2",
        fnv1a_64(&out),
        BROADCAST_STEP2_FINGERPRINT,
    );
    assert!(prepared.is_broadcast(), "`prepare` 必须标为广播");
    assert_eq!(
        prepared.sample_content_label(),
        "broadcast(1)",
        "⭐ 报告里的 `sample_content=broadcast(1)` 必须来自机械读数"
    );
    assert_eq!(prepared.sample_count(), 1);
    assert_eq!(prepared.silent_region_count(), 0);
    assert!(prepared.missing_sample_paths().is_empty());
    assert!(prepared.regions().iter().all(|r| r.sample_index() == 0));
}

/// S15（⭐ 本单改写，归属裁决见台账 R711）：⭐ **缺样本 ⇒ 静音 ＋ 计数**（⛔ 不静默、⛔ 不发明内容）。
///
/// # ⚠ 旧期望固化了**错内容**（如实说明）
///
/// 步 3 的 S15 断言的是"缺 `missing.wav` ⇒ 回退到 `samples[0]`，输出与'用 `a.wav` 播那个键'
/// **逐位相同**" ⇒ ⭐ 它把"缺样本的区域去播**别人的采样**"这条**内容错**钉成了期望值。
/// R711 裁决：**引擎出静音并计数**、**合成替代内容是控制侧的决定** ⇒ 本判据改为断言
/// **零输出 ＋ 计数正确**（新期望比旧期望**强**：旧的在"错内容"与"静音"之间分不出来）。
///
/// # 判定量（三条）
///
/// * 提供表**非空**但缺 `missing.wav` ⇒ 该区域 `NO_SAMPLE`：`silent_region_count() == 1`、
///   `missing_sample_paths() == ["missing.wav"]`、`sample_content_label() == "distinct(1)+silent(1)"`，
///   ⭐ 该键**一帧不写**（`render_block_into(64, 127)` 返回 **0** 且整块逐位 0）；
/// * 反向对照 ①：**同一份表**把 `missing.wav` 补上 ⇒ 同一个键**真的出声**且指纹 ≠ 0
///   （⇒ 上面那个 0 不是"这条区域本来就不响"）；
/// * 反向对照 ②：缺少的**只有** `missing.wav` ⇒ `a.wav` 那条区域（key=60）**照常出声**
///   （⇒ 静音的作用域**只到缺样本的那条区域**，⛔ 不是整条轨哑掉）；
/// * 提供表**为空** ⇒ `silent_region_count() == 2`、全部 `NO_SAMPLE`、**一帧不写**。
#[test]
fn s15_a_missing_sample_is_silent_and_is_counted() {
    const ONE_MISSING: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n\
                               <region> sample=missing.wav key=64 pitch_keycenter=60\n";
    let sfz = SfzInstrument::load(ONE_MISSING.to_owned(), &Default::default()).expect("解析");
    let a = InMemorySample::sine(440.0, 48_000.0, SAMPLE_FRAMES);
    let b = InMemorySample::sine(660.0, 48_000.0, SAMPLE_FRAMES);

    // ① 非空提供表 + 缺一份 ⇒ **静音** ＋ 计数（⛔ 不去播 `samples[0]`）
    let missing =
        PreparedSfz::prepare_with_samples(&sfz, &[("a.wav".to_owned(), a.clone())], 48_000.0)
            .expect("摊平必须成功");
    assert_eq!(missing.silent_region_count(), 1, "⭐ 缺样本的区域计数");
    assert_eq!(
        missing.missing_sample_paths().to_vec(),
        vec!["missing.wav".to_owned()],
        "⭐ 缺失路径必须**具名**登记（⛔ 不静默）"
    );
    assert_eq!(
        missing.sample_content_label(),
        "distinct(1)+silent(1)",
        "读数必须如实写出静音（⛔ 再没有 `+fallback(...)` 这一档）"
    );
    assert_eq!(
        missing.regions()[1].sample_index(),
        PreparedRegion::NO_SAMPLE,
        "⭐ 缺样本 ⇒ `NO_SAMPLE`（⛔ 不是回退到 `samples[0]`）"
    );
    assert!(
        !missing.regions()[1].has_sample(),
        "缺样本的区域必须**没有**样本可播"
    );
    let mut missing_out = vec![0.0f32; BLOCK];
    let missing_frames = missing.render_block_into(64, 127, &mut missing_out);
    // 反向对照 ② 的前置：**只有**缺样本的那条区域哑掉，key=60 那条照常出声。
    let mut sibling = vec![0.0f32; BLOCK];
    let sibling_frames = missing.render_block_into(60, 127, &mut sibling);
    println!(
        "[engine-sfz/S15] 缺样本：silent = {}、missing = {:?}、sample_content = {}、\
         key=64 写入 = {missing_frames}（整块非零 = {}）、key=60 写入 = {sibling_frames}",
        missing.silent_region_count(),
        missing.missing_sample_paths(),
        missing.sample_content_label(),
        missing_out.iter().filter(|s| **s != 0.0).count(),
    );
    assert_eq!(missing_frames, 0, "⭐ 缺样本 ⇒ **一帧不写**（引擎出静音）");
    assert!(
        missing_out.iter().all(|s| *s == 0.0),
        "⭐ 缺样本的区域必须**逐位静音**（⛔ 不许播别人的采样）"
    );
    assert!(
        sibling_frames > 0 && sibling.iter().any(|s| *s != 0.0),
        "反向对照②：缺的**只有** `missing.wav` ⇒ `a.wav` 那条区域必须照常出声"
    );

    // 反向对照 ①：把缺失那份补上 ⇒ 同一个键**真的出声**、指纹 ≠ 0
    let complete = PreparedSfz::prepare_with_samples(
        &sfz,
        &[
            ("a.wav".to_owned(), a.clone()),
            ("missing.wav".to_owned(), b.clone()),
        ],
        48_000.0,
    )
    .expect("摊平必须成功");
    let mut complete_out = vec![0.0f32; BLOCK];
    let complete_frames = complete.render_block_into(64, 127, &mut complete_out);
    assert!(
        complete.silent_region_count() == 0,
        "补上之后不得再有缺样本的区域"
    );
    assert!(complete.missing_sample_paths().is_empty());
    assert_ne!(
        complete_frames, 0,
        "⭐ 补上内容之后同一个键必须真的出声（否则上面那个 0 是空转）"
    );
    let mut reference = vec![0.0f32; BLOCK];
    offline_one("missing.wav", 64, &b, 64, &mut reference);
    assert_eq!(
        bits(&complete_out),
        bits(&reference),
        "补上的那一份必须**逐位**是它自己的内容（`b`，⛔ 不是 `a`）"
    );

    // ③ 空提供表 ⇒ 全部 NO_SAMPLE ＋ 计数 ＋ 一帧不写
    let empty = PreparedSfz::prepare_with_samples(&sfz, &[], 48_000.0).expect("摊平必须成功");
    let mut empty_out = vec![0.0f32; BLOCK];
    let empty_frames = empty.render_block_into(64, 127, &mut empty_out);
    println!(
        "[engine-sfz/S15] 空提供表：silent = {}、missing = {:?}、写入帧数 = {empty_frames}、\
         sample_content = {}",
        empty.silent_region_count(),
        empty.missing_sample_paths(),
        empty.sample_content_label(),
    );
    assert_eq!(empty.silent_region_count(), 2);
    assert!(empty.regions().iter().all(|r| !r.has_sample()));
    assert_eq!(empty_frames, 0, "没有样本 ⇒ 一帧不写");
    assert!(empty_out.iter().all(|s| *s == 0.0));
    assert_eq!(
        empty.missing_sample_paths().to_vec(),
        vec!["a.wav".to_owned(), "missing.wav".to_owned()],
        "缺失路径排序 ＋ 去重（确定性顺序）"
    );
}

/// S16（步 3）：⭐ **绑定注入 API 经公开路径可用（多采样）**。
///
/// 路径（全在集成测试里 ⇒ 只用 `pub` API）：
/// `SfzInstrument::referenced_sample_paths` →（控制侧解码 ⇒ 调用方）→
/// [`SfzPlan::prepare_with_samples`] → [`EngineSnapshot::with_sfz`] →
/// `render_snapshot`（快照槽 → 量子 → 声相 → 母线）。
///
/// 判定量：SFZ 臂非静音 ＋ 指纹 ≠ "不给 SFZ"；反向对照：**广播** plan 的端到端指纹与
/// 多采样版**不同**（⇒ 注入的多采样内容真的进了音频线程）。
#[test]
fn s16_the_multi_sample_binding_api_is_reachable_through_the_public_path() {
    let fixture = note_project(&E2E_NOTES);
    let track = fixture.track;
    let sfz =
        SfzInstrument::load(E2E_SFZ.to_owned(), &Default::default()).expect("SFZ 必须解析成功");

    // ① 控制侧：先查引用路径，再"解码"（判据里用内存样本；真读文件在 app 侧）
    let referenced = sfz.referenced_sample_paths().expect("路径查询");
    println!("[engine-sfz/S16] 引用路径 = {referenced:?}");
    assert_eq!(referenced, ["a.wav".to_owned(), "b.wav".to_owned()]);
    let a = InMemorySample::sine(440.0, 48_000.0, SAMPLE_FRAMES);
    let b = InMemorySample::sine(660.0, 48_000.0, SAMPLE_FRAMES);
    let plan = SfzPlan::prepare_with_samples(
        &sfz,
        &[
            ("a.wav".to_owned(), a.clone()),
            ("b.wav".to_owned(), b.clone()),
        ],
        48_000.0,
    )
    .expect("多采样摊平必须成功");
    assert_eq!(plan.prepared().sample_count(), 2);
    assert!(!plan.prepared().is_broadcast());

    // ② 注入（唯一公开绑定路径）＋ 端到端渲染
    let bare = EngineSnapshot::from_project(&fixture.project, 1).expect("快照");
    assert!(bare.sfz().is_empty(), "⛔ `from_project` 不得凭空绑 SFZ");
    let armed = EngineSnapshot::from_project(&fixture.project, 1)
        .expect("快照")
        .with_sfz(track, plan);
    let with_sfz = render_snapshot(armed, SFZ_QUANTA);
    let without = render(&fixture.project, SFZ_QUANTA);
    println!(
        "[engine-sfz/S16] 多采样臂: 非零={} 峰值={} 指纹=0x{:016x}；对照臂（不给 SFZ）: \
         非零={} 指纹=0x{:016x}",
        with_sfz.nonzero(),
        with_sfz.peak(),
        with_sfz.fingerprint(),
        without.nonzero(),
        without.fingerprint(),
    );
    assert!(with_sfz.nonzero() > 0, "⭐ 端到端必须非静音");
    assert_ne!(
        with_sfz.fingerprint(),
        without.fingerprint(),
        "⭐ 注入之后指纹必须不同（否则 SFZ 支路没接上）"
    );

    // ③ 反向对照：**广播** plan（只给 `a.wav`）⇒ 端到端指纹必须不同
    let broadcast_plan = SfzPlan::prepare(&sfz, &a, 48_000.0).expect("广播摊平");
    assert_eq!(
        broadcast_plan.prepared().sample_content_label(),
        "broadcast(1)"
    );
    let armed_broadcast = EngineSnapshot::from_project(&fixture.project, 1)
        .expect("快照")
        .with_sfz(track, broadcast_plan);
    let broadcast = render_snapshot(armed_broadcast, SFZ_QUANTA);
    println!(
        "[engine-sfz/S16] 广播臂: 非零={} 峰值={} 指纹=0x{:016x}",
        broadcast.nonzero(),
        broadcast.peak(),
        broadcast.fingerprint(),
    );
    assert!(
        broadcast.nonzero() > 0,
        "广播臂也必须出声（否则②的比较是空转）"
    );
    assert_ne!(
        broadcast.fingerprint(),
        with_sfz.fingerprint(),
        "⭐ 多采样 vs 广播的端到端指纹必须不同（`key=64` 那条区域换了内容）"
    );
}

// =====================================================================================
// ⭐ 归属裁决 `R711` 的两条落地判据（本单新增）：S17 = 轨推子（`needs-1`）、S18 = 防双重计。
// =====================================================================================

/// ⭐ S17／S18 用的**鼓机设备**：五个键位名写全 ⇒ `DrumsParams::from_devices` 认它是鼓机
/// （口径与 S7 的那个设备、与 `crates/yeban-engine/src/drums.rs` 的识别一致）。
fn drum_kit_device() -> DeviceDefinition {
    DeviceDefinition {
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
    }
}

/// ⭐ S17 的轨参数夹具：`volume_db` ＋ `mute` ＋ "**另一条轨在独奏**"（可闻门的三个自变量）。
///
/// `drums = true` ⇒ 本轨挂鼓机设备（分派次序 ＝ 鼓机 ＞ SFZ ⇒ 该臂走鼓机）；
/// `drums = false` ⇒ 不挂任何设备（该臂可由调用方注入 SFZ 摊平表）。
/// ⚠ 两个臂的**音符、推子、静音/独奏**逐字相同 ⇒ 差异**只有音源**。
fn fader_fixture(
    volume_db: f32,
    mute: bool,
    solo_elsewhere: bool,
    drums: bool,
) -> support::Fixture {
    let mut fixture = tuned_project(&E2E_NOTES, MixSpec::volume(volume_db));
    let track = fixture.track;
    {
        let entry = fixture
            .project
            .tracks
            .get_mut(&track)
            .expect("夹具必须有那条 MIDI 轨");
        entry.mute = mute;
        if drums {
            entry.devices = vec![drum_kit_device()];
        }
    }
    if solo_elsewhere {
        let other = EntityId::new();
        fixture.project.tracks.insert(
            other,
            TrackV3 {
                id: other,
                name: "Solo".to_owned(),
                kind: TrackKind::Midi,
                solo: true,
                ..TrackV3::default()
            },
        );
        fixture.project.routing_graph.nodes.push(other);
    }
    fixture
}

/// 判据窗口（帧）：`E2E_NOTES` 的两个起音（0 与 48 tick = **1200** 帧）都落在窗口内。
const FADER_FRAMES: usize = 2560;

/// 本轨的**单声道时域输出**（`render_track` ⇒ **声相与母线之前**）。
///
/// # ⚠ 量什么（先说再跑）
///
/// 量的是**本轨的样本序列**（单位：线性幅度，48 kHz 单声道，2560 帧）。
///
/// ⚠ **为什么⛔ 不量端到端**：母线限制器是**非线性**器件 ⇒ 它会给出一个与轨增益无关的
/// 因子，把"轨增益的线性"测糊。本判据要证的是"轨增益乘了一次、且两件音源乘的是**同一个**值"
/// ⇒ 必须在这一级量。
///
/// 路径：`EngineSnapshot::from_project` → `SynthEngine::arm_from_snapshot`（**产品路径**，
/// 含 `set_track_gain_scales`）→ `render_track`。
fn track_output(fixture: &support::Fixture, sfz: Option<&PreparedSfz>) -> Vec<f32> {
    let track = fixture.track;
    let snapshot = EngineSnapshot::from_project(&fixture.project, 1).expect("夹具工程必须成快照");
    let mut engine = SynthEngine::new(48_000);
    engine
        .arm_from_snapshot(&snapshot)
        .expect("武装最小工程不得报错");
    let schedule = snapshot
        .schedules()
        .get(&track)
        .cloned()
        .expect("本轨必须有调度表");
    let mut out = vec![0.0f32; FADER_FRAMES];
    engine.render_track(track, Some(&schedule), sfz, &mut out);
    out
}

/// ⭐ 一个档的**两臂峰值** ＋ 两臂的时域缓冲：(SFZ 臂, 鼓机臂)。
fn two_arm_outputs(
    volume_db: f32,
    mute: bool,
    solo_elsewhere: bool,
    prepared: &PreparedSfz,
) -> (Vec<f32>, Vec<f32>) {
    let sfz_arm = track_output(
        &fader_fixture(volume_db, mute, solo_elsewhere, false),
        Some(prepared),
    );
    let drum_arm = track_output(&fader_fixture(volume_db, mute, solo_elsewhere, true), None);
    (sfz_arm, drum_arm)
}

/// 峰值（绝对值最大；单位：线性幅度）。
fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs()))
}

/// S17（⭐ 本单新增；归属裁决 `R711` 的 `needs-1` 落地）：⭐ **轨推子变化 N dB ⇒
/// SFZ 与鼓机输出同比例变**。
///
/// # 判定量（三条腿，都可机械判定）
///
/// 1. **推子 −6 dB**：`peak(−6) / peak(0)` 在**两臂**上都等于 [`track_gain`]`(−6.0)`
///    （相对误差 ≤ `1e-5`），且两臂的比值**互相**相等 ⇒ ⭐ "同径、同一个算子"；
/// 2. **静音**（`mute = true`）：两臂**逐位**为 `0.0`（⇒ 可闻门真的到了声部上）；
/// 3. **另一条轨独奏**（`any_solo = true` 且本轨既不 solo 也不 solo_safe）：两臂**逐位** `0.0`
///    （⇒ 独奏与静音走的是**同一个** `track_is_audible` 门）。
///
/// # 怎么变红
///
/// 让 SFZ 那条支路不再乘轨增益（把 `trigger_note` 的 `region.gain_at(...) * track_gain`
/// 改回 `region.gain_at(...)`）⇒ 腿 ① 的 SFZ 比值变回 `1.0`、腿 ②/③ 的 SFZ 臂**不再全零**。
/// ⚠ 腿 ① 本身⛔ 不能证明"鼓机也吃了推子"：那由**同一个** `track_gain_scale` 在
/// `snapshot::project_schedules` 里的一次调用保证（鼓机读含轨门的 `note.gain`）。
#[test]
fn s17_a_track_fader_scales_the_sfz_and_the_drums_arm_by_the_same_ratio() {
    let prepared = prepare(E2E_SFZ);

    // ③ 前置：**两臂都必须真的出声**（否则"同比例"可能是 0/0 那种空话）。
    let (sfz_0, drum_0) = two_arm_outputs(0.0, false, false, &prepared);
    let (sfz_6, drum_6) = two_arm_outputs(-6.0, false, false, &prepared);
    let (sfz_mute, drum_mute) = two_arm_outputs(0.0, true, false, &prepared);
    let (sfz_solo, drum_solo) = two_arm_outputs(0.0, false, true, &prepared);

    let expected = track_gain(-6.0);
    let sfz_ratio = peak(&sfz_6) / peak(&sfz_0);
    let drum_ratio = peak(&drum_6) / peak(&drum_0);
    println!(
        "[engine-sfz/S17] 峰值：SFZ 臂 0 dB = {:.9} / −6 dB = {:.9} / 静音 = {:.9} / 别轨独奏 = {:.9}；\
         鼓机臂 0 dB = {:.9} / −6 dB = {:.9} / 静音 = {:.9} / 别轨独奏 = {:.9}；\
         比值 SFZ = {sfz_ratio:.9} vs 鼓机 = {drum_ratio:.9}（期望 track_gain(−6 dB) = {expected:.9}）",
        peak(&sfz_0),
        peak(&sfz_6),
        peak(&sfz_mute),
        peak(&sfz_solo),
        peak(&drum_0),
        peak(&drum_6),
        peak(&drum_mute),
        peak(&drum_solo),
    );

    assert!(
        peak(&sfz_0) > 0.0 && peak(&drum_0) > 0.0,
        "0 dB 档两臂都必须真的出声（SFZ = {} / 鼓机 = {}）",
        peak(&sfz_0),
        peak(&drum_0)
    );
    assert!(
        peak(&sfz_6) > 0.0 && peak(&drum_6) > 0.0,
        "−6 dB 档两臂都必须仍然出声（⇒ 比值不是 0/0）"
    );
    // ① 推子比例：两臂各自 == `track_gain(-6)`，且两臂**互相**相等。
    let rel = |a: f32, b: f32| (a - b).abs() / b.abs();
    assert!(
        rel(sfz_ratio, expected) <= 1e-5,
        "⭐ SFZ 臂的推子比例必须 ＝ `track_gain(-6 dB)`：实得 {sfz_ratio} vs {expected}"
    );
    assert!(
        rel(drum_ratio, expected) <= 1e-5,
        "⭐ 鼓机臂的推子比例必须 ＝ `track_gain(-6 dB)`：实得 {drum_ratio} vs {expected}"
    );
    assert!(
        rel(sfz_ratio, drum_ratio) <= 1e-5,
        "⭐ ⭐ **同径**：SFZ 与鼓机的推子比例必须一致（{sfz_ratio} vs {drum_ratio}）"
    );
    // ② 静音档：两臂**逐位** 0（⛔ 不是"很小"）。
    assert!(
        sfz_mute.iter().all(|s| *s == 0.0) && drum_mute.iter().all(|s| *s == 0.0),
        "⭐ 静音轨 ⇒ 两臂都必须**逐位**静音（SFZ 峰值 = {}、鼓机峰值 = {}）",
        peak(&sfz_mute),
        peak(&drum_mute)
    );
    // ③ 独奏档：同一道可闻门。
    assert!(
        sfz_solo.iter().all(|s| *s == 0.0) && drum_solo.iter().all(|s| *s == 0.0),
        "⭐ 另一条轨独奏 ⇒ 本轨两臂都必须**逐位**静音（SFZ 峰值 = {}、鼓机峰值 = {}）",
        peak(&sfz_solo),
        peak(&drum_solo)
    );
}

/// S18（⭐ 本单新增；归属裁决 `R711` 的**防双重计**腿）：⭐ **引擎层只乘轨增益，
/// ⛔ 绝不替乐器层再乘一次力度**。
///
/// # 判定量
///
/// 每个力度 `v ∈ {0, 32, 64, 100, 127}`：**引擎声部路径**与**摊平表（乐器层）一次性渲染**
/// **逐位相同**。手工调度表里 `ScheduledNote::gain` 显式写成 `velocity_gain(v)`
/// （＝"如果引擎把力度也交给 SFZ，就会多乘一次"的那一份）⇒ 只要两侧逐位相同，
/// 就证明引擎**没有**碰力度、只乘了轨增益（本档轨增益 ＝ 默认 `1.0`）。
///
/// # ⚠ 反向对照：为什么必须用**中段**力度
///
/// `velocity_gain(127) == 1.0` 是**恒等** ⇒ 在 127 上"多乘一次力度"看不出来。
/// S1 那条腿正是 127 ⇒ ⭐ **S1 抓不住双重计**，本判据补的正是这个盲区（下面两条断言钉住它）。
///
/// # 怎么变红
///
/// 在 `SfzVoicePool::trigger_note` 里把增益改成
/// `region.gain_at(note, velocity) * track_gain * velocity_gain(velocity)` ⇒
/// `v ∈ {32, 64, 100}` 三条腿当场**逐位不等**（`v = 127` 那条仍绿 ⇒ 盲区真实存在）。
#[test]
fn s18_the_engine_never_applies_a_second_velocity_gain() {
    let prepared = prepare(TWO_MATCH_ONE_MISS);
    let mut rows = Vec::new();
    for velocity in [0u8, 32, 64, 100, 127] {
        let track = EntityId::new();
        let mut engine = engine_with_track(track);
        // ⚠ 手工调度表：轨增益门 ＝ `NoteSchedule::from_sorted` 的默认 **1.0**（单位）；
        //    逐音符 `gain` 显式写成 `velocity_gain(v)`（见判据文档）。
        let schedule = NoteSchedule::from_sorted(vec![ScheduledNote::new(
            0,
            12_000,
            60,
            velocity,
            support::yeban_dsp_note_to_hz(60),
            velocity_gain(velocity),
            48_000.0,
        )]);
        let mut engine_out = vec![0.0f32; BLOCK];
        engine.render_track(track, Some(&schedule), Some(&prepared), &mut engine_out);
        let mut flat = vec![0.0f32; BLOCK];
        prepared.render_block_into(60, velocity, &mut flat);
        rows.push(format!(
            "v={velocity}: 引擎 = {:#010x} / 摊平 = {:#010x}",
            fnv1a_64(&engine_out),
            fnv1a_64(&flat)
        ));
        assert_eq!(
            bits(&engine_out),
            bits(&flat),
            "⭐ 力度 {velocity}：引擎声部路径必须与**乐器层**一次性渲染**逐位相同**             （⇒ 引擎⛔ 没有替乐器层再乘一次力度）"
        );
    }
    println!("[engine-sfz/S18] 逐位对账 ⇒ {}", rows.join("；"));

    // 反向对照：`velocity_gain(127) == 1.0`（恒等）⇒ 127 那条腿对"双重计"**没有牙**；
    // 中段力度才有（下面这条断言就是 S18 必须存在、而 S1 不够的理由）。
    assert_eq!(velocity_gain(127), 1.0, "127 的力度增益必须是**恒等**");
    assert!(
        velocity_gain(64) > 0.0 && velocity_gain(64) < 1.0,
        "中段力度必须既非 0 也非恒等（实得 {}）",
        velocity_gain(64)
    );
    assert_ne!(
        velocity_gain(64),
        1.0,
        "⭐ 中段力度是**恒等**的话，本判据就没有鉴别力了"
    );
}

/// ⭐ S19 的夹具：`regions` 条 `key` 的区域 ＋ 一份**常数 PCM**（全 `1.0`）。
///
/// 常数 PCM 的理由：淡出是**逐帧增益**，常数源 ⇒ 输出**就是那条增益曲线**
/// （`out[k] == gain × fade_gain(k)`），读数可以直接与 `FLOOR^(n/N)` 对账。
fn constant_plan(regions: usize, key: u8) -> PreparedSfz {
    let mut text = String::new();
    for _ in 0..regions {
        text.push_str(&format!(
            "<region> sample=a.wav key={key} pitch_keycenter=60\n"
        ));
    }
    let sfz = SfzInstrument::load(text, &Default::default()).expect("SFZ 必须解析成功");
    let sample = InMemorySample {
        pcm: vec![1.0f32; SAMPLE_FRAMES],
        sample_hz: 48_000.0,
    };
    PreparedSfz::prepare(&sfz, &sample, 48_000.0).expect("摊平必须成功")
}

/// ⭐ S19（本单新增；`sfz-needs-2`）：⭐ **池满 ⇒ 窃取最旧的声部 ＋ 3 ms 指数淡出**。
///
/// # 夹具（为什么要三次触发）
///
/// * 第 1 次：**只**触发 1 条区域（`key=60`，终点 `LATE`）⇒ 槽 0、触发序号最小；
/// * 第 2 次：触发 15 条区域（`key=60`，终点 `EARLY = 64`）⇒ 填满槽 1..15；
/// * 第 3 次：触发 1 条区域（`key=64`）⇒ 池满 ⇒ **窃取最旧的**（槽 0）。
///
/// 从 `position = 64` 起只有槽 0 还在响（其余 15 个的引擎终点已过 ⇒ 渲染时如实退休）
/// ⇒ 本判据量的**就是被窃取者那条淡出曲线**。
///
/// # 判定量
///
/// * 窃取读数：`created == 0`（没有空槽）、`stolen == 1`、`dropped == 0`；
/// * `fade_remaining_at(0) == 144`（3 ms @ 48 kHz，与复音**同一把舵**）；
/// * 淡出窗口里槽 0 仍宣称**被窃取者**（`note == 60`）；走完 144 帧后**就地换成**
///   新声部（`note == 64`）且仍是活声部 ⇒ "被换掉"可读；
/// * **输出读数**：`out[143] / out[0] == FLOOR^(143/144)`（逐位对账）；
/// * **硬窃取对照臂**（`set_steal_fade_frames(0)`）：同一次触发 ⇒ 槽 0 **立刻**是
///   `note == 64`、输出**无衰减**（`out[143] == out[0]` 逐位）⇒ 两条臂指纹不同
///   （⇒ ②"淡出确实生效"不是空转）。
///
/// ⚠ 本判据的**旧行为**是"第 3 次直接丢弃"⇒ 池仍是 16 个 `note == 60` 的声部、
/// `dropped == 1`、`stolen == 0` ⇒ 本判据当场变红（注入读数见提交报告）。
#[test]
fn s19_a_full_pool_steals_the_oldest_voice_behind_a_three_millisecond_fade() {
    const LATE: u64 = 12_000;
    const EARLY: u64 = 64;
    let one = constant_plan(1, 60);
    let fifteen = constant_plan(SFZ_VOICES_PER_TRACK - 1, 60);
    let stealer = constant_plan(1, 64);

    let mut pool = SfzVoicePool::empty();
    assert_eq!(pool.steal_fade_frames(), 144, "3 ms @ 48 kHz");
    assert_eq!(pool.trigger_note(&one, 60, 100, LATE, 1.0), 1, "槽 0");
    assert_eq!(
        pool.trigger_note(&fifteen, 60, 100, EARLY, 1.0),
        SFZ_VOICES_PER_TRACK - 1,
        "填满槽 1..15"
    );
    assert_eq!(pool.active_count(), SFZ_VOICES_PER_TRACK);

    // ③ 池满 ⇒ 窃取最旧的（槽 0 的触发序号最小）。
    let created = pool.trigger_note(&stealer, 64, 100, LATE, 1.0);
    println!(
        "[engine-sfz/S19] 池满触发：新建 = {created}；窃取 = {}；丢弃 = {}；挂起 = {}；\
         槽 0 淡出剩余 = {}；槽 0 在场音高 = {}",
        pool.stolen(),
        pool.dropped(),
        pool.pending_count(),
        pool.fade_remaining_at(0),
        pool.voices()[0].note(),
    );
    assert_eq!(created, 0, "池满 ⇒ 没有空槽可占（窃取占的是别人的槽）");
    assert_eq!(pool.stolen(), 1);
    assert_eq!(pool.dropped(), 0, "⭐ 窃取⛔ 不是丢弃");
    assert_eq!(pool.pending_count(), 1, "新声部挂在槽 0 等接管");
    assert_eq!(pool.fade_remaining_at(0), 144, "⭐ 3 ms ＝ 144 帧 @ 48 kHz");
    assert_eq!(
        pool.voices()[0].note(),
        60,
        "淡出窗口里槽位必须仍宣称**被窃取者**（口径与 PolySynth 同款）"
    );
    assert_eq!(pool.active_count(), SFZ_VOICES_PER_TRACK, "16 个槽一个不空");

    // ④ 输出读数：从 position = 64 起只有槽 0 出声，144 帧正好走完淡出。
    let mut out = vec![0.0f32; 144];
    pool.render_into(&one, EARLY, &mut out);
    // ⚠ 期望值写成**字面量** `1e-3`（−60 dBFS）而不是去读实现里的常量：
    //    判据用实现自己的常量做期望值是自我确认（实现改错了也一起改"对"）。
    let decay = 1.0e-3f64.powf(1.0 / 144.0) as f32;
    let mut expected = 1.0f32;
    for _ in 0..143 {
        expected *= decay;
    }
    println!(
        "[engine-sfz/S19] 淡出：out[0] = {:.9}；out[143] = {:.9}（期望 FLOOR^(143/144)·out[0] = {:.9}）；\
         比值 = {:.9}",
        out[0],
        out[143],
        out[0] * expected,
        out[143] / out[0],
    );
    assert!(out[0] > 0.0, "淡出首帧必须出声（否则比值是 0/0）");
    assert_eq!(
        out[143].to_bits(),
        (out[0] * expected).to_bits(),
        "⭐ 第 143 帧必须逐位等于 `out[0] × FLOOR^(143/144)`（StealFade 的指数口径）"
    );
    assert!(
        out[143] / out[0] < 0.002,
        "⭐ 淡出末尾必须落到 −60 dBFS 一带（实得 {}）",
        out[143] / out[0]
    );

    // ⑤ 走完 144 帧 ⇒ 挂起者**就地接管**（同一个槽），仍是活声部。
    assert_eq!(
        pool.voices()[0].note(),
        64,
        "⭐ 淡出走完 ⇒ 最旧的声部被换掉（槽 0 变成新声部）"
    );
    assert_eq!(pool.pending_count(), 0, "挂起者已经接管");
    assert_eq!(pool.fade_remaining_at(0), 0);
    assert!(pool.voices()[0].is_active(), "接管者必须开始发声");
    let mut after = vec![0.0f32; 64];
    pool.render_into(&one, EARLY + 144, &mut after);
    assert!(
        after.iter().any(|s| *s != 0.0),
        "⭐ 接管者必须真的出声（⛔ 不是把槽弄哑）"
    );

    // ⑥ 硬窃取对照臂：同样的触发序列，淡出帧数 = 0 ⇒ 立刻换人、输出无衰减。
    let mut hard = SfzVoicePool::empty();
    hard.set_steal_fade_frames(0);
    hard.trigger_note(&one, 60, 100, LATE, 1.0);
    hard.trigger_note(&fifteen, 60, 100, EARLY, 1.0);
    let hard_created = hard.trigger_note(&stealer, 64, 100, LATE, 1.0);
    let mut hard_out = vec![0.0f32; 144];
    hard.render_into(&one, EARLY, &mut hard_out);
    println!(
        "[engine-sfz/S19] 硬窃取对照：新建 = {hard_created}；槽 0 音高 = {}；淡出剩余 = {}；\
         指纹 软 = 0x{:016x} / 硬 = 0x{:016x}",
        hard.voices()[0].note(),
        hard.fade_remaining_at(0),
        fnv1a_64(&out),
        fnv1a_64(&hard_out),
    );
    assert_eq!(hard.stolen(), 1, "硬窃取同样计一次窃取");
    assert_eq!(hard.dropped(), 0);
    assert_eq!(
        hard.voices()[0].note(),
        64,
        "⭐ 硬窃取（帧数 0）⇒ **立刻**接管，没有淡出窗口"
    );
    assert_eq!(hard.fade_remaining_at(0), 0);
    assert_eq!(
        hard_out[143].to_bits(),
        hard_out[0].to_bits(),
        "硬窃取没有衰减（逐位相同）"
    );
    assert_ne!(
        fnv1a_64(&out),
        fnv1a_64(&hard_out),
        "⭐ 软/硬两条臂的指纹必须不同（否则'淡出生效'可能是空转）"
    );
}

/// ⭐ S20（本单新增；`sfz-needs-3`）：⭐ **三件同轨的可读信号**（⛔ 不改分派次序）。
///
/// # 裁决（本单）
///
/// 次序**维持** `鼓机 ＞ SFZ ＞ 复音`（与 `crates/yeban-engine/src/drums.rs` §4.1
/// "一条轨至多一件音源"的既有裁决一致）⇒ ⛔ **不改成叠加**。真正缺的是
/// "用户看得出被忽略了" ⇒ 新增**投影读数** [`SynthEngine::armed_sources`]
/// ／ [`SynthEngine::ignored_sources`]（`render_track` 每量子写一个 `u8`）。
///
/// # 判定量
///
/// * 三件同轨（鼓机设备 ＋ 注入 SFZ ＋ **恒在场**的复音）⇒ `armed_sources == 3`
///   ＋ `ignored_sources == 2`（⭐ 本单要求的"被忽略的音源计数 ＝ 2"）；
/// * 输出仍对"只有鼓机"**逐位相同**（S7 的复核腿 ⇒ 信号⛔ 不改变输出）；
/// * 反向对照：鼓机＋复音 ⇒ `2 / 1`；只有复音 ⇒ `1 / 0`；**没渲染过**的轨 ⇒ `None`。
#[test]
fn s20_three_sources_on_one_track_report_two_ignored() {
    let prepared = prepare(TWO_MATCH_ONE_MISS);
    let track = EntityId::new();
    let params = DrumsParams::from_devices(&[drum_kit_device()]).expect("鼓机设备必须命中");
    let schedule =
        NoteSchedule::from_sorted(vec![support::scheduled(0, 12_000, 60, 1.0, 48_000.0)]);

    let mut three = SynthEngine::new(48_000);
    let mut drums_only = SynthEngine::new(48_000);
    let mut poly_only = SynthEngine::new(48_000);
    let mut never_rendered = SynthEngine::new(48_000);
    three.begin_snapshot(48_000, &[track], [], [(&track, &params)]);
    drums_only.begin_snapshot(48_000, &[track], [], [(&track, &params)]);
    poly_only.begin_snapshot(48_000, &[track], [], []);
    never_rendered.begin_snapshot(48_000, &[track], [], []);

    let mut a = vec![0.0f32; 128];
    let mut b = vec![0.0f32; 128];
    let mut c = vec![0.0f32; 128];
    three.render_track(track, Some(&schedule), Some(&prepared), &mut a);
    drums_only.render_track(track, Some(&schedule), None, &mut b);
    poly_only.render_track(track, Some(&schedule), None, &mut c);

    println!(
        "[engine-sfz/S20] 三件同轨：武装 = {:?}；被忽略 = {:?}（鼓机＋SFZ 非零 = {}）；\
         鼓机＋复音：武装 = {:?}／被忽略 = {:?}；只有复音：武装 = {:?}／被忽略 = {:?}；\
         没渲染过：武装 = {:?}",
        three.armed_sources(track),
        three.ignored_sources(track),
        a.iter().filter(|s| **s != 0.0).count(),
        drums_only.armed_sources(track),
        drums_only.ignored_sources(track),
        poly_only.armed_sources(track),
        poly_only.ignored_sources(track),
        never_rendered.armed_sources(track),
    );
    assert_eq!(
        three.armed_sources(track),
        Some(3),
        "⭐ 鼓机 ＋ SFZ ＋ 复音 ⇒ 武装 3 件"
    );
    assert_eq!(
        three.ignored_sources(track),
        Some(2),
        "⭐ 三件同轨 ⇒ **被忽略的音源计数 ＝ 2**（本单的可读信号）"
    );
    assert!(
        b.iter().any(|s| *s != 0.0),
        "反向对照：鼓机臂必须真的出声（否则下面的等号可能只是两边都哑）"
    );
    assert_eq!(
        bits(&a),
        bits(&b),
        "⭐ 信号⛔ 不改变输出：三件同轨仍对'只有鼓机'**逐位相同**（S7 的复核腿）"
    );
    assert_eq!(
        drums_only.armed_sources(track),
        Some(2),
        "反向对照：鼓机 ＋ 复音 ⇒ 武装 2 件"
    );
    assert_eq!(drums_only.ignored_sources(track), Some(1));
    assert_eq!(
        poly_only.armed_sources(track),
        Some(1),
        "反向对照：只有复音 ⇒ 武装 1 件、没有取舍"
    );
    assert_eq!(poly_only.ignored_sources(track), Some(0));
    assert_eq!(
        never_rendered.armed_sources(track),
        None,
        "⛔ 没渲染过的槽必须读 `None`（不是'0 件'）"
    );
}

// ---------------------------------------------------------------------------
// ⭐ 任务单 `#78`：引擎侧**消费** `TrackV3.sampler`（⭐ 只接线、⛔ 不读盘）
// ---------------------------------------------------------------------------

/// S21／S22／S23 的路径夹具：`instruments/lead.sfz`。
///
/// ⚠ 它**刻意在磁盘上不存在**（判据不碰文件系统）：引擎侧⛔ 不读盘 ⇒ 这个路径无论
/// 真假都⛔ 不该改变任何东西（⛔ 不报错、⛔ 不建声部、⛔ 不改渲染）—— 那正是
/// "接线 ⛔ 不是读盘"的机械见证。
const SAMPLER_PATH: &str = "instruments/lead.sfz";

/// S24 的块长：**小于**样本帧数（`SAMPLE_FRAMES = 480`）⇒ 有样本的声部在这一次
/// 渲染里**不会播完退休** ⇒ `sfz_active_voices` 才有区分度（`BLOCK = 512` 会让
/// 声部在同一块里播完，两条腿都会读成 `0` —— 那会让"不建声部"这条腿失去牙齿）。
const S24_BLOCK: usize = 128;

/// 把夹具工程里那条 MIDI 轨的 `sampler` 设成给定值（`None` ⇒ 清空），其余一字不动。
///
/// ⭐ "只差这一个字段"是 S21／S23 反向对照成立的**前提**：两条臂的工程除 `sampler`
/// 之外完全相同（同一份夹具、同一批音符、同一条路由）。
fn project_with_sampler(fixture: &support::Fixture, sampler: Option<&str>) -> YebanProjectV1 {
    let mut project = fixture.project.clone();
    project
        .tracks
        .get_mut(&fixture.track)
        .expect("夹具里必须有那条 MIDI 轨")
        .sampler = sampler.map(|path| SamplerBinding {
        path: path.to_owned(),
    });
    project
}

/// ⭐ S21（任务单 `#78`；⭐ 本单核心的**反向对照**）：`sampler` 缺席 ⇒ 快照与渲染与
/// **接线前**逐位相同。
///
/// # 判定量
///
/// 两条臂（无设备链 / 带音色）的端到端 **FNV-1a 指纹**必须等于 [`BASELINE_PLAIN`]／
/// [`BASELINE_TUNED`] —— 那两个字面值是**接线前**（HEAD `720db9f1`，即本单的第一条
/// 命令之后）在同一夹具、同一量子数、同一条产品路径上实测的（同 S5 的口径）。
/// 另加一条**结构腿**：`sampler = None` ⇒ `sfz_pending()` 必须**空**。
///
/// # 怎么变红
///
/// 在 `from_project_with_latencies` 里把 `track.sampler` 也塞进**摊平表**（或让
/// 实时侧读 `sfz_pending`）⇒ 第二条腿（`sfz_pending()` 空）与第一条腿（指纹）当场红。
#[test]
fn s21_an_absent_sampler_is_bit_identical_to_before_the_wiring() {
    let plain = note_project(&SFZ_NOTES);
    let plain_snapshot = EngineSnapshot::from_project(&plain.project, 1).expect("快照");
    assert!(
        plain_snapshot.sfz_pending().is_empty(),
        "⛔ `sampler = None` ⇒ 待摊平表必须为空（实得 {} 条）",
        plain_snapshot.sfz_pending().len()
    );
    assert!(plain_snapshot.pending_sfz(&plain.track).is_none());

    let rendered = render(&plain.project, SFZ_QUANTA);
    assert!(rendered.nonzero() > 0, "夹具必须真的出声（否则等号是空转）");
    println!(
        "[engine-sfz/S21] `sampler = None` 无设备链: 帧={} 非零={} 峰值={} 指纹={} (0x{:016x})",
        rendered.frames(),
        rendered.nonzero(),
        rendered.peak(),
        rendered.fingerprint(),
        rendered.fingerprint(),
    );
    let plain_checked = check_fingerprint(
        "`sampler = None` 无设备链",
        rendered.fingerprint(),
        BASELINE_PLAIN,
    );

    let tuned = tuned_project(&SFZ_NOTES, MixSpec::tone(800.0, 0.3));
    let tuned_snapshot = EngineSnapshot::from_project(&tuned.project, 1).expect("快照");
    assert!(tuned_snapshot.sfz_pending().is_empty());
    let rendered_tuned = render(&tuned.project, SFZ_QUANTA);
    println!(
        "[engine-sfz/S21] `sampler = None` 音色(800, 0.3): 帧={} 非零={} 峰值={} 指纹={} (0x{:016x})",
        rendered_tuned.frames(),
        rendered_tuned.nonzero(),
        rendered_tuned.peak(),
        rendered_tuned.fingerprint(),
        rendered_tuned.fingerprint(),
    );
    let tuned_checked = check_fingerprint(
        "`sampler = None` 音色(800, 0.3)",
        rendered_tuned.fingerprint(),
        BASELINE_TUNED,
    );

    assert_eq!(
        plain_checked, tuned_checked,
        "两条臂必须在**同一平台判定**下（否则本判据的读数不可比）"
    );
}

/// ⭐ S22（任务单 `#78`；**正向**）：`sampler = Some(路径)` ⇒ 快照里出现**该轨**的
/// 待摊平项，路径**逐字**相同。
///
/// # 判定量
///
/// * `sfz_pending()` **恰好 1 条**、键就是那条 MIDI 轨（母线⛔ 不在表里）；
/// * `pending_sfz(track).path()` **逐字**等于模型里那个路径（⛔ 不规范化、⛔ 不改写）；
/// * ⛔ **摊平表 `sfz()` 仍空** —— 引擎没有凭空摊平（那需要读盘）；
/// * 反向对照：同一夹具把 `sampler` 清回 `None` ⇒ 待摊平表**空**。
#[test]
fn s22_a_sampler_path_lands_in_the_snapshot_as_a_pending_item() {
    let fixture = note_project(&SFZ_NOTES);
    let project = project_with_sampler(&fixture, Some(SAMPLER_PATH));
    let snapshot = EngineSnapshot::from_project(&project, 1).expect("快照");

    let paths: Vec<&str> = snapshot
        .sfz_pending()
        .values()
        .map(PendingSfz::path)
        .collect();
    println!(
        "[engine-sfz/S22] 待摊平条数 = {}；路径 = {paths:?}；摊平表条数 = {}；\
         母线在待摊平表里 = {}",
        snapshot.sfz_pending().len(),
        snapshot.sfz().len(),
        snapshot.sfz_pending().contains_key(&fixture.master),
    );

    assert_eq!(
        snapshot.sfz_pending().len(),
        1,
        "⭐ `sampler = Some(…)` ⇒ 恰好一条待摊平项"
    );
    let pending = snapshot
        .pending_sfz(&fixture.track)
        .expect("⭐ 正向：那条 MIDI 轨必须出现在待摊平表里");
    assert_eq!(
        pending.path(),
        SAMPLER_PATH,
        "⭐ 路径必须**逐字**来自模型（⛔ 不规范化、⛔ 不查存在性）"
    );
    assert!(
        !snapshot.sfz_pending().contains_key(&fixture.master),
        "没绑定 `sampler` 的轨（母线）⛔ 不得出现在待摊平表里"
    );
    assert!(
        snapshot.sfz().is_empty(),
        "⛔ 引擎不读盘 ⇒ 摊平表必须仍为空（只有控制侧解码后 `with_sfz` 才填它）"
    );

    // 反向对照：同一夹具、只把 `sampler` 清回 `None` ⇒ 待摊平表空。
    let cleared = project_with_sampler(&fixture, None);
    let cleared_snapshot = EngineSnapshot::from_project(&cleared, 1).expect("快照");
    assert!(
        cleared_snapshot.sfz_pending().is_empty(),
        "反向对照：`sampler = None` ⇒ 待摊平表必须为空"
    );
}

/// ⭐ S23（任务单 `#78`）：**待摊平项是纯事实** —— 它⛔ 不改渲染、⛔ 不报错；
/// 而 `with_sfz` 注入摊平表时**取代**它。
///
/// # 判定量
///
/// 1. 同夹具 `None`／`Some(不存在的路径)` 两条臂的端到端 FNV-1a 指纹**逐位相同**
///    （且都真的出声）⇒ ⭐ 引擎⛔ 没有第二份音源事实源；
/// 2. 那个路径**磁盘上不存在**，但 `from_project` 仍 `Ok` ⇒ ⭐ 引擎⛔ 不读盘
///    （真读盘会 `Err` 或至少要碰文件系统）；
/// 3. 控制侧注入摊平表（`with_sfz`）⇒ 待摊平项被**取代**（表清空、`sfz()` 一条），
///    且指纹**变**（⇒ 摊平表真的接上了实时侧）。
///
/// # 怎么变红
///
/// 把 `with_sfz` 里那句 `self.sfz_pending.remove(&track)` 删掉 ⇒ 腿 ③ 的"表清空"红；
/// 让 `from_project` 去读那个路径 ⇒ 腿 ② 红（不存在的文件必然 `Err`）。
#[test]
fn s23_a_pending_path_changes_nothing_audible_and_is_superseded_by_with_sfz() {
    let fixture = note_project(&SFZ_NOTES);
    let bare_project = project_with_sampler(&fixture, None);
    let pending_project = project_with_sampler(&fixture, Some(SAMPLER_PATH));

    let bare = render(&bare_project, SFZ_QUANTA);
    let with_pending = render(&pending_project, SFZ_QUANTA);
    println!(
        "[engine-sfz/S23] `sampler = None`: 非零={} 指纹={} (0x{:016x})；\
         `sampler = Some({SAMPLER_PATH})`（文件不存在）: 非零={} 指纹={} (0x{:016x})",
        bare.nonzero(),
        bare.fingerprint(),
        bare.fingerprint(),
        with_pending.nonzero(),
        with_pending.fingerprint(),
        with_pending.fingerprint(),
    );
    assert!(
        bare.nonzero() > 0 && with_pending.nonzero() > 0,
        "两条臂都必须真的出声（否则'逐位相同'可能只是两边都哑）"
    );
    assert_eq!(
        with_pending.fingerprint(),
        bare.fingerprint(),
        "⭐ 待摊平项⛔ 不改渲染：`None` 与 `Some(…)` 的输出必须**逐位相同**"
    );
    assert_eq!(
        bits(&with_pending.left),
        bits(&bare.left),
        "⭐ 左声道逐位相同（指纹之外的独立见证）"
    );

    // ② 不存在的路径仍能编译成快照 ⇒ ⭐ 引擎⛔ 不读盘。
    let snapshot = EngineSnapshot::from_project(&pending_project, 1)
        .expect("⭐ 引擎不读盘 ⇒ 磁盘上不存在的 SFZ 路径也必须能编译成快照");
    assert!(snapshot.sfz().is_empty(), "⛔ 没有摊平表（引擎没读盘）");

    // ③ 控制侧（允许 I/O）解码后注入 ⇒ 摊平表**取代**待摊平项。
    let sfz = SfzInstrument::load(
        "<region> sample=a.wav key=60 pitch_keycenter=60\n".to_owned(),
        &Default::default(),
    )
    .expect("SFZ 必须解析成功");
    let sample = InMemorySample::sine(440.0, 48_000.0, SAMPLE_FRAMES);
    let plan = SfzPlan::prepare_with_samples(&sfz, &[("a.wav".to_owned(), sample)], 48_000.0)
        .expect("多采样摊平必须成功");
    let armed = snapshot.with_sfz(fixture.track, plan);
    println!(
        "[engine-sfz/S23] 注入摊平表之后：待摊平条数 = {}；摊平表条数 = {}",
        armed.sfz_pending().len(),
        armed.sfz().len()
    );
    assert!(
        armed.sfz_pending().is_empty(),
        "⭐ 摊平表**取代**待摊平项（否则控制侧会重复解码同一条轨）"
    );
    assert_eq!(armed.sfz().len(), 1, "注入一次 ⇒ 摊平表一条");
    let armed_audio = render_snapshot(armed, SFZ_QUANTA);
    assert_ne!(
        armed_audio.fingerprint(),
        bare.fingerprint(),
        "⭐ 摊平表必须真的接上实时侧（否则'取代'只是账面动作）"
    );
}

/// ⭐ S24（任务单 `#78`）：**缺样本口径不变**，且它能**经新接线**走到。
///
/// 前三条判据只证明"路径进快照"；本条证明⭐ **新接线与控制侧既有分工衔接得上**：
/// 由 `sfz_pending()` 里的路径 →（控制侧"读盘"／解析）→ 只解码 `a.wav` →
/// `prepare_with_samples` → `render_track`：缺 `missing.wav` 的那条区域
/// **`NO_SAMPLE` ⇒ 不建声部 ＋ 计数**（既有口径，`R711`／`needs-7`）。
///
/// # 判定量
///
/// * `silent_region_count() == 1`、`missing_sample_paths() == ["missing.wav"]`、
///   该区域 `sample_index == PreparedRegion::NO_SAMPLE`；
/// * 引擎侧：key=64（缺样本）那次 `render_track` ⇒ `sfz_active_voices == Some(0)`
///   ＋ 输出**逐位 0**（⛔ 不建声部、⛔ 不回退播别人的采样）；
/// * 反向对照：同一份 plan 上 key=60（有样本）**真的出声** ⇒ 上一条不是"两边都哑"。
#[test]
fn s24_a_missing_sample_is_still_silent_and_counted_through_the_new_wiring() {
    const ONE_MISSING: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n\
                               <region> sample=missing.wav key=64 pitch_keycenter=60\n";
    let fixture = note_project(&SFZ_NOTES);
    let project = project_with_sampler(&fixture, Some(SAMPLER_PATH));
    let snapshot = EngineSnapshot::from_project(&project, 1).expect("快照");

    // 控制侧（模拟"读盘"）：路径必须**先**从待摊平项取到。
    let pending = snapshot
        .pending_sfz(&fixture.track)
        .expect("⭐ 待摊平项是控制侧的唯一入口");
    assert_eq!(pending.path(), SAMPLER_PATH);
    let sfz = SfzInstrument::load(ONE_MISSING.to_owned(), &Default::default()).expect("解析");
    assert_eq!(
        sfz.referenced_sample_paths().expect("路径全集"),
        ["a.wav".to_owned(), "missing.wav".to_owned()],
        "控制侧据此知道该解码哪两份（`missing.wav` 读不到 ⇒ ⛔ 不放进提供表）"
    );
    let a = InMemorySample::sine(440.0, 48_000.0, SAMPLE_FRAMES);
    let plan = SfzPlan::prepare_with_samples(&sfz, &[("a.wav".to_owned(), a)], 48_000.0)
        .expect("多采样摊平必须成功");
    let prepared = plan.prepared();
    println!(
        "[engine-sfz/S24] silent = {}、missing = {:?}、sample_content = {}、\
         key=64 的 sample_index = {}",
        prepared.silent_region_count(),
        prepared.missing_sample_paths(),
        prepared.sample_content_label(),
        prepared.regions()[1].sample_index(),
    );
    assert_eq!(prepared.silent_region_count(), 1, "⭐ 缺样本的区域计数");
    assert_eq!(
        prepared.missing_sample_paths().to_vec(),
        vec!["missing.wav".to_owned()],
        "⭐ 缺失路径必须**具名**登记（⛔ 不静默）"
    );
    assert_eq!(
        prepared.regions()[1].sample_index(),
        PreparedRegion::NO_SAMPLE,
        "⭐ 缺样本 ⇒ `NO_SAMPLE`（⛔ 不是回退到 `samples[0]`）"
    );

    // 引擎侧：缺样本那个键**不建声部**（`sfz_active_voices == Some(0)` ＋ 逐位 0）。
    let engine_track = EntityId::new();
    let mut engine = engine_with_track(engine_track);
    let missing_note = NoteSchedule::from_sorted(vec![ScheduledNote::new(
        0,
        12_000,
        64,
        127,
        support::yeban_dsp_note_to_hz(64),
        1.0,
        48_000.0,
    )]);
    let mut missing_out = vec![0.0f32; S24_BLOCK];
    engine.render_track(
        engine_track,
        Some(&missing_note),
        Some(prepared),
        &mut missing_out,
    );
    assert_eq!(
        engine.sfz_active_voices(engine_track),
        Some(0),
        "⭐ 缺样本 ⇒ **不建声部**（池里 0 个活声部）"
    );
    assert!(
        missing_out.iter().all(|s| *s == 0.0),
        "⭐ 缺样本的区域必须**逐位静音**"
    );

    // 反向对照：同一份 plan，key=60（有样本）必须真的出声。
    // ⚠ 用一条**新的**引擎：`SynthEngine` 的每槽 `cursor` 单调前进（同 S18 每条腿
    // 各建一台的理由）⇒ 在同一个引擎上换一份调度表不会重新触发。
    let sibling_note = NoteSchedule::from_sorted(vec![ScheduledNote::new(
        0,
        12_000,
        60,
        127,
        support::yeban_dsp_note_to_hz(60),
        1.0,
        48_000.0,
    )]);
    let mut sibling_engine = engine_with_track(engine_track);
    let mut sibling_out = vec![0.0f32; S24_BLOCK];
    sibling_engine.render_track(
        engine_track,
        Some(&sibling_note),
        Some(prepared),
        &mut sibling_out,
    );
    println!(
        "[engine-sfz/S24] 反向对照 key=60：活声部 = {:?}、非零 = {}",
        sibling_engine.sfz_active_voices(engine_track),
        sibling_out.iter().filter(|s| **s != 0.0).count(),
    );
    assert!(
        sibling_engine.sfz_active_voices(engine_track).unwrap_or(0) > 0
            && sibling_out.iter().any(|s| *s != 0.0),
        "反向对照：缺的**只有** `missing.wav` ⇒ key=60 那条区域必须照常出声"
    );
}
