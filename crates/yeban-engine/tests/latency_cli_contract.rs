//! `BASELINE-005` 工具（`examples/measure_latency.rs`）的**运行期契约判据**。
//!
//! 这个文件跑的是**真的二进制**（`cargo test` 会构建 examples，所以它一定在
//! `target/<profile>/examples/measure_latency`），不是把逻辑重写一遍。
//!
//! # 为什么本机也能跑
//!
//! 本机（Apple M2，按纪律不编译 cpal）走 `--no-default-features` ⇒ example 编译成
//! "未启用 `device` feature" 的**明确失败**分支（退出码 5）。本文件先探测这一点，
//! 然后**响亮地 SKIP** 真实路径断言 —— 不是为了好看，而是因为
//! "本机没编译 cpal" 与 "工具悄悄假装成功" 是**完全不同**的两件事：
//! 前者被下面的断言钉住（必须是 5 + 明确文案），后者会让判据变红。
//!
//! # 判据（对着工具输出解析，不是对着实现）
//!
//! 1. **永不冒充达标**：任何调用形态下 `verdict` 都不得是 `within-target`，
//!    `measured_roundtrip_ms` 都必须是字面 `none`，`loopback` 都必须是 `false`；
//! 2. **无设备路径**（`--force-no-device`，任何机器上都确定性可跑）：退出码 **3**、
//!    `verdict=no-device`、文案含 `NOT a pass and NOT 0 ms`、且输出里**没有** `0.0000`
//!    这种"看起来像 0 ms 读数"的东西；
//! 3. **便利开关不改判定**：`--allow-unmeasurable` 把退出码变成 0，但 `verdict` 与
//!    `measured_roundtrip_ms` **一个字节都不变**；
//! 4. **边界必须打印**：每次运行都要有 `MEASURES` 与 `DOES-NOT-MEASURE` 两行；
//! 5. **有设备时**（若 CI/本机恰有）：标称与抖动都被打印且解析出来非负；
//!    无声卡时**响亮地**说明"本环境无法测量"，而不是静默通过；
//! 6. **用法错误**：`--nope` / `--frames 0` ⇒ 退出码 2。

use std::path::{Path, PathBuf};
use std::process::Command;

use yeban_engine::latency::{
    EXIT_NO_DEVICE, EXIT_TOOL_DISABLED, EXIT_UNMEASURABLE, EXIT_USAGE, parse_bench_line,
};

/// 工具"未启用"时的退出码字面量（防串线）。
const DISABLED: i32 = EXIT_TOOL_DISABLED as i32;
/// 无设备时的退出码字面量。
const NO_DEVICE: i32 = EXIT_NO_DEVICE as i32;
/// 有设备但无回环时的退出码字面量。
const UNMEASURABLE: i32 = EXIT_UNMEASURABLE as i32;
/// 用法错误的退出码字面量。
const USAGE: i32 = EXIT_USAGE as i32;

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// 找到 `cargo test` 顺带构建出来的 example 二进制。
///
/// 路径推导：`target/<profile>/deps/<test>-<hash>` → 上两级 → `examples/measure_latency`。
fn example_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let profile_dir = exe.parent()?.parent()?;
    let mut candidate = profile_dir.join("examples").join("measure_latency");
    if cfg!(windows) {
        candidate.set_extension("exe");
    }
    candidate.is_file().then_some(candidate)
}

fn run(binary: &Path, args: &[&str]) -> Run {
    let output = Command::new(binary)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("无法运行 {}: {error}", binary.display()));
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// 从 stdout 里挑出机器可读行。
fn bench_line(stdout: &str) -> Option<&str> {
    stdout
        .lines()
        .find(|line| line.starts_with("BENCH baseline=005"))
}

fn bench_of(run: &Run) -> yeban_engine::latency::BenchFields {
    let line = bench_line(&run.stdout).unwrap_or_else(|| {
        panic!(
            "工具没有输出可解析的 BENCH 行\n--- stdout ---\n{}\n--- stderr ---\n{}",
            run.stdout, run.stderr
        )
    });
    parse_bench_line(line).unwrap_or_else(|| {
        panic!("BENCH 行不可解析（格式漂移）: {line}");
    })
}

/// 判定与退出码必须**互相自洽**（防"行里写 no-device 但退出码说达标"这种撕裂）。
fn assert_verdict_matches_exit(run: &Run) {
    let fields = bench_of(run);
    let expected = match run.code {
        0 => None, // 只允许 `--allow-unmeasurable` 之后的"没测到"；由调用方进一步钉
        NO_DEVICE => Some("no-device"),
        UNMEASURABLE => Some("unmeasurable-without-loopback"),
        _ => None,
    };
    if let Some(expected) = expected {
        assert_eq!(
            fields.verdict, expected,
            "退出码 {} 必须对应 verdict={expected}，实际 {}",
            run.code, fields.verdict
        );
    }
    // 无条件的不变量：本工具**永不**声称达标，也**永不**给出实测往返数。
    assert_ne!(fields.verdict, "within-target", "工具声称达标（不可能）");
    assert_ne!(
        fields.verdict, "over-target",
        "工具声称实测超目标（不可能）"
    );
    assert_eq!(
        fields.measured_roundtrip_ms, None,
        "工具给出了实测往返数 —— 它没有回环，不可能测得"
    );
    assert!(!fields.loopback, "工具声称用了回环");
    assert!(!fields.nominal_io_sum_is_roundtrip, "把标称合计当成了往返");
    assert_eq!(fields.baseline, "005");
}

