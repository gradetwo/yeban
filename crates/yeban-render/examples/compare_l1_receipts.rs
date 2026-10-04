//! # `MUST-GATE-002/003` 的**收据比较器**：两份读数 → 一个结构化判决
//!
//! ```text
//! cargo run --release -p yeban-render --example compare_l1_receipts -- <收据A> <收据B>
//! ```
//!
//! ## 退出码（CI 直接可用）
//!
//! | 码 | 含义 | 典型场景 |
//! | :--- | :--- | :--- |
//! | `0` | 通过：`L1-bit-exact`（位级相同）或 `L2-within-budget`（差异只在超越函数类且落在预算内） | `MUST-GATE-002` / `MUST-GATE-003` 绿 |
//! | `1` | **`FAIL`**：IEEE 精确类出现差异 / 绝对误差 `>= 1e-6` / 超过 D32 的 4096 ulp 预算 | 门禁红，且报告点名越界样本 |
//! | `2` | **无法判决**：收据缺失/格式错误/两份不可比 | 不是"通过"，也不是"样本超差" —— 是环境/产物问题 |
//!
//! 判决报告是 `key=value` 文本，首行 `VERDICT <判决>`，因此 CI 里可以直接
//! `... | tee report.txt` + `grep '^VERDICT'` 而**不会**把门禁结果吞掉
//! （重要：判决必须来自退出码，`VERDICT` 行只是给人看的摘要）。
//!
//! ## 三种判决如何被证明能出现
//!
//! 由 `examples/support/l1_receipt_tests.rs`（**本机可跑**，零重依赖）用人工构造的收据证明：
//! 完全相同 ⇒ `L1-bit-exact`；超越函数类上 1 ulp ⇒ `L2-within-budget`；大幅偏差 ⇒
//! `FAIL` 且点名样本。另有两条反向判据：IEEE 精确类上 1 ulp 也必须 `FAIL`（D32 零容差）、
//! 绝对预算通过但 ulp 超预算也必须 `FAIL`。
//!
//! ## 判决口径（严格按规范）
//!
//! - `MUST-GATE-002`（同平台 bit-exact）/ `MUST-GATE-003`（跨架构 `< 1e-6`）由
//!   **两份收据是否同目标三元组**自动区分（`gate=` 字段）；
//! - **ADR-0001 D32**：每一条样本带类别码。`E`（IEEE 精确类）上任何位级差异直接 `FAIL`；
//!   `T`（超越函数类）上给两条预算：绝对 `< 1e-6`（`MUST-GATE-003`）与 `4096 ulp`（D32）。

#[allow(dead_code)]
#[path = "support/l1_receipt.rs"]
mod l1_receipt;

use l1_receipt::{judge, parse, report};

const USAGE: &str = "用法: compare_l1_receipts <收据A> <收据B>";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        eprintln!("error: 需要恰好两个收据路径, 收到 {}", args.len());
        eprintln!("{USAGE}");
        std::process::exit(2);
    }
    let receipt_a = match load(&args[0]) {
        Ok(receipt) => receipt,
        Err(message) => {
            eprintln!("error: {message}");
            std::process::exit(2);
        }
    };
    let receipt_b = match load(&args[1]) {
        Ok(receipt) => receipt,
        Err(message) => {
            eprintln!("error: {message}");
            std::process::exit(2);
        }
    };
    match judge(&receipt_a, &receipt_b) {
        Ok(judgement) => {
            print!("{}", report(&receipt_a, &receipt_b, &judgement));
            std::process::exit(judgement.verdict.exit_code());
        }
        Err(error) => {
            // "不可比"不是"样本超差"：退出码 2 让 CI 把"环境/产物问题"与"门禁红"分开。
            eprintln!("error: {error}");
            eprintln!("cannot-judge");
            std::process::exit(2);
        }
    }
}

fn load(path: &str) -> Result<l1_receipt::Receipt, String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("读取 {path} 失败: {error}"))?;
    parse(&text).map_err(|error| format!("{path}: {error}"))
}
