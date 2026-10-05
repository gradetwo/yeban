//! 走带的**端到端接线判据**（`line/transport-engine`）。
//!
//! 这里测的不是状态机的数学（那是 `yeban_engine::transport` 的单元判据），
//! 而是**接线**：控制线程经**既有**的无锁事件通道发命令 → 实时侧在量子边界应用
//! → 位置/静音/快照退役三条既有契约都不破。
//!
//! 规范来源 (Normative):
//! - `[ARCH-DET-001]`：960 PPQ 整数 tick、固定 128 帧量子、逐位确定性；
//! - `[ARCH-RT-001]`：命令走 `rtrb` 批量 API，实时侧零分配零锁（另有 `harness = false`
//!   的分配计数判据 `transport_rt_zero_alloc`）；
//! - `[ARCH-RT-002]` / `[ROAD-M2-002]`：快照退役回收**不得**因为走带改动而退化；
//! - `[MODEL-ISO-001]`：播放头不进模型 / 快照；
//! - `docs/ledger/engine-sound-notes.md` needs **N2**："未播放时输出静音"的契约。

mod support;

use support::{TransportRig, note_project, tuned_project};
use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_engine::ring::TransportCommand;
use yeban_engine::snapshot::EngineSnapshot;
use yeban_engine::transport::{TransportReading, TransportState};
use yeban_model::project::SampleRate;

/// 48 kHz / 120 BPM 下的有理数（本文件里判据自己算一份，**不**读引擎内部字段）：
/// `tick_num = 120_000_000 × 960`、`tick_den = 1_000_000 × 60 × 48_000`。
const TICK_NUM_48K: u128 = 115_200_000_000;
const TICK_DEN_48K: u128 = 2_880_000_000_000;
/// 44.1 kHz 的分母（分子与上面相同：分子只含 BPM）。
const TICK_DEN_44K: u128 = 2_646_000_000_000;

/// 判据 ①：48 kHz、真实运行时量子（128 帧）下，播 N 个量子后的 tick 位置
/// **恰等于** `floor(N × 128 × tick_num / tick_den)`（逐点断言，不是"约等于"）。
#[test]
fn runtime_positions_are_exactly_the_integer_formula() {
    let fixture = note_project(&[]);
    let mut rig = TransportRig::new(&fixture.project, 1);
    assert_eq!(rig.runtime.stats().transport_state, TransportState::Playing);

    let mut expected = Vec::new();
    for quantum in 1..=64u128 {
        rig.quantum();
        let want = u64::try_from(quantum * 128 * TICK_NUM_48K / TICK_DEN_48K).unwrap();
        expected.push(want);
        assert_eq!(
            rig.runtime.position_ticks(),
            want,
            "第 {quantum} 个量子: 位置必须是公式的整数值"
        );
        assert_eq!(
            rig.runtime.position_samples(),
            u64::try_from(quantum * 128).unwrap()
        );
    }
    // 抽几个逐点读数贴出来（128 帧 = 5.12 tick ⇒ 25 / 51 / 102 / 307 …）
    assert_eq!(expected[0], 5);
    assert_eq!(expected[9], 51);
    assert_eq!(expected[24], 128);
    assert_eq!(expected[49], 256);
    assert_eq!(*expected.last().unwrap(), 327, "64 × 5.12 = 327.68 ⇒ 327");
    let stats = rig.runtime.stats();
    assert_eq!(stats.position_frames, 64 * 128);
    assert_eq!(stats.transport_quanta, 64);
    assert_eq!(stats.transport_commands, 0, "自由跑不需要任何命令");
}

/// 判据 ②：**不许**按 48 kHz 硬编码 —— 44.1 kHz 工程的位置仍然等于同一个公式，
/// 而且与 48 kHz 的读数**不同**（否则"换个分母"这件事根本没发生）。
#[test]
fn positions_do_not_assume_48khz() {
    let mut project = note_project(&[]).project;
    project.audio_config.sample_rate = SampleRate::Hz44100;
    let mut rig = TransportRig::new(&project, 1);
    assert_eq!(rig.slot.current().sample_rate(), 44_100);

    let mut seen = Vec::new();
    for quantum in 1..=4u128 {
        rig.quantum();
        let want = u64::try_from(quantum * 128 * TICK_NUM_48K / TICK_DEN_44K).unwrap();
        assert_eq!(
            rig.runtime.position_ticks(),
            want,
            "44.1 kHz 第 {quantum} 个量子"
        );
        seen.push(want);
    }
    assert_eq!(seen, vec![5, 11, 16, 22], "128 帧 = 5.57 tick ⇒ 5/11/16/22");
    assert_ne!(seen[3], 20, "若按 48 kHz 硬编码, 第 4 个量子会给出 20");
    // 同样的输入在 48 kHz 下必须是另一个值 —— 这条断言把"硬编码"钉死。
    let mut rig48 = TransportRig::new(&note_project(&[]).project, 1);
    rig48.quanta(4);
    assert_eq!(rig48.runtime.position_ticks(), 20);
    assert_ne!(rig48.runtime.position_ticks(), seen[3]);
}

