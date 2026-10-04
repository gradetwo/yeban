//! 电平口径的**再导出**：实现已上移到 `yeban-dsp`。[ARCH-UI-002, ROAD-M2-008]
//!
//! ## 这一层还剩什么
//!
//! **只剩 `pub use`。** 峰值、峰值保持（20 dB/s 指数释放）、块 RMS、平滑 RMS
//! （τ = 300 ms 一阶低通）、dBFS 换算、静音下限、`NaN`/`±∞` 钳位、`supersedes`
//! 取最新判据、以及 4× 真峰值检测器，全部住在 `yeban_dsp::meter`。
//!
//! 为什么搬：那是**纯数学**（无队列、无线程、无设备、无模型类型），属于
//! `yeban-dsp` 的职责；上移之后混音台、母带、导出都能复用同一份口径，
//! 而不是各自再实现一遍。engine 只保留"实时侧状态 + 发布"（[`crate::meter`] 的
//! `MeterBank`/`MeterFrame`/SPSC 与 [`crate::rt`] 的每量子一次批量发布）。
//!
//! ## 为什么保留这个模块而不是把路径改掉
//!
//! `crate::level::*` 是引擎内部与既有判据（`src/meter.rs`、`src/rt.rs`、
//! `tests/meter_rt_contract.rs`）的公共面。保留模块名 + 再导出，
//! 让"搬家"对调用方**完全透明**：一行调用都不用改。
//!
//! ## 没有第二份实现（机械可查）
//!
//! - 编译期：[`tests::engine_level_is_literally_the_dsp_type`] 把 engine 路径下的类型
//!   赋给 dsp 路径下的类型，并用 `core::ptr::fn_addr_eq` 比较函数指针地址 ——
//!   若引擎私藏一份自己的 `LevelDetector`/`sanitize_sample`，本文件**编译不过**；
//! - 源码期：[`tests::engine_level_module_has_no_second_implementation`] 用
//!   `include_str!` 读出本文件自身的源码，断言其中不存在实现记号
//!   （`struct`/`impl`/`fn` 定义），只有 `pub use`。
//!
//! 上移的逐位不变证据（285 个 `f32` 位模式）在
//! `yeban_dsp::meter::tests::frozen_pre_hoist_table_is_reproduced_bit_for_bit` 与
//! 本文件的 [`tests::frozen_level_table_through_the_engine_path`] 两处。

