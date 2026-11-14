//! 判据：缺陷 `D-UI-11` —— **`--enable-ui-mcp-http` 那份装配没接轨道头的三条手势**
//!
//! ## 症状（活体实测，2026-11-01，构建 `38ead231` = `5e9dc6a2` + 一次性构建作业）
//!
//! 用 UI 控制面（模拟人类手势）创作第二首时，15 颗剪辑**全部**落在第 1 条轨，
//! 其余 5 条轨一颗都没有。探针（`probe_d11.py`）在同一份活体上把四条路逐个试完：
//!
//! | 路 | 手势 | 读数 `device-rack-track-label` |
//! | :--- | :--- | :--- |
//! | A | `ui/dispatch_pointer_down("track-2-header", 中心)` → move → up | `轨 MIDI 1 · 设备链`（不动） |
//! | B | 同行**空白处**点按 (60, 60) | 不动 |
//! | B2 | 同行**标题文本**上点按 (60, 14) | 不动 |
//! | C | `ui/dispatch_key_chord("Down", alt=true)` | 不动，而返回值是 **`consumed = true`** |
//!
//! C 的 `consumed = true` 是最坏的一格：调用方读到"这一键有绑定"，实际**一位没动**。
//!
//! ## 根因（不在手势、不在命中测试、不在投影）
//!
//! `src/main.rs` 的 `main()` 在 `options.ui_mcp_http` 时**提前 `return`** 进
//! `ui_mcp_serve::run` ⇒ `run_gui` 那段接线（含 `host::wire_track_select`、
//! `host::wire_track_height_drag`、`host::wire_undo`）**一行都不执行**。
//! 而 `src/live_surface.rs::build_live_ui_with` 是**手抄的第二份清单**，恰好漏掉这三条 ——
//! 它接了九个 `wire_*` 的写入面（D-UI-1）与保存（D-UI-8），独独没接这三条。
//! Slint 的回调没注册 handler = 空操作 ⇒ 在这条执行面上「当前轨」**没有写者**，
//! `bridge::display_track` 每次回落到第一条非主总线轨 ⇒ 建片段 / 铅笔提交恒指向 MIDI 1。
//!
//! ## 为什么此前的判据一条都没抓到
//!
//! `tests/track_selection.rs` 里那条**名字叫 `clicking_the_track_header_...`** 的判据
//! 调的是 `handle.invoke_accessible_default_action()`（无障碍默认动作），⛔ 不是指针事件；
//! 而 `ui/dispatch_pointer_down` 走 `window.pointer_down(窗口坐标)` ⇒ Slint 命中测试，
//! 落在该行的 `TouchArea` 上 —— **两条不同的路**。更关键的是判据侧各自**手工补一次**
//! `host::wire_track_select`（`track_selection.rs` 的 `live_with_port`、
//! `track_select_keyboard.rs` 的 `live_with_keys_and_select`、`mixer_keyboard_nudge.rs`、
//! `mixer_keyboard_switch.rs`），所以"这份装配到底接没接"
//! 在判据里永远读不出来。⇒ 本文件判据 ① 把这件事变成**机械对账**。
//!
//! ## 判据
//!
//! | # | 断言 | 证什么 | 哪一档 CI 跑 |
//! | :--- | :--- | :--- | :--- |
//! | ① | `main.rs` 里出现的每一个 `host::wire_*(` 调用，要么在 `live_surface.rs` 里也在场，要么在**写明理由**的例外清单里（原生文件对话框绑定） | 两份清单不再漂移；将来给 `run_gui` 加一条接线而忘了 serve 侧 ⇒ 红 | 默认档（读源码，不编译 Slint） |
//! | ② | serve 装配（`build_live_ui_with` + 带端口的选项）上，一次**真指针事件**点 `track-1-header` ⇒ `selected-track-id` 变成第 2 条轨的身份、投影的设备链标题跟着走，且 `commit_count` Δ0 | 用户点行与注入点行同一条链；选中⛔ 不是工程编辑 | 默认档 |
//! | ③ | 同一次装配上先点行、再点 `clip-create-button` ⇒ 新摆放落在**被选中的那条轨**上，第 1 条轨的摆放数一位不动 | 直接钉住创作时撞上的症状（"15 颗全在第 1 条轨"） | 默认档 |
//! | ④ | 同一装配上 `Alt+↓` 真的把选中态推到下一条轨（`consumed` 与状态**同时**成立） | 把 C 那一格（"说消费、没动"）钉死 | 默认档 |
//! | ⑤ | 同一装配上按住行头**在本行内**向上拖 `rise` px ⇒ 那一行的注入行高**正好**长 `rise` px，且拖拽态两位（进行中 / 收尾清零）分别成立 | 行高这一条同样漏接的入口（`wire_track_height_drag`）；两位状态读数把"回调没转发"与"收尾没做"分开 | 默认档 |
//!
//! ## 若变红，怎么读
//!
//! - ① 红 ⇒ 有人往 `run_gui` 加了接线而 serve 侧没跟上：要么补接线，要么把**理由**写进
//!   `ASSEMBLY_ONLY_IN_GUI`（那一格必须写清**缺的是哪个运行期对象**——到现在为止只剩
//!   原生文件对话框，⛔ 不许写"判据太严"。两张先例值得记住：`D-UI-30` 之前有一条写
//!   "需要引擎句柄"的 `host::wire_transport`、`D-UI-31` 之前有三条写"需要活投影
//!   `Rc<RefCell<ViewState>>`"的定位接线 —— 句柄与投影装配**都给得出**（把字段升成
//!   `Rc<RefCell<..>>` 即可），那两批例外都是**被清单掩护着的缺陷**）；
//! - ②③④⑤ 任一条红而 ① 绿 ⇒ 接线在场但**回调没落到状态**，那是 `host::wire_*` 自己的回退，
//!   与本条修复同源；
//! - ② 红在"命中不到"（`ui/node` 报 no bounds）⇒ 编排行几何漂了，先看 ① 之外的第三条线
//!   `tests/track_selection.rs` 是否同红。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 线程局部且每线程只能装一次 ⇒ 本文件只有**一处**装配
//! 入口（`Rig::new`），五条判据里需要窗口的那四条各自调它（每个 `#[test]` 一个新线程）。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用 `live_surface.rs` 的装配那一半 ⇒ 其余项在本测试目标里是 dead_code，
// 而 `-D warnings` 会判红（与 `tests/track_select_keyboard.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use serde_json::json;
use slint::ComponentHandle as _;
use slint::Model as _;
use yeban_app::host;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::project::{TrackV3, YebanProjectV1};
use yeban_model::samples::filled_project;
use yeban_ui_mcp::live::request_line;
use yeban_ui_mcp::methods::{
    METHOD_DISPATCH_KEY_CHORD, METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE,
    METHOD_DISPATCH_POINTER_UP, METHOD_NODE,
};
use yeban_ui_test_port::port::Permission;

