//! ⭐ **踏板抬起必须放掉同一音高的全部挂起声部**（T-FIX-406，T-FIX-274 返工）。
//!
//! 复验实证的可达场景（不是纸面推演）：时间轴上有两段**同音高重叠**的音符
//!
//! ```text
//!  tick:  0        480        960        1440
//!  X:     [========================]                  (0..960)
//!  Y:                 [========================]      (480..1440)
//! ```
//!
//! 两段都在门内时来一次**实时** `NoteOn`（同一 pitch）⇒ 建出实时声部 **L**
//! （`end_sample = u64::MAX` ⇒ 它**永远不会**自己结束）。⚠ T-FIX-389 之后这次按键
//! **不**再截断时间轴上的 X/Y（实时裁定只放活声部，见 `live_release_scope.rs`）⇒
//! 池里是 X、Y、L **三条**同在按住。接着踩下踏板、松开实时键（L 被挂起）、抬起踏板。
//!
//! 抬起那一刻 `pedal_pending[pitch]` 只是**一个** bool，而 `note_off` 每次最多放掉
//! **一个**同频声部（T-FIX-275 语义）⇒ 单次调用只解决一条，**L 仍门开且再没有任何
//! 事件会释放它** ⇒ 用户听到"有个音卡住了"。
//!
//! 本文件钉两条（产品路径：`LiveRig` ＋ 走带 ＋ 实时事件，⛔ 不直调器件）：
//!
//! 1. `pedal_up_releases_every_held_voice_of_that_pitch` —— 抬起后渲染 500 量子
//!    （位置已过全部时间轴音符终点 ＋ 释放尾巴），尾部必须**静音**；
//! 2. `without_the_pedal_up_the_same_tail_is_still_audible` —— 对照组：**不抬踏板**
//!    时同一时刻的尾部仍**可闻** ⇒ 证明判据 1 的静音是"真的放掉了"，不是"装置本身没声"。
//!
//! 夹具栅格与 `support` 一致：120 BPM / 48 kHz / 960 PPQ ⇒ 1 tick = 25 样本，
//! 一个量子 = `DEFAULT_BLOCK_FRAMES` = 128 样本 = 5.12 tick。

mod support;

use std::sync::Arc;

use support::{NoteSpec, note_project};
use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_engine::meter::meter_channel;
use yeban_engine::ring::{EngineEvent, EventSender, TransportCommand, event_channel};
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{EngineSnapshot, RetireQueue, SnapshotSlot, retire_channel};
use yeban_model::EntityId;

/// 中音 C：时间轴两段重叠音符与实时试听声部**都用这一个音高**（重叠才是本单的病灶）。
const PITCH: u8 = 60;
/// 演奏力度。
const VELOCITY: u8 = 100;

/// 时间轴第一段 X：tick 0–960（样本 0–24000）。
const X_START_TICK: u64 = 0;
/// 时间轴第二段 Y：tick 480–1440（样本 12000–36000），与 X 在 480–960 重叠。
const Y_START_TICK: u64 = 480;
/// 两段都是 960 tick（四分音符）。
const NOTE_TICKS: u64 = 960;

/// 走到"Y 已起音、X 还没结束"的那个窗口里再动手（tick 480–960 ⇒ 量子 93.75–187.5）。
///
/// 取**量子 110**（样本 14080、tick 563.2）：稳稳落在窗口中部（Y 已响 16.25 tick、
/// X 还剩 396.8 tick）。
const QUANTA_INTO_OVERLAP: usize = 110;

/// 抬起之后再渲染这么多个量子：500 × 128 = 64000 样本 ≈ 1333 ms，
/// 位置已过 Y 的终点（36000）与任何释放尾巴（默认约 50 ms）。
const TAIL_QUANTA: usize = 500;

/// 取尾部 20 个量子（≈53 ms）当"释放走完之后"的读数窗口。
const WINDOW_QUANTA: usize = 20;

/// 一个只有**纯复音轨**（无 SFZ、无鼓机）的实时装置：事件环 → 运行时 → 音频。
///
/// ⚠ 与 `realtime_sustain_pedal.rs` 的 `LiveRig` **刻意各自持有一份**：那份的夹具是
/// 单音符（`NoteSpec::quarter`），本文件要的是**同音高重叠两段**；把两份合成一个
/// 参数化夹具会改动那条已绿的 6 条判据的输入（它们用 `rms` 阈值钉着幅度），不值得。
struct LiveRig {
    /// 夹具里那条 MIDI 轨。
    track: EntityId,
    _slot: Arc<SnapshotSlot>,
    _retire: RetireQueue,
    _collector: yeban_engine::meter::MeterCollector,
    sender: EventSender,
    runtime: Box<EngineRuntime>,
    out: Vec<f32>,
}

impl LiveRig {
    fn new() -> Self {
        let fixture = note_project(&[
            NoteSpec::at(X_START_TICK, NOTE_TICKS, PITCH, VELOCITY),
            NoteSpec::at(Y_START_TICK, NOTE_TICKS, PITCH, VELOCITY),
        ]);
        let snapshot =
            EngineSnapshot::from_project(&fixture.project, 1).expect("夹具工程必须能编译成快照");
        let slot = SnapshotSlot::new(snapshot);
        let (retire, retire_consumer) = retire_channel(32);
        let (sender, receiver) = event_channel(64);
        let (publisher, collector) = meter_channel(4096);
        let runtime = EngineRuntime::new_boxed(&slot, retire, receiver, publisher);
        Self {
            track: fixture.track,
            _slot: slot,
            _retire: retire_consumer,
            _collector: collector,
            sender,
            runtime,
            out: vec![0.0_f32; DEFAULT_BLOCK_FRAMES * 2],
        }
    }

