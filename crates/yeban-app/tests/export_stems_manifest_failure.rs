//! **T-FIX-501**：manifest 写失败时，回执必须说明**分轨已经完整落盘**。
//!
//! ## 病
//!
//! `--export-stems` 的顺序是「先逐条渲完并落盘 N 条分轨，再写
//! `<dir>/stems-manifest.json`」。manifest 写失败（目录只读 / 磁盘满 / 落点被占）时，
//! 修前的错误只有 `write_project_atomic` 的 I/O 原因 —— **只字不提 N 条分轨已经在盘上**。
//! 用户从回执里分不清「分轨已齐、只需补写 manifest」和「整批作废」，最可能的反应是
//! 重跑整批渲染（大工程数分钟到数十分钟全白付）。
//!
//! ## 判据（每条修前必红 / 修后必绿）
//!
//! | # | 断言 | 证什么 |
//! | :--- | :--- | :--- |
//! | ① 文案说明已落盘 | 失败回执的 `reason` 含「{N} 条分轨已完整落盘」，N = **实测**盘上的 wav 数 | 回执不再把"只差 manifest"说成"整批失败" |
//! | ② 给出续跑口径 | `reason` 含 `rewriteManifestOnly`（= 只补写 manifest） | 用户知道下一步只需写一个文件，⛔ 不必重跑渲染 |
//! | ③ 分轨真的完整 | 每个 wav 都存在且 > 44 字节（RIFF 头 + 非空 data） | 文案不是空口安慰：字节确实在盘上 |
//! | ④ 错误码仍是 IO_ERROR | `code == "IO_ERROR"` | 不因为"部分成功"就把失败伪装成成功 |
//!
//! ## 怎么造出"manifest 写失败"
//!
//! 把 `stems-manifest.json` **建成同名目录** —— `write_project_atomic` 必然失败，
//! 跨平台且不依赖只读权限（判据常在 root 下跑，`chmod` 挡不住写入）。分轨落点
//! 本身不受影响（manifest 不走"已存在即拒绝"的守卫）。

#![cfg(feature = "in-process-mcp")]

use std::path::{Path, PathBuf};

use yeban_app::cli::{self, CliError};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-iss-stem-manifest-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

fn run_batch(args: &[&str]) -> Result<Vec<String>, CliError> {
    let argv: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let options = cli::parse(&argv).map_err(|error| CliError::Usage(error.to_string()))?;
    cli::run_batch(&options)
}

/// 盘上真的存在的分轨（实测，⛔ 不按请求推断）。
fn landed_stems(dir: &Path) -> Vec<PathBuf> {
    let mut wavs: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("读输出目录 {dir:?} 失败: {error}"))
        .map(|entry| entry.expect("目录项").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "wav"))
        .collect();
    wavs.sort();
    wavs
}

/// ⭐ 主判据：manifest 失败的回执必须带「已落盘 N 条」与"只补写 manifest"的续跑口径。
#[test]
fn a_manifest_write_failure_says_the_stems_are_already_on_disk() {
    let dir = scratch("partial");
    let out = dir.join("stems");
    std::fs::create_dir_all(&out).expect("建输出目录");
    // 占住 manifest 落点（同名目录 ⇒ 原子写必失败）。
    std::fs::create_dir(out.join("stems-manifest.json")).expect("占住 manifest 落点");
    assert!(
        !out.join("stems-manifest.json").is_file(),
        "夹具前提：manifest 落点必须是目录，不是文件"
    );

    let error = run_batch(&[
        "--project-sample",
        "default",
        "--export-stems",
        &out.display().to_string(),
    ])
    .expect_err("manifest 落点被目录占住 ⇒ 整次导出必须失败（⛔ 不许静默成功）");

    let CliError::ExportStems { code, reason, .. } = error else {
        panic!("必须是 ExportStems 失败档: {error:?}");
    };

    // ③ 分轨实测：真的在盘上、真的有字节。
    let landed = landed_stems(&out);
    assert!(
        !landed.is_empty(),
        "夹具前提：分轨必须已经落盘（否则测不到\"部分成功\"）"
    );
    for path in &landed {
        let len = std::fs::metadata(path)
            .unwrap_or_else(|error| panic!("读 {path:?} 失败: {error}"))
            .len();
        assert!(
            len > 44,
            "已落盘的分轨必须是非空 RIFF（{path:?} 只有 {len} 字节）"
        );
    }

    // ④ 失败档位不变。
    assert_eq!(
        code, "IO_ERROR",
        "manifest 写失败仍是 IO_ERROR，实测：{code}"
    );

    // ① 文案必须点名"已落盘 N 条"，N 与实测一致。
    let expected = format!("{} 条分轨已完整落盘", landed.len());
    assert!(
        reason.contains(&expected),
        "回执必须说明「{expected}」（实测盘上 {} 条 wav）；实际回执：{reason}",
        landed.len()
    );
    assert!(
        reason.contains("manifest"),
        "回执必须点名缺的是 manifest：{reason}"
    );

    // ② 续跑口径：只补写 manifest，⛔ 不是重跑渲染。
    assert!(
        reason.contains("rewriteManifestOnly"),
        "回执必须给出续跑口径 rewriteManifestOnly（只补写 manifest）：{reason}"
    );
    assert!(
        !reason.contains("整批需重跑"),
        "已落盘 N 条时⛔ 不许说整批需重跑：{reason}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 对照腿：manifest **能**写成功时，一次导出不该产生任何"部分成功"文案。
///
/// （保证 ① 的字符串不是恒真 —— 成功路径上它根本不该出现。）
#[test]
fn a_successful_export_reports_no_partial_manifest_wording() {
    let dir = scratch("ok");
    let out = dir.join("stems");
    std::fs::create_dir_all(&out).expect("建输出目录");

    let lines = run_batch(&[
        "--project-sample",
        "default",
        "--export-stems",
        &out.display().to_string(),
    ])
    .expect("manifest 落点正常 ⇒ 导出必须成功");

    let summary = lines
        .iter()
        .find(|line| line.starts_with("exported-stems:"))
        .expect("回执里必须有 exported-stems: 一行");
    assert!(
        !summary.contains("已完整落盘"),
        "成功路径不许出现\"已完整落盘\"（那是失败时说的）: {summary}"
    );
    assert!(
        out.join("stems-manifest.json").is_file(),
        "成功路径必须真的写出 manifest"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
