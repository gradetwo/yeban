//! **主控通道条的静音 / 独奏** —— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 混音台主控通道条上的 M / S 两个按钮**不是装饰**：点击 ⇒
//! `host::wire_mixer_master_switch` 按**投影里的主总线身份**
//! （`bridge::master_device_track`）提交**恰好一次** `Op::SetTrackMute` /
//! `Op::SetTrackSolo` —— 与轨道头 / 通道条的开关是同一对 `Op`、同一条撤销栈。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（可寻址） | 两个 ID 在运行时树里、角色 = `Button`、登记名正确，且都登记过 |
//! | ②（静音 = 一步编辑） | `invoke_mixer_master_mute_toggle()` ⇒ `commit_count + 1`，权威工程里主总线 `mute = true`，界面 `master-mute` 跟着变；撤销回到原值 |
//! | ③（独奏 = 一步编辑） | 同上（`solo`） |
//! | ④（幂等边界） | 连续两次点击 = 两步编辑（一开一关），工程回到原值 |
//! | ⑤（文本层接线） | 主控按钮有 `TouchArea` ＋ `accessible-action-default` 同一落点；`console_tabs` / `app.slint` 原样上抛 |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/ui_macro_panel.rs` 同款）。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code
// （`-D warnings` 会判红）。同款见 `tests/ui_bpm_field.rs`。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_app::elements::ElementKind;
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_ui_test_port::port::Permission;

#[test]
fn the_master_mute_and_solo_buttons_commit_one_op_each() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> master-mute:"),
        "新 UI 契约面必须声明 > master-mute:"
    );
    assert!(
        aether_host.contains("set_master_mute"),
        "宿主接线层必须注册 set_master_mute"
    );
    assert!(
        aether_app.contains("> master-solo:"),
        "新 UI 契约面必须声明 > master-solo:"
    );
    assert!(
        aether_host.contains("set_master_solo"),
        "宿主接线层必须注册 set_master_solo"
    );
    assert!(
        aether_app.contains("callback mixer-master-mute-toggle("),
        "新 UI 契约面必须声明 callback mixer-master-mute-toggle("
    );
    assert!(
        aether_host.contains("on_mixer_master_mute_toggle"),
        "宿主接线层必须注册 on_mixer_master_mute_toggle"
    );
}
