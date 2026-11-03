//! `synthesize_singing` / `svs_job` —— 伶人 SVS 服务的 MCP 客户端工具。
//!
//! ## 协议（唯一事实源 = 服务侧 Phase 1 的 `svs_client.py` 逻辑）
//!
//! | 工具调用 | 服务请求 | 成功响应 |
//! | :--- | :--- | :--- |
//! | `synthesize_singing` | `POST /v1/synthesize {parts}` | `{job_id, status, parts_total, takes_total}` |
//! | `svs_job` `want=status` | `GET /v1/jobs/{id}` | 状态全文的子集（`error` 原文照带） |
//! | `svs_job` `want=receipt` | `GET /v1/jobs/{id}/receipt` | 回执全文 |
//! | `svs_job` `want=audio` | `GET /v1/jobs/{id}/audio?part=&take=` | wav 字节 |
//!
//! 本模块**不**把 Python 客户端抄进仓库：HTTP 层按同一语义用 `std::net`
//! 重写（零新增依赖 ⇒ `cargo deny` 的包集合不变）。重试/轮询语义**不**
//! 另发明：`svs_job` 每次调用只做一次查询，轮询由调用方重复调用实现
//! （与 `svs_client.py` 的 `wait` 是同一回事，只是循环在调用方手里）。
//!
//! ## 配置：绝不进工程文件
//!
//! 服务地址：工具实参 `service_url` > 环境变量 `YEBAN_SVS_URL` >
//! 缺省 `http://127.0.0.1:8000`。API key **只**从环境变量
//! `YEBAN_SVS_API_KEY` 读：它**不是**工具参数（`ToolCall` 校验会把
//! `apiKey` 当未知键拒掉），也**绝不**出现在任何响应、错误信息与日志里。
//!
//! ## 只支持 `http`（如实登记的边界）
//!
//! `std` 客户端没有 TLS：`https` 服务地址会在配置解析时就响亮拒绝
//! （`IO_ERROR` + `reason: "httpsNotSupported"`），⛔ 不静默降级、不假装
//! 连上。远端 tunnel 目前只能经本地 `http` 端点接入；TLS 支持是后续项
//! （要么给服务加本地 http 出口，要么批一个 TLS 依赖），见 `NOTES.md`。
//!
//! ## plan / apply 分工（与 `import_audio` 同款）
//!
//! 网络请求发生在 `plan` 里（`import_audio::plan` 同样在 `plan` 里读盘解码；
//! `render` 的渲染也在 `plan` 里）。`dryRun` 是唯一的例外：
//!
//! - `synthesize_singing` + `dryRun`：只做参数校验，**不**提交（`wouldSubmit` 预览）。
//! - `svs_job` `want=audio` + `dryRun` + 落轨：只取回执算摆放预览，
//!   **不**取 wav、**不**写暂存文件、**不**建导入计划。
//! - `svs_job` `status` / `receipt` 本就是只读查询，`dryRun` 与真调用逐字段相同。
//!
//! ## 失败语义（响亮，不静默空成功）
//!
//! - 服务不可达 / 超时 / 非 2xx / 坏 JSON ⇒ `IO_ERROR`（带 `reason` 与状态码）。
//! - 未知任务 / 未知 part / 未知 take ⇒ `ENTITY_NOT_FOUND`。
//! - 任务已失败 ⇒ `IO_ERROR` + `reason: "svsJobFailed"`，服务端的 `error`
//!   原文照带（门耗尽 `takes gate exhausted` 同属这一条，⛔ 不返回静音 wav）。
//! - 任务未完成时取音频 ⇒ `IO_ERROR` + `reason: "jobNotCompleted"`；
//!   取回执 ⇒ `IO_ERROR` + `reason: "receiptNotReady"`。
//! - 任务失败**不**返回静音 wav、**不**返回空音频冒充成功（取音频前先查状态）。

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Map, Value};
use yeban_model::PPQ;

use super::error::Fault;
use crate::tools::{ErrorCode, ToolCall};

/// 本地缺省服务地址（规格：本地缺省 `http://127.0.0.1:8000`）。
pub const DEFAULT_SERVICE_URL: &str = "http://127.0.0.1:8000";

/// 服务地址的环境变量名。
pub const ENV_SERVICE_URL: &str = "YEBAN_SVS_URL";

/// API key 的环境变量名（唯一来源；⛔ 不是工具参数）。
pub const ENV_API_KEY: &str = "YEBAN_SVS_API_KEY";

/// 实测偏差超过该阈值（毫秒）时进 `deviationWarnings`（SoulX 与 VR 同阈值）。
///
/// 阈值是**工具侧的报告口径**，不是服务侧的门：服务侧的 takes 门
/// （包络相关 / F0 差 / 起音差）与本阈值无关；本阈值只决定\"哪条偏差
/// 值得在响应里单独标出来\"。
pub const DEVIATION_WARN_MS: f64 = 100.0;

/// 取回 wav 的本地暂存目录名（`std::env::temp_dir()` 之下）。
///
/// 路径是**稳定**的：`{job_id}_{part}_take{take}.wav`（part 经文件名净化），
/// 因此重复取同一遍 ⇒ 同一路径 ⇒ `import_audio` 的资产索引留痕
/// （`originalPath`）逐字段相同 ⇒ 幂等命中；若每次换路径，
/// 第二次导入会因索引项不同而 `CONFLICT`（`assetDeclaredDifferently`）。
pub const CACHE_DIR_NAME: &str = "yeban-svs";

/// JSON 请求的读写超时（与 `svs_client.py` 的 120 秒同口径）。
const JSON_TIMEOUT: Duration = Duration::from_secs(120);

/// 音频字节的读写超时（与 `svs_client.py` 的 300 秒同口径）。
const AUDIO_TIMEOUT: Duration = Duration::from_secs(300);

/// 连接超时。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// 解析后的服务配置（地址 + 可选 API key）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SvsConfig {
    /// 去掉末尾 `/` 的 base URL（`http://host[:port][/prefix]`）。
    pub base_url: String,
    /// `YEBAN_SVS_API_KEY`（空串按未提供处理）。
    pub api_key: Option<String>,
}

/// 解析服务配置：实参 > 环境变量 > 缺省。
///
/// # Errors
///
/// 地址不是 `http`（含 `https`）⇒ `IO_ERROR`（`httpsNotSupported`，
/// 见模块头的边界说明）；地址无 host ⇒ `INVALID_PARAMETER_RANGE`。
pub fn resolve_config(service_url_arg: Option<&str>) -> Result<SvsConfig, Fault> {
    let raw = match service_url_arg {
        Some(url) if !url.is_empty() => url.to_owned(),
        _ => match std::env::var(ENV_SERVICE_URL) {
            Ok(url) if !url.is_empty() => url,
            _ => DEFAULT_SERVICE_URL.to_owned(),
        },
    };
    let base_url = raw.trim_end_matches('/').to_owned();
    let (scheme, _) = base_url.split_once("://").ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`service_url` 必须是 http(s) 地址: {raw}"),
            serde_json::json!({ "field": "service_url", "reason": "serviceUrlNotHttp" }),
        )
    })?;
    if scheme != "http" {
        return Err(Fault::domain_with_data(
            ErrorCode::IoError,
            format!(
                "SVS 服务地址只支持 http（std 客户端无 TLS）：{base_url}；\
                 请给服务配一个本地 http 端点，或等 TLS 依赖获批后再用 https"
            ),
            serde_json::json!({ "field": "service_url", "reason": "httpsNotSupported" }),
        ));
    }
    let api_key = match std::env::var(ENV_API_KEY) {
        Ok(key) if !key.is_empty() => Some(key),
        _ => None,
    };
    Ok(SvsConfig { base_url, api_key })
}

/// `http://host[:port][/prefix]` 的解析结果。
#[derive(Clone, Debug, PartialEq, Eq)]
struct HttpTarget {
    /// `host` 或 `[ipv6]`（含括号，供 `TcpStream::connect` 直接用）。
    host: String,
    port: u16,
    /// 路径前缀（不含末尾 `/`；无前缀为空串）。
    prefix: String,
    /// `Host` 请求头值（`host` 或 `host:port`）。
    host_header: String,
}

