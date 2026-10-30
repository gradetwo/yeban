//! 判据：**运行期「导入音频」在 GUI 里有一个可点的入口**，点下去真的把一份 WAV
//! 作为**可撤销的片段**导入当前工程（取消不动一位 / 坏文件响亮且不毁工程 / 幂等不重复提交）。
//!
//! ## 本单补的是哪一格
//!
//! | 层 | 现状（本单之前） |
//! | :--- | :--- |
//! | 能力（Rust） | `crate::import_audio::import_audio_file` ✓（CLI `--import-audio` 与 MCP `yeban_import_audio` 在同一份实现上） |
//! | 会话池 | `UndoPort::insert_asset` ✓（保存经 `UndoPort::assets()` 写 `assets/{sha256}`） |
//! | **GUI 出口** | ⛔ 一个都没有：素材池面板只有**只读**三数组，界面里没有任何导入动作 |
//! | ⭐ **本单** | ⭐ 素材池面板的 `asset-pool-import-button`（控制台 **Tab 1**）⇒ `host::wire_import_audio` ⇒ `FilePicker` ⇒ `import_audio_file` ⇒ 资产登记 ⇒ 重投影 ⇒ `module-status` 回执 |
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | 树里有没有 `asset-pool-import-button` | 活窗口的运行时控件树（Tab 0 / Tab 1 各读一次） | 布尔 |
//! | `role` / `label` / `bounds` | 该节点的 `accessible-role` / `accessible-label` / 实几何 | kebab-case 角色 / 中文标签 / 像素 |
//! | `choose_open` 调用数 | 假选择器收到的请求数 | 次 |
//! | 工程指纹 | `UndoPort::fingerprint()` | 十六进制串 |
//! | 素材池行数 | `MainWindow.asset-pool-names` 的行数（= 投影行数） | 行 |
//! | `undoable` | `UndoPort::display().undoable`（可撤销步数） | 步 |
//! | 会话资产池 | `UndoPort::assets()` 的条目数 / 字节是否逐字等于源文件 | 条 / 布尔 |
//! | `module-status` | 宿主写的回执行原文 | 结构化文本 |
//!
//! ## 为什么入口住在控制台 Tab 1（与 `#90` 甲 / `#93` / `#129` 同一条纪律）
//!
//! `[UI-MCP-003]` 的 5 张 Linux 基准**只覆盖默认帧**。`ai-rail` 在默认帧里可见 ⇒
//! 在那里多一个按钮会让基准**确定性过期**，而本机是 macOS、`tests/golden/<platform>`
//! **只有** `linux` ⇒ 本机**无法**重录。素材池面板住在 `MixerConsole`（Tab 1），
//! 默认帧**一个像素都不画** ⇒ 基准逐位不变（本判据的 ① 就是这条的机械读数）。
//!
//! ## 这条判据怎么变红（注入读数见交付报告）
//!
//! - `.slint` 的 `accessible-action-default` / `TouchArea.clicked` 不调 `root.import-audio()`
//!   ⇒ 注入之后工程一位不动 ⇒ 正向腿红；
//! - `host::wire_import_audio` 不接回调 / 回执不写 `module-status` ⇒ 同上；
//! - 取消也去导入 / 写回执 ⇒ ② 的三条"一位不动"断言红；
//! - 失败路径先改工程再报错 ⇒ ③ 的"不毁工程"断言红；
//! - 导入不登记资产字节 ⇒ ④ 的 `UndoPort::assets()` 条目数 0 ⇒ 红（保存会结构性丢资产）；
//! - 导入不重投影 ⇒ 素材池行数不涨 ⇒ 红。
//!
//! ## 平台约束：本文件只有一个 `#[test]`
//!
//! `slint::platform::set_platform` 是**线程局部**的，`Tier1Window::install` 每线程只能成功
//! 一次（第二次 `AlreadySet`）⇒ 同款用法见 `tests/open_project_ui.rs`。

#[path = "common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::rc::Rc;

use common::ScriptedPicker;
use yeban_app::bridge::ViewState;
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::file_picker::{FilePicker, PickerRequest};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::{find_by_accessible_id, tree_from_element_root};
use yeban_ui_test_port::render::{Tier1Window, report_line};
use yeban_ui_test_port::tree::ControlTree;

/// 新入口的语义 ID（`ui/console/asset_pool_panel.slint` ↔ `src/elements.rs` 两处必须逐字相同）。
const IMPORT_BUTTON_ID: &str = "asset-pool-import-button";

