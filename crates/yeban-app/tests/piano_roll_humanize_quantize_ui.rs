//! 卷帘右键菜单的人性化 / 量化入口（ROI #1）：菜单存在、打开对话框、
//! 种子透传、确定性、量化吸附、失败响亮。
//!
//! ## 被验证的链路
//!
//! | 环 | 位置 |
//! | :--- | :--- |
//! | 输入面 | `ui/console/piano_roll.slint` 的右键 `PopupWindow`（语义 ID `piano-roll-context-menu-humanize` / `piano-roll-context-menu-quantize`），**默认不显示** ⇒ 默认帧一位不变 |
//! | 弹窗 | `ui/dialogs/humanize_dialog.slint`（五格文本 + 两枚应用 + 关闭 + 错误行） |
//! | 上抛 | `console_tabs.slint` ⇒ `app.slint`（`humanize-requested` / `quantize-requested`，只透传） |
//! | 宿主消费 | `src/host.rs` 的 `wire_humanize_dialog`：打开时播种（基础种子缺省 = 工程 `rng_seed`）、应用时按 MCP 同一口径解析、一次 `commit_ops`（一步撤销）、重投影卷帘 |
//! | 共享算术 | `yeban_model::clip_ops::{humanize_note, quantize_tick}`（`yeban-mcp` 的 `compile` 与宿主调**同一份**；交叉对照臂钉住） |
//!
//! ## 口径（每一条都是决定，不是意外）
//!
//! 1. 种子是**基础种子**：UI 只透传一个值（缺省工程 `rng_seed`，可显式改）；⛔ 不加每轨种子输入、⛔ 不在 UI 侧按轨编不同种子（按轨派生在引擎侧做，v2 线）。
//! 2. 抖动量**不标分布**：控件文案只写量与单位（ticks / ±n），⛔ 不写\"均匀随机\"（分布由引擎定，v2 会换分布而不改 UI）。
//! 3. 失败**响亮**：解析失败 / 未选中剪辑 / 不是 MIDI 片段 / 模型拒绝 ⇒ 原因写进 `humanize-error`，弹窗**不关**、⛔ 不提交。
//! 4. 空展开**不提交**：量化后没有音符要动（已在网格上或强度 0）⇒ 明文 + 弹窗不关，⛔ 不提交空批次。
//! 5. 一次菜单动作 = **一步撤销**：一次 `Undo` 回到应用前（判据②用一次撤销复位全量断言）。
//! 6. 右键抬起**不触发左键**：右键 `up` 不得落进既有的 `up` 分支（否则卷帘会把这次右键当一次左键点击处理）。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// 而 `-D warnings` 会把它判红（`piano_roll_drag_scroll.rs` 同款处理）。
#[allow(dead_code)]
mod live;

use live::{LiveControlPlane, build_live_ui_with};
use serde_json::json;
use slint::Model as _;
use std::rc::Rc;
// 只在交叉对照臂（`in-process-mcp` 构建）里用 ⇒ 导入本身也要门控，
// 否则默认构建会报 unused import（`-D warnings` 下是硬错）。
#[cfg(feature = "in-process-mcp")]
use yeban_app::host::{humanize_clip_ops_for, quantize_clip_ops_for};
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_model::music::MidiNote;
use yeban_model::ops::Op;
use yeban_model::samples::filled_project;
use yeban_model::{EntityId, YebanProjectV1};
use yeban_ui_mcp::live::request_line;
use yeban_ui_mcp::methods::{
    METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_UP, METHOD_NODE,
};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 权威会话的时间戳（固定值 ⇒ 确定性；判据不依赖真实时钟）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 卷帘拖动面的稳定语义 ID（`piano_roll_drag_scroll.rs` 同款）。
const SCROLL_AREA: &str = "piano-roll-scroll-area";
/// 右键菜单的两项。
const MENU_HUMANIZE: &str = "piano-roll-context-menu-humanize";
const MENU_QUANTIZE: &str = "piano-roll-context-menu-quantize";

