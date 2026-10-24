//! 判据（任务单 `#90` **甲**）：**导出 MIDI** 在界面上有**真**入口，且按下去真的导出。
//!
//! ## 本单补的是哪一格
//!
//! | 层 | 现状（本单之前） |
//! | :--- | :--- |
//! | 能力（Rust） | `crate::export_midi::export_project_to_file` ✓（与 CLI `--export-midi` **同一个**函数） |
//! | CLI 出口 | `yeban-app --export-midi out.mid` ✓ |
//! | MCP 出口 | `yeban_export_midi` ✓ |
//! | **GUI 出口** | ⛔ **一个都没有** —— 人在界面上只能导出 WAV，导不出 MIDI |
//! | ⭐ **本单** | ⭐ `module-export-midi-button`（控制台 **Tab 1 = 调音台**）⇒ `host::wire_module_actions` 的 `export-midi` 分支 ⇒ `module-status` 回执 |
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | 树里有没有 `module-export-midi-button` | 活窗口的运行时控件树（Tab 0 / Tab 1 各读一次） | 布尔 |
//! | `role` / `label` | 该节点的 `accessible-role` / `accessible-label` | kebab-case 角色 / 中文标签 |
//! | `module-status` | 宿主写的**回执行**原文（既有投影通道） | 结构化文本 |
//! | 落盘字节 | 目标 `.mid` 的长度 / 内容 | 字节（与共享导出器的产物**逐字节**比对） |
//!
//! ## 为什么入口住在控制台 Tab 1（而不是右栏那排「模块入口」）
//!
//! ⭐ `[UI-MCP-003]` 的 5 张 Linux 基准**只覆盖默认帧**（Arrangement / Tab 0 卷帘）。
//! `ai-rail` 在默认帧里是**可见的静态像素**，在那里多一个按钮 ⇒ 5 张基准确定性过期，
//! 而本机是 macOS、`tests/golden/<platform>` **只有** `linux` ⇒ 本机**无法**重录
//! （重录要走手动档 `gates-manual.yml` 的 `gate=goldens` + 人工复核 = 由负责人裁）。
//! `MixerConsole` 只在 `visible: root.active-tab == 1` 时可见 ⇒ 默认帧**一个像素都不变**。
//! 同一纪律的既有先例：任务单 `#86` 乙的「浏览…」按钮（同样落在 Tab 1）。
//!
//! ## 两个方向（本文件都测）
//!
//! 1. **Tab 0（默认帧）里没有它** ⇒ 基准不受影响（与 `src/test_port_adapter.rs` 的
//!    `DEFAULT_VIEW_MUST_NOT_HAVE` 是同一条断言的第二种写法）；
//! 2. **Tab 1 里有它**，且**事件注入**能触发它，且**结果可读** —— 但结果**不是**
//!    "元素存在"这种空断言：断言的是**落盘文件的字节** + `module-status` 回执的字段
//!    （`path` / `bytes` / `notes` / `format`），以及"没有撤销端口 ⇒ 如实拒绝且不留文件"。
//!
//! ## 这条判据怎么变红
//!
//! - 把 `ui/console/mixer_console.slint` 的 `clicked` / `accessible-action-default` 从
//!   `root.export-midi()` 改成别的东西（**断开回调**）⇒ 回执行仍为空、文件不落盘 ⇒ 红；
//! - `host::wire_module_actions` 不装 `on_export_midi` ⇒ 同上 ⇒ 红；
//! - 目标路径不读 `ModulePaths::export_midi`（界面或宿主写死一个路径）⇒ 红；
//! - 换一个第二份导出实现（字节与共享导出器不同）⇒ 逐字节比对红。
//!
//! ## 依赖从哪来（红线 6 没有被削弱）
//!
//! `yeban-ui-test-port` 是 `[dev-dependencies]`：**不进入** release 构建。核验命令（零编译）：
//! `cargo tree -p yeban-app -e normal --locked`。
//!
//! ## 平台约束：本文件只有一个 `#[test]`
//!
//! `slint::platform::set_platform` 是**线程局部**的，`Tier1Window::install` 每线程只能成功
//! 一次（第二次 `AlreadySet`）⇒ 同款用法见 `tests/ui_module_actions.rs` / `tests/engine_error_banner.rs`。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的装配入口 ⇒ 其余项在本测试目标里是 dead_code
// （`-D warnings` 会判红）。同款见 `tests/engine_error_banner.rs`。
#[allow(dead_code)]
mod live;