/// 判据 1 + 4：真实二进制下的无冒充达标 + 边界必须打印。
#[test]
fn the_tool_never_claims_a_pass_and_always_prints_its_boundary() {
    let Some(binary) = example_binary() else {
        eprintln!(
            "SKIP(loud): target/<profile>/examples/measure_latency 不存在（本次 cargo 调用没构建 \
examples）。用 `cargo test -p yeban-engine` 或 `--all-targets` 可覆盖这条判据。"
        );
        return;
    };
    let probe = run(&binary, &["--seconds", "0.2"]);

    // 本机轻量变体：`--no-default-features` ⇒ 明确的"未启用"分支。
    if probe.code == DISABLED {
        eprintln!(
            "SKIP(loud): example 是用 --no-default-features 构建的（本机轻量变体，cpal 未编译）。"
        );
        assert!(
            probe.stderr.contains("WITHOUT the `device` feature"),
            "未启用路径必须**明确**说明原因，实际 stderr:\n{}",
            probe.stderr
        );
        assert!(
            !probe.stdout.contains("within-target"),
            "未启用路径不许输出任何达标字样"
        );
        return;
    }

    for args in [
        &[][..],
        &["--seconds", "0.2"][..],
        &["--force-no-device"][..],
        &["--force-no-device", "--allow-unmeasurable"][..],
    ] {
        let result = run(&binary, args);
        assert!(
            matches!(result.code, 0 | NO_DEVICE | UNMEASURABLE),
            "args={args:?} 退出码 {} 不在 {{0, 3, 4}} 内（0/1 只能在真·实测回环下出现）\nstdout:\n{}\nstderr:\n{}",
            result.code,
            result.stdout,
            result.stderr
        );
        assert_verdict_matches_exit(&result);
        // 边界两行是**契约**：任何一次运行都必须说清测了什么、没测什么。
        assert!(
            result.stdout.contains("MEASURES:"),
            "args={args:?} 没有打印 MEASURES 边界"
        );
        assert!(
            result.stdout.contains("DOES-NOT-MEASURE:"),
            "args={args:?} 没有打印 DOES-NOT-MEASURE 边界"
        );
        assert!(
            result.stdout.contains("cpal 0.18.2"),
            "args={args:?} 没有点名 cpal 缺少硬件时延 API"
        );
    }
}

/// 判据 2：**无设备路径**（任何机器上确定性可跑）。
#[test]
fn the_no_device_path_is_explicit_and_never_reports_zero_ms() {
    let Some(binary) = example_binary() else {
        eprintln!("SKIP(loud): example 二进制不存在（见上一条判据的说明）");
        return;
    };
    let probe = run(&binary, &["--seconds", "0.2"]);
    if probe.code == DISABLED {
        eprintln!("SKIP(loud): example 是 --no-default-features 变体，无设备路径交给 CI");
        return;
    }

    let forced = run(&binary, &["--force-no-device"]);
    assert_eq!(
        forced.code, NO_DEVICE,
        "--force-no-device 必须给出退出码 {NO_DEVICE}（'没测到'），实际 {}\nstdout:\n{}",
        forced.code, forced.stdout
    );
    let fields = bench_of(&forced);
    assert_eq!(fields.verdict, "no-device");
    assert_eq!(fields.devices, 0);
    assert_eq!(fields.measured_roundtrip_ms, None);
    assert_eq!(fields.nominal_out_ms, None, "无设备时不许有标称数");
    assert_eq!(fields.nominal_in_ms, None);
    assert_eq!(fields.nominal_io_sum_ms, None);
    assert!(
        forced.stdout.contains("NOT a pass and NOT 0 ms"),
        "无设备文案必须**明确**说清不是达标、不是 0 ms\nstdout:\n{}",
        forced.stdout
    );
    assert!(forced.stdout.contains("NO-DEVICE:"));
    assert!(
        !forced.stdout.contains("0.0000"),
        "无设备时输出里出现了看起来像 0 ms 的读数\nstdout:\n{}",
        forced.stdout
    );
    assert!(
        !forced.stdout.contains("within-target"),
        "无设备时不许出现达标字样"
    );
    assert_ne!(
        forced.code, 0,
        "无设备**必须**非零退出（除非 --allow-unmeasurable）"
    );
}

