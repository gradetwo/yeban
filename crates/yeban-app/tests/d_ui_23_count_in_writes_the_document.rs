//! **预备拍开关真的写进文档** —— `D-UI-23` 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! `Cmd/Ctrl+Alt+D` 按下去之后，`transport.count_in_bars` 必须**在权威文档里**改掉，
//! 并且能被**一步撤销**放回去；录音那条腿读的是这份**活文档**，⛔ 不是装配期那份快照。
//!
//! 修前的形状（`docs/ledger/app-zimmer-ui-notes.md` §38 登记）：那条臂落在
//! `apply_engine_command` 里，「下一个值」从 `context.project.transport.count_in_bars` 算，
//! 而那个 `project` 是**装配期冻结**的 `Rc<YebanProjectV1>`（`main.rs` 只在**换工程**时
//! 重装配）⇒ ① 提交⛔ 不发生（文档一位没改）⇒ 第二下还算出同一个值 ⇒ **关不掉**；
//! ② 唯一的读者 `record_wav::count_in_frames` 读的也是那份冻结快照 ⇒ 换的哪一代都一样；
//! ③ 换代要把走带**停住再恢复** —— 代价付了，而实时引擎根本不读这个字段
//! （`crates/yeban-engine/src/lib.rs` 逐字把「预备拍（`transport.count_in_bars`）」列在
//! **仍未实现**那一栏）。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（前提读数） | 夹具的 `count_in_bars = 2`（**小节**）、`bpm = 128.0`、`numerator = 4` ⇒ 预备拍 = 180 000（**帧**，`4 拍 × 60 ÷ 128 × 48 000`）。②–⑧ 全靠这几个数有判别力 |
//! | ②（第一下真的改文档） | 一次 `Cmd/Ctrl+Alt+D` ⇒ 被消费 ＋ 文档 `2 → 0` ＋ 提交 `+1` 条 ＋ 由文档算出的帧数归 0。⚠ 修前这一条是红的：文档停在 2 |
//! | ③（这是**开关**，不是单向置位） | 第二下 `0 → 1`、第三下 `1 → 0`；三下合计 `+3` 条提交（⛔ 一次按键留两条会把撤销步数变成猜） |
//! | ④（兄弟字段逐字不动） | `Op::SetTransportConfig` 是**整体替换**三个字段 ⇒ `metronome_enabled` / `launch_quantization` 拨动前后**逐字段相同**；界面镜面 `metronome-on` 一位不动（⛔ 不顺手把节拍器写进文档，那会造出第二个事实源） |
//! | ⑤（重投影没把别的读数画歪） | 六格与预备拍**无关**的视图读数（`bpm-display` / `timecode` / 状态栏两格 / `active-tool` / `console-maximized`）在落点那句 `refresh_undo_window(…, reproject = true)` 前后**逐值不变**；对照取两档：装配时 vs 第一下后、三下后 vs 撤销后 |
//! | ⑥（一步撤销 ＋ redo） | `Cmd+Z` ⇒ 文档回到 `1`（撤的正是第三下那一笔）、`undone = 1`、`can_redo`；`Cmd+Shift+Z` ⇒ 再回到 `0` |
//! | ⑦（没有端口 ⇒ 不消费） | `undo = None` 的装配（判据 / 无头形态）里同一份按键返回 `false`（`.slint` 把它放行给焦点系统），⛔ 不假装提交过；对照臂钉住「这台装配的键路是活的」 |
//! | ⑧（录音那条腿读**活文档**） | `record_count_in` 的优先级三档：端口在场 ⇒ 读端口（即便装配期快照说 2 小节）；端口不在场 ⇒ 退回冻结快照；两份都没有 ⇒ 0。单位＝**帧** |
//! | ⑨（出处纪律） | 从源码现读（先剥 `//` 开头的行 ⇒ **注释不是实现站点**）：`Op::SetTransportConfig` 的构造点**恰好 1 处**、`record_count_in(` 定义＋唯一调用 = 2 处、`Action::ToggleCountIn` 在 `host.rs` 代码里**恰好 1 处**、`set_count_in_bars` 在 `host.rs` 与 `metronome.rs` 代码里 **0 处**（换代那条路真的拆了，⛔ 不是留一枚没人调的 `pub fn`）、借宿主那一族的判据只列节拍器 |
//!
//! ## 注入分两格（本文件的边界）
//!
//! * **键**：`Cmd/Ctrl+Alt+D` 经 `MainWindow::invoke_key_action`（`.slint` 的 `key-handler`
//!   派发快捷键时调的**同一个**回调，返回值就是「消费 / 不消费」），与
//!   `tests/clip_mute_toggle.rs` / `tests/ai_drawer_key_ui.rs` 同款；端口那侧的修饰键事件源
//!   由 `tests/d_ui_3_modifier_chords.rs` 负责，本文件不重复证明。
//! * **文档**：只读 `UndoPort::try_project()`（权威那一份），⛔ 不读界面属性冒充事实源。
//!
//! ## 如实登记的边界（⛔ 不假装整条链都通了）
//!
//! 1. **实时播放里仍然听不到预备拍**：`yeban-engine` 不实现它（上面那一栏）。本票修的是
//!    「UI 的开关 ⇒ 文档 ⇒ 读得到文档的读者」，⛔ 不是「走带时会数拍子」。那是**引擎功能请求**。
//! 2. **界面上没有预备拍那一格**：`ui/**/*.slint` 里 `count_in` / `countIn` / `预备拍`
//!    **0 命中**（实测：不加限定的 `count` 子串有 91 处，全是 `accessible-item-count` /
//!    `routing-count-label` / `undo-commit-count` 这一类，没有一处承载 `count_in_bars`）
//!    ⇒ 按键之后用户**看不见**状态。本票⛔ 新增控件：加控件 = 改像素 = 金帧重录 = 人类裁决。
//!    这条与「键位从画布进文本框」（`ADR-0068`）一起等 UI 面的裁决。
//!    ⚠ 因此 ⑤ 那六格读数是**无关项**（证的是重投影没顺手动别的），⛔ 不是「预备拍的显示」。
//! 3. `UndoPort` 在场但会话被控制面关掉（`try_project() == None`）⇒ 取数 **0 帧**、
//!    按键**不消费**并打一行 stderr；判据⛔ 造不出这一档 —— 实测口径：`UndoPort::try_project`
//!    的 `None` 只存在于 `UndoBackend::Authority` 那一支（特性 `in-process-mcp`，控制面可关工程），
//!    本文件的夹具是 `UndoBackend::Local`（打开后没有关掉工程的公开入口 ⇒ 恒 `Some`），故只登记不验。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部且每线程只能装配一次；每例一新线程由 libtest
//! 负责 ⇒ 本文件可以有多个 `#[test]`（与 `tests/clip_mute_toggle.rs` 同一做法）。
//! ⑧ 与 ⑨ 不建窗口（纯函数 / 读源码），因此⛔ 不碰平台。
//!
//! ## 本机不编译重依赖（`AGENTS.md` §5）
//!
//! Slint 是重依赖 ⇒ 本文件在本机**不编译**；编译与判决交 CI。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// `-D warnings` 会判红（与 `tests/clip_mute_toggle.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveUi, build_live_ui_with};
use yeban_app::host::record_count_in;
use yeban_app::record_wav::count_in_frames;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;

