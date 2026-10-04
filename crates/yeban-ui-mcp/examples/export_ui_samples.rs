//! 规范样本导出（**非测试**入口）：`cargo run -p yeban-ui-mcp --example export_ui_samples -- --out <dir>`。
//!
//! 与 `yeban-model` / `yeban-mcp` 的导出器同一套做法，工具本身不做断言 ——
//! 它只写文件并打印每一份的路径，对账交给
//! `python3 scripts/gates/validate_schemas.py --samples-dir <dir>`（两个独立实现互相钉住）。
//!
//! ## ⚠️ 顺序要求（集成者必读）
//!
//! 本 crate 导出的三份样本**全部**是 `.meta.` 文档样本（理由见 `src/samples.rs` 的模块文档）。
//! 对账脚本有一条前缀守卫："每个前缀至少要有一份**真实例**，否则该契约等于没有对账"。
//! 因此本导出器必须跑在 `export_mcp_samples` **之后**（后者的 `mcp-tools.call.*.json`
//! 是同一前缀下的真实例）。单独跑本导出器时，脚本会**响亮地**报告：
//!
//! ```text
//! 前缀 `mcp-tools` 只有 .meta. 文档样本, 没有任何真实例 —— 该契约等于没有对账(全是 meta 就是假绿)
//! ```
//!
//! 这是**有意的**行为（判据 `ui_samples_alone_leave_the_prefix_without_a_real_instance`
//! 钉住它），不是缺陷：一份只有 `.meta.` 的样本目录确实什么都没对账。
//!
//! 导出后本程序会自己检查这一点并**如实打印**（当目录里没有同前缀的真实例时给 WARNING），
//! 免得调用方以为"导出成功 == 对账通过"。

use std::path::PathBuf;
use std::process::ExitCode;

use yeban_ui_mcp::samples;

/// `--help` 文本。
const USAGE: &str = "\
用法: export_ui_samples [--out <目录>]

  --out <目录>   样本输出目录 (默认: <仓库>/target/schema-samples)
  --help        显示本帮助

导出 3 份 `.meta.` **文档样本** (方法注册表 / 安全矩阵 / 控件树投影)。
对账: python3 scripts/gates/validate_schemas.py --samples-dir <目录>
";

fn main() -> ExitCode {
    let mut out: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => match args.next() {
                Some(value) => out = Some(PathBuf::from(value)),
                None => {
                    eprintln!("error: `--out` 需要一个目录参数\n{USAGE}");
                    return ExitCode::from(2);
                }
            },
            "--help" | "-h" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("error: 未知参数 `{other}`\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let dir = out.unwrap_or_else(samples::default_out_dir);

    match samples::export_all(&dir) {
        Ok(paths) => {
            for path in &paths {
                println!("{}", path.display());
            }
            println!("导出 {} 份文档样本 (.meta.)", paths.len());
            report_prefix_dependency(&dir);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: 导出样本失败: {error}");
            ExitCode::FAILURE
        }
    }
}

/// 检查同一前缀下有没有**真实例**（对账脚本的前缀守卫），并如实打印。
fn report_prefix_dependency(dir: &std::path::Path) {
    let prefix = format!("{}.", samples::FILE_PREFIX);
    let mut real_instances: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&prefix) && name.ends_with(".json") && !name.contains(".meta.") {
                real_instances.push(name);
            }
        }
    }
    real_instances.sort();
    if real_instances.is_empty() {
        println!(
            "WARNING: 目录 `{}` 里没有 `{prefix}*` 的真实例 —— \
             对账脚本会报\"前缀只有 .meta. 文档样本\"。\n\
             请先跑: cargo run -p yeban-mcp --locked --example export_mcp_samples -- --out {}",
            dir.display(),
            dir.display()
        );
    } else {
        println!(
            "同前缀真实例 {} 份 (对账脚本的前缀守卫会用到它们)",
            real_instances.len()
        );
    }
}
