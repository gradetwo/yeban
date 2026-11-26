//! ⭐ T-FIX-339（插件崩溃隔离）：子进程探测器。
//!
//! 一期宿主是**进程内** VST3 宿主：坏插件在 `dlopen` / 工厂枚举 / 验证激活时
//! `abort()` 或无限挂起会带走整个应用与音频线程。`catch_unwind` 抓不住 `abort()` /
//! SIGSEGV，唯一可靠的隔离边界是**进程**。
//!
//! 本模块提供两半：
//! - [`run_probe_scan`]：在**子进程**里执行「加载模块 → 枚举类 → 验证激活」，把结果
//!   编码成紧凑文本协议写到 stdout；任何 panic / abort 都只杀子进程。
//! - [`spawn_scan`]：父进程用 [`std::process::Command`] 启动探针子进程，带**超时**
//!   （`try_wait` 轮询 + `kill`），崩溃 / 超时 / 非零退出 ⇒ 一条 `error` 描述符，
//!   父进程存活、继续扫下一个候选。
//!
//! 探针可执行文件定位：环境变量 `YEBAN_PLUGIN_PROBE_BIN`（部署 / 测试注入）；未设置时
//! 回退到与当前 exe 同目录的 `yeban-plugin-probe`（[`probe_binary_path`]）。
//! ⚠ 部署约定：应用分发必须携带该探针二进制；缺失时 [`spawn_scan`] 返回 `None`，
//! 由调用方显式登记「未隔离」缺口而不是静默降级（不把 phase 2 当成已解决）。
//!
//! 协议（stdout，每行一条，`\t` 分隔；`%` 后是转义后的字节）：
//! ```text
//! V\t<vendor>
//! C\t<tuid-hex>\t<name>\t<category>\t<kind: i|e|u>\t<error|->
//! E\t<error>
//! OK
//! ```

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::AraFactoryInfo;
use crate::types::{PluginClass, PluginKind};

/// 探针环境变量名（指向探针可执行文件）。
pub const PROBE_BIN_ENV: &str = "YEBAN_PLUGIN_PROBE_BIN";
/// 默认探针可执行文件名（与 `Cargo.toml` 的 `[[bin]]` 同名）。
pub const PROBE_BIN_NAME: &str = "yeban-plugin-probe";
/// 默认探测超时。
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// 超时覆盖环境变量名（毫秒；判据/部署按需压短）。
pub const PROBE_TIMEOUT_ENV: &str = "YEBAN_PLUGIN_PROBE_TIMEOUT_MS";

/// 当前探测超时：优先读 [`PROBE_TIMEOUT_ENV`]（毫秒），否则 [`DEFAULT_PROBE_TIMEOUT`]。
#[must_use]
pub fn probe_timeout() -> Duration {
    std::env::var(PROBE_TIMEOUT_ENV)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_PROBE_TIMEOUT)
}

fn esc(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\t', "%09")
        .replace('\n', "%0A")
        .replace('\r', "%0D")
}

fn unesc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hi = chars.next();
            let lo = chars.next();
            if let (Some(hi), Some(lo)) = (hi, lo) {
                let hex: String = [hi, lo].iter().collect();
                if let Ok(b) = u8::from_str_radix(&hex, 16) {
                    out.push(b as char);
                    continue;
                }
            }
            out.push('%');
            if let Some(hi) = hi {
                out.push(hi);
            }
            if let Some(lo) = lo {
                out.push(lo);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// 在子进程内执行扫描探测，把结果写到 stdout（供父进程按协议解析）。
///
/// 本函数**不返回 Result**：任何失败都编码成 `E\t...` 行（子进程随后正常退出）；
/// 只有 `abort()` / SIGSEGV 才会让子进程异常退出 —— 那正是父进程要检测的崩溃信号。
pub fn run_probe_scan(path: &str) {
    let mut stdout = std::io::stdout().lock();
    match crate::vst3::Vst3Module::load(path) {
        Err(error) => {
            let _ = writeln!(stdout, "E\t{}", esc(&error.to_string()));
        }
        Ok(module) => {
            match crate::vst3::inspect_loaded_module(&module) {
                Err(error) => {
                    let _ = writeln!(stdout, "E\t{}", esc(&error.to_string()));
                }
                Ok((vendor, classes)) => {
                    let _ = writeln!(stdout, "V\t{}", esc(&vendor));
                    for class in &classes {
                        let kind = match class.kind {
                            PluginKind::Instrument => "i",
                            PluginKind::Effect => "e",
                            PluginKind::Unknown => "u",
                        };
                        let err = class.error.as_deref().map_or("-".to_owned(), esc);
                        let _ = writeln!(
                            stdout,
                            "C\t{}\t{}\t{}\t{}\t{}",
                            crate::types::tuid_hex(&class.id),
                            esc(&class.name),
                            esc(&class.category),
                            kind,
                            err
                        );
                    }
                }
            }
            // ARA 探针同样在子进程内跑：它也可能触发坏插件崩溃。全字段编码成一行。
            if let Some(ara) = crate::ara::inspect_ara_factory(&module) {
                let types: Vec<String> = ara
                    .analyzeable_content_types
                    .iter()
                    .map(|v| v.to_string())
                    .collect();
                let _ = writeln!(
                    stdout,
                    "A\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    esc(&ara.plug_in_name),
                    esc(&ara.manufacturer),
                    esc(&ara.version),
                    esc(&ara.factory_id),
                    esc(&ara.document_archive_id),
                    types.join(","),
                    ara.supported_playback_transformation_flags,
                    ara.lowest_api_generation,
                    ara.highest_api_generation,
                );
            }
            let _ = writeln!(stdout, "OK");
        }
    }
    let _ = stdout.flush();
}

/// 子进程扫描结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeScanResult {
    /// 厂商名（子进程读到的 `PFactoryInfo.vendor`）。
    pub vendor: String,
    /// 工厂类表（含每个类的实例化探针结论）。
    pub classes: Vec<PluginClass>,
    /// 模块级失败原因（崩溃 / 超时 / 加载失败；此时 `classes` 为空）。
    pub error: Option<String>,
    /// ARA 能力（子进程探针读到的全字段；无 ARA 时为 `None`）。
    pub ara: Option<AraFactoryInfo>,
}

