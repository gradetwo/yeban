//! 旧 `.groove` ⇒ 新 `.yeban` 迁移器的**真二进制**判据（任务单 `#146`）。
//!
//! 这里只跑**无窗口**命令：`--migrate-groove … --save-as …` 与 `--headless --open …`。
//! 两条都是批处理路径，⛔ 不创建窗口、⛔ 不进事件循环。
//!
//! 判据覆盖（对应任务单 §4）：
//! 1. 迁移成功且**旧读数 == 新读数**（轨道 / 音符 / 片段 / 段落）；
//! 2. 旧文件 `sha256` 前后相同（只读）；
//! 3. 新容器可被 `--headless --open` 打开（rc 0 ＋ `project-counts:` 行）；
//! 4. 坏 / 缺字段 / 未知 `format` 的旧文件**响亮失败**，且⛔ 不覆盖已有目标；
//! 5. 输入 = 输出 / 缺 `--save-as` 都是用法错误（退出码 2）。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use yeban_model::ids::AssetHash;

/// 被测二进制的绝对路径（由 Cargo 注入）。
const BIN: &str = env!("CARGO_BIN_EXE_yeban-app");

/// 一次子进程执行允许的最长时间（与 `tests/cli_contract.rs` 同一条理由：
/// 万一某个"无窗口"开关回归进 GUI 路径，这里要**响亮失败**而不是把 CI 挂住）。
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

/// 一次子进程的全部可观测输出。
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn invoke_paths(args: &[PathBuf]) -> Run {
    let mut child = Command::new(BIN)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("无法执行 {BIN}: {error}"));
    let deadline = Instant::now() + RUN_TIMEOUT;
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break,
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("`yeban-app {args:?}` 在 {RUN_TIMEOUT:?} 内没有退出");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    let output = child.wait_with_output().expect("收集输出");
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-migrate-bin-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

fn sha256(path: &Path) -> String {
    let bytes = std::fs::read(path).expect("读文件");
    AssetHash::of_bytes(&bytes).to_string()
}

fn field(line: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(rest[..end].to_owned())
}

fn line_with<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.lines().find(|line| line.starts_with(prefix))
}

/// 一个最小的 v1 工程包：2 条轨 × 2 套 pattern × 16 步，共 **9** 个音符。
fn fixture() -> String {
    r#"{
        "format": "groove-project",
        "version": 1,
        "exportedAt": 1700000000000,
        "appVersion": "2.34.0",
        "project": {
            "id": "proj_fixture",
            "name": "迁移夹具",
            "genreId": "neo-soul",
            "genreName": "Neo-Soul",
            "bpm": 84,
            "swing": 30,
            "timeSignature": "4/4",
            "resolution": "1/16",
            "stepCount": 16,
            "patterns": {
                "A": {
                    "genre_id": "neo-soul",
                    "bpm": 84,
                    "scale": "F minor",
                    "swing": 45,
                    "tracks": [
                        {"track_id": "kick", "name": "Kick", "instrument": "acoustic_kick",
                         "steps": [1,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0],
                         "velocity": [120,0,0,0,0,0,0,0,105,0,0,0,0,0,0,0],
                         "volume": 0.9, "pan": 0.0},
                        {"track_id": "bass", "name": "Bass", "instrument": "finger_bass",
                         "steps": [1,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0],
                         "pitch": [41,null,44,null,null,null,null,null,null,null,null,null,null,null,null,null],
                         "volume": 0.9, "pan": 0.0}
                    ]
                },
                "B": {
                    "genre_id": "neo-soul",
                    "bpm": 84,
                    "scale": "F minor",
                    "swing": 45,
                    "tracks": [
                        {"track_id": "kick", "name": "Kick", "instrument": "acoustic_kick",
                         "steps": [1,0,0,0,1,0,0,0,1,0,0,0,0,0,0,0],
                         "velocity": [120,0,0,0,110,0,0,0,105,0,0,0,0,0,0,0],
                         "volume": 0.9, "pan": 0.0},
                        {"track_id": "bass", "name": "Bass", "instrument": "finger_bass",
                         "steps": [1,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0],
                         "pitch": [41,null,null,null,null,null,44,null,null,null,null,null,null,null,null,null],
                         "volume": 0.9, "pan": 0.0}
                    ]
                }
            },
            "activeSlot": "A",
            "songMode": true,
            "songChain": ["A", "B"],
            "sections": [
                {"id": "sec-1", "slot": "A", "bars": 1, "label": "intro"},
                {"id": "sec-2", "slot": "B", "bars": 2, "label": "chorus"}
            ],
            "loopRange": null,
            "effectsRack": {"filterEnabled": false},
            "drumKit": "acoustic",
            "isMetronome": false,
            "isCountIn": false,
            "tags": ["fixture"],
            "isFavorite": false,
            "notes": "迁移夹具说明",
            "createdAt": 1700000000000,
            "updatedAt": 1700000000001
        }
    }"#
    .to_owned()
}

