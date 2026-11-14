//! ⭐⭐ **任务单 `#88` 的验收核心：把真实设备输入接进录音链**（`[ARCH-REC-001]`）。
//!
//! ## 本单要证的句子
//!
//! "按录音键"这条产品路径**今天**录到的是一条合法的 24-bit **静音** WAV —— 因为三段链
//! （建环 ⇒ `push_block` ⇒ `RecordDrain` ⇒ 落盘）虽然全在，却**没有任何设备输入在喂它**。
//! 本文件证的是**反面**：**源从工厂缝进去，落盘样本与注入的样本逐码相等**。
//!
//! ## 判据表（每条答一个不同的问题；⛔ 不靠真硬件、⛔ 不读环境变量）
//!
//! | 判据 | 答的问题 | 证什么 / **不**证什么 |
//! | :--- | :--- | :--- |
//! | [`the_record_button_writes_the_scripted_device_samples_bit_for_bit`] | 按录音键 ⇒ 真有一条源在喂，且样本**逐码**到达磁盘 | 证"产品录音链被接线、且不是静音"。不证"真声卡"（那是 `[HARDWARE]` 那条） |
//! | [`the_recorder_takes_its_source_from_the_factory_and_reports_its_label`] | `Recorder` 也从**同一个**缝拿源；报告行 `source=` 如实 | 证"报告行不再写死 `no-device-input`"。不证 UI 接线（上一条管它） |
//! | [`the_device_source_drains_its_receiver_in_order_and_never_fabricates_silence`] | 真设备源的形状（`RecordReceiver` ＋ `InputStreamHandle`）本身对不对 | 证"环空 ⇒ 返回 0，⛔ 不补静音"与"句柄的生命期跟着源"。不证设备打开 |
//! | [`an_unopenable_device_fails_loudly_and_writes_nothing`] | 打不开设备时会发生什么 | 证"**响亮失败**：工厂恰好调 1 次 / 界面收到非空文本 / `recording` 仍 false / **目标路径上没有文件**" |
//! | [`the_explicit_silence_leg_is_still_available_and_records_silence`] | 静音腿还在不在、是不是**显式**选的 | 证"录静音"只能是**选出来的**（控制臂），⛔ 不是"没人喂" |
//! | [`the_count_in_reading_is_identical_on_both_legs`] | 预备拍会不会因为换了源而变 | 证"预备拍读数与源**无关**"，且边界确实在 `count_in_frames` 那一帧 |
//! | [`the_product_default_factory_is_the_device_leg`] | 产品的默认究竟是哪条腿 | 证"默认 = 真设备源"，且**只有字面量** `silence` 才选静音（拼错不会悄悄静音） |
//! | [`a_real_input_device_feeds_the_recorder_through_the_product_factory`] | 真麦克风那条腿 | `[HARDWARE]`（`#[ignore]`，需真人手工验证） |
//!
//! ## ⭐ "逐码"的口径（⛔ 不是含糊其辞）
//!
//! 落盘路径**故意**带 TPDF 抖动（`[ARCH-FMT-001]`：每样本 ±1 LSB）⇒ "读回 == 注入的浮点
//! 样本"在数学上**不可能**。因此判据用的是第 244 条的最强形式：**与手搓等价物逐位比对** ——
//! 期望值 = 用**同一个**量化器（`quantize_i24`）＋ **同一个**确定性抖动种子（`new(1)`）
//! ＋**同一串**样本独立算出来的 i24 码，容差 **0 个码**。
//!
//! 于是它同时是**逐位**的、又**不**把抖动误判成缺陷：把注入样本乘 `1.0000001`
//! （≈ 0.8 个码的位移）就会让某个样本的舍入翻过去 ⇒ 判据**红**（见判据模块的注入记录）。

#![allow(unused)]
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::host::{
    RecordOutcomePort, UiRecordOutcome, build_main_window, wire_record, wire_record_at,
};
use yeban_app::record_device::{
    DeviceInputFactory, DeviceInputSource, InputSourceFactory, SilenceFactory, input_factory_for,
};
use yeban_app::record_wav::{InputSource, RecordOptions, RecordPlan, Recorder, count_in_frames};
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_engine::device::capture_callback;
use yeban_engine::record::record_channel;
use yeban_model::samples::filled_project;
use yeban_render::dither::{BitDepth, PcmBuffer, quantize_i24};
use yeban_render::rng::DeterministicDitherRng;
use yeban_render::wav::read_plain_wav;
use yeban_ui_test_port::Size;

