//! T-FIX-289：`T-FIX-219` 的导出对话框守卫推广到其余五只模态 —— 六只模态开着时，
//! 数字键 `1`–`5` 不得漏到 DAW 画布工具切换上。
//!
//! ## 漏键路径（六只模态同一条，与 219 的回执 §3 同根因）
//!
//! 按钮格的 `FocusScope` 只接 `Return` / `Space`、其余一律 `reject`；陷阱
//! `FocusScope` 只接 `Tab` / `Backtab` / `Esc`（鼓机另接 `Up` / `Down` 换曲风）、
//! 其余一律 `reject`。数字键于是从两层 `reject` 里冒泡出来，落到对话框的祖先
//! `app.slint` 的 `key-handler`，那里 `root.trigger-shortcut(event.text, event)`
//! 把它当成画布工具切换吃掉 —— 陷阱里那句"模态开着时 DAW 收不到键"只对快捷键层
//! 那个 `FocusScope` 成立，⛔ 不是对 `key-handler` 这个祖先。
//!
//! ## 落点：路线 A（集中一处）
//!
//! `key-handler` 顶部只有一条守卫：六只模态的 `*-open` 标志或起来，任一开着且
//! 无修饰键时，把 219 那张键表（`Tab` / `Backtab`、`Space`、`r` / `c` / `1`–`5` /
//! `b`、`i` / `h` / `t` / `d`、`F3` / `F4` / `F7` / `F8` / `F9`）一律 `reject`。
//! 选 A 不选 B（逐模态六条 `if` 臂）的理由：六只模态的漏键机制完全相同，
//! 逐只加臂会把同一张键表抄六遍 —— 下一只模态、下一个新快捷键都要改六处，
//! 正是 219 只修一只、其余五只继续漏的根源。维护约定写在守卫注释里：
//! ① 新快捷键加进来时同步进键表；② 新模态加 `*-open` 标志时同步进模态表。
//! `T-FIX-217` 的音源库 `Tab` 守卫原地不动（它是"放行给原生遍历"的另一语义，
//! 本守卫的 `Tab` 臂与它同效，两处并存无冲突）。
//!
//! ## 判据（一条链，六只模态各一条；另加一张表纪律）
//!
//! 每只：① 焦点在快捷键层时按 `3` ⇒ 工具 3（弹窗外快捷键层活着）；
//! ② 打开（`set_<x>_open(true)` + `invoke_focus_*_dialog()`，与
//! `tests/modal_focus_entry.rs` 同一顺序）⇒ 焦点落首格；
//! ③ `Tab` 走到一只按钮格（导出 ✕ / 音源库 ✕ / 缺音源 ✕ / 模板保存钮 /
//! 人性化"应用人性化" / 鼓机"生成"）；④ 在按钮格上按 `1`–`5` ⇒ 工具一位不动；
//! ⑤ `Esc` 关弹窗 ⇒ 焦点复位回快捷键层；⑥ 再按 `1` ⇒ 工具切到 1（守卫只在
//! 模态开着时生效，关了就恢复）。
//!
//! 另加一则扩表模拟（T-FIX-343）：往键表里插约 900 字节新键行，窗口量法仍绿
//! —— 旧 3000 字节固定窗口在同样插入下会红（标记到 reject 超 3000 字节）。
//!
//! ## 判别力（改动 ⇒ 红；缺行就说明那条断言本来就是空的）
//!
//! | 若改成 | 本判据 |
//! | :--- | :--- |
//! | 守卫整条删掉（或基线无守卫） | 六条行为判据全红（`1` 切工具）—— 本文件先在基线跑过红，见回执 |
//! | 模态表里漏掉某一只的 `*-open` | 对应那只的判据红（其余五只仍绿） |
//! | 键表里漏掉某个数字 | 对应数字那一步红 |
//! | 守卫改成连 `Esc` 也拦 | ⑤ 红（`Esc` 关不掉弹窗） |
//! | 守卫改成有修饰键也拦 | 本文件测不到（修饰位注不进来）⇒ 如实登记在边界里 |
//! | 守卫放到 `trigger-shortcut` 之后 | 六条全红（快捷键已先被消费） |
//! | 键表再扩约 900 字节 | 旧 3000 字节窗口红（reject 被推出窗口）、新窗口绿（本票新增的扩表模拟） |
//!
//! ## 已知边界（如实登记，⛔ 不假装覆盖）
//!
//! * 文本框里的打字不受守卫影响：文本框先于冒泡接住按键（`T-FIX-219` 的
//!   `key_capture` 已证导出路径框 `pad` / `pad1`），本文件不重复测其余五只的
//!   文本框打字 —— 守卫在冒泡链的末端，只看得到"无人接"的键。
//! * `Shift+Tab` 反向轮转注不进来（平台通道带不了修饰位），与
//!   `modal_focus_entry.rs` 同款边界；本文件只用正向 `Tab` 走到按钮格。
//! * `Esc` 不进守卫是 219 定下的口径：陷阱先接住，焦点在弹窗外时宿主
//!   `Action::Cancel` 仍是兜底；本文件 ⑤ 只钉"陷阱的 `Esc` 关弹窗"这一半。
//! * 键表与 219 同款，⛔ 不是 `input.rs` 第 6 步的全部裸键：`m` / `s` / `x` /
//!   `e` / `p` / `z` / `[` / `]` / `=` / `-` / 方向键 / `Return` / `Delete` /
//!   `F5` / `F6` 也不在 219 的表里 —— 它们在模态按钮格上同样会漏（同一条
//!   冒泡路径），但简报定的是"219 同款键表"，扩表是另一票（见回执 §5 的残留登记）。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的装配入口 ⇒ 其余项在本测试目标里是 dead_code。
#[allow(dead_code)]
mod live;

