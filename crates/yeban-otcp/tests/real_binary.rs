//! 真实二进制集成测试（设计稿 §8 的 T1–T13）。
//!
//! 全部 `#[ignore]`：须 `YEBAN_OTCP_TEST_BIN` 指向 API 分支的
//! OpenTune Standalone 构建（带 control API；普通 v1.8.2 发行版
//! 无监听器，过不了 `session.info` 门）。
//!
//! 跑法（盒子空闲时，⛔ 不抢盒子、不唤醒盒子）：
//! ```sh
//! YEBAN_OTCP_TEST_BIN=/path/to/OpenTune \
//!   cargo test -p yeban-otcp --test real_binary -- --ignored --test-threads=1 --nocapture
//! ```
//! 串行跑（同一台机器上一次只起一个服务端，避免端口与显示争用）。
//! `--nocapture` 必带：跳过原因打在 stderr 上，不带会被 cargo 吞掉。
//!
//! 二进制缺失时大声跳过（打印 `SKIPPED` + 原因），⛔ 不空绿：
//! CI 只在已部署二进制的环境里跑本文件。
//!
//! T5 / T6 / T8 / T10 / T12 / T13 在真实二进制上无法无头触发，
//! 各自由 mock 测试或 MCP 层测试覆盖——本文件里对应测试只打印
//! 覆盖指向并跳过，⛔ 不编造通过。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tempfile::TempDir;
use yeban_otcp::config::OtcpConfig;
use yeban_otcp::discovery::pid_alive;
use yeban_otcp::pipeline::{CorrectionRequest, Key, OtcpDriver};
use yeban_otcp::supervisor::start_server;
use yeban_otcp::wav::write_wav_mono_pcm16;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// 真实二进制路径；缺失时打印 `SKIPPED` 并返回 `None`。
fn real_binary_path() -> Option<PathBuf> {
    match std::env::var("YEBAN_OTCP_TEST_BIN") {
        Ok(path) if !path.is_empty() => {
            let path = PathBuf::from(path);
            if path.exists() {
                Some(path)
            } else {
                eprintln!(
                    "SKIPPED: YEBAN_OTCP_TEST_BIN 指向不存在的文件：{}",
                    path.display()
                );
                None
            }
        }
        _ => {
            eprintln!(
                "SKIPPED: 未设 YEBAN_OTCP_TEST_BIN（须指向 API 分支 OpenTune Standalone 构建）"
            );
            None
        }
    }
}

fn test_config(bin: &Path) -> (OtcpConfig, TempDir) {
    let dir = TempDir::new().expect("建临时目录");
    let mut config = OtcpConfig::default();
    config.opentune_bin = bin.to_owned();
    config.control_root = dir.path().join("control-root");
    // 盒上跑时继承会话的 DISPLAY（yanshi-5 上为 :1）；未设 DISPLAY 时
    // 驱动包 xvfb-run（须盒上有 xvfb-run，否则启动失败）。
    config.display = yeban_otcp::config::DisplayConfig::Inherit;
    (config, dir)
}

fn request(job_id: &str, source_wav: &Path, duration_secs: f64) -> CorrectionRequest {
    CorrectionRequest {
        job_id: job_id.to_owned(),
        source_wav: source_wav.to_owned(),
        source_duration_secs: duration_secs,
        key: Key {
            root: 9,
            scale: "major".to_owned(),
        },
        mode: "snapNotes".to_owned(),
        tuning_hz: 440.0,
        dry_run: false,
        note_ops: Vec::new(),
        track_id: String::new(),
        clip_id: String::new(),
        auto_merge: false,
        source_asset_hash: String::new(),
        source_license: String::new(),
    }
}

