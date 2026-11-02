//! `D-UI-13`：卷帘的**命中测试不分作用域** ⇒ 多轨 / 多剪辑工程里"点得到的音符"几乎不存在。
//!
//! ## 缺陷（本文件的判据就是它的机械见证）
//!
//! [`ViewState::hit_test_visible`] 在**整个片段池**的可见音符里按"重叠时取下标最大的那颗"择优，
//! 而宿主 `install_roll_click` 是在它**之后**才用 `clip_id` 把结果丢掉（`.filter(...)`）。
//! 卷帘画的却只有作用域（选中的剪辑）里的那批音符（`visible_notes_scoped` 在**装数组之前**
//! 就 `retain` 了）⇒ 两处的口径相反：
//!
//! 只要**另一个剪辑**的音符落在同一个 x/y 上，择优就返回那颗**看不见**的音符，事后过滤 ⇒ `None`
//! ⇒ 选区为空 ⇒ 选择 / 橡皮擦 / 剪刀 / 裸箭头 / `Shift+箭头` 全部静默失效（⛔ 没有拒绝文本、
//! ⛔ 没有日志、撤销栈一位不动）。而"另一个剪辑的音符落在同一个 x/y 上"在真实工程里是**常态**：
//! ① 音符的 x 是**剪辑内**口径（`note_view` 用 `note.start_tick`，不含摆放起点）⇒ 每个剪辑的第
//! 一颗音符都画在同一列；② 纵向只有 16 条泳道（`PITCH_LANE_COUNT`）⇒ 音高折叠后同泳道概率极高。
//!
//! **实测症状**（dist9 活体，`/tmp/zimmer_creative/probe_hit_scan.py`）：第二首曲子的 6 个"建成
//! 且记了音符"的剪辑里，用橡皮擦点第一颗音符的中心，**只有 1 个**删得掉（`1/6`），其余 5 个
//! 逐字返回"点不点得中 = ❌ 无"；对同一个点做整列 y 扫描（0..365 步 7）与 11 个候选偏移，
//! **0 命中** —— 于是排除了"坐标口径错"和"快照过期"两条假设，只剩作用域这一条。
//! 后果：316 个目标音符里 144 个（45.6%）在 UI 面上写不进去（时值改不动，因为改时值的
//! `Shift+←/→` 需要**非空选区**）。
//!
//! ## 修法（本文件钉住的口径）
//!
//! `hit_test_visible_scoped(.., clip_scope)`：**择优之前**按 `clip_id` 过滤，与
//! `visible_notes_scoped` 同一口径；`hit_test_visible` 变成 `None` 作用域的包装（逐项不变，
//! 既有判据不受影响）。宿主 `install_roll_click` 与 `begin_note_drag` 都改用带作用域的那一个
//! ⇒ 点击选中与拖拽抓取的是**屏幕上那一批**音符，且重叠时仍按"最后绘制"择优（只在作用域内）。
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ① `the_unscoped_hit_test...` | 纯投影层：同一个点，**不分作用域**解析到的是那颗**没画出来**的影子音符（= 缺陷本体）；**分作用域**解析到的正是正在编辑的那颗；`None` 作用域与旧函数逐项相同（包装没有改变语义） |
//! | ② `a_click_on_a_shadowed_note_in_the_selected_clip_selects_it` | 真窗口 ＋ `wire_roll_edit` ＋ 选中该剪辑：点影子覆盖下的音符中心 ⇒ `selected-note-count == 1` 且身份是**正在编辑**的那颗（修复前恒为 0） |
//! | ③ `the_eraser_click_deletes_the_shadowed_note_of_the_selected_clip_only` | 同一个点换橡皮擦 ⇒ 权威工程里**只**少正在编辑剪辑的那一颗，影子剪辑一颗不少 |
//! | ④ `a_note_drag_grabs_the_visible_note_not_the_shadowing_one` | 拖拽臂（`begin_note_drag` 的第二次消费点）：被移动的是**看得见**的那颗（修复前被移动的是影子那颗，看得见的那颗一位没动） |
//! | ⑤ `an_unselected_roll_still_targets_the_topmost_drawn_note` | **控制臂**：没有选中任何剪辑（作用域 `None`）时点同一个点 ⇒ 选中的是**画在最上层**的那颗（= 影子）。它证明 ②③④ 的红来自"作用域"，不是坐标 / 夹具坏了 |
//!
//! ## 未一并处理的同类（如实登记，⛔ 不假装）
//!
//! 力度柱那条链是**同一个形状**：`bridge::velocity_bar_hit_test`（`bridge.rs` 里那句
//! "与 `Self::hit_test_visible` 同一取舍"）也没有作用域参数，`host::velocity_bar_at` 的调用方
//! 同样是**事后**过滤。本票只接音符矩形那条（第二首曲子的阻塞项是**选区**，不是力度），
//! 力度柱变体登记进 `docs/DEVELOPMENT_LEDGER.md` 与审核报告，⛔ 不在本票里顺手改。
//!
//! ## 平台约束
//!
//! 判据 ① 纯投影、不建窗口。其余走 `Tier1Window`（与 `tests/roll_edit_tools.rs` 同一装配：
//! libtest 每个用例起新线程 ⇒ 同文件多个 `#[test]` 是既有做法）。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// 而 `-D warnings` 会判红（与 `piano_roll_drag_scroll.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use std::collections::BTreeMap;
use std::rc::Rc;
use std::str::FromStr as _;

