//! 时间基准的**纯换算**：音频帧 ↔ tick [MODEL-AST-001]。
//!
//! ## 为什么住在模型层
//!
//! 「一段 N 帧的素材占多少个 tick」是**模型时间基准**的算术，不是某个入口的私事：
//! 它只用到 [`crate::PPQ`]（960）＋ 工程速度 ＋ 采样率，没有 I/O、没有第三方 crate、
//! 没有会话状态 ⇒ 放在这里可以被 `yeban-app`（录音落点）与 `yeban-mcp`（音频导入）
//! 的**任何**一侧调用，而不必让 app 去够一个默认关闭的可选 crate。
//!
//! ⚠ **重复实现登记（如实）**：`yeban-mcp` 的
//! `src/domain/render_math.rs::frames_to_ticks_ceil` 在本模块出现**之前**就是
//! 「帧 → tick」的唯一实现。本模块落地时**没有**删掉那一份，也没有让它转调这里：
//! 那份文件被 `crates/yeban-mcp/verify/render_pure.rs` 用**裸 `rustc`**（`#[path]`
//! 引入真实源文件、不带任何 `--extern`）编译执行，一旦它 `use yeban_model::…`，
//! 那条本机零依赖验证路径就断了。⇒ 今天仓里是**两份逐字同式**的实现，
//! 并轨（让 MCP 转调模型层，同时改写验证脚手架的依赖注入）留作**独立裁决**。
//! 两边的判据（本文件的 `#[cfg(test)]` 与 `render_math.rs` 的 `mod tests`）
//! 钉住同一组取值，任何一侧漂移都会在各自的 `cargo test` 里红。
//!
//! ## 公式
//!
//! ```text
//! 秒数      = frames / sample_rate
//! 四分音符数 = 秒数 × bpm / 60
//! tick 数   = 四分音符数 × PPQ
//!           = ceil(frames × PPQ × bpm / (sample_rate × 60))
//! ```
//!
//! **向上取整**（不是四舍五入、也不是向下）：换算的对象是**摆放时值**，向上取整
//! 保证素材尾部不足一 tick 的部分不会被切掉；向下取整会让最后几帧永远听不到，
//! 那是一种静默丢音（同一取舍的论证见 `yeban-mcp/.../render_math.rs` 的文档）。
//!
//! 结果下界是 **1**：`ClipPlacement::validate` 拒绝零时值，而 0 帧素材也不该产出
//! 一个会被模型层拒掉的 0（本函数对 0 帧返回 `Some(1)`，与既有实现对账）。

/// 帧 → tick（**向上取整**，下界 1）。
///
/// `sample_rate` 是**这些帧自己的**采样率（Hz）：`frames` 是它在那个速率下数出来的
/// 样本数，把它换成秒必须除它自己的速率。产品路径上录音 WAV 与工程同为 48 kHz
/// （`RecordOptions::default().sample_rate = 48_000`、工程缺省 48 kHz），但函数本身
/// 不假设两者相等 —— 用错速率会把时值算成另一个秒数。
///
/// 病态输入（`ppq == 0`、`bpm <= 0`／非有限、`sample_rate == 0`）返回 `None`：
/// 这里**没有**安全的哨兵值（0 会被 [`crate::ClipPlacement::validate`] 拒绝，
/// 1 会凭空造出一 tick 的素材）⇒ 调用方必须显式处置，⛔ 不许悄悄写一个 0。
///
/// # 例子
///
/// ```
/// use yeban_model::{PPQ, time_math::frames_to_ticks_ceil};
///
/// // 0.5 秒 @ 48 kHz @ 120 BPM：0.5 × 2 = 1 个四分音符 = 960 tick。
/// assert_eq!(frames_to_ticks_ceil(24_000, PPQ, 120.0, 48_000), Some(960));
/// // 0 帧仍给出下界 1（0 会被 ClipPlacement::validate 拒绝）。
/// assert_eq!(frames_to_ticks_ceil(0, PPQ, 120.0, 48_000), Some(1));
/// // 病态输入没有哨兵值。
/// assert_eq!(frames_to_ticks_ceil(1, PPQ, 0.0, 48_000), None);
/// ```
#[must_use]
pub fn frames_to_ticks_ceil(frames: u64, ppq: u64, bpm: f64, sample_rate: u32) -> Option<u64> {
    if ppq == 0 || !bpm.is_finite() || bpm <= 0.0 || sample_rate == 0 {
        return None;
    }
    let ticks = (frames as f64) * (ppq as f64) * bpm / (f64::from(sample_rate) * 60.0);
    if !ticks.is_finite() || ticks < 0.0 {
        return None;
    }
    if ticks >= u64::MAX as f64 {
        return Some(u64::MAX);
    }
    Some((ticks.ceil() as u64).max(1))
}

