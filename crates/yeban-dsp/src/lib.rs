//! # yeban-dsp — 纯数学 DSP 原语
//!
//! 本 crate 是最底层的纯数学信号处理库：无 I/O、无分配、无锁、无时钟。
//! 所有算法必须能在实时音频回调内以固定块长运行且**零堆分配** [ARCH-RT-001]。
//!
//! ## 设计纪律
//!
//! 1. **零堆分配**：所有 DSP 类型的容量在构造期确定（表、缓冲均为定长数组或
//!    构造期一次性建好的 `Vec`）。`process*` 类函数内不做任何 `push`/`Box::new`/
//!    `format!`/`collect`，也不调用会阻塞的系统调用 [ARCH-RT-001]。
//! 2. **无全局状态**：不存在可变的 `static`。参数由调用方逐实例设置，
//!    采样率**显式传入**（不作为隐式上下文）。
//! 3. **参数一律 `f32`、频率一律 Hz、时间一律秒**。
//! 4. **`#![forbid(unsafe_code)]`** [AGENTS.md 红线 8]。本文档内所有 SIMD 路径
//!    只保留标量实现：不为了向量化引入 `unsafe` 或内联汇编。
//! 5. **确定性**：所有随机源是显式播种的 xorshift，跨运行、跨平台同序 [ARCH-DET-001]。
//!
//! ## 复用来源 (Reuse provenance)
//!
//! 本 crate 是 `synth/crates/synth-core`（MIT, Copyright (c) 2026 GROOVE SYNTH
//! GS-1 contributors）中 `src/dsp/**` + `fx_shaping.rs` 的**改写移植**，
//! 不是逐行转译：接口、命名、采样率传递方式都按夜半的约定重写。
//!
//! - 逐文件裁决（来源 → 目标 → 改了哪里 → 为什么）：`docs/ledger/dsp-core-provenance.md`
//! - 复用审计结论：`docs/ledger/legacy-reuse-audit.md`
//! - 需要集成者登记的 `THIRD_PARTY_LICENSES.md` 条目原文见 provenance 文档 §6。
//!
//! 明确**不**移植的来源文件及其理由见 provenance 文档 §4（`alloc_arena.rs`、
//! `shim.rs`、`dual_filter.rs`、vendored C/C++、`dsp/fmath.rs`）。
//!
//! ## 模块地图
//!
//! | 模块 | 内容 | 来源 |
//! | :--- | :--- | :--- |
//! | [`block`] | 定长块的标量混音/峰值原语 | `dsp/simd.rs`（仅标量路径） |
//! | [`math`] | 音高换算、软限幅、线性插值、共享 FFT | `dsp/util.rs` |
//! | [`noise`] | 确定性 xorshift RNG 与白/粉/棕噪声 | `dsp/noise.rs` |
//! | [`smoothing`] | 一阶低通参数平滑（τ≈5ms） | 新写 [ARCH-DSP-001] |
//! | [`loop_window`] | 64 点升余弦循环点微平滑窗 | 新写 [ARCH-DSP-001] |
//! | [`meter`] | 电平口径（峰值/峰值保持/RMS/平滑 RMS/dBFS/钳位/取最新）+ **8×/16× 真峰值**（HD-26） | engine `level.rs` 上移 [ARCH-UI-002, ROAD-M2-008] |
//! | [`loudness`] | K 加权（BS.1770-4，44.1/48/88.2/96 kHz）+ 门限积分 LUFS（−70 LUFS / −10 LU）+ 瞬时/短时窗口（HD-27） | 新写 [ARCH-UI-002] |
//! | [`envelope`] | 可覆盖 release 的 ADSR | `dsp/adsr.rs` |
//! | [`filter`] | TPT/ZDF 4 极梯形低通 + 有界饱和 | `dsp/ladder.rs` |
//! | [`oscillator`] | mipmap 限带波表振荡器 + 全局 LFO | `dsp/wavetable.rs`、`dsp/lfo.rs` |
//! | [`oversample`] | 2× 过采样往返（显式延迟语义） | `dsp/oversample.rs` |
//! | [`delay`] | 插值延迟线 | `dsp/delay.rs` |
//! | [`comb`] | 梳状/全通滤波单元 | `dsp/comb.rs` |
//! | [`reverb`] | Freeverb 风格混响 | `dsp/reverb.rs` |
//! | [`shaping`] | bitcrusher / shaping EQ / transient shaper | `fx_shaping.rs` |
//! | [`compressor`] | 前馈式压缩器：软膝静态曲线 + RMS 检波 + 线性域增益弹道（**零延迟**） | 新写 [ARCH-RT-001]；本机无移植源，见该模块 §8 |
//! | [`channel_strip`] | 通道条：输入增益 → EQ → 滤波 → 动态（压缩） → 输出增益（**组合**既有器件，零延迟） | 组合 [ARCH-DSP-001]，见该模块 §2 |
//! | [`limiter`] | 母线前瞻式峰值限制器（立体声联动、33 帧延迟、立即攻击/速率上限释放、软膝天花板） | engine `mixer.rs` 上移 [ARCH-DSP-001, ARCH-PDC-001]，见该模块 §0 |
//! | [`polysynth`] | 双振荡器减法复音合成器（定容声部池 + 确定性窃取 + 3 ms 指数淡出 + 整数相位波表 + 声部级梯形低通） | engine `synth.rs` 上移 [ARCH-RT-001, ARCH-RT-004, ARCH-DET-001]，见该模块 §0 |
//!
//! ## 规范来源 (Normative)
//!
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3.2–§3.4 / §4.1
//!   （`ARCH-RT-001`、`ARCH-RT-003`、`ARCH-RT-004`、`ARCH-PDC-001`、`ARCH-DSP-001`）
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` §2.2
//! - `docs/ledger/legacy-reuse-audit.md` §2 复用纪律
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod block;
pub mod channel_strip;
pub mod comb;
pub mod compressor;
pub mod delay;
pub mod envelope;
pub mod filter;
pub mod limiter;
pub mod loop_window;
pub mod loudness;
pub mod math;
pub mod meter;
pub mod noise;
pub mod oscillator;
pub mod oversample;
pub mod polysynth;
pub mod reverb;
pub mod shaping;
pub mod smoothing;

/// 采样率的合法性下限（Hz）。所有需要采样率的构造/配置入口都用它做钳制，
/// 以免调用方传 0 或负数时把系数算成 `NaN`/`inf` [ARCH-DSP-001]。
pub const MIN_SAMPLE_RATE: f32 = 1_000.0;
