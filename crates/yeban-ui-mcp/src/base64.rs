//! 零依赖 Base64（RFC 4648 §4，标准字母表 + `=` 填充）。
//!
//! ## 为什么需要它
//!
//! `[UI-MCP-001]` §12.3 的 `ReadOnly` 层包含"无头 Framebuffer 截图捕获"，
//! 而 JSON-RPC 的 `result` 是 JSON —— JSON 里没有字节串类型。
//! 把 PNG 字节塞进 `[255, 80, 78, ...]` 这样的数字数组会让 1080p 的截图膨胀约 4 倍
//! （base64 只膨胀 4/3），而 base64 是 HTTP/JSON 生态里对二进制最通用的编码。
//!
//! ## 为什么手写而不是加 `base64` crate
//!
//! 与 `yeban-ui-test-port` 手写 PNG 的理由同源（`docs/adr/ADR-0001` D20/D21 的零新增依赖倾向）：
//! 这点需求（一个字母表、一张查表、一次往返）不值得多一个依赖与一处版本漂移面，
//! 而且手写之后"往返必须无损"这条判据可以在**本机**（零 Slint）真跑。
//!
//! ## 严格性
//!
//! [`decode`] 是**严格**的：长度必须是 4 的倍数、只接受标准字母表、
//! 填充只允许出现在末尾。宽松解码（忽略非法字符、接受 URL-safe 字母表）会让
//! "调用方编码错了但看起来成功了"变成一条静默的假绿路径。

/// 标准字母表（RFC 4648 §4）。
pub const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 填充字符。
pub const PADDING: u8 = b'=';

/// 编码错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Base64Error {
    /// 长度不是 4 的倍数（含填充在内）。
    #[error("base64 长度必须是 4 的倍数, 实际 {actual}")]
    BadLength {
        /// 实际长度。
        actual: usize,
    },
    /// 出现标准字母表以外的字符。
    #[error("base64 第 {index} 个字节 `{byte}` 不在标准字母表里")]
    BadCharacter {
        /// 字节下标。
        index: usize,
        /// 出错的字节（以可打印形式给出）。
        byte: char,
    },
    /// `=` 填充的位置非法（只允许出现在末尾，且最多两个）。
    #[error("base64 填充位置非法 (下标 {index})")]
    BadPadding {
        /// 出错的字节下标。
        index: usize,
    },
}

/// 标准字母表的反查表：`None` 表示非法字节。
const fn reverse_table() -> [Option<u8>; 256] {
    let mut table = [None; 256];
    let mut index = 0_usize;
    while index < 64 {
        table[ALPHABET[index] as usize] = Some(index as u8);
        index += 1;
    }
    table
}

/// 反查表（编译期算好）。
static REVERSE: [Option<u8>; 256] = reverse_table();

/// 编码：标准字母表 + `=` 填充（与 `base64::engine::general_purpose::STANDARD` 同口径）。
#[must_use]
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).copied().map_or(0, u32::from);
        let b2 = chunk.get(2).copied().map_or(0, u32::from);
        let triple = (b0 << 16) | (b1 << 8) | b2;
        let indices = [
            (triple >> 18) & 0x3f,
            (triple >> 12) & 0x3f,
            (triple >> 6) & 0x3f,
            triple & 0x3f,
        ];
        for (position, index) in indices.iter().enumerate() {
            if position > chunk.len() {
                out.push(char::from(PADDING));
            } else {
                out.push(char::from(ALPHABET[*index as usize]));
            }
        }
    }
    out
}

/// 严格解码。
///
/// # Errors
///
/// 长度不是 4 的倍数、出现字母表外的字符、或 `=` 位置非法。
pub fn decode(text: &str) -> Result<Vec<u8>, Base64Error> {
    let raw = text.as_bytes();
    if raw.len() % 4 != 0 {
        return Err(Base64Error::BadLength { actual: raw.len() });
    }
    let mut out = Vec::with_capacity(raw.len() / 4 * 3);
    let mut padding_seen = false;
    for (chunk_index, chunk) in raw.chunks(4).enumerate() {
        let mut values = [0_u8; 4];
        let mut padding = 0_usize;
        for (offset, byte) in chunk.iter().enumerate() {
            let index = chunk_index * 4 + offset;
            if *byte == PADDING {
                padding += 1;
                padding_seen = true;
                // 填充只能出现在这一组的末尾, 且这一组必须是最后一组。
                if offset < 2 || chunk_index * 4 + 4 != raw.len() {
                    return Err(Base64Error::BadPadding { index });
                }
                continue;
            }
            if padding_seen {
                return Err(Base64Error::BadPadding { index });
            }
            values[offset] = REVERSE[*byte as usize].ok_or(Base64Error::BadCharacter {
                index,
                byte: char::from(*byte),
            })?;
        }
        if padding > 2 {
            return Err(Base64Error::BadPadding {
                index: chunk_index * 4,
            });
        }
        let triple = (u32::from(values[0]) << 18)
            | (u32::from(values[1]) << 12)
            | (u32::from(values[2]) << 6)
            | u32::from(values[3]);
        out.push((triple >> 16) as u8);
        if padding < 2 {
            out.push((triple >> 8) as u8);
        }
        if padding < 1 {
            out.push(triple as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 判据: RFC 4648 §10 的官方测试向量 (含三种填充情形) 与往返无损。
    #[test]
    fn rfc4648_vectors_round_trip() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(plain.as_bytes()), encoded, "编码 `{plain}`");
            assert_eq!(
                decode(encoded).expect("解码官方向量"),
                plain.as_bytes(),
                "解码 `{encoded}`"
            );
        }
    }

    /// 判据: **全 256 个字节值**都能无损往返 (不是只试 ASCII)。
    ///
    /// PNG 的前 8 个字节 `89 50 4E 47 0D 0A 1A 0A` 里就有 >0x7F 的字节 ——
    /// 这条判据防的正是"只在可打印 ASCII 上测过"的那类假绿。
    #[test]
    fn every_byte_value_round_trips() {
        let all: Vec<u8> = (0..=255_u8).collect();
        let encoded = encode(&all);
        assert_eq!(decode(&encoded).expect("解码全字节"), all);
        for length in 0..64_usize {
            let slice = &all[..length];
            let round_trip = decode(&encode(slice)).expect("解码");
            assert_eq!(round_trip, slice, "长度 {length}");
        }
    }

    /// 判据: 非法输入必须**报错**而不是宽松跳过。
    #[test]
    fn malformed_input_is_rejected() {
        assert_eq!(decode("Zg="), Err(Base64Error::BadLength { actual: 3 }));
        assert!(matches!(
            decode("Zm9v\n"),
            Err(Base64Error::BadLength { actual: 5 })
        ));
        assert!(matches!(
            decode("Zm-v"),
            Err(Base64Error::BadCharacter {
                index: 2,
                byte: '-'
            })
        ));
        // URL-safe 字母表不被接受: 严格性是有意的。
        assert!(matches!(
            decode("Zm9_"),
            Err(Base64Error::BadCharacter { index: 3, .. })
        ));
        // 填充出现在中间。
        assert!(matches!(
            decode("Z=9v"),
            Err(Base64Error::BadPadding { .. })
        ));
    }
}
