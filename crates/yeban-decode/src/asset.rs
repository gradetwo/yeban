//! 不可变资产：解码结果、内容寻址与 `yeban-model` 的对接。
//!
//! 这一层回答的是产品问题："导入一个文件 → 得到一个内容寻址的不可变 PCM 资产"。
//! 链路只有三步，每一步都可以单独核验：
//!
//! 1. **原始字节 → [`yeban_model::AssetHash`]**：SHA-256 是 CAS 的键
//!    （`.yeban` 归档里的 `assets/{sha256}`），见 [MODEL-AST-007]。摘要由
//!    [`yeban_model::AssetHasher`] **增量**算出，因此磁盘入口 [`import_path`] 不需要
//!    把容器整份读进内存（[`hash_reader`] 只驻留一个定长缓冲）；
//! 2. **原始字节 → [`DecodedAsset`]**：解码事实（声道/采样率/位深/帧数/时长对账）+ 交织 `f32` PCM；
//! 3. **解码结果 → [`DecodedAsset::pcm_hash`]**：把"解出来的样本"本身也做成内容摘要，
//!    用来给 [ARCH-DET-001] 的"同输入 → 同输出"提供一条**可比较的指纹**
//!    （逐样本 `to_bits()` 比较在高帧数下太贵，摘要足够红）。摘要按定长分块
//!    喂给 [`AssetHasher`]，因此**不**复制样本缓冲。
//!
//! 不可变性 [ARCH-TOP-002]：`DecodedAsset` 只提供只读访问器，没有任何 `&mut` 出口，
//! 因此它可以被 `Arc` 起来在实时线程里只读消费 —— 实时回调路径上永远不会调用本模块的
//! 任何构造/解码函数。

use std::io::Read;
use std::path::Path;

use yeban_model::{AssetHash, AssetHasher, AssetMetadata, BitDepth, MediaKind};

use crate::decode::{self, DecodeOptions};
use crate::duration::Reconciliation;
use crate::error::{DecodeError, DecodeResult};
use crate::limits;

/// 解码器实际产出的样本格式。
///
/// 与 `symphonia` 的 `GenericAudioBufferRef` 十个变体一一对应。**位深来自"解码器真的
/// 吐出了什么"，不是来自容器声明** —— 容器可以撒谎，缓冲区不能。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcmFormat {
    /// 8-bit 无符号整数。
    U8,
    /// 16-bit 无符号整数。
    U16,
    /// 24-bit 无符号整数。
    U24,
    /// 32-bit 无符号整数。
    U32,
    /// 8-bit 有符号整数。
    S8,
    /// 16-bit 有符号整数。
    S16,
    /// 24-bit 有符号整数。
    S24,
    /// 32-bit 有符号整数。
    S32,
    /// 32-bit IEEE 754 浮点。
    F32,
    /// 64-bit IEEE 754 浮点。
    F64,
}

impl PcmFormat {
    /// 由 symphonia 的通用音频缓冲判定实际样本格式。
    ///
    /// 用 `match` 而不是"读容器的 `bits_per_sample`"：后者是**声明**，前者是**事实**。
    #[must_use]
    pub fn from_buffer(buffer: &symphonia::core::audio::GenericAudioBufferRef<'_>) -> Self {
        use symphonia::core::audio::GenericAudioBufferRef as B;
        match buffer {
            B::U8(_) => Self::U8,
            B::U16(_) => Self::U16,
            B::U24(_) => Self::U24,
            B::U32(_) => Self::U32,
            B::S8(_) => Self::S8,
            B::S16(_) => Self::S16,
            B::S24(_) => Self::S24,
            B::S32(_) => Self::S32,
            B::F32(_) => Self::F32,
            B::F64(_) => Self::F64,
        }
    }

    /// 每个样本的位数。
    #[must_use]
    pub fn bit_depth(self) -> u16 {
        match self {
            Self::U8 | Self::S8 => 8,
            Self::U16 | Self::S16 => 16,
            Self::U24 | Self::S24 => 24,
            Self::U32 | Self::S32 | Self::F32 => 32,
            Self::F64 => 64,
        }
    }

    /// 是否为浮点样本格式。
    #[must_use]
    pub fn is_float(self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }

    /// 映射到 `yeban-model` 的 [`BitDepth`]。
    ///
    /// `yeban-model` 的 `BitDepth` 只有 `Int16` / `Int24` / `Float32` 三个变体
    /// （规范枚举），因此 8-bit、32-bit 整数、64-bit 浮点**没有**对应值 —— 这里返回
    /// `None` 而不是硬塞一个近似值。这些格式在夜半资产池里没有下游语义。
    #[must_use]
    pub fn model_bit_depth(self) -> Option<BitDepth> {
        match self {
            Self::S16 => Some(BitDepth::Int16),
            Self::S24 => Some(BitDepth::Int24),
            Self::F32 => Some(BitDepth::Float32),
            Self::U8 | Self::U16 | Self::U24 | Self::U32 | Self::S8 | Self::S32 | Self::F64 => None,
        }
    }
}

/// 一次解码得到的事实（不含样本本身）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeFacts {
    /// 声道数（由每个解码缓冲的平面数确定）。
    pub channels: u16,
    /// 采样率 (Hz)。
    pub sample_rate: u32,
    /// 解码器实际产出的样本格式。
    pub pcm_format: PcmFormat,
    /// 容器/解码器**声明**的位深（可能与实际不同，仅供对账与展示）。
    pub declared_bit_depth: Option<u16>,
    /// 容器声明的总帧数（`None` 表示未上报）。
    pub declared_frames: Option<u64>,
    /// 容器上报的编码器前置延迟帧数（本切片**不**裁剪，仅记录）。
    pub encoder_delay_frames: Option<u32>,
    /// 容器上报的编码器尾部填充帧数（本切片**不**裁剪，仅记录）。
    pub encoder_padding_frames: Option<u32>,
    /// 声明帧数与解出帧数的对账结论 [ARCH-DET-001]。
    pub duration: Reconciliation,
}

/// 解码后的不可变资产。
///
/// `samples` 是**交织**（interleaved）的 `f32`，通道顺序按容器原样，长度恒等于
/// `frame_count() × channels`。选交织而不是分声道（planar）的理由：
/// - symphonia 的直接出口就是交织（`copy_to_slice_interleaved`）；
/// - `rubato` 的 `InterleavedSlice` 适配器直接吃交织缓冲，导入 → 重采样之间不需要转置；
/// - 代价是"取某一声道的整段"要跨步读，这个代价由下游（渲染）一次性付出。
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedAsset {
    facts: DecodeFacts,
    samples: Vec<f32>,
}

