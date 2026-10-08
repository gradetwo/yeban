//! `line/engine-reverb` 的端到端判据：**每轨插入混响**（`crate::insert` → `rt`）。
//! [ARCH-RT-001, ARCH-DET-001, ROAD-M2-006]
//!
//! 本文件与本 crate 的 `tests/compressor_insert.rs`（裸压缩器）与
//! `tests/channel_strip_insert.rs`（通道条）是**同一条链的第三件器件**：
//! `c792fdc` 接压缩器、`db1850f` 把槽位换成通道条、本票在**同一条链**上加混响
//! （`yeban_dsp::reverb`，Freeverb 拓扑）。
//!
//! 全部判据断言在**真实渲染路径**产出的样本上：
//! `YebanProjectV1` → `EngineSnapshot::from_project` → `SnapshotSlot` →
//! `EngineRuntime::process_quantum`。`support` 模块刻意不给测试专用捷径。
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | R0 | **没有已识别混响参数**的四种口径（参数为空 / 参数不认识（含 `reverb_mix` 与 `reverb_width`）/ `bypassed` / 内置乐器）与"没有设备链"逐字节相同 | 收下 `reverb_mix` 或 `reverb_width` / 对"没有参数的设备"也挂混响 / bypass 后偷偷取默认参数 |
//! | R1 | 命中的设备**真的武装进实时侧**（`armed_reverb(轨)` 逐字段逐位等于快照投影）＋ `EngineStats::insert_reverb_frames` 等于渲染帧数 | 快照里有、实时侧没读（`armed_reverb` 为 `None` / 计数恒 0） |
//! | R2 | **混响真的加上了尾巴**：音符结束之后的窗口里，未武装的渲染**逐位静音**而武装的渲染有能量 | 混响被接到不参与混音的地方 / 只有投影没有处理 |
//! | R3 | `reverb_wet` 与 `reverb_size` **方向正确**：湿声与衰减变长都让尾窗能量单调上升 | 参数投影到错误的字段 / 器件参数没进器件 |
//! | R4 | **确定性**：两次独立装配逐位相同；同一装配里周期性发布等价快照（重新武装）也不改变输出 | 引入真熵源 / 重新武装清空各级状态 |
//! | R5 | **不同量子数（块切分）**下前缀逐位相同（[ARCH-DET-001] 的引擎侧对账） | 用了跨量子残留的临时缓冲 / 量子边界上有一次性副作用 |
//! | R6 | 引擎的 `Reverb` **就是** `yeban_dsp::reverb::Reverb`（类型 + `new`/`set_params`/`process` 函数地址同一性 ＋ **零延迟**契约） | 在引擎侧留第二份混响实现 / 器件带延迟却登记 0 |
//! | R7 | 第一个**含已识别混响参数**的效果器是唯一来源；更早的无关效果器不参与、更晚的不覆盖；只带通道条参数的设备**不**产生混响 | 取"最后一个"或"任意一个"设备 / 把通道条参数当混响参数 |
//! | R8 | `reverb_wet = 0` 是**逐位直通**（器件自己的 `mix ≤ 1e-4` 守卫） | 把 `is_active()` 守卫删掉 |
//! | R9 | **换采样率的快照不武装混响**（延迟线不在音频线程重建）：`insert_reverb_rate_rejects` 累加、槽位归零、输出回到未武装的逐位读数 | 在快照边界调用 `Reverb::set_sample_rate`（那会在音频线程分配 + 释放） |
//!
//! ## R0 的跨提交对账（**不硬编码哈希**）
//!
//! R0 把未武装的原始样本落盘：`$TMPDIR/yeban-reverb-unarmed.raw`
//! （交错小端 `f32`）。判据刻意**不**把哈希写进源码：那会把"本机 libm 的某一位"
//! 当成规范常数。实测读数见本票的交付报告（注释里只写"怎么量"，不写"量到多少"）。
//!
//! ## 覆盖范围（**明说**）
//!
//! 本文件覆盖：`insert.rs` 的混响投影、`snapshot.rs` 的插入链字段、
//! `rt.rs` 的**构造期预分配** / **快照边界武装** / **逐样本调用** / **采样率守卫**。
//! 它**不**覆盖 `yeban-dsp` 内部混响算法的正确性（那是
//! `crates/yeban-dsp/src/reverb.rs` 的 8 条单元判据的职责），也不覆盖 `drums`
//! ——它仍未接线。`reverb_width` 是**刻意未接线**的（单声道插入点，见
//! `crate::insert` 模块文档 §8.1）。

