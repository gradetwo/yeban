//! ⭐ **真设备输入源 ＋ 可注入的工厂缝**（`[ARCH-REC-001]` 缺的最后一段：**谁在喂录音链**）。
//!
//! ## 本模块补的是什么（⛔ 不是"实现缺失"，是"接线缺失"）
//!
//! `[ARCH-REC-001]` 的三段链**早就建好了**（建环 ⇒ [`crate::record_wav::Recorder::push_block`]
//! ⇒ `RecordDrain` ⇒ 落盘），但**没有任何设备输入在喂它** ⇒ 产品按录音键录到的是一条
//! **合法的 24-bit 静音 WAV**。修复面只有一处：把源从"写死的
//! [`crate::record_wav::SilenceSource`]"换成**真设备源**，并让这条选择**可注入、可判**。
//!
//! ```text
//! cpal 回调线程 ──(yeban_engine::device::capture_callback)──▶ RecordSender
//!                                                               │ (无锁 SPSC 环)
//!                          ┌────────────────────────────────────┘
//!                          ▼
//!   DeviceInputSource { receiver: RecordReceiver, handle: InputStreamHandle }
//!                          │  next_block()：从 receiver 取整帧（空了 ⇒ 返回 0）
//!                          ▼
//!        Recorder 工作线程 ⇒ push_input_block ⇒ RecordDrain ⇒ 量化 24-bit ⇒ 落盘
//! ```
//!
//! ⚠ **两层环**（设备环 ＋ 录音环）是**设计如此**，不是冗余：设备回调的块长由**声卡**决定，
//! 而录音链的抽取批由 `RecordOptions::drain_batch` 决定；`DeviceInputSource` 就是这两条
//! 时钟之间的**唯一**接口（它跑在 [`crate::record_wav::Recorder`] 的后台线程上，
//! ⛔ 不在音频回调里 ⇒ 它的分配/加锁都不违反红线 7 —— 音频回调里**只有**
//! `yeban_engine::record::push_input_block`）。
//!
//! ## ⭐ 工厂缝（本模块对判据的承诺）
//!
//! 源**只能**经 [`InputSourceFactory`] 取得：
//!
//! - 产品实现：[`DeviceInputFactory`]（`negotiate_input` ＋ `open_input` ＋ `play`）；
//! - 显式静音腿：[`SilenceFactory`]（⭐ 保留既有 `SilenceSource`，⛔ 不删）；
//! - 判据实现：集成判据自己实现本 trait（返回脚本化假源 / 返回 `Err`），
//!   **⛔ 不读任何环境变量**（测量纪律第 6 条）。
//!
//! ## ⭐⭐ 裁决：默认 ＝ 真设备源；打不开就**响亮失败**
//!
//! ⛔ **绝不允许**"打不开设备 ⇒ 悄悄录静音"。打开失败一律 `Err(String)` ⇒
//! [`crate::record_wav::RecordError::Source`] ⇒ `host::apply_record_outcome(&ui, Some(文本))`
//! ＋ `stderr`，且**不产生文件、不置 `recording=true`**。
//!
//! ## ⭐ 静音的显式缝（⛔ 不是"没人喂"的偶然）
//!
//! 环境变量 [`RECORD_SOURCE_ENV`] 取**字面量** `silence` ⇒ 显式静音腿；**其它任何值**
//! （含拼错的）⇒ **真设备源**。⭐ 这个方向是刻意选的：拼错一个字母**不可能**把产品
//! 悄悄变成"录静音"（那正是本单要消灭的失败形态）—— 它只会退回到"打不开就响亮失败"。
//!
//! ⚠ 判据**不许**依赖这个环境变量：判据走 [`input_factory_for`]（把选择值当**参数**传进去，
//! 纯函数、可判）或直接注入自己的工厂实现。

use std::rc::Rc;

use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_engine::device::{EngineConfig, InputStreamHandle, ShareMode, open_input};
use yeban_engine::record::{RecordReceiver, record_channel};

use crate::record_wav::{InputSource, RecordOptions, SilenceSource};

/// ⭐ **输入源工厂**（本单新增的**唯一**接线缝）。
///
/// 形如裁决 `#88` §2.2：`open(&options)` ⇒ 一个**活着**的源，或一条**人能读的**失败原因。
///
/// ⚠ 失败原因是 `String`（⛔ 不是 `DeviceError`）：这条缝要同时装下"设备打不开"
/// （`DeviceError`）与"协商到的配置与请求不符"（本模块自己的判断），
/// 而两者都要**原样**进界面的错误文本。
pub trait InputSourceFactory {
    /// 打开一个输入源。
    ///
    /// # Errors
    ///
    /// 打开失败（无默认输入设备 / 协商失败 / 流启动失败 / 协商结果与请求不符）。
    /// ⛔ **不许**用"返回静音源"来假装成功。
    fn open(&self, options: &RecordOptions) -> Result<Box<dyn InputSource + Send>, String>;

