//! **每一代新引擎交到用户手上时必须是停住的** —— `D-UI-32` 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 在**无头执行面**（`build_live_ui_with` ⇒ 人真的拿来作曲的那份装配）里，每一次**换代**
//! （起引擎 / 换工程 / `ui/reload_engine`）之后，本装配自己的宿主必须读起来是
//! **停住**（`TransportState::Stopped`）、位置在 `0`、界面上那颗播放键显示"没在播"，
//! 而**第一次**按播放的语义必须是**起播**（⛔ 不是"停住"）。
//!
//! ## 修前的形状（实测读数）
//!
//! `EngineRuntime` 的走带**构造默认是 `Playing`**（`Transport::free_running`，`crates/yeban-engine/src/rt.rs`
//! 里现读 `put!(transport, Transport::free_running(...))` = **1 处**）。那是刻意的、⛔ 不是缺陷：
//! 接入走带之前 `process_quantum` 的语义就是"快照一发布就从 tick 0 起滚"，默认值改成停住会让
//! 引擎侧那族既有判据变红。引擎侧为此把出口写死在自己的文档里（现读
//! `crates/yeban-engine/src/transport.rs` 的 `Transport` 文档）：
//!
//! > 要"加载即停住"的控制面发一条 `Stop` 即可（`yeban-app::engine_host` 就是这么做的）
//!
//! 于是**这一步是控制面的义务**。两条腿里：
//!
//! | 腿 | 装载即停住 | 现读 `.stop()`（代码行，剥掉 `//` 与 `///` 之后） |
//! | :--- | :--- | :--- |
//! | 生产 GUI（`src/main.rs`） | ✅ 建完引擎就 `engine_handle().borrow_mut().stop()` | **2 处**（装载停住 + ALSA 挂载 `mount.stop()`） |
//! | serve 执行面（`src/live_surface.rs`） | ❌ **三处换代站点一处都没做** | 修前 **0 处** |
//!
//! 用户可见的症状因此是三件（都只在人真的拿来作曲那份界面上）：
//!
//! 1. **界面一开就显示「播放中」**：第一跳心跳的 `host::apply_transport` 如实投出引擎的
//!    `Playing` —— 镜面没有比引擎多说一句，是引擎自己带着构造默认值跑起来的；
//! 2. **第一次按「播放」是停**：`wire_transport` 的切换读的是镜面 `playing`（那时已经是
//!    `true`）⇒ 走的是「停住」那一支。用户看到的现象是"按了播放，播放键跳回去，什么都没播"；
//! 3. **播放头会自己走**：只要有任何一跳在推量子（设备腿的真实时钟，或控制面的
//!    `drive_audio`），自由跑就每量子推进 tick。
//!
//! ⚠ **为什么此前没有判据抓到它**：三条都在**别的前提**下面通过。
//!
//! * `tests/transport_timecode_tick.rs` / `tests/rewind_sound.rs` / `tests/metronome_switch.rs`
//!   都是判据侧**自己** `host::wire_transport` + 自己 `host.play()` 先把状态摆成"在播"再按 ⇒
//!   从来不读"装配刚建完引擎时它在不在跑"；
//! * `tests/new_project_ui.rs` / `tests/open_project_ui.rs` 断的是 `!window.get_playing()`，
//!   而那是**没有心跳**的装配 ⇒ 镜面还停在 Slint 的默认 `false`（引擎那头其实在跑，从外面读不出来）；
//! * `D-UI-30` 本票把这条当成**显然成立的前提**写了 `assert!(!running)` —— ⭐ 前提本身是
//!   一条需要控制的义务。它一红就把本票的五条判据全打红（CI run `41569883250`：
//!   `d_ui_30_serve_transport_is_wired` = 3 passed / **5 failed**，五条全红在前提读数上）。
//!   那**不是**判据写坏了：判据要的形态是对的一侧，缺的是装配那一步。
//!
//! ⚠ 另一条**不能**用的路：把默认值改进 `EngineHost::reload`。`reload` 里发命令必须**多推
//! 一个量子**才在量子边界生效，那会把 `ui/reload_engine` 报给调用方的 `quanta` 顶到
//! 它要的数字之外（实测红在 `tests/live_ui_mcp.rs` 的
//! `admin_reload_engine_rebuilds_and_resets_the_meter_tap`，那边 `engine_quanta: 4` 必须报 4）。
//! 因此停住必须在 `reload` **之后**，且⛔ 不在 `reload` 里面 —— ⑦ 就是钉这一条的。
//!
//! ## 判据
//!
//! | 判据 | 证的事 | 修前读什么 |
//! | :--- | :--- | :--- |
//! | ①（换代之后是停住的） | 起引擎 ⇒ 本装配宿主 `running = false`、位置 `0`、命令计数**恰好 1**（只有装载那条 `Stop`）、界面镜面 `playing = false` | `running = **true**`、计数 `0` |
//! | ②（第一次按播放是起播） | 在 ① 之后按一次播放 ⇒ `running = **true**`、计数 `+1`、镜面跟真 | 按下之后 `running = **false**`（按播放 = 停） |
//! | ③（没人按 ⇒ 播放头不动） | 停住之后推 `N` 个量子 ⇒ 位置**逐位为 0**、`running` 仍假、计数仍 1 | 位置每量子往前爬（自由跑） |
//! | ④（第二代也一样，且计数器是**每一代自己的**） | 再 `start_engine()` ⇒ 新快照修订号 +1、`running = false`、计数**回到 1**（新引擎 = 新镜面，`Transport::commands_applied` 从 0 起算） | 第二代仍然 `running = true` |
//! | ⑤（换工程那一样本也停住） | `apply_project(另一份工程)` ⇒ `running = false`、位置 `0`、计数 1、镜面 `playing = false` | `running = true` |
//! | ⑥（没有引擎时不许假装停过） | 不起引擎的装配里按播放 ⇒ 计数 `0`、`running` 假、镜面假（`send_transport` 在 `!transport_ready()` 时返回 `TransportReading::cold()`） | 与修后相同（本票⛔ 不改这一格） |
//! | ⑦（修法不许搬进引擎、也不许搬进 `reload`） | 直接用 `EngineHost`：`reload(project, 0)` 之后读数**仍是** `running = true`、计数 `0` ⇒ 停住是**调用方**补的那一步 | 与修后相同（这条是**防线**，不是症状） |
//! | ⑧（出处纪律） | 从源码现读：`live_surface.rs` 里 `.stop()` **恰好 1 处**且落在 `fn rebuild_engine` 的函数体内；`rebuild_engine()` 的调用点 = **3 处**；`main.rs` 的 `.stop()` 仍 **2 处**（两族⛔ 不得互删）；心跳 `engine_tick` 里没有 `.stop()`（⛔ 不每跳替用户按一次） | 0 处 ⇒ 红 |
//!
//! ## 注入（每一条都由本文件的判据打红）
//!
//! * **注入 ①「删掉 `rebuild_engine` 里那条 `stop()`」**（回到修前形状）⇒ ①②③④⑤ 全红
//!   （`running` 真 / 按播放变停 / 播放头自己走），⑧ 红（`.stop()` 1 → 0）；
//! * **注入 ②「把停住搬进 `EngineHost::reload`」**⇒ ⑦ 红（reload 之后 `running` 变假），
//!   而 `live_ui_mcp` 的 `admin_reload_engine_...` 同时红在 `quanta`（那条数字契约）；
//! * **注入 ③「把默认值改成停住」（`rt.rs` 的 `Transport::free_running` → `stopped`）**⇒
//!   ⑦ 红（本票要的**不是**这个：那会同时改掉"没接走带时"的行为，代价见引擎侧文档），
//!   引擎侧既有判据那族也会红；
//! * **注入 ④「三个换代点各抄一遍 `stop()`」**（把 1 处拆成 3 处）⇒ ⑧ 红（`.stop()` 计数
//!   1 → 3）。⭐ 本缺陷的**成因形态**就是"同一步骤在两个装配现场各抄一次，其中一份抄漏" ⇒
//!   这条数把它钉回**单一入口**；
//! * **注入 ⑤「顺手在心跳里也停一次」**⇒ ⑧ 红（`engine_tick` 出现 `.stop()`）＋ ③ 红
//!   （计数不再恰好 1）；
//! * **注入 ⑥「停住之后不投回界面」**（删掉那一句 `apply_transport`）⇒ ① 的镜面那一格
//!   在没有心跳的装配里仍然绿 ⇒ 因此本票把"投回"这件事**交给** ② 与 `D-UI-30` ①
//!   去读（按下之后镜面必须跟真），⛔ 不新造一条只测"没心跳"的假阳性判据。
//!
//! ## 如实登记的边界（⛔ 不假装整条链都通了）
//!
//! 1. **⛔ 不改引擎的构造默认值**：`Playing` 是刻意保留的（见上），本票只在**控制面**补
//!    那条 `Stop`。⑦ 是这一条边界的机械防线。
//! 2. **④ 那个"计数回到 1"是引擎的事实，不是本票的选择**：`EngineRuntime::new` 每代都
//!    `Arc::new(TransportMirror::new())`（全零）⇒ `commands_applied` 是**每一代**自己的数。
//!    因此任何"跨代累加"的读数都是误读，本条把它钉住。
//! 3. **停住 = `Stop`，⛔ 不是 `stop_and_rewind`**：位置本来就在新运行时里是 0，`Stop` 只换
//!    状态；换成两条命令会让"装载"这件看起来无害的事**替用户**发第二条命令（也违反
//!    `D-UI-30` ⑧ 那条"执行面不替用户发命令"的纪律，唯一的例外就是这一条 `Stop`）。
//! 4. **不证明"听得到"**：本文件读的是 RT→控制侧的走带镜面，逐样本那一侧由
//!    `crates/yeban-engine` 的判据负责。
//! 5. **设备腿不在本票范围内**：`open_device` 把新运行时带到**当前**走带位置，本票的
//!    `stop()` 发生在控制驱动形态（`runtime.is_some()` ⇒ `pump(1)`）。代价如实登记：
//!    控制驱动形态下每次换代多应用一条命令 = 多推一个量子，而那条命令是 `Stop` ⇒
//!    **位置一位不走**（③ 就是钉这一句的）。
//! 6. **`mark_applied` 那一格没跟着补**：`main.rs` 建完引擎还记了"当前快照就是这份工程"
//!    （免得第一次心跳白发一份投影），本执行面**没有** ⇒ 第一跳心跳会多发一份同内容的
//!    快照。那不是错误（发布是幂等的、修订号 +1），本票⛔ 顺手改：它会挪动 `D-UI-29`/`D-UI-30`
//!    ⑤ 那两条"心跳发布**恰好一代**"的读数，属于另一件事，已登记进台账 §53 的 needs。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部且每线程只能装配一次；每例一新线程由 libtest
//! 负责 ⇒ 本文件可以有多个 `#[test]`（与 `tests/d_ui_30_serve_transport_is_wired.rs` 同一做法）。
//! ⑥⑦ 不起/不建窗口的那一格（⑦ 只用 `EngineHost`）与 ⑧（纯读源码）因此⛔ 不碰平台。
//!
//! ## 本机不编译重依赖（`AGENTS.md` §5）
//!
//! Slint 是重依赖 ⇒ 本文件在本机**不编译**；编译与判决交 CI。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的装配入口与走带读数 ⇒ 其余项在本测试目标里是
// dead_code（`-D warnings` 会判红）。同款见 `tests/engine_error_banner.rs`。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveUi, LiveWiringOptions, build_live_ui_with};
use slint::ComponentHandle;
use yeban_app::engine_host::EngineHost;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;

