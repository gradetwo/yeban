//! T-FIX-93 #1（ROAD-M2-001）：`rt_priority` 模块判据。
//!
//! 三层：
//! 1. 非法优先级在任何系统调用之前被结构化拒绝（确定性，Linux）。
//! 2. Linux 合法规格的**实测语义**：非特权环境（本沙箱/CI）得
//!    `PermissionDenied{os_error: EPERM}` + 非空 hint；特权环境得 `Ok(())`。
//!    两种结果都是正确行为 —— 本判据钉住的是「结构化 + 响亮 + 不 panic」，
//!    不是某个特定结果（结果由环境权限决定，报告里已注明本沙箱实测为 EPERM 分支）。
//! 3. macOS time-constraint 校验是**纯函数**（任意平台可测）：关系违例先于系统调用拒绝。

use yeban_engine::rt_priority::{MacosTimeConstraint, RtPriorityFailure, validate_time_constraint};
#[cfg(target_os = "linux")]
use yeban_engine::rt_priority::{RtPrioritySpec, RtScheduler, set_current_thread_realtime};

#[cfg(target_os = "linux")]
#[test]
fn invalid_priority_is_rejected_before_any_syscall() {
    let spec = RtPrioritySpec {
        scheduler: RtScheduler::Fifo,
        priority: 0,
        macos: None,
    };
    match set_current_thread_realtime(&spec) {
        Err(RtPriorityFailure::InvalidPriority {
            priority,
            valid_min,
            valid_max,
        }) => {
            assert_eq!(priority, 0);
            assert!(
                valid_min >= 1,
                "Linux SCHED_FIFO 下界应为 1，实测 {valid_min}"
            );
            assert!(valid_max >= valid_min);
        }
        other => panic!("priority 0 must be InvalidPriority, got {other:?}"),
    }

    let spec_high = RtPrioritySpec {
        scheduler: RtScheduler::RoundRobin,
        priority: 1000,
        macos: None,
    };
    match set_current_thread_realtime(&spec_high) {
        Err(RtPriorityFailure::InvalidPriority { priority, .. }) => assert_eq!(priority, 1000),
        other => panic!("priority 1000 must be InvalidPriority, got {other:?}"),
    }
}

#[cfg(target_os = "linux")]
#[test]
fn linux_valid_spec_is_structured_and_loud_never_silent() {
    let spec = RtPrioritySpec {
        scheduler: RtScheduler::Fifo,
        priority: 10,
        macos: None,
    };
    match set_current_thread_realtime(&spec) {
        // 特权环境（CAP_SYS_NICE / RLIMIT_RTPRIO > 0）：设置成功也是正确结果。
        Ok(()) => {}
        // 非特权环境（本沙箱 2026-11-24 实测分支）：EPERM + 非空响亮 hint。
        Err(RtPriorityFailure::PermissionDenied { os_error, hint }) => {
            assert_eq!(os_error, libc::EPERM, "非特权环境的 errno 应为 EPERM");
            assert!(
                hint.contains("CAP_SYS_NICE"),
                "hint 必须点名权限来源: {hint}"
            );
        }
        Err(other) => panic!("valid spec must not fail with {other:?} on linux"),
    }
}

#[test]
fn macos_time_constraint_is_validated_before_syscall() {
    // 关系违例：computation > constraint
    let bad1 = MacosTimeConstraint {
        period_us: 10_000,
        computation_us: 6_000,
        constraint_us: 5_000,
        preemptible: false,
    };
    assert!(matches!(
        validate_time_constraint(&bad1),
        Err(RtPriorityFailure::MacosInvalidDurations { .. })
    ));
    // 关系违例：constraint > period
    let bad2 = MacosTimeConstraint {
        period_us: 5_000,
        computation_us: 1_000,
        constraint_us: 8_000,
        preemptible: false,
    };
    assert!(matches!(
        validate_time_constraint(&bad2),
        Err(RtPriorityFailure::MacosInvalidDurations { .. })
    ));
    // 关系违例：period == 0
    let bad3 = MacosTimeConstraint {
        period_us: 0,
        computation_us: 0,
        constraint_us: 0,
        preemptible: true,
    };
    assert!(matches!(
        validate_time_constraint(&bad3),
        Err(RtPriorityFailure::MacosInvalidDurations { .. })
    ));
    // 合法规格：字段按 period/computation/constraint/preemptible 顺序映射。
    let ok = MacosTimeConstraint {
        period_us: 10_666,
        computation_us: 2_000,
        constraint_us: 8_000,
        preemptible: false,
    };
    assert_eq!(
        validate_time_constraint(&ok).expect("valid tc must pass"),
        [10_666, 2_000, 8_000, 0]
    );
    // preemptible=true 映射 1。
    let ok2 = MacosTimeConstraint {
        period_us: 1_000,
        computation_us: 500,
        constraint_us: 1_000,
        preemptible: true,
    };
    assert_eq!(
        validate_time_constraint(&ok2).expect("valid tc must pass")[3],
        1
    );
}

#[test]
fn error_display_is_loud_and_actionable() {
    // 响亮语义的文本面：报告/日志直接可读。
    let denied = RtPriorityFailure::PermissionDenied {
        os_error: 1,
        hint: "needs CAP_SYS_NICE",
    };
    assert!(denied.to_string().contains("denied"));
    let unsupported = RtPriorityFailure::UnsupportedPlatform {
        platform: "windows",
    };
    assert!(unsupported.to_string().contains("windows"));
}
