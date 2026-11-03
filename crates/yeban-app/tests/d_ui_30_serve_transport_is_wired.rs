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
    let (live, _port, window) = assemble("<d-ui-30-play>", true);

    // ---- 前置读数：这台装配只发过**装载即停住**那一条（`D-UI-32`）----
    assert_eq!(
        live.engine_transport_commands_applied(),
        1,
        "① 前提：装配 + 起引擎⛔ 不许自发「替用户按」的命令，但**必须**自发一条 `Stop` \
         （换代即停住，`D-UI-32`）—— 实测已经应用 {} 条。0 条 = 没停住（引擎的构造默认是\
         自由跑 `Playing`）；2 条以上 = 有第二个替用户按键的站点",
        live.engine_transport_commands_applied()
    );
    assert!(
        !live.engine_transport_running(),
        "① 前提：起引擎后必须是停住的（第一次按播放的语义才是起播）。⚠ 这条前提是\
         `D-UI-32` 修出来的：`EngineRuntime` 的走带**构造默认 `Playing`**（刻意的，见 \
         `crates/yeban-engine/src/transport.rs` 的 `Transport` 文档），控制面必须补一条 \
         `Stop` —— 本票第一版在这里读到的就是「按播放 = 停」"
    );
    assert_eq!(
        live.engine_transport_position_ticks(),
        0,
        "① 前提：`Stop` 只换状态、**位置保留**（新换代的位置本就在 0；实测 {} tick）",
        live.engine_transport_position_ticks()
    );
    assert!(
        !window.get_playing(),
        "① 前提：镜面⛔ 不许比引擎多说一句 —— 换代之后界面上必须是停住的"
    );

    window.invoke_toggle_play();

    assert_eq!(
        live.engine_transport_commands_applied(),
        2,
        "① 一次按下必须让**本装配的宿主**恰好再应用 1 条命令（实测累计 {} 条 = 装载那条 \
         + 这一条）—— 增量 0 = 回调没有注册者（修前的形状），增量 2 = 装了第二个注册者",
        live.engine_transport_commands_applied()
    );
    assert!(
        live.engine_transport_running(),
        "① 按下播放之后，引擎自己的读数必须是「在跑」（读的是 `TransportReading`，\
         ⛔ 不是界面属性）"
    );
    assert!(
        window.get_playing(),
        "① 界面镜面 `playing` 必须跟着引擎读数变真（它是投影，不能反过来当事实源）"
    );

    report_line(
        "[d-ui-30] ① 播放按钮 ⇒ 本装配宿主：装载即停住 + 按下 commands_applied=2、running=true",
    );
}

/// ② 那颗**停止**按钮 = 停住**并**回起点（`Stop` + `SeekTicks(0)`）。
#[test]
fn the_stop_button_stops_and_rewinds_through_the_engine() {
    let (mut live, _port, window) = assemble("<d-ui-30-stop>", true);

    window.invoke_toggle_play();
    live.drive_audio(DRIVE_QUANTA);
    let running_position = live.engine_transport_position_ticks();
    assert!(
        running_position > 0,
        "② 前提：推 {} 个量子之后播放头必须离开 0（读到 {} tick）—— 否则「回 0」\
         这一条是空转",
        DRIVE_QUANTA,
        running_position
    );

    let before = live.engine_transport_commands_applied();
    window.invoke_stop();
    // 命令在**量子边界**出队：`send_transport` 自己会推一跳，这里再补一跳，因此
    // 「两条都应用」不依赖某个 drain 深度。
    live.drive_audio(1);

    assert!(!live.engine_transport_running(), "② 按停止之后引擎必须停住");
    assert_eq!(
        live.engine_transport_position_ticks(),
        0,
        "② 停止按钮的语义是**停止并回起点**（位置读引擎镜面；实测 {} tick）",
        live.engine_transport_position_ticks()
    );
    assert!(
        live.engine_transport_commands_applied() >= before + 2,
        "② `Stop` + `SeekTicks(0)` 是**两条**命令（实测增量 {}）—— 只发一条 = 少了一半语义",
        live.engine_transport_commands_applied()
            .saturating_sub(before)
    );
    assert_eq!(
        window.get_playhead_ticks(),
        0,
        "② 界面上的播放头必须跟着归 0（`apply_transport` 的写者只有引擎读数）"
    );
    assert!(!window.get_playing(), "② 界面镜面 `playing` 必须回假");

    report_line(&format!(
        "[d-ui-30] ② 停止按钮：停住 + 回起点（按下前 {running_position} tick ⇒ 0 tick）"
    ));
}

