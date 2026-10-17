//! 任务单 `#173`：**主总线（master）上的插入链** —— 让 master 上的 EQ / 混响 /
//! 卷积混响在**实时监听**里真的生效（`#158` 之后唯一还"挂得上、听不到"的位置）。
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | M1 | master `eq_low_gain: +12` ⇒ 低频夹具的 RMS 上升（给 dB 读数）；`master_insert_frames == 渲染帧数`、`insert_strip_frames == 渲染帧数` | 3a⁗⁗ 不跑 / 武装循环仍跳过 master |
//! | M2 | master `reverb_wet: 1` ⇒ 音符结束后 **200–700 ms** 的尾窗 RMS 显著高于 `reverb_wet: 0`；`master_insert_frames` 与 `insert_reverb_frames` 都 == 渲染帧数（每量子**恰好一次**） | 链跑两遍（计数 2×、尾巴两倍速衰减）/ 链没接 |
//! | M3 | **快路**：master 无设备 ⇒ `master_insert_frames == 0`、三个插入计数全 0、输出**非静音**（判据不是空转）；同夹具只给 master 挂一台湿混响 ⇒ 计数 == 渲染帧数且指纹**必变** | 无设备也进插入路径（下混会改变硬声相的 L/R） |
//! | M4 | **旁通**：master 的设备 `bypassed = true` ⇒ 计数全 0、指纹与"无设备"**逐位相同** | 旁通设备仍被武装 |
//! | M5 | **裁决 (甲) 的代价可判**：master 有**已武装但听不见**的器件（`reverb_wet: 0`）⇒ 母线被下混成单声道（L == R）；无设备时同一硬声相夹具 L ≠ R | "武装但不可闻"没有进下混分支 / 没有下混 |
//!
//! 判据全部走**产品路径**：`YebanProjectV1` → `EngineSnapshot::from_project` →
//! `EngineRuntime::process_quantum`（`support::render`）。时间栅格与
//! `tests/bus_insert.rs` 相同：120 BPM / 48 kHz / 960 PPQ ⇒ **1 tick = 25 样本**。
//!
//! ⚠ **本文件不证明"与导出逐位相同"**：实时主总线插入是**下混成单声道**
//! （裁决 (甲)），离线（`yeban-render`）是**真立体声**插入；对齐的是"有/没有"插入，
//! 不是逐位（报告 §7 登记，与 `#155`／`#158` 同族）。

mod support;

use std::collections::BTreeMap;

use support::{NoteSpec, render, rms_peak};
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition, DeviceKind, EntityId, LoopConfig,
    MidiNote, ParameterValue, RoutingEdge, RoutingGraph, RoutingKind, TrackKind, TrackV3,
    YebanProjectV1,
};

/// M1/M3/M4/M5 的渲染长度：400 × 128 = 51,200 帧 ≈ 1.07 s（覆盖 1 s 的音符）。
const SHORT_QUANTA: usize = 400;

/// M2 的渲染长度：2,400 × 128 = 307,200 帧 = 6.4 s @48 kHz（尾窗远在音符之后）。
const TAIL_QUANTA: usize = 2_400;

/// M2 尾窗：音符在 tick 960 结束 = 样本 **24,000**；200 ms = 9,600、700 ms = 33,600
/// ⇒ `[33_600, 57_600)`（与 `tests/bus_insert.rs` 的 B1 同一个窗口粒度）。
const TAIL_WINDOW: core::ops::Range<usize> = 33_600..57_600;

/// 一张效果器设备（`InternalEffect`）。
fn effect(params: &[(&str, f32)]) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: "Effect".to_owned(),
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

/// 一台混响设备；`wet = 0` 是"武装但器件的 `is_active()` 为假"的那一档。
fn reverb_device(wet: f32, size: f32) -> DeviceDefinition {
    effect(&[("reverb_size", size), ("reverb_wet", wet)])
}

/// 线性 RMS → dBFS（与 `tests/bus_insert.rs` 的 B2 同一把尺子）。
fn dbfs(linear: f64) -> f64 {
    20.0 * (linear.max(1e-12)).log10()
}

