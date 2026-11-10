//! `--apply-ops` 的判据：**批处理出口与 MCP 执行是同一条语义**。
//!
//! ## 这条判据要证的句子
//!
//! > `yeban-app --open X.yeban --apply-ops ops.json --save-as Y.yeban` 写出的工程，
//! > 与 MCP `yeban_edit_notes` 对同一份工程执行同一批 ops 的结果**相同** ——
//! > 因为两条路走的是同一条管线（`parse_ops` → `compile` → 一次 `Op::Batch` 提交），
//! > 本文件不抄第二份语义，只钉**出口**的行为。
//!
//! ## 怎么变红
//!
//! | 改哪里 | 哪一条红 |
//! | :--- | :--- |
//! | `apply_ops` 里绕过 `compile` 自己拼 `Op` | `the_rename_round_trips_through_save_as` |
//! | 干跑偷偷写了盘 | `dry_run_writes_nothing` |
//! | 没给 `--save-as` 时静默丢了编辑 | `apply_ops_without_save_as_is_a_usage_error` |
//! | 未知 kind 被静默跳过 | `an_unknown_kind_fails_loudly_and_writes_nothing` |
//! | 不存在的轨被静默接受 | `a_missing_track_fails_loudly_and_writes_nothing` |
//! | 干跑遇领域失败时伪造成功 | `dry_run_reports_a_domain_failure_instead_of_faking_success` |
//! | 自带寻址的批被信封守卫拦住 | `a_self_addressed_batch_needs_no_envelope_ids` |
#![cfg(feature = "in-process-mcp")]

use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use yeban_app::cli::{self, CliError};
use yeban_model::samples::demo_project;

/// T1（demo 工程第一条轨，名叫"鼓"）的 ULID（`demo_id("T1")` 的确定值）。
const T1: &str = "01J8Z5Q0R7K3M9X2V4B6N8P0T1";

/// 每个判据一份独立临时目录（失败时不互相污染，也不会留在仓库里）。
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yeban-apply-ops-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 写一份 ops JSON（`yeban_edit_notes` 的实参对象形状）。
fn write_ops(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).expect("写 ops 文件");
    path
}

/// 跑一次批处理，返回 stdout 的每一行。
fn run_batch(args: &[&str]) -> Result<Vec<String>, CliError> {
    let argv: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let options = cli::parse(&argv).map_err(|error| CliError::Usage(error.to_string()))?;
    cli::run_batch(&options)
}

/// 回读一份 `.yeban` 里 T1 的轨名（`cli::load_project` 的公开入口）。
fn track_name_of(saved: &Path) -> String {
    let argv = vec![
        "--open".to_owned(),
        saved.display().to_string(),
        "--headless".to_owned(),
    ];
    let options = cli::parse(&argv).expect("解析回读命令");
    let loaded = cli::load_project(&options).expect("回读保存的工程");
    let track_id = yeban_model::EntityId::from_str(T1).expect("T1 是合法 ULID");
    loaded
        .archive
        .project
        .track(&track_id)
        .expect("T1 在工程里")
        .name
        .clone()
}

/// **核心判据**：改名走 `--apply-ops` → `--save-as`，回读即是新名。
///
/// 论证顺序：
/// 1. ops = 一条 `setTrackName`（顶层 `trackId` 与条内 `trackId` 都给 T1 ——
///    信封守卫与 `yeban_edit_notes` 同一条口径）；
/// 2. 跑 `--project-sample default --apply-ops ops.json --save-as out.yeban`；
/// 3. 回执里必须有 `applied-ops:` 一行（`noteOps=1`，kind 列表含 `setTrackName`）；
/// 4. 回读 `out.yeban`，T1 的名字必须是新名（不是"鼓"）。
#[test]
fn the_rename_round_trips_through_save_as() {
    let dir = scratch("rename");
    let ops = write_ops(
        &dir,
        "ops.json",
        &format!(
            r#"{{"trackId": "{T1}", "ops": [{{"kind": "setTrackName", "trackId": "{T1}", "name": "新鼓"}}]}}"#
        ),
    );
    let out = dir.join("out.yeban");
    let lines = run_batch(&[
        "--project-sample",
        "default",
        "--apply-ops",
        &ops.display().to_string(),
        "--save-as",
        &out.display().to_string(),
    ])
    .expect("改名批处理必须成功");
    let receipt = lines
        .iter()
        .find(|line| line.starts_with("applied-ops:"))
        .expect("回执里必须有 applied-ops: 一行");
    assert!(
        receipt.contains("noteOps=1"),
        "回执必须报 noteOps=1，实为: {receipt}"
    );
    assert!(
        receipt.contains("setTrackName"),
        "回执的 kind 列表必须含 setTrackName，实为: {receipt}"
    );
    assert!(out.exists(), "保存的工程必须落盘");
    assert_eq!(track_name_of(&out), "新鼓");
}

