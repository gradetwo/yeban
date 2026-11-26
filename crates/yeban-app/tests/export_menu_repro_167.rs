//! T-FIX-167 复现：文件菜单「导出 MIDI 文件...」与「导出分轨 (Stems)...」
//! 在活窗口里走**真实菜单入口**（`command-execute` → `dispatch_aether_command` /
//! `on_command_execute`）的表现。
//!
//! 装配与 `main.rs::run_gui` 同序：先接模块入口（`wire_module_actions_with`），
//! 最后 `wire_aether_actions` 用 `on_*` 覆盖语义接管（与
//! `tests/aether_callback_matrix.rs` 同款）。⚠ `build_live_ui_with` 不调
//! `wire_aether_actions`，只用它装窗口会在这里读到「菜单点了没反应」的假象 ——
//! 那是装配缺口，不是产品缺陷；本文件显式补上生产那一步。
//!
//! 三条臂各记一条 `[167-repro]` 读数（回执原文 / 人话行 / toast 原文 / 落盘文件）：
//! - 臂 1：导出 MIDI，缺省路径（`ui-export.mid` 相对 cwd）。判据只认两件事 ——
//!   文件真落盘 **且** 人话行是「导出 MIDI 完成」+ 回执带 `path=`（成功分支维持
//!   现状、人话行不缀路径；路径在回执字段与成功提示条里）。
//! - 臂 2：导出 MIDI，目标落在**不存在的目录**（写盘必败）。判据只认一件事 ——
//!   人话行必须带出失败原因，⛔ 不能只剩「上一步操作没做成」。
//! - 臂 3：导出分轨。工作线程跑完才落盘 ⇒ 等 `stems-manifest.json` 出现
//!   （有界轮询：分轨线程没有可 join 的句柄，这是唯一观测法；60 秒上限）。
//!   ⚠ Tier-1 平台没有 `new_event_loop_proxy` ⇒
//!   `upgrade_in_event_loop` 在这里恒返回 `Err`，工作线程的 toast 回调
//!   **跑不到** UI 线程 —— toast 读数只记录、不断言；"分轨真能导"的证据是
//!   落盘文件本身。toast 能不能到，只在真机事件循环上可证，本文件如实留白。

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use yeban_app::bridge::ViewState;
use yeban_app::engine_host::EngineHost;
use yeban_app::host;
use yeban_app::host::{ModulePaths, ProductionLoop};
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::YebanProjectV1;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW_MS: u64 = 1_700_000_000_000;

/// 两个测试进程若同时跑本文件与分轨镜像，产品回退目录 `temp_dir()/yeban-stems`
/// 是同一份；本文件内串行，跨文件由各测试进程自己的锁兜住同进程并发。
static STEMS_DIR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 把本测试进程的 profile 指到进程唯一根，产品 `host::stems_output_dir`
/// 与实际断言就都落在同一份私有目录，不再与别的测试二进制抢
/// `temp_dir()/yeban-stems`。
fn isolate_stems_dir() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let root = std::env::temp_dir().join(format!("yeban-stems-{}", std::process::id()));
        // SAFETY: 只在本进程第一次碰分轨目录前写一次；本文件没有别的 USERPROFILE 读者。
        unsafe { std::env::set_var("USERPROFILE", root) };
    });
}

/// 分轨输出目录：与生产 `host::stems_output_dir` **同一来源**。
fn stems_dir() -> PathBuf {
    isolate_stems_dir();
    host::stems_output_dir()
}

fn clear_stems_dir() {
    let dir = stems_dir();
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("清理分轨输出目录");
    }
}

fn list_files(dir: &PathBuf) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.push((name, len));
        }
    }
    out.sort();
    out
}

#[test]
fn menu_export_midi_and_stems_reproduction() {
    let _guard = STEMS_DIR_LOCK.lock().expect("分轨目录锁必须能拿到");
    behavior_arm();
    // ⭐ 臂 3b 在另一条线程起第二个活窗口（平台是线程局部的，每线程只能装一次 ——
    // 与 `tests/import_midi_losses_ui.rs` 的行为臂/几何臂同款）。
    let handle = std::thread::spawn(stems_project_dir_arm);
    handle.join().expect("分轨工程目录臂的线程必须正常结束");
}

