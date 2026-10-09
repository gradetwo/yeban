//! `line/engine-6` 的端到端判据：**实时侧参数目标表**（`crate::param` → `rt`）。
//! [ARCH-DSP-001, ARCH-RT-001, ROAD-M2-007]
//!
//! `line/engine-9` 追加了**主总线槽位**（`MASTER_GAIN_SLOT`）的判据 P9..P12；
//! 逐轨槽位的全部既有判据（P0..P8）**一字未改**。
//!
//! 全部判据断言在**真实渲染路径**产出的样本上：`YebanProjectV1` →
//! `EngineSnapshot::from_project` → `SnapshotSlot` → 事件 SPSC →
//! `EngineRuntime::process_quantum`。`support` 模块刻意不给测试专用捷径。
//!
//! 本文件回答三个问题（与 `crate::param` 的模块文档一一对应）：
//!
//! 1. **接线不改变既有输出**：没有事件、地址未映射、值非法、目标恰为 `1.0`
//!    这四种口径下的输出与"接线前"**逐位相同**（判据 P0 / P4 / P5 / P11）；
//! 2. **自动化真的改变了声音**：被接受的乘子**逐位**地作用到输出上
//!    （`0.25` 是 2 的幂 ⇒ `armed == unarmed · 0.25` 是**精确等式**，判据 P2 / P9）；
//! 3. **改变是平滑的**：阶跃目标不许在第一个样本上跳过去（判据 P3 / P10）。
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | P0 | 没有事件 / 地址未映射（槽位号不是 0、实体是主总线）/ 值非法（`NaN`、负数）/ 目标恰为 `1.0` 这四种口径与"接线前"**逐字节相同** | 把 `params.apply` 的调用移出"有槽位且非恒等"的守卫 / 对未映射地址也建槽位 / 把非法值钳成 `0.0`（静音） |
//! | P1 | 被接受的事件真的**建了槽位**并武装了目标（`armed_param_slot_count` / `armed_param_target` 的读数与事件逐值相等） | 事件只计数不建槽位（`accept` 不落表） |
//! | P2 | **乘子逐位生效**：平滑走完之后 `armed[i] == unarmed[i] · 0.25`（2 的幂 ⇒ 精确）；`param_gain_frames` 等于被乘过的帧数 | 忘了把增益乘进样本 / 乘错位置 / 计数不落账 |
//! | P3 | **平滑而不是阶跃**：目标 `1.0 → 0.25` 之后，第一个样本的相对变化 `< 1 %`，且比值单调不增、最终收敛到 `0.25` | 去掉单极点低通（`set_target` 直接 `snap_to`） |
//! | P4 | 非法值被**忽略并计数**（`param_gain_rejects`），目标与输出都不动 | 非法值也 `set_target` / 不计数 |
//! | P5 | 未映射的地址被**计数**（`param_unmapped_events`）且不建槽位 | 静默丢弃 / 给未映射地址建槽位 |
//! | P6 | 槽位表满时不静默：第 17 个实体被计数（`param_capacity_drops`），前 16 个仍可用 | 容量不足时静默丢弃 / 无限建槽位 |
//! | P7 | **确定性**：同样的工程 + 同样的事件序列 ⇒ 两次独立装配逐位相同 | 引入真熵源 / 让事件顺序影响结果 |
//! | P8 | 换采样率的快照之后**自动化仍然生效**（目标保留、平滑器跟随新采样率） | 换采样率时把平滑器复位（`snap_to(1.0)`） |
//! | P9 | **主总线乘子逐位生效**且**两条声道共用同一个增益**：平滑走完之后 `armed[i] == unarmed[i] · 0.25`（左右各自成立）；`param_master_gain_frames` 等于被乘过的帧数、`param_gain_frames` 仍为 0 | 忘了在主总线路径上乘 / 只乘一条声道 / 左右各自平滑 / 计数不落账 |
//! | P10 | 主总线的阶跃目标同样**被平滑**（第一个帧的相对变化 `< 1 %`，比值单调不增，最终收敛到 `0.25`） | 主总线路径走 `snap_to`（旁路单极点低通） |
//! | P11 | 主总线槽位的地址空间：一条轨带 `MASTER_GAIN_SLOT`、主总线带 `TRACK_GAIN_SLOT`、主总线带别的槽位号、主总线的非法值 —— 四种口径的输出与接线前**逐字节相同**且被**计数**（不占逐轨槽位） | 串用两个槽位号也接受 / 非法值也 `set_target` / 不计数 |
//! | P12 | **主总线自动化的确定性**：同样的工程 + 同样的事件序列 ⇒ 两次独立装配的两条声道都逐位相同 | 引入真熵源 / 让事件顺序影响结果 |
//!
//! ## 覆盖范围（**明说**）
//!
//! 本文件覆盖 `crate::param` 的槽位裁决与 `rt.rs` 的**事件边界**（`accept`）、
//! **快照边界**（`set_sample_rate` 与主总线身份）与**逐样本应用**（`apply` 与
//! `apply_master`）。它**不**覆盖：`yeban_dsp::smoothing` 内部的一阶低通正确性
//! （那是该模块自己的单元判据）、参数曲线的插值（那是
//! `yeban_model::automation_value_at` 的职责，`tests/rt_zero_alloc.rs` 场景 ⑤
//! 已经在控制侧调它）、以及**声相**与**插入器件参数**的自动化槽位（本模块
//! 模块文档 §6 与 §7 的 P4 明说它们没有开）。

