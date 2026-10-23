//! `[ARCH-PDC-001, ARCH-PDC-002]` 的**读数**判据：
//! `EngineStats::pdc_alignment_frames` 与 `EngineStats::engine_output_latency_frames`
//! 必须**如实**是已武装快照的 PDC 计划的 `L_max` 与引擎输出延迟。
//!
//! # 这条缺口是什么（先量后做）
//!
//! `PdcPlan::output_latency`（= `L_max` ＋ `master` 自身延迟）由"母线限制器延迟回填"
//! 那一条改动（`b6842b0`）引入，`PdcPlan::total_latency`（对齐基准 `L_max`）更早就有。
//! 但 `yeban-engine` 的**运行时**从不读它们：
//!
//! ```text
//! $ grep -rn 'output_latency' crates/yeban-engine/src      # 本次改动前的读数
//! graph.rs    13 行（定义、文档与 getter）
//! mixer.rs     2 行（文档引用）
//! snapshot.rs  5 行（文档与判据里的引用）
//! rt.rs        0 行  ← 运行时没有任何读者
//! ```
//!
//! ⇒ 引擎**知道**自己有多少输出延迟，却没有任何读者能读到它（`EngineStats` 只在
//! 前一版补了"延迟线装不下多少帧"的**计数**，没有延迟**量**）。`[ARCH-PDC-002]` 把
//! "内部 DSP 拓扑调度 1.00 ms"列为端到端监听预算的一格，而那一格在引擎侧此前不可读。
//!
//! # 判据（本文件）
//!
//! | # | 判据（本文件断言什么） |
//! | :-: | :--- |
//! | L1 | **默认档**（关）：两个读数都等于 `PdcPlan` 的 400 / 400；**开启档**：400 / 433，且两者不同 |
//! | L2 | **开关双态**：同一夹具 off/on ⇒ 读数差恰好 33，**音频也真的后移 33**（on[33..] 与 off[..] 逐位相同） |
//! | L3 | 冷值：`EngineRuntime::new` 之后、任何量子之前，两个读数（权威与镜像）都是 0 |
//! | L4 | 快照边界更新 + 镜像静止点等号：换一份延迟表不同的快照 ⇒ 读数换成**新**计划的值；静止点 `mirror.read() == stats()` |
//! | L5 | 内部一致性：逐支路 `armed_pdc_delay(v) == plan.compensation(v)` 且 `L_max − D(v) == arrival(v)`；`输出 ≥ 对齐` |
//!
//! # 注入（本机真跑；逐字的红行见本票报告）
//!
//! | 注入 | 改了什么 | 结果 |
//! | :-: | :--- | :--- |
//! | E1 | 快照边界把 `output_latency` 写成 `total_latency` | 四条判据**全红**（`left: 400 right: 433` 一族） |
//! | E2 | 两个字段的右值互换 | 四条判据**全红**（`left: (433, 400) right: (400, 433)`） |
//! | E3 | 镜像 `publish` 漏存 `engine_output_latency_frames` | `stats_mirror` 的漂移闸门判据红 ＋ 本文件 L3+L4 的镜像等号红 |
//! | E4 | 删掉**开启档**的母线限制器 33 帧回填 | 开启档断言红（`left: 400 right: 433` 一族） |
//! | E5 | 删掉 `EngineRuntime::new` 的**冷镜像发布** | ⛔ **没红**（全 crate 全目标仍绿）：镜像的构造初值与引擎冷值逐字段相同 ⇒ 那条发布是冗余的（见报告） |
//!
//! ⚠ `#302` 起默认档限制器**关**：上表 E1–E4 的逐字红行测于"默认开"的旧口径，
//! 本票**没有**重跑它们 ⇒ 它们不是本票的证据。本票的证据是 L1/L2 的双态读数与
//! `crates/yeban-app/tests/monitor_export_offset.rs` 的 `best_offset = 0 / 33`。
//!
//! ⚠ 覆盖边界（诚实登记）：
//! 1. 跑的是**回调函数体**（`EngineRuntime::process_quantum`），不是 cpal 回调线程
//!    —— 与 `tests/pdc_mix_path.rs` 的边界登记同口径，`default-features = false` 下即可跑。
//! 2. 本判据测的是**读数**，不是"延迟量在样本上正确"；后者由 `tests/pdc_mix_path.rs`
//!    的 P1/P2（逐帧后移比对）负责。
//! 3. 夹具的 `DeviceDefinition::latency_samples` 是**上报值**，引擎的节点处理实际引入 0 帧
//!    （与 `tests/pdc_mix_path.rs` 的边界登记同一条）。

