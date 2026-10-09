//! 均匀分块卷积（uniform-partitioned overlap-add）—— 卷积混响的核。[ARCH-RT-001] [ARCH-PDC-001]
//!
//! 本模块是**新写**的（无上游来源）。它把一条**脉冲响应**（IR）与输入流做线性卷积，
//! 用频域分块把它压到实时可行的成本，并且逐块零分配。
//!
//! ## 1. 为什么需要它
//!
//! `reverb` 模块是 Freeverb 拓扑：全是梳状滤波器与全通扩散器，没有一条抽头来自
//! 真实空间。卷积混响是另一族算法 —— 它把一段**测量到的**脉冲响应原样卷进信号，
//! 因此音色就是那段空间的音色。`docs/ledger/legacy-reuse-audit.md:37` 把来源的
//! `dsp/convolution.rs`（869 行）登记为"**移植后可用**、落到 `yeban-dsp`"，但该文件
//! **尚未移植**（`docs/ledger/dsp-core-provenance.md:65`、`:117`、`:294` 三条都记着
//! "本次不移植"）。本模块不依赖那份来源（它不在本机磁盘上），是按夜半的接口与
//! 零分配纪律新写的。
//!
//! ## 2. 算法（均匀分区 + 频域延迟线）
//!
//! IR 按 [`CONV_BLOCK_FRAMES`] 切成 `P` 个分区 `h_0 … h_{P-1}`。第 `m` 个输入块在
//! 频域与全部分区相乘再求和：
//!
//! ```text
//! Y_m[k] = Σ_{p=0}^{P-1} H_p[k] · X_{m−p}[k]
//! ```
//!
//! `X_{m−p}` 是前 `P` 个输入块的频谱，存在一条环形"频域延迟线"里（**不是**时域
//! 环形缓冲）。一次逆变换给出 `h_p * x_{m−p}` 的叠加；它的前 `B` 帧是第 `m` 个输出块，
//! 后 `B` 帧是重叠相加（OLA）尾，留给下一个块。这就是标准的 UPOLA。
//!
//! ## 3. 延迟与补偿 [ARCH-PDC-001]
//!
//! 输出块与输入块**对齐**（第 `m` 个输出只用到 `m` 及更早的输入），因此
//! [`CONV_LATENCY`] = **0**。分块**不**引入额外延迟，这是 UPOLA 相对"整段 FFT"
//! 的关键好处。判据：脉冲的第 0 帧必须立刻产出 IR 的第 0 个抽头。
//!
//! ## 4. 分配纪律 [ARCH-RT-001]
//!
//! [`Convolution::set_impulse_response`] 是**唯一**的分配入口（延迟线、频谱历史、
//! 旋转因子表都在那里建），必须在音频回调**之外**调用。[`Convolution::process`] /
//! [`Convolution::reset`] 逐块零分配：内部缓冲全部在构造期建好，块内只有乘加与
//! 定长数组的原地读写。
//!
//! ⚠ 待接线时的**硬约束**（登记为缺口，见 §7）：引擎当前的器件参数更新发生在音频
//! 线程上（`crates/yeban-engine/src/insert.rs:141`）。**改变 IR 长度**会分配，因此
//! 那条路径上不能换 IR；**长度不变**的换 IR 实测零分配（判据见
//! `tests/convolution_rt_zero_alloc.rs`），但它仍要重算全部 IR 频谱（`P` 次 256 点
//! 变换，1 秒 IR 是 375 次）—— 那是乘加，不是分配。干净的口径是把"IR 变更"放在
//! 快照边界**之外**；本器件不替上层裁决这件事。
//!
//! ## 5. 与 `math::fft` 的分工
//!
//! [`crate::math::fft`] 是 `f64`、为**非音频速率**的波表分析服务的（`math.rs:130`–
//! `:134` 的模块注释明说"它不在渲染循环里"）。本模块的变换跑在渲染循环里，因此
//! 自带一个 `f32` 基 2 变换，且旋转因子在构造期预计算（块内零三角运算）。两份变换
//! 的**语义**相同（原地、位反转、无分配），区别只在元素类型与因子表的来源。
//!
//! ## 6. 输入取值域
//!
//! 本器件是**线性**系统：非有限输入（`NaN`/`±inf`）会产生非有限输出，并经频域延迟线
//! 污染后续若干个块。逐样本净化不在本器件的职责内（`delay`/`comb`/`reverb` 同样不做），
//! 净化由调用方在进入器件之前完成。
//!
//! ⚠ 这条只管**运行期的输入样本**。**配置期交进来的 IR 是另一回事**：它是一份静态
//! 数据，校验在这里既便宜（每个分区扫 129×2 项）又是唯一的拦截点。实测（本机 aarch64，
//! 本票读数）：IR 里放**一个** `NaN`（或 `±inf`）后，输出 **128/128** 个样本非有限，
//! 且 `reset()` 之后仍是 **128/128** —— 因为 `reset()` 只清频域延迟线与 OLA 尾，不清
//! `ir_*`。所以校验放在 [`Convolution::set_impulse_response`] 里，见该方法的说明。
//!
//! ## 7. 到"对标 ReaVerb"的距离（未实现清单）
//!
//! 本模块交付的是卷积**核**，不是一台完整的卷积混响。明确**没有**做的：
//!
//! 1. **真立体声 / 四通路卷积**：~~本模块**不**规定那套装配~~ ⇒ **已交付**，装配在
//!    [`crate::convolution_stereo`]（四条卷积，`h_LL`/`h_LR`/`h_RL`/`h_RR`）。
//!    本模块仍是单声道核，这一点不变；
//! 2. **IR 载入与预处理**：文件 I/O、采样率换算、首波对齐、长度归一化、淡出 ——
//!    全部属于上层（`yeban-dsp` 无 I/O，这是本 crate 的物理边界）。本器件只接受
//!    已经就绪的 `&[f32]`。⚠ 例外只有**验收**一处：非有限（或频谱溢出）的 IR 在本
//!    模块被**拒绝**（见 [`Convolution::set_impulse_response`]），因为那是唯一能挡住
//!    "湿路永久变成非有限值"的地方；"载入"与"预处理"仍然全在上层；
//! 3. **非均匀分区**：长 IR 用更大的尾部分区可以把成本再降一档；本器件是严格均匀的；
//! 4. **IR 的增益/湿干混合/预延迟**：~~由调用方（或 `reverb` 那类外壳）承担~~ ⇒
//!    **已交付**，那个外壳是 [`crate::convolution_reverb`]（湿路预延迟 ＋ 独立湿/干
//!    电平 ＋ IR 增益）。本模块仍是"只做卷积、不碰干信号"的核，这一点不变。
//!
//! ## 8. 实测代价
//!
//! **量什么**：一个 128 帧块的单次 [`Convolution::process`] 墙钟耗时（单位
//! **纳秒/块**，换算成 **微秒/量子**；1 量子 = 128 帧）。**怎么量**（Apple M2，
//! 最终源码的 release 档）：`std::time::Instant` 包住 20 000 次调用取均值，
//! `std::hint::black_box` 防止整段被优化掉，先预热 1 000 块；IR 是 48 kHz 下
//! 指数衰减的伪随机序列。预算取 **2666.667 µs / 128 帧立体声量子**
//! （算术：`128 / 48 000 Hz = 2.66667 ms`，即 375 量子/秒）。**最终源码上两次独立
//! 运行**的逐行差异 ≤ 2 %（1 秒立体声两次读数：`39.268 / 38.569` µs；10 秒立体声：
//! `311.716 / 313.019` µs）；下表是第二次。
//!
//! | IR 长度 | 分区数 | 单声道 µs/量子 | 立体声（2 核）µs/量子 | 立体声占预算 |
//! | ---: | ---: | ---: | ---: | ---: |
//! | 0.10 s（4 800 帧） | 38 | 5.318 | 11.096 | 0.416 % |
//! | 0.50 s（24 000 帧） | 188 | 11.681 | 23.296 | 0.874 % |
//! | **1.00 s（48 000 帧）** | **375** | **19.341** | **38.569** | **1.446 %** |
//! | 2.00 s（96 000 帧） | 750 | 35.793 | 68.934 | 2.585 % |
//! | 10.0 s（480 000 帧） | 3 750 | 156.182 | 313.019 | 11.738 % |
//!
//! 解析算术（1 秒 IR、单声道、每块）：分区乘加 `375 × 129 × 8 = 387 000` flops；
//! 两次 256 点变换 `2 × 5·256·log₂256 = 20 480` flops；合计 `407 480` flops/块
//! = `3 183` flops/帧。实测 `19.341 µs` ⇒ 等效标量吞吐
//! `407 480 flops / 19.341 µs ≈ 21.1` Gflop/s（立体声读数
//! `814 960 flops / 38.569 µs ≈ 21.1` Gflop/s，两者一致）。
//!
//! 内存算术（每声道）：4 条 `P × CONV_BINS` 的 `f32` 数组
//! `4 × 375 × 129 × 4 B = 774 000 B`，加工作缓冲 `2 × 256 × 4 = 2 048 B`、
//! OLA 尾 `128 × 4 = 512 B`、旋转因子 `2 × 128 × 4 = 1 024 B`，合计 `777 584 B`
//! = **759.4 KiB/声道**（立体声 `1 555 168 B` = 1.483 MiB）。
//!
//! 结论：**代价不是本器件的门槛**。1 秒 IR 的立体声混响占本机量子预算的
//! **1.45 %**，10 秒 IR 也只有 **11.7 %**。需要裁决的是 §7 的接线口径（IR 载入走
//! 哪条线程、真立体声要几条通路），不是 CPU。

