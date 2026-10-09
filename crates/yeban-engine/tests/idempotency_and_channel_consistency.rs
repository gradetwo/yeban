//! `line/engine-21` 的端到端判据：类别⑤ **幂等性** 与 类别⑥ **多声道一致性**。
//! [ARCH-DET-001, ARCH-RT-002, ARCH-PDC-002, ARCH-DSP-001]
//!
//! 全部判据在 **`--no-default-features`（不编译 cpal）** 下运行，与 `mix_render.rs` /
//! `param_automation.rs` 同一条主路径。
//!
//! ## 1. 本票量的是哪两件事
//!
//! | 类别 | 一句话定义 | 本文件的判据 |
//! | :--- | :--- | :--- |
//! | ⑤ 幂等性 | 在**同一个对象**上把**同一个值**再施加一次，输出/读数与只施加一次**逐位**相同 | 幂等 ⑤-1 ~ ⑤-5 |
//! | ⑥ 多声道一致性 | 同一个信号进 N 路（或"只填一路"）⇒ 各路之间的关系是**声明的那一种**，且没有未定义行为 | 声道 ⑥-6 ~ ⑥-10 |
//!
//! "同一个对象上同一个值"在本 crate 里的**全部**形态（机械枚举见模块 §3）：
//! 重复的同值 `SetParam`（逐轨槽 / 主总线槽；**同批**与**隔量子**两种形态）、
//! 重复的同值走带命令、重复的同采样率 `set_sample_rate`、以及"内容逐位相同、
//! 只有修订号不同"的快照**重新武装**（PDC 计划相同 ⇒ 不得重绑、不得清延迟线）。
//!
//! ## 2. 判据表（每条都写清"怎么变红"）
//!
//! | 编号 | 判据 | 怎么变红（注入） | 实测 |
//! | :--- | :--- | :--- | :--- |
//! | ⑤-1 | 逐轨增益 `SetParam` 同值重复施加（同批 + 隔 20 量子）== 一次（整段左右声道逐位） | `accept` 里在 `set_target` 之前 `snap_to(1.0)`（重启斜坡） | 注入后**隔量子**形态变红（同批形态**看不出来**，见下） |
//! | ⑤-2 | 主总线增益 `SetParam` 同值重复施加（同批 + 隔量子）== 一次 | 同上（`MASTER_GAIN_SLOT` 分支） | 未单独注入（与 ⑤-1 同一条代码路径） |
//! | ⑤-3 | 同一条走带命令重复施加 == 一次（`Play`/`Stop`/两种 `SeekTicks` 的音频 + 位置读数；`Play`/`Stop` 另有隔量子形态） | `Transport::apply` 让重复命令再动一次状态（例如重复 `SeekTicks` 再 `synth.seek` 一次并清相位） | 见 §4 的口径说明 |
//! | ⑤-4 | `ParamTable::set_sample_rate(同值)` 两次 == 一次（逐轨 + 主总线逐样本轨迹） | 让同值 `set_sample_rate` 也去动 `value`/`target`（例如按新旧采样率折算在途斜坡） | 未注入（早返回是唯一入口，见代码） |
//! | ⑤-5 | 同一份 PDC 计划重新武装不重绑、不清延迟线（等价快照周期重发 == 单次发布） | `CompensationBank::rearm` 改成"每次先 `line.reset()`" | 注入后变红（重放窗口 ⇒ 音频不同） |
//! | ⑥-6 | 居中的单声道轨在立体声母线上左右**逐位相同**（限制器未介入） | 左右用两条独立的增益轨迹 / 声相表只写一路 | 见 ⑥-9 的注入 |
//! | ⑥-7 | 母线限制器**介入**时，居中的单声道母线仍然左右逐位相同（立体声联动） | `process_stereo` 改成两路各自检波 | 联动本身由 `tests/limiter_contract.rs` 钉住 |
//! | ⑥-8 | 全左声相 + 限制器介入：右声道**整段逐位为 `+0.0`**（"只填一路"没有串扰） | 立体声器件把左路的信号混进右路（串扰）/ 右路乘子不为 `+0.0` | 注入后变红 |
//! | ⑥-9 | 左右两路来自**同一条标量声相曲线**：全右的左声道 == `+0.0 + 全左的左声道 × cos(π/2)`，全右的右声道 == 全左的左声道（均逐位） | 两条声道各算一份曲线（哪怕差一个 ulp） | 注入后变红 |
//! | ⑥-10 | 交错输出的通道映射：`ch0 = 左`、`ch1..chN-1 = 右`（N = 1/2/4/6，逐量子逐路逐位） | `AudioBlock::get` 的通道判定取反 | 注入后变红（参照取自 `AudioBlock::left()/right()`，**不是**再跑一遍 `process_quantum`） |
//!
//! ⚠ ⑤-1 的两种形态**都必须有**：两条同值事件落在同一个事件批次里时，任何
//! "先在 `accept` 里吸附、再设目标"的实现都会被顺序抹平（实测注入：同批形态全绿、
//! 隔量子形态变红）。"UI 反复重发同一个值"才是真实形态，判据必须覆盖它。
//!
//! ## 3. 机械枚举（量法 + 读数）
//!
//! 引擎把事件/参数/快照施加到音频线程的**入口**只有两处集中点：每个量子的
//! `events.drain_with(...)`（`src/rt.rs` 现位于第 1703 行）与每个**修订**一次的
//! 快照边界分支（同文件现位于第 1727 行起）。边界内的**可变更**调用点共 20 处
//! （`params.accept` / `params.set_sample_rate` / `bank.set_quanta_per_second` /
//! `synth.begin_snapshot` / `synth.align_cursors` / `synth.seek` / `transport.arm` /
//! `transport.apply` / `metronome.resync`（两处）/ `metronome.silence` /
//! `pdc.rearm` / `strip.set_params` / `strip.set_sample_rate` /
//! `reverb_pool[..].set_params`（两处）/ `reverb_pool[..].set_sample_rate` /
//! `conv_pool[..].set_params`（两处）/ `conv_pool[..].set_impulse_response`）。
//! ⛔ `NoteOn` / `NoteOff` **不是**入口：实时侧对它们只计数、不接渲染
//! （登记在 `docs/ledger/engine-rt-notes.md`）⇒ 重复施加"同一个音符事件"是
//! **平凡幂等**（两次与一次都不出声），本文件不为它写判据。
//!
//! 加词边界的计数对照（量法：`grep -rn <记号> crates/yeban-engine/src` 与
//! `grep -rnw <记号> …` 各自的**命中行数**）：
//!
//! | 记号 | 不加边界 | `-w` 加边界 | 被筛掉的假阳性是什么（实测分类） |
//! | :--- | ---: | ---: | :--- |
//! | `arm` | 335 | 7 | `armed`（54 行）、`armed_nodes`（21）、`armed_insert_slots` / `armed_conv_slots`（各 13）… |
//! | `reset` | 48 | 34 | `reset_*` 与 `*_reset_*` 族：`reset_current_thread`、`reset_silence`、`reset_keeps_the_armed_ballistics` … |
//! | `buffer` | 62 | 23 | `buffer_frames`（11）、`buffer_size`（10）、`buffer_frames_reported`（4）、`buffer_range_unknown`（3）、`buffer_{min,max}_frames`（各 3）…（合计 39） |
//! | `apply` | 65 | 52 | `apply_master`（11，**逐样本路径上的第二个真实入口**）、`apply_kit_knob`（2） |
//! | `accept` | 38 | 23 | `fixed_block_accepted`（7）、`accepted`（4）、`*_accepts_*` 判据名（3）… |
//!
//! ⚠ 本仓库流传的两个假阳性例子（`preset` ⊂ `reset`、`PcmBuffer` ⊂ `buffer`）
//! 在**本 crate 的 `src/` 里都不存在**：`grep -rn preset src/` = **0** 行、
//! `grep -rn PcmBuffer src/` = **0** 行（两个记号加不加边界都是 0）。因此本票的
//! 假阳性全部来自上表右侧那一列更长标识符，不是仓库别处的例子。
//!
//! ## 4. 本票发现（**不改**，只报告）
//!
//! **发现 1（多声道）：单声道设备上的声道映射会把右声相的内容丢掉。**
//! `channels == 1` 时 `process_quantum` 只写第 0 路（= 左），而 `device::negotiate`
//! 的第 2 步只看采样率、**不问通道数** ⇒ 只提供单声道的 f32 设备会被协商成功，
//! 于是全右声相的轨在单声道设备上**不可闻**。
//! 量法：对同一个工程（力度 127、pan = +1.0、音量 +6 dB，120 个量子）分别调
//! `process_quantum(out, 2)` 与 `process_quantum(out, 1)`，然后
//! ①逐帧逐位比较单声道输出与立体声**左**声道；②取各自峰值。实测读数：
//!
//! ```text
//! hard-right +6 dB: stereo peak_l=3.934455e-8 peak_r=9.000982e-1
//!                   mono  peak =3.934455e-8  mono==left bits=true
//! ```
//!
//! ⇒ 单声道输出是立体声左声道的**逐位副本**，而右声道（9.000982e-1）在单声道
//! 设备上无处可去（相差 7 个数量级）。
//! 修法（例如单声道求和 `(L+R)·0.5`）会**改变既有渲染输出** ⇒ 按本票纪律停在这里
//! 报告，不在判据里把它写成期望值（⑥-10 只钉"通道映射"，不钉"单声道应当怎么混"）。
//!
//! **发现 2（幂等性，口径澄清而非缺陷）：`SeekTicks(t)` 的"隔量子重发"不是幂等性问题。**
//! 两次施加之间时钟已经推进（`position_ticks` 越过 `t`、相位余数已经攒起来），
//! 因此第二条是一条**真正的回跳**：它把 `position_frames` 拉回 `t` 的网格帧、
//! 清零相位余数，并让 `synth.seek` 释放全部声部。实测（同一个工程，跑 4 个量子，
//! 第 4 个量子前发一条 `SeekTicks(当前 tick)`）：
//!
//! ```text
//! no-seek:          after 4 quanta tick=20 frames=512
//! seek-to-current:  after 4 quanta tick=20 frames=503
//! ```
//!
//! 差额 9 帧 = `frames_for(15) = 375` 与 seek 前的 384 帧之差（375 + 128 = 503；
//! 不 seek 的 512 = 4 × 128）。这与"同一个值施加在同一个状态上"不是一回事 ⇒
//! ⑤-3 只对 `SeekTicks` 测**同批**形态，并把这个界写在这里。
//!
//! 判据的实测读数与探针命令见交付报告；探针一律住在 `/tmp`，不在本目录留档。

