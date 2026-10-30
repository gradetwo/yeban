//! **卷帘作用域**（选中的剪辑 ⇒ 只注入该片段池条目的音符）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 过滤本身在投影层有单元判据（`bridge.rs` 判据 14/15）；本文件证**宿主接线**真的把
//! "选中的剪辑摆放"带到了三个消费者，而且可以**原样退回**：
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（MIDI 剪辑 = 恒等） | 选中演示工程的 MIDI 剪辑 ⇒ 注入的五个音符数组与基线**逐项相同**；像素变化只在它的包头内 |
//! | ②（作用域下可点） | 在卷帘上点一个音符 ⇒ 它进 `selected-ulids`（命中测试在作用域内仍工作） |
//! | ③（音频剪辑 = 清空） | 换选音频剪辑 ⇒ `note-ulids` / `note-velocities` 清空、控件树里音符 / 力度柱一个不剩 |
//! | ④（看不见的不可点） | 音频剪辑选中时点同一处 ⇒ 选区**被清空**（`install_roll_click` 与注入同一作用域，⛔ 不选中看不见的音符） |
//! | ⑤（换回即还原） | 再选 MIDI 剪辑 ⇒ 注入数组逐项等于基线，且**帧与①的帧逐字节相同** |
//!
//! 期望值全部从**投影与基线帧**读（⛔ 不从被测注入面读回来当期望）。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/clip_selection.rs` 同款）。

use slint::Model as _;
use std::cell::RefCell;
use std::rc::Rc;
use yeban_app::bridge::{ViewState, host_to_painted};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::{Permission, Rect, Rgb8Image, Size};

/// 与其余 Tier-1 判据同款。
const WINDOW: Size = Size::new(1440, 900);

/// 音符矩形高（与 `host::NOTE_HEIGHT_PX` / `piano_roll.slint` 的 `12px` 同一读数）。
const NOTE_HEIGHT: f32 = 12.0;

fn live_window() -> LivePort<MainWindow> {
    let view =
        ViewState::from_project(&yeban_model::samples::demo_project()).expect("演示工程必须能投影");
    let scene = DemoScene::from_view(&view);
    // 剪辑选中回调的作用域回落视图（没有撤销端口时用它重注入，与产品进程同一条 `apply_view`）。
    let scope_view = Rc::new(RefCell::new(view.clone()));
    let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    host::wire_view_callbacks(port.ui());
    // 与产品进程 / 无头装配调的是**同一个** `host::wire_clip_scope`。
    host::wire_clip_scope(port.ui(), scope_view, None);
    port
}

fn bounds_of(port: &mut LivePort<MainWindow>, id: &str) -> Rect {
    port.refresh_tree(None)
        .expect("必须能重抓控件树")
        .find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里找不到 `{id}`"))
        .bounds
        .unwrap_or_else(|| panic!("`{id}` 必须有几何包围盒"))
}

/// 两帧的差异像素数，按"是否落在 `rect` 内"分成 `(矩形内, 矩形外)`。
fn diff_split(before: &Rgb8Image, after: &Rgb8Image, rect: Rect) -> (u64, u64) {
    assert_eq!(
        (before.width(), before.height()),
        (after.width(), after.height()),
        "两帧必须同尺寸"
    );
    let (x0, y0) = (i64::from(rect.x), i64::from(rect.y));
    let (x1, y1) = (x0 + i64::from(rect.width), y0 + i64::from(rect.height));
    let mut inside = 0_u64;
    let mut outside = 0_u64;
    for y in 0..before.height() {
        for x in 0..before.width() {
            if before.pixel(x, y) != after.pixel(x, y) {
                let in_rect = i64::from(x) >= x0
                    && i64::from(x) < x1
                    && i64::from(y) >= y0
                    && i64::from(y) < y1;
                if in_rect {
                    inside += 1;
                } else {
                    outside += 1;
                }
            }
        }
    }
    (inside, outside)
}

/// 运行时树里属于**卷帘音符面**的音符身份（`note-{ulid}-rect` 的 `{ulid}`，升序）。
///
/// 静态叠加层 `note-suggestion-overlay-rect` **排除**（它不在 `note-ulids` 的循环里）。
fn note_leaf_ids(port: &mut LivePort<MainWindow>) -> Vec<String> {
    let tree = port.refresh_tree(None).expect("必须能重抓控件树");
    let mut ids: Vec<String> = tree
        .iter()
        .filter_map(|node| {
            let id = node.id.as_str();
            if id == "note-suggestion-overlay-rect" {
                return None;
            }
            Some(id.strip_prefix("note-")?.strip_suffix("-rect")?.to_string())
        })
        .collect();
    ids.sort();
    ids
}

