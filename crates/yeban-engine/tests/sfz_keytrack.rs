//! ⭐ **键盘跟踪（`pitch_keytrack` / `amp_keytrack` / `amp_keycenter`）在
//! 摊平、离线渲染与实时触发三条路上的落地**。
//!
//! # 这条判据在修什么
//!
//! 三个 opcode 此前在全仓**零实现**：解析层不认、引擎不判，未知 opcode
//! 被静默忽略 ⇒ 速率公式恒按 `pitch_keytrack=100`、增益链没有键位段。
//! 写了 `pitch_keytrack=0` 的打击乐/音效层会被错误移调，写了
//! `amp_keytrack` 的钢琴/吉他类库全键盘音量曲线错，而且**没有任何告警**。
//!
//! # 判据表
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | K1 | 摊平表真的带上 | `PreparedRegion::amp_keytrack()` / `amp_keycenter()` 逐字回读；`keytrack_gain` 与 `Region::keytrack_gain` 全键盘逐位相同 |
//! | K2 | 增益对账网格 | 128 键 × 3 力度：`PreparedRegion::gain_at` 与 `Region::playback_spec(..).total_gain()` **逐位**相同（结合序契约） |
//! | K3 | 音高三态（离线） | `pitch_keytrack=0` 时 60/84 两键渲染逐位相同；缺省时不同；`-100` 时 84 键渲染与缺省 36 键逐位相同（同速率） |
//! | K4 | 振幅端到端（离线） | 常值样本：中心键首帧 = 原值；±10 键 = 原值 × 10^(±10·keytrack/20) |
//! | K5 | 振幅端到端（实时） | 声部池触发：首帧与离线同值 |

use std::collections::BTreeMap;
use std::sync::Arc;

use yeban_engine::sfz_instrument::{InMemorySample, PreparedSfz, SfzInstrument, SfzVoicePool};
use yeban_sfz::parser::parse_text;
use yeban_sfz::playback::RenderRates;

/// 引擎采样率（样本同率 ⇒ 步进比恰是音高比）。
const HZ: f32 = 48_000.0;

/// 样本帧数（斜坡样本要够长：速率 4 的读法在 512 帧输出内不越界）。
const FRAMES: usize = 4096;

/// 一条常值 PCM。
fn flat(value: f32) -> InMemorySample {
    InMemorySample {
        pcm: Arc::new(vec![value; FRAMES]),
        sample_hz: HZ,
        loop_points: None,
    }
}

/// 一条斜坡 PCM（`pcm[i] = i / FRAMES`，全是精确二进制小数 ⇒
/// 读位置不同必然读出不同值，音高差可直接由输出反解）。
fn ramp() -> InMemorySample {
    InMemorySample {
        pcm: Arc::new((0..FRAMES).map(|i| i as f32 / FRAMES as f32).collect()),
        sample_hz: HZ,
        loop_points: None,
    }
}

/// 近似比较（增益链的乘法允许 1e-6 的 binary32 误差）。
fn near(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() < 1e-6
}

#[test]
fn k1_the_flattened_region_carries_the_keytrack_pair() {
    let text = "<region>sample=a.wav pitch_keytrack=25 amp_keytrack=1.3 amp_keycenter=48";
    let instrument = parse_text(text, &Default::default()).expect("parses");
    let region = &instrument.regions()[0];
    assert_eq!(region.pitch_keytrack, 25);
    assert_eq!(region.amp_keytrack, 1.3);
    assert_eq!(region.amp_keycenter, 48);

    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let plan = PreparedSfz::prepare_with_samples(&sfz, &[("a.wav".to_owned(), flat(0.5))], HZ)
        .expect("摊平必须成功");
    let prepared = &plan.regions()[0];
    assert_eq!(
        prepared.amp_keytrack(),
        1.3,
        "摊平表必须逐字带上 amp_keytrack"
    );
    assert_eq!(
        prepared.amp_keycenter(),
        48,
        "摊平表必须逐字带上 amp_keycenter"
    );
    // 两条路的因子是同一份公式：全键盘逐位相同（⛔ 不是近似）。
    for note in 0..=127u8 {
        assert_eq!(
            prepared.keytrack_gain(note).to_bits(),
            region.keytrack_gain(note).to_bits(),
            "note {note} 的键位因子必须逐位相同"
        );
    }
    assert_eq!(prepared.keytrack_gain(48), 1.0, "中心键上因子恰为 1.0");
}