/// 判据 ③ + ⑩（负向）：`Stop` 冻结时钟（再推 200 个量子位置一位不动），
/// `Play` 之后**从停住的位置继续**（不是回 0、不是跳一格）。
#[test]
fn stop_freezes_and_play_resumes_end_to_end() {
    let fixture = note_project(&[]);
    let mut rig = TransportRig::new(&fixture.project, 1);
    assert_eq!(rig.send(&[TransportCommand::Stop]), 1);
    rig.quantum();
    assert_eq!(rig.runtime.stats().transport_state, TransportState::Stopped);

    // 停住时推进 200 个量子：一个 tick 都不许动。
    rig.quanta(200);
    assert_eq!(rig.runtime.position_ticks(), 0, "停住不得推进 tick");
    assert_eq!(
        rig.runtime.stats().transport_quanta,
        0,
        "停住不产生推进量子"
    );
    assert_eq!(rig.runtime.position_samples(), 0);
    assert_eq!(rig.nonzero(), 0, "停住必须输出静音（needs N2 的契约）");

    // 播放 7 个量子 ⇒ 位置 = 7 × 5.12 = 35.84 ⇒ 35。
    assert_eq!(rig.send(&[TransportCommand::Play]), 1);
    rig.quanta(7);
    let frozen_at = rig.runtime.position_ticks();
    assert_eq!(frozen_at, 35);

    // 停住 → 再推 → 位置逐位不变。
    assert_eq!(rig.send(&[TransportCommand::Stop]), 1);
    rig.quanta(50);
    assert_eq!(rig.runtime.position_ticks(), frozen_at, "停止不得改变位置");

    // 再播：起点**恰好**是停住的位置。
    assert_eq!(rig.send(&[TransportCommand::Play]), 1);
    rig.quantum();
    assert_eq!(
        rig.runtime.position_ticks(),
        frozen_at + 5,
        "从停住的位置继续（35 + 5.12 ⇒ 40）"
    );
    assert_ne!(rig.runtime.position_ticks(), 0, "不得回到 0");
}

/// 判据 ④（端到端）：`SeekTicks(t)` 经事件通道生效 ⇒ 位置**恰好** t，
/// 并且此后从 t 起按公式推进；`t` 处的音符**真的**从该处开始发声。
#[test]
fn seek_via_the_event_channel_sets_the_exact_origin() {
    // 一个在 tick 960 起音的音符：1 tick = 25 帧（120 BPM / 48 kHz）⇒ 第 24_000 帧。
    let fixture = note_project(&[support::NoteSpec::at(960, 960, 69, 100)]);
    let mut rig = TransportRig::new(&fixture.project, 1);
    assert_eq!(rig.send(&[TransportCommand::Stop]), 1);
    rig.quantum();
    assert_eq!(rig.send(&[TransportCommand::SeekTicks(960)]), 1);
    rig.quantum();
    assert_eq!(rig.runtime.position_ticks(), 960, "定位后位置恰是 t");
    assert_eq!(rig.runtime.position_samples(), 24_000);

    // 从 960 起播：第一个量子（128 帧）内必须出声。
    assert_eq!(rig.send(&[TransportCommand::Play]), 1);
    rig.quantum();
    assert!(rig.nonzero() > 0, "定位到音符起点之后必须真的出声");
    assert_eq!(rig.runtime.position_ticks(), 960 + 5);

    // 定位到音符**之后**：该音符不再触发（同一量子内静音）。
    assert_eq!(rig.send(&[TransportCommand::SeekTicks(2_000)]), 1);
    rig.quanta(2);
    assert_eq!(rig.nonzero(), 0, "跳过的音符不得补触发");
    assert_eq!(rig.runtime.position_ticks(), 2_000 + 10);
}

/// 判据 ⑦：走带推进**不破坏**快照退役回收 —— 在 500 个走带量子之间反复换快照，
/// 旧快照必须经退役队列交回主线程（实时侧一次都不许就地释放）。
#[test]
fn snapshot_retire_recycling_survives_transport_playback() {
    let fixture = note_project(&[support::NoteSpec::at(0, 960, 69, 100)]);
    let mut rig = TransportRig::new(&fixture.project, 1);
    assert_eq!(rig.send(&[TransportCommand::Play]), 1);

    rig.quanta(10);
    let before = rig.runtime.position_ticks();
    for revision in 2..=8u64 {
        let next = EngineSnapshot::from_project(&fixture.project, revision).expect("等价快照");
        rig.slot.publish(next);
        rig.quantum();
    }
    assert_eq!(rig.runtime.revision(), Some(8), "音频线程追上最新快照");
    assert_eq!(
        rig.runtime.stats().snapshot_switches,
        7,
        "七次 publish 七次切换"
    );
    assert!(
        rig.queue.pending() >= 7,
        "旧快照必须全部在退役队列里（实际 {}）",
        rig.queue.pending()
    );
    assert!(rig.queue.drain(64) >= 7, "主线程侧负责真正 Drop");
    assert!(rig.slot.prune() >= 1, "读者推进后写者侧清单也可回收");
    // 走带在换快照期间**连续**推进（换快照不清零、不回跳）。
    assert!(
        rig.runtime.position_ticks() > before,
        "换快照期间走带必须继续推进"
    );
}

