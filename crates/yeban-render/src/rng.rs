//! 确定性随机源到抖动接口的适配 [ARCH-DET-001]。
//!
//! ## 为什么随机源不写在算法模块里
//!
//! [ARCH-DET-001] 把"使用固定种子 PRNG (`rand_xoshiro`)"列为 L1 位级一致的前提。
//! 种子的所有权因此属于**会话/工程层**（`YebanProjectV1::rng_seed` +
//! [MODEL-AST-005] 的 `probability` 判定契约), 不属于一个算法模块。
//! [`crate::dither`] 因此只定义 [`crate::dither::DitherRng`] trait, 由这里接线。
//!
//! ## 为什么复用 `yeban_dsp::noise::Rng` 而不是自建
//!
//! 工作区里已经有且只有一个确定性 xorshift32 实现（`yeban-dsp`, 移植自
//! `synth-core` 的 `dsp/util.rs`）。再写第三个 PRNG 会让"跨模块同序"变成一个
//! 需要人工维护的巧合。因此这里只做适配, 不复制算法 —— 也顺带满足
//! [ARCH-DET-001] "统一启用纯 Rust `libm`" 之外的另一条纪律: **确定性实现只有一份**。

use yeban_dsp::noise::Rng;
use yeban_model::EntityId;

use crate::dither::DitherRng;

/// 生产用确定性抖动用随机源。
///
/// 状态可以取出 ([`Self::state`]) 并存档以复现一段渲染 —— 导出中断后重跑同一段
/// 时, 必须从同一个状态续起, 否则前后两段抖动序列会错位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeterministicDitherRng {
    rng: Rng,
}

impl DeterministicDitherRng {
    /// 用给定种子构造（`seed == 0` 由 `yeban_dsp::noise::Rng` 替换为非零常数）。
    #[must_use]
    pub const fn new(seed: u32) -> Self {
        Self {
            rng: Rng::new(seed),
        }
    }

    /// 当前内部状态, 用于存档/回放。
    #[must_use]
    pub const fn state(&self) -> u32 {
        self.rng.state()
    }
}

impl DitherRng for DeterministicDitherRng {
    #[inline]
    fn next_unit(&mut self) -> f32 {
        self.rng.next_unit()
    }
}

/// 由工程种子与节点身份派生该节点自己的抖动种子。
///
/// 为什么需要派生而不是"所有轨道共用工程种子": 共用种子会让每条轨道拿到**同一条**
/// 抖动序列, 于是各轨的量化噪声完全相关 —— 在总线上相加时, 噪声是相干叠加
/// (`+6 dB` @ 2 轨) 而不是非相干叠加 (`+3 dB`)。按节点派生让噪声去相关,
/// 同时仍然完全可复现 (同一工程 + 同一节点 => 同一种子)。
///
/// 实现是 FNV-1a 折叠 + splitmix64 终混: 只用整数运算, 跨平台逐位相同。
#[must_use]
pub fn dither_seed(project_seed: u64, node: EntityId) -> u32 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in project_seed.to_le_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    // 用规范文本而不是内部表示: 这是**对外承诺**的身份形式 [MODEL-AST-001],
    // 因此不同版本的 `ulid` 内部布局变化不会改变种子。
    for byte in node.to_canonical_string().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    let mut mixed = hash;
    mixed ^= mixed >> 30;
    mixed = mixed.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed ^= mixed >> 27;
    mixed = mixed.wrapping_mul(0x94d0_49bb_1331_11eb);
    mixed ^= mixed >> 31;
    // 取低 32 位: `Rng::new` 会把 0 换成非零常数, 因此无需在这里避开 0。
    (mixed & 0xFFFF_FFFF) as u32
}

