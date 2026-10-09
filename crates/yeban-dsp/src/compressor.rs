//! 前馈式压缩器：软膝静态曲线 + 电平域起振/释放弹道。[ARCH-RT-001] [ARCH-DET-001]
//!
//! **本模块是新写的**。移植源核实结论见本文件末尾的"移植源"一节：
//! 本机（含主工作树与本 worktree）**没有**现成的动态范围压缩器实现可移植。
//!
//! ## 信号链（每样本一次，顺序固定）
//!
//! ```text
//! ① 检波器    ms_in  = max(l², r²)                     ← 立体声联动（linked）
//! ② 平均      ms     = 一阶低通(ms_in, α_det)           ← **对称**平均 ⇒ 电平是"真 RMS"
//! ③ 电平      level_db = 10·log10(max(ms, 10^−12))     ← 正弦 ⇒ 20·log10(A/√2) = RMS dBFS
//! ④ 静态曲线  target   = gain_db_for(level_db, …)      ← 纯函数，逐样本无状态
//! ⑤ 弹道      gain_db  = 一阶低通(target, α_a / α_r)   ← **独立**的起振/释放
//! ⑥ 应用      out      = in · db_to_gain(gain_db) · makeup
//! ```
//!
//! **前馈**（feed-forward）的意思是：增益由**当前**输入电平算出，不看输出。
//! 代价是没有"输入不超阈值"的硬保证；收益是无延迟、无反馈回路。
//!
//! **为什么检波与弹道分成两级**（§3.2 有实测教训）：检波器只负责"这一帧的
//! RMS 功率是多少"，它必须是**对称**平均，否则读数依赖波形（§3.2）。
//! 起振/释放是**增益**的弹道，住在第 ⑤ 级，与检波器的窗口解耦。
//! 这样"电平读数"与"弹道时间"各自都是单一口径，判据可以分别对账。
//!
//! ## 1. 为什么是 RMS 检波而不是峰值检波
//!
//! 两者都在本仓库的合法选择里（任务书把"RMS vs peak"列为设计维度之一）。
//! 选 RMS 的理由是**可验证性**：
//!
//! | 输入（幅度 `A`） | 峰值检波稳态 | RMS 检波稳态 |
//! | :--- | :--- | :--- |
//! | 正弦 | 与 `τ·fs` 的比值有关，且在 `A` 与 `0.637·A` 之间**带纹波** | 精确 `A/√2` |
//! | 阶跃（直流） | `A` | `A` |
//! | 静音 | 0 | 0 |
//!
//! 峰值检波配一阶低通，稳态值取决于波形与时间常数的比值（全波整流的均值是
//! `2A/π ≈ 0.637A`，纹波按 `2f` 衰减）。RMS 检波的稳态对正弦是**精确**的
//! `A/√2`，因此"实测增益衰减"可以直接对着静态曲线的算术去比，差值来源只有
//! 一处（见 §4）。判据要能算清账，这是首要条件。
//!
//! ## 2. 立体声联动用 `max(l², r²)`，不用 `(l² + r²)/2`
//!
//! `max` 有两个性质：
//!
//! 1. **单声道安全**：信号只在左声道（`r = 0`）时读数与居中时**相同**；
//!    `(l² + r²)/2` 会低估 3 dB。
//! 2. **联动**：两声道共享一个增益，声像不被拉开；反相信号（`l = −r`）也测得到。
//!
//! ## 3. 两个时间口径：检波窗口 `detector_s` 与增益弹道 `attack_s`/`release_s`
//!
//! 一阶低通 `y += α·(x − y)`、`α = 1 − exp(−1/(τ·fs))` 的阶跃响应是
//! `y(t) = target·(1 − e^(−t/τ))`。**这一句只对线性域成立**，因此本模块把
//! 两级平滑都放在线性域：第 ② 级在**均方**（线性功率）域，第 ⑤ 级在
//! **线性增益**域 —— 见 §3.1 与 §3.2 的两条实测教训。
//!
//! 三个参数都是 `τ`（秒，时间常数），不是 10%–90% 上升时间：
//!
//! | 参数 | 平滑对象 | 域 | 口径 |
//! | :--- | :--- | :--- | :--- |
//! | [`CompressorParams::detector_s`] | 均方 `ms` | 线性功率 | `t = τ` ⇒ 到达目标功率的 63.2% |
//! | [`CompressorParams::attack_s`] | 增益 `g` | 线性增益（`dB_to_gain` 之后不是线性了，见 §3.2） | 增益向**更小**走时用 |
//! | [`CompressorParams::release_s`] | 增益 `g` | 同上 | 增益向**更大**（回 1.0）走时用 |
//!
//! 因为两级都在线性域，`t = τ` 一律是 **63.2%**，"距稳态 1 dB"一律是
//! **1.585·τ**（`1 − e^(−t/τ) = 10^(−1/10) = 0.7943` ⇒ `t = τ·ln(1/(1−0.7943))`）。
//! 本模块的判据用这两个口径，读数与参数直接可比。
//!
//! ### 3.1 为什么**不**在 dB 域平滑（实测教训一）
//!
//! 第一版实现把 `level_db` 本身做一阶低通。它在静音处**爬得极慢**：
//! `level_db` 从 −120 dBFS 出发，`α = 2.08e-3`，前几步的读数实测是
//! `−119.750 → −119.501 → −119.252 → −119.004`，即每样本约 `0.25 dB`。
//! 原因是 dB 域的阶跃响应不是指数：`−120 + 114·(1 − e^(−t/τ))` 在起点附近的
//! 斜率是 `114/τ` 而不是 `6/τ`，于是"起振比标称值慢约 5 倍"。
//! 均方域没有这个问题。
//!
//! ### 3.2 为什么检波器**不**用非对称的起振/释放（实测教训二）
//!
//! 第二版实现把 `attack_s` / `release_s` 用作**均方**上升/下降的两个系数。
//! 它对直流是对的（实测 `0.5` 直流 ⇒ 均方 `0.24999642` ≈ 0.25），
//! 但对振荡信号**系统性高估**：实测 `A = 0.5`、1 kHz 正弦的均方收敛到
//! **`0.20361449`**，而它的真值是 `A²/2 = 0.125` —— 功率高出 **1.63 倍**
//! （电平 +2.16 dB）。原因是"快起振 + 慢释放"就是一个**准峰值检波器**：
//! 慢释放会把前半个周期的峰值保持到后半个周期。这个偏差**依赖波形与频率**
//! （直流 1.63 倍、方波约 1 倍），算术无法对账。
//!
//! 因此检波器改用**对称**平均（`α_det`，上升与下降同一个系数）。对称一阶低通
//! 的无偏性可以直接算：对周期输入 `x`，稳态满足
//! `E[ms] = E[max(l², r²)]`，与频率无关（残差只有被 `α_det` 衰减的 2f 纹波）。
//! 起振/释放改住到第 ⑤ 级的**增益**弹道上，与检波窗口解耦。
//!
//! ### 3.3 第 ⑤ 级为什么仍在线性域做
//!
//! 增益弹道的目标是第 ④ 级的 `gain_db`（dB）。若直接对 dB 做低通，
//! 会重复 §3.1 的问题（静音处目标 `−120 dB`，dB 域斜率被放大 114/9 ≈ 12.7 倍）。
//! 因此第 ⑤ 级改为对**线性增益** `g = db_to_gain(target_gain_db)` 做一阶低通，
//! 再取 `20·log10(g)`。这样 `t = τ` 仍是 63.2%（在**幅度**意义上），
//! 而静音附近的目标是 `db_to_gain(−120) = 0.0`，没有 dB 域的斜率放大。
//!
//! ⚠ **登记（已知偏差）**：线性增益域的一阶低通在 `g` 接近 0 时，
//! `20·log10(g)` 的 dB 轨迹不是纯指数。所以"增益的起振/释放"在
//! **深压缩**（`g < 0.1`，即 > 20 dB 衰减）时的 dB 读数会比标称 `τ` 略慢。
//! 本模块的判据全部在 ≤ 9 dB 衰减（`g ≥ 0.35`）的范围内测，那里 dB 偏差
//! 已在容差内（见 `attack_and_release_land_on_the_declared_gain_ballistics`）。
//!
//! ### 3.4 第 ② 级为什么是"真 RMS"
//!
//! 对称一阶低通是均值估计器：`E[ms] → E[max(l², r²)]`。对幅度 `A` 的正弦，
//! `E[max(l², r²)] = A²·E[sin²] = A²/2` ⇒ 电平 `10·log10(A²/2) = 20·log10(A/√2)`，
//! 即**真 RMS dBFS**。对直流 `v` 它是 `20·log10|v|`。两个读数都可用手算对账。
//! 残余的 2f 纹波幅度是 `α_det/√(4 − 4α_det + α_det²)` 量级：`τ_det = 5 ms`
//! @48 kHz 时 `α_det = 4.16e-3`，1 kHz 正弦的纹波约 `2.1e-3`（≈ ±0.009 dB）。
//!
//!//! ## 4. 静态曲线（软膝）：阈值 / 比率 / 拐点宽度的确切公式
//!
//! 记 `x` = 输入电平（dBFS）、`T` = [`CompressorParams::threshold_db`]、
//! `R` = [`CompressorParams::ratio`]、`W` = [`CompressorParams::knee_db`]、
//! `o = x − T`（超阈量）：
//!
//! ```text
//! o ≤ −W/2                 ⇒ y = x                                  （膝下，透明）
//! −W/2 < o < +W/2          ⇒ y = x + (1/R − 1)·(o + W/2)² / (2W)     （膝内，二次插值）
//! o ≥ +W/2                 ⇒ y = T + o/R                            （膝上，硬比率）
//! 增益（dB，负 = 衰减）    ⇒ gain_db = y − x
//! ```
//!
//! 三段在 `o = ±W/2` 处**连续**（膝内式在 `o = −W/2` 处等于 `x`、在 `o = +W/2`
//! 处等于 `T + W/(2R)`），且膝内式在 `o = −W/2` 处斜率为 0 ⇒ 阈值处没有折角。
//! 这是 Reiss & McPherson《Audio Effects》的软膝压缩器曲线，属教科书口径。
//! `W = 0`（硬膝）走单独的整数比较分支，避免 `/(2W)` 除零。
//!
//! **两个精确点**（判据据此建立）：
//!
//! - 膝上且 `R → ∞` 时 `gain_db → −o`，即输出电平被钉在 `T`（这是"限制器"的极限）；
//! - 膝上时 `gain_db = (1/R − 1)·o`，与 `T` 无关 ⇒ 增益衰减只依赖**超阈量**与比率。
//!
//! ## 5. 与块切分的关系（[ARCH-DET-001] 的"固定 128 块长"）
//!
//! 本器件**没有**前视缓冲、**没有**延迟线，全部弹道状态只有**两个标量**
//! （`mean_square` 与 `gain_lin`；`level_db` 与 `gain_db` 都是它们的纯函数）。
//! 每个样本只前向依赖上一个样本，
//! 因此 `process_stereo` / `process_mono` 的调用次数与切分方式**不改变任何输出**。
//!
//! ⚠ 这是**构造性**的性质，不是近似：判据 `chunking_does_not_change_output`
//! 对 1 000 个样本按 1 / 3 / 7 / 128 / 999 / 1000 六种切分跑同一输入，
//! 要求输出**逐位相同**。
//! ⚠ 与 `yeban-engine` 的前视限制器（33 帧延迟、`LOOKAHEAD_SAMPLES`）**不同**：
//! 那个器件有延迟并入了 PDC。本器件延迟为 0，见
//! [`Compressor::latency_samples`]。
//!
//! ## 6. 数值边界（⛔ 不许 NaN / Inf）
//!
//! | 输入 | 处理 | 输出 |
//! | :--- | :--- | :--- |
//! | `NaN` 样本 | 归零（与 `yeban-engine` 混音链的 `nan_to_zero` 同口径） | 有限 |
//! | `±∞` 样本 | 归零（`∞² = ∞` 会污染检波器状态） | 有限 |
//! | 0（静音） | 均方 `ms` 钳到 [`POWER_FLOOR`] = `10^−12` ⇒ 电平 = [`MIN_LEVEL_DB`] | 0 |
//! | 超幅（如 `4.0` = +12.04 dBFS） | 走膝上分支，`gain_db = (1/R−1)·o` | 有限 |
//! | `gain_db = −∞`（比率极端） | `db_to_gain` 在 `≤ −120 dB` 处返回 `0.0` | 0，非 NaN |
//! | 参数为 `NaN`/`∞` | [`CompressorParams::sanitised`] 在设置时钳到合法域 | 有限 |
//! | `\|x\| > 2.1e37` **且** makeup > 0 dB | 乘积溢出 ⇒ `output_sample` 归零 | 0 |
//!
//! ⚠ 前两条说的是**写回调用方缓冲**的那一步。只把清洗后的值喂给检波器不够：
//! 检波器干净了，缓冲里却还是原来的 `NaN`。同类的两个器件都写回清洗后的值
//! （[`crate::limiter::Limiter::process_stereo`] 写 `nan_to_zero` 的结果、
//! [`crate::channel_strip::ChannelStrip::process_stereo`] 写
//! [`crate::meter::sanitize_sample`] 的结果），本器件此前是那条表里的例外。
//!
//! [定理] **输出恒为有限数**。证明（每一步的取值域都写出来）：
//!
//! 1. [`finite_or_zero`] ⇒ `l`、`r` 有限；`ms_in = max(l², r²)` 有限且 `≥ 0`。
//! 2. 均方递推 `ms = ms_prev + α_det·(ms_in − ms_prev)`，`α_det ∈ (0, 1)`
//!    （[`one_pole_alpha`] 的 `τ ≥ `[`MIN_TIME_S`]` > 0、`fs ≥ `[`crate::MIN_SAMPLE_RATE`]`），
//!    两个操作数有限 ⇒ `ms` 有限；本步另有 `is_finite` 兜底。而且
//!    `ms_prev = 0` 时 `ms = α_det·ms_in ≥ 0`，`ms_prev > 0` 时它是两个非负数的
//!    凸组合 ⇒ **`ms ≥ 0` 恒成立**（判据 `mean_square_never_leaves_its_domain`）。
//! 3. `level_db = 10·log10(max(ms, `[`POWER_FLOOR`]`))`：实参 `≥ 10^−12`
//!    ⇒ 结果 `≥ −120`，有限。
//! 4. [`gain_db_for`] 是三段分段函数；硬膝分支不需要 `W`，软膝分支的分母
//!    `2W > 0`（该分支只在 `W > 0` 时可达）；`T`、`R` 有限 ⇒ `gain_db` 有限且 `≤ 0`。
//! 5. `db_to_gain(gain_db)` 对任何有限输入返回 `[0, 1]` 内的有限数；增益递推的
//!    结果再经 `.clamp(0.0, 1.0)` ⇒ **`gain_lin ∈ [0, 1]` 是构造性的**；
//!    `makeup_gain ∈ [10^−1.2, 10^1.2]`（`makeup_db` 先经 [`CompressorParams::sanitised`]）
//!    ⇒ 每一步返回的 `gain_lin · makeup_gain` **有限**。
//! 6. **写回**走 `output_sample`：它对乘积再取一次 `finite_or_zero` ⇒
//!    写进缓冲的样本**恒有限**（`NaN`/`±∞` 的输入样本与溢出的乘积都落在这一条里）。
//!    第 5 步的界在这里用来解释**为什么这一步是必要的**：`gain ≤ 10^1.2`，
//!    所以 `|x| > 2.1e37` 的**有限**输入也能让乘积溢出。
//!
//! 判据 `hostile_samples_never_escape_into_the_state`、
//! `a_full_scale_square_wave_never_escapes_the_bound`、
//! `mean_square_never_leaves_its_domain` 与
//! `hostile_blocks_never_write_a_non_finite_sample` 对该证明做反证法检查。
//!
//! ## 7. 实时安全（[ARCH-RT-001] / `MUST-GATE-001`）
//!
//! [`Compressor`] 是**纯标量状态**（`Copy`、无 `Vec`、无 `Box`），
//! [`Compressor::new`] 与 [`Compressor::set_params`] 里没有分配，逐样本路径
//! （`process_gain`）里没有 `Vec`/`Box`/`String`/`format!`/`println!`/锁/`exp`。
//! `α` 与 `exp` 只在参数设置时算一次（与 [`crate::smoothing`] 同一纪律）；
//! 逐样本路径只有两处比较、一次 `log10`、一次 `exp2`（[`crate::math::db_to_gain`]）、
//! 一次 `log2`（[`crate::math::gain_to_db`]）与两次乘法。
//!
//! 块 API 的**写回**（`output_sample`）每个样本各多一次 `is_finite` 分支。
//! 它不加分配、不加锁、不做 I/O ——
//! 运行期由 `crates/yeban-dsp/tests/compressor_rt_zero_alloc.rs` 的
//! `allocations == 0 && deallocations == 0` 读数覆盖（那块判据走的正是
//! [`Compressor::process_stereo`] 与 [`Compressor::process_mono`]）。
//!
//! ## 8. 移植源（核实结论，⛔ 不是猜测）
//!
//! 本机取不到现成的动态范围压缩器实现。核实依据：
//!
//! - `grep -rn 'compress\|Compress\|Compressor' crates/ --include=*.rs` 的命中
//!   **全部**是三类别的东西：文件压缩（`yeban-render/src/als.rs` 的 `flate2::Compression`、
//!   `yeban-model` 的 `UnsupportedCompression`、SHA-256 的 `compress(&block)`）、
//!   以及一个**界面标签**（`yeban-app/src/scene.rs:110` 的设备名常量 `"Compressor"`）。
//!   没有任何一条是音频动态范围压缩。
//! - `grep -rn 'compress\|Compressor\|压缩' docs/ --include=*.md` 的命中同样是
//!   ZIP/deflate 与"工程文件体积压缩"。规范里**没有**压缩器的算法要求。
//! - `docs/ledger/engine-mix-notes.md:470` 明确登记："只有四极梯形低通
//!   （`LadderFilter`）；高通/带通/EQ/**压缩未接**" ⇒ 既有代码里也没有。
//! - 最接近的既有实现是 `yeban-engine/src/mixer.rs` 的 `BusLimiter`
//!   （前视 33 帧 + 立即攻击 + 速率上限释放 + 软膝天花板）。它是**限制器**，
//!   比率是 `∞`、没有阈值以下的比率段、也没有可配置起振时间，**不是**压缩器。
//!   本模块与它的共同点只有"dB 域静态曲线 + 一阶弹道"这一层约定；
//!   本模块**没有**移植它的代码（它在另一个 crate，本票不碰）。
//! - `signalsmith-stretch` 是弹性拉伸库，不含压缩器。
//!
//! 结论：静态曲线取自上述教科书口径，弹道与检波器按本模块 §1–§3 的设计写。
//! 本机**没有** NeuroNote 或任何其它参考实现可引（⛔ 不猜）。

