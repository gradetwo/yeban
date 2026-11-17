//! 去透明线出帧取证（非产品代码）：离屏软件渲染三组帧，供 1:1 / 3x 目视验收。
//!
//! 模式：
//! - `main`：主窗口 1600×900（filled 工程）—— Clip 矩阵抽屉帧 + lower dock
//!   （混音台页 + 控制台最大化，露通道条徽章/选中与总线条）帧。
//! - `rack`：900×210 微型 harness 直接渲染 DeviceRack 本体 —— EQ 频响填充帧
//!   （DeviceRack 在产品主窗口里只有 `if false` 转发桩、不实际渲染）。
//!
//! 平台/写出器与 `sound_library_shots` 同款（MinimalSoftwareWindow 离屏渲染）。

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError};
use slint::{ComponentHandle, PhysicalSize, Rgb8Pixel};

use yeban_app::cli::{self, Language, Theme};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::Theme as UiTheme;

thread_local! {
    static PANIC_WINDOW: RefCell<Option<Rc<MinimalSoftwareWindow>>> = const { RefCell::new(None) };
}

fn install_platform(width: u32, height: u32) -> Rc<MinimalSoftwareWindow> {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(PhysicalSize::new(width, height));
    PANIC_WINDOW.with(|slot| *slot.borrow_mut() = Some(window.clone()));
    struct PanicPlatform;
    impl Platform for PanicPlatform {
        fn create_window_adapter(
            &self,
        ) -> Result<Rc<dyn slint::platform::WindowAdapter>, PlatformError> {
            PANIC_WINDOW.with(|slot| {
                slot.borrow()
                    .clone()
                    .map(|window| window as Rc<dyn slint::platform::WindowAdapter>)
                    .ok_or(PlatformError::Other("取证平台窗口未安装".to_string()))
            })
        }
    }
    slint::platform::set_platform(Box::new(PanicPlatform))
        .unwrap_or_else(|_| panic!("安装取证平台失败（进程内只能装一次）"));
    window
}

fn render_frame(
    window: &MinimalSoftwareWindow,
    width: u32,
    height: u32,
) -> Option<(Vec<Rgb8Pixel>, Duration)> {
    let start = std::time::Instant::now();
    let mut frame = vec![Rgb8Pixel::new(0, 0, 0); (width * height) as usize];
    // 必须先显式 request_redraw（仓内既定写法，理由见 headless_idle.rs）。
    window.request_redraw();
    let drawn = window.draw_if_needed(|renderer| {
        renderer.render(&mut frame, width as usize);
    });
    drawn.then_some((frame, start.elapsed()))
}

fn non_black_pixels(frame: &[Rgb8Pixel]) -> usize {
    frame
        .iter()
        .filter(|pixel| pixel.r > 8 || pixel.g > 8 || pixel.b > 8)
        .count()
}

fn write_png(path: &str, frame: &[Rgb8Pixel], width: u32, height: u32) {
    let mut raw = Vec::with_capacity(((width * 3 + 1) * height) as usize);
    for y in 0..height {
        raw.push(0u8);
        for x in 0..width {
            let pixel = frame[(y * width + x) as usize];
            raw.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
        }
    }
    let mut compressed = vec![0x78, 0x01];
    for chunk in raw.chunks(65_535) {
        let last = chunk.len() + compressed.len() >= raw.len() + 2 - 4;
        compressed.push(if last { 0x01 } else { 0x00 });
        compressed.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        compressed.extend_from_slice(&(!(chunk.len() as u16)).to_le_bytes());
        compressed.extend_from_slice(chunk);
    }
    compressed.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    write_chunk(&mut png, b"IHDR", &ihdr);
    write_chunk(&mut png, b"IDAT", &compressed);
    write_chunk(&mut png, b"IEND", &[]);
    std::fs::write(path, png).unwrap_or_else(|e| panic!("写出 {path} 失败: {e}"));
}