use std::path::PathBuf;
use std::rc::Rc;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_app::bridge::ViewState;
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::host::{ModulePaths, wire_module_actions_with};
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::inspect::{find_by_accessible_id, tree_from_element_root};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;
use yeban_ui_test_port::tree::ControlTree;

/// 新入口的语义 ID（`ui/console/mixer_console.slint` ↔ `src/elements.rs` 两处必须逐字相同）。
const MIDI_BUTTON_ID: &str = "module-export-midi-button";

/// 新入口的无障碍名称（与 `elements.rs` 登记的标签**逐字相同**，运行时对账会断言相等）。
const MIDI_BUTTON_LABEL: &str = "导出 MIDI（把当前工程写成 SMF 1 文件）";

/// 既有投影通道的落点（回执行的**可寻址**节点，住在右栏 —— 默认帧里就可见）。
const MODULE_STATUS_ID: &str = "module-last-status";

/// 撤销会话的"现在"（与 `sfz_ui_selector.rs` 的常量同款：判据不需要墙钟）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 回执的字段名闭集（**本判据自己的那一份**：只认这几条作 `field_of` 的切分边界）。
///
/// 加字段就加一行 —— 漏加不会静默：`field_of` 取不到值会**当场 panic 并打印整条回执**。
const RECEIPT_KEYS: [&str; 12] = [
    "action", "outcome", "error", "message", "path", "bytes", "ppq", "format", "tracks", "notes",
    "tempos", "temp",
];

/// 临时目录（`Drop` 时清理，**绝不污染仓库**）。形态照 `tests/ui_module_actions.rs` 的 `Scratch`。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-export-midi-ui-{}-{}",
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

/// 一条回执里，从 `at` 起**第一个** `:<已知字段名>=` 的冒号位置（口径与
/// `tests/ui_module_actions.rs` 的 `receipt_field_end` **逐字同款**：值里可能自带冒号）。
fn receipt_field_end(receipt: &str, at: usize, skip: &str) -> Option<usize> {
    let mut index = at;
    while index < receipt.len() {
        if receipt.as_bytes()[index] == b':' {
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

/// 一条回执的**顶层**字段取值（引号感知，`#321`）；缺席返回 `None`。
///
/// 字段名只认**顶层**位置（`key=` 必须紧跟一个**顶层** `:`；引号内的 `:` 不算起点）：
/// 引号值跳到**第一个闭合引号**（引号内的 `format=` / `channels=` 等一律不是字段名），普通值仍按
/// [`receipt_field_end`] 的"已知字段名 + `=`"边界取。引号值**保留引号**（本文件既有约定）。
///
/// ## 引号里的 `"`（写死的契约）
///
/// 回执没有转义 ⇒ 第一个 `"` 即闭合引号；值里真带 `"` 时其后的字段取不到（`None`）。
fn field_of_opt(receipt: &str, key: &str) -> Option<String> {
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
                        return Some(receipt[value_start..=value_start + 1 + close].to_owned());
                    }
                    let end = receipt_field_end(receipt, value_start, key).unwrap_or(receipt.len());
                    return Some(receipt[value_start..end].to_owned());
                }
            }
            _ => {}
        }
    }
    None
}

/// 取一个字段；缺席即判据失败（⛔ 不返回空串 —— "没找到"与"值是空的"必须可分）。
fn field_of(receipt: &str, key: &str) -> String {
    field_of_opt(receipt, key).unwrap_or_else(|| panic!("回执里没有字段 `{key}`: {receipt}"))
}