/// 运行时树里力度柱的条数（`velocity-{i}-bar`）。
fn velocity_bar_count(port: &mut LivePort<MainWindow>) -> usize {
    let tree = port.refresh_tree(None).expect("必须能重抓控件树");
    tree.iter()
        .filter(|node| {
            let id = node.id.as_str();
            id.strip_prefix("velocity-")
                .and_then(|rest| rest.strip_suffix("-bar"))
                .is_some_and(|index| index.chars().all(|c| c.is_ascii_digit()))
        })
        .count()
}

fn ulids_of(ui: &MainWindow) -> Vec<String> {
    ui.get_note_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect()
}

fn floats_of(model: slint::ModelRc<f32>) -> Vec<f32> {
    model.iter().collect()
}

fn bools_of(model: slint::ModelRc<bool>) -> Vec<bool> {
    model.iter().collect()
}

/// 选中标志数组（第六个平行数组）。
fn flags_of(ui: &MainWindow) -> Vec<bool> {
    bools_of(ui.get_note_selected())
}

/// 同一次注入的五个音符数组 ＋ 选中标志。
fn reading(
    ui: &MainWindow,
) -> (
    Vec<String>,
    Vec<f32>,
    Vec<f32>,
    Vec<f32>,
    Vec<f32>,
    Vec<bool>,
) {
    (
        ulids_of(ui),
        floats_of(ui.get_note_positions()),
        floats_of(ui.get_note_widths()),
        floats_of(ui.get_note_ys()),
        floats_of(ui.get_note_velocities()),
        flags_of(ui),
    )
}

/// `selected-ulids` 的当前读数。
fn selected_ids(ui: &MainWindow) -> Vec<String> {
    ui.get_selected_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect()
}

/// 找一个**人真的点得到**的音符（返回它在 `view.notes` 里的下标 ＋ 绘制帧坐标）。
///
/// 两个条件：宿主帧中心被 `hit_test_visible` 解析到它自己；绘制帧（`host_to_painted`）
/// 中心仍落在窗口内。`invoke_clicked` 送的是**绘制帧**（`install_roll_click` 唯一一次
/// `painted_to_host` 把它搬回宿主帧）。
fn first_clickable_note(view: &ViewState, scroll: f32) -> (usize, (f32, f32)) {
    let width = WINDOW.width as f32;
    (0..view.notes.len())
        .find_map(|index| {
            let note = &view.notes[index];
            let (hx, hy) = (
                note.x - scroll + note.width / 2.0,
                note.y + NOTE_HEIGHT / 2.0,
            );
            if view.hit_test_visible(scroll, width, hx, hy, NOTE_HEIGHT) != Some(index) {
                return None;
            }
            let (px, py) = host_to_painted(hx, hy);
            if (0.0..=width).contains(&px) && (0.0..=WINDOW.height as f32).contains(&py) {
                Some((index, (px, py)))
            } else {
                None
            }
        })
        .expect("夹具里必须有一个宿主命中的音符，其绘制帧中心也落在窗口内")
}

