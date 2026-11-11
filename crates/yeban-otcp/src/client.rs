//! OTCP JSON-RPC 2.0 客户端：`TcpStream` + 换行帧 + id 匹配。
//!
//! 协议要点（spec `PROTOCOL.md` §3–§4）：
//! * 连接后 2000 ms 内必须发 `session.hello`；
//! * 每个请求 `{jsonrpc: "2.0", id, method, params}`，一行一个 JSON；
//! * 服务端回 `{jsonrpc: "2.0", id, result}` 或 `{id, error: {code, message, data}}`；
//! * `id` 从 1 递增，响应必须回显同一 `id`；
//! * ⛔ 连接只打字面量 `127.0.0.1:<port>`，⛔ 不解析 `localhost`；
//! * ⛔ token / serverProof 只出现在 `session.hello` 的参数里，
//!   永不进日志、错误详情与收据（见 [`redact`] 的单元测试）。

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::{Value, json};

use crate::error::{OtcpError, OtcpRpcError};

/// 对日志与错误文本做脱敏：`token` / `serverProof` 字段值一律打码。
///
/// 口径：任何含这两个键的 JSON 对象，值换成 `"[REDACTED]"`；
/// 其它键原样保留。单元测试（T9）钉死：脱敏后文本里不含原值。
#[must_use]
pub fn redact(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, val) in map {
                if key == "token" || key == "serverProof" {
                    out.insert(key.clone(), Value::String("[REDACTED]".to_owned()));
                } else {
                    out.insert(key.clone(), redact(val));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
        other => other.clone(),
    }
}

/// OTCP 客户端（单连接，单线程顺序调用）。
pub struct OtcpClient {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    next_id: u64,
}

impl OtcpClient {
    /// 连接到字面量 `127.0.0.1:<port>`，读超时按 `timeout` 设。
    pub fn connect(port: u16, timeout: Duration) -> Result<Self, OtcpError> {
        let addr = format!("127.0.0.1:{port}");
        let stream = TcpStream::connect(&addr)
            .map_err(|error| OtcpError::transport(format!("连接 {addr} 失败: {error}")))?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|error| OtcpError::transport(format!("设读超时失败: {error}")))?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|error| OtcpError::transport(format!("设写超时失败: {error}")))?;
        let reader = BufReader::new(
            stream
                .try_clone()
                .map_err(|error| OtcpError::transport(format!("克隆连接失败: {error}")))?,
        );
        Ok(Self {
            stream,
            reader,
            next_id: 1,
        })
    }

    /// 发一个请求并等响应；返回 `result`。
    ///
    /// 服务端返回 `error` 时按 [`OtcpError::from_otcp`] 映射后返回 `Err`。
    /// `job_id` 只用于错误上下文（可为空串）。
    pub fn call(&mut self, method: &str, params: Value, job_id: &str) -> Result<Value, OtcpError> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_string(&request)
            .map_err(|error| OtcpError::protocol(format!("序列化 {method} 请求失败: {error}")))?;
        line.push('\n');
        self.stream
            .write_all(line.as_bytes())
            .map_err(|error| OtcpError::transport(format!("发 {method} 失败: {error}")))?;
        self.stream
            .flush()
            .map_err(|error| OtcpError::transport(format!("发 {method} 失败: {error}")))?;
        let mut response_line = String::new();
        self.reader
            .read_line(&mut response_line)
            .map_err(|error| OtcpError::transport(format!("读 {method} 响应失败: {error}")))?;
        if response_line.trim().is_empty() {
            return Err(OtcpError::transport(format!(
                "{method} 响应为空（连接已断）"
            )));
        }
        let response: Value = serde_json::from_str(&response_line)
            .map_err(|error| OtcpError::protocol(format!("解析 {method} 响应失败: {error}")))?;
        let response_id = response.get("id").and_then(Value::as_u64);
        if response_id != Some(id) {
            return Err(OtcpError::protocol(format!(
                "{method} 响应 id 不符：期望 {id}，实得 {response_id:?}"
            )));
        }
        if let Some(error) = response.get("error") {
            let rpc = OtcpRpcError::from_value(error).ok_or_else(|| {
                OtcpError::protocol(format!("{method} 错误对象形状非法: {}", redact(error)))
            })?;
            return Err(rpc.to_driver_error(job_id));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| OtcpError::protocol(format!("{method} 响应缺 result")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_strips_token_and_server_proof_everywhere() {
        let value = json!({
            "token": "secret-token-123",
            "serverProof": "proof-abc",
            "nested": {"token": "inner-secret", "keep": 1},
            "list": [{"serverProof": "list-secret"}],
            "instanceId": "not-secret"
        });
        let redacted = redact(&value);
        let text = serde_json::to_string(&redacted).expect("序列化");
        assert!(!text.contains("secret-token-123"));
        assert!(!text.contains("proof-abc"));
        assert!(!text.contains("inner-secret"));
        assert!(!text.contains("list-secret"));
        assert!(text.contains("not-secret"));
        assert!(text.contains("[REDACTED]"));
    }

    #[test]
    fn client_roundtrips_against_a_local_tcp_server() {
        use std::io::Write as _;
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定");
        let port = listener.local_addr().expect("取端口").port();
        let handle = thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("接受连接");
            let mut reader = BufReader::new(socket.try_clone().expect("克隆"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("读请求");
            let request: Value = serde_json::from_str(&line).expect("解析请求");
            assert_eq!(request["method"], "session.info");
            assert_eq!(request["id"], 1);
            let response = json!({"jsonrpc": "2.0", "id": 1, "result": {"ok": true}});
            socket
                .write_all(serde_json::to_string(&response).expect("序列化").as_bytes())
                .expect("回写");
            socket.write_all(b"\n").expect("回写换行");
        });
        let mut client = OtcpClient::connect(port, Duration::from_secs(5)).expect("连接");
        let result = client.call("session.info", json!({}), "").expect("调用");
        assert_eq!(result["ok"], true);
        handle.join().expect("服务端线程");
    }

    #[test]
    fn server_error_maps_to_driver_error() {
        use std::io::Write as _;
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定");
        let port = listener.local_addr().expect("取端口").port();
        let handle = thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("接受连接");
            let mut reader = BufReader::new(socket.try_clone().expect("克隆"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("读请求");
            let response = json!({
                "jsonrpc": "2.0", "id": 1,
                "error": {
                    "code": -32000, "message": "NOT_READY",
                    "data": {
                        "kind": "NOT_READY", "reason": "f0_not_ready",
                        "retryable": true, "detail": "f0 not ready", "executed": false
                    }
                }
            });
            socket
                .write_all(serde_json::to_string(&response).expect("序列化").as_bytes())
                .expect("回写");
            socket.write_all(b"\n").expect("回写换行");
        });
        let mut client = OtcpClient::connect(port, Duration::from_secs(5)).expect("连接");
        let error = client
            .call("content.getNotes", json!({}), "job-x")
            .expect_err("应返回 NOT_READY");
        assert_eq!(error.reason, "otcpNotReady");
        handle.join().expect("服务端线程");
    }
}