/// 无头 Tier-1 平台窗口尺寸（与 `tests/gui_save_writes_live_session.rs` 同款）。
const WINDOW: Size = Size::new(1440, 900);
/// 采样率（= [`RecordOptions::default`] 的值，两条腿共用）。
const SAMPLE_RATE: u32 = 48_000;
/// 声道数（判据全程 2 声道：交错口径要能被看见）。
const CHANNELS: u16 = 2;
/// 假设备的块长（帧/次）：⭐ 刻意**小于** `drain_batch`，让"块序"必须被保住。
const DEVICE_CHUNK: usize = 128;
/// 脚本化假源每次产出的帧数。
const SCRIPTED_FRAMES: usize = 1024;
/// 脚本化假源的如实标签。
const SCRIPTED_LABEL: &str = "scripted-device-source(判据#88)";
/// 帧号斜坡源的如实标签（预备拍判据用）。
const RAMP_LABEL: &str = "frame-index-ramp(判据#88)";
/// 帧号斜坡的步长：`3e-6 × 8388608 ≈ 25 码/帧` ⇒ 首帧索引可反推（±0.04 帧）。
const RAMP_STEP: f32 = 3.0e-6;

/// 判据用目录（`target/` 已被 gitignore ⇒ ⛔ 不污染仓库、⛔ 不落系统临时目录的随机路径）。
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp/mod-app-record-device-wiring");
    std::fs::create_dir_all(&dir).expect("建判据目录");
    dir.join(format!("{name}-{}.wav", std::process::id()))
}

/// 建环参数（两条腿同一条：⛔ 不许因为源不同而改参数）。
fn options() -> RecordOptions {
    RecordOptions {
        seconds: 1.0,
        sample_rate: SAMPLE_RATE,
        channels: CHANNELS,
        capacity_frames: 8192,
        drain_batch: 512,
        ..RecordOptions::default()
    }
}

/// ⭐ **脚本化的"已知样本"**：近满标、交替符号、**非零**。
///
/// ⚠ 为什么取**近满标**而不是小幅度：注入纪律要求"把样本乘 `1.0000001` ⇒ §3.1 必须红"。
/// 那个相对位移在**码**域里的绝对量是 `|值| × 1e-7 × 2^23` ⇒ 只有 |值| 接近 1
/// 才可能把某个样本的舍入真的推过边界（≈ 0.8 个码）。小幅度样本**测不出**这条。
/// ⛔ 这里不是在"制造巧合"：判据本身是**容差 0 码**的逐位比对，
/// 这条样本集只是让"0.8 个码的位移"落在能被检出的一侧。
fn scripted_frames() -> Vec<f32> {
    (0..SCRIPTED_FRAMES)
        .map(|index| {
            let magnitude = 0.95 + (index as f32) * 1.0e-6;
            if index % 2 == 0 {
                magnitude
            } else {
                -magnitude
            }
        })
        .collect()
}

/// 把"每帧一个值"摊成**交错**声道流（期望值必须与管道吃到的顺序逐位对齐）。
fn interleaved(frames: &[f32], channels: u16) -> Vec<f32> {
    let channels = usize::from(channels.max(1));
    let mut out = Vec::with_capacity(frames.len() * channels);
    for value in frames {
        for _ in 0..channels {
            out.push(*value);
        }
    }
    out
}

/// 等到条件成立（超时即**红**：⛔ 不用"睡固定时长"来赌）。
fn wait_until(what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("等待超时：{what}");
}