mod support;

use support::{MixSpec, NoteSpec, note_project, pdc_rebind_fixture, render, tuned_project};
use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_engine::meter::meter_channel;
use yeban_engine::mixer::{LIMITER_CEILING, PanLaw, pan_gains};
use yeban_engine::param::{MASTER_GAIN_SLOT, ParamTable, TRACK_GAIN_SLOT};
use yeban_engine::ring::{EngineEvent, ParamAddress, TransportCommand, event_channel};
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};
use yeban_model::{EntityId, YebanProjectV1};

/// 事件可发的装配（`support::Runtime` 刻意丢掉生产端）。
///
/// 与 `tests/param_automation.rs` 的 `ParamRig` 同族，但这里要的**不止**参数事件
/// （还有走带命令），所以两条声道都留档。
struct EventRig {
    slot: std::sync::Arc<SnapshotSlot>,
    queue: yeban_engine::snapshot::RetireQueue,
    sender: yeban_engine::ring::EventSender,
    runtime: EngineRuntime,
    output: Vec<f32>,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl EventRig {
    fn new(project: &YebanProjectV1, revision: u64, channels: u16) -> Self {
        let snapshot = EngineSnapshot::from_project(project, revision).expect("夹具工程必须能编译");
        let slot = SnapshotSlot::new(snapshot);
        let (retire, queue) = retire_channel(64);
        let (sender, receiver) = event_channel(256);
        let (publisher, _collector) = meter_channel(65_536);
        let runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
        Self {
            slot,
            queue,
            sender,
            runtime,
            output: vec![0.0f32; DEFAULT_BLOCK_FRAMES * usize::from(channels)],
            left: Vec::new(),
            right: Vec::new(),
        }
    }

    /// 推一个量子：第 0 路进 `left`，第 1 路（没有则为第 0 路）进 `right`。
    fn quantum(&mut self, channels: u16) {
        let stride = usize::from(channels);
        self.output.fill(0.0);
        self.runtime.process_quantum(&mut self.output, channels);
        let right_channel = if stride > 1 { 1 } else { 0 };
        for frame in 0..DEFAULT_BLOCK_FRAMES {
            self.left.push(self.output[frame * stride]);
            self.right.push(self.output[frame * stride + right_channel]);
        }
    }

    /// 跑 `quanta` 个量子，`at` 在**每个量子渲染之前**调用。
    fn run(&mut self, quanta: usize, channels: u16, mut at: impl FnMut(usize, &mut Self)) {
        for quantum in 0..quanta {
            at(quantum, self);
            self.quantum(channels);
            if quantum % 8 == 0 {
                let _ = self.queue.drain(64);
                self.slot.prune();
            }
        }
    }
}

/// 左右两声道是否**逐位**相同。
fn channels_are_bit_identical(left: &[f32], right: &[f32]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right.iter())
            .all(|(l, r)| l.to_bits() == r.to_bits())
}

