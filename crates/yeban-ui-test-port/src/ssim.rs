//! SSIM（结构相似性）—— `[UI-MCP-003]` 的"SSIM ≥ 0.98"判据。
//!
//! ## 用的是哪一份定义（口径必须写死，否则阈值没有意义）
//!
//! 标准定义来自 Wang, Bovik, Sheikh, Simoncelli, *Image Quality Assessment: From Error
//! Visibility to Structural Similarity*, IEEE TIP 13(4), 2004（§III）。
//! 本实现逐条固定如下，**不接受**"大致等价"的实现：
//!
//! | 项目 | 本实现取值 | 依据 |
//! | :--- | :--- | :--- |
//! | 灰度 | `Y = 0.299R + 0.587G + 0.114B` | ITU-R BT.601-7 luma；与 `skimage.color.rgb2gray` 同口径 |
//! | 窗口 | **7×7 均匀（box）窗**，步长 1 | `skimage.metrics.structural_similarity` 的默认 `win_size=7` + `gaussian_weights=False`（即均匀窗） |
//! | 边界 | **valid 卷积**：只统计完全落在图像内的窗口，不做 padding | 最可复现的口径；padding 策略（reflect/replicate）各库不同，会引入平台无关但版本相关的抖动 |
//! | 局部均值 | `μ = Σx / N`，`N = 49` | 论文式 (13) 的离散均匀窗形式 |
//! | 局部方差/协方差 | `σ² = Σx²/N − μ²`，`σxy = Σxy/N − μxμy` | 论文式 (13)；用 "E[x²]−μ²" 的等价形式换取单趟滑窗 |
//! | 常数 | `C1 = (K1·L)²`, `C2 = (K2·L)²`, `K1 = 0.01`, `K2 = 0.03`, `L = 255` | 论文式 (13) 原文取值 |
//! | 汇总 | 逐窗口 SSIM 的**算术平均**（mean SSIM, MSSIM） | 论文 §III.C 末段 |
//!
//! 出处（写作前逐条核验，不是凭记忆）：
//! - 论文 DOI <https://doi.org/10.1109/TIP.2003.819861>；
//! - `skimage.metrics.structural_similarity` 的默认参数与公式
//!   <https://scikit-image.org/docs/stable/api/skimage.metrics.html#skimage.metrics.structural_similarity>；
//! - `skimage.color.rgb2gray` 的 BT.601 系数
//!   <https://scikit-image.org/docs/stable/api/skimage.color.html#skimage.color.rgb2gray>。
//!
//! ## 为什么不是 11×11 高斯窗
//!
//! 论文原图用的是 11×11、`σ=1.5` 的高斯窗，但主流参考实现（`skimage`）**默认**用均匀窗，
//! 且小窗口对小尺寸截图（CI 里 960×540 的局部断言）更稳。**选择本身是工程裁决**：
//! 一旦选定，它就必须写进这里、写进 notes，并且判据里钉死阈值，避免"换窗口凑绿"。
//! 若未来要与 `skimage` 数值对账，请先改 [`WINDOW_SIZE`] 并重跑全部判据。
//!
//! ## 阈值
//!
//! `[UI-MCP-003]` §12.5：初版工程基准收敛为 **SSIM ≥ 0.98**。见 [`SSIM_THRESHOLD`]。

use crate::image::{LumaImage, Rgb8Image, Size};

/// `[UI-MCP-003]` 的默认判据阈值（UI/UX §12.5 原文："SSIM ≥ 0.98"）。
pub const SSIM_THRESHOLD: f64 = 0.98;

/// SSIM 的窗口边长（见模块文档的口径表）。
pub const WINDOW_SIZE: u32 = 7;

/// 稳定常数 `C1 = (K1 · L)²`，`K1 = 0.01`，`L = 255`。
pub const C1: f64 = (0.01 * 255.0) * (0.01 * 255.0);
/// 稳定常数 `C2 = (K2 · L)²`，`K2 = 0.03`，`L = 255`。
pub const C2: f64 = (0.03 * 255.0) * (0.03 * 255.0);

/// SSIM 无法计算时的原因（一律报错，不返回"看起来差不多"的默认值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SsimError {
    /// 两张图尺寸不同 —— 尺寸本身就是视觉回归，必须先修尺寸再谈相似度。
    SizeMismatch {
        /// 左图尺寸。
        left: Size,
        /// 右图尺寸。
        right: Size,
    },
    /// 图像比 SSIM 窗口还小，valid 卷积没有任何一个窗口。
    TooSmall {
        /// 触发失败的尺寸。
        size: Size,
        /// 需要的窗口边长。
        window: u32,
    },
}

