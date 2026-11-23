//! OTCP 音高校正 GUI（Hub #711）——提交 / 轮询 / 音符表 / 交付四个面。
//!
//! 入口：菜单「编辑 → 音高校正 (OTCP)...」（`app.slint` 的 `open-otcp` 回调）。
//! 语义与 MCP `yeban_correct_pitch` / `yeban_correct_pitch_job` /
//! `yeban_correct_pitch_notes` 共用 `yeban_mcp::domain::otcp` 的同一组函数：
//! 参数值域、错误原文、收据投影都在那边，本模块只做 Slint 属性与文本拼装。
//!
//! 交付走**直接提交**（`UndoPort::commit_ops`）而不是 MCP 提案：GUI 没有提案审阅
//! 面，用户按下「交付到当前工程」就是明确同意；收据的 `delivered_proposal_id`
//! 记 `gui-direct` 哨兵，后续轮询显示 `alreadyDelivered`（见 RECEIPT 的差异登记）。

use std::rc::Rc;
use std::str::FromStr as _;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use yeban_mcp::domain::error::Fault;
use yeban_mcp::domain::otcp::{self, OtcpDeliveryParts};
use yeban_mcp::tools::ErrorCode;
use yeban_model::{AssetHash, ClipContent, EntityId, YebanProjectV1};

use crate::host;
use crate::ui::MainWindow;
use crate::undo::UndoPort;

/// `Fault` 的人话：领域失败原样用 `message`（与 MCP 回执逐字一致）。
fn fault_text(fault: &Fault) -> String {
    match fault {
        Fault::Domain { message, .. } => message.clone(),
        Fault::Impl { error } => format!("{error:?}"),
    }
}

fn param_error(message: impl Into<String>) -> Fault {
    Fault::domain(ErrorCode::InvalidParameterRange, message)
}

fn selected_audio_clip(
    ui: &MainWindow,
    project: &YebanProjectV1,
) -> Result<(EntityId, EntityId, String), String> {
    let text = ui.get_selected_clip_ulid();
    let text = text.trim();
    if text.is_empty() {
        return Err("先在编曲视图里选一个音频片段".to_owned());
    }
    let clip_id = EntityId::from_str(text)
        .map_err(|error| format!("选中的片段 id 不是合法 ULID（{text}）：{error}"))?;
    let clip = project
        .clip_pool
        .get(&clip_id)
        .ok_or_else(|| format!("片段 {clip_id} 已不在工程里"))?;
    if !matches!(clip.content, ClipContent::Audio { .. }) {
        return Err(format!(
            "片段 {clip_id} 不是音频片段（OTCP 校正只接音频片段）"
        ));
    }
    for track in project.tracks.values() {
        if track
            .clips
            .values()
            .any(|placement| placement.clip_id == clip_id)
        {
            return Ok((track.id, clip_id, format!("{} · {}", track.name, clip.name)));
        }
    }
    Err(format!("片段 {clip_id} 没有摆在任何音轨上"))
}

fn parse_key_root(text: &str) -> Result<u8, Fault> {
    let text = text.trim();
    if text.is_empty() {
        return Err(param_error("`keyRoot` 必须是 0–11 的整数（C=0）"));
    }
    let value = text
        .parse::<u64>()
        .map_err(|_| param_error("参数 `keyRoot` 类型非法, 期望 `integer`"))?;
    otcp::validate_key_root(value)
}

fn parse_tuning(text: &str) -> Result<f32, Fault> {
    let text = text.trim();
    if text.is_empty() {
        return otcp::validate_tuning_hz(440.0);
    }
    let value = text
        .parse::<f64>()
        .map_err(|_| param_error("参数 `tuningHz` 类型非法, 期望 `number`"))?;
    otcp::validate_tuning_hz(value)
}

fn note_rows(data: &serde_json::Value) -> Vec<SharedString> {
    let notes = data
        .get("notes")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    notes
        .iter()
        .map(|note| {
            let index = note
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let start = note
                .get("startContentSec")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let end = note
                .get("endContentSec")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let detected = note
                .get("detectedMidi")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let target = note
                .get("targetMidi")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let cents = note
                .get("offsetCents")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let corrected = note
                .get("corrected")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            SharedString::from(format!(
                "#{index:<3} {start:>6.2}–{end:>6.2}s  MIDI {detected:>3}→{target:<3}  {cents:+6.1}¢  {}",
                if corrected { "校正" } else { "保持" }
            ))
        })
        .collect()
}

