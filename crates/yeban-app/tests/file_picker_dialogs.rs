//! 判据（任务单 `#86` **乙**）：**原生文件对话框**接三处 —— 打开工程 / 保存工程 / 选 SFZ 路径。
//!
//! ## 本单补的是哪一格
//!
//! 勘察（本单实测）：全仓 `rfd` / `native-dialog` / `FileDialog` 命中 **0** ⇒
//! "打开 / 保存工程、选 SFZ"今天只能靠 **CLI** 或混音台的**文本框** —— 用户没有
//! 系统级的选择器可点。本单引入 `rfd`，并把"挑一个路径"抽成**可注入**的能力面
//! （`yeban_app::file_picker::FilePicker`）。
//!
//! ## ⭐ 判据能测到哪一步（**⛔ 不声称测过弹窗**）
//!
//! 真弹窗在 CI 上起不来（没有窗口服务器；就算有，它会**阻塞在用户输入上**）。
//! 因此本文件注入一个**假实现**（[`ScriptedPicker`]），它：
//!
//! * **记下**每一次请求（⇒ 判据能断言"**对话框被调用了**"，连标题与后缀过滤一起）；
//! * 按脚本回一个路径（⇒ 判据能断言"**选了路径 ⇒ 模型被提交**"）。
//!
//! ⛔ **本文件没有验证**：`rfd` 真弹窗的外观、平台行为、用户按取消时的系统语义。
//! 那一层是 [`yeban_app::file_picker::NativeFilePicker`] 的边界（见那个模块的文档第 3 条）。
//! ⇒ 这条如实登记为**未被自动判据覆盖**的部分（需要真人手工验证，本单不做）。
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `calls` | 假选择器收到的请求数 | 次 |
//! | `request.title` / `.extensions` / `.file_name` | 最近一次请求的三个字段 | 字符串 / 条 |
//! | 输入框 `value` | 运行时控件树里 `track-{i}-sampler-path` 的 `accessible-value` | 字符串（路径或空串） |
//! | `sampler.path` | 权威工程里该轨的 `TrackV3::sampler` | 字符串 / `None` |
//! | `undoable` | `UndoPort::display().undoable`（可撤销步数） | 步 |
//! | `status` | `MainWindow.save-status` 原文 | 字符串 |
//! | 磁盘上的文件 | `target` 处存在且可被 `open_project_file` 读回 | 布尔 |
//!
//! ## 注入（每一条都由本文件的判据打红；读数贴在本单交付报告里）
//!
//! * **① 保存腿不接线**（把 `wire_save` 里那次 `picker.choose_save(..)` 换成
//!   `None`，即"没有目标路径就直接放弃"）⇒
//!   [`saving_without_an_assembly_time_target_asks_the_picker_and_writes_there`] 红
//!   （`calls == 0` 且磁盘上没有那个文件）。
//! * **③ 另存为不接线**（`wire_save` 里不注册 `on_save_project_as`）⇒
//!   [`saving_as_asks_the_picker_and_retargets_subsequent_saves`] 红（`calls == 0`、
//!   新文件不存在）；把「另存为」做成"装配期目标优先"⇒ 该判据第二段红（保存写回旧文件）。
//! * **② SFZ 浏览不接线**（把 `wire_mixer_sampler_browse` 的回调体清空）⇒
//!   [`browsing_for_an_sfz_fills_the_text_box_and_only_set_commits_it`] 红
//!   （输入框的 `value` 仍是空串）。
//! * **应绿腿**：[`the_text_box_and_the_cli_path_stay_usable_without_any_dialog`]
//!   在 ①② 两种注入下**必须照旧绿** —— 它断言"手敲文本 ⇒ SET 仍然提交"与
//!   "`NoPicker` ⇒ 一次都不弹"。注入 ①② 都动不了这两条（它们不经过对话框）。

#![allow(unused)]
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;

