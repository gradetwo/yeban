//! # 本机零依赖验证脚手架（**不是** cargo 目标）
//!
//! `yeban-render` 含 `rayon`（重依赖），因此 `scripts/gates/run-gates.sh crate yeban-render`
//! 在本机自动 **SKIP**（用户硬性纪律：M2 上不跑高耗 CPU 任务），渲染只能在 CI 上跑。
//! 但**收据的序列化 / 解析 / 判决判定是纯逻辑**，零第三方依赖 —— 因此它可以在本机真跑：
//!
//! ```text
//! rustc --edition 2024 --test -D warnings -W missing_docs \
//!   crates/yeban-render/examples/support/l1_receipt_tests.rs -o /tmp/l1-receipt-tests
//! /tmp/l1-receipt-tests
//! ```
//!
//! ## 覆盖范围（如实声明）
//!
//! - ✅ 覆盖：收据格式的往返与确定性、严格解析（缺字段/坏值不 panic）、
//!   **三种判决**（`L1-bit-exact` / `L2-within-budget` / `FAIL`）与两条反向判据
//!   （IEEE 类零容差、D32 ulp 预算）、指纹不可比、PDC 自洽、防抽样假绿。
//! - ❌ **不覆盖**：任何真正调用 `RenderPlan::execute` 的东西（需要 `rayon`）。
//!   "线程数变化不改变 digest"、"真实渲染两次收据逐字节相同"、"`longest_path_frames`
//!   等于注入的 PDC `L_max`" 这三条在 `tests/l1_digest_contract.rs` 里，**只由 CI 判定**。
//!
//! 本文件不被 `cargo` 自动发现（`examples/support/` 下没有 `main.rs`），
//! 因此不会给 CI 增加任何编译目标 —— 这是刻意的（已用 `cargo metadata` 实测）。

// 本文件是 `rustc --test` 的 **crate root**, 不是库；被包含模块的公开 API 在这里没有
// "外部消费者", 因此 `dead_code` 会误报。真实判定由 CI 的
// `cargo clippy -p yeban-render --all-targets -- -D warnings` 执行。
#![allow(dead_code)]

#[path = "l1_receipt.rs"]
mod l1_receipt;

use l1_receipt::{
    ABS_LIMIT, EdgeGain, Fingerprint, JudgeError, PathOp, Receipt, SampleClass, Threads, Toolchain,
    ULP_LIMIT, Verdict, judge, parse, probes_all, report, stats_of, to_text,
};

// ---------------------------------------------------------------------------
// 脚手架夹具
// ---------------------------------------------------------------------------

/// 参考工程形状的"测试收据"构造器：只填判决真正读到的字段。
fn receipt_with(
    samples: &[f32],
    class: SampleClass,
    gain: EdgeGain,
    threads: Threads,
    triple: &str,
) -> Receipt {
    let (sample_count, abs_max, sum_squares) = stats_of(samples);
    let ops = ops_for(gain);
    Receipt {
        fingerprint: Fingerprint {
            fixture: "reference-a".to_owned(),
            tracks: 32,
            frames: samples.len() as u64 / 2,
            channels: 2,
            sample_rate: 48_000,
            block_size: 128,
            threads,
            seed: 0x5EED,
            gain,
            latency: std::collections::BTreeMap::new(),
        },
        toolchain: Toolchain {
            target_arch: triple.split('-').next().unwrap_or("unknown").to_owned(),
            target_os: "test".to_owned(),
            target_env: String::new(),
            target_endian: "little".to_owned(),
            target_pointer_width: "64".to_owned(),
            target_triple: triple.to_owned(),
            rustc_version: "rustc test (脚手架)".to_owned(),
        },
        // 真 digest 由 `RenderOutput::digest_of`（SHA-256 over 位型）给出, 需要 `sha2`;
        // 本脚手架只需要一个"样本不同 ⇒ digest 不同"的**替身**, 用 FNV-1a 足够。
        digest: fake_digest(samples),
        longest_path_frames: 0,
        pdc_expected_frames: 0,
        sample_count,
        abs_max,
        sum_squares,
        ops,
        probes: probes_all(samples, class),
    }
}

