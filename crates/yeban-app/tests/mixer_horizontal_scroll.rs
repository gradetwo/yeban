//! ⭐ **调音台通道条横向滚动** —— 滚轮把可视区外的通道条带进窗口。
//!
//! ## 被补上的缺口
//!
//! `ui/console/mixer_console.slint` 把通道条按 `8 + 104px * i` 平铺，三块侧面板（导出 MIDI /
//! 打开工程 / 新建工程 + 素材池）的位置同样跟着轨道数 `n` 走。轨道一多，第 K 条之后的通道条
//! 与全部侧面板就冲出窗口 —— `zimmer-ui-mode-report.md` §5.1 / §7 的实测：1440 档下调音台树里
//! 只有 `mixer-master-*` 加 `track-0`..`track-13`（"第 14 条通道起于 1456 像素"），报告把它登记为
//! **测试能力缺口 N-UI-1**，并注明"第 14 条以后的通道对真实用户是否可达仍未验证"。
//! 本片把横向流程里的全部（侧面板 + 通道条）包进一个 `Flickable`，主控通道条留在容器外
//! （它钉在 `root.width - 112`，属于视口而不是内容）。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（15 轨 ⇒ 末条不可寻址） | 滚动前 `track-14-channel-strip` **不在控件树里**（视口外的条目不进树 ⇒ 不可达） |
//! | ②（向右滚 400 ⇒ 仍在树里的每一条左移 400） | `PointerScrolled{delta_x=-400}` ⇒ 两次快照交集的每个通道条 x 恰好减 400，且更大索引的通道条进入树中；再向左滚 400 ⇒ 快照复原 |
//! | ③（滚到底 ⇒ 末条可达，且夹紧） | 大幅滚动 ⇒ `track-14-channel-strip` 与 `track-14-fader` 可寻址且左沿落在视口内；`track-0` 被裁出；再滚一次快照逐位不变（`ensure_in_bound`） |
//! | ④（侧面板回来） | 滚动前 `module-export-midi-button` 不可达 ⇒ 滚到底可达（报告里"跟着 n 走"的那批控件同样被救回） |
//! | ⑤（纵向滚轮不动混音台） | `delta_y=-400` ⇒ 快照逐位不变（`content-height: self.height` ⇒ 上游 `is_allowed_scroll_direction` 为假） |
//! | ⑥（主控通道条不滚） | `mixer-master-strip` 的 x 在滚动前后与到底时逐位相同（它在容器**外面**） |
//! | ⑦（前置：内容宽注入到位） | 容器内容宽（`8 + 104n + 面板宽`，与 `.slint` 同一组常量）必须**高出视口宽**且最大偏移 > 单次滚动量 —— 没有它，上游 `Flickable` 的滚轮分支恒假 ⇒ 滚轮被**静默忽略并转发给父级**（编排行区那票红的根因之一，见 `docs/ledger/app-zimmer-ui-notes.md` §4） |
//! | ⑧（内容不宽 ⇒ 一位不动） | 演示工程（6 轨）滚轮 ⇒ 快照逐位不变（默认帧 / golden / `tests/pane_no_overlap.rs` 的 73 控件预算不漂移） |
//!
//! ## 边界（不许当成已证）
//!
//! - ⛔ 不证"拖动滚动条"：容器是裸 `Flickable`，上游**不生成滚动条**（`ScrollView` 才会加）
//!   ⇒ 本片只接滚轮。
//! - ⛔ 不证"普通鼠标滚轮能横滚"：判据⑤证的恰恰相反 —— 只有**横向 delta**（触控板左右扫，
//!   或非 macOS 下按住 Shift 的滚轮：`items/flickable.rs` 的 `scroll_delta` 仅在 Shift 且
//!   非 macOS 时交换轴）才滚混音台。键盘与拖动路径本片不做。
//! - ⛔ 不证"纵向该滚混音台"：纵向照旧是宿主的（卷帘横向 `scroll-x` 与编排行区滚动各归各的容器）。
//!
//! ## 为什么判据不直接读"末条的 x"
//!
//! 测试口的控件树**按可见性裁剪**（N-UI-1 实测），离屏通道条连 `find_by_id` 都找不到。判据因此：
//! - **可达性** = `track-{i}-channel-strip` 是否可寻址（进树 ⇒ 未被裁 ⇒ 可达）；
//! - **位移** = 滚动前后两次快照的**交集**逐条比较（只有两次都在树里的条才有 x 可读）；
//! - 视口边界全部从树里现取（`mixer-console` 的包围盒），⛔ 不假设控制台在窗口原点。
//!
//! ## 夹具条数
//!
//! 15 条：1440 档下视口宽 `1440 − 112 = 1328`，内容宽 `8 + 104×15 + 438 = 2006` ⇒ 最大偏移
//! 678px。单次滚动 400px 必须**小于**这个上限才读得出语义（否则被 `ensure_in_bound` 夹紧）。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 线程局部、每线程一次 ⇒ 每个建窗口的 `#[test]` 只建**一个**
//! `LivePort`（与 `tests/arrangement_vertical_scroll.rs` 同款）。

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
/// 底部控制台的混音台标签号（`console_tabs.slint`：0 卷帘 / 1 调音台 / 2 设备面板）。
/// 通道条只在 Tab 1 进入运行时树（`visible` 为假的分支不进树）。
const MIXER_TAB: i32 = 1;
/// 夹具通道数：演示工程 6 条 + 9 条 MIDI 轨。
const STRIPS: usize = 15;
/// 通道条几何 —— 与 `mixer_console.slint` 的 `for` 循环共用同一组常量。
const STRIP_PITCH: i32 = 104;
/// `Tokens.space-4`：首条通道条（及导出面板）的左沿。
const STRIP_LEFT: i32 = 8;
/// 主控通道条栏位宽（`root.width - 112`）—— 也就是滚动视口要给视口让出的那一段。
const MASTER_GUTTER: i32 = 112;
/// 侧面板占宽：宽档三块并排 `216 + 6 + 216`，窄档竖排只剩一列 `216`。
const PANELS_WIDE: i32 = 438;
const PANELS_NARROW: i32 = 216;
/// 窄档断点（`mixer_console.slint` 的 `narrow: root.width < 1182px`）。
const NARROW_BREAKPOINT: i32 = 1182;
/// 单次滚动量 —— 判据②用它读"恰好位移"，所以必须小于夹具的最大偏移（判据⑦）。
const SCROLL: i32 = 400;