/// 组装工程：**一条 MIDI 轨（直连 master）＋ 主总线轨（可带设备）**。
fn master_project(
    note: NoteSpec,
    track_pan: f32,
    track_volume_db: f32,
    master_devices: Vec<DeviceDefinition>,
) -> YebanProjectV1 {
    let master = EntityId::new();
    let track = EntityId::new();
    let mut tracks: BTreeMap<EntityId, TrackV3> = BTreeMap::new();
    let mut pool: BTreeMap<EntityId, ClipPoolEntry> = BTreeMap::new();

    let clip = EntityId::new();
    let placement = EntityId::new();
    let note_id = EntityId::new();
    let mut notes = BTreeMap::new();
    let mut entry = MidiNote::new(note_id, note.start_tick, note.pitch, note.duration_ticks);
    entry.velocity = note.velocity;
    notes.insert(note_id, entry);
    pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "Clip".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );
    let mut track_model = TrackV3 {
        id: track,
        name: "Track".to_owned(),
        kind: TrackKind::Midi,
        pan: track_pan,
        volume_db: track_volume_db,
        ..TrackV3::default()
    };
    track_model.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: note.start_tick + note.duration_ticks,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );
    tracks.insert(track, track_model);
    tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            devices: master_devices,
            ..TrackV3::default()
        },
    );

    let mut routing = RoutingGraph {
        nodes: vec![track, master],
        ..RoutingGraph::default()
    };
    let edge = EntityId::new();
    routing.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: track,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );

    YebanProjectV1 {
        bpm: 120.0,
        master_bus_track_id: master,
        tracks,
        routing_graph: routing,
        clip_pool: pool,
        ..YebanProjectV1::default()
    }
}

/// M1：**实时 master EQ 生效**。
///
/// 夹具音高 36（C2 ≈ 65.4 Hz）+ master `eq_low_gain: +12 dB`（低架 @200 Hz）。
/// 轨音量 −12 dB 让提升之后仍低于母线限制器阈值（否则 RMS 差不能唯一归因给 EQ）。
#[test]
fn m1_the_master_eq_raises_the_low_band() {
    let note = NoteSpec::at(0, 1_920, 36, 40);
    let plain = master_project(note, 0.0, -12.0, Vec::new());
    let boosted = master_project(
        note,
        0.0,
        -12.0,
        vec![effect(&[("eq_low_gain", 12.0), ("eq_low_freq", 200.0)])],
    );
    let plain = render(&plain, SHORT_QUANTA);
    let boosted = render(&boosted, SHORT_QUANTA);

    // 只看**音符还在响**的窗口（0..1 s = 0..48,000 样本），尾巴不参与。
    let window = 0..48_000;
    let (plain_rms, plain_peak) = rms_peak(&plain.left[window.clone()]);
    let (boosted_rms, boosted_peak) = rms_peak(&boosted.left[window]);
    let expected_frames = (SHORT_QUANTA * 128) as u64;
    println!(
        "[#173/M1] 低频夹具（pitch 36 ≈ 65.4 Hz，轨音量 −12 dB）：无 master 设备 RMS={:.9}\
         （{:.3} dBFS，峰值 {:.6}）；master EQ +12 dB RMS={:.9}（{:.3} dBFS，峰值 {:.6}）；\
         差={:+.3} dB；master_insert_frames 无设备={} / 有设备={}（渲染帧数 {}）；\
         insert_strip_frames={}；fingerprint 无设备={:#018x} / 有设备={:#018x}",
        plain_rms,
        dbfs(plain_rms),
        plain_peak,
        boosted_rms,
        dbfs(boosted_rms),
        boosted_peak,
        dbfs(boosted_rms) - dbfs(plain_rms),
        plain.stats.master_insert_frames,
        boosted.stats.master_insert_frames,
        expected_frames,
        boosted.stats.insert_strip_frames,
        plain.fingerprint(),
        boosted.fingerprint(),
    );

    assert_eq!(
        plain.stats.master_insert_frames, 0,
        "master 无设备 ⇒ 3a⁗⁗ 整段跳过（快路），计数必须为 0"
    );
    assert_eq!(
        boosted.stats.master_insert_frames, expected_frames,
        "master 的链必须**恰好每量子处理一次**"
    );
    assert_eq!(
        boosted.stats.insert_strip_frames, expected_frames,
        "master 的通道条必须恰好每量子处理一次"
    );
    assert_eq!(
        boosted.stats.limiter_gain_reductions, 0,
        "提升臂必须整体低于母线限制器阈值 ⇒ RMS 差归因唯一"
    );
    assert!(
        boosted_peak < 0.9,
        "峰值 {boosted_peak:.6} 必须低于限制器阈值 0.9"
    );
    assert!(
        dbfs(boosted_rms) - dbfs(plain_rms) > 3.0,
        "master 上的 +12 dB 低架必须让低频夹具的 RMS 上升 > 3 dB（实得 {:+.3} dB）",
        dbfs(boosted_rms) - dbfs(plain_rms)
    );
    assert_ne!(
        plain.fingerprint(),
        boosted.fingerprint(),
        "master EQ 必须真的改变输出（指纹相同 ⇒ 链路没接上）"
    );
}

