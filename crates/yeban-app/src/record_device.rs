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
use yeban_engine::record::{RecordReceiver, RecordSender, record_channel};

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
        Self::engine_config_for(options, options.channels.max(1))
    }

    /// 同上，但**显式**给声道数（`#130`：按**协商到的**声道数请求第二次开流）。
    #[must_use]
    fn engine_config_for(options: &RecordOptions, channels: u16) -> EngineConfig {
        EngineConfig {
            sample_rate: options.sample_rate,
            block_frames: DEFAULT_BLOCK_FRAMES as u32,
            channels: channels.max(1),
            share_mode: ShareMode::default(),
        }
    }
}

/// ⭐⭐ **设备环的唯一建法**（任务单 `#130`）：宽度是**协商到的**声道数，⛔ 不是请求值。
///
/// ## 为什么这一行也要是一个**可判**的函数（⛔ 不是包装癖）
///
/// `RecordSender::push_frame` 的契约是 `frame.len() == self.channels`，**不等就整帧拒收**
/// （`crates/yeban-record/src/lib.rs`：`if frame.len() != channels { return false }`），
/// 而设备回调是按**设备**的声道数切块推的（`capture_callback(..., negotiated.channels)`）。
/// 因此"环的宽度"与"回调的宽度"只要差一位 ⇒ **每一帧都被拒收、环永远是空的**，
/// 而外部读数恰好是：设备打开成功、`play()` 成功、**拉到 0 帧**、落一条 **0 帧** WAV
/// —— 也就是人类报的"GUI 录音没用"（`#130` 的实测形态）。
///
/// ⚠ 本函数就是那条不变式的**唯一**落点：调用方**必须**把 `negotiated.channels` 传进来。
fn device_ring(options: &RecordOptions, channels: u16) -> (RecordSender, RecordReceiver) {
    record_channel(options.capacity_frames.max(1), channels.max(1))
}

impl InputSourceFactory for DeviceInputFactory {
    fn open(&self, options: &RecordOptions) -> Result<Box<dyn InputSource + Send>, String> {
        let requested = options.channels.max(1);
        // ⭐⭐ **环的宽度必须 ＝ 协商到的声道数**（任务单 `#130`）。
        //
        // 协商发生在 [`open_input`] **内部**，而它又**要求**先把发送端交进来 ⇒
        // "先问协商、再建环"这条路只有两条腿：
        //   ① 按**请求**声道建环试开一次（设备与请求一致 ⇒ 就是这一次，⛔ 不开第二次）；
        //   ② 协商结果不同 ⇒ ⭐ **按协商值重建环并重开一次**。
        // 为什么不能凑合：`push_frame` 的 `frame.len() != channels` 会**逐帧拒收**
        // （见 [`device_ring`] 的文档）⇒ 拉到 0 帧、落 0 帧 WAV、人看到"录了没用"。
        // 第一次那条流**从未 `play()`** ⇒ 丢掉它不占用设备（引擎自己的
        // `opening_the_default_input_device_never_panics` 用的就是这条生命期口径）。
        let (sender, receiver) = device_ring(options, requested);
        let handle = open_input(&Self::engine_config_for(options, requested), sender)
            .map_err(|error| format!("打开输入设备失败（{error}）"))?;
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
        // ② **采样率不同 ⇒ 在抽取侧做采样率转换**（T-FIX-72，取代旧的响亮拒绝）。
        //    旧拒绝的理由不是洁癖：WAV 头的采样率与**预备拍帧数**都是按
        //    `RecordOptions::sample_rate` 算的（`host::wire_record` 的
        //    `count_in_frames`），静默按设备率落盘会让音高/时长与预备拍**双重错位**。
        //    但 44100-only 输入设备在真实环境里同样常见，拒绝等于录不了 ——
        //    与输出腿同一个裁决（T-FIX-72）：**转换**正面解决错位：设备环里是
        //    **设备率**帧，[`DeviceInputSource::next_block`] 交出去之前先换成
        //    **请求率** ⇒ WAV 头与预备拍数的都是 `next_block` 交出去的帧，
        //    口径自洽。转换器在这里构造（分配只在打开期），推进在录音工作
        //    线程上（⛔ 不在音频回调里，不受红线 7 约束）。转换状态由协商
        //    结果如实携带（`resampling()`），⛔ 不是静默转换。
        let negotiated = *handle.negotiated();
        let (receiver, handle, negotiated) = if negotiated.channels.max(1) == requested {
            (receiver, handle, negotiated)
        } else {
            // ① 的第一半：设备的声道数与请求值不同 ⇒ **按协商值重建环**。
            let channels = negotiated.channels.max(1);
            drop(handle);
            let (sender, receiver) = device_ring(options, channels);
            let handle = open_input(&Self::engine_config_for(options, channels), sender).map_err(
                |error| format!("按设备协商到的 {channels} 声道重开输入流失败（{error}）"),
            )?;
            // ⭐ 第二次协商必须与第一次**逐项一致**（否则环的宽度又和回调宽度分叉，
            // 转换器的两侧率也会跟着分叉）。
            let again = *handle.negotiated();
            if again.channels.max(1) != channels
                || again.sample_rate != negotiated.sample_rate
                || again.source_rate != negotiated.source_rate
            {
                return Err(format!(
                    "输入设备的第二次协商（{} 声道 / 设备 {} Hz / 工程 {} Hz）与第一次\
                     （{channels} 声道 / 设备 {} Hz / 工程 {} Hz）不一致 ⇒ 拒绝录音\
                     （环的宽度必须与设备回调的宽度逐位一致）",
                    again.channels,
                    again.sample_rate,
                    again.source_rate,
                    negotiated.sample_rate,
                    negotiated.source_rate
                ));
            }
            (receiver, handle, again)
        };
        // ⚠ **必须 `play()`**：`InputStreamHandle` 只是"流在场"，不起播 ⇒ 回调一帧都不来。
        handle
            .play()
            .map_err(|error| format!("启动输入流失败（{error}）"))?;
        // ⭐ 声道数**取自环本身**（`DeviceInputSource::new`）：源报的宽度与环的宽度
        // 因此在结构上不可能分叉 ⇒ `#130` 那条"逐帧拒收"的缺陷无法再出现。
        let mut source = DeviceInputSource::new(receiver, handle);
        // ⭐ 转换器按**最终**协商结果挂：设备率 → 请求率；等率 ⇒ `None`（直通）。
        source.attach_converter(yeban_engine::resample::RateConverter::new(
            negotiated.sample_rate,
            options.sample_rate,
            negotiated.channels,
        ));
        Ok(Box::new(source))
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
    /// 声道数（⭐ **恒等于环的声道数**：`new` 从 `receiver.channels()` 取，
    /// ⛔ 不是调用方另报一个数 —— 两者分叉就是 `#130` 的"逐帧拒收、拉到 0 帧"）。
    channels: u16,
    /// ⭐ **采样率转换器**（T-FIX-72）：设备率 ≠ 请求率时由工厂在打开后经
    /// [`DeviceInputSource::attach_converter`] 挂上；`next_block` 据此把
    /// **设备率**帧换成**请求率**帧再交出去。`None` = 等率直通（现行为）。
    converter: Option<yeban_engine::resample::RateConverter>,
}

impl<H> DeviceInputSource<H> {
    /// 组装一个设备源（⭐ `handle` 的所有权交给本类型）。
    ///
    /// ⭐ **声道数的唯一来源是环**（`#130`）：`RecordSender::push_frame` 按**环**的声道数
    /// 判"整帧"（`frame.len() != channels` ⇒ 拒收），而设备回调按**设备**的声道数切块。
    /// 因此让"源报的宽度"与"环的宽度"在类型构造处**结构上相等**，是这条链唯一的
    /// 正确形状 —— 任何"另传一个 channels 参数"的写法都把分叉重新变成可能。
    #[must_use]
    pub fn new(receiver: RecordReceiver, handle: H) -> Self {
        let channels = receiver.channels().max(1);
        Self {
            receiver,
            handle,
            channels,
            converter: None,
        }
    }

