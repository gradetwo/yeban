//! `yeban_edit_notes` 的 `ops` **item schema**（`tools/list` 的
//! `inputSchema.properties.ops.items`）。
//!
//! ## 为什么需要它
//!
//! 此前 `ops` 只声明 `{"type": "array"}` ⇒ 外部 Agent 只能靠一段长 description
//! 猜 op 形状与效果器参数名（#153 勘察：`insertDevice`/`eq_`/`reverb`/`SendToAux`/
//! `device.params` 在 `schemas/mcp-tools.schema.json` 里 **0 命中**）。本模块把
//! **解析器真正接受的形状**逐 `kind` 写成 `oneOf` 的一个分支。
//!
//! ## 真相从哪里来（⛔ 不是凭想象写）
//!
//! 字段名与必填性来自 `crate::domain::notes::parse_one` 的每一个 `match` 臂：
//!
//! | schema 分支 | 解析器 |
//! | :--- | :--- |
//! | `add` / `delete` / `move` / `velocity` | `notes.rs` 的同名臂 + `parse_note` |
//! | `setParam` / `setTrackMute` / `setTrackSolo` | `parse_static_lane` / `parse_track_flag_value` |
//! | `insertDevice` / `removeDevice` | `parse_device` / `read_id` |
//! | `setAutomationLane` / `removeAutomationPoint` | `parse_lane_edit` / `parse_point_removal` |
//! | 其余 | 各自的 `reject_*_fields` 白名单 |
//!
//! `kind` 的**全集**直接引用 [`crate::domain::notes::OP_KINDS`]（判据
//! `tests/effect_contract.rs::the_published_ops_item_schema_covers_every_parser_kind`
//! 双向对账），枚举词表同样引用解析器的那份常量（`LANE_NAMES` / `LANE_WRITE_MODES` /
//! `DEVICE_KINDS`）。
//!
//! ## `additionalProperties` 是**如实**的
//!
//! `parse_one` 里只有 `add` / `delete` / `move` / `velocity` / `setParam` /
//! `setAutomationLane` 六个形态**不**检查操作对象上的多余键（那些键被静默忽略）；
//! 其余形态都有 `reject_*_fields`。schema 照实写 `true` / `false`，⛔ 不把
//! "会被忽略"写成"会被拒绝"。
//!
//! ## 效果器参数
//!
//! `insertDevice.device.params.items` 引用 [`crate::effect_params::param_schema_branches`]
//! —— 名字、类型、范围、单位、缺省值全部机器可读。

use serde_json::{Value, json};

use yeban_model::music::{
    MICRO_TIMING_MAX_ABS, MIDI_PITCH_MAX, MIDI_VELOCITY_MAX, PITCH_BEND_CENTS_MAX_ABS,
    PROBABILITY_MAX, PROBABILITY_MIN, RATCHET_MAX, RATCHET_MIN,
};
use yeban_model::project::{MAX_BPM, MIN_BPM};

use crate::domain::extension_pure::{CURVE_NAMES, LANE_NAMES};
use crate::domain::notes::{DEVICE_KINDS, LANE_WRITE_MODES, OP_KINDS};
use crate::effect_params;

/// 一个 op 分支：判别键 + 必填集 + 属性 + （可选）封闭性。
fn op(kind: &str, description: &str, required: &[&str], properties: Value, closed: bool) -> Value {
    json!({
        "type": "object",
        "title": kind,
        "description": description,
        "required": required,
        "properties": properties,
        "additionalProperties": !closed,
    })
}

/// 一个 `kind` 判别属性。
fn kind(kind: &'static str) -> Value {
    json!({ "const": kind })
}

/// 一个 EntityId 属性（26 字符 ULID 文本）。
fn id(description: &str) -> Value {
    json!({
        "type": "string",
        "description": format!("{description}（26 字符 ULID）"),
        "minLength": 26,
        "maxLength": 26,
    })
}

/// 一个非负整数属性。
fn uint(description: &str) -> Value {
    json!({ "type": "integer", "minimum": 0, "description": description })
}

/// 一个非空字符串属性。
fn non_empty(description: &str) -> Value {
    json!({ "type": "string", "minLength": 1, "description": description })
}

/// 一条弯音曲线的 JSON schema（`add.note.pitchBendCurve` 与 `setNotePitchShape`
/// 的 `newPitchBendCurve` **共用**这一份；⛔ 不复制第二套边界/形状）。
fn pitch_bend_curve_schema(description: &str) -> Value {
    json!({
        "type": "array",
        "description": description,
        "items": {
            "type": "array",
            "minItems": 2,
            "maxItems": 2,
            "items": {
                "type": "integer",
                "description": "第 0 项 = tick（≥ 0），第 1 项 = cents（±1200）",
            },
        },
    })
}

/// 一个滑音对象的 JSON schema（`add.note.slide` 与 `setNotePitchShape` 的滑音字段
/// **共用**这一份；⛔ 不复制第二套边界/形状）。
fn slide_schema(description: &str) -> Value {
    json!({
        "type": "object",
        "description": description,
        "required": ["durationTicks", "targetPitch"],
        "properties": {
            "durationTicks": {
                "type": "integer",
                "minimum": 1,
                "description": "滑音时值（tick，必须非零）",
            },
            "targetPitch": {
                "type": "integer",
                "minimum": 0,
                "maximum": MIDI_PITCH_MAX,
                "description": "滑音结束时的目标音高（MIDI 0..=127）",
            },
            "curve": {
                "type": "string",
                "enum": CURVE_NAMES,
                "description": "曲线形状（可选，缺省 Linear）；只接受这四个规范名，别名/小写拼写一律拒绝",
            },
        },
        "additionalProperties": false,
    })
}