/// 两段渲染的左右声道是否逐位相同。
fn renders_are_bit_identical(a: &EventRig, b: &EventRig) -> bool {
    channels_are_bit_identical(&a.left, &b.left) && channels_are_bit_identical(&a.right, &b.right)
}

/// 一个全右声相（+6 dB，与模块 §4 发现 1 同一档）的工程在**单声道**输出上的峰值。
fn mono_peak_of_hard_right() -> f32 {
    let notes = [NoteSpec::at(0, 960, 69, 127)];
    let project = tuned_project(
        &notes,
        MixSpec {
            volume_db: 6.0,
            ..MixSpec::pan(1.0)
        },
    )
    .project;
    let mut rig = EventRig::new(&project, 1, 1);
    rig.run(120, 1, |_, _| {});
    rig.left.iter().fold(0.0f32, |peak, s| peak.max(s.abs()))
}

// ---------------------------------------------------------------------------
// 类别⑤ 幂等性
// ---------------------------------------------------------------------------

/// ⑤-1：逐轨增益的 `SetParam` **同值重复施加** == 只施加一次（整段逐位）。
///
/// 两种重复形态都测：
/// 1. **同一个事件批次里两条**（"重复施加"的最严格形态：两次之间没有状态推进）；
/// 2. **相隔 20 个量子再发一次**（控制面的真实形态：UI 反复重发同一个值）。
///
/// 发射点刻意取第 30 个量子：那时 `1.0 → 0.25` 的斜坡**正在走**，因此"重复施加"
/// 若在 `accept` 里做任何"重启斜坡 / 重置平滑器 / 吸附到目标"的动作，
/// 第 2 种形态立刻可见（第 1 种形态里两条事件在同一个量子边界被一起消费，
/// 任何"先吸附后设目标"的实现都看不出来 —— 这是本判据必须有两种形态的理由）。
#[test]
fn repeated_identical_track_gain_events_are_bit_identical_to_one() {
    const QUANTA: usize = 80;
    const AT: usize = 30;
    const LATER: usize = 20;
    let fixture = note_project(&[NoteSpec::quarter(60)]);
    let (track, project) = (fixture.track, fixture.project.clone());
    let event = EngineEvent::SetParam {
        target: ParamAddress::new(track, TRACK_GAIN_SLOT),
        value: 0.25,
    };

    let arm = |second_at: Option<usize>| {
        let mut rig = EventRig::new(&project, 1, 2);
        rig.run(QUANTA, 2, |quantum, rig| {
            if quantum == AT {
                rig.sender.publish(&[event]);
            }
            if second_at == Some(quantum) {
                rig.sender.publish(&[event]);
            }
        });
        rig
    };
    let once = arm(None);
    let twice_same_batch = arm(Some(AT));
    let twice_later = arm(Some(AT + LATER));
    let mut unarmed = EventRig::new(&project, 1, 2);
    unarmed.run(QUANTA, 2, |_, _| {});

    // 见证：乘子真的走到了音频（否则下面几条等号可能是"两边都没接"）。
    assert!(
        !channels_are_bit_identical(&once.left, &unarmed.left),
        "参数事件必须真的改变音频（夹具必须出声且乘子真的被施加）"
    );
    // 帧数 = 事件之后剩下的全部帧（事件在第 `AT` 个量子之前生效）。
    let expected_frames = ((QUANTA - AT) * DEFAULT_BLOCK_FRAMES) as u64;
    for rig in [&once, &twice_same_batch, &twice_later] {
        assert_eq!(rig.runtime.stats().param_gain_frames, expected_frames);
    }
    assert!(
        renders_are_bit_identical(&once, &twice_same_batch),
        "同批两条同值 SetParam 必须与一次逐位相同（逐轨槽）"
    );
    assert!(
        renders_are_bit_identical(&once, &twice_later),
        "相隔 {LATER} 个量子重发同值 SetParam 必须与一次逐位相同（逐轨槽）"
    );
    println!(
        "[engine-idem/5-1] 逐轨同值重复施加（同批 / 隔 {LATER} 量子）：帧数={} 逐位相同=true",
        once.left.len()
    );
}