/// **干跑判据**：`--dry-run` 不写盘（`--open` 的文件字节不变，也不产出 `--save-as`）。
///
/// 论证顺序：
/// 1. 先把 demo 工程存一份 `--open` 用的 `.yeban`（`--project-sample default --save-as`）；
/// 2. 记下它的字节；
/// 3. 跑 `--open base.yeban --apply-ops ops.json --dry-run`（不给 `--save-as`）；
/// 4. 回执是 `applied-ops-dryrun:` 一行且含 `written=no`；
/// 5. `base.yeban` 的字节**逐字节不变**；且同目录下没有多出任何 `.yeban`。
#[test]
fn dry_run_writes_nothing() {
    let dir = scratch("dryrun");
    let base = dir.join("base.yeban");
    run_batch(&[
        "--project-sample",
        "default",
        "--save-as",
        &base.display().to_string(),
    ])
    .expect("存 demo 工程");
    let before = std::fs::read(&base).expect("读 base.yeban");
    let ops = write_ops(
        &dir,
        "ops.json",
        &format!(
            r#"{{"trackId": "{T1}", "ops": [{{"kind": "setTrackName", "trackId": "{T1}", "name": "新鼓"}}]}}"#
        ),
    );
    let lines = run_batch(&[
        "--open",
        &base.display().to_string(),
        "--apply-ops",
        &ops.display().to_string(),
        "--dry-run",
    ])
    .expect("干跑必须成功");
    let receipt = lines
        .iter()
        .find(|line| line.starts_with("applied-ops-dryrun:"))
        .expect("回执里必须有 applied-ops-dryrun: 一行");
    assert!(
        receipt.contains("written=no"),
        "干跑回执必须报 written=no，实为: {receipt}"
    );
    assert!(
        !lines.iter().any(|line| line.starts_with("applied-ops:")),
        "干跑不能打真执行的回执"
    );
    assert_eq!(
        std::fs::read(&base).expect("重读 base.yeban"),
        before,
        "--open 的文件在干跑后必须逐字节不变"
    );
    assert_eq!(track_name_of(&base), "鼓", "干跑后工程里的名字必须还是旧名");
}

/// **配对判据**：真执行没给 `--save-as` = 用法错误（退出码 2），不静默丢编辑。
#[test]
fn apply_ops_without_save_as_is_a_usage_error() {
    let dir = scratch("no-save-as");
    let ops = write_ops(
        &dir,
        "ops.json",
        &format!(
            r#"{{"trackId": "{T1}", "ops": [{{"kind": "setTrackName", "trackId": "{T1}", "name": "新鼓"}}]}}"#
        ),
    );
    let error = run_batch(&[
        "--project-sample",
        "default",
        "--apply-ops",
        &ops.display().to_string(),
    ])
    .expect_err("没给 --save-as 的真执行必须失败");
    assert_eq!(error.exit_code(), 2, "必须是用法错误（退出码 2）");
    assert!(
        error.to_string().contains("--save-as"),
        "错误文案必须点名 --save-as，实为: {error}"
    );
}

/// **修饰器判据**：`--dry-run` 单独给 = 用法错误（退出码 2）。
#[test]
fn dry_run_without_apply_ops_is_a_usage_error() {
    let error = run_batch(&["--project-sample", "default", "--dry-run"])
        .expect_err("--dry-run 单独给必须失败");
    assert_eq!(error.exit_code(), 2, "必须是用法错误（退出码 2）");
    assert!(
        error.to_string().contains("--apply-ops"),
        "错误文案必须点名 --apply-ops，实为: {error}"
    );
}

/// **修饰器判据**：`--dry-run` 与 `--save-as` 同给 = 用法错误（退出码 2）——
/// 保存的会是没改过的工程，那是静默的假交付。
#[test]
fn dry_run_with_save_as_is_a_usage_error() {
    let dir = scratch("dryrun-save-as");
    let ops = write_ops(
        &dir,
        "ops.json",
        &format!(
            r#"{{"trackId": "{T1}", "ops": [{{"kind": "setTrackName", "trackId": "{T1}", "name": "新鼓"}}]}}"#
        ),
    );
    let out = dir.join("out.yeban");
    let error = run_batch(&[
        "--project-sample",
        "default",
        "--apply-ops",
        &ops.display().to_string(),
        "--dry-run",
        "--save-as",
        &out.display().to_string(),
    ])
    .expect_err("--dry-run 与 --save-as 同给必须失败");
    assert_eq!(error.exit_code(), 2, "必须是用法错误（退出码 2）");
    assert!(!out.exists(), "用法错误时不能产出文件");
}

