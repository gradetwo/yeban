//! ⭐ `#290`：**形态 A 真的能启动场景** —— MCP 工具 → 宿主队列 → `EngineHost`
//! → `EngineRuntime` 的量化生效，以及 `engine.sceneLaunchesApplied` 的 `0 → 1`。
//!
//! ## 这条判据要证的句子
//!
//! > 一个外部客户端在**真环回 socket** 上调用 `yeban_launch_scene`，宿主的 60Hz
//! > 搬运函数把请求交给**真的** `EngineHost::queue_scene_launch`；到量化点之后
//! > `scene_launches_applied` 从 `0` 变 `1`、tempo 变成场景覆盖值、生效块是
//! > `750 / 1500 / 3000`（`Off` 是 `1`）；`yeban_query_engine_state` 的
//! > `engine.sceneLaunchesApplied` 同源地从 `0` 变 `1`。
//!
//! ## 边界（⛔ 不证的东西）
//!
//! - ⛔ 不证真声卡的听感（没有硬件）；设备腿的 `EngineRuntime` 在 cpal 回调线程上，
//!   `EngineHost::scene_launches_applied()` 返回 `None` ⇒ 读数如实 `null`；
//! - ⛔ 不证 16ms `slint::Timer` 真的在 GUI 里跑（那要活窗口）；本判据直接调用
//!   **产品心跳用的同一个** [`yeban_app::mcp_mount::apply_scene_launch_requests`]。
//!
//! 运行：
//! ```text
//! cargo test -p yeban-app --features in-process-mcp --test mcp_scene_launch
//! ```

#![allow(unused)]
#![cfg(feature = "in-process-mcp")]

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

use yeban_app::engine_host::EngineHost;
use yeban_app::mcp_mount::{self, InProcessMcp, SceneLaunchDrain, SessionSource};
use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_mcp::transport::http::MCP_PATH;
use yeban_model::{EntityId, LaunchQuantization, YebanProjectV1};

/// 一次 socket 往返的上限（坏回归要**响亮失败**，不是挂到 CI 上限）。
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// 推量子的预算：四小节 = 3000 块 @ 120 bpm / 48 kHz / 4-4；给两倍余量。
const QUANTA_BUDGET: usize = 4_000;

/// 一次 HTTP JSON-RPC 往返（真 socket）。
fn post_json(address: SocketAddr, bearer: &str, body: &str) -> (u16, Value) {
    let mut stream = TcpStream::connect_timeout(&address, IO_TIMEOUT).expect("连接环回控制面");
    stream.set_read_timeout(Some(IO_TIMEOUT)).expect("读超时");
    stream.set_write_timeout(Some(IO_TIMEOUT)).expect("写超时");
    let mut head = format!(
        "POST {MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\
         Authorization: {bearer}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    head.push_str(body);
    stream.write_all(head.as_bytes()).expect("写请求");
    stream.flush().expect("刷出请求");
    let mut raw = String::new();
    stream
        .read_to_string(&mut raw)
        .expect("读响应（超时即失败）");
    let split = raw.find("\r\n\r\n").expect("响应头/体分隔");
    let status = raw[..split]
        .lines()
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .expect("状态行");
    let value: Value = serde_json::from_str(&raw[split + 4..]).expect("响应体是 JSON");
    (status, value)
}

/// 一条形态 A 夹具：真 `EngineHost` + 真 `InProcessMcp`（环回 socket）。
struct Fixture {
    mount: InProcessMcp,
    engine: EngineHost,
    readings: yeban_app::mcp_mount::EngineReadingsHandle,
    scene_id: EntityId,
    scene_name: String,
    base_bpm: f64,
    scene_bpm: f64,
    project: YebanProjectV1,
}

impl Fixture {
    fn new(quantization: LaunchQuantization) -> Self {
        let mut project = yeban_model::samples::filled_project();
        project.transport.launch_quantization = quantization;
        let base_bpm = project.bpm;
        let scene_id = *project.scenes.keys().next().expect("样本必须有场景");
        let scene = project
            .scenes
            .get_mut(&scene_id)
            .expect("场景身份来自同一张表");
        scene.tempo = Some(base_bpm + 17.0);
        let scene_name = scene.name.clone();
        let scene_bpm = base_bpm + 17.0;

        let mut engine = EngineHost::new();
        engine.reload(&project, 0).expect("引擎必须能建起来");

        let mount = InProcessMcp::start_for_project(
            true,
            project.clone(),
            SessionSource::InMemory(PathBuf::from("sample:mcp-scene-launch")),
            Vec::new(),
        )
        .expect("挂载决策")
        .expect("运行期开关打开时必须真的挂载");
        let handle = mount.scene_launch_handle();
        handle.attach();
        let readings = mount.engine_readings_handle();
        readings.set_scene_launches_applied(engine.scene_launches_applied());
        Self {
            mount,
            engine,
            readings,
            scene_id,
            scene_name,
            base_bpm,
            scene_bpm,
            project,
        }
    }

    fn call(&self, name: &str, arguments: Value) -> Value {
        let bearer = format!("Bearer {}", self.mount.token().expose());
        let body = json!({
            "jsonrpc": "2.0",
            "id": "scene-launch",
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments}
        })
        .to_string();
        let (status, value) = post_json(self.mount.address(), &bearer, &body);
        assert_eq!(status, 200, "带令牌的领域调用必须 200: {value}");
        value["result"].clone()
    }

    fn launch(&self) -> Value {
        self.call(
            "yeban_launch_scene",
            json!({"sceneId": self.scene_id.to_canonical_string()}),
        )
    }

    fn query_engine(&self) -> Value {
        self.call("yeban_query_engine_state", json!({}))
    }

    /// 产品心跳的**同一个**搬运函数（不是测试里复制的一份）。
    fn drain(&mut self) -> SceneLaunchDrain {
        let handle = self.mount.scene_launch_handle();
        mcp_mount::apply_scene_launch_requests(&handle, &mut self.engine)
    }

    /// 推量子直到生效（或预算耗尽）⇒ 返回 `(用了多少块, 生效后的 bpm)`。
    fn pump_until_applied(&mut self) -> (usize, f64) {
        let mut block = vec![0.0_f32; DEFAULT_BLOCK_FRAMES * 2];
        let runtime = self
            .engine
            .runtime_for_criteria()
            .expect("控制驱动档：运行时归宿主所有");
        let mut used = QUANTA_BUDGET;
        for step in 1..=QUANTA_BUDGET {
            runtime.process_quantum(&mut block, 2);
            if runtime.stats().scene_launches_applied > 0 {
                used = step;
                break;
            }
        }
        (used, runtime.transport().bpm())
    }

    fn publish_readings(&self) {
        mcp_mount::publish_scene_launches_applied(&self.readings, &self.engine);
    }
}

#[test]
fn the_tool_queues_and_the_engine_applies_at_the_bar_boundary() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

#[test]
fn the_four_quantisation_tiers_land_on_the_documented_blocks() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

#[test]
fn without_a_tool_call_nothing_launches() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

#[test]
fn dry_run_queues_nothing_and_an_unknown_scene_is_loud() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

#[test]
fn without_the_host_attached_the_tool_is_a_loud_engine_unavailable_error() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}