use live::{LiveWiringOptions, build_live_ui_with};
use slint::ComponentHandle;
use yeban_ui_test_port::port::{KeyCode, Permission};
use yeban_ui_test_port::render::report_line;

fn build() -> live::LiveUi {
    let project = yeban_model::samples::filled_project();
    build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            ..LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 执行面")
}

fn press(live: &mut live::LiveUi, key: KeyCode) {
    live.dispatch_key_press(key)
        .expect("按键注入必须落到活窗口（Interactive 档）");
}

/// 起点：焦点在快捷键层，按 `3` 把工具拨到 3（与 `1` 不同，且证弹窗外快捷键活着）。
fn arm_tool_3(live: &mut live::LiveUi, ui: &yeban_app::ui::MainWindow) {
    assert!(
        ui.get_shortcut_layer_focused(),
        "起点：快捷键层必须持有键盘"
    );
    press(live, KeyCode::Character('3'));
    assert_eq!(
        ui.get_active_tool(),
        3,
        "按 3 ⇒ 工具 3（弹窗外快捷键层活着）"
    );
}

/// 按钮格上按 `1`–`5`：工具一位不动。
fn digits_hold(live: &mut live::LiveUi, ui: &yeban_app::ui::MainWindow, modal: &str) {
    for digit in ['1', '2', '3', '4', '5'] {
        press(live, KeyCode::Character(digit));
        assert_eq!(
            ui.get_active_tool(),
            3,
            "{modal}：按钮格上按 {digit} ⇒ 工具不动"
        );
    }
}

/// `Esc` 关弹窗 ⇒ 焦点复位回快捷键层；再按 `1` ⇒ 工具切到 1（守卫已松开）。
fn esc_closes_and_shortcuts_restore(
    live: &mut live::LiveUi,
    ui: &yeban_app::ui::MainWindow,
    modal: &str,
    is_open: impl Fn(&yeban_app::ui::MainWindow) -> bool,
    focused_control: impl Fn(&yeban_app::ui::MainWindow) -> i32,
) {
    press(live, KeyCode::Escape);
    assert!(!is_open(ui), "{modal}：Esc 必须关掉弹窗");
    assert_eq!(focused_control(ui), -1, "{modal}：关闭后焦点必须不在模态内");
    assert!(
        ui.get_shortcut_layer_focused(),
        "{modal}：关闭后焦点必须复位回快捷键层"
    );
    press(live, KeyCode::Character('1'));
    assert_eq!(
        ui.get_active_tool(),
        1,
        "{modal}：关弹窗后按 1 ⇒ 工具切到 1（守卫已松开）"
    );
}