mod support;

use std::ops::Range;
use std::path::Path;

use support::{NoteSpec, note_project, render, render_with, rms_peak};
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, ParameterValue, YebanProjectV1};

/// 判据用的栅格：120 BPM / 48 kHz / 960 PPQ ⇒ 1 tick = 25 样本（见 `support` 模块文档）。
///
/// 音符在 tick 2880 结束 = 样本 72,000（1.5 s）⇒ 尾巴窗口 [96_000, 144_000) 落在
/// "所有音符的包络都已释放完"之后（R2 用未武装的渲染证明那一点）。
const NOTES: [NoteSpec; 4] = [
    NoteSpec::at(0, 480, 60, 127),
    NoteSpec::at(480, 480, 64, 110),
    NoteSpec::at(960, 960, 67, 100),
    NoteSpec::at(1920, 960, 72, 90),
];

/// 渲染量子数：2,400 × 128 = 307,200 帧 = 6.4 s @48 kHz。
///
/// 比通道条的 400 量子长得多：混响的尾巴（`size = 1.0` 时反馈 0.94）衰减到
/// 可忽略需要数秒，尾窗必须在**尾巴还听得见**的位置取样。
const QUANTA: usize = 2_400;

/// 尾巴窗口（样本下标，左声道）：2.0 s – 3.0 s，落在最后一个音符结束（1.5 s）之后。
const TAIL: Range<usize> = 96_000..144_000;

/// 一张效果器设备（`InternalEffect`）。
fn effect(name: &str, params: &[(&str, f32)]) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: name.to_owned(),
        kind: DeviceKind::InternalEffect,
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

/// 把设备链接到夹具的那条 MIDI 轨上（**只改夹具**，不改模型层）。
fn mount(project: &mut YebanProjectV1, track: EntityId, devices: Vec<DeviceDefinition>) {
    let entry = project
        .tracks
        .get_mut(&track)
        .expect("夹具里必须有那条 MIDI 轨");
    entry.devices = devices;
}

/// 把交错样本按小端 `f32` 落盘，返回 `(路径, 字节数)`（与 C0/C8 同款）。
fn dump(name: &str, left: &[f32], right: &[f32]) -> (std::path::PathBuf, usize) {
    let mut bytes = Vec::with_capacity(left.len() * 8);
    for index in 0..left.len() {
        bytes.extend_from_slice(&left[index].to_le_bytes());
        bytes.extend_from_slice(&right[index].to_le_bytes());
    }
    let path = Path::new(&std::env::temp_dir()).join(name);
    std::fs::write(&path, &bytes).expect("原始样本必须能落盘");
    (path, bytes.len())
}

/// 挂一台混响设备（全湿、最长衰减）的夹具。
fn wet_project() -> (YebanProjectV1, EntityId) {
    let mut fixture = note_project(&NOTES);
    let track = fixture.track;
    mount(
        &mut fixture.project,
        track,
        vec![effect(
            "Reverb",
            &[
                ("reverb_size", 1.0),
                ("reverb_wet", 1.0),
                ("reverb_predelay", 0.02),
            ],
        )],
    );
    (fixture.project, track)
}

