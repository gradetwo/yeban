//! **跨架构 L1/L2 收据的端到端契约判据**（真渲染；本机因 `rayon` 而只能交给 CI）。
//!
//! 本文件补上 [`examples/support/l1_receipt_tests.rs`] 覆盖不到的部分 —— 那一份是
//! 收据"格式/解析/判决"的**纯逻辑**判据，本机可跑；这一份要求真的调用
//! `RenderPlan::execute`，因此**只由 CI 判定**（`scripts/gates/run-gates.sh crate yeban-render`
//! 在本机 SKIP 的设计如此）。
//!
//! ## 判据地图（每一条都能变红）
//!
//! | # | 判据 | 被什么注入破坏 |
//! | :--- | :--- | :--- |
//! | 1 | 同一台机器两次导出 ⇒ 收据**逐字节相同**（无时间戳/无耗时） | 往收据里加时间戳/主机名 |
//! | 2 | 收据自证架构：`target_arch == std::env::consts::ARCH`，`target_triple` 非空且等于 `rustc -vV` 的 host | 写死一个三元组 |
//! | 3 | **线程数 1/2/4 ⇒ digest 相同**（`[ARCH-DET-002]` 的实测） | 归约改成按完成顺序 |
//! | 4 | 收据里的 digest/统计量/样本位型与 `RenderOutput` **逐位一致**（读数不是编的） | 报估算值 |
//! | 5 | `longest_path_frames` == 注入的 PDC `L_max` == 导出侧复算值（判据 ⑧） | PDC 只报 0 |
//! | 6 | 注入延迟**真的改变了音频**（否则判据 5 是空转） | 延迟线变成旁路 |
//! | 7 | `gain_db none` ⇒ 全部样本 `E` 类且无 `op T` 声明（D32 分类如实） | 无中生有声明超越函数 |
//! | 8 | `gain_db 3` ⇒ 全部样本 `T` 类且有 `op T` 声明 | 漏报超越函数 |
//! | 9 | 真实收据的**超越函数类 1 ulp 差异** ⇒ `L2-within-budget`（`MUST-GATE-003` 的可达形态） | 把预算改成 0 |
//! | 10 | 收据文本往返 = 数据模型相等（导出/比较两个进程之间的契约） | 序列化丢字段 |
//!
//! 夹具取小参数（8 轨 / 512 帧），因为这里验的是**不变量**而不是吞吐。

// `l1_receipt` 与 `reference_project_a` 是 `export_pipeline` 的子模块 ——
// 与 `examples/export_l1_receipt.rs` 引入的是**同一份文件**（不允许漂移）。
#[allow(dead_code)]
#[path = "../examples/support/export_pipeline.rs"]
mod export_pipeline;

use export_pipeline::l1_receipt::{
    EdgeGain, JudgeError, SampleClass, Threads, Verdict, judge, parse, to_text,
};
use export_pipeline::{FixtureOptions, LatencyMode, build_receipt};

/// 判据用的**小夹具**：形状与 `reference-a` 完全一致，只是短。
fn fixture(gain: EdgeGain, threads: Threads, latency: LatencyMode) -> FixtureOptions {
    FixtureOptions {
        tracks: 8,
        frames: 512,
        threads,
        gain,
        latency,
    }
}

/// 判据 1：同一台机器两次导出 ⇒ 收据**逐字节相同**。
///
/// 这同时钉住了"收据里不许有时间戳/耗时/主机名"这条设计约束。
#[test]
fn two_real_exports_are_byte_identical() {
    let options = fixture(EdgeGain::Identity, Threads::Auto, LatencyMode::None);
    let first = build_receipt(&options).expect("渲染应成功");
    let second = build_receipt(&options).expect("渲染应成功");
    assert_eq!(
        to_text(&first.receipt),
        to_text(&second.receipt),
        "同一输入两次导出必须逐字节相同（判据 ①）"
    );
    assert_eq!(first.receipt.digest, second.receipt.digest);
}

