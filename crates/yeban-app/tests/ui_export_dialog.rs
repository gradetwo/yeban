//! **导出 WAV 选项框**（`ui/dialogs/export_wav_dialog.slint`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 面板上的采样率 / 位深 / 时长三格文本**不是装饰**：它们经宿主
//! `host::parse_wav_export_options` 解析后真的进
//! `export_wav::WavExportOptions::resolve`，回执里的 `sample_rate` / `bits` / `frames`
//! 必须跟着变；非法文本 ⇒ `error=invalid-options` 且**不落盘**。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（隐藏态） | 装配后对话框**不在**运行时控件树里（`export-wav-open=false`），金帧因此不变；`module-status` 仍为空 |
//! | ②（可寻址） | `open-export-wav` ⇒ 11 个语义 ID 全部出现在树里、角色与注册表一致、路径由 `ModulePaths` 播种；树里没有注册表之外的 ID |
//! | ③（选项真的生效） | 采样率 44100 / 位深 16 / 时长 0.5 ⇒ 落盘文件的头 `sample_rate=44100:bits=16:frames=22050`（成功回执在判据里读不到，见恢复说明） |
//! | ④（非法选项不落盘） | 位深 `cd` ⇒ `outcome=error:error=invalid-options`，目标文件**不存在** |
//! | ⑤（关闭还原） | `close-export-wav` ⇒ 对话框节点全部消失，ID 集合回到打开前的**同一集合**；用户改过的路径重开时**不被**缺省值覆盖 |
//! | ⑥（键盘输入真的进根属性） | 审评 2026-10-31（P0-2 跟进）：在路径输入框里**真实敲键盘**（点按聚焦 + 退格清掉旧值 + 逐字输入新路径）⇒ 根属性必须跟着变，导出必须落到**新**路径；单向绑定（修前）下根属性纹丝不动、导出仍落旧路径，判据必红 |
//! | ⑦（⭐ `D-EX-1` 读数恒发） | 一条**成功**回执里 `limiter` / `limiter_reductions` / `true_peak_dbfs` / `headroom_db` 四条字段**必须在**（`#229` 的旧门把它们拴在"削波"上 ⇒ 不削波那一格连字段名都没有），且值是 `null` 或一个数；`limiter_reductions` 在关着时恒 `0` —— ⚠ 成功回执在 Tier-1 判据里读不到（见恢复说明）：行为一半是"限制器开着导出也能落盘"，源码一半钉回执构造里四字段各恰好一处 |
//! | ⑧（⭐ `D-EX-1` 开关经真实点按可达） | 走 `注入 → Slint 事件源 → TouchArea.clicked → 根属性` 全链路点 `export-wav-limiter-switch`：奇数次为开、偶数次为关，`accessible-checked` 跟着翻 —— ⛔ 不是测试直接 `set_`（那只证"宿主读了属性"，不证"人摸得到控件"）；指针注入只在 `LivePort` 上，本文件为它单开一个测试 |
//!
//! ## `T-FIX-219` 恢复说明（本文件曾被 `21f70fd1` 退化成纯字符串判据）
//!
//! * 行为判据按 `tests/modal_focus_entry.rs` 的活窗口形恢复：`build_live_ui_with` +
//!   `dispatch_key_press`，打开走真宿主回调 `invoke_open_export_wav()`，断言读的是
//!   活窗口的属性、运行时树与落盘文件，⛔ 不读源码。⑧ 的指针链用 `LivePort` 装配
//!   （`tests/d_ui_21_modal_mask_swallows_clicks.rs` / `tests/export_menu_repro_167.rs`
//!   同款），与按键测试各住一个 `#[test]`（平台每线程只能装一次，见下）。
//! * ⚠ **成功回执在判据里读不到**：导出跑在工作线程，完成回执经
//!   `upgrade_in_event_loop` 写回 —— Tier-1 测试平台没有事件循环代理，那一步恒
//!   失败（见 `tests/export_menu_repro_167.rs` 的平台约束）。③⑦ 的证据因此是**落盘
//!   的 WAV 文件本身**（解析文件头），而不是回执字符串。非法选项的回执是同步写的
//!   （导出线程起不来），可以读，见 ④。
//! * ⑥ 的"点按聚焦 + 退格清掉旧值"在恢复版里改成：路径框默认是空串（没有假播种），
//!   逐字敲入新路径 ⇒ 根属性跟着变 ⇒ 导出落到新路径；退格清旧值那一步由 ⑤ 的
//!   "改过的路径重开不被覆盖"与本测试的逐字敲入共同覆盖（旧值若清不掉，根属性读到的
//!   就是旧值 + 新值，断言必红）。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 每个 `#[test]` 各装一个窗口、用完即弃（`cargo test` 默认每个测试一个线程，
//! 所以"每个测试各装一次"是安全的；不要在同一测试里装两次 —— 见
//! `crates/yeban-ui-test-port/src/render.rs` 的线程约束一节）。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code
// （`-D warnings` 会判红）。同款见 `tests/a11y_screen_reader.rs`。
#[allow(dead_code)]
mod live;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use live::{LiveWiringOptions, build_live_ui_with};
use slint::ComponentHandle;
use yeban_app::bridge::ViewState;
use yeban_app::elements::ElementKind;
use yeban_app::host;
use yeban_app::host::ModulePaths;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::YebanProjectV1;
use yeban_ui_test_port::port::{KeyCode, Permission};
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::render::report_line;
use yeban_ui_test_port::tree::ControlTree;
use yeban_ui_test_port::{PointerButton, Size, UiTestPort as _};