/// 导出 WAV：按钮格是 ✕（第 7 格）。
#[test]
fn export_wav_modal_holds_digits_on_its_close_button() {
    let mut live = build();
    let ui = live.ui().clone_strong();
    arm_tool_3(&mut live, &ui);

    ui.set_export_wav_open(true);
    ui.invoke_focus_export_wav_dialog();
    assert!(ui.get_export_wav_open(), "导出：弹窗必须打开");
    assert_eq!(ui.get_export_wav_focused_control(), 0, "导出：入口落 0 格");
    assert!(!ui.get_shortcut_layer_focused(), "导出：画布层必须不再持焦");
    for expected in [1, 2, 3, 4, 5, 6, 7] {
        press(&mut live, KeyCode::Tab);
        assert_eq!(
            ui.get_export_wav_focused_control(),
            expected,
            "导出：Tab 轮转读数必须是 {expected}"
        );
    }
    digits_hold(&mut live, &ui, "导出");
    esc_closes_and_shortcuts_restore(
        &mut live,
        &ui,
        "导出",
        |ui| ui.get_export_wav_open(),
        |ui| ui.get_export_wav_focused_control(),
    );
    report_line("modal-guard export-wav: digits 1-5 held, esc-closed, 1 restores tool 1");
}

/// 音源管理：空数据时只有两格（搜索框 0 → ✕ 700），按钮格是 ✕。
#[test]
fn sound_library_modal_holds_digits_on_its_close_button() {
    let mut live = build();
    let ui = live.ui().clone_strong();
    arm_tool_3(&mut live, &ui);

    ui.set_sound_library_open(true);
    ui.invoke_focus_sound_library_dialog();
    assert!(ui.get_sound_library_open(), "音源管理：弹窗必须打开");
    assert_eq!(
        ui.get_sound_library_focused_control(),
        0,
        "音源管理：入口落 0 格（搜索框）"
    );
    assert!(
        !ui.get_shortcut_layer_focused(),
        "音源管理：画布层必须不再持焦"
    );
    press(&mut live, KeyCode::Tab);
    assert_eq!(
        ui.get_sound_library_focused_control(),
        700,
        "音源管理：Tab 一次 ⇒ ✕（700 格）"
    );
    digits_hold(&mut live, &ui, "音源管理");
    esc_closes_and_shortcuts_restore(
        &mut live,
        &ui,
        "音源管理",
        |ui| ui.get_sound_library_open(),
        |ui| ui.get_sound_library_focused_control(),
    );
    report_line("modal-guard sound-library: digits 1-5 held, esc-closed, 1 restores tool 1");
}

/// 缺音源救援：入口就是 ✕（第 0 格），不用 `Tab`。
/// `close` 回调活窗口装配没接（与 `modal_focus_entry.rs` 同款）：按宿主的同一写法装一份。
#[test]
fn missing_sounds_modal_holds_digits_on_its_close_button() {
    let mut live = build();
    let ui = live.ui().clone_strong();
    {
        let weak = ui.as_weak();
        ui.on_missing_close(move || {
            if let Some(ui) = weak.upgrade() {
                let was_open = ui.get_missing_sounds_open();
                ui.set_missing_sounds_open(false);
                if was_open {
                    ui.invoke_focus_shortcut_layer();
                }
            }
        });
    }
    arm_tool_3(&mut live, &ui);

    ui.set_missing_sounds_open(true);
    ui.invoke_focus_missing_sounds_dialog();
    assert!(ui.get_missing_sounds_open(), "缺音源：弹窗必须打开");
    assert_eq!(
        ui.get_missing_sounds_focused_control(),
        0,
        "缺音源：入口落 0 格（✕）"
    );
    assert!(
        !ui.get_shortcut_layer_focused(),
        "缺音源：画布层必须不再持焦"
    );
    digits_hold(&mut live, &ui, "缺音源");
    esc_closes_and_shortcuts_restore(
        &mut live,
        &ui,
        "缺音源",
        |ui| ui.get_missing_sounds_open(),
        |ui| ui.get_missing_sounds_focused_control(),
    );
    report_line("modal-guard missing-sounds: digits 1-5 held, esc-closed, 1 restores tool 1");
}

