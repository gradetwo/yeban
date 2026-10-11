//! 录音侧的无锁 PCM 环形缓冲与后台落盘（[`ARCH-REC-001`]）。
//!
//! # 规范要求（逐字要点）
//!
//! `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:756`：
//! 「音频线程直接从声卡物理输入捕获 PCM 流并写入**预分配的无锁环形缓冲区**，
//! 由**后台 I/O 线程**刷盘至 CAS 临时暂存；录音停止后，系统依据测得的物理声卡
//! 输入延迟做补偿。」
//!
//! 本模块实现其中的**前两段**：
//!
//! 1. [`record_channel`]：**预分配**的无锁 SPSC 环（生产端 = 音频线程，消费端 = I/O 线程）；
//! 2. [`RecordDrain`]：后台侧把环里的样本**按帧**刷到一个写入器。
//!
//! ⛔ **本模块不碰声卡**（cpal 输入流属另一条边，交 CI）；它只做"把已到手的 PCM
//! 安全地从音频线程搬到 I/O 线程"这一件事，因此**本机可编译、可判据化**。
//!
//! # 为什么用 `rtrb` 而不是自造
//!
//! `rtrb` 已是本 crate 的依赖（`ring.rs` 的 `event_channel` 就建在它之上）。
//! [AGENTS.md] 的取向是"⛔ 不重复造轮子"：环的正确性（无锁、无 ABA、预分配）
//! 已在既有依赖里。本模块**只**加录音特有的语义：**声道数**与**溢出计数**。
//!
//! # 红线 7（实时音频安全）
//!
//! [`RecordSender::push_frame`] 是**给音频回调用的**：它
//! **不分配**、**不加锁**、**不做 I/O**（`rtrb` 的 `push` 满足这三点）。
//! 环满时它**不阻塞**：丢弃该帧并把**溢出计数**加一 —— 丢弃并计数是**可观测**的
//! 降级，阻塞或分配则不是。
//!
//! [`ARCH-REC-001`]: ../../../docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md

use std::io::Write;

/// 录音信道的生产端（**音频线程持有**）。
///
/// ⛔ 只允许调用 [`RecordSender::push_frame`] 与 [`RecordSender::dropped_frames`]：
/// 这两个都不分配、不加锁、不做 I/O [MUST-GATE-001 / 红线 7]。
#[derive(Debug)]
pub struct RecordSender {
    inner: rtrb::Producer<f32>,
    channels: u16,
    /// 因环满而被丢弃的**帧**数（可观测降级，见模块头）。
    dropped_frames: u64,
    /// 已成功推入的**帧**数。
    pushed_frames: u64,
}

/// 录音信道的消费端（**后台 I/O 线程持有**）。
#[derive(Debug)]
pub struct RecordReceiver {
    inner: rtrb::Consumer<f32>,
    channels: u16,
}

/// 建立一条录音信道。`capacity_frames` 是**帧**容量（内部会乘声道数）。
///
/// 这里是**唯一**分配点：调用方必须在**构造期/控制侧**调用它
/// （⛔ 不可在音频回调里调用）。
#[must_use]
pub fn record_channel(capacity_frames: usize, channels: u16) -> (RecordSender, RecordReceiver) {
    let samples = capacity_frames.saturating_mul(usize::from(channels.max(1)));
    let (producer, consumer) = rtrb::RingBuffer::new(samples.max(1));
    (
        RecordSender {
            inner: producer,
            channels: channels.max(1),
            dropped_frames: 0,
            pushed_frames: 0,
        },
        RecordReceiver {
            inner: consumer,
            channels: channels.max(1),
        },
    )
}

impl RecordSender {
    /// 声道数。
    #[must_use]
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// 已推入的帧数。
    #[must_use]
    pub fn pushed_frames(&self) -> u64 {
        self.pushed_frames
    }

