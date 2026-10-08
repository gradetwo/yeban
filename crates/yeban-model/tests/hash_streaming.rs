//! [`AssetHasher`] 的流式契约：**任意分块**都必须与 [`AssetHash::of_bytes`] 逐位一致。
//!
//! 需求来源：`line/decode-limits` 登记的 needs
//! （`docs/ledger/decode-limits-notes.md` §8「增量 / 流式 SHA-256」）：
//!
//! > `AssetHash::of_bytes` 要求完整字节，因此 `import_path` 必须把容器整份读进内存……
//! > 建议在 `yeban-model` 侧提供 `AssetHasher`（`update`/`finalize`），本 crate 就能把两条路径都压到 O(块)。
//!
//! 消费者 `crates/yeban-decode` **不依赖 `sha2`** ⇒ 状态机必须由模型层提供。
//! 因此本文件钉住的是"分块不改变摘要"这一条，而不是某个固定向量
//! （固定向量另有单元判据 `ids::tests::default_asset_hasher_is_the_empty_digest`）。

use proptest::prelude::*;
use yeban_model::{AssetHash, AssetHasher};

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        max_shrink_iters: 64,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// 随机字节串 + 随机切点：逐块喂入的结果必须等于一次性 `of_bytes`。
    #[test]
    fn streaming_hash_equals_of_bytes_for_any_chunking(
        bytes in prop::collection::vec(any::<u8>(), 0..512),
        cuts in prop::collection::vec(0usize..512, 0..16),
    ) {
        let expected = AssetHash::of_bytes(&bytes);
        // 切点取模落在 [0, len] 内，再补上两端 ⇒ 覆盖"空串"与"一次喂完"两种退化情形。
        let bound = bytes.len() + 1;
        let mut points: Vec<usize> = cuts.into_iter().map(|c| c % bound).collect();
        points.push(0);
        points.push(bytes.len());
        points.sort_unstable();
        points.dedup();
        let mut hasher = AssetHasher::new();
        for pair in points.windows(2) {
            hasher.update(&bytes[pair[0]..pair[1]]);
        }
        prop_assert_eq!(hasher.finalize(), expected);
    }

    /// 分块与逐字节喂入必须给出同一个摘要（写者形状不同 ⇒ 结果不许不同）。
    #[test]
    fn streaming_hash_is_stable_across_chunk_shapes(
        bytes in prop::collection::vec(any::<u8>(), 0..512),
    ) {
        let mut whole = AssetHasher::new();
        whole.update(&bytes);
        let mut per_byte = AssetHasher::new();
        for byte in &bytes {
            per_byte.update(std::slice::from_ref(byte));
        }
        prop_assert_eq!(whole.finalize(), per_byte.finalize());
    }
}
