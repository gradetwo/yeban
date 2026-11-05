//! `D-UI-20`（`[UI-A11Y-003]` §7.3 第 1 条的第二半）：导出弹窗里那三格**手绘控件**
//! 第一次摸得到键盘，并且按下去真的走到底 —— 加上「`Return` = 默认动作」。
//!
//! ## 这一票修的是哪个真实缺陷
//!
//! `D-UI-19` 让模态持住了键盘，但轮转只在**四个 `TextInput`** 之间：✕、「导出」、
//! 限制器开关三格是 `Rectangle` + `TouchArea`，**没有** `FocusScope` ⇒ 收不到焦点
//! ⇒ 只用键盘的人填得完四个框，却⛔ **按不动导出**。规范那句"全键盘无障碍"在这一格
//! 是字面失败的，而且失败得很安静：界面看起来一切正常，只是键盘走到底就没了。
//!
//! 现在每格里加一枚同尺寸、**零视觉**的 `FocusScope`（⛔ 不画焦点描边 —— 见下面的边界），
//! `Return` / `Space` 触发的是**既有那一条**回调；`Tab` 轮转从四格扩到**七格**。
//! 另外四个文本框接上 `accepted => { root.run(); }` ⇒ `Return` 即默认动作。
//!
//! ## 为什么这两件事能同时成立（实测，Slint 1.18.1）
//!
//! | 事实 | 出处 |
//! | :--- | :--- |
//! | `FocusScope::input_event_filter_before_children` 恒 `ForwardEvent` | `items/input_items.rs:657-665` |
//! | `focus-on-click: false` ⇒ `input_event` 对 `Pressed` 返回 `EventIgnored`（只有 `enabled && focus-on-click && !has_focus` 才吞） | 同文件 667-688 |
//! | ⇒ 这枚容器**不与 `TouchArea` 抢命中**；`TouchArea` 又排在它之后（更上层），鼠标那条路一位不变 | — |
//! | `.focus()` ⇒ `set_focus_item(item, true, FocusReason::Programmatic)`，而 `focus_event` 只对 `TabNavigation` / `PointerClick` 两种理由设闸 ⇒ **程序取焦恒成立** | `i-slint-compiler` 的 `generator/rust.rs:4541` ＋ `input_items.rs:780-793` |
//! | `Key.Return` = `\u{000a}`、`Key.Space` = `\u{0020}`；`Key` 是**内建命名空间**，`.slint` 里无需 import | `i-slint-common/key_codes.rs:34,54` ＋ `i-slint-compiler/lookup.rs:1094` |
//! | 注入侧同码位：`KeyCode::Return → '\n'`、`KeyCode::Space → ' '` | `crates/yeban-ui-test-port/src/port.rs:343-348,435` |
//! | `TextInput.accepted` 仅在**非只读 + 单行**时触发（四框都满足），触发即吃掉这一键 ⇒ ⛔ 不会既打字又导出 | `i-slint-core/items/text.rs:1068-1074` |
//!
//! ## 判据（一条链，九个读数 ＋ 一条出处纪律）
//!
//! ① 起点：弹窗关着 ⇒ 七格读数 `-1`、快捷键层持焦、目标文件不存在、状态行空；
//! ② 打开（宿主入口）⇒ 读数 `0`、四框读数 `0`、快捷键层失去焦点；
//! ③ `Tab` ×4 ⇒ `1→2→3→4`：第 4 格是**限制器**（`D-UI-19` 之后新加的第一格按钮），
//!    此时四框读数如实为 `-1`（焦点不在文本框里），期间左栏搜索框从未持焦、
//!    画布工具一位不动；
//! ④ 焦点在限制器格上按 `Space` ⇒ `export-wav-limiter` 翻真（开关真的被键盘拨动了），
//!    且焦点**不移动**（激活≠换格）；
//! ⑤ 再 `Tab` ⇒ 第 5 格「导出」；按 `Return` ⇒ **落盘一个真 WAV**：回执
//!    `action=export-wav` / `outcome=ok` / `limiter=on` / `frames>0`，文件非空。
//!    ⚠ 同时钉住两条既有事实：弹窗**不关**（`set_export_wav_open(` 全仓只有两个写者，
//!    导出这条路不在其中）、焦点仍在那一格；
//! ⑥ 再 `Tab` ⇒ 第 6 格 ✕；按 `Return` ⇒ 走的是**既有的** `close()`：弹窗关、两个读数
//!    都回 `-1`、焦点复位快捷键层、弹窗离开运行时树；
//! ⑦ 重开 ⇒ `0`；Backtab ×1 ⇒ `6`（第一格再退 = 末格，反向也闭合）、再退 ×2 ⇒ `5→4`；
//! ⑧ 退到第 3 格（时长框）后由 Rust 改 `seconds = "1"`，按 `Return` ⇒ `accepted` 走
//!    同一条 `run()`：新回执的 `frames` **严格小于** ⑤（改了的那一格真的被消费了，
//!    ⛔ 不是拿上一条回执自比），且框内文本逐字仍是 `1`（`Return` ⛔ 没被打进文本），
//!    弹窗仍开着；
//! ⑨ 焦点在按钮格上按一个 DAW 语义键（`1`）⇒ 工具不动、文本框不变、状态行不变
//!    ——「模态持键盘」对按钮格同样成立。
//!
//! ⑩ 出处纪律（第二条 `#[test]`，⛔ 不开窗口）：七格的「序号 → 控件」表**只有一份**
//!    （`.focus()` 调用点恒 8 = 首格 1 + 表 7）、三格按钮的键盘落点与它们的
//!    `TouchArea` / 无障碍动作**是同一条回调**（`root.run()` 出现 7 处，全指向同一条
//!    `callback run()`）、⛔ 不新增可寻址控件（`accessible-id:` 恒 10 ⇒ 默认帧与
//!    `tests/golden/linux/` 的 5 张基准一位不改）、两个焦点读数都仍是**推导**。
//!
//! ## 已知边界（如实登记，⛔ 不假装覆盖）
//!
//! * **焦点落在按钮格上看不见**。三格容器刻意不画描边：画了就改像素，而弹窗打开态
//!   牵动两条已判绿的视觉基准 ⇒ 改基准属人类/集成者裁决（`docs/ledger/` §32 边界 2）。
//!   ⇒ 今天键盘用户**走得**到这三格、**按得**动，只是看不出站在哪一格。
//! * **鼠标点击不改变焦点**（三格都 `focus-on-click: false`）⇒ 这一票⛔ 没有测鼠标：
//!   `LiveUi` 只暴露按键注入（指针注入在 `LiveControlPlane` 上，而本判据需要的是
//!   **带焦点**的按键通道）。"容器不抢命中"是**从上游源码**证的（上表前三行），
//!   ⛔ 不是从运行时证的。把它连同 `D-UI-21`（遮罩不吞鼠标）一并重判 ⇒ 另票。
//! * `Shift+Tab` 在真键盘上以 Backtab `\u{19}` 到达，本判据注入的就是那个码位；
//!   trap 里按修饰位判的 `Tab`+`shift` 分支仍⛔ 注入不出来（与 `D-UI-19` 同边界）。
//! * 三格各自的 `FocusScope` **不参与全局轮转**：`Tab` 一律先被 trap 吞掉再自己分配，
//!   所以 Slint 内建那条整窗链根本没机会跑（`focus-on-tab-navigation: false` 在 trap 上）。
//!
//! ## 判别力（改动 ⇒ 红；缺行就说明那条断言本来就是空的）
//!
//! | 若改成 | 本判据 |
//! | :--- | :--- |
//! | 删掉限制器格那枚 `FocusScope` | ③ 红（`Tab` 到不了 `4`，直接绕回 `0`）、④⑤ 随之全红 |
//! | 把 `key-pressed` 里的 `Key.Space` 那一半去掉 | ④ 红（`Space` 拨不动开关；`Return` 仍有效 ⇒ 只红在开关那一步） |
//! | 按钮格的键盘落点写成"另开一份导出"（例如直接 `root.close()`） | ⑤ 红（没有 `outcome=ok` 回执 / 文件不落盘），⑩ 红（`root.run()` 计数变化） |
//! | 删掉四框任一条 `accepted` | ⑧ 红（`Return` 变成打字或不做事，`frames` 不更新） |
//! | 「导出」之后再 `set_export_wav_open(false)`（顺手关弹窗） | ⑤ 红（钉的是**既有**行为：导出从不关弹窗）—— ⛔ 这条不是放宽，是把事实写对；真要改成"导出即关闭"是另一票界面决定 |
//! | 轮转表在别处再抄一遍（多出第 9 个 `.focus()`） | ⑩ 红 |
//! | 给某个 `FocusScope` 加 `accessible-id` | ⑩ 红（10 ⇒ 11），且金帧/寻址面基准要重录 —— 本票刻意不碰 |
//! | 把 `focus-on-click` 打开 | 本判据⛔ 测不到（不测鼠标）⇒ 如实登记在边界里，不是覆盖到了 |

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的装配入口 ⇒ 其余项在本测试目标里是 dead_code。
#[allow(dead_code)]
mod live;

