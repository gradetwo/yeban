//! raw DEFLATE（RFC 1951）的**只读**解码器：零依赖、有界、无 `unsafe`、不 panic。
//!
//! ## 规范出处
//!
//! RFC 1951 *DEFLATE Compressed Data Format Specification version 1.3* 是**外部**规范，
//! ⛔ 本仓库**不含**它的正文（与 `docs/ledger/integration-rulings-notes.md` 的 R1 同口径）。
//! 本文件按该规范的三处实现：
//!
//! - **§3.1.1** 位的打包顺序：字节内**低位优先**；Huffman 码按**高位在前**填入；
//!   `LEN` / `NLEN` 这类定长字段按低位优先 ⇒ 读出来就是小端 `u16`。
//! - **§3.2.3 / §3.2.4 / §3.2.5** 块类型（`BTYPE` 0/1/2）、固定 Huffman 表、
//!   长度与距离的基值/额外位表。
//! - **§3.2.7** dynamic 块的码长表头（`HLIT` / `HDIST` / `HCLEN` 与 16/17/18 重复码）。
//!
//! `.mxl` 的 ZIP 条目载荷是 **raw** DEFLATE（⛔ 无 zlib 头、⛔ 无 gzip 头）⇒
//! 本模块**不**校验 Adler-32 或 gzip CRC：完整性由 ZIP 条目**自己**的 CRC-32 字段保证
//! （见 [`super`] 的 `read_entry`）。
//!
//! ## 为什么 `.mxl` 需要**完整**窗口
//!
//! 本仓库实测：6 个真 `.mxl`（本机 `/tmp/musicxml/**`，**未提交**）的 `score.xml`
//! DEFLATE 流的最远匹配距离落在 29393..32502 ⇒ 上界 **32502 > 16384**（16 KiB）。
//! 因此 `MAX_WINDOW` 必须是 RFC 1951 的**完整 32 KiB**，16 KiB 的捷径不够。
//!
//! ## 有界性（⛔ 不是音频线程）
//!
//! 本模块**不在**音频线程上被调用（MusicXML / `.mxl` 是离线导入）。
//! 因此这里允许分配，但分配**有界**：输出不得超过调用方给的 `max_output`
//! （上界在**每次**写入前检查 ⇒ 一个声明小、实际膨胀大的"压缩炸弹"会在上界处 `Err`，
//! 而不是把内存吃光）。
//!
//! ## 不 panic 的承诺
//!
//! 任意输入只产生 `Ok` 或 [`InflateError`]：没有 `unwrap` / `expect` / 索引恐慌 /
//! 算术溢出（表查找一律用 `get`，长度与距离相加前先查上界）。

/// RFC 1951 §3.2.5 允许的最大 Huffman 码长（单位：**位**）。
const MAX_CODE_BITS: usize = 15;

/// RFC 1951 的滑动窗口上界（单位：**字节**）。
///
/// `32768` 是规范给出的最大值；距离码 29 的基值 24577 加 13 位额外位正好到 32768。
const MAX_WINDOW: usize = 32 * 1024;

/// 解码失败的**类别**：流本身坏了，还是调用方给的上界太小？
///
/// 这两件事必须分开：前者是**输入**的缺陷，后者是**调用方的策略**（同一份合法流换个上界就能读）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InflateErrorKind {
    /// 流本身非法（截断 / 未定义的块类型 / 越界的距离 …）。
    Malformed,
    /// 输出超过了 `max_output`（流可能完全合法）。
    Limit,
}

/// DEFLATE 解码失败的**字面**读数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InflateError {
    /// 出错时**已消费**的输入字节偏移（不是比特偏移；字节内不回退）。
    pub offset: usize,
    /// 一句话说明（`'static` ⇒ 不分配）。
    pub detail: &'static str,
    /// 失败类别（见 [`InflateErrorKind`]）。
    pub kind: InflateErrorKind,
}

impl core::fmt::Display for InflateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "偏移 {} 处 DEFLATE 流非法: {}", self.offset, self.detail)
    }
}

impl std::error::Error for InflateError {}

/// 长度码 257..=285 的基值（RFC 1951 §3.2.5 的表；单位：**字节**）。
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];