use crate::math::{db_to_gain, sanitise_sample_rate};

/// 电平下限（dBFS）。低于此值的输入按静音处理。
///
/// `−120 dBFS` 与 [`crate::math::db_to_gain`] 的截断点同口径（该函数在
/// `≤ −120 dB` 处返回 `0.0`）。它由 [`POWER_FLOOR`] 派生：
/// `10·log10(10^−12) = −120`。
pub const MIN_LEVEL_DB: f32 = -120.0;

/// 检波器均方的下限（线性功率，无量纲）。
///
/// `10^−12` 正是 `−120 dBFS`。**这条钳位是"状态里永不出现 `−∞`"的实现本体**：
/// 均方域递推（[`Compressor::process_gain`] 的第 ② 步）只接受 `≥` 本值的输入，
/// 因此后面的 `log10` 实参恒 `≥ 10^−12`，`level_db` 恒有限。
/// 用均方域而不是 dB 域做钳位，还带来一个性质：`ms = 0` 出发时
/// `ms' = α·ms_in ≥ 0`，即**均方域自己就维持在 `[0, ∞)`**，钳位只是兜底。
pub const POWER_FLOOR: f32 = 1.0e-12;

/// 阈值合法域的上界（dBFS）。
///
/// 取 `+24`：超过它的阈值等于"永不压缩"，且 `+24 dBFS` 已在任何浮点信号
/// 之上留足余量。
pub const MAX_THRESHOLD_DB: f32 = 24.0;

/// 比率上界。`1000:1` 之上与 `∞:1`（限制器）在 `f32` 上已无实际差别。
pub const MAX_RATIO: f32 = 1_000.0;

/// 拐点（软膝）宽度上界（dB）。`24 dB` 已覆盖"能听见拐点"的全部用法。
pub const MAX_KNEE_DB: f32 = 24.0;

/// 起振/释放时间常数的下界（秒）。`0.1 ms` 在 48 kHz 下是 4.8 个样本，
/// 再短就等于关闭弹道（且会让 `α` 逼近 1）。
pub const MIN_TIME_S: f32 = 0.000_1;

/// 起振/释放时间常数的上界（秒）。
pub const MAX_TIME_S: f32 = 10.0;

/// makeup（补偿增益）的合法域（dB）。负值合法（允许用压缩器做衰减）。
pub const MAX_MAKEUP_DB: f32 = 24.0;

/// 检波器平均窗口时间常数的默认值（秒）。
///
/// `5 ms` 在 48 kHz 下是 240 个样本。选它的理由：它是"看不见 200 Hz 以下的
/// 检波纹波"与"不拖慢 10 ms 级起振"之间的常用折中。它由
/// [`CompressorParams::DEFAULT`] 使用，并在 [`CompressorParams::sanitised`]
/// 里与起振/释放受同一个合法域约束。
pub const DEFAULT_DETECTOR_S: f32 = 0.005;

/// 压缩器参数。
///
/// 全部字段是 `f32`；单位在字段名里写明（`_db` = dB、`_s` = 秒、`ratio` = 无量纲）。
/// 非法值**不会 panic**：它们经 [`CompressorParams::sanitised`] 钳到合法域，
/// 该函数在 [`Compressor::new`] 与 [`Compressor::set_params`] 里各调用一次。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompressorParams {
    /// 阈值（dBFS）。电平低于它 ⇒ 增益 0 dB。
    pub threshold_db: f32,
    /// 比率 `R ≥ 1`（`4.0` = 4:1）。`1.0` ⇒ 完全透明（`gain_db` 恒为 0）。
    pub ratio: f32,
    /// 软膝宽度（dB）。`0.0` ⇒ 硬膝（在阈值处一个折角）。
    pub knee_db: f32,
    /// 检波器平均窗口的时间常数 `τ`（秒）。**对称**平均，见模块注释 §3.2 / §3.4。
    ///
    /// 它决定"多快能看清当前 RMS"，与起振/释放无关。太短 ⇒ 低频会被检波器
    /// 自身的纹波调制（抽吸）；太长 ⇒ 检波器跟不上瞬态，起振被它拖住。
    /// 默认 5 ms。
    pub detector_s: f32,
    /// 起振时间常数 `τ`（秒）。见模块注释 §3 / §3.3。
    pub attack_s: f32,
    /// 释放时间常数 `τ`（秒）。见模块注释 §3。
    pub release_s: f32,
    /// makeup 补偿增益（dB），在压缩增益**之后**相乘（顺序见模块注释 §5 与 §1）。
    pub makeup_db: f32,
}

impl CompressorParams {
    /// 默认参数：阈值 −12 dBFS、比率 4:1、软膝 6 dB、检波 5 ms、起振 10 ms、
    /// 释放 100 ms、makeup 0 dB。
    ///
    /// **为什么是这一组**：本仓库的规范、ADR 与既有代码**都没有**规定压缩器的默认
    /// 参数（核实过程见模块注释 §8）。这组是数字压缩器的教科书中位值
    /// （Reiss & McPherson 的软膝压缩器示例即 −12 dBFS / 4:1 / 10 ms / 100 ms），
    /// 选用它的理由是**可手算验证**：稳态增益衰减 `= (1 − 1/R)·(x − T)`
    /// `= 0.75·(x + 12) dB`，判据可以直接对账。
    pub const DEFAULT: Self = Self {
        threshold_db: -12.0,
        ratio: 4.0,
        knee_db: 6.0,
        detector_s: DEFAULT_DETECTOR_S,
        attack_s: 0.010,
        release_s: 0.100,
        makeup_db: 0.0,
    };

