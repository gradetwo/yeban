//! D4 接线判据：`BitCrusher::process_in_place` 与 `process` 逐位同解；
//! 两参数结构的 `sanitised()` 与器件入口钳制逐字同式。
//!
//! 与 `tests/transient_in_place.rs` 同款：插入链运行态在左右缓冲上原地
//! 处理，两条路径同解是运行态敢用 `process_in_place` 的前提。

use yeban_dsp::shaping::{BitCrusher, CrushParams, TransientParams};

/// 一段非平凡的立体声信号（正弦混合，左右不同相），足够走过量化、
/// 采样保持与抗混叠两级单极点的全部状态。
fn signal(frames: usize) -> (Vec<f32>, Vec<f32>) {
    let left: Vec<f32> = (0..frames)
        .map(|i| (i as f32 * 0.11).sin() * 0.7 + (i as f32 * 0.013).cos() * 0.2)
        .collect();
    let right: Vec<f32> = (0..frames)
        .map(|i| (i as f32 * 0.07 + 1.3).sin() * 0.6 - (i as f32 * 0.021).cos() * 0.25)
        .collect();
    (left, right)
}

#[test]
fn bitcrusher_process_in_place_matches_process() {
    let sr = 48_000.0;
    for params in [
        CrushParams {
            bits: 10.0,
            down: 3.0,
            aa: 0.4,
        },
        CrushParams {
            bits: 4.0,
            down: 64.0,
            aa: 0.0,
        },
        CrushParams {
            bits: 16.0,
            down: 1.0,
            aa: 1.0,
        },
    ] {
        let (in_l, in_r) = signal(512);
        let mut out_l = vec![0.0f32; 512];
        let mut out_r = vec![0.0f32; 512];
        BitCrusher::new().process(&in_l, &in_r, &mut out_l, &mut out_r, params, sr);

        let mut in_place_l = in_l.clone();
        let mut in_place_r = in_r.clone();
        BitCrusher::new().process_in_place(&mut in_place_l, &mut in_place_r, params, sr);

        assert_eq!(in_place_l, out_l, "左声道必须逐位同解: {params:?}");
        assert_eq!(in_place_r, out_r, "右声道必须逐位同解: {params:?}");
        // 对照必须非平凡：这组参数下输出与输入确有差异（否则比的是恒等）。
        assert_ne!(out_l, in_l, "判据信号必须真的被碾碎: {params:?}");
    }
}

#[test]
fn crush_params_sanitised_matches_the_device_entry_clamp() {
    assert_eq!(
        CrushParams {
            bits: 99.0,
            down: 0.0,
            aa: 2.0
        }
        .sanitised(),
        CrushParams {
            bits: 16.0,
            down: 1.0,
            aa: 1.0
        }
    );
    assert_eq!(
        CrushParams {
            bits: 2.0,
            down: 100.0,
            aa: -1.0
        }
        .sanitised(),
        CrushParams {
            bits: 4.0,
            down: 64.0,
            aa: 0.0
        }
    );
    assert_eq!(
        CrushParams {
            bits: f32::NAN,
            down: f32::INFINITY,
            aa: f32::NAN
        }
        .sanitised(),
        CrushParams::DEFAULT,
        "非有限逐字回落缺省（bits→16、down→1、aa→0）"
    );
    assert_eq!(CrushParams::default(), CrushParams::DEFAULT);
    assert_eq!(CrushParams::new(), CrushParams::DEFAULT);
}

#[test]
fn transient_params_sanitised_matches_the_device_entry_clamp() {
    assert_eq!(
        TransientParams {
            attack_amt: 2.5,
            sustain_amt: -7.0
        }
        .sanitised(),
        TransientParams {
            attack_amt: 1.0,
            sustain_amt: -1.0
        }
    );
    assert_eq!(
        TransientParams {
            attack_amt: f32::NAN,
            sustain_amt: f32::INFINITY
        }
        .sanitised(),
        TransientParams::DEFAULT,
        "非有限逐字回落 0"
    );
    assert!(TransientParams::DEFAULT.is_identity());
    assert!(
        !TransientParams {
            attack_amt: 5.0,
            sustain_amt: 0.0
        }
        .is_identity(),
        "钳制后非零 ⇒ 非恒等"
    );
    assert_eq!(TransientParams::default(), TransientParams::DEFAULT);
    assert_eq!(TransientParams::new(), TransientParams::DEFAULT);
}