use live::{LiveWiringOptions, build_live_ui_with};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::port::{KeyCode, Permission};
use yeban_ui_test_port::render::report_line;

/// 七格的语义顺序（⛔ 不是新的状态，只是给判据一个可读的名字）。
const PATH: i32 = 0;
const SAMPLE_RATE: i32 = 1;
const BIT_DEPTH: i32 = 2;
const SECONDS: i32 = 3;
const LIMITER: i32 = 4;
const RUN: i32 = 5;
const CLOSE: i32 = 6;

/// 临时目录：`Drop` 里清掉，判据不留垃圾文件。
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("yeban-d-ui-20-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 取回执里的一个顶层字段（`key=value` 以 `:` 分隔）。
///
/// ⚠ 只做**前缀**匹配，因此 `limiter=` 不会撞上 `limiter_reductions=`（分隔后每一段
/// 恰好一个字段）。回执里唯一可能含 `:` 的是 `path=` 与 `clipping_note=`，本判据不读二者
/// 的值（只读 `action` / `outcome` / `frames` / `limiter`），故不做引号感知。
fn field_of(receipt: &str, key: &str) -> String {
    let needle = format!("{key}=");
    for part in receipt.split(':') {
        if let Some(value) = part.strip_prefix(needle.as_str()) {
            return value.to_owned();
        }
    }
    panic!("回执里没有字段 `{key}`: {receipt}");
}

fn assert_nonempty_file(path: &Path) {
    let meta = std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("键盘导出的产物没落盘 {}: {error}", path.display()));
    assert!(meta.is_file(), "{} 不是文件", path.display());
    assert!(meta.len() > 0, "{} 是 0 字节", path.display());
}