/// `add` 的 `note` 对象（`parse_note` 的字段全集，多一个键就响亮拒绝）。
fn note_object() -> Value {
    json!({
        "type": "object",
        "description": "音符载荷；`NOTE_FIELDS` 之外的键一律 `INVALID_PARAMETER_RANGE`",
        "required": ["startTick", "pitch", "durationTicks"],
        "properties": {
            "id": {
                "type": "string",
                "description": "可选；缺省由 (startTick, pitch, durationTicks) 确定性派生",
            },
            "startTick": uint("起点（tick，960 PPQ）"),
            "pitch": {
                "type": "integer",
                "minimum": 0,
                "maximum": MIDI_PITCH_MAX,
                "description": "音高（MIDI 0..=127）",
            },
            "durationTicks": {
                "type": "integer",
                "minimum": 1,
                "description": "时值（tick；0 由模型的 `ZeroDuration` 拒绝）",
            },
            "velocity": {
                "type": "integer",
                "minimum": 0,
                "maximum": MIDI_VELOCITY_MAX,
                "description": "力度（可选；缺省 `DEFAULT_VELOCITY`）",
            },
            "channel": {
                "type": "integer",
                "minimum": 0,
                "maximum": 15,
                "description": "MIDI 通道（可选，0..=15；缺省遵循所属轨道默认通道）",
            },
            "probability": {
                "type": "number",
                // ⚠ 这一对**刻意保留整数拼法**（`0` / `1`）：改成 f32 常量会让
                // `tools/list` 的字节从 `0`/`1` 变成 `0.0`/`1.0`。区间由判据
                // `expressive_schema_bounds_reconcile_with_the_model` 做**数值**对账。
                "minimum": 0,
                "maximum": 1,
                "description": "确定性概率触发（可选）",
            },
            "ratchet": {
                "type": "integer",
                "minimum": RATCHET_MIN,
                "maximum": RATCHET_MAX,
                "description": "连击细分（可选）",
            },
            "microTimingTicks": {
                "type": "integer",
                "minimum": -MICRO_TIMING_MAX_ABS,
                "maximum": MICRO_TIMING_MAX_ABS,
                "description": "起点偏移（可选，tick）",
            },
            "pitchBendCurve": pitch_bend_curve_schema(&format!(
                "弯音曲线（可选，缺省 = 无弯音）：`[tick, cents]` 二元组数组。tick 是**音符局部**坐标（0 = 音符起点）且严格升序；首点可为任意 tick ≥ 0（首点之前 cents 取 0）；cents 相对 `pitch`，闭区间 -{max}..={max}。形状/值域由模型 `MidiNote::validate` 把关（`ADR-0012` D1-D7）",
                max = PITCH_BEND_CENTS_MAX_ABS
            )),
            "slide": slide_schema("滑音（可选，缺省 = 无滑音）。`durationTicks` 与 `targetPitch` **必填**（没有目标的滑音不是滑音）；未知键响亮拒绝"),
            "syllable": {
                "type": "string",
                "description": "歌词音节（可选，字符串）",
            },
        },
        "additionalProperties": false,
    })
}

/// `device` 对象（`parse_device` 的字段全集；`params.items` 是效果器参数契约）。
fn device_object() -> Value {
    json!({
        "type": "object",
        "description": "设备定义。`kind` 必须是 `InternalEffect` 且 `bypassed:false` 才会武装引擎内置效果器；`params` 的已识别名字见 items（名字大小写不敏感）。⚠ 拼错的 `name`（与某个已识别拼写的编辑距离 ≤ 2，例如 `eq_low_gian`）会被**响亮拒绝**（`INVALID_PARAMETER_RANGE` / `unknownEffectParam`，带最近邻建议）；距离更远的未识别名字仍会被存储但**不生效**，并在提案响应的 `warnings[]` 与读回的 `inserts[].unrecognisedParams` 里出现",
        "required": [
            "deviceId",
            "name",
            "kind",
            "bypassed",
            "latencySamples",
            "params",
        ],
        "properties": {
            "deviceId": id("设备身份（设备链上唯一）"),
            "name": non_empty("设备显示名"),
            "kind": {
                "type": "string",
                "enum": DEVICE_KINDS,
                "description": "设备类型（模型 serde 的变体名）",
            },
            "bypassed": {
                "type": "boolean",
                "description": "旁通开关；旁通设备不计入 PDC、也不武装效果器",
            },
            "latencySamples": {
                "type": "integer",
                "minimum": 0,
                "maximum": 4_294_967_295_u64,
                "description": "设备延迟（样本，PDC 的唯一来源）",
            },
            "params": {
                "type": "array",
                "description": "效果器参数。每个元素 {name, value, unit?}；`name` 的已识别拼写、单位、值域与缺省值见 items（0 个元素 = 不武装）。⚠ 拼错的 `name` 在写入时被拒（编辑距离 ≤ 2，带最近邻）；其余的未知名字被存储但不生效、并在 `warnings[]` / `unrecognisedParams` 里可见",
                "items": {
                    "oneOf": effect_params::param_schema_branches(),
                },
            },
        },
        "additionalProperties": false,
    })
}

