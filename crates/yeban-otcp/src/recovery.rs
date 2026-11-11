//! 模态框恢复（设计稿 §4.2）：`busy.modalOpen == true` 是失败态。
//!
//! 循环（每次写操作前、每次报错后都跑）：
//! 1. `ui.getState` → `busy.modalOpen`、`dialogs[{kind, trackId, closeCommits}]`
//!    （从底到顶）与 `onboarding.showing`；
//! 2. 无对话框、无 onboarding 遮罩 → 继续；
//! 3. 全部 `closeCommits: false` → `ui.dismiss {target: "dialogs"}` →
//!    校验 `sideEffects.dialogClosed.dialogs` 列出它们；重查直到
//!    `modalOpen` 为假；
//! 4. 任一对话框 `closeCommits: true` → 普通解散会报
//!    `INVALID_ARGUMENT(dialog_commits_on_close)`，驱动**停住**并报
//!    `otcpDialogCommits` —— ⛔ 永不自行设 `confirm: true`
//!    （那是最终用户的同意，只能由 MCP 调用方显式传回）；
//! 5. 首跑 onboarding 遮罩（`onboarding.showing: true`）：新鲜服务端
//!    （每任务一个 `OPENTUNE_DATA_DIR`）启动即有；解散对话框时服务端
//!    常随之关掉它，若仍在 → `ui.dismiss {target: "onboarding"}`
//!    （需 `ui.onboardingClose` 能力；缺失 ⇒ `otcpModalBlocked`）。
//!    解散只改本次任务临时数据目录里的 `onboardingShown` 偏好，
//!    关机即删目录，用户侧无残留。
//!
//! 唯一允许在模态框打开时发的写操作就是 `ui.dismiss`。

use serde_json::{Value, json};

use crate::client::OtcpClient;
use crate::error::OtcpError;

/// 对话框（`ui.getState` 的 `dialogs[]` 条目）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dialog {
    /// 对话框种类（如 `message`）。
    pub kind: String,
    /// 关闭时是否提交（`true` ⇒ 驱动停住，报 `otcpDialogCommits`）。
    pub close_commits: bool,
    /// 所属轨道（可空）。
    pub track_id: Option<String>,
}

impl Dialog {
    fn from_value(value: &Value) -> Self {
        Self {
            kind: value
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            close_commits: value
                .get("closeCommits")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            track_id: value
                .get("trackId")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        }
    }
}

/// `ui.getState` 的状态快照。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiState {
    /// 是否有模态框打开。
    pub modal_open: bool,
    /// 对话框（从底到顶）。
    pub dialogs: Vec<Dialog>,
    /// 首跑 onboarding 遮罩是否正在显示。
    pub onboarding_showing: bool,
}