/// 另存为模板：按钮格是保存钮（第 3 格）；`Tab` ×3：曲风 → 子曲风 → 模板名 → 保存钮。
#[test]
fn template_save_modal_holds_digits_on_its_save_button() {
    let mut live = build();
    let ui = live.ui().clone_strong();
    arm_tool_3(&mut live, &ui);

    ui.set_template_save_open(true);
    ui.invoke_focus_template_save_dialog();
    assert!(ui.get_template_save_open(), "模板：弹窗必须打开");
    assert_eq!(
        ui.get_template_save_focused_control(),
        0,
        "模板：入口落 0 格（曲风）"
    );
    assert!(!ui.get_shortcut_layer_focused(), "模板：画布层必须不再持焦");
    for expected in [1, 2, 3] {
        press(&mut live, KeyCode::Tab);
        assert_eq!(
            ui.get_template_save_focused_control(),
            expected,
            "模板：Tab 轮转读数必须是 {expected}"
        );
    }
    digits_hold(&mut live, &ui, "模板");
    esc_closes_and_shortcuts_restore(
        &mut live,
        &ui,
        "模板",
        |ui| ui.get_template_save_open(),
        |ui| ui.get_template_save_focused_control(),
    );
    report_line("modal-guard template-save: digits 1-5 held, esc-closed, 1 restores tool 1");
}

/// 人性化：按钮格是"应用人性化"（第 5 格）；`Tab` ×5：时值 → 力度 → 种子 → 网格 → 强度 → 应用。
#[test]
fn humanize_modal_holds_digits_on_its_apply_button() {
    let mut live = build();
    let ui = live.ui().clone_strong();
    arm_tool_3(&mut live, &ui);

    ui.set_humanize_open(true);
    ui.invoke_focus_humanize_dialog();
    assert!(ui.get_humanize_open(), "人性化：弹窗必须打开");
    assert_eq!(
        ui.get_humanize_focused_control(),
        0,
        "人性化：入口落 0 格（时值抖动量）"
    );
    assert!(
        !ui.get_shortcut_layer_focused(),
        "人性化：画布层必须不再持焦"
    );
    for expected in [1, 2, 3, 4, 5] {
        press(&mut live, KeyCode::Tab);
        assert_eq!(
            ui.get_humanize_focused_control(),
            expected,
            "人性化：Tab 轮转读数必须是 {expected}"
        );
    }
    digits_hold(&mut live, &ui, "人性化");
    esc_closes_and_shortcuts_restore(
        &mut live,
        &ui,
        "人性化",
        |ui| ui.get_humanize_open(),
        |ui| ui.get_humanize_focused_control(),
    );
    report_line("modal-guard humanize: digits 1-5 held, esc-closed, 1 restores tool 1");
}

/// 鼓机：按钮格是"生成"（第 6 格）；`Tab` ×6：复杂度 → 强度 → 摇摆 → 小节 → 种子 → 填充 → 生成。
#[test]
fn drum_gen_modal_holds_digits_on_its_apply_button() {
    let mut live = build();
    let ui = live.ui().clone_strong();
    arm_tool_3(&mut live, &ui);

    ui.set_drum_gen_open(true);
    ui.invoke_focus_drum_gen_dialog();
    assert!(ui.get_drum_gen_open(), "鼓机：弹窗必须打开");
    assert_eq!(
        ui.get_drum_gen_focused_control(),
        0,
        "鼓机：入口落 0 格（复杂度）"
    );
    assert!(!ui.get_shortcut_layer_focused(), "鼓机：画布层必须不再持焦");
    for expected in [1, 2, 3, 4, 5, 6] {
        press(&mut live, KeyCode::Tab);
        assert_eq!(
            ui.get_drum_gen_focused_control(),
            expected,
            "鼓机：Tab 轮转读数必须是 {expected}"
        );
    }
    digits_hold(&mut live, &ui, "鼓机");
    esc_closes_and_shortcuts_restore(
        &mut live,
        &ui,
        "鼓机",
        |ui| ui.get_drum_gen_open(),
        |ui| ui.get_drum_gen_focused_control(),
    );
    report_line("modal-guard drum-gen: digits 1-5 held, esc-closed, 1 restores tool 1");
}

