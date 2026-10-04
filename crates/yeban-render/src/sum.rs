//! 确定性有序归约核 [ARCH-DET-002]。
//!
//! ## 为什么需要它
//!
//! 浮点加法**不满足结合律**: `(a + b) + c != a + (b + c)`。并行渲染里, 各轨道缓冲
//! 由不同线程算出, 如果"谁先算完谁先加", 那么同一份输入在不同运行/不同线程数下
//! 会得到最低有效位 (LSB) 不同的输出 —— L1 bit-exact 契约立刻失效。
//!
//! 架构规范用一句话钉死了这件事:
//!
//! > **[ARCH-DET-002]** 在汇聚到父总线及 Master 母带总线时, 所有并行音频缓冲区的
//! > 加法求和严格按照音轨排序键 (基于 `EntityId` 的确定性字典序) 执行**单线程固定
//! > 顺序串行累加**。
//!
//! 本模块就是那句话的可执行形式, 并且**刻意不依赖 rayon**:
//!
//! - [`fixed_order`] 把"带排序键的贡献集合"折成固定顺序的序列 —— 顺序只由键决定,
//!   与产生顺序、完成顺序、线程数无关;
//! - [`reduce_ordered`] 按**传入的**顺序串行累加 —— 它从不重排, 因此"顺序即结果";
//! - [`accumulate_into`] 是单条贡献的累加原语, 与 [`yeban_dsp::block::accumulate`]
//!   语义相同 (渲染路径只走本函数, 并由 `sum.rs` 的一致性测试钉住两者等价)。
//!
//! 归约核是**纯计算**, 因此可以用 `rustc --edition 2024 --test src/sum.rs` 单独执行,
//! 包括"任意产生顺序 -> 逐位相同"与"故意按完成顺序归约 -> 必红"两条判据
//! (见 `docs/ledger/render-master-notes.md` §5)。

/// `output[i] += input[i] * gain` —— 与 `yeban_dsp::block::accumulate` 同语义。
///
/// 按最短长度工作, 因此尾部永远不会越界; 不分配、不 panic。
#[inline]
pub fn accumulate_into(input: &[f32], output: &mut [f32], gain: f32) {
    for (out, inp) in output.iter_mut().zip(input) {
        *out += *inp * gain;
    }
}

/// 按排序键升序把 `(key, value)` 折成固定顺序的 `Vec<value>`。
///
/// 这是 [ARCH-DET-002] 的"确定性字典序"落点: 输出顺序只由键的全序决定,
/// 与输入顺序、产生顺序、线程完成顺序无关。
///
/// ## 键必须唯一（硬要求, 不是建议）
///
/// 排序是**稳定**的: 键相等时保留输入的相对顺序。也就是说, 重复键会让结果重新
/// 依赖输入顺序 —— 那正是 [ARCH-DET-002] 要消灭的不确定性。因此本函数在排序后
/// **断言键严格递增**。
///
/// 这里刻意用 `assert!` 而不是 `debug_assert!`: 确定性契约不允许在 release 构建里
/// 静默退化。"重复键"不是性能问题 (每个总线只在构建渲染计划时排一次序),
/// 而是"输出会随线程数变化"的缺陷。
///
/// 调用方用复合键保证唯一, 例如 `(source_node, edge_id)` —— 边身份本身唯一,
/// 因此这个键一定唯一。
///
/// # Panics
///
/// `items` 中出现相等的键时 panic。
#[must_use]
pub fn fixed_order<K, T>(items: impl IntoIterator<Item = (K, T)>) -> Vec<T>
where
    K: Ord,
{
    let mut keyed: Vec<(K, T)> = items.into_iter().collect();
    keyed.sort_by(|left, right| left.0.cmp(&right.0));
    for pair in keyed.windows(2) {
        assert!(
            pair[0].0 < pair[1].0,
            "fixed_order 的键必须唯一: 重复键会让结果重新依赖输入顺序 [ARCH-DET-002]"
        );
    }
    keyed.into_iter().map(|(_, value)| value).collect()
}

/// 按传入顺序**串行**累加全部贡献: `out[i] += Σ_k input_k[i] * gain_k`。
///
/// 顺序完全由 `contributions` 给出 —— 本函数从不重排, 也不并行。
/// 调用方必须先把贡献排成 [ARCH-DET-002] 的固定顺序 (见 [`fixed_order`]).
///
/// 长度不一致的贡献只按最短长度参与 (与 [`accumulate_into`] 一致),
/// 因此末尾的静音块不会污染结果。
pub fn reduce_ordered(contributions: &[(&[f32], f32)], out: &mut [f32]) {
    for (input, gain) in contributions {
        accumulate_into(input, out, *gain);
    }
}

