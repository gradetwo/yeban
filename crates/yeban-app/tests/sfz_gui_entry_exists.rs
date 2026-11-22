//! 「SFZ 乐器装载」的 GUI 入口**已经在**（审计 `T-FIX-88` 项 4 的先验）。
//!
//! 台账 `docs/ledger/feature-alignment.md` 第 153 行原记 GUI 列「无｜无乐器加载控件
//! （`ui/console/device_rack.slint` 仍是演示常量）」—— 实测**已过期**：活界面里的
//! 入口有三处，且都落到同一条宿主实现。本判据钉四件事：
//!
//! 1. **入口存在**：下舱空设备链的「+ 加载 SFZ」按钮（`execute-action("load-sfz")`）、
//!    音色菜单的「采样库 • 加载 SFZ 音色...」项、通道条的采样器绑定/解绑手势；
//! 2. **接线到位**：`host::wire_module_actions` 的 `on_load_sfz` 真的读盘装载
//!    （`sfz_load::load_sfz_with_instrument`），并在路径缺失时走 `T-FIX-21` 的
//!    音源库打开序列；
//! 3. **差在哪**：GUI **没有文件选择器** —— 来源是宿主预配置的 `ModulePaths.load_sfz`
//!    单一路径（`PathBuf`），不是用户在界面上挑的文件。这条正是"已有入口、但不是
//!    任意文件装载"的边界，⛔ 不该再被写成「无入口」；
//! 4. **台账不回退**：第 153 行的 GUI 列必须以「部分｜」开头并点名这三个落点。

use std::path::PathBuf;

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    let path = crate_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("读不到 {relative}: {error}"))
}

#[test]
fn the_dock_offers_an_sfz_load_entry() {
    let dock = read("ui/lower_dock.slint");
    assert!(
        dock.contains("@tr(\"+ 加载 SFZ\")"),
        "下舱空设备链必须有「+ 加载 SFZ」按钮（GUI 入口之一）"
    );
    assert!(
        dock.contains("root.execute-action(\"load-sfz\")"),
        "按钮必须落到 `load-sfz` 这条模块动作（⛔ 不是只画一个按钮）"
    );
    assert!(
        dock.contains("{ id: \"sfz:default\", label: @tr(\"采样库 • 加载 SFZ 音色...\") }"),
        "音色菜单必须有 SFZ 音色项（入口之二：从采样库挑）"
    );
    // ⚠ 第三条候选（通道条 `mixer-sampler-set`）住在**未编译**的
    // `ui/console/console_tabs.slint` 里，活界面里没有 ⇒ 不当作入口，
    // 它是"部分"的边界之一（见台账那一行）。
}

#[test]
fn the_host_wires_the_entry_to_the_real_loader() {
    let host = read("src/host.rs");
    assert!(
        host.contains("ui.on_load_sfz(move || {"),
        "宿主必须接 `load-sfz` 回调"
    );
    assert!(
        host.contains("crate::sfz_load::load_sfz_with_instrument("),
        "装载必须走唯一实现 `sfz_load::load_sfz_with_instrument`"
    );
    assert!(
        host.contains("open_sound_library_for_missing_sfz(&ui);"),
        "路径缺失时必须走 T-FIX-21 的音源库打开序列（⛔ 不只是 set_sound_library_open）"
    );
    assert!(
        host.contains("\"sfz:default\" => {"),
        "音色菜单项 `sfz:default` 必须有宿主分支（与下舱按钮同一条 `invoke_load_sfz`）"
    );
}

#[test]
fn the_only_gap_is_that_the_source_path_is_preconfigured() {
    let host = read("src/host.rs");
    // 来源是**预配置的单一路径**：这就是"入口已有、但没有文件选择器"的机械证据。
    assert!(
        host.contains("/// `装载 SFZ` 的来源文件。"),
        "`ModulePaths.load_sfz` 的文档必须仍如实写着「来源文件」"
    );
    assert!(
        host.contains("pub load_sfz: std::path::PathBuf,"),
        "来源是 `PathBuf` 字段（宿主配置），⛔ 不是界面上的文件对话框"
    );
}

#[test]
fn the_ledger_row_records_the_measured_reality() {
    let ledger = read("../../docs/ledger/feature-alignment.md");
    let row = ledger
        .lines()
        .find(|line| {
            line.starts_with("| SFZ 乐器装载（`yeban-sfz` 的 `parse_text` + 会话内 registry）")
        })
        .expect("台账必须有这一行");
    assert!(
        row.contains("| 部分｜"),
        "GUI 列必须改成「部分」（入口已有，差在无文件选择器）：{row}"
    );
    assert!(
        row.contains("+ 加载 SFZ"),
        "GUI 列必须点名实测落点（下舱按钮）：{row}"
    );
    assert!(
        !row.contains("| 无｜无乐器加载控件"),
        "旧读数「无｜无乐器加载控件」已过期，不得留在台账里：{row}"
    );
}
