//! [G15] `sections.timeline` 派生视图的判据（`yeban_query_project` 的段落时间轴读数）。
//!
//! | # | 判据 | 用例 |
//! | :--- | :--- | :--- |
//! | a | 数组逐条等于**独立重算**的 `(start_tick, name)` 升序表；且身份序 ≠ 时间序（排序**有 observable 差**，删掉 sort 必红） | `sections_timeline_is_sorted_by_start_tick_not_by_identity` |
//! | b | `sections` 原始形状**不变**（派生视图走独立键 `sectionsTimeline`） | 同上（`assert_ne` 那条 + 形状断言） |
//! | c | 同一工程两次调用逐字节相同（确定性） | `the_timeline_view_is_deterministic` |

use serde_json::Value;
use yeban_mcp::domain::view::project_view;
use yeban_model::samples::filled_project;
use yeban_model::{EntityId, SectionV3, YebanProjectV1};

/// 造一个带三段"身份序与时间序**必然相反**"的夹具工程：
/// `EntityId::new()` 是单调 ULID ⇒ 键序 = 创建序 = (30k, 20k, 10k) 降序。
fn fixture_project() -> YebanProjectV1 {
    let mut project = filled_project();
    for (name, start) in [("Outro", 30_000u64), ("Bridge", 20_000), ("Intro", 10_000)] {
        let entry = SectionV3 {
            id: EntityId::new(),
            name: name.to_owned(),
            start_tick: start,
            end_tick: start + 5_000,
            color: None,
        };
        project.sections.insert(entry.id, entry);
    }
    project.validate().expect("夹具工程必须合法");
    project
}

fn timeline_of(view: &Value) -> Vec<(u64, String)> {
    view["sectionsTimeline"]
        .as_array()
        .expect("`sectionsTimeline` 必须是数组")
        .iter()
        .map(|entry| {
            (
                entry["start_tick"].as_u64().expect("start_tick"),
                entry["name"].as_str().expect("name").to_owned(),
            )
        })
        .collect()
}

#[test]
fn sections_timeline_is_sorted_by_start_tick_not_by_identity() {
    let project = fixture_project();
    let fields = vec!["sections.timeline".to_owned(), "sections".to_owned()];
    let view = project_view(&project, &fields).expect("视图必须成功");

    // ⭐ 独立重算的期望序（⛔ 不手抄段数或名字表）。
    let mut expected: Vec<(u64, String)> = project
        .sections
        .values()
        .map(|section| (section.start_tick, section.name.clone()))
        .collect();
    expected.sort();
    assert_eq!(
        timeline_of(&view),
        expected,
        "时间轴视图必须逐条等于按 tick 升序"
    );

    // 排序必须**observable**：原始 `sections` 的键序（身份序）与时间序不同。
    let identity_first: &SectionV3 = project.sections.values().next().expect("夹具至少有 3 段");
    let timeline_first = &view["sectionsTimeline"][0];
    assert_ne!(
        identity_first.name,
        timeline_first["name"].as_str().expect("name"),
        "夹具必须保证身份序 ≠ 时间序（否则删掉 sort 也测不出来）"
    );

    // `sections` 自己的形状**不变**：仍是身份键对象的稀疏展开（`{id: {name}}` 只带被
    // 选中的下钻叶子时是 `{id: {name}}`；这里选了整份 `sections` ⇒ 值是完整段落对象）。
    let raw = view["sections"].as_object().expect("`sections` 仍是对象");
    assert_eq!(
        raw.len(),
        project.sections.len(),
        "派生视图不得增减 `sections` 的条目"
    );
}

#[test]
fn the_timeline_view_is_deterministic() {
    let project = fixture_project();
    let fields = vec!["sections.timeline".to_owned()];
    let first = project_view(&project, &fields).expect("第一次视图");
    let second = project_view(&project, &fields).expect("第二次视图");
    assert_eq!(first, second, "同一工程两次查询必须逐字节相同");
}
