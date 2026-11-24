//! 屏幕工作区（work area）读取，以及"把窗口收进工作区"的**唯一**判据 —— T-FIX-224。
//!
//! ## 这一层在修什么（`ISS-MWTUUI0U-3599DC`）
//!
//! `.slint` 的 `preferred-width/height` 是**常量** 1600×920，`reset_window_layout`
//! 也把同一个常量喂给 `set_size`。常量与屏幕无关 ⇒ 在工作区逻辑高只有 724 的
//! 150% DPI 小屏上，窗口底边 196 逻辑像素从一开始就在屏幕外：「另存为模板」的
//! 动作行与主界面底部状态栏都点不到。实测读数（物理 2266×1146 / 逻辑 1510×764，
//! 任务栏占 60px）：窗口 2422×1436 物理，右溢 308、底溢 502。
//!
//! 本模块只做两件事：**读工作区**、**收敛尺寸**。调用点全部在
//! [`crate::host`]（`apply_window_layout` / `reset_window_layout` /
//! `fit_window_to_work_area`），界面层不读这里。
//!
//! ## 两个口径必须分开：整窗 vs 内容区
//!
//! - 系统的"工作区"（`SPI_GETWORKAREA` / `visibleFrame` / `_NET_WORKAREA`）
//!   说的是**整窗**可以占的屏幕区域；
//! - Slint 的 `Window::size()` / `set_size(LogicalSize)` 是**内容区**（winit 的
//!   inner size），不含边框与标题栏。
//!
//! 两者不换算就会出现"客户区明明收到 1510×724，`GetWindowRect` 仍是 2287×1142"
//! 的假通过（本机 144 DPI 实测差 22×86 物理像素，含原生菜单条 30px）。所以对外暴露的是
//! **可用内容区**（[`client_area_physical`]，已扣非客户区），而不是裸工作区。
//!
//! ## 口径：主屏 + 系统 DPI，两者必须同源
//!
//! 工作区与缩放比**不能**各取一个来源：混用会在"窗口在副屏、缩放取主屏"时算出
//! 一个既不属于这块屏也不属于那块屏的数。本模块统一取**主屏**工作区与其 DPI
//! （Windows 侧是 `SPI_GETWORKAREA` + `GetDpiForSystem`，二者都只认主屏/系统值）。
//!
//! ## 各平台的来源
//!
//! | 平台 | 工作区来源 | 原始口径 | 缩放来源 |
//! | --- | --- | --- | --- |
//! | Windows | `SystemParametersInfoW(SPI_GETWORKAREA)` | 物理像素（进程 per-monitor-v2 感知，系统不虚化） | `GetDpiForSystem()/96` |
//! | macOS | `NSScreen.mainScreen.visibleFrame` | 点（已扣菜单栏与 Dock） | 主屏 `backingScaleFactor` |
//! | Linux/X11 | 根窗口属性 `_NET_WORKAREA`（4×CARDINAL） | 根窗口像素 | 调用方递进来的窗口缩放比 |
//! | Linux/Wayland、其它 | 无可靠信号 ⇒ `None` | 不收敛（保持 `.slint` preferred） | — |
//!
//! Wayland 没有"全局工作区"这个概念（每个 surface 由合成器摆放），硬编一个数反而
//! 更糟 ⇒ 显式返回 `None`，调用方原样保留 preferred 尺寸，行为与本票之前一致。
//!
//! ## 为什么缩放比不问 Slint
//!
//! `run()` 之前 winit 窗口还没创建，`Window::scale_factor()` 返回默认 1.0；而
//! 收敛必须在**首帧之前**做完（否则用户先看到一帧溢出，再看着窗口缩一下）。
//! 所以 Windows 走 `GetDpiForSystem`，macOS 用主屏 backing，Linux 用调用方递进来的值。
//!
//! ## 已知边界
//!
//! - **非客户区只扣了 Windows**：macOS / Linux 这一项是 `(0, 0)` —— 两边的精确
//!   来源都要求"已经有窗口"（`+[NSWindow frameRectForContentRect:styleMask:]`
//!   要构造 `NSRect`；X11 的 `_NET_FRAME_EXTENTS` 要窗口 id），而收敛必须发生在
//!   首帧之前。⇒ 这两条腿在"刚好塞满工作区"的屏上仍会多出约一个标题栏，
//!   需要实机校准；本票不写"看着像对"的代码。
//! - 多显示器：只按主屏算。窗口在副屏且副屏更小时不会被收（留待后续票）。
//! - 最大化：恢复的布局若标记 `maximized`，尺寸由窗口管理器决定，本模块不插手。

