//! ⭐ **节拍器开关的「点得动、关得掉」判据**（`#16`，UI 侧那一半）。
//!
//! ## ⭐ 本文件钉的是**哪一条腿**（`D-UI-29` 之后重读）
//!
//! 节拍器拨动有两条腿：**有权威会话**时提交 `Op::SetTransportConfig`（文档 ⇒ 60 Hz
//! 发布腿 ⇒ 新快照；⭐ 可撤销、⭐ 会持久化、⛔ 不动走带），**没有会话**时才落回
//! "当场换一代引擎"（`crate::metronome::set_enabled`）。
//!
//! 本文件的夹具把端口传成 `None`（`wire_transport(.., None)` / `wire_keys(.., None, ..)`）
//! ⇒ 它证的正是**后一条腿**：镜面当累加器、换代真的出声、走带不被换代打坏。
//! ⛔ 它因此**不**再是"整条节拍器链"的判据 —— 文档那一半由
//! `tests/d_ui_29_metronome_writes_the_document.rs` 钉（那里的装配形有端口）。
//!
//! ## 被钉住的缺陷（接线前的推理，未上线）
//!
//! 在**没有权威会话**这一形里，拨动**不写工程文档**（没有 `UndoPort` ⇒ 没有地方提交
//! `Op::SetTransportConfig`）。若"下一个值"永远从**文档字段**算，第一次拨动之后文档仍是
//! `false` ⇒ 第二次点击/按键会**再算出 `true`** ⇒ **开关关不掉**（点了没反应）。
//! ⇒ 宿主引入运行态镜面 `MainWindow.metronome-on`（`MODEL-ISO-001` 挥发性层，
//! 唯一写者是宿主），两个入口（按钮 / `Cmd/Ctrl+Alt+N`）共用**同一个**落点
//! `host::apply_action` 的节拍器臂。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（可寻址） | `transport-metronome-button` 在运行时树里、角色 = `Button`、标签与注册表逐字一致；装配态 `metronome-on == 文档字段`（`false`） |
//! | ②（真点击 → 真引擎） | 注入真实指针点击 ⇒ 属性翻 `true`，走带**仍是 Playing**（换引擎不许破坏走带），再推进 2 拍后 `metronome_clicks` **新增**（⛔ 不是只看属性；换引擎的重建自身会 `pump` 出 tick 0 那一次，最多 1） |
//! | ③（关得掉） | 再点一次 ⇒ 属性回 `false`；推进 2 拍后 `metronome_clicks == 0`（新快照真的关掉了）—— 这正是"从文档算下一个值"会红的格 |
//! | ④（键盘同一落点） | `Cmd/Ctrl+Alt+N` 被消费 ⇒ 属性与音频面读数与点击**逐格相同**（开 ⇒ 有咔哒，再按 ⇒ 无咔哒） |
//! | ⑤（属性是唯一镜面） | 点击**只发** `toggle-metronome` 回调、⛔ 不在 `.slint` 里翻转属性；属性由宿主写 —— 由 ②③④ 的往返共同证明 |
//! | ⑥（这条腿不写文档） | 本装配**没有端口** ⇒ 拨动只动镜面与快照；文档字段在四次拨动之后**一位不动**（如实打印读数）。⭐ 有端口那一形由 `tests/d_ui_29_metronome_writes_the_document.rs` 判 |
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
//! - ⛔ **不证文档持久化**：本装配**没有** `UndoPort`（夹具刻意传 `None`）⇒ 没有可提交的
//!   对象，拨动只动镜面与当代快照。文档那一半（⭐ 可撤销、⭐ 会持久化）已由
//!   `D-UI-29` 接上并另有判据（`tests/d_ui_29_metronome_writes_the_document.rs`）；
//!   本文件下方只打印文档字段读数，⛔ 不假装它变了。
//! - ⛔ `run_gui` 的事件循环没有执行：这里驱动的是它调用的**同一个**装配。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/console_maximize.rs` / `tests/rewind_sound.rs` 同款）。

#![allow(unused)]
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("callback toggle-play("),
        "新 UI 契约面必须声明 callback toggle-play("
    );
    assert!(
        aether_host.contains("on_toggle_play"),
        "宿主接线层必须注册 on_toggle_play"
    );
}
