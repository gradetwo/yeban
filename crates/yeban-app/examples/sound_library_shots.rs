//! 音源管理证据图（`line/sound-library-manager` 的取证工具，**不是**产品路径）。
//!
//! 用法：
//! `cargo run -p yeban-app --example sound_library_shots -- [输出目录]`
//! （缺省输出到 `~/workspace/sound-manager-shots`）。
//!
//! 走**真实接线**：`host::build_main_window` + `host::wire_sound_library_dialog`
//! + `invoke_open_sound_library()`（= 主菜单「设置 → 音源管理...」的宿主
//!   落点），不绕过任何一层。夹具只有两件，且都走产品自己的注入口：
//! 1. 用户音源库根经 `YEBAN_SAMPLE_LIBRARY` 指到临时目录，里面放一个带
//!    `.yeban-pack.json` 标记的夹具包（已安装态的数据来源）；
//! 2. 下载目录经 `YEBAN_SOUND_CATALOG_FILE` 指到本地文件（产品的判据 /
//!    离线注入口），先后给「空目录」与「一个包的目录」两份。
//!
//! 两帧：
//! 1. `manager-installed.png` —— 空目录：已安装列出内置库 + 夹具包，
//!    可下载区如实显示「暂无可下载音源」；
//! 2. `manager-downloadable.png` —— 目录里有一个未安装的包：包行带
//!    版本、大小与「下载」按钮。
//!
//! 排空方式：本工具的软件平台没有事件循环代理，工作线程的
//! `upgrade_in_event_loop` 会返回 Err、结局留在槽里（与 Tier-1 平台
//! 同一口径），故这里直接调 `sound-library-drain-requested` 排空；
//! 等待用有界轮询 —— 这是人工取证工具，不是 CI 判据。

use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::{ComponentHandle as _, Model as _, PhysicalSize, Rgb8Pixel};
use yeban_app::cli::{self, Language, Theme};
use yeban_app::host;
use yeban_app::scene::DemoScene;

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

/// 有界等待目录结局落槽并排空（见文件头：人工取证工具，非判据）。
fn wait_for_catalog(ui: &yeban_app::ui::MainWindow) {
    for _ in 0..200 {
        ui.invoke_sound_library_drain_requested();
        if ui.get_sound_catalog_state() != 0 {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("等待目录排空超时（200 × 25ms）");
}

fn main() {
    let out_dir = std::env::args().nth(1).unwrap_or_else(|| {
        let home = std::env::var("HOME").expect("HOME 必须存在");
        format!("{home}/workspace/sound-manager-shots")
    });
    std::fs::create_dir_all(&out_dir).expect("创建输出目录");

    // ---- 夹具：临时用户音源库根 + 一个已装夹具包 + 两份目录文件 --------
    let fixture_root = std::env::temp_dir().join("yeban-sound-shots-fixture");
    let _ = std::fs::remove_dir_all(&fixture_root);
    let library_root = fixture_root.join("library");
    let pack_dir = library_root.join("fixture-piano");
    std::fs::create_dir_all(&pack_dir).expect("创建夹具包目录");
    std::fs::write(
        pack_dir.join(yeban_app::sound_library::PACK_MARKER_FILE),
        r#"{"name":"夹具钢琴","version":"1.0.0"}"#,
    )
    .expect("写夹具包标记");
    std::fs::write(
        pack_dir.join("fixture-piano.sfz"),
        "<region> sample=keys/*.wav\n",
    )
    .expect("写夹具 SFZ");
    // 一块有体积的负载，让「占用大小」列显示非零读数。
    std::fs::write(pack_dir.join("payload.bin"), vec![7u8; 12_345]).expect("写夹具负载");

    let empty_catalog = fixture_root.join("catalog-empty.json");
    std::fs::write(&empty_catalog, r#"{"version":1,"packs":[]}"#).expect("写空目录夹具");
    let one_pack_catalog = fixture_root.join("catalog-one.json");
    std::fs::write(
        &one_pack_catalog,
        r#"{"version":1,"packs":[{"id":"fixture-strings","name":"夹具弦乐","description":"取证用夹具包，不随产品分发","version":"0.1.0","sizeBytes":2048,"sha256":"0000000000000000000000000000000000000000000000000000000000000000","url":"https://example.invalid/fixture-strings.zip"}]}"#,
    )
    .expect("写单包目录夹具");

    // SAFETY: `set_var` 在 Rust 2024 下是 `unsafe`；这里是进程启动后主线程
    // 的第一件事，窗口与工作线程尚未创建。两帧之间再改目录文件变量时，
    // 上一轮的拉取线程已随排空完成而退出，无并发读。
    unsafe {
        std::env::set_var(
            yeban_app::settings::SETTINGS_PATH_ENV,
            format!("{out_dir}/.settings-test.json"),
        );
        std::env::set_var(
            "YEBAN_SAMPLE_LIBRARY",
            library_root.to_string_lossy().as_ref(),
        );
        std::env::set_var(
            yeban_app::sound_library::CATALOG_FILE_ENV,
            empty_catalog.to_string_lossy().as_ref(),
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
    // 与 main.rs 同一条接线（产品进程在装配点显式调用）。
    host::wire_sound_library_dialog(&ui);
    ui.show().expect("show 主窗口");
    let _ = render_frame(&window, width, height);

    let shot = |name: &str| {
        let (frame, elapsed) =
            render_frame(&window, width, height).unwrap_or_else(|| panic!("{name} 帧必须重绘"));
        write_png(&format!("{out_dir}/{name}.png"), &frame, width, height);
        println!(
            "sound-shot frame={name} catalog_state={} installed_rows={} pack_rows={} render_ms={:.2} non_black={} path={out_dir}/{name}.png",
            ui.get_sound_catalog_state(),
            ui.get_sound_installed_ids().row_count(),
            ui.get_sound_pack_ids().row_count(),
            elapsed.as_secs_f64() * 1000.0,
            non_black_pixels(&frame),
        );
    };

    // ---- 帧 1：空目录（已安装态 + 暂无可下载）------------------------------
    ui.invoke_open_sound_library();
    wait_for_catalog(&ui);
    assert_eq!(ui.get_sound_catalog_state(), 1, "空目录必须拉取成功");
    assert_eq!(ui.get_sound_pack_ids().row_count(), 0, "空目录零包行");
    assert!(
        ui.get_sound_installed_ids().row_count() >= 2,
        "已安装至少含内置库与夹具包"
    );
    shot("manager-installed");
    ui.invoke_close_sound_library();

    // ---- 帧 2：一包目录（可下载态）-----------------------------------------
    // SAFETY: 同上；上一轮拉取线程已退出。
    unsafe {
        std::env::set_var(
            yeban_app::sound_library::CATALOG_FILE_ENV,
            one_pack_catalog.to_string_lossy().as_ref(),
        );
    }
    ui.invoke_open_sound_library();
    wait_for_catalog(&ui);
    assert_eq!(ui.get_sound_catalog_state(), 1, "单包目录必须拉取成功");
    assert_eq!(ui.get_sound_pack_ids().row_count(), 1, "单包目录一行包");
    shot("manager-downloadable");
    ui.invoke_close_sound_library();

    println!("sound-shot OK");
}

// ---------------------------------------------------------------------------
// 最小 PNG 写出（存储式 deflate；与 examples/i18n_shots.rs 同款，无新依赖）
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