/// ⑤-2：**主总线**增益槽的同值 `SetParam` 重复施加 == 一次（同批 + 隔量子两种形态）。
#[test]
fn repeated_identical_master_gain_events_are_bit_identical_to_one() {
    const QUANTA: usize = 80;
    const AT: usize = 30;
    const LATER: usize = 20;
    let fixture = note_project(&[NoteSpec::quarter(60)]);
    let (master, project) = (fixture.master, fixture.project.clone());
    let event = EngineEvent::SetParam {
        target: ParamAddress::new(master, MASTER_GAIN_SLOT),
        value: 0.5,
    };

    let arm = |second_at: Option<usize>| {
        let mut rig = EventRig::new(&project, 1, 2);
        rig.run(QUANTA, 2, |quantum, rig| {
            if quantum == AT {
                rig.sender.publish(&[event]);
            }
            if second_at == Some(quantum) {
                rig.sender.publish(&[event]);
            }
        });
        rig
    };
    let once = arm(None);
    let twice_same_batch = arm(Some(AT));
    let twice_later = arm(Some(AT + LATER));
    let mut unarmed = EventRig::new(&project, 1, 2);
    unarmed.run(QUANTA, 2, |_, _| {});

    assert!(
        !channels_are_bit_identical(&once.left, &unarmed.left),
        "主总线乘子必须真的改变音频"
    );
    let expected_frames = ((QUANTA - AT) * DEFAULT_BLOCK_FRAMES) as u64;
    for rig in [&once, &twice_same_batch, &twice_later] {
        assert_eq!(
            rig.runtime.stats().param_master_gain_frames,
            expected_frames
        );
    }
    assert!(
        renders_are_bit_identical(&once, &twice_same_batch),
        "同批两条同值 SetParam 必须与一次逐位相同（主总线槽）"
    );
    assert!(
        renders_are_bit_identical(&once, &twice_later),
        "相隔 {LATER} 个量子重发同值 SetParam 必须与一次逐位相同（主总线槽）"
    );
    println!(
        "[engine-idem/5-2] 主总线同值重复施加（同批 / 隔 {LATER} 量子）：帧数={} 逐位相同=true",
        once.left.len()
    );
}

