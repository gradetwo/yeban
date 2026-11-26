//! 发现文件：`ready.json` 与 `<pid>.json` 的读取与校验。
//!
//! 规则（设计稿 §2.1）：
//! * 控制目录须为 0700（组／其他可写时服务端静默拒绝 control API，
//!   驱动先自检、⛔ 不等到超时）；
//! * `ready.json` 等待上限 30 秒；
//! * `<pid>.json` 校验三项：`launchNonce` 一致、pid 存活、
//!   `processStartTimeMs` 与本机实测的进程启动时间相差 ≤ 2000 ms
//!   —— 任一不符即视为陈旧文件，⛔ 不连接、可删除；
//! * token / serverProof 只出现在发现文件与 `session.hello` 里，
//!   ⛔ 不写进任何日志、错误详情、任务收据与 MCP 响应。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::OtcpError;

/// `ready.json` 的内容。
///
/// 协议原文（`PROTOCOL.md` §5.5，`schemas/discovery-file.schema.json`
/// `$defs/readyFile`，闭合）：ready 文件只含 `{pid, instanceId}`，
/// 不含秘密、**不含端口**；端口在发现文件 `<pid>.json` 里。
/// 2026-11-11 在 yanshi-5 上对 9ef45f9 真二进制实测确认：
/// `{"pid":1077812,"instanceId":"f6f9a2f12862b59c0d5f4b41852e61da"}`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadyFile {
    /// 进程 pid；发现文件即 `<control_dir>/<pid>.json`。
    pub pid: u32,
    /// 实例 id（⛔ 不是秘密）；须与发现文件的 `instanceId` 一致。
    #[serde(rename = "instanceId")]
    pub instance_id: String,
}

/// `<pid>.json` 发现文件的内容。
///
/// 协议原文（`PROTOCOL.md` §5.4，`schemas/discovery-file.schema.json`
/// `$defs/knownMembers`）：14 个成员恒在场；`build` 是对象
/// `{commit, tag}`、`launchNonce` 为 string|null、`buildTag` 为
/// string|null。2026-11-11 在 yanshi-5 上对 9ef45f9 真二进制实测：
/// `{"protocolMajors":[1],"pid":…,"processStartTimeMs":…,"instanceId":…,
/// "port":36833,"token":…,"serverProof":…,"launchNonce":…,
/// "appVersion":"1.8.2","build":{"commit":"9ef45f9a…","tag":null},
/// "buildTag":null,"sourceUrl":"https://github.com/YuFeng926/OpenTune",
/// "startedAtMs":…,"flavor":"standalone"}`。
/// 本结构只取驱动要用的成员，未知成员按协议 §2.3 规则 1 忽略。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryFile {
    /// 控制 API 监听端口。
    pub port: u16,
    /// 进程 pid。
    pub pid: u32,
    /// 服务端启动时验证用的 nonce（⛔ 不记日志）。
    /// 未传 `--control-api-launch-nonce` 时服务端写 null。
    #[serde(rename = "launchNonce")]
    pub launch_nonce: Option<String>,
    /// 进程启动时间（Unix 毫秒）。
    #[serde(rename = "processStartTimeMs")]
    pub process_start_time_ms: u64,
    /// 控制 API token（⛔ 秘密，不记日志）。
    #[serde(default)]
    pub token: String,
    /// 服务端证明（⛔ 秘密，不记日志）。
    #[serde(default, rename = "serverProof")]
    pub server_proof: String,
    /// 实例 id（⛔ 不是秘密）。
    #[serde(default, rename = "instanceId")]
    pub instance_id: String,
}

impl DiscoveryFile {
    /// 去除秘密字段后的调试视图（日志只准打这个）。
    #[must_use]
    pub fn redacted_debug(&self) -> String {
        format!(
            "DiscoveryFile {{ port: {}, pid: {}, processStartTimeMs: {}, instanceId: {} }}",
            self.port, self.pid, self.process_start_time_ms, self.instance_id
        )
    }
}