/// 解析 `http` 地址（`resolve_config` 已保证 scheme 为 `http`）。
///
/// # Errors
///
/// 无 host / 端口非数字 ⇒ `INVALID_PARAMETER_RANGE`。
fn parse_http_target(base_url: &str) -> Result<HttpTarget, Fault> {
    let rest = base_url.strip_prefix("http://").unwrap_or(base_url);
    let (authority, prefix) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, String::new()),
    };
    if authority.is_empty() {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`service_url` 没有 host: {base_url}"),
            serde_json::json!({ "field": "service_url", "reason": "serviceUrlNoHost" }),
        ));
    }
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        // `[ipv6]` 或 `[ipv6]:port`。
        match bracketed.split_once("]:") {
            Some((inner, port_text)) => (format!("[{inner}]"), parse_port(port_text, base_url)?),
            None => (authority.to_owned(), 80),
        }
    } else {
        match authority.rsplit_once(':') {
            Some((host, port_text)) => {
                // 冒号后有东西：它只能是端口；非数字或空都不能静默当成 host 的一部分。
                if port_text.is_empty() || !port_text.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(Fault::domain_with_data(
                        ErrorCode::InvalidParameterRange,
                        format!("`service_url` 的端口非法: {base_url}"),
                        serde_json::json!({ "field": "service_url", "reason": "serviceUrlBadPort" }),
                    ));
                }
                (host.to_owned(), parse_port(port_text, base_url)?)
            }
            None => (authority.to_owned(), 80),
        }
    };
    let host_header = if port == 80 {
        host.clone()
    } else {
        format!("{host}:{port}")
    };
    Ok(HttpTarget {
        host,
        port,
        prefix: prefix.trim_end_matches('/').to_owned(),
        host_header,
    })
}

/// 解析端口文本（1..=65535）。
fn parse_port(text: &str, base_url: &str) -> Result<u16, Fault> {
    text.parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| {
            Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!("`service_url` 的端口非法: {base_url}"),
                serde_json::json!({ "field": "service_url", "reason": "serviceUrlBadPort" }),
            )
        })
}

/// 百分号编码（查询参数值；保留 `A-Z a-z 0-9 . _ ~ -`）。
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'~' | b'-') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// 文件名净化（暂存路径用；`part` 是用户侧 label，可能含 `/` 等字符）。
fn sanitize_file_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

/// 取回 wav 的稳定暂存路径（见 [`CACHE_DIR_NAME`] 的幂等说明）。
#[must_use]
pub fn wav_cache_path(job_id: &str, part: &str, take: u64) -> PathBuf {
    std::env::temp_dir().join(CACHE_DIR_NAME).join(format!(
        "{}_{}_take{take}.wav",
        job_id,
        sanitize_file_component(part)
    ))
}

/// 最小 HTTP/1.1 客户端（`std::net`；`Connection: close` + 读到 EOF）。
///
/// 响应体读取：先读头（`\r\n\r\n` 截止），再读到连接关闭；若响应头声明
/// `Transfer-Encoding: chunked`，对体做分块解码；若声明 `Content-Length`，
/// 体截断到该长度（防御多余字节）。API key 只进请求头，⛔ 不进任何错误。
pub struct SvsClient {
    base_url: String,
    target: HttpTarget,
    api_key: Option<String>,
}

impl SvsClient {
    /// 由已解析配置建客户端。
    ///
    /// # Errors
    ///
    /// 地址解析失败（无 host / 端口非法）⇒ `INVALID_PARAMETER_RANGE`。
    pub fn new(config: &SvsConfig) -> Result<Self, Fault> {
        Ok(Self {
            base_url: config.base_url.clone(),
            target: parse_http_target(&config.base_url)?,
            api_key: config.api_key.clone(),
        })
    }

    /// 供响应回显的服务地址（不含任何凭据）。
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// `POST /v1/synthesize`：提交 parts，返回服务端原样回执。
    ///
    /// # Errors
    ///
    /// 不可达 / 超时 / 非 2xx / 坏 JSON ⇒ `IO_ERROR`（见模块头失败语义）。
    pub fn synthesize(&self, parts: &Value) -> Result<Value, Fault> {
        let body = serde_json::to_vec(&serde_json::json!({ "parts": parts })).map_err(|error| {
            Fault::domain_with_data(
                ErrorCode::IoError,
                format!("提交载荷序列化失败: {error}"),
                serde_json::json!({ "reason": "svsRequestEncodeFailed" }),
            )
        })?;
        let response = self.request("POST", "/v1/synthesize", Some(&body), JSON_TIMEOUT)?;
        self.expect_json(response, "提交歌声合成任务")
    }

    /// `GET /v1/jobs/{id}`：返回服务端状态全文。
    ///
    /// # Errors
    ///
    /// 404 ⇒ `ENTITY_NOT_FOUND`（未知任务）；其余失败见 [`Self::synthesize`]。
    pub fn job_status(&self, job_id: &str) -> Result<Value, Fault> {
        let response = self.request("GET", &format!("/v1/jobs/{job_id}"), None, JSON_TIMEOUT)?;
        if response.status == 404 {
            return Err(Fault::domain_with_data(
                ErrorCode::EntityNotFound,
                format!("SVS 服务上没有任务 {job_id}"),
                serde_json::json!({ "reason": "svsJobNotFound", "job_id": job_id }),
            ));
        }
        self.expect_json(response, "查询 SVS 任务状态")
    }

    /// `GET /v1/jobs/{id}/receipt`：返回回执全文。
    ///
    /// # Errors
    ///
    /// 404 ⇒ `ENTITY_NOT_FOUND`；409（任务未完、回执未写）⇒ `IO_ERROR`
    /// + `reason: "receiptNotReady"`；其余失败见 [`Self::synthesize`]。
    pub fn job_receipt(&self, job_id: &str) -> Result<Value, Fault> {
        let response = self.request(
            "GET",
            &format!("/v1/jobs/{job_id}/receipt"),
            None,
            JSON_TIMEOUT,
        )?;
        if response.status == 404 {
            return Err(Fault::domain_with_data(
                ErrorCode::EntityNotFound,
                format!("SVS 服务上没有任务 {job_id}"),
                serde_json::json!({ "reason": "svsJobNotFound", "job_id": job_id }),
            ));
        }
        if response.status == 409 {
            return Err(Fault::domain_with_data(
                ErrorCode::IoError,
                format!("任务 {job_id} 尚未完成，回执未写出（先用 want=status 轮询）"),
                serde_json::json!({
                    "reason": "receiptNotReady",
                    "job_id": job_id,
                    "detail": excerpt(&response.body),
                }),
            ));
        }
        self.expect_json(response, "取 SVS 任务回执")
    }

    /// `GET /v1/jobs/{id}/audio?part=&take=`：返回 wav 字节。
    ///
    /// # Errors
    ///
    /// 404 ⇒ `IO_ERROR` + `reason: "svsAudioNotFound"`（调用方取音频前已查
    /// 状态，因此这里的 404 是\"任务已完成但该遍音频缺失\"的不一致态）；
    /// 其余失败见 [`Self::synthesize`]。
    pub fn job_audio(&self, job_id: &str, part: &str, take: u64) -> Result<Vec<u8>, Fault> {
        let path = format!(
            "/v1/jobs/{job_id}/audio?part={}&take={take}",
            percent_encode(part)
        );
        let response = self.request("GET", &path, None, AUDIO_TIMEOUT)?;
        if response.status == 404 {
            return Err(Fault::domain_with_data(
                ErrorCode::IoError,
                format!("任务 {job_id} 的音频缺失（part={part} take={take}）"),
                serde_json::json!({
                    "reason": "svsAudioNotFound",
                    "job_id": job_id,
                    "part": part,
                    "take": take,
                }),
            ));
        }
        if !(200..300).contains(&response.status) {
            return Err(http_fault(&response, "取 SVS 音频"));
        }
        Ok(response.body)
    }