/// M2：**实时 master 混响生效**。
///
/// 判据：音符结束后 200–700 ms 的尾窗里，`reverb_wet: 1` 的 RMS 显著高于
/// `reverb_wet: 0`；并且 `master_insert_frames` 与 `insert_reverb_frames` 都
/// **恰好等于渲染帧数**（每量子一次 —— 若逐轨/试听分支也跑，计数会翻倍）。
#[test]
fn m2_the_master_reverb_tail_is_audible_after_the_note_ends() {
    let note = NoteSpec::at(0, 960, 48, 40);
    let dry = master_project(note, 0.0, 0.0, vec![reverb_device(0.0, 0.8)]);
    let wet = master_project(note, 0.0, 0.0, vec![reverb_device(1.0, 0.8)]);
    let dry = render(&dry, TAIL_QUANTA);
    let wet = render(&wet, TAIL_QUANTA);

    let (dry_tail, _) = rms_peak(&dry.left[TAIL_WINDOW.clone()]);
    let (wet_tail, _) = rms_peak(&wet.left[TAIL_WINDOW.clone()]);
    let expected_frames = (TAIL_QUANTA * 128) as u64;
    println!(
        "[#173/M2] 尾窗 [{}..{})：master wet=0 RMS={:.9}（{:.3} dBFS）；wet=1 RMS={:.9}\
         （{:.3} dBFS）；两档差={:+.3} dB；master_insert_frames: wet=0 {} / wet=1 {}；\
         insert_reverb_frames: wet=0 {} / wet=1 {}（渲染帧数 {}）；\
         limiter_gain_reductions={}",
        TAIL_WINDOW.start,
        TAIL_WINDOW.end,
        dry_tail,
        dbfs(dry_tail),
        wet_tail,
        dbfs(wet_tail),
        dbfs(wet_tail) - dbfs(dry_tail),
        dry.stats.master_insert_frames,
        wet.stats.master_insert_frames,
        dry.stats.insert_reverb_frames,
        wet.stats.insert_reverb_frames,
        expected_frames,
        wet.stats.limiter_gain_reductions,
    );

    assert_eq!(
        dry.stats.insert_reverb_frames, 0,
        "wet=0 ⇒ 器件的 is_active() 为假 ⇒ 状态不推进、计数必须为 0"
    );
    assert_eq!(
        dry.stats.master_insert_frames, expected_frames,
        "武装但湿声为 0 的 master 器件仍会进下混路径（裁决 (甲) 的代价），计数照常推进"
    );
    assert_eq!(
        wet.stats.insert_reverb_frames, expected_frames,
        "master 混响必须**恰好每量子处理一次**（每节点每量子一次）"
    );
    assert_eq!(
        wet.stats.master_insert_frames, expected_frames,
        "master 的链每量子**恰好一次**"
    );
    assert!(dry_tail < 1e-5, "wet=0 的尾窗必须是静音：{dry_tail:.9}");
    assert!(
        wet_tail > 1e-3,
        "wet=1 的尾窗必须听到混响尾巴（> 1e-3）：{wet_tail:.9}"
    );
    assert!(
        wet_tail > 10.0 * dry_tail,
        "wet=1 的尾窗 RMS 必须显著高于 wet=0 基线（实得 {:.1}×）",
        wet_tail / dry_tail.max(f64::MIN_POSITIVE)
    );
    assert_eq!(
        wet.stats.limiter_gain_reductions, 0,
        "尾窗读数必须整体低于母线限制器阈值 ⇒ 归因唯一"
    );
}

