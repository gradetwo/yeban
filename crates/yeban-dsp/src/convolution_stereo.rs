//! 真立体声（四通路）卷积混响 —— [`Convolution`] 的上下层装配。[ARCH-RT-001] [ARCH-PDC-001]
//!
//! [`crate::convolution`] 交付的是**单声道卷积核**，它的 §7 把"真立体声 / 四通路卷积"
//! 列为**没有**做的一项，并写道："立体声 = 两个实例（普通立体声）或四个实例
//! （真立体声），由上层装配。本模块**不**规定那套装配"。本模块就是那套装配 ——
//! 它是**逐样本**的路径拓扑，因此属于本 crate 的实时预算之内（`ARCH-RT-001` 的
//! 零分配承诺在这里同样成立），而不是上层的接线决定。
//!
//! ## 1. 什么是"真立体声"
//!
//! 普通立体声混响是**两条互不相干的单声道**：`L → (h_L) → L`、
//! `R → (h_R) → R`。它卷不出"左声道的能量跑到右声道去"这件事，因此丢掉了空间的
//! **交叉**信息。
//!
//! 真立体声要**四条**脉冲响应，构成一个 2×2 的卷积矩阵：
//!
//! ```text
//! out_L = in_L * h_LL + in_R * h_RL
//! out_R = in_L * h_LR + in_R * h_RR
//! ```
//!
//! `h_RL` 是"在左声道激励、于右声道接收"的那条测量结果。四条合起来才是那段空间的
//! 双耳传递函数，也正是 ReaVerb 的 "True Stereo" 模式要的形态。
//!
//! ## 2. 本模块钉住的是**映射**，不是**文件格式**
//!
//! `h_LL` / `h_LR` / `h_RL` / `h_RR` 是四个**具名**参数，本模块**不**对任何一种
//! 4 通道 IR 文件的通道顺序表态。原因是核实过的：两份外部权威说法都在本机取不到
//! ——`forum.cockos.com` 的帖子与 `forum.steinberg.net` 的帖子都要求 JavaScript
//! 才放行（实测：HTTP 200 + "validating your browser"，二次抓取同样被挡）。
//! ⚠ 因此**本模块不声称**与 ReaVerb 的通道顺序逐条对齐；它只保证
//! "你给的 `h_XY` 就是被当作 `X → Y` 用的那一对"。把某种 4 通道文件解成交错/非交错
//! 的活由调用方（或未来的容器层）做，那是 I/O，`yeban-dsp` 的物理边界之外。
//!
//! 判据 [`tests::the_four_paths_are_wired_as_named`] 用一个**逐条**的脉冲实验钉住
//! 这个映射：向 `L` 打一个单位脉冲 ⇒ 偶数槽位（`L` 输出）出现 `h_LL`、奇数槽位
//! （`R` 输出）出现 `h_LR`；向 `R` 打脉冲 ⇒ `L` 出 `h_RL`、`R` 出 `h_RR`。
//! 四条 IR 取**互不相同**的单抽头（1 / 2 / 4 / 8）⇒ 任何一处错接都会被点名。
//!
//! ## 3. 四条 IR 必须等长
//!
//! [`TrueStereoConvolution::set_impulse_response`] 要求四个切片**长度相同**。
//! 不等长（或任一为空）时它**拒绝**配置、把实例置回直通并返回 `0`。
//!
//! 为什么是拒绝而不是逐条截断到最短：四条 IR 代表**同一个空间**的四个通路，
//! "左耳比右耳短 3 000 帧"没有物理意义，只会是调用方的接线错误。拒绝让这个错误
//! 有一个**响亮**的后果（`is_configured()` 为假 ⇒ 输出逐位等于输入），而不是一段
//! 听起来只是有点怪的声音。长度上限与截断语义沿用 [`CONV_MAX_IR_FRAMES`]。
//!
//! 同一条拒绝纪律也覆盖**每一条** IR 的取值：任一条含非有限样本（`NaN`/`±inf`）
//! 或频谱溢出时，四条**一起**被拒绝 —— 只留三条新型号会让被拒那条继续用旧 IR 响，
//! 而"四条代表同一个空间"的前提已经被破坏。每条的校验本体在
//! [`crate::convolution::Convolution::set_impulse_response`]，本层负责把四个返回值
//! 收齐并裁决。
//!
//! ## 4. 分配纪律 [ARCH-RT-001]
//!
//! [`TrueStereoConvolution::set_impulse_response`] 是**唯一**的分配入口（它把四个
//! 切片转发给四个 [`Convolution`]，各自的分配入口随之被调用）。必须在音频回调
//! **之外**调用。[`TrueStereoConvolution::process`] / [`TrueStereoConvolution::reset`]
//! 逐样本零分配：本类型自带的**六条定长内联 scratch**（两条输入 + 四条输出，
//! 构造期就在结构体里、不经堆）只在块内做拷贝；块内没有 `push` / `Box` /
//! `collect`。运行期读数见
//! `tests/convolution_stereo_rt_zero_alloc.rs`（同一套计数型分配器仪器）。
//!
//! ## 5. 延迟 [ARCH-PDC-001]
//!
//! 四条通路各自的延迟都是 [`CONV_LATENCY`] = 0，故本器件的延迟也是 **0**。

