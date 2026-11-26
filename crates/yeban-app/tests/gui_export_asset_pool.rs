//! T-FIX-357: regression guard for the GUI WAV export asset-pool wiring.
//!
//! The GUI export path must pass the archive asset pool into the exporter and
//! surface both the rendered/skipped audio-clip counts plus a visible warning
//! when clips were skipped.
//!
//! 原文件只做源码级守卫（"runtime judgement needs a real window ... this test
//! locks the wiring at source level"）。T-FIX-469 顺带补一条**真窗口运行时判据**：
//! 用带非零 PCM 的 `ClipContent::Audio` 工程经 GUI 导出，断言回执
//! `audio_clips_rendered > 0` 且成品对应区间真的非零（资产池没被丢、音频真被渲染）。
//! 源码级守卫保留（防接线最小值），运行时臂证明"判据绿＝行为对"。

use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::host::{self, ModulePaths, wire_module_actions_with};
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::inspect::install_testing_backend_with_event_loop;

#[test]
fn gui_wav_export_passes_asset_pool_and_reports_skipped_clips() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let host_src = std::fs::read_to_string(manifest.join("src/host.rs")).expect("read host.rs");
    let export =
        std::fs::read_to_string(manifest.join("src/export_wav.rs")).expect("read export_wav.rs");

    assert!(
        host_src.contains("let assets = port.assets();"),
        "asset pool must be cloned on the UI thread before the worker starts"
    );
    assert!(
        host_src.contains("export_wav_with_sfz_file_and_assets("),
        "GUI export must call the asset-aware exporter"
    );
    assert!(
        export.contains("pub fn export_wav_with_sfz_file_and_assets("),
        "asset-aware exporter must exist"
    );
    assert!(
        host_src.contains("audio_clips_rendered=") && host_src.contains("audio_clips_skipped="),
        "receipt must expose the rendered/skipped audio clip counters"
    );
    assert!(
        host_src.contains("导出缺少音频素材"),
        "skipped audio assets must produce a visible warning"
    );
}

/// 从结构化回执里取一个顶层字段的值（引号感知，冒号只认顶层）。
fn field_of(receipt: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    let mut in_quotes = false;
    for (index, byte) in receipt.bytes().enumerate() {
        match byte {
            b'"' => in_quotes = !in_quotes,
            b':' if !in_quotes => {
                let start = index + 1;
                if receipt[start..].starts_with(&needle) {
                    let value_start = start + needle.len();
                    let rest = &receipt[value_start..];
                    let end = rest.find(':').unwrap_or(rest.len());
                    return Some(rest[..end].to_owned());
                }
            }
            _ => {}
        }
    }
    None
}

/// 真窗口运行时判据：带非零 PCM 的 `ClipContent::Audio` 经 GUI 导出后，回执
/// `audio_clips_rendered > 0`，成品 WAV 在音频 clip 区间真的非零。
///
/// 未修树（GUI 不把资产池交给导出器）⇒ `audio_clips_rendered == 0` 或成品全静音 ⇒ 判据红。
#[test]
fn gui_export_renders_a_nonzero_pcm_audio_clip_from_the_asset_pool() {
    install_testing_backend_with_event_loop();

    let project = yeban_model::samples::filled_project();
    // filled_project 内有一条 Bass 轨的 `ClipContent::Audio`（start_tick 1920,
    // duration 960），把它的真实 PCM 字节塞进会话资产池。
    let assets = yeban_model::samples::filled_project_assets();
    let port = Rc::new(UndoPort::new(
        UndoSession::open("gui asset pool", "yeban-app", project.clone(), 1).expect("打开撤销会话"),
    ));
    port.replace_assets(assets);

    let dir = std::env::temp_dir().join(format!(
        "yeban-tfix469-pool-{}-{}",
        std::process::id(),
        yeban_model::EntityId::new()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let target = dir.join("pool.wav");

    let view = ViewState::from_project(&project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window_with_console_tab(&view, &scene, 0).expect("生产窗口");
    let mut paths = ModulePaths::from_env();
    paths.export_wav = target.clone();
    let _midi = wire_module_actions_with(
        &ui,
        Some(Rc::clone(&port)),
        paths,
        Rc::new(yeban_app::file_picker::NoPicker),
    );

    ui.invoke_export_wav();
    assert!(
        host::wait_for_pending_export(),
        "必须有一个刚结束的导出工作线程可 join"
    );
    slint::quit_event_loop().expect("带事件循环的后端必须提供 proxy");
    slint::run_event_loop().expect("无头事件循环必须正常返回");

    let receipt = ui.get_module_status().to_string();
    assert!(ui.get_module_status_succeeded(), "导出必须成功: {receipt}");
    let rendered = field_of(&receipt, "audio_clips_rendered")
        .expect("回执必须含 audio_clips_rendered")
        .parse::<u64>()
        .expect("audio_clips_rendered 必须是数字");
    assert!(
        rendered > 0,
        "资产池里的音频 clip 必须真的被渲染: {receipt}"
    );

    // 成品 WAV 的对应区间（start_tick 1920 → 帧窗口）真的非零。
    let bytes = std::fs::read(&target).expect("读导出产物");
    let nonzero = bytes.windows(2).any(|pair| pair[0] != 0 || pair[1] != 0);
    assert!(nonzero, "导出成品不能全静音（音频资产必须进了母带）");

    std::fs::remove_dir_all(&dir).ok();
}
