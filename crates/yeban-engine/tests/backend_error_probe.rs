//! `#147`：`backend_errors` 的**分档口径**判据（运行期 vs 关闭/停止期）。
//!
//! # 这个文件钉住什么
//!
//! 实测异常：`measure_latency --loopback` 三次运行都报 `backend_errors_out=1`
//! `backend_errors_in=1`，而其它读数全健康。旧口径把"运行期真错误"与"暂停/关流阶段
//! 到达的错误"加成一个数 ⇒ 那个 **1** 读不出是哪一种。
//!
//! 本文件用**具名桩**（直接构造 `cpal::Error`）注入事件，**不需要音频设备**：
//!
//! 1. 阶段切换前到达 ⇒ `runtime`；`begin_stop()` 之后到达 ⇒ `stop`；
//! 2. `total()` 与旧口径逐位相同（分档只增加读数，不改总数）；
//! 3. 首条错误保留种类与（截断的）消息；之后的错误不覆盖它；
//! 4. `begin_run()` 能把阶段切回运行期（`pause` 后再 `play`）；
//! 5. **证伪臂**：同一事件序列喂给"只数总数"的旧式计数器 ⇒ 它无法给出两档，
//!    因此"把总数当运行期"在这条序列上必然错 —— 这条证明分档是**承重**的，不是装饰。
//!
//! 覆盖边界（如实登记）：本文件**不**打开任何流、**不**验证 cpal 在真实设备上
//! 何时调用错误回调。那需要一台有声卡的机器；本文件只证明"事件到了之后怎么记账"。

#![cfg(feature = "device")]

use std::sync::atomic::{AtomicU64, Ordering};

use cpal::{Error, ErrorKind};
use yeban_engine::device::{
    BACKEND_ERROR_MESSAGE_MAX, BackendErrorLog, backend_error_kind_code, backend_error_kind_name,
};

/// 一个典型的**运行期**错误（XRun：缓冲区欠载/过载）。
fn xrun() -> Error {
    Error::new(ErrorKind::Xrun)
}

/// 一个典型的**关流/设备消失**错误（消息与 cpal CoreAudio 的字面一致）。
fn disconnected() -> Error {
    Error::with_message(ErrorKind::DeviceNotAvailable, "Device disconnected")
}

/// 判据 1：同一次"每方向 1 次"的观测，落在哪个阶段必须能被两档读数分开。
///
/// 这是实测异常的**最小复现**：只有一次错误。旧口径只能报 `total=1`；分档后
/// `runtime=1, stop=0` 与 `runtime=0, stop=1` 是**两个不同的事实**。
#[test]
fn one_error_is_classified_by_the_phase_it_arrives_in() {
    // 运行期：什么都没切，来了 1 次。
    let running = BackendErrorLog::new();
    running.record(&xrun());
    let counts = running.counts();
    assert_eq!(
        (counts.runtime, counts.stop, counts.total()),
        (1, 0, 1),
        "运行期到达的 1 次必须记在 runtime 档"
    );

    // 关闭/停止期：先切阶段，再来了 1 次。
    let stopping = BackendErrorLog::new();
    stopping.begin_stop();
    stopping.record(&disconnected());
    let counts = stopping.counts();
    assert_eq!(
        (counts.runtime, counts.stop, counts.total()),
        (0, 1, 1),
        "begin_stop() 之后到达的 1 次必须记在 stop 档"
    );
}

/// 判据 2：两档分别累加，`total()` 与旧的单计数器逐位相同。
#[test]
fn the_two_buckets_add_up_to_the_old_total() {
    let log = BackendErrorLog::new();
    log.record(&xrun());
    log.record(&Error::new(ErrorKind::DeviceBusy));
    log.begin_stop();
    log.record(&disconnected());
    log.record(&Error::new(ErrorKind::StreamInvalidated));

    let counts = log.counts();
    assert_eq!(counts.runtime, 2, "阶段切换前的两次必须都在 runtime");
    assert_eq!(counts.stop, 2, "阶段切换后的两次必须都在 stop");
    assert_eq!(
        counts.total(),
        4,
        "total 必须等于旧口径的单个计数器（分档不改总数）"
    );
}

/// 判据 3：首条错误的种类与消息被保留；后续错误不覆盖它；超长消息按上限截断。
#[test]
fn only_the_first_error_keeps_its_kind_and_bounded_message() {
    let long = "x".repeat(BACKEND_ERROR_MESSAGE_MAX + 40);
    let log = BackendErrorLog::new();
    log.record(&Error::with_message(ErrorKind::DeviceNotAvailable, long));
    log.record(&Error::with_message(ErrorKind::Xrun, "later error"));

    let first = log.counts().first.expect("至少有一条错误");
    assert_eq!(first.kind_name(), "device-not-available");
    assert_eq!(
        first.message().len(),
        BACKEND_ERROR_MESSAGE_MAX,
        "消息必须截断到固定缓冲上限"
    );
    assert!(
        first.message().bytes().all(|byte| byte == b'x'),
        "首条消息的内容必须原样保留（截断只砍尾部）"
    );
    // 第二条没有覆盖首条。
    assert_eq!(first.kind_name(), "device-not-available");
}

/// 判据 4：`begin_run()` 把阶段切回运行期（`pause()` 后再 `play()` 的语义）。
#[test]
fn begin_run_reopens_the_runtime_phase() {
    let log = BackendErrorLog::new();
    log.record(&xrun()); // runtime = 1
    log.begin_stop();
    log.record(&disconnected()); // stop = 1
    log.begin_run();
    log.record(&xrun()); // runtime = 2

    let counts = log.counts();
    assert_eq!((counts.runtime, counts.stop), (2, 1));
    assert_eq!(counts.total(), 3);
}