/// ⑤-3：同一条**走带命令**重复施加 == 一次（音频 + 位置读数）。
///
/// 命令先在第 3 个量子之后发出：那时 `advance_frames` 已经留下**非零相位余数**
/// （128 帧 @48 kHz/120 BPM = 5.12 tick）。`SeekTicks` 是唯一会清余数的命令，
/// 因此它是"重复施加会不会再动一次状态"的最强被测对象。
///
/// 两种形态：**同一批次两条**（状态不变 ⇒ 严格幂等），以及 `Play`/`Stop` 的
/// **隔一个量子再发一条**（这两个命令在状态没变时返回 `None`，因此也应该逐位相同）。
///
/// ⚠ `SeekTicks(t)` 的**隔量子重发不是幂等性问题**：两次之间时钟已经推进，
/// `position_ticks` 已经越过 `t` ⇒ 第二条是一条**真正的回跳**（状态变了），
/// 不在"同一个值施加在同一个状态上"的定义里。因此只对它测同批形态。
#[test]
fn repeated_identical_transport_commands_are_bit_identical_to_one() {
    const QUANTA: usize = 60;
    const AT: usize = 3;
    let fixture = note_project(&[NoteSpec::quarter(60), NoteSpec::at(1_920, 960, 64, 90)]);
    let project = fixture.project.clone();

    // `second_at`: `Some(quantum)` = 在那个量子前再发同一条命令（`AT` 即同批）。
    let run = |command: TransportCommand, second_at: Option<usize>| {
        let mut rig = EventRig::new(&project, 1, 2);
        let event = EngineEvent::Transport { command };
        let mut sent = false;
        rig.run(AT + QUANTA, 2, |quantum, rig| {
            if quantum == AT {
                rig.sender.publish(&[event]);
                sent = true;
            }
            if second_at == Some(quantum) {
                rig.sender.publish(&[event]);
            }
        });
        assert!(sent, "命令必须真的发出去");
        (
            rig.left.clone(),
            rig.right.clone(),
            rig.runtime.position_ticks(),
            rig.runtime.position_samples(),
        )
    };

    let cases = [
        ("Play", TransportCommand::Play, true),
        ("Stop", TransportCommand::Stop, true),
        ("SeekTicks(0)", TransportCommand::SeekTicks(0), false),
        ("SeekTicks(1920)", TransportCommand::SeekTicks(1_920), false),
    ];
    let mut cross_quantum_checked = 0usize;
    for (label, command, cross_quantum) in cases {
        let once = run(command, None);
        let same_batch = run(command, Some(AT));
        assert_eq!(once.2, same_batch.2, "{label}: 同批重复不得改变位置 tick");
        assert_eq!(once.3, same_batch.3, "{label}: 同批重复不得改变位置帧");
        assert!(
            channels_are_bit_identical(&once.0, &same_batch.0)
                && channels_are_bit_identical(&once.1, &same_batch.1),
            "{label}: 同批重复同值命令必须与一次逐位相同"
        );
        if cross_quantum {
            let later = run(command, Some(AT + 1));
            assert_eq!(once.2, later.2, "{label}: 隔量子重复不得改变位置 tick");
            assert_eq!(once.3, later.3, "{label}: 隔量子重复不得改变位置帧");
            assert!(
                channels_are_bit_identical(&once.0, &later.0)
                    && channels_are_bit_identical(&once.1, &later.1),
                "{label}: 隔量子重复同值命令必须与一次逐位相同"
            );
            cross_quantum_checked += 1;
        }
    }
    assert_eq!(cross_quantum_checked, 2, "Play/Stop 必须各测过隔量子形态");

    // 见证：命令真的改变了状态（否则上面的等号可能是"两边都没生效"）。
    let play = run(TransportCommand::Play, None);
    let stop = run(TransportCommand::Stop, None);
    assert!(
        stop.2 < play.2,
        "Stop 必须让时钟冻结在更早的位置（play tick={} stop tick={}）",
        play.2,
        stop.2
    );
    let seek = run(TransportCommand::SeekTicks(1_920), None);
    // 1920 tick @120 BPM/48 kHz = 48000 帧（25 帧/tick），之后再跑 `QUANTA` 个量子。
    assert_eq!(
        seek.3,
        48_000 + (QUANTA as u64) * DEFAULT_BLOCK_FRAMES as u64,
        "SeekTicks(1920) 必须把播放头放到 48000 帧（实得 {} 帧）",
        seek.3
    );
    assert!(
        seek.2 > play.2,
        "seek 之后的位置必须领先于不 seek 的那一臂（{} vs {}）",
        seek.2,
        play.2
    );
    println!("[engine-idem/5-3] 四种同值命令：同批重复全部逐位相同；Play/Stop 另有隔量子形态");
}

