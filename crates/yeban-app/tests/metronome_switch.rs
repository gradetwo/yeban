//! ⭐ **节拍器开关的「点得动、关得掉」判据**（`#16`，UI 侧那一半）。
//!
//! ## 被钉住的缺陷（接线前的推理，未上线）
//!
//! 拨动开关**不写工程文档**（`transport.metronome_enabled` 的文档级提交要走
//! `Op::SetTransportConfig`，那条通道尚未接线）。若"下一个值"永远从**文档字段**算，
//! 第一次拨动之后文档仍是 `false` ⇒ 第二次点击/按键会**再算出 `true`**
//! ⇒ **开关关不掉**（点了没反应）。⇒ 宿主引入运行态镜面 `MainWindow.metronome-on`
//! （`MODEL-ISO-001` 挥发性层，唯一写者是宿主），两个入口（按钮 / `Cmd/Ctrl+Alt+N`）
//! 共用**同一个**落点 `host::apply_engine_command`。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（可寻址） | `transport-metronome-button` 在运行时树里、角色 = `Button`、标签与注册表逐字一致；装配态 `metronome-on == 文档字段`（`false`） |
//! | ②（真点击 → 真引擎） | 注入真实指针点击 ⇒ 属性翻 `true`，走带**仍是 Playing**（换引擎不许破坏走带），推进 2 拍后 `metronome_clicks > 0`（⛔ 不是只看属性） |
//! | ③（关得掉） | 再点一次 ⇒ 属性回 `false`；推进 2 拍后 `metronome_clicks == 0`（新快照真的关掉了）—— 这正是"从文档算下一个值"会红的格 |
//! | ④（键盘同一落点） | `Cmd/Ctrl+Alt+N` 被消费 ⇒ 属性与音频面读数与点击**逐格相同**（开 ⇒ 有咔哒，再按 ⇒ 无咔哒） |
//! | ⑤（属性是唯一镜面） | 点击**只发** `toggle-metronome` 回调、⛔ 不在 `.slint` 里翻转属性；属性由宿主写 —— 由 ②③④ 的往返共同证明 |
//!
//! ## 走的是哪条腿
//!
//! 生产装配 [`ProductionLoop::start_with_audio_source`] ＋ 真 Tier-1 活窗口
//! （`host::build_main_window` / `wire_transport(.., Some(project), _)` / `wire_keys`）＋
//! [`EngineHost::render_into`]（内部即 `EngineRuntime::process_quantum`）。
//!
//! ## 边界（不许当成已证）
//!
//! - ⛔ **没有人耳听**：判据证明的是"咔哒计数与属性往返"，⛔ 不是听感。
//! - ⛔ **不证文档持久化**：拨动**刻意不写** `project.transport.metronome_enabled`
//!   （挥发性镜面）。保存/重开后恢复开关状态需要 `Op::SetTransportConfig` 提交通道，
//!   本票**如实登记为剩余缺口**，下方只打印文档字段读数，⛔ 不假装它变了。
//! - ⛔ `run_gui` 的事件循环没有执行：这里驱动的是它调用的**同一个**装配。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/console_maximize.rs` / `tests/rewind_sound.rs` 同款）。

use std::cell::RefCell;
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::elements::ElementKind;
use yeban_app::engine_host::EngineHost;
use yeban_app::host::{self, ProductionLoop};
use yeban_app::input::{Focus, InputContext};
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_engine::transport::TransportState;
use yeban_model::samples::filled_project;
use yeban_ui_test_port::{LivePort, Permission, PointerButton, Size, UiTestPort as _};

/// 必须 ≥ 1120：节拍器键几何上钉在徽章右侧的缝里（窄档按设计隐藏）。
const WINDOW: Size = Size::new(1440, 900);

/// 工程声道的兜底值（投影失败时才用）。
const FALLBACK_CHANNELS: u16 = 2;

/// 覆盖 ≥2 拍：120 BPM / 48 kHz ⇒ 1 拍 = 24_000 帧 = 187.5 个量子；
/// 400 个量子 = 51_200 帧 ⇒ 无论开关落点在拍内哪里，窗口里都**至少**有一次咔哒。
const QUANTA_TWO_BEATS: usize = 400;

struct Rig {
    /// 持有它 ⇒ `EngineRebuild::collector` 的消费端一直在（与 `main.rs` 同一装配）。
    _production: ProductionLoop,
    engine: Rc<RefCell<EngineHost>>,
    port: LivePort<MainWindow>,
    /// 装配层交给宿主的**同一份**工程（只读；打印它的 `metronome_enabled` 证明文档没被改）。
    project: yeban_model::YebanProjectV1,
}

