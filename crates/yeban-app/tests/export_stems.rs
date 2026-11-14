//! `--export-stems` 的判据：**批处理出口与 MCP 分轨是同一份后端**。
//!
//! ## 这条判据要证的句子
//!
//! > `yeban-app --open X.yeban --export-stems dir/` 写出的每个 WAV，
//! > 与 MCP `yeban_export_stems` 对同一份工程导出的**同一批文件** ——
//! > 因为两条路走的是同一份后端（`stems::export` / `stems::write`），
//! > 本文件不抄第二份渲染，只钉**出口**的行为：按轨出文件、不静默丢轨、
//! > 失败响亮且不留半批假交付。
//!
//! ## 怎么变红
//!
//! | 改哪里 | 哪一条红 |
//! | :--- | :--- |
//! | `export_stems` 里绕过 `stems::export` 自己渲染 | `each_track_gets_one_valid_float_wav` |
//! | 跳过的轨不点名 | `each_track_gets_one_valid_float_wav`（`stems + skipped == 7`） |
//! | 目录不存在时拒绝导出（退回 B6 前语义） | `a_missing_directory_is_created_and_export_succeeds` |
//! | 目标已存在时覆盖或跳过 | `an_existing_target_refuses_the_whole_batch` |
#![cfg(feature = "in-process-mcp")]

use std::path::{Path, PathBuf};

use yeban_app::cli::{self, CliError};

/// 每个判据一份独立临时目录（失败时不互相污染，也不会留在仓库里）。
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yeban-export-stems-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 跑一次批处理，返回 stdout 的每一行。
fn run_batch(args: &[&str]) -> Result<Vec<String>, CliError> {
    let argv: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let options = cli::parse(&argv).map_err(|error| CliError::Usage(error.to_string()))?;
    cli::run_batch(&options)
}

/// 从 `exported-stems:` 汇总行里读一个 `key=value` 字段。
fn summary_field<'a>(line: &'a str, key: &str) -> &'a str {
    let needle = format!("{key}=");
    line.split_whitespace()
        .find(|field| field.starts_with(&needle))
        .unwrap_or_else(|| panic!("汇总行里没有 {key}= 字段: {line}"))
        .strip_prefix(&needle)
        .expect("字段有前缀")
}

/// 最小 WAV 头校验：RIFF/WAVE + fmt 块是 IEEE Float + 32-bit + data 块非空。
/// （只读头，不解码 —— 判据钉的是"容器与位深"，不是"音色"。）
fn assert_float32_wav(path: &Path) {
    let bytes = std::fs::read(path).unwrap_or_else(|error| panic!("读 {path:?} 失败: {error}"));
    assert!(
        bytes.len() > 44,
        "WAV 太短（{} 字节）: {path:?}",
        bytes.len()
    );
    assert_eq!(&bytes[0..4], b"RIFF", "必须是 RIFF 容器: {path:?}");
    assert_eq!(&bytes[8..12], b"WAVE", "必须是 WAVE: {path:?}");
    let mut offset = 12;
    let mut format: Option<u16> = None;
    let mut bits: Option<u16> = None;
    let mut data_bytes: Option<u32> = None;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(
            bytes[offset + 4..offset + 8]
                .try_into()
                .expect("chunk 长度 4 字节"),
        ) as usize;
        let body = offset + 8;
        match id {
            b"fmt " => {
                assert!(body + 16 <= bytes.len(), "fmt 块被截断: {path:?}");
                format = Some(u16::from_le_bytes(
                    bytes[body..body + 2].try_into().expect("2 字节"),
                ));
                bits = Some(u16::from_le_bytes(
                    bytes[body + 14..body + 16].try_into().expect("2 字节"),
                ));
            }
            b"data" => data_bytes = Some(size as u32),
            _ => {}
        }
        offset = body + size + (size % 2);
    }
    assert_eq!(format, Some(3), "必须是 IEEE Float（format=3）: {path:?}");
    assert_eq!(bits, Some(32), "必须是 32-bit: {path:?}");
    assert!(
        data_bytes.is_some_and(|size| size > 0),
        "data 块必须非空: {path:?}"
    );
}

