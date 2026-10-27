//! **`ADR-0034` 的判据**：`transposeClip` —— 整片段移调是**一条形态 → N 条 `Op::MoveNote`**
//! 的**展开**（缺的是**寻址粒度**，⛔ 不是新 `Op`）。
//!
//! 本文件钉住 `ADR-0034` D1…D5 的每一条可测断言，⛔ 复制 D4（段级目标调 /
//! in-scale 转调 / 平滑声部连接）—— 那三条是**待人类裁决**的规范缺口，
//! 本层的实现里⛔ 没有任何一处读 `key_signature`。
//!
//! | 判据 | 对象 | 读数 |
//! | :--- | :--- | :--- |
//! | [`transpose_clip_expands_into_one_move_per_note_in_a_single_batch`] | 一次 `yeban_edit_notes`（`autoMerge`） | `data.merge.appliedOps` == 片段的音符数 N ＋ `data.autoMerged: true` ＋ `commitModel: "proposalAutoMerged"`（D1/D2：一个批 = 一次提交） |
//! | [`every_note_moves_in_pitch_and_nothing_moves_in_time`] | 合并后的 `query_project` | N 条 `pitch` 逐条 `+2`；`startTick` / `durationTicks` / `velocity` / 身份集合**逐字节不变**（D1：`deltaTick` 恒 0） |
//! | [`one_undo_step_reverts_the_whole_transposition`] | 一次 `yeban_undo` | 工程摘要**回到**合并前的值（D2：一步撤销，⛔ N 步） |
//! | [`an_out_of_range_note_rejects_the_whole_batch_without_partial_apply`] | `deltaPitch: 127`（或全零片段时的 `-128`） | `OUT_OF_RANGE` ＋ `data.noteId` ＋ 合并前后摘要一位不动（D3：整批拒绝，⛔ 部分应用） |
//! | [`the_payload_names_no_note_and_rejects_every_illegal_shape`] | 四种非法载荷 | `noteId` ⇒ `unknownTransposeClipField`；缺 `deltaPitch` / `2.5` ⇒ `INVALID_PARAMETER_RANGE`；`128` ⇒ `OUT_OF_RANGE`（D5） |
//! | [`an_empty_midi_clip_expands_to_zero_ops_and_is_not_an_error`] | 空 MIDI 片段的 `compile` | `Ok(vec![])`（D5：`appliedOps: 0` 是**合法**结果，⛔ 不是错误码） |
//! | [`the_form_is_published_on_both_surfaces`] | `tools/list` 的 `ops.items` 分支 | `additionalProperties: false` ＋ `x-envelope.clipId: "required"` ＋ `deltaPitch` 的 `±128` 界（D1 的发布面） |
//!
//! 复跑读数：`cargo test -p yeban-mcp --test transpose_clip -- --nocapture`

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::notes::{
    ENVELOPE_CLIP_KINDS, NOTE_LEVEL_KINDS, TRANSPOSE_CLIP_FIELDS, TRANSPOSE_CLIP_KIND, compile,
    parse_ops,
};
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::{ClipContent, EntityId, TrackKind, YebanProjectV1};

/// 注入规范样本工程的分发器（与 `tests/trap_179.rs` 同一形状）。
fn dispatcher() -> (Dispatcher, String, YebanProjectV1) {
    let token = BearerToken::generate().token;
    let authorization = format!("Bearer {}", token.expose());
    let project = yeban_model::samples::filled_project();
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            PathBuf::from("/tmp/yeban-transpose-clip.yeban"),
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
        "id": "transpose-clip",
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

/// 一条音符的逐字段读数（**前后同一形状**，因此可以整条比较）。
///
/// 单位：`(身份, start_tick, pitch, duration_ticks, velocity)`。模型的 serde 键名是
/// 蛇形、`ClipContent` 是外标签 ⇒ 本文件的**两份**读数都走**类型**（`YebanProjectV1`
/// 的反序列化对偶 / `MidiNote` 的字段），⛔ 从响应里猜 JSON 字段名。
/// 命名成结构体⛔ 用五位元组：clippy 的 `type_complexity` 在 `-D warnings` 下会
/// 直接把 `Vec<(String, u64, u8, u64, u8)>` 报成编译错误（本轮实测第二条红）。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct NoteRow {
    id: String,
    start_tick: u64,
    pitch: u8,
    duration_ticks: u64,
    velocity: u8,
}

