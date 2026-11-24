//! OS 高优先级实时线程调度（[ROAD-M2-001]，T-FIX-93 第 1 项，2026-11-24）。
//!
//! ## 依赖裁决（本模块的存在前提）
//!
//! 管理侧 T-FIX-93 任务单明文准许为此引入 `libc` 依赖（DAW 惯例：Logic 等均设
//! 实时优先级），解锁了 `docs/ledger/engine-rt-notes.md` §5.2 needs N3 的
//! 「依赖图 + 平台 unsafe 审计」裁决项。本模块因此是引擎内**唯一**的平台 `unsafe`
//! 系统调用面：Linux 走 `libc::sched_setscheduler`，macOS 走 Mach
//! `thread_policy_set`（extern 声明见下方审计注记）。
//!
//! ## 平台覆盖矩阵（覆盖不了的逐项写明）
//!
//! | 平台 | 机制 | 状态 |
//! | :--- | :--- | :--- |
//! | Linux | `sched_setscheduler(0, SCHED_FIFO/SCHED_RR, …)` | ✅ 本模块实现；需要 `CAP_SYS_NICE` 或 `RLIMIT_RTPRIO > 0`，容器/沙箱通常 `EPERM` ⇒ 结构化 [`RtPriorityFailure::PermissionDenied`] 响亮返回（附 hint），**绝不静默** |
//! | macOS | `thread_policy_set(THREAD_TIME_CONSTRAINT_POLICY)` | ✅ 本模块实现；**要求调用方显式给** [`MacosTimeConstraint`]（period/computation/constraint 微秒，XNU 要求 `computation ≤ constraint ≤ period`，违者 [`RtPriorityFailure::MacosInvalidDurations`] 先于系统调用响亮拒绝）。运行时只在 macOS 机器上验证过（本节点无 macOS）；**编译覆盖由 CI 的 macOS 腿保证** |
//! | Windows / 其它 | 无本模块路径 | ⛔ [`RtPriorityFailure::UnsupportedPlatform`]；cpal 0.18 的 `realtime` feature 覆盖 WASAPI/AAudio/PipeWire/JACK（见 `lib.rs` 缺口注记与 `device` 模块文档） |
//!
//! ## Mach extern 审计注记（macOS 腿）
//!
//! `libc` crate 不保证导出全部 Mach 策略 API，本模块**自声明且仅自声明两个符号**：
//! `mach_thread_self` 与 `thread_policy_set`（签名按 `mach/thread_policy.h` 公开 ABI：
//! `kern_return_t thread_policy_set(thread_port_t, thread_policy_flavor_t,
//! thread_policy_t, mach_msg_type_number_t)`，`integer_t = i32`、端口即 `u32`）。
//! 策略结构 [`build_time_constraint`] 是**纯函数**，在任意平台可单测。
//! 已知取舍：`mach_thread_self()` 每次调用返回一个新的 send right，本模块**不做**
//! `mach_port_deallocate`（省掉两个额外 extern 与配套 unsafe；每次调用泄漏一个
//! 端口权限，音频线程生命周期内调用一次量级，量有界且不累积增长 —— 审计知情取舍，
//! 不算静默失败）。
//!
//! ## 线程覆盖边界（为什么引擎不在内部自动调用）
//!
//! cpal 0.18 自有 ALSA/WASAPI 回调线程，**没有注入钩子**，引擎无法给 cpal 的
//! 回调线程设优先级；默认墙钟泵送形态跑在宿主线程上，引擎库对宿主当前线程
//! 悄悄改调度策略属于意外副作用。因此本模块只提供**显式 API**：宿主（app 的
//! `engine_host` / TUI 的泵送线程 / 未来的 cpal 替代宿主）在**自己拥有的**音频
//! 相关线程入口自行调用并把结构化失败写进响亮报告。本 crate 现有线程（如
//! `rt.rs` 的抽干线程）不属于 RT 关键路径，不接入。
//!
//! ## 判据
//!
//! - [`build_time_constraint`] 纯函数校验（任意平台可测）：时长关系违例先于系统调用响亮拒绝。
//! - Linux 合法规格实测：非特权环境得 `PermissionDenied{os_error: EPERM}` + 非空 hint
//!   （结构化、不 panic、不静默）；特权环境得 `Ok(())` —— 两种结果都是正确行为。
//! - 非法优先级在任何系统调用之前被拒（[`RtPriorityFailure::InvalidPriority`]）。

use thiserror::Error;

/// Linux 实时调度策略（macOS 腿忽略此字段）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RtScheduler {
    /// `SCHED_FIFO`：先进先出实时调度。
    Fifo,
    /// `SCHED_RR`：时间片轮转实时调度。
    RoundRobin,
}

