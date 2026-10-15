//! 判据（任务单 #71；闭合 `#69` 在 `render_core.rs` 模块文档里登记的既有边界）：
//! app 侧的**最小音源**（[`NoteToneSource`]，只在 `use_engine_source = false` 时用）
//! 必须吃**引擎层的轨增益**（推子 / 静音 / 独奏），且必须与引擎用**同一个算子**。
//!
//! ## 背景（为什么这是"补边界"而不是"新能力"）
//!
//! [`NoteToneSource`] 与 [`yeban_app::sfz_load::SfzTrackSource`] 是**同形的两条旁路**：
//! 都**整个替换**引擎音源，都由 `render_core` 逐轨构造 ⇒ 都拿不到投影期已乘好的那一份
//! （`NoteSchedule::track_gain`）。`#69` 已把 SFZ 那条接上；本单接最小音源这条
//! —— 两者都用引擎的**同一个** `yeban_engine::synth::track_gain_scale(...)`。
//!
//! ⚠ **⛔ 不改产品路径**：`--export-wav` / `record_wav` 都给 `use_engine_source = true`
//! ⇒ 引擎乐器那条路的字节**一位没变**。
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `peak(arm)` | 一条导出的**交错 f32 样本**（左声道抽取）的绝对值最大值 | 线性幅度（`[-1, 1]`） |
//! | `ratio` | 同工程、**只差轨推子**（0 dB 对 −6 dB）的峰值比 | 无量纲倍数 |
//! | `expected` | [`yeban_engine::synth::track_gain`]`(−6.0)`（**引擎自己的**函数） | 无量纲倍数 |
//!
//! ## 在**写盘前**还是落盘后量
//!
//! 两条都量：
//!
//! * **写盘前**：`render_core::render_project_f32_with_assets` 的交错 f32 样本
//!   （⛔ 无量化、⛔ 无抖动）⇒ 1e-5 的那条腿；
//! * **落盘后**：用 **32f 位深**导出（`quantize` 对 `Float32` 是**透传**、⛔ 不抖动）再回读
//!   ⇒ 钉住"写盘这一步不改变比值"。
//!
//! ⚠ 24-bit 整数落盘会走 **TPDF 抖动** ⇒ 静音会变成 ±1 LSB，因此 24-bit 那条腿只断言
//! "≤ 1 LSB（抖动本底）"；"逐位 0"在**写盘前**样本与 32f 回读上断言。

use std::path::{Path, PathBuf};

use yeban_app::export_wav::{WavExportOptions, export_wav_with_sfz_file_and_assets};
use yeban_app::render_core::{self, RenderProjectOutput};
use yeban_engine::synth::track_gain;
use yeban_model::{EntityId, TrackKind, TrackV3, YebanProjectV1};
use yeban_render::dither::{BitDepth, PcmBuffer};
use yeban_render::wav::read_plain_wav;

/// 判据用的推子档（dB）：−6 dB ⇒ 期望倍数 `track_gain(−6) ≈ 0.501187`。
const FADER_DB: f32 = -6.0;

/// 第二档（dB）：一个标量巧合就能过的判据没有牙 ⇒ 再加一档不同倍数的推子。
const SECOND_FADER_DB: f32 = -12.0;

/// 24-bit 满量程（归一化分母；与 `sfz_track_gain.rs` 同口径）。
const I24_SCALE: f32 = 8_388_608.0;

/// 32f 位深让"落盘"这一步**不引入抖动**（`quantize` 对 `Float32` 是透传）。
const NO_DITHER: BitDepth = BitDepth::Float32;

/// 相对误差（判据用的唯一定义）。
fn rel(a: f32, b: f32) -> f32 {
    (a - b).abs() / b.abs()
}

/// 峰值（绝对值最大；单位：线性幅度）。
fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0_f32, |acc, sample| acc.max(sample.abs()))
}

/// 临时目录（形态照 `sfz_track_gain.rs` 的 `scratch`）。
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-tone-gain-{tag}-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 工程里那条**带 MIDI 音符**的非主总线轨。
///
/// ⚠ `filled_project` 的另一条非主总线轨是**音频**摆放（`ClipContent::Audio`）而本判据
/// 的资产池是空的 ⇒ 它被**如实跳过**（`audio_clips_rendered == 0` 有断言钉住）；
/// `Aux Reverb` 到不了 master ⇒ 被渲染计划剪枝。⇒ **母带 = 这条 MIDI 轨**，
/// 于是"整段导出的峰值比"就是这条轨轨增益的标量倍。
fn midi_track_of(project: &YebanProjectV1) -> EntityId {
    *project
        .tracks
        .iter()
        .find(|(id, track)| {
            **id != project.master_bus_track_id
                && track.clips.values().any(|placement| {
                    project
                        .clip_pool
                        .get(&placement.clip_id)
                        .is_some_and(|entry| entry.content.notes().is_some())
                })
        })
        .map(|(id, _)| id)
        .expect("filled_project 必须有一条带 MIDI 音符的非主总线轨")
}