/// `setAutomationLane` 的 `lane` 对象。
fn lane_object() -> Value {
    json!({
        "type": "object",
        "description": "自包含的泳道寻址 + 属性覆盖（`remove:true` 与属性键互斥）",
        "required": ["lane"],
        "properties": {
            "lane": {
                "type": "string",
                "enum": LANE_NAMES,
                "description": "自动化目标变体名（与 `yeban_edit_automation` 同一份词表）",
            },
            "edgeId": id("SendGain 的发送边身份（该目标必填）"),
            "slotIndex": uint("DeviceParam 的设备链插槽下标"),
            "paramIndex": uint("DeviceParam 的参数下标"),
            "macroIndex": uint("Macro 的宏下标"),
            "readEnabled": {
                "type": "boolean",
                "description": "读开关（可选；缺省 = 保留现值）",
            },
            "writeMode": {
                "type": "string",
                "enum": LANE_WRITE_MODES,
                "description": "写模式（可选；缺省 = 保留现值）",
            },
            "domain": {
                "description": "取值域覆盖：{min,max} = 设显式覆盖，null = 清掉覆盖，缺省 = 保留现值",
                "oneOf": [
                    { "type": "null" },
                    {
                        "type": "object",
                        "required": ["min", "max"],
                        "properties": {
                            "min": { "type": "number" },
                            "max": { "type": "number" },
                        },
                        "additionalProperties": false,
                    },
                ],
            },
            "remove": {
                "type": "boolean",
                "description": "true = 取走这条泳道（与属性键同给是 `removeTakesNoProperties`）",
            },
        },
        "additionalProperties": false,
    })
}

/// `removeAutomationPoint` 的 `point` 对象（泳道寻址 + `tick`/`pointId` 二选一）。
fn point_object() -> Value {
    json!({
        "type": "object",
        "description": "泳道寻址 + 点寻址；`tick` 与 `pointId` **恰好给一个**",
        "required": ["lane"],
        "properties": {
            "lane": { "type": "string", "enum": LANE_NAMES },
            "edgeId": id("SendGain 的发送边身份（该目标必填）"),
            "slotIndex": uint("DeviceParam 的设备链插槽下标"),
            "paramIndex": uint("DeviceParam 的参数下标"),
            "macroIndex": uint("Macro 的宏下标"),
            "tick": uint("点的时间位置（与 `pointId` 恰好给一个）"),
            "pointId": id("点的显式身份（与 `tick` 恰好给一个）"),
        },
        "additionalProperties": false,
        // 两个 `required` 分支互斥：都不给 / 都给都匹配不上 `oneOf` ⇒ 与解析器一致。
        "oneOf": [
            { "required": ["tick"] },
            { "required": ["pointId"] },
        ],
    })
}

/// `setSection` 的 `section` 载荷。
fn section_object() -> Value {
    json!({
        "type": "object",
        "description": "自包含的段落载荷（`create` 显式区分新建/更新；属性按合并语义施加）",
        "required": ["sectionId"],
        "properties": {
            "sectionId": id("段落身份"),
            "create": {
                "type": "boolean",
                "description": "true = 新建（`name` 必填），false/缺省 = 更新",
            },
            "name": non_empty("段落名（`create:true` 时必填）"),
            "startTick": uint("区间起点（tick）"),
            "endTick": uint("区间终点（tick；模型要求 > startTick）"),
            "color": {
                "type": ["string", "null"],
                "description": "非空字符串 = 覆盖；null = 清空",
            },
        },
        "additionalProperties": false,
    })
}

/// `setScene` 的 `scene` 载荷。
fn scene_object() -> Value {
    json!({
        "type": "object",
        "description": "自包含的场景载荷（`create` 显式区分新建/更新；属性按合并语义施加）",
        "required": ["sceneId"],
        "properties": {
            "sceneId": id("场景身份"),
            "create": {
                "type": "boolean",
                "description": "true = 新建（`name` 必填），false/缺省 = 更新",
            },
            "name": non_empty("场景名（`create:true` 时必填）"),
            "tempo": {
                "type": ["number", "null"],
                "description": format!("速度覆盖（BPM {MIN_BPM}..={MAX_BPM}）；null = 清空 = 跟随工程速度"),
            },
            "color": {
                "type": ["string", "null"],
                "description": "非空字符串 = 覆盖；null = 清空",
            },
        },
        "additionalProperties": false,
    })
}

