//! # 本机零依赖验证脚手架（**不是** cargo 目标）
//!
//! 为什么存在: `yeban-render` 引入 `rayon`（重依赖）, 因此
//! `scripts/gates/run-gates.sh crate yeban-render` 在本机自动跳过编译
//! （用户硬性纪律: M2 上不跑高耗 CPU 任务）。但**纯计算**部分并不需要 rayon。
//!
//! 本文件把三个**零第三方依赖**的模块按路径包含进来, 用
//! `rustc --edition 2024 --test` 单独编译执行:
//!
//! ```text
//! rustc --edition 2024 --test -D warnings -W missing_docs \
//!   crates/yeban-render/verify/pure_modules.rs -o /tmp/yeban-render-pure
//! /tmp/yeban-render-pure
//! ```
//!
//! 覆盖范围（**如实声明**）:
//!
//! - ✅ 覆盖: VLQ 编解码、确定性有序归约核、TPDF 抖动与位深转换, 以及三者串起来的
//!   "同一输入 + 任意产生顺序 ⇒ 逐位相同"端到端判据。
//! - ❌ 不覆盖: 任何依赖 `rayon` / `hound` / `midly` / `yeban-model` 的代码
//!   （`render.rs`、`rf64.rs` 的写入入口、`midi.rs`、`wav.rs`）。那些交给 CI。
//!
//! 本文件不被 `cargo` 自动发现（不是 `src/`、`tests/`、`benches/`、`examples/`）,
//! 因此不会给 CI 增加任何编译目标 —— 这是刻意的。

// 本文件是 `rustc --test` 的 **crate root**, 不是库。被包含模块的公开 API 在这里
// 没有"外部消费者", 因此 `dead_code` 会误报为错误。真实的 dead-code 判定由 CI 的
// `cargo clippy -p yeban-render --all-targets -- -D warnings` 在 lib crate 上执行。
// 下面这条 allow 只影响本脚手架的作用域。
#![allow(dead_code)]

#[path = "../src/dither.rs"]
mod dither;
#[path = "../src/pdc.rs"]
mod pdc;
#[path = "../src/rf64.rs"]
mod rf64;
#[path = "../src/sum.rs"]
mod sum;
#[path = "../src/vlq.rs"]
mod vlq;

use dither::DitherRng;

/// 脚手架自带的确定性随机源（同 `dither.rs` 内的测试脚手架, 算法与
/// `yeban_dsp::noise::Rng` 一致）。**仅用于本脚手架**, 不是生产随机源。
struct ScaffoldRng(u32);

impl dither::DitherRng for ScaffoldRng {
    fn next_unit(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x as f32 / u32::MAX as f32
    }
}

/// 模拟"离线渲染 → 汇聚 → 抖动 → 母带字节流"的**纯计算骨架**:
/// 每条轨道的缓冲由确定性伪随机数据填充, 汇聚走 `sum::fixed_order`（固定顺序）,
/// 最后走 `dither::quantize`（固定种子）产出 24-bit 字节流。
///
/// 它与 `render.rs` 的真实调度器共用同两个核, 因此对它的判据就是对该不变量的判据;
/// 差别只在"谁来并行、谁先完成"——而**那正是本判据要证明无关的东西**。
fn render_master_bytes(track_order: &[usize], seed: u32) -> Vec<u8> {
    // 每条"轨道"= (EntityId 风格的排序键, 缓冲)
    let tracks: Vec<(u32, Vec<f32>)> = (0..8)
        .map(|track| {
            let mut buffer = vec![0.0f32; 64];
            let mut noise = ScaffoldRng(seed ^ (track as u32).wrapping_mul(0x9E37_79B9));
            for slot in &mut buffer {
                *slot = (noise.next_unit() - 0.5) * 1.0e6 * ((track % 3) as f32 + 1.0);
            }
            // 排序键: 刻意不用 0..8 的自然序, 以证明"顺序由键决定"而不是"恰好是输入序"
            ((track as u32).wrapping_mul(37).wrapping_add(11), buffer)
        })
        .collect();

    let produced: Vec<(u32, Vec<f32>)> = track_order.iter().map(|&i| tracks[i].clone()).collect();
    let ordered = sum::fixed_order(produced);

    let mut master = vec![0.0f32; 64];
    let contributions: Vec<(&[f32], f32)> = ordered
        .iter()
        .enumerate()
        .map(|(index, buffer)| (buffer.as_slice(), 1.0 / (index as f32 + 1.0)))
        .collect();
    sum::reduce_ordered(&contributions, &mut master);

    // 母带电平远超 [-1,1]（那是刻意的: 顺带压到钳位路径）——先归一化再抖动,
    // 否则这条判据只测到钳位, 测不到抖动。
    let peak = master.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
    if peak > 0.0 {
        for slot in &mut master {
            *slot /= peak;
        }
    }
    let mut rng = ScaffoldRng(seed);
    dither::quantize(&master, dither::BitDepth::Int24, &mut rng).to_le_bytes()
}

