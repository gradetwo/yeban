//! `line/engine-drums` 的端到端判据：**每轨鼓机音源**（`crate::drums` → `crate::synth`）。
//! [ARCH-RT-001, ARCH-RT-004, ARCH-DSP-001, ARCH-DET-001]
//!
//! 本文件是 `crates/yeban-engine` 接线的**最后一件** `yeban_dsp` 器件
//! （量法：`grep -rn 'use yeban_dsp' crates/yeban-engine/src`，只数真实 `use`/`pub use`
//! 行；接线前 `drums` 在该目录里只命中 3 行注释）。
//!
//! 全部判据断言在**真实路径**上：
//!
//! - 端到端：`YebanProjectV1` → `EngineSnapshot::from_project` → `SnapshotSlot` →
//!   `EngineRuntime::process_quantum`（`support` 模块刻意不给测试专用捷径）；
//! - 逐位对账：`SynthEngine::render_track` 的输出对一台**手工驱动**的
//!   [`DrumMachine`]（同一份参数、同一串触发、同一样本位置）逐位比较。
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | D0 | **没有完整键位映射**的工程（无设备 / 只有音色旋钮 / 参数不认识 / `bypassed` / 效果器 / 外部乐器）输出与**接线前**逐位相同（三个硬编码指纹是接线前实测的） | 让"只写了音色旋钮"的设备也武装鼓机 / 发明一张默认键位表 |
//! | D1 | 快照层：`drums()` **只**收录识别出鼓机的轨道；没有设备的工程是空表 | 快照里有、实时侧没读 / 把所有 `InternalInstrument` 设备都当鼓机 |
//! | D2 | 引擎的 `DrumMachine` **就是** `yeban_dsp::drums::DrumMachine`（类型同一性 + `trigger`/`render` 函数地址同一性） | 在引擎侧留第二份鼓机实现 |
//! | D3 | **逐位忠实**：引擎路径与手工驱动的器件在 400 个量子（51 200 帧）上逐位相同 | 触发时机错 / 位置传错 / 参数投影错 / 增益折错 |
//! | D4 | 映射里没有的音高**不触发**（`drum_hits == 0`）⇒ 该轨逐位静音 | 把所有音高都当成某个鼓件 / 猜一张默认键位表 |
//! | D5 | **鼓机取代复音合成器**：同一轨武装鼓机后输出与"只有复音合成器"不同，且 `insert_*` 读数不变 | 两件音源同时渲染（叠音）/ 鼓机被接到插入链上 |
//! | D6 | 端到端：`EngineStats::drum_hits` 恰好等于**键位映射命中的**音符数；`armed_drum_slot_count` 报数 | 武装了但从不触发 / 触发不计 / 每个量子重复触发 |
//! | D7 | 确定性：同输入两次独立装配逐位相同；**同一装配**里周期性重新武装（等价快照）不改变输出，且累计触发数单调 | 引入真熵源 / 重新武装清掉在响的鼓 |
//! | D8 | 换快照：同一轨从鼓机换回复音合成器 ⇒ `armed_drums(轨)` 变 `None`、武装槽位归 0；换一套映射 ⇒ 读数跟着变 | 武装表只增不减 / 换轨不 `reset` |
//!
//! ## D0 的指纹是**接线前**的实测值
//!
//! D0 的三个 u64 由 `line/engine-drums` 在**改动任何源码之前**于本机测得
//! （同一个夹具、同一台机器、同一条产品路径）：`8204613887399841269`
//! （两个音符、无设备链）、`7993387413444691233`（`cutoff_hz = 800` /
//! `resonance = 0.3`）、`8204613887399841269`（设备链只有 `kick_tune_hz` /
//! `snare_tone_hz`，无键位映射 —— 与第一行**同一个数**）。
//! 接线后原样重测 ⇒ 三个数不变 ⇒ "未武装路径逐位不变"是**跑出来的**。
//!
//! ⚠ 这两个字面值**不是**规范常数，而是本机 libm/LLVM 的一次实现读数；
//! 它们的作用是"跨提交对账"（同一个二进制里前后两版代码给出同一个数），
//! 不是"跨架构必须相等"（跨架构对账是 `ARCH-DET-002` 的独立票）。
//!
//! ## 覆盖范围（**明说**）
//!
//! 本文件覆盖：`drums.rs` 的投影、`snapshot.rs` 的 `drums` 字段、
//! `synth.rs` 的快照边界武装与逐样本触发/渲染、`rt.rs` 的读数。
//! 它**不**覆盖：`yeban-dsp` 内部四个配方的正确性
//! （那是 `crates/yeban-dsp/tests/drums_render.rs` 的职责），也**不**覆盖
//! `tests/synth_rt_zero_alloc.rs` 的运行期零分配（那是另一个 `harness = false` 目标）。