    /// 一次原始请求（方法 + 路径 + 可选 JSON 体 + 超时）。
    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<HttpResponse, Fault> {
        let full_path = format!("{}{path}", self.target.prefix);
        let addr = format!("{}:{}", self.target.host, self.target.port);
        let mut resolved = addr
            .to_socket_addrs()
            .map_err(|error| io_fault("解析服务地址", &error))?;
        let socket_addr = resolved.next().ok_or_else(|| {
            Fault::domain_with_data(
                ErrorCode::IoError,
                format!("SVS 服务地址解析不到任何地址: {addr}"),
                serde_json::json!({ "reason": "serviceUnreachable" }),
            )
        })?;
        let mut stream = TcpStream::connect_timeout(&socket_addr, CONNECT_TIMEOUT)
            .map_err(|error| io_fault("连接 SVS 服务", &error))?;
        stream
            .set_read_timeout(Some(timeout))
            .and_then(|()| stream.set_write_timeout(Some(timeout)))
            .map_err(|error| io_fault("设置 SVS 连接超时", &error))?;
        let mut request = format!(
            "{method} {full_path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
            self.target.host_header
        );
        if let Some(key) = &self.api_key {
            request.push_str(&format!("Authorization: Bearer {key}\r\n"));
        }
        if let Some(body) = body {
            request.push_str(&format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                body.len()
            ));
        } else {
            request.push_str("\r\n");
        }
        stream
            .write_all(request.as_bytes())
            .and_then(|()| body.map_or(Ok(()), |body| stream.write_all(body)))
            .map_err(|error| io_fault("发送 SVS 请求", &error))?;
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .map_err(|error| io_fault("读取 SVS 响应", &error))?;
        parse_response(&raw)
    }

    /// 2xx 响应体解析为 JSON；非 2xx ⇒ `serviceHttp`，坏 JSON ⇒ `serviceBadJson`。
    fn expect_json(&self, response: HttpResponse, context: &str) -> Result<Value, Fault> {
        if !(200..300).contains(&response.status) {
            return Err(http_fault(&response, context));
        }
        serde_json::from_slice(&response.body).map_err(|error| {
            Fault::domain_with_data(
                ErrorCode::IoError,
                format!("{context}：服务端返回的不是合法 JSON: {error}"),
                serde_json::json!({ "reason": "serviceBadJson" }),
            )
        })
    }
}

/// 原始 HTTP 响应（状态码 + 解码后的体）。
struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

/// 解析原始响应字节：状态行 + 头 + 体（分块解码 / 长度截断）。
fn parse_response(raw: &[u8]) -> Result<HttpResponse, Fault> {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            Fault::domain_with_data(
                ErrorCode::IoError,
                "SVS 服务返回的响应没有完整的头",
                serde_json::json!({ "reason": "serviceBadResponse" }),
            )
        })?;
    let (head, body) = (&raw[..split], &raw[split + 4..]);
    let head_text = String::from_utf8_lossy(head);
    let mut lines = head_text.lines();
    let status_line = lines.next().unwrap_or("");
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| {
            Fault::domain_with_data(
                ErrorCode::IoError,
                format!("SVS 服务返回的状态行无法解析: {status_line}"),
                serde_json::json!({ "reason": "serviceBadResponse" }),
            )
        })?;
    let mut chunked = false;
    let mut content_length: Option<usize> = None;
    for line in lines {
        let (name, value) = match line.split_once(':') {
            Some((name, value)) => (name.trim().to_ascii_lowercase(), value.trim()),
            None => continue,
        };
        if name == "transfer-encoding" && value.to_ascii_lowercase().contains("chunked") {
            chunked = true;
        }
        if name == "content-length" {
            content_length = value.parse().ok();
        }
    }
    let body = if chunked {
        decode_chunked(body)?
    } else if let Some(length) = content_length {
        body[..body.len().min(length)].to_vec()
    } else {
        body.to_vec()
    };
    Ok(HttpResponse { status, body })
}

/// 分块传输解码（忽略块扩展与 trailer）。
fn decode_chunked(mut body: &[u8]) -> Result<Vec<u8>, Fault> {
    let mut out = Vec::new();
    loop {
        let line_end = body
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| {
                Fault::domain_with_data(
                    ErrorCode::IoError,
                    "SVS 服务返回的分块体截断（无块大小行）",
                    serde_json::json!({ "reason": "serviceBadResponse" }),
                )
            })?;
        let size_text = String::from_utf8_lossy(&body[..line_end]);
        let size_text = size_text.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| {
            Fault::domain_with_data(
                ErrorCode::IoError,
                format!("SVS 服务返回的分块大小无法解析: {size_text}"),
                serde_json::json!({ "reason": "serviceBadResponse" }),
            )
        })?;
        body = &body[line_end + 2..];
        if size == 0 {
            break;
        }
        if body.len() < size {
            return Err(Fault::domain_with_data(
                ErrorCode::IoError,
                "SVS 服务返回的分块体截断（块数据不足）",
                serde_json::json!({ "reason": "serviceBadResponse" }),
            ));
        }
        out.extend_from_slice(&body[..size]);
        body = &body[size..];
        // 块后必有一行 `\r\n`（缺了也按截断处理）。
        body = body.strip_prefix(b"\r\n").ok_or_else(|| {
            Fault::domain_with_data(
                ErrorCode::IoError,
                "SVS 服务返回的分块体截断（块后无换行）",
                serde_json::json!({ "reason": "serviceBadResponse" }),
            )
        })?;
    }
    Ok(out)
}

/// `std::io::Error` ⇒ `IO_ERROR`（`reason: "serviceUnreachable"` 族；⛔ 不带凭据）。
fn io_fault(context: &str, error: &std::io::Error) -> Fault {
    use std::io::ErrorKind::{ConnectionRefused, NotFound, TimedOut};
    let reason = match error.kind() {
        TimedOut => "serviceTimeout",
        ConnectionRefused | NotFound => "serviceUnreachable",
        _ => "serviceIoError",
    };
    Fault::domain_with_data(
        ErrorCode::IoError,
        format!("{context}失败：{error}"),
        serde_json::json!({ "reason": reason }),
    )
}

/// 非 2xx 响应 ⇒ `IO_ERROR`（状态码 + 体前 500 字节摘要，与 `svs_client.py` 同口径）。
fn http_fault(response: &HttpResponse, context: &str) -> Fault {
    Fault::domain_with_data(
        ErrorCode::IoError,
        format!(
            "{context}：服务返回 HTTP {}：{}",
            response.status,
            excerpt(&response.body)
        ),
        serde_json::json!({
            "reason": "serviceHttp",
            "httpStatus": response.status,
        }),
    )
}

/// 体摘要（前 500 字节，`String::from_utf8_lossy`；⛔ 响应体里没有凭据可带）。
fn excerpt(body: &[u8]) -> String {
    String::from_utf8_lossy(&body[..body.len().min(500)]).into_owned()
}

// ===========================================================================
// 参数校验与放置语义
// ===========================================================================

use std::str::FromStr as _;

use yeban_model::EntityId;

use super::ids::deterministic_id;
use super::import_audio;
use super::{Domain, Plan};
use crate::tools::ToolResponse;

/// `svs_job` 的 `want` 取值（缺省 `status`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SvsWant {
    /// 查状态（`GET /v1/jobs/{id}`）。
    Status,
    /// 取一遍 wav（`GET /v1/jobs/{id}/audio?part=&take=`）。
    Audio,
    /// 取回执全文（`GET /v1/jobs/{id}/receipt`）。
    Receipt,
}

/// `svs_job` 的已解析实参（`ToolCall` 只校验了浅层类型；语义规则在这里）。
pub struct SvsJobArgs {
    /// 任务 id（字母 / 数字 / `_` / `-`，1..=128 字符）。
    pub job_id: String,
    /// 要做什么（缺省 `status`）。
    pub want: SvsWant,
    /// part label（`want=audio` 时必填）。
    pub part: Option<String>,
    /// 遍号（`want=audio` 时必填，0 起）。
    pub take: Option<u64>,
    /// 落轨目标音轨（给了才落轨；不给只取回）。
    pub track_id: Option<EntityId>,
    /// 落轨时乐谱起点 tick（缺省 0；只在给了 `trackId` 时有效）。
    pub start_tick: Option<u64>,
    /// 落轨时资产许可标识（落轨时必填，须在白名单内）。
    pub license: Option<String>,
    /// 落轨时片段增益（dB；只在给了 `trackId` 时有效）。
    pub gain_db: Option<f64>,
    /// 服务地址覆盖（缺省走配置解析：环境变量 > 缺省地址）。
    pub service_url: Option<String>,
}

/// 参数错误（`INVALID_PARAMETER_RANGE` + `field` + `reason`，绝不静默丢弃）。
fn param_fault(field: &str, reason: &str, message: String) -> Fault {
    Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        message,
        serde_json::json!({ "field": field, "reason": reason }),
    )
}

/// 取必填字符串（缺键 / 非字符串 / 空串 ⇒ 参数错误）。
fn req_string(args: &Map<String, Value>, field: &str) -> Result<String, Fault> {
    match args.get(field) {
        Some(Value::String(text)) if !text.is_empty() => Ok(text.clone()),
        Some(Value::String(_)) => Err(param_fault(
            field,
            "valueMustBeNonEmptyString",
            format!("`{field}` 必须是非空字符串"),
        )),
        Some(_) => Err(param_fault(
            field,
            "valueMustBeString",
            format!("`{field}` 必须是字符串"),
        )),
        None => Err(param_fault(
            field,
            "missingField",
            format!("`{field}` 必填"),
        )),
    }
}