#[cfg(test)]
mod scaffold_tests {
    use super::*;

    /// **核心判据（ARCH-DET-002 的端到端形式, 本机可跑）**:
    /// 同一输入 + 任意"轨道完成顺序" ⇒ 母带字节流逐位相同。
    #[test]
    fn master_bytes_are_bit_identical_across_completion_orders() {
        let reference = render_master_bytes(&[0, 1, 2, 3, 4, 5, 6, 7], 0x00C0_FFEE);
        for order in [
            [7usize, 6, 5, 4, 3, 2, 1, 0],
            [3, 0, 6, 1, 7, 2, 5, 4],
            [5, 4, 3, 2, 1, 0, 7, 6],
            [1, 2, 3, 4, 5, 6, 7, 0],
        ] {
            let bytes = render_master_bytes(&order, 0x00C0_FFEE);
            assert_eq!(
                bytes, reference,
                "完成顺序 {order:?} 改变了母带字节流 —— 汇聚没有走固定顺序"
            );
        }
    }

    /// **变异敏感度自证**: 把汇聚改成"按完成顺序"（不排序）, 同一条判据必须变红。
    /// 这条测试证明上面那条不是真空判据 —— 它同时是本机版的"注入验证"。
    #[test]
    fn completion_order_reduction_is_detectably_different() {
        fn render_unsorted(track_order: &[usize]) -> Vec<u8> {
            let tracks: Vec<Vec<f32>> = (0..8)
                .map(|track| {
                    let mut buffer = vec![0.0f32; 64];
                    let mut noise =
                        ScaffoldRng(0x00C0_FFEE ^ (track as u32).wrapping_mul(0x9E37_79B9));
                    for slot in &mut buffer {
                        *slot = (noise.next_unit() - 0.5) * 1.0e6 * ((track % 3) as f32 + 1.0);
                    }
                    buffer
                })
                .collect();
            let mut master = vec![0.0f32; 64];
            for (index, &track) in track_order.iter().enumerate() {
                // 故意: 按传入顺序累加, 不排序
                sum::accumulate_into(&tracks[track], &mut master, 1.0 / (index as f32 + 1.0));
            }
            let mut bytes = Vec::with_capacity(master.len() * 4);
            for sample in master {
                bytes.extend_from_slice(&sample.to_bits().to_le_bytes());
            }
            bytes
        }
        let forward = render_unsorted(&[0, 1, 2, 3, 4, 5, 6, 7]);
        let backward = render_unsorted(&[7, 6, 5, 4, 3, 2, 1, 0]);
        assert_ne!(
            forward, backward,
            "测试数据无法暴露浮点非结合律 —— 换一组更极端的幅值"
        );
    }

