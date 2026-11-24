//! T-FIX-305：模态守卫键表扩到 `input.rs::resolve_logical` 实际消费的全部裸键。
//!
//! # 背景
//!
//! T-FIX-219 只给导出弹窗加了守卫，T-FIX-289 把它推广成六模态集中守卫——但两单
//! 的键表都只覆盖 219 当时那张表（Tab / Backtab、Space、r / c / 1–5 / b、i / h /
//! t / d、F3 / F4 / F7 / F8 / F9）。`trigger-shortcut` 经宿主 `resolve_logical`
//! 消费的裸键不止这些：表外键在任一模态打开、焦点落在按钮格上时，仍沿同一条冒泡
//! 路径（按钮 `FocusScope` reject → 陷阱 `reject` → `key-handler` 祖先 →
//! `trigger-shortcut`）漏到 DAW 并被执行。其中 Delete 会删选区、方向键会微调选区，
//! 属于会改数据的误触，比 219 的工具切换更严重。
//!
//! 本单把守卫键表扩到 `resolve_logical` 第 1 步（F5 / F6，早于合成态判定）与第 6
//! 步消费的全部裸键，并把 219 的导出专属臂与 289 的集中守卫合并为 `app.slint`
//! 里的一条集中守卫（T-FIX-289 的路线 A；289 补丁若后合入，删掉它那一臂即可——
//! 见回执的合入顺序说明）。`Esc` 刻意不在表里（陷阱先接住并 `accept`；焦点在
//! 弹窗外时宿主的 `Action::Cancel` 臂仍是兜底）。
//!
//! # 逐键可观测读数
//!
//! | 键 | 读数（模态开着时须全程不变） |
//! | --- | --- |
//! | Delete / Backspace | 选区计数（`selected-note-count`）与工程里那颗音符仍在 |
//! | Left / Right | 该音符 `tick`（±240 网格步进） |
//! | Up / Down | 该音符 `pitch`（±1 半音） |
//! | Return | `playhead-ticks`（GoToBeginning 会归零） |
//! | m / s | 全轨 `(mute, solo)` 快照 |
//! | x / e / p | `console-tab`（x→1，e/p→0） |
//! | z | `roll-ticks-per-pixel`（ZoomToSelection 会重算） |
//! | = / - | `roll-ticks-per-pixel`（60→45 / 45→60） |
//! | F5 / F6 | `arrangement-view`（F5→false，F6→true） |
//! | [ / ] | 今天无读数（见下边界说明）——表里留着是为接线落地那天，纪律判据钉住 |
//!
//! 注入走真实平台通道（`dispatch_key_press`，与 T-FIX-219 / 289 同款）：
//! Delete 用 `Character('\u{7f}')`、方向键用 `Character('\u{f700}'..'\u{f703}')`、
//! F5 / F6 用 `Character('\u{f708}' / '\u{f709}')`、Backspace / Return 用
//! `KeyCode::Backspace / KeyCode::Return`——五种注入方式已在 `probe_305_injection`
//! 里逐一证实能到达宿主（Delete 删音符、Left 左移 240、F5 切视图、Return 归零
//! 走带、Backspace 删音符；其余键与它们走同一条 `type_char` / `key_press` 路径）。
//! `invoke_key_action` 不可用：它绕过 `key-handler`、直捣宿主回调，测不到守卫。
//!
//! # 已知边界（如实登记）
//!
//! * **Return 的活窗红对照受焦点拓扑限制**：六只模态里每个可聚焦格都自己吃掉
//!   Return（按钮格触发回调、文本框触发 `accepted`），所以今天没有一格能把
//!   Return 递到 `key-handler`——`return_does_not_reset_playhead_in_modal` 在基线
//!   上也是绿的，它钉的是端到端行为（"模态里按 Return 不归零走带"），不是守卫本条。
//!   守卫里的 `Key.Return` 由表项纪律判据（`guard_table_covers_every_bare_key`）
//!   红对照：删掉它 → 纪律判据变红。表里留着它是对的：它是 `resolve_logical`
//!   第 6 步的裸键，将来任何一只带滑块（不吃 Return）的模态都会把它递上来。
//! * **鼓机 Up / Down 是陷阱吃的**：鼓机陷阱把 Up / Down 接去换曲风（`accept`），
//!   所以鼓机判据里 Up / Down 断言的是"选区不动"（陷阱、守卫任一层拦住都成立），
//!   不单独算守卫的功劳；Left / Right 仍由守卫拦。
//! * **`[` / `]` 今天没有任何可观测读数**：`resolve_logical` 把它们解成
//!   `AuditionMain` / `AuditionProposal`，但 `apply_action` 开头的
//!   `action_has_implementation` 对这两条返回 `false`（试听能力尚未接线，
//!   T-FIX-259）——按键臂里写气泡的那段今天到不了。于是 `[` / `]` 在模态内外
//!   按下去都是静默无操作：模态里的"气泡不动"断言与关窗后的断言钉的都是端到端
//!   行为，在基线上也是绿的，**不是**守卫的红对照。它们仍进表、仍由纪律判据钉住，
//!   理由与 Return 相同：表与 `resolve_logical` 的裸键集逐一对应是守卫的不变量；
//!   接线一旦落地（只差按键臂到 `audition::start` 的接线），`[` 立刻会发声，
//!   那时没有这一条它就会在模态里漏。
//! * 判据用真实 `UndoPort` 搭建（`undo: Some(port)`）：`undo: None` 时 Delete /
//!   Nudge / m / s / z 的宿主臂会拒绝执行（见 T-FIX-219 回执 §4），那样判据
//!   在基线上就是假绿。
//! * 每只模态的判据末尾都有"关窗后放行"段：Esc 关窗 → 同一把键确实执行（Delete
//!   删音符、m 改静音……）—— 证明这套读数本身不哑，也证明守卫只在模态开着时生效。