fn set_note_rows(ui: &MainWindow, data: &serde_json::Value) {
    ui.set_otcp_notes_rows(ModelRc::new(VecModel::from(note_rows(data))));
}

fn job_status(data: &serde_json::Value) -> String {
    let status = data
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let before = data
        .get("notesBefore")
        .and_then(|value| value.get("count"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let after = data
        .get("notesAfter")
        .and_then(|value| value.get("count"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let delivery = match data.get("delivery") {
        Some(value) if value.get("ready").and_then(serde_json::Value::as_bool) == Some(true) => {
            "交付可取".to_owned()
        }
        Some(value) => value
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .map_or_else(
                || "交付未就绪".to_owned(),
                |reason| format!("交付未就绪（{reason}）"),
            ),
        None => "交付未就绪".to_owned(),
    };
    let asset = data
        .get("asset")
        .and_then(|value| value.get("path"))
        .and_then(serde_json::Value::as_str)
        .map_or_else(String::new, |path| format!(" · {path}"));
    let error = data
        .get("error")
        .and_then(|value| value.get("message"))
        .and_then(serde_json::Value::as_str)
        .map_or_else(String::new, |message| format!(" · {message}"));
    format!("状态 {status} · 音符 {before}→{after} · {delivery}{asset}{error}")
}

/// 接 OTCP 对话框的打开 / 关闭 / 提交 / 轮询 / 音符表 / 交付。
pub fn wire_otcp_dialog(ui: &MainWindow, port: Option<Rc<UndoPort>>) {
    let weak = ui.as_weak();
    ui.on_open_otcp({
        let weak = weak.clone();
        let port = port.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            ui.set_otcp_open(true);
            ui.set_otcp_status("".into());
            if let Some(port) = &port
                && let Some(project) = port.try_project()
                && let Ok((_, _, label)) = selected_audio_clip(&ui, &project)
            {
                ui.set_otcp_clip_label(label.into());
            }
        }
    });

    ui.on_close_otcp({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_otcp_open(false);
            }
        }
    });

    ui.on_otcp_submit({
        let weak = weak.clone();
        let port = port.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Some(port) = &port else {
                ui.set_otcp_status("没有打开的工程（撤销端口未装配）".into());
                return;
            };
            let Some(project) = port.try_project() else {
                ui.set_otcp_status("没有打开的工程".into());
                return;
            };
            let (track_id, clip_id, label) = match selected_audio_clip(&ui, &project) {
                Ok(value) => value,
                Err(message) => {
                    ui.set_otcp_status(message.into());
                    return;
                }
            };
            let key_root = match parse_key_root(ui.get_otcp_key_root().as_str()) {
                Ok(value) => value,
                Err(fault) => {
                    ui.set_otcp_status(fault_text(&fault).into());
                    return;
                }
            };
            let scale = ui.get_otcp_scale().to_string();
            let scale = if scale.trim().is_empty() {
                Err(param_error(
                    "`scale` 必须是音阶名（major | minor | chromatic | harmonicMinor | dorian | mixolydian | pentatonicMajor | pentatonicMinor）",
                ))
            } else {
                otcp::validate_scale(&scale)
            };
            let scale = match scale {
                Ok(value) => value,
                Err(fault) => {
                    ui.set_otcp_status(fault_text(&fault).into());
                    return;
                }
            };
            let mode = ui.get_otcp_mode();
            let mode = if mode.trim().is_empty() {
                "snapNotes".to_owned()
            } else {
                mode.to_string()
            };
            let mode = match otcp::validate_mode(&mode) {
                Ok(value) => value,
                Err(fault) => {
                    ui.set_otcp_status(fault_text(&fault).into());
                    return;
                }
            };
            let tuning_hz = match parse_tuning(ui.get_otcp_tuning_hz().as_str()) {
                Ok(value) => value,
                Err(fault) => {
                    ui.set_otcp_status(fault_text(&fault).into());
                    return;
                }
            };
            ui.set_otcp_clip_label(label.into());
            let outcome = otcp::submit_correction(
                &project,
                |hash| port.asset_bytes(hash),
                track_id,
                clip_id,
                key_root,
                scale,
                mode,
                tuning_hz,
                // GUI 没有提案审阅面：`交付` 是直接提交（见文件头差异登记）。
                // MCP 的 `autoMerge` 只控制“提案是否当场合并”，在这里恒等价于
                // “并入”，所以不再给一个按了也不改变行为的复选框。
                true,
                false,
                Vec::new(),
            );
            match outcome {
                Ok(outcome) => {
                    ui.set_otcp_job_id(outcome.job_id.as_str().into());
                    ui.set_otcp_status(format!("提交成功 · 任务 {} 运行中", outcome.job_id).into());
                }
                Err(fault) => ui.set_otcp_status(fault_text(&fault).into()),
            }
        }
    });

    ui.on_otcp_poll({
        let weak = weak.clone();
        let port = port.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let job_id = ui.get_otcp_job_id().to_string();
            if job_id.trim().is_empty() {
                ui.set_otcp_status("先提交任务，或填一个任务号".into());
                return;
            }
            let project = port.as_ref().and_then(|port| port.try_project());
            match otcp::job_data(&job_id, project.as_ref()) {
                Ok(data) => {
                    ui.set_otcp_status(job_status(&data).into());
                    if data.get("status").and_then(serde_json::Value::as_str) == Some("succeeded") {
                        let notes = if data
                            .get("notesAfter")
                            .and_then(|value| value.get("count"))
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0)
                            > 0
                        {
                            data.get("notesAfter")
                        } else {
                            data.get("notesBefore")
                        };
                        if let Some(notes) = notes {
                            set_note_rows(&ui, notes);
                        }
                    }
                }
                Err(fault) => ui.set_otcp_status(fault_text(&fault).into()),
            }
        }
    });

    ui.on_otcp_notes({
        let weak = weak.clone();
        let port = port.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let job_id = ui.get_otcp_job_id().to_string();
            let result = if job_id.trim().is_empty() {
                let Some(project) = port.as_ref().and_then(|port| port.try_project()) else {
                    ui.set_otcp_status("没有打开的工程".into());
                    return;
                };
                selected_audio_clip(&ui, &project).and_then(|(track_id, clip_id, _)| {
                    otcp::notes_data_for_clip(&track_id, &clip_id)
                        .map_err(|fault| fault_text(&fault))
                })
            } else {
                otcp::notes_data_for_job(&job_id).map_err(|fault| fault_text(&fault))
            };
            match result {
                Ok(data) => {
                    set_note_rows(&ui, &data);
                    let count = data
                        .get("totalCount")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    ui.set_otcp_status(format!("音符表 {count} 个").into());
                }
                Err(message) => ui.set_otcp_status(message.into()),
            }
        }
    });

    ui.on_otcp_deliver({
        let weak = weak.clone();
        let port = port.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Some(port) = &port else {
                ui.set_otcp_status("没有打开的工程（撤销端口未装配）".into());
                return;
            };
            let Some(project) = port.try_project() else {
                ui.set_otcp_status("没有打开的工程".into());
                return;
            };
            let job_id = ui.get_otcp_job_id().to_string();
            if job_id.trim().is_empty() {
                ui.set_otcp_status("先提交任务，或填一个任务号".into());
                return;
            }
            let parts = match otcp::delivery_for_job(&project, &job_id) {
                Ok(parts) => parts,
                Err(fault) => {
                    ui.set_otcp_status(fault_text(&fault).into());
                    return;
                }
            };
            let OtcpDeliveryParts {
                ops,
                asset_bytes,
                title,
                ..
            } = parts;
            // MCP 的 `apply_deliver` 先 `put_asset` 再施加 op；GUI 直接提交也必须
            // 先把字节进撤销端口的 CAS 池，否则 `SetAsset` 指向的哈希在保存 / 重投影
            // 时没有字节（工程会引用一个空资产）。
            let inserted = port.insert_asset(AssetHash::of_bytes(&asset_bytes), asset_bytes);
            let _ = inserted;
            match port.commit_ops(host::now_ms(), &title, ops) {
                Ok(id) => {
                    match otcp::mark_job_delivered(&job_id, "gui-direct") {
                        Ok(()) => ui.set_otcp_status(
                            format!("已交付 · 提交 {} · 任务 {job_id}", id.to_canonical_string())
                                .into(),
                        ),
                        Err(fault) => ui.set_otcp_status(
                            format!("已提交 {id}，但交付收据写失败：{}", fault_text(&fault)).into(),
                        ),
                    }
                    host::reproject_now(&ui, port);
                }
                Err(refusal) => ui.set_otcp_status(format!("交付被拒：{refusal}").into()),
            }
        }
    });
}
