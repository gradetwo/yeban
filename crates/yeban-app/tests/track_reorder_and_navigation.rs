//! **夜半 UI 轨道交互修复线（2026-11-07）** —— 缩放 / 移轨 / 横向滚动的端到端判据。
//!
//! ## 本文件要证的句子
//!
//! Wang 2026-11-07 16:35 真用报的三项缺失（编曲视图无法缩放、轨道无法上下移动、
//! 无法横向查看）+ 16:37 补充（每项都要按钮、快捷键、手势三路齐全，参照 Logic Pro）：
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（缩放：按钮 + 快捷键 + 滑块） | 放大按钮 ⇒ `roll-ticks-per-pixel` 变小、剪辑 x 变大；`Cmd+→/←` 同一条步进；滑块右端 ⇒ 缩放变大、左端 ⇒ 变小 |
//! | ②（纵向缩放方向按 Logic） | `Cmd+↓` = 放大（行高 percent 变大）、`Cmd+↑` = 缩小，往返精确回 100（⛔ 旧方向是反的） |
//! | ③（移轨：快捷键） | `Cmd+Shift+↓/↑` 移动选中轨、提交数 +1；两端停住**且不消费**、提交数不动 |
//! | ④（移轨：手势） | 包头名称区向下拖过一行中点 ⇒ 松手恰好提交一次；拖动中插入线在树里；小拖（< 4px）零提交；`Escape` 取消零提交 |
//! | ⑤（横向滚动：Shift+滚轮） | `Shift+滚轮下` ⇒ `roll-scroll-x` 增加、剪辑 x 等量左移、包头 x 不动；滚回 ⇒ 复原；无修饰滚轮 ⇒ 横向一位不动 |
//! | ⑥（横向滚动：Option+滚轮 = 纵向缩放） | `Option+滚轮下` ⇒ 行高 percent 变大（不是横向滚动）；`Option+Shift+滚轮` ⇒ 横向缩放 |
//! | ⑦（横向滚动：滚动条 + 空车道拖动） | 点滚动条轨道 ⇒ `roll-scroll-x` 跳到对应位置、剪辑等量左移；空车道区向左拖 100px ⇒ 偏移 +100 |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 线程局部、每线程一次 ⇒ 每个 `#[test]` 只建
//! **一个** `LivePort`（与 `tests/arrangement_vertical_scroll.rs` 同款）。
//!
//! ## 本机不编译重依赖（`AGENTS.md` §5）
//!
//! Slint 是重依赖 ⇒ 本文件在本机**不编译**；编译与无头比对交 CI。
//! （模型层与宿主层单测已在本地跑过，见完结回执。）

use std::cell::RefCell;
use std::rc::Rc;

use slint::LogicalPosition;
use slint::platform::{Key, WindowEvent};
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::input::InputContext;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::samples::filled_project;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, Rect, Size, UiTestPort as _};

/// 会话打开时刻（与其余界面判据同一个常量，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

const WINDOW: Size = Size::new(1440, 900);

/// 逻辑键入口的两个方向（Slint 的私有码点，与 `input.rs` 的 `LogicalKey::from_text` 同源）。
/// （`LEFT` / `RIGHT` 曾供 `Cmd+←/→` 缩放路使用，2026-11-07 跟进删掉
/// —— 既有判据要求 `Cmd/Ctrl+←/→` 放行，快捷键路改走 `Cmd+=` / `Cmd+-`。）
const UP: &str = "\u{f700}";
const DOWN: &str = "\u{f701}";

/// 一次滚轮的步长（像素）。`advance_scroll` 是纯累加 ⇒ 期望值可以直接手算。
const WHEEL_STEP: f32 = 120.0;

struct Rig {
    port: LivePort<MainWindow>,
    undo: Rc<UndoPort>,
}