mod support;

use support::{NoteSpec, render_snapshot, two_track_project};
use yeban_engine::graph::LatencyTable;
use yeban_engine::meter::meter_channel;
use yeban_engine::mixer::BUS_LIMITER_LATENCY_FRAMES;
use yeban_engine::ring::event_channel;
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, YebanProjectV1};

/// 夹具音轨的设备链上报延迟（采样点）。随便选一个既非 0、也不等于 33 的值，
/// 这样"对齐基准"与"引擎输出延迟"**不可能**被同一个错误常量同时满足。
const TRACK_LATENCY: u32 = 400;

/// 每条轨一个四分音符（与 `tests/pdc_mix_path.rs` 的栅格同源：120 BPM / 48 kHz）。
fn base_project() -> (YebanProjectV1, EntityId, EntityId) {
    two_track_project(&[NoteSpec::quarter(60)], &[NoteSpec::quarter(64)])
}

/// 给一条轨挂一台**不含任何已识别参数**的内置效果器，只上报延迟。
///
/// 参数为空 ⇒ `InsertParams::from_devices` 得到空链 ⇒ 实时侧整段跳过
/// （见 `yeban_engine::insert` 模块文档 §4 规则 8）⇒ 本夹具的差别**只在延迟表**上。
fn attach_latency_device(project: &mut YebanProjectV1, track: EntityId, frames: u32) {
    let device = DeviceDefinition {
        id: EntityId::new(),
        name: "Reported latency".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: Vec::new(),
        latency_samples: frames,
    };
    project
        .tracks
        .get_mut(&track)
        .expect("夹具轨必须在 tracks 里")
        .devices
        .push(device);
}

/// L1：生产路径的两个读数逐位等于同一份快照的 `PdcPlan` 读数。
#[test]
fn engine_latency_readings_are_the_armed_plans_latency() {
    let (mut project, _first, second) = base_project();
    attach_latency_device(&mut project, second, TRACK_LATENCY);

    // 覆盖度自检：夹具的设备链延迟真的进了延迟表（否则下面全是 0 的空转）。
    let raw = LatencyTable::from_project(&project);
    assert_eq!(
        raw.get(&second),
        TRACK_LATENCY,
        "夹具的第二条轨必须上报 {TRACK_LATENCY} 帧设备延迟"
    );

    // --- 默认（限制器关）：两个读数相等，都是 L_max = 400 ---
    let snapshot = EngineSnapshot::from_project(&project, 1).expect("夹具工程必须能编译成快照");
    let plan_l_max = snapshot.pdc().total_latency();
    let plan_output = snapshot.pdc().output_latency();
    assert_eq!(plan_l_max, TRACK_LATENCY, "对齐基准 = 最长支路 = 400");
    assert_eq!(
        plan_output, TRACK_LATENCY,
        "默认关：引擎输出延迟 = L_max（母线限制器不跑、不回填 33）"
    );
    assert!(!snapshot.limiter_enabled(), "默认快照必须是关的");

    let render = render_snapshot(snapshot, 2);
    assert_eq!(
        render.stats.pdc_alignment_frames, plan_l_max,
        "读数必须等于已武装快照的 `PdcPlan::total_latency()`"
    );
    assert_eq!(
        render.stats.engine_output_latency_frames, plan_output,
        "读数必须等于已武装快照的 `PdcPlan::output_latency()`"
    );
    // 字面值：默认档两个字段都必须是 400。
    assert_eq!(render.stats.pdc_alignment_frames, 400);
    assert_eq!(render.stats.engine_output_latency_frames, 400);
    assert_eq!(
        render.stats.pdc_alignment_frames, render.stats.engine_output_latency_frames,
        "默认关：两个读数相等（没有 master 自身的 33 帧）"
    );
    assert!(
        render.nonzero() > 0,
        "夹具必须真的出声，否则本判据只是在一个静音渲染上读计划"
    );

    // --- 开启（限制器 on）：输出延迟 = 400 + 33，两个读数必须不同 ---
    let on = EngineSnapshot::from_project_with_limiter(&project, 1, true).expect("开启快照");
    let on_plan_l_max = on.pdc().total_latency();
    let on_plan_output = on.pdc().output_latency();
    assert!(
        on.limiter_enabled(),
        "开启快照必须报告 limiter_enabled = true"
    );
    assert_eq!(on_plan_l_max, TRACK_LATENCY, "开启也不改对齐基准");
    assert_eq!(
        on_plan_output,
        TRACK_LATENCY + BUS_LIMITER_LATENCY_FRAMES,
        "开启：引擎输出延迟 = L_max ＋ 回填的 33 帧"
    );
    let on_render = render_snapshot(on, 2);
    assert_eq!(on_render.stats.pdc_alignment_frames, 400);
    assert_eq!(on_render.stats.engine_output_latency_frames, 433);
    assert_ne!(
        on_render.stats.pdc_alignment_frames, on_render.stats.engine_output_latency_frames,
        "开启时两个读数是两个不同的数（差 master 自身的 33 帧）"
    );
    assert!(on_render.nonzero() > 0, "开启态夹具也必须真的出声");

    println!(
        "[engine-latency] L1：默认非零样本={} 对齐={} 输出={}；开启非零样本={} 对齐={} 输出={} \
         （计划 total_latency={}；开启 output_latency={}；母线限制器回填={}）",
        render.nonzero(),
        render.stats.pdc_alignment_frames,
        render.stats.engine_output_latency_frames,
        on_render.nonzero(),
        on_render.stats.pdc_alignment_frames,
        on_render.stats.engine_output_latency_frames,
        plan_l_max,
        on_plan_output,
        BUS_LIMITER_LATENCY_FRAMES,
    );
}

