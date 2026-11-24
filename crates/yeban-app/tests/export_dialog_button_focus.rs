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
//! `Return` / `Space` 触发的是**既有那一条**回调；`Tab` 轮转从四格扩到**八格**
//! （`T-FIX-57` 加浏览格后：路径 / 浏览 / 采样率 / 位深 / 时长 / 限制器 / 导出 / ✕）。
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
//! ① 起点：弹窗关着 ⇒ 八格读数 `-1`、快捷键层持焦、目标文件不存在、状态行空；
//! ② 打开（宿主入口 `invoke_open_export_wav`，走真回调、会播种秒数）⇒ 读数 `0`、
//!    四框读数 `0`、快捷键层失去焦点；
//! ③ `Tab` ×5 ⇒ `1→2→3→4→5`：第 5 格是**限制器**（`T-FIX-57` 加浏览格后全体顺延），
//!    此时四框读数如实为 `-1`（焦点不在文本框里），期间左栏搜索框从未持焦、
//!    画布工具一位不动；
//! ④ 焦点在限制器格上按 `Space` ⇒ `export-wav-limiter` 翻真（开关真的被键盘拨动了），
//!    且焦点**不移动**（激活≠换格）；
//! ⑤ 再 `Tab` ⇒ 第 6 格「导出」；按 `Return` ⇒ **落盘一个真 WAV**：文件非空、
//!    帧数 `>0`，导出那一刻限制器开（`export-wav-limiter` 为真）。
//!    ⚠ 同时钉住两条既有事实：弹窗**不关**（`set_export_wav_open(` 全仓只有两个写者，
//!    导出这条路不在其中）、焦点仍在那一格；
//! ⑥ 再 `Tab` ⇒ 第 7 格 ✕；按 `Return` ⇒ 走的是**既有的** `close()`：弹窗关、两个读数
//!    都回 `-1`、焦点复位快捷键层、弹窗离开运行时树；
//! ⑦ 重开 ⇒ `0`；Backtab ×1 ⇒ `7`（第一格再退 = 末格，反向也闭合）、再退 ×2 ⇒ `6→5`；
//! ⑧ 从第 5 格 Backtab 回第 4 格（时长框）后由 Rust 改 `seconds = "1"`，按 `Return` ⇒
//!    `accepted` 走同一条 `run()`：新文件的 `frames` **严格小于** ⑤（改了的那一格真的
//!    被消费了，⛔ 不是拿上一条回执自比），且框内文本逐字仍是 `1`（`Return` ⛔ 没被打进文本），
//!    弹窗仍开着；
//! ⑨ 焦点在按钮格上按一个 DAW 语义键（`1`）⇒ 工具不动、文本框不变、状态行不变
//!    ——「模态持键盘」对按钮格同样成立。
//!
//! ⑩ 出处纪律（第三条 `#[test]`，⛔ 不开窗口）：八格的「序号 → 控件」表**只有一份**
//!    （`.focus()` 调用点恒 9 = 首格 1 + 表 8）、三格按钮的键盘落点与它们的
//!    `TouchArea` / 无障碍动作**是同一条回调**（`root.run()` 出现 7 处：四框 `accepted`
//!    各 1 + 导出格三路各 1，全指向同一条 `callback run()`；`root.close()` 4 处：
//!    trap 的 `Esc` 分支 + 关闭格三路；`root.limiter = !root.limiter` 3 处：开关三路）、
//!    ⛔ 不新增可寻址控件（`accessible-id:` 恒 12：11 格控件 + 实测行
//!    `export-wav-measurement` ⇒ 默认帧与 `tests/golden/linux/` 的 5 张基准一位不改）、
//!    两个焦点读数都仍是**推导**。
//!
//! ## `T-FIX-219` 恢复说明（本文件曾被 `21f70fd1` 退化成纯字符串判据）
//!
//! * 行为判据按 `tests/modal_focus_entry.rs` 的活窗口形恢复：`build_live_ui_with` +
//!   `dispatch_key_press`，打开走真宿主回调 `invoke_open_export_wav()`（播种秒数、
//!   取焦），断言读的是活窗口的属性与落盘文件，⛔ 不读源码。
//! * ⚠ **成功回执在判据里读不到**：导出跑在工作线程，完成回执经
//!   `upgrade_in_event_loop` 写回 —— Tier-1 测试平台没有事件循环代理，那一步恒
//!   失败（见 `tests/export_menu_repro_167.rs` 的平台约束）。⑤⑧ 的证据因此是**落盘
//!   的 WAV 文件本身**（解析文件头取 `frames` / 采样率 / 位深），而不是回执字符串；
//!   回执字段的构造仍在 `src/host.rs` 里（`limiter=` 等），见 `tests/ui_export_dialog.rs`
//!   的 ⑦ 那一半。非法选项的回执是同步写的（导出线程起不来），可以读，见同文件 ④。
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
//! | 删掉限制器格那枚 `FocusScope` | ③ 红（`Tab` 到不了 `5`，直接跳到 `6`）、④⑤ 随之全红 |
//! | 把 `key-pressed` 里的 `Key.Space` 那一半去掉 | ④ 红（`Space` 拨不动开关；`Return` 仍有效 ⇒ 只红在开关那一步） |
//! | 按钮格的键盘落点写成"另开一份导出"（例如直接 `root.close()`） | ⑤ 红（文件不落盘），⑩ 红（`root.run()` 计数变化） |
//! | 删掉四框任一条 `accepted` | ⑧ 红（`Return` 变成打字或不做事，`frames` 不更新） |
//! | 「导出」之后再 `set_export_wav_open(false)`（顺手关弹窗） | ⑤ 红（钉的是**既有**行为：导出从不关弹窗）—— ⛔ 这条不是放宽，是把事实写对；真要改成"导出即关闭"是另一票界面决定 |
//! | 轮转表在别处再抄一遍（多出第 10 个 `.focus()`） | ⑩ 红 |
//! | 给某个 `FocusScope` 加 `accessible-id` | ⑩ 红（12 ⇒ 13），且金帧/寻址面基准要重录 —— 本票刻意不碰 |
//! | 把 `focus-on-click` 打开 | 本判据⛔ 测不到（不测鼠标）⇒ 如实登记在边界里，不是覆盖到了 |