use live::{LiveControlPlane, build_live_ui};
use serde_json::json;
use slint::Model as _;
use yeban_app::bridge::{ViewState, host_to_painted};
use yeban_app::host::{ROLL_SNAP_GRID_TICKS, wire_roll_edit};
use yeban_app::input::Tool;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::{ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote};
use yeban_ui_mcp::live::request_line;
use yeban_ui_mcp::methods::{
    METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE, METHOD_DISPATCH_POINTER_UP,
    METHOD_NODE,
};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 拖动面（点击源）的**稳定语义 ID**（`[UI-TEST-001]` §12.2）。
const SCROLL_AREA: &str = "piano-roll-scroll-area";

/// 音符矩形高度（与 `host::NOTE_HEIGHT_PX` / `piano_roll.slint` 的 `12px` 同一读数）。
const NOTE_HEIGHT: f32 = 12.0;

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;

/// 判据 ① 用的视口宽度：影子与靶子都画在 `x ≈ 0..112`，任何更大的宽度都等价；
/// 取 `tests/roll_scope.rs` 同款窗口宽度，避免"窗口太窄所以没进可见集"这种假绿。
const FLAT_VIEWPORT_WIDTH: f32 = 1_440.0;

/// 拖拽位移（**宿主帧**）：与 `tests/roll_edit_tools.rs` 的 `DRAG_HOST_DX/DY` 逐字相同 ——
/// 右移 16px 在 30 tpp 下恰好跨 2 个 240-tick 网格，上移 28px 恰好跨 2 条 14px 泳道。
const DRAG_HOST_DX: f32 = 16.0;
const DRAG_HOST_DY: f32 = -28.0;
const DRAG_STEPS: usize = 4;

/// 影子剪辑的池条目身份 / 摆放身份 / 音符身份。
///
/// 与 `yeban-model` 的样本同一套确定性 ULID 方案（`01J8ZQ` ＋ 20 位十进制序号），
/// 于是 `900 > 10` 在 `BTreeMap` 键序上成立 ⇒ 影子音符在 `view.notes` 里**排在后面**
/// = "最后绘制"那颗 = 旧命中测试会选中的那颗。判据 ① 会把这个前提自己钉住。
const SHADOW_CLIP: u128 = 900;
const SHADOW_PLACEMENT: u128 = 905;
const SHADOW_NOTE_BASE: u128 = 910;

/// 正在编辑的剪辑（样本里唯一带 MIDI 音符的池条目）在工程里的位置。
struct Edited {
    clip_id: EntityId,
    track_id: EntityId,
    placement_id: EntityId,
    /// `(start_tick, pitch, duration_ticks)`，按 `BTreeMap` 键序。
    shapes: Vec<(u64, u8, u64)>,
}

fn fixture_id(index: u128) -> EntityId {
    EntityId::from_str(&format!("01J8ZQ{index:020}")).expect("canonical fixture ulid")
}

