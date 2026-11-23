//! 自动相位对齐 GUI（Hub #711）。
//!
//! 入口：菜单「轨道 → 自动相位对齐...」（`app.slint` 的 `open-phase-align`）。
//! 测量与补偿语义全部复用 `yeban_mcp::domain::phase_align_auto::measure_phase_alignment`
//! （同一个 provider、同一套 `maxLagMs` / 轨存在性 / 同轨 / 低相关 / ±4800 采样门），
//! 本模块只做轨名解析、Slint 文本拼装与 `UndoPort::commit_ops`。

use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr as _;

use std::collections::BTreeMap;

use slint::{ComponentHandle, Model as _};

use yeban_mcp::domain::error::Fault;
use yeban_mcp::domain::phase_align_auto::{self, PhaseAlignAutoOutcome};
use yeban_mcp::phase_align_auto_provider::RealPhaseAlignAutoProvider;
use yeban_mcp::song_overview_provider::ProviderContext;
use yeban_model::{AssetHash, EntityId, YebanProjectV1};

use crate::host;
use crate::ui::MainWindow;
use crate::undo::UndoPort;

fn fault_text(fault: &Fault) -> String {
    match fault {
        Fault::Domain { message, .. } => message.clone(),
        Fault::Impl { error } => format!("{error:?}"),
    }
}

fn track_at(ui: &MainWindow, index: usize) -> Option<(EntityId, String)> {
    let ids = ui.get_track_ids();
    let names = ui.get_track_names();
    let id = ids.row_data(index)?;
    let name = names.row_data(index)?;
    EntityId::from_str(id.as_str())
        .ok()
        .map(|id| (id, name.to_string()))
}

fn selected_track(ui: &MainWindow) -> Option<(EntityId, String)> {
    let index = usize::try_from(ui.get_selected_track_index()).ok()?;
    track_at(ui, index)
}

fn first_other_track(ui: &MainWindow, exclude: EntityId) -> Option<(EntityId, String)> {
    let count = ui.get_track_ids().row_count();
    (0..count).find_map(|index| {
        let (id, name) = track_at(ui, index)?;
        (id != exclude).then_some((id, name))
    })
}

fn resolve_track(project: &YebanProjectV1, text: &str, param: &str) -> Result<EntityId, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(format!("`{param}` 必填，必须是 26 字符 ULID"));
    }
    if let Ok(id) = EntityId::from_str(text) {
        if project.tracks.contains_key(&id) {
            return Ok(id);
        }
        let label = if param == "referenceTrackId" {
            "基准轨"
        } else {
            "目标轨"
        };
        return Err(format!("{label} {id} 不在工程里"));
    }
    let lower = text.to_lowercase();
    project
        .tracks
        .values()
        .find(|track| track.name.to_lowercase() == lower)
        .map(|track| track.id)
        .ok_or_else(|| format!("工程里没有名为 `{text}` 的轨"))
}

fn format_status(outcome: &PhaseAlignAutoOutcome) -> String {
    let data = &outcome.data;
    let lag_samples = data
        .get("lagSamples")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    let lag_ms = data
        .get("lagMs")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let correlation = data
        .get("correlation")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let confidence = data
        .get("confidence")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let applied = data
        .get("applied")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let delay = data.get("appliedDelay").and_then(|value| {
        let before = value.get("delaySamplesBefore")?.as_i64()?;
        let after = value.get("delaySamplesAfter")?.as_i64()?;
        Some(format!("延时 {before}→{after} 采样"))
    });
    let polarity = data
        .get("appliedPolarity")
        .and_then(|value| value.get("invertAfter"))
        .and_then(serde_json::Value::as_bool)
        .map(|invert| format!("极性翻转={invert}"));
    let head = if applied {
        "已测量并补偿"
    } else {
        "只测量（未改工程）"
    };
    let mut parts = vec![
        head.to_owned(),
        format!("lag {lag_samples} 采样 / {lag_ms:.2} ms"),
        format!("ρ={correlation:.2}"),
        format!("confidence {confidence}"),
    ];
    if let Some(delay) = delay {
        parts.push(delay);
    }
    if let Some(polarity) = polarity {
        parts.push(polarity);
    }
    parts.join(" · ")
}

