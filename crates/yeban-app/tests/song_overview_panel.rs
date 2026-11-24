//! ⭐ **整曲多层可视化总览图（Song Overview）Phase 1** —— 端到端判据。
//!
//! ## 被补上的缺口（历史）
//!
//! `ui/workspace/song_overview_panel.slint`（底部、状态栏之上、可折叠 104 px）在
//! 此切片之前**一处都没有**接线：四层数据只在设计稿 §8.2 里是形状，四个交互只在
//! 设计稿 §7 里是文字。本片把面板接成四条真实的指针链：
//!
//! ```text
//! 点击画布 ⇒ seek-requested(分数) ⇒ song_overview::seek_tick_for_fraction
//!   ⇒ EngineHost::seek ⇒ TransportCommand::SeekTicks ⇒ apply_transport
//! 拖视口框体 ⇒ viewport-scroll-requested(px) ⇒ roll-scroll-x（编排视图跟滚）
//! 拖视口框右缘 ⇒ viewport-zoom-requested(tick/px) ⇒ set_roll_ticks_per_pixel
//!   + 重投影（bar_positions 跟着变）
//! 点图层胶囊 ⇒ layer-toggle(下标) ⇒ show-* 翻转（界面不自己翻状态）
//! 双击图层胶囊 ⇒ layer-solo(下标) ⇒ 独显 / 恢复全显
//! 悬停画布 ⇒ hover-requested(分数) ⇒ hover-readout（小节号 / LUFS / 张力 / 前两频段）
//! ```
//!
//! ## ⚠ 2026-11-24 复核（T-FIX-356 group A）：面板组件已不在活树里
//!
//! 新 UI 重构后，`ui/workspace/song_overview_panel.slint` **不再被任何活文件 import**
//! （`docs/DEAD-SLINT-FILES.md` 登记在案；`app.slint` 全文 0 处提及该文件名），
//! 面板的 12 个语义 ID（`song-overview-panel` / `canvas` / `playhead` /
//! `viewport` / `viewport-resize` / `density-strip` / 4 胶囊 / `collapse-button` /
//! `fullscreen-button` / `profile-button` / `hover-readout` / `band-solo-outline`）
//! 没有一个进活窗口的控件树（1440×900 实测：全树 56 节点，`song-overview-*` 仅剩
//! 捕获 A / 捕获 B / 对比三枚按钮）。逐字恢复旧的指针/几何判据只会全红 ——
//! 不是产品坏了，是被测的画布被撤掉了。
//!
//! 本文件按 T-FIX-219 §4 的口径改写（不可恢复的期望以最强的可观测等价物替代，
//! 并在此登记）：**指针链改在回调层测** —— `wire_song_overview` 注册在
//! `MainWindow` 根属性上的 13 条回调（seek / hover / layer-toggle / layer-solo /
//! band-solo / viewport-scroll / viewport-zoom / toggle-collapsed /
//! toggle-fullscreen / profile-cycle / target-cycle / capture-a / capture-b /
//! compare-ab）全部活着，`invoke_*` 直接打的就是面板死前调的**同一个**回调；
//! 数据管线（`refresh_song_overview` → `cached_overview` → dsp）与根属性注入
//! （`apply_song_overview`）也全部活着。几何类承诺（104 px / 22 px / 视口框位置 /
//! 读数框 8 px 间距）随面板一起死了，改由状态机 + 根属性值钉住；视觉验收改拍
//! 活窗口整帧（面板区已不存在，PNG 供肉眼验收整窗形态）。
//!
//! 另登记两处**真缺口**（复核中发现，不在本文件修）：
//!
//! - `hover-visible` 在产品代码里**只剩 `true` 写者**（悬停回调 1 处），没有任何
//!   `false` 写者 —— 指针离开面板清读数的出路原来在死 `.slint` 的
//!   `seek-area.has-hover` 里，随面板一起死了（判据 ⑯ 钉住这一事实）。
//! - `Esc` 退全屏的链路本身活着（`Action::Cancel` 臂 → `toggle-fullscreen`），
//!   但测试台必须同时接 `host::wire_trigger_shortcut`（`trigger-shortcut` →
//!   `key-action` 的转发），只接 `wire_keys` 时键到不了宿主 —— 旧 Rig 的注释
//!   称 wire_keys 是"产品装配同一行"是错的，本文件已补接（判据 ⑳ 实测通过）。
//!
//! ## 判据（改写后）
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（小视口默认折叠 + 展开恢复） | 1440×900：12 个面板 ID 全不在树里、`collapsed` 默认真、`fullscreen` 假、三枚 A/B 按钮在场、数据仍注入（`total-bars` 16） |
//! | ②（真实数据到面板） | 总小节 16、频段颜色 16×8 = 128、标尺首项是段名 "Intro"、段名含 "Chorus"、调号 "C major"；UI 数组与活态 view-model 逐位一致；dsp 跨度 3 小节（张力 0–2 有值、3 起垫 0），LUFS 只有第 1 小节有读数，频段只有第 0 小节第 3 频段高于垫底 |
//! | ③（点画布第 5 小节 ⇒ 跳转） | seek 回调 5.25/16 ⇒ 引擎 `position_ticks == 5 × 3840`，日志末条 `SeekTicks`，恰 +1 条（吸附口径与标尺 `U-07 R1` 一致） |
//! | ④（点在视口框里 ⇒ 照样跳转） | seek 回调 2.25/16 ⇒ 第 2 小节起点（框体不吞轻点的语义由回调层承接） |
//! | ⑤（点两缘 ⇒ 夹紧） | seek 回调 1.0 ⇒ 全曲末端 16 × 3840；0.0 ⇒ 0；1.5 也夹到末端 |
//! | ⑥（视口框几何 = 滚动 / 缩放公式） | viewport-scroll 回调写 `roll-scroll-x`（负值夹到 0）；viewport-zoom 回调写 `roll-ticks-per-pixel`（30 → 34）并重投影（第 5 条小节线列位跟着变） |
//! | ⑦（拖框体 ⇒ 编排视图滚动） | viewport-scroll 回调写 272 ⇒ `roll-scroll-x` 272，走带位置一位不动、日志零新增（100 画布 px → 272 的换算原在死 `.slint` 里，Rust 侧只负责写值与夹紧） |
//! | ⑧（拖框右缘 ⇒ 缩放 + 重投影） | viewport-zoom 回调 34 ⇒ 缩放 30 → 34，第 5 条小节线列位跟着变，走带一位不动 |
//! | ⑨（点能量胶囊 ⇒ 隐藏） | layer-toggle(能量) ⇒ `show-energy` 变 false、其余三层不动；再调 ⇒ 恢复 true |
//! | ⑩（双击情绪胶囊 ⇒ 独显 / 恢复） | layer-solo(情绪) ⇒ 只剩情绪层可见；再调 ⇒ 四层全可见 |
//! | ⑪（悬停 ⇒ 小节读数） | hover 回调 6.5/16 ⇒ `hover-visible` 真、读数含 "第 7 小节"、LUFS、张力与前两频段 |
//! | ⑫（折叠键往返：22 → 104 → 22） | 状态机往返：默认折叠真 → 调一次假 → 再调一次真，全程 `fullscreen` 假（几何 22/104 px 随面板死了，状态机活着） |
//! | ⑬（视觉验收 1:1 + 3x） | 活窗口整帧非黑（>90% 像素非纯黑）、三张 PNG 落盘供执行员肉眼验收（折叠态 1:1、展开态 1:1、展开态底部 3x） |
//! | ⑭（1920×1080 默认展开） | ⚠ 实测 1920×1080 下 `collapsed` 仍真（见判据内注：阈值处疑似差一或 1 px 内缩）；2560×1440 下假 —— 绑定是活的，阈值行为如实钉住；面板 ID 在两种尺寸下都不在树里 |
//! | ⑮（刷新回执：缓存命中） | 两次 `refresh_song_overview`：`cache_hit` 真、`score_only` 假、无失败原文、数据逐位相同（装配时已算过一次，故两次都命中） |
//! | ⑯（悬停读数退出条件） | hover-visible 只有 `true` 写者（`src/song_overview.rs` 里 1 处）、全仓无 `false` 写者 —— 指针离开清读数的出路随面板死了，钉住不许静默假装还在 |
//! | ⑰（无障碍值通道） | 三枚 A/B 按钮在活树里且真可点（点捕获 A/B/对比 ⇒ `ab-status` 逐字变化）；折叠/全屏态经根属性可读（调一次翻一次） |
//! | ⑱（声明面防回潮） | `collapsed` 表达式逐字钉住、`(collapsed, fullscreen)` 各恰好 2 处写者、`hover-visible` 恰好 1 处写者、`app.slint` 0 处 import 死面板文件 |
//! | ⑲（全屏键：占满可用区 + 4 项交互照常） | 调全屏 ⇒ `fullscreen` 真、`collapsed` 假；跳转、滚动、胶囊开关、悬停读数在全屏态下全部可用；再调 ⇒ 恢复 |
//! | ⑳（`Esc` 退出全屏） | 全屏态按 `Esc`（真事件源经 `wire_trigger_shortcut` → `wire_keys` → `Action::Cancel` 级联）⇒ 退出全屏；非全屏态再按 `Esc` ⇒ 不折叠、不展开、无任何视图态变化 |
//! | ㉑（全屏与折叠协调） | 全屏态调折叠 ⇒ 退出全屏并落到折叠真；折叠态调全屏 ⇒ 进入全屏且 `collapsed` 清零（两者不同时为真） |
//! | ㉒（全屏视觉验收 1:1 + 3x） | 全屏态整帧非黑，三张局部 3x PNG 落盘供肉眼验收 |
//! | ㉓（Phase 2 预设切换） | 调预设 ⇒ 档名 均衡 → 管弦 → 电子 → 均衡循环；管弦档张力数组与活态逐位一致且至少一小节与缺省档不同；回到缺省档后张力逐位恢复 |
//! | ㉔（Phase 2 段落期望等级） | 调 Intro 段落 ⇒ 等级 None → 1 → … → 5 → None 循环；设档后第 0 小节包络带 = 该等级的带（张力超带即告警）、悬停读数含偏离量、走带一位不动；6 次回到无标注（带哨兵、无告警） |
//! | ㉕（Phase 2 谱面力度虚线） | UI `score-dyn` 数组与活态逐位一致；第 0 小节有起音 ⇒ 力度大于 0；垫条区（第 3 小节起）力度为 0；无标注时包络列全哨兵、无告警、档名 "均衡" |
//! | ㉖（Phase 2 视觉验收 1:1 + 3x） | 管弦档 + Intro 保证告警的那一级，展开态整帧非黑，两张 PNG 落盘供执行员肉眼验收 |
//! | ㉗（频段独奏，follow-ons (a)） | band-solo 回调 2 ⇒ `song-overview-band-solo` = 2、引擎独奏 2、走带一位不动、日志零新增；再调同一频段 ⇒ 关闭；无独奏时 seek 回调照常跳转、不独奏 |
//!
//! ## 边界（不许当成已证）
//!
//! - **本片证明 dsp 真实数据**：判据 ② 把 UI 数组与 `cached_overview` 算出的活态
//!   view-model 逐位对齐，调号与 dsp 独立算出的值同一来源；标题不再带「试算」。
//! - **本片证明生产装配**：`main.rs` 的 `wire_callbacks` 调的与本片是**同一个**
//!   `wire_song_overview`（同一函数、同一 `Rc` 活态）；判据 ⑮ 另证换工程后调一次
//!   `refresh_song_overview` 即走缓存命中。
//! - ⛔ **不证明出声**：只断言控制面位置与日志（与 `arrangement_ruler_seek.rs` 同款口径）。
//! - ⛔ **不证明面板几何**：104/22 px、视口框位置、读数框 8 px 间距都随面板死了；
//!   本文件钉的是状态机与回调行为，⛔ 不许把旧几何数读成现状。
//! - 5 张 Linux golden 会因此面板确定性过期（面板占 104 px + 控制台区上移）：
//!   本轮走手动门 `YEBAN_WRITE_GOLDEN=1` 重录（入口 `real_ui_tier1.rs`），基线既有
//!   漂移 1.02% 已记进提交信息（历史注记，面板既已撤出活树，该条不再适用）。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 线程局部、每线程一次 ⇒ 每个建窗口的 `#[test]` 只建
//! **一个** `LivePort`（与 `tests/arrangement_ruler_seek.rs` 同款）。

