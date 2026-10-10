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
//! ## 多块流（`BFINAL` 链）与其判据
//!
//! RFC 1951 §3.2.3 允许一条流由**多个**块组成：只有 `BFINAL=1` 的块是最后一块。
//! [`inflate_raw`] 因此**循环**读块，并在 `stored` 块之前把比特游标对齐到字节边界
//! （§3.1.1 的余量丢弃）。
//!
//! 已提交语料**碰不到**这条路径：本机 8 个 `.mxl`（2 份已提交夹具 + 6 个真文件
//! `/tmp/musicxml/**`，**未提交**）的 **16/16** 个 DEFLATE 流都是**单块**
//! （首块 `BFINAL=1`；单位 = **流**；量法 = 本机手写的 raw-DEFLATE 块走查）。
//! 该形状由 `crates/yeban-midi/tests/fixtures/README.md` 第 8 节的**自造**夹具
//! （`zlib.compressobj` + `Z_FULL_FLUSH` ⇒ 3 块，中间那块是 `stored`）与判据
//! `mxl_multiblock_deflate_stream_is_read_to_its_last_block` 钉住。
//!
//! ## 为什么 `.mxl` 需要**完整**窗口
//!
//! 本仓库实测：6 个真 `.mxl`（本机 `/tmp/musicxml/**`，**未提交**）的 `score.xml`
//! DEFLATE 流的最远匹配距离落在 29393..32502 ⇒ 上界 **32502 > 16384**（16 KiB）。
//! 因此 `MAX_WINDOW` 必须是 RFC 1951 的**完整 32 KiB**，16 KiB 的捷径不够。
//!
//! ⚠️ **上面那段是未提交语料**，所以它不构成判据。本票之前那 4 份**已提交** `.mxl` 夹具的
//! **8 个** DEFLATE 流的最远匹配距离只有 **1881 / 1881 / 1881 / 1187**（`score.xml`）与
//! 4×**95**（`META-INF/container.xml`）字节（量法 = 手写 raw-DEFLATE 走查器；单位 = 字节）
//! ⇒ 在本票之前，把 `MAX_WINDOW` 从 `32768` 降到 `2048` 也不会让任何已提交判据变红。
//! 本票因此补了两条证据：
//!
//! - **集成**（独立生产者 = CPython `zlib`）：已提交夹具
//!   `tests/fixtures/handmade_mvp_partwise_long_match.mxl` 的 `score.xml` 里有一次距离
//!   **32506** 字节的匹配（配方见 `tests/fixtures/README.md` 第 9 节），由判据
//!   `mxl_long_range_match_needs_the_full_32_kib_window` 钉住。
//! - **单元**（判据自己拼流，⛔ 不依赖生产者）：下面的判据
//!   `full_window_match_is_accepted_and_the_history_check_runs_after_it` 用一条固定 Huffman 流
//!   钉住 RFC 1951 的**精确**最大距离 **32768**（`zlib` 的 `MAX_DIST` 只到 **32506**
//!   = `32768 - MIN_LOOKAHEAD(262)`，所以生产者给的流到不了这个边界）。
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

    /// raw-DEFLATE 的**判据侧**位写出器（RFC 1951 §3.1.1：字节内低位先出）。
    ///
    /// ⛔ 它不是受测代码 —— 只被下面的判据用来**造**输入。它的输出由一个**独立生产者**
    /// 核验过：CPython `zlib.decompressobj(-15)` 对同一份字节给出同样的 `32771` 字节。
    struct BitWriter {
        out: Vec<u8>,
        hold: u32,
        /// `hold` 里已填的比特数（`< 8`）。
        bits: u32,
    }

    impl BitWriter {
        fn new() -> Self {
            Self {
                out: Vec::new(),
                hold: 0,
                bits: 0,
            }
        }

        /// Huffman 码：**高位先**入。
        fn code(&mut self, code: u32, len: u32) {
            for shift in (0..len).rev() {
                self.bit((code >> shift) & 1);
            }
        }

        /// 定长字段与额外位：**低位先**入。
        fn value(&mut self, value: u32, len: u32) {
            for shift in 0..len {
                self.bit((value >> shift) & 1);
            }
        }

        fn bit(&mut self, bit: u32) {
            self.hold |= bit << self.bits;
            self.bits += 1;
            if self.bits == 8 {
                self.out.push(self.hold as u8);
                self.hold = 0;
                self.bits = 0;
            }
        }

        fn finish(mut self) -> Vec<u8> {
            if self.bits > 0 {
                self.out.push(self.hold as u8);
            }
            self.out
        }
    }

    /// 固定 Huffman 表（RFC 1951 §3.2.6）里的一个符号：字面量、长度码、块结束码共用这张表。
    fn fixed_symbol(bits: &mut BitWriter, symbol: u32) {
        match symbol {
            0..=143 => bits.code(0x30 + symbol, 8),
            144..=255 => bits.code(0x190 + (symbol - 144), 9),
            256..=279 => bits.code(symbol - 256, 7),
            // ⭐ 固定表只定义到 **287**（RFC 1951 §3.2.6）。⛔ 不许用 `_` 兜底：
            // `_ => bits.code(0xc0 + (symbol - 280), 8)` 会把 288..=u32::MAX **静默别名**
            // 成 280..（例如 `fixed_symbol(b, 288)` 发的是 symbol **280** 的码），
            // 于是"用这个助手写的判据"会**静默测错对象**。
            280..=287 => bits.code(0xc0 + (symbol - 280), 8),
            _ => panic!("fixed_symbol 只定义到 287，收到 {symbol}（会别名成别的符号）"),
        }
    }

    /// 只支持本判据需要的两个距离码：`1`（码 0）与 `32768`（码 29 + 13 位额外位全 1）。
    fn fixed_distance(bits: &mut BitWriter, distance: u32) {
        match distance {
            1 => bits.code(0, 5),
            32768 => {
                bits.code(29, 5);
                bits.value(8191, 13);
            }
            other => panic!("本编码器只支持距离 1 与 32768，收到 {other}"),
        }
    }

    /// 一个固定 Huffman 块：先铺 `pad` 个 `'a'` 字节（1 个字面 + 若干个长度 258 / 距离 1 的匹配），
    /// 再放一次长度 3 / 距离 `32768` 的匹配，最后是块结束码。`BFINAL = 1`。
    fn fixed_stream(pad: usize) -> Vec<u8> {
        assert!(pad >= 1, "至少要有一个字面量");
        let mut bits = BitWriter::new();
        bits.value(1, 1); // BFINAL = 1
        bits.value(1, 2); // BTYPE = 01（固定 Huffman；低位先出 ⇒ 先 1 后 0）
        fixed_symbol(&mut bits, u32::from(b'a')); // 1 字节
        let mut remaining = pad - 1;
        while remaining >= 258 {
            fixed_symbol(&mut bits, 285); // 长度码 285 = 258 字节
            fixed_distance(&mut bits, 1);
            remaining -= 258;
        }
        for _ in 0..remaining {
            fixed_symbol(&mut bits, u32::from(b'a'));
        }
        fixed_symbol(&mut bits, 257); // 长度码 257 = 3 字节
        fixed_distance(&mut bits, 32768);
        fixed_symbol(&mut bits, 256); // 块结束码
        bits.finish()
    }

    /// 窗口的**精确**上界：距离 `32768`（RFC 1951 §3.2.5 的距离码 29 的最大值）必须可读。
    ///
    /// ## 量什么（单位 = 字节）
    ///
    /// 输入是**判据自己**拼的固定 Huffman 流（长 **213** 字节），输出 **32771** 字节 `'a'`。
    /// `32768` 是 RFC 1951 允许的**最大**距离 ⇒ 接受它意味着 `MAX_WINDOW` **必须 ≥ 32768**。
    /// 这是 `MAX_WINDOW` 精确值的**唯一**证据：集成判据用的独立生产者（`zlib`）到不了这个边界
    /// （它的 `MAX_DIST` 是 `32506`），而 `MAX_WINDOW` 一旦小于 `32768`，两条断言都会报
    /// `"匹配距离超过 32 KiB 窗口"`。
    ///
    /// ## 两项检查的**次序**
    ///
    /// 对照臂：少放**一个**字节（输出 `32767` 字节）时，同一个距离只能由**历史**检查拒绝
    /// （`"匹配距离超过已输出字节数"`）—— 它证明 `back > MAX_WINDOW` 的检查在**先**，
    /// 且它在 `32768` 处**没有**开火（否则报的是窗口越界）。
    #[test]
    fn full_window_match_is_accepted_and_the_history_check_runs_after_it() {
        let full = fixed_stream(32768);
        assert_eq!(full.len(), 213, "拼出来的输入字节数（字面读数）");
        let out = inflate_raw(&full, 1 << 20).expect("距离 32768 是 RFC 1951 的最大值，必须可读");
        assert_eq!(out.len(), 32771);
        assert!(out.iter().all(|&byte| byte == b'a'));

        // 输出上界仍然开火（同一个流，上界只比输出少 1 字节）。
        assert_eq!(
            inflate_raw(&full, 32770),
            Err(InflateError {
                offset: 212,
                detail: "输出超过上界",
                kind: InflateErrorKind::Limit,
            })
        );

        // 对照臂：少一个字节 ⇒ 同一个距离落在**历史**检查上。
        let short = fixed_stream(32767);
        assert_eq!(short.len(), 212, "少一次字面量就少一个字节");
        assert_eq!(
            inflate_raw(&short, 1 << 20),
            Err(InflateError {
                offset: 211,
                detail: "匹配距离超过已输出字节数",
                kind: InflateErrorKind::Malformed,
            })
        );
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

    /// 判据: **任意字节**进 `inflate_raw` 只产生 `Ok` 或 `Err`，绝不 panic。
    ///
    /// 量的是"跑了几次解码"（单位 = 次调用）；任何一次 panic 都会让本判据失败。
    /// 输入分三组：
    ///
    /// 1. **穷举**全部 1 字节（256）与 2 字节（65536）输入 —— 短输入是"块头被切一半"
    ///    的最密集形状，穷举它比抽样强；这一组同时用上界 `usize::MAX`（2 字节最多只能
    ///    膨胀出几百字节，因此取出上界不会变成内存炸弹）。
    /// 2. 种子固定的 xorshift64\* 伪随机字节（长度 0..=1024，2 万次；上界 `1 << 20`）。
    ///    不用系统熵也不引第三方 `rand` ⇒ 可复现。
    /// 3. 手工挑的病态形状：`stored` 的 `LEN` / `NLEN` 极值、保留块类型 3、`dynamic`
    ///    头被切在 `HLIT` / 码长表中间、全 `0xFF`。
    ///
    /// 调用总数因此恒为 `256 + 65536 + 20000 + 16 = 85808`（单位 = 次调用），
    /// 下面只断言一个下界。
    ///
    /// 本判据**不**证明"输出正确"（那由上面的往返与窗口判据负责），只证明"不 panic"。
    #[test]
    fn arbitrary_bytes_never_panic() {
        /// 种子固定的 xorshift64\*。
        struct Rng(u64);
        impl Rng {
            fn next(&mut self) -> u64 {
                let mut x = self.0;
                x ^= x >> 12;
                x ^= x << 25;
                x ^= x >> 27;
                self.0 = x;
                x.wrapping_mul(0x2545_F491_4F6C_DD1D)
            }
        }

        let mut runs: usize = 0;
        let mut probe = |bytes: &[u8], max_output: usize| {
            let _ = inflate_raw(bytes, max_output);
            runs += 1;
        };

        // ① 穷举 1 字节与 2 字节。
        for a in 0..=0xffu8 {
            probe(&[a], usize::MAX);
            for b in 0..=0xffu8 {
                probe(&[a, b], usize::MAX);
            }
        }

        // ② 伪随机长度与内容。
        let mut rng = Rng(0x0BAD_C0DE_F00D_1234);
        for _ in 0..20_000 {
            let len = (rng.next() % 1025) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| (rng.next() >> 33) as u8).collect();
            probe(&bytes, 1 << 20);
        }

        // ③ 病态形状。
        let pathological: [&[u8]; 8] = [
            &[],
            &[0x00],
            &[0x01],
            &[0x06],                         // BFINAL = 0, BTYPE = 11（保留）
            &[0x01, 0x00, 0x00, 0xff, 0xff], // stored: LEN = 0, NLEN 互补
            &[0x01, 0xff, 0xff, 0x00, 0x00], // stored: LEN = 65535, NLEN 互补，但载荷缺失
            &[0x05, 0x00, 0x00, 0xff, 0xff], // 同上但 BFINAL = 1
            &[0xff; 64],
        ];
        for bytes in pathological {
            probe(bytes, usize::MAX);
            probe(bytes, 1 << 20);
        }

        println!("arbitrary_bytes_never_panic (inflate): runs={runs}");
        assert!(runs >= 85_000, "探针只跑了 {runs} 次，样本太少");
    }

    // -----------------------------------------------------------------------
    // 第二批判据（本票新增）：码长上界、dynamic 头的两个码数上界、码长重复的**恰好填满**
    // -----------------------------------------------------------------------

    /// 码长 **15**（RFC 1951 §3.2.5 的上界）必须被接受，且能解出符号。
    ///
    /// 注入实测（本票）：把 `index > MAX_CODE_BITS` 收成 `>=` 后全绿
    /// （既有判据的固定表最长 9 位，已提交的 dynamic 流也没用到 15 位）。
    #[test]
    fn a_fifteen_bit_huffman_code_is_accepted() {
        // 16 个符号的码长 1,2,…,14,15,15：Kraft 和恰好 = 1（完整的 canonical 表）。
        let mut lengths: Vec<u8> = (1..=14).collect();
        lengths.push(15);
        lengths.push(15);
        let table = Huffman::build(&lengths).expect("15 位码长在 RFC 1951 的范围内");
        // 全 1 的位流是最后（最长）那个符号的码。
        let mut bits = Bits::new(&[0xff, 0xff]);
        assert_eq!(table.decode(&mut bits), Ok(15));
    }

    /// dynamic 头的 `HLIT` 上界：码数 **287** 必须被拒绝并给出**字面**说明。
    ///
    /// 注入实测（本票）：把 `literal_count > 286` 放宽到 `> 287` 后全绿
    /// （既有判据的流都只声明 257..=286 个 literal/length 码）。
    #[test]
    fn a_dynamic_header_with_too_many_literal_codes_is_rejected() {
        let mut bits = BitWriter::new();
        bits.value(1, 1); // BFINAL = 1
        bits.value(2, 2); // BTYPE = 10（dynamic）
        bits.value(30, 5); // HLIT = 30 ⇒ literal_count = 287（上界是 286）
        bits.value(0, 5); // HDIST = 0 ⇒ distance_count = 1
        bits.value(0, 4); // HCLEN = 0 ⇒ 4 个码长
        let stream = bits.finish();
        let error = inflate_raw(&stream, 1 << 20).expect_err("HLIT 声明的码数超过 286");
        assert_eq!(error.detail, "literal/length 码数超过 286");
        assert_eq!(error.kind, InflateErrorKind::Malformed);
    }

    /// dynamic 头的 `HDIST` 上界：码数 **31** 必须被拒绝并给出**字面**说明。
    ///
    /// 注入实测（本票）：把 `distance_count > 30` 放宽到 `> 31` 后全绿。
    #[test]
    fn a_dynamic_header_with_too_many_distance_codes_is_rejected() {
        let mut bits = BitWriter::new();
        bits.value(1, 1); // BFINAL = 1
        bits.value(2, 2); // BTYPE = 10（dynamic）
        bits.value(0, 5); // HLIT = 0 ⇒ literal_count = 257（合法）
        bits.value(30, 5); // HDIST = 30 ⇒ distance_count = 31（上界是 30）
        bits.value(0, 4); // HCLEN = 0
        let stream = bits.finish();
        let error = inflate_raw(&stream, 1 << 20).expect_err("HDIST 声明的码数超过 30");
        assert_eq!(error.detail, "distance 码数超过 30");
        assert_eq!(error.kind, InflateErrorKind::Malformed);
    }

    /// 码长重复码 **16** 的**恰好填满**边界：`index + repeat == 表长` 必须接受
    /// （RFC 1951 §3.2.7 的重复只受"不越过表尾"约束）。
    ///
    /// 流（判据自己拼）：`HLIT=0` / `HDIST=0` ⇒ 表长 258；前面的 251 项用两个
    /// `18`（零重复）填掉，第 252 项写码长 8，末尾用 `16` 重复 **6** 次 ⇒ 正好 258。
    /// 注入实测（本票）：把这一处的 `>` 改成 `>=` 后全绿
    /// （既有判据的重复都没有落在表尾上）。
    #[test]
    fn a_code_length_repeat_that_exactly_fills_the_table_is_accepted() {
        let mut bits = BitWriter::new();
        bits.value(1, 1); // BFINAL = 1
        bits.value(2, 2); // BTYPE = 10（dynamic）
        bits.value(0, 5); // HLIT = 0 ⇒ literal_count = 257
        bits.value(0, 5); // HDIST = 0 ⇒ distance_count = 1
        bits.value(1, 4); // HCLEN = 1 ⇒ 5 个码长（顺序 = 16,17,18,0,8）
        // 码长表的码长：s16 = 3、s17 = 0、s18 = 1、s0 = 0、s8 = 2。
        for length in [3u32, 0, 1, 0, 2] {
            bits.value(length, 3);
        }
        // canonical 码：s18 = `0`（1 位）、s8 = `10`（2 位）、s16 = `110`（3 位）。
        bits.code(0, 1);
        bits.value(138 - 11, 7); // 18: 138 个 0
        bits.code(0, 1);
        bits.value(113 - 11, 7); // 18: 113 个 0 ⇒ 累计 251
        bits.code(2, 2); // s8 ⇒ lengths[251] = 8
        bits.code(6, 3);
        bits.value(6 - 3, 2); // 16: 重复 6 次 ⇒ 252..=257 = 8，正好填满 258
        // literal/length 表里符号 251..=256 的码长都是 8 ⇒ 块结束码 256 的 canonical 码 = 5。
        bits.code(5, 8);
        let stream = bits.finish();
        assert_eq!(
            inflate_raw(&stream, 1 << 20).as_deref(),
            Ok(&b""[..]),
            "恰好填满表尾的 16 重复必须被接受, 且块在块结束码处收束"
        );
    }

    /// 判据: dynamic 块的表头字段按 RFC 1951 §3.2.7 的**精确**上界拒绝。
    ///
    /// `HLIT` 是 5 位 ⇒ literal/length 码数 = `HLIT + 257` 落在 257..=288，而规范只允许
    /// 257..=286；`HDIST` 同样是 5 位 ⇒ distance 码数 = `HDIST + 1` 落在 1..=32，规范只
    /// 允许 1..=30。四个越界表头必须在读码长表**之前**被点名拒绝。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `literal_count > 286` 放宽成 `> 288`
    /// （注入 I12）后，**118** 条判据全绿 ⇒ 已提交语料的 `HLIT` 只到 29（= 286 个码），
    /// 两个上界当时都没有判据。
    #[test]
    fn dynamic_code_counts_above_the_rfc_limits_are_rejected() {
        /// 判据自己拼的表头：BFINAL=1、BTYPE=2、再三个定长字段（17 位 ⇒ 3 字节）。
        fn header(hlit: u32, hdist: u32) -> Vec<u8> {
            let mut bits = BitWriter::new();
            bits.value(1, 1); // BFINAL = 1
            bits.value(2, 2); // BTYPE = 10（dynamic；低位先出 ⇒ 先 0 后 1）
            bits.value(hlit, 5);
            bits.value(hdist, 5);
            bits.value(0, 4); // HCLEN = 0 ⇒ 只传 4 个码长码
            bits.finish()
        }
        for (hlit, hdist, detail) in [
            (30u32, 0u32, "literal/length 码数超过 286"), // 287 个码
            (31, 0, "literal/length 码数超过 286"),       // 288 个码
            (0, 30, "distance 码数超过 30"),              // 31 个码
            (0, 31, "distance 码数超过 30"),              // 32 个码
        ] {
            match inflate_raw(&header(hlit, hdist), 1 << 20) {
                Err(InflateError {
                    detail: got,
                    kind: InflateErrorKind::Malformed,
                    ..
                }) => assert_eq!(got, detail, "HLIT={hlit} HDIST={hdist} 的读数"),
                other => panic!("HLIT={hlit} HDIST={hdist} 必须被拒绝，得到 {other:?}"),
            }
        }
    }

    /// 判据: dynamic 块取**规范上界本身**（`HLIT = 29` ⇒ 286、`HDIST = 29` ⇒ 30）时不被
    /// 那两个上界检查拒绝，而是一路走到"缺失块结束码 256"这一步。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `lengths[256] == 0` 那一步去掉
    /// （注入 I7）后，**118** 条判据全绿 ⇒ 一个没有符号 256 的表当时没有判据
    /// （`arbitrary_bytes_never_panic` 只要求"不 panic"）。
    ///
    /// 量什么（单位 = 一个 `InflateError`）：表头后面每个码长都是 0 ⇒ 唯一可能的拒绝
    /// 就是"缺失块结束码 256"，因此这一条同时钉住 286 / 30 是**上界本身**（不越界）。
    #[test]
    fn a_dynamic_block_without_an_end_of_block_code_is_rejected() {
        let mut bits = BitWriter::new();
        bits.value(1, 1); // BFINAL = 1
        bits.value(2, 2); // BTYPE = dynamic
        bits.value(29, 5); // HLIT = 29 ⇒ 286 个 literal/length 码（恰是规范上界）
        bits.value(29, 5); // HDIST = 29 ⇒ 30 个 distance 码（恰是规范上界）
        bits.value(0, 4); // HCLEN = 0 ⇒ 传输顺序里的 4 个码长码
        // 码长码的长度按 RFC 1951 §3.2.7 的顺序传输：16, 17, 18, 0。
        bits.value(0, 3); // 符号 16 的长度 = 0
        bits.value(0, 3); // 符号 17 = 0
        bits.value(0, 3); // 符号 18 = 0
        bits.value(1, 3); // 符号 0 的长度 = 1 ⇒ 一位就能解出"码长 0"
        for _ in 0..(286 + 30) {
            bits.bit(0);
        }
        match inflate_raw(&bits.finish(), 1 << 20) {
            Err(InflateError {
                detail,
                kind: InflateErrorKind::Malformed,
                ..
            }) => assert_eq!(detail, "缺失块结束码 256"),
            other => panic!("没有块结束码的 dynamic 块必须被拒绝，得到 {other:?}"),
        }
    }

    /// 判据: 压缩块的**字面量**路径在输出上界处精确拒绝（多一个字节都不行）。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `codes` 里那句
    /// `out.len() >= max_output` 放宽成 `>`（注入 I14）后，**118** 条判据全绿
    /// ⇒ 字面量路径的上界当时只被"匹配路径"的上界间接掩护
    /// （`full_window_match_...` 的上界断言落在匹配那一步）。
    #[test]
    fn a_literal_run_cannot_exceed_the_output_limit() {
        /// 固定 Huffman 块：`count` 个字面量 `'a'` + 块结束码（BFINAL = 1）。
        fn literal_block(count: usize) -> Vec<u8> {
            let mut bits = BitWriter::new();
            bits.value(1, 1);
            bits.value(1, 2); // BTYPE = 01（固定 Huffman）
            for _ in 0..count {
                fixed_symbol(&mut bits, u32::from(b'a'));
            }
            fixed_symbol(&mut bits, 256);
            bits.finish()
        }
        // 对照臂：输出**恰好**等于上界 ⇒ 接受（上界数的是字节数，不是"最多能再放几个"）。
        assert_eq!(
            inflate_raw(&literal_block(4), 4).as_deref(),
            Ok(&b"aaaa"[..])
        );
        // 越界一个字节 ⇒ 在上界处报 Limit。
        match inflate_raw(&literal_block(5), 4) {
            Err(InflateError { detail, kind, .. }) => {
                assert_eq!(detail, "输出超过上界");
                assert_eq!(kind, InflateErrorKind::Limit);
            }
            other => panic!("越界一个字节必须报 Limit，得到 {other:?}"),
        }
    }

    /// 判据: `stored` 块的输出上界同样是"字节数"，载荷**恰好**等于上界时必须接受。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `stored` 里那句
    /// `out.len() + length > max_output` 改成 `>=`（注入 I15）后，**118** 条判据全绿
    /// ⇒ 既有的 `stored_block_respects_the_output_limit` 用的是 100 字节载荷配 99 的上界
    /// （差 1 字节**越界**），恰好相等的那一侧没有判据。
    #[test]
    fn a_stored_payload_exactly_equal_to_the_limit_is_accepted() {
        let stream = encode_stored(&[0u8; 100]);
        assert_eq!(
            inflate_raw(&stream, 100).as_deref(),
            Ok(&[0u8; 100][..]),
            "载荷恰好等于上界 ⇒ 必须接受"
        );
        match inflate_raw(&stream, 99) {
            Err(InflateError { detail, kind, .. }) => {
                assert_eq!(detail, "输出超过上界");
                assert_eq!(kind, InflateErrorKind::Limit);
            }
            other => panic!("少一个字节的上界必须报 Limit，得到 {other:?}"),
        }
    }

    /// 判据: Kraft 不等式的上界是**恰好完整**（`left == 0`）：超出一个码、且超出发生在
    /// 最后一层时必须拒绝。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `if left < 0` 放宽成 `if left < -1`
    /// （注入 I5）后，**118** 条判据全绿 —— 既有的
    /// `over_subscribed_huffman_table_is_rejected` 用 `[1, 1, 1]`，它在**第二层**就
    /// 让 `left` 变成 `-2` ⇒ 那个注入碰不到它。差别只在"最后一层恰好差半个码位"：
    /// 32769 个长度 15 的码 ⇒ Kraft 和 = 32769/32768。
    ///
    /// ⚠️ 这个形状**不是**从文件字节可达的（dynamic 块最多 286 + 30 = 316 个码长，
    /// 固定表是 288 / 30）⇒ 本条钉的是 `Huffman::build` 这个**内部函数**的契约，
    /// 不是容器层的输入。
    #[test]
    fn a_full_table_plus_one_code_at_the_last_level_is_rejected() {
        assert!(
            Huffman::build(&vec![15u8; 32768]).is_ok(),
            "32768 个长度 15 的码是恰好完整的表"
        );
        assert_eq!(
            Huffman::build(&vec![15u8; 32769]).err(),
            Some("Huffman 码被过度订阅"),
            "多一个码 ⇒ Kraft 和 > 1 ⇒ 必须拒绝"
        );
    }

    /// 判据: 压缩块的**匹配**（LZ77 回拷）路径在输出上界处同样是"字节数"：
    /// 一次匹配令输出**恰好**等于上界时必须接受。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `codes` 里匹配那一步的
    /// `out.len() + length > max_output` 改成 `>=`（注入 F10）后全部判据**保持绿**
    /// —— `a_literal_run_cannot_exceed_the_output_limit` 只走**字面量**那一步，
    /// `a_stored_payload_exactly_equal_to_the_limit_is_accepted` 走的是 `stored` 块，
    /// `full_window_match_...` 的上界断言是"少 1 字节必须拒绝" ⇒ 匹配路径上
    /// "**恰好**落在上界"那一侧没有判据。
    #[test]
    fn a_match_that_lands_exactly_on_the_output_limit_is_accepted() {
        // `fixed_stream(32768)`：1 个字面 + 127 次长度 258 的匹配 + 1 个字面 = 32768 字节，
        // 再放一次长度 **3** / 距离 32768 的匹配 ⇒ 输出恰好 **32771** 字节。
        let stream = fixed_stream(32768);
        assert_eq!(
            inflate_raw(&stream, 32771).map(|out| out.len()),
            Ok(32771),
            "匹配令输出恰好等于上界 ⇒ 必须接受（上界数的是字节数）"
        );
        match inflate_raw(&stream, 32770) {
            Err(InflateError { detail, kind, .. }) => {
                assert_eq!(detail, "输出超过上界");
                assert_eq!(kind, InflateErrorKind::Limit);
            }
            other => panic!("少一个字节的上界必须报 Limit，得到 {other:?}"),
        }
    }

    /// 判据: 输出上界横跨**整条流**（不是逐块重置）—— 两个 `stored` 块各 4 字节、
    /// 上界 6 时，第二块必须在上界处被拒。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `stored` 的 `out.len() + length > max_output`
    /// 换成 `length > max_output`（注入 E01，即把累计上界改成**逐块**上界）后全部判据
    /// **保持绿** —— 已提交的多块判据（`mxl_multiblock_deflate_stream_is_read_to_its_last_block`）
    /// 只证明"块链被读到最后"，它的上界给得很宽；而三条上界判据用的都是**单块**流
    /// ⇒ "上界跨块累计"这一步此前没有判据。
    #[test]
    fn the_output_limit_spans_every_block_of_a_multi_block_stream() {
        /// 两个 stored 块：第一个 `BFINAL=0`、第二个 `BFINAL=1`，载荷相同。
        fn two_stored_blocks(payload: &[u8]) -> Vec<u8> {
            let mut out = vec![0x00u8]; // BFINAL=0, BTYPE=00（低位先出）
            let length = payload.len() as u16;
            out.extend_from_slice(&length.to_le_bytes());
            out.extend_from_slice(&(!length).to_le_bytes());
            out.extend_from_slice(payload);
            out.push(0x01u8); // BFINAL=1, BTYPE=00
            out.extend_from_slice(&length.to_le_bytes());
            out.extend_from_slice(&(!length).to_le_bytes());
            out.extend_from_slice(payload);
            out
        }

        let stream = two_stored_blocks(b"abcd");
        assert_eq!(
            stream.len(),
            18,
            "两个 stored 块各 (1 位头 + 2 + 2 + 4) = 9 字节"
        );

        // 对照臂：上界恰好等于输出总长 ⇒ 接受。
        assert_eq!(
            inflate_raw(&stream, 8).as_deref(),
            Ok(&b"abcdabcd"[..]),
            "上界 8 = 8 字节输出 ⇒ 必须接受"
        );
        // 上界 6：第一块（4 字节）通过，第二块把它累计推过界 ⇒ 必须报 Limit。
        match inflate_raw(&stream, 6) {
            Err(InflateError { detail, kind, .. }) => {
                assert_eq!(detail, "输出超过上界");
                assert_eq!(kind, InflateErrorKind::Limit);
            }
            other => panic!("累计输出越过上界必须报 Limit，得到 {other:?}"),
        }
    }

    /// 判据 (类别③ 明确 Err / 错误文案): `InflateError` 的 `Display` 是**字面**读数
    /// （偏移 + 说明）。
    ///
    /// 补的是哪个缺口（本票注入实测）：把文案里的 `: ` 去掉（注入 S01）后全部判据
    /// **保持绿** ⇒ `InflateError` 的 `Display` 此前零判据（判据只比对
    /// `detail` / `kind` / `offset` 三个**字段**，从不读整句文案）。
    #[test]
    fn inflate_error_display_text_is_pinned() {
        let error = InflateError {
            offset: 12,
            detail: "块类型 3 未定义（RFC 1951 §3.2.3）",
            kind: InflateErrorKind::Malformed,
        };
        assert_eq!(
            error.to_string(),
            "偏移 12 处 DEFLATE 流非法: 块类型 3 未定义（RFC 1951 §3.2.3）"
        );

        // ⚠️ **登记（未被注入验证）**：`InflateError` 的 `Error::source()` 走 std 的
        // 默认实现 ⇒ 恒 `None`；没有可做字面替换的臂 ⇒ 本批没有能打它的注入。
        assert!(std::error::Error::source(&error).is_none());
    }

    /// 判据 (类别④ 参数极值 / 上界的**两个端点**): `max_output` 的 `0` 与 `usize::MAX`
    /// 都必须按**普通上界**处理 —— `0` 立刻拒绝，`usize::MAX` 不特判成拒绝。
    ///
    /// 补的是哪个缺口（本票注入实测）：把字面量路径的 `out.len() >= max_output` 放宽成
    /// `>`（注入 U01）、把 `stored` 的 `out.len() + length > max_output` 放宽成
    /// `> max_output.max(1)`（注入 U02）后全部判据**保持绿** —— 既有的三条上界判据用的
    /// 上界都 ≥ 4 ⇒ `0` 这个端点此前没有判据。
    #[test]
    fn the_output_limit_has_both_endpoints() {
        // `stored` 路径：上界 0 必须拒绝 1 字节。
        let stored = encode_stored(b"a");
        match inflate_raw(&stored, 0) {
            Err(InflateError { detail, kind, .. }) => {
                assert_eq!(detail, "输出超过上界");
                assert_eq!(kind, InflateErrorKind::Limit);
            }
            other => panic!("stored 路径在上界 0 处必须报 Limit，得到 {other:?}"),
        }

        // 字面量路径：同一个端点。
        let fixed = {
            let mut bits = BitWriter::new();
            bits.value(1, 1); // BFINAL = 1
            bits.value(1, 2); // BTYPE = 01（固定 Huffman）
            fixed_symbol(&mut bits, u32::from(b'a'));
            fixed_symbol(&mut bits, 256); // 块结束码
            bits.finish()
        };
        match inflate_raw(&fixed, 0) {
            Err(InflateError { detail, kind, .. }) => {
                assert_eq!(detail, "输出超过上界");
                assert_eq!(kind, InflateErrorKind::Limit);
            }
            other => panic!("字面量路径在上界 0 处必须报 Limit，得到 {other:?}"),
        }
        // 上界 1：恰好收下这 1 个字节。
        assert_eq!(inflate_raw(&fixed, 1).as_deref(), Ok(&b"a"[..]));
        // 另一个端点：`usize::MAX` 不是"无上界"的哨兵，但也不得被特判成拒绝。
        assert_eq!(
            inflate_raw(&fixed, usize::MAX).as_deref(),
            Ok(&b"a"[..]),
            "usize::MAX 必须按普通上界处理（⛔ 不是特判为 0）"
        );
    }

    /// 判据: `max_output` 恰好落在**中间块边界**上时，**下一块**必须在上界处被拒
    /// （⛔ 不是"上界在块边界上就放行"）。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `stored` 的上界改成 `max_output + out.len()`
    /// （注入 INB01）或把固定表的 `codes` 上界改成 `max_output + out.len()`
    /// （注入 INB02，即把**累计**上界改成**逐块**上界）后全部判据**保持绿** ——
    /// 第五批的 `the_output_limit_spans_every_block_of_a_multi_block_stream` 打的是
    /// "第二块越界"，第六批打的是**单块**的两端点；"上界**恰好**等于到某个中间块末尾的
    /// 累计字节数、而流还有下一块"这一步此前没有判据。
    #[test]
    fn a_limit_that_lands_on_a_block_boundary_still_rejects_the_next_block() {
        /// `count` 个连续 stored 块，每块 `payload.len()` 字节，只有最后一块 `BFINAL=1`。
        fn stored_chain(payload: &[u8], count: usize) -> Vec<u8> {
            let mut out = Vec::new();
            for index in 0..count {
                out.push(if index + 1 == count { 0x01u8 } else { 0x00u8 });
                let length = payload.len() as u16;
                out.extend_from_slice(&length.to_le_bytes());
                out.extend_from_slice(&(!length).to_le_bytes());
                out.extend_from_slice(payload);
            }
            out
        }
        /// 一个 `BFINAL` / 固定 Huffman（`BTYPE=01`）的块：`count` 个字面量 `'a'` + 块结束码。
        fn fixed_block(payload: &[u8], final_block: bool) -> Vec<u8> {
            let mut bits = BitWriter::new();
            bits.value(u32::from(final_block), 1);
            bits.value(1, 2);
            for _ in 0..payload.len() {
                fixed_symbol(&mut bits, u32::from(payload[0]));
            }
            fixed_symbol(&mut bits, 256);
            bits.finish()
        }

        let chain = stored_chain(b"abcd", 3);
        assert_eq!(chain.len(), 27, "3 × (1 位头 + 2 LEN + 2 NLEN + 4 字节)");
        // 上界 8 = 恰好两块 ⇒ 第三块必须被拒。
        match inflate_raw(&chain, 8) {
            Err(InflateError { detail, kind, .. }) => {
                assert_eq!(detail, "输出超过上界");
                assert_eq!(kind, InflateErrorKind::Limit);
            }
            other => panic!("上界 8 恰好落在块边界上 ⇒ 第三块必须报 Limit，得到 {other:?}"),
        }
        // 上界 12 = 整条流 ⇒ 接受。
        assert_eq!(inflate_raw(&chain, 12).as_deref(), Ok(&b"abcdabcdabcd"[..]));

        // 第二条：**第一块是 stored、第二块是压缩块**（固定 Huffman）⇒ 压缩块那一步
        // 看到的是 `out.len() = 4`（不是 0），"逐块上界"与"累计上界"在这里分岔。
        let mut mixed = vec![0x00u8]; // BFINAL=0, BTYPE=00（stored）
        let length = 4u16;
        mixed.extend_from_slice(&length.to_le_bytes());
        mixed.extend_from_slice(&(!length).to_le_bytes());
        mixed.extend_from_slice(b"abcd");
        let fixed = fixed_block(b"aaaa", true);
        mixed.extend_from_slice(&fixed);
        assert_eq!(mixed.len(), 9 + fixed.len(), "stored 9 字节 + 固定块");

        // 上界 6：stored 收 4 字节，压缩块的第 3 个字面量越界 ⇒ Limit。
        match inflate_raw(&mixed, 6) {
            Err(InflateError { detail, kind, .. }) => {
                assert_eq!(detail, "输出超过上界");
                assert_eq!(kind, InflateErrorKind::Limit);
            }
            other => panic!("压缩块中途越界必须报 Limit，得到 {other:?}"),
        }
        // 上界 4 = 恰好第一块 ⇒ 压缩块一个字面量都不许写。
        match inflate_raw(&mixed, 4) {
            Err(InflateError { detail, kind, .. }) => {
                assert_eq!(detail, "输出超过上界");
                assert_eq!(kind, InflateErrorKind::Limit);
            }
            other => panic!("上界 4 时压缩块必须立刻报 Limit，得到 {other:?}"),
        }
        // 上界 8 = 整条流 ⇒ 接受。
        assert_eq!(inflate_raw(&mixed, 8).as_deref(), Ok(&b"abcdaaaa"[..]));
    }

    /// 判据: 长度码 **286 / 287** 未定义（RFC 1951 §3.2.5）⇒ 明确的 `Malformed`
    /// （⛔ 不是静默当成块结束码）。
    ///
    /// 补的是哪个缺口（本票注入实测）：把那个 `let … else` 的分支从
    /// `return Err("长度码 286/287 未定义")` 改成 `return Ok(())`（注入 `b14:HLIT286`）后
    /// 全部判据**保持绿** —— 既有判据只覆盖了 `HLIT` 的**码数**上界
    /// （`a_dynamic_header_with_too_many_literal_codes_is_rejected`），
    /// ⛔ 没有覆盖"码**值** 286/287 出现在数据里"这一半。
    #[test]
    fn the_undefined_length_codes_are_rejected() {
        /// 固定 Huffman 块：`BFINAL=1`、`BTYPE=01`，正文只有一个 `symbol`。
        fn only_symbol(symbol: u32) -> Vec<u8> {
            let mut bits = BitWriter::new();
            bits.value(1, 1); // BFINAL = 1
            bits.value(1, 2); // BTYPE = 01（固定 Huffman）
            fixed_symbol(&mut bits, symbol);
            bits.finish()
        }

        for symbol in [286u32, 287] {
            match inflate_raw(&only_symbol(symbol), 1024) {
                Err(InflateError { detail, kind, .. }) => {
                    assert_eq!(detail, "长度码 286/287 未定义（RFC 1951 §3.2.5）");
                    assert_eq!(kind, InflateErrorKind::Malformed);
                }
                other => panic!("长度码 {symbol} 必须被拒绝，得到 {other:?}"),
            }
        }

        // 对照臂：**285** 是合法的最大长度码（258 字节）⇒ 不能顺手把它也算作未定义；
        // 它之后缺少距离码，报的必须是**另一个**错。
        match inflate_raw(&only_symbol(285), 1024) {
            Err(InflateError { detail, .. }) => assert_ne!(
                detail, "长度码 286/287 未定义（RFC 1951 §3.2.5）",
                "285 是合法长度码，⛔ 不许被这一支拒掉"
            ),
            other => panic!("285 之后缺距离码应当是另一个错误，得到 {other:?}"),
        }
    }

    /// 判据 (类别: 测试助手的**上界守卫**): `fixed_symbol` 只定义到 **287**
    /// （RFC 1951 §3.2.6 的固定表是 0..=287）⇒ **288 及以上必须立刻 panic**。
    ///
    /// 补的是哪个缺口（注入实测）：改前 `_ => bits.code(0xc0 + (symbol - 280), 8)`
    /// 覆盖到 `u32::MAX` ⇒ `fixed_symbol(b, 288)` 发出的是 symbol **280** 的码 ⇒
    /// ⭐ **用该助手写的判据会静默测错对象**（这正是本助手最危险的用法）。
    /// ⚠️ 本判据自身是 `#[should_panic]`，其"已知红"＝**去掉那句 panic**
    /// （此时本判据会因为"没有 panic"而变红）。
    #[test]
    #[should_panic(expected = "fixed_symbol 只定义到 287")]
    fn fixed_symbol_rejects_symbols_above_the_fixed_table() {
        let mut bits = BitWriter::new();
        fixed_symbol(&mut bits, 288);
    }

    /// 判据 (R112 正对照): 上表的**最后一个**合法符号 **287** 必须**照常接受**
    /// （⛔ 不许把守卫写成"把 280..=287 也一起拒掉"）。
    #[test]
    fn fixed_symbol_accepts_the_last_defined_symbol() {
        for symbol in [280u32, 287] {
            let mut bits = BitWriter::new();
            fixed_symbol(&mut bits, symbol);
            assert!(!bits.finish().is_empty(), "符号 {symbol} 必须被编码");
        }
    }

    /// **编译期穷举探针**（R177）: 给 `InflateErrorKind` 的每个变体一个唯一编号 ⇒
    /// **新增一个变体**就让下面这个 `match` 非穷尽、**编译失败**。
    ///
    /// ⚠️ 为什么需要它：`InflateError` 的 `kind` 字段此前**没有**臂探针
    /// （现场查证：`grep -rn 'InflateErrorKind' crates/yeban-midi/src/mxl/inflate.rs` 只见
    /// 定义与构造点；`grep -rn 'fn inflate.*arm'` = 0）⇒ 给这个 2 变体公开枚举**加一个变体**
    /// 当时不会被任何判据机械抓到。
    /// ⛔ **不许给这个 `match` 加 `_ =>` 通配臂**（R51）：加了以后新增变体也能编译过，
    /// 探针立刻**静默失效**，而**所有判据仍然全绿**。
    fn inflate_error_kind_arm(kind: InflateErrorKind) -> u8 {
        match kind {
            InflateErrorKind::Malformed => 0,
            InflateErrorKind::Limit => 1,
        }
    }

    /// 判据 (R160/R177 **双向归零**): `InflateErrorKind` 的臂码集合必须**恰好** `0..2`。
    ///
    /// - **少一条**（某个变体没有臂）⇒ 编号集合不完整 ⇒ 红；
    /// - **重复一条**（两个变体共用编号）⇒ 集合有多余 ⇒ 红。
    ///
    /// ⭐ R120／R174 **根绑定**：下界**根绑定到这张全表自己**（去重后的长度必须等于
    /// 变体数组的长度），⛔ 不是"借用邻居"的界 —— 借来的界会随窗口大小来回摆动。
    #[test]
    fn every_inflate_error_kind_arm_is_covered() {
        let all = [InflateErrorKind::Malformed, InflateErrorKind::Limit];
        let mut arms: Vec<u8> = all.iter().copied().map(inflate_error_kind_arm).collect();
        arms.sort_unstable();
        let mut distinct = arms.clone();
        distinct.dedup();
        // ⭐ R180：自检**不用集合大小界**（`distinct.len() == all.len()` 是"借来的界"），
        // 改成与**字面值集合**比较 ⇒ 界根绑定到"这两个变体应有的编号"本身。
        assert_eq!(
            distinct,
            vec![0u8, 1u8],
            "两个变体不许共用同一个编号（重复臂 ⇒ 集合有多余）"
        );
        assert_eq!(
            arms,
            (0..2).collect::<Vec<u8>>(),
            "臂码必须恰好 `0..2`（缺一臂 ⇒ 集合不完整）"
        );
    }

    /// 判据 (头号项② **选项② —— 如实登记"`kind` 不在 Display 里"**):
    /// 两个**只差 `kind`** 的 `InflateError` 必须产生**完全相同**的 `Display` 文案。
    ///
    /// ⭐ **登记读数**，⛔ 不是缺陷断言：`Display` 的模板是
    /// `"偏移 {} 处 DEFLATE 流非法: {}"`（只吃 `offset` 与 `detail`）⇒ `kind` 不在文案里。
    /// ⭐ **R189 机械形态**：把"不可见"写成**可判定的等价断言**（只差 `kind` ⇒ 文案相等），
    /// 并配**配对对照**（只差 `detail` ⇒ 文案必须**不同**；否则上面的相等是平凡的）。
    /// ⭐ **已知红 ＝ 把 `kind` 加进 `Display` 模板**（外部注入；按 **R187** 打 `SELFTEST` 标记）。
    /// ⭐ **R188**：规模/成本面读数降级为 `eprintln!`（诊断），⛔ 不当自检判据。
    #[test]
    fn two_inflate_errors_differing_only_in_kind_have_the_same_display_text() {
        let malformed = InflateError {
            offset: 7,
            detail: "同一个说明",
            kind: InflateErrorKind::Malformed,
        };
        let limit = InflateError {
            offset: 7,
            detail: "同一个说明",
            kind: InflateErrorKind::Limit,
        };
        assert_eq!(
            malformed.to_string(),
            limit.to_string(),
            "`kind` 不在 `Display` 模板里 ⇒ 只差 kind 的两条错误文案必须逐字相同"
        );

        // ⭐ 配对对照（R189）：只差 `detail` 的两条 ⇒ 文案必须**不同**。
        let other_detail = InflateError {
            offset: 7,
            detail: "另一个说明",
            kind: InflateErrorKind::Malformed,
        };
        assert_ne!(
            malformed.to_string(),
            other_detail.to_string(),
            "配对对照：只差 detail 的两条文案必须不同（否则上面的相等是平凡的）"
        );

        // ⭐ R188：规模/成本面只做**诊断**（⛔ 不用集合大小界当判据）。
        eprintln!(
            "DIAGNOSTIC two_inflate_errors_differing_only_in_kind: display_len={} template_fields=offset+detail",
            malformed.to_string().len()
        );
    }

    /// 判据 (R193/R196 **常驻绊线**): 两处判据**观测同一个产线面**（`InflateError::Display`）——
    /// 既有 `inflate_error_display_text_is_pinned`（字面文案）与
    /// `two_inflate_errors_differing_only_in_kind_have_the_same_display_text`（只差 kind 相等）。
    /// 本条把**两处观测绑在一起**：它独立复述**登记下来的模板**，再与本判据直接读到的
    /// `to_string()` 比对。
    ///
    /// ⚠️ **用法（R193：⛔ 不许删断言、⛔ 不许改判据、⛔ 不许当失败跳过）**：
    /// 本判据**变红** ⇒ **产线的 `Display` 模板被改过**（`b21:SELFTEST-KINDVISIBLE` 那次注入
    /// 正是这种改动）⇒ 必须 **① 重跑那条外部注入样本 ② 按新实现同步更新登记**
    /// （字面判据与选项②判据的期望值都要改）。
    /// ⭐ R110/R126：这条绊线**故意不是**"自比"——它复述的是**登记值**，所以实现一改就红。
    #[test]
    fn the_two_display_observation_paths_agree() {
        eprintln!(
            "[R187-PROBE b25:mxl::inflate::tests::the_two_display_observation_paths_agree] ran"
        );
        let offset = 12usize;
        let detail = "块类型 3 未定义（RFC 1951 §3.2.3）";
        let error = InflateError {
            offset,
            detail,
            kind: InflateErrorKind::Malformed,
        };
        // 观测路径 A：**登记下来的模板**（与 `inflate_error_display_text_is_pinned` 同一期望值）。
        let registered = format!("偏移 {offset} 处 DEFLATE 流非法: {detail}");
        // 观测路径 B：直接读产线的 `Display`。
        let observed = error.to_string();
        assert_eq!(
            observed, registered,
            "两处观测必须一致；变红 ⇒ 产线 Display 改了 ⇒ 重跑 b21:SELFTEST-KINDVISIBLE 并更新登记"
        );
        assert_ne!(
            observed,
            format!(
                "偏移 {offset} 处 DEFLATE 流非法: {detail}（{kind:?}）",
                kind = error.kind
            ),
            "配对对照：若模板**带上** kind，上面那条就不再成立（⛔ 不是平凡相等）"
        );
        // ⭐ R188：规模只做诊断，⛔ 不当自检判据。
        eprintln!(
            "DIAGNOSTIC tripwire: observed_len={} registered_len={}",
            observed.len(),
            registered.len()
        );
    }
}
