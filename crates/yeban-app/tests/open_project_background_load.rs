//! T-FIX-428：打开工程的读盘 / 解码 / 资产核对必须在**后台线程**跑，UI 线程只等排空。
//!
//! ## 本判据证什么（简报的三条，一条一个断言簇）
//!
//! ① 读 / 解析**不在** UI 线程上：工作线程在第一个阶段回调里把自己的线程 id 写进
//!    状态（`OpenProjectLoadState::progress().load_thread`）；判据断言它与 UI 线程
//!    （本测试线程）**不同**，且 `file_bytes` 等于夹具文件的真实字节数。
//! ② UI 在加载中途**没有冻住**：触发之后、排空之前，在 UI 线程上做一次属性往返
//!    （`playhead-ticks` 写 / 读 —— 加载路径碰都不碰它），证明回调立刻返回、
//!    窗口照常响应。
//! ③ 取消**不是**失败：加载中途取消 ⇒ 工程一位不动（指纹不变），回执是
//!    `outcome=cancelled`（⛔ 不是 `outcome=error`）。取消走 `MainWindow` 的
//!    `cancel-open-project` 回调（`ui.invoke_cancel_open_project()`）—— 取消按钮
//!    本身住在控制台树（`mixer_console.slint` 的加载行）里，而那棵树目前只在
//!    `app.slint` 的 `if false` 分支里实例化（aether 重构后未进活窗口）⇒ 按钮的
//!    点击路径与「打开工程」按钮同款，只能走源码级接线（`accessible-action-default`
//!    ⇒ `root.cancel-open-project()`），活窗口里按不到它；判据钉的是回调 →
//!    置标志 → 排空出 `cancelled` 这一段（与按钮调的是**同一个**回调）。
//!
//! ## 夹具为什么这么大
//!
//! 读 / 解码必须**慢到能被观测到**：触发 `invoke_open_project()` 返回的那一刻，
//! 加载必须还在飞（`is_in_flight()` 为真），否则"不在 UI 线程上"这句话无从验证 ——
//! 夹具太小会直接 FAIL（不是 flaky 红，是响亮的"夹具太小"）。尺寸口径（盒上实测，
//! 见收据）：10 万音符 + 4×8 MiB 随机资产 + 8 MiB history.dag，容器约 44 MiB，
//! 后台加载约数百毫秒（每次跑都在 stderr 打一行实测耗时）。
//!
//! ## 平台与排空（Tier-1 的边界，如实登记）
//!
//! 本文件用 Tier-1 无头平台（`build_main_window_with_console_tab`，与
//! `tests/missing_sounds_focus_scroll.rs` 同款活窗口）：它**没有**
//! `new_event_loop_proxy` ⇒ 工作线程的 `upgrade_in_event_loop` 返回 `Err`（不是
//! 错误：进度与结果仍在状态里），排空走**确定性**那条路 ——
//! [`yeban_app::host::wait_for_pending_open_project`]（join）＋
//! [`yeban_app::host::apply_pending_open_project`]（UI 线程上直接落）。生产路径
//! （有事件循环的真窗口）走同一段 `apply_pending_open_project`，只差"谁请它排空"。
//! 事件循环那一跳本身（`upgrade_in_event_loop` 请 UI 线程跑排空闭包）与
//! `midi-in` 是逐字同一行习语，本文件不另造一个事件循环后端去证它。
//!
//! ## 反向腿（怎么变红）
//!
//! * 把 `stepwise` 调回 `on_open_project` 回调里（同步读）⇒ `load_thread` 就是
//!   UI 线程 ⇒ ① 红；
//! * 把 `cancel` 检查点从 `stepwise` 里拿掉 ⇒ 取消后工程照常换进来 ⇒ ③ 红；
//! * 把"读盘 + 换会话"都放回回调里 ⇒ ② 的往返在触发后立刻冻住 ⇒ ② 红。
//!
//! ## 平台约束：本文件有两个 `#[test]`
//!
//! `slint::platform::set_platform` 是**线程局部**的，`Tier1Window::install` 每线程只能成功
//! 一次；libtest 每个判据跑在自己的线程上 ⇒ 两个判据各装一次，互不干扰（与
//! `tests/missing_sounds_focus_scroll.rs` 两个判据同款）。

#![allow(unused)]
#[path = "common/mod.rs"]
mod common;

use std::cell::RefCell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;

