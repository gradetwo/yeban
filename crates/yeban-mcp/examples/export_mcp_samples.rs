//! 规范样本导出入口（**非测试**）[MUST-GATE-010, TEST-SPEC-005, MCP-TOOL-001..010]。
//!
//! 用法:
//!
//! ```text
//! cargo run -p yeban-mcp --example export_mcp_samples -- [--out <dir>]
//! ```
//!
//! 默认目录是 `<repo>/target/schema-samples`（见
//! [`yeban_mcp::samples::default_out_dir`]）。导出后由
//! `python3 scripts/gates/validate_schemas.py --samples-dir <dir>` 交给
//! **Python jsonschema** 对账 —— 与写出样本的 **Rust serde** 互为独立实现。
//!
//! 为什么要有可执行入口（与 `yeban-model` 同一条理由）：样本曾经只能靠 `#[test]`
//! 的副作用产生，那会让"导出失败"看起来像"测试通过"。现在测试与 CI 走的是同一个
//! 库函数 [`yeban_mcp::samples::export_all`]，只是入口不同。

use std::path::PathBuf;
use std::process::ExitCode;

use yeban_mcp::samples::{default_out_dir, export_all};

/// `--help` 文本。
const USAGE: &str = "\
用法: export_mcp_samples [--out <dir>]

把 yeban-mcp 的规范样本写到 <dir>:
  mcp-tools.registry.meta.json        文档样本: 十个工具的注册表快照
  mcp-tools.error-codes.meta.json     文档样本: 错误码四个集合 + D25 历史
  mcp-tools.response.dry-run.json     契约实例: 真实管线产出的 ToolResponse (dryRun)
  mcp-tools.call.<tool>.json          契约实例: 每个工具一份规范 ToolCall (10 份)

`.meta.json` = 文档样本(顶层是清单/快照, 跳过 schema 对账); 其余 = 契约实例(必须通过根校验)。
两者都由 tests/contract.rs 的双射判据钉住(实例集合穷举, 改名成 .meta. 逃逸会红)。

不传 --out 时写到 <repo>/target/schema-samples。
";

/// 解析结果。
enum Invocation {
    /// 导出到该目录。
    Export(PathBuf),
    /// 打印用法并以 0 退出。
    Help,
}

fn main() -> ExitCode {
    let dir = match parse_args(std::env::args().skip(1)) {
        Ok(Invocation::Export(dir)) => dir,
        Ok(Invocation::Help) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("error: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    match export_all(&dir) {
        Ok(written) => {
            for path in &written {
                println!("{}", path.display());
            }
            println!("导出 {} 份样本到 {}", written.len(), dir.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: 导出样本失败: {error}");
            ExitCode::from(1)
        }
    }
}

/// 解析 `--out <dir>` / `--help`。
fn parse_args<I: Iterator<Item = String>>(mut args: I) -> Result<Invocation, String> {
    let mut out: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Invocation::Help),
            "--out" => {
                let value = args.next().ok_or("`--out` 需要一个目录参数")?;
                out = Some(PathBuf::from(value));
            }
            other => return Err(format!("未知参数 `{other}`")),
        }
    }
    Ok(Invocation::Export(out.unwrap_or_else(default_out_dir)))
}
