//! 任务单 `#135` 的**实时**判据：音频片段（`ClipContent::Audio`）随走带真的发声。
//!
//! # 为什么需要这个文件
//!
//! `#134` 只接通了**离线**渲染（`yeban-app::render_core` → `audio_clip::AudioClipSource`）；
//! `EngineSnapshot::from_project` 的登记（`snapshot.rs` 的 `project_schedules` 第 5 条）
//! 当时写着"音频片段不产生声音"。本单把摆放投影进快照、把 PCM 由控制侧注入、在
//! `rt::render_branch` 的**逐轨单声道暂存**上累加 ⇒ 实时走带也发声。
//!
//! # 判据与文件头的对应（逐条可查）
//!
//! | 判据 | 内容 |
//! | :--- | :--- |
//! | [`realtime_transport_plays_an_audio_clip`] | 走带里跑 200 个量子 ⇒ 非零样本 ＋ 峰值（**正腿**） |
//! | [`an_audio_clip_without_pcm_and_a_removed_clip_are_both_silent`] | **反向腿**：有摆放但没注入 PCM ⇒ 逐位静音；移除摆放 ⇒ 与它**指纹相同** |
//! | [`the_play_cursor_is_derived_from_the_transport_position`] | 起点 tick ≠ 0：前 / 中 / 后三段读数 ⇒ 音频恰在**那一 tick**（＋母线限制器的 33 帧前瞻）起声、在摆放终点收声 |
//! | [`a_short_asset_leaves_a_silent_tail`] | 素材比摆放短 ⇒ 硬切（留静音尾巴，⛔ 不越界读） |
//! | [`stereo_assets_are_downmixed_and_mono_is_read_directly`] | 立体声 `(L+R)/2` 与单声道**逐位相同**；单声道直读 |
//! | [`the_clip_gain_is_projected_into_the_sample_path`] | `gain_db = -6` ⇒ 峰值恰为 0 dB 臂的 `db_to_gain(-6)` 倍 |
//! | [`pcm_injection_rejects_unsupported_shapes`] | 采样率不符 / 声道数 ≠ 1,2 / 半帧 ⇒ **拒绝**（⛔ 不静默） |
//!
//! # 口径（与离线路径的**唯一**差别，如实登记）
//!
//! 实时的逐轨暂存是**单声道、声相之前**，因此：① 立体声素材按 `(L+R)/2` 折单声道
//! （离线是 `StereoToMono`，同式）；② 声相由**逐轨混音的后续一级**施加（与 MIDI 同径），
//! 因此同一个 −6 dB 片段在两条路上可能差 1 ULP（两条路各算一次 `db_to_gain`）——
//! 本单⛔ 不声称与离线**逐位**相同，只声称**同一个增益公式**与**同一个 tick → 帧换算**。
//! ③ ⚠ `pan == 0` 时离线是 `(gain, gain)`、实时是引擎既有的等功率居中
//! `(1/√2, 1/√2)`（差 −3.01 dB）⇒ A1 的期望峰值就带着这个 `FRAC_1_SQRT_2`；
//! 那是引擎**既有**的逐轨声相口径（MIDI 同径），⛔ 不是本单引入。
//!
//! # 实时约束
//!
//! 本文件测的是**数值与游标**；**零分配/零锁/零 I/O** 由 `tests/rt_zero_alloc.rs` 的
//! 音频片段场景（四元组读数）钉住。

mod support;

use std::sync::Arc;

use yeban_engine::audio_clip::{AudioClipError, AudioClipSchedule, AudioClipSpan, AudioPcm};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_engine::synth::{samples_per_tick, tick_to_sample};
use yeban_model::YebanProjectV1;

use support::{
    Render, audio_clip_asset, audio_clip_project, audio_clip_project_at, render_snapshot,
};

/// 夹具采样率（`support` 的工程用模型默认 `audio_config`；本文件**现读**它，⛔ 不硬编码）。
fn fixture_rate(project: &YebanProjectV1) -> u32 {
    project.audio_config.sample_rate.hz()
}

