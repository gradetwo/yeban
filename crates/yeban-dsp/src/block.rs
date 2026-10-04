//! 定长块的标量混音原语。[ARCH-RT-001]
//!
//! Ported from synth-core (MIT, Copyright (c) 2026 GROOVE SYNTH GS-1 contributors)
//! `src/dsp/simd.rs`（**仅标量路径**）。
//!
//! ## 与来源的差异
//!
//! 来源在 `wasm32` + `simd128` 下有 `unsafe` 的 `core::arch::wasm32` 向量路径与
//! 标量回退。夜半是**纯血 Rust**：`#![forbid(unsafe_code)]` 是硬红线
//! [AGENTS.md 红线 8]，因此这里只保留标量实现（来源里那部分本来就是尾块与
//! 非 wasm 目标的实际执行路径）。向量化留给编译器自动向量化——它不需要 `unsafe`。
//!
//! 所有函数都**按最短长度**工作，不做 `debug_assert` 之外的假设、不分配、不 panic。

/// `output[i] += input[i] * gain`（给总线累加一条轨道）。
///
/// `zip` 天然按 `input.len()` 与 `output.len()` 的较小者处理，因此尾部永远不会越界。
#[inline]
pub fn accumulate(input: &[f32], output: &mut [f32], gain: f32) {
    for (out, inp) in output.iter_mut().zip(input) {
        *out += *inp * gain;
    }
}

/// `output[i] = input[i] * gain`。
#[inline]
pub fn scale_into(input: &[f32], output: &mut [f32], gain: f32) {
    for (out, inp) in output.iter_mut().zip(input) {
        *out = *inp * gain;
    }
}

/// `out[i] = a[i] * ga + b[i] * gb`（双振荡器混音）。
#[inline]
pub fn mix2_into(a: &[f32], b: &[f32], out: &mut [f32], ga: f32, gb: f32) {
    for ((slot, &av), &bv) in out.iter_mut().zip(a).zip(b) {
        *slot = av * ga + bv * gb;
    }
}

/// 块内绝对值峰值（电平表用）。
#[inline]
#[must_use]
pub fn peak(samples: &[f32]) -> f32 {
    let mut peak = 0.0f32;
    for &sample in samples {
        let magnitude = sample.abs();
        if magnitude > peak {
            peak = magnitude;
        }
    }
    peak
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulate_matches_scalar_reference() {
        let input: Vec<f32> = (0..257).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut out = vec![0.25f32; input.len()];
        let mut expect = out.clone();
        accumulate(&input, &mut out, 0.7);
        for i in 0..input.len() {
            expect[i] += input[i] * 0.7;
            assert!((out[i] - expect[i]).abs() < 1e-6, "mismatch at {i}");
        }
    }

    #[test]
    fn mix2_handles_non_multiple_of_four() {
        let a = vec![1.0f32; 13];
        let b = vec![2.0f32; 13];
        let mut out = vec![0.0f32; 13];
        mix2_into(&a, &b, &mut out, 0.5, 0.25);
        for v in out {
            assert!((v - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn peak_finds_maximum() {
        assert_eq!(peak(&[0.1, -0.8, 0.3]), 0.8);
        assert_eq!(peak(&[]), 0.0);
    }

    #[test]
    fn scale_into_is_exact_and_touches_nothing_else() {
        let input = [1.0f32, -2.0, 0.5];
        let mut out = [9.0f32; 5];
        scale_into(&input, &mut out, 2.0);
        assert_eq!(out, [2.0, -4.0, 1.0, 9.0, 9.0]);
    }

    #[test]
    fn mismatched_lengths_are_truncated_instead_of_panicking() {
        let mut out = [1.0f32; 2];
        accumulate(&[1.0f32; 8], &mut out, 1.0);
        assert_eq!(out, [2.0, 2.0]);

        let mut out = [0.0f32; 8];
        scale_into(&[1.0f32; 3], &mut out, 1.0);
        assert_eq!(out[..4], [1.0, 1.0, 1.0, 0.0]);

        let mut out = [7.0f32; 4];
        mix2_into(&[1.0f32; 2], &[1.0f32; 9], &mut out, 1.0, 1.0);
        assert_eq!(out, [2.0, 2.0, 7.0, 7.0]);

        // 零长度输入是合法的（空块）并且不得改动输出。
        let mut out = [3.0f32; 2];
        accumulate(&[], &mut out, 1.0);
        assert_eq!(out, [3.0, 3.0]);
    }
}