/// R0：**未武装**的四种口径必须与"没有设备链"逐字节相同
/// （`crate::insert` 模块文档 §8.1 / §8.2）。
///
/// 四种输入的唯一差别是设备链：
///
/// 1. `params` 为空的效果器（只为上报延迟而存在的那一类）；
/// 2. 参数名一个都不认识的效果器 —— **本仓库既有的**两个"弄不懂的名字"
///    （`reverb_mix` 与 `filter_cutoff_hz`，见 `compressor_insert.rs` 的 C0）
///    加上**本票刻意拒收**的 `reverb_width`；
/// 3. `bypassed = true` 但混响参数**命中**；
/// 4. `InternalInstrument` 设备带混响参数（乐器不是插入器件）。
#[test]
fn projects_without_recognised_reverb_params_are_bit_identical() {
    let fixture = note_project(&NOTES);
    let track = fixture.track;

    let bare = render(&fixture.project, QUANTA);

    let mut unknown_project = fixture.project.clone();
    mount(&mut unknown_project, track, vec![effect("Unknown", &[])]);
    let unknown = render(&unknown_project, QUANTA);

    let mut unrelated_project = fixture.project.clone();
    mount(
        &mut unrelated_project,
        track,
        vec![effect(
            "Unrelated",
            &[
                ("reverb_mix", 0.3),
                ("reverb_width", 0.9),
                ("filter_cutoff_hz", 800.0),
            ],
        )],
    );
    let unrelated = render(&unrelated_project, QUANTA);

    let mut bypassed_project = fixture.project.clone();
    let mut bypassed_device = effect("Reverb", &[("reverb_size", 1.0), ("reverb_wet", 1.0)]);
    bypassed_device.bypassed = true;
    mount(&mut bypassed_project, track, vec![bypassed_device]);
    let bypassed = render(&bypassed_project, QUANTA);

    let mut instrument_project = fixture.project.clone();
    let mut instrument = effect("Hollow", &[("reverb_size", 1.0), ("reverb_wet", 1.0)]);
    instrument.kind = DeviceKind::InternalInstrument;
    mount(&mut instrument_project, track, vec![instrument]);
    let instrument = render(&instrument_project, QUANTA);

    let (path, bytes) = dump("yeban-reverb-unarmed.raw", &bare.left, &bare.right);
    println!(
        "[engine-reverb/R0] 未武装: 落盘={} 字节={bytes} 帧={} 指纹={:#018x}",
        path.display(),
        bare.frames(),
        bare.fingerprint()
    );

    assert!(bare.frames() > 0, "夹具必须真的渲染出帧");
    assert!(bare.peak() > 0.0, "夹具必须真的出声（否则逐位相同是空转）");
    for (label, other) in [
        ("参数为空的效果器", &unknown),
        (
            "参数不认识的效果器（含 reverb_mix / reverb_width）",
            &unrelated,
        ),
        ("bypassed 的混响", &bypassed),
        ("内置乐器设备带混响参数", &instrument),
    ] {
        assert_eq!(
            bare.left_bits(),
            other.left_bits(),
            "{label}: 输出必须与没有设备链时逐位相同（指纹 {:#018x} vs {:#018x}）",
            bare.fingerprint(),
            other.fingerprint()
        );
        assert_eq!(bare.fingerprint(), other.fingerprint(), "{label}: 指纹相同");
    }
}