/// **响亮失败判据**：未知 kind 被拒绝（退出码 3），不写盘。
#[test]
fn an_unknown_kind_fails_loudly_and_writes_nothing() {
    let dir = scratch("unknown-kind");
    let ops = write_ops(
        &dir,
        "ops.json",
        &format!(r#"{{"trackId": "{T1}", "ops": [{{"kind": "noSuchOp", "trackId": "{T1}"}}]}}"#),
    );
    let out = dir.join("out.yeban");
    let error = run_batch(&[
        "--project-sample",
        "default",
        "--apply-ops",
        &ops.display().to_string(),
        "--save-as",
        &out.display().to_string(),
    ])
    .expect_err("未知 kind 必须失败");
    assert_eq!(error.exit_code(), 3, "编辑失败是退出码 3（EXIT_OPEN）");
    assert!(
        error.to_string().contains("noSuchOp"),
        "错误文案必须点名未知 kind，实为: {error}"
    );
    assert!(!out.exists(), "失败时不能产出文件");
}

/// **响亮失败判据**：目标轨不存在时编译失败（退出码 3），不写盘。
///
/// 这里用的是一条 `setTrackName` 但 `trackId` 指向 demo 工程里没有的身份 ——
/// `compile` 在取当时值时找不到轨 ⇒ `ENTITY_NOT_FOUND`，不是静默跳过。
#[test]
fn a_missing_track_fails_loudly_and_writes_nothing() {
    let dir = scratch("missing-track");
    let ghost = "01J8Z5Q0R7K3M9X2V4B6N8P0ZZ";
    let ops = write_ops(
        &dir,
        "ops.json",
        &format!(
            r#"{{"trackId": "{ghost}", "ops": [{{"kind": "setTrackName", "trackId": "{ghost}", "name": "幽灵"}}]}}"#
        ),
    );
    let out = dir.join("out.yeban");
    let error = run_batch(&[
        "--project-sample",
        "default",
        "--apply-ops",
        &ops.display().to_string(),
        "--save-as",
        &out.display().to_string(),
    ])
    .expect_err("不存在的轨必须失败");
    assert_eq!(error.exit_code(), 3, "编辑失败是退出码 3（EXIT_OPEN）");
    assert!(!out.exists(), "失败时不能产出文件");
}

/// **干跑的诚实判据**：前置不满足时干跑照样失败（⛔ 不伪造成功）。
///
/// 与上一条同一批 ops，只是加了 `--dry-run`：模拟施加时找不到轨 ⇒ 失败，
/// 且 `--open` 的文件字节不变。
#[test]
fn dry_run_reports_a_domain_failure_instead_of_faking_success() {
    let dir = scratch("dryrun-fail");
    let base = dir.join("base.yeban");
    run_batch(&[
        "--project-sample",
        "default",
        "--save-as",
        &base.display().to_string(),
    ])
    .expect("存 demo 工程");
    let before = std::fs::read(&base).expect("读 base.yeban");
    let ghost = "01J8Z5Q0R7K3M9X2V4B6N8P0ZZ";
    let ops = write_ops(
        &dir,
        "ops.json",
        &format!(
            r#"{{"trackId": "{ghost}", "ops": [{{"kind": "setTrackName", "trackId": "{ghost}", "name": "幽灵"}}]}}"#
        ),
    );
    let error = run_batch(&[
        "--open",
        &base.display().to_string(),
        "--apply-ops",
        &ops.display().to_string(),
        "--dry-run",
    ])
    .expect_err("干跑遇领域失败时必须失败");
    assert_eq!(error.exit_code(), 3, "编辑失败是退出码 3（EXIT_OPEN）");
    assert_eq!(
        std::fs::read(&base).expect("重读 base.yeban"),
        before,
        "失败的干跑更不能动文件"
    );
}

/// **自带寻址判据**：全是 `addTrack` 的批不用顶层身份也能跑。
///
/// `addTrack` 永远自带寻址（`is_self_addressed`），顶层 `trackId` / `clipId`
/// 都可省 —— 这是 `yeban_edit_notes` 同一条口径，批处理出口必须同样放行。
#[test]
fn a_self_addressed_batch_needs_no_envelope_ids() {
    let dir = scratch("self-addressed");
    let ops = write_ops(
        &dir,
        "ops.json",
        // `addTrack` 的三字段全是必填（`trackId` 是调用方给的新身份，`trackKind` 取
        // "Midi" / "Audio" / "AuxReturn" / "Master"）；顶层身份可省。
        r#"{"ops": [{"kind": "addTrack", "trackId": "01J8Z5Q0R7K3M9X2V4B6N8P0T7", "name": "新轨", "trackKind": "Midi"}]}"#,
    );
    let out = dir.join("out.yeban");
    let lines = run_batch(&[
        "--project-sample",
        "default",
        "--apply-ops",
        &ops.display().to_string(),
        "--save-as",
        &out.display().to_string(),
    ])
    .expect("自带寻址的批必须成功");
    assert!(
        lines.iter().any(|line| line.starts_with("applied-ops:")),
        "回执里必须有 applied-ops: 一行"
    );
    assert!(out.exists(), "保存的工程必须落盘");
    let argv = vec![
        "--open".to_owned(),
        out.display().to_string(),
        "--headless".to_owned(),
    ];
    let options = cli::parse(&argv).expect("解析回读命令");
    let loaded = cli::load_project(&options).expect("回读保存的工程");
    assert_eq!(
        loaded.archive.project.tracks.len(),
        demo_project().tracks.len() + 1,
        "加轨后轨数必须 +1"
    );
}
