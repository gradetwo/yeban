//! **`--enable-ui-mcp-http` 那份装配自己接了走带** —— `D-UI-30` 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 在**无头执行面**（`build_live_ui_with` ⇒ 人真的拿来作曲的那份装配）里，走带区那颗
//! **播放**、那颗**停止**、那颗**节拍器**，以及键盘上那一下 `Space`，都必须把命令发给
//! **本装配自己**的那个 `EngineHost` —— 而⛔ 不是由判据替它接好线之后再"能用"。
//!
//! ## 修前的形状（实测读数）
//!
//! `host::wire_transport` 的唯一调用点过去在 `main.rs` 的 `wire_callbacks` 里，而
//! `--enable-ui-mcp-http` 在 `main()` 里**提前**分流进 `ui_mcp_serve::run` ⇒ 那一段根本不跑。
//! 现读（剥掉 `//` 行之后的**代码**行数，单位：处）：`main.rs` = 1、`live_surface.rs` = **0**。
//! Slint 的 `on_*` 没注册者就是**空操作**，于是：
//!
//! 1. ⛔ 播放 / 停止 / 录音 / 节拍器四颗按钮按下去什么都不动（回调没人听）；
//! 2. ⛔ `Space` 比"不消费"**更坏** —— `wire_keys` 的 `PlayPause` 臂落到
//!    `ui.invoke_toggle_play()`，`apply_action` 因此返回 `true`（="这一键有绑定"），
//!    而那条回调没人听 ⇒ 控制面读到"按了、被消费了"，界面上什么都不发生；
//! 3. ⛔ `metronome-on` 的**装配期初值**一起丢（`wire_transport` 是它唯一的初值写者）⇒
//!    文档写着开、界面画着关。
//!
//! ⚠ **为什么此前没有判据抓到它**：判据侧**各自补一次** `host::wire_transport`
//! （`tests/metronome_switch.rs` / `tests/rewind_sound.rs` / `tests/transport_timecode_tick.rs`
//! / `tests/arrangement_ruler_seek.rs` / `tests/d_ui_11_serve_header_gestures.rs`），而且补的是
//! **自己新建的** `Rc<RefCell<EngineHost>>` ⇒ "这份装配接没接、接的是不是同一个宿主"
//! 从外面**永远读不出来**。因此本文件的**第一条纪律**就是：⛔ 判据不调 `wire_transport`，
//! 只按界面上的那颗键，然后读**装配自己的**宿主。
//! 同一族的机械防线是 `tests/d_ui_11_serve_header_gestures.rs` 的名单对账（本票把
//! `host::wire_transport` 从"只在 GUI 有"的例外清单里**删掉**了）。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（按钮 ⇒ 本装配的宿主） | 前置读数 `commands_applied = 1`（装载即停住那一条 `Stop`，`D-UI-32`）、`running = false`、位置 `0`、镜面 `playing = false`；按一次播放 ⇒ 同一宿主 `commands_applied` **+1**（累计 2）、`running = true`、界面镜面 `playing` 跟着变真。⭐ 修前这条红在**第一件事**上（回调无注册者 ⇒ 计数恒 0） |
//! | ②（停止 = 停住**并**回起点） | 起播 + 推 64 个量子 ⇒ 播放头离开 0；按停止 ⇒ 状态停 **且** 位置回 `0`（`Stop` + `SeekTicks(0)` 两条命令都应用）＋ 界面 `playhead-ticks` 归 0 |
//! | ③（`#227` 那套连按状态在本装配里活着） | 播放 → `Space` 停 → `Space` 再按 ⇒ **只是**回起点（仍停住、位置 0、命令 +1）；第四下才是新的起播 |
//! | ④（`Space` 那条键路接到同一个宿主） | 全新装配 + 起引擎 ⇒ 前置读数**恰好** 1（只有装载那条 `Stop`）；注入 `" "` 返回 `true`（消费）**且** `commands_applied` **+1**（累计 2）＋ `running = true`。⭐ 修前正是"返回 `true` 而计数恒 0"这个组合 |
//! | ⑤（节拍器那颗按钮真的写文档） | 带会话装配里按一次 ⇒ 文档 `false → true` ＋ 恰好 `+1` 条提交 ＋ 快照**修订号**一位不涨（⛔ 不换引擎）；下一跳心跳 ⇒ 快照变 `true`。这是 `D-UI-29` ⑧ 那条"两个入口、一个落点"的**运行期**版本 |
//! | ⑥（换工程重接一次，且是**覆盖**不是第二个注册者） | `apply_project(新工程)` ⇒ 界面 `metronome-on` 的初值**跟着新工程走**（真值为 `true`），而一次播放按下 ⇒ 命令计数**恰好 +1**（双注册会是 `+2`） |
//! | ⑦（没有引擎时如实冷读，⛔ 不假装） | 不起引擎的装配里按播放 ⇒ `commands_applied` 仍 `0`、`running` 仍 `false`、界面 `playing` 仍 `false`（`EngineHost::send_transport` 在 `!transport_ready()` 时返回 `TransportReading::cold()`） |
//! | ⑧（出处纪律） | 从源码现读（先剥 `//` 行 ⇒ **注释不是实现站点**）：`live_surface.rs` 里 `host::wire_transport(` = **2 处**（装配 + 换工程重接）、`EngineHost::new(` = **恰好 1 处**（⛔ 第二份宿主）、执行面**替用户**发的命令 = **0 处**（`send_transport(` / `.play()` / `stop_and_rewind(` / `.seek(` 各 0）⇒ 发那些命令的唯一入口是界面回调；唯一的例外是装载即停住那一条 `.stop()`，它必须**恰好 1 处**且在换代入口里（`D-UI-32`），而 `main.rs` 的 `.stop()` 必须仍是 **2 处**（装载停住 + ALSA 挂载关闭） |
//!
//! ## 注入（每一条都由本文件的判据打红）
//!
//! * **注入 ①「把装配期那一句 `wire_transport` 删掉」**（回到修前形状）⇒ ①②③④ 全红
//!   （回调无注册者 ⇒ 命令计数恒 0）；
//! * **注入 ②「宿主留回裸值、由判据自己补接线」**⇒ ⑧ 红（`wire_transport(` 2 处 → 1 处）
//!   ＋ ①②③④ 红（判据⛔ 不许调 `wire_transport`，本文件没有那句就装不出走带）；
//! * **注入 ③「`apply_project` 里不再重接」**⇒ ⑥ 红（`metronome-on` 停在旧工程的初值）；
//! * **注入 ④「`apply_project` 里再装一次成追加语义」**（假设 Slint 改成注册列表）⇒ ⑥ 的
//!   命令计数 `+1` 变 `+2`；
//! * **注入 ⑤「换代之后**不**停住」（把 `rebuild_engine` 里那条 `stop()` 删掉）**⇒ ① 的前置
//!   读数 `1` 变 `0`、`running` 由假变**真**（引擎的构造默认就是 `Playing`）、镜面 `playing`
//!   变真；⑧ 的 `.stop()` 计数 `1` 变 `0`。⚠ 这一格**本票第一版就是红在这一处**：判据当时
//!   把"起引擎后必须停住"当成**显然成立**的前提，而它其实是控制面必须做、`main.rs` 做了、
//!   serve 没做的一步 ⇒ 单独立票 `D-UI-32`（判据 `tests/d_ui_32_serve_parks_every_new_engine.rs`）；
//! * **注入 ⑥「节拍器那颗按钮改回拿装配期工程换引擎」**（去掉 `metronome_undo.is_some()`
//!   那一支）⇒ ⑤ 红（文档不翻、修订号涨）。
//!
//! ## 如实登记的边界（⛔ 不假装整条链都通了）
//!
//! 1. **录音键只证"有了注册者"，不证落盘**：`wire_transport` 内部经
//!    `host::wire_record_at` 注册 `on_toggle_record`，而它要一个**采集设备**
//!    （`default_input_factory()`）。无头 CI 上开不开得因机而异 ⇒ 本文件⛔ 不按录音键，
//!    免得把设备依赖与写文件的副作用引进判据。落盘那侧由 `tests/record_*.rs` 那族判。
//! 2. **不证明"听得到"**：②③ 读的是**引擎自己的走带读数**（RT→控制侧镜面），逐样本
//!    那一侧由 `crates/yeban-engine` 的判据负责。
//! 3. **③ 依赖 1.5 s 连按窗口**：`#227` 的 `HEAD_DOUBLE_TAP_WINDOW` 用 `Instant`，
//!    本例三次注入之间没有任何 I/O（只有微秒级的 Slint 调用 + 推量子），因此窗口恒成立；
//!    同族做法见 `tests/rewind_sound.rs`（它也直接从界面上按 `Space`）。
//! 4. **`wire_keys` 的无端口回退臂仍拿 `engine: None`**（本票⛔ 不扩范围）：因此
//!    "没有会话"的装配里节拍器键照旧**不消费**（`D-UI-29` ⑨ 已把那条形状钉住）。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部且每线程只能装配一次；每例一新线程由 libtest
//! 负责 ⇒ 本文件可以有多个 `#[test]`（与 `tests/clip_mute_toggle.rs` 同一做法）。
//! ⑧ 不建窗口（纯读源码），因此⛔ 不碰平台。
//!
//! ## 本机不编译重依赖（`AGENTS.md` §5）
//!
//! Slint 是重依赖 ⇒ 本文件在本机**不编译**；编译与判决交 CI。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的装配入口、走带读数与心跳 ⇒ 其余项在本测试目标里是
// dead_code（`-D warnings` 会判红）。同款见 `tests/engine_error_banner.rs`。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveUi, LiveWiringOptions, build_live_ui_with};
use slint::ComponentHandle;
use yeban_app::engine_host::EditMark;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;

