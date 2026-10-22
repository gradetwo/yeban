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
    let mut fixture = Fixture::new(LaunchQuantization::Bar);
    assert_eq!(fixture.engine.scene_launches_applied(), Some(0));
    fixture.publish_readings();
    let before = fixture.query_engine();
    assert_eq!(before["status"], "success", "{before}");
    assert_eq!(
        before["data"]["engine"]["sceneLaunchesApplied"], 0,
        "注入后的起点必须是 0: {before}"
    );
    assert_eq!(
        before["data"]["engine"]["sceneLaunchesAppliedSource"],
        "hostEngineMirror"
    );

    let response = fixture.launch();
    assert_eq!(response["status"], "success", "{response}");
    assert_eq!(response["data"]["queued"], true, "{response}");
    assert_eq!(response["data"]["applied"], false, "{response}");
    assert_eq!(
        response["data"]["sceneId"],
        fixture.scene_id.to_canonical_string()
    );
    assert_eq!(response["data"]["sceneName"], fixture.scene_name);
    assert_eq!(response["data"]["tempoBpm"], fixture.scene_bpm);
    assert_eq!(response["data"]["tempoSource"], "sceneOverride");
    assert_eq!(response["data"]["quantization"], "Bar");
    assert_eq!(
        fixture.engine.scene_launches_applied(),
        Some(0),
        "⛔ 排队 ≠ 生效"
    );
    assert_eq!(fixture.engine.has_pending_scene_launch(), Some(false));

    let drain = fixture.drain();
    assert_eq!(
        drain,
        SceneLaunchDrain {
            delivered: 1,
            refused: 0
        },
        "宿主必须真的把请求交给引擎"
    );
    assert_eq!(fixture.engine.has_pending_scene_launch(), Some(true));

    let (used, bpm) = fixture.pump_until_applied();
    println!(
        "[scene-launch] Bar: queued=0->1, scene_launches_applied=0->{}, used_blocks={used}, bpm={}->{bpm}, pending=false",
        fixture.engine.scene_launches_applied().unwrap_or(u64::MAX),
        fixture.base_bpm
    );
    assert_eq!(used, 750, "Bar ⇒ 第 750 块（一小节 @ 120bpm/48k/4-4）");
    assert!(
        (bpm - fixture.scene_bpm).abs() < 1e-9,
        "tempo 必须变成场景覆盖 {bpm}"
    );
    assert_eq!(fixture.engine.scene_launches_applied(), Some(1));
    assert_eq!(fixture.engine.has_pending_scene_launch(), Some(false));

    fixture.publish_readings();
    let after = fixture.query_engine();
    assert_eq!(after["status"], "success", "{after}");
    assert_eq!(
        after["data"]["engine"]["sceneLaunchesApplied"], 1,
        "⭐ 0 → 1 必须越过控制面读得到: {after}"
    );
    assert_eq!(
        after["data"]["engine"]["sceneLaunchesAppliedSource"],
        "hostEngineMirror"
    );
}

#[test]
fn the_four_quantisation_tiers_land_on_the_documented_blocks() {
    for (quantization, expected) in [
        (LaunchQuantization::Off, 1_usize),
        (LaunchQuantization::Bar, 750),
        (LaunchQuantization::TwoBars, 1_500),
        (LaunchQuantization::FourBars, 3_000),
    ] {
        let mut fixture = Fixture::new(quantization);
        let response = fixture.launch();
        assert_eq!(response["status"], "success", "{response}");
        let drain = fixture.drain();
        assert_eq!(drain.delivered, 1, "{quantization:?}: {drain:?}");
        let (used, bpm) = fixture.pump_until_applied();
        println!(
            "[scene-launch] {quantization:?}: used_blocks={used}, bpm={}->{bpm}, scene_launches_applied={}",
            fixture.base_bpm,
            fixture.engine.scene_launches_applied().unwrap_or(u64::MAX)
        );
        assert_eq!(
            used, expected,
            "⭐ {quantization:?} ⇒ 生效块必须恰好是 {expected}（实测 {used}）"
        );
        assert!(
            (bpm - fixture.scene_bpm).abs() < 1e-9,
            "{quantization:?}: tempo 必须是场景覆盖"
        );
        assert_eq!(fixture.engine.scene_launches_applied(), Some(1));
    }
}

#[test]
fn without_a_tool_call_nothing_launches() {
    let mut fixture = Fixture::new(LaunchQuantization::Off);
    assert_eq!(fixture.drain(), SceneLaunchDrain::default());
    let (used, bpm) = fixture.pump_until_applied();
    assert_eq!(used, QUANTA_BUDGET, "没有请求 ⇒ 预算耗尽也不生效");
    assert_eq!(fixture.engine.scene_launches_applied(), Some(0));
    assert!((bpm - fixture.base_bpm).abs() < 1e-9, "tempo 不得变化");
}

#[test]
fn dry_run_queues_nothing_and_an_unknown_scene_is_loud() {
    let mut fixture = Fixture::new(LaunchQuantization::Bar);
    let dry = fixture.call(
        "yeban_launch_scene",
        json!({"sceneId": fixture.scene_id.to_canonical_string(), "dryRun": true}),
    );
    assert_eq!(dry["status"], "success", "{dry}");
    assert_eq!(dry["data"]["dryRun"], true, "{dry}");
    assert_eq!(dry["data"]["preview"]["queued"], false, "{dry}");
    assert_eq!(fixture.drain(), SceneLaunchDrain::default());

    let ghost = "01J8ZQ00000000000000000000";
    let unknown = fixture.call("yeban_launch_scene", json!({"sceneId": ghost}));
    assert_eq!(unknown["status"], "error", "{unknown}");
    assert_eq!(unknown["error"]["code"], "ENTITY_NOT_FOUND", "{unknown}");
    assert_eq!(
        unknown["error"]["data"]["reason"], "sceneNotFound",
        "{unknown}"
    );
    assert_eq!(
        unknown["error"]["data"]["sceneCount"],
        fixture.project.scenes.len()
    );
    assert_eq!(fixture.drain(), SceneLaunchDrain::default());
    assert_eq!(fixture.engine.scene_launches_applied(), Some(0));
}

#[test]
fn without_the_host_attached_the_tool_is_a_loud_engine_unavailable_error() {
    let project = yeban_model::samples::filled_project();
    let scene = *project.scenes.keys().next().expect("场景");
    let mount = InProcessMcp::start_for_project(
        true,
        project,
        SessionSource::InMemory(PathBuf::from("sample:mcp-scene-launch-no-host")),
    )
    .expect("挂载决策")
    .expect("挂载");
    let bearer = format!("Bearer {}", mount.token().expose());
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "yeban_launch_scene", "arguments": {"sceneId": scene.to_canonical_string()}}
    })
    .to_string();
    let (status, value) = post_json(mount.address(), &bearer, &body);
    assert_eq!(status, 200, "{value}");
    let response = &value["result"];
    assert_eq!(response["status"], "error", "{response}");
    assert_eq!(response["error"]["code"], "CONFLICT", "{response}");
    assert_eq!(response["error"]["data"]["reason"], "engineUnavailable");
    assert_eq!(response["error"]["data"]["hostAttached"], false);
    assert!(
        response["error"]["message"]
            .as_str()
            .expect("message")
            .contains("本形态没有引擎"),
        "报文必须逐字说明没有引擎: {response}"
    );
}