impl Rig {
    fn new() -> Self {
        let project = filled_project();
        let view = ViewState::from_project(&project).expect("夹具工程必须能投影");
        let scene = DemoScene::from_view(&view);
        let undo = Rc::new(UndoPort::new(
            UndoSession::open(
                "<判据 UI 轨道交互修复>",
                "yeban-app",
                project.clone(),
                NOW_MS,
            )
            .expect("打开撤销会话"),
        ));
        let mut port = LivePort::new(WINDOW, Permission::Interactive, None, || {
            host::build_main_window(&view, &scene)
        })
        .expect("Tier-1 平台 + 主窗口");
        // 接线：与 `main.rs` 的产品进程 / `live_surface.rs` 的无头装配调**同一个**函数
        // （`tests/d_ui_11_serve_header_gestures.rs` 的名单对账要求两处都在场）。
        // `wire_keys` 的引擎与工程参数：引擎 `None`（本文件不测引擎动作）、
        // 工程 `Some`（`Cmd/Ctrl+Alt+N` 这类"要换一代引擎"的动作才有落点）。
        let input = Rc::new(RefCell::new(InputContext::new()));
        host::wire_keys(
            port.ui(),
            Rc::clone(&input),
            Some(Rc::clone(&undo)),
            None,
            Some(Rc::new(project)),
        );
        host::wire_zoom_controls(port.ui(), &undo);
        host::wire_track_reorder(port.ui(), &undo);
        host::wire_arrangement_wheel(port.ui(), &undo);
        host::wire_arrangement_hscroll(port.ui(), &undo);
        host::wire_track_select(port.ui(), &undo);
        // ⭐ 2026-11-07：`wire_roll_edit` 不可少 —— 它把 `install_roll_scroll` 从建窗时的
        // 无端口形态（`scroll-requested` 回落到建窗快照、默认 tpp=30）重装为带端口形态
        // （每帧从此刻的工程 + 窗口视图态投影）。不接它，空车道拖动第一步就把缩放打回
        // 默认（D-UI-10 的同一缺陷）；生产面（`main.rs:827`）与无头面（`live_surface.rs:2398`）
        // 都接了它，测试 Rig 与生产同一条接线。
        host::wire_roll_edit(port.ui(), &undo);
        port.refresh_tree(None).expect("必须能抓控件树");
        Self { port, undo }
    }

    fn window(&self) -> &MainWindow {
        self.port.ui()
    }

    fn bounds_of(&self, id: &str) -> Rect {
        self.port
            .tree()
            .find_by_id(id)
            .unwrap_or_else(|| panic!("{id} 必须可寻址"))
            .bounds
            .unwrap_or_else(|| panic!("{id} 必须有几何"))
    }

    fn has_element(&self, id: &str) -> bool {
        self.port.tree().find_by_id(id).is_some()
    }

    fn refresh(&mut self) {
        self.port.refresh_tree(None).expect("重抓控件树");
    }

    /// 窗口自己那一列**非主总线**轨身份（`track-ids`，与 `track-{i}-header` 同索引集）。
    fn track_ids(&self) -> Vec<String> {
        use slint::Model as _;
        self.window()
            .get_track_ids()
            .iter()
            .map(|id| id.to_string())
            .collect()
    }

    fn selected(&self) -> String {
        self.window().get_selected_track_id().to_string()
    }

    fn commits(&self) -> usize {
        self.undo.display().commit_count
    }

    fn tpp(&self) -> i32 {
        self.window().get_roll_ticks_per_pixel()
    }

    fn height_percent(&self) -> i32 {
        self.window().get_track_height_percent()
    }

    fn scroll_x(&self) -> f32 {
        self.window().get_roll_scroll_x()
    }

    fn content_px(&self) -> f32 {
        self.window().get_arrangement_content_px()
    }

    /// 树里可见、且 `clip-positions` 最大的那条剪辑拖动面的 x（窗口绝对坐标，整数像素）。
    ///
    /// ⛔ 不取第一条：夹具的第一条剪辑在 tick 0（`clip-positions[0] == 0`），
    /// 它的 x = 视图偏移 + 168 + 0，缩放与横向滚动都动不了它 —— 用它断言"剪辑跟动"
    /// 会假红（实测：tpp 30→22 时第一条 x 恒为 408，第二条 472→495）。
    /// ⛔ 也不无条件取最后一条：横向滚动后末条剪辑可能出视口、控件树里找不到它。
    /// 取"可见且 position 最大"的那条 ⇒ 缩放与滚动都跟得动，且恒可寻址。
    fn moving_clip_x(&self) -> i32 {
        use slint::Model as _;
        let positions: Vec<f32> = self.window().get_clip_positions().iter().collect();
        let ulids: Vec<String> = self
            .window()
            .get_clip_ulids()
            .iter()
            .map(|id| id.to_string())
            .collect();
        let mut best: Option<(i32, f32)> = None;
        for (index, ulid) in ulids.iter().enumerate() {
            let Some(node) = self.port.tree().find_by_id(&format!("clip-{ulid}-body")) else {
                continue;
            };
            let Some(bounds) = node.bounds else {
                continue;
            };
            let position = positions[index];
            if best.is_none_or(|(_, best_position)| position > best_position) {
                best = Some((bounds.x, position));
            }
        }
        best.expect("至少一条剪辑必须在控件树里").0
    }