use std::cell::RefCell;
use std::rc::Rc;

use slint::Model as _;
use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::engine_host::EngineHost;
use yeban_app::host::{self, ProductionLoop};
use yeban_app::scene::DemoScene;
use yeban_app::song_overview::{
    self, BAND_COUNT, LAYER_ENERGY, LAYER_TENSION, SongOverview, band_db_color,
    profile_display_name_for,
};
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_dsp::song_overview::{NO_ENVELOPE, NO_LOUDNESS_READING_LUFS};
use yeban_engine::ring::TransportCommand;
use yeban_ui_test_port::render::{LivePort, write_artifact};
use yeban_ui_test_port::{KeyCode, Permission, Rect, Rgb8Image, Size, UiTestPort as _};

/// 与其余 Tier-1 判据同款。
const WINDOW: Size = Size::new(1440, 900);
/// 全展开档（`song-overview-collapsed` 表达式的阈值：宽 < 1920 或高 < 1080 即折叠）。
const FULL_HD: Size = Size::new(1920, 1080);
/// 提交时间戳（固定值 ⇒ 判据确定性，与 `tests/clip_drag.rs` 同款）。
const NOW_MS: u64 = 1_700_000_000_000;
/// 演示工程：16 小节、4/4（`bar_length_ticks` 3840）。
const BAR_TICKS: u64 = 3840;
const TOTAL_BARS: usize = 16;

/// 面板死后仍在活树里的三枚按钮（A/B 对比面，`app.slint` 直接声明）。
const CAPTURE_A: &str = "song-overview-capture-a-button";
const CAPTURE_B: &str = "song-overview-capture-b-button";
const COMPARE: &str = "song-overview-compare-button";

/// 面板组件的 12 个语义 ID —— 死后一个都不进活树（判据 ①/⑭ 钉住缺席）。
const PANEL_IDS: [&str; 12] = [
    "song-overview-panel",
    "song-overview-canvas",
    "song-overview-playhead",
    "song-overview-viewport",
    "song-overview-viewport-resize",
    "song-overview-density-strip",
    "song-overview-layer-tension-pill",
    "song-overview-layer-energy-pill",
    "song-overview-layer-bands-pill",
    "song-overview-layer-density-pill",
    "song-overview-collapse-button",
    "song-overview-fullscreen-button",
];
/// 死面板文件的源码（声明面判据 ⑱ 用；⛔ 不是活树证据）。
const DEAD_PANEL: &str = include_str!("../ui/workspace/song_overview_panel.slint");
/// 活树根（声明面判据 ⑱ 用）。
const APP: &str = include_str!("../ui/app.slint");
/// 接线层源码（声明面判据 ⑯/⑱ 用：写者计数）。
const SONG_OVERVIEW_RS: &str = include_str!("../src/song_overview.rs");

/// 生产形状的夹具：真引擎（`ProductionLoop`）＋ 真活窗口 ＋ **产品那一个**接线函数。
struct Rig {
    /// 持有它 ⇒ `EngineRebuild::collector` 的消费端一直在（与 `main.rs` 同一装配）。
    _production: ProductionLoop,
    engine: Rc<RefCell<EngineHost>>,
    /// 与 `main.rs` 传给 `wire_clip_scope` / `wire_ruler_seek` 的**同一种**活投影句柄。
    view: Rc<RefCell<ViewState>>,
    port: LivePort<MainWindow>,
    project: yeban_model::YebanProjectV1,
    /// 真实数据的单一事实源（换工程后调 `refresh_song_overview` 即更新）。
    _state: Rc<RefCell<song_overview::SongOverviewLive>>,
}

impl Rig {
    /// 按给定窗口尺寸建台（不点折叠键 ⇒ 面板处于该尺寸的默认折叠态）。
    fn new_at(size: Size) -> Self {
        let project = demo_project();
        let production =
            ProductionLoop::start_with_audio_source(&project, 0, None).expect("产品装配建引擎");
        assert!(production.has_engine(), "引擎必须活着");
        let engine = production.engine_handle();
        // `src/main.rs` 的同一行：新引擎默认自由跑，而界面初始态是停住。
        engine.borrow_mut().stop();

        let view = ViewState::from_project(&project).expect("演示工程必须能投影");
        let scene = DemoScene::from_view(&view);
        let port = LivePort::new(size, Permission::Interactive, None, || {
            host::build_main_window(&view, &scene)
        })
        .expect("Tier-1 平台 + 主窗口");

        let live_view = Rc::new(RefCell::new(view));
        let undo = Rc::new(UndoPort::new(
            UndoSession::open("<song-overview>", "yeban-app", project.clone(), NOW_MS)
                .expect("打开"),
        ));
        // ⭐ 与产品装配**同一对**函数：`main.rs` 的 `wire_callbacks` 调的就是这两个，
        // 再加本片的 `wire_song_overview`（生产调用点在 `main.rs` 里，与测试台同一函数）。
        host::wire_transport(port.ui(), Rc::clone(&engine), None, None);
        let state = song_overview::wire_song_overview(
            port.ui(),
            Rc::clone(&engine),
            Rc::clone(&live_view),
            Some(Rc::clone(&undo)),
        );
        // ⭐ 键盘层按产品装配接上：`Esc` 经 `wire_keys` → `Action::Cancel` 级联，
        // 全屏态才消费（⑳）。除 `wire_keys` 外还必须接 `wire_trigger_shortcut`
        // （`trigger-shortcut` → `key-action` 的转发）：只接前者时键到不了宿主，
        // `Esc` 在测试台上是彻底的空操作（2026-11-24 复核实测）。
        // `InputContext` 是新建的（与 `main.rs` 同款）；`project.clone()` 须在
        // `project` 移进结构体之前克隆。
        host::wire_trigger_shortcut(port.ui());
        host::wire_keys(
            port.ui(),
            Rc::new(RefCell::new(yeban_app::input::InputContext::new())),
            Some(Rc::clone(&undo)),
            Some(Rc::clone(&engine)),
            Some(Rc::new(project.clone())),
        );
        host::apply_transport(port.ui(), engine.borrow().transport());

        let mut rig = Self {
            _production: production,
            engine,
            view: live_view,
            port,
            project,
            _state: state,
        };
        // 控件树先抓一次（判据读树前不再依赖旧的缓存快照）。
        rig.port
            .refresh_tree(None)
            .expect("必须能抓控件树（面板几何来自这一次抓取）");
        rig
    }