/// 检查控制目录模式：须为 0700（组／其他不可写）。
///
/// Unix 下读 `permissions().mode()` 的低 9 位；非 Unix 平台跳过检查
/// （服务端只在类 Unix 上以此规则拒收）。
pub fn check_control_dir_mode(dir: &Path) -> Result<(), OtcpError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(dir)
            .map_err(|error| OtcpError::io(format!("读控制目录 {} 失败: {error}", dir.display())))?
            .permissions()
            .mode()
            & 0o777;
        if mode & 0o077 != 0 {
            return Err(OtcpError::with_data(
                "otcpNotConfigured",
                format!(
                    "控制目录 {} 模式为 {mode:03o}（组／其他可写），服务端会静默拒绝 control API；须为 0700",
                    dir.display()
                ),
                serde_json::json!({"control_dir": dir.display().to_string(), "mode": format!("{mode:03o}")}),
            ));
        }
    }
    // 非 Unix 下 `dir` 只在 Unix 权限检查里用；显式消费，避免 Windows 构建报 unused variable。
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
    Ok(())
}

/// 等待 `ready.json` 出现并解析（上限 `timeout`）。
pub fn wait_ready_file(dir: &Path, timeout: Duration) -> Result<ReadyFile, OtcpError> {
    let path = dir.join("ready.json");
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match fs::read_to_string(&path) {
            Ok(text) => {
                return serde_json::from_str::<ReadyFile>(&text).map_err(|error| {
                    OtcpError::protocol(format!("解析 {} 失败: {error}", path.display()))
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if std::time::Instant::now() >= deadline {
                    return Err(OtcpError::transport(format!(
                        "等 ready.json 超时（{} 秒）：{}",
                        timeout.as_secs(),
                        path.display()
                    )));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(error) => {
                return Err(OtcpError::io(format!(
                    "读 {} 失败: {error}",
                    path.display()
                )));
            }
        }
    }
}

/// 读并校验发现文件。
///
/// * `expected_nonce`：本次启动用的 nonce，必须一致；
/// * pid 必须存活（平台原生检查：Linux `/proc`、macOS `kill -0`、Windows `OpenProcess`）；
/// * `processStartTimeMs` 与本机实测的进程启动时间相差 ≤ 2000 ms。
///   任一不符 ⇒ 陈旧文件，返回 `Err`（调用方可删文件后重试启动）。
pub fn read_discovery_file(path: &Path, expected_nonce: &str) -> Result<DiscoveryFile, OtcpError> {
    let text = fs::read_to_string(path)
        .map_err(|error| OtcpError::io(format!("读发现文件 {} 失败: {error}", path.display())))?;
    let file: DiscoveryFile = serde_json::from_str(&text).map_err(|error| {
        OtcpError::protocol(format!("解析发现文件 {} 失败: {error}", path.display()))
    })?;
    if file.launch_nonce.as_deref() != Some(expected_nonce) {
        return Err(OtcpError::protocol(format!(
            "发现文件 {} 的 launchNonce 与本次启动不符（陈旧文件）",
            path.display()
        )));
    }
    if !pid_alive(file.pid) {
        return Err(OtcpError::protocol(format!(
            "发现文件 {} 的 pid {} 已死（陈旧文件）",
            path.display(),
            file.pid
        )));
    }
    let start_ms = process_start_time_ms(file.pid).ok_or_else(|| {
        OtcpError::protocol(format!("读 pid {} 的启动时间失败（陈旧文件）", file.pid))
    })?;
    let delta = start_ms.abs_diff(file.process_start_time_ms);
    if delta > 2000 {
        return Err(OtcpError::protocol(format!(
            "发现文件 {} 的 processStartTimeMs 与实测相差 {delta} ms（陈旧文件）",
            path.display()
        )));
    }
    Ok(file)
}

/// pid 是否存活。
///
/// Linux 读 `/proc/<pid>`；macOS 用 `kill -0`（本 crate 无 libc 依赖，与
/// supervisor 的 SIGTERM 同走 `/bin/kill`）；Windows 用
/// `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` 并用 `GetExitCodeProcess`
/// （`STILL_ACTIVE` 才算存活）。其余平台没有可靠实现，按“不可判定”返回
/// `false`，由后续 TCP 连接与握手校验兜底。
///
/// ⚠ 审计 #18：`/proc` 是 Linux 专属，`#[cfg(unix)]` 会把没有 `/proc` 的
/// macOS 一并圈进来恒判已死——/proc 实现只许 `target_os = "linux"` 门控。
#[must_use]
#[cfg(target_os = "linux")]
pub fn pid_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[must_use]
#[cfg(target_os = "macos")]
pub fn pid_alive(pid: u32) -> bool {
    // `kill -0 <pid>`：进程存在 ⇒ 退出码 0。本驱动查的恒是自家拉起的同用户
    // 进程，EPERM（他人进程）不出现；真出现也只会误判已死，由发现文件校验
    // 响亮报错、握手兜底，不会静默放行陈旧文件。
    std::process::Command::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[must_use]
#[cfg(windows)]
pub fn pid_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // SAFETY: OpenProcess 返回的句柄在本函数内配对 CloseHandle；pid 来自发现文件，
    // 只请求查询权限，不写目标进程。
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code);
        let _ = CloseHandle(handle);
        ok != 0 && code == STILL_ACTIVE as u32
    }
}

/// Linux 读 `/proc/<pid>/stat` 第 22 字段（starttime，时钟节拍数），
/// 换算成 Unix 毫秒。读不到 ⇒ `None`。
///
/// macOS 用 `ps -o etime=` 反推（见下）；Windows 用 `GetProcessTimes` 的
/// creation FILETIME（1601 起 100ns）换算 Unix 毫秒。
/// ⚠ 审计 #18：/proc 实现只许 `target_os = "linux"` 门控（理由同 `pid_alive`）。
#[must_use]
#[cfg(target_os = "linux")]
pub fn process_start_time_ms(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // comm 字段可能含空格与括号：从最后一个 ')' 之后取字段。
    let after_comm = stat.rsplit(')').next()?;
    let fields: Vec<&str> = after_comm.split_whitespace().collect();
    // rsplit 之后字段 3（state）起算；starttime 是原第 22 字段 ⇒ 下标 19。
    let starttime_ticks: u64 = fields.get(19)?.parse().ok()?;
    let ticks_per_sec = ticks_per_second();
    let boot_ms = boot_time_ms()?;
    let start_ms = boot_ms + starttime_ticks * 1000 / ticks_per_sec;
    Some(start_ms)
}

/// macOS 没有 /proc：用 `ps -o etime=` 取进程已运行秒数，
/// 启动时刻 ≈ 当前时刻 − 已运行秒数。`etime` 是秒级分辨率，
/// 误差 < 1 s + 命令耗时，在发现文件的 2000 ms 容差内。读不到 ⇒ `None`。
#[must_use]
#[cfg(target_os = "macos")]
pub fn process_start_time_ms(pid: u32) -> Option<u64> {
    let output = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "etime="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let elapsed_secs = parse_etime_secs(&String::from_utf8_lossy(&output.stdout))?;
    Some(now_ms().saturating_sub(elapsed_secs * 1000))
}

/// 解析 BSD `ps -o etime=` 的输出：`[[dd-]hh:]mm:ss`（右对齐，含空白）。
/// 分／秒必须 < 60；形状不对 ⇒ `None`（响亮失败，不猜）。
#[cfg(any(target_os = "macos", test))]
fn parse_etime_secs(text: &str) -> Option<u64> {
    let text = text.trim();
    let (days, rest) = match text.split_once('-') {
        Some((days, rest)) => (days.parse::<u64>().ok()?, rest),
        None => (0, text),
    };
    let fields: Vec<&str> = rest.split(':').collect();
    let (hours, minutes, seconds) = match fields.as_slice() {
        [minutes, seconds] => (
            0,
            minutes.parse::<u64>().ok()?,
            seconds.parse::<u64>().ok()?,
        ),
        [hours, minutes, seconds] => (
            hours.parse::<u64>().ok()?,
            minutes.parse::<u64>().ok()?,
            seconds.parse::<u64>().ok()?,
        ),
        _ => return None,
    };
    if minutes >= 60 || seconds >= 60 {
        return None;
    }
    Some(days * 86_400 + hours * 3_600 + minutes * 60 + seconds)
}

#[must_use]
#[cfg(windows)]
pub fn process_start_time_ms(pid: u32) -> Option<u64> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // FILETIME 是 1601-01-01 起的 100ns 计数；Unix 毫秒 = ticks/10000 - 该偏移。
    const UNIX_EPOCH_FILETIME_MS: u64 = 11_644_473_600_000;
    // SAFETY: 句柄在本函数内关闭；只查询创建时间。
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut creation = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut exit = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut kernel = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut user = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let ok = GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user);
        let _ = CloseHandle(handle);
        if ok == 0 {
            return None;
        }
        let ticks = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
        Some(ticks / 10_000 - UNIX_EPOCH_FILETIME_MS)
    }
}

