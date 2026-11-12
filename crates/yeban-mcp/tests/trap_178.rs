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
//! | [`the_warned_track_is_named_in_empty_sources_in_render_master`] | 同一工程的两处读数 | 警告轨不在 `sources[]`，且其 id == `emptySources.trackIds` 唯一一条 |
//! | [`dry_run_preview_carries_the_same_warning`] | `dryRun:true` 的 `preview.warnings` | 与真实响应同一份 |
//! | [`a_device_on_master_never_warns`] | 装到 `Master` 的 `warnings[]` | 0 条（主总线永不报） |
//! | [`a_device_on_a_bus_fed_by_sounding_source_has_no_warning`] | 装到接有声源的总线轨 | 0 条（入边上游有声） |
//! | [`a_device_on_a_transitive_bus_has_no_warning`] | 装到二级总线轨（总线→总线） | 0 条（传递闭包） |
//! | [`a_device_on_a_bus_fed_only_by_silent_tracks_still_warns`] | 装到只接静音轨的总线轨 | 1 条警告（上游全静音；与静音轨成环也不打转） |
//! | [`a_device_on_master_never_warns_even_when_everything_is_silent`] | 全静音工程里装到 `Master` | 0 条（主总线豁免无条件）；同工程里装到静音源轨仍 1 条 |
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
            offset_samples: None,
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
                offset_samples: None,
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

/// 总线夹具：`Track 1`（有声）→ `Bus` → `Bus 2` → `Master`；`Track 2`（静音）
/// → `Quiet Bus` → `Master`，且 `Quiet Bus` ↔ `Track 2` 成环。
/// 顶层身份复用 `fixture(false)` 的 ULID，使 `insert_device_on` 可直接用。
struct BusFixture {
    project: YebanProjectV1,
    master: EntityId,
    bus: EntityId,
    bus2: EntityId,
    quiet_bus: EntityId,
}

