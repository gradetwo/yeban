//! 判据（任务单 #69；闭合台账 `R713` 登记的边界）：app 侧 **SFZ 离线渲染**必须吃
//! **引擎层的轨增益**（推子 / 静音 / 独奏），且必须与引擎用**同一个算子**。
//!
//! ## 背景（为什么这是"补边界"而不是"新能力"）
//!
//! 归属裁决 `R711`：**乐器层**拥有乐器内增益（`volume_gain` / `velocity_gain[128]` /
//! `key_crossfade` / `velocity_fade_*` / `amp_veltrack`），**引擎层**拥有轨增益
//! （推子 / 静音 / 独奏）⇒ 两者**相乘、⛔ 不重复**。`mod-engine` 已把引擎侧做完
//! （`NoteSchedule::track_gain`）；而 app 的 `--export-wav --sfz` 走
//! [`yeban_app::sfz_load::SfzTrackSource`] **旁路**、不经引擎 SFZ 分支 ⇒ 原先吃不到。
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `peak(arm)` | 一条导出的**交错 f32 样本**（左声道抽取）的绝对值最大值 | 线性幅度（`[-1, 1]`） |
//! | `ratio` | 同工程、同 SFZ、**只差轨推子**（0 dB 对 −6 dB）的峰值比 | 无量纲倍数 |
//! | `expected` | [`yeban_engine::synth::track_gain`]`(−6.0)`（**引擎自己的**函数） | 无量纲倍数 |
//!
//! ## 为什么写盘前也要量一遍
//!
//! 24-bit 整数落盘会走 **TPDF 抖动**（`crates/yeban-render/src/dither.rs`）⇒ 静音会变成
//! ±1 LSB、小峰值上量化噪声会盖过 `1e-5`。因此：
//!
//! * **1e-5 的那条腿量"写盘前"的交错 f32 样本**（`render_core` 的输出，⛔ 无量化、⛔ 无抖动）；
//! * 另有一条腿用 **32f 位深**（`quantize` 对 `Float32` 是**透传**，⛔ 不抖动）落盘再回读，
//!   钉住"落盘这一步不改变比值"；
//! * 静音的"逐位 0"同样在**写盘前**的样本与 32f 回读上断言；24-bit 那条腿只断言
//!   "≤ 1 LSB（抖动本底）"，因为抖动**按设计**会让纯零信号不全零。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use yeban_app::export_wav::{WavExportOptions, export_wav_with_sfz_file_and_assets};
use yeban_app::render_core::{self, RenderProjectOutput};
use yeban_app::sfz_load::SfzTrackSource;
use yeban_engine::sfz_instrument::{InMemorySample, SfzInstrument};
use yeban_engine::synth::{track_gain, velocity_gain};
use yeban_model::{EntityId, TrackKind, TrackV3, YebanProjectV1};
use yeban_render::dither::{BitDepth, PcmBuffer};
use yeban_render::render::{AudioSource, BlockContext};
use yeban_render::wav::{read_plain_wav, write_plain_wav};

/// 判据用的推子档（dB）：−6 dB ⇒ 期望倍数 `track_gain(−6) ≈ 0.501187`。
const FADER_DB: f32 = -6.0;

/// 24-bit 满量程（归一化分母；与 `read_plain_wav_mono_f32` 同口径）。
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

/// ⚠ 与 [`SfzTrackSource::new`] **同一算式**的那份**共享样本**（内存正弦）：
/// 帧数 = `engine_hz.max(1.0) as u64`、至少 1 帧。
///
/// 判据要用"乐器层自己"重算一遍期望，就必须拿到**逐位相同**的样本输入 ⇒ 这里刻意
/// 复制那一条算式（而不是另取一个长度）。`sfz_load` 若改了那条算式，本判据会红 ——
/// 那时请同步这里，而不是怀疑乘法。
fn shared_sample(engine_hz: f32) -> InMemorySample {
    let frames = usize::try_from(engine_hz.max(1.0) as u64)
        .unwrap_or(48_000)
        .max(1);
    InMemorySample::sine(440.0, engine_hz, frames)
}