mod support;

use support::{MixSpec, NoteSpec, note_project, render, render_with, tuned_project};
use yeban_engine::drums::{DRUM_SLOTS, DrumHit, DrumMachine, DrumNoteMap, DrumVoice, DrumsParams};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_engine::synth::{NoteSchedule, SynthEngine};
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, ParameterValue, YebanProjectV1};

/// D0 的夹具：两个音符、20 个量子（与 `support::render` 的默认形状一致）。
const BASELINE_NOTES: [NoteSpec; 2] = [
    NoteSpec::at(0, 480, 60, 100),
    NoteSpec::at(240, 480, 67, 100),
];

/// D0：**接线前**实测的三个指纹（见模块文档）。
const BASELINE_PLAIN: u64 = 8_204_613_887_399_841_269;
/// `cutoff_hz = 800` / `resonance = 0.3`。
const BASELINE_TUNED: u64 = 7_993_387_413_444_691_233;
/// 设备链只有音色旋钮、没有键位映射 ⇒ 与 `BASELINE_PLAIN` **同一个数**。
const BASELINE_KNOBS_ONLY: u64 = 8_204_613_887_399_841_269;

/// 上面三个字面值的**测量平台**：`aarch64` + `macos`。
///
/// L1 只承诺"相同 OS 与 CPU 架构 + 纯 Rust libm"下的逐位一致
/// `[ARCH-DET-001]`（规范正文见 `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md`
/// 的"L1 级确定性"一条）；跨架构的位级哈希差异是 L2 的领域（`max|Δ| < 1e-6`），
/// 拿它当本判据的失败就是**假红**。
const MEASURED_ARCH: &str = "aarch64";
/// 见 [`MEASURED_ARCH`]。
const MEASURED_OS: &str = "macos";

/// 本机（**编译期事实**）是否就是上面那个测量平台。
///
/// 取 `cfg!` 而不是运行期探测：`target_arch`/`target_os` 是编译期确定的，
/// 比运行期比较更不容易想错。
const ON_MEASURED_PLATFORM: bool = cfg!(target_arch = "aarch64") && cfg!(target_os = "macos");

/// 本平台的 `arch/os`（只用于打印，让人一眼看出为什么跳过）。
const CURRENT_PLATFORM: &str = if cfg!(target_arch = "aarch64") {
    if cfg!(target_os = "macos") {
        "aarch64/macos"
    } else {
        "aarch64/非macos"
    }
} else if cfg!(target_arch = "x86_64") {
    "x86_64/非aarch64"
} else {
    "非aarch64"
};

/// 判据 D0 的字面值对账：**同平台硬红，异平台如实跳过并点名原因**。
///
/// 返回 `false` 表示"本轮没有判定"（异平台）—— **不等于通过**。
///
/// 为什么要有这一层：`BASELINE_*` 是某一台机器上的 libm/LLVM 实现读数，
/// 不是规范常数。在 `x86_64` 的 CI runner 上拿它判红，量到的是架构差异而不是缺陷。
/// 因此这里照 `crates/yeban-render/tests/l1_digest_parity.rs` 的既有形态
/// （异平台 SKIP + 打印原因、同平台仍硬红）改成本平台的运行时判定，
/// 而**不是**去改 `BASELINE_*` 迁就 `x86_64`。
fn check_fingerprint(label: &str, actual: u64, expected: u64) -> bool {
    check_fingerprint_on(
        label,
        actual,
        expected,
        ON_MEASURED_PLATFORM,
        CURRENT_PLATFORM,
    )
}

/// [`check_fingerprint`] 的**纯函数形态**：把"是否同平台"当作参数传进来。
///
/// 这样"同平台硬红 / 异平台跳过"这条规则本身可被单独对账
/// （见 `the_same_platform_rule_is_hard_red_and_foreign_platforms_only_skip`），
/// 而不是靠 `cfg!` 恰好命中本机来"碰运气验证"。
fn check_fingerprint_on(
    label: &str,
    actual: u64,
    expected: u64,
    on_measured_platform: bool,
    current_platform: &str,
) -> bool {
    if on_measured_platform {
        assert_eq!(
            actual, expected,
            "{label}: 同平台 ({current_platform} == 测量平台 {MEASURED_ARCH}/{MEASURED_OS}) \
             ⇒ 指纹必须逐位等于实测值; 不同就是真缺陷"
        );
        return true;
    }
    println!(
        "[engine-drums/D0] SKIP (未判定, 不是通过): {label} 的字面值 {expected} \
         (0x{expected:016x}) 实测于 {MEASURED_ARCH}/{MEASURED_OS}, 而本机是 {current_platform}; \
         跨架构的位级差异是 L2 的领域 (max|Δ| < 1e-6), 不是本判据的失败"
    );
    false
}

