//! **T-FIX-754：预备拍档位 0 → 1 → 2 → 0** 的纯函数判据。
//!
//! 本文件只钉产品语义与 2 小节工程的读数；⛔ 不建 UI / 不碰声卡：
//! 1. `next_count_in_bars` 必须是三档循环，⛔ 不是 0⇄1 的开关；
//! 2. `filled_project` 的 2 小节必须真的能算成 180000 帧（4/4、128 BPM、48 kHz）；
//! 3. 从 2 小节开始按三下回到 2 小节，且 2 小节工程的帧数始终是 180000 ——
//!    证明“2 小节工程恢复”不是只把菜单文案写对。
//!
//! 文档提交 / 撤销 / 持久化由 `d_ui_23_count_in_writes_the_document` 与
//! `transport_*` 套件覆盖；本文件**不**代替它们，只把本单新增的“下一个值”算法
//! 单独钉死，避免 UI 文案与产品语义分叉。

use yeban_app::host::next_count_in_bars;
use yeban_app::record_wav::count_in_frames;
use yeban_model::samples::filled_project;

const RATE: u32 = 48_000;

#[test]
fn count_in_toggle_cycles_zero_one_two_zero() {
    assert_eq!(next_count_in_bars(0), 1, "0 的第一下必须是 1 小节");
    assert_eq!(next_count_in_bars(1), 2, "1 的第二下必须恢复 2 小节");
    assert_eq!(next_count_in_bars(2), 0, "2 的第三下必须关掉（⛔ 回到 1）");
    // 未知档位按 0 处理：⛔ 把脏值留在文档里，也⛔ 让下一跳继续漂。
    assert_eq!(next_count_in_bars(3), 0);
    assert_eq!(next_count_in_bars(255), 0);
}

#[test]
fn filled_project_keeps_two_bars_and_restores_them_after_a_full_cycle() {
    let mut project = filled_project();
    assert_eq!(
        project.transport.count_in_bars, 2,
        "夹具必须是 2 小节工程（这是本单要恢复的那一档）"
    );
    assert_eq!(
        count_in_frames(&project, RATE),
        180_000,
        "2 小节 × 4 拍 × 60/128 × 48 kHz = 180000 帧"
    );
    assert_eq!(
        count_in_frames(&project, RATE),
        180_000,
        "录音腿读到的 2 小节长度必须是 180000 帧"
    );

    // 从 2 小节开始：2 → 0 → 1 → 2。第三下之后必须回到 2，而不是停在 1。
    project.transport.count_in_bars = next_count_in_bars(project.transport.count_in_bars);
    assert_eq!(project.transport.count_in_bars, 0);
    project.transport.count_in_bars = next_count_in_bars(project.transport.count_in_bars);
    assert_eq!(project.transport.count_in_bars, 1);
    project.transport.count_in_bars = next_count_in_bars(project.transport.count_in_bars);
    assert_eq!(project.transport.count_in_bars, 2, "第三下必须恢复 2 小节");
    assert_eq!(
        count_in_frames(&project, RATE),
        180_000,
        "恢复 2 小节后，录音腿必须重新读到 180000 帧"
    );
}

#[test]
fn zero_bars_still_reads_zero_frames() {
    let mut project = filled_project();
    project.transport.count_in_bars = 0;
    assert_eq!(
        count_in_frames(&project, RATE),
        0,
        "0 小节必须仍然不产生预备拍"
    );
}

#[test]
fn menu_source_declares_all_three_labels_and_the_property_driver() {
    let app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    for label in [
        "预备拍开关：关 (Ctrl+Alt+D)",
        "预备拍开关：1 小节 (Ctrl+Alt+D)",
        "预备拍开关：2 小节 (Ctrl+Alt+D)",
    ] {
        assert!(app.contains(label), "菜单缺档位文案：{label}");
    }
    assert!(
        app.contains("root.count-in-bars <= 0") && app.contains("root.count-in-bars == 1"),
        "菜单标题必须由 count-in-bars 驱动 0/1/2 三档"
    );
}
