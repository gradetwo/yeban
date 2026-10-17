//! 判据（任务单 `#129`，界面腿）：GUI 里「打开工程」**旁边**有一个「新建工程」入口，
//! 点下去 ⇒ 问一次保存对话框 ⇒ 落一个 **1136 B／0 音轨 ＋ 1 主总线** 的 `.yeban`
//! ⇒ **就地换成新工程**（视图清空 / 标题目标路径跟着走）；取消不动一位；
//! **没有对话框** ⇒ 如实报错。
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | 树里有没有 `module-new-project-button` | 活窗口的运行时控件树（Tab 0 / Tab 1 各读一次） | 布尔 |
//! | `role` / `label` / `bounds` | 该节点的 `accessible-role` / `accessible-label` / 实几何 | kebab-case 角色 / 中文标签 / 像素 |
//! | `calls` | 假选择器收到的 `choose_save` 请求数 | 次 |
//! | `request.title` / `.file_name` / `.extensions` | 最近一次请求的三个字段 | 字符串 / 字符串 / 条 |
//! | 新建文件的大小 | `std::fs::metadata(path).len()` | 字节 |
//! | 工程指纹 | `UndoPort::fingerprint()`（模型容器字节的 SHA-256） | 十六进制串 |
//! | 界面的轨道行数 | `MainWindow.track-names` 的行数 | 行 |
//! | `undoable` | `UndoPort::display().undoable`（可撤销步数） | 步 |
//! | `after` 收到的视图轨道数 | `OpenProjectAfter` 第三/四个参数 | 条 |
//! | `module-status` | 宿主写的**回执行**原文（既有投影通道） | 结构化文本 |
//!
//! ## 为什么入口住在控制台 Tab 1（与 `#90` / `#93` **同一条纪律**）
//!
//! `[UI-MCP-003]` 的 5 张 Linux 基准只覆盖默认帧（Tab 0）。`MixerConsole` 只在
//! `active-tab == 1` 时可见 ⇒ 新按钮在默认帧里**一个像素都不画** ⇒ 基准逐位不变。
//!
//! ## 这条判据怎么变红（本单注入读数见交付报告）
//!
//! - `.slint` 的 `accessible-action-default` / `TouchArea.clicked` 不调 `root.new-project()`
//!   （**断开入口**）⇒ 对话框一次没被问 ⇒ 正向腿红；
//! - `host::wire_new_project` 不接回调 / 不写 `module-status` ⇒ 同上；
//! - 落盘改成 `filled_project()`（3 轨）⇒ 1136 B / `tracks=0` 两条红；
//! - **没有对话框**那一支改成静默 `return` ⇒ 反向腿红（回执为空）；
//! - 新建后**不换工程**（只写文件）⇒ 指纹 / 行数 / `after` 三条红。
//!
//! ## 平台约束：本文件只有一个 `#[test]`
//!
//! `slint::platform::set_platform` 是线程局部的，`Tier1Window::install` 每线程只能成功
//! 一次（第二次 `AlreadySet`）⇒ 同款用法见 `tests/open_project_ui.rs`。

#[path = "common/mod.rs"]
mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use common::ScriptedPicker;
use yeban_app::bridge::ViewState;
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::file_picker::{FilePicker, NoPicker, PickerRequest};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession, project_fingerprint};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::{find_by_accessible_id, tree_from_element_root};
use yeban_ui_test_port::render::{Tier1Window, report_line};
use yeban_ui_test_port::tree::ControlTree;

/// 新入口的语义 ID（`ui/console/mixer_console.slint` ↔ `src/elements.rs` 两处必须逐字相同）。
const NEW_BUTTON_ID: &str = "module-new-project-button";

/// 新入口的无障碍名称（与 `elements.rs` 登记的标签**逐字相同**，运行时对账会断言相等）。
const NEW_BUTTON_LABEL: &str =
    "新建工程（用文件对话框给新工程选一个落点，写出 0 音轨 ＋ 1 主总线的空白容器并就地换过去）";