/// 判据用的**键位映射**（五个鼓件各一个音高）。
const NOTES: [(&str, f32); 5] = [
    ("kick_note", 36.0),
    ("snare_note", 38.0),
    ("closed_hat_note", 42.0),
    ("open_hat_note", 46.0),
    ("clap_note", 39.0),
];

/// 四个音高**全部在映射里**的音符（单位：tick；1 tick = 25 样本）。
///
/// 四个起音落在 0 / 12 000 / 24 000 / 36 000 帧，全部在 400 个量子（51 200 帧）
/// 的窗口内；同时发声数 ≤ 4 < [`DRUM_SLOTS`] ⇒ **不触发窃取**
/// （D3 的手工参照因此可以把触发一次性发完，见该判据的文档）。
const DRUM_NOTES: [NoteSpec; 4] = [
    NoteSpec::at(0, 480, 36, 127),
    NoteSpec::at(480, 480, 38, 110),
    NoteSpec::at(960, 480, 42, 100),
    NoteSpec::at(1440, 480, 39, 90),
];

/// 渲染量子数：400 × 128 = 51 200 帧 ≈ 1.067 s @48 kHz（覆盖上面四个起音）。
const QUANTA: usize = 400;

/// 一张**内置乐器**设备（鼓机的设备形状）。
fn instrument(params: &[(&str, f32)]) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: "Yeban Drums".to_owned(),
        kind: DeviceKind::InternalInstrument,
        bypassed: false,
        params: params
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

/// 把 [`NOTES`] 与额外旋钮拼成一台完整的鼓机设备。
fn kit(params: &[(&str, f32)]) -> DeviceDefinition {
    let mut all = NOTES.to_vec();
    all.extend_from_slice(params);
    instrument(&all)
}

/// 把设备链接到夹具的那条 MIDI 轨上（**只改夹具**，不改模型层）。
fn mount(project: &mut YebanProjectV1, track: EntityId, devices: Vec<DeviceDefinition>) {
    let entry = project
        .tracks
        .get_mut(&track)
        .expect("夹具里必须有那条 MIDI 轨");
    entry.devices = devices;
}

/// 投影一份**手工的**参数（判据的独立参照用；不经过模型 → 快照）。
fn reference_params() -> DrumsParams {
    DrumsParams::from_devices(&[kit(&[("kick_tune_hz", 48.0), ("master_level", 0.8)])])
        .expect("五个键位写全 ⇒ 必须武装")
}