use common::ScriptedPicker;
use slint::ComponentHandle;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession, project_fingerprint};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::render::Tier1Window;

/// 撤销会话的"现在"（与 `open_project_ui.rs` 的常量同款：判据不需要墙钟）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 夹具尺寸：10 万音符；4 块 8 MiB 随机资产；8 MiB history.dag。
const NOTE_COUNT: usize = 100_000;
const ASSET_CHUNKS: usize = 4;
const ASSET_CHUNK_BYTES: usize = 8 * 1024 * 1024;
const HISTORY_BYTES: usize = 8 * 1024 * 1024;

/// `YEBAN_SETTINGS_PATH` 是进程级变量，两个判据用 [`ENV_LOCK`] 串行（与
/// `tests/file_picker_dialogs.rs` 同一条纪律）：排空那一步 `record_recent_project`
/// 会写设置文件，⛔ 不能写进用户真正的设置目录。
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// `YEBAN_SETTINGS_PATH` 的 RAII 守卫（`Drop` 时恢复原值）。
struct SettingsEnv {
    previous: Option<OsString>,
}

impl SettingsEnv {
    /// ⚠ 调用方**必须**先持有 [`ENV_LOCK`]。
    fn set(path: &Path) -> Self {
        let previous = std::env::var_os(yeban_app::settings::SETTINGS_PATH_ENV);
        // SAFETY: 调用方持有 ENV_LOCK；本测试进程内没有其它线程读这个变量。
        unsafe {
            std::env::set_var(yeban_app::settings::SETTINGS_PATH_ENV, path);
        }
        Self { previous }
    }
}

impl Drop for SettingsEnv {
    fn drop(&mut self) {
        // SAFETY: 守卫比调用方的 ENV_LOCK 先析构（同一测试内后声明者先 drop），
        // 且恢复的是进入时读到的原值。
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(yeban_app::settings::SETTINGS_PATH_ENV, value),
                None => std::env::remove_var(yeban_app::settings::SETTINGS_PATH_ENV),
            }
        }
    }
}

/// 临时目录（`Drop` 时清理，**绝不污染仓库**）。形态照 `open_project_ui.rs`。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-open-project-bg-{tag}-{}-{}",
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

/// 确定性伪随机字节（不可压缩 —— 容器不会把它压小；种子固定 ⇒ 夹具可复现）。
fn random_bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed | 1;
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        // xorshift64*（⛔ 不用 rand —— 不给判据加依赖）。
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        out.extend_from_slice(&state.wrapping_mul(0x2545F4914F6CDD1D).to_le_bytes());
    }
    out.truncate(len);
    out
}

/// 大夹具：一份**真的**落盘的工程容器 —— 很多音符（解码慢）＋ 大块随机资产
/// （读盘 / 核对慢）＋ 大块 history.dag（容器解码慢）。
///
/// 返回 `(容器路径, 文件字节数, 夹具工程指纹)`。
/// `history.dag` 用随机字节：容器层只按原始字节搬运（不解读、不校验），核对那一步
/// 解不开的历史**直接跳过**（`open.rs` 的 `collect_asset_references`，`if let Ok`
/// 口径）⇒ 随机历史不会把打开变失败；它贡献的是读盘 + 解压的体量。
fn write_large_container(dir: &Path, name: &str) -> (PathBuf, u64, String) {
    let project = yeban_model::samples::project_with_notes(NOTE_COUNT);
    let expected_fingerprint = project_fingerprint(&project).expect("夹具工程指纹");
    // T-FIX-330：`filled_project` 的音频片段引用运行期合成的 kick ⇒ 严格核对要求
    // 夹具带上**同一份**资产字节，否则打开会正确地报 `MissingAsset`。
    let mut assets = yeban_model::samples::filled_project_assets();
    for i in 0..ASSET_CHUNKS {
        let bytes = random_bytes(ASSET_CHUNK_BYTES, 0xC10C + i as u64);
        assets.push((yeban_model::ids::AssetHash::of_bytes(&bytes), bytes));
    }
    let archive = yeban_model::container::ProjectArchive {
        project,
        history_dag: random_bytes(HISTORY_BYTES, 0xD46),
        proposals: None,
        assets,
        embedded_presets: Vec::new(),
        load_notes: Vec::new(),
    };
    let path = dir.join(name);
    yeban_app::save::save_archive_file(&archive, &path).expect("写出一份真容器");
    let len = std::fs::metadata(&path).expect("夹具文件").len();
    (path, len, expected_fingerprint)
}

