//! # 收据导出的**共用管线**（`export_l1_receipt` 与 `tests/l1_digest_contract` 同源）
//!
//! 为什么单独一个文件：判据要求"渲染 → 收据 → 判决"这条链上的**每一个**环节都被真的
//! 验证过。如果 example 与 CI 判据各写一份装配代码，那么"判据绿"与"导出器绿"就是两件事 ——
//! 本仓库已经吃过"两套实现漂移"的教训（见 `docs/ledger/render-master-notes.md` 的
//! PDC needs 与 `docs/adr/ADR-0001` D32 的 1-ULP 发现）。
//!
//! 本文件把 `l1_receipt.rs`（纯格式/判决）与 `reference_project_a.rs`（参考工程 A 夹具）
//! 作为**子模块**引入，于是它自成一个闭环：拿到夹具参数 → 真的渲染 → 真的装配收据。
//! 上面那条 `#[path]` 嵌套解析已实测（`#[path]` 相对于声明它的文件所在目录解析）。
//!
//! ## 渲染路径（与基准同源）
//!
//! `RenderPlan::compile_with_latencies` / `execute` / `RenderOutput::digest` —— 只有这一条。
//!
//! ## 收据里没有时间戳
//!
//! 判据 ① 要求"同一台机器两次导出 ⇒ 收据逐字节相同"，因此这里没有任何
//! 时间/耗时/主机名/进程信息；唯一的机器相关信息是 `rustc -vV` 的 host 与 `cfg!` 的
//! 编译期事实（同一台机器上稳定）。

#[allow(dead_code)]
#[path = "l1_receipt.rs"]
pub mod l1_receipt;
#[allow(dead_code)]
#[path = "reference_project_a.rs"]
pub mod reference_project_a;
// L1 摘要记录（`MUST-GATE-002` 的载体）。零第三方依赖 ⇒ 也可被 `rustc --test` 单独跑。
#[allow(dead_code)]
#[path = "l1_digest_record.rs"]
pub mod l1_digest_record;

use std::collections::BTreeMap;
use std::process::Command;

use yeban_model::EntityId;
use yeban_render::render::{RenderOptions, RenderPlan};

use self::l1_receipt::{
    EdgeGain, Fingerprint, PathOp, Receipt, SampleClass, Threads, Toolchain, probes_all, stats_of,
};
use self::reference_project_a::{reference_project, tone_sources};

/// 收据默认渲染长度（帧）：约 171 ms @48 kHz 立体声。
///
/// 见 `l1_receipt.rs` 的"为什么不做抽样"：收据携带**全部**样本位型，
/// 30 秒立体声的文本约 37 MB，因此默认取一个既能上 CI artifact、又足够让 32 轨
/// 路径跑出多样运算组合的长度。两台机器必须用同一组参数。
pub const DEFAULT_FRAMES: u64 = 8192;
/// 收据夹具的采样率。
pub const SAMPLE_RATE: u32 = 48_000;
/// 收据夹具的声道数。
pub const CHANNELS: usize = 2;
/// `[ARCH-DET-001]` 的固定种子。
pub const SEED: u64 = 0x5EED;
/// PDC 演示用的每轨延迟步长（帧）。
pub const LATENCY_STEP: u32 = 64;
/// PDC 演示用的延迟阶梯周期。
pub const LATENCY_PERIOD: u32 = 4;

/// PDC 演示开关。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LatencyMode {
    /// 不注入任何延迟（`L_max = 0`，参考工程 A 的字面形态）。
    None,
    /// 第 `i` 条轨道注入 `(i % LATENCY_PERIOD) * LATENCY_STEP` 帧 ⇒ `L_max = 192` 帧。
    Staircase,
}

impl LatencyMode {
    /// 收据/日志里的文本形式。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Staircase => "staircase",
        }
    }
}

/// 夹具参数（= 收据指纹里除工具链之外的全部内容）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixtureOptions {
    /// 轨道数。
    pub tracks: u32,
    /// 总帧数。
    pub frames: u64,
    /// 请求的线程数。
    pub threads: Threads,
    /// 边增益设置。
    pub gain: EdgeGain,
    /// 延迟注入模式。
    pub latency: LatencyMode,
}

impl Default for FixtureOptions {
    fn default() -> Self {
        Self {
            tracks: 32,
            frames: DEFAULT_FRAMES,
            threads: Threads::Auto,
            gain: EdgeGain::Identity,
            latency: LatencyMode::None,
        }
    }
}