    /// 树里可见的剪辑拖动面中最右沿 x（窗口绝对坐标）—— 空车道点的落点由此算。
    /// 树外的剪辑跳过（横向滚动后末条可能出视口）。
    fn rightmost_clip_right(&self) -> i32 {
        use slint::Model as _;
        self.window()
            .get_clip_ulids()
            .iter()
            .filter_map(|ulid| {
                let node = self.port.tree().find_by_id(&format!("clip-{ulid}-body"))?;
                let bounds = node.bounds?;
                Some(bounds.x + bounds.width as i32)
            })
            .max()
            .expect("至少一条剪辑必须在控件树里")
    }

    /// 放大直到内容宽出视口（步进表不写死：按几步由 `zoom_horizontal_step` 定，
    /// 判据只要求"能放大到宽出"，到不了就红）。
    ///
    /// ⚠ 每按一次键都要 `refresh()`：控件树是**快照**，`key()` 只改属性不重抓树，
    /// 不刷新则后面的 `bounds_of` 读到缩放前的旧几何（测试 ⑤ 曾因此假红：tpp 已是 12，
    /// 树里剪辑仍停在 tpp 30 的 x=472）。
    fn zoom_in_until_content_overflows(&mut self) {
        for _ in 0..12 {
            if self.content_px() > 1440.0 - 168.0 {
                return;
            }
            assert!(
                self.key("=", false, false, false, true),
                "`Cmd+=` 必须被消费（横向放大）"
            );
            self.refresh();
        }
        assert!(
            self.content_px() > 1440.0 - 168.0,
            "放大 12 步后内容总长 {} 仍未宽出视口（1440−168）",
            self.content_px()
        );
    }

    /// 缩小直到内容窄于视口（与上面同款，不写死步数；每步后 `refresh()`，理由同上）。
    fn zoom_out_until_content_fits(&mut self) {
        for _ in 0..12 {
            if self.content_px() < 1440.0 - 168.0 {
                return;
            }
            self.key("-", false, false, false, true);
            self.refresh();
        }
        assert!(
            self.content_px() < 1440.0 - 168.0,
            "缩小 12 步后内容总长 {} 仍未窄于视口（1440−168）",
            self.content_px()
        );
    }

    /// 一次**真指针**点按：down（元素局部坐标）→ up（复用按下点）。
    /// `dispatch_pointer_down` 是元素制的（`[UI-TEST-002]` §12.4），
    /// `dispatch_pointer_move` 才是窗口绝对坐标 —— 两者不要混用。
    fn tap_element(&mut self, element_id: &str, x_offset: f64, y_offset: f64) {
        self.port
            .dispatch_pointer_down(
                element_id,
                x_offset,
                y_offset,
                yeban_ui_test_port::PointerButton::Left,
            )
            .expect("按下必须被执行面接受");
        self.port
            .dispatch_pointer_up(yeban_ui_test_port::PointerButton::Left)
            .expect("松手必须被执行面接受");
        self.refresh();
    }

    /// 按钮一族的点法：**现读几何 → 打在中心**（与创作 harness 的 `ui.click` 同一落点）。
    fn tap_center(&mut self, element_id: &str) {
        let bounds = self.bounds_of(element_id);
        self.tap_element(
            element_id,
            f64::from(bounds.width) / 2.0,
            f64::from(bounds.height) / 2.0,
        );
    }

    /// 只按下（不松手）：拖拽的起手（元素局部坐标）。
    fn press_element(&mut self, element_id: &str, x_offset: f64, y_offset: f64) {
        self.port
            .dispatch_pointer_down(
                element_id,
                x_offset,
                y_offset,
                yeban_ui_test_port::PointerButton::Left,
            )
            .expect("按下必须被执行面接受");
    }

    /// 一次**窗口绝对坐标**的移动（`dispatch_pointer_move` 的口径就是绝对值）。
    fn move_to(&mut self, absolute_x: f64, absolute_y: f64) {
        self.port
            .dispatch_pointer_move(absolute_x, absolute_y)
            .expect("移动必须被执行面接受");
    }

    fn release(&mut self) {
        self.port
            .dispatch_pointer_up(yeban_ui_test_port::PointerButton::Left)
            .expect("松手必须被执行面接受");
        self.refresh();
    }