use yeban_app::bridge::ViewState;
use yeban_app::file_picker::{FilePicker, NoPicker, PickerRequest};
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::{EntityId, SamplerBinding, YebanProjectV1};
use yeban_ui_test_port::inspect::{find_by_accessible_id, tree_from_element_root};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的装配入口与打开入口 ⇒ 其余项是 dead_code
// （`-D warnings` 会判红）。同款见 `tests/sfz_ui_selector.rs`。
#[allow(dead_code)]
mod live;

use live::{LiveUi, LiveWiringOptions, OpenViaPicker, build_live_ui_with};

#[path = "common/mod.rs"]
mod common;

/// ⭐ **假的选择器**（`#86` 乙）住在 `tests/common/mod.rs`：任务单 `#93` 的
/// `tests/open_project_ui.rs` 断的是**同一个**替身（"对话框被问了几次"这类读数不允许有两份）。
use common::ScriptedPicker;

/// 撤销会话的"现在"（与 `sfz_ui_selector.rs` 的常量同款：判据不需要墙钟）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 临时目录（形态照 `sfz_ui_selector.rs` 的 `scratch`）。
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-file-picker-{tag}-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 进程级环境状态的串行锁（`YEBAN_SAMPLE_LIBRARY`；与 `sample_library.rs` 同一条纪律：
/// 本文件内持锁的判据才碰这个变量；其余判据要么没有采样器绑定、要么存绝对路径
/// —— 规则 1 不查库根，所以它们读不到这个变量）。
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// `YEBAN_SAMPLE_LIBRARY` 的 RAII 守卫（`Drop` 时恢复原值）。
///
/// SAFETY（Rust 2024 的 `set_var` 是 `unsafe`）：本文件的全部 env 判据都先拿 [`ENV_LOCK`]，
/// 且调用方持有锁的期间本测试二进制里没有其它线程读这个变量；`Drop` 在同一把锁内恢复。
struct LibraryEnv {
    previous: Option<OsString>,
}

impl LibraryEnv {
    /// ⚠ 调用方**必须**先持有 [`ENV_LOCK`]（本函数不再锁一次：那是同一把非重入锁）。
    fn set(dir: &Path) -> Self {
        let previous = std::env::var_os("YEBAN_SAMPLE_LIBRARY");
        // SAFETY: 调用方持有 ENV_LOCK；本测试进程内没有其它线程读这个变量。
        unsafe {
            std::env::set_var("YEBAN_SAMPLE_LIBRARY", dir);
        }
        Self { previous }
    }
}

impl Drop for LibraryEnv {
    fn drop(&mut self) {
        // SAFETY: 守卫比调用方的 ENV_LOCK 先析构（同一测试内后声明者先 drop），
        // 且恢复的是进入时读到的原值。
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var("YEBAN_SAMPLE_LIBRARY", value),
                None => std::env::remove_var("YEBAN_SAMPLE_LIBRARY"),
            }
        }
    }
}

/// 工程里那条**带 MIDI 音符**的非主总线轨（与 `sfz_ui_selector.rs` 同一份口径）。
fn midi_track_of(project: &YebanProjectV1) -> EntityId {
    *project
        .tracks
        .iter()
        .find(|(id, track)| {
            **id != project.master_bus_track_id
                && track.clips.values().any(|placement| {
                    project
                        .clip_pool
                        .get(&placement.clip_id)
                        .is_some_and(|entry| entry.content.notes().is_some())
                })
        })
        .map(|(id, _)| id)
        .expect("filled_project 必须有一条带 MIDI 音符的非主总线轨")
}

/// 写一份"每个工程音高一条区域"的 SFZ（`sample=` 用相对路径 ⇒ 按 SFZ 目录解析）。
fn write_sfz(dir: &Path, name: &str, pitches: &[u8], sample: &str) -> PathBuf {
    let mut text = String::new();
    for pitch in pitches {
        text.push_str(&format!(
            "<region> sample={sample} key={pitch} pitch_keycenter={pitch}\n"
        ));
    }
    let path = dir.join(name);
    std::fs::write(&path, text).expect("写 SFZ 夹具");
    path
}

