//! 审评 2026-10-31（P2-3）判据：GUI 导出 WAV 的缺省路径。
//!
//! 旧行为：`ModulePaths::default` 的 `export_wav` 是相对名 `ui-export.wav` ⇒
//! 落到**进程 cwd**，用户导完不知道落哪（审评 §5.2 的原话）。
//!
//! 新行为（[`yeban_app::host::ModulePaths::from_env_for_project`] 的三档优先级）：
//! env `YEBAN_UI_EXPORT_WAV_PATH` 已给 ⇒ 原样；否则工程有文件落点 ⇒
//! 工程目录 + `ui-export.wav`（绝对路径）；否则 ⇒ 相对缺省（旧行为）。
//!
//! 本文件只有一个 `#[test]`（与 `tests/ui_export_dialog.rs` 同款）：环境变量是
//! 进程全局的，单测二进制里改它才不会干扰其它判据。

use yeban_app::host::ModulePaths;

/// 临时目录（`Drop` 时清理，**绝不污染仓库**）。
struct Scratch {
    dir: std::path::PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-export-wav-project-dir-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn export_wav_default_prefers_env_then_project_dir_then_cwd() {
    let saved = std::env::var("YEBAN_UI_EXPORT_WAV_PATH").ok();
    // SAFETY: 本测试二进制只有一个 `#[test]` ⇒ 不存在第二个线程读这个变量。
    unsafe {
        std::env::remove_var("YEBAN_UI_EXPORT_WAV_PATH");
    }

    let scratch = Scratch::new();
    let project_dir = scratch.dir.join("song-dir");
    std::fs::create_dir_all(&project_dir).expect("建工程目录");

    // ① env 未设 + 工程有落点 ⇒ 工程目录 + ui-export.wav（绝对路径）。
    let paths = ModulePaths::from_env_for_project(Some(&project_dir));
    let expected = std::fs::canonicalize(&project_dir)
        .expect("工程目录必须能规范化")
        .join("ui-export.wav");
    assert_eq!(
        paths.export_wav, expected,
        "env 未设时缺省必须落到工程目录，而不是进程 cwd"
    );
    assert!(
        paths.export_wav.is_absolute(),
        "工程目录缺省必须是绝对路径: {}",
        paths.export_wav.display()
    );

    // ② env 已设 ⇒ env 赢（工程目录不许覆盖用户显式选择）。
    let env_path = scratch.dir.join("env-chosen.wav");
    // SAFETY: 同上 —— 单 `#[test]` 的进程。
    unsafe {
        std::env::set_var("YEBAN_UI_EXPORT_WAV_PATH", &env_path);
    }
    let paths = ModulePaths::from_env_for_project(Some(&project_dir));
    assert_eq!(
        paths.export_wav, env_path,
        "env 已设时工程目录缺省不许覆盖它"
    );

    // ③ env 未设 + 新建未保存工程 ⇒ 相对缺省（旧行为，不拦接线）。
    // SAFETY: 同上 —— 单 `#[test]` 的进程。
    unsafe {
        std::env::remove_var("YEBAN_UI_EXPORT_WAV_PATH");
    }
    let paths = ModulePaths::from_env_for_project(None);
    assert_eq!(
        paths.export_wav,
        std::path::PathBuf::from("ui-export.wav"),
        "无工程落点时保持相对缺省"
    );

    // 恢复调用方环境（本二进制单测，仍按规矩还原）。
    // SAFETY: 同上 —— 单 `#[test]` 的进程。
    unsafe {
        match saved {
            Some(value) => std::env::set_var("YEBAN_UI_EXPORT_WAV_PATH", value),
            None => std::env::remove_var("YEBAN_UI_EXPORT_WAV_PATH"),
        }
    }
}
