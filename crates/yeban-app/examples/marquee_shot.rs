//! 框选目视取帧（line/blank-drag-selection）：在活窗口里真实拖一次橡皮筋，
//! 把「拖拽中（矩形可见）/ 松手后（选中边框）」两帧连同 3x 裁切落盘 PNG，
//! 供 1:1 与 3x 目视验收。数字只作诊断，验收靠眼睛（见判据
//! `tests/clip_marquee_selection.rs` 的语义断言）。
//!
//! 用法：`cargo run -p yeban-app --example marquee_shot -- <输出目录>`
//! 产物：`marquee-before.png` / `marquee-drag.png` / `marquee-drag-3x.png` /
//! `marquee-after.png`，并在 stdout 打印拖拽的内容帧矩形与松手后的选区集。

use std::cell::RefCell;
use std::rc::Rc;

use slint::Model as _;
use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{
    Permission, PointerButton, Rgb8Image, Size, UiTestPort as _, encode_rgb8,
};

const NOW_MS: u64 = 1_700_000_000_000;
const WINDOW: Size = Size::new(1440, 900);
const FLICKABLE: &str = "arr-lanes-flickable";
// 车道区左缘在 Flickable 局部帧的偏移 = `headers-width` + 5（实窗里
// `headers-width` 绑根属性 `track-headers-width` = 240px，⛔ 不是组件
// 默认值 216px）⇒ 主流程里从窗口现读。

fn save(image: &Rgb8Image, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::write(path, encode_rgb8(image))?;
    println!("wrote {}", path.display());
    Ok(())
}

/// 3x 最近邻裁切：`(cx, cy)` 为中心、源图取 `w/3 × h/3` 一块放大三倍。
fn crop3x(image: &Rgb8Image, cx: u32, cy: u32, w: u32, h: u32) -> Rgb8Image {
    let sw = w / 3;
    let sh = h / 3;
    let x0 = cx
        .saturating_sub(sw / 2)
        .min(image.width().saturating_sub(sw));
    let y0 = cy
        .saturating_sub(sh / 2)
        .min(image.height().saturating_sub(sh));
    let mut out = Rgb8Image::new(Size::new(w, h));
    for y in 0..h {
        for x in 0..w {
            let sx = x0 + x / 3;
            let sy = y0 + y / 3;
            if let (Some(rgb), true) = (
                image.pixel(sx, sy),
                sx < image.width() && sy < image.height(),
            ) {
                out.set_pixel(x, y, rgb);
            }
        }
    }
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/marquee-shots".to_string());
    std::fs::create_dir_all(&out_dir)?;
    let out = std::path::Path::new(&out_dir);

    let project = demo_project();
    let view = ViewState::from_project(&project).expect("演示工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let mut port = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })?;
    let undo = Rc::new(UndoPort::new(UndoSession::open(
        "<clip-marquee-shot>",
        "yeban-app",
        project.clone(),
        NOW_MS,
    )?));
    let live_view = Rc::new(RefCell::new(view.clone()));
    host::wire_clip_scope(port.ui(), Rc::clone(&live_view), Some(Rc::clone(&undo)));
    port.refresh_tree(None)?;

    let bounds = port
        .tree()
        .find_by_id(FLICKABLE)
        .and_then(|h| h.bounds)
        .ok_or("编曲滚动面必须有几何")?;
    let origin_x = f64::from(port.ui().get_track_headers_width()) + 5.0;
    let visible_tx_max = bounds.width as f32 - origin_x as f32 - 24.0;

    // 与判据同款的几何复算：块 = (x.max(0), track_ys+4, w.max(12), max(52, h-8))。
    let ys = view.track_ys();
    let heights = view.track_heights();
    let block = |clip: &yeban_app::bridge::ClipView| {
        (
            clip.x.max(0.0),
            ys[clip.track_index] + 4.0,
            clip.width.max(12.0),
            (heights[clip.track_index] - 8.0).max(52.0),
        )
    };
    // 空白按下点：第一条轨道行内第一处 ≥40px 空档中点（连同所在轨号返回，
    // 目标必须取**异轨**片段 —— 同轨目标会让矩形高度塌成 0，只剩一条线，
    // 目视等于没画：第一版实帧就是这样被诊断出来的）。
    let (mut sx, mut sy, mut start_track) = (0.0_f32, 0.0_f32, usize::MAX);
    'scan: for track in 0..view.tracks.len() {
        let mut intervals: Vec<(f32, f32)> = view
            .clips
            .iter()
            .filter(|c| c.track_index == track)
            .map(|c| {
                let (x, _, w, _) = block(c);
                (x, x + w)
            })
            .collect();
        intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).expect("有限坐标"));
        let mut cursor = 8.0_f32;
        for (start, end) in intervals {
            if start - cursor >= 40.0 && cursor + 20.0 < visible_tx_max {
                sx = cursor + 20.0;
                sy = ys[track] + heights[track] / 2.0;
                start_track = track;
                break 'scan;
            }
            cursor = cursor.max(end);
        }
        if visible_tx_max - cursor >= 40.0 && cursor + 20.0 < visible_tx_max {
            sx = cursor + 20.0;
            sy = ys[track] + heights[track] / 2.0;
            start_track = track;
            break 'scan;
        }
    }
    // 目标：异轨可见片段的中心（与判据 `clip_marquee_selection` 同款选法）。
    let target = view
        .clips
        .iter()
        .find(|c| {
            let (x, _, w, _) = block(c);
            c.track_index != start_track && x + w / 2.0 < visible_tx_max
        })
        .ok_or("必须有异轨可见片段")?;
    let (tx0, ty0, tw, th) = block(target);
    let (ex, ey) = (tx0 + tw / 2.0, ty0 + th / 2.0);
    println!(
        "marquee content rect: ({sx:.1}, {sy:.1}) -> ({ex:.1}, {ey:.1}); target clip {}",
        target.placement_id
    );

    save(&port.window().capture()?, &out.join("marquee-before.png"))?;

    port.dispatch_pointer_down(
        FLICKABLE,
        origin_x + f64::from(sx),
        f64::from(sy),
        PointerButton::Left,
    )?;
    for step in 1..=3 {
        let t = f64::from(step) / 3.0;
        port.dispatch_pointer_move(
            f64::from(bounds.x) + origin_x + f64::from(sx) + f64::from(ex - sx) * t,
            f64::from(bounds.y) + f64::from(sy) + f64::from(ey - sy) * t,
        )?;
    }
    let drag_frame = port.window().capture()?;
    save(&drag_frame, &out.join("marquee-drag.png"))?;
    // 3x 裁切中心 = 矩形窗口帧中心。
    let ccx = (f64::from(bounds.x) + origin_x + f64::from(sx + ex) / 2.0) as u32;
    let ccy = (f64::from(bounds.y) + f64::from(sy + ey) / 2.0) as u32;
    save(
        &crop3x(&drag_frame, ccx, ccy, 720, 480),
        &out.join("marquee-drag-3x.png"),
    )?;

    port.dispatch_pointer_up(PointerButton::Left)?;
    save(&port.window().capture()?, &out.join("marquee-after.png"))?;
    let selected: Vec<String> = port
        .ui()
        .get_selected_clip_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    println!("selected after drag: {selected:?}");
    println!("primary: {}", port.ui().get_selected_clip_ulid().as_str());
    Ok(())
}