fn project_with_9_extra_tracks() -> YebanProjectV1 {
    let mut project = demo_project();
    for index in 0..(STRIPS - 6) {
        let id = EntityId::new();
        project.tracks.insert(
            id,
            TrackV3 {
                id,
                name: format!("横滚通道 {index}"),
                kind: TrackKind::Midi,
                ..TrackV3::default()
            },
        );
    }
    project
}

struct Rig {
    port: LivePort<MainWindow>,
    strips: usize,
    /// 滚轮落点：首条通道条内（从控件树现取 ⇒ 不假设混音台在窗口原点）。
    ///
    /// 落点压在条头空白处；即便命中条内的 `TouchArea`，滚轮也照旧向上冒泡到容器
    /// （仓库内没有任何 `scroll-event` 处理器会先截走它）。
    wheel_point: LogicalPosition,
}

impl Rig {
    fn with_project(project: YebanProjectV1) -> Self {
        let view = ViewState::from_project(&project).expect("夹具工程必须能投影");
        let scene = DemoScene::from_view(&view);
        let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
            host::build_main_window_with_console_tab(&view, &scene, MIXER_TAB)
        })
        .expect("Tier-1 平台 + 主窗口（混音台标签）");
        let mut rig = Self {
            port,
            strips: view.tracks.len(),
            wheel_point: LogicalPosition::new(0.0, 0.0),
        };
        rig.port.refresh_tree(None).expect("必须能抓控件树");
        let strip = rig.bounds_of("track-0-channel-strip");
        // ⚠ `as f32`：i32→f32 是有损转换 ⇒ 标准库没给 `From<i32> for f32`（本仓库同款写法
        // 见 `tests/asset_pool_panel.rs`）。这里只是取一个滚轮落点，精度无关判据。
        rig.wheel_point = LogicalPosition::new(strip.x as f32 + 8.0, strip.y as f32 + 8.0);
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

    fn x_of(&self, id: &str) -> i32 {
        self.bounds_of(id).x
    }

    /// 混音台的左沿与可视宽（视口 = 混音台宽 − 主控栏位）。
    fn console_bounds(&self) -> (i32, i32) {
        let bounds = self.bounds_of("mixer-console");
        (
            bounds.x,
            i32::try_from(bounds.width).expect("混音台宽可转 i32"),
        )
    }

    /// 滚动视口宽（`.slint` 里就是 `parent.width - 112px`）。
    fn viewport_width(&self) -> i32 {
        self.console_bounds().1 - MASTER_GUTTER
    }

    /// 容器内容宽 —— 按 `.slint` 里 `content-width` 的同一式子从树里现取的宽档/窄档算。
    fn content_width(&self) -> i32 {
        let console_width = self.console_bounds().1;
        let panels = if console_width < NARROW_BREAKPOINT {
            PANELS_NARROW
        } else {
            PANELS_WIDE
        };
        STRIP_LEFT + STRIP_PITCH * i32::try_from(self.strips).expect("通道数可转 i32") + panels
    }

    /// 判据⑦：内容宽高出视口，且最大偏移大于单次滚动量。
    fn assert_overflows(&self) {
        let content = self.content_width();
        let viewport = self.viewport_width();
        assert!(
            content > viewport,
            "夹具内容宽 {content} 必须高出视口宽 {viewport}，否则滚动容器不成立"
        );
        assert!(
            content - viewport > SCROLL,
            "最大偏移 {} 必须大于单次滚动量 {SCROLL}，否则判据②证的不是位移而是夹紧",
            content - viewport
        );
    }

    /// 判据⑧的前半：内容宽不高出视口 ⇒ 偏移恒 0。
    fn assert_fits(&self) {
        let content = self.content_width();
        let viewport = self.viewport_width();
        assert!(
            content < viewport,
            "演示工程内容宽 {content} 必须不高出视口宽 {viewport}，否则判据⑧证的不是'不溢出'"
        );
    }

    /// 当前**在控件树里**的通道条：`(序号, 绝对 x)`，按序号升序。
    fn visible_strips(&self) -> Vec<(usize, i32)> {
        (0..self.strips)
            .filter_map(|index| {
                let node = self
                    .port
                    .tree()
                    .find_by_id(&format!("track-{index}-channel-strip"))?;
                let bounds = node.bounds?;
                Some((index, bounds.x))
            })
            .collect()
    }

    fn has_strip(&self, index: usize) -> bool {
        self.port
            .tree()
            .find_by_id(&format!("track-{index}-channel-strip"))
            .is_some()
    }

    fn is_addressable(&self, id: &str) -> bool {
        self.port.tree().find_by_id(id).is_some()
    }

    /// 向右滚 `amount` 像素（= 看后面的通道）。
    ///
    /// ⚠ 上游把 `content-x` 夹在 `[视口宽 − 内容宽, 0]`（`items/flickable.rs` 的
    /// `ensure_in_bound`：`min = w - cw`、`max = 0`），而 `new_pos = current + delta`
    /// **不取反** ⇒ 静止位（`content-x = 0`）上**正的 `delta_x` 被夹回 0**、什么也不动。
    /// "看后面的通道"因此必须送**负** delta（编排行区那票红已经为此付过一轮代价）。
    fn scroll_strips_right(&mut self, amount: i32) {
        self.dispatch_wheel(-amount, 0);
    }

    /// 向左滚 `amount` 像素（= 回到前面的通道）。
    fn scroll_strips_left(&mut self, amount: i32) {
        self.dispatch_wheel(amount, 0);
    }

    /// 送一次滚轮事件（单位：逻辑像素），随后**重抓控件树**。
    ///
    /// 上游 `PointerScrolled` 走 `TouchPhase::Cancelled` ⇒ `content-x` 是**直接 set**、
    /// 无动画 ⇒ 下一次抓树就能读到新几何（不需要等帧）。
    fn dispatch_wheel(&mut self, delta_x: i32, delta_y: i32) {
        self.port.window().dispatch(WindowEvent::PointerScrolled {
            position: self.wheel_point,
            // ⚠ `as f32`：i32→f32 有损 ⇒ 标准库不给 `From<i32> for f32`；像素级 delta 无关判据。
            delta_x: delta_x as f32,
            delta_y: delta_y as f32,
        });
        self.port.refresh_tree(None).expect("滚动后重抓控件树");
    }
}