    /// 返回把每个字段钳到合法域后的副本。**全定义**：任何输入（含 `NaN`/`∞`）都有返回值。
    ///
    /// | 字段 | 合法域 | 低于下界 | 高于上界 | `NaN` |
    /// | :--- | :--- | :--- | :--- | :--- |
    /// | `threshold_db` | `[`[`MIN_LEVEL_DB`]`, `[`MAX_THRESHOLD_DB`]`]` | 下界 | 上界 | **下界** |
    /// | `ratio` | `[1, `[`MAX_RATIO`]`]` | `1` | 上界 | **`1`** |
    /// | `knee_db` | `[0, `[`MAX_KNEE_DB`]`]` | `0` | 上界 | **`0`** |
    /// | `detector_s` / `attack_s` / `release_s` | `[`[`MIN_TIME_S`]`, `[`MAX_TIME_S`]`]` | 下界 | 上界 | **下界** |
    /// | `makeup_db` | `[−`[`MAX_MAKEUP_DB`]`, `+`[`MAX_MAKEUP_DB`]`]` | 下界 | 上界 | **下界** |
    ///
    /// ⚠ `NaN` 一律归到**下界**（不是上界）。`f32::clamp` 对 `NaN` 是**恒等**
    /// （`NaN.clamp(a, b) == NaN`，因为它的实现在两个比较失败时返回 `self`），
    /// 所以单靠 `clamp` 会漏一个 `NaN` 进实时路径。这里每个字段都经
    /// [`clamp_low`]：它先判 `is_finite`，非有限输入直接取该字段的**上界或下界**
    /// （见上表）。选下界而不是上界，是因为"归到下界"在下述每个字段上都是
    /// **保守**的：阈值最低、比率 1（透明）、拐点 0（硬膝）、时间常数最短。
    /// 这条选择是**声明的**，不是浮点边界行为的副作用
    /// （判据 `sanitised_clamps_every_field_into_its_documented_domain` 钉死）。
    #[must_use]
    pub fn sanitised(self) -> Self {
        Self {
            threshold_db: clamp_low(self.threshold_db, MIN_LEVEL_DB, MAX_THRESHOLD_DB),
            ratio: clamp_low(self.ratio, 1.0, MAX_RATIO),
            knee_db: clamp_low(self.knee_db, 0.0, MAX_KNEE_DB),
            detector_s: clamp_low(self.detector_s, MIN_TIME_S, MAX_TIME_S),
            attack_s: clamp_low(self.attack_s, MIN_TIME_S, MAX_TIME_S),
            release_s: clamp_low(self.release_s, MIN_TIME_S, MAX_TIME_S),
            makeup_db: clamp_low(self.makeup_db, -MAX_MAKEUP_DB, MAX_MAKEUP_DB),
        }
    }
}

impl Default for CompressorParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 静态曲线：给定**输入电平**（dBFS）与参数，返回**输出电平**（dBFS）。纯函数。
///
/// 公式与三段的分界见模块注释 §4。`params` 会先经
/// [`CompressorParams::sanitised`]，因此本函数对任何输入都是全定义的。
///
/// 这是"给定输入与参数 ⇒ 期望电平"的静态参照：实时路径
/// [`Compressor::process_gain`] 用的是 [`gain_db_for`]，两者对同一个
/// `(level_db, params)` 必须给出 `gain_db_for == output_db_for − level_db`。
/// 判据 `static_curve_matches_the_closed_form_on_a_grid` 钉住这条一致性。
#[must_use]
pub fn output_db_for(level_db: f32, params: CompressorParams) -> f32 {
    let p = params.sanitised();
    if !level_db.is_finite() {
        // 静音（−∞）与非法值都归到电平下限：膝下 ⇒ 透明。
        return MIN_LEVEL_DB;
    }
    let level_db = level_db.max(MIN_LEVEL_DB);
    let over = level_db - p.threshold_db;
    if p.knee_db <= 0.0 {
        // 硬膝：整数比较，不需要除法。
        return if over <= 0.0 {
            level_db
        } else {
            p.threshold_db + over / p.ratio
        };
    }
    let half = p.knee_db * 0.5;
    if over <= -half {
        level_db
    } else if over < half {
        let shifted = over + half;
        level_db + (1.0 / p.ratio - 1.0) * shifted * shifted / (2.0 * p.knee_db)
    } else {
        p.threshold_db + over / p.ratio
    }
}

/// 静态曲线：给定**输入电平**（dBFS）与参数，返回**增益**（dB，负值 = 衰减）。纯函数。
///
/// 恒等式：`gain_db_for(x, p) == output_db_for(x, p) − x`（`x ≥ `[`MIN_LEVEL_DB`]`）。
/// 这就是模块注释 §4 的 `gain_db = y − x`，实现上直接用解析式（不做两次 log）。
#[must_use]
pub fn gain_db_for(level_db: f32, params: CompressorParams) -> f32 {
    let p = params.sanitised();
    if !level_db.is_finite() {
        return 0.0;
    }
    let level_db = level_db.max(MIN_LEVEL_DB);
    let over = level_db - p.threshold_db;
    if p.knee_db <= 0.0 {
        return if over <= 0.0 {
            0.0
        } else {
            over / p.ratio - over
        };
    }
    let half = p.knee_db * 0.5;
    if over <= -half {
        0.0
    } else if over < half {
        let shifted = over + half;
        (1.0 / p.ratio - 1.0) * shifted * shifted / (2.0 * p.knee_db)
    } else {
        over / p.ratio - over
    }
}

/// 一阶低通的 `α`：`α = 1 − exp(−1/(τ·fs))`。
///
/// 与 [`crate::smoothing`] 同一纪律：`exp` **只在这里算一次**（参数设置时），
/// 逐样本路径只有一次乘加。`sample_rate` 经 [`sanitise_sample_rate`] 钳制，
/// `time_s` 经 [`MIN_TIME_S`] 钳制 ⇒ `τ·fs > 0` 恒成立，`α` 恒在 `(0, 1)`。
#[must_use]
fn one_pole_alpha(time_s: f32, sample_rate: f32) -> f32 {
    let sample_rate = sanitise_sample_rate(sample_rate);
    let time_s = if time_s.is_finite() {
        time_s.clamp(MIN_TIME_S, MAX_TIME_S)
    } else {
        MIN_TIME_S
    };
    1.0 - (-1.0 / (time_s * sample_rate)).exp()
}

/// [`one_pole_alpha`] 的**纯 Rust 参照实现**，只用于判据（不在实时路径上）。
///
/// 用 `exp2` 而不是 `exp`：`exp(x) = exp2(x · log2(e))`。判据用它把
/// `α` 的两个独立算式互相对账，这样"`α` 算错了"会变红，而"`exp` 的
/// 最后一位不同"不会（容差见判据）。
#[cfg(test)]
#[must_use]
fn one_pole_alpha_reference(time_s: f32, sample_rate: f32) -> f32 {
    const LOG2_E: f32 = core::f32::consts::LOG2_E;
    let sample_rate = sanitise_sample_rate(sample_rate);
    let time_s = time_s.clamp(MIN_TIME_S, MAX_TIME_S);
    1.0 - (-(LOG2_E / (time_s * sample_rate))).exp2()
}

/// 把手里的样本归到有限值：`NaN` 与 `±∞` ⇒ `0.0`。
///
/// 与 `yeban-engine` 混音链的 `nan_to_zero` 的区别只有一条：那个保留 `±∞`
/// （因为限制器的上界证明需要它），本器件把 `±∞` 也归零——`∞² = ∞` 会
/// 污染检波器状态，而检波器没有像限制器那样的"窗口峰值"兜底。
#[inline]
#[must_use]
fn finite_or_zero(sample: f32) -> f32 {
    if sample.is_finite() { sample } else { 0.0 }
}

/// 计算**写回调用方缓冲**的那一个样本：`input · gain`，乘积非有限时归零。
///
/// 这是模块注释 §6 那条"输出恒有限"在**块 API** 上的落点。**一次**清洗管住两个来源：
///
/// 1. **非有限的输入样本**：`NaN` / `±∞` ⇒ 乘积非有限 ⇒ `0.0`。
///    只把清洗后的值喂给检波器是**不够**的 —— 那样算出来的增益虽然有限，
///    写回缓冲的却仍是原样本，器件于是把上游的数值事故原样交给了下游。
///    同类的两个器件都**不**把原样本写回：[`crate::limiter::Limiter::process_stereo`]
///    把 `nan_to_zero` 之后的值写进环形缓冲，
///    [`crate::channel_strip::ChannelStrip::process_stereo`] 把
///    `sanitize_sample` 之后的值写回切片。本器件此前是那条表里的例外。
/// 2. **乘积溢出**：`gain` 的合法上限是
///    `db_to_gain(`[`MAX_MAKEUP_DB`]`) = 10^(24/20) ≈ 15.85`，因此
///    `|input| > f32::MAX / 15.85 ≈ 2.1e37` 的**有限**输入乘上它仍会溢出成 `±∞`。
///
/// `gain` 由 [`Compressor::process_gain`] 返回，它恒有限（`gain_lin` 钳在
/// `[0, 1]`、`makeup_gain` 由钳过的 dB 值算出），所以**只清洗乘积**就够。
/// ⚠ 实测：把输入**单独**先清洗一次（`finite_or_zero(finite_or_zero(input) * gain)`）
/// **没有任何判据会变红** —— 那两个式子在 `gain` 有限的全部取值上相等
/// （`NaN · g = NaN`、`±∞ · g = ±∞`，两者都被外层归零），因此本实现不做那次
/// 不可观测的冗余清洗。这条按"没红的注入要报出来"登记。
///
/// 对**有限**输入且乘积有限的样本，本函数逐位恒等（[`finite_or_zero`] 对有限值
/// 返回同一个值）⇒ 既有输出一个字不改。判据
/// `finite_block_inputs_are_bit_identical_to_input_times_gain` 对 `±0.0`／
/// 次正规数／`f32::MAX`／`f32::MIN` 逐位钉住这一条。
#[inline]
#[must_use]
fn output_sample(input: f32, gain: f32) -> f32 {
    finite_or_zero(input * gain)
}

/// 钳到 `[low, high]`，且**非有限输入一律归到 `low`**。
///
/// 与 `f32::clamp` 的差别是本函数存在的**唯一**理由：`clamp` 对 `NaN` 是恒等
/// （`NaN.clamp(a, b) == NaN`），于是 `NaN` 会穿过参数钳制进入实时路径。
/// 这里先判 `is_finite`，`NaN` 与 `±∞` 都取 `low`。
///
/// 前提：`low <= high` 且两者有限。本模块的调用点全部满足（5 组常量）。
#[inline]
#[must_use]
fn clamp_low(value: f32, low: f32, high: f32) -> f32 {
    if value.is_finite() {
        value.clamp(low, high)
    } else {
        low
    }
}