/// 期望尺寸（逻辑像素）：与 `.slint` 的 `preferred-width/height` 同步维护。
///
/// ⛔ 这两个数**不再**被直接喂给 `set_size`：`reset_window_layout` 也走
/// [`fit_size`]，否则「恢复默认布局」会把窗口重新顶到屏幕外。
pub const PREFERRED: (f64, f64) = (1600.0, 920.0);

/// 最小尺寸（逻辑像素）：与 `.slint` 的 `min-width/min-height` 同步维护。
pub const MIN: (f64, f64) = (1200.0, 700.0);

/// 可用区域，**逻辑像素**，带原点（主屏工作区原点通常是 `(0, 0)`，
/// 任务栏在左/上时不是）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    /// 区域左上角的 x（屏幕坐标）。
    pub x: f64,
    /// 区域左上角的 y（屏幕坐标）。
    pub y: f64,
    /// 可用宽度。
    pub width: f64,
    /// 可用高度。
    pub height: f64,
}

impl Area {
    /// 以 `(x, y)` 为原点的可用区域。
    #[must_use]
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// 区域是否装得下给定尺寸（两个方向都不超；容忍半像素的折算零头）。
    #[must_use]
    pub fn fits(&self, size: (f64, f64)) -> bool {
        size.0 <= self.width + 0.5 && size.1 <= self.height + 0.5
    }

    /// 把 `size` 居中放进本区域时的左上角（放不下就贴左上）。
    #[must_use]
    pub fn center_for(&self, size: (f64, f64)) -> (f64, f64) {
        let x = self.x + ((self.width - size.0) / 2.0).max(0.0);
        let y = self.y + ((self.height - size.1) / 2.0).max(0.0);
        (x.floor(), y.floor())
    }
}

/// 同一块区域的**物理**像素形态（给恢复布局那条路用 —— 存下来的几何是物理像素）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicalArea {
    /// 左上角 x（屏幕坐标，物理像素）。
    pub x: i32,
    /// 左上角 y。
    pub y: i32,
    /// 宽。
    pub width: u32,
    /// 高。
    pub height: u32,
}

/// 读主屏**可用内容区**（逻辑像素，已扣非客户区）。
///
/// `ui_scale` 是调用方手上的窗口缩放比（`Window::scale_factor()`）；只有 Linux
/// 会用它，Windows/macOS 各自从系统取更可靠的值（见模块文档）。
///
/// `None` = 本平台没有可靠信号，调用方保持现状（⛔ 不要拿一个猜的数字代替）。
#[must_use]
pub fn primary_logical(ui_scale: f32) -> Option<Area> {
    let physical = client_area_physical()?;
    let scale = effective_scale(ui_scale);
    if !(scale.is_finite() && scale > 0.05) {
        return None;
    }
    Some(Area {
        x: f64::from(physical.x) / scale,
        y: f64::from(physical.y) / scale,
        width: f64::from(physical.width) / scale,
        height: f64::from(physical.height) / scale,
    })
}

/// 主屏**可用内容区**的物理像素形态 = 工作区 − 非客户区（边框 + 标题栏）。
///
/// 为什么要减：见模块文档"两个口径必须分开"。`AdjustWindowRectExForDpi` 在本机
/// 144 DPI 下给出的非客户区是 22×86 物理像素（含原生菜单条，见 [`frame_physical`]），
/// 与真实窗口的 `GetWindowRect − 客户区` 逐像素吻合。
#[must_use]
pub fn client_area_physical() -> Option<PhysicalArea> {
    let work = primary_work_area_physical()?;
    let (frame_w, frame_h) = frame_physical();
    let width = work.width.saturating_sub(frame_w);
    let height = work.height.saturating_sub(frame_h);
    if width == 0 || height == 0 {
        return None;
    }
    Some(PhysicalArea {
        width,
        height,
        ..work
    })
}

