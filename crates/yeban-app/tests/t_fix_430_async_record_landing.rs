//! **T-FIX-430** 停录不阻塞 UI 主线程——静态判据（`include_str!`，本机不编 Slint）。
//!
//! 背景：录音键的 `on_toggle_record` 在 Slint 回调（UI 主线程）里直接 `Recorder::stop()`，
//! 停录那一刻 UI 线程要把整段录音量化并写几百 MB。10 分钟 48 kHz 立体声 = 5,760 万次量化，
//! UI 明显卡死（issue ISS-MWVUAKIW-C1B040，T-FIX-203 审计）。
//!
//! 本文件钉住修复后的**结构契约**（每条修前必红/修后必绿）：
//!
//! | # | 判据 | 修前为什么红 |
//! | :--- | :--- | :--- |
//! | 1 | `Recorder::stop_async` 存在，且量化+写盘在 `thread::spawn` 的闭包里 | 修前只有同步 `stop()`，量化/写盘在调用线程 |
//! | 2 | `RecordLanding` 提供非阻塞 `try_take` | 修前没有可轮询句柄 |
//! | 3 | `RecordControl` 用 `pending_landing` 槽 + 轮询定时器，而非停录内同步落工程 | 修前 `stop()` 后立刻 `apply_stopped` |
//! | 4 | 停录瞬间给用户"正在保存录音…"回执 | 修前停录期间界面无反馈 |
//!
//! ⚠ 纯源码字串判据；行为由 record_wav 单测（stop_async_* / record_landing_poll_*）与 CI 真跑。

const RECORD_WAV: &str = include_str!("../src/record_wav.rs");
const HOST: &str = include_str!("../src/host.rs");

/// 判据 1：`stop_async` 存在，且 `land_sink`（量化＋写盘）只在 `thread::spawn` 里被调用。
#[test]
fn stop_async_moves_quantize_and_write_off_the_calling_thread() {
    assert!(
        RECORD_WAV.contains("pub fn stop_async("),
        "缺 `Recorder::stop_async`：停录重活必须能交给后台线程"
    );
    // land_sink 是承重量化+write_plain_wav 的函数。
    assert!(
        RECORD_WAV.contains("fn land_sink("),
        "缺 `land_sink`：量化+写盘应抽成后台线程主体"
    );
    // ⭐ 核心：land_sink 必须只出现在 thread::spawn 的闭包里（⛔ 不在 stop_async 体内直接调）。
    assert!(
        RECORD_WAV.contains("std::thread::spawn(move || land_sink("),
        "`land_sink` 未在 `thread::spawn` 闭包里调用 ⇒ 量化/写盘仍可能在调用线程（回退）"
    );
    // ⛔ 反向腿：stop_async 体内不许直接 `land_sink(`（那会同步执行）。
    // 取 stop_async 函数体（到下一个 `pub fn` 或 `}` 结束的近似切片）做严格检查。
    let start = RECORD_WAV
        .find("pub fn stop_async(")
        .expect("定位 stop_async");
    let tail = &RECORD_WAV[start..];
    let body_end = tail[1..]
        .find("\n    pub fn ")
        .map_or(tail.len(), |offset| offset + 1);
    let body = &tail[..body_end];
    assert!(
        body.contains("std::thread::spawn(move || land_sink("),
        "stop_async 体内没有把 land_sink 交给 thread::spawn"
    );
    assert!(
        !body
            .replace("std::thread::spawn(move || land_sink(", "")
            .contains("land_sink("),
        "stop_async 体内除了 spawn 之外还直接调用了 land_sink ⇒ 同步执行回退"
    );
}

/// 判据 2：`RecordLanding` 提供非阻塞 `try_take`（UI 轮询用）与阻塞 `wait`（同步口径用）。
#[test]
fn record_landing_exposes_nonblocking_try_take() {
    assert!(
        RECORD_WAV.contains("pub struct RecordLanding"),
        "缺 `RecordLanding`：停录后要交回一个可轮询的落盘任务句柄"
    );
    assert!(
        RECORD_WAV.contains("pub fn try_take("),
        "`RecordLanding` 缺非阻塞 `try_take`：UI 只能靠它轮询，⛔ 不能阻塞等"
    );
    // try_take 必须用 is_finished 做非阻塞判断（⛔ 不能 join 阻塞在 UI 线程）。
    assert!(
        RECORD_WAV.contains("is_finished()"),
        "`try_take` 未用 `is_finished()` 做非阻塞判断 ⇒ 可能阻塞 UI 主线程"
    );
    assert!(
        RECORD_WAV.contains("pub fn wait("),
        "`RecordLanding` 缺 `wait`：同步 `stop()` 口径需要阻塞等结果"
    );
}

/// 判据 3：UI 层用 `pending_landing` 槽 + 轮询定时器，⛔ 不在停录回调里同步落工程。
#[test]
fn record_control_polls_the_landing_instead_of_blocking() {
    assert!(
        HOST.contains("pending_landing"),
        "`RecordControl` 缺 `pending_landing` 槽：停录结果无处暂存，只能同步处理"
    );
    assert!(
        HOST.contains("fn poll_landing("),
        "缺 `poll_landing`：UI 需要一个定时器轮询后台落盘是否完成"
    );
    assert!(
        HOST.contains("fn begin_landing("),
        "缺 `begin_landing`：停录后要接上后台任务而非当场落工程"
    );
    // stop_if_recording / toggle 必须走 stop_async（⛔ 不是同步 stop）。
    assert!(
        HOST.contains("recorder.stop_async()"),
        "UI 停录路径未调用 `stop_async` ⇒ 仍在 UI 主线程同步量化（回退）"
    );
    // 轮询定时器用 Repeated 50ms（与既有的输入电平定时器同款）。
    assert!(
        HOST.contains("slint::TimerMode::Repeated") && HOST.contains("from_millis(50)"),
        "缺轮询定时器：后台落盘完成后需要一个 50ms Repeated 定时器把它落工程"
    );
}

/// 判据 4：停录瞬间给用户"正在保存录音…"回执（⛔ 不出声等于没保存中的反馈）。
#[test]
fn stopping_recording_reports_saving_progress_to_the_user() {
    assert!(
        HOST.contains("正在保存录音"),
        "停录期间没有「正在保存录音…」回执：用户按了停录看不到反馈（审计 P0-3 同款问题）"
    );
}