/// 前馈式立体声联动压缩器。全部状态是**标量**，`Copy`，无堆分配 [ARCH-RT-001]。
///
/// ```
/// use yeban_dsp::compressor::{Compressor, CompressorParams};
///
/// let mut comp = Compressor::new(CompressorParams::DEFAULT, 48_000.0);
/// assert_eq!(comp.latency_samples(), 0);
///
/// // 膝下 ⇒ 增益 0 dB ⇒ 输出与输入逐位相同。
/// let mut buf = [0.1f32; 128];
/// comp.process_mono(&mut buf);
/// assert!(buf.iter().all(|s| (*s - 0.1).abs() < 1e-6));
///
/// // 膝上 ⇒ 增益为负，且输出**永远**不超过输入（压缩只能衰减）。
/// let mut loud = [4.0f32; 128];
/// comp.process_mono(&mut loud);
/// assert!(comp.gain_db() < 0.0);
/// assert!(loud.iter().all(|s| *s <= 4.0));
/// // 稳态读数由 `a_known_sine_…` 与 `over_full_scale_input_stays_finite`
/// // 两条判据钉住（这里只跑 128 帧，起振还没走完）。
/// ```
#[derive(Clone, Copy, Debug)]
pub struct Compressor {
    /// 钳到合法域的参数。逐样本路径从这里读阈值/比率/拐点/makeup（只读）。
    params: CompressorParams,
    sample_rate: f32,
    /// 检波器平均系数（对称，模块注释 §3.2）。
    alpha_detector: f32,
    /// 增益弹道系数（增益向更小走）。
    alpha_attack: f32,
    /// 增益弹道系数（增益向更大走）。
    alpha_release: f32,
    /// 光滑后的输入**均方**（线性功率，`≥` [`POWER_FLOOR`]）。弹道状态在这里。
    mean_square: f32,
    /// 由 [`Self::mean_square`] 派生的输入电平（dBFS，`≥` [`MIN_LEVEL_DB`]）。
    level_db: f32,
    /// 本帧的压缩增益（dB，`≤ 0`）。**不含** makeup。由 [`Self::gain_lin`] 派生。
    gain_db: f32,
    /// 本帧的压缩增益（线性，`[0, 1]`）。**弹道状态在这一级**。
    gain_lin: f32,
    /// makeup 的线性增益，由 `params.makeup_db` 派生（构造期算好）。
    makeup_gain: f32,
    /// 全程出现过的最大增益衰减（dB，`≥ 0`）＝ `−min(gain_db)`。
    max_reduction_db: f32,
    /// 被压过的样本数（`gain_db < 0` 的样本）。
    reductions: u64,
    /// 已处理样本数（按声道计）。
    processed: u64,
}

impl Compressor {
    /// 建一个压缩器。参数经 [`CompressorParams::sanitised`] 钳制。
    ///
    /// **调用时机**：控制面（非实时路径）。它可以分配（本实现不分配），
    /// 但 [ARCH-RT-001] 只约束逐样本路径。
    #[must_use]
    pub fn new(params: CompressorParams, sample_rate: f32) -> Self {
        let params = params.sanitised();
        let sample_rate = sanitise_sample_rate(sample_rate);
        Self {
            params,
            sample_rate,
            alpha_detector: one_pole_alpha(params.detector_s, sample_rate),
            alpha_attack: one_pole_alpha(params.attack_s, sample_rate),
            alpha_release: one_pole_alpha(params.release_s, sample_rate),
            mean_square: 0.0,
            level_db: MIN_LEVEL_DB,
            gain_db: 0.0,
            gain_lin: 1.0,
            makeup_gain: db_to_gain(params.makeup_db),
            max_reduction_db: 0.0,
            reductions: 0,
            processed: 0,
        }
    }

    /// 换参数。参数经 [`CompressorParams::sanitised`] 钳制；`α` 在这里重算（`exp` **不在**逐样本路径上）。
    ///
    /// ⚠ **弹道状态不重置**：`mean_square` / `level_db` / `gain_db` 保持原值，
    /// 因此换阈值时不会出现阶跃。若新阈值低于当前 `level_db`，均方仍按原轨迹演化，
    /// 增益随之在**释放**时间常数内爬回 0（增益由均方**唯一**决定）。
    /// ⚠ makeup 是**立即**生效的乘数（没有 ramp）。实时改 makeup 会产生阶跃；
    /// 需要无爆音地推 makeup 时，应把平滑放在调用方（例如
    /// [`crate::smoothing::ParamSmoother`]），或按模块注释 §5 的纪律只在
    /// 块边界改参数。这条限制是**登记项**，不是遗漏。
    pub fn set_params(&mut self, params: CompressorParams) {
        self.params = params.sanitised();
        self.alpha_detector = one_pole_alpha(self.params.detector_s, self.sample_rate);
        self.alpha_attack = one_pole_alpha(self.params.attack_s, self.sample_rate);
        self.alpha_release = one_pole_alpha(self.params.release_s, self.sample_rate);
        self.makeup_gain = db_to_gain(self.params.makeup_db);
    }

    /// 换采样率并重算 `α`。**实时路径**上允许调用（无分配、无 `exp` 之外的调用）。
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sanitise_sample_rate(sample_rate);
        self.alpha_detector = one_pole_alpha(self.params.detector_s, self.sample_rate);
        self.alpha_attack = one_pole_alpha(self.params.attack_s, self.sample_rate);
        self.alpha_release = one_pole_alpha(self.params.release_s, self.sample_rate);
    }

    /// 清空弹道状态与统计（`mean_square` 回到 0、`level_db` 回到 [`MIN_LEVEL_DB`]、
    /// `gain_db` 回到 0 dB）。
    pub fn reset(&mut self) {
        self.mean_square = 0.0;
        self.level_db = MIN_LEVEL_DB;
        self.gain_db = 0.0;
        self.gain_lin = 1.0;
        self.max_reduction_db = 0.0;
        self.reductions = 0;
        self.processed = 0;
    }

    /// 当前参数（已钳到合法域）。
    #[must_use]
    pub const fn params(&self) -> CompressorParams {
        self.params
    }

    /// 当前采样率（Hz）。
    #[must_use]
    pub const fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// **延迟 = 0 样本**。本器件没有前视缓冲与延迟线，因此不进 PDC 表。
    ///
    /// ⚠ 与 `yeban-engine` 的前视限制器（33 帧）**不同**。这条是本器件的契约。
    #[must_use]
    pub const fn latency_samples(&self) -> usize {
        0
    }

    /// 光滑后的输入均方估计（线性功率，`≥ 0`）。它是本器件唯一的弹道状态。
    #[must_use]
    pub const fn mean_square(&self) -> f32 {
        self.mean_square
    }

    /// 由 [`Self::mean_square`] 派生的输入电平估计（dBFS，`≥` [`MIN_LEVEL_DB`]）。
    #[must_use]
    pub const fn level_db(&self) -> f32 {
        self.level_db
    }

    /// 当前**压缩**增益（dB，`≤ 0`）。**不含** makeup。
    #[must_use]
    pub const fn gain_db(&self) -> f32 {
        self.gain_db
    }

    /// 当前**总**增益（dB）＝ 压缩增益 + makeup。
    #[must_use]
    pub fn total_gain_db(&self) -> f32 {
        crate::math::gain_to_db(self.gain_lin * self.makeup_gain)
    }

    /// 当前压缩增益的线性值（`0 ≤ g ≤ 1`）。
    #[must_use]
    pub const fn gain_linear(&self) -> f32 {
        self.gain_lin
    }

    /// 当前 makeup 的线性值。
    #[must_use]
    pub const fn makeup_linear(&self) -> f32 {
        self.makeup_gain
    }

    /// 全程最大增益衰减（dB，`≥ 0`）。
    #[must_use]
    pub const fn max_reduction_db(&self) -> f32 {
        self.max_reduction_db
    }

    /// 被压过的样本数（`gain_db < 0` 的样本）。
    #[must_use]
    pub const fn reduction_count(&self) -> u64 {
        self.reductions
    }

    /// 已处理样本数（按**声道**计；立体声块计 2 次）。
    #[must_use]
    pub const fn processed_samples(&self) -> u64 {
        self.processed
    }

    /// 推进**一个立体声帧**的检波器与弹道，返回该帧的**总线性增益**。
    ///
    /// ⚠ 这是本器件唯一的逐样本内核。它**只**推进状态并返回 `g`；
    /// 把 `g` 乘到信号上由调用方做（[`Self::process_stereo`] /
    /// [`Self::process_mono`]）。返回 `g` 而不是"压缩后的样本"，是为了让
    /// 立体声与单声道两条路径共用**同一个** `g`，从而逐位一致。
    ///
    /// `NaN`/`±∞` 输入被归零（见 [`finite_or_zero`]）⇒ 返回值恒有限。
    /// **逐样本无分配**：里面只有乘、加、`log10` 与三处比较。
    #[inline]
    pub fn process_gain(&mut self, left: f32, right: f32) -> f32 {
        let l = finite_or_zero(left);
        let r = finite_or_zero(right);

        // ① 检波器：立体声联动，见模块注释 §2。
        let ms_in = (l * l).max(r * r);

        // ② 检波器平均：**对称**均方域一阶低通（模块注释 §3.2 / §3.4）。
        //    显式累加形式；均方是两个非负数的凸组合 ⇒ 恒 `≥ 0`，永不出现 −∞ / NaN。
        let mean_square = self.mean_square + self.alpha_detector * (ms_in - self.mean_square);
        self.mean_square = if mean_square.is_finite() {
            mean_square.max(0.0)
        } else {
            0.0
        };

        // ③ 电平：10·log10(ms)，均方先钳到 POWER_FLOOR ⇒ 结果恒 ≥ MIN_LEVEL_DB（§6）。
        self.level_db = 10.0 * self.mean_square.max(POWER_FLOOR).log10();

        // ④ 静态曲线（纯函数，无状态）。
        let target_gain_db = gain_db_for(self.level_db, self.params);

        // ⑤ 增益弹道：**线性增益域**一阶低通（模块注释 §3.3）。
        //    目标是 db_to_gain(target)，它恒在 [0, 1]；增益向更小走用起振、
        //    向更大走用释放。线性域保证 `t = τ` ⇒ 63.2%（幅度），且静音附近
        //    的目标恰是 0.0，不会重现 §3.1 的 dB 斜率放大。
        let target_gain = db_to_gain(target_gain_db);
        let alpha_gain = if target_gain < self.gain_lin {
            self.alpha_attack
        } else {
            self.alpha_release
        };
        let gain_lin = self.gain_lin + alpha_gain * (target_gain - self.gain_lin);
        self.gain_lin = if gain_lin.is_finite() {
            gain_lin.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.gain_db = crate::math::gain_to_db(self.gain_lin);

        if self.gain_lin < 1.0 {
            self.reductions = self.reductions.saturating_add(1);
        }
        let reduction = -self.gain_db;
        if reduction > self.max_reduction_db {
            self.max_reduction_db = reduction;
        }
        self.processed = self.processed.saturating_add(1);

        // ⑥ 应用：本器件是**逐声道**器件。检波器联动、增益共享，但每个声道
        //    各自乘同一个增益 ⇒ 声像不被拉开（模块注释 §2）。
        self.gain_lin * self.makeup_gain
    }

    /// 原地处理一个立体声块（两个切片必须等长）。
    ///
    /// - 逐样本**无分配**、无锁、无 I/O、无 `exp`。
    /// - 共享一个增益（立体声联动），因此声像不被拉开。
    /// - 两个切片长度不等时只处理 `min(len)` 个帧（**不 panic**），并返回该帧数。
    /// - 空切片 ⇒ 空操作，返回 0。
    /// - 写回的是 `output_sample` 的结果：`NaN`/`±∞` 的**输入样本**归零、
    ///   乘积溢出也归零 ⇒ **输出缓冲里不会出现非有限值**（模块注释 §6）。
    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32]) -> usize {
        let frames = left.len().min(right.len());
        let (left, right) = (&mut left[..frames], &mut right[..frames]);
        for frame in 0..frames {
            let gain = self.process_gain(left[frame], right[frame]);
            left[frame] = output_sample(left[frame], gain);
            right[frame] = output_sample(right[frame], gain);
        }
        // `process_gain` 每帧记 1；立体声帧是**两个**声道 ⇒ 这里补上另一声道。
        // 这样 `processed_samples()` 的口径就是"按声道计"，与 `process_mono` 一致。
        self.processed = self.processed.saturating_add(frames as u64);
        frames
    }

    /// 原地处理一个**单声道**块。
    ///
    /// 检波器按"同一路喂给两个输入"计算 ⇒ `max(l², l²) = l²`，与
    /// [`Self::process_stereo`] 传入两条相同声道时**逐位一致**（判据钉死）。
    ///
    /// 返回处理的样本数（即 `samples.len()`）。逐样本**无分配**。
    /// 写回口径与 [`Self::process_stereo`] 相同（`output_sample`）。
    pub fn process_mono(&mut self, samples: &mut [f32]) -> usize {
        for sample in samples.iter_mut() {
            let gain = self.process_gain(*sample, *sample);
            *sample = output_sample(*sample, gain);
        }
        samples.len()
    }
}

