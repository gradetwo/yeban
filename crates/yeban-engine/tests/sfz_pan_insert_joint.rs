//! ⭐ **stage 17**：**区域声相 ＋ 插入器件**的端到端联合夹具（补登记在案的判据缺口）。
//!
//! 缺口原文（`build/patches/v28/README_补丁说明.md` 的"判据缺口"一节、
//! `rt.rs` 的 stage 11–15 注释）：stage 11–15 把插入链改成"每件器件内部带立体声
//! 入口"，但**没有一条判据**同时端起"源侧区域声相"与"已武装插入器件" ——
//! 现有证据都是两半分开的（`sfz_instrument.rs` 测源侧分声道、
//! `channel_strip_insert.rs` 测插入链）。于是"两边都绿、接起来却下混了"这种
//! 缺陷**不可能被发现**。
//!
//! # 怎么让它有牙
//!
//! 夹具是一条 **`pan=100`（硬右）** 的 SFZ 区域（直流样本 ⇒ 逐帧同一个电平）：
//!
//! * 硬右 ⇒ 源侧左缓冲**逐位 0**、右缓冲非零；
//! * 轨上再挂一台**已武装**的通道条（`eq_low_gain=6`）；
//! * 若插入链（或它前后任何一段搬运）把左右压成中值，左缓冲就**必然**出现非零
//!   ⇒ 判据必红；只有真正的分声道链才可能让左**逐位 0**撑到最后。
//!
//! 三条腿：
//! 1. **无设备**：左逐位 0、右非零（源侧分声道真的在跑）；
//! 2. **有设备**：`insert_strip_frames > 0`（链真的处理了帧）＋ 左**仍逐位 0** ＋
//!    右波形被 EQ 改过（指纹与无设备臂**不同** ⇒ 设备确实动了信号）；
//! 3. **反向腿**：同一夹具改成 `pan=0`（居中）⇒ 左**非零**（证明第 1/2 条腿的
//!    "左 0"来自声相，⛔ 不是"左边这条腿本来就哑"）。

mod support;

use std::sync::Arc;

use support::{NoteSpec, note_project, render_snapshot};
use yeban_engine::sfz_instrument::{InMemorySample, SfzInstrument, SfzPlan};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, ParameterValue, YebanProjectV1};
use yeban_sfz::parser::ParseLimits;

/// 一个音高、一秒：120 BPM / 960 PPQ ⇒ 480 tick ＝ 12 000 帧。
const NOTES: [NoteSpec; 1] = [NoteSpec::at(0, 480, 60, 127)];

/// 400 × 128 = 51 200 帧 ≈ 1.07 s（覆盖整个音符）。
const QUANTA: usize = 400;

/// 直流样本（0.4）：逐帧同一电平 ⇒ "左是否逐位 0"不受插值影响。
fn dc_sample(amplitude: f32, frames: usize) -> InMemorySample {
    InMemorySample {
        channels: 1,
        pcm: Arc::new(vec![amplitude; frames]),
        sample_hz: 48_000.0,
        loop_points: None,
    }
}

/// `pan` 由参数决定（`100` ＝ 硬右、`0` ＝ 居中）；其余逐字相同。
fn plan_with_pan(pan: Option<i32>, sample: &InMemorySample) -> SfzPlan {
    let pan_text = pan.map_or(String::new(), |pan| format!(" pan={pan}"));
    let text = format!(
        "<region> sample=a.wav key=60 pitch_keycenter=60{pan_text} \
         loop_mode=loop_continuous loop_start=0 loop_end=1000 \
         ampeg_attack=0.001 ampeg_sustain=100 ampeg_release=0.001 amp_veltrack=0"
    );
    let instrument = SfzInstrument::load(text, &ParseLimits::default()).expect("有效 SFZ");
    SfzPlan::prepare(&instrument, sample, 48_000.0).expect("SFZ 摊平")
}

/// 一台已武装的通道条（只开 EQ 低架，数值大 ⇒ 输出必变）。
fn armed_strip() -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: "Strip".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: vec![ParameterValue {
            name: "eq_low_gain".to_owned(),
            value: 6.0,
            unit: None,
        }],
        latency_samples: 0,
        plugin: None,
    }
}

fn mount(project: &mut YebanProjectV1, track: EntityId, device: Option<DeviceDefinition>) {
    let entry = project.tracks.get_mut(&track).expect("夹具里必须有那条轨");
    entry.devices = device.into_iter().collect();
}