/// 会话打开时刻（与其余界面判据同一个常量，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 打在轨道头上的**横向**落点（元素局部像素）。
///
/// ⚠ 不用"行宽的一半"：编排行里静音 / 独奏两块的命中区是 `x 16..38, y 26..44`，
/// 而它们是包头的**后**置子元素（Slint 自前往后命中 ⇒ 按钮先拿走点击，行头的拖拽面
/// 只接管剩下的区域）。行宽随视口断点变，"半宽"既可能落进按钮区，也可能随断点漂移，
/// 于是"点击被按钮吃掉了"会和"接线没接"混在一起读不出来。
/// `150.0` 是 `tests/live_ui_mcp.rs` 那条**已验证可用**的轨道头拖拽判据所用的同一个值
/// （标题文本止于 `x 112`、色标止于 `x 134`，两者都是非交互元素）。
const HEADER_SAFE_X: f64 = 150.0;

/// `filled_project()` 的三条非主总线轨（身份升序 = 界面下标序）：
/// `0 = Lead`（MIDI）／`1 = Bass`（音频）／`2 = Aux Reverb`。
///
/// ⚠ 与 `tests/track_selection.rs` 的 `fixture_tracks` 同一个解析 —— 那里说
/// "身份升序 = 界面下标序"，这里就用它（判据 ② 的前置断言把它钉在窗口自己的
/// `track-ids` 列上，⛔ 不是只信这个解析）。
fn fixture_tracks(project: &YebanProjectV1) -> Vec<TrackV3> {
    project
        .tracks
        .values()
        .filter(|track| track.id != project.master_bus_track_id)
        .cloned()
        .collect()
}

