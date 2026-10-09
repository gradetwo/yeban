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
//! | P13 | **先换采样率、后武装主总线槽位**：新采样率上的 `α` 与"一开始就在该采样率"逐位相同（`armed_master_param_gain` 等号） | 快照边界只把新采样率转发给**已武装**的主总线槽位（判据 ⑧ 钉住的那条不对称） |
//! | P14 | **参数极值不得让母线出现非有限样本**：逐轨槽与主总线槽各推 `1.0e20`（两个合法值）⇒ 母线上必然 `±∞`，但输出必须**全有限**（`NaN` 与非有限样本各 0 条） | 删掉 `render_block` 里 `saturate_bus_to_finite(...)` 那一行**调用**（函数本体的判据都在 `src/rt.rs`，全是直接调用，**没有**一条走调用点） |
//! | P15 | **两个主总线乘子的顺序是契约**：静态推子 `scale_bus`（步骤 3a''）在**前**、自动化乘子 `apply_master`（步骤 3a'''）在**后** ⇒ `g` 与 `m` 都非恒等且都**不是** 2 的幂时，`both == fl(fader · m)` 逐位 | 把 `scale_bus(block, master_gain)` 挪到 `apply_master(...)` **之后**（既有主总线判据全部取 2 的幂目标 ⇒ 逐位同解；本票实测全量 24 个目标全绿） |
//!
//! ## 覆盖范围（**明说**）
//!
//! 本文件覆盖 `crate::param` 的槽位裁决与 `rt.rs` 的**事件边界**（`accept`）、
//! **快照边界**（`set_sample_rate` 与主总线身份）与**逐样本应用**（`apply` 与
//! `apply_master`）。P13（`line/engine-18`）补上了"两个边界之间的**顺序**"这一维。
//! 它**不**覆盖：`yeban_dsp::smoothing` 内部的一阶低通正确性
//! （那是该模块自己的单元判据）、参数曲线的插值（那是
//! `yeban_model::automation_value_at` 的职责，`tests/rt_zero_alloc.rs` 场景 ⑤
//! 已经在控制侧调它）、以及**声相**与**插入器件参数**的自动化槽位（本模块
//! 模块文档 §6 与 §7 的 P4 明说它们没有开）。

mod support;

use support::{NoteSpec, note_project, render};
use yeban_engine::level::MAX_LINEAR_MAGNITUDE;
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

/// 判据 P13：**先换采样率、后武装主总线槽位**时，平滑器的 `α` 必须已经是新采样率的
/// —— 也就是与"一开始就构造在该采样率"的那一次运行**逐位相同**。
///
/// 为什么这个顺序是**真实可达**的：`ParamTable::set_sample_rate` 的调用点在**快照边界**
/// （`rt.rs` 的步骤 2a'），而主总线槽位的武装点在**事件边界**（步骤 1 的
/// `params.accept`），两者之间隔着"换了快照采样率但还没有收到过任何主总线目标"这个
/// 状态。同一条纪律下，逐轨槽位从来没有这个不对称：它们在快照边界**无条件**全量跟随。
///
/// 窗口取 8 个量子（1 024 帧）：44.1 kHz 上一条 5 ms 的 `1.0 → 0.25` 斜坡需要
/// ≈ 2 470 个样本才吸附，按旧采样率（48 kHz）算出的 `α` 需要 ≈ 2 694 个
/// ⇒ 这一窗**对两个 `α` 敏感**（实测两者约 0.2572 与 0.2605，见下面的字面读数）。
#[test]
fn a_master_slot_armed_after_a_sample_rate_change_uses_the_new_sample_rate() {
    let fixture = note_project(&[NOTE]);
    let mut shifted = fixture.project.clone();
    shifted.audio_config.sample_rate = yeban_model::SampleRate::Hz44100;

    // A：先在 48 kHz 上装配，换到 44.1 kHz 的快照**过一个量子之后**才武装主总线。
    let mut late = ParamRig::new(&fixture.project, 1);
    late.quantum();
    let snapshot = EngineSnapshot::from_project(&shifted, 2).expect("换采样率快照");
    late.slot.publish(snapshot);
    late.quantum(); // 快照边界：把 44.1 kHz 同步给平滑器（此刻主总线**还没有**武装）
    assert_eq!(
        late.runtime.armed_master_param_gain(),
        None,
        "本判据的前提：换采样率时主总线槽位还没有收到过任何目标"
    );
    late.set_param(fixture.master, MASTER_GAIN_SLOT, 0.25);

    // B：一开始就装配在 44.1 kHz —— 唯一的事实源；量子位置与 A 对齐。
    let mut fresh = ParamRig::new(&shifted, 1);
    fresh.quantum();
    let same = EngineSnapshot::from_project(&shifted, 2).expect("同采样率快照");
    fresh.slot.publish(same);
    fresh.quantum();
    fresh.set_param(fixture.master, MASTER_GAIN_SLOT, 0.25);

    const WINDOW: usize = 8;
    late.quanta(WINDOW);
    fresh.quanta(WINDOW);

    let late_gain = late
        .runtime
        .armed_master_param_gain()
        .expect("A 必须已武装");
    let fresh_gain = fresh
        .runtime
        .armed_master_param_gain()
        .expect("B 必须已武装");
    assert!(
        fresh_gain > 0.25 && fresh_gain < 1.0,
        "参照必须仍在平滑中（否则本判据对 α 不敏感，实得 {fresh_gain}）"
    );
    assert_eq!(
        late_gain, fresh_gain,
        "先换采样率再武装主总线，平滑器的 α 必须是新采样率的（逐位）"
    );
    assert_eq!(
        late.runtime.stats().param_master_gain_frames,
        fresh.runtime.stats().param_master_gain_frames,
        "两条路径被乘过的帧数必须相同"
    );
    let _ = late.queue.drain(64);
    let _ = fresh.queue.drain(64);
}