/// ⑤-4：`ParamTable::set_sample_rate` 用**同一个采样率**连调两次 == 一次。
///
/// 逐样本比较两条声道（逐轨槽 4096 帧 + 主总线槽 4096 帧）。采样率取 96 kHz、
/// 构造在 48 kHz ⇒ `α` 真的被重算过一次，因此"第二次调用是不是又把 `α`/状态
/// 动了一遍"是可观察的。
#[test]
fn reapplying_the_same_sample_rate_to_the_param_table_is_bit_identical() {
    const SR_BEFORE: f32 = 48_000.0;
    const SR_AFTER: f32 = 96_000.0;
    const FRAMES: usize = 4_096;
    let track = EntityId::new();
    let master = EntityId::new();

    let mut once = ParamTable::new(SR_BEFORE);
    once.set_sample_rate(SR_AFTER);
    let mut twice = ParamTable::new(SR_BEFORE);
    twice.set_sample_rate(SR_AFTER);
    twice.set_sample_rate(SR_AFTER);

    for table in [&mut once, &mut twice] {
        table.accept(ParamAddress::new(track, TRACK_GAIN_SLOT), 0.3, master);
        table.accept(ParamAddress::new(master, MASTER_GAIN_SLOT), 0.7, master);
    }
    let mut once_track = [1.0f32; FRAMES];
    let mut twice_track = [1.0f32; FRAMES];
    let (mut once_l, mut once_r) = ([1.0f32; FRAMES], [-1.0f32; FRAMES]);
    let (mut twice_l, mut twice_r) = ([1.0f32; FRAMES], [-1.0f32; FRAMES]);
    assert_eq!(once.apply(track, &mut once_track), FRAMES as u64);
    assert_eq!(twice.apply(track, &mut twice_track), FRAMES as u64);
    assert_eq!(once.apply_master(&mut once_l, &mut once_r), FRAMES as u64);
    assert_eq!(
        twice.apply_master(&mut twice_l, &mut twice_r),
        FRAMES as u64
    );

    // 见证：两条轨迹真的被推进过（第一个样本不许已经是目标值）。
    assert!(
        once_track[0] < 1.0 && once_track[0] > 0.3,
        "96 kHz 的第一窗必须仍在平滑中（实得 {}）",
        once_track[0]
    );
    assert_eq!(
        once_track.map(f32::to_bits),
        twice_track.map(f32::to_bits),
        "同值 set_sample_rate 两次的逐轨轨迹必须与一次逐位相同"
    );
    assert_eq!(once_l.map(f32::to_bits), twice_l.map(f32::to_bits));
    assert_eq!(once_r.map(f32::to_bits), twice_r.map(f32::to_bits));
    println!("[engine-idem/5-4] 同值 set_sample_rate 两次：逐轨与主总线轨迹逐位相同");
}

/// ⑤-5：**同一份 PDC 计划**重新武装不得重绑、不得清延迟线。
///
/// 夹具 `pdc_rebind_fixture` 的满计划里两条短支路的 `D = 400` 帧
/// （常量 `support::PDC_REBIND_LATENCY`），并且唯一出声的支路**自己就占一条延迟线**
/// ⇒ 清线会直接吞掉/重放 400 帧音频，肉眼可见。
///
/// 量法：周期发布**内容逐位相同、只有修订号不同**的快照（每 16 个量子一份，
/// 共 7 份），与只发布一次的同长度渲染比较左右声道的位模式。
#[test]
fn rearming_the_same_pdc_plan_neither_rebinds_nor_clears_a_delay_line() {
    const QUANTA: usize = 140;
    const EVERY: usize = 16;
    let fixture = pdc_rebind_fixture();
    let project = fixture.project.clone();

    let mut once = EventRig::new(&project, 1, 2);
    once.run(QUANTA, 2, |_, _| {});
    let mut republished = EventRig::new(&project, 1, 2);
    republished.run(QUANTA, 2, |quantum, rig| {
        if quantum > 0 && quantum % EVERY == 0 {
            let revision = 1 + (quantum / EVERY) as u64;
            rig.slot
                .publish(EngineSnapshot::from_project(&project, revision).expect("等价快照"));
        }
    });

    // 见证 ①：这条支路真的出声（否则"逐位相同"是静音对静音）。
    assert!(
        once.left.iter().any(|sample| *sample != 0.0),
        "夹具必须真的出声"
    );
    // 见证 ②：重绑分支真的被走到过 —— 首次武装把 4 个空槽绑到 4 个节点键上。
    assert_eq!(
        once.runtime.pdc_rebindings(),
        4,
        "首次武装必须把 4 个槽位绑到计划的 4 个键上（空槽的键是 nil）"
    );
    // 见证 ③：重新武装的次数 == 发布份数（否则这段测的是"从没武装过"）。
    let publishes = (QUANTA - 1) / EVERY;
    assert!(
        publishes >= 6,
        "周期重发必须真的发生多次（实得 {publishes}）"
    );
    assert_eq!(
        republished.runtime.stats().snapshot_switches,
        publishes as u64,
        "{publishes} 份等价快照必须触发同样多次切换"
    );
    // 判据：同计划重发 **不增加** 重绑次数（不换主人 ⇒ 不清线）。
    assert_eq!(
        republished.runtime.pdc_rebindings(),
        once.runtime.pdc_rebindings(),
        "同一份计划的重新武装不得产生任何新的槽位重绑（⇒ 不得清延迟线）"
    );
    assert_eq!(once.runtime.stats().pdc_unarmed_nodes, 0);
    assert_eq!(republished.runtime.stats().pdc_unarmed_nodes, 0);
    assert!(
        renders_are_bit_identical(&once, &republished),
        "同一份 PDC 计划的重新武装必须与只武装一次逐位相同"
    );
    println!(
        "[engine-idem/5-5] 同计划重发 7 次：重绑 {} -> {}（不增），音频逐位相同",
        once.runtime.pdc_rebindings(),
        republished.runtime.pdc_rebindings()
    );
}

// ---------------------------------------------------------------------------
// 类别⑥ 多声道一致性
// ---------------------------------------------------------------------------