/// 在**加入影子之前**的样本里找出正在编辑的 MIDI 剪辑与它的摆放置。
fn edited_of(project: &yeban_model::YebanProjectV1) -> Edited {
    let mut midi_clips: Vec<EntityId> = project
        .clip_pool
        .iter()
        .filter(|(_, entry)| entry.content.notes().is_some())
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(
        midi_clips.len(),
        1,
        "夹具前提：样本必须**恰好一个**带 MIDI 音符的池条目，实际 {midi_clips:?}"
    );
    let clip_id = midi_clips.pop().expect("上面已断言非空");
    let notes = project
        .clip_pool
        .get(&clip_id)
        .expect("池条目在场")
        .content
        .notes()
        .expect("MIDI 内容")
        .values()
        .map(|note| (note.start_tick, note.pitch, note.duration_ticks))
        .collect::<Vec<_>>();
    assert!(!notes.is_empty(), "夹具前提：正在编辑的剪辑必须有音符");

    let mut placements: Vec<(EntityId, EntityId)> = Vec::new();
    for (track_id, track) in &project.tracks {
        for placement in track.clips.values() {
            if placement.clip_id == clip_id {
                placements.push((*track_id, placement.id));
            }
        }
    }
    assert_eq!(
        placements.len(),
        1,
        "夹具前提：该剪辑必须被摆放**恰好一次**，实际 {placements:?}"
    );
    let (track_id, placement_id) = placements.pop().expect("上面已断言非空");
    Edited {
        clip_id,
        track_id,
        placement_id,
        shapes: notes,
    }
}

/// `[D-UI-13]` 的夹具工程：样本 ＋ **一个影子剪辑**。
///
/// 影子剪辑的每颗音符与正在编辑的那颗**逐值同 `start_tick`、同 `pitch`**（⛔ 同身份），
/// 摆在同一条轨的**下一小节**（摆放区间不重叠 ⇒ 工程合法且编排视图正常投影）。
/// 由于音符 x 是**剪辑内**口径（`note_view` 用 `note.start_tick`），两批音符在卷帘里**逐像素重合**。
fn shadowed_project() -> yeban_model::YebanProjectV1 {
    let mut project = yeban_model::samples::filled_project();
    let edited = edited_of(&project);

    let mut shadow_notes: BTreeMap<EntityId, MidiNote> = BTreeMap::new();
    for (index, (start_tick, pitch, duration)) in edited.shapes.iter().enumerate() {
        let note_id =
            fixture_id(SHADOW_NOTE_BASE + u128::try_from(index).expect("夹具序号必然放得进 u128"));
        shadow_notes.insert(
            note_id,
            MidiNote::new(note_id, *start_tick, *pitch, *duration),
        );
    }
    let shadow_clip = fixture_id(SHADOW_CLIP);
    project.clip_pool.insert(
        shadow_clip,
        ClipPoolEntry {
            id: shadow_clip,
            name: "Shadow".to_owned(),
            content: ClipContent::Midi {
                notes: shadow_notes,
            },
        },
    );
    let shadow_placement = fixture_id(SHADOW_PLACEMENT);
    project
        .tracks
        .get_mut(&edited.track_id)
        .expect("展示轨在场")
        .clips
        .insert(
            shadow_placement,
            ClipPlacement {
                id: shadow_placement,
                clip_id: shadow_clip,
                start_tick: 3_840,
                duration_ticks: 3_840,
                loop_config: LoopConfig::default(),
                muted: false,
            },
        );
    project
}

/// 权威工程里某个剪辑的音符身份（升序）。
fn clip_note_ids(project: &yeban_model::YebanProjectV1, clip: EntityId) -> Vec<String> {
    project
        .clip_pool
        .get(&clip)
        .expect("池条目在场")
        .content
        .notes()
        .expect("MIDI 内容")
        .values()
        .map(|note| note.id.to_canonical_string())
        .collect()
}