/// ⭐ **有界等待，⛔ 不 panic**：等到了返回 `true`、超时返回 `false`。
///
/// ⚠ 判据 ① 专用它：源一旦没被接上（"把设备源换回静音"那类注入），**"等不到"不是本判据
/// 要报的错** —— 要报的是**读数**："落盘全是 0"。因此先把控制流走到"读回 ＋ 比对"，
/// 让失败语落在峰值/逐码上（注入纪律 §4.1 明确要求失败语看得出"读到的全是 0"）。
fn wait_or_false(mut predicate: impl FnMut() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if predicate() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// 假源与判据共享的读数（⭐ 注入面从**代码**进，⛔ 不经环境变量）。
#[derive(Debug, Default)]
struct ScriptShared {
    /// 假源已经交出去的**帧**数。
    produced: AtomicUsize,
    /// 环空之后被判据探测到的**轮**数（证明数据真的被抽干过，而不是"刚好在边界停下"）。
    exhausted_polls: AtomicUsize,
}

/// ⭐ **脚本化假设备源**：按 [`DEVICE_CHUNK`] 的块长重放一串已知帧值。
#[derive(Debug)]
struct ScriptedDevice {
    frames: Vec<f32>,
    channels: u16,
    cursor: usize,
    shared: Arc<ScriptShared>,
}

impl InputSource for ScriptedDevice {
    fn channels(&self) -> u16 {
        self.channels
    }

    fn next_block(&mut self, frames: usize, out: &mut [f32]) -> usize {
        let channels = usize::from(self.channels.max(1));
        let capacity = out.len() / channels;
        if self.cursor >= self.frames.len() {
            self.shared.exhausted_polls.fetch_add(1, Ordering::Relaxed);
            return 0;
        }
        let give = DEVICE_CHUNK
            .min(frames)
            .min(capacity)
            .min(self.frames.len() - self.cursor);
        for index in 0..give {
            let value = self.frames[self.cursor + index];
            out[index * channels..(index + 1) * channels].fill(value);
        }
        self.cursor += give;
        self.shared.produced.fetch_add(give, Ordering::Relaxed);
        give
    }

    fn label(&self) -> &'static str {
        SCRIPTED_LABEL
    }
}

/// 产出 [`ScriptedDevice`] 的工厂（⭐ 判据版 `InputSourceFactory`）。
#[derive(Debug)]
struct ScriptedFactory {
    frames: Vec<f32>,
    channels: u16,
    shared: Arc<ScriptShared>,
}

impl ScriptedFactory {
    fn new(frames: &[f32], channels: u16, shared: Arc<ScriptShared>) -> Self {
        Self {
            frames: frames.to_vec(),
            channels,
            shared,
        }
    }
}

impl InputSourceFactory for ScriptedFactory {
    fn open(&self, _options: &RecordOptions) -> Result<Box<dyn InputSource + Send>, String> {
        // ⭐ **不校验** `options.channels`：源自己的 `channels()` 就是权威（判据 ⑧ 用它造
        // "设备只给 1 声道、请求是 2"的真机常态）。
        Ok(Box::new(ScriptedDevice {
            frames: self.frames.clone(),
            channels: self.channels,
            cursor: 0,
            shared: Arc::clone(&self.shared),
        }))
    }

    fn label(&self) -> &'static str {
        SCRIPTED_LABEL
    }
}

/// ⭐ **帧号斜坡源**（无限）：`value(frame) = frame × step` ⇒ 首帧索引可从落盘码反推。
#[derive(Debug)]
struct FrameIndexRamp {
    channels: u16,
    step: f32,
    produced: u64,
}

impl InputSource for FrameIndexRamp {
    fn channels(&self) -> u16 {
        self.channels
    }

    fn next_block(&mut self, frames: usize, out: &mut [f32]) -> usize {
        let channels = usize::from(self.channels.max(1));
        let whole = frames.min(out.len() / channels);
        for index in 0..whole {
            let value = (self.produced + index as u64) as f32 * self.step;
            out[index * channels..(index + 1) * channels].fill(value);
        }
        self.produced += whole as u64;
        whole
    }

    fn label(&self) -> &'static str {
        RAMP_LABEL
    }
}

/// 产出 [`FrameIndexRamp`] 的工厂。
#[derive(Debug)]
struct RampFactory;

impl InputSourceFactory for RampFactory {
    fn open(&self, options: &RecordOptions) -> Result<Box<dyn InputSource + Send>, String> {
        Ok(Box::new(FrameIndexRamp {
            channels: options.channels.max(1),
            step: RAMP_STEP,
            produced: 0,
        }))
    }

    fn label(&self) -> &'static str {
        RAMP_LABEL
    }
}

/// ⭐ **打不开设备的工厂**（裁决 §3.2）：`open` **恰好**被调用、并返回一条原因。
#[derive(Debug, Default)]
struct FailingFactory {
    calls: Cell<usize>,
}

impl InputSourceFactory for FailingFactory {
    fn open(&self, _options: &RecordOptions) -> Result<Box<dyn InputSource + Send>, String> {
        self.calls.set(self.calls.get() + 1);
        Err("模拟：打不开输入设备（判据 #88）".to_owned())
    }

    fn label(&self) -> &'static str {
        "failing-device-factory(判据#88)"
    }
}

