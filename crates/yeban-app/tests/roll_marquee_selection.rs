//! **卷帘多音符选区入口**回归判据（T-FIX-204 / ISS-MWTFLI5C-10E44F）。
//!
//! 三条入口：① 空白拖拽框选（选择工具）；② Shift/Ctrl/Cmd 单击 toggle；③ Cmd/Ctrl+A 全选。
//! 判据只看可观测读数：`selected-note-count` / `selected-ulids`，再用 Delete 键验证
//! “框选 3 颗 ⇒ 一次删 3 颗 ⇒ 一次撤销全部回来”。
//!
//! ⚠ 单 `#[test]`：Slint 平台每线程只能装一次。

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::Model;
use yeban_app::bridge::{ViewState, host_to_painted};
use yeban_app::host;
use yeban_app::input::InputContext;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::project::ClipContent;
use yeban_model::{EntityId, MidiNote, YebanProjectV1};
use yeban_ui_test_port::Rect;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::{KeyCode, Permission, PointerButton, UiTestPort as _};
use yeban_ui_test_port::render::LivePort;

const WINDOW: Size = Size::new(1440, 900);
const NOW_MS: u64 = 1_760_000_000_000;

/// 夹具：`filled_project` 的 Lead 片段里放 3 颗音符（60/62/64，tick 0/960/1920）。
fn three_note_project() -> (YebanProjectV1, EntityId, EntityId) {
    let mut project = yeban_model::samples::filled_project();
    let mut lead_id = None;
    let mut clip_id = None;
    for track in project.tracks.values() {
        for placement in track.clips.values() {
            if matches!(
                &project.clip_pool[&placement.clip_id].content,
                ClipContent::Midi { .. }
            ) {
                lead_id = Some(track.id);
                clip_id = Some(placement.clip_id);
            }
        }
    }
    let lead_id = lead_id.expect("夹具必须有 MIDI 轨");
    let clip_id = clip_id.expect("夹具必须有 MIDI 片段");
    let ClipContent::Midi { notes } = &mut project
        .clip_pool
        .get_mut(&clip_id)
        .expect("片段池条目")
        .content
    else {
        unreachable!();
    };
    notes.clear();
    for (start, pitch) in [(0_u64, 60_u8), (960, 62), (1920, 64)] {
        let id = EntityId::new();
        notes.insert(id, MidiNote::new(id, start, pitch, 480));
    }
    (project, lead_id, clip_id)
}

fn midi_note_count(project: &YebanProjectV1) -> usize {
    project
        .clip_pool
        .values()
        .map(|entry| match &entry.content {
            ClipContent::Midi { notes } => notes.len(),
            ClipContent::Audio { .. } => 0,
        })
        .sum()
}