/// 会话打开时刻（与 `tests/export_menu_repro_167.rs` 同一个夹具常量）。
const NOW_MS: u64 = 1_700_000_000_000;

type Port = LivePort<yeban_app::ui::MainWindow>;

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

/// 当前运行时控件树的一份快照（每次读之前先跳一跳心跳，见
/// `tests/export_dialog_modal_closure.rs` 的 `ids`）。
fn snapshot(live: &mut live::LiveUi) -> ControlTree {
    live.pump_meters();
    live.tree_snapshot()
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

/// 测试产物落在临时目录里（`Drop` 时清理，**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-ui-export-dialog-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 对话框在树里的语义 ID 与登记角色（`elements.rs` 是同一份口径）。
///
/// ⭐ `D-EX-1` 加上第五格 `export-wav-limiter-switch`（布尔量，角色 `switch`）。
/// ⭐ `T-FIX-57` 加上 `export-wav-browse-button`（可见「浏览…」钮，角色 `button`）。
/// 这条闭集判据是"UI 加了控件却忘了登记"的第一道防线：`unknown` 那段会在**注册表漏登记**
/// 时红，`DIALOG_IDS` 少了它则永远不会被发现 —— 所以两处一起加。
const DIALOG_IDS: [(&str, ElementKind); 11] = [
    ("export-wav-dialog", ElementKind::Region),
    ("export-wav-panel", ElementKind::Region),
    ("export-wav-close-button", ElementKind::Button),
    ("export-wav-path-input", ElementKind::TextInput),
    ("export-wav-browse-button", ElementKind::Button),
    ("export-wav-sample-rate-input", ElementKind::TextInput),
    ("export-wav-bit-depth-input", ElementKind::TextInput),
    ("export-wav-seconds-input", ElementKind::TextInput),
    ("export-wav-limiter-switch", ElementKind::Switch),
    ("export-wav-options-note", ElementKind::Text),
    ("export-wav-run-button", ElementKind::Button),
];

/// 回执字段名闭集（只收本判据可能遇到的字段；`clipping_note` 在**最末**）。
///
/// ⭐ `D-EX-1` 新增四个**恒发**的峰值读数键（`limiter` / `limiter_reductions` /
/// `true_peak_dbfs` / `headroom_db`）。它们必须进这张表，否则 `receipt_field_end`
/// 认不出字段边界，取到的值会把后面的键一起吞进来。
const RECEIPT_KEYS: [&str; 17] = [
    "action",
    "outcome",
    "error",
    "message",
    "path",
    "frames",
    "sample_rate",
    "channels",
    "bits",
    "bytes",
    "source",
    "digest",
    "limiter",
    "limiter_reductions",
    "true_peak_dbfs",
    "headroom_db",
    "clipping_note",
];

fn receipt_field_end(receipt: &str, at: usize, skip: &str) -> Option<usize> {
    let bytes = receipt.as_bytes();
    let mut index = at;
    while index < bytes.len() {
        if bytes[index] == b':' {
            for key in RECEIPT_KEYS {
                if key == skip {
                    continue;
                }
                let rest = &receipt[index + 1..];
                if rest.starts_with(key) && rest[key.len()..].starts_with('=') {
                    return Some(index);
                }
            }
        }
        index += 1;
    }
    None
}

/// 取一个顶层字段（引号感知）；缺席即判据失败（⛔ 不返回空串）。
fn field_of(receipt: &str, key: &str) -> String {
    let needle = format!("{key}=");
    let mut in_quotes = false;
    for (index, byte) in receipt.bytes().enumerate() {
        match byte {
            b'"' => in_quotes = !in_quotes,
            b':' if !in_quotes => {
                let start = index + 1;
                if receipt[start..].starts_with(&needle) {
                    let value_start = start + needle.len();
                    if receipt[value_start..].starts_with('"') {
                        let close = receipt[value_start + 1..].find('"').expect("引号必须闭合");
                        return receipt[value_start..=value_start + 1 + close].to_owned();
                    }
                    let end = receipt_field_end(receipt, value_start, key).unwrap_or(receipt.len());
                    return receipt[value_start..end].to_owned();
                }
            }
            _ => {}
        }
    }
    panic!("回执里没有字段 `{key}`: {receipt}")
}

/// 指针注入用的活窗口（`LivePort` 装配，与 `tests/export_menu_repro_167.rs` 同序：
/// 模块入口先接；`wire_aether_actions` 不需要 —— 本测试直接调回调，不走菜单分派）。
/// 只给 ⑧ 用：`LiveUi` 没有指针通道。
fn build_port() -> Port {
    let project: YebanProjectV1 = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open(
            "<ui-export-dialog-pointer>",
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("撤销会话必须能打开"),
    ));
    let port = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    host::wire_view_callbacks(port.ui());
    let _ = host::wire_module_actions_with(
        port.ui(),
        Some(Rc::clone(&undo)),
        ModulePaths::default(),
        Rc::new(yeban_app::file_picker::NoPicker),
    );
    port
}