#![allow(unused)]

#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;

use live::{LiveWiringOptions, build_live_ui_with};
use slint::{ComponentHandle, Model, SharedString};
use std::collections::BTreeMap;
use std::rc::Rc;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::samples::filled_project;
use yeban_ui_test_port::port::{KeyCode, Permission};
use yeban_ui_test_port::render::report_line;

const NOW_MS: u64 = 1_700_000_000_000;

// ---------------------------------------------------------------------------
// 搭建
// ---------------------------------------------------------------------------

fn build() -> (live::LiveUi, Rc<UndoPort>) {
    let project = filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<modal-guard-keytable>",
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("撤销会话必须能打开"),
    ));
    let ui = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            undo: Some(Rc::clone(&port)),
            ..LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 执行面");
    (ui, port)
}

/// 真实平台通道（`dispatch_key_press`）：按钮格的 reject 能冒到 `key-handler`。
fn press(live: &mut live::LiveUi, key: KeyCode) {
    live.dispatch_key_press(key);
}

/// 与 `tests/roll_edit_tools.rs` 的 `note_readings` 同款：
///
/// 规范名 → (start_tick, pitch, duration_ticks, velocity)。
fn note_readings(project: &yeban_model::YebanProjectV1) -> BTreeMap<String, (u64, u8, u64, u8)> {
    let mut out = BTreeMap::new();
    for entry in project.clip_pool.values() {
        let Some(map) = entry.content.notes() else {
            continue;
        };
        for (id, note) in map {
            out.insert(
                id.to_canonical_string(),
                (
                    note.start_tick,
                    note.pitch,
                    note.duration_ticks,
                    note.velocity,
                ),
            );
        }
    }
    out
}