#[test]
fn marquee_selects_three_notes_and_delete_removes_them_in_one_undo_step() {
    let (project, _lead_id, _clip_id) = three_note_project();
    let view = ViewState::from_project(&project).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<roll-marquee>", "yeban-app", project.clone(), NOW_MS)
            .expect("打开撤销会话"),
    ));
    let scene = DemoScene::from_view(&view);
    let mut live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台");
    let ui = live.ui();
    // 生产接线：点击/marquee 带端口重装一次；键盘（Delete / Cmd+Z）接同一端口。
    host::wire_roll_edit(ui, &port);
    host::wire_keys(
        ui,
        Rc::new(RefCell::new(InputContext::new())),
        Some(Rc::clone(&port)),
        None,
        Some(Rc::new(project.clone())),
    );

    // ── ① 框选：把三颗音符全包进矩形（宿主坐标 → 绘制帧）。
    let ui_base = ui.get_roll_base_pitch().clamp(0, 112) as u8;
    let scroll = ui.get_roll_scroll_x();
    let width = slint::ComponentHandle::window(ui).size().width as f32;
    let mut visible = view.visible_notes_scoped(scroll, width, None);
    view.reproject_note_ys_with_base(&visible.ulids, &mut visible.ys, ui_base);
    assert_eq!(visible.ulids.len(), 3, "夹具必须 3 颗可见音符");
    let min_x = visible.positions.iter().copied().fold(f32::MAX, f32::min) - 8.0;
    let max_x = visible
        .positions
        .iter()
        .zip(&visible.widths)
        .map(|(x, w)| x + w)
        .fold(f32::MIN, f32::max)
        + 8.0;
    let min_y = visible.ys.iter().copied().fold(f32::MAX, f32::min) - 8.0;
    let max_y = visible.ys.iter().copied().fold(f32::MIN, f32::max) + 18.0;
    let (x1, y1) = host_to_painted(min_x, min_y);
    let (x2, y2) = host_to_painted(max_x, max_y);
    ui.invoke_roll_marquee(x1, y1, x2, y2, false);
    assert_eq!(
        ui.get_selected_note_count(),
        3,
        "框选必须选中 3 颗（矩形 {x1},{y1} → {x2},{y2}）"
    );
    assert_eq!(ui.get_selected_ulids().iter().count(), 3);

    // ── ② Shift 单击第 4 次? 没有第 4 颗：用 Shift 单击**已选**的一颗 ⇒ 取消它（2 颗）。
    let (cx, cy) = host_to_painted(
        visible.positions[0] + visible.widths[0] / 2.0,
        visible.ys[0] + 3.0,
    );
    ui.invoke_clicked(cx, cy, true);
    assert_eq!(
        ui.get_selected_note_count(),
        2,
        "Shift 单击已选音符必须取消它"
    );
    // 再 Shift 一次 ⇒ 加回来（3 颗）。
    ui.invoke_clicked(cx, cy, true);
    assert_eq!(
        ui.get_selected_note_count(),
        3,
        "Shift 单击未选音符必须加选"
    );

    // ── ③ 先裸单击一颗（镜像 = 1，与界面 1 对齐），再用 Cmd/Ctrl+A 全选。
    //    这样界面属性 = 3 而派发器镜像 = 1 —— 正是 204 复验否决 V1 的现场。
    ui.invoke_clicked(cx, cy, false);
    assert_eq!(ui.get_selected_note_count(), 1, "裸单击必须先塌到 1 颗");
    ui.invoke_key_action("a".into(), false, true, false, false);
    assert_eq!(
        ui.get_selected_note_count(),
        3,
        "Cmd/Ctrl+A 必须全选作用域内音符"
    );

    // ── ③b T-FIX-213 V1：Cmd+A 之后**加选框选**仍必须是并集（镜像先从界面同步）。
    //    只框住中间那一颗 ⇒ 并集仍是 3 颗；旧实现（镜像为空）会把它算成 1 颗。
    let (mx1, my1) = host_to_painted(visible.positions[1] - 4.0, visible.ys[1] - 4.0);
    let (mx2, my2) = host_to_painted(
        visible.positions[1] + visible.widths[1] + 4.0,
        visible.ys[1] + 14.0,
    );
    ui.invoke_roll_marquee(mx1, my1, mx2, my2, true);
    assert_eq!(
        ui.get_selected_note_count(),
        3,
        "Cmd+A 后加选框选必须保持并集（3 颗），⛔ 不能丢掉全选"
    );

    // ── ③c T-FIX-213 F3：Shift/Cmd + 空白 = **保留选区**（管理侧裁决）。
    ui.invoke_clicked(1400.0, 800.0, true);
    assert_eq!(
        ui.get_selected_note_count(),
        3,
        "Shift 单击空白必须保留选区，不清空"
    );

    // ── ④ Delete：一次删 3 颗；一次撤销全部回来。
    assert_eq!(
        midi_note_count(port.try_project().as_ref().expect("工程")),
        3
    );
    ui.invoke_key_action("\u{7f}".into(), false, false, false, false);
    assert_eq!(
        midi_note_count(port.try_project().as_ref().expect("工程")),
        0,
        "Delete 必须一次删掉 3 颗选中音符"
    );
    ui.invoke_key_action("z".into(), false, true, false, false);
    assert_eq!(
        midi_note_count(port.try_project().as_ref().expect("工程")),
        3,
        "一次撤销必须把 3 颗全部恢复（原子 Batch）"
    );
    // ── ⑤ T-FIX-220：真实 Ctrl/Meta 指针事件 + 真实 Ctrl 框选。
    //    这里不能再用 `invoke_clicked(..., true)` 绕过 Slint 的修饰位解析；
    //    `KeyCode::Control` / `KeyCode::Meta` 按下后保持修饰态，指针事件从
    //    `piano_roll.slint` 的 `event.modifiers` 读到它，和用户真按 Ctrl/Cmd 同链。
    //    先裸点空白把选区清空。
    live.ui().invoke_clicked(1400.0, 800.0, false);
    let p0 = host_to_painted(
        visible.positions[0] + visible.widths[0] / 2.0,
        visible.ys[0] + 3.0,
    );
    let p1 = host_to_painted(
        visible.positions[1] + visible.widths[1] / 2.0,
        visible.ys[1] + 3.0,
    );
    pointer_click(&mut live, Some(KeyCode::Control), p0.0, p0.1);
    assert_eq!(
        live.ui().get_selected_note_count(),
        1,
        "Ctrl 单击第一颗必须选中 1 颗"
    );
    pointer_click(&mut live, Some(KeyCode::Control), p1.0, p1.1);
    assert_eq!(
        live.ui().get_selected_note_count(),
        2,
        "Ctrl 单击第二颗必须加选到 2 颗"
    );
    pointer_click(&mut live, Some(KeyCode::Control), p0.0, p0.1);
    assert_eq!(
        live.ui().get_selected_note_count(),
        1,
        "Ctrl 再点第一颗必须 toggle 掉它，只留第二颗"
    );
    pointer_click(&mut live, Some(KeyCode::Meta), p0.0, p0.1);
    assert_eq!(
        live.ui().get_selected_note_count(),
        2,
        "Meta/Cmd 单击未选中的第一颗必须加选到 2 颗"
    );
    pointer_click(&mut live, Some(KeyCode::Meta), p0.0, p0.1);
    assert_eq!(
        live.ui().get_selected_note_count(),
        1,
        "Meta/Cmd 再点第一颗必须 toggle 掉它，只留第二颗"
    );

    // Ctrl 框选并集：先 Ctrl 点第一颗 ⇒ 1 颗；再用 Ctrl 框住第二颗 ⇒ 必须保住第一颗、加进第二颗。
    live.ui().invoke_clicked(1400.0, 800.0, false);
    pointer_click(&mut live, Some(KeyCode::Control), p0.0, p0.1);
    assert_eq!(
        live.ui().get_selected_note_count(),
        1,
        "Ctrl 框选前先选中第一颗"
    );
    let note_left = visible.positions[1];
    let note_right = note_left + visible.widths[1];
    // 起点固定在琴键列右侧的空白竖带（x=64，网格内、不是琴键列），纵坐标在所有音符上方；
    // 终点必须跨过第二颗音符：音符在起点右侧时向右框，贴着左边界（left < 64）时向左框。
    let start_x = 64.0;
    let end_x = if note_left >= start_x {
        note_right + 20.0
    } else {
        (note_left - 20.0).max(0.0)
    };
    let (mx1, my1) = host_to_painted(start_x, min_y);
    let (mx2, my2) = host_to_painted(end_x, visible.ys[1] + 20.0);
    let before_ids: Vec<String> = live
        .ui()
        .get_selected_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    assert_eq!(before_ids.len(), 1, "Ctrl 框选前先选中第一颗");
    pointer_marquee(&mut live, Some(KeyCode::Control), mx1, my1, mx2, my2);
    let after_ids: Vec<String> = live
        .ui()
        .get_selected_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    assert!(
        after_ids.len() > 1,
        "Ctrl 框选必须至少加进一颗新音符（并集，而不是塌成单颗）：{after_ids:?}"
    );
    for id in &before_ids {
        assert!(
            after_ids.contains(id),
            "Ctrl 框选必须保留框选前已选中的 {id}（T-FIX-220）：{after_ids:?}"
        );
    }
}