/// 判据 2：收据自证架构。`target_arch` 必须等于编译期事实，`target_triple` 必须非空
/// 且等于 `rustc -vV` 的 host（取不到 `rustc` 时至少由 arch/os 合成）。
#[test]
fn receipt_self_identifies_the_target() {
    let reading = build_receipt(&fixture(
        EdgeGain::Identity,
        Threads::Auto,
        LatencyMode::None,
    ))
    .expect("渲染应成功");
    let toolchain = &reading.receipt.toolchain;
    assert_eq!(toolchain.target_arch, std::env::consts::ARCH);
    assert_eq!(toolchain.target_os, std::env::consts::OS);
    assert!(!toolchain.target_triple.is_empty());
    assert!(!toolchain.rustc_version.is_empty());
    let (host, _) = export_pipeline::rustc_identity();
    if let Some(host) = host {
        assert_eq!(
            toolchain.target_triple, host,
            "target_triple 必须来自 `rustc -vV` 的 host"
        );
    }
    let text = to_text(&reading.receipt);
    assert!(text.contains(&format!("target_arch {}", std::env::consts::ARCH)));
}

/// 判据 3：**线程数不改变 digest**（`[ARCH-DET-002]` 的实测，也是 `ARCH-DET-001` 的 L1 条件）。
#[test]
fn thread_counts_do_not_change_the_digest() {
    let one = build_receipt(&fixture(
        EdgeGain::Identity,
        Threads::Fixed(1),
        LatencyMode::None,
    ))
    .expect("渲染应成功");
    let four = build_receipt(&fixture(
        EdgeGain::Identity,
        Threads::Fixed(4),
        LatencyMode::None,
    ))
    .expect("渲染应成功");
    assert_eq!(
        one.receipt.digest, four.receipt.digest,
        "1 线程与 4 线程的位级摘要必须相同 [ARCH-DET-002]"
    );
    let judgement = judge(&one.receipt, &four.receipt).expect("同一夹具 ⇒ 可比");
    assert_eq!(judgement.verdict, Verdict::L1BitExact);
    assert!(judgement.threads_differ, "线程数不同必须如实报告");
    assert_eq!(
        judgement.gate, "MUST-GATE-002",
        "同目标 ⇒ 这是同平台 bit-exact"
    );
}

/// 判据 4：收据里的读数**逐位**来自 `RenderOutput`（digest / 统计量 / 样本位型）。
#[test]
fn receipt_readings_come_from_the_render_output() {
    let reading = build_receipt(&fixture(
        EdgeGain::Db(3.0),
        Threads::Auto,
        LatencyMode::None,
    ))
    .expect("渲染应成功");
    let receipt = &reading.receipt;
    assert_eq!(receipt.sample_count, reading.samples.len() as u64);
    assert_eq!(
        receipt.sample_count,
        receipt.fingerprint.frames * receipt.fingerprint.channels as u64
    );
    // 独立复算统计量：顺序 `f64` 累加是 IEEE 精确的，必须**完全相等**（不是近似）。
    let mut abs_max = 0.0f64;
    let mut sum_squares = 0.0f64;
    for (index, sample) in reading.samples.iter().enumerate() {
        let value = f64::from(*sample);
        if value.abs() > abs_max {
            abs_max = value.abs();
        }
        sum_squares += value * value;
        assert_eq!(
            receipt.probes[index].bits,
            sample.to_bits(),
            "收据第 {index} 个样本位型必须等于渲染输出"
        );
    }
    assert_eq!(receipt.abs_max, abs_max);
    assert_eq!(receipt.sum_squares, sum_squares);
}

/// 判据 5：`longest_path_frames` 与**注入的 PDC `L_max`** 一致（要求 ⑧）。
///
/// 阶梯延迟 `(i % 4) * 64` 在 8 条轨道上给出 `L_max = 3 * 64 = 192` 帧。
#[test]
fn longest_path_frames_matches_the_injected_l_max() {
    let expected = 3 * export_pipeline::LATENCY_STEP;
    let reading = build_receipt(&fixture(
        EdgeGain::Identity,
        Threads::Auto,
        LatencyMode::Staircase,
    ))
    .expect("渲染应成功");
    let receipt = &reading.receipt;
    assert_eq!(receipt.longest_path_frames, expected);
    assert_eq!(receipt.pdc_expected_frames, expected);
    assert_eq!(
        receipt.fingerprint.latency.len(),
        8,
        "8 条轨道的延迟都要入账"
    );
    assert!(
        receipt
            .fingerprint
            .latency
            .values()
            .all(|frames| *frames % export_pipeline::LATENCY_STEP == 0)
    );
}

