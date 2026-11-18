//! 临时几何探针（不是断言测试，永不失败）：把不同窗口高度下的真实布局数打出来。
//!
//! 目的：A1「编曲区只露 2.5 轨」在像素层面的反推已经卡住两轮，需要在**测试口**
//! 里直接读几何，而不是继续按截图猜。这里只读、不写、不改仓库：
//!
//! - `workspace-arrangement-canvas`：编曲画布（行区真正分到的槽位）
//! - `arrangement-ruler` / `track-{i}-header`：标尺带与每条轨道的包头
//! - `console-tab-rail`：下坞（它的 y 就是轨道区可视槽位的底沿）
//! - 注入的 `arrangement-sheet-height`：`.slint` 拿它当内容高的原料
//!
//! 跑法（复用 release 产物，避免为了测试把依赖重编一遍）：
//! `cargo test --release --test ui_geometry_probe -- --nocapture --test-threads=1`

use slint::ComponentHandle as _;
use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, Size, UiTestPort as _};

const RULER_HEIGHT: f32 = 22.0;

fn dump(width: u32, height: u32) {
    let project = demo_project();
    let view = ViewState::from_project(&project).expect("演示工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let mut port = LivePort::new(
        Size::new(width, height),
        Permission::Interactive,
        None,
        || host::build_main_window(&view, &scene),
    )
    .expect("Tier-1 平台 + 主窗口");
    port.refresh_tree(None).expect("必须能抓控件树");

    println!("\n===== {width}×{height} 几何探针 =====");
    let rect_of = |id: &str| port.tree().find_by_id(id).and_then(|node| node.bounds);

    let canvas = rect_of("workspace-arrangement-canvas");
    let splitter = rect_of("splitter-workspace-console");
    let dock = rect_of("console-tab-rail");
    let ruler = rect_of("arrangement-ruler");
    let lanes = rect_of("arr-lanes-flickable");
    let roll = rect_of("piano-roll");

    println!("编曲画布 canvas      : {canvas:?}");
    println!("分区拖拽条 splitter   : {splitter:?}");
    println!("下坞 console-tab-rail: {dock:?}");
    println!("标尺 arrangement-ruler: {ruler:?}");
    println!("行区 flickable       : {lanes:?}");
    println!("钢琴卷帘 piano-roll  : {roll:?}");

    let mut seen_rows = 0usize;
    for index in 0..view.tracks.len() {
        let id = format!("track-{index}-header");
        match rect_of(&id) {
            Some(b) => {
                seen_rows += 1;
                println!("  {id}: y={} h={}", b.y, b.height);
            }
            None => println!("  {id}: 不在控件树"),
        }
    }

    let injected = port.ui().get_arrangement_sheet_height();
    println!("在树的轨道行数       : {seen_rows}/{}", view.tracks.len());
    println!("注入 sheet-height    : {injected:.1}");
    println!("注入 window-h        : {:.1}", port.ui().get_window_h());

    if let (Some(c), Some(d)) = (canvas, dock) {
        let canvas_bottom = c.y as f32 + c.height as f32;
        println!("--- 推导 ---");
        println!("画布 y..底沿         : {}..{canvas_bottom:.0}", c.y);
        println!("下坞顶               : {}", d.y);
        if let Some(r) = ruler {
            println!(
                "行区可视槽位高(模型)  : {:.1}（画布底 − 标尺 y − 22）",
                canvas_bottom - r.y as f32 - RULER_HEIGHT
            );
        }
        let last = view.tracks.last().expect("至少一行");
        println!(
            "最后一行底沿(模型)    : {:.1}（相对画布顶 {:.1}）",
            last.y + last.height,
            last.y + last.height + yeban_app::automation::TRACK_LANE_TOP_PX
        );
        println!("内容高(注入+42+46)    : {:.1}", injected + 42.0 + 46.0);
    }
    println!("===== 结束 =====\n");

    let _ = port.ui().hide();
}

#[test]
fn probe_1440x700() {
    dump(1440, 700);
}

#[test]
fn probe_1440x900() {
    dump(1440, 900);
}

#[test]
fn probe_1440x600() {
    dump(1440, 600);
}