/// 处理块长（帧）。与引擎的固定量子同值 [ARCH-DET-001]，因此逐块接口可以直通引擎。
pub const CONV_BLOCK_FRAMES: usize = 128;

/// FFT 长度。两个 128 点序列的线性卷积长 `2·128 − 1 = 255`，故取 256。
pub const CONV_FFT_FRAMES: usize = 2 * CONV_BLOCK_FRAMES;

/// 实输入一侧谱的 bin 数（含 DC 与 Nyquist）：`256 / 2 + 1 = 129`。
///
/// 只用一侧谱是**无损**的：输入与 IR 都是实序列，其频谱共轭对称，因此
/// `129` 个复数携带全部信息，乘加成本与内存都减半。
pub const CONV_BINS: usize = CONV_FFT_FRAMES / 2 + 1;

/// 接受的脉冲响应长度上限（帧）。`480_000` 帧 @48 kHz = **10 秒**。
///
/// 超过此长度的 IR 会被**截断**到 10 秒，[`Convolution::set_impulse_response`]
/// 的返回值报出真正接受的帧数。设上限是为了让"载入一个误选的超长文件"有一个
/// 有界的、可预测的后果。
pub const CONV_MAX_IR_FRAMES: usize = 48_000 * 10;

/// 本器件引入的处理延迟（帧）。UPOLA 的输出块与输入块对齐，因此是 **0**。
///
/// [ARCH-PDC-001] 要求"每个插件与内置设备必须精确上报其引入的处理延迟"：这里
/// 上报 0，且有一条判据锁住它（脉冲的第 0 帧立即产出 IR 的第 0 个抽头）。
pub const CONV_LATENCY: usize = 0;