impl Rig {
    fn new() -> Self {
        let project = filled_project();
        let production =
            ProductionLoop::start_with_audio_source(&project, 0, None).expect("产品装配建引擎");
        let engine = production.engine_handle();
        // `src/main.rs` 的同一行：新引擎默认自由跑 ⇒ 显式 `Stop`（界面初始态是停住）。
        engine.borrow_mut().stop();

        let view = ViewState::from_project(&project).expect("filled_project 必须能投影");
        let scene = DemoScene::from_view(&view);
        let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
            host::build_main_window(&view, &scene)
        })
        .expect("Tier-1 平台 + 生产窗口构造入口");

        // 与 `main.rs` 同一对装配函数：走带（⭐ 带工程 ⇒ 镜面初值来自文档）＋ 输入 ＋ 键盘。
        host::wire_transport(
            port.ui(),
            Rc::clone(&engine),
            Some(Rc::new(project.clone())),
            None,
        );
        let context = Rc::new(RefCell::new(InputContext::new()));
        // `Cmd/Ctrl+Alt+N` 的绑定在画布焦点下解析（与 `src/metronome.rs` 判据同一前置）。
        context.borrow_mut().set_focus(Focus::MainCanvas);
        host::wire_input(port.ui(), Rc::clone(&context));
        host::wire_keys(
            port.ui(),
            context,
            None,
            Some(Rc::clone(&engine)),
            Some(Rc::new(project.clone())),
        );
        host::apply_transport(port.ui(), engine.borrow().transport());

        Self {
            _production: production,
            engine,
            port,
            project,
        }
    }

    fn ui(&self) -> &MainWindow {
        self.port.ui()
    }

    fn metronome_on(&self) -> bool {
        self.ui().get_metronome_on()
    }

    fn transport_state(&self) -> TransportState {
        self.engine.borrow().transport().state
    }

    fn clicks(&self) -> u64 {
        self.engine
            .borrow()
            .engine_stats()
            .expect("控制腿必须有统计读数")
            .metronome_clicks
    }

    /// 推 `quanta` 个量子（`render_into` ⇒ `EngineRuntime::process_quantum`）。
    fn render(&self, quanta: usize) {
        let channels = {
            let engine = self.engine.borrow();
            engine
                .current_snapshot()
                .map_or(FALLBACK_CHANNELS, |snapshot| snapshot.channels().max(1))
        };
        let block = DEFAULT_BLOCK_FRAMES * usize::from(channels);
        let mut samples = vec![0.0_f32; block * quanta];
        let driven = self
            .engine
            .borrow_mut()
            .render_into(&mut samples, channels, quanta as u64);
        assert_eq!(
            driven, quanta as u64,
            "render_into 必须推进请求的量子数（控制腿没有设备）"
        );
    }

    /// 真实指针点击（按下 → 松手）⇒ 命中 `metronome_area` 的 `TouchArea`，⛔ 不是直接调 Rust 回调。
    fn click_metronome(&mut self) {
        // 每次点击前重抓树（`dispatch_*` 用的是端口里**缓存的**几何）。
        self.port.refresh_tree(None).expect("必须能重抓控件树");
        self.port
            .dispatch_pointer_down(
                "transport-metronome-button",
                16.0,
                16.0,
                PointerButton::Left,
            )
            .expect("指针按下必须落到节拍器键");
        self.port
            .dispatch_pointer_up(PointerButton::Left)
            .expect("指针释放必须落到活窗口");
    }

    /// 按 `Cmd/Ctrl+Alt+N`（走 `.slint` 的 `key-action` ⇒ `wire_keys` ⇒ `apply_action`，
    /// 与生产同一条链）。返回值 = `.slint` 的 `EventResult`（是否被消费）。
    fn press_metronome_key(&self) -> bool {
        // 回调参数顺序与 `app.slint` 的声明一致：(text, shift, ctrl, alt, meta)。
        self.ui()
            .invoke_key_action("n".into(), false, true, true, false)
    }
}