/// 取可选字符串（`null` 按未提供处理；非字符串 ⇒ 参数错误）。
fn opt_string(args: &Map<String, Value>, field: &str) -> Result<Option<String>, Fault> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(param_fault(
            field,
            "valueMustBeString",
            format!("`{field}` 必须是字符串"),
        )),
    }
}

/// 取可选非负整数（`ToolCall` 的 `integer` 接受负数 ⇒ 这里再拦一道）。
fn opt_non_negative_u64(args: &Map<String, Value>, field: &str) -> Result<Option<u64>, Fault> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => match value.as_u64() {
            Some(number) => Ok(Some(number)),
            None => Err(param_fault(
                field,
                "valueMustBeNonNegativeInteger",
                format!("`{field}` 必须是非负整数"),
            )),
        },
    }
}

/// 取可选有限数字。
fn opt_finite_f64(args: &Map<String, Value>, field: &str) -> Result<Option<f64>, Fault> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => match value.as_f64() {
            Some(number) if number.is_finite() => Ok(Some(number)),
            _ => Err(param_fault(
                field,
                "valueMustBeFiniteNumber",
                format!("`{field}` 必须是有限数字"),
            )),
        },
    }
}

/// 解析 `svs_job` 实参（含跨键规则：`want` 与摆放实参的搭配）。
///
/// # Errors
///
/// 缺键 / 类型错 / 取值越界 / 搭配非法 ⇒ `INVALID_PARAMETER_RANGE`。
pub fn parse_svs_job_args(call: &ToolCall) -> Result<SvsJobArgs, Fault> {
    let args = &call.arguments;
    let job_id = req_string(args, "job_id")?;
    if job_id.len() > 128
        || !job_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(param_fault(
            "job_id",
            "jobIdInvalid",
            "`job_id` 只接受 1..=128 个字母 / 数字 / `_` / `-`（服务端生成的 \
             16 位十六进制在此集合内；收窄是为防止它进 URL 路径与暂存文件名时越界）"
                .to_owned(),
        ));
    }
    let want = match args.get("want") {
        None | Some(Value::Null) => SvsWant::Status,
        Some(Value::String(text)) => match text.as_str() {
            "status" => SvsWant::Status,
            "audio" => SvsWant::Audio,
            "receipt" => SvsWant::Receipt,
            _ => {
                return Err(param_fault(
                    "want",
                    "wantInvalid",
                    "`want` 只认 status / audio / receipt".to_owned(),
                ));
            }
        },
        Some(_) => {
            return Err(param_fault(
                "want",
                "valueMustBeString",
                "`want` 必须是字符串".to_owned(),
            ));
        }
    };
    let part = opt_string(args, "part")?.filter(|text| !text.is_empty());
    if want != SvsWant::Audio && part.is_some() {
        return Err(param_fault(
            "part",
            "audioOnlyParameter",
            "`part` 只在 want=audio 时有效".to_owned(),
        ));
    }
    let take = opt_non_negative_u64(args, "take")?;
    if want != SvsWant::Audio && take.is_some() {
        return Err(param_fault(
            "take",
            "audioOnlyParameter",
            "`take` 只在 want=audio 时有效".to_owned(),
        ));
    }
    if want == SvsWant::Audio {
        if part.is_none() {
            return Err(param_fault(
                "part",
                "missingField",
                "want=audio 时 `part` 必填（服务端允许多遍歧义回落，但本工具⛔ \
                 不替调用方猜是哪一遍）"
                    .to_owned(),
            ));
        }
        if take.is_none() {
            return Err(param_fault(
                "take",
                "missingField",
                "want=audio 时 `take` 必填".to_owned(),
            ));
        }
    }
    let track_id = match opt_string(args, "trackId")? {
        None => None,
        Some(text) => Some(EntityId::from_str(&text).map_err(|error| {
            param_fault(
                "trackId",
                "valueMustBeUlid",
                format!("`trackId` 必须是 26 字符 ULID 字符串: {error}"),
            )
        })?),
    };
    let start_tick = opt_non_negative_u64(args, "startTick")?;
    let license = opt_string(args, "license")?.filter(|text| !text.is_empty());
    let gain_db = opt_finite_f64(args, "gainDb")?;
    // 摆放实参没有目标音轨 ⇒ 响亮拒绝（与 `import_audio` 的
    // `placementWithoutTrack` 同一条纪律：⛔ 不静默忽略）。
    if track_id.is_none() {
        for (field, present) in [
            ("startTick", start_tick.is_some()),
            ("license", license.is_some()),
            ("gainDb", gain_db.is_some()),
        ] {
            if present {
                return Err(param_fault(
                    field,
                    "placementWithoutTrack",
                    format!("给了 `{field}` 却没有给 `trackId` —— 没有目标音轨就无处摆放"),
                ));
            }
        }
    }
    // 落轨时许可必填（`import_audio` 的 MUST-GATE-014 规则；这里先拦一道，
    // 免得取完音频、算完摆放才在导入计划里被拒）。
    if track_id.is_some() && license.is_none() {
        return Err(param_fault(
            "license",
            "licenseRequired",
            "落轨时 `license` 必填（资产许可标识，须在 \
             assets/samples/manifest.json 白名单内；本工具不发明默认值）"
                .to_owned(),
        ));
    }
    Ok(SvsJobArgs {
        job_id,
        want,
        part,
        take,
        track_id,
        start_tick,
        license,
        gain_db,
        service_url: opt_string(args, "service_url")?,
    })
}

/// 校验 `synthesize_singing` 的 `parts`（深层形状；`ToolCall` 只验了 `array`）。
///
/// 服务端是 part 形状的权威（未知 `model` 由服务端响亮拒绝，⛔ 本层不另抄
/// 一份模型白名单 —— 抄了就会在服务端加模型时漂移）。本层只验**形状**：
/// 键集合、必填、类型、数值范围。
///
/// # Errors
///
/// 形状非法 ⇒ `INVALID_PARAMETER_RANGE`（带 `field` 与 `reason`）。
///
/// 返回 `(parts_total, takes_total)`（`takes` 缺省 1）。
pub fn validate_parts(parts: &Value) -> Result<(usize, usize), Fault> {
    let items = parts
        .as_array()
        .ok_or_else(|| param_fault("parts", "valueMustBeArray", "`parts` 必须是数组".to_owned()))?;
    if items.is_empty() {
        return Err(param_fault(
            "parts",
            "partsEmpty",
            "`parts` 至少要有一项".to_owned(),
        ));
    }
    let mut takes_total = 0usize;
    for (index, item) in items.iter().enumerate() {
        takes_total += validate_part(item, index)?;
    }
    Ok((items.len(), takes_total))
}

/// 校验一个 part 对象，返回它的遍数（`takes` 缺省 1）。
fn validate_part(item: &Value, index: usize) -> Result<usize, Fault> {
    let part = item.as_object().ok_or_else(|| {
        param_fault(
            "parts",
            "partMustBeObject",
            format!("`parts[{index}]` 必须是对象"),
        )
    })?;
    for key in part.keys() {
        if !matches!(
            key.as_str(),
            "label" | "score" | "voice_id" | "model" | "variant" | "takes" | "seeds" | "options"
        ) {
            return Err(param_fault(
                "parts",
                "unknownPartField",
                format!("`parts[{index}]` 有未知键 `{key}`（⛔ 不静默丢弃）"),
            ));
        }
    }
    let label = part
        .get("label")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| {
            param_fault(
                "parts",
                "partLabelRequired",
                format!("`parts[{index}].label` 必须是非空字符串"),
            )
        })?;
    let _ = label;
    part.get("voice_id")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| {
            param_fault(
                "parts",
                "partVoiceRequired",
                format!("`parts[{index}].voice_id` 必须是非空字符串"),
            )
        })?;
    if let Some(model) = part.get("model")
        && !matches!(model, Value::String(text) if !text.is_empty())
    {
        return Err(param_fault(
            "parts",
            "partModelInvalid",
            format!("`parts[{index}].model` 必须是非空字符串"),
        ));
    }
    if let Some(variant) = part.get("variant")
        && !matches!(variant, Value::Null | Value::String(_))
    {
        return Err(param_fault(
            "parts",
            "partVariantInvalid",
            format!("`parts[{index}].variant` 必须是字符串或 null"),
        ));
    }
    let takes = match part.get("takes") {
        None | Some(Value::Null) => 1usize,
        Some(value) => {
            let number = value.as_u64().ok_or_else(|| {
                param_fault(
                    "parts",
                    "partTakesInvalid",
                    format!("`parts[{index}].takes` 必须是 1..=8 的整数"),
                )
            })?;
            if !(1..=8).contains(&number) {
                return Err(param_fault(
                    "parts",
                    "partTakesInvalid",
                    format!("`parts[{index}].takes` 必须是 1..=8 的整数，实际 {number}"),
                ));
            }
            usize::try_from(number).unwrap_or(8)
        }
    };
    if let Some(seeds) = part.get("seeds") {
        let seeds = seeds.as_array().ok_or_else(|| {
            param_fault(
                "parts",
                "partSeedsInvalid",
                format!("`parts[{index}].seeds` 必须是整数数组"),
            )
        })?;
        for seed in seeds {
            if seed.as_i64().is_none() && seed.as_u64().is_none() {
                return Err(param_fault(
                    "parts",
                    "partSeedsInvalid",
                    format!("`parts[{index}].seeds` 里有非整数"),
                ));
            }
        }
    }
    if let Some(options) = part.get("options")
        && !options.is_object()
    {
        return Err(param_fault(
            "parts",
            "partOptionsInvalid",
            format!("`parts[{index}].options` 必须是对象"),
        ));
    }
    let score = part.get("score").ok_or_else(|| {
        param_fault(
            "parts",
            "partScoreRequired",
            format!("`parts[{index}].score` 必填"),
        )
    })?;
    validate_score(score, index)?;
    Ok(takes)
}