/// R1：命中的设备**真的武装进了实时侧**（三层证据，缺一不可）。
///
/// 1. **快照层**：`EngineSnapshot::insert_params(轨)` 给出投影参数（模型 → 快照）；
/// 2. **实时层**：`EngineRuntime::armed_reverb(轨)` 逐字段逐位等于第 1 层
///    （快照 → 逐样本路径的武装表）；
/// 3. **行为层**：`EngineStats::insert_reverb_frames` 等于渲染帧数。
#[test]
fn recognised_reverb_is_armed_into_the_realtime_path() {
    let (project, track) = wet_project();

    let snapshot =
        yeban_engine::snapshot::EngineSnapshot::from_project(&project, 1).expect("夹具快照");
    let projected = snapshot
        .insert_params(&track)
        .and_then(yeban_engine::insert::InsertParams::reverb)
        .expect("命中的效果器设备必须投影出混响参数");

    let mut captured: Option<yeban_engine::insert::ReverbParams> = None;
    let mut slots = usize::MAX;
    let mut armed_sample_rate = 0u32;
    let rendered = render_with(&project, QUANTA, 1, |quantum, rig| {
        if quantum == 1 {
            captured = rig.runtime.armed_reverb(&track);
            slots = rig.runtime.armed_reverb_slot_count();
            armed_sample_rate = rig.runtime.armed_reverb_sample_rate();
        }
    });

    let armed = captured.expect("命中的混响必须被武装进实时侧");
    for (name, actual, expected) in [
        ("size", armed.size, projected.size),
        ("damp", armed.damp, projected.damp),
        ("mix", armed.mix, projected.mix),
        ("width", armed.width, projected.width),
        ("predelay", armed.predelay, projected.predelay),
    ] {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "武装表的 {name} 必须逐位等于快照投影: 武装={actual} 投影={expected}"
        );
    }
    assert_eq!(slots, 1, "夹具只有一台混响 ⇒ 只应占一个槽位");
    assert_eq!(
        armed_sample_rate, 48_000,
        "延迟线池必须按初始快照的采样率武装"
    );
    // 投影的值确实不是默认值（否则"接线了"与"没接线"读数相同）。
    let default = yeban_engine::insert::ReverbParams::default();
    assert_ne!(armed.size.to_bits(), default.size.to_bits());
    assert_ne!(armed.mix.to_bits(), default.mix.to_bits());

    let stats = rendered.stats;
    println!(
        "[engine-reverb/R1] 武装: 槽位={slots} 采样率={armed_sample_rate} 混响处理帧数={} \
         渲染帧数={} 峰值={:.6}",
        stats.insert_reverb_frames,
        rendered.frames(),
        rendered.peak()
    );
    // 行为层：混响处理帧数 = 渲染帧数（逐样本路径真的调用了器件）。
    assert_eq!(
        stats.insert_reverb_frames,
        rendered.frames() as u64,
        "混响处理帧数必须等于渲染帧数（差 ⇒ 逐样本路径漏掉了混响级）"
    );
    assert_eq!(
        stats.insert_reverb_rate_rejects, 0,
        "采样率没有变 ⇒ 不许出现拒绝"
    );
}

/// R2：**混响真的加上了尾巴**。
///
/// 观测方式：同一个音符栅格，未武装的渲染在尾窗里必须**逐位静音**
/// （否则"有能量"归因不唯一）；武装全湿混响的渲染在**同一个**窗口里必须有能量。
#[test]
fn the_reverb_really_adds_a_tail_after_the_note_ends() {
    let fixture = note_project(&NOTES);
    let (wet, _track) = wet_project();

    let bare = render(&fixture.project, QUANTA);
    let reverbed = render(&wet, QUANTA);

    let (bare_tail_rms, bare_tail_peak) = rms_peak(&bare.left[TAIL.clone()]);
    let (wet_tail_rms, wet_tail_peak) = rms_peak(&reverbed.left[TAIL.clone()]);
    println!(
        "[engine-reverb/R2] 尾窗 {:?}: 未武装 rms={bare_tail_rms:.9} 峰值={bare_tail_peak:.9}；\
         武装 rms={wet_tail_rms:.9} 峰值={wet_tail_peak:.9}（线性幅度）",
        TAIL
    );
    assert_eq!(
        bare_tail_peak, 0.0,
        "未武装的渲染在尾窗里必须逐位静音（峰值 {bare_tail_peak}）—— 否则本判据的归因不唯一"
    );
    assert!(
        wet_tail_peak > 1e-4,
        "武装混响的渲染在同一个尾窗里必须有能量，实测峰值 {wet_tail_peak}"
    );
    assert!(
        wet_tail_rms > 1e-5,
        "尾窗 RMS 必须 > 1e-5（线性幅度），实测 {wet_tail_rms}"
    );
}