    /// 逻辑键注入（`invoke_key_action` 的口径：`(文本, shift, ctrl, alt, meta)`）。
    /// 返回界面是否消费。
    fn key(&self, text: &str, shift: bool, ctrl: bool, alt: bool, meta: bool) -> bool {
        self.window()
            .invoke_key_action(text.into(), shift, ctrl, alt, meta)
    }

    /// 滚轮注入（`arrangement_vertical_scroll.rs` 同款）：修饰键按住 → 发滚轮 → 放开。
    /// `shift` = 横向滚动，`alt` = 纵向缩放，`shift + alt` = 横向缩放；
    /// `delta_y < 0` = 轮下（向后看 / 放大），`delta_y > 0` = 轮上。
    fn wheel(&mut self, point: LogicalPosition, delta_y: f32, shift: bool, alt: bool) {
        if shift {
            self.port.window().key_press(Key::Shift);
        }
        if alt {
            self.port.window().key_press(Key::Alt);
        }
        self.port.window().dispatch(WindowEvent::PointerScrolled {
            position: point,
            delta_x: 0.0,
            delta_y,
        });
        if alt {
            self.port.window().key_release(Key::Alt);
        }
        if shift {
            self.port.window().key_release(Key::Shift);
        }
        self.refresh();
    }

    /// 轨道包头几何（窗口绝对坐标）+ 重排区按下点：
    /// x 取包头靠右 120px（躲开左上角的静音 / 独奏按钮与色标），
    /// y 取行中（= **重排区**，底边 6px 才是高度拖拽 —— 见 `.slint` 的手势分区注释）。
    /// 返回 `(包头 x, 包头 y, 包头宽, 包头高, 按下 x, 按下 y)`。
    fn header_geometry(&self, index: usize) -> (f64, f64, f64, f64, f64, f64) {
        let bounds = self.bounds_of(&format!("track-{index}-header"));
        let (x, y, w, h) = (
            f64::from(bounds.x),
            f64::from(bounds.y),
            f64::from(bounds.width),
            f64::from(bounds.height),
        );
        (x, y, w, h, x + 120.0, y + h / 2.0)
    }
}

/// ①：缩放按钮 + `Cmd+=/-` + 滑块三路都改横向缩放，且剪辑 x 跟着动。
/// （⚠ 2026-11-07 跟进：初版用 `Cmd+→/←` 作快捷键路，但既有判据
/// `arrow_keys_resolve_to_the_four_spec_nudge_steps` 要求 `Cmd/Ctrl+←/→`
/// 归 caret 语义、必须放行，故快捷键路改走 `Cmd+=` / `Cmd+-`。）
#[test]
fn zoom_button_shortcut_and_slider_change_horizontal_zoom() {
    let mut rig = Rig::new();
    assert_eq!(rig.tpp(), 30, "起点缩放 = 默认 30");
    let clip_before = rig.moving_clip_x();

    // 按钮路：放大 ⇒ tpp 变小、剪辑 x 变大（同一 tick 占更多像素）。
    rig.tap_center("arrangement-zoom-h-in-button");
    let zoomed_in = rig.tpp();
    assert!(zoomed_in < 30, "放大按钮必须减小 tpp（30 → {zoomed_in}）");
    assert!(
        rig.moving_clip_x() > clip_before,
        "放大后剪辑必须向右展开（x {clip_before} → {}）",
        rig.moving_clip_x()
    );
    // 按钮路：缩小 ⇒ 回到起点（往返精确）。
    rig.tap_center("arrangement-zoom-h-out-button");
    assert_eq!(rig.tpp(), 30, "缩小按钮必须回到起点 30");
    assert_eq!(
        rig.moving_clip_x(),
        clip_before,
        "缩放往返后剪辑 x 必须复原"
    );

    // 快捷键路：`Cmd+=` = 放大（与按钮同一条步进算法）、`Cmd+-` = 缩小。
    assert!(
        rig.key("=", false, false, false, true),
        "`Cmd+=` 必须被消费（横向放大）"
    );
    assert_eq!(rig.tpp(), zoomed_in, "`Cmd+=` 必须与放大按钮同一步进");
    assert!(
        rig.key("-", false, false, false, true),
        "`Cmd+-` 必须被消费（横向缩小）"
    );
    assert_eq!(rig.tpp(), 30, "`Cmd+-` 必须回到起点 30");

    // 滑块路：右端 ⇒ tpp 变大（缩小），左端 ⇒ tpp 变小（放大）。
    // 滑块 TouchArea 的 fraction = (mouse-x − 7px) / 114px（局部坐标，见 `.slint` 注释）。
    let slider_h = f64::from(rig.bounds_of("arrangement-zoom-h-slider").height);
    rig.tap_element("arrangement-zoom-h-slider", 120.0, slider_h / 2.0);
    assert!(
        rig.tpp() > 500,
        "滑块右端必须把 tpp 拉大（实得 {}）",
        rig.tpp()
    );
    rig.tap_element("arrangement-zoom-h-slider", 10.0, slider_h / 2.0);
    assert_eq!(rig.tpp(), 1, "滑块左端必须把 tpp 拉到最小 1");
}

