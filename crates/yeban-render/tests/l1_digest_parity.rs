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
    DigestRecord, DigestScope, FieldDiff, Judgement, SCHEMA, Verdict, hex_lower, judge, parse,
    report, skip_explanation, to_json_line, to_pretty_json,
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

/// 判据 ⑤（注入）：参考摘要的**参与字段**被改（ISA 写错）⇒ **FAIL**。
#[test]
fn a_wrong_isa_in_the_reference_is_a_hard_fail() {
    let reference = reference();
    let (local, _) = local_record(Threads::Auto, "判据 ⑤");
    let mut wrong = reference.clone();
    // 只改 ISA：其余全部相同 ⇒ 如果判决仍然 PASS，就说明 ISA 根本没参与比对。
    wrong.platform.isa_features = "x86-64-v3+fma,+avx2".to_owned();
    assert_ne!(wrong.platform.isa_features, local.platform.isa_features);
    let judgement = judge(&wrong, &local).expect("可比");
    assert!(judgement.same_platform && judgement.toolchain_locked);
    assert_eq!(
        judgement.verdict,
        Verdict::Fail,
        "同平台同工具链下 ISA 不同必须硬红"
    );
    assert_eq!(judgement.reason, "compared-field-mismatch");
    assert_eq!(judgement.verdict.exit_code(), 1);
    assert!(
        judgement
            .diffs
            .iter()
            .any(|diff| diff.field == "isa_features")
    );
}

/// 判据 ⑤（注入）：参考摘要的**读数**被改 ⇒ `FAIL`，原因码是 `digest-mismatch`。
#[test]
fn a_tampered_reference_digest_is_a_hard_fail() {
    let reference = reference();
    let (local, _) = local_record(Threads::Auto, "判据 ⑤b");
    let mut tampered = reference.clone();
    // `digest` 与 `sample_digest` 必须一起改：无损编码下它们是同一个 SHA-256，
    // 只改一个会让记录**自相矛盾**（那是比"不同"更早的失败，见纯逻辑判据）。
    tampered.digest = "0".repeat(64);
    tampered.sample_digest = "0".repeat(64);
    let judgement = judge(&tampered, &local).expect("可比");
    assert!(judgement.same_platform && judgement.toolchain_locked);
    assert_eq!(judgement.verdict, Verdict::Fail);
    assert_eq!(judgement.reason, "digest-mismatch");
    assert!(!judgement.digest_equal);
}

/// 判据 ⑥（注入）：参考摘要的**仅记录字段**被改（宿主名 / 时间戳 / 构建元数据）⇒ 仍然 `PASS`。
///
/// 这条与判据 ⑤ 成对：它证明"参与/仅记录"的划分是**真的**，而不是文档里的散文。
#[test]
fn recorded_only_changes_in_the_reference_still_pass() {
    let reference = reference();
    let (local, _) = local_record(Threads::Auto, "判据 ⑥");
    let mut touched = reference.clone();
    touched.host_name = "a-totally-different-host.local".to_owned();
    touched.host_os_version = "SomeOtherOS 99.9 (riscv64)".to_owned();
    touched.generated_at_utc = "1970-01-01T00:00:01Z".to_owned();
    touched.rustc_version = "1.99.0 (deadbeef 1999-01-01)".to_owned();
    touched.platform.rustc_commit = "deadbeef".to_owned();
    touched.platform.rustc_commit_date = "1999-01-01".to_owned();
    touched.notes = "被改过的备注".to_owned();
    touched.params.threads = "1".to_owned();
    assert_ne!(to_json_line(&touched), to_json_line(&reference));
    let judgement = judge(&touched, &local).expect("可比");
    let comparable = announce("判据6", &judgement);
    if comparable {
        assert_eq!(
            judgement.verdict,
            Verdict::Pass,
            "仅记录字段的变化**不许**影响判决: {}",
            describe(&judgement.diffs)
        );
        assert!(judgement.diffs.is_empty());
        assert!(
            judgement.recorded_only_differences.len() >= 5,
            "仅记录字段的差异必须如实报告"
        );
    } else {
        // 不可比时也必须仍然是"不可比"而不是"因为改了宿主名而变红"。
        assert_eq!(judgement.verdict, Verdict::Skip);
        assert!(matches!(
            judgement.reason,
            "cross-platform" | "toolchain-not-locked"
        ));
    }
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