/// 读一份 `src/` 下的源码（判据 ① 的对账对象）。
fn crate_source(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join(name),
    )
    .expect("读源码")
}

/// 把源码里的 `host::wire_xxx(` **调用点**收集成名单。
///
/// ⚠ 先按行剥掉 `//` 之后的注释再匹配：这两份文件里到处是
/// "与产品进程调的是**同一个** `host::wire_track_select`"这种**提及**，
/// 把提及算成调用会让守卫永远绿（那正是它要抓的那种假象）。
/// ⚠ 只认带左括号的形态：`host::wire_module_actions` 是
/// `host::wire_module_actions_for_project` 的前缀，不带括号就会把一条算成两条。
fn wiring_calls(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    for raw in source.lines() {
        let code = raw.split("//").next().unwrap_or("");
        let mut cursor = 0usize;
        while let Some(at) = code[cursor..].find("host::wire_") {
            let start = cursor + at + "host::wire_".len();
            let tail = &code[start..];
            let ident_len = tail
                .find(|c: char| !(c.is_ascii_lowercase() || c == '_'))
                .unwrap_or(tail.len());
            let name = format!("host::wire_{}", &tail[..ident_len]);
            if tail[ident_len..].starts_with('(') && !names.contains(&name) {
                names.push(name);
            }
            cursor = start + ident_len;
        }
    }
    names.sort();
    names
}

/// 例外清单（GUI → serve 方向）：这些接线在 serve 装配里**结构上装不出来**，理由逐条写死。
///
/// ⭐ `D-UI-30`（2026-11-03）把 `host::wire_transport` 从这张表里**删掉了** —— 它当时
/// 记的理由是"需要引擎句柄"，而句柄从来不是障碍：`LiveAdminSurface` 本来就**持有**一个
/// `EngineHost`，缺的只是把它装成 `Rc<RefCell<..>>`（与 `host::ProductionLoop` 同款）。
/// 这条例外因此是**被清单掩护着的缺陷**，正是本守卫要抓的形状。
///
/// ⭐ `D-UI-31`（同日）把**同一族**的第二批删掉：`host::wire_ruler_seek` /
/// `host::wire_section_seek` / `host::wire_scene_launch` 三条当时记的理由是"需要活投影
/// `Rc<RefCell<ViewState>>`（本装配存裸值）"—— 这句**本身没错**，错在把"现在没有"写成
/// "结构上装不出来"：把裸值升成 `Rc<RefCell<..>>` 正是那一票的内容（且顺手修掉了
/// `wire_sidebar_contents` / `wire_clip_scope` 各持一份**装配期快照**的陈旧缺陷，
/// 那是 `D-UI-10` 那一族）。判据见 `tests/d_ui_31_serve_seek_is_wired.rs`。
///
/// 剩下的条目都缺一个**原生文件对话框**：无头执行面问不出路径（与 `host::wire_save`
/// 在 serve 侧用 `NoPicker` 同一口径）。
///
/// ⚠ 名字必须是 [`wiring_calls`] 吐出的**裸名**（不带 `(`），且两族相邻名字不能靠
/// 前缀匹配 —— `host::wire_save` 是 `host::wire_save_with_engine` 的前缀，
/// 因此下面的对账一律用**全等**。
const ASSEMBLY_ONLY_IN_GUI: &[(&str, &str)] = &[
    ("host::wire_save_with_engine", "需要原生文件对话框"),
    ("host::wire_new_project", "需要原生文件对话框"),
    ("host::wire_open_project", "需要原生文件对话框"),
    ("host::wire_import_audio", "需要原生文件对话框"),
    ("host::wire_mixer_sampler_browse", "需要原生文件对话框"),
];

