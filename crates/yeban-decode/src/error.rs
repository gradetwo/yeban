//! 统一解码错误类型。
//!
//! **不可信输入边界零 panic** [AGENTS.md §2 红线 7 的离线侧对应约束]：畸形文件、
//! 截断流、未知编码、声明与实际不符、超预算的尺寸，全部走 `Result`。
//! 本 crate 里**没有** `unwrap` / `expect` / `panic!` / 索引越界可能出现在输入驱动的
//! 路径上（夹具与判据里的 `expect` 属于测试代码，不算）。
//!
//! 规范来源 (Normative): `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §5.3
//! [ARCH-SEC-003]（解压炸弹/畸形归档的防御必须"拒绝并报错"）。

use thiserror::Error;

use crate::duration::Mismatch;
use crate::limits::{LenContractViolation, LimitViolation};

/// `yeban-decode` 的统一错误类型。
///
/// 不派生 `PartialEq`：`Io` 变体携带 [`std::io::Error`]，它不实现 `PartialEq`。
/// 判据用 `matches!` 匹配变体，而不是比较整个错误值。
#[derive(Debug, Error)]
pub enum DecodeError {
    /// 底层 I/O 失败（文件打不开、读过程中断、seek 失败）。
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// 容器格式无法识别（symphonia 的探测器没有认领该流）。
    #[error("unrecognised or unsupported container format")]
    UnsupportedFormat,

    /// 流里没有可解码的音频轨道。
    #[error("container has no decodable audio track")]
    NoAudioTrack,

    /// 轨道缺少编解码参数（symphonia 判定该轨道不可播放）。
    #[error("track is missing codec parameters")]
    MissingCodecParameters,

    /// 本构建没有启用该编解码器所需的 cargo feature。
    #[error("codec not enabled in this build: {detail}")]
    UnsupportedCodec {
        /// symphonia 给出的原文说明。
        detail: String,
    },

    /// 轨道没有上报采样率。
    #[error("track does not report a sample rate")]
    MissingSampleRate,

    /// 流中途要求重建解码器（`ResetRequired`，例如串联 Ogg 物理流）。
    ///
    /// 本 crate 明确拒绝而不是"重启一次试试"：重启会让资产内容依赖流的内部结构，
    /// 破坏 [ARCH-DET-001] 的"同输入 → 同输出"。
    #[error("stream requires a decoder reset mid-decode (chained stream)")]
    ResetRequired,

    /// 数据畸形（symphonia 的 `DecodeError` / `LimitError` 文本）。
    #[error("malformed stream: {detail}")]
    Malformed {
        /// 上游给出的原文说明。
        detail: String,
    },

    /// 解码在开始之前就被拒绝：超出了尺寸/布局预算。
    #[error("asset exceeds the decode budget: {0}")]
    Budget(#[from] LimitViolation),

    /// 流在解码过程中改变了形状（声道数变了、`samples != frames × channels`）。
    #[error("stream layout is inconsistent: {detail}")]
    InconsistentLayout {
        /// 具体哪里不一致。
        detail: String,
    },

    /// 流没有解出任何采样（空资产）。
    ///
    /// 口径：夜半不导入零长资产 —— 一个 0 帧的剪辑在时间线上没有意义，却会一路
    /// 传递到渲染与 UI 层。宁可在这里明确拒绝。
    #[error("stream decoded to zero audio frames")]
    EmptyStream,

    /// 容器声明的总帧数与解出的帧数不一致 [ARCH-DET-001]。
    #[error("declared duration disagrees with decoded frames: {0}")]
    DurationMismatch(#[from] Mismatch),

    /// 重采样器无法按给定参数构造。
    #[error("resampler could not be configured: {detail}")]
    ResamplerConfiguration {
        /// 上游给出的原文说明（`ResamplerConstructionError` 的 `Debug` 形式）。
        detail: String,
    },

    /// 重采样过程失败（缓冲布局不符、比例越界等）。
    #[error("resampling failed: {detail}")]
    Resampling {
        /// 上游给出的原文说明（`ResampleError` 的 `Debug` 形式）。
        detail: String,
    },

    /// 重采样输出帧数不满足长度契约 [ARCH-DSP-002]。
    #[error("resampler violated the length contract: {0}")]
    LengthContract(#[from] LenContractViolation),
}

impl DecodeError {
    /// 把 symphonia 的错误映射为本 crate 的错误。
    ///
    /// 单独抽出来是为了让"哪个上游错误落到哪个变体"这条映射能被逐条审阅，
    /// 而不是散落在解码循环里。
    #[must_use]
    pub(crate) fn from_symphonia(err: &symphonia::core::errors::Error) -> Self {
        use symphonia::core::errors::Error as S;
        match err {
            S::IoError(io) => Self::Io(std::io::Error::new(io.kind(), io.to_string())),
            S::DecodeError(detail) => Self::Malformed {
                detail: (*detail).to_owned(),
            },
            S::Unsupported(detail) => Self::UnsupportedCodec {
                detail: (*detail).to_owned(),
            },
            S::LimitError(detail) => Self::Malformed {
                detail: format!("decoder limit reached: {detail}"),
            },
            S::SeekError(kind) => Self::Malformed {
                detail: format!("seek failed: {kind:?}"),
            },
            S::ResetRequired => Self::ResetRequired,
            // `symphonia::core::errors::Error` 是 `#[non_exhaustive]`：上游新增变体时
            // 我们保守地归到"畸形流"，而不是悄悄当成成功。
            other => Self::Malformed {
                detail: format!("{other:?}"),
            },
        }
    }
}

/// 本 crate 的 `Result` 别名。
pub type DecodeResult<T> = Result<T, DecodeError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symphonia_errors_map_onto_typed_variants() {
        use symphonia::core::errors::Error as S;
        assert!(matches!(
            DecodeError::from_symphonia(&S::Unsupported("no decoder")),
            DecodeError::UnsupportedCodec { .. }
        ));
        assert!(matches!(
            DecodeError::from_symphonia(&S::DecodeError("bad frame")),
            DecodeError::Malformed { .. }
        ));
        assert!(matches!(
            DecodeError::from_symphonia(&S::ResetRequired),
            DecodeError::ResetRequired
        ));
        assert!(matches!(
            DecodeError::from_symphonia(&S::LimitError("too many frames")),
            DecodeError::Malformed { .. }
        ));
        // 消息里必须保留上游原文，否则 CI 日志无法定位。
        let rendered = DecodeError::from_symphonia(&S::Unsupported("no decoder")).to_string();
        assert!(rendered.contains("no decoder"), "got {rendered}");
    }

    #[test]
    fn budget_violations_convert_without_losing_the_number() {
        let err: DecodeError = LimitViolation::TooManyChannels {
            channels: 99,
            limit: 64,
        }
        .into();
        assert!(matches!(err, DecodeError::Budget(_)));
        assert!(err.to_string().contains("99"));
    }
}
