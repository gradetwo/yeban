//! 判据（任务单 `#83`）：**SFZ 样本的"格式面"在报告里可判别**。
//!
//! ## 本单的交接形态（勘察结论，见 `sfz_load` 模块文档 ①）
//!
//! 本仓**已有**一份能解 FLAC／OGG 的解码入口 —— `yeban-decode`（symphonia ＋ rubato）。
//! 但 `yeban-app` **没有**那条依赖边（全仓只有 `yeban-mcp` 依赖它）⇒ 接进 app 需要新增
//! `crates/yeban-app/Cargo.toml` 一条边 ＋ 重生成 `Cargo.lock`（集成者独占）⇒ **本单不做**，
//! 按任务单的**乙**口径交付：不改格式面，只把"读不到"从**一句无法判别的错误文案**
//! 变成**结构化的可判别理由**（文件不存在 / 格式不受支持 / 文件在但读不动）。
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `fnv1a_64` | 一条导出的**交错 f32 样本**（逐位卷字节） | 无符号 64 位整数 |
//! | `unreadable_not_found_count` | [`SfzDecodeReport`] 里理由 = "文件不存在"的样本 | 条 |
//! | `unreadable_unsupported_format_count` | 理由 = "格式不受支持"（非 PCM／float WAV）的样本 | 条 |
//! | `unreadable_other_count` | 以上都不是的读失败 | 条 |
//! | `silent_region_count` | 引擎口径：缺样本 ⇒ `NO_SAMPLE` 的区域数 | 区域数 |
//!
//! ## 判据清单
//!
//! | # | 判据 | 作用 |
//! | :--- | :--- | :--- |
//! | ① | `the_wav_sample_path_is_bit_identical_to_the_pre_change_fingerprint` | **应绿的反向腿**：WAV 路径逐位不变 |
//! | ② | `the_unreadable_reason_separates_missing_files_from_unsupported_formats` | **乙核心**：`.xyz`／FLAC 与"文件不存在"分得开 |
//! | ③ | `the_export_report_line_carries_the_reason_counts` | 用户可见：导出报告行带可判别计数 |
//!
//! [`SfzDecodeReport`]: yeban_app::sfz_load::SfzDecodeReport

use std::path::{Path, PathBuf};

use yeban_app::render_core::render_project_f32_with_assets;
use yeban_app::sfz_load::{SampleReadFailure, decode_sfz_plan};
use yeban_model::{EntityId, SamplerBinding, YebanProjectV1};
use yeban_render::dither::{BitDepth, PcmBuffer};
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;

/// `24-bit` 满量程（夹具写入常量样本用；与 `read_plain_wav_mono_f32` 同口径）。
const I24_SCALE: f32 = 8_388_608.0;

/// ⭐ **接线前**（本单改动前的 HEAD `9a939318`，工作树干净）实测：`filled_project` 那条
/// MIDI 轨的 `sampler` 指向一份**内容合法**的 `present.wav`（24-bit 单声道常量 `0.25`）时，
/// `render_project_f32_with_assets(.., Float32)` 的 `RenderProjectOutput::samples`
/// （交错 `f32`）的 FNV-1a 64 指纹（**逐位**）。
///
/// ⚠ 它是**某台机器上的实测值**，⛔ 不是规范常数（同 `sfz_model_chain::BASELINE_APP` 的口径）：
/// [ARCH-DET-001] 的 L1 只在"相同 OS ＋ CPU 架构 ＋ 纯 Rust libm"下承诺逐位一致。
/// ⚠ 为什么需要它：既有判据 `sfz_model_chain::the_model_chain_is_bit_identical_to_the_hand_built_plan`
/// 的两条腿（`decode_sfz_plan` 与手搭 `read_sample_file`）**都要经过被本单改动的那个函数**
/// ⇒ 它对"WAV 路径被改坏了"**不独立**。本常量是那条腿的**独立**见证。
const BASELINE_WAV_READ: u64 = 0x0df4_78fa_1526_4e79;