/// 够让播放头离开 0 的量子数（单位：个；一个量子 = 128 帧）。
///
/// ⚠ 只用来**证明位置动过**，不用来断言具体 tick 数 —— 那会掺进夹具 BPM 的算术。
/// 同一条前提（推量子 ⇒ `position_ticks > 0`）在 CI 上已被 `tests/transport_timecode_tick.rs`
/// 钉着（那边推 3 个、断言 `> 0`），因此这里不新造一种对平台的假设。
const DRIVE_QUANTA: u64 = 64;

/// 装配一台**带权威会话**的活窗口（端口当装配选项传进去 ⇒ `host::wire_transport` /
/// `host::wire_keys` 从第一刻起就握着它，与产品进程同一形态）。
///
/// `start = true` ⇒ 顺手起一代引擎，而它**必须是停住的**（装载即停住 = `D-UI-32`；
/// ⛔ 不是"引擎构造完自己在跑"—— 那会让下面每一条 `+1` 都读成"按下是停"）。
///
/// ⛔ 本函数**不调** `host::wire_transport`：接线必须是装配自己的事（那是本票的命题）。
fn assemble(tag: &str, start: bool) -> (LiveUi, Rc<UndoPort>, MainWindow) {
    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open(tag, "yeban-app", project.clone(), NOW_MS).expect("打开会话"),
    ));
    let mut live = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            // ⚠ 0 = 起引擎时⛔ 不推量子 ⇒ 每一条读数都只可能来自「装配」或「按键」。
            engine_quanta: 0,
            undo: Some(Rc::clone(&port)),
            ..LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 执行面");
    if start {
        live.start_engine().expect("起一代引擎");
    }
    let window = live.ui().clone_strong();
    (live, port, window)
}

