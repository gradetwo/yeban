//! `[UI-A11Y-003]` §7.3 第 1 条 / `D-UI-19`：**导出弹窗开着的时候键盘必须归它**，
//! 四格输入框（路径 / 采样率 / 位深 / 时长）第一次真的能只用键盘填完。
//!
//! ## 这一票修的是哪个真实缺陷
//!
//! 弹窗是**整帧遮罩**（`ui/dialogs/export_wav_dialog.slint` 的 `background: mask-black`），
//! 在此之前它自己不接键盘：焦点始终停在 `key-handler` 那层，于是「遮罩盖住画面」与
//! 「键盘归 DAW」**同时成立** —— 用户正准备填导出路径，按 `Space` 后面的走带开始播放，
//! 按 `1` 画布换了工具，而四个框**一个都进不去**（`D-UI-14` 抓到左栏搜索框同款的
//! 「有控件、摸不到键盘」，这一格是它的模态版）。
//!
//! 现在 `export-trap` 是那八格的**祖先** `FocusScope`（⛔ 不能像时光机那样放成兄弟覆盖层：
//! Slint 的投递是「焦点项 ⇒ 只有**祖先**链」，见 `i-slint-core-1.18.1/window.rs` 里
//! `Deliver key_event (to focused item, going up towards the window)` 那一段 ——
//! 兄弟接不到 `TextInput` 拒绝的 `Tab`）。
//!
//! ## 判据（一条链，八个读数）
//!
//! ① 起点：弹窗关着 ⇒ 焦点读数 `-1`、快捷键层持焦、`export-wav-dialog` 不在树里；
//! ② 打开（**宿主的入口**，与用户点按钮同一条链）⇒ 焦点落在第一格、快捷键层**失去**焦点；
//! ③ 打一个可打印字符 ⇒ 它进**路径框**（两向绑定 `text <=> root.path` 的证据），
//!    且画布的工具 / 视图一位不动；
//! ④ `Tab` ×8 ⇒ `1→2→3→4→5→6→7→0`（**闭环 + 绕回**），期间左栏搜索框**从未**被选中，
//!    画布语义的切视图 ⛔ 没被触发；并核对两条焦点读数**一致**（焦点在 5/6/7 那三格
//!    按钮上时，四框读数如实为 `-1`）；
//! ⑤ Backtab（`Shift+Tab` 的到达形态）⇒ 反向绕回，`0→7→6→5→4→3→2→1→0`；
//! ⑥ 弹窗持键盘时按 `1` ⇒ 它落进**当前那一格**，`active-tool` ⛔ 不变 ——
//!    与 ⑦ 的最后一个是**同一个键、两种落点**，那才是「捕获」二字的判别力所在；
//! ⑦ `Esc` ⇒ 关闭 + 焦点**复位**到快捷键层 ⇒ 随后 `1` 又到达宿主（`active-tool` = 1）；
//! ⑧ 出处纪律（另一条 `#[test]`）：轮转只定义一次、焦点读数是**推导**而非第三份状态、
//!    没有新增可寻址控件 ⇒ 默认帧与 `tests/golden/linux/` 的 5 张基准一位不改。
//!
//! ## `T-FIX-219` 恢复说明（本文件曾被 `21f70fd1` 退化成纯字符串判据）
//!
//! 行为判据按 `tests/modal_focus_entry.rs` 的活窗口形恢复：`build_live_ui_with` +
//! `dispatch_key_press`，打开走真宿主回调 `invoke_open_export_wav()`，断言读的是
//! 活窗口的属性，⛔ 不读源码。八格读数（`T-FIX-57` 加浏览格后）与
//! `tests/export_dialog_button_focus.rs` 同口径。
//!
//! ## 为什么这里能按「焦点值」断言（既有 a11y 判据做不到）
//!
//! 控件树里没有焦点字段（`yeban_ui_test_port::tree` 与 `yeban_ui_mcp::tree` 里 `focus`
//! 命中 0），因此 `tests/a11y_screen_reader.rs` 只能按「寻址面」近似。本切片沿用
//! `tests/undo_tree_focus_trap.rs` 的手法：把 Slint **自己的**焦点状态镜像成 MainWindow 的
//! 只读数（`export-wav-focused-field` 从四格的 `has-focus` **推导**，⛔ 不是「我以为焦点在
//! 哪」的写态）。
//!
//! ⚠ 读数取的是**四个 `TextInput` 的** `has-focus`，⛔ 不是 trap 自己的：焦点项就是那四格
//! 本身（`window.rs` 的 `move_focus` 只在**目标项**上 `publish_focus_item`），因此 trap 的
//! `has-focus` 在轮转期间恒为 `false` —— 拿它当读数会得到一条永远为假的判据。
//!
//! ## 已知边界（如实登记，⛔ 不假装覆盖）
//!
//! * 关闭时复位目标是**快捷键层**，不是规范那句「触发控件」：触发方
//!   `module-export-wav-button` 今天不是可聚焦控件（连 `FocusScope` 都没有）⇒ 要先给它
//!   建焦点身份，那是另一票。
//! * 「导出」/「关闭」/ 限制器开关三格：本票落地时它们**没有** `FocusScope`（只有
//!   `Rectangle` + `TouchArea`），故当时如实登记为"仍只能鼠标与无障碍动作"。
//!   ⚠ `D-UI-20` 已把这三格各接上一枚零视觉 `FocusScope` 并把轮转从四格扩到八格 ⇒
//!   那句边界**已失效**；本票的 ④⑤ 因此改成读八格读数（同一提交里一起改，否则红）。
//!   这三格**按下去做什么**不在本票判据里，判据是 `tests/export_dialog_button_focus.rs`。
//! * `Cmd/Ctrl` 组合（保存 / 撤销 / `D-UI-18` 的搜索聚焦）在弹窗打开期间**同样到不了宿主**。
//!   ⚠ 这是模态持键盘的**目的**而非副作用，但成因有两条，⛔ 别记成一条：trap 只处理
//!   `Tab` / `Backtab` / `Esc`，其余裸键 `reject` 后冒到本组件之外就没人接（`key-handler`
//!   不是它的祖先）；而带修饰位的**可打印**字符是 `TextInput` 自己吃掉的
//!   （`i-slint-core-1.18.1/items/text.rs`：`accept_text_input` 只按 `InputType` 筛，
//!   ⛔ 不看修饰位）⇒ `Cmd+S` 在框里会落进一个 `s`。
//! * `Shift+Tab` **无法**从平台事件通道注入（`slint::platform::WindowEvent::KeyPressed`
//!   没有修饰位字段，见 `live_surface.rs` 里 `dispatch_key_chord` 的整段说明），而
//!   `dispatch_key_chord` 虽然带修饰位却**绕过焦点** ⇒ 与焦点有关的键只能走平台通道。
//!   真键盘上的 `Shift+Tab` 由 Slint 投成 Backtab `\u{19}`（`window.rs` 的内建那段两种
//!   形态都认），本判据注入的正是那个码位；trap 里按修饰位判的 `Tab`+`shift` 分支
//!   ⛔ 本判据覆盖不到（注入不出来），如实登记。
//!
//! ## 判别力（改动 ⇒ 红；缺行就说明那条断言本来就是空的）
//!
//! | 若改成 | 本判据 |
//! | :--- | :--- |
//! | trap 的 `Tab` 分支不调 `focus-next-control()`（改回 `reject`） | ④ 红（读数停在 0；而 `Tab` 会撞上画布切视图） |
//! | 轮转表把 `7 → 0` 写成 `7 → 7` | ④ 第 8 步红（绕不回第一格） |
//! | 把 `focus-on-tab-navigation` 打开 | ④ 的「左栏搜索框从未被选中」红 —— Slint 内建轮转走的是**整窗**链 |
//! | 打开弹窗时不回焦（删掉 `invoke_focus_export_wav_dialog()`） | ② 红（读数停在 `-1`），③④⑤⑥ 随之全红 |
//! | 关闭弹窗时不复位 | ⑦ 红（`active-tool` 停在 3） |
//! | trap 的 `Esc` 分支不调 `close()` | ⑦ 红 —— ⚠ **只**红在这一条：`key-handler` 那条 `Action::Cancel` 路今天仍在，但已不是这条注入的必经之路（`tests/export_dialog_modal_closure.rs` 的 ③ 同口径更正） |
//! | 给某个框新增一个 `accessible-id` | ⑧ 红（12 ⇒ 13），金帧与树基准随之要重录 —— 本票刻意不碰 |
//! | 在别处再抄一份轮转（第 10 个 `.focus()`） | ⑧ 红（`.focus()` 计数 9 ⇒ 10） |