/// 挑一颗 Left/Right/Up/Down 都有余量的音符（tick ≥ 480，pitch 2..=125）。
fn pick_note(project: &yeban_model::YebanProjectV1) -> (String, (u64, u8, u64, u8)) {
    note_readings(project)
        .into_iter()
        .find(|(_, reading)| reading.0 >= 480 && (2..=125).contains(&reading.1))
        .expect("夹具里必须有 tick ≥ 480 且 pitch 在 2..=125 的音符")
}

/// 把一批音符身份写进选区的**唯一事实源** `selected-ulids`（并同步计数）——
///
/// 与 `tests/roll_edit_tools.rs` 的 `select_ulids` 同款。
fn select_ulids(window: &yeban_app::ui::MainWindow, ids: &[String]) {
    window.set_selected_ulids(slint::ModelRc::new(slint::VecModel::from(
        ids.iter()
            .map(|id| slint::SharedString::from(id.as_str()))
            .collect::<Vec<_>>(),
    )));
    window.set_selected_note_count(i32::try_from(ids.len()).unwrap_or(i32::MAX));
}

fn track_switches(project: &yeban_model::YebanProjectV1) -> Vec<(bool, bool)> {
    project
        .tracks
        .values()
        .map(|track| (track.mute, track.solo))
        .collect()
}

fn arm_tool_3(live: &mut live::LiveUi) {
    let ui = live.ui().clone_strong();
    ui.set_active_tool(3);
    report_line("modal-guard-keytable: tool armed to 3");
}

// ---------------------------------------------------------------------------
// 六只模态的打开链（与 T-FIX-289 同款：焦点落按钮格上）
// ---------------------------------------------------------------------------

fn open_export(live: &mut live::LiveUi) {
    let ui = live.ui().clone_strong();
    ui.set_export_wav_open(true);
    ui.invoke_focus_export_wav_dialog();
    for _ in 0..7 {
        press(live, KeyCode::Character('\u{9}'));
    }
    assert_eq!(
        ui.get_export_wav_focused_control(),
        7,
        "导出弹窗焦点须落在 ✕（7）"
    );
}

fn open_sound_library(live: &mut live::LiveUi) {
    let ui = live.ui().clone_strong();
    ui.set_sound_library_open(true);
    ui.invoke_focus_sound_library_dialog();
    press(live, KeyCode::Character('\u{9}'));
    assert_eq!(
        ui.get_sound_library_focused_control(),
        700,
        "音色库焦点须落在 ✕（700）"
    );
}

fn open_missing_sounds(live: &mut live::LiveUi) {
    let ui = live.ui().clone_strong();
    // 活窗口装配没接这条回调（宿主 `host.rs` 的 `on_missing_close` 写法），按同一
    // 写法装一份，否则 Esc / ✕ 关不了窗。
    ui.on_missing_close({
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let was_open = ui.get_missing_sounds_open();
            ui.set_missing_sounds_open(false);
            if was_open {
                ui.invoke_focus_shortcut_layer();
            }
        }
    });
    ui.set_missing_sounds_open(true);
    ui.invoke_focus_missing_sounds_dialog();
    assert_eq!(
        ui.get_missing_sounds_focused_control(),
        0,
        "缺失音色弹窗焦点须落在 ✕（0）"
    );
}

fn open_template_save(live: &mut live::LiveUi) {
    let ui = live.ui().clone_strong();
    ui.set_template_save_open(true);
    ui.invoke_focus_template_save_dialog();
    for _ in 0..3 {
        press(live, KeyCode::Character('\u{9}'));
    }
    assert_eq!(
        ui.get_template_save_focused_control(),
        3,
        "另存模板焦点须落在保存（3）"
    );
}

fn open_humanize(live: &mut live::LiveUi) {
    let ui = live.ui().clone_strong();
    ui.set_humanize_open(true);
    ui.invoke_focus_humanize_dialog();
    for _ in 0..5 {
        press(live, KeyCode::Character('\u{9}'));
    }
    assert_eq!(
        ui.get_humanize_focused_control(),
        5,
        "人性化焦点须落在应用（5）"
    );
}