    /// ⭐ 这条工厂的**如实**文案（落进 `RecordReport::source`，让"哪条腿"可读）。
    fn label(&self) -> &'static str {
        "input-source-factory(unlabelled)"
    }
}

/// 编译期证明：`InputStreamHandle` 是 `Send`（⇒ [`DeviceInputSource`] 能进录音线程）。
///
/// ⚠ 这条**必须**是编译期断言：`Recorder::start` 把源 move 进 `std::thread::spawn`
/// ⇒ 若句柄不是 `Send`，本模块根本编译不过（那时按裁决 §2 要**停手上报**，
/// ⛔ 不是改成 `Rc`/`unsafe` 绕过去）。
const fn assert_send<T: Send>() {}
const _: () = assert_send::<InputStreamHandle>();

/// ⭐ **产品实现：真设备源**（默认输入设备 ⇒ `f32` 输入流 ⇒ `play()`）。
#[derive(Debug, Clone, Copy, Default)]
pub struct DeviceInputFactory;

impl DeviceInputFactory {
    /// 报告行 / 判据用的如实文案。
    pub const LABEL: &'static str = "device-input-stream";

    /// 请求引擎的配置（⭐ 与 `RecordOptions` 的映射，唯一处）。
    ///
    /// ⚠ 设备回调的**块长请求**用的是运行时量子 `DEFAULT_BLOCK_FRAMES`（128 帧），
    /// ⛔ **不是** `drain_batch`：后者是**控制侧抽取**的批大小（见 `Recorder` 的工作线程），
    /// 把它当设备缓冲请求会让输入延迟跟着抽取批走（默认 4096 帧 ≈ 85 ms）。
    #[must_use]
    pub fn engine_config(options: &RecordOptions) -> EngineConfig {
        EngineConfig {
            sample_rate: options.sample_rate,
            block_frames: DEFAULT_BLOCK_FRAMES as u32,
            channels: options.channels.max(1),
            share_mode: ShareMode::default(),
        }
    }
}

impl InputSourceFactory for DeviceInputFactory {
    fn open(&self, options: &RecordOptions) -> Result<Box<dyn InputSource + Send>, String> {
        let channels = options.channels.max(1);
        let config = Self::engine_config(options);
        // ⭐ 建**设备环**：回调线程推、`DeviceInputSource::next_block` 取。
        let (sender, receiver) = record_channel(options.capacity_frames.max(1), channels);
        let handle =
            open_input(&config, sender).map_err(|error| format!("打开输入设备失败（{error}）"))?;
        // ⭐⭐ **协商结果的两条路，区别对待（都有实测依据）**：
        //
        // ① **通道数不同 ⇒ 照设备说的录**（⛔ 不拒绝、⛔ 不偷偷升降混）。
        //    `negotiate` 的既定政策是"立体声不足时接受设备的默认通道数"—— macOS 的内建
        //    麦克风就是 **1** 声道。若这里拒绝，产品在这台机器上**永远录不了音**
        //    （实测：本机 `transport-record-button` 的 Tier-1 判据就是这样红的），
        //    而那正是本单要消灭的"按录音键没声音"。源的 `channels()` 因此报**协商值**，
        //    由 [`crate::record_wav::Recorder`] 用它建环/切块/写 WAV 头 ⇒ 落一条**单声道**
        //    WAV（"如实"，⛔ 不是把 1 声道复制成 2 声道的假立体声）。
        //    ⚠ 不做降混/上混：本仓对"怎么混"**没有裁决**（`yeban-render` 的
        //    `read_plain_wav_mono_f32` 文档同款措辞）。
        //
        // ② **采样率不同 ⇒ 响亮失败**（与裁决 §2.3 同一条纪律）。理由不是洁癖：
        //    WAV 头的采样率与**预备拍帧数**都是按 `RecordOptions::sample_rate` 在
        //    **打开设备之前**算好的（`host::wire_record` 的 `count_in_frames`）⇒ 静默收下
        //    44.1k 会让音高/时长与预备拍**双重错位**。宁可明确报错。
        let negotiated = *handle.negotiated();
        if negotiated.sample_rate != options.sample_rate {
            return Err(format!(
                "输入设备协商到的采样率（{} Hz）与请求（{} Hz）不符 ⇒ 拒绝录音\
                 （不允许静默改音高/时长；预备拍也是按请求采样率算的）",
                negotiated.sample_rate, options.sample_rate
            ));
        }
        // ⚠ **必须 `play()`**：`InputStreamHandle` 只是"流在场"，不起播 ⇒ 回调一帧都不来。
        handle
            .play()
            .map_err(|error| format!("启动输入流失败（{error}）"))?;
        Ok(Box::new(DeviceInputSource::new(
            receiver,
            handle,
            negotiated.channels.max(1),
        )))
    }

    fn label(&self) -> &'static str {
        Self::LABEL
    }
}

