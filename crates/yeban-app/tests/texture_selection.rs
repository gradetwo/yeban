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
// 判据（源码钉）: 二期五面（2026-11-17）
//
// ① dock 钢琴卷帘页、② 左栏检查器 + 智能音色库、③ 走带栏本体、
// ④ 编曲区轨道头/片段、⑤ 状态栏。口径同一期：使用点三元的假分支必须是
// 现行字面值/现行角色色（flat 逐像素不变靠三元假分支或令牌 flat 分支）；
// §12 专用材质（轨道头/片段/卷帘键面与车道）的 flat 分支必须是使用点
// 现行字面值的同色双停渐变。
// ---------------------------------------------------------------------------

#[test]
fn the_phase2_surfaces_bind_texture_tokens_with_flat_fallback() {
    let ui_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let read = |rel: &str| {
        std::fs::read_to_string(ui_dir.join(rel)).unwrap_or_else(|_| panic!("读 {rel}"))
    };
    let tokens = read("tokens.slint");
    // §12 专用材质令牌: flat 分支 = 现行字面值同色双停。
    for (needle, what) in [
        (
            "out property <brush> track-head-brush: texture-physical",
            "轨道头刷令牌（§12）",
        ),
        (
            ": @linear-gradient(180deg, #0b0e16, #0b0e16);",
            "轨道头刷 flat 分支 = 现行字面 #0b0e16",
        ),
        (
            "out property <color> clip-sheen: #ffffff.with-alpha(texture-physical ? 0.07 : 0.0);",
            "片段顶光令牌（flat alpha = 0）",
        ),
        (
            "out property <color> clip-shade: #000000.with-alpha(texture-physical ? 0.35 : 0.0);",
            "片段底影令牌（flat alpha = 0）",
        ),
        (
            ": @linear-gradient(180deg, #e4e9f2, #e4e9f2);",
            "卷帘白键刷 flat 分支 = 现行字面 #e4e9f2",
        ),
        (
            ": @linear-gradient(180deg, #191e2b, #191e2b);",
            "卷帘黑键行刷 flat 分支 = 现行字面 #191e2b",
        ),
        (
            ": @linear-gradient(180deg, #0b0e15, #0b0e15);",
            "卷帘黑键帽刷 flat 分支 = 现行字面 #0b0e15",
        ),
        (
            ": @linear-gradient(180deg, #242c3e, #242c3e);",
            "卷帘白键车道刷 flat 分支 = 现行字面 #242c3e",
        ),
        (
            ": @linear-gradient(180deg, #161b26, #161b26);",
            "卷帘黑键车道刷 flat 分支 = 现行字面 #161b26",
        ),
        (
            ": @linear-gradient(180deg, #2c364b, #2c364b);",
            "卷帘根音车道刷 flat 分支 = 现行字面 #2c364b",
        ),
        (
            ": @linear-gradient(180deg, #1a202e, #1a202e);",
            "卷帘网格刷 flat 分支 = 现行字面 #1a202e",
        ),
    ] {
        assert!(
            tokens.contains(needle),
            "tokens.slint 必须含 {what}: {needle}"
        );
    }

    // ① 钢琴卷帘页。使用点一律三元: flat 走字面色支 (渐变刷作
    // background 时软件渲染器的圆角 AA 与纯色填充不一致, 盒上实证)。
    let roll = read("console/piano_roll.slint");
    for (needle, what) in [
        (
            "Tokens.texture-physical ? Tokens.void-brush : Tokens.bg-void",
            "卷帘根底走 void-brush (flat 色值支)",
        ),
        (
            "Tokens.texture-physical ? Tokens.shell-brush : #0d1018",
            "键列容器 (shell-brush, flat #0d1018)",
        ),
        ("Tokens.roll-key-white-brush", "白键面走 §12 键刷"),
        ("Tokens.roll-key-black-brush", "黑键行走 §12 键刷"),
        ("Tokens.roll-key-cap-brush", "黑键帽走 §12 键刷"),
        ("Tokens.roll-lane-white-brush", "白键车道走 §12 车道刷"),
        ("Tokens.roll-lane-black-brush", "黑键车道走 §12 车道刷"),
        ("Tokens.roll-lane-root-brush", "根音车道走 §12 车道刷"),
        (
            "Tokens.texture-physical ? Tokens.roll-grid-brush : #1a202e",
            "网格底走 §12 网格刷 (flat 字面支)",
        ),
        (
            "Tokens.texture-physical ? Tokens.panel-brush : Tokens.bg-panel",
            "力度泳道/右键菜单走 panel-brush (flat 色值支)",
        ),
    ] {
        assert!(
            roll.contains(needle),
            "piano_roll.slint 必须含 {what}: {needle}"
        );
    }
    for gone in [
        "background: Tokens.roll-grid-brush;",
        "background: Tokens.void-brush;",
    ] {
        assert!(
            !roll.contains(gone),
            "渐变刷不得直作 background (flat 圆角 AA 纪律): {gone}"
        );
    }

    // ② 左栏检查器 + 智能音色库。
    let left = read("left_panel.slint");
    for (needle, what) in [
        (
            "Tokens.texture-physical ? Tokens.panel-brush : Theme.bg-panel",
            "左栏根底 (panel-brush, flat Theme.bg-panel)",
        ),
        (
            "Tokens.texture-physical ? Tokens.shell-brush : #080a10",
            "子页签栏 (shell-brush, flat #080a10)",
        ),
        (
            "Tokens.texture-physical ? Tokens.panel-brush : Theme.bg-surface",
            "轨道头卡/通道条/总线条/音色条目卡 (panel-brush, flat Theme.bg-surface)",
        ),
        (
            "Tokens.texture-physical ? Tokens.glass-brush : #06080d",
            "EQ 屏/总线压缩屏 (glass-brush, flat #06080d)",
        ),
        (
            "Tokens.texture-physical ? Tokens.void-brush : #06080d",
            "推子导轨/表井/搜索框 (void-brush, flat #06080d)",
        ),
        (
            "Tokens.texture-physical ? Tokens.raised-brush : #94a3b8",
            "推子帽 (raised-brush, flat #94a3b8)",
        ),
        (
            "Tokens.texture-physical ? Tokens.glass-brush : #080c14",
            "dB 读数牌 (glass-brush, flat #080c14)",
        ),
    ] {
        assert!(
            left.contains(needle),
            "left_panel.slint 必须含 {what}: {needle}"
        );
    }
    assert!(
        left.contains("Tokens.glass-reflection"),
        "左栏两块屏必须有玻璃斜向反光"
    );
    assert!(
        !left.contains("background: #06080d;") && !left.contains("background: Theme.bg-surface;"),
        "左栏屏/井/卡的字面背景不得回归（必须走质感三元）"
    );

    // ③ 走带栏本体（Group L 质感切换钮的渲染与行为不在本轮范围, 钉住未动）。
    let transport = read("transport_bar.slint");
    for (needle, what) in [
        (
            "Tokens.texture-physical ? Tokens.panel-brush : Theme.header-bg",
            "走带栏体 (panel-brush, flat Theme.header-bg)",
        ),
        (
            "Tokens.texture-physical ? Tokens.shell-brush : #0a0d14",
            "走带组板 (shell-brush, flat #0a0d14)",
        ),
        (
            "Tokens.texture-physical ? Tokens.shell-brush : #090c12",
            "走带簇底板 (shell-brush, flat #090c12)",
        ),
        (
            "Tokens.texture-physical ? Tokens.glass-brush : #05070a",
            "LCD 框 (glass-brush, flat #05070a)",
        ),
        (
            "Tokens.texture-physical ? Tokens.raised-brush : #10141d",
            "命令钮 (raised-brush, flat #10141d)",
        ),
        (
            "background: Tokens.texture == TextureMode.physical ? Theme.accent.with-alpha(0.18) : #10141d;",
            "Group L 质感切换钮渲染未动",
        ),
    ] {
        assert!(
            transport.contains(needle),
            "transport_bar.slint 必须含 {what}: {needle}"
        );
    }
    assert!(
        transport.contains("Tokens.glass-reflection"),
        "走带 LCD 必须有玻璃斜向反光"
    );

    // ④ 编曲区轨道头/片段。
    let arrangement = read("arrangement.slint");
    for (needle, what) in [
        (
            ": (Tokens.texture-physical ? Tokens.track-head-brush : #0b0e16);",
            "轨道头未选中支走 §12 轨道头刷 (flat 字面支)",
        ),
        ("background: Tokens.clip-sheen;", "片段顶光走 §12 令牌"),
        ("background: Tokens.clip-shade;", "片段底影走 §12 令牌"),
        (
            "Tokens.texture-physical ? Tokens.void-brush : #06080d",
            "轨道头表井 (void-brush, flat #06080d)",
        ),
    ] {
        assert!(
            arrangement.contains(needle),
            "arrangement.slint 必须含 {what}: {needle}"
        );
    }
    assert!(
        !arrangement.contains("with-alpha(0.10) : #0b0e16;"),
        "轨道头二期前的直写平色形不得回归 (必须走 §12 三元)"
    );

    // ⑤ 状态栏。
    let status = read("status_bar.slint");
    assert!(
        status.contains("Tokens.texture-physical ? Tokens.panel-brush : Tokens.bg-panel"),
        "status_bar.slint 状态栏底必须走 panel-brush 三元 (flat 色值支)"
    );
    assert!(
        !status.contains("background: Tokens.bg-panel;"),
        "status_bar.slint 不得回归 bg-panel 平色"
    );
}