#[cfg(target_os = "linux")]
impl RtScheduler {
    fn linux_policy(self) -> i32 {
        match self {
            RtScheduler::Fifo => libc::SCHED_FIFO,
            RtScheduler::RoundRobin => libc::SCHED_RR,
        }
    }
}

/// macOS `THREAD_TIME_CONSTRAINT_POLICY` 的显式时长参数（微秒）。
///
/// XNU 的硬性关系：`computation ≤ constraint ≤ period` 且 `period > 0`；
/// 违例在进入系统调用**之前**被 [`validate_time_constraint`] 响亮拒绝。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacosTimeConstraint {
    /// 周期（µs）；音频回调场景 = 缓冲区时长。
    pub period_us: u32,
    /// 每周期保证的计算时间（µs）；不得超过 [`Self::constraint_us`]。
    pub computation_us: u32,
    /// 每周期的完成期限（µs）；不得超过 [`Self::period_us`]。
    pub constraint_us: u32,
    /// 是否可被其它线程抢占（音频回调惯例 `false`）。
    pub preemptible: bool,
}

/// 实时优先级规格。
///
/// Linux 用 `scheduler` + `priority`（1..=99，范围以 `sched_get_priority_min/max`
/// 实测为准）；macOS 用 `macos`（未给则响亮拒绝，不发明缺省时长）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RtPrioritySpec {
    /// Linux 实时调度策略（macOS 腿忽略）。
    pub scheduler: RtScheduler,
    /// 实时优先级；Linux 合法范围 1..=99（以 sched_get_priority_min/max 实测为准）。
    pub priority: i32,
    /// macOS 腿的显式时长参数；未给则在 macOS 上响亮拒绝（不发明缺省值）。
    pub macos: Option<MacosTimeConstraint>,
}

/// 结构化失败（响亮语义：调用方拿到枚举就能写报告，无 panic、无静默降级）。
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RtPriorityFailure {
    /// 优先级超出平台合法范围（在任何系统调用之前拒绝）。
    #[error("invalid realtime priority {priority} (valid {valid_min}..={valid_max})")]
    InvalidPriority {
        /// 被拒的优先级值。
        priority: i32,
        /// 平台实测合法下界。
        valid_min: i32,
        /// 平台实测合法上界。
        valid_max: i32,
    },
    /// macOS 腿没有显式 time-constraint 参数（本模块不发明缺省时长）。
    #[error(
        "macOS leg requires explicit MacosTimeConstraint durations; refusing to invent defaults"
    )]
    MacosTimeConstraintRequired,
    /// macOS 时长关系违例（`computation ≤ constraint ≤ period` 且 `period > 0`）。
    #[error("invalid macOS time-constraint durations: {detail}")]
    MacosInvalidDurations {
        /// 违例说明（指明哪条关系被破坏）。
        detail: &'static str,
    },
    /// 内核拒绝（权限类：Linux 需要 `CAP_SYS_NICE` 或 `RLIMIT_RTPRIO > 0`；
    /// 容器/沙箱通常 `EPERM`）。`hint` 是给运维/报告的响亮指引。
    #[error("realtime priority denied (os_error={os_error}): {hint}")]
    PermissionDenied {
        /// 原始 errno（Linux 典型 `EPERM`/`EACCES`）。
        os_error: i32,
        /// 给运维/报告的响亮指引（点名权限来源与解锁路径）。
        hint: &'static str,
    },
    /// 其它内核错误（`stage` 指明是哪个调用）。
    #[error("realtime priority syscall failed at {stage} (os_error={os_error})")]
    OsError {
        /// 原始 errno（macOS 腿为 kern_return_t）。
        os_error: i32,
        /// 发生错误的调用名。
        stage: &'static str,
    },
    /// 平台没有本模块路径（Windows / 其它；见平台覆盖矩阵）。
    #[error(
        "realtime priority not implemented on {platform}; cpal's realtime feature covers WASAPI/AAudio/PipeWire/JACK there"
    )]
    UnsupportedPlatform {
        /// 平台标识（std::env::consts::OS）。
        platform: &'static str,
    },
}

/// 把 macOS 显式时长编译成 `thread_time_constraint_policy` 的 4 个 `integer_t`
/// 字段（period / computation / constraint / preemptible），**纯函数**、先校验后构造。
pub fn validate_time_constraint(tc: &MacosTimeConstraint) -> Result<[i32; 4], RtPriorityFailure> {
    if tc.period_us == 0 {
        return Err(RtPriorityFailure::MacosInvalidDurations {
            detail: "period_us must be > 0",
        });
    }
    if tc.computation_us > tc.constraint_us {
        return Err(RtPriorityFailure::MacosInvalidDurations {
            detail: "computation_us must be <= constraint_us",
        });
    }
    if tc.constraint_us > tc.period_us {
        return Err(RtPriorityFailure::MacosInvalidDurations {
            detail: "constraint_us must be <= period_us",
        });
    }
    Ok([
        tc.period_us as i32,
        tc.computation_us as i32,
        tc.constraint_us as i32,
        if tc.preemptible { 1 } else { 0 },
    ])
}

