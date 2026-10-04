//! # `MUST-GATE-002/003` 的**读数导出**：把一次渲染变成一份可跨机比较的收据
//!
//! 这是让 `MUST-GATE-003`（跨架构 L2 一致性 `< 1e-6`）**可以执行**的那一半：
//!
//! ```text
//! # x86_64 runner
//! cargo run --release -p yeban-render --example export_l1_receipt -- --out receipt-x86.txt
//! # ubuntu-24.04-arm runner
//! cargo run --release -p yeban-render --example export_l1_receipt -- --out receipt-arm.txt
//! # 任意一台机器（或 CI 的下一个 job）
//! cargo run --release -p yeban-render --example compare_l1_receipts -- receipt-x86.txt receipt-arm.txt
//! ```
//!
//! ## 它渲染什么
//!
//! **参考工程 A**（与 `examples/bench_render.rs` 共用同一份夹具文件
//! `examples/support/reference_project_a.rs`：32 轨 → 母线星形路由），固定长度。
//! 渲染路径只有一条：`RenderPlan::compile_with_latencies` / `execute` 与
//! `RenderOutput::digest`。装配逻辑在 `examples/support/export_pipeline.rs`，
//! 与 `tests/l1_digest_contract.rs` **同源**。
//!
//! ## 收据为什么没有时间戳
//!
//! 判据 ① 要求"同一台机器两次导出 ⇒ 收据**逐字节相同**"。任何时间戳/耗时/机器名都会
//! 破坏它。因此收据里只有**输入指纹 + 输入无关的读数 + 工具链身份**。
//!
//! ## 默认长度为什么是 8192 帧而不是 30 秒
//!
//! 收据携带**全部**样本位型（不做抽样：抽样会让"最大误差"变成假绿，见
//! `support/l1_receipt.rs` 的模块文档）。30 秒立体声 48 kHz 的文本约 37 MB；
//! 8192 帧（约 171 ms）约 300 KB —— 足够上 CI artifact，也足够让 32 轨的路径跑出
//! 多样化的运算组合。要跑更长：`--frames 1440000`（**两台机器必须用同一组参数**）。
//!
//! ## 用法
//!
//! ```text
//! export_l1_receipt [--tracks 32] [--frames 8192] [--threads auto|<N>]
//!                   [--gain-db none|<f32>] [--latency none|staircase] [--out <路径>]
//! ```
//!
//! - `--gain-db <f32>`：给每条边写入 `RoutingEdge::gain_db`。增益由 `libm::powf`
//!   （`render::db_to_linear`）得出 ⇒ 路径上出现**超越函数类**运算，全部样本的类别
//!   变成 `T`。`none`（默认）保持参考工程 A 的字面形态（纯 IEEE 精确类）。
//! - `--latency staircase`：给第 i 条轨道注入 `(i % 4) * 64` 帧的设备延迟，
//!   于是 `L_max = 192` 帧（`[ARCH-PDC-001]`）。收据同时记录**渲染计划**给出的
//!   `longest_path_frames` 与导出侧**独立复算**的 `pdc_expected_frames`；
//!   两者不等时收据被拒（判据 ⑧）。
//! - `--out <路径>` 不写时收据打到 stdout（stdout 只有收据本身，日志一律走 stderr）。
//!
//! 退出码：`0` 成功；`1` 渲染/写入失败；`2` 用法错误。

use std::path::PathBuf;

// 收据格式 + 夹具 + 装配管线（`l1_receipt` / `reference_project_a` 是它的子模块）。
#[allow(dead_code)]
#[path = "support/export_pipeline.rs"]
mod export_pipeline;

use export_pipeline::l1_receipt::{EdgeGain, Threads, to_text};
use export_pipeline::{FixtureOptions, LatencyMode, build_receipt};

const USAGE: &str = "用法: export_l1_receipt [--tracks 32] [--frames 8192] [--threads auto|<N>] \
                     [--gain-db none|<f32>] [--latency none|staircase] [--out <路径>]";

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
    let (options, out) = parse_args()?;
    let reading = build_receipt(&options).map_err(Failure::Failed)?;
    let receipt = &reading.receipt;
    let text = to_text(receipt);
    match &out {
        Some(path) => std::fs::write(path, &text)
            .map_err(|error| Failure::Failed(format!("写入 {} 失败: {error}", path.display())))?,
        None => print!("{text}"),
    }
    // 人读摘要走 stderr：stdout 必须**只有**收据本身（否则重定向出来的文件无法解析）。
    eprintln!(
        "receipt: fixture={} tracks={} frames={} gain_db={} latency={} threads={} \
         target={} digest={} samples={} longest_path_frames={}",
        receipt.fingerprint.fixture,
        receipt.fingerprint.tracks,
        receipt.fingerprint.frames,
        receipt.fingerprint.gain.to_token(),
        options.latency.as_str(),
        receipt.fingerprint.threads.to_token(),
        receipt.toolchain.target_triple,
        receipt.digest_hex(),
        receipt.sample_count,
        receipt.longest_path_frames,
    );
    Ok(())
}

fn parse_args() -> Result<(FixtureOptions, Option<PathBuf>), Failure> {
    let mut options = FixtureOptions::default();
    let mut out: Option<PathBuf> = None;
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
                if raw == "auto" {
                    options.threads = Threads::Auto;
                } else {
                    let count: usize = raw
                        .parse()
                        .map_err(|_| Failure::Usage(format!("`--threads` 非法: `{raw}`")))?;
                    if count == 0 {
                        return Err(Failure::Usage("`--threads` 不能是 0".to_owned()));
                    }
                    options.threads = Threads::Fixed(count);
                }
            }
            "--gain-db" => {
                let raw = value("--gain-db")?;
                if raw == "none" {
                    options.gain = EdgeGain::Identity;
                } else {
                    let db: f32 = raw
                        .parse()
                        .map_err(|_| Failure::Usage(format!("`--gain-db` 非法: `{raw}`")))?;
                    if !db.is_finite() {
                        return Err(Failure::Usage(format!("`--gain-db` 非有限: `{raw}`")));
                    }
                    options.gain = EdgeGain::Db(db);
                }
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
            "--out" => out = Some(PathBuf::from(value("--out")?)),
            other => return Err(Failure::Usage(format!("未知参数 `{other}`"))),
        }
    }
    if options.frames == 0 {
        return Err(Failure::Usage("`--frames` 不能是 0".to_owned()));
    }
    if options.tracks == 0 {
        return Err(Failure::Usage("`--tracks` 不能是 0".to_owned()));
    }
    Ok((options, out))
}
