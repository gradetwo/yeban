//! 身份标识与时间基准 [MODEL-AST-001]。
//!
//! 本模块定义全项目唯一的身份体系与 tick 时钟基准：
//!
//! - [`PPQ`] — 960 PPQ 整数时钟（每四分音符 960 tick）。
//! - [`EntityId`] — 基于 ULID 的稳定实体身份，**文本形式为 26 字符 Crockford Base32**。
//! - [`AssetHash`] — 原始资产的不可变 SHA-256（内容寻址存储 CAS 的键）。
//! - [`ContentHash`] — 提交快照树的 SHA-256。
//!
//! 规范要求 `EntityId` 的序列化必须是 26 字符 Crockford Base32 且**大小写不敏感**。
//! `ulid` 3.0.0 不再提供 `serde` feature，因此这里手写 `Serialize` / `Deserialize`，
//! 把规范要求固定在类型上，而不是依赖第三方 crate 的实现细节。

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::error::ModelError;

/// 每四分音符的 tick 数 [MODEL-AST-001]。
///
/// 960 = 2^6 × 3 × 5，可被 2、3、4、5、6、8、12、16、24、32、64 整除，
/// 因此二分/三连音/五连音网格都能用整数表示，永不产生浮点累计误差。
pub const PPQ: u64 = 960;

/// ULID 的 Crockford Base32 规范文本长度（26 字符）。
pub const ULID_TEXT_LEN: usize = 26;

/// SHA-256 十六进制摘要的规范文本长度（64 字符）。
pub const SHA256_HEX_LEN: usize = 64;

/// 全项目实体的稳定身份 [MODEL-AST-001]。
///
/// - 序列化形式：26 字符 Crockford Base32（大写），`#[serde(transparent)]` 语义。
/// - 反序列化：接受大小写混写，规范化后一律大写输出。
/// - 排序：ULID 高位为 48-bit 毫秒时间戳，因此 `Ord` 与创建时间同序，
///   满足 [MODEL-AST-003] 对跨进程迭代顺序一致性的要求。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId(pub ulid::Ulid);

impl EntityId {
    /// 生成一个新的随机 `EntityId`。
    ///
    /// # 规范漂移记录
    ///
    /// 规范正文写的是 `ulid::Ulid::new()`，那是 `ulid` 1.x 的 API。本工作区钉死
    /// `ulid` 3.0.0，其等价函数为 `Ulid::generate()`（当前时间 + 随机低位）。
    /// 漂移被限制在此处一行，对外 API 仍是 `EntityId::new()`，调用方无感。
    /// 详见 `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md`。
    #[must_use]
    pub fn new() -> Self {
        Self(ulid::Ulid::generate())
    }

    /// 从已有 ULID 构造。
    #[must_use]
    pub const fn from_ulid(ulid: ulid::Ulid) -> Self {
        Self(ulid)
    }

    /// 取出内部 ULID。
    #[must_use]
    pub const fn as_ulid(&self) -> ulid::Ulid {
        self.0
    }

    /// 规范文本形式（26 字符、大写 Crockford Base32）。
    #[must_use]
    pub fn to_canonical_string(&self) -> String {
        self.0.to_string()
    }

    /// `true` 表示这是空 (`nil`) 身份，即 [`Default`] 值。
    #[must_use]
    pub fn is_nil(&self) -> bool {
        self.0.is_nil()
    }
}

impl Default for EntityId {
    /// [MODEL-AST-001, rev2] 要求派生 `Default`：返回 `nil` ULID（全 0，26 个 `0`）。
    fn default() -> Self {
        Self(ulid::Ulid::nil())
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_canonical_string())
    }
}

impl FromStr for EntityId {
    type Err = ModelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ulid::Ulid::from_string(s)
            .map(Self)
            .map_err(|_| ModelError::InvalidEntityId {
                value: s.to_owned(),
            })
    }
}

impl Serialize for EntityId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_canonical_string())
    }
}

impl<'de> Deserialize<'de> for EntityId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::from_str(&raw).map_err(serde::de::Error::custom)
    }
}

