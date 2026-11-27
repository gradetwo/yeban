//! ⭐ T-FIX-945：SFZ 装载异常矩阵。
//!
//! 四类异常都必须**响亮**且不静默降级：
//! 1. 缺失 SFZ 文件 ⇒ 领域 `IO_ERROR`，带 `reason` / `path` / `ioError`；
//! 2. 损坏 SFZ（非法 UTF-8 / 未闭合段头）⇒ `INVALID_PARAMETER_RANGE` + `sfzParseFailed`；
//! 3. 越界 opcode：
//!    - 硬越界（`lovel=128`）⇒ 解析器明确拒绝，带 `IntegerOutOfRange`；
//!    - 规范允许的装载期钳位（`pan=200`）⇒ 装载成功，但 `warnings[]` 必须逐条
//!      透出 `sfzParserWarning`，不能被 `loaded:true` 静默吞掉；
//! 4. 超预算样本集 ⇒ 装载成功，但 `sfzSampleBudgetTruncated` 必须点名上限、
//!    跳过文件数与因此静音的区域数。
//!
//! 另含缺失样本文件的非阻断告警（`sfzSamplesUnreadable`）作为矩阵对照。
//!
//! 红对照：`pan=200` 的 `sfzParserWarning` 在未透出解析器告警的旧实现上红。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yeban_mcp::domain::error::Fault;
use yeban_mcp::domain::sfz::{SampleLimits, load};
use yeban_mcp::tools::ErrorCode;
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-sfz-anomaly-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |delta| delta.as_nanos())
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn write(&self, name: &str, body: &str) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, body).expect("写 SFZ 夹具");
        path
    }

    fn write_wav(&self, name: &str, frames: usize) {
        write_plain_wav(
            self.dir.join(name),
            &PcmFormat::float(1, 48_000, 32),
            &PcmBuffer::Float32(vec![0.25; frames]),
        )
        .expect("写样本 WAV");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn load_error(path: &Path) -> (ErrorCode, serde_json::Value) {
    match load(
        path,
        48_000.0,
        1,
        false,
        SampleLimits::default(),
        &BTreeMap::new(),
    )
    .expect_err("异常输入必须失败")
    {
        Fault::Domain { code, data, .. } => (code, data.expect("异常必须带结构化 data")),
        Fault::Impl { error } => panic!("必须是领域错误，实际是实现级错误: {error:?}"),
    }
}

fn warning_codes(data: &serde_json::Value) -> Vec<String> {
    data["warnings"]
        .as_array()
        .expect("warnings 是数组")
        .iter()
        .map(|warning| warning["code"].as_str().expect("code 是字符串").to_owned())
        .collect()
}

fn warning<'a>(data: &'a serde_json::Value, code: &str) -> &'a serde_json::Value {
    data["warnings"]
        .as_array()
        .expect("warnings 是数组")
        .iter()
        .find(|warning| warning["code"] == code)
        .unwrap_or_else(|| panic!("必须出现 `{code}` 警告: {data}"))
}

/// ① 缺失 SFZ 文件：领域 `IO_ERROR`，结构化点名路径与 I/O 类别。
#[test]
fn a_missing_sfz_is_a_loud_io_error() {
    let scratch = Scratch::new("missing-sfz");
    let absent = scratch.dir.join("does-not-exist.sfz");
    let (code, data) = load_error(&absent);
    assert_eq!(code, ErrorCode::IoError, "{data}");
    assert_eq!(data["reason"], "sfzReadFailed", "{data}");
    assert_eq!(data["path"], absent.display().to_string(), "{data}");
    assert!(
        data["ioError"]
            .as_str()
            .is_some_and(|kind| !kind.is_empty()),
        "必须点名 I/O 类别: {data}"
    );
}

/// ② 损坏 SFZ：非法 UTF-8 与未闭合段头都必须明确拒绝，不得替换字符“发明输入”。
#[test]
fn a_corrupt_sfz_is_a_loud_parse_error() {
    let scratch = Scratch::new("corrupt");
    let binary = scratch.dir.join("binary.sfz");
    std::fs::write(&binary, [0x3c, 0x72, 0x3e, 0xff, 0xfe]).expect("写二进制夹具");
    let (code, data) = load_error(&binary);
    assert_eq!(code, ErrorCode::InvalidParameterRange, "{data}");
    assert_eq!(data["reason"], "sfzNotUtf8", "{data}");
    assert_eq!(data["path"], binary.display().to_string(), "{data}");
    assert!(
        data["validUpTo"].as_u64().is_some(),
        "必须点名合法前缀长度: {data}"
    );

    let truncated = scratch.write("truncated.sfz", "<region sample=a.wav");
    let (code, data) = load_error(&truncated);
    assert_eq!(code, ErrorCode::InvalidParameterRange, "{data}");
    assert_eq!(data["reason"], "sfzParseFailed", "{data}");
    assert!(
        data["parser"]
            .as_str()
            .is_some_and(|parser| parser.contains("UnterminatedHeader")),
        "必须点名解析器拒绝原因: {data}"
    );
}