    /// 因环满而丢弃的帧数。
    #[must_use]
    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames
    }

    /// 推入**一帧**（`frame.len()` 必须等于声道数）。
    ///
    /// ⛔ **不分配／不加锁／不做 I/O**（红线 7）。空间不足 ⇒ **丢弃该帧**并让
    /// [`RecordSender::dropped_frames`] 加一，**⛔ 不阻塞**。
    ///
    /// # 整帧原子（这条是**必需**的，不是优化）
    ///
    /// 本函数**先**检查整帧是否放得下，**再**写样本。若改成"边写边试"，
    /// 一个写了一半才失败的帧会**留在环里** —— 而 `rtrb` 的 `push` 一旦接受就无法回滚
    /// ⇒ 下一个帧的样本会接在**半帧**后面 ⇒ **声道错位**（左声道的数据出现在右声道位置）。
    /// 所以"先问后写"是**正确性**要求。（此前的实现只做了逐样本试推并在失败处返回，
    /// 会把半帧留在环里 —— 由 `yeban-app` 侧的读数暴露，本版修掉。）
    ///
    /// # 计数口径
    ///
    /// [`RecordSender::dropped_frames`] 计的是**帧**，⛔ 不是样本。调用方**无需**
    /// 自己再判一次 `spare_frames()`；直接调用本函数即可（[`spare_frames`] 仍公开，
    /// 供需要预知剩余空间的调用方使用）。
    ///
    /// [`spare_frames`]: RecordSender::spare_frames
    ///
    /// 返回 `true` 表示整帧都进去了。
    pub fn push_frame(&mut self, frame: &[f32]) -> bool {
        let channels = usize::from(self.channels);
        if frame.len() != channels {
            return false;
        }
        // 前置检查：整帧原子。`rtrb` 的 `slots()` 是**样本**数 ⇒ 除以声道数得帧数。
        if self.inner.slots() < channels {
            self.dropped_frames += 1;
            return false;
        }
        // 到这里整帧必然放得下（单生产者 ⇒ 不会与本函数自身竞争）。
        for &sample in frame {
            let pushed = self.inner.push(sample);
            debug_assert!(pushed.is_ok(), "前置检查已保证整帧放得下");
        }
        self.pushed_frames += 1;
        true
    }

    /// 环里还能放下多少**帧**（调用方可在推之前问，从而**整帧原子**地推）。
    ///
    /// 用法：`if sender.spare_frames() >= 1 { sender.push_frame(&f); }`
    #[must_use]
    pub fn spare_frames(&self) -> usize {
        self.inner.slots() / usize::from(self.channels)
    }
}

impl RecordReceiver {
    /// 声道数。
    #[must_use]
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// 取走最多 `max_frames` 帧，交给 `f` 逐帧处理（后台 I/O 线程用）。
    ///
    /// 返回实际处理的帧数。⚠ 允许部分帧：若环里的样本数不是帧的整数倍
    /// （理论上不该发生），**余数留在环里**等下一轮，⛔ 不丢。
    pub fn drain_frames<F>(&mut self, max_frames: usize, mut f: F) -> usize
    where
        F: FnMut(&[f32]),
    {
        let channels = usize::from(self.channels);
        let available = self.inner.slots();
        let whole = (available / channels).min(max_frames);
        let mut frame = vec![0.0f32; channels];
        for _ in 0..whole {
            for slot in frame.iter_mut() {
                match self.inner.pop() {
                    Ok(sample) => *slot = sample,
                    Err(_) => return 0,
                }
            }
            f(&frame);
        }
        whole
    }
}

/// 把收到的帧**按交错顺序**刷到一个 [`Write`]（后台 I/O 线程用）。
///
/// 本类型**只**在后台线程使用：它持有接收端，并在每次 [`RecordDrain::pump`] 时
/// 把样本写出去。⛔ 它不做重采样、不做延迟补偿（那属后续"逐步完善"）。
#[derive(Debug)]
pub struct RecordDrain<W: Write> {
    receiver: RecordReceiver,
    sink: W,
    frames_written: u64,
}