/// 例外清单（serve → GUI 方向）：无头面**特有**的接线。
///
/// `host::wire_save` 是 serve 面的保存腿 —— 落点在装配期就定死（`D-UI-8` 的
/// `NoPicker`：问不出路径，因此不问），产品 GUI 走的是 `host::wire_save_with_engine`
/// （原生对话框 + 引擎目录重定向）。两条腿的**判据不同**但**不能互删**，故在此留痕。
const ASSEMBLY_ONLY_IN_SERVE: &[(&str, &str)] = &[(
    "host::wire_save",
    "无头面的保存腿：`NoPicker` ⇒ 落点恒为装配期那个文件（`D-UI-8`）；\
         产品面走 `host::wire_save_with_engine`（原生对话框 + 引擎目录重定向）",
)];

/// 在全等语义下查一条接线的例外理由（没有 ⇒ `None`）。
///
/// 两族名字互为前缀（`host::wire_save` ⊂ `host::wire_save_with_engine`），
/// 因此这里**只用全等**，`starts_with` 会把"缺对话框的那条"错配到"缺引擎的那条"上。
fn exception<'a>(name: &str, list: &[(&'a str, &'a str)]) -> Option<&'a str> {
    list.iter()
        .find(|(listed, _)| *listed == name)
        .map(|(_, why)| *why)
}

// ===========================================================================
// 判据 ①：两份接线清单机械对账（默认档，⛔ 不编译 Slint）
// ===========================================================================
#[test]
fn the_serve_surface_wires_every_wiring_the_gui_path_can_wire() {
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
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
    assert!(
        aether_app.contains("> track-height-drag-active:"),
        "新 UI 契约面必须声明 > track-height-drag-active:"
    );
    assert!(
        aether_host.contains("set_track_height_drag_active"),
        "宿主接线层必须注册 set_track_height_drag_active"
    );
    assert!(
        aether_app.contains("> track-height-drag-track:"),
        "新 UI 契约面必须声明 > track-height-drag-track:"
    );
    assert!(
        aether_host.contains("set_track_height_drag_track"),
        "宿主接线层必须注册 set_track_height_drag_track"
    );
}

// ===========================================================================
// 装配：与 `--enable-ui-mcp-http` 测试面**同一份**选项（带端口）
// ===========================================================================
/// 一次 serve 装配 + 控制面（真指针 / 真组合键都从这里注入）。
struct Rig {
    window: MainWindow,
    port: Rc<UndoPort>,
    plane: live::LiveControlPlane,
    seq: i64,
}

impl Rig {
    fn new() -> Self {
        let project = filled_project();
        let port = Rc::new(UndoPort::new(
            UndoSession::open("<判据 D-UI-11>", "yeban-app", project.clone(), NOW_MS)
                .expect("打开撤销会话"),
        ));
        let options = live::LiveWiringOptions {
            permission: Permission::Administrative,
            console_tab: 0,
            save_path: None,
            engine_quanta: 0,
            undo: Some(Rc::clone(&port)),
            ..live::LiveWiringOptions::default()
        };
        let built = live::build_live_ui_with(&project, &options).expect("Tier-1 装配");
        // 窗口的共享句柄（`Rc` 语义，不是第二份窗口状态）：下面要读窗口属性。
        // ⚠ `built` 随后被 `into_control_plane` 消耗掉，活组件由控制面撑着 ——
        // `clone_strong` 这一步与 `tests/track_select_keyboard.rs` 的
        // `live_with_keys_and_select` 同款（那里额外留一个 `LiveUi`，因为它不建控制面）。
        let window = built.ui().clone_strong();
        let plane = built.into_control_plane(Permission::Administrative);
        Self {
            window,
            port,
            plane,
            seq: 1,
        }
    }

    fn line(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let answer = self
            .plane
            .plane()
            .try_line(&request_line(self.seq, method, Some(params)));
        self.seq += 1;
        assert!(
            !answer.is_error(),
            "`{method}` 必须被执行面接受: {answer:?}"
        );
        answer.result.expect("有 result")
    }