/// R3：`reverb_wet` 与 `reverb_size` **方向正确**。
///
/// 湿声变大 ⇒ 尾窗能量上升；衰减变长（`size` 变大）⇒ 尾窗能量上升。
/// 两条都单调（同一个窗口、同一份干声）。
#[test]
fn the_wet_and_size_knobs_move_the_tail_in_the_right_direction() {
    let fixture = note_project(&NOTES);
    let track = fixture.track;

    let mut energies = Vec::new();
    for wet in [0.2f32, 0.6, 1.0] {
        let mut project = fixture.project.clone();
        mount(
            &mut project,
            track,
            vec![effect(
                "Reverb",
                &[("reverb_size", 1.0), ("reverb_wet", wet)],
            )],
        );
        let rendered = render(&project, QUANTA);
        let (rms, _) = rms_peak(&rendered.left[TAIL.clone()]);
        energies.push((wet, rms));
    }
    println!("[engine-reverb/R3] 湿声 → 尾窗 rms: {energies:?}（线性幅度）");
    for pair in energies.windows(2) {
        assert!(
            pair[1].1 > pair[0].1,
            "湿声 {} → {} 必须让尾窗能量上升：{:.9} → {:.9}",
            pair[0].0,
            pair[1].0,
            pair[0].1,
            pair[1].1
        );
    }

    let mut sizes = Vec::new();
    for size in [0.3f32, 0.6, 1.0] {
        let mut project = fixture.project.clone();
        mount(
            &mut project,
            track,
            vec![effect(
                "Reverb",
                &[("reverb_size", size), ("reverb_wet", 1.0)],
            )],
        );
        let rendered = render(&project, QUANTA);
        let (rms, _) = rms_peak(&rendered.left[TAIL.clone()]);
        sizes.push((size, rms));
    }
    println!("[engine-reverb/R3] 衰减 → 尾窗 rms: {sizes:?}（线性幅度）");
    for pair in sizes.windows(2) {
        assert!(
            pair[1].1 > pair[0].1,
            "衰减 {} → {} 必须让尾窗能量上升：{:.9} → {:.9}",
            pair[0].0,
            pair[1].0,
            pair[0].1,
            pair[1].1
        );
    }

    // 夹具本身必须真的被改到（否则上面的单调只是噪声）。
    assert!(fixture.project.tracks.contains_key(&track));
}

/// R4：**确定性**（两次独立装配逐位相同；等价快照的周期发布不改变输出）。
///
/// 第二半是"重新武装不得清空器件状态"的机械判据：混响的延迟线是**时间状态**，
/// 一次 `set_sample_rate`（哪怕是同值复位）都会把尾巴擦掉。
#[test]
fn two_independent_runs_with_an_armed_reverb_are_bit_identical() {
    let (project, _track) = wet_project();

    let first = render(&project, QUANTA);
    let second = render(&project, QUANTA);
    assert_eq!(
        first.left_bits(),
        second.left_bits(),
        "两次独立装配必须逐位相同（指纹 {:#018x} vs {:#018x}）",
        first.fingerprint(),
        second.fingerprint()
    );
    assert_eq!(first.fingerprint(), second.fingerprint());

    // 同一个装配里每 128 个量子发布一份**等价**快照（只改 revision）⇒ 重新武装。
    let switched = render_with(&project, QUANTA, 1, |quantum, rig| {
        if quantum > 0 && quantum % 128 == 0 {
            rig.publish_equivalent(&project, (quantum / 128) as u64 + 100);
        }
    });
    assert_eq!(
        first.left_bits(),
        switched.left_bits(),
        "周期性重新武装不得改变输出（指纹 {:#018x} vs {:#018x}）",
        first.fingerprint(),
        switched.fingerprint()
    );
}

/// R5：**不同量子数（块切分）**下前缀逐位相同（[ARCH-DET-001] 的引擎侧对账）。
#[test]
fn different_quantum_counts_agree_on_the_shared_prefix() {
    let (project, _track) = wet_project();

    let short = render(&project, 37);
    let long = render(&project, 400);
    assert_eq!(
        short.left_bits(),
        long.left_bits()[..short.left.len()],
        "37 个量子的前缀必须与 400 个量子的前缀逐位相同"
    );
    assert_eq!(
        short.right.as_slice(),
        &long.right[..short.right.len()],
        "右声道同样"
    );
    assert!(short.peak() > 0.0, "短渲染必须真的出声");
}

