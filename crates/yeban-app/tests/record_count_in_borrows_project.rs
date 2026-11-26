//! ⭐⭐ **T-FIX-464 的判据：`record_count_in` / `record_count_in_ticks` 只借工程、
//! 不整工程克隆。**
//!
//! ## 它补的是哪一格
//!
//! 修前：两函数在端口在场时都走 `port.try_project()` —— `Local` 后端把整份
//! `YebanProjectV1` 深拷贝一次，只为读一个标量（预备拍长度）。录音键那条腿
//! （`host.rs` 的起录分支）每按一次录音键就付两次这样的克隆。本单之后：
//! 两函数都走 `port.with_project` 借用读（`Local` 后端零克隆）；`Authority`
//! 后端仍经控制面快照一次 —— 那是控制面路径的既有文档口径，本单接受，
//! 收据里如实写明、⛔ 不宣称 Authority 腿零克隆。
//!
//! ## 为什么"零克隆"只能用源码级证据
//!
//! 对纯读而言，克隆与借用在**行为上完全相同**（读到的工程内容一位不差，
//! 算出的帧数／tick 数也一位不差）—— 没有任何行为判据能区分"借着读"还是
//! "克隆了再读"；性能／分配在判据里也测不稳。所以本单的"零克隆"钉在源码上：
//! 把两函数的函数体抽出来，断言它们经 `with_project` 取工程、且体内没有任何
//! `try_project()`／`.clone()`。红对照：把旧的两行快照写回去 ⇒ 本判据立刻红
//! （见收据）。
//!
//! ## 行为部分
//!
//! 源码钉不住行为，所以另有三条行为判据（⛔ 不开窗口，纯函数）：端口在场时
//! 活文档赢过冻结快照（优先级逐字保留：端口 > 冻结 > 不知道 ⇒ 0）；端口
//! 在场但工程是 0 小节 ⇒ 0（冻结那份 ⛔ 不能漏进来）；没有端口时退回冻结
//! 快照；两份都没有 ⇒ 0。
//!
//! 端口在场但读不出工程那一档（控制面关掉会话 ⇒ 如实 0、⛔ 不拿冻结冒充）
//! 判据⛔ 造不出来：`try_project`／`with_project` 的 `None` 只存在于
//! `UndoBackend::Authority` 那一支（特性 `in-process-mcp`，控制面可关工程），
//! 本文件的夹具是 `UndoBackend::Local`（打开后没有关掉工程的公开入口 ⇒
//! 恒 `Some`）—— 与 `d_ui_23_count_in_writes_the_document.rs` 的登记相同，
//! 只登记不验；读不出 ⇒ 0 的形状由 `unwrap_or(0)` 与 `try_project` 的
//! `map_or(0, …)` 逐分支等价，见收据。

use std::rc::Rc;

use yeban_app::host::{record_count_in, record_count_in_ticks};
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::YebanProjectV1;

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;

/// 取数用的采样率（单位：**赫兹**）。
const RATE: u32 = 48_000;

/// 夹具的预备拍：`bars = 2` 小节、`bpm = 128.0`、`numerator = 4`
/// ⇒ 帧数 `2 × 4 × 60 ÷ 128 × 48_000 = 180_000`（二进制精确，⛔ 不靠四舍五入），
/// tick 数 `2 × 4 × 960 = 7_680`。
const BARS: u8 = 2;
const FRAMES: u64 = 180_000;
const TICKS: u64 = 7_680;

/// 冻结快照那一档的预备拍（`bars = 1` ⇒ 90_000 帧／3_840 tick），
/// 与活文档的 2 小节**可区分** —— 优先级判据靠它分得清读的是哪一份。
const FROZEN_BARS: u8 = 1;
const FROZEN_FRAMES: u64 = 90_000;
const FROZEN_TICKS: u64 = 3_840;

/// 夹具工程：`filled_project()`＋ 显式定死预备拍三要素（⛔ 不靠夹具默认值）。
fn fixture_project(bars: u8) -> YebanProjectV1 {
    let mut project = yeban_model::samples::filled_project();
    project.transport.count_in_bars = bars;
    project.bpm = 128.0;
    project.time_signature.numerator = 4;
    project
}

/// 带权威会话的端口（`UndoBackend::Local`）。
fn port_for(project: YebanProjectV1) -> Rc<UndoPort> {
    Rc::new(UndoPort::new(
        UndoSession::open("<t-fix-464-count-in>", "yeban-app", project, NOW_MS).expect("打开会话"),
    ))
}