/// 装配一台**带权威会话**的活窗口（`d_ui_23_count_in_writes_the_document.rs` 同款）。
/// 返回（窗口，端口，控制面）：端口是读权威工程的**唯一**事实源。
fn assemble() -> (yeban_app::ui::MainWindow, Rc<UndoPort>, LiveControlPlane) {
    let project = filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<piano-roll-humanize-quantize-ui>",
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("打开会话"),
    ));
    let ui = build_live_ui_with(
        &project,
        &live::LiveWiringOptions {
            permission: Permission::Interactive,
            undo: Some(Rc::clone(&port)),
            ..live::LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 执行面");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, port, plane)
}

/// 权威工程里**第一个**带音符的 MIDI 片段：（摆放身份 ULID，音轨身份，片段身份）。
///
/// 按 `tracks` 的 `BTreeMap` 键序（确定）⇒ 每次都是同一条（`filled_project` 的主音轨片段）。
fn first_midi_clip(project: &YebanProjectV1) -> (String, EntityId, EntityId) {
    for (track_id, track) in project.tracks.iter() {
        for placement in track.clips.values() {
            if let Some(entry) = project.clip_pool.get(&placement.clip_id)
                && entry.content.notes().is_some_and(|notes| !notes.is_empty())
            {
                return (
                    placement.id.to_canonical_string(),
                    *track_id,
                    placement.clip_id,
                );
            }
        }
    }
    panic!("filled_project 必须含至少一个带音符的 MIDI 片段");
}

/// 片段音符的（起点，力度，微时值）快照 —— 按 `BTreeMap` 键序（确定），只取应用前后的
/// **可观测**三字段（概率 / 连击不参与本票的断言）。
fn note_state(project: &YebanProjectV1, clip_id: EntityId) -> Vec<(u64, u8, Option<i16>)> {
    let entry = project.clip_pool.get(&clip_id).expect("片段必须在池里");
    let notes = entry.content.notes().expect("必须是 MIDI 片段");
    notes
        .values()
        .map(|note| (note.start_tick, note.velocity, note.micro_timing_ticks))
        .collect()
}

/// 片段音符的**全量**快照（判据⑤的撤销复位用：一步撤销必须把每一字段都还回来）。
fn full_note_state(project: &YebanProjectV1, clip_id: EntityId) -> Vec<MidiNote> {
    let entry = project.clip_pool.get(&clip_id).expect("片段必须在池里");
    let notes = entry.content.notes().expect("必须是 MIDI 片段");
    notes.values().cloned().collect()
}

/// 权威文档的一份快照（读不到 ⇒ 直接判据红：那是前提不成立，⛔ 不是「改动没生效」）。
fn live_project(port: &UndoPort) -> YebanProjectV1 {
    port.try_project()
        .expect("端口必须有一份权威工程（没有 ⇒ 本文件的每一条都无从判起）")
}

/// 把某个剪辑**选中**（走 `.slint` 包头 `clicked` 调的那一个回调；宿主自己校验身份）。
/// 与 `tests/clip_duration.rs` 的 `select_clip` 同款。
fn select_clip(window: &yeban_app::ui::MainWindow, ulid: &str) {
    window.invoke_clip_select(ulid.into());
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        ulid,
        "夹具前提：宿主必须接受这个身份（否则下面的应用判据空转）"
    );
}

/// 取某个**语义 ID** 在运行时控件树里的包围盒。
/// 端口里那份几何会**过期** ⇒ 每次注入前都重读一次（`piano_roll_drag_scroll.rs` 同款）。
fn element_bounds(plane: &mut LiveControlPlane, seq: i64, element_id: &str) -> serde_json::Value {
    let node = plane.plane().try_line(&request_line(
        seq,
        METHOD_NODE,
        Some(json!({ "elementId": element_id })),
    ));
    assert!(
        !node.is_error(),
        "`{element_id}` 必须在运行时控件树里（[UI-TEST-001] 语义 ID 寻址）: {node:?}"
    );
    let bounds = node.result.expect("有 result")["node"]["bounds"].clone();
    assert!(!bounds.is_null(), "`{element_id}` 必须有几何包围盒");
    bounds
}

