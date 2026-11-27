//! T-FIX-866：`yeban_edit_notes` 的 `humanizeClip` 在 **schema 合规省略**
//! `timeJitterTicks` / `velocityJitter` 时必须成功，并与显式 0 等价。
//!
//! 这条走真 `tools/call`（不是只调 `parse_ops`）：上报的失败点就在工具入口。

use std::path::PathBuf;

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::{EntityId, TrackKind, YebanProjectV1};

fn dispatcher() -> (Dispatcher, String, YebanProjectV1) {
    let token = BearerToken::generate().token;
    let authorization = format!("Bearer {}", token.expose());
    let project = yeban_model::samples::filled_project();
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            PathBuf::from("/tmp/yeban-humanize-optional.yeban"),
            project.clone(),
            false,
        )
        .expect("注入规范工程");
    (dispatcher, authorization, project)
}

fn call(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "humanize-optional",
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    let response = outcome.response.expect("必须有响应");
    assert!(
        response.error.is_none(),
        "JSON-RPC 层错误: {:?}",
        response.error
    );
    response.result.expect("必须有 result")
}

fn midi_target(project: &YebanProjectV1) -> (EntityId, EntityId, usize) {
    let entry = project
        .clip_pool
        .values()
        .filter(|entry| entry.content.notes().is_some())
        .max_by_key(|entry| entry.content.notes().map_or(0, |notes| notes.len()))
        .expect("样本里必须有带音符的 MIDI 片段");
    let track_id = *project
        .tracks
        .iter()
        .find(|(_, track)| {
            track
                .clips
                .values()
                .any(|placement| placement.clip_id == entry.id)
        })
        .or_else(|| {
            project
                .tracks
                .iter()
                .find(|(_, track)| track.kind == TrackKind::Midi)
        })
        .expect("必须能寻址到该片段")
        .0;
    let note_count = entry.content.notes().expect("上面筛过").len();
    (track_id, entry.id, note_count)
}

fn read_full_project(auth: &str, dispatcher: &mut Dispatcher) -> YebanProjectV1 {
    let view = call(dispatcher, auth, "yeban_query_project", json!({}));
    assert_eq!(view["status"], "success", "{view}");
    let full = call(
        dispatcher,
        auth,
        "yeban_query_project",
        json!({"fields": ["clip_pool.full"]}),
    );
    assert_eq!(full["status"], "success", "{full}");
    let mut document = view["data"]["project"].clone();
    document["clip_pool"] = full["data"]["project"]["clip_pool"].clone();
    serde_json::from_value(document).expect("视图必须能读回模型类型")
}

fn note_signature(document: &YebanProjectV1, clip: &EntityId) -> String {
    let notes = document
        .clip_pool
        .get(clip)
        .expect("片段必须还在")
        .content
        .notes()
        .expect("MIDI 片段");
    let rows: Vec<_> = notes
        .iter()
        .map(|(id, note)| {
            (
                id.to_canonical_string(),
                note.start_tick,
                note.pitch,
                note.duration_ticks,
                note.velocity,
                note.micro_timing_ticks,
            )
        })
        .collect();
    format!("{rows:?}")
}

fn run(model_case: &str) -> (usize, String) {
    let (mut dispatcher, auth, project) = dispatcher();
    let (track, clip, note_count) = midi_target(&project);
    let mut op = json!({"kind": "humanizeClip", "seed": 42});
    if model_case == "structured" {
        op["model"] = json!("structured");
    }
    let response = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track.to_canonical_string(),
            "clipId": clip.to_canonical_string(),
            "ops": [op],
            "autoMerge": true,
        }),
    );
    assert_eq!(response["status"], "success", "{model_case}: {response}");
    let applied = response["data"]["merge"]["appliedOps"]
        .as_u64()
        .unwrap_or_else(|| panic!("回执缺 appliedOps: {response}")) as usize;
    assert_eq!(
        applied,
        note_count * 2,
        "humanizeClip 每音符两条 op（时值表达式 + 力度）"
    );
    let after = read_full_project(&auth, &mut dispatcher);
    (note_count, note_signature(&after, &clip))
}

/// 省略两字段必须成功；默认 uniform 与 structured 下都与显式 0 同形同产物。
#[test]
fn edit_notes_humanize_clip_accepts_omitted_jitters_like_explicit_zero() {
    let (_, implicit_uniform) = run("uniform");
    let (_, implicit_structured) = run("structured");

    // 显式 0：同一路径，补上两键。
    let explicit = |model_case: &str| {
        let (mut dispatcher, auth, project) = dispatcher();
        let (track, clip, note_count) = midi_target(&project);
        let mut op = json!({
            "kind": "humanizeClip", "seed": 42,
            "timeJitterTicks": 0, "velocityJitter": 0,
        });
        if model_case == "structured" {
            op["model"] = json!("structured");
        }
        let response = call(
            &mut dispatcher,
            &auth,
            "yeban_edit_notes",
            json!({
                "trackId": track.to_canonical_string(),
                "clipId": clip.to_canonical_string(),
                "ops": [op],
                "autoMerge": true,
            }),
        );
        assert_eq!(
            response["status"], "success",
            "explicit {model_case}: {response}"
        );
        assert_eq!(
            response["data"]["merge"]["appliedOps"].as_u64(),
            Some((note_count * 2) as u64)
        );
        let after = read_full_project(&auth, &mut dispatcher);
        note_signature(&after, &clip)
    };
    assert_eq!(
        implicit_uniform,
        explicit("uniform"),
        "uniform：省略 timeJitterTicks/velocityJitter ⇔ 显式 0"
    );
    assert_eq!(
        implicit_structured,
        explicit("structured"),
        "structured：省略 timeJitterTicks/velocityJitter ⇔ 显式 0"
    );
}