#[cfg(test)]
mod tests {
    use super::frames_to_ticks_ceil;
    use crate::PPQ;

    /// 手算对照：`0.5 s @ 48 kHz @ 960 PPQ @ 120 BPM`。
    ///
    /// `seconds = 24000 / 48000 = 0.5`；`quarters = 0.5 × 120 / 60 = 1`；
    /// `ticks = 1 × 960 = 960` ⇒ 逐字相等（不是"约等于"）。
    #[test]
    fn half_a_second_at_48k_120bpm_is_exactly_960_ticks() {
        assert_eq!(frames_to_ticks_ceil(24_000, PPQ, 120.0, 48_000), Some(960));
    }

    /// 边界：0 帧 ⇒ 下界 1；1 帧 ⇒ 向上取整仍是 1。
    #[test]
    fn zero_frames_clamps_to_one_tick_and_one_frame_also_rounds_up_to_one() {
        assert_eq!(frames_to_ticks_ceil(0, PPQ, 120.0, 48_000), Some(1));
        assert_eq!(frames_to_ticks_ceil(1, PPQ, 120.0, 48_000), Some(1));
    }

    /// 边界：**非整 tick 向上取整**。
    ///
    /// 23 040 帧 @ 48 kHz = 0.48 s ⇒ 0.96 个四分音符 ⇒ 921.6 tick ⇒ **922**。
    /// 若取整方向写反（floor/round）会得到 921，尾部的 0.6 tick 被静默丢掉。
    #[test]
    fn a_fractional_tick_rounds_up_not_down() {
        assert_eq!(frames_to_ticks_ceil(23_040, PPQ, 120.0, 48_000), Some(922));
        // 4 800 帧 @ 48 kHz @ 128 BPM = 0.1 s ⇒ 0.21333… 四分音符 ⇒ 204.8 tick ⇒ 205。
        assert_eq!(frames_to_ticks_ceil(4_800, PPQ, 128.0, 48_000), Some(205));
    }

    /// 采样率是**素材自己的**：同一秒数换一个采样率，tick 必须相同。
    ///
    /// 0.5 s = 24 000 帧 @ 48 kHz = 22 050 帧 @ 44.1 kHz ⇒ 都是 960 tick。
    #[test]
    fn the_same_duration_at_a_different_rate_gives_the_same_ticks() {
        assert_eq!(frames_to_ticks_ceil(22_050, PPQ, 120.0, 44_100), Some(960));
    }

    /// 病态输入一律 `None`（没有哨兵值）。
    #[test]
    fn degenerate_inputs_have_no_answer() {
        assert_eq!(frames_to_ticks_ceil(24_000, 0, 120.0, 48_000), None);
        assert_eq!(frames_to_ticks_ceil(24_000, PPQ, 0.0, 48_000), None);
        assert_eq!(frames_to_ticks_ceil(24_000, PPQ, -1.0, 48_000), None);
        assert_eq!(frames_to_ticks_ceil(24_000, PPQ, f64::NAN, 48_000), None);
        assert_eq!(frames_to_ticks_ceil(24_000, PPQ, 120.0, 0), None);
    }

    /// 上界：换算结果溢出 `u64` 时饱和到 `u64::MAX`，⛔ 不回绕成 0。
    ///
    /// 取 `ppq = u64::MAX`（函数只拒绝 `ppq == 0`）⇒ 结果远超 `u64::MAX`。
    #[test]
    fn huge_inputs_saturate_instead_of_wrapping() {
        assert_eq!(
            frames_to_ticks_ceil(u64::MAX, u64::MAX, 1.0, 1),
            Some(u64::MAX)
        );
    }
}