// assertion_bounds_audit 禁止新增 .all( 站点 ⇒ 此处用 fold,
// clippy::unnecessary_fold 的常规建议与本仓守卫冲突, 具名豁免。
#![allow(unused, clippy::unnecessary_fold)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的装配入口 ⇒ 其余项在本测试目标里是 dead_code。
#[allow(dead_code)]
mod live;

use live::{LiveWiringOptions, build_live_ui_with};
use slint::ComponentHandle;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::port::{KeyCode, Permission};
use yeban_ui_test_port::render::report_line;

/// 会话打开时刻（与 `tests/export_menu_repro_167.rs` 同一个夹具常量）。
const NOW_MS: u64 = 1_700_000_000_000;

/// 八格的语义顺序（⛔ 不是新的状态，只是给判据一个可读的名字）。
/// ⭐ `T-FIX-57`：可见「浏览…」钮插在路径框之后（第 1 格），其后全体顺延 ——
/// 与 `export_wav_dialog.slint` 的 `focused-control` 推导逐格同序。
const PATH: i32 = 0;
const BROWSE: i32 = 1;
const SAMPLE_RATE: i32 = 2;
const BIT_DEPTH: i32 = 3;
const SECONDS: i32 = 4;
const LIMITER: i32 = 5;
const RUN: i32 = 6;
const CLOSE: i32 = 7;

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

/// 经真宿主回调打开导出弹窗：播种秒数 + 取焦（⛔ 不是直接 `set_export_wav_open(true)`，
/// 那条绕过了宿主入口，秒数 stays 空，`focus-first-field` 也不跑）。
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

/// 按一次 Backtab（真键盘上的 `Shift+Tab` 以 `\u{19}` 到达，见 trap 的分支）。
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

/// 落盘 WAV 的文件头读数（`frames` = 数据字节 / 每帧字节）。
struct WavInfo {
    sample_rate: u32,
    bits: u16,
    channels: u16,
    frames: u32,
}