/// 判据 5：`cpal::ErrorKind` 的编号/名字是稳定字面量（机器可读行直接打印名字）。
#[test]
fn every_documented_error_kind_has_a_stable_name() {
    let cases = [
        (ErrorKind::DeviceBusy, "device-busy"),
        (ErrorKind::DeviceChanged, "device-changed"),
        (ErrorKind::DeviceNotAvailable, "device-not-available"),
        (ErrorKind::HostUnavailable, "host-unavailable"),
        (ErrorKind::InvalidInput, "invalid-input"),
        (ErrorKind::PermissionDenied, "permission-denied"),
        (ErrorKind::RealtimeDenied, "realtime-denied"),
        (ErrorKind::ResourceExhausted, "resource-exhausted"),
        (ErrorKind::StreamInvalidated, "stream-invalidated"),
        (ErrorKind::UnsupportedConfig, "unsupported-config"),
        (ErrorKind::UnsupportedOperation, "unsupported-operation"),
        (ErrorKind::Xrun, "xrun"),
        (ErrorKind::BackendError, "backend-error"),
        (ErrorKind::Other, "other"),
    ];
    for (kind, name) in cases {
        let code = backend_error_kind_code(kind);
        assert_ne!(code, 0, "{name} 不许落进 unknown(0)");
        assert_eq!(backend_error_kind_name(code), name, "{kind:?} 的名字漂移了");
    }
    assert_eq!(backend_error_kind_name(0), "unknown");
}

/// 判据 6（**证伪臂**）：只数总数的旧式实现**无法**给出两档。
///
/// 用同一事件序列喂两个实现：
/// - 新：`runtime=1, stop=1, total=2`；
/// - 旧（单原子）：只有 `total=2`。
///
/// 于是"把 total 当 runtime"读出来是 `2`，而真值是 `1` ⇒ 旧口径在这条序列上
/// **必然错**。这条断言是分档"承重"的机械证据，不是注释里的声明。
#[test]
fn a_total_only_counter_cannot_tell_the_two_phases_apart() {
    struct TotalOnly(AtomicU64);

    let log = BackendErrorLog::new();
    let old = TotalOnly(AtomicU64::new(0));

    log.record(&xrun()); // runtime
    old.0.fetch_add(1, Ordering::Relaxed);
    log.begin_stop();
    log.record(&disconnected()); // stop
    old.0.fetch_add(1, Ordering::Relaxed);

    let counts = log.counts();
    let old_total = old.0.load(Ordering::Relaxed);
    assert_eq!(counts.total(), old_total, "总数口径必须一致");
    assert_ne!(
        counts.runtime, old_total,
        "把旧总数当成运行期错误数会多报一条 —— 这正是 #147 要消除的歧义"
    );
    assert_eq!(counts.runtime, 1, "运行期真值");
    assert_eq!(counts.stop, 1, "关闭期真值");
}

/// 判据 7：机器可读行**逐字**稳定、无空白、可被 `split_whitespace` 解析。
///
/// 这是"两档读数"的**输出契约**：集成者/人类重跑时读的就是这一行，所以它的
/// 键序、字面量与"没有错误时写 none"的规则都由本判据钉住（无头、无设备）。
#[test]
fn the_machine_readable_line_is_stable_and_parseable() {
    let empty = BackendErrorLog::new();
    assert_eq!(
        empty.counts().machine_line("output"),
        "BACKEND-ERRORS direction=output runtime=0 stop=0 total=0 first_kind=none \
first_message=none"
    );

    let stopping = BackendErrorLog::new();
    stopping.begin_stop();
    stopping.record(&disconnected());
    assert_eq!(
        stopping.counts().machine_line("input"),
        "BACKEND-ERRORS direction=input runtime=0 stop=1 total=1 \
first_kind=device-not-available first_message=Device_disconnected"
    );

    // 无消息的 XRun：`first_message` 必须是字面 `none`，不许是空值。
    let running = BackendErrorLog::new();
    running.record(&xrun());
    assert_eq!(
        running.counts().machine_line("output"),
        "BACKEND-ERRORS direction=output runtime=1 stop=0 total=1 first_kind=xrun \
first_message=none"
    );

    // 任意空白/引号都被压成 `_` ⇒ 值里不可能出现空白。
    let odd = BackendErrorLog::new();
    odd.record(&Error::with_message(ErrorKind::Other, "a b\"c\nd\te=f"));
    let line = odd.counts().machine_line("output");
    assert!(
        line.ends_with("first_message=a_b_c_d_e_f"),
        "消息必须被压成不含空白的 token: {line}"
    );

    // 严格解析：除首 token 外每个 token 都必须是 `key=value`，且值非空。
    for line in [
        empty.counts().machine_line("output"),
        stopping.counts().machine_line("input"),
        line,
    ] {
        let mut tokens = line.split_whitespace();
        assert_eq!(tokens.next(), Some("BACKEND-ERRORS"));
        for token in tokens {
            let (key, value) = token.split_once('=').unwrap_or_else(|| {
                panic!("token {token:?} 不是 key=value（行长格式漂移）: {line}")
            });
            assert!(!key.is_empty(), "空键: {line}");
            assert!(!value.is_empty(), "空值: {line}");
        }
    }
}