/// 够让自由跑走出可见一格的时间（单位：个量子；一个量子 = 128 帧）。
///
/// ⚠ 只用来**证明位置动过 / 没动过**，⛔ 不断言任何具体 tick 数 —— 那会掺进夹具 BPM
/// 的算术。同一条前提（推量子 ⇒ `position_ticks > 0`）在 CI 上已被
/// `tests/transport_timecode_tick.rs` 钉着，这里不新造一种对平台的假设。
const DRIVE_QUANTA: u64 = 64;

/// 装配一台**带权威会话**的活窗口（端口当装配选项传进去 ⇒ `host::wire_transport` /
/// `host::wire_keys` 从第一刻起就握着它，与产品进程同一形态）。
///
/// `start = true` ⇒ 顺手起一代引擎，而本票证的正是"起完是**停住**的"。
///
/// ⛔ 本函数不调 `host::wire_transport`，也⛔ 不调 `EngineHost::stop()`：接线与停住都必须是
/// 装配自己的事（那是本票的命题）。
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

/// 把源码压成**只有代码行**：注释里出现过的字面量不算实现站点
/// （口径实测于 `tests/search_focus_key.rs`，同款见 `D-UI-30` ⑧ / `D-UI-31` ⑧）。
fn code_only(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `fn {header}` 那一项的**函数体**（从签名起，到下一个同级 `fn` 之前）。
///
/// 够用即可：本文件只用它判"某条实现站点落在哪个函数里"，而同级方法在 `impl` 块里都缩进
/// 4 空格 ⇒ 分隔符就是 `"\n    fn "`。它⛔ 不是语法解析（也不假装是）。
fn body_of<'a>(code: &'a str, header: &str) -> &'a str {
    let start = code
        .find(header)
        .unwrap_or_else(|| panic!("⑧ `live_surface.rs` 里必须找得到 `{header}`"));
    let tail = &code[start..];
    &tail[..tail.find("\n    fn ").unwrap_or(tail.len())]
}

