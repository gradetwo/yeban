//! T-FIX-152：编排区曲式段落带 —— 真实数据渲染证据。
//!
//! 证：工程里有 sections 时，活编排区顶部画出段落带，且**用的是工程内色标**。
//! ⛔ 不证：人眼观感、macOS 实机。
//!
//! 产物：target/ui-test-port/tfix152-sections-1x.png / -3x.png

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_model::EntityId;
use yeban_model::project::SectionV3;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, write_artifact};
use yeban_ui_test_port::{Rgb8Image, Size};

const OK: [u8; 3] = [0x34, 0x78, 0x64];
const WARN: [u8; 3] = [0x9a, 0x6a, 0x38];

fn matches(rgb: [u8; 3], target: [u8; 3]) -> bool {
    rgb.iter()
        .zip(target.iter())
        .all(|(a, b)| a.abs_diff(*b) <= 6)
}

#[test]
fn arrangement_draws_section_band_with_project_colors() {
    let mut project = yeban_model::samples::filled_project();
    let mut first = SectionV3 {
        id: EntityId::new(),
        name: "Intro".to_owned(),
        start_tick: 0,
        end_tick: 1_920,
        color: Some("#347864".to_owned()),
    };
    first.id = EntityId::new();
    let second = SectionV3 {
        id: EntityId::new(),
        name: "Verse".to_owned(),
        start_tick: 1_920,
        end_tick: 3_840,
        color: Some("#9a6a38".to_owned()),
    };
    project.sections.insert(first.id, first);
    project.sections.insert(second.id, second);
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    assert!(
        view.sections.len() >= 2,
        "投影必须带两段段落: view={} project={}",
        view.sections.len(),
        project.sections.len()
    );

    let scene = DemoScene::from_view(&view);
    let port = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("LivePort 装配");
    host::apply_view(port.ui(), &view, 1440.0, 0.0);
    port.window().request_redraw();
    let image = port.window().capture().expect("1:1 抓帧");
    let path = write_artifact("tfix152-sections-1x", &image).expect("落盘");
    println!("[T-FIX-152] 1:1 渲染: {}", path.display());

    let (mut ok_px, mut warn_px) = (0u32, 0u32);
    let mut top = u32::MAX;
    let (mut min_x, mut max_x, mut max_y) = (u32::MAX, 0u32, 0u32);
    for y in 0..image.size().height.min(160) {
        for x in 0..image.size().width {
            let Some(px) = image.pixel(x, y) else {
                continue;
            };
            if matches(px, OK) || matches(px, WARN) {
                if matches(px, OK) {
                    ok_px += 1;
                } else {
                    warn_px += 1;
                }
                top = top.min(y);
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    println!("[T-FIX-152] 段色像素: ok={ok_px} warn={warn_px} 最高 y={top}");
    assert!(ok_px > 100, "第一段色标必须真的画出来: {ok_px} 像素");
    assert!(warn_px > 100, "第二段色标必须真的画出来: {warn_px} 像素");
    assert!(top < 100, "段落带必须在编排区顶部: 最高 y={top}");

    // 3x 裁切：按段落带实际包围盒裁（左右各留 20px、上下各留 10px）。
    let crop_x = min_x.saturating_sub(20);
    let crop_y = top.saturating_sub(10);
    let crop_w = (max_x + 20).saturating_sub(crop_x);
    let crop_h = (max_y + 10).saturating_sub(crop_y);
    let mut zoom = Rgb8Image::new(Size::new(crop_w * 3, crop_h * 3));
    for y in 0..crop_h {
        for x in 0..crop_w {
            let Some(px) = image.pixel(crop_x + x, crop_y + y) else {
                continue;
            };
            for dy in 0..3 {
                for dx in 0..3 {
                    zoom.set_pixel(x * 3 + dx, y * 3 + dy, px);
                }
            }
        }
    }
    let path3x = write_artifact("tfix152-sections-3x", &zoom).expect("落盘 3x");
    println!("[T-FIX-152] 3x 裁切: {}", path3x.display());
}