fn bus_fixture() -> BusFixture {
    let base = fixture(false);
    let master = base.project.master_bus_track_id;
    let bus = ulid("01ARZ3NDEKTSV4RRFFQ69G5F04");
    let bus2 = ulid("01ARZ3NDEKTSV4RRFFQ69G5F05");
    let quiet_bus = ulid("01ARZ3NDEKTSV4RRFFQ69G5F06");
    let mut project = base.project;
    project.tracks.insert(
        bus,
        TrackV3 {
            id: bus,
            name: "Bus".to_owned(),
            kind: TrackKind::AuxReturn,
            ..TrackV3::default()
        },
    );
    project.tracks.insert(
        bus2,
        TrackV3 {
            id: bus2,
            name: "Bus 2".to_owned(),
            kind: TrackKind::AuxReturn,
            ..TrackV3::default()
        },
    );
    project.tracks.insert(
        quiet_bus,
        TrackV3 {
            id: quiet_bus,
            name: "Quiet Bus".to_owned(),
            kind: TrackKind::AuxReturn,
            ..TrackV3::default()
        },
    );
    let edge = |id: &str, source: EntityId, destination: EntityId| {
        (
            ulid(id),
            RoutingEdge {
                id: ulid(id),
                source_node: source,
                destination_node: destination,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        )
    };
    project.routing_graph.nodes.extend([bus, bus2, quiet_bus]);
    project.routing_graph.edges.extend([
        edge("01ARZ3NDEKTSV4RRFFQ69G5F42", base.lead, bus),
        edge("01ARZ3NDEKTSV4RRFFQ69G5F43", bus, bus2),
        edge("01ARZ3NDEKTSV4RRFFQ69G5F44", bus2, master),
        edge("01ARZ3NDEKTSV4RRFFQ69G5F45", base.empty, quiet_bus),
        // 成环：Quiet Bus → Track 2。上游遍历必须带已访集，否则在这里打转。
        edge("01ARZ3NDEKTSV4RRFFQ69G5F46", quiet_bus, base.empty),
        edge("01ARZ3NDEKTSV4RRFFQ69G5F47", quiet_bus, master),
    ]);
    project
        .validate()
        .unwrap_or_else(|error| panic!("总线夹具工程必须合法: {error}"));
    BusFixture {
        project,
        master,
        bus,
        bus2,
        quiet_bus,
    }
}

/// 全静音夹具：`Silent Track` 只有一个 0 音符片段，`Track 2` 没有摆放，
/// 两轨都接 `Master`。用于钉"主总线豁免无条件"：工程里没有任何可发声内容时，
/// 装到 `Master` 仍不报，而装到静音源轨仍报。
struct SilentFixture {
    project: YebanProjectV1,
    master: EntityId,
    silent_lead: EntityId,
    silent_clip: EntityId,
    empty: EntityId,
}

fn silent_fixture() -> SilentFixture {
    let master = ulid("01ARZ3NDEKTSV4RRFFQ69G5F61");
    let silent_lead = ulid("01ARZ3NDEKTSV4RRFFQ69G5F62");
    let empty = ulid("01ARZ3NDEKTSV4RRFFQ69G5F63");
    let silent_clip = ulid("01ARZ3NDEKTSV4RRFFQ69G5F70");
    let placement = ulid("01ARZ3NDEKTSV4RRFFQ69G5F80");
    let lead_edge = ulid("01ARZ3NDEKTSV4RRFFQ69G5F90");
    let empty_edge = ulid("01ARZ3NDEKTSV4RRFFQ69G5F91");

    let mut project = YebanProjectV1 {
        id: ulid("01ARZ3NDEKTSV4RRFFQ69G5F99"),
        title: "trap-178-silent".to_owned(),
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
        silent_lead,
        TrackV3 {
            id: silent_lead,
            name: "Silent Track".to_owned(),
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
    project.clip_pool.insert(
        silent_clip,
        ClipPoolEntry {
            id: silent_clip,
            name: "Empty Clip".to_owned(),
            content: ClipContent::Midi {
                notes: BTreeMap::new(),
            },
        },
    );
    project
        .tracks
        .get_mut(&silent_lead)
        .expect("silent lead")
        .clips
        .insert(
            placement,
            ClipPlacement {
                id: placement,
                clip_id: silent_clip,
                start_tick: 0,
                duration_ticks: 960,
                loop_config: LoopConfig::default(),
                muted: false,
                offset_samples: None,
            },
        );
    project.routing_graph = RoutingGraph {
        nodes: vec![silent_lead, empty, master],
        edges: BTreeMap::from([
            (
                lead_edge,
                RoutingEdge {
                    id: lead_edge,
                    source_node: silent_lead,
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
        .unwrap_or_else(|error| panic!("全静音夹具工程必须合法: {error}"));
    SilentFixture {
        project,
        master,
        silent_lead,
        silent_clip,
        empty,
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
    insert_device_on_with_top(
        dispatcher,
        auth,
        fixture.lead,
        fixture.clip,
        target,
        dry_run,
    )
}

/// 与 `insert_device_on` 相同，但顶层 `trackId` / `clipId` 由调用方指定
///（供自带夹具的测试用）。
fn insert_device_on_with_top(
    dispatcher: &mut Dispatcher,
    auth: &str,
    top_track: EntityId,
    top_clip: EntityId,
    target: EntityId,
    dry_run: bool,
) -> Value {
    call(
        dispatcher,
        auth,
        "yeban_edit_notes",
        json!({
            "trackId": top_track.to_canonical_string(),
            "clipId": top_clip.to_canonical_string(),
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
/// 收据里被点名（两处读数对照）。
///
/// ⭐ 2026-11-12 雨窗空轨包合入后契约迁移：空轨**不进** `sources[]`（不再有
/// `kind:"silent"` 的源条目），改在 `emptySources.trackIds` 里按 id 点名。
/// 本判据的对照关系不变 —— 警告点名的轨必须在渲染收据里以同一身份出现，
/// ⛔ 不许静默消失；出现的位置按新契约改查 `emptySources`。
#[test]
fn the_warned_track_is_named_in_empty_sources_in_render_master() {
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
    assert!(
        sources
            .iter()
            .find(|source| source["track"] == warned_track)
            .is_none(),
        "空轨不得再出现在 sources[]: {sources:?}"
    );
    let empty_sources = &rendered["data"]["emptySources"];
    assert_eq!(
        empty_sources["count"], 1,
        "被点名的空轨恰一条: {empty_sources}"
    );
    let track_ids = empty_sources["trackIds"]
        .as_array()
        .expect("emptySources.trackIds 必须是数组");
    assert_eq!(
        track_ids.len(),
        1,
        "count 与 trackIds.len() 必须一致: {empty_sources}"
    );
    assert_eq!(
        track_ids[0],
        json!(fixture.empty.to_canonical_string()),
        "被点名的必须是装设备时警告的那条轨: {empty_sources}"
    );

    // 对照：有声的那条轨不是 silent。
    let sounding = sources
        .iter()
        .find(|source| source["track"] == "Track 1")
        .unwrap_or_else(|| panic!("sources[] 里必须有 Track 1: {sources:?}"));
    assert_eq!(sounding["kind"], "engine-instrument", "{sounding}");
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

/// 总线 / 主总线豁免判据：`Master` 本轨 0 片段是结构使然，且离线渲染里
/// 它的设备链跑在"求和之后 → 插入" —— 旧规则对主总线必报是误报。
#[test]
fn a_device_on_master_never_warns() {
    let fixture = bus_fixture();
    let (mut dispatcher, auth) = dispatcher(&fixture.project);
    let written = insert_device_on(&mut dispatcher, &auth, fixture.master, false);
    assert_eq!(written["status"], "success", "{written}");
    assert_eq!(
        written["data"]["warnings"],
        json!([]),
        "主总线永不报 deviceOnSilentTrack: {written}"
    );
}

/// 总线轨经入边连到有声源 ⇒ 汇总音频会过它的设备链 ⇒ 不报。
#[test]
fn a_device_on_a_bus_fed_by_sounding_source_has_no_warning() {
    let fixture = bus_fixture();
    let (mut dispatcher, auth) = dispatcher(&fixture.project);
    let written = insert_device_on(&mut dispatcher, &auth, fixture.bus, false);
    assert_eq!(written["status"], "success", "{written}");
    assert_eq!(
        written["data"]["warnings"],
        json!([]),
        "接有声源的总线轨不该有警告: {written}"
    );
}

/// 传递闭包：`Bus 2` 的上游是 `Bus`，`Bus` 的上游才是有声的 `Track 1`。
#[test]
fn a_device_on_a_transitive_bus_has_no_warning() {
    let fixture = bus_fixture();
    let (mut dispatcher, auth) = dispatcher(&fixture.project);
    let written = insert_device_on(&mut dispatcher, &auth, fixture.bus2, false);
    assert_eq!(written["status"], "success", "{written}");
    assert_eq!(
        written["data"]["warnings"],
        json!([]),
        "二级总线轨（传递闭包连到有声源）不该有警告: {written}"
    );
}

/// 反向钉子：只接静音轨的总线轨仍报 —— 豁免不是"总线轨一律不报"。
/// 夹具里 `Quiet Bus` ↔ `Track 2` 成环：遍历必须终止且仍判静音。
#[test]
fn a_device_on_a_bus_fed_only_by_silent_tracks_still_warns() {
    let fixture = bus_fixture();
    let (mut dispatcher, auth) = dispatcher(&fixture.project);
    let written = insert_device_on(&mut dispatcher, &auth, fixture.quiet_bus, false);
    assert_eq!(written["status"], "success", "{written}");
    let warnings = written["data"]["warnings"]
        .as_array()
        .expect("warnings 必须是数组");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0]["code"], "deviceOnSilentTrack", "{warnings:?}");
    assert_eq!(warnings[0]["track"], "Quiet Bus", "{warnings:?}");
}

/// 主总线豁免无条件：全静音工程里装到 `Master` 仍不报；同工程里装到
/// 静音源轨仍报 1 条（对照：豁免只覆盖主总线，不覆盖源轨）。
#[test]
fn a_device_on_master_never_warns_even_when_everything_is_silent() {
    let fixture = silent_fixture();
    let (mut dispatcher, auth) = dispatcher(&fixture.project);
    let on_master = insert_device_on_with_top(
        &mut dispatcher,
        &auth,
        fixture.silent_lead,
        fixture.silent_clip,
        fixture.master,
        false,
    );
    assert_eq!(on_master["status"], "success", "{on_master}");
    assert_eq!(
        on_master["data"]["warnings"],
        json!([]),
        "全静音工程里主总线仍不报: {on_master}"
    );

    let on_silent = insert_device_on_with_top(
        &mut dispatcher,
        &auth,
        fixture.silent_lead,
        fixture.silent_clip,
        fixture.empty,
        false,
    );
    assert_eq!(on_silent["status"], "success", "{on_silent}");
    let warnings = on_silent["data"]["warnings"]
        .as_array()
        .expect("warnings 必须是数组");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0]["code"], "deviceOnSilentTrack", "{warnings:?}");
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
