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

    /// 判据（类别④ 参数极值）：三个数值参数取 `u64::MAX`／`u64::MAX - 1` 时，对账结论
    /// 必须与"绝对差 ≤ 容差"这条定义逐条一致，**不得回绕**、不得 panic。
    ///
    /// 逐项判定（量什么 → 单位 → 结论）：
    ///
    /// | `declared` | `decoded` | `tolerance` | 绝对差（帧） | 期望结论 | 依据 |
    /// | :--- | :--- | :--- | :--- | :--- | :--- |
    /// | `u64::MAX` | `u64::MAX` | `0` | `0` | `Exact` | 差值定义 |
    /// | `u64::MAX` | `0` | `u64::MAX` | `u64::MAX` | `WithinTolerance` | 闭区间：差 = 容差 |
    /// | `1` | `u64::MAX` | `u64::MAX - 1` | `u64::MAX - 1` | `WithinTolerance` | 闭区间边界 |
    /// | `1` | `u64::MAX` | `u64::MAX - 2` | `u64::MAX - 1` | `OutsideTolerance` | 差比容差大 1 |
    /// | `u64::MAX` | `1` | `0` | `u64::MAX - 1` | `OutsideTolerance` | 方向不影响结论（对称） |
    /// | 任意 ≥ 1 | 任意 | `u64::MAX` | ≤ `u64::MAX` | 永不 `OutsideTolerance` | 容差是最大值 |
    ///
    /// 量的是**帧**。为什么必须钉：本模块用 `u64::abs_diff`（差值无符号、不回绕），
    /// 若换成 `declared - decoded`，第 3、4、5 行就会回绕 —— 实测（debug 构建）把这一行
    /// 注入成回绕减法后，本条以 `attempt to subtract with overflow` 红；根 `Cargo.toml` 的
    /// `[profile.release] overflow-checks = true` 让同一条在 release 下同样会炸（该 profile
    /// 行是读出来的，未在本机跑 release 构建）。那正是"不可信输入 ⇒ 明确错误"红线的反面。
    #[test]
    fn reconcile_at_the_u64_endpoints_matches_the_definition() {
        assert_eq!(
            reconcile(Some(u64::MAX), u64::MAX, 0),
            Reconciliation::Exact
        );
        assert_eq!(
            reconcile(Some(u64::MAX), 0, u64::MAX),
            Reconciliation::WithinTolerance { delta: u64::MAX }
        );
        assert_eq!(
            reconcile(Some(1), u64::MAX, u64::MAX - 1),
            Reconciliation::WithinTolerance {
                delta: u64::MAX - 1
            }
        );
        assert_eq!(
            reconcile(Some(1), u64::MAX, u64::MAX - 2),
            Reconciliation::OutsideTolerance {
                declared: 1,
                decoded: u64::MAX,
                tolerance: u64::MAX - 2,
                delta: u64::MAX - 1,
            }
        );
        assert!(!reconcile(Some(u64::MAX), 1, 0).is_reconciled());
        // 最大容差永不判失败（声明值 0 仍走"未知"，因此下限取 1）。
        for declared in [1u64, 2, u64::MAX - 1, u64::MAX] {
            for decoded in [0u64, 1, u64::MAX - 1, u64::MAX] {
                assert!(
                    reconcile(Some(declared), decoded, u64::MAX).is_reconciled(),
                    "declared {declared} / decoded {decoded} with the maximal tolerance"
                );
            }
        }
        // 与定义式逐条复算（不受上表手写值影响）。
        for declared in [1u64, 2, 7, u64::MAX / 2, u64::MAX - 1, u64::MAX] {
            for decoded in [0u64, 2, 7, u64::MAX / 2, u64::MAX - 1, u64::MAX] {
                for tolerance in [0u64, 1, u64::MAX - 1, u64::MAX] {
                    let delta = declared.abs_diff(decoded);
                    let expected_reconciled = delta <= tolerance;
                    assert_eq!(
                        reconcile(Some(declared), decoded, tolerance).is_reconciled(),
                        expected_reconciled,
                        "declared {declared} / decoded {decoded} / tolerance {tolerance}"
                    );
                }
            }
        }
    }

    /// 判据（诊断文案黄金表）：[`Mismatch`] 渲染出逐字固定的文案。
    ///
    /// 为什么需要它：注入普查把 `container declares {} frames but {} frames were decoded`
    /// 改成 `... while ...`，**全部判据照旧通过** —— 对账失败的文案 template 此前没有判据。
    /// 这句会随 [`crate::error::DecodeError::DurationMismatch`] 一路进 MCP 响应体。
    ///
    /// 注入（实测）：把 `but` 改成 `while` ⇒ 本条红。
    #[test]
    fn the_frame_mismatch_renders_its_documented_text() {
        let mismatch = Mismatch {
            declared: 768,
            decoded: 512,
            tolerance: 0,
            delta: 256,
        };
        assert_eq!(
            mismatch.to_string(),
            "container declares 768 frames but 512 frames were decoded (delta 256, tolerance 0)"
        );
        // 四个字段都必须出现在文案里（漏一个就是"数字丢了"，而这正是该类型的职责）。
        let rendered = mismatch.to_string();
        for field in ["768", "512", "256", "0"] {
            assert!(rendered.contains(field), "the rendering must keep {field}");
        }
    }
}