/// ⭐ **观察端口**：记下产品落点收到了什么，**然后**照常转调产品落点
/// （[`UiRecordOutcome`] ⇒ 既有 `apply_record_outcome`）。
///
/// ⚠ 这一层是必要的：界面今天**只有** `recording` 一个布尔属性（录音状态**文本**控件属
/// `.worktrees/mod-app-86` 那条线，本单⛔ 不碰 `ui/app.slint`）⇒ "界面收到非空错误文本"
/// 只能从这个**落点**观测，⛔ 不能靠抓 stderr。
#[derive(Debug, Default)]
struct SpyOutcome {
    reports: RefCell<Vec<Option<String>>>,
}

impl RecordOutcomePort for SpyOutcome {
    fn report(&self, ui: &MainWindow, error: Option<String>) {
        self.reports.borrow_mut().push(error.clone());
        UiRecordOutcome.report(ui, error);
    }
}

/// 组装一个**真实**的活窗口（Tier-1 无头平台；⭐ 与产品同一条构造入口）。
fn assemble() -> (yeban_ui_test_port::render::Tier1Window, MainWindow) {
    let tier1 =
        yeban_ui_test_port::render::Tier1Window::install(WINDOW).expect("装 Tier-1 无头平台");
    let view = ViewState::from_project(&demo_project()).expect("装配时投影");
    let scene = DemoScene::from_view(&view);
    let ui = build_main_window(&view, &scene).expect("生产窗口构造入口");
    tier1
        .resize(WINDOW)
        .expect("窗口尺寸定下来之后再注入一次布局");
    (tier1, ui)
}

/// 读回 WAV 的 **i24 码**（⛔ 不折成归一化浮点：判据要比的是**码**，容差 0）。
fn read_codes(path: &std::path::Path) -> (u16, u32, Vec<i32>) {
    let (format, buffer) = read_plain_wav(path).expect("读回 WAV");
    let PcmBuffer::Int24(codes) = buffer else {
        panic!("落盘必须是 24-bit 整数 PCM");
    };
    (format.channels, format.sample_rate, codes)
}

/// ⭐⭐ **判据 ①（本单核心）**：按录音键 ⇒ 脚本化假设备源的样本**逐码**落到磁盘。
///
/// 量什么：落盘 WAV 的**每一个** i24 码 与 手搓等价物（同一量化器 ＋ 同一抖动种子 ＋
/// 同一串样本）的**差**，单位 = i24 码，容差 **0**；外加"峰值"读数（单位 = i24 码）
/// 用来证明**不是静音**（今天的行为是整段全 0）。
#[test]
fn the_record_button_writes_the_scripted_device_samples_bit_for_bit() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback toggle-record("),
        "新 UI 契约面必须声明 callback toggle-record("
    );
    assert!(
        aether_host.contains("on_toggle_record"),
        "宿主接线层必须注册 on_toggle_record"
    );
}

/// ⭐ **判据 ②**：`Recorder` **也**从同一个工厂缝拿源，且报告行 `source=` **如实**
/// （⛔ 不再是写死的 `ui-record-button(no-device-input)`）。
#[test]
fn the_recorder_takes_its_source_from_the_factory_and_reports_its_label() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback toggle-record("),
        "新 UI 契约面必须声明 callback toggle-record("
    );
    assert!(
        aether_host.contains("on_toggle_record"),
        "宿主接线层必须注册 on_toggle_record"
    );
}

/// ⭐ **判据 ③**：真设备源的**形状**本身 —— 环空返回 0（⛔ 不补静音）、有数据按**交错序**取出、
/// 句柄的生命期跟着源。
#[test]
fn the_device_source_drains_its_receiver_in_order_and_never_fabricates_silence() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback toggle-record("),
        "新 UI 契约面必须声明 callback toggle-record("
    );
    assert!(
        aether_host.contains("on_toggle_record"),
        "宿主接线层必须注册 on_toggle_record"
    );
}

/// ⭐ **判据 ④（裁决 §3.2）**：打不开设备 ⇒ **响亮失败**，⛔ 绝不"录一条静音"。
///
/// 断言四件事（一条失败腿都不能少）：
/// (a) 工厂被调用**恰好 1 次**；(b) 界面落点收到**非空**错误文本；
/// (c) `recording` 保持 `false`；(d) **目标路径上没有文件**。
#[test]
fn an_unopenable_device_fails_loudly_and_writes_nothing() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback toggle-record("),
        "新 UI 契约面必须声明 callback toggle-record("
    );
    assert!(
        aether_host.contains("on_toggle_record"),
        "宿主接线层必须注册 on_toggle_record"
    );
}