/// ① + ②：向右滚轮 = 看后面的通道；仍在树里的每一条恰好左移该距离，滚回去逐位复原。
#[test]
fn wheel_scroll_moves_strips_left_and_reveals_later_tracks() {
    let mut rig = Rig::with_project(project_with_9_extra_tracks());
    assert_eq!(rig.strips, STRIPS, "夹具必须有 {STRIPS} 条通道");
    rig.assert_overflows();

    let before = rig.visible_strips();
    assert!(!before.is_empty(), "首条通道必须可达");
    let (console_x, _) = rig.console_bounds();
    assert_eq!(
        before[0],
        (0, console_x + STRIP_LEFT),
        "未滚动时首条通道贴着视口左沿"
    );
    assert!(
        !rig.has_strip(STRIPS - 1),
        "滚动前末条通道不在控件树里 —— 这正是本片要修的缺口（它不可达，报告 §7 把它记成 N-UI-1）"
    );

    rig.scroll_strips_right(SCROLL);

    let after = rig.visible_strips();
    let moved: Vec<(usize, i32, i32)> = before
        .iter()
        .filter_map(|&(index, x_before)| {
            after
                .iter()
                .find(|&&(other, _)| other == index)
                .map(|&(_, x_after)| (index, x_before, x_after))
        })
        .collect();
    assert!(
        moved.len() >= 5,
        "两次快照必须有一段交集才读得出位移（交集只有 {} 条）",
        moved.len()
    );
    for (index, x_before, x_after) in moved {
        assert_eq!(
            x_after,
            x_before - SCROLL,
            "第 {index} 条通道必须整体左移 {SCROLL}（同一份通道几何）"
        );
    }

    let deepest_before = before.last().expect("before 非空").0;
    let deepest_after = after.last().expect("滚动后必有可见通道").0;
    assert!(
        deepest_after > deepest_before,
        "向右滚动必须把更大序号的通道带进控件树（{deepest_before} → {deepest_after}），否则方向语义不成立"
    );

    rig.scroll_strips_left(SCROLL);
    assert_eq!(
        rig.visible_strips(),
        before,
        "向左滚回 {SCROLL} ⇒ 快照逐位复原（没有累计漂移）"
    );
    assert!(!rig.has_strip(STRIPS - 1), "回到起点 ⇒ 末条重新不可寻址");
}