#[must_use]
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn pid_alive(_pid: u32) -> bool {
    false
}

#[must_use]
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn process_start_time_ms(_pid: u32) -> Option<u64> {
    None
}

/// 时钟节拍数／秒（Linux 缺省 100；读不到时用 100）。
#[cfg(target_os = "linux")]
fn ticks_per_second() -> u64 {
    // libc 不可用（std-only 依赖）；Linux 上 CLK_TCK 恒为 100。
    100
}

/// 系统启动时间（Unix 毫秒），取自 `/proc/stat` 的 `btime` 行。
#[cfg(target_os = "linux")]
fn boot_time_ms() -> Option<u64> {
    let stat = fs::read_to_string("/proc/stat").ok()?;
    for line in stat.lines() {
        if let Some(rest) = line.strip_prefix("btime ") {
            let secs: u64 = rest.trim().parse().ok()?;
            return Some(secs * 1000);
        }
    }
    None
}

/// 当前 Unix 毫秒。
#[must_use]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 生成启动 nonce：16–128 字符，`[A-Za-z0-9_-]`，每任务新鲜。
///
/// 随机源是系统时间 + 进程 id + 计数器的混合（⛔ 不是密码学随机数，
/// 但 nonce 本身只是防重放的会话绑定，不是长期密钥）。
#[must_use]
pub fn generate_nonce() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NONCE_COUNTER: AtomicU64 = AtomicU64::new(1);
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let seq = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut seed = now_ms()
        ^ (std::process::id() as u64).wrapping_mul(0x9E3779B97F4A7C15)
        ^ seq.wrapping_mul(0xBF58476D1CE4E5B9);
    let mut nonce = String::with_capacity(32);
    for _ in 0..32 {
        // xorshift64* 混洗。
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        let index = (seed.wrapping_mul(0x2545F4914F6CDD1D) >> 58) as usize % ALPHABET.len();
        nonce.push(ALPHABET[index] as char);
    }
    nonce
}