/// ②：纵向缩放方向按 Logic（`Cmd+↓` = 放大、`Cmd+↑` = 缩小），往返精确。
#[test]
fn cmd_down_up_change_vertical_zoom_in_logic_direction() {
    let mut rig = Rig::new();
    assert_eq!(rig.height_percent(), 100, "起点行高乘子 = 100");

    // ⛔ 旧方向是反的（`Cmd+↑` = 放大）：本判据钉的是 Logic 的方向。
    assert!(
        rig.key(DOWN, false, false, false, true),
        "`Cmd+↓` 必须被消费（纵向放大）"
    );
    assert!(
        rig.height_percent() > 100,
        "`Cmd+↓` 必须放大（100 → {}）",
        rig.height_percent()
    );
    assert!(
        rig.key(UP, false, false, false, true),
        "`Cmd+↑` 必须被消费（纵向缩小）"
    );
    assert_eq!(
        rig.height_percent(),
        100,
        "`Cmd+↑` 必须回到起点 100（往返精确）"
    );

    // 按钮路：纵向放大 / 缩小按钮同一条步进。
    rig.tap_center("arrangement-zoom-v-in-button");
    assert!(
        rig.height_percent() > 100,
        "纵向放大按钮必须放大（实得 {}）",
        rig.height_percent()
    );
    rig.tap_center("arrangement-zoom-v-out-button");
    assert_eq!(rig.height_percent(), 100, "纵向缩小按钮必须回到 100");
}

/// ③：`Cmd+Shift+↓/↑` 移动选中轨、提交数 +1；两端停住且不消费、提交数不动。
#[test]
fn cmd_shift_arrows_move_selected_track_and_stop_at_the_ends() {
    let mut rig = Rig::new();
    let ids = rig.track_ids();
    assert_eq!(ids.len(), 3, "夹具的非主总线轨数: {ids:?}");
    let commits_at_start = rig.commits();

    // 选中中间轨（行中点按 = 重排区：只按下不拖动 ⇒ 只选中、零提交）。
    let (_, _, _, h, _, _) = rig.header_geometry(1);
    rig.tap_element("track-1-header", 120.0, h / 2.0);
    assert_eq!(rig.selected(), ids[1], "点包头必须选中第 2 条轨");
    assert_eq!(rig.commits(), commits_at_start, "单纯选中不产生提交");

    // 下移一格 ⇒ 顺序换位、提交 +1。
    assert!(
        rig.key(DOWN, true, false, false, true),
        "`Cmd+Shift+↓` 必须被消费"
    );
    assert_eq!(
        rig.track_ids(),
        vec![ids[0].clone(), ids[2].clone(), ids[1].clone()],
        "下移一格后选中轨必须与下一轨换位"
    );
    assert_eq!(rig.selected(), ids[1], "选中态跟随轨道身份（不是下标）");
    assert_eq!(rig.commits(), commits_at_start + 1, "一次移轨 = 一次提交");

    // 末轨再下移 ⇒ 边界：不消费、不提交、顺序不动。
    assert!(
        !rig.key(DOWN, true, false, false, true),
        "末轨下移必须是空操作（不消费）"
    );
    assert_eq!(
        rig.track_ids(),
        vec![ids[0].clone(), ids[2].clone(), ids[1].clone()],
        "边界下移不得改顺序"
    );
    assert_eq!(rig.commits(), commits_at_start + 1, "边界下移不得提交");

    // 上移两格 ⇒ 回到起点再到顶；顶轨再上移 ⇒ 边界。
    assert!(
        rig.key(UP, true, false, false, true),
        "`Cmd+Shift+↑` 必须被消费"
    );
    assert_eq!(
        rig.track_ids(),
        vec![ids[0].clone(), ids[1].clone(), ids[2].clone()],
        "上移一格必须回到起点顺序"
    );
    assert!(
        rig.key(UP, true, false, false, true),
        "第二次上移必须被消费"
    );
    assert_eq!(
        rig.track_ids(),
        vec![ids[1].clone(), ids[0].clone(), ids[2].clone()],
        "选中轨必须移到顶"
    );
    assert!(
        !rig.key(UP, true, false, false, true),
        "顶轨上移必须是空操作（不消费）"
    );
    assert_eq!(
        rig.commits(),
        commits_at_start + 3,
        "三次有效移轨 = 三次提交"
    );
}

