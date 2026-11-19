//! 最小动态库加载器（std＋平台 `dlopen`／`LoadLibraryW`，零第三方依赖）。
//!
//! ⚠ 本模块的 `extern` 声明只覆盖一期用到的 4 个 Unix 符号与 3 个 Windows 符号；
//! 不做通用 FFI 封装。Windows 上使用 `LoadLibraryW` 传入 UTF-16 路径，
//! 完整支持含非 ASCII 字符的插件路径。

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
    /// 打开动态库（`dlopen`／`LoadLibraryW`）。
    ///
    /// # Errors
    ///
    /// 路径含 NUL ⇒ [`HostError::InvalidPluginPath`]；
    /// 打开失败 ⇒ [`HostError::LibraryLoad`]（含系统错误文本）。
    pub fn open(path: &str) -> Result<Self, HostError> {
        let handle = platform_open(path)?;
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
    use super::{HostError, c_void};
    use std::ffi::{CString, c_char};

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

    pub(super) fn platform_open(path: &str) -> Result<*mut c_void, HostError> {
        let c_path = CString::new(path)
            .map_err(|_| HostError::InvalidPluginPath(format!("路径含 NUL 字节: {path}")))?;
        // SAFETY: `c_path` 来自 `CString`（NUL 结尾），`dlopen` 只读。
        let handle = unsafe { dlopen(c_path.as_ptr(), RTLD_NOW) };
        if handle.is_null() {
            return Err(HostError::LibraryLoad(platform_last_error()));
        }
        Ok(handle)
    }

    pub(super) unsafe fn platform_symbol(handle: *mut c_void, name: *const c_char) -> *mut c_void {
        // SAFETY: `handle` 是 `dlopen` 返回的有效句柄，`name` 是 NUL 结尾串。
        unsafe { dlsym(handle, name) }
    }

    pub(super) unsafe fn platform_close(handle: *mut c_void) {
        // SAFETY: `handle` 是 `dlopen` 返回的有效句柄，且只关一次（`Drop` 置空）。
        unsafe { dlclose(handle) };
    }

    fn platform_last_error() -> String {
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
    use super::{HostError, c_void};
    use std::ffi::{OsStr, c_char};
    use std::os::windows::ffi::OsStrExt;

    // SAFETY: `kernel32` 的公开接口；`LoadLibraryW` 接受 NUL 结尾 UTF-16 路径。
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
        fn FreeLibrary(module: *mut c_void) -> i32;
    }

    pub(super) fn platform_open(path: &str) -> Result<*mut c_void, HostError> {
        if path.contains('\0') {
            return Err(HostError::InvalidPluginPath(format!(
                "路径含 NUL 字节: {path}"
            )));
        }
        let wide: Vec<u16> = OsStr::new(path).encode_wide().chain(Some(0)).collect();
        // SAFETY: `wide` 是 NUL 结尾的 UTF-16 缓冲且在调用期间存活，`LoadLibraryW` 只读。
        let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
        if handle.is_null() {
            let err = std::io::Error::last_os_error();
            return Err(HostError::LibraryLoad(format!("LoadLibraryW 失败: {err}")));
        }
        Ok(handle)
    }

    pub(super) unsafe fn platform_symbol(handle: *mut c_void, name: *const c_char) -> *mut c_void {
        // SAFETY: `handle` 是 `LoadLibraryW` 返回的有效句柄，`name` 是 NUL 结尾 ANSI 串。
        unsafe { GetProcAddress(handle, name) }
    }

    pub(super) unsafe fn platform_close(handle: *mut c_void) {
        // SAFETY: `handle` 有效且只关一次（`Drop` 置空）。
        unsafe { FreeLibrary(handle) };
    }
}

use platform::{platform_close, platform_open, platform_symbol};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_library_reports_load_error_not_panic() {
        let result = Library::open("/definitely/not/a/plugin.so");
        assert!(matches!(result, Err(HostError::LibraryLoad(_))));
    }

    #[test]
    fn non_ascii_missing_library_reports_load_error_not_panic() {
        let result = Library::open("/definitely/not/a/中文_插件.so");
        assert!(matches!(result, Err(HostError::LibraryLoad(_))));
    }

    #[test]
    fn nul_in_path_is_rejected() {
        let result = Library::open("bad\0path.so");
        assert!(matches!(result, Err(HostError::InvalidPluginPath(_))));
    }
}