fn ops_for(gain: EdgeGain) -> Vec<PathOp> {
    let ieee = |op: &str| PathOp {
        class: SampleClass::IeeeExact,
        op: op.to_owned(),
    };
    let mut ops = vec![
        ieee("add@master-serial-reduction"),
        ieee("mul@edge-gain-apply"),
        ieee("add@tone-mix"),
        ieee("cast@tone-integer-to-f32"),
    ];
    if matches!(gain, EdgeGain::Db(_)) {
        ops.push(PathOp {
            class: SampleClass::Transcendental,
            op: "libm::powf@db-to-linear".to_owned(),
        });
    }
    ops
}

/// 确定性 digest 替身：样本位型变了它一定变（判据只需要这个性质）。
fn fake_digest(samples: &[f32]) -> [u8; 32] {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for sample in samples {
        for byte in sample.to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    let mut out = [0u8; 32];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = ((hash >> ((index % 8) * 8)) & 0xFF) as u8;
    }
    out
}

/// 典型的 IEEE 精确类样本：全是 2 的负幂，1 ulp 的差异不会碰到零。
const IEEE_SAMPLES: [f32; 4] = [0.5, -0.25, 0.125, -0.0625];

fn with_sample(samples: &[f32], index: usize, value: f32) -> Vec<f32> {
    let mut out = samples.to_vec();
    out[index] = value;
    out
}

fn plus_ulp(value: f32, ulps: u32) -> f32 {
    f32::from_bits(value.to_bits() + ulps)
}

// ---------------------------------------------------------------------------
// 判据
// ---------------------------------------------------------------------------

/// 判据 1a：序列化 → 解析 → 再次序列化 = 逐字节相同（往返无损）。
#[test]
fn round_trip_is_byte_exact() {
    let receipt = receipt_with(
        &IEEE_SAMPLES,
        SampleClass::IeeeExact,
        EdgeGain::Identity,
        Threads::Auto,
        "aarch64-apple-darwin",
    );
    let text = to_text(&receipt);
    let parsed = parse(&text).expect("自己写出的收据必须能解析回来");
    assert_eq!(parsed, receipt, "往返后数据模型必须完全相等");
    assert_eq!(to_text(&parsed), text, "往返后文本必须逐字节相同");
}

/// 判据 1b：极端位型也必须精确往返（`-0.0`、最小次正规、最大规格化、±1、1e±30）。
///
/// 这正是"用位型而不是数值"的理由：`-0.0` 与 `+0.0` 数值相等但位型不同。
#[test]
fn extreme_bit_patterns_round_trip() {
    let samples = [
        0.0f32,
        -0.0,
        f32::from_bits(1),
        f32::from_bits(0x007f_ffff),
        f32::from_bits(0x0080_0000),
        f32::from_bits(0x7f7f_ffff),
        1.0,
        -1.0,
        1.0e-30,
        -1.0e30,
    ];
    let receipt = receipt_with(
        &samples,
        SampleClass::IeeeExact,
        EdgeGain::Identity,
        Threads::Fixed(4),
        "x86_64-unknown-linux-gnu",
    );
    let text = to_text(&receipt);
    let parsed = parse(&text).expect("极端位型必须能往返");
    assert_eq!(parsed.probes, receipt.probes);
    assert_eq!(parsed, receipt);
}

/// 判据 2：同一台机器两次导出 ⇒ 收据**逐字节相同**（确定性；格式里不许有时间戳）。
#[test]
fn two_exports_of_the_same_input_are_byte_identical() {
    let build = || {
        receipt_with(
            &with_sample(&IEEE_SAMPLES, 1, 0.75),
            SampleClass::Transcendental,
            EdgeGain::Db(3.0),
            Threads::Auto,
            "aarch64-apple-darwin",
        )
    };
    assert_eq!(to_text(&build()), to_text(&build()));
}