/// 新入口的无障碍名称（与 `elements.rs` 登记的标签**逐字相同**）。
const IMPORT_BUTTON_LABEL: &str = "导入音频（用文件对话框挑一个 WAV 并作为片段导入）";

/// 撤销会话的"现在"（判据不需要墙钟，与 `open_project_ui.rs` 同款）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 窗口尺寸（与 `open_project_ui.rs` 同款：展开档）。
const WINDOW: Size = Size::new(1440, 900);

/// 临时目录（`Drop` 时清理，**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-import-audio-entry-{}-{}",
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

/// 现造一份**真** WAV（本仓唯一写出器）；返回 `(路径, 字节)`。
fn write_wav(dir: &Path, frames: usize) -> (PathBuf, Vec<u8>) {
    let path = dir.join("take.wav");
    let samples: Vec<i16> = (0..frames)
        .map(|index| i16::try_from(index % 64).expect("夹具样本") * 100 - 3_000)
        .collect();
    yeban_render::wav::write_plain_wav(
        &path,
        &PcmFormat::integer(1, 48_000, 16),
        &PcmBuffer::Int16(samples),
    )
    .expect("写夹具 WAV");
    let bytes = std::fs::read(&path).expect("读夹具 WAV");
    (path, bytes)
}

/// 读活窗口的运行时控件树（[UI-TEST-001] 的载体：全树内省 + JSON 往返）。
fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

/// 界面此刻的素材池行数（`[UI-TEST-001]` 的语义属性；⛔ 不数像素）。
fn pool_rows(window: &MainWindow) -> usize {
    use slint::Model as _;
    window.get_asset_pool_names().iter().count()
}

/// 触发一个按钮的**默认无障碍动作**（与鼠标点击是逐字同一段落点）。
fn press(window: &MainWindow, id: &str) {
    let handle = find_by_accessible_id(window, id)
        .unwrap_or_else(|| panic!("控件树里没有 `{id}` —— 入口不在当前可见分支里（或 ID 改了）"));
    handle.invoke_accessible_default_action();
}

/// 把假选择器接到**本单的唯一实现**上（重复调用 = `on_*` setter 覆盖）。
fn wire(window: &MainWindow, port: &Rc<UndoPort>, picker: &Rc<ScriptedPicker>) {
    let owned: Rc<ScriptedPicker> = Rc::clone(picker);
    let picker: Rc<dyn FilePicker> = owned;
    host::wire_import_audio(window, Rc::clone(port), picker);
}

