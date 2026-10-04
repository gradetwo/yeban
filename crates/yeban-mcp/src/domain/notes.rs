//! `yeban_edit_notes` 的 `NoteOp` → [`Op`] 编译 [MCP-TOOL-006]。
//!
//! ## 逆操作**不在这里**
//!
//! 本模块只负责把 JSON 编译成 `yeban-model` 的 `Op` 变体。逆操作一律由
//! [`Op::invert`] 提供 —— 在 MCP 层再写一套 `invert` 就会有两份会漂移的真相
//! （判据 `note_ops_are_reversible_through_the_model_inverse` 直接复用模型侧的判据思路：
//! `apply` 之后 `invert` 回去，序列化字节必须回到原样）。
//!
//! ## 规范缺口（`NoteOp` 的形状没有契约）
//!
//! `schemas/mcp-tools.schema.json` 把 `ops` 声明为无约束数组，
//! `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §7.2 只写 `ops: Vec<NoteOp>`。
//! 本模块因此**定义了**四种 `kind`（`add` / `delete` / `move` / `velocity`）作为
//! 本地决策，并把它登记为待裁决项（见 `docs/ledger/tools-domain-notes.md`）。

use std::str::FromStr as _;

use serde_json::{Map, Value};

use yeban_model::{EntityId, MidiNote, Op, YebanProjectV1};

use super::error::{Fault, from_model};
use super::ids::deterministic_id;
use crate::tools::ErrorCode;

/// 单个片段的**发声数**上限（同时发声的音符数）。
///
/// §7.2 要求 `yeban_edit_notes` "自动进行音域与发声数合法性校验"；
/// 音域（`0..=127`）由模型层把关，发声数这一半由本常量把关。
/// 32 是"一个片段内同时 32 个音符"的保守上限（超过它多半是批量生成的产物，
/// 而不是编曲意图）；超过即 `OUT_OF_RANGE` 并回报**峰值重叠**与位置。
pub const MAX_POLYPHONY: usize = 32;

/// 一个音符编辑操作。
#[derive(Clone, Debug, PartialEq)]
pub enum NoteOp {
    /// 插入音符。
    Add {
        /// 完整音符。
        note: Box<MidiNote>,
    },
    /// 删除音符。
    Delete {
        /// 音符身份。
        note_id: EntityId,
    },
    /// 平移音符。
    Move {
        /// 音符身份。
        note_id: EntityId,
        /// tick 增量。
        delta_tick: i64,
        /// 半音增量。
        delta_pitch: i8,
    },
    /// 修改力度。
    Velocity {
        /// 音符身份。
        note_id: EntityId,
        /// 新力度 `0..=127`。
        velocity: u8,
    },
}

/// 解析 `arguments.ops`。
///
/// 支持的 `kind`（本地定义，见模块头）：
///
/// ```json
/// {"kind":"add","note":{"id":"<可选 26 字符 ULID>","startTick":0,"pitch":60,
///                       "durationTicks":480,"velocity":100}}
/// {"kind":"delete","noteId":"<ULID>"}
/// {"kind":"move","noteId":"<ULID>","deltaTick":960,"deltaPitch":12}
/// {"kind":"velocity","noteId":"<ULID>","velocity":80}
/// ```
///
/// # Errors
///
/// - `ops` 不是数组 / 元素不是对象 / 缺字段 / 字段类型不对 →
///   `INVALID_PARAMETER_RANGE`（含未知 `kind`）；
/// - 音高、力度、时值越界 → `OUT_OF_RANGE`；
/// - 身份文本不是合法 ULID → `INVALID_PARAMETER_RANGE`。
pub fn parse_ops(value: &Value) -> Result<Vec<NoteOp>, Fault> {
    let Value::Array(items) = value else {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`ops` 必须是数组",
        ));
    };
    if items.is_empty() {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`ops` 不得为空数组（空操作不是一次编辑请求）",
        ));
    }
    items.iter().map(parse_one).collect()
}