/// 指纹的**测量平台**。
const MEASURED_PLATFORM: &str = "aarch64/macos";

/// 本机（**编译期事实**）是否就是那个测量平台。
const ON_MEASURED_PLATFORM: bool = cfg!(target_arch = "aarch64") && cfg!(target_os = "macos");

/// FNV-1a 64（逐位；把每个 `f32` 的 4 个字节按小端卷进哈希）—— 与 `sfz_model_chain` 同一算式。
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

/// 非零样本**个数**（单位：个）。
fn nonzero(samples: &[f32]) -> usize {
    samples.iter().filter(|sample| **sample != 0.0).count()
}

/// 峰值（单位：线性幅度）。
fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0_f32, |acc, sample| acc.max(sample.abs()))
}

/// 临时目录（形态照 `sfz_model_chain.rs` 的 `scratch`）。
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-sfz-formats-{tag}-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 32f 位深让"落盘"这一步**不引入抖动**（`quantize` 对 `Float32` 是透传）。
fn float_options() -> yeban_app::export_wav::WavExportOptions {
    yeban_app::export_wav::WavExportOptions {
        bit_depth: BitDepth::Float32,
        ..yeban_app::export_wav::WavExportOptions::default()
    }
}

/// 写一条**常量** PCM 样本（24-bit 单声道；值 = 满量程的 `value` 倍）。
fn write_const_sample(path: &Path, value: f32) {
    let scale = (I24_SCALE * value) as i32;
    let buffer = PcmBuffer::Int24(vec![scale; 4_800]);
    let format = PcmFormat::integer(1, 48_000, 24);
    write_plain_wav(path, &format, &buffer).expect("写样本夹具");
}

/// 工程里那条**带 MIDI 音符**的非主总线轨。
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
/// `sample_for` 决定第 `index` 条区域引用哪一份样本。
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

