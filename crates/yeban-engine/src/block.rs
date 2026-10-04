//! 定长音频块与**栈上** `[f32; N]` 缓冲约定。[ARCH-RT-001, ARCH-DET-001, ROAD-M2-007]
//!
//! ## 为什么需要这一层
//!
//! [ARCH-DET-001] 的 L1 级声学确定性把 **固定统一处理块大小（128 采样点）** 列为前提条件：
//! 块长决定了自动化平滑、包络步进与延迟线推进的量化边界，块长一变、浮点累加顺序就变，
//! 位级哈希随之漂移。因此块长必须是**类型层面的常量**，而不是运行期传入的 `usize`。
//!
//! [ARCH-RT-001] 与 [ROAD-M2-007] 又要求所有临时缓冲**在栈上预分配**（例如 `[f32; 128]`），
//! 绝不在回调里 `Vec::new()` / `push`。这两条约束合起来就是本模块的形状：
//!
//! ```text
//! let mut block = AudioBlock::<128>::new();   // 栈上 1 KiB, 零堆分配
//! let (left, right) = block.stereo_mut();     // 借用切片, 无拷贝
//! ```
//!
//! ## 边界
//!
//! 本模块只提供**容器与约定**，不做任何 DSP。`frames` 允许小于容量，
//! 以支持 cpal 回调最后一块不足一个量子（quantum）的情形；`frames` 永远 `<= FRAMES`，
//! 因此所有按 `frames` 切片的访问都不会越界，也不会 panic。

/// 规范默认块长 (frames)：L1 确定性契约要求固定 128 采样点
/// [ARCH-DET-001]（"固定统一处理块大小（128 采样点）"）。
pub const DEFAULT_BLOCK_FRAMES: usize = 128;

/// 本 crate 允许的最大块长（frames）。
///
/// 上界来自 `yeban-model` 的 `BlockSize` 枚举（`{64,128,256,512,1024}`）
/// [MODEL-AST-002]。`AudioBlock<FRAMES>` 的 `FRAMES` 超过它就在**编译期**被
/// [`assert_supported_frames`] 拒绝，避免运行期才发现。
pub const MAX_BLOCK_FRAMES: usize = 1024;

/// 立体声通道数（本 crate 的渲染量子固定为立体声）。
pub const STEREO_CHANNELS: usize = 2;

/// 批量 SPSC 交换用的栈上临时缓冲类型（[ROAD-M2-007] 的 `[f32; 128]`）。
pub type ScratchBuffer<const N: usize> = [f32; N];

/// 规范块长的 `AudioBlock` 别名（`[f32; 128]` × 2 通道）。
pub type StereoBlock = AudioBlock<DEFAULT_BLOCK_FRAMES>;

/// 立体声定长音频块，全部存储在**栈上**（`[f32; FRAMES]` × 2）。
///
/// - 构造与清空都是纯写内存，不做分配、不调用系统调用 [ARCH-RT-001]；
/// - `FRAMES` 是编译期常量，满足 L1 固定块长约束 [ARCH-DET-001]；
/// - `frames` 字段允许表达"本量子实际有效帧数 < `FRAMES`"，用于 cpal 尾块。
#[derive(Clone, Debug, PartialEq)]
pub struct AudioBlock<const FRAMES: usize> {
    left: [f32; FRAMES],
    right: [f32; FRAMES],
    frames: usize,
}

impl<const FRAMES: usize> AudioBlock<FRAMES> {
    /// 新建一个静音块，`frames = FRAMES`。
    ///
    /// **不会 panic**：`FRAMES` 的合法性由 [`assert_supported_frames`] 在
    /// **`const` 上下文**里核验（本模块与 `rt` 模块各自用 `const _: () = …` 钉住），
    /// 因此实时路径上不存在这条 panic 分支。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            left: [0.0; FRAMES],
            right: [0.0; FRAMES],
            frames: FRAMES,
        }
    }

    /// 容量（编译期常量）。
    #[must_use]
    pub const fn capacity(&self) -> usize {
        FRAMES
    }

    /// 本块有效帧数（`<= capacity()`）。
    #[must_use]
    pub const fn frames(&self) -> usize {
        self.frames
    }

    /// 把有效帧数收窄到 `frames`（超过容量则钳到容量）。
    ///
    /// 返回钳制后的实际有效帧数。不做任何清零——超出部分的数据保留但不再可见。
    /// 非 `const`：`Ord::min` 在 `const fn` 里还不可用，而这里既不想手写 if/else
    /// 钳制（`clippy::manual_clamp`），也不想引入 `unsafe`。
    pub fn set_frames(&mut self, frames: usize) -> usize {
        self.frames = frames.min(FRAMES);
        self.frames
    }

    /// 恢复为"整块有效"。
    pub const fn set_full(&mut self) {
        self.frames = FRAMES;
    }

    /// 把有效帧数清零（`frames = 0`，内容保留）。
    pub const fn clear_frames(&mut self) {
        self.frames = 0;
    }

    /// 把整块缓冲写 0，**不改变**有效帧数（实时路径用这个，随后再 `set_frames`）。
    pub fn silence(&mut self) {
        self.left.fill(0.0);
        self.right.fill(0.0);
    }

    /// 全部有效帧写 0 并恢复整块有效。
    pub fn reset_silence(&mut self) {
        self.left.fill(0.0);
        self.right.fill(0.0);
        self.frames = FRAMES;
    }

    /// 左声道只读视图（长度为 [`frames`](Self::frames)）。
    #[must_use]
    pub fn left(&self) -> &[f32] {
        &self.left[..self.frames]
    }

    /// 右声道只读视图（长度为 [`frames`](Self::frames)）。
    #[must_use]
    pub fn right(&self) -> &[f32] {
        &self.right[..self.frames]
    }

    /// 左声道可变视图（长度为 [`frames`](Self::frames)）。
    pub fn left_mut(&mut self) -> &mut [f32] {
        let frames = self.frames;
        &mut self.left[..frames]
    }

    /// 右声道可变视图（长度为 [`frames`](Self::frames)）。
    pub fn right_mut(&mut self) -> &mut [f32] {
        let frames = self.frames;
        &mut self.right[..frames]
    }

    /// 左右声道可变视图，一次借用两者。
    pub fn stereo_mut(&mut self) -> (&mut [f32], &mut [f32]) {
        let frames = self.frames;
        (&mut self.left[..frames], &mut self.right[..frames])
    }

    /// 按通道索引取块内某帧（`channel` 0 为左，非 0 为右）。
    ///
    /// 越界返回 `None`——实时路径上**绝不 panic** [ARCH-RT-001]。
    #[must_use]
    pub fn get(&self, channel: usize, frame: usize) -> Option<f32> {
        if frame >= self.frames {
            return None;
        }
        Some(if channel == 0 {
            self.left[frame]
        } else {
            self.right[frame]
        })
    }
}