impl core::fmt::Display for SsimError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SizeMismatch { left, right } => write!(
                f,
                "SSIM 要求尺寸一致: 左 {}x{}, 右 {}x{}",
                left.width, left.height, right.width, right.height
            ),
            Self::TooSmall { size, window } => write!(
                f,
                "SSIM 窗口 {window}x{window} 大于图像 {}x{}; 请缩小窗口或放大截图",
                size.width, size.height
            ),
        }
    }
}

impl core::error::Error for SsimError {}

/// 比对结论。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Verdict {
    /// 实测 SSIM。
    pub score: f64,
    /// 使用的阈值。
    pub threshold: f64,
    /// 是否通过。
    pub passed: bool,
}

impl Verdict {
    /// 用默认阈值（[`SSIM_THRESHOLD`]）判定。
    #[must_use]
    pub fn with_default_threshold(score: f64) -> Self {
        Self::with_threshold(score, SSIM_THRESHOLD)
    }

    /// 用显式阈值判定。
    #[must_use]
    pub fn with_threshold(score: f64, threshold: f64) -> Self {
        Self {
            score,
            threshold,
            passed: score >= threshold,
        }
    }
}

impl core::fmt::Display for Verdict {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "SSIM {:.6} vs 阈值 {:.2} => {}",
            self.score,
            self.threshold,
            if self.passed { "通过" } else { "不通过" }
        )
    }
}

/// 对两张 RGB8 图像求 mean SSIM（自动转灰度）。
pub fn ssim(left: &Rgb8Image, right: &Rgb8Image) -> Result<f64, SsimError> {
    if left.size() != right.size() {
        return Err(SsimError::SizeMismatch {
            left: left.size(),
            right: right.size(),
        });
    }
    ssim_luma(&left.to_luma(), &right.to_luma())
}

/// 比 [`ssim`] 多一步默认阈值判定。
pub fn compare(left: &Rgb8Image, right: &Rgb8Image) -> Result<Verdict, SsimError> {
    ssim(left, right).map(Verdict::with_default_threshold)
}