/// 样本里**音符最多**的 MIDI 片段：`(音轨身份, 片段身份, 逐音符读数)`。
///
/// 取"最多"是为了让 `appliedOps == N` 这条读数⛔ 恰好落在 1 上（N=1 时
/// "展开"与"一条 `move`"看不出区别）。
fn midi_target(project: &YebanProjectV1) -> (EntityId, EntityId, Vec<NoteRow>) {
    let entry = project
        .clip_pool
        .values()
        .filter(|entry| entry.content.notes().is_some())
        .max_by_key(|entry| entry.content.notes().map_or(0, |notes| notes.len()))
        .expect("样本里必须有带音符的 MIDI 片段");
    let mut rows: Vec<NoteRow> = entry
        .content
        .notes()
        .expect("上面筛过")
        .iter()
        .map(|(id, note)| NoteRow {
            id: id.to_canonical_string(),
            start_tick: note.start_tick,
            pitch: note.pitch,
            duration_ticks: note.duration_ticks,
            velocity: note.velocity,
        })
        .collect();
    rows.sort();
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
        .expect("样本里必须有能寻址到该片段的音轨（或一条 MIDI 轨）")
        .0;
    (track_id, entry.id, rows)
}

/// 从 `query_project` 里读一条片段的逐音符读数（与 [`midi_target`] 同一形状）。
///
/// ⚠ 读数口径 = **把工具返回的 `data.project` 用模型自己的反序列化器读回类型**，
/// ⛔ 从响应里猜 JSON 字段名（`start_tick` 还是 `startTick`、`"Midi"` 那层标签）。
/// 默认视图是 `view::project_view` 的**全量顶层序列化**（`fields` 为空 ⇒ 不裁剪），
/// 而容器层的双向对偶就是 `YebanProjectV1` 的 serde —— 用它，字段名不会漂移。
fn read_back(auth: &str, dispatcher: &mut Dispatcher, clip: &EntityId) -> Vec<NoteRow> {
    let view = call(dispatcher, auth, "yeban_query_project", json!({}));
    assert_eq!(view["status"], "success", "{view}");
    let document: YebanProjectV1 =
        serde_json::from_value(view["data"]["project"].clone()).expect("视图必须能读回模型类型");
    let notes = document
        .clip_pool
        .get(clip)
        .unwrap_or_else(|| panic!("片段 {clip} 必须还在池里"))
        .content
        .notes()
        .unwrap_or_else(|| panic!("片段必须是 MIDI 且带音符表: {clip}"));
    let mut rows: Vec<NoteRow> = notes
        .iter()
        .map(|(id, note)| NoteRow {
            id: id.to_canonical_string(),
            start_tick: note.start_tick,
            pitch: note.pitch,
            duration_ticks: note.duration_ticks,
            velocity: note.velocity,
        })
        .collect();
    rows.sort();
    rows
}

/// ⭐ 判据 1（D1 + D2）：**一条形态展开成 N 条 `Op::MoveNote`，全在同一个批里**。
#[test]
fn transpose_clip_expands_into_one_move_per_note_in_a_single_batch() {
    let (mut dispatcher, auth, project) = dispatcher();
    let (track, clip, rows) = midi_target(&project);
    let want = rows.len();
    assert!(
        want > 1,
        "夹具片段必须至少两个音符, 否则测不到展开: {rows:?}"
    );

    let merged = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track.to_canonical_string(),
            "clipId": clip.to_canonical_string(),
            "ops": [{"kind": TRANSPOSE_CLIP_KIND, "deltaPitch": 2}],
            "autoMerge": true,
            "commitMessage": "整片段 +2 半音",
        }),
    );
    assert_eq!(merged["status"], "success", "{merged}");
    // ⚠ `edit_notes` + `autoMerge` 的回执是**两段式**（`ADR-0029` D3）：合并的那半
    // 挂在 `data.merge` 里（`domain/mod.rs:4275`），顶层只有 `data.autoMerged`。
    assert_eq!(merged["data"]["autoMerged"], true, "{merged}");
    assert_eq!(
        merged["data"]["merge"]["merged"], true,
        "一个批 ⇒ 一次提交: {merged}"
    );
    assert_eq!(
        merged["data"]["merge"]["appliedOps"]
            .as_u64()
            .expect("appliedOps") as usize,
        want,
        "`appliedOps` 必须**恰好**等于片段的音符数（一条形态 → N 条 `Op`）"
    );
    // `ADR-0032`：回执自述提交模型。`edit_notes` 是提案类, `autoMerge` 后是
    // `proposalAutoMerged` —— ⛔ 不是 `direct`（那是 `edit_automation` 那一族）。
    assert_eq!(
        merged["data"]["commitModel"], "proposalAutoMerged",
        "{merged}"
    );
    println!(
        "### 逐字：N = {want} / appliedOps = {} / title = {}",
        merged["data"]["merge"]["appliedOps"], merged["data"]["proposal"]["title"]
    );
}