/// ① 换代之后本装配读起来必须是**停住**的，而且只为此发过**一条**命令。
#[test]
fn a_freshly_started_engine_is_parked() {
    let (live, _port, window) = assemble("<d-ui-32-park>", true);

    assert!(
        !live.engine_transport_running(),
        "① ⭐ 本票的正主：起引擎之后本装配宿主必须读起来是**停住**（实测在跑）。修前这里是 \
         `true` —— `EngineRuntime` 的走带构造默认 `Playing`（刻意的），而 `live_surface.rs` \
         从来没补那条 `Stop` ⇒ 界面上「没人按却在播」",
    );
    assert_eq!(
        live.engine_transport_commands_applied(),
        1,
        "① 装载停住 = **恰好 1 条**命令（实测 {} 条）—— 0 = 没停住（回到修前形状）；\
         2 以上 = 执行面在替用户按键（`Stop` + `SeekTicks(0)` 那种「顺手回起点」也算）",
        live.engine_transport_commands_applied()
    );
    assert_eq!(
        live.engine_transport_position_ticks(),
        0,
        "① 停住 = `Stop`（**位置保留**，新代的位置本就在 0）；实测 {} tick",
        live.engine_transport_position_ticks()
    );
    assert!(
        !window.get_playing(),
        "① 界面镜面 `playing` 必须跟着引擎读数停在假（停住之后还要把读数投回界面 —— \
         ⛔ 等下一跳心跳）"
    );

    report_line("[d-ui-32] ① 起引擎 ⇒ 停住：running=false、commands_applied=1、位置 0、镜面 false");
}