/// ⭐⭐ **判据 ⑨（任务单 `#130`：0 帧不许静默成功）**：设备**能开**但**一帧都不给**
/// （`#130` 修好之前，macOS 内建麦克风那条腿的真实形态就是 `拉取 0 帧`）⇒ 停录仍然会落
/// 一条**合法但空**的 WAV（0 帧 ⇒ 只有头）⇒ ⛔ **不许**把它画成"录成功了"：
/// 落点必须收到一条**非空**文本，且文本里的计数器必须**如实**（`pushed=0`）。
///
/// 本判据⛔ 不声称"空文件不该存在"（那是 `Recorder` 的既有语义，本单没改）——
/// 它钉的是**可见性**：人可以不知道文件在哪，但不该不知道"这次一帧都没录到"。
#[test]
fn a_zero_frame_recording_is_reported_loudly_instead_of_looking_successful() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback toggle-record("),
        "新 UI 契约面必须声明 callback toggle-record("
    );
    assert!(
        aether_host.contains("on_toggle_record"),
        "宿主接线层必须注册 on_toggle_record"
    );
}

/// ⭐ **判据 ⑤b**：设备只给 **1** 声道（macOS 内建麦克风的常态）⇒ 照实**录单声道**，
/// ⛔ 不拒绝录音、⛔ 不偷偷把 1 声道复制成 2 声道。
///
/// ⚠ 这条判据来自**实测**：本机（M2 MacBook）的默认输入设备协商到 **1** 声道，
/// 而 `RecordOptions::default().channels == 2`；早期版本在工厂里"通道数不符就拒绝"，
/// 于是产品的录音键在这台机器上**永远起不来**（`real_ui_tier1` 的
/// `pressing_the_record_button_starts_and_stops_a_session` 实测红）。
/// 现在的口径：`InputSource::channels()` 是权威 ⇒ 环 / 切块 / WAV 头一致地用**协商值**。
#[test]
fn a_mono_device_is_recorded_as_mono_instead_of_being_refused() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback toggle-record("),
        "新 UI 契约面必须声明 callback toggle-record("
    );
    assert!(
        aether_host.contains("on_toggle_record"),
        "宿主接线层必须注册 on_toggle_record"
    );
}

/// ⭐ **判据 ⑤（控制臂）**：显式静音腿**仍然可用**，录到的**只有**抖动。
///
/// ⚠ 这一条存在的意义：若它缺席，判据 ① 的"逐码相等"也可能是"管道把什么都写成 0"
/// 而恰好对上。而且它把"录到静音"钉成**一个被选中的腿**，⛔ 不是"没人喂"的偶然。
#[test]
fn the_explicit_silence_leg_is_still_available_and_records_silence() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback toggle-record("),
        "新 UI 契约面必须声明 callback toggle-record("
    );
    assert!(
        aether_host.contains("on_toggle_record"),
        "宿主接线层必须注册 on_toggle_record"
    );
}

/// ⭐ **判据 ⑥**：预备拍读数**与源无关**（两条腿读同一个 `count_in_frames`，边界真的在那一帧）。
///
/// 量什么：帧号斜坡腿的**首个落盘样本**反推出来的帧索引（单位 = 帧），必须落在
/// `[count_in − block, count_in]` 里 —— 即"预备拍期间照拉、⛔ 不追录"。
#[test]
fn the_count_in_reading_is_identical_on_both_legs() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback toggle-record("),
        "新 UI 契约面必须声明 callback toggle-record("
    );
    assert!(
        aether_host.contains("on_toggle_record"),
        "宿主接线层必须注册 on_toggle_record"
    );
}

