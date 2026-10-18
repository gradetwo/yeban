//! **#178 的判据**：`#174` 验收抓到的坑⑦ —— 设备装到一条"没有任何片段"的
//! 音轨上时，响应此前**一个字节都不说**，于是外部 agent 按 `tracks` 顺序取第一条
//! （实测：`trackCount:2`、只有一条有摆放）就白跑一轮。
//!
//! 判据全部走真实 `tools/call` 管线。夹具是 **2 条 MIDI 源轨 → Master**：
//! `Track 1` 有一个带音符的片段，`Track 2` 没有摆放（`#174` 实测的形态）。
//!
//! | 判据 | 对象 | 读数 |
//! | :--- | :--- | :--- |
//! | [`a_device_on_an_empty_track_warns_and_does_not_block`] | 装到 `Track 2` 的 `warnings[]` | 1 条 `deviceOnSilentTrack` ＋ 提案/提交计数各 +1 |
//! | [`a_device_on_a_track_with_notes_has_no_warning`] | 装到 `Track 1` 的 `warnings[]` | 0 条 |
//! | [`an_empty_note_clip_is_silent_too_with_its_own_reading`] | 有摆放但片段里 0 音符 | 1 条警告 ＋ `placements:1` / `notes:0` |
//! | [`the_warned_track_is_the_silent_source_in_render_master`] | 同一工程的两处读数 | `warnings[0].track == sources[].track` 且 `kind:"silent"` |
//! | [`dry_run_preview_carries_the_same_warning`] | `dryRun:true` 的 `preview.warnings` | 与真实响应同一份 |
//!
//! 复跑读数：`cargo test -p yeban-mcp --test trap_178 -- --nocapture`

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingGraph, RoutingKind, SampleRate, TrackKind, TrackV3, YebanProjectV1,
};

/// 外部 agent 实测的那句提示（`#174` 坑⑦ 的逐字说法）。
const EMPTY_TRACK_HINT: &str =
    "该轨没有任何片段 ⇒ 这台设备对该轨的输出不会产生影响；若你是想给别的轨装、请核对 `trackId`";

/// 有摆放、但摆放里没有会发声的内容时的逐字提示。
const SILENT_PLACEMENTS_HINT: &str = "该轨的摆放里没有任何可发声内容（片段里没有会触发的音符 / 摆放全被静音）⇒ 这台设备对该轨的输出不会产生影响；若你是想给别的轨装、请核对 `trackId`";

fn ulid(text: &str) -> EntityId {
    EntityId::from_str(text).expect("夹具 ULID 必须合法")
}

/// 2 条 MIDI 源轨 → Master 的夹具。`Track 1` 有一个带 1 个音符的片段；
/// `Track 2` 依 `empty_clip` 选择"没有任何摆放"或"有一个 0 音符的片段"。
struct Fixture {
    project: YebanProjectV1,
    lead: EntityId,
    empty: EntityId,
    clip: EntityId,
    empty_clip: EntityId,
}