/// ③ 越界 opcode：硬越界明确拒绝；规范允许的钳位必须逐条透出解析器告警。
///
/// 红对照点在第二半：旧实现只回 `loaded:true`，`warnings[]` 里没有
/// `sfzParserWarning`，本判据会红在 `warning()` 的查找上。
#[test]
fn out_of_range_opcodes_are_loud_in_both_modes() {
    let scratch = Scratch::new("opcode-range");

    let hard = scratch.write("hard.sfz", "<region> sample=a.wav lovel=128\n");
    let (code, data) = load_error(&hard);
    assert_eq!(code, ErrorCode::InvalidParameterRange, "{data}");
    assert_eq!(data["reason"], "sfzParseFailed", "{data}");
    let parser = data["parser"].as_str().expect("parser 是字符串");
    assert!(
        parser.contains("IntegerOutOfRange"),
        "硬越界必须点名 IntegerOutOfRange: {data}"
    );
    assert!(parser.contains("lovel"), "必须点名 opcode: {data}");

    scratch.write_wav("a.wav", 8);
    let soft = scratch.write("soft.sfz", "<region> sample=a.wav pan=200\n");
    let loaded = load(
        &soft,
        48_000.0,
        1,
        false,
        SampleLimits::default(),
        &BTreeMap::new(),
    )
    .expect("钳位属于非阻断降级，装载必须成功");
    let data = loaded.response_data();
    let parsed = warning(&data, "sfzParserWarning");
    assert_eq!(
        parsed["display"], "line 1: `pan` value 200 out of range, clamped to 100",
        "钳位告警必须逐字可查: {data}"
    );
    assert!(
        parsed["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("ClampedStaticValue")),
        "detail 必须保留结构化变体: {data}"
    );
    assert_eq!(
        warning_codes(&data),
        ["sfzParserWarning"],
        "样本可读时只应有一条解析器告警: {data}"
    );
}

/// ④ 超预算样本集：装载成功但必须点名命中哪条上限、跳过几个、几个区域静音。
#[test]
fn an_over_budget_sample_set_truncates_loudly() {
    let scratch = Scratch::new("budget");
    for index in 0..3 {
        scratch.write_wav(&format!("s{index}.wav"), 8);
    }
    let sfz = scratch.write(
        "budget.sfz",
        "<region> sample=s0.wav key=60\n\
         <region> sample=s1.wav key=61\n\
         <region> sample=s2.wav key=62\n",
    );
    let limits = SampleLimits {
        max_files: 2,
        max_frames: 0,
    };
    let loaded = load(&sfz, 48_000.0, 1, false, limits, &BTreeMap::new()).expect("装载");
    assert!(loaded.sample_reads_truncated, "必须命中文件数上限");
    assert_eq!(loaded.sample_sources.len(), 2);
    assert_eq!(loaded.budget_silent_regions, 1);
    let data = loaded.response_data();
    assert_eq!(warning_codes(&data), ["sfzSampleBudgetTruncated"], "{data}");
    let truncated = warning(&data, "sfzSampleBudgetTruncated");
    assert_eq!(truncated["filesSkipped"], 1, "{data}");
    assert_eq!(truncated["filesSkippedByFileLimit"], 1, "{data}");
    assert_eq!(truncated["silentRegions"], 1, "{data}");
    assert_eq!(truncated["hits"][0]["limit"], "maxSampleFiles", "{data}");
    assert_eq!(truncated["hits"][0]["limitValue"], 2, "{data}");
}

/// ①' 缺失样本文件：装载不失败，但必须发 `sfzSamplesUnreadable`，不许假装有声。
#[test]
fn a_missing_sample_file_is_a_loud_non_blocking_warning() {
    let scratch = Scratch::new("missing-sample");
    let sfz = scratch.write("missing-sample.sfz", "<region> sample=gone.wav key=60\n");
    let loaded = load(
        &sfz,
        48_000.0,
        1,
        false,
        SampleLimits::default(),
        &BTreeMap::new(),
    )
    .expect("缺样本不阻断 SFZ 装载");
    let data = loaded.response_data();
    assert_eq!(data["sampleFilesRead"], false, "{data}");
    assert_eq!(warning_codes(&data), ["sfzSamplesUnreadable"], "{data}");
    let missing = warning(&data, "sfzSamplesUnreadable");
    assert_eq!(missing["count"], 1, "{data}");
    assert_eq!(missing["allSamplesUnreadable"], true, "{data}");
}
