//! `yeban-plugin-probe`：插件崩溃隔离的**子进程探测器**（T-FIX-339）。
//!
//! 由宿主 `probe::spawn_scan` 以子进程启动；当前只支持 `scan <path>`：在子进程内
//! 加载 VST3 模块并枚举 / 验证，把结果按 `probe` 模块的文本协议写 stdout。
//! 坏插件 `abort()` / SIGSEGV / 挂起只影响本子进程，父应用存活。
//!
//! ⚠ 本二进制只做「一次性探测」，不承载渲染实例 —— 音频线程的 watchdog 是后续切片；
//! 探测期的崩溃隔离是本单落地的第一部分，不把 phase 2 当成整体已解决。

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("scan") => {
            if let Some(path) = args.next() {
                yeban_plugin_host::probe::run_probe_scan(&path);
            } else {
                eprintln!("usage: yeban-plugin-probe scan <path>");
                std::process::exit(2);
            }
        }
        _ => {
            eprintln!("usage: yeban-plugin-probe scan <path>");
            std::process::exit(2);
        }
    }
}
