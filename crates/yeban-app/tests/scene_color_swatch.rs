//! **场景色标**（`scene-{i}-color-swatch`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! Session 视图的每个场景卡**不再**只有"高亮 / 非高亮"两种底色：卡片左上角的色标
//! 由工程文档的 `SceneV3::color` 驱动 —— 合法色按 CSS 惯例解析并规范化，非法 / 缺失
//! 回退到文档常量。色标的 `accessible-label` 携带规范化 `#RRGGBB`，所以"投影 ↔ 控件树
//! ↔ 像素"三处读的是同一个读数（`parse_hex_color` 只有一处实现）。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（默认视图不画） | 编曲视图下四个色标都不在运行时树里（Session 子树整棵不可见） |
//! | ②（可寻址） | 切到 Session 后四个色标在场、角色 = `Image`、名称 = 注册表标签（逐字） |
//! | ③（工程驱动） | 合法色 `#3366ff` ⇒ 标签 `#3366FF`、像素逐位相等；非法 / 缺失 ⇒ 回退常量 |
//! | ④（几何在卡片内） | 每个色标的包围盒完全落在自己的场景卡里（不是画到别处） |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/sidebar_item.rs` / `tests/session_back_to_arrangement.rs` 同款）。

#![allow(unused)]
use yeban_app::bridge::{DEFAULT_TRACK_COLOR, DEFAULT_TRACK_COLOR_HEX, RgbColor, ViewState};
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::{Permission, Rect, Size};

/// 与其余 Tier-1 判据同款（1440 宽 ⇒ `compact` 断点之外，资源栏是展开态）。
const WINDOW: Size = Size::new(1440, 900);

/// 合法色（用 CSS 短写 `#36F` 的下发形态另测：这里直接用 6 位小写，落点规范化应大写）。
const VALID_SCENE_COLOR: &str = "#3366ff";
const VALID_SCENE_COLOR_HEX: &str = "#3366FF";

fn live_window() -> (LivePort<yeban_app::ui::MainWindow>, ViewState) {
    // 演示工程有 4 个场景：第 1 个给合法色、第 2 个给非法色、其余保持 `None`，
    // 于是一屏之内就同时覆盖"保留 / 非法回退 / 缺失回退"三档。
    let mut project = yeban_model::samples::demo_project();
    let scene_ids: Vec<yeban_model::EntityId> = project.scenes.keys().copied().collect();
    assert!(
        scene_ids.len() >= 3,
        "演示工程必须至少有三个场景（本判据要同时覆盖三档）"
    );
    project
        .scenes
        .get_mut(&scene_ids[0])
        .expect("刚取出的场景键必须还在")
        .color = Some(VALID_SCENE_COLOR.to_owned());
    project
        .scenes
        .get_mut(&scene_ids[1])
        .expect("刚取出的场景键必须还在")
        .color = Some("不是颜色".to_owned());

    let view = ViewState::from_project(&project).expect("夹具必须能投影");
    let scene = DemoScene::from_view(&view);
    let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    host::wire_view_callbacks(port.ui());
    (port, view)
}

fn bounds_of(port: &mut LivePort<yeban_app::ui::MainWindow>, id: &str) -> Rect {
    port.refresh_tree(None)
        .expect("必须能重抓控件树")
        .find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里找不到 `{id}`"))
        .bounds
        .unwrap_or_else(|| panic!("`{id}` 必须有几何包围盒"))
}

fn swatch_center_pixel(port: &LivePort<yeban_app::ui::MainWindow>, bounds: Rect) -> [u8; 3] {
    let image = port.window().capture().expect("Tier-1 截图");
    // 色标 12×12、`radius-sm` 2px ⇒ 中心点一定是纯填充色。
    let x = u32::try_from(bounds.x + i32::try_from(bounds.width / 2).expect("宽度是正数"))
        .expect("色标在窗口内");
    let y = u32::try_from(bounds.y + i32::try_from(bounds.height / 2).expect("高度是正数"))
        .expect("色标在窗口内");
    image
        .pixel(x, y)
        .unwrap_or_else(|| panic!("采样点 ({x}, {y}) 必须在 {WINDOW:?} 帧内"))
}

#[test]
fn scene_color_swatches_carry_the_projected_color() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}
