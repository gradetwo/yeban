//! 规范样本导出入口（**非测试**）[MUST-GATE-010, TEST-SPEC-005]。
//!
//! 用法:
//!
//! ```text
//! cargo run -p yeban-model --example export_schema_samples -- [--out <dir>]
//! ```
//!
//! 默认目录是 `<repo>/target/schema-samples`（见
//! [`yeban_model::samples::default_out_dir`]）。导出后由
//! `python3 scripts/gates/validate_schemas.py --samples-dir <dir>` 交给
//! **Python jsonschema** 逐份对账 —— 与写出样本的 **Rust serde** 互为独立实现。
//!
//! 为什么要有这个可执行入口：样本曾经只能靠 `#[test]` 的副作用产生，那会让
//! "导出失败"看起来像"测试通过"，也让 CI 必须靠跑测试来生成对账输入。
//! 现在测试与 CI 走的是同一个库函数
//! [`yeban_model::samples::export_all`]，只是**入口不同**。

use std::path::PathBuf;
use std::process::ExitCode;

use yeban_model::samples::{default_out_dir, export_all};

/// `--help` 文本。
const USAGE: &str = "\
用法: export_schema_samples [--out <dir>]

把夜半的规范样本 (project.default/filled.json, ops.default/filled.json) 写到 <dir>。
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
            eprintln!("导出样本失败: {error}");
            ExitCode::FAILURE
        }
    }
}

/// 解析命令行参数（只用标准库：这个入口不值得引入 clap）。
///
/// 支持 `--out <dir>` / `--out=<dir>` / `-o <dir>` / `--help` / `-h`。
///
/// # Errors
///
/// 未知参数或 `--out` 缺少取值时返回人话错误。
fn parse_args(args: impl Iterator<Item = String>) -> Result<Invocation, String> {
    let mut out: Option<PathBuf> = None;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(Invocation::Help),
            "--out" | "-o" => {
                let value = args.next().ok_or("`--out` 后面必须跟一个目录")?;
                out = Some(PathBuf::from(value));
            }
            _ => {
                if let Some(value) = arg.strip_prefix("--out=") {
                    if value.is_empty() {
                        return Err("`--out=` 后面必须跟一个目录".to_owned());
                    }
                    out = Some(PathBuf::from(value));
                } else {
                    return Err(format!("未知参数 `{arg}`"));
                }
            }
        }
    }
    Ok(Invocation::Export(out.unwrap_or_else(default_out_dir)))
}