mod support;

use support::{NoteSpec, note_project, render};
use yeban_engine::meter::meter_channel;
use yeban_engine::mixer::BUS_LIMITER_LATENCY_FRAMES;
use yeban_engine::param::{MASTER_GAIN_SLOT, PARAM_SLOTS, TRACK_GAIN_SLOT};
use yeban_engine::ring::{EngineEvent, ParamAddress, event_channel};
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};
use yeban_model::{EntityId, YebanProjectV1};

/// 判据用的音符：一个 480 tick（= 12 000 样本）的音符，随后 `QUANTA` 个量子。
///
/// 120 BPM / 48 kHz / 960 PPQ ⇒ 1 tick = 25 样本（见 `support` 模块文档）。
const NOTE: NoteSpec = NoteSpec::at(0, 480, 60, 100);

/// 渲染量子数：400 × 128 = 51 200 帧 = 1.0667 s。
///
/// 比平滑器走完一个 `1.0 → 0.25` 阶跃所需的样本数（≈ 2 700，见 `crate::param` 的
/// 单元判据）大一个量级 ⇒ "走完之后"的窗口在本判据里一定存在。
const QUANTA: usize = 400;

/// 判据 P3 观察的首窗（样本下标）：事件在第 1 个量子之前发布。
const RAMP: core::ops::Range<usize> = 0..3_000;