/// 解析单个操作。
fn parse_one(item: &Value) -> Result<NoteOp, Fault> {
    let object = item.as_object().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`ops` 的元素必须是对象, 实际收到 {item}"),
        )
    })?;
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| missing("kind", "字符串"))?;
    match kind {
        "add" => {
            let note = object
                .get("note")
                .and_then(Value::as_object)
                .ok_or_else(|| missing("note", "对象"))?;
            Ok(NoteOp::Add {
                note: Box::new(parse_note(note)?),
            })
        }
        "delete" => Ok(NoteOp::Delete {
            note_id: read_id(object, "noteId")?,
        }),
        "move" => Ok(NoteOp::Move {
            note_id: read_id(object, "noteId")?,
            delta_tick: read_i64(object, "deltaTick")?,
            delta_pitch: read_i8(object, "deltaPitch")?,
        }),
        "velocity" => Ok(NoteOp::Velocity {
            note_id: read_id(object, "noteId")?,
            velocity: read_range(object, "velocity", 0, 127)?,
        }),
        other => Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("未知 `kind`: `{other}`"),
            serde_json::json!({ "supportedKinds": ["add", "delete", "move", "velocity"] }),
        )),
    }
}

/// 解析 `note` 对象。
fn parse_note(object: &Map<String, Value>) -> Result<MidiNote, Fault> {
    let id = match object.get("id") {
        None | Some(Value::Null) => deterministic_id(&format!(
            "note:{start}:{pitch}:{duration}",
            start = object.get("startTick").and_then(Value::as_u64).unwrap_or(0),
            pitch = object.get("pitch").and_then(Value::as_u64).unwrap_or(0),
            duration = object
                .get("durationTicks")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        )),
        Some(value) => {
            let text = value.as_str().ok_or_else(|| {
                Fault::domain(ErrorCode::InvalidParameterRange, "`note.id` 必须是字符串")
            })?;
            EntityId::from_str(text).map_err(|error| {
                Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    format!("`note.id` 不是合法 ULID: {error}"),
                )
            })?
        }
    };
    let start_tick = read_u64(object, "startTick")?;
    let pitch = read_range(object, "pitch", 0, 127)?;
    let duration_ticks = read_u64(object, "durationTicks")?;
    let velocity = match object.get("velocity") {
        None => yeban_model::music::DEFAULT_VELOCITY,
        Some(_) => read_range(object, "velocity", 0, 127)?,
    };
    let mut note = MidiNote::new(id, start_tick, pitch, duration_ticks);
    note.velocity = velocity;
    note.validate()
        .map_err(|error| from_model("音符校验", &error))?;
    Ok(note)
}