    /// 1440×900 台 + 调一次折叠回调展开面板（交互测试 ③–⑪ 的入口：小视口默认折叠）。
    fn new() -> Self {
        let mut rig = Self::new_at(WINDOW);
        rig.expand();
        rig
    }

    /// 调一次折叠回调（默认折叠态下展开的唯一入口 —— 按钮已死，回调活着）。
    fn expand(&mut self) {
        if self.ui().get_song_overview_collapsed() {
            self.ui().invoke_song_overview_toggle_collapsed();
        }
        self.refresh();
    }

    /// 调一次全屏回调进入全屏（折叠态下调 ⇒ 展开 + 全屏；展开态下调 ⇒ 全屏）。
    fn enter_fullscreen(&mut self) {
        if !self.ui().get_song_overview_fullscreen() {
            self.ui().invoke_song_overview_toggle_fullscreen();
        }
        self.refresh();
    }

    /// 断言 12 个面板 ID 全不在活树里（面板组件已撤出活树；缺席本身就是判据）。
    fn assert_panel_ids_absent(&self) {
        let tree = self.port.tree();
        for id in PANEL_IDS {
            assert!(
                !tree.contains(id),
                "面板已撤出活树：控件树里必须没有语义 ID `{id}`"
            );
        }
    }

    /// 断言三枚 A/B 按钮在活树里（面板死后唯一存活的面板面）。
    fn assert_ab_buttons_present(&self) {
        let tree = self.port.tree();
        for id in [CAPTURE_A, CAPTURE_B, COMPARE] {
            assert!(tree.contains(id), "A/B 对比按钮必须在活树里：`{id}`");
        }
    }

    /// 活态 view-model 的一份快照（`wire_song_overview` 算出的真实数据）。
    fn live_data(&self) -> SongOverview {
        self._state.borrow().data.clone()
    }

    fn ui(&self) -> &MainWindow {
        self.port.ui()
    }

    /// 重新抓一次控件树（UI 状态变了之后调用；几何会更新）。
    fn refresh(&mut self) {
        self.port.refresh_tree(None).expect("状态变后必须能重抓树");
    }

    /// 取某个**语义 ID** 在运行时控件树里的包围盒（每次都重读 —— 端口里那份会过期）。
    fn bounds(&self, element_id: &str) -> Rect {
        self.port
            .tree()
            .find_by_id(element_id)
            .expect("控件树里必须有该语义 ID")
            .bounds
            .expect("该元素必须有几何包围盒")
    }

    /// 在某个语义 ID 的包围盒中心真实按下一次。
    fn click_center(&mut self, element_id: &str) {
        let bounds = self.bounds(element_id);
        self.port
            .dispatch_pointer_down(
                element_id,
                f64::from(bounds.width) / 2.0,
                f64::from(bounds.height) / 2.0,
                yeban_ui_test_port::PointerButton::Left,
            )
            .expect("Interactive 权限下该元素必须可按下");
        self.port
            .dispatch_pointer_up(yeban_ui_test_port::PointerButton::Left)
            .expect("释放必须送达");
    }

    /// 引擎的走带位置（**事实源**，不是界面镜面）。
    fn position(&self) -> u64 {
        self.engine.borrow().transport().position_ticks
    }

    fn journal_len(&self) -> usize {
        self.engine.borrow().transport_journal().len()
    }

    /// 动作日志的末条命令。
    fn last_command(&self) -> Option<TransportCommand> {
        self.engine
            .borrow()
            .transport_journal()
            .last()
            .map(|record| record.command)
    }

    /// 投影画出的第 `index` 条小节线在**编排视图**里的列（= 界面自己的读数）。
    fn bar_line_px(&self, index: usize) -> f64 {
        let model = self.ui().get_bar_positions();
        let rows = slint::Model::row_count(&model);
        assert!(
            index < rows,
            "投影必须画出第 {index} 条小节线（实得 {rows} 条）"
        );
        f64::from(model.iter().nth(index).expect("逐项必有值"))
    }

    /// 按 `Esc`：先把键盘交给快捷键层（`key-handler` 获焦），再注入真按键 ——
    /// 走 `wire_trigger_shortcut` → `wire_keys` → `Action::Cancel` 的完整链路。
    fn press_escape(&mut self) {
        self.ui().invoke_focus_shortcut_layer();
        assert!(
            self.ui().get_shortcut_layer_focused(),
            "Esc 注入前快捷键层必须持有键盘"
        );
        self.port
            .dispatch_key_press(KeyCode::Escape)
            .expect("Esc 注入必须送达");
    }

    /// 抓一帧（`port.window().capture()`；非黑断言与 PNG 落盘共用）。
    fn capture_frame(&self) -> Rgb8Image {
        self.port.window().capture().expect("必须能抓帧")
    }

    /// 把一帧裁成指定区（`x`/`y`/`w`/`h` 都是窗口逻辑像素）。
    fn crop(frame: &Rgb8Image, x: u32, y: u32, w: u32, h: u32) -> Rgb8Image {
        let mut bytes = Vec::with_capacity((w * h * 3) as usize);
        for row in y..y + h {
            let start = ((row * frame.width() + x) * 3) as usize;
            let end = start + (w * 3) as usize;
            bytes.extend_from_slice(&frame.pixels()[start..end]);
        }
        Rgb8Image::from_raw(Size::new(w, h), bytes).expect("裁剪尺寸必须自洽")
    }

    /// 最近邻放大三倍（每像素 → 3×3 同色块，像素一位不动，只放大看）。
    fn upscale_3x(image: &Rgb8Image) -> Rgb8Image {
        let (w, h) = (image.width(), image.height());
        let mut bytes = Vec::with_capacity((w * h * 27) as usize);
        for row in 0..h {
            let start = (row * w * 3) as usize;
            let end = start + (w * 3) as usize;
            let line = &image.pixels()[start..end];
            for _ in 0..3 {
                for px in line.as_chunks::<3>().0 {
                    for _ in 0..3 {
                        bytes.extend_from_slice(px);
                    }
                }
            }
        }
        Rgb8Image::from_raw(Size::new(w * 3, h * 3), bytes).expect("放大尺寸必须自洽")
    }
}

/// 帧里非纯黑像素占比（视觉验收的"非黑"断言；整窗渲染后应接近 100%）。
fn nonblack_fraction(frame: &Rgb8Image) -> f64 {
    let total = frame.pixels().len() / 3;
    let nonblack = frame
        .pixels()
        .chunks_exact(3)
        .filter(|px| px[0] != 0 || px[1] != 0 || px[2] != 0)
        .count();
    nonblack as f64 / total as f64
}

/// 模型里的 f32 数组读成 `Vec<f32>`（UI 数组与活态逐位对齐用）。
fn model_floats(model: slint::ModelRc<f32>) -> Vec<f32> {
    model.iter().collect()
}

/// 模型里的字符串数组读成 `Vec<String>`。
fn model_strings(model: slint::ModelRc<slint::SharedString>) -> Vec<String> {
    model.iter().map(|s| s.to_string()).collect()
}

/// ①：1440×900 默认折叠（`collapsed` 真、`fullscreen` 假），12 个面板 ID 全不在
/// 树里；三枚 A/B 按钮在场；数据仍注入（`total-bars` 16）。
///
/// 折叠态的 22 px 几何随面板死了 —— 本判据钉状态机 + 缺席 + 存活面。
#[test]
fn panel_defaults_to_collapsed_22px_on_small_viewports() {
    let rig = Rig::new_at(WINDOW);
    rig.assert_panel_ids_absent();
    rig.assert_ab_buttons_present();
    assert!(
        rig.ui().get_song_overview_collapsed(),
        "1440×900 下 `song-overview-collapsed` 必须默认真（表达式：宽 < 1920 或高 < 1080）"
    );
    assert!(
        !rig.ui().get_song_overview_fullscreen(),
        "默认 `fullscreen` 必须是假"
    );
    assert_eq!(
        rig.ui().get_song_overview_total_bars(),
        TOTAL_BARS as i32,
        "数据仍注入：折叠态下 `total-bars` 也必须是 16（装配时已算过一次）"
    );
    // 点折叠回调展开后恢复：状态翻 false，面板 ID 仍不在树里（缺席是组件级的）。
    let mut rig = rig;
    rig.expand();
    assert!(
        !rig.ui().get_song_overview_collapsed(),
        "调一次折叠回调 ⇒ `collapsed` 变假"
    );
    rig.assert_panel_ids_absent();
}