/// 一次渲染的完整读数（收据 + 原始样本，后者供判据独立复算统计量）。
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    /// 装配好并通过自洽性检查的收据。
    pub receipt: Receipt,
    /// 交错母带样本（与收据里的位型一一对应）。
    pub samples: Vec<f32>,
}

/// 跑一次参考工程 A → 产出一份收据。
///
/// # Errors
///
/// 渲染计划编译失败 / 渲染失败 / 收据自洽性检查失败（含 `longest_path_frames` 与
/// 导出侧复算的 PDC 读数不一致 —— 判据 ⑧）。
pub fn build_receipt(options: &FixtureOptions) -> Result<Reading, String> {
    let gain_db = match options.gain {
        EdgeGain::Identity => None,
        EdgeGain::Db(db) => Some(db),
    };
    let (graph, master, source_nodes) = reference_project(options.tracks, gain_db);

    let mut latencies: BTreeMap<EntityId, u32> = BTreeMap::new();
    if let LatencyMode::Staircase = options.latency {
        for (index, node) in source_nodes.iter().enumerate() {
            latencies.insert(*node, (index as u32 % LATENCY_PERIOD) * LATENCY_STEP);
        }
    }

    let plan_options = match options.threads {
        Threads::Auto => RenderOptions::l1(options.frames, CHANNELS, SAMPLE_RATE, SEED),
        Threads::Fixed(count) => {
            RenderOptions::l1(options.frames, CHANNELS, SAMPLE_RATE, SEED).with_threads(count)
        }
    };

    let mut plan = RenderPlan::compile_with_latencies(&graph, master, plan_options, &latencies)
        .map_err(|error| format!("渲染计划编译失败: {error}"))?;
    // 收据记录**渲染计划真正用的**选项，而不是我们以为传进去的参数。
    let used = plan.options();
    let longest_path_frames = plan.longest_path_frames();

    // 导出侧**独立复算** `L_max`（判据 ⑧）。
    //
    // 本夹具是星形图（每条轨道 → Master，无中间总线），因此关键路径就是
    // "某条轨道自己的累计延迟"：`pdc::plan` 的 `arrival[master]` 取各入边
    // `output_latency` 的最大值，而 Master 自身没有延迟（必需字段，值就是 0）。这与
    // `RenderPlan::longest_path_frames()` 是**两个独立实现**，必须给出同一个数；
    // 不等时 `Receipt::validate` 会拒绝这份收据。
    let pdc_expected_frames = source_nodes
        .iter()
        .filter_map(|node| latencies.get(node).copied())
        .max()
        .unwrap_or(0);

    let output = plan
        .execute(tone_sources(&source_nodes))
        .map_err(|error| format!("渲染失败: {error}"))?;

    let (sample_count, abs_max, sum_squares) = stats_of(&output.samples);

    let mut latency_by_name: BTreeMap<String, u32> = BTreeMap::new();
    for (node, frames) in &latencies {
        latency_by_name.insert(node.to_string(), *frames);
    }

    let receipt = Receipt {
        fingerprint: Fingerprint {
            fixture: "reference-a".to_owned(),
            tracks: options.tracks,
            frames: used.frames,
            channels: used.channels,
            sample_rate: used.sample_rate,
            block_size: used.block_size,
            threads: options.threads,
            seed: used.seed,
            gain: options.gain,
            latency: latency_by_name,
        },
        toolchain: toolchain_identity(),
        digest: output.digest,
        longest_path_frames,
        pdc_expected_frames,
        sample_count,
        abs_max,
        sum_squares,
        ops: path_ops(options.gain, options.latency),
        probes: probes_all(&output.samples, sample_class(options.gain)),
    };
    // 自洽性检查先行：宁可在导出端失败，也不要产出一份无法对账的收据。
    receipt
        .validate()
        .map_err(|message| format!("收据自洽性检查失败: {message}"))?;

    Ok(Reading {
        receipt,
        samples: output.samples,
    })
}

/// 样本类别：路径上有没有超越函数（`[ARCH-DET-001]` 的增益换算走 `libm::powf`）。
#[must_use]
pub fn sample_class(gain: EdgeGain) -> SampleClass {
    match gain {
        EdgeGain::Identity => SampleClass::IeeeExact,
        EdgeGain::Db(_) => SampleClass::Transcendental,
    }
}