/// R6：引擎的 `Reverb` **就是** dsp 的混响（类型 + 函数地址同一性 + 零延迟）。
#[test]
fn engine_reverb_is_literally_the_dsp_reverb() {
    use yeban_engine::insert::{DspReverb, Reverb};

    // 类型同一性：把 engine 路径构造的值赋给 dsp 路径的类型。
    let engine: Reverb = Reverb::new();
    let as_dsp: DspReverb = engine;
    assert!(!as_dsp.is_configured(), "刚构造的混响必须是未配置的");

    // 函数地址同一性（`new` / `set_params` / `process`）。
    let engine_new: fn() -> Reverb = Reverb::new;
    let dsp_new: fn() -> DspReverb = DspReverb::new;
    assert_eq!(
        engine_new as usize, dsp_new as usize,
        "engine 的 Reverb::new 必须就是 dsp 的那个函数"
    );
    let engine_params: fn(&mut Reverb, yeban_engine::insert::ReverbParams) = Reverb::set_params;
    let dsp_params: fn(&mut DspReverb, yeban_engine::insert::ReverbParams) = DspReverb::set_params;
    assert_eq!(
        engine_params as usize, dsp_params as usize,
        "engine 的 Reverb::set_params 必须就是 dsp 的那个函数"
    );
    let engine_process: fn(&mut Reverb, &mut [f32], &mut [f32]) = Reverb::process;
    let dsp_process: fn(&mut DspReverb, &mut [f32], &mut [f32]) = DspReverb::process;
    assert_eq!(
        engine_process as usize, dsp_process as usize,
        "engine 的 Reverb::process 必须就是 dsp 的那个函数"
    );

    // **零延迟契约**：混响设备登记 0 采样 ⇒ 该轨的 PDC 补偿必须是 0
    // （否则"混响接在 PDC 之前"这条位置说明会让相位错位）。
    let (project, track) = wet_project();
    let mut delay = None;
    let _ = render_with(&project, 8, 1, |quantum, rig| {
        if quantum == 1 {
            delay = rig.runtime.armed_pdc_delay(&track);
        }
    });
    assert_eq!(delay, Some(0), "零延迟器件不得让该轨获得 PDC 补偿");
}

/// R7：**首个来源**规则 + 只带通道条参数的设备不产生混响
/// （`crate::insert` 模块文档 §8 的 `from_devices` 文档）。
#[test]
fn the_first_recognised_effect_in_the_chain_is_the_only_reverb_source() {
    let fixture = note_project(&NOTES);
    let track = fixture.track;

    // 更早的无关效果器不参与，更晚的混响不覆盖更早的。
    let mut ordered = fixture.project.clone();
    mount(
        &mut ordered,
        track,
        vec![
            effect("Unrelated", &[("reverb_mix", 0.3)]),
            effect("First", &[("reverb_size", 0.9)]),
            effect("Later", &[("reverb_size", 0.1)]),
        ],
    );
    let snapshot = yeban_engine::snapshot::EngineSnapshot::from_project(&ordered, 1).expect("快照");
    let params = snapshot
        .insert_params(&track)
        .and_then(yeban_engine::insert::InsertParams::reverb)
        .expect("第二个设备必须命中");
    assert_eq!(
        params.size.to_bits(),
        0.9f32.to_bits(),
        "首个含已识别名字的设备是唯一来源"
    );

    // 只带通道条参数的设备**不**产生混响。
    let mut strip_only = fixture.project.clone();
    mount(
        &mut strip_only,
        track,
        vec![effect(
            "Strip",
            &[
                ("threshold_db", -24.0),
                ("ratio", 4.0),
                ("eq_low_gain", 3.0),
            ],
        )],
    );
    let snapshot =
        yeban_engine::snapshot::EngineSnapshot::from_project(&strip_only, 1).expect("快照");
    let insert = snapshot.insert_params(&track).expect("通道条必须命中");
    assert!(insert.strip().is_some(), "通道条必须命中");
    assert!(
        insert.reverb().is_none(),
        "只有通道条参数的设备不许产生混响"
    );
}