/// 真实指针手势用的 `piano-roll` 根元素包围盒；`dispatch_pointer_down` 的偏移是元素局部，
/// `dispatch_pointer_move` 的坐标是窗口绝对（见 `LivePort::dispatch_pointer_move_impl`）。
fn piano_roll_rect<T: slint::ComponentHandle>(live: &LivePort<T>) -> Rect {
    live.tree()
        .find_by_id("piano-roll")
        .expect("piano-roll 必须在控件树里")
        .bounds
        .expect("piano-roll 必须有几何")
}

/// 真实左键单击（可选按住一个加选修饰键）。按下时修饰键已经在
/// `WindowEvent::KeyPressed` 里置位，抬起后才释放 —— 与用户按住 Ctrl/Meta 点一下同一条事件链。
fn pointer_click<T: slint::ComponentHandle>(
    live: &mut LivePort<T>,
    modifier: Option<KeyCode>,
    x: f32,
    y: f32,
) {
    if let Some(key) = modifier {
        live.dispatch_key_press(key).expect("按下修饰键");
    }
    live.dispatch_pointer_down(
        "piano-roll",
        f64::from(x),
        f64::from(y),
        PointerButton::Left,
    )
    .expect("按下左键");
    live.dispatch_pointer_up(PointerButton::Left)
        .expect("抬起左键");
    if let Some(key) = modifier {
        live.dispatch_key_release(key).expect("释放修饰键");
    }
    // 同一位置连续点击会被 Slint 的 `ClickState` 判成双击（`click_interval` 内、10px 内），
    // 从而走 `on_double_clicked` 而不是 `on_clicked`；本判据要单击语义，隔开一个窗口。
    std::thread::sleep(Duration::from_millis(600));
}