/// 渲染一条腿：`pan` ＋ 是否挂通道条。
fn render_leg(pan: Option<i32>, strip: bool) -> support::Render {
    let mut fixture = note_project(&NOTES);
    let track = fixture.track;
    if strip {
        mount(&mut fixture.project, track, Some(armed_strip()));
    }
    let sample = dc_sample(0.4, 48_000);
    let plan = plan_with_pan(pan, &sample);
    let snapshot = EngineSnapshot::from_project(&fixture.project, 1)
        .expect("工程快照")
        .with_sfz(track, plan);
    render_snapshot(snapshot, QUANTA)
}

#[test]
fn a_panned_sfz_region_keeps_left_silent_through_an_armed_insert_device() {
    let bare = render_leg(Some(100), false);
    let with_strip = render_leg(Some(100), true);
    let centred = render_leg(Some(0), false);
    // ⭐ stage 18 反向腿：**没有 `pan`** 的区域 ⇒ 分声道路径**不该**被走到。
    let mono = render_leg(None, false);

    // ⚠ `pan=100` 的左增益是 `cos(π/2)` ＝ **6.1e-8**（不是精确 0）⇒ 左缓冲里有一层
    // 浮点残留；口径取"残留量级 ≤ 1e-6、且远小于右声道"。
    const RESIDUE: f32 = 1e-6;
    let bare_left_is_silent = bare.left.iter().all(|sample| sample.abs() <= RESIDUE);
    let strip_left_is_silent = with_strip.left.iter().all(|sample| sample.abs() <= RESIDUE);
    let bare_right_peak = bare.right.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let strip_right_peak = with_strip.right.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let centred_left_peak = centred.left.iter().fold(0.0f32, |m, s| m.max(s.abs()));

    let bare_left_peak = bare.left.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let bare_left_nonzero = bare.left.iter().filter(|s| **s != 0.0).count();
    println!(
        "[stage17] 诊断：pan=100 无设备 左峰={bare_left_peak:.9}、左非零帧数={bare_left_nonzero}；\
         pan=100+设备 左峰={:.9}",
        with_strip.left.iter().fold(0.0f32, |m, s| m.max(s.abs())),
    );
    println!(
        "[stage17] pan=100 无设备：左逐位 0={}、右峰={:.6}；pan=100 ＋ 通道条：\
         左逐位 0={}、右峰={:?}、insert_strip_frames={}；pan=0 无设备：左峰={:.6}；\
         stereo_track_frames 有设备={} / 无 pan={}；指纹 无设备={:#018x} / 有设备={:#018x}",
        bare_left_is_silent,
        bare_right_peak,
        strip_left_is_silent,
        strip_right_peak,
        with_strip.stats.insert_strip_frames,
        centred_left_peak,
        with_strip.stats.stereo_track_frames,
        mono.stats.stereo_track_frames,
        bare.fingerprint(),
        with_strip.fingerprint(),
    );

    // ⭐ stage 18：分声道路径真的被走到，且**每轨每量子恰好一次**（不是 2×）。
    let expected_stereo_frames = (QUANTA * yeban_engine::block::DEFAULT_BLOCK_FRAMES) as u64;
    assert_eq!(
        bare.stats.stereo_track_frames, expected_stereo_frames,
        "带区域声相的轨必须走分声道渲染（每量子恰好 ＋frames）"
    );
    assert_eq!(
        with_strip.stats.stereo_track_frames, expected_stereo_frames,
        "挂插入器件不得改变分声道渲染帧数（⛔ 不是 2×）"
    );
    assert_eq!(
        mono.stats.stereo_track_frames, 0,
        "没有 pan 的区域不得走分声道路径（反向腿）"
    );

    // 腿 1：硬右 ⇒ 源侧左只剩浮点残留、右真的出声。
    assert!(
        bare_left_is_silent,
        "pan=100 时左缓冲必须只剩浮点残留（源侧分声道）"
    );
    assert!(
        bare_right_peak > 0.01,
        "pan=100 时右缓冲必须真的出声（否则夹具是哑的）：{bare_right_peak}"
    );

    // 腿 2：挂上已武装的通道条后，链真的跑了，左**仍然**逐位 0。
    assert!(
        with_strip.stats.insert_strip_frames > 0,
        "已武装通道条必须真的处理过帧（insert_strip_frames 必须 > 0）"
    );
    assert!(
        strip_left_is_silent,
        "插入链前后都不得把左右压成中值 —— 左缓冲必须**仍然**只剩浮点残留"
    );
    assert!(
        strip_right_peak > 0.01,
        "挂通道条后右缓冲必须仍然出声：{strip_right_peak:?}"
    );
    assert_ne!(
        bare.fingerprint(),
        with_strip.fingerprint(),
        "EQ 必须真的改变了右声道（指纹必须不同）"
    );

    // 腿 3：反向腿 —— 居中时左**非零**（否则"左 0"可能只是左边本来就哑）。
    assert!(
        centred_left_peak > 0.01,
        "pan=0 时左缓冲必须出声（反向腿，证明 leg 1/2 的『左 0』来自声相）：{centred_left_peak}"
    );
}

