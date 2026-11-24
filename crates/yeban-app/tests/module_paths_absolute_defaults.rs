//! T-FIX-179 判据：GUI 模块入口缺省路径的三档与绝对缺省。
//!
//! 旧行为：`ModulePaths::default` 的五条缺省都是相对文件名
//! （`ui-export.wav` / `ui-export.mid` / `ui-import.mid` / `ui-instrument.sfz` /
//! `ui-record.wav`）⇒ 落到**进程 cwd**；`from_env_for_project` 只给
//! `export_wav`（审评 P2-3）与 `export_midi`（T-FIX-167 补齐）三档，
//! `import_midi` / `load_sfz` / `record_wav` 只有 env / 缺省两档，工程目录
//! 对它们不生效。从 Finder 双击 .app / 桌面快捷方式启动时 cwd 可能是 `/`、
//! `C:\Windows\System32` 等不可写目录 ⇒ 导出 `PermissionDenied`、导入 /
//! 装载 `NotFound`。
//!
//! 新行为（[`yeban_app::host::ModulePaths::from_env_for_project`]）：
//! 四条（`export_midi` / `import_midi` / `load_sfz` / `record_wav`）与
//! `export_wav` 同款三档（env > 工程目录 > 缺省），只是第三档是**绝对**缺省
//! —— 文档目录 `Yeban/<file>`，⛔ 不再落进程 cwd。`export_wav` 的第三档
//! 不动（仍是相对缺省，旧行为）。
//!
//! 本文件只有一个 `#[test]`（与 `tests/export_wav_project_dir_default.rs`
//! 同款）：环境变量与 `HOME` / cwd 都是进程全局的，单测二进制里改它才不会
//! 干扰其它判据。`HOME` 被指向临时目录 ⇒ 文档目录落在临时目录里，
//! **绝不写真实的 `~/Documents`**。

use yeban_app::host::{ModulePaths, ensure_default_parent_dir};

