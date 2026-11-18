//! 实时钢琴卷帘 / 键盘 `NoteOn` / `NoteOff` 按轨真实音源派发判据（鼓机 ＞ SFZ ＞ 复音）。
//!
//! 覆盖：
//! 1. 停止态 SFZ 实时试听播放真实 SFZ 采样波形（与默认复音正弦波可区分），`NoteOff` 走完 `ampeg_release` 衰减至静音且 `orphan_note_offs == 0`；
//! 2. 停止态 `loop_mode=one_shot` SFZ 在收到 `NoteOff` 后继续播完样本，且 `orphan_note_offs == 0`；
//! 3. 停止态鼓机 `NoteOn` 触发 `DrumMachine` 真实击音，未映射音高保持静音（不回退为复音正弦音）；
//! 4. 仅存在时间轴已调度 SFZ/鼓机尾音、而从未发过实时 `NoteOn` 的轨道，在 `TransportCommand::Stop` 后逐位静音（不破坏 `transport_contract`）；
//! 5. 同一量子内 `SeekTicks` 之前的实时 `NoteOn` 不会残留出声，而 `SeekTicks` 之后的 `NoteOn` 正常发声。

mod support;

use std::sync::Arc;

use support::{NoteSpec, note_project};
use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_engine::meter::meter_channel;
use yeban_engine::ring::{EngineEvent, EventSender, TransportCommand, event_channel};
use yeban_engine::rt::EngineRuntime;
use yeban_engine::sfz_instrument::{InMemorySample, SfzInstrument, SfzPlan};
use yeban_engine::snapshot::{EngineSnapshot, RetireQueue, SnapshotSlot, retire_channel};
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, ParameterValue};
use yeban_sfz::parser::ParseLimits;

struct LiveRig {
    _slot: Arc<SnapshotSlot>,
    _retire: RetireQueue,
    _collector: yeban_engine::meter::MeterCollector,
    sender: EventSender,
    runtime: Box<EngineRuntime>,
    out: Vec<f32>,
}

impl LiveRig {
    fn from_snapshot(snapshot: EngineSnapshot) -> Self {
        let slot = SnapshotSlot::new(snapshot);
        let (retire, retire_consumer) = retire_channel(32);
        let (sender, receiver) = event_channel(64);
        let (publisher, collector) = meter_channel(4096);
        let runtime = EngineRuntime::new_boxed(&slot, retire, receiver, publisher);
        Self {
            _slot: slot,
            _retire: retire_consumer,
            _collector: collector,
            sender,
            runtime,
            out: vec![0.0_f32; DEFAULT_BLOCK_FRAMES * 2],
        }
    }

    fn stop_first(&mut self) {
        assert_eq!(
            self.sender.publish(&[EngineEvent::Transport {
                command: TransportCommand::Stop,
            }]),
            1
        );
        self.runtime.process_quantum(&mut self.out, 2);
    }

    fn render_quanta(&mut self, quanta: usize) -> Vec<f32> {
        let mut samples = Vec::with_capacity(quanta * DEFAULT_BLOCK_FRAMES * 2);
        for _ in 0..quanta {
            self.runtime.process_quantum(&mut self.out, 2);
            samples.extend_from_slice(&self.out);
        }
        samples
    }
}

fn rms(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples
        .iter()
        .map(|&s| {
            let v = f64::from(s);
            v * v
        })
        .sum();
    (sum_sq / samples.len() as f64).sqrt()
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0_f32, |m, &s| m.max(s.abs()))
}

fn constant_sfz_sample(amplitude: f32, frames: usize) -> InMemorySample {
    InMemorySample {
        channels: 1,
        pcm: Arc::new(vec![amplitude; frames]),
        sample_hz: 48_000.0,
        loop_points: None,
    }
}

fn prepare_plan(sfz_text: &str, sample: &InMemorySample) -> SfzPlan {
    let inst = SfzInstrument::load(sfz_text.to_owned(), &ParseLimits::default()).expect("有效 SFZ");
    SfzPlan::prepare(&inst, sample, 48_000.0).expect("SFZ 摊平")
}

fn drum_device() -> DeviceDefinition {
    let params = [
        ("kick_note", 36.0_f32),
        ("snare_note", 38.0),
        ("closed_hat_note", 42.0),
        ("open_hat_note", 46.0),
        ("clap_note", 39.0),
    ];
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
        plugin: None,
    }
}