/// 路径上的运算声明（ADR-0001 D32 的按类别分策）。顺序固定 ⇒ 收据确定。
#[must_use]
pub fn path_ops(gain: EdgeGain, latency: LatencyMode) -> Vec<PathOp> {
    let ieee = |op: &str| PathOp {
        class: SampleClass::IeeeExact,
        op: op.to_owned(),
    };
    let mut ops = vec![
        ieee("cast@tone-integer-to-f32"),
        ieee("div@tone-noise-normalize"),
        ieee("div@tone-carrier-normalize"),
        ieee("sub@tone-center"),
        ieee("mul@tone-carrier-scale"),
        ieee("mul@tone-noise-scale"),
        ieee("add@tone-mix"),
        ieee("mul@edge-gain-apply"),
        ieee("add@master-serial-reduction"),
    ];
    if let LatencyMode::Staircase = latency {
        // 延迟线是环形缓冲的拷贝，仍属 IEEE 精确类。
        ops.push(ieee("copy@pdc-delay-line"));
    }
    if matches!(gain, EdgeGain::Db(_)) {
        ops.push(PathOp {
            class: SampleClass::Transcendental,
            op: "libm::powf@db-to-linear".to_owned(),
        });
    }
    ops
}

/// 工具链身份：`cfg!` 的编译期事实 + `rustc -vV` 的 host/release。
///
/// 取不到 `rustc` 时 `target_triple` 由 arch/os 合成（仍然**非空**，判据 ② 要求非空）。
#[must_use]
pub fn toolchain_identity() -> Toolchain {
    let (host, version) = rustc_identity();
    let target_arch = std::env::consts::ARCH.to_owned();
    let target_os = std::env::consts::OS.to_owned();
    let target_triple = host.unwrap_or_else(|| format!("{target_arch}-{target_os}"));
    Toolchain {
        target_arch,
        target_os,
        target_env: target_env(),
        target_endian: if cfg!(target_endian = "big") {
            "big".to_owned()
        } else {
            "little".to_owned()
        },
        target_pointer_width: usize::BITS.to_string(),
        target_triple,
        rustc_version: version.unwrap_or_else(|| "unknown".to_owned()),
    }
}

/// `cfg!(target_env)` 的真实取值（只区分本仓 CI 会出现的几种；完整身份看 `target_triple`）。
#[must_use]
pub fn target_env() -> String {
    if cfg!(target_env = "gnu") {
        "gnu".to_owned()
    } else if cfg!(target_env = "musl") {
        "musl".to_owned()
    } else if cfg!(target_env = "msvc") {
        "msvc".to_owned()
    } else {
        String::new()
    }
}

/// 跑 `rustc -vV`，取 `host:` 与 `release:` + `commit-hash:` + `commit-date:`。
#[must_use]
pub fn rustc_identity() -> (Option<String>, Option<String>) {
    let Ok(output) = Command::new("rustc").arg("-vV").output() else {
        return (None, None);
    };
    if !output.status.success() {
        return (None, None);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let field = |name: &str| -> Option<String> {
        text.lines()
            .find_map(|line| line.strip_prefix(name).map(str::trim).map(str::to_owned))
    };
    let host = field("host:");
    let version = match (
        field("release:"),
        field("commit-hash:"),
        field("commit-date:"),
    ) {
        (Some(release), Some(commit), Some(date)) => Some(format!("{release} ({commit} {date})")),
        (Some(release), _, _) => Some(release),
        _ => None,
    };
    (host, version)
}

// ---------------------------------------------------------------------------
// L1 摘要记录（`MUST-GATE-002`）的共享装配器
// ---------------------------------------------------------------------------

/// 摘要记录里"与机器有关"的元数据（**仅记录**字段的取值来源）。
///
/// 为什么要显式传进来：`host_name` / `generated_at_utc` 这类字段在判据里必须**可注入**，
/// 否则"改了非参与字段仍然绿"这条判据就只能靠改文件来做（那就不是判据了）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DigestMetadata {
    /// 基准指令集记号（如 `baseline` / `baseline+neon,aes`）。
    pub isa_features: String,
    /// 宿主名（**仅记录**）。
    pub host_name: String,
    /// 宿主 OS 版本（**仅记录**）。
    pub host_os_version: String,
    /// 生成时刻（**仅记录**）。
    pub generated_at_utc: String,
    /// 备注（**仅记录**）。
    pub notes: String,
}