/// 权威文档的一份快照（读不到 ⇒ 前提不成立，直接判红）。
fn live_project(port: &UndoPort) -> yeban_model::YebanProjectV1 {
    port.try_project()
        .expect("端口必须有一份权威工程（没有 ⇒ 本文件的每一条都无从判起）")
}

/// 文档里的节拍器开关（单位：**布尔** —— 权威那一份，⛔ 不是界面属性）。
fn doc_metronome_enabled(port: &UndoPort) -> bool {
    live_project(port).transport.metronome_enabled
}

/// 快照的修订号（单位：**修订号**，换代才涨）—— "这颗按钮有没有换引擎"的读数。
fn snapshot_revision(live: &LiveUi) -> u64 {
    live.engine_snapshot_counts()
        .expect("夹具已经起引擎")
        .revision
}

/// 生产 `run_gui` 那 60Hz 心跳的一跳（`src/main.rs` 调用序列的逐字复制）。
fn heartbeat(live: &mut LiveUi, port: &UndoPort) -> live::EngineTick {
    let mark = EditMark::from_display(&port.display());
    let project = live_project(port);
    live.engine_heartbeat(mark, Some(&project))
}

/// 注入一次裸 `Space`（`text, shift, control, alt, meta` 与 `key-action` 逐位对齐；
/// 同款拼法见 `tests/rewind_sound.rs` 的 `press`）。
fn press_space(ui: &MainWindow) -> bool {
    ui.invoke_key_action(" ".into(), false, false, false, false)
}

/// 把源码压成**只有代码行**：注释里出现过的字面量不算实现站点
/// （口径实测于 `tests/search_focus_key.rs`，同款见 `D-UI-29` ⑩）。
fn code_only(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// ① 那颗**播放**按钮把命令发给了本装配自己的宿主（判据侧一行接线都没做）。
#[test]
fn the_play_button_drives_the_assemblys_own_engine() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
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
        aether_app.contains("> metronome-on:"),
        "新 UI 契约面必须声明 > metronome-on:"
    );
    assert!(
        aether_host.contains("set_metronome_on"),
        "宿主接线层必须注册 set_metronome_on"
    );
    assert!(
        aether_app.contains("> playhead-ticks:"),
        "新 UI 契约面必须声明 > playhead-ticks:"
    );
    assert!(
        aether_host.contains("set_playhead_ticks"),
        "宿主接线层必须注册 set_playhead_ticks"
    );
}

/// ② 那颗**停止**按钮 = 停住**并**回起点（`Stop` + `SeekTicks(0)`）。
#[test]
fn the_stop_button_stops_and_rewinds_through_the_engine() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
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
        aether_app.contains("> metronome-on:"),
        "新 UI 契约面必须声明 > metronome-on:"
    );
    assert!(
        aether_host.contains("set_metronome_on"),
        "宿主接线层必须注册 set_metronome_on"
    );
    assert!(
        aether_app.contains("> playhead-ticks:"),
        "新 UI 契约面必须声明 > playhead-ticks:"
    );
    assert!(
        aether_host.contains("set_playhead_ticks"),
        "宿主接线层必须注册 set_playhead_ticks"
    );
}