/// 轨参数夹具：`volume_db` ＋ `mute` ＋ "**另一条轨在独奏**"（可闻门的三个自变量）。
///
/// ⚠ 两臂之间**只差** `volume_db` 这一个字段（同一份 `filled_project`、同一批音符）；
/// 工程自带的 `TrackVolume` 自动化泳道**显式清空** ⇒ "只差轨推子"是字面事实
/// （与 `sfz_track_gain.rs` 的 `fader_fixture` 同款处置）。
fn fader_fixture(volume_db: f32, mute: bool, solo_elsewhere: bool) -> YebanProjectV1 {
    let mut project = yeban_model::samples::filled_project();
    let lead = midi_track_of(&project);
    {
        let track = project
            .tracks
            .get_mut(&lead)
            .expect("上一步找到的轨必须还在");
        track.volume_db = volume_db;
        track.mute = mute;
        track.automation_lanes.clear();
    }
    if solo_elsewhere {
        let other = EntityId::new();
        project.tracks.insert(
            other,
            TrackV3 {
                id: other,
                name: "Solo".to_owned(),
                kind: TrackKind::Midi,
                solo: true,
                ..TrackV3::default()
            },
        );
        // 进轨道表（⇒ `any_solo == true`）＋ 进图节点；⛔ 不给它出边 ⇒ 计划把它剪枝。
        project.routing_graph.nodes.push(other);
    }
    project
}

/// 最小音源档的选项（`use_engine_source = false`）。
fn tone_options(depth: BitDepth) -> WavExportOptions {
    WavExportOptions {
        use_engine_source: false,
        bit_depth: depth,
        ..WavExportOptions::default()
    }
}

/// 写盘**前**的渲染读数（⛔ 无量化、⛔ 无抖动）。
fn pre_write(project: &YebanProjectV1) -> RenderProjectOutput {
    render_core::render_project_f32_with_assets(project, tone_options(NO_DITHER), None, &[])
        .expect("最小音源离线渲染必须成功")
}

/// 导出并回读**左声道**（单位：归一化线性幅度）。
fn export_and_read(project: &YebanProjectV1, path: &Path, depth: BitDepth) -> Vec<f32> {
    let report = export_wav_with_sfz_file_and_assets(project, path, tone_options(depth), None, &[])
        .expect("最小音源导出必须成功");
    assert!(
        report.source.contains("最小音源"),
        "报告必须明说走了最小音源: {}",
        report.source
    );
    assert_eq!(
        report.audio_clips_rendered, 0,
        "夹具的资产池为空 ⇒ 音频摆放必须被**如实跳过**（否则母带不止一条轨的声音）"
    );
    let (format, buffer) = read_plain_wav(path).expect("导出必须可回读");
    let channels = usize::from(format.channels.max(1));
    match buffer {
        PcmBuffer::Float32(values) => values.iter().step_by(channels).copied().collect(),
        PcmBuffer::Int24(values) => values
            .iter()
            .step_by(channels)
            .map(|value| (*value as f32) / I24_SCALE)
            .collect(),
        other => panic!("本判据只导出 32f / 24-bit：{other:?}"),
    }
}