/// 在某个语义 ID 的中心点按下（`button` = "left" / "right"），再在原地抬起。
fn click_by_id(plane: &mut LiveControlPlane, seq: i64, element_id: &str, button: &str) {
    let bounds = element_bounds(plane, seq, element_id);
    let x_offset = bounds["width"].as_f64().expect("width") / 2.0;
    let y_offset = bounds["height"].as_f64().expect("height") / 2.0;
    let down = plane.plane().try_line(&request_line(
        seq + 1,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": element_id,
            "xOffset": x_offset,
            "yOffset": y_offset,
            "button": button
        })),
    ));
    assert!(
        !down.is_error(),
        "`{element_id}` 按下（{button}）必须落到真实窗口: {down:?}"
    );
    let up = plane.plane().try_line(&request_line(
        seq + 2,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({ "button": button })),
    ));
    assert!(
        !up.is_error(),
        "`{element_id}` 松手（{button}）必须落到真实窗口: {up:?}"
    );
}

/// 在卷帘元素的（x_offset, y_offset）处点一下（左/右键）：端口按元素包围盒 +
/// 偏移算**绝对**位置，因此可以点到盖在卷帘上的弹出层菜单项（见判据①的注释）。
fn click_scroll_at(
    plane: &mut LiveControlPlane,
    seq: i64,
    x_offset: f64,
    y_offset: f64,
    button: &str,
) {
    let down = plane.plane().try_line(&request_line(
        seq + 1,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": SCROLL_AREA,
            "xOffset": x_offset,
            "yOffset": y_offset,
            "button": button
        })),
    ));
    assert!(
        !down.is_error(),
        "卷帘（{x_offset}, {y_offset}）按下（{button}）必须落到真实窗口: {down:?}"
    );
    let up = plane.plane().try_line(&request_line(
        seq + 2,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({ "button": button })),
    ));
    assert!(
        !up.is_error(),
        "卷帘（{x_offset}, {y_offset}）松手（{button}）必须落到真实窗口: {up:?}"
    );
}
/// 某个语义 ID 是否在运行时控件树里（不 panic：用于诊断「弹出层还在不在」）。
fn tree_has(plane: &mut LiveControlPlane, seq: i64, element_id: &str) -> bool {
    let node = plane.plane().try_line(&request_line(
        seq,
        METHOD_NODE,
        Some(json!({ "elementId": element_id })),
    ));
    !node.is_error()
}

/// 只按下（不抬起）：右键菜单在 `down` 那一刻弹出，判据随后在菜单打开的状态下读树。
fn press_by_id(plane: &mut LiveControlPlane, seq: i64, element_id: &str, button: &str) {
    let bounds = element_bounds(plane, seq, element_id);
    let x_offset = bounds["width"].as_f64().expect("width") / 2.0;
    let y_offset = bounds["height"].as_f64().expect("height") / 2.0;
    let down = plane.plane().try_line(&request_line(
        seq + 1,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": element_id,
            "xOffset": x_offset,
            "yOffset": y_offset,
            "button": button
        })),
    ));
    assert!(
        !down.is_error(),
        "`{element_id}` 按下（{button}）必须落到真实窗口: {down:?}"
    );
}