/// 连续无间隙乐句（T3 夹具）：440 Hz → 493.88 Hz 平滑过渡，4 秒，无静音段。
fn continuous_phrase(path: &Path) {
    let sample_rate = 48_000u32;
    let mut samples = Vec::with_capacity(48_000 * 4);
    for i in 0..48_000 * 4 {
        let t = i as f32 / sample_rate as f32;
        // 前 2 秒 A4，后 2 秒 B4，0.1 秒交叉淡化（无间隙、无静音）。
        let freq = if t < 1.95 {
            440.0
        } else if t > 2.05 {
            493.88
        } else {
            440.0 + (493.88 - 440.0) * (t - 1.95) / 0.1
        };
        samples.push((2.0 * std::f32::consts::PI * freq * t).sin() * 0.5);
    }
    write_wav_mono_pcm16(path, sample_rate, &samples).expect("写 T3 夹具");
}

#[test]
#[ignore = "needs a real OpenTune API-branch binary (YEBAN_OTCP_TEST_BIN)"]
fn t1_sine_correction_end_to_end() {
    let Some(bin) = real_binary_path() else {
        return;
    };
    let (config, dir) = test_config(&bin);
    let source = fixtures_dir().join("sine452-orig.wav");
    assert!(source.exists(), "夹具缺失：{}", source.display());
    let driver = OtcpDriver::new(config.clone());
    let receipt = driver
        .correct_clip(request("t1-sine", &source, 5.0))
        .expect("T1：452 Hz 正弦 snap 到 A 大调应成功");
    assert_eq!(receipt.status, "succeeded");
    assert!(!receipt.notes_after.is_empty());
    let measurement = receipt.measurement.expect("T1：有量测结果");
    assert!(measurement.passed, "T1：量测门应过");
    assert!(
        measurement.median_cents < 10.0,
        "T1：导出应接近 440 Hz，中位 |偏差| = {} 音分",
        measurement.median_cents
    );
    assert!(measurement.detection_rate >= 0.8);
    let export_path = receipt.export_path.expect("T1：有导出文件");
    assert!(Path::new(&export_path).exists());
    let _ = dir;
}

#[test]
#[ignore = "needs a real OpenTune API-branch binary (YEBAN_OTCP_TEST_BIN)"]
fn t2_unvoiced_gap_reproduction_matches_self_test() {
    let Some(bin) = real_binary_path() else {
        return;
    };
    let (config, dir) = test_config(&bin);
    let source = fixtures_dir().join("sliceA-source.wav");
    assert!(source.exists(), "夹具缺失：{}", source.display());
    let driver = OtcpDriver::new(config.clone());
    let outcome = driver.correct_clip(request("t2-slicea", &source, 10.988));
    // 自测记录决定期望：fail ⇒ 渲染失败；pass ⇒ 导出成功。
    let record_text = fs::read_to_string(config.self_test_path()).expect("T2：自测记录应在");
    let record: Value = serde_json::from_str(&record_text).expect("T2：解析自测记录");
    let gap = record["unvoiced_gap"].as_str().unwrap_or("unknown");
    eprintln!(
        "T2：自测 unvoiced_gap={gap}，构建={}，任务结果={}",
        record["build_commit"].as_str().unwrap_or("?"),
        if outcome.is_ok() {
            "succeeded"
        } else {
            "failed"
        }
    );
    match gap {
        "fail" => {
            let error = outcome.expect_err("T2：缺陷仍在时导出应失败");
            assert_eq!(error.reason, "otcpRenderFailed");
            let block = &error.data["block"];
            assert!(block["startSample"].is_number(), "T2：失败块应有名");
            assert_eq!(block["actualSamples"], 0, "T2：失败块实际 0 采样");
            assert!(error.data["note_gap"]["prev_note_end_sec"].is_number());
            assert!(error.data["note_gap"]["next_note_start_sec"].is_number());
            assert_eq!(error.data["self_test"]["unvoiced_gap"], "fail");
            // 什么都没写。
            assert!(!config.job_dir("t2-slicea").join("export.wav").exists());
        }
        "pass" => {
            let receipt = outcome.expect("T2：修补已在时导出应成功");
            assert_eq!(receipt.status, "succeeded");
            assert!(receipt.measurement.expect("T2：有量测").passed);
        }
        other => panic!("T2：自测记录未定（unvoiced_gap={other}），无法判定"),
    }
    let _ = dir;
}

