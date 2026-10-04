//! 声明时长与解码帧数的对账 —— **零第三方依赖**的纯逻辑层。
//!
//! 为什么单独一层：容器（WAV 的 `data` 块尺寸、FLAC 的 `STREAMINFO.total_samples`）
//! 会**声明**一个总帧数，而解码器实际解出的是另一个数。两者不一致时，产品行为必须
//! 是"明确报错"而不是"猜一个"—— 猜出来的时长会让卷帘视口、剪辑边界与自动化对齐全部偏移。
//!
//! 规范来源 (Normative): `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §5.1
//! [ARCH-DET-001]（同输入 → 同输出；时长是资产元数据的一部分，必须可信）。
//!
//! 边界: 本模块只做整数对账，不认识容器、不认识 symphonia。默认容差是 **0 帧**
//! （本 crate 只启用无损编解码与按包解码的 Vorbis），见
//! [`crate::limits::DURATION_TOLERANCE_FRAMES`]。

use std::error::Error;
use std::fmt;

/// 对账结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reconciliation {
    /// 容器没有可用的声明（`None`，或声明为 0 —— 容器规范里 0 表示"未知"）。
    DeclaredUnknown,
    /// 声明值与解出值逐帧一致。
    Exact,
    /// 声明值与解出值在调用方给出的容差内一致。
    WithinTolerance {
        /// 两者的绝对差。
        delta: u64,
    },
    /// 超出容差 —— 调用方必须据此报错。
    OutsideTolerance {
        /// 容器声明的帧数。
        declared: u64,
        /// 解码器实际解出的帧数。
        decoded: u64,
        /// 生效的容差（帧）。
        tolerance: u64,
        /// 绝对差。
        delta: u64,
    },
}

impl Reconciliation {
    /// 是否被认为"对账通过"。
    #[must_use]
    pub fn is_reconciled(self) -> bool {
        !matches!(self, Self::OutsideTolerance { .. })
    }

    /// 把结论转成 `Result`：失败时给出可直接进错误消息的结构化细节。
    ///
    /// # Errors
    ///
    /// 当结论是 [`Reconciliation::OutsideTolerance`] 时返回 [`Mismatch`]。
    pub fn into_result(self) -> Result<Self, Mismatch> {
        match self {
            Self::OutsideTolerance {
                declared,
                decoded,
                tolerance,
                delta,
            } => Err(Mismatch {
                declared,
                decoded,
                tolerance,
                delta,
            }),
            other => Ok(other),
        }
    }
}

/// 对账失败的细节（携带双方数字，让 CI 日志能直接定位）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mismatch {
    /// 容器声明的帧数。
    pub declared: u64,
    /// 解码器实际解出的帧数。
    pub decoded: u64,
    /// 生效的容差（帧）。
    pub tolerance: u64,
    /// 绝对差。
    pub delta: u64,
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "container declares {} frames but {} frames were decoded \
             (delta {}, tolerance {})",
            self.declared, self.decoded, self.delta, self.tolerance
        )
    }
}

impl Error for Mismatch {}

/// 把容器声明的总帧数与解码得到的帧数对账。
///
/// 语义（写死进判据）：
/// - `declared` 为 `None` 或 `Some(0)` ⇒ [`Reconciliation::DeclaredUnknown`]（0 在容器规范里
///   表示"未知"，不得当成"零长"去和真实帧数比较）；
/// - `|declared - decoded| == 0` ⇒ [`Reconciliation::Exact`]；
/// - `0 < |declared - decoded| <= tolerance` ⇒ [`Reconciliation::WithinTolerance`]；
/// - 否则 ⇒ [`Reconciliation::OutsideTolerance`]（调用方据此报错，绝不"取较小者"）。
#[must_use]
pub fn reconcile(declared: Option<u64>, decoded: u64, tolerance: u64) -> Reconciliation {
    let Some(declared) = declared else {
        return Reconciliation::DeclaredUnknown;
    };
    if declared == 0 {
        return Reconciliation::DeclaredUnknown;
    }
    let delta = declared.abs_diff(decoded);
    if delta == 0 {
        Reconciliation::Exact
    } else if delta <= tolerance {
        Reconciliation::WithinTolerance { delta }
    } else {
        Reconciliation::OutsideTolerance {
            declared,
            decoded,
            tolerance,
            delta,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_declaration_is_not_treated_as_zero_length() {
        assert_eq!(reconcile(None, 512, 0), Reconciliation::DeclaredUnknown);
        assert_eq!(reconcile(Some(0), 512, 0), Reconciliation::DeclaredUnknown);
        assert!(reconcile(Some(0), 512, 0).is_reconciled());
    }

    #[test]
    fn exact_match_is_exact() {
        assert_eq!(reconcile(Some(512), 512, 0), Reconciliation::Exact);
    }

    #[test]
    fn mismatch_beyond_tolerance_is_a_structural_error() {
        let outcome = reconcile(Some(511), 512, 0);
        assert!(!outcome.is_reconciled());
        let err = outcome.into_result().unwrap_err();
        assert_eq!(
            err,
            Mismatch {
                declared: 511,
                decoded: 512,
                tolerance: 0,
                delta: 1
            }
        );
        assert!(err.to_string().contains("declares 511 frames"));
    }

    #[test]
    fn tolerance_is_inclusive_when_the_caller_relaxes_it() {
        assert_eq!(
            reconcile(Some(511), 512, 1),
            Reconciliation::WithinTolerance { delta: 1 }
        );
        assert!(reconcile(Some(511), 512, 1).into_result().is_ok());
        assert!(!reconcile(Some(500), 512, 1).is_reconciled());
    }

    #[test]
    fn a_single_frame_mismatch_is_not_masked_by_the_unknown_rule() {
        // 声明为 0 且解出 0：按"未知"处理（不是 Exact），调用方另有 EmptyStream 口径。
        assert_eq!(reconcile(Some(0), 0, 0), Reconciliation::DeclaredUnknown);
        // 声明为 3 却解出 0：必须报错。
        assert!(!reconcile(Some(3), 0, 0).is_reconciled());
    }
}