/// **核心判据**：demo 工程的每条轨各得一个有效 32-bit Float WAV，主总线被点名跳过。
///
/// 论证顺序：
/// 1. 跑 `--project-sample default --export-stems <已存在的空目录>`；
/// 2. 回执里必须有 `exported-stems:` 汇总行；
/// 3. `stems + skipped == 7`（demo 共 7 条轨：6 条 + 主总线 —— ⛔ 不静默丢轨，
///    分轨数本身由渲染的源节点判定决定，本文件不猜那个数，只钉"账目对得上"）；
/// 4. `renders == stems + 1`（基线 1 次 + 每条分轨 1 次）；
/// 5. 目录里的 `.wav` 文件数 == `stems`，且每个都是有效 32-bit Float WAV；
/// 6. `stem-skipped:` 行里必须点名主总线（`reason=masterBusTrack`）。
#[test]
fn each_track_gets_one_valid_float_wav() {
    let dir = scratch("all-tracks");
    let out = dir.join("stems");
    std::fs::create_dir(&out).expect("建输出目录");
    let lines = run_batch(&[
        "--project-sample",
        "default",
        "--export-stems",
        &out.display().to_string(),
    ])
    .expect("分轨导出必须成功");
    let summary = lines
        .iter()
        .find(|line| line.starts_with("exported-stems:"))
        .expect("回执里必须有 exported-stems: 一行");
    let stems: usize = summary_field(summary, "stems")
        .parse()
        .expect("stems 必须是整数");
    let skipped: usize = summary_field(summary, "skipped")
        .parse()
        .expect("skipped 必须是整数");
    let renders: usize = summary_field(summary, "renders")
        .parse()
        .expect("renders 必须是整数");
    assert_eq!(
        stems + skipped,
        7,
        "分轨 + 跳过必须等于 demo 的全部 7 条轨，汇总行: {summary}"
    );
    assert_eq!(
        renders,
        stems + 1,
        "渲染次数必须是基线 1 次 + 每条分轨 1 次，汇总行: {summary}"
    );
    assert!(
        summary.contains("bitDepth=32"),
        "汇总行必须报 bitDepth=32，汇总行: {summary}"
    );
    let mut wavs: Vec<PathBuf> = std::fs::read_dir(&out)
        .expect("读输出目录")
        .map(|entry| entry.expect("目录项").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "wav"))
        .collect();
    wavs.sort();
    assert_eq!(
        wavs.len(),
        stems,
        "目录里的 .wav 文件数必须等于 stems={stems}"
    );
    assert!(!wavs.is_empty(), "至少要有一条分轨");
    for wav in &wavs {
        assert_float32_wav(wav);
    }
    let skipped_lines: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("stem-skipped:"))
        .collect();
    assert_eq!(
        skipped_lines.len(),
        skipped,
        "stem-skipped: 行数必须等于 skipped={skipped}"
    );
    assert!(
        skipped_lines
            .iter()
            .any(|line| line.contains("reason=masterBusTrack")),
        "跳过的轨里必须点名主总线（reason=masterBusTrack），行: {skipped_lines:?}"
    );
    let stem_lines: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("stem:"))
        .collect();
    assert_eq!(stem_lines.len(), stems, "stem: 行数必须等于 stems={stems}");
}

/// **B6 判据（2026-11-14）**：输出目录不存在 ⇒ 写盘时创建、导出成功。
///
/// B6 前本条钉的是"响亮失败（退出码 5）＋⛔ 不自动创建"；第四份报告 B6 把
/// CLI 口径与 `--export-wav` 对齐（那边父目录不存在从不预检）后随之翻钉：
/// 现在**拒绝**才是红，静默不建目录也是红。
#[test]
fn a_missing_directory_is_created_and_export_succeeds() {
    let dir = scratch("missing-dir");
    let out = dir.join("does-not-exist");
    assert!(!out.exists(), "测试前置：目录必须不存在");
    let lines = run_batch(&[
        "--project-sample",
        "default",
        "--export-stems",
        &out.display().to_string(),
    ])
    .expect("不存在的目录必须被创建并完成导出");
    assert!(out.is_dir(), "导出后目录必须已被创建");
    let summary = lines
        .iter()
        .find(|line| line.starts_with("exported-stems:"))
        .expect("回执里必须有 exported-stems: 一行");
    let stems: usize = summary_field(summary, "stems")
        .parse()
        .expect("stems 必须是整数");
    assert!(stems > 0, "必须真导出分轨，汇总行: {summary}");
    let wav_count = std::fs::read_dir(&out)
        .expect("读新建目录")
        .map(|entry| entry.expect("目录项").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "wav"))
        .count();
    assert_eq!(
        wav_count, stems,
        "新建目录里的 .wav 数必须等于 stems={stems}"
    );
}

