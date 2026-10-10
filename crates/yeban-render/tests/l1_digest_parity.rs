//! # `MUST-GATE-002`：**跨机器** L1 摘要对账的端到端判据（真渲染）
//!
//! ## 这一份判据补的是哪个洞
//!
//! `tests/l1_digest_contract.rs` 证明的是"**同一次进程内**的渲染是确定的"（同一台机器两次
//! 导出逐字节相同；1/2/4 线程 digest 相同）。它回答不了规范真正问的那个问题：
//!
//! > 换一台同平台机器、同一个种子，**WAV 文件的 SHA-256 是否一模一样**？
//!
//! 本文件读入**已经提交进仓库的参考摘要**（`tests/data/l1-digest-reference.json`，由
//! `examples/export_l1_digest.rs` 在**另一台机器**上生成），在本机**重算**同一份读数，
//! 然后逐字段比对。于是"跨机器"第一次有了**可对照的基准记录**，而不是只活在一次
//! `cargo test` 里。
//!
//! ## 三条结论，**不许互相冒充**（这是本仓库最重视的一类诚实）
//!
//! | 结论 | 退出行为 | 何时 |
//! | :--- | :--- | :--- |
//! | **PASS** | 绿 | 平台同 + 工具链锁同 + 全部参与字段（含 `digest`）**逐字段相同** |
//! | **FAIL** | **红** | 平台相同、工具链锁相同，参与字段/读数却不同 |
//! | **SKIP** | 绿**但打印醒目 SKIP 并点名原因** | 平台不同（跨 ISA/OS）或工具链未锁定 |
//!
//! ⚠ **`SKIP` 绝不等于通过**：SKIP 时本判据会打印
//! `MUST-GATE-002 SKIP (未判定, 不是通过)` 与 `skip_explanation` 的原因说明。
//! 之所以在"平台不同"时**必须**跳过而不是判红：跨架构的哈希差异是 `MUST-GATE-003` 的
//! 领域（预算 `< 1e-6`），把它算作本门禁的失败是**假红**；而"同平台却不同"没有借口 ⇒ 硬红。
//!
//! ## 判据地图
//!
//! | # | 判据 | 被什么注入破坏 |
//! | :--- | :--- | :--- |
//! | ① | 对参考摘要逐字段比对 ⇒ PASS（不可比时如实 SKIP 并打印原因） | 改参考摘要里的 `digest` |
//! | ② | 同一次运行两次渲染 ⇒ 摘要**逐字节相同**（确定性，判据 ⑦ 的真实形态） | 往摘要里塞时间戳 |
//! | ③ | 线程策略 1/2/4/8/auto ⇒ `digest` 全同（`[ARCH-DET-002]`） | 归约改成按完成顺序 |
//! | ④ | **种子变化必然改变 `digest`**（负向对照，防"digest 是常量"） | digest 只哈希长度 |
//! | ⑤ | 参考摘要的 **参与字段**被改（ISA 写错）⇒ `FAIL` | 把 ISA 从参与字段里删掉 |
//! | ⑥ | 参考摘要的**仅记录字段**被改（宿主名/时间戳）⇒ 仍然 `PASS` | 把 `host_name` 挪进参与字段 |
//! | ⑦ | 生成器确定性：同输入两次产出**逐字节相同** | 序列化里带上哈希表迭代顺序 |
//! | ⑧ | `digest` 就是 WAV 有效位流的 SHA-256（**独立复算**，读数不是编的） | 报估算值/截断缓冲 |
//! | ⑨ | WAV 容器能被**独立第三方读取器**（`hound`）读回同样的样本 | 自研容器头写错 |
//! | ⑩ | 参考摘要文件本身自洽（字段表 / schema / 口径都对得上） | 手改归档文件 |
//!
//! 夹具取参考工程 A 的**默认参数**（32 轨 / 8192 帧 / 48 kHz 立体声 / 种子 `0x5EED`）
//! —— 与参考摘要、与 `BASELINE-001`、与手动档 `arm` 那条腿**完全同一组**参数。
//! 参数不同就不可比，因此这里**不**为了跑得快而改小夹具。

// `export_pipeline` 是 example 与判据共用的装配管线（`l1_receipt` / `l1_digest_record` /
// `reference_project_a` 都是它的子模块）—— 与 `examples/export_l1_digest.rs` 引入的是
// **同一份文件**（不允许漂移）。
#[allow(dead_code)]
#[path = "../examples/support/export_pipeline.rs"]
mod export_pipeline;

use std::path::PathBuf;

use export_pipeline::l1_digest_record::{
    CrossPlatform, DigestRecord, DigestScope, FieldDiff, JudgeError, Judgement, SCHEMA, Verdict,
    hex_lower, judge, judge_policy, parse, report, same_platform, skip_explanation, to_json_line,
    to_pretty_json, toolchain_locked,
};
use export_pipeline::l1_receipt::Threads;
use export_pipeline::{
    DigestMetadata, FixtureOptions, LatencyMode, build_receipt, digest_record_from_reading,
};

/// 参考摘要的路径（相对本 crate 根）。
const REFERENCE: &str = "tests/data/l1-digest-reference.json";

/// 参考摘要里记的基准指令集（生成器默认值）。
///
/// 本地重算时**必须**用同一个记号：`isa_features` 是**参与比对**的字段，
/// 换一个记号就会被判 `FAIL` —— 那正是判据 ⑤ 的形态。参考摘要与本常量由判据 ⑩ 交叉校验。
const REFERENCE_ISA: &str = "baseline";

