//! 实体化试点对比图（`line/ui-skeuo-theme` 一期证据工具，**不是**产品路径）。
//!
//! 用法：
//! `cargo run -p yeban-app --example skeuo_pilot_shots -- <mixer|devices|showcase> [输出目录]`
//!
//! 每个场景产出两张 PNG（`flat` / `physical`），同一份**真实工程**
//! （`yeban_model::samples::filled_project()`：3 轨 + 主总线，带设备 / 音量 /
//! 声相 / 发送的真投影），同一支 yeban 深色配色，只切质感一维。
//! `showcase` 是控件陈列场景（旋钮等试点面板里没有的控件在那里验渲染）。
//!
//! 指标口径（性能注记的数据来源）：整帧 SoftwareRenderer 光栅化耗时，
//! 单位毫秒；每种质感取「切换后第一帧」的实测（布局已由首帧热身，
//! 不把构建 / 布局计入质感开销）。stdout 同时打印非黑像素数作非空见证。

use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::{ComponentHandle as _, PhysicalSize, Rgb8Pixel};
use yeban_app::cli::{self, Texture, Theme};
use yeban_app::host;
use yeban_app::scene::DemoScene;

// 陈列场景（旋钮 / 按键 / 开关的 physical 渲染验证; 试点两面板里没有旋钮，
// 见 controls/knob.slint 的文件头）。路径相对本文件（examples/ 的上一级）。
slint::slint! {
    // ⚠ `Tokens` / `TextureMode` 必须从这个内联文件**再导出**才属于它的公开
    // API、Rust 侧才有对应类型 (只 `import` 会在 Rust 侧报 E0425 /
    // deprecated type alias —— CI run 43864308074 rust 腿实证)。
    export { Tokens, TextureMode } from "../ui/tokens.slint";
    import { UiText } from "../ui/tokens.slint";
    import { Knob } from "../ui/controls/knob.slint";
    import { Fader, HFader } from "../ui/controls/fader.slint";
    import { LedMeter } from "../ui/controls/led_meter.slint";
    import { PushButton } from "../ui/controls/push_button.slint";
    import { ToggleSwitch } from "../ui/controls/toggle_switch.slint";

    export component SkeuoShowcase inherits Window {
        width: 820px;
        height: 380px;
        background: Tokens.bg-void;

        // 质感开关由 Rust 侧经 `Tokens.texture` 全局写 (与应用侧
        // `host::apply_texture` 同路)。注意: 全局属性不能在组件内绑定
        // (`Tokens.texture: ...` 是 Slint 解析错误 —— 盒上 clippy 实证)。

        Rectangle {
            x: 16px;
            y: 16px;
            width: parent.width - 32px;
            height: parent.height - 32px;
            background: Tokens.panel-brush;
            border-radius: Tokens.radius-md;

            Knob {
                x: 24px;
                y: 24px;
                width: 96px;
                height: 96px;
                value: 0.25;
                indicator-color: Tokens.gold;
            }

            Knob {
                x: 144px;
                y: 24px;
                width: 96px;
                height: 96px;
                value: 0.62;
                indicator-color: Tokens.accent;
            }

            Knob {
                x: 264px;
                y: 24px;
                width: 96px;
                height: 96px;
                value: 0.9;
                indicator-color: Tokens.ok-green;
            }

            UiText {
                x: 24px;
                y: 126px;
                width: 96px;
                height: 14px;
                text: "截止 CUTOFF";
                font-size: Tokens.font-size-xs;
                color: Tokens.ink-2;
                horizontal-alignment: center;
            }

            UiText {
                x: 144px;
                y: 126px;
                width: 96px;
                height: 14px;
                text: "共振 RESO";
                font-size: Tokens.font-size-xs;
                color: Tokens.ink-2;
                horizontal-alignment: center;
            }

            UiText {
                x: 264px;
                y: 126px;
                width: 96px;
                height: 14px;
                text: "驱动 DRIVE";
                font-size: Tokens.font-size-xs;
                color: Tokens.ink-2;
                horizontal-alignment: center;
            }

            Fader {
                x: 404px;
                y: 24px;
                width: 48px;
                height: 116px;
                fraction: 0.68;
            }

            LedMeter {
                x: 478px;
                y: 30px;
                width: 14px;
                height: 104px;
                level: 0.72;
            }

            PushButton {
                x: 528px;
                y: 24px;
                width: 44px;
                height: 28px;
                text: "M";
                checked: true;
                checked-bg: Tokens.record-red;
            }

            PushButton {
                x: 584px;
                y: 24px;
                width: 44px;
                height: 28px;
                text: "S";
                checked: false;
                checked-bg: Tokens.gold-deep;
            }

            ToggleSwitch {
                x: 528px;
                y: 68px;
                width: 100px;
                height: 24px;
                checked: true;
            }

            ToggleSwitch {
                x: 528px;
                y: 108px;
                width: 100px;
                height: 24px;
                checked: false;
            }

            HFader {
                x: 24px;
                y: 168px;
                width: 300px;
                height: 24px;
                fraction: 0.35;
                rail-y: 10px;
                thumb-y: 4px;
                node-height: 24px;
                show-zero-tick: true;
            }

            Rectangle {
                x: 404px;
                y: 168px;
                width: 300px;
                height: 100px;
                background: Tokens.glass-brush;
                border-radius: Tokens.radius-md;
                clip: true;

                Path {
                    x: 0;
                    y: 0;
                    width: parent.width;
                    height: parent.height;
                    viewbox-x: 0;
                    viewbox-y: 0;
                    viewbox-width: 300;
                    viewbox-height: 100;
                    fill: Tokens.glass-reflection;
                    stroke-width: 0px;
                    MoveTo {
                        x: 14;
                        y: 0;
                    }

                    LineTo {
                        x: 108;
                        y: 0;
                    }

                    LineTo {
                        x: 70;
                        y: 100;
                    }

                    LineTo {
                        x: -24;
                        y: 100;
                    }

                    LineTo {
                        x: 14;
                        y: 0;
                    }
                }

                UiText {
                    x: 12px;
                    y: 10px;
                    width: 200px;
                    height: 14px;
                    text: "玻璃屏 GLASS";
                    font-size: Tokens.font-size-xs;
                    color: Tokens.ink-2;
                }
            }
        }
    }
}

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