/// **响亮失败判据**：目标文件已存在时整批拒绝（退出码 5），且不产出其它分轨。
///
/// 文件名不猜：先跑一次成功导出取真实文件名，再预建其中第一个，换新目录重跑。
#[test]
fn an_existing_target_refuses_the_whole_batch() {
    let dir = scratch("collision");
    let first_out = dir.join("first");
    std::fs::create_dir(&first_out).expect("建第一份输出目录");
    let first_lines = run_batch(&[
        "--project-sample",
        "default",
        "--export-stems",
        &first_out.display().to_string(),
    ])
    .expect("第一份导出必须成功");
    let first_wav = std::fs::read_dir(&first_out)
        .expect("读第一份输出目录")
        .map(|entry| entry.expect("目录项").path())
        .find(|path| path.extension().is_some_and(|ext| ext == "wav"))
        .expect("第一份导出必须至少有一个 WAV");
    let file_name = first_wav.file_name().expect("WAV 有文件名").to_owned();
    let _ = first_lines;

    let out = dir.join("second");
    std::fs::create_dir(&out).expect("建第二份输出目录");
    std::fs::write(out.join(&file_name), b"dummy").expect("预建冲突文件");
    let error = run_batch(&[
        "--project-sample",
        "default",
        "--export-stems",
        &out.display().to_string(),
    ])
    .expect_err("目标已存在时整批必须失败");
    assert_eq!(error.exit_code(), 5, "导出失败是退出码 5（EXIT_EXPORT）");
    assert!(
        error.to_string().contains("已存在"),
        "错误文案必须说清是目标已存在，实为: {error}"
    );
    let remaining: Vec<PathBuf> = std::fs::read_dir(&out)
        .expect("读第二份输出目录")
        .map(|entry| entry.expect("目录项").path())
        .collect();
    assert_eq!(
        remaining,
        vec![out.join(&file_name)],
        "整批拒绝后目录里只能剩预建的那一个文件，不能多出其它分轨"
    );
}

/// **组合判据**：`--apply-ops` 改完的工程再 `--export-stems`，分轨里用的是新名。
///
/// 这是 Wang 的工作流（批量改 → 取分轨）：同一命令里编辑先于导出，因此分轨文件
/// 名里必须出现改后的轨名（清洗后），而不是旧名。
#[test]
fn stems_after_apply_ops_use_the_edited_names() {
    let dir = scratch("edit-then-stems");
    let out = dir.join("stems");
    std::fs::create_dir(&out).expect("建输出目录");
    // T1（"鼓"）改成 ASCII 名 "Kick" —— 清洗后文件名里能直接读出来。
    let ops = dir.join("ops.json");
    std::fs::write(
        &ops,
        r#"{"trackId": "01J8Z5Q0R7K3M9X2V4B6N8P0T1", "ops": [{"kind": "setTrackName", "trackId": "01J8Z5Q0R7K3M9X2V4B6N8P0T1", "name": "Kick"}]}"#,
    )
    .expect("写 ops 文件");
    let saved = dir.join("edited.yeban");
    let lines = run_batch(&[
        "--project-sample",
        "default",
        "--apply-ops",
        &ops.display().to_string(),
        "--save-as",
        &saved.display().to_string(),
        "--export-stems",
        &out.display().to_string(),
    ])
    .expect("改名 + 分轨必须成功");
    assert!(
        lines.iter().any(|line| line.starts_with("applied-ops:")),
        "回执里必须有 applied-ops: 一行"
    );
    assert!(
        lines.iter().any(|line| line.starts_with("exported-stems:")),
        "回执里必须有 exported-stems: 一行"
    );
    let names: Vec<String> = std::fs::read_dir(&out)
        .expect("读输出目录")
        .map(|entry| {
            entry
                .expect("目录项")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(
        names.iter().any(|name| name.contains("Kick")),
        "分轨文件名里必须出现改后的轨名 Kick，实为: {names:?}"
    );
    assert!(
        !names
            .iter()
            .any(|name| name.contains("track.wav") && name.starts_with("0001.")),
        "第一条分轨不能再是旧名清洗出的 0001.track.wav，实为: {names:?}"
    );
}