fn open_drum_gen(live: &mut live::LiveUi) {
    let ui = live.ui().clone_strong();
    ui.set_drum_gen_open(true);
    ui.invoke_focus_drum_gen_dialog();
    for _ in 0..6 {
        press(live, KeyCode::Character('\u{9}'));
    }
    assert_eq!(
        ui.get_drum_gen_focused_control(),
        6,
        "鼓机焦点须落在应用（6）"
    );
}

// ---------------------------------------------------------------------------
// 模态开着时：逐键按，读数全程不变（Return 除外——见文件头边界说明）
// ---------------------------------------------------------------------------

/// `seed` 之后调用：模态已开、焦点在按钮格上；逐键按扩表里的键并断言读数不动。
/// `note` 是种子选区的音符（`tick`/`pitch` 读数锚点）。
fn assert_keys_held(
    live: &mut live::LiveUi,
    port: &Rc<UndoPort>,
    note: &str,
    start_tick: u64,
    start_pitch: u8,
) {
    let ui = live.ui().clone_strong();
    assert_eq!(ui.get_active_tool(), 3, "开窗前须已把工具武装到 3");
    assert_eq!(ui.get_playhead_ticks(), 960, "走带须已播种到 960");
    assert_eq!(ui.get_selected_note_count(), 1, "选区须已播种 1 颗音符");
    assert!(ui.get_arrangement_view(), "视图须已播种到编排");
    assert_eq!(ui.get_console_tab(), 0, "控制台须已播种到编辑器（0）");
    assert_eq!(ui.get_roll_ticks_per_pixel(), 60, "缩放须已播种到 60");
    assert_eq!(ui.get_toast_message().as_str(), "", "气泡须为空");

    // 数字键：集中守卫的"旧表"部分仍在（T-FIX-289 的判据只验了数字键，这里复验一键）。
    press(live, KeyCode::Character('1'));
    assert_eq!(ui.get_active_tool(), 3, "模态开着时按 1 工具须留在 3");

    // Delete / Backspace：选区计数与工程里的音符都不动。
    press(live, KeyCode::Character('\u{7f}'));
    assert_eq!(
        ui.get_selected_note_count(),
        1,
        "模态开着时 Delete 须不删选区"
    );
    assert!(
        note_readings(&port.try_project().expect("端口必须握着工程")).contains_key(note),
        "模态开着时 Delete 须不删工程里的音符"
    );
    press(live, KeyCode::Backspace);
    assert_eq!(
        ui.get_selected_note_count(),
        1,
        "模态开着时 Backspace 须不删选区"
    );
    assert!(
        note_readings(&port.try_project().expect("端口必须握着工程")).contains_key(note),
        "模态开着时 Backspace 须不删工程里的音符"
    );

    // 方向键：tick / pitch 都不动。
    press(live, KeyCode::Character('\u{f702}'));
    press(live, KeyCode::Character('\u{f703}'));
    press(live, KeyCode::Character('\u{f700}'));
    press(live, KeyCode::Character('\u{f701}'));
    let after = note_readings(&port.try_project().expect("端口必须握着工程"));
    let (tick, pitch, _, _) = after[note];
    assert_eq!(tick, start_tick, "模态开着时方向键须不微调 tick");
    assert_eq!(pitch, start_pitch, "模态开着时方向键须不微调 pitch");

    // m / s：全轨静音/独奏快照不动。
    let switches_before = track_switches(&port.try_project().expect("端口必须握着工程"));
    press(live, KeyCode::Character('m'));
    assert_eq!(
        track_switches(&port.try_project().expect("端口必须握着工程")),
        switches_before,
        "模态开着时 m 须不改静音"
    );
    press(live, KeyCode::Character('s'));
    assert_eq!(
        track_switches(&port.try_project().expect("端口必须握着工程")),
        switches_before,
        "模态开着时 s 须不改独奏"
    );

    // x / e / p：控制台页签不动（x→1、e/p→0，各播种一次再按）。
    press(live, KeyCode::Character('x'));
    assert_eq!(ui.get_console_tab(), 0, "模态开着时 x 须不切到混音台");
    ui.set_console_tab(1);
    press(live, KeyCode::Character('e'));
    assert_eq!(ui.get_console_tab(), 1, "模态开着时 e 须不切回编辑器");
    press(live, KeyCode::Character('p'));
    assert_eq!(ui.get_console_tab(), 1, "模态开着时 p 须不切回编辑器");

    // z / = / -：缩放不动（tpp 播种在 60；= 的步进值是 45、- 是 80、z 的拟合值由
    // 选区跨度/视口算——三者都不在 60 上，见关窗后放行段的实测）。
    press(live, KeyCode::Character('z'));
    assert_eq!(
        ui.get_roll_ticks_per_pixel(),
        60,
        "模态开着时 z 须不重算缩放"
    );
    press(live, KeyCode::Character('='));
    assert_eq!(ui.get_roll_ticks_per_pixel(), 60, "模态开着时 = 须不放大");
    press(live, KeyCode::Character('-'));
    assert_eq!(ui.get_roll_ticks_per_pixel(), 60, "模态开着时 - 须不缩小");

    // F5 / F6：视图不动（各播种一次再按）。
    press(live, KeyCode::Character('\u{f708}'));
    assert!(
        ui.get_arrangement_view(),
        "模态开着时 F5 须不切到 Session 视图"
    );
    ui.set_arrangement_view(false);
    press(live, KeyCode::Character('\u{f709}'));
    assert!(!ui.get_arrangement_view(), "模态开着时 F6 须不切到编排视图");

    // [ / ]：气泡不动。
    press(live, KeyCode::Character('['));
    assert_eq!(
        ui.get_toast_message().as_str(),
        "",
        "模态开着时 [ 须不弹试听拒绝"
    );
    press(live, KeyCode::Character(']'));
    assert_eq!(
        ui.get_toast_message().as_str(),
        "",
        "模态开着时 ] 须不弹试听拒绝"
    );
}

