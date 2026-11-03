//! **节拍器开关真的写进文档** —— `D-UI-29` 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! `Cmd/Ctrl+Alt+N`（与走带区那颗键**同一个**落点）按下去之后，
//! `transport.metronome_enabled` 必须**在权威文档里**改掉；播放侧由**既有的** 60 Hz
//! 发布腿按它换快照；界面上那颗键的点亮态（`metronome-on`）最终**就是**那一代快照的值。
//!
//! 修前的形状（`docs/ledger/app-zimmer-ui-notes.md` §50 登记）：那条臂落在
//! `apply_engine_command` → `metronome::set_enabled`，而它的 `project` 是**装配期冻结**的
//! `Rc<YebanProjectV1>`（`main.rs` 只在换工程时重装配）⇒ 拨一次开关 = 拿那份**旧文档**
//! 重放一遍 `reload`：
//!
//! 1. ⛔ 文档一位没改 ⇒ 保存 / 重开就弹回装配期初值；
//! 2. ⛔ 撤销 / 重做管不到它（没有提交）；
//! 3. ⛔ 用户此后画的音符、改的速度在**播放侧**全部消失（旧文档赢了），而界面上还在；
//! 4. ⛔ 走带被 `stop` → `reload` → `seek` 补回来 —— 控制驱动形态下必然漂几个 tick
//!    （`src/metronome.rs` 的模块文档逐字登记 `MAX_RESTORE_DRIFT_TICKS = 16`）；
//! 5. ⛔ 镜面与快照只在拨动那一刻一起动 ⇒ 撤销 / 运行期「打开工程」/ 控制面（MCP）写文档
//!    之后，**界面上点亮的**与**耳朵里响的**分叉。
//!
//! ## 为什么修法是「只提交 op」
//!
//! 「编辑 ⇒ 发声」那条腿**早就存在**：`main.rs` 的心跳在 `EditMark`（活跃分支头 ＋ 已撤销
//! 步数）一变时就 `UndoPort::try_project()` → `ProductionLoop::tick` →
//! `EngineHost::publish_project` → `EngineSnapshot::from_project`（节拍器计划正是在**投影期**
//! 进快照的那一格，见 `crates/yeban-engine/src/snapshot.rs` 的 `with_metronome`）。这条腿
//! **不重建引擎、不推量子、不动走带** ⇒ 一次 op 同时拿到 ⭐ 发声（下一跳 ≤ 16 ms）、
//! ⭐ 无损走带、⭐ 可撤销、⭐ 会持久化。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（前提读数） | 夹具文档 `metronome_enabled = false`、镜面 `false`、起引擎后快照 `false` ⇒ 三个读数在装配时**已经对齐** |
//! | ②（第一下真的改文档，且⛔ 不碰引擎） | 一次 `Cmd/Ctrl+Alt+N` ⇒ 被消费 ＋ 文档 `false → true` ＋ 恰好 `+1` 条提交 ＋ 镜面立刻跟；⭐ 同一时刻快照**仍是** `false` 且快照**修订号**一位没涨 ⇒ 拨动压根没动引擎（修前这两条都红） |
//! | ③（发声只经发布腿） | 下一跳心跳 ⇒ `published_revision = 修订号 + 1`、快照变 `true`、镜面仍 `true`（三个读数重新对齐） |
//! | ④（这是**开关**，不是单向置位） | 第二下 ⇒ 文档 `false`、镜面 `false`；再来一跳 ⇒ 快照 `false` |
//! | ⑤（兄弟字段逐字不动） | `Op::SetTransportConfig` 是**整体替换**三个字段 ⇒ `count_in_bars` / `launch_quantization` 在两次拨动前后**逐字段相同**；反向对照：预备拍那一臂⛔ 动节拍器那一格 |
//! | ⑥（不发布的那一跳⛔ 不写镜面） | 标记没变 ⇒ `published_revision = None` ＋ `publish_error = None`，被人为拨歪的镜面**一位不动**（不猜、不「照着文档重算」） |
//! | ⑦（撤销：一跳之内对齐） | `Cmd+Z` ⇒ 文档回 `false`、镜面**这一瞬间**仍是旧的 `true`（`apply_view` 压根不写这一格：`bridge.rs` 里 `set_metronome_on` 0 处），下一跳发布 ⇒ 镜面 / 快照 / 文档三者同为 `false` ⇒ 陈旧窗口 = **恰好一跳**（≤ 16 ms） |
//! | ⑧（两个入口、一个落点） | 键与走带按钮的回调都落在 `apply_action` 的**同一条臂**（从源码文本钉）⇒ 不会算出两个「下一个值」 |
//! | ⑨（无会话装配的诚实形状） | `undo = None` 的 `live_surface` 装配（`wire_keys(.., engine: None, project: None)`）里同一键返回 `false`；对照臂钉住「这台装配的键路是活的」 |
//! | ⑩（出处纪律） | 从源码现读（先剥 `//` 开头的行 ⇒ **注释不是实现站点**）：`Op::SetTransportConfig` 的构造点仍**恰好 1 处**、`commit_transport_config(` = 定义＋2 调用、`metronome::set_enabled(` 在宿主代码里只剩 **1 处**（无会话那一支）、镜面校正点生产与执行面**各调一次同一个函数**、`bridge.rs` 那侧 `set_metronome_on` **0 处**、两个入口都在「端口在场」时分岔（⛔ 不借宿主） |
//!
//! ## 注入（每一条都由本文件的判据打红）
//!
//! * **注入 ①「拨动仍然换引擎」**（把 `apply_action` 的节拍器臂改回只走
//!   `apply_engine_command`）⇒ ② 红（文档没改、快照修订号涨了）；
//! * **注入 ②「镜面在 `apply_view` 里重算」**（在 `bridge.rs` 加一句
//!   `set_metronome_on(project.transport.metronome_enabled)`）⇒ ⑥ 红（不发布的那一跳
//!   也擦了人为拨歪的镜面），并被 `tests/perf_draw_budget.rs` 的属性写入点预算判红；
//! * **注入 ③「撤销那一支不重投影」**⇒ ⑦ 红（时光机读数不跟）；
//! * **注入 ④「镜面校正点从执行面删掉」**（删 `live_surface::engine_tick` 那一句调用）
//!   ⇒ ⑩ 红（live 侧 1 处变 0 处）。⚠ 这条注入⛔ 打不红 ⑦ —— ⑦ 走的是执行面这一条腿，
//!   它靠的是「`apply_metronome_mirror` 在**这一跳**被调到」这一事实，而 ⑦ 的断言读的是
//!   镜面值：删掉调用后撤销那一跳镜面停在 `true` ⇒ ⑦ **同样红**（两条腿都钉着）。
//!
//! ## 如实登记的边界（⛔ 不假装整条链都通了）
//!
//! 1. **本装配没有走带接线**：`live_surface` 不调 `host::wire_transport`（那份
//!    `Rc<RefCell<EngineHost>>` 在 `LiveSurface` 里、没有以可共享的形态交出去）⇒
//!    无头执行面上**那颗按钮**（以及播放 / 停止 / 录音）本来就没有注册者。因此 ⑧ 只能
//!    从**源码文本**证「两个入口同一个落点」；按按钮那一格只在生产 `run_gui` 里可达。
//!    该缺口与 `D-UI-11` 同族，已登记台账（⛔ 本票不扩范围去补整条走带接线）。
//! 2. **不证明「听得到咔哒声」**：本文件判的是「快照里有没有节拍器计划」
//!    （`EngineSnapshot::metronome_enabled` = 投影自那个字段）。逐样本那一侧由
//!    `crates/yeban-engine/tests/rt_zero_alloc.rs` 的 `armed_metronome_enabled` 判据负责。
//! 3. **陈旧窗口 = 一跳**：拨动之后镜面立刻跟（不等心跳，判据装配里不跑心跳）；
//!    反方向（文档被**不经拨动**的路改掉：撤销 / 换工程 / MCP）镜面要等**下一跳发布**。
//!    这个 ≤ 16 ms 的窗口是设计取舍，不是缺陷 —— 要⛔ 不留窗口就得让 `apply_view` 写
//!    这一格，而那正是 ⑥ 与注入 ② 钉住不许做的事。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部且每线程只能装配一次；每例一新线程由 libtest
//! 负责 ⇒ 本文件可以有多个 `#[test]`（与 `tests/clip_mute_toggle.rs` 同一做法）。
//! ⑧ 与 ⑩ 不建窗口（纯读源码），因此⛔ 不碰平台。
//!
//! ## 本机不编译重依赖（`AGENTS.md` §5）
//!
//! Slint 是重依赖 ⇒ 本文件在本机**不编译**；编译与判决交 CI。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的装配入口与心跳 ⇒ 其余项在本测试目标里是 dead_code
// （`-D warnings` 会判红）。同款见 `tests/engine_error_banner.rs`。
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

