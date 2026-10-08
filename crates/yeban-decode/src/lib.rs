//! # yeban-decode — 离线解码与高精重采样
//!
//! 把"一个文件"变成"一个内容寻址的不可变 PCM 资产"：`symphonia` 负责容器解封装与
//! 采样解码，`rubato` 负责 44.1k / 48k / 96k 互转，`yeban-model` 负责内容寻址。
//!
//! ## 硬边界：**解码永远不在实时音频回调路径上** [ARCH-TOP-002] / [ARCH-RT-001]
//!
//! 规范 §0.1 把主进程划成五类执行上下文，本 crate 属于**第 5 类**
//! （后台任务线程池 / Background IO & Render Pool），不属于第 4 类
//! （cpal 实时音频回调线程）。这不是"实现风格"，是接口形状：
//!
//! | 本 crate **可以** | 实时回调线程 **绝不** |
//! | :--- | :--- |
//! | 阻塞式文件 I/O、`seek` | 任何文件/网络 I/O |
//! | 堆分配（`Vec` 增长、`try_reserve`） | 任何 `malloc` / `free` / `Box::new` |
//! | 逐 packet 解码循环 | 逐样本解码 |
//! | 一次几毫秒到几秒的重采样 | 任何"按需现算"的重采样 |
//!
//! 因此本 crate 的产物是一个**不可变资产** [`DecodedAsset`]：交织 `f32` PCM +
//! 解码事实 + 内容摘要。它只暴露 `&self` 访问器，可以放进 `Arc` 后被实时线程**只读**
//! 消费（渲染量子边界上的原子指针交换由 `yeban-engine` 负责，不在本 crate 范围内）。
//!
//! 反过来说：任何"实时线程里现解码一段素材"的设计都是本 crate 的**误用**，
//! 必须先离线解码成资产。
//!
//! ## 确定性 [ARCH-DET-001]
//!
//! L1（同平台同编译器位级一致）要求"固定算法与参数、无未固定种子随机、超越函数走
//! `libm`"。本 crate 的落实方式是：
//!
//! 1. **无随机源**：全 crate 不引用任何 PRNG，也不读时钟、环境变量或线程数；
//! 2. **无超越函数**：唯一的浮点运算是一次 `f64` 除法（时长换算）与 `rubato` 内部的
//!    乘加。**不需要** `libm`，因此本 crate 不依赖它；
//! 3. **固定的重采样配置**：sinc 窗 `BlackmanHarris2`、`sinc_len = 256`、分块 1024、
//!    最大相对比例 1.0 —— 全部是 `pub const`，见 [`resample`]；
//! 4. **不启用 symphonia 的 SIMD feature**（`opt-simd-*`）：那会让解码路径依赖运行时
//!    CPU 探测，与 L1/L2 的目标相悖，也会拉入 `rustfft`；
//! 5. **可比较的指纹**：[`DecodedAsset::pcm_hash`] 把解码结果做成 SHA-256 摘要，
//!    "同输入 → 同输出"因此可以被一条断言证明，而不必逐样本比几百万个数。
//!    摘要是分块流式算的（`yeban-model` 的 `AssetHasher`），不复制样本缓冲。
//!
//! ## 支持的格式（**明确清单**）
//!
//! 规范点名的两类（`ROAD-M-1-004`：仅保留开放的 SFZ v2 与标准 PCM WAV/FLAC）在
//! 本 crate 里落在 **WAV + FLAC** 上，另外保留 symphonia 上游默认集里的
//! **OGG / Vorbis / ADPCM**。逐条理由与"哪些是构造的字节真的测过、哪些只是代码路径存在"
//! 见 `docs/ledger/decode-core-notes.md` §3。上游的 Matroska、MP3、AAC、ALAC、AIFF、CAF、
//! ISO-MP4 一律**不启用**。
//!
//! ## 资源上限（[`limits::PcmBudget`]）—— **安全闸门 + 内存预算**（`HD-24`）
//!
//! 改建前这里是两个写死的 2 GiB 常量，`MAX_PCM_BYTES` 在 96 kHz 立体声下只够约
//! 46 分钟（96 kHz 8 声道只有约 11.6 分钟）⇒ 长工程必撞墙。改建后的口径：
//!
//! | 口径 | 默认值 | 依据 |
//! | :--- | :--- | :--- |
//! | 输入容器字节 | PCM 预算 + 1 MiB | 换算自 PCM 预算 + [`limits::CONTAINER_OVERHEAD_BYTES`] |
//! | 解码后交织 `f32` PCM | 96 kHz 立体声 **3 小时** = 96 kHz 8 声道 **45 分钟** | 产品要求推导，见 [`limits`] 模块文档 |
//! | 声道数 | 64 | [`limits::DEFAULT_MAX_CHANNELS`]（7.1 的 8 倍余量，挡畸形声明） |
//! | 采样率 | 768 kHz | [`limits::DEFAULT_MAX_SAMPLE_RATE`]（DXD 之上再留一倍） |
//! | 时长 | 6 小时 | [`limits::DEFAULT_MAX_DURATION_SECS`]（低采样率 × 少声道的独立 backstop） |
//!
//! 四道闸门**各自独立**、全部发生在**分配之前**；判定是闭区间（恰好等于上限通过）。
//! 预算显式可配置（[`DecodeOptions::budget`]、`resample_interleaved_with_budget`），
//! 并且一律用 `Vec::try_reserve` 把"分配器拒绝"变成错误而不是 abort —— 畸形输入
//! 不能把进程吃掉 [ARCH-SEC-003]。
//!
//! **为什么不自动探测可用内存**：那会让"同输入 → 同输出"（[ARCH-DET-001]）变成运行
//! 机器的函数。默认值是**有依据的常量**，应用层按自己的内存情况显式调大/调小。
//!
//! ## 不可信输入边界零 panic
//!
//! 畸形文件、截断流、未知编码、声明与实际不符，全部返回 [`DecodeError`]。
//! 本 crate 的**非测试代码**里没有 `unwrap` / `expect` / `panic!` / 切片索引。
//!
//! ## 模块地图
//!
//! | 模块 | 职责 | 第三方依赖 |
//! | :--- | :--- | :--- |
//! | [`decode`] | 路径 / 内存 / `Read + Seek` → [`DecodedAsset`] | symphonia |
//! | [`asset`] | 解码事实、`PcmFormat`、CAS 索引、`pcm_hash` | yeban-model |
//! | [`resample`] | 44.1k / 48k / 96k 互转与长度契约 | rubato |
//! | [`duration`] | 声明帧数 vs 解出帧数的对账 | 无 |
//! | [`limits`] | 尺寸预算与长度算术 | 无 |
//! | [`error`] | 统一错误类型与 symphonia 错误映射 | thiserror |
//!
//! [`duration`] 与 [`limits`] 是**零第三方依赖**的纯逻辑层，可以脱离 symphonia/rubato
//! 单独编译执行（本机验证方式见 notes §7）。
//!
//! ## 内存安全
//!
//! 本 crate 全程不需要 `unsafe`，因此按 AGENTS.md §2 红线 8 的"更好的状态"要求显式
//! `#![forbid(unsafe_code)]`（该红线只点名了 model/theory/dsp/render 四个 crate，
//! 这里是主动加严）。
//!
//! 规范来源 (Normative): `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §0.1/§5.1/§5.3/§10.2、
//! `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` `ROAD-M-1-004`、
//! `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md` D5/D20/D21。

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod asset;
pub mod decode;
pub mod duration;
pub mod error;
pub mod limits;
pub mod resample;

#[cfg(test)]
mod propcheck;

#[cfg(test)]
mod testfix;

pub use asset::{
    DecodeFacts, DecodedAsset, ImportedAsset, PcmFormat, asset_index, import_bytes, import_path,
};
pub use decode::{
    DecodeOptions, MeasuredSource, decode_bytes, decode_path, decode_reader, decode_source,
};
pub use duration::{Mismatch, Reconciliation, reconcile};
pub use error::{DecodeError, DecodeResult};
pub use limits::PcmBudget;
pub use resample::{
    resample_asset, resample_asset_with_budget, resample_interleaved,
    resample_interleaved_with_budget,
};