impl<W: Write> RecordDrain<W> {
    /// 用一条已建立的接收端与一个写入器构造。
    #[must_use]
    pub fn new(receiver: RecordReceiver, sink: W) -> Self {
        Self {
            receiver,
            sink,
            frames_written: 0,
        }
    }

    /// 已写出的帧数。
    #[must_use]
    pub fn frames_written(&self) -> u64 {
        self.frames_written
    }

    /// 抽干一轮（最多 `max_frames` 帧），把样本**小端 f32** 写进 sink。
    ///
    /// # Errors
    ///
    /// 底层写入器报错时原样透出（本层不吞错、不重试 —— 重试策略属调用方）。
    pub fn pump(&mut self, max_frames: usize) -> std::io::Result<usize> {
        let mut bytes: Vec<u8> = Vec::new();
        let written = self.receiver.drain_frames(max_frames, |frame| {
            for sample in frame {
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
        });
        if written > 0 {
            self.sink.write_all(&bytes)?;
            self.sink.flush()?;
            self.frames_written += written as u64;
        }
        Ok(written)
    }

    /// 归还写入器（录音落盘后取回句柄）。
    pub fn into_inner(self) -> W {
        self.sink
    }
}

/// ⭐ **音频输入回调的"回调体"**（cpal 数据回调里**唯一**允许调用的函数）。
///
/// 把一块**交错（interleaved）**输入切成整帧，逐帧交给 [`RecordSender::push_frame`]。
///
/// # 红线 7（[MUST-GATE-001]）—— 回调内禁令与后果
/// 纯函数：只读 `block`、只写环。**⛔ 不分配**（无 `Vec::push`／`Box`／`String`）、**⛔ 不加锁**、
/// **⛔ 无任何 I/O**。违反后果：音频线程不可预测停顿（分配可能 `malloc`、锁可能被控制线程
/// 长持、I/O 直接阻塞采样时钟）⇒ 由 `tests/rt_zero_alloc.rs` 的计数型全局分配器窗口检测
/// （⚠ 该 target 是 `harness = false`，场景必须由它的 `main` 显式调用）。
///
/// # 语义（可判定）
/// * **整帧才推**：末尾不足一帧的**残余样本被忽略**；返回值按**帧**计（⛔ 不按样本）；
/// * `channels == 0` ⇒ 返回 0（⛔ 不 panic、⛔ 不写环）；
/// * 环满 ⇒ `push_frame` 返回 `false` 并把 `dropped_frames` **按帧**加一（⛔ 不阻塞、⛔ 不分配）；
/// * ⚠ `frame.len() != channels` 的帧**不计入丢弃**（`push_frame` 的既有语义）——
///   因此**本函数的 `channels` 必须与建环时的 `channels` 一致**，否则会出现"看似丢弃为 0"的假象。
///
/// 返回：**被接受的帧数**（⛔ 不含丢弃；丢弃读 [`RecordSender::dropped_frames`]）。
#[must_use]
pub fn push_input_block(sender: &mut RecordSender, block: &[f32], channels: u16) -> usize {
    if channels == 0 {
        return 0;
    }
    let width = usize::from(channels);
    let frames = block.len() / width;
    let mut accepted = 0usize;
    for index in 0..frames {
        let start = index * width;
        if sender.push_frame(&block[start..start + width]) {
            accepted += 1;
        }
    }
    accepted
}

#[cfg(test)]
mod tests {
    use super::record_channel;
    use crate::record::push_input_block;
    use std::io::Cursor;

    /// 判据 ①：推入的帧能**整帧**取回，且**逐样本**一致（数量 + 值）。
    #[test]
    fn frames_pushed_are_drained_back_byte_exact() {
        let (mut tx, mut rx) = record_channel(16, 2);
        assert!(tx.push_frame(&[0.25, -0.25]));
        assert!(tx.push_frame(&[0.5, -0.5]));
        assert_eq!(tx.pushed_frames(), 2);
        assert_eq!(tx.spare_frames(), 14);
        let mut got: Vec<Vec<f32>> = Vec::new();
        let n = rx.drain_frames(16, |frame| got.push(frame.to_vec()));
        assert_eq!(n, 2);
        assert_eq!(got, vec![vec![0.25, -0.25], vec![0.5, -0.5]]);
    }