#[test]
fn the_metronome_switch_round_trips_from_both_entries() {
    let mut rig = Rig::new();

    // ---- ① 可寻址：节点在场、角色 / 标签与注册表一致 ----
    {
        let tree = rig.port.refresh_tree(None).expect("重抓");
        let node = tree
            .find_by_id("transport-metronome-button")
            .expect("`transport-metronome-button` 必须在运行时控件树里（1440 宽档）");
        assert_eq!(
            node.role.as_str(),
            ElementKind::Button.accessible_role(),
            "节拍器键的角色必须与注册表一致"
        );
        assert_eq!(
            node.label, "节拍器 (Cmd/Ctrl + Alt + N)",
            "无障碍名必须与注册表逐字一致"
        );
    }
    assert!(
        !rig.metronome_on(),
        "装配态必须等于文档字段（filled_project 的 metronome_enabled = false）"
    );
    assert!(
        !rig.project.transport.metronome_enabled,
        "前置：文档字段必须是 false（镜面初值的来源）"
    );

    // ---- 起播：咔哒声只在跑的时候才有意义（与 `src/metronome.rs` 判据同一前置）----
    rig.ui().invoke_toggle_play();
    assert_eq!(
        rig.transport_state(),
        TransportState::Playing,
        "前置：必须正在播放"
    );

    // ---- ② 点击开：属性翻 true，走带不被换引擎破坏，推进 2 拍有咔哒 ----
    rig.click_metronome();
    assert!(
        rig.metronome_on(),
        "第一次点击必须把 `metronome-on` 翻成 true"
    );
    assert_eq!(
        rig.transport_state(),
        TransportState::Playing,
        "拨节拍器不许把播放变成停住（换引擎必须恢复走带状态）"
    );
    assert_eq!(rig.clicks(), 0, "新引擎在推进之前还没有咔哒");
    rig.render(QUANTA_TWO_BEATS);
    let on_clicks = rig.clicks();
    println!(
        "[#16/②] 点击开：metronome-on=true；推进 {QUANTA_TWO_BEATS} 个量子后 metronome_clicks={on_clicks}"
    );
    assert!(
        on_clicks > 0,
        "⭐ 开之后推进 ≥2 拍必须有咔哒（实测 {on_clicks}）"
    );

    // ---- ③ 再点击关：属性回 false，推进 2 拍不再有咔哒（往返）----
    rig.click_metronome();
    assert!(
        !rig.metronome_on(),
        "⭐ 第二次点击必须把开关**关掉**（旧实现从文档算下一个值 ⇒ 这里恒为 true）"
    );
    assert_eq!(
        rig.transport_state(),
        TransportState::Playing,
        "关节拍器同样不许破坏走带"
    );
    assert_eq!(
        rig.clicks(),
        0,
        "关掉之后换上的新引擎应当是「关」的快照（计数已归零）"
    );
    rig.render(QUANTA_TWO_BEATS);
    let off_clicks = rig.clicks();
    println!(
        "[#16/③] 点击关：metronome-on=false；推进 {QUANTA_TWO_BEATS} 个量子后 metronome_clicks={off_clicks}"
    );
    assert_eq!(
        off_clicks, 0,
        "⭐ 关掉之后推进 ≥2 拍必须**不再**有咔哒（实测 {off_clicks}）"
    );

    // ---- ④ 键盘同一落点：`Cmd/Ctrl+Alt+N` 与点击逐格同形 ----
    assert!(rig.press_metronome_key(), "⭐ `Cmd/Ctrl+Alt+N` 必须被消费");
    assert!(
        rig.metronome_on(),
        "键盘开必须写**同一个**镜面（第一次按 ⇒ true）"
    );
    assert_eq!(rig.transport_state(), TransportState::Playing);
    rig.render(QUANTA_TWO_BEATS);
    let key_on_clicks = rig.clicks();
    println!("[#16/④] 键盘开：metronome-on=true；metronome_clicks={key_on_clicks}");
    assert!(
        key_on_clicks > 0,
        "⭐ 键盘开之后推进 ≥2 拍必须有咔哒（实测 {key_on_clicks}）"
    );

    assert!(rig.press_metronome_key(), "第二次按键也必须被消费");
    assert!(
        !rig.metronome_on(),
        "⭐ 键盘再按必须关得掉（同一镜面 ⇒ 同一往返）"
    );
    assert_eq!(rig.transport_state(), TransportState::Playing);
    rig.render(QUANTA_TWO_BEATS);
    let key_off_clicks = rig.clicks();
    println!("[#16/④] 键盘关：metronome-on=false；metronome_clicks={key_off_clicks}");
    assert_eq!(
        key_off_clicks, 0,
        "⭐ 键盘关掉之后推进 ≥2 拍必须**不再**有咔哒（实测 {key_off_clicks}）"
    );

    // ---- ⑤ 如实登记的边界：拨动**不写文档**（挥发性镜面）----
    println!(
        "[#16/⑤] 文档字段 transport.metronome_enabled = {}（⛔ 拨动不写文档：\
         持久化需 `Op::SetTransportConfig` 提交通道，本票登记为剩余缺口）",
        rig.project.transport.metronome_enabled
    );
    assert!(
        !rig.project.transport.metronome_enabled,
        "本判据钉住「镜面是挥发性层」：文档字段在四次拨动之后仍然一位不动"
    );
}