/// 把期望尺寸收进可用区域。
///
/// 顺序是 **min → max(下限) → min(区域)**：先按区域封顶，再抬到最小尺寸，
/// 最后再压一次 —— 区域比 `min-*` 还小时以区域为准（宁可小于最小尺寸，
/// 也不许溢出屏幕，那正是本票要修的形态）。
///
/// 向下取整到整逻辑像素：Slint 的 `LogicalSize` 是 `f32`，留小数会让
/// 「窗口右沿」落在半像素上，截图读数对不齐。
#[must_use]
pub fn fit_size(wanted: (f64, f64), min: (f64, f64), area: Area) -> (f64, f64) {
    let w = wanted.0.min(area.width).max(min.0).min(area.width).max(1.0);
    let h = wanted
        .1
        .min(area.height)
        .max(min.1)
        .min(area.height)
        .max(1.0);
    (w.floor(), h.floor())
}

/// [`PREFERRED`] 的便捷形态：装得下就原样返回 1600×920，装不下才收。
///
/// 返回 `None` = 读不到工作区，调用方**不要**动窗口（大屏行为不变的字面含义：
/// 连一次多余的 `set_size` 都不发）。
#[must_use]
pub fn fit_preferred(ui_scale: f32) -> Option<(f64, f64)> {
    let area = primary_logical(ui_scale)?;
    if area.fits(PREFERRED) {
        return Some(PREFERRED);
    }
    Some(fit_size(PREFERRED, MIN, area))
}

/// 恢复布局的几何按**物理**口径夹：尺寸不超可用区，位置推回区内。
///
/// 为什么是物理：[`crate::settings::WindowGeometry`] 存的是 `Window::size()/
/// position()` 的原值（物理像素），`apply_window_layout` 也用
/// `PhysicalSize`/`PhysicalPosition` 写回。换成逻辑再换回去只会多一次舍入。
///
/// 位置也要夹 —— 只夹尺寸不够：本机实测的恢复几何是 `(152,152)` 起、2422 宽，
/// 即使把宽度收到 2266，右沿仍是 152+2266 > 2266。
///
/// 区域读不到时调用方传 [`PhysicalArea`] 的零值，本函数原样返回（别乱动）。
#[must_use]
pub fn fit_physical_rect(
    size: (u32, u32),
    pos: (i32, i32),
    area: PhysicalArea,
) -> ((u32, u32), (i32, i32)) {
    if area.width == 0 || area.height == 0 {
        return (size, pos);
    }
    let w = size.0.min(area.width);
    let h = size.1.min(area.height);
    // 用 i64 算上界：窗口比区域还大时 `area - self` 会变成负数，i32 减法没错但
    // 语义要显式表达成"夹到区域原点"。
    let max_x = i64::from(area.x) + i64::from(area.width) - i64::from(w);
    let max_y = i64::from(area.y) + i64::from(area.height) - i64::from(h);
    let x = i64::from(pos.0).clamp(i64::from(area.x), max_x.max(i64::from(area.x)));
    let y = i64::from(pos.1).clamp(i64::from(area.y), max_y.max(i64::from(area.y)));
    ((w, h), (x as i32, y as i32))
}

/// 主屏**整窗**工作区的物理像素（未扣非客户区）。
#[cfg(windows)]
fn primary_work_area_physical() -> Option<PhysicalArea> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETWORKAREA, SystemParametersInfoW};

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: `rect` 是本次调用独占的栈上 `RECT`，`SPI_GETWORKAREA` 只写它
    // （第三个参数在文档里就是 `RECT*`），不持有指针、不与别的线程共享。
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            (&mut rect as *mut RECT).cast::<core::ffi::c_void>(),
            0,
        )
    };
    if ok == 0 {
        return None;
    }
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(PhysicalArea {
        x: rect.left,
        y: rect.top,
        // 已判过 > 0 ⇒ i32 → u32 无损。
        width: width as u32,
        height: height as u32,
    })
}

/// Windows 的缩放比：系统 DPI / 96。
///
/// `run()` 之前 Slint 的窗口还没建，`scale_factor()` 是 1.0；`GetDpiForSystem`
/// 不受这个影响，且与 `SPI_GETWORKAREA` 同为主屏口径。
#[cfg(windows)]
fn effective_scale(_ui_scale: f32) -> f64 {
    let dpi = system_dpi();
    if dpi == 0 { 1.0 } else { f64::from(dpi) / 96.0 }
}

#[cfg(windows)]
fn system_dpi() -> u32 {
    use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;

    // SAFETY: 无参、无副作用、不碰任何指针。
    unsafe { GetDpiForSystem() }
}

