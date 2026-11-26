//! 恶意夹具：dlopen 时立刻 `abort()`（T-FIX-339 崩溃隔离判据）。
//!
//! 由测试用 `rustc --crate-type cdylib` 现场编译。`.init_array` 里的构造函数在
//! `dlopen` 阶段执行 —— 宿主若在**本进程**加载它，整个测试进程会被 abort 带走；
//! 只有把加载放进子进程（探针）才能隔离。⚠ 故意不导出 `GetPluginFactory`：
//! 崩溃发生在 load 之前，足以证明隔离边界。

#![no_main]

#[used]
#[cfg_attr(target_os = "linux", link_section = ".init_array")]
#[cfg_attr(target_os = "macos", link_section = "__DATA,__mod_init_func,mod_init_funcs")]
static INIT: extern "C" fn() = {
    extern "C" fn die() {
        std::process::abort();
    }
    die
};