#[test]
#[ignore = "needs a real OpenTune API-branch binary (YEBAN_OTCP_TEST_BIN)"]
fn t3_continuously_voiced_vocal_succeeds() {
    let Some(bin) = real_binary_path() else {
        return;
    };
    let (config, dir) = test_config(&bin);
    let source = dir.path().join("t3-phrase.wav");
    continuous_phrase(&source);
    let driver = OtcpDriver::new(config.clone());
    let receipt = driver
        .correct_clip(request("t3-phrase", &source, 4.0))
        .expect("T3：连续有声乐句应导出成功");
    assert_eq!(receipt.status, "succeeded");
    let measurement = receipt.measurement.expect("T3：有量测结果");
    assert!(measurement.passed, "T3：量测门应过");
    assert!(measurement.median_cents <= 50.0);
    assert!(measurement.detection_rate >= 0.8);
}

#[test]
#[ignore = "needs a real OpenTune API-branch binary (YEBAN_OTCP_TEST_BIN)"]
fn t4_failure_leaves_server_quittable() {
    let Some(bin) = real_binary_path() else {
        return;
    };
    let (config, dir) = test_config(&bin);
    let source = fixtures_dir().join("sliceA-source.wav");
    let driver = OtcpDriver::new(config.clone());
    // 先跑一次可能失败的任务（缺陷在则 RENDER_FAILED，修补在则成功），
    // 再跑一次正弦任务：两次都须干净结束，第二次须成功——证明首次
    // 失败没把服务端 / 模态框状态 wedge 住。
    let first = driver.correct_clip(request("t4-first", &source, 10.988));
    if let Err(error) = &first {
        assert_eq!(error.reason, "otcpRenderFailed", "T4：首次只应是渲染失败");
    }
    let sine = fixtures_dir().join("sine452-orig.wav");
    let second = driver
        .correct_clip(request("t4-second", &sine, 5.0))
        .expect("T4：首次失败后新任务仍应成功（服务端干净退出）");
    assert_eq!(second.status, "succeeded");
    let _ = dir;
}

#[test]
#[ignore = "needs a real OpenTune API-branch binary (YEBAN_OTCP_TEST_BIN)"]
fn t5_dialog_that_commits_manual_only() {
    // T5 须人工在编辑器里造一个 closeCommits: true 的对话框；
    // 无头跑法无法合成。逻辑由 mock 测试
    // dialog_that_commits_stops_without_confirm 覆盖。
    eprintln!(
        "SKIPPED: T5 须人工造提交型对话框；见 mock 测试 dialog_that_commits_stops_without_confirm"
    );
    let _ = real_binary_path();
}

#[test]
#[ignore = "needs a real OpenTune API-branch binary (YEBAN_OTCP_TEST_BIN)"]
fn t6_revision_conflict_mock_covered() {
    // T6 须编辑器侧并发改动才触发；无头跑法无法合成。
    // 重试逻辑由 mock 测试 revision_conflict_retries_once_and_succeeds 覆盖。
    eprintln!("SKIPPED: T6 须并发修订；见 mock 测试 revision_conflict_retries_once_and_succeeds");
    let _ = real_binary_path();
}

#[test]
#[ignore = "needs a real OpenTune API-branch binary (YEBAN_OTCP_TEST_BIN)"]
fn t7_cold_start_f0_wait() {
    let Some(bin) = real_binary_path() else {
        return;
    };
    let (config, dir) = test_config(&bin);
    let source = fixtures_dir().join("sine452-orig.wav");
    let driver = OtcpDriver::new(config.clone());
    // 冷启动服务端：首次 F0 轮询未就绪（可花 ~60 s），驱动必须等到
    // ready 才 autoTune——不等就会 NOT_READY 失败。
    let started = std::time::Instant::now();
    let receipt = driver
        .correct_clip(request("t7-cold", &source, 5.0))
        .expect("T7：冷启动也应等 F0 就绪后成功");
    assert_eq!(receipt.status, "succeeded");
    eprintln!("T7：冷启动整任务耗时 {} 秒", started.elapsed().as_secs());
    let _ = dir;
}

