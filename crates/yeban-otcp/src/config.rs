//! 驱动配置：二进制路径、任务根、显示、超时、自测记录。
//!
//! 来源优先级：`YEBAN_OPENTUNE_BIN` 环境变量 ＞ 配置文件 ＞
//! `~/.local/bin/OpenTune`。配置文件的默认位置是控制根下的
//! `config.json`（控制根本身缺省为系统临时目录下的 `yeban-otcp/`）。
//!
//! ⚠ 许可边界：这里只记录**用户自备构建的路径**，⛔ 不打包、不分发
//! OpenTune 二进制与声码器权重（见 crate 文档）。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 超时（毫秒／秒，见设计稿 §2）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtcpTimeouts {
    /// `session.hello` 必须在连接后 2000 ms 内发出（spec 固定值）。
    pub hello_ms: u64,
    /// `ready.json` 等待上限（秒）。
    pub ready_secs: u64,
    /// F0 等待预算（秒）。
    pub f0_wait_secs: u64,
    /// 导出等待预算（秒）。
    pub export_wait_secs: u64,
    /// 单次 `job.get` 长轮询上限（毫秒，spec 上限 30000）。
    pub job_poll_ms: u64,
    /// 整任务预算（秒）。
    pub job_budget_secs: u64,
    /// `app.quit` 后等进程退出的上限（秒）。
    pub quit_wait_secs: u64,
    /// SIGTERM 后等进程退出的上限（秒）。
    pub term_wait_secs: u64,
}

impl Default for OtcpTimeouts {
    fn default() -> Self {
        Self {
            hello_ms: 2000,
            ready_secs: 30,
            f0_wait_secs: 180,
            export_wait_secs: 120,
            job_poll_ms: 30_000,
            job_budget_secs: 600,
            quit_wait_secs: 10,
            term_wait_secs: 5,
        }
    }
}

/// 显示配置（设计稿 §1.4：模态框恢复需要编辑器窗口）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisplayConfig {
    /// 继承进程的 `DISPLAY`（未设时由 supervisor 决定是否包 `xvfb-run`）。
    #[default]
    Inherit,
    /// 显式 X 显示（如 `:1`）。
    Explicit(String),
    /// 用 `xvfb-run` 包起来跑（须实测 Xvfb 下 `ui.getState` 可用）。
    Xvfb,
}

/// 驱动配置。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OtcpConfig {
    /// OpenTune Standalone 二进制（API 分支构建，含 control API）。
    pub opentune_bin: PathBuf,
    /// 任务根目录（每任务一个子目录，模式 0700）。
    pub control_root: PathBuf,
    /// 显示配置。
    pub display: DisplayConfig,
    /// 最大并行校正任务数（缺省 2）。
    pub max_concurrent_jobs: usize,
    /// 超时表。
    pub timeouts: OtcpTimeouts,
    /// A4 频率（Hz，缺省 440）。
    pub tuning_hz: f32,
    /// 量测门：导出文件相对音符目标的中位 |偏差| 上限（音分，缺省 50）。
    pub measure_gate_cents: f32,
}

impl Default for OtcpConfig {
    fn default() -> Self {
        Self {
            opentune_bin: default_bin_path(),
            control_root: std::env::temp_dir().join("yeban-otcp"),
            display: DisplayConfig::default(),
            max_concurrent_jobs: 2,
            timeouts: OtcpTimeouts::default(),
            tuning_hz: 440.0,
            measure_gate_cents: 50.0,
        }
    }
}

/// 缺省二进制路径：`YEBAN_OPENTUNE_BIN` ＞ `~/.local/bin/OpenTune`。
#[must_use]
pub fn default_bin_path() -> PathBuf {
    if let Some(path) = std::env::var("YEBAN_OPENTUNE_BIN")
        .ok()
        .filter(|path| !path.is_empty())
    {
        return PathBuf::from(path);
    }
    home_dir().join(".local").join("bin").join("OpenTune")
}

/// 用户家目录（`HOME` 环境变量；取不到时退回当前目录）。
#[must_use]
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

impl OtcpConfig {
    /// 自测记录文件（控制根下的 `self-test.json`）。
    #[must_use]
    pub fn self_test_path(&self) -> PathBuf {
        self.control_root.join("self-test.json")
    }

