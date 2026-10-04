//! 零依赖 PNG 编码器（只用 **stored deflate**）—— `[UI-MCP-003]` Golden 图的落盘格式。
//!
//! ## 为什么自己写而不加 `png` / `image` crate（二选一的理由）
//!
//! 任务允许"自己写一个只用 stored-deflate 的零依赖 PNG 写入器"或"加 `png` crate"。
//! 这里选**自己写**，理由是三条**可核验**的工程后果，不是"少个依赖更优雅"：
//!
//! 1. **不让 Golden 的产出依赖一个我们无法判定的压缩器**。CI 的判据是"像素一致"，
//!    不是"压缩率一致"；stored deflate 的字节是**完全确定**的（同一输入 ⇒ 同一 PNG 字节，
//!    无 zlib 版本/CPU/优化级别变量），因此 PNG 的 SHA-256 可以直接进断言。
//!    `png` crate 走的是系统/内置 zlib，压缩字节随版本变化 —— 那会让"文件哈希"这种最便宜的
//!    判据失效。
//! 2. **零新增许可与零依赖图漂移**。引入 `png`（+ `flate2`/`miniz_oxide`）要重新走
//!    `license_inventory.py` + `cargo-deny`；本线已经因为引入 `slint` 而必须重生成一次
//!    依赖清单，没有理由再加一层。
//! 3. **可被独立解码器验证**：stored deflate 的 IDAT 可以被标准 zlib `inflate` 解开，
//!    所以"我们写的 PNG 是合法 PNG"这条判据可以**在本机**用 `python3 zlib` 真跑，
//!    不必等 CI（见 `docs/ledger/ui-test-port-notes.md` §本机判据）。
//!
//! ## 代价（必须说清，不许静默）
//!
//! stored deflate **不做熵编码**，所以文件体积 ≈ `宽 × 高 × 3 + 开销`
//! （1080p ≈ 6.2 MB）。仓库红线（`AGENTS.md` §2 红线 9 / 守卫 G06）禁止提交 >10MB 的单文件，
//! 因此：
//! - [`encode_rgb8_limited`] 让"Golden 超过自设上限"变成**显式错误**，而不是某天被 G06 拦下；
//! - 真正要**提交进仓库**的 Golden 应当是小尺寸截图（例如 960×540 ≈ 1.5 MB）或改用压缩器产出，
//!   这一点已登记为 needs（见 notes）。
//!
//! 规范来源 (Normative)：
//! - `[UI-MCP-003]` UI/UX §12.5 —— Framebuffer 编码为 PNG 二进制流；
//! - `[MUST-GATE-015]` 路线图 §5 —— Golden 尺寸非零且非全黑。
//!
//! 格式依据（逐条核验过，不是凭记忆）：
//! - PNG 规范 RFC 2083 / W3C PNG (Second Edition)：签名、IHDR/IDAT/IEND 块结构、
//!   chunk = `长度(BE) + 类型 + 数据 + CRC32(类型+数据)`，truecolor 8-bit 为 color type 2，
//!   filter type 0 为 None <https://www.w3.org/TR/PNG/>；
//! - DEFLATE 的 stored（未压缩）块：RFC 1951 §3.2.4，块头 3 bit（BFINAL + BTYPE=00）
//!   补齐到字节边界，随后 `LEN`/`NLEN` 小端 16 位，`LEN ≤ 65535`
//!   <https://www.rfc-editor.org/rfc/rfc1951#section-3.2.4>；
//! - zlib 容器：RFC 1950，2 字节头 + deflate 数据 + 4 字节大端 Adler-32
//!   <https://www.rfc-editor.org/rfc/rfc1950>。

use crate::image::{ImageError, Rgb8Image};

/// PNG 文件签名（8 字节）。
pub const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// 仓库对单文件的硬上限（`AGENTS.md` §2 红线 9 与守卫 G06 都是 10 MB）。
pub const REPO_MAX_FILE_BYTES: usize = 10 * 1024 * 1024;

/// 编码错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngError {
    /// 图像构造阶段就失败（例如空缓冲长度不符）。
    Image(ImageError),
    /// 编码结果超过调用方给的上限 —— 拒绝静默产出可能触发 G06 的文件。
    TooLarge {
        /// 编码后字节数。
        encoded: usize,
        /// 调用方允许的上限。
        limit: usize,
    },
}

