//! T-FIX-70 判据：调试输出开关的**输出面**钉。
//!
//! - 默认（不开 `--debug`）：批处理报告里**一条** `[debug]` 行都不许有
//!   —— 关闭时与旧版逐字一致不是口号，是这条判据。
//! - 开 `--debug`：设备枚举行与装载逐项明细行必须出现。无声卡环境
//!   （CI/盒子）枚举为空集 ⇒ 枚举行报"0 台"，行本身必须在。

use yeban_app::cli::{Options, parse, run_batch};

fn argv(raw: &[&str]) -> Vec<String> {
    raw.iter().map(|item| (*item).to_owned()).collect()
}

fn headless_options(extra: &[&str]) -> Options {
    let mut raw = vec!["--headless"];
    raw.extend_from_slice(extra);
    parse(&argv(&raw)).expect("判据参数必须解析成功")
}

#[test]
fn debug_off_adds_no_debug_lines() {
    let lines = run_batch(&headless_options(&[])).expect("默认批处理必须成功");
    assert!(!lines.is_empty(), "默认批处理报告必须非空");
    assert!(
        lines.iter().all(|line| !line.contains("[debug]")),
        "调试关闭时报告里不许出现 [debug] 行: {lines:?}"
    );
}

#[test]
fn debug_on_adds_enumeration_and_load_detail_lines() {
    let lines = run_batch(&headless_options(&["--debug"])).expect("--debug 批处理必须成功");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("[debug]") && line.contains("输出设备枚举")),
        "--debug 时必须出现输出设备枚举行: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("[debug]") && line.contains("输入设备枚举")),
        "--debug 时必须出现输入设备枚举行: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("[debug] 轨[")),
        "--debug 时必须出现逐轨装载明细行: {lines:?}"
    );
}

#[test]
fn log_file_receives_the_same_debug_lines() {
    // `--log-file` 的文件副本必须含报告里的调试段（枚举 + 逐轨明细），
    // 否则用户回传的诊断文件缺的正是最有用的部分。
    // 注：`init` 的开关位故意给 `false` —— 汇点是进程级单份，本判据
    // 只钉「文件副本」这一面；报告侧的调试段由 `--log-file` 选项本身
    // 触发（`run_batch` 的门），`append_to_log` 只看文件句柄不看开关，
    // 这样不会把并行跑的「关闭」判据污染成开启态。
    let dir = std::env::temp_dir().join(format!("yeban-fix70-log-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("临时目录必须建得出来");
    let log_path = dir.join("diag.log");
    yeban_app::debug_out::init(false, Some(&log_path)).expect("日志文件必须打开");
    let lines = run_batch(&headless_options(&[
        "--log-file",
        log_path.to_str().expect("临时路径是 UTF-8"),
    ]))
    .expect("--log-file 批处理必须成功");
    let written = std::fs::read_to_string(&log_path).expect("日志文件必须可读");
    assert!(
        written.contains("输出设备枚举"),
        "日志文件里必须有输出设备枚举行: {written}"
    );
    assert!(
        written.contains("[debug] 轨["),
        "日志文件里必须有逐轨装载明细行: {written}"
    );
    assert!(
        lines.iter().any(|line| line.contains("输出设备枚举")),
        "控制台报告里同样必须有枚举行: {lines:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn log_file_switch_also_turns_debug_lines_on() {
    // `--log-file` 给出即视为开启（否则文件恒空）：批处理报告同样出调试行。
    let lines = run_batch(&headless_options(&["--log-file", "diag-probe.log"]))
        .expect("--log-file 批处理必须成功");
    assert!(
        lines.iter().any(|line| line.contains("[debug]")),
        "--log-file 时报告里必须有 [debug] 行: {lines:?}"
    );
}