/// 原地基 2 复数变换（`f32`），旋转因子由调用方预计算。
///
/// 与 [`crate::math::fft`] 的差别只有元素类型与因子表来源，语义相同：原地、位反转、
/// 正向不缩放、逆向乘 `1/n`。长度不是 2 的幂或长度不匹配时直接返回（不 panic、不分配）。
fn fft_radix2(re: &mut [f32], im: &mut [f32], tw_re: &[f32], tw_im: &[f32], inverse: bool) {
    let n = re.len();
    if im.len() != n || tw_re.len() != n / 2 || tw_im.len() != n / 2 || n < 2 {
        return;
    }

    // 位反转置换。
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    // 蝶形：每级跨度翻倍。第 k 个旋转因子是 `exp(−i·2πk/span)`，在长度 `n` 的表里
    // 的下标是 `k · (n / span)`；逆向只需取共轭（虚部取反）。
    let mut half = 1usize;
    while half < n {
        let span = half * 2;
        let stride = n / span;
        let mut base = 0usize;
        while base < n {
            for k in 0..half {
                let table = k * stride;
                let wr = tw_re[table];
                let wi = if inverse { -tw_im[table] } else { tw_im[table] };
                let (ar, ai) = (re[base + k], im[base + k]);
                let (br, bi) = (re[base + k + half], im[base + k + half]);
                let (vr, vi) = (br * wr - bi * wi, br * wi + bi * wr);
                re[base + k] = ar + vr;
                im[base + k] = ai + vi;
                re[base + k + half] = ar - vr;
                im[base + k + half] = ai - vi;
            }
            base += span;
        }
        half = span;
    }

    if inverse {
        let scale = 1.0 / n as f32;
        for value in re.iter_mut() {
            *value *= scale;
        }
        for value in im.iter_mut() {
            *value *= scale;
        }
    }
}

/// 单声道的均匀分块卷积器。
///
/// 缓冲区在 [`Self::set_impulse_response`] 里按 IR 长度**精确**分配一次；此后
/// [`Self::process`] 与 [`Self::reset`] 不再分配 [ARCH-RT-001]。
pub struct Convolution {
    /// 预计算的 IR 分区频谱，`partitions · CONV_BINS` 个 `f32`（实部）。
    ir_re: Vec<f32>,
    /// 同上（虚部）。
    ir_im: Vec<f32>,
    /// 输入块频谱的环形历史（频域延迟线），布局同 `ir_re`。
    hist_re: Vec<f32>,
    /// 同上（虚部）。
    hist_im: Vec<f32>,
    /// `CONV_FFT_FRAMES` 长的实部工作缓冲：输入补零 → 频谱 → 累加 → 逆变换结果。
    work_re: Vec<f32>,
    /// 同上（虚部）。
    work_im: Vec<f32>,
    /// 旋转因子表，`CONV_FFT_FRAMES / 2` 项，构造期建好后只读。
    tw_re: Vec<f32>,
    /// 同上（虚部）。
    tw_im: Vec<f32>,
    /// 重叠相加尾，`CONV_BLOCK_FRAMES` 帧。
    ola: Vec<f32>,
    /// 频域延迟线的写头（下一个输入块写入的槽位）。
    head: usize,
    /// 活跃分区数（= 接受帧数除块长向上取整）。
    partitions: usize,
    /// 接受的 IR 帧数。
    ir_frames: usize,
    /// 是否已配置。未配置时 [`Self::process`] 是直通。
    configured: bool,
}

