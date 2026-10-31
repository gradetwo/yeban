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
//! | ①（20 轨 ⇒ 末行在窗口外） | 滚动前 `track-19-header` 的绝对 y ≥ 窗口高（没有滚动就够不着） |
//! | ②（滚轮 +400 ⇒ 每行上移 400） | `PointerScrolled{delta_y>0}` ⇒ 包头 y 恰好减 400 ⇒ 正方向的语义 = "看下面的行" |
//! | ③（滚到底 ⇒ 末行进入可视区，且夹紧） | 大幅滚动 ⇒ 末行 y ∈ [42, 窗口高−行高]；再滚一次位置不变（`ensure_in_bound`） |
//! | ④（标尺 / 章节卡片不动） | 滚动只动行区：`arrangement-ruler` 与 `section-0-card` 的 y 逐位不变 |
//! | ⑤（内容不高 ⇒ 一位不动） | 演示工程（6 轨）滚轮 ⇒ 所有包头 y 不变（默认帧 / golden 不漂移） |
//!
//! ## 边界（不许当成已证）
//!
//! - ⛔ 不证"拖动滚动条"：本片只接**滚轮**（上游 `Flickable` 的 `PointerScrolled`
//!   在 `Cancelled` 相位是直接 set、无动画 ⇒ 判据可以立即读几何）。
//! - ⛔ 不证横向滚动：`viewport-width: self.width` ⇒ 横向恒为 0（横向仍归宿主的
//!   `scroll-x`，见账本第 199 轮"Flickable 与宿主偏移会双重计算"）。
//! - 控件树的 `bounds` 是**元素几何**（绝对位置 + 尺寸），不按祖先裁剪 ⇒ 判据用
//!   "y 落在窗口里"表达"可达"，而不是"节点是否存在"。
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

/// 演示工程（6 轨）+ 14 条 MIDI 轨 = 20 行（默认行槽 56px ⇒ 内容高 ≈ 1118px）。
fn project_with_19_extra_tracks() -> YebanProjectV1 {
    let mut project = demo_project();
    for index in 0..14 {
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
        self.port
            .tree()
            .find_by_id(id)
            .unwrap_or_else(|| panic!("{id} 必须可寻址"))
            .bounds
            .unwrap_or_else(|| panic!("{id} 必须有几何"))
            .y
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

/// ① + ②：滚轮正方向 = 看下面的行；每行恰好上移一个 delta。
#[test]
fn wheel_scroll_moves_rows_up_by_the_delta() {
    let mut rig = Rig::with_project(project_with_19_extra_tracks());
    assert_eq!(rig.view.tracks.len(), 20, "夹具必须有 20 行");

    let top_before = rig.y_of("track-0-header");
    let last_before = rig.y_of("track-19-header");
    assert_eq!(top_before, rig.rows_top(), "未滚动时首行贴着行区上沿");
    assert!(
        last_before >= i32::try_from(WINDOW.height).expect("窗口高可转 i32"),
        "滚动前末行必须在窗口外（y={last_before}）—— 这正是本片要修的缺口"
    );

    rig.scroll_wheel(400.0);

    assert_eq!(
        rig.y_of("track-0-header"),
        top_before - 400,
        "滚轮正方向 ⇒ 内容上移（看下面的行）"
    );
    assert_eq!(
        rig.y_of("track-19-header"),
        last_before - 400,
        "所有行同一位移（同一份行几何）"
    );
}

/// ③：滚到底 ⇒ 末行进入可视区；继续滚 ⇒ 夹紧（位置不变）。
#[test]
fn scrolling_to_the_end_clamps_and_reveals_the_last_row() {
    let mut rig = Rig::with_project(project_with_19_extra_tracks());

    rig.scroll_wheel(100_000.0);

    let rows_top = rig.rows_top();
    let top = rig.y_of("track-0-header");
    let last = rig.y_of("track-19-header");
    let window_height = i32::try_from(WINDOW.height).expect("窗口高可转 i32");
    assert!(
        top < rows_top,
        "滚到底后首行应被裁到行区上沿之上（y={top}）"
    );
    assert!(
        (rows_top..window_height).contains(&last),
        "滚到底后末行顶沿必须落在窗口内（y={last}）"
    );

    rig.scroll_wheel(100_000.0);
    assert_eq!(rig.y_of("track-0-header"), top, "到底之后不再移动（夹紧）");
    assert_eq!(
        rig.y_of("track-19-header"),
        last,
        "到底之后不再移动（夹紧）"
    );
}

/// ④：滚动只动行区 —— 标尺与章节卡片的 y 逐位不变。
#[test]
fn scrolling_does_not_move_the_ruler_or_the_section_cards() {
    let mut rig = Rig::with_project(project_with_19_extra_tracks());
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