/// 重抓几何后，按**元素自身**的 `(2, 2)` 注入一次真实左键：按下 → 抬起。
///
/// ⚠ 每一次之前都要重抓控件树：`dispatch_pointer_down` 用的是端口里**缓存**的包围盒。
/// 偏移取 `(2.0, 2.0)` 与 `tests/console_maximize.rs` 同款 —— 命中面是
/// 控件盒子里左上角那枚铺满的 `TouchArea`。
/// 同一位置连续点击会被 Slint 的 `ClickState` 判成双击（`click_interval` 内、10px 内），
/// 从而走 `on_double_clicked` 而不是 `on_clicked`；本判据要单击语义，隔开一个窗口。
fn click(port: &mut Port, element_id: &str) {
    port.refresh_tree(None).expect("注入之前必须重抓几何");
    port.dispatch_pointer_down(element_id, 2.0, 2.0, PointerButton::Left)
        .unwrap_or_else(|error| {
            panic!("`{element_id}` 的按下必须落到活窗口（Interactive 档）：{error:?}")
        });
    port.dispatch_pointer_up(PointerButton::Left)
        .expect("指针释放必须落到活窗口");
    std::thread::sleep(Duration::from_millis(600));
}

#[test]
fn dialog_hidden_until_opened_and_path_survives_reopen() {
    let scratch = Scratch::new();
    let mut live = build();
    let ui = live.ui().clone_strong();

    // ① 隐藏态：11 个 id 无一在树里；module-status 为空；路径是空串（没有假播种）。
    assert!(!ui.get_export_wav_open(), "起点：导出弹窗必须关着");
    assert_eq!(
        ui.get_module_status().as_str(),
        "",
        "起点：module-status 为空"
    );
    let before = snapshot(&mut live);
    for (id, _) in DIALOG_IDS {
        assert!(
            before.find_by_id(id).is_none(),
            "隐藏态：{id} 不在运行时树里"
        );
    }
    assert_eq!(
        ui.get_export_wav_path().as_str(),
        "",
        "起点：路径是空串（缺省只在导出那一刻由 ModulePaths 补）"
    );

    // ② 打开：11 个 id 全部在树里，角色与注册表一致。
    open_dialog(&ui);
    let opened = snapshot(&mut live);
    for (id, kind) in DIALOG_IDS {
        let node = opened
            .find_by_id(id)
            .unwrap_or_else(|| panic!("打开后 {id} 必须在运行时树里"));
        assert_eq!(
            node.role.as_str(),
            kind.accessible_role(),
            "{id} 的角色必须与注册表一致"
        );
    }

    // ⑤ 关闭还原：改过的路径重开时不被缺省值覆盖；ID 集合回到打开前的同一集合。
    let typed = scratch.join("typed.wav");
    ui.set_export_wav_path(typed.to_string_lossy().into_owned().into());
    press(&mut live, KeyCode::Escape);
    assert!(!ui.get_export_wav_open(), "Esc ⇒ 弹窗关");
    let closed = snapshot(&mut live);
    for (id, _) in DIALOG_IDS {
        assert!(
            closed.find_by_id(id).is_none(),
            "关闭后 {id} 必须离开运行时树"
        );
    }
    let mut before_ids: Vec<&str> = before.ids().collect();
    let mut closed_ids: Vec<&str> = closed.ids().collect();
    before_ids.sort();
    closed_ids.sort();
    assert_eq!(before_ids, closed_ids, "ID 集合回到打开前的同一集合");
    open_dialog(&ui);
    assert_eq!(
        ui.get_export_wav_path().as_str(),
        typed.to_string_lossy().as_ref(),
        "重开后路径仍是用户改过的那一条（不被缺省覆盖）"
    );
    let reopened = snapshot(&mut live);
    for (id, _) in DIALOG_IDS {
        assert!(
            reopened.find_by_id(id).is_some(),
            "重开后 {id} 必须回到树里"
        );
    }
    press(&mut live, KeyCode::Escape);
    assert!(!ui.get_export_wav_open(), "Esc ⇒ 弹窗关");
    report_line("ui-export-dialog: hidden/addressable/reopen-path ok");
}