/// 原始资产内容的 SHA-256 摘要 [MODEL-AST-007]。
///
/// 用作内容寻址存储 (CAS) 的键：`.yeban` 归档内资产路径即 `assets/{sha256}`。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetHash(
    /// 64 字符小写十六进制 SHA-256 摘要。
    pub String,
);

impl AssetHash {
    /// 计算字节流的 SHA-256 并包装为 [`AssetHash`]。
    #[must_use]
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(hex_lower(&Sha256::digest(bytes)))
    }

    /// 把已有摘要文本解析为 [`AssetHash`]，拒绝非规范形式。
    ///
    /// # Errors
    ///
    /// 当文本不是 64 位小写十六进制时返回 [`ModelError::InvalidHash`]。
    pub fn parse(value: impl Into<String>) -> Result<Self, ModelError> {
        let value = value.into();
        if is_lower_hex_256(&value) {
            Ok(Self(value))
        } else {
            Err(ModelError::InvalidHash { value })
        }
    }

    /// 规范文本形式。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AssetHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for AssetHash {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for AssetHash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(raw).map_err(serde::de::Error::custom)
    }
}

/// [`AssetHash`] 的**增量（流式）**计算器 [MODEL-AST-007]。
///
/// ## 为什么模型层要有它
///
/// [`AssetHash::of_bytes`] 要求**完整字节**同时在内存里。`crates/yeban-decode` 的
/// `import_path` 因此只能把整份输入文件读进内存，`DecodedAsset::pcm_hash` 还会为此
/// 再分配一整份样本缓冲。该线把这一点登记成 needs，原文（`docs/ledger/decode-limits-notes.md` §8）：
///
/// > **增量 / 流式 SHA-256** | `AssetHash::of_bytes` 要求完整字节……
/// > 建议在 `yeban-model` 侧提供 `AssetHasher`（`update`/`finalize`），本 crate 就能把两条路径都压到 O(块)。
///
/// 障碍不是"没人想到流式"，而是**状态机没有归属**：`crates/yeban-decode` 依赖表里
/// **没有 `sha2`**（`crates/yeban-decode/Cargo.toml`），自己维护一份摘要状态机等于新增依赖。
/// 把状态机放进模型层（`sha2` 已是本 crate 的既有依赖）后，消费者可以按块喂入。
///
/// ## 契约
///
/// 对**任意**分块方式，逐块 `update` 之后 `finalize` 的结果恒等于
/// [`AssetHash::of_bytes`] 对同一字节串的结果。判据：
/// `ids::tests::streaming_hash_equals_of_bytes_for_every_chunking`（单元）与
/// `tests/hash_streaming.rs`（属性测试，随机切点）。
///
/// ## 用法
///
/// ```
/// # use yeban_model::{AssetHash, AssetHasher};
/// let mut hasher = AssetHasher::new();
/// hasher.update(b"ye");
/// hasher.update(b"ban");
/// assert_eq!(hasher.finalize(), AssetHash::of_bytes(b"yeban"));
/// ```
#[derive(Clone, Debug)]
pub struct AssetHasher {
    /// 底层的 `sha2` 摘要状态机（唯一持有者，不进任何持久化结构）。
    inner: Sha256,
}

impl AssetHasher {
    /// 未喂入任何字节的新状态。
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Sha256::new(),
        }
    }

    /// 喂入下一块字节。可调用任意多次，调用次数与分块大小都不影响结果。
    pub fn update(&mut self, bytes: &[u8]) {
        self.inner.update(bytes);
    }

    /// 结束计算并产出规范 [`AssetHash`]（64 字符小写十六进制）。
    #[must_use]
    pub fn finalize(self) -> AssetHash {
        AssetHash(hex_lower(&self.inner.finalize()))
    }
}

impl Default for AssetHasher {
    /// 与 [`AssetHasher::new`] 等价（空状态）。
    fn default() -> Self {
        Self::new()
    }
}