/// 长度码 257..=285 的额外位数（单位：**位**）。
const LENGTH_EXTRA: [u32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];

/// 距离码 0..=29 的基值（RFC 1951 §3.2.5 的表；单位：**字节**）。
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];

/// 距离码 0..=29 的额外位数（单位：**位**）。
const DIST_EXTRA: [u32; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// dynamic 块里 19 个码长符号的**传输顺序**（RFC 1951 §3.2.7）。
const CODE_LENGTH_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// 解码一段 **raw** DEFLATE 流（`input` 必须以块边界结束；尾随字节不读）。
///
/// `max_output` 是**膨胀后**字节数的上界（单位：**字节**）。超过即 `Err`，
/// 已分配的内存随即释放。
///
/// # Errors
///
/// 任意非法输入（截断 / 未定义的块类型 / 过度订阅的 Huffman 表 / 越界的距离 /
/// 超过 `max_output`）都返回 [`InflateError`]。任意输入都**不** panic。
pub fn inflate_raw(input: &[u8], max_output: usize) -> Result<Vec<u8>, InflateError> {
    let mut bits = Bits::new(input);
    let mut out: Vec<u8> = Vec::new();
    loop {
        let last = bits.take(1)? == 1;
        let kind = bits.take(2)?;
        match kind {
            0 => stored(&mut bits, &mut out, max_output)?,
            1 => {
                let (literal, distance) = fixed_tables();
                codes(&mut bits, &mut out, max_output, &literal, &distance)?;
            }
            2 => {
                let (literal, distance) = dynamic_tables(&mut bits)?;
                codes(&mut bits, &mut out, max_output, &literal, &distance)?;
            }
            _ => return Err(bits.error("块类型 3 未定义（RFC 1951 §3.2.3）")),
        }
        if last {
            return Ok(out);
        }
    }
}

/// 低位优先的比特读取器（RFC 1951 §3.1.1）。
struct Bits<'a> {
    data: &'a [u8],
    /// 已从 `data` 取走的字节数（也用作错误的偏移读数）。
    pos: usize,
    /// 已取走但尚未消费的比特（低位对齐）。
    hold: u32,
    /// `hold` 里有效比特数（`0..=23`）。
    bits: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            hold: 0,
            bits: 0,
        }
    }

    fn error(&self, detail: &'static str) -> InflateError {
        InflateError {
            offset: self.pos,
            detail,
            kind: InflateErrorKind::Malformed,
        }
    }

    /// 上界开火的读数（**不是**流的缺陷）。
    fn limit(&self, detail: &'static str) -> InflateError {
        InflateError {
            offset: self.pos,
            detail,
            kind: InflateErrorKind::Limit,
        }
    }

    /// 保证 `hold` 里至少有 `count` 个比特（`count <= 16`）。
    fn ensure(&mut self, count: u32) -> Result<(), InflateError> {
        while self.bits < count {
            let Some(byte) = self.data.get(self.pos) else {
                return Err(self.error("输入在块中途结束"));
            };
            self.hold |= u32::from(*byte) << self.bits;
            self.pos += 1;
            self.bits += 8;
        }
        Ok(())
    }

    /// 取 `count` 个比特（低位先出），返回其无符号值。
    fn take(&mut self, count: u32) -> Result<u32, InflateError> {
        debug_assert!(count <= 16, "定长字段最多 16 位");
        self.ensure(count)?;
        let value = self.hold & ((1u32 << count) - 1);
        self.hold >>= count;
        self.bits -= count;
        Ok(value)
    }

    /// 丢弃**半个字节**的余量，使下一次 `take(8)` 从字节边界开始（stored 块要求）。
    fn align_to_byte(&mut self) {
        let drop = self.bits % 8;
        self.hold >>= drop;
        self.bits -= drop;
    }
}

/// 一个 canonical Huffman 解码表（RFC 1951 §3.2.2 的码构造）。
struct Huffman {
    /// `counts[len]` = 码长为 `len` 的符号数（`counts[0]` 未使用）。
    counts: [u16; MAX_CODE_BITS + 1],
    /// 按 canonical 顺序排列的符号（长度升序、同长度按符号升序）。
    symbols: Vec<u16>,
}