/// 把**当前线程**设为实时优先级（失败结构化返回，绝不 panic、绝不静默）。
///
/// Linux：`sched_setscheduler(0, policy, …)`；macOS：`thread_policy_set`
/// （`THREAD_TIME_CONSTRAINT_POLICY`）；其它平台：[`RtPriorityFailure::UnsupportedPlatform`]。
pub fn set_current_thread_realtime(spec: &RtPrioritySpec) -> Result<(), RtPriorityFailure> {
    #[cfg(target_os = "linux")]
    {
        let policy = spec.scheduler.linux_policy();
        let (valid_min, valid_max) = unsafe {
            // SAFETY: sched_get_priority_min/max 是无状态的纯查询 syscall，
            // 参数为编译期常量（policy 来自枚举映射，只会是 SCHED_FIFO/SCHED_RR），
            // 不解引用任何指针，不可能违反内存安全。
            (
                libc::sched_get_priority_min(policy),
                libc::sched_get_priority_max(policy),
            )
        };
        if valid_min < 0 || valid_max < 0 {
            return Err(RtPriorityFailure::OsError {
                os_error: valid_min.min(valid_max),
                stage: "sched_get_priority_min/max",
            });
        }
        if spec.priority < valid_min || spec.priority > valid_max {
            return Err(RtPriorityFailure::InvalidPriority {
                priority: spec.priority,
                valid_min,
                valid_max,
            });
        }
        let param = libc::sched_param {
            sched_priority: spec.priority,
        };
        let rc = unsafe {
            // SAFETY: sched_setscheduler 只读 &param（sched_param 按平台 ABI 初始化，
            // sched_priority 已过范围校验），pid=0 指当前线程（进程）；失败走结构化错误分支。
            libc::sched_setscheduler(0, policy, &param)
        };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            let raw = err.raw_os_error().unwrap_or(0);
            return Err(match raw {
                libc::EPERM | libc::EACCES => RtPriorityFailure::PermissionDenied {
                    os_error: raw,
                    hint: "needs CAP_SYS_NICE or RLIMIT_RTPRIO > 0 (container/sandbox \
                           defaults deny it); grant via setcap/limits/rtkit or run privileged",
                },
                _ => RtPriorityFailure::OsError {
                    os_error: raw,
                    stage: "sched_setscheduler",
                },
            });
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        let tc = spec
            .macos
            .ok_or(RtPriorityFailure::MacosTimeConstraintRequired)?;
        let fields = validate_time_constraint(&tc)?;
        // THREAD_TIME_CONSTRAINT_POLICY = 2（mach/thread_policy.h 公开常量）。
        const THREAD_TIME_CONSTRAINT_POLICY: u32 = 2;
        // SAFETY: Mach 公开 ABI（审计注记见模块文档）：integer_t = i32，端口 = u32。
        unsafe extern "C" {
            fn mach_thread_self() -> u32;
            fn thread_policy_set(thread: u32, flavor: u32, policy: *mut i32, count: u32) -> i32;
        }
        let mut policy = fields;
        let port = unsafe {
            // SAFETY: mach_thread_self 无参数，返回当前线程的 send right（端口），
            // 任何返回值都是合法端口名（MACH_PORT_NULL 亦会被 thread_policy_set 拒绝并走错误分支）。
            mach_thread_self()
        };
        let kr = unsafe {
            // SAFETY: thread_policy_set 按公开 ABI 调用：policy 指向栈上 [i32; 4]
            // （正好 4 个 integer_t，与 THREAD_TIME_CONSTRAINT_POLICY 的 count=4 匹配），
            // 内容已过 validate_time_constraint 校验；仅写策略不接管内存。
            thread_policy_set(
                port,
                THREAD_TIME_CONSTRAINT_POLICY,
                policy.as_mut_ptr(),
                policy.len() as u32,
            )
        };
        if kr != 0 {
            return Err(RtPriorityFailure::OsError {
                os_error: kr,
                stage: "thread_policy_set(THREAD_TIME_CONSTRAINT_POLICY)",
            });
        }
        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = spec;
        Err(RtPriorityFailure::UnsupportedPlatform {
            platform: std::env::consts::OS,
        })
    }
}