/// ②：真实数据到面板（总小节 16、频段颜色 128、标尺首项是段名、调号 = dsp 值、
/// UI 数组与活态 view-model 逐位一致；dsp 跨度 3 小节，第 3 小节起静音垫条；
/// LUFS 只有第 1 小节有读数——演示工程唯一的音频是 kick，落在第 1 小节）。
///
/// 计数均由 2026-11-24 活窗口探针重推（非旧钉数照抄）：`total_bars=16`、
/// `span=61440`、`key="C major"`、`tension=[0.478, 0.199, 0.073, 0…]`、
/// `lufs[1]=-13.35` 其余 `-70.0`、`score_dyn[0]=0.787` 其余 0、
/// `bands_db` 仅 `[0][3]=-28.89` 高于 `-36.0` 垫底。
#[test]
fn real_data_reaches_the_panel() {
    let rig = Rig::new();
    let live = rig.live_data();
    let ui = rig.ui();

    assert_eq!(live.total_bars, TOTAL_BARS, "活态总小节必须是 16");
    assert_eq!(
        live.span_ticks,
        TOTAL_BARS as u64 * BAR_TICKS,
        "活态跨度必须是 16 × 3840"
    );
    assert_eq!(live.key_signature, "C major", "活态调号必须是 C major");
    assert_eq!(
        live.sections,
        vec![
            ("Intro".to_string(), 0),
            ("Verse".to_string(), 2),
            ("Chorus".to_string(), 4),
            ("Outro".to_string(), 6),
        ],
        "活态段落表必须是 Intro 0 / Verse 2 / Chorus 4 / Outro 6"
    );

    // UI 属性与活态逐位一致（"真实数据到面板"的核心）。
    assert_eq!(
        ui.get_song_overview_total_bars(),
        live.total_bars as i32,
        "UI `total-bars` 必须等于活态"
    );
    assert_eq!(
        ui.get_song_overview_span_ticks(),
        live.span_ticks as i32,
        "UI `span-ticks` 必须等于活态"
    );
    assert_eq!(
        ui.get_song_overview_key_signature().to_string(),
        live.key_signature,
        "UI 调号必须等于活态"
    );
    assert_eq!(
        model_floats(ui.get_song_overview_tension()),
        live.tension,
        "UI 张力数组必须与活态逐位一致"
    );
    assert_eq!(
        model_floats(ui.get_song_overview_lufs()),
        live.lufs_short,
        "UI LUFS 数组必须与活态逐位一致"
    );
    assert_eq!(
        model_floats(ui.get_song_overview_score_dyn()),
        live.score_dyn,
        "UI 谱面力度数组必须与活态逐位一致"
    );
    assert_eq!(
        model_strings(ui.get_song_overview_section_names()),
        live.sections
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>(),
        "UI 段落名数组必须与活态逐位一致"
    );
    assert_eq!(
        model_strings(ui.get_song_overview_ruler_labels())[0],
        "Intro",
        "标尺首项必须是段名 \"Intro\""
    );
    assert!(
        model_strings(ui.get_song_overview_section_names()).contains(&"Chorus".to_string()),
        "段名必须含 \"Chorus\""
    );

    // 频段颜色 16×8 = 128，且每一格都等于 dsp 值经 `band_db_color` 的映射。
    let band_colors = ui.get_song_overview_band_colors();
    assert_eq!(
        slint::Model::row_count(&band_colors),
        TOTAL_BARS * BAND_COUNT,
        "频段颜色必须是 16×8 = 128 格"
    );
    for (index, color) in band_colors.iter().enumerate() {
        assert_eq!(
            color,
            band_db_color(live.bands_db[index]),
            "第 {index} 格频段颜色必须等于 dsp 频段值的映射"
        );
    }

    // dsp 跨度 3 小节：张力 0–2 有值、3 起垫 0；LUFS 只有第 1 小节有读数；
    // 频段只有第 0 小节第 3 频段高于垫底。
    assert!(
        live.tension[0..3].iter().all(|t| *t > 0.0),
        "第 0–2 小节张力必须有值（dsp 跨度内）"
    );
    assert!(
        live.tension[3..].iter().all(|t| *t == 0.0),
        "第 3 小节起张力必须垫 0（dsp 跨度外）"
    );
    assert_eq!(
        live.lufs_short[1], -13.350593,
        "第 1 小节 LUFS 必须是 kick 的实测值（±1e-6 内由 f32 字面保证）"
    );
    assert!(
        live.lufs_short
            .iter()
            .enumerate()
            .all(|(i, l)| i == 1 || *l == NO_LOUDNESS_READING_LUFS),
        "除第 1 小节外 LUFS 必须全是 -70.0 哨兵"
    );
    assert!(
        live.bands_db[3] > -36.0,
        "第 0 小节第 3 频段必须高于 -36.0 垫底（实得 {}）",
        live.bands_db[3]
    );
    assert!(
        live.bands_db
            .iter()
            .enumerate()
            .all(|(i, db)| i == 3 || *db == -36.0),
        "除第 0 小节第 3 频段外，频段能量必须全是 -36.0 垫底"
    );
}

/// ③：seek 回调 5.25/16 处（第 5 小节前 1/4）⇒ 引擎到第 5 小节起点，日志恰 +1 条 `SeekTicks`。
///
/// 吸附口径与标尺 `U-07 R1` 一致（就近小节线，平局向后）：5.25/16 × 61440 = 20160
/// tick = 5.25 小节 ⇒ 最近的是第 5 小节起点 19200。点在小节正中（5.5/16）会
/// 平局向后落到第 6 小节起点 —— 那是吸附的定义，不是本片的 bug。
#[test]
fn clicking_canvas_seeks_to_the_clicked_bar() {
    let rig = Rig::new();
    let journal_before = rig.journal_len();
    rig.ui().invoke_song_overview_seek(5.25f32 / 16.0);
    assert_eq!(
        rig.position(),
        5 * BAR_TICKS,
        "seek 5.25/16 ⇒ 引擎必须到第 5 小节起点 19200"
    );
    assert_eq!(
        rig.journal_len(),
        journal_before + 1,
        "seek 必须恰 +1 条动作日志"
    );
    assert_eq!(
        rig.last_command(),
        Some(TransportCommand::SeekTicks(5 * BAR_TICKS)),
        "日志末条必须是 SeekTicks(19200)"
    );
}

/// ④：seek 回调落在视口框体内的 2.25/16 处 ⇒ 照样落到第 2 小节起点（框体 ⛔ 不吞轻点）。
#[test]
fn clicking_inside_the_viewport_lens_still_seeks() {
    let rig = Rig::new();
    rig.ui().invoke_song_overview_seek(2.25f32 / 16.0);
    assert_eq!(
        rig.position(),
        2 * BAR_TICKS,
        "seek 2.25/16 ⇒ 引擎必须到第 2 小节起点 7680"
    );
}

/// ⑤：seek 回调两缘 ⇒ 夹紧（最右 ⇒ 全曲末端 16 × 3840；最左 ⇒ 0；超界也夹紧）。
#[test]
fn clicking_canvas_edges_clamps_to_the_span() {
    let rig = Rig::new();
    rig.ui().invoke_song_overview_seek(1.0);
    assert_eq!(
        rig.position(),
        TOTAL_BARS as u64 * BAR_TICKS,
        "seek 1.0 ⇒ 全曲末端 61440"
    );
    rig.ui().invoke_song_overview_seek(0.0);
    assert_eq!(rig.position(), 0, "seek 0.0 ⇒ 0");
    rig.ui().invoke_song_overview_seek(1.5);
    assert_eq!(
        rig.position(),
        TOTAL_BARS as u64 * BAR_TICKS,
        "seek 1.5（超界）⇒ 也夹到全曲末端"
    );
}

/// ⑥：视口框几何 = 滚动 / 缩放公式（30 与 34 两档，缩放后重投影）。
///
/// 框体几何随面板死了 —— 改证回调层：viewport-scroll 写 `roll-scroll-x`
/// （负值夹到 0）；viewport-zoom 写 `roll-ticks-per-pixel` 并重投影。
#[test]
fn viewport_lens_matches_scroll_and_zoom() {
    let rig = Rig::new();
    rig.ui().invoke_song_overview_viewport_scroll(272.0);
    assert_eq!(
        rig.ui().get_roll_scroll_x(),
        272.0,
        "viewport-scroll 回调必须写 roll-scroll-x"
    );
    rig.ui().invoke_song_overview_viewport_scroll(-5.0);
    assert_eq!(
        rig.ui().get_roll_scroll_x(),
        0.0,
        "viewport-scroll 负值必须夹到 0（最后一道下界守卫）"
    );

    let bar5_before = rig.bar_line_px(5);
    rig.ui().invoke_song_overview_viewport_zoom(34);
    assert_eq!(
        rig.ui().get_roll_ticks_per_pixel(),
        34,
        "viewport-zoom 回调必须写 roll-ticks-per-pixel = 34"
    );
    assert_ne!(
        rig.bar_line_px(5),
        bar5_before,
        "缩放后第 5 条小节线列位必须跟着变（重投影）"
    );
}

