//! CRC-32 (IEEE 802.3 / PKZIP) 的手写实现 —— 零第三方依赖、无 `unsafe`。
//!
//! # 为什么自己写
//!
//! `.yeban` 容器的解包路径是**不可信输入**的解析边界，而引入 `crc32fast` 会让
//! `yeban-model` 多出一个第三方依赖（见本线裁决：不引入 `zip` / `flate2`，
//! 也不为 CRC 单独引依赖）。24 行查表算法换来的是：这条线的全部判据可以在本机
//! `bash scripts/gates/run-gates.sh crate yeban-model` 真跑（而不是被判定为"重依赖"交给 CI）。
//!
//! # 算法与标准
//!
//! PKZIP APPNOTE 4.4.5 规定 ZIP 条目校验使用 CRC-32/ISO-HDLC（IEEE 802.3）：
//! 反射多项式 `0x04C11DB7` 的位反转形式 `0xEDB88320`、初始值 `0xFFFF_FFFF`、
//! 输入/输出均反射、结果异或 `0xFFFF_FFFF`。标准测试向量在文件末的单元测试里逐条钉住。

/// 反射多项式：`0x04C11DB7` 的位反转形式。
const POLY: u32 = 0xEDB8_8320;

/// 编译期生成 256 项查表。
///
/// 用 `const fn` 而不是 `OnceLock` / `lazy_static`：编译期求值 ⇒ 运行期零初始化、
/// 零分配、零锁，也不需要在解析路径上关心"表是否已初始化"。
const fn build_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ POLY
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

/// CRC-32 查表（编译期常量）。
static TABLE: [u32; 256] = build_table();

/// 生成 slice-by-8 的第 `n` 张辅助表：`TABLEn[i]` 是把字节 `i`
/// 推入寄存器后再推 `n` 个零字节（`crc = (crc >> 8) ^ TABLE[crc & 0xFF]`）
/// 得到的寄存器值。用 `const fn` 求值 ⇒ 运行期零初始化、零锁。
const fn build_table_n(n: usize) -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = TABLE[i];
        let mut step = 0;
        while step < n {
            crc = (crc >> 8) ^ TABLE[(crc & 0xFF) as usize];
            step += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

static TABLE1: [u32; 256] = build_table_n(1);
static TABLE2: [u32; 256] = build_table_n(2);
static TABLE3: [u32; 256] = build_table_n(3);
static TABLE4: [u32; 256] = build_table_n(4);
static TABLE5: [u32; 256] = build_table_n(5);
static TABLE6: [u32; 256] = build_table_n(6);
static TABLE7: [u32; 256] = build_table_n(7);

/// 计算 `bytes` 的 CRC-32/ISO-HDLC 摘要。
///
/// 纯函数、无分配、无 `unsafe`、可跨平台位级一致（无浮点、无端序依赖）。
/// 主循环用 slice-by-8 一次推进 8 字节，尾部不足 8 字节的部分走原逐字节循环。
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    // workspace MSRV 是 1.92，`slice::as_chunks` 太新不可用；`chunks_exact(8)`
    // 的等价写法，故此处按仓库惯例局部放行该 lint（见 music.rs 的同类 allow）。
    #[allow(clippy::chunks_exact_to_as_chunks)]
    let mut chunks = bytes.chunks_exact(8);
    for chunk in &mut chunks {
        let lo = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        let hi = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
        crc ^= lo;
        crc = TABLE7[(crc & 0xFF) as usize]
            ^ TABLE6[((crc >> 8) & 0xFF) as usize]
            ^ TABLE5[((crc >> 16) & 0xFF) as usize]
            ^ TABLE4[((crc >> 24) & 0xFF) as usize]
            ^ TABLE3[(hi & 0xFF) as usize]
            ^ TABLE2[((hi >> 8) & 0xFF) as usize]
            ^ TABLE1[((hi >> 16) & 0xFF) as usize]
            ^ TABLE[((hi >> 24) & 0xFF) as usize];
    }
    for &byte in chunks.remainder() {
        let index = ((crc ^ u32::from(byte)) & 0xFF) as usize;
        crc = (crc >> 8) ^ TABLE[index];
    }
    crc ^ 0xFFFF_FFFF
}

#[cfg(test)]
mod tests {
    use super::crc32;

    /// 标准测试向量：CRC-32/ISO-HDLC("123456789") = 0xCBF43926。
    #[test]
    fn standard_check_vector_123456789() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    /// 空输入恒为 0（`0xFFFFFFFF ^ 0xFFFFFFFF`）。
    #[test]
    fn empty_input_is_zero() {
        assert_eq!(crc32(b""), 0);
    }

    /// 其余参考向量。
    ///
    /// 这些期望值**不是**我手算的，而是用独立实现（Python 3 标准库 `zlib.crc32`，
    /// 底层是 zlib 的 CRC-32）当场算出来的（见 `docs/ledger/container-notes.md` §CRC）。
    /// 其中 `0xFF6CAB0B`（32 个 `0xFF`）是 CRC-32 的著名"难点向量"之一 ——
    /// 手抄的向量很容易错，所以这里逐条用独立实现核对过。
    #[test]
    fn other_published_vectors() {
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
        assert_eq!(crc32(b"abc"), 0x3524_41C2);
        assert_eq!(
            crc32(b"The quick brown fox jumps over the lazy dog"),
            0x414F_A339
        );
        assert_eq!(crc32(&[0x00]), 0xD202_EF8D);
        assert_eq!(crc32(&[0xFF]), 0xFF00_0000);
        assert_eq!(crc32(&[0x00; 32]), 0x190A_55AD);
        assert_eq!(crc32(&[0xFF; 32]), 0xFF6C_AB0B);
    }

    /// 单比特翻转必须改变摘要 —— 否则 CRC 判据形同虚设。
    #[test]
    fn every_single_bit_flip_changes_the_digest() {
        let base: Vec<u8> = (0..64u8).collect();
        let baseline = crc32(&base);
        for index in 0..base.len() {
            for bit in 0..8 {
                let mut mutated = base.clone();
                mutated[index] ^= 1 << bit;
                assert_ne!(crc32(&mutated), baseline, "第 {index} 字节第 {bit} 位");
            }
        }
    }

    /// 与"逐位无查表"参考实现一致（防止查表生成写错）。
    #[test]
    fn table_matches_bitwise_reference() {
        fn bitwise(bytes: &[u8]) -> u32 {
            let mut crc = 0xFFFF_FFFFu32;
            for &byte in bytes {
                crc ^= u32::from(byte);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 {
                        (crc >> 1) ^ 0xEDB8_8320
                    } else {
                        crc >> 1
                    };
                }
            }
            crc ^ 0xFFFF_FFFF
        }

        let sample: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        assert_eq!(crc32(&sample), bitwise(&sample));
    }
}
