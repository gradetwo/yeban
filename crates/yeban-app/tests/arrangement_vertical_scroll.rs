//! ⭐ **编排行区纵向滚动** —— 滚轮把可视区外的轨道行带进窗口。
//!
//! ## 被补上的缺口
//!
//! `ui/workspace/arrangement_view.slint` 的行区此前是**固定高度**的：轨道数超过可视
//! 行数之后，第 N 行之后既看不到也点不到（真实工程 28 轨时只有前 ~8 行可达）。
//! 本片把五个行锚定层（包头 / 车道 / 剪辑 / 剪辑拖动面 / 自动化带）包进一个
//! `Flickable`，并新增注入属性 `sheet-height`（行区内容高）作为 `content-height`。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（30 轨 ⇒ 末行不可寻址） | 滚动前 `track-29-header` **不在控件树里**（可视区外的行不可达） |
//! | ②（向下滚两个行距 ⇒ 仍在树里的每一行上移该距离） | `PointerScrolled{delta_y=-2×行距}` ⇒ 两次快照交集的每个包头 y 恰好减该距离，且更大索引的行进入树中；再向上滚同样距离 ⇒ 快照复原（⚠ 符号：上游把 `content-y` 夹在 `[h−ch, 0]` 且 delta **不取反** ⇒ 静止位上正 `delta_y` 被夹回 0，什么也不动） |
//! | ③（滚到底 ⇒ 末行可达，且夹紧） | 大幅滚动 ⇒ `track-29-header` 可寻址且 y 落在行区内；再滚一次快照逐位不变（`ensure_in_bound`） |
//! | ④（标尺 / 章节卡片不动） | 滚动只动行区：`arrangement-ruler` 与 `section-0-card` 的 y 逐位不变 |
//! | ⑤（内容不高 ⇒ 一位不动） | 演示工程（6 轨）滚轮 ⇒ 所有包头 y 不变（默认帧 / golden 不漂移） |
//! | ⑥（前置：内容高注入到位） | 主窗口的 `arrangement-sheet-height` = 夹具最后一行底沿 − 行区上沿，且**高出窗口** —— 没有它，上游 `Flickable` 的 `is_allowed_scroll_direction` 恒 false ⇒ 滚轮被**静默忽略并转发给父级**（`41208446082` 那票"一位都不动"的红就有两个独立原因：内容高别名 + 滚轮符号，见账本 §4） |
//!
//! ## 边界（不许当成已证）
//!
//! - ⛔ 不证"拖动滚动条"：本片只接**滚轮**（上游 `Flickable` 的 `PointerScrolled`
//!   在 `Cancelled` 相位是直接 set、无动画 ⇒ 判据可以立即读几何）。
//! - ⛔ 不证横向滚动：`content-width: self.width` ⇒ 横向恒为 0（横向仍归宿主的
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
//! ## 夹具行数与"为什么不写死 delta"
//!
//! 30 行，内容高 1678px（行区上沿在窗口内 y=90）。**行区视口高不是窗口高减去行区上沿** ——
//! 编排区下面还有底部控制台，实测 1440×900 档只有约 456px（run `41247717524`：固定 400px 的
//! 探针吃满整个视口 ⇒ 两次快照的交集只剩 1 行，判据②红）。所以判据②的位移量取
//! **两个行距**（行距从同一份投影的树里现读），并且先把"滚动前行区里至少有 5 行"钉成前置条件。
//! 判据③滚到底用 `100_000`，靠 `ensure_in_bound` 夹紧来证"到底不再动"，与视口高无关。
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

    /// 宿主注入到主窗口的行区内容高（`.slint` 拿它当 `content-height` 的原料）。
    ///
    /// ⚠ 这里读的是**注入属性**，不是 `content-height` 本身：`.slint` 侧必须写规范名
    /// `content-*`，不能写 `@deprecated` 别名 `viewport-*` —— 上游 `flickable` 编译
    /// pass 跑在别名折叠**之前**，它按名字检查 `content-height` 有没有显式绑定来决定
    /// 是否自动生成内容高（自动值 = 容器自身高）；绑定挂在别名上时它读不到，于是装上
    /// 自动绑定，我们的显式值在合并时可能被丢弃 ⇒ 内容高恒等于可视高 ⇒ 滚轮恒被忽略。
    fn sheet_height(&self) -> f32 {
        self.port.ui().get_arrangement_sheet_height()
    }

    /// 夹具应有的内容高：最后一行底沿 − 行区上沿（与 `bridge::track_sheet_height` 同式）。
    fn expected_sheet_height(&self) -> f32 {
        let last = self.view.tracks.last().expect("夹具至少一行");
        (last.y + last.height) - yeban_app::automation::TRACK_LANE_TOP_PX
    }

    /// 判据⑥：滚轮语义的前置条件 —— 内容高注入到位，且**高出窗口**。
    fn assert_content_overflows(&self) {
        let injected = self.assert_injected();
        let window_height = WINDOW.height as f32;
        assert!(
            injected > window_height,
            "夹具内容高 {injected} 必须高出窗口 {window_height}，否则滚动容器不成立"
        );
    }

    /// 判据⑤的前半：内容高注入到位，且**不高出窗口** ⇒ 偏移恒 0。
    fn assert_content_fits(&self) {
        let injected = self.assert_injected();
        let window_height = WINDOW.height as f32;
        assert!(
            injected < window_height,
            "演示工程内容高 {injected} 必须不高出窗口 {window_height}，否则判据⑤证的不是'不溢出'"
        );
    }

    /// 注入值必须等于夹具的行几何末沿（同一份投影，不是界面重算的）。
    fn assert_injected(&self) -> f32 {
        let injected = self.sheet_height();
        let expected = self.expected_sheet_height();
        assert!(
            (injected - expected).abs() < 1.0,
            "宿主必须将行区内容高注入主窗口（注入 {injected}，夹具期望 {expected}）"
        );
        injected
    }

    /// 向下滚 `amount` 像素（= 看下面的行）。
    ///
    /// ⚠ 上游把 `content-y` 夹在 `[视口高 − 内容高, 0]`（`items/flickable.rs` 的
    /// `ensure_in_bound`：`min = h - ch`、`max = 0`），而 `new_pos = current + delta`
    /// **不取反** ⇒ 静止位（`content-y = 0`）上**正的 `delta_y` 被夹回 0**、什么也不动。
    /// "看下面的行"因此必须送**负** delta（这也是真实滚轮向下的符号）。
    /// 判据因此用两个具名助手，把符号留在一个地方。
    fn scroll_rows_down(&mut self, amount: i32) {
        self.dispatch_wheel(-amount);
    }

    /// 向上滚 `amount` 像素（= 回到上面的行）。
    fn scroll_rows_up(&mut self, amount: i32) {
        self.dispatch_wheel(amount);
    }

    fn dispatch_wheel(&mut self, delta_y: i32) {
        self.port.window().dispatch(WindowEvent::PointerScrolled {
            position: self.wheel_point,
            delta_x: 0.0,
            // ⚠ `as f32`：i32→f32 有损 ⇒ 标准库不给 `From<i32> for f32`；像素级 delta 无关判据。
            delta_y: delta_y as f32,
        });
        self.port.refresh_tree(None).expect("滚动后重抓控件树");
    }
}