/// ③ `#227` 的连按语义（第二次停、第三次回起点、第四次才是新的起播）在本装配里可达。
#[test]
fn the_double_tap_rewind_lands_on_the_same_engine() {
    let (mut live, _port, window) = assemble("<d-ui-30-double-tap>", true);

    // 第一次 `Space` ⇒ 起播。
    assert!(
        press_space(&window),
        "③ 前提：`Space` 必须被消费（本票修的就是「消费了却没人听」）"
    );
    assert!(
        live.engine_transport_running(),
        "③ 前提：第一次按下必须在跑"
    );
    let played_commands = live.engine_transport_commands_applied();

    // 第二次 ⇒ 停住、**位置保留**（不能顺手回起点，否则用户的"再听一遍刚才那几秒"没了）。
    assert!(press_space(&window), "③ 第二次 `Space` 必须被消费");
    assert!(
        !live.engine_transport_running(),
        "③ 第二次 `Space` 必须停住"
    );
    assert_eq!(
        live.engine_transport_commands_applied(),
        played_commands + 1,
        "③ 每一次按下都⛔ 只能发**一批**命令（这里恰好 +1）"
    );

    // 第三次（连按窗口内）⇒ 只是回起点，仍然是停住。
    assert!(press_space(&window), "③ 第三次 `Space` 必须被消费");
    live.drive_audio(1);
    assert_eq!(
        live.engine_transport_position_ticks(),
        0,
        "③ 连按的第三次必须是 `SeekTicks(0)`（实测 {} tick）",
        live.engine_transport_position_ticks()
    );
    assert!(
        !live.engine_transport_running(),
        "③ 回起点那一跳⛔ 不许顺手起播"
    );
    assert_eq!(
        live.engine_transport_commands_applied(),
        played_commands + 2,
        "③ 第三次只多发 1 条（`SeekTicks(0)`；实测累计增量 {}）",
        live.engine_transport_commands_applied()
            .saturating_sub(played_commands)
    );

    // 第四次 ⇒ 时刻已被清掉 ⇒ 是**新的第一次**（起播），⛔ 不是再定位一次。
    assert!(press_space(&window), "③ 第四下 `Space` 必须被消费");
    assert!(
        live.engine_transport_running(),
        "③ 第四下必须是新的起播（回起点那一跳已经把「待配对的停住时刻」清掉了；\
         不清就会把用户困在「按了播放却不动」里）"
    );

    report_line("[d-ui-30] ③ 连按 `Space`：停 → 回起点 → 新的起播，全在本装配的宿主上");
}

/// ④ `Space` 那条**键路**（`wire_keys` → `apply_action` → `invoke_toggle_play`）真的落到同一个宿主。
#[test]
fn the_space_key_reaches_the_engine_instead_of_only_being_consumed() {
    let (live, _port, window) = assemble("<d-ui-30-space>", true);
    let before = live.engine_transport_commands_applied();
    assert_eq!(
        before, 1,
        "④ 前提：这台装配只发过装载那条 `Stop`（`D-UI-32`；实测 {} 条）",
        before
    );

    // 裸键 `" "`（无修饰）走 `.slint` 的 `key-action` ⇒ `wire_keys` ⇒ `apply_action`。
    let consumed = press_space(&window);

    assert!(consumed, "④ `Space` 必须被消费（它有绑定）");
    assert_eq!(
        live.engine_transport_commands_applied(),
        2,
        "④ ⭐ 本票的正主：消费 ⇒ **必须**同时有命令进引擎（实测累计 {} 条 = 装载那条 \
         `Stop` + 这一条）。修前这里是 `consumed = true` 而计数停在 **0** —— 界面读起来\
         就是「按了没反应」",
        live.engine_transport_commands_applied()
    );
    assert!(
        live.engine_transport_running(),
        "④ 键路与按钮共用同一个宿主 ⇒ 按下之后必须在跑"
    );

    report_line(
        "[d-ui-30] ④ `Space`：consumed=true 且 commands_applied 累计 2（装载 + 这一条，同一个宿主）",
    );
}