/// 判据 1+2+3：迁移成功、旧读数 == 新读数、旧文件未被改、新容器可被 `--open` 打开。
#[test]
fn migration_keeps_the_counts_and_never_touches_the_old_file() {
    let dir = scratch_dir("roundtrip");
    let old = dir.join("old.groove");
    let new = dir.join("new.yeban");
    std::fs::write(&old, fixture()).expect("写旧夹具");
    let before = sha256(&old);
    let old_bytes = std::fs::read(&old).expect("读旧夹具");

    let run = invoke_paths(&[
        PathBuf::from("--migrate-groove"),
        old.clone(),
        PathBuf::from("--save-as"),
        new.clone(),
    ]);
    assert_eq!(run.code, 0, "stderr={}", run.stderr);
    assert!(
        run.stdout.lines().any(|line| line == "headless ok"),
        "迁移路径是无窗口路径，必须打印握手行:\n{}",
        run.stdout
    );
    let counts = line_with(&run.stdout, "migrated-groove-counts:")
        .unwrap_or_else(|| panic!("缺 migrated-groove-counts: 行:\n{}", run.stdout));
    assert_eq!(field(counts, "legacy-tracks").as_deref(), Some("2"));
    assert_eq!(
        field(counts, "migrated-tracks-musical").as_deref(),
        Some("2")
    );
    assert_eq!(field(counts, "legacy-notes").as_deref(), Some("9"));
    assert_eq!(field(counts, "migrated-notes").as_deref(), Some("9"));
    assert_eq!(field(counts, "legacy-clips").as_deref(), Some("4"));
    assert_eq!(field(counts, "migrated-clips").as_deref(), Some("4"));
    assert_eq!(field(counts, "legacy-sections").as_deref(), Some("2"));
    assert_eq!(field(counts, "migrated-sections").as_deref(), Some("2"));
    let saved = line_with(&run.stdout, "migrated: path=")
        .unwrap_or_else(|| panic!("缺 migrated: 行:\n{}", run.stdout));
    assert!(
        field(saved, "bytes")
            .unwrap_or_default()
            .parse::<u64>()
            .expect("字节数")
            > 0,
        "容器字节数必须为正: {saved}"
    );

    assert_eq!(sha256(&old), before, "旧文件 sha256 必须不变");
    assert_eq!(std::fs::read(&old).expect("读旧夹具"), old_bytes);
    assert!(new.exists(), "新容器必须存在");

    // 判据 4：新容器真的能打开，且计数行与迁移读数逐项相等。
    let opened = invoke_paths(&[
        PathBuf::from("--headless"),
        PathBuf::from("--open"),
        new.clone(),
    ]);
    assert_eq!(opened.code, 0, "stderr={}", opened.stderr);
    let project_counts = line_with(&opened.stdout, "project-counts:")
        .unwrap_or_else(|| panic!("缺 project-counts: 行:\n{}", opened.stdout));
    assert_eq!(field(project_counts, "tracks-all").as_deref(), Some("3"));
    assert_eq!(field(project_counts, "master-track").as_deref(), Some("1"));
    assert_eq!(field(project_counts, "clips-pool").as_deref(), Some("4"));
    assert_eq!(field(project_counts, "midi-notes").as_deref(), Some("9"));
    assert_eq!(field(project_counts, "sections").as_deref(), Some("2"));
    // 逐项相等：新容器的**音乐轨** = 旧轨道；总轨 = 旧轨 + 1 条新格式必需的主总线。
    assert_eq!(
        field(project_counts, "tracks-all"),
        Some("3".to_owned()),
        "旧 2 轨 + 1 主总线"
    );
    assert_eq!(
        field(project_counts, "midi-notes"),
        field(counts, "legacy-notes"),
        "音符数必须逐项相等"
    );
    assert_eq!(
        field(project_counts, "clips-pool"),
        field(counts, "legacy-clips"),
        "片段数必须逐项相等"
    );
    assert_eq!(
        field(project_counts, "sections"),
        field(counts, "legacy-sections"),
        "段落数必须逐项相等"
    );

    // 原子落盘不留下临时文件 / 锁文件。
    for entry in std::fs::read_dir(&dir).expect("列目录") {
        let name = entry.expect("目录项").file_name();
        let name = name.to_string_lossy();
        assert!(!name.contains(".tmp-"), "不得残留临时文件: {name}");
        assert!(!name.ends_with(".yeban.lock"), "不得残留锁文件: {name}");
    }

    let _ = std::fs::remove_dir_all(dir);
}