// ---------------------------------------------------------------------------
// 判据（源码钉）: 负责人 2026-11-16 四条 UI 要求（并入拟物线一期）
//
// 1. 删冗余说明文字: 只说明区域名、不承载信息、不参与操作的可见标签
//    一律删（钉住实际删除的原文不再以 `text:` 出现; 注释里提及旧文案
//    不算 —— 钉的是 `text: "…"` 的赋值形）。
// 2. 轨道菜单前置: `arrangement.slint` 里「轨道菜单」的首个 `@tr` 必须
//    在「+ 轨道」的首个 `@tr` 之前（左置工具行是文件里先出现的那行）。
// 3. 分界线走令牌: 行底线 / 顶行横线 / 分隔条 / 网格竖线一律
//    `Tokens.divider-*`; 行框按 2026-11-17 rebase 裁定保留 main 形态
//    （未选中行 #ffffff0a 淡框 + 行底单线对选中行门控）。
// 4. 标尺走令牌: 标尺底 `Tokens.ruler-brush`、小节号 `Tokens.ruler-ink`,
//    标尺刻度（仅拟物套可见，rebase 裁定）与网格同用 `divider-grid-strong`。
// ---------------------------------------------------------------------------

#[test]
fn the_track_area_follows_the_four_ui_rulings() {
    let ui_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let read = |rel: &str| {
        std::fs::read_to_string(ui_dir.join(rel)).unwrap_or_else(|_| panic!("读 {rel}"))
    };
    let tokens = read("tokens.slint");
    // §11 同键双套值（负责人定案：扁平/实体并存可切换）：扁平分支必须是
    // main 现行值，拟物分支才许换质感值。2026-11-17 rebase 到 4af1fe2c1
    // 按「以 main 为底、flat 逐像素零差」把扁平分支逐支重钉回 main 的
    // ui-header-cleanup 同题实现（border-subtle / #ffffff04 / #ffffff0d /
    // Theme.bg-panel / #cbd5e1）；本支初版的 line 家族扁平值已被取代。
    for (needle, what) in [
        (
            "out property <color> divider-row: texture-physical ? #43536a : Theme.border-subtle;",
            "行间分界令牌（扁平套 = main 现行 Theme.border-subtle）",
        ),
        (
            "out property <color> divider-row-highlight: #ffffff.with-alpha(texture-physical ? 0.05 : 0.0);",
            "分界刻蚀高光令牌（扁平套 alpha = 0）",
        ),
        (
            "out property <color> divider-grid: texture-physical ? line.with-alpha(0.55) : #ffffff04;",
            "网格弱档令牌（扁平套 = main 现行 #ffffff04）",
        ),
        (
            "out property <color> divider-grid-strong: texture-physical ? line-strong.with-alpha(0.8) : #ffffff0d;",
            "网格强档令牌（扁平套 = main 现行 #ffffff0d）",
        ),
        (
            "? @linear-gradient(180deg, bg-panel, bg-shell)",
            "标尺带底拟物套渐变",
        ),
        (
            ": @linear-gradient(180deg, Theme.bg-panel, Theme.bg-panel);",
            "标尺带底扁平套纯色（main 现行 Theme.bg-panel）",
        ),
        (
            "out property <color> ruler-ink: texture-physical ? ink-0 : #cbd5e1;",
            "小节号墨色令牌（扁平套 = main 现行 #cbd5e1）",
        ),
    ] {
        assert!(
            tokens.contains(needle),
            "tokens.slint 必须含 {what}: {needle}"
        );
    }

    // 用户切换入口（走带栏 Group L）：翻的是同一个 `Tokens.texture`。
    let transport_bar = read("transport_bar.slint");
    assert!(
        transport_bar.contains(
            "Tokens.texture = Tokens.texture == TextureMode.physical ? TextureMode.flat : TextureMode.physical;"
        ),
        "走带栏必须有质感切换钮（翻 Tokens.texture 本身）"
    );
    assert!(
        transport_bar.contains("质感主题: 实体 (点击切回扁平)"),
        "质感切换钮必须有状态 tooltip"
    );

    let arrangement = read("arrangement.slint");
    for gone in [
        "text: \"轨道控制头 (TRACKS)\"",
        "text: \"轨道控制头 (右置)\"",
        "text: \"右置轨道头模式 (镜像工作区)\"",
        "text: \"智能和声声学\"",
    ] {
        assert!(
            !arrangement.contains(gone),
            "冗余说明文字不得回归 arrangement.slint: {gone}"
        );
    }
    let menu_at = arrangement
        .find("@tr(\"轨道菜单\")")
        .expect("轨道菜单必须在");
    let add_at = arrangement.find("@tr(\"+ 轨道\")").expect("+ 轨道必须在");
    assert!(
        menu_at < add_at,
        "轨道菜单必须排在 + 轨道之前（工具行最前）: menu@{menu_at} add@{add_at}"
    );
    for (needle, what) in [
        ("background: Tokens.divider-row;", "分界线走 divider-row"),
        (
            "background: Tokens.divider-row-highlight;",
            "分界刻蚀高光走 divider-row-highlight",
        ),
        ("background: Tokens.ruler-brush;", "标尺底走 ruler-brush"),
        ("color: Tokens.ruler-ink;", "小节号走 ruler-ink"),
        (
            "Tokens.divider-grid-strong : Tokens.divider-grid",
            "网格强弱两档走令牌",
        ),
    ] {
        assert!(
            arrangement.contains(needle),
            "arrangement.slint 必须含 {what}: {needle}"
        );
    }
    // 2026-11-17 rebase 裁定（以 main 为底）：main 的 ui-header-cleanup
    // 保留未选中行 #ffffff0a 淡框 + 行底单线（选中行门控不叠线），本支
    // 初版「删全框、只留行底线」的半条让位；钉住 main 形态与门控在。
    assert!(
        arrangement.contains("with-alpha(0.70) : #ffffff0a"),
        "rebase 裁定: 未选中行保留 main 的 #ffffff0a 淡框（以 main 为底）"
    );
    assert!(
        arrangement.contains("visible: trk.id != root.selected-track-id;"),
        "行底分界线必须保留 main 的选中行门控"
    );

    // 全 UI 同类排查的删除钉（各文件实际删掉的区域名/描述标签）。
    for (rel, gone) in [
        ("clip_matrix.slint", "text: \"Clip 激发矩阵\""),
        ("control_room.slint", "text: \"监听控制室\""),
        ("control_room.slint", "text: \"MONITOR MATRIX\""),
        ("sidebar.slint", "text: \"资源库\""),
        ("console/asset_pool_panel.slint", "text: \"素材池\""),
        (
            "console/device_rack.slint",
            "text: \"动态、侧链与瞬态控制\"",
        ),
        ("left_panel.slint", "text: \"总线效果器 (BUS INSERTS)\""),
    ] {
        assert!(
            !read(rel).contains(gone),
            "冗余说明文字不得回归 {rel}: {gone}"
        );
    }
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