/// ⑤ 那颗**节拍器**按钮在有会话时写的是文档（`D-UI-29` ⑧ 那条源码纪律的运行期版本）。
#[test]
fn the_metronome_button_writes_the_document_instead_of_swapping_the_engine() {
    let (mut live, port, window) = assemble("<d-ui-30-metronome>", true);
    let commits_at_start = port.display().commit_count;
    let revision_at_start = snapshot_revision(&live);

    // ---- 前提：装配期的镜面播种跟着文档（修前这条也红：`wire_transport` 没跑，\
    //      初值那句 `set_metronome_on` 因此从未执行）----
    assert_eq!(
        window.get_metronome_on(),
        doc_metronome_enabled(&port),
        "⑤ 前提：`metronome-on` 的初值必须来自文档（`wire_transport` 是它唯一的初值写者）"
    );

    window.invoke_toggle_metronome();

    assert!(
        doc_metronome_enabled(&port),
        "⑤ 按一次那颗按钮必须把 `transport.metronome_enabled` 翻成 `true`（权威那一份）"
    );
    assert_eq!(
        port.display().commit_count,
        commits_at_start + 1,
        "⑤ 必须**恰好**多一条提交（单位：条；实测增量 {}）",
        port.display().commit_count.saturating_sub(commits_at_start)
    );
    assert_eq!(
        snapshot_revision(&live),
        revision_at_start,
        "⑤ 按下这一瞬间⛔ 不换引擎（修订号一位不涨；发声交给心跳那一跳）"
    );
    assert!(
        window.get_metronome_on(),
        "⑤ 镜面必须立刻跟（不等心跳 ⇒ 判据装配里不跑心跳也能读到）"
    );

    // 下一跳：既有的发布腿把文档投影进快照（⛔ 不是这颗按钮自己换的引擎）。
    let tick = heartbeat(&mut live, &port);
    assert_eq!(
        tick.published_revision,
        Some(revision_at_start + 1),
        "⑤ 心跳必须发布**一代**新快照（实测 {:#?}）",
        tick.published_revision
    );
    assert_eq!(
        live.engine_metronome_enabled(),
        Some(true),
        "⑤ 快照里必须有节拍器计划了（播放侧真正在用的那一代）"
    );

    report_line("[d-ui-30] ⑤ 节拍器按钮：写文档 +1 条提交、不换引擎、下一跳进快照");
}

/// ⑥ 换工程**重接**一次：初值跟着新工程走，且 Slint 的覆盖语义 ⇒ 不是第二个注册者。
#[test]
fn applying_a_new_project_rewires_the_transport_exactly_once() {
    // 端口在场与否都不改本例判的两格（初值播种 + 覆盖语义），因此与其余各例**同一台**装配。
    let (mut live, _port, window) = assemble("<d-ui-30-rewire>", true);

    let mut other = yeban_model::samples::filled_project();
    other.transport.metronome_enabled = true;
    live.apply_project(&other)
        .expect("换工程（同一个活窗口上的重投影）");

    assert!(
        window.get_metronome_on(),
        "⑥ ⭐ 重接之后 `metronome-on` 必须跟着**新工程**（修前这一格从未被播种，换工程\
         更无从跟：读到的是假 ⇒ 界面与文档脱钩）"
    );

    let before = live.engine_transport_commands_applied();
    window.invoke_toggle_play();
    assert_eq!(
        live.engine_transport_commands_applied(),
        before + 1,
        "⑥ 一次按下必须只发**一批**命令（实测增量 {}）—— `+2` 就说明重接变成了\
         **追加**一个注册者（Slint 的 `on_*` 是设值语义，本条把它钉住）",
        live.engine_transport_commands_applied()
            .saturating_sub(before)
    );
    assert!(
        live.engine_transport_running(),
        "⑥ 换工程之后播放键必须仍然有效"
    );

    report_line("[d-ui-30] ⑥ 换工程：初值跟新工程、重接是覆盖（命令增量恰好 +1）");
}

/// ⑦ 没有引擎的那台装配：按下之后⛔ 不假装切换过（冷读数，一路都是 `false`）。
#[test]
fn without_an_engine_a_press_leaves_every_reading_stopped() {
    let (live, _port, window) = assemble("<d-ui-30-cold>", false);

    window.invoke_toggle_play();

    assert_eq!(
        live.engine_transport_commands_applied(),
        0,
        "⑦ 没起引擎 ⇒ 一条命令都⛔ 不可能被应用（`send_transport` 在 \
         `!transport_ready()` 时如实返回冷读数）；实测 {}",
        live.engine_transport_commands_applied()
    );
    assert!(
        !live.engine_transport_running(),
        "⑦ 没起引擎 ⇒ 读数必须是停住"
    );
    assert!(
        !window.get_playing(),
        "⑦ 界面镜面⛔ 不得比引擎多说一句（拿冷读数写成 `true` 就是假状态）"
    );

    report_line("[d-ui-30] ⑦ 无引擎装配：按下不消费状态，三个读数一路为冷（不假装）");
}

