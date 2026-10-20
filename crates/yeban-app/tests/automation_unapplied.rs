//! 任务单 `#259`：**实时侧数出它没落地的自动化泳道**（先只做 `SendGain`）。
//!
//! ## 本文件钉住的事实
//!
//! `SendGain` 泳道在界面里画得出来（`track-{i}-automation-send-{edge}-lane`）、MCP 也
//! 写得进去（`yeban_edit_automation` 的词表含 `SendGain`）。`#259` 当时**实时引擎没有
//! 发送量槽位**（`yeban_engine::param` 只映射 0..=3，其余 `Unmapped`）⇒ 监听时它
//! **一条都不执行**（`#258` 实测：开头比导出响 ≈45 dB），而离线导出 / MCP 渲染**会**执行它。
//!
//! 本票把"沉默"变成"读数"：`EngineHost::publish_automation` 每跳数一次
//! [`yeban_app::engine_host::unapplied_automation_lanes`]，`--headless` 打印
//! `automation-engine:` 行。
//!
//! ## ⚠ ⭐ `#261` 对上面这条事实的修订（本文件已按新事实改写）
//!
//! 实时引擎**有了逐边发送量槽位**（`SEND_GAIN_SLOT = 4`，容量 `SEND_SLOTS = 64`）⇒
//! **边在实时侧有落点**的 `SendGain` 泳道**真的会下发** `SetParam { entity: 边, slot: 4 }`，
//! 而 `send-gain` 这一列只剩"**边没有实时落点**"的泳道：
//!
//! * 落点的判据是 `yeban_engine::routing::edge_carries_audio`（引擎编译路由计划用的
//!   **同一份**过滤条件）—— 侧链边、源是母线、源不是可渲染音轨的边都没有落点；
//! * 判据夹具因此用**侧链边**（工程合法：`SendGain` 的目标校验只要求边存在）来表达
//!   "没有落点"，而用普通 `TrackToBus` 边表达"有落点、会下发"。
//!
//! ## 判据（与任务单 §2 的七条对应；`#261` 之后的两列）
//!
//! | 判据 | 测试 |
//! | :--- | :--- |
//! | 无落点的一条 `SendGain` ⇒ 1；两条 ⇒ 2；无 ⇒ 0 | [`a_send_gain_lane_without_a_landing_is_counted_but_publishes_nothing`] / [`two_send_gain_lanes_count_two`] / [`volume_and_pan_lanes_are_not_counted`] |
//! | ⭐ **有落点的一条 ⇒ 0（它落地了）且**真的**下发一条 `(边, 4, db_to_gain)`** | [`a_send_gain_lane_on_an_audio_edge_lands`] |
//! | `read_enabled:false` ⇒ 0 | [`a_disabled_send_gain_lane_is_not_counted`] |
//! | `TrackVolume` / `TrackPan` 不计 | [`volume_and_pan_lanes_are_not_counted`] |
//! | 读数可见（CLI 逐字） | [`headless_cli_names_the_unapplied_send_gain_lane`] |
//! | `DeviceParam` / `Macro` 同类但分开数 | [`an_enabled_device_param_lane_is_counted_separately`] |
//!
//! 音频**导出**不变的正对照在 `tests/send_gain_automation_export.rs`（同一个
//! `--export-wav` 渲染核心，`#261` 不动离线路径 ⇒ 文件 `sha256` 相同）。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use yeban_app::bridge::{demo_id, demo_project};
use yeban_app::cli;
use yeban_app::engine_host::{EngineHost, automation_unapplied_line, unapplied_automation_lanes};
use yeban_model::container::write_project_container;
use yeban_model::{
    AutomationLane, AutomationPoint, AutomationTarget, AutomationWriteMode, CurveType, EntityId,
    RoutingKind, YebanProjectV1,
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 临时目录（`Drop` 删掉自己；工作树里不留临时文件）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-259-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        Self { dir }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 演示工程里身份序第一条非主总线轨（`#259` 的夹具只用它挂泳道）。
fn first_track(project: &YebanProjectV1) -> EntityId {
    *project
        .tracks
        .keys()
        .find(|id| **id != project.master_bus_track_id)
        .expect("演示工程必须有非主总线轨")
}

/// 往一条轨上挂一条 `SendGain` 泳道（一个点；`read_enabled` 由调用方给）。
fn insert_send_lane(
    project: &mut YebanProjectV1,
    track_id: EntityId,
    edge_id: EntityId,
    read_enabled: bool,
) {
    let target = AutomationTarget::SendGain { track_id, edge_id };
    let point = AutomationPoint {
        id: EntityId::new(),
        tick: 0,
        value: -6.0,
        curve: CurveType::Linear,
    };
    project
        .tracks
        .get_mut(&track_id)
        .expect("目标轨必须在")
        .automation_lanes
        .insert(
            target,
            AutomationLane {
                target,
                points: BTreeMap::from([(point.id, point)]),
                read_enabled,
                write_mode: AutomationWriteMode::Off,
                domain: None,
            },
        );
}

/// 把一条边的 kind 改成 `Sidechain`（⭐ `#261` 的"**没有实时落点**"夹具）。
///
/// 侧链边**不搬音频** ⇒ 引擎的 `RoutingPlan` 不收它（`edge_carries_audio` 的第一条）
/// ⇒ 实时侧没有它的发送量槽位。工程仍然**合法**：`SendGain::validate_against`
/// 只要求"边存在"，不要求"边搬音频"。
///
/// ⚠ 改 kind 会顺带改变该轨的路由（R1 是 `TrackToBus`）⇒ 它退回"无音频出边 ⇒
/// 默认直连 master"，与本文件关心的读数无关（本文件不判音频）。
fn make_sidechain(project: &mut YebanProjectV1, edge: EntityId) {
    let edge = project
        .routing_graph
        .edges
        .get_mut(&edge)
        .expect("夹具引用的边必须存在");
    edge.kind = RoutingKind::Sidechain;
}

/// 取一行里 `key=value` 的 `value`（到下一个空白为止；与 CLI 判据同一口径）。
fn field(line: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(rest[..end].to_owned())
}

/// 判据 1 + 3（负对照）：一条开着的 `SendGain` 泳道，**边没有实时落点**（侧链）
/// ⇒ 计数 1，且它**不**产生事件。
#[test]
fn a_send_gain_lane_without_a_landing_is_counted_but_publishes_nothing() {
    let mut project = demo_project();
    let track = first_track(&project);
    make_sidechain(&mut project, demo_id("R1"));
    let mut host = EngineHost::new();
    host.reload(&project, 0).expect("重建");
    let baseline_written = host.publish_automation(Some(&project));
    assert!(
        baseline_written >= 1,
        "演示工程开着的音量/声相泳道必须真的下发（实得 {baseline_written}）"
    );
    assert_eq!(
        host.automation_unapplied(),
        0,
        "演示工程没有开着的「未落地」泳道（它的 DeviceParam 泳道是 read_enabled:false）"
    );
    let baseline_counts = host.automation_counts();

    insert_send_lane(&mut project, track, demo_id("R1"), true);
    let written = host.publish_automation(Some(&project));
    assert_eq!(
        written, baseline_written,
        "SendGain 泳道不得产生任何 SetParam 事件（否则就动了音频）"
    );
    assert_eq!(
        host.automation_unapplied(),
        1,
        "一条开着的 SendGain 泳道 ⇒ 计数 1"
    );
    let breakdown = host.automation_unapplied_breakdown();
    assert_eq!(breakdown.send_gain, 1);
    assert_eq!(breakdown.device_param, 0);
    assert_eq!(breakdown.macro_lanes, 0);
    assert_eq!(
        host.automation_counts(),
        (
            baseline_counts.0 + u64::try_from(written).expect("条数"),
            baseline_counts.1
        ),
        "SendGain 泳道不得被记成 published / dropped"
    );
}

/// ⭐ `#261` 判据（正对照）：边在实时侧**有落点**（普通 `TrackToBus` 边）⇒
/// ① `send-gain` **不**再数它（它落地了）；② 真的多下发**恰好一条**事件；
/// ③ 关掉读开关（`read_enabled:false`）又回到"一条都不发"（裁决 4）。
#[test]
fn a_send_gain_lane_on_an_audio_edge_lands() {
    let mut project = demo_project();
    let track = first_track(&project);
    let edge = demo_id("R1");
    let mut host = EngineHost::new();
    host.reload(&project, 0).expect("重建");
    let baseline_written = host.publish_automation(Some(&project));
    assert!(
        baseline_written >= 1,
        "演示工程开着的音量/声相泳道必须真的下发（实得 {baseline_written}）"
    );
    assert_eq!(host.automation_unapplied(), 0);

    insert_send_lane(&mut project, track, edge, true);
    let written = host.publish_automation(Some(&project));
    assert_eq!(
        written,
        baseline_written + 1,
        "有落点的 SendGain ⇒ 恰好多一条事件（baseline {baseline_written}）"
    );
    assert_eq!(
        host.automation_unapplied(),
        0,
        "它落地了 ⇒ 不是「未落地」缺口"
    );
    assert_eq!(host.automation_unapplied_breakdown().send_gain, 0);

    // 裁决 4：`read_enabled:false` ⇒ 等于不发事件（读数也回到 0）。
    let mut disabled = project.clone();
    let lane = disabled
        .tracks
        .get_mut(&track)
        .expect("目标轨必须在")
        .automation_lanes
        .values_mut()
        .find(|lane| matches!(lane.target, AutomationTarget::SendGain { .. }))
        .expect("刚插进去的泳道必须在");
    lane.read_enabled = false;
    assert_eq!(
        unapplied_automation_lanes(&disabled).send_gain,
        0,
        "关掉的泳道不是缺口"
    );
    assert_eq!(
        host.publish_automation(Some(&disabled)),
        baseline_written,
        "关掉的 SendGain 不多发任何事件"
    );
}

/// 判据 1：**两条**没有落点的 `SendGain` 泳道 ⇒ 计数 2（数的是泳道，不是采样点/事件）。
#[test]
fn two_send_gain_lanes_count_two() {
    let mut project = demo_project();
    let track = first_track(&project);
    make_sidechain(&mut project, demo_id("R1"));
    make_sidechain(&mut project, demo_id("R2"));
    insert_send_lane(&mut project, track, demo_id("R1"), true);
    insert_send_lane(&mut project, track, demo_id("R2"), true);

    assert_eq!(unapplied_automation_lanes(&project).send_gain, 2);
    let mut host = EngineHost::new();
    host.publish_automation(Some(&project));
    assert_eq!(host.automation_unapplied(), 2);
}

/// 判据 2：`read_enabled:false` ⇒ 计数 **0**（关掉的泳道本来就不该生效）。
#[test]
fn a_disabled_send_gain_lane_is_not_counted() {
    let mut project = demo_project();
    let track = first_track(&project);
    // 边**有落点**（`TrackToBus`）⇒ 这条判据只让"关掉"这一个原因成立。
    insert_send_lane(&mut project, track, demo_id("R1"), false);
    assert_eq!(unapplied_automation_lanes(&project), Default::default());

    let mut host = EngineHost::new();
    host.publish_automation(Some(&project));
    assert_eq!(host.automation_unapplied(), 0, "关掉的泳道不产生缺口读数");
}

/// 判据 3（正对照）：`TrackVolume` / `TrackPan` **不计** —— 它们已经真的落到引擎里了。
///
/// 同时证明"读数不是恒 0"：同一份工程里那两条泳道**真的**下发了事件。
///
/// ⚠ ⭐ **任务单 `#260` 撤回了 `#259` 的绕法**：本判据原先把演示工程的贝斯轨
/// （只有 `TrackPan`、没有 `TrackVolume`）补了一条音量泳道，理由是"`publish_automation`
/// 的既有结构是先有音量泳道才走声相分支"。那句话**正是** `#260` 修掉的缺陷 ——
/// 声相段已改为独立采样，⛔ 不再需要给夹具补泳道。现在**不给贝斯轨加任何东西**：
/// 它的声相泳道必须自己发 2 条事件（下面断言 `== 3`：鼓的音量 1 ＋ 贝斯声相 2）。
#[test]
fn volume_and_pan_lanes_are_not_counted() {
    let project = demo_project();
    let pan_track = project
        .tracks
        .values()
        .find(|track| {
            track.automation_lanes.values().any(|lane| {
                lane.read_enabled && matches!(lane.target, AutomationTarget::TrackPan { .. })
            })
        })
        .map(|track| track.id)
        .expect("演示工程必须有一条开着的 TrackPan 泳道");
    // ⭐ `#260`：夹具前提 —— 那条轨**确实没有**音量泳道（这正是缺陷的复现条件）。
    assert!(
        !project
            .tracks
            .get(&pan_track)
            .expect("目标轨必须在")
            .automation_lanes
            .keys()
            .any(|target| matches!(target, AutomationTarget::TrackVolume { .. })),
        "夹具前提：声相泳道必须挂在一条**没有音量泳道**的轨上（#259 曾在这里补泳道绕过缺陷）"
    );

    let landed = project
        .tracks
        .values()
        .flat_map(|track| track.automation_lanes.values())
        .filter(|lane| {
            lane.read_enabled
                && matches!(
                    lane.target,
                    AutomationTarget::TrackVolume { .. } | AutomationTarget::TrackPan { .. }
                )
        })
        .count();
    assert_eq!(
        landed, 2,
        "夹具前提：两条开着的「已落地」泳道（音量 ×1 + 声相 ×1；#260 之后不再补第三条）"
    );
    assert!(
        unapplied_automation_lanes(&project).is_empty(),
        "TrackVolume / TrackPan 不得被算成缺口"
    );

    let mut host = EngineHost::new();
    host.reload(&project, 0).expect("重建");
    assert_eq!(
        host.publish_automation(Some(&project)),
        3,
        "两条落地泳道 = 音量 1 条 + 声相 2 条事件（读数不是恒 0 的假探针）"
    );
    assert_eq!(host.automation_unapplied(), 0);
}

/// `DeviceParam` 与 `SendGain` **同属**"实时引擎不执行"，但**不同类**：
/// 前者连离线渲染也没有落点 ⇒ 读数里**分开列**（⛔ 不混进 `send-gain`）。
#[test]
fn an_enabled_device_param_lane_is_counted_separately() {
    let mut project = demo_project();
    let mut enabled = 0_usize;
    for track in project.tracks.values_mut() {
        for lane in track.automation_lanes.values_mut() {
            if matches!(lane.target, AutomationTarget::DeviceParam { .. }) {
                lane.read_enabled = true;
                enabled += 1;
            }
        }
    }
    assert_eq!(enabled, 1, "演示工程前提：恰好一条 DeviceParam 泳道");

    let lanes = unapplied_automation_lanes(&project);
    assert_eq!(lanes.device_param, 1);
    assert_eq!(lanes.send_gain, 0, "DeviceParam 不得混进 send-gain");
    assert_eq!(lanes.macro_lanes, 0);
    assert_eq!(lanes.total(), 1);
}

/// 判据 4：**CLI 逐字**点名（`--open <带 SendGain 泳道的容器> --headless`）。
#[test]
fn headless_cli_names_the_unapplied_send_gain_lane() {
    let scratch = Scratch::new("cli");
    let mut project = demo_project();
    let track = first_track(&project);
    make_sidechain(&mut project, demo_id("R1"));
    insert_send_lane(&mut project, track, demo_id("R1"), true);
    project.validate().expect("夹具工程必须合法");

    let archive = write_project_container(&project, b"", &BTreeMap::new()).expect("写出容器");
    let path = scratch.dir.join("send-gain.yeban");
    std::fs::write(&path, &archive).expect("写容器文件");

    let options = cli::parse(&[
        "--open".to_owned(),
        path.display().to_string(),
        "--headless".to_owned(),
    ])
    .expect("解析");
    let lines = cli::run_batch(&options).expect("无头批处理必须成功");
    let line = lines
        .iter()
        .find(|line| line.starts_with("automation-engine:"))
        .unwrap_or_else(|| panic!("缺 automation-engine: 行:\n{}", lines.join("\n")));
    println!("### {line}");

    assert_eq!(
        field(line, "unapplied-lanes").as_deref(),
        Some("1"),
        "{line}"
    );
    assert_eq!(field(line, "send-gain").as_deref(), Some("1"), "{line}");
    assert_eq!(field(line, "device-param").as_deref(), Some("0"), "{line}");
    assert_eq!(field(line, "macro").as_deref(), Some("0"), "{line}");
    assert!(
        line.contains("监听里没落地"),
        "读数必须说清「没落地」而不是只给一个数: {line}"
    );
    assert!(
        line.contains("SendGain 的边在监听里没有落点"),
        "读数必须说清「这条边的发送量在监听里没有落点」: {line}"
    );
}

/// 诊断行的**形状**单独钉住：没有泳道时也照样打印（0 是读数，不是"没这一行"）。
#[test]
fn the_diagnostic_line_is_present_even_when_nothing_is_unapplied() {
    let line = automation_unapplied_line(Default::default());
    assert_eq!(
        line,
        "automation-engine: unapplied-lanes=0 send-gain=0 device-param=0 macro=0 \
         note=\"有 0 条自动化泳道在监听里没落地；SendGain 的边在监听里没有落点\
         （离线导出/MCP 按边求值），DeviceParam/Macro 没有任何生效路径\""
    );
}