fn pitches_of(project: &YebanProjectV1, track_id: EntityId) -> Vec<u8> {
    let mut pitches: Vec<u8> = project
        .tracks
        .get(&track_id)
        .expect("轨必须在")
        .clips
        .values()
        .filter_map(|placement| project.clip_pool.get(&placement.clip_id))
        .filter_map(|entry| entry.content.notes())
        .flat_map(|notes| notes.values().map(|note| note.pitch))
        .collect();
    pitches.sort_unstable();
    pitches.dedup();
    pitches
}

/// **没有撤销端口**的活窗口（`ui/open_project` 只在这一种装配下能就地换工程）。
///
/// 为什么必须没有端口（⛔ 不是本单的取舍，是本仓既有的口径，逐字引自拒绝文案）：
/// `UndoPort` **自己持有一份工程**，挂着一个外部端口时换工程会让它把旧工程写回引擎
/// ⇒ `open_project_now` 如实拒绝"换一半"。产品形态里对应的是
/// `--enable-ui-mcp-http` 那一种（`LiveWiringOptions::undo = None`）。
fn live_window_without_undo_port(permission: Permission) -> (LiveUi, yeban_app::ui::MainWindow) {
    let project = yeban_model::samples::filled_project();
    // ⚠ 这里**不**先自己投影：`build_live_ui_with` 内部就做那件事（同款见
    // `sfz_ui_selector.rs` 的 `Fixture::new` —— 它只为取 `lead_index` 才投影）。
    let live = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission,
            console_tab: 1,
            save_path: None,
            engine_quanta: 0,
            undo: None,
            proposal_source: yeban_app::proposals::ProposalSource::none(),
        },
    )
    .expect("装配（无撤销端口的活窗口）");
    let window = slint::ComponentHandle::clone_strong(live.ui());
    (live, window)
}

/// 一条**生产形态**的窗口（`build_main_window_with_console_tab` 的三个参数由本函数配齐）
/// ＋ 它对应的那份工程。
///
/// 为什么不用 `Fixture`：本判据里"保存 / 取消 / NoPicker"那几条**不需要** Tier-1 执行面
/// 与引擎 —— 它们测的是 `wire_save` 的回调体（纯宿主逻辑）。少装一代引擎也让这几条判据
/// 快得多，且⛔ 不会把"引擎起没起来"混进"对话框接没接上"。
/// ⚠ 返回值里的 `Tier1Window` **必须被持有到本判据结束**：`install` 装的是**每线程一份**
/// 的 Slint 平台，丢了它窗口就没了后端（与 `gui_save_writes_live_session.rs` 同款）。
fn production_window() -> (
    yeban_ui_test_port::render::Tier1Window,
    yeban_app::ui::MainWindow,
    YebanProjectV1,
) {
    let tier1 =
        yeban_ui_test_port::render::Tier1Window::install(yeban_ui_test_port::Size::new(1440, 900))
            .expect("装 Tier-1 无头平台");
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    let scene = yeban_app::scene::DemoScene::from_view(&view);
    let ui = yeban_app::host::build_main_window_with_console_tab(&view, &scene, 1)
        .expect("生产窗口构造入口");
    (tier1, ui, project)
}