/// 夹具的一 tick 帧数（120 BPM / 48 kHz / 960 PPQ ⇒ 25 帧）。
fn frames_per_tick(project: &YebanProjectV1) -> f64 {
    samples_per_tick(project.bpm, fixture_rate(project))
}

/// 一段**交织单声道**常量 PCM（`frames` 帧、每帧 `value`）。
fn mono_pcm(value: f32, frames: usize, sample_rate: u32) -> AudioPcm {
    let samples: Arc<[f32]> = Arc::from(vec![value; frames].into_boxed_slice());
    AudioPcm::new(samples, 1, sample_rate).expect("单声道常量 PCM 必须合法")
}

/// 从工程构造快照并注入一轨音频 PCM（**构造期**，允许分配）。
fn snapshot_with_pcm(project: &YebanProjectV1, pcm: AudioPcm) -> EngineSnapshot {
    EngineSnapshot::from_project(project, 1)
        .expect("夹具工程必须能编译成快照")
        .with_audio_asset(&audio_clip_asset(), pcm)
        .expect("注入的 PCM 必须被接受")
}

/// 左声道里**第一个非零**样本的下标（没有则 `None`）。
fn first_nonzero(render: &Render) -> Option<usize> {
    render.left.iter().position(|sample| *sample != 0.0)
}

/// 左声道 `[from, to)` 是否**全部**为零。
fn silent_in(render: &Render, from: usize, to: usize) -> bool {
    render.left[from..to].iter().all(|sample| *sample == 0.0)
}

/// A1（**正腿**）：实时走带跑 200 个量子 ⇒ 该轨输出非静音（非零样本 ＋ 峰值）。
///
/// 200 量子 = 25 600 帧 < 摆放的 960 tick = 24 000 帧？不：25 600 > 24 000 ⇒ 窗口**跨过**
/// 摆放终点，因此下面把"非零"的统计限制在摆放区间内（前 24 000 帧）。
#[test]
fn realtime_transport_plays_an_audio_clip() {
    let fixture = audio_clip_project();
    let rate = fixture_rate(&fixture.project);
    let snapshot = snapshot_with_pcm(&fixture.project, mono_pcm(0.5, rate as usize, rate));

    // 快照面的**事实**（不是从输出反推）：一条摆放、注入前 0 条有 PCM、注入后 1 条。
    assert_eq!(snapshot.audio_spans(), 1, "工程里有一个音频摆放");
    assert_eq!(snapshot.audio_pcm_spans(), 1, "注入之后那一条必须有 PCM");
    let schedule = snapshot
        .audio_clip_schedule(&fixture.track)
        .expect("音频轨必须在表里");
    assert_eq!(schedule.len(), 1);
    assert_eq!(schedule.spans()[0].start(), 0, "tick 0 ⇒ 帧 0");
    assert!(
        (schedule.spans()[0].gain() - 1.0).abs() < 1e-6,
        "0 dB 片段 × 0 dB 轨 ⇒ 单位增益（现读 {}）",
        schedule.spans()[0].gain()
    );

    // 200 量子 = 25 600 帧；摆放 24 000 帧 ⇒ 只看摆放区间内的样本。
    let rendered = render_snapshot(snapshot, 200);
    let inside = 24_000;
    let nonzero = rendered.left[..inside]
        .iter()
        .filter(|sample| **sample != 0.0)
        .count();
    let peak = rendered.left[..inside]
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    println!(
        "[#135/A1] 200 量子（25 600 帧）：摆放区间内非零样本={nonzero} 峰值={peak:.9} \
         左声道总非零={}",
        rendered.left.iter().filter(|s| **s != 0.0).count()
    );
    assert!(nonzero > 0, "实时走带里音频摆放必须真的发声");
    // 声相居中（`#300`）：`pan == 0` ⇒ 恒等 `(1, 1)`（⛔ 不再是等功率 −3 dB）
    // ⇒ 每声道 = 素材值 × 1.0。修前这里期望 `0.5 × 1/√2 = 0.35355338`（实测现为 0.5）。
    let expected = 0.5;
    assert!(
        (peak - expected).abs() < 1e-6,
        "峰值必须等于 素材值 × 居中声相增益（期望 {expected}, 实测 {peak}）"
    );
    // 反向腿的**同一条判据**：右声道也必须出声（母线与声相同一条路径）。
    assert!(
        rendered.right[..inside].iter().any(|sample| *sample != 0.0),
        "右声道同样必须出声"
    );
}

