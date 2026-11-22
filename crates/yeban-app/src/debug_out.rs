//! 调试输出汇点（T-FIX-70）：进程内**唯一**的调试诊断开关与落点。
//!
//! ## 为什么是"汇点"而不是日志框架
//!
//! 本仓的诊断一直是同一个形状：调用点把报告组装成 `Vec<String>`，经
//! [`crate::cli::emit`] 逐行打到 stdout。本模块⛔ 不改这个形状、不引入
//! 级别/目标/过滤那套框架概念，只多做三件事：
//!
//! 1. **一个进程级开关**：由 `main` 在参数解析后按
//!    [`crate::settings::resolve_debug_output`]（`--debug` / `--log-file` /
//!    已存菜单开关任一开启）定案，经 [`init`] 注入；运行期主菜单
//!    「设置 → 调试输出」经 [`set_enabled`] 翻转，**即时**对之后发出的行
//!    生效（启动早期已经发出的枚举/协商行不在追溯范围内，菜单写回设置
//!    后下次启动从第一行起生效）。
//! 2. **一个放行口**：[`emit`] 在开启时把整批行照 `cli::emit` 的样子打到
//!    stdout；关闭时整批丢弃。调用点通常先查 [`enabled`] 再组装行
//!    （枚举/协商这类要真读设备的组装不白做）。
//! 3. **一份可选的文件副本**：`--log-file <path>` 时同一批行另写一份到
//!    文件（用户回传诊断用）。文件在 [`init`] 时创建/截断；**打不开 ⇒
//!    `Err`**，由 `main` 响亮地以用法错误退出 —— 用户明确要了这份文件，
//!    静默丢掉比启动失败更坏。运行期写失败 ⇒ stderr 响亮报一次并停写
//!    文件（控制台输出继续），⛔ 不静默吞。
//!
//! 实时音频回调⛔ 不经本模块（红线 7：回调内零日志）；本模块的全部调用
//! 点都在控制面（启动、菜单、导出回执）。

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// 汇点状态（进程内单份）。
struct DebugState {
    /// 当前是否放行调试行。
    enabled: bool,
    /// `--log-file` 的文件副本（`init` 时打开；写失败后置 `None` 停写）。
    log_file: Option<File>,
}

static STATE: OnceLock<Mutex<DebugState>> = OnceLock::new();

fn state() -> &'static Mutex<DebugState> {
    STATE.get_or_init(|| {
        Mutex::new(DebugState {
            enabled: false,
            log_file: None,
        })
    })
}

/// 启动时定案（`main` 在参数解析后、路径分流前调用一次）：写入开关与
/// 可选的日志文件。重复调用以最后一次为准（单测/无头路径可能重入）。
///
/// # Errors
///
/// `log_file` 给出但创建/截断失败 ⇒ `Err(原文)`（含路径与 io 错误），
/// 调用方应当响亮失败，⛔ 不降级成"只打控制台"。
pub fn init(enabled: bool, log_file: Option<&Path>) -> Result<(), String> {
    let mut guard = state().lock().expect("调试汇点状态锁被毒化");
    if let Some(path) = log_file {
        let file = File::create(path).map_err(|error| {
            format!(
                "--log-file {} 打不开（{error}），调试日志无处落盘",
                path.display()
            )
        })?;
        guard.log_file = Some(file);
    }
    guard.enabled = enabled;
    Ok(())
}

/// 当前是否放行调试行。
#[must_use]
pub fn enabled() -> bool {
    state().lock().expect("调试汇点状态锁被毒化").enabled
}

/// 运行期翻转（主菜单「设置 → 调试输出」的落点）：即时对之后经 [`emit`]
/// 发出的行生效。持久化由调用方走 `settings::save_debug_output`。
pub fn set_enabled(enabled: bool) {
    state().lock().expect("调试汇点状态锁被毒化").enabled = enabled;
}

/// 放行一批调试行：开启时逐行 `println!`（与 [`crate::cli::emit`] 同形）
/// 并追加进日志文件（若有）；关闭时整批丢弃。
pub fn emit(lines: &[String]) {
    let mut guard = state().lock().expect("调试汇点状态锁被毒化");
    if !guard.enabled {
        return;
    }
    for line in lines {
        println!("{line}");
    }
    write_to_log(&mut guard, lines);
}

