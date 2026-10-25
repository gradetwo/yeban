//! **`#179` 的判据**：`#177` 交底的那条**行为差集** —— 六个**属性级**形态
//! （`setClipName` / `setDeviceName` / `setDeviceBypassed` /
//! `setClipPlacementMuted` / `setTrackColor` / `setClipPlacementDuration`）
//! 改的是名字 / 旁通 / 静音 / 颜色 / 时长，**一个音符都不读**，此前却落在
//! `NoteOp::is_note_level()` 的漏网一侧（反选清单没排除它们）。
//!
//! `#177` 之后片段池查找只在批里真有形态读顶层 `clipId` 时才发生，因此那六个形态
//! **不再撞**"目标池条目必须是 MIDI 片段"的断言；本票把谓词也收窄到**恰好**
//! 音符形态，并给出"它们真的能作用在非 MIDI 片段上"的证据。
//!
//! ⚠ 本线新增 `setNoteTiming`（写音符的起点/时值绝对值）⇒ 音符形态由四个变**五个**；
//! 它读顶层 `clipId` 也读目标音符，因此仍在"音频片段必须响亮失败"与
//! `NOTE_LEVEL_KINDS` 的覆盖里。四个旧形态的基线 digest 常量**逐字未动**。
//!
//! | 判据 | 对象 | 读数 |
//! | :--- | :--- | :--- |
//! | [`six_property_forms_succeed_on_an_audio_clip`] | 六个属性级形态 + **音频**顶层 `clipId` | 6 条 `status:"success"` ＋ 6 个 `projectDigest` 变化 ＋ 6 条标题（`音轨级编辑: 1 步`） |
//! | [`four_note_forms_are_still_blocked_on_an_audio_clip`] | 五个音符形态 + **音频** `clipId` | 5 条 `CLIP_NOT_FOUND`（逐字含"不是 MIDI 片段"） |
//! | [`the_four_note_forms_are_byte_identical_on_a_midi_clip`] | 四个**基线**音符形态 + **MIDI** 片段 | 4 个合并后 `projectDigest`（收窄前录下、收窄后逐字相等） |
//! | [`the_note_level_predicate_is_exactly_six`] | `NOTE_LEVEL_KINDS` / `ENVELOPE_CLIP_KINDS` | 6 个成员 ＋ 子集关系（唯一非音符的 `clipId` 读者 = `removeClip`） |
//!
//! 复跑读数：`cargo test -p yeban-mcp --test trap_179 -- --nocapture`

use std::path::PathBuf;
use std::str::FromStr as _;

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::notes::{ENVELOPE_CLIP_KINDS, NOTE_LEVEL_KINDS, OP_KINDS};
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::{ClipContent, EntityId, TrackKind, YebanProjectV1};

/// 注入规范样本工程的分发器（与 `tests/trap_177.rs` 同一形状）。
fn dispatcher() -> (Dispatcher, String, YebanProjectV1) {
    let token = BearerToken::generate().token;
    let authorization = format!("Bearer {}", token.expose());
    let project = yeban_model::samples::filled_project();
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            PathBuf::from("/tmp/yeban-trap-179.yeban"),
            project.clone(),
            false,
        )
        .expect("注入规范工程");
    (dispatcher, authorization, project)
}

