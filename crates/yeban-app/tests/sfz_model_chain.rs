//! 判据（任务单 `#79`）：**模型 `TrackV3.sampler` ⇒ 控制侧解码 ⇒ 引擎 SFZ 摊平表 ⇒ 导出真的发声**。
//!
//! ## 本单补的是哪一环
//!
//! | 层 | 现状 |
//! | :--- | :--- |
//! | 模型面 | `TrackV3.sampler: Option<SamplerBinding>`（`#76`／`R731`）✓ |
//! | 引擎面 | `EngineSnapshot.sfz_pending: BTreeMap<EntityId, PendingSfz>`（`#78`／`R734`）⇒ 只带"这条轨有一份 SFZ 待摊平"这一事实、**⛔ 不读盘** ✓ |
//! | **控制面** | ⭐ **本单**：`PendingSfz.path` ⇒ 读盘 ⇒ `SfzInstrument::load` ⇒ `referenced_sample_paths` ⇒ 逐个解码 ⇒ `prepare_with_samples` ⇒ `with_sfz` ⇒ `render_track` |
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `nonzero` | 一条导出的**交错 f32 样本**里的非零样本数 | 个（0 ⇒ 全静音） |
//! | `peak` | 同一批样本的绝对值最大值 | 线性幅度（`[-1, 1]`） |
//! | `fnv1a_64` | 同一批样本（逐位卷字节）的 FNV-1a 指纹 | 无符号 64 位整数 |
//! | `missing_sample_paths` / `silent_region_count` | 引擎摊平表的口径读数 | 路径条数 / 区域数 |
//!
//! ## 为什么逐位比而不是"接近"
//!
//! "模型链条真的接上了"这件事只有两种机械判法：① 输出**逐位**等于一份**独立搭出来**的
//! 摊平表渲染（见 `the_model_chain_is_bit_identical_to_the_hand_built_plan`）；② 反向
//! `sampler = None` **逐位**等于接线前实测指纹。只比"接近"会放过"两条路都错得一样"。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yeban_app::export_wav::{WavExportError, WavExportOptions, export_wav};
use yeban_app::render_core::{render_project_f32_with_assets, render_project_f32_with_plans};
use yeban_app::sfz_load::{load_sfz_with_instrument, read_sample_file};
use yeban_engine::sfz_instrument::{InMemorySample, SfzPlan};
use yeban_model::{EntityId, SamplerBinding, YebanProjectV1};
use yeban_render::dither::{BitDepth, PcmBuffer};
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;

/// **上一基线**：`filled_project` ＋ [`WavExportOptions::default`] 在
/// `render_project_f32_with_assets(sampler = None)` 上的 FNV-1a 指纹（逐位；
/// 样本 = `RenderProjectOutput::samples` 的交错 `f32`）。
///
/// ⭐ **任务单 `#233` 重钉（第 298／323 条三件套）**：
/// * 自报旧数：`0xa0a7_0aab_b72b_b369`（`#209` 的读数，见下一段）；
/// * 自报新数：`0xe872_e762_b159_a33d`（本机 `aarch64/macos` 实测，判据输出见下）；
/// * 原因：`#233` 把引擎音频路径上的宿主超越函数换成纯 Rust `libm`
///   （波表 `libm::sinf`、声相 `libm::cosf`/`sinf`、增益 `libm::exp2f`、滤波 `libm::tanf`）
///   ⇒ 同一批样本的最后一位与本机宿主 libm 不再相同 ⇒ 这份指纹**必然**变。
///   本判据钉的是"`sampler = None` 的渲染输出逐位稳定"，⛔ 不是"宿主 libm 不变"。
///
/// ⭐ **任务单 `#209` 的旧记录（保留作历史）**：
/// * 自报旧数：`0x442c_774e_81dc_2bd1`（接线前 `e4f00a37` 实测）；
/// * 自报新数：`0xa0a7_0aab_b72b_b369`（本机 `aarch64/macos` 实测，判据输出见下）；
/// * 原因：离线 `EngineTrackSource::render_block` 补上了 `self.engine.advance(frames)`
///   —— `#207` 实测 `--export-wav` 的多音符只出第一个（`distinct=1`），因为每个块都
///   从位置 0 渲染。修复后多块渲染的播放头真的前进 ⇒ 这份指纹**必然**变。
///   本判据钉的是"`sampler = None` 的渲染输出逐位稳定"，⛔ 不是"播放头永不前进"。
///
/// ⚠ 它是**某台机器上的实测值**，⛔ 不是规范常数（同 `yeban-engine` 的 S21 口径）：
/// [ARCH-DET-001] 的 L1 只在"相同 OS ＋ CPU 架构 ＋ 纯 Rust libm"下承诺逐位一致。
const BASELINE_APP: u64 = 0xe872_e762_b159_a33d;