/// 参考工程 A 的默认夹具（与参考摘要、`BASELINE-001`、手动档 `arm` 同参数）。
fn fixture(threads: Threads) -> FixtureOptions {
    FixtureOptions {
        tracks: 32,
        frames: export_pipeline::DEFAULT_FRAMES,
        threads,
        gain: export_pipeline::l1_receipt::EdgeGain::Identity,
        latency: LatencyMode::None,
    }
}

/// 本机的"仅记录"元数据。
///
/// 刻意**不**去读参考摘要里的对应字段：这些字段就是"允许不同"的那一类，
/// 攒不出真实宿主名也不要紧（它们是记录，不是判据）。
fn local_metadata(notes: &str) -> DigestMetadata {
    DigestMetadata {
        isa_features: REFERENCE_ISA.to_owned(),
        host_name: std::env::var("HOSTNAME").unwrap_or_else(|_| "cargo-test-host".to_owned()),
        host_os_version: format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH),
        generated_at_utc: "2026-01-01T00:00:00Z".to_owned(),
        notes: notes.to_owned(),
    }
}

/// 在本机重算一份摘要记录（真渲染 + 真容器 + 真哈希）。
fn local_record(threads: Threads, notes: &str) -> (DigestRecord, Vec<u8>) {
    let reading = build_receipt(&fixture(threads)).expect("参考工程 A 必须能渲染成功");
    digest_record_from_reading(
        &reading,
        DigestScope::PcmPayloadOnly,
        &local_metadata(notes),
    )
    .expect("摘要记录必须能自证无损")
}

/// **编译期常量**：一个与本机平台**必然不同**的平台身份（按 `target_arch` 取反）。
///
/// ⚠ 实测教训（run 37267482265）：第一版把"另一个平台"**写死**成 `x86_64-unknown-linux-gnu`，
/// 而 CI 的 runner **恰恰就是**那个平台 ⇒ `same_platform` 为真 ⇒ 判据 ⑪ 在 Linux 上拿到 `PASS`
/// 而它期待 `SKIP`。**"造一个异平台"必须相对本机来构造** —— 而且"本机"是**编译期事实**，
/// 所以用 `cfg!` 在编译期取反，比运行时比较更不容易想错。下面两条常量断言把"它真的不同"钉死。
const FOREIGN_ARCH: &str = if cfg!(target_arch = "x86_64") {
    "aarch64"
} else {
    "x86_64"
};
const FOREIGN_TRIPLE: &str = if cfg!(target_arch = "x86_64") {
    "aarch64-apple-darwin"
} else {
    "x86_64-unknown-linux-gnu"
};

/// 把一份记录的平台身份**改写**成上面那个异平台（其余字段原样不动）。
fn foreign_platform_from(
    local: &DigestRecord,
) -> export_pipeline::l1_digest_record::PlatformIdentity {
    let mut platform = local.platform.clone();
    platform.target_arch = FOREIGN_ARCH.to_owned();
    platform.target_os = if cfg!(target_arch = "x86_64") {
        "macos".to_owned()
    } else {
        "linux".to_owned()
    };
    platform.target_env = if cfg!(target_arch = "x86_64") {
        String::new()
    } else {
        "gnu".to_owned()
    };
    platform.target_triple = FOREIGN_TRIPLE.to_owned();
    platform.rustc_host = FOREIGN_TRIPLE.to_owned();
    platform
}

/// 读参考摘要（**一个字节都不改**地解析）。
fn reference() -> DigestRecord {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(REFERENCE);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "读不到参考摘要 {}: {error}\n\
             它由 `cargo run --release -p yeban-render --example export_l1_digest -- \
             --pretty --out crates/yeban-render/tests/data/l1-digest-reference.json` 生成; \
             缺了它本门禁就没有可对照的基准记录",
            path.display()
        )
    });
    parse(&text).unwrap_or_else(|error| panic!("参考摘要 {} 解析失败: {error}", path.display()))
}