impl Convolution {
    /// 构造一个**未配置**的实例（直通）。全部缓冲区为空，`new` 不分配。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ir_re: Vec::new(),
            ir_im: Vec::new(),
            hist_re: Vec::new(),
            hist_im: Vec::new(),
            work_re: Vec::new(),
            work_im: Vec::new(),
            tw_re: Vec::new(),
            tw_im: Vec::new(),
            ola: Vec::new(),
            head: 0,
            partitions: 0,
            ir_frames: 0,
            configured: false,
        }
    }

    /// 设定脉冲响应。**这是本类型唯一的分配入口**，必须在音频回调之外调用。
    ///
    /// 返回**真正接受**的帧数：`min(ir.len(), CONV_MAX_IR_FRAMES)`。返回 `0`
    /// 有两条出路，都落到未配置的直通状态：空 IR，以及**校验不通过**（见下一段）。
    /// 两条出路都会清掉 IR 频谱与全部历史。
    ///
    /// **校验（配置期，唯一的拦截点）**：接受的 IR 必须满足"分区频谱逐项有限"。
    /// 非有限样本（`NaN` / `±inf`）会把每一个 bin 污染成非有限值；**有限但过大**的
    /// 样本同样会（例如 512 帧的 `3e38`，求和即溢出）。这类 IR 一旦被接受就会把
    /// `ir_*` 永久写成非有限值，而 [`Self::reset`] **清不掉它**（它只清频域延迟线与
    /// OLA 尾）⇒ 湿路从此恒为非有限值。因此这里拒绝，而不是让它进去。
    ///
    /// 重复调用会重算 IR 频谱并清空频域延迟线与重叠相加尾，因此上一个 IR 的尾巴
    /// **不会**残留。**长度不变**时缓冲区原地复用（此时零分配）；长度改变时按新
    /// 长度重新分配。判据：`tests/convolution_rt_zero_alloc.rs` 的两条
    /// "同长度换 IR 零分配 / 改长度换 IR 会分配"，以及本文件的
    /// `non_finite_impulse_responses_are_rejected`。
    pub fn set_impulse_response(&mut self, ir: &[f32]) -> usize {
        let frames = ir.len().min(CONV_MAX_IR_FRAMES);
        if frames == 0 {
            return self.reject_impulse_response();
        }

        let partitions = frames.div_ceil(CONV_BLOCK_FRAMES);
        let span = partitions * CONV_BINS;
        // 这里只管**容量**：长度没变就一个字节都不动（于是同长度换 IR 零分配）。
        // 清零**只有一处** —— 方法末尾的 `reset()`。两个站点分开，是为了让
        // "残留上一个 IR 的尾巴"这条判据的注入是**单点删除**（删 `reset()`）。
        // `ir_*` 不需要清零：下面 `0..partitions` 的循环把 `[0, span)` 全部覆盖。
        if self.ir_re.len() != span {
            self.ir_re = vec![0.0; span];
            self.ir_im = vec![0.0; span];
        }
        if self.hist_re.len() != span {
            self.hist_re = vec![0.0; span];
            self.hist_im = vec![0.0; span];
        }
        if self.work_re.len() != CONV_FFT_FRAMES {
            self.work_re = vec![0.0; CONV_FFT_FRAMES];
            self.work_im = vec![0.0; CONV_FFT_FRAMES];
        }
        if self.ola.len() != CONV_BLOCK_FRAMES {
            self.ola = vec![0.0; CONV_BLOCK_FRAMES];
        }
        if self.tw_re.len() != CONV_FFT_FRAMES / 2 {
            let half = CONV_FFT_FRAMES / 2;
            self.tw_re = Vec::with_capacity(half);
            self.tw_im = Vec::with_capacity(half);
            for k in 0..half {
                let angle = -core::f32::consts::TAU * k as f32 / CONV_FFT_FRAMES as f32;
                self.tw_re.push(angle.cos());
                self.tw_im.push(angle.sin());
            }
        }

        // 逐分区：时域块补零到 256 → 正向变换 → 取一侧谱存进 `ir_*`。
        for p in 0..partitions {
            let start = p * CONV_BLOCK_FRAMES;
            let end = (start + CONV_BLOCK_FRAMES).min(frames);
            self.work_re.fill(0.0);
            self.work_im.fill(0.0);
            self.work_re[..end - start].copy_from_slice(&ir[start..end]);
            fft_radix2(
                &mut self.work_re,
                &mut self.work_im,
                &self.tw_re,
                &self.tw_im,
                false,
            );
            let slot = p * CONV_BINS;
            self.ir_re[slot..slot + CONV_BINS].copy_from_slice(&self.work_re[..CONV_BINS]);
            self.ir_im[slot..slot + CONV_BINS].copy_from_slice(&self.work_im[..CONV_BINS]);
        }

        // 校验：存下来的 IR 频谱必须逐项有限。放在**写入之后、置 configured 之前** ——
        // 这是"能被接受"的唯一入口。一次扫描，不分配。
        if !self.ir_re[..span].iter().all(|v| v.is_finite())
            || !self.ir_im[..span].iter().all(|v| v.is_finite())
        {
            return self.reject_impulse_response();
        }

        self.partitions = partitions;
        self.ir_frames = frames;
        self.configured = true;
        // 清零的**唯一**站点：频域延迟线、OLA 尾、写头都在这里回到"刚建好"。
        self.reset();
        frames
    }

    /// 是否已经过 [`Self::set_impulse_response`]（且 IR 非空）。
    #[must_use]
    pub const fn is_configured(&self) -> bool {
        self.configured
    }

    /// 接受的 IR 帧数（`0` = 未配置）。
    #[must_use]
    pub const fn ir_frames(&self) -> usize {
        self.ir_frames
    }

    /// 活跃分区数（`0` = 未配置）。
    #[must_use]
    pub const fn partitions(&self) -> usize {
        self.partitions
    }

    /// 本器件引入的处理延迟（帧），恒为 [`CONV_LATENCY`] = 0 [ARCH-PDC-001]。
    #[must_use]
    pub const fn latency_samples(&self) -> usize {
        CONV_LATENCY
    }

    /// 清空频域延迟线与重叠相加尾（零分配）。下一个块与刚配置完时逐位相同。
    pub fn reset(&mut self) {
        self.hist_re.fill(0.0);
        self.hist_im.fill(0.0);
        self.ola.fill(0.0);
        self.head = 0;
    }

    /// 拒绝一次配置：清掉 IR 频谱与全部历史，置回**未配置的直通**状态，返回 `0`。
    ///
    /// 清 `ir_*` 不是装饰。`process` 的守卫（`!configured || partitions == 0`）已经
    /// 挡住被拒绝的 IR，但把非有限频谱留在缓冲区里，等于让"未配置"只靠一个标志位
    /// 成立 —— 与 [`crate::convolution_stereo::TrueStereoConvolution`] 的拒绝路径
    /// 同一条纪律。全程零分配（`fill` 与 [`Self::reset`] 都是原地清零）。
    fn reject_impulse_response(&mut self) -> usize {
        self.configured = false;
        self.partitions = 0;
        self.ir_frames = 0;
        self.ir_re.fill(0.0);
        self.ir_im.fill(0.0);
        self.reset();
        0
    }

    /// 原地处理一个块，返回处理的帧数。
    ///
    /// 语义：
    ///
    /// 1. 未配置时**直通**（逐位不变）；
    /// 2. 每次调用把一个 [`CONV_BLOCK_FRAMES`] 帧的块推进频域延迟线。`block` 短于
    ///    该长度时按**零补齐**（只有前 `block.len()` 帧写回），且历史照常按整块前进；
    ///    长于该长度时只处理前 `CONV_BLOCK_FRAMES` 帧；
    /// 3. 全程零分配、零锁、零 I/O [ARCH-RT-001]。
    pub fn process(&mut self, block: &mut [f32]) -> usize {
        let n = block.len().min(CONV_BLOCK_FRAMES);
        if !self.configured || self.partitions == 0 {
            return n;
        }

        // 输入补零到 CONV_FFT_FRAMES，正向变换，写进频域延迟线。
        self.work_re.fill(0.0);
        self.work_im.fill(0.0);
        self.work_re[..n].copy_from_slice(&block[..n]);
        fft_radix2(
            &mut self.work_re,
            &mut self.work_im,
            &self.tw_re,
            &self.tw_im,
            false,
        );
        let slot = self.head;
        let base = slot * CONV_BINS;
        self.hist_re[base..base + CONV_BINS].copy_from_slice(&self.work_re[..CONV_BINS]);
        self.hist_im[base..base + CONV_BINS].copy_from_slice(&self.work_im[..CONV_BINS]);

        // Y = Σ_p H_p · X_{m−p}（一侧谱）。`p = 0` 是刚写进去的那个块。
        self.work_re[..CONV_BINS].fill(0.0);
        self.work_im[..CONV_BINS].fill(0.0);
        for p in 0..self.partitions {
            let past = (slot + self.partitions - p) % self.partitions;
            let h = p * CONV_BINS;
            let x = past * CONV_BINS;
            for k in 0..CONV_BINS {
                let hr = self.ir_re[h + k];
                let hi = self.ir_im[h + k];
                let xr = self.hist_re[x + k];
                let xi = self.hist_im[x + k];
                self.work_re[k] += hr * xr - hi * xi;
                self.work_im[k] += hr * xi + hi * xr;
            }
        }

        // 一侧谱 → 全谱（共轭对称）。高 bin 写在低 bin 之外，故可原地展开。
        for k in CONV_BINS..CONV_FFT_FRAMES {
            let mirror = CONV_FFT_FRAMES - k;
            self.work_re[k] = self.work_re[mirror];
            self.work_im[k] = -self.work_im[mirror];
        }

        fft_radix2(
            &mut self.work_re,
            &mut self.work_im,
            &self.tw_re,
            &self.tw_im,
            true,
        );

        // 前 B 帧 + 上一块的 OLA 尾 ⇒ 本块输出；本块的后 B 帧成为新的尾。
        for (i, out) in block[..n].iter_mut().enumerate() {
            *out = self.work_re[i] + self.ola[i];
        }
        self.ola
            .copy_from_slice(&self.work_re[CONV_BLOCK_FRAMES..CONV_FFT_FRAMES]);
        self.head = (slot + 1) % self.partitions;
        n
    }
}