impl Huffman {
    /// 由码长表构造解码表。`Err` 的载荷是**不分配**的说明文字。
    fn build(lengths: &[u8]) -> Result<Self, &'static str> {
        let mut counts = [0u16; MAX_CODE_BITS + 1];
        for &length in lengths {
            let index = usize::from(length);
            if index > MAX_CODE_BITS {
                return Err("码长超过 15 位");
            }
            counts[index] += 1;
        }
        // 全为 0 = 这张表**没有**符号。合法：dynamic 块允许"一个都不用"的距离表
        // （RFC 1951 §3.2.7 的注），只要数据里不出现距离码。
        if usize::from(counts[0]) == lengths.len() {
            return Ok(Self {
                counts,
                symbols: Vec::new(),
            });
        }
        // Kraft 不等式：`left` 是这一层还剩多少码位；为负 = 过度订阅。
        // `left > 0`（不完整）是**合法**的：单个距离码的表天然不完整。
        let mut left: i32 = 1;
        for &count in counts.iter().take(MAX_CODE_BITS + 1).skip(1) {
            left = (left << 1) - i32::from(count);
            if left < 0 {
                return Err("Huffman 码被过度订阅");
            }
        }
        let mut offsets = [0u16; MAX_CODE_BITS + 2];
        for length in 1..=MAX_CODE_BITS {
            offsets[length + 1] = offsets[length] + counts[length];
        }
        let mut symbols = vec![0u16; lengths.len() - usize::from(counts[0])];
        for (symbol, &length) in lengths.iter().enumerate() {
            if length != 0 {
                let slot = usize::from(offsets[usize::from(length)]);
                if let Some(cell) = symbols.get_mut(slot) {
                    *cell = symbol as u16;
                }
                offsets[usize::from(length)] += 1;
            }
        }
        Ok(Self { counts, symbols })
    }

    /// 解出一个符号：逐位走 canonical 码表（`code` 高位先入）。
    fn decode(&self, bits: &mut Bits<'_>) -> Result<u16, InflateError> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for length in 1..=MAX_CODE_BITS {
            code |= bits.take(1)? as i32;
            let count = i32::from(self.counts[length]);
            if code - first < count {
                let slot = index + code - first;
                let Some(&symbol) = usize::try_from(slot)
                    .ok()
                    .and_then(|slot| self.symbols.get(slot))
                else {
                    return Err(bits.error("Huffman 码指向不存在的符号"));
                };
                return Ok(symbol);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(bits.error("Huffman 码不是任何符号的前缀"))
    }
}

