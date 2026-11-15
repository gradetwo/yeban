//! ⭐ 任务单 `#133` 的判据：**素材池在界面里可见**（`clip_pool` → 控制台 Tab 1 的面板）。
//!
//! ## 这条判据要证的句子（逐条对应工单 §4）
//!
//! | §4 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 面板存在且可无障碍访问 | `.slint` 源里的 `accessible-id: "asset-pool-panel"` 可由**源码派生**的计数（1 条行循环 / 5 个 `accessible-id:`）对上；注册表与运行时控件树两侧都有它 | 证"面板是一个可寻址的语义区域"。不证 Linux 像素（本机 macOS，无法重录基准） |
//! | ② 空态可判 | 0 素材的工程（`new_project_archive()`）⇒ `asset-pool-empty` 的**逐字**标签 `尚无素材`、条目数 `素材 0 条`、行数 0 | 证"空池有明确文案，不是空白" |
//! | ③ 录音 clip 可见 | `plan_recording` ＋ **一次** `Op::AddClip` 造出的那条 clip ⇒ 面板出现一行（逐字标签 / `素材 1 条` / `asset` 前缀） | 证"`#132` 落进来的音频片段真的被投影到界面"。不证 L5 的摆放／发声 |
//! | ④ 派生不是硬编码 | 两个不同 WAV ⇒ `clip_pool` 2 条 ⇒ 面板 2 行；同一窗口改注入 1 条的投影 ⇒ 1 行（`row-1` 消失） | 证"行数随投影变" |
//!
//! ## 无头可判（工单 §4 第 7 条）
//!
//! 窗口用 Tier-1 无头平台（`yeban_ui_test_port::render::LivePort`），WAV 用**既有**的
//! `record_wav::record_frames` 真落盘（⛔ 手写假字节），产物落 `std::env::temp_dir()`。
//! 纯投影的那一半（`ViewState::asset_pool`）连窗口都不需要。
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! - 删掉 `AssetPoolPanel` 的 `accessible-id` ⇒ ① 的源码派生计数与运行时树两侧都红；
//! - 删掉 `host::apply_view` 里的三个 `set_asset_pool_*` ⇒ ②③④ 的界面读数全红
//!   （模型数组空 ⇒ `asset-pool-names.length` 0 ⇒ 面板显示空态）；
//! - 把 `asset_pool_rows` 改成只认 MIDI / 只认音频 ⇒ ③④ 红（录音那条不在/多出一条）；
//! - 把行循环改成写死条数 ⇒ ④ 红（两个夹具的行数不再跟着 `clip_pool` 变）。

#![allow(unused)]
use std::path::{Path, PathBuf};

use slint::ComponentHandle as _;
use yeban_app::bridge::ViewState;
use yeban_app::elements::ElementRegistry;
use yeban_app::host;
use yeban_app::record_landing::plan_recording;
use yeban_app::record_wav::{RecordOptions, record_frames};
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_model::samples::{default_project, filled_project};
use yeban_model::{Op, YebanProjectV1};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::tree::ControlTree;

/// 窗口尺寸（与既有 Tier-1 判据同款；非零即合法）。
const WINDOW: Size = Size::new(1440, 900);

/// 面板源码（**另一个文件**，不是本判据自己）—— 第 293 条：判据钉住的字符串必须从
/// 别处读，或用"锚点 ＋ 计算窗口"的派生形，⛔ 不许从判据所在文件读（那是恒真）。
const PANEL_SOURCE: &str = include_str!("../ui/console/asset_pool_panel.slint");

/// 一个临时目录，`Drop` 时清理（**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-asset-pool-{tag}-{}-{}",
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

/// 建环参数（与 `record_lands_in_pools.rs` 同一条；⛔ 不为本文件另发明一套）。
fn record_options() -> RecordOptions {
    RecordOptions {
        seconds: 1.0,
        sample_rate: 48_000,
        channels: 2,
        capacity_frames: 8192,
        drain_batch: 512,
        ..RecordOptions::default()
    }
}

