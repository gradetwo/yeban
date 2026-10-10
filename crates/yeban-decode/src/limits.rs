//! 解码预算与长度算术 —— **零第三方依赖**的纯逻辑层。
//!
//! 为什么单独一层：这些判据决定"畸形输入会不会把进程吃掉"，它们必须在**不编译
//! symphonia / rubato** 的前提下就能被逐条跑红跑绿。本模块只用 `core`/`alloc`/`std`，
//! 因此可以用
//! `rustc --edition 2024 --test` 把本文件单独编成测试二进制在本机执行
//! （见 `docs/ledger/decode-core-notes.md` §7 的验证范围声明）。
//!
//! 规范来源 (Normative):
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §5.1 [ARCH-DET-001]：重采样必须
//!   "同输入 → 同输出"，所以长度契约只用**精确有理数整数运算**表达，不引入任何浮点
//!   或超越函数（`libm` 都不需要）；同一条契约也是"预算不随运行机器变化"的依据（见下）；
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §10.2 [ARCH-DSP-002]：
//!   44.1k/48k/96k 互转的输入输出长度关系；
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §5.3 [ARCH-SEC-003]：不可信输入的
//!   资源上限**必须仍然生效**（本模块是解码侧的落点）。注意：该条文的"单个解压条目
//!   ≤ 2 GB"约束的是**容器层**（`yeban-model` 的归档解包），不是解码侧的读缓冲
//!   —— 两者的信任上下文不同，改建前把同一个数字沿用到解码侧属于口径混淆。
//!
//! ## 上限的定性（`HD-24` 结论）：安全闸门 + 内存预算，**不是**实现限制
//!
//! 改建前这里是两个写死的 2 GiB 常量；`MAX_PCM_BYTES` 在 96 kHz 立体声下只够约
//! 46 分钟、96 kHz 8 声道下只够约 11.6 分钟 ⇒ 长工程必撞墙。逐条核对之后，
//! 它的正确定性是：
//!
//! 1. **它是安全闸门** —— 防止不可信输入（畸形声明、炸弹式头字段）把堆吃光，
//!    所以**不能删**；
//! 2. **它不是实现限制** —— 没有任何代码路径要求"单个缓冲 ≤ 2 GiB"：PCM 容器是
//!    `Vec<f32>`（只受地址空间限制）、算术全程 `u64`/`u128` + `checked_mul`、
//!    `rubato` 只吃切片。2 GiB 是从 [ARCH-SEC-003] 的**归档单条目**上限借来的数字
//!    （原文自述"同一个数字只在一个地方被裁决"），是**口径一致性**的选择，
//!    不是从解码行为推导出来的；
//! 3. 所以正确形态是 [`PcmBudget`]：**显式可配置 + 默认值有依据 + 与"可用内存"相关**。
//!
//! ## 默认值怎么来的（可被判据复算）
//!
//! 默认预算由**产品要求**推导，而不是拍一个数字：覆盖一次 **96 kHz 立体声 3 小时**
//! 的整场工程（`HD-24` 点名的长工程场景），并且不小于 **96 kHz 8 声道 30 分钟**；
//! 两者取字节数较大者。由此得到的三条等价关系（判据逐条复算，见 `#[cfg(test)]`）：
//!
//! | 布局 | 同一预算折算的时长 |
//! | :--- | :--- |
//! | 96 kHz 立体声 | **3 小时**（定义值） |
//! | 96 kHz 8 声道 | **45 分钟**（= 3 h × 2/8，改建前只有 11.6 分钟） |
//! | 48 kHz 立体声 | **6 小时**（字节上允许 6 h，但会被时长闸门卡在 6 h 整） |
//!
//! **为什么不自动探测可用内存**：那会让"同输入 → 同输出"（[ARCH-DET-001]）依赖运行
//! 机器 —— 同一个文件在一台机器上解码成功、在另一台上被预算拒绝，于是资产内容变成
//! 环境函数。预算值由**调用方**显式给出（应用层最清楚自己有多少内存），本 crate 只
//! 提供有依据的默认值，以及 [`PcmBudget::for_layout`] 这个"从产品要求反推预算"的构造。
//!
//! ## 峰值内存（本模块能证明的上界）
//!
//! `check_layout` 通过 ⇒ `frames × channels × 4 ≤ max_pcm_bytes`。解码期峰值 ≈ 1 份
//! 资产；重采样期峰值 ≈ 资产 + 输出（≈ 2 份，见 [`crate::resample`]）；
//! [`DecodedAsset::pcm_hash`](crate::asset::DecodedAsset::pcm_hash) 的峰值**与样本数
//! 无关** —— 它按定长分块喂摘要，暂存缓冲是固定大小的一块栈空间，不再复制整份样本。
//! 因此 `max_pcm_bytes` 是**单份 PCM** 的预算，不是进程峰值；调用方按需放大。
//!
//! ⚠ 2026-10-09 追加的例外：`decode_source` 收到一个**不可回退**的源
//! （`is_seekable() == false`，例如指向 FIFO 的 `File`）时会先把整份输入读进内存，因此
//! 那条路径的峰值还要再加**一份输入容器字节**（上限是 `max_input_bytes`）。可回退的源
//! 不受影响 —— 本 crate 的三个入口给出的都是可回退的源。见
//! [`crate::decode::decode_source`] 的文档。
//!
//! ⚠ 2026-10-09 更正：本段此前写"`pcm_hash` 另需一份与样本等长的字节缓冲"，那是
//! `ccee870` 把该路径改成流式**之前**的事实，改完后没有回写这里。同一份更正也已写进
//! `docs/ledger/decode-limits-notes.md` 的 §8 更正注（本 crate 不改该文件）。
//!
//! ## 重采样工作集：为什么比例本身必须是一道闸门
//!
//! `rubato` 的异步 sinc 重采样器在**任何**输入长度上都要按"一个处理分块 + 半个滤波器"
//! 准备输出缓冲。那两项是**输入**帧数：`CHUNK_FRAMES + SINC_LEN/2` = 1 152 帧。折算到
//! **输出**侧就是 `1 152 × 比例` 帧，**与输入有多少帧无关**（实测：1 帧、1 000 帧、
//! 4 096 帧输入在同一个比例下的"要求帧数 − 理想输出帧数"是同一个数）。
//!
//! 于是比例成了资源放大的**唯一**来源。合法参数的上界是 1 Hz → 768 kHz（比例 768 000）：
//! 那时**1 帧**输入要求 `885 504 010` 帧 = `3 542 016 040` 字节的输出缓冲，而
//! [`check_layout`] 的四道闸门一道都不挡它 —— 时长闸门按 `frames / 输出率` 算（1 153 秒，
//! 远低于 6 小时），字节闸门是 96 kHz 立体声 3 小时的 `8 294 400 000` 字节。
//!
//! 因此本模块把比例抽成一道**独立闸门**：[`PcmBudget::max_resample_ratio`] +
//! [`check_resample_ratio`]。它只读调用方声明的两个采样率，所以在**任何分配之前**判定，
//! 也不依赖 `rubato` 的任何行为。它**只拒绝**，不改动任何被放行的输出 —— 被放行的那一份
//! 输出与闸门存在之前**逐位相同**。
//!
//! 边界: 本模块**不做**任何 I/O、不持有缓冲、不知道 symphonia 的存在。它只回答
//! "这个尺寸/这个长度是否在预算内"。

use std::error::Error;
use std::fmt;

// ---------------------------------------------------------------------------
// 默认预算的**推导输入**（全部是产品要求，不是实现细节）
// ---------------------------------------------------------------------------

/// 默认预算的第一档参考采样率：96 kHz（`HD-24` 点名的最坏采样率）。
pub const DEFAULT_REFERENCE_RATE: u32 = 96_000;

/// 默认预算的第一档参考声道数：2（立体声整场工程）。
pub const DEFAULT_REFERENCE_CHANNELS: u16 = 2;

/// 默认预算的第一档时长要求：3 小时（一次整场录音/工程）。
pub const DEFAULT_REFERENCE_SECONDS: u64 = 3 * 60 * 60;

/// 默认预算的第二档参考声道数：8（多声道母带/现场分轨）。
pub const DEFAULT_MULTITRACK_CHANNELS: u16 = 8;

/// 默认预算的第二档时长要求：30 分钟。
pub const DEFAULT_MULTITRACK_SECONDS: u64 = 30 * 60;

/// 时长闸门的默认值：6 小时。
///
/// 为什么需要一个**与字节无关**的时长闸门：低采样率 × 少声道的素材"字节便宜、时间
/// 昂贵"（44.1 kHz 单声道 6 小时只有约 3.8 GiB PCM），单靠字节预算会允许任意长的
/// 时间轴。6 小时是这样选的：它**故意松于**默认字节预算在 96 kHz 立体声下的 3 小时
/// （那里字节先跳闸），又**故意紧于**默认字节预算在 44.1 kHz 立体声下的约 6.53 小时
/// （那里时长先跳闸）。超过 6 小时的素材是有意的例外，应由调用方显式给预算。
pub const DEFAULT_MAX_DURATION_SECS: u64 = 6 * 60 * 60;

/// 容器开销余量：1 MiB。
///
/// 用于把"PCM 字节预算"换算成"输入字节预算"：WAV/RIFF 的块头、FLAC 的元数据块、
/// Ogg 的页头都远小于这个数（未压缩 WAV 只多 44 字节量级），留 1 MiB 是为了容纳
/// 合法但啰嗦的元数据块，而不是给"压缩容器比 PCM 还大"这种情况开口子。
pub const CONTAINER_OVERHEAD_BYTES: u64 = 1024 * 1024;

/// 默认声道数闸门：64。
///
/// 本 crate 启用的容器（WAV/FLAC/Ogg）的常见布局最多到 7.1（8 声道），64 留了 8×
/// 余量；它同时是"畸形文件声明 65535 声道"在 `frames × channels` 乘法之前的硬上界。
pub const DEFAULT_MAX_CHANNELS: u16 = 64;

/// 默认采样率闸门：768 kHz（DXD 级别之上再留一倍余量）。
pub const DEFAULT_MAX_SAMPLE_RATE: u32 = 768_000;

/// 默认重采样**比例**闸门：`1_000`（单位是"输出率 / 输入率"的倍数）。
///
/// 为什么需要它：`rubato` 的异步 sinc 在每个处理分块上都要按"一个分块 + 半个滤波器"
/// 准备输出缓冲，折算到**输出**侧就是 `CHUNK_FRAMES + SINC_LEN/2` = 1 152 帧 × 比例 ——
/// 与输入帧数无关。极端比例（1 Hz → 768 kHz 是 768 000×）会让**1 帧**输入要求约 3.54 GB。
/// 见 [`PcmBudget::max_resample_ratio`] 与 [`check_resample_ratio`]。
///
/// 1 000 是这样选的：本 crate 的采样率域是 `(0, 768 kHz]`，而**真实音频**的最低标准采样率
/// 是 8 kHz（ITU-T G.711 的电话带宽），所以域内最宽的真实转换是 8 kHz → 768 kHz = **96×**。
/// 1 000× 是它的**十倍**余量 —— 这道闸门是安全闸门而不是实现限制（见模块文档），因此它
/// 只拒绝 `输出率 > 1 000 × 输入率` 的采样率对（在 768 kHz 的上限下就是"输入率低于
/// 768 Hz"），而那正是资源放大的唯一来源。1 000× 同时把
/// 重采样器的额外工作集钉在 `1 152 × 1 000 = 1 152 000` 帧：单声道 `4 608 000` 字节，
/// 声道数上限（64）下 `294 912 000` 字节。
pub const DEFAULT_MAX_RESAMPLE_RATIO: u64 = 1_000;

/// `seconds` 秒 × `channels` 声道交织 `f32` PCM 的精确字节数。
///
/// 全程 `u128` 中间量，因此**不会回绕**；超出 `u64` 或任一参数为 0 时返回 `None`。
/// 这是默认预算与 [`PcmBudget::for_layout`] 的**唯一**换算函数 —— "为什么是这个数"
/// 因此可以被判据按同一条公式复算。
#[must_use]
pub const fn pcm_bytes_for(seconds: u64, sample_rate: u32, channels: u16) -> Option<u64> {
    if seconds == 0 || sample_rate == 0 || channels == 0 {
        return None;
    }
    let frames = seconds as u128 * sample_rate as u128;
    let samples = frames * channels as u128;
    let bytes = samples * 4; // 交织 f32，每个样本 4 字节
    if bytes > u64::MAX as u128 {
        return None;
    }
    Some(bytes as u64)
}

/// 一次解码的资源预算（**安全闸门**）。
///
/// 这是 `HD-24` 的落地形态：改建前的两个写死常量被删除，所有使用点都必须拿到一个
/// 显式的预算值。默认值见 [`PcmBudget::default`]（由产品要求推导，见模块文档）。
///
/// 语义约定：
/// - 六道上限（输入字节 / PCM 字节 / 声道数 / 采样率 / 时长 / 重采样比例）**各自独立**生效，
///   且判定全部发生在**分配之前**；
/// - 判定是闭区间：恰好等于上限**通过**，超出一个单位即 [`LimitViolation`]；
/// - 预算为 0 是合法的（等价于"拒绝一切非空资产"），用于调用方主动收紧。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmBudget {
    /// 单个输入容器（磁盘文件/内存切片）的字节上限。
    pub max_input_bytes: u64,
    /// 解码后**交织 f32 PCM** 的字节上限。
    pub max_pcm_bytes: u64,
    /// 声道数上限。
    pub max_channels: u16,
    /// 采样率上限 (Hz)。
    pub max_sample_rate: u32,
    /// 单次解码允许的音频时长上限（秒）。与声道数无关，因此能独立约束"低采样率 ×
    /// 少声道但极长"的素材。
    pub max_duration_secs: u64,
    /// 重采样**比例**上限（输出率 / 输入率，闭区间）。唯一判定点是
    /// [`check_resample_ratio`]，只有重采样入口会用到它。
    ///
    /// 为什么比例需要自己的一道闸门：`rubato` 的异步 sinc 在**任何**输入长度上都要按
    /// "一个处理分块 + 半个滤波器"（`CHUNK_FRAMES + SINC_LEN/2` = 1 152 个**输入**帧）
    /// 准备输出缓冲，折算到输出侧就是 `1 152 × 比例` 帧 —— 与输入帧数无关。所以比例
    /// 是资源放大的唯一来源，而它不在其他任何一道闸门的度量里：时长闸门按输出率折算
    /// （极端比例下反而"显得很短"），字节与声道闸门只管输出那一份。
    ///
    /// 默认值见 [`DEFAULT_MAX_RESAMPLE_RATIO`]；取 `u64::MAX` 等于**关掉**这道闸门。
    /// 判据 `the_resample_ratio_cap_is_closed_and_reads_the_callers_budget` 钉住它的
    /// 语义，`the_resample_ratio_cap_bounds_the_resampler_working_set` 钉住它挡下的那个资源。
    pub max_resample_ratio: u64,
}