#[test]
fn stopped_sfz_live_audition_plays_real_sample_and_releases_cleanly() {
    let fixture = note_project(&[]);
    let track = fixture.track;

    // 直流恒值样本（0.4）+ `ampeg_release=0.05`：若错误回落到 PolySynth 正弦波，
    // 波形均值/符号不会稳定呈纯正包络平台。
    let sample = constant_sfz_sample(0.4, 48_000);
    let plan = prepare_plan(
        "<region> sample=a.wav key=60 pitch_keycenter=60 loop_mode=loop_continuous loop_start=0 loop_end=1000 ampeg_attack=0.001 ampeg_sustain=100 ampeg_release=0.04",
        &sample,
    );
    let snapshot = EngineSnapshot::from_project(&fixture.project, 1)
        .expect("工程快照")
        .with_sfz(track, plan);
    let mut rig = LiveRig::from_snapshot(snapshot);
    rig.stop_first();

    assert_eq!(
        rig.sender.publish(&[EngineEvent::NoteOn {
            track,
            pitch: 60,
            velocity: 127,
        }]),
        1
    );

    let sustained = rig.render_quanta(6);
    assert!(
        peak(&sustained) > 0.1,
        "停止态 SFZ 试听必须发出真实采样电平（peak={}）",
        peak(&sustained)
    );
    // 验证采样波形确实是正向 SFZ DC 样本而非复音正负对称正弦波。
    let min_val = sustained.iter().copied().fold(f32::INFINITY, f32::min);
    assert!(
        min_val >= -1.0e-5,
        "SFZ 直流正向样本不应出现复音负半周（min={min_val}）"
    );

    assert_eq!(
        rig.sender
            .publish(&[EngineEvent::NoteOff { track, pitch: 60 }]),
        1
    );
    // 40ms @ 48kHz ≈ 15 个量子；给足 30 个量子确保完整归零。
    let _releasing = rig.render_quanta(30);
    let settled_tail = rig.render_quanta(4);
    assert_eq!(
        rig.runtime.stats().orphan_note_offs,
        0,
        "SFZ 释放必须命中活跃声部，不得产生孤儿 NoteOff"
    );
    assert!(
        rms(&settled_tail) < 1.0e-4,
        "SFZ 松键完成 ampeg_release 后必须归零静音（tail_rms={}）",
        rms(&settled_tail)
    );
}

#[test]
fn stopped_one_shot_sfz_ignores_note_off_and_completes_sample() {
    let fixture = note_project(&[]);
    let track = fixture.track;

    // 4096 帧 ≈ 85ms @ 48kHz，32 个量子，one_shot 应播完整个样本。
    let sample = constant_sfz_sample(0.35, 4096);
    let plan = prepare_plan(
        "<region> sample=a.wav key=62 pitch_keycenter=62 loop_mode=one_shot",
        &sample,
    );
    let snapshot = EngineSnapshot::from_project(&fixture.project, 1)
        .expect("工程快照")
        .with_sfz(track, plan);
    let mut rig = LiveRig::from_snapshot(snapshot);
    rig.stop_first();

    assert_eq!(
        rig.sender.publish(&[
            EngineEvent::NoteOn {
                track,
                pitch: 62,
                velocity: 127,
            },
            EngineEvent::NoteOff { track, pitch: 62 },
        ]),
        2
    );

    let early = rig.render_quanta(8);
    assert!(
        peak(&early) > 0.1,
        "one_shot SFZ 在 NoteOff 后仍须继续发声（peak={}）",
        peak(&early)
    );
    assert_eq!(
        rig.runtime.stats().orphan_note_offs,
        0,
        "one_shot SFZ 的 NoteOff 不应被计为孤儿松键"
    );

    let _rest = rig.render_quanta(30);
    let tail = rig.render_quanta(4);
    assert!(
        rms(&tail) < 1.0e-5,
        "one_shot 样本播完后须自动静音（tail_rms={}）",
        rms(&tail)
    );
}