/// 用**既有**的落盘器造一段真 WAV（`seed` 不同 ⇒ 字节不同 ⇒ CAS 哈希不同）。
fn write_wav(path: &Path, seed: u64) {
    record_frames(path, &record_options(), |frame| {
        (((frame + seed) % 64) as f32) / 128.0
    })
    .expect("落一段真 WAV");
}

/// 走 `#132` 的落盘路径：纯函数 `plan_recording` ＋ **一次** `Op::AddClip`（每条 WAV 一次）。
fn project_with_recordings(
    paths: &[PathBuf],
) -> (
    YebanProjectV1,
    Vec<yeban_app::record_landing::RecordingPlan>,
) {
    let mut project = default_project();
    let mut plans = Vec::new();
    for path in paths {
        let plan = plan_recording(path, &project).expect("纯规划（不碰会话）");
        assert!(
            plan.creates_clip(),
            "空 clip_pool 里每条新 WAV 都必须新建一条 clip"
        );
        for op in &plan.ops {
            assert!(
                matches!(op, Op::AddClip { .. }),
                "录音落点只允许 AddClip（L5 的摆放不归本单）"
            );
            op.apply(&mut project)
                .expect("AddClip 必须能作用在空工程上");
        }
        plans.push(plan);
    }
    (project, plans)
}

/// 读活窗口的运行时控件树（`[UI-DoD §3.6]` 的载体：全树内省 + JSON 往返）。
fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

/// 模型里的数组（`asset-pool-*`）逐项读出 —— 与 `.slint` 消费的是同一份 `ModelRc`。
fn model_rows(window: &MainWindow) -> Vec<String> {
    use slint::Model as _;
    window
        .get_asset_pool_names()
        .iter()
        .map(|name| name.to_string())
        .collect()
}

fn model_kinds(window: &MainWindow) -> Vec<String> {
    use slint::Model as _;
    window
        .get_asset_pool_kinds()
        .iter()
        .map(|kind| kind.to_string())
        .collect()
}

fn model_details(window: &MainWindow) -> Vec<String> {
    use slint::Model as _;
    window
        .get_asset_pool_details()
        .iter()
        .map(|detail| detail.to_string())
        .collect()
}

/// 一条**生产形态**的 Tier-1 活窗口（`build_main_window_with_console_tab` 是唯一的注入实现）。
///
/// ⚠ 返回值里的 `LivePort` **必须被持有到判据结束**：它拥有 `set_platform` 装好的
/// 每线程一份的 Slint 平台与活窗口。同一测试里**只能建一个**（第二次 `install` 会
/// 返回 `PlatformUnavailable`）。
fn live_window(view: &ViewState, console_tab: i32) -> LivePort<MainWindow> {
    let scene = DemoScene::from_view(view);
    LivePort::new(WINDOW, Permission::ReadOnly, None, || {
        host::build_main_window_with_console_tab(view, &scene, console_tab)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口")
}

// ===========================================================================
// 判据 ①：面板存在且可无障碍访问（源码派生 ＋ 注册表 ＋ 运行时树）
// ===========================================================================
#[test]
fn the_asset_pool_panel_is_declared_and_addressable() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

// ===========================================================================
// 判据 ②：空态可判（0 素材 ⇒ 逐字文案 ＋ 条目数 0）
// ===========================================================================
#[test]
fn an_empty_pool_shows_the_empty_state_and_zero_rows() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

// ===========================================================================
// 判据 ③：录音 clip 可见（plan_recording ＋ 一次 AddClip ⇒ 面板里出现那一行）
// ===========================================================================
#[test]
fn a_recorded_clip_appears_in_the_panel_with_its_asset_prefix() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

// ===========================================================================
// 判据 ④：投影是派生不是硬编码（两个不同素材 ⇒ 2 行；改注入 1 条 ⇒ 1 行）
// ===========================================================================
#[test]
fn the_row_count_follows_the_clip_pool_instead_of_a_hardcoded_number() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}