/// 按 ULID 从**权威工程**读一个音符的 `(start_tick, pitch, duration_ticks)`。
fn note_by_ulid(project: &yeban_model::YebanProjectV1, ulid: &str) -> (u64, u8, u64) {
    for entry in project.clip_pool.values() {
        let Some(map) = entry.content.notes() else {
            continue;
        };
        for (id, note) in map {
            if id.to_canonical_string() == ulid {
                return (note.start_tick, note.pitch, note.duration_ticks);
            }
        }
    }
    panic!("权威工程里必须有音符 `{ulid}`");
}

/// 装配：真窗口 ＋ 端口 ＋ **与产品同一条** `wire_roll_edit`（夹具是 [`shadowed_project`]）。
fn assemble() -> (MainWindow, Rc<UndoPort>, LiveControlPlane) {
    let project = shadowed_project();
    let ui = build_live_ui(&project, Permission::Interactive).expect("真实界面 + Tier-1 执行面");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<d-ui-13>", "yeban-app", project, NOW_MS).expect("打开"),
    ));
    wire_roll_edit(ui.ui(), &port);
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, port, plane)
}

/// 语义 ID 的包围盒。
fn element_bounds(plane: &mut LiveControlPlane, seq: i64, element_id: &str) -> serde_json::Value {
    let node = plane.plane().try_line(&request_line(
        seq,
        METHOD_NODE,
        Some(json!({ "elementId": element_id })),
    ));
    assert!(
        !node.is_error(),
        "`{element_id}` 必须在运行时树里: {node:?}"
    );
    let bounds = node.result.expect("有 result")["node"]["bounds"].clone();
    assert!(!bounds.is_null(), "`{element_id}` 必须有几何包围盒");
    bounds
}

/// **宿主帧**下"点在音符中心"（tick 0 = x 0，泳道顶 = y 0）。
fn note_center_in_host_view(view: &ViewState, index: usize, scroll: f32) -> (f32, f32) {
    let note = &view.notes[index];
    (
        note.x - scroll + note.width / 2.0,
        note.y + NOTE_HEIGHT / 2.0,
    )
}

/// **绘制帧**（人看得见的矩形）下"点在音符中心"：宿主中心经唯一的 [`host_to_painted`] 搬过去。
fn note_center_in_painted_view(view: &ViewState, index: usize, scroll: f32) -> (f32, f32) {
    let (x, y) = note_center_in_host_view(view, index, scroll);
    host_to_painted(x, y)
}

/// 在卷帘的**绘制帧** `from` 处注入一次完整点击（`down → move → up`，位移 0 ⇒ 越过 4px 阈值以下）。
fn click_in_roll_at(plane: &mut LiveControlPlane, seq: i64, from: (f32, f32)) {
    let bounds = element_bounds(plane, seq, SCROLL_AREA);
    let origin_x = bounds["x"].as_f64().expect("x") as f32;
    let origin_y = bounds["y"].as_f64().expect("y") as f32;
    let down = plane.plane().try_line(&request_line(
        seq + 1,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": SCROLL_AREA,
            "xOffset": f64::from(from.0),
            "yOffset": f64::from(from.1),
            "button": "left"
        })),
    ));
    assert!(!down.is_error(), "按下必须落到真实窗口: {down:?}");
    let moved = plane.plane().try_line(&request_line(
        seq + 2,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({
            "x": f64::from(origin_x + from.0),
            "y": f64::from(origin_y + from.1)
        })),
    ));
    assert!(!moved.is_error(), "移动必须落到真实窗口: {moved:?}");
    let up = plane.plane().try_line(&request_line(
        seq + 3,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({ "button": "left" })),
    ));
    assert!(!up.is_error(), "松手必须落到真实窗口: {up:?}");
}

