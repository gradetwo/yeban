//! ⭐ T-FIX-339：插件崩溃隔离判据（真子进程，非 mock）。
//!
//! 现场用 `rustc` 编译两个恶意 cdylib：
//! - `fixture_abort.rs`：dlopen 时 `abort()` —— 进程内加载会带走整个测试进程；
//! - `fixture_sleep.rs`：dlopen 时 sleep 30 s —— 进程内同步加载会卡死。
//!
//! 把探针二进制路径注入 `YEBAN_PLUGIN_PROBE_BIN` 后调用 [`Host::scan`]，断言两个
//! 恶意模块各返回一条 `error` 描述符、且**本测试进程存活**（能走到断言之后）。
//! 这就是派单「Host::scan 返回失败行且进程存活」的判据。
//!
//! 平台：`.init_array` 构造函数在 Linux/macOS 生效；Windows 上该夹具不触发
//! （构造函数段不同），故本判据 `#[cfg(unix)]`。

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use yeban_plugin_host::Host;

#[cfg(target_os = "macos")]
const DYLIB_EXT: &str = "dylib";
#[cfg(all(unix, not(target_os = "macos")))]
const DYLIB_EXT: &str = "so";

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn test_dir() -> PathBuf {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("yeban-plugin-crash-{}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

fn compile_fixture(src_name: &str, out_name: &str, dir: &Path) -> PathBuf {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(src_name);
    assert!(source.is_file(), "夹具源码不在 {source:?}");
    let out = dir.join(format!("{out_name}.{DYLIB_EXT}"));
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let status = Command::new(&rustc)
        .args([
            "--crate-type",
            "cdylib",
            "--edition",
            "2021",
            "-o",
            out.to_str().expect("UTF-8 路径"),
            source.to_str().expect("UTF-8 路径"),
        ])
        .status()
        .expect("启动 rustc");
    assert!(status.success(), "夹具 {src_name} 编译失败");
    out
}

#[test]
fn scan_survives_abort_and_hang_in_child_probe() {
    // 注入探针二进制（编译期由 Cargo 提供路径）。
    // SAFETY: 单线程测试函数入口设置本测试专用 env，无并发读者（本文件仅一个 #[test]）。
    unsafe {
        std::env::set_var(
            "YEBAN_PLUGIN_PROBE_BIN",
            env!("CARGO_BIN_EXE_yeban-plugin-probe"),
        );
    }

    let dir = test_dir();
    let _abort_lib = compile_fixture("fixture_abort.rs", "abort_plugin", &dir);
    let _sleep_lib = compile_fixture("fixture_sleep.rs", "sleep_plugin", &dir);

    // 把超时压到 1 s，避免判据慢（sleep 夹具睡 30 s，必被 kill）。
    // SAFETY: 同上，单线程测试入口设置 env。
    unsafe {
        std::env::set_var("YEBAN_PLUGIN_PROBE_TIMEOUT_MS", "1200");
    }

    let descriptors = Host::scan(std::slice::from_ref(&dir));

    // 两个恶意模块都必须有一条 error 行；本进程必须存活（能执行到这里）。
    let abort_row = descriptors
        .iter()
        .find(|d| d.path.contains("abort_plugin"))
        .expect("abort 模块的扫描行必须在");
    assert!(
        abort_row.error.is_some(),
        "abort 模块必须返回失败行（子进程崩溃），实际 error={:?}",
        abort_row.error
    );
    let sleep_row = descriptors
        .iter()
        .find(|d| d.path.contains("sleep_plugin"))
        .expect("sleep 模块的扫描行必须在");
    assert!(
        sleep_row.error.is_some(),
        "超时模块必须返回失败行，实际 error={:?}",
        sleep_row.error
    );
    assert!(
        sleep_row
            .error
            .as_deref()
            .is_some_and(|e| e.contains("超时")),
        "超时失败原因必须点名 timeout，实际 {:?}",
        sleep_row.error
    );

    // 清理：确认本进程仍在（上面断言已证明），临时目录删掉。
    let _ = std::fs::remove_dir_all(&dir);
}
