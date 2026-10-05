//! # `MUST-GATE-002` 的 **L1 摘要记录生成器**（一行命令产出可存档的读数）
//!
//! ```text
//! cargo run --release -p yeban-render --example export_l1_digest -- --out l1-digest.json
//! ```
//!
//! 它做四件事，缺一不可：
//!
//! 1. **真渲染**参考工程 A（与 `bench_render` / `export_l1_receipt` 共用
//!    `examples/support/reference_project_a.rs` 这一份夹具，不许漂移）；
//! 2. 把交错母带编成**固定布局**的 `pcm-f32-le` WAV
//!    （`support/l1_digest_record.rs` 的 `encode_wav_f32_le`）；
//! 3. 算 SHA-256 并**自证无损**：`data` 载荷的哈希必须等于样本位型串接的哈希
//!    （不等就退出码 1，绝不产出一份口径可疑的摘要）；
//! 4. 输出**单行 JSON**（`--pretty` 给多行存档形态），字段与参与/仅记录身份见
//!    `support/l1_digest_record.rs` 的 `FIELD_TABLE`。
//!
//! 装配逻辑在 `support/export_pipeline.rs` 的 `digest_record_from_reading`，
//! 与 `tests/l1_digest_parity.rs` **同源** —— 判据与生成器不许各写一份（本仓库吃过
//! "两套实现漂移"的亏，见 `export_pipeline.rs` 的模块文档）。
//!
//! ## 为什么"WAV 文件的 SHA-256"要写清口径
//!
//! 规范原文要的是"WAV 文件 SHA-256 哈希"，但容器字节里有一堆**与声学无关**的东西
//! （`bext` 时间戳、RIFF 块顺序、填充、`LIST/INFO`）。摘要记录因此显式写出
//! `digest_input` / `digest_scope` / `sample_format` / `wav_encoding` / `wav_bytes`，
//! 并**自证无损** —— 于是"PCM 载荷的 SHA-256"与"整个无损 WAV 文件的 SHA-256"是同一个数，
//! 不存在"这个哈希到底哈希了什么"的含糊。
//!
//! ## 用法
//!
//! ```text
//! export_l1_digest [--tracks 32] [--frames 8192] [--threads auto|<N>]
//!                  [--gain-db none|<f32>] [--latency none|staircase]
//!                  [--isa <记号>] [--notes <文本>] [--wav <路径>] [--out <路径>] [--pretty]
//! ```
//!
//! - stdout 只有摘要本身（`--out` 时什么都不打），人读摘要一律走 stderr ⇒
//!   重定向出来的文件**就是**归档物；
//! - `SOURCE_DATE_EPOCH`（秒）可覆盖 `generated_at_utc`，便于复现 ——
//!   该字段是**仅记录**的，不参与比对（见 `FIELD_TABLE`）；
//! - `--isa` **必须**与参考摘要里记的一致（默认 `baseline`）：基准指令集是参与字段，
//!   写错就会被判据硬红（这正是判据 ⑤ 的形态）；
//! - 退出码：`0` 成功；`1` 渲染/写入/自证失败；`2` 用法错误。

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

// 渲染 → 摘要记录的共用装配管线（`l1_digest_record` 是它的子模块：**只引入一次**，
// 否则 clippy 的 `duplicate_mod` 会报"同一个文件被当成多个模块加载"）。
#[allow(dead_code)]
#[path = "support/export_pipeline.rs"]
mod export_pipeline;

use export_pipeline::l1_digest_record::DigestScope;
use export_pipeline::l1_receipt::{EdgeGain, Threads};
use export_pipeline::{
    DigestMetadata, FixtureOptions, LatencyMode, build_receipt, digest_record_from_reading,
    l1_digest_record::{format_utc, recorded_only_fields, to_json_line, to_pretty_json},
};