/// ⭐ 判据 2（D1）：**只动音高、⛔ 动时间**，且身份集合一位不变。
#[test]
fn every_note_moves_in_pitch_and_nothing_moves_in_time() {
    let (mut dispatcher, auth, project) = dispatcher();
    let (track, clip, rows) = midi_target(&project);
    let before = read_back(&auth, &mut dispatcher, &clip);
    assert_eq!(before.len(), rows.len(), "读数口径必须与夹具一致");

    call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track.to_canonical_string(),
            "clipId": clip.to_canonical_string(),
            "ops": [{"kind": TRANSPOSE_CLIP_KIND, "deltaPitch": 2}],
            "autoMerge": true,
        }),
    );
    let after = read_back(&auth, &mut dispatcher, &clip);

    assert_eq!(
        after.len(),
        before.len(),
        "移调⛔ 增删音符: 前 {} 条 / 后 {} 条",
        before.len(),
        after.len()
    );
    for (before_row, after_row) in before.iter().zip(&after) {
        assert_eq!(before_row.id, after_row.id, "身份必须逐条对齐（⛔ 换 id）");
        assert_eq!(
            before_row.start_tick, after_row.start_tick,
            "`deltaTick` 恒 0 ⇒ 起点一位不动"
        );
        assert_eq!(
            before_row.duration_ticks, after_row.duration_ticks,
            "时值⛔ 被波及"
        );
        assert_eq!(before_row.velocity, after_row.velocity, "力度⛔ 被波及");
        assert_eq!(
            i16::from(after_row.pitch) - i16::from(before_row.pitch),
            2,
            "每条都必须恰好 +2 半音（半音级：⛔ 读谱调 ⇒ `C♯` 与 `F` 一样各升两格）"
        );
    }
}

/// ⭐ 判据 3（D2）：**一次 `yeban_undo` 撤销整个转调**（⛔ 需要 N 次）。
#[test]
fn one_undo_step_reverts_the_whole_transposition() {
    let (mut dispatcher, auth, project) = dispatcher();
    let (track, clip, _) = midi_target(&project);
    let before = read_back(&auth, &mut dispatcher, &clip);

    call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track.to_canonical_string(),
            "clipId": clip.to_canonical_string(),
            "ops": [{"kind": TRANSPOSE_CLIP_KIND, "deltaPitch": -5}],
            "autoMerge": true,
        }),
    );
    let moved = read_back(&auth, &mut dispatcher, &clip);
    assert_ne!(moved, before, "夹具必须先真的改动, 否则本判据测的是空气");

    let undone = call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(undone["status"], "success", "{undone}");
    assert_eq!(
        read_back(&auth, &mut dispatcher, &clip),
        before,
        "一步撤销必须把**整批** N 条 `MoveNote` 全部还原"
    );
}

/// ⭐ 判据 4（D3）：**任一条越界 ⇒ 整批 `OUT_OF_RANGE`，⛔ 部分应用**。
#[test]
fn an_out_of_range_note_rejects_the_whole_batch_without_partial_apply() {
    let (mut dispatcher, auth, project) = dispatcher();
    let (track, clip, rows) = midi_target(&project);
    let before = read_back(&auth, &mut dispatcher, &clip);
    // 取**能进 `i8` 的最远的一端**：`deltaPitch: 127` 对任何 `pitch ≥ 1` 的音符都越界
    // （`1 + 127 = 128 > 127`）；片段全为 `pitch == 0` 时改用 `-128`（`0 - 128 < 0`）。
    // ⛔ 用 `128 - highest` 那种算法：`highest == 0` 时它会算出 `128`，在 `i8` 外 panic。
    let highest = rows.iter().map(|row| row.pitch).max().expect("非空");
    let delta: i8 = if highest > 0 { 127 } else { -128 };

    let rejected = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track.to_canonical_string(),
            "clipId": clip.to_canonical_string(),
            "ops": [{"kind": TRANSPOSE_CLIP_KIND, "deltaPitch": delta}],
            "autoMerge": true,
        }),
    );
    assert_eq!(rejected["status"], "failure", "{rejected}");
    assert_eq!(
        rejected["error"]["code"], "OUT_OF_RANGE",
        "越界必须在**构造期**就报（与逐条 `move` 同一份算式、同一个码）: {rejected}"
    );
    assert!(
        rejected["error"]["data"]["noteId"].is_string(),
        "报文必须点名**是哪一条**越界: {}",
        rejected["error"]["data"]
    );
    assert_eq!(
        read_back(&auth, &mut dispatcher, &clip),
        before,
        "整批拒绝 ⇒ 工程一个字节都不能动"
    );
}