/// 指纹的**测量平台**。
const MEASURED_PLATFORM: &str = "aarch64/macos";

/// 本机（**编译期事实**）是否就是那个测量平台。
const ON_MEASURED_PLATFORM: bool = cfg!(target_arch = "aarch64") && cfg!(target_os = "macos");

/// `24-bit` 满量程（夹具写入常量样本用；与 `read_plain_wav_mono_f32` 同口径）。
const I24_SCALE: f32 = 8_388_608.0;

/// FNV-1a 64（逐位；把每个 `f32` 的 4 个字节按小端卷进哈希）—— 与引擎侧判据**同一算式**。
fn fnv1a_64(samples: &[f32]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for sample in samples {
        for byte in sample.to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// 峰值（绝对值最大；单位：线性幅度）。
fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0_f32, |acc, sample| acc.max(sample.abs()))
}

/// 非零样本**个数**（单位：个）。
fn nonzero(samples: &[f32]) -> usize {
    samples.iter().filter(|sample| **sample != 0.0).count()
}

/// 同一批样本的位级读数（逐位比的载体）。
fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|sample| sample.to_bits()).collect()
}

/// 临时目录（形态照 `sfz_track_gain.rs` 的 `scratch`）。
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-sfz-chain-{tag}-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 32f 位深让"落盘"这一步**不引入抖动**（`quantize` 对 `Float32` 是透传）。
fn float_options() -> WavExportOptions {
    WavExportOptions {
        bit_depth: BitDepth::Float32,
        ..WavExportOptions::default()
    }
}

/// 写一条**常量** PCM 样本（24-bit 单声道；值 = 满量程的 `value` 倍）。
fn write_const_sample(path: &Path, value: f32) {
    let scale = (I24_SCALE * value) as i32;
    let buffer = PcmBuffer::Int24(vec![scale; 4_800]);
    let format = PcmFormat::integer(1, 48_000, 24);
    write_plain_wav(path, &format, &buffer).expect("写样本夹具");
}

/// 工程里那条**带 MIDI 音符**的非主总线轨（`filled_project` 的 Lead）。
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

/// 那条轨上**出现过的全部音高**（排序 ＋ 去重）。
fn pitches_of(project: &YebanProjectV1, track_id: EntityId) -> Vec<u8> {
    let mut pitches: Vec<u8> = project
        .tracks
        .get(&track_id)
        .expect("轨必须在")
        .clips
        .values()
        .filter_map(|placement| project.clip_pool.get(&placement.clip_id))
        .filter_map(|entry| entry.content.notes())
        .flat_map(|notes| notes.values().map(|note| note.pitch))
        .collect();
    pitches.sort_unstable();
    pitches.dedup();
    assert!(!pitches.is_empty(), "夹具必须真的带音符");
    pitches
}

/// 给那条 MIDI 轨写一个 `sampler = Some(路径)`。
fn bind_sampler(project: &mut YebanProjectV1, track_id: EntityId, path: &Path) {
    project.tracks.get_mut(&track_id).expect("轨必须在").sampler = Some(SamplerBinding {
        path: path.to_string_lossy().into_owned(),
    });
}

/// 写一份"每个工程音高一条区域"的 SFZ（`sample=` 用相对路径 ⇒ 按 SFZ 目录解析）。
///
/// `sample_for` 决定第 `index` 条区域引用哪一份样本（判据用它造"缺样本"的情形）。
fn write_sfz<F>(dir: &Path, name: &str, pitches: &[u8], mut sample_for: F) -> PathBuf
where
    F: FnMut(usize) -> String,
{
    let mut text = String::new();
    for (index, pitch) in pitches.iter().enumerate() {
        text.push_str(&format!(
            "<region> sample={} key={pitch} pitch_keycenter={pitch}\n",
            sample_for(index)
        ));
    }
    let path = dir.join(name);
    std::fs::write(&path, text).expect("写 SFZ 夹具");
    path
}