/// 校验一个 score 对象（`{bpm, language?, notes}`）。
fn validate_score(score: &Value, part_index: usize) -> Result<(), Fault> {
    let score = score.as_object().ok_or_else(|| {
        param_fault(
            "parts",
            "scoreMustBeObject",
            format!("`parts[{part_index}].score` 必须是对象"),
        )
    })?;
    for key in score.keys() {
        if !matches!(key.as_str(), "bpm" | "language" | "notes") {
            return Err(param_fault(
                "parts",
                "unknownScoreField",
                format!("`parts[{part_index}].score` 有未知键 `{key}`"),
            ));
        }
    }
    let bpm = score.get("bpm").and_then(Value::as_f64).ok_or_else(|| {
        param_fault(
            "parts",
            "scoreBpmRequired",
            format!("`parts[{part_index}].score.bpm` 必须是数字"),
        )
    })?;
    if !bpm.is_finite() || bpm <= 0.0 {
        return Err(param_fault(
            "parts",
            "scoreBpmInvalid",
            format!("`parts[{part_index}].score.bpm` 必须 > 0，实际 {bpm}"),
        ));
    }
    if let Some(language) = score.get("language")
        && !language.is_string()
    {
        return Err(param_fault(
            "parts",
            "scoreLanguageInvalid",
            format!("`parts[{part_index}].score.language` 必须是字符串"),
        ));
    }
    let notes = score
        .get("notes")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            param_fault(
                "parts",
                "scoreNotesRequired",
                format!("`parts[{part_index}].score.notes` 必须是数组"),
            )
        })?;
    for (note_index, note) in notes.iter().enumerate() {
        validate_note(note, part_index, note_index)?;
    }
    Ok(())
}

/// 校验一个音符（`{lyric, midi, start_s, dur_s, tie?, phoneme?}`）。
fn validate_note(note: &Value, part_index: usize, note_index: usize) -> Result<(), Fault> {
    let note = note.as_object().ok_or_else(|| {
        param_fault(
            "parts",
            "noteMustBeObject",
            format!("`parts[{part_index}].score.notes[{note_index}]` 必须是对象"),
        )
    })?;
    for key in note.keys() {
        if !matches!(
            key.as_str(),
            "lyric" | "midi" | "start_s" | "dur_s" | "tie" | "phoneme"
        ) {
            return Err(param_fault(
                "parts",
                "unknownNoteField",
                format!("`parts[{part_index}].score.notes[{note_index}]` 有未知键 `{key}`"),
            ));
        }
    }
    if !matches!(note.get("lyric"), Some(Value::String(_))) {
        return Err(param_fault(
            "parts",
            "noteLyricRequired",
            format!(
                "`parts[{part_index}].score.notes[{note_index}].lyric` 必须是字符串（'' 或 '<SP>' = 休止）"
            ),
        ));
    }
    let midi = note.get("midi").ok_or_else(|| {
        param_fault(
            "parts",
            "noteMidiRequired",
            format!("`parts[{part_index}].score.notes[{note_index}].midi` 必填"),
        )
    })?;
    if midi.as_i64().is_none() && midi.as_u64().is_none() {
        return Err(param_fault(
            "parts",
            "noteMidiInvalid",
            format!(
                "`parts[{part_index}].score.notes[{note_index}].midi` 必须是整数（<=0 = 休止）"
            ),
        ));
    }
    let start_s = note.get("start_s").and_then(Value::as_f64).ok_or_else(|| {
        param_fault(
            "parts",
            "noteStartRequired",
            format!("`parts[{part_index}].score.notes[{note_index}].start_s` 必须是数字"),
        )
    })?;
    if !start_s.is_finite() || start_s < 0.0 {
        return Err(param_fault(
            "parts",
            "noteStartInvalid",
            format!("`parts[{part_index}].score.notes[{note_index}].start_s` 必须 >= 0"),
        ));
    }
    let dur_s = note.get("dur_s").and_then(Value::as_f64).ok_or_else(|| {
        param_fault(
            "parts",
            "noteDurRequired",
            format!("`parts[{part_index}].score.notes[{note_index}].dur_s` 必须是数字"),
        )
    })?;
    if !dur_s.is_finite() || dur_s <= 0.0 {
        return Err(param_fault(
            "parts",
            "noteDurInvalid",
            format!("`parts[{part_index}].score.notes[{note_index}].dur_s` 必须 > 0"),
        ));
    }
    if let Some(tie) = note.get("tie")
        && !tie.is_boolean()
    {
        return Err(param_fault(
            "parts",
            "noteTieInvalid",
            format!("`parts[{part_index}].score.notes[{note_index}].tie` 必须是布尔值"),
        ));
    }
    if let Some(phoneme) = note.get("phoneme")
        && !matches!(phoneme, Value::Null | Value::String(_))
    {
        return Err(param_fault(
            "parts",
            "notePhonemeInvalid",
            format!("`parts[{part_index}].score.notes[{note_index}].phoneme` 必须是字符串或 null"),
        ));
    }
    Ok(())
}

// ===========================================================================
// 放置语义（纯函数：回执对齐数组 + 工程 BPM ⇒ 片段起点 tick）
// ===========================================================================

/// 放置语义的计算结果。
#[derive(Clone, Debug, PartialEq)]
pub struct PlacementOutcome {
    /// 片段起点 tick（已按模型语义算好，可直接交给 `import_audio` 的 `startTick`）。
    pub start_tick: u64,
    /// 锚点：`"score"`（乐谱时钟）或 `"measured"`（实测首音节）。
    pub anchor: &'static str,
    /// 首音节实际落点 tick（仅 `measured` 锚点有值；`score` 锚点为 `None`）。
    pub first_syllable_lands_at_tick: Option<u64>,
    /// 放置说明（进响应的 `placementNotes`）。
    pub notes: Vec<String>,
    /// 超阈值偏差条目（进响应的 `deviationWarnings`；休止 SP 已排除）。
    pub deviation_warnings: Vec<Value>,
}

/// 休止判定：回执对齐条目的 `lyric` 为 `""` 或 `"<SP>"` ⇒ 无真实歌唱起点。
fn is_rest_entry(entry: &Value) -> bool {
    matches!(
        entry.get("lyric").and_then(Value::as_str),
        Some("") | Some("<SP>") | None
    )
}

/// 秒 ⇒ tick（`round(seconds * bpm * PPQ / 60)`；非法输入 ⇒ `None`）。
fn seconds_to_ticks(seconds: f64, bpm: f64) -> Option<u64> {
    if !seconds.is_finite() || seconds < 0.0 || !bpm.is_finite() || bpm <= 0.0 {
        return None;
    }
    let ticks = (seconds * bpm * PPQ as f64 / 60.0).round();
    if !ticks.is_finite() || ticks < 0.0 || ticks > u64::MAX as f64 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some(ticks as u64)
}

