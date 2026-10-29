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

use yeban_app::bridge::{DEFAULT_TRACK_COLOR, DEFAULT_TRACK_COLOR_HEX, ViewState};
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::{Permission, Rect, Size};

/// 与其余 Tier-1 判据同款（1440 宽 ⇒ 三轨 + 一场景全部装得下）。
const WINDOW: Size = Size::new(1440, 900);

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
    let (mut port, view) = live_window();
    let section_count = view.sections.len();
    let registry = ElementRegistry::from_view(&view);

    // ---- ① + ② 默认（编曲）视图里就在场：角色 / 名称与注册表逐字一致 ----
    let mut bounds = Vec::with_capacity(section_count);
    let mut labels = Vec::with_capacity(section_count);
    for index in 0..section_count {
        let id = format!("section-{index}-color-swatch");
        let tree = port.refresh_tree(None).expect("重抓");
        let node = tree
            .find_by_id(&id)
            .unwrap_or_else(|| panic!("`{id}` 必须在默认（编曲）视图的运行时树里"));
        assert_eq!(
            node.role.as_str(),
            ElementKind::Image.accessible_role(),
            "`{id}` 的角色必须与注册表一致"
        );
        let meta = registry
            .get(&id)
            .unwrap_or_else(|| panic!("`{id}` 必须在装配时的注册表里（[UI-TEST-001]）"));
        assert_eq!(meta.kind, ElementKind::Image);
        assert_eq!(
            node.label.as_str(),
            meta.label.as_str(),
            "屏读器读到的色标文本必须与注册表登记的逐字一致"
        );
        labels.push(node.label.clone());
        bounds.push(bounds_of(&mut port, &id));
        assert!(
            bounds[index].width > 0 && bounds[index].height > 0,
            "`{id}` 必须有可见几何: {:?}",
            bounds[index]
        );
    }

    // ---- ③ 工程驱动：合法色 / 非法回退 / 缺失回退 ----
    let expected = [
        VALID_SECTION_COLOR_HEX,
        DEFAULT_TRACK_COLOR_HEX,
        DEFAULT_TRACK_COLOR_HEX,
        DEFAULT_TRACK_COLOR_HEX,
    ];
    for (index, hex) in expected.iter().enumerate().take(section_count) {
        assert_eq!(
            labels[index],
            format!("段落色标 {hex}"),
            "段落 {index} 的色标标签必须来自投影（合法色规范化 / 非法与缺失回退）"
        );
        assert_eq!(
            view.section_color_labels()[index],
            *hex,
            "同一读数必须能在投影侧读到"
        );
    }
    let valid = swatch_sample_pixel(&port, bounds[0]);
    assert_eq!(
        valid,
        [0x22, 0xaa, 0x88],
        "合法色必须真的画出来（像素 == 投影 RGB）"
    );
    let fallback = swatch_sample_pixel(&port, bounds[1]);
    assert_eq!(
        fallback,
        [
            DEFAULT_TRACK_COLOR.red,
            DEFAULT_TRACK_COLOR.green,
            DEFAULT_TRACK_COLOR.blue
        ],
        "非法色必须回退到文档常量（像素 == 回退 RGB）"
    );
    assert_eq!(
        view.sections[0].color_rgb,
        yeban_app::bridge::RgbColor::new(0x22, 0xaa, 0x88),
        "投影字段必须与像素同源"
    );

    // ---- ④ 几何在卡片内：色标属于它那一张段落卡 ----
    for (index, swatch) in bounds.iter().copied().enumerate() {
        let card = bounds_of(&mut port, &format!("section-{index}-card"));
        let (sx, sy) = (f64::from(swatch.x), f64::from(swatch.y));
        let (sw, sh) = (f64::from(swatch.width), f64::from(swatch.height));
        let (cx, cy) = (f64::from(card.x), f64::from(card.y));
        let (cw, ch) = (f64::from(card.width), f64::from(card.height));
        assert!(
            sx >= cx && sy >= cy && sx + sw <= cx + cw && sy + sh <= cy + ch,
            "`section-{index}-color-swatch` 必须完全落在 `section-{index}-card` 里: \
             swatch={swatch:?} card={card:?}"
        );
    }

    report_line(&format!(
        "[section-color] {section_count} 个段落色标；合法色 {VALID_SECTION_COLOR_HEX} ⇒ 像素 {:?}；\
         非法 / 缺失 ⇒ 回退 {DEFAULT_TRACK_COLOR_HEX}（像素 {:?}）",
        valid, fallback
    ));
}
