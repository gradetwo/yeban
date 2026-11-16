//! 双语 i18n 证据图（`line/ui-i18n` 的取证工具，**不是**产品路径）。
//!
//! 用法：
//! `cargo run -p yeban-app --example i18n_shots -- [输出目录]`
//! （缺省输出到 `~/workspace/i18n-shots`）。
//!
//! 同一份**真实工程**（`yeban_model::samples::filled_project()`）+ 真投影，
//! 在同一活窗口上依次产出三帧：
//! 1. `main-zh.png` —— 启动语言 zh-CN（经 `host::apply_language`，与
//!    `main.rs` 的 `--lang` 写入口同路）；
//! 2. `main-en.png` —— 经**真实开关路径**切英文：调用主窗口的
//!    `toggle-language` 回调（走带栏 Group L 开关在 .slint 侧转发的就是它，
//!    宿主接管后走 `apply_language`），不重启、不重建窗口；
//! 3. `main-zh-back.png` —— 再调一次回调切回中文；与第 1 帧逐像素比对，
//!    不一致即非零退出（往返无损的机械见证）。
//!
//! stdout 打印每帧的非黑像素数与 `language-en` 属性读数作见证。

use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::{ComponentHandle as _, PhysicalSize, Rgb8Pixel};
use yeban_app::cli::{self, Language, Theme};
use yeban_app::host;
use yeban_app::scene::DemoScene;

/// 与 `tests/theme_selection.rs` 同款的自研软件平台。
struct ShotPlatform {
    window: Rc<MinimalSoftwareWindow>,
}

impl slint::platform::Platform for ShotPlatform {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
}

fn install_platform(width: u32, height: u32) -> Rc<MinimalSoftwareWindow> {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(PhysicalSize::new(width, height));
    let platform_window = window.clone();
    slint::platform::set_platform(Box::new(ShotPlatform {
        window: platform_window,
    }))
    .expect("安装软件平台");
    window
}

/// 光栅化一帧并计时。`None` = 窗口未要求重绘（帧与上一帧逐像素相同）。
/// 必须先显式 `request_redraw`（仓内既定写法，理由见 headless_idle.rs）。
fn render_frame(
    window: &MinimalSoftwareWindow,
    width: u32,
    height: u32,
) -> Option<(Vec<Rgb8Pixel>, Duration)> {
    let mut buffer = vec![Rgb8Pixel::new(0, 0, 0); (width * height) as usize];
    window.request_redraw();
    let start = Instant::now();
    let drawn = window.draw_if_needed(|renderer| {
        renderer.render(&mut buffer, width as usize);
    });
    let elapsed = start.elapsed();
    drawn.then_some((buffer, elapsed))
}

fn non_black_pixels(frame: &[Rgb8Pixel]) -> usize {
    frame
        .iter()
        .filter(|pixel| pixel.r != 0 || pixel.g != 0 || pixel.b != 0)
        .count()
}

fn main() {
    let out_dir = std::env::args().nth(1).unwrap_or_else(|| {
        let home = std::env::var("HOME").expect("HOME 必须存在");
        format!("{home}/workspace/i18n-shots")
    });
    std::fs::create_dir_all(&out_dir).expect("创建输出目录");

    let (width, height) = (1600u32, 900u32);
    let window = install_platform(width, height);

    let project = yeban_model::samples::filled_project();
    let view = cli::project_view(&project).expect("投影 filled 工程");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window(&view, &scene).expect("构造主窗口");
    host::apply_theme(&ui, Theme::Yeban);
    // 与 main.rs 同一时机同一写入口：建窗后、run 之前显式应用启动语言。
    host::apply_language(&ui, Language::ZhCn);
    ui.show().expect("show 主窗口");

    // 热身帧（布局与首帧构建不计入语言切换的见证）。
    let _ = render_frame(&window, width, height);

    let (zh_frame, zh_elapsed) = render_frame(&window, width, height).expect("中文帧必须重绘");
    write_png(&format!("{out_dir}/main-zh.png"), &zh_frame, width, height);
    println!(
        "i18n-shot frame=zh language_en={} render_ms={:.2} non_black={} path={out_dir}/main-zh.png",
        ui.get_language_en(),
        zh_elapsed.as_secs_f64() * 1000.0,
        non_black_pixels(&zh_frame),
    );

    // 真实开关路径：Group L 开关 → MainWindow.toggle-language 回调 → 宿主接管。
    ui.invoke_toggle_language();
    let (en_frame, en_elapsed) = render_frame(&window, width, height).expect("英文帧必须重绘");
    write_png(&format!("{out_dir}/main-en.png"), &en_frame, width, height);
    println!(
        "i18n-shot frame=en language_en={} render_ms={:.2} non_black={} path={out_dir}/main-en.png",
        ui.get_language_en(),
        en_elapsed.as_secs_f64() * 1000.0,
        non_black_pixels(&en_frame),
    );
    assert!(ui.get_language_en(), "切换后 language-en 必须为 true");

    ui.invoke_toggle_language();
    let (back_frame, _) = render_frame(&window, width, height).expect("回切中文帧必须重绘");
    write_png(
        &format!("{out_dir}/main-zh-back.png"),
        &back_frame,
        width,
        height,
    );
    assert!(!ui.get_language_en(), "回切后 language-en 必须为 false");
    let roundtrip_diff = zh_frame
        .iter()
        .zip(back_frame.iter())
        .filter(|(a, b)| a != b)
        .count();
    println!(
        "i18n-shot frame=zh-back roundtrip_diff_px={roundtrip_diff} path={out_dir}/main-zh-back.png"
    );
    assert_eq!(roundtrip_diff, 0, "语言往返必须逐像素无损");
    println!("i18n-shot OK");
}

// ---------------------------------------------------------------------------
// 最小 PNG 写出（存储式 deflate；无新依赖，与仓内既有写出器的同款取舍）
// ---------------------------------------------------------------------------

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

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

fn write_png(path: &str, frame: &[Rgb8Pixel], width: u32, height: u32) {
    // 原始扫描行（每行 1 字节 filter=0 + RGB）。
    let mut raw = Vec::with_capacity((width * height * 3 + height) as usize);
    for row in 0..height {
        raw.push(0u8);
        for column in 0..width {
            let pixel = frame[(row * width + column) as usize];
            raw.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
        }
    }
    // zlib 存储式流：头 0x78 0x01 + 若干 ≤65535 的 stored 块 + adler32。
    let mut idat = vec![0x78u8, 0x01];
    let mut chunks = raw.chunks(65_535).peekable();
    while let Some(block) = chunks.next() {
        let last = chunks.peek().is_none();
        idat.push(u8::from(last));
        let len = block.len() as u16;
        idat.extend_from_slice(&len.to_le_bytes());
        idat.extend_from_slice(&(!len).to_le_bytes());
        idat.extend_from_slice(block);
    }
    idat.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut png = Vec::new();
    png.extend_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    write_chunk(&mut png, b"IHDR", &ihdr);
    write_chunk(&mut png, b"IDAT", &idat);
    write_chunk(&mut png, b"IEND", &[]);
    std::fs::write(path, png).expect("写 PNG");
}
