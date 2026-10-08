//! 混音链的两件**母线级**器件：声相定律与母线峰值限制器。
//! [ARCH-DSP-001, ARCH-RT-001, ARCH-DET-001, MODEL-AST-002]
//!
//! `line/engine-sound` 让引擎真的出声了，但母线汇流只是**等增益复制**
//! （`sum_into_bus` 把单声道轨同时加到 L/R），于是 `TrackParams::pan` 与
//! `audio_config.pan_law` 都不影响输出；母线也没有任何峰值约束，实测峰值
//! **1.0058**（三个首尾相接的四分音符，前一个的释放尾与后一个的起音叠加）
//! 就是限制器缺席的直接后果。本模块补上这两件：
//!
//! ```text
//! 逐轨: synth.render_track → track_scratch(单声道, 声相之前) → 逐轨电平
//!         │
//!         │ pan_gains(TrackParams::pan)          ← 构造期算好的 (cos θ, sin θ)
//!         ▼
//!      母线左 += m × gain_l ; 母线右 += m × gain_r
//!         │
//!         ▼
//!      BusLimiter::process_stereo(...)            ← 前瞻 + 立即攻击/平滑释放
//!         │                                        （实现住在 yeban_dsp::limiter）
//!         ▼
//!      母线电平（限制**之后**）→ cpal / NullBackend
//! ```
//!
//! ## 1. 声相定律的口径
//!
//! 默认律 [`PanLaw::ConstantPowerMinus3dB`] 取**等功率**曲线
//! `gain_l = cos θ`、`gain_r = sin θ`、`θ = (pan + 1) · π/4`：
//!
//! | `pan` | `gain_l` | `gain_r` | 备注 |
//! | :--- | :--- | :--- | :--- |
//! | −1.0 | 1.0 | 0.0 | 全左：右声道**逐位**静音 |
//! | 0.0 | √2/2 ≈ 0.70710678 | √2/2 | 居中 = 每声道 **−3.01 dB** |
//! | +1.0 | 0.0 | 1.0 | 全右 |
//!
//! 居中给出 `(√2/2, √2/2)` 而不是 `(1, 1)`：这正是"等功率 **−3 dB**"这个名字的
//! 物理含义（切到单声道时总能量不变），也是 `yeban-mcp` 离线渲染
//! （`src/domain/render.rs::pan_gains`）与 `docs/ledger/mcp-render-notes.md` 已经
//! 落地的同一口径 —— **本模块刻意抄它，不另立第二套定义**。
//!
//! ⚠ **另三个变体明确未实现**（[ADR-0001 **D43**]：1.0.0 之前不留历史包袱、
//! 也不为"猜一个合理的值"写代码）。`PanLaw` 的四个变体在规范里**只有枚举名**、
//! 没有曲线定义：
//!
//! | 变体 | 缺什么定义 | 本模块的处置 |
//! | :--- | :--- | :--- |
//! | `ConstantPowerMinus3dB`（模型层 `#[default]`） | —— | **已实现**：上表 |
//! | `Linear` | 居中给 `(0.5, 0.5)`（−6.02 dB）还是 `(1, 1)`（不衰减） | **未实现** |
//! | `ConstantPowerMinus4_5dB` | "−4.5 dB"指居中额外衰减，还是另一族曲线？ | **未实现** |
//! | `ConstantPowerMinus6dB` | 同上（若指居中衰减 ⇒ `(0.5, 0.5)`） | **未实现** |
//!
//! "未实现"的具体含义（它是**响亮**的，不是静默的）：
//!
//! - [`pan_gains`] 的 `match` **逐变体显式列出**三个未实现分支，每个分支上方写明
//!   "缺的是哪一条定义"，并**回落到默认律的曲线**（**不是**回落到 line 之前的
//!   "等增益复制" —— 那会让电平涨 3 dB，是更坏的行为）；
//! - 判据 `unimplemented_pan_laws_fall_back_to_the_documented_curve` 把这件事
//!   **钉死**：三个变体与默认律**输出相同**（一个断言），并且
//!   `yeban_model::PanLaw` 有第四个变体时 `from_model` 是**穷举 match**
//!   ⇒ 编译失败而不是悄悄归类；
//! - 台账 §1.3 与 needs N1 记着"缺的是哪一条定义"。补齐定义时只改
//!   [`pan_gains`] 一个函数（增益在构造期算，实时侧不受影响）。
//!
//! ## 2. 母线限制器：**实现已上移到 `yeban-dsp`**（本模块只 re-export）
//!
//! `BusLimiter` 的算法（前瞻窗口、立即攻击、速率上限释放、软膝天花板）**一行未改**，
//! 现在住在 [`yeban_dsp::limiter`]（`crates/yeban-dsp/src/limiter.rs`）。
//! 本模块只做两件事：
//!
//! ```text
//! pub use yeban_dsp::limiter::Limiter as BusLimiter;   // 类型（含方法与 Default）
//! pub use yeban_dsp::limiter::{LIMITER_CEILING, ...};  // 五个规范常量
//! ```
//!
//! 上移的裁决与方式（**逐条可查**）：
//!
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:107` 把限制器算法的目的地
//!   写成 `crates/yeban-dsp/src/limiter.rs`；
//! - `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md:465`（D44(c)）写明
//!   上移之后 `yeban-engine` 侧**保留 `pub use` re-export**、**不得留第二份实现**
//!   —— 与 `level.rs` 上移 `yeban_dsp::meter` 是**同一条处理方式**；
//! - 因此本模块**没有**限制器的类型定义、没有它的固有实现块、没有两个私有辅助
//!   函数（软膝映射与 `NaN` 归零）、也没有那五个规范常量。这件事由
//!   `tests::engine_mixer_module_has_no_second_limiter_implementation`
//!   （`include_str!("mixer.rs")` 源码级检查）与
//!   `tests::engine_bus_limiter_is_literally_the_dsp_limiter`（类型/函数/常量同一性）
//!   两条判据机械钉住。
//!
//! ### 2.1 调用形状的变化（唯一的一处）
//!
//! 原 `BusLimiter::apply(&mut AudioBlock<FRAMES>, frames)` 的签名里含**本 crate 的块类型**
//! ⇒ 它不可能跟着实现一起上移（`yeban-dsp` 不认识 `AudioBlock`）。上移时入口改成
//! `process_stereo(&mut [f32], &mut [f32]) -> usize`（与 `yeban_dsp::compressor`、
//! `yeban_dsp::channel_strip` 的既有风格一致），语义**不变**：
//!
//! - `frames == 0` ⇒ 空操作（不推进状态）；
//! - `frames` 超界 ⇒ 按"两条切片长度的较小者"钳制（原实现按块容量钳制）。
//!
//! `crates/yeban-engine/src/rt.rs` 的调用点因此写成
//! `limiter.process_stereo(&mut left[..frames], &mut right[..frames])`；
//! 其余调用点（判据）用同样的方式把本量子的帧数切出来。
//!
//! ### 2.2 尚未回填的延迟（照原样登记，本票不改）
//!
//! 限制器的 [`LOOKAHEAD_SAMPLES`] = **33 帧**延迟**仍未**回填进
//! [`crate::graph::LatencyTable`]（[ADR-0001 D44(b)] 要求回填）。上移**不改**这件事，
//! 也不把 33 帧塞进别的路径 —— 它仍然是待办，不是已完成的口径。

// 母线限制器的唯一实现住在 yeban-dsp；这里只做转发（类型 + 五个规范常量）。
// ⚠ 名字保留 `BusLimiter`：它是引擎内全部调用点与判据使用的公共面，改名会让
// "搬家"对调用方可见（与 `level.rs` 保留模块名同理）。
pub use yeban_dsp::limiter::{
    LIMITER_CEILING, LIMITER_LATENCY_FRAMES, LIMITER_RELEASE_PER_SAMPLE, LIMITER_THRESHOLD,
    LOOKAHEAD_SAMPLES, Limiter as BusLimiter,
};

/// 声相衰减律的**引擎侧**枚举。
///
/// ⚠ 这是**引擎侧的临时形状**，不是模型层的第二份定义：模型层已经有
/// `yeban_model::PanLaw`（`audio_config.pan_law` 是它的唯一来源），
/// 本枚举只是把那个规范枚举的**四个变体名**在引擎侧重述一遍，
/// 以便 `yeban-engine` 在 `--no-default-features` 下也能独立编译与测试。
/// 两者的对应关系由 [`PanLaw::from_model`] 与
/// `tests/mix_render.rs::engine_pan_law_names_match_the_model`（穷举四个变体）钉住。
///
/// 模型线的对齐项：若将来把 `pan_law` 直接放进 `EngineSnapshot` 的类型签名
/// （即引擎公开依赖 `yeban_model::PanLaw`），本枚举应当**整体删除**，
/// 只保留 `pan_gains` 的曲线实现。见 `docs/ledger/engine-mix-notes.md` 的 needs。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PanLaw {
    /// 线性（`gain_l = (1 - pan)/2`、`gain_r = (1 + pan)/2`）。
    ///
    /// ⚠ **未实现**：规范只给了枚举名，居中处是 `(0.5, 0.5)`（即 −6.02 dB）
    /// 还是 `(1, 1)`（线性不衰减）没有定义。当前与默认律同曲线（见模块文档 §1）。
    Linear,
    /// 等功率 −3 dB：[`PanLaw::default`]，本切片**已实现**的唯一口径。
    #[default]
    ConstantPowerMinus3dB,
    /// 等功率 −4.5 dB。⚠ **未实现**（缺"−4.5 dB 指什么"的定义），当前同默认律。
    ConstantPowerMinus4_5dB,
    /// 等功率 −6 dB。⚠ **未实现**（缺定义），当前同默认律。
    ConstantPowerMinus6dB,
}

impl PanLaw {
    /// 模型层 `yeban_model::PanLaw` → 引擎侧枚举（穷举，无 `_` 兜底分支）。
    ///
    /// 刻意**不写通配分支**：模型层将来新增变体时，这里会**编译失败**而不是
    /// 悄悄把所有新变体当成默认律。
    #[must_use]
    pub const fn from_model(law: yeban_model::PanLaw) -> Self {
        match law {
            yeban_model::PanLaw::Linear => Self::Linear,
            yeban_model::PanLaw::ConstantPowerMinus3dB => Self::ConstantPowerMinus3dB,
            yeban_model::PanLaw::ConstantPowerMinus4_5dB => Self::ConstantPowerMinus4_5dB,
            yeban_model::PanLaw::ConstantPowerMinus6dB => Self::ConstantPowerMinus6dB,
        }
    }
}

/// 声相增益：`(左, 右)`，由 `pan ∈ [-1, 1]` 与衰减律给出。
///
/// **必须只在构造期调用**（控制线程）：它含 `cos`/`sin`，属 [ADR-0001 D32] 的
/// **超越函数类**（4096 ulp 预算）。实时侧只读 [`crate::snapshot::TrackParams::pan_gains`]
/// 里预计算好的那两个 `f32`。
///
/// 非有限输入按 `pan = 0`（居中）处理；越界输入先钳到 `[-1, 1]`。
/// 这两条与模型层的 [`TrackV3::pan`](yeban_model::TrackV3::pan) 语义一致：
/// 声相是一个有界的界面参数，退化值不该让整条链路变 `NaN`。
#[must_use]
pub fn pan_gains(pan: f32, law: PanLaw) -> (f32, f32) {
    let clamped = if pan.is_finite() {
        pan.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    // θ = (pan + 1) · π/4：pan = -1 ⇒ 0（全左），pan = 0 ⇒ π/4（居中），pan = +1 ⇒ π/2（全右）。
    let angle = (clamped + 1.0) * core::f32::consts::FRAC_PI_4;
    match law {
        PanLaw::ConstantPowerMinus3dB => (angle.cos(), angle.sin()),
        // ⚠ 未实现：缺"居中给 (0.5, 0.5)（−6.02 dB）还是 (1, 1)（不衰减）"的定义。
        // 回落到**默认律的曲线**（而不是 line 之前的等增益复制 —— 那会涨 3 dB）。
        PanLaw::Linear => (angle.cos(), angle.sin()),
        // ⚠ 未实现：缺"−4.5 dB 指居中额外衰减，还是另一族曲线"的定义。
        PanLaw::ConstantPowerMinus4_5dB => (angle.cos(), angle.sin()),
        // ⚠ 未实现：同上（若指居中衰减 ⇒ 居中 `(0.5, 0.5)`）。
        PanLaw::ConstantPowerMinus6dB => (angle.cos(), angle.sin()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 声相定律：左/中/右三点的增益必须落在**声明值**上（公式 + 容差）。
    #[test]
    fn pan_gains_land_on_the_declared_constant_power_curve() {
        let law = PanLaw::ConstantPowerMinus3dB;
        let (left, right) = pan_gains(-1.0, law);
        assert!((left - 1.0).abs() < 1e-7, "全左的左增益必须是 1.0: {left}");
        assert!(right.abs() < 1e-7, "全左的右增益必须 ≈ 0: {right}");

        let (left, right) = pan_gains(1.0, law);
        assert!(left.abs() < 1e-7, "全右的左增益必须 ≈ 0: {left}");
        assert!(
            (right - 1.0).abs() < 1e-7,
            "全右的右增益必须是 1.0: {right}"
        );

        let (left, right) = pan_gains(0.0, law);
        assert!(
            (left - core::f32::consts::FRAC_1_SQRT_2).abs() < 1e-7,
            "居中的左增益必须是 √2/2: {left}"
        );
        assert!((left - right).abs() < 1e-7, "居中必须左右对称");
    }

    /// 等功率性：`gain_l² + gain_r² == 1` 在整条 `pan` 范围上成立（0.1 步长扫描）。
    #[test]
    fn constant_power_holds_across_the_pan_range() {
        let law = PanLaw::ConstantPowerMinus3dB;
        let mut pan = -1.0f32;
        while pan <= 1.0 {
            let (left, right) = pan_gains(pan, law);
            let power = left * left + right * right;
            assert!(
                (power - 1.0).abs() < 1e-6,
                "pan {pan}: 增益平方和应恒为 1, 实际 {power}"
            );
            pan += 0.1;
        }
    }

    /// **判据**：三个**未实现**的衰减律显式回落到默认律的曲线（不是等增益复制）。
    ///
    /// [ADR-0001 D43]：不猜语义，但也不许"悄悄用另一种曲线"。这条判据把回落**钉死**：
    /// 谁改 [`pan_gains`] 的 `match` 让某个未实现分支走了别的形状，它就变红；
    /// 补齐定义之后，把对应分支改成真曲线、并把这条判据的期望一并改掉即可。
    #[test]
    fn unimplemented_pan_laws_fall_back_to_the_documented_curve() {
        for pan in [-1.0f32, -0.5, 0.0, 0.5, 1.0] {
            let expected = pan_gains(pan, PanLaw::ConstantPowerMinus3dB);
            for law in [
                PanLaw::Linear,
                PanLaw::ConstantPowerMinus4_5dB,
                PanLaw::ConstantPowerMinus6dB,
            ] {
                let actual = pan_gains(pan, law);
                assert_eq!(
                    (actual.0.to_bits(), actual.1.to_bits()),
                    (expected.0.to_bits(), expected.1.to_bits()),
                    "pan={pan} law={law:?} 必须显式回落到默认律（不得静默换成别的曲线）"
                );
            }
        }
    }

    /// 退化输入：`NaN`/越界 `pan` 必须被钳制，绝不产生 `NaN`。
    #[test]
    fn degenerate_pan_values_are_clamped() {
        for pan in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let (left, right) = pan_gains(pan, PanLaw::ConstantPowerMinus3dB);
            assert!(
                left.is_finite() && right.is_finite(),
                "pan {pan} 产生了非有限增益"
            );
        }
        let (left, right) = pan_gains(-3.0, PanLaw::ConstantPowerMinus3dB);
        assert!(
            (left - 1.0).abs() < 1e-7 && right.abs() < 1e-7,
            "越界必须钳到全左"
        );
        let (left, right) = pan_gains(3.0, PanLaw::ConstantPowerMinus3dB);
        assert!(
            left.abs() < 1e-7 && (right - 1.0).abs() < 1e-7,
            "越界必须钳到全右"
        );
    }

    // -----------------------------------------------------------------------
    // 上移的机械证据（类型/函数/常量同一性 + 无第二份实现）
    // -----------------------------------------------------------------------

    /// 判据：engine 的 `BusLimiter` 与 dsp 的 `Limiter` **是同一个东西**（不是同构复制品）。
    ///
    /// 这是编译期判据（类型赋值）+ 运行期判据（函数地址相等 + 常量位模式相等）。
    /// 注入：在 engine 里偷偷加回一份自己的限制器类型或 `soft_knee`
    /// ⇒ 类型赋值与 `fn_addr_eq` 双双编译失败/变红。
    #[test]
    fn engine_bus_limiter_is_literally_the_dsp_limiter() {
        // 类型同一性（编译期）：dsp 类型的变量可以直接由 engine 路径构造。
        let via_engine: yeban_dsp::limiter::Limiter = BusLimiter::new();
        let via_dsp: yeban_dsp::limiter::Limiter = yeban_dsp::limiter::Limiter::new();
        assert_eq!(via_engine.gain().to_bits(), via_dsp.gain().to_bits());

        // 方法同一性（地址相等）：这些**必须**是同一个函数项，而不是两份同构实现。
        let engine_process: fn(&mut BusLimiter, &mut [f32], &mut [f32]) -> usize =
            BusLimiter::process_stereo;
        let dsp_process: fn(&mut yeban_dsp::limiter::Limiter, &mut [f32], &mut [f32]) -> usize =
            yeban_dsp::limiter::Limiter::process_stereo;
        assert!(core::ptr::fn_addr_eq(engine_process, dsp_process));

        let engine_new: fn() -> BusLimiter = BusLimiter::new;
        let dsp_new: fn() -> yeban_dsp::limiter::Limiter = yeban_dsp::limiter::Limiter::new;
        assert!(core::ptr::fn_addr_eq(engine_new, dsp_new));

        let engine_reset: fn(&mut BusLimiter) = BusLimiter::reset;
        let dsp_reset: fn(&mut yeban_dsp::limiter::Limiter) = yeban_dsp::limiter::Limiter::reset;
        assert!(core::ptr::fn_addr_eq(engine_reset, dsp_reset));

        // 常量同一性（逐位）：engine 路径读到的五个规范常量就是 dsp 的那五个。
        assert_eq!(LOOKAHEAD_SAMPLES, yeban_dsp::limiter::LOOKAHEAD_SAMPLES);
        assert_eq!(
            LIMITER_LATENCY_FRAMES,
            yeban_dsp::limiter::LIMITER_LATENCY_FRAMES
        );
        assert_eq!(
            LIMITER_THRESHOLD.to_bits(),
            yeban_dsp::limiter::LIMITER_THRESHOLD.to_bits()
        );
        assert_eq!(
            LIMITER_CEILING.to_bits(),
            yeban_dsp::limiter::LIMITER_CEILING.to_bits()
        );
        assert_eq!(
            LIMITER_RELEASE_PER_SAMPLE.to_bits(),
            yeban_dsp::limiter::LIMITER_RELEASE_PER_SAMPLE.to_bits()
        );
    }

    /// 判据：**engine 侧没有第二份限制器实现**（源码级机械检查）。
    ///
    /// `include_str!("mixer.rs")` 读到本文件自身的源码；断言其中不出现实现记号。
    /// 记号用 `concat!` 拼出来，避免判据自己的字面量命中自己。
    ///
    /// 注入：把 `yeban_dsp::limiter` 的实现复制进本文件（例如加回环形缓冲与
    /// `soft_knee`）⇒ 本判据立即变红。
    #[test]
    fn engine_mixer_module_has_no_second_limiter_implementation() {
        let source = include_str!("mixer.rs");
        let forbidden = [
            concat!("struct", " BusLimiter", " {"),
            concat!("impl", " BusLimiter"),
            concat!("struct", " Limiter", " {"),
            concat!("impl", " Limiter"),
            concat!("fn", " soft_knee("),
            concat!("fn", " nan_to_zero("),
            concat!("const", " LOOKAHEAD_SAMPLES"),
            concat!("const", " LIMITER_LATENCY_FRAMES"),
            concat!("const", " LIMITER_THRESHOLD"),
            concat!("const", " LIMITER_CEILING"),
            concat!("const", " LIMITER_RELEASE_PER_SAMPLE"),
        ];
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "engine 的 mixer.rs 里出现了实现记号 `{needle}` —— 上移之后这里只允许 `pub use`"
            );
        }
        assert!(
            source.contains("pub use yeban_dsp::limiter::"),
            "engine 的 mixer.rs 必须是 dsp 限制器的再导出"
        );
    }
}