/// 预备拍键的**字面**（逻辑键 `Character('d')`，与 `.slint` 报来的 `event.text` 同形）。
const D_TEXT: &str = "d";

/// 取数用的采样率（单位：**赫兹**）。⚠ 这是本文件**问纯函数时自己给的**那一档，
/// 与产品默认的 `RecordOptions::sample_rate` 无关（那条腿的取值由 `plan` 决定）。
const RATE: u32 = 48_000;

/// 一小节的预备拍长度（单位：**帧**）。
///
/// 由夹具事实算出并在 ① 里**逐字核验**：`numerator = 4`、`bpm = 128.0` ⇒
/// `4 × 60 ÷ 128 = 1.875 s`，`1.875 × 48_000 = 90_000`（二进制精确，⛔ 不靠四舍五入）。
const FRAMES_PER_BAR: u64 = 90_000;

/// 装配一台**带权威会话**的活窗口（端口当装配选项传进去 ⇒ `host::wire_keys` 从第一刻起
/// 就握着它，与产品进程 `main.rs` 同一形态）。
fn assemble_with_session() -> (LiveUi, Rc<UndoPort>) {
    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<d-ui-23-count-in>", "yeban-app", project.clone(), NOW_MS)
            .expect("打开会话"),
    ));
    let ui = build_live_ui_with(
        &project,
        &live::LiveWiringOptions {
            permission: Permission::Interactive,
            undo: Some(Rc::clone(&port)),
            ..live::LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 执行面");
    (ui, port)
}

/// 装配一台**没有权威会话**的活窗口（判据 / 无头装配那一形）。
fn assemble_without_session() -> LiveUi {
    let project = yeban_model::samples::filled_project();
    build_live_ui_with(
        &project,
        &live::LiveWiringOptions {
            permission: Permission::Interactive,
            undo: None,
            ..live::LiveWiringOptions::default()
        },
    )
    .expect("真实界面（无撤销会话）+ Tier-1 执行面")
}

/// 权威文档的一份快照（读不到 ⇒ 直接判据红：那是前提不成立，⛔ 不是「改动没生效」）。
fn live_project(port: &UndoPort) -> yeban_model::YebanProjectV1 {
    port.try_project()
        .expect("端口必须有一份权威工程（没有 ⇒ 本文件的每一条都无从判起）")
}

/// 文档里的预备拍长度（单位：**小节**）。
fn bars(port: &UndoPort) -> u8 {
    live_project(port).transport.count_in_bars
}

/// 由**当前文档**算出的预备拍长度（单位：**帧**）—— 录音那条腿实际会拿到的那个数。
fn live_frames(port: &UndoPort) -> u64 {
    count_in_frames(&live_project(port), RATE)
}

/// 六格与预备拍**无关**的视图读数（⑤ 的对照面）。
///
/// 单位混着（字符串 / 整数 / 布尔）⇒ 逐格进元组，⛔ 印成一整行文本再比子串。
/// 选这六格的口径：本票⛔ 改它们的任何一个事实源（`bpm` / `time_signature` / 选区 /
/// 设备 / 工具 / 控制台最大化都**不是** `TransportConfig` 的成员），因此「重投影前后
/// 逐值相同」是唯一应该发生的事。
/// ⛔ 不含 `undo-commit-count` / `undo-node-count` —— 那两格正是被这次提交改动的东西。
fn view_readings(window: &MainWindow) -> (String, String, String, String, i32, bool) {
    (
        window.get_bpm_display().to_string(),
        window.get_timecode().to_string(),
        window.get_status_device().to_string(),
        window.get_status_selection().to_string(),
        window.get_active_tool(),
        window.get_console_maximized(),
    )
}

/// 注入一次 `Cmd/Ctrl+Alt+D`（`text, shift, control, alt, meta` 与 `key-action` 逐位对齐，
/// 与 `tests/ai_drawer_key_ui.rs` 的 `Cmd/Ctrl+Alt+A` 同款拼法）。
fn press_count_in(ui: &LiveUi) -> bool {
    ui.ui()
        .invoke_key_action(D_TEXT.into(), false, true, true, false)
}

/// 注入一次 `Cmd/Ctrl+Z`（撤销）。
fn press_undo(ui: &LiveUi) -> bool {
    ui.ui()
        .invoke_key_action("z".into(), false, true, false, false)
}

/// 注入一次 `Cmd/Ctrl+Shift+Z`（重做）。
fn press_redo(ui: &LiveUi) -> bool {
    ui.ui()
        .invoke_key_action("z".into(), true, true, false, false)
}

/// 把源码压成**只有代码行**：注释里出现过的字面量不算实现站点。
///
/// ⚠ 口径实测于 run `41516839490`（`tests/search_focus_key.rs` 的「唯一实现」数出 2，
/// 第二处在说明注释里）；本文件大量引用着这些字面量，数原文必然虚高。
fn code_only(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_count_in_press_writes_the_authoritative_document_and_is_reversible() {
    let (ui, port) = assemble_with_session();
    let window: &MainWindow = ui.ui();

    // ---- ① 前提读数（单位都点明，⛔ 无单位的数字不许进结论）----
    let opening = live_project(&port);
    assert_eq!(
        opening.transport.count_in_bars, 2,
        "① 夹具前提：`filled_project()` 的预备拍必须是 2 **小节**（读到 {} 小节）",
        opening.transport.count_in_bars
    );
    assert_eq!(
        opening.bpm, 128.0,
        "① 夹具前提：`bpm` 必须是 128.0（帧长换算靠它）"
    );
    assert_eq!(
        opening.time_signature.numerator, 4,
        "① 夹具前提：拍号分子必须是 4（单位：**拍 / 小节**）"
    );
    assert_eq!(
        live_frames(&port),
        FRAMES_PER_BAR * 2,
        "① 前提换算：2 小节 ⇒ 180 000 **帧**（{FRAMES_PER_BAR} 帧/小节 @ {RATE} Hz）"
    );
    let metronome_at_start = opening.transport.metronome_enabled;
    let quantization_at_start = opening.transport.launch_quantization;
    let commits_at_start = port.display().commit_count;
    let mirror_at_start = window.get_metronome_on();
    let view_at_start = view_readings(window);

    // ---- ② 第一下：文档真的被改 ----
    assert!(
        press_count_in(&ui),
        "② `Cmd/Ctrl+Alt+D` 必须被消费（返回 `false` ⇒ `.slint` 把它放行给焦点系统，\
         也就是「这一键按了什么都没发生」）"
    );
    assert_eq!(
        bars(&port),
        0,
        "② 第一下必须把**文档**里的预备拍拨成 0 **小节**（修前停在 2：那条臂压根不提交）"
    );
    assert_eq!(
        live_frames(&port),
        0,
        "② 由文档算出的长度必须归 0 **帧**（这就是录音那条腿将要拿到的数）"
    );
    assert_eq!(
        port.display().commit_count,
        commits_at_start + 1,
        "② 一次按键必须留下**恰好一条**提交（单位：条）"
    );
    // ⑤（第一档）：这一笔提交自带一次重投影 ⇒ 六格无关读数一位不动。
    assert_eq!(
        view_readings(window),
        view_at_start,
        "⑤ 落点里那句 `refresh_undo_window(…, reproject = true)` ⛔ 不许动这六格读数"
    );

    // ---- ③ 开关的两极性 ----
    assert!(press_count_in(&ui), "③ 第二下必须同样被消费");
    assert_eq!(
        bars(&port),
        1,
        "③ `0 → 1`：这是**开关**，不是单向关闭（0 小节再按一次必须回到 1 小节）"
    );
    assert_eq!(
        live_frames(&port),
        FRAMES_PER_BAR,
        "③ 1 小节 ⇒ 90 000 **帧**（与 ① 的换算同一口径）"
    );
    assert!(press_count_in(&ui), "③ 第三下必须同样被消费");
    assert_eq!(bars(&port), 0, "③ 第三下必须又关掉");
    assert_eq!(
        port.display().commit_count,
        commits_at_start + 3,
        "③ 三下合计必须**恰好 3** 条提交（单位：条）"
    );

    // ---- ④ 兄弟字段逐字不动（整体替换那条 `Op` 的代价就在这里）----
    let after = live_project(&port).transport;
    assert_eq!(
        after.metronome_enabled, metronome_at_start,
        "④ `metronome_enabled` 必须逐字带着走（⛔ 把节拍器的运行态镜面写进文档 ⇒ 第二个事实源）"
    );
    assert_eq!(
        after.launch_quantization, quantization_at_start,
        "④ `launch_quantization` 必须逐字带着走（本票⛔ 不碰场景量化）"
    );
    assert_eq!(
        window.get_metronome_on(),
        mirror_at_start,
        "④ 界面镜面 `metronome-on` 一位不动（预备拍那条臂⛔ 不碰节拍器那一格）"
    );
    let view_after_presses = view_readings(window);
    assert_eq!(
        view_after_presses, view_at_start,
        "⑤ 三下之后这六格读数仍与装配时逐值相同"
    );

    // ---- ⑥ 一步撤销 ＋ redo ----
    assert!(press_undo(&ui), "⑥ `Cmd/Ctrl+Z` 必须被消费");
    assert_eq!(
        bars(&port),
        1,
        "⑥ 撤销一步必须放回**上一条拨动之前**的值（第三下 `1 → 0` 被放回成 1 **小节**）"
    );
    assert_eq!(
        port.display().undone,
        1,
        "⑥ 已撤销步数必须是 1（单位：步；⛔ 一次按键撤掉两笔）"
    );
    assert!(port.display().can_redo, "⑥ 放回之后必须还能重做");
    // ⑤（第二档）：撤销那一次同样重投影 ⇒ 与「按下三下之后」那份快照比。
    assert_eq!(
        view_readings(window),
        view_after_presses,
        "⑤＋⑥ 撤销后的重投影 ⛔ 不许动这六格读数 —— 预备拍不画在任何控件上，\
         画歪了就是顺带改到了别的字段"
    );
    assert!(press_redo(&ui), "⑥ `Cmd/Ctrl+Shift+Z` 必须被消费");
    assert_eq!(bars(&port), 0, "⑥ redo 必须再拨回 0 **小节**");
    assert_eq!(
        live_frames(&port),
        0,
        "⑥ redo 后算出的长度必须又是 0 **帧**"
    );

    report_line(
        "[d-ui-23] 预备拍：文档 2→0→1→0（小节）、提交 +3（条）、撤销一步放回 1、redo 再回 0；\
         兄弟字段与六格视图读数一位不动",
    );
}

#[test]
fn a_count_in_press_without_an_undo_session_is_not_consumed() {
    let ui = assemble_without_session();
    // ⭐ 对照臂：先把「这台装配的键路是活的」钉住 —— 同一个形态下，一个**不需要**端口的
    // 动作必须照常消费（否则下面的 `false` 可能只是「这一键整体没接」，什么也不证明）。
    assert!(
        ui.ui()
            .invoke_key_action("3".into(), false, false, false, false),
        "⑦ 对照臂：无端口装配里数字键必须被消费（`Action::SelectTool` 不依赖端口）"
    );
    assert_eq!(
        ui.ui().get_active_tool(),
        3,
        "⑦ 对照臂：数字键必须真的换了工具（否则本文件的「不消费」无从判起）"
    );

    assert!(
        !press_count_in(&ui),
        "⑦ 没有权威会话 ⇒ 预备拍这一键⛔ 不得消费（既无文档可写，就只有「放行给焦点系统」\
         这一条诚实答案；⛔ 不假装成功）"
    );
    assert_eq!(
        ui.ui().get_active_tool(),
        3,
        "⑦ 不消费必须是**真的什么都没做**：工具读数一位不动"
    );
    report_line("[d-ui-23] 无端口装配：同一键返回 `false`（对照臂：数字键照常消费）");
}

/// ⑧ 取数口径（⛔ 不开窗口）：录音那条腿的优先级 —— 活文档 > 装配期快照 > 不知道。
#[test]
fn the_record_leg_prefers_the_live_document_over_the_assembly_snapshot() {
    let frozen = yeban_model::samples::filled_project();
    assert_eq!(
        count_in_frames(&frozen, RATE),
        FRAMES_PER_BAR * 2,
        "⑧ 前提：装配期那份快照算出的是 180 000 **帧**（修前录音读的就是它）"
    );

    // 档位一：端口在场、文档已被拨成 0 ⇒ 必须读到 **0**，⛔ 不是冻结快照里那个 180 000。
    let mut off = frozen.clone();
    off.transport.count_in_bars = 0;
    let port_off = Rc::new(UndoPort::new(
        UndoSession::open("<d-ui-23-off>", "yeban-app", off, NOW_MS).expect("打开会话"),
    ));
    assert_eq!(
        record_count_in(Some(port_off.as_ref()), Some(&frozen), RATE),
        0,
        "⑧ 端口说 0 小节 ⇒ 取数必须是 0 **帧**（修前这里给的是冻结快照的 180 000 ⇒ 键白按）"
    );

    // 档位一变体：文档是 1 小节、快照是 2 ⇒ 读到的是**文档**那一档。
    let mut one = frozen.clone();
    one.transport.count_in_bars = 1;
    let port_one = Rc::new(UndoPort::new(
        UndoSession::open("<d-ui-23-one>", "yeban-app", one, NOW_MS).expect("打开会话"),
    ));
    assert_eq!(
        record_count_in(Some(port_one.as_ref()), Some(&frozen), RATE),
        FRAMES_PER_BAR,
        "⑧ 端口 1 小节 / 快照 2 小节 ⇒ 必须给 90 000 **帧**（证的是「读端口」，\
         ⛔ 不是「非零就当快照」）"
    );
    assert_eq!(
        record_count_in(Some(port_one.as_ref()), None, RATE),
        FRAMES_PER_BAR,
        "⑧ 只有端口、没有快照 ⇒ 同样 90 000 **帧**（优先级与快照在场与否无关）"
    );

    // 档位二：没有端口（判据 / 无头装配）⇒ 退回装配期那份，既有形态一位不改。
    assert_eq!(
        record_count_in(None, Some(&frozen), RATE),
        FRAMES_PER_BAR * 2,
        "⑧ 无端口时必须照旧读冻结快照（180 000 **帧**）—— ⛔ 本票不改既有形态"
    );
    // 档位三：两份都没有 ⇒ 如实 0（⛔ 不假装知道工程）。
    assert_eq!(
        record_count_in(None, None, RATE),
        0,
        "⑧ 谁都不在场 ⇒ 0 **帧**"
    );

    report_line(
        "[d-ui-23] 取数三档：活文档 0 / 90 000 帧 ⇒ 照文档；无端口 ⇒ 照快照 180 000 帧；\
         都没有 ⇒ 0 帧",
    );
}

/// ⑨ 出处纪律（⛔ 不开窗口）：文档级提交只有一个落点、取数只有一个口径、
/// 换代那条路真的被拆掉了。
#[test]
fn the_document_is_the_only_count_in_landing_point() {
    let host = code_only(include_str!("../src/host.rs"));
    let metronome = code_only(include_str!("../src/metronome.rs"));

    // `Op::SetTransportConfig` 的构造点：只许一处（多处 = 两个落点各算各的「下一个值」，必分叉）。
    assert_eq!(
        host.matches("Op::SetTransportConfig {").count(),
        1,
        "⑨ `host.rs` 代码里 `Op::SetTransportConfig` 的构造点必须**恰好 1 处**（单位：处）"
    );
    // 取数口径：定义 + 唯一调用点。
    assert_eq!(
        host.matches("record_count_in(").count(),
        2,
        "⑨ `record_count_in(` 必须只有**定义 ＋ 唯一调用点**两处（实测 {} 处）—— \
         预备拍的取数只许一个真相源",
        host.matches("record_count_in(").count()
    );
    // 动作在宿主代码里只有一个臂（引擎集合里那份已经摘掉）。
    assert_eq!(
        host.matches("Action::ToggleCountIn").count(),
        1,
        "⑨ `Action::ToggleCountIn` 在 `host.rs` 代码里必须**恰好 1 处**（`apply_action` 那一臂）；\
         出现第二次就说明「换引擎」与「改文档」两条落点又并存了"
    );
    // 换代那条路：⛔ 不留一枚没人调的 `pub fn`。
    assert_eq!(
        host.matches("set_count_in_bars").count() + metronome.matches("set_count_in_bars").count(),
        0,
        "⑨ `metronome::set_count_in_bars` 必须已经**没有站点**（代码里 0 处）：\
         预备拍不再换引擎 ⇒ 那枚服务留着就是死代码"
    );
    // 借宿主那一族只剩节拍器（与 `only_engine_commands_take_the_engine_borrow` 同一口径）。
    assert_eq!(
        host.matches("matches!(action, Action::ToggleMetronome)")
            .count(),
        1,
        "⑨ `action_needs_engine` 的 `matches!` 必须只列节拍器**一条**（预备拍已摘出这条链）"
    );

    report_line(
        "[d-ui-23] 源码纪律：文档提交构造点 1 处、取数口径 2 处（定义＋调用）、动作臂 1 处、\
         set_count_in_bars 0 处",
    );
}