/// A2（**反向腿**）：有摆放但**没注入 PCM** ⇒ 逐位静音；把摆放**移除** ⇒ 与它**指纹相同**。
///
/// 这是"接线前逐位不变"的运行期证据：新增的逐轨混合代码在 `pcm == None` 与
/// "没有这张表"两种情形下都必须**一个样本都不碰**。
#[test]
fn an_audio_clip_without_pcm_and_a_removed_clip_are_both_silent() {
    let mut fixture = audio_clip_project();
    let rate = fixture_rate(&fixture.project);

    // (a) 有摆放、没有注入 PCM（`from_project` 的默认口径）
    let no_pcm_snapshot =
        EngineSnapshot::from_project(&fixture.project, 1).expect("夹具工程必须能编译成快照");
    assert_eq!(no_pcm_snapshot.audio_spans(), 1, "摆放**在**表里");
    assert_eq!(
        no_pcm_snapshot.audio_pcm_spans(),
        0,
        "没有注入 ⇒ 0 条有 PCM（⛔ 不把它读成静音的摆放）"
    );
    let no_pcm = render_snapshot(no_pcm_snapshot, 64);

    // (b) 把摆放从轨道上**移除**（clip_pool 保持不动：只去掉摆放）
    let track = fixture.track;
    fixture
        .project
        .tracks
        .get_mut(&track)
        .expect("夹具里必须有那条音频轨")
        .clips
        .clear();
    let removed_snapshot =
        EngineSnapshot::from_project(&fixture.project, 1).expect("夹具工程必须能编译成快照");
    assert_eq!(removed_snapshot.audio_spans(), 0, "移除后表里一条都没有");
    let removed = render_snapshot(removed_snapshot, 64);

    println!(
        "[#135/A2] 无 PCM：非零={} 峰值={} 指纹=0x{:016x}；移除摆放：非零={} 指纹=0x{:016x} \
         （PCM 帧数 {rate} 无关，仅记录采样率 {rate}）",
        no_pcm.nonzero(),
        no_pcm.peak(),
        no_pcm.fingerprint(),
        removed.nonzero(),
        removed.fingerprint()
    );
    assert_eq!(no_pcm.nonzero(), 0, "有摆放但没注入 PCM ⇒ 必须逐位静音");
    assert_eq!(removed.nonzero(), 0, "移除摆放 ⇒ 必须逐位静音");
    assert_eq!(
        no_pcm.fingerprint(),
        removed.fingerprint(),
        "两条反向腿必须**逐位**相同（新增路径一个样本都不许碰）"
    );
}