// 电平口径的唯一实现住在 yeban-dsp；这里只做转发（含真峰值能力）。
pub use yeban_dsp::meter::{
    DEFAULT_PEAK_DECAY_DB_PER_SEC, DEFAULT_QUANTA_PER_SECOND, DEFAULT_RMS_TIME_CONSTANT_SEC,
    LevelDetector, LevelReading, MAX_LINEAR_MAGNITUDE, SILENCE_FLOOR_DBFS,
    TRUE_PEAK_LATENCY_SAMPLES, TRUE_PEAK_PHASES, TRUE_PEAK_TAPS, TruePeakDetector, dbfs,
    dbfs_clamped, sanitize_sample, supersedes,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// 无分配生成一段正弦（测试用；实时路径不生成信号，只测量）。
    fn sine(amplitude: f32, cycles: f32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|index| {
                let phase = cycles * std::f32::consts::TAU * index as f32 / len as f32;
                amplitude * phase.sin()
            })
            .collect()
    }

    #[test]
    fn full_scale_sine_peaks_at_zero_dbfs() {
        // 一个完整周期的采样数必须足够多, 峰值才能贴近 1.0: 4800 点覆盖 10 周期,
        // 理论峰值落在样本间的误差 < 1 %(≈0.09 dB), 因此容差取 0.05 dB 过严,
        // 取 0.1 dB 并附理由: sin 的离散采样峰值 = cos(π/N) ≈ 1 - 5e-6 ⇒ 实际误差远小于 0.1 dB。
        let samples = sine(1.0, 10.0, 4800);
        let mut detector = LevelDetector::new();
        let reading = detector.analyze(&samples);
        assert!(reading.is_sane());
        assert!((reading.peak - 1.0).abs() < 1e-3, "峰值应达满量程");
        assert!(
            reading.peak_dbfs().abs() < 0.1,
            "满幅正弦峰值应接近 0 dBFS, 实际 {}",
            reading.peak_dbfs()
        );
        // 正弦 RMS = 1/√2 ⇒ -3.01 dBFS(容差给 0.05 dB: 整数周期离散 RMS 误差 < 1e-3 dB)
        assert!(
            (reading.rms - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3,
            "正弦 RMS 应为 1/√2, 实际 {}",
            reading.rms
        );
        assert!(
            (dbfs(reading.rms) + 3.0103).abs() < 0.05,
            "正弦 RMS 应约 -3.01 dBFS, 实际 {}",
            dbfs(reading.rms)
        );
        // 平滑 RMS 在**第一个**量子必然远低于块 RMS(一阶低通从 0 起步), 这正是"平滑"的证据
        assert!(
            reading.rms_smoothed < reading.rms,
            "首块的平滑 RMS 必须低于块 RMS"
        );
    }

    #[test]
    fn half_scale_sine_is_minus_six_dbfs() {
        let samples = sine(0.5, 10.0, 4800);
        let reading = LevelDetector::new().analyze(&samples);
        assert!(
            (reading.peak_dbfs() + 6.0206).abs() < 0.1,
            "半幅峰值应约 -6.02 dBFS, 实际 {}",
            reading.peak_dbfs()
        );
    }

    #[test]
    fn zero_input_is_negative_infinity_or_floor_never_nan() {
        let silence = [0.0f32; 128];
        let mut detector = LevelDetector::new();
        let reading = detector.analyze(&silence);
        assert!(reading.is_sane(), "静音不得产生 NaN/Inf");
        assert_eq!(reading.peak, 0.0);
        assert_eq!(reading.rms, 0.0);
        assert_eq!(reading.peak_dbfs(), f32::NEG_INFINITY);
        assert_eq!(reading.rms_dbfs(), f32::NEG_INFINITY);
        assert_eq!(
            dbfs_clamped(reading.peak, SILENCE_FLOOR_DBFS),
            SILENCE_FLOOR_DBFS,
            "UI 侧应得到静音下限而不是 NaN"
        );
        // 空切片同理(0/0 必须是 0 而不是 NaN)
        let empty = detector.analyze(&[]);
        assert!(empty.is_sane());
        assert_eq!(empty.rms, 0.0);
        let empty_stereo = detector.analyze_stereo(&[], &[]);
        assert!(empty_stereo.is_sane());
        assert_eq!(empty_stereo.rms, 0.0);
    }

    #[test]
    fn levels_are_monotonic_in_amplitude() {
        let amplitudes = [0.001f32, 0.01, 0.1, 0.5, 1.0];
        let mut previous_peak = -1.0f32;
        let mut previous_rms = -1.0f32;
        let mut previous_db = f32::NEG_INFINITY;
        for amplitude in amplitudes {
            let samples = sine(amplitude, 10.0, 4800);
            // 每个幅度用**全新**检测器: 排除峰值保持/平滑的历史影响。
            let reading = LevelDetector::new().analyze(&samples);
            assert!(reading.peak > previous_peak, "峰值必须随幅度单调增");
            assert!(reading.rms > previous_rms, "RMS 必须随幅度单调增");
            assert!(reading.peak_dbfs() > previous_db, "dBFS 必须随幅度单调增");
            previous_peak = reading.peak;
            previous_rms = reading.rms;
            previous_db = reading.peak_dbfs();
        }
    }

    #[test]
    fn nan_and_infinity_inputs_never_produce_nan_levels() {
        let hostile = [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            1.0e30,
            -1.0e30,
            0.5,
            f32::NAN,
        ];
        let mut detector = LevelDetector::new();
        let reading = detector.analyze(&hostile);
        assert!(
            reading.is_sane(),
            "NaN/Inf 输入必须被钳位, 实际 {reading:?}"
        );
        assert!(
            reading.peak <= MAX_LINEAR_MAGNITUDE,
            "钳位后的峰值不得超过上限, 实际 {}",
            reading.peak
        );
        assert!(reading.peak_hold <= MAX_LINEAR_MAGNITUDE);
        assert!(reading.rms <= MAX_LINEAR_MAGNITUDE);
        assert!(reading.peak_dbfs().is_finite(), "有限幅度 ⇒ 有限 dBFS");

        // 全部是 NaN 的块 ⇒ 静音, 且状态不被污染(后续静音仍然 sane)
        let all_nan = [f32::NAN; 64];
        let nan_reading = detector.analyze(&all_nan);
        assert!(nan_reading.is_sane());
        assert_eq!(nan_reading.peak, 0.0);
        let after = detector.analyze(&[0.0f32; 64]);
        assert!(after.is_sane(), "NaN 之后仍必须 sane");
    }

    #[test]
    fn sanitize_clamps_and_never_returns_nan() {
        assert_eq!(sanitize_sample(f32::NAN), 0.0);
        assert_eq!(sanitize_sample(f32::INFINITY), MAX_LINEAR_MAGNITUDE);
        assert_eq!(sanitize_sample(f32::NEG_INFINITY), -MAX_LINEAR_MAGNITUDE);
        assert_eq!(sanitize_sample(1.0e30), MAX_LINEAR_MAGNITUDE);
        assert_eq!(sanitize_sample(-1.0e30), -MAX_LINEAR_MAGNITUDE);
        assert_eq!(sanitize_sample(0.25), 0.25);
        assert!(sanitize_sample(f32::NAN).is_finite());
    }

    #[test]
    fn peak_hold_decays_by_the_documented_rate() {
        // 默认口径: 375 量子/s、20 dB/s ⇒ 375 个量子之后恰好降 20 dB。
        let qps = DEFAULT_QUANTA_PER_SECOND;
        let quanta = qps as usize;
        let mut detector = LevelDetector::new();
        let first = detector.analyze(&[1.0f32; 64]);
        assert!((first.peak_hold - 1.0).abs() < 1e-6);

        let mut last = first;
        for _ in 0..quanta {
            last = detector.analyze(&[0.0f32; 64]);
        }
        // 起始幅度是 1.0(0 dBFS) ⇒ 回落量就是保持值的负 dBFS
        let decay_db = -last.peak_hold_dbfs();
        assert!(
            (decay_db - DEFAULT_PEAK_DECAY_DB_PER_SEC).abs() < 0.05,
            "1 秒应恰好回落 {} dB, 实际 {} dB",
            DEFAULT_PEAK_DECAY_DB_PER_SEC,
            decay_db
        );
    }

    #[test]
    fn smoothed_rms_follows_the_documented_time_constant() {
        // 常数 0.5 输入下, 均方一阶低通在 τ 秒后达到目标的 1-1/e。
        let qps = DEFAULT_QUANTA_PER_SECOND;
        let tau_quanta = (DEFAULT_RMS_TIME_CONSTANT_SEC * qps).round() as usize;
        let mut detector = LevelDetector::new();
        let mut last = LevelReading::silence();
        for _ in 0..tau_quanta {
            last = detector.analyze(&[0.5f32; 64]);
        }
        let expected = 0.5 * (1.0 - (-1.0f32).exp()).sqrt();
        assert!(
            (last.rms_smoothed - expected).abs() < 0.01,
            "τ 之后平滑 RMS 应约 {expected}, 实际 {}",
            last.rms_smoothed
        );

        // 充分长时间后收敛到块 RMS(0.5)
        for _ in 0..(tau_quanta * 20) {
            last = detector.analyze(&[0.5f32; 64]);
        }
        assert!(
            (last.rms_smoothed - 0.5).abs() < 0.005,
            "稳态应收敛到 0.5, 实际 {}",
            last.rms_smoothed
        );
    }

    #[test]
    fn stereo_reading_is_channel_linked() {
        let left = sine(1.0, 10.0, 4800);
        let right = vec![0.0f32; 4800];
        let mut detector = LevelDetector::new();
        let reading = detector.analyze_stereo(&left, &right);
        assert!(
            reading.peak_dbfs().abs() < 0.1,
            "单声道满幅时联动峰值应达 0 dBFS, 实际 {}",
            reading.peak_dbfs()
        );
        // 均方按两声道平均: 只有一个满幅声道 ⇒ RMS = √(1/2)·(1/√2) = 0.5
        assert!(
            (reading.rms - 0.5).abs() < 1e-3,
            "联动 RMS 应为 0.5, 实际 {}",
            reading.rms
        );

        // 长度不等(实时尾块)不 panic, 按较短者工作
        let short = detector.analyze_stereo(&[1.0, 1.0], &[1.0]);
        assert!(short.is_sane());
        assert_eq!(short.peak, 1.0);
    }

    #[test]
    fn empty_block_still_decays_hold_and_keeps_finite_rms() {
        let mut detector = LevelDetector::new();
        let loud = detector.analyze(&[1.0f32; 128]);
        assert!((loud.peak_hold - 1.0).abs() < 1e-6);
        let mut quiet = loud;
        for _ in 0..375 {
            quiet = detector.analyze(&[]);
        }
        assert!(quiet.is_sane());
        assert_eq!(quiet.peak, 0.0, "空块峰值必须是 0");
        assert!(
            quiet.peak_hold < 0.11,
            "空块也必须让保持值回落, 实际 {}",
            quiet.peak_hold
        );
        assert!(quiet.rms.is_finite() && quiet.rms_smoothed.is_finite());
    }

    #[test]
    fn supersedes_accepts_equal_and_newer_but_not_older() {
        assert!(supersedes(7, 7));
        assert!(supersedes(8, 7));
        assert!(!supersedes(6, 7));
        // 单调推进: 新值被接受之后, 旧值再也不能翻回来
        assert!(!supersedes(7, 8));
    }

    #[test]
    fn ballistics_reject_invalid_parameters_without_nan() {
        let mut detector = LevelDetector::with_ballistics(f32::NAN, -5.0, 0.0);
        let reading = detector.analyze(&[1.0f32; 32]);
        assert!(reading.is_sane(), "非法弹道参数必须退回默认而不是 NaN");
        assert!(reading.peak_hold <= 1.0);
        // 不释放(decay=0) ⇒ 保持值不下降
        let mut hold = LevelDetector::with_ballistics(375.0, 0.0, 0.3);
        let loud = hold.analyze(&[1.0f32; 32]);
        let later = hold.analyze(&[]);
        assert!((later.peak_hold - loud.peak_hold).abs() < 1e-6);
    }

    #[test]
    fn silent_detector_has_no_ballistics() {
        let mut detector = LevelDetector::silent();
        let first = detector.analyze(&[0.5f32; 32]);
        assert!((first.peak_hold - 0.5).abs() < 1e-6);
        assert!(
            (first.rms_smoothed - 0.5).abs() < 1e-6,
            "不平滑 ⇒ 等于块 RMS"
        );
        // 不释放: 静音块之后保持值不下降
        let later = detector.analyze(&[0.0f32; 32]);
        assert!((later.peak_hold - 0.5).abs() < 1e-6);
        assert_eq!(later.rms, 0.0);
        assert_eq!(later.rms_smoothed, 0.0, "均方系数 0 ⇒ 跟随当块");
    }

    #[test]
    fn reset_clears_state_but_keeps_ballistics() {
        let mut detector = LevelDetector::new();
        let _ = detector.analyze(&[1.0f32; 32]);
        assert!(detector.peak_hold() > 0.0);
        detector.reset();
        assert_eq!(detector.peak_hold(), 0.0);
        assert_eq!(detector.mean_square(), 0.0);
        let reading = detector.analyze(&[]);
        assert!(reading.is_sane());
        assert_eq!(reading.peak_hold, 0.0);
    }

    // -----------------------------------------------------------------------
    // 上移的机械证据（类型归属 + 无第二份实现 + 冻结数值表）
    // -----------------------------------------------------------------------

    /// 判据：engine 的 `level` 项与 dsp 的项**是同一个东西**（不是同构复制品）。
    ///
    /// 这是编译期判据：类型赋值要求两侧是**同一个**类型；函数指针用地址比较。
    /// 注入：在 engine 里偷偷加回一份自己的检测器类型或采样钳位函数
    /// ⇒ 类型赋值与 `fn_addr_eq` 双双编译失败/变红。
    #[test]
    fn engine_level_is_literally_the_dsp_type() {
        // 类型同一性（编译期）：dsp 类型的变量可以直接由 engine 路径构造。
        let detector: yeban_dsp::meter::LevelDetector = LevelDetector::new();
        let reading: yeban_dsp::meter::LevelReading = detector.clone().analyze(&[0.5f32; 8]);
        let _: yeban_dsp::meter::TruePeakDetector = TruePeakDetector::new();
        assert!(reading.is_sane());

        // 函数同一性（地址相等）：这些**必须**是同一个函数项，而不是两份同构实现。
        let engine_sanitize: fn(f32) -> f32 = sanitize_sample;
        let dsp_sanitize: fn(f32) -> f32 = yeban_dsp::meter::sanitize_sample;
        assert!(core::ptr::fn_addr_eq(engine_sanitize, dsp_sanitize));

        let engine_dbfs: fn(f32) -> f32 = dbfs;
        let dsp_dbfs: fn(f32) -> f32 = yeban_dsp::meter::dbfs;
        assert!(core::ptr::fn_addr_eq(engine_dbfs, dsp_dbfs));

        let engine_clamped: fn(f32, f32) -> f32 = dbfs_clamped;
        let dsp_clamped: fn(f32, f32) -> f32 = yeban_dsp::meter::dbfs_clamped;
        assert!(core::ptr::fn_addr_eq(engine_clamped, dsp_clamped));

        let engine_supersedes: fn(u64, u64) -> bool = supersedes;
        let dsp_supersedes: fn(u64, u64) -> bool = yeban_dsp::meter::supersedes;
        assert!(core::ptr::fn_addr_eq(engine_supersedes, dsp_supersedes));

        // 常量同一性（编译期）。
        const _: () = assert!(SILENCE_FLOOR_DBFS == yeban_dsp::meter::SILENCE_FLOOR_DBFS);
        const _: () = assert!(MAX_LINEAR_MAGNITUDE == yeban_dsp::meter::MAX_LINEAR_MAGNITUDE);
        const _: () =
            assert!(DEFAULT_QUANTA_PER_SECOND == yeban_dsp::meter::DEFAULT_QUANTA_PER_SECOND);
    }

    /// 判据：**engine 侧没有第二份实现**（源码级机械检查）。
    ///
    /// `include_str!("level.rs")` 读到本文件自身的源码；断言其中不出现实现记号。
    /// 记号用 `concat!` 拼出来，避免判据自己的字面量命中自己。
    ///
    /// 注入：把 `yeban_dsp` 的实现复制进本文件（例如加回检测器类型的定义）
    /// ⇒ 本判据立即变红。
    #[test]
    fn engine_level_module_has_no_second_implementation() {
        let source = include_str!("level.rs");
        // 记号带 `(` 或 ` {`：只有"定义形式"才命中 —— 调用点、测试名与文档提及
        // 都不会误命中（测试名里的下划线后缀因此是安全的）。
        let forbidden = [
            concat!("struct", " LevelDetector", " {"),
            concat!("struct", " LevelReading", " {"),
            concat!("struct", " TruePeakDetector", " {"),
            concat!("impl", " LevelDetector"),
            concat!("impl", " LevelReading"),
            concat!("impl", " TruePeakDetector"),
            concat!("fn", " sanitize_sample("),
            concat!("fn", " dbfs("),
            concat!("fn", " dbfs_clamped("),
            concat!("fn", " supersedes("),
            concat!("fn", " mean_of_squares("),
            concat!("const", " TRUE_PEAK_KERNEL"),
        ];
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "engine 的 level.rs 里出现了实现记号 `{needle}` —— 上移之后这里只允许 `pub use`"
            );
        }
        assert!(
            source.contains("pub use yeban_dsp::meter::"),
            "engine 的 level.rs 必须是 dsp 的再导出"
        );
    }

    /// IEEE-754 单调映射：`f32` 位模式 → 可比较大小的整数，差值即 ulp 距离。
    fn ordered_bits(bits: u32) -> i64 {
        if bits & 0x8000_0000 != 0 {
            -i64::from(bits & 0x7fff_ffff)
        } else {
            i64::from(bits)
        }
    }

    /// 经过 `log10`/`exp`/`powf` 的冻结值在非冻结架构上允许的 ulp 预算。
    ///
    /// 与 `yeban_dsp::meter::tests` 的常量同源同值（各自的测例私有，不能共享）。
    /// 依据见那份判据的文档：超越函数不要求正确舍入，跨架构可差 1 ulp 并沿弹道累积；
    /// 4096 ulp ≈ 2.4e-4 相对，远小于任何真实行为漂移（最小的注入效应 ≈ 50000 ulp）。
    const TRANSCENDENTAL_ULP_BUDGET: i64 = 4096;

    /// **判据**：搬迁前的关键数值经 **engine 路径**仍然一致。
    ///
    /// 这是 `yeban-engine/src/level.rs`（main `b014e8f`）上实测冻结的位模式子集；
    /// 完整 285 条的对照在 `yeban_dsp::meter::tests`。
    ///
    /// - 只经过 IEEE 正确舍入运算的量（钳位、比较、`abs`/`max`、乘除、`sqrt`）：
    ///   **逐位**相同，没有容差；
    /// - 经过 `log10`/`exp`/`powf` 的量：给 4096 ulp 的跨架构预算，
    ///   在冻结架构（aarch64）上仍然要求逐位相同。
    #[test]
    fn frozen_level_table_through_the_engine_path() {
        /// 超越函数类条目的比较：ulp 预算 + 冻结架构上逐位。
        fn assert_within_ulp_budget(name: &str, got: f32, frozen: u32) {
            let distance = (ordered_bits(got.to_bits()) - ordered_bits(frozen)).abs();
            assert!(
                distance <= TRANSCENDENTAL_ULP_BUDGET,
                "{name} 超出 ulp 预算: 实测 {:#010x} ({}), 搬迁前 {frozen:#010x} ({}), 相距 {distance} ulp",
                got.to_bits(),
                got,
                f32::from_bits(frozen)
            );
            if cfg!(target_arch = "aarch64") {
                assert_eq!(got.to_bits(), frozen, "{name} 在冻结架构上必须逐位相同");
            }
        }

        // 纯函数：钳位 / dBFS / 取最新（位模式来自搬迁前实测）。
        assert_eq!(sanitize_sample(f32::NAN).to_bits(), 0x0000_0000);
        assert_eq!(sanitize_sample(f32::INFINITY).to_bits(), 0x4180_0000);
        assert_eq!(sanitize_sample(f32::NEG_INFINITY).to_bits(), 0xc180_0000);
        assert_eq!(sanitize_sample(1.0e30).to_bits(), 0x4180_0000);
        assert_eq!(sanitize_sample(-1.0e30).to_bits(), 0xc180_0000);
        assert_eq!(sanitize_sample(0.25).to_bits(), 0x3e80_0000);
        assert_eq!(sanitize_sample(-0.0).to_bits(), 0x8000_0000);
        // `dbfs(1.0) = 0`、`dbfs(≤0) = −∞` 是**精确**边界, 逐位钉住。
        assert_eq!(dbfs(1.0).to_bits(), 0x0000_0000);
        assert_eq!(dbfs(0.0).to_bits(), 0xff80_0000);
        assert_eq!(dbfs(f32::NAN).to_bits(), 0xff80_0000);
        // 一般点由 `log10` 得出 ⇒ 走 ulp 预算。
        assert_within_ulp_budget("dbfs(0.5)", dbfs(0.5), 0xc0c0_a8c2);
        assert_within_ulp_budget("dbfs(16.0)", dbfs(16.0), 0x41c0_a8c2);
        assert_eq!(dbfs_clamped(0.0, -120.0).to_bits(), 0xc2f0_0000);
        assert_eq!(dbfs_clamped(1.0e-9, -120.0).to_bits(), 0xc2f0_0000);
        assert_eq!(dbfs_clamped(1.0, -120.0).to_bits(), 0x0000_0000);
        assert!(supersedes(7, 7) && supersedes(8, 7) && !supersedes(6, 7));
        assert_eq!(DEFAULT_QUANTA_PER_SECOND.to_bits(), 0x43bb_8000);

        // 满幅正弦（10 周期 / 4800 点）经 engine 路径的完整读数。
        let samples: Vec<f32> = (0..4800)
            .map(|index| {
                let phase = 10.0 * std::f32::consts::TAU * index as f32 / 4800.0;
                phase.sin()
            })
            .collect();
        let reading = LevelDetector::new().analyze(&samples);
        assert_eq!(reading.peak.to_bits(), 0x3f80_0000);
        assert_eq!(reading.peak_hold.to_bits(), 0x3f80_0000);
        assert_eq!(reading.rms.to_bits(), 0x3f35_04f3);
        assert_within_ulp_budget("sine1.rms_smoothed", reading.rms_smoothed, 0x3d88_3b02);
        assert_eq!(reading.peak_dbfs().to_bits(), 0x0000_0000);
        assert_eq!(reading.peak_hold_dbfs().to_bits(), 0x0000_0000);
        assert_within_ulp_budget("sine1.rms_dbfs", reading.rms_dbfs(), 0xc1bc_5432);

        // 峰值保持的 1 量子释放乘子与平滑均方的一量子系数（内部弹道系数）。
        let mut detector = LevelDetector::new();
        let _ = detector.analyze(&[1.0f32; 64]);
        assert_within_ulp_budget(
            "release.one_quantum",
            detector.analyze(&[]).peak_hold,
            0x3f7e_6ed4,
        );
        let mut detector = LevelDetector::new();
        let one = detector.analyze(&[1.0f32; 64]);
        assert_within_ulp_budget("smooth.one_quantum_rms", one.rms_smoothed, 0x3dc0_a8b6);
        assert_within_ulp_budget("smooth.one_quantum_ms", detector.mean_square(), 0x3c10_fd80);

        // 静音 / 空块的边界。
        let mut detector = LevelDetector::new();
        let silence = detector.analyze(&[0.0f32; 128]);
        assert!(silence.is_sane());
        assert_eq!(silence.peak.to_bits(), 0x0000_0000);
        assert_eq!(silence.rms.to_bits(), 0x0000_0000);
        assert_eq!(silence.peak_dbfs(), f32::NEG_INFINITY);
        assert_eq!(detector.analyze(&[]).rms.to_bits(), 0x0000_0000);
        assert_eq!(detector.analyze_stereo(&[], &[]).rms.to_bits(), 0x0000_0000);

        // 真峰值上移之后同样可从 engine 路径使用。
        let mut true_peak = TruePeakDetector::new();
        let block: Vec<f32> = (0..1024)
            .map(|i| {
                let phase =
                    std::f32::consts::FRAC_PI_4 + std::f32::consts::FRAC_PI_2 * (i % 4) as f32;
                phase.sin()
            })
            .collect();
        let sample_peak = block.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let measured = true_peak.process(&block);
        assert!(sample_peak < 0.72);
        assert!(measured > 0.999, "真峰值应抓到采样点之间的过冲: {measured}");
    }
}