/// 临时目录（`Drop` 时清理，**绝不污染仓库与真实家目录**）。
struct Scratch {
    dir: std::path::PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-179-module-paths-{}-{}",
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

const ENV_VARS: [&str; 7] = [
    "HOME",
    "XDG_DOCUMENTS_DIR",
    "YEBAN_UI_EXPORT_WAV_PATH",
    "YEBAN_UI_EXPORT_MIDI_PATH",
    "YEBAN_UI_IMPORT_MIDI_PATH",
    "YEBAN_UI_LOAD_SFZ_PATH",
    "YEBAN_UI_RECORD_WAV_PATH",
];

#[test]
fn module_paths_four_entries_follow_three_tiers_with_absolute_no_project_default() {
    // 保存调用方环境（本二进制单测，仍按规矩还原）。
    let saved: Vec<(String, Option<String>)> = ENV_VARS
        .iter()
        .map(|name| (name.to_string(), std::env::var(name).ok()))
        .collect();
    let saved_cwd = std::env::current_dir().expect("读 cwd");
    // SAFETY: 本测试二进制只有一个 `#[test]` ⇒ 不存在第二个线程读这些变量。
    unsafe {
        for name in ENV_VARS {
            std::env::remove_var(name);
        }
    }

    let scratch = Scratch::new();
    let home = scratch.dir.join("home");
    let docs_root = home.join("Documents").join("Yeban");
    let project_dir = scratch.dir.join("proj");
    std::fs::create_dir_all(&project_dir).expect("建工程目录");
    // SAFETY: 同上 —— 单 `#[test]` 的进程。
    unsafe {
        std::env::set_var("HOME", &home);
    }

    let files = [
        ("export_wav", "ui-export.wav"),
        ("export_midi", "ui-export.mid"),
        ("import_midi", "ui-import.mid"),
        ("load_sfz", "ui-instrument.sfz"),
        ("record_wav", "ui-record.wav"),
    ];
    let get = |paths: &ModulePaths, name: &str| match name {
        "export_wav" => paths.export_wav.clone(),
        "export_midi" => paths.export_midi.clone(),
        "import_midi" => paths.import_midi.clone(),
        "load_sfz" => paths.load_sfz.clone(),
        "record_wav" => paths.record_wav.clone(),
        _ => unreachable!(),
    };

    // ① env 未设 + 工程有落点 ⇒ 五条都落工程目录（绝对路径）。
    let canonical_proj = std::fs::canonicalize(&project_dir).expect("工程目录必须能规范化");
    let paths = ModulePaths::from_env_for_project(Some(&project_dir));
    for (name, file) in files {
        assert_eq!(
            get(&paths, name),
            canonical_proj.join(file),
            "工程目录档：{name} 必须落工程目录"
        );
        assert!(get(&paths, name).is_absolute(), "{name} 必须是绝对路径");
    }

    // ② env 未设 + 新建未保存工程 ⇒ export_wav 保持相对缺省（旧行为，不动）；
    // 其余四条 ⇒ 文档目录 Yeban/ 下的绝对路径，⛔ 不落进程 cwd。
    let paths = ModulePaths::from_env_for_project(None);
    assert_eq!(
        paths.export_wav,
        std::path::PathBuf::from("ui-export.wav"),
        "无工程落点时 export_wav 保持相对缺省（T-FIX-179 不动它）"
    );
    for (name, file) in files.iter().skip(1) {
        let got = get(&paths, name);
        assert_eq!(
            got,
            docs_root.join(file),
            "无工程落点时 {name} 必须是文档目录 Yeban/ 下的绝对路径"
        );
        assert!(
            got.is_absolute(),
            "{name} 必须是绝对路径: {}",
            got.display()
        );
    }

    // ③ env 已设 ⇒ env 赢（工程目录 / 缺省都不许覆盖用户显式选择），
    // 无工程与有工程两种情形都一样。
    let env_dir = scratch.dir.join("env-chosen");
    std::fs::create_dir_all(&env_dir).expect("建 env 目录");
    // SAFETY: 同上 —— 单 `#[test]` 的进程。
    unsafe {
        for (name, file) in files.iter().skip(1) {
            let var = match *name {
                "export_midi" => "YEBAN_UI_EXPORT_MIDI_PATH",
                "import_midi" => "YEBAN_UI_IMPORT_MIDI_PATH",
                "load_sfz" => "YEBAN_UI_LOAD_SFZ_PATH",
                "record_wav" => "YEBAN_UI_RECORD_WAV_PATH",
                _ => unreachable!(),
            };
            std::env::set_var(var, env_dir.join(file));
        }
    }
    for project in [None, Some(project_dir.as_path())] {
        let paths = ModulePaths::from_env_for_project(project);
        for (name, file) in files.iter().skip(1) {
            assert_eq!(
                get(&paths, name),
                env_dir.join(file),
                "env 已设时 {name} 必须原样用 env 值（project={project:?}）"
            );
        }
    }
    // SAFETY: 同上 —— 单 `#[test]` 的进程。
    unsafe {
        for name in [
            "YEBAN_UI_EXPORT_MIDI_PATH",
            "YEBAN_UI_IMPORT_MIDI_PATH",
            "YEBAN_UI_LOAD_SFZ_PATH",
            "YEBAN_UI_RECORD_WAV_PATH",
        ] {
            std::env::remove_var(name);
        }
    }

    // ④ 验收 (1)：cwd 设为只读目录 ⇒ 缺省导出 MIDI 的目标不在 cwd，
    // 且导出成功落到可写目录（文档目录 Yeban/，父目录由
    // `ensure_default_parent_dir` 建出来，与 GUI 的导出 MIDI 入口同一条）。
    let ro_cwd = scratch.dir.join("ro-cwd");
    std::fs::create_dir_all(&ro_cwd).expect("建只读 cwd");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = std::fs::metadata(&ro_cwd).expect("元数据").permissions();
        permissions.set_mode(0o555);
        std::fs::set_permissions(&ro_cwd, permissions).expect("降权");
    }
    std::env::set_current_dir(&ro_cwd).expect("切到只读 cwd");
    let paths = ModulePaths::from_env_for_project(None);
    let target = paths.export_midi.clone();
    assert!(
        target.is_absolute() && !target.starts_with(&ro_cwd),
        "缺省导出目标必须是 cwd 之外的绝对路径: {}",
        target.display()
    );
    assert_eq!(
        target,
        docs_root.join("ui-export.mid"),
        "缺省导出目标必须是文档目录 Yeban/ui-export.mid"
    );
    ensure_default_parent_dir(&target);
    assert!(docs_root.is_dir(), "缺省父目录必须已被建出来");
    let report =
        yeban_app::export_midi::export_project_to_file(&yeban_app::bridge::demo_project(), &target)
            .expect("只读 cwd 下缺省导出 MIDI 必须成功落到可写目录");
    assert_eq!(report.path, target, "回执路径必须是绝对缺省目标");
    assert!(report.bytes > 0, "导出的 MIDI 必须非空");
    assert!(
        !ro_cwd.join("ui-export.mid").exists(),
        "只读 cwd 里绝不能落下 ui-export.mid"
    );

    // ⑤ 录音同款：缺省目标绝对、父目录建出、0.1 秒测试音成功落盘。
    let record_target = paths.record_wav.clone();
    assert_eq!(
        record_target,
        docs_root.join("ui-record.wav"),
        "缺省录音目标必须是文档目录 Yeban/ui-record.wav"
    );
    ensure_default_parent_dir(&record_target);
    let options = yeban_app::record_wav::RecordOptions {
        seconds: 0.1,
        ..yeban_app::record_wav::RecordOptions::default()
    };
    let record = yeban_app::record_wav::record_test_tone(&record_target, &options)
        .expect("只读 cwd 下缺省录音必须成功落到可写目录");
    assert_eq!(record.path, record_target, "录音回执路径必须是绝对缺省目标");
    assert!(record.bytes > 0, "录下的 WAV 必须非空");

    // 恢复 cwd / 权限 / 环境（本二进制单测，仍按规矩还原）。
    std::env::set_current_dir(&saved_cwd).expect("恢复 cwd");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = std::fs::metadata(&ro_cwd).expect("元数据").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&ro_cwd, permissions).expect("还原权限");
    }
    // SAFETY: 同上 —— 单 `#[test]` 的进程。
    unsafe {
        for (name, value) in saved {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}