/// D0：**没有完整键位映射**的工程输出与接线前逐位相同（模块文档 §3.1 的口径）。
///
/// 六种输入的唯一差别是设备链：无设备 / 空设备 / 只有音色旋钮 / 参数不认识 /
/// `bypassed` / 效果器（不是音源）/ 外部乐器。**全部六种**都必须与第一行同解。
#[test]
fn projects_without_a_complete_note_map_are_bit_identical() {
    let fixture = note_project(&BASELINE_NOTES);
    let track = fixture.track;
    let bare = render(&fixture.project, 20);

    let mut empty_project = fixture.project.clone();
    mount(&mut empty_project, track, vec![instrument(&[])]);
    let empty = render(&empty_project, 20);

    let mut knobs_project = fixture.project.clone();
    mount(
        &mut knobs_project,
        track,
        vec![instrument(&[
            ("kick_tune_hz", 45.0),
            ("snare_tone_hz", 200.0),
        ])],
    );
    let knobs = render(&knobs_project, 20);

    let mut unknown_project = fixture.project.clone();
    mount(
        &mut unknown_project,
        track,
        vec![instrument(&[("filter_slope", 0.7), ("drum_kit", 1.0)])],
    );
    let unknown = render(&unknown_project, 20);

    let mut bypassed_project = fixture.project.clone();
    let mut bypassed = kit(&[]);
    bypassed.bypassed = true;
    mount(&mut bypassed_project, track, vec![bypassed]);
    let bypassed_render = render(&bypassed_project, 20);

    // 效果器不是音源、外部乐器由插件宿主负责：同一组名字都不认。
    let mut effect_project = fixture.project.clone();
    let mut effect = kit(&[]);
    effect.kind = DeviceKind::InternalEffect;
    mount(&mut effect_project, track, vec![effect]);
    let effect_render = render(&effect_project, 20);

    let mut external_project = fixture.project.clone();
    let mut external = kit(&[]);
    external.kind = DeviceKind::ExternalInstrument;
    mount(&mut external_project, track, vec![external]);
    let external_render = render(&external_project, 20);

    // 第一行：接线前实测的字面值。
    println!(
        "[engine-drums/D0] 无设备链: 帧={} 非零={} 峰值={} 指纹={} (0x{BASELINE_PLAIN:016x})",
        bare.frames(),
        bare.nonzero(),
        bare.peak(),
        bare.fingerprint()
    );
    check_fingerprint("无设备链", bare.fingerprint(), BASELINE_PLAIN);
    assert!(bare.peak() > 0.0, "夹具必须真的出声（否则逐位相同是空转）");
    assert_eq!(bare.stats.drum_hits, 0, "没有鼓机设备 ⇒ 鼓击数必须为 0");

    for (label, other) in [
        ("空设备", &empty),
        ("只有音色旋钮（无键位映射）", &knobs),
        ("参数不认识", &unknown),
        ("bypassed 的鼓机", &bypassed_render),
        ("效果器（不是音源）", &effect_render),
        ("外部乐器", &external_render),
    ] {
        assert_eq!(
            other.fingerprint(),
            bare.fingerprint(),
            "{label}: 输出必须与没有设备链时逐位相同"
        );
        assert_eq!(other.left_bits(), bare.left_bits(), "{label}: 左声道逐位");
        assert_eq!(other.stats.drum_hits, 0, "{label}: 鼓击数必须为 0");
    }
    check_fingerprint("只有旋钮", knobs.fingerprint(), BASELINE_KNOBS_ONLY);

    // 第二个字面值：复音合成器路径（`cutoff_hz`）。
    let tuned = tuned_project(&BASELINE_NOTES, MixSpec::tone(800.0, 0.3));
    let tuned_render = render(&tuned.project, 20);
    println!(
        "[engine-drums/D0] tuned(800,0.3): 指纹={} (0x{BASELINE_TUNED:016x})",
        tuned_render.fingerprint()
    );
    check_fingerprint("滤波器路径", tuned_render.fingerprint(), BASELINE_TUNED);
}

/// D1：快照层只收录**识别出鼓机**的轨道；没有设备的工程是空表。
#[test]
fn the_snapshot_only_carries_recognised_drum_tracks() {
    let fixture = note_project(&DRUM_NOTES);
    let track = fixture.track;
    let bare = EngineSnapshot::from_project(&fixture.project, 1).expect("快照");
    assert!(bare.drums().is_empty(), "没有设备链 ⇒ 不得有鼓机");
    assert!(bare.drum_params(&track).is_none());

    let mut armed_project = fixture.project.clone();
    mount(&mut armed_project, track, vec![kit(&[])]);
    let armed = EngineSnapshot::from_project(&armed_project, 2).expect("快照");
    assert_eq!(armed.drums().len(), 1, "恰好一条轨武装鼓机");
    let params = armed.drum_params(&track).expect("本轨必须是鼓机");
    assert_eq!(params.notes().as_array(), [36, 38, 42, 46, 39]);
    assert_eq!(
        params.kit().kick.tune_hz.to_bits(),
        55.0f32.to_bits(),
        "未写的字段取器件默认值"
    );

    // 映射不完整（缺一个键位）⇒ 不是鼓机。
    let mut partial_project = fixture.project.clone();
    mount(&mut partial_project, track, vec![instrument(&NOTES[..4])]);
    let partial = EngineSnapshot::from_project(&partial_project, 3).expect("快照");
    assert!(partial.drums().is_empty(), "缺一个键位 ⇒ 不得武装");
}

/// D2：引擎的鼓机**就是** `yeban_dsp::drums` 的鼓机（类型 + 函数地址同一性）。
#[test]
fn the_engine_drum_machine_is_the_dsp_drum_machine() {
    // 类型同一性：同一个类型可以从两条路径写出来并互相赋值。
    let engine: DrumMachine<DRUM_SLOTS> = DrumMachine::new(48_000);
    let dsp: yeban_dsp::drums::DrumMachine<DRUM_SLOTS> = yeban_dsp::drums::DrumMachine::new(48_000);
    assert_eq!(engine.slots(), dsp.slots());

    // 函数地址同一性：`trigger` 与 `render` 必须是同一份代码。
    let engine_trigger: fn(&mut DrumMachine<DRUM_SLOTS>, DrumHit) = DrumMachine::trigger;
    let dsp_trigger: fn(&mut yeban_dsp::drums::DrumMachine<DRUM_SLOTS>, yeban_dsp::drums::DrumHit) =
        yeban_dsp::drums::DrumMachine::trigger;
    assert_eq!(
        engine_trigger as usize, dsp_trigger as usize,
        "trigger 必须是同一份实现"
    );
    let engine_render: fn(&mut DrumMachine<DRUM_SLOTS>, u64, &mut [f32]) = DrumMachine::render;
    let dsp_render: fn(&mut yeban_dsp::drums::DrumMachine<DRUM_SLOTS>, u64, &mut [f32]) =
        yeban_dsp::drums::DrumMachine::render;
    assert_eq!(
        engine_render as usize, dsp_render as usize,
        "render 必须是同一份实现"
    );
}

