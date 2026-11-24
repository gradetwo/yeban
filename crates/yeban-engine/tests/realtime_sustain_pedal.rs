//! ⭐ **实时 CC64（延音踏板）在复音合成器上真的生效**（T-FIX-274）。
//!
//! 缺口：`SynthEngine::set_sustain_pedal` 原来只操作该轨的 SFZ 声部池（没有 SFZ 时
//! 直接返回），`note_off_with_sources` 的复音支路**无条件** `synth.note_off(...)`，
//! 而实时泵只对"有 SFZ 摊平表"的轨应用 CC64 ⇒ 纯合成器轨上的踏板整个失效：
//! CC64=127 时松开一个键，声音立刻进 Release，不像钢琴/采样器那样被踏板挂住。
//!
//! 本文件钉两层：
//!
//! * **音频层**（走产品路径 `EngineEvent::MidiCc` → `EngineRuntime::process_quantum`）：
//!   踏板踩下时松键**仍然出声**，踏板抬起后按释放尾巴归零；无踏板时逐位不变。
//! * **状态层**（`SynthEngine` 直调）：声部阶段（`Release` 与否）＋ 挂起表读数，
//!   并要求 `seek` 与"轨道不在快照里"两条路径都把踏板状态清干净。
//!
//! 夹具栅格与 `support` 一致：120 BPM / 48 kHz / 960 PPQ ⇒ 1 tick = 25 样本。

mod support;

use std::sync::Arc;

use support::{NoteSpec, note_project};
use yeban_dsp::envelope::AdsrStage;
use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_engine::meter::meter_channel;
use yeban_engine::ring::{EngineEvent, EventSender, TransportCommand, event_channel};
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{EngineSnapshot, RetireQueue, SnapshotSlot, retire_channel};
use yeban_engine::synth::SynthEngine;
use yeban_model::EntityId;

/// 中音 C（实时演奏用的那个键）。
const PITCH: u8 = 60;
/// 演奏力度。
const VELOCITY: u8 = 100;

