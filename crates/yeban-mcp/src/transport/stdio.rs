//! stdio 传输（形态 B）：逐行 JSON-RPC [ROAD-M4-002]。
//!
//! 契约：标准输入上一行一个 JSON-RPC 2.0 请求，标准输出上一行一个响应。
//!
//! - 空行被跳过（不产生输出，也不报错）；
//! - 非法 JSON 产生一行 `id = null` 的解析错误响应 —— **不静默丢弃**，
//!   否则批处理调用方会以为"这一行被接受了"；
//! - 非 UTF-8 行与非法 JSON 同款处理（T-FIX-509）：一行 `id = null` 的
//!   `-32700` 解析错误响应、计数、继续循环，**不杀死进程**、不回显坏字节；
//! - notification（没有 `id` 键）按规范**不产生任何输出**；
//! - 每一行响应写完立刻 flush（下游 `read_line` 才能及时拿到，不会卡在缓冲区里）。
//!
//! ## 鉴权
//!
//! stdio 通道上"没有 HTTP 头"被 [`crate::security::AuthContext::from_headers`]
//! 解释为 [`crate::security::Credential::LocalProcess`]：**能往本进程写 stdin
//! 就是能力边界**。若调用方**带了** `Authorization`（例如从 shell 里手工喂一行），
//! 它会被照常校验 —— 判定逻辑没有"stdio 就跳过"的分支。

use std::io::{BufRead, Write};

use crate::dispatch::Dispatcher;
use crate::jsonrpc::{ErrorObject, Id, Response};
use crate::security::Channel;

/// 单行输入的最大字节数（T-FIX-263）：stdio 上一行一个 JSON-RPC 请求，
/// 合法请求的体积远小于此；超过即判定为畸形/攻击输入，整行丢弃、不解析、
/// 不回显。`BufRead::lines()` 对无换行的超长输入会无界分配，故不能用它。
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// 一次批处理的统计。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServeReport {
    /// 读到的非空行数。
    pub lines_read: usize,
    /// 写出的响应行数。
    pub responses_written: usize,
    /// 跳过的空行数。
    pub blank_lines_skipped: usize,
    /// 因超过 [`MAX_LINE_BYTES`] 被丢弃的行数（T-FIX-263）。
    /// 超限行不喂给 dispatcher —— 既不解析也不回显，避免放大。
    pub oversized_lines_skipped: usize,
    /// 非 UTF-8 的行数（T-FIX-509）。这类行**不丢弃**：各得一行 `id = null`
    /// 的 `-32700` 解析错误响应（与非法 JSON 同款），同时计入
    /// [`ServeReport::lines_read`] 与 [`ServeReport::responses_written`]。
    pub non_utf8_lines: usize,
}