#[test]
fn roll_scope_follows_the_selected_clip_and_restores_the_baseline() {
    let mut port = live_window();
    let view = ViewState::from_project(&yeban_model::samples::demo_project()).expect("投影");
    let all_notes = view.note_ulids.clone();
    assert_eq!(all_notes.len(), 6, "演示夹具的 MIDI 片段有 6 个音符");

    let clip_ids: Vec<String> = port
        .ui()
        .get_clip_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    assert_eq!(clip_ids.len(), 3, "演示工程有三个剪辑摆放");
    // 摆放序 = 轨道序：第一个是 MIDI 片段，第二个是音频片段（同一片段池条目）。
    let midi = clip_ids[0].clone();
    let audio = clip_ids[1].clone();

    // ---- ① 基线（无选中）＋ 选中 MIDI 剪辑 = 恒等 ----
    assert_eq!(
        port.ui().get_selected_clip_ulid().as_str(),
        "",
        "默认不选中任何剪辑"
    );
    let base = reading(port.ui());
    assert_eq!(base.0, all_notes, "默认注入面 = 片段池全部音符");
    assert_eq!(base.5.len(), all_notes.len(), "选中标志与音符数组同索引集");
    assert!(base.5.iter().all(|flag| !*flag), "默认没有选中任何音符");
    let baseline_frame = port.window().capture().expect("Tier-1 截图");
    assert_eq!(note_leaf_ids(&mut port), all_notes, "基线控件树的音符面");
    assert_eq!(velocity_bar_count(&mut port), all_notes.len());

    let midi_header = bounds_of(&mut port, &format!("clip-{midi}-header"));
    port.ui().invoke_clip_select(midi.as_str().into());
    assert_eq!(port.ui().get_selected_clip_ulid().as_str(), midi.as_str());
    let midi_arrays = reading(port.ui());
    assert_eq!(midi_arrays, base, "选中 MIDI 剪辑 ⇒ 五个数组逐项等于基线");
    assert_eq!(note_leaf_ids(&mut port), all_notes);
    let midi_frame = port.window().capture().expect("Tier-1 截图");
    let (inside, outside) = diff_split(&baseline_frame, &midi_frame, midi_header);
    assert!(
        inside > 0,
        "选中态必须真的画出来（包头描边）: {midi_header:?}"
    );
    assert_eq!(
        outside, 0,
        "选中 MIDI 剪辑只许改变它自己的包头像素（卷帘一位不动）: {midi_header:?}"
    );

    // ---- ② 作用域下仍然可以选中音符 ----
    let scroll = port.ui().get_roll_scroll_x();
    let (index, (click_x, click_y)) = first_clickable_note(&view, scroll);
    port.ui().invoke_clicked(click_x, click_y);
    let clicked = view.notes[index].id.clone();
    assert_eq!(
        selected_ids(port.ui()),
        vec![clicked.clone()],
        "MIDI 剪辑选中时，点可见音符必须选中它（命中测试与注入同一作用域）"
    );
    assert_eq!(port.ui().get_selected_note_count(), 1);
    let flags = flags_of(port.ui());
    let selected_positions: Vec<usize> = flags
        .iter()
        .enumerate()
        .filter_map(|(slot, flag)| flag.then_some(slot))
        .collect();
    assert_eq!(
        selected_positions,
        vec![index],
        "选中标志必须与注入数组同索引集（第 {index} 位亮起）"
    );

    // ---- ③ 换选音频剪辑 ⇒ 卷帘作用域清空 ----
    port.ui().invoke_clip_select(audio.as_str().into());
    assert_eq!(port.ui().get_selected_clip_ulid().as_str(), audio.as_str());
    assert!(
        ulids_of(port.ui()).is_empty(),
        "音频片段池条目没有 MIDI 音符 ⇒ 注入面必须清空"
    );
    assert!(floats_of(port.ui().get_note_positions()).is_empty());
    assert!(floats_of(port.ui().get_note_velocities()).is_empty());
    assert!(
        flags_of(port.ui()).is_empty(),
        "作用域清空后选中标志数组也必须为空（同索引集）"
    );
    assert!(
        note_leaf_ids(&mut port).is_empty(),
        "控件树里不得残留音符矩形"
    );
    assert_eq!(velocity_bar_count(&mut port), 0, "控件树里不得残留力度柱");
    let audio_injected = ulids_of(port.ui()).len();
    let audio_tree = note_leaf_ids(&mut port).len();

    // ---- ④ 看不见的音符不可点：点同一处 ⇒ 选区被清空 ----
    port.ui().invoke_clicked(click_x, click_y);
    assert!(
        selected_ids(port.ui()).is_empty(),
        "作用域外的音符不得被选中（⛔ 不选中看不见的音符）"
    );
    assert_eq!(port.ui().get_selected_note_count(), 0);

    // ---- ⑤ 换回 MIDI 剪辑 ⇒ 原样还原 ----
    port.ui().invoke_clip_select(midi.as_str().into());
    let restored = reading(port.ui());
    assert_eq!(restored, base, "换回 MIDI 剪辑 ⇒ 五个数组逐项等于基线");
    assert_eq!(note_leaf_ids(&mut port), all_notes);
    assert_eq!(velocity_bar_count(&mut port), all_notes.len());
    let restored_frame = port.window().capture().expect("Tier-1 截图");
    assert_eq!(
        restored_frame.pixels(),
        midi_frame.pixels(),
        "换回 MIDI 剪辑的帧必须与①的帧逐字节相同（还原是完整的，不残留高亮）"
    );

    report_line(&format!(
        "[roll-scope] 剪辑 {} 个（MIDI `{midi}` / 音频 `{audio}`）；基线音符 {} 个；\
         选 MIDI ⇒ 数组恒等、包头内 {} 像素 / 区外 0；作用域下点中 `{clicked}`；\
         选音频 ⇒ 注入面 {} 个、控件树 {} 条；点同一处 ⇒ 选区清空；换回 ⇒ 数组恒等且帧逐字节相同",
        clip_ids.len(),
        all_notes.len(),
        inside,
        audio_injected,
        audio_tree,
    ));
}
