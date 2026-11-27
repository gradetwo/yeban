//! T-FIX-865：保存已撤销的工程后重开，redo 栈不可应用时必须返回
//! **类型化拒绝 + 下一步提示**，⛔ 不再抛裸模型 `CONFLICT`。
//!
//! 红对照：把 `undo_session::redo` 里的 `StateMismatch` 升级去掉（恢复
//! `model_refusal` 直抛）⇒ 第一条断言拿不到 `reason` 即红。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};
use yeban_mcp::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::container;
use yeban_model::samples::{filled_project, filled_project_assets};
use yeban_model::{EntityId, YebanProjectV1};

const NOW: u64 = 1_760_000_000_000;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yeban-t865-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir.join("demo.yeban")
}

fn new_dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(NOW);
    (dispatcher, auth)
}

fn call(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "t",
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    let response = outcome.response.expect("必须有响应");
    assert!(response.error.is_none(), "不该有 JSON-RPC 层错误");
    response.result.expect("必须有 result")
}

fn lead_track(project: &YebanProjectV1) -> EntityId {
    project
        .tracks
        .values()
        .find(|track| track.kind == yeban_model::TrackKind::Midi)
        .expect("规范样本必须有 MIDI 轨")
        .id
}

/// 造三步历史（addTrack / setTrackName / setTempo），全部 autoMerge 落盘成提交。
fn seed_three_commits(dispatcher: &mut Dispatcher, auth: &str) -> (EntityId, EntityId) {
    let track = lead_track(dispatcher.domain().active_project().expect("活跃工程"));
    let new_track = EntityId::new();
    call(
        dispatcher,
        auth,
        "yeban_edit_notes",
        json!({ "trackId": track.to_canonical_string(), "ops": [
            {"kind": "addTrack", "trackId": new_track.to_canonical_string(), "name": "New", "trackKind": "Midi"}
        ], "autoMerge": true }),
    );
    call(
        dispatcher,
        auth,
        "yeban_edit_notes",
        json!({ "trackId": new_track.to_canonical_string(), "ops": [
            {"kind": "setTrackName", "trackId": new_track.to_canonical_string(), "name": "Renamed"}
        ], "autoMerge": true }),
    );
    call(
        dispatcher,
        auth,
        "yeban_edit_notes",
        json!({ "trackId": track.to_canonical_string(), "ops": [
            {"kind": "setTempo", "bpm": 140}
        ], "autoMerge": true }),
    );
    (track, new_track)
}

#[test]
fn redo_after_reopening_a_partially_undone_project_is_a_typed_refusal() {
    let path = scratch("mismatch");
    let pristine = filled_project();
    let assets: BTreeMap<_, _> = filled_project_assets().into_iter().collect();
    std::fs::write(
        &path,
        container::write_project_container(&pristine, &[], &assets).expect("容器"),
    )
    .expect("写盘");

    {
        let (mut dispatcher, auth) = new_dispatcher();
        assert_eq!(
            call(
                &mut dispatcher,
                &auth,
                "yeban_open_project",
                json!({"path": path.display().to_string()})
            )["status"],
            "success"
        );
        seed_three_commits(&mut dispatcher, &auth);
        let undone = call(&mut dispatcher, &auth, "yeban_undo", json!({"steps": 2}));
        assert_eq!(undone["status"], "success");
        assert_eq!(undone["data"]["steps"], 2, "前置：撤销两步");
        let saved = call(&mut dispatcher, &auth, "yeban_save_project", json!({}));
        assert_eq!(saved["status"], "success");
    } // 释放锁

    let (mut reopened, auth2) = new_dispatcher();
    assert_eq!(
        call(
            &mut reopened,
            &auth2,
            "yeban_open_project",
            json!({"path": path.display().to_string()})
        )["status"],
        "success"
    );
    let redo = call(&mut reopened, &auth2, "yeban_redo", json!({"steps": 1}));
    assert_eq!(redo["status"], "error", "不一致必须响亮拒绝: {redo}");
    assert_eq!(redo["error"]["code"], "CONFLICT");
    assert_eq!(
        redo["error"]["data"]["reason"], "cursorProjectMismatch",
        "必须是类型化 reason，不能是裸模型 CONFLICT: {redo}"
    );
    let hint = redo["error"]["data"]["hint"]
        .as_str()
        .expect("hint 必须存在");
    assert!(
        hint.contains("重做栈不一致") || hint.contains("不能安全重做"),
        "{hint}"
    );
    assert!(
        hint.contains("请在撤销前保存"),
        "提示必须给出下一步: {hint}"
    );
    assert!(
        redo["error"]["data"]["op"].as_str().is_some(),
        "带出冲突 op: {redo}"
    );

    let _ = std::fs::remove_dir_all(path.parent().expect("父目录"));
}

#[test]
fn ordinary_redo_still_succeeds_without_a_mismatch_reason() {
    let path = scratch("normal");
    let pristine = filled_project();
    let assets: BTreeMap<_, _> = filled_project_assets().into_iter().collect();
    std::fs::write(
        &path,
        container::write_project_container(&pristine, &[], &assets).expect("容器"),
    )
    .expect("写盘");
    let (mut dispatcher, auth) = new_dispatcher();
    call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({"path": path.display().to_string()}),
    );
    seed_three_commits(&mut dispatcher, &auth);
    call(&mut dispatcher, &auth, "yeban_undo", json!({"steps": 1}));
    let redo = call(&mut dispatcher, &auth, "yeban_redo", json!({"steps": 1}));
    assert_eq!(
        redo["status"], "success",
        "正常重做不得被类型化拒绝: {redo}"
    );
    assert_eq!(redo["data"]["steps"], 1);
    let _ = std::fs::remove_dir_all(path.parent().expect("父目录"));
}