/// 判据 5（注入一）：缺字段的旧文件**响亮失败**，且⛔ 不覆盖已有目标、⛔ 不产半成品。
#[test]
fn a_missing_field_fails_loudly_and_never_overwrites_the_target() {
    let dir = scratch_dir("bad");
    let old = dir.join("broken.groove");
    let new = dir.join("target.yeban");
    std::fs::write(&old, fixture().replace("\"patterns\":", "\"patternz\":")).expect("写坏夹具");
    std::fs::write(&new, b"sentinel-not-a-container").expect("写哨兵目标");
    let before = sha256(&old);

    let run = invoke_paths(&[
        PathBuf::from("--migrate-groove"),
        old.clone(),
        PathBuf::from("--save-as"),
        new.clone(),
    ]);
    assert_eq!(run.code, 3, "stderr={}", run.stderr);
    assert!(
        run.stderr.contains("缺少字段 `project.patterns`"),
        "必须逐字点名缺的字段:\n{}",
        run.stderr
    );
    assert_eq!(
        std::fs::read(&new).expect("读目标"),
        b"sentinel-not-a-container",
        "坏输入不得覆盖已有目标"
    );
    assert_eq!(sha256(&old), before, "坏输入也不得改旧文件");
    let _ = std::fs::remove_dir_all(dir);
}

/// 判据 5（注入二）：`groove-package` 是**旧实现里没有读取器**的形状，必须按名拒绝。
#[test]
fn a_groove_package_is_rejected_by_name() {
    let dir = scratch_dir("package");
    let old = dir.join("unknown.groove");
    let new = dir.join("out.yeban");
    std::fs::write(
        &old,
        br#"{"format":"groove-package","schemaVersion":"2.34.16-dev","title":"x"}"#,
    )
    .expect("写夹具");

    let run = invoke_paths(&[
        PathBuf::from("--migrate-groove"),
        old.clone(),
        PathBuf::from("--save-as"),
        new.clone(),
    ]);
    assert_eq!(run.code, 3, "stderr={}", run.stderr);
    assert!(run.stderr.contains("groove-package"), "{}", run.stderr);
    assert!(run.stderr.contains("没有读取器"), "{}", run.stderr);
    assert!(!new.exists(), "拒绝时不得产半成品");
    let _ = std::fs::remove_dir_all(dir);
}