/// 校验 nonce 形状：长度 16–128，字符集 `[A-Za-z0-9_-]`。
#[must_use]
pub fn valid_nonce(nonce: &str) -> bool {
    (16..=128).contains(&nonce.len())
        && nonce
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// 校验 `job_id` 为安全单段文件名（防路径穿越：`..`、`/`、`\`、`\0` 与控制字符）。
///
/// # Errors
///
/// 若 `job_id` 为空、含路径分隔符、`..`、NUL 或 ASCII 控制字符，返回 [`OtcpError`]。
pub fn validate_job_id(job_id: &str) -> Result<(), OtcpError> {
    if job_id.is_empty()
        || job_id == "."
        || job_id == ".."
        || job_id.contains("..")
        || job_id.contains('/')
        || job_id.contains('\\')
        || job_id.bytes().any(|b| b == 0 || b < 0x20)
    {
        return Err(OtcpError::protocol(format!(
            "非法 OTCP job_id（禁止空串、路径分隔符、.. 与控制字符）: {job_id:?}"
        )));
    }
    Ok(())
}

/// 控制目录路径（任务根下的 `control/<job_id>`）。
#[must_use]
pub fn control_dir_for(control_root: &Path, job_id: &str) -> PathBuf {
    if validate_job_id(job_id).is_err() {
        return control_root.join("control").join("_invalid_job_id");
    }
    control_root.join("control").join(job_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试目录按进程唯一：同机多用户或并发测试不抢同一路径。
    fn test_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("yeban-otcp-{tag}-{}", std::process::id()))
    }

    #[test]
    fn etime_parser_reads_bsd_ps_formats() {
        // macOS `ps -o etime=` 的输出形状：[[dd-]hh:]mm:ss，右对齐带空格。
        assert_eq!(parse_etime_secs("00:00"), Some(0));
        assert_eq!(parse_etime_secs("03:25"), Some(3 * 60 + 25));
        assert_eq!(parse_etime_secs("01:02:03"), Some(3723));
        assert_eq!(parse_etime_secs("2-01:02:03"), Some(2 * 86_400 + 3723));
        assert_eq!(parse_etime_secs("  04:05\n"), Some(245));
        assert_eq!(parse_etime_secs(""), None);
        assert_eq!(parse_etime_secs("not-a-time"), None);
        assert_eq!(parse_etime_secs("1:2:3:4"), None);
        assert_eq!(parse_etime_secs("12:60"), None);
    }

    #[test]
    fn own_pid_is_alive_with_a_start_time() {
        // 平台原生进程检查的回归钉（Linux /proc、macOS kill+ps、Windows Win32）：
        // 本进程必然存活，且启动时间应在不久之前。
        let pid = std::process::id();
        assert!(pid_alive(pid), "本进程 pid {pid} 应判存活");
        let start = process_start_time_ms(pid).expect("本进程应读得到启动时间");
        let now = now_ms();
        assert!(
            start <= now && now - start < 10 * 60 * 1000,
            "启动时间应在 10 分钟内: start={start} now={now}"
        );
    }

    #[test]
    fn nonce_is_valid_shape() {
        let nonce = generate_nonce();
        assert!(valid_nonce(&nonce));
        assert_eq!(nonce.len(), 32);
        assert!(!valid_nonce("short"));
        assert!(!valid_nonce("has space in it 123456"));
        assert!(valid_nonce("a".repeat(16).as_str()));
        assert!(!valid_nonce("a".repeat(129).as_str()));
    }

    #[test]
    fn control_dir_mode_check_passes_on_0700() {
        let dir = test_dir("mode-0700");
        std::fs::create_dir_all(&dir).expect("建测试目录");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).expect("设模式");
        }
        assert!(check_control_dir_mode(&dir).is_ok());
        std::fs::remove_dir_all(&dir).expect("清理测试目录");
    }

    #[test]
    fn control_dir_mode_check_fails_on_0775() {
        let dir = test_dir("mode-0775");
        std::fs::create_dir_all(&dir).expect("建测试目录");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o775)).expect("设模式");
            let error = check_control_dir_mode(&dir).expect_err("0775 应拒");
            assert_eq!(error.reason, "otcpNotConfigured");
        }
        // 非 Unix 没有 POSIX 模式位；`check_control_dir_mode` 明确跳过检查，
        // 这里断言“应放行”，避免把 Unix 专属规则当成跨平台契约。
        #[cfg(not(unix))]
        {
            assert!(
                check_control_dir_mode(&dir).is_ok(),
                "非 Unix 平台无 0775 语义，应 Ok"
            );
        }
        std::fs::remove_dir_all(&dir).expect("清理测试目录");
    }

    #[test]
    fn stale_discovery_rejected_when_pid_dead() {
        let dir = test_dir("stale-disc");
        std::fs::create_dir_all(&dir).expect("建测试目录");
        let path = dir.join("999999.json");
        let file = DiscoveryFile {
            port: 41234,
            pid: 999_999,
            launch_nonce: Some("valid-nonce-0123456789".to_owned()),
            process_start_time_ms: now_ms(),
            token: "t".to_owned(),
            server_proof: "p".to_owned(),
            instance_id: "i".to_owned(),
        };
        std::fs::write(&path, serde_json::to_string(&file).expect("序列化")).expect("写文件");
        let error = read_discovery_file(&path, "valid-nonce-0123456789").expect_err("死 pid 应拒");
        assert!(error.message.contains("已死"));
        std::fs::remove_dir_all(&dir).expect("清理测试目录");
    }

    #[test]
    fn discovery_redacted_debug_hides_secrets() {
        let file = DiscoveryFile {
            port: 41234,
            pid: 1234,
            launch_nonce: Some("nonce-secret".to_owned()),
            process_start_time_ms: 1_000_000,
            token: "token-secret".to_owned(),
            server_proof: "proof-secret".to_owned(),
            instance_id: "inst-1".to_owned(),
        };
        let debug = file.redacted_debug();
        assert!(!debug.contains("nonce-secret"));
        assert!(!debug.contains("token-secret"));
        assert!(!debug.contains("proof-secret"));
        assert!(debug.contains("inst-1"));
    }

    #[test]
    fn wait_ready_file_times_out() {
        let dir = test_dir("no-ready");
        std::fs::create_dir_all(&dir).expect("建测试目录");
        let error =
            wait_ready_file(&dir, Duration::from_millis(300)).expect_err("无 ready.json 应超时");
        assert!(error.message.contains("超时"));
        std::fs::remove_dir_all(&dir).expect("清理测试目录");
    }

    #[test]
    fn validate_job_id_rejects_traversal_and_separators() {
        assert!(validate_job_id("job-123_abc").is_ok());
        assert!(validate_job_id("").is_err());
        assert!(validate_job_id("..").is_err());
        assert!(validate_job_id("../etc").is_err());
        assert!(validate_job_id("a/b").is_err());
        assert!(validate_job_id("a\\b").is_err());
        assert!(validate_job_id("bad\0id").is_err());
        assert_ne!(generate_nonce(), generate_nonce());
    }
}
