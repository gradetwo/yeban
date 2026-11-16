//! 质感切换（`--texture`）的判据（`line/ui-skeuo-theme` 一期，2026-11-16）。
//!
//! 与 `tests/theme_selection.rs` 同款三层耦合，各自能独立变红：
//!
//! | # | 判据 | 观测面 |
//! | :-- | :--- | :--- |
//! | ① | CLI 默认 = `flat`；`flat` / `physical` 被接受；未知取值被**点名拒绝**（`UnknownTexture`），⛔ 不静默回退 | `cli::parse` 的返回值 |
//! | ② | **初值耦合**：不调 `apply_texture` 时，活窗口的 `Tokens.texture` 初值 == `TextureMode::Flat` == `cli::Texture::default()`（截图 / golden 路径不调 apply，看到的就是初值 —— 默认外观一位不动靠这一条） | 真 `MainWindow` 的全局回读 + `tokens.slint` 源码钉 |
//! | ③ | **生效判定**：`physical` + 深色自绘配色（`yeban` / `brand`）⇒ `Tokens.texture-physical == true`；`physical` + 其余配色 ⇒ `false`；`flat` 恒 `false` | 真 `MainWindow` 的 token 回读 |
//! | ④ | **flat 不变式**：flat 时每个面板 brush 的渐变停点色 == 它对应的扁平纯色（brush 令牌「同色双停」的机械形态） | 真 `MainWindow` 的 brush 回读 |
//!
//! 活窗口判据只放在**一个**测试里：软件平台每个线程只能装一次
//! （与 `theme_selection.rs` 的 `ThemePlatform` 同款纪律）。

use yeban_app::cli::{self, ParseError, Texture, Theme};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::{TextureMode, Tokens};

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}

// ---------------------------------------------------------------------------
// 判据 ①: CLI 层（不碰 Slint）
// ---------------------------------------------------------------------------

#[test]
fn the_cli_default_texture_is_flat_and_values_are_checked() {
    // 缺省 = flat（默认外观一位不动的 CLI 侧钉）。
    let options = cli::parse(&[]).expect("空参数应可解析");
    assert_eq!(options.texture, Texture::Flat);
    assert_eq!(Texture::default(), Texture::Flat);

    // 两个合法取值都被接受（分离式与 `--texture=<v>` 内联式各一）。
    let options = cli::parse(&args(&["--texture", "physical"])).expect("physical 应被接受");
    assert_eq!(options.texture, Texture::Physical);
    let options = cli::parse(&args(&["--texture=flat"])).expect("flat 应被接受");
    assert_eq!(options.texture, Texture::Flat);

    // 重复给以最后一个为准（与 `--theme` 同款）。
    let options =
        cli::parse(&args(&["--texture", "physical", "--texture", "flat"])).expect("重复给应可解析");
    assert_eq!(options.texture, Texture::Flat);

    // 未知取值 ⇒ 点名拒绝，⛔ 不静默回退到 flat。
    let error = cli::parse(&args(&["--texture", "glossy"])).expect_err("未知质感必须被拒绝");
    assert_eq!(error, ParseError::UnknownTexture("glossy".to_owned()));
    let message = error.to_string();
    assert!(
        message.contains("flat") && message.contains("physical"),
        "错误信息必须列出可用集合, 实得: {message}"
    );

    // 取值集合的唯一推导（用法 / 错误 / 本判据共用）。
    assert_eq!(Texture::accepted_names(), vec!["flat", "physical"]);
    assert_eq!(Texture::from_name("flat"), Some(Texture::Flat));
    assert_eq!(Texture::from_name("physical"), Some(Texture::Physical));
    assert_eq!(Texture::from_name("glossy"), None);

    // 用法文本必须列出这个开关（藏起来等于没有）。
    assert!(
        cli::usage_text().contains("--texture"),
        "用法文本必须列出 `--texture`"
    );
}

// ---------------------------------------------------------------------------
// 判据 ②④（源码钉部分）: Slint 初值与令牌层的形状
// ---------------------------------------------------------------------------

#[test]
fn the_slint_initial_texture_is_flat_in_the_source() {
    let tokens = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/tokens.slint"),
    )
    .expect("读 tokens.slint");
    assert!(
        tokens.contains("in-out property <TextureMode> texture: TextureMode.flat;"),
        "Tokens.texture 的 Slint 初值必须是 flat（截图路径不调 apply_texture）"
    );
    assert!(
        tokens.contains("texture-physical"),
        "生效判定必须收在 Tokens.texture-physical 一格里"
    );
}

// ---------------------------------------------------------------------------
// 判据（源码钉）: 活面板的质感落点 —— 下层 dock，而非 console/*
//
// 背景：最大化窗口只渲染 `UnifiedLowerDock`；`console/mixer_console.slint`
// 与 `console/device_rack.slint` 只挂在 `app.slint` 里被 `if false` 禁用
// 的 `ConsoleTabs` 上，真实界面根本不走它们。只迁移 console/*，主窗口
// 的 flat/physical 渲染逐像素相同（实测 0 差异像素）。一期试点因此必须
// 同时覆盖 dock 的混音条带与设备卡片；本钉锁住这些落点与 flat 分支的
// 旧字面值（flat 逐像素不变是靠三元的假分支，不是靠令牌的 flat 落点 ——
// dock 的旧色与令牌角色色并不相等，见各断言里的字面值）。
// ---------------------------------------------------------------------------