#[test]
fn the_three_hand_drawn_controls_are_reachable_and_firable_by_keyboard() {
    let project = yeban_model::samples::filled_project();
    let scratch = Scratch::new("keyboard");
    let target = scratch.0.join("keyboard-fired-export.wav");

    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<判据 D-UI-20: 三格接键盘>",
            "yeban-app",
            project.clone(),
            1,
        )
        .expect("打开撤销会话"),
    ));

    // 导出落点 = 临时目录里那一个文件。宿主打开弹窗时把它播种进路径框（非空 ⇒ 之后不会被覆写）。
    // SAFETY: 本测试二进制只有一个开窗口的 `#[test]` ⇒ 没有第二个线程读这个变量；
    // 设置发生在任何装配之前。
    unsafe {
        std::env::set_var("YEBAN_UI_EXPORT_WAV_PATH", &target);
    }

    let mut ui = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            console_tab: 0,
            save_path: None,
            engine_quanta: 0,
            undo: Some(Rc::clone(&port)),
            proposal_source: yeban_app::proposals::ProposalSource::none(),
        },
    )
    .expect("真实界面 + Tier-1 执行面");
    ui.pump_meters();

    let mut trace: Vec<String> = vec![format!(
        "起点 control={}",
        ui.ui().get_export_wav_focused_control()
    )];

    // ---- ① 起点：弹窗关着 ⇒ 焦点归快捷键层，两个读数都是 `-1` ----
    assert_eq!(
        ui.ui().get_export_wav_focused_control(),
        -1,
        "① 起点：弹窗没开 ⇒ 焦点不许在七格里的任何一格"
    );
    assert_eq!(
        ui.ui().get_export_wav_focused_field(),
        -1,
        "① 起点：四框读数同样为 -1"
    );
    assert!(
        ui.ui().get_shortcut_layer_focused(),
        "① 起点：快捷键层必须持焦"
    );
    assert!(
        !target.exists(),
        "① 起点：{} 不该已经存在（⑥ 要靠它证「✕ 不导出」）",
        target.display()
    );
    assert!(
        ui.ui().get_module_status().is_empty(),
        "① 起点：状态行必须是空的（⑤ 要靠它证「这一键真的跑了动作」）"
    );

    // 先给画布一个**已知的**工具，⑨ 的"到不了宿主"才有判别力。
    ui.dispatch_key_press(KeyCode::Character('3'))
        .expect("① 前置：`3` 必须落到真实窗口");
    assert_eq!(ui.ui().get_active_tool(), 3, "① 前置：`3` 没落到宿主");

    // ---- ② 打开：走宿主的入口（与用户点 `module-export-wav-button` 同一条链）----
    ui.ui().invoke_open_export_wav();
    ui.pump_meters();
    assert_eq!(
        ui.ui().get_export_wav_focused_control(),
        PATH,
        "② 打开 ⇒ 焦点落第一格（路径框）"
    );
    assert_eq!(
        ui.ui().get_export_wav_focused_field(),
        PATH,
        "② 打开 ⇒ 四框读数同为 0"
    );
    assert!(
        !ui.ui().get_shortcut_layer_focused(),
        "② 弹窗持键盘时快捷键层 ⛔ 不得同时持焦"
    );
    assert!(
        !ui.ui().get_export_wav_limiter(),
        "② 前置：限制器必须默认关着（④ 翻的是**这一格**，不是读到一条恒真）"
    );

    // ---- ③ `Tab` ×4 ⇒ 1→2→3→4（第 4 格是新接上键盘的限制器开关）----
    for (step, expected) in [SAMPLE_RATE, BIT_DEPTH, SECONDS, LIMITER]
        .iter()
        .enumerate()
    {
        ui.dispatch_key_press(KeyCode::Tab)
            .expect("③ `Tab` 必须落到真实窗口");
        let got = ui.ui().get_export_wav_focused_control();
        assert_eq!(
            got,
            *expected,
            "③ 第 {} 次 `Tab` 之后焦点应在第 {} 格（读到 {got}）",
            step + 1,
            expected
        );
        assert!(
            !ui.ui().get_sidebar_search_focused(),
            "③ 焦点 ⛔ 不许逃出模态落到左栏搜索框"
        );
        assert_eq!(ui.ui().get_active_tool(), 3, "③ 轮转期间工具不许被动");
    }
    assert_eq!(
        ui.ui().get_export_wav_focused_field(),
        -1,
        "③ 焦点在**按钮格**上时四框读数必须如实为 `-1`（⛔ 不许谎报「还在文本框里」）"
    );
    trace.push("Tab×4 ⇒ 1→2→3→4".to_owned());

    // ---- ④ `Space` 拨动限制器（格内激活，焦点不移动）----
    ui.dispatch_key_press(KeyCode::Space)
        .expect("④ `Space` 必须落到真实窗口");
    assert!(
        ui.ui().get_export_wav_limiter(),
        "④ 焦点在限制器格上按 `Space` ⇒ 开关必须翻真（这是键盘第一次拨得动它）"
    );
    assert_eq!(
        ui.ui().get_export_wav_focused_control(),
        LIMITER,
        "④ 激活一格 ⛔ 不得把焦点搬走"
    );

    // ---- ⑤ `Tab` 到「导出」，`Return` ⇒ 真的落盘 ----
    ui.dispatch_key_press(KeyCode::Tab)
        .expect("⑤ `Tab` 必须落到真实窗口");
    assert_eq!(
        ui.ui().get_export_wav_focused_control(),
        RUN,
        "⑤ 第 5 格必须是「导出」"
    );
    // 四框一位不改（此刻是宿主播种 + `app.slint` 的缺省 `auto` / `24` / `8`）⇒
    // ⑧ 之后拿"改了时长的那一次"与这一次比，才有严格的 `frames` 关系。
    ui.dispatch_key_press(KeyCode::Return)
        .expect("⑤ `Return` 必须落到真实窗口");
    ui.pump_meters();
    let first = ui.ui().get_module_status().to_string();
    assert!(
        ui.ui().get_module_status_succeeded(),
        "⑤ 焦点在「导出」上按 `Return` 必须跑成一次真导出: {first}"
    );
    assert_eq!(field_of(&first, "action"), "export-wav", "{first}");
    assert_eq!(field_of(&first, "outcome"), "ok", "{first}");
    assert_eq!(
        field_of(&first, "limiter"),
        "on",
        "⑤ 回执里的限制器必须是 ④ 用键盘翻开的那个: {first}"
    );
    let frames_first: u64 = field_of(&first, "frames").parse().unwrap_or_else(|error| {
        panic!(
            "⑤ `frames` 不是整数 `{}`: {error}",
            field_of(&first, "frames")
        )
    });
    assert!(frames_first > 0, "⑤ `frames` 必须 > 0: {first}");
    assert_nonempty_file(&target);
    assert!(
        ui.ui().get_export_wav_open(),
        "⑤ 导出**不关**弹窗（既有事实：`set_export_wav_open(` 的两个写者都不是这条路）⇒ \
         这一条钉的是现状，⛔ 不是把「导出即关闭」当已实现"
    );
    assert_eq!(
        ui.ui().get_export_wav_focused_control(),
        RUN,
        "⑤ 导出之后焦点仍在那一格"
    );
    trace.push(format!("Return@导出 ⇒ outcome=ok frames={frames_first}"));

    // ---- ⑥ `Tab` 到 ✕，`Return` ⇒ 关弹窗（同一条 `close()`），且不跑动作 ----
    let status_before_close = ui.ui().get_module_status().to_string();
    ui.dispatch_key_press(KeyCode::Tab)
        .expect("⑥ `Tab` 必须落到真实窗口");
    assert_eq!(
        ui.ui().get_export_wav_focused_control(),
        CLOSE,
        "⑥ 第 6 格必须是 ✕ 关闭"
    );
    ui.dispatch_key_press(KeyCode::Return)
        .expect("⑥ `Return` 必须落到真实窗口");
    ui.pump_meters();
    assert!(
        !ui.ui().get_export_wav_open(),
        "⑥ 焦点在 ✕ 上按 `Return` 必须关掉弹窗（走的是既有的 `close()`）"
    );
    assert_eq!(
        ui.ui().get_export_wav_focused_control(),
        -1,
        "⑥ 关闭之后七格读数必须回 `-1`"
    );
    assert_eq!(
        ui.ui().get_export_wav_focused_field(),
        -1,
        "⑥ 关闭之后四框读数必须回 `-1`"
    );
    assert!(
        ui.ui().get_shortcut_layer_focused(),
        "⑥ 关闭之后焦点必须复位到快捷键层"
    );
    assert!(
        !ui.tree_snapshot().contains("export-wav-dialog"),
        "⑥ 关闭之后弹窗必须离开运行时树"
    );
    assert_eq!(
        ui.ui().get_module_status().to_string(),
        status_before_close,
        "⑥ ✕ 那条 `Return` ⛔ 不得顺手跑任何动作（状态行一位不变）"
    );
    trace.push("Return@✕ ⇒ 关闭、状态行不变".to_owned());

    // ---- ⑦ 重开 ⇒ 0；反向轮转也闭合：0 → 6 → 5 → 4 ----
    ui.ui().invoke_open_export_wav();
    ui.pump_meters();
    assert_eq!(
        ui.ui().get_export_wav_focused_control(),
        PATH,
        "⑦ 重开 ⇒ 焦点回第一格"
    );
    for (step, expected) in [CLOSE, RUN, LIMITER].iter().enumerate() {
        ui.dispatch_key_press(KeyCode::Character('\u{19}'))
            .expect("⑦ Backtab 必须落到真实窗口");
        let got = ui.ui().get_export_wav_focused_control();
        assert_eq!(
            got,
            *expected,
            "⑦ 第 {} 次 Backtab 之后焦点应在第 {} 格（读到 {got}）",
            step + 1,
            expected
        );
    }
    trace.push("Backtab×3 ⇒ 6→5→4（反向绕回）".to_owned());

    // ---- ⑧ `Return` 在文本框里 = 默认动作，且它消费的是**当前**那格的值 ----
    ui.dispatch_key_press(KeyCode::Character('\u{19}'))
        .expect("⑧ Backtab 必须落到真实窗口");
    assert_eq!(
        ui.ui().get_export_wav_focused_control(),
        SECONDS,
        "⑧ 退一格应在时长框（焦点在框里才谈 `accepted`）"
    );
    assert_eq!(
        ui.ui().get_export_wav_focused_field(),
        SECONDS,
        "⑧ 四框读数必须与七格读数在这一格一致"
    );
    // 由 Rust 改这一格（两向绑定的正向那半），再把 `Return` 按下去。
    ui.ui().set_export_wav_seconds("1".into());
    ui.dispatch_key_press(KeyCode::Return)
        .expect("⑧ `Return` 必须落到真实窗口");
    ui.pump_meters();
    let second = ui.ui().get_module_status().to_string();
    assert!(
        ui.ui().get_module_status_succeeded(),
        "⑧ 文本框里的 `Return` 必须跑成一次导出: {second}"
    );
    assert_eq!(field_of(&second, "outcome"), "ok", "{second}");
    let frames_second: u64 = field_of(&second, "frames")
        .parse()
        .expect("`frames` 是整数");
    assert!(
        frames_second < frames_first,
        "⑧ 改了时长之后 `frames` 必须**变小**（{frames_second} < {frames_first}）：\
         证这一键跑的是**新的一次**导出、消费的是改过的那一格 —— ⛔ 不是拿上一条回执自比"
    );
    assert_eq!(
        ui.ui().get_export_wav_seconds().to_string(),
        "1",
        "⑧ `Return` ⛔ 不许被当成字符打进去（框内文本逐字仍是 `1`）"
    );
    assert!(
        ui.ui().get_export_wav_open(),
        "⑧ `accepted` 触发的是 `run()`，⛔ 不是 `close()` ⇒ 弹窗仍开着"
    );
    trace.push(format!("Return@时长框 ⇒ frames={frames_second}（比 ⑤ 小）"));

    // ---- ⑨ 焦点在按钮格上时，DAW 语义键仍然到不了宿主 ----
    ui.dispatch_key_press(KeyCode::Tab)
        .expect("⑨ `Tab` 必须落到真实窗口");
    ui.dispatch_key_press(KeyCode::Tab)
        .expect("⑨ `Tab` 必须落到真实窗口");
    assert_eq!(
        ui.ui().get_export_wav_focused_control(),
        RUN,
        "⑨ 前置：焦点必须挪到「导出」格（不在文本框里）"
    );
    ui.dispatch_key_press(KeyCode::Character('1'))
        .expect("⑨ `1` 必须落到真实窗口");
    assert_eq!(
        ui.ui().get_active_tool(),
        3,
        "⑨ 焦点在按钮格上按 `1` ⛔ 不得换工具"
    );
    assert_eq!(
        ui.ui().get_export_wav_seconds().to_string(),
        "1",
        "⑨ 这一键也不许落进任何文本框"
    );
    assert_eq!(
        ui.ui().get_module_status().to_string(),
        second,
        "⑨ 这一键⛔ 不得触发任何动作（状态行一位不变）"
    );
    trace.push("Return@导出格以外的 `1` ⇒ 三处读数一位不动".to_owned());

    report_line(&format!("[export-button-focus] {}", trace.join(" ⇒ ")));
}