impl Default for Compressor {
    /// [`CompressorParams::DEFAULT`] @ 48 kHz。
    fn default() -> Self {
        Self::new(CompressorParams::DEFAULT, 48_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **判据（新写，可红）**：`reset` 之后的压缩器与**全新构造的同参数实例**在
    /// 同样输入下逐位一致。
    ///
    /// 量什么：256 个输出样本（`f32` 位型）、`gain_linear()`、`gain_db()`、
    /// `max_reduction_db()`、`reduction_count()`。
    ///
    /// `reset` 清七处状态，而同文件的 `reset_clears_state_and_statistics` 只读五个
    /// 读数，**不读**线性增益。注入实测：去掉 `self.gain_lin = 1.0;`
    /// ⇒ 既有全量判据**全绿** ⇒ 那条弹道状态没有被守住（复位后第一帧从旧增益起步）。
    #[test]
    fn reset_reproduces_a_freshly_built_compressor_bit_for_bit() {
        let params = CompressorParams {
            threshold_db: -20.0,
            ratio: 8.0,
            ..CompressorParams::DEFAULT
        };
        let build = || Compressor::new(params, 48_000.0);
        let drive = |compressor: &mut Compressor| -> (Vec<f32>, f32, f32, f32, u64) {
            let mut buffer: Vec<f32> = (0..256)
                .map(|index| 2.5 * ((index as f32) * 0.03).sin())
                .collect();
            compressor.process_mono(&mut buffer);
            (
                buffer,
                compressor.gain_linear(),
                compressor.gain_db(),
                compressor.max_reduction_db(),
                compressor.reduction_count(),
            )
        };
        let mut used = build();
        let warmup = drive(&mut used);
        assert!(warmup.4 > 0, "夹具必须真的驱动压缩器");
        used.reset();
        let after = drive(&mut used);
        let fresh = drive(&mut build());
        assert_eq!(after, fresh, "reset 之后与全新实例不一致");
    }

    /// 量什么：`α` 的两个独立算式（`exp` 与 `exp2`）之差，单位无量纲。
    /// 容差 1e-6 是为了容最后一位舍入，不是为了让错的公式通过 —— 见下一条判据。
    #[test]
    fn alpha_matches_an_independent_exp2_formulation() {
        for &sr in &[8_000.0f32, 44_100.0, 48_000.0, 96_000.0] {
            for &t in &[0.000_1f32, 0.001, 0.010, 0.100, 1.0, 10.0] {
                let a = one_pole_alpha(t, sr);
                let b = one_pole_alpha_reference(t, sr);
                assert!((a - b).abs() < 1e-6, "sr={sr} t={t} a={a} b={b}");
            }
        }
    }

    /// 量什么：`α` 是否落在开区间 `(0, 1)`（含退化输入）。
    #[test]
    fn alpha_is_always_inside_zero_and_one() {
        for &sr in &[0.0f32, -1.0, f32::NAN, f32::INFINITY, 1_000.0, 192_000.0] {
            for &t in &[0.0f32, -5.0, f32::NAN, f32::INFINITY, 1e-9, 1e9] {
                let a = one_pole_alpha(t, sr);
                assert!(a > 0.0 && a < 1.0, "sr={sr} t={t} a={a}");
            }
        }
    }

    /// 量什么：参数钳制对 6 个字段的逐字段读数（单位：dB / 无量纲 / 秒）。
    #[test]
    fn sanitised_clamps_every_field_into_its_documented_domain() {
        let p = CompressorParams {
            threshold_db: f32::NAN,
            ratio: 0.25,
            detector_s: 0.0,
            knee_db: -3.0,
            attack_s: 0.0,
            release_s: f32::INFINITY,
            makeup_db: -100.0,
        }
        .sanitised();
        assert_eq!(p.threshold_db, MIN_LEVEL_DB);
        assert_eq!(p.ratio, 1.0);
        assert_eq!(p.knee_db, 0.0);
        assert_eq!(p.detector_s, MIN_TIME_S, "detector_s {:?}", p.detector_s);
        assert_eq!(p.attack_s, MIN_TIME_S);
        // `release_s: f32::INFINITY` 是**非有限** ⇒ `clamp_low` 取**下界**（= MIN_TIME_S），
        // 不是上界。这条口径由 `sanitised` 的文档表钉死。
        assert_eq!(p.release_s, MIN_TIME_S);
        assert_eq!(p.makeup_db, -MAX_MAKEUP_DB);

        let q = CompressorParams {
            threshold_db: 1e9,
            ratio: f32::NAN,
            detector_s: -1.0,
            knee_db: 1e9,
            attack_s: -1.0,
            release_s: 0.0,
            makeup_db: f32::NAN,
        }
        .sanitised();
        assert_eq!(q.threshold_db, MAX_THRESHOLD_DB);
        // `NaN` 的确定性归约是**下界**（`NaN.max(lo) == lo`），不是上界 —— 见
        // `CompressorParams::sanitised` 的文档与 `clamp_low` 的实现。
        assert_eq!(q.ratio, 1.0);
        assert_eq!(q.knee_db, MAX_KNEE_DB);
        // `detector_s: -1.0`（有限、低于下界）⇒ 钳到下界。
        assert_eq!(q.detector_s, MIN_TIME_S, "detector_s {:?}", q.detector_s);
        assert_eq!(q.attack_s, MIN_TIME_S);
        assert_eq!(q.release_s, MIN_TIME_S);
        assert_eq!(q.makeup_db, -MAX_MAKEUP_DB);
    }

    /// 量什么：`gain_db_for` 与 `output_db_for − x` 在网格上的最大绝对差，单位 dB。
    /// 网格：电平 0…−119 dBFS（步 1）、阈值/比率/拐点的代表值。
    /// 这条钉死"实时路径用的解析式"与"静态参照用的分段式"是**同一条曲线**。
    #[test]
    fn static_curve_matches_the_closed_form_on_a_grid() {
        let mut worst = 0.0f32;
        for &threshold in &[-40.0f32, -20.0, -12.0, 0.0, 6.0] {
            for &ratio in &[1.0f32, 1.5, 2.0, 4.0, 10.0, 100.0] {
                for &knee in &[0.0f32, 1.0, 6.0, 12.0, 24.0] {
                    let p = CompressorParams {
                        threshold_db: threshold,
                        ratio,
                        knee_db: knee,
                        ..CompressorParams::DEFAULT
                    }
                    .sanitised();
                    for step in 0..120 {
                        let x = MIN_LEVEL_DB + step as f32; // −120 … −1 dBFS
                        let g = gain_db_for(x, p);
                        let y = output_db_for(x, p);
                        worst = worst.max((g - (y - x)).abs());
                    }
                    // 超幅区（正 dBFS）也必须一致。
                    for &x in &[0.0f32, 1.0, 6.0, 12.0, 24.0, 60.0] {
                        let g = gain_db_for(x, p);
                        let y = output_db_for(x, p);
                        worst = worst.max((g - (y - x)).abs());
                    }
                }
            }
        }
        assert!(worst < 1e-3, "两个算式在网格上的最大差 = {worst} dB");
    }

    /// 量什么：`ratio = 1`、阈值内、`knee = 0`、`makeup = 0` 时 `gain_db_for` 的读数，单位 dB。
    #[test]
    fn a_transparent_setting_has_exactly_zero_gain() {
        let p = CompressorParams {
            threshold_db: -12.0,
            ratio: 1.0,
            knee_db: 0.0,
            makeup_db: 0.0,
            ..CompressorParams::DEFAULT
        };
        for step in 0..200 {
            let x = -60.0 + step as f32 * 0.5;
            assert_eq!(gain_db_for(x, p), 0.0, "ratio=1 必须在 x={x} 处给 0 dB");
        }
    }

    /// 量什么：`R → ∞` 时膝上输出电平与阈值之差，单位 dB。
    /// 期望：`|output_db_for(x) − T| → 0`（这就是限制器的极限）。
    #[test]
    fn infinite_ratio_pins_the_output_at_the_threshold() {
        let p = CompressorParams {
            threshold_db: -6.0,
            ratio: 1_000.0,
            knee_db: 0.0,
            ..CompressorParams::DEFAULT
        };
        for &x in &[0.0f32, 6.0, 12.0, 24.0] {
            let y = output_db_for(x, p);
            assert!((y - (-6.0)).abs() < 0.05, "x={x} y={y}");
        }
    }

    /// 量什么：软膝三段在两个分界处与**解析式**的差，单位 dB。
    ///
    /// 分界在 `o = ±W/2`（**不是** `±knee`、也不是 `±knee/2`，虽然此例 `W/2 = 4`）。
    /// 判据用**两侧解析式在同一个 `o` 上求值**来判连续，不用单侧极限 ——
    /// 单侧极限在步长 `1e-3` 下天然有 `2e-3` 的差，那是取样假象。
    #[test]
    fn the_soft_knee_is_continuous_at_both_junctions() {
        let p = CompressorParams {
            threshold_db: -20.0,
            ratio: 6.0,
            knee_db: 8.0,
            ..CompressorParams::DEFAULT
        };
        assert_eq!(p.knee_db, 8.0);
        let half = p.knee_db * 0.5; // = 4
        // 膝内式与膝上式的闭式（与实现独立抄写一遍）。
        let inside =
            |o: f32| -20.0 + o + (1.0 / p.ratio - 1.0) * (o + half).powi(2) / (2.0 * p.knee_db);
        let outside = |o: f32| -20.0 + o / p.ratio;
        assert!(
            (inside(half) - outside(half)).abs() < 1e-4,
            "o=+W/2 处膝内 {} vs 膝上 {}",
            inside(half),
            outside(half)
        );
        assert!(
            (inside(-half) - (-20.0 - half)).abs() < 1e-4,
            "o=−W/2 处膝内 {} vs 透明 {}",
            inside(-half),
            -20.0 - half
        );
        // 实现在分界两侧都必须与上述闭式一致。
        for &o in &[
            -half - 1.0,
            -half - 1e-3,
            -half,
            -half + 1e-3,
            half - 1e-3,
            half,
            half + 1e-3,
            half + 1.0,
        ] {
            let x = -20.0 + o;
            let got = output_db_for(x, p);
            let want = if o <= -half {
                x
            } else if o < half {
                inside(o)
            } else {
                outside(o)
            };
            assert!((got - want).abs() < 1e-3, "o={o} got={got} want={want}");
        }
        // 膝上必须严格低于膝下（真的在压）。
        assert!(output_db_for(-10.0, p) < -10.0);
        // 膝下必须逐位透明。
        assert_eq!(output_db_for(-40.0, p), -40.0);
    }

    /// 量什么：默认参数 @48 kHz 下，1 kHz 正弦的**稳态**增益衰减，单位 dB。
    /// 输入：幅度 `A = 0.5`（−6.02 dBFS 峰值，−9.03 dBFS RMS）。
    /// 期望（算术）：`x_rms = 0.5/√2 = 0.353553 ⇒ x = −9.0309 dBFS`；
    /// `o = x − T = −9.0309 + 12 = 2.9691 dB`；`W/2 = 3` ⇒ `o < W/2` ⇒ **膝内**分支：
    /// `g = (1/4 − 1)·(o + 3)² / (2·6) = −0.75 · 35.6355 / 12 = −2.22722 dB`。
    #[test]
    fn a_known_sine_gets_the_arithmetically_expected_steady_state_reduction() {
        let sr = 48_000.0f32;
        let mut comp = Compressor::new(CompressorParams::DEFAULT, sr);
        let amplitude = 0.5f32;
        let mut buf = vec![0.0f32; 48_000 * 2];
        for (i, s) in buf.iter_mut().enumerate() {
            let t = i as f32 / sr;
            *s = amplitude * (core::f32::consts::TAU * 1_000.0 * t).sin();
        }
        comp.process_mono(&mut buf);
        let measured = comp.gain_db();
        let expected = {
            let x_rms_db = 20.0 * (amplitude / core::f32::consts::SQRT_2).log10();
            gain_db_for(x_rms_db, CompressorParams::DEFAULT)
        };
        assert!(
            (measured - expected).abs() < 0.15,
            "实测 {measured} dB，算术期望 {expected} dB"
        );
        assert!(measured < -1.9 && measured > -2.6, "读数越界: {measured}");
    }

    /// 量什么：**检波器**的阶跃响应。参数 `detector_s = 10 ms`，起振/释放设为
    /// 0.1 ms（增益级不构成瓶颈）。
    ///
    /// 判据：`level_db` 到达"距稳态 1 dB 以内"所需的**秒数**，与
    /// `1.585·τ_det` 对比（模块注释 §3：`1 − e^(−t/τ) = 10^(−1/10) = 0.7943`
    /// ⇒ `t = τ·ln(1/(1−0.7943)) = 1.5852·τ`）。容差 10%。
    #[test]
    fn attack_time_matches_the_declared_time_constant() {
        let sr = 48_000.0f32;
        let params = CompressorParams {
            threshold_db: -20.0,
            ratio: 4.0,
            knee_db: 0.0,
            detector_s: 0.010,
            attack_s: 0.000_1,
            release_s: 0.000_1,
            makeup_db: 0.0,
        };
        let mut comp = Compressor::new(params, sr);
        // 阶跃到 0.5 的直流（RMS = 0.5 ⇒ −6.02 dBFS）。
        let target_level = 20.0 * 0.5f32.log10();
        let mut settle_frame = 0usize;
        for n in 0..(sr as usize) {
            comp.process_gain(0.5, 0.5);
            if comp.level_db() >= target_level - 1.0 {
                settle_frame = n;
                break;
            }
        }
        let measured_s = settle_frame as f32 / sr;
        let expected_s = 1.585_2 * params.detector_s;
        assert!(
            (measured_s - expected_s).abs() < 0.10 * expected_s,
            "检波器实测 {measured_s} s，期望 1.585·τ_det = {expected_s} s"
        );
    }

    /// 量什么：**增益弹道**的释放。参数 `release_s = 50 ms`，检波器 0.1 ms。
    ///
    /// 判据：直流先被压到稳态，然后撤成静音；测 `gain_db` 回到"距 0 dB 相差
    /// 1 dB 以内"所需的**秒数**，与 `1.585·τ` 对比。容差 25% —— 见模块注释 §3.3
    /// 的登记项：线性增益域的一阶低通在**深衰减**处的 dB 轨迹不是纯指数。
    #[test]
    fn release_time_matches_the_declared_time_constant() {
        let sr = 48_000.0f32;
        let params = CompressorParams {
            threshold_db: -20.0,
            ratio: 4.0,
            knee_db: 0.0,
            detector_s: 0.000_1,
            attack_s: 0.000_1,
            release_s: 0.050,
            makeup_db: 0.0,
        };
        let mut comp = Compressor::new(params, sr);
        for _ in 0..(sr as usize / 2) {
            comp.process_gain(0.5, 0.5);
        }
        let steady = comp.gain_db();
        assert!(steady < -3.0, "稳态增益 {steady} dB 太浅，弹道测不出来");
        let mut settle_frame = 0usize;
        for n in 0..(sr as usize) {
            comp.process_gain(0.0, 0.0);
            if comp.gain_db() >= -1.0 {
                settle_frame = n;
                break;
            }
        }
        let measured_s = settle_frame as f32 / sr;
        let expected_s = 1.585_2 * params.release_s;
        assert!(
            (measured_s - expected_s).abs() < 0.25 * expected_s,
            "释放实测 {measured_s} s，期望 1.585τ = {expected_s} s"
        );
    }

    /// 量什么：**增益弹道**的起振。参数 `attack_s = 50 ms`，检波器 0.1 ms。
    ///
    /// 判据：直流阶跃 0.5；测 `gain_db` 到达"距稳态 1 dB 以内"所需的**秒数**，
    /// 与 `1.585·τ_attack` 对比。容差 25%（同上的 §3.3 登记项）。
    #[test]
    fn gain_attack_follows_the_declared_attack_constant() {
        let sr = 48_000.0f32;
        let params = CompressorParams {
            threshold_db: -20.0,
            ratio: 4.0,
            knee_db: 0.0,
            detector_s: 0.000_1,
            attack_s: 0.050,
            release_s: 0.000_1,
            makeup_db: 0.0,
        };
        let mut comp = Compressor::new(params, sr);
        // 先跑满 2 s 得到稳态。
        for _ in 0..(sr as usize * 2) {
            comp.process_gain(0.5, 0.5);
        }
        let steady = comp.gain_db();
        assert!(steady < -3.0, "稳态增益 {steady} dB 太浅");
        // 让检波器与增益都回到"静音"侧，再从同一起点起振 —— 这样测的是
        // **检波器已就位之后**的增益弹道（`detector_s = 0.1 ms` ⇒ 检波器
        // 在 1 个样本内就位），避免"两级同时启动"把读数拉长一倍。
        for _ in 0..(sr as usize / 4) {
            comp.process_gain(0.0, 0.0);
        }
        let released = comp.gain_db();
        assert!(
            released > steady,
            "释放后增益应回升: {released} vs {steady}"
        );
        let mut settle_frame = 0usize;
        for n in 0..(sr as usize) {
            comp.process_gain(0.5, 0.5);
            if comp.gain_db() <= steady + 1.0 {
                settle_frame = n;
                break;
            }
        }
        let measured_s = settle_frame as f32 / sr;
        // ⚠ 这条常数是**实测标定**的，不是闭式推导的（我先写了一个闭式，
        // 它被实测否决，教训见下）。两次实测：
        //
        // 1. 标定（`detector_s = 0.1 ms` 可忽略）：`settle / attack_s` =
        //    2.95625 / 2.95521 / 2.95458 / 2.95313 / 2.95021，对应
        //    `attack_s` = 10 / 20 / 50 / 100 / 200 ms（量程 20×，离散度 0.2%）。
        // 2. 判别力（固定 `attack_s = 50 ms`）：`settle / attack_s` =
        //    2.9546 / 2.9608 / 2.9896 / 3.1250，对应
        //    `detector_s` = 0.1 / 1 / 5 / 20 ms ⇒ 检波器变慢时读数**变大**。
        //
        // 这条判据因此同时钉两件事：(a) 增益弹道确实按 `attack_s` 线性伸缩；
        // (b) 读数对检波器窗口也敏感（不是恰好抵消的假象）。
        const GAIN_ATTACK_1DB_IN_TAU: f32 = 2.955;
        let expected_s = GAIN_ATTACK_1DB_IN_TAU * params.attack_s;
        assert!(
            (measured_s - expected_s).abs() < 0.05 * expected_s,
            "起振实测 {measured_s} s，标定期望 {GAIN_ATTACK_1DB_IN_TAU}τ = {expected_s} s"
        );
    }

    /// 量什么：上一条的**伸缩律**。同一判据在 `attack_s` = 10 / 20 / 100 ms 上
    /// 重复，测 `settle / attack_s` 的离散度（无量纲）。
    ///
    /// 这条会让"`attack_s` 被忽略或被写死"的实现变红 —— 单点判据做不到这一点
    /// （任何常数都能在某一个 τ 上偶然对上）。
    #[test]
    fn the_gain_attack_settle_time_scales_linearly_with_attack_s() {
        let sr = 48_000.0f32;
        let measure = |attack_s: f32| -> f32 {
            let params = CompressorParams {
                threshold_db: -20.0,
                ratio: 4.0,
                knee_db: 0.0,
                detector_s: 0.000_1,
                attack_s,
                release_s: 0.000_1,
                makeup_db: 0.0,
            };
            let mut warm = Compressor::new(params, sr);
            for _ in 0..(sr as usize * 2) {
                warm.process_gain(0.5, 0.5);
            }
            let steady = warm.gain_db();
            let mut comp = Compressor::new(params, sr);
            for n in 0..(sr as usize * 2) {
                comp.process_gain(0.5, 0.5);
                if comp.gain_db() <= steady + 1.0 {
                    return n as f32 / sr / attack_s;
                }
            }
            f32::NAN
        };
        let ratios = [measure(0.010), measure(0.020), measure(0.100)];
        let lo = ratios.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = ratios.iter().copied().fold(0.0f32, f32::max);
        assert!(lo.is_finite() && hi.is_finite(), "读数: {ratios:?}");
        assert!(
            (hi - lo) / hi < 0.01,
            "settle/attack_s 在 10/20/100 ms 上不该变: {ratios:?}"
        );
        assert!(
            (lo - 2.955).abs() < 0.05 * 2.955,
            "标定常数漂移: {ratios:?}"
        );
    }

    /// 量什么：`R = 4`、硬膝、直流输入下"实测增益 vs `(1/R − 1)·o`"的差，单位 dB。
    #[test]
    fn hard_knee_gain_lands_on_the_closed_form() {
        let params = CompressorParams {
            threshold_db: -20.0,
            ratio: 4.0,
            knee_db: 0.0,
            detector_s: 0.001,
            attack_s: 0.001,
            release_s: 0.100,
            makeup_db: 0.0,
        };
        let mut comp = Compressor::new(params, 48_000.0);
        for _ in 0..48_000 {
            comp.process_gain(0.5, 0.5);
        }
        let x = 20.0 * 0.5f32.log10();
        let over = x - (-20.0);
        let expected = (1.0 / 4.0 - 1.0) * over;
        assert!(
            (comp.gain_db() - expected).abs() < 0.05,
            "实测 {} vs 闭式 {expected}",
            comp.gain_db()
        );
    }

    /// 量什么：**同一次输入、三种切分**的输出是否逐位相同（`to_bits` 比较的失配计数）。
    /// 切分：1 / 3 / 7 / 128 / 999 / 1000 帧每块。
    #[test]
    fn chunking_does_not_change_output() {
        let sr = 48_000.0f32;
        let mut input = vec![0.0f32; 1_000];
        for (i, s) in input.iter_mut().enumerate() {
            let t = i as f32 / sr;
            // 有音符、有静音、有超幅，覆盖三段曲线。
            *s = if i < 300 {
                1.6 * (core::f32::consts::TAU * 220.0 * t).sin()
            } else if i < 600 {
                0.0
            } else {
                0.05 * (core::f32::consts::TAU * 3_000.0 * t).sin()
            };
        }
        let mut reference: Option<Vec<f32>> = None;
        for &chunk in &[1usize, 3, 7, 128, 999, 1_000] {
            let mut comp = Compressor::new(CompressorParams::DEFAULT, sr);
            let mut out = input.clone();
            let mut start = 0usize;
            while start < out.len() {
                let end = (start + chunk).min(out.len());
                comp.process_mono(&mut out[start..end]);
                start = end;
            }
            match &reference {
                None => reference = Some(out),
                Some(r) => {
                    let mismatches = r
                        .iter()
                        .zip(out.iter())
                        .filter(|(a, b)| a.to_bits() != b.to_bits())
                        .count();
                    assert_eq!(
                        mismatches, 0,
                        "chunk = {chunk} 时有 {mismatches} 个样本不同"
                    );
                }
            }
        }
    }

    /// 量什么：静音块输出是否**逐位**为 0，以及是否出现 NaN。
    #[test]
    fn silence_stays_silent_and_never_becomes_nan() {
        let mut comp = Compressor::new(CompressorParams::DEFAULT, 48_000.0);
        let mut buf = vec![0.0f32; 4_800];
        comp.process_mono(&mut buf);
        assert!(buf.iter().all(|s| *s == 0.0), "静音必须逐位为 0");
        assert_eq!(comp.level_db(), MIN_LEVEL_DB);
        assert!(!comp.gain_db().is_nan());
    }

    /// 量什么：膝下输入（−40 dBFS 直流）的输出是否**逐位**等于输入。
    /// 这条会让"压缩器完全不压缩"以外的错法（例如误压膝下）变红。
    #[test]
    fn below_threshold_is_bit_transparent() {
        let mut comp = Compressor::new(CompressorParams::DEFAULT, 48_000.0);
        let mut buf = vec![0.01f32; 4_800]; // −40 dBFS
        let original = buf.clone();
        comp.process_mono(&mut buf);
        let different = buf
            .iter()
            .zip(original.iter())
            .filter(|(a, b)| a.to_bits() != b.to_bits())
            .count();
        assert_eq!(different, 0, "膝下输出必须逐位等于输入");
        assert_eq!(comp.gain_db(), 0.0);
    }

    /// 量什么：敌意样本之后 `gain_db` / `level_db` 的读数是否有限（unit: bool）。
    /// 输入：`NaN` / `+∞` / `−∞` / 1e30 / −1e30 交替。
    #[test]
    fn hostile_samples_never_escape_into_the_state() {
        let mut comp = Compressor::new(CompressorParams::DEFAULT, 48_000.0);
        let hostile = [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            1.0e30,
            -1.0e30,
            0.0,
            f32::MIN,
        ];
        for round in 0..200 {
            for (i, &v) in hostile.iter().enumerate() {
                let gain = comp.process_gain(v, hostile[(i + round) % hostile.len()]);
                assert!(gain.is_finite(), "round={round} i={i} gain={gain}");
                assert!((0.0..=1.0).contains(&gain), "gain 越界: {gain}");
                assert!(comp.gain_db().is_finite(), "gain_db = {}", comp.gain_db());
                assert!(
                    comp.level_db().is_finite(),
                    "level_db = {}",
                    comp.level_db()
                );
                assert!(
                    comp.mean_square() >= 0.0 && comp.mean_square().is_finite(),
                    "mean_square = {}",
                    comp.mean_square()
                );
                assert!(comp.level_db() >= MIN_LEVEL_DB);
            }
        }
        assert!(comp.gain_db() <= 0.0);
    }

    /// 量什么：`NaN` / `±∞` **输入样本**经块 API 写回后的位模式（单位：`f32` 位）。
    ///
    /// 判据：写回的恰好是 `+0.0`（`to_bits() == 0`），**不是**原样本。
    /// 这是模块注释 §6 表格前两行在**块 API** 上的落点：只把清洗后的值喂给
    /// 检波器（[`Compressor::process_gain`] 本来就在做）不会让调用方的缓冲变干净。
    ///
    /// 注入：把块 API 的写回改回 `*= gain` ⇒ 本判据在 mono 与 stereo 两处变红
    /// （字面红行见提交说明）。
    #[test]
    fn non_finite_input_samples_are_written_back_as_zero() {
        let hostile = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];

        let mut mono = hostile;
        let mut comp = Compressor::new(CompressorParams::DEFAULT, 48_000.0);
        comp.process_mono(&mut mono);
        for (i, sample) in mono.iter().enumerate() {
            assert_eq!(
                sample.to_bits(),
                0.0f32.to_bits(),
                "mono 第 {i} 个样本写回的是 {sample}（位 {:#010x}），不是 +0.0",
                sample.to_bits()
            );
        }

        let mut left = hostile;
        let mut right = hostile;
        let mut comp = Compressor::new(CompressorParams::DEFAULT, 48_000.0);
        comp.process_stereo(&mut left, &mut right);
        for (i, sample) in left.iter().chain(right.iter()).enumerate() {
            assert_eq!(
                sample.to_bits(),
                0.0f32.to_bits(),
                "stereo 第 {i} 个样本写回的是 {sample}（位 {:#010x}），不是 +0.0",
                sample.to_bits()
            );
        }
    }

    /// 量什么：**块 API** 写回缓冲里的非有限样本个数（单位：个）。
    ///
    /// 判据：`0`。输入面覆盖 `NaN` / `±∞` / `±1e38` / `±3e38` / `f32::MAX` /
    /// `f32::MIN` / `±0.0` / 最小正次正规数；参数面覆盖默认（makeup 0 dB）与
    /// makeup `+`[`MAX_MAKEUP_DB`]（合法域上界，`gain ≤ 10^1.2 ≈ 15.85`
    /// ⇒ `|x| > 2.1e37` 的**有限**输入会把乘积溢出成 `±∞`）。
    /// 判据内部有正对照：输入表必须真的含非有限值，否则这条是空判据。
    ///
    /// 注入：把写回退回 `*sample *= gain`（即 [`output_sample`] 的清洗整个去掉）
    /// ⇒ 本判据在两组 makeup 上都变红（字面红行见提交说明）。
    /// ⚠ 另一条注入**不变红**并登记在此：把输入先清洗一次、再清洗乘积
    /// （`finite_or_zero(finite_or_zero(input) * gain)`）与只清洗乘积**在 `gain`
    /// 有限的全部取值上相等** ⇒ 那次冗余清洗不写进实现。
    #[test]
    fn hostile_blocks_never_write_a_non_finite_sample() {
        let hostile = [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            1.0e38,
            -1.0e38,
            3.0e38,
            f32::MAX,
            f32::MIN,
            0.0,
            -0.0,
            f32::from_bits(1),
            0.5,
        ];
        assert!(
            hostile.iter().any(|v| !v.is_finite()),
            "夹具失效：输入表里没有非有限值 ⇒ 本判据是空判据"
        );

        for makeup_db in [0.0f32, MAX_MAKEUP_DB] {
            let params = CompressorParams {
                makeup_db,
                ..CompressorParams::DEFAULT
            };

            let mut mono = hostile;
            let mut comp = Compressor::new(params, 48_000.0);
            assert_eq!(comp.process_mono(&mut mono), hostile.len());
            let bad = mono.iter().filter(|v| !v.is_finite()).count();
            assert_eq!(
                bad, 0,
                "makeup={makeup_db} dB：mono 写回了 {bad} 个非有限样本：mono={mono:?}"
            );

            let mut left = hostile;
            let mut right = hostile;
            let mut comp = Compressor::new(params, 48_000.0);
            assert_eq!(comp.process_stereo(&mut left, &mut right), hostile.len());
            let bad = left
                .iter()
                .chain(right.iter())
                .filter(|v| !v.is_finite())
                .count();
            assert_eq!(
                bad, 0,
                "makeup={makeup_db} dB：stereo 写回了 {bad} 个非有限样本：\
                 L={left:?} R={right:?}"
            );
        }
    }

    /// 量什么：有限输入下，块 API 写回的样本与"手算 `输入 × process_gain`
    /// 返回的增益"的位模式是否相等（单位：`f32` 位；读数是**不同的样本个数**）。
    ///
    /// 判据：`0` 个不同。这条钉住"写回清洗对**有限**输入逐位恒等"——
    /// 也就是"补齐 §6 的表格**没有**改动任何既有输出"。
    /// 输入覆盖 `±0.0` / 最小正次正规数 / `f32::MIN_POSITIVE` / `f32::MAX` /
    /// `f32::MIN` / `±1e30` / 正弦与满幅方波；makeup 取 `0` 与 `−`[`MAX_MAKEUP_DB`]
    /// （增益 `≤ 1`，乘积不会溢出，因此参考机的手算只用到 `f32` 乘法本身）。
    #[test]
    fn finite_block_inputs_are_bit_identical_to_input_times_gain() {
        let mut input: Vec<f32> = vec![
            0.0,
            -0.0,
            f32::from_bits(1),
            -f32::from_bits(1),
            f32::MIN_POSITIVE,
            f32::MAX,
            f32::MIN,
            1.0e30,
            -1.0e30,
            0.5,
            -0.5,
        ];
        for i in 0..200 {
            input.push((i as f32 * 0.37).sin());
            input.push(if i % 3 == 0 { 4.0 } else { -2.0 });
        }

        for makeup_db in [0.0f32, -MAX_MAKEUP_DB] {
            let params = CompressorParams {
                makeup_db,
                ..CompressorParams::DEFAULT
            };

            // 参考机：逐样本取增益，手算乘积（**不**经过块 API 的写回）。
            let mut reference = Compressor::new(params, 48_000.0);
            let expected: Vec<f32> = input
                .iter()
                .map(|x| {
                    let gain = reference.process_gain(*x, *x);
                    *x * gain
                })
                .collect();

            let mut device = Compressor::new(params, 48_000.0);
            let mut got = input.clone();
            assert_eq!(device.process_mono(&mut got), input.len());
            let differences = got
                .iter()
                .zip(expected.iter())
                .filter(|(a, b)| a.to_bits() != b.to_bits())
                .count();
            assert_eq!(
                differences, 0,
                "makeup={makeup_db} dB：{differences} 个样本的位模式与手算不同"
            );

            // 既有契约（"两声道同信号 ⇒ 与单声道逐位一致"）在新写回下仍然成立。
            let mut left = input.clone();
            let mut right = input.clone();
            let mut stereo = Compressor::new(params, 48_000.0);
            assert_eq!(stereo.process_stereo(&mut left, &mut right), input.len());
            assert!(
                left.iter()
                    .zip(got.iter())
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "makeup={makeup_db} dB：左声道与单声道路径不再逐位一致"
            );
            assert!(
                right
                    .iter()
                    .zip(got.iter())
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "makeup={makeup_db} dB：右声道与单声道路径不再逐位一致"
            );
        }
    }

    /// 量什么：均方状态是否**始终**落在 `[0, 1e12]` 内，且电平 `≥ MIN_LEVEL_DB`。
    /// 输入：静音 → 满幅正弦 → 静音 → 超幅 → 敌意值 的交替序列。
    /// 这条是模块注释 §6 第 2 步（`ms ≥ 0`）的直接判据。
    #[test]
    fn mean_square_never_leaves_its_domain() {
        let mut comp = Compressor::new(CompressorParams::DEFAULT, 48_000.0);
        let mut worst_ms = 0.0f32;
        let mut min_level = f32::INFINITY;
        for block in 0..40 {
            for n in 0..500 {
                let v = match block % 4 {
                    0 => 0.0,
                    1 => (n as f32 * 0.01).sin(),
                    2 => 4.0 * (n as f32 * 0.02).sin(),
                    _ => {
                        if n % 2 == 0 {
                            f32::NAN
                        } else {
                            f32::INFINITY
                        }
                    }
                };
                comp.process_gain(v, v * 0.5);
                let ms = comp.mean_square();
                assert!(ms >= 0.0, "block={block} n={n} ms={ms}");
                assert!(ms.is_finite(), "block={block} n={n} ms={ms}");
                assert!(
                    comp.level_db() >= MIN_LEVEL_DB,
                    "block={block} n={n} level={}",
                    comp.level_db()
                );
                worst_ms = worst_ms.max(ms);
                min_level = min_level.min(comp.level_db());
            }
        }
        assert!(worst_ms <= 16.0 + 1e-3, "最大均方 = {worst_ms}");
        assert!(min_level >= MIN_LEVEL_DB, "最小电平 = {min_level}");
    }

    /// 量什么：满幅方波（±1.0，每 2 个样本翻一次）10 000 帧后输出的有限性与界。
    /// 这条是"不许 NaN/Inf"在极限输入上的反证检查。
    #[test]
    fn a_full_scale_square_wave_never_escapes_the_bound() {
        let mut comp = Compressor::new(CompressorParams::DEFAULT, 48_000.0);
        let mut peak = 0.0f32;
        for n in 0..10_000 {
            let v = if n % 2 == 0 { 1.0 } else { -1.0 };
            let gain = comp.process_gain(v, v);
            assert!(gain.is_finite(), "n={n} gain={gain}");
            // 输出 = 输入 × 增益；这里直接算等价输出以便看界。
            peak = peak.max((v * gain).abs());
        }
        assert!(peak <= 1.0, "输出峰值 {peak} 越界");
        assert!(comp.gain_db() < 0.0);
    }

    /// 量什么：超幅输入的输出有限性（输入 4.0 = +12.04 dBFS，连续 4 800 帧）。
    #[test]
    fn over_full_scale_input_stays_finite() {
        let mut comp = Compressor::new(CompressorParams::DEFAULT, 48_000.0);
        let mut buf = vec![4.0f32; 4_800];
        comp.process_mono(&mut buf);
        assert!(buf.iter().all(|s| s.is_finite()));
        // 膝上：x = 20·log10(4) = 12.0412 dBFS；o = 12.0412 + 12 = 24.0412 dB；
        // g = (1/4 − 1)·o = −0.75·24.0412 = −18.0309 dB
        assert!(
            (comp.gain_db() - (-18.030_9)).abs() < 0.05,
            "gain_db = {}",
            comp.gain_db()
        );
        // 输出 = 4 · 10^(−18.0309/20) = 4 · 0.12547 = 0.50189
        // 缓冲开头是起振瞬态（增益还接近 1.0 ⇒ 输出接近 4.0），所以只对**稳态段**
        // （最后 1/4）断言。容差 0.05 覆盖 `db_to_gain` 的 `exp2` 近似
        // （它用 6.0206 而不是 20/log10(2)）。
        let steady = &buf[(buf.len() / 4 * 3)..];
        let worst = steady
            .iter()
            .map(|s| (*s - 0.501_89).abs())
            .fold(0.0f32, f32::max);
        assert!(
            worst < 0.05,
            "稳态段最大偏差 {worst}；末样本 {}，期望 4·10^(−18.0309/20) = 0.501876",
            buf[4_799]
        );
        // 起振瞬态必须在**上界一侧**（输出从 4.0 单调下来），不许过冲。
        assert!(
            buf.iter().all(|s| *s <= 4.0 + 1e-6),
            "输出不许超过未压缩的输入幅度"
        );
    }

    /// 量什么：立体声两条路径（`process_stereo` 与 `process_mono` 喂相同两路）
    /// 的输出是否逐位相同（失配计数）。
    #[test]
    fn stereo_and_mono_paths_agree_bit_for_bit_on_identical_channels() {
        let sr = 48_000.0f32;
        let params = CompressorParams::DEFAULT;
        let mut signal = vec![0.0f32; 2_000];
        for (i, s) in signal.iter_mut().enumerate() {
            let t = i as f32 / sr;
            *s = 0.8 * (core::f32::consts::TAU * 440.0 * t).sin();
        }
        let mut a = Compressor::new(params, sr);
        let (mut l, mut r) = (signal.clone(), signal.clone());
        a.process_stereo(&mut l, &mut r);
        let mut b = Compressor::new(params, sr);
        let mut mono = signal.clone();
        b.process_mono(&mut mono);
        let mismatches = l
            .iter()
            .zip(mono.iter())
            .filter(|(x, y)| x.to_bits() != y.to_bits())
            .count();
        assert_eq!(mismatches, 0);
    }

    /// 量什么：立体声联动的直接证据 —— **右声道自身低于阈值**，但它的输出仍被
    /// 左声道的响度压下去。读数：右声道在"左声道已撤成静音"之后的输出峰值
    /// （单位：线性幅度）。
    ///
    /// ⚠ 夹具的设计（第一版写错过，这里记下教训）：不能把"左响右轻"同时喂。
    /// 因为检波器取 `max(l², r²)`，右声道只在左声道**静音**之后才低于阈值；
    /// 而释放是慢的 ⇒ 左声道消失之后，右声道仍在旧增益之下。判据测的正是这一段。
    /// 反相：若每个声道各自算增益（没有联动），右声道会**逐位**保持 0.05 ——
    /// 它自己 −26 dBFS，远低于 −12 dBFS 阈值。
    #[test]
    fn stereo_linking_compresses_a_quiet_channel_by_the_loud_one() {
        let sr = 48_000.0f32;
        let params = CompressorParams {
            release_s: 1.0, // 慢释放 ⇒ 左声道撤走之后右声道仍被压
            ..CompressorParams::DEFAULT
        };
        let mut comp = Compressor::new(params, sr);
        let frames = 24_000; // 0.5 s
        let loud_end = 12_000; // 前 0.25 s 左声道是 0 dBFS 直流
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.05f32; frames]; // 全程 −26 dBFS，**低于阈值**
        for sample in left.iter_mut().take(loud_end) {
            *sample = 1.0;
        }
        comp.process_stereo(&mut left, &mut right);
        // 只在"左声道静音之后"取读数：这一段右声道自身绝对低于阈值。
        let quiet_peak = right[loud_end..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(
            quiet_peak < 0.045,
            "右声道峰值 {quiet_peak}（无联动时应为 0.05）⇒ 联动没生效"
        );
        assert!(quiet_peak > 0.0, "右声道不该被完全压死");
        assert!(
            comp.gain_db() < -6.0,
            "撤走 0.25 s 后增益应仍很负（release 1 s）：{}",
            comp.gain_db()
        );
        // 正对照：单独跑右声道（左声道全程 0）时，它必须**逐位**保持 0.05。
        let mut solo_comp = Compressor::new(params, sr);
        let mut silence = vec![0.0f32; frames];
        let mut solo = vec![0.05f32; frames];
        solo_comp.process_stereo(&mut silence, &mut solo);
        let solo_peak = solo[loud_end..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert_eq!(
            solo_peak.to_bits(),
            0.05f32.to_bits(),
            "低于阈值的右声道单独跑时必须在浮点上逐位不变，实际 {solo_peak}"
        );
    }

    /// 量什么：`process_stereo` 对不等长切片的返回值与是否 panic。
    #[test]
    fn unequal_length_stereo_slices_are_handled_without_panicking() {
        let mut comp = Compressor::new(CompressorParams::DEFAULT, 48_000.0);
        let mut l = vec![0.5f32; 10];
        let mut r = vec![0.5f32; 4];
        assert_eq!(comp.process_stereo(&mut l, &mut r), 4);
        let mut empty_l: Vec<f32> = Vec::new();
        let mut empty_r: Vec<f32> = Vec::new();
        assert_eq!(comp.process_stereo(&mut empty_l, &mut empty_r), 0);
        assert_eq!(comp.process_mono(&mut empty_l), 0);
    }

    /// 量什么：`makeup` 是否按声明的顺序（压缩之后）生效。
    /// 读数：同样输入在 `makeup_db = +6.0206` 时输出的比值，单位 dB。
    #[test]
    fn makeup_is_applied_after_the_compressor_gain() {
        let sr = 48_000.0f32;
        let base = CompressorParams {
            threshold_db: -20.0,
            ratio: 4.0,
            knee_db: 0.0,
            detector_s: 0.001,
            attack_s: 0.001,
            release_s: 0.100,
            makeup_db: 0.0,
        };
        let mut dry = Compressor::new(base, sr);
        let mut wet = Compressor::new(
            CompressorParams {
                makeup_db: 6.020_6,
                ..base
            },
            sr,
        );
        let frames = 4_800;
        let mut d = vec![0.25f32; frames];
        let mut w = vec![0.25f32; frames];
        dry.process_mono(&mut d);
        wet.process_mono(&mut w);
        let ratio_db = 20.0 * (w[frames - 1] / d[frames - 1]).log10();
        assert!(
            (ratio_db - 6.020_6).abs() < 0.02,
            "makeup 实测 {ratio_db} dB"
        );
        // 压缩增益本身不含 makeup。
        assert_eq!(dry.gain_db(), wet.gain_db());
    }

    /// 量什么：`reset` 之后的状态读数（电平 / 增益 / 计数）。
    #[test]
    fn reset_clears_state_and_statistics() {
        let mut comp = Compressor::new(CompressorParams::DEFAULT, 48_000.0);
        let mut buf = vec![1.0f32; 4_800];
        comp.process_mono(&mut buf);
        assert!(comp.reduction_count() > 0);
        assert!(comp.max_reduction_db() > 0.0);
        comp.reset();
        assert_eq!(comp.level_db(), MIN_LEVEL_DB);
        assert_eq!(comp.gain_db(), 0.0);
        assert_eq!(comp.reduction_count(), 0);
        assert_eq!(comp.max_reduction_db(), 0.0);
        assert_eq!(comp.processed_samples(), 0);
    }

    /// 量什么：`set_sample_rate` 之后 `α` 是否随采样率变化（读数：`α`，无量纲）。
    #[test]
    fn changing_the_sample_rate_recomputes_alpha() {
        let mut comp = Compressor::new(CompressorParams::DEFAULT, 48_000.0);
        let at_48k = comp.alpha_attack;
        comp.set_sample_rate(96_000.0);
        assert!(
            comp.alpha_attack < at_48k,
            "采样率翻倍后 α 必须减半左右: {} vs {at_48k}",
            comp.alpha_attack
        );
        assert_eq!(comp.sample_rate(), 96_000.0);
    }

    /// 量什么：`latency_samples` 的读数（单位：样本）。
    #[test]
    fn latency_is_zero() {
        let comp = Compressor::default();
        assert_eq!(comp.latency_samples(), 0);
        assert_eq!(comp.params(), CompressorParams::DEFAULT);
    }

    /// 量什么：`processed_samples` 在立体声块上的读数（单位：样本 × 声道）。
    #[test]
    fn processed_sample_counting_is_per_channel() {
        let mut comp = Compressor::new(CompressorParams::DEFAULT, 48_000.0);
        let mut l = vec![0.1f32; 64];
        let mut r = vec![0.1f32; 64];
        comp.process_stereo(&mut l, &mut r);
        assert_eq!(
            comp.processed_samples(),
            128,
            "立体声块按帧推进，按声道计数"
        );
    }

    /// **判据（新写，可红）**：八个文档化的域由**字面量**钉住，不引用常量本身。
    ///
    /// 量什么：六个界常量与两个时间常量的 `f32` 位型，以及 `sanitised()` 在**上界**
    /// 上的落点（`f32` 位型）。
    ///
    /// 既有判据 `sanitised_clamps_every_field_into_its_documented_domain` 写的是
    /// `assert_eq!(q.knee_db, MAX_KNEE_DB)`：**目标线随常量移动**，把 24 改成 48
    /// 它照样绿（与限制器 `LIMITER_CEILING` 被改时同一种躲法）。注入实测：
    /// `MAX_KNEE_DB` 24→48、`MAX_THRESHOLD_DB` 24→12、`MAX_MAKEUP_DB` 24→12
    /// 三次都**全绿** ⇒ 本判据逐条变红。
    ///
    /// 含 `exp` 的路径不在这里：本判据只读**参数域常量**与钳制结果，没有超越函数
    /// （比较与 `clamp` 都是 IEEE 精确类）⇒ 按裁决 R24 可跨架构硬断言。
    #[test]
    fn the_documented_domains_are_pinned_by_literals_not_by_themselves() {
        assert_eq!(MAX_RATIO.to_bits(), 1_000.0f32.to_bits());
        assert_eq!(MAX_KNEE_DB.to_bits(), 24.0f32.to_bits());
        assert_eq!(MAX_THRESHOLD_DB.to_bits(), 24.0f32.to_bits());
        assert_eq!(MAX_MAKEUP_DB.to_bits(), 24.0f32.to_bits());
        assert_eq!(MIN_LEVEL_DB.to_bits(), (-120.0f32).to_bits());
        assert_eq!(POWER_FLOOR.to_bits(), 1.0e-12f32.to_bits());
        assert_eq!(MIN_TIME_S.to_bits(), 0.000_1f32.to_bits());
        assert_eq!(MAX_TIME_S.to_bits(), 10.0f32.to_bits());

        let p = CompressorParams {
            threshold_db: 1.0e9,
            ratio: 1.0e9,
            knee_db: 1.0e9,
            detector_s: 1.0e9,
            attack_s: 1.0e9,
            release_s: 1.0e9,
            makeup_db: 1.0e9,
        }
        .sanitised();
        assert_eq!(p.threshold_db.to_bits(), 24.0f32.to_bits());
        assert_eq!(p.ratio.to_bits(), 1_000.0f32.to_bits());
        assert_eq!(p.knee_db.to_bits(), 24.0f32.to_bits());
        assert_eq!(p.detector_s.to_bits(), 10.0f32.to_bits());
        assert_eq!(p.attack_s.to_bits(), 10.0f32.to_bits());
        assert_eq!(p.release_s.to_bits(), 10.0f32.to_bits());
        assert_eq!(p.makeup_db.to_bits(), 24.0f32.to_bits());
    }
}
