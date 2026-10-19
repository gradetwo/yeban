//! ⭐ 任务单 `#239`（缺陷 F3）：`--record-wav` 与 `--track-sampler` **同一调用**时，
//! 录制必须听到**本次**绑定；`#222` 实测的旧行为是"录制先跑（旧 `cli.rs:3986`）、
//! 绑定在后（`4138`）" ⇒ 录到默认音源。
//!
//! ## 这条判据证的句子
//!
//! 同一份演示工程、**只换**绑定的样本字节（0.5 / 0.25 常量）⇒ 两条 `--record-wav …
//! --track-sampler` 的峰值之比必须是 **2**（±0.1 %），且 `recorded-wav:` 行的 `source=`
//! 必须含 `sfz(tracks=1 regions=1 samples=1 silent=0 …missing=[])`。
//!
//! * 若录制发生在绑定之前 ⇒ 两次录制都读默认音源 ⇒ 峰值**相同**（比值为 1），`source=`
//!   里也**没有** `sfz(` —— 两条断言都会红（这正是修前 `#239` 的实测形状）。
//! * 只在 `source=` 里看到 `sfz(` **不够**：那只能证明"绑定的 SFZ 被解码过"，
//!   不能证明"写进 WAV 的样本来自它" ⇒ 所以峰值比是同一条判据的**正对照**。
//!
//! ## 它**不**证什么
//!
//! - ⛔ 没人耳听过：只比 `f32` 峰值与来源读数；
//! - ⛔ 不证 MCP 的 `notesUsingFileSample` 计数（那是 `yeban_render_master` 的读数面，
//!   ⛔ 不在 `--record-wav` 的 stdout 上；交付报告里单独给读数）；
//! - ⛔ 不证单独 `--record-wav` 与 `--export-wav` 的**逐字节**不变：那两条用修前/修后
//!   二进制的 `sha256` 对账，写进交付报告，不进本判据（判据里没有"修前二进制"）。
//!
//! ## 环境边界
//!
//! 样本与 SFZ 全在 `std::env::temp_dir()` 的临时目录里（`Drop` 删除），⛔ 不写仓库、
//! ⛔ 不读用户采样库；演示工程是内建 `--project-sample default`，⛔ 不依赖任何本机文件。

use std::path::{Path, PathBuf};
use std::process::Command;

use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::{read_plain_wav_mono_f32, write_plain_wav};

const RATE: u32 = 48_000;

/// 临时夹具目录（`Drop` 删掉自己；⛔ 工作树里不留临时文件）。
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-239-{tag}-{}-{}",
            std::process::id(),
            yeban_model::EntityId::new().to_canonical_string()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        Self { dir }
    }

    /// 写一份**常量** 16-bit 单声道样本（`value` 是满量程比例）。
    fn write_sample(&self, name: &str, value: f32) -> PathBuf {
        let frames = 24_000; // 0.5 s @ 48 kHz，覆盖判据的 0.5 s 录制
        let amplitude = (value * f32::from(i16::MAX)) as i16;
        let path = self.dir.join(name);
        write_plain_wav(
            &path,
            &PcmFormat::integer(1, RATE, 16),
            &PcmBuffer::Int16(vec![amplitude; frames]),
        )
        .expect("写夹具样本");
        path
    }

    /// 写一份**全键域** SFZ（一个区域指向 `sample`）—— 演示工程的 6 个音符都会落到它上面。
    fn write_sfz(&self, name: &str, sample: &str) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, format!("<region> key=0 hikey=127 sample={sample}\n"))
            .expect("写夹具 SFZ");
        path
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 跑一次真二进制，返回 `recorded-wav:` 那一行、整份 stdout、峰值（线性幅度）。
fn record(fixture: &Fixture, sfz: Option<&Path>, out: &str) -> (String, String, f32) {
    let wav = fixture.path(out);
    let mut command = Command::new(env!("CARGO_BIN_EXE_yeban-app"));
    if let Some(sfz) = sfz {
        command.args(["--track-sampler", sfz.to_str().expect("utf8 SFZ 路径")]);
    }
    let run = command
        .args([
            "--record-wav",
            wav.to_str().expect("utf8 WAV 路径"),
            "--record-seconds",
            "0.5",
        ])
        .output()
        .expect("跑 yeban-app --record-wav");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&run.stderr).into_owned();
    assert!(
        run.status.success(),
        "退出码={:?} stderr={stderr}\nstdout={stdout}",
        run.status.code()
    );
    let line = stdout
        .lines()
        .find(|line| line.starts_with("recorded-wav:"))
        .expect("必须打印录制回执")
        .to_owned();
    let (samples, rate, _describe) = read_plain_wav_mono_f32(&wav).expect("读回录制的 WAV");
    assert_eq!(rate, RATE, "录制采样率");
    let peak = samples
        .iter()
        .fold(0.0_f32, |acc, sample| acc.max(sample.abs()));
    (line, stdout, peak)
}