    /// 按下之前**现读**一次几何（缓存的包围盒会随重投影过期）。
    fn bounds(&mut self, element_id: &str) -> (f64, f64, f64, f64) {
        let node = self.line(METHOD_NODE, json!({ "elementId": element_id }));
        let bounds = node["node"]["bounds"].clone();
        assert!(
            !bounds.is_null(),
            "`{element_id}` 必须在运行时树里带几何: {node}"
        );
        (
            bounds["x"].as_f64().expect("x"),
            bounds["y"].as_f64().expect("y"),
            bounds["width"].as_f64().expect("width"),
            bounds["height"].as_f64().expect("height"),
        )
    }

    /// 一次**真指针**点按：down(元素局部坐标) → move(窗口绝对坐标) → up。
    ///
    /// ⛔ 不是 `invoke_accessible_default_action()`：那条走无障碍动作，本判据要的是
    /// 用户手指 / 鼠标那一条命中测试路径（`D-UI-11` 的症状只在这条路上出）。
    fn tap(&mut self, element_id: &str, local_x: f64, local_y: f64) {
        let (x, y, _, _) = self.bounds(element_id);
        self.line(
            METHOD_DISPATCH_POINTER_DOWN,
            json!({
                "elementId": element_id,
                "xOffset": local_x,
                "yOffset": local_y,
                "button": "left",
            }),
        );
        self.line(
            METHOD_DISPATCH_POINTER_MOVE,
            json!({ "x": x + local_x, "y": y + local_y }),
        );
        self.line(METHOD_DISPATCH_POINTER_UP, json!({ "button": "left" }));
    }

    /// 按钮一族的点法：**现读几何 → 打在中心**（与创作 harness 的 `ui.click` 同一落点）。
    fn tap_center(&mut self, element_id: &str) {
        let (_, _, w, h) = self.bounds(element_id);
        self.tap(element_id, w / 2.0, h / 2.0);
    }

    /// 轨道行的点法：纵向取行中，横向取 [`HEADER_SAFE_X`]（⛔ 不用半宽，理由见该常量）。
    fn tap_header(&mut self, element_id: &str) {
        let (_, _, _, h) = self.bounds(element_id);
        self.tap(element_id, HEADER_SAFE_X, h / 2.0);
    }

    /// 只按下（不松手）：拖拽的起手。
    fn tap_start(&mut self, element_id: &str, local_x: f64, local_y: f64) {
        self.line(
            METHOD_DISPATCH_POINTER_DOWN,
            json!({
                "elementId": element_id,
                "xOffset": local_x,
                "yOffset": local_y,
                "button": "left",
            }),
        );
    }

    /// 一次**窗口绝对坐标**的移动（`ui/dispatch_pointer_move` 的口径就是绝对值）。
    fn pointer_move(&mut self, absolute_x: f64, absolute_y: f64) {
        self.line(
            METHOD_DISPATCH_POINTER_MOVE,
            json!({ "x": absolute_x, "y": absolute_y }),
        );
    }

    fn pointer_up(&mut self) {
        self.line(METHOD_DISPATCH_POINTER_UP, json!({ "button": "left" }));
    }

    /// 手势状态两位（`ADR-0004` S1：拖拽态由宿主持有，⛔ 不是像素）。
    fn drag_state(&self) -> (bool, String) {
        (
            self.window.get_track_height_drag_active(),
            self.window.get_track_height_drag_track().to_string(),
        )
    }

    /// `ui/dispatch_key_chord` 的返回值 = **界面是否消费**（`D-UI-11` 的 C 那一格）。
    fn chord(&mut self, key_name: &str, alt: bool) -> bool {
        let answer = self.line(
            METHOD_DISPATCH_KEY_CHORD,
            json!({ "keyName": key_name, "shift": false, "ctrl": false, "alt": alt, "meta": false }),
        );
        answer["consumed"].as_bool().unwrap_or(false)
    }

    fn selected(&self) -> String {
        self.window.get_selected_track_id().to_string()
    }

    /// 注入的行高数组（`[length]` → `Vec<f64>`，与 `tests/live_ui_mcp.rs` 的
    /// `injected_lengths` 同一读数，只是升位宽便于做增量比较）。
    fn heights(&self) -> Vec<f64> {
        self.window
            .get_track_heights()
            .iter()
            .map(f64::from)
            .collect()
    }