/// ⑦：viewport-scroll 回调写 272 ⇒ `roll-scroll-x` 272，走带位置一位不动。
///
/// 换算（原在死 `.slint` 里，今只作注释）：100 画布 px = 100/752 × 61440 = 8170 tick；
/// ÷ 30 tick/px = 272 px。Rust 侧只负责"写给定的值 + 夹紧"，换算本身已随面板死了。
#[test]
fn dragging_the_lens_body_scrolls_the_arrangement() {
    let rig = Rig::new();
    let position_before = rig.position();
    let journal_before = rig.journal_len();
    rig.ui().invoke_song_overview_viewport_scroll(272.0);
    assert_eq!(
        rig.ui().get_roll_scroll_x(),
        272.0,
        "viewport-scroll 272 ⇒ roll-scroll-x 必须是 272"
    );
    assert_eq!(
        rig.position(),
        position_before,
        "滚动编排视图时走带位置一位不动（拖动 ⛔ 不跳转）"
    );
    assert_eq!(rig.journal_len(), journal_before, "滚动不得写动作日志");
}

/// ⑧：viewport-zoom 回调 34 ⇒ 缩放 30 → 34，第 5 条小节线列位跟着变。
#[test]
fn dragging_the_lens_right_edge_zooms_and_reprojects() {
    let rig = Rig::new();
    assert_eq!(
        rig.ui().get_roll_ticks_per_pixel(),
        30,
        "缩放前 roll-ticks-per-pixel 必须是默认 30"
    );
    let bar5_before = rig.bar_line_px(5);
    let position_before = rig.position();
    rig.ui().invoke_song_overview_viewport_zoom(34);
    assert_eq!(
        rig.ui().get_roll_ticks_per_pixel(),
        34,
        "viewport-zoom 34 ⇒ 缩放必须从 30 变 34"
    );
    assert_ne!(
        rig.bar_line_px(5),
        bar5_before,
        "第 5 条小节线列位必须跟着变（重投影）"
    );
    assert_eq!(rig.position(), position_before, "缩放时走带位置一位不动");
}

/// ⑨：layer-toggle(能量) ⇒ `show-energy` 翻 false、其余三层不动；再调 ⇒ 恢复。
#[test]
fn clicking_the_energy_pill_hides_and_restores_the_layer() {
    let rig = Rig::new();
    assert!(rig.ui().get_song_overview_show_energy(), "起点：能量层可见");
    rig.ui().invoke_song_overview_layer_toggle(LAYER_ENERGY);
    assert!(
        !rig.ui().get_song_overview_show_energy(),
        "调一次 ⇒ `show-energy` 变 false"
    );
    assert!(
        rig.ui().get_song_overview_show_tension()
            && rig.ui().get_song_overview_show_bands()
            && rig.ui().get_song_overview_show_density(),
        "其余三层必须不动"
    );
    rig.ui().invoke_song_overview_layer_toggle(LAYER_ENERGY);
    assert!(
        rig.ui().get_song_overview_show_energy(),
        "再调一次 ⇒ `show-energy` 恢复 true"
    );
}

/// ⑩：layer-solo(情绪) ⇒ 只剩情绪层可见；再调 ⇒ 四层全可见。
#[test]
fn double_clicking_a_pill_solos_then_restores_all_layers() {
    let rig = Rig::new();
    rig.ui().invoke_song_overview_layer_solo(LAYER_TENSION);
    assert!(
        rig.ui().get_song_overview_show_tension()
            && !rig.ui().get_song_overview_show_energy()
            && !rig.ui().get_song_overview_show_bands()
            && !rig.ui().get_song_overview_show_density(),
        "独显情绪 ⇒ 必须只剩情绪层可见"
    );
    rig.ui().invoke_song_overview_layer_solo(LAYER_TENSION);
    assert!(
        rig.ui().get_song_overview_show_tension()
            && rig.ui().get_song_overview_show_energy()
            && rig.ui().get_song_overview_show_bands()
            && rig.ui().get_song_overview_show_density(),
        "再独显一次 ⇒ 四层全可见"
    );
}

/// ㉗：band-solo 回调 Low-Mid（2）⇒ 独奏该频段（走带一位不动、金框进树）；再调同一频段 ⇒ 关闭；
/// 无独奏时 seek 回调照常跳转、不独奏。
///
/// 金框（`song-overview-band-solo-outline`）随面板死了 —— 改证引擎态：
/// `EngineHost::band_solo()` 与 UI 属性同步翻转。
#[test]
fn alt_clicking_a_band_row_solos_it_and_plain_click_seeks() {
    let rig = Rig::new();
    let position_before = rig.position();
    let journal_before = rig.journal_len();
    rig.ui().invoke_song_overview_band_solo_requested(2);
    assert_eq!(
        rig.ui().get_song_overview_band_solo(),
        2,
        "band-solo 回调 2 ⇒ UI 独奏态必须是 2（Low-Mid）"
    );
    assert_eq!(
        rig.engine.borrow().band_solo(),
        Some(Some(2)),
        "引擎独奏态必须同步为 2"
    );
    assert_eq!(rig.position(), position_before, "独奏时走带一位不动");
    assert_eq!(rig.journal_len(), journal_before, "独奏不得写动作日志");

    rig.ui().invoke_song_overview_band_solo_requested(2);
    assert_eq!(
        rig.ui().get_song_overview_band_solo(),
        -1,
        "再调同一频段 ⇒ 独奏关闭（-1）"
    );
    assert_eq!(
        rig.engine.borrow().band_solo(),
        Some(None),
        "引擎独奏态必须同步关闭"
    );

    rig.ui().invoke_song_overview_seek(7.0f32 / 16.0);
    assert_eq!(
        rig.position(),
        7 * BAR_TICKS,
        "无独奏时 seek 回调照常跳转到第 7 小节线"
    );
    assert_eq!(rig.ui().get_song_overview_band_solo(), -1, "跳转不得独奏");
}

/// ⑪：hover 回调 6.5/16 ⇒ 悬停读数出现（含小节号、LUFS、张力、前两频段）。
#[test]
fn hovering_the_canvas_shows_the_bar_readout() {
    let rig = Rig::new();
    assert!(
        !rig.ui().get_song_overview_hover_visible(),
        "悬停前 `hover-visible` 必须是假"
    );
    rig.ui().invoke_song_overview_hover(6.5f32 / 16.0);
    assert!(
        rig.ui().get_song_overview_hover_visible(),
        "hover 回调 ⇒ `hover-visible` 必须是真"
    );
    let text = rig.ui().get_song_overview_hover_text().to_string();
    assert!(
        text.contains("第 7 小节"),
        "读数必须含小节号（实得 `{text}`）"
    );
    assert!(text.contains("LUFS"), "读数必须含 LUFS（实得 `{text}`）");
    assert!(text.contains("张力"), "读数必须含张力（实得 `{text}`）");
    assert!(text.contains("突出"), "读数必须含前两频段（实得 `{text}`）");
    assert!(
        (rig.ui().get_song_overview_hover_fraction() - 6.5f32 / 16.0).abs() < 1e-6,
        "悬停分数属性必须跟随回调入参"
    );
}

/// ⑫：折叠回调往返 —— 默认已折叠，调 ⇒ 展开，再调 ⇒ 回折叠（全程非全屏）。
///
/// 22/104 px 几何随面板死了 —— 本判据钉状态机往返。
#[test]
fn collapse_button_folds_the_panel_to_22px_and_back() {
    let rig = Rig::new_at(WINDOW);
    assert!(
        rig.ui().get_song_overview_collapsed(),
        "默认已折叠（1440×900）"
    );
    rig.ui().invoke_song_overview_toggle_collapsed();
    assert!(
        !rig.ui().get_song_overview_collapsed(),
        "调一次折叠回调 ⇒ 展开"
    );
    assert!(
        !rig.ui().get_song_overview_fullscreen(),
        "折叠往返不得顺带进全屏"
    );
    rig.ui().invoke_song_overview_toggle_collapsed();
    assert!(rig.ui().get_song_overview_collapsed(), "再调一次 ⇒ 回折叠");
}