/// R8：`reverb_wet = 0` 是**逐位直通**（器件自己的 `mix ≤ 1e-4` 守卫）。
#[test]
fn a_wet_of_zero_is_a_bit_identical_passthrough() {
    let fixture = note_project(&NOTES);
    let track = fixture.track;

    let bare = render(&fixture.project, QUANTA);

    let mut silent_wet = fixture.project.clone();
    mount(
        &mut silent_wet,
        track,
        vec![effect(
            "Reverb",
            &[("reverb_size", 1.0), ("reverb_wet", 0.0)],
        )],
    );
    let rendered = render(&silent_wet, QUANTA);
    println!(
        "[engine-reverb/R8] wet=0: 混响处理帧数={} 指纹={:#018x} vs 未武装 {:#018x}",
        rendered.stats.insert_reverb_frames,
        rendered.fingerprint(),
        bare.fingerprint()
    );
    assert_eq!(
        bare.left_bits(),
        rendered.left_bits(),
        "湿声 0 ⇒ 必须与没有混响逐位相同（指纹 {:#018x} vs {:#018x}）",
        bare.fingerprint(),
        rendered.fingerprint()
    );
    assert_eq!(
        rendered.stats.insert_reverb_frames, 0,
        "湿声 0 时器件自己早返回 ⇒ 引擎的'处理过帧数'必须是 0"
    );
}

/// R9：**换采样率的快照不武装混响**（延迟线不在音频线程重建）。
///
/// 观测方式：先用 48 kHz 的快照渲染（混响武装），再发布一份 **44.1 kHz** 的快照：
///
/// 1. 拒绝计数 +1，槽位归零；
/// 2. 输出回到"未武装"的读数 —— 但**不是**与 bare 逐位相同：干声本身也换了采样率
///    （音高/时间栅格都变），因此这里断言的是"那一刻之后 `insert_reverb_frames`
///    不再增长"，而不是逐位比较。
/// 3. 换回 48 kHz ⇒ 重新武装（守卫是"拒绝这一份"，不是"永久停用"）。
#[test]
fn a_sample_rate_change_does_not_rebuild_the_reverb_lines() {
    let (project, _track) = wet_project();

    let mut rejects = 0u64;
    let mut slots_after_shift = usize::MAX;
    let mut frames_after_shift = 0u64;
    let mut slots_after_restore = usize::MAX;
    let mut shift_quantum = usize::MAX;
    let mut scratch = [0.0f32; 128 * 2];

    let rendered = render_with(&project, 24, 1, |quantum, rig| {
        if quantum == 12 {
            // 发布一份换采样率的快照（44.1 kHz）：延迟线不为它重建。
            let mut shifted = project.clone();
            shifted.audio_config.sample_rate = yeban_model::SampleRate::Hz44100;
            rig.publish_equivalent(&shifted, 2);
        }
        if quantum == 16 {
            let before = rig.runtime.stats().insert_reverb_frames;
            // 再推一个量子：这一份快照里混响必须**整段不武装**。
            rig.runtime.process_quantum(&mut scratch, 2);
            let after = rig.runtime.stats();
            frames_after_shift = after.insert_reverb_frames.saturating_sub(before);
            slots_after_shift = rig.runtime.armed_reverb_slot_count();
            rejects = after.insert_reverb_rate_rejects;
            shift_quantum = quantum;
            // 换回 48 kHz：必须重新武装。
            rig.publish_equivalent(&project, 3);
            rig.runtime.process_quantum(&mut scratch, 2);
            slots_after_restore = rig.runtime.armed_reverb_slot_count();
        }
    });

    println!(
        "[engine-reverb/R9] 换采样率: 第 {shift_quantum} 个量子后 拒绝累计={rejects} \
         武装槽位={slots_after_shift} 该量子新增处理帧数={frames_after_shift}；\
         换回 48 kHz 后槽位={slots_after_restore}；总渲染帧={}",
        rendered.frames()
    );
    assert_eq!(rejects, 1, "换采样率必须恰好累加 1 次拒绝");
    assert_eq!(slots_after_shift, 0, "换采样率之后混响必须整段不武装");
    assert_eq!(frames_after_shift, 0, "不武装的量子不许推进'混响处理帧数'");
    assert_eq!(slots_after_restore, 1, "换回 48 kHz 后必须重新武装");
    assert!(rendered.peak() > 0.0, "夹具必须真的出声");
}