/// ⑩ **出处纪律**（⛔ 不开窗口）：轮转表只有一份、三格按钮的键盘落点与鼠标/无障碍
/// 落点是**同一条回调**、寻址面一位不改、两个读数都仍是推导。
///
/// 手法取自 `tests/export_dialog_key_capture.rs` 的 ⑧：读自己的源码。
///
/// ⚠ 计数一律走 `code_only`（先删 `//` 行）：本文件在 `.slint` 里写了大量解释性注释，
/// 其中就引用着这些字面量（`root.run()` / `Key.Return` / `focus-on-click: false`），
/// 数原文会把解释当成实现。同一条教训实测于 run `41516839490`（`search_focus_key.rs` ⑤）。
#[test]
fn the_rotation_table_is_single_sourced_and_no_new_control_becomes_addressable() {
    let read = |rel: &str| -> String {
        let full = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        std::fs::read_to_string(&full)
            .unwrap_or_else(|error| panic!("读 `{}` 失败: {error}", full.display()))
    };
    let dialog = code_only(&read("ui/dialogs/export_wav_dialog.slint"));
    let app = code_only(&read("ui/app.slint"));
    let host = code_only(&read("src/host.rs"));

    // ---- 取焦只有一张表 ----
    // `focus-first-field` 1 处 + `focus-control` 七处；轮转的两条只做序号加减。
    assert_eq!(
        dialog.matches(".focus()").count(),
        8,
        "⑩ 取焦调用点必须恰好 8 处（首格 1 + 「序号 → 控件」表 7）；\
         多出来的那一处意味着轮转表在别处又被抄了一遍"
    );
    assert_eq!(
        dialog.matches("public function focus-").count(),
        4,
        "⑩ 焦点函数必须恰好四条：`focus-first-field` / `focus-control`（表）/ \
         `focus-next-control` / `focus-previous-control`"
    );

    // ---- 三格按钮各有且仅有一枚零视觉容器 ----
    assert_eq!(
        dialog.matches(":= FocusScope").count(),
        4,
        "⑩ 焦点作用域必须是 4 枚：`export-trap` ＋ 三格按钮（✕ / 限制器 / 导出）各一枚。\
         第五枚就是第二个键盘入口"
    );
    assert_eq!(
        dialog.matches("focus-on-click: false").count(),
        4,
        "⑩ 四枚容器⛔ 都不吃鼠标点击（trap 一条 ＋ 三格各一条）⇒ 点击的落点仍是 `TouchArea`"
    );
    for key in ["Key.Return", "Key.Space"] {
        assert_eq!(
            dialog.matches(key).count(),
            3,
            "⑩ `{key}` 必须恰好出现 3 次（三格按钮各一条键盘落点）；实测 {}",
            dialog.matches(key).count()
        );
    }

    // ---- 落点同一条：`run` 只有一条回调，七个调用点全写 `root.run()` ----
    assert_eq!(
        dialog.matches("callback run();").count(),
        1,
        "⑩ 导出回调必须只有一条声明"
    );
    assert_eq!(
        dialog.matches("root.run()").count(),
        7,
        "⑩ `root.run()` 必须恰好 7 处：「导出」的无障碍 / 鼠标 / 键盘三条 ＋ 四个框的 `accepted`。\
         数目变了就是有人另开了一份「导出」的实现"
    );
    assert_eq!(
        dialog.matches("accepted => { root.run(); }").count(),
        4,
        "⑩ `Return` = 默认动作必须四框各有且仅有一条"
    );
    assert_eq!(
        dialog.matches("root.close()").count(),
        4,
        "⑩ 关闭必须四条落点同一实现：✕ 的无障碍 / 鼠标 / 键盘 ＋ trap 里那条 `Esc`\
         （四个入口，一条回调 ⇒ ⛔ 没有第二份「关弹窗」的逻辑）"
    );

    // ---- 两个读数都是推导 ----
    for name in ["focused-field", "focused-control"] {
        assert!(
            dialog.contains(&format!("out property <int> {name}:")),
            "⑩ `{name}` 必须是 `out`（弹窗只报数，宿主不许往里写）"
        );
        assert_eq!(
            writes_of(name, &dialog)
                + writes_of(name, &app)
                + writes_of(&format!("export-wav-{name}"), &app),
            0,
            "⑩ 焦点读数 `{name}` 必须是**推导**（⛔ 不许有第三份「我以为焦点在哪」的写态）"
        );
    }
    assert!(
        app.contains(
            "out property <int> export-wav-focused-control: export_dialog.focused-control;"
        ),
        "⑩ MainWindow 那一条必须是**纯转发**，⛔ 不重新推导一遍"
    );
    assert_eq!(
        host.matches("export_wav_focused_control").count(),
        0,
        "⑩ Rust 侧⛔ 既不读也不写这条读数（一出现 snake_case 名字就是有人把它当状态用了）"
    );

    // ---- 寻址面一位不改 ----
    assert_eq!(
        dialog.matches("accessible-id:").count(),
        10,
        "⑩ 弹窗里的可寻址控件数必须是 10（本票⛔ 不新增控件 ⇒ 默认帧、金帧、运行时树的寻址面都一位不改）"
    );

    // ---- 宿主侧的两处既有事实，本票一位未动 ----
    assert_eq!(
        host.matches("invoke_focus_export_wav_dialog").count(),
        1,
        "⑩ 取焦必须只有打开弹窗那一处"
    );
    assert_eq!(
        host.matches("set_export_wav_open(").count(),
        2,
        "⑩ 弹窗开关的写者必须恰好两处（打开 + 关闭）⇒ ⑤ 断的「导出从不关弹窗」仍然是事实"
    );
}

