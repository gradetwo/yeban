//! 服务端监管：启动、健康检查、关闭。
//!
//! 生命周期（设计稿 §2.1）：
//! * 任务临时目录在控制根下，子目录先建好并校验 0700；
//! * 启动：`OpenTune --control-api --control-api-dir=<dir>`
//!   `--control-api-ready-file=<dir>/ready.json`
//!   `--control-api-launch-nonce=<nonce>`，`OPENTUNE_DATA_DIR` 指向任务临时目录；
//! * 等 `ready.json` ≤ 30 s → 读 `<pid>.json` → 校验 nonce / pid 存活 /
//!   启动时间（陈旧即不连、可删）；
//! * TCP 连字面量 `127.0.0.1:<port>` → 2000 ms 内发 `session.hello`，
//!   校验 `instanceId` / `serverProof` / `pid` 与发现文件一致；
//! * `session.info` 门：必需能力与构建提交号，否则闭合失败
//!   `otcpUnsupportedBuild` 并点名实测提交号；
//! * 关闭（每条路径都走）：取消未终态任务 → 等终态 → `ui.getState` +
//!   解散模态框 → `app.quit` → 等退出 ≤ 10 s → SIGTERM → 等 ≤ 5 s →
//!   SIGKILL → 回收 → 删任务临时目录。
//!
//! ⛔ std-only：没有 `libc` 依赖，SIGTERM 走 `/bin/kill -TERM <pid>`
//! （best-effort；失败时直接 SIGKILL），SIGKILL 用 `Child::kill()`。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::client::OtcpClient;
use crate::config::{DisplayConfig, OtcpConfig};
use crate::discovery::{
    DiscoveryFile, check_control_dir_mode, control_dir_for, generate_nonce, read_discovery_file,
    wait_ready_file,
};
use crate::error::OtcpError;

/// `session.info` 要求的能力（设计稿 §2.1；`prefs.write` 只在请求
/// prefs 操作时才要求，v1 只改 `tuningHz`，故不在启动门里）。
pub const REQUIRED_CAPABILITIES: &[&str] = &[
    "session.v1",
    "read.v1",
    "jobs.v1",
    "import.v1",
    "export.v1",
    "ui.bridge",
    "edit.v1",
];

/// 自测记录里认可的最低构建提交（设计稿 §2.1：9ef45f9 或更新且过自测）。
///
/// ⚠ 上游 API 分支移动快，提交号只作**下限提示**，不作硬门：
/// 真正的门是自测记录（§6.3）——构建提交号只在自测记录里点名，
/// 不凭号段放行或拒绝。
pub const MIN_BUILD_COMMIT: &str = "9ef45f9";

/// 已启动并握手完成的服务端会话。
pub struct ServerSession {
    /// 客户端（已完成 `session.hello`）。
    pub client: OtcpClient,
    /// 发现文件（去秘密化后只留端口/pid/实例 id 供日志）。
    pub discovery: DiscoveryFile,
    /// `session.info` 的 `build.commit`。
    pub build_commit: String,
    /// `session.info` 的 `processing` 块（原样，供收据记录）。
    pub processing: Value,
    /// `session.info` 的 `editingScheme`。
    pub editing_scheme: String,
    /// 子进程句柄（Drop 时兜底杀掉）。
    child: Option<Child>,
    /// 任务临时目录（Drop 时删除）。
    job_temp_dir: PathBuf,
    /// 控制目录（任务根下的 `control/<job_id>`）。
    control_dir: PathBuf,
}

impl ServerSession {
    /// 进程 pid。
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.discovery.pid
    }

    /// 实例 id（⛔ 不是秘密）。
    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.discovery.instance_id
    }
}

impl Drop for ServerSession {
    fn drop(&mut self) {
        // 兜底：子进程还在就 SIGKILL，临时目录删除。
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.job_temp_dir);
        let _ = fs::remove_dir_all(&self.control_dir);
    }
}

