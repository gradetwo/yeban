//! 最小动态库加载器（std＋平台 `dlopen`／`LoadLibrary`，零第三方依赖）。
//!
//! ⚠ 本模块的 `extern` 声明只覆盖一期用到的 4 个 Unix 符号与 3 个 Windows 符号；
//! 不做通用 FFI 封装。Windows 上符号名必须是 ANSI（`LoadLibraryA`），
//! 因此路径在加载前先按 UTF-8→ANSI 有损转换——含非 ASCII 的路径在 Windows 上
//! 可能打不开（已知一期限缩，见 ADR-0064 D12 注）。

use std::ffi::{CString, c_void};

use crate::error::HostError;

/// 已打开的动态库句柄（单主；`Drop` 时关闭）。
pub struct Library {
    handle: *mut c_void,
}

// SAFETY: 句柄单主持有；`dlclose`／`FreeLibrary` 本身线程安全；
// 符号解析只在构造期（单线程）做，句柄不跨线程共享可变状态。
unsafe impl Send for Library {}

impl Library {
    /// 打开动态库（`dlopen`／`LoadLibraryA`）。
    ///
    /// # Errors
    ///
    /// 打开失败 ⇒ [`HostError::LibraryLoad`]（含系统错误文本）。
    pub fn open(path: &str) -> Result<Self, HostError> {
        let c_path = CString::new(path)
            .map_err(|_| HostError::InvalidPluginPath(format!("路径含 NUL 字节: {path}")))?;
        // SAFETY: 路径来自 CString（NUL 结尾）；dlopen 只读。
        let handle = unsafe { platform_open(c_path.as_ptr()) };
        if handle.is_null() {
            return Err(HostError::LibraryLoad(platform_last_error()));
        }
        Ok(Self { handle })
    }

    /// 取导出符号的裸指针（未找到 ⇒ `SymbolMissing`）。
    ///
    /// # Errors
    ///
    /// 符号不存在 ⇒ [`HostError::SymbolMissing`]。
    pub fn symbol(&self, name: &'static str) -> Result<*mut c_void, HostError> {
        let c_name = CString::new(name).map_err(|_| HostError::SymbolMissing(name))?;
        // SAFETY: 句柄来自 dlopen；符号名来自 CString。
        let ptr = unsafe { platform_symbol(self.handle, c_name.as_ptr()) };
        if ptr.is_null() {
            return Err(HostError::SymbolMissing(name));
        }
        Ok(ptr)
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 句柄来自 dlopen 且只关一次（Drop 后置空）。
            unsafe { platform_close(self.handle) };
            self.handle = std::ptr::null_mut();
        }
    }
}

#[cfg(unix)]
mod platform {
    use super::c_void;
    use std::ffi::c_char;

    // RTLD_NOW = 0x2（立即解析，加载时就暴露缺失符号，不拖到调用时）；
    // RTLD_LOCAL = 0x0（符号不进全局表，两个插件同名符号不互相干扰）。
    const RTLD_NOW: i32 = 0x2;

    // SAFETY: 以下符号是 libc／libSystem 的公开接口（glibc ≥ 2.34 后 `dlopen`
    // 在 libc 内；macOS 在 libSystem 内）。Rust 的 std 本身已链接 libc，
    // 故无需显式 `#[link]`。
    unsafe extern "C" {
        fn dlopen(filename: *const c_char, flags: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> i32;
        fn dlerror() -> *const c_char;
    }

    pub(super) unsafe fn platform_open(path: *const c_char) -> *mut c_void {
        // SAFETY: `path` 来自调用方构造的 `CString`（NUL 结尾），`dlopen` 只读。
        unsafe { dlopen(path, RTLD_NOW) }
    }

    pub(super) unsafe fn platform_symbol(handle: *mut c_void, name: *const c_char) -> *mut c_void {
        // SAFETY: `handle` 是 `dlopen` 返回的有效句柄，`name` 是 NUL 结尾串。
        unsafe { dlsym(handle, name) }
    }

    pub(super) unsafe fn platform_close(handle: *mut c_void) {
        // SAFETY: `handle` 是 `dlopen` 返回的有效句柄，且只关一次（`Drop` 置空）。
        unsafe { dlclose(handle) };
    }

    pub(super) fn platform_last_error() -> String {
        // SAFETY: `dlerror` 返回线程局部的 NUL 结尾串（失败时非空；为空则给兜底文案）。
        let ptr = unsafe { dlerror() };
        if ptr.is_null() {
            return "dlopen 失败（无系统错误文本）".to_owned();
        }
        // SAFETY: `dlerror` 的返回值在下一次 dl* 调用前有效；此处立即复制。
        let bytes = unsafe { std::ffi::CStr::from_ptr(ptr) }.to_bytes();
        String::from_utf8_lossy(bytes).into_owned()
    }
}

#[cfg(windows)]
mod platform {
    use super::c_void;
    use std::ffi::c_char;

    // SAFETY: `kernel32` 的公开接口；`LoadLibraryA` 要求 ANSI 路径，
    // 调用方已按 UTF-8→ANSI 有损转换（见模块文档）。
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryA(name: *const c_char) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
        fn FreeLibrary(module: *mut c_void) -> i32;
        fn GetLastError() -> u32;
    }

    pub(super) unsafe fn platform_open(path: *const c_char) -> *mut c_void {
        // SAFETY: `path` 是 NUL 结尾的 ANSI 串。
        unsafe { LoadLibraryA(path) }
    }

    pub(super) unsafe fn platform_symbol(handle: *mut c_void, name: *const c_char) -> *mut c_void {
        // SAFETY: `handle` 是 `LoadLibraryA` 返回的有效句柄。
        unsafe { GetProcAddress(handle, name) }
    }

    pub(super) unsafe fn platform_close(handle: *mut c_void) {
        // SAFETY: `handle` 有效且只关一次（`Drop` 置空）。
        unsafe { FreeLibrary(handle) };
    }

    pub(super) fn platform_last_error() -> String {
        // SAFETY: `GetLastError` 无指针参数，恒可调用。
        let code = unsafe { GetLastError() };
        format!("LoadLibraryA 失败（Win32 错误码 {code}）")
    }
}

use platform::{platform_close, platform_last_error, platform_open, platform_symbol};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_library_reports_load_error_not_panic() {
        let result = Library::open("/definitely/not/a/plugin.so");
        assert!(matches!(result, Err(HostError::LibraryLoad(_))));
    }

    #[test]
    fn nul_in_path_is_rejected() {
        let result = Library::open("bad\0path.so");
        assert!(matches!(result, Err(HostError::InvalidPluginPath(_))));
    }
}