/// ⭐ 判据一（本单新增；台账 `R713`／`#69` 登记的边界）：**同一工程、只差轨推子 ⇒
/// 最小音源的峰值比 ＝ 引擎的期望比**；静音 / 别轨独奏 ⇒ **逐位 0**。
///
/// # 怎么变红
///
/// 去掉 `render_core::render_project_f32_with_assets` 里那条
/// `NoteToneSource::with_track_gain(...)`（或 `export_wav.rs` 里那次乘法）⇒
/// 推子档的比值变回 `1.0`（期望 `0.501187…` / `0.251188…`），静音 / 独奏档**不再全零**。
#[test]
fn the_tone_source_scales_with_the_track_fader_and_gates_on_mute_and_solo() {
    let dir = scratch("fader");
    let at_0db = fader_fixture(0.0, false, false);
    let muted = fader_fixture(0.0, true, false);
    let solo_elsewhere = fader_fixture(0.0, false, true);

    // ① 推子：**写盘前**样本（⛔ 无量化／抖动）。
    let pre_0 = pre_write(&at_0db);
    assert_eq!(
        pre_0.audio_clips_rendered, 0,
        "音频摆放必须被如实跳过 ⇒ 母带只有 MIDI 轨"
    );
    assert!(
        peak(&pre_0.samples) > 0.0,
        "0 dB 臂必须真的出声（否则比值是 0/0 那种空话）"
    );
    // ② 推子：**落盘后**回读（32f 透传 ⇒ 不抖动）。
    let wav_0 = export_and_read(&at_0db, &dir.join("fader-0db.wav"), NO_DITHER);
    assert!(peak(&wav_0) > 0.0, "0 dB 臂落盘后必须真的出声");

    let mut rows = Vec::new();
    for db in [FADER_DB, SECOND_FADER_DB] {
        let arm = fader_fixture(db, false, false);
        let pre = pre_write(&arm);
        let expected = track_gain(db);
        let pre_ratio = peak(&pre.samples) / peak(&pre_0.samples);

        let tag = format!("fader-{}.wav", db.abs() as i32);
        let wav = export_and_read(&arm, &dir.join(&tag), NO_DITHER);
        let wav_ratio = peak(&wav) / peak(&wav_0);

        rows.push(format!(
            "{db} dB: 写盘前 {:.9}/{:.9} ⇒ {pre_ratio:.9}；落盘(32f) ⇒ {wav_ratio:.9}；\
             期望 track_gain({db}) = {expected:.9}",
            peak(&pre.samples),
            peak(&pre_0.samples),
        ));
        assert!(
            rel(pre_ratio, expected) <= 1e-5,
            "⭐ 写盘前的峰值比必须 ＝ `track_gain({db} dB)`：实得 {pre_ratio} vs {expected}"
        );
        assert!(
            rel(wav_ratio, expected) <= 1e-5,
            "⭐ 导出 WAV 回读的峰值比必须 ＝ `track_gain({db} dB)`：实得 {wav_ratio} vs {expected}"
        );
        assert!(
            rel(wav_ratio, pre_ratio) <= 1e-5,
            "⭐ 落盘这一步不许改变比值（{wav_ratio} vs {pre_ratio}）"
        );
    }
    println!(
        "[app-tone-gain/fader] 0 dB 峰值（写盘前）= {:.9}；{}",
        peak(&pre_0.samples),
        rows.join("；")
    );

    // ③ 静音 / 别轨独奏：写盘前样本的**逐位**读数。
    let mute_pre = pre_write(&muted);
    let solo_pre = pre_write(&solo_elsewhere);
    let nonzero = |samples: &[f32]| samples.iter().filter(|sample| **sample != 0.0).count();
    println!(
        "[app-tone-gain/gate] 静音(写盘前) 非零样本 = {}；别轨独奏(写盘前) 非零样本 = {}",
        nonzero(&mute_pre.samples),
        nonzero(&solo_pre.samples),
    );
    assert!(
        mute_pre.samples.iter().all(|sample| sample.to_bits() == 0),
        "⭐ 静音轨 ⇒ 写盘前样本必须**逐位** 0（`to_bits() == 0`），实得峰值 {}",
        peak(&mute_pre.samples)
    );
    assert!(
        solo_pre.samples.iter().all(|sample| sample.to_bits() == 0),
        "⭐ 别轨独奏 ⇒ 写盘前样本必须**逐位** 0（`to_bits() == 0`），实得峰值 {}",
        peak(&solo_pre.samples)
    );

    // ④ 32f 落盘（透传）上同样逐位 0 ⇒ "写盘这一步也没有偷偷加东西"。
    for (tag, project) in [("mute-32f.wav", &muted), ("solo-32f.wav", &solo_elsewhere)] {
        let read_back = export_and_read(project, &dir.join(tag), NO_DITHER);
        assert!(
            read_back.iter().all(|sample| sample.to_bits() == 0),
            "⭐ {tag}：32f 回读的样本必须**逐位** 0（实得峰值 {}）",
            peak(&read_back)
        );
    }

    // ⑤ ⚠ 24-bit 落盘会**按设计**加 TPDF 抖动 ⇒ 纯零信号变成 ±1 LSB；这里只钉"没有信号"。
    let mute_i24 = export_and_read(&muted, &dir.join("mute-24bit.wav"), BitDepth::Int24);
    let dither_floor = 1.0 / I24_SCALE;
    assert!(
        mute_i24.iter().all(|sample| sample.abs() <= dither_floor),
        "⚠ 24-bit 静音导出只允许剩抖动本底（≤ 1 LSB = {dither_floor}），实得峰值 {}",
        peak(&mute_i24)
    );

    std::fs::remove_dir_all(&dir).ok();
}