    /// 判据 ②：环满 ⇒ **丢弃并计数**，⛔ 不阻塞、⛔ 不 panic（可观测降级）。
    #[test]
    fn a_full_ring_drops_and_counts_instead_of_blocking() {
        let (mut tx, _rx) = record_channel(2, 1);
        assert_eq!(tx.spare_frames(), 2);
        assert!(tx.push_frame(&[1.0]));
        assert!(tx.push_frame(&[2.0]));
        assert_eq!(tx.spare_frames(), 0);
        // 第三帧必然进不去。
        assert!(!tx.push_frame(&[3.0]));
        assert_eq!(tx.dropped_frames(), 1);
        assert_eq!(tx.pushed_frames(), 2);
    }

    /// 判据 ③：错长度的帧被**拒绝且不污染环**（否则会声道错位）。
    #[test]
    fn a_frame_of_the_wrong_width_is_refused_without_touching_the_ring() {
        let (mut tx, mut rx) = record_channel(4, 2);
        assert!(!tx.push_frame(&[1.0]));
        assert!(!tx.push_frame(&[1.0, 2.0, 3.0]));
        assert_eq!(tx.pushed_frames(), 0);
        assert_eq!(tx.spare_frames(), 4);
        assert_eq!(rx.drain_frames(4, |_| {}), 0);
    }

    /// 判据 ④：`RecordDrain` 把样本**小端 f32 交错**写出，帧数与推入数一致。
    #[test]
    fn the_drain_writes_little_endian_interleaved_samples() {
        let (mut tx, rx) = record_channel(8, 2);
        assert!(tx.push_frame(&[1.0, -1.0]));
        assert!(tx.push_frame(&[0.5, -0.5]));
        let sink = Cursor::new(Vec::new());
        let mut drain = super::RecordDrain::new(rx, sink);
        assert_eq!(drain.pump(8).expect("写"), 2);
        assert_eq!(drain.frames_written(), 2);
        let bytes = drain.into_inner().into_inner();
        assert_eq!(bytes.len(), 4 * 4, "2 帧 × 2 声道 × 4 字节");
        assert_eq!(&bytes[0..4], &1.0f32.to_le_bytes());
        assert_eq!(&bytes[4..8], &(-1.0f32).to_le_bytes());
        assert_eq!(&bytes[8..12], &0.5f32.to_le_bytes());
        assert_eq!(&bytes[12..16], &(-0.5f32).to_le_bytes());
    }

    /// ⭐ 判据（容量口径，**机械**）：建环后**立刻**读 `spare_frames()`。
    ///
    /// 这条把"环容量是**样本**、业务口径是**帧**"钉死：`record_channel(n, ch)` 的 `n`
    /// **就是帧数**（实现里 `samples = n × ch`、`spare_frames() = slots() / ch`）。
    #[test]
    fn record_channel_capacity_is_in_frames_right_after_creation() {
        for (capacity_frames, channels) in [(2usize, 2u16), (16, 2), (8, 1), (7, 4)] {
            let (tx, _rx) = record_channel(capacity_frames, channels);
            println!(
                "[record] 建环(capacity_frames={capacity_frames}, channels={channels}) ⇒ 立刻 spare_frames={}",
                tx.spare_frames()
            );
            assert_eq!(
                tx.spare_frames(),
                capacity_frames,
                "⭐ 建环后立刻 spare_frames() 必须**等于 capacity_frames**（容量口径 = 帧）"
            );
            assert_eq!(tx.channels(), channels, "通道数必须原样保留");
        }
    }

