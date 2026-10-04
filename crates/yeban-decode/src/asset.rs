//! 不可变资产：解码结果、内容寻址与 `yeban-model` 的对接。
//!
//! 这一层回答的是产品问题："导入一个文件 → 得到一个内容寻址的不可变 PCM 资产"。
//! 链路只有三步，每一步都可以单独核验：
//!
//! 1. **原始字节 → [`yeban_model::AssetHash`]**：`AssetHash::of_bytes` 的 SHA-256 是
//!    CAS 的键（`.yeban` 归档里的 `assets/{sha256}`），见 [MODEL-AST-007]；
//! 2. **原始字节 → [`DecodedAsset`]**：解码事实（声道/采样率/位深/帧数/时长对账）+ 交织 `f32` PCM；
//! 3. **解码结果 → [`DecodedAsset::pcm_hash`]**：把"解出来的样本"本身也做成内容摘要，
//!    用来给 [ARCH-DET-001] 的"同输入 → 同输出"提供一条**可比较的指纹**
//!    （逐样本 `to_bits()` 比较在高帧数下太贵，摘要足够红）。
//!
//! 不可变性 [ARCH-TOP-002]：`DecodedAsset` 只提供只读访问器，没有任何 `&mut` 出口，
//! 因此它可以被 `Arc` 起来在实时线程里只读消费 —— 实时回调路径上永远不会调用本模块的
//! 任何构造/解码函数。

use std::path::Path;

use yeban_model::{AssetHash, AssetMetadata, BitDepth, MediaKind};

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
    #[must_use]
    pub fn pcm_hash(&self) -> AssetHash {
        let mut bytes = Vec::with_capacity(32 + self.samples.len() * 4);
        bytes.extend_from_slice(PCM_HASH_DOMAIN);
        bytes.extend_from_slice(&self.facts.channels.to_le_bytes());
        bytes.extend_from_slice(&self.facts.sample_rate.to_le_bytes());
        bytes.extend_from_slice(&self.frame_count().to_le_bytes());
        for sample in &self.samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        AssetHash::of_bytes(&bytes)
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

/// 从磁盘路径导入：先把文件读进内存（受 `max_input_bytes` 约束）以计算摘要，再解码。
///
/// 为什么不流式哈希：`AssetHash::of_bytes` 的契约是"对完整字节做 SHA-256"，
/// 而流式哈希需要在解码器之外再维护一份 `sha2::Sha256` 状态机。当前口径是
/// **2 GiB 以内走内存**，超过上限直接拒绝 —— 见 notes 的 `needs` 条目。
///
/// # Errors
///
/// 见 [`crate::error::DecodeError`]；文件读失败也会上报。
pub fn import_path(
    path: &Path,
    license: &str,
    options: &DecodeOptions,
) -> DecodeResult<ImportedAsset> {
    let declared_len = std::fs::metadata(path)?.len();
    limits::check_input_len(declared_len, options.max_input_bytes)?;
    let bytes = std::fs::read(path)?;
    // 文件可能在元数据检查之后变大：读到之后再核一次，绝不"先读了再说"。
    let actual_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    limits::check_input_len(actual_len, options.max_input_bytes)?;
    let original_path = path.to_string_lossy().into_owned();
    import_bytes(&bytes, &original_path, license, options)
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
            max_input_bytes: 8,
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