use crate::convolution::{CONV_BLOCK_FRAMES, CONV_LATENCY, CONV_MAX_IR_FRAMES, Convolution};

/// 通路的条数：`2` 个输入声道 × `2` 个输出声道。单位：**条**。
pub const TRUE_STEREO_PATHS: usize = 4;

/// 真立体声卷积：四条 [`Convolution`] 按 2×2 矩阵装配。
///
/// 内部布局是 `kernels[输入声道][输出声道]`，即
/// `[[h_LL, h_LR], [h_RL, h_RR]]`。取这个形状而不是一维数组，是为了让
/// [`Self::process`] 能在一条输入声道内**同时**借到那两条 `Convolution` 的可变引用
/// （`split_at_mut` / 行切片），从而逐路径分派而**不引入**任何运行期索引检查失败路径，
/// 也不需要在结构体里再放一层"当前路径"的游标状态。
pub struct TrueStereoConvolution {
    /// `kernels[输入声道][输出声道]`。见类型注释。
    kernels: [[Convolution; 2]; 2],
    /// 两条**输入**声道的定长 scratch：`in_scratch[声道]`，声道 `0` = `L`、`1` = `R`。
    /// 每条是 [`CONV_BLOCK_FRAMES`] 个 `f32` 的**内联数组**，随结构体一次成型
    /// ⇒ 不经过堆。用途见 [`Self::process`]。
    in_scratch: [[f32; CONV_BLOCK_FRAMES]; 2],
    /// 四条通路的**输出** scratch，布局与 [`Self::kernels`] 相同：
    /// `out_scratch[输入声道][输出声道]`。同 [`Self::in_scratch`]，内联、不经堆。
    out_scratch: [[[f32; CONV_BLOCK_FRAMES]; 2]; 2],
    /// 接受的 IR 帧数（四条相同）；`0` = 未配置。
    ir_frames: usize,
    /// 是否已配置（四个切片等长且非空）。
    configured: bool,
}