/// ⭐ T-FIX-242：**没有区域声相的多声道样本**也必须选择分声道通路。
///
/// # 为什么必须有这条
///
/// `stereo_track` 旧条件只看 `regions_with_pan()` / `regions_with_cc_pan()`；
/// 样本自带右声道、区域 `pan=0` 时它是 `false` ⇒ 走 `render_track_with_cc`
/// （单声道）⇒ 只读声道 0，右声道整段丢失。`sfz_pan_insert_joint` 原来的
/// `mono` 反向腿只覆盖**单声道样本**，抓不到这一维。
///
/// 本判据用左右电平不同的双声道样本：修复前 `stereo_track_frames == 0` 且
/// 左右输出相同；修复后每量子恰 `＋frames`，左右输出必须不同。
#[test]
fn a_neutral_multichannel_sample_selects_the_stereo_path() {
    fn stereo_sample() -> InMemorySample {
        const FRAMES: usize = 48_000;
        let mut pcm = Vec::with_capacity(FRAMES * 2);
        for _ in 0..FRAMES {
            pcm.push(0.4);
            pcm.push(-0.3);
        }
        InMemorySample {
            channels: 2,
            pcm: Arc::new(pcm),
            sample_hz: 48_000.0,
            loop_points: None,
        }
    }

    fn render_sample_leg(sample: &InMemorySample) -> support::Render {
        let fixture = note_project(&NOTES);
        let track = fixture.track;
        let plan = plan_with_pan(None, sample);
        let snapshot = EngineSnapshot::from_project(&fixture.project, 1)
            .expect("工程快照")
            .with_sfz(track, plan);
        render_snapshot(snapshot, QUANTA)
    }

    let stereo = render_sample_leg(&stereo_sample());
    let mono = render_sample_leg(&dc_sample(0.4, 48_000));
    let expected_stereo_frames = (QUANTA * yeban_engine::block::DEFAULT_BLOCK_FRAMES) as u64;
    let peak = |pcm: &[f32]| pcm.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let bits = |pcm: &[f32]| -> Vec<u32> { pcm.iter().map(|s| s.to_bits()).collect() };

    println!(
        "[T-FIX-242] 多声道无 pan：stereo_track_frames={}（期望 {expected_stereo_frames}）、\
         左峰={:.6}、右峰={:.6}；单声道对照 stereo_track_frames={}",
        stereo.stats.stereo_track_frames,
        peak(&stereo.left),
        peak(&stereo.right),
        mono.stats.stereo_track_frames,
    );

    assert_eq!(
        stereo.stats.stereo_track_frames, expected_stereo_frames,
        "样本 channels>1 且区域声相全中性 ⇒ 必须走分声道通路（每量子恰好 ＋frames）"
    );
    assert_eq!(
        mono.stats.stereo_track_frames, 0,
        "单声道样本在同条件下仍走单声道通路（反向腿）"
    );
    assert!(
        peak(&stereo.left) > 0.01 && peak(&stereo.right) > 0.01,
        "两个声道都必须真的出声（否则『左右不同』可能只是右边全哑）"
    );
    assert_ne!(
        bits(&stereo.left),
        bits(&stereo.right),
        "右声道必须保留样本自己的内容（旧路径只读声道 0 ⇒ 两声道相同）"
    );
    assert_ne!(
        stereo.fingerprint(),
        mono.fingerprint(),
        "多声道样本与单声道样本的输出必须不同（证明样本真的按双声道进图）"
    );
}
