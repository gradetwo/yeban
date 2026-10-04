//! stdio 传输：逐行 JSON-RPC（形态 B，[ROAD-M4-002] 的同款形状）。
//!
//! 契约与 `yeban-mcp::transport::stdio` **完全一致**（本文件是同一套语义的第二份实现，
//! 因为分发器类型不同）：
//!
//! - 标准输入上一行一个 JSON-RPC 2.0 请求，标准输出上一行一个响应；
//! - 空行被跳过（不产生输出，也不报错）；
//! - 非法 JSON 产生一行 `id = null` 的解析错误 —— **不静默丢弃**，
//!   否则批处理调用方会以为"这一行被接受了"；
//! - notification（没有 `id` 键）按规范**不产生任何输出**；
//! - 每一行响应写完立刻 flush（下游 `read_line` 才能及时拿到）。
//!
//! ## 鉴权
//!
//! stdio 上"没有 HTTP 头"被 `AuthContext::from_headers` 解释为
//! `Credential::LocalProcess`：**能往本进程写 stdin 就是能力边界**。
//! 若调用方**带了** `Authorization`（例如从 shell 里手工喂一行），它会被照常校验。
//! 注意：`LocalProcess` 只解决"凭据"，**不**解决 scope —— `ui:inject` 在生产模式下
//! 依然在这一层之上被硬拒（见 [`crate::service`]）。

use std::io::{BufRead, Write};

use yeban_mcp::security::Channel;

use crate::service::UiService;

/// 一次批处理的统计。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServeReport {
    /// 读到的非空行数。
    pub lines_read: usize,
    /// 写出的响应行数。
    pub responses_written: usize,
    /// 跳过的空行数。
    pub blank_lines_skipped: usize,
}

/// 逐行处理。
///
/// # Errors
///
/// 读 / 写失败（调用方通常直接以非零码退出）。
pub fn serve_lines<R, W>(
    reader: R,
    mut writer: W,
    service: &mut UiService,
) -> Result<ServeReport, std::io::Error>
where
    R: BufRead,
    W: Write,
{
    let mut report = ServeReport::default();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            report.blank_lines_skipped += 1;
            continue;
        }
        report.lines_read += 1;
        let outcome = service.handle_line(Channel::Stdio, None, line.trim());
        if let Some(response) = outcome.response {
            writeln!(writer, "{}", response.to_json())?;
            writer.flush()?;
            report.responses_written += 1;
        }
    }
    Ok(report)
}

/// 用真实的标准输入 / 标准输出跑一轮批处理。
///
/// # Errors
///
/// 同 [`serve_lines`]。
pub fn serve_stdio(service: &mut UiService) -> Result<ServeReport, std::io::Error> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    serve_lines(stdin.lock(), stdout.lock(), service)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::methods;
    use crate::testing::{FakeSurface, fixture_tree, shared};
    use yeban_mcp::security::{BearerToken, RunMode, ScopeSet};
    use yeban_ui_test_port::port::Permission;

    fn build_service() -> UiService {
        let state = shared(Permission::Administrative);
        UiService::new(
            BearerToken::generate().token,
            ScopeSet::all(),
            RunMode::Production,
            Box::new(FakeSurface {
                state,
                tree: fixture_tree(),
            }),
        )
    }

    fn run(input: &str) -> (ServeReport, Vec<serde_json::Value>) {
        let mut service = build_service();
        let mut output: Vec<u8> = Vec::new();
        let report = serve_lines(input.as_bytes(), &mut output, &mut service).expect("批处理");
        let text = String::from_utf8(output).expect("UTF-8");
        let lines = text
            .lines()
            .map(|line| serde_json::from_str(line).expect("每行必须是 JSON"))
            .collect::<Vec<_>>();
        (report, lines)
    }

    /// 判据: 逐行处理、回显 id、空行跳过、非法 JSON 独立成行、notification 无输出。
    #[test]
    fn processes_lines_and_reports_errors_on_their_own_line() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"ui/methods"}"#,
            "\n",
            "\n",
            r#"{"jsonrpc":"2.0","id":"two","method":"ui/tree","params":{"prefix":"track-"}}"#,
            "\n",
            "{oops\n",
            r#"{"jsonrpc":"2.0","method":"ui/tree"}"#,
            "\n",
        );
        let (report, lines) = run(input);
        assert_eq!(report.lines_read, 4);
        assert_eq!(report.responses_written, 3, "notification 不产生输出");
        assert_eq!(report.blank_lines_skipped, 1);
        assert_eq!(lines.len(), 3);

        assert_eq!(lines[0]["id"], 1);
        assert_eq!(
            lines[0]["result"]["methods"]
                .as_array()
                .expect("数组")
                .len(),
            methods::METHOD_COUNT
        );
        assert_eq!(lines[1]["id"], "two");
        assert_eq!(lines[1]["result"]["tree"]["count"], 1);
        assert_eq!(lines[2]["id"], serde_json::Value::Null);
        assert_eq!(
            lines[2]["error"]["code"],
            yeban_mcp::jsonrpc::PARSE_ERROR,
            "非法 JSON 必须独立成行, 不静默丢弃"
        );
    }

    /// 判据: 生产模式下 stdio 通道也**不能**注入 —— 便于本机复现的"同口径"证据。
    #[test]
    fn stdio_channel_cannot_inject_in_production() {
        let (report, lines) = run(concat!(
            r#"{"jsonrpc":"2.0","id":9,"method":"ui/dispatch_key_press","params":{"keyCode":"Tab"}}"#,
            "\n",
        ));
        assert_eq!(report.lines_read, 1);
        assert_eq!(lines[0]["error"]["code"], yeban_mcp::jsonrpc::FORBIDDEN);
        assert_eq!(lines[0]["error"]["data"]["kind"], "forbidden-in-production");
    }
}
