//! ⭐ 任务单 `#217`：`sample=*silence` 是 SFZ **内建静音伪样本**（app 侧判据）。
//!
//! ## 量什么（对象 ＋ 单位，先说再跑）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `regions` | [`SfzDecodeReport::regions`] | 区域数 |
//! | `samples` | [`SfzDecodeReport::decoded_samples`] | 真的解出 PCM 的样本**份数** |
//! | `silent` | [`SfzDecodeReport::silent_region_count`] | **不产生采样输出**的区域数（缺样本 ＋ 内建静音） |
//! | `builtin_silent` | [`SfzDecodeReport::builtin_silent_region_count`] | `sample=*silence` 区域数 |
//! | `missing` | [`SfzDecodeReport::missing_sample_paths`] | **缺文件**路径条数（⛔ 不含 `*silence`） |
//! | `unreadable` | [`SfzDecodeReport::unreadable_sample_reasons`] | 读盘失败条数（⛔ 不含 `*silence`） |
//! | `frames` / `energy` | `probe` 与离线渲染 | 帧 / `Σx²`（0 ＝ 静音） |
//! | `peak` | 导出 WAV 的样本 | `max |x|`（0.0 ＝ 逐位静音） |
//!
//! ## 它**不**证什么
//!
//! ⛔ 没人耳听过；⛔ 不证真库 3 件乐器的完整读数（那三条在交付报告的 `--sfz`／
//! `--track-sampler` 实跑读数里，不进自动化判据 —— 它们依赖开发机的库）。
//!
//! ## 反向对照（判据有牙）
//!
//! 每条"静音"断言都配一条**同夹具里的真样本区域**：它必须解出 1 份 PCM、
//! 必须出声（能量 > 0）。⇒ "0"⛔ 不是"整条夹具都不匹配"的假象。

use std::path::PathBuf;

use yeban_app::sfz_load::{
    BUILTIN_SILENCE_SOURCE, decode_sfz_plan, load_sfz_with_instrument, region_sample_for,
};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::{read_plain_wav_mono_f32, write_plain_wav};

const RATE: f32 = 48_000.0;

/// 临时夹具目录（`Drop` 删掉自己；⛔ 工作树里不留临时文件）。
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-217-{name}-{}-{}",
            std::process::id(),
            yeban_model::EntityId::new().to_canonical_string()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        // 一份 480 帧、常量 0.25 的 16-bit 单声道 WAV（"真样本"腿）。
        let samples: Vec<i16> = (0..480).map(|_| 8_192).collect();
        write_plain_wav(
            dir.join("tone.wav"),
            &PcmFormat::integer(1, RATE as u32, 16),
            &PcmBuffer::Int16(samples),
        )
        .expect("写夹具 WAV");
        Self { dir }
    }

    fn write_sfz(&self, name: &str, text: &str) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, text).expect("写夹具 SFZ");
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|value| value * value).sum()
}

fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0_f32, |acc, value| acc.max(value.abs()))
}

/// ⭐ 解码管线：`*silence` ⇒ `silent=1` ＋ `builtin_silent=1`，
/// ⛔ `missing=[]`、⛔ `unreadable=[]`、⛔ 不读盘；另一条真样本区域**不受影响**。
#[test]
fn the_decode_pipeline_counts_silence_as_silent_and_never_as_missing() {
    let fixture = Fixture::new("decode");
    let path = fixture.write_sfz(
        "mixed.sfz",
        "<region> key=60 sample=*silence\n<region> key=72 sample=tone.wav\n",
    );
    let (_plan, report) = decode_sfz_plan(&path, RATE).expect("解码管线");
    println!(
        "[app/silence/decode] regions={} samples={} silent={} builtin_silent={} missing={:?} \
         unreadable={:?} referenced={:?}",
        report.regions,
        report.decoded_samples,
        report.silent_region_count,
        report.builtin_silent_region_count,
        report.missing_sample_paths,
        report.unreadable_samples,
        report.referenced_sample_paths,
    );
    assert_eq!(report.regions, 2, "区域数");
    assert_eq!(report.decoded_samples, 1, "只解码真样本那一份");
    assert_eq!(report.silent_region_count, 1, "不产生输出的区域数");
    assert_eq!(
        report.builtin_silent_region_count, 1,
        "内建静音区域数（silent 的子集）"
    );
    assert!(
        report.missing_sample_paths.is_empty(),
        "`*silence` ⛔ 不是缺文件；实得 {:?}",
        report.missing_sample_paths
    );
    assert!(
        report.unreadable_sample_reasons.is_empty(),
        "`*silence` ⛔ 不是读盘失败；实得 {:?}",
        report.unreadable_samples
    );
    assert_eq!(report.unreadable_not_found_count(), 0, "NotFound 条数");
    assert_eq!(
        report.referenced_sample_paths,
        vec!["*silence".to_owned(), "tone.wav".to_owned()],
        "`referenced_sample_paths` 仍是**事实记录**（逐字含 `*silence`）"
    );
    // 摊平表：静音区域一帧不写；真样本区域出声（反向对照）。
    let (plan, _report) = decode_sfz_plan(&path, RATE).expect("解码管线");
    let mut silence_out = vec![0.0_f32; 128];
    let silence_frames = plan.prepared().render_block_into(60, 100, &mut silence_out);
    let mut tone_out = vec![0.0_f32; 128];
    let tone_frames = plan.prepared().render_block_into(72, 100, &mut tone_out);
    println!(
        "[app/silence/decode/render] silence_frames={silence_frames} \
         silence_energy={:.9} tone_frames={tone_frames} tone_energy={:.9}",
        energy(&silence_out),
        energy(&tone_out)
    );
    assert_eq!(silence_frames, 0, "内建静音：写入帧数 0");
    assert_eq!(energy(&silence_out), 0.0, "内建静音：能量 0");
    assert!(tone_frames > 0, "反向对照：真样本区域必须出声");
    assert!(energy(&tone_out) > 0.0, "反向对照：真样本区域能量 > 0");
}