/// 输入 = 输出是用法错误（退出码 2），旧文件一个字节都不动。
#[test]
fn the_same_input_and_output_path_is_a_usage_error() {
    let dir = scratch_dir("same");
    let old = dir.join("old.groove");
    std::fs::write(&old, fixture()).expect("写旧夹具");
    let before = sha256(&old);

    let run = invoke_paths(&[
        PathBuf::from("--migrate-groove"),
        old.clone(),
        PathBuf::from("--save-as"),
        old.clone(),
    ]);
    assert_eq!(run.code, 2, "stderr={}", run.stderr);
    assert!(run.stderr.contains("同一个路径"), "{}", run.stderr);
    assert_eq!(sha256(&old), before);
    let _ = std::fs::remove_dir_all(dir);
}

/// 缺 `--save-as` 是用法错误（退出码 2），不默认落点。
#[test]
fn the_output_path_is_required() {
    let dir = scratch_dir("missing-output");
    let old = dir.join("old.groove");
    std::fs::write(&old, fixture()).expect("写旧夹具");
    let run = invoke_paths(&[PathBuf::from("--migrate-groove"), old]);
    assert_eq!(run.code, 2, "stderr={}", run.stderr);
    assert!(run.stderr.contains("必须配 `--save-as"), "{}", run.stderr);
    let _ = std::fs::remove_dir_all(dir);
}

/// 别的动作与迁移组合 = 用法错误（⛔ 不静默丢掉那个动作）。
#[test]
fn unrelated_switches_are_rejected_instead_of_ignored() {
    let dir = scratch_dir("conflict");
    let old = dir.join("old.groove");
    let new = dir.join("new.yeban");
    std::fs::write(&old, fixture()).expect("写旧夹具");
    let run = invoke_paths(&[
        PathBuf::from("--migrate-groove"),
        old,
        PathBuf::from("--save-as"),
        new,
        PathBuf::from("--export-midi"),
        PathBuf::from("x.mid"),
    ]);
    assert_eq!(run.code, 2, "stderr={}", run.stderr);
    assert!(
        run.stderr.contains("不能与 `--export-midi` 组合"),
        "{}",
        run.stderr
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// 本机那份唯一的真旧工程（`HD-64` 点名的 19,111 字节夹具）：8 轨 / 59 音符。
/// 其它机器没有它 ⇒ skip（不是失败）。
#[test]
fn the_real_legacy_project_migrates_when_present() {
    let old = Path::new("/Users/crow/Downloads/假如时光能够倒流.groove");
    if !old.exists() {
        eprintln!("skip: 本机没有真旧工程 {old:?}");
        return;
    }
    let dir = scratch_dir("real");
    let new = dir.join("real.yeban");
    let before = sha256(old);
    let run = invoke_paths(&[
        PathBuf::from("--migrate-groove"),
        old.to_path_buf(),
        PathBuf::from("--save-as"),
        new.clone(),
    ]);
    assert_eq!(run.code, 0, "stderr={}", run.stderr);
    let counts = line_with(&run.stdout, "migrated-groove-counts:").expect("计数行");
    assert_eq!(field(counts, "legacy-tracks").as_deref(), Some("8"));
    assert_eq!(field(counts, "legacy-notes").as_deref(), Some("59"));
    assert_eq!(
        field(counts, "migrated-tracks-musical").as_deref(),
        Some("8")
    );
    assert_eq!(field(counts, "migrated-tracks-all").as_deref(), Some("9"));
    assert_eq!(field(counts, "migrated-notes").as_deref(), Some("59"));
    assert_eq!(sha256(old), before, "旧文件 sha256 必须不变");

    let opened = invoke_paths(&[PathBuf::from("--headless"), PathBuf::from("--open"), new]);
    assert_eq!(opened.code, 0, "stderr={}", opened.stderr);
    let project_counts = line_with(&opened.stdout, "project-counts:").expect("project-counts");
    assert_eq!(field(project_counts, "tracks-all").as_deref(), Some("9"));
    assert_eq!(field(project_counts, "master-track").as_deref(), Some("1"));
    assert_eq!(field(project_counts, "midi-notes").as_deref(), Some("59"));
    assert_eq!(field(project_counts, "sections").as_deref(), Some("8"));
    let _ = std::fs::remove_dir_all(dir);
}
