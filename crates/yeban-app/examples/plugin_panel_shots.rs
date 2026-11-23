//! 插件与 ARA 面板证据图（T-FIX-83，取证工具，**不是**产品路径）。
//!
//! 用法：
//! `cargo run -p yeban-app --example plugin_panel_shots -- [输出目录]`
//!
//! 三帧：
//! 1. `plugin-entry-1x.png` —— 下坞「插件」入口（关闭面板时的 1:1）；
//! 2. `plugin-panel-1x.png` —— 面板打开后的 1:1，左右两栏同时可见；
//! 3. `plugin-panel-3x.png` —— 面板中段最近邻 3x 裁切，供逐字目视。
//!
//! 面板里的数据行是本工具直接写模型（视觉取证），不是扫描结果；扫描 /
//! 装载 / ARA 的真实语义由 `plugin_panel` 的单元测试与宿主编排覆盖。

use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::{
    ComponentHandle as _, Model as _, ModelRc, PhysicalSize, Rgb8Pixel, SharedString, VecModel,
};
use yeban_app::cli::{self, Language, Theme};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::{AraRowData, PluginRowData};

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

/// 3x 最近邻裁切：以 `(cx, cy)` 为中心，从源图取 `w / 3 × h / 3` 放大三倍。
fn crop3x(
    frame: &[Rgb8Pixel],
    width: u32,
    height: u32,
    cx: u32,
    cy: u32,
    w: u32,
    h: u32,
) -> Vec<Rgb8Pixel> {
    let sw = w / 3;
    let sh = h / 3;
    let x0 = cx.saturating_sub(sw / 2).min(width.saturating_sub(sw));
    let y0 = cy.saturating_sub(sh / 2).min(height.saturating_sub(sh));
    let mut out = vec![Rgb8Pixel::new(0, 0, 0); (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let sx = x0 + x / 3;
            let sy = y0 + y / 3;
            if sx < width && sy < height {
                out[(y * w + x) as usize] = frame[(sy * width + sx) as usize];
            }
        }
    }
    out
}

