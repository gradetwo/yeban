//! ⭐ 缺陷 `D-UI-1`：`--enable-ui-mcp-http` 这一档的**写入面有没有接线**。
//!
//! ## 缺陷本身（怎么发现的）
//!
//! 第二首曲子要用"模拟人类"的 UI 控制面创作：真指针事件落到活窗口上的按钮。实测
//! （本机 `ui/dispatch_pointer_*` + `ui/force_save` 前后digest）是：控制台里**九个**改工程
//! 的按钮（建轨 / 装设备 / 建片段 / 混音台 / 发送条 / 宏 / BPM / 卷帘铅笔 / 剪辑拖动）
//! 点下去**全部无响应** —— 既不提交、也不报错，而按钮照投影画成"可用"
//! （`Tokens.gold-deep` 填充）。根因不是命中测试：
//! [`yeban_app::live_surface`] 把那九个 `host::wire_*` 整体 gate 在
//! [`LiveWiringOptions::undo`] 上，而 `src/ui_mcp_serve.rs` 给了 `None`。
//! Slint 的回调没注册 handler 就是空操作 ⇒ "界面装好了"与"改得动不动工程"是两件事。
//!
//! ## 判据
//!
//! | # | 断言 | 证什么 | 哪一档 CI 跑 |
//! | :--- | :--- | :--- | :--- |
//! | ① | `run` 把测试面开出的端口 `test_undo` 喂给 `serve_wiring`，且 `src/ui_mcp_serve.rs` 里不再出现 `undo: None`；会话走**生产同一个**入口 `open_undo_session`（其配方只有一份，在 `undo::session_from_archive`）；端口带上归档的资产池 | serve 侧不再"装配一个不能写的界面" | 默认档（读源码，不编译 Slint） |
//! | ② | `session_from_archive` 把归档里的 `history.dag` **读回**成一条活的撤销链（重开后 `undoable == 1`、撤销回到起点）；空 `dag` ⇒ 新会话；非法 `dag` ⇒ **如实报错**，不降级成空历史 | 修复没把"撤销历史"这条路写窄 | 默认档 |
//! | ③ | 端到端：拿 ① 的那**一份**选项装配活窗口 → 点 `tab-devices-button`（真指针）→ 点 `track-create-midi-button`（真指针）⇒ 端口的工程轨数 +1、`ui/force_save` 回执 `writtenFromUndoPort = true`、磁盘上的容器读回轨数 +1 | 注入的点击与用户点击**同一条** `Op` + `UndoPort` 链，且保存写的是端口那一版而不是装配缓存 | 手动档 `gates-manual.yml` 的 `all-features`（`ui-mcp-http` 不在自动档特性清单里，见 `.github/workflows/ci.yml` 的 `APP_FEATURES` 注释） |
//!
//! ## ③ 若变红，怎么读
//!
//! - 红在"轨数没 +1"而 `serve_wiring` 的 `undo.is_some()`（①）绿 ⇒ 命中测试层面另有缺陷
//!   （按钮被别的层挡住 / 几何没进树），**另立条目**，不是本条修复的回退；
//! - 红在 `writtenFromUndoPort = false` ⇒ `save_now` 又退回写装配缓存（那正是"保存成功、
//!   文件里却没有这一笔"的原始症状）。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 线程局部且每线程只能装一次 ⇒ 本文件只有一处装配
//! （与 `tests/ui_bpm_field.rs` 同款）。

#![allow(unused)]
use yeban_app::undo::{UndoPort, session_from_archive};
use yeban_model::CommitGraph;
use yeban_model::container::ProjectArchive;
use yeban_model::samples::filled_project;

/// 一次提交用的时间戳（与其余界面判据同一个值，⛔ 不引入时钟依赖）。
const NOW: u64 = 1_760_000_000_000;

/// 一个临时目录，`Drop` 时清理（**绝不污染仓库**）。只在端到端那一腿需要（默认档里
/// 没有装配入口 ⇒ 不加 cfg 会被 `-D warnings` 判成 dead_code）。
#[cfg(feature = "ui-mcp-http")]
struct Scratch {
    dir: std::path::PathBuf,
}

