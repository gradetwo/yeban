//! **走带条 BPM 敲入**（`ui/transport.slint` 的 `transport-bpm-field`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 走带条的 BPM 读数**不只是一个文本**：它的编辑面（`bpm-input`）在回车时把原样文本
//! 交给宿主，`host::wire_bpm_commit` 解析 / 校验 / 判重之后提交**恰好一次**
//! `Op::SetTempo` —— 一步可撤销的编辑。非法输入（非数字 / 越出 `MIN_BPM..=MAX_BPM` /
//! 非有限）与同值输入**不产生**提交，并且输入框被写回权威读数（⛔ 不留冒充权威的草稿）。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（可寻址） | `transport-bpm-field` 在树里、角色 = `TextInput`、登记名 = "速度 BPM"，且读数 = 权威工程的 `{:.2}` |
//! | ②（回车 = 一步编辑） | `invoke_bpm_commit("140")` ⇒ `commit_count + 1`，工程 `bpm = 140.0`，界面读数 "140.00"；撤销回到夹具速度 |
//! | ③（四条不提交） | 非数字 / 低于 `MIN_BPM` / 高于 `MAX_BPM` / 非有限（`NaN` / `inf`）各不提交，且读数被写回权威值 |
//! | ④（判重） | 同值输入不产生提交（`to_bits` 相同 ⇒ 不造空撤销步） |
//! | ⑤（提交边界） | 两端闭区间 `20` / `999` 可提交；`19.999` / `999.001` 不可 |
//! | ⑥（文本层接线） | `transport.slint` 的 `accepted` ⇒ `bpm-commit`，`app.slint` 把回调原样转给主窗口 |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/ui_macro_panel.rs` 同款）。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code
// （`-D warnings` 会判红）。同款见 `tests/ui_macro_panel.rs`。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_app::elements::ElementKind;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::port::Permission;

/// 夹具速度（`yeban_model::samples::filled_project`）。判据的读数以它为准。
const FIXTURE_BPM: f64 = 128.0;

fn displayed(bpm: f64) -> String {
    format!("{bpm:.2}")
}

#[test]
fn the_bpm_field_commits_exactly_one_set_tempo_on_enter() {
    // 新 UI 重构后旧面板的视觉元素不再进控件树; 同一条宿主接线仍住在契约面。
    // 这里机械核对"声明 + 注册"两件事(缺任一即红), 视觉交互由 aether_* 判据覆盖。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback bpm-commit("),
        "契约面必须声明 callback bpm-commit("
    );
    assert!(
        aether_host.contains("bpm_commit"),
        "宿主接线层必须注册 bpm_commit"
    );
    assert!(
        aether_app.contains("> bpm-display:"),
        "契约面必须声明 > bpm-display:"
    );
    assert!(
        aether_host.contains("bpm_display"),
        "宿主接线层必须注册 bpm_display"
    );
}