/// 既有投影通道的落点（回执行的**可寻址**节点，住在右栏 —— 默认帧里就可见）。
const MODULE_STATUS_ID: &str = "module-last-status";

/// 撤销会话的"现在"（与 `open_project_ui.rs` 的常量同款：判据不需要墙钟）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 窗口尺寸（非零即合法；判据只要求"窗口真的建起来过"）。
const WINDOW: Size = Size::new(1440, 900);

/// 临时目录（`Drop` 时清理，**绝不污染仓库**）。形态照 `open_project_ui.rs`。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-new-project-ui-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// 目录里的**条目数**（用来证明"取消 / 无对话框"一个文件都没建）。
    fn entries(&self) -> usize {
        std::fs::read_dir(&self.dir).expect("读临时目录").count()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 读活窗口的运行时控件树（`[UI-DoD §3.6]` 的载体：全树内省 + JSON 往返）。
fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

/// 界面此刻的轨道行数（`[UI-TEST-001]` 的语义属性；⛔ 不数像素）。
fn rows(window: &MainWindow) -> usize {
    use slint::Model as _;
    window.get_track_names().iter().count()
}

/// 触发一个按钮的**默认无障碍动作**（与鼠标点击是逐字同一段落点）。
fn press(window: &MainWindow, id: &str) {
    let handle = find_by_accessible_id(window, id)
        .unwrap_or_else(|| panic!("控件树里没有 `{id}` —— 入口不在当前可见分支里（或 ID 改了）"));
    handle.invoke_accessible_default_action();
}

/// `after` 的读数：`(path, 工程轨道数, 投影轨道数)` —— 证明**打开工程那条 `after`**
/// 真的被这一次新建触发了，而且它拿到的是**新投影**（0 轨）。
type AfterReadings = Rc<RefCell<Vec<(String, usize, usize)>>>;

/// 把「新建工程」接到**本单的唯一实现**上（每次调用 = `on_new_project` setter 覆盖）。
fn wire(
    window: &MainWindow,
    picker: Rc<dyn FilePicker>,
    runtime: &Rc<dyn host::OpenProjectRuntime>,
) {
    host::wire_new_project(window, picker, Rc::clone(runtime));
}

#[test]
fn the_gui_new_project_entry_writes_an_empty_container_and_swaps_it_in() {
    let scratch = Scratch::new();
    // 当前工程 = `filled_project()`（3 条非主总线轨）⇒ "换成 0 轨"有对照。
    let current = yeban_model::samples::filled_project();
    let current_rows = ViewState::from_project(&current)
        .expect("投影")
        .tracks
        .len();
    assert!(
        current_rows > 0,
        "夹具必须是**非空**工程，否则『清空』没有对照"
    );

    // ---- 装配：**生产形态**的窗口（`run_gui` 用的同一个构造入口）＋ Tier-1 无头平台 ----
    let tier1 = Tier1Window::install(WINDOW).expect("装 Tier-1 无头平台");
    let view = ViewState::from_project(&current).expect("投影");
    let scene = DemoScene::from_view(&view);
    // ⚠ 起始 Tab = 0（默认帧）—— 与产品启动态一致；下面显式切到 Tab 1 来对照。
    let window = host::build_main_window_with_console_tab(&view, &scene, 0).expect("生产窗口");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据 #129>", "yeban-app", current.clone(), NOW_MS)
            .expect("打开撤销会话"),
    ));
    // ⭐ 先做**一次可撤销的编辑** ⇒ `undoable` 从 0 变成 >0；新建之后必须归零
    //（「打开一个工程 = **新会话**」这条既有口径同样适用于新建 —— 它是"打开刚写的文件"）。
    let fixture = yeban_app::undo::undo_session::wiring_fixture(&port.project()).expect("夹具");
    port.commit_ops(NOW_MS + 1, "判据 #129", vec![fixture.op()])
        .expect("提交");
    let undoable_before = port.display().undoable;
    assert!(
        undoable_before > 0,
        "夹具必须先制造出可撤销的历史，否则下面那条『归零』没有对照"
    );
    assert_eq!(rows(&window), current_rows, "初始投影必须已经注入界面");

    // 一代**真引擎**（当前工程有主总线 ⇒ 引擎真的起来）。
    let production = Rc::new(RefCell::new(
        host::ProductionLoop::start(&current, 0).expect("起一代引擎"),
    ));

    // `after` = 装配方**最后一跳**（产品 `run_gui` 那一个更新心跳基准 / 保存落点 /
    // 按键快照）。本判据断言的是"新建走的是**同一条**换工程路径" ⇒ 录下它收到的三样。
    let after_readings: AfterReadings = Rc::new(RefCell::new(Vec::new()));
    let after_sink = Rc::clone(&after_readings);
    let after: host::OpenProjectAfter = Rc::new(
        move |_ui: &MainWindow,
              project: &yeban_model::YebanProjectV1,
              path: &str,
              view: &yeban_app::bridge::ViewState| {
            after_sink.borrow_mut().push((
                path.to_owned(),
                project.tracks.len(),
                view.tracks.len(),
            ));
            Ok(())
        },
    );
    // ⭐ **同一个** `OpenProjectRuntime`（`#93` 的 `PortOpenProject`）—— 新建 = 写空容器
    // 再走它那条换工程路径；⛔ 没有第二个换工程实现。
    let runtime: Rc<dyn host::OpenProjectRuntime> = Rc::new(host::PortOpenProject {
        port: Rc::clone(&port),
        production: Some(Rc::clone(&production)),
        quanta: 0,
        after,
    });

    // =====================================================================
    // 判据 ①：**默认帧（Tab 0）里没有它** ⇒ 5 张 Linux 基准不受影响
    // =====================================================================
    let cancelling = ScriptedPicker::cancelling();
    // 两步：先克隆**具体**类型，再让 unsize 强制转换把它交给 `Rc<dyn FilePicker>`
    // （一步写会让 `Rc::clone` 的参数类型被期望类型反过来定成 trait 对象）。
    let cancelling_owned: Rc<ScriptedPicker> = Rc::clone(&cancelling);
    let cancelling_dyn: Rc<dyn FilePicker> = cancelling_owned;
    wire(&window, cancelling_dyn, &runtime);
    let tab0 = runtime_tree(&window);
    report_line(&format!(
        "[#129/ui] 控制台 Tab 0（默认帧）运行时控件树 {} 条；含 `{NEW_BUTTON_ID}` = {}",
        tab0.len(),
        tab0.contains(NEW_BUTTON_ID)
    ));
    assert!(
        !tab0.contains(NEW_BUTTON_ID),
        "⛔ 默认帧（Tab 0 = 卷帘）里不得出现 `{NEW_BUTTON_ID}` —— 它是 `[UI-MCP-003]` 金标\
         覆盖的那一帧，多一个可见控件就会让 5 张 Linux 基准确定性过期（本机无法重录）"
    );

    window.set_console_tab(1);
    let tab1 = runtime_tree(&window);
    assert!(
        tab1.contains("mixer-console"),
        "切到 Tab 1 之后 `mixer-console` 必须在树里（否则 Tab 切换没生效）"
    );

    // =====================================================================
    // 判据 ②：**Tab 1 里有它**（ID + 角色 + 标签 + 几何），且静态注册表也在
    // =====================================================================
    let registry = ElementRegistry::from_view(&view);
    assert!(
        registry.contains(NEW_BUTTON_ID),
        "`{NEW_BUTTON_ID}` 不在静态语义注册表里 —— UI 加了 ID 却忘了登记（[UI-TEST-001]）"
    );
    let node = tab1.find_by_id(NEW_BUTTON_ID).unwrap_or_else(|| {
        panic!(
            "`{NEW_BUTTON_ID}` 不在运行时控件树里 —— 被裁掉 / 改了 accessible-id / 没接进 app.slint。\
             树里 {} 条节点",
            tab1.len()
        )
    });
    assert_eq!(
        node.role.as_str(),
        ElementKind::Button.accessible_role(),
        "入口的角色必须是 `{}`（它在 elements.rs 里登记成 Button）",
        ElementKind::Button.accessible_role()
    );
    assert_eq!(
        node.label, NEW_BUTTON_LABEL,
        "运行时 `accessible-label` 必须与 `elements.rs` 登记的**逐字相同**"
    );
    let bounds = node.bounds.expect("可见控件必须有几何");
    report_line(&format!(
        "[#129/ui] 入口节点: role={} label={:?} bounds={bounds:?}",
        node.role.as_str(),
        node.label
    ));
    assert!(
        bounds.width > 0 && bounds.height > 0,
        "入口必须有实几何（宽 {} × 高 {}）",
        bounds.width,
        bounds.height
    );
    // 既有投影通道的落点也在（回执行住在右栏）—— 回执要写在**它**上面。
    let status = tab1
        .find_by_id(MODULE_STATUS_ID)
        .expect("回执行 `module-last-status` 必须在树里");
    assert_eq!(status.role.as_str(), ElementKind::Text.accessible_role());
    assert_eq!(
        window.get_module_status(),
        "",
        "刚装配的状态行必须是空的 —— 任何非空值都意味着宿主在没人按之前就写了回执"
    );
    assert!(!window.get_module_status_succeeded());

    // =====================================================================
    // 判据 ③：**取消不动一位、不建文件、不写回执**（取消不是失败）
    // =====================================================================
    let fingerprint_before = port.fingerprint();
    let entries_before = scratch.entries();
    press(&window, NEW_BUTTON_ID);
    report_line(&format!(
        "[#129/ui] 取消: choose_save 调用 {} 次；工程指纹 {fingerprint_before:?} → {:?}；\
         track-names {current_rows} → {}；目录条目 {entries_before} → {}；回执 {:?}",
        cancelling.call_count("save"),
        port.fingerprint(),
        rows(&window),
        scratch.entries(),
        window.get_module_status().to_string(),
    ));
    assert_eq!(
        cancelling.call_count("save"),
        1,
        "取消也要先开一次保存对话框（⛔ 不能『没选就不问』）"
    );
    assert_eq!(
        cancelling.last_request("save"),
        Some(PickerRequest::save("新建工程", "project.yeban").with_extensions(["yeban"])),
        "请求的标题 / 建议文件名 / 后缀过滤必须是本单约定的那一条"
    );
    assert_eq!(
        port.fingerprint(),
        fingerprint_before,
        "⛔ 取消之后工程指纹一位不得变"
    );
    assert_eq!(rows(&window), current_rows, "⛔ 取消之后界面行数一位不得变");
    assert_eq!(
        scratch.entries(),
        entries_before,
        "⛔ 取消之后一个文件都不得被建出来"
    );
    assert_eq!(
        window.get_module_status(),
        "",
        "取消**不是**失败：状态行必须保持空（⛔ 不写成一条错误）"
    );
    assert!(!window.get_module_status_succeeded(), "取消不得报成功");
    assert!(
        after_readings.borrow().is_empty(),
        "取消之后 `after`（换工程）一次都不该跑"
    );

    // =====================================================================
    // 判据 ④（反向腿）：**没有对话框** ⇒ **如实报错**（⛔ 不静默、⛔ 不建文件）
    // =====================================================================
    //
    // `NoPicker` 是 `FilePicker::can_prompt() = false` 的唯一实现：它**没有**一个能给用户
    // 看的对话框。新建工程的落点只能来自对话框 ⇒ 这一档必须**响亮**拒绝，而⛔ 不能把
    // "没有对话框"伪装成"用户取消"（那会让一次点击静默消失）。
    wire(&window, Rc::new(NoPicker), &runtime);
    press(&window, NEW_BUTTON_ID);
    let receipt = window.get_module_status().to_string();
    report_line(&format!(
        "[#129/ui] 无对话框（NoPicker）: succeeded={}；回执 {receipt:?}；\
         工程指纹 {fingerprint_before:?} → {:?}；目录条目 {entries_before} → {}",
        window.get_module_status_succeeded(),
        port.fingerprint(),
        scratch.entries(),
    ));
    assert!(
        !window.get_module_status_succeeded(),
        "没有对话框 ⇒ 必须**如实失败**，实得 {receipt:?}"
    );
    assert!(
        receipt.contains("outcome=error")
            && receipt.contains("action=new-project")
            && receipt.contains("error=no-picker"),
        "错误必须走**既有的** `module-status` 投影通道并点名 `no-picker`，实得 {receipt:?}"
    );
    assert!(
        receipt.contains("can_prompt") && receipt.contains("不静默"),
        "错误文本必须说明『没有可用的文件对话框』（不是用户取消），实得 {receipt:?}"
    );
    assert_eq!(
        port.fingerprint(),
        fingerprint_before,
        "⛔ 没有对话框之后工程一位不动"
    );
    assert_eq!(rows(&window), current_rows, "⛔ 没有对话框之后界面一位不动");
    assert_eq!(
        scratch.entries(),
        entries_before,
        "⛔ 没有对话框时不得凭空建文件（也⛔ 不猜文件名）"
    );
    assert!(
        after_readings.borrow().is_empty(),
        "没有对话框之后 `after` 一次都不该跑"
    );

    // =====================================================================
    // 判据 ⑤（正向腿）：点一下 ⇒ 问一次对话框 ⇒ **1136 B** 落盘
    // ⇒ **换成 0 音轨 ＋ 1 主总线的新工程**
    // =====================================================================
    //
    // 对话框给的路径**没有后缀**（`brand-new`）—— 与 `wire_save` 同一条口径，必须补成
    // `.yeban`（macOS 的 `rfd` 会补，别的平台不一定 ⇒ 这条口径钉在 `with_default_extension`）。
    let picked = scratch.join("brand-new");
    let final_path = scratch.join("brand-new.yeban");
    let picker = ScriptedPicker::answering_save(&picked);
    let picker_owned: Rc<ScriptedPicker> = Rc::clone(&picker);
    let picker_dyn: Rc<dyn FilePicker> = picker_owned;
    wire(&window, picker_dyn, &runtime);
    assert!(
        !final_path.exists(),
        "夹具前提：新建之前目标文件必须不存在（否则『它被建出来了』不是新建的证据）"
    );
    press(&window, NEW_BUTTON_ID);
    let receipt = window.get_module_status().to_string();
    let size = std::fs::metadata(&final_path)
        .expect("新建必须真的写出一份容器")
        .len();
    let fingerprint_after = port.fingerprint();
    let default_fingerprint =
        project_fingerprint(&yeban_model::samples::default_project()).expect("空工程指纹");
    report_line(&format!(
        "[#129/ui] 正向: choose_save {} 次；文件 {} = {size} B；结局 succeeded={}；\
         回执 {receipt:?}；工程指纹 {fingerprint_before:?} → {fingerprint_after:?}\
         （`default_project()` = {default_fingerprint}）；track-names {current_rows} → {}；\
         undoable {undoable_before} → {}；after={:?}",
        picker.call_count("save"),
        final_path.display(),
        window.get_module_status_succeeded(),
        rows(&window),
        port.display().undoable,
        after_readings.borrow(),
    ));

    // ① 对话框被问**恰好一次**，且请求是约定的那一条。
    assert_eq!(
        picker.call_count("save"),
        1,
        "⭐ 对话框必须被问**恰好一次**（多一次 = 每次点击弹两次；少一次 = 路径来历不明）"
    );
    assert_eq!(
        picker.last_request("save"),
        Some(PickerRequest::save("新建工程", "project.yeban").with_extensions(["yeban"]))
    );
    assert!(
        window.get_module_status_succeeded(),
        "界面上的新建必须成功: {receipt}"
    );
    // ② 落盘：**1136 B**（无后缀的落点被补成 `.yeban`；`#161` 前是 884 B）。
    assert!(
        final_path.exists(),
        "⛔ 新建必须写出 `{}`（用户敲的 `brand-new` 必须被补成 `.yeban`）",
        final_path.display()
    );
    assert_eq!(
        size, 1136,
        "新建工程的容器必须恰是 1136 字节（`#161`：`default_project()` 自带主总线后，\
         `--project-sample empty --save-as` 同样是 `saved: … bytes=1136`；`#161` 前是 884）；\
         实得 {size} 字节"
    );
    // ③ 回执：成功的字段可读（`tracks` 是**回读**界面行数，不是回显入参）。
    assert!(receipt.contains("outcome=ok"), "{receipt}");
    assert!(
        receipt.contains(&format!("path={}", final_path.display())),
        "{receipt}"
    );
    assert!(receipt.contains("bytes=1136"), "{receipt}");
    assert!(receipt.contains("tracks=0"), "{receipt}");
    // ④ **换成新工程**：端口里的工程 = `default_project()`（指纹逐字节相等），
    //    工程里 0 条音轨 ＋ 1 条主总线；界面音轨行 0 行。
    assert_eq!(
        fingerprint_after.as_deref(),
        Some(default_fingerprint.as_str()),
        "⛔ 端口里的工程必须**逐字节**等于 `samples::default_project()`（新建出来的那一份）"
    );
    assert_eq!(
        port.project().tracks.len(),
        1,
        "新建之后端口里的轨道数必须恰是 1（主总线）"
    );
    assert!(
        !port.project().master_bus_track_id.is_nil(),
        "新建工程的 `master_bus_track_id` 必须非 nil（`#161` 起出厂自带主总线）"
    );
    assert_eq!(
        port.project()
            .tracks
            .get(&port.project().master_bus_track_id)
            .map(|track| track.kind),
        Some(yeban_model::project::TrackKind::Master),
        "那 1 条轨必须就是主总线"
    );
    assert_eq!(rows(&window), 0, "⛔ 新建之后界面音轨行必须清空（0 行）");
    // ⑤ 「新建 = 打开刚写的文件 = **新会话**」：可撤销步数归零。
    assert_eq!(
        port.display().undoable,
        0,
        "⭐ 新建工程 = **新会话**：`undoable` 必须从 {undoable_before} 归零"
    );
    // ⑥ **同一个** `after`（打开工程那条换工程路径的装配方最后一跳）被调用**恰好一次**，
    //    而且它拿到的就是刚写出的那个路径 / 0 音轨 ＋ 1 主总线工程 / 0 音轨行投影。
    let after_now = after_readings.borrow().clone();
    assert_eq!(
        after_now,
        vec![(final_path.to_string_lossy().into_owned(), 1, 0)],
        "⭐ 新建必须走 `OpenProjectRuntime::open_runtime`（`#93` 的换工程路径）—— \
         `after` 应当恰好收到一次 (path=刚写出的文件, 1 条轨[主总线], 0 条音轨行投影)"
    );
    // ⑦ 引擎：新工程**有主总线** ⇒ `EngineSnapshot::from_project` 的既有裁决是
    //    "可用"（⛔ 不再 `NoMasterBus`）⇒ 换工程路径把引擎换到新快照（0 条音轨 ＋ 1 主总线），
    //    且界面停在停止态。⛔ 不再清空引擎。
    {
        let engine = production.borrow().engine_handle();
        assert!(
            engine.borrow().has_engine(),
            "有主总线的新工程必须能投影出快照（`#161` 前是 `NoMasterBus` ⇒ 清空引擎）"
        );
        let snapshot = engine
            .borrow()
            .current_snapshot()
            .expect("换代成功之后必须有当前快照");
        assert_eq!(
            snapshot.master(),
            port.project().master_bus_track_id,
            "快照的主总线必须是新工程的主总线"
        );
        assert_eq!(snapshot.tracks().len(), 1, "快照里必须只有主总线那 1 条轨");
        assert!(!window.get_playing(), "新建之后界面必须停在停止态");
    }
    drop(tier1);
}