/// `ops` 数组的 item schema：`OP_KINDS` 里的每个 kind 一个分支。
#[must_use]
pub fn ops_items() -> Value {
    let mut items = json!({
        "oneOf": [
            op(
                "add",
                "在目标片段里插入一个音符。操作对象上的多余键被**静默忽略**；`note` 里的未知键会被响亮拒绝",
                &["kind", "note"],
                json!({ "kind": kind("add"), "note": note_object() }),
                false,
            ),
            op(
                "delete",
                "按键身份删除一个音符；多余键被静默忽略",
                &["kind", "noteId"],
                json!({
                    "kind": kind("delete"),
                    "noteId": id("要删除的音符身份"),
                }),
                false,
            ),
            op(
                "move",
                "平移一个音符（`deltaTick` / `deltaPitch` 是增量）；多余键被静默忽略",
                &["kind", "noteId", "deltaTick", "deltaPitch"],
                json!({
                    "kind": kind("move"),
                    "noteId": id("要平移的音符身份"),
                    "deltaTick": { "type": "integer", "description": "tick 增量（可为负）" },
                    "deltaPitch": {
                        "type": "integer",
                        "minimum": -128,
                        "maximum": 127,
                        "description": "半音增量（i8）",
                    },
                }),
                false,
            ),
            op(
                "velocity",
                "改一个音符的力度；多余键被静默忽略",
                &["kind", "noteId", "velocity"],
                json!({
                    "kind": kind("velocity"),
                    "noteId": id("目标音符身份"),
                    "velocity": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": MIDI_VELOCITY_MAX,
                    },
                }),
                false,
            ),
            op(
                "setNoteTiming",
                "把音符的起点/时值设为绝对值（不改音高、力度与表现力字段）；两个旧值由服务端从当前文档读取，操作对象上的多余键被响亮拒绝",
                &["kind", "noteId", "startTick", "durationTicks"],
                json!({
                    "kind": kind("setNoteTiming"),
                    "noteId": id("目标音符身份"),
                    "startTick": uint("新起点（tick，960 PPQ）"),
                    "durationTicks": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "新时值（tick；0 由模型的 `ZeroDuration` 拒绝）",
                    },
                }),
                true,
            ),
            op(
                "setNoteExpression",
                "整体替换一个已有音符的三个表现力字段（触发概率 / 连击 / 微时序）。⚠ 三个键**全部必填**（缺键 ⛔ 不等于 `null`）：`null` = 清空 = 模型的 `None`（必然触发 / 等价于 1 / 无偏移）。三个旧值由服务端从**当前文档**读取（⛔ 不信调用方），操作对象上的多余键被响亮拒绝",
                &[
                    "kind",
                    "noteId",
                    "probability",
                    "ratchet",
                    "microTimingTicks",
                ],
                json!({
                    "kind": kind("setNoteExpression"),
                    "noteId": id("目标音符身份"),
                    "probability": {
                        "type": ["number", "null"],
                        "minimum": PROBABILITY_MIN,
                        "maximum": PROBABILITY_MAX,
                        "description": "触发概率 0.0..=1.0 且有限；null = 清空 = 必然触发",
                    },
                    "ratchet": {
                        "type": ["integer", "null"],
                        "minimum": RATCHET_MIN,
                        "maximum": RATCHET_MAX,
                        "description": "连击次数 1..=16；null = 清空 = 等价于 1",
                    },
                    "microTimingTicks": {
                        "type": ["integer", "null"],
                        "minimum": -MICRO_TIMING_MAX_ABS,
                        "maximum": MICRO_TIMING_MAX_ABS,
                        "description": "起点微偏移 -240..=240 tick；null = 清空 = 无偏移",
                    },
                }),
                true,
            ),
            op(
                "setNotePitchShape",
                "整体替换一个已有音符的**音高形状**（弯音曲线 ＋ 滑音）。⚠ 载荷**只带新值**：`newPitchBendCurve` 与 `newSlide` **全部必填**；两个旧值由服务端从**当前文档**读（与 `setNoteExpression` 同一条纪律），把 `oldPitchBendCurve` / `oldSlide` 写进来是**未知键**、响亮拒绝。弯音\"无\"**只有**空数组一种拼法（`newPitchBendCurve: []`，⛔ 无 `null`），滑音用 `newSlide: null` 清空；两者同时存在时**相加**（`ADR-0012` D9）。操作对象上的多余键被响亮拒绝",
                &[
                    "kind",
                    "noteId",
                    "newPitchBendCurve",
                    "newSlide",
                ],
                json!({
                    "kind": kind("setNotePitchShape"),
                    "noteId": id("目标音符身份"),
                    "newPitchBendCurve": pitch_bend_curve_schema(
                        "修改后的弯音曲线（空数组 = 清空 = 无弯音；数值边界由模型 `MidiNote::validate` 把关）"
                    ),
                    "newSlide": {
                        "description": "修改后的滑音（null = 清空 = 无滑音）",
                        "oneOf": [{ "type": "null" }, slide_schema("修改后的滑音对象")],
                    },
                }),
                true,
            ),
            op(
                "insertDevice",
                "在目标音轨的设备链里插入一台设备。要得到 EQ / 混响 / 卷积混响：`device.kind:\"InternalEffect\"` + `bypassed:false` + 在 `device.params` 里写 `eq_*` / `reverb_*` / `conv_*` 名字（名字、单位、值域、缺省值见 `device.params.items`）",
                &["kind", "trackId", "device"],
                json!({
                    "kind": kind("insertDevice"),
                    "trackId": id("目标音轨身份"),
                    "slotIndex": uint("插槽下标（可选；缺省 = 追加到链尾；> 链长则 OUT_OF_RANGE）"),
                    "device": device_object(),
                }),
                true,
            ),
            op(
                "removeDevice",
                "按身份从目标音轨的设备链里取走一台设备",
                &["kind", "trackId", "deviceId"],
                json!({
                    "kind": kind("removeDevice"),
                    "trackId": id("目标音轨身份"),
                    "deviceId": id("要取走的设备身份"),
                }),
                true,
            ),
            op(
                "setParam",
                "写**静态**参数：`lane:\"TrackVolume\"`（dB，有限值）/ `lane:\"TrackPan\"`（-1.0..=1.0）/ `lane:\"DeviceParam\"`（设备参数，另需 `slotIndex` + `paramIndex`）。目标音轨是工具顶层的 `trackId`；设备参数的 `value` 必须是离线渲染器**真的消费**的名字所指的那个参数（名字不在白名单 ⇒ `paramNotSupported`，⛔ 不静默写入）；多余键被静默忽略",
                &["kind", "lane", "value"],
                json!({
                    "kind": kind("setParam"),
                    "lane": {
                        "type": "string",
                        "enum": ["TrackVolume", "TrackPan", "DeviceParam"],
                        "description": "只认这三个规范名（SendGain/Macro 与未知名字都响亮拒绝）",
                    },
                    "slotIndex": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "仅 `lane:\"DeviceParam\"`：`track.devices` 的槽位下标（必填，越界 ⇒ `slotOutOfRange`）",
                    },
                    "paramIndex": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "仅 `lane:\"DeviceParam\"`：该设备 `params` 的参数下标（必填，越界 ⇒ `paramIndexOutOfRange`）",
                    },
                    "value": {
                        "type": "number",
                        "description": "TrackVolume 单位 dB（有限值）；TrackPan 单位 1.0 = 右、-1.0 = 左；DeviceParam 是目标参数的绝对值",
                    },
                }),
                false,
            ),
            op(
                "setTrackMute",
                "写目标音轨的 `mute` 开关（目标 = 顶层 `trackId`）",
                &["kind", "value"],
                json!({
                    "kind": kind("setTrackMute"),
                    "value": { "type": "boolean", "description": "目标态（不做真假值强转）" },
                }),
                true,
            ),
            op(
                "setTrackSolo",
                "写目标音轨的 `solo` 开关（目标 = 顶层 `trackId`）",
                &["kind", "value"],
                json!({
                    "kind": kind("setTrackSolo"),
                    "value": { "type": "boolean", "description": "目标态（不做真假值强转）" },
                }),
                true,
            ),
            op(
                "setTransportConfig",
                "整体替换走带配置（三个字段全部必填）",
                &["kind", "metronomeEnabled", "countInBars", "launchQuantization"],
                json!({
                    "kind": kind("setTransportConfig"),
                    "metronomeEnabled": { "type": "boolean" },
                    "countInBars": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 255,
                        "description": "预备小节数",
                    },
                    "launchQuantization": {
                        "type": "string",
                        "enum": ["Off", "Bar", "TwoBars", "FourBars", "EightBars"],
                        "description": "启动量化（模型 serde 名）",
                    },
                }),
                true,
            ),
            op(
                "setTempo",
                &format!("写工程速度（BPM {MIN_BPM:?}..={MAX_BPM:?}）"),
                &["kind", "bpm"],
                json!({
                    "kind": kind("setTempo"),
                    "bpm": {
                        "type": "number",
                        "minimum": MIN_BPM,
                        "maximum": MAX_BPM,
                        "description": "新速度（BPM；模型 `BpmOutOfRange` 判）",
                    },
                }),
                true,
            ),
            op(
                "setClipPlacementDuration",
                "改一条已有摆放的长度（必须 ≥ 1）",
                &["kind", "trackId", "placementId", "durationTicks"],
                json!({
                    "kind": kind("setClipPlacementDuration"),
                    "trackId": id("摆放所在的音轨身份"),
                    "placementId": id("摆放身份"),
                    "durationTicks": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "新长度（tick；0 由模型 `ZeroDuration` 拒绝）",
                    },
                }),
                true,
            ),
            op(
                "setTrackColor",
                "写一条音轨的色标（`null` = 清空回主题默认色）",
                &["kind", "trackId", "color"],
                json!({
                    "kind": kind("setTrackColor"),
                    "trackId": id("目标音轨身份"),
                    "color": {
                        "type": ["string", "null"],
                        "description": "非空字符串 = 设值；null = 清除（缺键 ≠ 清除）",
                    },
                }),
                true,
            ),
            op(
                "setTrackSampler",
                "给一条音轨绑定采样器定义文件（`null` = 解绑）。⚠ 这是**模型里早就有、工具面此前不可达**的那条路（`Op::SetTrackSampler` ⇒ `TrackV3.sampler`）；`yeban_render_master` 会**真的**用它发声（与 app 的 `--track-sampler` 同一条音源）",
                &["kind", "trackId", "sampler"],
                json!({
                    "kind": kind("setTrackSampler"),
                    "trackId": id("目标音轨身份"),
                    "sampler": {
                        "type": ["string", "null"],
                        "minLength": 1,
                        "description": "非空字符串 = SFZ 定义文件路径（原样留痕，不查存在性）；null = 解绑（缺键 ≠ 解绑）",
                    },
                }),
                true,
            ),
            op(
                "setClipPlacementMuted",
                "切换一条摆放的静音",
                &["kind", "trackId", "placementId", "muted"],
                json!({
                    "kind": kind("setClipPlacementMuted"),
                    "trackId": id("摆放所在的音轨身份"),
                    "placementId": id("摆放身份"),
                    "muted": { "type": "boolean" },
                }),
                true,
            ),
            op(
                "setClipPlacementLoop",
                "整体替换一条摆放的循环配置（`LoopConfig` 三项全部必填；旧值由服务端从当前文档读，载荷只带新值）",
                &[
                    "kind",
                    "trackId",
                    "placementId",
                    "loopEnabled",
                    "loopStartTick",
                    "loopEndTick",
                ],
                json!({
                    "kind": kind("setClipPlacementLoop"),
                    "trackId": id("摆放所在的音轨身份"),
                    "placementId": id("摆放身份"),
                    "loopEnabled": {
                        "type": "boolean",
                        "description": "是否启用循环（整体替换的一部分，必填）",
                    },
                    "loopStartTick": uint("循环起点（摆放相对 tick，必填）"),
                    "loopEndTick": uint("循环终点（摆放相对 tick，不含；必填）"),
                }),
                true,
            ),
            op(
                "setDeviceBypassed",
                "切换一台设备的旁通（旁通设备不计入 PDC、也不武装效果器）",
                &["kind", "trackId", "deviceId", "bypassed"],
                json!({
                    "kind": kind("setDeviceBypassed"),
                    "trackId": id("设备所在的音轨身份"),
                    "deviceId": id("目标设备身份"),
                    "bypassed": { "type": "boolean" },
                }),
                true,
            ),
            op(
                "setDeviceName",
                "改一台设备的显示名（身份 `deviceId` 不变）",
                &["kind", "trackId", "deviceId", "name"],
                json!({
                    "kind": kind("setDeviceName"),
                    "trackId": id("设备所在的音轨身份"),
                    "deviceId": id("目标设备身份"),
                    "name": { "type": "string", "description": "新显示名（可空串）" },
                }),
                true,
            ),
            op(
                "setClipName",
                "改片段池条目的显示名（身份 `clipId` 不变）",
                &["kind", "clipId", "name"],
                json!({
                    "kind": kind("setClipName"),
                    "clipId": id("目标片段身份"),
                    "name": { "type": "string", "description": "新显示名（可空串）" },
                }),
                true,
            ),
            op(
                "setClipGain",
                "改音频片段的增益（目标 `clipId` 在操作对象里，⛔ 不读顶层 `clipId`；MIDI 片段会被拒）",
                &["kind", "clipId", "gainDb"],
                json!({
                    "kind": kind("setClipGain"),
                    "clipId": id("目标音频片段身份（必须存在且内容是 Audio）"),
                    "gainDb": {
                        "type": "number",
                        "description": "新增益 (dB)；任意有限值（模型没有 dB 范围），收窄到 f32 后非有限则拒",
                    },
                }),
                true,
            ),
            op(
                "setTrackName",
                "改音轨显示名（身份 `trackId` 不变；空串 = 显式清空）",
                &["kind", "trackId", "name"],
                json!({
                    "kind": kind("setTrackName"),
                    "trackId": id("目标音轨身份"),
                    "name": { "type": "string", "description": "新名字（可空串）" },
                }),
                true,
            ),
            op(
                "setAuthor",
                "写工程作者署名（模型没有空串校验）",
                &["kind", "author"],
                json!({
                    "kind": kind("setAuthor"),
                    "author": { "type": "string", "description": "新署名" },
                }),
                true,
            ),
            op(
                "setTitle",
                "写工程标题（`title` 是必需顶层字段；模型没有空串/长度校验）",
                &["kind", "title"],
                json!({
                    "kind": kind("setTitle"),
                    "title": { "type": "string", "description": "新标题（可空串）" },
                }),
                true,
            ),
            op(
                "setTimeSignature",
                "写工程拍号（顶层 `time_signature`；同一份 `\"N/D\"` 文本与 `yeban_open_project` 的 `timeSignature` 逐字同词同义，值域由模型 `TimeSignature::validate` 判）",
                &["kind", "timeSignature"],
                json!({
                    "kind": kind("setTimeSignature"),
                    "timeSignature": {
                        "type": "string",
                        "description": "新拍号 \"N/D\" 文本（例 \"3/4\"、\"6/8\"）。形状错误 ⇒ `INVALID_PARAMETER_RANGE`；取值越界 ⇒ `OUT_OF_RANGE`。⛔ 这里不复制值域表：判定它的唯一入口是模型 `TimeSignature::validate`",
                    },
                }),
                true,
            ),
            op(
                "setKeySignature",
                "写工程调号（顶层 `key_signature`；`null` 表示未设定/清空调号，对象包含 `sharpsFlats` 与 `isMinor`，值域由模型 `KeySignature::validate` 判）",
                &["kind", "keySignature"],
                json!({
                    "kind": kind("setKeySignature"),
                    "keySignature": {
                        "type": ["object", "null"],
                        "description": "新调号对象或 null（清空调号）。`sharpsFlats` 为升降号数量（-7..=7，负数为降号，正数为升号，0 为 C 大调 / A 小调），`isMinor` 为是否小调（false = 大调, true = 小调）",
                        "properties": {
                            "sharpsFlats": {
                                "type": "integer",
                                "minimum": -7,
                                "maximum": 7,
                                "description": "升降号数量（-7 到 +7：负数为降号，正数为升号，0 为 C 大调 / A 小调）",
                            },
                            "isMinor": {
                                "type": "boolean",
                                "description": "是否为小调（false = 大调 Major, true = 小调 Minor）",
                            },
                        },
                        "required": ["sharpsFlats", "isMinor"],
                        "additionalProperties": false,
                    },
                }),
                true,
            ),
            op(
                "setTempoPoint",
                "写 `tempo_map` 里一个速度映射点（`tick` ≥ 1；`bpm` 为 `null` = 删除该点，⛔ 缺键不是删除；旧值由服务端从当前文档读）",
                &["kind", "tick", "bpm"],
                json!({
                    "kind": kind("setTempoPoint"),
                    "tick": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "映射点 tick（≥ 1）。tick 0 属于工程速度本身，归 `setTempo` ⇒ `tempoMapZeroTick`",
                    },
                    "bpm": {
                        "type": ["number", "null"],
                        "minimum": 20.0,
                        "maximum": 999.0,
                        "description": "新 BPM（20.0..=999.0，越界 ⇒ `bpmOutOfRange`）或 `null` = 删除该映射点（点不存在 ⇒ `tempoPointNotFound`）",
                    },
                }),
                true,
            ),
            op(
                "setTimeSignaturePoint",
                "写 `time_signature_map` 里一个拍号映射点（`tick` ≥ 1；`timeSignature` 为 `null` = 删除该点；`\"N/D\"` 文本与 `setTimeSignature` 逐字同词同义，值域由模型 `TimeSignature::validate` 判）",
                &["kind", "tick", "timeSignature"],
                json!({
                    "kind": kind("setTimeSignaturePoint"),
                    "tick": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "映射点 tick（≥ 1）。tick 0 的拍号是顶层 `project.time_signature`，归 `setTimeSignature` ⇒ `tempoMapZeroTick`",
                    },
                    "timeSignature": {
                        "type": ["string", "null"],
                        "description": "新拍号 \"N/D\" 文本（例 \"3/4\"、\"6/8\"）或 `null` = 删除该映射点（点不存在 ⇒ `timeSignaturePointNotFound`）。形状错误 ⇒ `INVALID_PARAMETER_RANGE`；取值越界 ⇒ `OUT_OF_RANGE`。⛔ 这里不复制值域表：判定它的唯一入口是模型 `TimeSignature::validate`",
                    },
                }),
                true,
            ),
            op(
                "setAutomationLane",
                "写一条自动化泳道自己的属性，或 `remove:true` 取走它（目标音轨 = 顶层 `trackId`）",
                &["kind", "lane"],
                json!({
                    "kind": kind("setAutomationLane"),
                    "lane": lane_object(),
                }),
                false,
            ),
            op(
                "removeAutomationPoint",
                "取走泳道里的一个自动化点（`point` 里 `tick` 或 `pointId` 恰好一个）",
                &["kind", "point"],
                json!({
                    "kind": kind("removeAutomationPoint"),
                    "point": point_object(),
                }),
                true,
            ),
            op(
                "removeClip",
                "取走顶层 `clipId` 那个片段池条目（还有摆放引用它则 CONFLICT）；载荷为空",
                &["kind"],
                json!({ "kind": kind("removeClip") }),
                true,
            ),
            op(
                "removeAsset",
                "取走一条资产索引项（自带 `assetHash`；被片段引用则拒）",
                &["kind", "assetHash"],
                json!({
                    "kind": kind("removeAsset"),
                    "assetHash": {
                        "type": "string",
                        "description": "资产内容哈希（64 位 SHA-256 十六进制）",
                        "minLength": 64,
                        "maxLength": 64,
                    },
                }),
                true,
            ),
            op(
                "setAsset",
                "改一条资产索引项的许可 / 原路径（至少给一个可改字段）",
                &["kind", "assetHash"],
                json!({
                    "kind": kind("setAsset"),
                    "assetHash": {
                        "type": "string",
                        "description": "资产内容哈希（64 位 SHA-256 十六进制）",
                        "minLength": 64,
                        "maxLength": 64,
                    },
                    "license": {
                        "type": "string",
                        "minLength": 1,
                        "description": "许可标识（可选，但不能是空串）",
                    },
                    "originalPath": {
                        "type": "string",
                        "minLength": 1,
                        "description": "原路径（可选，但不能是空串）",
                    },
                }),
                true,
            ),
            op(
                "removeTrack",
                "取走一条音轨本身（连带它的摆放与自动化泳道；主总线被拒）",
                &["kind", "trackId"],
                json!({
                    "kind": kind("removeTrack"),
                    "trackId": id("要取走的音轨身份"),
                }),
                true,
            ),
            op(
                "setRoutingGain",
                "写一条路由边的静态增益（`value:null` = 单位增益，与 0 dB 可区分）",
                &["kind", "edgeId", "value"],
                json!({
                    "kind": kind("setRoutingGain"),
                    "edgeId": id("路由边身份"),
                    "value": {
                        "type": ["number", "null"],
                        "description": "目标增益（dB）；null = 单位增益（必填，缺键不是 null）",
                    },
                }),
                true,
            ),
            op(
                "connectRouting",
                "建立一条路由边（两端必须已在 `routing_graph.nodes` 里）。要建一条**发送**：`edgeKind:\"SendToAux\"`，`sourceNode` = 音轨身份，`destinationNode` = 辅助返回轨身份",
                &["kind", "edgeId", "sourceNode", "destinationNode", "edgeKind"],
                json!({
                    "kind": kind("connectRouting"),
                    "edgeId": id("新边身份"),
                    "sourceNode": id("源节点身份（必须已在 routing_graph.nodes 里）"),
                    "destinationNode": id("目标节点身份（必须已在 routing_graph.nodes 里）"),
                    "edgeKind": {
                        "type": "string",
                        "enum": ["TrackToBus", "BusToMaster", "SendToAux", "Sidechain"],
                        "description": "路由类型（模型 serde 名）；发送 = SendToAux",
                    },
                    "gainDb": {
                        "type": ["number", "null"],
                        "description": "可选；省略或 null = 单位增益，数字 = 该边增益（dB，必须有限）",
                    },
                }),
                true,
            ),
            op(
                "disconnectRouting",
                "断开（取走）一条路由边；载荷为空",
                &["kind", "edgeId"],
                json!({
                    "kind": kind("disconnectRouting"),
                    "edgeId": id("要断开的路由边身份"),
                }),
                true,
            ),
            op(
                "addTrack",
                "新增一条空形状音轨（不自动进 routing_graph.nodes、也不建边）",
                &["kind", "trackId", "name", "trackKind"],
                json!({
                    "kind": kind("addTrack"),
                    "trackId": id("新音轨身份"),
                    "name": non_empty("音轨显示名"),
                    "trackKind": {
                        "type": "string",
                        "enum": ["Midi", "Audio", "AuxReturn", "Master"],
                        "description": "音轨类型（模型 serde 名）",
                    },
                }),
                true,
            ),
            op(
                "addRoutingNode",
                "把一个身份加进 `routing_graph.nodes`（不要求它是一条真实音轨）",
                &["kind", "nodeId"],
                json!({
                    "kind": kind("addRoutingNode"),
                    "nodeId": id("节点身份"),
                }),
                true,
            ),
            op(
                "removeRoutingNode",
                "从 `routing_graph.nodes` 取走一个节点（没有任何边引用它；主总线被拒）",
                &["kind", "nodeId"],
                json!({
                    "kind": kind("removeRoutingNode"),
                    "nodeId": id("要取走的节点身份"),
                }),
                true,
            ),
            op(
                "removeSection",
                "取走一个曲式段落",
                &["kind", "sectionId"],
                json!({
                    "kind": kind("removeSection"),
                    "sectionId": id("要取走的段落身份"),
                }),
                true,
            ),
            op(
                "removeScene",
                "取走一个场景",
                &["kind", "sceneId"],
                json!({
                    "kind": kind("removeScene"),
                    "sceneId": id("要取走的场景身份"),
                }),
                true,
            ),
            op(
                "setSection",
                "写入一个曲式段落（`create:true` 新建，缺省更新；属性按合并语义施加）",
                &["kind", "section"],
                json!({
                    "kind": kind("setSection"),
                    "section": section_object(),
                }),
                true,
            ),
            op(
                "setScene",
                "写入一个场景（`create:true` 新建，缺省更新；属性按合并语义施加）",
                &["kind", "scene"],
                json!({
                    "kind": kind("setScene"),
                    "scene": scene_object(),
                }),
                true,
            ),
        ],
        "description": format!(
            "逐 kind 的 op 形状（`OP_KINDS` 全集，共 {} 个）。要写 EQ / 混响 / 卷积混响见 `insertDevice` 分支的 `device.params.items`；要建发送见 `connectRouting` 的 `edgeKind:\"SendToAux\"`。⭐ 顶层 `clipId` 的逐 kind 口径见根上的 `x-envelope` 与每个分支的 `x-envelope.clipId`。",
            OP_KINDS.len(),
        ),
    });
    publish_envelope_clip(&mut items);
    items
}