/// D3：引擎路径与**手工驱动**的器件逐位相同（400 个量子 = 51 200 帧）。
///
/// 手工参照的做法：
///
/// 1. 一次性把**四个音高**对应的 `DrumHit` 全部触发（每个音符一次，
///    位置取该音符的 `start_sample`、增益取 `ScheduledNote::gain`）；
/// 2. 逐量子调用 `render(position, out)`，`position` 与引擎的播放头同步。
///
/// ⚠ 夹具的四个起音最多 4 个槽位同时占用（< [`DRUM_SLOTS`] = 16）⇒ **不发生窃取**
/// ⇒ "提前触发"与"到点触发"在器件契约下同解（[`DrumMachine::render`] 只在
/// `now >= start_sample` 时才让槽位发声）。若夹具改成会窃取，本判据的参照必须
/// 改成与引擎逐量子同步的触发（否则窃取对象不同）。
#[test]
fn the_engine_drum_path_matches_a_hand_driven_machine_bit_for_bit() {
    let params = reference_params();
    let track = EntityId::new();
    let mut engine = SynthEngine::new(48_000);
    engine.begin_snapshot(48_000, &[track], [], [(&track, &params)]);

    // 手工参照用的调度表：48 kHz、1 tick = 25 样本、增益固定 0.5
    //（`ScheduledNote::new` 的 `gain` 与力度无关，判据直接给值 ⇒ 两侧同源）。
    let schedule = NoteSchedule::from_sorted(
        DRUM_NOTES
            .iter()
            .map(|spec| {
                support::scheduled(
                    spec.start_tick * 25,
                    (spec.start_tick + spec.duration_ticks) * 25,
                    spec.pitch,
                    0.5,
                    48_000.0,
                )
            })
            .collect(),
    );

    let mut reference = DrumMachine::<DRUM_SLOTS>::new(48_000);
    reference.set_params(params.kit());
    for note in schedule.notes() {
        let voice = params
            .notes()
            .voice_for(note.pitch())
            .expect("夹具里的音高都必须在映射里");
        reference.trigger(DrumHit::new(voice, note.start_sample(), note.gain()));
    }

    let mut out = vec![0.0f32; 128];
    let mut reference_block = vec![0.0f32; 128];
    let mut rendered = Vec::with_capacity(QUANTA * 128);
    let mut reference_samples = Vec::with_capacity(QUANTA * 128);
    for _ in 0..QUANTA {
        engine.render_track(track, Some(&schedule), &mut out);
        let position = engine.position();
        reference.render(position, &mut reference_block);
        rendered.extend_from_slice(&out);
        reference_samples.extend_from_slice(&reference_block);
        engine.advance(128);
    }

    let engine_bits: Vec<u32> = rendered.iter().map(|s| s.to_bits()).collect();
    let reference_bits: Vec<u32> = reference_samples.iter().map(|s| s.to_bits()).collect();
    println!(
        "[engine-drums/D3] 引擎 vs 手工器件: 帧={} 引擎触发={} 参照触发={} 参照窃取={}",
        rendered.len(),
        engine.drum_hits(),
        reference.triggers(),
        reference.voice_steals()
    );
    assert_eq!(engine.drum_hits(), 4, "四个音高都在映射里 ⇒ 恰好四次触发");
    assert_eq!(reference.triggers(), 4);
    assert_eq!(
        reference.voice_steals(),
        0,
        "夹具必须不发生窃取（见本判据的文档）"
    );
    assert!(rendered.iter().any(|sample| *sample != 0.0), "必须真的出声");
    assert_eq!(
        engine_bits, reference_bits,
        "引擎路径必须与手工参照逐位相同"
    );
    assert_eq!(engine.armed_drum_slots(), 1);
    assert_eq!(engine.drum_params(track).expect("武装").kit(), params.kit());
}