const USAGE: &str = "用法: export_l1_digest [--tracks 32] [--frames 8192] [--threads auto|<N>] \
                     [--gain-db none|<f32>] [--latency none|staircase] [--isa <记号>] \
                     [--notes <文本>] [--wav <路径>] [--out <路径>] [--pretty]";

fn main() {
    match run() {
        Ok(()) => {}
        Err(Failure::Usage(message)) => {
            eprintln!("error: {message}");
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
        Err(Failure::Failed(message)) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
    }
}

/// 失败的两类：用法错误（退出码 2）与操作失败（退出码 1）。
enum Failure {
    Usage(String),
    Failed(String),
}

fn run() -> Result<(), Failure> {
    let args = parse_args()?;
    let reading = build_receipt(&args.options).map_err(Failure::Failed)?;
    let meta = DigestMetadata {
        isa_features: args.isa.clone(),
        host_name: host_name(),
        host_os_version: host_os_version(),
        generated_at_utc: generated_at_utc(),
        notes: args.notes.clone(),
    };
    let (record, wav) = digest_record_from_reading(&reading, DigestScope::PcmPayloadOnly, &meta)
        .map_err(Failure::Failed)?;

    if let Some(path) = &args.wav_out {
        std::fs::write(path, &wav).map_err(|error| {
            Failure::Failed(format!("写入 WAV {} 失败: {error}", path.display()))
        })?;
    }

    let text = if args.pretty {
        to_pretty_json(&record)
    } else {
        to_json_line(&record)
    };
    match &args.out {
        Some(path) => std::fs::write(path, &text)
            .map_err(|error| Failure::Failed(format!("写入 {} 失败: {error}", path.display())))?,
        None => print!("{text}"),
    }

    // 人读摘要走 stderr：stdout 必须是**只有**摘要本身（否则重定向出来的文件不是摘要）。
    eprintln!(
        "digest: fixture={} tracks={} frames={} seed={} gain_db={} latency={} threads={} \
         target={} rustc={} isa={} digest={} wav_bytes={} scope={}",
        record.params.fixture,
        record.params.tracks,
        record.params.frames,
        record.params.seed,
        record.params.gain_db,
        if record.params.latency.is_empty() {
            "none"
        } else {
            "staircase"
        },
        record.params.threads,
        record.platform.target_triple,
        record.platform.rustc_release,
        record.platform.isa_features,
        record.digest,
        record.envelope.wav_bytes,
        record.envelope.scope.token(),
    );
    eprintln!(
        "口径: digest_input={} sample_format={} wav_encoding={} (仅记录字段 {} 个: {})",
        record.envelope.input,
        record.envelope.sample_format,
        record.envelope.wav_encoding,
        recorded_only_fields().len(),
        recorded_only_fields().join(", ")
    );
    Ok(())
}

/// 命令行参数。
struct Args {
    options: FixtureOptions,
    isa: String,
    notes: String,
    wav_out: Option<PathBuf>,
    out: Option<PathBuf>,
    pretty: bool,
}

fn parse_args() -> Result<Args, Failure> {
    let mut options = FixtureOptions::default();
    let mut isa = "baseline".to_owned();
    let mut notes = String::new();
    let mut wav_out: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut pretty = false;
    let mut iter = std::env::args().skip(1);
    while let Some(flag) = iter.next() {
        let mut value = |name: &str| -> Result<String, Failure> {
            iter.next()
                .ok_or_else(|| Failure::Usage(format!("`{name}` 缺少取值")))
        };
        match flag.as_str() {
            "--tracks" => {
                let raw = value("--tracks")?;
                options.tracks = raw
                    .parse()
                    .map_err(|_| Failure::Usage(format!("`--tracks` 非法: `{raw}`")))?;
            }
            "--frames" => {
                let raw = value("--frames")?;
                options.frames = raw
                    .parse()
                    .map_err(|_| Failure::Usage(format!("`--frames` 非法: `{raw}`")))?;
            }
            "--threads" => {
                let raw = value("--threads")?;
                options.threads = if raw == "auto" {
                    Threads::Auto
                } else {
                    let count: usize = raw
                        .parse()
                        .map_err(|_| Failure::Usage(format!("`--threads` 非法: `{raw}`")))?;
                    if count == 0 {
                        return Err(Failure::Usage("`--threads` 不能是 0".to_owned()));
                    }
                    Threads::Fixed(count)
                };
            }
            "--gain-db" => {
                let raw = value("--gain-db")?;
                options.gain = if raw == "none" {
                    EdgeGain::Identity
                } else {
                    let db: f32 = raw
                        .parse()
                        .map_err(|_| Failure::Usage(format!("`--gain-db` 非法: `{raw}`")))?;
                    if !db.is_finite() {
                        return Err(Failure::Usage(format!("`--gain-db` 非有限: `{raw}`")));
                    }
                    EdgeGain::Db(db)
                };
            }
            "--latency" => {
                let raw = value("--latency")?;
                options.latency = match raw.as_str() {
                    "none" => LatencyMode::None,
                    "staircase" => LatencyMode::Staircase,
                    other => {
                        return Err(Failure::Usage(format!(
                            "`--latency` 只认 none / staircase, 实际 `{other}`"
                        )));
                    }
                };
            }
            "--isa" => {
                let raw = value("--isa")?;
                if raw.is_empty() || raw.contains(' ') {
                    return Err(Failure::Usage(format!(
                        "`--isa` 必须是不含空格的记号 (如 baseline 或 x86-64-v3+fma), 实际 `{raw}`"
                    )));
                }
                isa = raw;
            }
            "--notes" => notes = value("--notes")?,
            "--wav" => wav_out = Some(PathBuf::from(value("--wav")?)),
            "--out" => out = Some(PathBuf::from(value("--out")?)),
            "--pretty" => pretty = true,
            other => return Err(Failure::Usage(format!("未知参数 `{other}`"))),
        }
    }
    if options.frames == 0 {
        return Err(Failure::Usage("`--frames` 不能是 0".to_owned()));
    }
    if options.tracks == 0 {
        return Err(Failure::Usage("`--tracks` 不能是 0".to_owned()));
    }
    Ok(Args {
        options,
        isa,
        notes,
        wav_out,
        out,
        pretty,
    })
}

/// 宿主名（**仅记录**：判据 ⑥ 靠它证明"非参与字段真的不参与"）。
fn host_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| uname("-n"))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// 宿主 OS 版本（**仅记录**）：`<sysname> <release> (<machine>)`。
///
/// ⚠ 实测教训：**不许**靠 `uname -srm` 的**字段位置**猜语义。第一次实现把 `-srm` 当成
/// "`-s -m -r`" 读，于是 `host_name` 拿到了内核版本号 `27.0.0`、`host_os_version` 拿到了
/// `arm64`。非参与字段写错**不会**让任何判据变红（那正是判据 ⑥ 要证明的性质），
/// 但人类读账本时会误解 ⇒ 每个字段都用自己的开关**按名字**取。
fn host_os_version() -> String {
    let sysname = uname("-s").unwrap_or_default();
    let release = uname("-r").unwrap_or_default();
    let machine = uname("-m").unwrap_or_default();
    if sysname.is_empty() {
        return format!("{} (unknown kernel)", std::env::consts::OS);
    }
    format!("{sysname} {release} ({machine})")
}

/// 跑一次 `uname <开关>` 并返回首个字段。
fn uname(flag: &str) -> Option<String> {
    let output = Command::new("uname").arg(flag).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .map(std::borrow::ToOwned::to_owned)
}

/// 生成时刻（UTC，秒精度）。`SOURCE_DATE_EPOCH` 可覆盖 ⇒ 复现时不产生假差异。
fn generated_at_utc() -> String {
    let seconds = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|raw| raw.trim().parse::<i64>().ok())
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |delta| delta.as_secs() as i64)
        });
    format_utc(seconds)
}