/// 装配：保留事件生产端 ⇒ 可以在量子之间真的发 `SetParam`。
///
/// 与 `support::Runtime` 的差别只有一处：那个刻意丢掉生产端。
struct ParamRig {
    slot: std::sync::Arc<SnapshotSlot>,
    queue: yeban_engine::snapshot::RetireQueue,
    sender: yeban_engine::ring::EventSender,
    runtime: EngineRuntime,
    output: Vec<f32>,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl ParamRig {
    fn new(project: &YebanProjectV1, revision: u64) -> Self {
        let snapshot = EngineSnapshot::from_project(project, revision).expect("夹具工程必须能编译");
        let slot = SnapshotSlot::new(snapshot);
        let (retire, queue) = retire_channel(64);
        let (sender, receiver) = event_channel(64);
        let (publisher, _collector) = meter_channel(4096);
        let runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
        Self {
            slot,
            queue,
            sender,
            runtime,
            output: vec![0.0f32; 128 * 2],
            left: Vec::new(),
            right: Vec::new(),
        }
    }

    /// 发**一条**参数事件（批量 API 的一次调用）。
    fn set_param(&mut self, entity: EntityId, slot: u16, value: f32) -> usize {
        self.sender.publish(&[EngineEvent::SetParam {
            target: ParamAddress::new(entity, slot),
            value,
        }])
    }

    /// 推一个量子并把结果累加进两条声道。
    fn quantum(&mut self) {
        self.output.fill(0.0);
        self.runtime.process_quantum(&mut self.output, 2);
        for frame in self.output.as_chunks::<2>().0 {
            self.left.push(frame[0]);
            self.right.push(frame[1]);
        }
    }

    fn quanta(&mut self, count: usize) {
        for _ in 0..count {
            self.quantum();
        }
    }
}

/// 把一条轨道的左声道原样渲染（**没有**任何事件）——"接线前"的参照。
fn unarmed_left(project: &YebanProjectV1) -> Vec<f32> {
    render(project, QUANTA).left
}

/// 判据 P0：四种"本表不改变音频"的口径与接线前**逐字节相同**。
#[test]
fn the_default_paths_are_bit_identical_to_no_wiring() {
    let fixture = note_project(&[NOTE]);
    let reference = unarmed_left(&fixture.project);
    assert_eq!(reference.len(), QUANTA * 128);
    assert!(
        reference.iter().any(|sample| *sample != 0.0),
        "参照渲染必须真的出声（否则本判据是空转）"
    );

    // ① 完全没有事件。
    let mut idle = ParamRig::new(&fixture.project, 1);
    idle.quanta(QUANTA);
    assert_eq!(idle.left, reference, "没有事件时必须逐位相同");
    assert_eq!(idle.runtime.stats().param_gain_frames, 0);

    // ② 未映射的地址（槽位号不是 0 / 实体是主总线）。
    let mut unmapped = ParamRig::new(&fixture.project, 1);
    unmapped.set_param(fixture.track, TRACK_GAIN_SLOT + 1, 0.25);
    unmapped.set_param(fixture.master, TRACK_GAIN_SLOT, 0.25);
    unmapped.quanta(QUANTA);
    assert_eq!(unmapped.left, reference, "未映射的地址不许改变音频");
    assert_eq!(unmapped.runtime.stats().param_unmapped_events, 2);
    assert_eq!(unmapped.runtime.armed_param_slot_count(), 0);

    // ③ 非法值（非有限 / 负数）。
    let mut illegal = ParamRig::new(&fixture.project, 1);
    illegal.set_param(fixture.track, TRACK_GAIN_SLOT, f32::NAN);
    illegal.set_param(fixture.track, TRACK_GAIN_SLOT, -1.0);
    illegal.quanta(QUANTA);
    assert_eq!(illegal.left, reference, "非法值不许改变音频");
    assert_eq!(illegal.runtime.stats().param_gain_rejects, 2);
    assert_eq!(illegal.runtime.armed_param_slot_count(), 0);

    // ④ 目标恰为恒等 `1.0`（建槽位但走恒等快路径）。
    let mut identity = ParamRig::new(&fixture.project, 1);
    identity.set_param(fixture.track, TRACK_GAIN_SLOT, 1.0);
    identity.quanta(QUANTA);
    assert_eq!(identity.left, reference, "恒等目标不许改变音频");
    assert_eq!(identity.runtime.armed_param_slot_count(), 1);
    assert_eq!(identity.runtime.stats().param_gain_frames, 0);
}

/// 判据 P1 + P2：被接受的事件建槽位、改输出，且乘子**逐位**生效。
#[test]
fn an_accepted_gain_multiplies_the_render_bit_for_bit() {
    let fixture = note_project(&[NOTE]);
    let reference = unarmed_left(&fixture.project);

    let mut rig = ParamRig::new(&fixture.project, 1);
    let published = rig.set_param(fixture.track, TRACK_GAIN_SLOT, 0.25);
    assert_eq!(published, 1, "事件必须真的进队列");
    rig.quanta(QUANTA);

    let stats = rig.runtime.stats();
    assert_eq!(
        rig.runtime.armed_param_slot_count(),
        1,
        "被接受的事件必须建一个槽位"
    );
    assert_eq!(rig.runtime.armed_param_target(&fixture.track), Some(0.25));
    assert_eq!(
        rig.runtime.armed_param_gain(&fixture.track),
        Some(0.25),
        "平滑必须已经吸附到目标（本窗口 51 200 帧 ≫ 2 700 帧）"
    );
    assert_ne!(
        rig.left, reference,
        "被接受的乘子必须真的改变输出（否则就是'只计数'）"
    );
    assert_eq!(
        stats.param_gain_frames,
        QUANTA as u64 * 128,
        "见证读数必须等于被乘过的帧数"
    );
    assert_eq!(stats.param_unmapped_events, 0);
    assert_eq!(stats.param_gain_rejects, 0);
    assert_eq!(stats.param_capacity_drops, 0);

    // `0.25` 是 2 的幂 ⇒ 逐位精确：`armed == unarmed · 0.25`（不是容差比较）。
    let settled = 3_000..QUANTA * 128;
    let mut checked = 0usize;
    for index in settled {
        assert_eq!(
            rig.left[index],
            reference[index] * 0.25,
            "第 {index} 个样本：乘子必须逐位生效"
        );
        checked += 1;
    }
    assert!(
        checked > 40_000,
        "窗口必须覆盖绝大多数样本（实得 {checked}）"
    );
}

/// 判据 P3：阶跃目标**不许**在第一个样本上跳过去 —— 单极点低通必须有牙。
#[test]
fn a_step_target_is_smoothed_rather_than_jumped() {
    let fixture = note_project(&[NOTE]);
    let reference = unarmed_left(&fixture.project);

    let mut rig = ParamRig::new(&fixture.project, 1);
    // 让第一个量子先把场景暖起来（声部起音），再在第 2 个量子之前发目标。
    rig.quanta(1);
    rig.set_param(fixture.track, TRACK_GAIN_SLOT, 0.25);
    rig.quanta(QUANTA - 1);

    // ⚠ 对齐：母线限制器有 `BUS_LIMITER_LATENCY_FRAMES`（33 帧）的**前瞻延迟**，
    // 因此输出样本 `n` 对应的是 `track_scratch` 的样本 `n − 33`。本判据要读"第一个被乘的
    // 输出样本"，就必须把这 33 帧算进去，否则读到的是乘法**之前**的历史（实测：
    // 不加这 33 帧时首个被乘的输出样本是下标 161，而不是 128）。
    let offset = 128 + BUS_LIMITER_LATENCY_FRAMES as usize;
    let ratio = |index: usize| rig.left[index + offset] / reference[index + offset];
    let first = ratio(0);
    let first_diff = rig
        .left
        .iter()
        .zip(reference.iter())
        .position(|(armed, plain)| armed != plain);
    assert_eq!(
        first_diff,
        Some(offset),
        "第一个被乘的输出样本必须正好在 128 + 33 = {offset}（实得 {first_diff:?}）"
    );
    assert!(
        first > 0.99,
        "第一个样本的相对变化必须 < 1 %（实得 {first}）—— 阶跃就是这里变红"
    );
    let mut previous = first;
    for index in 1..RAMP.len() {
        let current = ratio(index);
        assert!(
            current <= previous + f32::EPSILON,
            "比值必须单调不增（第 {index} 个样本：{previous} → {current}）"
        );
        assert!(current >= 0.25, "比值不许越过目标（第 {index} 个样本）");
        previous = current;
    }
    // 收敛：窗口末尾的比值必须已经到目标（同一窗口的 P2 已经断言了逐位相等）。
    assert_eq!(
        rig.left[QUANTA * 128 - 1],
        reference[QUANTA * 128 - 1] * 0.25
    );
}

/// 判据 P6：槽位表满时不静默；前 [`PARAM_SLOTS`] 个实体仍然各自可用。
#[test]
fn a_full_table_is_counted_and_earlier_slots_keep_working() {
    let fixture = note_project(&[NOTE]);
    let mut rig = ParamRig::new(&fixture.project, 1);
    // 第一个实体是**真的**那条轨（因此音频确实被乘），其余是随机实体
    // （`accept` 只按地址建槽位，不要求实体在快照里 —— 那是模块文档 §4 的口径）。
    rig.set_param(fixture.track, TRACK_GAIN_SLOT, 0.5);
    for _ in 1..PARAM_SLOTS {
        rig.set_param(EntityId::new(), TRACK_GAIN_SLOT, 0.5);
    }
    let overflow = EntityId::new();
    rig.set_param(overflow, TRACK_GAIN_SLOT, 0.5);
    rig.quanta(QUANTA);

    let stats = rig.runtime.stats();
    assert_eq!(rig.runtime.armed_param_slot_count(), PARAM_SLOTS);
    assert_eq!(
        stats.param_capacity_drops,
        1,
        "第 {} 个实体必须被计数（不静默）",
        PARAM_SLOTS + 1
    );
    assert_eq!(
        rig.runtime.armed_param_target(&overflow),
        None,
        "容量不足的实体不许有槽位"
    );
    assert_eq!(rig.runtime.armed_param_target(&fixture.track), Some(0.5));
    assert!(stats.param_gain_frames > 0, "真轨的乘子必须仍然生效");
}

/// 判据 P7：同样的工程 + 同样的事件序列 ⇒ 两次独立装配逐位相同。
#[test]
fn the_same_event_sequence_is_bit_identical_across_assemblies() {
    let fixture = note_project(&[NOTE]);
    let run = || {
        let mut rig = ParamRig::new(&fixture.project, 1);
        rig.set_param(fixture.track, TRACK_GAIN_SLOT, 0.5);
        rig.quanta(64);
        rig.set_param(fixture.track, TRACK_GAIN_SLOT, 0.125);
        rig.quanta(64);
        rig.left
    };
    let first = run();
    let second = run();
    assert_eq!(first.len(), 128 * 128);
    assert_eq!(first, second, "同样的输入必须逐位相同 [ARCH-DET-001]");
    assert!(first.iter().any(|sample| *sample != 0.0), "不许是空转");
}

/// 判据 P8：换采样率的快照之后自动化仍然生效（目标保留、平滑器跟随新采样率）。
#[test]
fn automation_survives_a_sample_rate_change() {
    let fixture = note_project(&[NOTE]);
    let mut shifted = fixture.project.clone();
    shifted.audio_config.sample_rate = yeban_model::SampleRate::Hz44100;

    let mut rig = ParamRig::new(&fixture.project, 1);
    rig.set_param(fixture.track, TRACK_GAIN_SLOT, 0.25);
    rig.quanta(QUANTA);
    assert_eq!(rig.runtime.armed_param_target(&fixture.track), Some(0.25));

    // 换采样率：快照边界把新采样率同步给**已分配**的平滑器（`α` 重算，含 `exp`）。
    let snapshot = EngineSnapshot::from_project(&shifted, 2).expect("换采样率快照");
    rig.slot.publish(snapshot);
    rig.quanta(QUANTA);

    assert_eq!(
        rig.runtime.armed_param_target(&fixture.track),
        Some(0.25),
        "换采样率不许丢掉已武装的目标"
    );
    assert_eq!(
        rig.runtime.armed_param_gain(&fixture.track),
        Some(0.25),
        "换采样率之后平滑器必须仍然收敛到目标"
    );
    let _ = rig.queue.drain(64);
}

// ---------------------------------------------------------------------------
// P9..P12（`line/engine-9`）：**主总线槽位**（`MASTER_GAIN_SLOT`）
// ---------------------------------------------------------------------------

/// 主总线槽位的参照渲染：两条声道都要（P9 断言"左右用同一个增益"）。
fn unarmed_stereo(project: &YebanProjectV1) -> (Vec<f32>, Vec<f32>) {
    let render = render(project, QUANTA);
    (render.left, render.right)
}

/// 判据 P9 + P10：主总线上的被接受事件**逐位**改变整条母线，
/// 两条声道共用同一个平滑增益，且阶跃**不是**跳变。
#[test]
fn a_master_gain_multiplies_the_whole_bus_bit_for_bit_and_smoothly() {
    let fixture = note_project(&[NOTE]);
    let (reference_left, reference_right) = unarmed_stereo(&fixture.project);
    assert_eq!(reference_left.len(), QUANTA * 128);
    assert!(
        reference_left.iter().any(|sample| *sample != 0.0),
        "参照渲染必须真的出声（否则本判据是空转）"
    );

    let mut rig = ParamRig::new(&fixture.project, 1);
    // 与判据 P3 同一条对齐纪律：先让第 1 个量子把场景暖起来，再在**第 2 个量子之前**
    // 发布目标 ⇒ 变化从 `track_scratch` 的下标 128 起算；母线限制器的 33 帧前瞻
    // 让"第一个被乘的输出样本"落在 `128 + 33`（详见 P3 的对齐注释）。
    rig.quanta(1);
    let published = rig.set_param(fixture.master, MASTER_GAIN_SLOT, 0.25);
    assert_eq!(published, 1, "事件必须真的进队列");
    rig.quanta(QUANTA - 1);

    let offset = 128 + BUS_LIMITER_LATENCY_FRAMES as usize;
    let ratio = |index: usize| rig.left[index + offset] / reference_left[index + offset];
    let first_diff = rig
        .left
        .iter()
        .zip(reference_left.iter())
        .position(|(armed, plain)| armed != plain);
    assert_eq!(
        first_diff,
        Some(offset),
        "第一个被乘的输出样本必须正好在 128 + 33 = {offset}（实得 {first_diff:?}）"
    );

    // ---- P10：平滑而不是阶跃（主总线路径也必须走单极点低通）----
    let first = ratio(0);
    assert!(
        first > 0.99,
        "主总线的阶跃不许在第一个帧上跳过去（实得比值 {first}）"
    );
    let mut previous = first;
    for index in 1..RAMP.len() {
        let current = ratio(index);
        assert!(
            current <= previous + f32::EPSILON,
            "第 {index} 个帧：比值必须单调不增（{previous} → {current}）"
        );
        assert!(current >= 0.25, "比值不许越过目标（第 {index} 个帧）");
        previous = current;
    }

    // ---- P9：平滑走完之后逐位生效，两条声道共用同一个增益 ----
    let stats = rig.runtime.stats();
    assert_eq!(
        rig.runtime.armed_master_param_target(),
        Some(0.25),
        "被接受的事件必须武装主总线目标"
    );
    assert_eq!(
        rig.runtime.armed_master_param_gain(),
        Some(0.25),
        "平滑必须已经吸附到目标（本窗口 51 200 帧 ≫ 2 700 帧）"
    );
    assert_ne!(
        rig.left, reference_left,
        "主总线上的乘子必须真的改变输出（否则就是'只计数'）"
    );
    assert_eq!(
        stats.param_master_gain_frames,
        (QUANTA - 1) as u64 * 128,
        "主总线见证读数必须等于被乘过的帧数（第 1 个量子在事件之前 ⇒ 少 128 帧）"
    );
    assert_eq!(
        stats.param_gain_frames, 0,
        "本判据没有逐轨事件 ⇒ 逐轨见证必须仍是 0（两个读数不许互相冒充）"
    );
    assert_eq!(
        rig.runtime.armed_param_slot_count(),
        0,
        "主总线不是逐轨槽位表里的一项"
    );
    assert_eq!(stats.param_unmapped_events, 0);
    assert_eq!(stats.param_gain_rejects, 0);
    assert_eq!(stats.param_capacity_drops, 0);

    let settled = 3_000..QUANTA * 128;
    let mut checked = 0usize;
    for index in settled {
        // `0.25` 是 2 的幂 ⇒ 逐位精确（不是容差比较）。
        assert_eq!(
            rig.left[index],
            reference_left[index] * 0.25,
            "第 {index} 个样本：左声道上的主总线乘子必须逐位生效"
        );
        assert_eq!(
            rig.right[index],
            reference_right[index] * 0.25,
            "第 {index} 个样本：右声道上的主总线乘子必须逐位生效"
        );
        checked += 1;
    }
    assert!(
        checked > 40_000,
        "窗口必须覆盖绝大多数样本（实得 {checked}）"
    );
}

/// 判据 P11：主总线槽位的**地址空间**与非法值 —— 四种口径都与接线前逐字节相同。
#[test]
fn the_master_slot_address_space_rejects_crossed_slots_and_illegal_values() {
    let fixture = note_project(&[NOTE]);
    let (reference_left, reference_right) = unarmed_stereo(&fixture.project);

    // ① 一条轨带 MASTER_GAIN_SLOT；② 主总线带 TRACK_GAIN_SLOT；
    // ③ 主总线带别的槽位号；④ 主总线的非法值（`NaN` 与负数）。
    let mut rig = ParamRig::new(&fixture.project, 1);
    rig.set_param(fixture.track, MASTER_GAIN_SLOT, 0.25);
    rig.set_param(fixture.master, TRACK_GAIN_SLOT, 0.25);
    rig.set_param(fixture.master, MASTER_GAIN_SLOT + 1, 0.25);
    rig.set_param(fixture.master, MASTER_GAIN_SLOT, f32::NAN);
    rig.set_param(fixture.master, MASTER_GAIN_SLOT, -1.0);
    rig.quanta(QUANTA);

    assert_eq!(
        rig.left, reference_left,
        "未映射/非法的主总线事件不许改变左声道"
    );
    assert_eq!(
        rig.right, reference_right,
        "未映射/非法的主总线事件不许改变右声道"
    );
    let stats = rig.runtime.stats();
    assert_eq!(
        stats.param_unmapped_events, 3,
        "三个串用/越界的槽位必须被计数"
    );
    assert_eq!(stats.param_gain_rejects, 2, "两个非法值必须被计数");
    assert_eq!(
        stats.param_master_gain_frames, 0,
        "一个被接受的主总线事件都没有 ⇒ 见证必须为 0"
    );
    assert_eq!(
        rig.runtime.armed_master_param_target(),
        None,
        "被拒/未映射的事件不许武装主总线目标"
    );
    assert_eq!(rig.runtime.armed_param_slot_count(), 0);
}

/// 判据 P12：主总线自动化的确定性 —— 同样的输入两次装配逐位相同（两条声道）。
#[test]
fn the_master_event_sequence_is_bit_identical_across_assemblies() {
    let fixture = note_project(&[NOTE]);
    let run = || {
        let mut rig = ParamRig::new(&fixture.project, 1);
        rig.set_param(fixture.master, MASTER_GAIN_SLOT, 0.5);
        rig.quanta(64);
        rig.set_param(fixture.master, MASTER_GAIN_SLOT, 0.125);
        rig.quanta(64);
        (rig.left, rig.right)
    };
    let (first_left, first_right) = run();
    let (second_left, second_right) = run();
    assert_eq!(first_left.len(), 128 * 128);
    assert_eq!(
        first_left, second_left,
        "同样的输入必须逐位相同 [ARCH-DET-001]"
    );
    assert_eq!(first_right, second_right);
    assert!(first_left.iter().any(|sample| *sample != 0.0), "不许是空转");
}