impl TrueStereoConvolution {
    /// 构造一个**未配置**的实例（直通）。不分配。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            kernels: [
                [Convolution::new(), Convolution::new()],
                [Convolution::new(), Convolution::new()],
            ],
            in_scratch: [[0.0; CONV_BLOCK_FRAMES]; 2],
            out_scratch: [[[0.0; CONV_BLOCK_FRAMES]; 2]; 2],
            ir_frames: 0,
            configured: false,
        }
    }

    /// 一次配置四条脉冲响应：`h_ll` / `h_lr` / `h_rl` / `h_rr`。
    ///
    /// 参数名就是映射：`h_lr` 是 `L → R` 那条（在左声道激励、于右声道接收）。
    ///
    /// 返回**真正接受**的帧数：四个切片等长且全部通过每条的校验时为
    /// `min(len, CONV_MAX_IR_FRAMES)`；四个切片不等长、任一为空、或任一条的 IR 校验
    /// 不通过（非有限样本 / 频谱溢出，见
    /// [`crate::convolution::Convolution::set_impulse_response`]）时返回 `0`，
    /// 实例被置回**未配置的直通**状态（四条核一起清空）。
    ///
    /// "任一条不通过就四条一起拒绝"是**必须**的：四条等长的假设是这套 2×2 装配的
    /// 前提，只留三条新型号会让被拒那条继续用旧 IR 响。
    ///
    /// **这是本类型唯一的分配入口**，必须在音频回调之外调用。**长度不变**时
    /// 缓冲区原地复用（零分配），长度改变时按新长度重新分配 —— 与
    /// [`Convolution::set_impulse_response`] 同一条纪律。
    pub fn set_impulse_response(
        &mut self,
        h_ll: &[f32],
        h_lr: &[f32],
        h_rl: &[f32],
        h_rr: &[f32],
    ) -> usize {
        if h_ll.len() != h_lr.len() || h_ll.len() != h_rl.len() || h_ll.len() != h_rr.len() {
            self.reject();
            return 0;
        }
        let frames = h_ll.len().min(CONV_MAX_IR_FRAMES);
        if frames == 0 {
            self.reject();
            return 0;
        }

        // 逐条转发。四条等长 ⇒ 四个返回值只有在**校验拒绝**（返回 0）时才不同。
        let accepted_ll = self.kernels[0][0].set_impulse_response(&h_ll[..frames]);
        let accepted_lr = self.kernels[0][1].set_impulse_response(&h_lr[..frames]);
        let accepted_rl = self.kernels[1][0].set_impulse_response(&h_rl[..frames]);
        let accepted_rr = self.kernels[1][1].set_impulse_response(&h_rr[..frames]);
        if accepted_ll != frames
            || accepted_lr != frames
            || accepted_rl != frames
            || accepted_rr != frames
        {
            self.reject();
            return 0;
        }

        self.ir_frames = frames;
        self.configured = true;
        frames
    }

    /// 拒绝一次配置：置回未配置的**直通**，并把四条核的 IR 频谱与历史都清空。
    ///
    /// 清空四条核是必须的：否则"未配置"只是本结构体上的一个标志位，而四条核里
    /// 还留着上一次 IR 的能量（若将来 `process` 的守卫被改坏，泄漏就会真的响）。
    /// ⚠ 它**不**释放四条核已经拿到的堆缓冲（`Convolution` 的拒绝路径只做原地清零，
    /// `Vec` 的容量保留）—— 这里清的是内容，不是容量。
    fn reject(&mut self) {
        for row in &mut self.kernels {
            for kernel in row {
                // 空 IR ⇒ 该核回到未配置的直通状态（内容清零，容量保留）。
                kernel.set_impulse_response(&[]);
            }
        }
        self.ir_frames = 0;
        self.configured = false;
    }

    /// 是否已配置（四个等长且非空的切片曾经被接受）。
    #[must_use]
    pub const fn is_configured(&self) -> bool {
        self.configured
    }

    /// 接受的 IR 帧数（四条相同；`0` = 未配置）。
    #[must_use]
    pub const fn ir_frames(&self) -> usize {
        self.ir_frames
    }

    /// 本器件引入的处理延迟（帧），恒为 [`CONV_LATENCY`] = 0 [ARCH-PDC-001]。
    #[must_use]
    pub const fn latency_samples(&self) -> usize {
        CONV_LATENCY
    }

    /// 清空四条通路的频域延迟线与重叠相加尾（零分配）。
    ///
    /// 下一个块与"刚配置完"逐位相同 —— 这条契约由四条 [`Convolution::reset`] 各自
    /// 承担，本方法不做别的事。
    pub fn reset(&mut self) {
        for row in &mut self.kernels {
            for kernel in row {
                kernel.reset();
            }
        }
    }

    /// 原地处理一个**交错立体声**块（`[L, R, L, R, …]`），返回处理的**帧**数。
    ///
    /// 语义（与 [`Convolution::process`] 逐条对齐）：
    ///
    /// 1. 未配置时**直通**（逐位不变）；
    /// 2. 处理的帧数是 `block.len() / 2`，且**至多** [`CONV_BLOCK_FRAMES`] 帧
    ///    （超出的尾部**原样不碰**）。奇数长度的块的**最后一帧**（只有 `L`、没有
    ///    `R`）不参与处理，也原样不碰；
    /// 3. 短块按零补齐推进历史（与单声道核一致）：`n < CONV_BLOCK_FRAMES` 时四条核
    ///    各推进一个整块，但只写回前 `n` 帧；
    /// 4. 全程零分配、零锁、零 I/O [ARCH-RT-001]。
    pub fn process(&mut self, block: &mut [f32]) -> usize {
        let frames = (block.len() / 2).min(CONV_BLOCK_FRAMES);
        if !self.configured || frames == 0 {
            return frames;
        }

        // 段 1：解交错成两条**连续**的输入声道（`0` = `L`、`1` = `R`）。
        // 这一步不能省：`in[2i]`（L 输入）与 `L → L` 的输出是同一格，直接原地做
        // 会让先跑的通路覆盖掉后跑的还要读的输入。
        for (channel, scratch) in self.in_scratch.iter_mut().enumerate() {
            for i in 0..frames {
                scratch[i] = block[2 * i + channel];
            }
        }

        // 段 2：每条输入声道跑两条核，各写自己的**输出** scratch：
        //   out[channel][0] = in[channel] * h_{channel → L}
        //   out[channel][1] = in[channel] * h_{channel → R}
        // `out_scratch[输入][输出]` 与 `kernels[输入][输出]` 同序 ⇒ 下标一一对应。
        //
        // ⚠ `Convolution::process` 是**原地**的（结果写回传进去的那一段），
        // 而同一段输入要被两条核各读一次 ⇒ 两条核必须各拿一份输入：
        // 一条读 `in_scratch[channel]`，另一条读它的副本。这里让 `→ L` 读原件
        // （结果被拷走后原件即被丢弃）、`→ R` 读副本（原件因此始终只被读一次）。
        for channel in 0..2 {
            let mut copy = [0.0f32; CONV_BLOCK_FRAMES];
            copy[..frames].copy_from_slice(&self.in_scratch[channel][..frames]);
            // `→ R`：读副本，写输出段。
            self.kernels[channel][1].process(&mut copy[..frames]);
            self.out_scratch[channel][1][..frames].copy_from_slice(&copy[..frames]);
            // `→ L`：读原件（就地覆盖），结果拷进输出段。
            let mut original = [0.0f32; CONV_BLOCK_FRAMES];
            core::mem::swap(&mut original, &mut self.in_scratch[channel]);
            self.kernels[channel][0].process(&mut original[..frames]);
            self.out_scratch[channel][0][..frames].copy_from_slice(&original[..frames]);
            // 原件已消费；段 1 会在下一次调用时重新填满它，这里归零即可。
        }

        // 段 3：交错回调用方的块，并把每条输出声道的两条通路贡献相加：
        //   out_L = in_L * h_LL + in_R * h_RL
        //   out_R = in_L * h_LR + in_R * h_RR
        for i in 0..frames {
            block[2 * i] = self.out_scratch[0][0][i] + self.out_scratch[1][0][i];
            block[2 * i + 1] = self.out_scratch[0][1][i] + self.out_scratch[1][1][i];
        }
        frames
    }
}

