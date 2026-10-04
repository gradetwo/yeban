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
//! `Proposal::id` / 提交身份 / 保存用的临时文件名仍然用 [`EntityId::new`]
//! （那里"可预测"反而是缺点：两个并发会话必须拿到不同的提案身份）。
//! 本模块只用于"同一请求应当产出同一载荷"的**确定性**场景。

use std::str::FromStr as _;

use yeban_model::EntityId;

/// Crockford Base32 字母表（ULID 规范；排除了 `I` `L` `O` `U`）。
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// FNV-1a 128 的偏移基。
const FNV_OFFSET_BASIS_128: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;

/// FNV-1a 128 的质数（2^88 + 0x13B）。
const FNV_PRIME_128: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;

/// `label` 的 128 bit 稳定摘要（FNV-1a，跨进程、跨平台、跨重启一致）。
#[must_use]
pub fn digest128(label: &str) -> u128 {
    let mut hash = FNV_OFFSET_BASIS_128;
    for byte in label.as_bytes() {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME_128);
    }
    hash
}

/// 把 128 bit 值编码成 26 字符 Crockford Base32（ULID 文本形态，前 2 bit 补零）。
#[must_use]
pub fn to_ulid_text(value: u128) -> String {
    let mut out = [0_u8; 26];
    let mut rest = value;
    for slot in out.iter_mut().rev() {
        *slot = CROCKFORD[(rest & 0x1f) as usize];
        rest >>= 5;
    }
    // 安全: 26 个字节全部来自 ASCII 字母表。
    String::from_utf8(out.to_vec()).expect("Crockford 字母表是 ASCII")
}

/// 由标签派生一个确定性 `EntityId`。
///
/// # Panics
///
/// 不会 panic：编码器与 `EntityId::from_str` 的口径由单元判据钉住
/// （`deterministic_ids_are_stable_canonical_ulids`）。
#[must_use]
pub fn deterministic_id(label: &str) -> EntityId {
    EntityId::from_str(&to_ulid_text(digest128(label))).expect("编码器产出合法 ULID")
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // 不同标签 → 不同身份。
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
        // 字母表里没有 I / L / O / U。
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
}