impl PcmBudget {
    /// 逐字段构造。
    #[must_use]
    pub const fn new(
        max_input_bytes: u64,
        max_pcm_bytes: u64,
        max_channels: u16,
        max_sample_rate: u32,
        max_duration_secs: u64,
        max_resample_ratio: u64,
    ) -> Self {
        Self {
            max_input_bytes,
            max_pcm_bytes,
            max_channels,
            max_sample_rate,
            max_duration_secs,
            max_resample_ratio,
        }
    }

    /// 交织 `f32` 样本总数上限（由 [`PcmBudget::max_pcm_bytes`] 换算）。
    #[must_use]
    pub const fn interleaved_samples_limit(&self) -> u64 {
        self.max_pcm_bytes / 4
    }

    /// 时长闸门在 `sample_rate` Hz 下的**帧数上界**：`max_duration_secs × sample_rate`。
    ///
    /// 这是 [`check_layout`] 判定顺序里那道独立时长闸门的**唯一**数值来源。单独抽出来的
    /// 理由是"边解边判"：只按声明帧数预检、再在解完之后终检的实现，会让一个**没有声明
    /// 帧数**的输入（`declared_frames == None`）被字节预算而不是时长预算约束住 —— 于是
    /// "6 小时"这道闸门在峰值内存上等于不存在（实测：44.1 kHz 立体声下字节预算允许
    /// 约 6.53 小时，8 kHz 单声道下允许约 75 小时）。解码循环必须能逐包用同一条公式
    /// 判定，否则两处判定会各自漂移。
    ///
    /// 乘法在 `u128` 里做，因此**不会回绕**；乘积超出 `u64` 表示范围时返回 `u64::MAX`
    /// （等价于"这道闸门在此采样率下不构成约束"，与 [`check_layout`] 在 `u128` 里比较
    /// 的结论一致）。
    #[must_use]
    pub const fn max_duration_frames(&self, sample_rate: u32) -> u64 {
        let max_frames = self.max_duration_secs as u128 * sample_rate as u128;
        if max_frames > u64::MAX as u128 {
            u64::MAX
        } else {
            max_frames as u64
        }
    }

    /// 由**产品要求**反推一个预算：至少容纳 `seconds` 秒的 `sample_rate` Hz
    /// `channels` 声道素材。
    ///
    /// 返回的预算**恰好**允许该布局（声道数/采样率/时长三个闸门都设成要求值，
    /// PCM 字节数设成要求值的精确字节数），因此"要求内的素材一定进得来"是构造上
    /// 成立的，而不是靠调数字。任一参数为 0 或字节数超出 `u64` 时返回 `None`。
    ///
    /// ⚠ **比例上限与会话要求无关，恒为全局默认**（[`DEFAULT_MAX_RESAMPLE_RATIO`]，
    /// `2026-10-10` 由裁决 `R32` 写明）。这个字段**不**随 `seconds` / `sample_rate` /
    /// `channels` 推导：它是一个全局安全帽，按调用方包络放宽会削弱资源保护
    /// （[ARCH-SEC-003]）。因此本函数"恰好允许该布局"这句话覆盖的是**单一采样率上的
    /// 素材**，而**不**承诺"包络内的任意采样率对都能转换" —— 例如
    /// `for_layout(1, 768_000, 1)` 放行 1 Hz 与 768 kHz 两端各自的素材，但拒绝
    /// 1 Hz → 768 kHz 这一次转换（比例 768 000× 超过 1 000×）。判据
    /// `for_layout_pins_a_ratio_cap_that_is_narrower_than_its_own_rate_envelope`
    /// 逐项钉住这个事实，并检查本段文字仍在。
    #[must_use]
    pub fn for_layout(seconds: u64, sample_rate: u32, channels: u16) -> Option<Self> {
        let pcm_bytes = pcm_bytes_for(seconds, sample_rate, channels)?;
        Some(Self {
            max_input_bytes: pcm_bytes.saturating_add(CONTAINER_OVERHEAD_BYTES),
            max_pcm_bytes: pcm_bytes,
            max_channels: channels,
            max_sample_rate: sample_rate,
            max_duration_secs: seconds,
            max_resample_ratio: DEFAULT_MAX_RESAMPLE_RATIO,
        })
    }
}

impl Default for PcmBudget {
    /// 由两条产品要求推导的默认预算（见模块文档）：
    /// 96 kHz 立体声 **3 小时** ∪ 96 kHz 8 声道 **30 分钟**，取字节数较大者。
    ///
    /// 声道数/采样率/时长三个闸门取 [`DEFAULT_MAX_CHANNELS`] /
    /// [`DEFAULT_MAX_SAMPLE_RATE`] / [`DEFAULT_MAX_DURATION_SECS`]（对上述两条要求都是
    /// 宽松的，因此不会误伤要求内的素材）。
    ///
    /// 两个要求都是编译期常量，`pcm_bytes_for` 在此不可能失败；真失败了会在第一条
    /// 判据上炸，而不是静默退化成一个零预算。
    fn default() -> Self {
        let reference = pcm_bytes_for(
            DEFAULT_REFERENCE_SECONDS,
            DEFAULT_REFERENCE_RATE,
            DEFAULT_REFERENCE_CHANNELS,
        )
        .expect("3 h @ 96 kHz stereo must fit in u64 bytes");
        let multitrack = pcm_bytes_for(
            DEFAULT_MULTITRACK_SECONDS,
            DEFAULT_REFERENCE_RATE,
            DEFAULT_MULTITRACK_CHANNELS,
        )
        .expect("30 min @ 96 kHz 8 ch must fit in u64 bytes");
        let pcm_bytes = if reference > multitrack {
            reference
        } else {
            multitrack
        };
        Self {
            max_input_bytes: pcm_bytes.saturating_add(CONTAINER_OVERHEAD_BYTES),
            max_pcm_bytes: pcm_bytes,
            max_channels: DEFAULT_MAX_CHANNELS,
            max_sample_rate: DEFAULT_MAX_SAMPLE_RATE,
            max_duration_secs: DEFAULT_MAX_DURATION_SECS,
            max_resample_ratio: DEFAULT_MAX_RESAMPLE_RATIO,
        }
    }
}

/// 重采样所用的 sinc 滤波器长度（抽头数）。
///
/// 与 `rubato` 构造参数里的字面量 `256` 是同**一个**被钉死的值：字面量写在调用点
/// 是为了避免与上游参数位宽（`usize` vs `u32`）耦合，这里的常量负责让判据能引用它、
/// 并在判据里与上游参数对上（见 `resample.rs` 的配置判据）。
pub const SINC_LEN: u64 = 256;

/// 长度契约的相对容差：百万分之 1 000，也就是 0.1%。
///
/// `PPM` 是 "parts per million"：本常量的单位就是 ppm，换成百分比要再除以 10 000。
/// 它与 [`MIN_LEN_TOLERANCE_FRAMES`] 一起构成 [`resample_len_contract`] 的相对容差项
/// （`max(理想输出帧数 × 0.1%, 8 帧)`），再按 [`SHORT_CLIP_THRESHOLD_FRAMES`] 决定是否
/// 追加 [`SINC_LEN`]。
pub const LEN_TOLERANCE_PPM: u64 = 1_000;

/// 长度契约的绝对容差下限（帧）。防止"短片段 + 相对容差"退化成零容差。
pub const MIN_LEN_TOLERANCE_FRAMES: u64 = 8;

/// "短片段"阈值（帧）：理想输出帧数低于此值时，重采样器被裁掉的前置延迟
/// 相对整个片段不再可忽略，契约额外放宽 [`SINC_LEN`] 帧。
///
/// 这不是"放水"，而是把 `rubato` 的真实语义写进来：延迟被裁掉之后剩下的不是
/// 一个整数帧的舍入误差，而是滤波器长度量级的边界效应。
pub const SHORT_CLIP_THRESHOLD_FRAMES: u64 = 4_096;

/// 声明的帧数与解出的帧数的默认容差：**0 帧**。
///
/// 本 crate 只启用无损编解码（PCM / FLAC）与 Ogg Vorbis（按包解，无整段延迟字段）；
/// 对 PCM/FLAC 而言容器声明的帧数必须与解出的帧数**逐帧相等**，不给容差。
pub const DURATION_TOLERANCE_FRAMES: u64 = 0;

/// 解码循环里允许的"连续不推进"包数上限。
///
/// 存在的理由是一个**实测到的上游行为**，不是假想的防御：RIFF 解封装用**声明的**
/// `data` 块长度算 `data_end_pos`（源码 `symphonia-format-riff-0.6.1/src/common.rs:475`
/// 与 `wave/mod.rs:145`），`next_packet` 又只按 `data_end_pos - pos` 判断"还有多少块"
/// （`common.rs:404-408`）。于是一个"`data` 头完整、但数据体被截断"的 WAV 会让
/// `pos` 停住不动、每轮都返回**空包**，而 `read_boxed_slice` 对 EOF 是**截短返回**
/// 而不是报错（`symphonia-core-0.6.1/src/io/mod.rs:553-584`）。
/// 结果是"解析器不报错、也不推进"——没有这道闸门，解码线程会**永远转下去**。
///
/// **2026-10-08 实测更正**：上面推出来的那条路径**没有复现**。同一形状的输入
/// （44 字节头 + 64 字节真实样本，`data` 声明 65535 字节）在三次运行里都返回
/// `Err(Io(UnexpectedEof))`，耗时 25–83 µs：symphonia 的 `MediaSourceStream` 在底层
/// 真实字节耗尽时把 EOF 报给 `next_packet`，而不是持续返回空包。把 `decode.rs` 的
/// 记账停掉后读数**逐字相同**。因此本闸门的正当性目前是"防御一个**未复现**的上游
/// 路径"，不是"已复现的挂死"；见 [`IdleGuard`] 的覆盖现状声明。原数字保留不改写。
///
/// 1024 这个数字对合法输入是极宽松的：WAV / FLAC 的音频包之间最多夹几个非音频包，
/// 不可能连续 1024 个包一帧音频都不出。而任何"不推进"的病态输入都会在 1024 轮内被拒。
///
/// **2026-10-10 起这条数字有端到端读数**：1024 个"读端收下、解码器拒收"的 FLAC 包
/// ⇒ `Err(EmptyStream)`（闭区间内侧），1025 个 ⇒ `Err(Malformed)`（越界一档）。
/// 见 [`IdleGuard`] 的覆盖现状表。
pub const MAX_IDLE_PACKETS: u32 = 1_024;

/// "解码不推进"计数器。
///
/// 纯逻辑、零依赖，因此**闸门本身的行为可以在本机单独跑**（见 `#[cfg(test)]`）。
///
/// **覆盖现状（实测，2026-10-10 起）**：本闸门**已有**端到端判据 ——
/// `decode::tests::the_idle_guard_trips_on_1025_consecutive_bad_packets`。它用"读端收下、
/// 解码器拒收"的包把闭区间钉住：连续 1 024 次不推进 ⇒ 走完流、`Err(EmptyStream)`；
/// 第 1 025 次 ⇒ `Err(Malformed("demuxer stopped making progress…"))`。
///
/// 走上这条判据的路是 2026-10-10 实测出来的，写在这里免得下一位重走：
///
/// | 构造 | 读端行为 | `bump_idle` 次数 |
/// | :--- | :--- | :--- |
/// | 帧尾 CRC-16 被破坏 | 读端自己校验 CRC-16，**跳过整个帧**、不产出包 | **0** |
/// | 子帧类型改成规范保留值 + **正确**的 CRC-16 | 帧结构自洽 ⇒ 读端照常产出包，解码器拒收 | 每帧 1 次 |
///
/// 表里第一行是反直觉的那一格：`flac_constant_with_broken_frame_crc(spec, 1025, _)` 的
/// 读数是 `EmptyStream`（1025 帧，却一次 `bump_idle` 都没有），因此"破坏 CRC"**构造不出**
/// 本闸门的输入。第二行才是可达形状（`flac_constant_with_reserved_subframes`）。
///
/// 在此之前本段的结论是"没有端到端判据"，理由是 WAV 那条路径不复现（2026-10-08）：
/// symphonia 的 RIFF/WAVE 读端在真实字节耗尽时先返回 `UnexpectedEof`
/// （44 字节头 + 64 字节样本、`data` 声明 128 / 4096 / 65535 的实测全部是
/// `Err(Io(UnexpectedEof))`），"每轮返回空包、永不推进"没有出现。原结论因此**已被取代**，
/// 但那条 WAV 读数仍然成立（它说明"为什么要换一条路构造"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IdleGuard {
    idle: u32,
}

impl IdleGuard {
    /// 全新的计数器（0 次不推进）。
    #[must_use]
    pub const fn new() -> Self {
        Self { idle: 0 }
    }

    /// 记一次"这一轮没有产出样本"，返回 `true` 表示**已经超过** [`MAX_IDLE_PACKETS`]。
    #[must_use]
    pub fn bump(&mut self) -> bool {
        self.idle = self.idle.saturating_add(1);
        self.idle > MAX_IDLE_PACKETS
    }

    /// 归零（这一轮真的产出了样本）。
    pub fn reset(&mut self) {
        self.idle = 0;
    }

    /// 当前连续不推进的包数。
    #[must_use]
    pub fn idle(self) -> u32 {
        self.idle
    }
}

