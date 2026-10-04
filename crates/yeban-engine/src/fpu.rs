//! CPU 浮点环境开关：FTZ (Flush-To-Zero) 与 DAZ (Denormals-Are-Zero)。
//! [ARCH-RT-003, ROAD-M2-003]
//!
//! ## 为什么必须做
//!
//! 非正规数（subnormal / denormal，绝对值约 `< 1.18e-38`）在多数 x86 与 ARM 核上会触发
//! **微码辅助路径**，单条浮点指令的周期数可以暴涨两个数量级。混响尾巴、包络释放段
//! 与滤波器状态变量最容易落进这个区间，表现为"某个音符结束后 CPU 突然飙升"。
//! 规范把它列为强制项：x86 设置 MXCSR 的 bit 15 (FTZ) 与 bit 6 (DAZ)，
//! ARM64 设置 FPCR 的 FZ 位 [ARCH-RT-003]。
//!
//! ## 本模块是 `unsafe` 的**白名单**地点之一
//!
//! [AGENTS.md §2 红线 8] 要求 `unsafe` 必须带 `// SAFETY:` 证明。本模块只有两类操作：
//! 读写 CPU 控制寄存器（x86 用 `stmxcsr`/`ldmxcsr`，aarch64 用 `mrs`/`msr`，都是内联汇编；
//! 不用 `core::arch::_mm_getcsr/_mm_setcsr` —— 它们自 Rust 1.75 起已 deprecated）。
//! 两者都**不触碰内存**、不产生别名、不改变栈，唯一的副作用是当前线程的浮点舍入行为
//! ——这正是本模块的语义。控制寄存器是**线程局部**状态（POSIX 规定新线程继承创建者
//! 的浮点环境），因此必须在音频回调**线程内部**调用，而不是在打开设备的主线程上调用。
//!
//! ## 平台覆盖
//!
//! | 目标 | 实现 | 说明 |
//! | :--- | :--- | :--- |
//! | `x86_64` | 内联汇编 `stmxcsr` / `ldmxcsr` | SSE 是 x86-64 架构基线，无需运行期探测 |
//! | `x86` | 同上 + `is_x86_feature_detected!("sse")` | 无 SSE 时执行会 `SIGILL`，故先探测 |
//! | `aarch64` | `mrs fpcr` / `msr fpcr` | AArch64 的浮点/AdvSIMD 是 ABI 强制项 |
//! | 其它 | **安全空实现** | 返回 [`FtzDazOutcome::Unsupported`]，绝不 panic、不动寄存器 |
//!
//! [ARCH-DET-001] 的 L1 确定性也依赖这里：FTZ/DAZ 的开关状态影响最低有效位，
//! 因此实时引擎与离线渲染器必须在**同一状态**下工作（离线侧同样调用本函数即可）。

/// [`enable_ftz_daz`] 的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FtzDazOutcome {
    /// 已写入本线程的浮点控制寄存器。
    Applied,
    /// 当前平台没有等价的 FTZ/DAZ 控制位——已执行安全空实现。
    Unsupported,
}

/// `true` 表示当前编译目标在**架构层面**保证有 FTZ/DAZ 控制位。
///
/// 只对 `x86_64` 与 `aarch64` 返回 `true`。32 位 `x86` 的 SSE（进而是 MXCSR）是运行期
/// 探测出来的，因此这里保守地返回 `false`；那种平台上 [`enable_ftz_daz`] 仍可能返回
/// [`FtzDazOutcome::Applied`]。
#[must_use]
pub const fn supports_ftz_daz() -> bool {
    cfg!(any(target_arch = "x86_64", target_arch = "aarch64"))
}

/// 在本线程上开启 FTZ + DAZ [ARCH-RT-003]。
///
/// 幂等：重复调用只是重复写同一个值。实时回调可以在开头配合一个普通 `bool` 标志
/// 做到"每线程一次"，不必依赖 `Once`——`Once` 在竞争路径上会阻塞等待，
/// 属于红线 7 禁止的"锁等待"。
pub fn enable_ftz_daz() -> FtzDazOutcome {
    imp::enable()
}

/// 关闭 FTZ + DAZ（清位），返回调用前是否处于开启状态。
///
/// **不在实时路径上使用**：规范只要求"开启"。本函数用于给测试提供可逆性证据
/// （证明置位是真实的，而不是别处遗留的状态），以及给需要逐位复现非正规数行为的
/// 离线数值对账留一个开关。
pub fn disable_ftz_daz() -> bool {
    imp::disable()
}

/// 读取当前线程的 FTZ/DAZ 状态。
///
/// - `Some(true)`：x86 的 FTZ+DAZ 两位（或 aarch64 的 FZ 位）都已置位；
/// - `Some(false)`：可读取，但未开启；
/// - `None`：当前平台没有等价控制位。
#[must_use]
pub fn ftz_daz_enabled() -> Option<bool> {
    imp::enabled()
}