/// 提交快照树的 SHA-256 摘要 [ARCH-OPS-002]。
///
/// 与 [`AssetHash`] 同为内容寻址键，但语义不同：`ContentHash` 标识一次
/// 提交的全量工程快照树，`AssetHash` 标识一份不可变的原始媒体资产。
/// 两者有意做成**不同类型**，避免在 API 边界上互换。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentHash(
    /// 64 字符小写十六进制 SHA-256 摘要。
    pub String,
);

impl ContentHash {
    /// 计算字节流的 SHA-256 并包装为 [`ContentHash`]。
    #[must_use]
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(hex_lower(&Sha256::digest(bytes)))
    }

    /// 把已有摘要文本解析为 [`ContentHash`]，拒绝非规范形式。
    ///
    /// # Errors
    ///
    /// 当文本不是 64 位小写十六进制时返回 [`ModelError::InvalidHash`]。
    pub fn parse(value: impl Into<String>) -> Result<Self, ModelError> {
        let value = value.into();
        if is_lower_hex_256(&value) {
            Ok(Self(value))
        } else {
            Err(ModelError::InvalidHash { value })
        }
    }

    /// 规范文本形式。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for ContentHash {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ContentHash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(raw).map_err(serde::de::Error::custom)
    }
}

/// 小写十六进制编码（避免为一个格式化需求引入额外依赖）。
fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

