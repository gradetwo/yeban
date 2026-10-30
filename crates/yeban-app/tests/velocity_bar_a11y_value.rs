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
    use slint::Model as _;

    let project = contrived_project();
    let ui = build_live_ui(&project, Permission::Interactive).expect("真实界面 + Tier-1 执行面");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let tree = tree_from_element_root(&window, None).expect("运行时控件树");

    // 索引集：柱 i ↔ `note-ulids[i]`（宿主用同一份 `visible` 注入）。
    let ulids: Vec<String> = window
        .get_note_ulids()
        .iter()
        .map(|ulid| ulid.to_string())
        .collect();
    assert!(
        ulids.len() >= 2,
        "[UI-A11Y-004] 夹具必须投影出至少 2 个音符（实际 {}）",
        ulids.len()
    );

    let mut reported = Vec::with_capacity(ulids.len());
    for (index, ulid) in ulids.iter().enumerate() {
        let id = format!("velocity-{index}-bar");
        let node = tree
            .find_by_id(&id)
            .unwrap_or_else(|| panic!("运行时树里必须有 `{id}`"));
        let value = node
            .value
            .clone()
            .unwrap_or_else(|| panic!("`{id}` 必须声明 `accessible-value`"));
        let expected = model_velocity(&project, ulid).to_string();
        assert_eq!(
            value, expected,
            "[UI-A11Y-004] `{id}`（音符 {ulid}）的屏读值必须等于工程的力度 0–127"
        );
        report_line(&format!(
            "[UI-A11Y-004] {id} ui/property value={value:?} 权威力度={expected:?}"
        ));
        reported.push(value);
    }

    // 防"假绿"守卫：如果所有柱恰好同值（夹具退化），逐值比对就检验不了"每根柱自己的值"。
    let mut distinct = reported.clone();
    distinct.sort();
    distinct.dedup();
    assert!(
        distinct.len() >= 2,
        "[UI-A11Y-004] 夹具退化了：全部力度柱同值 {distinct:?}，无法区分'逐柱读数'与'常量'"
    );
    // 被删掉的那个常量本身也不得再次成为每一根柱的读数。
    assert!(
        distinct != ["100".to_owned()],
        "[UI-A11Y-004] 全部力度柱都自称 100 —— 这是被删除的写死字面量"
    );
}
