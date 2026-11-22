//! T-FIX-114：主控真立体声 —— **1:1 渲染证据**。
//!
//! ## 这条判据要证的句子
//!
//! 引擎帧带左右两个峰值（`MeterFrame::peak_l` / `peak_r`），快照分成两根柱高
//! （`MeterSnapshot::master_level_l` / `master_level_r`），界面两柱分别显示。
//! 这里把「左 0.95 / 右 0.15」的读数真的画出来，抓 1:1 像素，**按柱子自身的
//! 填充色**量两根柱子的点亮高度：左柱必须明显高于右柱。
//!
//! ⛔ 不证：人耳听感、主题以外的配色、macOS 实机。
//! ⭐ 产物：`target/ui-test-port/tfix114-master-stereo-1x.png`（整窗 1:1）与
//! `tfix114-master-stereo-3x.png`（电平区最近邻 3× 裁切）。

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::meters::{ChannelMeter, MeterSnapshot};
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, write_artifact};
use yeban_ui_test_port::{Rgb8Image, Size};

/// `Theme.status-ok`（#347864）：0.15 那根柱的填充色。
const STATUS_OK: [u8; 3] = [0x34, 0x78, 0x64];
/// `Theme.status-warn`（#9a6a38）：0.95 那根柱的填充色。
const STATUS_WARN: [u8; 3] = [0x9a, 0x6a, 0x38];
/// `Theme.status-alert`（#a44848）：高于 0.85 的柱段填充色（0.95 走这一档）。
const STATUS_ALERT: [u8; 3] = [0xa4, 0x48, 0x48];

fn close(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| x.abs_diff(*y) <= 4)
}

fn matches(rgb: [u8; 3]) -> bool {
    close(rgb, STATUS_OK) || close(rgb, STATUS_WARN) || close(rgb, STATUS_ALERT)
}

fn demo_snapshot(view: &ViewState, left: f32, right: f32) -> MeterSnapshot {
    let mut master = ChannelMeter::silent(usize::MAX, "master".into(), "Master".into());
    master.level = left.max(right);
    master.level_l = left;
    master.level_r = right;
    let tracks = view
        .tracks
        .iter()
        .map(|track| ChannelMeter::silent(track.index, track.id.clone(), track.name.clone()))
        .collect();
    MeterSnapshot {
        tracks,
        master: Some(master),
        quantum: Some(11),
        nodes_seen: 1,
    }
}

/// 一列里柱子填充色的像素数，以及最上面的那一行（限定在 `[top, bottom]` 带内）。
fn column_fill(image: &Rgb8Image, x: u32, top: u32, bottom: u32) -> (usize, Option<u32>) {
    let mut count = 0usize;
    let mut first = None;
    for y in top..bottom {
        if let Some(px) = image.pixel(x, y)
            && matches(px)
        {
            count += 1;
            first.get_or_insert(y);
        }
    }
    (count, first)
}

/// 相邻列聚成一组（间距 ≤ 4 px）。
fn cluster(hits: Vec<(u32, usize, u32)>) -> Vec<Vec<(u32, usize, u32)>> {
    let mut groups: Vec<Vec<(u32, usize, u32)>> = Vec::new();
    for hit in hits {
        match groups.last_mut() {
            Some(group) if hit.0 <= group.last().unwrap().0 + 4 => group.push(hit),
            _ => groups.push(vec![hit]),
        }
    }
    groups
}

/// 在右侧控制室区域里找出两根柱子。
///
/// 两柱共用同一个纵向带，但高度可能相差很大（0.95 对 0.15 ⇒ 56px 对 9px），
/// 所以先找高的那根作种子，再在**同一纵向带**里找矮的那根（阈值降到 3 px）。
fn find_bars(image: &Rgb8Image) -> Vec<(u32, u32, usize, u32)> {
    let width = image.size().width;
    let height = image.size().height;
    let x0 = width.saturating_sub(320);

    let mut seed: Vec<(u32, usize, u32)> = Vec::new();
    for x in x0..width {
        let (count, top) = column_fill(image, x, 0, height);
        if count >= 20
            && let Some(top) = top
        {
            seed.push((x, count, top));
        }
    }
    let mut groups = cluster(seed);
    groups.sort_by_key(|group| std::cmp::Reverse(group.len()));
    let tall = groups
        .into_iter()
        .next()
        .expect("必须找到一根有高度的主控柱");
    assert!(
        tall.len() >= 5,
        "高柱应是一个实心列组（≥5 列），实际 {} 列",
        tall.len()
    );
    let tall_start = tall.first().unwrap().0;
    let tall_end = tall.last().unwrap().0;
    let tall_height = tall.iter().map(|hit| hit.1).max().unwrap();
    let band_top = tall.iter().map(|hit| hit.2).min().unwrap();
    let band_bottom = band_top + tall_height as u32 + 3;

    let mut other: Vec<(u32, usize, u32)> = Vec::new();
    for x in x0..width {
        if x >= tall_start.saturating_sub(2) && x <= tall_end + 2 {
            continue;
        }
        let (count, top) = column_fill(image, x, band_top, band_bottom);
        if count >= 3
            && let Some(top) = top
        {
            other.push((x, count, top));
        }
    }
    let short_group = cluster(other)
        .into_iter()
        .filter(|group| group.len() >= 5)
        .min_by_key(|group| group.first().unwrap().0.abs_diff(tall_start))
        .expect("两根柱子必须都在同一纵向带里");
    let short_start = short_group.first().unwrap().0;
    let short_end = short_group.last().unwrap().0;
    let short_height = short_group.iter().map(|hit| hit.1).max().unwrap();
    let short_top = short_group.iter().map(|hit| hit.2).min().unwrap();

    let mut bars = vec![
        (tall_start, tall_end, tall_height, band_top),
        (short_start, short_end, short_height, short_top),
    ];
    bars.sort_by_key(|bar| bar.0);
    bars
}

