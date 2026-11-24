//! **段落色标**（`section-{i}-color-swatch`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 编曲视图的曲式段落带（T-FIX-152，18px）每一段的底色由工程文档的
//! `SectionV3::color` 驱动 —— 合法色按 CSS 惯例解析并规范化，非法 / 缺失回退到
//! 文档常量。段落带的底色、投影的 `color_hex` 与像素三处读的是同一个读数
//! （`parse_hex_color` 只有一处实现）。
//!
//! ⭐ **色标节点已死**：旧编曲视图的 `section-{i}-card` / `section-{i}-color-swatch`
//! （3px 竖条）随死文件 `ui/workspace/arrangement_view.slint` 一起删了；新编曲视图
//! （`ui/arrangement.slint`）的段落带没有 `accessible-id`、没有竖条节点 ——
//! **整段底色就是色标**（`background: root.section-colors[section-index]`）。
//! 本判据改钉：① 活树里没有旧节点；② 段落带底色绑定到 `section-colors`（源码）；
//! ③ 注册表标签与投影逐字一致；④ 像素采样段落带（合法色 / 非法回退）；⑤ 各段带
//! 互不重叠、落在画布内。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（旧节点不在树里） | 编曲视图下 `section-{i}-card` / `section-{i}-color-swatch` 都不在运行时树里 |
//! | ②（底色绑定） | `arrangement.slint` 的段落带 `background` 就是 `section-colors[section-index]` |
//! | ③（工程驱动） | 合法色 `#22aa88` ⇒ 投影 `#22AA88`、注册表标签 `#22AA88`、像素逐位相等；非法 / 缺失 ⇒ 回退常量 |
//! | ④（几何在画布内） | 各段带的投影矩形互不重叠、完全落在编曲画布内 |
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

fn band_sample_pixel(port: &LivePort<yeban_app::ui::MainWindow>, x: u32, y: u32) -> [u8; 3] {
    let image = port.window().capture().expect("Tier-1 截图");
    image
        .pixel(x, y)
        .unwrap_or_else(|| panic!("采样点 ({x}, {y}) 必须在 {WINDOW:?} 帧内"))
}

#[test]
fn section_color_swatches_carry_the_projected_color() {
    let (mut port, view) = live_window();
    let section_count = view.sections.len();
    let registry = ElementRegistry::from_view(&view);

    // ---- ① 旧节点不在树里（活树里没有段落卡/色标节点）----
    let tree = port.refresh_tree(None).expect("重抓");
    for index in 0..section_count {
        for id in [
            format!("section-{index}-card"),
            format!("section-{index}-color-swatch"),
        ] {
            assert!(
                tree.find_by_id(&id).is_none(),
                "活树里不许再有 `{id}`（它只活在死文件 `ui/workspace/arrangement_view.slint` 里）"
            );
        }
    }

    // ---- ② 段落带底色绑定到 `section-colors`（源码）----
    let arrangement = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/arrangement.slint"),
    )
    .expect("读 arrangement.slint");
    assert!(
        arrangement.contains(
            "background: section-index < root.section-colors.length ? root.section-colors[section-index] : Theme.accent;"
        ),
        "段落带的底色必须是 `root.section-colors[section-index]`（整段底色就是色标）"
    );

    // ---- ③ 工程驱动：注册表标签与投影逐字一致 ----
    let expected = [
        VALID_SECTION_COLOR_HEX,
        DEFAULT_TRACK_COLOR_HEX,
        DEFAULT_TRACK_COLOR_HEX,
        DEFAULT_TRACK_COLOR_HEX,
    ];
    for (index, hex) in expected.iter().enumerate().take(section_count) {
        let meta = registry
            .get(&format!("section-{index}-color-swatch"))
            .unwrap_or_else(|| {
                panic!("`section-{index}-color-swatch` 必须在装配时的注册表里（[UI-TEST-001]）")
            });
        assert_eq!(meta.kind, ElementKind::Image);
        assert_eq!(
            meta.label,
            format!("段落色标 {hex}"),
            "段落 {index} 的注册表标签必须来自投影（合法色规范化 / 非法与缺失回退）"
        );
        assert_eq!(
            view.section_color_labels()[index],
            *hex,
            "同一读数必须能在投影侧读到"
        );
    }

    // ---- ④ 像素：采样段落带（合法色 / 非法回退）----
    // 段落带几何（窗口坐标）：画布顶沿 + 泳道区顶沿偏移（和弦行 22px + 间隙）+
    // 18px 带高中点；x = 时间轴原点（轨道头右沿 + 5px 分隔条）+ 该段投影列 +
    // 段宽的 3/4（段名文本左对齐在 6px 处，3/4 点一定是纯底色）。
    let canvas = tree
        .find_by_id("workspace-arrangement-canvas")
        .expect("控件树里必须有编曲画布")
        .bounds
        .expect("画布必须有几何");
    let header = tree
        .find_by_id("track-0-header")
        .expect("控件树里必须有轨道头")
        .bounds
        .expect("轨道头必须有几何");
    let lanes = tree
        .find_by_id("arr-lanes-flickable")
        .expect("控件树里必须有泳道区")
        .bounds
        .expect("泳道区必须有几何");
    let origin_x = header.x + header.width as i32 + 5;
    let band_y = canvas.y + (lanes.y - canvas.y) + 9;
    let sample = |index: usize| -> [u8; 3] {
        let section = &view.sections[index];
        let x = origin_x + (section.x + section.width * 0.75) as i32;
        band_sample_pixel(
            &port,
            u32::try_from(x).expect("采样点在窗口内"),
            u32::try_from(band_y).expect("采样点在窗口内"),
        )
    };
    let valid = sample(0);
    assert_eq!(
        valid,
        [0x22, 0xaa, 0x88],
        "合法色必须真的画出来（像素 == 投影 RGB）"
    );
    let fallback = sample(1);
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

    // ---- ⑤ 几何：各段带互不重叠、完全落在编曲画布内 ----
    let mut prev_end: Option<f32> = None;
    for (index, section) in view.sections.iter().enumerate() {
        assert!(
            section.width > 0.0,
            "段落 {index} 的带宽必须是正数（实得 {}）",
            section.width
        );
        if let Some(end) = prev_end {
            assert!(
                section.x >= end,
                "段落 {index} 的带必须从上一段右沿之后开始（x={}，上一段右沿={end}）",
                section.x
            );
        }
        prev_end = Some(section.x + section.width);
        let band_left = origin_x as f32 + section.x;
        let band_right = band_left + section.width;
        assert!(
            band_left >= canvas.x as f32 && band_right <= canvas.x as f32 + canvas.width as f32,
            "段落 {index} 的带必须完全落在编曲画布内（{band_left}..{band_right}，画布 {}..{}）",
            canvas.x,
            canvas.x as f32 + canvas.width as f32
        );
    }

    report_line(&format!(
        "[section-color] {section_count} 个段落带；合法色 {VALID_SECTION_COLOR_HEX} ⇒ 像素 {valid:?}；\
         非法 / 缺失 ⇒ 回退 {DEFAULT_TRACK_COLOR_HEX}（像素 {fallback:?}）",
    ));
}