/// ① + ②：向下滚轮 = 看下面的行；仍在树里的每一行恰好上移该距离。
#[test]
fn wheel_scroll_moves_rows_up_by_the_delta() {
    let mut rig = Rig::with_project(project_with_24_extra_tracks());
    assert_eq!(rig.view.tracks.len(), ROWS, "夹具必须有 {ROWS} 行");
    rig.assert_content_overflows();

    let before = rig.visible_rows();
    assert!(
        before.len() >= 5,
        "滚动前的视口里必须有足够的行（实测 {before_len} 行）—— 少于 5 行就读不出位移样本",
        before_len = before.len()
    );
    assert_eq!(before[0], (0, rig.rows_top()), "未滚动时首行贴着行区上沿");
    assert!(
        !rig.has_row(ROWS - 1),
        "滚动前末行不在控件树里 —— 这正是本片要修的缺口（它不可达）"
    );

    // 位移量**不硬编码**：行区视口高由窗口布局决定（编排区被底部控制台分掉，实测 1440×900 档
    // 只有约 456px ⇒ 一个固定 400px 的探针几乎吃满整个视口，交集退化到 1 行）。
    // 取"两个行距"—— 行距从同一份投影的树里现读，判据仍然精确到像素。
    let pitch = before[1].1 - before[0].1;
    assert!(pitch > 0, "行距必须是正数（读到 {pitch}）");
    let delta = pitch * 2;

    rig.scroll_rows_down(delta);

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
        moved.len() >= 3,
        "两次快照必须有一段交集才读得出位移（交集只有 {} 行，delta={delta}）",
        moved.len()
    );
    for (index, y_before, y_after) in moved {
        assert_eq!(
            y_after,
            y_before - delta,
            "第 {index} 行必须整体上移 {delta}（= 两个行距，同一份行几何）"
        );
    }

    let deepest_before = before.last().expect("before 非空").0;
    let deepest_after = after.last().expect("滚动后必有可见行").0;
    assert!(
        deepest_after > deepest_before,
        "向下滚动必须把更大索引的行带进控件树（{deepest_before} → {deepest_after}），否则方向语义不成立"
    );

    rig.scroll_rows_up(delta);
    assert_eq!(
        rig.visible_rows(),
        before,
        "向上滚回 {delta} ⇒ 快照逐位复原（没有累计漂移）"
    );
    assert!(!rig.has_row(ROWS - 1), "回到顶部 ⇒ 末行重新不可寻址");
}

/// ③：滚到底 ⇒ 末行可达；继续滚 ⇒ 夹紧（快照不变）。
#[test]
fn scrolling_to_the_end_clamps_and_reveals_the_last_row() {
    let mut rig = Rig::with_project(project_with_24_extra_tracks());
    rig.assert_content_overflows();

    rig.scroll_rows_down(100_000);

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
    rig.scroll_rows_down(100_000);
    assert_eq!(rig.visible_rows(), snapshot, "到底之后不再移动（夹紧）");
}

/// ④：滚动只动行区 —— 标尺与章节卡片的 y 逐位不变。
#[test]
fn scrolling_does_not_move_the_ruler_or_the_section_cards() {
    let mut rig = Rig::with_project(project_with_24_extra_tracks());
    rig.assert_content_overflows();
    let ruler_before = rig.y_of("arrangement-ruler");
    let card_before = rig.y_of("section-0-card");

    rig.scroll_rows_down(400);

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
    rig.assert_content_fits();

    let before: Vec<i32> = (0..rig.view.tracks.len())
        .map(|index| rig.y_of(&format!("track-{index}-header")))
        .collect();
    rig.scroll_rows_down(400);
    let after: Vec<i32> = (0..rig.view.tracks.len())
        .map(|index| rig.y_of(&format!("track-{index}-header")))
        .collect();

    assert_eq!(before, after, "内容不高出可视区 ⇒ 偏移恒 0（默认帧不漂移）");
}