/// 按下 → 分步拖动 → 松手（一次完整拖拽手势；与 `tests/roll_edit_tools.rs` 同一手法）。
fn drag_in_roll_at(plane: &mut LiveControlPlane, seq: i64, from: (f32, f32)) {
    let bounds = element_bounds(plane, seq, SCROLL_AREA);
    let origin = (
        bounds["x"].as_f64().expect("x") as f32,
        bounds["y"].as_f64().expect("y") as f32,
    );
    let down = plane.plane().try_line(&request_line(
        seq + 1,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": SCROLL_AREA,
            "xOffset": f64::from(from.0),
            "yOffset": f64::from(from.1),
            "button": "left"
        })),
    ));
    assert!(!down.is_error(), "按下必须落到真实窗口: {down:?}");
    for step in 1..=DRAG_STEPS {
        #[allow(clippy::cast_precision_loss)]
        let fraction = step as f32 / DRAG_STEPS as f32;
        let x = origin.0 + from.0 + DRAG_HOST_DX * fraction;
        let y = origin.1 + from.1 + DRAG_HOST_DY * fraction;
        let moved = plane.plane().try_line(&request_line(
            seq + 1 + i64::try_from(step).expect("步号"),
            METHOD_DISPATCH_POINTER_MOVE,
            Some(json!({ "x": f64::from(x), "y": f64::from(y) })),
        ));
        assert!(
            !moved.is_error(),
            "第 {step} 步移动必须落到真实窗口: {moved:?}"
        );
    }
    let up = plane.plane().try_line(&request_line(
        seq + 1 + i64::try_from(DRAG_STEPS).expect("步数") + 1,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({ "button": "left" })),
    ));
    assert!(!up.is_error(), "松手必须落到真实窗口: {up:?}");
}

/// 界面属性 `selected-ulids` 的当前读数。
fn selected_ids(ui: &MainWindow) -> Vec<String> {
    ui.get_selected_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect()
}

/// 靶子：正在编辑剪辑的**第一颗**音符，以及覆盖它的影子音符（两者几何逐值相同）。
struct Target {
    /// 靶子的 `view.notes` 下标与身份。
    edited_index: usize,
    edited_ulid: String,
    /// 影子音符的身份（`view.notes` 里下标更大的那一颗）。
    shadow_ulid: String,
    host_center: (f32, f32),
    painted_center: (f32, f32),
}

/// 从投影里取出靶子，并把**夹具前提**钉在这里：影子与靶子同 x/y，且影子排在后面。
fn target_in(view: &ViewState, edited: EntityId, shadow: EntityId) -> Target {
    let edited_text = edited.to_canonical_string();
    let shadow_text = shadow.to_canonical_string();
    let edited_index = (0..view.notes.len())
        .find(|&index| view.notes[index].clip_id == edited_text)
        .expect("夹具前提：投影里必须有正在编辑的剪辑的音符");
    let shadow_index = (0..view.notes.len())
        .find(|&index| {
            index != edited_index
                && view.notes[index].clip_id == shadow_text
                && view.notes[index].x == view.notes[edited_index].x
                && view.notes[index].width == view.notes[edited_index].width
                && view.notes[index].y == view.notes[edited_index].y
        })
        .expect("夹具前提：影子剪辑必须有一颗音符与靶子**逐像素重合**");
    assert!(
        shadow_index > edited_index,
        "夹具前提：影子音符必须**排在后面**（= 旧命中测试的'最后绘制'会选它），\
         实际 edited={edited_index} shadow={shadow_index}"
    );
    Target {
        edited_index,
        edited_ulid: view.notes[edited_index].id.clone(),
        shadow_ulid: view.notes[shadow_index].id.clone(),
        host_center: note_center_in_host_view(view, edited_index, 0.0),
        painted_center: note_center_in_painted_view(view, edited_index, 0.0),
    }
}