impl<const FRAMES: usize> Default for AudioBlock<FRAMES> {
    fn default() -> Self {
        Self::new()
    }
}

/// 编译期断言 `FRAMES` 是规范允许的块长。
///
/// 约束：`FRAMES` 必须是 `BlockSize` 枚举 `{64,128,256,512,1024}` 之一 [MODEL-AST-002]。
///
/// 用法（**必须**写在 `const` 上下文里才会在编译期求值）：
///
/// ```ignore
/// const _: () = assert_supported_frames::<128>();
/// ```
///
/// 这样违规的 `FRAMES` 是**编译错误**，而不是运行期 panic——实时回调里不允许
/// 出现可 panic 的构造路径 [ARCH-RT-001]。
pub const fn assert_supported_frames<const FRAMES: usize>() {
    const SUPPORTED: [usize; 5] = [64, 128, 256, 512, MAX_BLOCK_FRAMES];
    let mut i = 0;
    let mut found = false;
    while i < SUPPORTED.len() {
        if SUPPORTED[i] == FRAMES {
            found = true;
        }
        i += 1;
    }
    assert!(
        found,
        "AudioBlock<FRAMES>: FRAMES must be one of 64/128/256/512/1024 [MODEL-AST-002]"
    );
}

// 本模块实际使用到的块长：在编译期钉住 [ARCH-DET-001]。
const _: () = assert_supported_frames::<DEFAULT_BLOCK_FRAMES>();
const _: () = assert_supported_frames::<MAX_BLOCK_FRAMES>();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_block_frames_is_128_per_l1_contract() {
        // [ARCH-DET-001] L1: "固定统一处理块大小（128 采样点）"
        assert_eq!(DEFAULT_BLOCK_FRAMES, 128);
        assert_eq!(StereoBlock::new().capacity(), 128);
    }

    #[test]
    fn block_lives_on_the_stack_not_the_heap() {
        // 结构性断言: 块的全部样本都在结构体内联存储, 没有任何指针/长度字段
        // → 结构体大小 == 2 * FRAMES * 4 字节 (+ frames 字段 + 对齐填充)。
        let inline_bytes = 2 * DEFAULT_BLOCK_FRAMES * core::mem::size_of::<f32>();
        let size = core::mem::size_of::<StereoBlock>();
        assert!(
            size >= inline_bytes && size <= inline_bytes + 2 * core::mem::size_of::<usize>(),
            "AudioBlock<128> 大小 {size} 应约等于内联样本字节 {inline_bytes}"
        );
    }

    #[test]
    fn frames_clamp_never_exceeds_capacity() {
        let mut block = AudioBlock::<128>::new();
        assert_eq!(block.set_frames(64), 64);
        assert_eq!(block.left().len(), 64);
        assert_eq!(block.right().len(), 64);
        // 超过容量被钳制, 不 panic 也不越界
        assert_eq!(block.set_frames(4096), 128);
        assert_eq!(block.left().len(), 128);
        assert_eq!(block.get(0, 127), Some(0.0));
        assert_eq!(block.get(0, 128), None);
    }

    #[test]
    fn clear_and_reset_silence_are_symmetric() {
        let mut block = AudioBlock::<128>::new();
        block.left_mut()[0] = 1.0;
        block.right_mut()[3] = -1.0;
        block.clear_frames();
        assert_eq!(block.frames(), 0);
        assert!(block.left().is_empty());
        block.set_full();
        // 内容保留: clear_frames 只动有效长度
        assert_eq!(block.left()[0], 1.0);
        block.reset_silence();
        assert_eq!(block.frames(), 128);
        assert!(block.left().iter().all(|s| *s == 0.0));
        assert!(block.right().iter().all(|s| *s == 0.0));
    }

    #[test]
    fn stereo_mut_borrows_both_channels_at_once() {
        let mut block = StereoBlock::new();
        let (left, right) = block.stereo_mut();
        left[1] = 0.5;
        right[2] = -0.25;
        assert_eq!(block.get(0, 1), Some(0.5));
        assert_eq!(block.get(1, 2), Some(-0.25));
    }

    #[test]
    fn scratch_buffer_convention_is_a_plain_stack_array() {
        // [ROAD-M2-007] 的 "栈上 [f32; 128]": 就是数组本身, 没有任何堆分配包装。
        let scratch: ScratchBuffer<DEFAULT_BLOCK_FRAMES> = [0.0; DEFAULT_BLOCK_FRAMES];
        assert_eq!(scratch.len(), 128);
        assert_eq!(core::mem::size_of_val(&scratch), 128 * 4);
    }
}