fn read_app_slint() -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"))
        .expect("读 app.slint")
}

/// 守卫窗口（T-FIX-343）：从 `T-FIX-289` 标记起，量到守卫臂的 `return reject;`
/// 行尾再加固定余量。
///
/// 旧量法是固定 3000 字节：键表一扩就过期——T-FIX-332 扩表后标记到 reject 已
/// 约 2175 字节（余量约 825），再加约 900 字节键行就把 reject 推出窗口，判据
/// 红的原因不是行为坏了，而是窗口常数不够大。新量法跟表长走：表再长，窗口
/// 终点也跟着 reject 往后挪；扩表模拟（本文件末尾）先在旧量法下红过，见回执。
///
/// 余量取 512 字节：盖住守卫臂的收尾 `}` 与 reject 行后可能追加的几行注释；
/// 不到守卫体（约 2.2 KB）的四分之一，窗口仍收在守卫邻域内。余量那段尾巴
/// （`}`、`trigger-shortcut` 调用、组字注释）经核实不含六模态、五个数字、
/// reject 任一被断言串，断言仍由守卫体本身满足，不是尾巴凑出来的。窗口不是
/// 无界的：它止于 reject 行尾 + 余量；标记全文件恰好一处（本文件另一断言），
/// 别处同名标记混不进来。
///
/// reject 行缺失 ⇒ 这里直接 panic（判据红），不是静默放过。
fn guard_window(app: &str) -> &str {
    let marker = "T-FIX-289";
    let start = app.find(marker).expect("T-FIX-289 标记");
    let reject_end = start
        + app[start..]
            .find("return reject;")
            .expect("守卫必须以 reject 放行")
        + "return reject;".len();
    // 字节窗口须落在字符边界上：app.slint 是 UTF-8，窗口终点可能切在汉字中间
    // 而直接切片会 panic。终点向下取整到边界。
    let mut end = (reject_end + 512).min(app.len());
    while !app.is_char_boundary(end) {
        end -= 1;
    }
    &app[start..end]
}

/// 守卫窗口内必须有的内容（六模态 + 五个数字 + reject），表纪律与扩表模拟
/// 共用——扩表模拟测的是同一套断言，不是另一套。窗口本身就锚在 reject 行尾，
/// 下面那条 reject 断言是意图复述；reject 缺失时 `guard_window` 先 panic，
/// 同样是红。
fn assert_guard_contents(guard: &str) {
    for flag in [
        "export-wav-open",
        "sound-library-open",
        "missing-sounds-open",
        "template-save-open",
        "humanize-open",
        "drum-gen-open",
    ] {
        assert!(guard.contains(flag), "守卫的模态表里必须有 {flag}");
    }
    for digit in ['1', '2', '3', '4', '5'] {
        assert!(
            guard.contains(&format!("\"{digit}\"")),
            "守卫的键表里必须有数字 {digit}"
        );
    }
    assert!(
        guard.contains("return reject;"),
        "守卫必须以 reject 放行（⛔ 不是 accept 吃掉）"
    );
}