/// ④：包头名称区拖动 ⇒ 松手恰好提交一次；拖动中插入线在树里；小拖零提交；`Escape` 取消。
#[test]
fn header_drag_moves_track_once_and_escape_cancels() {
    let mut rig = Rig::new();
    let ids = rig.track_ids();
    assert_eq!(ids.len(), 3, "夹具的非主总线轨数: {ids:?}");
    let commits_at_start = rig.commits();

    // 小拖（2px < 4px 阈值）⇒ 不进入重排、松手零提交。
    let (_, _, _, h, press_x, press_y) = rig.header_geometry(1);
    rig.press_element("track-1-header", 120.0, h / 2.0);
    rig.move_to(press_x, press_y + 2.0);
    rig.release();
    assert_eq!(rig.track_ids(), ids, "小拖不得改顺序");
    assert_eq!(rig.commits(), commits_at_start, "小拖不得提交");
    assert!(
        !rig.has_element("track-reorder-insertion-line"),
        "小拖不得出现插入线"
    );

    // 真拖：向下 60px（越过第 3 行中点：行距 54px）⇒ 拖动中插入线在树里。
    let (_, _, _, h, press_x, press_y) = rig.header_geometry(1);
    rig.press_element("track-1-header", 120.0, h / 2.0);
    for step in 1..=6 {
        rig.move_to(press_x, press_y + f64::from(step) * 10.0);
    }
    rig.refresh();
    assert!(
        rig.window().get_track_reorder_preview_index() >= 0,
        "越过 4px 阈值后必须有插入预览"
    );
    assert!(
        rig.has_element("track-reorder-insertion-line"),
        "拖动中插入线必须在控件树里"
    );
    // 草稿期不碰工程：提交数与顺序一位不动。
    assert_eq!(rig.track_ids(), ids, "拖动中顺序不得变（草稿期不碰工程）");
    assert_eq!(rig.commits(), commits_at_start, "拖动中不得提交");
    rig.release();
    assert_eq!(
        rig.track_ids(),
        vec![ids[0].clone(), ids[2].clone(), ids[1].clone()],
        "松手后选中轨必须移到下一行"
    );
    assert_eq!(rig.commits(), commits_at_start + 1, "一次拖动 = 一次提交");
    assert!(
        !rig.has_element("track-reorder-insertion-line"),
        "松手后插入线必须消失"
    );

    // `Escape` 取消：再拖一次、中途按 `Escape` ⇒ 零提交、顺序不动、预览清零。
    // （拖 60px：须越过下一行中点才有插入预览；30px 不越中点、无预览可取消。）
    let (_, _, _, h, press_x, press_y) = rig.header_geometry(1);
    rig.press_element("track-1-header", 120.0, h / 2.0);
    rig.move_to(press_x, press_y + 60.0);
    rig.refresh();
    assert!(
        rig.window().get_track_reorder_preview_index() >= 0,
        "第二次拖动也必须有插入预览"
    );
    assert!(
        rig.key("\u{1b}", false, false, false, false),
        "`Escape` 必须被消费（有进行中的重排可取消）"
    );
    assert_eq!(
        rig.window().get_track_reorder_preview_index(),
        -1,
        "`Escape` 后预览下标必须清零"
    );
    rig.refresh();
    assert_eq!(
        rig.track_ids(),
        vec![ids[0].clone(), ids[2].clone(), ids[1].clone()],
        "`Escape` 取消后顺序必须回到松手后的样子"
    );
    assert_eq!(rig.commits(), commits_at_start + 1, "`Escape` 取消不得提交");
    assert!(
        !rig.has_element("track-reorder-insertion-line"),
        "`Escape` 后插入线必须消失"
    );
    rig.release();
    assert_eq!(
        rig.commits(),
        commits_at_start + 1,
        "取消后的松手也不得提交"
    );
}

