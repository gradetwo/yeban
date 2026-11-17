//! 鼓机生成证据图（`line/drum-machine-p1` 的取证工具，**不是**产品路径）。
//!
//! 用法：
//! `cargo run -p yeban-app --example drum_gen_shots -- [输出目录]`
//! （缺省输出到 `~/workspace/drum-gen-shots`）。
//!
//! 两帧：
//! 1. `drum-gen-dialog.png` —— 走**真实接线**：`host::build_main_window` +
//!    `host::wire_drum_gen_dialog` + `invoke_open_drum_gen()`（= 主菜单
//!    「编辑 → 鼓机生成 (Drums)...」的宿主落点），对话框带着 10 行出厂
//!    风格与播种好的五格缺省打开。同一窗口上继续按产品顺序驱动
//!    `invoke_drum_gen_style_picked(0)` 与 `invoke_apply_drum_gen()`：
//!    本工具没有撤销会话，apply 必须停在「没有撤销会话」错误行 ——
//!    证明点击链真的到达宿主提交口（提交本身的落盘由 bridge 判据与
//!    MCP 端到端判据覆盖）。
//! 2. `drum-gen-result.png` —— 把宿主将提交的同一批 `Op`
//!    （`bridge::drum_gen_ops`，新轨路径）经模型自己的 `Op::apply`
//!    落到夹具工程上，再走产品投影（`cli::project_view` +
//!    `host::build_main_window`）渲染：编曲区出现「Trap Drums」轨与
//!    4 小节片段。
//!
//! 点击计数口径（验收「新建工程起 ≤5 次点击、60 秒出 loop」）：
//! ① 点「编辑」菜单 ② 点「鼓机生成 (Drums)...」③ 点风格行
//! ④ 点「生成」—— 共 4 次点击；生成是本地纯计算（毫秒级），片段
//! 落轨即可播放，远在 60 秒内。本工具驱动的正是 ②③④ 三个回调。

use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::{ComponentHandle as _, Model as _, PhysicalSize, Rgb8Pixel};
use yeban_app::bridge;
use yeban_app::cli::{self, Language, Theme};
use yeban_app::host;
use yeban_app::scene::DemoScene;

/// 与 `examples/sound_library_shots.rs` 同款的自研软件平台。
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

fn main() {
    let out_dir = std::env::args().nth(1).unwrap_or_else(|| {
        let home = std::env::var("HOME").expect("HOME 必须存在");
        format!("{home}/workspace/drum-gen-shots")
    });
    std::fs::create_dir_all(&out_dir).expect("创建输出目录");

    // SAFETY: `set_var` 在 Rust 2024 下是 `unsafe`；进程启动后主线程第一件事。
    unsafe {
        std::env::set_var(
            yeban_app::settings::SETTINGS_PATH_ENV,
            format!("{out_dir}/.settings-test.json"),
        );
    }

    let (width, height) = (1600u32, 900u32);
    let window = install_platform(width, height);

    // ---- 帧 1：对话框（真实接线）------------------------------------------
    let project = yeban_model::samples::filled_project();
    let view = cli::project_view(&project).expect("投影 filled 工程");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window(&view, &scene).expect("构造主窗口");
    host::apply_theme(&ui, Theme::Yeban);
    host::apply_language(&ui, Language::ZhCn);
    host::wire_drum_gen_dialog(&ui, None);
    ui.show().expect("show 主窗口");
    let _ = render_frame(&window, width, height);

    ui.invoke_open_drum_gen();
    assert!(ui.get_drum_gen_open(), "对话框必须打开");
    assert_eq!(ui.get_drum_gen_rows().row_count(), 10, "出厂风格必须 10 行");
    assert_eq!(ui.get_drum_gen_complexity().as_str(), "50");
    assert_eq!(ui.get_drum_gen_intensity().as_str(), "80");
    assert_eq!(ui.get_drum_gen_bars().as_str(), "4");
    // 选风格（点击 ③ 的回调）：摇摆重播为 trap 定值 515。
    ui.invoke_drum_gen_style_picked(0);
    assert_eq!(
        ui.get_drum_gen_swing().as_str(),
        "515",
        "选 trap 后摇摆播种"
    );

    let (frame, elapsed) =
        render_frame(&window, width, height).expect("drum-gen-dialog 帧必须重绘");
    write_png(
        &format!("{out_dir}/drum-gen-dialog.png"),
        &frame,
        width,
        height,
    );
    println!(
        "drum-shot frame=dialog rows={} target={:?} render_ms={:.2} non_black={} path={out_dir}/drum-gen-dialog.png",
        ui.get_drum_gen_rows().row_count(),
        ui.get_drum_gen_target_label().as_str(),
        elapsed.as_secs_f64() * 1000.0,
        non_black_pixels(&frame),
    );

    // 点击 ④ 的回调：无撤销会话 ⇒ 必须停在错误行、弹窗不关、不提交。
    ui.invoke_apply_drum_gen();
    assert!(ui.get_drum_gen_open(), "失败时弹窗不关");
    assert!(
        ui.get_drum_gen_error().as_str().contains("没有撤销会话"),
        "apply 必须到达提交口并报无会话: {}",
        ui.get_drum_gen_error()
    );
    ui.invoke_close_drum_gen();
    assert!(!ui.get_drum_gen_open(), "关闭后 open 必须为假");
    drop(ui);

    // ---- 帧 2：生成结果（同一批 Op 落到工程后的产品投影）------------------
    let mut project = yeban_model::samples::filled_project();
    let preset = yeban_theory::factory_drum_style("trap")
        .expect("预置解析")
        .expect("trap 预置必须存在");
    let params = yeban_theory::DrumGenParams {
        complexity: 50,
        intensity: 80,
        swing_permille: Some(515),
        seed: project.rng_seed,
        bars: 4,
        fill: true,
    };
    let ops = bridge::drum_gen_ops(&project, &preset, &params, None).expect("构造生成批次");
    for op in &ops {
        op.apply(&mut project).expect("生成批次必须能落到工程");
    }
    let drum_track = project
        .tracks
        .values()
        .find(|track| track.name == "Trap Drums")
        .expect("生成后必须有 Trap Drums 轨");
    assert!(
        drum_track
            .devices
            .iter()
            .any(|device| device.params.iter().any(|p| p.name == "kick_note")),
        "新轨必须装好合成鼓机"
    );
    let note_count: usize = project
        .clip_pool
        .values()
        .map(|clip| match &clip.content {
            yeban_model::ClipContent::Midi { notes } => notes.len(),
            _ => 0,
        })
        .sum();
    assert!(note_count > 0, "生成片段必须有音符");

    let view = cli::project_view(&project).expect("投影生成后工程");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window(&view, &scene).expect("构造结果主窗口");
    host::apply_theme(&ui, Theme::Yeban);
    host::apply_language(&ui, Language::ZhCn);
    ui.show().expect("show 结果主窗口");
    let (frame, elapsed) =
        render_frame(&window, width, height).expect("drum-gen-result 帧必须重绘");
    write_png(
        &format!("{out_dir}/drum-gen-result.png"),
        &frame,
        width,
        height,
    );
    println!(
        "drum-shot frame=result notes={} render_ms={:.2} non_black={} path={out_dir}/drum-gen-result.png",
        note_count,
        elapsed.as_secs_f64() * 1000.0,
        non_black_pixels(&frame),
    );

    println!("drum-shot OK");
}

// ---------------------------------------------------------------------------
// 最小 PNG 写出（存储式 deflate；与 examples/sound_library_shots.rs 同款）
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
