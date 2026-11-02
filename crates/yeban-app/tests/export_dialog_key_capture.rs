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
//! 现在 `export-trap` 是那四格的**祖先** `FocusScope`（⛔ 不能像时光机那样放成兄弟覆盖层：
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
//! ④ `Tab` ×4 ⇒ `1→2→3→0`（**闭环 + 绕回**），期间左栏搜索框**从未**被选中，
//!    画布语义的切视图 ⛔ 没被触发；
//! ⑤ Backtab（`Shift+Tab` 的到达形态）⇒ 反向绕回，`0→3→2→1→0`；
//! ⑥ 弹窗持键盘时按 `1` ⇒ 它落进**当前那一格**，`active-tool` ⛔ 不变 ——
//!    与 ⑦ 的最后一个是**同一个键、两种落点**，那才是「捕获」二字的判别力所在；
//! ⑦ `Esc` ⇒ 关闭 + 焦点**复位**到快捷键层 ⇒ 随后 `1` 又到达宿主（`active-tool` = 1）；
//! ⑧ 出处纪律（另一条 `#[test]`）：轮转只定义一次、焦点读数是**推导**而非第三份状态、
//!    没有新增可寻址控件 ⇒ 默认帧与 `tests/golden/linux/` 的 5 张基准一位不改。
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
//! * 「导出」/「关闭」/ 限制器开关三格**仍然只能鼠标与无障碍动作**（无 `FocusScope` 的
//!   `Rectangle` + `TouchArea`）⇒ 键盘走完四格之后按 `Return` **不会**触发导出。那是
//!   `D-UI-20`，本票不顺手猜它的命中测试。
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
//! | trap 的 `Tab` 分支不调 `focus-next-field()`（改回 `reject`） | ④ 红（读数停在 0；而 `Tab` 会撞上画布切视图） |
//! | 轮转表把 `3 → 0` 写成 `3 → 3` | ④ 第 4 步红（绕不回第一格） |
//! | 把 `focus-on-tab-navigation` 打开 | ④ 的「左栏搜索框从未被选中」红 —— Slint 内建轮转走的是**整窗**链 |
//! | 打开弹窗时不回焦（删掉 `invoke_focus_export_wav_dialog()`） | ② 红（读数停在 `-1`），③④⑤⑥ 随之全红 |
//! | 关闭弹窗时不复位 | ⑦ 红（`active-tool` 停在 3） |
//! | trap 的 `Esc` 分支不调 `close()` | ⑦ 红 —— ⚠ **只**红在这一条：`key-handler` 那条 `Action::Cancel` 路今天仍在，但已不是这条注入的必经之路（`tests/export_dialog_modal_closure.rs` 的 ③ 同口径更正） |
//! | 给某个框新增一个 `accessible-id` | ⑧ 红（10 ⇒ 11），金帧与树基准随之要重录 —— 本票刻意不碰 |
//! | 在别处再抄一份轮转（第 10 个 `.focus()`） | ⑧ 红（`.focus()` 计数 9 ⇒ 10） |

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code（`-D warnings` 会判红）。
#[allow(dead_code)]
mod live;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_ui_test_port::port::{KeyCode, Permission};
use yeban_ui_test_port::render::report_line;

/// 弹窗里那四格的语义 ID（顺序 = 轮转顺序 = `focused-field` 的 0..3）。
const FIELD_IDS: [&str; 4] = [
    "export-wav-path-input",
    "export-wav-sample-rate-input",
    "export-wav-bit-depth-input",
    "export-wav-seconds-input",
];