/// 把解析过的操作编译成 [`Op`]（**读文档**补齐撤销载荷，但绝不改文档）。
///
/// `Delete` / `Move` / `Velocity` 需要当前音符状态：`DeleteNote` 自带
/// `previous_note`，`MoveNote` 与 `ModifyNoteVelocity` 的前置条件会核对
/// "音符此刻确实存在且内容一致"。
///
/// # Errors
///
/// - 片段不存在 / 不是 MIDI 片段 → `CLIP_NOT_FOUND`；
/// - 音符不存在 → `ENTITY_NOT_FOUND`；
/// - 模型层校验失败 → [`super::error::code_for_model`] 给出的契约码。
pub fn compile(
    project: &YebanProjectV1,
    track_id: &EntityId,
    clip_id: &EntityId,
    ops: &[NoteOp],
) -> Result<Vec<Op>, Fault> {
    project
        .track(track_id)
        .map_err(|error| from_model("音轨查找", &error))?;
    let entry = project
        .clip_pool
        .get(clip_id)
        .ok_or_else(|| Fault::domain(ErrorCode::ClipNotFound, format!("片段不存在: {clip_id}")))?;
    if entry.content.notes().is_none() {
        return Err(Fault::domain(
            ErrorCode::ClipNotFound,
            format!("片段 {clip_id} 不是 MIDI 片段, 没有音符集合"),
        ));
    }

    let mut compiled = Vec::with_capacity(ops.len());
    for op in ops {
        compiled.push(match op {
            NoteOp::Add { note } => Op::AddNote {
                track_id: *track_id,
                clip_id: *clip_id,
                note: (**note).clone(),
            },
            NoteOp::Delete { note_id } => {
                let previous_note = project
                    .note(clip_id, note_id)
                    .map_err(|error| from_model("音符查找", &error))?
                    .clone();
                Op::DeleteNote {
                    track_id: *track_id,
                    clip_id: *clip_id,
                    note_id: *note_id,
                    previous_note,
                }
            }
            NoteOp::Move {
                note_id,
                delta_tick,
                delta_pitch,
            } => {
                let current = project
                    .note(clip_id, note_id)
                    .map_err(|error| from_model("音符查找", &error))?;
                // 音域与时间轴越界在**这里**就报 OUT_OF_RANGE（§7.2 给本工具声明的码），
                // 而不是让模型层的 OpStateMismatch 冒泡成 CONFLICT。
                let shifted_pitch = i16::from(current.pitch) + i16::from(*delta_pitch);
                if !(0..=127).contains(&shifted_pitch) {
                    return Err(Fault::domain_with_data(
                        ErrorCode::OutOfRange,
                        format!(
                            "平移后音高越界: {} + {} = {shifted_pitch}",
                            current.pitch, delta_pitch
                        ),
                        serde_json::json!({ "noteId": note_id.to_canonical_string(), "pitch": current.pitch, "deltaPitch": delta_pitch }),
                    ));
                }
                let shifted_tick = i128::from(current.start_tick) + i128::from(*delta_tick);
                if !(0..=i128::from(u64::MAX)).contains(&shifted_tick) {
                    return Err(Fault::domain_with_data(
                        ErrorCode::OutOfRange,
                        format!(
                            "平移后 tick 越界: {} + {} = {shifted_tick}",
                            current.start_tick, delta_tick
                        ),
                        serde_json::json!({ "noteId": note_id.to_canonical_string(), "startTick": current.start_tick, "deltaTick": delta_tick }),
                    ));
                }
                Op::MoveNote {
                    track_id: *track_id,
                    clip_id: *clip_id,
                    note_id: *note_id,
                    delta_tick: *delta_tick,
                    delta_pitch: *delta_pitch,
                }
            }
            NoteOp::Velocity { note_id, velocity } => {
                let current = project
                    .note(clip_id, note_id)
                    .map_err(|error| from_model("音符查找", &error))?;
                Op::ModifyNoteVelocity {
                    track_id: *track_id,
                    clip_id: *clip_id,
                    note_id: *note_id,
                    old_vel: current.velocity,
                    new_vel: *velocity,
                }
            }
        });
    }
    Ok(compiled)
}

/// 峰值同时发声数（在 `[start, start + duration)` 上的最大重叠）。
#[must_use]
pub fn peak_polyphony(notes: impl IntoIterator<Item = (u64, u64)>) -> usize {
    let mut events: Vec<(u64, i64)> = Vec::new();
    for (start, duration) in notes {
        events.push((start, 1));
        events.push((start.saturating_add(duration), -1));
    }
    // 同一 tick 上"结束"排在"开始"之前: 首尾相接的两个音符不算重叠。
    events.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
    let mut current = 0_i64;
    let mut peak = 0_i64;
    for (_, delta) in events {
        current += delta;
        peak = peak.max(current);
    }
    usize::try_from(peak).unwrap_or(usize::MAX)
}

