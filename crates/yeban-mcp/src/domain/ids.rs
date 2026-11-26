//! **确定性**夹具身份：`FNV-1a(128) → Crockford Base32 ULID`。
//!
//! ## 为什么需要它
//!
//! 工具产出的 `Op` 里带身份（`SectionV3::id`、`MidiNote::id`、`RoutingEdge::id`……）。
//! 如果这些身份用 `EntityId::new()`（ULID 里含 80 bit 加密随机），同一个请求两次
//! 会产出**两套不同的 op 载荷**，于是：
//!
//! 1. `dryRun` 的预览不可能等于真调用将要施加的东西 —— 预览只能是"大概是这么几步"；
//! 2. 判据无法断言"预览 == 实际"，只能断言"步数一样"。
//!
//! 本模块让身份**是参数的纯函数**，于是 `dryRun` 的预览与真调用施加的 `Op`
//! 可以逐字节相同（判据 `dry_run_preview_ops_equal_the_ops_actually_committed`）。
//!
//! ## 这不是加密随机源，也**不是**工程实体身份的分配器
//!
//! `Proposal::id` / 提交身份 / 保存用的临时文件名仍然用
//! [`yeban_model::EntityId::new`]。
//! 本模块只用于"同一请求应当产出同一载荷"的**确定性**场景。
//!
//! ## 单一实现
//!
//! 派生算法在 [`yeban_model::ids`]；这里只做再导出，因为渲染侧也要用同一个
//! [`macro_cascade_point_id`] 识别物化点（两条路径共享一份身份公式）。

pub use yeban_model::ids::{deterministic_id, digest128, macro_cascade_point_id, to_ulid_text};

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr as _;

    use yeban_model::EntityId;

    #[test]
    fn deterministic_ids_are_stable_canonical_ulids() {
        let first = deterministic_id("section:Chorus:cinematic-orchestral:8");
        let second = deterministic_id("section:Chorus:cinematic-orchestral:8");
        assert_eq!(first, second, "同一标签必须产出同一身份");
        let text = first.to_canonical_string();
        assert_eq!(text.len(), 26);
        // 130 bit 编码里最高的 2 bit 是补零位, 因此首字符只可能是 '0'..='7'
        // （ULID 的规范形态；`Ulid::from_string` 拒绝首字符 > '7' 的文本）。
        assert!(
            matches!(text.as_bytes()[0], b'0'..=b'7'),
            "ULID 首字符必须在 0..=7: {text}"
        );
        assert_eq!(
            EntityId::from_str(&text).expect("回环"),
            first,
            "文本形态必须能被解析回同一身份"
        );
        assert_ne!(
            first,
            deterministic_id("section:Chorus:cinematic-orchestral:9")
        );
    }

    #[test]
    fn ulid_text_encoding_is_crockford_base32() {
        assert_eq!(to_ulid_text(0), "00000000000000000000000000");
        assert_eq!(to_ulid_text(1), "00000000000000000000000001");
        assert_eq!(to_ulid_text(31), "0000000000000000000000000Z");
        assert_eq!(to_ulid_text(32), "00000000000000000000000010");
        let text = to_ulid_text(u128::MAX);
        assert_eq!(text.len(), 26);
        for banned in ['I', 'L', 'O', 'U'] {
            assert!(!text.contains(banned), "Crockford 字母表不含 {banned}");
        }
    }

    #[test]
    fn digest_is_order_sensitive_and_not_constant() {
        assert_ne!(digest128("ab"), digest128("ba"));
        assert_ne!(digest128(""), 0);
    }

    #[test]
    fn macro_cascade_point_ids_are_the_same_formula_as_the_callers() {
        let track = EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8P001").expect("ULID");
        assert_eq!(
            macro_cascade_point_id(track, 0, 0, 0),
            deterministic_id("macro-cascade:01J8Z5Q0R7K3M9X2V4B6N8P001:0:0:0")
        );
    }
}