/// L2：**开关的双态判据**（任务单 `#302`）—— 同一夹具只改 `limiter_enabled`：
///
/// - 读数差**恰好 33 帧**（`engine_output_latency_frames`，`pdc_alignment` 不动）；
/// - **音频也真的差 33 帧**：开启态的 `[33..]` 与关闭态的 `[..]` 逐位相同。
///
/// ⚠ 旧名 `output_latency_reading_separates_two_snapshots_that_render_identically`
/// 已不成立：那 33 帧现在**同时**是"读数"和"音频上的真实后移"（默认关、开启 on）。
/// 这正是 `#302` 要的语义：读数不再是一个渲染里看不见的声明。
#[test]
fn limiter_switch_moves_the_reading_and_the_audio_by_exactly_33_frames() {
    let (mut project, _first, second) = base_project();
    attach_latency_device(&mut project, second, TRACK_LATENCY);
    // ⭐ 让限制器**透明**（峰值 < 阈值 0.9）：本判据要量的是"开关带来的 33 帧
    // 时间差"，不是压限量。12 dB 余量足够让中性音符落在阈值以下。
    for track in project.tracks.values_mut() {
        track.volume_db = -12.0;
    }

    let off = EngineSnapshot::from_project(&project, 1).expect("默认（关）快照");
    let on = EngineSnapshot::from_project_with_limiter(&project, 1, true).expect("开启快照");

    // 渲染会**接管**快照所有权 ⇒ 先取出计划读数。
    let alignment_off = off.pdc().total_latency();
    let output_off = off.pdc().output_latency();
    let alignment_on = on.pdc().total_latency();
    let output_on = on.pdc().output_latency();

    let a = render_snapshot(off, 8);
    let b = render_snapshot(on, 8);

    assert!(
        a.nonzero() > 0 && b.nonzero() > 0,
        "两侧都必须真的出声（防空转假绿）"
    );
    assert_eq!(
        (alignment_off, output_off),
        (400, 400),
        "默认关：L_max 与输出延迟都是 400"
    );
    assert_eq!(
        (alignment_on, output_on),
        (400, 433),
        "开启：L_max 仍是 400，输出延迟是 433"
    );
    assert_eq!(
        b.stats.engine_output_latency_frames,
        a.stats.engine_output_latency_frames + BUS_LIMITER_LATENCY_FRAMES,
        "读数之差必须**恰好**是母线限制器的 33 帧"
    );
    assert_eq!(
        a.stats.pdc_alignment_frames, b.stats.pdc_alignment_frames,
        "对齐基准 `L_max` 不受开关影响"
    );

    // 音频真的后移 33 帧：on[33..] == off[..]，逐位比较。
    let frames = a.frames().min(b.frames());
    assert!(
        frames > BUS_LIMITER_LATENCY_FRAMES as usize,
        "渲染太短，比较不到对齐段"
    );
    let mut mismatch = 0usize;
    for index in 0..frames - BUS_LIMITER_LATENCY_FRAMES as usize {
        if b.left[index + BUS_LIMITER_LATENCY_FRAMES as usize].to_bits() != a.left[index].to_bits()
            || b.right[index + BUS_LIMITER_LATENCY_FRAMES as usize].to_bits()
                != a.right[index].to_bits()
        {
            mismatch += 1;
        }
    }
    assert_eq!(
        mismatch, 0,
        "开启态的 [33..] 必须与关闭态的 [..] 逐位相同（实测不匹配 {mismatch} 帧）"
    );
    assert_ne!(
        a.fingerprint(),
        b.fingerprint(),
        "两态的整段指纹必须不同（33 帧后移必须可见）"
    );

    println!(
        "[engine-latency] L2 开关双态：关闭 对齐/输出={}/{}；开启={}/{}；\
         音频后移={} 帧（逐位不匹配={mismatch}）",
        a.stats.pdc_alignment_frames,
        a.stats.engine_output_latency_frames,
        b.stats.pdc_alignment_frames,
        b.stats.engine_output_latency_frames,
        BUS_LIMITER_LATENCY_FRAMES,
    );
}