/// `pcm_hash` 的域分隔前缀（改它等于改摘要口径，必须同时改 notes 与判据）。
const PCM_HASH_DOMAIN: &[u8] = b"yeban.pcm.f32le.v1";

/// [`hash_reader`] 一次从流里读入的字节数：64 KiB。
///
/// 这个数字只决定 **I/O 缓冲**的大小，不参与摘要口径：SHA-256 的分块方式不影响结果
/// （`yeban_model::AssetHasher` 的契约）。64 KiB 与常见文件系统的块大小同量级，
/// 因此顺序读的 syscall 次数少，而缓冲本身小到与素材长度无关。
const HASH_BUFFER_BYTES: usize = 64 * 1024;

/// `pcm_hash` 一次编码进暂存缓冲的样本数上限。
///
/// 256 个 `f32` = 1 KiB，落在栈上而不是堆上。规范字节流因此被切成定长块喂给
/// [`AssetHasher`]：摘要路径**不持有与样本数成比例的任何缓冲**。这就是
/// `docs/ledger/decode-limits-notes.md` §2.3 峰值表里 `DecodedAsset::pcm_hash`
/// 从"×2 份 PCM"变成"×1 份 PCM"的全部机制。
const PCM_HASH_CHUNK_SAMPLES: usize = 256;

/// 暂存缓冲的字节数（= [`PCM_HASH_CHUNK_SAMPLES`] × 4）。
const PCM_HASH_STAGING_BYTES: usize = PCM_HASH_CHUNK_SAMPLES * 4;

impl DecodedAsset {
    /// 由解码器构造（crate 内部唯一入口，避免外部拼出一个自相矛盾的资产）。
    pub(crate) fn new(facts: DecodeFacts, samples: Vec<f32>) -> Self {
        Self { facts, samples }
    }

    /// 解码事实。
    #[must_use]
    pub fn facts(&self) -> &DecodeFacts {
        &self.facts
    }

    /// 声道数。
    #[must_use]
    pub fn channels(&self) -> u16 {
        self.facts.channels
    }

    /// 采样率 (Hz)。
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.facts.sample_rate
    }

    /// 解码器实际产出的样本格式。
    #[must_use]
    pub fn pcm_format(&self) -> PcmFormat {
        self.facts.pcm_format
    }

    /// 交织 `f32` 样本（只读）。
    #[must_use]
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// 帧数（每声道的采样数）。
    #[must_use]
    pub fn frame_count(&self) -> u64 {
        if self.facts.channels == 0 {
            return 0;
        }
        u64::try_from(self.samples.len() / usize::from(self.facts.channels)).unwrap_or(u64::MAX)
    }

    /// 时长（秒）。
    ///
    /// 只做一次 `f64` 除法，不引入任何超越函数 —— 因此结果在 L1 与 L2 上都是同一位模式。
    #[must_use]
    pub fn duration_seconds(&self) -> f64 {
        if self.facts.sample_rate == 0 {
            return 0.0;
        }
        self.frame_count() as f64 / f64::from(self.facts.sample_rate)
    }

    /// 映射到 `yeban-model` 的 [`BitDepth`]（见 [`PcmFormat::model_bit_depth`]）。
    #[must_use]
    pub fn model_bit_depth(&self) -> Option<BitDepth> {
        self.facts.pcm_format.model_bit_depth()
    }

    /// 容器声明的总帧数。
    #[must_use]
    pub fn declared_frames(&self) -> Option<u64> {
        self.facts.declared_frames
    }

    /// 声明帧数与解出帧数是否已对账通过。
    #[must_use]
    pub fn duration_is_reconciled(&self) -> bool {
        self.facts.duration.is_reconciled()
    }

    /// 解码后 PCM 的**规范化内容摘要** [ARCH-DET-001]。
    ///
    /// 口径（写死）：`"yeban.pcm.f32le.v1"` ‖ `channels: u16 LE` ‖ `sample_rate: u32 LE`
    /// ‖ `frame_count: u64 LE` ‖ 每个样本 `f32::to_le_bytes()`。
    ///
    /// 注意：摘要对**位模式**敏感，`-0.0` 与 `0.0` 是不同的摘要 —— 这正是确定性判据
    /// 想要的（"值相等"不够，位级相等才算）。
    ///
    /// 上面这串字节流**不再**先拼成一个 `Vec<u8>`：它按 [`PCM_HASH_CHUNK_SAMPLES`]
    /// 分块喂给 [`AssetHasher`]（`update` / `finalize`），因此本方法不分配与样本数成
    /// 比例的临时缓冲。改建前这里是 `Vec::with_capacity(32 + samples.len() * 4)`，
    /// 等于在资产之外再复制一整份 PCM（`docs/ledger/decode-limits-notes.md` §2.3）。
    /// 摘要值本身逐位未变 —— 判据
    /// [`tests::pcm_hash_matches_an_independent_python_oracle`] 用一个不由本 crate
    /// 生成的期望值把它钉住。
    #[must_use]
    pub fn pcm_hash(&self) -> AssetHash {
        let mut hasher = AssetHasher::new();
        hasher.update(PCM_HASH_DOMAIN);
        hasher.update(&self.facts.channels.to_le_bytes());
        hasher.update(&self.facts.sample_rate.to_le_bytes());
        hasher.update(&self.frame_count().to_le_bytes());
        // `chunks` 最后一块可能短于 `PCM_HASH_CHUNK_SAMPLES`，因此每次只把前
        // `chunk.len() * 4` 字节交给摘要器；`as_chunks_mut` 把暂存缓冲切成定长 4 字节
        // 槽位，于是 `copy_from_slice` 不会因为长度不符而 panic。
        let mut staging = [0u8; PCM_HASH_STAGING_BYTES];
        for chunk in self.samples.chunks(PCM_HASH_CHUNK_SAMPLES) {
            let (slots, _) = staging.as_chunks_mut::<4>();
            for (slot, sample) in slots.iter_mut().zip(chunk) {
                slot.copy_from_slice(&sample.to_le_bytes());
            }
            let filled = chunk.len() * 4;
            hasher.update(&staging[..filled]);
        }
        hasher.finalize()
    }
}