/// A3（**游标**）：起点 tick ≠ 0 ⇒ 音频从**那一 tick** 开始（给前 / 中 / 后三段读数）。
///
/// 夹具：起点 `480 tick`（= 12 000 帧）、时值 `960 tick`（= 24 000 帧）⇒ 终点 36 000 帧。
/// 走带从 tick 0 自由跑 400 个量子（51 200 帧）⇒ 三段：
///
/// * **前** `[0, 12 000)`：全零（tick 480 之前不放）；
/// * **中** `[12 000, 36 000)`：全非零（`#302` 起默认档限制器关 ⇒ 无 33 帧延迟）；
/// * **后** `[36 000, 51 200)`：全零（摆放终点硬切）。
///
/// ⚠ **素材用斜坡而不是常量**：常量 PCM 下"每个量子都从素材帧 0 重读"与"按绝对帧
/// 读"给出**同一个**值 ⇒ 游标错误会被常量掩盖。斜坡（`帧号 × 1e-6`）让输出值成为
/// **位置本身**的读数：下面三个精确值断言就是"素材帧 = 绝对帧 − span.start"的机械见证。
///
/// ⚠ **为什么没有游标状态也能对**：播放位置由走带的**绝对帧**导出 ⇒ 本判据就是那条
/// 导出的端到端读数（`EngineRuntime` 尺寸 ⛔ 不许为它增长，见 `tests/rt_new_boxed.rs`）。
#[test]
fn the_play_cursor_is_derived_from_the_transport_position() {
    const START_TICK: u64 = 480;
    const DURATION_TICKS: u64 = 960;
    const QUANTA: usize = 400;
    const RAMP_SCALE: f32 = 1e-6;
    let fixture = audio_clip_project_at(START_TICK, DURATION_TICKS, 0.0);
    let rate = fixture_rate(&fixture.project);
    let per_tick = frames_per_tick(&fixture.project);
    let start = usize::try_from(tick_to_sample(START_TICK, per_tick).expect("tick 换算"))
        .expect("帧号放得进 usize");
    let end =
        usize::try_from(tick_to_sample(START_TICK + DURATION_TICKS, per_tick).expect("tick 换算"))
            .expect("帧号放得进 usize");
    // ⭐ `#302`：默认档限制器关 ⇒ 全链路 **0 帧延迟**（旧的 33 是限制器前瞻环）。
    // 时序前移 33 帧是**修对**：监听与导出在样本层对齐。
    let latency = 0usize;

    // 斜坡：素材第 n 帧 = (n + 1) × 1e-6（＋1 让**第一帧**就非零 ⇒ "首个非零帧"
    // 与"起声帧"重合；峰值 96 001 × 1e-6 = 0.096，远低于限制器阈值）。
    let asset_frames = 2 * rate as usize;
    let ramp: Vec<f32> = (0..asset_frames)
        .map(|frame| ((frame as f32) + 1.0) * RAMP_SCALE)
        .collect();
    let samples: Arc<[f32]> = Arc::from(ramp.into_boxed_slice());
    let pcm = AudioPcm::new(samples, 1, rate).expect("单声道斜坡 PCM 必须合法");
    let snapshot = snapshot_with_pcm(&fixture.project, pcm);
    let rendered = render_snapshot(snapshot, QUANTA);
    let frames = rendered.frames();
    assert!(
        frames > end + latency,
        "窗口必须覆盖摆放终点之后的一段（帧数 {frames} vs 终点 {end}）"
    );

    let first = first_nonzero(&rendered);
    let before_ok = silent_in(&rendered, 0, start + latency);
    let middle_from = start + latency;
    let middle_to = end + latency;
    let middle_nonzero = rendered.left[middle_from..middle_to]
        .iter()
        .filter(|sample| **sample != 0.0)
        .count();
    let middle_len = middle_to - middle_from;
    let after_ok = silent_in(&rendered, middle_to, frames);

    // 精确值：输出第 `position` 帧（绝对）必须等于"素材第 (position − latency − start) 帧"
    // 乘居中声相增益。三点：起声第一帧 / 中段随机一帧 / 收声最后一帧。
    let positions = [middle_from, middle_from + 1_000, middle_to - 1];
    let mut value_report = Vec::new();
    let mut value_ok = true;
    for position in positions {
        let asset_frame = position - latency - start;
        // `#300`：中心声相是恒等 ⇒ 只乘 1.0（修前这里还乘 `1/√2`）。
        let expected = ((asset_frame as f32) + 1.0) * RAMP_SCALE;
        let actual = rendered.left[position];
        let error = (actual - expected).abs();
        value_report.push(format!(
            "帧 {position} ⇒ 素材帧 {asset_frame}: 实测 {actual:.9} 期望 {expected:.9} 误差 {error:e}"
        ));
        if error > 1e-7 {
            value_ok = false;
        }
    }

    println!(
        "[#135/A3] 起点 tick={START_TICK} ⇒ 帧 {start}；终点 {end}；限制器前瞻 {latency} 帧；\
         首个非零帧={first:?}（期望 {}）；前段 [0,{}) 全零={before_ok}；\
         中段 [{middle_from},{middle_to}) 非零 {middle_nonzero}/{middle_len}；\
         后段 [{middle_to},{frames}) 全零={after_ok}；精确值：{}",
        start + latency,
        start + latency,
        value_report.join("；")
    );
    assert_eq!(
        first,
        Some(start + latency),
        "音频必须恰好在摆放起点开始（#302 默认档无限制器延迟）"
    );
    assert!(before_ok, "起点之前必须逐位静音");
    assert_eq!(
        middle_nonzero, middle_len,
        "摆放区间内必须**每一个**样本都非零（斜坡从 +1 起，没有零帧）"
    );
    assert!(after_ok, "摆放终点之后必须逐位静音（硬切）");
    assert!(
        value_ok,
        "素材帧必须 = 绝对帧 − span.start（斜坡值就是位置本身；常量素材会掩盖这条）"
    );
}

