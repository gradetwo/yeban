//! T-FIX-357: regression guard for the GUI WAV export asset-pool wiring.
//!
//! The GUI export path must pass the archive asset pool into the exporter and
//! surface both the rendered/skipped audio-clip counts plus a visible warning
//! when clips were skipped. The runtime judgement needs a real window and a
//! demo containing `ClipContent::Audio`; this test locks the wiring at source
//! level so a later edit cannot silently drop it again.

#[test]
fn gui_wav_export_passes_asset_pool_and_reports_skipped_clips() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let host = std::fs::read_to_string(manifest.join("src/host.rs")).expect("read host.rs");
    let export =
        std::fs::read_to_string(manifest.join("src/export_wav.rs")).expect("read export_wav.rs");

    assert!(
        host.contains("let assets = port.assets();"),
        "asset pool must be cloned on the UI thread before the worker starts"
    );
    assert!(
        host.contains("export_wav_with_sfz_file_and_assets("),
        "GUI export must call the asset-aware exporter"
    );
    assert!(
        export.contains("pub fn export_wav_with_sfz_file_and_assets("),
        "asset-aware exporter must exist"
    );
    assert!(
        host.contains("audio_clips_rendered=") && host.contains("audio_clips_skipped="),
        "receipt must expose the rendered/skipped audio clip counters"
    );
    assert!(
        host.contains("导出缺少音频素材"),
        "skipped audio assets must produce a visible warning"
    );
}