/// ⭐ `--sfz` 旁路：探测能量 ＝ 0（这一条**恰好**可以用能量判 —— 它**该**是 0），
/// `region_sample_for` 给出 [`BUILTIN_SILENCE_SOURCE`]（⛔ 不是回退正弦）。
#[test]
fn the_sfz_probe_and_region_sample_report_silence_not_a_sine_fallback() {
    let fixture = Fixture::new("probe");
    let path = fixture.write_sfz("silence-only.sfz", "<region> key=60 sample=*silence\n");
    let (report, instrument) = load_sfz_with_instrument(&path, Some(60), RATE).expect("装载");
    let probe = report.probe.expect("给了探测音高");
    println!(
        "[app/silence/probe] regions={} frames={} energy={:.9} sample_source={}",
        report.regions, probe.frames, probe.energy, report.sample_source
    );
    assert_eq!(report.regions, 1, "区域数");
    assert_eq!(probe.frames, 0, "`*silence` 区域：探测帧数 0");
    assert_eq!(probe.energy, 0.0, "`*silence` 区域：探测能量 0");
    assert!(!probe.is_audible(), "`is_audible()` 必须为 false");

    let (sample, source) = region_sample_for(&instrument, &path, 60, 100).expect("匹配到静音区域");
    println!(
        "[app/silence/region_sample] source={source} pcm_len={} sample_hz={}",
        sample.pcm.len(),
        sample.sample_hz
    );
    assert_eq!(source, BUILTIN_SILENCE_SOURCE, "来源文案必须逐字");
    assert!(sample.pcm.is_empty(), "内建静音：PCM 必须为空");
    assert!(
        !source.contains("回退"),
        "⛔ 不许出现 `回退`（那会把内建值说成缺文件）: {source}"
    );
}