/// 定位探针可执行文件。
pub fn probe_binary_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var(PROBE_BIN_ENV) {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Some(path);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let sibling = exe.with_file_name(PROBE_BIN_NAME);
    if sibling.is_file() {
        return Some(sibling);
    }
    None
}

fn decode(output: &str) -> ProbeScanResult {
    let mut vendor = String::new();
    let mut classes = Vec::new();
    let mut error = None;
    let mut ara = None;
    for line in output.lines() {
        let mut parts = line.split('\t');
        match parts.next() {
            Some("V") => vendor = parts.next().map(unesc).unwrap_or_default(),
            Some("C") => {
                let id_hex = parts.next().unwrap_or("");
                let name = parts.next().map(unesc).unwrap_or_default();
                let category = parts.next().map(unesc).unwrap_or_default();
                let kind = match parts.next() {
                    Some("i") => PluginKind::Instrument,
                    Some("e") => PluginKind::Effect,
                    _ => PluginKind::Unknown,
                };
                let err = parts.next().map(unesc).filter(|s| s != "-");
                let id = crate::types::tuid_from_hex(id_hex);
                classes.push(PluginClass {
                    id,
                    name,
                    category,
                    kind,
                    error: err,
                });
            }
            Some("E") => error = Some(parts.next().map(unesc).unwrap_or_default()),
            Some("A") => {
                let next = |parts: &mut std::str::Split<'_, char>| -> String {
                    parts.next().map(unesc).unwrap_or_default()
                };
                let types: Vec<i32> = next(&mut parts)
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .filter_map(|s| s.parse().ok())
                    .collect();
                ara = Some(AraFactoryInfo {
                    plug_in_name: next(&mut parts),
                    manufacturer: next(&mut parts),
                    version: next(&mut parts),
                    factory_id: next(&mut parts),
                    document_archive_id: next(&mut parts),
                    analyzeable_content_types: types,
                    supported_playback_transformation_flags: parts
                        .next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0),
                    lowest_api_generation: parts.next().and_then(|s| s.parse().ok()).unwrap_or(0),
                    highest_api_generation: parts.next().and_then(|s| s.parse().ok()).unwrap_or(0),
                });
            }
            _ => {}
        }
    }
    ProbeScanResult {
        vendor,
        classes,
        error,
        ara,
    }
}

/// 在子进程里探测一个模块；返回 `None` 表示探针不可用（未部署 / 无法启动），
/// 由调用方决定是否降级。崩溃 / 超时 / 非零退出返回一条 `error` 结果，父进程存活。
#[must_use]
pub fn spawn_scan(path: &Path, timeout: Duration) -> Option<ProbeScanResult> {
    let bin = probe_binary_path()?;
    let mut child = std::process::Command::new(bin)
        .arg("scan")
        .arg(path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut output = String::new();
                use std::io::Read as _;
                if let Some(mut stdout) = child.stdout.take() {
                    let _ = stdout.read_to_string(&mut output);
                }
                if status.success() {
                    return Some(decode(&output));
                }
                return Some(ProbeScanResult {
                    vendor: String::new(),
                    classes: Vec::new(),
                    error: Some(format!(
                        "插件探测子进程异常退出（status={status}）—— 插件可能已崩溃"
                    )),
                    ara: None,
                });
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Some(ProbeScanResult {
                        vendor: String::new(),
                        classes: Vec::new(),
                        error: Some(format!(
                            "插件探测超时（{} ms）—— 插件可能已挂起",
                            timeout.as_millis()
                        )),
                        ara: None,
                    });
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => {
                let _ = child.kill();
                return Some(ProbeScanResult {
                    vendor: String::new(),
                    classes: Vec::new(),
                    error: Some("插件探测子进程状态查询失败".to_owned()),
                    ara: None,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_unesc_round_trip() {
        let raw = "a\tb\nc%d\re";
        assert_eq!(unesc(&esc(raw)), raw);
    }
}