/// ⑤：`Shift+滚轮` = 横向滚动（剪辑等量左移、包头不动）；无修饰滚轮 ⇒ 横向一位不动。
#[test]
fn shift_wheel_scrolls_horizontally_and_moves_clips() {
    let mut rig = Rig::new();
    // 前置：内容必须宽出视口（否则没有可滚的距离）。放大直到宽出（步数不写死）。
    rig.zoom_in_until_content_overflows();
    let content = rig.content_px();
    assert!(
        content > 1440.0 - 168.0,
        "夹具内容总长 {content} 必须宽出视口（1440−168），否则没有可滚的距离"
    );
    assert_eq!(rig.scroll_x(), 0.0, "起点偏移 = 0");
    let clip_before = rig.moving_clip_x();
    let header_before = rig.bounds_of("track-0-header");

    // `Shift+滚轮下`（delta_y < 0）⇒ 向后看：偏移 +120、剪辑左移 120、包头不动。
    let point = LogicalPosition::new(
        header_before.x as f32 + 120.0,
        header_before.y as f32 + header_before.height as f32 / 2.0,
    );
    rig.wheel(point, -WHEEL_STEP, true, false);
    assert_eq!(rig.scroll_x(), WHEEL_STEP, "偏移必须恰好 +120");
    assert_eq!(
        rig.moving_clip_x(),
        clip_before - WHEEL_STEP as i32,
        "剪辑必须等量左移 120"
    );
    assert_eq!(
        rig.bounds_of("track-0-header"),
        header_before,
        "包头列固定：横向滚动不得动包头"
    );

    // `Shift+滚轮上` ⇒ 回到原点（往返精确）。
    rig.wheel(point, WHEEL_STEP, true, false);
    assert_eq!(rig.scroll_x(), 0.0, "滚回后偏移必须回到 0");
    assert_eq!(rig.moving_clip_x(), clip_before, "滚回后剪辑 x 必须复原");

    // 无修饰滚轮 ⇒ 横向一位不动（`reject` 冒泡给 Flickable 纵向滚动；3 轨内容
    // 不高出视口 ⇒ 纵向也不动）。
    rig.wheel(point, -WHEEL_STEP, false, false);
    assert_eq!(rig.scroll_x(), 0.0, "无修饰滚轮不得改横向偏移");
    assert_eq!(rig.moving_clip_x(), clip_before, "无修饰滚轮不得动剪辑");
}

/// ⑥：`Option+滚轮` = 纵向缩放、`Option+Shift+滚轮` = 横向缩放（不是横向滚动）。
#[test]
fn option_wheel_zooms_and_option_shift_wheel_zooms_horizontally() {
    let mut rig = Rig::new();
    assert_eq!(rig.height_percent(), 100, "起点行高乘子 = 100");
    assert_eq!(rig.tpp(), 30, "起点缩放 = 30");
    assert_eq!(rig.scroll_x(), 0.0, "起点偏移 = 0");
    let header = rig.bounds_of("track-0-header");
    let point = LogicalPosition::new(
        header.x as f32 + 120.0,
        header.y as f32 + header.height as f32 / 2.0,
    );

    // `Option+滚轮上`（delta_y > 0）⇒ 纵向放大（行高变大），横向偏移不动。
    // （简报口径：delta_y > 0 = 放大，< 0 = 缩小；与浏览器 Ctrl+滚轮同向。）
    rig.wheel(point, WHEEL_STEP, false, true);
    assert!(
        rig.height_percent() > 100,
        "`Option+滚轮上` 必须纵向放大（实得 {}）",
        rig.height_percent()
    );
    assert_eq!(rig.scroll_x(), 0.0, "`Option+滚轮` 不得改横向偏移");

    // `Option+滚轮下` ⇒ 回到 100。
    rig.wheel(point, -WHEEL_STEP, false, true);
    assert_eq!(rig.height_percent(), 100, "`Option+滚轮下` 必须回到 100");

    // `Option+Shift+滚轮上` ⇒ 横向放大（tpp 变小），横向偏移不动。
    rig.wheel(point, WHEEL_STEP, true, true);
    assert!(
        rig.tpp() < 30,
        "`Option+Shift+滚轮上` 必须横向放大（实得 {}）",
        rig.tpp()
    );
    assert_eq!(rig.scroll_x(), 0.0, "`Option+Shift+滚轮` 不得改横向偏移");
}

