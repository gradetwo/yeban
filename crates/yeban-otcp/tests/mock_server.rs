//! Mock 服务端集成测试：驱动对假 OpenTune 跑完整流水线。
//!
//! 假服务端是 `tests/mock_opentune.py`（Python 标准库，无 OpenTune 代码）。
//! 每测一个独立控制根 + 一个场景（`mock-scenario.json`），互不干扰。
//!
//! 对应设计稿 §8 的 T1 / T2 形 / T4 / T5 / T6 / T7 / T8 / T9：
//! 真实二进制的 T1–T13 见 `tests/real_binary.rs`（`#[ignore]`，须
//! `YEBAN_OTCP_TEST_BIN` 指向 API 分支构建）。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tempfile::TempDir;
use yeban_otcp::config::{DisplayConfig, OtcpConfig};
use yeban_otcp::pipeline::{CorrectionRequest, Key, OtcpDriver};
use yeban_otcp::wav::write_wav_mono_pcm16;

fn mock_script() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_opentune.py");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o755));
    }
    path
}

fn test_config(mode: &str) -> (OtcpConfig, TempDir) {
    let dir = TempDir::new().expect("建临时目录");
    let control_root = dir.path().join("control-root");
    fs::create_dir_all(&control_root).expect("建控制根");
    let scenario = json!({"mode": mode, "build_commit": "mock-build-001"});
    fs::write(
        control_root.join("mock-scenario.json"),
        serde_json::to_string(&scenario).expect("序列化场景"),
    )
    .expect("写场景");
    let config = OtcpConfig {
        opentune_bin: mock_script(),
        control_root,
        display: DisplayConfig::Explicit(":0".to_owned()),
        ..OtcpConfig::default()
    };
    (config, dir)
}

/// 源 WAV：2.0 秒，两段音（440 Hz 0–1 s + 493.88 Hz 1–2 s），48 kHz 单声道。
fn two_tone_source(path: &Path) {
    let sample_rate = 48_000u32;
    let mut samples = Vec::with_capacity(96_000);
    for i in 0..48_000 {
        samples
            .push((2.0 * std::f32::consts::PI * 440.0 * i as f32 / sample_rate as f32).sin() * 0.5);
    }
    for i in 0..48_000 {
        samples.push(
            (2.0 * std::f32::consts::PI * 493.88 * i as f32 / sample_rate as f32).sin() * 0.5,
        );
    }
    write_wav_mono_pcm16(path, sample_rate, &samples).expect("写源 WAV");
}

fn request(job_id: &str, source_wav: &Path, dry_run: bool) -> CorrectionRequest {
    CorrectionRequest {
        job_id: job_id.to_owned(),
        source_wav: source_wav.to_owned(),
        source_duration_secs: 2.0,
        key: Key {
            root: 9,
            scale: "major".to_owned(),
        },
        mode: "snapNotes".to_owned(),
        tuning_hz: 440.0,
        dry_run,
        note_ops: Vec::new(),
        track_id: String::new(),
        clip_id: String::new(),
        auto_merge: false,
        source_asset_hash: String::new(),
        source_license: String::new(),
    }
}