/// 判据 ⑪（端到端）：快照里的 BPM 变了 ⇒ 位置**不跳变**（一位不动），
/// 之后按新速度推进；演奏状态也不因换快照而中断。
#[test]
fn bpm_change_through_a_snapshot_never_jumps_the_position() {
    let fixture = note_project(&[]);
    let mut rig = TransportRig::new(&fixture.project, 1);
    assert_eq!(rig.send(&[TransportCommand::Play]), 1);
    rig.quanta(37);
    let before = rig.runtime.position_ticks();
    assert!(before > 0);

    let mut faster = fixture.project.clone();
    faster.bpm = 140.0;
    let snapshot = EngineSnapshot::from_project(&faster, 2).expect("新速度快照");
    assert_eq!(snapshot.bpm(), 140.0);
    rig.slot.publish(snapshot);
    rig.quantum();
    // arm 发生在量子边界：位置一位不动（换快照不得把播放头拉回 tick 0）。
    // 128 帧 @140BPM = 5.973 tick ⇒ 位置 = before + 5 或 +6（取决于相位余数）。
    let after = rig.runtime.position_ticks();
    assert!(
        after == before + 5 || after == before + 6,
        "换 BPM 后位置增量必须落在新速度的 [5, 6] 区间, 实际 {before} -> {after}"
    );
    assert_eq!(rig.runtime.stats().transport_state, TransportState::Playing);
    assert_eq!(rig.runtime.transport().bpm(), 140.0);
}

/// 走带的"停住 ⇒ 静音 / 播放 ⇒ 出声"是**输出**上的契约（needs N2），
/// 而且电平面板的结构性契约（每量子一次批量发布）与走带状态**无关**。
#[test]
fn stopped_engine_is_silent_and_meters_still_publish() {
    let fixture = tuned_project(
        &[support::NoteSpec::at(0, 4_000, 69, 120)],
        support::MixSpec::default(),
    );
    let mut rig = TransportRig::new(&fixture.project, 1);

    // 默认自由跑 ⇒ 出声。
    rig.quantum();
    assert!(rig.nonzero() > 0, "自由跑的默认行为必须与接入走带之前一致");

    assert_eq!(rig.send(&[TransportCommand::Stop]), 1);
    rig.quantum();
    // ⚠ 停住的**第一个**量子允许留下母线限制器的**前瞻尾巴**：`BusLimiter` 有 33 帧
    // 前瞻延迟（ADR-0001 D44b：这段延迟必须被 PDC 看见），因此输入变静音之后它还会
    // 吐出 ≤33 帧**已经存进去的**历史样本（≤ 33×2 = 66 个非零样本）。
    // 这不是"停住没生效"，而是一段**有意引入的延迟**；下一个量子必须逐位全零。
    assert!(
        rig.nonzero() <= 66,
        "停住后的尾巴只允许是限制器的 33 帧前瞻（实际 {} 个非零样本）",
        rig.nonzero()
    );
    rig.quantum();
    let stats_after_stop = rig.runtime.stats();
    assert_eq!(stats_after_stop.transport_state, TransportState::Stopped);
    assert_eq!(rig.nonzero(), 0, "停住之后必须逐位静音（尾巴最多一个量子）");
    assert_eq!(
        stats_after_stop.meter_bulk_publishes, 3,
        "每量子恰好一次批量发布 —— 与走带状态无关"
    );
    assert_eq!(
        stats_after_stop.meter_frames,
        2 * 3,
        "两条节点（1 轨 + 母线）× 3 个量子"
    );

    // 恢复播放：声音回来（声部池没有被停住期间的状态污染）。
    assert_eq!(rig.send(&[TransportCommand::Play]), 1);
    rig.quantum();
    assert!(rig.nonzero() > 0, "恢复播放必须出声");
}

/// 判据：走带读数镜面（RT → 控制侧）在真实装配上给出**一致的一帧**，
/// 并且与 `EngineStats` 逐字段一致（两个读数源不许互相打架）。
#[test]
fn transport_mirror_matches_the_engine_stats() {
    let fixture = note_project(&[]);
    let mut rig = TransportRig::new(&fixture.project, 1);
    let cold = rig.runtime.transport_reading();
    assert_eq!(cold.state, TransportState::Playing, "构造即自由跑");
    assert_eq!(cold.position_ticks, 0);

    assert_eq!(rig.send(&[TransportCommand::Play]), 1);
    rig.quanta(3);
    let reading: TransportReading = rig.runtime.transport_reading();
    let stats = rig.runtime.stats();
    assert_eq!(reading.state, stats.transport_state);
    assert_eq!(reading.position_ticks, stats.position_ticks);
    assert_eq!(reading.position_frames, stats.position_frames);
    assert_eq!(reading.commands_applied, stats.transport_commands);
    assert_eq!(reading.quanta_played, stats.transport_quanta);
    assert_eq!(reading.position_frames, 3 * DEFAULT_BLOCK_FRAMES as u64);
}