/// 导入结果：CAS 索引项 + 解码后的不可变资产。
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedAsset {
    /// `yeban-model` 的资产元数据（内容寻址池的索引项）[MODEL-AST-007]。
    pub index: AssetMetadata,
    /// 解码后的不可变 PCM。
    pub decoded: DecodedAsset,
}

impl ImportedAsset {
    /// 原始输入字节的 SHA-256（CAS 键）。
    #[must_use]
    pub fn asset_hash(&self) -> &AssetHash {
        &self.index.hash
    }

    /// 解码后 PCM 的规范化摘要。
    #[must_use]
    pub fn pcm_hash(&self) -> AssetHash {
        self.decoded.pcm_hash()
    }
}

/// 为一个音频文件生成 `yeban-model` 的 CAS 索引项（**不解码**，纯摘要）。
///
/// `license` 必须由调用方给出（`MUST-GATE-014` 要求资产许可逐条可查）：本 crate 不会
/// 凭空编一个许可字符串。
#[must_use]
pub fn asset_index(bytes: &[u8], original_path: &str, license: &str) -> AssetMetadata {
    AssetMetadata {
        hash: AssetHash::of_bytes(bytes),
        original_path: original_path.to_owned(),
        byte_len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        media_kind: MediaKind::Audio,
        license: license.to_owned(),
    }
}

/// 读一个流并按 [MODEL-AST-007] 的 CAS 口径算出内容摘要（SHA-256）。
///
/// 摘要状态机住在 `yeban-model`（[`AssetHasher`]），因此本 crate 不需要 `sha2` 依赖，
/// 也不必自己维护一份摘要状态。这里**分块**喂入，所以驻留内存只有一个
/// [`HASH_BUFFER_BYTES`] 的缓冲 —— 与流的长度无关。这正是
/// `docs/ledger/decode-limits-notes.md` §8 那条 needs 的落点：
/// 摘要是增量的，因此 `import_path` 不必把容器整份读进内存。
///
/// 返回值与 `AssetHash::of_bytes` 对**同一条字节流**的结果相同；这条等价由
/// `yeban_model` 自己的判据保证（`AssetHasher` 的契约），本 crate 的
/// `tests/import_streaming.rs` 再从"真实文件"一侧复算一遍。
///
/// **同时返回摘要实际覆盖的字节数**（单位：字节）。为什么要这个数：分离的
/// `std::fs::metadata` 读数与"摘要真的读了几个字节"是两件事 —— 文件在摘要过程中被改写
/// 时它们会不同，而内容寻址的键只对**被摘要的那份字节**成立。调用方
/// （[`import_path_with_len`]）因此用这个数当 `byte_len`，并在它与第二次测量不等时拒绝。
///
/// # Errors
///
/// 流自身的 I/O 失败原样上报（[`DecodeError::Io`]）。
fn hash_reader<R: Read>(mut reader: R) -> DecodeResult<(AssetHash, u64)> {
    let mut hasher = AssetHasher::new();
    let mut buffer = vec![0u8; HASH_BUFFER_BYTES];
    let mut hashed: u64 = 0;
    loop {
        match reader.read(&mut buffer) {
            // 读到 0 字节即流结束。`read` 对空缓冲返回 0 是合法的，因此这里不靠
            // "缓冲是否填满"判断结束，只有 `Ok(0)` 才是终点。
            Ok(0) => return Ok((hasher.finalize(), hashed)),
            Ok(filled) => {
                hasher.update(&buffer[..filled]);
                hashed = hashed.saturating_add(u64::try_from(filled).unwrap_or(u64::MAX));
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err.into()),
        }
    }
}

/// 从内存字节导入：摘要 + 解码，一次完成。
///
/// # Errors
///
/// 见 [`crate::error::DecodeError`]。
pub fn import_bytes(
    bytes: &[u8],
    original_path: &str,
    license: &str,
    options: &DecodeOptions,
) -> DecodeResult<ImportedAsset> {
    let index = asset_index(bytes, original_path, license);
    let decoded = decode::decode_bytes(bytes, options)?;
    Ok(ImportedAsset { index, decoded })
}

/// 从磁盘路径导入：流式算摘要 + 流式解码，容器**不整份驻留**。
///
/// 两次遍历同一个文件：
/// 1. 只读地顺序读一遍算 SHA-256（[`hash_reader`]，驻留一个
///    [`HASH_BUFFER_BYTES`] 缓冲）；
/// 2. 再把 `File` 交给 [`decode::decode_path`]，由 symphonia 自己流式解封装。
///
/// 因此建成后的峰值内存是 **×1 份 PCM**（`docs/ledger/decode-limits-notes.md` §2.3
/// 的 `import_path` 行），而改建前是 ×1 + 整份容器字节。
///
/// 为什么是两遍而不是"边解码边摘要"：symphonia 的解封装会**回退 seek**（探测、块头
/// 重读），因此"在 `Read::read` 上顺手 `update`"会把回读的字节重复计进摘要。两遍
/// 顺序读的代价是一次顺序 I/O，换来的是摘要口径与 `AssetHash::of_bytes` 恒等。
///
/// ## 三道长度测量（TOCTOU）
///
/// 本函数在两个时刻量同一个文件（算摘要之前、算完之后），而两次之间文件可能被别的进程
/// 改写。因此长度要过**三道**判定，全部发生在分配与解码之前：
///
/// 1. 第一次量到的长度（算摘要之前）；
/// 2. 摘要**实际读到**的字节数 —— 文件在摘要过程中变大时，只有这个数看得住；
/// 3. 第二次量到的长度。
///
/// 任何一道超预算 ⇒ [`DecodeError::Budget`]（[`LimitViolation::InputTooLarge`]）。
/// 三道都过之后，若第二次量到的长度与摘要实际读到的字节数**不同**，返回
/// [`DecodeError::Malformed`]，文案同时点出两个数 —— 因为此时"被摘要的字节"与"磁盘上的
/// 这份文件"不是同一个东西，把 `byte_len` 写成任何一个数都会是假话。⛔ 不静默取其一。
///
/// 对正常文件零影响：没有并发改写时三道测量相等，比对恒真。判据
/// `tests::the_second_length_measurement_obeys_the_input_byte_budget`、
/// `tests::a_file_that_changes_between_the_two_measurements_is_refused`（注入长度来源）、
/// `tests::a_stable_file_records_the_length_that_was_actually_hashed`（这一侧）。
///
/// # Errors
///
/// 见 [`crate::error::DecodeError`]；文件读失败也会上报。
pub fn import_path(
    path: &Path,
    license: &str,
    options: &DecodeOptions,
) -> DecodeResult<ImportedAsset> {
    import_path_with_len(path, license, options, file_len)
}