/// Windows：非客户区（左右边框 + 标题栏 + 下边框 + **原生菜单条**）的物理尺寸。
///
/// 用 `AdjustWindowRectExForDpi` 按 `WS_OVERLAPPEDWINDOW` 算，而不是拿
/// `GetSystemMetrics` 的四个分量自己拼：后者要判断"要不要加 padded border、
/// 下边框算不算标题栏、菜单条在不在"，而前者是系统对"这个样式在这个 DPI 下
/// 会被加多少"的唯一权威答案。
///
/// `b_menu` 必须传 `TRUE`：根窗口**无条件**声明了 `MenuBar`（`ui/app.slint:1187`），
/// Windows 上 Slint 把它做成真正的 Win32 菜单（不画在客户区里），菜单条高度属于
/// 非客户区 —— 漏掉它就是本机实测的 30px（`SM_CYMENU@144 = 30`）。两个口径的
/// 差别是可验证的：
///
/// | 口径 | 结果（144 DPI） |
/// | --- | --- |
/// | `AdjustWindowRectExForDpi(b_menu=FALSE)` | 22×56 |
/// | `AdjustWindowRectExForDpi(b_menu=TRUE)` | 22×**86** |
/// | 真实窗口 `GetWindowRect − GetClientRect` | 22×**86** |
///
/// 传 `TRUE` 的**失效方向是安全的**：万一某天原生菜单没建起来（走了 Slint 的
/// 客户区内回退实现），这里会高估 30px ⇒ 窗口比"刚好塞满"再小一点点，不会溢出。
#[cfg(windows)]
fn frame_physical() -> (u32, u32) {
    use windows_sys::Win32::Foundation::RECT;
    // `AdjustWindowRectExForDpi` 在 windows-sys 里挂在 `UI::HiDpi`（不在
    // `WindowsAndMessaging` —— 0.61 的分组如此），两个 feature 都已启用。
    use windows_sys::Win32::UI::HiDpi::AdjustWindowRectExForDpi;
    use windows_sys::Win32::UI::WindowsAndMessaging::WS_OVERLAPPEDWINDOW;

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: `rect` 是本次调用独占的栈上 `RECT`，函数只往里写；样式与扩展样式
    // 都是常量，不持有指针、不跨线程。
    let ok =
        unsafe { AdjustWindowRectExForDpi(&mut rect, WS_OVERLAPPEDWINDOW, 1, 0, system_dpi()) };
    if ok == 0 {
        // 算不出来就按 0 处理：宁可少让几个像素，也不要凭空编一个边框宽度。
        return (0, 0);
    }
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    if w < 0 || h < 0 {
        return (0, 0);
    }
    (w as u32, h as u32)
}

/// macOS：`NSScreen.mainScreen.visibleFrame`。
///
/// `visibleFrame` 是**点**（逻辑像素，已扣菜单栏与 Dock），乘上主屏 backing
/// 缩放比才与 Windows 侧同为物理像素 —— 全模块统一"物理进来，除以缩放比变逻辑"。
#[cfg(target_os = "macos")]
fn primary_work_area_physical() -> Option<PhysicalArea> {
    let (w, h, backing) = macos_visible_frame()?;
    if !(backing.is_finite() && backing > 0.05) {
        return None;
    }
    let (w, h) = (w * backing, h * backing);
    if !(w.is_finite() && h.is_finite() && w >= 1.0 && h >= 1.0) {
        return None;
    }
    // 屏幕原点由 AppKit 定（主屏原点为 (0,0)，上方屏幕 y 为负），这里不拿它做
    // 位置换算 —— 恢复布局那条路只在"钳到区内"时用到原点，见 `fit_physical_rect`。
    Some(PhysicalArea {
        x: 0,
        y: 0,
        width: round_to_u32(w),
        height: round_to_u32(h),
    })
}

#[cfg(target_os = "macos")]
fn macos_visible_frame() -> Option<(f64, f64, f64)> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;

    // 工作区只在主线程问（NSScreen 是 MainThreadOnly）：调用点都在 `run()` 之前
    // 的主线程上；`new()` 返回 None 说明不在主线程 ⇒ 老实放弃。
    let mtm = MainThreadMarker::new()?;
    let screen = NSScreen::mainScreen(mtm)?;
    let frame = screen.visibleFrame();
    Some((
        frame.size.width,
        frame.size.height,
        screen.backingScaleFactor(),
    ))
}

/// macOS 的缩放比 = 主屏 backing scale（Retina 为 2.0，普通屏 1.0）。
#[cfg(target_os = "macos")]
fn effective_scale(_ui_scale: f32) -> f64 {
    macos_visible_frame()
        .map(|(_, _, backing)| backing)
        .filter(|backing| backing.is_finite() && *backing > 0.05)
        .unwrap_or(1.0)
}