/// A4：素材比摆放**短** ⇒ 硬切（余下是静音尾巴），⛔ 不越界读。
#[test]
fn a_short_asset_leaves_a_silent_tail() {
    const START_TICK: u64 = 480;
    const DURATION_TICKS: u64 = 960;
    let fixture = audio_clip_project_at(START_TICK, DURATION_TICKS, 0.0);
    let rate = fixture_rate(&fixture.project);
    let per_tick = frames_per_tick(&fixture.project);
    let start = usize::try_from(tick_to_sample(START_TICK, per_tick).expect("tick 换算"))
        .expect("帧号放得进 usize");
    let end =
        usize::try_from(tick_to_sample(START_TICK + DURATION_TICKS, per_tick).expect("tick 换算"))
            .expect("帧号放得进 usize");
    // ⭐ `#302`：默认档限制器关 ⇒ 全链路 **0 帧延迟**（旧的 33 是限制器前瞻环）。
    // 时序前移 33 帧是**修对**：监听与导出在样本层对齐。
    let latency = 0usize;
    // 素材只有 5 000 帧 ⇒ 摆放的 24 000 帧里后 19 000 帧必须是静音尾巴。
    let asset_frames = 5_000usize;
    let snapshot = snapshot_with_pcm(&fixture.project, mono_pcm(0.5, asset_frames, rate));
    let rendered = render_snapshot(snapshot, 400);

    let asset_end = start + latency + asset_frames;
    let tail_nonzero = rendered.left[asset_end..end + latency]
        .iter()
        .filter(|sample| **sample != 0.0)
        .count();
    let sounding = rendered.left[start + latency..asset_end]
        .iter()
        .filter(|sample| **sample != 0.0)
        .count();
    println!(
        "[#135/A4] 素材 {asset_frames} 帧 / 摆放 {} 帧：起声段非零={sounding}/{asset_frames}；\
         尾巴 [{asset_end},{}) 非零={tail_nonzero}",
        end - start,
        end + latency
    );
    assert_eq!(sounding, asset_frames, "素材区间内必须每一帧都出声");
    assert_eq!(tail_nonzero, 0, "素材用完之后必须是静音尾巴（⛔ 越界读）");
}

/// A5：立体声 `(L+R)/2` 与单声道**逐位相同**（同一份折单声道口径）。
#[test]
fn stereo_assets_are_downmixed_and_mono_is_read_directly() {
    let fixture = audio_clip_project();
    let rate = fixture_rate(&fixture.project);
    let mono = {
        let snapshot = snapshot_with_pcm(&fixture.project, mono_pcm(0.5, 6_000, rate));
        render_snapshot(snapshot, 48)
    };
    // 立体声：L = 0.25、R = 0.75 ⇒ (L + R) / 2 = 0.5，与上面那份单声道**同一个值**。
    let interleaved: Vec<f32> = (0..6_000).flat_map(|_| [0.25f32, 0.75f32]).collect();
    let samples: Arc<[f32]> = Arc::from(interleaved.into_boxed_slice());
    let stereo = AudioPcm::new(samples, 2, rate).expect("立体声 PCM 必须合法");
    let stereo = {
        let snapshot = snapshot_with_pcm(&fixture.project, stereo);
        render_snapshot(snapshot, 48)
    };
    println!(
        "[#135/A5] 单声道指纹=0x{:016x}；立体声折单声道指纹=0x{:016x}；峰值 {} / {}",
        mono.fingerprint(),
        stereo.fingerprint(),
        mono.peak(),
        stereo.peak()
    );
    assert!(mono.peak() > 0.0, "单声道臂必须出声");
    assert_eq!(
        mono.fingerprint(),
        stereo.fingerprint(),
        "立体声 (L+R)/2 必须与等值单声道**逐位**相同"
    );
}