/// 由回执对齐数组算放置起点。
///
/// - SoulX（含 stub 等其他模型）：按乐谱时钟贴轨，起点 = 调用方给的
///   `start_tick`；实测偏差只作校验（超阈值进 `deviation_warnings`），⛔ 不移位。
/// - VocalRender：按实测首音节贴轨 —— 片段起点 = `start_tick` −
///   `ticks(首个非休止音节的 measured_onset_s)`，使首音节恰好落在 `start_tick`；
///   若该移位超出 `start_tick`（起点会为负），钳制到 0 并警告首音节的实际落点。
/// - 休止（SP）条目：放置与偏差警告两处都忽略（它们没有真实歌唱起点，
///   服务侧给的只是邻近音节的候选值）。
/// - 对齐失败（条目带 `error`）或无可用实测起点时：VR 回落到乐谱时钟贴轨并警告，
///   ⛔ 不静默按一个假起点贴。
pub fn plan_placement(
    model: &str,
    start_tick: u64,
    bpm: f64,
    alignment: &[Value],
) -> PlacementOutcome {
    let mut notes = Vec::new();
    let mut deviation_warnings = Vec::new();
    let mut sp_skipped = 0usize;
    let mut alignment_error: Option<String> = None;
    for entry in alignment {
        if let Some(error) = entry.get("error").and_then(Value::as_str) {
            alignment_error = Some(error.to_owned());
            continue;
        }
        if is_rest_entry(entry) {
            sp_skipped += 1;
            continue;
        }
        if let Some(deviation) = entry.get("deviation_ms").and_then(Value::as_f64)
            && deviation.is_finite()
            && deviation.abs() > DEVIATION_WARN_MS
        {
            deviation_warnings.push(serde_json::json!({
                "lyric": entry.get("lyric").cloned().unwrap_or(Value::Null),
                "scoreOnsetS": entry.get("score_onset_s").cloned().unwrap_or(Value::Null),
                "measuredOnsetS": entry.get("measured_onset_s").cloned().unwrap_or(Value::Null),
                "deviationMs": deviation,
            }));
        }
    }
    if sp_skipped > 0 {
        notes.push(format!(
            "对齐数组里 {sp_skipped} 条休止（SP）无真实歌唱起点，放置与偏差警告里已忽略"
        ));
    }
    if model == "vocalrender" {
        let first_measured = alignment.iter().find_map(|entry| {
            if entry.get("error").is_some() || is_rest_entry(entry) {
                return None;
            }
            entry.get("measured_onset_s").and_then(Value::as_f64)
        });
        match first_measured.and_then(|seconds| seconds_to_ticks(seconds, bpm)) {
            Some(shift) => {
                let clamped = shift > start_tick;
                let placed_start = start_tick.saturating_sub(shift);
                let lands = if clamped { shift } else { start_tick };
                notes.push(
                    "VocalRender 按实测首音节贴轨：首音节落在 startTick，\
                     其余音节按实测相对位置落位"
                        .to_owned(),
                );
                if clamped {
                    notes.push(format!(
                        "首音节实测起点超出 startTick，片段起点钳制到 0，\
                         首音节实际落在第 {lands} tick（比请求的 startTick 晚 {} tick）",
                        lands - start_tick
                    ));
                }
                return PlacementOutcome {
                    start_tick: placed_start,
                    anchor: "measured",
                    first_syllable_lands_at_tick: Some(lands),
                    notes,
                    deviation_warnings,
                };
            }
            None => {
                if let Some(error) = &alignment_error {
                    notes.push(format!("对齐失败（服务端：{error}），无可用实测起点"));
                } else {
                    notes.push("无可用实测起点（首个非休止音节没有 measured_onset_s）".to_owned());
                }
                notes.push("回落到乐谱时钟贴轨（与 SoulX 同口径）".to_owned());
            }
        }
    } else {
        notes.push(
            "SoulX 按乐谱时钟贴轨：回执实测起点只作校验，偏差超过 100 ms 的条目见 deviationWarnings，⛔ 不自动移位"
                .to_owned(),
        );
    }
    PlacementOutcome {
        start_tick,
        anchor: "score",
        first_syllable_lands_at_tick: None,
        notes,
        deviation_warnings,
    }
}

/// 任务失败的响亮错误（服务端 `error` 原文照带；门耗尽同属这一条）。
fn job_failed_fault(job_id: &str, error: &Value) -> Fault {
    let text = error.as_str().unwrap_or("任务失败（服务端未给原因）");
    Fault::domain_with_data(
        ErrorCode::IoError,
        format!("SVS 任务 {job_id} 已失败：{text}"),
        serde_json::json!({
            "reason": "svsJobFailed",
            "job_id": job_id,
            "error": error,
        }),
    )
}

/// 状态摘要（snake_case ⇒ camelCase；缺键 ⇒ `serviceBadJson`，⛔ 不编缺省）。
fn status_summary(status: &Value) -> Result<Value, Fault> {
    let bad = || {
        Fault::domain_with_data(
            ErrorCode::IoError,
            "服务端任务状态缺字段（job_id / status / parts_total / takes_total / parts_completed / takes_completed）"
                .to_owned(),
            serde_json::json!({ "reason": "serviceBadJson" }),
        )
    };
    let job_id = status
        .get("job_id")
        .and_then(Value::as_str)
        .ok_or_else(bad)?;
    let state = status
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(bad)?;
    let parts_total = status
        .get("parts_total")
        .and_then(Value::as_u64)
        .ok_or_else(bad)?;
    let takes_total = status
        .get("takes_total")
        .and_then(Value::as_u64)
        .ok_or_else(bad)?;
    let parts_completed = status
        .get("parts_completed")
        .and_then(Value::as_u64)
        .ok_or_else(bad)?;
    let takes_completed = status
        .get("takes_completed")
        .and_then(Value::as_u64)
        .ok_or_else(bad)?;
    Ok(serde_json::json!({
        "jobId": job_id,
        "status": state,
        "partsTotal": parts_total,
        "takesTotal": takes_total,
        "partsCompleted": parts_completed,
        "takesCompleted": takes_completed,
        "error": status.get("error").cloned().unwrap_or(Value::Null),
    }))
}

/// 从状态全文里读任务状态字符串（缺失 ⇒ `serviceBadJson`）。
fn status_state(status: &Value) -> Result<&str, Fault> {
    status.get("status").and_then(Value::as_str).ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::IoError,
            "服务端任务状态缺 `status` 字段".to_owned(),
            serde_json::json!({ "reason": "serviceBadJson" }),
        )
    })
}

// ===========================================================================
// plan 入口（由 `domain::plan` 路由；`apply` 在 `domain::apply_plan_model`）
// ===========================================================================

/// `synthesize_singing` 的 plan：提交（`dryRun` 只校验不提交）。
///
/// # Errors
///
/// 参数非法 / 服务失败见模块头失败语义。
pub fn plan_synthesize(_domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let parts = call.arguments.get("parts").cloned().unwrap_or(Value::Null);
    let (parts_total, takes_total) = validate_parts(&parts)?;
    let service_url = call.arguments.get("service_url").and_then(Value::as_str);
    let config = resolve_config(service_url)?;
    if call.is_dry_run() {
        return Ok(Plan::SvsSubmit {
            data: serde_json::json!({
                "wouldSubmit": true,
                "serviceUrl": config.base_url,
                "partsTotal": parts_total,
                "takesTotal": takes_total,
                "note": "dryRun：只校验参数，未向服务提交",
            }),
        });
    }
    let client = SvsClient::new(&config)?;
    let submitted = client.synthesize(&parts)?;
    let bad = || {
        Fault::domain_with_data(
            ErrorCode::IoError,
            "服务端提交回执缺字段（job_id / status / parts_total / takes_total）".to_owned(),
            serde_json::json!({ "reason": "serviceBadJson" }),
        )
    };
    let job_id = submitted
        .get("job_id")
        .and_then(Value::as_str)
        .ok_or_else(bad)?;
    let state = submitted
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(bad)?;
    let server_parts = submitted
        .get("parts_total")
        .and_then(Value::as_u64)
        .ok_or_else(bad)?;
    let server_takes = submitted
        .get("takes_total")
        .and_then(Value::as_u64)
        .ok_or_else(bad)?;
    Ok(Plan::SvsSubmit {
        data: serde_json::json!({
            "jobId": job_id,
            "status": state,
            "partsTotal": server_parts,
            "takesTotal": server_takes,
            "serviceUrl": client.base_url(),
            "note": "提交成功只是任务已入队；用 svs_job 轮询完成情况",
        }),
    })
}

/// `svs_job` 的 plan：按 `want` 分流（状态 / 回执 / 音频）。
///
/// # Errors
///
/// 参数非法 / 服务失败 / 任务失败 / 未完成取音频见模块头失败语义。
/// 落轨（`trackId`）时要求活跃工程（`NO_ACTIVE_PROJECT`）。
pub fn plan_svs_job(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let args = parse_svs_job_args(call)?;
    let config = resolve_config(args.service_url.as_deref())?;
    let client = SvsClient::new(&config)?;
    match args.want {
        SvsWant::Status => {
            let status = client.job_status(&args.job_id)?;
            Ok(Plan::SvsJob {
                data: status_summary(&status)?,
            })
        }
        SvsWant::Receipt => {
            let receipt = client.job_receipt(&args.job_id)?;
            let job_id = receipt
                .get("job_id")
                .and_then(Value::as_str)
                .unwrap_or(&args.job_id);
            Ok(Plan::SvsJob {
                data: serde_json::json!({
                    "jobId": job_id,
                    "receipt": receipt,
                }),
            })
        }
        SvsWant::Audio => plan_svs_audio(domain, call, &args, &client),
    }
}