/// ⭐ `#177` 坑④：把"顶层 `clipId` 在**这个** kind 上是否必需"逐分支广播出去。
///
/// 真相只有一份（[`crate::domain::notes::ENVELOPE_CLIP_KINDS`]，由
/// `NoteOp::reads_envelope_clip` 派生）；本函数只负责把它**写成可读的形状**：
///
/// - 每个 `oneOf` 分支的描述末尾多一句（人读）；
/// - 每个分支多一个 `x-envelope.clipId`（机器读：`"required"` / `"ignored"`）；
/// - 根上多一个 `x-envelope`（两个 kind 清单 + 两条"不在 `ops` 里"的附加条件）。
///
/// ⚠ 用 `x-` 前缀：JSON Schema 允许未知关键字（它们是注解）。分支里**无法**表达
/// "顶层实参"——这正是本票选"顶层条件必需 ＋ 运行期校验"而不是 `if/then` 的原因
/// （`if/then` 的 `if` 只能看 `ops`，而 `required` 在顶层 ⇒ 两者不在同一层）。
fn publish_envelope_clip(items: &mut Value) {
    use crate::domain::notes::{ENVELOPE_CLIP_KINDS, OP_KINDS};

    if let Some(branches) = items.get_mut("oneOf").and_then(Value::as_array_mut) {
        for branch in branches {
            let Some(kind) = branch
                .get("properties")
                .and_then(|properties| properties.get("kind"))
                .and_then(|kind| kind.get("const"))
                .and_then(Value::as_str)
                .map(str::to_owned)
            else {
                continue;
            };
            let required = ENVELOPE_CLIP_KINDS.contains(&kind.as_str());
            let note = if required {
                "⭐ 顶层 `clipId`: **必需**（本 kind 读它；缺 ⇒ `INVALID_PARAMETER_RANGE` / `clipIdRequired`）"
            } else {
                "⭐ 顶层 `clipId`: **不读**（省略 = 占位；给了 = 接受但忽略，不解析、不查片段池）"
            };
            let Value::Object(map) = branch else {
                continue;
            };
            let description = map
                .get("description")
                .and_then(Value::as_str)
                .map_or_else(String::new, |text| format!("{text} {note}"));
            map.insert("description".to_owned(), Value::from(description));
            map.insert(
                "x-envelope".to_owned(),
                json!({ "clipId": if required { "required" } else { "ignored" } }),
            );
        }
    }
    let ignored: Vec<&str> = OP_KINDS
        .iter()
        .copied()
        .filter(|kind| !ENVELOPE_CLIP_KINDS.contains(kind))
        .collect();
    items["x-envelope"] = json!({
        "clipIdRequiredKinds": ENVELOPE_CLIP_KINDS,
        "clipIdIgnoredKinds": ignored,
        "alsoRequiredWhen": {
            "placement": "任何 `placement` 在场（摆放编辑要核对它引用的片段）",
            "create": "`create:true`（顶层 `clipId` 是**将要新建的**片段身份）",
        },
        "runtimeCheck": "需要而缺 ⇒ `INVALID_PARAMETER_RANGE`（reason `clipIdRequired`）；不读它的 kind 给了 = 接受但忽略",
    });
}