/// 光栅化一帧并计时。返回 `None` = 窗口未要求重绘（帧与上一帧逐像素相同）。
fn render_frame(
    window: &MinimalSoftwareWindow,
    width: u32,
    height: u32,
) -> Option<(Vec<Rgb8Pixel>, Duration)> {
    let mut buffer = vec![Rgb8Pixel::new(0, 0, 0); (width * height) as usize];
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
    let mut arguments = std::env::args().skip(1);
    let scene_name = arguments.next().unwrap_or_else(|| "mixer".to_owned());
    let out_dir = arguments.next().unwrap_or_else(|| {
        let home = std::env::var("HOME").expect("HOME 必须存在");
        format!("{home}/workspace/skeuo-pilot-shots")
    });
    std::fs::create_dir_all(&out_dir).expect("创建输出目录");

    match scene_name.as_str() {
        "mixer" | "devices" => shoot_console(&scene_name, &out_dir),
        "showcase" => shoot_showcase(&out_dir),
        other => {
            eprintln!("未知场景 `{other}` (可用: mixer | devices | showcase)");
            std::process::exit(2);
        }
    }
}

/// 控制台两面板：真实工程 + 真投影，控制台最大化 ⇒ 整帧就是面板本体。
fn shoot_console(scene_name: &str, out_dir: &str) {
    let (width, height) = (1600u32, 900u32);
    let window = install_platform(width, height);

    let project = yeban_model::samples::filled_project();
    let view = cli::project_view(&project).expect("投影 filled 工程");
    let scene = DemoScene::from_view(&view);
    let console_tab = if scene_name == "mixer" { 1 } else { 2 };
    let ui =
        host::build_main_window_with_console_tab(&view, &scene, console_tab).expect("构造主窗口");
    host::apply_theme(&ui, Theme::Yeban);
    ui.set_console_maximized(true);
    ui.show().expect("show 主窗口");

    // 热身帧（布局 + 首帧构建不计入质感开销），随后两模式各测一帧。
    let _ = render_frame(&window, width, height);
    for (texture, label) in [(Texture::Physical, "physical"), (Texture::Flat, "flat")] {
        host::apply_texture(&ui, texture);
        let (frame, elapsed) =
            render_frame(&window, width, height).expect("切换质感后必须重绘一帧");
        let path = format!("{out_dir}/{scene_name}-{label}.png");
        write_png(&path, &frame, width, height);
        println!(
            "skeuo-shot scene={scene_name} texture={label} render_ms={:.2} non_black={} path={path}",
            elapsed.as_secs_f64() * 1000.0,
            non_black_pixels(&frame),
        );
    }
}

/// 控件陈列：同一场景两模式各一帧（顺序同上，先 physical 后 flat，
/// 两帧都是切换后的真渲染）。
fn shoot_showcase(out_dir: &str) {
    let (width, height) = (820u32, 380u32);
    let window = install_platform(width, height);
    let ui = SkeuoShowcase::new().expect("构造陈列场景");
    ui.show().expect("show 陈列场景");
    let _ = render_frame(&window, width, height);
    for (mode, label) in [
        (TextureMode::Physical, "physical"),
        (TextureMode::Flat, "flat"),
    ] {
        ui.global::<Tokens<'_>>().set_texture(mode);
        let (frame, elapsed) =
            render_frame(&window, width, height).expect("切换质感后必须重绘一帧");
        let path = format!("{out_dir}/showcase-{label}.png");
        write_png(&path, &frame, width, height);
        println!(
            "skeuo-shot scene=showcase texture={label} render_ms={:.2} non_black={} path={path}",
            elapsed.as_secs_f64() * 1000.0,
            non_black_pixels(&frame),
        );
    }
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
