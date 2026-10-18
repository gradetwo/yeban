//! 判据（任务单 `#206`）：`#include` 在**装载入口**真的展开。
//!
//! 这一条测的是**用户路径**（`load_sfz_with_instrument` ⇒ `SfzInstrument::load_with_includes`），
//! 不是解析器内部的单元行为：
//!
//! | 判据 | 机械读数 |
//! | :--- | :--- |
//! | 三层递归 | `report.regions` = 被包含文件里的 region 数（≠ 0） |
//! | `default_path` 跨 include | `referenced_sample_paths()` 里出现前缀 ＋ 被包含文件的 `sample=` |
//! | 缺失 include | `report.include_issues.len() == 1`，其余 region 照常 |
//! | 成环 | 装载返回 `Err`，消息里是 `include cycle`（⛔ 不是无限递归/panic） |

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use yeban_app::sfz_load::load_sfz_with_instrument;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "yeban-sfz-include-{tag}-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

fn write(root: &Path, name: &str, contents: &str) {
    let path = root.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("建父目录");
    }
    std::fs::write(path, contents).expect("写夹具");
}

#[test]
fn recursive_includes_and_default_path_survive_the_loader() {
    let dir = temp_dir("recursive");
    write(&dir, "entries/c.sfz", "<region> sample=c.wav key=62\n");
    write(&dir, "entries/b.sfz", "#include \"entries/c.sfz\"\n");
    write(
        &dir,
        "main.sfz",
        "#include \"entries/b.sfz\"\n<region> sample=main.wav key=60\n",
    );

    let (report, instrument) = load_sfz_with_instrument(&dir.join("main.sfz"), None, 48_000.0)
        .expect("三层 include 必须装载成功");
    assert_eq!(
        report.regions, 2,
        "B→C 的 region 与 main 的 inline region 都要在"
    );
    assert!(report.include_issues.is_empty(), "没有缺失 ⇒ 零条 issue");
    assert_eq!(instrument.region_count(), 2);
    println!(
        "[#206 recursive] regions={} include_issues={} summary={}",
        report.regions,
        report.include_issues.len(),
        report.summary()
    );

    // `#include` 边界不重置 `<control> default_path`：被包含文件的 region 也吃前缀。
    let dir2 = temp_dir("default-path");
    write(
        &dir2,
        "parts/regions.sfz",
        "<region> sample=child.wav key=62\n",
    );
    write(
        &dir2,
        "main.sfz",
        "<control> default_path=Samples/\n#include \"parts/regions.sfz\"\n",
    );
    let (_report, instrument) =
        load_sfz_with_instrument(&dir2.join("main.sfz"), None, 48_000.0).expect("装载");
    let paths = instrument.referenced_sample_paths().expect("重解析");
    assert_eq!(paths, vec!["Samples/child.wav".to_string()]);
    println!("[#206 default_path] referenced={paths:?}");
}

#[test]
fn a_missing_include_is_one_recorded_issue_not_a_load_failure() {
    let dir = temp_dir("missing");
    write(&dir, "inc.sfz", "<region> sample=inc.wav key=62\n");
    write(
        &dir,
        "main.sfz",
        "<region> sample=main.wav key=60\n#include \"nope.sfz\"\n#include \"inc.sfz\"\n",
    );

    let (report, instrument) =
        load_sfz_with_instrument(&dir.join("main.sfz"), None, 48_000.0).expect("缺失不许整体失败");
    assert_eq!(report.regions, 2, "缺失项之外的 region 照常");
    assert_eq!(report.include_issues.len(), 1, "恰好记一条");
    assert_eq!(report.include_issues[0].line, 2);
    assert_eq!(report.include_issues[0].path, "nope.sfz");
    assert!(
        report.summary().contains("include_issues=1"),
        "汇总行必须带条数: {}",
        report.summary()
    );
    assert!(instrument.region_count() == 2);
    println!(
        "[#206 missing] regions={} issue=`{}`",
        report.regions, report.include_issues[0]
    );
}

#[test]
fn a_cycle_is_a_clear_error_not_a_hang() {
    let dir = temp_dir("cycle");
    write(&dir, "a.sfz", "#include \"b.sfz\"\n");
    write(&dir, "b.sfz", "#include \"a.sfz\"\n");
    write(&dir, "main.sfz", "#include \"a.sfz\"\n");

    let error = load_sfz_with_instrument(&dir.join("main.sfz"), None, 48_000.0)
        .expect_err("成环必须响亮失败");
    let shown = error.to_string();
    assert!(shown.contains("include cycle"), "错误原文: {shown}");
    println!("[#206 cycle] error={shown}");
}