/// 一个只有**纯复音轨**（无 SFZ、无鼓机）的实时装置：事件环 → 运行时 → 音频。
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
        let fixture = note_project(&[NoteSpec::quarter(PITCH)]);
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

    /// 把走带停下来 ⇒ 实时 `NoteOn` 走"停止态试听"那条路径（与真人演奏一致）。
    fn stop_first(&mut self) {
        assert_eq!(
            self.sender.publish(&[EngineEvent::Transport {
                command: TransportCommand::Stop,
            }]),
            1
        );
        self.runtime.process_quantum(&mut self.out, 2);
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

/// 60 个量子 ≈ 160 ms：比默认释放尾巴（约 50 ms）长一倍以上。
const TAIL_QUANTA: usize = 60;
/// 取尾部 20 个量子（≈53 ms）当"释放走完之后"的读数窗口。
const WINDOW_QUANTA: usize = 20;

#[test]
fn pedal_holds_a_synth_note_until_it_is_released() {
    // ⭐ 判据 1（音频层，产品路径）：踏板踩下 ⇒ 松键后仍在响；踏板抬起 ⇒ 归零。
    let mut rig = LiveRig::new();
    rig.stop_first();
    rig.publish(&[EngineEvent::NoteOn {
        track: rig.track,
        pitch: PITCH,
        velocity: VELOCITY,
    }]);
    let onset = rig.render(8);
    assert!(
        rms(&onset) > 1e-4,
        "实时 NoteOn 必须真的出声（否则后面的判据没有意义）：rms={}",
        rms(&onset)
    );

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
    let held = rig.render(TAIL_QUANTA);
    let window = &held[held.len() - WINDOW_QUANTA * DEFAULT_BLOCK_FRAMES * 2..];
    assert!(
        rms(window) > 1e-4,
        "踏板踩下时松键**不许**停声部：释放窗口内仍有声，实测 rms={}",
        rms(window)
    );

    rig.publish(&[EngineEvent::ControlChange {
        controller: 64,
        value: 0,
    }]);
    let after = rig.render(TAIL_QUANTA);
    let window = &after[after.len() - WINDOW_QUANTA * DEFAULT_BLOCK_FRAMES * 2..];
    assert!(
        rms(window) < 1e-6,
        "踏板抬起 + 释放尾巴之后必须归零，实测 rms={}",
        rms(window)
    );
}

#[test]
fn without_the_pedal_the_same_release_is_bit_for_bit_unchanged() {
    // ⭐ 判据 1（对照组）：不碰 CC64 ⇒ 松键立刻进释放段，与改动前一样。
    let mut rig = LiveRig::new();
    rig.stop_first();
    rig.publish(&[EngineEvent::NoteOn {
        track: rig.track,
        pitch: PITCH,
        velocity: VELOCITY,
    }]);
    assert!(rms(&rig.render(8)) > 1e-4, "NoteOn 必须出声");
    rig.publish(&[EngineEvent::NoteOff {
        track: rig.track,
        pitch: PITCH,
    }]);
    let after = rig.render(TAIL_QUANTA);
    let window = &after[after.len() - WINDOW_QUANTA * DEFAULT_BLOCK_FRAMES * 2..];
    assert!(
        rms(window) < 1e-6,
        "没有踏板时松键必须照旧立刻释放，实测 rms={}",
        rms(window)
    );
}

/// 只有一条轨的 `SynthEngine`（不经过 `EngineRuntime`，为的是能读内部读数）。
fn engine_rig() -> (SynthEngine, EntityId) {
    let track = EntityId::new();
    let mut engine = SynthEngine::new(48_000);
    engine.begin_snapshot(48_000, &[track], [], []);
    (engine, track)
}

/// 该轨**在响的那个**声部的包络阶段（`None` = 没有活跃声部）。
///
/// ⚠ 不能取 `first()`：转储里 16 条槽位记录全在，第一条通常是**空槽**
/// （`stage = Idle`，`active = false`）⇒ 那样读到的是"没声部"而不是"被释放"。
fn first_stage(engine: &SynthEngine, track: EntityId) -> Option<AdsrStage> {
    engine
        .debug_voice_state(track)
        .into_iter()
        .find(|(active, _, _, _)| *active)
        .map(|(_, _, stage, _)| stage)
}

#[test]
fn pedal_keeps_the_voice_out_of_release_and_releasing_it_drains_the_table() {
    // ⭐ 判据 2（状态层）：踏板踩下 ⇒ 松键后**不进 Release**、挂起表有 1 条；
    // 踏板抬起 ⇒ 进 Release、挂起表清空。
    let (mut engine, track) = engine_rig();
    assert!(
        !engine.note_on_with_sources(track, PITCH, VELOCITY, None, None, false),
        "首次触发没有窃取/重触发 ⇒ 返回值必须是 false（池没满、也没有同频声部）"
    );
    assert_ne!(
        first_stage(&engine, track),
        Some(AdsrStage::Release),
        "刚触发时不该在释放段"
    );

    assert_eq!(
        engine.set_sustain_pedal(track, true, None),
        0,
        "踩下不建声部，返回 0"
    );
    assert_eq!(
        engine.debug_pedal_state(track),
        Some((true, 0)),
        "刚踩下时还没有挂起键"
    );

    assert!(
        engine.note_off_with_sources(track, PITCH, None, false),
        "踏板踩下时松键必须算「命中」（否则调用方会记成孤儿松键）"
    );
    assert_ne!(
        first_stage(&engine, track),
        Some(AdsrStage::Release),
        "踏板踩下时松键**不许**把声部送进释放段（本条是 T-FIX-274 的核心）"
    );
    assert_eq!(
        engine.debug_pedal_state(track),
        Some((true, 1)),
        "松开的那个键必须被记进挂起表"
    );

    assert_eq!(
        engine.set_sustain_pedal(track, false, None),
        0,
        "纯合成器轨没有 SFZ 释放声部可建 ⇒ 返回 0"
    );
    assert_eq!(
        first_stage(&engine, track),
        Some(AdsrStage::Release),
        "踏板抬起后声部必须进释放段"
    );
    assert_eq!(
        engine.debug_pedal_state(track),
        Some((false, 0)),
        "踏板抬起后挂起表必须清空（无残留）"
    );
}

#[test]
fn re_pressing_the_key_drops_its_pending_flag() {
    // ⭐ 判据 2（配对）：键又按下了 ⇒ 它不再是挂起键（踏板抬起时不许误放它）。
    let (mut engine, track) = engine_rig();
    engine.note_on_with_sources(track, PITCH, VELOCITY, None, None, false);
    engine.set_sustain_pedal(track, true, None);
    engine.note_off_with_sources(track, PITCH, None, false);
    assert_eq!(engine.debug_pedal_state(track), Some((true, 1)));
    engine.note_on_with_sources(track, PITCH, VELOCITY, None, None, false);
    assert_eq!(
        engine.debug_pedal_state(track),
        Some((true, 0)),
        "重新按下的键必须从挂起表里摘掉"
    );
}

#[test]
fn seek_drops_the_pedal_state() {
    // ⭐ 判据 2（复位）：`seek` 把全部声部放掉 ⇒ 踏板状态与挂起表也必须清空。
    let (mut engine, track) = engine_rig();
    engine.note_on_with_sources(track, PITCH, VELOCITY, None, None, false);
    engine.set_sustain_pedal(track, true, None);
    engine.note_off_with_sources(track, PITCH, None, false);
    assert_eq!(engine.debug_pedal_state(track), Some((true, 1)));
    engine.seek(0);
    assert_eq!(
        engine.debug_pedal_state(track),
        Some((false, 0)),
        "seek 之后踏板状态必须归零（挂起表清空）"
    );
}

#[test]
fn a_track_leaving_the_snapshot_releases_its_pedalled_keys() {
    // ⭐ 判据 2（复位）：轨道从快照里消失 ⇒ 踏板抬起事件永远不会来 ⇒ 挂起的键
    // 必须在这里放掉（否则会一直占着声部 = 用户听到"卡住的音"）。
    let (mut engine, track) = engine_rig();
    engine.note_on_with_sources(track, PITCH, VELOCITY, None, None, false);
    engine.set_sustain_pedal(track, true, None);
    engine.note_off_with_sources(track, PITCH, None, false);
    assert_eq!(engine.debug_pedal_state(track), Some((true, 1)));

    let empty: [&EntityId; 0] = [];
    engine.begin_snapshot(48_000, empty, [], []);
    // 轨道回到快照里再读：槽位按身份保留，所以读数仍然属于同一条轨。
    engine.begin_snapshot(48_000, &[track], [], []);
    assert_eq!(
        engine.debug_pedal_state(track),
        Some((false, 0)),
        "轨道离场时踏板状态与挂起表都必须清空"
    );
    assert_eq!(
        first_stage(&engine, track),
        Some(AdsrStage::Release),
        "被踏板挂住的那个键必须在离场时真的进释放段"
    );
}