fn main() {
    let out_dir = std::env::args().nth(1).unwrap_or_else(|| {
        std::env::temp_dir()
            .join("yeban-tfix83-shots")
            .to_string_lossy()
            .into_owned()
    });
    std::fs::create_dir_all(&out_dir).expect("创建输出目录");

    // SAFETY: 进程启动后主线程第一件事；避免写到用户真实设置。
    unsafe {
        std::env::set_var(
            yeban_app::settings::SETTINGS_PATH_ENV,
            format!("{out_dir}/.settings-test.json"),
        );
    }

    let (width, height) = (1600u32, 900u32);
    let window = install_platform(width, height);

    let project = yeban_model::samples::filled_project();
    let view = cli::project_view(&project).expect("投影 filled 工程");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window(&view, &scene).expect("构造主窗口");
    host::apply_theme(&ui, Theme::Yeban);
    host::apply_language(&ui, Language::ZhCn);
    host::wire_plugin_panel(&ui, None);
    ui.show().expect("show 主窗口");
    let _ = render_frame(&window, width, height);

    // 帧 1：关闭面板，下坞「插件」入口 1:1。
    let (frame, elapsed) = render_frame(&window, width, height).expect("entry 帧必须重绘");
    write_png(
        &format!("{out_dir}/plugin-entry-1x.png"),
        &frame,
        width,
        height,
    );
    println!(
        "plugin-shot frame=entry render_ms={:.2} non_black={} path={out_dir}/plugin-entry-1x.png",
        elapsed.as_secs_f64() * 1000.0,
        non_black_pixels(&frame),
    );

    // 帧 2：开面板，视觉数据行（两列同时可见）。
    ui.invoke_open_plugin_panel();
    assert!(ui.get_plugin_panel_open(), "插件面板必须打开");
    ui.set_plugin_target_label(SharedString::from("Track 2 · Pad（示例目标轨）"));
    ui.set_plugin_scan_paths(SharedString::from(r"C:\VST3;D:\Audio\Plugins"));
    ui.set_plugin_scan_status(SharedString::from(
        "扫描 3 个目录：2 个插件可用 / 1 个失败（失败原因见行内原文）",
    ));
    ui.set_plugin_notice(SharedString::from(
        "装载走 Host::load(_effect) 的同一验证；失败原文保留，不回退默认。",
    ));
    ui.set_plugin_rows(ModelRc::new(VecModel::from(vec![
        PluginRowData {
            name: SharedString::from("Serum"),
            meta: SharedString::from("VST3 · instrument · 2 类"),
            state: SharedString::from("可用"),
            path: SharedString::from(r"C:\VST3\Serum.vst3"),
            class_id: SharedString::from("com.xfer.serum"),
            kind: SharedString::from("instrument"),
            usable: true,
            ara: false,
        },
        PluginRowData {
            name: SharedString::from("Pro-Q 3"),
            meta: SharedString::from("VST3 · effect · 1 类"),
            state: SharedString::from("可用"),
            path: SharedString::from(r"C:\VST3\Pro-Q 3.vst3"),
            class_id: SharedString::from("com.fabfilter.proq3"),
            kind: SharedString::from("effect"),
            usable: true,
            ara: false,
        },
        PluginRowData {
            name: SharedString::from("BrokenAmp"),
            meta: SharedString::from("VST3 · 模块级失败"),
            state: SharedString::from("失败：入口点缺失（原文保留）"),
            path: SharedString::from(r"C:\VST3\BrokenAmp.vst3"),
            class_id: SharedString::from(""),
            kind: SharedString::from(""),
            usable: false,
            ara: false,
        },
    ])));
    ui.set_plugin_selected_row(1);
    ui.set_plugin_error(SharedString::from(""));
    ui.set_ara_rows(ModelRc::new(VecModel::from(vec![AraRowData {
        id: SharedString::from("ara-1"),
        factory: SharedString::from("Melodyne 5"),
        state: SharedString::from("分析 可用"),
        detail: SharedString::from("region 2 · head -0.25s / tail 0.50s · 音符 12"),
    }])));
    ui.set_ara_selected_row(0);
    ui.set_ara_plugin_path(SharedString::from(r"C:\VST3\Melodyne.vst3"));
    ui.set_ara_wav_path(SharedString::from(r"C:\Temp\pad-loop.wav"));
    ui.set_ara_status(SharedString::from("已绑定 1 个会话（上限 64）"));
    ui.set_ara_query_result(SharedString::from(
        "分析可用：是 · region 2 · 采样率 48000 Hz · 时长 4.00s · head -0.25s / tail 0.50s · 音符 12",
    ));
    ui.set_ara_out_path(SharedString::from(r"C:\Temp\yeban-ara-render.wav"));
    ui.set_ara_progress(0.62);
    ui.set_ara_result(SharedString::from(
        r"渲染中 62% · 已写 128 块 · 目标 C:\Temp\yeban-ara-render.wav",
    ));

    let (frame, elapsed) = render_frame(&window, width, height).expect("panel 帧必须重绘");
    write_png(
        &format!("{out_dir}/plugin-panel-1x.png"),
        &frame,
        width,
        height,
    );
    println!(
        "plugin-shot frame=panel rows={} ara_rows={} render_ms={:.2} non_black={} path={out_dir}/plugin-panel-1x.png",
        ui.get_plugin_rows().row_count(),
        ui.get_ara_rows().row_count(),
        elapsed.as_secs_f64() * 1000.0,
        non_black_pixels(&frame),
    );

    // 帧 3：面板中段 3x 裁切（中心 800,450；源 480×320 → 1440×960）。
    let cropped = crop3x(&frame, width, height, 800, 450, 1440, 960);
    write_png(
        &format!("{out_dir}/plugin-panel-3x.png"),
        &cropped,
        1440,
        960,
    );
    println!("plugin-shot frame=panel-3x path={out_dir}/plugin-panel-3x.png");

    ui.invoke_plugin_close();
    assert!(!ui.get_plugin_panel_open(), "关闭后 open 必须为假");
    println!("plugin-shot OK");
}

// ---------------------------------------------------------------------------
// 最小 PNG 写出（存储式 deflate；与 examples/drum_gen_shots.rs 同款）
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
    let mut raw = Vec::with_capacity((width * height * 3 + height) as usize);
    for row in 0..height {
        raw.push(0u8);
        for column in 0..width {
            let pixel = frame[(row * width + column) as usize];
            raw.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
        }
    }
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