/// A6：`gain_db` 真的进样本路径：−6 dB 臂的峰值 = 0 dB 臂峰值 × `db_to_gain(-6)`。
#[test]
fn the_clip_gain_is_projected_into_the_sample_path() {
    let fixture = audio_clip_project();
    let rate = fixture_rate(&fixture.project);
    let unity = {
        let snapshot = snapshot_with_pcm(&fixture.project, mono_pcm(0.5, 6_000, rate));
        render_snapshot(snapshot, 48)
    };
    let quiet_fixture = audio_clip_project_at(0, 960, -6.0);
    let quiet = {
        let snapshot = snapshot_with_pcm(&quiet_fixture.project, mono_pcm(0.5, 6_000, rate));
        render_snapshot(snapshot, 48)
    };
    let expected_ratio = yeban_dsp::math::db_to_gain(-6.0);
    let ratio = quiet.peak() / unity.peak();
    println!(
        "[#135/A6] 0 dB 峰值={:.9}；−6 dB 峰值={:.9}；比值={ratio:.9}（期望 {expected_ratio:.9}）",
        unity.peak(),
        quiet.peak()
    );
    assert!(unity.peak() > 0.0, "0 dB 臂必须出声");
    assert!(
        (ratio - expected_ratio).abs() < 1e-6,
        "片段增益必须真的进样本路径（比值 {ratio} vs {expected_ratio}）"
    );
}

/// A7：注入期**拒绝**非法形状（⛔ 不静默、⛔ 不改标签、⛔ 不丢帧）。
#[test]
fn pcm_injection_rejects_unsupported_shapes() {
    let fixture = audio_clip_project();
    let rate = fixture_rate(&fixture.project);
    let snapshot = || EngineSnapshot::from_project(&fixture.project, 1).expect("快照");

    // ① 采样率不符 ⇒ `SampleRateMismatch`（引擎不重采样）。
    let wrong_rate = mono_pcm(0.5, 128, rate + 1);
    let error = snapshot()
        .with_audio_asset(&audio_clip_asset(), wrong_rate)
        .expect_err("采样率不符必须被拒绝");
    assert_eq!(
        error,
        AudioClipError::SampleRateMismatch {
            pcm_rate: rate + 1,
            snapshot_rate: rate,
        }
    );

    // ② 声道数 3 ⇒ `UnsupportedChannels`。
    let samples: Arc<[f32]> = Arc::from(vec![0.5f32; 30].into_boxed_slice());
    assert_eq!(
        AudioPcm::new(samples, 3, rate).expect_err("3 声道必须被拒绝"),
        AudioClipError::UnsupportedChannels(3)
    );

    // ③ 半帧（3 个样本 / 2 声道）⇒ `RaggedInterleaved`。
    let samples: Arc<[f32]> = Arc::from(vec![0.5f32; 3].into_boxed_slice());
    assert_eq!(
        AudioPcm::new(samples, 2, rate).expect_err("半帧必须被拒绝"),
        AudioClipError::RaggedInterleaved {
            samples: 3,
            channels: 2,
        }
    );

    // ④ 合法的 1 / 2 声道仍然被接受（⛔ 一刀切）。
    let mono: Arc<[f32]> = Arc::from(vec![0.5f32; 8].into_boxed_slice());
    assert!(AudioPcm::new(mono, 1, rate).is_ok());
    let stereo: Arc<[f32]> = Arc::from(vec![0.5f32; 8].into_boxed_slice());
    assert!(AudioPcm::new(stereo, 2, rate).is_ok());
    println!(
        "[#135/A7] 拒绝：采样率（{rate}+1）/ 3 声道 / 半帧；接受：1 声道、2 声道（采样率 {rate}）"
    );
}

