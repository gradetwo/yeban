//! ⭐ **实时松键/重触发的作用域**（T-FIX-389）：只碰"活声部"，⛔ 不截断时间轴排音。
//!
//! ## 缺口
//!
//! 复音触发有两条路：
//!
//! * **实时**（环事件 `NoteOn`）→ [`SynthEngine::note_on_with_sources`]：先对同频做一次
//!   [`PolySynth::note_off`] 再触发（T-FIX-275 的"抢占同一槽 = 重触发"裁定）；
//! * **时间轴/回放**（快照投影 → `render_track`）→ 直接 `slot.synth.note_on`，每条音符
//!   自带 `end_sample`，由器件在 `now >= end_sample` 时自己关 gate。
//!
//! 两条路的声部**共处同一个池**，而 [`PolySynth::note_off`] 的挑选口径只看"同频且未进
//! Release"，**不区分**这个声部是用户按下的（`end_sample = u64::MAX`，开区间）还是时间轴
//! 排进来的（有限 `end_sample`）。于是实时松键会按 `start_sample` 最小挑走**时间轴的**
//! 声部：用户松开自己按的键，被截短的却是正在回放的音符，而用户按下的那个声部
//! （`u64::MAX`）没人放，一直占着槽 —— 同一条裁定在"时间轴有同音"时**根本不成立**。
//!
//! ## 本文件钉的四条
//!
//! 1. 时间轴同音高重叠**是刻意的复音**（两条都该响）⇒ 直调是有理由的，⛔ 不能给它套
//!    实时裁定（套上去会把叠层音符吃掉）。
//! 2. 实时松键只放**活声部**：时间轴的两条同音排音必须**仍在按住**。
//! 3. 实时重触发（同键再按）只吞**活声部**：时间轴排音不受影响，且返回值仍报"重触发"。
//! 4. 没有活声部时，实时 `NoteOff` 必须报 **false**（调用方记孤儿松键），⛔ 不许把
//!    正在回放的时间轴音符当成"找到了"。

mod support;

use support::{NoteSpec, bare_track_project, note_project};
use yeban_dsp::envelope::AdsrStage;
use yeban_engine::snapshot::EngineSnapshot;
use yeban_engine::synth::SynthEngine;
use yeban_model::EntityId;

/// 中音 C。
const PITCH: u8 = 60;
/// 演奏力度。
const VELOCITY: u8 = 100;
/// 渲染块长（与 `DEFAULT_BLOCK_FRAMES` 同量级即可，本文件不依赖具体值）。
const BLOCK: usize = 512;
/// **活声部**的开区间终点（与 `note_on_with_sources` 的构造一致）。
const LIVE_END: u64 = u64::MAX;

/// 时间轴两条同音高声部的布局（120 BPM / 48 kHz / 960 PPQ ⇒ 1 tick = 25 样本）。
///
/// * A：tick 0 起、1920 ticks（⇒ 样本 0..48_000）
/// * B：tick 480 起、1920 ticks（⇒ 样本 12_000..60_000）
///
/// ⇒ 两条在样本 12_000..48_000 区间**同时按住**。
const NOTE_A: NoteSpec = NoteSpec::at(0, 1920, PITCH, VELOCITY);
const NOTE_B: NoteSpec = NoteSpec::at(480, 1920, PITCH, VELOCITY);
/// B 的起点（样本）＝ 480 ticks × 25。
const B_START: u64 = 12_000;

/// 武装好的时间轴装置。
struct TimelineRig {
    engine: SynthEngine,
    snapshot: EngineSnapshot,
    track: EntityId,
}

impl TimelineRig {
    fn with_overlap() -> Self {
        let fixture = note_project(&[NOTE_A, NOTE_B]);
        let snapshot =
            EngineSnapshot::from_project(&fixture.project, 1).expect("夹具工程必须能编译成快照");
        let mut engine = SynthEngine::new(48_000);
        engine
            .arm_from_snapshot(&snapshot)
            .expect("武装时间轴排音不得报错");
        assert!(
            snapshot
                .schedules()
                .get(&fixture.track)
                .is_some_and(|schedule| schedule.notes().len() == 2),
            "夹具必须投影出两条同音高音符（否则本文件测的是空壳）"
        );
        Self {
            engine,
            snapshot,
            track: fixture.track,
        }
    }

    /// 只推时间轴（不碰实时事件），把播放头推进到 `target`（样本）。
    fn render_to(&mut self, target: u64) {
        let schedule = self
            .snapshot
            .schedules()
            .get(&self.track)
            .expect("夹具必须有排音表");
        let mut out = vec![0.0_f32; BLOCK];
        while self.engine.position() < target {
            self.engine
                .render_track(self.track, Some(schedule), None, &mut out);
            // ⚠ `render_track` 只渲染、**不**推播放头（走带推进是 `advance` 的职责）⇒
            // 漏掉这一行，循环永远停在 position = 0（实测把判据挂成不返回）。
            self.engine.advance(BLOCK);
        }
    }

    /// 当前声部转储（`(活跃, end_sample, 包络阶段, 淡出剩余)`），只留活跃的。
    fn active(&self) -> Vec<(u64, AdsrStage)> {
        self.engine
            .debug_voice_state(self.track)
            .into_iter()
            .filter(|(active, _, _, _)| *active)
            .map(|(_, end, stage, _)| (end, stage))
            .collect()
    }