/// D4：映射里**没有**的音高不触发任何鼓件 ⇒ 该轨逐位静音。
#[test]
fn pitches_outside_the_map_do_not_trigger_anything() {
    let fixture = note_project(&[
        NoteSpec::at(0, 480, 60, 127),
        NoteSpec::at(480, 480, 64, 100),
    ]);
    let track = fixture.track;
    let mut unmapped_project = fixture.project.clone();
    mount(&mut unmapped_project, track, vec![kit(&[])]);
    let unmapped = render(&unmapped_project, 20);

    println!(
        "[engine-drums/D4] 映射外的音高: 非零={} 峰值={} 鼓击={}",
        unmapped.nonzero(),
        unmapped.peak(),
        unmapped.stats.drum_hits
    );
    assert_eq!(unmapped.stats.drum_hits, 0, "映射里没有的音高不许触发");
    assert_eq!(unmapped.nonzero(), 0, "没有触发 ⇒ 逐位静音");
    assert_eq!(unmapped.peak(), 0.0);

    // 对照：同一份工程、只把音高换成映射内的 ⇒ 必须出声
    //（否则上面那句"逐位静音"可能只是因为整条链没接线）。
    let control = note_project(&[NoteSpec::at(0, 480, 36, 127)]);
    let control_track = control.track;
    let mut control_project = control.project.clone();
    mount(&mut control_project, control_track, vec![kit(&[])]);
    let armed = render(&control_project, 20);
    assert_eq!(armed.stats.drum_hits, 1, "映射内的音高必须触发一次");
    assert!(armed.peak() > 0.0, "对照臂必须出声");
}

/// D5：鼓机**取代**复音合成器（不是叠加），且不经过插入链。
#[test]
fn the_drum_machine_replaces_the_poly_synth_on_that_track() {
    let fixture = note_project(&DRUM_NOTES);
    let track = fixture.track;

    // 同一轨、同一批音符：设备链换成"只有截止频率的复音合成器"。
    let mut synth_project = fixture.project.clone();
    mount(
        &mut synth_project,
        track,
        vec![instrument(&[("cutoff_hz", 800.0), ("resonance", 0.3)])],
    );
    let synth_render = render(&synth_project, QUANTA);

    let mut drum_project = fixture.project.clone();
    mount(&mut drum_project, track, vec![kit(&[])]);
    let drum_render = render(&drum_project, QUANTA);

    println!(
        "[engine-drums/D5] 复音: 鼓击={} 峰值={} 指纹={:#018x} | 鼓机: 鼓击={} 峰值={} 指纹={:#018x}",
        synth_render.stats.drum_hits,
        synth_render.peak(),
        synth_render.fingerprint(),
        drum_render.stats.drum_hits,
        drum_render.peak(),
        drum_render.fingerprint()
    );
    assert_eq!(synth_render.stats.drum_hits, 0, "复音合成器轨不得产生鼓击");
    assert_eq!(drum_render.stats.drum_hits, 4, "四个映射内的音符各触发一次");
    assert_ne!(
        drum_render.fingerprint(),
        synth_render.fingerprint(),
        "鼓机必须改变这一轨的输出"
    );
    assert!(
        drum_render.peak() > 0.0 && synth_render.peak() > 0.0,
        "两臂都必须出声（否则'不同'是空转）"
    );
    // 鼓机是**音源**，不是插入器件：插入链的读数必须全 0。
    assert_eq!(drum_render.stats.insert_strip_frames, 0);
    assert_eq!(drum_render.stats.insert_gain_reductions, 0);
    assert_eq!(drum_render.stats.insert_reverb_frames, 0);
    assert_eq!(drum_render.stats.voice_steals, 0, "鼓机轨不得占用复音声部");
}