/// A8（**形状/字节数**）：本单的代价逐项现读（判据 5 的算术自证）。
///
/// | 对象 | 字节 | 组成 |
/// | :--- | ---: | :--- |
/// | [`AudioPcm`] | 32 | `Arc<[f32]>` 16（胖指针）＋ `frames` 8 ＋ `sample_rate` 4 ＋ `channels` 2 ＋ 2 填充 |
/// | [`AudioClipSpan`] | 64 | `start` 8 ＋ `end` 8 ＋ `gain` 4 ＋ `AssetHash`(`String`) 24 ＋ `Option<Arc<AudioPcm>>` 8 ＋ `source_offset` 8（`#249`）＋ 4 填充 |
/// | [`AudioClipSchedule`] | 32 | 一个 `Vec<AudioClipSpan>` 的头部（24）＋ `placements` 计数 8（`#249`） |
/// | `EngineSnapshot` | 4528 | 现读（`Tests/sfz_instrument.rs` 的 S9 打印）；本单 ＋32 B（新增一个 `BTreeMap` 字段 24 B ＋ 8 B 布局填充） |
/// | `EngineRuntime` | 180144 | **＋0 B**（⛔ 没有游标表；余量 196608 − 180144 = 16464 B 不动） |
///
/// ⚠ PCM 本体按 `4 B / 帧`计（`f32` 交织），**每个素材只存一份**：多个摆放共享
/// 同一个 `Arc<AudioPcm>` ⇒ span 里只多一个 8 B 细指针。
#[test]
fn the_shape_costs_are_read_not_hand_written() {
    let pcm = std::mem::size_of::<AudioPcm>();
    let span = std::mem::size_of::<AudioClipSpan>();
    let schedule = std::mem::size_of::<AudioClipSchedule>();
    let snapshot = std::mem::size_of::<EngineSnapshot>();
    let runtime = std::mem::size_of::<yeban_engine::rt::EngineRuntime>();
    println!(
        "[#135/A8] size_of: AudioPcm={pcm} AudioClipSpan={span} AudioClipSchedule={schedule} \
         EngineSnapshot={snapshot} EngineRuntime={runtime} \
         （运行时门槛 196608；余量 = {}）",
        196_608 - runtime
    );
    assert_eq!(
        pcm, 32,
        "AudioPcm = Arc<[f32]> 16 + frames 8 + sample_rate 4 + channels 2 + 2 填充"
    );
    assert_eq!(
        span, 64,
        "AudioClipSpan = start 8 + end 8 + gain 4 + AssetHash 24 + Option<Arc> 8 +          source_offset 8 + 4 填充"
    );
    assert_eq!(
        schedule, 32,
        "AudioClipSchedule = Vec 头部 24 + placements 计数 8"
    );
    assert_eq!(
        runtime, 186_736,
        "EngineRuntime 的现读值（`#261` 之后 186720 = `#173` 的 181936 ＋ 4784 B 的\
         逐边发送量表/平滑器/重放副本；⭐ **S1 第一片**再加 **16 B** = 两个 `u64` 事件计数\
         `strip_output_gain_events` / `strip_output_gain_value_rejects`，⛔ 不新增任何表\
         ——落点复用既有的 `armed_strips`；本判据只要求音频片段**不再**加字节 ——\
         见 `tests/bus_insert.rs` 的 B5）"
    );
    assert!(
        (4_496..=4_608).contains(&snapshot),
        "EngineSnapshot 只许为一个 BTreeMap 字段增长（实测 {snapshot} B；门槛不是本单的约束，但⛔ 不许长出重数据）"
    );
}
