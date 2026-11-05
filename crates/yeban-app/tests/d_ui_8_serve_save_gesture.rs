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
    use yeban_app::cli::{Loaded, ProjectSource};
    use yeban_app::live_surface::build_live_ui_with;
    use yeban_app::ui_mcp_serve::serve_wiring;
    use yeban_app::undo::UndoPort;
    use yeban_model::CommitGraph;
    // ⛔ 读回容器的那两个名字在**助手** `on_disk` 里全路径限定：模块级函数看不见本函数作用域的 import。
    use yeban_model::container::write_project_container;
    use yeban_model::samples::demo_project;
    use yeban_ui_test_port::port::Permission;
    use yeban_ui_test_port::render::report_line;

    // 会话起点时间戳（固定值 ⇒ 判据可复现）；放在函数体内是为了不触发默认档的 `dead_code`。
    const NOW: u64 = 1_760_000_000_000;

    let scratch = Scratch::new("save");
    let target = scratch.dir.join("serve.yeban");
    let project = demo_project();
    let starting_graph = CommitGraph::new();
    let seed = write_project_container(
        &project,
        &serde_json::to_vec(&starting_graph).expect("起始图谱 JSON"),
        &std::collections::BTreeMap::new(),
    )
    .expect("写起始容器");
    std::fs::write(&target, &seed).expect("落起始容器");

    // ---- 1. 与 `--open <path>` 逐字同一条读取路径，构造 serve 侧的装配选项 ----
    let opened = yeban_app::open::open_project_document_file(&target, &Default::default())
        .expect("打开起始容器");
    let loaded = Loaded {
        archive: opened.archive,
        source: ProjectSource::File {
            path: target.clone(),
            bytes: opened.file_bytes,
        },
    };
    // 端口按 `run` 在**测试面**（两道门都过）那条路逐字构造：同一个入口、同一份资产池。
    // `ADR-0057` 选项 A 之后，"要不要建会话"的判定留在 `run` 那道闸上，`serve_wiring` 退化成
    // **纯装配** ⇒ 本判据必须自己把端口递进去，⛔ 不能再假设装配器内部替我建会话。
    let undo_port = std::rc::Rc::new(UndoPort::new(
        yeban_app::undo::open_undo_session(&loaded.undo_source(), &loaded, NOW)
            .expect("测试面会话（与 run 同一个入口）"),
    ));
    undo_port.replace_assets(loaded.archive.assets.clone());
    let wiring = serve_wiring(&loaded, Permission::Administrative, Some(undo_port));
    let port = wiring
        .undo
        .clone()
        .expect("D-UI-1 的运行时版：端口必须在选项里");
    let tracks_before = port.project().tracks.len();

    // ---- 2. 拿这**一份**选项装配活窗口（与 `run` 的调用逐字相同） ----
    let live = build_live_ui_with(&loaded.archive.project, &wiring).expect("Tier-1 装配");
    let window = slint::ComponentHandle::clone_strong(live.ui());
    let mut plane = live.into_control_plane(Permission::Administrative);

    // ---- 3. 起点：还没按过保存 ⇒ 磁盘一位未动，回执格不在树里，提示格在树里 ----
    let disk_before = on_disk(&target);
    assert_eq!(
        disk_before.bytes,
        seed,
        "起点：保存按钮一次都没按过 ⇒ 文件必须还是播种那一份（实得 {} 字节，播种 {} 字节）",
        disk_before.bytes.len(),
        seed.len()
    );
    assert!(
        !has_element(&mut plane, 10, "status-bar-save-status"),
        "起点：`save-status` 为空 ⇒ `status-bar-save-status` 不该在树里（它是 `visible: save-status != \"\"`）"
    );
    assert!(
        has_element(&mut plane, 11, "status-bar-shortcut-tip"),
        "起点：`save-status` 为空时可见的必须是快捷键提示那一格"
    );
    assert_eq!(window.get_save_status().to_string(), "");
    assert!(!window.get_save_succeeded());

    // ---- 4. 先真指针点一次「新建 MIDI 轨」：要保存的必须是**改过之后**的那一版 ----
    click(&mut plane, 20, "tab-devices-button");
    click(&mut plane, 30, "track-create-midi-button");
    assert_eq!(
        port.project().tracks.len(),
        tracks_before + 1,
        "夹具：一次点击必须给工程多一条轨（前 {tracks_before}）"
    );

    // ---- 5. 一次**真指针**点击「保存」：缺陷 D-UI-8 的症状就在这一笔 ----
    click(&mut plane, 40, "transport-save-button");

    // 5a. 界面回执（人看得见的那一句 + 屏读器可达的那个节点）。
    let status = window.get_save_status().to_string();
    assert!(
        status.starts_with("已保存"),
        "按过保存后状态文本必须是回执原话，实得: {status:?} —— \
         空字符串就意味着保存腿又没人注册（缺陷本身）"
    );
    assert!(
        window.get_save_succeeded(),
        "`save-succeeded` 必须为真（回执是成功而不是失败）: {status}"
    );
    assert!(
        has_element(&mut plane, 50, "status-bar-save-status"),
        "回执必须以独立语义节点进运行时控件树（`[UI-TEST-001]` 的语义寻址）"
    );
    assert!(
        !has_element(&mut plane, 51, "status-bar-shortcut-tip"),
        "两格共用同一块宽度：写了回执 ⇒ 提示格必须让位（同一时刻只有一个在画）"
    );

    // 5a'. ⭐ 缺陷 **D-UI-9**：回执**文本**本身必须是自动化读得出的东西（不是只有「这格出现了」）。
    // `label` 是静态说明句，上游 `ElementHandle`（i-slint-backend-testing 1.18.1）**没有**
    // `accessible_name()` ⇒ 唯一诚实的通道是把它再声明成 `[ARCH-UI-004]` 的绑定状态。
    // 这里走的是与创作脚本**同一个** JSON 行协议（⛔ 不碰 Rust 侧的 `window.get_*`）。
    {
        use serde_json::json;
        use yeban_ui_mcp::live::request_line;
        let via_property = plane.plane().try_line(&request_line(
            52,
            "ui/property",
            Some(json!({ "elementId": "status-bar-save-status", "name": "value" })),
        ));
        assert!(
            !via_property.is_error(),
            "`ui/property(..., \"value\")` 必须读得到（读不到 = 这一格没声明绑定状态）: {via_property:?}"
        );
        let read_back = via_property.result.expect("有 result")["value"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert_eq!(
            read_back, status,
            "控制面读回的 `value` 必须逐字等于人看见的那句回执 —— \
             读不回文本时脚本只能「看见有一格」，看不见写了什么（D-UI-9）"
        );
        let via_node = plane.plane().try_line(&request_line(
            53,
            "ui/node",
            Some(json!({ "elementId": "status-bar-save-status" })),
        ));
        assert_eq!(
            via_node.result.expect("有 result")["node"]["value"].as_str(),
            Some(status.as_str()),
            "`ui/node` 的 `value` 字段必须同样带回那句回执（两个通道同一个来源，⛔ 只修一个）"
        );
    }

    // 5b. 磁盘：写出去的是**此刻**的活会话（轨数 +1、图谱非空），不是装配快照。
    let disk_after = on_disk(&target);
    assert_ne!(
        disk_after.bytes, disk_before.bytes,
        "保存必须在磁盘上留下可见的一笔（文件字节一位没变 = 回调没注册者）"
    );
    assert_eq!(
        disk_after.tracks,
        tracks_before + 1,
        "磁盘上必须真的多出一条轨（前 {tracks_before}，实得 {}）—— \
         写装配时刻的快照就是丢掉这一笔",
        disk_after.tracks
    );
    assert!(
        disk_after.history_bytes > 0,
        "落盘的 `history_dag` 必须带上那一步提交的字节（`encode_history_dag(端口图谱)`，\
         实得 {history} 字节）",
        history = disk_after.history_bytes
    );

    report_line(&format!(
        "[D-UI-8] 真指针点击「保存」：权威 {} → {} 轨；磁盘 {} → {} 字节、{} → {} 轨、\
         history_dag {} 字节；回执={:?}；控件树里 status-bar-save-status 已由缺失变为存在",
        tracks_before,
        port.project().tracks.len(),
        disk_before.bytes.len(),
        disk_after.bytes.len(),
        disk_before.tracks,
        disk_after.tracks,
        disk_after.history_bytes,
        status,
    ));
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