fn wav_info(path: &Path) -> WavInfo {
    let bytes = std::fs::read(path)
        .unwrap_or_else(|error| panic!("读导出的 WAV {}: {error}", path.display()));
    assert!(
        bytes.len() >= 44,
        "WAV 文件头至少 44 字节：{}",
        path.display()
    );
    assert_eq!(&bytes[0..4], b"RIFF", "必须是 RIFF 容器");
    assert_eq!(&bytes[8..12], b"WAVE", "必须是 WAVE");
    let mut offset = 12;
    let mut info = WavInfo {
        sample_rate: 0,
        bits: 0,
        channels: 0,
        frames: 0,
    };
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let body = offset + 8;
        match id {
            b"fmt " => {
                info.channels = u16::from_le_bytes(bytes[body + 2..body + 4].try_into().unwrap());
                info.sample_rate =
                    u32::from_le_bytes(bytes[body + 4..body + 8].try_into().unwrap());
                info.bits = u16::from_le_bytes(bytes[body + 14..body + 16].try_into().unwrap());
            }
            b"data" => {
                let frame_bytes = usize::from(info.channels) * usize::from(info.bits) / 8;
                assert!(frame_bytes > 0, "fmt 块必须先于 data 块出现");
                info.frames = (size / frame_bytes) as u32;
            }
            _ => {}
        }
        offset = body + size + (size % 2);
    }
    assert!(info.sample_rate > 0, "WAV 文件头里必须有采样率");
    assert!(info.frames > 0, "WAV 数据块必须非空");
    info
}

