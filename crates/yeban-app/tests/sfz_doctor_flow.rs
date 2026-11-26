//! SFZ Doctor 接线事实 + 判据往返：
//! ① 对话框有「体检」两步确认、宿主有两条回调的落点；
//! ② 非官方库的状态机（Needed → Done / Declined；官方库恒 Official）；
//! ③ 真实修复往返：一件大小写路径 + 越界 tune 的假库，跑完必须真能装载。

use std::fs;
use std::path::{Path, PathBuf};

use yeban_app::sfz_doctor::{
    DoctorState, doctor_state, format_doctor_readout, mark_declined, quarantine_corrupt_record, run,
};
use yeban_app::sound_library::{InstalledLibrary, PACK_MARKER_FILE};

const DIALOG: &str = include_str!("../ui/dialogs/sound_library_dialog.slint");
const APP: &str = include_str!("../ui/app.slint");
const HOST: &str = include_str!("../src/host.rs");

#[test]
fn dialog_and_host_wire_doctor_callbacks() {
    for needle in [
        "callback doctor-check(string);",
        "callback doctor-decline(string);",
        "sound-library-doctor-button",
        "sound-library-doctor-confirm-button",
        "sound-library-doctor-decline-button",
    ] {
        assert!(DIALOG.contains(needle), "对话框缺少：{needle}");
    }
    for needle in [
        "sound-installed-doctor-needed: [];",
        "installed-doctor-needed: root.sound-installed-doctor-needed",
        "doctor-check(id) => { root.sound-library-doctor-check(id); }",
        "doctor-decline(id) => { root.sound-library-doctor-decline(id); }",
    ] {
        assert!(APP.contains(needle), "app.slint 缺少：{needle}");
    }
    for needle in [
        "ui.on_sound_library_doctor_check",
        "ui.on_sound_library_doctor_decline",
        "crate::sfz_doctor::run(",
        "crate::sfz_doctor::guard(",
        "DoctorRunStatus::Refused",
        "library_id.as_str(), library_path.as_path()",
        "crate::sfz_doctor::mark_declined(",
        "set_sound_installed_doctor_needed",
        // T-FIX-306：跳过记录落盘失败必须如实报错，坏记录要隔离备份、行内仍提示。
        "无法保存",
        "下次仍会提示",
        "record_error.is_some()",
        "quarantine_corrupt_record",
        "DoctorState::Corrupt",
    ] {
        assert!(HOST.contains(needle), "host 缺少：{needle}");
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-sfz-doctor-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 一段最小可解码的 16-bit 单声道 WAV（10 个采样）。
fn write_tiny_wav(path: &Path) {
    let mut bytes: Vec<u8> = Vec::new();
    let data_len: u32 = 20;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&48_000u32.to_le_bytes());
    bytes.extend_from_slice(&96_000u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..10i16 {
        bytes.extend_from_slice(&(i * 100).to_le_bytes());
    }
    fs::write(path, bytes).expect("写 WAV");
}

fn library(id: &str, location: PathBuf) -> InstalledLibrary {
    InstalledLibrary {
        id: id.to_string(),
        name: "ext-lib".to_string(),
        location,
        size_bytes: 0,
        deletable: true,
    }
}

/// 造一件坏库：采样目录大小写不对（Samples/flute vs 磁盘 Samples/Flute）
/// + region 里 tune=-109 越界。返回库根。
fn make_broken_library(base: &Path) -> PathBuf {
    let lib = base.join("ext-lib");
    let samples = lib.join("Samples").join("Flute");
    fs::create_dir_all(&samples).expect("建采样目录");
    write_tiny_wav(&samples.join("take1.wav"));
    // SFZ 放在库根（与真实库同一布局：default_path 相对 SFZ 所在目录）。
    fs::write(
        lib.join("test.sfz"),
        "<control>\ndefault_path=Samples/flute/\n<region> sample=take1.wav key=60 tune=-109\n",
    )
    .expect("写 SFZ");
    lib
}

/// 本机文件系统是否大小写敏感：写一个混合大小写探针文件，再以全小写名看它在不在。
/// macOS APFS 默认与 Windows NTFS 不敏感——错大小写的路径在那种盘上本身就能解析，
/// 不是坏路径；Linux ext4 敏感，错大小写就是不存在。
fn fs_is_case_sensitive(dir: &Path) -> bool {
    let probe = dir.join("CaSeSeNsItIvItY.tmp");
    fs::write(&probe, b"x").expect("写大小写探针");
    let insensitive = dir.join("casesensitivity.tmp").exists();
    let _ = fs::remove_file(&probe);
    !insensitive
}

#[test]
fn external_library_state_machine_and_repair_roundtrip() {
    let base = temp_dir("roundtrip");
    let lib_dir = make_broken_library(&base);
    let lib = library("user:ext-lib", lib_dir.clone());

    // 未表态 ⇒ Needed。
    assert_eq!(doctor_state(&lib), DoctorState::Needed);

    // 跑体检：大小写链接数与本机文件系统的大小写敏感性一致、tune 钳值 ≥1、
    // 修完必须真能装载（探针口径）。大小写不敏感的盘上错大小写路径本身可解析，
    // 体检无链接可建、也不需要建——两种都对，只钉本机文件系统的真值。
    let report = run("user:ext-lib", &lib_dir).expect("Needed 库应允许体检");
    let expected_links = if fs_is_case_sensitive(&base) { 1 } else { 0 };
    assert_eq!(
        report.links_created.len(),
        expected_links,
        "大小写链接数应与文件系统大小写敏感性一致：{report:?}"
    );
    assert!(
        report
            .edits
            .iter()
            .any(|e| e.opcode == "tune" && e.new == "-100"),
        "tune 应被钳到 -100：{report:?}"
    );
    assert!(
        report.still_failing.is_empty(),
        "修完不应再有装不上的：{:?}",
        report.still_failing
    );
    let readout = format_doctor_readout("ext-lib", &report);
    assert!(readout.contains("全部可装载"), "读数应报全好：{readout}");
    assert!(
        report.record_error.is_none(),
        "记录落盘成功时 record_error 应为空：{report:?}"
    );

    // 记录已落盘 ⇒ Done，不再提示。
    assert!(lib_dir.join(".sfz-doctor.json").exists());
    assert_eq!(doctor_state(&lib), DoctorState::Done);

    // 记录已落盘 ⇒ Done：执行层拒绝重复检查（不重跑、不再写文件）。
    let refusal = run("user:ext-lib", &lib_dir).expect_err("Done 库必须拒绝重复体检");
    assert_eq!(refusal.state(), DoctorState::Done);

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn declined_and_official_are_never_prompted() {
    let base = temp_dir("states");
    let lib_dir = make_broken_library(&base);
    let lib = library("user:ext-lib", lib_dir.clone());

    mark_declined(&lib_dir).expect("跳过记录应能落盘");
    assert_eq!(doctor_state(&lib), DoctorState::Declined);

    // Declined 不是可修复状态：执行层必须拒绝重跑，记录保持原样。
    let refusal = run("user:ext-lib", &lib_dir).expect_err("Declined 库必须拒绝体检");
    assert_eq!(refusal.state(), DoctorState::Declined);
    let record = fs::read_to_string(lib_dir.join(".sfz-doctor.json")).expect("跳过记录仍在");
    assert!(
        record.contains("declined"),
        "拒绝体检不得改写跳过记录：{record}"
    );

    // 带包标记 ⇒ 官方库，恒 Official（哪怕没有体检记录）。
    fs::write(lib_dir.join(".yeban-pack.json"), "{}").expect("写包标记");
    let _ = fs::remove_file(lib_dir.join(".sfz-doctor.json"));
    assert_eq!(doctor_state(&lib), DoctorState::Official);

    // 内置 id 同样恒 Official。
    let builtin = library("builtin:ext-lib", lib_dir.clone());
    assert_eq!(doctor_state(&builtin), DoctorState::Official);

    let _ = fs::remove_dir_all(&base);
}

/// T-FIX-618：执行层硬守卫 —— 官方库（带包标记 / 内置 id）调用 `run`
/// 必须被拒绝，且**一个字节都不写**：无链接、无记录、无操作码改动。
#[test]
fn official_library_run_is_refused_without_writes() {
    let base = temp_dir("official-guard");
    let lib_dir = make_broken_library(&base);
    let lib = library("user:ext-lib", lib_dir.clone());
    fs::write(lib_dir.join(PACK_MARKER_FILE), "{}").expect("写包标记");
    assert_eq!(doctor_state(&lib), DoctorState::Official);

    let sfz_before = fs::read(lib_dir.join("test.sfz")).expect("读原始 SFZ");
    let entries_before: Vec<_> = fs::read_dir(&lib_dir)
        .expect("读库根")
        .flatten()
        .map(|e| e.file_name())
        .collect();
    let refusal = run("user:ext-lib", &lib_dir).expect_err("官方库必须拒绝体检");
    assert_eq!(refusal.state(), DoctorState::Official);
    assert_eq!(refusal.reason(), "officialLibrary");
    assert_eq!(
        fs::read(lib_dir.join("test.sfz")).expect("读原 SFZ"),
        sfz_before,
        "官方库的 SFZ 文本不得被改动"
    );
    assert!(
        !lib_dir.join(".sfz-doctor.json").exists(),
        "官方库不得写体检记录"
    );
    let entries_after: Vec<_> = fs::read_dir(&lib_dir)
        .expect("读库根")
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(entries_before, entries_after, "官方库根不得新增任何条目");
    let wrong_case = lib_dir.join("Samples/flute/take1.wav");
    assert!(
        fs::symlink_metadata(&wrong_case).map_or(true, |m| !m.file_type().is_symlink()),
        "官方库不得建大小写链接：{wrong_case:?}"
    );

    // 内置 id 同样拒绝（哪怕路径上没有包标记）。
    let base2 = temp_dir("builtin-guard");
    let builtin_dir = make_broken_library(&base2);
    let sfz_before = fs::read(builtin_dir.join("test.sfz")).expect("读原始 SFZ");
    let refusal = run("builtin:ext-lib", &builtin_dir).expect_err("内置 id 必须拒绝体检");
    assert_eq!(refusal.state(), DoctorState::Official);
    assert_eq!(
        fs::read(builtin_dir.join("test.sfz")).expect("读原 SFZ"),
        sfz_before
    );
    assert!(
        !builtin_dir.join(".sfz-doctor.json").exists(),
        "内置库不得写体检记录"
    );

    let _ = fs::remove_dir_all(&base);
    let _ = fs::remove_dir_all(&base2);
}

/// T-FIX-306：跳过记录落盘失败必须返回 Err，且状态绝不能变成 Declined。
///
/// 本机以 root 运行，`chmod` 只读目录仍能写 —— 失败目标必须真写不进去：
/// ① 根本不存在的库目录（`fs::write` 报 ENOENT）；② 记录路径本身已被占成
/// 目录（`fs::write` 报 EISDIR，root 也写不进去）。两种都是真失败。
#[test]
fn declined_record_write_failure_is_reported_and_still_prompts() {
    let base = temp_dir("decline-fail");

    // ① 库目录不存在：写失败，仍是 Needed（从未表态），绝不"不再提示"。
    let missing_dir = base.join("no-such-lib");
    let missing_lib = library("user:no-such-lib", missing_dir.clone());
    let err = mark_declined(&missing_dir).expect_err("往不存在的目录写记录必须失败");
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(doctor_state(&missing_lib), DoctorState::Needed);

    // ② 记录路径被占成目录：写失败；读记录也失败 ⇒ Corrupt（不是 Declined）。
    let lib_dir = make_broken_library(&base);
    let lib = library("user:ext-lib", lib_dir.clone());
    fs::create_dir_all(lib_dir.join(".sfz-doctor.json")).expect("把记录路径占成目录");
    let err = mark_declined(&lib_dir).expect_err("记录路径是目录时写记录必须失败");
    assert_eq!(err.kind(), std::io::ErrorKind::IsADirectory);
    assert_eq!(doctor_state(&lib), DoctorState::Corrupt);
    assert!(
        !lib_dir.join(".sfz-doctor.json").is_file(),
        "失败时不得留下一份假的 Declined 记录"
    );

    let _ = fs::remove_dir_all(&base);
}

/// T-FIX-306：体检跑完了，但记录写不进去 ⇒ 报告带 record_error、读数带警告、
/// 状态不是 Done（下次仍会提示）。
#[test]
fn run_record_write_failure_is_visible_in_report() {
    let base = temp_dir("run-fail");
    let lib_dir = make_broken_library(&base);
    let lib = library("user:ext-lib", lib_dir.clone());
    fs::create_dir_all(lib_dir.join(".sfz-doctor.json")).expect("把记录路径占成目录");

    let report = run("user:ext-lib", &lib_dir).expect("Needed 库应允许体检");
    assert!(
        report.record_error.is_some(),
        "记录写失败时 record_error 必须带上：{report:?}"
    );
    let readout = format_doctor_readout("ext-lib", &report);
    assert!(
        readout.contains("记录未能保存"),
        "读数必须带记录失败的警告：{readout}"
    );
    assert!(
        readout.contains("下次可能再次提示"),
        "读数必须说明下次仍会提示：{readout}"
    );
    // 体检本身的修复仍发生了，但状态绝不能是 Done（记录没存住）。
    assert_eq!(doctor_state(&lib), DoctorState::Corrupt);

    let _ = fs::remove_dir_all(&base);
}

/// T-FIX-306：记录文件存在但解析失败 ⇒ Corrupt；隔离备份一次后回到 Needed，
/// 坏记录留着备查，不删除；重复隔离返回 None。
#[test]
fn corrupt_record_is_quarantined_and_reprompts() {
    let base = temp_dir("corrupt");
    let lib_dir = make_broken_library(&base);
    let lib = library("user:ext-lib", lib_dir.clone());
    fs::write(lib_dir.join(".sfz-doctor.json"), "{ 不是 json").expect("写坏记录");

    assert_eq!(doctor_state(&lib), DoctorState::Corrupt);

    let backup = quarantine_corrupt_record(&lib_dir).expect("坏记录应被隔离备份");
    assert!(backup.exists(), "备份文件应存在：{backup:?}");
    assert!(
        backup
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .starts_with(".sfz-doctor.json.corrupt-"),
        "备份应以 .sfz-doctor.json.corrupt- 开头：{backup:?}"
    );
    assert!(
        !lib_dir.join(".sfz-doctor.json").exists(),
        "隔离后原坏记录应已改名"
    );
    assert_eq!(
        doctor_state(&lib),
        DoctorState::Needed,
        "隔离后回到 Needed，下次提示体检"
    );
    assert!(
        quarantine_corrupt_record(&lib_dir).is_none(),
        "没有坏记录时隔离应返回 None"
    );

    // 好记录不受隔离影响。
    mark_declined(&lib_dir).expect("好记录应能落盘");
    assert_eq!(doctor_state(&lib), DoctorState::Declined);
    assert!(
        quarantine_corrupt_record(&lib_dir).is_none(),
        "好记录不应被隔离"
    );

    let _ = fs::remove_dir_all(&base);
}

/// T-FIX-618：Corrupt 是「需要先给可见警告的待检查库」，执行层仍允许体检；
/// 拒绝只针对 Official / Done / Declined，不能把 Corrupt 一并挡掉。
#[test]
fn corrupt_library_is_still_repairable() {
    let base = temp_dir("corrupt-run");
    let lib_dir = make_broken_library(&base);
    let lib = library("user:ext-lib", lib_dir.clone());
    fs::write(lib_dir.join(".sfz-doctor.json"), "{ 不是 json").expect("写坏记录");
    assert_eq!(doctor_state(&lib), DoctorState::Corrupt);

    let report = run("user:ext-lib", &lib_dir).expect("Corrupt 库应允许体检");
    assert!(
        report.record_error.is_none(),
        "坏记录被正常记录覆盖时应无 record_error：{report:?}"
    );
    assert!(
        report.edits.iter().any(|e| e.opcode == "tune"),
        "Corrupt 库的体检仍应执行修复：{report:?}"
    );
    assert_eq!(
        doctor_state(&lib),
        DoctorState::Done,
        "体检成功后坏记录应被合法记录取代"
    );

    let _ = fs::remove_dir_all(&base);
}
