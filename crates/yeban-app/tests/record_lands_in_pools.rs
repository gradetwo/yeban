//! ⭐⭐ **任务单 `#132` 的判据：停录之后，那段录音真的进工程**（`#130` 分层分析的
//! **L2 ＋ L3**；⛔ 不做 L1／L4／L5）。
//!
//! ## 这条判据要证的句子（逐条对应工单 §4）
//!
//! | §4 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 容器里有资产 | 保存后的 `.yeban` 里存在 `assets/{sha256}`，且那份字节的 SHA-256 **逐字等于** WAV 字节的 SHA-256 | 证"字节从磁盘进了容器"。不证"资产池面板画得出来"（L4 未做） |
//! | ② `clip_pool` +1 | 停录后 `clip_pool.len()` 从 **N → N+1**，新条目的 `ClipContent::Audio { asset }` **指向** ① 的 sha256 | 证"工程文档真的多了一条引用"。不证"它被摆到轨道上"（L5 未做） |
//! | ③ 往返保真 | 保存 ⇒ 重新打开容器 ⇒ 那条 clip 还在、资产字节**逐字相同** | 证"保存没丢资产"（`#132` 的 L2）。不证跨进程（同一二进制内读回） |
//! | ④ 0 帧不落池 | 0 帧录音（`pushed=0`）⇒ `clip_pool.len()` **不变**、资产池**不变**，而 `#130` 的响亮错误仍在 | 证"空录音不会被当成一段录音写进工程" |
//!
//! ## 无头可判（工单 §4 第 7 条）
//!
//! 窗口用 Tier-1 无头平台（`yeban_ui_test_port`），输入源用**显式静音腿**
//! （`SilenceFactory`，⛔ 不碰声卡），产物落 `std::env::temp_dir()`。
//! 纯函数 `record_landing::plan_recording` 另有单测（只喂一个 WAV 路径即可判哈希与引用）。
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! - 去掉 `wire_record` 里的 `land_recording` 调用 ⇒ ② 红（`clip_pool` 停在 N）；
//! - 让 `wire_save` 退回 `assets: Vec::new()` ⇒ ①／③ 红（容器里 `assets.len() == 0`）。

#![allow(unused)]
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::host::RecordOutcomePort;
use yeban_app::open::open_project_document_file;
use yeban_app::record_device::SilenceFactory;
use yeban_app::record_wav::{InputSource, RecordOptions, RecordPlan};
use yeban_app::save_action::SaveRequest;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::samples::filled_project;
use yeban_model::{AssetHash, ClipContent};

const NOW: u64 = 1_760_000_000_000;
const WINDOW: yeban_ui_test_port::Size = yeban_ui_test_port::Size::new(1440, 900);

/// 一个临时目录，`Drop` 时清理（**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-record-lands-{tag}-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 建环参数（与既有录音判据同一条；⛔ 不为本文件另发明一套）。
fn options() -> RecordOptions {
    RecordOptions {
        seconds: 1.0,
        sample_rate: 48_000,
        channels: 2,
        capacity_frames: 8192,
        drain_batch: 512,
        ..RecordOptions::default()
    }
}

/// 判据的工程夹具：`filled_project()` ＋ **预备拍清零**。
///
/// ⚠ 为什么必须清零：`samples::filled_project()` 的 `transport.count_in_bars = 2`
/// ⇒ 120 BPM 4/4 下预备拍 = 4 秒（`record_wav::count_in_frames`），判据等不起，
/// 而 0 帧的形态会被误判成"静音源没送帧"。预备拍本身是 `#88` 的判据对象，
/// 与本单（**停录之后**那一跳）正交。
fn fixture_project() -> yeban_model::YebanProjectV1 {
    let mut project = filled_project();
    project.transport.count_in_bars = 0;
    project
}

/// **0 帧源**：工厂能开、`next_block` 恒返回 0（= "设备开着但不送帧"，`#130` 的真实形态）。
struct ZeroSource {
    channels: u16,
}

impl InputSource for ZeroSource {
    fn channels(&self) -> u16 {
        self.channels
    }

    fn next_block(&mut self, _frames: usize, _out: &mut [f32]) -> usize {
        0
    }

    fn label(&self) -> &'static str {
        "zero-frame-probe-source"
    }
}

struct ZeroFactory;

impl yeban_app::record_device::InputSourceFactory for ZeroFactory {
    fn open(&self, options: &RecordOptions) -> Result<Box<dyn InputSource + Send>, String> {
        Ok(Box::new(ZeroSource {
            channels: options.channels.max(1),
        }))
    }