/// 读活窗口的运行时控件树（`[UI-DoD §3.6]` 的载体：全树内省 + JSON 往返）。
fn runtime_tree(window: &yeban_app::ui::MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

/// 一个语义 ID 必须**在这个活窗口的运行时控件树里**，且角色符合登记。
fn assert_addressable(tree: &ControlTree, id: &str, kind: ElementKind) {
    let node = tree.find_by_id(id).unwrap_or_else(|| {
        panic!(
            "`{id}` 不在运行时控件树里 —— 被裁掉 / 改了 accessible-id / 没接进 app.slint。\
             树里 {} 条节点",
            tree.len()
        )
    });
    assert_eq!(
        node.role.as_str(),
        kind.accessible_role(),
        "`{id}` 的角色必须是 `{}`（它在 elements.rs 里登记成 {kind:?}）",
        kind.accessible_role()
    );
}

/// 触发一个按钮的**默认无障碍动作**（与鼠标点击是逐字同一段落点）。
fn press(window: &yeban_app::ui::MainWindow, id: &str) {
    let handle = find_by_accessible_id(window, id)
        .unwrap_or_else(|| panic!("控件树里没有 `{id}` —— 入口不在当前可见分支里（或 ID 改了）"));
    handle.invoke_accessible_default_action();
}

#[test]
fn the_gui_export_midi_entry_really_writes_a_smf_and_reports_it() {
    let scratch = Scratch::new();
    let target = scratch.join("gui-export.mid");
    let shared = scratch.join("shared-export.mid");

    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据 #90 甲>", "yeban-app", project.clone(), NOW_MS)
            .expect("打开撤销会话"),
    ));

    // 装配：真窗口 + Tier-1 执行面 + **默认帧的 Tab 0**（`build_live_ui_with` 自己会用
    // `ModulePaths::from_env()` 接一次模块入口；下面再显式接一次**同一组**回调，路径由
    // 本判据给出 —— `set_handler` 可覆盖，与 `tests/ui_module_actions.rs` 同款）。
    let options = LiveWiringOptions {
        permission: Permission::Interactive,
        console_tab: 0,
        undo: Some(Rc::clone(&port)),
        ..LiveWiringOptions::default()
    };
    let live_ui = build_live_ui_with(&project, &options).expect("Tier-1 平台 + 活窗口 + 接线");
    let window = slint::ComponentHandle::clone_strong(live_ui.ui());

    let mut paths = ModulePaths::from_env();
    paths.export_midi = target.clone();
    wire_module_actions_with(&window, Some(Rc::clone(&port)), paths.clone());

    // =====================================================================
    // 判据 ①：**默认帧（Tab 0）里没有它** ⇒ 5 张 Linux 基准不受影响
    // =====================================================================
    let tab0 = runtime_tree(&window);
    report_line(&format!(
        "[#90 甲] 控制台 Tab 0（默认帧）运行时控件树 {} 条；含 `{MIDI_BUTTON_ID}` = {}",
        tab0.len(),
        tab0.contains(MIDI_BUTTON_ID)
    ));
    assert!(
        !tab0.contains(MIDI_BUTTON_ID),
        "⛔ 默认帧（Tab 0 = 卷帘）里不得出现 `{MIDI_BUTTON_ID}` —— 它是`[UI-MCP-003]` 金标\
         覆盖的那一帧，多一个可见控件就会让 5 张 Linux 基准确定性过期（本机无法重录）"
    );

    // 切到 Tab 1（调音台）—— 这一跳必须**真的**发生，否则下面"在树里"可能只是别的原因。
    window.set_console_tab(1);
    let tab1 = runtime_tree(&window);
    assert!(
        tab1.contains("mixer-console"),
        "切到 Tab 1 之后 `mixer-console` 必须在树里（否则 Tab 切换没生效，下面的断言就没有对照）"
    );
    report_line(&format!(
        "[#90 甲] 控制台 Tab 1（调音台）运行时控件树 {} 条；含 `mixer-console` = {}",
        tab1.len(),
        tab1.contains("mixer-console")
    ));

    // =====================================================================
    // 判据 ②：**Tab 1 里有它**（ID + 角色 + 标签），且两侧（注册表 / 运行时）都在
    // =====================================================================
    let registry = ElementRegistry::from_view(&view);
    assert!(
        registry.contains(MIDI_BUTTON_ID),
        "`{MIDI_BUTTON_ID}` 不在静态语义注册表里 —— UI 加了 ID 却忘了登记（[UI-TEST-001]）"
    );
    assert_addressable(&tab1, MIDI_BUTTON_ID, ElementKind::Button);
    assert_addressable(&tab1, "mixer-console", ElementKind::Region);
    let node = tab1.find_by_id(MIDI_BUTTON_ID).expect("上面刚断言过");
    assert_eq!(
        node.label, MIDI_BUTTON_LABEL,
        "运行时 `accessible-label` 必须与 `elements.rs` 登记的**逐字相同**"
    );
    let bounds = node.bounds.expect("可见控件必须有几何");
    report_line(&format!(
        "[#90 甲] 入口节点: role={} label={:?} bounds={bounds:?}",
        node.role.as_str(),
        node.label
    ));
    assert!(
        bounds.width > 0 && bounds.height > 0,
        "入口必须有实几何（宽 {} × 高 {}）",
        bounds.width,
        bounds.height
    );
    // 既有投影通道的落点也在（回执行住在右栏 —— 非 compact 下默认可见）。
    assert_addressable(&tab1, MODULE_STATUS_ID, ElementKind::Text);

    // 初始态 = **空回执**（"本次会话还没按过任何模块入口"），不是编造的读数。
    assert_eq!(
        window.get_module_status(),
        "",
        "刚装配的状态行必须是空的 —— 任何非空值都意味着宿主在没人按之前就写了回执"
    );
    assert!(!window.get_module_status_succeeded());

    // =====================================================================
    // 判据 ③（反向腿）：**没有撤销端口** ⇒ 如实拒绝，且**不留文件**
    // =====================================================================
    let no_port_target = scratch.join("no-port.mid");
    let mut no_port_paths = paths.clone();
    no_port_paths.export_midi = no_port_target.clone();
    wire_module_actions_with(&window, None, no_port_paths);
    press(&window, MIDI_BUTTON_ID);
    let refused = window.get_module_status().to_string();
    report_line(&format!("[#90 甲] 无端口回执行: {refused}"));
    assert!(
        !window.get_module_status_succeeded(),
        "没有撤销端口 ⇒ 拿不到工程的权威读数 ⇒ 必须如实失败: {refused}"
    );
    assert_eq!(field_of(&refused, "action"), "export-midi", "{refused}");
    assert_eq!(field_of(&refused, "outcome"), "error", "{refused}");
    assert_eq!(field_of(&refused, "error"), "no-project", "{refused}");
    assert!(
        !no_port_target.exists(),
        "被拒的导出不得留下目标文件 {}",
        no_port_target.display()
    );

    // =====================================================================
    // 判据 ④（正向腿）：按下去**真的**导出一份 SMF，且**字节等于共享导出器**的产物
    // =====================================================================
    wire_module_actions_with(&window, Some(Rc::clone(&port)), paths.clone());
    assert!(!target.exists(), "按之前目标文件必须不存在");
    press(&window, MIDI_BUTTON_ID);

    let receipt = window.get_module_status().to_string();
    report_line(&format!("[#90 甲] 导出回执行: {receipt}"));
    assert!(
        window.get_module_status_succeeded(),
        "界面上的导出必须成功: {receipt}"
    );
    assert_eq!(field_of(&receipt, "action"), "export-midi", "{receipt}");
    assert_eq!(field_of(&receipt, "outcome"), "ok", "{receipt}");

    // 回执里的路径必须是**宿主给的那一个**（⛔ 界面不猜路径）。
    let reported_path = PathBuf::from(field_of(&receipt, "path"));
    assert_eq!(
        reported_path, target,
        "回执里的路径必须逐字等于 `ModulePaths::export_midi`（⛔ 不是界面自己拼的）"
    );

    // ⭐ 可观察的结果：文件真的在，长度与回执一致，且是 SMF（`MThd` 魔数）。
    let written = std::fs::read(&target)
        .unwrap_or_else(|error| panic!("目标文件没落盘 {}: {error}", target.display()));
    assert_eq!(
        written.len(),
        field_of(&receipt, "bytes")
            .parse::<usize>()
            .expect("bytes 必须是十进制数"),
        "回执里的 `bytes` 必须等于盘上文件的长度"
    );
    assert!(
        written.starts_with(b"MThd"),
        "导出的必须是标准 MIDI 文件（前 4 字节 `MThd`），实得 {:?}",
        &written[..written.len().min(4)]
    );
    assert!(written.len() > 14, "SMF 至少要有 MThd 头");

    // ⭐ 单实现证据：GUI 这一跳的字节必须与**共享导出器**（CLI `--export-midi` 用的那个
    //    函数）对同一工程的产物**逐字节相同** ⇒ ⛔ 不存在第二份导出器。
    let shared_report =
        yeban_app::export_midi::export_project_to_file(&project, &shared).expect("共享导出器");
    let shared_bytes = std::fs::read(&shared).expect("共享导出的文件");
    assert_eq!(
        written, shared_bytes,
        "GUI 导出的字节必须与 `export_midi::export_project_to_file` 的产物**逐字节相同**"
    );
    assert_eq!(
        shared_report.bytes,
        shared_bytes.len(),
        "共享导出器自报的字节数与盘上长度必须一致"
    );

    // 回执的读数与共享报告对得上（⛔ 不是界面编的一串数字）。
    assert_eq!(
        field_of(&receipt, "ppq").parse::<u16>().expect("ppq"),
        shared_report.ppq,
        "回执的 PPQ 必须来自同一份报告"
    );
    assert_eq!(
        field_of(&receipt, "notes").parse::<usize>().expect("notes"),
        shared_report.notes,
        "回执的音符数必须来自同一份报告"
    );
    assert_eq!(
        field_of(&receipt, "tracks")
            .parse::<usize>()
            .expect("tracks"),
        shared_report.tracks,
        "回执的轨道数必须来自同一份报告"
    );
    assert_eq!(
        field_of(&receipt, "format"),
        match shared_report.format {
            yeban_midi::midi::MidiFormat::SingleTrack => "single-track",
            yeban_midi::midi::MidiFormat::Parallel => "parallel",
        },
        "回执的格式名必须来自同一份报告"
    );
    assert!(shared_report.notes > 0, "夹具工程必须有可导出的音符");

    // 不留临时文件（原子落盘的失败模式：`.tmp-` 残留）。
    let leftovers: Vec<String> = std::fs::read_dir(&scratch.dir)
        .expect("列目录")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(yeban_app::save::TEMP_INFIX))
        .collect();
    assert!(leftovers.is_empty(), "残留临时文件: {leftovers:?}");
}

/// `#321` 负向判据：**引号内的键 ⛔ 可被顶层取到**（本文件的取值器也必须引号感知）。
///
/// 修前：`field_of("channels")` 会命中 `loss_detail` 引号内的 `channels=`，取到
/// `2"`（把闭合引号也带进值里）而不是"缺席"。修后必须是 `None`，而 `loss_detail`
/// 仍是整串（引号内的 `=` 不切）。
#[test]
fn a_quoted_value_does_not_expose_its_inner_keys_to_the_top_level() {
    let receipt = concat!(
        r#"module:action=import-midi:outcome=ok:losses="…""#,
        r#":loss_detail="format=1 channels=2""#,
        r#":lyrics_detail="lyrics=2 attached=1 dropped=1 non-utf8=0 empty=0""#
    );
    assert_eq!(field_of_opt(receipt, "channels"), None);
    assert_eq!(field_of_opt(receipt, "format"), None);
    assert_eq!(field_of(receipt, "loss_detail"), "\"format=1 channels=2\"");
    assert_eq!(
        field_of(receipt, "lyrics_detail"),
        "\"lyrics=2 attached=1 dropped=1 non-utf8=0 empty=0\""
    );
}