/// 臂 1 / 1b / 2 / 3：菜单导出 MIDI（缺省路径、工程目录档、失败）与分轨。
fn behavior_arm() {
    let scratch = std::env::temp_dir().join(format!(
        "yeban-167-repro-{}-{}",
        std::process::id(),
        yeban_model::ids::EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&scratch).expect("建临时目录");
    clear_stems_dir();
    let cwd = std::env::current_dir().expect("读 cwd");
    let default_mid = cwd.join("ui-export.mid");
    let _ = std::fs::remove_file(&default_mid);

    let project: YebanProjectV1 = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<167-repro>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    let production = ProductionLoop::start(&project, 0).expect("引擎重建");
    let engine = production.engine_handle();
    let view_cell = Rc::new(RefCell::new(view.clone()));

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();

    // 生产同序：模块入口先接，分派层最后覆盖。
    let _ = host::wire_module_actions_with(
        ui,
        Some(Rc::clone(&undo)),
        ModulePaths::default(),
        Rc::new(yeban_app::file_picker::NoPicker),
    );
    host::wire_aether_actions(
        ui,
        Rc::clone(&undo),
        Rc::clone(&engine),
        Rc::clone(&view_cell),
    );

    assert!(
        undo.try_project().is_some(),
        "复现前提：撤销端口必须握着活跃工程"
    );
    // ⭐ 三臂各自记账、最后统一判：臂 1 红时臂 2/3 仍要跑（否则复现记录只有一半）。
    let mut failures: Vec<String> = Vec::new();

    // ---- 臂 1：菜单导出 MIDI（缺省相对路径）----
    ui.invoke_command_execute("export-midi".into());
    let receipt1 = ui.get_module_status().to_string();
    let text1 = ui.get_module_status_text().to_string();
    let ok1 = ui.get_module_status_succeeded();
    let toast1 = ui.get_toast_message().to_string();
    let landed1 = std::fs::metadata(&default_mid).ok().map(|m| m.len());
    eprintln!(
        "[167-repro 臂1] succeeded={ok1} receipt={receipt1} text={text1:?} toast={toast1:?} file={default_mid:?} bytes={landed1:?}"
    );
    if !ok1 {
        failures.push(format!("臂 1：缺省路径导出 MIDI 必须成功: {receipt1}"));
    }
    if !landed1.is_some_and(|len| len > 0) {
        failures.push(format!(
            "臂 1：ui-export.mid 必须落盘且非空: {default_mid:?}"
        ));
    }
    if !text1.starts_with("导出 MIDI 完成") {
        failures.push(format!(
            "臂 1：人话行必须是「导出 MIDI 完成」，实得 {text1:?}（回执 {receipt1}）"
        ));
    }
    // 成功分支维持现状（人话行不缀路径）；路径在结构化回执 `path=` 字段与成功
    // 提示条里 —— 简报的「带路径」指回执带路径，不是人话行。
    if !receipt1.contains("path=ui-export.mid") {
        failures.push(format!("臂 1：回执必须带 path=ui-export.mid: {receipt1}"));
    }

    // ---- 臂 1b：菜单导出 MIDI（工程目录档：从文件打开的工程 ⇒ 落工程目录）----
    // ⭐ T-FIX-167 修法 2 的端到端：`from_env_for_project(Some(dir))` 必须让 MIDI
    // 与 WAV 同款走「工程目录」这一档，而不是相对 cwd。
    let project_mid = scratch.join("ui-export.mid");
    let _ = host::wire_module_actions_with(
        ui,
        Some(Rc::clone(&undo)),
        ModulePaths::from_env_for_project(Some(&scratch)),
        Rc::new(yeban_app::file_picker::NoPicker),
    );
    ui.invoke_command_execute("export-midi".into());
    let receipt1b = ui.get_module_status().to_string();
    let text1b = ui.get_module_status_text().to_string();
    let ok1b = ui.get_module_status_succeeded();
    let landed1b = std::fs::metadata(&project_mid).ok().map(|m| m.len());
    eprintln!(
        "[167-repro 臂1b] succeeded={ok1b} receipt={receipt1b} text={text1b:?} file={project_mid:?} bytes={landed1b:?}"
    );
    if !ok1b {
        failures.push(format!("臂 1b：工程目录档导出 MIDI 必须成功: {receipt1b}"));
    }
    if !landed1b.is_some_and(|len| len > 0) {
        failures.push(format!(
            "臂 1b：ui-export.mid 必须落在工程目录且非空: {project_mid:?}"
        ));
    }
    if !text1b.starts_with("导出 MIDI 完成") {
        failures.push(format!(
            "臂 1b：人话行必须是「导出 MIDI 完成」，实得 {text1b:?}（回执 {receipt1b}）"
        ));
    }
    if !receipt1b.contains("path=") || !receipt1b.contains("ui-export.mid") {
        failures.push(format!("臂 1b：回执必须带工程目录下的 path: {receipt1b}"));
    }

    // ---- 臂 2：菜单导出 MIDI（目标目录不存在 ⇒ 写盘必败）----
    let bad_target = scratch.join("missing-dir").join("ui-export.mid");
    let _ = host::wire_module_actions_with(
        ui,
        Some(Rc::clone(&undo)),
        ModulePaths {
            export_midi: bad_target.clone(),
            ..ModulePaths::default()
        },
        Rc::new(yeban_app::file_picker::NoPicker),
    );
    ui.invoke_command_execute("export-midi".into());
    let receipt2 = ui.get_module_status().to_string();
    let text2 = ui.get_module_status_text().to_string();
    let ok2 = ui.get_module_status_succeeded();
    eprintln!("[167-repro 臂2] succeeded={ok2} receipt={receipt2} text={text2:?}");
    if ok2 {
        failures.push(format!(
            "臂 2：目标目录不存在时导出 MIDI 必须失败: {receipt2}"
        ));
    }
    if !receipt2.contains("outcome=error") {
        failures.push(format!("臂 2：失败回执必须带 outcome=error: {receipt2}"));
    }
    if !text2.contains("missing-dir") {
        failures.push(format!(
            "臂 2：人话行必须带出失败原因（缺失的目录名），实得 {text2:?}（回执 {receipt2}）"
        ));
    }
    // ⭐ 原因里带 `:` 时也不许拦腰截断（`field()` 引号感知）。
    if !text2.contains("No such file or directory") {
        failures.push(format!(
            "臂 2：人话行必须带出完整原因（含 `No such file or directory`），实得 {text2:?}（回执 {receipt2}）"
        ));
    }

    // ---- 臂 3：菜单导出分轨 ----
    ui.invoke_command_execute("export-stems".into());
    let manifest = stems_dir().join("stems-manifest.json");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !manifest.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
    }
    let files = list_files(&stems_dir());
    let toast3 = ui.get_toast_message().to_string();
    eprintln!(
        "[167-repro 臂3] manifest={manifest:?} exists={} files={files:?} toast={toast3:?} (Tier-1 上 toast 回调跑不到，见文件头)",
        manifest.exists()
    );
    if !manifest.exists() {
        failures.push(format!(
            "臂 3：分轨 manifest 必须落盘（60 秒内），目录内容 {files:?}"
        ));
    }
    let wavs: Vec<_> = files.iter().filter(|(n, _)| n.ends_with(".wav")).collect();
    if wavs.is_empty() {
        failures.push(format!(
            "臂 3：至少一条分轨 WAV 必须落盘，目录内容 {files:?}"
        ));
    }
    if !wavs.iter().all(|(_, len)| *len > 0) {
        failures.push(format!("臂 3：分轨 WAV 必须非空，目录内容 {files:?}"));
    }

    // 清理：不留产物污染仓库与 tmp。
    let _ = std::fs::remove_file(&default_mid);
    let _ = std::fs::remove_dir_all(&scratch);
    clear_stems_dir();

    assert!(
        failures.is_empty(),
        "T-FIX-167 复现判据未全过（{} 条）：\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// 臂 3b：菜单导出分轨，工程从文件打开（`project_dir` 已知）+ Lead 轨绑了
/// 工程旁的相对 SFZ（`lead.sfz`，`sample=*silence`）。
///
/// ⭐ T-FIX-167 修法 3 的端到端：分轨臂必须用真实工程目录当 `project_path` ——
/// 占位那一档解析不到工程旁的 SFZ（见下面纯后端臂的红），真实目录才能导出。
/// 引擎用 `start_with_audio_source_and_project_dir` 启动（目录在 `reload` 之前
/// 写下，生产同款顺序），分轨臂点击时现读 `engine.project_dir()`。
fn stems_project_dir_arm() {
    use yeban_model::project::SamplerBinding;

    const ONE_REGION_SILENCE: &str = "<region> sample=*silence key=60 pitch_keycenter=60\n";
    let scratch = std::env::temp_dir().join(format!(
        "yeban-167-repro-3b-{}-{}",
        std::process::id(),
        yeban_model::ids::EntityId::new().to_canonical_string()
    ));
    let proj_dir = scratch.join("proj");
    std::fs::create_dir_all(&proj_dir).expect("建工程目录");
    std::fs::write(proj_dir.join("lead.sfz"), ONE_REGION_SILENCE).expect("写 SFZ 夹具");

    let mut project: YebanProjectV1 = yeban_model::samples::filled_project();
    let lead = project
        .tracks
        .values_mut()
        .find(|track| track.name == "Lead")
        .expect("filled_project 必须有 Lead 轨");
    lead.sampler = Some(SamplerBinding {
        path: "lead.sfz".to_owned(),
        note_octave_override: None,
    });

    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<167-repro-3b>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    let production =
        ProductionLoop::start_with_audio_source_and_project_dir(&project, 0, None, Some(&proj_dir))
            .expect("带工程目录启动引擎");
    let engine = production.engine_handle();
    // ⭐ T-FIX-422：真实工程文件名（不是 `project.yeban` 合成名）写进宿主；
    // 分轨臂点击时现读它，`guard_output_path` 才护得住真实工程文件/锁文件。
    let project_file = proj_dir.join("Song.yeban");
    std::fs::write(&project_file, b"fixture project").expect("写工程文件占位");
    engine
        .borrow_mut()
        .set_project_path(Some(project_file.clone()));
    assert_eq!(
        engine.borrow().project_path(),
        Some(project_file.as_path()),
        "臂 3b 前提：引擎必须记着真实工程文件名"
    );
    let view_cell = Rc::new(RefCell::new(view.clone()));

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();
    let _ = host::wire_module_actions_with(
        ui,
        Some(Rc::clone(&undo)),
        ModulePaths::default(),
        Rc::new(yeban_app::file_picker::NoPicker),
    );
    host::wire_aether_actions(
        ui,
        Rc::clone(&undo),
        Rc::clone(&engine),
        Rc::clone(&view_cell),
    );
    assert!(
        undo.try_project().is_some(),
        "臂 3b 前提：撤销端口必须握着活跃工程"
    );
    assert_eq!(
        engine.borrow().project_dir(),
        Some(proj_dir.as_path()),
        "臂 3b 前提：引擎必须记着真实工程目录"
    );

    clear_stems_dir();
    ui.invoke_command_execute("export-stems".into());
    let manifest = stems_dir().join("stems-manifest.json");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !manifest.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
    }
    let files = list_files(&stems_dir());
    let toast = ui.get_toast_message().to_string();
    eprintln!(
        "[167-repro 臂3b] manifest exists={} files={files:?} toast={toast:?} (Tier-1 上 toast 回调跑不到，见文件头)",
        manifest.exists()
    );
    assert!(
        manifest.exists(),
        "臂 3b：真实工程目录下分轨 manifest 必须落盘（60 秒内），目录内容 {files:?}"
    );
    let wavs: Vec<_> = files.iter().filter(|(n, _)| n.ends_with(".wav")).collect();
    assert!(
        !wavs.is_empty(),
        "臂 3b：至少一条分轨 WAV 必须落盘，目录内容 {files:?}"
    );
    assert!(
        wavs.iter().all(|(_, len)| *len > 0),
        "臂 3b：分轨 WAV 必须非空，目录内容 {files:?}"
    );

    let _ = std::fs::remove_dir_all(&scratch);
    clear_stems_dir();
}

#[test]
fn stems_project_path_unsaved_falls_back_to_cwd_not_output_dir() {
    // 未保存工程：没有真实工程路径 ⇒ 用 cwd（CLI 样本工程同一口径），
    // ⛔ 不再拿 out_dir.join("project.yeban") 占位。
    let empty = EngineHost::new();
    let cwd = std::env::current_dir().expect("读 cwd");
    assert_eq!(
        host::stems_project_path(&empty),
        cwd,
        "未保存工程的分轨 project_path 必须回退到 cwd，而不是输出目录占位"
    );
    let mut saved = EngineHost::new();
    let saved_path = std::env::temp_dir().join("Song.yeban");
    saved.set_project_path(Some(saved_path.clone()));
    assert_eq!(
        host::stems_project_path(&saved),
        saved_path,
        "有磁盘落点时必须返回真实工程文件名"
    );
}

/// 后端护栏臂：`project_path` 是真实工程文件名，某个分轨落点与它同路径时，
/// 预检必须给出“输出路径不能是工程文件本身”，而不是渲染后覆盖。
#[test]
fn export_stems_guard_rejects_the_real_project_file_path() {
    let scratch = std::env::temp_dir().join(format!(
        "yeban-422-stems-guard-backend-{}-{}",
        std::process::id(),
        yeban_model::ids::EntityId::new().to_canonical_string()
    ));
    let out_dir = scratch.join("stems");
    std::fs::create_dir_all(&out_dir).expect("建分轨输出目录");
    let project: YebanProjectV1 = yeban_model::samples::filled_project();
    let project_path = out_dir.join("0001.Lead.wav");
    let error = yeban_app::export_stems::export_stems(&project, &project_path, &out_dir, None, &[])
        .expect_err("输出等于真实工程文件时必须被 guard 拒绝");
    let text = format!("{error}");
    assert!(
        text.contains("输出路径不能是工程文件本身"),
        "必须点明工程文件护栏: {text}"
    );
    assert!(!project_path.exists(), "不能把工程文件写成输出");
    let _ = std::fs::remove_dir_all(&scratch);
}

/// 臂 3c：真实工程文件名与某个分轨落点重合 ⇒ `guard_output_path` 必须响亮拒绝。
///
/// 构造方式：把工程路径设成 `out_dir/0001.Lead.wav`（filled_project 的第一条
/// 非 master 轨是 Lead，分轨文件名按 ADR-0030 D7 是 `0001.Lead.wav`）。
/// - 修前：分轨臂把 `project_path` 合成成 `out_dir/project.yeban`，护栏比对不到，
///   会把分轨写进这个路径；
/// - 修后：`project_path` 是真实工程文件名 ⇒ 落点预检在渲染前拒绝，manifest
///   和分轨文件都不出现。
#[test]
fn stems_guard_sees_the_real_project_file_name() {
    // ⭐ T-FIX-778：本判据必须拿 STEMS_DIR_LOCK —— 它与主判据共用同一份
    // 进程级分轨目录（`isolate_stems_dir` 的 USERPROFILE 隔离只分进程、不分
    // 判据），且它当"工程文件"的 `0001.Lead.wav` 正是臂 3 分轨的第一个落点
    // 文件名（Lead 是第一条非 master 轨）。不串行时两面都误判：臂 3 的分轨
    // 先落盘 ⇒ 本判据把臂 3 的 stem 认成"护栏放行了覆盖工程文件"；本判据
    // 的 `remove_dir_all` 落在臂 3 导出中途 ⇒ 臂 3 写盘失败、manifest 60 秒
    // 不到（上报人与 T-FIX-719 见到的那一面）。护栏本身没坏，坏的是判据
    // 的隔离 —— 锁是 71544825 立下的既有约定（主判据与另两个分轨镜像文件
    // 都已持锁），这里只是补上漏的那一处。
    let _guard = STEMS_DIR_LOCK.lock().expect("分轨目录锁必须能拿到");
    let scratch = std::env::temp_dir().join(format!(
        "yeban-422-stems-guard-{}-{}",
        std::process::id(),
        yeban_model::ids::EntityId::new().to_canonical_string()
    ));
    let out_dir = stems_dir();
    let _ = std::fs::remove_dir_all(&out_dir);

    let project: YebanProjectV1 = yeban_model::samples::filled_project();
    let lead = project
        .tracks
        .values()
        .find(|track| track.name == "Lead")
        .expect("filled_project 必须有 Lead 轨");
    let project_path = out_dir.join("0001.Lead.wav");
    assert!(
        lead.name == "Lead",
        "夹具前提：第一条分轨名必须能推出 0001.Lead.wav"
    );

    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<422-guard>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    let production = ProductionLoop::start(&project, 0).expect("引擎重建");
    let engine = production.engine_handle();
    engine
        .borrow_mut()
        .set_project_path(Some(project_path.clone()));
    assert_eq!(
        engine.borrow().project_path(),
        Some(project_path.as_path()),
        "前提：引擎必须记着真实工程文件名"
    );

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();
    let _ = host::wire_module_actions_with(
        ui,
        Some(Rc::clone(&undo)),
        ModulePaths::default(),
        Rc::new(yeban_app::file_picker::NoPicker),
    );
    host::wire_aether_actions(
        ui,
        Rc::clone(&undo),
        Rc::clone(&engine),
        Rc::new(RefCell::new(view.clone())),
    );
    ui.invoke_command_execute("export-stems".into());
    assert!(
        out_dir.exists(),
        "export-stems 回调必须真的跑到（它负责创建输出目录）"
    );

    let manifest = out_dir.join("stems-manifest.json");
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if manifest.exists() || project_path.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        !manifest.exists(),
        "真实工程文件名下，输出 == 工程文件必须在落盘前被 guard 拒绝；manifest={}",
        manifest.display()
    );
    assert!(
        !project_path.exists(),
        "护栏必须拒绝把分轨写到真实工程文件路径 {}",
        project_path.display()
    );

    let _ = std::fs::remove_dir_all(&out_dir);
    let _ = std::fs::remove_dir_all(&scratch);
}

