//! MIDI 可变长度量 (VLQ) 的**零依赖参考编解码器** [ARCH-FMT-001 §5.5]。
//!
//! 为什么单独一个模块: SMF 的 `MTrk` 事件都用 VLQ 编码 delta-time 与 meta 长度
//! (Standard MIDI File 1.0, "Variable-Length Quantity")。夜半的导出走 `midly`,
//! 但**本模块是独立实现**——它存在的唯一理由是充当"独立解码器"去逐字节核验
//! `midly` 写出来的字节, 而不是复述 `midly` 自己的逻辑。两个独立实现互相钉住,
//! 这是唯一能发现"库的编码与规范不符"的办法。
//!
//! 本模块**不依赖任何第三方 crate**, 因此可以用
//! `rustc --edition 2024 --test src/vlq.rs` 单独编译执行 (见
//! `docs/ledger/render-master-notes.md` §5 的本机验证记录)。
//!
//! ## VLQ 规则 (Standard MIDI File 1.0 §"Variable-Length Quantity")
//!
//! - 大端序、7 位一组, 最高位 (bit 7) 是"后面还有字节"标志;
//! - 最后一个字节 bit 7 = 0;
//! - 因此 4 字节 VLQ 的最大值是 `0x0FFF_FFFF` (28 位)。
//!   `0x0FFF_FFFF` 的编码恰好是 `FF FF FF 7F` —— 这是本模块与 `midi.rs` 的边界判据。

/// VLQ 能表示的最大值 (28 位, 4 字节)。
///
/// 规范把 delta-time 与 meta 长度都限制在这个上界; `midly` 用 `u28` 表达同一约束。
pub const VLQ_MAX: u32 = 0x0FFF_FFFF;

/// VLQ 编码的最大字节数。
pub const VLQ_MAX_BYTES: usize = 4;

/// 把 `value` 以 VLQ 追加到 `out`。
///
/// # Panics
///
/// `value > VLQ_MAX` 时 panic —— 上界是规范硬约束, 不是可恢复错误:
/// 调用方 (`midi.rs`) 必须在进入本函数前把"tick 越界"变成 `Result`。
/// 在这里静默截断会让导出的文件与源工程不一致, 那比崩溃更糟。
pub fn write_into(value: u32, out: &mut Vec<u8>) {
    assert!(
        value <= VLQ_MAX,
        "VLQ 只能表示 28 位 (<= 0x0FFF_FFFF), 收到 {value:#010X}"
    );
    // 从最高有效 7 位组开始输出。先算出组数, 再逐组写, 避免分配临时数组。
    let mut groups = 1;
    while (value >> (7 * groups)) != 0 {
        groups += 1;
    }
    for index in (0..groups).rev() {
        let mut byte = ((value >> (7 * index)) & 0x7F) as u8;
        if index != 0 {
            byte |= 0x80;
        }
        out.push(byte);
    }
}

/// 把 `value` 编码为 VLQ 并返回新分配的最短字节序列。
#[must_use]
pub fn encode(value: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(VLQ_MAX_BYTES);
    write_into(value, &mut out);
    out
}

/// 从 `bytes[*pos..]` 解码一个 VLQ, 并把 `*pos` 推进到下一个字节。
///
/// 返回 `None` 表示: 字节耗尽、或 4 字节之内仍未见结束标志 (即数值超出 28 位)。
/// 后者是**拒绝**, 不是回绕 —— 恶意/损坏的文件不得被解析成一个"看起来合法"的大数。
#[must_use]
pub fn decode(bytes: &[u8], pos: &mut usize) -> Option<u32> {
    let mut value: u32 = 0;
    for _ in 0..VLQ_MAX_BYTES {
        let byte = *bytes.get(*pos)?;
        *pos += 1;
        value = (value << 7) | u32::from(byte & 0x7F);
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 规范与公开实现都给过这几个标准向量 (SMF 1.0 的经典例子)。
    #[test]
    fn encodes_known_spec_vectors() {
        assert_eq!(encode(0), vec![0x00]);
        assert_eq!(encode(0x40), vec![0x40]);
        assert_eq!(encode(0x7F), vec![0x7F]);
        assert_eq!(encode(0x80), vec![0x81, 0x00]);
        assert_eq!(encode(0x2000), vec![0xC0, 0x00]);
        assert_eq!(encode(0x3FFF), vec![0xFF, 0x7F]);
        assert_eq!(encode(0x4000), vec![0x81, 0x80, 0x00]);
        assert_eq!(encode(0x10_0000), vec![0xC0, 0x80, 0x00]);
        assert_eq!(encode(0x0FFF_FFFF), vec![0xFF, 0xFF, 0xFF, 0x7F]);
    }

    #[test]
    fn max_value_uses_exactly_four_bytes() {
        assert_eq!(encode(VLQ_MAX).len(), VLQ_MAX_BYTES);
        assert_eq!(encode(VLQ_MAX - 1), vec![0xFF, 0xFF, 0xFF, 0x7E]);
    }

    #[test]
    fn every_encoded_value_round_trips() {
        // 覆盖每个字节长度的边界与中点, 而不是"抽几个看看"。
        let mut cases = vec![0u32, 1, 0x7F, 0x80, 0x3FFF, 0x4000, 0x1F_FFFF, 0x20_0000];
        cases.push(VLQ_MAX - 1);
        cases.push(VLQ_MAX);
        for value in cases {
            let bytes = encode(value);
            let mut pos = 0;
            assert_eq!(decode(&bytes, &mut pos), Some(value), "value={value:#X}");
            assert_eq!(pos, bytes.len(), "解码器必须恰好消费 {bytes:?}");
        }
    }

    #[test]
    fn decode_rejects_five_continuation_bytes() {
        // 5 个字节全是 continuation -> 超出 28 位 (规范外的值) -> 必须拒绝。
        let bogus = [0xFFu8, 0xFF, 0xFF, 0xFF, 0x7F, 0x00];
        let mut pos = 0;
        assert_eq!(decode(&bogus, &mut pos), None);
    }

    #[test]
    fn decode_rejects_truncated_input() {
        let mut pos = 0;
        assert_eq!(decode(&[0x81], &mut pos), None);
        let mut pos = 0;
        assert_eq!(decode(&[], &mut pos), None);
    }

    #[test]
    fn decode_does_not_advance_pos_on_rejection_by_more_than_scanned() {
        // 拒绝时 pos 的最终值不是契约的一部分, 但**不得越界**, 否则调用方回退会踩空。
        let bogus = [0xFFu8, 0xFF, 0xFF, 0xFF];
        let mut pos = 0;
        assert_eq!(decode(&bogus, &mut pos), None);
        assert!(pos <= bogus.len());
    }
}