/// ⑥-6：居中的**单声道轨**在立体声母线上左右逐位相同（限制器未介入）。
#[test]
fn a_centred_mono_track_is_bit_identical_in_both_channels() {
    let notes = [NoteSpec::at(0, 960, 69, 127)];
    let rendered = render(&tuned_project(&notes, MixSpec::volume(-6.0)).project, 120);

    assert!(rendered.peak() > 0.01, "夹具必须真的出声");
    assert_eq!(
        rendered.stats.limiter_gain_reductions, 0,
        "这条判据要的是透明区（限制器一旦压限，测的是它的联动而不是声相）"
    );
    assert!(
        channels_are_bit_identical(&rendered.left, &rendered.right),
        "居中的单声道轨必须在两路上逐位相同"
    );
    println!(
        "[engine-chan/6-6] 居中：帧数={} 左右逐位相同=true 峰值={}",
        rendered.frames(),
        rendered.peak()
    );
}

/// ⑥-7：母线限制器**介入**时，居中的单声道母线仍然左右逐位相同。
///
/// 与 ⑥-6 的差别只有一处：音量 +6 dB 把夹具推到阈值之上 ⇒
/// `limiter_gain_reductions > 0`（联动增益真的在动），而左右仍必须逐位相同。
#[test]
fn the_stereo_linked_limiter_keeps_a_centred_mono_bus_bit_identical() {
    let notes = [NoteSpec::at(0, 960, 69, 127)];
    let rendered = render(&tuned_project(&notes, MixSpec::volume(6.0)).project, 120);

    assert!(
        rendered.stats.limiter_gain_reductions > 0,
        "夹具必须真的触发限制器（否则这条判据退化成 ⑥-6）"
    );
    assert!(
        rendered.peak() <= LIMITER_CEILING,
        "限制之后的峰值必须在天花板之下：{}",
        rendered.peak()
    );
    assert!(
        channels_are_bit_identical(&rendered.left, &rendered.right),
        "立体声联动的限制器不得让居中的两路分叉"
    );
    println!(
        "[engine-chan/6-7] 限制器介入：压限计数={} 峰值={} 左右逐位相同=true",
        rendered.stats.limiter_gain_reductions,
        rendered.peak()
    );
}

/// ⑥-8：全左声相 + 限制器介入 ⇒ 右声道**整段逐位 `+0.0`**（"只填一路"无串扰）。
///
/// 这就是"单声道信号喂立体声器件、另一路填 `0`"的形态：母线限制器两路进来，
/// 右路是 `+0.0`。联动增益对 `+0.0` 的乘法必须仍是 `+0.0`
/// （**逐位** `to_bits() == 0`，不是"约等于零"）。
#[test]
fn a_hard_left_pan_leaves_the_right_channel_bitwise_zero_through_the_limiter() {
    let notes = [NoteSpec::at(0, 960, 69, 127)];
    // 透明档：−6 dB ⇒ 全左峰值 4.94e-1，**低于**阈值（限制器不参与）。
    let quiet = render(
        &tuned_project(
            &notes,
            MixSpec {
                volume_db: -6.0,
                ..MixSpec::pan(-1.0)
            },
        )
        .project,
        120,
    );
    // 压限档：+6 dB ⇒ 全左峰值 9.00e-1，越过阈值（限制器真的在压）。
    let loud = render(
        &tuned_project(
            &notes,
            MixSpec {
                volume_db: 6.0,
                ..MixSpec::pan(-1.0)
            },
        )
        .project,
        120,
    );
    assert_eq!(
        quiet.stats.limiter_gain_reductions, 0,
        "透明档必须真的在透明区（否则这一档测的是限制器，不是声相）"
    );
    assert!(
        loud.stats.limiter_gain_reductions > 0,
        "响的那一档必须真的压限"
    );
    assert!(
        quiet.peak() > 0.1 && loud.peak() > 0.5,
        "左声道必须出声（透明档 {} / 压限档 {}）",
        quiet.peak(),
        loud.peak()
    );
    for (label, arm) in [("透明档", &quiet), ("压限档", &loud)] {
        assert!(
            arm.right.iter().all(|sample| sample.to_bits() == 0),
            "{label}: 全左时右声道必须整段逐位 +0.0（不得有串扰或 -0.0）"
        );
    }
    println!(
        "[engine-chan/6-8] 全左：透明档压限计数={} 压限档压限计数={} 右声道逐位 +0.0",
        quiet.stats.limiter_gain_reductions, loud.stats.limiter_gain_reductions
    );
}