// assertion_bounds_audit 禁止新增 .all( 站点 ⇒ 此处用 fold,
// clippy::unnecessary_fold 的常规建议与本仓守卫冲突, 具名豁免。
#![allow(unused, clippy::unnecessary_fold)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code（`-D warnings` 会判红）。
#[allow(dead_code)]
mod live;

use live::{LiveWiringOptions, build_live_ui_with};
use slint::ComponentHandle;
use std::rc::Rc;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::port::{KeyCode, Permission};
use yeban_ui_test_port::render::report_line;

/// 会话打开时刻（与 `tests/export_menu_repro_167.rs` 同一个夹具常量）。
const NOW_MS: u64 = 1_700_000_000_000;

fn build() -> live::LiveUi {
    let project = yeban_model::samples::filled_project();
    // 活窗口 + 真撤销端口（⛔ 不能用 `..Default::default()` 的 `undo: None`：
    // 秒数播种（`audible_end_seconds`）与导出（`on_export_wav` 读工程）都要它；
    // 生产 GUI 在 `main.rs` 传的也是 `Some(undo_port)`）。
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<export-dialog>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            undo: Some(Rc::clone(&undo)),
            ..LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 执行面")
}

/// 经真宿主回调打开导出弹窗（播种秒数 + 取焦）。
fn open_dialog(ui: &yeban_app::ui::MainWindow) {
    ui.invoke_open_export_wav();
    assert!(ui.get_export_wav_open(), "宿主入口必须打开导出弹窗");
    assert_eq!(
        ui.get_export_wav_focused_control(),
        0,
        "打开落 0 格（路径框）"
    );
    assert_eq!(ui.get_export_wav_focused_field(), 0, "打开落路径框");
    assert!(!ui.get_shortcut_layer_focused(), "打开后快捷键层失去焦点");
}