/// 预算/尺寸违规。全部是"输入不可信"导致的，一律返回而非 panic。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitViolation {
    /// 输入字节数超过上限。
    InputTooLarge {
        /// 实际（或声明）的字节数。
        bytes: u64,
        /// 生效的上限。
        limit: u64,
    },
    /// 声道数为 0。
    ZeroChannels,
    /// 声道数超过上限。
    TooManyChannels {
        /// 声明的声道数。
        channels: u16,
        /// 生效的上限。
        limit: u16,
    },
    /// 采样率为 0。
    ZeroSampleRate,
    /// 采样率超过上限。
    SampleRateTooHigh {
        /// 声明的采样率。
        rate: u32,
        /// 生效的上限。
        limit: u32,
    },
    /// 时长超过预算（**与字节无关**的独立闸门）。
    DurationTooLong {
        /// 帧数。
        frames: u64,
        /// 采样率。
        sample_rate: u32,
        /// `frames / sample_rate`（整秒，仅用于报错文案）。
        seconds: u64,
        /// 生效的时长上限（秒）。
        limit_secs: u64,
    },
    /// 交织样本总数超过 PCM 字节预算（`samples × 4 > max_pcm_bytes`）。
    PcmBudgetExceeded {
        /// 帧数。
        frames: u64,
        /// 声道数。
        channels: u16,
        /// `frames × channels` 的交织样本数。
        samples: u64,
        /// 生效的样本数上限（`max_pcm_bytes / 4`）。
        limit_samples: u64,
    },
    /// `frames × channels` 在 `u64` 里溢出。
    LayoutOverflow {
        /// 帧数。
        frames: u64,
        /// 声道数。
        channels: u16,
    },
    /// 重采样**比例**超过上限（`out_rate > in_rate × limit`）。
    ///
    /// 这是一道**独立**闸门，理由见 [`PcmBudget::max_resample_ratio`]：比例是重采样工作集
    /// 的唯一放大来源，而其他五道闸门都不度量它。
    ResampleRatioTooHigh {
        /// 输入采样率。
        in_rate: u32,
        /// 输出采样率。
        out_rate: u32,
        /// 生效的比例上限（输出率 / 输入率的倍数）。
        limit: u64,
    },
    /// 向分配器申请缓冲被拒绝（`Vec::try_reserve` 失败）。
    AllocationRefused {
        /// 本次想要追加的样本数。
        samples: u64,
    },
}

impl fmt::Display for LimitViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputTooLarge { bytes, limit } => {
                write!(f, "input is {bytes} bytes, over the {limit}-byte cap")
            }
            Self::ZeroChannels => f.write_str("stream declares zero channels"),
            Self::TooManyChannels { channels, limit } => {
                write!(
                    f,
                    "stream declares {channels} channels, over the {limit} cap"
                )
            }
            Self::ZeroSampleRate => f.write_str("stream declares a zero sample rate"),
            Self::SampleRateTooHigh { rate, limit } => {
                write!(f, "stream declares {rate} Hz, over the {limit} Hz cap")
            }
            Self::DurationTooLong {
                frames,
                sample_rate,
                seconds,
                limit_secs,
            } => write!(
                f,
                "{frames} frames at {sample_rate} Hz is {seconds} s of audio, \
                 over the {limit_secs}-second duration cap"
            ),
            Self::PcmBudgetExceeded {
                frames,
                channels,
                samples,
                limit_samples,
            } => write!(
                f,
                "{frames} frames x {channels} channels = {samples} interleaved samples \
                 ({} bytes of f32 PCM), over the {limit_samples}-sample / {}-byte PCM budget",
                samples.saturating_mul(4),
                limit_samples.saturating_mul(4)
            ),
            Self::LayoutOverflow { frames, channels } => write!(
                f,
                "frame/channel product overflows: {frames} frames x {channels} channels"
            ),
            Self::ResampleRatioTooHigh {
                in_rate,
                out_rate,
                limit,
            } => write!(
                f,
                "resampling {in_rate} Hz to {out_rate} Hz is over the {limit}x \
                 output/input sample-rate ratio cap"
            ),
            Self::AllocationRefused { samples } => {
                write!(f, "allocator refused a buffer for {samples} more samples")
            }
        }
    }
}

impl Error for LimitViolation {}

/// 重采样长度契约：解出的输出帧数必须落在 `[min, max]` 闭区间内。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LenContract {
    /// 允许的最小输出帧数。
    pub min: u64,
    /// 允许的最大输出帧数。
    pub max: u64,
    /// `⌊输入帧数 × 输出率 / 输入率⌋`。
    pub ideal_floor: u64,
    /// `⌈输入帧数 × 输出率 / 输入率⌉`。
    pub ideal_ceil: u64,
}

/// 长度契约违约。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LenContractViolation {
    /// 采样率为 0，比例无定义。
    UndefinedRatio {
        /// 输入采样率。
        in_rate: u32,
        /// 输出采样率。
        out_rate: u32,
    },
    /// 采样率都非 0，但理想输出帧数连 `u64` 都放不下 ⇒ 契约无表示。
    ///
    /// 与 [`Self::UndefinedRatio`] 分开的理由：两者的**原因与处置都不同**。比例无定义是
    /// "调用方给的采样率非法"，改采样率即可；这一条是"帧数 × 比例超出了 `u64` 量程"，
    /// 采样率完全合法。把后者报成"ratio undefined"是在错误文案里说假话，而本 crate 的
    /// 错误是要进 MCP 响应体的（`decodeError` 分类），所以文案必须指对原因。
    ///
    /// 判定仍然**只拒**：无法表示契约时返回错误，绝不退化成"放行"。
    Unrepresentable {
        /// 输入帧数。
        input_frames: u64,
        /// 输入采样率。
        in_rate: u32,
        /// 输出采样率。
        out_rate: u32,
    },
    /// 实际输出帧数落在契约区间之外。
    OutsideBounds {
        /// 实际输出帧数。
        produced: u64,
        /// 生效的契约。
        contract: LenContract,
    },
}

impl fmt::Display for LenContractViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UndefinedRatio { in_rate, out_rate } => {
                write!(f, "resample ratio undefined: {in_rate} Hz -> {out_rate} Hz")
            }
            Self::Unrepresentable {
                input_frames,
                in_rate,
                out_rate,
            } => write!(
                f,
                "resample length contract for {input_frames} frames ({in_rate} Hz -> \
                 {out_rate} Hz) does not fit in u64 frames"
            ),
            Self::OutsideBounds { produced, contract } => write!(
                f,
                "resampler produced {produced} frames, outside the contract {}..={} \
                 (ideal {}..={})",
                contract.min, contract.max, contract.ideal_floor, contract.ideal_ceil
            ),
        }
    }
}

impl Error for LenContractViolation {}

/// 校验输入字节数是否在预算内。
///
/// # Errors
///
/// 超过 [`PcmBudget::max_input_bytes`] 时返回 [`LimitViolation::InputTooLarge`]。
pub fn check_input_len(bytes: u64, budget: &PcmBudget) -> Result<(), LimitViolation> {
    if bytes > budget.max_input_bytes {
        return Err(LimitViolation::InputTooLarge {
            bytes,
            limit: budget.max_input_bytes,
        });
    }
    Ok(())
}

/// 精确计算 `frames × channels`；`u64` 放不下时返回错误，**绝不回绕**。
///
/// 这里**不查预算** —— 它没有 `budget` 参数，因此它只回答"乘法本身是否成立"。要判
/// "这个尺寸是否在预算内"请用 [`check_layout`]（它内部就用本函数算样本数，再与
/// [`PcmBudget::interleaved_samples_limit`] 比较）。
///
/// # Errors
///
/// 只有一种：[`LimitViolation::LayoutOverflow`]（乘积超出 `u64`）。
pub fn interleaved_samples(frames: u64, channels: u16) -> Result<u64, LimitViolation> {
    frames
        .checked_mul(u64::from(channels))
        .ok_or(LimitViolation::LayoutOverflow { frames, channels })
}

/// 校验一次解码的布局是否在预算内。
///
/// 判定顺序（**每一道闸门各自独立**，任何一道都能单独把输入挡下）：
/// 声道数为 0 → 声道数超限 → 采样率为 0 → 采样率超限 → 时长超限 → PCM 字节超预算。
/// 全部用 `u128`/`u64` 精确整数比较，因此不会先溢出再判定。
///
/// 时长闸门那一步用的是 [`PcmBudget::max_duration_frames`]，**不是**就地写一遍乘法：
/// 解码循环在只知道采样率、还不知道容器声明帧数时必须能逐包判同一道闸门（见
/// [`crate::decode::decode_source`] 的实测理由）。
///
/// # Errors
///
/// 见 [`LimitViolation`]。
pub fn check_layout(
    channels: u16,
    sample_rate: u32,
    frames: u64,
    budget: &PcmBudget,
) -> Result<(), LimitViolation> {
    if channels == 0 {
        return Err(LimitViolation::ZeroChannels);
    }
    if channels > budget.max_channels {
        return Err(LimitViolation::TooManyChannels {
            channels,
            limit: budget.max_channels,
        });
    }
    if sample_rate == 0 {
        return Err(LimitViolation::ZeroSampleRate);
    }
    if sample_rate > budget.max_sample_rate {
        return Err(LimitViolation::SampleRateTooHigh {
            rate: sample_rate,
            limit: budget.max_sample_rate,
        });
    }
    // 时长闸门：`frames > max_duration_secs × sample_rate` 即超过 [`PcmBudget::max_duration_secs`]
    // 秒（整数精确，闭区间：恰好等于上限通过）。乘法在 `u128` 里做，不会回绕；数值来自
    // [`PcmBudget::max_duration_frames`]，因此解码循环里的逐包判定用的是**同一条**公式。
    let max_frames = budget.max_duration_frames(sample_rate);
    if frames > max_frames {
        return Err(LimitViolation::DurationTooLong {
            frames,
            sample_rate,
            seconds: frames / u64::from(sample_rate),
            limit_secs: budget.max_duration_secs,
        });
    }
    let samples = interleaved_samples(frames, channels)?;
    let limit_samples = budget.interleaved_samples_limit();
    if samples > limit_samples {
        return Err(LimitViolation::PcmBudgetExceeded {
            frames,
            channels,
            samples,
            limit_samples,
        });
    }
    Ok(())
}

/// 校验重采样**比例**（`out_rate / in_rate`）不超过 [`PcmBudget::max_resample_ratio`]。
///
/// 量的是"输出率是输入率的几倍"。为什么必须有这道闸门：`rubato` 的异步 sinc 在**任何**
/// 输入长度上都要按"一个处理分块 + 半个滤波器"（`CHUNK_FRAMES + SINC_LEN/2` = 1 152 个
/// **输入**帧）准备输出缓冲，折算到输出侧就是 `1 152 × 比例` 帧，与输入帧数无关。于是
/// 那 3.54 GB 的读数（1 帧、1 Hz → 768 kHz）完全由比例决定，而 [`check_layout`] 的四道
/// 闸门一道都不度量它：时长闸门按**输出率**折算帧数（1 153 秒，远低于 6 小时）。
///
/// 判定是**闭区间**：`out_rate == in_rate × 上限` 通过。比较在 `u128` 里做，因此不会回绕。
/// 本函数只读两个声明出来的采样率，所以判定发生在**任何分配之前**，也不依赖 `rubato`
/// 的任何行为。它**只拒绝**：被放行的采样率对上，重采样输出与这道闸门存在之前逐位相同。
///
/// # Errors
///
/// - [`LimitViolation::ZeroSampleRate`]：任一采样率为 0（比例无定义）；
/// - [`LimitViolation::ResampleRatioTooHigh`]：`out_rate > in_rate × 上限`。
pub fn check_resample_ratio(
    in_rate: u32,
    out_rate: u32,
    budget: &PcmBudget,
) -> Result<(), LimitViolation> {
    if in_rate == 0 || out_rate == 0 {
        return Err(LimitViolation::ZeroSampleRate);
    }
    // `u32 × u64` 在 `u128` 里最多到 2^96，不会回绕。
    if u128::from(out_rate) > u128::from(in_rate) * u128::from(budget.max_resample_ratio) {
        return Err(LimitViolation::ResampleRatioTooHigh {
            in_rate,
            out_rate,
            limit: budget.max_resample_ratio,
        });
    }
    Ok(())
}

/// 计算重采样的长度契约（精确有理数运算，零浮点）。
///
/// 语义：输出帧数 ≈ `输入帧数 × out_rate / in_rate`，容差为
/// `max(理想值 × 0.1%, 8 帧)`；当理想输出帧数小于 [`SHORT_CLIP_THRESHOLD_FRAMES`]
/// 时再额外放宽 [`SINC_LEN`] 帧（被裁掉的前置延迟相对短片段不可忽略）。
///
/// **零帧输入是精确的，不享受任何容差**：输入 0 帧 ⇒ 契约恰为 `[0, 0]`。容差描述的是
/// "有限长滤波器把前置延迟裁掉之后剩下的边界效应"，而 0 帧输入没有任何边界可谈；
/// 没有这条特例，容差公式会给 `[0, SINC_LEN + 8]` —— 一个"零帧输入最多能产出 264 帧"
/// 的区间，等于把契约放宽到能接受一个不可能正确的输出。实现侧同样如此：
/// [`crate::resample::resample_interleaved_with_budget`] 对 0 帧输入直接返回空。
///
/// 返回 `None` 有两种原因，二者的处置不同：
/// - 任一采样率为 0（比例无定义）—— 调用方该改采样率；
/// - 采样率都非 0，但理想输出帧数（及其容差）放不进 `u64` —— 输入帧数与比例的组合
///   超出量程，见 [`LenContractViolation::Unrepresentable`]。
///
/// 两种 `None` 都不会退化成"放行"：[`check_resampled_len`] 一律返回错误。
#[must_use]
pub fn resample_len_contract(
    input_frames: u64,
    out_rate: u32,
    in_rate: u32,
) -> Option<LenContract> {
    if in_rate == 0 || out_rate == 0 {
        return None;
    }
    // 0 帧输入 ⇒ 恰好 0 帧输出。见上面的文档：容差在这里没有物理含义。
    if input_frames == 0 {
        return Some(LenContract {
            min: 0,
            max: 0,
            ideal_floor: 0,
            ideal_ceil: 0,
        });
    }
    let num = u128::from(out_rate);
    let den = u128::from(in_rate);
    let ideal = u128::from(input_frames) * num;
    let floor = ideal / den;
    let ceil = ideal.div_ceil(den);
    let relative = floor * u128::from(LEN_TOLERANCE_PPM) / 1_000_000;
    let mut slack = relative.max(u128::from(MIN_LEN_TOLERANCE_FRAMES));
    if floor < u128::from(SHORT_CLIP_THRESHOLD_FRAMES) {
        slack += u128::from(SINC_LEN);
    }
    Some(LenContract {
        min: u64::try_from(floor.saturating_sub(slack)).ok()?,
        max: u64::try_from(ceil + slack).ok()?,
        ideal_floor: u64::try_from(floor).ok()?,
        ideal_ceil: u64::try_from(ceil).ok()?,
    })
}