impl Default for Convolution {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for Convolution {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // 手写而不是 derive：内部的 `Vec` 有上万个元素，派生的 Debug 会把它们全部打印。
        f.debug_struct("Convolution")
            .field("configured", &self.configured)
            .field("ir_frames", &self.ir_frames)
            .field("partitions", &self.partitions)
            .field("latency_samples", &CONV_LATENCY)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 直接时域卷积（参照实现，`O(n·m)`）。判据的**独立**参照：它不共用被测代码的
    /// 任何一行，因此能抓住分区索引、共轭展开、OLA 三处的错误。
    fn direct_convolution(input: &[f32], ir: &[f32]) -> Vec<f32> {
        let mut out = vec![0.0f32; input.len() + ir.len() - 1];
        for (i, &x) in input.iter().enumerate() {
            for (j, &h) in ir.iter().enumerate() {
                out[i + j] += x * h;
            }
        }
        out
    }

    /// 把 `input` 按 128 帧块喂进去，收集 `frames` 帧输出（`input` 之后按零补齐）。
    fn render(conv: &mut Convolution, input: &[f32], frames: usize) -> Vec<f32> {
        let blocks = frames.div_ceil(CONV_BLOCK_FRAMES);
        let mut out = vec![0.0f32; blocks * CONV_BLOCK_FRAMES];
        for (b, chunk) in out.chunks_mut(CONV_BLOCK_FRAMES).enumerate() {
            let start = b * CONV_BLOCK_FRAMES;
            let end = (start + CONV_BLOCK_FRAMES).min(input.len());
            if end > start {
                chunk[..end - start].copy_from_slice(&input[start..end]);
            }
            conv.process(chunk);
        }
        out
    }

    /// 确定性伪随机输入（本 crate 的 xorshift，显式播种）。
    fn pseudo_random(len: usize, seed: u32) -> Vec<f32> {
        let mut rng = crate::noise::Rng::new(seed);
        (0..len).map(|_| rng.next_bipolar()).collect()
    }

