//! ⭐ 阶段 2 端到端夹具（设计 `docs/ledger/sfz-core-notes.md` §14.4）：
//! 记法适配 (b)/(c) 在 **app 装载路径**（`sfz_load`）上的三条必做判据。
//!
//! | 判据 | 内容 |
//! | :--- | :--- |
//! | [`yamaha_style_library_is_detected_and_sounds_at_the_written_notes`] | ① Yamaha 式（`key=c3` 意指 60）＋ WAV 嵌入根音 ⇒ (b) 命中 +12、出声键位 60..=65、回执带证据计数 |
//! | [`sso_style_drum_kit_misses_detection_and_manual_override_fixes_it`] | ② SSO 式鼓组（嵌入根音缺省/散列）⇒ (b) 正确不命中；(c) 覆盖 +12 后 60..=65 触发 |
//! | [`plain_library_loads_unchanged`] | ③ 无移位普通 SFZ（数字键位、无 `smpl`）⇒ 移位 0、无响亮行、键位照旧（回归） |
//!
//! WAV 夹具是手拼字节（`fmt` + `data` + `smpl`，`dwMIDIUnityNote` 在 payload
//! 偏移 12），形态照 `tests/sfz_wav_smpl_loop.rs` 的既有夹具。

use std::path::PathBuf;

use yeban_app::sfz_load::{decode_sfz_plan, decode_sfz_plan_with_override, load_sfz_adapted};
use yeban_engine::sfz_instrument::NotationShiftSource;
use yeban_model::EntityId;

const HZ: f32 = 48_000.0;

/// 临时目录（`Drop` 删掉自己）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-octave-notation-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, bytes).expect("写夹具文件");
        path
    }

    fn write_text(&self, name: &str, text: &str) -> PathBuf {
        self.write(name, text.as_bytes())
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

/// 拼一份 48 kHz / 16-bit / 单声道 WAV（480 帧常量 PCM）。
/// `unity_note = Some(n)` ⇒ 带 `smpl` 块且 `dwMIDIUnityNote = n`（0 条 loop）。
fn wav_bytes(unity_note: Option<u32>) -> Vec<u8> {
    const FRAMES: usize = 480;
    let data_len = (FRAMES * 2) as u32;
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&u16le(1)); // PCM
    fmt.extend_from_slice(&u16le(1)); // 单声道
    fmt.extend_from_slice(&u32le(48_000));
    fmt.extend_from_slice(&u32le(48_000 * 2));
    fmt.extend_from_slice(&u16le(2));
    fmt.extend_from_slice(&u16le(16));

    let mut body = Vec::new();
    body.extend_from_slice(b"WAVE");
    body.extend_from_slice(b"fmt ");
    body.extend_from_slice(&u32le(fmt.len() as u32));
    body.extend_from_slice(&fmt);
    body.extend_from_slice(b"data");
    body.extend_from_slice(&u32le(data_len));
    for _ in 0..FRAMES {
        body.extend_from_slice(&8_192i16.to_le_bytes());
    }
    if let Some(root) = unity_note {
        let mut payload = Vec::new();
        payload.extend_from_slice(&u32le(0)); // dwManufacturer
        payload.extend_from_slice(&u32le(0)); // dwProduct
        payload.extend_from_slice(&u32le(20_833)); // dwSamplePeriod
        payload.extend_from_slice(&u32le(root)); // ⭐ dwMIDIUnityNote（偏移 12）
        payload.extend_from_slice(&u32le(0)); // dwMIDIPitchFraction
        payload.extend_from_slice(&u32le(0)); // dwSMPTEFormat
        payload.extend_from_slice(&u32le(0)); // dwSMPTEOffset
        payload.extend_from_slice(&u32le(0)); // dwNumSampleLoops
        payload.extend_from_slice(&u32le(0)); // dwSamplerData
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

/// 六个音名键位（IPN 48..=53）：两份夹具共用同一份写出键位。
const NOTE_NAMES: [&str; 6] = ["c3", "c#3", "d3", "d#3", "e3", "f3"];

fn sfz_text(sample_prefix: &str) -> String {
    NOTE_NAMES
        .iter()
        .enumerate()
        .map(|(index, name)| format!("<region> sample={sample_prefix}{index}.wav key={name}\n"))
        .collect()
}

#[test]
fn yamaha_style_library_is_detected_and_sounds_at_the_written_notes() {
    let scratch = Scratch::new("yamaha");
    for index in 0..6 {
        scratch.write(
            &format!("k{index}.wav"),
            &wav_bytes(Some(60 + index as u32)),
        );
    }
    let sfz = scratch.write_text("yamaha.sfz", &sfz_text("k"));

    let (report, _instrument) = load_sfz_adapted(&sfz, Some(60), HZ, None).expect("装载必须成功");
    println!("[octave/yamaha] lines={:?}", report.notation_lines());
    assert_eq!(report.notation_shift, 12, "⭐ (b) 必须命中 +12");
    assert_eq!(report.notation_source, NotationShiftSource::AutoDetected);
    let detection = report.notation.as_ref().expect("(b) 跑过 ⇒ 报告在场");
    assert_eq!(detection.evidence_count, 6);
    assert_eq!(detection.sampled_region_count, 6);
    assert_eq!(detection.matching_count(), 6);
    assert_eq!(
        report.key_range,
        Some((60, 65)),
        "移位后覆盖范围必须是 60..=65"
    );
    // 出声键位：探测 60 有能量（探测在适配后做）。
    let probe = report.probe.expect("带了探测音");
    assert!(
        probe.energy > 0.0,
        "⭐ 按 60 必须出声（energy={})",
        probe.energy
    );
    // 响亮行带证据计数。
    let lines = report.notation_lines();
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].contains("+12") && lines[0].contains("6/6"),
        "响亮行：{}",
        lines[0]
    );

    // 移位前的写出键 48 不再出声。
    let (report48, _) = load_sfz_adapted(&sfz, Some(48), HZ, None).expect("装载必须成功");
    assert_eq!(
        report48.probe.expect("带了探测音").energy,
        0.0,
        "⭐ 48 不许再出声"
    );

    // 摊平路径同结论：region 接受 60..=65、不接受 48..=53。
    let (plan, decode_report) = decode_sfz_plan(&sfz, HZ).expect("摊平必须成功");
    assert_eq!(decode_report.notation_shift, 12);
    let regions = plan.prepared().regions();
    assert_eq!(regions.len(), 6);
    for (index, region) in regions.iter().enumerate() {
        let note = 60 + index as u8;
        assert!(region.accepts(note, 100), "region {index} 必须接受 {note}");
        assert!(
            !region.accepts(note - 12, 100),
            "region {index} 不许再接受 {}",
            note - 12
        );
    }
}