// ---------------------------------------------------------------------------
// 平台实现: 每个 cfg 分支一个独立模块, 避免"永远不可达"的代码路径
// (如果用 cfg 块 + 尾部 fallback, 在 x86_64 上会触发 unreachable_code 警告,
//  而 DoD 要求 clippy -D warnings 零告警)。
// ---------------------------------------------------------------------------

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
use self::imp_x86 as imp;

#[cfg(target_arch = "aarch64")]
use self::imp_aarch64 as imp;

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
use self::imp_none as imp;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod imp_x86 {
    use super::FtzDazOutcome;

    /// MXCSR 的 FTZ 位（bit 15）。
    pub(super) const FTZ: u32 = 1 << 15;
    /// MXCSR 的 DAZ 位（bit 6）。
    pub(super) const DAZ: u32 = 1 << 6;

    /// 32 位 x86 上 SSE 可选：无 SSE 就没有 MXCSR，执行 `stmxcsr` 会触发非法指令异常。
    /// x86_64 上 SSE 是架构基线，恒为 `true`。
    #[cfg(target_arch = "x86")]
    fn mxcsr_available() -> bool {
        std::arch::is_x86_feature_detected!("sse")
    }

    #[cfg(target_arch = "x86_64")]
    const fn mxcsr_available() -> bool {
        true
    }

    /// 读 MXCSR（`stmxcsr`）。
    ///
    /// 用内联汇编而不是 `core::arch::x86_64::_mm_getcsr`：后者自 Rust 1.75 起被标记为
    /// **deprecated**（"use inline assembly instead"），而本仓库把 `-D warnings` 作为
    /// DoD，因此必须走汇编。这也正是官方推荐的替代做法。
    fn read_mxcsr() -> u32 {
        let mut csr: u32 = 0;
        // SAFETY: `stmxcsr [mem]` 把 32 位 MXCSR 写入给定地址。
        // - 目标是我们自己的栈上局部变量 `csr`（`&raw mut` 取地址 ⇒ 编译器会把它落实在内存里，
        //   不会只留在寄存器里）；写入宽度 32 位与 `u32` 完全一致，不越界。
        // - 不声明 `nomem`：该指令**写内存**，必须让编译器知道内存被改过（否则 `csr` 的读取
        //   可能被优化成常量）。
        // - `nostack` 是准确的：汇编本身不压栈/弹栈；`preserves_flags` 也准确 ——
        //   `stmxcsr` 不修改 EFLAGS。
        // - 对 MXCSR 的读写只影响**当前线程**的浮点环境，不产生跨线程别名。
        unsafe {
            core::arch::asm!(
                "stmxcsr [{ptr}]",
                ptr = in(reg) &raw mut csr,
                options(nostack, preserves_flags)
            );
        }
        csr
    }

    /// 写 MXCSR（`ldmxcsr`）。
    fn write_mxcsr(value: u32) {
        // SAFETY: `ldmxcsr [mem]` 从给定地址读 32 位写入 MXCSR。
        // - 源是函数参数 `value` 的地址（`&raw const` ⇒ 编译器会把它落实在内存里）；
        //   读取宽度 32 位与 `u32` 一致。
        // - **故意不声明 `nomem` 也不声明 `readonly`**：这条汇编没有输出操作数，
        //   若把它标记成"无内存副作用"，LLVM 就有权把它当死代码删掉 ——
        //   那会让 FTZ/DAZ 的设置**静默失效**。保留未建模的内存副作用即禁止消除。
        // - `nostack` / `preserves_flags` 是准确的（`ldmxcsr` 不压栈、不改 EFLAGS）。
        // - 只影响当前线程的浮点环境；不动内存所有权、不产生别名。
        unsafe {
            core::arch::asm!(
                "ldmxcsr [{ptr}]",
                ptr = in(reg) &raw const value,
                options(nostack, preserves_flags)
            );
        }
    }

    pub(super) fn enable() -> FtzDazOutcome {
        if !mxcsr_available() {
            return FtzDazOutcome::Unsupported;
        }
        // 只做按位或置位 FTZ/DAZ，保留调用方已有的舍入模式与异常屏蔽位。
        write_mxcsr(read_mxcsr() | FTZ | DAZ);
        FtzDazOutcome::Applied
    }

    pub(super) fn disable() -> bool {
        if !mxcsr_available() {
            return false;
        }
        let csr = read_mxcsr();
        let was_enabled = csr & (FTZ | DAZ) != 0;
        write_mxcsr(csr & !(FTZ | DAZ));
        was_enabled
    }

    pub(super) fn enabled() -> Option<bool> {
        if !mxcsr_available() {
            return None;
        }
        let csr = read_mxcsr();
        Some(csr & (FTZ | DAZ) == (FTZ | DAZ))
    }
}

#[cfg(target_arch = "aarch64")]
mod imp_aarch64 {
    use super::FtzDazOutcome;

    /// FPCR 的 FZ 位（bit 24）。
    const FZ: u64 = 1 << 24;