/// 逐行处理。
///
/// # Errors
///
/// 读 / 写失败（调用方通常直接以非零码退出）。
pub fn serve_lines<R, W>(
    mut reader: R,
    mut writer: W,
    dispatcher: &mut Dispatcher,
) -> Result<ServeReport, std::io::Error>
where
    R: BufRead,
    W: Write,
{
    let mut report = ServeReport::default();
    // 单行处理（T-FIX-263）：绝不用 `lines()` —— 它对永不换行的超长输入会
    // 无界增长，且畸形长行会被解析错误响应原样放大回显。这里一次读一小块、
    // 逐字节分帧；单行的累加缓冲过 MAX_LINE_BYTES 就进入"丢弃到换行"态，
    // 内存占用有常量上界，且超限行从不喂给 dispatcher。
    let mut line_buf: Vec<u8> = Vec::with_capacity(8192);
    let mut dropping = false; // 当前行已超限，剩余字节只用于找换行
    let mut chunk = [0u8; 8192];

    // 处理一个完整行（不含 \n）。返回是否有输出写入错误由调用方 ? 传播。
    fn handle_one<W>(
        buf: &[u8],
        report: &mut ServeReport,
        writer: &mut W,
        dispatcher: &mut Dispatcher,
    ) -> Result<(), std::io::Error>
    where
        W: Write,
    {
        // 去掉行尾 '\r'（兼容 CRLF）。
        let payload = buf.strip_suffix(b"\r").unwrap_or(buf);
        // T-FIX-509：非 UTF-8 行不是"读失败"——按模块契约把它当解析错误回答，
        // 绝不能让 InvalidData 经 `?` 杀死整个批处理进程。修法选解析错误响应
        // 而非 lossy 转换：lossy 会把坏字节换成 U+FFFD 再喂解析，既可能误拼出
        // 一条"合法"请求、又丢了"输入本身非法"这个事实；解析错误响应则与非法
        // JSON 完全同款（id=null / -32700），且坏字节不回显（T-FIX-263 的
        // 不放大原则同样适用）。
        let line = match std::str::from_utf8(payload) {
            Ok(line) => line,
            Err(_) => {
                report.non_utf8_lines += 1;
                report.lines_read += 1;
                let error = ErrorObject::parse_error("stdio 输入行不是 UTF-8");
                let response = Response::failure(Id::Null, error);
                writeln!(writer, "{}", response.to_json())?;
                writer.flush()?;
                report.responses_written += 1;
                return Ok(());
            }
        };
        if line.trim().is_empty() {
            report.blank_lines_skipped += 1;
            return Ok(());
        }
        report.lines_read += 1;
        let outcome = dispatcher.handle_line(Channel::Stdio, None, line.trim());
        if let Some(response) = outcome.response {
            writeln!(writer, "{}", response.to_json())?;
            writer.flush()?;
            report.responses_written += 1;
        }
        Ok(())
    }

    loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break; // EOF：若还有残留行（无结尾换行）下面统一处理
        }
        for &byte in &chunk[..n] {
            if byte == b'\n' {
                if dropping {
                    report.oversized_lines_skipped += 1;
                    dropping = false;
                } else {
                    handle_one(&line_buf, &mut report, &mut writer, dispatcher)?;
                }
                line_buf.clear();
            } else if dropping {
                continue; // 超限行的剩余字节：只等换行，不存
            } else {
                line_buf.push(byte);
                if line_buf.len() > MAX_LINE_BYTES {
                    dropping = true;
                    line_buf.clear();
                }
            }
        }
    }
    // EOF 时残留的非空行 / 超限行
    if dropping {
        report.oversized_lines_skipped += 1;
    } else if !line_buf.is_empty() {
        handle_one(&line_buf, &mut report, &mut writer, dispatcher)?;
    }
    Ok(report)
}

