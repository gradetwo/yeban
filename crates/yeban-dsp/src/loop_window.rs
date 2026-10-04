//! 循环点微平滑窗：64 点升余弦。[ARCH-DSP-001]
//!
//! **本模块是新写的**（来源库没有对应实现；规范 §3.3 给了公式，但没有给代码）。
//!
//! 规范 §3.3 原文："循环点微平滑窗：在小节末端应用 64 采样点升余弦微窗，
//! 消除采样波形不连续产生的杂音"：
//!
//! ```text
//! w(n) = ½ · [1 − cos(π·n / (N − 1))],   n ∈ [0, N−1],  N = 64
//! ```
//!
//! ## 这条曲线是什么形状（以及为什么"两端都为 0"是另一条曲线）
//!
//! 上式是**半余弦斜坡**：`w(0) = 0`、`w(N−1) = 1`、`w(N/2) = 0.5`，单调递增。
//! 它描述的是"从循环点起把信号从小淡入到满"。
//!
//! 它与对称 Hann 窗 `½[1 − cos(2πn/(N−1))]`（两端为 0、中间为 1）**不是同一条曲线**。
//! 循环接缝要真正无跳变，需要的是**一对互补的窗**：尾部用 `w` 的镜像淡出
//! （最后一个样本乘以 `w(0) = 0`），头部用 `w` 淡入（第一个样本乘以 `w(0) = 0`）。
//! 由于 `w(n) + w(N−1−n) ≡ 1`，这对窗的能量互补且接缝两侧都是 0。
//! [`LoopWindow::fade_in`] 与 [`LoopWindow::fade_out`] 实现的正是这一对。
//!
//! 该差异已登记在 `docs/ledger/dsp-core-provenance.md` §5（任务书示例判据写成
//! "两端为 0、中点为 1"，那是 2π 的 Hann 窗，与任务书自己给的公式及规范 §3.3 冲突）。
//!
//! 本模块无堆分配（表在构造期算好）、无全局状态 [ARCH-RT-001]。

/// 循环微平滑窗的长度（采样点），规范钉死为 64 [ARCH-DSP-001]。
pub const LOOP_WINDOW_LEN: usize = 64;

/// 按规范公式直接求 `w(index)`（不做表查）。
///
/// `index >= N` 会被钳到 `N − 1`，因此这是一个**全定义**函数：实时路径上
/// 越界参数不会 panic。
#[inline]
#[must_use]
pub fn raised_cosine(index: usize) -> f32 {
    let last = LOOP_WINDOW_LEN - 1;
    let position = index.min(last) as f32 / last as f32;
    let angle = core::f32::consts::PI * position;
    0.5 * (1.0 - angle.cos())
}

/// 64 点升余弦循环微平滑窗（值在构造期算好，之后只读）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopWindow {
    values: [f32; LOOP_WINDOW_LEN],
}

impl LoopWindow {
    /// 按规范公式构造窗表。
    #[must_use]
    pub fn new() -> Self {
        let mut values = [0.0f32; LOOP_WINDOW_LEN];
        for (index, slot) in values.iter_mut().enumerate() {
            *slot = raised_cosine(index);
        }
        Self { values }
    }

    /// 窗长（恒为 [`LOOP_WINDOW_LEN`]）。
    #[must_use]
    pub const fn len(&self) -> usize {
        LOOP_WINDOW_LEN
    }

    /// 该窗永远非空（宏 `clippy::len_without_is_empty` 要求成对出现）。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        false
    }

    /// 只读窗表。
    #[must_use]
    pub const fn values(&self) -> &[f32; LOOP_WINDOW_LEN] {
        &self.values
    }

    /// 取第 `index` 个系数（越界钳制，绝不 panic）。
    #[must_use]
    pub fn at(&self, index: usize) -> f32 {
        self.values[index.min(LOOP_WINDOW_LEN - 1)]
    }

    /// 把块的**开头** `min(N, len)` 个样本乘上升窗 `w(0..k)`。
    ///
    /// 块长小于 [`LOOP_WINDOW_LEN`] 时淡入无法完成（最后一个样本仍非满值）；
    /// 调用方应保证块长 ≥ 64，否则这里只做它能做的那部分。
    pub fn fade_in(&self, block: &mut [f32]) {
        let count = block.len().min(LOOP_WINDOW_LEN);
        for (sample, gain) in block.iter_mut().zip(self.values.iter()).take(count) {
            *sample *= gain;
        }
    }

    /// 把块的**结尾** `min(N, len)` 个样本乘上降窗：`w(k−1), …, w(1), w(0)`。
    ///
    /// 最后一个样本乘以 `w(0) = 0`，因此循环回绕到块首时两侧都是 0。
    /// 块长小于 [`LOOP_WINDOW_LEN`] 时用该前缀的逆序，淡出仍然**以 0 结束**。
    pub fn fade_out(&self, block: &mut [f32]) {
        let count = block.len().min(LOOP_WINDOW_LEN);
        if count == 0 {
            return;
        }
        let start = block.len() - count;
        for (sample, gain) in block[start..]
            .iter_mut()
            .zip(self.values[..count].iter().rev())
        {
            *sample *= gain;
        }
    }
}

