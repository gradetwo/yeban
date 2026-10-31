//! ⭐ **编排行区纵向滚动** —— 滚轮把可视区外的轨道行带进窗口。
//!
//! ## 被补上的缺口
//!
//! `ui/workspace/arrangement_view.slint` 的行区此前是**固定高度**的：轨道数超过可视
//! 行数之后，第 N 行之后既看不到也点不到（真实工程 28 轨时只有前 ~8 行可达）。
//! 本片把五个行锚定层（包头 / 车道 / 剪辑 / 剪辑拖动面 / 自动化带）包进一个
//! `Flickable`，并新增注入属性 `sheet-height`（行区内容高）作为 `viewport-height`。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（30 轨 ⇒ 末行不可寻址） | 滚动前 `track-29-header` **不在控件树里**（可视区外的行不可达） |
//! | ②（滚轮 +400 ⇒ 仍在树里的每一行上移 400） | `PointerScrolled{delta_y>0}` ⇒ 两次快照交集的每个包头 y 恰好减 400，且更大索引的行进入树中 ⇒ 正方向的语义 = "看下面的行"；再滚 −400 ⇒ 快照复原 |
//! | ③（滚到底 ⇒ 末行可达，且夹紧） | 大幅滚动 ⇒ `track-29-header` 可寻址且 y 落在行区内；再滚一次快照逐位不变（`ensure_in_bound`） |
//! | ④（标尺 / 章节卡片不动） | 滚动只动行区：`arrangement-ruler` 与 `section-0-card` 的 y 逐位不变 |
//! | ⑤（内容不高 ⇒ 一位不动） | 演示工程（6 轨）滚轮 ⇒ 所有包头 y 不变（默认帧 / golden 不漂移） |
//!
//! ## 边界（不许当成已证）
//!
//! - ⛔ 不证"拖动滚动条"：本片只接**滚轮**（上游 `Flickable` 的 `PointerScrolled`
//!   在 `Cancelled` 相位是直接 set、无动画 ⇒ 判据可以立即读几何）。
//! - ⛔ 不证横向滚动：`viewport-width: self.width` ⇒ 横向恒为 0（横向仍归宿主的
//!   `scroll-x`，见账本第 199 轮"Flickable 与宿主偏移会双重计算"）。
//!
//! ## 为什么判据不直接读"末行的 y"
//!
//! 测试口的控件树**按可见性裁剪**（`[N-UI-1]` 实测：视口外的条目根本不进树 ——
//! `ElementHandle::is_visible()` 对被子孙裁剪链盖掉的元素返回 false）。所以离屏行
//! 连 `find_by_id` 都找不到，`bounds` 无从读起。判据因此：
//! - **可达性** = `track-{i}-header` 是否可寻址（进树 ⇒ 可见 ⇒ 可达）；
//! - **位移** = 滚动前后两次快照的**交集**逐行比较（只有两次都在树里的行才有 y 可读）。
//!
//! ## 夹具行数
//!
//! 30 行：可视行区 858px（窗口 900 − 行区上沿 42），内容高 1678px ⇒ 最大偏移 820px。
//! 判据②滚 400 / 判据③滚到底都必须**小于或等于**这个上限才读得出语义（否则被
//! `ensure_in_bound` 夹紧，"每行恰好上移该 delta"就退化成"上移到头"）。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 线程局部、每线程一次 ⇒ 每个建窗口的 `#[test]`
//! 只建**一个** `LivePort`（与 `tests/arrangement_ruler_seek.rs` 同款）。

use slint::LogicalPosition;
use slint::platform::WindowEvent;
use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_model::{EntityId, TrackKind, TrackV3, YebanProjectV1};
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, Size, UiTestPort as _};

const WINDOW: Size = Size::new(1440, 900);
/// 标尺带高：行区上沿 = 标尺带 y + 22（`automation::TRACK_LANE_TOP_PX = 20 + 22`）。
const RULER_HEIGHT: i32 = 22;
/// 夹具行数：演示工程 6 轨 + 24 条 MIDI 轨。
const ROWS: usize = 30;

fn project_with_24_extra_tracks() -> YebanProjectV1 {
    let mut project = demo_project();
    for index in 0..(ROWS - 6) {
        let id = EntityId::new();
        project.tracks.insert(
            id,
            TrackV3 {
                id,
                name: format!("滚轮轨道 {index}"),
                kind: TrackKind::Midi,
                ..TrackV3::default()
            },
        );
    }
    project
}

struct Rig {
    port: LivePort<MainWindow>,
    view: ViewState,
    /// 滚轮落点：首行包头内（从控件树现取 ⇒ 不假设编排行区在窗口原点）。
    wheel_point: LogicalPosition,
}

