//! 任务单 `#259`：**实时侧数出它没落地的自动化泳道**（先只做 `SendGain`）。
//!
//! ## 本文件钉住的事实
//!
//! `SendGain` 泳道在界面里画得出来（`track-{i}-automation-send-{edge}-lane`）、MCP 也
//! 写得进去（`yeban_edit_automation` 的词表含 `SendGain`），但**实时引擎没有发送量槽位**
//! （`yeban_engine::param` 只映射 0..=3，其余 `Unmapped`）⇒ 监听时它**一条都不执行**
//! （`#258` 实测：开头比导出响 ≈45 dB），而离线导出 / MCP 渲染**会**执行它。
//!
//! 本票把"沉默"变成"读数"：`EngineHost::publish_automation` 每跳数一次
//! [`yeban_app::engine_host::unapplied_automation_lanes`]，`--headless` 打印
//! `automation-engine:` 行。⛔ 本文件**不**改音频：SendGain 泳道**不得**产生任何
//! `SetParam` 事件，既有 `automation_published` / `automation_dropped` 的增量与
//! 没有这条泳道时**逐条相同**。
//!
//! ## 判据（与任务单 §2 的七条对应）
//!
//! | 判据 | 测试 |
//! | :--- | :--- |
//! | 一条 `SendGain` ⇒ 1；两条 ⇒ 2；无 ⇒ 0 | [`one_send_gain_lane_is_counted_but_publishes_nothing`] / [`two_send_gain_lanes_count_two`] / [`volume_and_pan_lanes_are_not_counted`] |
//! | `read_enabled:false` ⇒ 0 | [`a_disabled_send_gain_lane_is_not_counted`] |
//! | `TrackVolume` / `TrackPan` 不计 | [`volume_and_pan_lanes_are_not_counted`] |
//! | 读数可见（CLI 逐字） | [`headless_cli_names_the_unapplied_send_gain_lane`] |
//! | `DeviceParam` / `Macro` 同类但分开数 | [`an_enabled_device_param_lane_is_counted_separately`] |
//!
//! 音频不变的正对照在 `tests/send_gain_automation_export.rs`（同一个 `--export-wav`
//! 渲染核心，修前/修后的文件 `sha256` 相同；读数由那条判据的 `println!` 给出）。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use yeban_app::bridge::{demo_id, demo_project};
use yeban_app::cli;
use yeban_app::engine_host::{EngineHost, automation_unapplied_line, unapplied_automation_lanes};
use yeban_model::container::write_project_container;
use yeban_model::{
    AutomationLane, AutomationPoint, AutomationTarget, AutomationWriteMode, CurveType, EntityId,
    YebanProjectV1,
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

/// 往一条轨上挂一条 `TrackVolume` 泳道（一个点）。用于"声相正对照"的夹具：
/// `publish_automation` 的既有结构是**先找到音量泳道**才进该轨的声相分支，而演示
/// 工程的声相泳道挂在一条没有音量泳道的轨上 ⇒ 不补这一条，"声相真的落地了"就没有正对照。
fn insert_volume_lane(project: &mut YebanProjectV1, track_id: EntityId) {
    let target = AutomationTarget::TrackVolume { track_id };
    let point = AutomationPoint {
        id: EntityId::new(),
        tick: 0,
        value: -3.0,
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
                read_enabled: true,
                write_mode: AutomationWriteMode::Off,
                domain: None,
            },
        );
}

/// 取一行里 `key=value` 的 `value`（到下一个空白为止；与 CLI 判据同一口径）。
fn field(line: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(rest[..end].to_owned())
}

/// 判据 1 + 3（负对照）：**一条开着的 `SendGain` 泳道 ⇒ 计数 1**，且它**不**产生事件
/// （`automation_published` / `automation_dropped` 的增量与没有这条泳道时逐条相同）。
#[test]
fn one_send_gain_lane_is_counted_but_publishes_nothing() {
    let mut project = demo_project();
    let track = first_track(&project);
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

/// 判据 1：**两条** `SendGain` 泳道 ⇒ 计数 2（数的是泳道，不是采样点/事件）。
#[test]
fn two_send_gain_lanes_count_two() {
    let mut project = demo_project();
    let track = first_track(&project);
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
    insert_send_lane(&mut project, track, demo_id("R1"), false);
    assert_eq!(unapplied_automation_lanes(&project), Default::default());

    let mut host = EngineHost::new();
    host.publish_automation(Some(&project));
    assert_eq!(host.automation_unapplied(), 0, "关掉的泳道不产生缺口读数");
}

/// 判据 3（正对照）：`TrackVolume` / `TrackPan` **不计** —— 它们已经真的落到引擎里了。
///
/// 同时证明"读数不是恒 0"：同一份工程里那两条泳道**真的**下发了事件。
/// ⚠ 演示工程的声相泳道在一条没有音量泳道的轨上，而 `publish_automation` 的既有结构
/// 是"先有音量泳道才走声相分支" ⇒ 这里给那条轨补一条音量泳道，让**声相**也真的落地。
/// ⛔ 这是夹具补全，不是改实现（本票不改音频路径）。
#[test]
fn volume_and_pan_lanes_are_not_counted() {
    let mut project = demo_project();
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
    insert_volume_lane(&mut project, pan_track);

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
        landed, 3,
        "夹具前提：三条开着的「已落地」泳道（音量 ×2 + 声相 ×1）"
    );
    assert!(
        unapplied_automation_lanes(&project).is_empty(),
        "TrackVolume / TrackPan 不得被算成缺口"
    );

    let mut host = EngineHost::new();
    host.reload(&project, 0).expect("重建");
    assert!(
        host.publish_automation(Some(&project)) >= 4,
        "三条落地泳道 = 音量 2 条 + 声相 2 条事件（读数不是恒 0 的假探针）"
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
        line.contains("只在离线导出/MCP 渲染里生效"),
        "读数必须说清「只在导出里生效」: {line}"
    );
}

/// 诊断行的**形状**单独钉住：没有泳道时也照样打印（0 是读数，不是"没这一行"）。
#[test]
fn the_diagnostic_line_is_present_even_when_nothing_is_unapplied() {
    let line = automation_unapplied_line(Default::default());
    assert_eq!(
        line,
        "automation-engine: unapplied-lanes=0 send-gain=0 device-param=0 macro=0 \
         note=\"有 0 条自动化泳道在监听里没落地；SendGain 只在离线导出/MCP 渲染里生效，\
         DeviceParam/Macro 没有任何生效路径\""
    );
}