// ---------------------------------------------------------------------------
// 关窗后：同一把键确实执行（读数本身不哑 + 守卫只在模态开着时生效）
// ---------------------------------------------------------------------------

fn assert_keys_fire_after_close(
    live: &mut live::LiveUi,
    port: &Rc<UndoPort>,
    note: &str,
    start_tick: u64,
    start_pitch: u8,
) {
    let ui = live.ui().clone_strong();
    // Esc 关窗，焦点回快捷键层。
    press(live, KeyCode::Escape);
    assert!(!ui.get_export_wav_open(), "Esc 后导出弹窗须关");
    assert!(!ui.get_sound_library_open(), "Esc 后音色库须关");
    assert!(!ui.get_missing_sounds_open(), "Esc 后缺失音色弹窗须关");
    assert!(!ui.get_template_save_open(), "Esc 后另存模板须关");
    assert!(!ui.get_humanize_open(), "Esc 后人性化须关");
    assert!(!ui.get_drum_gen_open(), "Esc 后鼓机须关");
    assert!(ui.get_shortcut_layer_focused(), "Esc 后焦点须回快捷键层");

    // 数字键恢复。
    press(live, KeyCode::Character('1'));
    assert_eq!(ui.get_active_tool(), 1, "关窗后按 1 工具须切到 1");
    ui.set_active_tool(3);

    // 方向键：Left -240，Right +240 回来；Up +1，Down -1 回来。
    press(live, KeyCode::Character('\u{f702}'));
    let after = note_readings(&port.try_project().expect("端口必须握着工程"));
    assert_eq!(after[note].0, start_tick - 240, "关窗后 Left 须左移 240");
    press(live, KeyCode::Character('\u{f703}'));
    let after = note_readings(&port.try_project().expect("端口必须握着工程"));
    assert_eq!(after[note].0, start_tick, "关窗后 Right 须右移回来");
    press(live, KeyCode::Character('\u{f700}'));
    let after = note_readings(&port.try_project().expect("端口必须握着工程"));
    assert_eq!(after[note].1, start_pitch + 1, "关窗后 Up 须升半音");
    press(live, KeyCode::Character('\u{f701}'));
    let after = note_readings(&port.try_project().expect("端口必须握着工程"));
    assert_eq!(after[note].1, start_pitch, "关窗后 Down 须降回来");

    // m / s：改一次、再改回来。
    let switches_before = track_switches(&port.try_project().expect("端口必须握着工程"));
    press(live, KeyCode::Character('m'));
    assert_ne!(
        track_switches(&port.try_project().expect("端口必须握着工程")),
        switches_before,
        "关窗后 m 须改静音（读数不哑）"
    );
    press(live, KeyCode::Character('m'));
    assert_eq!(
        track_switches(&port.try_project().expect("端口必须握着工程")),
        switches_before,
        "关窗后再按 m 须改回来"
    );
    press(live, KeyCode::Character('s'));
    assert_ne!(
        track_switches(&port.try_project().expect("端口必须握着工程")),
        switches_before,
        "关窗后 s 须改独奏（读数不哑）"
    );
    press(live, KeyCode::Character('s'));
    assert_eq!(
        track_switches(&port.try_project().expect("端口必须握着工程")),
        switches_before,
        "关窗后再按 s 须改回来"
    );

    // x / e / p。
    ui.set_console_tab(0);
    press(live, KeyCode::Character('x'));
    assert_eq!(ui.get_console_tab(), 1, "关窗后 x 须切到混音台");
    press(live, KeyCode::Character('e'));
    assert_eq!(ui.get_console_tab(), 0, "关窗后 e 须切回编辑器");
    ui.set_console_tab(1);
    press(live, KeyCode::Character('p'));
    assert_eq!(ui.get_console_tab(), 0, "关窗后 p 须切回编辑器");

    // = / -（从 60 出发：= → 45，- → 60）与 z（拟合值 ≠ 60）。
    ui.set_roll_ticks_per_pixel(60);
    press(live, KeyCode::Character('='));
    assert_eq!(ui.get_roll_ticks_per_pixel(), 45, "关窗后 = 须放大到 45");
    press(live, KeyCode::Character('-'));
    assert_eq!(ui.get_roll_ticks_per_pixel(), 60, "关窗后 - 须缩回 60");
    press(live, KeyCode::Character('z'));
    assert_ne!(
        ui.get_roll_ticks_per_pixel(),
        60,
        "关窗后 z 须重算缩放（读数不哑）"
    );

    // F5 / F6。
    ui.set_arrangement_view(true);
    press(live, KeyCode::Character('\u{f708}'));
    assert!(!ui.get_arrangement_view(), "关窗后 F5 须切到 Session 视图");
    press(live, KeyCode::Character('\u{f709}'));
    assert!(ui.get_arrangement_view(), "关窗后 F6 须切回编排视图");

    // [：今天静默无操作（`action_has_implementation` 挡在气泡臂之前，见文件头）——
    // 断言气泡仍空，只钉"按键无副作用"这一端到端行为。
    press(live, KeyCode::Character('['));
    assert_eq!(
        ui.get_toast_message().as_str(),
        "",
        "关窗后 [ 今天仍无气泡（未接线）"
    );

    // Return：走带归零。
    ui.set_playhead_ticks(960);
    press(live, KeyCode::Return);
    assert_eq!(ui.get_playhead_ticks(), 0, "关窗后 Return 须归零走带");

    // Delete：最后删选区（摧毁种子，须放末尾）。
    press(live, KeyCode::Character('\u{7f}'));
    assert_eq!(ui.get_selected_note_count(), 0, "关窗后 Delete 须删选区");
    assert!(
        !note_readings(&port.try_project().expect("端口必须握着工程")).contains_key(note),
        "关窗后 Delete 须删工程里的音符"
    );
}