#[test]
fn the_export_dialog_holds_the_keyboard_and_rotates_its_four_fields() {
    let project = yeban_model::samples::filled_project();
    // ⛔ 不接撤销端口：本判据读的是 `active-tool` / `arrangement-view` 这两个**界面**属性，
    // 而 `wire_keys(…, None, …)` 那一条对工具 / 视图 / 走带照常生效（只有撤销族如实
    // `reject`）—— 见 `live_surface.rs` 的装配注释。
    let mut ui = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            console_tab: 0,
            save_path: None,
            engine_quanta: 0,
            undo: None,
            proposal_source: yeban_app::proposals::ProposalSource::none(),
        },
    )
    .expect("真实界面 + Tier-1 执行面");
    ui.pump_meters();

    // ---- ① 起点：弹窗关着 ⇒ 键盘归快捷键层 ----
    let mut trace: Vec<String> = vec![format!(
        "起点 field={} key-layer={}",
        ui.ui().get_export_wav_focused_field(),
        ui.ui().get_shortcut_layer_focused()
    )];
    assert_eq!(
        ui.ui().get_export_wav_focused_field(),
        -1,
        "① 起点：弹窗没开 ⇒ 焦点不许在四格里的任何一格"
    );
    assert!(
        ui.ui().get_shortcut_layer_focused(),
        "① 起点：快捷键层必须持焦（`forward-focus: key-handler` 的机械证据）"
    );
    assert!(
        !ui.tree_snapshot().contains("export-wav-dialog"),
        "① 起点：对话框不许在运行时树里"
    );

    // ---- ② 前置：先把工具挪到 `3`，再给路径框一个**已知的**播种值 ----
    // `1` 与 `3` 都是工具矩阵的行 ⇒ 「同一个键两种落点」要靠这个起点才有判别力。
    ui.dispatch_key_press(KeyCode::Character('3'))
        .expect("② 前置：`3` 必须落到真实窗口（Interactive 档）");
    assert_eq!(
        ui.ui().get_active_tool(),
        3,
        "② 前置：`3` 到不了宿主的话，⑥ 的「到不了宿主」就是空的"
    );
    // 非空 ⇒ 宿主那条播种分支（`if get_export_wav_path().trim().is_empty()`）不会覆写它。
    // ⛔ 不用真路径：本判据从不点「导出」，这串字符只是**被键盘编辑的对象**。
    // 刻意不含 `x` ⇒ ③ 可以用「摘掉一个 `x` 逐字回到播种值」这条无歧义的判法。
    ui.ui()
        .set_export_wav_path("seed-without-that-letter.wav".into());

    // ---- ② 打开：走**宿主的入口**（与用户点 `module-export-wav-button` 同一条链）----
    ui.ui().invoke_open_export_wav();
    ui.pump_meters();
    assert!(
        ui.tree_snapshot().contains("export-wav-dialog"),
        "② 打开之后对话框必须进运行时树"
    );
    for id in FIELD_IDS {
        assert!(ui.tree_snapshot().contains(id), "② `{id}` 必须在树里");
    }
    assert_eq!(
        ui.ui().get_export_wav_focused_field(),
        0,
        "② 打开 ⇒ 焦点必须落在**第一格**（路径），这是四格能被键盘填的前提"
    );
    assert!(
        !ui.ui().get_shortcut_layer_focused(),
        "② 弹窗持键盘时快捷键层 ⛔ 不得同时持焦 —— 焦点是**唯一**的"
    );
    assert!(
        !ui.ui().get_sidebar_search_focused(),
        "② 打开那一步不许把焦点顺手交给左栏搜索框"
    );
    trace.push(format!(
        "打开后 field={} key-layer={}",
        ui.ui().get_export_wav_focused_field(),
        ui.ui().get_shortcut_layer_focused()
    ));

    let view_before = ui.ui().get_arrangement_view();

    // ---- ③ 可打印字符进路径框（两向绑定的证据），画布一位不动 ----
    let path_before = ui.ui().get_export_wav_path().to_string();
    ui.dispatch_key_press(KeyCode::Character('x'))
        .expect("③ `x` 必须落到真实窗口");
    let path_after = ui.ui().get_export_wav_path().to_string();
    assert!(
        path_after.contains('x'),
        "③ 打进去的字符必须出现在 `export-wav-path` 里（`text <=> root.path` 的反向那半）: 前 `{path_before}` 后 `{path_after}`"
    );
    assert_eq!(
        path_after.chars().count(),
        path_before.chars().count() + 1,
        "③ 只多了**这一个**字符: 前 {} 后 {}",
        path_before.chars().count(),
        path_after.chars().count()
    );
    let mut stripped = path_after.clone();
    if let Some(pos) = stripped.find('x') {
        stripped.remove(pos);
    }
    assert_eq!(
        stripped, path_before,
        "③ 除那一个字符外，路径必须与播种值逐字相同（⛔ 不是整框被覆写了一遍）"
    );
    assert_eq!(
        ui.ui().get_active_tool(),
        3,
        "③ `x` ⛔ 不许被当成画布语义的键"
    );
    assert_eq!(
        ui.ui().get_arrangement_view(),
        view_before,
        "③ 同一个字符也不许切视图"
    );
    trace.push(format!("`x` 后 path={path_after}"));

    // ---- ④ `Tab` 闭环轮转：0→1→2→3→0，且焦点从不逃出模态 ----
    for (step, expected) in [1_i32, 2, 3, 0].iter().enumerate() {
        ui.dispatch_key_press(KeyCode::Tab)
            .expect("④ `Tab` 必须落到真实窗口");
        let got = ui.ui().get_export_wav_focused_field();
        assert_eq!(
            got,
            *expected,
            "④ 第 {} 次 `Tab` 之后焦点必须在第 {} 格（读到 {got}）",
            step + 1,
            expected
        );
        assert!(
            !ui.ui().get_sidebar_search_focused(),
            "④ 焦点 ⛔ 不许逃出模态落到左栏搜索框（Slint 的内建轮转走的是**整窗**链，那正是关掉 `focus-on-tab-navigation` 的理由）"
        );
        assert_eq!(
            ui.ui().get_arrangement_view(),
            view_before,
            "④ 模态内 `Tab` ⛔ 不得触发画布语义的切视图（`accept` 把这一键吞在模态里）"
        );
        assert_eq!(ui.ui().get_active_tool(), 3, "④ 轮转期间工具不许被动");
    }
    trace.push("Tab×4 ⇒ 1→2→3→0（闭环 + 绕回）".to_owned());

    // ---- ⑤ Backtab 反向：0→3→2→1→0 ----
    for (step, expected) in [3_i32, 2, 1, 0].iter().enumerate() {
        // `Shift+Tab` 以 Backtab `\u{19}` 到达（`i-slint-common` 的 `key_codes::Backtab`）；
        // 平台事件通道带不了修饰位 ⇒ 注入那个码位就是它在真键盘上的到达形态。
        ui.dispatch_key_press(KeyCode::Character('\u{19}'))
            .expect("⑤ Backtab 必须落到真实窗口");
        let got = ui.ui().get_export_wav_focused_field();
        assert_eq!(
            got,
            *expected,
            "⑤ 第 {} 次 Backtab 之后焦点必须在第 {} 格（读到 {got}）",
            step + 1,
            expected
        );
        assert_eq!(ui.ui().get_active_tool(), 3, "⑤ 反向轮转也不许动工具");
    }
    trace.push("Backtab×4 ⇒ 3→2→1→0（反向绕回）".to_owned());

    // ---- ⑥ 弹窗持键盘时，`1` 归文本框、⛔ 不归 DAW ----
    let len_before = ui.ui().get_export_wav_path().chars().count();
    ui.dispatch_key_press(KeyCode::Character('1'))
        .expect("⑥ `1` 必须落到真实窗口");
    assert_eq!(
        ui.ui().get_active_tool(),
        3,
        "⑥ 弹窗开着时按 `1` ⛔ 不得换工具 —— 这一条就是「键盘归模态」本身"
    );
    assert_eq!(
        ui.ui().get_export_wav_path().chars().count(),
        len_before + 1,
        "⑥ 同一个 `1` 必须落进当前那一格（此刻是路径框）"
    );
    trace.push(format!(
        "`1` 后 tool={} field={}",
        ui.ui().get_active_tool(),
        ui.ui().get_export_wav_focused_field()
    ));

    // ---- ⑦ `Esc` 收尾 + 焦点复位 + 同一个键重新到达宿主 ----
    ui.dispatch_key_press(KeyCode::Escape)
        .expect("⑦ `Esc` 必须落到真实窗口");
    ui.pump_meters();
    assert!(
        !ui.ui().get_export_wav_open(),
        "⑦ `Esc` 必须关掉弹窗（走的是 trap 里那条 `close()`，与 ✕ 按钮、无障碍默认动作同一实现）"
    );
    assert!(
        !ui.tree_snapshot().contains("export-wav-dialog"),
        "⑦ 关闭之后对话框必须离开运行时树"
    );
    assert_eq!(
        ui.ui().get_export_wav_focused_field(),
        -1,
        "⑦ 关闭之后焦点 ⛔ 不许还留在任何一格（否则下一个键会打进一个看不见的框）"
    );
    assert!(
        ui.ui().get_shortcut_layer_focused(),
        "⑦ 关闭之后焦点必须**复位**到快捷键层"
    );
    ui.dispatch_key_press(KeyCode::Character('1'))
        .expect("⑦ `1` 必须落到真实窗口");
    assert_eq!(
        ui.ui().get_active_tool(),
        1,
        "⑦ 复位之后 `1` 必须重新到达宿主（与 ⑥ 是**同一个键、两种落点**）"
    );
    trace.push(format!(
        "`Esc` 后 field={} key-layer={}；`1` 后 tool={}",
        ui.ui().get_export_wav_focused_field(),
        ui.ui().get_shortcut_layer_focused(),
        ui.ui().get_active_tool()
    ));

    report_line(&format!("[export-key] {}", trace.join(" ⇒ ")));
}