/// 校验重采样实际输出的帧数是否满足长度契约。
///
/// # Errors
///
/// - [`LenContractViolation::UndefinedRatio`]：任一采样率为 0；
/// - [`LenContractViolation::Unrepresentable`]：采样率合法，但契约放不进 `u64`；
/// - [`LenContractViolation::OutsideBounds`]：`produced` 落在
///   [`resample_len_contract`] 给出的区间之外。
pub fn check_resampled_len(
    input_frames: u64,
    out_rate: u32,
    in_rate: u32,
    produced: u64,
) -> Result<LenContract, LenContractViolation> {
    // 先分开"比例无定义"，再让 `resample_len_contract` 的 `None` 只剩"契约无表示"一种
    // 含义 —— 否则错误文案会把一条合法的采样率组合说成 "ratio undefined"。
    if in_rate == 0 || out_rate == 0 {
        return Err(LenContractViolation::UndefinedRatio { in_rate, out_rate });
    }
    let contract = resample_len_contract(input_frames, out_rate, in_rate).ok_or(
        LenContractViolation::Unrepresentable {
            input_frames,
            in_rate,
            out_rate,
        },
    )?;
    if produced < contract.min || produced > contract.max {
        return Err(LenContractViolation::OutsideBounds { produced, contract });
    }
    Ok(contract)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    /// 判据 (时长闸门的单值来源)：`max_duration_frames` 必须与"逐包边解边判"用到的
    /// 数值**逐位相同**，且与 [`check_layout`] 的闭区间边界一致。
    ///
    /// 为什么单独钉：解码循环用它做逐包判定，而 `check_layout` 用它做终检。两处一旦各自
    /// 算一遍，就会出现"循环放行的帧数被终检拒绝"这种只在长素材上暴露的分歧。
    ///
    /// 注入：把 `max_duration_frames` 的返回改成 `u64::MAX`（等于"这道闸门永不跳闸"）
    /// ⇒ 本判据的第一条与第三条断言红；把 `check_layout` 的 `>` 改成 `>=` ⇒ 第二条红。
    #[test]
    fn the_duration_gate_has_exactly_one_numeric_source() {
        let budget = PcmBudget {
            max_duration_secs: 6 * 60 * 60,
            ..PcmBudget::default()
        };
        // 与逐包判定同值。
        assert_eq!(budget.max_duration_frames(8_000), 6 * 60 * 60 * 8_000);
        assert_eq!(budget.max_duration_frames(48_000), 6 * 60 * 60 * 48_000);
        // 闭区间：恰好等于上界通过，多一帧即拒（终检一侧）。为了只让**时长**这一道
        // 闸门生效，其余四道放宽 —— 默认预算在 96 kHz **立体声**下是字节预算先跳闸
        // （3 小时），拿默认预算判时长边界会量错闸门。
        let by_time_only = PcmBudget {
            max_input_bytes: u64::MAX,
            max_pcm_bytes: u64::MAX,
            max_channels: u16::MAX,
            max_sample_rate: u32::MAX,
            max_duration_secs: budget.max_duration_secs,
            max_resample_ratio: u64::MAX,
        };
        let exact = 6 * 60 * 60 * 96_000;
        assert_eq!(by_time_only.max_duration_frames(96_000), exact);
        assert_eq!(check_layout(2, 96_000, exact, &by_time_only), Ok(()));
        let err = check_layout(2, 96_000, exact + 1, &by_time_only).unwrap_err();
        assert!(
            matches!(err, LimitViolation::DurationTooLong { .. }),
            "got {err}"
        );
        // 逐包判定一侧：用同一个数，超一帧就返回。
        assert!(exact <= by_time_only.max_duration_frames(96_000));
        assert!(exact + 1 > by_time_only.max_duration_frames(96_000));

        // 乘积超出 u64 时**饱和**而不是回绕（回绕会把上界变成一个小数字，于是把
        // 合法素材误拒 —— 那是 fail-closed 的反面：一道只会误报的闸门）。
        let absurd = PcmBudget {
            max_duration_secs: u64::MAX,
            ..PcmBudget::default()
        };
        assert_eq!(absurd.max_duration_frames(u32::MAX), u64::MAX);
        // 采样率为 0 时上界是 0：任何非零帧数都被时长闸门挡下。
        assert_eq!(budget.max_duration_frames(0), 0);
    }

    /// 判据 (时长闸门的单值来源)：`max_duration_frames` 与逐包判定同值。
    ///
    /// 覆盖"规范采样率 + 极端采样率"两类：`1 Hz` 与 `768 kHz` 都必须在同一条公式上
    /// 得到闭区间边界，且 `u64::MAX` 上界不得 panic（`limit + 1` 会回绕，因此那里只断言
    /// 不 panic 的取值）。
    #[test]
    fn max_duration_frames_matches_the_layout_gate_at_the_boundary() {
        let budget = PcmBudget::default();
        let by_time_only = PcmBudget {
            max_input_bytes: u64::MAX,
            max_pcm_bytes: u64::MAX,
            max_channels: u16::MAX,
            max_sample_rate: u32::MAX,
            max_duration_secs: budget.max_duration_secs,
            max_resample_ratio: u64::MAX,
        };
        for &rate in &[1u32, 8_000, 44_100, 48_000, 96_000, 768_000] {
            let limit = budget.max_duration_frames(rate);
            assert_eq!(check_layout(1, rate, limit, &by_time_only), Ok(()));
            if limit < u64::MAX {
                let err = check_layout(1, rate, limit + 1, &by_time_only).unwrap_err();
                assert!(
                    matches!(err, LimitViolation::DurationTooLong { .. }),
                    "rate {rate}: got {err}"
                );
            }
        }
    }

    /// 判据 ①（HD-24 核心）：默认预算是**产品要求的函数**，不是拍出来的数字。
    ///
    /// 注入"默认上限 = `u64::MAX`"会在这里的红 —— 这是本线最重要的一条判据。
    #[test]
    fn default_budget_is_recomputed_from_the_product_requirements() {
        let reference = pcm_bytes_for(
            DEFAULT_REFERENCE_SECONDS,
            DEFAULT_REFERENCE_RATE,
            DEFAULT_REFERENCE_CHANNELS,
        )
        .expect("3 h @ 96 kHz stereo");
        let multitrack = pcm_bytes_for(
            DEFAULT_MULTITRACK_SECONDS,
            DEFAULT_REFERENCE_RATE,
            DEFAULT_MULTITRACK_CHANNELS,
        )
        .expect("30 min @ 96 kHz 8 ch");
        let budget = PcmBudget::default();
        assert_eq!(budget.max_pcm_bytes, reference.max(multitrack));
        assert_eq!(
            budget.max_input_bytes,
            budget.max_pcm_bytes + CONTAINER_OVERHEAD_BYTES
        );
        assert_eq!(budget.interleaved_samples_limit(), budget.max_pcm_bytes / 4);
        assert_eq!(budget.max_channels, DEFAULT_MAX_CHANNELS);
        assert_eq!(budget.max_sample_rate, DEFAULT_MAX_SAMPLE_RATE);
        assert_eq!(budget.max_duration_secs, DEFAULT_MAX_DURATION_SECS);

        // "为什么是这个数" 的算术关系（同一预算折算成时长，可逐条复算）：
        // 96 kHz 立体声 3 小时 ⇔ 96 kHz 8 声道 45 分钟 ⇔ 48 kHz 立体声 6 小时。
        let stereo_frames = budget.interleaved_samples_limit() / 2;
        assert_eq!(
            stereo_frames / u64::from(DEFAULT_REFERENCE_RATE),
            3 * 60 * 60,
            "the default must still cover a 3-hour 96 kHz stereo session"
        );
        let multitrack_frames = budget.interleaved_samples_limit() / 8;
        assert_eq!(
            multitrack_frames / u64::from(DEFAULT_REFERENCE_RATE),
            45 * 60,
            "the same budget is 45 minutes of 96 kHz 8-channel (was 11.6 minutes)"
        );
        let cd_frames = budget.interleaved_samples_limit() / 2;
        // 44.1 kHz 立体声：字节上允许约 6.53 小时 ⇒ 6 小时的时长闸门**先**跳闸。
        assert_eq!(cd_frames / 44_100, 23_510);
        assert!(6 * 60 * 60 * 44_100 < cd_frames);
        assert_eq!(
            48_000 * 6 * 60 * 60,
            budget.interleaved_samples_limit() / 2,
            "48 kHz stereo: the byte budget is exactly 6 hours, so the 6-hour \
             duration gate is the binding one at that layout"
        );

        // 结构判据：预算必须是"可用的有限值"（用 u128/u64 中间量都算得出来）。
        assert_ne!(
            budget.max_pcm_bytes,
            2 * GIB,
            "the old hard-coded 2 GiB is gone"
        );
        assert!(budget.max_pcm_bytes > 0 && budget.max_pcm_bytes <= u64::MAX / 8);
        assert!(
            budget
                .max_duration_secs
                .checked_mul(u64::from(budget.max_sample_rate))
                .is_some()
        );
    }

    /// 判据 ①（构造侧）：`for_layout` 必须**恰好**容纳它声明的布局。
    #[test]
    fn for_layout_admits_exactly_the_requirement_it_was_derived_from() {
        let budget = PcmBudget::for_layout(3 * 60 * 60, 96_000, 2).expect("a 3-hour stereo layout");
        assert_eq!(
            budget.max_pcm_bytes,
            pcm_bytes_for(3 * 60 * 60, 96_000, 2).unwrap()
        );
        let frames = 3 * 60 * 60 * 96_000;
        assert_eq!(check_layout(2, 96_000, frames, &budget), Ok(()));
        // `for_layout` 把三道闸门都设在要求值上，因此"多一帧"命中的是**先判定**的那一道
        // （时长）；要单独验证字节闸门的闭区间，把时长放宽即可。
        assert!(matches!(
            check_layout(2, 96_000, frames + 1, &budget),
            Err(LimitViolation::DurationTooLong { .. } | LimitViolation::PcmBudgetExceeded { .. })
        ));
        let bytes_bind_only = PcmBudget {
            max_duration_secs: 24 * 60 * 60,
            ..budget
        };
        assert_eq!(check_layout(2, 96_000, frames, &bytes_bind_only), Ok(()));
        assert_eq!(
            check_layout(2, 96_000, frames + 1, &bytes_bind_only),
            Err(LimitViolation::PcmBudgetExceeded {
                frames: frames + 1,
                channels: 2,
                samples: (frames + 1) * 2,
                limit_samples: budget.interleaved_samples_limit(),
            })
        );
        // 退化参数不产生预算，而不是产生一个"几乎放行一切"的预算。
        assert_eq!(PcmBudget::for_layout(0, 96_000, 2), None);
        assert_eq!(PcmBudget::for_layout(3_600, 0, 2), None);
        assert_eq!(PcmBudget::for_layout(3_600, 96_000, 0), None);
        assert_eq!(PcmBudget::for_layout(u64::MAX, 768_000, 64), None);
        assert_eq!(pcm_bytes_for(0, 96_000, 2), None);
        assert_eq!(pcm_bytes_for(u64::MAX, 768_000, 64), None);
    }

    /// 判据 ③：输入字节闸门是**闭区间** —— 恰好等于上限必须通过。
    ///
    /// 注入 `>` → `>=` 会在这里先红。
    #[test]
    fn input_byte_budget_is_enforced() {
        let budget = PcmBudget::new(1_024, 4_096, 8, 96_000, 60, DEFAULT_MAX_RESAMPLE_RATIO);
        assert_eq!(check_input_len(0, &budget), Ok(()));
        assert_eq!(check_input_len(1_024, &budget), Ok(()));
        assert_eq!(
            check_input_len(1_025, &budget),
            Err(LimitViolation::InputTooLarge {
                bytes: 1_025,
                limit: 1_024,
            })
        );
        // 默认预算同口径。
        let default = PcmBudget::default();
        assert_eq!(check_input_len(default.max_input_bytes, &default), Ok(()));
        assert!(check_input_len(default.max_input_bytes + 1, &default).is_err());
    }

    /// 判据 ⑤：声道数 / 采样率 / 时长 / PCM 字节四道闸门**各自独立**生效。
    ///
    /// 每条断言只让**一道**闸门变紧，其余三道都宽到不可能触发 —— 因此红了就只可能是
    /// 那一道。
    #[test]
    fn every_budget_gate_trips_on_its_own() {
        let wide = PcmBudget::new(
            u64::MAX,
            !3u64,
            DEFAULT_MAX_CHANNELS,
            DEFAULT_MAX_SAMPLE_RATE,
            u64::MAX / u64::from(DEFAULT_MAX_SAMPLE_RATE),
            DEFAULT_MAX_RESAMPLE_RATIO,
        );
        assert_eq!(check_layout(8, 96_000, 96_000, &wide), Ok(()));

        // 声道数闸门。
        let channels_only = PcmBudget {
            max_channels: 2,
            ..wide
        };
        assert_eq!(check_layout(2, 48_000, 1, &channels_only), Ok(()));
        assert_eq!(
            check_layout(3, 48_000, 1, &channels_only),
            Err(LimitViolation::TooManyChannels {
                channels: 3,
                limit: 2
            })
        );
        // 0 声道与"超上限"是两件事：前者是畸形声明，后者是预算拒绝。
        assert_eq!(
            check_layout(0, 48_000, 1, &channels_only),
            Err(LimitViolation::ZeroChannels)
        );

        // 采样率闸门。
        let rate_only = PcmBudget {
            max_sample_rate: 48_000,
            ..wide
        };
        assert_eq!(check_layout(2, 48_000, 1, &rate_only), Ok(()));
        assert_eq!(
            check_layout(2, 48_001, 1, &rate_only),
            Err(LimitViolation::SampleRateTooHigh {
                rate: 48_001,
                limit: 48_000
            })
        );
        assert_eq!(
            check_layout(2, 0, 1, &rate_only),
            Err(LimitViolation::ZeroSampleRate)
        );

        // 时长闸门：字节宽到用不完，唯一可能红的就是时长。
        let duration_only = PcmBudget {
            max_duration_secs: 1,
            ..wide
        };
        assert_eq!(check_layout(1, 48_000, 48_000, &duration_only), Ok(()));
        assert_eq!(
            check_layout(1, 48_000, 48_001, &duration_only),
            Err(LimitViolation::DurationTooLong {
                frames: 48_001,
                sample_rate: 48_000,
                seconds: 1,
                limit_secs: 1
            })
        );
        // 高采样率下同一秒数也是"恰好通过"（时长闸门与采样率无关）。
        let duration_at_96k = PcmBudget {
            max_duration_secs: 1,
            ..wide
        };
        assert_eq!(check_layout(1, 96_000, 96_000, &duration_at_96k), Ok(()));
        assert!(check_layout(1, 96_000, 96_001, &duration_at_96k).is_err());

        // PCM 字节闸门：声道/采样率/时长都宽松，只有字节预算被卡到 1 个样本。
        let bytes_only = PcmBudget {
            max_pcm_bytes: 4,
            ..wide
        };
        assert_eq!(check_layout(1, 48_000, 1, &bytes_only), Ok(()));
        assert_eq!(
            check_layout(1, 48_000, 2, &bytes_only),
            Err(LimitViolation::PcmBudgetExceeded {
                frames: 2,
                channels: 1,
                samples: 2,
                limit_samples: 1
            })
        );
    }

    /// 判据 ⑥：重采样**比例**闸门（[`check_resample_ratio`]）闭区间、独立、且读调用方的预算。
    ///
    /// 为什么它必须是一道**独立**闸门：比例不在其他五道闸门的度量里 —— `check_layout` 拿到
    /// 的是**输出**那一份（帧数、声道数、采样率、时长），而放大项 `1 152 × 比例` 与输入帧数
    /// 无关（见 `crate::resample` 的两条读数判据）。本模块是纯逻辑层，因此这里的断言只用
    /// `u32`/`u64` 算术，不碰 `rubato`。
    #[test]
    fn the_resample_ratio_cap_is_closed_and_reads_the_callers_budget() {
        let default = PcmBudget::default();
        assert_eq!(default.max_resample_ratio, DEFAULT_MAX_RESAMPLE_RATIO);
        assert_eq!(default.max_resample_ratio, 1_000);

        // 闭区间：恰好等于上限通过，低一档的输入率即拒。
        assert_eq!(
            check_resample_ratio(768, DEFAULT_MAX_SAMPLE_RATE, &default),
            Ok(())
        );
        assert_eq!(
            check_resample_ratio(767, DEFAULT_MAX_SAMPLE_RATE, &default),
            Err(LimitViolation::ResampleRatioTooHigh {
                in_rate: 767,
                out_rate: DEFAULT_MAX_SAMPLE_RATE,
                limit: 1_000
            })
        );

        // 降采样（比例 < 1）不受约束；同率（比例 1）也不受约束。
        assert_eq!(check_resample_ratio(96_000, 8_000, &default), Ok(()));
        assert_eq!(check_resample_ratio(48_000, 48_000, &default), Ok(()));

        // 0 率仍然是"比例无定义"，不是比例越界 —— 两个错误的处置不同。
        assert_eq!(
            check_resample_ratio(0, 48_000, &default),
            Err(LimitViolation::ZeroSampleRate)
        );
        assert_eq!(
            check_resample_ratio(48_000, 0, &default),
            Err(LimitViolation::ZeroSampleRate)
        );

        // 闸门读的是调用方的预算：收紧到 1 ⇒ 2 倍转换即拒；放宽到 `u64::MAX` ⇒ 闸门关掉。
        let tight = PcmBudget {
            max_resample_ratio: 1,
            ..PcmBudget::default()
        };
        assert_eq!(
            check_resample_ratio(48_000, 96_000, &tight),
            Err(LimitViolation::ResampleRatioTooHigh {
                in_rate: 48_000,
                out_rate: 96_000,
                limit: 1
            })
        );
        let off = PcmBudget {
            max_resample_ratio: u64::MAX,
            ..PcmBudget::default()
        };
        assert_eq!(
            check_resample_ratio(1, DEFAULT_MAX_SAMPLE_RATE, &off),
            Ok(())
        );

        // 类型极大值端点：比较在 `u128` 里做，因此不会回绕成"通过"。
        assert_eq!(
            check_resample_ratio(u32::MAX, u32::MAX, &default),
            Ok(()),
            "the same rate is ratio 1 regardless of magnitude"
        );
        assert_eq!(check_resample_ratio(1, 2, &default), Ok(()));
        assert_eq!(
            check_resample_ratio(1, u32::MAX, &tight),
            Err(LimitViolation::ResampleRatioTooHigh {
                in_rate: 1,
                out_rate: u32::MAX,
                limit: 1
            })
        );
    }

    /// 判据 ⑥ 的对照：默认上限必须放行**真实音频的每一个标准采样率对** —— 这道闸门是
    /// 安全闸门，不是实现限制（见模块文档的"上限的定性"）。
    #[test]
    fn the_default_resample_ratio_cap_admits_every_standard_audio_rate_pair() {
        let default = PcmBudget::default();
        // ITU-T G.711 电话带宽到 DXD 之上：本 crate 采样率域内的真实音频速率。
        let rates = [
            8_000u32,
            11_025,
            16_000,
            22_050,
            32_000,
            44_100,
            48_000,
            88_200,
            96_000,
            176_400,
            192_000,
            352_800,
            384_000,
            DEFAULT_MAX_SAMPLE_RATE,
        ];
        for &in_rate in &rates {
            for &out_rate in &rates {
                assert_eq!(
                    check_resample_ratio(in_rate, out_rate, &default),
                    Ok(()),
                    "{in_rate} Hz -> {out_rate} Hz must pass the default ratio cap"
                );
            }
        }
    }

    /// 判据 ③/④：PCM 字节闸门在**默认预算下**闭区间，且**可配置**（小预算立刻生效）。
    #[test]
    fn layout_budget_rejects_an_asset_over_the_pcm_cap() {
        let budget = PcmBudget::default();
        // 正常的一秒立体声 48k 通过。
        assert_eq!(check_layout(2, 48_000, 48_000, &budget), Ok(()));
        // 恰好用满预算通过；再多一帧就被拒绝。用 96 kHz 立体声：字节预算折算 3 小时，
        // 严格松于 6 小时的时长闸门，因此这条边界红只可能是字节闸门。
        let frames = budget.interleaved_samples_limit() / 2;
        assert_eq!(check_layout(2, 96_000, frames, &budget), Ok(()));
        match check_layout(2, 96_000, frames + 1, &budget) {
            Err(LimitViolation::PcmBudgetExceeded {
                frames: f,
                channels,
                samples,
                limit_samples,
            }) => {
                assert_eq!(f, frames + 1);
                assert_eq!(channels, 2);
                assert_eq!(samples, (frames + 1) * 2);
                assert_eq!(limit_samples, budget.interleaved_samples_limit());
            }
            other => panic!("expected PcmBudgetExceeded, got {other:?}"),
        }

        // 判据 ④：把预算调小 ⇒ 一份**小**素材也会被拒（证明上限真的可配置、真的生效）。
        let tiny = PcmBudget::new(1_024, 128, 8, 96_000, 60, DEFAULT_MAX_RESAMPLE_RATIO);
        assert_eq!(check_layout(1, 8_000, 32, &tiny), Ok(()));
        assert_eq!(
            check_layout(1, 8_000, 33, &tiny),
            Err(LimitViolation::PcmBudgetExceeded {
                frames: 33,
                channels: 1,
                samples: 33,
                limit_samples: 32
            })
        );
    }

    #[test]
    fn layout_product_overflow_is_detected_not_wrapped() {
        assert_eq!(
            interleaved_samples(u64::MAX, 2),
            Err(LimitViolation::LayoutOverflow {
                frames: u64::MAX,
                channels: 2
            })
        );
        assert_eq!(interleaved_samples(4, 2), Ok(8));
        // 乘法溢出必须被检出，而不是回绕成小值。注意：任何**现实**预算都会先被时长闸门
        // 拦下（"u64::MAX 帧"首先是一个时长问题），所以这里显式把时长闸门开到 u64::MAX
        // 才能把 `frames × channels` 的溢出路径单独逼出来 —— 这条断言钉的是
        // "调用方给了荒唐预算时，乘法仍然不回绕"。
        let overflowing = PcmBudget::new(
            u64::MAX,
            !3u64,
            64,
            u32::MAX,
            u64::MAX,
            DEFAULT_MAX_RESAMPLE_RATIO,
        );
        assert_eq!(
            check_layout(2, 48_000, u64::MAX, &overflowing),
            Err(LimitViolation::LayoutOverflow {
                frames: u64::MAX,
                channels: 2
            })
        );
    }

    /// 判据 (确定性契约的配置侧)：与预算无关的钉子必须还在。
    #[test]
    fn sinc_and_idle_constants_are_pinned() {
        assert_eq!(SINC_LEN, 256);
        // 判据 (MUST-GATE-011 防挂死): 不推进包数的闸门必须存在且非零。
        assert_eq!(MAX_IDLE_PACKETS, 1_024);
    }

    #[test]
    fn zero_rate_has_no_length_contract() {
        assert!(resample_len_contract(1_000, 0, 48_000).is_none());
        assert!(resample_len_contract(1_000, 48_000, 0).is_none());
        assert_eq!(
            check_resampled_len(1_000, 0, 48_000, 0),
            Err(LenContractViolation::UndefinedRatio {
                in_rate: 48_000,
                out_rate: 0
            })
        );
    }

    #[test]
    fn length_contract_always_contains_the_ideal_output() {
        // 判据 (ARCH-DSP-002 长度关系): 三个规范采样率两两互转，理想值必在区间内。
        let rates = [44_100u32, 48_000, 96_000];
        for &in_rate in &rates {
            for &out_rate in &rates {
                for &frames in &[0u64, 1, 7, 1_000, 44_100, 48_000, 96_000, 480_000] {
                    let c =
                        resample_len_contract(frames, out_rate, in_rate).expect("non-zero rates");
                    assert!(
                        c.min <= c.ideal_floor,
                        "contract {c:?} excludes its own floor"
                    );
                    assert!(
                        c.ideal_ceil <= c.max,
                        "contract {c:?} excludes its own ceil"
                    );
                    assert!(c.min <= c.max);
                    // 理想值本身必须被接受。
                    assert_eq!(
                        check_resampled_len(frames, out_rate, in_rate, c.ideal_floor),
                        Ok(c.clone())
                    );
                    assert_eq!(
                        check_resampled_len(frames, out_rate, in_rate, c.ideal_ceil),
                        Ok(c)
                    );
                }
            }
        }
    }

    #[test]
    fn length_contract_pins_the_three_normative_conversions() {
        // 48000 @48k -> 44.1k 恰好 44100 帧（无舍入）。
        let c = resample_len_contract(48_000, 44_100, 48_000).unwrap();
        assert_eq!((c.ideal_floor, c.ideal_ceil), (44_100, 44_100));
        assert_eq!(c.min, 44_100 - 44);
        assert_eq!(c.max, 44_100 + 44);
        // 44100 @44.1k -> 48k 恰好 48000 帧。
        let c = resample_len_contract(44_100, 48_000, 44_100).unwrap();
        assert_eq!((c.ideal_floor, c.ideal_ceil), (48_000, 48_000));
        assert_eq!(c.min, 48_000 - 48);
        assert_eq!(c.max, 48_000 + 48);
        // 48000 @48k -> 96k 恰好翻倍。
        let c = resample_len_contract(48_000, 96_000, 48_000).unwrap();
        assert_eq!((c.ideal_floor, c.ideal_ceil), (96_000, 96_000));
    }

    #[test]
    fn length_contract_rejects_an_untrimmed_delay() {
        // 判据 (预填充/延迟语义): 若前置延迟没有被裁掉，输出会多出滤波器长度量级的帧。
        let in_frames = 48_000u64;
        let produced_without_trim = 44_100 + SINC_LEN; // sinc_len/2 量级的延迟在此之上
        assert!(
            check_resampled_len(in_frames, 44_100, 48_000, produced_without_trim).is_err(),
            "a {SINC_LEN}-frame delay must fall outside the contract"
        );
        // 而正确裁剪后的结果在区间内。
        assert!(check_resampled_len(in_frames, 44_100, 48_000, 44_100).is_ok());
    }

    /// 判据 (长度契约的零帧端点)：0 帧输入 ⇒ 契约恰为 `[0, 0]`。
    ///
    /// 为什么单独钉：短片段特例会往容差里加 [`SINC_LEN`] 帧，于是 0 帧输入的区间会变成
    /// `[0, SINC_LEN + MIN_LEN_TOLERANCE_FRAMES]` —— 一个"零帧输入最多能产出 264 帧"的
    /// 区间。那是一个**不可能正确**的输出，契约不能接受它。
    ///
    /// 注入：删掉 `input_frames == 0` 的提前返回（让它继续走容差公式）⇒ 本判据的第一条
    /// 断言红，实测字面值是 `left: (0, 264, 0, 0)` / `right: (0, 0, 0, 0)`。
    #[test]
    fn a_zero_frame_input_admits_exactly_zero_output_frames() {
        for &(out_rate, in_rate) in &[(44_100u32, 48_000u32), (96_000, 48_000), (48_000, 44_100)] {
            let c = resample_len_contract(0, out_rate, in_rate).expect("non-zero rates");
            assert_eq!(
                (c.min, c.max, c.ideal_floor, c.ideal_ceil),
                (0, 0, 0, 0),
                "a zero-frame input has no boundary effect to allow for: {c:?}"
            );
            // 恰好 0 帧通过；多一帧即拒（闭区间，且不能是空判据）。
            assert_eq!(check_resampled_len(0, out_rate, in_rate, 0), Ok(c));
            assert!(matches!(
                check_resampled_len(0, out_rate, in_rate, 1),
                Err(LenContractViolation::OutsideBounds { produced: 1, .. })
            ));
        }
        // 特例**只**作用于 0 帧：1 帧输入仍然享受短片段容差（否则就是把契约焊死）。
        let one = resample_len_contract(1, 44_100, 48_000).expect("non-zero rates");
        assert!(one.max >= MIN_LEN_TOLERANCE_FRAMES + SINC_LEN);
        // 零采样率仍然没有契约 —— 与 0 帧是两件事。
        assert!(resample_len_contract(0, 0, 48_000).is_none());
        assert!(resample_len_contract(0, 48_000, 0).is_none());
    }

    /// 判据 (长度契约的两种 `None` 必须分开报)：比例无定义 ≠ 契约无表示。
    ///
    /// 采样率是合法的（1 Hz → 768 kHz），只是 `u64::MAX` 帧乘上这个比例之后连理想值都
    /// 放不进 `u64`。改建前这条路径复用 `UndefinedRatio`，于是错误文案会把一组完全合法的
    /// 采样率说成 "ratio undefined"，与 MCP 侧 `decodeError` 的分类一起把原因指错。
    ///
    /// 注入：把 `check_resampled_len` 里新加的零采样率分支删掉（回到"`None` 一律
    /// `UndefinedRatio`"）⇒ 本判据的第二条断言红：拿到 `UndefinedRatio { in_rate: 1,
    /// out_rate: 768_000 }` 而不是 `Unrepresentable`。
    #[test]
    fn an_unrepresentable_contract_is_not_reported_as_an_undefined_ratio() {
        let input_frames = u64::MAX;
        assert!(
            resample_len_contract(input_frames, 768_000, 1).is_none(),
            "the fixture must actually overflow the contract bounds"
        );
        assert_eq!(
            check_resampled_len(input_frames, 768_000, 1, 0),
            Err(LenContractViolation::Unrepresentable {
                input_frames,
                in_rate: 1,
                out_rate: 768_000,
            })
        );
        // 两种原因都**不**放行：`produced` 给什么都不行（fail-closed）。
        assert!(check_resampled_len(input_frames, 768_000, 1, u64::MAX).is_err());
        // 零采样率仍走 UndefinedRatio，且文案说的是采样率。
        let err = check_resampled_len(1_000, 0, 48_000, 0).unwrap_err();
        assert_eq!(
            err,
            LenContractViolation::UndefinedRatio {
                in_rate: 48_000,
                out_rate: 0,
            }
        );
        assert!(err.to_string().contains("ratio undefined"), "got {err}");
        // 无表示这条的文案说的是帧数量程，不是"比例无定义"。
        let overflow = check_resampled_len(input_frames, 768_000, 1, 0).unwrap_err();
        assert!(
            overflow.to_string().contains("does not fit in u64"),
            "got {overflow}"
        );
        assert!(
            !overflow.to_string().contains("ratio undefined"),
            "got {overflow}"
        );
    }

    #[test]
    fn idle_guard_trips_exactly_at_the_cap_and_resets_on_progress() {
        // 判据 (MUST-GATE-011 防挂死): 闸门必须真的会在上限处跳闸，且"有推进就清零"。
        let mut guard = IdleGuard::new();
        assert_eq!(guard.idle(), 0);
        for _ in 0..MAX_IDLE_PACKETS {
            assert!(!guard.bump(), "must not trip before the cap");
        }
        assert_eq!(guard.idle(), MAX_IDLE_PACKETS);
        assert!(guard.bump(), "must trip once past the cap");
        guard.reset();
        assert_eq!(guard.idle(), 0);
        assert!(!guard.bump());
        // 计数饱和而不是回绕（否则一个超长挂死又会从 0 开始）。
        let mut saturated = IdleGuard::new();
        for step in 0..(MAX_IDLE_PACKETS + 10) {
            assert_eq!(
                saturated.bump(),
                step >= MAX_IDLE_PACKETS,
                "step {step} tripped at the wrong moment"
            );
        }
        assert_eq!(saturated.idle(), MAX_IDLE_PACKETS + 10);
    }

    #[test]
    fn short_clips_get_an_explicit_delay_allowance() {
        // 短片段：理想输出 100 帧 < 4096，容差里必须含 SINC_LEN。
        let c = resample_len_contract(100, 44_100, 48_000).unwrap();
        assert!(c.max - c.ideal_ceil >= SINC_LEN);
        // 长片段：额外延迟放宽必须**消失**，容差只剩 0.1% 相对项。
        // 取理想输出 ≈ 199_369 帧（0.1% = 199 帧，仍小于 SINC_LEN），这样
        // "是否多给了 SINC_LEN" 可以从差值上直接读出来。
        let c = resample_len_contract(217_000, 44_100, 48_000).unwrap();
        let relative = c.ideal_floor * LEN_TOLERANCE_PPM / 1_000_000;
        assert!(
            relative < SINC_LEN,
            "test fixture must sit below the sinc allowance"
        );
        assert_eq!(c.max - c.ideal_ceil, relative);
        assert!(c.max - c.ideal_ceil < SINC_LEN);
    }

    /// 判据（类别⑤ 幂等性）：[`IdleGuard`] 是全 crate **唯一**的可变状态对象
    /// （非测试代码里的 `&mut self` 只有 `bump` 与 `reset` 两处），因此"把同一个值重复
    /// 施加到同一个对象"在本 crate 里唯一的落点就是它的 `reset`。重复复位必须与一次复位
    /// 得到**逐字段相同**的状态，且不得留下任何残余。
    ///
    /// 与 `idle_guard_trips_exactly_at_the_cap_and_resets_on_progress` 的分工：那条钉跳闸点
    /// 与"一次复位"，本条钉"重复复位"以及复位之后的计数**真的**从头起算。
    ///
    /// 注入：把 `reset` 的 `self.idle = 0` 换成 `self.idle = self.idle.saturating_sub(1)`
    /// （复位留下与次数有关的残余）⇒ 本条以 `IdleGuard { idle: 1030 }` 对
    /// `IdleGuard { idle: 1029 }` 红，读数是 `109 passed / 2 failed`（另一条红的是既有的
    /// `idle_guard_trips_exactly_at_the_cap_and_resets_on_progress`）。
    #[test]
    fn repeated_resets_leave_the_guard_in_the_same_state_as_a_single_reset() {
        let mut once = IdleGuard::new();
        let mut twice = IdleGuard::new();
        // 走到闸门之外若干次（计数是饱和加法，因此这里也会覆盖饱和之后的状态）。
        for _ in 0..(MAX_IDLE_PACKETS + 7) {
            let _ = once.bump();
            let _ = twice.bump();
        }
        assert_eq!(once.idle(), twice.idle());

        once.reset();
        twice.reset();
        twice.reset();
        // 重复复位与一次复位逐字段相同，且状态回到"全新"。
        assert_eq!(once, twice);
        assert_eq!(twice, IdleGuard::new());
        assert_eq!(twice.idle(), 0);

        // 复位是**真的**重新起算：再跳 `MAX_IDLE_PACKETS` 次不得提前跳闸，且每一步都与
        // 一个全新计数器同值。
        let mut fresh = IdleGuard::new();
        for step in 0..MAX_IDLE_PACKETS {
            assert_eq!(
                twice.bump(),
                fresh.bump(),
                "step {step} after a double reset must match a fresh guard"
            );
        }
        assert_eq!(twice, fresh);
    }

    /// 判据 (长度契约的两个容差常数)：短片段阈值与绝对容差下限都必须**真的**在闭区间的
    /// 边界上生效，而且两个常数都取文档写明的字面值。
    ///
    /// 为什么需要它：既有的 `short_clips_get_an_explicit_delay_allowance` 只取理想的
    /// 100 帧与 199 369 帧两端，因此阈值取 4096 还是 8192、下限取 8 还是 16 都读不出
    /// 差别 —— 这些数字只在"恰好等于阈值"与"相对容差小于下限"这两格上可观察。
    /// 本判据把两格都钉住（纯整数运算，与架构无关，因此字面值可以在任何目标上硬断言）。
    #[test]
    fn the_length_contract_pins_the_tolerance_floor_and_the_short_clip_boundary() {
        // 理想输出恰好 4096 帧：**不**追加 SINC_LEN（阈值是"小于"）。
        let at_threshold = resample_len_contract(4_096, 48_000, 48_000).expect("non-zero rates");
        assert_eq!(
            (at_threshold.ideal_floor, at_threshold.ideal_ceil),
            (4_096, 4_096)
        );
        // 相对容差 = 4096 × 1000 ppm = 4 帧，小于 8 帧下限 ⇒ 生效的是下限。
        assert_eq!(at_threshold.min, 4_096 - 8);
        assert_eq!(at_threshold.max, 4_096 + 8);
        // 低于阈值一帧：追加 SINC_LEN = 256 帧。
        let below_threshold = resample_len_contract(4_095, 48_000, 48_000).expect("non-zero rates");
        assert_eq!(below_threshold.min, 4_095 - 8 - 256);
        assert_eq!(below_threshold.max, 4_095 + 8 + 256);
        // 更短：相对容差（1000 × 1000 ppm = 1 帧）同样被下限抬起。
        let tiny = resample_len_contract(1_000, 48_000, 48_000).expect("non-zero rates");
        assert_eq!(tiny.min, 1_000 - 8 - 256);
        assert_eq!(tiny.max, 1_000 + 8 + 256);
    }

    /// 判据 (长度契约的上下界公式)：上界必须由**上取整**得出，相对容差必须由**下取整**
    /// 得出 —— 两个取整方向各自都有可观察的字面值。
    ///
    /// 为什么需要它：既有的契约判据只钉"区间包含理想值"与三个整数比例的规范转换，
    /// 那几组里 floor 与 ceil 恰好相等（或相对容差恰好不受取整方向影响），因此
    /// "上取整退化成下取整"与"相对容差改用 ceil"两种改法都读不出差别。
    /// 本判据取两组 floor ≠ ceil 的比例：1001 帧 44.1 kHz → 48 kHz（1089 / 1090），
    /// 与 13 333 帧 2 Hz → 3 Hz（19 999 / 20 000，这一组的相对容差两侧相差 1 帧）。
    /// 全部是整数运算，与架构无关。
    #[test]
    fn the_length_contract_takes_its_ceiling_and_its_relative_slack_from_the_right_rounding() {
        let up = resample_len_contract(1_001, 48_000, 44_100).expect("non-zero rates");
        assert_eq!((up.ideal_floor, up.ideal_ceil), (1_089, 1_090));
        // floor = 1089 < 4096 ⇒ 容差 = max(1089 × 1000 ppm, 8) + 256 = 1 + 256。
        assert_eq!(up.min, 1_089 - 264);
        assert_eq!(up.max, 1_090 + 264);

        // floor = 19 999 / ceil = 20 000：相对容差必须取 floor 那一侧（19 帧）。
        let extreme = resample_len_contract(13_333, 3, 2).expect("non-zero rates");
        assert_eq!((extreme.ideal_floor, extreme.ideal_ceil), (19_999, 20_000));
        assert_eq!(extreme.min, 19_999 - 19);
        assert_eq!(extreme.max, 20_000 + 19);
    }

    /// 判据 (默认预算是冻结的常量)：`PcmBudget::default()` 的五个字段必须是字面值，
    /// 而不只是"与同名的公开常量一致"。
    ///
    /// 为什么需要它：`default_budget_is_recomputed_from_the_product_requirements` 的
    /// 断言都用同一个常量或同一条公式复算（例如
    /// `max_input_bytes == max_pcm_bytes + CONTAINER_OVERHEAD_BYTES`），因此单独改
    /// `DEFAULT_MAX_SAMPLE_RATE`、`DEFAULT_MAX_DURATION_SECS` 或
    /// `CONTAINER_OVERHEAD_BYTES` 的**数值**不会让任何断言变红。[ARCH-DET-001] 要求
    /// 预算是不随运行机器变化的常量，这些数字同时写在 `lib.rs` 的口径表里，因此它们
    /// 必须由判据逐条钉住。
    #[test]
    fn the_default_budget_numbers_are_frozen_literals() {
        let budget = PcmBudget::default();
        // 3 h @ 96 kHz 立体声交织 f32 = 10 800 s × 96 000 帧/s × 2 声道 × 4 字节。
        assert_eq!(budget.max_pcm_bytes, 8_294_400_000);
        // 输入字节上限 = PCM 预算 + 1 MiB 的容器开销。
        assert_eq!(budget.max_input_bytes, 8_294_400_000 + 1_048_576);
        assert_eq!(budget.max_channels, 64);
        assert_eq!(budget.max_sample_rate, 768_000);
        assert_eq!(budget.max_duration_secs, 21_600);
        // 推导输入本身也钉住：只钉结果的话，"两条产品要求一起被改小"仍然读不出来。
        assert_eq!(DEFAULT_REFERENCE_SECONDS, 10_800);
        assert_eq!(DEFAULT_REFERENCE_RATE, 96_000);
        assert_eq!(DEFAULT_REFERENCE_CHANNELS, 2);
        assert_eq!(DEFAULT_MULTITRACK_SECONDS, 1_800);
        assert_eq!(DEFAULT_MULTITRACK_CHANNELS, 8);
        assert_eq!(DEFAULT_MAX_CHANNELS, 64);
        assert_eq!(DEFAULT_MAX_SAMPLE_RATE, 768_000);
        assert_eq!(DEFAULT_MAX_DURATION_SECS, 21_600);
        assert_eq!(CONTAINER_OVERHEAD_BYTES, 1_048_576);
    }

    /// 判据 (构造侧 `for_layout`)：反推出来的预算必须把**容器开销**算进输入字节上限，
    /// 而且那是闭区间。
    ///
    /// 为什么需要它：`for_layout_admits_exactly_the_requirement_it_was_derived_from`
    /// 只断言 `max_pcm_bytes`，因此"输入字节上限丢掉 1 MiB 容器开销"在那条判据下完全
    /// 不可见（一份 44 字节头 + 恰好 `max_pcm_bytes` 的 WAV 会因此被误拒）。
    #[test]
    fn for_layout_adds_the_container_overhead_to_the_input_cap() {
        let pcm = pcm_bytes_for(3_600, 96_000, 2).expect("1 h @ 96 kHz stereo fits u64");
        assert_eq!(pcm, 3_600 * 96_000 * 2 * 4);
        let budget = PcmBudget::for_layout(3_600, 96_000, 2).expect("a 1-hour stereo layout");
        assert_eq!(budget.max_pcm_bytes, pcm);
        assert_eq!(budget.max_input_bytes, pcm + 1_048_576);
        // 闭区间：恰好 pcm + 1 MiB 的容器字节通过，多一字节即拒。
        let cap = pcm + 1_048_576;
        assert_eq!(check_input_len(cap, &budget), Ok(()));
        assert_eq!(
            check_input_len(cap + 1, &budget),
            Err(LimitViolation::InputTooLarge {
                bytes: cap + 1,
                limit: cap,
            })
        );
    }

    /// 判据（类别④ 参数极值／0 端点）：**全零预算**是合法的，且它的语义是
    /// "只放行空输入"，不是"放行一切"。
    ///
    /// 逐项判定（量什么 → 怎么量 → 单位 → 结论）：
    ///
    /// | 参数 | 取值 | 被调用的函数 | 结论 | 依据 |
    /// | :--- | :--- | :--- | :--- | :--- |
    /// | 五个字段全部 | `0` | `interleaved_samples_limit()` | `0` 个样本 | `0 / 4` |
    /// | 五个字段全部 | `0` | `max_duration_frames(任意)` | `0` 帧 | `0 × 采样率` |
    /// | 五个字段全部 | `0` | `check_input_len(0, ..)` | `Ok` | 闭区间：`0 > 0` 为假 |
    /// | 五个字段全部 | `0` | `check_input_len(1, ..)` | `InputTooLarge` | 非空输入一律拒 |
    /// | 五个字段全部 | `0` | `check_layout(1, 1, 0, ..)` | `TooManyChannels` | `1 > max_channels = 0` |
    /// | 五个字段全部 | `0` | `check_layout(0, 1, 0, ..)` | `ZeroChannels` | 声道数为 0 是畸形声明，先于预算 |
    /// | `max_channels` | `0` | `check_layout(任意 ≥ 1, ..)` | 一律 `TooManyChannels` | 声道闸门在最前 |
    ///
    /// 与"声道数为 0"是**两件事**：前者是调用方把预算收到零（合法的收紧），后者是流声明的
    /// 畸形（[`LimitViolation::ZeroChannels`]）。判据把两者分开断言，免得实现把
    /// "预算零"错报成"声道零"。
    ///
    /// 注入（实测）：把 `check_input_len` 的 `bytes > budget.max_input_bytes` 改成 `>=`
    /// （闭区间变成开区间）⇒ 本条以
    /// `left: Err(InputTooLarge { bytes: 0, limit: 0 })` / `right: Ok(())` 红，且只打红
    /// 这一条（`0 passed / 1 failed`）。
    #[test]
    fn an_all_zero_budget_admits_only_the_empty_input() {
        let zero = PcmBudget::new(0, 0, 0, 0, 0, 0);
        assert_eq!(zero.interleaved_samples_limit(), 0);
        for rate in [0u32, 1, 48_000, u32::MAX] {
            assert_eq!(zero.max_duration_frames(rate), 0, "rate {rate}");
        }
        assert_eq!(check_input_len(0, &zero), Ok(()));
        assert_eq!(
            check_input_len(1, &zero),
            Err(LimitViolation::InputTooLarge { bytes: 1, limit: 0 })
        );
        assert_eq!(
            check_input_len(u64::MAX, &zero),
            Err(LimitViolation::InputTooLarge {
                bytes: u64::MAX,
                limit: 0
            })
        );
        // 声道数为 0 的声明先于预算判定：两条错误不能混成一个。
        assert_eq!(
            check_layout(0, 48_000, 0, &zero),
            Err(LimitViolation::ZeroChannels)
        );
        // 任何非空布局都撞在"声道数上限 0"上（时长/字节闸门对 0 帧是恒真的）。
        for channels in [1u16, 2, u16::MAX] {
            assert_eq!(
                check_layout(channels, 48_000, 0, &zero),
                Err(LimitViolation::TooManyChannels { channels, limit: 0 }),
                "{channels} channels"
            );
        }
        // `for_layout` 从不产生"几乎放行一切"的零预算：退化参数返回 `None`。
        assert_eq!(PcmBudget::for_layout(0, 0, 0), None);
    }

    /// 判据（类别④ 参数极值）：长度契约在**采样率的两个类型端点**上仍然有定义，而且
    /// 定义不住时就明确 `None`（不许退化成"放行"）。
    ///
    /// 逐项判定（量什么 → 单位 → 结论）：
    ///
    /// | `input_frames` | `out_rate` | `in_rate` | 理想输出帧数 | 结论 | 依据 |
    /// | :--- | :--- | :--- | :--- | :--- | :--- |
    /// | `1` | `u32::MAX` | `1` | 4 294 967 295 | 契约有表示，`ideal` 落在区间内 | `u128` 中间量 |
    /// | `1` | `1` | `u32::MAX` | `0..=1` | 契约有表示（`floor = 0`，`ceil = 1`） | 有理数除法 |
    /// | `u64::MAX` | `u32::MAX` | `1` | 约 `7.9e28` | `None` ⇒ `Unrepresentable` | 放不进 `u64` |
    ///
    /// 量的是**帧**。这一格此前没有判据：既有的极值判据用的是
    /// `u64::MAX` 帧 × 768 kHz（`an_unrepresentable_contract_is_not_reported_as_an_undefined_ratio`），
    /// 没有把 `out_rate` / `in_rate` 推到 `u32::MAX`。
    ///
    /// 注入（实测）：把 `resample_len_contract` 末尾那四个 `u64::try_from(..).ok()?`
    /// 全部改成 `unwrap_or(u64::MAX)`（fail-closed 退化成钳位）⇒ 第 3 行不再是 `None`，
    /// 本条以 `assertion failed: resample_len_contract(u64::MAX, u32::MAX, 1).is_none()` 红，
    /// 且只打红这一条（`0 passed / 1 failed`）。
    #[test]
    fn the_length_contract_is_defined_at_the_rate_endpoints() {
        let widest = resample_len_contract(1, u32::MAX, 1)
            .expect("u32::MAX Hz is a legal rate, and the contract still fits in u64");
        assert_eq!(
            (widest.ideal_floor, widest.ideal_ceil),
            (4_294_967_295, 4_294_967_295)
        );
        // 容差按同一条公式复算：长片段（理想值 ≥ SHORT_CLIP_THRESHOLD_FRAMES）没有
        // 那 256 帧的额外放宽，只剩 0.1% 相对项。
        let slack =
            (widest.ideal_floor * LEN_TOLERANCE_PPM / 1_000_000).max(MIN_LEN_TOLERANCE_FRAMES);
        assert!(widest.ideal_floor >= SHORT_CLIP_THRESHOLD_FRAMES);
        assert_eq!(slack, 4_294_967);
        assert_eq!(widest.min, widest.ideal_floor - slack);
        assert_eq!(widest.max, widest.ideal_ceil + slack);
        assert_eq!((widest.min, widest.max), (4_290_672_328, 4_299_262_262));
        assert_eq!(
            check_resampled_len(1, u32::MAX, 1, widest.ideal_floor),
            Ok(widest.clone())
        );
        // 闭区间：两个端点各自"多一个单位"即拒。
        assert!(check_resampled_len(1, u32::MAX, 1, widest.min - 1).is_err());
        assert!(check_resampled_len(1, u32::MAX, 1, widest.max + 1).is_err());

        // 反方向：`u32::MAX` Hz → 1 Hz，1 帧输入的理想输出是 0 或 1 帧。
        let narrowest = resample_len_contract(1, 1, u32::MAX).expect("representable");
        assert_eq!((narrowest.ideal_floor, narrowest.ideal_ceil), (0, 1));
        assert_eq!(narrowest.min, 0);
        assert_eq!(narrowest.max, 1 + SINC_LEN + MIN_LEN_TOLERANCE_FRAMES);

        // 两端同时拉到极大：契约放不进 `u64` ⇒ `None`（fail-closed，不是"放行"）。
        assert!(resample_len_contract(u64::MAX, u32::MAX, 1).is_none());
        assert_eq!(
            check_resampled_len(u64::MAX, u32::MAX, 1, 0),
            Err(LenContractViolation::Unrepresentable {
                input_frames: u64::MAX,
                in_rate: 1,
                out_rate: u32::MAX,
            })
        );
    }

    /// 判据（类别④ 数值参数与预算推导）：[`PcmBudget::for_layout`] 的**比例**字段是一个
    /// 与它自己的采样率包络无关的常量，因此"该构造函数返回的预算恰好容纳这个布局"这句话
    /// **不覆盖采样率对**。
    ///
    /// 量什么：`for_layout(1, 768 000, 1)` 返回的预算上，[`check_resample_ratio`] 对采样率
    /// 对的判定；顺带钉住同一预算的采样率闸门对这两端都放行。
    /// 怎么量：`for_layout` 把 `max_sample_rate` 设成要求值 768 000，因此 1 Hz 与 768 kHz
    /// 两端各自都过采样率闸门；再看比例闸门。闭区间用"恰好等于上限"的那一对复算。
    ///
    /// 读数（本机、debug 构建）：
    ///
    /// | 采样率对 | 比例 | `for_layout(1, 768 000, 1)` 的结论 |
    /// | :--- | :--- | :--- |
    /// | 768 Hz → 768 kHz | 1 000（恰好等于上限） | `Ok` |
    /// | 767 Hz → 768 kHz | 1 001.3 | `ResampleRatioTooHigh` |
    /// | 1 Hz → 768 kHz | 768 000 | `ResampleRatioTooHigh` |
    ///
    /// 结论：`for_layout` 的 `max_resample_ratio` **恒为** [`DEFAULT_MAX_RESAMPLE_RATIO`]，
    /// 与 `seconds` / `sample_rate` / `channels` 三个要求无关。既有的
    /// `the_default_resample_ratio_cap_admits_every_standard_audio_rate_pair` 在**默认预算**
    /// 上问过同一个问题（结论：标准音频速率对全通过）；本条把同一条问题在 `for_layout`
    /// 上问一遍，结论相反 —— 因为默认预算的采样率上限（768 kHz）远宽于它的比例上限所覆盖
    /// 的下限（768 Hz），而 `for_layout` 的采样率上限是**要求值**，可以低到 1 Hz。
    ///
    /// 这一段是裁决 `R32`（`2026-10-10`）的验收判据。裁决结论是**不改数值**：比例上限是
    /// 全局安全帽（[ARCH-SEC-003]），按调用方包络放宽会削弱资源保护。改为在
    /// [`PcmBudget::for_layout`] 的文档里写明它与会话要求无关，并由本条把**事实**钉住 ——
    /// 逐档复算"四个不同布局拿到同一个上限"，以及文档句子仍在。
    ///
    /// 注入（实测）：把 `for_layout` 里的 `max_resample_ratio: DEFAULT_MAX_RESAMPLE_RATIO`
    /// 改成 `u64::MAX` ⇒ 本条以"767 Hz → 768 kHz 必须被拒"红；把这一行改成按
    /// `sample_rate` 推导（例如 `u64::from(sample_rate)`）⇒ 本条的逐档一致性断言红。
    #[test]
    fn for_layout_pins_a_ratio_cap_that_is_narrower_than_its_own_rate_envelope() {
        // 事实一（裁决 R32 的核心）：上限与会话要求无关 —— 逐档复算四组互不相同的布局。
        let layouts = [
            (1u64, 8_000u32, 1u16),
            (60, 48_000, 2),
            (3 * 60 * 60, 96_000, 8),
            (1, DEFAULT_MAX_SAMPLE_RATE, 64),
        ];
        for (seconds, sample_rate, channels) in layouts {
            let derived = PcmBudget::for_layout(seconds, sample_rate, channels)
                .expect("every listed layout is representable");
            assert_eq!(
                derived.max_resample_ratio, DEFAULT_MAX_RESAMPLE_RATIO,
                "for_layout({seconds}, {sample_rate}, {channels}) must take the GLOBAL ratio cap, \
                 not one derived from the session requirement"
            );
            // 同一档的其余三个字段**确实**来自要求（对照组：证明上面那一条不是"什么都没设"）。
            assert_eq!(derived.max_sample_rate, sample_rate);
            assert_eq!(derived.max_channels, channels);
            assert_eq!(derived.max_duration_secs, seconds);
        }

        // 事实二：文档必须写明这一条。判据不能直接写整句 —— 那句话会在本文件里出现两次
        // （注释一次、判据字面量一次）而**自我满足**。把它拆成两截拼起来，整句因此只出现在
        // `for_layout` 的文档里。
        let needle = concat!("比例上限与会话要求无关", "，恒为全局默认");
        assert!(
            include_str!("limits.rs").contains(needle),
            "PcmBudget::for_layout must document that the ratio cap is independent of the \
             session requirement (裁决 R32)"
        );

        let budget = PcmBudget::for_layout(1, DEFAULT_MAX_SAMPLE_RATE, 1)
            .expect("1 s of 768 kHz mono is a representable layout");
        // 该构造函数的采样率上限就是要求值：包络的两端都在闸门内。
        assert_eq!(budget.max_sample_rate, DEFAULT_MAX_SAMPLE_RATE);
        assert_eq!(budget.max_resample_ratio, DEFAULT_MAX_RESAMPLE_RATIO);
        assert_eq!(check_layout(1, 1, 1, &budget), Ok(()));
        assert_eq!(check_layout(1, DEFAULT_MAX_SAMPLE_RATE, 1, &budget), Ok(()));

        // 闭区间：恰好等于上限的一对通过，越界一档即拒，且报的数就是调用方给的那两个。
        // 比例上限是 `u64`，采样率是 `u32`，因此这里显式折算一次（上限本身就小于 u32 量程）。
        let cap = u32::try_from(DEFAULT_MAX_RESAMPLE_RATIO).expect("the cap fits in u32");
        let at_cap = DEFAULT_MAX_SAMPLE_RATE / cap;
        assert_eq!(
            at_cap * cap,
            DEFAULT_MAX_SAMPLE_RATE,
            "the cap must divide evenly here"
        );
        assert_eq!(
            check_resample_ratio(at_cap, DEFAULT_MAX_SAMPLE_RATE, &budget),
            Ok(())
        );
        assert_eq!(
            check_resample_ratio(at_cap - 1, DEFAULT_MAX_SAMPLE_RATE, &budget),
            Err(LimitViolation::ResampleRatioTooHigh {
                in_rate: at_cap - 1,
                out_rate: DEFAULT_MAX_SAMPLE_RATE,
                limit: DEFAULT_MAX_RESAMPLE_RATIO,
            })
        );

        // 包络的两端互转：两道闸门都放行这两个采样率，只有比例闸门拒绝。
        assert_eq!(
            check_resample_ratio(1, DEFAULT_MAX_SAMPLE_RATE, &budget),
            Err(LimitViolation::ResampleRatioTooHigh {
                in_rate: 1,
                out_rate: DEFAULT_MAX_SAMPLE_RATE,
                limit: DEFAULT_MAX_RESAMPLE_RATIO,
            })
        );
        // 下采样方向（比例 < 1）不受约束 —— 这一格与默认预算一致。
        assert_eq!(
            check_resample_ratio(DEFAULT_MAX_SAMPLE_RATE, 1, &budget),
            Ok(())
        );
    }

    /// 判据（诊断文案黄金表）：[`LimitViolation`] 的**全部 10 个变体**逐个渲染出**逐字固定**
    /// 的文案。
    ///
    /// 量什么：每个变体的 `Display` 输出（单位：字符）。怎么量：**直接构造变体值**，逐个
    /// `to_string()` 与黄金表比对 —— 不需要解码任何字节。
    ///
    /// 为什么需要它：本批的注入普查逐条改了 10 个变体的文案字面量，结果是 **9 个
    /// `ALL-GREEN`**（只有 `ZeroSampleRate` 被判据 `a_zero_sample_rate_stream_is_refused_not_a_panic`
    /// 的 `contains("zero sample rate")` 碰到）。这 10 句是要进 MCP 响应体与用户诊断的，
    /// 而它们的**模板**此前没有判据。
    ///
    /// 注入（实测）：改任一臂的字面量（例如 `input is {bytes} bytes` 改成 `input has ...`）
    /// ⇒ 本条红。
    #[test]
    fn every_limit_violation_arm_renders_its_documented_text() {
        let cases: [(&str, LimitViolation, &str); 10] = [
            (
                "InputTooLarge",
                LimitViolation::InputTooLarge {
                    bytes: 4_096,
                    limit: 1_024,
                },
                "input is 4096 bytes, over the 1024-byte cap",
            ),
            (
                "ZeroChannels",
                LimitViolation::ZeroChannels,
                "stream declares zero channels",
            ),
            (
                "TooManyChannels",
                LimitViolation::TooManyChannels {
                    channels: 65,
                    limit: 64,
                },
                "stream declares 65 channels, over the 64 cap",
            ),
            (
                "ZeroSampleRate",
                LimitViolation::ZeroSampleRate,
                "stream declares a zero sample rate",
            ),
            (
                "SampleRateTooHigh",
                LimitViolation::SampleRateTooHigh {
                    rate: 1_000_000,
                    limit: 768_000,
                },
                "stream declares 1000000 Hz, over the 768000 Hz cap",
            ),
            (
                "DurationTooLong",
                LimitViolation::DurationTooLong {
                    frames: 60_000,
                    sample_rate: 8_000,
                    seconds: 7,
                    limit_secs: 6,
                },
                "60000 frames at 8000 Hz is 7 s of audio, over the 6-second duration cap",
            ),
            (
                "PcmBudgetExceeded",
                LimitViolation::PcmBudgetExceeded {
                    frames: 3,
                    channels: 2,
                    samples: 6,
                    limit_samples: 0,
                },
                "3 frames x 2 channels = 6 interleaved samples (24 bytes of f32 PCM), over the \
                 0-sample / 0-byte PCM budget",
            ),
            (
                "LayoutOverflow",
                LimitViolation::LayoutOverflow {
                    frames: u64::MAX,
                    channels: 2,
                },
                "frame/channel product overflows: 18446744073709551615 frames x 2 channels",
            ),
            (
                "ResampleRatioTooHigh",
                LimitViolation::ResampleRatioTooHigh {
                    in_rate: 1,
                    out_rate: 768_000,
                    limit: 1_000,
                },
                "resampling 1 Hz to 768000 Hz is over the 1000x output/input sample-rate ratio cap",
            ),
            (
                "AllocationRefused",
                LimitViolation::AllocationRefused { samples: 4_096 },
                "allocator refused a buffer for 4096 more samples",
            ),
        ];
        assert_eq!(cases.len(), 10, "the golden table must cover every arm");
        for (arm, violation, expected) in cases {
            assert_eq!(limit_violation_arm(&violation), arm, "arm label {arm}");
            assert_eq!(violation.to_string(), expected, "arm {arm}");
        }
    }

    /// **无通配符**的 `match`：枚举新增一个变体就会让这段**编译**失败。
    ///
    /// 存在理由：黄金表里的 `assert_eq!(cases.len(), 10)` 只保证"表里有 10 行"，**抓不到**
    /// "枚举多了一个变体而表没跟上"（表长仍是 10，断言照过）。加了本函数之后，"覆盖全部臂"
    /// 从一句口号变成机器保证 —— 新增变体、或给某个臂**改名字**，都在 `cargo check` 上红。
    ///
    /// 同一件事也钉住了 `#[derive(Debug)]` 的**形状**：`Debug` 的输出就是变体名 ＋ 字段名，
    /// 而本函数与黄金表的构造式**写出**了全部变体名与全部字段名，因此形状不可能悄悄变。
    /// ⚠ **本 `match` 必须保持无通配符**：加上 `_ =>` 之后新增变体不会再红，而编译只出
    /// `unreachable_patterns` **警告**（裁决 R51 的实测读数：加 `_ => {}` 之后全部判据仍全绿）。
    fn limit_violation_arm(violation: &LimitViolation) -> &'static str {
        match violation {
            LimitViolation::InputTooLarge { .. } => "InputTooLarge",
            LimitViolation::ZeroChannels => "ZeroChannels",
            LimitViolation::TooManyChannels { .. } => "TooManyChannels",
            LimitViolation::ZeroSampleRate => "ZeroSampleRate",
            LimitViolation::SampleRateTooHigh { .. } => "SampleRateTooHigh",
            LimitViolation::DurationTooLong { .. } => "DurationTooLong",
            LimitViolation::PcmBudgetExceeded { .. } => "PcmBudgetExceeded",
            LimitViolation::LayoutOverflow { .. } => "LayoutOverflow",
            LimitViolation::ResampleRatioTooHigh { .. } => "ResampleRatioTooHigh",
            LimitViolation::AllocationRefused { .. } => "AllocationRefused",
        }
    }

    /// 判据（诊断文案黄金表）：[`LenContractViolation`] 的**全部 3 个变体**逐个渲染出逐字
    /// 固定的文案。
    ///
    /// 为什么需要它：注入普查里 `OutsideBounds` 与 `UndefinedRatio` 两条改动全绿（只有
    /// `Unrepresentable` 被 `an_unrepresentable_contract_is_not_reported_as_an_undefined_ratio`
    /// 碰到）。三个变体的**区分**（"比例无定义" vs "契约放不进 u64"）正是那条既有判据的
    /// 要点，而三句模板本身没有判据。
    ///
    /// 注入（实测）：把 `resampler produced` 改成 `resampler emitted` ⇒ 本条红。
    #[test]
    fn every_length_contract_violation_arm_renders_its_documented_text() {
        let contract = LenContract {
            min: 0,
            max: 0,
            ideal_floor: 0,
            ideal_ceil: 0,
        };
        let cases: [(&str, LenContractViolation, &str); 3] = [
            (
                "UndefinedRatio",
                LenContractViolation::UndefinedRatio {
                    in_rate: 0,
                    out_rate: 48_000,
                },
                "resample ratio undefined: 0 Hz -> 48000 Hz",
            ),
            (
                "Unrepresentable",
                LenContractViolation::Unrepresentable {
                    input_frames: u64::MAX,
                    in_rate: 1,
                    out_rate: u32::MAX,
                },
                "resample length contract for 18446744073709551615 frames (1 Hz -> 4294967295 \
                 Hz) does not fit in u64 frames",
            ),
            (
                "OutsideBounds",
                LenContractViolation::OutsideBounds {
                    produced: 1,
                    contract,
                },
                "resampler produced 1 frames, outside the contract 0..=0 (ideal 0..=0)",
            ),
        ];
        assert_eq!(cases.len(), 3, "the golden table must cover every arm");
        for (arm, violation, expected) in cases {
            assert_eq!(
                len_contract_violation_arm(&violation),
                arm,
                "arm label {arm}"
            );
            assert_eq!(violation.to_string(), expected, "arm {arm}");
        }
    }

    /// 同 [`limit_violation_arm`]：[`LenContractViolation`] 的**无通配符** `match`。
    ///
    /// 注入（实测）：给枚举加一个 `LenContractViolation::Placeholder` 变体（不在本函数里
    /// 列出）⇒ `cargo check` 以 `non-exhaustive patterns: Placeholder not covered` 红。
    /// ⚠ **本 `match` 必须保持无通配符**：加上 `_ =>` 之后新增变体不会再红，而编译只出
    /// `unreachable_patterns` **警告**（裁决 R51 的实测读数：加 `_ => {}` 之后全部判据仍全绿）。
    fn len_contract_violation_arm(violation: &LenContractViolation) -> &'static str {
        match violation {
            LenContractViolation::UndefinedRatio { .. } => "UndefinedRatio",
            LenContractViolation::Unrepresentable { .. } => "Unrepresentable",
            LenContractViolation::OutsideBounds { .. } => "OutsideBounds",
        }
    }

    /// 判据（R86：**`Default` 不是"非现实的缺陷类"**）：默认预算必须与**逐字段的合法替代状态**
    /// 都可区分，而且**每个实例都是单独构造的**（⛔ 驱动不得对两个被测实例做相同的初始化 ——
    /// 那会遮蔽构造期差异）。
    ///
    /// 量什么：`PcmBudget::default()` 与 6 个"只改一个字段"的合法替代状态之间的 `==`（布尔）。
    /// 怎么量：每个替代状态都从 `default()` 出发，**只**把它自己的那一个字段改成一个合法的、
    /// 更紧的值（`..base` 其余照抄）⇒ 两侧的初始化**必然不同**，任何一处"该字段没被
    /// `PartialEq` 看见"或"默认值其实是 0/未设"都会被抓住。
    ///
    /// 读数（本机、debug 构建）：6 个替代状态**全部**与默认值不等（`assert_ne!` 逐一成立），
    /// 且默认值等于它自己（`==` 反身）。
    ///
    /// 为什么需要它：`PcmBudget::default()` 是六个闸门的**唯一**默认来源；若某个字段的默认值
    /// 退化成 0（或被 `PartialEq` 忽略），"闸门默认开着"这件事就没有任何判据能发现 ——
    /// 既有判据只逐字段核对**数值**，不核对"这些字段彼此可区分"。
    ///
    /// ⭐ R114（下界要**根绑定**）：本判据的界来自 `base` **自己的字段**（`base.max_channels`
    /// 等），⛔ 不借用相邻字段或常量表，因此字段被重命名/改类型时这里会编译失败，而不是
    /// 悄悄比对一个无关的数。
    ///
    /// 注入（实测）：把 `impl PartialEq for PcmBudget`（或派生）改成"只比 `max_pcm_bytes`"
    /// ⇒ 本条红（至少一个替代状态变得与默认值相等）。
    #[test]
    fn every_default_field_is_distinguishable_from_a_legal_alternative_state() {
        let base = PcmBudget::default();
        // ⭐ 每个替代状态**单独构造**（R86 第二条），且只改**它自己**那一个字段。
        let alternatives: [(&str, PcmBudget); 6] = [
            (
                "max_input_bytes",
                PcmBudget {
                    max_input_bytes: base.max_input_bytes / 2,
                    ..base
                },
            ),
            (
                "max_pcm_bytes",
                PcmBudget {
                    max_pcm_bytes: base.max_pcm_bytes / 2,
                    ..base
                },
            ),
            (
                "max_channels",
                PcmBudget {
                    max_channels: base.max_channels / 2,
                    ..base
                },
            ),
            (
                "max_sample_rate",
                PcmBudget {
                    max_sample_rate: base.max_sample_rate / 2,
                    ..base
                },
            ),
            (
                "max_duration_secs",
                PcmBudget {
                    max_duration_secs: base.max_duration_secs / 2,
                    ..base
                },
            ),
            (
                "max_resample_ratio",
                PcmBudget {
                    max_resample_ratio: base.max_resample_ratio / 2,
                    ..base
                },
            ),
        ];
        // R93：先证明被扫集合非空且达到下界。
        assert_eq!(
            alternatives.len(),
            6,
            "all six gate fields must be exercised"
        );
        assert_eq!(base, base, "the default must at least equal itself");
        for (field, alternative) in alternatives {
            assert_ne!(
                base, alternative,
                "changing only `{field}` must make the budget distinguishable — otherwise the \
                 default value of `{field}` is invisible to equality"
            );
            // 反向自证：这一档**恰好只**改了一个字段。⭐ 这一条还顺带抓住"默认值退化" ——
            // 若某字段的默认值是 0，那么 `0 / 2 == 0` ⇒ 差异数会是 0 ⇒ 本条红（R86 的核心）。
            let differing = [
                alternative.max_input_bytes != base.max_input_bytes,
                alternative.max_pcm_bytes != base.max_pcm_bytes,
                alternative.max_channels != base.max_channels,
                alternative.max_sample_rate != base.max_sample_rate,
                alternative.max_duration_secs != base.max_duration_secs,
                alternative.max_resample_ratio != base.max_resample_ratio,
            ]
            .iter()
            .filter(|changed| **changed)
            .count();
            assert_eq!(
                differing, 1,
                "the `{field}` case must differ from the default in exactly one field — 0 means \
                 that field's default is degenerate (e.g. zero), above 1 means the driver \
                 initialised more than one field"
            );
        }
    }
}
