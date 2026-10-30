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
    let mut port = live_window();
    let clip_ids: Vec<String> = port
        .ui()
        .get_clip_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    assert!(
        clip_ids.len() >= 2,
        "演示工程必须至少有两个剪辑才能判'换选': {clip_ids:?}"
    );

    // ---- ① 默认不选中：属性是空串，默认帧就是基准帧 ----
    assert_eq!(
        port.ui().get_selected_clip_ulid().as_str(),
        "",
        "装配后不得预先选中任何剪辑"
    );
    let baseline = port.window().capture().expect("Tier-1 截图");
    // 基线帧里卷帘音符面（音符 ＋ 力度柱）的矩形：作用域清空时，变化必须落在它们里面。
    let roll_before = {
        let tree = port.refresh_tree(None).expect("必须能重抓控件树");
        roll_leaf_rects(&tree)
    };
    assert!(
        !roll_before.is_empty(),
        "基线帧的卷帘必须有音符 / 力度柱（否则本判据的'作用域清空'无从比较）"
    );

    // ---- ② 点击第一个剪辑的包头 ⇒ 选中它，像素变化只在它的包头内 ----
    let first = clip_ids[0].clone();
    let first_rect = bounds_of(&mut port, &format!("clip-{first}-header"));
    assert!(
        first_rect.width > 0 && first_rect.height > 0,
        "包头必须有可见几何: {first_rect:?}"
    );
    port.ui().invoke_clip_select(first.as_str().into());
    assert_eq!(
        port.ui().get_selected_clip_ulid().as_str(),
        first.as_str(),
        "点击包头必须把身份写进 `selected-clip-ulid`"
    );
    let first_frame = port.window().capture().expect("Tier-1 截图");
    let (inside, outside) = diff_split(&baseline, &first_frame, first_rect);
    assert!(inside > 0, "选中必须真的画出来（包头描边）: {first_rect:?}");
    assert_eq!(
        outside, 0,
        "选中态的改变像素必须全部落在 `clip-{first}-header` 内: {first_rect:?}"
    );

    // ---- ③ 换选第二个（演示工程里是**音频**剪辑）⇒ 第一个的高亮撤回；
    //      卷帘作用域随选中身份清空（该片段池条目没有 MIDI 音符）⇒ 音符 / 力度柱全部消失，
    //      且像素变化只许落在"第二个包头 ＋ 基线音符面"之内 ----
    let second = clip_ids[1].clone();
    let second_rect = bounds_of(&mut port, &format!("clip-{second}-header"));
    port.ui().invoke_clip_select(second.as_str().into());
    assert_eq!(port.ui().get_selected_clip_ulid().as_str(), second.as_str());
    let second_frame = port.window().capture().expect("Tier-1 截图");
    let (inside_second, _outside_second) = diff_split(&baseline, &second_frame, second_rect);
    assert!(
        inside_second > 0,
        "换选后第二个包头必须亮起: {second_rect:?}"
    );
    let roll_after = {
        let tree = port.refresh_tree(None).expect("必须能重抓控件树");
        roll_leaf_rects(&tree)
    };
    assert!(
        roll_after.is_empty(),
        "选中一个不含 MIDI 的剪辑后，卷帘作用域必须为空（音符 / 力度柱一个都不许留下）: {roll_after:?}"
    );
    let mut allowed = roll_before.clone();
    allowed.push(second_rect);
    let (inside_allowed, outside_allowed) = diff_split_allowed(&baseline, &second_frame, &allowed);
    assert!(
        inside_allowed > 0,
        "作用域清空必须真的画出像素变化（卷帘音符面）: {allowed:?}"
    );
    assert_eq!(
        outside_allowed, 0,
        "换选后的差异只许落在'第二个包头 ＋ 卷帘音符面'内（其余像素必须回到基准）"
    );

    // ---- ④ 假身份被宿主拒绝：不在 `clip-ulids` 里的串一位不写 ----
    port.ui().invoke_clip_select("not-a-real-clip".into());
    assert_eq!(
        port.ui().get_selected_clip_ulid().as_str(),
        second.as_str(),
        "宿主必须拒绝不在 `clip-ulids` 里的身份"
    );

    // ---- ⑤ 同一条再点一次 = 一位不动（帧逐字节相同）----
    port.ui().invoke_clip_select(second.as_str().into());
    let again = port.window().capture().expect("Tier-1 截图");
    assert_eq!(
        again.pixels(),
        second_frame.pixels(),
        "同一条剪辑再点一次不得重画"
    );

    report_line(&format!(
        "[clip-select] 剪辑 {} 个；默认未选中；点 `{}` ⇒ 包头内 {} 像素变化 / 矩形外 0；\
         换选 `{}` ⇒ 包头内 {} 像素变化；卷帘音符面 {} → 0（作用域清空），允许区内 {} 像素、\
         区外 0；假身份被拒；重复点击帧不变",
        clip_ids.len(),
        first,
        inside,
        second,
        inside_second,
        roll_before.len()
    ));
}