/// M3：**快路可判** —— master 没有设备时不进插入路径。
///
/// 三条腿：
/// 1. 无 master 设备 ⇒ `master_insert_frames == 0`、三个插入计数全 0；
/// 2. 输出**非静音**（否则"计数为 0"可能只是空转）；
/// 3. **正对照**：同一夹具只给 master 挂一台湿混响 ⇒ 计数 == 渲染帧数且指纹**必变**。
#[test]
fn m3_a_master_without_devices_stays_on_the_fast_path() {
    let note = NoteSpec::at(0, 960, 48, 40);
    let fast = master_project(note, 0.0, 0.0, Vec::new());
    let armed = master_project(note, 0.0, 0.0, vec![reverb_device(1.0, 0.8)]);
    let fast = render(&fast, SHORT_QUANTA);
    let armed = render(&armed, SHORT_QUANTA);
    let expected_frames = (SHORT_QUANTA * 128) as u64;
    let (fast_rms, _) = rms_peak(&fast.left);
    println!(
        "[#173/M3] master 无设备：master_insert_frames={}（insert strip/reverb/conv={}/{}/{}）；\
         窗口 RMS={:.9}；fingerprint={:#018x}；挂湿混响：master_insert_frames={}、\
         insert_reverb_frames={}（渲染帧数 {}）、fingerprint={:#018x}",
        fast.stats.master_insert_frames,
        fast.stats.insert_strip_frames,
        fast.stats.insert_reverb_frames,
        fast.stats.insert_convolution_frames,
        fast_rms,
        fast.fingerprint(),
        armed.stats.master_insert_frames,
        armed.stats.insert_reverb_frames,
        expected_frames,
        armed.fingerprint(),
    );

    assert_eq!(
        fast.stats.master_insert_frames, 0,
        "master 无设备 ⇒ 不进 3a⁗⁗（快路）"
    );
    assert_eq!(fast.stats.insert_strip_frames, 0);
    assert_eq!(fast.stats.insert_reverb_frames, 0);
    assert_eq!(fast.stats.insert_convolution_frames, 0);
    assert!(fast_rms > 0.0, "快路臂必须真的出声（否则计数 0 是空转）");
    assert_eq!(armed.stats.master_insert_frames, expected_frames);
    assert_eq!(armed.stats.insert_reverb_frames, expected_frames);
    assert_ne!(
        fast.fingerprint(),
        armed.fingerprint(),
        "只给 master 挂一台湿混响就必须改变输出"
    );
}

/// M4：**旁通** —— `bypassed = true` 的 master 设备不武装 ⇒ 回到干信号。
#[test]
fn m4_a_bypassed_master_insert_returns_the_dry_signal() {
    let note = NoteSpec::at(0, 960, 48, 40);
    let mut bypassed_reverb = reverb_device(1.0, 0.8);
    bypassed_reverb.bypassed = true;
    let dry = master_project(note, 0.0, 0.0, Vec::new());
    let bypassed = master_project(note, 0.0, 0.0, vec![bypassed_reverb]);
    let dry = render(&dry, SHORT_QUANTA);
    let bypassed = render(&bypassed, SHORT_QUANTA);
    let (dry_tail, _) = rms_peak(&dry.left[30_000..]);
    let (bypassed_tail, _) = rms_peak(&bypassed.left[30_000..]);
    println!(
        "[#173/M4] bypassed=true：master_insert_frames={}、insert counts={}/{}/{}；\
         fingerprint={:#018x}（无设备 {:#018x}）；尾段 RMS={:.9}（无设备 {:.9}）",
        bypassed.stats.master_insert_frames,
        bypassed.stats.insert_strip_frames,
        bypassed.stats.insert_reverb_frames,
        bypassed.stats.insert_convolution_frames,
        bypassed.fingerprint(),
        dry.fingerprint(),
        bypassed_tail,
        dry_tail,
    );

    assert_eq!(
        bypassed.stats.master_insert_frames, 0,
        "旁通的 master 设备不武装 ⇒ 3a⁗⁗ 整段跳过"
    );
    assert_eq!(bypassed.stats.insert_strip_frames, 0);
    assert_eq!(bypassed.stats.insert_reverb_frames, 0);
    assert_eq!(bypassed.stats.insert_convolution_frames, 0);
    assert_eq!(
        dry.fingerprint(),
        bypassed.fingerprint(),
        "bypassed=true 必须与'没有设备'逐位相同"
    );
    assert!(
        bypassed_tail < 1e-6,
        "旁通之后尾段必须回到干信号（静音）：{bypassed_tail:.9}"
    );
}