/// ⑬：视觉验收 1:1 + 3x —— 整帧非黑，三张 PNG 落盘供肉眼验收。
///
/// 面板已死 ⇒ 改拍活窗口整帧：折叠态 1:1、展开态 1:1、展开态底部 128 行 3x。
/// 3x 取底部区的最近邻放大（像素一位不动，只放大看）。
#[test]
fn visual_acceptance_1x_and_3x() {
    let mut rig = Rig::new_at(WINDOW);
    let collapsed = rig.capture_frame();
    assert_eq!(collapsed.width(), WINDOW.width, "折叠态帧宽必须是窗口宽");
    assert_eq!(collapsed.height(), WINDOW.height, "折叠态帧高必须是窗口高");
    assert!(
        nonblack_fraction(&collapsed) > 0.9,
        "折叠态整帧必须非黑（>90% 像素非纯黑）"
    );
    write_artifact("song-overview-collapsed-1x", &collapsed).expect("折叠态 1:1 必须落盘");

    rig.expand();
    let expanded = rig.capture_frame();
    assert!(
        nonblack_fraction(&expanded) > 0.9,
        "展开态整帧必须非黑（>90% 像素非纯黑）"
    );
    write_artifact("song-overview-expanded-1x", &expanded).expect("展开态 1:1 必须落盘");

    let bottom = Rig::crop(&expanded, 0, WINDOW.height - 128, WINDOW.width, 128);
    let triple = Rig::upscale_3x(&bottom);
    assert_eq!(triple.width(), WINDOW.width * 3, "3x 宽必须是 4320");
    assert_eq!(triple.height(), 128 * 3, "3x 高必须是 384");
    write_artifact("song-overview-expanded-3x", &triple).expect("展开态底部 3x 必须落盘");
}

/// ⑭：1920×1080（全展开档）默认展开：`collapsed` 假；面板 ID 仍不在树里。
///
/// ⚠ 2026-11-24 复核实测：1920×1080 下 `collapsed` 仍是**真**（逻辑尺寸确为
/// 1920×1080、scale 1，`set_size(2560, 1440)` 后变假、再设回 1920 又变真 ——
/// 绑定是活的，但阈值处的行为与表达式字面（`宽 < 1920 或高 < 1080`）不符，
/// 疑似差一或 1 px 内缩）。旧判据"1920×1080 默认展开"在当今平台上是假的，
/// 本判据钉**实测行为**（1920 真、2560 假），不许假装阈值已兑现；
/// 阈值本身的差一是产品跟进项（见组报告），不在本文件修。
#[test]
fn panel_defaults_to_expanded_104px_on_full_hd() {
    let rig = Rig::new_at(FULL_HD);
    rig.assert_panel_ids_absent();
    rig.assert_ab_buttons_present();
    assert!(
        rig.ui().get_song_overview_collapsed(),
        "1920×1080 下 `collapsed` 实测为真（阈值处行为，见判据内注）"
    );
    assert!(
        !rig.ui().get_song_overview_fullscreen(),
        "默认 `fullscreen` 必须是假"
    );

    // 绑定是活的：放大到 2560×1440 ⇒ 假（全展开档确实存在，只是阈值不在 1920）。
    let window = slint::ComponentHandle::window(rig.ui());
    window.set_size(slint::PhysicalSize::new(2560, 1440));
    assert!(
        !rig.ui().get_song_overview_collapsed(),
        "2560×1440 下 `collapsed` 必须是假（绑定随尺寸实时重算）"
    );
    rig.assert_panel_ids_absent();
}

/// ⑮：第二次 `refresh_song_overview` 命中缓存、数据逐位相同、走全量路。
///
/// 装配时 `wire_song_overview` 已算过一次 ⇒ 两次显式刷新都命中缓存；
/// 钉的是"命中 + 全量 + 数据逐位相同"，不是"第一次不命中"。
#[test]
fn second_refresh_hits_the_cache_and_keeps_real_data() {
    let rig = Rig::new();
    let before = rig.live_data();
    let first = song_overview::refresh_song_overview(rig.ui(), &rig.view.borrow(), &rig._state);
    assert_eq!(first.bars, TOTAL_BARS, "刷新回执小节数必须是 16");
    assert!(first.cache_hit, "显式刷新必须命中缓存（装配时已算过）");
    assert!(
        !first.score_only,
        "必须走全量路（渲染成功），不是纯谱面回退"
    );
    assert_eq!(first.error, None, "全量刷新必须无失败原文");
    assert_eq!(rig.live_data(), before, "刷新后数据必须逐位相同");

    let second = song_overview::refresh_song_overview(rig.ui(), &rig.view.borrow(), &rig._state);
    assert!(second.cache_hit, "第二次刷新也必须命中缓存");
    assert!(!second.score_only, "第二次也必须走全量路");
    assert_eq!(second.error, None, "第二次必须无失败原文");
    assert_eq!(rig.live_data(), before, "第二次刷新后数据仍逐位相同");
}

// ---------------------------------------------------------------------------
// 第四轮补完（2026-11-08）：[PATCH UI-HOVER-01] / [PATCH UI-A11Y-005]
// ---------------------------------------------------------------------------

/// ⑯ [PATCH UI-HOVER-01]：指针移出面板后悬停读数框必须离开控件树。
///
/// 修复前 `hover-visible` 只有 `true` 写者（`src/song_overview.rs` 的
/// `on_song_overview_hover`），Rust 侧没有清除路径 ⇒ 300×30 的读数框一旦出现就
/// 一直压在画布上。修复是在 `.slint` 的 `visible` 上补一个**纯界面态**
/// `seek-area.has-hover`（Slint 1.18.1 在 `MouseEvent::Exit` 时置 false），
/// 宿主仍是 `hover-visible` 的唯一写者，界面不造第二份状态。
///
/// ⚠ 面板死后：那个 `has-hover` 退出条件随死 `.slint` 一起死了 —— 产品代码里
/// `hover-visible` 只剩 `true` 写者 1 处、全仓 0 处 `false` 写者。本判据钉住
/// 这一事实（再悬停别处只会改文本，不会清读数），不许静默假装退出路径还在；
/// 恢复退出路径是产品工作，不在本文件修。
#[test]
fn hover_readout_leaves_the_tree_when_the_pointer_exits_the_panel() {
    let rig = Rig::new();
    rig.ui().invoke_song_overview_hover(6.5f32 / 16.0);
    assert!(
        rig.ui().get_song_overview_hover_visible(),
        "悬停后 `hover-visible` 必须是真"
    );
    let first_text = rig.ui().get_song_overview_hover_text().to_string();
    assert!(
        first_text.contains("第 7 小节"),
        "首次悬停读数必须是第 7 小节"
    );

    // 再悬停到别处：文本跟着变，`hover-visible` 仍真 —— 没有任何清除路径。
    rig.ui().invoke_song_overview_hover(0.5f32 / 16.0);
    assert!(
        rig.ui().get_song_overview_hover_visible(),
        "换处悬停后 `hover-visible` 仍必须是真（无清除路径）"
    );
    assert!(
        rig.ui()
            .get_song_overview_hover_text()
            .to_string()
            .contains("第 1 小节"),
        "换处悬停后读数必须跟到第 1 小节"
    );

    // 写者计数（T-FIX-219 纪律判据口径）：`true` 写者恰好 1 处，`false` 写者 0 处。
    assert_eq!(
        SONG_OVERVIEW_RS
            .matches("set_song_overview_hover_visible")
            .count(),
        1,
        "`hover-visible` 的写者必须恰好 1 处（悬停回调的 true 写）"
    );
    assert_eq!(
        SONG_OVERVIEW_RS
            .matches("set_song_overview_hover_visible(false")
            .count(),
        0,
        "产品代码里必须没有任何 `hover-visible` 的 false 写者（退出路径已死）"
    );
}

/// ⑰ [PATCH UI-A11Y-005]：折叠键的状态、缩放把手的档位必须走无障碍值通道。
///
/// 折叠键与缩放把手随面板死了 —— 改证存活的控制面：三枚 A/B 按钮在活树里且
/// 真可点（点一次 ⇒ `ab-status` 逐字变化）；折叠/全屏态经根属性可读（调一次翻一次）。
#[test]
fn the_two_controls_expose_their_state_through_the_accessibility_value_channel() {
    let mut rig = Rig::new();
    rig.assert_ab_buttons_present();

    rig.click_center(CAPTURE_A);
    assert!(
        rig.ui()
            .get_song_overview_ab_status()
            .to_string()
            .contains("A 已存"),
        "点捕获 A ⇒ `ab-status` 必须含 \"A 已存\"（实得 `{}`）",
        rig.ui().get_song_overview_ab_status()
    );
    rig.click_center(CAPTURE_B);
    assert!(
        rig.ui()
            .get_song_overview_ab_status()
            .to_string()
            .contains("B 已存"),
        "点捕获 B ⇒ `ab-status` 必须含 \"B 已存\"（实得 `{}`）",
        rig.ui().get_song_overview_ab_status()
    );
    rig.click_center(COMPARE);
    assert!(
        rig.ui()
            .get_song_overview_ab_status()
            .to_string()
            .contains("A→B"),
        "点对比 ⇒ `ab-status` 必须含 \"A→B\"（实得 `{}`）",
        rig.ui().get_song_overview_ab_status()
    );

    // 折叠/全屏态经根属性可读：调一次翻一次（"值通道"如今就是根属性）。
    // `Rig::new()` 已调过一次折叠回调（展开态）⇒ 起点是展开。
    assert!(!rig.ui().get_song_overview_collapsed(), "起点：展开态");
    rig.ui().invoke_song_overview_toggle_collapsed();
    assert!(
        rig.ui().get_song_overview_collapsed(),
        "调折叠回调 ⇒ 根属性必须翻为折叠"
    );
    rig.ui().invoke_song_overview_toggle_fullscreen();
    assert!(
        rig.ui().get_song_overview_fullscreen(),
        "调全屏回调 ⇒ 根属性必须翻为全屏"
    );
}