/// 判据 6：注入延迟**真的改变了音频** —— 否则判据 5 只是"两个 0 相等"的空转。
#[test]
fn latency_injection_actually_changes_the_audio() {
    let none = build_receipt(&fixture(
        EdgeGain::Identity,
        Threads::Auto,
        LatencyMode::None,
    ))
    .expect("渲染应成功");
    let staircase = build_receipt(&fixture(
        EdgeGain::Identity,
        Threads::Auto,
        LatencyMode::Staircase,
    ))
    .expect("渲染应成功");
    assert_ne!(
        none.receipt.digest, staircase.receipt.digest,
        "注入的 PDC 补偿必须真的作用到样本上"
    );
    assert_ne!(none.samples, staircase.samples);
}

/// 判据 7：`gain_db none` ⇒ 路径上**没有**超越函数：全部样本 `E` 类、无 `op T`。
#[test]
fn ieee_only_fixture_declares_no_transcendental_ops() {
    let reading = build_receipt(&fixture(
        EdgeGain::Identity,
        Threads::Auto,
        LatencyMode::None,
    ))
    .expect("渲染应成功");
    assert!(!reading.receipt.has_transcendental_ops());
    assert!(
        reading
            .receipt
            .probes
            .iter()
            .all(|probe| probe.class == SampleClass::IeeeExact)
    );
    let text = to_text(&reading.receipt);
    assert!(!text.contains("op T "), "不应无中生有地声明超越函数");
    assert!(
        text.contains("class T transcendental"),
        "类别字母表必须显式声明"
    );
}

/// 判据 8：`gain_db 3` ⇒ 增益走 `libm::powf`，全部样本 `T` 类且有 `op T` 声明。
#[test]
fn gain_fixture_declares_transcendental_ops() {
    let reading = build_receipt(&fixture(
        EdgeGain::Db(3.0),
        Threads::Auto,
        LatencyMode::None,
    ))
    .expect("渲染应成功");
    assert!(reading.receipt.has_transcendental_ops());
    assert!(
        reading
            .receipt
            .probes
            .iter()
            .all(|probe| probe.class == SampleClass::Transcendental)
    );
    assert!(to_text(&reading.receipt).contains("op T libm::powf@db-to-linear"));
}

/// 判据 9：真实夹具上，**超越函数类**的 1 ulp 扰动 ⇒ `L2-within-budget`
/// —— 这就是 `MUST-GATE-003` 想要判定的那个形态。
#[test]
fn a_one_ulp_transcendental_perturbation_lands_in_the_l2_budget() {
    let base = build_receipt(&fixture(
        EdgeGain::Db(3.0),
        Threads::Auto,
        LatencyMode::None,
    ))
    .expect("渲染应成功");
    let mut perturbed = base.receipt.clone();
    let index = perturbed.probes.len() / 3;
    let original = perturbed.probes[index].bits;
    perturbed.probes[index].bits = original + 1;
    // digest 必须随之改变（否则比较器会判"收据自相矛盾"）—— 这就是真实 SHA-256 的性质。
    perturbed.digest[0] ^= 0x01;
    let judgement = judge(&base.receipt, &perturbed).expect("同一指纹 ⇒ 可比");
    assert_eq!(judgement.verdict, Verdict::L2WithinBudget);
    assert_eq!(judgement.verdict.exit_code(), 0);
    assert_eq!(judgement.max_ulp, 1);
    assert_eq!(judgement.max_abs_index, Some(index as u64));
    assert!(judgement.max_abs_diff < export_pipeline::l1_receipt::ABS_LIMIT);
}

/// 判据 10：收据文本往返 = 数据模型相等（导出进程与比较进程之间的契约）。
#[test]
fn receipt_text_round_trips() {
    let reading = build_receipt(&fixture(
        EdgeGain::Db(-6.0),
        Threads::Fixed(2),
        LatencyMode::Staircase,
    ))
    .expect("渲染应成功");
    let text = to_text(&reading.receipt);
    let parsed = parse(&text).expect("自己写出的收据必须能解析");
    assert_eq!(parsed, reading.receipt);
    assert_eq!(to_text(&parsed), text);
    // 不可比的情形也必须给明确错误而不是 panic。
    let mut other = reading.receipt.clone();
    other.fingerprint.tracks = 4;
    assert!(matches!(
        judge(&reading.receipt, &other),
        Err(JudgeError::Incomparable(_))
    ));
}