    /// **判据（可红）**：分块卷积必须与直接时域卷积逐样本一致。
    ///
    /// 观测方式：400 帧的 IR（**跨 4 个分区边界**，其中最后一个分区不满）与 1000 帧
    /// 伪随机输入，比较 1399 个输出样本。容差按参照峰值缩放（`f32` 的 256 点变换
    /// 舍入约 `1e-6` 相对量级）。
    ///
    /// 注入：把 `for p in 0..self.partitions` 里的 `(slot + self.partitions - p)`
    /// 改成 `slot`（只读最新块）⇒ 本判据立即变红。
    #[test]
    fn partitioned_convolution_matches_direct_time_domain_convolution() {
        let ir = pseudo_random(400, 0x5EED_1234);
        let input = pseudo_random(1_000, 0x0BAD_F00D);
        let mut conv = Convolution::new();
        assert_eq!(conv.set_impulse_response(&ir), 400);
        assert_eq!(conv.partitions(), 4, "400 帧 @128 ⇒ 4 个分区");

        let want = direct_convolution(&input, &ir);
        let got = render(&mut conv, &input, want.len());
        let scale = want.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-12);
        let mut worst = 0.0f32;
        for (i, &w) in want.iter().enumerate() {
            worst = worst.max((got[i] - w).abs());
        }
        assert!(
            worst <= scale * 1e-4,
            "分块卷积与时域参照不符：最大绝对误差 {worst}（参照峰值 {scale}，相对 {}）",
            worst / scale
        );
        // 尾巴之外必须是数值噪声级（否则 OLA 尾在泄漏）。门限与主比较同量级：
        // 真正的泄漏是 O(峰值) 的，不可能是 1e-4 相对量级。
        for (i, &v) in got[want.len()..].iter().enumerate() {
            assert!(
                v.abs() <= scale * 1e-4,
                "第 {} 帧（超出线性卷积长度）不是噪声级：{v}（参照峰值 {scale}）",
                want.len() + i
            );
        }
    }