/// ⭐ 判据 ①（**应绿的反向腿**）：**WAV 样本路径的输出与接线前逐位相同**。
///
/// ## 判定量
///
/// * `decoded_samples == 1`（那份 `present.wav` 真的进了提供表 ⇒ 走的是**读到了**那条腿）；
/// * `nonzero > 0`（真的发声，不是"全零也算逐位相同"）；
/// * `fnv1a_64(samples) == BASELINE_WAV_READ`（逐位；仅在测量平台上断言）。
///
/// ⚠ **为什么本判据在"新增可判别理由"之后必须仍然绿**：本单只增加**读失败之后**的分类
/// 信息，⛔ 一个字节都不碰 `read_plain_wav_mono_f32` 的调用与它的成功路径
/// ⇒ 指纹必须**逐位不变**。它变红 = 改动了 WAV 路径（本单明确禁止）。
#[test]
fn the_wav_sample_path_is_bit_identical_to_the_pre_change_fingerprint() {
    let dir = scratch("wav-fingerprint");
    write_const_sample(&dir.join("present.wav"), 0.25);
    let mut project = yeban_model::samples::filled_project();
    let lead = midi_track_of(&project);
    let pitches = pitches_of(&project, lead);
    let sfz = write_sfz(&dir, "wav.sfz", &pitches, |_| "present.wav".to_owned());
    bind_sampler(&mut project, lead, &sfz);

    let out = render_project_f32_with_assets(&project, float_options(), None, &[])
        .expect("合法 WAV 样本必须能导出");
    let fingerprint = fnv1a_64(&out.samples);
    let (_, report) = &out.sfz_decoded[0];
    println!(
        "[app-sfz-formats/wav-fingerprint] platform={MEASURED_PLATFORM} fnv1a_64=0x{fingerprint:016x} \
         nonzero={} peak={:.9} decoded_samples={} unreadable={:?}",
        nonzero(&out.samples),
        peak(&out.samples),
        report.decoded_samples,
        report.unreadable_samples
    );

    assert_eq!(
        report.decoded_samples, 1,
        "`present.wav` 必须真的进提供表（否则本判据比的是'两条腿都没读到'）"
    );
    assert!(nonzero(&out.samples) > 0, "样本必须真的发声");
    assert!(
        report.unreadable_samples.is_empty(),
        "没有读失败的样本：{:?}",
        report.unreadable_samples
    );
    if ON_MEASURED_PLATFORM {
        assert_eq!(
            fingerprint, BASELINE_WAV_READ,
            "⭐ 同平台（本机 == {MEASURED_PLATFORM}）⇒ WAV 样本路径的输出必须逐位等于\
             **接线前**实测指纹 0x{BASELINE_WAV_READ:016x}（实得 0x{fingerprint:016x}）\
             —— 改坏了 WAV 路径即红"
        );
    } else {
        println!(
            "[app-sfz-formats/wav-fingerprint] SKIP（未判定，不是通过）：字面值 \
             0x{BASELINE_WAV_READ:016x} 的测量平台是 {MEASURED_PLATFORM}，本机不是它"
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}

/// 写一份"五条区域各指一份样本"的 SFZ（区域键各不相同；`sample=` 用相对路径）。
fn write_five_region_sfz(dir: &Path, name: &str, samples: &[&str]) -> PathBuf {
    let mut text = String::new();
    for (offset, sample) in samples.iter().enumerate() {
        let key = 60 + offset;
        text.push_str(&format!(
            "<region> sample={sample} key={key} pitch_keycenter={key}\n"
        ));
    }
    let path = dir.join(name);
    std::fs::write(&path, text).expect("写 SFZ 夹具");
    path
}

/// ⭐ 判据 ②（**本单核心的乙口径**）：一份读不到的样本，报告里能**判别**"文件不存在"
/// 与"格式不受支持（FLAC／OGG／未知后缀）"。
///
/// ## 五条区域（⛔ 每一条都是真文件，除 `nothere.wav`）
///
/// | `sample=` | 盘上是什么 | 期望理由 |
/// | :--- | :--- | :--- |
/// | `present.wav` | 合法 24-bit PCM WAV | （**读成功**） |
/// | `nothere.wav` | **不存在** | [`SampleReadFailure::NotFound`] |
/// | `blob.xyz` | 存在；内容是任意文本 | [`SampleReadFailure::UnsupportedFormat`] |
/// | `fake.flac` | 存在；内容以 `fLaC` 开头（这就是真实音色库里的那一类） | [`SampleReadFailure::UnsupportedFormat`] |
/// | `broken.wav` | 存在；`RIFF…WAVE` 头 ＋ 垃圾体（**像 WAV 但读不动**） | [`SampleReadFailure::Unreadable`] |
///
/// ## 判定量
///
/// * 计数：`not_found == 1`、`unsupported == 2`、`other == 1`（单位：条）；
/// * 逐条：四条登记的 `reason` 与上表**逐行相同**（单位：理由枚举）；
/// * `decoded_samples == 1`（`present.wav` 那一份**真的读了** ⇒ 不是"全都没读"）；
/// * `silent_region_count == 4`（引擎口径：缺样本的区域数）；
/// * 旧的字符串视图 `unreadable_samples` 仍是 4 条且每条点名自己的路径（⛔ 不因本单消失）。
///
/// ⚠ **红/绿怎么读**：若把分类退化成"读不到就只有一种理由"，下面的 `unsupported == 2`
/// 与逐条 `reason` 断言立刻红 —— 这正是本单要挡住的那件事（"缺样本"读起来像一个数）。
#[test]
fn the_unreadable_reason_separates_missing_files_from_unsupported_formats() {
    let dir = scratch("reasons");
    // ① 合法 WAV（正向对照：这一条必须**读成功**，否则本判据比的是"全都读不到"）。
    write_const_sample(&dir.join("present.wav"), 0.25);
    // ② 不存在：不写。
    // ③ 未知后缀 ＋ 任意内容。
    std::fs::write(dir.join("blob.xyz"), b"this is not audio at all\n").expect("写 .xyz 夹具");
    // ④ 真 FLAC 的魔数（`fLaC`）＋ 垃圾体：本机没有真 FLAC 样本 ⇒ 用**魔数**造一个
    //    "读起来像 FLAC 但本入口必拒"的夹具（⛔ 它不证明能解 FLAC，只证明**理由**分得开）。
    let mut flac = b"fLaC\x00\x00\x00\x22".to_vec();
    flac.extend_from_slice(&[0xAB; 64]);
    std::fs::write(dir.join("fake.flac"), &flac).expect("写 FLAC 魔数夹具");
    // ⑤ 像 WAV（RIFF/WAVE 头）但体是垃圾 ⇒ 归"文件在但读不动"。
    let mut broken = b"RIFF\x24\x00\x00\x00WAVE".to_vec();
    broken.extend_from_slice(&[0x00; 32]);
    std::fs::write(dir.join("broken.wav"), &broken).expect("写坏 WAV 夹具");

    let sfz = write_five_region_sfz(
        &dir,
        "reasons.sfz",
        &[
            "present.wav",
            "nothere.wav",
            "blob.xyz",
            "fake.flac",
            "broken.wav",
        ],
    );
    let (_plan, report) =
        decode_sfz_plan(&sfz, 48_000.0).expect("装载必须成功（样本读不到不是错误）");
    println!("[app-sfz-formats/reasons] {}", report.summary());
    for entry in &report.unreadable_sample_reasons {
        println!(
            "[app-sfz-formats/reasons]   {} => {} （{}）",
            entry.path, entry.reason, entry.detail
        );
    }

    assert_eq!(
        report.referenced_sample_paths.len(),
        5,
        "五条区域各指一份样本：{:?}",
        report.referenced_sample_paths
    );
    assert_eq!(report.decoded_samples, 1, "只有 `present.wav` 该被读成功");

    // ---- 逐条：理由必须与夹具的真实情形一致（按文件名取，避开临时目录前缀）----
    let reason_of = |suffix: &str| -> Option<SampleReadFailure> {
        report
            .unreadable_sample_reasons
            .iter()
            .find(|entry| entry.path.ends_with(suffix))
            .map(|entry| entry.reason)
    };
    assert_eq!(
        reason_of("nothere.wav"),
        Some(SampleReadFailure::NotFound),
        "盘上没有的文件 ⇒ 必须是 `NotFound`（⛔ 不能与格式问题混为一谈）"
    );
    assert_eq!(
        reason_of("blob.xyz"),
        Some(SampleReadFailure::UnsupportedFormat),
        "⭐ `.xyz` 后缀 ⇒ 必须是 `UnsupportedFormat`（任务单 `#83` 逐字点名的那条）"
    );
    assert_eq!(
        reason_of("fake.flac"),
        Some(SampleReadFailure::UnsupportedFormat),
        "⭐ FLAC ⇒ 必须是 `UnsupportedFormat`（⛔ 不是「缺文件」）"
    );
    assert_eq!(
        reason_of("broken.wav"),
        Some(SampleReadFailure::Unreadable),
        "有 RIFF/WAVE 头但读不动 ⇒ 是**内容**问题，⛔ 不是格式不受支持"
    );

    // ---- 计数：三个数各就各位（单位：条）----
    assert_eq!(report.unreadable_not_found_count(), 1, "文件不存在 1 条");
    assert_eq!(
        report.unreadable_unsupported_format_count(),
        2,
        "格式不受支持 2 条（`.xyz` ＋ FLAC 魔数）"
    );
    assert_eq!(report.unreadable_other_count(), 1, "读不动 1 条");

    // ---- 引擎口径 ＋ 旧视图：缺样本的区域数不变；字符串登记不因本单消失 ----
    assert_eq!(
        report.silent_region_count, 4,
        "四条读不到的区域 ⇒ 引擎标 `NO_SAMPLE`（静音 ＋ 计数）"
    );
    assert_eq!(
        report.unreadable_samples.len(),
        4,
        "旧字符串视图必须仍是 4 条（⛔ 本单不改它的形状）：{:?}",
        report.unreadable_samples
    );
    for entry in &report.unreadable_sample_reasons {
        let name = Path::new(&entry.path)
            .file_name()
            .expect("必须有文件名")
            .to_string_lossy()
            .into_owned();
        assert!(
            report
                .unreadable_samples
                .iter()
                .any(|row| row.contains(&name)),
            "字符串视图必须逐条点名 `{name}`：{:?}",
            report.unreadable_samples
        );
    }
    assert!(
        report
            .summary()
            .contains("unreadable_kind(not_found=1,unsupported=2,other=1)"),
        "摘要里必须带可判别计数：{}",
        report.summary()
    );
    assert!(
        SampleReadFailure::UnsupportedFormat
            .to_string()
            .contains("FLAC"),
        "`UnsupportedFormat` 的文案必须点名 FLAC／OGG（用户要能读懂「为什么」）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// ⭐ 判据 ③（**用户可见腿**）：导出报告行 `source=…sfz(… unreadable(not_found=…,…))`
/// 真的带**可判别计数** —— 即"用户在 CLI 上看得见"，⛔ 不是只在结构体里。
///
/// ## 夹具
///
/// 工程那条 MIDI 轨的 `sampler` 指向一份 SFZ：第 0 个音高 → 合法 `present.wav`，
/// 其余音高 → **FLAC 魔数** `fake.flac`（去重后 `referenced_sample_paths` 恰好 2 条）。
///
/// ## 判定量
///
/// * `source_label` 含 `unreadable(not_found=0,unsupported=1,other=0)`（判据：子串存在）；
/// * `silent_region_count == 音高数 − 1`、`nonzero > 0`（有样本的那条**照常发声**）。
#[test]
fn the_export_report_line_carries_the_reason_counts() {
    let dir = scratch("label");
    write_const_sample(&dir.join("present.wav"), 0.25);
    let mut flac = b"fLaC\x00\x00\x00\x22".to_vec();
    flac.extend_from_slice(&[0xCD; 64]);
    std::fs::write(dir.join("fake.flac"), &flac).expect("写 FLAC 魔数夹具");

    let mut project = yeban_model::samples::filled_project();
    let lead = midi_track_of(&project);
    let pitches = pitches_of(&project, lead);
    assert!(pitches.len() >= 2, "夹具需要至少两个音高");
    let sfz = write_sfz(&dir, "label.sfz", &pitches, |index| {
        if index == 0 {
            "present.wav".to_owned()
        } else {
            "fake.flac".to_owned()
        }
    });
    bind_sampler(&mut project, lead, &sfz);

    let out = render_project_f32_with_assets(&project, float_options(), None, &[])
        .expect("格式不受支持的样本**不是**错误（区域静音 ＋ 计数）");
    println!(
        "[app-sfz-formats/label] source_label={} nonzero={} peak={:.9}",
        out.source_label,
        nonzero(&out.samples),
        peak(&out.samples)
    );

    assert!(
        out.source_label
            .contains("unreadable(not_found=0,unsupported=1,other=0)"),
        "⭐ 导出报告行必须带可判别计数（FLAC ⇒ unsupported=1，⛔ 不是 not_found）：{}",
        out.source_label
    );
    let (_, report) = &out.sfz_decoded[0];
    assert_eq!(
        report.silent_region_count,
        pitches.len() - 1,
        "引用 FLAC 的区域必须静音并计数"
    );
    assert!(
        nonzero(&out.samples) > 0,
        "有样本的那条区域必须照常发声（否则「静音」没有作用域）"
    );

    std::fs::remove_dir_all(&dir).ok();
}