/// 接相位对齐对话框的打开 / 关闭 / 测量。
pub fn wire_phase_align_dialog(
    ui: &MainWindow,
    port: Option<Rc<UndoPort>>,
    project_dir: Option<PathBuf>,
) {
    let weak = ui.as_weak();
    ui.on_open_phase_align({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            ui.set_phase_align_open(true);
            ui.set_phase_align_status("".into());
            let (reference, target) = match selected_track(&ui) {
                Some((id, name)) => match first_other_track(&ui, id) {
                    Some((_, other)) => (name, other),
                    None => (name, String::new()),
                },
                None => match (track_at(&ui, 0), track_at(&ui, 1)) {
                    (Some((_, first)), Some((_, second))) => (first, second),
                    _ => (String::new(), String::new()),
                },
            };
            ui.set_phase_align_reference(reference.into());
            ui.set_phase_align_target(target.into());
        }
    });

    ui.on_close_phase_align({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_phase_align_open(false);
            }
        }
    });

    ui.on_run_phase_align({
        let weak = weak.clone();
        let port = port.clone();
        let project_dir = project_dir.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Some(port) = &port else {
                ui.set_phase_align_status("没有打开的工程（撤销端口未装配）".into());
                return;
            };
            let Some(project) = port.try_project() else {
                ui.set_phase_align_status("没有打开的工程".into());
                return;
            };
            let reference = match resolve_track(
                &project,
                ui.get_phase_align_reference().as_str(),
                "referenceTrackId",
            ) {
                Ok(id) => id,
                Err(message) => {
                    ui.set_phase_align_status(message.into());
                    return;
                }
            };
            let target = match resolve_track(
                &project,
                ui.get_phase_align_target().as_str(),
                "targetTrackId",
            ) {
                Ok(id) => id,
                Err(message) => {
                    ui.set_phase_align_status(message.into());
                    return;
                }
            };
            if reference == target {
                ui.set_phase_align_status(
                    "`referenceTrackId` 与 `targetTrackId` 不能是同一条轨：自己与自己没有时差可测"
                        .into(),
                );
                return;
            }
            let max_lag_text = ui.get_phase_align_max_lag_ms();
            let max_lag_text = max_lag_text.trim();
            let max_lag_ms = if max_lag_text.is_empty() {
                50.0
            } else {
                match max_lag_text.parse::<f64>() {
                    Ok(value) => value,
                    Err(_) => {
                        ui.set_phase_align_status("参数 `maxLagMs` 类型非法, 期望 `number`".into());
                        return;
                    }
                }
            };
            let assets = port
                .assets()
                .into_iter()
                .collect::<BTreeMap<AssetHash, Vec<u8>>>();
            let provider = RealPhaseAlignAutoProvider::new(ProviderContext::shared(
                project_dir.clone(),
                std::sync::Arc::new(assets),
            ));
            let outcome = match phase_align_auto::measure_phase_alignment(
                &project,
                &provider,
                reference,
                target,
                max_lag_ms,
                ui.get_phase_align_apply(),
            ) {
                Ok(outcome) => outcome,
                Err(fault) => {
                    ui.set_phase_align_status(fault_text(&fault).into());
                    return;
                }
            };
            if ui.get_phase_align_apply() && !outcome.ops.is_empty() {
                let ops = outcome.ops.clone();
                match port.commit_ops(host::now_ms(), &outcome.title, ops) {
                    Ok(id) => {
                        let mut status = format_status(&outcome);
                        status.push_str(&format!(" · 提交 {}", id.to_canonical_string()));
                        ui.set_phase_align_status(status.into());
                        host::reproject_now(&ui, port);
                    }
                    Err(refusal) => {
                        ui.set_phase_align_status(format!("补偿被拒：{refusal}").into());
                    }
                }
            } else {
                ui.set_phase_align_status(format_status(&outcome).into());
            }
        }
    });
}