/// 判据 3：收据自带**非空**的目标三元组与架构（让收据能自证来自哪个机器）。
#[test]
fn receipt_carries_a_non_empty_target_identity() {
    let receipt = receipt_with(
        &IEEE_SAMPLES,
        SampleClass::IeeeExact,
        EdgeGain::Identity,
        Threads::Auto,
        "x86_64-unknown-linux-gnu",
    );
    assert!(!receipt.toolchain.target_triple.is_empty());
    assert!(!receipt.toolchain.target_arch.is_empty());
    let text = to_text(&receipt);
    assert!(text.contains("target_triple x86_64-unknown-linux-gnu"));
    assert!(text.contains("target_arch x86_64"));
}

/// 判据 4：完全相同 ⇒ `L1-bit-exact`，退出码 0。
#[test]
fn identical_receipts_are_l1_bit_exact() {
    let a = receipt_with(
        &IEEE_SAMPLES,
        SampleClass::IeeeExact,
        EdgeGain::Identity,
        Threads::Auto,
        "aarch64-apple-darwin",
    );
    let b = receipt_with(
        &IEEE_SAMPLES,
        SampleClass::IeeeExact,
        EdgeGain::Identity,
        Threads::Auto,
        "x86_64-unknown-linux-gnu",
    );
    let judgement = judge(&a, &b).expect("形状相同 ⇒ 可比");
    assert_eq!(judgement.verdict, Verdict::L1BitExact);
    assert_eq!(judgement.verdict.exit_code(), 0);
    assert_eq!(
        judgement.gate, "MUST-GATE-003",
        "不同目标 ⇒ 这是 MUST-GATE-003 的读数"
    );
    assert_eq!(judgement.max_abs_diff, 0.0);
}

/// 判据 5：超越函数类上的人工 **1 ulp** 差异 ⇒ `L2-within-budget`，且点名到样本。
#[test]
fn one_ulp_on_a_transcendental_sample_is_within_budget() {
    let a = receipt_with(
        &IEEE_SAMPLES,
        SampleClass::Transcendental,
        EdgeGain::Db(3.0),
        Threads::Auto,
        "aarch64-apple-darwin",
    );
    let b_samples = with_sample(&IEEE_SAMPLES, 2, plus_ulp(IEEE_SAMPLES[2], 1));
    let b = receipt_with(
        &b_samples,
        SampleClass::Transcendental,
        EdgeGain::Db(3.0),
        Threads::Auto,
        "x86_64-unknown-linux-gnu",
    );
    let judgement = judge(&a, &b).expect("可比");
    assert_eq!(judgement.verdict, Verdict::L2WithinBudget);
    assert_eq!(judgement.verdict.exit_code(), 0);
    assert_eq!(judgement.reason, "transcendental-only-within-budget");
    assert_eq!(judgement.max_abs_index, Some(2), "必须点名到具体样本");
    assert_eq!(judgement.max_abs_class, Some(SampleClass::Transcendental));
    assert_eq!(judgement.max_ulp, 1);
    assert_eq!(judgement.ieee_diffs, 0);
    assert_eq!(judgement.transcendental_diffs, 1);
}

/// 判据 6：大幅偏差 ⇒ `FAIL`、非零退出码、**点名样本**（判据 ⑤）。
#[test]
fn a_large_deviation_fails_and_names_the_sample() {
    let a = receipt_with(
        &IEEE_SAMPLES,
        SampleClass::Transcendental,
        EdgeGain::Db(3.0),
        Threads::Auto,
        "aarch64-apple-darwin",
    );
    let b_samples = with_sample(&IEEE_SAMPLES, 3, IEEE_SAMPLES[3] * 2.0);
    let b = receipt_with(
        &b_samples,
        SampleClass::Transcendental,
        EdgeGain::Db(3.0),
        Threads::Auto,
        "x86_64-unknown-linux-gnu",
    );
    let judgement = judge(&a, &b).expect("可比");
    assert_eq!(judgement.verdict, Verdict::Fail);
    assert_eq!(judgement.verdict.exit_code(), 1);
    assert_eq!(judgement.reason, "abs-budget-exceeded");
    assert!(!judgement.abs_budget_ok);
    assert_eq!(judgement.max_abs_index, Some(3));
    assert_eq!(judgement.offenders_total, 1);
    assert_eq!(judgement.offenders.len(), 1);
    assert_eq!(judgement.offenders[0].index, 3);
    let text = report(&a, &b, &judgement);
    assert!(text.starts_with("VERDICT FAIL\n"));
    assert!(text.contains("offender index=3 class=T"));
}

