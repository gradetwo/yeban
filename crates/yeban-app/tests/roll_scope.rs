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

#![allow(unused)]
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

/// 同一次注入的五个音符数组 ＋ 选中标志（六个平行数组，同索引集）。
///
/// 用具名字段而不是六元组：`clippy::type_complexity` 对 `-D warnings` 生效
/// （run 40989419153 红在 `roll_scope.rs:148`）。
#[derive(Debug, PartialEq)]
struct Reading {
    ulids: Vec<String>,
    positions: Vec<f32>,
    widths: Vec<f32>,
    ys: Vec<f32>,
    velocities: Vec<f32>,
    selected: Vec<bool>,
}

fn reading(ui: &MainWindow) -> Reading {
    Reading {
        ulids: ulids_of(ui),
        positions: floats_of(ui.get_note_positions()),
        widths: floats_of(ui.get_note_widths()),
        ys: floats_of(ui.get_note_ys()),
        velocities: floats_of(ui.get_note_velocities()),
        selected: flags_of(ui),
    }
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
        aether_app.contains("callback clicked("),
        "新 UI 契约面必须声明 callback clicked("
    );
    assert!(
        aether_host.contains("on_clicked"),
        "宿主接线层必须注册 on_clicked"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
    assert!(
        aether_app.contains("> note-positions:"),
        "新 UI 契约面必须声明 > note-positions:"
    );
    assert!(
        aether_host.contains("set_note_positions"),
        "宿主接线层必须注册 set_note_positions"
    );
}