impl Default for TrueStereoConvolution {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for TrueStereoConvolution {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // 手写而不是 derive：四个 `Convolution` 各有上万个内部元素。
        f.debug_struct("TrueStereoConvolution")
            .field("configured", &self.configured)
            .field("ir_frames", &self.ir_frames)
            .field("paths", &TRUE_STEREO_PATHS)
            .field("latency_samples", &CONV_LATENCY)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::Rng;

    /// 直接时域卷积（参照实现，`O(n·m)`）。**独立参照**：它不共用被测代码的任何一行。
    fn direct(input: &[f32], ir: &[f32]) -> Vec<f32> {
        let mut out = vec![0.0f32; input.len() + ir.len().saturating_sub(1)];
        for (i, &x) in input.iter().enumerate() {
            for (j, &h) in ir.iter().enumerate() {
                out[i + j] += x * h;
            }
        }
        out
    }

    /// 把交错的 `frames` 帧输入喂进去，收集 `out_frames` 帧交错输出。
    fn render(
        conv: &mut TrueStereoConvolution,
        interleaved_in: &[f32],
        out_frames: usize,
    ) -> Vec<f32> {
        let mut out = vec![0.0f32; out_frames * 2];
        let mut written = 0usize;
        while written < out_frames {
            let take = (out_frames - written).min(CONV_BLOCK_FRAMES);
            let chunk = &mut out[written * 2..(written + take) * 2];
            let src_start = written * 2;
            let src_end = (src_start + take * 2).min(interleaved_in.len());
            if src_end > src_start {
                chunk[..src_end - src_start].copy_from_slice(&interleaved_in[src_start..src_end]);
            }
            conv.process(chunk);
            written += take;
        }
        out
    }