/// ⑦：横向滚动条（点轨道跳位置）+ 空车道拖动（向左拖 100px ⇒ 偏移 +100）。
#[test]
fn scrollbar_and_lane_drag_scroll_horizontally() {
    let mut rig = Rig::new();
    // 前置：内容宽出视口（放大直到宽出，步数不写死）。
    rig.zoom_in_until_content_overflows();
    let content = rig.content_px();
    assert!(
        content > 1440.0 - 168.0,
        "夹具内容总长 {content} 必须宽出视口，否则没有可滚的距离"
    );
    let clip_before = rig.moving_clip_x();

    // 滚动条路：点轨道 50% 处 ⇒ 偏移跳到对应位置（滑块中心跟指针）、剪辑等量左移。
    // ⛔ 不点 75%：滑块占轨道大半（thumb 540px / 轨道 752px），75% 处 fraction 已夹到 1、
    // 偏移 = max-scroll 528px，夹具最右剪辑会被推出 Flickable 裁剪区、树里读不到它，
    // 「剪辑左移量 == 偏移量」无从断言。50% 处 fraction 恰为 0.5、偏移 264px，最右剪辑
    // 仍在视口内（x=304），可断言；75% 的夹紧行为另起一条断言（见下）。
    let bar = rig.bounds_of("arrangement-hscroll");
    rig.tap_element(
        "arrangement-hscroll",
        f64::from(bar.width) * 0.5,
        f64::from(bar.height) / 2.0,
    );
    let scrolled = rig.scroll_x();
    assert!(
        scrolled > 0.0,
        "点滚动条 50% 处必须滚出正偏移（实得 {scrolled}）"
    );
    assert!(
        (f64::from(clip_before - rig.moving_clip_x()) - f64::from(scrolled)).abs() < 1.5,
        "剪辑左移量必须等于偏移量（剪辑 {clip_before} → {}，偏移 {scrolled}）",
        rig.moving_clip_x()
    );

    // 夹紧路：再点轨道 75% 处 ⇒ fraction 夹到 1，偏移必须恰好等于 max-scroll
    // （内容 − 视口），不得越过（未夹时实测 733px）。此处剪辑已出视口，不再断言剪辑。
    let bar = rig.bounds_of("arrangement-hscroll");
    let max_scroll = f64::from(rig.content_px()) - f64::from(bar.width);
    rig.tap_element(
        "arrangement-hscroll",
        f64::from(bar.width) * 0.75,
        f64::from(bar.height) / 2.0,
    );
    assert!(
        (f64::from(rig.scroll_x()) - max_scroll).abs() < 1.5,
        "点滚动条 75% 处偏移必须夹到 max-scroll（实得 {}，max-scroll {max_scroll}）",
        rig.scroll_x()
    );

    // 空车道拖动路：先缩小回"内容窄于视口"（空车道区出现在内容右端），
    // 再在空车道区向左拖 100px ⇒ 偏移 +100（`advance_scroll` 只夹下界 0，
    // 上限暂不设 —— 与卷帘同一条既有口径）。
    rig.zoom_out_until_content_fits();
    assert!(
        rig.content_px() < 1440.0 - 168.0,
        "缩小后内容总长 {} 必须窄于视口，空车道区才出现",
        rig.content_px()
    );
    // 回到原点：点滚动条左端（滑块 fraction 夹到 0 ⇒ 偏移 0）。
    let bar = rig.bounds_of("arrangement-hscroll");
    rig.tap_element("arrangement-hscroll", 5.0, f64::from(bar.height) / 2.0);
    assert_eq!(rig.scroll_x(), 0.0, "点滚动条左端必须回到偏移 0");
    // 空点：第 0 行车道中部、全部剪辑右沿 + 50px（从控件树现读，不写死坐标）。
    let lane_area = rig.bounds_of("arrangement-lane-scroll-area");
    let header0 = rig.bounds_of("track-0-header");
    let lane_y = f64::from(header0.y) + f64::from(header0.height) / 2.0;
    let empty_x = f64::from(rig.rightmost_clip_right()) + 50.0;
    assert!(empty_x < 1400.0, "空点 x={empty_x} 必须在窗口内");
    let clip_before_drag = rig.moving_clip_x();
    rig.press_element(
        "arrangement-lane-scroll-area",
        empty_x - f64::from(lane_area.x),
        lane_y - f64::from(lane_area.y),
    );
    for step in 1..=4 {
        rig.move_to(empty_x - f64::from(step) * 25.0, lane_y);
    }
    rig.release();
    assert_eq!(rig.scroll_x(), 100.0, "向左拖 100px ⇒ 偏移必须恰好 +100");
    assert_eq!(
        rig.moving_clip_x(),
        clip_before_drag - 100,
        "剪辑必须等量左移 100"
    );
}
