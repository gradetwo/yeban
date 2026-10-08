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
use crate::error::DecodeResult;
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
/// # Errors
///
/// 流自身的 I/O 失败原样上报（[`DecodeError::Io`]）。
fn hash_reader<R: Read>(mut reader: R) -> DecodeResult<AssetHash> {
    let mut hasher = AssetHasher::new();
    let mut buffer = vec![0u8; HASH_BUFFER_BYTES];
    loop {
        match reader.read(&mut buffer) {
            // 读到 0 字节即流结束。`read` 对空缓冲返回 0 是合法的，因此这里不靠
            // "缓冲是否填满"判断结束，只有 `Ok(0)` 才是终点。
            Ok(0) => return Ok(hasher.finalize()),
            Ok(filled) => hasher.update(&buffer[..filled]),
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
/// 摘要**写不进**逐字节的预算声明：读的过程中文件可能变大，因此第二遍的边界仍由
/// `decode_path` 自己的闸门把守（见 [`DecodeError::Budget`]）。
///
/// # Errors
///
/// 见 [`crate::error::DecodeError`]；文件读失败也会上报。
pub fn import_path(
    path: &Path,
    license: &str,
    options: &DecodeOptions,
) -> DecodeResult<ImportedAsset> {
    // 第一道闸门：先看声明长度，比打开文件更便宜（畸形/超大素材在这里就被拒）。
    let declared_len = std::fs::metadata(path)?.len();
    limits::check_input_len(declared_len, &options.budget)?;
    let hash = hash_reader(std::fs::File::open(path)?)?;
    // 第一遍读完后的真实长度：文件可能在 `metadata` 之后变大，绝不"先读了再说"。
    let actual_len = std::fs::metadata(path)?.len();
    limits::check_input_len(actual_len, &options.budget)?;
    let decoded = decode::decode_path(path, options)?;
    let index = AssetMetadata {
        hash,
        original_path: path.to_string_lossy().into_owned(),
        byte_len: actual_len,
        media_kind: MediaKind::Audio,
        license: license.to_owned(),
    };
    Ok(ImportedAsset { index, decoded })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::decode_bytes;
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
}
