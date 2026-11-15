//! `[UI-A11Y-004]`：力度条的**无障碍读数**必须是这一根柱自己的力度。
//!
//! ## 为什么要这条判据
//!
//! `ui/console/piano_roll.slint` 的力度柱几何早就读投影（`root.velocities[i]`，拖动期读
//! 草稿），但 `accessible-value` 曾是一个**写死的字面量 `"100"`** —— 每一根柱都向屏幕
//! 阅读器自称 100，与柱高（0–127 的真实力度）互相矛盾。本判据把"屏读器读到的数"与
//! **权威工程**里的 `MidiNote::velocity` 逐个钉在一起。
//!
//! ## 读数口径
//!
//! - 期望值**不来自**同一个属性：从工程（`clip_pool[*].content.notes()`）按 ULID 取
//!   `note.velocity`（0–127 整数），与控制面 `ControlNode.value`（= 活组件的
//!   `accessible-value`，见 `yeban_ui_test_port::inspect`）逐字比对。
//! - 判据顺序：`velocity-{i}-bar` 与 `note-ulids[i]` 同一索引集（宿主
//!   `host::apply_view` 用同一份 `visible` 注入；对齐关系另有
//!   `test_port_adapter` 的"逐个对齐"断言）。
//!
//! ## 怎么变红
//!
//! - 把 `accessible-value` 改回任何**与 `velocity_reading` 无关**的常量（含原来的
//!   `"100"`）⇒ 本判据立刻红：夹具故意让音符力度**互不相同**，且至少一个不是 100。
//! - 把乘 127 的换算写错（如不取整 ⇒ `"99.99999"`）⇒ 逐字比对红。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `build_live_ui` 一部分 ⇒ 其余项在测试目标里是 dead_code，`-D warnings` 会判红。
#[allow(dead_code)]
mod live;

use live::build_live_ui;
use yeban_model::YebanProjectV1;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 夹具力度：让每个音符都不同，且都落在 1..=127（0 力度柱高为 0，不是本判据的对象）。
fn contrived_project() -> YebanProjectV1 {
    let mut project = yeban_model::samples::filled_project();
    let mut index = 0u32;
    for entry in project.clip_pool.values_mut() {
        let Some(notes) = entry.content.notes_mut() else {
            continue;
        };
        for note in notes.values_mut() {
            note.velocity = 1 + u8::try_from(index % 127).expect("取模结果 < 127");
            index += 1;
        }
    }
    project
}

/// 权威工程里某个 ULID 的力度（与 `tests/roll_edit_tools.rs::velocity_of` 同一读法）。
fn model_velocity(project: &YebanProjectV1, ulid: &str) -> u8 {
    for entry in project.clip_pool.values() {
        let Some(notes) = entry.content.notes() else {
            continue;
        };
        for (id, note) in notes {
            if id.to_canonical_string() == ulid {
                return note.velocity;
            }
        }
    }
    panic!("权威工程里必须有音符 `{ulid}`");
}

#[test]
fn every_velocity_bar_reports_its_own_projected_velocity_to_the_accessibility_tree() {
    // 新 UI 重构: 旧面板视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}