    /// 确定性伪随机序列（本 crate 的 xorshift，显式播种）。
    fn pseudo_random(len: usize, seed: u32) -> Vec<f32> {
        let mut rng = Rng::new(seed);
        (0..len).map(|_| rng.next_bipolar()).collect()
    }

    /// 从交错缓冲里取一条声道（偶 = `L`，奇 = `R`）。
    fn channel(interleaved: &[f32], right: bool) -> Vec<f32> {
        interleaved
            .iter()
            .skip(usize::from(right))
            .step_by(2)
            .copied()
            .collect()
    }

    /// **判据（可红）**：四通路卷积必须与四条**直接时域卷积**的矩阵求和逐样本一致。
    ///
    /// 观测方式：四条各 300 帧的 IR（**跨 3 个分区**，最后一条不满）与 600 帧交错
    /// 伪随机输入。参照由 `direct` 逐条算出四条线性卷积，再按
    /// `out_L = in_L * h_LL + in_R * h_RL`、`out_R = in_L * h_LR + in_R * h_RR` 相加。
    ///
    /// 注入（实测两种，见报告）：把 `kernels[1][0]` 换成 `kernels[0][0]`（`R → L`
    /// 那路用错核）⇒ 本判据变红；把 `process` 里的两条输出核**对调**
    /// （`kernel_to_l` / `kernel_to_r`）⇒ 本判据变红。
    #[test]
    fn the_four_paths_match_four_direct_convolutions() {
        const FRAMES_IN: usize = 600;
        let h_ll = pseudo_random(300, 0x1111_0001);
        let h_lr = pseudo_random(300, 0x1111_0002);
        let h_rl = pseudo_random(300, 0x1111_0003);
        let h_rr = pseudo_random(300, 0x1111_0004);
        let inter: Vec<f32> = pseudo_random(FRAMES_IN * 2, 0x2222_0005);

        let in_l = channel(&inter, false);
        let in_r = channel(&inter, true);

        // ---- 参照：四条直接时域卷积，按矩阵相加 ----
        let ll = direct(&in_l, &h_ll);
        let lr = direct(&in_l, &h_lr);
        let rl = direct(&in_r, &h_rl);
        let rr = direct(&in_r, &h_rr);
        let out_frames = ll.len().max(rl.len());
        let mut want_l = vec![0.0f32; out_frames];
        let mut want_r = vec![0.0f32; out_frames];
        for i in 0..out_frames {
            want_l[i] = ll.get(i).copied().unwrap_or(0.0) + rl.get(i).copied().unwrap_or(0.0);
            want_r[i] = lr.get(i).copied().unwrap_or(0.0) + rr.get(i).copied().unwrap_or(0.0);
        }

        // ---- 被测 ----
        let mut conv = TrueStereoConvolution::new();
        assert_eq!(conv.set_impulse_response(&h_ll, &h_lr, &h_rl, &h_rr), 300);
        assert!(conv.is_configured());
        assert_eq!(conv.ir_frames(), 300);
        assert_eq!(conv.kernels[0][0].partitions(), 3, "300 帧 @128 ⇒ 3 个分区");
        let got = render(&mut conv, &inter, out_frames);
        let got_l = channel(&got, false);
        let got_r = channel(&got, true);

        let scale = want_l
            .iter()
            .chain(&want_r)
            .fold(0.0f32, |m, v| m.max(v.abs()))
            .max(1e-12);
        let mut worst = 0.0f32;
        for i in 0..out_frames {
            worst = worst.max((got_l[i] - want_l[i]).abs());
            worst = worst.max((got_r[i] - want_r[i]).abs());
        }
        assert!(
            worst <= scale * 1e-4,
            "四通路卷积与时域参照不符：最大绝对误差 {worst}（参照峰值 {scale}，相对 {}）",
            worst / scale
        );
    }