    /// ⭐ 判据（喂 k 帧、容量 n ⇒ `pushed == min(k, n)`、`dropped == max(0, k − n)`）。
    ///
    /// 期望值**由上面那条读数直接给出**，⛔ 不推断。
    #[test]
    fn feeding_k_frames_into_capacity_n_pushes_min_and_drops_the_rest() {
        for (capacity_frames, channels, k) in
            [(2usize, 2u16, 5usize), (4, 2, 4), (4, 2, 9), (0, 2, 3)]
        {
            let (mut tx, mut rx) = record_channel(capacity_frames, channels);
            let block: Vec<f32> = (0..k)
                .flat_map(|i| (0..usize::from(channels)).map(move |c| (i * 10 + c) as f32))
                .collect();
            let accepted = push_input_block(&mut tx, &block, channels);
            let expected_pushed = k.min(capacity_frames);
            let expected_dropped = k.saturating_sub(capacity_frames);
            let mut drained = 0usize;
            rx.drain_frames(usize::MAX, |_| drained += 1);
            println!(
                "[record] capacity={capacity_frames} channels={channels} k={k} ⇒ accepted={accepted} dropped={} drained={drained}",
                tx.dropped_frames()
            );
            assert_eq!(accepted, expected_pushed, "接受帧数必须 == min(k, n)");
            assert_eq!(
                tx.dropped_frames(),
                u64::try_from(expected_dropped).expect("小整数"),
                "⭐ 丢弃帧数必须 == max(0, k − n)（帧级）"
            );
            assert_eq!(
                drained, expected_pushed,
                "环里能被整帧取回的条数 == 接受帧数"
            );
        }
    }

    /// ⭐ 判据 a：喂入的块 ⇒ 环里能被 `drain_frames` 整帧取回，且**逐样本一致**。
    /// 判定量（已命名）：**逐样本最大绝对差**（按**样本**计，通道交错）。
    #[test]
    fn input_block_round_trips_sample_for_sample() {
        let channels = 2u16;
        let (mut tx, mut rx) = record_channel(64, channels);
        let block: Vec<f32> = (0..16u32)
            .flat_map(|i| [i as f32 / 10.0, 1.0 + i as f32 / 10.0])
            .collect(); // 32 样本 = 16 帧
        let accepted = push_input_block(&mut tx, &block, channels);
        assert_eq!(accepted, 16, "32 样本 / 2 通道 ⇒ 16 帧");
        let mut got = Vec::new();
        let drained = rx.drain_frames(64, |frame| got.extend_from_slice(frame));
        assert_eq!(drained, 16, "环里应有 16 整帧");
        let max_abs_diff = block
            .iter()
            .zip(got.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        println!(
            "[record] 逐样本最大绝对差 = {max_abs_diff:e}（喂入 {} 样本，取回 {} 样本）",
            block.len(),
            got.len()
        );
        assert_eq!(got.len(), block.len(), "取回样本数必须与喂入一致");
        assert_eq!(max_abs_diff, 0.0, "⭐ 逐样本最大绝对差必须为 0");
        assert_eq!(tx.dropped_frames(), 0, "容量足够 ⇒ 不得丢帧");
    }

    /// ⭐ 判据（退化输入不 panic，语义可判定）：`channels == 0`、残余样本、通道不匹配。
    #[test]
    fn input_block_degenerate_inputs_are_defined_not_panicking() {
        let (mut tx, mut rx) = record_channel(8, 2);
        assert_eq!(
            crate::record::push_input_block(&mut tx, &[1.0, 2.0, 3.0], 0),
            0,
            "channels=0 ⇒ 0 帧"
        );
        assert_eq!(rx.drain_frames(8, |_| {}), 0, "channels=0 不得写入任何帧");
        // 3 样本 / 2 通道 ⇒ 1 整帧，1 个残余样本被忽略
        assert_eq!(
            crate::record::push_input_block(&mut tx, &[1.0, 2.0, 3.0], 2),
            1,
            "整帧才推"
        );
        let mut frames = 0usize;
        rx.drain_frames(8, |_| frames += 1);
        assert_eq!(frames, 1, "只应有 1 帧");
        assert_eq!(tx.dropped_frames(), 0, "残余样本⛔ 不计入丢弃");
    }
}