/// 每只模态判据的公共前奏：播种读数 → 武装工具 3 → 开窗落按钮格。
fn seed_and_open(
    live: &mut live::LiveUi,
    port: &Rc<UndoPort>,
    open: impl Fn(&mut live::LiveUi),
) -> (String, u64, u8) {
    let project = port.try_project().expect("端口必须握着工程");
    let (note, reading) = pick_note(&project);
    let (start_tick, start_pitch) = (reading.0, reading.1);
    // m / s 的宿主臂只认 `selected-track-id`（`wire_track_select` 的唯一事实源）——
    // 空串时臂如实拒收。播种第二条轨（lead）的身份，否则 m / s 的读数在基线上是假绿。
    let track_id = project
        .tracks
        .keys()
        .nth(1)
        .expect("夹具里必须有至少两条轨")
        .to_canonical_string();
    let ui = live.ui().clone_strong();
    ui.set_selected_track_id(SharedString::from(track_id.as_str()));
    select_ulids(&ui, std::slice::from_ref(&note));
    ui.set_playhead_ticks(960);
    ui.set_arrangement_view(true);
    ui.set_console_tab(0);
    ui.set_roll_ticks_per_pixel(60);
    arm_tool_3(live);
    open(live);
    (note, start_tick, start_pitch)
}