#[test]
fn the_live_dock_panels_bind_texture_tokens_with_flat_fallback() {
    let dock = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/lower_dock.slint"),
    )
    .expect("读 lower_dock.slint");
    for (needle, what) in [
        (
            "Tokens.texture-physical ? Tokens.panel-brush : #080a10",
            "通道条/总线条背景 (panel-brush, flat #080a10)",
        ),
        (
            "Tokens.texture-physical ? Tokens.shell-brush : Theme.bg-surface",
            "混音区外容器背景 (shell-brush, flat Theme.bg-surface)",
        ),
        (
            "Tokens.texture-physical ? Tokens.void-brush : #06080d",
            "推子导轨背景 (void-brush, flat #06080d)",
        ),
        (
            "Tokens.texture-physical ? Tokens.raised-brush : #94a3b8",
            "推子帽背景 (raised-brush, flat #94a3b8)",
        ),
        (
            "Tokens.texture-physical ? Tokens.glass-brush : #06080d",
            "电平槽/设备屏背景 (glass-brush, flat #06080d)",
        ),
        (
            "Tokens.texture-physical ? (root.device-bypassed[idx] ? Tokens.void-brush : Tokens.panel-brush) : Theme.bg-surface",
            "设备卡片背景 (旁通 void-brush, 常态 panel-brush)",
        ),
        (
            "Tokens.texture-physical ? Tokens.raised-brush : #cbd5e1",
            "主推子帽背景 (raised-brush, flat #cbd5e1)",
        ),
    ] {
        assert!(
            dock.contains(needle),
            "lower_dock.slint 必须含 {what}: {needle}"
        );
    }
    assert!(
        dock.contains("Tokens.glass-reflection"),
        "设备屏与通道条 EQ 预览屏必须有玻璃斜向反光"
    );
    assert!(
        dock.contains("Tokens.bevel-light") && dock.contains("Tokens.bevel-dark"),
        "推子帽/M/S/声像必须有斜面亮/暗边"
    );
}

// ---------------------------------------------------------------------------
// 判据 ②③④: 活窗口回读（本文件唯一碰 Slint 的测试）
// ---------------------------------------------------------------------------

/// 与 `theme_selection.rs` 的 `ThemePlatform` 同款软件平台。
struct TexturePlatform {
    window: std::rc::Rc<slint::platform::software_renderer::MinimalSoftwareWindow>,
}

impl slint::platform::Platform for TexturePlatform {
    fn create_window_adapter(
        &self,
    ) -> Result<std::rc::Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
}

#[test]
fn the_effective_texture_follows_the_selection() {
    use slint::ComponentHandle as _;
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(slint::PhysicalSize::new(1600, 900));
    slint::platform::set_platform(Box::new(TexturePlatform { window }))
        .expect("本线程还没有平台 (本文件只有这一个测试碰 Slint)");

    let project = yeban_model::samples::default_project();
    let view = cli::project_view(&project).expect("投影空工程");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window(&view, &scene).expect("构造主窗口");

    // ② 初值耦合: 还没调任何 apply —— 回读到的必须是 flat / 未生效。
    assert_eq!(
        ui.global::<Tokens<'_>>().get_texture(),
        TextureMode::Flat,
        "Tokens.texture 初值必须是 flat"
    );
    assert!(
        !ui.global::<Tokens<'_>>().get_texture_physical(),
        "初值下质感不得生效"
    );

    // ④ flat 不变式: flat 时 panel-brush 的每个渐变停点 == bg-panel 纯色。
    {
        let tokens = ui.global::<Tokens<'_>>();
        let panel = tokens.get_bg_panel();
        match tokens.get_panel_brush() {
            slint::Brush::LinearGradient(gradient) => {
                assert!(
                    gradient.stops().count() >= 2,
                    "panel-brush 必须是渐变刷 (flat 时为同色双停)"
                );
                for stop in gradient.stops() {
                    assert_eq!(
                        stop.color, panel,
                        "flat 时 panel-brush 停点必须 == bg-panel"
                    );
                }
            }
            other => panic!("panel-brush 应是线性渐变, 实得 {other:?}"),
        }
    }

    // ③ 生效判定: physical + yeban ⇒ 生效, 且 brush 真的变了 (不是假开关)。
    host::apply_theme(&ui, Theme::Yeban);
    host::apply_texture(&ui, Texture::Physical);
    assert!(
        ui.global::<Tokens<'_>>().get_texture_physical(),
        "physical + yeban 必须生效"
    );
    {
        let tokens = ui.global::<Tokens<'_>>();
        let panel = tokens.get_bg_panel();
        match tokens.get_panel_brush() {
            slint::Brush::LinearGradient(gradient) => {
                assert!(
                    gradient.stops().any(|stop| stop.color != panel),
                    "physical 时 panel-brush 必须真的带渐变 (至少一停点 != bg-panel)"
                );
            }
            other => panic!("panel-brush 应是线性渐变, 实得 {other:?}"),
        }
    }

    // ③ physical + brand ⇒ 生效。
    host::apply_theme(&ui, Theme::Brand);
    assert!(
        ui.global::<Tokens<'_>>().get_texture_physical(),
        "physical + brand 必须生效"
    );

    // ③ physical + inkmoor ⇒ 回落 flat (材质只为深色自绘两支标定)。
    host::apply_theme(&ui, Theme::InkMoor);
    assert!(
        !ui.global::<Tokens<'_>>().get_texture_physical(),
        "physical + inkmoor 必须回落 flat"
    );

    // ③ flat 恒不生效 (哪怕配色回到 yeban)。
    host::apply_theme(&ui, Theme::Yeban);
    host::apply_texture(&ui, Texture::Flat);
    assert!(
        !ui.global::<Tokens<'_>>().get_texture_physical(),
        "flat 时质感恒不生效"
    );
}