/// 找一个与工程缺省种子**结果不同**的显式种子（确定性扫描 1..=1000 ⇒ 一定找得到；
/// 找不到 ⇒ 判据前提不成立，⛔ 不是「透传没生效」）。
///
/// 用它才能证明「显式种子真的被用了」：若显式值恰好与缺省值撞出同一份抖动，
/// 「结果等于缺省种子算出的那份」就分不清是透传了还是没透传。
fn explicit_seed_that_differs(
    notes: &[MidiNote],
    default_seed: u64,
    time_jitter: u64,
    velocity_jitter: u8,
) -> u64 {
    let baseline: Vec<(i16, u8)> = notes
        .iter()
        .enumerate()
        .map(|(index, note)| {
            yeban_model::clip_ops::humanize_note(
                note,
                default_seed,
                index,
                time_jitter,
                velocity_jitter,
            )
        })
        .collect();
    for seed in 1..=1000_u64 {
        let candidate: Vec<(i16, u8)> = notes
            .iter()
            .enumerate()
            .map(|(index, note)| {
                yeban_model::clip_ops::humanize_note(
                    note,
                    seed,
                    index,
                    time_jitter,
                    velocity_jitter,
                )
            })
            .collect();
        if candidate != baseline {
            return seed;
        }
    }
    panic!("1..=1000 里没有与缺省种子结果不同的种子（前提不成立）");
}

/// 判据 ①：右键菜单的两项存在、能点开对话框；打开时种子预填工程 `rng_seed`。
#[test]
fn the_context_menu_items_exist_and_open_the_dialog() {
    let (window, port, mut plane) = assemble();
    assert_eq!(window.get_console_tab(), 0, "默认底部标签必须是卷帘");
    assert!(
        !window.get_humanize_open(),
        "夹具前提：对话框默认关闭（否则默认帧已被改动）"
    );

    // 注册表双侧对账（与 `piano_roll_drag_scroll.rs` 的判据①同款）。
    for id in [MENU_HUMANIZE, MENU_QUANTIZE, "humanize-dialog"] {
        assert!(
            plane.registry().find_by_id(id).is_some(),
            "`{id}` 必须登记在**注册表**里（[UI-TEST-001]：否则两侧对账会漂移）"
        );
    }

    // 右键按下 ⇒ 菜单弹出（菜单项进运行时树）；右键抬起 ⇒ 菜单**不**关（它是弹出层），
    // 且抬起不得触发左键点击（判据⑤单独钉；这里只钉「菜单还在」）。
    // ⚠ 弹出层子项的树包围盒是**弹出层局部坐标**（x:0, y:0），⛔ 不能按元素 ID +
    // 偏移去点（端口会按树坐标算到窗口左上角去，点不中菜单）。菜单顶左 = 右键点
    // （卷帘中心，远离边缘 ⇒ 不夹），「人性化…」项中心 = 右键点 + (75, 16) ⇒
    // 按卷帘元素的绝对偏移点过去（命中测试由 Slint 做，弹出层在最上层）。
    let scroll_bounds = element_bounds(&mut plane, 1, SCROLL_AREA);
    let center_x = scroll_bounds["width"].as_f64().expect("width") / 2.0;
    let center_y = scroll_bounds["height"].as_f64().expect("height") / 2.0;
    press_by_id(&mut plane, 2, SCROLL_AREA, "right");
    let humanize_bounds = element_bounds(&mut plane, 10, MENU_HUMANIZE);
    let quantize_bounds = element_bounds(&mut plane, 20, MENU_QUANTIZE);
    report_line(&format!(
        "[humanize-ui] 右键菜单包围盒：人性化 {humanize_bounds}；量化 {quantize_bounds}"
    ));
    let up = plane.plane().try_line(&request_line(
        30,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({ "button": "right" })),
    ));
    assert!(!up.is_error(), "右键松手必须落到真实窗口: {up:?}");
    // 抬起之后菜单项仍在树里（弹出层没被抬起关掉）。
    let _ = element_bounds(&mut plane, 40, MENU_HUMANIZE);

    // 点「人性化…」⇒ 对话框打开，焦点落人性化区段，种子预填工程 rng_seed。
    click_scroll_at(&mut plane, 50, center_x + 75.0, center_y + 16.0, "left");
    // 诊断二分（见文件头）：点下去之后菜单还在不在树里 ——
    // 不在 ⇒ 这一击落在弹出层外（位置算错，弹出层被点外关闭）；
    // 还在 ⇒ 这一击落在弹出层内、但菜单项的 `clicked` 没跑（测试后端不把指针事件送进弹出层）。
    let menu_still_there = tree_has(&mut plane, 60, MENU_HUMANIZE);
    report_line(&format!(
        "[humanize-ui] 点菜单后：对话框开={}，菜单仍在树里={menu_still_there}",
        window.get_humanize_open()
    ));
    assert!(
        window.get_humanize_open(),
        "点菜单「人性化…」必须打开对话框"
    );
    assert_eq!(
        window.get_humanize_focus_section(),
        0,
        "从人性化入口打开 ⇒ 焦点落人性化区段"
    );
    let rng_seed = live_project(&port).rng_seed;
    assert_eq!(
        window.get_humanize_seed().as_str(),
        rng_seed.to_string(),
        "种子必须预填工程 rng_seed（{rng_seed}），⛔ 不是一个藏起来的缺省"
    );
    report_line(&format!(
        "[humanize-ui] 对话框已开，种子预填 rng_seed={rng_seed}"
    ));

    // 关掉，再从量化入口打开 ⇒ 焦点落量化区段（同一弹窗，两个入口只差焦点落点）。
    window.invoke_close_humanize();
    assert!(!window.get_humanize_open(), "关闭必须关掉对话框");
    window.invoke_open_quantize();
    assert!(window.get_humanize_open(), "量化入口必须打开同一弹窗");
    assert_eq!(
        window.get_humanize_focus_section(),
        1,
        "从量化入口打开 ⇒ 焦点落量化区段"
    );
}