/// 活窗口 ＋ 撤销端口 ＋ 一代引擎的夹具（与 `sfz_ui_selector.rs` 的 `Fixture` 同一形状）。
struct Fixture {
    /// ⚠ 本字段**从没被读过**，但**必须持有**：它拥有那条装配好的执行面（窗口 ＋
    /// 引擎 ＋ 电平消费端）。丢掉它 = 把这一代执行面在判据中途拆掉。
    /// `#[allow(dead_code)]` 因此是"为 Drop 而留"的显式登记，⛔ 不是藏死代码。
    #[allow(dead_code)]
    live: LiveUi,
    window: yeban_app::ui::MainWindow,
    port: Rc<UndoPort>,
    lead: EntityId,
    lead_index: usize,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        Self::new_with_permission(tag, Permission::Interactive)
    }

    /// ⚠ `ui/open_project` 要求 **`Administrative`**（`[UI-MCP-001]` 的三级分层），
    /// 而其余判据用 `Interactive` 就够 ⇒ 权限因此是一个参数，⛔ 不是写死的一档。
    fn new_with_permission(tag: &str, permission: Permission) -> Self {
        let project = yeban_model::samples::filled_project();
        let lead = midi_track_of(&project);
        let view = ViewState::from_project(&project).expect("投影");
        let lead_index = view
            .tracks
            .iter()
            .position(|track| track.id == lead.to_canonical_string())
            .unwrap_or_else(|| panic!("投影里必须有身份 {} 的轨道", lead.to_canonical_string()));
        let port = Rc::new(UndoPort::new(
            UndoSession::open(
                format!("<判据 {tag}>"),
                "yeban-app",
                project.clone(),
                NOW_MS,
            )
            .expect("打开撤销会话"),
        ));
        let mut live = build_live_ui_with(
            &project,
            &LiveWiringOptions {
                permission,
                console_tab: 1,
                save_path: None,
                engine_quanta: 0,
                undo: Some(Rc::clone(&port)),
                proposal_source: yeban_app::proposals::ProposalSource::none(),
            },
        )
        .expect("装配（真窗口 ＋ Tier-1 执行面）");
        live.start_engine().expect("起一代引擎");
        let window = slint::ComponentHandle::clone_strong(live.ui());
        Self {
            live,
            window,
            port,
            lead,
            lead_index,
        }
    }

    fn path_id(&self) -> String {
        format!("track-{}-sampler-path", self.lead_index)
    }

    fn set_id(&self) -> String {
        format!("track-{}-sampler-set", self.lead_index)
    }

    fn browse_id(&self) -> String {
        format!("track-{}-sampler-browse", self.lead_index)
    }

    /// 运行时控件树里某个 ID 的 `accessible-value` 原文（不在树里 ⇒ `None`）。
    fn value_of(&self, id: &str) -> Option<String> {
        let tree = tree_from_element_root(&self.window, None).expect("运行时控件树");
        tree.find_by_id(id).and_then(|node| node.value.clone())
    }

    /// 在输入框里"敲"一段文本（`[UI-TEST-002]` §12.4 的 `SetValue` 无障碍动作）。
    fn type_path(&self, text: &str) {
        let handle = find_by_accessible_id(&self.window, &self.path_id())
            .unwrap_or_else(|| panic!("控件树里没有 `{}`", self.path_id()));
        handle.set_accessible_value(text);
    }

    /// 触发一个按钮的**默认无障碍动作**（与鼠标点击是逐字同一段落点）。
    fn press(&self, id: &str) {
        let handle = find_by_accessible_id(&self.window, id)
            .unwrap_or_else(|| panic!("控件树里没有 `{id}`"));
        handle.invoke_accessible_default_action();
    }

    fn sampler_of_lead(&self) -> Option<SamplerBinding> {
        self.port
            .try_project()
            .expect("此刻必须有活跃工程")
            .tracks
            .get(&self.lead)
            .expect("轨必须在")
            .sampler
            .clone()
    }
}