/// ⑥-9：两条声道来自**同一条标量声相曲线**（逐位，含 `cos(π/2)` 的浮点残差）。
///
/// 声明关系（`pan = -1` 记 `hl`、`pan = +1` 记 `hr`、`c = cos(π/2)`）：
///
/// ```text
/// hr.left[i]  == hl.left[i] * c     （全右的左声道 == 全左的左声道 × cos(π/2)）
/// hr.right[i] == hl.left[i]         （全右的右声道 == 全左的左声道）
/// hl.right[i] == +0.0               （全左的右声道逐位零）
/// ```
///
/// 三条都是**逐位**等号：`c` 取自 [`pan_gains`]（构造期算好的那一条曲线），
/// 因此"左右各算一份曲线、差一个 ulp"会立刻变红。夹具刻意在透明区
/// （−6 dB）⇒ 限制器不参与。
///
/// ⚠ 左声道的期望写成 `+0.0 + hl × c` 而不是 `hl × c`：母线汇流是
/// `母线 += 轨 × 增益`（初值来自 `AudioBlock::silence()` 的 `+0.0`），而
/// `+0.0 + (-0.0) = +0.0` ⇒ 单声道源样本为 `±0.0` 的那些帧，符号位被累加器
/// 归一成 `+0.0`。加号不是容差，它把累加器的初值也钉进等式。
#[test]
fn both_channels_follow_the_same_scalar_pan_curve_bit_for_bit() {
    let notes = [NoteSpec::at(0, 960, 69, 127)];
    let quiet = |pan: f32| MixSpec {
        volume_db: -6.0,
        ..MixSpec::pan(pan)
    };
    let hard_left = render(&tuned_project(&notes, quiet(-1.0)).project, 120);
    let hard_right = render(&tuned_project(&notes, quiet(1.0)).project, 120);
    assert_eq!(hard_left.stats.limiter_gain_reductions, 0);
    assert_eq!(hard_right.stats.limiter_gain_reductions, 0);
    assert!(hard_left.peak() > 0.1, "全左必须出声");

    let (cos_half_pi, sin_half_pi) = pan_gains(1.0, PanLaw::ConstantPowerMinus3dB);
    assert_eq!(
        sin_half_pi.to_bits(),
        1.0f32.to_bits(),
        "全右的右增益必须是 1.0"
    );
    for (frame, (hl, hr)) in hard_left
        .left
        .iter()
        .zip(hard_right.left.iter())
        .enumerate()
    {
        assert_eq!(
            hr.to_bits(),
            (0.0f32 + *hl * cos_half_pi).to_bits(),
            "第 {frame} 帧：全右的左声道必须是全左的左声道 × cos(π/2)（逐位）"
        );
    }
    for (frame, (hl, hr)) in hard_left
        .left
        .iter()
        .zip(hard_right.right.iter())
        .enumerate()
    {
        assert_eq!(
            hr.to_bits(),
            hl.to_bits(),
            "第 {frame} 帧：全右的右声道必须等于全左的左声道（逐位）"
        );
    }
    assert!(
        hard_left.right.iter().all(|sample| sample.to_bits() == 0),
        "全左的右声道必须逐位 +0.0"
    );
    println!(
        "[engine-chan/6-9] 同一标量曲线：左残差峰值={:e}（= 全幅 × |cos(π/2)|）",
        hard_right
            .left
            .iter()
            .fold(0.0f32, |peak, s| peak.max(s.abs()))
    );
}

/// ⑥-10：交错输出的**通道映射**：`ch0 = 左`、`ch1..chN-1 = 右`（N = 1/2/4/6 逐位）。
///
/// 参照刻意取自**块自己的两条声道**（`AudioBlock::left()` / `right()` 直接访问
/// `left` / `right` 数组），而不是再跑一遍两声道渲染 —— 后者与 `process_quantum`
/// 走的是**同一个** `AudioBlock::get(channel, _)`，映射一旦写反，参照会跟着一起
/// 写反，判据就是自证的（实测：把 `get` 的左右分支对调，两声道渲染做参照时本判据
/// **仍然全绿**）。用 `left()` / `right()` 做参照，注入当场变红。
///
/// 每判据跑 40 个量子并**逐量子**核对（不是只核对最后一个块）。
///
/// ⚠ 本判据**只钉映射**，不钉"单声道设备应当怎么混"（见模块文档 §4 的发现）。
#[test]
fn the_interleaved_output_maps_channel_zero_to_left_and_the_rest_to_right() {
    for (label, pan) in [("centre", 0.0f32), ("hard-right", 1.0)] {
        let notes = [NoteSpec::at(0, 960, 69, 127)];
        let project = tuned_project(&notes, MixSpec::pan(pan)).project;
        assert!(render(&project, 1).peak() > 0.01, "{label}: 夹具必须出声");

        for channels in [1u16, 2, 4, 6] {
            let stride = usize::from(channels);
            let mut rig = EventRig::new(&project, 1, channels);
            let mut checked = 0usize;
            for _ in 0..40 {
                rig.quantum(channels);
                // 刚渲染完的那个块仍在 `last_block()` 里 ⇒ 与交错输出逐路核对。
                let block = rig.runtime.last_block();
                for frame in 0..DEFAULT_BLOCK_FRAMES {
                    for channel in 0..stride {
                        let expected = if channel == 0 {
                            block.left()[frame]
                        } else {
                            block.right()[frame]
                        };
                        assert_eq!(
                            rig.output[frame * stride + channel].to_bits(),
                            expected.to_bits(),
                            "{label} ch={channels}: 第 {frame} 帧第 {channel} 路"
                        );
                        checked += 1;
                    }
                }
            }
            assert_eq!(checked, 40 * DEFAULT_BLOCK_FRAMES * stride);
        }
    }
    // 发现（不改，只报数）：单声道设备上，全右声相的轨落在第 0 路以外的信息**无处可去**。
    println!(
        "[engine-chan/6-10] 单声道设备 + 全右声相（+6 dB）的峰值 = {:e}（左声道残差；见模块文档 §4 发现 1）",
        mono_peak_of_hard_right()
    );
}