/// 判据 P14：**参数极值**不得让母线出现非有限样本 —— `render_block` 的母线有限值
/// 守卫必须真的**接在限制器输入侧**。
///
/// 量什么：整段渲染里 `NaN` 与非有限样本的**条数**（单位：条），以及
/// `limiter_max_reduction`（无量纲，1.0 = 全压）。
///
/// 构造：逐轨槽与主总线槽各推一个 `1.0e20` 的乘子。两者都是**合法**值（有限、非负），
/// 但它们在母线上**相乘**（先逐轨 `apply`、后 `apply_master`）⇒ 母线样本
/// ≈ `样本 × 1e20 × 1e20 = 样本 × 1e40`，超过 `f32::MAX`（≈ 3.4e38）⇒ `±∞`。
/// 守卫把 `±∞` 收进 `±MAX_LINEAR_MAGNITUDE`（16.0）⇒ 限制器看到的是有限值。
/// 少了这一步，限制器的窗口峰值 `W = ∞` ⇒ 目标增益 `T / W = 0` ⇒ `∞ · 0 = NaN`
/// **静默**污染整条母线（`4309018` 修的正是这个缺陷）。
///
/// 为什么单独立一条：`src/rt.rs` 的三条守卫判据都**直接调用**
/// `saturate_bus_to_finite`（函数本体），**没有**一条判据走 `render_block` 的
/// 调用点 ⇒ 删掉那一行调用，全量 24 个目标全绿（本票注入实测，已还原）。
#[test]
fn an_extreme_gain_must_not_push_the_bus_out_of_the_finite_range() {
    const HUGE: f32 = 1.0e20;
    let fixture = note_project(&[NOTE]);
    let mut rig = ParamRig::new(&fixture.project, 1);
    assert_eq!(rig.set_param(fixture.track, TRACK_GAIN_SLOT, HUGE), 1);
    assert_eq!(rig.set_param(fixture.master, MASTER_GAIN_SLOT, HUGE), 1);
    rig.quanta(QUANTA);

    let stats = rig.runtime.stats();
    let track_gain = rig
        .runtime
        .armed_param_gain(&fixture.track)
        .expect("逐轨槽位必须已建立");
    let master_gain = rig
        .runtime
        .armed_master_param_gain()
        .expect("主总线槽位必须已武装");
    // 覆盖度：两个乘子必须**真的**吸附到极端值（否则"无非有限样本"是空转）。
    assert!(
        track_gain > HUGE / 2.0,
        "覆盖度：逐轨乘子必须到达极端值，实测 {track_gain:e}"
    );
    assert!(
        master_gain > HUGE / 2.0,
        "覆盖度：主总线乘子必须到达极端值，实测 {master_gain:e}"
    );
    // 覆盖度：`1e20 × 1e20 = 1e40 > f32::MAX(≈ 3.4e38)` ⇒ 母线上必然出现 `±∞`，
    // 而限制器随后必须**强压** —— 这是"守卫真的把 `∞` 收进了有限域"的可见后果。
    assert!(
        stats.limiter_max_reduction > 0.5,
        "覆盖度：饱和后的 ±{MAX_LINEAR_MAGNITUDE} 必须让母线限制器强压，\
         实测 limiter_max_reduction={}",
        stats.limiter_max_reduction
    );
    let non_finite = rig
        .left
        .iter()
        .chain(rig.right.iter())
        .filter(|sample| !sample.is_finite())
        .count();
    let nan = rig
        .left
        .iter()
        .chain(rig.right.iter())
        .filter(|sample| sample.is_nan())
        .count();
    assert_eq!(nan, 0, "母线不得出现 NaN（{nan} 条）");
    assert_eq!(non_finite, 0, "母线不得出现非有限样本（{non_finite} 条）");
    assert!(
        rig.left.iter().any(|sample| *sample != 0.0),
        "极端增益下仍然必须出声（不是静音欺骗）"
    );
    println!(
        "[engine-param/P14] 极端增益 {HUGE:e} × {HUGE:e}: 非有限={non_finite} NaN={nan} \
         最大压限={:.4} 当前压限={:.4}",
        stats.limiter_max_reduction, stats.limiter_current_reduction
    );
}

