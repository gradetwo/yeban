//! ⭐ 缺陷 `D-UI-8`：`--enable-ui-mcp-http` 这一档的**保存入口有没有接线**。
//!
//! ## 缺陷本身（怎么发现的）
//!
//! 第二首曲子要求"只用人能做的动作"创作：真指针事件落到活窗口上的按钮，最后按一下
//! 「保存」。实测（本机 `ui/dispatch_pointer_*` 点 `transport-save-button`，随后 `Ctrl+S`）：
//! **两种手势都毫无反应**，而工程此刻确实有 6 条轨 / 4 个片段 / 96 个音符。磁盘上的
//! `.yeban` 停在播种那一次写入（mtime 与字节一位未变，1187 字节）。
//!
//! 根因不是命中测试，也不是权限：`host::wire_save`（注册 `save-project` 回调的**唯一**
//! 入口）的调用点只有 `main.rs` 的 `run_gui` 与测试目标，而本档在 `main.rs` 里
//! **直接 `return`** 到 `ui_mcp_serve::run` ⇒ `run_gui` 根本不跑 ⇒ 回调**没有注册者**。
//! Slint 的回调没注册 handler 就是空操作。
//!
//! 它比"报错"更坏一层：失败时 `save-status` 一位不写，而状态栏那格文本
//! （`status-bar-save-status`）是 `visible: root.save-status != ""` —— **没有失败就没有那个
//! 节点**。于是自动化界面读不到任何东西（`ui/property` 对不存在的 ID 回
//! `-32006 element-not-found`，而 `ui.value()` 把它吞成 `None`），人只看到"点了没反应"。
//! 差分证据就是这一对：按过保存之后 `status-bar-shortcut-tip` **还在**（它只在
//! `save-status == ""` 时可见）而 `status-bar-save-status` **不在**。
//!
//! ## 判据
//!
//! | # | 断言 | 证什么 | 哪一档 CI 跑 |
//! | :--- | :--- | :--- | :--- |
//! | ① | `src/live_surface.rs` 里有 `host::wire_save(` 这一次注册，且本档的对话框腿是 `NoPicker`；`ui/force_save`（一条**命令**）不许被当成"人能按的保存" | 装配里确实存在保存腿；⛔ 拿命令冒充手势 | 默认档（读源码，不编译 Slint） |
//! | ② | 端到端：`serve_wiring` 那**一份**选项装配活窗口 → 点 `track-create-midi-button`（真指针，轨数 +1）→ 点 `transport-save-button`（真指针）⇒ 磁盘上的容器读回轨数 +1 且 `history_dag` 非空；`save-status` 写了原话、`save-succeeded == true`；`status-bar-save-status` 进树而 `status-bar-shortcut-tip` 出树 | 保存按钮与用户点的是**同一条**链（写的是**此刻**的活会话，不是装配快照），并且回执**在人看得见的地方** | 手动档 `gates-manual.yml` 的 `all-features`（`ui-mcp-http` 不在自动档特性清单里） |
//! | ③ | ⭐ `D-UI-9`：同一份回执必须**从控制面读得回来** —— `ui/property(status-bar-save-status, "value")` 与 `ui/node` 的 `node.value` 都**逐字等于** `window.get_save_status()`（载体是 `ui/status_bar.slint` 上新加的 `accessible-value: root.save-status`） | 人看得见 ≠ 自动化读得到：`accessible-label` 是静态说明句，而上游 `ElementHandle`（i-slint-backend-testing 1.18.1）**没有** `accessible_name()` ⇒ 不声明绑定状态时脚本只能"看见这格出现了"，读不出它写了什么 | 同上（`all-features`，走真 JSON 行协议，⛔ 不走 Rust 侧后门） |
//!
//! ## ② 若变红，怎么读
//!
//! - 红在"磁盘轨数没 +1"而 `save-status` 也仍是空 ⇒ 保存腿又没接上（缺陷本身回来了）；
//! - 红在"磁盘轨数没 +1"但 `save-status` 写了**失败**那句 ⇒ 落盘被拒（锁 / 权限），
//!   那是另一件事，且界面这次**说得出**它；
//! - 红在磁盘 +1 而 `status-bar-save-status` 不在树里 ⇒ 回执没进界面属性（`ROAD-M4-008`
//!   的那条承诺，与 `production_save_ui.rs` 判的是同一格）。
//!
//! ## ③ 若变红，怎么读
//!
//! - 红在 `ui/property` 报**未知属性** ⇒ 属性名写错了（`value` 在支持清单里，见
//!   `yeban-ui-test-port::inspect::property_of`），与界面无关；
//! - 红在读回 `""` 而 `save-status` 有原话 ⇒ `.slint` 那条 `accessible-value` 被摘掉了
//!   （缺陷以另一个 id 回来：节点在树里、文本读不出）；
//! - 红在 `ui/property` 对而 `ui/node` 的 `value` 为 `null` ⇒ 两个通道各自实现了一份，
//!   ⛔ 只修一个（它们必须同源于 `ElementHandle::accessible_value()`）。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 线程局部且每线程只能装一次 ⇒ 本文件只有一处装配
//! （与 `tests/d_ui_1_serve_write_surface.rs` 同款）。

#![allow(unused)]
/// 一次保存的**起点**读数：磁盘字节 + 容器里的轨数。
#[cfg(feature = "ui-mcp-http")]
struct OnDisk {
    bytes: Vec<u8>,
    tracks: usize,
    history_bytes: usize,
}

/// 一个临时目录，`Drop` 时清理（**绝不污染仓库**）。
#[cfg(feature = "ui-mcp-http")]
struct Scratch {
    dir: std::path::PathBuf,
}

#[cfg(feature = "ui-mcp-http")]
impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-d-ui-8-{tag}-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }
}