/// ③ `#227` 的连按语义（第二次停、第三次回起点、第四次才是新的起播）在本装配里可达。
#[test]
fn the_double_tap_rewind_lands_on_the_same_engine() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
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
        aether_app.contains("> metronome-on:"),
        "新 UI 契约面必须声明 > metronome-on:"
    );
    assert!(
        aether_host.contains("set_metronome_on"),
        "宿主接线层必须注册 set_metronome_on"
    );
    assert!(
        aether_app.contains("> playhead-ticks:"),
        "新 UI 契约面必须声明 > playhead-ticks:"
    );
    assert!(
        aether_host.contains("set_playhead_ticks"),
        "宿主接线层必须注册 set_playhead_ticks"
    );
}

/// ④ `Space` 那条**键路**（`wire_keys` → `apply_action` → `invoke_toggle_play`）真的落到同一个宿主。
#[test]
fn the_space_key_reaches_the_engine_instead_of_only_being_consumed() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
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
        aether_app.contains("> metronome-on:"),
        "新 UI 契约面必须声明 > metronome-on:"
    );
    assert!(
        aether_host.contains("set_metronome_on"),
        "宿主接线层必须注册 set_metronome_on"
    );
    assert!(
        aether_app.contains("> playhead-ticks:"),
        "新 UI 契约面必须声明 > playhead-ticks:"
    );
    assert!(
        aether_host.contains("set_playhead_ticks"),
        "宿主接线层必须注册 set_playhead_ticks"
    );
}

/// ⑤ 那颗**节拍器**按钮在有会话时写的是文档（`D-UI-29` ⑧ 那条源码纪律的运行期版本）。
#[test]
fn the_metronome_button_writes_the_document_instead_of_swapping_the_engine() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
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
        aether_app.contains("> metronome-on:"),
        "新 UI 契约面必须声明 > metronome-on:"
    );
    assert!(
        aether_host.contains("set_metronome_on"),
        "宿主接线层必须注册 set_metronome_on"
    );
    assert!(
        aether_app.contains("> playhead-ticks:"),
        "新 UI 契约面必须声明 > playhead-ticks:"
    );
    assert!(
        aether_host.contains("set_playhead_ticks"),
        "宿主接线层必须注册 set_playhead_ticks"
    );
}

/// ⑥ 换工程**重接**一次：初值跟着新工程走，且 Slint 的覆盖语义 ⇒ 不是第二个注册者。
#[test]
fn applying_a_new_project_rewires_the_transport_exactly_once() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
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
        aether_app.contains("> metronome-on:"),
        "新 UI 契约面必须声明 > metronome-on:"
    );
    assert!(
        aether_host.contains("set_metronome_on"),
        "宿主接线层必须注册 set_metronome_on"
    );
    assert!(
        aether_app.contains("> playhead-ticks:"),
        "新 UI 契约面必须声明 > playhead-ticks:"
    );
    assert!(
        aether_host.contains("set_playhead_ticks"),
        "宿主接线层必须注册 set_playhead_ticks"
    );
}

/// ⑦ 没有引擎的那台装配：按下之后⛔ 不假装切换过（冷读数，一路都是 `false`）。
#[test]
fn without_an_engine_a_press_leaves_every_reading_stopped() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
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
        aether_app.contains("> metronome-on:"),
        "新 UI 契约面必须声明 > metronome-on:"
    );
    assert!(
        aether_host.contains("set_metronome_on"),
        "宿主接线层必须注册 set_metronome_on"
    );
    assert!(
        aether_app.contains("> playhead-ticks:"),
        "新 UI 契约面必须声明 > playhead-ticks:"
    );
    assert!(
        aether_host.contains("set_playhead_ticks"),
        "宿主接线层必须注册 set_playhead_ticks"
    );
}

/// ⑧ 出处纪律（⛔ 不开窗口）：接线在装配里、宿主只有一份、执行面自己不发送走带命令。
#[test]
fn the_execution_surface_wires_once_and_never_sends_transport_itself() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
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
        aether_app.contains("> metronome-on:"),
        "新 UI 契约面必须声明 > metronome-on:"
    );
    assert!(
        aether_host.contains("set_metronome_on"),
        "宿主接线层必须注册 set_metronome_on"
    );
    assert!(
        aether_app.contains("> playhead-ticks:"),
        "新 UI 契约面必须声明 > playhead-ticks:"
    );
    assert!(
        aether_host.contains("set_playhead_ticks"),
        "宿主接线层必须注册 set_playhead_ticks"
    );
}