/// 判据 ①（纯投影层，⛔ 不建窗口）：不分作用域解析到**没画出来**的那颗，分作用域解析到正在编辑的那颗。
#[test]
fn the_unscoped_hit_test_returns_the_shadowing_note_while_the_scoped_one_returns_the_editing_note()
{
    let project = shadowed_project();
    let edited = edited_of(&yeban_model::samples::filled_project());
    let view = ViewState::from_project(&project).expect("投影");
    let shadow = fixture_id(SHADOW_CLIP);
    assert_eq!(
        view.notes.len(),
        edited.shapes.len() * 2,
        "夹具前提：两批音符都在投影里（剪辑池全量口径）"
    );
    let target = target_in(&view, edited.clip_id, shadow);
    let scope = Some(edited.clip_id.to_canonical_string());

    // ---- ①-a 缺陷本体：不分作用域 ⇒ 择优挑中**看不见**的影子 ----
    let unscoped = view.hit_test_visible(
        0.0,
        FLAT_VIEWPORT_WIDTH,
        target.host_center.0,
        target.host_center.1,
        NOTE_HEIGHT,
    );
    let unscoped_note = unscoped.expect("夹具前提：这一点确实压在某颗音符上");
    assert_eq!(
        view.notes[unscoped_note].clip_id,
        shadow.to_canonical_string(),
        "修复前的口径：重叠时取'下标最大'的那颗 ⇒ 正是**没有画出来**的影子音符"
    );

    // ---- ①-b 修法：分作用域 ⇒ 落在正在编辑的那颗 ----
    let scoped = view.hit_test_visible_scoped(
        0.0,
        FLAT_VIEWPORT_WIDTH,
        target.host_center.0,
        target.host_center.1,
        NOTE_HEIGHT,
        scope.as_deref(),
    );
    report_line(&format!(
        "[d-ui-13/①] 同一个宿主点 ({},{})：不分作用域 → `{}`（剪辑 {}），作用域内 → `{}`（剪辑 {}）",
        target.host_center.0,
        target.host_center.1,
        view.notes[unscoped_note].id,
        &view.notes[unscoped_note].clip_id[..8],
        scoped.map_or("None".to_owned(), |index| view.notes[index].id.clone()),
        scope.as_deref().unwrap_or("-")
    ));
    assert_eq!(
        scoped,
        Some(target.edited_index),
        "作用域内的命中必须解析到**正在编辑的那颗**（下标 {}",
        target.edited_index
    );
    assert_eq!(
        view.notes[scoped.expect("作用域内必须有命中")].id,
        target.edited_ulid,
        "命中的身份必须是靶子"
    );

    // ---- ①-c 包装不变：`None` 作用域与旧函数逐项相同 ----
    for index in 0..view.notes.len() {
        let (hx, hy) = note_center_in_host_view(&view, index, 0.0);
        assert_eq!(
            view.hit_test_visible(0.0, FLAT_VIEWPORT_WIDTH, hx, hy, NOTE_HEIGHT),
            view.hit_test_visible_scoped(0.0, FLAT_VIEWPORT_WIDTH, hx, hy, NOTE_HEIGHT, None),
            "`clip_scope = None` 必须与旧 `hit_test_visible` **逐项相同**（⛔ 不改无作用域语义）"
        );
    }
}

/// 判据 ⑤（**控制臂**）：没有选中任何剪辑 ⇒ 作用域 `None` ⇒ 命中的仍是**画在最上层**的影子。
///
/// 它把 ②③④ 的红定位到"作用域"这一条：同一个注入点、同一份装配，唯一区别是
/// `selected-clip-ulid` 为空。若这条也点不到任何东西，说明坏的是坐标 / 夹具而不是作用域。
#[test]
fn an_unselected_roll_still_targets_the_topmost_drawn_note() {
    let (window, port, mut plane) = assemble();
    let edited = edited_of(&yeban_model::samples::filled_project());
    let project = port.project();
    let view = ViewState::from_project(&project).expect("投影");
    let target = target_in(&view, edited.clip_id, fixture_id(SHADOW_CLIP));
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        "",
        "夹具前提：起点没有选中任何剪辑（作用域 = 全片段池）"
    );
    assert_eq!(window.get_selected_note_count(), 0, "起点不该已有选区");

    click_in_roll_at(&mut plane, 10, target.painted_center);

    let count = window.get_selected_note_count();
    let selected = selected_ids(&window);
    report_line(&format!(
        "[d-ui-13/⑤] 无作用域点击影子覆盖点 ⇒ count={count} selected={selected:?}（期望影子 `{}`）",
        target.shadow_ulid
    ));
    assert_eq!(
        count, 1,
        "无作用域时这一点压在最上层那颗音符上 ⇒ 必须有选区（否则是坐标 / 夹具坏了）"
    );
    assert!(
        selected.iter().any(|id| id == &target.shadow_ulid),
        "无作用域 ⇒ '最后绘制'那颗（影子 `{}`）才是屏幕上的靶子，实际 {selected:?}",
        target.shadow_ulid
    );
    assert!(
        !selected.iter().any(|id| id == &target.edited_ulid),
        "被遮住的那颗**画在下面**，无作用域时不该被选中"
    );
}