fn fixture(empty_clip: bool) -> Fixture {
    let master = ulid("01ARZ3NDEKTSV4RRFFQ69G5F01");
    let lead = ulid("01ARZ3NDEKTSV4RRFFQ69G5F02");
    let empty = ulid("01ARZ3NDEKTSV4RRFFQ69G5F03");
    let clip = ulid("01ARZ3NDEKTSV4RRFFQ69G5F10");
    let placement = ulid("01ARZ3NDEKTSV4RRFFQ69G5F20");
    let note = ulid("01ARZ3NDEKTSV4RRFFQ69G5F30");
    let empty_clip_id = ulid("01ARZ3NDEKTSV4RRFFQ69G5F11");
    let empty_placement = ulid("01ARZ3NDEKTSV4RRFFQ69G5F21");
    let lead_edge = ulid("01ARZ3NDEKTSV4RRFFQ69G5F40");
    let empty_edge = ulid("01ARZ3NDEKTSV4RRFFQ69G5F41");

    let mut project = YebanProjectV1 {
        id: ulid("01ARZ3NDEKTSV4RRFFQ69G5F99"),
        title: "trap-178".to_owned(),
        bpm: 120.0,
        master_bus_track_id: master,
        ..YebanProjectV1::default()
    };
    project.audio_config.sample_rate = SampleRate::Hz48000;
    project.tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    project.tracks.insert(
        lead,
        TrackV3 {
            id: lead,
            name: "Track 1".to_owned(),
            kind: TrackKind::Midi,
            ..TrackV3::default()
        },
    );
    project.tracks.insert(
        empty,
        TrackV3 {
            id: empty,
            name: "Track 2".to_owned(),
            kind: TrackKind::Midi,
            ..TrackV3::default()
        },
    );

    let mut notes = BTreeMap::new();
    notes.insert(note, MidiNote::new(note, 0, 60, 960));
    project.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "Clip".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );
    project.tracks.get_mut(&lead).expect("lead").clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 960,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );

    if empty_clip {
        project.clip_pool.insert(
            empty_clip_id,
            ClipPoolEntry {
                id: empty_clip_id,
                name: "Empty Clip".to_owned(),
                content: ClipContent::Midi {
                    notes: BTreeMap::new(),
                },
            },
        );
        project.tracks.get_mut(&empty).expect("empty").clips.insert(
            empty_placement,
            ClipPlacement {
                id: empty_placement,
                clip_id: empty_clip_id,
                start_tick: 0,
                duration_ticks: 960,
                loop_config: LoopConfig::default(),
                muted: false,
            },
        );
    }

    project.routing_graph = RoutingGraph {
        nodes: vec![lead, empty, master],
        edges: BTreeMap::from([
            (
                lead_edge,
                RoutingEdge {
                    id: lead_edge,
                    source_node: lead,
                    destination_node: master,
                    kind: RoutingKind::TrackToBus,
                    gain_db: None,
                },
            ),
            (
                empty_edge,
                RoutingEdge {
                    id: empty_edge,
                    source_node: empty,
                    destination_node: master,
                    kind: RoutingKind::TrackToBus,
                    gain_db: None,
                },
            ),
        ]),
    };
    project
        .validate()
        .unwrap_or_else(|error| panic!("夹具工程必须合法: {error}"));

    Fixture {
        project,
        lead,
        empty,
        clip,
        empty_clip: empty_clip_id,
    }
}

fn dispatcher(project: &YebanProjectV1) -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            PathBuf::from("/tmp/yeban-trap-178.yeban"),
            project.clone(),
            false,
        )
        .expect("注入夹具工程");
    (dispatcher, auth)
}

fn call(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "trap-178",
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    let response = outcome.response.expect("必须有响应");
    assert!(
        response.error.is_none(),
        "{name} 不该有 JSON-RPC 层错误: {:?}",
        response.error
    );
    response.result.expect("必须有 result")
}

fn effect_device(device_id: EntityId) -> Value {
    json!({
        "deviceId": device_id.to_canonical_string(),
        "name": "Probe EQ",
        "kind": "InternalEffect",
        "bypassed": false,
        "latencySamples": 0,
        "params": [{"name": "eq_low_gain", "value": 3.0, "unit": "dB"}],
    })
}

/// 一次 `insertDevice` 调用：顶层两个实参指向**有声**的 `Track 1`（证明警告取决于
/// 操作对象自带的 `trackId`，不是顶层信封）。
fn insert_device_on(
    dispatcher: &mut Dispatcher,
    auth: &str,
    target: EntityId,
    dry_run: bool,
) -> Value {
    let fixture = fixture(false);
    call(
        dispatcher,
        auth,
        "yeban_edit_notes",
        json!({
            "trackId": fixture.lead.to_canonical_string(),
            "clipId": fixture.clip.to_canonical_string(),
            "dryRun": dry_run,
            "ops": [{
                "kind": "insertDevice",
                "trackId": target.to_canonical_string(),
                "device": effect_device(ulid("01ARZ3NDEKTSV4RRFFQ69G5F50")),
            }],
        }),
    )
}