#[test]
fn recording_in_the_same_invocation_uses_the_newly_bound_sampler() {
    let fixture = Fixture::new("bind-before-record");
    fixture.write_sample("loud.wav", 0.5);
    fixture.write_sample("quiet.wav", 0.25);
    let loud = fixture.write_sfz("loud.sfz", "loud.wav");
    let quiet = fixture.write_sfz("quiet.sfz", "quiet.wav");

    let (loud_line, loud_stdout, loud_peak) = record(&fixture, Some(&loud), "loud-take.wav");
    let (quiet_line, _quiet_stdout, quiet_peak) = record(&fixture, Some(&quiet), "quiet-take.wav");
    println!("[#239] loud: {loud_line} peak={loud_peak}");
    println!("[#239] quiet: {quiet_line} peak={quiet_peak}");

    for (tag, line) in [("0.5", &loud_line), ("0.25", &quiet_line)] {
        assert!(
            line.contains("sfz(tracks=1 regions=1 samples=1 silent=0"),
            "{tag}: 录制必须报告绑定的 SFZ 被解码（source= 含 sfz(...)）: {line}"
        );
        assert!(line.contains("missing=[]"), "{tag}: 不得有缺失样本: {line}");
    }
    // 绑定只施加**一次**：合并调用的 `track-sampler:` 行仍然是 `ops=1`（不是被预应用吃掉后的
    // `ops=0`）—— 这条断言挡住"把预应用的副本写成真身"的错修（会在合并调用里改变提交语义）。
    let sampler_lines: Vec<&str> = loud_stdout
        .lines()
        .filter(|line| line.starts_with("track-sampler:"))
        .collect();
    assert_eq!(
        sampler_lines.len(),
        1,
        "恰好一条绑定回执: {sampler_lines:?}"
    );
    assert!(
        sampler_lines[0].contains("ops=1 commit_count=2 undoable=1"),
        "绑定必须恰好提交一次（预应用只作用于录音副本）: {}",
        sampler_lines[0]
    );
    assert!(loud_peak > 0.0, "录制必须非静音: peak={loud_peak}");
    // 正对照：只换绑定的样本字节 ⇒ 峰值必须跟着变，且比例可独立算出（0.5 / 0.25 = 2）。
    let ratio = loud_peak / quiet_peak;
    assert!(
        (ratio - 2.0).abs() < 2.0 * 1.0e-3,
        "峰值比必须是 2.0（录制读的是绑定的样本）: {loud_peak} / {quiet_peak} = {ratio}"
    );

    // 反向对照（同一夹具）：不给 `--track-sampler` ⇒ `source=` 里**没有** `sfz(`。
    let (plain_line, _plain_stdout, plain_peak) = record(&fixture, None, "plain-take.wav");
    println!("[#239] plain: {plain_line} peak={plain_peak}");
    assert!(
        !plain_line.contains("sfz("),
        "无绑定 ⇒ 不得出现 SFZ 来源: {plain_line}"
    );
    assert!(plain_peak > 0.0, "演示工程本身必须出声: peak={plain_peak}");
}