/// macOS：非客户区按 0 处理（见模块文档"已知边界"）。
#[cfg(target_os = "macos")]
fn frame_physical() -> (u32, u32) {
    (0, 0)
}

/// Linux/X11：根窗口属性 `_NET_WORKAREA`。
///
/// 值是 4 个 CARDINAL：`[x, y, w, h]`，单位是根窗口像素。没有窗口管理器
/// （或 Wayland 下 `DISPLAY` 打不开）⇒ `None`。
#[cfg(target_os = "linux")]
fn primary_work_area_physical() -> Option<PhysicalArea> {
    use std::ffi::c_void;
    use std::os::raw::{c_long, c_ulong};
    use std::ptr;
    use x11_dl::xlib;

    let xlib = xlib::Xlib::open().ok()?;
    // SAFETY: 下面每个 Xlib 调用都按 EWMH 的用法传参；本轮申请的两块内存
    // （`display`、`prop`）在同一个 `unsafe` 块里配对释放（`XCloseDisplay` /
    // `XFree`），中途任何一条早退都先释放再返回。
    unsafe {
        let display = (xlib.XOpenDisplay)(ptr::null());
        if display.is_null() {
            return None;
        }
        let root = (xlib.XDefaultRootWindow)(display);
        let atom = (xlib.XInternAtom)(display, c"_NET_WORKAREA".as_ptr(), 0);
        if atom == 0 {
            (xlib.XCloseDisplay)(display);
            return None;
        }
        let mut actual_type: c_ulong = 0;
        let mut actual_format: i32 = 0;
        let mut nitems: c_ulong = 0;
        let mut bytes_after: c_ulong = 0;
        let mut prop: *mut u8 = ptr::null_mut();
        let status = (xlib.XGetWindowProperty)(
            display,
            root,
            atom,
            0,
            4,
            0,
            xlib::XA_CARDINAL,
            &mut actual_type,
            &mut actual_format,
            &mut nitems,
            &mut bytes_after,
            &mut prop,
        );
        let ok = status == 0 && !prop.is_null() && nitems >= 4 && actual_format == 32;
        let values = if ok {
            // 32 位 CARDINAL 在 Xlib 里按 long 存放 ⇒ 按 `c_long` 读。
            let values = prop.cast::<c_long>();
            Some((*values, *values.add(1), *values.add(2), *values.add(3)))
        } else {
            None
        };
        if !prop.is_null() {
            (xlib.XFree)(prop.cast::<c_void>());
        }
        (xlib.XCloseDisplay)(display);

        let (x, y, w, h) = values?;
        if w <= 0 || h <= 0 {
            return None;
        }
        Some(PhysicalArea {
            x: i32::try_from(x).unwrap_or(0),
            y: i32::try_from(y).unwrap_or(0),
            width: u32::try_from(w).unwrap_or(0),
            height: u32::try_from(h).unwrap_or(0),
        })
    }
}

/// Linux：`_NET_WORKAREA` 是根窗口像素 ⇒ 除以窗口缩放比折成逻辑像素。
#[cfg(target_os = "linux")]
fn effective_scale(ui_scale: f32) -> f64 {
    if ui_scale.is_finite() && ui_scale > 0.05 {
        f64::from(ui_scale)
    } else {
        1.0
    }
}

/// Linux：非客户区按 0 处理（见模块文档"已知边界"）。
#[cfg(target_os = "linux")]
fn frame_physical() -> (u32, u32) {
    (0, 0)
}