    /// 任务目录（控制根下的 `jobs/<job_id>`；收据与导出文件住这里，
    /// 关闭服务端时⛔ 不删——MCP 层稍后读它做交付）。
    #[must_use]
    pub fn job_dir(&self, job_id: &str) -> PathBuf {
        self.control_root.join("jobs").join(job_id)
    }

    /// 服务端临时目录（控制根下的 `tmp/<job_id>`；`OPENTUNE_DATA_DIR`
    /// 与自测夹具住这里，关闭服务端时删除）。
    #[must_use]
    pub fn job_tmp_dir(&self, job_id: &str) -> PathBuf {
        self.control_root.join("tmp").join(job_id)
    }

    /// 任务收据路径（任务目录下的 `receipt.json`）。
    #[must_use]
    pub fn receipt_path(&self, job_id: &str) -> PathBuf {
        self.job_dir(job_id).join("receipt.json")
    }

    /// 片段 → 任务索引文件（控制根下的 `clip-index.json`）。
    #[must_use]
    pub fn clip_index_path(&self) -> PathBuf {
        self.control_root.join("clip-index.json")
    }
}

/// 自测记录（设计稿 §6.3）：构建是否含无声间隙修补。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelfTestRecord {
    /// 自测时所见的构建提交号（`session.info` 的 `build.commit`）。
    pub build_commit: String,
    /// 无声间隙导出：`pass` = 修补已在构建里，`fail` = 缺陷仍在，
    /// `unknown` = 自测未跑过或未完成。
    pub unvoiced_gap: String,
    /// 自测完成时间（UTC，ISO 8601）。
    pub at: String,
}

impl SelfTestRecord {
    /// 未跑过的记录。
    #[must_use]
    pub fn unknown() -> Self {
        Self {
            build_commit: String::new(),
            unvoiced_gap: "unknown".to_owned(),
            at: String::new(),
        }
    }

    /// 该记录是否对应当前构建（提交号相同且已跑过）。
    #[must_use]
    pub fn matches_build(&self, build_commit: &str) -> bool {
        !build_commit.is_empty()
            && self.build_commit == build_commit
            && self.unvoiced_gap != "unknown"
    }
}

/// 驱动状态（状态工具一次返回：构建提交 + 自测 + 可达性）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DriverStatus {
    /// 二进制是否存在且可执行。
    pub binary_present: bool,
    /// 二进制路径（原样回显，不展开）。
    pub binary_path: PathBuf,
    /// 自测记录（未跑过时 `unvoiced_gap` 为 `unknown`）。
    pub self_test: SelfTestRecord,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_design_defaults() {
        let config = OtcpConfig::default();
        assert_eq!(config.max_concurrent_jobs, 2);
        assert_eq!(config.tuning_hz, 440.0);
        assert_eq!(config.measure_gate_cents, 50.0);
        assert_eq!(config.timeouts.f0_wait_secs, 180);
        assert_eq!(config.timeouts.export_wait_secs, 120);
        assert_eq!(config.timeouts.job_budget_secs, 600);
    }

    #[test]
    fn bin_path_prefers_env() {
        // SAFETY: 本测试独占该环境变量（其它测试不读它），且在断言后立即清除；
        // 单进程内测试并行时，读到该变量的其它测试只影响缺省路径，不影响断言。
        unsafe {
            std::env::set_var("YEBAN_OPENTUNE_BIN", "/opt/ot/OpenTune");
        }
        assert_eq!(default_bin_path(), PathBuf::from("/opt/ot/OpenTune"));
        // SAFETY: 同上，测试结束时清除该变量，不留给其它测试。
        unsafe {
            std::env::remove_var("YEBAN_OPENTUNE_BIN");
        }
    }

    #[test]
    fn self_test_record_matches_only_same_build() {
        let record = SelfTestRecord {
            build_commit: "abc123".to_owned(),
            unvoiced_gap: "pass".to_owned(),
            at: "2026-11-11T00:00:00Z".to_owned(),
        };
        assert!(record.matches_build("abc123"));
        assert!(!record.matches_build("def456"));
        assert!(!SelfTestRecord::unknown().matches_build("abc123"));
    }
}