/// ③ + ④：滚到底 ⇒ 末条通道与其推子可达、三块侧面板回来；继续滚 ⇒ 夹紧。
#[test]
fn scrolling_to_the_end_clamps_and_reveals_the_last_strip_and_the_panels() {
    let mut rig = Rig::with_project(project_with_9_extra_tracks());
    rig.assert_overflows();
    assert!(
        !rig.is_addressable("module-export-midi-button"),
        "未滚动时导出 MIDI 按钮被推出视口（它的位置跟着轨道数走）—— 缺口的一半"
    );

    rig.scroll_strips_right(100_000);

    let (console_x, console_width) = rig.console_bounds();
    let viewport_right = console_x + console_width - MASTER_GUTTER;
    assert!(
        rig.has_strip(STRIPS - 1),
        "滚到底后末条通道必须进入控件树（可达）—— 本片的产品面"
    );
    assert!(
        rig.is_addressable(&format!("track-{}-fader", STRIPS - 1)),
        "末条通道的**推子**必须可达（用户要的是能推它，不只是看到条）"
    );
    let last = rig.x_of(&format!("track-{}-channel-strip", STRIPS - 1));
    assert!(
        last < viewport_right && last >= console_x,
        "滚到底后末条通道左沿必须落在视口内（x={last}，视口 [{console_x}, {viewport_right})）"
    );
    assert!(!rig.has_strip(0), "滚到底后首条通道被裁出视口（不可寻址）");
    assert!(
        rig.is_addressable("module-export-midi-button"),
        "滚到底后导出 MIDI 按钮必须回到视口（侧面板与通道条共用同一个滚动流）"
    );

    let snapshot = rig.visible_strips();
    rig.scroll_strips_right(SCROLL * 100);
    assert_eq!(rig.visible_strips(), snapshot, "到底之后不再移动（夹紧）");
}