/// 从 `src/host.rs` 里抽出指定函数的函数体（花括号配平）。
///
/// 体内没有含花括号的字面量／注释（以 `marker` 在场为准做窗口自检：
/// 窗口切错时自检先红，⛔ 不静默放过）。
fn function_body(name: &str, marker: &str) -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/host.rs");
    let text = std::fs::read_to_string(path).expect("读 host.rs");
    let lines: Vec<&str> = text.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.contains(&format!("fn {name}(")))
        .unwrap_or_else(|| panic!("host.rs 必须有 fn {name}"));
    let mut depth = 0i32;
    let mut seen_open = false;
    let mut end = start;
    for (index, line) in lines.iter().enumerate().skip(start) {
        for c in line.chars() {
            if c == '{' {
                depth += 1;
                seen_open = true;
            } else if c == '}' {
                depth -= 1;
            }
        }
        if seen_open && depth == 0 {
            end = index;
            break;
        }
    }
    assert!(
        end > start,
        "⭐ {name} 的函数体必须能配平抽出（起 {start} 行）"
    );
    let body = lines[start..=end].join("\n");
    assert!(
        body.contains(marker),
        "⭐ 抽出的 {name} 窗口必须含 {marker}，否则窗口切错了（起 {start} 行）"
    );
    body
}

/// ⭐ **验收 ②**：两函数体内零整工程克隆 —— 经 `with_project` 借用，
/// ⛔ 不许 `try_project()`、⛔ 不许 `.clone()`。
#[test]
fn record_count_in_functions_read_the_project_by_borrow_not_by_clone() {
    for (name, marker) in [
        ("record_count_in", "count_in_frames"),
        ("record_count_in_ticks", "count_in_ticks"),
    ] {
        let body = function_body(name, marker);
        assert!(
            body.contains("with_project"),
            "⭐ {name} 必须走 `with_project` 借用（体实测无该调用）：\n{body}"
        );
        assert!(
            !body.contains("try_project"),
            "⭐ {name} 体内 ⛔ 不许再走 `try_project()` 快照（实测含该调用）：\n{body}"
        );
        assert!(
            !body.contains(".clone()"),
            "⭐ {name} 体内 ⛔ 不许出现整工程克隆（实测含 .clone()）：\n{body}"
        );
    }
}

/// ⭐ **验收 ①a**：端口在场 ⇒ 读活文档（即便冻结快照说是另一档）。
#[test]
fn port_present_prefers_the_live_document_over_the_frozen_snapshot() {
    let port = port_for(fixture_project(BARS));
    let frozen = fixture_project(FROZEN_BARS);
    // 夹具自检：两档必须可区分，否则这条优先级判据会空心通过。
    assert_ne!(
        FRAMES, FROZEN_FRAMES,
        "⭐ 夹具的活文档与冻结快照必须算出不同的帧数，否则优先级判据没有判别力"
    );
    assert_ne!(
        TICKS, FROZEN_TICKS,
        "⭐ 夹具的活文档与冻结快照必须算出不同的 tick 数，否则优先级判据没有判别力"
    );
    assert_eq!(
        record_count_in(Some(&port), Some(&frozen), RATE),
        FRAMES,
        "⭐ 端口在场时帧数必须取活文档的 2 小节（实测不是 {FRAMES} ⇒ 读了冻结那份）"
    );
    assert_eq!(
        record_count_in_ticks(Some(&port), Some(&frozen)),
        TICKS,
        "⭐ 端口在场时 tick 数必须取活文档的 2 小节（实测不是 {TICKS} ⇒ 读了冻结那份）"
    );
}

/// ⭐ **验收 ①b**：端口在场但活文档是 0 小节 ⇒ 0（冻结那份 ⛔ 不能漏进来）。
#[test]
fn port_present_with_zero_bars_means_zero_not_the_frozen_value() {
    let port = port_for(fixture_project(0));
    let frozen = fixture_project(FROZEN_BARS);
    assert_eq!(
        record_count_in(Some(&port), Some(&frozen), RATE),
        0,
        "⭐ 活文档 0 小节时帧数必须是 0（实测非 0 ⇒ 冻结那份漏进来了）"
    );
    assert_eq!(
        record_count_in_ticks(Some(&port), Some(&frozen)),
        0,
        "⭐ 活文档 0 小节时 tick 数必须是 0（实测非 0 ⇒ 冻结那份漏进来了）"
    );
}

/// ⭐ **验收 ①c**：没有端口 ⇒ 退回冻结快照；两份都没有 ⇒ 0。
#[test]
fn no_port_falls_back_to_the_frozen_snapshot_then_to_zero() {
    let frozen = fixture_project(BARS);
    assert_eq!(
        record_count_in(None, Some(&frozen), RATE),
        FRAMES,
        "⭐ 没有端口时帧数必须退回冻结快照的 2 小节"
    );
    assert_eq!(
        record_count_in_ticks(None, Some(&frozen)),
        TICKS,
        "⭐ 没有端口时 tick 数必须退回冻结快照的 2 小节"
    );
    assert_eq!(
        record_count_in(None, None, RATE),
        0,
        "⭐ 两份都没有时帧数必须是 0"
    );
    assert_eq!(
        record_count_in_ticks(None, None),
        0,
        "⭐ 两份都没有时 tick 数必须是 0"
    );
}