/// 一条**生产形态**的窗口（与 `tests/export_wav_browse.rs::window` 同款）：
/// 只装 Tier-1 无头平台 + 构造 `MainWindow`，⛔ 不起引擎、⛔ 不开撤销会话。
/// ⚠ 返回的 `Tier1Window` 必须被持有到判据结束（Slint 平台是每线程一份）。
fn window(initial: &yeban_model::YebanProjectV1) -> (Tier1Window, MainWindow) {
    let tier1 = Tier1Window::install(Size::new(1440, 900)).expect("装 Tier-1 无头平台");
    let view = ViewState::from_project(initial).expect("投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window_with_console_tab(&view, &scene, 1).expect("生产窗口构造入口");
    (tier1, ui)
}

/// 把「打开工程」接到**本单的唯一实现**上（`PortOpenProject`，`production: None` ⇒
/// 引擎换代那一步跳过 —— 本判据测的是"读 / 解析在哪条线程"，换代由既有的
/// `open_project_ui` 覆盖；`after` 是空装配）。
/// 返回 `(跨线程状态, 运行时)` —— 判据用状态做**确定性**排空（join + apply），
/// 用运行时做那一次 `apply_pending_open_project`。
fn wire(
    ui: &MainWindow,
    picker: &Rc<ScriptedPicker>,
    port: &Rc<UndoPort>,
) -> (host::OpenProjectLoadState, Rc<dyn host::OpenProjectRuntime>) {
    let owned: Rc<ScriptedPicker> = Rc::clone(picker);
    // 两步：先克隆**具体**类型，再让 unsize 强制转换把它交给 `Rc<dyn FilePicker>`
    // （一步写会让 `Rc::clone` 的参数类型被期望类型反过来定成 trait 对象）。
    let picker: Rc<dyn yeban_app::file_picker::FilePicker> = owned;
    let runtime: Rc<dyn host::OpenProjectRuntime> = Rc::new(host::PortOpenProject {
        port: Rc::clone(port),
        production: None,
        quanta: 0,
        after: Rc::new(|_, _, _| Ok(())),
    });
    let state = host::wire_open_project(ui, picker, Rc::clone(&runtime), false);
    (state, runtime)
}

/// 界面此刻的轨道行数（`[UI-TEST-001]` 的语义属性；⛔ 不数像素）。
fn rows(window: &MainWindow) -> usize {
    use slint::Model as _;
    window.get_track_names().iter().count()
}

#[test]
fn opening_a_large_project_loads_off_the_ui_thread() {
    let _env_guard = ENV_LOCK.lock().expect("ENV_LOCK");
    let scratch = Scratch::new("load");
    let _settings = SettingsEnv::set(&scratch.join("settings.json"));
    let initial = yeban_model::samples::default_project();
    let (_tier1, ui) = window(&initial);
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<open-project-bg-load>",
            "yeban-app",
            initial.clone(),
            NOW_MS,
        )
        .expect("撤销会话必须能打开"),
    ));
    let rows_before = rows(&ui);
    let (path, file_bytes, expected_fingerprint) =
        write_large_container(&scratch.dir, "large.yeban");
    let picker = ScriptedPicker::answering_open(path.clone());
    let (state, runtime) = wire(&ui, &picker, &port);
    let ui_thread = std::thread::current().id();

    let started = std::time::Instant::now();
    ui.invoke_open_project();
    // 触发必须**立刻**返回、加载必须还在飞 —— 否则夹具太小，"不在 UI 线程上"
    // 这句话无从验证（不是 flaky 红，是响亮的"夹具太小"）。
    assert!(
        state.is_in_flight(),
        "夹具太小：加载在 invoke_open_project() 返回前就跑完了（load 必须慢到能被观测到）"
    );
    assert!(
        ui.get_open_project_loading(),
        "触发后加载行必须立刻亮起（中间态在 spawn 之前同步写）"
    );
    // ② UI 在加载中途没有冻住：一次属性往返（`playhead-ticks` 加载路径碰都不碰它）。
    ui.set_playhead_ticks(123456);
    assert_eq!(
        ui.get_playhead_ticks(),
        123456,
        "加载在飞时 UI 线程必须照常响应（属性往返）"
    );
    // 确定性排空：join 工作线程（它里面的 `upgrade_in_event_loop` 在 Tier-1 上
    // 返回 Err —— 不是错误，结果仍在槽里），再在 UI 线程上落结果。
    assert!(
        host::wait_for_pending_open_project(&state),
        "必须真的等到一个工作线程（join）"
    );
    let progress = state.progress();
    assert_eq!(
        progress.file_bytes,
        Some(file_bytes),
        "进度里读到的文件字节数必须等于夹具文件的真实字节数"
    );
    let load_thread = progress.load_thread.expect("进度里必须记下加载线程");
    assert_ne!(
        load_thread, ui_thread,
        "读 / 解析必须不在 UI 线程上（反向腿：把 stepwise 调回回调里，这里就是 UI 线程 ⇒ 红）"
    );
    assert!(
        host::apply_pending_open_project(&ui, &state, &runtime),
        "排空必须真的落了一个结果"
    );
    // 实测耗时只打出来、不做断言（盒速会变；可观测性由上面的 `is_in_flight` 钉住）。
    eprintln!(
        "[yeban-app] T-FIX-428 判据：夹具 {file_bytes} 字节，后台加载耗时 {:.0} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    // 工程真的换成了夹具那一份（指纹 + 行数），加载行收掉，回执是 ok。
    assert_eq!(
        port.fingerprint().as_deref(),
        Some(expected_fingerprint.as_str()),
        "排空后端口里的工程必须是指纹对得上的夹具工程"
    );
    assert!(
        rows(&ui) > rows_before,
        "排空后界面轨道行数必须跟着新工程变多（空工程 {} 行 → 夹具工程 {} 行）",
        rows_before,
        rows(&ui)
    );
    assert!(!ui.get_open_project_loading(), "排空后加载行必须收掉");
    assert_eq!(ui.get_open_project_progress(), 1.0, "成功排空后进度写 1.0");
    let receipt = ui.get_module_status().to_string();
    assert!(
        receipt.contains("outcome=ok"),
        "回执必须是 outcome=ok，实际：{receipt}"
    );
    assert!(
        receipt.contains(&format!("path={}", path.display())),
        "回执必须带上打开的路径，实际：{receipt}"
    );
}