/// 装配一台**带权威会话**的活窗口（端口当装配选项传进去 ⇒ `host::wire_keys` 从第一刻起
/// 就握着它，与产品进程 `main.rs` 同一形态），并**起一代引擎**。
///
/// ⚠ `engine_quanta: 0`：起引擎时⛔ 不推量子 ⇒ 本文件每一条读数都只可能来自「装配」或
/// 「心跳」，不会掺进量子推进的副作用。
fn assemble_with_session(tag: &str) -> (LiveUi, Rc<UndoPort>, MainWindow) {
    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open(tag, "yeban-app", project.clone(), NOW_MS).expect("打开会话"),
    ));
    let mut live = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            engine_quanta: 0,
            undo: Some(Rc::clone(&port)),
            ..LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 执行面");
    live.start_engine().expect("起一代引擎");
    let window = live.ui().clone_strong();
    (live, port, window)
}

/// 权威文档的一份快照（读不到 ⇒ 直接判据红：那是前提不成立，⛔ 不是「改动没生效」）。
fn live_project(port: &UndoPort) -> yeban_model::YebanProjectV1 {
    port.try_project()
        .expect("端口必须有一份权威工程（没有 ⇒ 本文件的每一条都无从判起）")
}

/// 文档里的节拍器开关（单位：**布尔** —— 权威那一份，⛔ 不是界面属性）。
fn doc_enabled(port: &UndoPort) -> bool {
    live_project(port).transport.metronome_enabled
}