/// 判据 3：便利开关只动退出码，**不动判定**。
#[test]
fn allow_unmeasurable_changes_the_exit_code_but_not_the_verdict() {
    let Some(binary) = example_binary() else {
        eprintln!("SKIP(loud): example 二进制不存在（见上一条判据的说明）");
        return;
    };
    let probe = run(&binary, &["--seconds", "0.2"]);
    if probe.code == DISABLED {
        eprintln!("SKIP(loud): example 是 --no-default-features 变体；便利开关交给 CI");
        return;
    }

    let strict = run(&binary, &["--force-no-device"]);
    let lenient = run(&binary, &["--force-no-device", "--allow-unmeasurable"]);
    assert_eq!(strict.code, NO_DEVICE);
    assert_eq!(lenient.code, 0, "--allow-unmeasurable 应把 3 映射成 0");
    assert_eq!(
        bench_line(&strict.stdout),
        bench_line(&lenient.stdout),
        "便利开关改动了机器可读行 —— 它只许改退出码"
    );
    let fields = bench_of(&lenient);
    assert_eq!(fields.verdict, "no-device", "开关不许把没测到变成达标");
    assert_eq!(fields.measured_roundtrip_ms, None);
}

/// 判据 5：有设备时标称与抖动都打印且非负；无声卡时**响亮**说明。
#[test]
fn a_machine_with_devices_reports_non_negative_nominals_and_jitter() {
    let Some(binary) = example_binary() else {
        eprintln!("SKIP(loud): example 二进制不存在（见上一条判据的说明）");
        return;
    };
    let probe = run(&binary, &["--seconds", "0.5"]);
    if probe.code == DISABLED {
        eprintln!("SKIP(loud): example 是 --no-default-features 变体；真实设备路径交给 CI");
        return;
    }
    if probe.code == NO_DEVICE {
        eprintln!(
            "SKIP(loud): 本环境报告 0 个音频设备（托管 runner 常态）⇒ BASELINE-005 在本环境\
**无法测量**（不是达标）。判据只验证了'如实报告没测到'那一半。"
        );
        assert!(probe.stdout.contains("NO-DEVICE:"));
        return;
    }
    assert_eq!(
        probe.code, UNMEASURABLE,
        "有设备但没有回环 ⇒ 必须是 {UNMEASURABLE}（不可判定），实际 {}\nstderr:\n{}",
        probe.code, probe.stderr
    );
    let fields = bench_of(&probe);
    if let Some(nominal) = fields.nominal_out_ms {
        assert!(nominal >= 0.0, "标称时延不许为负: {nominal}");
        assert!(
            probe.stdout.contains("NOMINAL direction=output"),
            "有标称数却没打印 NOMINAL 行"
        );
    } else {
        eprintln!(
            "SKIP(loud): 后端既不给 buffer_size() 也没接受 Fixed ⇒ 标称时延无事实来源，\
工具如实给了 none（不许猜）。"
        );
    }
    if let Some(jitter) = fields.jitter_out_p99_ms {
        assert!(jitter >= 0.0, "抖动 p99 不许为负: {jitter}");
        assert!(
            probe.stdout.contains("CALLBACK direction=output"),
            "有抖动数却没打印 CALLBACK 行"
        );
    } else {
        eprintln!("SKIP(loud): 输出流没有采到 ≥2 次回调 ⇒ 抖动 unmeasured（工具如实说 none）");
    }
    assert!(
        probe.stdout.contains("unmeasurable-without-loopback")
            || probe.stdout.contains("NO-LOOPBACK:"),
        "有设备但无回环时，工具必须说明为什么判不了"
    );
}

/// 判据 6：用法错误 ⇒ 退出码 2；`--help` ⇒ 0。
#[test]
fn usage_errors_exit_two_and_help_exits_zero() {
    let Some(binary) = example_binary() else {
        eprintln!("SKIP(loud): example 二进制不存在（见上一条判据的说明）");
        return;
    };
    let probe = run(&binary, &["--seconds", "0.2"]);
    if probe.code == DISABLED {
        eprintln!("SKIP(loud): example 是 --no-default-features 变体；CLI 契约交给 CI");
        return;
    }
    for args in [
        &["--nope"][..],
        &["--frames", "0"][..],
        &["--seconds", "-1"][..],
    ] {
        let result = run(&binary, args);
        assert_eq!(
            result.code, USAGE,
            "args={args:?} 应给出用法错误码 {USAGE}，实际 {}\nstderr:\n{}",
            result.code, result.stderr
        );
        assert!(result.stderr.contains("usage:"), "用法错误必须打印 usage");
        assert!(
            bench_line(&result.stdout).is_none(),
            "用法错误时不许输出 BENCH 读数行"
        );
    }
    let help = run(&binary, &["--help"]);
    assert_eq!(help.code, 0);
    assert!(help.stdout.contains("usage:"));
    assert!(bench_line(&help.stdout).is_none(), "--help 不该产生读数");
}