    /// 仍在按住（未进 Release）的声部数。
    fn gated(&self) -> usize {
        self.active()
            .into_iter()
            .filter(|(_, stage)| *stage != AdsrStage::Release)
            .count()
    }

    /// 活声部（`end_sample = u64::MAX`）的包络阶段。
    fn live_stage(&self) -> Option<AdsrStage> {
        self.active()
            .into_iter()
            .find(|(end, _)| *end == LIVE_END)
            .map(|(_, stage)| stage)
    }

    /// 有限终点（时间轴排音）的声部阶段。
    fn timeline_stages(&self) -> Vec<AdsrStage> {
        self.active()
            .into_iter()
            .filter(|(end, _)| *end != LIVE_END)
            .map(|(_, stage)| stage)
            .collect()
    }
}

#[test]
fn timeline_same_pitch_overlap_is_intentional_polyphony() {
    // ⭐ 判据 1（钉死"直调的理由"）：时间轴两条同音高音符重叠 ⇒ 池里**两条都在响**。
    // 这条同时是后三条的前提：如果哪天有人给时间轴通路套上实时裁定，这里会先红。
    let mut rig = TimelineRig::with_overlap();
    rig.render_to(B_START + BLOCK as u64 * 2);
    assert_eq!(
        rig.gated(),
        2,
        "时间轴同音高重叠必须是两条同时按住的声部，实测转储={:?}",
        rig.active()
    );
    assert_eq!(
        rig.live_stage(),
        None,
        "这一档没有实时事件 ⇒ 池里不许出现活声部，实测转储={:?}",
        rig.active()
    );
}

#[test]
fn live_release_only_touches_the_live_voice() {
    // ⭐ 判据 2（本单核心）：用户按住同音键再松开 ⇒ 只放**用户按下的那个**，
    // 时间轴那两条仍在回放的声部必须**原样按住**。
    let mut rig = TimelineRig::with_overlap();
    rig.render_to(B_START + BLOCK as u64 * 2);

    let retriggered = rig
        .engine
        .note_on_with_sources(rig.track, PITCH, VELOCITY, None, None, true);
    assert!(!retriggered, "首次按下同音键时还没有活声部 ⇒ 不算重触发");
    assert_eq!(rig.gated(), 3, "按下之后池里应是「两条排音 ＋ 一条活声部」");

    assert!(
        rig.engine
            .note_off_with_sources(rig.track, PITCH, None, true),
        "松键必须命中活声部（否则调用方会记成孤儿松键）"
    );
    assert_eq!(
        rig.live_stage(),
        Some(AdsrStage::Release),
        "松键必须放掉**用户按下的**那条活声部，实测转储={:?}",
        rig.active()
    );
    // ⚠ 只断言"**没有一条**时间轴声部进 Release"：它们此刻的具体阶段取决于包络形状
    // （实测是 Sustain / Decay），钉死阶段值等于把 ADSR 参数抄进判据；本单要钉的是
    // "松键没有截断它们"这件事。
    let stages = rig.timeline_stages();
    assert_eq!(stages.len(), 2, "时间轴两条排音必须都还在池里");
    assert!(
        stages.iter().all(|stage| *stage != AdsrStage::Release),
        "时间轴排音不许被实时松键截断（⛔ 这是本单判据），实测阶段={stages:?}"
    );
}

#[test]
fn live_retrigger_only_swallows_live_voices() {
    // ⭐ 判据 3（裁定本身不退化）：没有时间轴排音时，同键再按仍然"抢占同一槽、重触发"。
    let fixture = bare_track_project();
    let snapshot =
        EngineSnapshot::from_project(&fixture.project, 1).expect("夹具工程必须能编译成快照");
    let mut engine = SynthEngine::new(48_000);
    engine
        .arm_from_snapshot(&snapshot)
        .expect("武装空轨不得报错");
    let track = fixture.track;

    assert!(
        !engine.note_on_with_sources(track, PITCH, VELOCITY, None, None, true),
        "首次触发不算重触发"
    );
    assert!(
        engine.note_on_with_sources(track, PITCH, VELOCITY, None, None, true),
        "同键再按必须报「重触发」（T-FIX-275 裁定不许在本单退化成两条并存）"
    );
    // ⚠ 只数**仍按住**的声部：被重触发那条进 Release 后要等包络走完才回收
    // （`retire_finished` 的谓词），所以 `debug_active_voices` 会短暂读到 2。
    let gated = engine
        .debug_voice_state(track)
        .into_iter()
        .filter(|(active, _, stage, _)| *active && *stage != AdsrStage::Release)
        .count();
    assert_eq!(gated, 1, "重触发后只能有一条仍在按住的活声部");
}

#[test]
fn a_release_without_a_live_voice_is_an_orphan() {
    // ⭐ 判据 4（口径）：只有时间轴排音在响时，实时 `NoteOff` 必须报 false ——
    // 它指的是"用户按下的那条"，那条不存在 ⇒ 调用方按孤儿松键登记；
    // ⛔ 不许把正在回放的时间轴音符当成"找到了"（那正是判据 2 事故的另一面）。
    let mut rig = TimelineRig::with_overlap();
    rig.render_to(B_START + BLOCK as u64 * 2);

    assert!(
        !rig.engine
            .note_off_with_sources(rig.track, PITCH, None, true),
        "没有活声部时实时 NoteOff 必须报 false，实测转储={:?}",
        rig.active()
    );
    assert_eq!(
        rig.gated(),
        2,
        "孤儿松键不许动时间轴排音，实测转储={:?}",
        rig.active()
    );
}