/// 表纪律（⛔ 不开窗口）：`T-FIX-289` 守卫恰好一处，模态表里六只齐全、键表里
/// 五个数字齐全，且守卫在 `trigger-shortcut` 之前（之后就拦不住了）。
#[test]
fn the_guard_lists_all_six_modals_and_all_five_digits_before_trigger_shortcut() {
    let app = read_app_slint();
    let marker = "T-FIX-289";
    assert_eq!(
        app.matches(marker).count(),
        1,
        "T-FIX-289 守卫注释必须恰好一处（集中一处，⛔ 不得多处各抄一份）"
    );
    let guard = guard_window(&app);
    assert_guard_contents(guard);
    let guard_at = app.find(marker).expect("T-FIX-289 标记");
    let trigger_at = app
        .find("root.trigger-shortcut(event.text, event);")
        .expect("trigger-shortcut 调用");
    assert!(
        guard_at < trigger_at,
        "守卫必须在 trigger-shortcut 之前（之后就拦不住了）"
    );
}

/// 扩表模拟（T-FIX-343）：往键表里插约 900 字节新键行后，窗口量法仍绿。
/// 旧 3000 字节固定窗口在同样插入下会红（标记到 reject 约 3065 字节）——本
/// 判据就是那条旧窗口的回归网：它先在旧量法下红过，见回执。
#[test]
fn the_guard_window_survives_a_key_table_expansion() {
    let app = read_app_slint();
    // 插入点：键表里的 `Key.F9` 行之后、reject 之前——与真实扩表（T-FIX-305
    // 在同表追加 F5/F6、Return、方向键等）同位置、同写法；插入行不含任何
    // 被断言的字符串（六模态、五个数字、reject、trigger-shortcut），断言
    // 只能由原守卫体满足，不能由插入行凑出来。
    let anchor = "|| event.text == Key.F9\n";
    let at = app.find(anchor).expect("键表锚点 Key.F9 行") + anchor.len();
    let extra = concat!(
        "                    || event.text == Key.F10 || event.text == Key.F11\n",
        "                    || event.text == Key.F12\n",
        "                    || event.text == \"j\" || event.text == \"J\"\n",
        "                    || event.text == \"k\" || event.text == \"K\"\n",
        "                    || event.text == \"l\" || event.text == \"L\"\n",
        "                    || event.text == \";\" || event.text == \":\"\n",
        "                    || event.text == \",\" || event.text == \".\"\n",
        "                    || event.text == \"/\" || event.text == \"?\"\n",
        "                    || event.text == Key.Home || event.text == Key.End\n",
        "                    || event.text == Key.PageUp || event.text == Key.PageDown\n",
        "                    || event.text == Key.Insert\n",
        "                    || event.text == \"u\" || event.text == \"U\"\n",
        "                    || event.text == \"o\" || event.text == \"O\"\n",
        "                    || event.text == \"n\" || event.text == \"N\"\n",
        "                    || event.text == \"g\" || event.text == \"G\"\n",
    );
    // 模拟必须够强：插入量 ≥900 字节，且标记到 reject 超过旧窗口 3000 字节——
    // 否则本判据测不到旧窗口的毛病（插入被谁悄悄缩短就先红在这里）。
    assert!(extra.len() >= 900, "模拟扩表量必须 ≥900 字节");
    let mut expanded = String::with_capacity(app.len() + extra.len());
    expanded.push_str(&app[..at]);
    expanded.push_str(extra);
    expanded.push_str(&app[at..]);
    let marker = "T-FIX-289";
    let start = expanded.find(marker).expect("T-FIX-289 标记");
    let reject_at = expanded[start..]
        .find("return reject;")
        .expect("守卫必须以 reject 放行")
        + start;
    assert!(
        reject_at - start > 3000,
        "模拟扩表必须把 reject 推出旧 3000 字节窗口（当前 {} 字节），否则测不到旧窗口的毛病",
        reject_at - start
    );
    // 同一套窗口量法 + 同一套断言，在扩表后的文本上必须全绿。
    assert_eq!(
        expanded.matches(marker).count(),
        1,
        "T-FIX-289 守卫注释必须恰好一处"
    );
    let guard = guard_window(&expanded);
    assert_guard_contents(guard);
    let trigger_at = expanded
        .find("root.trigger-shortcut(event.text, event);")
        .expect("trigger-shortcut 调用");
    assert!(
        start < trigger_at,
        "扩表后守卫必须仍在 trigger-shortcut 之前"
    );
}