fn pointer_marquee<T: slint::ComponentHandle>(
    live: &mut LivePort<T>,
    modifier: Option<KeyCode>,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
) {
    let rect = piano_roll_rect(live);
    if let Some(key) = modifier {
        live.dispatch_key_press(key).expect("按下修饰键");
    }
    live.dispatch_pointer_down(
        "piano-roll",
        f64::from(x1),
        f64::from(y1),
        PointerButton::Left,
    )
    .expect("按下左键");
    live.dispatch_pointer_move(
        f64::from(rect.x) + f64::from(x2),
        f64::from(rect.y) + f64::from(y2),
    )
    .expect("拖到矩形终点");
    live.dispatch_pointer_up(PointerButton::Left)
        .expect("抬起左键");
    if let Some(key) = modifier {
        live.dispatch_key_release(key).expect("释放修饰键");
    }
}

/// 界面侧的接线是“空白拖拽 = 框选”的**唯一**手势来源：这条判据钉住三件事——
/// ① 选择工具（active-tool == 1）且宿主没接管音符拖动时才进入 marquee；
/// ② 松手拖过阈值上抛 `roll-marquee`，没拖过阈值仍是一次空白单击；
/// ③ cancel 会丢弃框选草稿（不改选区）。
#[test]
fn slint_wires_blank_drag_to_marquee_only_for_the_select_tool() {
    let source = include_str!("../ui/console/piano_roll.slint");
    assert!(
        source.contains("if !root.note-dragging && root.active-tool == 1 {"),
        "空白拖拽必须限定在选择工具且宿主未接管音符拖动"
    );
    assert!(
        source.contains("root.roll-marquee(x1, y1, self.mouse-x, self.mouse-y, additive);"),
        "拖过阈值必须上抛 roll-marquee"
    );
    assert!(
        source.contains("root.pointer-additive = event.modifiers.shift")
            && source.contains("|| event.modifiers.control")
            && source.contains("|| event.modifiers.meta"),
        "按下时必须同时识别 Shift / Ctrl / Meta 三个加选修饰键"
    );
    assert!(
        source.contains("root.clicked(self.mouse-x, self.mouse-y, root.pointer-additive);"),
        "单击必须上报按下时算好的加选状态（toggle 语义）"
    );
    assert!(
        source.contains("if root.marquee-active {"),
        "cancel 分支必须丢弃框选草稿"
    );
}