/// 把一次真实渲染读数装配成 [`DigestRecord`]（`export_l1_digest` 与
/// `tests/l1_digest_parity.rs` **共用这一份** —— 判据与生成器不许各写一份）。
///
/// 它同时完成"无损自证"：编码出的 WAV 的 `data` 载荷必须逐字节等于样本位型串接，
/// 且三段哈希（位型 / 载荷 / 从容器里取回的载荷）必须全同。做不到就返回 `Err`，
/// 绝不产出一份口径可疑的摘要。
///
/// # Errors
///
/// 声道数超出 `u16`、容器载荷与位型不一致、任一哈希不自洽时返回说明。
pub fn digest_record_from_reading(
    reading: &Reading,
    scope: l1_digest_record::DigestScope,
    metadata: &DigestMetadata,
) -> Result<(l1_digest_record::DigestRecord, Vec<u8>), String> {
    use l1_digest_record::{
        DIGEST_INPUT, DigestEnvelope, DigestRecord, PlatformIdentity, RenderParams, SAMPLE_FORMAT,
        SCHEMA, WAV_ENCODING, encode_wav_f32_le, hex_lower, pcm_bits_bytes, sample_digest_of,
        sha256, wav_data_payload,
    };

    let receipt = &reading.receipt;
    let fingerprint = &receipt.fingerprint;
    let channels = u16::try_from(fingerprint.channels)
        .map_err(|_| format!("声道数 {} 超出 u16", fingerprint.channels))?;
    let samples = &reading.samples;
    let payload = pcm_bits_bytes(samples);
    let wav = encode_wav_f32_le(samples, channels, fingerprint.sample_rate);
    let sample_digest = sample_digest_of(samples);
    let payload_digest = hex_lower(&sha256(&payload));
    let extracted = wav_data_payload(&wav)?;
    if extracted != payload.as_slice() {
        return Err(format!(
            "容器的 data 载荷 ({} 字节) 与位型串接 ({} 字节) 不同: 摘要口径无法自证无损",
            extracted.len(),
            payload.len()
        ));
    }
    let extracted_digest = hex_lower(&sha256(extracted));
    if extracted_digest != sample_digest || payload_digest != sample_digest {
        return Err(format!(
            "无损自证失败: 位型={sample_digest} 载荷={payload_digest} 取出={extracted_digest}"
        ));
    }

    let mut latency: BTreeMap<String, u32> = BTreeMap::new();
    for (node, frames) in &fingerprint.latency {
        latency.insert(node.clone(), *frames);
    }
    // ⚠ `rustc -vV` 只跑**一次**：三次调用不仅慢，还可能在不同时刻拿到不同结果。
    let (host, version) = rustc_identity();
    let host = host.unwrap_or_default();
    // `rustc_identity` 的第二个返回值形如 `"1.99.0 (b940084d7 2026-09-28)"`；
    // 锁定工具链的钥匙是它开头的**发布号**，构建元数据只做记录。
    let (release, commit, commit_date) = l1_digest_record::split_rustc_version(version.as_deref());

    let record = DigestRecord {
        schema: SCHEMA.to_owned(),
        params: RenderParams {
            fixture: fingerprint.fixture.clone(),
            seed: fingerprint.seed,
            sample_rate: fingerprint.sample_rate,
            channels: fingerprint.channels,
            frames: fingerprint.frames,
            tracks: fingerprint.tracks,
            block_size: fingerprint.block_size,
            gain_db: fingerprint.gain.to_token(),
            threads: fingerprint.threads.to_token(),
            latency,
        },
        platform: PlatformIdentity {
            target_arch: receipt.toolchain.target_arch.clone(),
            target_os: receipt.toolchain.target_os.clone(),
            target_env: receipt.toolchain.target_env.clone(),
            target_endian: receipt.toolchain.target_endian.clone(),
            target_pointer_width: receipt.toolchain.target_pointer_width.clone(),
            target_triple: receipt.toolchain.target_triple.clone(),
            // 锁定工具链的钥匙：解析后的**发布号**（不是 `rustc -vV` 的全文行）。
            rustc_release: release,
            rustc_host: host,
            rustc_commit: commit,
            rustc_commit_date: commit_date,
            isa_features: metadata.isa_features.clone(),
        },
        envelope: DigestEnvelope {
            algorithm: "sha256".to_owned(),
            input: DIGEST_INPUT.to_owned(),
            sample_format: SAMPLE_FORMAT.to_owned(),
            wav_encoding: WAV_ENCODING.to_owned(),
            scope,
            wav_bytes: wav.len() as u64,
        },
        digest: sample_digest.clone(),
        sample_digest,
        rustc_version: receipt.toolchain.rustc_version.clone(),
        host_name: metadata.host_name.clone(),
        host_os_version: metadata.host_os_version.clone(),
        generated_at_utc: metadata.generated_at_utc.clone(),
        notes: metadata.notes.clone(),
    };
    record
        .validate()
        .map_err(|error| format!("摘要记录自洽性检查失败: {error}"))?;
    Ok((record, wav))
}