/// 判据 ②：人性化用**显式种子**、且同一份种子两次结果相同、一次撤销回到应用前。
#[test]
fn humanize_applies_with_the_explicit_seed_and_is_deterministic() {
    let (window, port, _plane) = assemble();
    let project = live_project(&port);
    let (placement_ulid, _track_id, clip_id) = first_midi_clip(&project);
    let original = full_note_state(&project, clip_id);
    let original_state = note_state(&project, clip_id);
    select_clip(&window, &placement_ulid);

    // 显式种子：扫描一个与缺省种子结果不同的值（见 `explicit_seed_that_differs`）。
    let seed = explicit_seed_that_differs(&original, project.rng_seed, 20, 10);
    report_line(&format!(
        "[humanize-ui] 显式种子 {seed}（工程 rng_seed={}，两者结果不同）",
        project.rng_seed
    ));

    window.invoke_open_humanize();
    assert!(window.get_humanize_open(), "夹具前提：对话框必须打开");
    window.set_humanize_seed(seed.to_string().into());
    window.set_humanize_time_jitter("20".into());
    window.set_humanize_velocity_jitter("10".into());
    window.invoke_apply_humanize();
    assert!(
        !window.get_humanize_open(),
        "应用成功 ⇒ 对话框关闭（失败才留着）"
    );
    assert!(
        window.get_humanize_error().is_empty(),
        "应用成功 ⇒ 错误行必须为空：{}",
        window.get_humanize_error()
    );
    let applied = note_state(&live_project(&port), clip_id);
    // 期望值：同一份共享算术（`clip_ops::humanize_note`）+ 显式种子算出。
    let expected: Vec<(u64, u8, Option<i16>)> = original
        .iter()
        .enumerate()
        .map(|(index, note)| {
            let (micro, velocity) = yeban_model::clip_ops::humanize_note(note, seed, index, 20, 10);
            (note.start_tick, velocity, Some(micro))
        })
        .collect();
    assert_eq!(
        applied, expected,
        "人性化必须用显式种子 {seed} 算出的值（⛔ 不是工程缺省种子那份）"
    );
    // 与缺省种子算出的那份**不同** ⇒ 上一条等式不是\"两个种子恰好撞车\"的假绿。
    let default_expected: Vec<(u64, u8, Option<i16>)> = original
        .iter()
        .enumerate()
        .map(|(index, note)| {
            let (micro, velocity) =
                yeban_model::clip_ops::humanize_note(note, project.rng_seed, index, 20, 10);
            (note.start_tick, velocity, Some(micro))
        })
        .collect();
    assert_ne!(
        applied, default_expected,
        "显式种子 {seed} 的结果必须与缺省种子不同（否则透传无从证明）"
    );

    // 一次撤销 ⇒ 全量回到应用前（一步撤销覆盖整条片段的全部音符）。
    let undone = port.perform(UiAction::Undo);
    assert!(undone.changed(), "撤销必须有变化（应用确实提交过）");
    assert_eq!(
        full_note_state(&live_project(&port), clip_id),
        original,
        "一次撤销必须把全部音符还回应用前（一次菜单动作 = 一步撤销）"
    );
    assert_eq!(
        note_state(&live_project(&port), clip_id),
        original_state,
        "撤销后的快照必须与应用前逐字相同"
    );

    // 同一份种子再应用一次 ⇒ 与第一次逐字相同（确定性：同一份种子 ⇒ 同一份输出）。
    window.invoke_open_humanize();
    window.set_humanize_seed(seed.to_string().into());
    window.set_humanize_time_jitter("20".into());
    window.set_humanize_velocity_jitter("10".into());
    window.invoke_apply_humanize();
    assert!(!window.get_humanize_open(), "第二次应用成功 ⇒ 对话框关闭");
    assert_eq!(
        note_state(&live_project(&port), clip_id),
        applied,
        "同一份种子第二次应用 ⇒ 必须与第一次逐字相同"
    );
}

