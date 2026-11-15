//! ⭐ 采样预算探针（2026-11-15，Wang 裁决落地）：
//!
//! 1. 缺省文件上限 512 → 1024：600 个样本文件的乐器（> 旧缺省、< 新缺省；
//!    外部实测 Salamander Grand 需 641 份）在缺省下必须**全量装载、零静默**。
//! 2. 超过新缺省（1030 个文件）：缺省装载必须截断，且回执**响亮**——
//!    命中上限名＋值（`maxSampleFiles` = 1024）、被跳过文件数、因此静音的
//!    区域数、改法文本四要素齐全，并发出 `sfzSampleBudgetTruncated` 警告。
//! 3. 帧数上限路径：显式传小 `maxSampleFrames`，同样截断且响亮。
//!
//! 夹具 WAV 每份只有数帧，1030 份的生成与装载开销都在秒级。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yeban_mcp::domain::sfz::{SampleLimits, load};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;

fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-mcp-sfz-probe-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |delta| delta.as_nanos())
    ));
    std::fs::create_dir_all(&dir).expect("建夹具目录");
    dir
}

/// 写 `count` 个微型 WAV（每份 `frames` 帧）＋一份每文件一条区域的 SFZ。
fn write_fixture(dir: &Path, count: usize, frames: usize) -> PathBuf {
    let mut text = String::new();
    for index in 0..count {
        let name = format!("s{index:04}.wav");
        write_plain_wav(
            dir.join(&name),
            &PcmFormat::float(1, 48_000, 32),
            &PcmBuffer::Float32(vec![0.25; frames]),
        )
        .expect("写夹具 WAV");
        text.push_str(&format!("<region> sample={name} key={}\n", index % 128));
    }
    let sfz = dir.join("fixture.sfz");
    std::fs::write(&sfz, text).expect("写夹具 SFZ");
    sfz
}

fn warning_codes(data: &serde_json::Value) -> Vec<String> {
    data["warnings"]
        .as_array()
        .expect("warnings 是数组")
        .iter()
        .map(|warning| warning["code"].as_str().expect("code 是字符串").to_owned())
        .collect()
}

/// 探针 1：600 个样本文件在缺省预算下全量装载、零静默、零警告。
#[test]
fn six_hundred_sample_files_load_fully_under_the_default_budget() {
    let dir = fixture_dir("p600");
    let sfz = write_fixture(&dir, 600, 8);
    let loaded = load(
        &sfz,
        48_000.0,
        1,
        false,
        SampleLimits::default(),
        &BTreeMap::new(),
    )
    .expect("装载");
    assert_eq!(loaded.sample_sources.len(), 600, "600 份样本必须全读到");
    assert!(
        loaded.sample_failures.is_empty(),
        "不得有读失败: {:?}",
        loaded.sample_failures
    );
    assert!(!loaded.sample_reads_truncated, "不得命中截断");
    assert_eq!(loaded.silent_regions, 0, "不得有静音区域");
    assert_eq!(loaded.budget_silent_regions, 0);
    let data = loaded.response_data();
    assert_eq!(data["sampleTruncation"]["filesSkipped"], 0);
    assert_eq!(data["budgetSilentRegionCount"], 0);
    assert_eq!(
        warning_codes(&data),
        Vec::<String>::new(),
        "全量装载不得发任何警告"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 探针 2：1030 个样本文件在缺省预算下截断 6 份，且回执四要素齐全。
#[test]
fn over_a_thousand_sample_files_truncate_loudly_under_the_default_budget() {
    let dir = fixture_dir("p1030");
    let sfz = write_fixture(&dir, 1030, 8);
    let loaded = load(
        &sfz,
        48_000.0,
        1,
        false,
        SampleLimits::default(),
        &BTreeMap::new(),
    )
    .expect("装载");
    assert!(loaded.sample_reads_truncated, "必须命中截断");
    assert_eq!(loaded.sample_sources.len(), 1024, "恰好读满新缺省上限");
    assert_eq!(loaded.sample_truncation.files_at_file_limit, 6);
    assert_eq!(loaded.budget_silent_regions, 6, "6 条区域因此静音");
    assert_eq!(loaded.silent_regions, 6);
    let data = loaded.response_data();
    assert_eq!(data["sampleTruncation"]["filesSkipped"], 6);
    assert_eq!(
        data["sampleTruncation"]["hits"][0]["limit"],
        "maxSampleFiles"
    );
    assert_eq!(data["sampleTruncation"]["hits"][0]["limitValue"], 1024);
    assert_eq!(data["budgetSilentRegionCount"], 6);
    assert_eq!(
        warning_codes(&data),
        ["sfzSampleBudgetTruncated"],
        "只发预算截断警告（文件都在盘上，不得混发外取提示）"
    );
    let warning = &data["warnings"][0];
    assert_eq!(warning["filesSkipped"], 6);
    assert_eq!(warning["filesSkippedByFileLimit"], 6);
    assert_eq!(warning["silentRegions"], 6);
    let hint = warning["hint"].as_str().expect("hint 是字符串");
    assert!(hint.contains("maxSampleFiles"), "改法须点名参数: {hint}");
    assert!(hint.contains("1024"), "提示须点名上限值: {hint}");
    assert!(hint.contains("`0` = 不限"), "提示须给出不限的开法: {hint}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 探针 3：显式小帧数上限（3 份 × 480 帧、预算 1000 帧）截断第 3 份且响亮。
#[test]
fn a_small_explicit_frame_budget_truncates_loudly() {
    let dir = fixture_dir("frames");
    let sfz = write_fixture(&dir, 3, 480);
    let limits = SampleLimits {
        max_files: 0,
        max_frames: 1_000,
    };
    let loaded = load(&sfz, 48_000.0, 1, false, limits, &BTreeMap::new()).expect("装载");
    assert!(loaded.sample_reads_truncated, "必须命中截断");
    assert_eq!(loaded.sample_sources.len(), 2, "前两份 960 帧装得下");
    assert_eq!(loaded.sample_truncation.files_at_frame_limit, 1);
    assert_eq!(loaded.budget_silent_regions, 1);
    let data = loaded.response_data();
    assert_eq!(data["sampleTruncation"]["filesSkipped"], 1);
    assert_eq!(
        data["sampleTruncation"]["hits"][0]["limit"],
        "maxSampleFrames"
    );
    assert_eq!(data["sampleTruncation"]["hits"][0]["limitValue"], 1_000);
    assert_eq!(warning_codes(&data), ["sfzSampleBudgetTruncated"]);
    let hint = data["warnings"][0]["hint"].as_str().expect("hint 是字符串");
    assert!(hint.contains("maxSampleFrames"), "改法须点名参数: {hint}");
    let _ = std::fs::remove_dir_all(&dir);
}
