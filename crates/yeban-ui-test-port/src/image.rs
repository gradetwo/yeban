//! 纯内存 RGB8 图像、几何基元与"非全黑"断言 —— 本 crate 的**零 Slint 依赖**底座。
//!
//! 为什么把 `Rgb8Image` / [`Rect`] 单独放一层（而不是塞进 `render.rs`）：
//! `[MUST-GATE-015]` 要求 Golden 图必须由 Tier-1 软件光栅化产出、**尺寸非零且非全黑**；
//! 而遮罩（`mask.rs`）、SSIM（`ssim.rs`）、PNG 编码（`png.rs`）都只跟像素打交道，
//! 不该被拖进 Slint 的重依赖里。把它们与 Slint 物理隔离之后，这些判据可以在**本机**用
//! `rustc --test` 直接编译执行（本机纪律禁止编译 Slint），CI 只需要再证明"Slint 真的能
//! 把像素写进这个缓冲"。
//!
//! 规范来源 (Normative)：
//! - `[UI-MCP-002]` UI/UX §12.5 —— 高频刷新区域在比对前置黑；
//! - `[UI-MCP-003]` UI/UX §12.5 —— 分平台 Golden；
//! - `[MUST-GATE-015]` 路线图 §5 —— 尺寸非零且非全黑。

/// 图像 / 区域的尺寸（物理像素）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Size {
    /// 宽（物理像素）。
    pub width: u32,
    /// 高（物理像素）。
    pub height: u32,
}

impl Size {
    /// 构造尺寸。
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// 是否至少有一边为 0。`[MUST-GATE-015]` 明文禁止零尺寸 Golden。
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// 像素总数（`u64` 以免 4K 以上溢出 `u32`）。
    #[must_use]
    pub const fn pixel_count(self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

/// 轴对齐整数矩形。坐标系约定：**原点在左上角，y 轴向下**（与 Slint 逻辑坐标一致）。
///
/// 允许负的 `x` / `y`（元素可能被父级裁剪到屏幕外），因此这里用 `i32`。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct Rect {
    /// 左上角 x。
    pub x: i32,
    /// 左上角 y。
    pub y: i32,
    /// 宽。
    pub width: u32,
    /// 高。
    pub height: u32,
}

impl Rect {
    /// 构造矩形。
    #[must_use]
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// 是否为空（任一边为 0）。
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// 右边界（不含）。
    #[must_use]
    pub const fn right(self) -> i32 {
        self.x.saturating_add(self.width as i32)
    }

    /// 下边界（不含）。
    #[must_use]
    pub const fn bottom(self) -> i32 {
        self.y.saturating_add(self.height as i32)
    }

    /// 与图像求交，并把结果夹到 `size` 之内。完全落在图像外时返回 `None`。
    ///
    /// 遮罩必须夹取：`[UI-MCP-002]` 的包围盒来自元素树，元素可能被视口裁到屏幕外，
    /// 直接按原矩形写内存会越界。
    #[must_use]
    pub fn intersect(self, size: Size) -> Option<Self> {
        if self.is_empty() || size.is_empty() {
            return None;
        }
        let left = self.x.max(0);
        let top = self.y.max(0);
        let right = self.right().min(size.width as i32);
        let bottom = self.bottom().min(size.height as i32);
        if right <= left || bottom <= top {
            return None;
        }
        Some(Self::new(
            left,
            top,
            (right - left) as u32,
            (bottom - top) as u32,
        ))
    }

    /// 面积（像素数）。
    #[must_use]
    pub const fn area(self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

/// 单像素：`[R, G, B]`（不透明，无 alpha 通道）。
pub type Rgb = [u8; 3];

/// 遮罩色。`[UI-MCP-002]` 原文：置为纯黑 `#000000`。
pub const MASK_COLOR: Rgb = [0, 0, 0];

/// 图像读写的错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageError {
    /// 尺寸为零。`[MUST-GATE-015]` 要求 Golden 尺寸非零。
    EmptySize {
        /// 出错的尺寸。
        size: Size,
    },
    /// 原始字节数与 `width * height * 3` 不符。
    BufferLengthMismatch {
        /// 期望字节数。
        expected: usize,
        /// 实际字节数。
        actual: usize,
    },
}

impl core::fmt::Display for ImageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptySize { size } => {
                write!(f, "图像尺寸不得为零: {}x{}", size.width, size.height)
            }
            Self::BufferLengthMismatch { expected, actual } => write!(
                f,
                "像素缓冲区长度不符: 期望 {expected} 字节 (宽*高*3), 实际 {actual} 字节"
            ),
        }
    }
}

impl core::error::Error for ImageError {}

/// 行主序、每像素 3 字节（RGB）的内存图像。
///
/// 这是 Tier-1 软件光栅化的落点：`slint::Rgb8Pixel` 与它**逐字节同布局**
/// （`i-slint-core` 的 `pub type Rgb8Pixel = rgb::RGB8` 即 `{ r: u8, g: u8, b: u8 }`，
/// 出处 <https://docs.rs/slint/1.18.1/slint/type.Rgb8Pixel.html>），
/// 但本类型刻意不依赖 Slint —— 转换由 `render.rs` 单向完成。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgb8Image {
    size: Size,
    pixels: Vec<u8>,
}