/// ⑧ **出处纪律**：这一票不许留下第二份焦点状态，也不许动寻址面。
///
/// 读的是**自己的源码**（`env!("CARGO_MANIFEST_DIR")`），手法取自
/// `tests/d_ui_11_serve_header_gestures.rs`。四条各钉一件事：
/// * 焦点读数是**推导**（从四格 `has-focus` 算出）⇒ 全仓没有任何一处**写**它；
/// * 轮转只定义一次 ⇒ 弹窗文件里 `.focus()` 的调用点恒为 9（首格 1 + 前进 4 + 后退 4）；
/// * 模态内只有**一个**焦点作用域（`export-trap`）⇒ ⛔ 没有第二个键盘入口；
/// * 没有新增可寻址控件 ⇒ `accessible-id:` 恒为 10 ⇒ 默认帧、5 张金帧、运行时树的寻址面
///   都一位不改（`tests/export_dialog_modal_closure.rs` 的 `DIALOG_IDS` 同口径）。
#[test]
fn the_focus_readout_is_derived_and_no_new_control_becomes_addressable() {
    let read = |rel: &str| -> String {
        let full = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        std::fs::read_to_string(&full)
            .unwrap_or_else(|error| panic!("读 `{}` 失败: {error}", full.display()))
    };
    let dialog = read("ui/dialogs/export_wav_dialog.slint");
    let app = read("ui/app.slint");
    let host = read("src/host.rs");

    // ---- 焦点读数没有任何写者 ----
    let writes = writes_of("focused-field", &dialog)
        + writes_of("focused-field", &app)
        + writes_of("export-wav-focused-field", &app);
    assert_eq!(
        writes, 0,
        "⑧ 焦点读数必须是**推导**（⛔ 不许有第三份「我以为焦点在哪」的写态）：实测写出 {writes} 处"
    );
    assert_eq!(
        host.matches("export_wav_focused_field").count(),
        0,
        "⑧ 宿主⛔ 既不读也不写这份读数（Rust 侧一出现 snake_case 名字就是有人把它当状态用了）"
    );
    assert!(
        dialog.contains("out property <int> focused-field:"),
        "⑧ `focused-field` 必须是 `out`（弹窗只报数，宿主不许往里写）"
    );
    assert!(
        app.contains("out property <int> export-wav-focused-field: export_dialog.focused-field;"),
        "⑧ MainWindow 那一条必须是**纯转发**，⛔ 不重新推导一遍（两处推导迟早分叉）"
    );

    // ---- 轮转与焦点入口都只有一份 ----
    assert_eq!(
        dialog.matches(".focus()").count(),
        9,
        "⑧ 取焦调用点必须恰好 9 处（`focus-first-field` 1 + `focus-next-field` 4 + `focus-previous-field` 4）；\
         多出来的那一处意味着轮转表在别处又被抄了一遍"
    );
    assert_eq!(
        dialog.matches(":= FocusScope").count(),
        1,
        "⑧ 模态内只能有**一个**焦点作用域（`export-trap`）；第二个就是第二个键盘入口"
    );
    for code in [r#""\u{9}""#, r#""\u{19}""#, r#""\u{1b}""#] {
        assert!(
            dialog.contains(code),
            "⑧ trap 必须认得 {code} 这一码位（Tab / Backtab / Esc）"
        );
    }

    // ---- 寻址面一位不改 ----
    assert_eq!(
        dialog.matches("accessible-id:").count(),
        10,
        "⑧ 弹窗里的可寻址控件数必须是 10（本票⛔ 不新增控件 ⇒ 默认帧与金帧基准不受影响）"
    );

    // ---- 开关与焦点同处，且回焦的调用者只有那两处 ----
    assert_eq!(
        host.matches("invoke_focus_export_wav_dialog").count(),
        1,
        "⑧ 取焦必须只有打开弹窗那一处（与 `set_export_wav_open(true)` 同一个闭包 ⇒ 开关与焦点不可能分叉）"
    );
    assert_eq!(
        host.matches("invoke_focus_shortcut_layer").count(),
        2,
        "⑧ 复位到快捷键层的调用者必须恰好两处：时光机关闭 + 导出弹窗关闭"
    );
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