impl Rig {
    fn with_project(project: YebanProjectV1) -> Self {
        let view = ViewState::from_project(&project).expect("夹具工程必须能投影");
        let scene = DemoScene::from_view(&view);
        let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
            host::build_main_window(&view, &scene)
        })
        .expect("Tier-1 平台 + 主窗口");
        let mut rig = Self {
            port,
            view,
            wheel_point: LogicalPosition::new(0.0, 0.0),
        };
        rig.port.refresh_tree(None).expect("必须能抓控件树");
        let header = rig.bounds_of("track-0-header");
        rig.wheel_point = LogicalPosition::new(header.x as f32 + 8.0, header.y as f32 + 8.0);
        rig
    }

    fn bounds_of(&self, id: &str) -> yeban_ui_test_port::Rect {
        self.port
            .tree()
            .find_by_id(id)
            .unwrap_or_else(|| panic!("{id} 必须可寻址"))
            .bounds
            .unwrap_or_else(|| panic!("{id} 必须有几何"))
    }

    /// 行区上沿（标尺带底沿）—— 窗口内的**绝对** y。
    fn rows_top(&self) -> i32 {
        self.y_of("arrangement-ruler") + RULER_HEIGHT
    }

    fn y_of(&self, id: &str) -> i32 {
        self.bounds_of(id).y
    }

    /// 当前**在控件树里**的轨道包头：`(行号, 绝对 y)`，按行号升序。
    ///
    /// 视口外的行不进树（见模块文档），所以这是"此刻可达的行"的读数。
    fn visible_rows(&self) -> Vec<(usize, i32)> {
        (0..self.view.tracks.len())
            .filter_map(|index| {
                let id = format!("track-{index}-header");
                let node = self.port.tree().find_by_id(&id)?;
                let bounds = node.bounds?;
                Some((index, bounds.y))
            })
            .collect()
    }

    fn has_row(&self, index: usize) -> bool {
        self.port
            .tree()
            .find_by_id(&format!("track-{index}-header"))
            .is_some()
    }

    fn scroll_wheel(&mut self, delta_y: f32) {
        self.port.window().dispatch(WindowEvent::PointerScrolled {
            position: self.wheel_point,
            delta_x: 0.0,
            delta_y,
        });
        self.port.refresh_tree(None).expect("滚动后重抓控件树");
    }
}

/// ① + ②：滚轮正方向 = 看下面的行；仍在树里的每一行恰好上移该 delta。
#[test]
fn wheel_scroll_moves_rows_up_by_the_delta() {
    let mut rig = Rig::with_project(project_with_24_extra_tracks());
    assert_eq!(rig.view.tracks.len(), ROWS, "夹具必须有 {ROWS} 行");

    let before = rig.visible_rows();
    assert!(!before.is_empty(), "首行必须可达");
    assert_eq!(before[0], (0, rig.rows_top()), "未滚动时首行贴着行区上沿");
    assert!(
        !rig.has_row(ROWS - 1),
        "滚动前末行不在控件树里 —— 这正是本片要修的缺口（它不可达）"
    );

    rig.scroll_wheel(400.0);

    let after = rig.visible_rows();
    let moved: Vec<(usize, i32, i32)> = before
        .iter()
        .filter_map(|&(index, y_before)| {
            after
                .iter()
                .find(|&&(other, _)| other == index)
                .map(|&(_, y_after)| (index, y_before, y_after))
        })
        .collect();
    assert!(
        moved.len() >= 8,
        "两次快照必须有一大段交集才读得出位移（交集只有 {} 行）",
        moved.len()
    );
    for (index, y_before, y_after) in moved {
        assert_eq!(
            y_after,
            y_before - 400,
            "第 {index} 行必须整体上移 400（同一份行几何）"
        );
    }

    let deepest_before = before.last().expect("before 非空").0;
    let deepest_after = after.last().expect("滚动后必有可见行").0;
    assert!(
        deepest_after > deepest_before,
        "正方向滚动必须把更大索引的行带进控件树（{deepest_before} → {deepest_after}），否则方向语义不成立"
    );

    rig.scroll_wheel(-400.0);
    assert_eq!(
        rig.visible_rows(),
        before,
        "滚回 −400 ⇒ 快照逐位复原（没有累计漂移）"
    );
    assert!(!rig.has_row(ROWS - 1), "回到顶部 ⇒ 末行重新不可寻址");
}

/// ③：滚到底 ⇒ 末行可达；继续滚 ⇒ 夹紧（快照不变）。
#[test]
fn scrolling_to_the_end_clamps_and_reveals_the_last_row() {
    let mut rig = Rig::with_project(project_with_24_extra_tracks());

    rig.scroll_wheel(100_000.0);

    let rows_top = rig.rows_top();
    let window_height = i32::try_from(WINDOW.height).expect("窗口高可转 i32");
    assert!(!rig.has_row(0), "滚到底后首行被裁出行区（不可寻址）");
    assert!(
        rig.has_row(ROWS - 1),
        "滚到底后末行必须进入控件树（可达）—— 本片的产品面"
    );
    let last = rig.y_of(&format!("track-{}-header", ROWS - 1));
    assert!(
        (rows_top..window_height).contains(&last),
        "滚到底后末行顶沿必须落在行区内（y={last}）"
    );

    let snapshot = rig.visible_rows();
    rig.scroll_wheel(100_000.0);
    assert_eq!(rig.visible_rows(), snapshot, "到底之后不再移动（夹紧）");
}

/// ④：滚动只动行区 —— 标尺与章节卡片的 y 逐位不变。
#[test]
fn scrolling_does_not_move_the_ruler_or_the_section_cards() {
    let mut rig = Rig::with_project(project_with_24_extra_tracks());
    let ruler_before = rig.y_of("arrangement-ruler");
    let card_before = rig.y_of("section-0-card");

    rig.scroll_wheel(400.0);

    assert_eq!(
        rig.y_of("arrangement-ruler"),
        ruler_before,
        "标尺不随行区滚动"
    );
    assert_eq!(
        rig.y_of("section-0-card"),
        card_before,
        "章节卡片不随行区滚动"
    );
}

/// ⑤：内容不高出可视区（演示工程 6 轨）⇒ 滚轮一位不动。
#[test]
fn short_projects_do_not_scroll() {
    let mut rig = Rig::with_project(demo_project());

    let before: Vec<i32> = (0..rig.view.tracks.len())
        .map(|index| rig.y_of(&format!("track-{index}-header")))
        .collect();
    rig.scroll_wheel(400.0);
    let after: Vec<i32> = (0..rig.view.tracks.len())
        .map(|index| rig.y_of(&format!("track-{index}-header")))
        .collect();

    assert_eq!(before, after, "内容不高出可视区 ⇒ 偏移恒 0（默认帧不漂移）");
}