/// 判断是否为 64 字符小写十六进制（规范摘要形式）。
fn is_lower_hex_256(value: &str) -> bool {
    value.len() == SHA256_HEX_LEN
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ppq_is_960() {
        assert_eq!(PPQ, 960);
        // 960 的整除性正是选择它的理由：常见网格与连音都落在整数 tick 上。
        for divisor in [2_u64, 3, 4, 5, 6, 8, 12, 16, 24, 32, 64] {
            assert_eq!(PPQ % divisor, 0, "PPQ must be divisible by {divisor}");
        }
    }

    #[test]
    fn entity_id_default_is_nil_and_canonical() {
        let id = EntityId::default();
        assert!(id.is_nil());
        assert_eq!(id.to_canonical_string().len(), ULID_TEXT_LEN);
        assert_eq!(id.to_canonical_string(), "0".repeat(ULID_TEXT_LEN));
    }

    #[test]
    fn entity_id_json_round_trip_is_26_chars() {
        let id = EntityId::new();
        let json = serde_json::to_string(&id).expect("serialize");
        let text = json.trim_matches('"');
        assert_eq!(text.len(), ULID_TEXT_LEN, "canonical text must be 26 chars");
        assert!(
            text.bytes().all(|b| !b.is_ascii_lowercase()),
            "canonical text must be uppercase, got {text}"
        );
        let back: EntityId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, id);
    }

    #[test]
    fn entity_id_deserialization_is_case_insensitive() {
        let id = EntityId::new();
        let upper = serde_json::to_string(&id).expect("serialize");
        let lower = format!("\"{}\"", id.to_canonical_string().to_lowercase());
        assert_ne!(upper, lower, "test needs an actually lowercased input");
        let from_lower: EntityId = serde_json::from_str(&lower).expect("lowercase must parse");
        assert_eq!(from_lower, id);
        // 规范化输出恒为大写
        assert_eq!(
            serde_json::to_string(&from_lower).expect("serialize"),
            upper
        );
    }

    #[test]
    fn entity_id_rejects_non_ulid_text() {
        // 26 个字符但含非法 Crockford 字母 (U 被排除)
        let bad = "\"UUUUUUUUUUUUUUUUUUUUUUUUUU\"";
        let err = serde_json::from_str::<EntityId>(bad).expect_err("must fail");
        assert!(err.to_string().contains("Crockford"), "got: {err}");
    }

    #[test]
    fn entity_id_ordering_is_stable_and_time_ordered() {
        let a = EntityId::from_str("00000000000000000000000000").expect("parse");
        let b = EntityId::from_str("00000000000000000000000001").expect("parse");
        assert!(a < b);
        // 排序与文本排序一致 —— BTreeMap 迭代顺序因此可直接对外承诺
        assert!(a.to_canonical_string() < b.to_canonical_string());
    }

    #[test]
    fn asset_hash_matches_known_sha256_vector() {
        // 空输入的 SHA-256 是众所周知的测试向量
        let hash = AssetHash::of_bytes(b"");
        assert_eq!(
            hash.as_str(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(hash.as_str().len(), SHA256_HEX_LEN);
    }

    #[test]
    fn streaming_hash_equals_of_bytes_for_every_chunking() {
        // 分块边界必须落在 SHA-256 分组边界（64 字节）的**内外**都成立，
        // 因此样本长度取 0 / 1 / 63 / 64 / 65。
        let cases: [&[u8]; 5] = [b"", b"a", &[0x5a; 63], &[0x5a; 64], &[0x5a; 65]];
        for bytes in cases {
            let expected = AssetHash::of_bytes(bytes);
            for chunk in [1_usize, 7, 64, 1000] {
                let mut hasher = AssetHasher::new();
                for part in bytes.chunks(chunk) {
                    hasher.update(part);
                }
                assert_eq!(
                    hasher.finalize(),
                    expected,
                    "chunk={chunk} len={} 的分块结果必须与 of_bytes 相同",
                    bytes.len()
                );
            }
            // 一次喂完是 `chunks(len)` 的退化情形，单独钉住。
            let mut one_shot = AssetHasher::new();
            one_shot.update(bytes);
            assert_eq!(one_shot.finalize(), expected);
        }
    }

    #[test]
    fn default_asset_hasher_is_the_empty_digest() {
        // 空状态必须等于空字节串的摘要（另一个独立已知向量）。
        assert_eq!(AssetHasher::default().finalize(), AssetHash::of_bytes(b""));
        assert_eq!(
            AssetHasher::new().finalize().as_str(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn streaming_hash_is_sensitive_to_the_last_bit() {
        // "喂了字节"本身要被证明：只差最后一位 ⇒ 摘要必须不同。
        let mut differing = AssetHasher::new();
        differing.update(&[0x00, 0x01]);
        let mut base = AssetHasher::new();
        base.update(&[0x00, 0x00]);
        assert_ne!(differing.finalize(), base.finalize());
        // 分块位置也不能改变结果（同一位翻转，换一个切点喂）。
        let mut split = AssetHasher::new();
        split.update(&[0x00]);
        split.update(&[0x01]);
        assert_eq!(split.finalize(), AssetHash::of_bytes(&[0x00, 0x01]));
    }

    #[test]
    fn content_hash_and_asset_hash_agree_on_the_same_bytes() {
        let bytes = b"yeban";
        assert_eq!(
            AssetHash::of_bytes(bytes).as_str(),
            ContentHash::of_bytes(bytes).as_str()
        );
    }

    #[test]
    fn hash_parsing_rejects_non_canonical_text() {
        let upper = "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855";
        assert!(
            AssetHash::parse(upper).is_err(),
            "uppercase must be rejected"
        );
        assert!(AssetHash::parse("abc").is_err(), "short must be rejected");
        assert!(
            AssetHash::parse("z".repeat(64)).is_err(),
            "non-hex must be rejected"
        );
        let ok = "e".repeat(SHA256_HEX_LEN);
        assert!(AssetHash::parse(ok.clone()).is_ok());
        assert_eq!(ContentHash::parse(ok.clone()).expect("parse").as_str(), ok);
    }

    /// SHA-256 摘要的规范文本是**恰好** 64 个小写十六进制字符，多一位也不行。
    ///
    /// 实测：把 `value.len() == SHA256_HEX_LEN` 改成 `>=` 时全仓判据保持全绿 ——
    /// 既有判据覆盖了"太短""非十六进制""大写"，却没有覆盖"太长"。超长摘要不是外观
    /// 问题：它会作为 `assets/{hash}` 的 CAS 键落进归档，而 CAS 键必须是内容寻址的
    /// 规范形态（`MODEL-AST-007`）。
    #[test]
    fn hash_parsing_requires_exactly_sixty_four_hex_digits() {
        let too_short = "e".repeat(SHA256_HEX_LEN - 1);
        let too_long = "e".repeat(SHA256_HEX_LEN + 1);
        assert_eq!(
            AssetHash::parse(too_short.clone()),
            Err(ModelError::InvalidHash { value: too_short })
        );
        assert_eq!(
            ContentHash::parse(too_long.clone()),
            Err(ModelError::InvalidHash {
                value: too_long.clone()
            })
        );
        // serde 入口同一把尺子（`assets/{hash}` 的键是从 JSON 读进来的）。
        assert!(
            serde_json::from_str::<AssetHash>(&format!("\"{too_long}\"")).is_err(),
            "超长摘要不得从 JSON 进来"
        );
        // 正侧对照：恰好 64 位必须放行（否则本判据会退化成"什么都拒绝"）。
        assert!(AssetHash::parse("e".repeat(SHA256_HEX_LEN)).is_ok());
    }

    #[test]
    fn hash_serde_round_trip() {
        let hash = AssetHash::of_bytes(b"2026-10-05");
        let json = serde_json::to_string(&hash).expect("serialize");
        let back: AssetHash = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, hash);
        assert!(serde_json::from_str::<AssetHash>("\"not-a-hash\"").is_err());
    }

    #[test]
    fn entity_id_error_carries_offending_value() {
        let err = EntityId::from_str("nope").expect_err("must fail");
        match err {
            ModelError::InvalidEntityId { value } => assert_eq!(value, "nope"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    /// `ContentHash` 的 `Display` 必须是**完整的小写十六进制摘要**。
    ///
    /// 为什么需要（第六轮注入实测）：把 `f.write_str(&self.0)` 换成 `f.write_str("")`
    /// 时全仓判据保持全绿 —— 既有判据只用 `as_str()` / `to_string()` 的**长度**做断言。
    #[test]
    fn content_hash_display_is_the_full_lowercase_hex_digest() {
        let hash = ContentHash::of_bytes(b"abc");
        let text = hash.to_string();
        assert_eq!(
            text, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "Display 必须给出完整摘要, 不是空串或截断"
        );
        assert_eq!(text.len(), SHA256_HEX_LEN);
        assert_eq!(text, hash.as_str());
        assert!(
            text.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    /// `EntityId::from_str` 的**规范口径**：往返稳定，且**不得**静默容忍前后空白。
    ///
    /// 为什么需要（第六轮注入实测）：把 `ulid::Ulid::from_string(s)` 换成
    /// `from_string(&s.trim().to_uppercase())` 时全仓判据保持全绿 —— 既有判据只喂
    /// 规范形态。实测（临时探针）：`ulid` crate 本来就接受小写输入，因此"小写"这一半是
    /// 等价变体；真正被这次注入放宽的是**前后空白**这一半。
    #[test]
    fn entity_id_from_str_is_canonical_and_rejects_surrounding_whitespace() {
        let canonical = "01J8ZQ0000000000000000000A";
        let parsed = EntityId::from_str(canonical).expect("规范形态必须可解析");
        assert_eq!(parsed.to_canonical_string(), canonical);
        // 规范形态恒为 26 个大写字符。
        assert_eq!(parsed.to_canonical_string().len(), ULID_TEXT_LEN);
        assert!(
            parsed
                .to_canonical_string()
                .chars()
                .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
        );
        // 小写输入（`ulid` crate 本来就收）必须**规范化**成同一个身份。
        assert_eq!(
            EntityId::from_str(&canonical.to_lowercase())
                .expect("小写由 ulid crate 接受")
                .to_canonical_string(),
            canonical
        );
        // 前后空白**不得**被静默吞掉。
        assert!(
            EntityId::from_str(&format!(" {canonical}")).is_err(),
            "前导空白必须被拒（不得静默 trim）"
        );
        assert!(
            EntityId::from_str(&format!("{canonical} ")).is_err(),
            "尾随空白必须被拒（不得静默 trim）"
        );
    }
}