#[test]
fn master_bars_render_with_different_heights_when_stereo_inputs_differ() {
    let view = ViewState::demo();
    let scene = DemoScene::from_view(&view);
    let port = LivePort::new(Size::new(1920, 1080), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("LivePort 必须成功装配 MainWindow");
    let ui = port.ui();
    ui.set_show_control_room(true);
    ui.set_control_room_width(200.0);

    // 左强右弱：真立体声的判别输入。
    host::apply_meters(ui, &demo_snapshot(&view, 0.95, 0.15));
    port.window().request_redraw();
    let image = port.window().capture().expect("1:1 抓帧");
    let path = write_artifact("tfix114-master-stereo-1x", &image).expect("落盘 1:1 渲染");
    println!("[T-FIX-114] 1:1 渲染: {}", path.display());

    let bars = find_bars(&image);
    assert_eq!(
        bars.len(),
        2,
        "主控必须恰好两根实心柱（左/右），实际 {bars:?}（(x0, x1, 高, 顶)）"
    );
    let (left_start, left_end, left_height, left_top) = bars[0];
    let (right_start, right_end, right_height, right_top) = bars[1];
    println!(
        "[T-FIX-114] 柱高像素: 左={left_height}px(x {left_start}..{left_end}, 顶 {left_top}) \
右={right_height}px(x {right_start}..{right_end}, 顶 {right_top})（期望左 >> 右：0.95 vs 0.15）"
    );
    assert!(
        left_height > right_height + 20 && left_height >= right_height * 3,
        "左柱（0.95）必须明显高于右柱（0.15）：左 {left_height}px vs 右 {right_height}px"
    );
    assert!(
        right_height >= 4,
        "右柱本身必须有可见高度（0.15 也画得出来），实际 {right_height}px"
    );

    // 3× 最近邻裁切：两柱 + 上下留白。
    let crop_x = left_start.saturating_sub(10);
    let crop_w = (right_end + 12).saturating_sub(crop_x);
    let crop_h = (left_top + left_height as u32 + 16).max(200);
    let mut zoom = Rgb8Image::new(Size::new(crop_w * 3, crop_h * 3));
    for y in 0..crop_h {
        for x in 0..crop_w {
            let Some(px) = image.pixel(crop_x + x, y) else {
                continue;
            };
            for dy in 0..3 {
                for dx in 0..3 {
                    zoom.set_pixel(x * 3 + dx, y * 3 + dy, px);
                }
            }
        }
    }
    let path3x = write_artifact("tfix114-master-stereo-3x", &zoom).expect("落盘 3× 裁切");
    println!("[T-FIX-114] 3× 裁切: {}", path3x.display());

    // 反相输入（左右互换）再抓一帧：两柱高度必须互换，证明确实各自读自己的声道。
    host::apply_meters(ui, &demo_snapshot(&view, 0.15, 0.95));
    port.window().request_redraw();
    let swapped = port.window().capture().expect("反相抓帧");
    let swapped_bars = find_bars(&swapped);
    assert_eq!(swapped_bars.len(), 2, "反相帧仍须两根柱: {swapped_bars:?}");
    let swapped_left = swapped_bars[0].2;
    let swapped_right = swapped_bars[1].2;
    println!("[T-FIX-114] 反相后柱高像素: 左={swapped_left} 右={swapped_right}");
    assert!(
        swapped_right > swapped_left + 20 && swapped_right >= swapped_left * 3,
        "左右互换后右柱必须变高、左柱变矮：左 {swapped_left}px vs 右 {swapped_right}px"
    );
}