    /// **判据（可红）**：IR 比一个分区短时（1 个分区）也必须一致 —— 这是最小配置，
    /// 单独一条是因为"只有 1 个分区"时环形索引的取模退化成常数，公式错了也可能碰巧对。
    #[test]
    fn a_single_partition_ir_matches_the_reference() {
        let ir = pseudo_random(37, 0x1111_2222);
        let input = pseudo_random(300, 0x3333_4444);
        let mut conv = Convolution::new();
        conv.set_impulse_response(&ir);
        assert_eq!(conv.partitions(), 1);
        let want = direct_convolution(&input, &ir);
        let got = render(&mut conv, &input, want.len());
        let scale = want.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-12);
        for (i, &w) in want.iter().enumerate() {
            assert!(
                (got[i] - w).abs() <= scale * 1e-4,
                "第 {i} 帧：{} vs 参照 {w}",
                got[i]
            );
        }
    }

    /// **判据（可红）**：延迟为 0 [ARCH-PDC-001]。
    ///
    /// 观测方式：脉冲输入的第 0 帧必须在**同一个块**的第 0 帧产出 `ir[0]`。
    /// 注入：把输出写成只用重叠相加尾 `*out = self.ola[i]`（丢掉本块的直接项）
    /// ⇒ 本判据变红（第 0 帧变成 0）。
    #[test]
    fn the_reported_latency_is_zero_and_the_first_tap_is_immediate() {
        let ir = vec![0.25f32, -0.5, 0.125, 0.0, 0.75];
        let mut conv = Convolution::new();
        conv.set_impulse_response(&ir);
        assert_eq!(conv.latency_samples(), CONV_LATENCY);
        assert_eq!(CONV_LATENCY, 0);
        let mut block = vec![0.0f32; CONV_BLOCK_FRAMES];
        block[0] = 1.0;
        conv.process(&mut block);
        assert!(
            (block[0] - ir[0]).abs() < 1e-6,
            "第 0 帧应为 ir[0]={}，实得 {}（延迟被引入了）",
            ir[0],
            block[0]
        );
        for (i, &h) in ir.iter().enumerate().skip(1) {
            assert!(
                (block[i] - h).abs() < 1e-6,
                "第 {i} 帧：{} vs {}",
                block[i],
                h
            );
        }
    }

    /// **判据（可红）**：换 IR 不残留上一个 IR 的尾巴。
    ///
    /// 观测方式：先用长 IR 灌入脉冲（把频域延迟线与 OLA 尾灌满能量），再换 IR，
    /// 然后喂**静音** —— 输出必须逐位为 0。两种换法都要测，因为它们走
    /// [`Convolution::set_impulse_response`] 的两条不同分支（原地复用 / 重新分配）：
    ///
    /// 1. **同长度**换（不重新分配 ⇒ 缓冲区里的旧内容**只能**靠 `reset()` 清掉）；
    /// 2. **变长度**换（延迟线被换短 ⇒ 写头也必须跟着归零）。
    ///
    /// 注入：删掉末尾的 `self.reset()` ⇒ 本判据变红，而且**两种情形都红**，只是
    /// 形态不同（实测）：情形 1 在断言处抓到残留尾巴（`[-2.19, -8.63, 6.91, …]`）；
    /// 把两种情形**对调顺序**后重测，情形 2 在 `process` 里**越界 panic**
    /// （`self.head` 仍是按旧 IR 算出的值，而延迟线已被换短）。⇒ `reset()` 同时
    /// 承担"清内容"与"归零写头"两件事，两者都不可省。
    #[test]
    fn reconfiguring_does_not_leak_the_previous_ir_tail() {
        let first = pseudo_random(1_000, 0xABCD_0001);
        let second = pseudo_random(1_000, 0xABCD_0002); // 同长度，不同内容
        let short = vec![1.0f32];

        // 先把延迟线灌满：脉冲 × 2 块（第二块把 OLA 尾也走一遍）。
        let fill = |conv: &mut Convolution| {
            let mut impulse = vec![0.0f32; CONV_BLOCK_FRAMES];
            impulse[0] = 1.0;
            conv.process(&mut impulse);
            conv.process(&mut impulse);
        };

        // 情形 1：同长度换 IR。
        let mut conv = Convolution::new();
        conv.set_impulse_response(&first);
        fill(&mut conv);
        conv.set_impulse_response(&second);
        let mut silence = vec![0.0f32; CONV_BLOCK_FRAMES];
        conv.process(&mut silence);
        assert!(
            silence.iter().all(|v| *v == 0.0),
            "同长度换 IR 后静音输入有输出（上一个 IR 的尾巴泄漏了）：{:?}",
            &silence[..8]
        );

        // 情形 2：变长度换 IR。
        let mut conv = Convolution::new();
        conv.set_impulse_response(&first);
        fill(&mut conv);
        conv.set_impulse_response(&short);
        let mut silence = vec![0.0f32; CONV_BLOCK_FRAMES];
        conv.process(&mut silence);
        assert!(
            silence.iter().all(|v| *v == 0.0),
            "变长度换 IR 后静音输入有输出（上一个 IR 的尾巴泄漏了）：{:?}",
            &silence[..8]
        );
    }

    /// **判据（可红）**：未配置（含空 IR）是**逐位**直通，且不得 panic。
    ///
    /// 注入：删掉 `process` 开头的 `!self.configured || self.partitions == 0` 守卫
    /// ⇒ 空 `Vec` 索引 panic 变红。
    #[test]
    fn an_unconfigured_convolution_is_a_bit_exact_passthrough() {
        let mut conv = Convolution::new();
        assert!(!conv.is_configured());
        assert_eq!(conv.ir_frames(), 0);
        assert_eq!(conv.latency_samples(), 0);
        let mut block = [0.5f32, -0.25, 0.125];
        assert_eq!(conv.process(&mut block), 3);
        assert_eq!(block, [0.5, -0.25, 0.125]);

        // 空 IR 与 `reset()` 到未配置状态都要直通。
        conv.set_impulse_response(&[]);
        assert!(!conv.is_configured());
        let mut block = [1.0f32, 2.0];
        conv.process(&mut block);
        assert_eq!(block, [1.0, 2.0]);

        // 已配置实例：静音输入（含全零块）也必须有界、有限。
        conv.set_impulse_response(&pseudo_random(200, 0x7777));
        let mut block = vec![0.0f32; 8];
        assert_eq!(conv.process(&mut block), 8, "短块只处理其自身长度");
        assert!(block.iter().all(|v| v.is_finite()));
    }

    /// **判据（可红）**：非有限（或频谱溢出）的 IR 被**拒绝**，实例回到未配置的直通。
    ///
    /// 观测方式：四种坏 IR —— 一个 `NaN`、一个 `+inf`、一个 `-inf`、以及**有限但溢出**
    /// 的 `3e38 × 512` —— 各自配置一次，检查返回值 `0`、`is_configured()` 假、
    /// `partitions()` 与 `ir_frames()` 都是 `0`，且随后一个块的输出**逐位**等于输入。
    ///
    /// 量什么：`set_impulse_response` 的返回值（单位：帧）与一个 128 帧块的每一个比特。
    ///
    /// 注入（实测见本票报告）：删掉 `set_impulse_response` 里的频谱有限性校验
    /// ⇒ 本判据在第一类（`NaN`）的返回值断言处变红（实得 `512`，期望 `0`）。
    #[test]
    fn non_finite_impulse_responses_are_rejected() {
        let good = pseudo_random(512, 0x0FF1_CE01);
        let mut cases: Vec<(&str, Vec<f32>)> = Vec::new();
        for (label, bad) in [
            ("NaN", f32::NAN),
            ("+inf", f32::INFINITY),
            ("-inf", f32::NEG_INFINITY),
        ] {
            let mut ir = good.clone();
            ir[7] = bad;
            cases.push((label, ir));
        }
        cases.push(("有限但溢出（3e38 × 512）", vec![3.0e38f32; 512]));

        for (label, ir) in cases {
            let mut conv = Convolution::new();
            assert_eq!(
                conv.set_impulse_response(&ir),
                0,
                "{label}：坏 IR 必须被拒绝（返回 0）"
            );
            assert!(!conv.is_configured(), "{label}：拒绝后必须是未配置");
            assert_eq!(conv.partitions(), 0, "{label}：拒绝后不得留下分区");
            assert_eq!(conv.ir_frames(), 0, "{label}：拒绝后不得留下帧数");

            let input = pseudo_random(CONV_BLOCK_FRAMES, 0x00C0_FFEE);
            let mut block = input.clone();
            assert_eq!(conv.process(&mut block), CONV_BLOCK_FRAMES);
            for (i, (out, want)) in block.iter().zip(&input).enumerate() {
                assert_eq!(
                    out.to_bits(),
                    want.to_bits(),
                    "{label}：拒绝后的样本 {i} 必须逐位直通"
                );
            }
        }
    }

    /// **判据（可红）**：一次坏 IR 不能把**已经跑起来**的实例永久毒化。
    ///
    /// 观测方式：先用好 IR 灌满频域延迟线（湿路有尾巴），再交一条**同长度**、含 `NaN`
    /// 的 IR，然后喂一个非静音块：输出必须**逐位**等于输入（未配置直通），而不是 `NaN`。
    /// 这条正是"`reset()` 清不掉 `ir_*`"那个失效模式的判据：它只有在拒绝路径同时清掉
    /// `ir_*` 与历史、并真的把 `configured` 置回假时才成立。
    ///
    /// 量什么：一次坏 IR 换入后，一个 128 帧块的每一个比特与非有限样本个数。
    ///
    /// 注入（实测见本票报告）：删掉频谱有限性校验（即恢复旧行为"照单全收"）
    /// ⇒ 本判据在**第一条**断言处变红（实得 `1000`，期望 `0`）；`is_configured()`
    /// 那条把守的是"拒绝必须把标志位也置回去"（另一处注入，见报告）。
    #[test]
    fn a_rejected_ir_cannot_poison_a_running_instance() {
        let good = pseudo_random(1_000, 0xDEAD_BEEF);
        let mut conv = Convolution::new();
        assert_eq!(conv.set_impulse_response(&good), 1_000);
        let mut impulse = vec![0.0f32; CONV_BLOCK_FRAMES];
        impulse[0] = 1.0;
        conv.process(&mut impulse);
        assert!(conv.is_configured());

        let mut bad = good.clone();
        bad[9] = f32::NAN;
        assert_eq!(
            conv.set_impulse_response(&bad),
            0,
            "含 NaN 的 IR 必须被拒绝"
        );
        assert!(!conv.is_configured(), "拒绝后必须回到未配置");

        let input = pseudo_random(CONV_BLOCK_FRAMES, 0x5A5A_1234);
        let mut block = input.clone();
        conv.process(&mut block);
        let non_finite = block.iter().filter(|v| !v.is_finite()).count();
        assert_eq!(non_finite, 0, "拒绝后的路径仍产出非有限值");
        for (i, (out, want)) in block.iter().zip(&input).enumerate() {
            assert_eq!(out.to_bits(), want.to_bits(), "样本 {i} 未逐位直通");
        }
    }

    /// **判据（可红）**：确定性 —— 同样输入两次逐位相同。
    #[test]
    fn the_same_input_twice_is_bit_identical() {
        let ir = pseudo_random(777, 0x2468_ACE0);
        let input = pseudo_random(2_000, 0x1357_9BDF);
        let mut a = Convolution::new();
        let mut b = Convolution::new();
        a.set_impulse_response(&ir);
        b.set_impulse_response(&ir);
        let ra = render(&mut a, &input, input.len());
        let rb = render(&mut b, &input, input.len());
        assert!(
            ra.iter().zip(&rb).all(|(x, y)| x.to_bits() == y.to_bits()),
            "两次渲染不是逐位相同"
        );
        // `reset()` 必须回到"刚配置完"的状态。
        a.reset();
        let ra2 = render(&mut a, &input, input.len());
        assert!(
            ra.iter().zip(&ra2).all(|(x, y)| x.to_bits() == y.to_bits()),
            "reset() 之后的重渲染不是逐位相同"
        );
    }

    /// **判据（可红）**：超长 IR 被截断到 [`CONV_MAX_IR_FRAMES`]，返回值报出真值。
    ///
    /// 注入：把 `ir.len().min(CONV_MAX_IR_FRAMES)` 改成 `ir.len()` ⇒ 本判据变红。
    #[test]
    fn an_over_long_ir_is_truncated_at_the_documented_cap() {
        let long = pseudo_random(CONV_MAX_IR_FRAMES + 1, 0x9999);
        let mut conv = Convolution::new();
        let accepted = conv.set_impulse_response(&long);
        assert_eq!(accepted, CONV_MAX_IR_FRAMES);
        assert_eq!(conv.ir_frames(), CONV_MAX_IR_FRAMES);
        assert_eq!(
            conv.partitions(),
            CONV_MAX_IR_FRAMES / CONV_BLOCK_FRAMES,
            "10 秒 @128 帧是整除的"
        );
    }

    /// **判据**：非 2 的幂 / 长度不匹配的变换输入不得 panic（内部守卫）。
    #[test]
    fn the_inner_transform_tolerates_degenerate_inputs() {
        let mut re = [0.0f32; 8];
        let mut im = [0.0f32; 8];
        // 长度不是 2 的幂：直接返回。
        fft_radix2(&mut re, &mut im, &[], &[], false);
        assert!(re.iter().all(|v| *v == 0.0));
        // 因子表长度不对：直接返回。
        let mut re = [1.0f32, 0.0];
        let mut im = [0.0f32, 0.0];
        fft_radix2(&mut re, &mut im, &[1.0], &[0.0], false);
        assert_eq!(re[0], 1.0);
        // 逆向变换的往返（2 点，手算的因子表）。
        let tw_re = [1.0f32];
        let tw_im = [0.0f32];
        let mut re = [3.0f32, 1.0];
        let mut im = [0.0f32, 0.0];
        fft_radix2(&mut re, &mut im, &tw_re, &tw_im, false);
        assert!((re[0] - 4.0).abs() < 1e-6 && (re[1] - 2.0).abs() < 1e-6);
        fft_radix2(&mut re, &mut im, &tw_re, &tw_im, true);
        assert!((re[0] - 3.0).abs() < 1e-6 && (re[1] - 1.0).abs() < 1e-6);
    }
}