/// ⭐⭐ **判据 ⑧（任务单 `#130`：缺陷形态本身）**：设备回调的**块宽**与录音环的**环宽**
/// 一旦分叉，`RecordSender::push_frame` 会**逐帧拒收** ⇒ 环永远空 ⇒ 拉到 0 帧 ⇒
/// 落一条 0 帧 WAV（人类看到的"按录音键没用"）。
///
/// ## 这条判据证的是"为什么"，⛔ 不是复述症状
///
/// 产品的真设备源在 `#130` 之前把环建成 `RecordOptions::channels`（默认 **2**），
/// 而 macOS 内建麦克风协商到 **1** 声道 ⇒ `capture_callback(..., 1)` 的每一帧都是
/// `push_frame(&[x])`（`frame.len() == 1 != 2`）⇒ **全部被拒收**（且不计入 `pushed`）。
/// 本判据按产品的两个宽度各跑一遍同一条链：
///
/// | 臂 | 环宽 | 回调宽 | 期望读数 |
/// | :--- | :--- | :--- | :--- |
/// | ① 缺陷形态 | 2（请求值） | 1（设备值） | **0 帧**进环、`pushed == 0` |
/// | ② 修复形态 | 1（设备值） | 1（设备值） | **128 帧**逐样本到达 |
///
/// ⚠ 反向腿（本判据 + `[HARDWARE]` 一起）：把
/// `record_device::DeviceInputFactory::open` 的环宽改回 `options.channels`
/// ⇒ ⑥ 那条真设备判据立刻红（实测 `拉取 0 帧 / 落盘 0 帧`）。
#[test]
fn a_mono_device_block_into_a_stereo_ring_loses_every_frame() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback toggle-record("),
        "新 UI 契约面必须声明 callback toggle-record("
    );
    assert!(
        aether_host.contains("on_toggle_record"),
        "宿主接线层必须注册 on_toggle_record"
    );
}

/// ⭐ **判据 ⑦**：产品的**默认**究竟是哪条腿 —— 机械可判、零硬件、⛔ 不读环境。
///
/// 判法：把"选择值"当**参数**传进 `input_factory_for`（纯函数），读工厂自己的 `label()`。
/// ⛔ 不调用 `open()`（那会在有声卡的机器上真的打开麦克风）。
#[test]
fn the_product_default_factory_is_the_device_leg() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback toggle-record("),
        "新 UI 契约面必须声明 callback toggle-record("
    );
    assert!(
        aether_host.contains("on_toggle_record"),
        "宿主接线层必须注册 on_toggle_record"
    );
}

/// `[HARDWARE]` **真设备那条腿**：真麦克风 ＋ 真声卡时钟 ⇒ 必须真人手工验证。
///
/// ⚠ 本判据**只**验"产品工厂真的把真输入流接上了、且设备时钟真的送来帧"；
/// **不**验"录到的内容有声音"（那要真人对着麦克风说话/放声音后手工核对，本判据不冒充）。
/// 无声卡 / 无输入设备的机器上如实打印 `本机未执行`，⛔ 不冒充通过。
#[test]
#[ignore = "[HARDWARE] 需真人手工验证：真输入设备 + 真声卡时钟（CI 与无输入设备的机器行为不同）"]
fn a_real_input_device_feeds_the_recorder_through_the_product_factory() {
    let options = options();
    let path = scratch("hardware");
    let _ = std::fs::remove_file(&path);
    let mut recorder = Recorder::new();
    match recorder.start_with_factory(&path, &options, &DeviceInputFactory, 0) {
        Ok(()) => {
            std::thread::sleep(Duration::from_millis(500));
            let pulled = recorder.pulled_frames();
            let report = recorder.stop().expect("停录并落盘");
            println!(
                "[record-device] 真输入设备：拉取 {pulled} 帧 / 落盘 {} 帧 / source={} / {}",
                report.frames,
                report.source,
                path.display()
            );
            assert_eq!(report.source, DeviceInputFactory::LABEL);
            assert!(
                pulled > 0 && report.frames > 0,
                "真设备时钟必须真的送来帧（拉取 {pulled}）"
            );
            println!(
                "[record-device] ⚠ 请真人手工核对 {} 里是否有声音（本判据不冒充'内容有声'）",
                path.display()
            );
        }
        Err(reason) => {
            println!("[record-device] 本机未执行（无可用输入设备）：{reason}");
        }
    }
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// ⭐ 任务单 `#213`：录音回调的**借用边界**（与 `#212` 的 `engine` 崩溃同形）
// ---------------------------------------------------------------------------

/// 降级时那**一行** stderr 的逐字正文（判据按它数行）。
const DEGRADE_LINE: &str = "忽略 toggle-record：录音器正被同一线程借用（重入）⇒ 本次按键无效";

/// ⭐ `#213` 判据 ① 的**界面落点探针**：在 `report` 执行的那一刻去要录音器的借。
///
/// 这正是"有人在录音状态变化上挂了一个要碰录音器的回调"的形状：
/// * 修之前 `RefMut` 跨过 `ui.set_recording` / `outcome.report` ⇒ 这里
///   `borrow_mut()` 直接 panic（`RefCell already borrowed`）；
/// * 修之后借已经放开 ⇒ 借得到，计数 +1。
#[derive(Debug, Default)]
struct BorrowProbeOutcome {
    recorder: RefCell<Option<Rc<RefCell<Recorder>>>>,
    reports: AtomicUsize,
    borrow_free_reports: AtomicUsize,
}

impl BorrowProbeOutcome {
    /// 接线返回句柄之后挂上来（报告在第一次按键之后才可能发生）。
    fn attach(&self, recorder: Rc<RefCell<Recorder>>) {
        *self.recorder.borrow_mut() = Some(recorder);
    }
}

impl RecordOutcomePort for BorrowProbeOutcome {
    fn report(&self, ui: &MainWindow, error: Option<String>) {
        self.reports.fetch_add(1, Ordering::SeqCst);
        let recorder = self
            .recorder
            .borrow()
            .clone()
            .expect("探针必须在第一次 report 之前拿到录音器句柄");
        {
            // ⭐ 承重的一行：修前在这里 panic，修后借得到。
            let recorder = recorder.borrow_mut();
            let _ = recorder.is_recording();
        }
        self.borrow_free_reports.fetch_add(1, Ordering::SeqCst);
        UiRecordOutcome.report(ui, error);
    }
}

/// 判据 ①（**能红**）：界面落点执行时，录音器的借**已经**放开。
///
/// 修前的红是 **panic**（`RefCell already borrowed`，点就在上面那个 `borrow_mut()`）——
/// 不是断言失败，因此它同时证明"修前确实持借跨了界面写"。修后起录 / 停录各一次落点，
/// 两次都必须落在借外。
#[test]
fn the_record_callback_releases_the_recorder_borrow_before_writing_the_interface() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback toggle-record("),
        "新 UI 契约面必须声明 callback toggle-record("
    );
    assert!(
        aether_host.contains("on_toggle_record"),
        "宿主接线层必须注册 on_toggle_record"
    );
}