fn press(live: &mut live::LiveUi, key: KeyCode) {
    live.dispatch_key_press(key)
        .expect("按键注入必须落到活窗口（Interactive 档）");
}

/// 按一次 `Tab`，断言轮转落到期望的那一格（控制格 + 四框读数一起钉）。
fn tab_once(live: &mut live::LiveUi, ui: &yeban_app::ui::MainWindow, control: i32, field: i32) {
    press(live, KeyCode::Tab);
    assert_eq!(
        ui.get_export_wav_focused_control(),
        control,
        "Tab 后控制格必须是 {control}"
    );
    assert_eq!(
        ui.get_export_wav_focused_field(),
        field,
        "Tab 后四框读数必须是 {field}"
    );
}

/// 按一次 Backtab（真键盘上的 `Shift+Tab` 以 `\u{19}` 到达）。
fn backtab_once(live: &mut live::LiveUi, ui: &yeban_app::ui::MainWindow, control: i32, field: i32) {
    press(live, KeyCode::Character('\u{19}'));
    assert_eq!(
        ui.get_export_wav_focused_control(),
        control,
        "Backtab 后控制格必须是 {control}"
    );
    assert_eq!(
        ui.get_export_wav_focused_field(),
        field,
        "Backtab 后四框读数必须是 {field}"
    );
}

/// 某个 id 在不在这一帧的运行时树里（每次读之前先跳一跳心跳，见
/// `tests/export_dialog_modal_closure.rs` 的 `ids`）。
fn in_tree(live: &mut live::LiveUi, id: &str) -> bool {
    live.pump_meters();
    live.tree_snapshot().find_by_id(id).is_some()
}