/// 只保留**代码行**（删掉 `//` / `///` 开头的行）再交给计数。
///
/// 本票在 `.slint` 里写下了大量"为什么这样接"的注释，而注释里就引用了被数的字面量
/// （`root.run()`、`Key.Return`、`focus-on-click: false`、方法名）。数原文 = 把解释
/// 当成实现 ⇒ 判据假红。⛔ 反过来也不许"为了喂判据删注释"。
fn code_only(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 数「以单个 `=` 写这个标识符」的出现次数（`==` / `>=` / `<=` / `!=` 都不算写）。
///
/// ⚠ 必须带**边界**判断：`export-wav-focused-control` 里含着 `focused-control`，
/// 不查前一个字符就会把它数成两遍。
fn writes_of(name: &str, source: &str) -> usize {
    let bytes = source.as_bytes();
    source
        .match_indices(name)
        .filter(|(offset, _)| {
            let offset = *offset;
            let after = offset + name.len();
            let independent = (offset == 0 || !is_ident(bytes[offset - 1]))
                && (after == bytes.len() || !is_ident(bytes[after]));
            let tail = source[after..].trim_start();
            independent && tail.starts_with('=') && !tail.starts_with("==")
        })
        .count()
}

/// `.slint` 与 Rust 的标识符字符集在这里取并集（kebab-case 的 `-` 必须算，
/// 否则上面那条边界判断形同不存在）。
fn is_ident(byte: u8) -> bool {
    (byte as char).is_alphanumeric() || byte == b'-' || byte == b'_'
}