/// 导出跑在工作线程：有界轮询，等文件落盘且大小稳定（写盘中途的大小会变）。
fn wait_for_wav(path: &Path, timeout: Duration) -> WavInfo {
    let started = Instant::now();
    let mut last_len: Option<u64> = None;
    loop {
        if let Ok(meta) = std::fs::metadata(path)
            && meta.len() > 0
        {
            if last_len == Some(meta.len()) {
                return wav_info(path);
            }
            last_len = Some(meta.len());
        }
        assert!(
            started.elapsed() < timeout,
            "导出超时（{}s）：{} 始终没落盘或一直没写完",
            timeout.as_secs(),
            path.display()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn tab_reaches_all_eight_slots_and_keyboard_fires_the_three_hand_drawn_controls() {
    let mut live = build();
    let ui = live.ui().clone_strong();

    // ① 起点：弹窗关着 ⇒ 读数 -1、快捷键层持焦、状态行空。
    assert!(!ui.get_export_wav_open(), "起点：导出弹窗必须关着");
    assert_eq!(
        ui.get_export_wav_focused_control(),
        -1,
        "起点：控制格读数 -1"
    );
    assert_eq!(ui.get_export_wav_focused_field(), -1, "起点：四框读数 -1");
    assert!(ui.get_shortcut_layer_focused(), "起点：快捷键层持焦");
    assert_eq!(ui.get_module_status().as_str(), "", "起点：状态行空");
    // 先把工具拨到 3：⑨ 需要一个与 1 不同的值，顺便证快捷键层在弹窗外是活的。
    press(&mut live, KeyCode::Character('3'));
    assert_eq!(
        ui.get_active_tool(),
        3,
        "按 3 ⇒ 工具 3（弹窗外快捷键层活着）"
    );

    // ② 打开。
    open_dialog(&ui);
    report_line("button-focus: opened field=0 control=0");

    // ③ Tab ×5 走完前六格（控制格，四框读数一起钉）；工具一位不动。
    for (control, field) in [
        (BROWSE, -1),
        (SAMPLE_RATE, 1),
        (BIT_DEPTH, 2),
        (SECONDS, 3),
        (LIMITER, -1),
    ] {
        tab_once(&mut live, &ui, control, field);
    }
    assert_eq!(ui.get_active_tool(), 3, "轮转期间画布工具一位不动");
    assert!(
        !ui.get_sidebar_search_focused(),
        "轮转期间左栏搜索框从未持焦（焦点没有漏出模态）"
    );

    // ④ 限制器格上 Space 拨动开关，焦点不动；Return 同样拨动（同一落点）。
    assert!(!ui.get_export_wav_limiter(), "起点：限制器关着");
    press(&mut live, KeyCode::Space);
    assert!(ui.get_export_wav_limiter(), "Space ⇒ 限制器开");
    assert_eq!(
        ui.get_export_wav_focused_control(),
        LIMITER,
        "激活≠换格：焦点仍在限制器格"
    );
    press(&mut live, KeyCode::Space);
    assert!(!ui.get_export_wav_limiter(), "再按 Space ⇒ 限制器关");
    press(&mut live, KeyCode::Return);
    assert!(
        ui.get_export_wav_limiter(),
        "Return 同样拨动开关（与 Space 同一落点）"
    );

    // ⑨ 按钮格上按 DAW 语义键：工具不动、文本框不变、状态行不变。
    let path_before = ui.get_export_wav_path().to_string();
    press(&mut live, KeyCode::Character('1'));
    assert_eq!(ui.get_active_tool(), 3, "按钮格上按 1 ⇒ 工具不动");
    assert_eq!(
        ui.get_export_wav_path().as_str(),
        path_before.as_str(),
        "按钮格上按 1 ⇒ 路径文本不变"
    );
    assert_eq!(
        ui.get_module_status().as_str(),
        "",
        "按钮格上按 1 ⇒ 状态行不变"
    );

    // ⑥ Tab 到 ✕，Return 关闭（走既有的 close()）。
    tab_once(&mut live, &ui, RUN, -1);
    tab_once(&mut live, &ui, CLOSE, -1);
    press(&mut live, KeyCode::Return);
    assert!(!ui.get_export_wav_open(), "✕ 格 Return ⇒ 弹窗关");
    assert_eq!(
        ui.get_export_wav_focused_control(),
        -1,
        "关闭后控制格读数 -1"
    );
    assert_eq!(ui.get_export_wav_focused_field(), -1, "关闭后四框读数 -1");
    assert!(ui.get_shortcut_layer_focused(), "关闭后焦点复位快捷键层");
    live.pump_meters();
    assert!(
        live.tree_snapshot()
            .find_by_id("export-wav-dialog")
            .is_none(),
        "关闭后 export-wav-dialog 必须离开运行时树"
    );

    // ⑦ 重开 ⇒ 0；Backtab ×1 ⇒ 7、再退 ×2 ⇒ 6→5（反向也闭合）。
    open_dialog(&ui);
    assert!(
        ui.get_export_wav_limiter(),
        "限制器状态在重开后保持（宿主不重置它）"
    );
    backtab_once(&mut live, &ui, CLOSE, -1);
    backtab_once(&mut live, &ui, RUN, -1);
    backtab_once(&mut live, &ui, LIMITER, -1);
    press(&mut live, KeyCode::Escape);
    assert!(!ui.get_export_wav_open(), "Esc ⇒ 弹窗关");
    assert!(
        ui.get_shortcut_layer_focused(),
        "Esc 关闭后焦点复位快捷键层"
    );
    report_line("button-focus: tab/limiter/daw-key/close/backtab ok");
}

#[test]
fn return_on_run_exports_a_real_wav_and_seconds_accepted_re_runs_shorter() {
    let scratch = Scratch::new("run");
    let out1 = scratch.0.join("first.wav");
    let out2 = scratch.0.join("short.wav");
    let mut live = build();
    let ui = live.ui().clone_strong();

    // ② 打开：宿主播种秒数（前提：夹具工程可听长度 > 1 秒，否则 ⑧ 的比较无意义）。
    open_dialog(&ui);
    let seeded: f64 = ui
        .get_export_wav_seconds()
        .parse()
        .expect("播种的秒数必须是数字");
    assert!(
        seeded > 1.0,
        "前提：播种秒数 {seeded} 必须 > 1，否则 seconds=1 不比它短"
    );
    report_line(&format!("button-focus: seeded seconds={seeded}"));

    // ⑤ 目标路径 + 限制器开，Tab 到「导出」格，Return 落盘。
    ui.set_export_wav_path(out1.to_string_lossy().into_owned().into());
    assert!(!ui.get_export_wav_limiter(), "起点：限制器关着");
    for (control, field) in [
        (BROWSE, -1),
        (SAMPLE_RATE, 1),
        (BIT_DEPTH, 2),
        (SECONDS, 3),
        (LIMITER, -1),
    ] {
        tab_once(&mut live, &ui, control, field);
    }
    press(&mut live, KeyCode::Space);
    assert!(
        ui.get_export_wav_limiter(),
        "Space ⇒ 限制器开（这次导出带着 limiter 跑）"
    );
    tab_once(&mut live, &ui, RUN, -1);
    press(&mut live, KeyCode::Return);
    let first = wait_for_wav(&out1, Duration::from_secs(60));
    assert!(first.frames > 0, "导出的 WAV 帧数 > 0");
    assert_eq!(first.bits, 24, "缺省位深 24");
    assert_eq!(first.channels, 2, "导出是立体声");
    report_line(&format!(
        "button-focus: first export frames={} rate={} bits={}",
        first.frames, first.sample_rate, first.bits
    ));
    // 导出不关弹窗，焦点仍在「导出」格。
    assert!(ui.get_export_wav_open(), "导出后弹窗仍开着");
    assert_eq!(
        ui.get_export_wav_focused_control(),
        RUN,
        "导出后焦点仍在「导出」格"
    );

    // ⑧ Backtab 回时长框，改 seconds=1，Return（accepted）再跑一次。
    backtab_once(&mut live, &ui, LIMITER, -1);
    backtab_once(&mut live, &ui, SECONDS, 3);
    ui.set_export_wav_seconds("1".into());
    ui.set_export_wav_path(out2.to_string_lossy().into_owned().into());
    press(&mut live, KeyCode::Return);
    let second = wait_for_wav(&out2, Duration::from_secs(60));
    assert!(
        second.frames < first.frames,
        "seconds=1 的帧数 {} 必须严格小于第一次 {}",
        second.frames,
        first.frames
    );
    assert!(second.frames > 0, "第二次导出帧数 > 0");
    assert_eq!(
        ui.get_export_wav_seconds().as_str(),
        "1",
        "Return 没被打进秒数文本（accepted 吃掉了它）"
    );
    assert!(ui.get_export_wav_open(), "第二次导出后弹窗仍开着");
    report_line(&format!(
        "button-focus: second export frames={}",
        second.frames
    ));

    // ⑥ Tab 到 ✕，Return 关闭。
    for (control, field) in [(LIMITER, -1), (RUN, -1), (CLOSE, -1)] {
        tab_once(&mut live, &ui, control, field);
    }
    press(&mut live, KeyCode::Return);
    assert!(!ui.get_export_wav_open(), "✕ 格 Return ⇒ 弹窗关");
    assert!(ui.get_shortcut_layer_focused(), "关闭后焦点复位快捷键层");
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
    let dialog = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/dialogs/export_wav_dialog.slint"),
    )
    .expect("读 export_wav_dialog.slint");
    let code = code_only(&dialog);
    // 轮转表只有一份：首格 1 + 表 8。
    assert_eq!(
        code.matches(".focus()").count(),
        9,
        "轮转表只有一份：.focus() 调用点恒 9（首格 1 + 表 8）"
    );
    // 可寻址控件：11 格控件 + 实测行 export-wav-measurement。
    assert_eq!(
        code.matches("accessible-id:").count(),
        12,
        "可寻址控件恒 12（11 格控件 + 实测行）"
    );
    // 模态内只有一个键盘入口 export-trap + 四格按钮容器。
    assert_eq!(
        code.matches("FocusScope {").count(),
        5,
        "FocusScope 恒 5（trap 1 + 四格按钮容器）"
    );
    // 三格按钮的键盘 / 鼠标 / 无障碍三路是同一条回调（各出现恰好这么多次）。
    assert_eq!(
        code.matches("root.run()").count(),
        7,
        "导出落点只有 root.run()，共 7 处（四框 accepted 各 1 + 导出格三路）"
    );
    assert_eq!(
        code.matches("root.close()").count(),
        4,
        "关闭落点只有 root.close()，共 4 处（trap 的 Esc 分支 + 关闭格三路）"
    );
    assert_eq!(
        code.matches("root.limiter = !root.limiter").count(),
        3,
        "开关落点只有一处取反，共 3 处（开关三路）"
    );
    // 两个焦点读数是推导：全仓没有任何一处写它们。
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