/// D6：端到端读数 —— 武装槽位数与鼓击数。
#[test]
fn the_end_to_end_readouts_report_the_armed_kit_and_the_hits() {
    let fixture = note_project(&DRUM_NOTES);
    let track = fixture.track;
    let mut project = fixture.project.clone();
    mount(&mut project, track, vec![kit(&[])]);
    // ⚠ 武装发生在**第一个量子内部**（`render_block` 的快照边界）⇒ 在量子 1 的
    // 边界上读才是"武装之后"的值（量子 0 的边界上还没武装）。
    let armed_slots = std::cell::Cell::new(0usize);
    let armed_kit = std::cell::Cell::new(false);
    let rendered = render_with(&project, 40, 7, |quantum, rig| {
        if quantum == 1 {
            armed_slots.set(rig.runtime.armed_drum_slot_count());
            armed_kit.set(rig.runtime.armed_drums(&track).is_some());
        }
    });

    println!(
        "[engine-drums/D6] 鼓击={} 武装槽位={} 武装本轨={} 非零={} 峰值={}",
        rendered.stats.drum_hits,
        armed_slots.get(),
        armed_kit.get(),
        rendered.nonzero(),
        rendered.peak()
    );
    assert_eq!(armed_slots.get(), 1, "恰好一轨武装鼓机");
    assert!(armed_kit.get(), "本轨的武装读数必须是 Some");
    // 40 个量子 = 5 120 帧 ⇒ 只有 tick 0 的那个音符（第 0 帧）起音。
    assert_eq!(rendered.stats.drum_hits, 1, "窗口内只有一个起音");
    assert!(rendered.nonzero() > 0, "必须真的出声");
    assert!(rendered.peak() > 0.0);

    // 快照层：武装的那个数必须逐字段等于投影（判据不必从音频反推）。
    let snapshot = EngineSnapshot::from_project(&project, 7).expect("快照");
    let params = snapshot.drum_params(&track).expect("本轨必须是鼓机");
    assert_eq!(params.notes().kick(), 36);
    assert_eq!(params.kit().master_level.to_bits(), 1.0f32.to_bits());
}

/// D7：确定性与"重新武装不改变输出"（同一装配内周期性发布等价快照）。
#[test]
fn the_drum_path_is_deterministic_across_rigs_and_rearmament() {
    let fixture = note_project(&DRUM_NOTES);
    let track = fixture.track;
    let mut project = fixture.project.clone();
    mount(&mut project, track, vec![kit(&[("kick_tune_hz", 48.0)])]);

    let first = render(&project, QUANTA);
    let second = render(&project, QUANTA);
    assert_eq!(
        first.fingerprint(),
        second.fingerprint(),
        "两次独立装配逐位相同"
    );
    assert_eq!(first.left_bits(), second.left_bits());

    // 同一个装配里每 10 个量子发布一份**等价**快照（重新武装路径）。
    let rearmed = render_with(&project, QUANTA, 11, |quantum, rig| {
        if quantum % 10 == 0 {
            rig.publish_equivalent(&project, 100 + quantum as u64);
        }
    });
    println!(
        "[engine-drums/D7] 单次={:#018x} 等价快照重武装={:#018x} 鼓击={}",
        first.fingerprint(),
        rearmed.fingerprint(),
        rearmed.stats.drum_hits
    );
    assert_eq!(
        rearmed.fingerprint(),
        first.fingerprint(),
        "等价快照重新武装不得改变输出（参数逐位相同 ⇒ 器件状态不动）"
    );
    assert_eq!(rearmed.stats.drum_hits, first.stats.drum_hits);
}

/// D8：换快照 —— 同一轨从鼓机换回复音合成器，以及换一套键位映射。
#[test]
fn swapping_the_kit_across_snapshots_is_reported() {
    let fixture = note_project(&DRUM_NOTES);
    let track = fixture.track;
    let mut drum_project = fixture.project.clone();
    mount(&mut drum_project, track, vec![kit(&[])]);
    let mut synth_project = fixture.project.clone();
    mount(
        &mut synth_project,
        track,
        vec![instrument(&[("cutoff_hz", 800.0)])],
    );

    // 前 10 个量子是鼓机，之后换成复音合成器。
    let switched = render_with(&drum_project, 20, 3, |quantum, rig| {
        if quantum == 10 {
            rig.publish_equivalent(&synth_project, 4);
        }
    });
    println!(
        "[engine-drums/D8] 换回复音: 鼓击={} 非零={}",
        switched.stats.drum_hits,
        switched.nonzero()
    );
    assert_eq!(
        switched.stats.drum_hits, 1,
        "只有换快照之前的那一个起音落在鼓机上"
    );
    assert!(
        switched.stats.voice_steals > 0 || switched.stats.notes_triggered >= 1,
        "换回之后复音合成器必须接手（音符仍被触发）"
    );

    // 直接读武装表：换回复音合成器之后必须归 0 / None。
    let snapshot = EngineSnapshot::from_project(&synth_project, 4).expect("快照");
    assert!(snapshot.drums().is_empty(), "复音合成器工程不得有鼓机");

    // 换一套映射：同一个八度里的音高整体上移 12 ⇒ 读数跟着变。
    let mut shifted_project = fixture.project.clone();
    mount(
        &mut shifted_project,
        track,
        vec![kit(&[
            ("kick_note", 48.0),
            ("snare_note", 50.0),
            ("closed_hat_note", 54.0),
            ("open_hat_note", 58.0),
            ("clap_note", 51.0),
        ])],
    );
    let shifted = render(&shifted_project, 20);
    assert_eq!(
        shifted.stats.drum_hits, 0,
        "映射整体上移之后，夹具里的音高一个都不在映射里"
    );
    let shifted_snapshot = EngineSnapshot::from_project(&shifted_project, 5).expect("快照");
    assert_eq!(
        shifted_snapshot
            .drum_params(&track)
            .expect("仍然是鼓机")
            .notes()
            .kick(),
        48
    );
    // 映射真的走到实时侧：`DrumNoteMap::voice_for` 的读数与投影一致。
    let map = DrumNoteMap::new(48, 50, 54, 58, 51);
    assert_eq!(map.voice_for(48), Some(DrumVoice::Kick));
    assert_eq!(map.voice_for(36), None);
}

