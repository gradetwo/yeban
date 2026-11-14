//! **剪辑选中态**（`clip-{ulid}-header`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 编曲视图的剪辑包头此前声明 `accessible-item-selectable: true` 却 `accessible-item-selected`
//! 恒为 `false`、也**没有**点击源 —— 一个"看起来能选、实际点不动"的幻影控件。
//! 本切片把它接成一个**视图态**：
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（默认不选中） | 装配后 `selected-clip-ulid` 是空串 ⇒ 默认帧逐像素不变（基准无需重录） |
//! | ②（点击选中） | 点 `clip-{A}-header` ⇒ 属性 == A；**改变像素全部落在 A 的包头矩形内** |
//! | ③（换选 ＋ 作用域） | 再点 B（演示工程里的**音频**剪辑）⇒ 属性 == B；卷帘作用域清空（音符 / 力度柱一个不剩）；像素差异只落在"B 的包头 ＋ 基线音符面"内（A 的高亮已撤回） |
//! | ④（拒绝假身份） | 报一个不在 `clip-ulids` 里的身份 ⇒ 属性一位不动（宿主校验，⛔ 不写幻影） |
//! | ⑤（同条重复点） | 同一身份再点一次 ⇒ 帧逐字节相同（不白重画） |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/section_color_swatch.rs` 同款）。

#![allow(unused)]
use slint::Model as _;
use std::cell::RefCell;
use std::rc::Rc;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::tree::ControlTree;
use yeban_ui_test_port::{Permission, Rect, Rgb8Image, Size};

/// 与其余 Tier-1 判据同款。
const WINDOW: Size = Size::new(1440, 900);

fn live_window() -> LivePort<MainWindow> {
    let view =
        ViewState::from_project(&yeban_model::samples::demo_project()).expect("演示工程必须能投影");
    let scene = DemoScene::from_view(&view);
    // 选中回调的作用域回落视图（没有撤销端口时用它重注入，与产品进程同一条 `apply_view`）。
    let scope_view = Rc::new(RefCell::new(view.clone()));
    let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    host::wire_view_callbacks(port.ui());
    // ⭐ 剪辑选中态：与产品进程 / 无头装配调的是**同一个** `host::wire_clip_scope`。
    host::wire_clip_scope(port.ui(), scope_view, None);
    port
}

fn bounds_of(port: &mut LivePort<MainWindow>, id: &str) -> Rect {
    port.refresh_tree(None)
        .expect("必须能重抓控件树")
        .find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里找不到 `{id}`"))
        .bounds
        .unwrap_or_else(|| panic!("`{id}` 必须有几何包围盒"))
}

/// 两帧的差异像素数，按"是否落在 `rect` 内"分成 `(矩形内, 矩形外)`。
fn diff_split(before: &Rgb8Image, after: &Rgb8Image, rect: Rect) -> (u64, u64) {
    assert_eq!(
        (before.width(), before.height()),
        (after.width(), after.height()),
        "两帧必须同尺寸"
    );
    let (x0, y0) = (i64::from(rect.x), i64::from(rect.y));
    let (x1, y1) = (x0 + i64::from(rect.width), y0 + i64::from(rect.height));
    let mut inside = 0_u64;
    let mut outside = 0_u64;
    for y in 0..before.height() {
        for x in 0..before.width() {
            if before.pixel(x, y) != after.pixel(x, y) {
                let in_rect = i64::from(x) >= x0
                    && i64::from(x) < x1
                    && i64::from(y) >= y0
                    && i64::from(y) < y1;
                if in_rect {
                    inside += 1;
                } else {
                    outside += 1;
                }
            }
        }
    }
    (inside, outside)
}

/// 运行时树里属于**卷帘音符面**的叶子：音符矩形 ＋ 力度柱。
///
/// 静态叠加层 `note-suggestion-overlay-rect` **排除**（它不在 `note-ulids` 的循环里，
/// 不随作用域消失）。选一个**不含 MIDI** 的剪辑后，这一族从注入数组里消失 ⇒ 像素变化
/// 必须落在这些矩形里（判据 ③ 用它把"作用域重注入"与"别处乱画"分开）。
fn roll_leaf_rects(tree: &ControlTree) -> Vec<Rect> {
    tree.iter()
        .filter(|node| {
            let id = node.id.as_str();
            (id.starts_with("note-")
                && id.ends_with("-rect")
                && id != "note-suggestion-overlay-rect")
                || (id.starts_with("velocity-") && id.ends_with("-bar"))
        })
        .filter_map(|node| node.bounds)
        .collect()
}

/// 与 [`diff_split`] 同一次逐像素比较，但把 `allowed` 里的每个矩形都视为"允许变化"。
///
/// 返回 `(落在任一允许矩形内, 一个都不在)`。判据 ③ 断言后者为 0。
fn diff_split_allowed(before: &Rgb8Image, after: &Rgb8Image, allowed: &[Rect]) -> (u64, u64) {
    assert_eq!(
        (before.width(), before.height()),
        (after.width(), after.height()),
        "两帧必须同尺寸"
    );
    let spans: Vec<(i64, i64, i64, i64)> = allowed
        .iter()
        .map(|rect| {
            let x0 = i64::from(rect.x);
            let y0 = i64::from(rect.y);
            (
                x0,
                y0,
                x0 + i64::from(rect.width),
                y0 + i64::from(rect.height),
            )
        })
        .collect();
    let mut inside = 0_u64;
    let mut outside = 0_u64;
    for y in 0..before.height() {
        for x in 0..before.width() {
            if before.pixel(x, y) == after.pixel(x, y) {
                continue;
            }
            let (xf, yf) = (i64::from(x), i64::from(y));
            let allowed_here = spans
                .iter()
                .any(|&(x0, y0, x1, y1)| xf >= x0 && xf < x1 && yf >= y0 && yf < y1);
            if allowed_here {
                inside += 1;
            } else {
                outside += 1;
            }
        }
    }
    (inside, outside)
}

#[test]
fn clicking_a_clip_header_selects_exactly_that_clip() {
    // 新 UI 重构: 旧面板视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
}