    /// **判据（可红）**：四条通路**逐条**接线正确 —— 映射不是猜的。
    ///
    /// 观测方式：给一条**只有一个抽头**的 IR，四条各不相同（`h_LL = 1`、
    /// `h_LR = 2`、`h_RL = 4`、`h_RR = 8`）。向 `L` 打一个单位脉冲 ⇒
    /// `out_L` 的第 0 帧必须是 `1`（`h_LL`），`out_R` 的第 0 帧必须是 `2`（`h_LR`）；
    /// 向 `R` 打一个单位脉冲 ⇒ `out_L` 是 `4`（`h_RL`）、`out_R` 是 `8`（`h_RR`）。
    /// 四个数互不相同 ⇒ 任何一处错接都会被这四条断言之一点名。
    ///
    /// 注入：把 `kernels[0]` 的两项对调 ⇒ 本判据在 `h_LL` / `h_LR` 处变红。
    #[test]
    fn the_four_paths_are_wired_as_named() {
        let mut conv = TrueStereoConvolution::new();
        conv.set_impulse_response(&[1.0], &[2.0], &[4.0], &[8.0]);
        assert!(conv.is_configured());
        assert_eq!(conv.ir_frames(), 1);

        // 左声道单位脉冲。
        let mut block = [0.0f32; CONV_BLOCK_FRAMES * 2];
        block[0] = 1.0;
        conv.process(&mut block);
        assert!(
            (block[0] - 1.0).abs() < 1e-6,
            "L→L 应为 h_LL=1，实得 {}",
            block[0]
        );
        assert!(
            (block[1] - 2.0).abs() < 1e-6,
            "L→R 应为 h_LR=2，实得 {}",
            block[1]
        );
        // 其余项必须**逐位**为零：只有 1 个抽头的 IR 不该在别处产出能量，
        // 也不该在重排边界漏下未写回的残值。
        assert!(
            block[2..].iter().all(|v| *v == 0.0),
            "1 个抽头的 IR 在别处产出了能量：{:?}",
            &block[..12]
        );

        // 右声道单位脉冲（先回到刚配置完的状态）。
        conv.reset();
        let mut block = [0.0f32; CONV_BLOCK_FRAMES * 2];
        block[1] = 1.0;
        conv.process(&mut block);
        assert!(
            (block[0] - 4.0).abs() < 1e-6,
            "R→L 应为 h_RL=4，实得 {}",
            block[0]
        );
        assert!(
            (block[1] - 8.0).abs() < 1e-6,
            "R→R 应为 h_RR=8，实得 {}",
            block[1]
        );
    }