/// 临时把 panic hook 换成静默，离开作用域时**一定**还原。
///
/// 只服务下面那条"故意 panic 的负面对照"：`catch_unwind` 抓住了 panic，
/// 但默认 hook 仍会往 stderr 打一条看起来像失败的记录。用 RAII 还原，
/// 保证即使中途再 panic 也不会把 hook 永久留在静默状态。
struct SilentPanic {
    previous: Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>,
}

impl SilentPanic {
    fn install() -> Self {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        Self { previous }
    }
}

impl Drop for SilentPanic {
    fn drop(&mut self) {
        std::panic::set_hook(core::mem::replace(&mut self.previous, Box::new(|_| {})));
    }
}

/// D0 的**平台规则本身**：同平台硬红、异平台只跳过（**不是**通过）。
///
/// 为什么单独钉这一条：`check_fingerprint` 在 `x86_64` 的 CI 上走的是"跳过"分支，
/// 那条分支永远不会被 D0 的主判据执行到 —— 于是"异平台不许冒充通过"这件事
/// 靠主判据是**验证不到**的。这里用纯函数的两个方向把它钉死：
///
/// 1. 同平台 + 指纹不同 ⇒ `assert_eq!` 必须**真的红**（负面对照，
///    防止"改了期望值换绿"这类假绿）；
/// 2. 异平台 + 指纹不同 ⇒ 只返回 `false` 并打印点名的 SKIP，
///    **不得** `panic`（跨架构位差是 L2 的领域，判红就是本票要消灭的假红）。
#[test]
fn the_same_platform_rule_is_hard_red_and_foreign_platforms_only_skip() {
    let expected = BASELINE_PLAIN;
    // 反向的前提：注入用的"坏指纹"必须真的与期望值不同，否则这一条是空转。
    let injected = expected ^ 0x0000_0000_0000_0001;

    // 1) 同平台 ⇒ 硬红。用 `catch_unwind` 抓住它，证明"红色真的被触发"。
    //    这里**故意**让它 panic，因此临时换成静默 hook：不然那条 panic 会被读成
    //    "测试失败了"，而它恰恰是判据生效的证据。
    let silence = SilentPanic::install();
    let same_platform = std::panic::catch_unwind(|| {
        check_fingerprint_on("注入", injected, expected, true, "aarch64/macos")
    });
    drop(silence);
    assert!(
        same_platform.is_err(),
        "同平台且指纹不符必须 panic(红); 没红说明门禁被削弱了"
    );

    // 2) 同平台且相符 ⇒ 绿（正向对照：硬红不是"永远红"）。
    assert!(
        check_fingerprint_on("正向", expected, expected, true, "aarch64/macos"),
        "同平台相符必须通过"
    );

    // 3) 异平台 ⇒ 只跳过并点名原因, 不许 panic。
    let foreign = std::panic::catch_unwind(|| {
        check_fingerprint_on("注入", injected, expected, false, "x86_64/非aarch64")
    });
    assert!(
        foreign.is_ok(),
        "异平台不得判红 —— 跨架构的位级差异是 L2 的领域, 判红就是假红"
    );
    assert!(
        !check_fingerprint_on("注入", injected, expected, false, "x86_64/非aarch64"),
        "异平台必须返回 false（未判定）, 不许当成通过"
    );

    // 4) 常量自洽：测量平台就是本判据点名的那一对 `arch/os`。
    assert_eq!(MEASURED_ARCH, "aarch64");
    assert_eq!(MEASURED_OS, "macos");
    assert_eq!(
        ON_MEASURED_PLATFORM,
        cfg!(target_arch = "aarch64") && cfg!(target_os = "macos"),
        "`ON_MEASURED_PLATFORM` 必须就是编译期平台事实"
    );
}