#[cfg(feature = "ui-mcp-http")]
impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-d-ui-1-{tag}-{}-{}",
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

// 本仓刻意没有 `* text=auto`（见根 `.gitattributes`：全仓归一的爆炸半径远超收益），
// 而 Windows runner 的 `core.autocrlf=true` 会把源码检出成 CRLF ⇒ 本文件那些**跨行**的
// `contains("…\n…")` 在 Windows 上永不匹配（实测红在 run 41370538851 的 windows 腿）。
// 判的是"文本内容"而非"逐字节"，所以读盘后归一行尾才是正解（`.gitattributes` 头部也这么分）。
fn crate_source(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join(name),
    )
    .expect("读源码")
    .replace("\r\n", "\n")
}

// ===========================================================================
// 判据 ①：serve 侧给出端口（源码腿 —— 默认档就能跑）
// ===========================================================================
#[test]
fn the_serve_path_hands_the_write_surface_an_undo_port() {
    let serve = crate_source("ui_mcp_serve.rs");
    // rebase 到 `ADR-0057` A 之后的形状：那道**闸**（`--ui-test-mode` +
    // `YEBAN_ALLOW_UI_INJECT=1`）住在 `run`，`serve_wiring` 是纯装配。所以这里判的是
    // "闸出来的那一份端口确实被喂进选项"，⛔ 不再是"serve_wiring 自己造会话"。
    assert!(
        serve.contains("serve_wiring(&loaded, permission, test_undo)"),
        "`run` 必须把测试面开出的端口 `test_undo` 交给 `serve_wiring` —— 喂 `None` \
         就是 `D-UI-1`：九个改工程的 `host::wire_*` 整段不接，界面变成一块画得出点不动的面板"
    );
    assert!(
        serve.contains("undo: test_undo") || serve.contains("undo,\n        proposal_source"),
        "`serve_wiring` 的 `undo` 字段必须是**传进来的那一份**，不许在本文件里写死"
    );
    assert!(
        !serve.contains("undo: None"),
        "`ui_mcp_serve.rs` 里不许再出现 `undo: None`（那正是缺陷本身）"
    );
    // 单一配方：⛔ 不在 serve 侧第二次发明"怎么从归档开会话"。
    assert!(
        serve.contains("open_undo_session"),
        "serve 侧的会话必须由 `undo::open_undo_session` 开出（与 `run_gui` 同一条入口）"
    );
    assert!(
        !serve.contains("session_from_archive"),
        "会话配方只有一份实现；serve 侧不许绕过 `open_undo_session` 直接调低层配方"
    );
    assert!(
        serve.contains("replace_assets"),
        "端口必须带上归档的资产池（`#132` 的边界：不带池 ⇒ 打开带资产的工程再保存会把池写空）"
    );
    assert!(
        crate_source("main.rs").contains("yeban_app::undo::open_undo_session"),
        "生产 GUI 与 UI 控制面必须调**同一个**会话入口（否则两条装配出的会话留下两种来源写法）"
    );
    assert!(
        crate_source("undo.rs").contains("session_from_archive(source, &loaded.archive, now_ms)"),
        "`open_undo_session` 必须是 `session_from_archive` 的薄包装（一份配方、一个报文口）"
    );

    // 保存腿：落盘的字节取自端口，而不是装配时刻的投影缓存。
    let surface = crate_source("live_surface.rs");
    assert!(
        surface.contains("port.try_project()"),
        "`save_now` 必须从端口取当前工程（写 `self.project` 那份缓存 = 悄悄丢掉编辑）"
    );
    assert!(
        surface.contains("\"writtenFromUndoPort\""),
        "落盘来源必须是**可断言的读数**（回执字段 `writtenFromUndoPort`），不是注释里的声明"
    );
}