/// 只保留**代码行**（删掉 `//` / `///` 开头的行）再交给计数。
fn code_only(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn typing_goes_into_the_path_field_and_tab_rotates_all_eight_slots() {
    let mut live = build();
    let ui = live.ui().clone_strong();

    // ① 起点：弹窗关着 ⇒ 读数 -1、快捷键层持焦、弹窗不在树里。
    assert!(!ui.get_export_wav_open(), "起点：导出弹窗必须关着");
    assert_eq!(
        ui.get_export_wav_focused_control(),
        -1,
        "起点：控制格读数 -1"
    );
    assert_eq!(ui.get_export_wav_focused_field(), -1, "起点：四框读数 -1");
    assert!(ui.get_shortcut_layer_focused(), "起点：快捷键层持焦");
    assert!(
        !in_tree(&mut live, "export-wav-dialog"),
        "起点：export-wav-dialog 不在运行时树里"
    );
    // 先把工具拨到 3：⑥⑦ 需要一个与 1 不同的值。
    press(&mut live, KeyCode::Character('3'));
    assert_eq!(
        ui.get_active_tool(),
        3,
        "按 3 ⇒ 工具 3（弹窗外快捷键层活着）"
    );

    // ② 打开；四格都在树里。
    open_dialog(&ui);
    for id in FIELD_IDS {
        assert!(in_tree(&mut live, id), "打开后 {id} 必须在运行时树里");
    }
    // 宿主在打开时把路径播种成缺省值（`ModulePaths::export_wav`）；本测试要证的是
    // "敲的字进框"，先经属性把框清成空串（测试准备，⛔ 不是被测行为）。
    ui.set_export_wav_path("".into());
    report_line("key-capture: opened field=0 control=0");

    // ③ 打三个可打印字符 ⇒ 进路径框（两向绑定的证据），工具一位不动。
    for ch in ['p', 'a', 'd'] {
        press(&mut live, KeyCode::Character(ch));
    }
    assert_eq!(
        ui.get_export_wav_path().as_str(),
        "pad",
        "打 pad ⇒ 路径框里是 pad"
    );
    assert_eq!(ui.get_active_tool(), 3, "打字期间画布工具一位不动");

    // ④ Tab ×8 绕回第一格（控制格 + 四框读数一起钉）。
    for (control, field) in [
        (1, -1),
        (2, 1),
        (3, 2),
        (4, 3),
        (5, -1),
        (6, -1),
        (7, -1),
        (0, 0),
    ] {
        tab_once(&mut live, &ui, control, field);
    }
    assert_eq!(ui.get_active_tool(), 3, "轮转期间画布工具一位不动");
    assert!(
        !ui.get_sidebar_search_focused(),
        "轮转期间左栏搜索框从未被选中（焦点没有漏出模态）"
    );

    // ⑤ Backtab ×8 反向绕回第一格。
    for (control, field) in [
        (7, -1),
        (6, -1),
        (5, -1),
        (4, 3),
        (3, 2),
        (2, 1),
        (1, -1),
        (0, 0),
    ] {
        backtab_once(&mut live, &ui, control, field);
    }

    // ⑥ 弹窗持键盘时按 1 ⇒ 落进当前那一格（路径框），工具不动。
    press(&mut live, KeyCode::Character('1'));
    assert_eq!(
        ui.get_export_wav_path().as_str(),
        "pad1",
        "弹窗开着按 1 ⇒ 它落进路径框（pad → pad1）"
    );
    assert_eq!(ui.get_active_tool(), 3, "弹窗开着按 1 ⇒ 工具不动");
    report_line("key-capture: typing/tab/backtab/capture ok");
}

#[test]
fn escape_closes_and_the_shortcut_layer_gets_keys_back() {
    let mut live = build();
    let ui = live.ui().clone_strong();

    press(&mut live, KeyCode::Character('3'));
    assert_eq!(ui.get_active_tool(), 3, "按 3 ⇒ 工具 3");
    open_dialog(&ui);
    // 宿主在打开时把路径播种成缺省值；先经属性把框清成空串（测试准备）。
    ui.set_export_wav_path("".into());

    // ⑥ 弹窗开着按 1 ⇒ 进框，不到宿主。
    press(&mut live, KeyCode::Character('1'));
    assert_eq!(
        ui.get_export_wav_path().as_str(),
        "1",
        "弹窗开着按 1 ⇒ 它落进路径框"
    );
    assert_eq!(ui.get_active_tool(), 3, "弹窗开着按 1 ⇒ 工具不动");

    // ⑦ Esc ⇒ 关闭 + 焦点复位到快捷键层 ⇒ 随后 1 又到达宿主。
    press(&mut live, KeyCode::Escape);
    assert!(!ui.get_export_wav_open(), "Esc ⇒ 弹窗关");
    assert_eq!(
        ui.get_export_wav_focused_control(),
        -1,
        "关闭后控制格读数 -1"
    );
    assert_eq!(ui.get_export_wav_focused_field(), -1, "关闭后四框读数 -1");
    assert!(ui.get_shortcut_layer_focused(), "关闭后焦点复位快捷键层");
    press(&mut live, KeyCode::Character('1'));
    assert_eq!(
        ui.get_active_tool(),
        1,
        "Esc 之后按 1 ⇒ 工具 1（键盘回到宿主）"
    );
    // 弹窗本就关着时再按 Esc：什么都不改（不抢焦点、不改工具）。
    press(&mut live, KeyCode::Escape);
    assert!(!ui.get_export_wav_open(), "第二次 Esc ⇒ 弹窗仍关着");
    assert_eq!(ui.get_active_tool(), 1, "第二次 Esc ⇒ 工具仍是 1");
    assert!(
        ui.get_shortcut_layer_focused(),
        "第二次 Esc ⇒ 焦点仍在快捷键层"
    );
    report_line("key-capture: esc-restore ok");
}

/// 弹窗里那四格的语义 ID（顺序 = 轮转顺序 = `focused-field` 的 0..3）。
const FIELD_IDS: [&str; 4] = [
    "export-wav-path-input",
    "export-wav-sample-rate-input",
    "export-wav-bit-depth-input",
    "export-wav-seconds-input",
];

/// ⑧ **出处纪律**：这一票不许留下第二份焦点状态，也不许动寻址面。
///
/// 读的是**自己的源码**（`env!("CARGO_MANIFEST_DIR")`），手法取自
/// `tests/d_ui_11_serve_header_gestures.rs`。四条各钉一件事：
/// * 焦点读数是**推导**（从四格 `has-focus` 算出）⇒ 全仓没有任何一处**写**它；
/// * 轮转只定义一次 ⇒ 弹窗文件里 `.focus()` 的调用点恒为 9（首格 1 + 表 8）；
/// * 模态内只有**一个**焦点作用域（`export-trap`）⇒ ⛔ 没有第二个键盘入口；
/// * 没有新增可寻址控件 ⇒ `accessible-id:` 恒为 12 ⇒ 默认帧、5 张金帧、运行时树的寻址面
///   一位不改（`T-FIX-57` 起该钉值有意改为 11：新增可见「浏览…」按钮一枚，注册表与
///   `DIALOG_IDS` 同提交跟上；实测行加入后改为 12；本条纪律的本意「新增必须同提交登记」不变。）
///   都一位不改（`tests/export_dialog_modal_closure.rs` 的 `DIALOG_IDS` 同口径）。
#[test]
fn the_focus_readout_is_derived_and_no_new_control_becomes_addressable() {
    let dialog = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/dialogs/export_wav_dialog.slint"),
    )
    .expect("读 export_wav_dialog.slint");
    let code = code_only(&dialog);
    assert_eq!(
        code.matches(".focus()").count(),
        9,
        "轮转只定义一次：.focus() 调用点恒 9（首格 1 + 表 8）"
    );
    assert_eq!(
        code.matches("accessible-id:").count(),
        12,
        "没有新增可寻址控件：accessible-id: 恒 12"
    );
    assert_eq!(
        code.matches("FocusScope {").count(),
        5,
        "模态内只有一个焦点作用域 export-trap + 四格按钮容器"
    );
    // 焦点读数是推导：全仓没有任何一处写它们。
    let app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    for name in ["focused-control", "focused-field"] {
        assert_eq!(writes_of(name, &code), 0, "弹窗里没有写 {name}（它是推导）");
        assert_eq!(
            writes_of(name, &app),
            0,
            "app.slint 里没有写 {name}（它是推导）"
        );
        assert_eq!(
            writes_of(name, &host),
            0,
            "host.rs 里没有写 {name}（它是推导）"
        );
    }
}

/// 数「以单个 `=` 写这个标识符」的出现次数（`==` / `>=` / `<=` / `!=` 都不算写）。
///
/// ⚠ 必须带**边界**判断：`export-wav-focused-field` 里含着 `focused-field`，
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