#[test]
fn sso_style_drum_kit_misses_detection_and_manual_override_fixes_it() {
    let scratch = Scratch::new("sso");
    // SSO 式：嵌入根音是缺省值/散列（真实 SSO 鼓组的形状）。
    let roots = [60, 60, 62, 55, 60, 48];
    for (index, root) in roots.iter().enumerate() {
        scratch.write(&format!("d{index}.wav"), &wav_bytes(Some(*root)));
    }
    let sfz = scratch.write_text("sso.sfz", &sfz_text("d"));

    // (b) 正确不命中：不移位、原因在报告里、无疑似行（delta 无主导方向）。
    let (report, _) = load_sfz_adapted(&sfz, None, HZ, None).expect("装载必须成功");
    assert_eq!(report.notation_shift, 0, "⭐ 鼓组不许被自动移位");
    assert_eq!(report.notation_source, NotationShiftSource::None);
    let detection = report.notation.as_ref().expect("未命中也必须有报告");
    assert_eq!(
        detection.miss_reason,
        Some(yeban_sfz::NotationMissReason::NoConsistentOctave)
    );
    assert_eq!(detection.suspected_shift, None);
    assert!(report.notation_lines().is_empty(), "无事可报时不产行");
    assert_eq!(report.key_range, Some((48, 53)));

    // (c) 覆盖 +12：60/61/62（乃至 60..=65）触发，报告记手动来源、检测停跑。
    let (report, _) = load_sfz_adapted(&sfz, None, HZ, Some(12)).expect("装载必须成功");
    assert_eq!(report.notation_shift, 12);
    assert_eq!(report.notation_source, NotationShiftSource::ManualOverride);
    assert_eq!(report.notation, None, "(c) 在场 ⇒ (b) 停跑");
    assert_eq!(report.key_range, Some((60, 65)));
    assert_eq!(report.notation_lines().len(), 1);

    let (plan, decode_report) =
        decode_sfz_plan_with_override(&sfz, HZ, Some(12)).expect("摊平必须成功");
    assert_eq!(decode_report.notation_shift, 12);
    for (index, region) in plan.prepared().regions().iter().enumerate() {
        let note = 60 + index as u8;
        assert!(
            region.accepts(note, 100),
            "⭐ 覆盖后 region {index} 必须接受 {note}（Wang 亲测的 60/61/62 触发）"
        );
    }
}

#[test]
fn plain_library_loads_unchanged() {
    let scratch = Scratch::new("plain");
    for index in 0..6 {
        scratch.write(&format!("p{index}.wav"), &wav_bytes(None));
    }
    let text: String = (0..6)
        .map(|index| format!("<region> sample=p{index}.wav key={}\n", 60 + index))
        .collect();
    let sfz = scratch.write_text("plain.sfz", &text);

    let (report, _) = load_sfz_adapted(&sfz, Some(60), HZ, None).expect("装载必须成功");
    assert_eq!(report.notation_shift, 0);
    assert_eq!(report.notation_source, NotationShiftSource::None);
    assert_eq!(report.declared_shifts, Vec::new());
    assert_eq!(report.key_range, Some((60, 65)));
    assert!(report.notation_lines().is_empty(), "普通乐器不产响亮行");
    assert!(
        report.probe.expect("带了探测音").energy > 0.0,
        "普通乐器按 60 照常出声"
    );

    let (plan, decode_report) = decode_sfz_plan(&sfz, HZ).expect("摊平必须成功");
    assert_eq!(decode_report.notation_shift, 0);
    for (index, region) in plan.prepared().regions().iter().enumerate() {
        assert!(region.accepts(60 + index as u8, 100));
    }
}