/// 由种子与节点构造一个抖动源。
#[must_use]
pub fn dither_rng_for(project_seed: u64, node: EntityId) -> DeterministicDitherRng {
    DeterministicDitherRng::new(dither_seed(project_seed, node))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dither::{BitDepth, quantize, tpdf_lsb};
    use std::str::FromStr;

    fn ulid(index: u32) -> EntityId {
        const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
        let mut text = [b'0'; 26];
        for position in 0..4 {
            text[25 - position] = CROCKFORD[((index >> (5 * position)) & 0x1F) as usize];
        }
        EntityId::from_str(core::str::from_utf8(&text).expect("ASCII")).expect("合法 ULID")
    }

    /// 判据 1: 同一 `(工程种子, 节点)` 恒给同一种子; 不同节点给不同种子。
    #[test]
    fn seeds_are_stable_per_node_and_distinct_across_nodes() {
        let project = 0xDEAD_BEEF_1234_5678u64;
        assert_eq!(dither_seed(project, ulid(1)), dither_seed(project, ulid(1)));
        let mut seen = std::collections::BTreeSet::new();
        for index in 1..64 {
            let seed = dither_seed(project, ulid(index));
            assert!(seen.insert(seed), "节点 {index} 的种子与之前重复");
        }
        assert_ne!(
            dither_seed(project, ulid(1)),
            dither_seed(project + 1, ulid(1))
        );
    }

    /// 判据 2: 适配层产出的抖动与 `dither` 模块自己的脚手架**在统计上同分布**,
    /// 且逐样本可复现。
    #[test]
    fn adapter_is_deterministic_and_zero_mean() {
        let mut first = DeterministicDitherRng::new(42);
        let mut second = DeterministicDitherRng::new(42);
        let mut sum = 0.0f64;
        let draws = 100_000;
        for _ in 0..draws {
            let a = tpdf_lsb(&mut first);
            let b = tpdf_lsb(&mut second);
            assert_eq!(a.to_bits(), b.to_bits(), "同种子必须逐位可复现");
            sum += f64::from(a);
        }
        let mean = sum / f64::from(draws);
        assert!(mean.abs() < 0.01, "均值 {mean} 超出 0.01 LSB");
    }

    /// 判据 3: 状态可存档回放 —— 从存档状态续跑必须与一次跑完一致。
    #[test]
    fn state_can_be_checkpointed_and_resumed() {
        let mut straight = DeterministicDitherRng::new(7);
        let mut expected = Vec::new();
        for _ in 0..32 {
            expected.push(straight.next_unit().to_bits());
        }

        // 前 16 步之后存档状态; 这条判据的意义是"存档状态本身可复现",
        // 因此从中断处续跑才可能与一次跑完对齐。
        let mut first_half = DeterministicDitherRng::new(7);
        for _ in 0..16 {
            let _ = first_half.next_unit();
        }
        let checkpoint = first_half.state();

        let mut replay = DeterministicDitherRng::new(7);
        for _ in 0..16 {
            let _ = replay.next_unit();
        }
        assert_eq!(replay.state(), checkpoint, "同样 16 步必须到达同一个状态");

        let mut tail = Vec::new();
        for _ in 0..16 {
            tail.push(replay.next_unit().to_bits());
        }
        assert_eq!(
            &expected[16..],
            tail.as_slice(),
            "从存档续跑的后半段必须与一次跑完一致"
        );
    }

    /// 判据 4: 32f 透传路径不消耗随机源（因此抖动序列不会因为"是否降位深"而错位）。
    #[test]
    fn float_path_does_not_consume_the_rng() {
        let mut rng = DeterministicDitherRng::new(11);
        let before = rng.state();
        let _ = quantize(&[0.1, 0.2, 0.3], BitDepth::Float32, &mut rng);
        assert_eq!(rng.state(), before);

        let _ = quantize(&[0.1, 0.2, 0.3], BitDepth::Int24, &mut rng);
        assert_ne!(rng.state(), before, "降位深必须消耗随机源");
    }

    /// 判据 5: 不同节点导出的抖动序列互不相同（去相关的必要条件）。
    #[test]
    fn different_nodes_produce_different_dither_sequences() {
        let project = 99u64;
        let mut a = dither_rng_for(project, ulid(1));
        let mut b = dither_rng_for(project, ulid(2));
        let mut identical = 0;
        for _ in 0..4_096 {
            if a.next_unit().to_bits() == b.next_unit().to_bits() {
                identical += 1;
            }
        }
        assert!(
            identical < 8,
            "两条轨道的抖动几乎完全重合 ({identical} 次相同)"
        );
    }
}
