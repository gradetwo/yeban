//! ⭐ 任务单 `#331`：WAV 的 `smpl` 块被**真读**（`loop_start`/`loop_end` 的缺省来源）。
//!
//! # 为什么这一层要单独钉
//!
//! 引擎层的判据（`sfz_sample_loop`）验的是"**给定** `SampleLoop` 之后窗口对不对"；
//! 本文件验的是"**字节 → `SampleLoop`**"这一段 —— 也就是 `sfz_load` 里那个
//! **自己扫 RIFF chunk** 的读法。两段任何一段错，用户听到的都是"只响一声"。
//!
//! `hound` 不暴露 `smpl` 块，`read_plain_wav_mono_f32` 也不读它 ⇒ 这一段只能是
//! 手写 chunk 遍历（见 `sfz_load::read_wav_smpl_loop` 的文档）。
//!
//! # 量什么（对象 ＋ 单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `loop_points` | [`InMemorySample::loop_points`] | `Option<SampleLoop>`（样本帧，半开） |
//! | 帧数 | 同一份样本的 `pcm.len()` | 帧（证明解码本身没被影响） |
//!
//! # 判据
//!
//! | 判据 | 钉住的事实 |
//! | :--- | :--- |
//! | [`the_smpl_loop_is_read_and_its_end_becomes_exclusive`] | `smpl` 的 `dwEnd`（**含**）读出来 **＋1**（半开） |
//! | [`a_wav_without_smpl_reports_no_loop`] | 没有 `smpl` 块 ⇒ `None`（⛔ 不是"全曲循环"、⛔ 不是报错） |
//! | [`a_smpl_block_with_zero_loops_reports_no_loop`] | `dwNumSampleLoops=0` ⇒ `None` |
//! | [`a_degenerate_smpl_loop_reports_no_loop`] | `dwEnd < dwStart` ⇒ `None` |
//! | [`a_broken_smpl_block_does_not_break_decoding`] | `smpl` 被截断 ⇒ 解码**照常成功**、`loop_points = None` |
//!
//! # 边界（⛔ 不声称）
//!
//! - ⛔ 不证 `RIFX` / `RF64`：本仓的 WAV 腿本来就只吃 RIFF；
//! - ⛔ 不证"多个 loop 取第几个"：只取第 0 条（规范逐字"the first loop defined"）；
//! - ⛔ 不证 `smpl` 之外的 loop 来源（`cue ` / `inst` / AIFF `MARK`）。

use std::path::PathBuf;

use yeban_app::sfz_load::read_sample_file;
use yeban_model::EntityId;

/// 夹具 WAV 的帧数（与 `#287` 的夹具同量级）。
const FRAMES: usize = 480;

/// 临时目录（`Drop` 删掉自己）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-331-smpl-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, bytes).expect("写夹具 WAV");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn u32le(value: u32) -> [u8; 4] {
    value.to_le_bytes()
}

fn u16le(value: u16) -> [u8; 2] {
    value.to_le_bytes()
}

/// 拼一份 48 kHz / 16-bit / 单声道 WAV。
///
/// `smpl` 参数是 `(dwStart, dwEnd, dwNumSampleLoops)` —— ⚠ `dwEnd` 是**含**的
/// （与 `smpl` 规范一致），判据靠它验"读出来 ＋1"。
fn wav_bytes(smpl: Option<(u32, u32, u32)>) -> Vec<u8> {
    let data_len = (FRAMES * 2) as u32;
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&u16le(1)); // PCM
    fmt.extend_from_slice(&u16le(1)); // 单声道
    fmt.extend_from_slice(&u32le(48_000));
    fmt.extend_from_slice(&u32le(48_000 * 2)); // byte rate
    fmt.extend_from_slice(&u16le(2)); // block align
    fmt.extend_from_slice(&u16le(16)); // bits

    let mut body = Vec::new();
    body.extend_from_slice(b"WAVE");
    body.extend_from_slice(b"fmt ");
    body.extend_from_slice(&u32le(fmt.len() as u32));
    body.extend_from_slice(&fmt);
    body.extend_from_slice(b"data");
    body.extend_from_slice(&u32le(data_len));
    // 常量 0.25 的 16-bit PCM（与 `#287` 的夹具同形）。
    for _ in 0..FRAMES {
        body.extend_from_slice(&8_192i16.to_le_bytes());
    }
    if let Some((start, end, count)) = smpl {
        let mut payload = Vec::new();
        payload.extend_from_slice(&u32le(0)); // dwManufacturer
        payload.extend_from_slice(&u32le(0)); // dwProduct
        payload.extend_from_slice(&u32le(20_833)); // dwSamplePeriod (48 kHz)
        payload.extend_from_slice(&u32le(60)); // dwMIDIUnityNote
        payload.extend_from_slice(&u32le(0)); // dwMIDIPitchFraction
        payload.extend_from_slice(&u32le(0)); // dwSMPTEFormat
        payload.extend_from_slice(&u32le(0)); // dwSMPTEOffset
        payload.extend_from_slice(&u32le(count)); // dwNumSampleLoops
        payload.extend_from_slice(&u32le(0)); // dwSamplerData
        payload.extend_from_slice(&u32le(0)); // dwCuePointID
        payload.extend_from_slice(&u32le(0)); // dwType（0 = 正向）
        payload.extend_from_slice(&u32le(start)); // dwStart
        payload.extend_from_slice(&u32le(end)); // dwEnd（含）
        payload.extend_from_slice(&u32le(0)); // dwFraction
        payload.extend_from_slice(&u32le(0)); // dwPlayCount
        body.extend_from_slice(b"smpl");
        body.extend_from_slice(&u32le(payload.len() as u32));
        body.extend_from_slice(&payload);
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&u32le(body.len() as u32));
    out.extend_from_slice(&body);
    out
}

