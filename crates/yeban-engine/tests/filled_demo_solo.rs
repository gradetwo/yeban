//! 官方 `filled` 样本的 Solo 判据。
//!
//! # 回归的事故
//!
//! `filled_project()` 的 Bass 曾预置 `solo_safe: true`。`solo_safe` 是模型里的
//! "不被别人的 Solo 压掉"开关，但界面今天没有写入口（ADR-0049）。
//! 于是用户打开第二个官方样本、点第一轨 Lead 的 S 后，Bass 仍照常发声、
//! 电平仍在跳，看起来就像**第一轨的 S 按钮失效**；点 Bass 自己的 S 则正常
//! （它本来就是被 Solo 的那条）。
//!
//! 本判据把用户口径固化成两条：
//! 1. 官方 filled 样本不得再给 Bass 预置 `solo_safe`；
//! 2. 第一轨 Lead `solo = true` 后，Bass 的音频片段增益必须为 `0.0`。
//!
//! 需要覆盖 `solo_safe` 功能本身的判据，应另建显式夹具，⛔ 不把它藏进官方样本。

use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::samples::filled_project;

#[test]
fn lead_solo_in_the_official_filled_sample_mutes_the_bass_audio_clip() {
    let project = filled_project();
    let lead = project
        .tracks
        .values()
        .find(|track| track.name == "Lead")
        .expect("filled 样本必须有 Lead 轨");
    let bass = project
        .tracks
        .values()
        .find(|track| track.name == "Bass")
        .expect("filled 样本必须有 Bass 轨");
    let lead_id = lead.id;
    let bass_id = bass.id;

    assert!(
        !bass.solo_safe,
        "官方 filled 样本的 Bass 不得预置 solo_safe=true：界面没有写入口，\
         用户点 Lead 的 S 会仍听见 Bass，看起来像 S 失效"
    );

    // 基线：没有 Solo 时 Bass 的音频片段确实有非零增益，后面的 0 才不是
    // "本来就无声"的假阳性。
    let baseline = EngineSnapshot::from_project(&project, 1).expect("filled 样本必须能投影");
    let bass_spans = baseline
        .audio_clip_schedule(&bass_id)
        .expect("Bass 必须有音频片段调度")
        .spans();
    assert!(!bass_spans.is_empty(), "Bass 必须有音频摆放");
    assert!(
        bass_spans.iter().any(|span| span.gain() > 0.0),
        "基线里 Bass 的音频片段增益必须大于 0"
    );

    // 用户操作：只点第一轨 Lead 的 S。
    let mut soloed = project;
    soloed.tracks.get_mut(&lead_id).expect("Lead 轨还在").solo = true;
    let snapshot = EngineSnapshot::from_project(&soloed, 2).expect("Lead solo 后仍必须能投影");

    let lead_gain = snapshot
        .schedule(&lead_id)
        .expect("Lead 必须有音符调度")
        .track_gain();
    assert!(
        lead_gain > 0.0,
        "被 Solo 的 Lead 自己必须仍然可闻，实际轨增益 {lead_gain}"
    );

    let bass_gain = snapshot
        .audio_clip_schedule(&bass_id)
        .expect("Bass 必须有音频片段调度")
        .spans()
        .iter()
        .map(|span| span.gain())
        .fold(0.0_f32, f32::max);
    assert_eq!(
        bass_gain, 0.0,
        "Lead Solo 后 Bass 的音频片段必须被压到 0；实际最大增益 {bass_gain}"
    );
}