/// ⭐ 判据 5（D5）：四种非法载荷各自响亮。
#[test]
fn the_payload_names_no_note_and_rejects_every_illegal_shape() {
    let ulid = "01J8ZQ00000000000000000999";
    // ① `noteId` —— 那是它**要取代**的写法，必须被点名为未知键。
    let fault = parse_ops(&json!([
        {"kind": TRANSPOSE_CLIP_KIND, "noteId": ulid, "deltaPitch": 2}
    ]))
    .expect_err("`noteId` 不是本形态的键");
    let value = fault.into_result().expect("带内");
    assert_eq!(value["error"]["code"], "INVALID_PARAMETER_RANGE", "{value}");
    assert_eq!(
        value["error"]["data"]["reason"], "unknownTransposeClipField",
        "{value}"
    );
    assert_eq!(
        value["error"]["data"]["supportedFields"],
        serde_json::json!(TRANSPOSE_CLIP_FIELDS),
        "必须逐字公布本形态的键集"
    );

    // ②③④ 缺键 / 非整数 / 出 `i8`：形状错误落 `INVALID_PARAMETER_RANGE`，
    // 区间错误落 `OUT_OF_RANGE`（`read_i8` 与逐条 `move` 用的是同一份算式）。
    for (payload, want_code) in [
        (
            json!({"kind": TRANSPOSE_CLIP_KIND}),
            "INVALID_PARAMETER_RANGE",
        ),
        (
            json!({"kind": TRANSPOSE_CLIP_KIND, "deltaPitch": 2.5}),
            "INVALID_PARAMETER_RANGE",
        ),
        (
            json!({"kind": TRANSPOSE_CLIP_KIND, "deltaPitch": 128}),
            "OUT_OF_RANGE",
        ),
    ] {
        let fault =
            parse_ops(&json!([payload.clone()])).expect_err(&format!("必须被拒: {payload}"));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["code"], want_code, "{payload} ⇒ {value}");
    }

    // 本形态**读**顶层 `clipId`、**读**音符：两张发布清单都必须有它（判据 7 之外再钉一次）。
    assert!(ENVELOPE_CLIP_KINDS.contains(&TRANSPOSE_CLIP_KIND));
    assert!(NOTE_LEVEL_KINDS.contains(&TRANSPOSE_CLIP_KIND));
}

/// ⭐ 判据 6（D5）：**空 MIDI 片段展开成 0 条 `Op`**，是合法结果、⛔ 不是错误。
#[test]
fn an_empty_midi_clip_expands_to_zero_ops_and_is_not_an_error() {
    let (_, _, project) = dispatcher();
    let (track, clip, _) = midi_target(&project);
    // 把那条片段的**内容**换成空音符表（身份、名字、摆放、其余文档逐字节不动）——
    // 用模型自己的构造，⛔ 造假身份。
    let mut document = project.clone();
    let entry = document
        .clip_pool
        .get_mut(&clip)
        .expect("夹具片段必须在池里");
    entry.content = ClipContent::Midi {
        notes: BTreeMap::new(),
    };

    let ops = parse_ops(&json!([{"kind": TRANSPOSE_CLIP_KIND, "deltaPitch": 7}])).expect("可解析");
    let compiled = compile(&document, &track, &clip, &ops)
        .unwrap_or_else(|fault| panic!("空片段必须能编译, 实际 {fault:?}"));
    assert_eq!(
        compiled.len(),
        0,
        "0 个音符 ⇒ 展开 0 条 `Op`（⛔ 不是错误码）"
    );
}

/// ⭐ 判据 7（D1 的发布面）：`tools/list` 的分支形状与 domain 层同源。
#[test]
fn the_form_is_published_on_both_surfaces() {
    let schema = yeban_mcp::op_schema::ops_items();
    let branch = schema["oneOf"]
        .as_array()
        .expect("oneOf")
        .iter()
        .find(|item| item["properties"]["kind"]["const"] == TRANSPOSE_CLIP_KIND)
        .expect("`transposeClip` 必须有 `oneOf` 分支");
    assert_eq!(
        branch["additionalProperties"], false,
        "本形态**有**键集守卫 ⇒ 分支必须封闭: {branch}"
    );
    assert_eq!(
        branch["x-envelope"]["clipId"], "required",
        "它读顶层 `clipId` ⇒ 广播面必须这么说"
    );
    assert_eq!(
        branch["required"],
        json!(["kind", "deltaPitch"]),
        "{branch}"
    );
    assert_eq!(
        branch["properties"]["deltaPitch"]["minimum"], -128,
        "{branch}"
    );
    assert_eq!(
        branch["properties"]["deltaPitch"]["maximum"], 127,
        "{branch}"
    );
    // 分支描述必须写明"展开成 N 条 `move`"与"半音级", 否则模型只能猜。
    let description = branch["description"].as_str().expect("描述");
    for phrase in ["整片段", "每个", "展开", "半音"] {
        assert!(
            description.contains(phrase),
            "描述必须逐字写出 {phrase}: {description}"
        );
    }
}
