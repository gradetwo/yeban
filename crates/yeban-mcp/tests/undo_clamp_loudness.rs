//! T-FIX-864：撤销/重做的步数钳位必须**响亮**回执。
//!
//! 钳位语义不变（`steps:10000` 仍撤到底并如实回实际步数），但：
//! 1. `requestedSteps != steps` 时回执 `data.clamped == true`；
//! 2. `content[0].text` 必须带出请求/实际步数的警告，只读 content 的客户端
//!    也能一眼看到输入被改小了；
//! 3. 正常步数回执**不带** `clamped` 键，也不带警告 —— 既有形状不回归。
//!
//! 红对照：把 `apply_undo` / `apply_redo` 的 `clamped` 注入或
//! `envelope_summary` 的警告段删掉 ⇒ 本文件的钳位断言即红。

use serde_json::{Value, json};
use yeban_mcp::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::samples::filled_project;

fn setup() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::env::temp_dir().join("yeban-t864-clamp.yeban"),
            filled_project(),
            false,
        )
        .expect("注入工程");
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
    assert!(
        response.error.is_none(),
        "不该有 JSON-RPC 层错误: {:?}",
        response.error
    );
    let result = response.result.expect("必须有 result");
    assert_eq!(result["status"], "success", "{name} 必须成功: {result}");
    result
}

fn content_text(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .expect("content[0].text")
        .to_owned()
}

/// 用三次真编辑造出三步可撤销历史（每次 autoMerge 成一条提交）。
fn seed_history(dispatcher: &mut Dispatcher, auth: &str) {
    let track = dispatcher
        .domain()
        .active_project()
        .expect("活跃工程")
        .tracks
        .values()
        .find(|track| track.kind == yeban_model::TrackKind::Midi)
        .expect("规范样本必须有 MIDI 轨")
        .id
        .to_canonical_string();
    for i in 0..3 {
        call(
            dispatcher,
            auth,
            "yeban_edit_notes",
            json!({
                "trackId": track,
                "ops": [{"kind": "setTitle", "title": format!("T{i}")}],
                "autoMerge": true,
            }),
        );
    }
}

#[test]
fn oversized_undo_reports_clamped_and_warns_in_content() {
    let (mut dispatcher, auth) = setup();
    seed_history(&mut dispatcher, &auth);

    let result = call(
        &mut dispatcher,
        &auth,
        "yeban_undo",
        json!({"steps": 10000}),
    );
    let data = &result["data"];
    assert_eq!(data["clamped"], true, "超大步数必须回 clamped:true: {data}");
    assert_eq!(data["requestedSteps"], 10000);
    let actual = data["steps"].as_u64().expect("实际步数");
    assert!(
        actual > 0 && actual < 10000,
        "实际步数必须是钳位后的值: {data}"
    );

    let text = content_text(&result);
    assert!(
        text.contains("步数已钳位"),
        "content 文案必须明示钳位: {text}"
    );
    assert!(text.contains("10000"), "content 文案必须带请求步数: {text}");
    assert!(
        text.contains(&actual.to_string()),
        "content 文案必须带实际步数 {actual}: {text}"
    );
}

#[test]
fn oversized_redo_reports_clamped_and_warns_in_content() {
    let (mut dispatcher, auth) = setup();
    seed_history(&mut dispatcher, &auth);
    call(
        &mut dispatcher,
        &auth,
        "yeban_undo",
        json!({"steps": 10000}),
    );

    let result = call(
        &mut dispatcher,
        &auth,
        "yeban_redo",
        json!({"steps": 10000}),
    );
    let data = &result["data"];
    assert_eq!(data["clamped"], true, "redo 同型钳位必须同口径: {data}");
    assert_eq!(data["requestedSteps"], 10000);
    let text = content_text(&result);
    assert!(
        text.contains("步数已钳位"),
        "content 文案必须明示钳位: {text}"
    );
    assert!(text.contains("10000"), "content 文案必须带请求步数: {text}");
}

#[test]
fn normal_steps_do_not_carry_clamped_or_warning() {
    let (mut dispatcher, auth) = setup();
    seed_history(&mut dispatcher, &auth);

    let undo = call(&mut dispatcher, &auth, "yeban_undo", json!({"steps": 1}));
    assert!(
        undo["data"].get("clamped").is_none(),
        "正常撤销不得带 clamped: {}",
        undo["data"]
    );
    assert!(
        !content_text(&undo).contains("步数已钳位"),
        "正常撤销 content 不得报警告"
    );

    let redo = call(&mut dispatcher, &auth, "yeban_redo", json!({"steps": 1}));
    assert!(
        redo["data"].get("clamped").is_none(),
        "正常重做不得带 clamped: {}",
        redo["data"]
    );
    assert!(
        !content_text(&redo).contains("步数已钳位"),
        "正常重做 content 不得报警告"
    );
}