/// 判据 ③：量化把离格音符吸到网格；强度 0 ⇒ 什么都不改、弹窗不关、给明文。
#[test]
fn quantize_snaps_off_grid_notes_and_strength_zero_changes_nothing() {
    let (window, port, _plane) = assemble();
    let project = live_project(&port);
    let (placement_ulid, _track_id, clip_id) = first_midi_clip(&project);
    let original = full_note_state(&project, clip_id);
    select_clip(&window, &placement_ulid);

    // 夹具手术：把第一个音符（起点 0）挪到 100（离格）—— 这是本判据自己的
    // 前置条件，⛔ 不是被测代码（被测的是下面的量化应用）。
    let first_note_id = *project
        .clip_pool
        .get(&clip_id)
        .expect("片段必须在池里")
        .content
        .notes()
        .expect("必须是 MIDI 片段")
        .keys()
        .next()
        .expect("片段必须有音符");
    let first_note = original
        .iter()
        .find(|note| note.id == first_note_id)
        .expect("快照里必须有第一个音符");
    port.commit_ops(
        NOW_MS,
        "test setup: move a note off grid",
        vec![Op::SetNoteTiming {
            track_id: _track_id,
            clip_id,
            note_id: first_note_id,
            old_start_tick: first_note.start_tick,
            old_duration_ticks: first_note.duration_ticks,
            new_start_tick: 100,
            new_duration_ticks: first_note.duration_ticks,
        }],
    )
    .expect("夹具手术必须提交成功");
    assert_eq!(
        note_state(&live_project(&port), clip_id)[0].0,
        100,
        "夹具前提：第一个音符已离格（起点 100）"
    );

    // 网格 240、强度 100% ⇒ 100 吸到 0（(100+120)/240*240），其余已在格上的不动。
    window.invoke_open_quantize();
    window.set_humanize_grid("240".into());
    window.set_humanize_strength("100".into());
    window.invoke_apply_quantize();
    assert!(
        !window.get_humanize_open(),
        "量化成功 ⇒ 对话框关闭（失败才留着）"
    );
    assert!(
        window.get_humanize_error().is_empty(),
        "量化成功 ⇒ 错误行必须为空：{}",
        window.get_humanize_error()
    );
    let starts: Vec<u64> = note_state(&live_project(&port), clip_id)
        .iter()
        .map(|(start, _, _)| *start)
        .collect();
    assert_eq!(
        starts,
        vec![0, 960, 1920, 2880],
        "量化必须把 100 吸到 0，其余已在格上的音符一位不动"
    );

    // 再把同一个音符挪离格，强度 0 ⇒ 不产生任何 op ⇒ 明文 + 弹窗不关 + 音符不动。
    port.commit_ops(
        NOW_MS,
        "test setup: move a note off grid again",
        vec![Op::SetNoteTiming {
            track_id: _track_id,
            clip_id,
            note_id: first_note_id,
            old_start_tick: 0,
            old_duration_ticks: first_note.duration_ticks,
            new_start_tick: 100,
            new_duration_ticks: first_note.duration_ticks,
        }],
    )
    .expect("夹具手术必须提交成功");
    window.invoke_open_quantize();
    window.set_humanize_grid("240".into());
    window.set_humanize_strength("0".into());
    window.invoke_apply_quantize();
    assert!(
        window.get_humanize_open(),
        "强度 0 ⇒ 没有音符要动 ⇒ 弹窗必须**不关**"
    );
    assert!(
        !window.get_humanize_error().is_empty(),
        "强度 0 ⇒ 必须给明文（⛔ 不静默）"
    );
    assert_eq!(
        note_state(&live_project(&port), clip_id)[0].0,
        100,
        "强度 0 ⇒ 音符起点必须一位不动"
    );
    report_line(&format!(
        "[humanize-ui] 强度 0 的明文：{}",
        window.get_humanize_error()
    ));
}