/// ② 用户第一次按播放得到的是**起播**（修前那次按下是"停"）。
#[test]
fn the_first_press_of_play_starts_instead_of_stopping() {
    let (live, _port, window) = assemble("<d-ui-32-first-press>", true);

    // 前提：停住的（① 证的那一格）⇒ 这一按的语义才是起播。
    assert!(
        !live.engine_transport_running(),
        "② 前提：按下之前必须停住，否则「第一次按播放」这一件事无从判"
    );

    window.invoke_toggle_play();

    assert!(
        live.engine_transport_running(),
        "② ⭐ 第一次按播放必须**在跑**（修前：镜面已经是 `true` ⇒ `wire_transport` 的切换\
         走的是「停住」那一支 ⇒ 用户看到的是「按了播放，什么都没播」）"
    );
    assert_eq!(
        live.engine_transport_commands_applied(),
        2,
        "② 装载那条 `Stop` + 这一条 = 累计 2（实测 {} 条）",
        live.engine_transport_commands_applied()
    );
    assert!(
        window.get_playing(),
        "② 按下之后界面镜面必须跟真（`apply_transport` 的写者只有引擎读数）"
    );

    report_line("[d-ui-32] ② 第一次按播放 ⇒ 起播（running=true，命令累计 2）");
}

/// ③ 没人按任何东西 ⇒ 播放头⛔ 不许自己走（自由跑的形状就是每量子推进）。
#[test]
fn the_playhead_does_not_walk_by_itself() {
    let (mut live, _port, _window) = assemble("<d-ui-32-no-walk>", true);
    let commands_at_start = live.engine_transport_commands_applied();

    // 只有"推量子"这一件事发生：没有按键、没有命令。
    let driven = live.drive_audio(DRIVE_QUANTA);
    assert_eq!(
        driven, DRIVE_QUANTA,
        "③ 前提：这一例确实推了量子（实得 {driven}）—— 没推量子的「零推进」是空转"
    );

    assert_eq!(
        live.engine_transport_position_ticks(),
        0,
        "③ ⭐ 停住之后推 {} 个量子，播放头必须**逐位为 0**（实测 {} tick）—— 非 0 就是\
         「自由跑」：没人按播放，界面上的播放头却自己往前走",
        DRIVE_QUANTA,
        live.engine_transport_position_ticks()
    );
    assert!(
        !live.engine_transport_running(),
        "③ 推量子⛔ 不许把状态推成「在跑」"
    );
    assert_eq!(
        live.engine_transport_commands_applied(),
        commands_at_start,
        "③ 这一跳⛔ 一条命令都不许应用（实测增量 {}）—— 增量非 0 说明有人在心跳里替用户按键",
        live.engine_transport_commands_applied()
            .saturating_sub(commands_at_start)
    );

    report_line("[d-ui-32] ③ 推 64 个量子而没人按键 ⇒ 位置 0、running=false、命令增量 0");
}