/// 判据 7（**D32 的核心**）：IEEE 精确类上哪怕只有 **1 ulp** 也必须 `FAIL`。
#[test]
fn one_ulp_on_an_ieee_exact_sample_fails() {
    let a = receipt_with(
        &IEEE_SAMPLES,
        SampleClass::IeeeExact,
        EdgeGain::Identity,
        Threads::Auto,
        "aarch64-apple-darwin",
    );
    let b_samples = with_sample(&IEEE_SAMPLES, 0, plus_ulp(IEEE_SAMPLES[0], 1));
    let b = receipt_with(
        &b_samples,
        SampleClass::IeeeExact,
        EdgeGain::Identity,
        Threads::Auto,
        "x86_64-unknown-linux-gnu",
    );
    let judgement = judge(&a, &b).expect("可比");
    assert_eq!(judgement.verdict, Verdict::Fail);
    assert_eq!(judgement.reason, "ieee-exact-sample-differs");
    assert_eq!(judgement.ieee_diffs, 1);
    assert!(
        judgement.abs_budget_ok,
        "1 ulp 的绝对误差极小 —— 红的原因必须是类别, 不是幅度"
    );
    assert!(judgement.max_abs_diff < ABS_LIMIT);
}

/// 判据 8：绝对预算通过、但 **D32 的 ulp 预算**超了 ⇒ 仍然 `FAIL`（两条预算都要守）。
#[test]
fn d32_ulp_budget_fails_even_when_the_absolute_budget_passes() {
    let tiny = 1.0e-30f32;
    let a = receipt_with(
        &[tiny, tiny, tiny, tiny],
        SampleClass::Transcendental,
        EdgeGain::Db(3.0),
        Threads::Auto,
        "aarch64-apple-darwin",
    );
    let shifted = plus_ulp(tiny, u32::try_from(ULP_LIMIT + 1).unwrap());
    let b = receipt_with(
        &[tiny, tiny, shifted, tiny],
        SampleClass::Transcendental,
        EdgeGain::Db(3.0),
        Threads::Auto,
        "x86_64-unknown-linux-gnu",
    );
    let judgement = judge(&a, &b).expect("可比");
    assert_eq!(judgement.verdict, Verdict::Fail);
    assert_eq!(judgement.reason, "d32-ulp-budget-exceeded");
    assert!(judgement.abs_budget_ok);
    assert!(!judgement.ulp_budget_ok);
    assert_eq!(judgement.max_ulp, ULP_LIMIT + 1);
}

/// 判据 9（`ARCH-DET-002`）：线程数**不参与**可比性判定 —— 否则这条判据无法表达。
#[test]
fn thread_count_does_not_make_receipts_incomparable() {
    let samples = [0.5f32, -0.25, 0.125, -0.0625];
    let one = receipt_with(
        &samples,
        SampleClass::IeeeExact,
        EdgeGain::Identity,
        Threads::Fixed(1),
        "aarch64-apple-darwin",
    );
    let four = receipt_with(
        &samples,
        SampleClass::IeeeExact,
        EdgeGain::Identity,
        Threads::Fixed(4),
        "aarch64-apple-darwin",
    );
    let judgement = judge(&one, &four).expect("线程数不同仍然可比");
    assert!(judgement.threads_differ);
    assert_eq!(
        judgement.verdict,
        Verdict::L1BitExact,
        "线程数不改变母带字节 [ARCH-DET-002]"
    );
    assert_eq!(
        judgement.gate, "MUST-GATE-002",
        "同目标 ⇒ 这是 MUST-GATE-002 的读数"
    );
}