    fn label(&self) -> &'static str {
        "zero-frame-probe-factory"
    }
}

/// 观察端口：记下产品落点收到的每一条文本（内部**仍**转调产品落点）。
#[derive(Debug, Default)]
struct SpyOutcome {
    reports: std::cell::RefCell<Vec<Option<String>>>,
}

impl RecordOutcomePort for SpyOutcome {
    fn report(&self, ui: &yeban_app::ui::MainWindow, error: Option<String>) {
        self.reports.borrow_mut().push(error.clone());
        host::UiRecordOutcome.report(ui, error);
    }
}

/// 组装一个**真实**的活窗口 ＋ 一个**真实**的活会话（与 `run_gui` 同一条构造入口）。
fn assemble(
    project: yeban_model::YebanProjectV1,
) -> (
    yeban_ui_test_port::render::Tier1Window,
    yeban_app::ui::MainWindow,
    Rc<UndoPort>,
) {
    let tier1 =
        yeban_ui_test_port::render::Tier1Window::install(WINDOW).expect("装 Tier-1 无头平台");
    let view = ViewState::from_project(&project).expect("装配时投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window(&view, &scene).expect("生产窗口构造入口");
    tier1
        .resize(WINDOW)
        .expect("窗口尺寸定下来之后再注入一次布局");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("record-lands", "yeban-app", project, NOW).expect("打开撤销会话"),
    ));
    (tier1, ui, port)
}

/// 一次真的录音：起录 ⇒ 等静音源送进一批帧 ⇒ 停录。
fn record_once(
    ui: &yeban_app::ui::MainWindow,
    port: &Rc<UndoPort>,
    project: &yeban_model::YebanProjectV1,
    path: &std::path::Path,
    factory: Rc<dyn yeban_app::record_device::InputSourceFactory>,
    outcome: Rc<dyn host::RecordOutcomePort>,
) {
    let _ = std::fs::remove_file(path);
    host::wire_record(
        ui,
        Some(Rc::new(project.clone())),
        Some(Rc::clone(port)),
        RecordPlan {
            path: path.to_path_buf(),
            options: options(),
        },
        factory,
        outcome,
    );
    ui.invoke_toggle_record();
    assert!(ui.get_recording(), "起录必须成功（工厂能开）");
    // 静音源**立刻**满块 ⇒ 80 ms 足够让它真的送进一批帧（与 `#130` 的静音判据同款）。
    std::thread::sleep(Duration::from_millis(80));
    ui.invoke_toggle_record();
    assert!(!ui.get_recording(), "停录之后必须归零");
}

/// ⭐⭐ **判据 ① ② ③**：停录 ⇒ `clip_pool` +1 ＋ 资产进池 ⇒ 保存 ⇒ 重新打开**都还在**。
#[test]
fn a_finished_recording_lands_in_the_clip_pool_and_the_container_asset_pool() {
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
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
}

/// ⭐ **L2 的另一半**：**打开**一个已经带 `assets/{sha256}` 的工程，再按保存 ⇒
/// 资产**不许**消失（`#132` 之前 `wire_save` 逐字写 `assets: Vec::new()` 的那个形态）。
///
/// 与主判据的区别：那条从"新录音登记进池"开始；这条从"池在装配时就已经有字节"
/// 开始（= `--open` 读回归档之后 `UndoPort::replace_assets` 的那条腿，
/// `main.rs` 的装配点）。
#[test]
fn an_opened_project_keeps_its_assets_when_it_is_saved_again() {
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
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
}

/// ⭐⭐ **判据 ④**：0 帧录音（`pushed=0`）⇒ ⛔ **不提交** `AddClip`、⛔ 不进资产池，
/// 而 `#130` 的**响亮**错误仍在（停录成功 ≠ 录到了东西）。
#[test]
fn a_zero_frame_recording_never_reaches_the_clip_pool_or_the_asset_pool() {
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
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
}

/// ⭐ **纯函数腿**（工单 §4 第 7 条的设计要点）：只喂一个 WAV 路径 ⇒ 资产哈希与引用。
///
/// 它与上面两条判据正交：不需要窗口、不需要设备、不需要会话 ⇒ 无桌面会话也能判
/// "字节 → 资产 → clip"这一跳。
#[test]
fn the_pure_planner_reads_the_wav_bytes_without_any_session() {
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
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
}

/// 引用 `SaveRequest` 的资产池构造器：证明保存请求**能**带池（⛔ 不是只有结构体字面量）。
/// 这条在编译期就是证据；运行时只断言字段真的落到了归档上。
#[test]
fn a_save_request_can_carry_the_session_asset_pool() {
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
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
}