/// ④ 第二代也一样停住，而且命令计数器是**每一代自己的**（跨代累加是误读）。
#[test]
fn each_new_generation_is_parked_and_counts_its_own_commands() {
    let (mut live, _port, window) = assemble("<d-ui-32-generation>", true);
    let revision_at_first = live.engine_snapshot_counts().expect("第一代引擎").revision;

    // 第二次换代（`ui/reload_engine` 走的是同一个入口，这里用公开的 `start_engine`）。
    live.start_engine().expect("起第二代引擎");

    assert_eq!(
        live.engine_snapshot_counts().expect("第二代引擎").revision,
        revision_at_first + 1,
        "④ 前提：这一次调用**真的**换了一代（修订号必须 +1，⛔ 不是把同一个数字读两遍）"
    );
    assert!(
        !live.engine_transport_running(),
        "④ ⭐ 新一代⛔ 不许带着构造默认的 `Playing` 交给用户（实测在跑）"
    );
    assert_eq!(
        live.engine_transport_commands_applied(),
        1,
        "④ 计数必须**回到 1**（实测 {} 条）：`EngineRuntime::new` 每代都新建一份全零 \
         `TransportMirror` ⇒ `commands_applied` 是**每一代**自己的数。读到 2 说明读数来自\
         上一代那份镜面（= 换代没换镜面，另一族缺陷）",
        live.engine_transport_commands_applied()
    );
    assert_eq!(
        live.engine_transport_position_ticks(),
        0,
        "④ 新一代的位置在 0（实测 {} tick）",
        live.engine_transport_position_ticks()
    );
    assert!(
        !window.get_playing(),
        "④ 换代之后界面上必须重新显示停住（镜面不许留着上一代的「在播」）"
    );

    report_line("[d-ui-32] ④ 第二代：修订号 +1、running=false、commands_applied 回到 1、位置 0");
}

/// ⑤ 换工程那一处换代站点也停住（与 `main.rs` 打开工程那一段同一形态）。
#[test]
fn a_project_switch_parks_the_new_generation() {
    let (mut live, _port, window) = assemble("<d-ui-32-switch>", true);

    // 先让界面"在播"，再换工程 ⇒ 停住必须是**换代**带来的，⛔ 不是没人按过播放键。
    window.invoke_toggle_play();
    assert!(
        live.engine_transport_running(),
        "⑤ 前提：换工程之前必须在跑（这一格用来证明「停住」是新事件，不是初值）"
    );

    live.apply_project(&yeban_model::samples::demo_project())
        .expect("换工程（同一个活窗口上的重投影 + 换代）");

    assert!(
        !live.engine_transport_running(),
        "⑤ ⭐ 换工程 = 换代 ⇒ 新一代必须停住（实测还在跑：新引擎带着 `Playing` 接上了\
         新工程，界面上的播放键从「播放中」直接继续「播放中」，用户不知道工程已经换了一代）"
    );
    assert_eq!(
        live.engine_transport_position_ticks(),
        0,
        "⑤ 换工程之后播放头在 0（实测 {} tick）",
        live.engine_transport_position_ticks()
    );
    assert_eq!(
        live.engine_transport_commands_applied(),
        1,
        "⑤ 新一代只发过装载那一条 `Stop`（实测 {} 条）",
        live.engine_transport_commands_applied()
    );
    assert!(
        !window.get_playing(),
        "⑤ 界面镜面必须跟着回假（`playing` 的写者只有引擎读数）"
    );

    report_line("[d-ui-32] ⑤ 换工程 ⇒ 停住：running=false、位置 0、命令计数 1、镜面 false");
}

