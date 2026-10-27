//! [G15] `sections.timeline` 派生视图的判据（`yeban_query_project` 的段落时间轴读数）。
//!
//! | # | 判据 | 用例 |
//! | :--- | :--- | :--- |
//! | a | 数组逐条等于**独立重算**的 `(start_tick, name)` 升序表；且身份序 ≠ 时间序（排序**有 observable 差**，删掉 sort 必红） | `sections_timeline_is_sorted_by_start_tick_not_by_identity` |
//! | b | `sections` 原始形状**不变**（派生视图走独立键 `sectionsTimeline`） | 同上（`assert_ne` 那条 + 形状断言） |
//! | c | 同一工程两次调用逐字节相同（确定性） | `the_timeline_view_is_deterministic` |
//! | d | 只选 `sections.timeline` 时根键 `sections` 也在，且与单独选 `sections` **逐字节同形**（单元判据"每个选择器都取得到根键"的正向钉） | `the_root_projection_equals_the_plain_selector` |

use std::str::FromStr;

use serde_json::Value;
use yeban_mcp::domain::view::project_view;
use yeban_model::samples::filled_project;
use yeban_model::{EntityId, SectionV3, YebanProjectV1};

/// 固定身份：只有末位随 `seed` 增长 ⇒ `id(1) < id(2) < id(3)` 是**文本事实**，
/// 与运行时刻和随机数无关（照 `tests/render_master.rs` 的同名夹具）。
fn id(seed: u32) -> EntityId {
    let text = format!("01J8ZR{:0>16}{seed:04}", 0);
    assert_eq!(text.len(), 26, "ULID 必须 26 字符");
    EntityId::from_str(&text).expect("合法 Crockford Base32 ULID")
}

/// 造一个带三段"身份序与时间序**必然相反**"的夹具工程。
///
/// ⚠ 身份**必须钉死**, ⛔ 不能用 `EntityId::new()`：它的低位是随机的（同一毫秒内
///   `Ord` 由随机位决定），于是"身份序 ≠ 时间序"这条断言变成掷硬币 —— CI Run
///   40468119126 的 Windows 腿就掷出了"三段的 id 序恰好与 tick 序同向", 判据 b
///   因此误红。钉死 id 后 `id(1)@30k` 排在 `id(2)@20k` 之前是**夹具属性**：
///   无论 `filled_project` 自带段落在哪个位置, 这两段在两个序列里的相对次序
///   必定相反, 整条序列必不相等。
/// ⚠ 名字**必须避开** `filled_project` 自带的段落名（`Intro`/`Drop`/…）：
///   CI 实测 `Intro` 会与自带段**同名不同条**，名字层面的"序不同"断言因此误报。
fn fixture_project() -> YebanProjectV1 {
    let mut project = filled_project();
    for (seed, name, start) in [
        (1, "FixtureOutro", 30_000u64),
        (2, "FixtureBridge", 20_000),
        (3, "FixtureBuild", 10_000),
    ] {
        let entry = SectionV3 {
            id: id(seed),
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

    // 排序必须**observable**：身份序**整条序列** ≠ 时间序整条序列。
    // ⚠ 不能只比"第一条"：`filled_project` 自带段的身份序与 tick 序恰好同向
    //   （fixture id 小且 tick 也小），CI 实测单条名字比较会**假绿/假红**；
    //   夹具的三段降序追加只在序列层面必然翻转。
    let identity_names: Vec<&str> = project
        .sections
        .values()
        .map(|section| section.name.as_str())
        .collect();
    let timeline_names: Vec<&str> = view["sectionsTimeline"]
        .as_array()
        .expect("`sectionsTimeline` 必须是数组")
        .iter()
        .map(|entry| entry["name"].as_str().expect("name"))
        .collect();
    assert_ne!(
        identity_names, timeline_names,
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

/// ⭐ CI Run 40453970637 的真实失败（`view.rs:368` 单元判据
/// `every_selector_resolves_against_a_filled_project`）正向钉住：
/// 下钻选择器的**根键**必须在输出里取得到，且形状与单独选择该根**逐字节同形**。
/// 派生视图只许**额外**多出一个 `sectionsTimeline` 键，⛔ 不许拿掉或改写根键。
#[test]
fn the_root_projection_equals_the_plain_selector() {
    let project = fixture_project();
    let drilled = project_view(&project, &["sections.timeline".to_owned()]).expect("下钻视图");
    let plain = project_view(&project, &["sections".to_owned()]).expect("整份视图");
    assert!(
        drilled.get("sections").is_some(),
        "下钻选择器 `sections.timeline` 的输出必须带根键 `sections` \
         (否则白名单与数据模型漂移, 单元判据会红)"
    );
    assert_eq!(
        drilled["sections"], plain["sections"],
        "`sections.timeline` 附带的 `sections` 投影必须与单独选 `sections` 逐字节同形"
    );
    // 派生键确实**额外**存在（不是靠改写根键把时间轴塞进去）。
    assert!(
        plain.get("sectionsTimeline").is_none(),
        "单独选 `sections` 不得冒出 `sectionsTimeline`（派生视图是下钻专属）"
    );
}