/// 判据 ④：未选中剪辑 / 非法输入 ⇒ 响亮失败、弹窗不关、工程一位不动。
#[test]
fn apply_without_a_selected_clip_or_with_bad_input_fails_loudly() {
    let (window, port, _plane) = assemble();
    let project = live_project(&port);
    let (_placement_ulid, _track_id, clip_id) = first_midi_clip(&project);
    let before = full_note_state(&project, clip_id);

    // 未选中剪辑 ⇒ 应用人性化必须失败、弹窗不关、工程不动。
    window.set_selected_clip_ulid("".into());
    window.invoke_open_humanize();
    window.set_humanize_seed("42".into());
    window.set_humanize_time_jitter("20".into());
    window.set_humanize_velocity_jitter("10".into());
    window.invoke_apply_humanize();
    assert!(window.get_humanize_open(), "未选中剪辑 ⇒ 弹窗必须**不关**");
    assert!(
        window.get_humanize_error().contains("未选中剪辑"),
        "未选中剪辑 ⇒ 错误行必须明说原因：{}",
        window.get_humanize_error()
    );
    assert_eq!(
        full_note_state(&live_project(&port), clip_id),
        before,
        "未选中剪辑 ⇒ 工程必须一位不动（⛔ 不提交）"
    );

    // 非法种子（不是数字）⇒ 同样响亮失败、弹窗不关。
    let (placement_ulid, _, _) = first_midi_clip(&live_project(&port));
    select_clip(&window, &placement_ulid);
    window.set_humanize_seed("abc".into());
    window.invoke_apply_humanize();
    assert!(window.get_humanize_open(), "非法种子 ⇒ 弹窗必须**不关**");
    assert!(
        window.get_humanize_error().contains("基础种子"),
        "非法种子 ⇒ 错误行必须点名是种子的问题：{}",
        window.get_humanize_error()
    );
    assert_eq!(
        full_note_state(&live_project(&port), clip_id),
        before,
        "非法种子 ⇒ 工程必须一位不动（⛔ 不提交）"
    );
}

