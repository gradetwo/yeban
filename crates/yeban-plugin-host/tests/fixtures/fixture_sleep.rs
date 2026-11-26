//! 恶意夹具：dlopen 时长时间 sleep（T-FIX-339 超时隔离判据）。
//!
//! 由测试用 `rustc --crate-type cdylib` 现场编译。`.init_array` 构造函数在
//! `dlopen` 阶段 sleep 30 s —— 宿主若在本进程同步加载会卡死；子进程探针按超时
//! `kill`，父进程继续。

#![no_main]

#[used]
#[cfg_attr(target_os = "linux", link_section = ".init_array")]
#[cfg_attr(target_os = "macos", link_section = "__DATA,__mod_init_func,mod_init_funcs")]
static INIT: extern "C" fn() = {
    extern "C" fn nap() {
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
    nap
};