/// 判据 ①＋④：装到 0 摆放的 `Track 2` ⇒ **1 条**警告（逐字 `code` / `hint` / 读数），
/// 且**不阻断**：`proposalCount` 与 `commitCount` 各 +1，合并后设备真的在链上。
#[test]
fn a_device_on_an_empty_track_warns_and_does_not_block() {
    let fixture = fixture(false);
    let (mut dispatcher, auth) = dispatcher(&fixture.project);
    let proposals_before = dispatcher.domain().proposal_count();
    let commits_before = dispatcher.domain().commit_count();

    let written = insert_device_on(&mut dispatcher, &auth, fixture.empty, false);
    assert_eq!(written["status"], "success", "{written}");

    let warnings = written["data"]["warnings"]
        .as_array()
        .expect("warnings 必须是数组");
    assert_eq!(
        warnings.len(),
        1,
        "装到 0 摆放的音轨上必须正好一条警告: {warnings:?}"
    );
    let warning = &warnings[0];
    assert_eq!(warning["code"], "deviceOnSilentTrack", "{warning}");
    assert_eq!(
        warning["trackId"],
        fixture.empty.to_canonical_string(),
        "{warning}"
    );
    assert_eq!(warning["track"], "Track 2", "{warning}");
    assert_eq!(warning["placements"], 0, "{warning}");
    assert_eq!(warning["soundingPlacements"], 0, "{warning}");
    assert_eq!(warning["notes"], 0, "{warning}");
    assert_eq!(warning["hint"], EMPTY_TRACK_HINT, "{warning}");

    // ④ 警告不阻断：两次计数各 +1（提案分支上真的写了一条提交）。
    assert_eq!(
        dispatcher.domain().proposal_count(),
        proposals_before + 1,
        "提案计数必须照常 +1"
    );
    assert_eq!(
        dispatcher.domain().commit_count(),
        commits_before + 1,
        "提案提交必须照常 +1"
    );

    // 装是合法的，不是"被吞掉"：合并后读侧真的报出这台设备。
    let proposal_id = written["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("提案 id")
        .to_owned();
    let merged = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal_id, "commitMessage": "#178 静音轨装设备"}),
    );
    assert_eq!(merged["status"], "success", "{merged}");
    let reading = call(
        &mut dispatcher,
        &auth,
        "yeban_query_engine_state",
        json!({"trackId": fixture.empty.to_canonical_string()}),
    );
    assert_eq!(reading["status"], "success", "{reading}");
    assert_eq!(reading["data"]["track"]["deviceCount"], 1, "{reading}");
    assert_eq!(
        reading["data"]["track"]["devices"][0]["deviceId"],
        ulid("01ARZ3NDEKTSV4RRFFQ69G5F50").to_canonical_string(),
        "设备必须真的落在那条轨上: {reading}"
    );
}

/// 判据 ②：装到**有声**的 `Track 1` ⇒ `warnings[]` 为空。
#[test]
fn a_device_on_a_track_with_notes_has_no_warning() {
    let fixture = fixture(false);
    let (mut dispatcher, auth) = dispatcher(&fixture.project);
    let written = insert_device_on(&mut dispatcher, &auth, fixture.lead, false);
    assert_eq!(written["status"], "success", "{written}");
    assert_eq!(
        written["data"]["warnings"],
        json!([]),
        "有声轨不该有警告: {written}"
    );
}

/// 判据 ③：**有 clip 但片段里 0 音符**也算静音 ⇒ 警告带自己的读数
/// （`placements:1` / `soundingPlacements:0` / `notes:0`）与**不撒谎的** hint。
///
/// 裁决理由：渲染器的 `sources[].kind` 只看"排程到的音符 + 真的渲染的音频片段"
/// （`render.rs` 的 `TrackSource::kind`），不看摆放行的多少。若写入侧只在
/// `clips.is_empty()` 时警告，同一份工程就会出现"装设备时不说、渲染时才说"的
/// 第二条裂缝 —— 而本票要补的正是"装设备时看不到"。
#[test]
fn an_empty_note_clip_is_silent_too_with_its_own_reading() {
    let fixture = fixture(true);
    let (mut dispatcher, auth) = dispatcher(&fixture.project);
    let written = insert_device_on(&mut dispatcher, &auth, fixture.empty, false);
    assert_eq!(written["status"], "success", "{written}");

    let warnings = written["data"]["warnings"]
        .as_array()
        .expect("warnings 必须是数组");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let warning = &warnings[0];
    assert_eq!(warning["code"], "deviceOnSilentTrack", "{warning}");
    assert_eq!(warning["placements"], 1, "{warning}");
    assert_eq!(warning["soundingPlacements"], 0, "{warning}");
    assert_eq!(warning["notes"], 0, "{warning}");
    assert_eq!(
        warning["hint"], SILENT_PLACEMENTS_HINT,
        "有摆放时不能再说『该轨没有任何片段』: {warning}"
    );

    // 阴性对照：同一批里给这条轨补一个音符 ⇒ 模拟后的工程有东西可发声 ⇒ 不警告。
    let filled = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": fixture.empty.to_canonical_string(),
            "clipId": fixture.empty_clip.to_canonical_string(),
            "ops": [
                {
                    "kind": "insertDevice",
                    "trackId": fixture.empty.to_canonical_string(),
                    "device": effect_device(ulid("01ARZ3NDEKTSV4RRFFQ69G5F51")),
                },
                {
                    "kind": "add",
                    "note": {
                        "id": ulid("01ARZ3NDEKTSV4RRFFQ69G5F31").to_canonical_string(),
                        "startTick": 0,
                        "pitch": 64,
                        "durationTicks": 240,
                    },
                },
            ],
        }),
    );
    assert_eq!(filled["status"], "success", "{filled}");
    assert_eq!(
        filled["data"]["warnings"],
        json!([]),
        "同批补上音符后，设备就有东西可处理 ⇒ 不误报: {filled}"
    );
}