/// 判据 ⑤：右键抬起不得触发左键点击 —— 先选中一个音符，右键按 + 抬之后
/// 选区与音符数必须一位不动（若抬起落进左键 `up` 分支，空地点击会清掉选区）。
#[test]
fn right_click_release_does_not_fire_a_left_click() {
    let (window, port, mut plane) = assemble();
    let project = live_project(&port);
    let (placement_ulid, _track_id, clip_id) = first_midi_clip(&project);
    select_clip(&window, &placement_ulid);
    let note_id = project
        .clip_pool
        .get(&clip_id)
        .expect("片段必须在池里")
        .content
        .notes()
        .expect("必须是 MIDI 片段")
        .keys()
        .next()
        .expect("片段必须有音符")
        .to_canonical_string();
    // 选中一个音符（`clip_duration.rs` 的 `select_notes` 同款）。
    window.set_selected_ulids(slint::ModelRc::new(slint::VecModel::from(vec![
        slint::SharedString::from(note_id.as_str()),
    ])));
    assert_eq!(
        window.get_selected_ulids().iter().count(),
        1,
        "夹具前提：必须选中恰好一个音符"
    );
    let note_count = note_state(&live_project(&port), clip_id).len();

    // 右键按 + 抬（卷帘空白处中央）。
    click_by_id(&mut plane, 1, SCROLL_AREA, "right");

    assert_eq!(
        note_state(&live_project(&port), clip_id).len(),
        note_count,
        "右键抬起不得插入音符（左键点击才插）"
    );
    let selected: Vec<String> = window
        .get_selected_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    assert_eq!(
        selected,
        vec![note_id],
        "右键抬起不得清掉音符选区（左键空地点击会清）"
    );
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        placement_ulid,
        "右键抬起不得改动剪辑选中态"
    );
}

/// 判据 ⑥（交叉对照，`in-process-mcp` 构建才有）：宿主的人性化 / 量化展开
/// 必须与 MCP `notes::compile` 的产物**逐字节**相同。
///
/// 这是「同一份实现」的证据：两条路（MCP 工具与 UI 菜单）调的是同一批共享算术，
/// ⛔ 不是各写一份展开。
#[test]
#[cfg(feature = "in-process-mcp")]
fn host_expansion_matches_mcp_compile_for_humanize_and_quantize() {
    use yeban_mcp::domain::notes::{NoteOp, compile};

    let project = filled_project();
    let (placement_ulid, track_id, clip_id) = first_midi_clip(&project);

    // 人性化：种子 42、时值 20、力度 10。
    let host_humanized = humanize_clip_ops_for(&project, &placement_ulid, 42, 20, 10)
        .expect("夹具前提：宿主的人性化展开必须成功");
    let mcp_humanized = compile(
        &project,
        &track_id,
        &clip_id,
        &[NoteOp::HumanizeClip {
            seed: 42,
            time_jitter_ticks: 20,
            velocity_jitter: 10,
        }],
    )
    .expect("夹具前提：MCP 的人性化展开必须成功");
    assert_eq!(
        host_humanized, mcp_humanized,
        "人性化：宿主展开必须与 MCP `compile` 逐字节相同"
    );

    // 量化：网格 240、强度 100% 与 50% 两档（插值臂也要对上）。
    for strength in [1000_u16, 500] {
        let host_quantized = quantize_clip_ops_for(&project, &placement_ulid, 240, strength)
            .expect("夹具前提：宿主的量化展开必须成功");
        let mcp_quantized = compile(
            &project,
            &track_id,
            &clip_id,
            &[NoteOp::QuantizeClip {
                grid_ticks: 240,
                strength_permille: strength,
            }],
        )
        .expect("夹具前提：MCP 的量化展开必须成功");
        assert_eq!(
            host_quantized, mcp_quantized,
            "量化（强度 {strength}‰）：宿主展开必须与 MCP `compile` 逐字节相同"
        );
    }
    report_line("[humanize-ui] 交叉对照：宿主展开 == MCP compile（人性化 + 量化两档）");
}