/// ⑥ 没有引擎的那台装配：⛔ 不许假装"我停住过"，读数一路是冷的。
#[test]
fn without_an_engine_the_park_stays_honest_and_silent() {
    let (live, _port, window) = assemble("<d-ui-32-cold>", false);

    assert_eq!(
        live.engine_transport_commands_applied(),
        0,
        "⑥ 没起引擎 ⇒ 一条命令都⛔ 可能被应用（`send_transport` 在 `!transport_ready()` \
         时如实返回 `TransportReading::cold()`）；实测 {}",
        live.engine_transport_commands_applied()
    );
    assert!(
        !live.engine_transport_running(),
        "⑥ 没起引擎 ⇒ 读数必须是停住（冷值），⛔ 不是「没有引擎所以在跑」"
    );

    window.invoke_toggle_play();

    assert_eq!(
        live.engine_transport_commands_applied(),
        0,
        "⑥ 按下之后仍然 0 条（实测 {} 条）—— 本票补的那条 `Stop` 只在**换代**时发，\
         而没引擎就没有换代",
        live.engine_transport_commands_applied()
    );
    assert!(!window.get_playing(), "⑥ 界面镜面⛔ 不得比引擎多说一句");

    report_line("[d-ui-32] ⑥ 无引擎装配：一路冷读数（0 条命令 / 停住 / 镜面 false），按下也不动");
}

/// ⑦ 机械防线：修法留在**控制面**，⛔ 不搬进 `EngineHost::reload`，⛔ 不搬进引擎默认值。
#[test]
fn reload_alone_still_hands_back_a_free_running_transport() {
    let project = yeban_model::samples::filled_project();
    let mut host = EngineHost::new();
    host.reload(&project, 0)
        .expect("无窗口的裸 `EngineHost::reload`（与本装配同一条路径）");

    let reading = host.transport();
    assert!(
        reading.state.is_running(),
        "⑦ ⭐ 裸 `reload` 之后必须**仍是**自由跑（实测已经停住）—— 停住是**调用方**在 \
         `reload` 之后补的那一步。把它搬进 `reload` 会让 `ui/reload_engine` 报的 `quanta` \
         比调用方要的多 1（让命令在量子边界生效必须多推一个量子），实测红在 \
         `tests/live_ui_mcp.rs` 的 `admin_reload_engine_rebuilds_and_resets_the_meter_tap`。\
         把它搬进引擎默认值（`Transport::free_running` → `stopped`）则改掉的是\
         「没接走带时」的行为，代价见 `crates/yeban-engine/src/transport.rs` 的文档"
    );
    assert_eq!(
        reading.commands_applied, 0,
        "⑦ `reload` 自己一条命令都不发（实测 {} 条）",
        reading.commands_applied
    );
    assert_eq!(
        reading.position_ticks, 0,
        "⑦ `engine_quanta = 0` ⇒ 不推量子、位置在 0（实测 {} tick）",
        reading.position_ticks
    );

    // 阳性对照：同一台宿主上补一条 `Stop` ⇒ 读数换档（证明上面那个 `running` 是**状态**，
    // 不是"读不到所以为真"的假象）。
    let parked = host.stop();
    assert!(
        !parked.state.is_running(),
        "⑦ 阳性对照：`EngineHost::stop()` 之后必须停住（实测还在跑 ⇒ 这一位失灵，\
         上面那条 `running` 的读数就无从判）"
    );
    assert_eq!(
        parked.commands_applied, 1,
        "⑦ 阳性对照：这一条命令被本代应用了（实测 {} 条）",
        parked.commands_applied
    );

    report_line(
        "[d-ui-32] ⑦ 裸 reload 仍自由跑（running=true、0 条）⇒ 停住是控制面的义务；补一条 Stop 立刻停住",
    );
}