/// 判据 ⑤：同一工程里，装设备时警告的那条轨在 `yeban_render_master` 的
/// `sources[]` 里就是 `kind:"silent"`（两处读数对照）。
#[test]
fn the_warned_track_is_the_silent_source_in_render_master() {
    let scratch = Scratch::new("trap-178-render");
    let fixture = fixture(false);
    let (mut dispatcher, auth) = dispatcher(&fixture.project);

    let written = insert_device_on(&mut dispatcher, &auth, fixture.empty, false);
    assert_eq!(written["status"], "success", "{written}");
    let warned_track = written["data"]["warnings"][0]["track"]
        .as_str()
        .expect("警告必须带 track 名")
        .to_owned();
    assert_eq!(warned_track, "Track 2", "{written}");
    let proposal_id = written["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("提案 id")
        .to_owned();
    let merged = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal_id, "commitMessage": "#178 render 对照"}),
    );
    assert_eq!(merged["status"], "success", "{merged}");

    let rendered = call(
        &mut dispatcher,
        &auth,
        "yeban_render_master",
        json!({
            "format": "wav",
            "sampleRate": 48_000,
            "path": scratch.join("master.wav").display().to_string(),
        }),
    );
    assert_eq!(rendered["status"], "success", "{rendered}");
    let sources = rendered["data"]["sources"]
        .as_array()
        .expect("sources 必须是数组");
    let warned = sources
        .iter()
        .find(|source| source["track"] == warned_track)
        .unwrap_or_else(|| panic!("sources[] 里必须有 {warned_track}: {sources:?}"));
    assert_eq!(
        warned["kind"], "silent",
        "装设备时警告的那条轨在渲染侧必须也是 silent: {warned}"
    );
    assert_eq!(warned["audible"], true, "{warned}");
    assert_eq!(warned["notes"], 0, "{warned}");
    assert_eq!(warned["audioClips"], 0, "{warned}");

    // 对照：有声的那条轨不是 silent。
    let sounding = sources
        .iter()
        .find(|source| source["track"] == "Track 1")
        .unwrap_or_else(|| panic!("sources[] 里必须有 Track 1: {sources:?}"));
    assert_eq!(sounding["kind"], "midi-synth-osc", "{sounding}");
    assert_eq!(sounding["notes"], 1, "{sounding}");
}

/// `dryRun:true` 的 `preview.warnings` 与真正创建提案时的 `warnings[]` 是**同一份**
/// （规划期只算一次，见 `Plan::Propose { warnings }`），且 dryRun 不创建提案。
#[test]
fn dry_run_preview_carries_the_same_warning() {
    let fixture = fixture(false);
    let (mut dispatcher, auth) = dispatcher(&fixture.project);
    let proposals_before = dispatcher.domain().proposal_count();

    let planned = insert_device_on(&mut dispatcher, &auth, fixture.empty, true);
    assert_eq!(planned["status"], "success", "{planned}");
    assert_eq!(
        dispatcher.domain().proposal_count(),
        proposals_before,
        "dryRun 不得创建提案"
    );
    let preview_warnings = planned["data"]["preview"]["warnings"]
        .as_array()
        .expect("preview.warnings 必须是数组");
    assert_eq!(preview_warnings.len(), 1, "{preview_warnings:?}");
    assert_eq!(preview_warnings[0]["code"], "deviceOnSilentTrack");
    assert_eq!(preview_warnings[0]["hint"], EMPTY_TRACK_HINT);

    let written = insert_device_on(&mut dispatcher, &auth, fixture.empty, false);
    assert_eq!(written["status"], "success", "{written}");
    assert_eq!(
        planned["data"]["preview"]["warnings"], written["data"]["warnings"],
        "dryRun 与真实响应的 warnings 必须逐字相同"
    );
}

/// 测试用临时目录（Drop 即清理；不落在工作树里）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |delta| delta.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-{tag}-{}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