    /// ⭐ 挂上采样率转换器（T-FIX-72）：由工厂在打开设备、协商落定后调用。
    ///
    /// ⛔ 只能在录音开始前挂：`next_block` 的帧数记账（WAV 帧数、预备拍）
    /// 全部以"交出去的是请求率帧"为前提，中途换转换器会让记账错位。
    /// `None` = 等率直通。
    pub fn attach_converter(&mut self, converter: Option<yeban_engine::resample::RateConverter>) {
        self.converter = converter;
    }

    /// 当前挂着的转换器（控制面/判据读两侧采样率与帧数记账）。
    #[must_use]
    pub const fn converter(&self) -> Option<&yeban_engine::resample::RateConverter> {
        self.converter.as_ref()
    }

    /// ⭐ **转换路径的抽取**（T-FIX-72）：设备环里是**设备率**帧，逐帧喂
    /// 转换器、把**请求率**帧排进 `out`，满 `want` 帧或环空为止，返回
    /// 排出的帧数（可 < `want`，与直通路径"环空返回实数"同口径）。
    ///
    /// 每轮先估一个够用的环帧数（`剩余 × in/out + 窗长 + 2`）再抽，⛔ 不把
    /// 整个环一次抽干（两层环的节拍见模块文档）；转换器自带待排输出，
    /// 本轮没排完的下一轮先排，顺序不乱。
    fn next_block_converted(&mut self, want: usize, out: &mut [f32]) -> usize {
        let mut filled = 0usize;
        while filled < want {
            let (in_rate, out_rate) = {
                let converter = self.converter.as_ref().expect("转换路径必有转换器");
                (converter.in_rate(), converter.out_rate())
            };
            let remaining = (want - filled) as u64;
            let estimate = (remaining * u64::from(in_rate) / u64::from(out_rate)) as usize
                + yeban_engine::resample::RESAMPLER_TAPS
                + 2;
            let converter = self.converter.as_mut().expect("转换路径必有转换器");
            let got = self.receiver.drain_frames(estimate, |frame| {
                converter.push_frame(frame, out, &mut filled);
            });
            if got == 0 {
                break;
            }
        }
        filled
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
        // ⭐ T-FIX-72：挂了转换器 ⇒ 环里是设备率帧，先换成请求率再交出去
        //（WAV 头与预备拍只认请求率，见 `open` 的 ② 号口径）。
        if self.converter.is_some() {
            return self.next_block_converted(want, out);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// ⭐⭐ **判据（任务单 `#130`）**：设备环的宽度**只**由调用方给的（协商到的）声道数决定，
    /// ⛔ 不是 `RecordOptions::channels`（请求值）。
    ///
    /// 这条判据钉住的是"环宽 = 协商值"这个**唯一**构造点：产品的默认请求值是 **2** 声道，
    /// 而 macOS 内建麦克风协商到 **1** ⇒ 若这里按请求值建环，回调的每帧都会被
    /// `push_frame` 拒收（`frame.len() != channels`），表现为 `拉取 0 帧 / 落盘 0 帧`。
    ///
    /// 反向腿：把 `device_ring` 的第二参数改回 `options.channels` ⇒ 本判据在
    /// `receiver.channels()` 那一行红（实得 2）。
    #[test]
    fn the_device_ring_takes_the_negotiated_channel_count_not_the_requested_one() {
        let options = RecordOptions::default();
        assert_eq!(
            options.channels, 2,
            "前提：`RecordOptions::default().channels == 2`（请求值，⛔ 不是设备值）"
        );
        // 设备协商到 1 声道（macOS 内建麦克风的实测形态）。
        let (mut sender, mut receiver) = device_ring(&options, 1);
        assert_eq!(
            receiver.channels(),
            1,
            "⭐ 环的宽度必须 = **协商到的** 1 声道（⛔ 不是请求的 2）"
        );
        assert_eq!(sender.channels(), 1);

        // 读数：按设备宽度推一整块 ⇒ 整块被接受、整块可取出（⛔ 不是逐帧拒收）。
        let block = [0.25_f32; 8];
        let accepted = yeban_engine::record::push_input_block(&mut sender, &block, 1);
        assert_eq!(accepted, 8, "同宽 ⇒ 8 帧整块进环（实得 {accepted}）");
        let mut frames = Vec::new();
        let drained = receiver.drain_frames(8, |frame| frames.extend_from_slice(frame));
        assert_eq!(drained, 8, "同宽 ⇒ 8 帧整块取出（实得 {drained}）");
        assert_eq!(frames, block, "取出的样本必须逐样本相同");
        assert_eq!(sender.dropped_frames(), 0);
    }

    /// ⭐⭐ **判据（T-FIX-72，输入腿）**：设备率（44100 Hz）的帧进环，源按
    /// **请求率（48000 Hz）**交出 —— 1000 Hz 正弦在两侧的频率必须一致
    /// （音高/速度不变），输出帧数按 48000/44100 放大（差转换窗半窗长以内）。
    ///
    /// 反向腿：把 `next_block` 的转换分支删掉 ⇒ 本判据在帧数（44100 ≠ 约
    /// 48000）与频率（实测约 918 Hz）两处同时红。
    #[test]
    fn device_input_source_converts_device_rate_frames_to_the_requested_rate() {
        let options = RecordOptions::default();
        assert_eq!(options.sample_rate, 48_000, "前提：默认请求率 48000 Hz");
        assert_eq!(options.channels, 2, "前提：默认请求 2 声道");
        let (mut sender, receiver) = device_ring(&options, 2);
        let mut source = DeviceInputSource::new(receiver, ());
        assert!(source.converter().is_none(), "挂转换器之前必须是直通");
        source.attach_converter(yeban_engine::resample::RateConverter::new(
            44_100, 48_000, 2,
        ));
        assert!(source.converter().is_some(), "异率必须挂上转换器");

        // 设备以 44100 Hz 喂 1 秒立体声 1000 Hz 正弦（左右同相）。
        let frames = 44_100usize;
        let mut block = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let phase = 2.0 * core::f64::consts::PI * 1000.0 * i as f64 / 44_100.0;
            let value = (0.8 * phase.sin()) as f32;
            block.push(value);
            block.push(value);
        }
        let accepted = yeban_engine::record::push_input_block(&mut sender, &block, 2);
        assert_eq!(accepted, frames, "整秒设备帧必须全部进环");

        let mut out = vec![0.0f32; 48_000 * 2];
        let got = source.next_block(48_000, &mut out);
        assert!(
            got > 47_900 && got <= 48_000,
            "转换后应排出约 48000 帧（差半窗长以内），实得 {got}"
        );
        // 左声道取 [0.1 s, 实得末尾前 100 帧) 的上行过零 ⇒ 频率。
        let left: Vec<f32> = out[..got * 2].iter().step_by(2).copied().collect();
        let (from, to) = (4_800usize, got - 100);
        let crossings = left[from..to]
            .windows(2)
            .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
            .count();
        let estimated = crossings as f64 * 48_000.0 / (to - from) as f64;
        assert!(
            (estimated - 1000.0).abs() <= 10.0,
            "输入腿转换后频率应为 1000 Hz ±1%，实测 {estimated} Hz"
        );
    }
}