fn write_chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    png.extend_from_slice(&body);
    png.extend_from_slice(&crc32(&body).to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (index, entry) in table.iter_mut().enumerate() {
        let mut value = index as u32;
        for _ in 0..8 {
            value = if value & 1 != 0 {
                0xEDB8_8320 ^ (value >> 1)
            } else {
                value >> 1
            };
        }
        *entry = value;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc = table[((crc ^ u32::from(*byte)) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

fn adler32(bytes: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for byte in bytes {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn shot(window: &MinimalSoftwareWindow, width: u32, height: u32, out_dir: &str, name: &str) {
    let (frame, elapsed) =
        render_frame(window, width, height).unwrap_or_else(|| panic!("{name} 帧必须重绘"));
    let path = format!("{out_dir}/{name}.png");
    write_png(&path, &frame, width, height);
    println!(
        "dealpha-shot frame={name} render_ms={:.2} non_black={} path={path}",
        elapsed.as_secs_f64() * 1000.0,
        non_black_pixels(&frame),
    );
}

fn main() {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "main".to_string());
    let out_dir = std::env::args().nth(2).unwrap_or_else(|| {
        let home = std::env::var("HOME").expect("HOME 必须存在");
        format!("{home}/workspace/dealpha-shots")
    });
    std::fs::create_dir_all(&out_dir).expect("创建输出目录");
    // 取证运行不碰真用户设置。
    // SAFETY: 进程启动后主线程第一件事，无并发读写。
    unsafe {
        std::env::set_var(
            yeban_app::settings::SETTINGS_PATH_ENV,
            format!("{out_dir}/.settings-test.json"),
        );
    }

    match mode.as_str() {
        "main" => main_window_shots(&out_dir),
        "rack" => device_rack_shots(&out_dir),
        other => panic!("未知模式 {other}（main | rack）"),
    }
    println!("dealpha-shot OK mode={mode}");
}

fn main_window_shots(out_dir: &str) {
    let (width, height) = (1600u32, 900u32);
    let window = install_platform(width, height);

    let project = yeban_model::samples::filled_project();
    let view = cli::project_view(&project).expect("投影 filled 工程");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window(&view, &scene).expect("构造主窗口");
    host::apply_theme(&ui, Theme::Yeban);
    host::apply_language(&ui, Language::ZhCn);
    ui.show().expect("show 主窗口");
    let _ = render_frame(&window, width, height);

    // 帧 1：Clip 矩阵抽屉（含片段槽/空槽实色 + 选中行提亮）。
    ui.set_show_clip_matrix(true);
    shot(&window, width, height, out_dir, "clip-matrix");

    // 帧 2：lower dock 混音台页 + 最大化（通道条徽章/选中提亮 + 总线条）。
    ui.set_show_clip_matrix(false);
    ui.global::<UiTheme<'_>>()
        .set_active_dock_tab("mixer".into());
    ui.set_console_maximized(true);
    let _ = render_frame(&window, width, height);
    shot(&window, width, height, out_dir, "lower-dock");
}

// DeviceRack 在产品主窗口里只有 `if false` 的 ConsoleTabs 转发桩、不实际渲染
// （三页功能在 UnifiedLowerDock 存活），故 EQ 帧用微型 harness 直接渲染
// DeviceRack 本体；曲线/填充路径为低架 +3 dB 的示意曲线，填充色即本线落点。
slint::slint! {
    import { DeviceRack } from "ui/console/device_rack.slint";
    export component RackHarness inherits Window {
        width: 900px;
        height: 210px;
        background: rgb(11, 14, 19);
        DeviceRack {
            width: 900px;
            height: 210px;
            track-label: "展示轨 · 设备链";
            device-names: ["展示 EQ"];
            device-ids: ["dev-eq"];
            device-recognised: ["内置 · 已识别"];
            eq-available: true;
            eq-gain-db: 3.0;
            eq-min-db: -12.0;
            eq-max-db: 12.0;
            eq-curve-svg: "M 0 50 C 105 50 150 14 210 14 C 270 14 315 50 420 50";
            eq-curve-fill-svg: "M 0 50 C 105 50 150 14 210 14 C 270 14 315 50 420 50 L 420 100 L 0 100 Z";
            eq-curve-info: "低架 +3.0 dB · 峰值 +2.4 dB @ 1 kHz";
        }
    }
}

fn device_rack_shots(out_dir: &str) {
    let (width, height) = (900u32, 210u32);
    let window = install_platform(width, height);

    let rack = RackHarness::new().expect("构造 DeviceRack harness");
    rack.show().expect("show DeviceRack harness");
    let _ = render_frame(&window, width, height);
    shot(&window, width, height, out_dir, "device-rack-eq");
}