/// 检查某片段在施加 `ops` **之后**的发声数（在克隆体上模拟，不改原文档）。
///
/// # Errors
///
/// 超过 [`MAX_POLYPHONY`] → `OUT_OF_RANGE`（带 `peak` / `limit` / `clipId`）。
pub fn check_polyphony(
    project: &YebanProjectV1,
    clip_id: &EntityId,
    ops: &[Op],
) -> Result<usize, Fault> {
    let mut simulated = project.clone();
    for op in ops {
        op.apply(&mut simulated)
            .map_err(|error| from_model("音符操作模拟", &error))?;
    }
    let notes = simulated
        .clip_pool
        .get(clip_id)
        .and_then(|entry| entry.content.notes())
        .map(|notes| {
            notes
                .values()
                .map(|note| (note.start_tick, note.duration_ticks))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let peak = peak_polyphony(notes);
    if peak > MAX_POLYPHONY {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("片段 {clip_id} 的发声数峰值 {peak} 超过上限 {MAX_POLYPHONY}"),
            serde_json::json!({ "clipId": clip_id.to_canonical_string(), "peak": peak, "limit": MAX_POLYPHONY }),
        ));
    }
    Ok(peak)
}

/// 缺字段的统一错误。
fn missing(field: &str, expected: &str) -> Fault {
    Fault::domain(
        ErrorCode::InvalidParameterRange,
        format!("缺少 `{field}`（期望 {expected}）"),
    )
}

/// 读 `0..=max` 的整数。
fn read_range(object: &Map<String, Value>, field: &str, min: u8, max: u8) -> Result<u8, Fault> {
    let value = object.get(field).ok_or_else(|| missing(field, "整数"))?;
    let number = value.as_i64().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是整数, 实际收到 {value}"),
        )
    })?;
    if !(i64::from(min)..=i64::from(max)).contains(&number) {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("`{field}` 越界: {number} 不在 {min}..={max}"),
            serde_json::json!({ "field": field, "value": number, "min": min, "max": max }),
        ));
    }
    Ok(u8::try_from(number).unwrap_or(max))
}

/// 读非负整数。
fn read_u64(object: &Map<String, Value>, field: &str) -> Result<u64, Fault> {
    let value = object
        .get(field)
        .ok_or_else(|| missing(field, "非负整数"))?;
    value.as_u64().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是非负整数, 实际收到 {value}"),
        )
    })
}

/// 读 `i64`。
fn read_i64(object: &Map<String, Value>, field: &str) -> Result<i64, Fault> {
    let value = object.get(field).ok_or_else(|| missing(field, "整数"))?;
    value.as_i64().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是整数, 实际收到 {value}"),
        )
    })
}

/// 读 `i8`。
fn read_i8(object: &Map<String, Value>, field: &str) -> Result<i8, Fault> {
    let number = read_i64(object, field)?;
    i8::try_from(number).map_err(|_| {
        Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("`{field}` 越界: {number} 不在 -128..=127"),
            serde_json::json!({ "field": field, "value": number }),
        )
    })
}