impl Rgb8Image {
    /// 新建全黑（`#000000`）图像。
    ///
    /// # Panics
    ///
    /// 当 `size` 任一边为 0 时 panic —— 零尺寸图像是编程错误（`[MUST-GATE-015]`），
    /// 而 `Result` 会迫使每个调用点处理一个不可能发生的情况。
    #[must_use]
    pub fn new(size: Size) -> Self {
        assert!(
            !size.is_empty(),
            "零尺寸图像: {}x{}",
            size.width,
            size.height
        );
        let bytes = usize::try_from(size.pixel_count() * 3).expect("图像体积超出 usize");
        Self {
            size,
            pixels: vec![0; bytes],
        }
    }

    /// 从原始 RGB 字节构造，校验长度与尺寸自洽。
    pub fn from_raw(size: Size, pixels: Vec<u8>) -> Result<Self, ImageError> {
        if size.is_empty() {
            return Err(ImageError::EmptySize { size });
        }
        let expected =
            usize::try_from(size.pixel_count() * 3).map_err(|_| ImageError::EmptySize { size })?;
        if pixels.len() != expected {
            return Err(ImageError::BufferLengthMismatch {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self { size, pixels })
    }

    /// 尺寸。
    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }

    /// 宽。
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.size.width
    }

    /// 高。
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.size.height
    }

    /// 每行字节数（`宽 * 3`）。
    #[must_use]
    pub fn stride(&self) -> usize {
        self.size.width as usize * 3
    }

    /// 只读像素字节（行主序）。
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// 可变像素字节（行主序）。长度恒为 `宽 * 高 * 3`。
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        &mut self.pixels
    }

    /// 取单像素；越界返回 `None`。
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<Rgb> {
        if x >= self.size.width || y >= self.size.height {
            return None;
        }
        let offset = y as usize * self.stride() + x as usize * 3;
        Some([
            self.pixels[offset],
            self.pixels[offset + 1],
            self.pixels[offset + 2],
        ])
    }

    /// 写单像素；越界静默忽略（写遮罩时越界是正常情况，见 [`Rect::intersect`]）。
    pub fn set_pixel(&mut self, x: u32, y: u32, rgb: Rgb) {
        if x >= self.size.width || y >= self.size.height {
            return;
        }
        let offset = y as usize * self.stride() + x as usize * 3;
        self.pixels[offset..offset + 3].copy_from_slice(&rgb);
    }

    /// 把整幅图填成一种颜色。
    pub fn fill(&mut self, rgb: Rgb) {
        for chunk in self.pixels.chunks_exact_mut(3) {
            chunk.copy_from_slice(&rgb);
        }
    }

    /// 把某个矩形填成一种颜色；矩形会被夹到图像边界内，越界部分忽略。
    ///
    /// 返回真正被写入的像素数（用于判据"遮罩确实改了像素"）。
    pub fn fill_rect(&mut self, rect: Rect, rgb: Rgb) -> usize {
        let Some(rect) = rect.intersect(self.size) else {
            return 0;
        };
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                let offset = y as usize * self.stride() + x as usize * 3;
                self.pixels[offset..offset + 3].copy_from_slice(&rgb);
            }
        }
        usize::try_from(rect.area()).unwrap_or(usize::MAX)
    }

    /// 非黑像素数（任一通道非 0）。`[MUST-GATE-015]` 的"非全黑"判据。
    #[must_use]
    pub fn non_black_pixels(&self) -> u64 {
        self.pixels
            .chunks_exact(3)
            .filter(|p| p[0] != 0 || p[1] != 0 || p[2] != 0)
            .count() as u64
    }

    /// 是否全黑。`[MUST-GATE-015]` 明文要求 Golden **不得**全黑。
    #[must_use]
    pub fn is_all_black(&self) -> bool {
        self.pixels.iter().all(|byte| *byte == 0)
    }

    /// 不同颜色（RGB 三元组）的数量 —— 用来把"非全黑"从"只有一个像素亮着"里区分出来。
    ///
    /// 用 `BTreeSet` 而不是 `HashSet`：数量本身与顺序无关，但红线 4 的精神是同一份输入
    /// 必须给出同一份确定的输出，这里没有理由引入哈希随机序。
    #[must_use]
    pub fn distinct_color_count(&self) -> usize {
        self.pixels
            .chunks_exact(3)
            .map(|p| [p[0], p[1], p[2]])
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    }

    /// 非黑像素占比（`0.0..=1.0`）。
    #[must_use]
    pub fn non_black_ratio(&self) -> f64 {
        if self.pixels.is_empty() {
            return 0.0;
        }
        self.non_black_pixels() as f64 / self.size.pixel_count() as f64
    }

    /// 转为灰度（ITU-R BT.601-7 亮度系数 `Y = 0.299R + 0.587G + 0.114B`），供 SSIM 消费。
    ///
    /// 出处：ITU-R BT.601-7 的 luma 系数，也是 `skimage.color.rgb2gray` 使用的口径
    /// （<https://scikit-image.org/docs/stable/api/skimage.color.html#skimage.color.rgb2gray>）。
    #[must_use]
    pub fn to_luma(&self) -> LumaImage {
        let data = self
            .pixels
            .chunks_exact(3)
            .map(|p| 0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2]))
            .collect();
        LumaImage {
            size: self.size,
            data,
        }
    }
}