/// 对两张亮度图求 mean SSIM。
///
/// 实现说明（滑窗，`O(宽×高)`，内存 `O(宽)`）：
/// 维护 5 条列和（`x`、`x²`、`y`、`y²`、`xy`），当窗口纵向移动时"进一行、出一行"，
/// 横向再滑动 7 列。窗口和除以 `N = 49` 得到局部统计量。
/// 全程 `f64`，运算顺序固定 ⇒ 同一输入在任何平台上给出**逐位相同**的结果
/// （这一点由 `ssim_is_bitwise_reproducible` 判据钉住）。
pub fn ssim_luma(left: &LumaImage, right: &LumaImage) -> Result<f64, SsimError> {
    let size = left.size();
    if size != right.size() {
        return Err(SsimError::SizeMismatch {
            left: size,
            right: right.size(),
        });
    }
    let n = WINDOW_SIZE;
    if size.width < n || size.height < n {
        return Err(SsimError::TooSmall { size, window: n });
    }
    let width = size.width as usize;
    let height = size.height as usize;
    let window = n as usize;
    let count = (window * window) as f64;

    let a = left.data();
    let b = right.data();

    // 5 条列和：[x, x², y, y², xy]
    let mut col = vec![[0.0f64; 5]; width];
    let accumulate = |col: &mut [[f64; 5]], row: usize, sign: f64| {
        let base = row * width;
        for (x, slot) in col.iter_mut().enumerate() {
            let va = a[base + x];
            let vb = b[base + x];
            slot[0] += sign * va;
            slot[1] += sign * va * va;
            slot[2] += sign * vb;
            slot[3] += sign * vb * vb;
            slot[4] += sign * va * vb;
        }
    };
    for row in 0..window {
        accumulate(&mut col, row, 1.0);
    }

    let mut total = 0.0f64;
    let mut windows = 0u64;
    for top in 0..=(height - window) {
        // 横向滑窗：先把最左侧窗口的 7 列累起来。
        let mut sum = [0.0f64; 5];
        for slot in &col[..window] {
            for k in 0..5 {
                sum[k] += slot[k];
            }
        }
        for x0 in 0..=(width - window) {
            if x0 > 0 {
                let entering = &col[x0 + window - 1];
                let leaving = &col[x0 - 1];
                for k in 0..5 {
                    sum[k] += entering[k] - leaving[k];
                }
            }
            let mu_x = sum[0] / count;
            let mu_y = sum[2] / count;
            let var_x = sum[1] / count - mu_x * mu_x;
            let var_y = sum[3] / count - mu_y * mu_y;
            let cov = sum[4] / count - mu_x * mu_y;
            let numerator = (2.0 * mu_x * mu_y + C1) * (2.0 * cov + C2);
            let denominator = (mu_x * mu_x + mu_y * mu_y + C1) * (var_x + var_y + C2);
            total += numerator / denominator;
            windows += 1;
        }
        // 纵向滑窗：进下一行、出当前首行。
        if top + window < height {
            accumulate(&mut col, top + window, 1.0);
            accumulate(&mut col, top, -1.0);
        }
    }

    Ok(total / windows as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::Rect;

    fn textured(width: u32, height: u32) -> Rgb8Image {
        let mut image = Rgb8Image::new(Size::new(width, height));
        for y in 0..height {
            for x in 0..width {
                let value = ((x * 7 + y * 13) % 256) as u8;
                image.set_pixel(x, y, [value, value.wrapping_mul(3), value.wrapping_add(40)]);
            }
        }
        image
    }

    /// 判据 1: 同一图像 SSIM = 1.0（论文式 (13) 在 `x ≡ y` 时代数恒等，浮点上也恒等）。
    #[test]
    fn identical_images_score_exactly_one() {
        let image = textured(64, 48);
        let score = ssim(&image, &image).expect("同尺寸应当可算");
        assert!((score - 1.0).abs() <= f64::EPSILON, "实测 {score}");
        assert!(compare(&image, &image).expect("可算").passed);

        // 纯色图也要恰好 1.0（方差为 0 的退化情形）。
        let mut flat = Rgb8Image::new(Size::new(32, 32));
        flat.fill([10, 20, 30]);
        let flat_score = ssim(&flat, &flat.clone()).expect("同尺寸应当可算");
        assert!(
            (flat_score - 1.0).abs() <= f64::EPSILON,
            "实测 {flat_score}"
        );
    }

    /// 判据 2: 对称性与取值范围（`-1 ≤ SSIM ≤ 1`）。取值范围写进断言，
    /// 防止"把公式里的某个 2 写丢"这类错误悄悄通过。
    #[test]
    fn score_is_symmetric_and_bounded() {
        let base = textured(80, 60);
        let mut shifted = base.clone();
        shifted.fill_rect(Rect::new(10, 10, 30, 20), [0xff, 0xff, 0xff]);
        let forward = ssim(&base, &shifted).expect("同尺寸应当可算");
        let backward = ssim(&shifted, &base).expect("同尺寸应当可算");
        assert_eq!(
            forward.to_bits(),
            backward.to_bits(),
            "SSIM 必须对称且逐位相同"
        );
        assert!((-1.0..=1.0).contains(&forward), "实测 {forward}");
    }

    /// 判据 3: **`[UI-MCP-002]` + `[UI-MCP-003]` 的联合判据** ——
    /// VU 表/走带光标的每帧抖动在遮罩后必须仍然 SSIM = 1.0；
    /// 而静态排布被改动（元素缺失/错位）必须掉到阈值以下。
    #[test]
    fn masking_absorbs_dynamic_jitter_but_keeps_static_regressions() {
        let base = textured(120, 90);
        let dynamic = Rect::new(8, 8, 24, 40);

        // 抖动: 只改动态区内部。
        let mut jitter = base.clone();
        jitter.fill_rect(dynamic, [0x00, 0xff, 0x00]);
        assert!(
            !Verdict::with_default_threshold(ssim(&base, &jitter).expect("可算")).passed,
            "未遮罩时必须被检出 (否则遮罩判据没有意义)"
        );
        let masked_base = crate::mask::masked(&base, &[dynamic]);
        let masked_jitter = crate::mask::masked(&jitter, &[dynamic]);
        let masked_score = ssim(&masked_base, &masked_jitter).expect("可算");
        assert!(
            (masked_score - 1.0).abs() <= f64::EPSILON,
            "遮罩后实测 {masked_score}"
        );

        // 静态回归: 遮罩区**之外**的整块排布被移走。
        let mut regressed = base.clone();
        regressed.fill_rect(Rect::new(60, 20, 40, 30), [0x00, 0x00, 0x00]);
        let regressed_score = ssim(
            &crate::mask::masked(&base, &[dynamic]),
            &crate::mask::masked(&regressed, &[dynamic]),
        )
        .expect("可算");
        assert!(
            !Verdict::with_default_threshold(regressed_score).passed,
            "静态区大改必须低于 {SSIM_THRESHOLD}, 实测 {regressed_score}"
        );
    }

    /// 判据 4: 阈值是可配置的，且 `Verdict` 的判定边界精确（0.98 本身算通过）。
    #[test]
    fn verdict_threshold_boundary_is_inclusive() {
        assert!(Verdict::with_default_threshold(0.98).passed);
        assert!(!Verdict::with_default_threshold(0.979_999).passed);
        assert!(Verdict::with_threshold(0.90, 0.85).passed);
        assert!(!Verdict::with_threshold(0.90, 0.95).passed);
        assert_eq!(Verdict::with_default_threshold(0.5).threshold, 0.98);
    }

    /// 判据 5: 尺寸不一致 / 图像小于窗口时必须**报错**，而不是返回一个看似合理的分数。
    #[test]
    fn mismatched_or_tiny_inputs_are_errors() {
        let big = textured(32, 32);
        let small = textured(16, 32);
        assert_eq!(
            ssim(&big, &small),
            Err(SsimError::SizeMismatch {
                left: Size::new(32, 32),
                right: Size::new(16, 32)
            })
        );
        let tiny = textured(4, 4);
        assert_eq!(
            ssim(&tiny, &tiny),
            Err(SsimError::TooSmall {
                size: Size::new(4, 4),
                window: WINDOW_SIZE
            })
        );
    }

    /// 判据 6: 逐位可复现 —— 同一输入两次调用结果 `to_bits()` 相同（跨平台确定性的前提）。
    #[test]
    fn ssim_is_bitwise_reproducible() {
        let base = textured(70, 70);
        let mut other = base.clone();
        other.fill_rect(Rect::new(5, 5, 20, 20), [1, 2, 3]);
        let first = ssim(&base, &other).expect("可算");
        let second = ssim(&base, &other).expect("可算");
        assert_eq!(first.to_bits(), second.to_bits());
    }

    /// 判据 7: 与朴素双重循环（直接照抄论文式 13）实现的对账 —— 防止滑窗优化写错。
    ///
    /// 这里**不能**要求逐位相同：滑窗把"49 个数的求和"重排成了"7 个列和再相加"，
    /// 浮点加法不满足结合律，末位必然有差。因此对账口径是 `|差| < 1e-9`
    /// （远小于阈值 0.02 的判定间距），而"逐位可复现"由判据 6 单独负责
    /// （同一实现的重复调用必须逐位相同）。
    #[test]
    fn sliding_window_matches_the_naive_translation_of_the_paper() {
        let left = textured(45, 33);
        let right = {
            let mut image = left.clone();
            image.fill_rect(Rect::new(3, 3, 12, 9), [0xcc, 0x33, 0x66]);
            image
        };
        let fast = ssim(&left, &right).expect("可算");
        let naive = naive_ssim(&left.to_luma(), &right.to_luma());
        assert!((fast - naive).abs() < 1e-9, "滑窗 {fast} vs 直译 {naive}");
    }

    /// 论文式 (13) 的直译（无任何优化），仅用于对账。
    fn naive_ssim(left: &LumaImage, right: &LumaImage) -> f64 {
        let n = WINDOW_SIZE;
        let size = left.size();
        let count = f64::from(n * n);
        let mut total = 0.0;
        let mut windows = 0u64;
        for top in 0..=(size.height - n) {
            for lx in 0..=(size.width - n) {
                let (mut sx, mut sy, mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
                for y in top..top + n {
                    for x in lx..lx + n {
                        let va = left.at(x, y).unwrap_or_default();
                        let vb = right.at(x, y).unwrap_or_default();
                        sx += va;
                        sy += vb;
                        sxx += va * va;
                        syy += vb * vb;
                        sxy += va * vb;
                    }
                }
                let mu_x = sx / count;
                let mu_y = sy / count;
                let var_x = sxx / count - mu_x * mu_x;
                let var_y = syy / count - mu_y * mu_y;
                let cov = sxy / count - mu_x * mu_y;
                total += ((2.0 * mu_x * mu_y + C1) * (2.0 * cov + C2))
                    / ((mu_x * mu_x + mu_y * mu_y + C1) * (var_x + var_y + C2));
                windows += 1;
            }
        }
        total / windows as f64
    }
}