/// 判据 ①：`smpl` 的 `dwEnd`（**含**）⇒ `SampleLoop.end` **＋1**（半开）。
#[test]
fn the_smpl_loop_is_read_and_its_end_becomes_exclusive() {
    let scratch = Scratch::new("read");
    let path = scratch.write("loop.wav", &wav_bytes(Some((120, 359, 1))));
    let (sample, frames, describe) = read_sample_file(&path).expect("带 smpl 的 WAV 必须能读");
    println!(
        "[app/#331-smpl] loop_points={:?} frames={frames} describe={describe}",
        sample.loop_points
    );
    assert_eq!(frames, FRAMES as u64, "解码本身不受 `smpl` 影响");
    let found = sample.loop_points.expect("⭐ 必须读出 smpl 的 loop");
    assert_eq!(
        (found.start, found.end),
        (120, 360),
        "⭐ `dwStart=120` 原样；`dwEnd=359`（含）⇒ 半开上界 **360**（⛔ 不是 359，差一会丢一个样本）"
    );
}

/// 判据 ②：没有 `smpl` 块 ⇒ `None`（⛔ 不是全曲循环、⛔ 不是报错）。
#[test]
fn a_wav_without_smpl_reports_no_loop() {
    let scratch = Scratch::new("noloop");
    let path = scratch.write("plain.wav", &wav_bytes(None));
    let (sample, frames, _) = read_sample_file(&path).expect("普通 WAV 必须能读");
    println!(
        "[app/#331-smpl] 无 smpl: loop_points={:?} frames={frames}",
        sample.loop_points
    );
    assert_eq!(frames, FRAMES as u64);
    assert_eq!(
        sample.loop_points, None,
        "⭐ 没有 `smpl` ⇒ `None`（引擎随后沿用 `#311` 的缺省窗口，⛔ 不是「不循环」）"
    );
}

/// 判据 ③：`dwNumSampleLoops=0` ⇒ `None`（块在、但里面没有 loop）。
#[test]
fn a_smpl_block_with_zero_loops_reports_no_loop() {
    let scratch = Scratch::new("zero");
    let path = scratch.write("zero.wav", &wav_bytes(Some((120, 359, 0))));
    let (sample, _, _) = read_sample_file(&path).expect("WAV 必须能读");
    println!("[app/#331-smpl] 零 loop: {:?}", sample.loop_points);
    assert_eq!(sample.loop_points, None, "`dwNumSampleLoops=0` ⇒ 没有 loop");
}

/// 判据 ④：`dwEnd < dwStart`（损坏/写错）⇒ `None`（⛔ 不造退化窗口）。
#[test]
fn a_degenerate_smpl_loop_reports_no_loop() {
    let scratch = Scratch::new("degenerate");
    let path = scratch.write("degenerate.wav", &wav_bytes(Some((300, 200, 1))));
    let (sample, _, _) = read_sample_file(&path).expect("WAV 必须能读");
    println!("[app/#331-smpl] 退化 loop: {:?}", sample.loop_points);
    assert_eq!(
        sample.loop_points, None,
        "`dwEnd < dwStart` ⇒ 不可用（读盘期就丢掉，⛔ 让引擎去判退化窗口）"
    );
}

/// 判据 ⑤：`smpl` 块被**截断** ⇒ 解码照常成功、`loop_points = None`。
///
/// 这条是"元数据坏了不该毁掉音频"的护栏：`smpl` 只是可选元数据，它的损坏
/// ⛔ 不该让整份采样变成"读不出来"（那会走到 `in-memory-sine` 回退，用户听到
/// 完全错误的声音）。
#[test]
fn a_broken_smpl_block_does_not_break_decoding() {
    let scratch = Scratch::new("broken");
    let mut bytes = wav_bytes(Some((120, 359, 1)));
    // 把 `smpl` 的 chunk 尺寸改成一个"远超实际数据"的数 ⇒ 读取时 `read_exact` 失败。
    let position = bytes
        .windows(4)
        .position(|window| window == b"smpl")
        .expect("夹具必须含 smpl");
    bytes[position + 4..position + 8].copy_from_slice(&u32le(9_999));
    let path = scratch.write("broken.wav", &bytes);
    let (sample, frames, describe) = read_sample_file(&path).expect("截断的 smpl ⛔ 不该毁掉解码");
    println!(
        "[app/#331-smpl] 截断 smpl: loop_points={:?} frames={frames} describe={describe}",
        sample.loop_points
    );
    assert_eq!(frames, FRAMES as u64, "PCM 必须照常解出来");
    assert_eq!(
        sample.loop_points, None,
        "截断的 `smpl` ⇒ 当作没有（⛔ 不 panic、⛔ 不猜一个窗口）"
    );
}