impl core::fmt::Display for PngError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Image(err) => write!(f, "PNG 编码输入非法: {err}"),
            Self::TooLarge { encoded, limit } => write!(
                f,
                "PNG 编码结果 {encoded} 字节超过上限 {limit} 字节 —— stored deflate 不做压缩, \
                 大尺寸 Golden 请改用有压缩器的产出路径 (AGENTS.md §2 红线 9 / 守卫 G06)"
            ),
        }
    }
}

impl core::error::Error for PngError {}

impl From<ImageError> for PngError {
    fn from(value: ImageError) -> Self {
        Self::Image(value)
    }
}

/// CRC-32（IEEE 802.3 / CRC-32/ISO-HDLC，反射多项式 `0xEDB88320`），用于 PNG 每个块的校验。
///
/// 逐位实现（不预生成 256 项表）：Golden 编码是每个用例一次的冷路径，一张表换来的
/// 常数级提速在这里没有意义，而少一份静态数据就少一份被误改的可能。
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Adler-32（RFC 1950），zlib 容器尾部的校验和。
#[must_use]
pub fn adler32(bytes: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    let mut a = 1u32;
    let mut b = 0u32;
    for byte in bytes {
        a = (a + u32::from(*byte)) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

/// 把 RGB8 图像编码为 PNG（stored deflate，无压缩）。
#[must_use]
pub fn encode_rgb8(image: &Rgb8Image) -> Vec<u8> {
    let raw = filtered_scanlines(image);
    let mut out = Vec::with_capacity(raw.len() + raw.len() / 65_535 * 5 + 128);
    out.extend_from_slice(&PNG_SIGNATURE);

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&image.width().to_be_bytes());
    ihdr.extend_from_slice(&image.height().to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(2); // color type: truecolor (RGB)
    ihdr.push(0); // compression method: deflate
    ihdr.push(0); // filter method: adaptive (但我们只用 filter 0)
    ihdr.push(0); // interlace: none
    write_chunk(&mut out, b"IHDR", &ihdr);

    write_chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    write_chunk(&mut out, b"IEND", &[]);
    out
}

/// 编码并施加体积上限。超过 `limit` 时返回 [`PngError::TooLarge`]，不产出文件。
pub fn encode_rgb8_limited(image: &Rgb8Image, limit: usize) -> Result<Vec<u8>, PngError> {
    let encoded = encode_rgb8(image);
    if encoded.len() > limit {
        return Err(PngError::TooLarge {
            encoded: encoded.len(),
            limit,
        });
    }
    Ok(encoded)
}

/// 编码后的**精确**字节数（不真正编码），供调用方在截图前就决定要不要落盘。
///
/// 推导：签名 8 + IHDR(4+4+13+4) + IDAT(4+4+`idat`+4) + IEND(4+4+0+4)，
/// 其中 `idat = 2(zlib 头) + 5*块数 + 原始字节 + 4(Adler-32)`，合计 `原始字节 + 5*块数 + 63`。
#[must_use]
pub fn encoded_len(image: &Rgb8Image) -> usize {
    let raw = filtered_len(image);
    raw + raw.div_ceil(65_535) * 5 + 63
}

/// 加了 filter byte 之后的原始数据长度。
fn filtered_len(image: &Rgb8Image) -> usize {
    (image.stride() + 1) * image.height() as usize
}

/// 逐行加 filter byte 0（None）后的原始数据。
fn filtered_scanlines(image: &Rgb8Image) -> Vec<u8> {
    let stride = image.stride();
    let height = image.height() as usize;
    let mut raw = Vec::with_capacity(stride * height + height);
    for line in image.pixels().chunks_exact(stride) {
        raw.push(0);
        raw.extend_from_slice(line);
    }
    raw
}

/// 把原始数据包成 zlib 流（CMF/FLG + stored deflate 块 + Adler-32）。
fn zlib_stored(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + raw.len() / 65_535 * 5 + 6);
    // CMF: CM=8(deflate), CINFO=7(32K window) -> 0x78
    // FLG: FCHECK 使 (CMF<<8|FLG) % 31 == 0, FDICT=0, FLEVEL=0 -> 0x01
    out.push(0x78);
    out.push(0x01);

    let mut chunks = raw.chunks(65_535).peekable();
    if raw.is_empty() {
        // 空数据也要有一个 final 空块，否则解压器读不到 BFINAL。
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xff, 0xff]);
    }
    while let Some(chunk) = chunks.next() {
        let final_block = chunks.peek().is_none();
        out.push(if final_block { 0x01 } else { 0x00 });
        let len = u16::try_from(chunk.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&adler32(raw).to_be_bytes());
    out
}