/// 没有实现平台读取的其余目标（BSD 等）：不收敛。
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn primary_work_area_physical() -> Option<PhysicalArea> {
    None
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn effective_scale(_ui_scale: f32) -> f64 {
    1.0
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn frame_physical() -> (u32, u32) {
    (0, 0)
}

#[cfg(target_os = "macos")]
fn round_to_u32(value: f64) -> u32 {
    // 点宽是 f64（CGFloat）；负数与 NaN 已在调用方挡掉。
    value.round().clamp(1.0, f64::from(u32::MAX)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用的矩形区域（原点在主屏左上）。
    fn area(w: f64, h: f64) -> Area {
        Area::new(0.0, 0.0, w, h)
    }

    fn physical(w: u32, h: u32) -> PhysicalArea {
        PhysicalArea {
            x: 0,
            y: 0,
            width: w,
            height: h,
        }
    }

    #[test]
    fn big_screen_keeps_preferred() {
        // 主流形态：可用区装得下 1600×920 ⇒ 一个像素都不动。
        let region = area(1920.0, 1057.0);
        assert_eq!(fit_size(PREFERRED, MIN, region), PREFERRED);
        assert!(region.fits(PREFERRED));
    }

    #[test]
    fn reported_150pct_screen_shrinks_to_work_area() {
        // 上报环境：1510×724 逻辑可用区（2266×1086 物理 @150%，已扣 22×86 非客户区）。
        let region = area(1496.0, 686.0);
        let (w, h) = fit_size(PREFERRED, MIN, region);
        assert_eq!((w, h), (1496.0, 686.0));
        assert!(region.fits((w, h)));
    }

    #[test]
    fn floor_keeps_edges_on_whole_pixels() {
        // 除不尽的逻辑宽（1510.666…）：向下取整，避免半像素右沿。
        let region = area(1510.666667, 700.0);
        assert_eq!(fit_size(PREFERRED, MIN, region).0, 1510.0);
    }

    #[test]
    fn min_size_wins_when_work_area_is_roomier_than_min() {
        // 区域比下限大、比期望小 ⇒ 收到区域，而不是掉到 1200×700。
        let region = area(1400.0, 800.0);
        assert_eq!(fit_size(PREFERRED, MIN, region), (1400.0, 800.0));
    }

    #[test]
    fn work_area_below_min_yields_work_area_not_min() {
        // 区域比 min-* 还小：宁可小于最小尺寸，也不许溢出屏幕。
        let region = area(1024.0, 600.0);
        assert_eq!(fit_size(PREFERRED, MIN, region), (1024.0, 600.0));
    }

    #[test]
    fn fits_tolerates_half_pixel() {
        // 浮点折算常留 0.0001 的零头，不该判成"装不下"。
        let region = area(1599.9999, 919.9999);
        assert!(region.fits(PREFERRED));
    }

    #[test]
    fn center_for_puts_window_inside_the_area() {
        let region = area(1496.0, 686.0);
        let (x, y) = region.center_for((1496.0, 686.0));
        assert_eq!((x, y), (0.0, 0.0));
        // 小于区域时居中；原点不为 0 时跟着偏移。
        let offset = Area::new(100.0, 50.0, 1000.0, 800.0);
        assert_eq!(offset.center_for((800.0, 600.0)), (200.0, 150.0));
    }

    #[test]
    fn restored_geometry_is_clamped_in_physical_pixels() {
        // 本机实测的恢复几何：2422×1436 @ (152,152)，可用区 2266×1086。
        let ((w, h), (x, y)) = fit_physical_rect((2422, 1436), (152, 152), physical(2266, 1086));
        assert_eq!((w, h), (2266, 1086));
        assert_eq!((x, y), (0, 0));
    }

    #[test]
    fn restored_geometry_inside_work_area_is_untouched() {
        let size = (1600, 920);
        let pos = (40, 60);
        assert_eq!(
            fit_physical_rect(size, pos, physical(2266, 1086)),
            (size, pos)
        );
        // 右沿刚好贴边也算"在区内"（2266 − 1600 = 666）。
        let edge = (2266 - 1600, 0);
        assert_eq!(
            fit_physical_rect(size, edge, physical(2266, 1086)),
            (size, edge)
        );
    }

    #[test]
    fn restored_geometry_with_negative_position_is_pushed_back() {
        // 上次在左侧副屏（x 为负）⇒ 推回主屏左上角。
        let ((w, h), (x, y)) = fit_physical_rect((1600, 920), (-1200, -50), physical(2266, 1086));
        assert_eq!((w, h), (1600, 920));
        assert_eq!((x, y), (0, 0));
    }

    #[test]
    fn restored_geometry_respects_non_zero_work_area_origin() {
        // 任务栏在左/上时工作区原点不是 0：窗口不该被推到 (0,0)。
        let origin = PhysicalArea {
            x: 80,
            y: 40,
            width: 2000,
            height: 1000,
        };
        let ((_, _), (x, y)) = fit_physical_rect((1600, 900), (-500, -500), origin);
        assert_eq!((x, y), (80, 40));
    }

    #[test]
    fn no_work_area_means_no_touching() {
        assert_eq!(
            fit_physical_rect((2422, 1436), (152, 152), physical(0, 0)),
            ((2422, 1436), (152, 152))
        );
    }
}