/// `svs_job` `want=audio` 的 plan（取音频；`trackId` 给定时落轨）。
fn plan_svs_audio(
    domain: &Domain,
    call: &ToolCall,
    args: &SvsJobArgs,
    client: &SvsClient,
) -> Result<Plan, Fault> {
    let part = args.part.clone().unwrap_or_default();
    let take = args.take.unwrap_or(0);
    // 取音频前先查状态：失败的任务、未完成的任务都在这里响亮拒绝，
    // ⛔ 不走到音频端点去拿一个 404 或静音文件。
    let status = client.job_status(&args.job_id)?;
    let state = status_state(&status)?;
    if state == "failed" {
        return Err(job_failed_fault(
            &args.job_id,
            status.get("error").unwrap_or(&Value::Null),
        ));
    }
    if state != "completed" {
        return Err(Fault::domain_with_data(
            ErrorCode::IoError,
            format!(
                "任务 {} 尚未完成（当前 {state}），音频未生成；先用 want=status 轮询",
                args.job_id
            ),
            serde_json::json!({
                "reason": "jobNotCompleted",
                "job_id": args.job_id,
                "status": state,
            }),
        ));
    }
    let receipt = client.job_receipt(&args.job_id)?;
    let (model, take_entry) = receipt_take(&receipt, &args.job_id, &part, take)?;
    let alignment: Vec<Value> = take_entry
        .get("alignment")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| {
            Fault::domain_with_data(
                ErrorCode::IoError,
                format!(
                    "任务 {} 的回执里 part={part} take={take} 没有 alignment 数组",
                    args.job_id
                ),
                serde_json::json!({ "reason": "serviceBadJson" }),
            )
        })?;
    let duration_s = take_entry.get("duration_s").cloned().unwrap_or(Value::Null);
    let cache_path = wav_cache_path(&args.job_id, &part, take);
    let cache_display = cache_path.display().to_string();
    // dryRun：只读回执已取到，摆放/取回皆可预览；⛔ 不取 wav、不写暂存、不建导入计划。
    if call.is_dry_run() {
        if args.track_id.is_some() {
            let project = super::require_active(domain)?;
            let outcome = plan_placement(
                &model,
                args.start_tick.unwrap_or(0),
                project.bpm,
                &alignment,
            );
            return Ok(Plan::SvsAudio {
                import: None,
                report: placement_report(args, &model, &outcome, &cache_display, duration_s, true),
            });
        }
        return Ok(Plan::SvsAudio {
            import: None,
            report: serde_json::json!({
                "wouldFetch": true,
                "jobId": args.job_id,
                "part": part,
                "take": take,
                "model": model,
                "wavPath": cache_display,
                "durationS": duration_s,
                "placed": false,
                "note": "dryRun：未取 wav、未写暂存文件；真调用会把 wav 写到 wavPath（未给 trackId，不落轨）",
            }),
        });
    }
    let wav = client.job_audio(&args.job_id, &part, take)?;
    let wav_bytes = wav.len();
    if let Some(track_id) = args.track_id {
        let project = super::require_active(domain)?;
        let outcome = plan_placement(
            &model,
            args.start_tick.unwrap_or(0),
            project.bpm,
            &alignment,
        );
        let mut import_args = Map::new();
        import_args.insert(
            "name".to_owned(),
            Value::from(format!("svs {part} take {take}")),
        );
        import_args.insert("path".to_owned(), Value::from(cache_display.clone()));
        import_args.insert(
            "license".to_owned(),
            Value::from(args.license.clone().unwrap_or_default()),
        );
        if let Some(gain_db) = args.gain_db {
            import_args.insert("gainDb".to_owned(), Value::from(gain_db));
        }
        import_args.insert(
            "trackId".to_owned(),
            Value::from(track_id.to_canonical_string()),
        );
        import_args.insert("startTick".to_owned(), Value::from(outcome.start_tick));
        // 片段身份由 (任务, part, 遍) 确定性派生 ⇒ 重复取同一遍命中同一条
        // clip_pool 记录（`import_audio` 的幂等口径），⛔ 不靠暂存路径派生
        // （路径只是缓存位置，不是内容身份）。
        import_args.insert(
            "clipId".to_owned(),
            Value::from(
                deterministic_id(&format!("svs:{}:{part}:{take}", args.job_id))
                    .to_canonical_string(),
            ),
        );
        // 先写暂存文件，再建导入计划（`import_audio::plan` 读 `path`）。
        if let Some(parent) = cache_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| super::error::from_io("创建 SVS 暂存目录", &error))?;
        }
        std::fs::write(&cache_path, &wav)
            .map_err(|error| super::error::from_io("写 SVS 暂存 wav", &error))?;
        let import = import_audio::plan(project, domain, &import_args)?;
        let mut report =
            placement_report(args, &model, &outcome, &cache_display, duration_s, false);
        if let Some(object) = report.as_object_mut() {
            object.insert("wavBytes".to_owned(), Value::from(wav_bytes));
        }
        return Ok(Plan::SvsAudio {
            import: Some(Box::new(import)),
            report,
        });
    }
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| super::error::from_io("创建 SVS 暂存目录", &error))?;
    }
    std::fs::write(&cache_path, &wav)
        .map_err(|error| super::error::from_io("写 SVS 暂存 wav", &error))?;
    Ok(Plan::SvsAudio {
        import: None,
        report: serde_json::json!({
            "jobId": args.job_id,
            "part": part,
            "take": take,
            "model": model,
            "wavPath": cache_display,
            "wavBytes": wav_bytes,
            "durationS": duration_s,
            "placed": false,
            "note": "未给 trackId，未落轨；wav 已写本地暂存路径（稳定路径，重复取同一遍覆盖同一文件）",
        }),
    })
}

/// 摆放报告（`SvsAudio` 的 report；落轨时再与 `import_audio` 的响应合并）。
fn placement_report(
    args: &SvsJobArgs,
    model: &str,
    outcome: &PlacementOutcome,
    wav_path: &str,
    duration_s: Value,
    dry_run: bool,
) -> Value {
    let mut report = serde_json::json!({
        "jobId": args.job_id,
        "part": args.part.clone().unwrap_or_default(),
        "take": args.take.unwrap_or(0),
        "model": model,
        "placementModel": outcome.anchor,
        "placementStartTick": outcome.start_tick,
        "placementNotes": outcome.notes,
        "deviationWarnings": outcome.deviation_warnings,
        "wavPath": wav_path,
        "durationS": duration_s,
        "placed": true,
    });
    if let Some(lands) = outcome.first_syllable_lands_at_tick {
        report["firstSyllableLandsAtTick"] = Value::from(lands);
    }
    if dry_run {
        report["wouldPlace"] = Value::from(true);
        report["note"] = Value::from("dryRun：只算摆放预览，未取 wav、未写暂存、未改工程");
    }
    report
}

/// 回执里定位某 part 某遍（缺失 ⇒ `ENTITY_NOT_FOUND`，附可用 part 清单）。
fn receipt_take<'a>(
    receipt: &'a Value,
    job_id: &str,
    part: &str,
    take: u64,
) -> Result<(String, &'a Value), Fault> {
    let parts = receipt
        .get("parts")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            Fault::domain_with_data(
                ErrorCode::IoError,
                format!("任务 {job_id} 的回执没有 parts 对象"),
                serde_json::json!({ "reason": "serviceBadJson" }),
            )
        })?;
    let entry = parts.get(part).ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::EntityNotFound,
            format!("任务 {job_id} 的回执里没有 part {part}"),
            serde_json::json!({
                "reason": "svsPartNotInReceipt",
                "job_id": job_id,
                "part": part,
                "availableParts": parts.keys().cloned().collect::<Vec<_>>(),
            }),
        )
    })?;
    let model = entry
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("soulx")
        .to_owned();
    let takes = entry
        .get("takes")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            Fault::domain_with_data(
                ErrorCode::IoError,
                format!("任务 {job_id} 的回执里 part={part} 没有 takes 数组",),
                serde_json::json!({ "reason": "serviceBadJson" }),
            )
        })?;
    let found = takes
        .iter()
        .find(|entry| entry.get("take").and_then(Value::as_u64) == Some(take));
    match found {
        Some(entry) => Ok((model, entry)),
        None => Err(Fault::domain_with_data(
            ErrorCode::EntityNotFound,
            format!("任务 {job_id} 的回执里 part={part} 没有 take={take}"),
            serde_json::json!({
                "reason": "svsTakeNotInReceipt",
                "job_id": job_id,
                "part": part,
                "take": take,
            }),
        )),
    }
}