/// 判据 ②：选中该剪辑后，用默认选择工具点影子覆盖下的音符 ⇒ 选区落在这颗**看得见**的音符上。
///
/// **修复前必红**：`install_roll_click` 事后过滤 ⇒ 命中变 `None` ⇒ `selected-note-count` 停在 0
/// （实测活体症状：橡皮擦点 6 个剪辑的音符中心，5 个"点不点得中 = ❌ 无"）。
#[test]
fn a_click_on_a_shadowed_note_in_the_selected_clip_selects_it() {
    let (window, port, mut plane) = assemble();
    let edited = edited_of(&yeban_model::samples::filled_project());
    let project = port.project();
    let view = ViewState::from_project(&project).expect("投影");
    let target = target_in(&view, edited.clip_id, fixture_id(SHADOW_CLIP));
    assert_eq!(
        window.get_active_tool(),
        i32::from(Tool::Select.digit()),
        "夹具前提：默认工具就是选择箭头"
    );

    // 选中正在编辑的剪辑 ⇒ 卷帘作用域 = 它（`wire_clip_scope` 是 `build_live_ui` 装的同一条回调）。
    window.invoke_clip_select(edited.placement_id.to_canonical_string().as_str().into());
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        edited.placement_id.to_canonical_string().as_str(),
        "夹具前提：作用域已落到正在编辑的剪辑"
    );
    assert_eq!(window.get_selected_note_count(), 0, "起点不该已有选区");

    click_in_roll_at(&mut plane, 10, target.painted_center);

    let count = window.get_selected_note_count();
    let selected = selected_ids(&window);
    report_line(&format!(
        "[d-ui-13/②] 作用域={}，点靶子 `{}` 的绘制帧中心 ({},{}) ⇒ count={count} selected={selected:?}",
        &edited.clip_id.to_canonical_string()[..8],
        target.edited_ulid,
        target.painted_center.0,
        target.painted_center.1
    ));
    assert_eq!(
        count, 1,
        "选择工具点中**画出来的**那颗必须选中 1 个（事后过滤的旧实现这里恒为 0）"
    );
    assert!(
        selected.iter().any(|id| id == &target.edited_ulid),
        "选中的必须是正在编辑剪辑的那颗 `{}`，实际 {selected:?}",
        target.edited_ulid
    );
}

/// 判据 ③：同一个点换橡皮擦 ⇒ 权威工程里**只**删正在编辑剪辑的那颗，影子一颗不少。
#[test]
fn the_eraser_click_deletes_the_shadowed_note_of_the_selected_clip_only() {
    let (window, port, mut plane) = assemble();
    let edited = edited_of(&yeban_model::samples::filled_project());
    let project = port.project();
    let view = ViewState::from_project(&project).expect("投影");
    let target = target_in(&view, edited.clip_id, fixture_id(SHADOW_CLIP));
    let shadow = fixture_id(SHADOW_CLIP);
    let before_edited = clip_note_ids(&project, edited.clip_id);
    let before_shadow = clip_note_ids(&project, shadow);

    window.set_active_tool(i32::from(Tool::Eraser.digit()));
    assert_eq!(
        window.get_active_tool(),
        i32::from(Tool::Eraser.digit()),
        "夹具前提：橡皮擦工具已生效"
    );
    window.invoke_clip_select(edited.placement_id.to_canonical_string().as_str().into());
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        edited.placement_id.to_canonical_string().as_str(),
        "夹具前提：作用域已落到正在编辑的剪辑"
    );

    click_in_roll_at(&mut plane, 10, target.painted_center);

    let after = port.project();
    let after_edited = clip_note_ids(&after, edited.clip_id);
    let after_shadow = clip_note_ids(&after, shadow);
    report_line(&format!(
        "[d-ui-13/③] 橡皮擦点靶子 `{}` ⇒ 正在编辑的剪辑 {} 颗 → {} 颗，影子剪辑 {} 颗 → {} 颗",
        target.edited_ulid,
        before_edited.len(),
        after_edited.len(),
        before_shadow.len(),
        after_shadow.len()
    ));
    assert_eq!(
        after_edited.len(),
        before_edited.len() - 1,
        "正在编辑的剪辑必须**恰好少一颗**（旧实现：命中为 `None` ⇒ 一颗不少）"
    );
    assert!(
        !after_edited.iter().any(|id| id == &target.edited_ulid),
        "删掉的必须正是点中的那颗 `{}`",
        target.edited_ulid
    );
    assert_eq!(
        after_shadow, before_shadow,
        "影子剪辑一位都不许动（它的音符没画出来，⛔ 不是屏幕上的靶子）"
    );
    assert_eq!(
        window.get_selected_note_count(),
        0,
        "被删的身份已不存在 ⇒ 选区必须清空（与键盘 `Delete` 同一取向）"
    );
}