/// **播放侧那一代快照**里的节拍器开关（单位：**布尔**；`None` = 没有引擎）。
fn snapshot_enabled(live: &LiveUi) -> Option<bool> {
    live.engine_metronome_enabled()
}

/// 快照的版本号（单位：**修订号**，每次换代 +1）—— 「拨动到底有没有动引擎」的读数。
fn revision(live: &LiveUi) -> u64 {
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

/// 注入一次 `Cmd/Ctrl+Alt+N`（`text, shift, control, alt, meta` 与 `key-action` 逐位对齐，
/// 与 `tests/d_ui_23_count_in_writes_the_document.rs` 的 `+D` 同款拼法）。
fn press_metronome(ui: &LiveUi) -> bool {
    ui.ui()
        .invoke_key_action("n".into(), false, true, true, false)
}

/// 注入一次 `Cmd/Ctrl+Alt+D`（预备拍 —— ⑤ 的反向对照）。
fn press_count_in(ui: &LiveUi) -> bool {
    ui.ui()
        .invoke_key_action("d".into(), false, true, true, false)
}

/// 注入一次 `Cmd+Z`（撤销）。
fn press_undo(ui: &LiveUi) -> bool {
    ui.ui()
        .invoke_key_action("z".into(), false, true, false, false)
}

/// 把源码压成**只有代码行**：注释里出现过的字面量不算实现站点
/// （口径实测于 `tests/search_focus_key.rs`，同款见 `D-UI-23` ⑨）。
fn code_only(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_metronome_press_writes_the_document_and_leaves_the_engine_alone() {
    let (mut live, port, window) = assemble_with_session("<d-ui-29-document>");

    // ---- ① 前提读数：三个读数在装配时**已经**对齐（都是 `false`）----
    assert!(
        !doc_enabled(&port),
        "① 夹具前提：`filled_project()` 的文档字段必须是 `false`（读到 {}）",
        doc_enabled(&port)
    );
    assert_eq!(
        window.get_metronome_on(),
        doc_enabled(&port),
        "① 前提：镜面必须与文档在装配时对（⛔ 让界面当事实源就是修前的形状）"
    );
    assert_eq!(
        snapshot_enabled(&live),
        Some(false),
        "① 前提：起引擎后快照里的节拍器必须是 `false`（`engine_quanta = 0` 不影响投影）"
    );
    let commits_at_start = port.display().commit_count;
    let count_in_at_start = live_project(&port).transport.count_in_bars;
    let quantization_at_start = live_project(&port).transport.launch_quantization;
    let revision_at_start = revision(&live);

    // ---- ② 第一下：文档被改，而引擎**一位没动** ----
    assert!(
        press_metronome(&live),
        "② `Cmd/Ctrl+Alt+N` 必须被消费（返回 `false` ⇒ `.slint` 把它放行给焦点系统，\
         也就是「这一键按了什么都没发生」）"
    );
    assert!(
        doc_enabled(&port),
        "② 第一下必须把**文档**里的节拍器拨成 `true`（修前文档停在 `false`：那条臂压根不提交）"
    );
    assert_eq!(
        port.display().commit_count,
        commits_at_start + 1,
        "② 一次按键必须留下**恰好一条**提交（单位：条）"
    );
    assert!(
        window.get_metronome_on(),
        "② 镜面必须**立刻**跟着走（那颗键的 `accessible-checked` 读的就是它，\
         而判据装配里不跑心跳）"
    );
    // ⭐⭐ 本票的核心断言：拨动**不碰引擎**。修前这里两条都红 —— 那条臂 `reload` 换一代
    // ⇒ 快照当场变 `true`、修订号立刻 +1、走带被 stop 再 seek 回来。
    assert_eq!(
        snapshot_enabled(&live),
        Some(false),
        "② 快照必须**还是**旧的那一代（`false`）：发声交给发布腿，⛔ 不在按键里换引擎。\
         若这里变成 `true`，就是那条臂又去 `reload` 了"
    );
    assert_eq!(
        revision(&live),
        revision_at_start,
        "② 快照**修订号**必须一位没涨（单位：修订号）—— 这是「拨动没有换代引擎」的直接读数"
    );

    // ---- ③ 下一跳心跳：文档 ⇒ 快照 ⇒ 镜面重新对齐 ----
    let hop = heartbeat(&mut live, &port);
    report_line(&format!(
        "[d-ui-29] 第一跳: published={:?} error={:?}",
        hop.published_revision, hop.publish_error
    ));
    assert_eq!(
        hop.published_revision,
        Some(revision_at_start + 1),
        "③ 这一跳必须**真的**换了一代快照（修订号 {} → {}）—— 这就是「编辑 ⇒ 发声」那条腿",
        revision_at_start,
        revision_at_start + 1
    );
    assert_eq!(
        snapshot_enabled(&live),
        Some(true),
        "③ 快照必须按**文档**投影成 `true`"
    );
    assert!(
        window.get_metronome_on(),
        "③ 镜面仍为 `true`（与快照、文档三者对齐）"
    );

    // ---- ④ 开关的两极性（⛔ 不是单向置位）----
    assert!(press_metronome(&live), "④ 第二下必须同样被消费");
    assert!(
        !doc_enabled(&port),
        "④ 第二下必须把文档拨回 `false`（修前做不到：那条臂算「下一个值」用的是装配期\
         冻结的快照 ⇒ 每次都算出同一个值）"
    );
    assert!(!window.get_metronome_on(), "④ 镜面必须跟着回 `false`");
    let second = heartbeat(&mut live, &port);
    assert!(
        second.published_revision.is_some(),
        "④ 第二下之后同样要有一跳发布（`EditMark` 又动了）"
    );
    assert_eq!(
        snapshot_enabled(&live),
        Some(false),
        "④ 快照必须又按文档变回 `false`（三个读数一起回）"
    );

    // ---- ⑤ 兄弟字段逐字不动（整体替换那条 `Op` 的代价就在这里）----
    let after = live_project(&port).transport;
    assert_eq!(
        after.count_in_bars, count_in_at_start,
        "⑤ `count_in_bars` 必须逐字带着走（单位：**小节**；⛔ 顺手把预备拍清零）"
    );
    assert_eq!(
        after.launch_quantization, quantization_at_start,
        "⑤ `launch_quantization` 必须逐字带着走（本票⛔ 不碰场景量化）"
    );

    // ⑤ 的反向对照：预备拍那一臂反过来⛔ 动节拍器那一格（与 `D-UI-23` ④ 对称）。
    assert!(
        press_count_in(&live),
        "⑤ 对照：`Cmd/Ctrl+Alt+D` 在同一装配里必须被消费"
    );
    assert_eq!(
        live_project(&port).transport.count_in_bars,
        0,
        "⑤ 对照前提：预备拍那一臂确实改了**文档**（{} 小节 → 0），否则这条对照什么都没测",
        count_in_at_start
    );
    assert!(
        !doc_enabled(&port),
        "⑤ 节拍器的文档字段⛔ 被预备拍那一臂顺手动（必须仍是 `false`）"
    );
    assert!(
        !window.get_metronome_on(),
        "⑤ 镜面也⛔ 被那一臂顺手动（两个落点各写镜面就是第二个事实源）"
    );
    report_line(
        "[d-ui-29] 文档 false→true→false（节拍器）、提交 +2 条、快照只在心跳那一跳换代；\
         兄弟字段一位不动",
    );
}

#[test]
fn the_mirror_follows_the_published_snapshot_and_no_publish_writes_nothing() {
    let (mut live, port, window) = assemble_with_session("<d-ui-29-mirror>");

    // 先把一次真的编辑发上去（文档 `true`、快照 `true`、镜面 `true`）。
    assert!(press_metronome(&live), "夹具前提：第一下必须被消费");
    heartbeat(&mut live, &port);
    assert_eq!(
        snapshot_enabled(&live),
        Some(true),
        "夹具前提：快照必须已经是 `true`"
    );
    assert!(window.get_metronome_on(), "夹具前提：镜面与快照同为 `true`");

    // ---- ⑥ 不发布的那一跳⛔ 不写镜面 ----
    // 人为把镜面拨歪（读出「这一跳到底写没写」—— 不注入就看不出来）。
    window.set_metronome_on(false);
    let stale = heartbeat(&mut live, &port);
    report_line(&format!(
        "[d-ui-29] 不发布的一跳: published={:?} error={:?} 镜面={}",
        stale.published_revision,
        stale.publish_error,
        window.get_metronome_on()
    ));
    assert_eq!(
        stale.published_revision, None,
        "⑥ 夹具前提：标记没变 ⇒ 这一跳不发布（否则测的不是「不发布」那一档）"
    );
    assert_eq!(stale.publish_error, None, "⑥ 夹具前提：没发布也谈不上失败");
    assert!(
        !window.get_metronome_on(),
        "⑥ ⭐ 这一跳**没有**换上按新文档投影的快照 ⇒ 镜面一位不动（`None` 那一档不写）。\
         若这里变成 `true`，就是校正点被改成了「每跳照文档重算」—— 那会在发布失败的\
         那一跳点亮一件没有发生的事"
    );

    // 把 ⑥ 注入的脏读数恢复成「与当前快照一致」，再测撤销那一档（否则两段互相踩状态）。
    window.set_metronome_on(true);

    // ---- ⑦ 撤销：文档先动，镜面在**下一跳发布**时对齐（陈旧窗口 = 恰好一跳）----
    assert!(
        press_undo(&live),
        "⑦ `Cmd+Z` 必须被消费（撤的正是那一笔节拍器提交）"
    );
    assert!(
        !doc_enabled(&port),
        "⑦ 撤销必须把**文档**拨回 `false`（这就是「撤销一步就关掉节拍器」）"
    );
    assert!(
        window.get_metronome_on(),
        "⑦ 撤销那一次只重投影（`apply_view` 压根不写这一格 ⇒ `bridge.rs` 里 \
         `set_metronome_on` 0 处）⇒ 镜面**此刻仍是**上一跳的 `true`。\
         这是 ≤ 16 ms 的设计窗口，下一跳发布就对齐（见文件头的边界 3）"
    );
    let hop = heartbeat(&mut live, &port);
    assert!(
        hop.published_revision.is_some(),
        "⑦ 撤销动了 `EditMark` 的 `undone` ⇒ 这一跳必须发布新快照（否则「撤销不影响播放」\
         这句就真的成立了）"
    );
    assert_eq!(
        snapshot_enabled(&live),
        Some(false),
        "⑦ 快照必须按撤销后的文档变回 `false`"
    );
    assert!(
        !window.get_metronome_on(),
        "⑦ ⭐ 镜面必须由**快照**校正回来（`apply_metronome_mirror`）⇒ 界面与耳朵重新对得上"
    );
    report_line("[d-ui-29] 不发布那一跳⛔ 写镜面；撤销后恰好一跳对齐（文档 / 快照 / 镜面）");
}

/// ⑧ 两个入口、一个落点：键与走带按钮的回调都交给 `apply_action` 的**同一条臂**。
///
/// ⚠ 本装配（`live_surface`）⛔ 不调 `wire_transport` ⇒ 按钮的 `on_toggle_metronome`
/// 在这里**没有注册者**（文件头的边界 1 如实登记），因此这一条只能从**源码文本**判。
#[test]
fn the_button_and_the_key_share_one_landing_point_in_source() {
    let host = code_only(include_str!("../src/host.rs"));
    assert_eq!(
        host.matches("apply_action(&ui, metronome_undo.as_ref(), None, Action::ToggleMetronome)")
            .count(),
        1,
        "⑧ `wire_transport` 的按钮回调（有会话那一支）必须把动作交给 `apply_action` 的\
         节拍器臂（1 处）"
    );
    assert_eq!(
        host.matches("apply_action(&ui, None, Some(&mut context), Action::ToggleMetronome)")
            .count(),
        1,
        "⑧ 按钮的**无会话**那一支也必须走 `apply_action`（1 处）—— ⛔ 不在那里再算一遍\
         「下一个值」"
    );
    report_line("[d-ui-29] 两个入口各 1 处调用、都落在 apply_action 的同一条臂");
}

#[test]
fn a_metronome_press_without_a_session_in_the_live_assembly_is_not_consumed() {
    let project = yeban_model::samples::filled_project();
    let live = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            undo: None,
            ..LiveWiringOptions::default()
        },
    )
    .expect("真实界面（无撤销会话）+ Tier-1 执行面");

    // ⭐ 对照臂：先把「这台装配的键路是活的」钉住 —— 一个**不需要**端口的动作必须照常
    // 消费（否则下面的 `false` 可能只是「这一键整体没接」，什么也不证明）。
    assert!(
        live.ui()
            .invoke_key_action("3".into(), false, false, false, false),
        "⑨ 对照臂：无端口装配里数字键必须被消费（`Action::SelectTool` 不依赖端口）"
    );
    assert_eq!(
        live.ui().get_active_tool(),
        3,
        "⑨ 对照臂：数字键必须真的换了工具（否则「不消费」这句话无从判起）"
    );

    assert!(
        !press_metronome(&live),
        "⑨ 本装配（`wire_keys(.., engine: None, project: None)`）既没有端口、也没有引擎\
         上下文 ⇒ 节拍器这一键⛔ 不得消费（既无文档可写、也无引擎可换，就只有「放行给\
         焦点系统」这一条诚实答案；⛔ 不假装成功）"
    );
    assert_eq!(
        live.ui().get_active_tool(),
        3,
        "⑨ 不消费必须是**真的什么都没做**：工具读数一位不动"
    );
    report_line("[d-ui-29] 无会话装配：同一键返回 `false`（对照臂：数字键照常消费）");
}