#[test]
#[ignore = "needs a real OpenTune API-branch binary (YEBAN_OTCP_TEST_BIN)"]
fn t8_measurement_gate_mock_covered() {
    // T8 须一个「上报在调、实测离调」的 hybrid 导出样本；无头跑法
    // 无法稳定合成。逻辑由 mock 测试 measure_gate_catches_off_pitch_export 覆盖。
    eprintln!("SKIPPED: T8 须离调导出样本；见 mock 测试 measure_gate_catches_off_pitch_export");
    let _ = real_binary_path();
}

#[test]
#[ignore = "needs a real OpenTune API-branch binary (YEBAN_OTCP_TEST_BIN)"]
fn t9_receipt_contains_no_secret_keys() {
    let Some(bin) = real_binary_path() else {
        return;
    };
    let (config, dir) = test_config(&bin);
    let source = fixtures_dir().join("sine452-orig.wav");
    let driver = OtcpDriver::new(config.clone());
    driver
        .correct_clip(request("t9-sine", &source, 5.0))
        .expect("T9：任务应成功");
    let text = fs::read_to_string(config.receipt_path("t9-sine")).expect("T9：读收据");
    let lower = text.to_lowercase();
    assert!(!lower.contains("token"), "T9：收据含 token 键");
    assert!(!lower.contains("serverproof"), "T9：收据含 serverProof 键");
    let self_test = fs::read_to_string(config.self_test_path()).expect("T9：读自测记录");
    assert!(!self_test.to_lowercase().contains("token"));
    assert!(!self_test.to_lowercase().contains("serverproof"));
    let _ = dir;
}

#[test]
#[ignore = "needs a real OpenTune API-branch binary (YEBAN_OTCP_TEST_BIN)"]
fn t10_discovery_dir_mode_unit_covered() {
    // T10 的驱动自检由单元测试 control_dir_mode_check_fails_on_0775 覆盖；
    // 服务端拒收 0775 目录的行为须手动 chmod 后验证，无头跑法不覆盖。
    eprintln!("SKIPPED: T10 见单元测试 control_dir_mode_check_fails_on_0775");
    let _ = real_binary_path();
}

#[test]
#[ignore = "needs a real OpenTune API-branch binary (YEBAN_OTCP_TEST_BIN)"]
fn t11_drop_kills_server_and_cleans_up() {
    let Some(bin) = real_binary_path() else {
        return;
    };
    let (config, dir) = test_config(&bin);
    let session = start_server(&config, "t11-drop").expect("T11：服务端应启动");
    let pid = session.pid();
    assert!(pid_alive(pid), "T11：启动后 pid 应存活");
    let tmp_dir = config.job_tmp_dir("t11-drop");
    let control_dir = config.control_root.join("control").join("t11-drop");
    assert!(tmp_dir.exists());
    assert!(control_dir.exists());
    drop(session);
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(!pid_alive(pid), "T11：Drop 后进程应死");
    assert!(!tmp_dir.exists(), "T11：Drop 后临时目录应删");
    assert!(!control_dir.exists(), "T11：Drop 后控制目录应删");
    let _ = dir;
}

#[test]
#[ignore = "needs a real OpenTune API-branch binary (YEBAN_OTCP_TEST_BIN)"]
fn t12_double_correction_mcp_covered() {
    // T12 是 MCP 层检测（轨道已挂 OpenTune ExternalEffect 绑定即拒），
    // 见 yeban-mcp tests/otcp_tools.rs 的双重校正测试；驱动层不做该检测。
    eprintln!("SKIPPED: T12 见 yeban-mcp tests/otcp_tools.rs");
    let _ = real_binary_path();
}

#[test]
#[ignore = "needs a real OpenTune API-branch binary (YEBAN_OTCP_TEST_BIN)"]
fn t13_resample_and_asset_swap_mcp_covered() {
    // T13 的重采样 + CAS + 提案换资产在 MCP 层测（合成导出文件，不须二进制），
    // 见 yeban-mcp tests/otcp_tools.rs 的交付测试；真实音频端到端
    // 须盒上跑 T1 成功后另行验证。
    eprintln!("SKIPPED: T13 见 yeban-mcp tests/otcp_tools.rs");
    let _ = real_binary_path();
}
