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

/// 按词界匹配 `needle`：`needle` 后随 `-` / 字母数字 / 下划线的不算命中。
/// 退役令牌名是现行令牌的前缀时（如 `Tokens.roll-lane` 之于
/// `Tokens.roll-lane-black` / `Tokens.roll-lane-white` 平色），裸 `contains`
/// 会误伤，须走词界匹配。
fn contains_token(haystack: &str, needle: &str) -> bool {
    let mut rest = haystack;
    while let Some(idx) = rest.find(needle) {
        let after = &rest[idx + needle.len()..];
        let boundary = after
            .chars()
            .next()
            .map(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(true);
        if boundary {
            return true;
        }
        rest = after;
    }
    false
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
            "Tokens.texture-physical ? Tokens.panel-brush : Tokens.bg-shell",
            "通道条/总线条背景 (panel-brush; 2026-11-19 裁决去 accent: flat 支恒 bg-shell, 选中靠边框与色标条)",
        ),
        (
            "Tokens.texture-physical ? Tokens.shell-brush : Theme.bg-surface",
            "混音区外容器背景 (shell-brush, flat Theme.bg-surface)",
        ),
        (
            "Tokens.texture-physical ? Tokens.void-brush : Tokens.bg-lane2",
            "推子导轨背景 (void-brush; 补丁包第四轮改钉: flat 支换 Tokens.bg-lane2)",
        ),
        (
            "Tokens.texture-physical ? Tokens.raised-brush : Tokens.ink-1",
            "推子帽背景 (raised-brush; 补丁包第四轮改钉: flat 支换 Tokens.ink-1)",
        ),
        (
            "Tokens.texture-physical ? Tokens.glass-brush : Tokens.bg-lane2",
            "电平槽/设备屏背景 (glass-brush; 补丁包第四轮改钉: flat 支换 Tokens.bg-lane2)",
        ),
        (
            "Tokens.texture-physical ? (root.device-bypassed[idx] ? Tokens.void-brush : Tokens.panel-brush) : Theme.bg-surface",
            "设备卡片背景 (旁通 void-brush, 常态 panel-brush)",
        ),
        (
            "Tokens.texture-physical ? Tokens.raised-brush : Tokens.skin-text-body",
            "主推子帽背景 (raised-brush; 补丁包第四轮改钉: flat 支换 Tokens.skin-text-body)",
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
            ": @linear-gradient(180deg, #d9dfe9, #d9dfe9);",
            "卷帘白键刷 flat 分支 = 现行字面 #d9dfe9（f7c5192f 改钉）",
        ),
        (
            ": @linear-gradient(180deg, #191e2b, #191e2b);",
            "卷帘黑键行刷 flat 分支 = 现行字面 #191e2b",
        ),
        (
            ": @linear-gradient(180deg, #0b0e15, #0b0e15);",
            "卷帘黑键帽刷 flat 分支 = 现行字面 #0b0e15",
        ),
    ] {
        assert!(
            tokens.contains(needle),
            "tokens.slint 必须含 {what}: {needle}"
        );
    }
    // 编辑面材质令牌已撤销（2026-11-17 拟物重点口径：编辑面回平）:
    // 片段顶光/底影、卷帘车道/网格刷不得回归。
    for gone in [
        "clip-sheen",
        "clip-shade",
        "roll-lane-white-brush",
        "roll-lane-black-brush",
        "roll-lane-root-brush",
        "roll-grid-brush",
    ] {
        assert!(
            !tokens.contains(gone),
            "编辑面材质令牌不得回归 tokens.slint: {gone}"
        );
    }

    // ① 钢琴卷帘页。使用点一律三元: flat 走字面色支 (渐变刷作
    // background 时软件渲染器的圆角 AA 与纯色填充不一致, 盒上实证)。
    let roll = read("console/piano_roll.slint");
    for (needle, what) in [
        (
            "background: Tokens.roll-key-white-brush;",
            "键列容器即白键平面 (f7c5192f3 改钉: 容器底即白键刷, 不再是 shell-brush/bg-lane 三元)",
        ),
        ("Tokens.roll-key-white-brush", "白键面走 §12 键刷"),
        ("Tokens.roll-key-black-brush", "黑键行走 §12 键刷"),
        ("Tokens.roll-key-cap-brush", "黑键帽走 §12 键刷"),
        (
            "Tokens.texture-physical ? Tokens.panel-brush : Tokens.bg-panel",
            "右键菜单走 panel-brush (flat 色值支)",
        ),
        // 编辑面回平目标值（2026-11-17 拟物重点口径）: 根底/网格/车道/
        // 力度泳道 flat 同为平色; 网格底与车道三档的 physical 支于
        // 2026-11-17 按 Bitwig 采样提亮 (Wang 定拟物版卷帘背景太深),
        // flat 支仍为回平字面值。
        ("background: Tokens.bg-void;", "卷帘根底回平 (bg-void 平色)"),
        (
            "background: Tokens.texture-physical ? Tokens.bg-lane : Tokens.bg-lane;",
            "网格底 (补丁包第四轮改钉: 两支同走 bg-lane 令牌, 质感差在令牌内)",
        ),
        (
            "background: (root.row-pitch(lane_index).mod(12) == 0) ? Tokens.grid-bar : Tokens.bg-lane2;",
            "车道三档 (2026-11-18 夜改钉: 根音行底边走 grid-bar 强度层 (基色 skin-grid-bar), 其余行 bg-lane2; 行身走 roll-lane 黑/白)",
        ),
        (
            "background: Tokens.bg-panel;",
            "力度泳道回平 (bg-panel 平色)",
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
    for gone in ["Tokens.roll-grid-brush", "Tokens.void-brush"] {
        assert!(!roll.contains(gone), "卷帘编辑面质感刷不得回归: {gone}");
    }
    // `Tokens.roll-lane` 退役刷不得回归；f7c5192f3 起行身走
    // `Tokens.roll-lane-black` / `Tokens.roll-lane-white` 平色令牌，裸子串
    // 匹配会误伤，故按词界匹配。
    assert!(
        !contains_token(&roll, "Tokens.roll-lane"),
        "卷帘编辑面质感刷不得回归: Tokens.roll-lane"
    );

    // ② 左栏检查器 + 智能音色库。
    let left = read("left_panel.slint");
    for (needle, what) in [
        (
            "Tokens.texture-physical ? Tokens.panel-brush : Theme.bg-panel",
            "左栏根底 (panel-brush, flat Theme.bg-panel)",
        ),
        (
            "Tokens.texture-physical ? Tokens.shell-brush : Tokens.bg-shell",
            "子页签栏 (shell-brush; 补丁包第四轮改钉: flat 支换 Tokens.bg-shell)",
        ),
        (
            "Tokens.texture-physical ? Tokens.panel-brush : Theme.bg-surface",
            "轨道头卡/通道条/总线条/音色条目卡 (panel-brush, flat Theme.bg-surface)",
        ),
        (
            "Tokens.texture-physical ? Tokens.glass-brush : Tokens.bg-lane2",
            "EQ 屏/总线压缩屏 (glass-brush; 补丁包第四轮改钉: flat 支换 Tokens.bg-lane2)",
        ),
        (
            "Tokens.texture-physical ? Tokens.void-brush : Tokens.bg-lane2",
            "推子导轨/表井/搜索框 (void-brush; 补丁包第四轮改钉: flat 支换 Tokens.bg-lane2)",
        ),
        (
            "Tokens.texture-physical ? Tokens.raised-brush : Tokens.ink-1",
            "推子帽 (raised-brush; 补丁包第四轮改钉: flat 支换 Tokens.ink-1)",
        ),
        (
            "Tokens.texture-physical ? Tokens.glass-brush : Tokens.bg-shell",
            "dB 读数牌 (glass-brush; 补丁包第四轮改钉: flat 支换 Tokens.bg-shell)",
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

    // ③ 走带栏本体（质感切换钮已按负责人 2026-11-17 口径撤出走带栏、
    // 迁主菜单「设置 → 质感」, 入口钉在本文件 the_track_area 一节; 这里
    // 只钉栏体渲染）。
    let transport = read("transport_bar.slint");
    for (needle, what) in [
        (
            "Tokens.texture-physical ? Tokens.panel-brush : Theme.header-bg",
            "走带栏体 (panel-brush, flat Theme.header-bg)",
        ),
        (
            "Tokens.texture-physical ? Tokens.shell-brush : transparent",
            "走带组板 (f7c5192f3 改钉: flat 支 no plate, 不再是 bg-panel)",
        ),
        (
            "Tokens.texture-physical ? Tokens.shell-brush : Tokens.bg-lane2",
            "走带簇底板 (shell-brush; 补丁包第四轮改钉: flat 支换 Tokens.bg-lane2)",
        ),
        (
            "Tokens.texture-physical ? Tokens.glass-brush : Tokens.skin-inset",
            "LCD 框 (glass-brush; 补丁包第四轮改钉: flat 支换 Tokens.skin-inset)",
        ),
        (
            "Tokens.texture-physical ? Tokens.raised-brush : Tokens.bg-panel",
            "命令钮 (raised-brush; 补丁包第四轮改钉: flat 支换 Tokens.bg-panel)",
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
            "background: Tokens.texture-physical ? (trk.id == root.selected-track-id ? Theme.accent.with-alpha(0.10) : Tokens.track-head-brush) : (trk.id == root.selected-track-id ? Theme.accent.with-alpha(0.12) : Tokens.bg-panel);",
            "轨道头底走三元 (拟物 §12 头刷/accent α0.10 选中; 补丁包第四轮改钉: 扁平支改 accent α0.12 选中 / bg-panel 未选中)",
        ),
        (
            "Tokens.texture-physical ? Tokens.void-brush : Tokens.bg-lane",
            "轨道头表井 (void-brush; 补丁包第四轮改钉: flat 支换 Tokens.bg-lane)",
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
    assert!(
        !arrangement.contains("Tokens.clip-sheen") && !arrangement.contains("Tokens.clip-shade"),
        "片段顶光/底影不得回归（编辑面回平，2026-11-17 口径）"
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
//    ——合入裁决甲 (Wang 2026-11-17 19:58) 后: 编曲区的行底分界线、
//    行框、网格竖线改为质感三元, main 形态只活在拟物支, 扁平支用
//    网格十一版字面定值; 本节相应钉改钉三元全串 (见下方断言)。
// 4. 标尺走令牌: 标尺底 `Tokens.ruler-brush`、小节号 `Tokens.ruler-ink`,
//    标尺刻度（仅拟物套可见，rebase 裁定）与网格同用 `divider-grid-strong`。
//    ——裁决甲后: 标尺底改质感三元 (拟物 ruler-brush / 扁平十一版
//    定值 #1d2733), 扁平支另有三档字面刻度块 (网格十一版)。
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
            "out property <color> divider-row: texture-physical ? #47535c : Theme.border-subtle;",
            "行间分界令牌（扁平套 = main 现行 Theme.border-subtle；拟物套 2026-11-17 换 Bitwig 描边家族 #47535c）",
        ),
        (
            "out property <color> divider-row-highlight: #ffffff.with-alpha(texture-physical ? 0.05 : 0.0);",
            "分界刻蚀高光令牌（扁平套 alpha = 0）",
        ),
        (
            "out property <color> divider-grid: texture-physical ? #7a8694 : #ffffff04;",
            "网格弱档令牌（扁平套 = main 现行 #ffffff04；拟物套 2026-11-17 车道底提亮后改实色 #7a8694，对新底 ≥3:1）",
        ),
        (
            "out property <color> divider-grid-strong: texture-physical ? #9ba5b0 : #ffffff0d;",
            "网格强档令牌（扁平套 = main 现行 #ffffff0d；拟物套 2026-11-17 改实色 #9ba5b0，对新底 ≥4.5:1）",
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

    // 用户切换入口（负责人 2026-11-17 口径：语言与质感两个切换入口
    // 统一进系统主菜单，不要走带栏按钮/chip）：主菜单「设置 → 质感」
    // 两项调 select-texture，宿主经唯一写入口 apply_texture 写同一个
    // `Tokens.texture`；走带栏两钮（质感开关 + 语言 chip）已撤，不得回归。
    let app = read("app.slint");
    assert!(
        app.contains("root.select-texture(\"flat\")"),
        "主菜单「设置 → 质感」必须有扁平项（调 select-texture）"
    );
    assert!(
        app.contains("root.select-texture(\"physical\")"),
        "主菜单「设置 → 质感」必须有实体项（调 select-texture）"
    );
    let host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        host.contains("ui.on_select_texture("),
        "宿主必须接 select-texture 回调（菜单项的宿主落点）"
    );
    assert!(
        host.contains("apply_texture(&ui, texture)"),
        "select-texture 必须经唯一写入口 apply_texture 应用"
    );
    let transport_bar = read("transport_bar.slint");
    assert!(
        !transport_bar.contains("touch_texture"),
        "走带栏质感切换钮不得回归（入口已迁主菜单）"
    );
    assert!(
        !transport_bar.contains("Tokens.texture = Tokens"),
        "走带栏不得再直接写 Tokens.texture（写者收敛为 host::apply_texture）"
    );
    assert!(
        !transport_bar.contains("lang-menu-popup"),
        "走带栏语言 chip 不得回归（入口已迁主菜单）"
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
        (
            "background: Tokens.grid-row;",
            "行底分界线走 grid-row 强度层令牌 (2026-11-18 夜改钉: 基色 skin-grid-row 经 tokens 强度派生; 基色值由 grid 判据看守)",
        ),
        (
            "background: Tokens.divider-row-highlight;",
            "分界刻蚀高光走 divider-row-highlight",
        ),
        (
            "background: Tokens.texture-physical ? Tokens.ruler-brush : Tokens.bg-shell;",
            "标尺底走三元 (拟物 ruler-brush; 补丁包第四轮改钉: 扁平支换 Tokens.bg-shell)",
        ),
        ("color: Tokens.ruler-ink;", "小节号走 ruler-ink"),
        (
            "background: Tokens.ruler-tick-bar;",
            "标尺刻度走 ruler-tick-* 令牌 (补丁包第四轮改钉: 取代 divider-grid 两档)",
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
    // ——合入裁决甲 (Wang 2026-11-17 19:58) 改钉: 上述 main 形态只许
    // 活在质感三元的拟物支, 两串改钉三元全串 (扁平支分别为恒
    // #ffffff0a 行框与十一版暗线恒画)。
    assert!(
        arrangement.contains("border-color: Tokens.texture-physical ? (trk.id == root.selected-track-id ? Theme.accent.with-alpha(0.80) : Theme.border-subtle) : Theme.border-subtle;"),
        "补丁包第四轮: 轨道头边框必须走三元 (拟物支选中 accent α0.80 边, 两支未选中同为 border-subtle)"
    );
    assert!(
        !arrangement.contains(
            "visible: Tokens.texture-physical ? trk.id != root.selected-track-id : true;"
        ),
        "补丁包第四轮: 行底分界线的质感门控已撤销 (grid-row 恒画), 旧门控串不得回归"
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
// 判据（源码钉）: 三期六面（2026-11-17）+ ConsoleTabs 普查 + Knob 禁用态
//
// 六面: ① 控制室、② Clip 矩阵抽屉、③ Session 视图、④ 全曲总览、
// ⑤ 对话框/模态（modals + dialogs 五件）、⑥ sidebar 本体；连带
// console/asset_pool_panel（ConsoleTabs 判据面的第三块面板，见下）。
// 口径同一/二期: 铬面底色一律质感三元（flat 分支 = 原表达式逐字），
// 内容色（轨道色 / 频带色 / 电平色 / diff 色 / 状态色 / 遮罩）不动；
// 六面文件里不得再有裸 `background: Tokens.bg-* / Theme.bg-*;`。
//
// 2026-11-17 晚口径修订（负责人拟物重点口径）: ②③④ 是编辑面，本体
// 回平 —— 矩阵/总览的根底、画布、插槽、列头回到裸平色（本节后段逐面
// 改钉回平目标值 + 负向钉），质感只留给它们内部的按钮（三元保留）；
// ①⑤⑥ 与素材池维持全质感口径不变。
//
// ConsoleTabs 普查结论（2026-11-17）: 组件在 app.slint 里挂在
// `if false` 转发块下（原注「Compatibility Forwarding Block for
// Automated Criteria Tests」），真实窗口不可达；但它不是可删死 UI ——
// 其三页（卷帘 / 混音 / 设备）的功能在 UnifiedLowerDock 以同一
// `console-tab` 状态存活（F7/F8/F9、屏读器动作、dock 页签同源），且
// 约五十份判据经它转发对账混音/设备接线（删它等于删活功能的验证
// 面）。处置 = 按「在用（判据面）」保留并与其他面一样拟物化（混音台 /
// 设备架一期已迁共享控件，素材池本期补迁）。本钉锁住该处置的形态：
// 转发块在、素材池走令牌。若日后要真删，须连判据面迁移一并立项。
//
// Knob 裁定（2026-11-17，学 Logic 无装饰旋钮）: 产品 UI 现无任何
// Knob 落点（声相是文本拖动面、静态 EQ 旋钮已于 #157 删除），唯一
// 使用方是陈列 example；组件已备 `enabled` 置灰禁用态，无真实参数
// 可绑的落点必须显式给 false。本钉锁住禁用态的源码形态。
// ---------------------------------------------------------------------------

#[test]
fn the_phase3_surfaces_bind_texture_tokens_with_flat_fallback() {
    let ui_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let read = |rel: &str| {
        std::fs::read_to_string(ui_dir.join(rel)).unwrap_or_else(|_| panic!("读 {rel}"))
    };
    // 全质感面（编辑面②③④已按 2026-11-17 晚口径移出，见节头注与
    // 本节后段的回平专钉）。
    let phase3_files = [
        "control_room.slint",
        "sidebar.slint",
        "modals.slint",
        "dialogs/export_wav_dialog.slint",
        "dialogs/humanize_dialog.slint",
        "dialogs/macro_panel.slint",
        "dialogs/musical_pr_drawer.slint",
        "dialogs/undo_tree_modal.slint",
        "console/asset_pool_panel.slint",
    ];
    for rel in phase3_files {
        let src = read(rel);
        assert!(
            src.contains("Tokens.texture-physical ? Tokens."),
            "{rel} 必须有质感三元落点（三期拟物化）"
        );
        for bare in [
            "background: Tokens.bg-panel-alt;",
            "background: Tokens.bg-void;",
            "background: Tokens.bg-raised;",
            "background: Tokens.bg-shell;",
            "background: Theme.bg-panel;",
            "background: Theme.bg-surface;",
        ] {
            assert!(
                !src.contains(bare),
                "{rel} 不得有裸底色（必须走质感三元）: {bare}"
            );
        }
    }
    // 补丁包第四轮改钉: `background: Tokens.bg-panel;` 从裸底禁单
    // 豁免——bg-panel 令牌定义本身已含质感支 (yeban 支
    // texture-physical ? #1b1d1f : #30343b, f7c5192f 改钉), 裸用点经令牌跟随
    // 质感; 改钉令牌定义支, 防豁免失效。
    assert!(
        read("tokens.slint").contains("texture-physical ? #1b1d1f : #30343b"),
        "Tokens.bg-panel 定义必须含质感支 (裸用豁免的前提)"
    );
    // 逐面代表钉。
    for (rel, needle, what) in [
        (
            "control_room.slint",
            "Tokens.texture-physical ? Tokens.glass-brush : Tokens.bg-lane2",
            "控制室主表井走 glass-brush (补丁包第四轮改钉: flat 支换 Tokens.bg-lane2)",
        ),
        (
            "control_room.slint",
            "background: Tokens.recess-dark;",
            "控制室表井必须有凹陷深线",
        ),
        (
            "sidebar.slint",
            "Tokens.texture-physical ? Tokens.panel-brush : Tokens.bg-panel;",
            "sidebar 根底走 panel-brush",
        ),
        (
            "modals.slint",
            "Tokens.texture-physical ? Tokens.panel-brush : Tokens.bg-panel-alt;",
            "命令面板体走 panel-brush (补丁包第四轮改钉: flat 支换 Tokens.bg-panel-alt)",
        ),
        (
            "console/asset_pool_panel.slint",
            "Tokens.texture-physical ? Tokens.panel-brush : Tokens.bg-panel;",
            "素材池根底走 panel-brush（ConsoleTabs 判据面同迁）",
        ),
    ] {
        assert!(read(rel).contains(needle), "{rel} 必须含 {what}: {needle}");
    }

    // 编辑面回平专钉（2026-11-17 晚口径）: ②③④ 本体必须是裸平色
    // （回平目标值逐字钉），内部按钮保留质感三元，片段/格面顶光底影
    // 令牌不得回归。
    let clip_matrix = read("clip_matrix.slint");
    assert!(
        clip_matrix.contains("background: Theme.bg-panel;"),
        "Clip 矩阵根底必须回平 (Theme.bg-panel)"
    );
    assert!(
        clip_matrix.contains("background: Tokens.bg-shell;"),
        "Clip 矩阵头部带必须回平 (补丁包第四轮改钉: 字面 #080a10 换 Tokens.bg-shell)"
    );
    assert!(
        clip_matrix.contains("Tokens.texture-physical ? Tokens.raised-brush : Tokens.bg-panel"),
        "Clip 矩阵按钮必须保留质感三元 (raised-brush; 补丁包第四轮改钉: flat 支换 Tokens.bg-panel)"
    );
    assert!(
        !clip_matrix.contains("Tokens.clip-sheen") && !clip_matrix.contains("Tokens.clip-shade"),
        "Clip 矩阵格面顶光/底影不得回归（编辑面回平）"
    );
    let session = read("workspace/session_view.slint");
    assert!(
        session.contains("background: Tokens.bg-void;"),
        "Session 根底必须回平 (bg-void)"
    );
    assert!(
        session.contains("background: Tokens.bg-shell;"),
        "Session 插槽列必须回平 (bg-shell)"
    );
    assert!(
        session.contains("background: Tokens.bg-panel;"),
        "Session 插槽格必须回平 (bg-panel)"
    );
    assert!(
        session.contains("Tokens.texture-physical ? ((scene_touch.pressed"),
        "Session 场景激发行（按钮面）必须保留质感三元"
    );
    let overview = read("workspace/song_overview_panel.slint");
    assert!(
        overview.contains("background: Tokens.bg-void;"),
        "全曲总览画布必须回平 (bg-void)"
    );
    assert!(
        !overview.contains("Tokens.void-brush"),
        "全曲总览不得再用 void-brush（编辑面回平）"
    );
    assert!(
        overview.contains("pill_touch.pressed ? Tokens.raised-brush"),
        "全曲总览层开关胶囊（按钮面）必须保留质感三元"
    );

    // ConsoleTabs 普查处置钉: 转发块保留（判据面在用，见本节头注）。
    let app = read("app.slint");
    assert!(
        app.contains("if false : console_tabs := ConsoleTabs {"),
        "ConsoleTabs 判据转发块必须保留（2026-11-17 普查裁定: 在用）"
    );

    // Knob 禁用态钉。
    let knob = read("controls/knob.slint");
    assert!(
        knob.contains("in property <bool> enabled: true;"),
        "Knob 必须有 enabled 属性（无参数可绑时置灰禁用）"
    );
    assert!(
        knob.contains("visible: !root.enabled;"),
        "Knob 禁用态必须有去色罩"
    );
    assert!(
        knob.contains("enabled: root.enabled;"),
        "Knob 禁用态必须关闭手势面"
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