impl Default for LoopWindow {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **判据（新写，可红）**：规范公式的端点与中点。
    ///
    /// `w(0) = 0`、`w(N−1) = 1`，严格单调递增；`N` 为偶数而 `N−1` 为奇数，
    /// 因此**中点略高于** ½（`w(32) = 0.5125`，而不是 0.5）——这是公式的直接
    /// 推论，不是误差。
    ///
    /// 把 `raised_cosine` 的分母从 `N − 1` 改成 `N`（差一个采样点的经典错位），
    /// `w(N−1)` 会掉到 ≈0.9994、中点也不再是 0.5125，本测试立即变红。
    #[test]
    fn the_window_matches_the_specified_curve() {
        let window = LoopWindow::new();
        let values = window.values();
        assert_eq!(values[0], 0.0, "w(0) must be exactly zero");
        assert!(
            (values[LOOP_WINDOW_LEN - 1] - 1.0).abs() < 1e-6,
            "w(N-1) must be one, got {}",
            values[LOOP_WINDOW_LEN - 1]
        );
        // 中点：¼ 周期不到 π/2，因此略高于 ½。
        let midpoint = values[LOOP_WINDOW_LEN / 2];
        let exact = 0.5
            * (1.0
                - (core::f32::consts::PI * (LOOP_WINDOW_LEN / 2) as f32
                    / (LOOP_WINDOW_LEN - 1) as f32)
                    .cos());
        assert!(
            (midpoint - exact).abs() < 1e-6,
            "w(N/2) must be {exact}, got {midpoint}"
        );
        assert!(
            midpoint > 0.5 && midpoint < 0.52,
            "even N puts the midpoint just above one half, got {midpoint}"
        );
        for pair in values.windows(2) {
            assert!(pair[1] > pair[0], "window is not strictly increasing");
        }
        // 表必须与公式逐点一致（防止构造期的错位）。
        for (index, value) in values.iter().enumerate() {
            assert_eq!(*value, raised_cosine(index), "table drift at {index}");
        }
    }

    /// 互补性：`w(n) + w(N−1−n) ≡ 1`。这是"一对窗能量守恒"的数学保证。
    #[test]
    fn the_window_is_complementary_to_its_mirror() {
        let window = LoopWindow::new();
        for index in 0..LOOP_WINDOW_LEN {
            let sum = window.at(index) + window.at(LOOP_WINDOW_LEN - 1 - index);
            assert!((sum - 1.0).abs() < 1e-6, "w({index}) + mirror = {sum}");
        }
    }

    /// **判据（新写，可红）**：首尾相接无跳变。
    ///
    /// 用一个在接缝处天然断裂的斜坡（末样本 ≈1、首样本 =0，跳变 ≈1），
    /// 施加淡出 + 淡入之后接缝两侧都必须落到 0，跳变必须消失。
    ///
    /// 若把 [`LoopWindow::fade_out`] 的取窗区间写成从窗头开始
    /// （`values[..count]`）而不是以 0 结尾（`values[N−count..]`），
    /// 末样本会停在 ≈1 上，本测试立即变红。
    #[test]
    fn a_loop_seam_wraps_without_a_step() {
        let window = LoopWindow::new();
        let length = LOOP_WINDOW_LEN * 2;
        let ramp: Vec<f32> = (0..length).map(|i| i as f32 / length as f32).collect();

        // 对照：不做任何平滑时接缝处就是一个 1.0 量级的断崖。
        let unwindowed_step = (ramp[length - 1] - ramp[0]).abs();
        assert!(
            unwindowed_step > 0.9,
            "the control signal must actually be discontinuous at the seam"
        );

        let mut looped = ramp;
        window.fade_in(&mut looped);
        window.fade_out(&mut looped);
        assert_eq!(looped[0], 0.0, "head must start at silence");
        assert_eq!(looped[length - 1], 0.0, "tail must end at silence");
        // 回绕：末样本 → 首样本。
        let wrapped_step = (looped[0] - looped[length - 1]).abs();
        assert!(
            wrapped_step < 1e-6,
            "loop seam still steps by {wrapped_step}"
        );
        assert!(
            wrapped_step < unwindowed_step * 1e-6,
            "the window did not remove the seam discontinuity"
        );
        // 窗内不得出现非有限值，也不得出现超过原信号的增益。
        for (index, value) in looped.iter().enumerate() {
            assert!(value.is_finite(), "non-finite at {index}");
            assert!(
                *value <= ramp_max_at(index),
                "window amplified sample {index}"
            );
        }
    }

    /// 斜坡在 index 处的原始上界（用来断言"窗只衰减、不放大"）。
    fn ramp_max_at(index: usize) -> f32 {
        (index as f32 / (LOOP_WINDOW_LEN * 2) as f32) + 1e-9
    }

    #[test]
    fn short_blocks_are_handled_without_panicking() {
        let window = LoopWindow::new();
        // 空块。
        let mut empty: [f32; 0] = [];
        window.fade_in(&mut empty);
        window.fade_out(&mut empty);
        // 短于窗长的块：只处理它有的部分，且尾部仍以 0 结束。
        let mut short = [1.0f32; 4];
        window.fade_in(&mut short);
        assert_eq!(short[0], 0.0);
        let mut short = [1.0f32; 4];
        window.fade_out(&mut short);
        assert_eq!(short[3], 0.0, "fade_out must always end at zero");
        assert!(short[0] > 0.0 && short[0] < 1.0);
    }

    #[test]
    fn out_of_range_lookup_is_clamped() {
        let window = LoopWindow::new();
        assert_eq!(window.at(0), 0.0);
        assert_eq!(window.at(usize::MAX), window.at(LOOP_WINDOW_LEN - 1));
        assert_eq!(
            raised_cosine(usize::MAX),
            raised_cosine(LOOP_WINDOW_LEN - 1)
        );
        assert_eq!(window.len(), LOOP_WINDOW_LEN);
        assert!(!window.is_empty());
        assert_eq!(LoopWindow::default(), window);
    }
}