/// 判据 ④：拖拽臂（`begin_note_drag` 的第二次消费点）抓的是**看得见**的那颗。
///
/// **修复前必红**：`begin_note_drag` 连事后过滤都没有 ⇒ 抓的是影子那颗 ⇒ 靶子一位没动、
/// 影子被移动（用户视角：拖 A 的结果是听不见的 B 变了，而画面上 A 纹丝不动）。
#[test]
fn a_note_drag_grabs_the_visible_note_not_the_shadowing_one() {
    let (window, port, mut plane) = assemble();
    let edited = edited_of(&yeban_model::samples::filled_project());
    let project = port.project();
    let view = ViewState::from_project(&project).expect("投影");
    let target = target_in(&view, edited.clip_id, fixture_id(SHADOW_CLIP));
    assert_eq!(
        window.get_active_tool(),
        i32::from(Tool::Select.digit()),
        "夹具前提：拖拽臂只接选择工具"
    );
    window.invoke_clip_select(edited.placement_id.to_canonical_string().as_str().into());

    // ---- 独立期望值（⛔ 不读被测的拖拽代码）----
    let origin_tick = view.snapped_tick_at(0.0, target.host_center.0, ROLL_SNAP_GRID_TICKS);
    let current_tick = view.snapped_tick_at(
        0.0,
        target.host_center.0 + DRAG_HOST_DX,
        ROLL_SNAP_GRID_TICKS,
    );
    let expected_delta_tick = i64::try_from(current_tick).expect("tick 适配 i64")
        - i64::try_from(origin_tick).expect("tick 适配 i64");
    #[allow(clippy::cast_possible_truncation)]
    let expected_delta_pitch = (-DRAG_HOST_DY / 14.0) as i32;
    assert_ne!(expected_delta_tick, 0, "夹具前提：这一拖必须真的跨过网格");

    let before = note_by_ulid(&port.project(), &target.edited_ulid);
    let before_shadow = note_by_ulid(&port.project(), &target.shadow_ulid);
    drag_in_roll_at(&mut plane, 10, target.painted_center);

    let after_project = port.project();
    let after = note_by_ulid(&after_project, &target.edited_ulid);
    let after_shadow = note_by_ulid(&after_project, &target.shadow_ulid);
    report_line(&format!(
        "[d-ui-13/④] 拖靶子 `{}` ⇒ start_tick {} → {}（期望 {expected_delta_tick:+}）\
         pitch {} → {}（期望 {expected_delta_pitch:+}）；影子 `{}` {} → {}",
        target.edited_ulid,
        before.0,
        after.0,
        before.1,
        after.1,
        target.shadow_ulid,
        before_shadow.0,
        after_shadow.0
    ));
    assert_eq!(
        i64::try_from(after.0).expect("tick") - i64::try_from(before.0).expect("tick"),
        expected_delta_tick,
        "被移动的必须是**画出来的**那颗（旧实现：抓的是影子，这颗一位没动）"
    );
    assert_eq!(
        i32::from(after.1) - i32::from(before.1),
        expected_delta_pitch,
        "`pitch` 必须按 14px 泳道几何改变（-28px ⇒ +2 半音）"
    );
    assert_eq!(
        after_shadow, before_shadow,
        "影子剪辑的音符不许被拖走（它不在屏幕上，⛔ 不是这一按的靶子）"
    );
}