/// ⭐ 判据一（**② 保存工程**）：装配期**没有**目标路径时，保存必须**问一次对话框**，
/// 并把选中的路径真的写出去。
///
/// # 判定量
///
/// * 按下 `save-project` 之前：`calls == 0`（⛔ 没按就不该弹）；
/// * 按下之后：`choose_save` 被调用**恰一次**，请求的 `title == "保存工程"`、
///   `file_name == Some("project.yeban")`、`extensions == ["yeban"]`；
/// * `save-status` 是**成功**文案（`save-succeeded == true`）；
/// * **磁盘上真的出现**那个文件，且能被 `yeban_app::open::open_project_file` 读回。
///
/// # 怎么变红
///
/// * **注入 ①**（`wire_save` 里不调 `picker.choose_save`）⇒ `calls == 0`、
///   磁盘上什么都没有 ⇒ 本判据红；
/// * 把 `.slint` 的 `save-project` 回调摘掉 ⇒ `calls == 0`。
#[test]
fn saving_without_an_assembly_time_target_asks_the_picker_and_writes_there() {
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
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("callback save-project-as("),
        "新 UI 契约面必须声明 callback save-project-as("
    );
    assert!(
        aether_host.contains("on_save_project_as"),
        "宿主接线层必须注册 on_save_project_as"
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

/// ⭐ 判据二（**取消不是失败**）：用户在对话框里按**取消** ⇒ **一位不动**。
///
/// # 判定量
///
/// * `choose_save` 被调用**恰一次**（取消也要先开一次）；
/// * `save-status` **仍是空串**（⛔ 没有被写成"保存失败"）；
/// * 目录里**没有**任何文件被创建。
#[test]
fn cancelling_the_save_dialog_writes_nothing_and_reports_nothing() {
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
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("callback save-project-as("),
        "新 UI 契约面必须声明 callback save-project-as("
    );
    assert!(
        aether_host.contains("on_save_project_as"),
        "宿主接线层必须注册 on_save_project_as"
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

/// ⭐ 判据二之 b（**另存为**，2026-10-31）：装配期**有**目标路径时按「另存为」
/// 仍然**必须**问一次对话框，并把后续「保存」**改道**到新文件。
///
/// # 判定量
///
/// * 按下另存为之前：`choose_save` 被调用 **0** 次；
/// * 按下之后：恰 **1** 次（`title == "另存为"`、`file_name == Some("project.yeban")`、
///   `extensions == ["yeban"]`）；新路径上**真的**出现了一个能读回的容器；
///   装配期那个旧文件的字节**一个都没变**（⛔ 另存为丢掉了"新位置"这件事就不叫另存为）；
/// * 随后再按「保存」（`invoke_save_project`）：对话框**仍然只有 1 次**（不重复问），
///   并且**新**路径上重新出现了文件（＝当前目标已切到新文件）；旧文件仍一位未动。
///
/// # 怎么变红
///
/// * `wire_save` 里不给 `on_save_project_as` 接线（回调空转）⇒ `calls == 0`、新文件不存在；
/// * 另存为走"装配期目标优先"⇒ 新文件不存在 / 旧文件被改写 ⇒ 第一段红；
/// * 另存为之后不把挑中的路径记成当前目标 ⇒ 第二段红（保存写回旧文件、新文件没有回来）。
#[test]
fn saving_as_asks_the_picker_and_retargets_subsequent_saves() {
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
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("callback save-project-as("),
        "新 UI 契约面必须声明 callback save-project-as("
    );
    assert!(
        aether_host.contains("on_save_project_as"),
        "宿主接线层必须注册 on_save_project_as"
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

/// ⭐ 判据三（**③ 选 SFZ 路径**）："浏览"按钮 ⇒ 一次对话框 ⇒ 选中的路径**填进输入框**；
/// 而**提交**仍然只发生在 SET（⛔ 浏览**不**改工程）。
///
/// # 判定量
///
/// * 按"浏览"之前：`calls == 0`、输入框 `value == Some("")`、`undoable == 0`；
/// * 按"浏览"之后：`choose_open` 调用**恰一次**（`title == "选择 SFZ 音色文件"`、
///   `extensions == ["sfz"]`）；输入框 `value` **逐字等于**对话框给的路径；
///   ⭐ 而 `sampler` 仍是 `None`、`undoable` 仍是 **0**（**浏览不提交**）；
/// * 再按 SET：`sampler.path == 那条路径`、`undoable == 1`（**这时才提交**）。
///
/// # 怎么变红
///
/// * **注入 ②**（`wire_mixer_sampler_browse` 的回调体清空）⇒ `calls == 0` 且
///   输入框仍是空串 ⇒ 本判据红；
/// * 把浏览做成"选完直接提交" ⇒ 中间那一段的 `undoable == 0` / `sampler == None` 红
///   （本仓要的是"浏览只填框"）。
#[test]
fn browsing_for_an_sfz_fills_the_text_box_and_only_set_commits_it() {
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
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("callback save-project-as("),
        "新 UI 契约面必须声明 callback save-project-as("
    );
    assert!(
        aether_host.contains("on_save_project_as"),
        "宿主接线层必须注册 on_save_project_as"
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

/// ⭐ 判据四（**应绿腿**，第 243 条）：**文本框与 CLI 两条路一位没变** ＋ **`NoPicker` 一个窗都不弹**。
///
/// # 为什么必须有这一条
///
/// 本单的硬要求是"⛔ 保留文本框与 CLI 路径（它们仍要可用）"。本判据把它变成读数：
///
/// * **手敲文本 ⇒ SET 仍然提交**（⛔ 不经过任何对话框）；
/// * **`NoPicker`**（产品里"装配期已给目标路径"那一档用它）⇒ `wire_save` 一次都不弹，
///   且**如实**报原有的 `NoTarget` 文案（⛔ 没有被本单改成静默）。
///
/// # 为什么它在注入 ①② 下**必须照旧绿**
///
/// 它**不经过**对话框：手敲那条路走的是 `mixer-sampler-set` 回调（注入 ② 没碰它），
/// 而 `NoPicker` 那条路由定义就是"永远 `None`"（注入 ① 改的是"问不问"，不是"回什么"）。
#[test]
fn the_text_box_and_the_cli_path_stay_usable_without_any_dialog() {
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
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("callback save-project-as("),
        "新 UI 契约面必须声明 callback save-project-as("
    );
    assert!(
        aether_host.contains("on_save_project_as"),
        "宿主接线层必须注册 on_save_project_as"
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

/// ⭐ 存入相对化（Wang 2026-11-01 定）：手敲库根下绝对路径 ＋ SET ⇒ 存入值是
/// `assets/samples/…` 相对形式（UI 路径走 `commit_sampler_binding`，与 CLI 同一条归一化）。
#[test]
fn typing_an_under_library_root_absolute_path_and_pressing_set_stores_the_relative_form() {
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
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("callback save-project-as("),
        "新 UI 契约面必须声明 callback save-project-as("
    );
    assert!(
        aether_host.contains("on_save_project_as"),
        "宿主接线层必须注册 on_save_project_as"
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

/// ⭐ 判据五（**① 打开工程**）：对话框选的路径**真的**换成活窗口上的工程。
///
/// # 它接在哪一条路上（⛔ 不编造）
///
/// 本仓**已有**的运行期打开入口是 [`LiveUi::open_project`]（控制面 `ui/open_project`
/// 的同一个落点）。本单把对话框接在它**之上** —— `open_project_via_picker` 只做两件事：
/// 问一次对话框、把路径交给**已有的**那一处。
///
/// # 判定量
///
/// * 取消 ⇒ `Cancelled`，且活窗口的工程**一条轨道都没变**（⛔ 不半途换掉）；
/// * 给一条**真的**工程文件 ⇒ `Opened { path }`，且活窗口的轨道数换成那一份的。
///
/// # 边界（如实登记）
///
/// ⚠ 本判据测的是"对话框 ⇒ **已有**的打开入口"。产品 `run_gui` 里今天**没有**运行期
/// 打开工程的实现（`--open` 只在启动时读一次），因此本单⛔ **不声称**给 GUI 加了
/// 可点的打开入口 —— 那会改默认帧（需重录 `tests/golden/linux/**`），是**另一个缺口**。
#[test]
fn opening_a_project_through_the_picker_swaps_the_live_window() {
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
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("callback save-project-as("),
        "新 UI 契约面必须声明 callback save-project-as("
    );
    assert!(
        aether_host.contains("on_save_project_as"),
        "宿主接线层必须注册 on_save_project_as"
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