fn trace_methods(config: &OtcpConfig, job_id: &str) -> Vec<String> {
    let path = config
        .control_root
        .join(format!("mock-trace-{job_id}.jsonl"));
    let text = fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| {
            value
                .get("method")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .collect()
}

#[test]
fn happy_path_corrects_clip_end_to_end() {
    let (config, dir) = test_config("happy");
    let source = dir.path().join("source.wav");
    two_tone_source(&source);
    let driver = OtcpDriver::new(config.clone());
    let receipt = driver
        .correct_clip(request("happy-1", &source, false))
        .expect("happy 路径应成功");
    assert_eq!(receipt.status, "succeeded");
    assert_eq!(receipt.content_id.as_deref(), Some("c-mock-2"));
    assert_eq!(receipt.notes_before.len(), 2);
    assert_eq!(receipt.notes_after.len(), 2);
    assert!(receipt.notes_after.iter().all(|n| n.corrected));
    let measurement = receipt.measurement.expect("有量测结果");
    assert!(measurement.passed);
    assert!(
        measurement.median_cents < 5.0,
        "median={}",
        measurement.median_cents
    );
    assert!(measurement.detection_rate > 0.8);
    assert!(measurement.hybrid_mode);
    assert_eq!(measurement.backend, "cpu");
    let export_path = receipt.export_path.expect("有导出路径");
    assert!(Path::new(&export_path).exists());
    assert_eq!(receipt.server["build"], "mock-build-001");
    assert_eq!(receipt.server["self_test"]["unvoiced_gap"], "pass");
    assert_eq!(receipt.editing_scheme, "openDyne");
    // 收据落盘且可重读；服务端已退出（app.quit 在轨迹里）。
    let reread = driver.read_receipt("happy-1").expect("收据可重读");
    assert_eq!(reread.status, "succeeded");
    let methods = trace_methods(&config, "happy-1");
    assert!(methods.contains(&"app.quit".to_owned()));
    assert!(methods.contains(&"session.hello".to_owned()));
    assert!(methods.contains(&"content.autoTune".to_owned()));
}

#[test]
fn render_failure_fails_loud_and_writes_nothing() {
    let (config, dir) = test_config("render_fail");
    let source = dir.path().join("source.wav");
    two_tone_source(&source);
    let driver = OtcpDriver::new(config.clone());
    let error = driver
        .correct_clip(request("render-fail-1", &source, false))
        .expect_err("渲染失败应大声失败");
    assert_eq!(error.reason, "otcpRenderFailed");
    // §6.2 结构化上报：失败块 + 音符间隙 + 构建 + 自测记录。
    assert_eq!(error.data["block"]["startSample"], 128000);
    assert_eq!(error.data["block"]["expectedSamples"], 27136);
    assert_eq!(error.data["block"]["actualSamples"], 0);
    assert_eq!(error.data["note_gap"]["prev_note_end_sec"], 2.0);
    assert_eq!(error.data["otcp_build"], "mock-build-001");
    assert_eq!(error.data["self_test"]["unvoiced_gap"], "fail");
    // 什么都没写：导出文件不存在，收据为 failed。
    let receipt = driver.read_receipt("render-fail-1").expect("收据在");
    assert_eq!(receipt.status, "failed");
    assert_eq!(
        receipt.error.as_ref().expect("有错误")["reason"],
        "otcpRenderFailed"
    );
    assert!(receipt.export_path.is_none());
    assert!(!config.job_dir("render-fail-1").join("export.wav").exists());
    // 模态框已解散、服务端已退出。
    let methods = trace_methods(&config, "render-fail-1");
    assert!(methods.contains(&"ui.dismiss".to_owned()));
    assert!(methods.contains(&"app.quit".to_owned()));
}

#[test]
fn modal_recovery_dismisses_and_continues() {
    let (config, dir) = test_config("modal");
    let source = dir.path().join("source.wav");
    two_tone_source(&source);
    let driver = OtcpDriver::new(config.clone());
    let receipt = driver
        .correct_clip(request("modal-1", &source, false))
        .expect("模态框解散后应继续成功");
    assert_eq!(receipt.status, "succeeded");
    let methods = trace_methods(&config, "modal-1");
    assert!(methods.contains(&"ui.getState".to_owned()));
    assert!(methods.contains(&"ui.dismiss".to_owned()));
    assert_eq!(
        methods
            .iter()
            .filter(|m| m.as_str() == "content.autoTune")
            .count(),
        4,
        "自测与任务各重试一次（各 2 次调用）"
    );
}

#[test]
fn dialog_that_commits_stops_without_confirm() {
    let (config, dir) = test_config("dialog_commits");
    let source = dir.path().join("source.wav");
    two_tone_source(&source);
    let driver = OtcpDriver::new(config.clone());
    let error = driver
        .correct_clip(request("dialog-1", &source, false))
        .expect_err("提交型对话框应停住");
    assert_eq!(error.reason, "otcpDialogCommits");
    // 驱动⛔ 没有自行确认：ui.dismiss 一次也没发。
    let methods = trace_methods(&config, "dialog-1");
    assert!(!methods.contains(&"ui.dismiss".to_owned()));
    let receipt = driver.read_receipt("dialog-1").expect("收据在");
    assert_eq!(receipt.status, "failed");
}

#[test]
fn revision_conflict_retries_once_and_succeeds() {
    let (config, dir) = test_config("revision_conflict");
    let source = dir.path().join("source.wav");
    two_tone_source(&source);
    let driver = OtcpDriver::new(config.clone());
    let receipt = driver
        .correct_clip(request("conflict-1", &source, false))
        .expect("修订冲突重试一次应成功");
    assert_eq!(receipt.status, "succeeded");
    let methods = trace_methods(&config, "conflict-1");
    assert_eq!(
        methods
            .iter()
            .filter(|m| m.as_str() == "content.autoTune")
            .count(),
        4,
        "自测与任务各重试一次（各 2 次调用）"
    );
}

#[test]
fn f0_not_ready_waits_then_succeeds() {
    let (config, dir) = test_config("f0_slow");
    let source = dir.path().join("source.wav");
    two_tone_source(&source);
    let driver = OtcpDriver::new(config.clone());
    let receipt = driver
        .correct_clip(request("f0slow-1", &source, false))
        .expect("F0 未就绪时应等待后成功");
    assert_eq!(receipt.status, "succeeded");
    let methods = trace_methods(&config, "f0slow-1");
    assert!(
        methods
            .iter()
            .filter(|m| m.as_str() == "content.getDetails")
            .count()
            >= 4,
        "自测与任务各应轮询多次：{methods:?}"
    );
}

#[test]
fn dry_run_returns_prediction_without_export() {
    let (config, dir) = test_config("happy");
    let source = dir.path().join("source.wav");
    two_tone_source(&source);
    let driver = OtcpDriver::new(config.clone());
    let receipt = driver
        .correct_clip(request("dryrun-1", &source, true))
        .expect("干跑应成功");
    assert_eq!(receipt.status, "succeeded");
    assert!(receipt.dry_run);
    assert_eq!(receipt.notes_after.len(), 2);
    assert!(receipt.notes_after.iter().all(|n| n.corrected));
    assert!(receipt.export_path.is_none());
    assert!(receipt.measurement.is_none());
    let methods = trace_methods(&config, "dryrun-1");
    // 自测仍会导出一次夹具（自测门需要导出结果）；任务本身不导出。
    assert_eq!(
        methods
            .iter()
            .filter(|m| m.as_str() == "export.start")
            .count(),
        1,
        "只有自测导出一次，任务不导出"
    );
    assert!(!config.job_dir("dryrun-1").join("export.wav").exists());
}

#[test]
fn measure_gate_catches_off_pitch_export() {
    let (config, dir) = test_config("export_off_pitch");
    let source = dir.path().join("source.wav");
    two_tone_source(&source);
    let driver = OtcpDriver::new(config.clone());
    let error = driver
        .correct_clip(request("offpitch-1", &source, false))
        .expect_err("离调导出应被量测门拦住");
    assert_eq!(error.reason, "otcpMeasureFailed");
    let median = error.data["median_cents"].as_f64().expect("有中位值");
    assert!(median > 50.0, "median={median}");
    let receipt = driver.read_receipt("offpitch-1").expect("收据在");
    assert_eq!(receipt.status, "failed");
    // 导出文件留在任务目录作证据，但收据 failed ⇒ MCP 层不交付。
    assert!(config.job_dir("offpitch-1").join("export.wav").exists());
}

#[test]
fn receipt_contains_no_secrets() {
    let (config, dir) = test_config("happy");
    let source = dir.path().join("source.wav");
    two_tone_source(&source);
    let driver = OtcpDriver::new(config.clone());
    driver
        .correct_clip(request("secrets-1", &source, false))
        .expect("应成功");
    let text = fs::read_to_string(config.receipt_path("secrets-1")).expect("读收据");
    assert!(!text.contains("mock-token-xyz"), "收据含 token");
    assert!(!text.contains("mock-proof-xyz"), "收据含 serverProof");
    // 自测记录同样不含秘密。
    let self_test = fs::read_to_string(config.self_test_path()).expect("读自测记录");
    assert!(!self_test.contains("mock-token-xyz"));
    assert!(!self_test.contains("mock-proof-xyz"));
    assert!(self_test.contains("\"unvoiced_gap\": \"pass\""));
}