    /// **判据（可红）**：四条 IR 不等长或任一为空 ⇒ **拒绝**配置并回到直通。
    ///
    /// 观测方式：五种坏输入（第 4 条短 1 帧、第 1 条长 1 帧、第 2 条为空、四条全空、
    /// 先成功配置再喂坏输入）都必须返回 `0`、`is_configured()` 为假、且输出**逐位**
    /// 等于输入。
    ///
    /// 注入：把长度检查那一行删掉（改成只查 `h_ll`）⇒ 第 1、2、5 种情形变红。
    #[test]
    fn four_unequal_or_empty_irs_are_rejected_and_force_passthrough() {
        let ok = pseudo_random(64, 0x3333_0001);
        let short = pseudo_random(63, 0x3333_0002);
        let long = pseudo_random(65, 0x3333_0003);

        // 四条 IR 的切片组（`h_ll` / `h_lr` / `h_rl` / `h_rr`）。
        type FourIrs<'a> = (&'a [f32], &'a [f32], &'a [f32], &'a [f32]);
        let cases: [FourIrs<'_>; 4] = [
            (&ok, &ok, &ok, &short),
            (&long, &ok, &ok, &ok),
            (&ok, &[], &ok, &ok),
            (&[], &[], &[], &[]),
        ];
        for (i, (a, b, c, d)) in cases.iter().enumerate() {
            let mut conv = TrueStereoConvolution::new();
            conv.set_impulse_response(&ok, &ok, &ok, &ok); // 先成功一次，确保"拒绝"要清掉旧状态
            assert!(conv.is_configured());
            assert_eq!(
                conv.set_impulse_response(a, b, c, d),
                0,
                "坏输入 #{i} 不该被接受"
            );
            assert!(!conv.is_configured(), "坏输入 #{i} 之后仍是已配置");
            assert_eq!(conv.ir_frames(), 0);

            let mut block = [0.3f32, -0.7, 0.1, 0.2, 0.9, -0.4];
            let before = block;
            assert_eq!(conv.process(&mut block), 3);
            assert_eq!(
                block, before,
                "坏输入 #{i} 之后不是逐位直通（旧 IR 的尾巴泄漏了）"
            );
        }
    }

    /// **判据（可红）**：`reset()` 之后的重渲染与"刚配置完"**逐位**相同。
    ///
    /// 注入：把 `reset()` 的循环体清空 ⇒ 本判据变红（第一条输出会带回第一批的能量）。
    #[test]
    fn reset_returns_to_the_just_configured_state_bit_for_bit() {
        let h: Vec<f32> = pseudo_random(200, 0x4444_0001);
        let inter: Vec<f32> = pseudo_random(256 * 2, 0x4444_0002);
        let mut conv = TrueStereoConvolution::new();
        conv.set_impulse_response(&h, &h, &h, &h);
        let first = render(&mut conv, &inter, 256);
        conv.reset();
        let second = render(&mut conv, &inter, 256);
        assert!(
            first
                .iter()
                .zip(&second)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "reset() 之后的重渲染不是逐位相同"
        );
    }

    /// **判据（可红）**：未配置时是**逐位**直通，且边界块长不 panic。
    ///
    /// 观测方式：`process` 对空块、奇数长度块（末帧只有 `L`）、长于
    /// [`CONV_BLOCK_FRAMES`] 的块都不得 panic；未配置时全部原样返回。
    ///
    /// 注入：删掉 `process` 开头的 `!self.configured || frames == 0` 守卫 ⇒
    /// 未配置实例的 `frames == 0` 分支仍会走 `split_at_mut(0)`，本条**不变红**
    /// （实测，见报告：这一条是**无效注入**）；真正会让它变红的是把 `frames`
    /// 的 `min(CONV_BLOCK_FRAMES)` 删掉，此时长块会越界 panic。
    #[test]
    fn an_unconfigured_instance_is_a_bit_exact_passthrough_for_odd_and_long_blocks() {
        let mut conv = TrueStereoConvolution::new();
        assert!(!conv.is_configured());
        assert_eq!(conv.ir_frames(), 0);
        assert_eq!(conv.latency_samples(), 0);
        assert_eq!(conv.latency_samples(), CONV_LATENCY);

        // 空块：0 帧。
        let mut empty: [f32; 0] = [];
        assert_eq!(conv.process(&mut empty), 0);

        // 奇数长度（3 帧的交错 = 1 帧 + 半个）：只处理 1 帧，末项原样。
        let mut odd = [0.5f32, -0.25, 0.125];
        assert_eq!(conv.process(&mut odd), 1);
        assert_eq!(odd, [0.5, -0.25, 0.125]);

        // 长于一个量子：只处理前 128 帧（256 项），尾部原样。
        let mut long = vec![0.75f32; CONV_BLOCK_FRAMES * 2 + 8];
        for (i, v) in long.iter_mut().enumerate() {
            *v = i as f32 * 0.001;
        }
        let before = long.clone();
        assert_eq!(conv.process(&mut long), CONV_BLOCK_FRAMES);
        assert_eq!(long, before, "未配置时必须是逐位直通");
    }