impl UiState {
    fn from_value(value: &Value) -> Self {
        let busy = value.get("busy").cloned().unwrap_or(Value::Null);
        let modal_open = busy
            .get("modalOpen")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let dialogs = value
            .get("dialogs")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().map(Dialog::from_value).collect())
            .unwrap_or_default();
        let onboarding_showing = value
            .get("onboarding")
            .and_then(|o| o.get("showing"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Self {
            modal_open,
            dialogs,
            onboarding_showing,
        }
    }

    /// 是否干净（无模态框、无对话框、无 onboarding 遮罩）。
    #[must_use]
    pub fn is_clear(&self) -> bool {
        !self.modal_open && self.dialogs.is_empty() && !self.onboarding_showing
    }
}

/// 读一次 `ui.getState`。
pub fn get_ui_state(client: &mut OtcpClient, job_id: &str) -> Result<UiState, OtcpError> {
    let state = client.call("ui.getState", json!({}), job_id)?;
    Ok(UiState::from_value(&state))
}

/// 模态框恢复循环。
///
/// * 干净（无模态框、无对话框、无 onboarding 遮罩）⇒ `Ok(())` 立即返回；
/// * 全部对话框 `closeCommits: false` ⇒ 解散对话框、再解散残留的
///   onboarding 遮罩，重查直到干净（最多 `max_rounds` 轮，
///   超轮 ⇒ `otcpModalBlocked`）；
/// * 任一对话框 `closeCommits: true` ⇒ `otcpDialogCommits`，⛔ 不解散。
pub fn recover_modal(
    client: &mut OtcpClient,
    job_id: &str,
    max_rounds: usize,
) -> Result<(), OtcpError> {
    for _ in 0..max_rounds.max(1) {
        let state = get_ui_state(client, job_id)?;
        if state.is_clear() {
            return Ok(());
        }
        if state.dialogs.iter().any(|d| d.close_commits) {
            let kinds: Vec<&str> = state
                .dialogs
                .iter()
                .filter(|d| d.close_commits)
                .map(|d| d.kind.as_str())
                .collect();
            return Err(OtcpError::with_data(
                "otcpDialogCommits",
                format!(
                    "对话框关闭时会提交（{}），须最终用户显式确认；驱动停住，不自行确认",
                    kinds.join(", ")
                ),
                json!({
                    "job_id": job_id,
                    "dialogs": kinds,
                }),
            ));
        }
        // 全部 closeCommits: false：先解散对话框。
        if !state.dialogs.is_empty() {
            let dismissed = client.call("ui.dismiss", json!({"target": "dialogs"}), job_id)?;
            let closed = count_closed_dialogs(&dismissed);
            if closed < state.dialogs.len() {
                // 服务端没列全已关闭的对话框：重查一次为准。
                let rechecked = get_ui_state(client, job_id)?;
                if rechecked.is_clear() {
                    return Ok(());
                }
            }
        }
        // 残留的 onboarding 遮罩：单独解散（解散对话框时服务端常随之
        // 关掉它；只在仍显示时才发这一次调用）。
        let rechecked = get_ui_state(client, job_id)?;
        if rechecked.is_clear() {
            return Ok(());
        }
        if rechecked.onboarding_showing && rechecked.dialogs.is_empty() {
            client.call("ui.dismiss", json!({"target": "onboarding"}), job_id)?;
        }
    }
    // 多轮仍未清：最后再查一次。
    let state = get_ui_state(client, job_id)?;
    if state.is_clear() {
        Ok(())
    } else {
        Err(OtcpError::with_data(
            "otcpModalBlocked",
            format!(
                "模态框解散 {} 轮后仍打开（{} 个对话框，onboarding 显示中：{}），写操作停住",
                max_rounds,
                state.dialogs.len(),
                state.onboarding_showing,
            ),
            json!({
                "job_id": job_id,
                "dialogs": state.dialogs.iter().map(|d| d.kind.clone()).collect::<Vec<_>>(),
                "onboarding_showing": state.onboarding_showing,
            }),
        ))
    }
}

/// 数 `ui.dismiss` 结果里列出的已关闭对话框数。
///
/// 真服务端形状（`ui.dismiss.schema.json`）：`sideEffects.dialogClosed`
/// 是对象 `{dialogs: [{kind, trackId}], revert: null}`；旧 mock 曾写
/// 字符串数组，两形都认。
fn count_closed_dialogs(result: &Value) -> usize {
    let closed = result
        .get("sideEffects")
        .and_then(|s| s.get("dialogClosed"));
    match closed {
        Some(Value::Array(arr)) => arr.len(),
        Some(Value::Object(_)) => closed
            .and_then(|c| c.get("dialogs"))
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_state_parses_dialogs() {
        let value = json!({
            "busy": {"modalOpen": true, "pointerDown": false},
            "dialogs": [
                {"kind": "message", "closeCommits": false},
                {"kind": "confirm", "closeCommits": true, "trackId": "t1"}
            ]
        });
        let state = UiState::from_value(&value);
        assert!(state.modal_open);
        assert_eq!(state.dialogs.len(), 2);
        assert!(!state.dialogs[0].close_commits);
        assert!(state.dialogs[1].close_commits);
        assert_eq!(state.dialogs[1].track_id.as_deref(), Some("t1"));
    }

    #[test]
    fn ui_state_empty_when_no_modal() {
        let state = UiState::from_value(&json!({
            "busy": {"modalOpen": false},
            "dialogs": []
        }));
        assert!(state.is_clear());
    }

    #[test]
    fn ui_state_parses_onboarding_showing() {
        // 真服务端首跑形状：confirm 对话框 + onboarding 遮罩同时在。
        let state = UiState::from_value(&json!({
            "busy": {"modalOpen": true},
            "dialogs": [{"kind": "confirm", "closeCommits": false}],
            "onboarding": {"shown": true, "showing": true}
        }));
        assert!(state.modal_open);
        assert!(state.onboarding_showing);
        assert!(!state.is_clear());
        let clear = UiState::from_value(&json!({
            "busy": {"modalOpen": false},
            "dialogs": [],
            "onboarding": {"shown": true, "showing": false}
        }));
        assert!(clear.is_clear());
    }

    #[test]
    fn count_closed_dialogs_accepts_both_shapes() {
        // 真服务端：对象 {dialogs: [{kind, trackId}], revert: null}。
        let object = json!({
            "sideEffects": {"dialogClosed": {
                "dialogs": [{"kind": "confirm", "trackId": null}],
                "revert": null
            }}
        });
        assert_eq!(count_closed_dialogs(&object), 1);
        // 旧 mock：字符串数组。
        let array = json!({
            "sideEffects": {"dialogClosed": ["confirm"]}
        });
        assert_eq!(count_closed_dialogs(&array), 1);
        assert_eq!(count_closed_dialogs(&json!({})), 0);
    }
}