/// `SvsAudio` 的 apply：有导入计划 ⇒ 走 `import_audio::apply` 再合并报告；
/// 无计划（只取回）⇒ 原样返回报告。
///
/// # Errors
///
/// 与 `import_audio::apply` 同（含工程摘要预测不一致的 `CONFLICT`）。
pub fn apply_svs_audio(
    domain: &mut Domain,
    import: Option<Box<import_audio::AudioImport>>,
    report: Value,
) -> Result<ToolResponse, Fault> {
    match import {
        None => Ok(ToolResponse::success(report)),
        Some(import) => {
            let response = import_audio::apply(domain, &import)?;
            match response {
                ToolResponse::Success { data } => {
                    let mut data = data;
                    if let (Some(map), Value::Object(extra)) = (data.as_object_mut(), report) {
                        for (key, value) in extra {
                            map.insert(key, value);
                        }
                    }
                    Ok(ToolResponse::success(data))
                }
                failure => Ok(failure),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alignment_entry(lyric: &str, score: f64, measured: Option<f64>, dev: Option<f64>) -> Value {
        let mut entry = serde_json::json!({
            "lyric": lyric,
            "score_onset_s": score,
            "measured_onset_s": measured,
            "deviation_ms": dev,
        });
        if measured.is_none() {
            entry["measured_onset_s"] = Value::Null;
        }
        if dev.is_none() {
            entry["deviation_ms"] = Value::Null;
        }
        entry
    }

    #[test]
    fn http_target_parsing_covers_defaults_prefix_and_ipv6() {
        let target = parse_http_target("http://127.0.0.1:8000").expect("解析");
        assert_eq!(target.host, "127.0.0.1");
        assert_eq!(target.port, 8000);
        assert_eq!(target.prefix, "");
        assert_eq!(target.host_header, "127.0.0.1:8000");
        let target = parse_http_target("http://localhost").expect("解析");
        assert_eq!(target.port, 80);
        assert_eq!(target.host_header, "localhost");
        let target = parse_http_target("http://127.0.0.1:9000/prefix/").expect("解析");
        assert_eq!(target.prefix, "/prefix");
        let target = parse_http_target("http://[::1]:8000").expect("解析");
        assert_eq!(target.host, "[::1]");
        assert_eq!(target.port, 8000);
        assert!(parse_http_target("http://").is_err());
        assert!(parse_http_target("http://host:0").is_err());
        assert!(parse_http_target("http://host:notaport").is_err());
    }

    #[test]
    fn https_is_rejected_loudly_at_config_time() {
        let error = resolve_config(Some("https://example.com")).expect_err("https 必须被拒");
        assert_eq!(error.domain_code(), Some(ErrorCode::IoError));
    }

    #[test]
    fn query_encoding_and_filename_sanitizing_do_not_let_labels_escape() {
        assert_eq!(percent_encode("lead vocal/1"), "lead%20vocal%2F1");
        assert_eq!(percent_encode("abc-._~"), "abc-._~");
        assert_eq!(sanitize_file_component("a/b:c"), "a_b_c");
        let path = wav_cache_path("abc123", "lead/vocal", 2);
        assert!(
            path.display()
                .to_string()
                .ends_with("abc123_lead_vocal_take2.wav")
        );
    }

    #[test]
    fn chunked_and_content_length_bodies_both_decode() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhelloEXTRA";
        let response = parse_response(raw).expect("解析");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"hello");
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        let response = parse_response(raw).expect("解析");
        assert_eq!(response.body, b"hello");
        assert!(parse_response(b"garbage").is_err());
    }

    #[test]
    fn soulx_placement_keeps_the_score_clock_and_flags_large_deviations() {
        let alignment = vec![
            alignment_entry("la", 0.0, Some(0.01), Some(10.0)),
            alignment_entry("<SP>", 1.0, Some(1.2), Some(500.0)),
            alignment_entry("ti", 2.0, Some(2.35), Some(350.0)),
        ];
        let outcome = plan_placement("soulx", 960, 120.0, &alignment);
        assert_eq!(outcome.start_tick, 960);
        assert_eq!(outcome.anchor, "score");
        assert_eq!(outcome.first_syllable_lands_at_tick, None);
        // SP 的 500 ms 不进警告；只有 ti 的 350 ms 进。
        assert_eq!(outcome.deviation_warnings.len(), 1);
        assert_eq!(outcome.deviation_warnings[0]["lyric"], "ti");
        assert!(outcome.notes.iter().any(|note| note.contains("SP")));
    }

    #[test]
    fn vocalrender_placement_anchors_the_first_sung_syllable() {
        // bpm 120、PPQ 960 ⇒ 1 秒 = 1920 tick。首音节实测 0.5 秒 ⇒ 移 960 tick。
        let alignment = vec![
            alignment_entry("<SP>", 0.0, Some(0.0), Some(0.0)),
            alignment_entry("la", 0.5, Some(0.5), Some(0.0)),
            alignment_entry("ti", 1.5, Some(1.6), Some(100.0)),
        ];
        let outcome = plan_placement("vocalrender", 1920, 120.0, &alignment);
        assert_eq!(outcome.anchor, "measured");
        assert_eq!(outcome.start_tick, 960);
        assert_eq!(outcome.first_syllable_lands_at_tick, Some(1920));
        // 100 ms 恰好不超阈值（严格大于才警告）。
        assert!(outcome.deviation_warnings.is_empty());
    }

    #[test]
    fn vocalrender_placement_clamps_and_reports_the_real_landing_tick() {
        let alignment = vec![alignment_entry("la", 0.0, Some(1.0), Some(0.0))];
        let outcome = plan_placement("vocalrender", 960, 120.0, &alignment);
        assert_eq!(outcome.start_tick, 0);
        assert_eq!(outcome.first_syllable_lands_at_tick, Some(1920));
        assert!(outcome.notes.iter().any(|note| note.contains("钳制到 0")));
    }

    #[test]
    fn vocalrender_without_measured_onsets_falls_back_to_score_loudly() {
        let alignment = vec![alignment_entry("la", 0.0, None, None)];
        let outcome = plan_placement("vocalrender", 480, 120.0, &alignment);
        assert_eq!(outcome.anchor, "score");
        assert_eq!(outcome.start_tick, 480);
        assert!(
            outcome
                .notes
                .iter()
                .any(|note| note.contains("回落到乐谱时钟"))
        );
        let failed = vec![serde_json::json!({"error": "alignment failed: boom"})];
        let outcome = plan_placement("vocalrender", 480, 120.0, &failed);
        assert_eq!(outcome.anchor, "score");
        assert!(outcome.notes.iter().any(|note| note.contains("boom")));
    }

    #[test]
    fn parts_validation_rejects_bad_shapes_and_unknown_keys() {
        let good = serde_json::json!([{
            "label": "lead",
            "score": {
                "bpm": 120.0,
                "notes": [
                    {"lyric": "la", "midi": 69, "start_s": 0.0, "dur_s": 1.0},
                    {"lyric": "<SP>", "midi": 0, "start_s": 1.0, "dur_s": 0.5},
                ],
            },
            "voice_id": "v1",
            "takes": 2,
        }]);
        assert_eq!(validate_parts(&good).expect("合法"), (1, 2));
        assert!(validate_parts(&serde_json::json!([])).is_err());
        assert!(validate_parts(&serde_json::json!([{"label": "x"}])).is_err());
        let bad_takes = serde_json::json!([{
            "label": "lead",
            "score": {"bpm": 120.0, "notes": []},
            "voice_id": "v1",
            "takes": 9,
        }]);
        assert!(validate_parts(&bad_takes).is_err());
        let bad_note = serde_json::json!([{
            "label": "lead",
            "score": {"bpm": 120.0, "notes": [
                {"lyric": "la", "midi": 69, "start_s": 0.0, "dur_s": 0.0},
            ]},
            "voice_id": "v1",
        }]);
        assert!(validate_parts(&bad_note).is_err());
        let unknown_key = serde_json::json!([{
            "label": "lead",
            "score": {"bpm": 120.0, "notes": []},
            "voice_id": "v1",
            "surprise": 1,
        }]);
        assert!(validate_parts(&unknown_key).is_err());
    }
}