/// RFC 1951 §3.2.6 的固定 Huffman 表：literal/length 288 个符号、distance 30 个符号。
///
/// 表本身是常量，但构造要走同一套 `build` ⇒ 出错只会返回 `Err`（⛔ 不 panic）。
fn fixed_tables() -> (Huffman, Huffman) {
    let mut literal_lengths = [0u8; 288];
    for (symbol, length) in literal_lengths.iter_mut().enumerate() {
        *length = match symbol {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    let distance_lengths = [5u8; 30];
    // 两张表都由上面的字面常量导出 ⇒ `build` 只可能成功；此处仍不 panic。
    let literal = Huffman::build(&literal_lengths).unwrap_or(Huffman {
        counts: [0; MAX_CODE_BITS + 1],
        symbols: Vec::new(),
    });
    let distance = Huffman::build(&distance_lengths).unwrap_or(Huffman {
        counts: [0; MAX_CODE_BITS + 1],
        symbols: Vec::new(),
    });
    (literal, distance)
}

/// stored 块（`BTYPE = 0`）：先把比特游标对齐到字节，再按 `LEN` 复制。
fn stored(bits: &mut Bits<'_>, out: &mut Vec<u8>, max_output: usize) -> Result<(), InflateError> {
    bits.align_to_byte();
    let length = bits.take(16)?;
    let complement = bits.take(16)?;
    if length ^ complement != 0xffff {
        return Err(bits.error("stored 块的 LEN 与其反码 NLEN 不互补"));
    }
    let length = length as usize;
    if out.len() + length > max_output {
        return Err(bits.limit("输出超过上界"));
    }
    for _ in 0..length {
        out.push(bits.take(8)? as u8);
    }
    Ok(())
}

/// dynamic 块（`BTYPE = 2`）的表头：读码长表，再由它展开 literal/length 与 distance 的码长。
fn dynamic_tables(bits: &mut Bits<'_>) -> Result<(Huffman, Huffman), InflateError> {
    let literal_count = bits.take(5)? as usize + 257;
    let distance_count = bits.take(5)? as usize + 1;
    let code_length_count = bits.take(4)? as usize + 4;
    // RFC 1951 §3.2.7：`HLIT` 的合法范围是 257..=286、`HDIST` 是 1..=30。
    if literal_count > 286 {
        return Err(bits.error("literal/length 码数超过 286"));
    }
    if distance_count > 30 {
        return Err(bits.error("distance 码数超过 30"));
    }
    let mut code_lengths = [0u8; 19];
    for &slot in CODE_LENGTH_ORDER.iter().take(code_length_count) {
        code_lengths[slot] = bits.take(3)? as u8;
    }
    let code_lengths = Huffman::build(&code_lengths).map_err(|detail| bits.error(detail))?;

    let mut lengths = vec![0u8; literal_count + distance_count];
    let mut index = 0usize;
    while index < lengths.len() {
        let symbol = code_lengths.decode(bits)?;
        match symbol {
            0..=15 => {
                lengths[index] = symbol as u8;
                index += 1;
            }
            16 => {
                let Some(&previous) = index.checked_sub(1).and_then(|slot| lengths.get(slot))
                else {
                    return Err(bits.error("码长重复码 16 出现在表首（没有可重复的前一项）"));
                };
                let repeat = 3 + bits.take(2)? as usize;
                if index + repeat > lengths.len() {
                    return Err(bits.error("码长重复越过表尾"));
                }
                for _ in 0..repeat {
                    lengths[index] = previous;
                    index += 1;
                }
            }
            17 | 18 => {
                let repeat = if symbol == 17 {
                    3 + bits.take(3)? as usize
                } else {
                    11 + bits.take(7)? as usize
                };
                if index + repeat > lengths.len() {
                    return Err(bits.error("码长重复越过表尾"));
                }
                index += repeat;
            }
            _ => return Err(bits.error("码长符号不是 0..=18")),
        }
    }
    // 没有块结束码 ⇒ 这个块永远不会结束（RFC 1951 §3.2.3 要求码 256 存在）。
    if lengths[256] == 0 {
        return Err(bits.error("缺失块结束码 256"));
    }
    let literal = Huffman::build(&lengths[..literal_count]).map_err(|detail| bits.error(detail))?;
    let distance =
        Huffman::build(&lengths[literal_count..]).map_err(|detail| bits.error(detail))?;
    Ok((literal, distance))
}

/// 压缩块（`BTYPE = 1` / `BTYPE = 2`）的符号循环，直到码 256。
fn codes(
    bits: &mut Bits<'_>,
    out: &mut Vec<u8>,
    max_output: usize,
    literal: &Huffman,
    distance: &Huffman,
) -> Result<(), InflateError> {
    loop {
        let symbol = literal.decode(bits)?;
        match symbol {
            0..=255 => {
                if out.len() >= max_output {
                    return Err(bits.limit("输出超过上界"));
                }
                out.push(symbol as u8);
            }
            256 => return Ok(()),
            _ => {
                let length_index = usize::from(symbol - 257);
                let (Some(&base), Some(&extra)) = (
                    LENGTH_BASE.get(length_index),
                    LENGTH_EXTRA.get(length_index),
                ) else {
                    return Err(bits.error("长度码 286/287 未定义（RFC 1951 §3.2.5）"));
                };
                let length = usize::from(base) + bits.take(extra)? as usize;

                let distance_symbol = distance.decode(bits)?;
                let distance_index = usize::from(distance_symbol);
                let (Some(&base), Some(&extra)) = (
                    DIST_BASE.get(distance_index),
                    DIST_EXTRA.get(distance_index),
                ) else {
                    return Err(bits.error("距离码 30/31 未定义（RFC 1951 §3.2.5）"));
                };
                let back = usize::from(base) + bits.take(extra)? as usize;

                if back > MAX_WINDOW {
                    return Err(bits.error("匹配距离超过 32 KiB 窗口"));
                }
                if back > out.len() {
                    return Err(bits.error("匹配距离超过已输出字节数"));
                }
                if out.len() + length > max_output {
                    return Err(bits.limit("输出超过上界"));
                }
                // 逐字节回拷：`back` 可以小于 `length`（重叠匹配），因此不能整段 `copy`。
                let start = out.len() - back;
                for offset in 0..length {
                    let byte = out[start + offset];
                    out.push(byte);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// stored 块（`BTYPE = 0`）的**测试侧**编码器：本模块的对照臂。
    fn encode_stored(payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() <= 0xffff, "单块 LEN 是 16 位");
        let mut out = vec![0x01u8]; // BFINAL=1, BTYPE=00（低位先出）, 余 5 位补 0
        let length = payload.len() as u16;
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&(!length).to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn stored_block_round_trips() {
        for payload in [b"".as_slice(), b"a", b"hello DEFLATE", &[0xffu8; 300]] {
            let stream = encode_stored(payload);
            assert_eq!(inflate_raw(&stream, 1 << 20).as_deref(), Ok(payload));
        }
    }

    #[test]
    fn stored_block_checks_the_length_complement() {
        let mut stream = encode_stored(b"abc");
        stream[3] ^= 0x01; // 破坏 NLEN
        assert_eq!(
            inflate_raw(&stream, 1 << 20),
            Err(InflateError {
                offset: 5,
                detail: "stored 块的 LEN 与其反码 NLEN 不互补",
                kind: InflateErrorKind::Malformed
            })
        );
    }

    #[test]
    fn stored_block_respects_the_output_limit() {
        let stream = encode_stored(&[0u8; 100]);
        assert_eq!(
            inflate_raw(&stream, 99),
            Err(InflateError {
                offset: 5,
                detail: "输出超过上界",
                kind: InflateErrorKind::Limit
            })
        );
    }

    #[test]
    fn reserved_block_type_is_rejected() {
        // BFINAL=0, BTYPE=11 ⇒ 第一字节的低 3 位是 0b110 = 0x06。
        assert_eq!(
            inflate_raw(&[0x06], 16),
            Err(InflateError {
                offset: 1,
                detail: "块类型 3 未定义（RFC 1951 §3.2.3）",
                kind: InflateErrorKind::Malformed
            })
        );
    }

    #[test]
    fn truncated_input_is_an_error_not_a_panic() {
        let stream = encode_stored(b"abcdefgh");
        for cut in 0..stream.len() {
            let result = inflate_raw(&stream[..cut], 1 << 20);
            assert!(result.is_err(), "{cut} 字节的前缀不可能是完整流");
        }
        assert_eq!(
            inflate_raw(&stream, 1 << 20).as_deref(),
            Ok(&b"abcdefgh"[..])
        );
    }

    #[test]
    fn over_subscribed_huffman_table_is_rejected() {
        // 3 个符号都给码长 1 ⇒ Kraft 和 = 3/2 > 1 ⇒ 过度订阅。
        assert_eq!(
            Huffman::build(&[1, 1, 1]).err(),
            Some("Huffman 码被过度订阅")
        );
        // 2 个符号各 1 位是**恰好完整**的表 ⇒ 接受。
        assert!(Huffman::build(&[1, 1]).is_ok());
        // 全 0 的表是"没有符号"，不是错误（dynamic 块允许不用距离码）。
        assert!(Huffman::build(&[0, 0, 0]).is_ok());
    }

    #[test]
    fn empty_huffman_table_cannot_decode() {
        let table = Huffman::build(&[0, 0, 0]).expect("全 0 表合法");
        let mut bits = Bits::new(&[0xff, 0xff]);
        assert_eq!(
            table.decode(&mut bits),
            Err(InflateError {
                offset: 2,
                detail: "Huffman 码不是任何符号的前缀",
                kind: InflateErrorKind::Malformed
            })
        );
    }
}