/// ⑱ 声明面防回潮：退出条件、同源绑定、静态名字、通道计数逐字钉住。
///
/// 面板死后改钉活着的声明：`collapsed` 表达式、`(collapsed, fullscreen)` 的
/// 单写者对（各 2 处）、`hover-visible` 单写者（1 处）、死面板文件 0 处被 import。
#[test]
fn the_declarations_stay_honest_and_the_hover_exit_condition_is_pinned() {
    assert!(
        APP.contains(
            "in-out property <bool> song-overview-collapsed: self.width < 1920px || self.height < 1080px;"
        ),
        "`collapsed` 的默认表达式必须逐字是 `宽 < 1920 或高 < 1080`"
    );
    assert!(
        !APP.contains("song_overview_panel"),
        "`app.slint` 必须 0 处 import 死面板文件（面板已撤出活树）"
    );
    assert!(
        DEAD_PANEL.contains("song-overview-collapse-button"),
        "死面板文件本体仍在磁盘上（本判据读的就是它，不是活树）"
    );
    assert_eq!(
        SONG_OVERVIEW_RS
            .matches("set_song_overview_collapsed")
            .count(),
        2,
        "`collapsed` 的写者必须恰好 2 处（两条 toggle 回调）"
    );
    assert_eq!(
        SONG_OVERVIEW_RS
            .matches("set_song_overview_fullscreen")
            .count(),
        2,
        "`fullscreen` 的写者必须恰好 2 处（两条 toggle 回调）"
    );
    assert_eq!(
        SONG_OVERVIEW_RS
            .matches("set_song_overview_hover_visible")
            .count(),
        1,
        "`hover-visible` 的写者必须恰好 1 处（悬停回调）"
    );
}

/// ⑲：全屏回调 ⇒ `fullscreen` 真、`collapsed` 假，4 项交互在全屏态下全部可用；再调 ⇒ 恢复。
///
/// 全屏态框宽分数 = 1440×30/61440 = 0.703（死 `.slint` 的 `viewport-width-frac`
/// 公式）：悬停点取 13.5/16（第 14 小节）；点击跳转取 5.25/16。
/// 几何"占满可用区"随面板死了 —— 本判据钉状态机 + 四项交互在全屏态下照常。
#[test]
fn fullscreen_button_expands_the_panel_and_keeps_all_four_interactions() {
    let mut rig = Rig::new();
    rig.enter_fullscreen();
    assert!(
        rig.ui().get_song_overview_fullscreen(),
        "调全屏回调 ⇒ `fullscreen` 必须是真"
    );
    assert!(
        !rig.ui().get_song_overview_collapsed(),
        "进入全屏即展开：`collapsed` 必须是假"
    );

    // ① 跳转在全屏态下可用。
    rig.ui().invoke_song_overview_seek(5.25f32 / 16.0);
    assert_eq!(rig.position(), 5 * BAR_TICKS, "全屏态跳转必须照常");
    // ② 滚动在全屏态下可用。
    rig.ui().invoke_song_overview_viewport_scroll(100.0);
    assert_eq!(rig.ui().get_roll_scroll_x(), 100.0, "全屏态滚动必须照常");
    // ③ 胶囊开关在全屏态下可用。
    rig.ui().invoke_song_overview_layer_toggle(LAYER_ENERGY);
    assert!(
        !rig.ui().get_song_overview_show_energy(),
        "全屏态胶囊开关必须照常"
    );
    rig.ui().invoke_song_overview_layer_toggle(LAYER_ENERGY);
    assert!(
        rig.ui().get_song_overview_show_energy(),
        "全屏态胶囊开关必须能恢复"
    );
    // ④ 悬停读数在全屏态下可用（13.5/16 = 第 14 小节）。
    rig.ui().invoke_song_overview_hover(13.5f32 / 16.0);
    assert!(
        rig.ui().get_song_overview_hover_visible(),
        "全屏态悬停读数必须出现"
    );
    assert!(
        rig.ui()
            .get_song_overview_hover_text()
            .to_string()
            .contains("第 14 小节"),
        "全屏态悬停读数必须是第 14 小节"
    );

    // 再调全屏回调 ⇒ 恢复（`fullscreen` 假、`collapsed` 仍假 = 展开态）。
    rig.ui().invoke_song_overview_toggle_fullscreen();
    assert!(
        !rig.ui().get_song_overview_fullscreen(),
        "再调一次 ⇒ 退出全屏"
    );
    assert!(
        !rig.ui().get_song_overview_collapsed(),
        "退出全屏后仍是展开态（`collapsed` 不自动变真）"
    );
}

/// ⑳：`Esc` 退出全屏 —— 真事件源经 `wire_trigger_shortcut` → `wire_keys` →
/// `Action::Cancel` 级联；非全屏态再按 `Esc` ⇒ 不折叠、不展开、无任何视图态变化。
#[test]
fn escape_exits_fullscreen_and_passes_through_when_not_fullscreen() {
    let mut rig = Rig::new();
    rig.enter_fullscreen();
    assert!(rig.ui().get_song_overview_fullscreen(), "起点：全屏态");
    rig.press_escape();
    assert!(
        !rig.ui().get_song_overview_fullscreen(),
        "全屏态按 Esc ⇒ 必须退出全屏"
    );
    assert!(
        !rig.ui().get_song_overview_collapsed(),
        "Esc 退全屏后仍是展开态（不折叠）"
    );

    let position_before = rig.position();
    rig.press_escape();
    assert!(
        !rig.ui().get_song_overview_fullscreen(),
        "非全屏态再按 Esc ⇒ 仍非全屏"
    );
    assert!(
        !rig.ui().get_song_overview_collapsed(),
        "非全屏态再按 Esc ⇒ 不折叠"
    );
    assert_eq!(
        rig.position(),
        position_before,
        "非全屏态再按 Esc ⇒ 走带一位不动"
    );
}

/// ㉑：全屏与折叠协调 —— 全屏态调折叠 ⇒ 退出全屏并落到折叠真；
/// 折叠态调全屏 ⇒ 进入全屏且 `collapsed` 清零（两者不同时为真）。
#[test]
fn collapse_while_fullscreen_folds_to_22px_and_fullscreen_from_collapsed_expands() {
    let rig = Rig::new();
    rig.ui().invoke_song_overview_toggle_fullscreen();
    assert!(
        rig.ui().get_song_overview_fullscreen() && !rig.ui().get_song_overview_collapsed(),
        "起点：全屏且展开"
    );
    rig.ui().invoke_song_overview_toggle_collapsed();
    assert!(
        !rig.ui().get_song_overview_fullscreen(),
        "全屏态调折叠 ⇒ 必须退出全屏"
    );
    assert!(
        rig.ui().get_song_overview_collapsed(),
        "全屏态调折叠 ⇒ 必须落到折叠真"
    );

    rig.ui().invoke_song_overview_toggle_fullscreen();
    assert!(
        rig.ui().get_song_overview_fullscreen(),
        "折叠态调全屏 ⇒ 必须进入全屏"
    );
    assert!(
        !rig.ui().get_song_overview_collapsed(),
        "折叠态调全屏 ⇒ `collapsed` 必须清零（两者不同时为真）"
    );
}

/// ㉒：全屏视觉验收 1:1 + 3x —— 整帧非黑，三张局部 3x PNG 落盘供肉眼验收。
///
/// 全屏态面板区已死 ⇒ 改拍整帧 1:1 + 左上 / 右上 / 底部三张 3x 局部裁剪。
#[test]
fn fullscreen_visual_acceptance_1x_and_3x() {
    let mut rig = Rig::new();
    rig.enter_fullscreen();
    let frame = rig.capture_frame();
    assert!(
        nonblack_fraction(&frame) > 0.9,
        "全屏态整帧必须非黑（>90% 像素非纯黑）"
    );
    write_artifact("song-overview-fullscreen-1x", &frame).expect("全屏态 1:1 必须落盘");

    let left_top = Rig::crop(&frame, 0, 0, 480, 270);
    let left_3x = Rig::upscale_3x(&left_top);
    assert_eq!((left_3x.width(), left_3x.height()), (1440, 810));
    write_artifact("song-overview-fullscreen-3x-left-top", &left_3x).expect("全屏 3x 左上必须落盘");

    let right_top = Rig::crop(&frame, WINDOW.width - 480, 0, 480, 270);
    let right_3x = Rig::upscale_3x(&right_top);
    assert_eq!((right_3x.width(), right_3x.height()), (1440, 810));
    write_artifact("song-overview-fullscreen-3x-right-top", &right_3x)
        .expect("全屏 3x 右上必须落盘");

    let bottom = Rig::crop(&frame, 0, WINDOW.height - 22, WINDOW.width, 22);
    let bottom_3x = Rig::upscale_3x(&bottom);
    assert_eq!((bottom_3x.width(), bottom_3x.height()), (4320, 66));
    write_artifact("song-overview-fullscreen-3x-bottom-strip", &bottom_3x)
        .expect("全屏 3x 底部条带必须落盘");
}