#[test]
fn the_asset_pool_import_button_really_imports_a_wav() {
    let scratch = Scratch::new();
    let (wav, wav_bytes) = write_wav(&scratch.dir, 256);
    let bad = scratch.dir.join("not-a-wav.wav");
    std::fs::write(&bad, b"this is not a RIFF/WAVE file").expect("写坏夹具");

    let project = yeban_model::samples::demo_project();
    let view = ViewState::from_project(&project).expect("投影");
    let scene = DemoScene::from_view(&view);

    // ---- 装配：**生产形态**的窗口（`run_gui` 用的同一个构造入口）＋ Tier-1 无头平台 ----
    let _tier1 = Tier1Window::install(WINDOW).expect("装 Tier-1 无头平台");
    let window = host::build_main_window_with_console_tab(&view, &scene, 0).expect("生产窗口");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据 import-audio>", "yeban-app", project.clone(), NOW_MS)
            .expect("打开撤销会话"),
    ));
    let rows_before = pool_rows(&window);
    assert_eq!(
        rows_before,
        view.asset_pool.len(),
        "夹具：初始素材池行数必须等于投影行数"
    );
    let fingerprint_before = port.fingerprint();
    let undoable_before = port.display().undoable;
    assert_eq!(undoable_before, 0, "夹具：新会话必须没有可撤销历史");

    // =====================================================================
    // 判据 ①：**默认帧（Tab 0）里没有它** ⇒ 5 张 Linux 基准不受影响
    // =====================================================================
    let cancelling = ScriptedPicker::cancelling();
    wire(&window, &port, &cancelling);
    let tab0 = runtime_tree(&window);
    report_line(&format!(
        "[import-audio] 控制台 Tab 0（默认帧）运行时控件树 {} 条；含 `{IMPORT_BUTTON_ID}` = {}",
        tab0.len(),
        tab0.contains(IMPORT_BUTTON_ID)
    ));
    assert!(
        !tab0.contains(IMPORT_BUTTON_ID),
        "⛔ 默认帧（Tab 0）里不得出现 `{IMPORT_BUTTON_ID}` —— 它是 `[UI-MCP-003]` 金标\
         覆盖的那一帧，多一个可见控件就会让 5 张 Linux 基准确定性过期（本机无法重录）"
    );

    // 切到 Tab 1（调音台）—— 这一跳必须**真的**发生，否则"在树里"可能只是别的原因。
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
        registry.contains(IMPORT_BUTTON_ID),
        "`{IMPORT_BUTTON_ID}` 不在静态语义注册表里 —— UI 加了 ID 却忘了登记（[UI-TEST-001]）"
    );
    let node = tab1
        .find_by_id(IMPORT_BUTTON_ID)
        .unwrap_or_else(|| panic!("`{IMPORT_BUTTON_ID}` 不在运行时控件树里"));
    assert_eq!(
        node.role.as_str(),
        ElementKind::Button.accessible_role(),
        "入口的角色必须是 `{}`",
        ElementKind::Button.accessible_role()
    );
    assert_eq!(
        node.label, IMPORT_BUTTON_LABEL,
        "运行时 `accessible-label` 必须与 `elements.rs` 登记的**逐字相同**"
    );
    let bounds = node.bounds.expect("可见控件必须有几何");
    report_line(&format!(
        "[import-audio] 入口节点: role={} label={:?} bounds={bounds:?}（树 {} 条）",
        node.role.as_str(),
        node.label,
        tab1.len()
    ));
    assert!(
        bounds.width > 0 && bounds.height > 0,
        "入口必须有实几何（宽 {} × 高 {}）",
        bounds.width,
        bounds.height
    );
    assert_eq!(
        window.get_module_status(),
        "",
        "刚装配的状态行必须是空的 —— 任何非空值都意味着宿主在没人按之前就写了回执"
    );

    // =====================================================================
    // 判据 ③：**取消不动一位**（用户按了对话框的取消 ⇒ 什么都不做）
    // =====================================================================
    press(&window, IMPORT_BUTTON_ID);
    report_line(&format!(
        "[import-audio] 取消: choose_open 调用 {} 次；指纹 {fingerprint_before:?} → {:?}；\
         素材池 {rows_before} → {}；undoable {undoable_before} → {}；回执 {:?}",
        cancelling.call_count("open"),
        port.fingerprint(),
        pool_rows(&window),
        port.display().undoable,
        window.get_module_status().to_string(),
    ));
    assert_eq!(
        cancelling.call_count("open"),
        1,
        "取消也要先开一次对话框（⛔ 不能『没选就不问』）"
    );
    assert_eq!(
        cancelling.last_request("open"),
        Some(PickerRequest::open("导入音频").with_extensions(["wav"])),
        "请求的标题与后缀过滤必须是「导入音频」+ 仅 wav"
    );
    assert_eq!(
        port.fingerprint(),
        fingerprint_before,
        "⛔ 取消之后工程指纹一位不得变"
    );
    assert_eq!(
        pool_rows(&window),
        rows_before,
        "⛔ 取消之后素材池行数一位不得变"
    );
    assert_eq!(
        port.display().undoable,
        undoable_before,
        "⛔ 取消之后撤销会话一位不得变"
    );
    assert_eq!(
        window.get_module_status(),
        "",
        "取消**不是**失败：状态行必须保持空（⛔ 不写成一条错误）"
    );
    assert!(!window.get_module_status_succeeded(), "取消不得报成功");

    // =====================================================================
    // 判据 ④：**导入失败要响亮**，且**当前工程与素材池不被毁掉**
    // =====================================================================
    {
        let picker = ScriptedPicker::answering_open(&bad);
        wire(&window, &port, &picker);
        press(&window, IMPORT_BUTTON_ID);
        let receipt = window.get_module_status().to_string();
        report_line(&format!(
            "[import-audio] 坏输入: choose_open {} 次；回执 {receipt:?}；指纹 {:?} → {:?}",
            picker.call_count("open"),
            fingerprint_before,
            port.fingerprint(),
        ));
        assert_eq!(picker.call_count("open"), 1, "必须问了恰好一次");
        assert!(
            !window.get_module_status_succeeded(),
            "坏文件必须**如实失败**，实得 {receipt:?}"
        );
        assert!(
            receipt.contains("outcome=error") && receipt.contains("import-audio"),
            "错误必须走**既有的** `module-status` 投影通道（结构化回执），实得 {receipt:?}"
        );
        assert!(
            receipt.contains(bad.to_string_lossy().as_ref()),
            "回执必须点名那个读不动的路径，实得 {receipt:?}"
        );
        assert_eq!(
            port.fingerprint(),
            fingerprint_before,
            "⛔ 失败**不得**把当前工程改坏"
        );
        assert_eq!(
            pool_rows(&window),
            rows_before,
            "⛔ 失败之后素材池必须还是原来那些行"
        );
        assert_eq!(
            port.display().undoable,
            undoable_before,
            "⛔ 失败不得动撤销会话"
        );
        assert!(port.assets().is_empty(), "⛔ 失败不得登记资产字节");
    }

    // =====================================================================
    // 判据 ⑤（正向腿）：注入默认无障碍动作 ⇒ **真的导入**（五条独立读数）
    // =====================================================================
    {
        let picker = ScriptedPicker::answering_open(&wav);
        wire(&window, &port, &picker);
        press(&window, IMPORT_BUTTON_ID);
        let receipt = window.get_module_status().to_string();
        report_line(&format!(
            "[import-audio] 正向: choose_open {} 次；回执 {receipt:?}；指纹 {:?} → {:?}；\
             素材池 {rows_before} → {}；undoable {undoable_before} → {}；资产池 {} 条",
            picker.call_count("open"),
            fingerprint_before,
            port.fingerprint(),
            pool_rows(&window),
            port.display().undoable,
            port.assets().len(),
        ));
        assert!(
            window.get_module_status_succeeded(),
            "正向腿必须报成功，实得 {receipt:?}"
        );
        assert!(
            receipt.contains("outcome=ok") && receipt.contains("name=take"),
            "回执必须点名导入的片段名（`take`），实得 {receipt:?}"
        );
        assert!(
            receipt.contains("created=true") && receipt.contains("asset_registered=true"),
            "首次导入必须 `created=true` 且登记了资产字节，实得 {receipt:?}"
        );
        assert_ne!(
            port.fingerprint(),
            fingerprint_before,
            "⛔ 导入之后工程指纹必须变（否则模型根本没被改）"
        );
        assert_eq!(
            pool_rows(&window),
            rows_before + 1,
            "素材池必须多出一行（重投影真的跑了）"
        );
        assert_eq!(
            port.display().undoable,
            undoable_before + 1,
            "一次导入 = 一步可撤销的编辑"
        );
        let assets = port.assets();
        assert_eq!(assets.len(), 1, "会话资产池必须恰好登记一份字节");
        assert_eq!(
            assets[0].1, wav_bytes,
            "登记的字节必须与源 WAV **逐字相同**（否则保存出来的容器指向错内容）"
        );
        // 运行时树里也要多出第三行（素材池行是投影驱动的动态族）。
        let tab1_after = runtime_tree(&window);
        assert!(
            tab1_after.contains(&format!("asset-pool-row-{rows_before}-row")),
            "重投影之后运行时树里必须出现第 {} 行（0 基下标 {}）",
            rows_before + 1,
            rows_before
        );
    }

    // =====================================================================
    // 判据 ⑥：**同一份字节再导入一次 = 幂等**（不重复提交、不涨行、不重复登记）
    // =====================================================================
    {
        let picker = ScriptedPicker::answering_open(&wav);
        wire(&window, &port, &picker);
        let rows_now = pool_rows(&window);
        let undoable_now = port.display().undoable;
        let fingerprint_now = port.fingerprint();
        press(&window, IMPORT_BUTTON_ID);
        let receipt = window.get_module_status().to_string();
        report_line(&format!(
            "[import-audio] 幂等: 回执 {receipt:?}；指纹 {:?} → {:?}；素材池 {rows_now} → {}；\
             undoable {undoable_now} → {}；资产池 {} 条",
            fingerprint_now,
            port.fingerprint(),
            pool_rows(&window),
            port.display().undoable,
            port.assets().len(),
        ));
        assert!(
            window.get_module_status_succeeded(),
            "幂等命中也是**成功**（池里已经有同内容片段），实得 {receipt:?}"
        );
        assert!(
            receipt.contains("created=false") && receipt.contains("commit=<none>"),
            "幂等命中必须如实报 `created=false` / `commit=<none>`，实得 {receipt:?}"
        );
        assert!(
            receipt.contains("asset_registered=false"),
            "同哈希已登记 ⇒ `insert_asset` 回 `false`，回执必须如实报出，实得 {receipt:?}"
        );
        assert_eq!(port.fingerprint(), fingerprint_now, "⛔ 幂等命中不得改工程");
        assert_eq!(pool_rows(&window), rows_now, "⛔ 幂等命中不得涨行");
        assert_eq!(
            port.display().undoable,
            undoable_now,
            "⛔ 幂等命中不得发提交"
        );
        assert_eq!(port.assets().len(), 1, "⛔ 幂等命中不得重复登记资产");
    }
}