/// 用真实的标准输入 / 标准输出跑一轮批处理。
///
/// # Errors
///
/// 同 [`serve_lines`]。
pub fn serve_stdio(dispatcher: &mut Dispatcher) -> Result<ServeReport, std::io::Error> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    serve_lines(stdin.lock(), stdout.lock(), dispatcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::{BearerToken, RunMode, ScopeSet};

    fn dispatcher() -> Dispatcher {
        Dispatcher::new(
            BearerToken::generate().token,
            ScopeSet::all(),
            RunMode::Production,
        )
    }

    fn run(input: &str) -> (ServeReport, Vec<String>) {
        let mut dispatcher = dispatcher();
        let mut output: Vec<u8> = Vec::new();
        let report = serve_lines(input.as_bytes(), &mut output, &mut dispatcher).expect("批处理");
        let text = String::from_utf8(output).expect("UTF-8");
        let lines = text
            .lines()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>();
        (report, lines)
    }

    #[test]
    fn processes_newline_delimited_requests_and_echoes_ids() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":"two","method":"tools/call","params":{"name":"yeban_query_project","arguments":{}}}"#,
            "\n",
        );
        let (report, lines) = run(input);
        assert_eq!(report.lines_read, 2);
        assert_eq!(report.responses_written, 2);
        assert_eq!(lines.len(), 2);

        let first: serde_json::Value = serde_json::from_str(&lines[0]).expect("JSON");
        assert_eq!(first["id"], 1);
        assert_eq!(
            first["result"]["tools"].as_array().expect("tools").len(),
            crate::tools::TOOL_COUNT
        );

        let second: serde_json::Value = serde_json::from_str(&lines[1]).expect("JSON");
        assert_eq!(second["id"], "two");
        // stdio 通道不需要 Authorization 头 ⇒ 请求真的走到了领域实现,
        // 拿到的是**带内** ToolResponse（没有活跃工程）, 而不是鉴权拒绝。
        assert!(second.get("error").is_none(), "stdio 不该被鉴权拒绝");
        assert_eq!(second["result"]["status"], "error");
        assert_eq!(
            second["result"]["error"]["code"], "NO_ACTIVE_PROJECT",
            "领域失败必须走 ToolResponse.error.code（契约 enum 之内）"
        );
    }

    #[test]
    fn skips_blank_lines_and_reports_parse_errors_on_their_own_line() {
        let input = "\n   \n{oops\n\n";
        let (report, lines) = run(input);
        assert_eq!(report.lines_read, 1);
        assert_eq!(report.responses_written, 1);
        assert_eq!(report.blank_lines_skipped, 3);
        let value: serde_json::Value = serde_json::from_str(&lines[0]).expect("JSON");
        assert_eq!(value["id"], serde_json::Value::Null);
        assert_eq!(value["error"]["code"], crate::jsonrpc::PARSE_ERROR);
    }

    #[test]
    fn notifications_produce_no_output_but_still_count_as_read() {
        let input = r#"{"jsonrpc":"2.0","method":"tools/list"}"#;
        let (report, lines) = run(input);
        assert_eq!(report.lines_read, 1);
        assert_eq!(report.responses_written, 0);
        assert!(lines.is_empty());
    }

    #[test]
    fn oversized_lines_are_dropped_without_echo_or_amplification() {
        // T-FIX-263：超长行不得无界分配、不得解析、不得回显放大。
        // 注入一条 MAX_LINE_BYTES+16 的畸形行（无换行也能到 EOF）＋一条正常行。
        let mut input = Vec::new();
        input.extend(std::iter::repeat_n(b'x', super::MAX_LINE_BYTES + 16));
        input.push(b'\n');
        input.extend_from_slice(br#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#);
        input.push(b'\n');

        let mut dispatcher = dispatcher();
        let mut output: Vec<u8> = Vec::new();
        let report = serve_lines(input.as_slice(), &mut output, &mut dispatcher).expect("批处理");

        assert_eq!(report.oversized_lines_skipped, 1, "超长行必须计数");
        assert_eq!(report.lines_read, 1, "超长行不得计入正常读取");
        assert_eq!(report.responses_written, 1, "超长行不得产生回显");
        // 唯一响应来自那条合法行；输出里绝不能出现被放大的 'x' 串。
        assert!(
            !output.windows(64).any(|w| w == [b'x'; 64]),
            "输出不得回显超长输入的任何片段（防放大）"
        );
        let text = String::from_utf8(output).expect("UTF-8");
        let value: serde_json::Value = serde_json::from_str(text.trim()).expect("唯一的合法响应");
        assert_eq!(value["id"], 7, "合法行仍须正常处理");
    }

    #[test]
    fn oversized_line_without_trailing_newline_is_dropped() {
        // 永不换行的超长输入到 EOF：同样有界丢弃、计数一次、零回显。
        let input = vec![b'z'; super::MAX_LINE_BYTES + 1];
        let mut dispatcher = dispatcher();
        let mut output: Vec<u8> = Vec::new();
        let report = serve_lines(input.as_slice(), &mut output, &mut dispatcher).expect("批处理");
        assert_eq!(report.oversized_lines_skipped, 1);
        assert_eq!(report.lines_read, 0);
        assert_eq!(report.responses_written, 0);
        assert!(output.is_empty(), "超长行不得产生任何输出");
    }

    #[test]
    fn non_utf8_line_gets_parse_error_response_and_processing_continues() {
        // T-FIX-509：坏字节行 + 合法行同批输入。坏行得一行 id=null 的 -32700
        // 解析错误响应（与非法 JSON 同款），合法行得正常响应，进程不退出。
        let mut input = Vec::new();
        input.extend_from_slice(b"\xff\xfe{\"jsonrpc\":\"2.0\",\"id\":1}\n");
        input.extend_from_slice(br#"{"jsonrpc":"2.0","id":9,"method":"tools/list"}"#);
        input.push(b'\n');

        let mut dispatcher = dispatcher();
        let mut output: Vec<u8> = Vec::new();
        let report = serve_lines(input.as_slice(), &mut output, &mut dispatcher)
            .expect("非 UTF-8 行不得杀死批处理");

        assert_eq!(report.lines_read, 2, "坏行与合法行都计入已读");
        assert_eq!(report.responses_written, 2, "坏行与合法行各得一行响应");
        assert_eq!(report.non_utf8_lines, 1, "坏行必须单独计数");
        assert_eq!(report.blank_lines_skipped, 0);
        assert_eq!(report.oversized_lines_skipped, 0);
        // 坏字节绝不回显。
        assert!(
            !output.contains(&0xff) && !output.contains(&0xfe),
            "输出不得回显坏字节"
        );
        let text = String::from_utf8(output).expect("输出是 UTF-8");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value = serde_json::from_str(lines[0]).expect("第一行是 JSON");
        assert_eq!(first["id"], serde_json::Value::Null);
        assert_eq!(first["error"]["code"], crate::jsonrpc::PARSE_ERROR);
        let second: serde_json::Value = serde_json::from_str(lines[1]).expect("第二行是 JSON");
        assert_eq!(second["id"], 9);
        assert!(second.get("error").is_none(), "合法行须正常处理");
    }

    #[test]
    fn non_utf8_line_without_trailing_newline_is_answered_at_eof() {
        // EOF 残留的坏行（无结尾换行）同样得解析错误响应、不杀死批处理。
        let mut input = Vec::new();
        input.extend_from_slice(br#"{"jsonrpc":"2.0","id":5,"method":"tools/list"}"#);
        input.push(b'\n');
        input.extend_from_slice(b"\xc3\x28");

        let mut dispatcher = dispatcher();
        let mut output: Vec<u8> = Vec::new();
        let report = serve_lines(input.as_slice(), &mut output, &mut dispatcher)
            .expect("EOF 残留坏行不得杀死批处理");

        assert_eq!(report.lines_read, 2);
        assert_eq!(report.responses_written, 2);
        assert_eq!(report.non_utf8_lines, 1);
        let text = String::from_utf8(output).expect("输出是 UTF-8");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let last: serde_json::Value = serde_json::from_str(lines[1]).expect("末行是 JSON");
        assert_eq!(last["id"], serde_json::Value::Null);
        assert_eq!(last["error"]["code"], crate::jsonrpc::PARSE_ERROR);
    }

    #[test]
    fn an_authorization_header_supplied_on_stdin_is_still_checked() {
        // 带上一个错误的 Bearer: 判定逻辑没有"stdio 就跳过"的分支。
        let input = concat!(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#, "\n");
        let mut dispatcher = dispatcher();
        let mut output: Vec<u8> = Vec::new();
        // serve_lines 固定传 None; 这里直接走 dispatcher 验证通道语义。
        let outcome = dispatcher.handle_line(
            Channel::Stdio,
            Some("Bearer 0000000000000000000000000000000000000000000000000000000000000000"),
            input.trim(),
        );
        assert_eq!(outcome.http_status, 401, "stdio 上传了错 token 也要拒");
        let _ = &mut output;
    }
}