/// 启动一个服务端会话（一个任务一个服务端，⛔ 不共享）。
///
/// `job_id` 是 yeban 任务 id（同时作 `import.start` 的 `idempotencyKey`）。
pub fn start_server(config: &OtcpConfig, job_id: &str) -> Result<ServerSession, OtcpError> {
    let bin = &config.opentune_bin;
    if !bin.exists() {
        return Err(OtcpError::with_data(
            "otcpNotConfigured",
            format!(
                "OpenTune 二进制不存在：{}（用户自备 API 分支构建；普通 v1.8.2 发行版无 control API）",
                bin.display()
            ),
            json!({"binary": bin.display().to_string()}),
        ));
    }
    let control_dir = control_dir_for(&config.control_root, job_id);
    fs::create_dir_all(&control_dir).map_err(|error| {
        OtcpError::io(format!(
            "建控制目录 {} 失败: {error}",
            control_dir.display()
        ))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&control_dir, fs::Permissions::from_mode(0o700)).map_err(|error| {
            OtcpError::io(format!(
                "设控制目录 {} 模式失败: {error}",
                control_dir.display()
            ))
        })?;
    }
    check_control_dir_mode(&control_dir)?;

    let job_temp_dir = config.job_tmp_dir(job_id);
    fs::create_dir_all(&job_temp_dir).map_err(|error| {
        OtcpError::io(format!(
            "建任务临时目录 {} 失败: {error}",
            job_temp_dir.display()
        ))
    })?;

    let nonce = generate_nonce();
    let ready_file = control_dir.join("ready.json");
    let mut command = Command::new(bin);
    command
        .arg("--control-api")
        .arg(format!("--control-api-dir={}", control_dir.display()))
        .arg(format!("--control-api-ready-file={}", ready_file.display()))
        .arg(format!("--control-api-launch-nonce={nonce}"))
        .env("OPENTUNE_DATA_DIR", &job_temp_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    apply_display(config, &mut command);
    let mut child = command.spawn().map_err(|error| {
        OtcpError::with_data(
            "otcpNotConfigured",
            format!("启动 OpenTune 失败（{}）: {error}", bin.display()),
            json!({"binary": bin.display().to_string()}),
        )
    })?;

    let session = match start_server_inner(config, job_id, &control_dir, &job_temp_dir, &nonce) {
        Ok(session) => session,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    // 内层已接管子进程的 Drop 兜底：把句柄交给会话。
    let mut session = session;
    session.child = Some(child);
    Ok(session)
}

fn start_server_inner(
    config: &OtcpConfig,
    job_id: &str,
    control_dir: &Path,
    job_temp_dir: &Path,
    nonce: &str,
) -> Result<ServerSession, OtcpError> {
    let ready = wait_ready_file(control_dir, Duration::from_secs(config.timeouts.ready_secs))?;
    // ready 文件只含 {pid, instanceId}（PROTOCOL §5.5）；发现文件即
    // <control_dir>/<pid>.json，端口在发现文件里，不在 ready 文件里。
    let discovery_path = control_dir.join(format!("{}.json", ready.pid));
    let discovery = read_discovery_file(&discovery_path, nonce)?;
    if discovery.instance_id != ready.instance_id {
        return Err(OtcpError::protocol(format!(
            "发现文件的 instanceId 与 ready 文件不符（陈旧文件）：{} vs {}",
            discovery.instance_id, ready.instance_id
        )));
    }
    if discovery.pid == 0 {
        return Err(OtcpError::protocol("发现文件的 pid 为 0"));
    }
    let mut client = OtcpClient::connect(
        discovery.port,
        Duration::from_millis(config.timeouts.hello_ms * 5),
    )?;
    // session.hello：连接后 2000 ms 内必须发出（spec 固定值）。
    let hello = client.call(
        "session.hello",
        json!({
            "token": discovery.token,
            "protocol": "1.0",
            "client": {"name": "yeban-otcp", "version": env!("CARGO_PKG_VERSION")}
        }),
        job_id,
    )?;
    verify_hello(&hello, &discovery)?;
    let info = client.call("session.info", json!({}), job_id)?;
    let (build_commit, processing, editing_scheme) = gate_session_info(&info, job_id)?;
    Ok(ServerSession {
        client,
        discovery,
        build_commit,
        processing,
        editing_scheme,
        child: None,
        job_temp_dir: job_temp_dir.to_owned(),
        control_dir: control_dir.to_owned(),
    })
}

/// 校验 `session.hello` 的回显与发现文件一致。
fn verify_hello(hello: &Value, discovery: &DiscoveryFile) -> Result<(), OtcpError> {
    let instance_id = hello
        .get("instanceId")
        .and_then(Value::as_str)
        .unwrap_or("");
    if instance_id != discovery.instance_id {
        return Err(OtcpError::protocol(format!(
            "session.hello 的 instanceId 与发现文件不符：{} vs {}",
            instance_id, discovery.instance_id
        )));
    }
    let pid = hello.get("pid").and_then(Value::as_u64).unwrap_or(0);
    if pid != u64::from(discovery.pid) {
        return Err(OtcpError::protocol(format!(
            "session.hello 的 pid 与发现文件不符：{pid} vs {}",
            discovery.pid
        )));
    }
    let proof = hello
        .get("serverProof")
        .and_then(Value::as_str)
        .unwrap_or("");
    if proof != discovery.server_proof {
        return Err(OtcpError::protocol(
            "session.hello 的 serverProof 与发现文件不符",
        ));
    }
    Ok(())
}

/// `session.info` 门：必需能力齐全；构建提交号非空。
///
/// 返回 `(build_commit, processing, editing_scheme)`。
/// 能力缺失 ⇒ `otcpUnsupportedBuild` 并点名实测提交号。
fn gate_session_info(info: &Value, job_id: &str) -> Result<(String, Value, String), OtcpError> {
    let build_commit = info
        .get("build")
        .and_then(|b| b.get("commit"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    if build_commit.is_empty() {
        return Err(OtcpError::protocol("session.info 缺 build.commit"));
    }
    let capabilities: Vec<String> = info
        .get("capabilities")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let missing: Vec<&str> = REQUIRED_CAPABILITIES
        .iter()
        .copied()
        .filter(|cap| !capabilities.iter().any(|c| c == cap))
        .collect();
    if !missing.is_empty() {
        return Err(OtcpError::with_data(
            "otcpUnsupportedBuild",
            format!(
                "OpenTune 构建 {build_commit} 缺必需能力：{}（须为 API 分支构建，普通 v1.8.2 发行版无 control API）",
                missing.join(", ")
            ),
            json!({
                "build_commit": build_commit,
                "missing_capabilities": missing,
                "job_id": job_id,
            }),
        ));
    }
    let processing = info.get("processing").cloned().unwrap_or(Value::Null);
    let editing_scheme = info
        .get("editingScheme")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    Ok((build_commit, processing, editing_scheme))
}

/// 显示配置落到启动命令上。
///
/// * `Inherit`：`DISPLAY` 已设时原样继承；未设时包 `xvfb-run`（须实测，
///   见设计稿 §9 开放问题 1——Xvfb 下 `ui.getState` 须可用）；
/// * `Explicit(display)`：设 `DISPLAY=<display>`；
/// * `Xvfb`：包 `xvfb-run`。
fn apply_display(config: &OtcpConfig, command: &mut Command) {
    match &config.display {
        DisplayConfig::Inherit => {
            if std::env::var_os("DISPLAY").is_none() {
                wrap_xvfb(command);
            }
        }
        DisplayConfig::Explicit(display) => {
            command.env("DISPLAY", display);
        }
        DisplayConfig::Xvfb => wrap_xvfb(command),
    }
}

/// 把命令包进 `xvfb-run -a`（原命令变 `xvfb-run` 的参数）。
fn wrap_xvfb(command: &mut Command) {
    let program = command.get_program().to_owned();
    let mut args: Vec<std::ffi::OsString> = vec!["-a".into()];
    args.push(program);
    args.extend(command.get_args().map(ToOwned::to_owned));
    *command = Command::new("xvfb-run");
    command.args(args);
}

/// 关闭会话（每条路径都走）：取消未终态任务 → 等终态 →
/// `ui.getState` + 解散模态框 → `app.quit` → 等退出 ≤ 10 s →
/// SIGTERM → 等 ≤ 5 s → SIGKILL → 回收 → 删任务临时目录。
///
/// `dismiss_modal` 由调用方传入（recovery 模块的闭包），以免循环依赖。
pub fn shutdown_server(
    session: &mut ServerSession,
    config: &OtcpConfig,
    job_ids: &[String],
    dismiss_modal: &dyn Fn(&mut OtcpClient, &str) -> Result<(), OtcpError>,
) {
    let job_id = job_ids.first().cloned().unwrap_or_default();
    // 1. 取消未终态任务（best-effort）。
    for id in job_ids {
        let _ = session
            .client
            .call("job.cancel", json!({"jobId": id}), &job_id);
    }
    // 2. 等任务终态（最多 5 秒，best-effort）。
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let mut all_terminal = true;
        for id in job_ids {
            if let Ok(state) = session
                .client
                .call("job.get", json!({"jobId": id}), &job_id)
            {
                let state = state
                    .get("job")
                    .and_then(|j| j.get("state"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if !matches!(state, "succeeded" | "failed" | "cancelled") {
                    all_terminal = false;
                }
            }
        }
        if all_terminal {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    // 3. 解散模态框（best-effort：失败也不拦住退出）。
    let _ = dismiss_modal(&mut session.client, &job_id);
    // 4. app.quit。
    let quit = session
        .client
        .call("app.quit", json!({"discardUnsaved": true}), &job_id);
    // 5. 等进程退出 ≤ 10 s。
    let quit_deadline = Instant::now() + Duration::from_secs(config.timeouts.quit_wait_secs);
    let mut exited = false;
    if quit.is_ok() {
        while Instant::now() < quit_deadline {
            if !crate::discovery::pid_alive(session.discovery.pid) {
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    // 6. SIGTERM（best-effort）→ 等 ≤ 5 s → 7. SIGKILL（Child::kill）。
    if !exited {
        send_sigterm(session.discovery.pid);
        let term_deadline = Instant::now() + Duration::from_secs(config.timeouts.term_wait_secs);
        while Instant::now() < term_deadline {
            if !crate::discovery::pid_alive(session.discovery.pid) {
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    if !exited {
        if let Some(mut child) = session.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    } else if let Some(mut child) = session.child.take() {
        let _ = child.wait();
    }
    // 8. 删任务临时目录与控制目录。
    let _ = fs::remove_dir_all(&session.job_temp_dir);
    let _ = fs::remove_dir_all(&session.control_dir);
}

/// 发 SIGTERM（`kill -TERM <pid>`，best-effort；失败即跳过）。
fn send_sigterm(pid: u32) {
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// 写一行脱敏日志（token / serverProof 已打码）。
///
/// 本 crate 的日志只走这个函数；⛔ 不直接 `eprintln!` 原始 JSON。
pub fn log_redacted(event: &str, value: &Value) {
    let redacted = crate::client::redact(value);
    eprintln!("[yeban-otcp] {event}: {redacted}");
}

/// 写一行纯文本日志。
pub fn log_line(line: &str) {
    let mut stderr = std::io::stderr();
    let _ = writeln!(stderr, "[yeban-otcp] {line}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_info_gate_rejects_missing_capability() {
        let info = json!({
            "build": {"commit": "abc123"},
            "capabilities": ["session.v1", "read.v1"],
            "processing": {},
            "editingScheme": "openDyne"
        });
        let error = gate_session_info(&info, "j1").expect_err("缺能力应拒");
        assert_eq!(error.reason, "otcpUnsupportedBuild");
        assert!(error.message.contains("abc123"));
        assert!(error.message.contains("jobs.v1"));
    }

    #[test]
    fn session_info_gate_accepts_full_capabilities() {
        let info = json!({
            "build": {"commit": "abc123"},
            "capabilities": ["session.v1", "read.v1", "jobs.v1", "import.v1", "export.v1", "ui.bridge", "edit.v1"],
            "processing": {"backend": "cpu"},
            "editingScheme": "openDyne"
        });
        let (commit, processing, scheme) = gate_session_info(&info, "j1").expect("全能力应过");
        assert_eq!(commit, "abc123");
        assert_eq!(processing["backend"], "cpu");
        assert_eq!(scheme, "openDyne");
    }

    #[test]
    fn verify_hello_rejects_mismatched_instance() {
        let discovery = DiscoveryFile {
            port: 41234,
            pid: 1234,
            launch_nonce: Some("n".to_owned()),
            process_start_time_ms: 1,
            token: "t".to_owned(),
            server_proof: "p".to_owned(),
            instance_id: "inst-1".to_owned(),
        };
        let hello = json!({
            "instanceId": "inst-2", "pid": 1234, "serverProof": "p"
        });
        assert!(verify_hello(&hello, &discovery).is_err());
        let hello = json!({
            "instanceId": "inst-1", "pid": 1234, "serverProof": "p"
        });
        assert!(verify_hello(&hello, &discovery).is_ok());
    }

    #[test]
    fn start_server_refuses_missing_binary() {
        let config = OtcpConfig {
            opentune_bin: PathBuf::from("/nonexistent/OpenTune-xyz"),
            control_root: std::env::temp_dir().join("yeban-otcp-no-bin"),
            ..OtcpConfig::default()
        };
        let error = match start_server(&config, "job-test") {
            Ok(_) => panic!("缺二进制应拒"),
            Err(error) => error,
        };
        assert_eq!(error.reason, "otcpNotConfigured");
    }
}