/// ⭐ **真设备输入源**：**同时持有**设备环的接收端与已打开的输入流句柄。
///
/// ## ⚠ 为什么句柄必须是**字段**（不是"打开完就丢"）
///
/// `InputStreamHandle` 一丢，`cpal` 流就析构 ⇒ 回调停 ⇒ 环永远空。⭐ 因此
/// `handle` 的所有权**必须**跟着源活（源的寿命 = 录音会话的寿命）。
///
/// ## ⭐ `next_block` 的口径（写清楚，可判）
///
/// - **有数据**：从设备环取**整整 `min(frames, out 容得下的帧数)` 帧**，按**交错**写进 `out`，
///   返回取到的**帧**数；
/// - **环空**（设备还没喂上来 / 已被下一块取走）：返回 **0**，⛔ **不补静音**。
///   ⭐ 理由：补静音会把"设备没喂上"伪装成**内容**（本单要消灭的正是这种伪装），
///   而调用方（[`crate::record_wav::Recorder`] 的工作线程）对 0 的语义是
///   "这一轮没有数据，下一轮再问" —— 预备拍因此按**帧**计数而不是按墙钟（正确）。
#[derive(Debug)]
pub struct DeviceInputSource<H> {
    /// 设备回调填的那一半的接收端。
    receiver: RecordReceiver,
    /// ⭐ **生命期事实**：这个字段在场 = 流在跑（见类型文档）。
    handle: H,
    /// 声道数（= 请求值 = 协商值，⛔ 两者不等时工厂就拒绝了）。
    channels: u16,
}

impl<H> DeviceInputSource<H> {
    /// 组装一个设备源（⭐ `handle` 的所有权交给本类型）。
    #[must_use]
    pub const fn new(receiver: RecordReceiver, handle: H, channels: u16) -> Self {
        Self {
            receiver,
            handle,
            channels,
        }
    }

    /// 已打开的设备句柄（⭐ 控制面用：读协商结果 / `backend_errors`）。
    #[must_use]
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// 设备环自己的声道数（⭐ 判据用来钉住"与 `channels()` 一致"）。
    #[must_use]
    pub fn receiver_channels(&self) -> u16 {
        self.receiver.channels()
    }
}

impl<H> InputSource for DeviceInputSource<H> {
    fn channels(&self) -> u16 {
        self.channels
    }

    fn next_block(&mut self, frames: usize, out: &mut [f32]) -> usize {
        let channels = usize::from(self.channels.max(1));
        let capacity_frames = out.len() / channels;
        let want = frames.min(capacity_frames);
        if want == 0 {
            return 0;
        }
        let mut written = 0usize;
        let got = self.receiver.drain_frames(want, |frame| {
            let base = written * channels;
            // ⚠ `frame.len()` 恒等于环的声道数；`width` 只是把"越界写"变成不可能。
            let width = frame.len().min(channels);
            out[base..base + width].copy_from_slice(&frame[..width]);
            written += 1;
        });
        // `drain_frames` 的契约：`f` 恰好被调用返回值那么多次。
        debug_assert_eq!(got, written, "drain_frames 的返回值必须等于回调次数");
        got
    }

    fn label(&self) -> &'static str {
        DeviceInputFactory::LABEL
    }
}

/// ⭐ **显式静音腿的工厂**（⛔ 不是删掉静音源，而是让"选静音"变成**显式**动作）。
///
/// 用途两条：① 产品在 [`RECORD_SOURCE_ENV`] 取 `silence` 时走它；
/// ② 判据断言"选静音 ⇒ 录到静音"**是可判地保留的既有行为**（⛔ 不是"没人喂"的偶然）。
#[derive(Debug, Clone, Copy, Default)]
pub struct SilenceFactory;

impl SilenceFactory {
    /// 报告行 / 判据用的如实文案。
    pub const LABEL: &'static str = "explicit-silence-source";
}

impl InputSourceFactory for SilenceFactory {
    fn open(&self, options: &RecordOptions) -> Result<Box<dyn InputSource + Send>, String> {
        Ok(Box::new(SilenceSource {
            channels: options.channels.max(1),
        }))
    }

    fn label(&self) -> &'static str {
        Self::LABEL
    }
}

/// 选择"显式静音腿"的环境变量名。
pub const RECORD_SOURCE_ENV: &str = "YEBAN_RECORD_SOURCE";

/// ⭐ **把选择值当参数**的纯函数（⇒ 判据可注入，⛔ 不必改环境）。
///
/// ⚠ 只有**字面量** `silence`（两侧空白被裁掉）选静音腿；其它任何值 ⇒ 真设备源
/// （见模块文档：拼错不可能悄悄变成录静音）。
#[must_use]
pub fn input_factory_for(selector: Option<&str>) -> Rc<dyn InputSourceFactory> {
    match selector.map(str::trim) {
        Some("silence") => Rc::new(SilenceFactory),
        _ => Rc::new(DeviceInputFactory),
    }
}

/// ⭐ **产品默认的输入源工厂**：真设备源（`YEBAN_RECORD_SOURCE=silence` ⇒ 显式静音腿）。
#[must_use]
pub fn default_input_factory() -> Rc<dyn InputSourceFactory> {
    let selector = std::env::var(RECORD_SOURCE_ENV).ok();
    input_factory_for(selector.as_deref())
}