/// 写一个 PNG 块：长度(BE) + 类型 + 数据 + CRC32(类型+数据)。
fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    let len = u32::try_from(data.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::{MASK_COLOR, Rect, Size};

    /// 判据 1: 校验和的**已知向量**（不是自己算自己）。
    ///
    /// 出处：CRC-32/ISO-HDLC 的标准测试向量 `"123456789" -> 0xCBF43926`
    /// <https://reveng.sourceforge.io/crc-catalogue/17plus.htm#crc.cat.crc-32iso-hdlc>；
    /// Adler-32 的标准向量 `"Wikipedia" -> 0x11E60398` <https://en.wikipedia.org/wiki/Adler-32>。
    #[test]
    fn checksums_match_published_test_vectors() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(adler32(b""), 1);
    }

    /// 判据 2: 产出的字节必须是一份**能被独立解码器解开**的 PNG。
    ///
    /// 这里在测试内实现一个最小解码器（解析块 + CRC 校验 + stored-deflate 还原 + Adler-32 校验），
    /// 逐像素比对。真实世界里的第三方解码验证（`python3 zlib`）记在 notes 的本机判据里。
    #[test]
    fn encoded_png_round_trips_through_minimal_decoder() {
        let mut image = Rgb8Image::new(Size::new(37, 11));
        image.fill([0x11, 0x22, 0x33]);
        image.fill_rect(Rect::new(4, 2, 6, 5), [0xff, 0x88, 0x00]);
        image.set_pixel(0, 0, MASK_COLOR);
        image.set_pixel(36, 10, [0x7f, 0x7f, 0x7f]);

        let png = encode_rgb8(&image);
        assert_eq!(&png[..8], &PNG_SIGNATURE);
        let decoded = decode_png(&png).expect("自产 PNG 必须能被最小解码器解开");
        assert_eq!(decoded.size(), image.size());
        assert_eq!(decoded.pixels(), image.pixels());
    }

    /// 判据 3: 高于 65535 的输入会跨多个 stored 块 —— 边界必须正确拼接。
    #[test]
    fn multi_block_deflate_covers_the_65535_boundary() {
        // 宽度取 397 -> 行字节 1191; 高 120 -> 142_920 字节 > 65535, 跨 3 个块。
        let mut image = Rgb8Image::new(Size::new(397, 120));
        image.fill([0x01, 0x02, 0x03]);
        image.set_pixel(396, 119, [0xaa, 0xbb, 0xcc]);
        let png = encode_rgb8(&image);
        let decoded = decode_png(&png).expect("跨块 PNG 必须能被解开");
        assert_eq!(decoded.pixels(), image.pixels());
        assert!(decoded.pixel(396, 119) == Some([0xaa, 0xbb, 0xcc]));
    }

    /// 判据 4: 同一图像两次编码必须**逐字节相同**（Golden 的 SHA-256 才可能是判据）。
    #[test]
    fn encoding_is_byte_deterministic() {
        let mut image = Rgb8Image::new(Size::new(64, 48));
        image.fill([9, 9, 9]);
        image.fill_rect(Rect::new(1, 1, 30, 30), [200, 100, 50]);
        let first = encode_rgb8(&image);
        let second = encode_rgb8(&image);
        assert_eq!(first, second);
        assert_eq!(
            encoded_len(&image),
            first.len(),
            "encoded_len 必须与实际长度一致"
        );
    }

    /// 判据 5: 超过上限时**报错而不是产出**（红线 9 / 守卫 G06 的机械保护）。
    #[test]
    fn size_limit_is_enforced_before_writing() {
        let image = Rgb8Image::new(Size::new(64, 64));
        let ok = encode_rgb8_limited(&image, 64 * 64 * 3 + 4096).expect("上限足够时应当成功");
        assert!(!ok.is_empty());
        let err = encode_rgb8_limited(&image, 1024).expect_err("上限不足时应当报错");
        assert_eq!(
            err,
            PngError::TooLarge {
                encoded: ok.len(),
                limit: 1024
            }
        );
    }

    /// 测试用的最小 PNG 解码器：只接受本模块产出的 stored-deflate 形式。
    fn decode_png(bytes: &[u8]) -> Result<Rgb8Image, String> {
        if bytes.len() < 8 || bytes[..8] != PNG_SIGNATURE {
            return Err("签名错误".into());
        }
        let mut offset = 8;
        let mut width = 0u32;
        let mut height = 0u32;
        let mut idat = Vec::new();
        while offset + 8 <= bytes.len() {
            let len =
                u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap_or([0; 4])) as usize;
            let kind = &bytes[offset + 4..offset + 8];
            let data = bytes
                .get(offset + 8..offset + 8 + len)
                .ok_or_else(|| "块数据越界".to_owned())?;
            let crc_at = offset + 8 + len;
            let stored_crc = u32::from_be_bytes(
                bytes
                    .get(crc_at..crc_at + 4)
                    .ok_or("缺少 CRC")?
                    .try_into()
                    .unwrap_or([0; 4]),
            );
            let mut crc_input = Vec::new();
            crc_input.extend_from_slice(kind);
            crc_input.extend_from_slice(data);
            if crc32(&crc_input) != stored_crc {
                return Err(format!(
                    "块 {:?} 的 CRC 不符",
                    String::from_utf8_lossy(kind)
                ));
            }
            match kind {
                b"IHDR" => {
                    if data.len() != 13 || data[8] != 8 || data[9] != 2 {
                        return Err("IHDR 仅支持 8bit truecolor".into());
                    }
                    width = u32::from_be_bytes(data[0..4].try_into().unwrap_or([0; 4]));
                    height = u32::from_be_bytes(data[4..8].try_into().unwrap_or([0; 4]));
                }
                b"IDAT" => idat.extend_from_slice(data),
                b"IEND" => break,
                _ => {}
            }
            offset = crc_at + 4;
        }
        let (raw, consumed) = inflate_stored(&idat)?;
        if consumed != idat.len() {
            return Err("zlib 流有多余尾字节".into());
        }
        let stride = width as usize * 3;
        if raw.len() != (stride + 1) * height as usize {
            return Err("解压后的原始数据长度不符".into());
        }
        let mut pixels = Vec::with_capacity(stride * height as usize);
        for line in raw.chunks_exact(stride + 1) {
            if line[0] != 0 {
                return Err(format!("只支持 filter 0, 遇到 {}", line[0]));
            }
            pixels.extend_from_slice(&line[1..]);
        }
        Rgb8Image::from_raw(Size::new(width, height), pixels).map_err(|err| err.to_string())
    }

    /// 还原 stored-deflate 的 zlib 流，返回（原始数据, 消耗的输入字节数）。
    fn inflate_stored(zlib_stream: &[u8]) -> Result<(Vec<u8>, usize), String> {
        if zlib_stream.len() < 6 {
            return Err("zlib 流过短".into());
        }
        let flg = u16::from(zlib_stream[1]);
        if (u16::from(zlib_stream[0]) * 256 + flg) % 31 != 0 {
            return Err("zlib 头校验位错误".into());
        }
        let body = &zlib_stream[2..zlib_stream.len() - 4];
        let expected_adler = u32::from_be_bytes(
            zlib_stream[zlib_stream.len() - 4..]
                .try_into()
                .unwrap_or([0; 4]),
        );

        let mut out = Vec::new();
        let mut pos = 0usize;
        loop {
            let header = *body.get(pos).ok_or("deflate 块头越界")?;
            pos += 1;
            if header & 0b110 != 0 {
                return Err("只支持 stored (BTYPE=00) 块".into());
            }
            let len = u16::from_le_bytes(
                body.get(pos..pos + 2)
                    .ok_or("LEN 越界")?
                    .try_into()
                    .unwrap_or([0; 2]),
            );
            let nlen = u16::from_le_bytes(
                body.get(pos + 2..pos + 4)
                    .ok_or("NLEN 越界")?
                    .try_into()
                    .unwrap_or([0; 2]),
            );
            pos += 4;
            if nlen != !len {
                return Err("NLEN 与 LEN 不互补".into());
            }
            let data = body.get(pos..pos + len as usize).ok_or("块数据越界")?;
            out.extend_from_slice(data);
            pos += len as usize;
            if header & 1 == 1 {
                break;
            }
        }
        if adler32(&out) != expected_adler {
            return Err("Adler-32 不符".into());
        }
        Ok((out, 2 + pos + 4))
    }
}