/// 把 `f32` 样本按小端 IEEE-754 位型折叠成 64 位 FNV-1a 指纹。
///
/// **非密码学**用途, 只在测试里回答"两个缓冲是否逐位相同"。L1 契约要求的
/// SHA-256 位级哈希由 `render::RenderDigest` 提供 (那里才是对外承诺)。
///
/// `+0.0` 与 `-0.0` 的位型不同 -> 指纹不同。这是**刻意的**: bit-exact 就是 bit-exact。
#[must_use]
pub fn fingerprint(samples: &[f32]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01B3;
    let mut hash = OFFSET_BASIS;
    for &sample in samples {
        for byte in sample.to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 键必须唯一: 重复键会让"固定顺序"重新依赖输入顺序, 因此必须响亮地失败。
    #[test]
    #[should_panic(expected = "fixed_order 的键必须唯一")]
    fn fixed_order_rejects_duplicate_keys() {
        let _ = fixed_order(vec![(1u32, "a"), (1, "b")]);
    }

    /// 复合键 (源节点, 边身份) 是全序唯一的 —— 这正是 `render.rs` 用的形状。
    #[test]
    fn composite_keys_are_unique() {
        let ordered = fixed_order(vec![
            ((2u32, 1u32), "b/1"),
            ((1, 9), "a/9"),
            ((1, 2), "a/2"),
            ((3, 0), "c/0"),
        ]);
        assert_eq!(ordered, vec!["a/2", "a/9", "b/1", "c/0"]);
    }

    #[test]
    fn fixed_order_is_invariant_under_input_permutation() {
        let items = || {
            vec![
                ((30u32, 0u32), 0.5f32),
                ((10, 0), 0.25),
                ((20, 0), -1.0),
                ((10, 1), 2.0),
            ]
        };
        let reference = fixed_order(items());
        assert_eq!(reference, vec![0.25, 2.0, -1.0, 0.5]);
        for permutation in [[0usize, 3, 1, 2], [3, 2, 1, 0], [1, 0, 3, 2], [2, 3, 0, 1]] {
            let source = items();
            let permuted: Vec<((u32, u32), f32)> = permutation.iter().map(|&i| source[i]).collect();
            assert_eq!(fixed_order(permuted), reference, "排列 {permutation:?}");
        }
    }

    /// [ARCH-DET-002] 的机制判据: 归约结果只取决于**固定顺序**, 与产生顺序无关。
    #[test]
    fn reduction_is_bit_identical_regardless_of_production_order() {
        // 刻意挑选非结合律敏感的幅值: 大数吃掉小数, 顺序一变 LSB 就变。
        let contributions: Vec<(u32, Vec<f32>)> = vec![
            (10, vec![1.0e7, 1.0]),
            (20, vec![1.0, -1.0e7]),
            (30, vec![3.0, 5.0]),
            (40, vec![-0.5, 0.25]),
        ];

        let ordered: Vec<Vec<f32>> = fixed_order(
            contributions
                .iter()
                .map(|(key, buffer)| (*key, buffer.clone())),
        );
        let mut reference = vec![0.0f32; 2];
        {
            let refs: Vec<(&[f32], f32)> = ordered.iter().map(|b| (b.as_slice(), 1.0)).collect();
            reduce_ordered(&refs, &mut reference);
        }

        // 任意"产生/完成顺序" => 同一个固定顺序 => 同一个指纹。
        for completion_order in [
            vec![2usize, 0, 3, 1],
            vec![3, 2, 1, 0],
            vec![1, 3, 0, 2],
            vec![0, 1, 2, 3],
        ] {
            let produced: Vec<(u32, Vec<f32>)> = completion_order
                .iter()
                .map(|&index| contributions[index].clone())
                .collect();
            let ordered: Vec<Vec<f32>> = fixed_order(produced);
            let mut result = vec![0.0f32; 2];
            let refs: Vec<(&[f32], f32)> = ordered.iter().map(|b| (b.as_slice(), 1.0)).collect();
            reduce_ordered(&refs, &mut result);
            assert_eq!(
                fingerprint(&result),
                fingerprint(&reference),
                "完成顺序 {completion_order:?} 改变了输出位型"
            );
        }
    }

    /// 变异敏感度自证: **故意**按完成顺序 (不排序) 归约, 结果必须与固定顺序不同。
    /// 没有这条, 上一条判据可能只是"恰好所有顺序都一样"的空判据。
    ///
    /// 数值是**实测**出来的, 不是编的: `1e8` 附近的 f32 ULP 是 8, 因此三个小数
    /// 在正序里被完全吞掉、在反序里先自相加再与大数相加, 结果差 1 ULP。
    /// 实测 (rustc 1.99.0, aarch64-apple-darwin):
    /// 正序 `0x4CD1CEF0` = 110000000.0, 反序 `0x4CD1CEF1` = 110000010.0。
    #[test]
    fn unsorted_reduction_is_detectably_different() {
        let magnitudes = [1.0e8f32, 0.5, 3.0, 3.0, 1.0e7];
        let contributions: Vec<Vec<f32>> = magnitudes.iter().map(|&m| vec![m]).collect();

        let refs: Vec<(&[f32], f32)> = contributions.iter().map(|b| (b.as_slice(), 1.0)).collect();
        let mut in_fixed_order = vec![0.0f32; 1];
        reduce_ordered(&refs, &mut in_fixed_order);

        let reversed: Vec<&[f32]> = contributions.iter().rev().map(|b| b.as_slice()).collect();
        let refs: Vec<(&[f32], f32)> = reversed.into_iter().map(|b| (b, 1.0)).collect();
        let mut by_completion = vec![0.0f32; 1];
        reduce_ordered(&refs, &mut by_completion);

        assert_eq!(in_fixed_order[0].to_bits(), 0x4CD1_CEF0);
        assert_eq!(by_completion[0].to_bits(), 0x4CD1_CEF1);
        assert_ne!(
            in_fixed_order, by_completion,
            "测试数据无法暴露浮点非结合律 —— 换一组更极端的幅值"
        );
    }

    #[test]
    fn accumulate_respects_gain_and_shortest_length() {
        let mut out = [0.0f32; 3];
        accumulate_into(&[1.0, 2.0], &mut out, 0.5);
        assert_eq!(out, [0.5, 1.0, 0.0]);
        accumulate_into(&[1.0, 1.0, 1.0, 1.0], &mut out, 1.0);
        assert_eq!(out, [1.5, 2.0, 1.0]);
    }

    #[test]
    fn fingerprint_distinguishes_signed_zero() {
        assert_ne!(fingerprint(&[0.0]), fingerprint(&[-0.0]));
        assert_eq!(fingerprint(&[1.0]), fingerprint(&[1.0]));
    }
}