/// 测试内**显式**走一遍六步，独立搭出摊平表（本单的"离线手搭参照"）。
///
/// ⚠ 它**故意不调用** `decode_sfz_plan`：那样才是**独立**见证 —— 两条路只要有一个字节不同，
/// 逐位判据立刻红。
fn hand_built_plan(sfz_path: &Path, engine_hz: f32) -> SfzPlan {
    let (_load, instrument) =
        load_sfz_with_instrument(sfz_path, None, engine_hz).expect("手搭：装载必须成功");
    let referenced = instrument
        .referenced_sample_paths()
        .expect("手搭：路径全集");
    let parent = sfz_path.parent().expect("SFZ 必须有父目录");
    let mut provided: Vec<(String, InMemorySample)> = Vec::new();
    for raw in &referenced {
        let raw_path = Path::new(raw);
        let resolved = if raw_path.is_absolute() {
            raw_path.to_path_buf()
        } else {
            parent.join(raw_path)
        };
        if let Ok((sample, _frames, _describe)) = read_sample_file(&resolved) {
            provided.push((raw.clone(), sample));
        }
    }
    SfzPlan::prepare_with_samples(&instrument, &provided, engine_hz).expect("手搭：摊平必须成功")
}

/// ⭐ 判据一（本单核心的正向腿）：`sampler = Some(真实路径)` ⇒
/// 导出**非静音**、开走了控制侧解码，且与"**显式手搭**的同一份摊平表"**逐位相同**。
///
/// # 判定量
///
/// * `nonzero > 0` / `peak > 0`（真的发声）；
/// * `auto.sfz_decoded` 恰好 1 条、键 = 那条 MIDI 轨、`regions` = 工程音高数、
///   `decoded_samples == 1`、`missing_sample_paths` 空、`silent_region_count == 0`；
/// * `bits(auto.samples) == bits(hand.samples)`（**逐位**；两条路只有解码这一步不同）；
/// * `auto` 的指纹 **≠** `sampler = None` 的指纹（⇒ 摊平表真的接上了实时侧，⛔ 不是账面动作）。
///
/// # 怎么变红
///
/// * 把 [`yeban_app::sfz_load::arm_pending_sfz`] 的调用删掉（= 跳过解码）⇒ `auto` 退回
///   复音合成器 ⇒ 逐位对照与指纹两条腿**同时红**（`sfz_decoded` 也空）；
/// * 把 `EngineTrackSource::render_block` 里从快照取摊平表那一段改回 `None` ⇒ 同上。
#[test]
fn the_model_sampler_path_is_decoded_and_sounds_only_through_the_new_chain() {
    let dir = scratch("positive");
    let engine_hz = 48_000.0_f32;
    write_const_sample(&dir.join("const.wav"), 0.25);

    let mut project = yeban_model::samples::filled_project();
    let lead = midi_track_of(&project);
    let pitches = pitches_of(&project, lead);
    let sfz_path = write_sfz(&dir, "keys.sfz", &pitches, |_| "const.wav".to_owned());
    bind_sampler(&mut project, lead, &sfz_path);

    // ---- 正向：模型链条（自动解码）----
    let auto = render_project_f32_with_assets(&project, float_options(), None, &[])
        .expect("模型链条渲染必须成功");

    // ---- 参照：测试内显式六步搭出的同一份摊平表，注入同一个渲染入口 ----
    let plan = hand_built_plan(&sfz_path, engine_hz);
    let mut plans = BTreeMap::new();
    plans.insert(lead, plan);
    let hand = render_project_f32_with_plans(&project, float_options(), None, &[], plans)
        .expect("手搭参照渲染必须成功");

    // ---- 反向对照：同工程的 `sampler = None` 版 ----
    let mut cleared = project.clone();
    cleared.tracks.get_mut(&lead).expect("轨在").sampler = None;
    let none = render_project_f32_with_assets(&cleared, float_options(), None, &[])
        .expect("`sampler = None` 渲染必须成功");

    println!(
        "[app-sfz-chain/positive] auto: nonzero={} peak={:.9} fnv=0x{:016x}；\
         hand: nonzero={} peak={:.9} fnv=0x{:016x}；none: nonzero={} fnv=0x{:016x}；\
         label={}",
        nonzero(&auto.samples),
        peak(&auto.samples),
        fnv1a_64(&auto.samples),
        nonzero(&hand.samples),
        peak(&hand.samples),
        fnv1a_64(&hand.samples),
        nonzero(&none.samples),
        fnv1a_64(&none.samples),
        auto.source_label,
    );
    assert!(
        nonzero(&auto.samples) > 0 && peak(&auto.samples) > 0.0,
        "⭐ 模型链条必须真的发声（nonzero={} peak={}）",
        nonzero(&auto.samples),
        peak(&auto.samples)
    );
    assert_eq!(
        auto.sfz_decoded.len(),
        1,
        "⭐ `sampler = Some(…)` ⇒ 恰好一条控制侧解码读数"
    );
    let (decoded_track, report) = &auto.sfz_decoded[0];
    assert_eq!(*decoded_track, lead, "解码读数必须挂在那条 MIDI 轨上");
    println!("[app-sfz-chain/positive] report: {}", report.summary());
    assert_eq!(
        report.regions,
        pitches.len(),
        "区域数必须 = 工程音高数（⛔ 不是硬编码）"
    );
    assert_eq!(
        report.decoded_samples, 1,
        "只引用一份 `const.wav` ⇒ 只解码一份（去重键 = `sample=` 原样字符串）"
    );
    assert!(
        report.missing_sample_paths.is_empty(),
        "样本都在 ⇒ 缺样本表必须为空：{:?}",
        report.missing_sample_paths
    );
    assert_eq!(report.silent_region_count, 0, "样本都在 ⇒ 没有静音区域");
    assert!(
        report.unreadable_samples.is_empty(),
        "样本都读得到 ⇒ 读失败表必须为空：{:?}",
        report.unreadable_samples
    );
    assert!(
        auto.source_label.contains("sfz(tracks=1"),
        "音源文案必须如实登记模型链条：{}",
        auto.source_label
    );
    assert_eq!(
        bits(&auto.samples),
        bits(&hand.samples),
        "⭐ 模型链条的输出必须与**显式手搭**的同一份摊平表**逐位相同**\
         （fnv auto=0x{:016x} hand=0x{:016x}）",
        fnv1a_64(&auto.samples),
        fnv1a_64(&hand.samples)
    );
    assert_eq!(
        auto.digest, hand.digest,
        "⭐ 摘要也必须相同（逐位比的独立见证）"
    );
    assert_ne!(
        fnv1a_64(&auto.samples),
        fnv1a_64(&none.samples),
        "⭐ `sampler = Some(…)` 必须与 `= None` **不同**（否则摊平表没接上实时侧）"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// ⭐ 判据二（本单核心的反向对照）：`sampler = None` ⇒ 输出与**接线前**逐位相同。
///
/// # 判定量
///
/// * `filled_project` ＋ 默认选项的 `RenderProjectOutput::samples` 的 FNV-1a **逐位**等于
///   [`BASELINE_APP`]（同平台硬红；异平台如实跳过并点名原因 —— 跳过**不是**通过）；
/// * `sfz_decoded` **空**、`source_label` **不含** `sfz(`（本轮接线一个字都没碰这条腿）；
/// * 结构腿：同一份工程的 `sampler` 显式清回 `None` ⇒ 再渲染仍是同一指纹。
#[test]
fn an_absent_sampler_is_bit_identical_to_before_the_wiring() {
    // ⭐ `#243`：这条判据的对象是“sampler 接线不改变非 sampler 路径”，自动化是另一个
    // 自变量 ⇒ 在这里把它清掉，让指纹仍能逐位对上 `#209` 的基线（隔离变量，不重钉）。
    let mut project = yeban_model::samples::filled_project();
    for track in project.tracks.values_mut() {
        track.automation_lanes.clear();
    }
    let out = render_project_f32_with_assets(&project, WavExportOptions::default(), None, &[])
        .expect("默认工程必须能导出");
    let actual = fnv1a_64(&out.samples);
    println!(
        "[app-sfz-chain/reverse] `sampler = None`: frames={} nonzero={} peak={:.9} fnv=0x{actual:016x} \
         label={}",
        out.frames,
        nonzero(&out.samples),
        peak(&out.samples),
        out.source_label,
    );
    assert!(
        nonzero(&out.samples) > 0,
        "夹具必须真的出声（否则等号是空转）"
    );
    assert!(
        out.sfz_decoded.is_empty(),
        "⛔ `sampler = None` ⇒ 解码读数必须为空（实得 {} 条）",
        out.sfz_decoded.len()
    );
    assert!(
        !out.source_label.contains("sfz("),
        "⛔ `sampler = None` ⇒ 音源文案不得出现模型 SFZ 链条：{}",
        out.source_label
    );
    if ON_MEASURED_PLATFORM {
        assert_eq!(
            actual, BASELINE_APP,
            "⭐ 同平台（本机 == {MEASURED_PLATFORM}）⇒ `sampler = None` 的输出必须逐位等于\
             **`#209` 重钉后的**实测指纹 0x{BASELINE_APP:016x}（实得 0x{actual:016x}）"
        );
    } else {
        println!(
            "[app-sfz-chain/reverse] SKIP（未判定，不是通过）：字面值 0x{BASELINE_APP:016x} \
             的测量平台是 {MEASURED_PLATFORM}，本机不是它"
        );
    }

    // 结构腿：显式清回 `None`（默认工程本来就是 `None`）⇒ 再渲染仍同指纹。
    let mut cleared = project.clone();
    let lead = midi_track_of(&cleared);
    assert!(
        cleared.tracks[&lead].sampler.is_none(),
        "`filled_project` 的 `sampler` 默认必须是 `None`"
    );
    cleared.tracks.get_mut(&lead).expect("轨在").sampler = None;
    let again = render_project_f32_with_assets(&cleared, WavExportOptions::default(), None, &[])
        .expect("清空后再渲染");
    assert_eq!(
        fnv1a_64(&again.samples),
        actual,
        "⭐ 显式 `sampler = None` 与默认必须是同一条路（同指纹）"
    );
}

/// ⭐ 判据三（缺样本）：路径**存在**但 `sample=` 指向的样本缺失 ⇒
/// 该区域**静音 ＋ 计数**（引擎既有口径，`R711`／`NEEDS-7`），且缺的是**谁**如实报出。
///
/// # 判定量
///
/// * 报告：`silent_region_count == 1`、`missing_sample_paths == ["missing.wav"]`、
///   `decoded_samples == 1`、`unreadable_samples` 有 1 条且含 `missing.wav`；
/// * 导出**非静音**（另一条区域照常发声 ⇒ 上一条不是"整条轨哑了"）；
/// * ⭐ 反向腿：一份**所有**区域都指向缺失样本的 SFZ ⇒ 导出样本**逐位全零**、
///   `silent_region_count == 区域数`（⛔ 不回退播别人的采样）。
#[test]
fn a_missing_sample_is_silent_and_counted_through_the_new_chain() {
    let dir = scratch("missing-sample");
    write_const_sample(&dir.join("present.wav"), 0.25);

    let mut project = yeban_model::samples::filled_project();
    let lead = midi_track_of(&project);
    let pitches = pitches_of(&project, lead);
    assert!(pitches.len() >= 2, "夹具需要至少两个音高");
    // 第 0 条区域有样本、其余全部缺失。
    let sfz_path = write_sfz(&dir, "one-missing.sfz", &pitches, |index| {
        if index == 0 {
            "present.wav".to_owned()
        } else {
            "missing.wav".to_owned()
        }
    });
    bind_sampler(&mut project, lead, &sfz_path);
    let out = render_project_f32_with_assets(&project, float_options(), None, &[])
        .expect("缺样本**不是**错误（区域静音 ＋ 计数）");
    assert_eq!(out.sfz_decoded.len(), 1, "恰好一条解码读数");
    let (_, report) = &out.sfz_decoded[0];
    println!(
        "[app-sfz-chain/missing] report: {}；nonzero={} peak={:.9}",
        report.summary(),
        nonzero(&out.samples),
        peak(&out.samples)
    );
    assert_eq!(
        report.silent_region_count,
        pitches.len() - 1,
        "缺样本的区域数必须 = 引用了 `missing.wav` 的区域数"
    );
    assert_eq!(
        report.missing_sample_paths,
        vec!["missing.wav".to_owned()],
        "⭐ 缺样本路径必须用**引擎口径**报出（排序 ＋ 去重）"
    );
    assert_eq!(report.decoded_samples, 1, "只有 `present.wav` 进了提供表");
    assert_eq!(
        report.unreadable_samples.len(),
        1,
        "读失败按**去重后的路径全集**各记一条（`missing.wav` 是同一个路径 ⇒ 一条），\
         实得：{:?}",
        report.unreadable_samples
    );
    assert!(
        report
            .unreadable_samples
            .iter()
            .all(|row| row.contains("missing.wav")),
        "读失败原因必须点名 `missing.wav`：{:?}",
        report.unreadable_samples
    );
    assert!(
        nonzero(&out.samples) > 0,
        "有样本的那条区域必须照常发声（否则'静音'没有作用域）"
    );

    // ---- 反向腿：所有区域都缺样本 ⇒ 逐位全零 ----
    let all_missing = write_sfz(&dir, "all-missing.sfz", &pitches, |_| {
        "missing.wav".to_owned()
    });
    let mut all_project = yeban_model::samples::filled_project();
    let all_lead = midi_track_of(&all_project);
    bind_sampler(&mut all_project, all_lead, &all_missing);
    let silent = render_project_f32_with_assets(&all_project, float_options(), None, &[])
        .expect("全缺样本也必须能导出");
    let (_, silent_report) = &silent.sfz_decoded[0];
    println!(
        "[app-sfz-chain/missing-all] report: {}；nonzero={}",
        silent_report.summary(),
        nonzero(&silent.samples)
    );
    assert_eq!(
        silent_report.silent_region_count,
        pitches.len(),
        "所有区域都缺样本"
    );
    assert_eq!(silent_report.decoded_samples, 0);
    assert!(
        silent.samples.iter().all(|sample| sample.to_bits() == 0),
        "⭐ 缺样本区域 ⇒ **不建声部** ⇒ 逐位全零（⛔ 不回退播别人的采样），实得峰值 {}",
        peak(&silent.samples)
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// ⭐ 判据四（SFZ 文件本身缺失）：**明确错误**，且 ⛔ 一个字节都不落盘。
///
/// # 判定量
///
/// * `render_project_f32_with_assets` ⇒ `Err(WavExportError::Sfz(msg))`，`msg` 里**点名路径**；
/// * `export_wav` 同样失败，且目标文件**不存在**（"拒绝时不留半成品"）。
///
/// ⚠ 与判据三的区别是**被读的东西**：文件本身读不到 ⇒ 错误；文件在、`sample=` 读不到 ⇒
/// 区域静音 ＋ 计数。两条都不静默，⛔ 不互相冒充。
#[test]
fn a_missing_sfz_file_is_an_explicit_error_and_writes_nothing() {
    let dir = scratch("missing-file");
    let mut project = yeban_model::samples::filled_project();
    let lead = midi_track_of(&project);
    let missing = dir.join("nope.sfz");
    bind_sampler(&mut project, lead, &missing);

    let error = render_project_f32_with_assets(&project, float_options(), None, &[])
        .expect_err("SFZ 文件不存在 ⇒ 必须明确失败");
    let WavExportError::Sfz(message) = &error else {
        panic!("必须是 `WavExportError::Sfz`（实得 {error:?}）");
    };
    println!("[app-sfz-chain/missing-file] {error}");
    assert!(
        message.contains("nope.sfz"),
        "错误消息必须点名那个路径：{message}"
    );

    let target = dir.join("out.wav");
    let export_error = export_wav(&project, &target, float_options())
        .expect_err("导出也必须失败（⛔ 不静默回退）");
    println!("[app-sfz-chain/missing-file] export: {export_error}");
    assert!(
        matches!(export_error, WavExportError::Sfz(_)),
        "导出错误也必须是 `Sfz`（实得 {export_error:?}）"
    );
    assert!(!target.exists(), "失败时⛔ 不得留下半个 WAV");
    std::fs::remove_dir_all(&dir).ok();
}