    /// **判据（可红）**：任一条 IR 含非有限样本 ⇒ **四条一起**被拒绝。
    ///
    /// 观测方式：四条里各挑一条放 `NaN`（依次放在 `h_LL` / `h_LR` / `h_RL` / `h_RR`），
    /// 每次检查返回 `0`、`is_configured()` 假、`ir_frames()` 为 `0`，且随后一个交错块的
    /// 输出**逐位**等于输入。四条各测一次，因为拒绝必须覆盖全部四个转发点。
    ///
    /// 量什么：`set_impulse_response` 的返回值（单位：帧）与一个 128 帧交错块的每个比特。
    ///
    /// 注入（实测见本票报告）：把 `if accepted_ll != frames || …` 的判据换成一条
    /// 永不成立的条件（`accepted_ll == usize::MAX && …`），即**不接受**下层的裁决
    /// ⇒ 本判据在返回值断言处变红（实得 `512`，期望 `0`）。
    #[test]
    fn a_non_finite_ir_rejects_all_four_paths() {
        let good = pseudo_random(512, 0x6666_0001);
        for victim in 0..4 {
            let mut paths = [good.clone(), good.clone(), good.clone(), good.clone()];
            paths[victim][7] = f32::NAN;

            let mut conv = TrueStereoConvolution::new();
            assert_eq!(
                conv.set_impulse_response(&paths[0], &paths[1], &paths[2], &paths[3]),
                0,
                "第 {victim} 条含 NaN ⇒ 四条必须一起被拒绝"
            );
            assert!(!conv.is_configured(), "第 {victim} 条：拒绝后必须是未配置");
            assert_eq!(conv.ir_frames(), 0, "第 {victim} 条：拒绝后不得留下帧数");

            let input = pseudo_random(CONV_BLOCK_FRAMES * 2, 0x6666_0002);
            let mut block = input.clone();
            assert_eq!(conv.process(&mut block), CONV_BLOCK_FRAMES);
            for (i, (out, want)) in block.iter().zip(&input).enumerate() {
                assert_eq!(
                    out.to_bits(),
                    want.to_bits(),
                    "第 {victim} 条：拒绝后的样本 {i} 必须逐位直通"
                );
            }
        }
    }

    /// **判据**：确定性 —— 同一交错输入两次逐位相同（四条通路各自都是确定性的）。
    #[test]
    fn the_same_input_twice_is_bit_identical() {
        let h = pseudo_random(500, 0x5555_0001);
        let inter = pseudo_random(1_000 * 2, 0x5555_0002);
        let mut a = TrueStereoConvolution::new();
        let mut b = TrueStereoConvolution::new();
        a.set_impulse_response(&h, &h, &h, &h);
        b.set_impulse_response(&h, &h, &h, &h);
        let ra = render(&mut a, &inter, 1_000);
        let rb = render(&mut b, &inter, 1_000);
        assert!(
            ra.iter().zip(&rb).all(|(x, y)| x.to_bits() == y.to_bits()),
            "两次渲染不是逐位相同"
        );
    }
}
