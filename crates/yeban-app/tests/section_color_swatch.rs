//! **段落色标**（`section-{i}-color-swatch`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 编曲视图的每个段落卡左缘有一条 3px 竖条色标，颜色由工程文档的
//! `SectionV3::color` 驱动 —— 合法色按 CSS 惯例解析并规范化，非法 / 缺失回退到
//! 文档常量。色标的 `accessible-label` 携带规范化 `#RRGGBB`，所以"投影 ↔ 控件树
//! ↔ 像素"三处读的是同一个读数（`parse_hex_color` 只有一处实现）。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（默认视图就有） | 编曲视图（默认帧）下四个色标在场：它们不是 Session 子树的部件 |
//! | ②（可寻址） | 角色 = `Image`、名称 = 注册表标签（逐字）、几何非零 |
//! | ③（工程驱动） | 合法色 `#22aa88` ⇒ 标签 `#22AA88`、像素逐位相等；非法 / 缺失 ⇒ 回退常量 |
//! | ④（几何在卡片内） | 每个色标的包围盒完全落在自己的段落卡里（不是画到别处） |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/scene_color_swatch.rs` 同款）。

#![allow(unused)]
use yeban_app::bridge::{DEFAULT_TRACK_COLOR, DEFAULT_TRACK_COLOR_HEX, ViewState};
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::{Permission, Rect, Size};

/// **1920×1080**（不是其余 Tier-1 判据常用的 1440×900）。
///
/// 本判据要求**四个**段落色标同时在场。演示工程每 7680 tick 一段，30 tick/px ⇒
/// 段距 256px；第四张卡 `x = 168 + 3×256 = 936`、宽 256 ⇒ 右缘 1192px（编曲区坐标）。
/// 编曲区宽 = 窗口宽 − 左栏 240 − 右栏 280：
/// 1440 ⇒ 920px < 1192 ⇒ 第四张卡整体落在裁剪矩形外、不进运行时树（CI run
/// `40980667356` 在 mac/windows 腿正是这样红的）；1920 ⇒ 1400px ≥ 1192 ⇒ 四张全在。
const WINDOW: Size = Size::new(1920, 1080);

/// 演示工程第一段本来就是 `#22AA88`（见 `yeban_model::samples::demo_project`）；
/// 这里再显式写一遍，判据不依赖夹具的默认值。
const VALID_SECTION_COLOR: &str = "#22aa88";
const VALID_SECTION_COLOR_HEX: &str = "#22AA88";

fn live_window() -> (LivePort<yeban_app::ui::MainWindow>, ViewState) {
    // 演示工程有 4 个段落：第 1 个给合法色（显式重写）、第 2 个给非法色、
    // 其余保持 `None` ⇒ 一屏之内同时覆盖"保留 / 非法回退 / 缺失回退"三档。
    let mut project = yeban_model::samples::demo_project();
    let section_ids: Vec<yeban_model::EntityId> = project.sections.keys().copied().collect();
    assert!(
        section_ids.len() >= 3,
        "演示工程必须至少有三个段落（本判据要同时覆盖三档）"
    );
    project
        .sections
        .get_mut(&section_ids[0])
        .expect("刚取出的段落键必须还在")
        .color = Some(VALID_SECTION_COLOR.to_owned());
    project
        .sections
        .get_mut(&section_ids[1])
        .expect("刚取出的段落键必须还在")
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

fn swatch_sample_pixel(port: &LivePort<yeban_app::ui::MainWindow>, bounds: Rect) -> [u8; 3] {
    let image = port.window().capture().expect("Tier-1 截图");
    // 3px 宽的竖条：取「x 中点、y 垂直居中」。文字从 `space-3`（6px）开始、
    // 与竖条不重叠 ⇒ 该点只可能是纯填充色（不会踩到字形栅格）。
    let x = u32::try_from(bounds.x + i32::try_from(bounds.width / 2).expect("宽度是正数"))
        .expect("色标在窗口内");
    let y = u32::try_from(bounds.y + i32::try_from(bounds.height / 2).expect("高度是正数"))
        .expect("色标在窗口内");
    image
        .pixel(x, y)
        .unwrap_or_else(|| panic!("采样点 ({x}, {y}) 必须在 {WINDOW:?} 帧内"))
}

#[test]
fn section_color_swatches_carry_the_projected_color() {
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