#[cfg(feature = "ui-mcp-http")]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn crate_source(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join(name),
    )
    .expect("读源码")
}

// ===========================================================================
// 判据 ①：装配里存在保存腿（源码腿 —— 默认档就能跑）
// ===========================================================================
#[test]
fn the_control_plane_assembly_registers_the_save_callback() {
    let surface = crate_source("live_surface.rs");
    assert!(
        surface.contains("host::wire_save("),
        "`build_live_ui_with` 必须调 `host::wire_save(`（`save-project` 回调的唯一注册点）—— \
         不接就是 `D-UI-8`：顶栏「保存」与 `Ctrl+S` 在本档都是空操作"
    );
    // 保存腿的**工程来源**必须是那个活会话端口（与九个写入面同一个 `Rc`），
    // ⛔ 不许用装配时刻的投影快照当唯一来源（那会"保存成功而丢掉这一笔"）。
    assert!(
        surface.contains("options.undo.clone()"),
        "`host::wire_save` 的 `live_port` 必须是 `options.undo`（写出去的是此刻的工程）"
    );
    // 本档**没有 OS 窗口** ⇒ 对话框腿必须问不出路径（`NoPicker`），而不是钉死在模态框上。
    assert!(
        surface.contains("yeban_app::file_picker::NoPicker"),
        "本装配的保存落点只能在装配期给出（`NoPicker`）—— 接 `NativeFilePicker` 会在一个\
         没有 OS 窗口的进程里弹一次没人能答的模态框"
    );
    // ⛔ 拿 `ui/force_save` 冒充"人能按的保存"：那是一条控制面**命令**，
    // 它一直在，而按钮一直点不动 —— 缺陷正是被这句话掩盖的。
    assert!(
        crate_source("ui_mcp_serve.rs").contains("build_live_ui_with"),
        "本档的装配入口必须仍是 `build_live_ui_with`（判据 ① 钉的那次注册要在**真的**被执行的那条路上）"
    );
}

// ===========================================================================
// 判据 ②（端到端，只在 `ui-mcp-http` 里存在）：一次真点击「保存」把活工程写进磁盘
// ===========================================================================
#[cfg(feature = "ui-mcp-http")]
#[test]
fn a_real_click_on_the_save_button_lands_the_live_project_on_disk() {
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
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
    assert!(
        aether_app.contains("> save-succeeded:"),
        "新 UI 契约面必须声明 > save-succeeded:"
    );
    assert!(
        aether_host.contains("set_save_succeeded"),
        "宿主接线层必须注册 set_save_succeeded"
    );
}

/// 读磁盘上那份容器的三个读数（字节 / 轨数 / 图谱字节）。
#[cfg(feature = "ui-mcp-http")]
fn on_disk(path: &std::path::Path) -> OnDisk {
    let bytes = std::fs::read(path).expect("读回容器");
    let archive = yeban_model::container::read_project_container(
        &bytes,
        &yeban_model::container::ContainerLimits::default(),
    )
    .expect("容器读回");
    OnDisk {
        tracks: archive.project.tracks.len(),
        history_bytes: archive.history_dag.len(),
        bytes,
    }
}

/// 这个语义 ID 此刻**在不在**运行时控件树里（走的是 `ui/node`，与判据其余读数同一条路）。
#[cfg(feature = "ui-mcp-http")]
fn has_element(
    plane: &mut yeban_app::live_surface::LiveControlPlane,
    seq: i64,
    element_id: &str,
) -> bool {
    use serde_json::json;
    use yeban_ui_mcp::live::request_line;
    let read = plane.plane().try_line(&request_line(
        seq,
        "ui/node",
        Some(json!({ "elementId": element_id })),
    ));
    !read.is_error()
}

/// 一次**真实**点击：读几何 → 按下 → 移到同一点 → 松手（与 `D-UI-1` 判据同一个助手，
/// ⛔ 不调 `invoke_*` —— 那会绕开命中测试，测不到"人够不够得着"）。
#[cfg(feature = "ui-mcp-http")]
fn click(plane: &mut yeban_app::live_surface::LiveControlPlane, seq: i64, element_id: &str) {
    use serde_json::json;
    use yeban_ui_mcp::live::request_line;
    use yeban_ui_mcp::methods::{
        METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE, METHOD_DISPATCH_POINTER_UP,
    };

    let node = plane.plane().try_line(&request_line(
        seq,
        "ui/node",
        Some(json!({ "elementId": element_id })),
    ));
    assert!(
        !node.is_error(),
        "`{element_id}` 必须在运行时树里: {node:?}"
    );
    let bounds = node.result.expect("有 result")["node"]["bounds"].clone();
    let x_offset = bounds["width"].as_f64().expect("width") / 2.0;
    let y_offset = bounds["height"].as_f64().expect("height") / 2.0;
    let absolute_x = bounds["x"].as_f64().expect("x") + x_offset;
    let absolute_y = bounds["y"].as_f64().expect("y") + y_offset;

    let down = plane.plane().try_line(&request_line(
        seq + 1,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": element_id,
            "xOffset": x_offset,
            "yOffset": y_offset,
            "button": "left"
        })),
    ));
    assert!(
        !down.is_error(),
        "`{element_id}` 按下必须落到真实窗口: {down:?}"
    );
    let moved = plane.plane().try_line(&request_line(
        seq + 2,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({ "x": absolute_x, "y": absolute_y })),
    ));
    assert!(
        !moved.is_error(),
        "`{element_id}` 移动必须落到真实窗口: {moved:?}"
    );
    let up = plane.plane().try_line(&request_line(
        seq + 3,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({ "button": "left" })),
    ));
    assert!(
        !up.is_error(),
        "`{element_id}` 松手必须落到真实窗口: {up:?}"
    );
}