#[test]
fn k2_gain_at_matches_total_gain_on_the_full_grid() {
    // volume / 力度曲线 / 键位段同时非缺省 ⇒ 结合序契约全程受检。
    let text = "<region>sample=a.wav volume=-3 amp_veltrack=80 amp_keytrack=1.3 amp_keycenter=48";
    let instrument = parse_text(text, &Default::default()).expect("parses");
    let region = &instrument.regions()[0];
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let plan = PreparedSfz::prepare_with_samples(&sfz, &[("a.wav".to_owned(), flat(0.5))], HZ)
        .expect("摊平必须成功");
    let prepared = &plan.regions()[0];
    let rates = RenderRates::new(HZ, HZ);
    for note in 0..=127u8 {
        for velocity in [1u8, 64, 127] {
            let offline = region.playback_spec(note, velocity, rates).total_gain();
            let baked = prepared.gain_at(note, velocity);
            assert_eq!(
                baked.to_bits(),
                offline.to_bits(),
                "note {note} velocity {velocity}：摊平增益必须与离线逐位相同"
            );
        }
    }
}

#[test]
fn k3_pitch_keytrack_zero_negative_and_default_render_differently() {
    let samples: BTreeMap<String, InMemorySample> =
        [("a.wav".to_owned(), ramp())].into_iter().collect();
    let render = |text: &str, note: u8| -> Vec<f32> {
        let sfz =
            SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
        let mut out = vec![0.0f32; 512];
        sfz.render_note_into_from_samples(&samples, note, 127, None, None, &mut out, HZ)
            .expect("离线渲染必须成功");
        out
    };

    // keytrack=0：全键盘同音高 ⇒ 60 与 84 的渲染逐位相同。
    let frozen = "<region>sample=a.wav pitch_keycenter=60 pitch_keytrack=0";
    assert_eq!(
        render(frozen, 60),
        render(frozen, 84),
        "pitch_keytrack=0 时两键必须逐位相同"
    );
    // 缺省（100）：84 键速率是 60 键的 4 倍 ⇒ 渲染必然不同。
    let plain = "<region>sample=a.wav pitch_keycenter=60";
    assert_ne!(
        render(plain, 60),
        render(plain, 84),
        "缺省 keytrack 时两键必须不同"
    );
    // keytrack=-100 的 84 键（+24 键 × -100 = -2400 音分）与缺省的 36 键
    // （-24 键 × 100 = -2400 音分）速率相同 ⇒ 渲染逐位相同。
    let reversed = "<region>sample=a.wav pitch_keycenter=60 pitch_keytrack=-100";
    assert_eq!(
        render(reversed, 84),
        render(plain, 36),
        "负 keytrack 的镜像键位必须与缺省同速率键位逐位相同"
    );
}

#[test]
fn k4_amp_keytrack_scales_the_offline_render_around_the_center() {
    let samples: BTreeMap<String, InMemorySample> =
        [("a.wav".to_owned(), flat(0.5))].into_iter().collect();
    let text = "<region>sample=a.wav amp_keytrack=1 amp_keycenter=60";
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let first_frame = |note: u8| -> f32 {
        let mut out = vec![0.0f32; 512];
        sfz.render_note_into_from_samples(&samples, note, 127, None, None, &mut out, HZ)
            .expect("离线渲染必须成功");
        out[0]
    };
    assert!(near(first_frame(60), 0.5), "中心键无偏移");
    // +10 键 × 1 dB/键 = +10 dB ⇒ ×10^(10/20) = ×√10。
    assert!(
        near(first_frame(70), 0.5 * 10.0f32.sqrt()),
        "+10 键必须 ×√10，实得 {}",
        first_frame(70)
    );
    // -10 键 = -10 dB ⇒ ×10^(-10/20) = ×1/√10。
    assert!(
        near(first_frame(50), 0.5 / 10.0f32.sqrt()),
        "-10 键必须 ×1/√10，实得 {}",
        first_frame(50)
    );
}

#[test]
fn k5_amp_keytrack_reaches_the_realtime_pool() {
    let text = "<region>sample=a.wav amp_keytrack=1 amp_keycenter=60";
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let plan = PreparedSfz::prepare_with_samples(&sfz, &[("a.wav".to_owned(), flat(0.5))], HZ)
        .expect("摊平必须成功");
    let mut pool = SfzVoicePool::new(HZ);
    let created = pool.trigger_note(&plan, 70, 127, u64::MAX, 1.0, None);
    assert_eq!(created, 1, "必须恰好建一个声部");
    let mut out = vec![0.0f32; 512];
    pool.render_into(&plan, 0, &mut out);
    assert!(
        near(out[0], 0.5 * 10.0f32.sqrt()),
        "实时首帧必须与离线同值（×√10），实得 {}",
        out[0]
    );
}
