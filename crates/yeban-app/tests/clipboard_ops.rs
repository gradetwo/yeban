//! **片段剪贴板：复制 / 粘贴 / 剪切**（ADR-0076 B3）。
//!
//! 修前：`app.slint` 三词（复制/粘贴/剪切）各 0 处，`host.rs` 无 `edit-copy/paste/cut` 臂
//! —— 想把片段搬到另一条轨只能重画或重拖。
//!
//! 修法：进程内 `CLIP_CLIPBOARD`（`ClipPoolEntry` + 来源 `ClipPlacement`）；
//! - 复制：存选中片段（`clipboard_copy_selected`）
//! - 粘贴：到当前选中轨的播放头位置，新片段身份 + 每个音符新身份，
//!   一次 `commit_ops`（`AddClip` + `AddClipPlacement`）⇒ **一步撤销**
//! - 剪切：复制 + `RemoveClipPlacement`，**一次**撤销同时回滚两者
//! - 快捷键 `Cmd/Ctrl+C/V/X`（文本焦点由第 2 步整体放行，不劫持系统剪贴板）
//! - 菜单「编辑」三项目 → 同一组函数（键盘与菜单不分叉）
//!
//! ⚠ 纯 `include_str!` 判据（本机编不了 Slint）；行为由 CI 的 yeban-app 腿真跑。

const HOST: &str = include_str!("../src/host.rs");
const INPUT: &str = include_str!("../src/input.rs");
const APP: &str = include_str!("../ui/app.slint");

#[test]
fn clipboard_core_functions_exist() {
    for f in [
        "pub fn clipboard_copy_selected(",
        "pub fn clipboard_paste(",
        "pub fn clipboard_cut(",
    ] {
        assert!(HOST.contains(f), "缺少 {f}");
    }
    assert!(
        HOST.contains("thread_local!") && HOST.contains("CLIP_CLIPBOARD"),
        "必须有进程内剪贴板状态"
    );
    // 粘贴必须是「新片段身份 + 每个音符新身份」+ 一次 commit（一步撤销）
    assert!(
        HOST.contains("copy.id = yeban_model::EntityId::new();"),
        "粘贴必须给每个音符新身份（复用会撞 MODEL-AST 唯一性）"
    );
}

#[test]
fn clipboard_keys_and_menu_are_wired() {
    for (key, action) in [
        ("'c'", "CopyClipboard"),
        ("'v'", "PasteClipboard"),
        ("'x'", "CutClipboard"),
    ] {
        assert!(
            INPUT.contains(&format!(
                "(Key::Character({key}), false, false) => Resolution::Action(Action::{action})"
            )),
            "Cmd/Ctrl+{key} 必须绑定 Action::{action}"
        );
        assert!(
            HOST.contains(&format!("Action::{action} =>")),
            "apply_action 必须有 Action::{action} 臂"
        );
    }
    for cmd in ["edit-copy", "edit-paste", "edit-cut"] {
        assert!(
            HOST.contains(&format!("\"{cmd}\" =>")),
            "命令臂必须有 {cmd}"
        );
    }
    for label in [
        "复制片段 (Ctrl+C)",
        "粘贴片段 (Ctrl+V)",
        "剪切片段 (Ctrl+X)",
    ] {
        assert!(APP.contains(label), "编辑菜单缺少「{label}」");
    }
}