// ===========================================================================
// 判据 ②：会话配方保真 —— `history.dag` 读得回来，坏数据不许降级
// ===========================================================================
#[test]
fn session_from_archive_restores_the_history_and_refuses_to_guess_an_empty_one() {
    // 夹具选 `filled_project`：判据 ② 要真提交一步编辑，`wiring_fixture` 只在**有 MIDI 音符**
    // 的工程上给得出夹具（`demo_project` 留给判据 ③，那里要的是"有主总线 ⇒ 能建轨"）。
    let project = filled_project();

    // ---- (1) 空 `history.dag` ⇒ 新会话（一条根提交，没有可撤销的） ----
    let fresh = ProjectArchive {
        project: project.clone(),
        history_dag: Vec::new(),
        assets: Vec::new(),
        embedded_presets: Vec::new(),
    };
    let port = UndoPort::new(
        session_from_archive("<判据: 空 dag>", &fresh, NOW).expect("空条目 = 新会话"),
    );
    assert_eq!(port.display().undoable, 0, "新会话起点不该有可撤销的编辑");

    // ---- (2) 编辑过的那一版归档 ⇒ 重开后撤销链还活着 ----
    let fixture = yeban_app::undo::undo_session::wiring_fixture(&port.project()).expect("夹具");
    port.commit_ops(NOW + 1, "判据: 一步编辑", vec![fixture.op()])
        .expect("提交");
    let edited_dag = yeban_model::commit::encode_history_dag(&port.graph());
    assert!(
        !edited_dag.is_empty(),
        "编辑之后图谱编码必须非空（否则下面的对照没有意义）"
    );
    let reopened = UndoPort::new(
        session_from_archive(
            "<判据: 带图谱重开>",
            &ProjectArchive {
                project: port.project(),
                history_dag: edited_dag.clone(),
                assets: Vec::new(),
                embedded_presets: Vec::new(),
            },
            NOW + 2,
        )
        .expect("带图谱重开会话"),
    );
    assert_eq!(
        reopened.display().undoable,
        1,
        "重开之后必须还能撤销那一步（游标由文档与图谱推导）"
    );
    assert!(
        reopened.perform(yeban_app::undo::UiAction::Undo).changed(),
        "重开的会话撤销必须真的生效"
    );

    // ---- (3) 非法 `history.dag` ⇒ 如实报错，⛔ 不降级成空历史 ----
    let broken = ProjectArchive {
        project: project.clone(),
        history_dag: b"{ this is not a CommitGraph".to_vec(),
        assets: Vec::new(),
        embedded_presets: Vec::new(),
    };
    let error = session_from_archive("<判据: 坏 dag>", &broken, NOW)
        .expect_err("非法图谱必须报错，而不是假装开了一条空历史");
    assert!(
        error.contains("无法恢复"),
        "报错必须点名是哪一份 `history.dag` 坏了，实得: {error}"
    );

    // ---- (4) **零提交**的图谱（条目有字节、`commits` 为空）⇒ 与空条目同一句读法 ----
    // `decode_history_dag` 的口径：空字节 ⇒ `None`，零提交图谱 ⇒ 也是 `None`（"还没有历史"，
    // 不是"历史坏了"）。这一条把修复与那条口径对齐：⛔ 不许把一份合法的起始容器读成错误。
    let zero_dag = serde_json::to_vec(&CommitGraph::new()).expect("零提交图谱 JSON");
    let zero = session_from_archive(
        "<判据: 零提交>",
        &ProjectArchive {
            project,
            history_dag: zero_dag,
            assets: Vec::new(),
            embedded_presets: Vec::new(),
        },
        NOW,
    )
    .expect("零提交图谱必须读成新会话，而不是报错");
    assert_eq!(zero.project().validate(), Ok(()), "新会话的文档必须自洽");
    assert_eq!(
        UndoPort::new(zero).display().undoable,
        0,
        "新会话起点没有可撤销的编辑"
    );
}

// ===========================================================================
// 判据 ③（端到端，只在 `ui-mcp-http` 里存在）：注入的点击真的改得动工程并落盘
// ===========================================================================
#[cfg(feature = "ui-mcp-http")]
#[test]
fn an_injected_click_on_the_serve_surface_moves_the_project_and_lands_on_disk() {
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
        aether_app.contains("> track-volumes:"),
        "新 UI 契约面必须声明 > track-volumes:"
    );
    assert!(
        aether_host.contains("set_track_volumes"),
        "宿主接线层必须注册 set_track_volumes"
    );
}

/// 一次**真实**点击：读几何 → 按下 → 移到同一点 → 松手（三步都走 §12.4 的注入方法）。
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