/// 判据 10：**字段缺失**必须给明确错误，而不是 panic。
#[test]
fn missing_required_fields_are_clear_errors() {
    let receipt = receipt_with(
        &IEEE_SAMPLES,
        SampleClass::IeeeExact,
        EdgeGain::Identity,
        Threads::Auto,
        "aarch64-apple-darwin",
    );
    let text = to_text(&receipt);
    for (field, needle) in [
        ("target_triple", "target_triple aarch64-apple-darwin\n"),
        ("digest", &format!("digest {}\n", receipt.digest_hex())[..]),
        ("threads", "threads auto\n"),
    ] {
        let broken = text.replace(needle, "");
        assert_ne!(broken, text, "注入必须真的删掉了 {field}");
        let error = parse(&broken).expect_err("缺字段必须报错");
        assert!(
            error.message.contains(field),
            "错误信息必须点名缺失字段 {field}: {error}"
        );
    }
}

/// 判据 11：**格式错误**（首行、未知字段、坏十六进制、未知类别码、坏数字）全部报错不 panic。
#[test]
fn malformed_receipts_are_errors_not_panics() {
    let receipt = receipt_with(
        &IEEE_SAMPLES,
        SampleClass::IeeeExact,
        EdgeGain::Identity,
        Threads::Auto,
        "aarch64-apple-darwin",
    );
    let text = to_text(&receipt);
    let mutations: [(&str, String); 5] = [
        (
            "首行版本错",
            text.replacen("yeban-l1-receipt v1", "yeban-l1-receipt v2", 1),
        ),
        ("未知字段", format!("{text}unknown_field 1\n")),
        ("坏十六进制", text.replace("sample 0 E", "sample 0 E 0x")),
        ("未知类别码", text.replace("sample 0 E", "sample 0 X")),
        (
            "不可比数字",
            text.replacen("tracks 32", "tracks thirty-two", 1),
        ),
    ];
    for (label, broken) in mutations {
        let error = parse(&broken).expect_err(label);
        assert!(error.line > 0, "{label}: 必须给出行号: {error}");
    }
    assert!(parse("").is_err(), "空文本必须报错");
    assert!(parse("这不是收据").is_err());
}

/// 判据 12：**抽样/截断**的收据必须被拒 —— 否则"最大误差"这条判据会变成假绿。
#[test]
fn sampled_or_truncated_receipts_are_refused() {
    let receipt = receipt_with(
        &IEEE_SAMPLES,
        SampleClass::IeeeExact,
        EdgeGain::Identity,
        Threads::Auto,
        "aarch64-apple-darwin",
    );
    let text = to_text(&receipt);
    let line = format!(
        "sample 0 E {:08x},{:08x},{:08x},{:08x}\n",
        IEEE_SAMPLES[0].to_bits(),
        IEEE_SAMPLES[1].to_bits(),
        IEEE_SAMPLES[2].to_bits(),
        IEEE_SAMPLES[3].to_bits()
    );
    assert!(text.contains(&line), "夹具的样本行形状变了, 判据需同步");
    let truncated = text.replace(&line, "sample 0 E 3f000000\n");
    let error = parse(&truncated).expect_err("抽样必须被拒");
    assert!(error.message.contains("样本表不完整"), "{error}");
    // sample_count 与实际样本数不符
    let miscounted = text.replace("sample_count 4", "sample_count 5");
    assert!(parse(&miscounted).is_err());
}

/// 判据 13（要求 ⑧）：`longest_path_frames` 与导出侧复算的 PDC 读数不一致 ⇒ 收据被拒。
#[test]
fn pdc_inconsistency_is_refused() {
    let mut receipt = receipt_with(
        &IEEE_SAMPLES,
        SampleClass::IeeeExact,
        EdgeGain::Identity,
        Threads::Auto,
        "aarch64-apple-darwin",
    );
    receipt.longest_path_frames = 192;
    receipt.pdc_expected_frames = 192;
    let text = to_text(&receipt);
    assert!(parse(&text).is_ok(), "自洽的 PDC 读数必须能解析");
    let broken = text.replace("pdc_expected_frames 192", "pdc_expected_frames 191");
    let error = parse(&broken).expect_err("PDC 不自洽必须被拒");
    assert!(error.message.contains("PDC"), "{error}");
}