#[test]
fn export_dialog_holds_expanded_keys() {
    let (mut live, port) = build();
    let (note, start_tick, start_pitch) = seed_and_open(&mut live, &port, open_export);
    assert_keys_held(&mut live, &port, &note, start_tick, start_pitch);
    assert_keys_fire_after_close(&mut live, &port, &note, start_tick, start_pitch);
}

#[test]
fn sound_library_holds_expanded_keys() {
    let (mut live, port) = build();
    let (note, start_tick, start_pitch) = seed_and_open(&mut live, &port, open_sound_library);
    assert_keys_held(&mut live, &port, &note, start_tick, start_pitch);
    assert_keys_fire_after_close(&mut live, &port, &note, start_tick, start_pitch);
}

#[test]
fn missing_sounds_holds_expanded_keys() {
    let (mut live, port) = build();
    let (note, start_tick, start_pitch) = seed_and_open(&mut live, &port, open_missing_sounds);
    assert_keys_held(&mut live, &port, &note, start_tick, start_pitch);
    assert_keys_fire_after_close(&mut live, &port, &note, start_tick, start_pitch);
}

#[test]
fn template_save_holds_expanded_keys() {
    let (mut live, port) = build();
    let (note, start_tick, start_pitch) = seed_and_open(&mut live, &port, open_template_save);
    assert_keys_held(&mut live, &port, &note, start_tick, start_pitch);
    assert_keys_fire_after_close(&mut live, &port, &note, start_tick, start_pitch);
}

#[test]
fn humanize_holds_expanded_keys() {
    let (mut live, port) = build();
    let (note, start_tick, start_pitch) = seed_and_open(&mut live, &port, open_humanize);
    assert_keys_held(&mut live, &port, &note, start_tick, start_pitch);
    assert_keys_fire_after_close(&mut live, &port, &note, start_tick, start_pitch);
}

#[test]
fn drum_gen_holds_expanded_keys() {
    let (mut live, port) = build();
    let (note, start_tick, start_pitch) = seed_and_open(&mut live, &port, open_drum_gen);
    assert_keys_held(&mut live, &port, &note, start_tick, start_pitch);
    assert_keys_fire_after_close(&mut live, &port, &note, start_tick, start_pitch);
}

// ---------------------------------------------------------------------------
// Return：端到端行为（见文件头边界说明）
// ---------------------------------------------------------------------------

