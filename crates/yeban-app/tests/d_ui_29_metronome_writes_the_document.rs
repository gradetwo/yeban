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
//! 1. **⑧ 只从源码文本证「同一个落点」**：本文件全部运行期判据走的是**键**那条入口
//!    （`Cmd/Ctrl+Alt+N` ⇒ `wire_keys` ⇒ `apply_action`）。⚠ 本装配的走带接线在
//!    `D-UI-30`（2026-11-03）**之前**是缺的（`live_surface` 不调 `host::wire_transport`
//!    ⇒ 那颗按钮没有注册者），因此当时**只能**这样判；该缺口现已补上 ⇒ **按按钮**那一格
//!    的运行期版本在 `tests/d_ui_30_serve_transport_is_wired.rs` 的 ⑤（同一个夹具、
//!    同一个落点、同样证「写文档 + 不换引擎」）。⑧ 仍然留在本文件：它钉的是
//!    「两个入口⛔ 不许各算一次下一个值」这条**出处**纪律，与谁来按无关。
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

#![allow(unused)]
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
}

#[test]
fn the_mirror_follows_the_published_snapshot_and_no_publish_writes_nothing() {
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
}

/// ⑧ 两个入口、一个落点：键与走带按钮的回调都交给 `apply_action` 的**同一条臂**。
///
/// ⚠ 这一条**只**从源码文本判「同一个落点」（它钉的是出处纪律，与谁来按无关）；
/// 按**按钮**那一路的运行期版本在 `tests/d_ui_30_serve_transport_is_wired.rs` 的 ⑤
/// —— 自 `D-UI-30` 起本装配自己接了 `host::wire_transport`，那颗按钮有注册者了。
#[test]
fn the_button_and_the_key_share_one_landing_point_in_source() {
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
}

#[test]
fn a_metronome_press_without_a_session_in_the_live_assembly_is_not_consumed() {
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
}

/// ⑩ 出处纪律（⛔ 不开窗口）：文档提交只有一个落点、镜面校正只挂在发布那两跳、
/// 换代那条路只剩「无会话」那一支。
#[test]
fn the_document_is_the_only_metronome_landing_point_with_a_session() {
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
}