///
/// GUI 分轨入口没有工程落盘路径 ⇒ 用输出目录下的 `project.yeban` 占位
/// （`host.rs` 的 `export-stems` 臂）。`resolve_sampler_path` 对相对 SFZ 按
/// 工程目录 ⇒ 采样库根 ⇒ 仓库根 ⇒ cwd 四档解析 —— SFZ 只放在工程文件旁边时，
/// 占位目录那一档是错的，其余三档也不中 ⇒ 渲染失败 ⇒ 分轨导不出。
/// 本臂用 `sample=*silence` 的 SFZ（不依赖任何 WAV 字节）钉住这一点：
/// 占位路径必败、真实工程路径必成。
#[test]
fn stems_placeholder_project_path_breaks_project_adjacent_relative_sfz() {
    use yeban_model::project::SamplerBinding;

    const ONE_REGION_SILENCE: &str = "<region> sample=*silence key=60 pitch_keycenter=60\n";
    let scratch = std::env::temp_dir().join(format!(
        "yeban-167-stems-ph-{}-{}",
        std::process::id(),
        yeban_model::ids::EntityId::new().to_canonical_string()
    ));
    let proj_dir = scratch.join("proj");
    std::fs::create_dir_all(&proj_dir).expect("建工程目录");
    std::fs::write(proj_dir.join("lead.sfz"), ONE_REGION_SILENCE).expect("写 SFZ 夹具");

    let mut project: YebanProjectV1 = yeban_model::samples::filled_project();
    let lead = project
        .tracks
        .values_mut()
        .find(|track| track.name == "Lead")
        .expect("filled_project 必须有 Lead 轨");
    lead.sampler = Some(SamplerBinding {
        path: "lead.sfz".to_owned(),
        note_octave_override: None,
    });

    // ① GUI 占位：输出目录下的 project.yeban（与 `export-stems` 臂逐字同款）。
    let out_ph = scratch.join("stems-placeholder");
    let err = yeban_app::export_stems::export_stems(
        &project,
        &out_ph.join("project.yeban"),
        &out_ph,
        None,
        &[],
    )
    .expect_err("占位 project_path 下，工程旁的相对 SFZ 必须解析失败");
    let err_text = format!("{err}");
    eprintln!("[167-stems-ph 占位] err={err_text}");
    assert!(
        err_text.contains("lead.sfz"),
        "占位失败原因必须点名 lead.sfz: {err_text}"
    );

    // ② 真实工程路径：同一份工程、同一个相对 SFZ ⇒ 必须成功落盘。
    let out_real = scratch.join("stems-real");
    let report = yeban_app::export_stems::export_stems(
        &project,
        &proj_dir.join("project.yeban"),
        &out_real,
        None,
        &[],
    )
    .expect("真实 project_path 下，工程旁的相对 SFZ 必须能解析");
    eprintln!(
        "[167-stems-ph 真实] stems={} skipped={} manifest={}",
        report.stems.len(),
        report.skipped.len(),
        report.manifest_path.display()
    );
    assert!(!report.stems.is_empty(), "真实路径下至少一条分轨必须落盘");
    assert!(
        report.manifest_path.exists(),
        "真实路径下 manifest 必须落盘"
    );

    let _ = std::fs::remove_dir_all(&scratch);
}