/// 只把一批行追加进日志文件（⛔ 不打控制台）：给「行已经由调用方自己
/// 的报告通道打过」的落点用 —— 批处理报告的调试段随 `run_batch` 的
/// 返回值经 `cli::emit` 统一打印，文件副本在这里补齐，否则
/// `--log-file` 拿到的回传文件会缺枚举/装载/导出明细。没有打开的日志
/// 文件时什么都不做；写失败与 [`emit`] 同口径（响亮一次并停写）。
pub fn append_to_log(lines: &[String]) {
    let mut guard = state().lock().expect("调试汇点状态锁被毒化");
    write_to_log(&mut guard, lines);
}

/// 日志文件副本的写入（[`emit`] 与 [`append_to_log`] 共用）：逐行追加
/// 并 flush；写失败 ⇒ stderr 响亮报一次并停写文件（置 `None`），
/// ⛔ 不静默吞、也不影响控制台输出。
fn write_to_log(guard: &mut DebugState, lines: &[String]) {
    if guard.log_file.is_none() {
        return;
    }
    let mut failure: Option<std::io::Error> = None;
    {
        let file = guard.log_file.as_mut().expect("刚查过是 Some");
        for line in lines {
            if let Err(error) = writeln!(file, "{line}") {
                failure = Some(error);
                break;
            }
        }
        if failure.is_none() {
            let _ = file.flush();
        }
    }
    if let Some(error) = failure {
        eprintln!("[yeban-app] 调试日志文件写入失败（{error}）—— 已停止写文件，控制台调试输出继续");
        guard.log_file = None;
    }
}

/// 设备枚举调试行（输出 + 输入两个方向）：每台一行 —— 名称、是否默认、
/// 默认配置与全部受支持配置。枚举本身失败 ⇒ 一行响亮的失败原文
/// （⛔ 不假装"0 台"）。设备读不到支持范围时 [`yeban_engine::device::DeviceInfo::supported`]
/// 里就是 `unavailable: <原文>`，原样带出。
#[must_use]
pub fn device_enumeration_lines() -> Vec<String> {
    let mut lines = Vec::new();
    match yeban_engine::device::enumerate_output_devices() {
        Ok(devices) => {
            lines.push(format!("[debug] 输出设备枚举: {} 台", devices.len()));
            for (index, device) in devices.iter().enumerate() {
                lines.push(format!(
                    "[debug] 输出设备[{index}]: {:?} 默认={} 默认配置={} ch @ {} Hz {:?} 支持: {}",
                    device.name,
                    if device.is_default { "是" } else { "否" },
                    device.channels,
                    device.sample_rate,
                    device.sample_format,
                    device.supported,
                ));
            }
        }
        Err(error) => lines.push(format!("[debug] 输出设备枚举失败: {error}")),
    }
    match yeban_engine::device::enumerate_input_devices() {
        Ok(devices) => {
            lines.push(format!("[debug] 输入设备枚举: {} 台", devices.len()));
            for (index, device) in devices.iter().enumerate() {
                lines.push(format!(
                    "[debug] 输入设备[{index}]: {:?} 默认={} 默认配置={} ch @ {} Hz {:?} 支持: {}",
                    device.name,
                    if device.is_default { "是" } else { "否" },
                    device.channels,
                    device.sample_rate,
                    device.sample_format,
                    device.supported,
                ));
            }
        }
        Err(error) => lines.push(format!("[debug] 输入设备枚举失败: {error}")),
    }
    lines
}

/// 打开失败时的协商 trace 调试行：对默认输出设备做一次只协商、不建流的
/// 重演（[`yeban_engine::device::trace_output_negotiation`]），逐步说明
/// 协商在哪一步、为什么落到这个结果。成功路径不走这里 —— 成功时 trace
/// 挂在流句柄/`DeviceOpening` 上，是那一次**真实**打开的记录。
#[must_use]
pub fn output_negotiation_trace_lines(want: &yeban_engine::device::EngineConfig) -> Vec<String> {
    yeban_engine::device::trace_output_negotiation(want)
        .into_iter()
        .map(|line| format!("[debug] {line}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_rejects_an_unwritable_log_file_loudly() {
        // 家目录之外的不存在路径：父目录不存在 ⇒ 创建必失败 ⇒ Err 点名路径，
        // ⛔ 不静默降级。注意本测试不动全局开关的断言（进程内其它测试可能
        // 并行读 enabled()），只钉 init 的失败形状。
        let path = Path::new("/nonexistent-dir-yeban-test/debug.log");
        let error = init(false, Some(path)).expect_err("打不开的日志文件必须响亮失败");
        assert!(error.contains("debug.log"), "错误原文须点名路径: {error}");
    }
}