#[test]
fn stopped_drum_live_audition_triggers_drum_machine_and_ignores_unmapped_pitches() {
    let mut fixture = note_project(&[]);
    let track = fixture.track;
    fixture
        .project
        .tracks
        .get_mut(&track)
        .expect("轨存在")
        .devices = vec![drum_device()];

    let snapshot = EngineSnapshot::from_project(&fixture.project, 1).expect("鼓机工程快照");
    let mut rig = LiveRig::from_snapshot(snapshot);
    rig.stop_first();

    // 1) 未映射音高 (90) 在鼓机轨不得错误回退到 PolySynth 正弦发声。
    assert_eq!(
        rig.sender.publish(&[
            EngineEvent::NoteOn {
                track,
                pitch: 90,
                velocity: 100,
            },
            EngineEvent::NoteOff { track, pitch: 90 },
        ]),
        2
    );
    let unmapped = rig.render_quanta(4);
    assert!(
        peak(&unmapped) == 0.0,
        "鼓机轨未映射音高不得回退复音正弦发声（peak={}）",
        peak(&unmapped)
    );

    // 2) 映射内的底鼓音高 (36) 必须真触发鼓机器件并出声。
    let before_hits = rig.runtime.stats().drum_hits;
    let before_orphans = rig.runtime.stats().orphan_note_offs;
    assert_eq!(
        rig.sender.publish(&[EngineEvent::NoteOn {
            track,
            pitch: 36,
            velocity: 120,
        }]),
        1
    );
    let kick = rig.render_quanta(6);
    assert!(
        peak(&kick) > 0.05,
        "鼓机轨映射音高 (36) 停止态试听必须发出底鼓波形（peak={}）",
        peak(&kick)
    );
    assert!(
        rig.runtime.stats().drum_hits > before_hits,
        "停止态试听必须真实命中 DrumMachine（before={}, after={}）",
        before_hits,
        rig.runtime.stats().drum_hits
    );
    assert_eq!(
        rig.sender
            .publish(&[EngineEvent::NoteOff { track, pitch: 36 }]),
        1
    );
    let _ = rig.render_quanta(1);
    assert_eq!(
        rig.runtime.stats().orphan_note_offs,
        before_orphans,
        "鼓机试听配对 NoteOff 不应计作孤儿松键"
    );
}

#[test]
fn stopped_playback_without_live_audition_does_not_leak_scheduled_sfz_tail() {
    let fixture = note_project(&[NoteSpec::at(0, 4800, 60, 127)]);
    let track = fixture.track;
    let sample = constant_sfz_sample(0.5, 48_000);
    let plan = prepare_plan(
        "<region> sample=a.wav key=60 pitch_keycenter=60 loop_mode=loop_continuous loop_start=0 loop_end=1000 ampeg_release=0.2",
        &sample,
    );
    let snapshot = EngineSnapshot::from_project(&fixture.project, 1)
        .expect("工程快照")
        .with_sfz(track, plan);
    let mut rig = LiveRig::from_snapshot(snapshot);

    // 先在运行态触发时间轴上的长音符。
    let playing = rig.render_quanta(4);
    assert!(peak(&playing) > 0.1, "前置：时间轴 SFZ 音符确实出声");

    // Stop 后从未触发实时 NoteOn：轨道必须立即冻结为严格静音（不误播已调度尾音）。
    rig.stop_first();
    let after_stop = rig.render_quanta(4);
    assert_eq!(
        peak(&after_stop),
        0.0,
        "Stop 后无实时试听时不得泄露时间轴 SFZ 尾音（peak={}）",
        peak(&after_stop)
    );
}

#[test]
fn pre_seek_note_on_in_same_quantum_is_discarded_while_post_seek_audition_sounds() {
    let fixture = note_project(&[]);
    let track = fixture.track;
    let sample = constant_sfz_sample(0.4, 48_000);
    let plan = prepare_plan(
        "<region> sample=a.wav key=60 pitch_keycenter=60 ampeg_sustain=100 ampeg_release=0.02",
        &sample,
    );
    let snapshot = EngineSnapshot::from_project(&fixture.project, 1)
        .expect("工程快照")
        .with_sfz(track, plan);

    // 1) `[NoteOn, SeekTicks(0)]` 同一块内到达 ⇒ Seek 清除其之前的活音符 ⇒ 静音。
    let mut cleared = LiveRig::from_snapshot(snapshot.clone());
    cleared.stop_first();
    assert_eq!(
        cleared.sender.publish(&[
            EngineEvent::NoteOn {
                track,
                pitch: 60,
                velocity: 120,
            },
            EngineEvent::Transport {
                command: TransportCommand::SeekTicks(0),
            },
        ]),
        2
    );
    let cleared_audio = cleared.render_quanta(4);
    assert_eq!(
        peak(&cleared_audio),
        0.0,
        "同一量子中 SeekTicks 之前的 NoteOn 必须被 Seek 清空"
    );

    // 2) `[SeekTicks(0), NoteOn]` 同一块内到达 ⇒ Seek 之后的活音符正常发声。
    let mut kept = LiveRig::from_snapshot(snapshot);
    kept.stop_first();
    assert_eq!(
        kept.sender.publish(&[
            EngineEvent::Transport {
                command: TransportCommand::SeekTicks(0),
            },
            EngineEvent::NoteOn {
                track,
                pitch: 60,
                velocity: 120,
            },
        ]),
        2
    );
    let kept_audio = kept.render_quanta(4);
    assert!(
        peak(&kept_audio) > 0.1,
        "同一量子中 SeekTicks 之后的 NoteOn 必须正常发声（peak={}）",
        peak(&kept_audio)
    );
}
