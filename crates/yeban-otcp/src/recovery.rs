//! 模态框恢复（设计稿 §4.2）：`busy.modalOpen == true` 是失败态。
//!
//! 循环（每次写操作前、每次报错后都跑）：
//! 1. `ui.getState` → `busy.modalOpen` 与 `dialogs[{kind, trackId, closeCommits}]`
//!    （从底到顶）；
//! 2. 无对话框 → 继续；
//! 3. 全部 `closeCommits: false` → `ui.dismiss {target: "dialogs"}` →
//!    校验 `sideEffects.dialogClosed` 列出它们；重查直到 `modalOpen` 为假；
//! 4. 任一对话框 `closeCommits: true` → 普通解散会报
//!    `INVALID_ARGUMENT(dialog_commits_on_close)`，驱动**停住**并报
//!    `otcpDialogCommits` —— ⛔ 永不自行设 `confirm: true`
//!    （那是最终用户的同意，只能由 MCP 调用方显式传回）。
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
        Self {
            modal_open,
            dialogs,
        }
    }
}

/// 读一次 `ui.getState`。
pub fn get_ui_state(client: &mut OtcpClient, job_id: &str) -> Result<UiState, OtcpError> {
    let state = client.call("ui.getState", json!({}), job_id)?;
    Ok(UiState::from_value(&state))
}

/// 模态框恢复循环。
///
/// * 无模态框 ⇒ `Ok(())` 立即返回；
/// * 全部对话框 `closeCommits: false` ⇒ 解散并重查，直到 `modalOpen` 为假
///   （最多 `max_rounds` 轮，缺省 3 轮；超轮 ⇒ `otcpModalBlocked`）；
/// * 任一对话框 `closeCommits: true` ⇒ `otcpDialogCommits`，⛔ 不解散。
pub fn recover_modal(
    client: &mut OtcpClient,
    job_id: &str,
    max_rounds: usize,
) -> Result<(), OtcpError> {
    for _ in 0..max_rounds.max(1) {
        let state = get_ui_state(client, job_id)?;
        if !state.modal_open && state.dialogs.is_empty() {
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
        // 全部 closeCommits: false：解散。
        let dismissed = client.call("ui.dismiss", json!({"target": "dialogs"}), job_id)?;
        let closed: Vec<String> = dismissed
            .get("sideEffects")
            .and_then(|s| s.get("dialogClosed"))
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let expected = state.dialogs.len();
        if closed.len() < expected {
            // 服务端没列全已关闭的对话框：重查一次为准。
            let rechecked = get_ui_state(client, job_id)?;
            if !rechecked.modal_open && rechecked.dialogs.is_empty() {
                return Ok(());
            }
        }
    }
    // 多轮仍未清：最后再查一次。
    let state = get_ui_state(client, job_id)?;
    if !state.modal_open && state.dialogs.is_empty() {
        Ok(())
    } else {
        Err(OtcpError::with_data(
            "otcpModalBlocked",
            format!(
                "模态框解散 {} 轮后仍打开（{} 个对话框），写操作停住",
                max_rounds,
                state.dialogs.len()
            ),
            json!({
                "job_id": job_id,
                "dialogs": state.dialogs.iter().map(|d| d.kind.clone()).collect::<Vec<_>>(),
            }),
        ))
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
        assert!(!state.modal_open);
        assert!(state.dialogs.is_empty());
    }
}