    fn publish(&mut self, events: &[EngineEvent]) {
        assert_eq!(
            self.sender.publish(events),
            events.len(),
            "事件环必须收下全部事件"
        );
    }

    /// 推 `quanta` 个量子，返回**交错立体声**样本。
    fn render(&mut self, quanta: usize) -> Vec<f32> {
        let mut samples = Vec::with_capacity(quanta * DEFAULT_BLOCK_FRAMES * 2);
        for _ in 0..quanta {
            self.runtime.process_quantum(&mut self.out, 2);
            samples.extend_from_slice(&self.out);
        }
        samples
    }
}

/// 均方根（交错样本）；空输入 ⇒ 0。
fn rms(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let energy: f64 = samples
        .iter()
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum();
    (energy / samples.len() as f64).sqrt()
}

/// 尾部 `WINDOW_QUANTA` 个量子的均方根。
fn tail_rms(samples: &[f32]) -> f64 {
    let window = WINDOW_QUANTA * DEFAULT_BLOCK_FRAMES * 2;
    assert!(
        samples.len() >= window,
        "样本数不足以取出尾部窗口：{} < {}",
        samples.len(),
        window
    );
    rms(&samples[samples.len() - window..])
}

/// 把装置走到"X 与 Y 都在响、且已被实时 `NoteOn` 抢掉 X"的状态，返回实时声部建好后的读数。
///
/// 步骤与真人操作一致：走带开始 → 实时按键（抢掉最早的 X、建出 L）→ 踩踏板 → 松实时键。
/// 返回 `true` 表示这几个动作期间**确实有声音**（否则后面的静音判据没有意义）。
fn drive_to_pedalled_realtime_voice(rig: &mut LiveRig) -> bool {
    rig.publish(&[EngineEvent::Transport {
        command: TransportCommand::Play,
    }]);
    let start = rig.render(4);
    assert!(rms(&start) > 1e-4, "走带开始后 X 必须出声");

    let overlap = rig.render(QUANTA_INTO_OVERLAP);
    assert!(
        rms(&overlap) > 1e-4,
        "重叠窗口内必须还在响（否则没走到本单的场景）"
    );

    // 实时按键：引擎的"同 pitch 重触发"先 `note_off` 放掉最早的 X，再建实时声部 L。
    rig.publish(&[EngineEvent::NoteOn {
        track: rig.track,
        pitch: PITCH,
        velocity: VELOCITY,
    }]);
    let after_on = rig.render(4);
    assert!(
        rms(&after_on) > 1e-4,
        "实时 NoteOn 之后必须出声（L 已经建出来了）"
    );

    // 踩踏板 ⇒ 之后松实时键只是"挂起"，⛔ 不停声部。
    rig.publish(&[
        EngineEvent::ControlChange {
            controller: 64,
            value: 127,
        },
        EngineEvent::NoteOff {
            track: rig.track,
            pitch: PITCH,
        },
    ]);
    let held = rig.render(8);
    rms(&held) > 1e-4
}

#[test]
fn pedal_up_releases_every_held_voice_of_that_pitch() {
    // ⭐ 判据 1（主判据）：抬起踏板 ⇒ 该 pitch 上**全部**挂起声部都被放掉。
    let mut rig = LiveRig::new();
    assert!(
        drive_to_pedalled_realtime_voice(&mut rig),
        "踩下踏板后松实时键必须仍出声（否则本判据测的不是「挂起」）"
    );

    rig.publish(&[EngineEvent::ControlChange {
        controller: 64,
        value: 0,
    }]);
    let after = rig.render(TAIL_QUANTA);
    let tail = tail_rms(&after);
    eprintln!("[判据 1] 抬起后尾部 rms = {tail:e}");
    assert!(
        tail < 1e-6,
        "踏板抬起后必须放掉**全部**同频挂起声部（含 end_sample = u64::MAX 的实时声部 L）\
         ⇒ 500 量子之后尾部必须静音，实测 rms={tail}"
    );
}

#[test]
fn without_the_pedal_up_the_same_tail_is_still_audible() {
    // ⭐ 判据 2（对照组）：**不抬踏板** ⇒ 同一时刻的尾部仍可闻。
    //
    // 作用：证明判据 1 的静音来自"真的放掉了声部"，而不是"装置本身早就没声了"
    // （复验探针的对照组口径，实测尾部 RMS ≈ 0.46）。
    let mut rig = LiveRig::new();
    assert!(
        drive_to_pedalled_realtime_voice(&mut rig),
        "踩下踏板后松实时键必须仍出声"
    );

    // ⛔ 不发 CC64=0：踏板一直踩着，L 一直被挂住。
    let after = rig.render(TAIL_QUANTA);
    let tail = tail_rms(&after);
    eprintln!("[判据 2 对照组] 不抬踏板时尾部 rms = {tail:e}");
    assert!(
        tail > 1e-3,
        "对照组：不抬踏板时 L 仍被挂住 ⇒ 尾部必须仍可闻，实测 rms={tail}"
    );
}