    /// 读 FPCR。
    fn read_fpcr() -> u64 {
        let fpcr: u64;
        // SAFETY: `mrs <reg>, fpcr` 读取 AArch64 浮点控制寄存器。
        // - `options(nomem, nostack, preserves_flags)` 的声明都是准确的：
        //   不访问内存、不修改栈、不修改 NZCV 条件标志。
        // - 输出是 `u64` 通用寄存器，与 fpcr 的 64 位宽度匹配。
        // - AArch64 上浮点/AdvSIMD 是 ABI 强制要求（AAPCS64 hard-float），
        //   因此该指令不会产生未定义指令异常。
        unsafe {
            core::arch::asm!(
                "mrs {fpcr}, fpcr",
                fpcr = out(reg) fpcr,
                options(nomem, nostack, preserves_flags)
            );
        }
        fpcr
    }

    /// 写 FPCR。
    fn write_fpcr(value: u64) {
        // SAFETY: `msr fpcr, <reg>` 写入 AArch64 浮点控制寄存器。
        // - 参数是通用寄存器里的位模式，不涉及内存；`nomem/nostack/preserves_flags`
        //   的声明准确。
        // - 只按位改写调用方给出的值（本模块只在 FZ 位上做与/或），保留舍入模式
        //   与异常陷阱使能等其它控制位。
        unsafe {
            core::arch::asm!(
                "msr fpcr, {fpcr}",
                fpcr = in(reg) value,
                options(nomem, nostack, preserves_flags)
            );
        }
    }

    pub(super) fn enable() -> FtzDazOutcome {
        write_fpcr(read_fpcr() | FZ);
        FtzDazOutcome::Applied
    }

    pub(super) fn disable() -> bool {
        let fpcr = read_fpcr();
        let was_enabled = fpcr & FZ != 0;
        write_fpcr(fpcr & !FZ);
        was_enabled
    }

    pub(super) fn enabled() -> Option<bool> {
        Some(read_fpcr() & FZ != 0)
    }
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
mod imp_none {
    use super::FtzDazOutcome;

    /// 其它架构：没有等价的 FTZ/DAZ 控制位，安全空实现。
    pub(super) fn enable() -> FtzDazOutcome {
        FtzDazOutcome::Unsupported
    }

    /// 其它架构：什么都没做，因此"此前是否开启"恒为 `false`。
    pub(super) fn disable() -> bool {
        false
    }

    /// 其它架构：无法读取，返回 `None`。
    pub(super) fn enabled() -> Option<bool> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 判据 (e)：开关调用本身**绝不 panic**，且在同一线程上幂等。
    ///
    /// 这条判据在任何架构上都执行（包括安全空实现的平台）。测试结束前把线程的
    /// 浮点环境复原成进入时的样子，避免影响同进程其它测试。
    #[test]
    fn enable_ftz_daz_never_panics_and_is_idempotent() {
        let before = ftz_daz_enabled();
        let first = enable_ftz_daz();
        let second = enable_ftz_daz();
        assert_eq!(first, second, "幂等: 同一线程重复调用结果一致");
        if !supports_ftz_daz() {
            // 32 位 x86 允许 Applied 或 Unsupported（取决于运行期 SSE 探测）。
            assert!(
                matches!(first, FtzDazOutcome::Applied | FtzDazOutcome::Unsupported),
                "非保证支持的架构只能返回 Applied 或 Unsupported"
            );
        } else {
            assert_eq!(first, FtzDazOutcome::Applied);
        }
        // 复原
        match before {
            Some(true) => {
                enable_ftz_daz();
            }
            Some(false) => {
                disable_ftz_daz();
            }
            None => {}
        }
    }

    /// 判据 (e) 的架构分支：在 `x86_64` / `aarch64` 上**确实置位**，并且可逆。
    ///
    /// MXCSR / FPCR 是线程局部状态，所以这条测试不会干扰其它测试线程；
    /// 仍然在结束前复原进入时的状态。
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    #[test]
    fn ftz_daz_bits_are_really_set_on_supported_arches() {
        let before = ftz_daz_enabled();
        assert!(supports_ftz_daz());
        assert_eq!(enable_ftz_daz(), FtzDazOutcome::Applied);
        assert_eq!(ftz_daz_enabled(), Some(true), "置位后必须能读回 true");

        // 可逆性证明: 清位必须报告"此前是开启的", 随后读回 false。
        // 这一步排除了"读到别处遗留状态"的假阳性。
        assert!(disable_ftz_daz(), "清位应报告此前处于开启状态");
        assert_eq!(ftz_daz_enabled(), Some(false));

        // 复原
        if before == Some(true) {
            assert_eq!(enable_ftz_daz(), FtzDazOutcome::Applied);
        } else {
            assert!(!disable_ftz_daz(), "复原后应仍是未开启");
        }
    }

    /// 不支持的架构：空实现必须报告 `Unsupported` 且读回 `None`。
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
    #[test]
    fn unsupported_arch_is_a_safe_no_op() {
        assert!(!supports_ftz_daz());
        assert_eq!(enable_ftz_daz(), FtzDazOutcome::Unsupported);
        assert_eq!(ftz_daz_enabled(), None);
        assert!(!disable_ftz_daz());
    }
}