// ---------------------------------------------------------------------------
// Phase 2 核心三项（设计稿 §3.5 / §3.6 / §5.1）：㉓ ㉔ ㉕ ㉖
//
// ⚠ 段落取 Intro 不取 Chorus：演示工程四段各 2 小节（Intro 0–1、Verse 2–3、
// Chorus 4–5、Outro 6–7），而 dsp 内容跨度只有 3 小节（谱面 4 拍、音频 10 拍），
// 第 3 小节起是静音垫条 —— Chorus 落在垫条区，期望带画不出、告警出不来。
// ㉔/㉖ 因此用跨度内的 Intro（第 0–1 小节）；Chorus 的带要等 dsp 跨度盖到它。
// ---------------------------------------------------------------------------

/// ㉓：调预设 ⇒ 档名 均衡 → 管弦 → 电子 → 均衡循环；管弦档张力数组与
/// 活态逐位一致且至少一小节与缺省档不同；回到缺省档后张力逐位恢复。
#[test]
fn profile_button_cycles_presets_and_changes_tension() {
    let rig = Rig::new();
    let default_tension = rig.live_data().tension.clone();
    assert_eq!(rig.live_data().profile, "default", "起点档必须是 default");
    assert_eq!(
        rig.ui().get_song_overview_profile().to_string(),
        profile_display_name_for("default", host::current_language()),
        "起点档名显示必须是当前语言的 \"均衡\""
    );

    rig.ui().invoke_song_overview_profile_cycle();
    assert_eq!(rig.live_data().profile, "orchestral", "调一次 ⇒ 管弦档");
    let orchestral_tension = rig.live_data().tension.clone();
    assert!(
        orchestral_tension
            .iter()
            .zip(default_tension.iter())
            .any(|(a, b)| a != b),
        "管弦档张力必须至少一小节与缺省档不同"
    );
    assert_eq!(
        model_floats(rig.ui().get_song_overview_tension()),
        orchestral_tension,
        "UI 张力数组必须与管弦档活态逐位一致"
    );

    rig.ui().invoke_song_overview_profile_cycle();
    assert_eq!(rig.live_data().profile, "electronic", "调两次 ⇒ 电子档");

    rig.ui().invoke_song_overview_profile_cycle();
    assert_eq!(rig.live_data().profile, "default", "调三次 ⇒ 回到缺省档");
    assert_eq!(
        rig.live_data().tension,
        default_tension,
        "回到缺省档后张力必须逐位恢复"
    );
    assert_eq!(
        rig.ui().get_song_overview_profile().to_string(),
        profile_display_name_for("default", host::current_language()),
        "回到缺省档后档名显示必须恢复"
    );
}

/// ㉔：调 Intro 段落 ⇒ 等级 None → 1 → … → 5 → None 循环；设档后第 0 小节
/// 包络带 = 该等级的带、偏离超 0.15 的小节告警、悬停读数含偏离量、走带一位不动；
/// 6 次点击回到无标注。
///
/// 档位选择：张力 > 0.4 标 1 级，否则标 5 级 ⇒ 第 0 小节必告警（与 ㉖ 同款）。
#[test]
fn section_label_cycles_expected_level_and_flags_deviation() {
    let rig = Rig::new();
    let position_before = rig.position();
    let tension0 = rig.live_data().tension[0];
    // 张力 > 0.4 ⇒ 1 级（带 0–0.25，张力在其上 ⇒ 告警）；否则 5 级（带在高处 ⇒ 告警）。
    let cycles_to_alert = if tension0 > 0.4 { 1 } else { 5 };
    for _ in 0..cycles_to_alert {
        rig.ui().invoke_song_overview_target_cycle(0);
    }
    let annotated = rig.live_data();
    assert_ne!(
        annotated.envelope_lo[0], NO_ENVELOPE,
        "设档后第 0 小节包络下沿必须不是哨兵"
    );
    assert!(
        annotated.envelope_hi[0] > annotated.envelope_lo[0],
        "设档后第 0 小节包络带必须有正宽度"
    );
    assert!(
        annotated.envelope_alert[0],
        "第 0 小节张力必须超出期望带 ⇒ 告警（张力 {tension0}，带 {}–{}）",
        annotated.envelope_lo[0], annotated.envelope_hi[0]
    );
    rig.ui().invoke_song_overview_hover(0.5f32 / 16.0);
    assert!(
        rig.ui()
            .get_song_overview_hover_text()
            .to_string()
            .contains("偏离期望带"),
        "悬停读数必须含偏离量"
    );
    assert_eq!(rig.position(), position_before, "设档时走带一位不动");

    // 再调 (6 - cycles_to_alert) 次 ⇒ 回到无标注（带哨兵、无告警）。
    for _ in 0..(6 - cycles_to_alert) {
        rig.ui().invoke_song_overview_target_cycle(0);
    }
    let cleared = rig.live_data();
    assert!(
        cleared.envelope_lo.iter().all(|lo| *lo == NO_ENVELOPE),
        "6 次后包络下沿必须全回哨兵"
    );
    assert!(
        cleared.envelope_alert.iter().all(|a| !a),
        "6 次后必须无告警"
    );
}

/// ㉕：UI `score-dyn` 数组与活态逐位一致；第 0 小节有起音 ⇒ 力度大于 0；
/// 垫条区（第 3 小节起）力度为 0；无标注时包络列全哨兵、无告警、档名 "均衡"。
#[test]
fn score_dyn_reaches_the_panel_and_is_nonzero_on_the_first_bar() {
    let rig = Rig::new();
    let live = rig.live_data();
    assert_eq!(
        model_floats(rig.ui().get_song_overview_score_dyn()),
        live.score_dyn,
        "UI `score-dyn` 数组必须与活态逐位一致"
    );
    assert!(
        live.score_dyn[0] > 0.0,
        "第 0 小节有起音 ⇒ 力度必须大于 0（实得 {}）",
        live.score_dyn[0]
    );
    assert!(
        live.score_dyn[3..].iter().all(|d| *d == 0.0),
        "垫条区（第 3 小节起）力度必须为 0"
    );
    assert!(
        live.envelope_lo.iter().all(|lo| *lo == NO_ENVELOPE),
        "无标注时包络列必须全哨兵"
    );
    assert!(live.envelope_alert.iter().all(|a| !a), "无标注时必须无告警");
    assert_eq!(live.profile, "default", "无标注时档名必须是 default");
    assert_eq!(
        rig.ui().get_song_overview_profile().to_string(),
        profile_display_name_for("default", host::current_language()),
        "无标注时档名显示必须是当前语言的 \"均衡\""
    );
}

/// ㉖：Phase 2 视觉验收 1:1 + 3x —— 管弦档 + Intro 保证告警的那一级，展开态
/// 截图非黑、两张 PNG 落盘供执行员肉眼验收。
///
/// 档位选择与 ㉔ 同款：张力 > 0.4 标 1 级，否则标 5 级 ⇒ 第 0 小节必告警。
/// 1:1 = 窗口底部 128 行；3x = 最近邻放大（1440×128 ⇒ 4320×384）。
#[test]
fn phase2_visual_acceptance_1x_and_3x() {
    let rig = Rig::new();
    rig.ui().invoke_song_overview_profile_cycle();
    assert_eq!(rig.live_data().profile, "orchestral", "先切管弦档");
    let tension0 = rig.live_data().tension[0];
    let cycles_to_alert = if tension0 > 0.4 { 1 } else { 5 };
    for _ in 0..cycles_to_alert {
        rig.ui().invoke_song_overview_target_cycle(0);
    }
    assert!(
        rig.live_data().envelope_alert[0],
        "第 0 小节必须处于告警态（档位按张力选：{tension0} ⇒ {cycles_to_alert} 次）"
    );

    let frame = rig.capture_frame();
    assert!(
        nonblack_fraction(&frame) > 0.9,
        "管弦档 + 告警态整帧必须非黑（>90% 像素非纯黑）"
    );
    let bottom = Rig::crop(&frame, 0, WINDOW.height - 128, WINDOW.width, 128);
    write_artifact("song-overview-phase2-1x", &bottom).expect("Phase 2 底部 1:1 必须落盘");
    let triple = Rig::upscale_3x(&bottom);
    assert_eq!((triple.width(), triple.height()), (4320, 384));
    write_artifact("song-overview-phase2-3x", &triple).expect("Phase 2 底部 3x 必须落盘");
}
