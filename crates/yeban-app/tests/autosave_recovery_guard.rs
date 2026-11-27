//! T-FIX-796 装配判据（源码文本级）：ARCH-SYS-001 三件事都接线且响亮。
//! 本机按 AGENTS.md §5 不跑 Slint 重依赖构建；GUI 实测交 CI。

const AUTOSAVE: &str = include_str!("../src/autosave.rs");
const HOST: &str = include_str!("../src/host.rs");
const MAIN: &str = include_str!("../src/main.rs");
const APP: &str = include_str!("../ui/app.slint");
const LIB: &str = include_str!("../src/lib.rs");
const SAVE: &str = include_str!("../src/save.rs");

#[test]
fn pure_layer_covers_interval_draft_sweep_quarantine() {
    for needle in [
        "pub const DEFAULT_INTERVAL_SECS: u64 = 60",
        "pub fn interval_secs()",
        "pub const DRAFT_EXT: &str = \"yeban.autosave\"",
        "pub fn draft_path(",
        "pub fn sweep(",
        "pub fn quarantine(",
        "eprintln!",
    ] {
        assert!(AUTOSAVE.contains(needle), "纯逻辑层缺少 {needle}");
    }
}

#[test]
fn writes_draft_through_existing_atomic_primitive() {
    assert!(
        HOST.contains("crate::save::save_archive_file(&archive, &draft_path)"),
        "草稿必须复用手动保存的原子落盘原语"
    );
    assert!(
        HOST.contains("crate::autosave::draft_path(&project_path)"),
        "草稿路径必须从正式工程派生且不同扩展名"
    );
    assert!(
        SAVE.contains("pub fn save_archive_file("),
        "被复用的原子写原语必须存在"
    );
}

#[test]
fn startup_detection_and_loud_disposition() {
    assert!(HOST.contains("crate::autosave::sweep(dir)"), "缺启动期扫描");
    assert!(
        HOST.contains("crate::autosave::quarantine(corrupt)"),
        "残缺草稿必须隔离而非静默丢弃"
    );
    assert!(
        HOST.contains("std::fs::remove_file(temp)"),
        "残留临时文件必须清理"
    );
    assert!(
        HOST.contains("outcome=saved") && HOST.contains("outcome=error"),
        "草稿写成功/失败都要进回执"
    );
    assert!(HOST.contains("autosave-recovery"), "恢复结果必须进模块回执");
    assert!(
        HOST.contains("crate::dirty_guard::mark_saved(port.display().head)"),
        "恢复后必须重设保存点，避免误报脏"
    );
}

#[test]
fn recovery_prompt_and_production_wiring() {
    assert!(LIB.contains("pub mod autosave;"), "模块未注册");
    assert!(
        HOST.contains("pub fn wire_autosave_and_recovery("),
        "宿主接线函数缺失"
    );
    assert!(
        MAIN.contains("host::wire_autosave_and_recovery(&ui, &undo_port, project_path, 0);"),
        "产品装配未调用接线"
    );
    for needle in [
        "recovery-prompt-open",
        "recovery-message",
        "recovery-notices",
        "callback recovery-accept(int)",
        "callback recovery-discard(int)",
        "root.recovery-accept(idx)",
        "root.recovery-discard(-1)",
    ] {
        assert!(APP.contains(needle), "恢复 UI 缺少 {needle}");
    }
}