/// ⭐ CLI 端到端：`--sfz` ＋ `--export-wav` ⇒ `source=sfz(builtin-silence(*silence))`
/// 且导出的 WAV **真的静音**。
///
/// ⚠ 两条位深各说一件事（⛔ 不混淆）：
/// * `--wav-bit-depth 32f`（**无抖动**）⇒ `energy == 0.0`、`peak == 0.0`（**逐位**静音）；
/// * 默认 24-bit ⇒ 输出仍带 **TPDF 抖动**（±1 LSB @24-bit）⇒ 读数必然有
///   `peak ≲ 2^-23`、`energy ≲ frames × (2^-23)²`。那是**量化噪声**，⛔ 不是信号：
///   判据断言它 ≤ 1 LSB（若 `*silence` 漏出声，正弦峰值会是 `0.5` 量级 ⇒ 必红）。
#[test]
fn the_cli_export_of_a_silence_only_instrument_is_really_silent() {
    let fixture = Fixture::new("cli-sfz");
    let path = fixture.write_sfz(
        "silence-only.sfz",
        "<region> key=0 hikey=127 sample=*silence\n",
    );

    // 一条跑两遍：32f（无抖动）与默认 24-bit（TPDF 抖动）。
    let run = |name: &str, depth: Option<&str>| {
        let out = fixture.dir.join(name);
        let mut argv = vec![
            "--sfz".to_owned(),
            path.display().to_string(),
            "--export-wav".to_owned(),
            out.display().to_string(),
        ];
        if let Some(depth) = depth {
            argv.push("--wav-bit-depth".to_owned());
            argv.push(depth.to_owned());
        }
        let options = yeban_app::cli::parse(&argv).expect("命令行必须解析");
        let lines = yeban_app::cli::run_batch(&options).expect("批处理");
        let loaded = lines
            .iter()
            .find(|line| line.starts_with("loaded-sfz:"))
            .expect("必须有一行 loaded-sfz:");
        let exported = lines
            .iter()
            .find(|line| line.starts_with("exported-wav:"))
            .expect("必须有一行 exported-wav:");
        println!("[app/silence/cli] {loaded}");
        println!("[app/silence/cli] {exported}");
        assert!(
            loaded.contains("probe(note:vel:frames:energy)=60:100:0:0"),
            "探测读数必须是 0 帧 / 0 能量: {loaded}"
        );
        assert!(
            exported.contains("source=sfz(builtin-silence(*silence))"),
            "来源必须逐字是内建静音: {exported}"
        );
        assert!(
            !exported.contains("回退") && !exported.contains("in-memory-sine"),
            "⛔ 不许回退到内存正弦: {exported}"
        );
        assert!(out.is_file(), "WAV 必须真的落盘");
        let (pcm, rate, describe) = read_plain_wav_mono_f32(&out).expect("读回导出的 WAV");
        println!(
            "[app/silence/cli/wav/{name}] {describe} energy={:.12e} peak={:.12e} sample_rate={rate}",
            energy(&pcm),
            peak(&pcm)
        );
        assert!(!pcm.is_empty(), "导出的 WAV 必须有帧");
        (pcm, rate)
    };

    let (float_pcm, _) = run("silence-32f.wav", Some("32f"));
    assert_eq!(energy(&float_pcm), 0.0, "32f 导出能量必须逐位 0");
    assert_eq!(peak(&float_pcm), 0.0, "32f 导出峰值必须逐位 0");

    let (pcm24, _) = run("silence-24.wav", None);
    // 24-bit 的 1 LSB：解码腿按 `Int24 / 2^23 = /8_388_608` 归一（`#254` 起；⛔ 不是
    // `2^-23` 的近似值，也⛔ 不是 `2^23 − 1`）。TPDF 抖动最多带来 ±1 LSB。
    let one_lsb = 1.0_f32 / 8_388_608.0;
    assert!(
        energy(&pcm24) <= pcm24.len() as f32 * one_lsb * one_lsb,
        "24-bit 导出的能量必须 ≤ 抖动底（每样本 ≤ 1 LSB²），实得 {:.12e}",
        energy(&pcm24)
    );
    assert!(
        peak(&pcm24) <= one_lsb,
        "24-bit 导出的峰值必须 ≤ 1 LSB（{one_lsb:.3e}），实得 {:.12e}",
        peak(&pcm24)
    );
    // ⛔ 反向对照：若 `*silence` 真的漏出声（旧行为是内存正弦），峰值会是 0.5 量级。
    assert!(
        peak(&pcm24) < 0.001,
        "出现可听信号（peak={:.6}）⇒ `*silence` 漏出声",
        peak(&pcm24)
    );
}

/// ⭐ 模型链（`TrackV3.sampler`）：`*silence` ⛔ 不再让"可装"谓词失败；
/// 导出报告逐字 `sfz(tracks=1 regions=2 samples=1 silent=1 missing=[])`。
#[test]
fn the_track_sampler_chain_reports_silent_without_missing() {
    let fixture = Fixture::new("track-sampler");
    let path = fixture.write_sfz(
        "mixed.sfz",
        "<region> key=0 hikey=127 sample=*silence\n<region> key=72 sample=tone.wav\n",
    );
    let out = fixture.dir.join("mixed.wav");
    let options = yeban_app::cli::parse(&[
        "--track-sampler".to_owned(),
        path.display().to_string(),
        "--export-wav".to_owned(),
        out.display().to_string(),
    ])
    .expect("命令行必须解析");
    let lines = yeban_app::cli::run_batch(&options).expect("批处理");
    let installed = lines
        .iter()
        .find(|line| line.starts_with("track-sampler:"))
        .expect("必须有一行 track-sampler:");
    let exported = lines
        .iter()
        .find(|line| line.starts_with("exported-wav:"))
        .expect("必须有一行 exported-wav:");
    println!("[app/silence/track] {installed}");
    println!("[app/silence/track] {exported}");
    assert!(
        exported.contains("sfz(tracks=1 regions=2 samples=1 silent=1 missing=[]"),
        "模型链读数必须把 `*silence` 记成 silent、⛔ 不是 missing: {exported}"
    );
    assert!(
        !exported.contains("*silence"),
        "`*silence` ⛔ 不许出现在 missing 里: {exported}"
    );
    // 正向：与"装不上"分开 —— 安装入口必须成功（decoded>0 且没有缺文件）。
    assert!(
        !installed.contains("装不了"),
        "有真样本、只有内建静音的乐器必须可装: {installed}"
    );
}