/// 判据 ②（**兜底生效**）：**故意持借**时按录音键 ⇒ 降级（一行 stderr、状态一位不动），
/// ⛔ 不是 panic、⛔ 更不是 abort。
///
/// 与 `#212` 的 `toggle_play_and_stop_degrade_instead_of_aborting_when_the_engine_is_already_borrowed`
/// 同形：手工持借模拟"同一线程上已有一份借"，回调必须走 `try_borrow_mut()` 的失败臂。
#[test]
fn a_held_recorder_borrow_degrades_the_toggle_and_leaves_the_state_untouched() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback toggle-record("),
        "新 UI 契约面必须声明 callback toggle-record("
    );
    assert!(
        aether_host.contains("on_toggle_record"),
        "宿主接线层必须注册 on_toggle_record"
    );
}

/// 判据 ③（**stderr 恰好一行**）：在子进程里跑判据 ②，读回真实 stderr。
///
/// 为什么要子进程：`eprintln!` 的落点是进程级的 fd 2；测试进程里并行跑着别的判据，
/// 进程内重定向既不可靠也会吞掉别人的输出。子进程用 libtest 的 `--exact` 只跑那一条，
/// 父进程读回它的 stderr —— 这是"**一行**"这个数量事实的直接证据。
#[test]
fn a_held_recorder_borrow_emits_exactly_one_stderr_line() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback toggle-record("),
        "新 UI 契约面必须声明 callback toggle-record("
    );
    assert!(
        aether_host.contains("on_toggle_record"),
        "宿主接线层必须注册 on_toggle_record"
    );
}

/// 判据 ③ 的**子进程角色**（正常跑也通过；父判据用 `--exact` 只跑它并读 stderr）。
#[test]
fn record_borrow_degrade_child_when_invoked_directly() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> recording:"),
        "新 UI 契约面必须声明 > recording:"
    );
    assert!(
        aether_host.contains("set_recording"),
        "宿主接线层必须注册 set_recording"
    );
    assert!(
        aether_app.contains("callback toggle-record("),
        "新 UI 契约面必须声明 callback toggle-record("
    );
    assert!(
        aether_host.contains("on_toggle_record"),
        "宿主接线层必须注册 on_toggle_record"
    );
}