    /// 判据: 固定格点上的"确定性"是**跨进程可复现**的, 不是"同一进程内两次调用相同"。
    /// 这里比较两次独立调用（各自新建 RNG 状态）的全字节流。
    #[test]
    fn master_bytes_are_reproducible() {
        let first = render_master_bytes(&[0, 1, 2, 3, 4, 5, 6, 7], 0x1234_5678);
        let second = render_master_bytes(&[0, 1, 2, 3, 4, 5, 6, 7], 0x1234_5678);
        assert_eq!(first, second);
        assert_eq!(first.len(), 64 * 3, "24-bit 母带每条样本 3 字节");
    }

    /// 判据: VLQ 的上界与 `midi.rs` 引用的常量一致, 且 4 字节边界恰为 `FF FF FF 7F`。
    /// `midi.rs` 的 CI 判据会把 `midly` 写出的真实字节与这里对账。
    #[test]
    fn vlq_boundary_matches_the_constant_used_by_the_midi_export() {
        assert_eq!(vlq::VLQ_MAX, 0x0FFF_FFFF);
        assert_eq!(vlq::encode(vlq::VLQ_MAX), vec![0xFF, 0xFF, 0xFF, 0x7F]);
    }

    /// 判据: 超出 28 位的值必须在编码前被**拒绝**, 而不是被静默截断成一个
    /// "看起来合法"的 tick。上界是规范硬约束, 因此这里是 panic 而不是 `Result`;
    /// `midi.rs` 负责在进入本函数前把越界变成 `MidiError::DeltaOverflow`。
    #[test]
    #[should_panic(expected = "VLQ 只能表示 28 位")]
    fn vlq_rejects_values_beyond_28_bits() {
        let _ = vlq::encode(vlq::VLQ_MAX + 1);
    }

    /// 判据: 公开面的常量与派生量互相自洽（`bits`/`bytes_per_sample`/`scale`/满量程）。
    /// 这些数值是格式契约的一部分, 写错一位就会导出错误位深的文件。
    #[test]
    fn public_surface_constants_are_self_consistent() {
        assert_eq!(dither::TPDF_PEAK_LSB, 1.0);
        assert_eq!(dither::BitDepth::Int24.scale(), 8_388_608.0);
        assert_eq!(dither::BitDepth::Int16.scale(), 32_768.0);
        assert_eq!(dither::BitDepth::Int24.full_scale_min(), -8_388_608);
        assert_eq!(dither::BitDepth::Int24.full_scale_max(), 8_388_607);
        assert_eq!(
            dither::BitDepth::Int24.full_scale_max() + 1,
            -dither::BitDepth::Int24.full_scale_min()
        );

        let mut rng = ScaffoldRng(7);
        let buffer = dither::quantize(&[0.0, 0.5, -0.5], dither::BitDepth::Int16, &mut rng);
        assert_eq!(buffer.len(), 3);
        assert!(!buffer.is_empty());
        assert_eq!(buffer.depth(), dither::BitDepth::Int16);
        assert_eq!(buffer.byte_len(), 6);
        assert_eq!(buffer.to_le_bytes().len(), 6);

        let mut encoded = vec![0xAAu8];
        vlq::write_into(0x80, &mut encoded);
        assert_eq!(encoded, vec![0xAA, 0x81, 0x00]);
    }

    /// 判据: `sum::fingerprint` 对位型敏感（`+0.0` vs `-0.0`）, 因此它真的能当
    /// "逐位相同"的判据用, 而不是"数值近似相同"的判据。
    #[test]
    fn fingerprint_is_bit_exact_not_value_approximate() {
        assert_ne!(sum::fingerprint(&[0.0]), sum::fingerprint(&[-0.0]));
        let mut rng = ScaffoldRng(3);
        let mut a = vec![0.0f32; 32];
        for slot in &mut a {
            *slot = rng.next_unit();
        }
        let mut b = a.clone();
        // 只翻转最低有效位
        b[7] = f32::from_bits(a[7].to_bits() ^ 1);
        assert_ne!(a, b);
        assert_ne!(sum::fingerprint(&a), sum::fingerprint(&b));
    }
}