/// 量一个路径当前的字节数。
///
/// 抽成独立函数，是为了让"两次测量之间文件被改写"这条 TOCTOU 窗口**可判**：
/// [`import_path_with_len`] 把它当参数收下，判据因此可以注入"第二次报大"或"第二次报小"的
/// 实现，而产线入口 [`import_path`] 传的永远是这里的真实实现。
///
/// # Errors
///
/// 路径不存在或不可访问时把 `std::io::Error` 原样上报。
fn file_len(path: &Path) -> std::io::Result<u64> {
    Ok(std::fs::metadata(path)?.len())
}

/// [`import_path`] 的完整版：**长度来源可注入**。
///
/// 为什么要这条缝：`import_path` 必须在**两个时刻**量同一个文件（算摘要之前、算完之后），
/// 而这两次测量之间文件可能被别的进程改写。判据要能构造那一格，就必须能替换"怎么量长度"。
///
/// ## 两次长度不一致时返回什么
///
/// 判定顺序（每一步都在**分配与解码之前**）：
/// 1. 第一次量到的长度（算摘要之前）—— 超预算 ⇒ [`LimitViolation::InputTooLarge`]；
/// 2. 摘要**实际读到**的字节数 —— 超预算 ⇒ 同上（读的过程中文件变大也不放过）；
/// 3. 第二次量到的长度 —— 超预算 ⇒ 同上。
///
/// 第 3 步之后，若第二次量到的长度与摘要实际读到的字节数不同，返回
/// [`DecodeError::Malformed`]，文案同时点出两个数。理由：内容寻址的键是**被摘要的那份
/// 字节**的 SHA-256；长度对不上意味着"被摘要的字节"与"现在磁盘上的这份文件"不是同一个
/// 东西，此时把 `byte_len` 写成任何一个数都会是假话。⛔ 不静默取其一。
///
/// 对正常文件零影响：没有并发改写时三次测量相等，"第 3 步之后的比对"恒真。
///
/// # Errors
///
/// 见 [`crate::error::DecodeError`]；长度来源自身的 I/O 失败原样上报。
fn import_path_with_len<F>(
    path: &Path,
    license: &str,
    options: &DecodeOptions,
    mut len_of: F,
) -> DecodeResult<ImportedAsset>
where
    F: FnMut(&Path) -> std::io::Result<u64>,
{
    // 第一道闸门：先看声明长度，比打开文件更便宜（畸形/超大素材在这里就被拒）。
    let declared_len = len_of(path)?;
    limits::check_input_len(declared_len, &options.budget)?;
    let (hash, hashed) = hash_reader(std::fs::File::open(path)?)?;
    // 第二道闸门：摘要**实际读到**的字节数。文件在摘要过程中变大时，第一次测量看不住它，
    // 而这个数才是"我们真的处理了多少字节"。
    limits::check_input_len(hashed, &options.budget)?;
    // 第三道闸门：第一遍读完后的真实长度 —— 文件可能在第一次测量之后变大，
    // 绝不"先读了再说"。
    let actual_len = len_of(path)?;
    limits::check_input_len(actual_len, &options.budget)?;
    // 三道测量必须相等。不等就意味着"被摘要的那份字节"与"现在磁盘上的这份文件"不是
    // 同一个东西：此时把 `byte_len` 写成任何一个数都会是假话（写第二次测量 ⇒ 与摘要覆盖
    // 的字节数不符；写摘要字节数 ⇒ 与文件当前长度不符）。因此拒绝，⛔ 不静默取其一。
    if actual_len != hashed {
        return Err(DecodeError::Malformed {
            detail: format!(
                "the file changed while it was being read: the digest covers {hashed} bytes but                  the file now reports {actual_len} bytes; refusing to index a container whose \
                 digest cannot be trusted"
            ),
        });
    }
    let decoded = decode::decode_path(path, options)?;
    let index = AssetMetadata {
        hash,
        original_path: path.to_string_lossy().into_owned(),
        // 权威长度 = 摘要实际覆盖的字节数（`== actual_len`，上面刚判过）。
        byte_len: hashed,
        media_kind: MediaKind::Audio,
        license: license.to_owned(),
    };
    Ok(ImportedAsset { index, decoded })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::decode_bytes;
    use crate::error::DecodeError;
    use crate::limits::{LimitViolation, PcmBudget};
    use crate::testfix::{WavFormat, WavSpec, encode_int_samples, wav};

    fn fixture(values: &[i32], channels: u16) -> Vec<u8> {
        let spec = WavSpec {
            channels,
            sample_rate: 44_100,
            bits: 16,
            format: WavFormat::Integer,
        };
        wav(&spec, &encode_int_samples(16, values))
    }

    #[test]
    fn cas_index_matches_yeban_model_and_is_content_addressed() {
        let bytes = fixture(&[1, -2, 3, -4], 2);
        let a = asset_index(&bytes, "samples/hat.wav", "CC0-1.0");
        let b = asset_index(&bytes, "somewhere/else/hat.wav", "CC0-1.0");
        // 路径只留痕，不参与寻址 —— 同内容必须同键。
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.hash, AssetHash::of_bytes(&bytes));
        assert_eq!(a.byte_len, u64::try_from(bytes.len()).unwrap());
        assert_eq!(a.media_kind, MediaKind::Audio);
        assert_eq!(a.license, "CC0-1.0");
        assert_ne!(a.original_path, b.original_path);

        // 内容变一个采样点，键必须变。
        let other = fixture(&[1, -2, 3, -5], 2);
        assert_ne!(
            a.hash,
            asset_index(&other, "samples/hat.wav", "CC0-1.0").hash
        );
    }

    #[test]
    fn importing_the_same_bytes_twice_yields_the_same_keys() {
        let bytes = fixture(&[100, -100, 200, -200], 2);
        let opts = DecodeOptions::default();
        let first = import_bytes(&bytes, "a.wav", "CC0-1.0", &opts).unwrap();
        let second = import_bytes(&bytes, "a.wav", "CC0-1.0", &opts).unwrap();
        assert_eq!(first.index, second.index);
        assert_eq!(first.pcm_hash(), second.pcm_hash());
        assert_eq!(first.asset_hash(), second.asset_hash());
    }

    #[test]
    fn pcm_hash_is_sensitive_to_the_last_bit_of_a_sample() {
        let facts = DecodeFacts {
            channels: 1,
            sample_rate: 48_000,
            pcm_format: PcmFormat::F32,
            declared_bit_depth: Some(32),
            declared_frames: Some(2),
            encoder_delay_frames: None,
            encoder_padding_frames: None,
            duration: Reconciliation::Exact,
        };
        let base = DecodedAsset::new(facts.clone(), vec![0.25, -0.5]);
        let nudged = DecodedAsset::new(
            facts.clone(),
            vec![f32::from_bits(0.25f32.to_bits() + 1), -0.5],
        );
        assert_ne!(base.pcm_hash(), nudged.pcm_hash());
        // 单独翻转 -0.0 / 0.0 也必须被区分（值相等，位模式不等）。
        let zero = DecodedAsset::new(facts.clone(), vec![0.0]);
        let neg_zero = DecodedAsset::new(facts, vec![-0.0]);
        assert_eq!(zero.samples(), neg_zero.samples());
        assert_ne!(zero.pcm_hash(), neg_zero.pcm_hash());
    }

    /// `pcm_hash` 的**独立预言机**：期望摘要由 Python `hashlib` 按资产模块文档写死的
    /// 口径（`"yeban.pcm.f32le.v1"` ‖ `channels` u16 LE ‖ `sample_rate` u32 LE ‖
    /// `frame_count` u64 LE ‖ 逐样本 `f32::to_le_bytes()`）算出，**不**由本 crate 的任何
    /// 代码生成。四个样本值都是 2 的幂（0.25 / -0.5 / 0.125 / -0.0625），在 IEEE-754
    /// `f32` 里无舍入，因此 Python 与 Rust 的字节必然一致。
    ///
    /// 复算命令（本机跑过，2026-10-08）：
    /// `python3 -B -c "...struct.pack('<H'/'<I'/'<Q'/'<f')..."` ⇒ 48 字节被摘要的输入。
    ///
    /// 这条判据同时钉住两件事：① 摘要口径不变 —— `yeban-mcp` 的 `yeban_import_audio`
    /// 把 `pcmHash` 直接交给客户端（`crates/yeban-mcp/src/domain/import_audio.rs:179`），
    /// 改口径是破坏性变更；② 分块喂入 `AssetHasher` 与一次性 `AssetHash::of_bytes` 等价。
    #[test]
    fn pcm_hash_matches_an_independent_python_oracle() {
        let facts = DecodeFacts {
            channels: 2,
            sample_rate: 48_000,
            pcm_format: PcmFormat::F32,
            declared_bit_depth: Some(32),
            declared_frames: Some(2),
            encoder_delay_frames: None,
            encoder_padding_frames: None,
            duration: Reconciliation::Exact,
        };
        let asset = DecodedAsset::new(facts, vec![0.25, -0.5, 0.125, -0.0625]);
        assert_eq!(asset.frame_count(), 2);
        assert_eq!(
            asset.pcm_hash().as_str(),
            "048efb90fd774c80d271f6c8c66c8ca2d867403f392c06d03b3b2f7e9b2bff70"
        );
    }

    /// 零样本资产的摘要 = **只有头部**的字节流（`channels` / `sample_rate` /
    /// `frame_count = 0`）。第二个 Python `hashlib` 预言机，覆盖"没有样本可喂"的分支。
    #[test]
    fn pcm_hash_of_a_zero_sample_asset_is_the_header_only_stream() {
        let facts = DecodeFacts {
            channels: 1,
            sample_rate: 44_100,
            pcm_format: PcmFormat::S16,
            declared_bit_depth: Some(16),
            declared_frames: Some(0),
            encoder_delay_frames: None,
            encoder_padding_frames: None,
            duration: Reconciliation::Exact,
        };
        let asset = DecodedAsset::new(facts, Vec::new());
        assert_eq!(asset.frame_count(), 0);
        assert_eq!(
            asset.pcm_hash().as_str(),
            "102adb8e3c1846e00eaf8f05a0b47812502894cdc8a1c932ccf94b09d47bd39a"
        );
    }

    /// 摘要与"分块方式"无关，且覆盖 `chunks(...)` 的**每一种余数**：1..=600 个样本，
    /// 逐个与测试内**独立重建**的规范字节流（`AssetHash::of_bytes` 一次性摘要）比对。
    ///
    /// 600 > 2 × 分块大小（256），因此跨块、块边界与尾巴三种情形都被走到；
    /// 120 000 个样本再加一条"很多块"（391 块）的读数。
    #[test]
    fn pcm_hash_is_independent_of_the_staging_chunking() {
        let facts = DecodeFacts {
            channels: 1,
            sample_rate: 48_000,
            pcm_format: PcmFormat::F32,
            declared_bit_depth: Some(32),
            declared_frames: None,
            encoder_delay_frames: None,
            encoder_padding_frames: None,
            duration: Reconciliation::DeclaredUnknown,
        };
        let check = |count: usize| {
            let samples: Vec<f32> = (0..count).map(|index| index as f32 * 0.031_25).collect();
            let asset = DecodedAsset::new(facts.clone(), samples.clone());
            // 独立重建规范字节流 —— 这份代码不看 `pcm_hash` 的实现，只按模块文档的口径写。
            let mut stream: Vec<u8> = Vec::new();
            stream.extend_from_slice(b"yeban.pcm.f32le.v1");
            stream.extend_from_slice(&1u16.to_le_bytes());
            stream.extend_from_slice(&48_000u32.to_le_bytes());
            stream.extend_from_slice(&u64::try_from(count).unwrap().to_le_bytes());
            for sample in &samples {
                stream.extend_from_slice(&sample.to_le_bytes());
            }
            assert_eq!(
                asset.pcm_hash(),
                AssetHash::of_bytes(&stream),
                "sample count {count}"
            );
        };
        for count in 1..=600usize {
            check(count);
        }
        check(120_000);
    }

    #[test]
    fn pcm_hash_covers_the_layout_header_not_just_the_samples() {
        let mut facts = DecodeFacts {
            channels: 1,
            sample_rate: 48_000,
            pcm_format: PcmFormat::S16,
            declared_bit_depth: Some(16),
            declared_frames: None,
            encoder_delay_frames: None,
            encoder_padding_frames: None,
            duration: Reconciliation::DeclaredUnknown,
        };
        let mono = DecodedAsset::new(facts.clone(), vec![0.1, -0.1, 0.2, -0.2]);
        facts.channels = 2;
        let stereo = DecodedAsset::new(facts.clone(), vec![0.1, -0.1, 0.2, -0.2]);
        assert_ne!(
            mono.pcm_hash(),
            stereo.pcm_hash(),
            "channel count is part of the key"
        );
        facts.channels = 1;
        facts.sample_rate = 44_100;
        let resampled = DecodedAsset::new(facts.clone(), vec![0.1, -0.1, 0.2, -0.2]);
        assert_ne!(
            mono.pcm_hash(),
            resampled.pcm_hash(),
            "sample rate is part of the key"
        );
        assert_eq!(mono.frame_count(), 4);
        assert_eq!(stereo.frame_count(), 2);
    }

    #[test]
    fn model_bit_depth_only_covers_the_normative_enum() {
        assert_eq!(PcmFormat::S16.model_bit_depth(), Some(BitDepth::Int16));
        assert_eq!(PcmFormat::S24.model_bit_depth(), Some(BitDepth::Int24));
        assert_eq!(PcmFormat::F32.model_bit_depth(), Some(BitDepth::Float32));
        // 规范枚举只有这三个变体 —— 其余一律 None，不发明近似值。
        for unsupported in [
            PcmFormat::U8,
            PcmFormat::U16,
            PcmFormat::U24,
            PcmFormat::U32,
            PcmFormat::S8,
            PcmFormat::S32,
            PcmFormat::F64,
        ] {
            assert_eq!(unsupported.model_bit_depth(), None, "{unsupported:?}");
        }
        assert_eq!(PcmFormat::F64.bit_depth(), 64);
        assert!(PcmFormat::F64.is_float());
        assert!(!PcmFormat::S32.is_float());
    }

    #[test]
    fn imported_asset_exposes_the_decoded_facts() {
        // 16-bit 夹具的合法区间是 -32768..=32767（`encode_int_samples` 会 `expect` 拦住越界值），
        // 所以这里用半量程附近的对称值，而不是 ±32768。
        let bytes = fixture(&[16_000, -16_000], 1);
        let imported =
            import_bytes(&bytes, "one-shot.wav", "CC0-1.0", &DecodeOptions::default()).unwrap();
        let decoded = &imported.decoded;
        assert_eq!(decoded.channels(), 1);
        assert_eq!(decoded.sample_rate(), 44_100);
        assert_eq!(decoded.model_bit_depth(), Some(BitDepth::Int16));
        assert_eq!(decoded.frame_count(), 2);
        assert!((decoded.duration_seconds() - 2.0 / 44_100.0).abs() < 1e-12);
    }

    #[test]
    fn import_never_decodes_bytes_that_are_over_budget() {
        let bytes = fixture(&[1, 2, 3, 4], 1);
        let strict = DecodeOptions {
            budget: crate::limits::PcmBudget {
                max_input_bytes: 8,
                ..crate::limits::PcmBudget::default()
            },
            ..DecodeOptions::default()
        };
        assert!(import_bytes(&bytes, "big.wav", "CC0-1.0", &strict).is_err());
    }

    #[test]
    fn imported_asset_is_reachable_only_through_shared_reads() {
        // 编译期判据：`DecodedAsset` 必须是 Send + Sync，且只能通过 &self 读。
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<DecodedAsset>();
        assert_send_sync::<ImportedAsset>();
        let bytes = fixture(&[1, 2, 3, 4], 1);
        let asset = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
        let shared = std::sync::Arc::new(asset);
        let clone = std::sync::Arc::clone(&shared);
        let handle = std::thread::spawn(move || clone.pcm_hash());
        assert_eq!(handle.join().unwrap(), shared.pcm_hash());
    }

    /// 判据（类别④ 参数极值／0 端点）：`DecodeFacts` 的两个数值字段取 **0** 与
    /// **类型极大值**时，只读访问器必须给出确定的值，而不是除零 panic / `NaN`。
    ///
    /// 逐项判定（量什么 → 怎么量 → 单位）：
    ///
    /// | 输入 | 访问器 | 判定 | 依据 |
    /// | :--- | :--- | :--- | :--- |
    /// | `channels = 0` | `frame_count()` | `0` 帧 | 访问器里有一条 `channels == 0` 的提前返回（现位于 `frame_count` 内） |
    /// | `channels = 0` | `duration_seconds()` | `0.0` 秒 | 同上，再走一次提前返回 |
    /// | `sample_rate = 0` | `duration_seconds()` | `0.0` 秒 | 访问器里有 `sample_rate == 0` 的提前返回（现位于 `duration_seconds` 内） |
    /// | `channels = u16::MAX` | `frame_count()` | `samples.len() / 65535` 帧 | 整数除法，无溢出 |
    /// | `sample_rate = u32::MAX` | `duration_seconds()` | 有限 `f64` | 一次 IEEE-754 除法，分母非 0 |
    ///
    /// 为什么必须钉：`channels == 0` 与 `sample_rate == 0` 在生产路径上是**不可达**的
    /// （[`DecodedAsset::new`] 是 crate 内唯一构造点，三处调用点都由「平面数 ≥ 1」与
    /// 「采样率闸门」保证），因此这两条提前返回是**防御性**的、没有被任何判据覆盖过。
    /// 未覆盖的防御分支会随重构静默消失；本判据把它的**可观测结论**（0 帧 / 0.0 秒）
    /// 写成字面值。删掉任一条提前返回都会在这里红，实测读数：
    ///
    /// - 删 `channels == 0` 那条 ⇒ `attempt to divide by zero`（`frame_count` 内做
    ///   `samples.len() / 0`）；
    /// - 删 `sample_rate == 0` 那条 ⇒ `left: inf` / `right: 0.0`（`4.0 / 0.0`）。
    ///
    /// 单位：帧（每声道采样数）、秒。
    #[test]
    fn facts_at_the_zero_and_max_endpoints_yield_defined_values() {
        let facts = DecodeFacts {
            channels: 0,
            sample_rate: 48_000,
            pcm_format: PcmFormat::F32,
            declared_bit_depth: Some(32),
            declared_frames: None,
            encoder_delay_frames: None,
            encoder_padding_frames: None,
            duration: Reconciliation::DeclaredUnknown,
        };
        let no_channels = DecodedAsset::new(facts.clone(), vec![0.25, -0.5, 0.125, -0.0625]);
        // 4 个样本、0 声道：不许做 `samples.len() / 0`。
        assert_eq!(no_channels.frame_count(), 0);
        assert_eq!(no_channels.duration_seconds(), 0.0);
        // 摘要仍必须能算出来（它把 `frame_count` 写进头部）。
        assert!(!no_channels.pcm_hash().as_str().is_empty());

        let zero_rate = DecodedAsset::new(
            DecodeFacts {
                // 声道数取 1，把"0 Hz"与"0 声道"两条守则分开量。
                channels: 1,
                sample_rate: 0,
                ..facts.clone()
            },
            vec![0.25, -0.5, 0.125, -0.0625],
        );
        // 1 声道、4 个样本、0 Hz：不许产生 `inf` / `NaN`。
        assert_eq!(zero_rate.frame_count(), 4);
        assert_eq!(zero_rate.duration_seconds(), 0.0);
        assert!(zero_rate.duration_seconds().is_finite());

        // 类型极大值一侧：`u16::MAX` 声道、`u32::MAX` Hz。
        let at_max = DecodedAsset::new(
            DecodeFacts {
                channels: u16::MAX,
                sample_rate: u32::MAX,
                ..facts
            },
            vec![0.25, -0.5, 0.125],
        );
        // 3 个样本 / 65535 声道 ⇒ 0 整帧（整数除法向下取整，不回绕）。
        assert_eq!(at_max.frame_count(), 0);
        // 0 帧 / (2^32 - 1) Hz = 0.0 秒，且必须是有限值。
        assert_eq!(at_max.duration_seconds(), 0.0);
        assert!(at_max.duration_seconds().is_finite());
        // 极大值的头部仍然进摘要，不 panic。
        assert!(!at_max.pcm_hash().as_str().is_empty());
    }

    /// 临时文件路径（进程号 + 名字，避免并行判据互相覆盖）。
    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("yeban-toctou-{}-{name}.wav", std::process::id()));
        path
    }

    /// 判据（`HD-24` 输入字节闸门 / TOCTOU 窗口）：**第二次**量到的长度也要过闸门。
    ///
    /// 量什么：`import_path_with_len` 在"注入的长度来源第二次报出超过预算的值"时的返回值。
    /// 怎么量：注入一个按序列返回 `[真实长度, 超限值]` 的闭包 —— 这就是"文件在两次测量
    /// 之间变大"的可判形态；预算设成只容得下真实长度。
    ///
    /// 读数（本机、debug 构建）：`InputTooLarge { bytes: <超限值>, limit: <真实长度> }`。
    ///
    /// 为什么需要它：`import_path` 在两个时刻量同一个文件，两次之间文件可能被改写。
    /// 第三批的注入普查实测：把第二次 `check_input_len(actual_len, …)` 的实参换成
    /// `declared_len` 之后**全部判据照旧通过** —— 这道闸门此前没有判据。
    ///
    /// 注入（实测）：把第二次 `check_input_len` 的实参换成 `declared_len` ⇒ 本条红。
    #[test]
    fn the_second_length_measurement_obeys_the_input_byte_budget() {
        let bytes = fixture(&[1, -2, 3, -4], 1);
        let real = u64::try_from(bytes.len()).expect("the fixture fits u64");
        let over = real + 1_000;
        let path = temp_path("second-gate");
        std::fs::write(&path, &bytes).expect("the fixture file must be writable");
        let options = DecodeOptions {
            budget: PcmBudget {
                max_input_bytes: real,
                ..PcmBudget::default()
            },
            ..DecodeOptions::default()
        };
        let mut measurements = 0u32;
        let outcome = import_path_with_len(&path, "CC0-1.0", &options, |_path| {
            measurements += 1;
            Ok(if measurements == 1 { real } else { over })
        });
        std::fs::remove_file(&path).expect("the fixture file must be removable");
        assert_eq!(measurements, 2, "the entry must measure the file twice");
        match outcome {
            Err(DecodeError::Budget(LimitViolation::InputTooLarge { bytes: got, limit })) => {
                assert_eq!(
                    (got, limit),
                    (over, real),
                    "the SECOND measurement must be the one that is gated"
                );
            }
            other => panic!("the second measurement must be gated, got {other:?}"),
        }
    }

    /// 判据（内容寻址的自洽性 / TOCTOU 窗口）：两次量到的长度与摘要实际读到的字节数
    /// **不一致**时必须拒绝，⛔ 不静默取其一。
    ///
    /// 量什么：注入的长度来源报出"第二次比真实文件**小**"时的返回值。
    /// 怎么量：闭包返回 `[真实长度, 真实长度 − 1]`，两者都在预算之内 —— 因此只有"三次测量
    /// 必须相等"这一步能拒它，闸门不会代劳。
    ///
    /// 读数（本机、debug 构建）：`Malformed`，文案同时点出摘要实际读到的字节数与第二次
    /// 量到的字节数。
    ///
    /// 为什么必须拒绝而不是取其一：CAS 的键是**被摘要的那份字节**的 SHA-256。长度对不上
    /// 意味着"被摘要的字节"与"现在磁盘上的这份文件"不是同一个东西，此时 `byte_len` 写成
    /// 任何一个数都会是假话（写小的那个 ⇒ 索引与摘要不符；写大的那个 ⇒ 与摘要覆盖的字节数
    /// 不符）。两条路都会让 `.yeban` 归档里的 `assets/{sha256}` 与 `byte_len` 互相矛盾。
    ///
    /// 注入（实测）：把"三次测量必须相等"那一步删掉 ⇒ 本条红（返回 `Ok`，且 `byte_len`
    /// 被写成那个不实的数）。
    #[test]
    fn a_file_that_changes_between_the_two_measurements_is_refused() {
        let bytes = fixture(&[1, -2, 3, -4], 1);
        let real = u64::try_from(bytes.len()).expect("the fixture fits u64");
        let shrunken = real - 1;
        let path = temp_path("mismatch");
        std::fs::write(&path, &bytes).expect("the fixture file must be writable");
        let mut measurements = 0u32;
        let outcome = import_path_with_len(&path, "CC0-1.0", &DecodeOptions::default(), |_path| {
            measurements += 1;
            Ok(if measurements == 1 { real } else { shrunken })
        });
        std::fs::remove_file(&path).expect("the fixture file must be removable");
        assert_eq!(measurements, 2);
        match outcome {
            Err(DecodeError::Malformed { detail }) => {
                assert!(
                    detail.contains(&real.to_string()) && detail.contains(&shrunken.to_string()),
                    "the refusal must name both numbers, got {detail}"
                );
                assert!(
                    detail.contains("changed"),
                    "the refusal must say the file changed, got {detail}"
                );
            }
            other => panic!(
                "a length that disagrees with what was hashed must be refused, not silently \\
                 picked, got {other:?}"
            ),
        }
    }

    /// 判据（对照组 / 正常文件零影响）：两次测量相等时，索引的 `byte_len` 等于**摘要实际
    /// 覆盖的字节数**，也等于真实文件长度。
    ///
    /// 量什么：真实临时文件经 `import_path` 得到的 `index.byte_len` 与 `index.hash`。
    /// 怎么量：写一份 WAV 到临时目录、导入、与 `bytes.len()` 和 `AssetHash::of_bytes` 比对。
    ///
    /// 读数（本机、debug 构建）：`byte_len == bytes.len()`、`hash == AssetHash::of_bytes(bytes)`。
    ///
    /// 为什么需要它：上面两条都在注入的长度来源上判。本条钉住**没有并发改写时三次测量
    /// 相等**，也就是"修复对正常文件零影响"这一侧。
    #[test]
    fn a_stable_file_records_the_length_that_was_actually_hashed() {
        let bytes = fixture(&[1, -2, 3, -4], 2);
        let path = temp_path("stable");
        std::fs::write(&path, &bytes).expect("the fixture file must be writable");
        let imported = import_path(&path, "CC0-1.0", &DecodeOptions::default())
            .expect("a stable file must import");
        std::fs::remove_file(&path).expect("the fixture file must be removable");
        assert_eq!(
            imported.index.byte_len,
            u64::try_from(bytes.len()).expect("the fixture fits u64")
        );
        assert_eq!(imported.index.hash, AssetHash::of_bytes(&bytes));
        assert_eq!(imported.decoded.frame_count(), 2);
    }

    /// 判据（枚举形状 / 裁决 R48）：[`PcmFormat`] 的**全部 10 个变体**都被覆盖。
    ///
    /// 量什么：每个变体的 `bit_depth()`、`is_float()`、`model_bit_depth()`（单位：位 / 布尔 /
    /// 规范枚举）。怎么量：一张 10 行的表，逐行比对；并由一个**无通配符 `match`** 把"枚举
    /// 新增变体"变成**编译期**错误。
    ///
    /// 读数（本机、debug 构建）：
    ///
    /// | 变体 | `bit_depth()` | `is_float()` | `model_bit_depth()` |
    /// | :--- | ---: | :--- | :--- |
    /// | `U8` | 8 | false | `None` |
    /// | `U16` | 16 | false | `None` |
    /// | `U24` | 24 | false | `None` |
    /// | `U32` | 32 | false | `None` |
    /// | `S8` | 8 | false | `None` |
    /// | `S16` | 16 | false | `Some(Int16)` |
    /// | `S24` | 24 | false | `Some(Int24)` |
    /// | `S32` | 32 | false | `None` |
    /// | `F32` | 32 | true | `Some(Float32)` |
    /// | `F64` | 64 | true | `None` |
    ///
    /// 为什么需要它：既有的 `model_bit_depth_only_covers_the_normative_enum` **列举**了
    /// 3 ＋ 7 个变体，但那是**手写清单**不是枚举保证 —— 加第 11 个变体时那份清单不会红。
    /// 本条的无通配符 `match` 会让它红。
    ///
    /// ⚠ **本 `match` 必须保持无通配符**：加上 `_ =>` 之后新增变体不会再红，而编译只出
    /// `unreachable_patterns` **警告**（裁决 R51 的实测读数：加 `_ => {}` 之后全部判据仍全绿）。
    ///
    /// 注入（实测）：把 `Self::F64 => (64, true, None)` 那一行改成 `(32, true, None)` ⇒
    /// 本条以 `F64` 行红。
    #[test]
    fn every_pcm_format_variant_is_covered() {
        /// 逐变体读三个属性。⛔ 不要加 `_ =>` 分支（见本条文档的 ⚠）。
        fn read(format: &PcmFormat) -> (u16, bool, Option<BitDepth>) {
            let _ = format; // 让"臂名"与"属性"分开，便于注入逐行改坏
            match format {
                PcmFormat::U8 => (8, false, None),
                PcmFormat::U16 => (16, false, None),
                PcmFormat::U24 => (24, false, None),
                PcmFormat::U32 => (32, false, None),
                PcmFormat::S8 => (8, false, None),
                PcmFormat::S16 => (16, false, Some(BitDepth::Int16)),
                PcmFormat::S24 => (24, false, Some(BitDepth::Int24)),
                PcmFormat::S32 => (32, false, None),
                PcmFormat::F32 => (32, true, Some(BitDepth::Float32)),
                PcmFormat::F64 => (64, true, None),
            }
        }
        let variants = [
            PcmFormat::U8,
            PcmFormat::U16,
            PcmFormat::U24,
            PcmFormat::U32,
            PcmFormat::S8,
            PcmFormat::S16,
            PcmFormat::S24,
            PcmFormat::S32,
            PcmFormat::F32,
            PcmFormat::F64,
        ];
        assert_eq!(variants.len(), 10, "the table must cover every arm");
        for format in variants {
            let (depth, is_float, model) = read(&format);
            assert_eq!(format.bit_depth(), depth, "{format:?}: bit_depth");
            assert_eq!(format.is_float(), is_float, "{format:?}: is_float");
            assert_eq!(
                format.model_bit_depth(),
                model,
                "{format:?}: model_bit_depth"
            );
        }
    }
}