/// ⑩ 出处纪律（⛔ 不开窗口）：文档提交只有一个落点、镜面校正只挂在发布那两跳、
/// 换代那条路只剩「无会话」那一支。
#[test]
fn the_document_is_the_only_metronome_landing_point_with_a_session() {
    let host = code_only(include_str!("../src/host.rs"));
    let live = code_only(include_str!("../src/live_surface.rs"));
    let bridge = code_only(include_str!("../src/bridge.rs"));

    // 构造 `Op` 的地方只许一处（两个落点各算各的「下一个值」必分叉 —— `D-UI-23` ⑨ 同口径）。
    assert_eq!(
        host.matches("Op::SetTransportConfig {").count(),
        1,
        "⑩ `host.rs` 代码里 `Op::SetTransportConfig` 的构造点必须**恰好 1 处**（单位：处）"
    );
    // 提交落点：定义 + 节拍器调用 + 预备拍调用。
    assert_eq!(
        host.matches("commit_transport_config(").count(),
        3,
        "⑩ `commit_transport_config(` 必须是**定义＋恰好 2 处调用**（实测 {} 处）—— \
         两个走带开关共用同一个提交落点",
        host.matches("commit_transport_config(").count()
    );
    // 换代那条路只剩「没有可提交的对象」那一支。
    assert_eq!(
        host.matches("metronome::set_enabled(").count(),
        1,
        "⑩ 宿主代码里 `metronome::set_enabled(` 必须只剩 **1 处**（`apply_engine_command`，\
         无会话那一支）—— 有会话时拨动⛔ 不换引擎"
    );
    // 借宿主那一族仍然只列节拍器（与 `only_engine_commands_take_the_engine_borrow` 同口径）。
    assert_eq!(
        host.matches("matches!(action, Action::ToggleMetronome)")
            .count(),
        1,
        "⑩ `action_needs_engine` 的 `matches!` 必须只列节拍器**一条**"
    );
    // 镜面校正点：定义 + 生产心跳那一跳（两条腿调的是**同一个**函数）。
    assert_eq!(
        host.matches("apply_metronome_mirror").count(),
        2,
        "⑩ `host.rs` 里 `apply_metronome_mirror` 必须是**定义＋生产心跳 1 处调用**\
         （实测 {} 处）",
        host.matches("apply_metronome_mirror").count()
    );
    assert_eq!(
        live.matches("apply_metronome_mirror").count(),
        1,
        "⑩ 执行面（`live_surface.rs`）必须调**同一个**校正点**恰好 1 次**（实测 {} 处）—— \
         两条腿的界面读数因此不可能漂移",
        live.matches("apply_metronome_mirror").count()
    );
    // 两个入口都在「端口在场」时分岔（⛔ 不借宿主 —— 那是 `#212` 的 abort 形态）。
    assert_eq!(
        host.matches("if metronome_undo.is_some()").count(),
        1,
        "⑩ 按钮那一支必须**先问端口**再决定借不借宿主（1 处）"
    );
    assert_eq!(
        host.matches("if undo.is_some()").count(),
        1,
        "⑩ 键那一支必须在借宿主**之前**就分岔（1 处）—— 带着 `RefMut` 进文档臂就是\
         `#212` 那次 abort 的形状"
    );
    // 镜面不是重投影的一部分（`apply_view` 不写它 ⇒ ⑦ 的那个一跳窗口由此而来）。
    assert_eq!(
        bridge.matches("set_metronome_on").count(),
        0,
        "⑩ `bridge.rs`（`apply_view` 那一族）里 `set_metronome_on` 必须 **0 处**：镜面由\
         发布那一跳校正，⛔ 不由属性重算（属性写入点预算由 `tests/perf_draw_budget.rs` 钉着）"
    );
    assert_eq!(
        host.matches("set_metronome_on").count(),
        4,
        "⑩ `host.rs` 里 `set_metronome_on` 必须**恰好 4 处**：装配初值 / 有会话拨动 /\
         无会话拨动 / 每跳校正（实测 {} 处）",
        host.matches("set_metronome_on").count()
    );

    report_line(
        "[d-ui-29] 源码纪律：Op 构造点 1 处、共用提交落点 3 处（定义＋2 调用）、\
         set_enabled 1 处、镜面校正点 host 2 / live 1、bridge 写入 0 处",
    );
}