/// 临时目录（形态照 `export_wav_audio_clips.rs` 的 `scratch`）。
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-sfz-gain-{tag}-{}-{}",
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
/// 于是"整段导出的峰值比"就是这条轨轨增益的标量倍（这正是判据要的量）。
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
/// ⚠ 两臂之间**只差** `volume_db` 这一个字段（同一份 `filled_project`、同一个 SFZ、
/// 同一批音符）⇒ 峰值比是**标量**比。
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
        // 工程自带的音量自动化泳道**不参与**离线渲染（导出路径不读自动化）；显式清空，
        // 让"只差轨推子"这句话是字面事实而不是"外加一条未经核验的假设"。
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
        // 与引擎侧 S17 的夹具同款：进轨道表（⇒ `any_solo == true`）＋ 进图节点；
        // ⛔ 不给它出边 ⇒ 渲染计划把它剪枝（它不需要音源）。
        project.routing_graph.nodes.push(other);
    }
    project
}

/// 一份"每个工程音高一条区域"的 SFZ（区域不匹配就会是静音 ⇒ 判据会假绿）。
fn sfz_for(project: &YebanProjectV1) -> Arc<SfzInstrument> {
    let mut pitches: Vec<u8> = project
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .flat_map(|notes| notes.values().map(|note| note.pitch))
        .collect();
    pitches.sort_unstable();
    pitches.dedup();
    assert!(!pitches.is_empty(), "夹具必须有音符");
    let text: String = pitches
        .iter()
        .map(|pitch| format!("<region> sample=a.wav key={pitch}\n"))
        .collect();
    Arc::new(
        SfzInstrument::load(text, &yeban_sfz::parser::ParseLimits::default())
            .expect("夹具 SFZ 必须可装载"),
    )
}

/// 写盘**前**的渲染读数（`render_core` 的输出：⛔ 无量化、⛔ 无抖动）。
fn pre_write(
    project: &YebanProjectV1,
    instrument: &Arc<SfzInstrument>,
    depth: BitDepth,
) -> RenderProjectOutput {
    let options = WavExportOptions {
        bit_depth: depth,
        ..WavExportOptions::default()
    };
    render_core::render_project_f32_with_assets(
        project,
        options,
        Some((Arc::clone(instrument), None)),
        &[],
    )
    .expect("SFZ 离线渲染必须成功")
}