/// 读 `EntityId`。
fn read_id(object: &Map<String, Value>, field: &str) -> Result<EntityId, Fault> {
    let value = object
        .get(field)
        .ok_or_else(|| missing(field, "26 字符 ULID 字符串"))?;
    let text = value.as_str().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是字符串, 实际收到 {value}"),
        )
    })?;
    EntityId::from_str(text).map_err(|error| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 不是合法 ULID: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_model::samples::filled_project;

    fn lead_clip(project: &YebanProjectV1) -> (EntityId, EntityId) {
        let clip = project
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_some())
            .expect("样本里必须有 MIDI 片段");
        let track = project
            .tracks
            .values()
            .find(|track| track.kind == yeban_model::TrackKind::Midi)
            .expect("样本里必须有 MIDI 音轨");
        (track.id, clip.id)
    }

    #[test]
    fn parse_rejects_unknown_kind_and_bad_shapes() {
        for broken in [
            serde_json::json!([]),
            serde_json::json!([{"kind": "explode"}]),
            serde_json::json!([{"kind": "delete"}]),
            serde_json::json!([{"kind": "move", "noteId": "x", "deltaTick": 1, "deltaPitch": 0}]),
            serde_json::json!("not an array"),
        ] {
            let fault = parse_ops(&broken).expect_err("必须被拒");
            assert!(
                matches!(
                    fault.domain_code(),
                    Some(ErrorCode::InvalidParameterRange | ErrorCode::OutOfRange)
                ),
                "{broken} 得到了 {:?}",
                fault.domain_code()
            );
        }
    }

    #[test]
    fn parse_rejects_out_of_range_pitch_and_velocity() {
        let fault = parse_ops(&serde_json::json!([{
            "kind": "add",
            "note": {"startTick": 0, "pitch": 128, "durationTicks": 480}
        }]))
        .expect_err("音高越界");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));

        let fault = parse_ops(&serde_json::json!([{
            "kind": "velocity", "noteId": "01J8ZQ00000000000000000001", "velocity": 200
        }]))
        .expect_err("力度越界");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));

        let fault = parse_ops(&serde_json::json!([{
            "kind": "add",
            "note": {"startTick": 0, "pitch": 60, "durationTicks": 0}
        }]))
        .expect_err("零时值");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
    }

    #[test]
    fn compile_reads_the_undo_payload_from_the_document() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let note_id = project.clip_pool[&clip_id]
            .content
            .notes()
            .expect("MIDI")
            .keys()
            .next()
            .copied()
            .expect("至少一个音符");
        let ops =
            compile(&project, &track_id, &clip_id, &[NoteOp::Delete { note_id }]).expect("编译");
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            Op::DeleteNote { previous_note, .. } => {
                assert_eq!(
                    previous_note,
                    project.note(&clip_id, &note_id).expect("音符"),
                    "撤销载荷必须来自当前文档"
                );
            }
            other => panic!("应当是 DeleteNote: {other:?}"),
        }
    }

    #[test]
    fn compile_reports_missing_note_as_entity_not_found() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let ghost = deterministic_id("ghost-note");
        let fault = compile(
            &project,
            &track_id,
            &clip_id,
            &[NoteOp::Velocity {
                note_id: ghost,
                velocity: 1,
            }],
        )
        .expect_err("音符不存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::EntityNotFound));
    }

    #[test]
    fn peak_polyphony_ignores_abutting_notes() {
        assert_eq!(peak_polyphony([] as [(u64, u64); 0]), 0);
        assert_eq!(peak_polyphony([(0, 100)]), 1);
        // 首尾相接不算重叠。
        assert_eq!(peak_polyphony([(0, 100), (100, 100)]), 1);
        // 真正重叠。
        assert_eq!(peak_polyphony([(0, 200), (100, 100)]), 2);
        assert_eq!(peak_polyphony([(0, 1000), (0, 1000), (0, 1000)]), 3);
    }

    #[test]
    fn polyphony_guard_is_reachable_and_bounded() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let mut ops = Vec::new();
        for index in 0..(MAX_POLYPHONY + 1) {
            let id = deterministic_id(&format!("flood-{index}"));
            let mut note = MidiNote::new(id, 0, 60, 960);
            note.velocity = 100;
            ops.push(NoteOp::Add {
                note: Box::new(note),
            });
        }
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        let fault = check_polyphony(&project, &clip_id, &compiled).expect_err("必须越界");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
        // 样本片段在 tick 0 已经有一个音符, 而注入的音符都在 tick 0 ⇒ 峰值 = 注入数 + 1。
        // 因此去掉 2 个（33 - 2 + 1 = 32）刚好落在上限之内。
        let ok = compile(&project, &track_id, &clip_id, &ops[2..]).expect("编译");
        assert_eq!(
            check_polyphony(&project, &clip_id, &ok).expect("上限之内"),
            MAX_POLYPHONY
        );
    }
}
