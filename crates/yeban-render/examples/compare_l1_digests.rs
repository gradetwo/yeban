//! # `MUST-GATE-002` 的**摘要比较器**：两份 L1 摘要记录 → 一个结构化判决
//!
//! ```text
//! cargo run --release -p yeban-render --example compare_l1_digests -- <摘要A> <摘要B>
//! cargo run --release -p yeban-render --example compare_l1_digests -- --cross-platform <A> <B>
//! ```
//!
//! ## 退出码（CI 直接可用）
//!
//! | 码 | 含义 |
//! | :--- | :--- |
//! | `0` | 同平台 + 锁工具链 + 参与字段逐字段相同 ⇒ `PASS`；或显式 `--cross-platform` 下跨平台读数逐字节相同 ⇒ `PASS-CROSS-PLATFORM` |
//! | `1` | **`FAIL`**：同平台、锁工具链，参与字段/读数却不同（**硬红**，点名差异字段） |
//! | `2` | **不可比**：跨平台（默认策略）或工具链未锁定 ⇒ `SKIP`；或文件缺失/格式错误/自相矛盾 |
//!
//! **`SKIP` 不是通过**：报告首行是 `VERDICT SKIP`（不是 `VERDICT PASS`），退出码 `2`，
//! 并打印"请勿记为通过"的原因说明。`PASS-CROSS-PLATFORM` 也**不是** `MUST-GATE-002` 的通过
//! （规范要求同平台）—— 它是"比同平台更强"的一条观测，记号刻意与 `PASS` 区分。
//!
//! 判决报告是 `key=value` 文本，首行 `VERDICT <记号>`；**判决只以退出码为准**，
//! `VERDICT` 行是给人看的摘要（因此在 CI 里 `| tee report.txt` 不会吞掉结论）。

use std::process::exit;

// 摘要记录（纯逻辑：字段表 / 归一化 / 判决 / JSON / SHA-256 / WAV 容器）。
#[allow(dead_code)]
#[path = "support/l1_digest_record.rs"]
mod l1_digest_record;

use l1_digest_record::{CrossPlatform, DigestRecord, judge_policy, parse, report};

const USAGE: &str = "用法: compare_l1_digests [--cross-platform] <摘要A> <摘要B>";

fn main() {
    let mut cross_platform = false;
    let mut paths: Vec<String> = Vec::new();
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--cross-platform" => cross_platform = true,
            other if other.starts_with("--") => {
                eprintln!("error: 未知参数 `{other}`");
                eprintln!("{USAGE}");
                exit(2);
            }
            other => paths.push(other.to_owned()),
        }
    }
    if paths.len() != 2 {
        eprintln!("error: 需要恰好两个摘要路径, 收到 {}", paths.len());
        eprintln!("{USAGE}");
        exit(2);
    }
    let mut records = Vec::new();
    for path in &paths {
        match load(path) {
            Ok(record) => records.push(record),
            Err(message) => {
                // "读不到/解析不了"是环境或产物问题, 不是门禁红 ⇒ 退出码 2。
                eprintln!("error: {message}");
                eprintln!("cannot-judge");
                exit(2);
            }
        }
    }
    let policy = if cross_platform {
        CrossPlatform::DigestParity
    } else {
        CrossPlatform::Skip
    };
    match judge_policy(&records[0], &records[1], policy) {
        Ok(judgement) => {
            print!("{}", report(&judgement));
            exit(judgement.verdict.exit_code());
        }
        Err(error) => {
            eprintln!("error: {error}");
            eprintln!("cannot-judge");
            exit(2);
        }
    }
}

fn load(path: &str) -> Result<DigestRecord, String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("读取 {path} 失败: {error}"))?;
    parse(&text).map_err(|error| format!("{path}: {error}"))
}
