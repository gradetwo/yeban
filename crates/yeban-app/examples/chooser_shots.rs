//! 启动工程选择器证据图（`line/startup-chooser` 的取证工具，**不是**
//! 产品路径）。
//!
//! 用法：
//! `cargo run -p yeban-app --example chooser_shots -- [输出目录]`
//! （缺省输出到 `~/workspace/chooser-shots`）。
//!
//! 在同一活窗口上依次产出六帧（flat 默认主题、中文）：
//! 1. `chooser-new.png` —— 「新建工程」分类（Details 收起）；
//! 2. `chooser-new-details.png` —— Details 展开（速度 / 调号 / 拍号 /
//!    采样率四个真实字段）；
//! 3. `chooser-recent.png` —— 「最近」分类（两条真实记录 + 一条失效
//!    记录置灰，数据经 `settings::record_recent_project` 真实落盘
//!    后由选择器汇集）；
//! 4. `chooser-demos.png` —— 「演示工程」（内置样本两条）；
//! 5. `chooser-factory.png` —— 「工程模板」（临时工厂目录里一条
//!    三级模板）；
//! 6. `chooser-user.png` —— 「我的模板」（临时用户目录里一条）。
//!
//! stdout 打印每帧的非黑像素数作见证（空画布充数会被这一行拆穿）。
//! 取证运行的设置文件与模板根都指到输出目录下，不碰真用户配置。

use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::{ComponentHandle as _, PhysicalSize, Rgb8Pixel};
use yeban_app::cli::{self, Language, Texture, Theme};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::startup_chooser::NewProjectDetails;
use yeban_app::templates::TemplateRoots;
use yeban_app::undo::{UndoPort, UndoSession};

/// 与 `examples/i18n_shots.rs` 同款的自研软件平台。
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

/// 光栅化一帧。`None` = 窗口未要求重绘（帧与上一帧逐像素相同）。
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

fn write_project(path: &std::path::Path, title: &str) {
    std::fs::create_dir_all(path.parent().expect("父目录")).expect("建目录");
    let mut archive = host::new_project_archive();
    archive.project.title = title.to_owned();
    yeban_app::save::save_archive_file(&archive, path).expect("写取证工程");
}

fn main() {
    let out_dir = std::env::args().nth(1).unwrap_or_else(|| {
        let home = std::env::var("HOME").expect("HOME 必须存在");
        format!("{home}/workspace/chooser-shots")
    });
    std::fs::create_dir_all(&out_dir).expect("创建输出目录");
    let out = std::path::PathBuf::from(&out_dir);

    // 取证运行不碰真用户设置：设置文件指到输出目录下。
    // SAFETY: `set_var` 在 Rust 2024 下是 `unsafe`；这里是进程启动后
    // 主线程的第一件事，窗口与工作线程尚未创建，无并发读写。
    unsafe {
        std::env::set_var(
            yeban_app::settings::SETTINGS_PATH_ENV,
            out.join(".settings-shots.json").as_os_str(),
        );
    }

    // ---- 夹具：两条真实最近记录 + 一条失效记录 + 两根模板 ---------------
    let now = yeban_model::project::now_unix_ms();
    let recent_a = out.join("夜半霓虹.yeban");
    let recent_b = out.join("练习曲.yeban");
    write_project(&recent_a, "夜半霓虹");
    write_project(&recent_b, "练习曲");
    yeban_app::settings::record_recent_project(&out.join("已删除的工程.yeban"), now - 3_600_000);
    yeban_app::settings::record_recent_project(&recent_b, now - 7_200_000);
    yeban_app::settings::record_recent_project(&recent_a, now - 300_000);
    let roots = TemplateRoots {
        factory: out.join("factory-templates"),
        user: out.join("user-templates"),
    };
    write_project(
        &roots
            .factory
            .join("流行")
            .join("抒情")
            .join("抒情钢琴.yeban"),
        "抒情钢琴",
    );
    write_project(
        &roots.user.join("电子").join("浩室").join("四拍底鼓.yeban"),
        "四拍底鼓",
    );

    let (width, height) = (1600u32, 900u32);
    let window = install_platform(width, height);

    let project = yeban_model::samples::filled_project();
    let view = cli::project_view(&project).expect("投影 filled 工程");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window(&view, &scene).expect("构造主窗口");
    host::apply_theme(&ui, Theme::Yeban);
    // 第二个命令行参数选质感：`flat` / 其它一律 physical。
    // 启动器要和主界面同一套皮肤，两种质感都得单独出一轮证据图。
    let texture = match std::env::args().nth(2).as_deref() {
        Some("flat") => Texture::Flat,
        _ => Texture::Physical,
    };
    host::apply_texture(&ui, texture);
    host::apply_language(&ui, Language::ZhCn);

    // 换工程运行时（取证装配：无引擎，after 空转——帧只看选择器本身）。
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<chooser-shots>", "yeban-app", project.clone(), now).expect("打开会话"),
    ));
    let after: host::OpenProjectAfter = Rc::new(|_ui, _project, _path, _view| Ok(()));
    let titled: Rc<dyn host::OpenProjectRuntime> = Rc::new(host::PortOpenProject {
        port: Rc::clone(&undo),
        production: None,
        quanta: 0,
        after: Rc::clone(&after),
    });
    let untitled_port: Rc<host::PortOpenProject> = Rc::new(host::PortOpenProject {
        port: Rc::clone(&undo),
        production: None,
        quanta: 0,
        after,
    });
    host::wire_startup_chooser(
        &ui,
        Rc::new(yeban_app::file_picker::NoPicker),
        titled,
        untitled_port,
        roots,
        NewProjectDetails::from_project(&project),
    );
    ui.set_startup_chooser_open(true);
    ui.show().expect("show 主窗口");

    // 热身帧（布局与首帧构建不计入见证）。
    let _ = render_frame(&window, width, height);

    let shot = |name: &str| {
        let (frame, elapsed) =
            render_frame(&window, width, height).unwrap_or_else(|| panic!("{name} 帧必须重绘"));
        write_png(
            &format!("{out_dir}/chooser-{name}.png"),
            &frame,
            width,
            height,
        );
        let non_black = non_black_pixels(&frame);
        println!(
            "chooser-shot frame={name} render_ms={:.2} non_black={non_black} path={out_dir}/chooser-{name}.png",
            elapsed.as_secs_f64() * 1000.0,
        );
        assert!(
            non_black > 200_000,
            "{name} 帧非黑像素过少（{non_black}），疑似空画布"
        );
    };

    shot("new");
    ui.set_startup_details_open(true);
    shot("new-details");
    ui.set_startup_details_open(false);
    ui.invoke_startup_category_selected(1);
    shot("recent");
    ui.invoke_startup_category_selected(2);
    shot("demos");
    ui.invoke_startup_category_selected(3);
    shot("factory");
    ui.invoke_startup_category_selected(4);
    shot("user");
    println!("chooser-shot OK");
}

// ---------------------------------------------------------------------------
// 最小 PNG 写出（存储式 deflate；与 `examples/i18n_shots.rs` 同款）
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