#[test]
fn cancelling_a_large_project_open_leaves_the_current_project_untouched() {
    let _env_guard = ENV_LOCK.lock().expect("ENV_LOCK");
    let scratch = Scratch::new("cancel");
    let _settings = SettingsEnv::set(&scratch.join("settings.json"));
    let initial = yeban_model::samples::default_project();
    let (_tier1, ui) = window(&initial);
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<open-project-bg-cancel>",
            "yeban-app",
            initial.clone(),
            NOW_MS,
        )
        .expect("撤销会话必须能打开"),
    ));
    let before = port.fingerprint().expect("初始工程指纹");
    let (path, _file_bytes, _expected) = write_large_container(&scratch.dir, "large.yeban");
    let picker = ScriptedPicker::answering_open(path);
    let (state, runtime) = wire(&ui, &picker, &port);

    ui.invoke_open_project();
    assert!(
        state.is_in_flight(),
        "夹具太小：加载在 invoke_open_project() 返回前就跑完了"
    );
    assert!(
        ui.get_open_project_loading(),
        "触发后加载行必须立刻亮起（中间态在 spawn 之前同步写）"
    );
    // 用户点了加载行的取消按钮（这里直接调 `MainWindow` 的回调 —— 与按钮调的是
    // **同一个** `cancel-open-project`；按钮本身住在 `if false` 的控制台树里，
    // 活窗口里按不到它，见文件头 ③）。
    ui.invoke_cancel_open_project();
    assert!(
        host::wait_for_pending_open_project(&state),
        "必须真的等到一个工作线程（join）"
    );
    assert!(
        host::apply_pending_open_project(&ui, &state, &runtime),
        "排空必须真的落了一个结果"
    );
    // 工程一位不动，回执是 cancelled（⛔ 不是 error），加载行收掉。
    assert_eq!(
        port.fingerprint().as_deref(),
        Some(before.as_str()),
        "取消后端口里的工程必须一位不动"
    );
    assert!(!ui.get_open_project_loading(), "排空后加载行必须收掉");
    let receipt = ui.get_module_status().to_string();
    assert!(
        receipt.contains("outcome=cancelled"),
        "取消的回执必须是 outcome=cancelled（⛔ 不是 outcome=error），实际：{receipt}"
    );
}