#[test]
fn return_does_not_reset_playhead_in_modal() {
    let (mut live, port) = build();
    let project = port.try_project().expect("端口必须握着工程");
    let (note, _) = pick_note(&project);
    let ui = live.ui().clone_strong();
    select_ulids(&ui, std::slice::from_ref(&note));
    ui.set_playhead_ticks(960);
    // 音色库搜索框：Return 只触发 `accepted`（无监听者），不关窗、不归零。
    open_sound_library(&mut live);
    let ui = live.ui().clone_strong();
    assert_eq!(ui.get_sound_library_focused_control(), 700, "须先落到 ✕ 格");
    press(&mut live, KeyCode::Character('\u{19}')); // Backtab 回搜索框（0）
    assert_eq!(
        ui.get_sound_library_focused_control(),
        0,
        "Backtab 后须回到搜索框"
    );
    press(&mut live, KeyCode::Return);
    let ui = live.ui().clone_strong();
    assert!(ui.get_sound_library_open(), "搜索框里按 Return 须不关窗");
    assert_eq!(
        ui.get_playhead_ticks(),
        960,
        "搜索框里按 Return 须不归零走带"
    );
    assert_eq!(
        ui.get_sound_library_focused_control(),
        0,
        "搜索框里按 Return 须不移开焦点"
    );
}

// ---------------------------------------------------------------------------
// 表项纪律：守卫标记恰好一处；六只模态标志、全部扩键都在表里；Esc 不在表里
// ---------------------------------------------------------------------------

#[test]
fn guard_table_covers_every_bare_key() {
    let slint = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("app.slint 必须可读");
    let marker = "T-FIX-305";
    assert_eq!(
        slint.matches(marker).count(),
        1,
        "T-FIX-305 标记在 app.slint 里必须恰好一处（集中守卫唯一）"
    );
    let guard_start = slint.find(marker).expect("标记必须存在");
    // 注释里也提到了 `trigger-shortcut`，所以从守卫的 `if` 起再找它调用处。
    let cond_start = slint[guard_start..]
        .find("if ((root.")
        .map(|offset| guard_start + offset)
        .expect("标记后必须有守卫的 if");
    let guard_end = slint[cond_start..]
        .find("trigger-shortcut")
        .map(|offset| cond_start + offset)
        .expect("守卫后必须有 trigger-shortcut 调用");
    let guard = &slint[cond_start..guard_end];
    for modal in [
        "export-wav-open",
        "sound-library-open",
        "missing-sounds-open",
        "template-save-open",
        "humanize-open",
        "drum-gen-open",
    ] {
        assert!(guard.contains(modal), "守卫必须覆盖模态 {modal}");
    }
    // 219 的旧表（集中守卫不得丢键）。
    for key in [
        "\"1\"", "\"5\"", "\"b\"", "\"i\"", "\"h\"", "\"t\"", "\"d\"",
    ] {
        assert!(guard.contains(key), "守卫必须保留旧表键 {key}");
    }
    for key in ["Key.F3", "Key.F4", "Key.F7", "Key.F8", "Key.F9"] {
        assert!(guard.contains(key), "守卫必须保留旧表键 {key}");
    }
    // 305 扩入的键。
    for key in [
        "Key.F5",
        "Key.F6",
        "Key.Return",
        "Key.Delete",
        "Key.Backspace",
        "Key.LeftArrow",
        "Key.RightArrow",
        "Key.UpArrow",
        "Key.DownArrow",
    ] {
        assert!(guard.contains(key), "守卫必须覆盖扩入键 {key}");
    }
    for key in [
        "\"m\"", "\"s\"", "\"x\"", "\"e\"", "\"p\"", "\"z\"", "\"[\"", "\"]\"", "\"=\"", "\"-\"",
    ] {
        assert!(guard.contains(key), "守卫必须覆盖扩入键 {key}");
    }
    assert!(
        !guard.contains("Key.Escape"),
        "Esc 必须不在守卫表里（陷阱先接住并 accept；焦点在弹窗外时宿主 Cancel 臂兜底）"
    );
}