/// 单通道亮度图（`f64`，范围 `0.0..=255.0`），SSIM 的输入。
#[derive(Debug, Clone, PartialEq)]
pub struct LumaImage {
    size: Size,
    data: Vec<f64>,
}

impl LumaImage {
    /// 尺寸。
    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }

    /// 取某点亮度；越界返回 `None`。
    #[must_use]
    pub fn at(&self, x: u32, y: u32) -> Option<f64> {
        if x >= self.size.width || y >= self.size.height {
            return None;
        }
        Some(self.data[y as usize * self.size.width as usize + x as usize])
    }

    /// 全部亮度值（行主序）。
    #[must_use]
    pub fn data(&self) -> &[f64] {
        &self.data
    }

    /// 从原始亮度值构造（测试与手工构造用）。
    pub fn from_raw(size: Size, data: Vec<f64>) -> Option<Self> {
        if size.is_empty() || data.len() != usize::try_from(size.pixel_count()).ok()? {
            return None;
        }
        Some(Self { size, data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 判据: `[MUST-GATE-015]` 的"尺寸非零"在类型层就被钉死 —— 零尺寸无法构造。
    #[test]
    #[should_panic(expected = "零尺寸图像")]
    fn zero_sized_image_cannot_be_constructed() {
        let _ = Rgb8Image::new(Size::new(0, 10));
    }

    /// 判据: `from_raw` 校验长度，长度不符必须报错而不是静默错位。
    #[test]
    fn from_raw_rejects_wrong_length() {
        assert_eq!(
            Rgb8Image::from_raw(Size::new(2, 2), vec![0; 11]),
            Err(ImageError::BufferLengthMismatch {
                expected: 12,
                actual: 11
            })
        );
        assert!(Rgb8Image::from_raw(Size::new(2, 2), vec![0; 12]).is_ok());
    }

    /// 判据: 全黑判定与"非全黑"计数互为补集，且 `[MUST-GATE-015]` 的两种失败可区分。
    #[test]
    fn all_black_detection_matches_non_black_count() {
        let mut image = Rgb8Image::new(Size::new(4, 3));
        assert!(image.is_all_black());
        assert_eq!(image.non_black_pixels(), 0);
        assert_eq!(image.distinct_color_count(), 1);

        image.set_pixel(3, 2, [1, 0, 0]);
        assert!(!image.is_all_black());
        assert_eq!(image.non_black_pixels(), 1);
        assert_eq!(image.distinct_color_count(), 2);
        assert!((image.non_black_ratio() - 1.0 / 12.0).abs() < f64::EPSILON);
    }

    /// 判据: `Rect::intersect` 必须夹取（保护遮罩不越界写内存），并区分"部分可见/完全不可见"。
    #[test]
    fn rect_intersection_clamps_and_drops_offscreen() {
        let size = Size::new(10, 10);
        assert_eq!(
            Rect::new(-5, -5, 8, 8).intersect(size),
            Some(Rect::new(0, 0, 3, 3))
        );
        assert_eq!(
            Rect::new(8, 8, 100, 100).intersect(size),
            Some(Rect::new(8, 8, 2, 2))
        );
        assert_eq!(Rect::new(10, 0, 5, 5).intersect(size), None);
        assert_eq!(Rect::new(0, 0, 0, 5).intersect(size), None);
        assert_eq!(Rect::new(-50, -50, 5, 5).intersect(size), None);
    }

    /// 判据: `fill_rect` 返回值必须等于真正写入的像素数（遮罩判据依赖这个数）。
    #[test]
    fn fill_rect_reports_written_pixels() {
        let mut image = Rgb8Image::new(Size::new(10, 10));
        assert_eq!(image.fill_rect(Rect::new(0, 0, 4, 3), MASK_COLOR), 12);
        assert_eq!(image.fill_rect(Rect::new(-5, -5, 8, 8), MASK_COLOR), 9);
        assert_eq!(image.fill_rect(Rect::new(100, 100, 4, 4), MASK_COLOR), 0);
    }

    /// 判据: 亮度转换用 BT.601 系数，纯色与手工算值逐位一致。
    #[test]
    fn luma_uses_bt601_coefficients() {
        let mut image = Rgb8Image::new(Size::new(3, 1));
        image.set_pixel(0, 0, [255, 255, 255]);
        image.set_pixel(1, 0, [0, 0, 0]);
        image.set_pixel(2, 0, [255, 0, 0]);
        let luma = image.to_luma();
        assert_eq!(luma.at(0, 0), Some(255.0));
        assert_eq!(luma.at(1, 0), Some(0.0));
        assert!((luma.at(2, 0).unwrap_or_default() - 0.299 * 255.0).abs() < 1e-9);
    }
}