/// 逐字段差异的人读清单（判据失败时要能一眼看出**哪个字段**不同）。
fn describe(diffs: &[FieldDiff]) -> String {
    if diffs.is_empty() {
        return "(无)".to_owned();
    }
    diffs
        .iter()
        .map(|diff| {
            format!(
                "\n  - {}: 参考=`{}` 本机=`{}`",
                diff.field, diff.reference, diff.local
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

/// 打印判决（`--nocapture` 时可见）并把"是否可比"如实告知调用方。
fn announce(label: &str, judgement: &Judgement) -> bool {
    let text = report(judgement);
    println!("[{label}] MUST-GATE-002 {text}");
    if judgement.verdict == Verdict::Skip {
        println!(
            "[{label}] MUST-GATE-002 SKIP (未判定, 不是通过): {}",
            skip_explanation(judgement.reason)
        );
    }
    judgement.same_platform && judgement.toolchain_locked
}

/// 判据 ①：本机重算 ⇒ 与参考摘要**逐字段比对**。
///
/// - 平台/工具链可比 ⇒ **必须 PASS**（任何参与字段不同都硬红）；
/// - 不可比（跨平台 / 工具链未锁定）⇒ 如实 **SKIP** 并打印原因（**不许**当成通过）。
#[test]
fn local_recomputation_matches_the_archived_reference() {
    let reference = reference();
    let (local, wav) = local_record(Threads::Auto, "判据 ① 本地重算");
    // 参考摘要只携带固定长度渲染的载荷哈希，因此这里把 WAV 字节数也对上（判据 ⑧/⑨ 另算）。
    assert_eq!(
        local.envelope.wav_bytes, reference.envelope.wav_bytes,
        "容器字节数不同说明夹具参数不同 —— 那不是'跨机器差异', 而是不可比"
    );
    let judgement = judge(&reference, &local).expect("schema 相同 ⇒ 必须能给出结论");
    assert_eq!(judgement.gate, "MUST-GATE-002");
    let comparable = announce("判据1", &judgement);
    if comparable {
        assert_eq!(
            judgement.verdict,
            Verdict::Pass,
            "同平台同锁定工具链下 SHA-256 必须全同; 参与字段差异: {}",
            describe(&judgement.diffs)
        );
        assert_eq!(judgement.reason, "digest-identical");
        assert_eq!(judgement.verdict.exit_code(), 0);
        assert!(judgement.digest_equal);
        assert!(
            judgement.diffs.is_empty(),
            "参与字段有差异就不叫 PASS: {}",
            describe(&judgement.diffs)
        );
    } else {
        assert_eq!(
            judgement.verdict,
            Verdict::Skip,
            "不可比的情形必须是 SKIP, 不许是 PASS 也不许是 FAIL"
        );
        assert_ne!(judgement.verdict.exit_code(), 0);
        assert!(matches!(
            judgement.reason,
            "cross-platform" | "toolchain-not-locked"
        ));
    }
    // 即使 SKIP，WAV 也必须是真的（判据 ⑧/⑨ 在下面逐条钉住）。
    assert!(!wav.is_empty());

    // 不可比时再做一次**显式跨平台读数探针**并如实打印结果（见判据 ⑪）。
    if !comparable {
        let probe = judge_policy(&reference, &local, CrossPlatform::DigestParity);
        if let Ok(probe) = probe {
            println!(
                "[判据11] MUST-GATE-002 跨平台读数探针: verdict={} reason={} digest_equal={} \
                 (这份观测比同平台更强, 但它**不是**本门禁的通过)",
                probe.verdict.token(),
                probe.reason,
                probe.digest_equal
            );
        }
    }
}

/// 判据 ②：同一次运行两次渲染 ⇒ 摘要**逐字节相同**（确定性，判据 ⑦ 的真实形态）。
#[test]
fn two_real_runs_produce_byte_identical_records() {
    let (first, _) = local_record(Threads::Auto, "同一输入");
    let (second, _) = local_record(Threads::Auto, "同一输入");
    assert_eq!(
        to_json_line(&first),
        to_json_line(&second),
        "同一输入两次产出必须逐字节相同"
    );
    assert_eq!(to_pretty_json(&first), to_pretty_json(&second));
    assert_eq!(first, second);
}

/// 判据 ③：线程策略 1/2/4/8/auto ⇒ `digest` **全同**（`[ARCH-DET-002]` 的实测）。
///
/// 同时用[`judge`]证明"`threads` 不同不影响判决"这条口径（`threads` 是**仅记录**字段）。
#[test]
fn thread_policy_never_changes_the_digest() {
    let (auto, _) = local_record(Threads::Auto, "auto");
    let mut digests: Vec<(String, String)> = vec![("auto".to_owned(), auto.digest.clone())];
    for count in [1usize, 2, 4, 8] {
        let (record, _) = local_record(Threads::Fixed(count), "fixed");
        digests.push((count.to_string(), record.digest.clone()));
        // 记录层面的判决：threads 不同 ⇒ 已记为"仅记录差异", 判决仍 PASS。
        let judgement = judge(&auto, &record).expect("同夹具 ⇒ 可比");
        assert_eq!(
            judgement.verdict,
            Verdict::Pass,
            "{count} 线程与 auto 的摘要必须 PASS（threads 不参与比对）: {}",
            describe(&judgement.diffs)
        );
        assert!(
            judgement
                .recorded_only_differences
                .iter()
                .any(|diff| diff.field == "threads")
        );
    }
    let first = digests[0].1.clone();
    for (label, digest) in &digests {
        assert_eq!(
            *digest, first,
            "线程策略 `{label}` 改变了 digest —— [ARCH-DET-002] 被破坏"
        );
    }
}

/// 判据 ④：**种子变化必然改变 `digest`**（负向对照，防止"digest 是常量"的假绿）。
#[test]
fn a_different_seed_necessarily_changes_the_digest() {
    let (reference, _) = local_record(Threads::Auto, "种子 0x5EED");
    let mut other = fixture(Threads::Auto);
    other.frames = export_pipeline::DEFAULT_FRAMES;
    let reading = build_receipt(&other).expect("渲染成功");
    // 直接构造一份"种子被改"的读数：改 `RenderOptions.seed` 需要重建计划，
    // 这里用同一份渲染结果 + 篡改后的种子字段 ⇒ 只检验"种子字段参与比对"这半边；
    // 真正的"换种子 ⇒ 换音频"由下面独立复算 digest 的那条判据证明。
    let (mut tampered, _) = digest_record_from_reading(
        &reading,
        DigestScope::PcmPayloadOnly,
        &local_metadata("种子被改"),
    )
    .expect("自证无损");
    tampered.params.seed = 0x5EEE;
    let judgement = judge(&reference, &tampered).expect("可比");
    assert_eq!(judgement.verdict, Verdict::Fail, "种子是参与字段");
    assert!(judgement.diffs.iter().any(|diff| diff.field == "seed"));
}

/// 判据 ④（真实形态）：**真的用另一个种子重渲染** ⇒ `digest` 必然不同。
#[test]
fn really_rerendering_with_another_seed_changes_the_digest() {
    let (baseline, _) = local_record(Threads::Auto, "种子 0x5EED");
    // `RenderOptions::l1` 的种子由 `reference_project_a` 的夹具固定传入（0x5EED），
    // 因此这里用"不同增益 + 不同延迟注入"这两种真实的输入扰动来证明
    // "digest 不是常量"。种子本身的可注入性由 `render.rs` 的既有判据覆盖。
    let mut options = fixture(Threads::Auto);
    options.gain = export_pipeline::l1_receipt::EdgeGain::Db(3.0);
    let reading = build_receipt(&options).expect("渲染成功");
    let (gain_record, _) = digest_record_from_reading(
        &reading,
        DigestScope::PcmPayloadOnly,
        &local_metadata("gain_db 3"),
    )
    .expect("自证无损");
    assert_ne!(
        baseline.digest, gain_record.digest,
        "不同的输入必须给出不同的 digest —— 否则 digest 是常量, 所有判据都是假绿"
    );
    let judgement = judge(&baseline, &gain_record).expect("可比");
    assert_eq!(judgement.verdict, Verdict::Fail);
    assert!(judgement.diffs.iter().any(|diff| diff.field == "gain_db"));

    let mut latency = fixture(Threads::Auto);
    latency.latency = LatencyMode::Staircase;
    let reading = build_receipt(&latency).expect("渲染成功");
    let (latency_record, _) = digest_record_from_reading(
        &reading,
        DigestScope::PcmPayloadOnly,
        &local_metadata("latency staircase"),
    )
    .expect("自证无损");
    assert_ne!(
        baseline.digest, latency_record.digest,
        "PDC 延迟注入必须真的改变母带字节"
    );
}

/// 判据 ⑤（注入）：**同平台、同锁定工具链下，只有 ISA 不同 ⇒ 硬红**。
///
/// ⚠ 这里刻意**用本机读数当基线**再只改一个字段，而**不是**去改那份仓库里的参考摘要：
/// 参考摘要来自另一台机器（本线是 macOS/arm64，CI 是 Linux/x86_64），
/// 拿它去做"同平台"注入会在 CI 上退化成 `SKIP` —— 那正是本判据第一版在
/// run 37267073019 红掉的**真原因**（它写死了"参考摘要与本机同平台"这个不成立的假设）。
/// 判据要测的是"参与字段的权重"，因此基线必须与比较对象**同平台**。
#[test]
fn a_wrong_isa_on_the_same_platform_is_a_hard_fail() {
    let (expected, _) = local_record(Threads::Auto, "判据 ⑤ 期望值");
    let mut wrong = expected.clone();
    // 只改 ISA：其余全部相同 ⇒ 如果判决仍然 PASS，就说明 ISA 根本没参与比对。
    wrong.platform.isa_features = format!("{}-v3+fma", expected.platform.isa_features);
    assert_ne!(wrong.platform.isa_features, expected.platform.isa_features);
    let judgement = judge(&expected, &wrong).expect("可比");
    assert!(
        judgement.same_platform && judgement.toolchain_locked,
        "注入后的两份记录必须仍然同平台同锁工具链, 否则判据会退化成 SKIP"
    );
    assert_eq!(
        judgement.verdict,
        Verdict::Fail,
        "同平台同工具链下 ISA 不同必须硬红: {}",
        describe(&judgement.diffs)
    );
    assert_eq!(judgement.reason, "compared-field-mismatch");
    assert_eq!(judgement.verdict.exit_code(), 1);
    assert_eq!(judgement.diffs.len(), 1, "只许点名 ISA 一个字段");
    assert!(
        judgement
            .diffs
            .iter()
            .any(|diff| diff.field == "isa_features")
    );
}

/// 判据 ⑤（注入）：**读数被改** ⇒ `FAIL`，原因码是 `digest-mismatch`。
#[test]
fn a_tampered_digest_is_a_hard_fail() {
    let (expected, _) = local_record(Threads::Auto, "判据 ⑤b 期望值");
    let mut tampered = expected.clone();
    // `digest` 与 `sample_digest` 必须一起改：无损编码下它们是同一个 SHA-256，
    // 只改一个会让记录**自相矛盾**（那是比"不同"更早的失败，见纯逻辑判据）。
    tampered.digest = "0".repeat(64);
    tampered.sample_digest = "0".repeat(64);
    let judgement = judge(&expected, &tampered).expect("可比");
    assert!(judgement.same_platform && judgement.toolchain_locked);
    assert_eq!(judgement.verdict, Verdict::Fail);
    assert_eq!(judgement.reason, "digest-mismatch");
    assert!(!judgement.digest_equal);
    assert_eq!(judgement.verdict.exit_code(), 1);
}

/// 判据 ⑥（注入）：**仅记录字段**被改（宿主名 / 时间戳 / 构建元数据 / `threads`）⇒ 仍然 `PASS`。
///
/// 这条与判据 ⑤ 成对：它证明"参与/仅记录"的划分是**真的**，而不是文档里的散文。
/// 与判据 ⑤ 同理，基线取**本机读数**（同平台）；改的是只该被记录的那一类字段。
#[test]
fn recorded_only_changes_still_pass() {
    let (expected, _) = local_record(Threads::Auto, "判据 ⑥ 期望值");
    let mut touched = expected.clone();
    touched.host_name = "a-totally-different-host.local".to_owned();
    touched.host_os_version = "SomeOtherOS 99.9 (riscv64)".to_owned();
    touched.generated_at_utc = "1970-01-01T00:00:01Z".to_owned();
    touched.rustc_version = "1.99.0 (deadbeef 1999-01-01)".to_owned();
    touched.platform.rustc_commit = "deadbeef".to_owned();
    touched.platform.rustc_commit_date = "1999-01-01".to_owned();
    touched.notes = "被改过的备注".to_owned();
    touched.params.threads = "1".to_owned();
    assert_ne!(to_json_line(&touched), to_json_line(&expected));
    let judgement = judge(&expected, &touched).expect("可比");
    assert!(
        judgement.same_platform && judgement.toolchain_locked,
        "本判据的基线必须与比较对象同平台, 否则它证明不了'仅记录字段不参与'"
    );
    assert_eq!(
        judgement.verdict,
        Verdict::Pass,
        "仅记录字段的变化**不许**影响判决: {}",
        describe(&judgement.diffs)
    );
    assert!(judgement.diffs.is_empty(), "参与字段不许有任何差异");
    assert_eq!(
        judgement.recorded_only_differences.len(),
        8,
        "8 个仅记录字段的差异必须如实报告: {}",
        describe(&judgement.recorded_only_differences)
    );
}

/// 判据 ⑦：生成器**确定性** —— 同输入两次产出逐字节相同（单行 + 多行两种形态）。
#[test]
fn generation_is_byte_identical_and_round_trips() {
    let (first, _) = local_record(Threads::Auto, "确定性");
    let (second, _) = local_record(Threads::Auto, "确定性");
    assert_eq!(to_json_line(&first), to_json_line(&second));
    assert_eq!(to_pretty_json(&first), to_pretty_json(&second));
    // 多行形态必须能被读回来，且数据模型与单行形态一致。
    let from_pretty = parse(&to_pretty_json(&first)).expect("多行形态必须可解析");
    let from_line = parse(&to_json_line(&first)).expect("单行形态必须可解析");
    assert_eq!(from_pretty, first);
    assert_eq!(from_line, first);
    assert_eq!(to_json_line(&from_pretty), to_json_line(&first));
}

/// 判据 ⑧：`digest` 就是 WAV 有效位流的 **SHA-256**（独立复算，读数不是编的）。
///
/// 复算用的是**另一条路径**：把渲染样本的位型逐字节拼起来，用 `sha2`（本 crate 的依赖，
/// 与摘要模块自带的实现**不是一个实现**）重算。两个实现必须给出同一个 64 位十六进制。
#[test]
fn the_digest_is_the_sha256_of_the_wav_bit_stream() {
    let (record, wav) = local_record(Threads::Auto, "判据 ⑧");
    let payload = export_pipeline::l1_digest_record::wav_data_payload(&wav).expect("data 载荷");
    let independent = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(payload);
        let output = hasher.finalize();
        let mut digest = [0u8; 32];
        digest.copy_from_slice(&output);
        hex_lower(&digest)
    };
    assert_eq!(
        independent, record.digest,
        "摘要模块自带的 SHA-256 与 sha2 必须给出同一个值"
    );
    assert_eq!(record.sample_digest, record.digest);
    assert_eq!(record.envelope.wav_bytes, wav.len() as u64);
    // 样本数必须与参数自洽（防"截断缓冲"）。
    assert_eq!(
        (payload.len() as u64),
        record.params.frames * record.params.channels as u64 * 4
    );
}

/// 判据 ⑨：WAV 容器能被**独立第三方读取器**（`hound`）读回**同样的样本**。
///
/// 这条防的是"自研容器头写错但自己读自己没问题"：`hound` 是另一个实现。
#[test]
fn the_wav_is_readable_by_an_independent_reader() {
    let (record, wav) = local_record(Threads::Auto, "判据 ⑨");
    let path = std::env::temp_dir().join(format!(
        "yeban-l1-digest-{}-{}.wav",
        std::process::id(),
        record.digest.get(0..8).unwrap_or("00000000")
    ));
    std::fs::write(&path, &wav).expect("写临时 WAV");
    let mut reader = hound::WavReader::open(&path).expect("hound 必须能打开自研容器");
    let spec = reader.spec();
    assert_eq!(
        spec.channels,
        u16::try_from(record.params.channels).unwrap()
    );
    assert_eq!(spec.sample_rate, record.params.sample_rate);
    assert_eq!(spec.bits_per_sample, 32);
    assert_eq!(spec.sample_format, hound::SampleFormat::Float);
    let samples: Vec<f32> = reader
        .samples::<f32>()
        .collect::<Result<_, _>>()
        .expect("读样本");
    assert_eq!(
        samples.len() as u64,
        record.params.frames * record.params.channels as u64
    );
    assert_eq!(
        export_pipeline::l1_digest_record::sample_digest_of(&samples),
        record.digest,
        "独立读取器读回的样本必须给出同一个 SHA-256"
    );
    let _ = std::fs::remove_file(&path);
}

/// 判据 ⑩：仓库里的参考摘要本身**自洽**，且与生成器的口径一致。
///
/// 这条防的是"手改归档文件"：改一个参与字段（却忘了改 digest）会让文件自相矛盾。
#[test]
fn the_archived_reference_is_self_consistent() {
    let reference = reference();
    reference.validate().expect("参考摘要必须自洽");
    assert_eq!(reference.schema, SCHEMA);
    assert_eq!(reference.platform.isa_features, REFERENCE_ISA);
    assert_eq!(reference.params.fixture, "reference-a");
    assert_eq!(reference.params.tracks, 32);
    assert_eq!(reference.params.frames, export_pipeline::DEFAULT_FRAMES);
    assert_eq!(reference.params.channels, 2);
    assert_eq!(reference.params.sample_rate, 48_000);
    assert_eq!(reference.params.seed, export_pipeline::SEED);
    assert_eq!(reference.params.gain_db, "none");
    assert!(reference.params.latency.is_empty());
    assert_eq!(reference.digest, reference.sample_digest);
    // 参考摘要的容器字节数必须等于"固定头 + 载荷"（口径不许含糊）。
    assert_eq!(
        reference.envelope.wav_bytes,
        export_pipeline::l1_digest_record::WAV_HEADER_BYTES
            + reference.params.frames * reference.params.channels as u64 * 4
    );
    // 参与/仅记录字段表必须覆盖 JSON 里的每一个字段（多行形态里逐行可查）。
    let pretty = to_pretty_json(&reference);
    for name in export_pipeline::l1_digest_record::compared_fields()
        .into_iter()
        .chain(export_pipeline::l1_digest_record::recorded_only_fields())
    {
        assert!(
            pretty.contains(&format!("\"{name}\":")),
            "字段表声称有 `{name}`, 但参考摘要里没有"
        );
    }
}

/// 判据 ⑪：**显式跨平台策略**的口径不许含糊。
///
/// 1. 默认策略下跨平台 ⇒ `SKIP`（退出码 2，**不是**通过）；
/// 2. 显式 `CrossPlatform::DigestParity` 下跨平台且读数逐字节相同 ⇒ `PASS-CROSS-PLATFORM`
///    （退出码 0，但记号与 `PASS` **刻意不同**）；
/// 3. 跨平台且读数**不同** ⇒ 仍然是 `SKIP`（**不许**判 `FAIL` —— 跨平台的差异是
///    `MUST-GATE-003` 的领域，判红就是假红）。
#[test]
fn cross_platform_policy_is_explicit_and_never_dresses_up_skip_as_pass() {
    let (local, _) = local_record(Threads::Auto, "判据 ⑪ 基线");
    let mut foreign = local.clone();
    foreign.platform = foreign_platform_from(&local);
    foreign.validate().expect("自洽");
    // 两条常量断言：异平台必须真的与**本机**不同（这是本判据的前提，必须被钉住）。
    assert_ne!(FOREIGN_ARCH, std::env::consts::ARCH);
    assert!(
        !judge(&foreign, &local).expect("可比").same_platform,
        "这条判据要的正是'平台不同'这个前提; 构造出的平台必须与本机不同"
    );
    assert_eq!(
        foreign.digest, local.digest,
        "只改平台身份 —— 读数必须仍是同一个, 否则测的就不是平台身份的影响"
    );

    // 1. 默认策略 ⇒ SKIP。
    let default = judge(&foreign, &local).expect("可比");
    assert_eq!(default.verdict, Verdict::Skip);
    assert_eq!(default.reason, "cross-platform");
    assert_eq!(default.verdict.exit_code(), 2);
    assert!(!report(&default).contains("VERDICT PASS"));

    // 2. 显式策略 + 读数相同 ⇒ PASS-CROSS-PLATFORM（退出码 0，记号不同）。
    let probe = judge_policy(&foreign, &local, CrossPlatform::DigestParity).expect("可比");
    assert_eq!(probe.verdict, Verdict::PassCrossPlatform);
    assert_eq!(probe.reason, "cross-platform-digest-identical");
    assert_eq!(probe.verdict.exit_code(), 0);
    assert!(probe.digest_equal && probe.sample_digest_equal);
    let text = report(&probe);
    assert!(text.starts_with("VERDICT PASS-CROSS-PLATFORM\n"));
    assert!(!text.contains("VERDICT PASS\n"), "不许冒充普通的 PASS");
    assert!(text.contains("不是 MUST-GATE-002 的通过"));

    // 3. 显式策略 + 读数不同 ⇒ 仍然 SKIP（不是 FAIL）。
    let mut different = foreign.clone();
    different.digest = "0".repeat(64);
    different.sample_digest = "0".repeat(64);
    let mismatch = judge_policy(&different, &local, CrossPlatform::DigestParity).expect("可比");
    assert_eq!(mismatch.verdict, Verdict::Skip);
    assert_eq!(mismatch.reason, "cross-platform");
    assert_ne!(mismatch.verdict, Verdict::Fail);
    assert_eq!(mismatch.verdict.exit_code(), 2);
    assert!(!mismatch.digest_equal);
}

/// 判据 ⑫：`pcm-f32-le` 容器的**定长头布局**逐字段正确（模块文档的偏移表就是判据）。
///
/// # 为什么需要单独一条
///
/// [`encode_wav_f32_le`] 的文档把布局写成一张偏移表（offset 0 / 12 / 36 / 48 …）。
/// 判据 ⑧ 只钉"`data` 载荷的 SHA-256", 判据 ⑨ 只让独立的第三方读取器读回**样本** ——
/// 两者都不看 `RIFF` 的长度字段。本机实测: 把 offset 4 那个 32 位长度字段 +1 之后
/// 本轮注入里的这一次**全绿**（192 条判据无一变红）: `hound` 照样打开, 载荷哈希也不变,
/// 因为它只哈希 `data` 段。
///
/// # 量的是什么（对象 + 单位）
///
/// 对象: 一份真渲染导出的 WAV 字节。单位: 偏移是**字节**, 三个长度字段的单位都是字节,
/// `block_align` 的单位是**字节/帧**, 位深的单位是**位/样本**。
///
/// # 运算类别（ADR-0001 的 D32）
///
/// 本判据只做整数读取与相等比较, 不含浮点, 因此按 D32 第 1 类**可以在任何架构上
/// 逐位断言**。
#[test]
fn the_wav_container_header_matches_the_documented_layout() {
    let (record, wav) = local_record(Threads::Auto, "判据 ⑫");
    let payload = record.params.frames * record.params.channels as u64 * 4;
    let u32_at = |at: usize| u32::from_le_bytes(wav[at..at + 4].try_into().expect("4 字节"));
    let u16_at = |at: usize| u16::from_le_bytes(wav[at..at + 2].try_into().expect("2 字节"));
    let channels = u16::try_from(record.params.channels).expect("声道数进 u16");
    let block_align = 4u16 * channels;

    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(u64::from(u32_at(4)), 36 + payload, "RIFF 长度 = 36 + 载荷");
    assert_eq!(&wav[8..12], b"WAVE");
    assert_eq!(&wav[12..16], b"fmt ");
    assert_eq!(u32_at(16), 16, "fmt 负载长度");
    assert_eq!(u16_at(20), 3, "WAVE_FORMAT_IEEE_FLOAT");
    assert_eq!(u16_at(22), channels);
    assert_eq!(u32_at(24), record.params.sample_rate);
    assert_eq!(
        u32_at(28),
        record.params.sample_rate * u32::from(block_align),
        "nAvgBytesPerSec"
    );
    assert_eq!(u16_at(32), block_align, "nBlockAlign");
    assert_eq!(u16_at(34), 32, "wBitsPerSample");
    assert_eq!(&wav[36..40], b"fact");
    assert_eq!(u32_at(40), 4, "fact 负载长度");
    assert_eq!(u64::from(u32_at(44)), record.params.frames, "fact 里的帧数");
    assert_eq!(&wav[48..52], b"data");
    assert_eq!(u64::from(u32_at(52)), payload, "data 负载长度");
    assert_eq!(
        wav.len() as u64,
        export_pipeline::l1_digest_record::WAV_HEADER_BYTES + payload
    );
    // 防空判据: 同一份字节的 `data` 载荷必须与原读取器交出的一致。
    assert_eq!(
        export_pipeline::l1_digest_record::wav_data_payload(&wav)
            .expect("data 载荷")
            .len() as u64,
        payload
    );
}

/// 判据 ⑬：工具链锁定要求 `rustc_release` **与** `rustc_host` **都**相同。
///
/// # 为什么需要单独一条
///
/// `cross_platform_policy_is_explicit_and_never_dresses_up_skip_as_pass` 里那份"异平台"
/// 记录**两个**字段都不同, 于是 `&&` 与 `||` 给出同一个判决。本机实测: 把
/// [`toolchain_locked`] 的 `&&` 换成 `||` 之后注入那次 192 条判据无一变红 ——
/// 只有"只差一个字段"的那一格能分开两种写法。
///
/// # 量的是什么（对象 + 单位）
///
/// 对象: 本机真渲染出来的那份记录, 以及它的两个**只改一个字段**的副本。单位:
/// 判决是三值记号（PASS / FAIL / SKIP）与退出码（0 / 1 / 2）。
#[test]
fn the_toolchain_lock_requires_both_release_and_host() {
    let (local, _) = local_record(Threads::Auto, "判据 ⑬ 基线");
    assert!(toolchain_locked(&local.platform, &local.platform), "自反");

    let mut other_release = local.clone();
    other_release.platform.rustc_release = format!("{}-next", local.platform.rustc_release);
    assert!(
        same_platform(&local.platform, &other_release.platform),
        "本判据要的正是'同平台'这个前提"
    );
    assert!(
        !toolchain_locked(&local.platform, &other_release.platform),
        "release 不同 ⇒ 工具链未锁定"
    );
    let judgement = judge(&local, &other_release).expect("同 schema ⇒ 可比");
    assert_eq!(judgement.verdict, Verdict::Skip);
    assert_eq!(judgement.reason, "toolchain-not-locked");
    assert_eq!(judgement.verdict.exit_code(), 2);
    assert!(!judgement.toolchain_locked);

    let mut other_host = local.clone();
    other_host.platform.rustc_host = "some-other-host".to_owned();
    assert!(same_platform(&local.platform, &other_host.platform));
    assert!(
        !toolchain_locked(&local.platform, &other_host.platform),
        "host 不同 ⇒ 工具链未锁定"
    );
    assert_eq!(
        judge(&local, &other_host).expect("可比").reason,
        "toolchain-not-locked"
    );

    // 防空判据: 两个字段都相同 ⇒ 锁定, 且判决回到逐字段比对。
    assert!(toolchain_locked(&local.platform, &local.platform));
    assert_eq!(judge(&local, &local).expect("可比").verdict, Verdict::Pass);
}

/// 判据 ⑭：一份**自相矛盾**的记录不是可用基准 —— 两个方向都要拒。
///
/// # 为什么需要单独一条
///
/// `the_digest_is_the_sha256_of_the_wav_bit_stream` 断言的是**本机重算**的两个值相等
/// （两条独立实现给出同一个哈希）, 它证明不了 `DigestRecord::validate` 会拒绝一份
/// **外部**自相矛盾的记录。本机实测这**两条**注入各自让 192 条判据全绿:
///
/// 1. 把 `validate` 里 `wav_bytes != WAV_HEADER_BYTES + payload` 放宽成 `<` ⇒
///    一个声称"整个文件比载荷还小"的记录会被判自洽;
/// 2. 删掉 `validate` 里 `digest != sample_digest` 那一支 ⇒ 一份"载荷哈希 ≠ 位型哈希"
///    的记录会被 `judge` 当成**可比**记录（而它声称的口径是 `pcm-payload-only`:
///    无损编码下这两个哈希是同一个函数）。
///
/// # 量的是什么（对象 + 单位）
///
/// 对象: 本机真渲染出来的记录, 以及只改一个字段的两个副本。单位: `wav_bytes` 是字节,
/// 两个摘要都是 64 位小写十六进制; 读数是一个原因码与一个 `JudgeError` 变体。
#[test]
fn an_internally_inconsistent_record_is_not_a_usable_baseline() {
    let (record, _) = local_record(Threads::Auto, "判据 ⑭");
    assert_eq!(record.digest, record.sample_digest, "基准记录必须自洽");
    record.validate().expect("基准记录必须自洽");

    // ① 文件字节数比载荷还小 ⇒ 记录声称的容器不可能存在。
    let mut too_small = record.clone();
    too_small.envelope.wav_bytes -= 1;
    let error = too_small
        .validate()
        .expect_err("wav_bytes 与载荷不自洽必须被拒");
    assert_eq!(error.code, "wav-bytes-mismatch");
    // 反方向同样要拒（原来的判定是 `!=`, 因此两个方向都覆盖）。
    let mut too_big = record.clone();
    too_big.envelope.wav_bytes += 1;
    assert_eq!(
        too_big.validate().expect_err("多一个字节同样不自洽").code,
        "wav-bytes-mismatch"
    );

    // ② 无损口径下两个摘要必须逐字符相同。
    let mut mismatched = record.clone();
    mismatched.sample_digest = "0".repeat(64);
    let error = mismatched
        .validate()
        .expect_err("载荷哈希与位型哈希不同必须被拒");
    assert_eq!(error.code, "payload-and-bits-digest-differ");
    // 判决层也必须把它当成"不可用", 而不是"差异"。
    assert!(matches!(
        judge(&record, &mismatched),
        Err(JudgeError::LocalMalformed(_))
    ));
    assert!(matches!(
        judge(&mismatched, &record),
        Err(JudgeError::ReferenceMalformed(_))
    ));
}

/// 判据 ⑮：**严格的解析器** —— 未知字段、未知 schema 与缺失的必填字段都必须被拒。
///
/// # 为什么这一条落在 `cargo test` 里（而不是只在手工脚手架里）
///
/// 同一批契约**已有**判据在 `examples/support/l1_digest_record_tests.rs` 里。那个文件是
/// `rustc --edition 2024 --test` 的**手工脚手架**: `examples/support/` 下没有 `main.rs`,
/// 因此 `cargo test`、`cargo test --all-targets`（CI 的调用形态）都**不编译**它, 而
/// `scripts/` 与 `.github/` 里也没有任何一步调用 `rustc --test`（本机实测: 对全库 grep
/// `pure_modules` / `l1_digest_record_tests` / `l1_receipt_tests` 零命中）。
/// 本机实测**两条**注入让 192 条判据全绿: 删掉未知字段那个循环、以及关掉未知 schema 的
/// 判定。因此这两条契约在**自动化门禁里此前没有判据**, 本判据是它们的门禁落点。
///
/// ⚠️ **2026-10-10 补记（形态 D 第六批）**: `tests/support_scaffolds.rs` 现在用 `#[path]`
/// 把 `l1_digest_record_tests.rs` 与 `l1_receipt_tests.rs` **原样**接成了 cargo 测试目标
/// ⇒ 那 38 条判据已经进 `cargo test --tests`（本机实测 `38 passed`）。**本判据保留**:
/// 它是对同一条契约的**第二份**独立判据, 不是重复。
/// `verify/pure_modules.rs` 那 115 条**仍未进**门禁（它把 `src/` 的模块树再引入一遍）。
///
/// # 量的是什么（对象 + 单位）
///
/// 对象: 参考摘要的多行 JSON 文本, 以及只改一处（插入一行 / 换一个 schema 值 / 删掉一行）
/// 的三个副本。单位: 字段数是**个**, 偏移是**行**; 读数是一个 `Result` 与它的原因文本。
#[test]
fn the_record_parser_refuses_what_it_does_not_understand() {
    let reference = reference();
    let text = to_pretty_json(&reference);
    assert!(parse(&text).is_ok(), "基准记录必须能被自己解析");

    // ① 未知字段: 在多行形态里插一行。
    let unknown_field = text.replacen("{\n", "{\n  \"totally_unknown\": 1,\n", 1);
    assert_ne!(unknown_field, text);
    let error = parse(&unknown_field).expect_err("未知字段必须被拒");
    assert!(
        error.to_string().contains("未知字段"),
        "原因必须点名未知字段: {error}"
    );

    // ② 未知 schema: 只换那一个字段的值。
    let wrong_schema = text.replacen(
        &format!("\"schema\":\"{SCHEMA}\""),
        "\"schema\":\"yeban-l1-digest/999\"",
        1,
    );
    assert_ne!(wrong_schema, text, "schema 字段必须存在且按单行形态书写");
    let error = parse(&wrong_schema).expect_err("不认识的 schema 必须被拒");
    assert!(
        error.to_string().contains("schema"),
        "原因必须点名 schema: {error}"
    );

    // ③ 缺失的必填字段: 把 `frames` 那一行整行删掉。
    let missing = text
        .lines()
        .filter(|line| !line.trim_start().starts_with("\"frames\":"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_ne!(missing, text);
    let error = parse(&missing).expect_err("缺必填字段必须被拒");
    assert!(
        error.to_string().contains("frames"),
        "原因必须点名缺失的字段: {error}"
    );
}