/// ⑧ 出处纪律（⛔ 不开窗口）：停住落在**唯一的换代入口**里，且两族都在。
#[test]
fn the_park_lives_in_the_single_generation_entry_point() {
    let live = code_only(include_str!("../src/live_surface.rs"));
    let main = code_only(include_str!("../src/main.rs"));

    // 唯一的例外必须**在**：执行面替用户发的命令 = 0 处（`D-UI-30` ⑧ 那条纪律的正面）。
    for token in ["send_transport(", ".play()", "stop_and_rewind(", ".seek("] {
        assert_eq!(
            live.matches(token).count(),
            0,
            "⑧ `live_surface.rs` 代码里 `{token}` 必须 0 处（实测 {} 处）",
            live.matches(token).count()
        );
    }
    assert_eq!(
        live.matches(".stop()").count(),
        1,
        "⑧ `live_surface.rs` 里 `.stop()` 必须**恰好 1 处**（实测 {} 处）—— 0 = 本票修的\
         缺陷（新一代带着 `Playing` 交给用户）；≥2 = 停住又被抄成多处事实源，那正是本缺陷\
         的成因形态（`main.rs` 有、`live_surface.rs` 抄漏）",
        live.matches(".stop()").count()
    );

    // 那一处必须落在**换代入口** `fn rebuild_engine` 的函数体里（⛔ 不在心跳、⛔ 不在某个调用点）。
    let entry_body = body_of(&live, "fn rebuild_engine");
    assert_eq!(
        entry_body.matches(".stop()").count(),
        1,
        "⑧ 那一条 `.stop()` 必须落在 `fn rebuild_engine` **体内**（实测 {} 处）",
        entry_body.matches(".stop()").count()
    );
    // 三个换代站点必须都走这个入口（装配起引擎 / 换工程 / `ui/reload_engine`）。
    assert_eq!(
        live.matches("rebuild_engine()").count(),
        3,
        "⑧ `rebuild_engine()` 的调用点必须 = **3 处**（`start_engine` + `apply_project` + \
         `reload_engine_now`；实测 {} 处）—— 少一处意味着某个换代站点又自己抄（或漏）了\
         停住那一步，本票的红就是那样来的",
        live.matches("rebuild_engine()").count()
    );
    // 心跳⛔ 不许替用户按停（③ 的运行期那一格由计数钉，这里钉形态）。
    let tick_body = body_of(&live, "fn engine_tick");
    assert_eq!(
        tick_body.matches(".stop()").count(),
        0,
        "⑧ 心跳 `engine_tick` 里 `.stop()` 必须 0 处（实测 {} 处）—— 每跳停一次 = 用户\
         按了播放却永远起不来",
        tick_body.matches(".stop()").count()
    );
    // 生产那条腿的停住必须**还在**（两族⛔ 不得互删）。
    assert_eq!(
        main.matches(".stop()").count(),
        2,
        "⑧ `main.rs` 里 `.stop()` 必须仍是 **2 处**（实测 {} 处）：装载即停住 + ALSA 设备\
         挂载关闭（`mount.stop()`，与走带无关）。掉到 1 ⇒ 「打开即播放」在生产界面上回来了，\
         而本判据的 serve 那一半仍然绿 ⇒ 只有这条能看见",
        main.matches(".stop()").count()
    );
    // 引擎侧的构造默认⛔ 不许被本票改掉（⑦ 的行为版本 + 这条的出处版本，两道）。
    let engine_rt = code_only(include_str!("../../yeban-engine/src/rt.rs"));
    assert_eq!(
        engine_rt.matches("Transport::free_running(").count(),
        1,
        "⑧ `rt.rs` 里 `Transport::free_running(` 必须仍是 **1 处**（实测 {} 处）—— \
         本票的修法**只能**在控制面补一条 `Stop`；改引擎默认值会连带改掉「没接走带时」\
         的行为（引擎侧文档写死了这一条）",
        engine_rt.matches("Transport::free_running(").count()
    );

    report_line(
        "[d-ui-32] ⑧ 源码纪律：live `.stop()`=1（在 rebuild_engine 体内）/ rebuild_engine() \
         调用点=3 / engine_tick 内 0 / main `.stop()`=2 / rt `free_running`=1",
    );
}