/// `value` 是否是 2 的幂（正有限值，且尾数位全零）。
///
/// ⛔ 刻意**不**用 `log2`（超越函数类，[ADR-0001 D32]）：这是一次纯整数判定。
fn is_power_of_two_gain(value: f32) -> bool {
    value > 0.0 && value.is_finite() && (value.to_bits() & 0x007f_ffff) == 0
}

/// 判据 P15：**两个主总线乘子的顺序是契约** —— 静态推子（`scale_bus`，步骤 3a''）
/// 在**前**、自动化乘子（`apply_master`，步骤 3a'''）在**后**。
///
/// ## 被测量
///
/// `note_project` 的整段左右声道样本（**位模式**）。三条臂：
///
/// | 臂 | 主总线推子 | 主总线自动化 | 输出 |
/// | :--- | :--- | :--- | :--- |
/// | `plain` | 0 dB | 无 | `x` |
/// | `fader` | −6 dB（`g`） | 无 | `fl(x · g)` |
/// | 被测 | −6 dB（`g`） | 目标 `m`（已吸附） | 契约：`fl(fl(x · g) · m)` |
///
/// `g = 10^(−6/20) ≈ 0.5011872` 与 `m = 0.7` 都**不是** 2 的幂，因此两种顺序
/// （`fl(fl(x·g)·m)` 与 `fl(fl(x·m)·g)`）在这一窗口里必然给出不同的位模式
/// （覆盖度见证 ④）。
///
/// ## 为什么单独立一条
///
/// 既有的主总线判据（P9 / P10 / P12 与 `tests/idempotency_and_channel_consistency.rs`
/// 的 ⑤-2）全部把自动化目标取成 **2 的幂**（`0.25` / `0.5` / `0.125`），而乘 2 的幂
/// 是**精确**的 ⇒ `fl(fl(x·g)·2⁻ⁿ) == fl(fl(x·2⁻ⁿ)·g)` 逐位成立，顺序写反没有任何
/// 判据变红。本票（engine-28）注入实测（把 `scale_bus` 挪到 `apply_master` 之后）：
/// 全量 24 个目标全绿。`src/rt.rs` 步骤 3a''' 的注释把顺序写成契约（"浮点乘法不满足
/// 结合律 ⇒ 顺序不是无关紧要的细节"）—— 本判据就是那条注释的机械形式。
///
/// ⚠ 覆盖边界：三条臂都必须在母线限制器的透明区（`limiter_gain_reductions == 0`），
/// 否则非线性会让线性等式失效 —— 这一点由覆盖度见证 ② 显式断言。
#[test]
fn the_static_master_fader_is_applied_before_the_automation_multiplier() {
    /// 主总线推子（dB）：−6 dB ⇒ 线性增益 ≈ 0.501 187 2（不是 2 的幂）。
    const FADER_DB: f32 = -6.0;
    /// 自动化乘子的目标：0.7（不是 2 的幂）。
    const TARGET: f32 = 0.7;

    let fixture = note_project(&[NOTE]);
    let master = fixture.master;
    let plain_project = fixture.project.clone();
    let mut fader_project = fixture.project.clone();
    fader_project
        .tracks
        .get_mut(&master)
        .expect("夹具里必须有主总线")
        .volume_db = FADER_DB;

    let gain = EngineSnapshot::from_project(&fader_project, 1)
        .expect("推子快照")
        .master_gain();
    // ---- 覆盖度见证 ①：`g` 与 `m` 都必须不是 2 的幂 ----
    assert!(
        gain > 0.0 && gain < 1.0,
        "−6 dB 的推子必须落在 (0, 1)（实得 {gain}）"
    );
    assert!(
        !is_power_of_two_gain(gain) && !is_power_of_two_gain(TARGET),
        "覆盖度：`g`={gain} 与 `m`={TARGET} 都必须不是 2 的幂，否则两种顺序逐位同解（判据空转）"
    );

    let plain = render(&plain_project, QUANTA);
    let fader = render(&fader_project, QUANTA);
    let mut rig = ParamRig::new(&fader_project, 1);
    assert_eq!(rig.set_param(master, MASTER_GAIN_SLOT, TARGET), 1);
    rig.quanta(QUANTA);

    // ---- 覆盖度见证 ②：三条臂都必须在母线限制器的透明区 ----
    for (label, arm) in [("无推子", &plain), ("有推子", &fader)] {
        assert_eq!(
            arm.stats.limiter_gain_reductions, 0,
            "{label} 臂必须落在限制器的透明区（否则下面的线性等式不成立）"
        );
        assert!(arm.peak() > 0.0, "{label} 臂必须真的出声");
    }
    assert_eq!(
        rig.runtime.stats().limiter_gain_reductions,
        0,
        "被测臂必须落在限制器的透明区"
    );
    assert_eq!(
        rig.runtime.armed_master_param_gain(),
        Some(TARGET),
        "自动化乘子必须已吸附到目标（否则下面的逐位等号不能写成常量 TARGET）"
    );

    let settled = 3_000..QUANTA * 128;
    let mut checked = 0usize;
    let mut order_sensitive = 0usize;
    for index in settled {
        // ---- 覆盖度见证 ③：静态推子是一次**逐位精确**的标量乘 ----
        assert_eq!(
            fader.left[index].to_bits(),
            (plain.left[index] * gain).to_bits(),
            "第 {index} 帧：静态推子必须恰好是 `x · g`（左声道）"
        );
        assert_eq!(
            fader.right[index].to_bits(),
            (plain.right[index] * gain).to_bits(),
            "第 {index} 帧：静态推子必须恰好是 `x · g`（右声道）"
        );
        // ---- 主判据：契约顺序 ⇒ fl(fl(x · g) · m)，逐位 ----
        assert_eq!(
            rig.left[index].to_bits(),
            (fader.left[index] * TARGET).to_bits(),
            "第 {index} 帧：自动化乘子必须在静态推子**之后**（左声道）"
        );
        assert_eq!(
            rig.right[index].to_bits(),
            (fader.right[index] * TARGET).to_bits(),
            "第 {index} 帧：自动化乘子必须在静态推子**之后**（右声道）"
        );
        // ---- 覆盖度见证 ④：写反顺序在这一帧上会给出另一个位模式 ----
        if ((plain.left[index] * TARGET) * gain).to_bits() != rig.left[index].to_bits() {
            order_sensitive += 1;
        }
        checked += 1;
    }
    assert!(
        checked > 40_000,
        "窗口必须覆盖绝大多数样本（实得 {checked}）"
    );
    assert!(
        order_sensitive > 0,
        "两种顺序必须在某一帧上给出不同的位模式，否则本判据测不到顺序"
    );
    println!(
        "[engine-param/P15] 主总线顺序：静态推子 g={gain} 先、自动化 m={TARGET} 后；\
         逐位核对 {checked} 帧，其中 {order_sensitive} 帧对两种顺序敏感"
    );
}