/// 走 `tools/call` 并返回带内 `ToolResponse`（成功或领域失败）。
fn call(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "trap-179",
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

/// 样本里**唯一**的非 MIDI（音频）池条目身份。
fn audio_clip(project: &YebanProjectV1) -> EntityId {
    project
        .clip_pool
        .values()
        .find(|entry| matches!(&entry.content, ClipContent::Audio { .. }))
        .expect("样本里必须有音频片段")
        .id
}

/// 样本里的音频轨身份（`Bass`，`TrackKind::Audio`）。
fn audio_track(project: &YebanProjectV1) -> EntityId {
    project
        .tracks
        .values()
        .find(|track| track.kind == TrackKind::Audio)
        .expect("样本里必须有音频轨")
        .id
}

/// 引用音频片段的那条摆放：`(音轨身份, 摆放身份)`。
fn audio_placement(project: &YebanProjectV1, clip: EntityId) -> (EntityId, EntityId) {
    project
        .tracks
        .values()
        .find_map(|track| {
            track
                .clips
                .values()
                .find(|placement| placement.clip_id == clip)
                .map(|placement| (track.id, placement.id))
        })
        .expect("样本里的音频片段必须被摆放引用")
}

/// 第一条带设备链的音轨：`(音轨身份, 第一台设备身份)`。
fn device_target(project: &YebanProjectV1) -> (EntityId, EntityId) {
    project
        .tracks
        .values()
        .find(|track| !track.devices.is_empty())
        .map(|track| (track.id, track.devices[0].id))
        .expect("样本里必须有带设备链的音轨")
}

/// 一条 MIDI 片段及其第一个音符：`(音轨身份, 片段身份, 音符身份)`。
fn midi_target(project: &YebanProjectV1) -> (EntityId, EntityId, EntityId) {
    let (clip_id, note_id) = project
        .clip_pool
        .values()
        .find_map(|entry| {
            entry
                .content
                .notes()
                .and_then(|notes| notes.keys().next().map(|note| (entry.id, *note)))
        })
        .expect("样本里必须有带音符的 MIDI 片段");
    let track_id = project
        .tracks
        .values()
        .find(|track| track.kind == TrackKind::Midi)
        .expect("样本里必须有 MIDI 音轨")
        .id;
    (track_id, clip_id, note_id)
}

/// 判据 ①（§2.1）：**六个属性级形态在音频片段上成功**，而且真的改到目标字段。
///
/// 顶层 `(trackId, clipId)` 始终指向**音频**片段 —— 六个形态各自的寻址在操作对象
/// 里（`clipId` / `trackId` / `deviceId` / `placementId`），顶层那一对只证明它们
/// **不要求**目标是 MIDI。
#[test]
fn six_property_forms_succeed_on_an_audio_clip() {
    let (mut dispatcher, auth, project) = dispatcher();
    let audio_clip_id = audio_clip(&project);
    let audio_clip_text = audio_clip_id.to_canonical_string();
    let audio_track_id = audio_track(&project);
    let audio_track_text = audio_track_id.to_canonical_string();
    let (placement_track_id, placement_id) = audio_placement(&project, audio_clip_id);
    let placement_track_text = placement_track_id.to_canonical_string();
    let placement_text = placement_id.to_canonical_string();
    let (device_track_id, device_id) = device_target(&project);
    let device_track_text = device_track_id.to_canonical_string();
    let device_text = device_id.to_canonical_string();

    for kind in [
        "setClipName",
        "setDeviceName",
        "setDeviceBypassed",
        "setClipPlacementMuted",
        "setTrackColor",
        "setClipPlacementDuration",
    ] {
        let op = match kind {
            "setClipName" => {
                json!({"kind": kind, "clipId": audio_clip_text, "name": "Kick Renamed"})
            }
            "setDeviceName" => json!({
                "kind": kind, "trackId": device_track_text, "deviceId": device_text,
                "name": "Renamed Synth",
            }),
            "setDeviceBypassed" => json!({
                "kind": kind, "trackId": device_track_text, "deviceId": device_text,
                "bypassed": true,
            }),
            "setClipPlacementMuted" => json!({
                "kind": kind, "trackId": placement_track_text,
                "placementId": placement_text, "muted": true,
            }),
            "setTrackColor" => json!({
                "kind": kind, "trackId": audio_track_text, "color": "#22AA88",
            }),
            "setClipPlacementDuration" => json!({
                "kind": kind, "trackId": placement_track_text,
                "placementId": placement_text, "durationTicks": 1920,
            }),
            other => panic!("未登记的属性级形态: {other}"),
        };

        let digest_before = dispatcher
            .domain()
            .project_digest()
            .expect("必须有活跃工程");
        let response = call(
            &mut dispatcher,
            &auth,
            "yeban_edit_notes",
            json!({
                "trackId": audio_track_text,
                "clipId": audio_clip_text,
                "dryRun": false,
                "includeOps": true,
                "ops": [op],
            }),
        );
        assert_eq!(
            response["status"], "success",
            "`{kind}` 必须成功: {response}"
        );
        assert_eq!(
            response["data"]["pendingMerge"], true,
            "`{kind}` 必须真的建出一条待合并提案: {response}"
        );
        // ⭐ `#179` 谓词收窄的**可观察后果**：属性写入不再被报成"音符编辑"。
        // 收窄前 `is_note_level() == true` ⇒ 标题是"音符编辑: 1 步"（本判据会红）。
        assert_eq!(
            response["data"]["proposal"]["title"], "音轨级编辑: 1 步",
            "`{kind}` 改的是属性（不读音符）, 不得被报成音符编辑: {response}"
        );
        let proposal_id = response["data"]["proposal"]["proposalId"]
            .as_str()
            .expect("提案 id")
            .to_owned();
        let merged = call(
            &mut dispatcher,
            &auth,
            "yeban_merge_proposal",
            json!({"proposalId": proposal_id, "commitMessage": format!("trap-179 {kind}")}),
        );
        assert_eq!(merged["status"], "success", "`{kind}` 必须能合并: {merged}");

        let digest_after = dispatcher
            .domain()
            .project_digest()
            .expect("必须有活跃工程");
        assert_ne!(
            digest_before, digest_after,
            "`{kind}` 必须真的改了工程（摘要不得原地不动）"
        );
        let document = dispatcher.domain().active_project().expect("活跃工程");
        let changed = match kind {
            "setClipName" => document.clip_pool[&audio_clip_id].name == "Kick Renamed",
            "setDeviceName" => document.tracks[&device_track_id]
                .devices
                .iter()
                .any(|device| device.id == device_id && device.name == "Renamed Synth"),
            "setDeviceBypassed" => document.tracks[&device_track_id]
                .devices
                .iter()
                .any(|device| device.id == device_id && device.bypassed),
            "setClipPlacementMuted" => {
                document.tracks[&placement_track_id].clips[&placement_id].muted
            }
            "setTrackColor" => document.tracks[&audio_track_id].color.as_deref() == Some("#22AA88"),
            "setClipPlacementDuration" => {
                document.tracks[&placement_track_id].clips[&placement_id].duration_ticks == 1920
            }
            other => panic!("未登记的属性级形态: {other}"),
        };
        assert!(changed, "`{kind}` 必须真的改到目标字段");
        println!(
            "### 逐字：{kind} + 音频顶层 clipId ⇒ status = {} / title = {} / projectDigest {} → {} / 字段已改 = {changed}",
            response["status"],
            response["data"]["proposal"]["title"],
            &digest_before[..12],
            &digest_after[..12]
        );
    }
}

/// 判据 ②（§2.2）：**音符形态仍拦** —— 音频片段 + 音符形态 ⇒ 各一条
/// `CLIP_NOT_FOUND`，且报文逐字是"不是 MIDI 片段"那条（⛔ 不是"片段不存在"）。
///
/// ⚠ 本线新增的 `setNoteTiming` / `setNoteExpression` 也读目标音符，因此音频片段上
/// 同样必须响亮失败。
#[test]
fn four_note_forms_are_still_blocked_on_an_audio_clip() {
    let (mut dispatcher, auth, project) = dispatcher();
    let audio_clip_id = audio_clip(&project);
    let audio_clip_text = audio_clip_id.to_canonical_string();
    let audio_track_id = audio_track(&project);
    let audio_track_text = audio_track_id.to_canonical_string();
    let ghost_note = EntityId::from_str("01J8ZQ0000000000000000D179").expect("ULID");
    let ghost_text = ghost_note.to_canonical_string();

    let cases = [
        (
            "add",
            json!({"kind": "add", "note": {"startTick": 0, "pitch": 60, "durationTicks": 480}}),
        ),
        ("delete", json!({"kind": "delete", "noteId": ghost_text})),
        (
            "move",
            json!({"kind": "move", "noteId": ghost_text, "deltaTick": 120, "deltaPitch": 2}),
        ),
        (
            "velocity",
            json!({"kind": "velocity", "noteId": ghost_text, "velocity": 64}),
        ),
        (
            "setNoteTiming",
            json!({
                "kind": "setNoteTiming", "noteId": ghost_text,
                "startTick": 0, "durationTicks": 480,
            }),
        ),
        (
            "setNoteExpression",
            json!({
                "kind": "setNoteExpression", "noteId": ghost_text,
                "probability": null, "ratchet": null, "microTimingTicks": null,
            }),
        ),
    ];
    for (kind, op) in cases {
        let proposals_before = dispatcher.domain().proposal_count();
        let response = call(
            &mut dispatcher,
            &auth,
            "yeban_edit_notes",
            json!({
                "trackId": audio_track_text,
                "clipId": audio_clip_text,
                "ops": [op],
            }),
        );
        assert_eq!(
            response["status"], "error",
            "`{kind}` + 音频片段必须响亮失败: {response}"
        );
        assert_eq!(
            response["error"]["code"], "CLIP_NOT_FOUND",
            "`{kind}` 的码: {response}"
        );
        let message = response["error"]["message"]
            .as_str()
            .expect("错误必须有 message");
        assert!(
            message.contains("不是 MIDI 片段"),
            "`{kind}` 必须撞的是「必须是 MIDI 片段」那条断言, 实际: {message}"
        );
        assert_eq!(
            dispatcher.domain().proposal_count(),
            proposals_before,
            "被拒的 `{kind}` 不得留下提案"
        );
        println!(
            "### 逐字：{kind} + 音频 clipId ⇒ {} / {}",
            response["error"]["code"], message
        );
    }
}

/// 判据 ③（§2.3）：四个音符形态在 **MIDI 片段**上逐字节不变 —— 合并后的
/// `projectDigest`（= 规范 JSON 的 SHA-256）必须逐字等于**收窄前**（`d15edd62`）
/// 在**同一夹具、同一时钟**下录下的读数。
///
/// 注入（实测红）：把 `is_note_level` 收窄时顺手删掉 `Add`/`Delete`/`Move`/`Velocity`
/// 任一条 ⇒ 对应的音符形态在 MIDI 片段上被误拦（`CLIP_NOT_FOUND`），本判据先红。
#[test]
fn the_four_note_forms_are_byte_identical_on_a_midi_clip() {
    // 收窄**前**（`d15edd62`）录下的读数：形态 → 合并后的 projectDigest。
    // ⚠ 这里的四个值不是在本票代码上"自测出来的"，而是先回退到基线源码跑一次
    // 录下、再写回常量（复跑时逐字相等）。
    let pinned: [(&str, &str); 4] = [
        (
            "add",
            "bbb9f0e567a7c3ae4d6186647141a6ceda9176717847336c5e6f2c8806f709ed",
        ),
        (
            "delete",
            "ffb678803f200eeb6648816389e053830785a24824559ce004f709d037064889",
        ),
        (
            "move",
            "23e7b64b45097bc14a77e120d939818b6bf967d1418364ba4368c30e5edd3f3f",
        ),
        (
            "velocity",
            "46bc085e8ad83b8c6b1934f7577d7907a1b6b208961b732778a03112cc95a167",
        ),
    ];
    for (kind, expected) in pinned {
        let (mut dispatcher, auth, project) = dispatcher();
        let (track_id, clip_id, note_id) = midi_target(&project);
        let note_text = note_id.to_canonical_string();
        let op = match kind {
            "add" => json!({
                "kind": "add",
                "note": {"id": "01J8ZQ0000000000000000C179",
                         "startTick": 3840, "pitch": 74, "durationTicks": 480},
            }),
            "delete" => json!({"kind": "delete", "noteId": note_text}),
            "move" => json!({
                "kind": "move", "noteId": note_text, "deltaTick": 120, "deltaPitch": 2,
            }),
            "velocity" => json!({"kind": "velocity", "noteId": note_text, "velocity": 64}),
            other => panic!("未登记的音符形态: {other}"),
        };
        let digest_before = dispatcher
            .domain()
            .project_digest()
            .expect("必须有活跃工程");
        let response = call(
            &mut dispatcher,
            &auth,
            "yeban_edit_notes",
            json!({
                "trackId": track_id.to_canonical_string(),
                "clipId": clip_id.to_canonical_string(),
                "dryRun": false,
                "ops": [op],
            }),
        );
        assert_eq!(
            response["status"], "success",
            "`{kind}` 必须成功: {response}"
        );
        let proposal_id = response["data"]["proposal"]["proposalId"]
            .as_str()
            .expect("提案 id")
            .to_owned();
        let merged = call(
            &mut dispatcher,
            &auth,
            "yeban_merge_proposal",
            json!({"proposalId": proposal_id, "commitMessage": format!("trap-179 midi {kind}")}),
        );
        assert_eq!(merged["status"], "success", "`{kind}` 必须能合并: {merged}");
        let digest_after = dispatcher
            .domain()
            .project_digest()
            .expect("必须有活跃工程");
        assert_ne!(digest_before, digest_after, "`{kind}` 必须真的改了工程");
        println!(
            "### 逐字：{kind} + MIDI 片段 ⇒ status = {} / projectDigest {} → {}",
            response["status"], digest_before, digest_after
        );
        assert_eq!(
            digest_after, expected,
            "`{kind}` 在 MIDI 片段上的合并摘要必须与收窄前逐字相同"
        );
    }
}

/// 判据 ④（§2.4）：收窄**可机械核** —— 发布的音符级清单**逐字**是六个，
/// 且它与"读顶层 `clipId` 的清单"的包含关系成立（音符形态必然是 `clipId`
/// 的消费者；池级 `removeClip` 读 `clipId` 却**不**读音符）。
///
/// ⚠ 谓词与清单在**全部 40 个 kind**上的对账住在 `src/domain/notes.rs` 的
/// `every_catalog_kind_compiles_to_exactly_one_op_variant`（那里才有 kind → 规范
/// 形状的完整表）。本判据钉住**发布面**的逐字值。
///
/// 注入（实测红）：把 `NOTE_LEVEL_KINDS` 加回任一属性级形态，或把谓词改回收窄前的
/// 反选清单 ⇒ 本判据红；`six_property_forms_succeed_on_an_audio_clip` 的标题断言也红。
#[test]
fn the_note_level_predicate_is_exactly_six() {
    assert_eq!(
        NOTE_LEVEL_KINDS,
        [
            "add",
            "delete",
            "move",
            "velocity",
            "setNoteTiming",
            "setNoteExpression"
        ],
        "音符级 kind 必须逐字是六个音符形态（⛔ 不是从常量自己比出来的长度）"
    );
    assert_eq!(NOTE_LEVEL_KINDS.len(), 6, "恰好六个");
    // 每个成员都在 `OP_KINDS` 目录里（不是幽灵名字）。
    for kind in NOTE_LEVEL_KINDS {
        assert!(OP_KINDS.contains(kind), "`{kind}` 必须在 `OP_KINDS` 目录里");
        assert!(
            ENVELOPE_CLIP_KINDS.contains(kind),
            "`{kind}` 是音符形态 ⇒ 必然读顶层 `clipId`"
        );
    }
    // 池级 `removeClip` 是唯一的"读 `clipId` 但不是音符级"的 kind。
    let non_note_readers: Vec<&str> = ENVELOPE_CLIP_KINDS
        .iter()
        .copied()
        .filter(|kind| !NOTE_LEVEL_KINDS.contains(kind))
        .collect();
    assert_eq!(
        non_note_readers,
        ["removeClip"],
        "读 `clipId` 却不读音符的 kind 必须恰好是池级 `removeClip`"
    );
    println!(
        "### 逐字：NOTE_LEVEL_KINDS = {} / ENVELOPE_CLIP_KINDS = {}",
        json!(NOTE_LEVEL_KINDS),
        json!(ENVELOPE_CLIP_KINDS)
    );
}