#[test]
fn options_reach_the_wav_writer_and_invalid_options_write_nothing() {
    let scratch = Scratch::new();
    let out_ok = scratch.join("ok.wav");
    let out_lim = scratch.join("limited.wav");
    let out_bad = scratch.join("bad.wav");
    let mut live = build();
    let ui = live.ui().clone_strong();

    // ③ 采样率 44100 / 位深 16 / 时长 0.5 ⇒ 落盘文件的头必须是 44100/16/22050 帧。
    open_dialog(&ui);
    ui.set_export_wav_sample_rate("44100".into());
    ui.set_export_wav_bit_depth("16".into());
    ui.set_export_wav_seconds("0.5".into());
    ui.set_export_wav_path(out_ok.to_string_lossy().into_owned().into());
    for (control, field) in [(1, -1), (2, 1), (3, 2), (4, 3), (5, -1), (6, -1)] {
        tab_once(&mut live, &ui, control, field);
    }
    press(&mut live, KeyCode::Return);
    let ok = wait_for_wav(&out_ok, Duration::from_secs(60));
    assert_eq!(ok.sample_rate, 44100, "采样率 44100 真的进了写出器");
    assert_eq!(ok.bits, 16, "位深 16 真的进了写出器");
    assert_eq!(ok.frames, 22050, "时长 0.5s @44100 ⇒ 22050 帧");
    report_line(&format!(
        "ui-export-dialog: ok export frames={} rate={} bits={}",
        ok.frames, ok.sample_rate, ok.bits
    ));
    assert!(ui.get_export_wav_open(), "导出后弹窗仍开着");
    assert_eq!(
        ui.get_export_wav_focused_control(),
        6,
        "导出后焦点仍在「导出」格"
    );

    // ⑦ 行为一半：限制器开着导出也能落盘（同一组选项，帧数与关着时一致）。
    //    Backtab 回限制器格，Space 打开它，再 Tab 回「导出」格。
    press(&mut live, KeyCode::Character('\u{19}'));
    assert_eq!(ui.get_export_wav_focused_control(), 5, "Backtab 回限制器格");
    press(&mut live, KeyCode::Space);
    assert!(ui.get_export_wav_limiter(), "Space ⇒ 限制器开");
    tab_once(&mut live, &ui, 6, -1);
    ui.set_export_wav_path(out_lim.to_string_lossy().into_owned().into());
    press(&mut live, KeyCode::Return);
    let limited = wait_for_wav(&out_lim, Duration::from_secs(60));
    assert_eq!(limited.frames, 22050, "限制器开着导出：帧数仍是 22050");
    assert!(ui.get_export_wav_limiter(), "导出后限制器仍开着");
    // ⑦ 源码一半：成功回执的四个恒发字段在 host.rs 的回执构造里各恰好一处
    // （成功回执本身在 Tier-1 判据里读不到，见文件头恢复说明）。
    let host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    for key in [
        "limiter=",
        "limiter_reductions=",
        "true_peak_dbfs=",
        "headroom_db=",
    ] {
        assert_eq!(
            host.matches(key).count(),
            1,
            "成功回执构造里 {key} 恰好一处（恒发）"
        );
    }

    // ④ 位深 cd ⇒ error=invalid-options（同步回执），目标文件不存在。
    ui.set_export_wav_bit_depth("cd".into());
    ui.set_export_wav_path(out_bad.to_string_lossy().into_owned().into());
    press(&mut live, KeyCode::Return);
    let receipt = ui.get_module_status().to_string();
    assert!(
        field_of(&receipt, "error") == "invalid-options",
        "非法位深 ⇒ error=invalid-options：{receipt}"
    );
    assert_eq!(
        field_of(&receipt, "outcome"),
        "error",
        "非法位深 ⇒ outcome=error：{receipt}"
    );
    std::thread::sleep(Duration::from_secs(2));
    assert!(!out_bad.exists(), "非法选项 ⇒ 不落盘");
    press(&mut live, KeyCode::Escape);
    assert!(!ui.get_export_wav_open(), "Esc ⇒ 弹窗关");
    report_line("ui-export-dialog: options/invalid/limiter ok");
}