/// M5：**裁决 (甲) 的代价可判** —— master 上"武装但听不见"的器件照样触发下混。
///
/// 硬声相（`pan = -1.0`）的夹具：
/// * master **无设备** ⇒ 母线保留左右差异（`L ≠ R`）；
/// * master 挂 `reverb_wet: 0`（已武装、`is_active()` 为假 ⇒ **不碰样本**）
///   ⇒ 3a⁗⁗ 仍然下混成单声道再复制两路 ⇒ `L == R`，且计数 == 渲染帧数。
///
/// 这条判据把"下混是**只要武装就发生**"这一代价钉住（⛔ 不是"湿声为 0 就绕过"）——
/// 与 `#158` 的 B4 同一条口径；没有它，"代价"只在注释里。
#[test]
fn m5_an_armed_but_silent_master_device_still_downmixes_the_bus() {
    let note = NoteSpec::at(0, 960, 48, 40);
    let dry = master_project(note, -1.0, 0.0, Vec::new());
    let armed_silent = master_project(note, -1.0, 0.0, vec![reverb_device(0.0, 0.8)]);
    let dry = render(&dry, SHORT_QUANTA);
    let armed_silent = render(&armed_silent, SHORT_QUANTA);
    let expected_frames = (SHORT_QUANTA * 128) as u64;
    let dry_lr_equal = dry
        .left
        .iter()
        .zip(dry.right.iter())
        .all(|(left, right)| left.to_bits() == right.to_bits());
    let armed_lr_equal = armed_silent
        .left
        .iter()
        .zip(armed_silent.right.iter())
        .all(|(left, right)| left.to_bits() == right.to_bits());
    let dry_energy: f64 = dry
        .left
        .iter()
        .chain(dry.right.iter())
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum();
    let armed_energy: f64 = armed_silent
        .left
        .iter()
        .chain(armed_silent.right.iter())
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum();
    println!(
        "[#173/M5] 硬声相 pan=-1：master 无设备 ⇒ L==R={}（总能量 {:.6}）；\
         master reverb_wet=0 ⇒ L==R={}（总能量 {:.6}）；master_insert_frames={}（渲染帧数 {}）、\
         insert_reverb_frames={}；fingerprint 无设备={:#018x} / 武装但静音={:#018x}",
        dry_lr_equal,
        dry_energy,
        armed_lr_equal,
        armed_energy,
        armed_silent.stats.master_insert_frames,
        expected_frames,
        armed_silent.stats.insert_reverb_frames,
        dry.fingerprint(),
        armed_silent.fingerprint(),
    );

    assert!(
        !dry_lr_equal,
        "无设备时硬声相必须保留左右差异（否则 M5 的第一条腿没有对照）"
    );
    assert!(
        armed_lr_equal,
        "master 有已武装器件 ⇒ 母线被下混成单声道并复制两路（裁决 (甲) 的代价）"
    );
    assert_eq!(
        armed_silent.stats.master_insert_frames, expected_frames,
        "武装但静音的 master 器件照样进 3a⁗⁗（每量子一次）"
    );
    assert_eq!(
        armed_silent.stats.insert_reverb_frames, 0,
        "wet=0 ⇒ 混响级不推进（状态不推进、计数 0）"
    );
    assert!(
        dry_energy > 0.0 && armed_energy > 0.0,
        "两条腿都必须真的出声（否则 L==R 可能是两边都静音）"
    );
    assert_ne!(
        dry.fingerprint(),
        armed_silent.fingerprint(),
        "下混本身就会改变硬声相母线的输出 ⇒ 指纹必变"
    );
}