    /// 窗口自己那一列**非主总线**轨身份（`track-ids`，与 `track-{i}-header` 同索引集）。
    ///
    /// 判据不猜顺序：行下标 ↔ 模型下标这条对齐由判据 ② 的前置断言现读现证。
    fn track_ids(&self) -> Vec<String> {
        self.window
            .get_track_ids()
            .iter()
            .map(|id| id.to_string())
            .collect()
    }

    fn placements(&self, track_id: yeban_model::ids::EntityId) -> usize {
        self.port
            .project()
            .tracks
            .get(&track_id)
            .map_or(0, |track| track.clips.len())
    }
}

// ===========================================================================
// 判据 ②：真指针点轨道行 ⇒ 选中态真的动
// ===========================================================================
#[test]
fn a_real_pointer_click_on_a_track_header_selects_it_on_the_serve_surface() {
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
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
    assert!(
        aether_app.contains("> track-height-drag-active:"),
        "新 UI 契约面必须声明 > track-height-drag-active:"
    );
    assert!(
        aether_host.contains("set_track_height_drag_active"),
        "宿主接线层必须注册 set_track_height_drag_active"
    );
    assert!(
        aether_app.contains("> track-height-drag-track:"),
        "新 UI 契约面必须声明 > track-height-drag-track:"
    );
    assert!(
        aether_host.contains("set_track_height_drag_track"),
        "宿主接线层必须注册 set_track_height_drag_track"
    );
}

// ===========================================================================
// 判据 ③：选好轨再「新建剪辑」⇒ 摆放落在被选中的那条轨（钉住创作现场的症状）
// ===========================================================================
#[test]
fn a_clip_created_after_selecting_a_header_lands_on_that_track_not_the_first_one() {
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
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
    assert!(
        aether_app.contains("> track-height-drag-active:"),
        "新 UI 契约面必须声明 > track-height-drag-active:"
    );
    assert!(
        aether_host.contains("set_track_height_drag_active"),
        "宿主接线层必须注册 set_track_height_drag_active"
    );
    assert!(
        aether_app.contains("> track-height-drag-track:"),
        "新 UI 契约面必须声明 > track-height-drag-track:"
    );
    assert!(
        aether_host.contains("set_track_height_drag_track"),
        "宿主接线层必须注册 set_track_height_drag_track"
    );
}

// ===========================================================================
// 判据 ④：`Alt+↓` 在这份装配上真的换轨（⛔ 不许只报 `consumed = true`）
// ===========================================================================
#[test]
fn alt_down_on_the_serve_surface_moves_the_selection_and_not_only_reports_consumed() {
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
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
    assert!(
        aether_app.contains("> track-height-drag-active:"),
        "新 UI 契约面必须声明 > track-height-drag-active:"
    );
    assert!(
        aether_host.contains("set_track_height_drag_active"),
        "宿主接线层必须注册 set_track_height_drag_active"
    );
    assert!(
        aether_app.contains("> track-height-drag-track:"),
        "新 UI 契约面必须声明 > track-height-drag-track:"
    );
    assert!(
        aether_host.contains("set_track_height_drag_track"),
        "宿主接线层必须注册 set_track_height_drag_track"
    );
}

// ===========================================================================
// 判据 ⑤：按住行头向上拖 ⇒ 那一行的行高真的变（`wire_track_height_drag`）
// ===========================================================================
#[test]
fn dragging_a_track_header_resizes_that_row_on_the_serve_surface() {
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
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
    assert!(
        aether_app.contains("> track-height-drag-active:"),
        "新 UI 契约面必须声明 > track-height-drag-active:"
    );
    assert!(
        aether_host.contains("set_track_height_drag_active"),
        "宿主接线层必须注册 set_track_height_drag_active"
    );
    assert!(
        aether_app.contains("> track-height-drag-track:"),
        "新 UI 契约面必须声明 > track-height-drag-track:"
    );
    assert!(
        aether_host.contains("set_track_height_drag_track"),
        "宿主接线层必须注册 set_track_height_drag_track"
    );
}