/// ⑤：纵向滚轮不动混音台（`content-height: self.height` ⇒ 上游不接这个方向）。
#[test]
fn vertical_wheel_does_not_scroll_the_console() {
    let mut rig = Rig::with_project(project_with_9_extra_tracks());
    rig.assert_overflows();
    let before = rig.visible_strips();
    let panels_before = rig.is_addressable("module-export-midi-button");

    rig.dispatch_wheel(0, -SCROLL);

    assert_eq!(
        rig.visible_strips(),
        before,
        "纵向滚轮必须不动横向容器（判据⑤证的正是'方向被上游拒收'这条语义，不是没跑到）"
    );
    assert_eq!(
        rig.is_addressable("module-export-midi-button"),
        panels_before,
        "纵向滚轮不得把侧面板带进或带出视口"
    );
}

/// ⑥：主控通道条不随滚动 —— 它的 x 在起点、滚动后、到底时逐位相同。
#[test]
fn scrolling_does_not_move_the_master_strip() {
    let mut rig = Rig::with_project(project_with_9_extra_tracks());
    rig.assert_overflows();
    let (console_x, console_width) = rig.console_bounds();
    let master_before = rig.x_of("mixer-master-strip");
    assert_eq!(
        master_before,
        console_x + console_width - MASTER_GUTTER,
        "未滚动时主控通道条必须钉在 `root.width - 112`（它在容器**外面**）"
    );

    rig.scroll_strips_right(SCROLL);
    assert_eq!(
        rig.x_of("mixer-master-strip"),
        master_before,
        "主控不随滚动"
    );

    rig.scroll_strips_right(SCROLL * 100);
    assert_eq!(
        rig.x_of("mixer-master-strip"),
        master_before,
        "滚到底时主控仍钉在原位（否则判据②的「位移」就分不清是谁动的）"
    );
}

/// ⑧：内容不宽出视口（演示工程 6 轨）⇒ 滚轮一位不动。
#[test]
fn few_tracks_do_not_scroll() {
    let mut rig = Rig::with_project(demo_project());
    rig.assert_fits();

    let before = rig.visible_strips();
    assert_eq!(before.len(), rig.strips, "演示工程的每条通道都必须可达");
    let panels_before = rig.is_addressable("module-export-midi-button");
    assert!(panels_before, "未滚动时导出 MIDI 按钮就在视口里");

    rig.scroll_strips_right(SCROLL);

    assert_eq!(
        rig.visible_strips(),
        before,
        "内容不宽出视口 ⇒ 偏移恒 0（默认帧不漂移）"
    );
    assert_eq!(
        rig.is_addressable("module-export-midi-button"),
        panels_before,
        "不溢出时侧面板一位不动"
    );
}