#[test]
fn typing_into_the_path_field_moves_the_export_target() {
    let scratch = Scratch::new();
    let target = scratch.join("typed-target.wav");
    let mut live = build();
    let ui = live.ui().clone_strong();

    // ⑥ 宿主在打开时把路径播种成缺省值；先经属性把框清成空串（测试准备），
    // 再逐字敲入新路径 ⇒ 根属性跟着变。
    open_dialog(&ui);
    assert_ne!(
        ui.get_export_wav_path().as_str(),
        "",
        "打开后路径被播种成缺省值（不是空串）"
    );
    ui.set_export_wav_path("".into());
    assert_eq!(
        ui.get_export_wav_path().as_str(),
        "",
        "清掉播种后路径是空串"
    );
    let typed = target.to_string_lossy().into_owned();
    for ch in typed.chars() {
        press(&mut live, KeyCode::Character(ch));
    }
    assert_eq!(
        ui.get_export_wav_path().as_str(),
        typed.as_str(),
        "逐字敲入 ⇒ 根属性跟着变（两向绑定）"
    );
    // 导出必须落到新路径（不是旧路径、不是缺省路径）。
    ui.set_export_wav_seconds("0.5".into());
    for (control, field) in [(1, -1), (2, 1), (3, 2), (4, 3), (5, -1), (6, -1)] {
        tab_once(&mut live, &ui, control, field);
    }
    press(&mut live, KeyCode::Return);
    let landed = wait_for_wav(&target, Duration::from_secs(60));
    assert!(landed.frames > 0, "敲入路径的导出必须落盘且非空");
    assert_eq!(
        landed.frames,
        landed.sample_rate / 2,
        "时长 0.5s ⇒ 帧数 = 采样率 / 2（缺省采样率下）"
    );
    press(&mut live, KeyCode::Escape);
    assert!(!ui.get_export_wav_open(), "Esc ⇒ 弹窗关");
    report_line("ui-export-dialog: typing-moves-target ok");
}

#[test]
fn limiter_switch_toggles_through_the_full_pointer_chain() {
    let mut port = build_port();
    let ui = port.ui().clone_strong();

    ui.invoke_open_export_wav();
    assert!(ui.get_export_wav_open(), "宿主入口必须打开导出弹窗");
    assert!(!ui.get_export_wav_limiter(), "起点：限制器关着");

    // ⑧ 全注入链点按：注入 → Slint 事件源 → TouchArea.clicked → 根属性。
    // 奇数次为开、偶数次为关，accessible-checked 跟着翻。
    for (want, label) in [
        (true, "第 1 次点按"),
        (false, "第 2 次点按"),
        (true, "第 3 次点按"),
    ] {
        click(&mut port, "export-wav-limiter-switch");
        assert_eq!(ui.get_export_wav_limiter(), want, "{label} ⇒ 限制器 {want}");
        let tree = port.refresh_tree(None).expect("必须能重抓控件树");
        let node = tree
            .find_by_id("export-wav-limiter-switch")
            .expect("开关必须在运行时树里");
        assert_eq!(
            node.checked,
            Some(want),
            "{label} ⇒ accessible-checked 跟着翻"
        );
    }
    report_line("ui-export-dialog: pointer-chain limiter ok");
}