/// 判据 14：指纹不同（轨道数/frames/种子/增益/延迟）⇒ **不可比**（退出码 2 的语义）。
#[test]
fn fingerprint_mismatch_is_incomparable() {
    let a = receipt_with(
        &IEEE_SAMPLES,
        SampleClass::IeeeExact,
        EdgeGain::Identity,
        Threads::Auto,
        "aarch64-apple-darwin",
    );
    let mut b = a.clone();
    b.fingerprint.tracks = 16;
    let error = judge(&a, &b).expect_err("形状不同必须不可比");
    assert!(matches!(error, JudgeError::Incomparable(_)));
    assert!(error.to_string().contains("工程形状不同"));

    let mut c = a.clone();
    c.fingerprint.gain = EdgeGain::Db(3.0);
    assert!(judge(&a, &c).is_err(), "增益设置不同 ⇒ 不可比");

    let mut d = a.clone();
    d.ops.push(PathOp {
        class: SampleClass::Transcendental,
        op: "libm::powf@db-to-linear".to_owned(),
    });
    assert!(judge(&a, &d).is_err(), "路径声明不同 ⇒ 不可比");
}

/// 判据 15：digest 与样本表**互相矛盾** ⇒ 不可比（防止被篡改或"只比 digest"的误用）。
#[test]
fn digest_sample_contradiction_is_incomparable() {
    let a = receipt_with(
        &IEEE_SAMPLES,
        SampleClass::IeeeExact,
        EdgeGain::Identity,
        Threads::Auto,
        "aarch64-apple-darwin",
    );
    // digest 相同、样本不同
    let mut same_digest = a.clone();
    same_digest.probes[0].bits = plus_ulp(IEEE_SAMPLES[0], 1).to_bits();
    assert!(judge(&a, &same_digest).is_err());
    // 样本相同、digest 不同
    let mut other_digest = a.clone();
    other_digest.digest[0] ^= 0xFF;
    assert!(judge(&a, &other_digest).is_err());
}

/// 判据 16：有 `T` 类样本却没有 `op T ...` 声明 ⇒ 收据自相矛盾，解析即被拒。
#[test]
fn transcendental_sample_without_declaration_is_refused() {
    let receipt = receipt_with(
        &IEEE_SAMPLES,
        SampleClass::Transcendental,
        EdgeGain::Db(3.0),
        Threads::Auto,
        "aarch64-apple-darwin",
    );
    let text = to_text(&receipt);
    let broken = text.replace("op T libm::powf@db-to-linear\n", "");
    let error = parse(&broken).expect_err("缺 op T 声明必须被拒");
    assert!(error.message.contains("自相矛盾"), "{error}");
}

/// 判据 17：报告首行是机器可读的判决，且失败时点名越界样本。
#[test]
fn report_is_machine_readable_and_names_offenders() {
    let a = receipt_with(
        &IEEE_SAMPLES,
        SampleClass::IeeeExact,
        EdgeGain::Identity,
        Threads::Auto,
        "aarch64-apple-darwin",
    );
    let b_samples = with_sample(&IEEE_SAMPLES, 1, IEEE_SAMPLES[1] * 4.0);
    let b = receipt_with(
        &b_samples,
        SampleClass::IeeeExact,
        EdgeGain::Identity,
        Threads::Auto,
        "x86_64-unknown-linux-gnu",
    );
    let judgement = judge(&a, &b).expect("可比");
    let text = report(&a, &b, &judgement);
    for needle in [
        "VERDICT FAIL",
        "reason=ieee-exact-sample-differs",
        "gate=MUST-GATE-003",
        "same_target=false",
        "offender index=1 class=E",
        "longest_path_frames_a=0",
        "pdc_expected_frames_a=0",
        // `1e-6` 的 f64 最近值略小于 1e-6；报告如实打印 17 位有效数字。
        "limit_abs=9.99999999999999955e-7",
    ] {
        assert!(text.contains(needle), "报告缺少 `{needle}`:\n{text}");
    }
}