/// 导出并回读**左声道**（单位：归一化线性幅度）。
fn export_and_read(
    project: &YebanProjectV1,
    instrument: &Arc<SfzInstrument>,
    path: &Path,
    depth: BitDepth,
) -> Vec<f32> {
    let options = WavExportOptions {
        bit_depth: depth,
        ..WavExportOptions::default()
    };
    let report = export_wav_with_sfz_file_and_assets(
        project,
        path,
        options,
        Some((Arc::clone(instrument), None)),
        &[],
    )
    .expect("SFZ 导出必须成功");
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

/// ⭐ 判据一（本单新增；台账 `R713`）：**同一工程、同一 SFZ、只差轨推子 ⇒
/// 导出 WAV 的峰值比 ＝ 引擎的期望比**；静音 / 别轨独奏 ⇒ **逐位 0**。
///
/// # 怎么变红
///
/// 去掉 `render_core::render_project_f32_with_assets` 里那条
/// `SfzTrackSource::with_track_gain(...)`（或 `sfz_load` 里那次乘法）⇒
/// 推子档的比值变回 `1.0`（期望 `0.501187…`），静音 / 独奏档**不再全零**。
#[test]
fn a_sfz_export_scales_with_the_track_fader_and_gates_on_mute_and_solo() {
    let dir = scratch("fader");
    let at_0db = fader_fixture(0.0, false, false);
    let at_6db = fader_fixture(FADER_DB, false, false);
    let muted = fader_fixture(0.0, true, false);
    let solo_elsewhere = fader_fixture(0.0, false, true);
    let instrument = sfz_for(&at_0db);

    // ① 推子：两臂的**写盘前**样本（⛔ 无量化／抖动）。
    let pre_0 = pre_write(&at_0db, &instrument, NO_DITHER);
    let pre_6 = pre_write(&at_6db, &instrument, NO_DITHER);
    let expected = track_gain(FADER_DB);
    let pre_ratio = peak(&pre_6.samples) / peak(&pre_0.samples);

    // ② 推子：**落盘**（32f 透传 ⇒ 不抖动）再回读。
    let wav_0 = export_and_read(&at_0db, &instrument, &dir.join("fader-0db.wav"), NO_DITHER);
    let wav_6 = export_and_read(&at_6db, &instrument, &dir.join("fader-6db.wav"), NO_DITHER);
    let wav_ratio = peak(&wav_6) / peak(&wav_0);

    // ③ 静音 / 别轨独奏：写盘前样本的**逐位**读数。
    let mute_pre = pre_write(&muted, &instrument, NO_DITHER);
    let solo_pre = pre_write(&solo_elsewhere, &instrument, NO_DITHER);
    let nonzero = |samples: &[f32]| samples.iter().filter(|sample| **sample != 0.0).count();

    // ④ 静音：**24-bit 默认档**（会抖动）只断言"≤ 1 LSB 抖动本底"。
    let mute_i24 = export_and_read(
        &muted,
        &instrument,
        &dir.join("mute-24bit.wav"),
        BitDepth::Int24,
    );
    let dither_floor = 1.0 / I24_SCALE;

    println!(
        "[app-sfz-gain/fader] 峰值：写盘前 0 dB = {:.9} / −6 dB = {:.9} ⇒ 比值 {pre_ratio:.9}；\
         落盘(32f) 0 dB = {:.9} / −6 dB = {:.9} ⇒ 比值 {wav_ratio:.9}；\
         期望 track_gain(−6 dB) = {expected:.9}",
        peak(&pre_0.samples),
        peak(&pre_6.samples),
        peak(&wav_0),
        peak(&wav_6),
    );
    println!(
        "[app-sfz-gain/gate] 静音(写盘前) 非零样本 = {}；别轨独奏(写盘前) 非零样本 = {}；\
         静音(24-bit 落盘) 最大 |样本| = {:.9}（抖动本底 {dither_floor:.9}）",
        nonzero(&mute_pre.samples),
        nonzero(&solo_pre.samples),
        peak(&mute_i24),
    );

    // 前置：0 dB / −6 dB 两臂都必须真的出声（否则"比值"是 0/0 那种空话）。
    assert!(
        peak(&pre_0.samples) > 0.0 && peak(&pre_6.samples) > 0.0,
        "两臂都必须出声（0 dB = {} / −6 dB = {}）",
        peak(&pre_0.samples),
        peak(&pre_6.samples)
    );
    assert!(
        peak(&wav_0) > 0.0 && peak(&wav_6) > 0.0,
        "落盘两臂都必须出声"
    );

    // ⭐ 推子（写盘前，1e-5）：峰值比 ＝ 引擎的 `track_gain(−6 dB)`。
    assert!(
        rel(pre_ratio, expected) <= 1e-5,
        "⭐ 写盘前的峰值比必须 ＝ `track_gain(−6 dB)`：实得 {pre_ratio} vs {expected}"
    );
    // ⭐ 推子（落盘后，1e-5）：32f 透传 ⇒ 比值不许因写盘而漂移。
    assert!(
        rel(wav_ratio, expected) <= 1e-5,
        "⭐ 导出 WAV 回读的峰值比必须 ＝ `track_gain(−6 dB)`：实得 {wav_ratio} vs {expected}"
    );
    assert!(
        rel(wav_ratio, pre_ratio) <= 1e-5,
        "⭐ 落盘这一步不许改变比值（{wav_ratio} vs {pre_ratio}）"
    );

    // ⭐ 静音 / 别轨独奏 ⇒ 样本**逐位**为 `+0.0`（⛔ 不是"很小"）。
    assert!(
        !mute_pre.samples.is_empty(),
        "夹具必须非空（否则下面那条 `.all(..)` 恒真）"
    );
    assert!(
        mute_pre.samples.iter().all(|sample| sample.to_bits() == 0),
        "⭐ 静音轨 ⇒ 写盘前样本必须**逐位** 0（`to_bits() == 0`），实得峰值 {}",
        peak(&mute_pre.samples)
    );
    assert!(
        !solo_pre.samples.is_empty(),
        "夹具必须非空（否则下面那条 `.all(..)` 恒真）"
    );
    assert!(
        solo_pre.samples.iter().all(|sample| sample.to_bits() == 0),
        "⭐ 别轨独奏 ⇒ 写盘前样本必须**逐位** 0（`to_bits() == 0`），实得峰值 {}",
        peak(&solo_pre.samples)
    );
    // 32f 落盘（透传）上同样逐位 0 ⇒ "写盘这一步也没有偷偷加东西"。
    for (tag, depth) in [("mute-32f.wav", NO_DITHER), ("solo-32f.wav", NO_DITHER)] {
        let path = dir.join(tag);
        let project = if tag.starts_with("mute") {
            &muted
        } else {
            &solo_elsewhere
        };
        let read_back = export_and_read(project, &instrument, &path, depth);
        assert!(
            !read_back.is_empty(),
            "夹具必须非空（否则下面那条 `.all(..)` 恒真）"
        );
        assert!(
            read_back.iter().all(|sample| sample.to_bits() == 0),
            "⭐ {tag}：32f 回读的样本必须**逐位** 0（实得峰值 {}）",
            peak(&read_back)
        );
    }
    // ⚠ 24-bit 落盘会**按设计**加 TPDF 抖动 ⇒ 纯零信号变成 ±1 LSB；这里只钉"没有信号"。
    assert!(
        !mute_i24.is_empty(),
        "夹具必须非空（否则下面那条 `.all(..)` 恒真）"
    );
    assert!(
        mute_i24.iter().all(|sample| sample.abs() <= dither_floor),
        "⚠ 24-bit 静音导出只允许剩抖动本底（≤ 1 LSB = {dither_floor}），实得峰值 {}",
        peak(&mute_i24)
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// ⭐ 判据二（本单新增；`R711` 的**防双重计**腿，参照引擎侧 `S18` 的做法）：
/// ⭐ **乐器层输出 × 轨增益 ＝ app 的输出**（⛔ 不替乐器层再乘一次力度）。
///
/// # 判定量
///
/// 每个力度 `v ∈ {0, 32, 64, 100, 127}`：把**乐器层**（`SfzInstrument::render_note_into`
/// ＋ 内存正弦，与 `SfzTrackSource::new` 用的是**同一份**样本）一次性渲染的结果乘上
/// 轨增益门 `g`，与 [`SfzTrackSource`]（`track_gain = g`）一个块的输出**逐位相同**
/// （单位：线性幅度，单声道，4800 帧）。
///
/// # ⚠ 反向对照：为什么必须用**中段**力度
///
/// `velocity_gain(127) == 1.0` 是**恒等** ⇒ 只测 127 抓不住"多乘一次力度"。
///
/// # 怎么变红
///
/// 在 `sfz_load::SfzTrackSource::ensure_rendered` 里把增益改成
/// `* track_gain * velocity_gain(velocity)` ⇒ `v ∈ {32, 64, 100}` 三条腿当场**逐位不等**。
#[test]
fn the_sfz_source_is_the_instrument_layer_times_the_track_gain_only() {
    let engine_hz = 48_000.0_f32;
    let block = 4_800_usize;
    let track_gain_gate = track_gain(FADER_DB);
    let instrument = Arc::new(
        SfzInstrument::load(
            "<region> sample=a.wav key=60\n".to_owned(),
            &yeban_sfz::parser::ParseLimits::default(),
        )
        .expect("夹具 SFZ 必须可装载"),
    );
    // ⚠ 与 `SfzTrackSource::new` 完全同一份共享样本（帧数 = `engine_hz as u64`）。
    let sample = shared_sample(engine_hz);

    let mut rows = Vec::new();
    for velocity in [0_u8, 32, 64, 100, 127] {
        let notes = vec![(0_u64, block as u64, 60_u8, velocity)];
        let mut source = SfzTrackSource::new(Arc::clone(&instrument), &notes, engine_hz, 1)
            .with_track_gain(track_gain_gate);
        assert_eq!(
            source.track_gain().to_bits(),
            track_gain_gate.to_bits(),
            "`with_track_gain` 必须原样存下注入的门"
        );
        let mut actual = vec![0.0_f32; block];
        source
            .render_block(
                BlockContext {
                    block_index: 0,
                    first_frame: 0,
                    frames: block,
                    channels: 1,
                    sample_rate: 48_000,
                    seed: 1,
                },
                &mut actual,
            )
            .expect("SFZ 源渲染不得失败");

        // 期望：乐器层（⛔ 不含任何轨参数）一次性渲染 **×** 轨增益门。
        let mut instrument_layer = vec![0.0_f32; block];
        let written = instrument
            .render_note_into(&sample, 60, velocity, &mut instrument_layer, engine_hz)
            .expect("乐器层渲染不得失败");
        for value in &mut instrument_layer[..written.min(block)] {
            *value *= track_gain_gate;
        }
        rows.push(format!(
            "v={velocity}: app 峰值 {:.9} / 乐器层×门 峰值 {:.9}",
            peak(&actual),
            peak(&instrument_layer)
        ));
        assert_eq!(
            actual
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            instrument_layer
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            "⭐ 力度 {velocity}：app 的 SFZ 源必须 ＝ 乐器层输出 × 轨增益（逐位相同）\
             ⇒ ⛔ 没有替乐器层再乘一次力度"
        );
    }
    println!("[app-sfz-gain/double-count] 逐位对账 ⇒ {}", rows.join("；"));

    // 反向对照：127 是**恒等** ⇒ 它那条腿对"双重计"没有牙；中段力度才有。
    assert_eq!(velocity_gain(127), 1.0, "127 的力度增益必须是**恒等**");
    assert!(
        velocity_gain(64) > 0.0 && velocity_gain(64) < 1.0,
        "中段力度必须既非 0 也非恒等（实得 {}）",
        velocity_gain(64)
    );
    assert_ne!(
        velocity_gain(64),
        1.0,
        "⭐ 中段力度是恒等的话，本判据就没有鉴别力了"
    );
}

/// 判据二的**对照腿**：`with_track_gain` 缺省（⛔ 没给轨参数）时输出与"乐器层原样"
/// **逐位相同**（`1.0` 是恒等 ⇒ 跳过乘法不改变任何位）。
///
/// ⚠ 这条腿解释"为什么既有判据的字节一个都没变"：凡是**没接轨增益**的调用点
/// （今天只有测试里的手工构造），输出与接线前逐位一致。
#[test]
fn the_default_track_gain_is_a_bit_exact_identity() {
    let engine_hz = 48_000.0_f32;
    let block = 4_800_usize;
    let instrument = Arc::new(
        SfzInstrument::load(
            "<region> sample=a.wav key=60\n".to_owned(),
            &yeban_sfz::parser::ParseLimits::default(),
        )
        .expect("夹具 SFZ 必须可装载"),
    );
    let sample = shared_sample(engine_hz);
    let notes = vec![(0_u64, block as u64, 60_u8, 64_u8)];
    let mut source = SfzTrackSource::new(Arc::clone(&instrument), &notes, engine_hz, 1);
    assert_eq!(
        source.track_gain(),
        1.0,
        "缺省必须是**单位增益**，⛔ 不是 0.0"
    );
    let mut actual = vec![0.0_f32; block];
    source
        .render_block(
            BlockContext {
                block_index: 0,
                first_frame: 0,
                frames: block,
                channels: 1,
                sample_rate: 48_000,
                seed: 1,
            },
            &mut actual,
        )
        .expect("SFZ 源渲染不得失败");
    let mut instrument_layer = vec![0.0_f32; block];
    instrument
        .render_note_into(&sample, 60, 64, &mut instrument_layer, engine_hz)
        .expect("乐器层渲染不得失败");
    assert_eq!(
        actual
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        instrument_layer
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        "⭐ 缺省轨增益 ⇒ 输出必须与乐器层**逐位相同**（`1.0` 恒等）"
    );
    // ⚠ 顺手核一下"写盘"这一步也没有引入别的东西（32f 透传 + 回读逐位一致）。
    let dir = scratch("identity-wav");
    let path = dir.join("identity.wav");
    let format = yeban_render::rf64::PcmFormat::float(1, 48_000, 32);
    write_plain_wav(
        &path,
        &format,
        &PcmBuffer::Float32(instrument_layer.clone()),
    )
    .expect("32f 落盘");
    let (_, buffer) = read_plain_wav(&path).expect("回读");
    let PcmBuffer::Float32(read_back) = buffer else {
        panic!("32f 回读必须是 Float32");
    };
    assert_eq!(
        read_back
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        instrument_layer
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        "⭐ 32f 落盘/回读必须是位级恒等（⇒ 上面那条 1e-5 的腿不是被写盘污染出来的）"
    );
    std::fs::remove_dir_all(&dir).ok();
}