/// ⑧ 出处纪律（⛔ 不开窗口）：接线在装配里、宿主只有一份、执行面自己不发送走带命令。
#[test]
fn the_execution_surface_wires_once_and_never_sends_transport_itself() {
    let live = code_only(include_str!("../src/live_surface.rs"));
    let main = code_only(include_str!("../src/main.rs"));

    // 接线点：装配 + 换工程重接。⚠ 注释里的字面量已被 `code_only` 剥掉 ⇒ 这里数的是**实现站点**。
    assert_eq!(
        live.matches("host::wire_transport(").count(),
        2,
        "⑧ `live_surface.rs` 代码里 `host::wire_transport(` 必须是**恰好 2 处**（装配 + \
         `apply_project` 重接；实测 {} 处）—— 0 处正是本票修掉的缺陷",
        live.matches("host::wire_transport(").count()
    );
    // 产品那条腿必须还在（两族⛔ 不得互删）。
    assert_eq!(
        main.matches("host::wire_transport(").count(),
        1,
        "⑧ `main.rs` 代码里 `host::wire_transport(` 必须仍是 1 处（在 `wire_callbacks` 里，\
         换工程靠**重调** `wire_callbacks`）—— 实测 {} 处",
        main.matches("host::wire_transport(").count()
    );
    // 宿主只许有一份：`Rc` 计数 +1 交出去，⛔ 不是再造一个 `EngineHost`。
    assert_eq!(
        live.matches("EngineHost::new(").count(),
        1,
        "⑧ `live_surface.rs` 里 `EngineHost::new(` 必须**恰好 1 处**（实测 {} 处）—— \
         出现第二处就是「界面上按的」与「判据读的」两份走带",
        live.matches("EngineHost::new(").count()
    );
    // 执行面自己不**替用户**发送走带命令：唯一入口是 `wire_transport` 装的那些回调。
    // ⭐ 唯一合法的例外是「装载即停住」那一条 `Stop`（`D-UI-32`），它单独钉在下面。
    for token in ["send_transport(", ".play()", "stop_and_rewind(", ".seek("] {
        assert_eq!(
            live.matches(token).count(),
            0,
            "⑧ `live_surface.rs` 代码里 `{token}` 必须 0 处（实测 {} 处）—— 执行面自己\
             播/停/回起点/定位就等于替用户按了一次键（① 的前置读数与 ⑥ 的增量都失去意义）",
            live.matches(token).count()
        );
    }
    // ⭐ `D-UI-32`：装载即停住那一条**必须**在，而且**只在**换代入口里。
    assert_eq!(
        live.matches(".stop()").count(),
        1,
        "⑧ `live_surface.rs` 代码里 `.stop()` 必须**恰好 1 处**（实测 {} 处）—— \
         0 处 = 新一代引擎带着构造默认的 `Playing` 交给用户（界面一开就「在播」、播放头\
         自己走、第一次按播放是**停**，即 `D-UI-32`）；≥2 处 = 停住这一步被抄成了两处\
         事实源（那正是本缺陷的成因形态）",
        live.matches(".stop()").count()
    );
    assert_eq!(
        main.matches(".stop()").count(),
        2,
        "⑧ `main.rs` 里 `.stop()` 必须仍是 **2 处**（实测 {} 处）：装载即停住那一条 \
         + ALSA 设备挂载关闭那一条（`mount.stop()`，与走带无关）。两族⛔ 不得互删；\
         若掉到 1，界面上「打开即播放」就回来了，而本判据的 serve 那一半仍然绿 ⇒ \
         只有这条能看见",
        main.matches(".stop()").count()
    );
    // 初值那一格在**宿主**里写，⛔ 不在执行面重算（`apply_view` 不碰这一格：`D-UI-29` ⑩ 同口径）。
    assert_eq!(
        live.matches("set_metronome_on(").count(),
        0,
        "⑧ `live_surface.rs` 里 `set_metronome_on(` 必须 0 处：镜面由 `host` 那两条腿写\
         （初值 + 每跳校正），执行面⛔ 不自己算「下一个值」"
    );

    report_line(
        "[d-ui-30] ⑧ 源码纪律：live 接线 2 处 / main 1 处、宿主 1 份、执行面自发命令 0 处、\
         set_metronome_on 0 处",
    );
}