/// L3 + L4 + L5：冷值、快照边界更新、镜像静止点等号、与逐节点 `D(v)` 的自洽。
#[test]
fn engine_latency_readings_track_the_armed_snapshot_and_the_mirror() {
    let (mut project, _first, second) = base_project();
    attach_latency_device(&mut project, second, TRACK_LATENCY);

    let snapshot = EngineSnapshot::from_project(&project, 1).expect("夹具快照");
    let slot = SnapshotSlot::new(snapshot);
    let (retire, mut queue) = retire_channel(8);
    let (_sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(4096);
    let mut runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
    let mirror = runtime.stats_mirror();
    let mut output = vec![0.0f32; 128 * 2];

    // ---- L3：还没有任何快照被处理 ⇒ 两个读数（权威与镜像）都是 0 ----
    let cold = runtime.stats();
    assert_eq!(
        (cold.pdc_alignment_frames, cold.engine_output_latency_frames),
        (0, 0),
        "冷值必须是 0（还没有任何计划被武装）"
    );
    let cold_mirror = mirror.read();
    assert_eq!(
        (
            cold_mirror.pdc_alignment_frames,
            cold_mirror.engine_output_latency_frames
        ),
        (0, 0),
        "冷镜像的两个新读数也必须是 0"
    );

    // ---- L5 的一半：武装之后读数与逐节点 D(v) 自洽 ----
    runtime.process_quantum(&mut output, 2);
    let first = runtime.stats();
    assert_eq!(
        (
            first.pdc_alignment_frames,
            first.engine_output_latency_frames
        ),
        (TRACK_LATENCY, TRACK_LATENCY),
        "默认关：第一个量子之后两个读数都必须是 L_max（没有 33 回填）"
    );
    assert!(
        first.engine_output_latency_frames >= first.pdc_alignment_frames,
        "引擎输出延迟不可能小于对齐基准"
    );
    // ---- L5：武装之后读数与**逐节点的计划 `D(v)`** 自洽 ----
    //
    // 这一条把"读数"与"延迟线池真的武装了什么"绑在一起：`armed_pdc_delay` 是池的
    // 读数，`PdcPlan::compensation` / `arrival` 是计划的读数，两者必须逐节点相等。
    let master = project.master_bus_track_id;
    let plan = EngineSnapshot::from_project(&project, 1).expect("同一份计划");
    let l_max = first.pdc_alignment_frames as usize;
    let mut checked = 0usize;
    for track in project.tracks.keys().filter(|track| **track != master) {
        let armed = runtime
            .armed_pdc_delay(track)
            .expect("可达 master 的支路必须有延迟线");
        assert_eq!(
            Some(armed),
            plan.pdc().compensation(track).map(|d| d as usize),
            "节点 {track:?} 武装的补偿量必须等于计划的 D(v)"
        );
        assert_eq!(
            l_max - armed,
            plan.pdc()
                .arrival(track)
                .expect("不可达 master 的节点没有 arrival") as usize,
            "内部一致性：L_max − D(v) 必须等于 arrival(v)"
        );
        checked += 1;
    }
    assert_eq!(checked, 2, "夹具的两条支路都必须被核对（防空转）");
    // 夹具见证：挂延迟设备的那条轨在关键路径上 ⇒ D = 0；另一条 D = L_max。
    assert_eq!(
        runtime.armed_pdc_delay(&second),
        Some(0),
        "关键路径上的支路补偿量是 0"
    );

    // ---- L4：换一份延迟表不同的快照 ⇒ 读数换成**新**计划的值 ----
    let mut next_table = LatencyTable::from_project(&project);
    next_table.add(second, 100);
    let next = EngineSnapshot::from_project_with_latencies(&project, 2, &next_table)
        .expect("新计划的快照");
    let (next_alignment, next_output) = (next.pdc().total_latency(), next.pdc().output_latency());
    assert_eq!(
        (next_alignment, next_output),
        (500, 500),
        "新计划：设备链 400 + 100 = 500，且显式注入路径不追加那 33 帧"
    );
    slot.publish(next);
    runtime.process_quantum(&mut output, 2);

    let updated = runtime.stats();
    assert_eq!(
        (
            updated.pdc_alignment_frames,
            updated.engine_output_latency_frames
        ),
        (next_alignment, next_output),
        "读数必须跟着**新**快照的计划走，而不是停在第一次武装的值上"
    );
    assert_ne!(
        updated.engine_output_latency_frames, first.engine_output_latency_frames,
        "两次武装的输出延迟读数必须不同（否则本判据没有判别力）"
    );

    // ---- L4 的第二半：静止点上镜像与权威读数**逐字段相等** ----
    let authoritative = runtime.stats();
    assert_eq!(
        mirror.read(),
        authoritative,
        "静止点上镜像必须与权威读数逐字段相等（含两个新读数）"
    );
    assert_eq!(
        mirror.read().engine_output_latency_frames,
        next_output,
        "镜像里的新读数必须是新计划的值"
    );

    queue.drain(8);
    println!(
        "[engine-latency] L3/L4/L5：冷值=({}, {}) → 首次武装=({}, {}) → 换表后=({}, {})；\
         逐支路 D(v) 与计划自洽（核对 {} 条）且 L_max={}；静止点镜像 == 权威",
        cold.pdc_alignment_frames,
        cold.engine_output_latency_frames,
        first.pdc_alignment_frames,
        first.engine_output_latency_frames,
        updated.pdc_alignment_frames,
        updated.engine_output_latency_frames,
        checked,
        l_max,
    );
}

/// 反向自检：**没有**任何设备延迟上报时，开关仍必须可见：
/// 默认关 ⇒ 两个读数都是 0；显式开启 ⇒ 引擎输出延迟是 33、对齐基准仍是 0。
///
/// 这条把"读数是计划的投影"与"读数是某个常数"分开：同一个夹具下只有
/// `engine_output_latency_frames` 随开关变化 ⇒ 两者不能被同一个硬编码值满足。
#[test]
fn limiter_switch_is_visible_without_any_reported_device_latency() {
    let (project, first, second) = base_project();
    assert!(
        project.tracks[&first].devices.is_empty() && project.tracks[&second].devices.is_empty(),
        "本夹具刻意不带任何设备"
    );
    let raw = LatencyTable::from_project(&project);
    assert!(
        raw.iter().all(|(_, frames)| *frames == 0),
        "没有设备上报延迟 ⇒ 原始延迟表全 0"
    );

    let render = support::render(&project, 2);
    assert_eq!(render.stats.pdc_alignment_frames, 0, "对齐基准 = 0");
    assert_eq!(
        render.stats.engine_output_latency_frames, 0,
        "默认关：没有设备延迟 ⇒ 引擎输出延迟也是 0"
    );

    let on = EngineSnapshot::from_project_with_limiter(&project, 1, true).expect("开启快照");
    let on_render = render_snapshot(on, 2);
    assert_eq!(
        on_render.stats.pdc_alignment_frames, 0,
        "开启也不改对齐基准"
    );
    assert_eq!(
        on_render.stats.engine_output_latency_frames, BUS_LIMITER_LATENCY_FRAMES,
        "开启：引擎输出延迟必须是母线限制器的 33 帧"
    );
    assert_eq!(
        BUS_LIMITER_LATENCY_FRAMES, 33,
        "母线限制器的前瞻环长是 33 帧"
    );
    // 夹具自检：两条轨确实不同、且工程是三节点图（主总线 ＋ 两条轨）。
    assert_ne!(first, second);
    assert_eq!(project.tracks.len(), 3);

    println!(
        "[engine-latency] 反向自检：无设备延迟工程 ⇒ 默认对齐/输出={}/{}；开启对齐/输出={}/{}（= 母线限制器前瞻环长）",
        render.stats.pdc_alignment_frames,
        render.stats.engine_output_latency_frames,
        on_render.stats.pdc_alignment_frames,
        on_render.stats.engine_output_latency_frames,
    );
}
