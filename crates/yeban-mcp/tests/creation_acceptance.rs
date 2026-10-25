//! **`#335` 验收：只用 MCP 工具面做一首曲子，然后量它。**
//!
//! 在此之前，每一个能力都有自己的判据（工程 / 音轨 / 片段 / 音符表现力 / 弯音 / 滑音 /
//! 设备 / 自动化 / 段落 / 母带导出），但没有一条判据证明**这些能力能拼成一首曲子**。
//! 本文件补的就是这一条：它**不构造任何模型结构体**，整首曲子经由
//! `yeban_open_project` / `yeban_edit_notes` / `yeban_merge_proposal` /
//! `yeban_edit_automation` / `yeban_render_master` / `yeban_query_project` /
//! `yeban_query_engine_state` 的 JSON 实参搭出来，再对产物做测量。
//!
//! ## 曲子蓝图（全部单位：tick / Hz / dB / 秒）
//!
//! | 项 | 值 | 怎么来 |
//! | :--- | :--- | :--- |
//! | 拍号 | **3/4** | `yeban_open_project` 的 `timeSignature`（唯一通路） |
//! | 速度 | 124 → **120 BPM** | `create.bpm = 124`，再经 `setTempo` 写成 120 |
//! | 长度 | **4 小节 × 3 拍 × 960 PPQ = 11_520 tick** | 四个片段摆放都写到 11_520 |
//! | 采样率 / 声道 | 48_000 Hz / 2 | `yeban_render_master` 的 `sampleRate: 48000` |
//! | 帧数/时长 | **288_000 帧 / 6.000 s** | 11_520 tick × (48_000×60)/(120×960) = 11_520×25 |
//! | 音轨 | 4 条 + 主总线 | 创建时 `trackCount: 4`（每条自动接 Master） |
//! | 打击 | 1 小节 3 音（kick 36 / snare 38 / kick 36）+ **循环 4 遍** | `placement.loopEnabled` / `loopStartTick:0` / `loopEndTick:2880` |
//! | 贝斯 | 每小节 3 个四分音（根音 + 根音 + 五度），12 音 | `add` ops |
//! | 和声 | 4 个三和弦各 1 小节（Am / F / C / G），12 音 | `add` ops |
//! | 主音 | 音符 A：0..5_760 tick，pitch 69（440 Hz），**±60 音分颤音**（周期 960 tick）；音符 B：5_760..11_520 tick，pitch 69，**滑音到 81（880 Hz）**（2_880 tick 内滑完，最后一小节保持 880） | `pitchBendCurve` / `slide` |
//! | 自动化 | 贝斯轨 `TrackVolume`：tick 0 = −46 dB → tick 11_520 = −6 dB（静态 −12 dB） | `yeban_edit_automation` 两个点 |
//! | 混响 | 和声轨 `insertDevice`：`reverb_size 0.8` / `reverb_damp 0.25` / `reverb_wet 0.45` | `insertDevice` |
//! | 段落 | `Intro` 0..5_760、`Drop` 5_760..11_520 | `setSection`（create） |
//!
//! ## 这一文件量什么（对象 ＋ 单位）
//!
//! | 判据 | 对象 | 单位 | 阈值（为什么） |
//! | :--- | :--- | :--- | :--- |
//! | 时长 | 母带帧数 / 秒 | 帧 / 秒 | 精确等于 `11_520×25 = 288_000 帧`、`6.000 s`；这是蓝图的算术，不是"大概 4 小节" |
//! | 峰值 | 24-bit 文件样本 `max|s|` 与工具的真峰值 | 满刻度线性（1.0 = 0 dBFS）/ dBTP | `≤ 1.0` 且 `clippedSamples == 0`、`truePeakDbfs ≤ 0`；见下方"阈值会漏掉什么" |
//! | 响度 | `data.loudness.measuredIntegratedLufs` | LUFS（ITU-R BS.1770-4 门限积分） | 回归带 `实测值 ± 1.0 LU`（实测值在首次跑通后钉住）；1 dB 的混音变化就会让它变红 |
//! | 逐轨可闻 | 静音/独奏渲染与基线母带的样本差 | dBFS（整段 RMS） | 每条轨 `> −30 dBFS`；见下方"阈值会漏掉什么" |
//! | 颤音 | 主音独奏母带两个窗口的主频 | Hz | 440×2^(±60/1200) = **455.5 Hz / 425.0 Hz**，各 ±2 Hz，且高频窗口 − 低频窗口 > 25 Hz |
//! | 滑音 | 主音独奏母带两个窗口的主频 | Hz | 起点 ≈440 Hz（±6）、末段 ≈880 Hz（±3） |
//! | 循环 | 打击独奏母带逐小节 RMS | dBFS | 四个小节都 `> −45 dBFS`（循环没展开 ⇒ 只有第 1 小节有声） |
//! | 段落 | `yeban_query_project` 的 `sections` | tick | `Intro` 0..5_760、`Drop` 5_760..11_520，逐 tick 相等 |
//! | 自动化 | 自动化开/关两次母带的逐小节 RMS | dBFS | 第 1 小节：开 < 关 − 1 dB；第 4 小节：开 > 关 + 0.5 dB（−46→−6 dB 的渐强方向） |
//! | 混响 | 混响开/旁通两次母带的 `sha256` 与样本差 | 哈希 / dBFS | 哈希不同且差 > −40 dBFS |
//!
//! ### 阈值会漏掉什么（先说清，免得"绿"被当成"对"）
//!
//! - **峰值**判据只保证"没有样本被钳位"。把整首曲子的推子各降 12 dB 它照样绿 ——
//!   它证明的是"没有过载"，⛔ 不是"有足够的余量 / 动态"。它也不评价**真实音乐**的
//!   瞬态是否难听（真峰值只是 16× 过采样读数，不是人耳）。
//! - **响度**的 ±1 LU 是**回归带**（钉在本次工具链的实测值上），不是"音乐上正确的
//!   响度"。它抓得住"多/少 1 dB 的混音变化"，抓不住"这个混音好不好听"。
//! - **逐轨可闻**的 −30 dBFS 地板只证明"拿掉这条轨，母带确实变了"。一条只在极短
//!   瞬态里响一下的轨也能过；它不证明这条轨在**应该**响的位置响。
//! - **颤音/滑音**的量法用 1 Hz 步长的 Hann 窗 DFT 主频。它证明音高**真的移动**，
//!   不证明移动的**曲线形状**是音乐上想要的；颤音的相位窗口由蓝图固定，换一段音乐
//!   就得重挑窗口。
//! - **循环**的逐小节 RMS 只证明四小节都有能量。它抓不住"循环区间取错了半拍"
//!   （只要每小节都有能量就绿）。
//! - **自动化/混响**的 A/B 只证明"这个开关对母带有影响"。它不证明影响**恰好**是
//!   作者要的增益/湿度；A/B 的方向性断言（第 1 小节 vs 第 4 小节）才把"渐强方向"
//!   钉住。
//!
//! ## 本次实测读数（`#335`，stable，48 kHz / 24-bit；由本判据自己打印）
//!
//! | 量 | 实测 | 说明 |
//! | :--- | :--- | :--- |
//! | 帧数 / 时长 | 288_000 帧 / 6.000000 s | 与蓝图算术逐帧相等 |
//! | 文件峰值 | 0.723624（−2.810 dBFS） | 24-bit 文件样本 `max\|s\|` |
//! | 工具真峰值 | −2.788 dBTP | 16× 过采样；`clippedSamples == 0` |
//! | 积分响度 | **−12.74 LUFS** | `data.loudness.measuredIntegratedLufs` |
//! | 逐轨静音差 | percussion −23.00 / bass −22.06 / harmony −23.07 / lead −18.20 dBFS | 整段逐样本差 RMS |
//! | 颤音 | 456 Hz（+60 音分）/ 425 Hz（−60 音分），差 31 Hz | 期望 455.5 / 425.0 |
//! | 滑音 | 起点窗 448 Hz → 末段窗 880 Hz | 末段保持目标 A5 |
//! | 循环 | 四小节打击独奏 RMS −23.0016 / −23.0016 / −23.0016 / −23.0016 dBFS | 1 小节的内容铺满 4 小节 |
//! | 自动化 | bar1 开 −15.85 / 关 −13.85 dBFS；bar4 开 −13.23 / 关 −13.94 dBFS | 方向与 −46→−6 dB 一致 |
//! | 混响旁通差 | −25.05 dBFS | 和声轨 Freeverb 对母带真的有贡献 |
//!
//! 稳定产物（未跟踪，**不进仓库**）：`target/acceptance-335/yeban-335-piece.wav`
//! （1_728_694 字节，sha256 见判据末尾的 `### [335] 产物` 行与同目录的
//! `yeban-335-piece-report.json`）。
//!
//! ## 仍然不对的地方（即便全绿）
//!
//! 这首曲子在音乐上是**粗糙**的：和声是四个三和弦的整小节长音，贝斯是根音+五度，
//! 打击只有 kick/snare 两个音高，主音只有两个音。这里没有任何判据评价**品味**
//! （和声进行是否好听、节奏是否有律动、混音是否有层次）；它只证明"这些能力拼得
//! 起来，且拼出来的东西在物理上真的是它们"。
//!
//! ## 工具面撞到的墙
//!
//! 见本文件末尾 [`WALLS`] 常量与模块末尾的说明；每一堵墙都记录了工具名、实参、
//! 精确错误/观察结果与绕法。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};

// ---------------------------------------------------------------------------
// 蓝图的算术（全部是编译期常量；测试里只用它们，不从响应里反推）
// ---------------------------------------------------------------------------

/// 工程速度（BPM）。先建成 124，再用 `setTempo` 写成 120 —— 于是"速度真的被设置"
/// 在 `yeban_query_project` 里是一个可检查的事实，而母带时长用最终值 120 算。
const FINAL_BPM: u64 = 120;

/// 960 PPQ（`yeban_model::PPQ` 的同一数值；本文件不 import 模型，只用蓝图算术）。
const PPQ: u64 = 960;

/// 4 小节。
const BARS: u64 = 4;

/// 每小节 3 拍（3/4）。
const BEATS_PER_BAR: u64 = 3;

/// 一小节的 tick 数：3 拍 × 960。
const BAR_TICKS: u64 = BEATS_PER_BAR * PPQ;

/// 整首曲子的 tick 数：4 小节 × 3 拍 × 960 = 11_520。
const TOTAL_TICKS: u64 = BARS * BAR_TICKS;

/// 母带采样率（Hz）。
const SAMPLE_RATE: u64 = 48_000;

/// 1 tick 等于多少母带帧：`48_000×60 / (120×960) = 25`。
const FRAMES_PER_TICK: u64 = SAMPLE_RATE * 60 / (FINAL_BPM * PPQ);

/// 请求的母带总帧数：`11_520 × 25 = 288_000`。
const EXPECTED_FRAMES: u64 = TOTAL_TICKS * FRAMES_PER_TICK;

/// 请求的母带时长（秒）：`288_000 / 48_000 = 6.000`。
const EXPECTED_SECONDS: f64 = 6.0;

/// 48 kHz 下 288_000 帧的秒数（用整数算一遍，避免手写 6.0 与帧数脱节）。
#[allow(clippy::cast_precision_loss)]
const COMPUTED_SECONDS: f64 = EXPECTED_FRAMES as f64 / SAMPLE_RATE as f64;

/// A4 = 440 Hz（MIDI 69）。
const LEAD_HZ: f64 = 440.0;

/// 颤音幅度（音分）：+60 / −60。
const VIBRATO_CENTS: f64 = 60.0;

/// 颤音周期（tick）：960 = 半秒 ⇒ 2 Hz。
const VIBRATO_PERIOD: u64 = 960;

/// 颤音的 +60 音分平台（音符局部 tick）：[240, 480)。
const VIBRATO_HIGH_TICKS: u64 = 240;

/// 颤音的 −60 音分平台（音符局部 tick）：[720, 960)。
const VIBRATO_LOW_TICKS: u64 = 720;

/// 颤音测量窗长度（帧）：4096 ≈ 85 ms ⇒ 在 440 Hz 上约 37 个周期。
const PITCH_WINDOW_FRAMES: usize = 4096;

/// 颤音两窗的期望主频（Hz）：`440 × 2^(±60/1200)`。
///
/// 440×2^0.05 = 455.5166…；440×2^(−0.05) = 425.0（数值上恰好落在 425.0 附近）。
const EXPECTED_HIGH_HZ: f64 = 455.5;
const EXPECTED_LOW_HZ: f64 = 425.0;

/// 主音的滑音音符：起点 tick = 2 小节 = 5_760。
const SLIDE_START_TICK: u64 = 2 * BAR_TICKS;

/// 滑音时长 1 小节（2_880 tick）；音符总长 5_760 tick（2 小节）⇒ 滑完后的最后
/// 一小节保持在目标音高（880 Hz），供末段窗口测量。
const SLIDE_DURATION_TICKS: u64 = BAR_TICKS;

/// 滑音目标音高：MIDI 81 = A5 = 880 Hz。
const SLIDE_TARGET_HZ: f64 = 880.0;

/// 打击乐循环区间：1 小节。
const LOOP_END_TICKS: u64 = BAR_TICKS;

// ---------------------------------------------------------------------------
// 测量的阈值（每个都在模块文档里解释了"为什么"与"会漏掉什么"）
// ---------------------------------------------------------------------------

/// 整段母带响度的**回归带**：实测值 ± 1.0 LU。
///
/// ⚠ 这不是"音乐上正确的响度"，是"同一份混音在同一个工具链上不应漂移超过 1 LU"。
const LUFS_TOLERANCE_LU: f64 = 1.0;

/// 逐轨静音/独奏的样本差地板（dBFS）。低于它就认为"这条轨拿掉后母带几乎没变"。
const TRACK_DIFF_FLOOR_DBFS: f64 = -30.0;

/// 混响旁通的样本差地板（dBFS）。
const REVERB_DIFF_FLOOR_DBFS: f64 = -40.0;

/// 打击独奏每一小节的 RMS 地板（dBFS）。
const LOOP_BAR_FLOOR_DBFS: f64 = -45.0;

/// 自动化方向性判据：第 1 小节"开"必须比"关"至少小这么多 dB（渐强起点）。
const AUTOMATION_RAMP_IN_DB: f64 = 1.0;

/// 自动化方向性判据：第 4 小节"开"必须比"关"至少大这么多 dB（渐强终点）。
const AUTOMATION_RAMP_OUT_DB: f64 = 0.5;

// ---------------------------------------------------------------------------
// 临时目录与稳定产物目录
// ---------------------------------------------------------------------------

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 一个临时目录，`Drop` 时删除。工程文件与中间母带都落在这里。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |delta| delta.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-creation-{}-{tag}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// 最终产物（WAV + 读数 JSON）放在**稳定**目录：`$YEBAN_ACCEPTANCE_DIR`，否则
/// `$CARGO_TARGET_DIR/acceptance-335`，否则系统临时目录。
///
/// 这个目录在 `target/` 下（未跟踪），因此 WAV **不会**进仓库。
fn artifact_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("YEBAN_ACCEPTANCE_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(target) = std::env::var_os("CARGO_TARGET_DIR") {
        return PathBuf::from(target).join("acceptance-335");
    }
    std::env::temp_dir().join("yeban-acceptance-335")
}

// ---------------------------------------------------------------------------
// 工具面管线（与 `render_master.rs` 同一形状，只走 `tools/call`）
// ---------------------------------------------------------------------------

/// 一个**空**的分发器：工程只能经 `yeban_open_project` 建出来。
fn fresh_dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    (dispatcher, auth)
}

/// 走真实 `tools/call` 管线；工具路径上不允许 JSON-RPC 层错误。
fn call(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "creation-acceptance",
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    let response = outcome.response.expect("必须有响应");
    assert!(
        response.error.is_none(),
        "{name} 不该有 JSON-RPC 层错误: {:?}",
        response.error
    );
    response.result.expect("必须有 result")
}

/// `yeban_edit_notes` → `yeban_merge_proposal`。返回 merge 的响应。
///
/// 工具面的**唯一**编辑路径是"提案 → 合并"（`yeban_edit_notes` 的响应里
/// `pendingMerge` 恒为 true）；本函数把这两步合成一次调用，避免每个编辑点都手抄一遍。
fn edit_and_merge(dispatcher: &mut Dispatcher, auth: &str, arguments: Value, label: &str) -> Value {
    let proposed = call(dispatcher, auth, "yeban_edit_notes", arguments);
    assert_eq!(proposed["status"], "success", "{label}: {proposed}");
    assert_eq!(
        proposed["data"]["pendingMerge"], true,
        "{label}: 编辑必须先进隔离提案: {proposed}"
    );
    let proposal = proposed["data"]["proposal"]["proposalId"]
        .as_str()
        .unwrap_or_else(|| panic!("{label}: 提案必须带 proposalId: {proposed}"))
        .to_owned();
    let merged = call(
        dispatcher,
        auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal, "commitMessage": format!("#335 {label}")}),
    );
    assert_eq!(merged["status"], "success", "{label} 合并: {merged}");
    merged
}

/// `yeban_query_project` 的**全量顶层视图**（`fields` 缺省）。
fn query_project(dispatcher: &mut Dispatcher, auth: &str) -> Value {
    let response = call(dispatcher, auth, "yeban_query_project", json!({}));
    assert_eq!(response["status"], "success", "{response}");
    response["data"]["project"].clone()
}

/// `yeban_render_master` → 落盘 WAV；返回响应。
fn render_master(dispatcher: &mut Dispatcher, auth: &str, path: &Path, label: &str) -> Value {
    let response = call(
        dispatcher,
        auth,
        "yeban_render_master",
        json!({
            "format": "wav",
            "sampleRate": 48_000,
            "path": path.display().to_string(),
        }),
    );
    assert_eq!(response["status"], "success", "{label}: {response}");
    response
}

// ---------------------------------------------------------------------------
// 母带读数（独立解码器；⛔ 不复用生产写入器的读数）
// ---------------------------------------------------------------------------

/// 读 24-bit PCM WAV → `(交织样本, 声道数)`。用 `yeban-render` 的**容器解析**
/// （MCP 母带就是它写的），但样本解码是本地 8 行实现。
fn read_master(path: &Path) -> (Vec<f32>, usize) {
    let bytes = fs::read(path).expect("读母带");
    let parsed = yeban_render::rf64::parse_container(&bytes).expect("母带必须是合法容器");
    let samples: Vec<f32> = bytes[parsed.data.clone()]
        .as_chunks::<3>()
        .0
        .iter()
        .map(|chunk| {
            let raw =
                i32::from(chunk[0]) | (i32::from(chunk[1]) << 8) | (i32::from(chunk[2]) << 16);
            let signed = (raw << 8) >> 8;
            #[allow(clippy::cast_precision_loss)]
            let value = signed as f32 / 8_388_608.0;
            value
        })
        .collect();
    (samples, usize::from(parsed.format.channels))
}

/// 交错样本的峰值（线性满刻度；1.0 = 0 dBFS）。
fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0f32, |acc, sample| acc.max(sample.abs()))
}

/// `[start_frame, start_frame + frames)` 的 RMS（dBFS，两个声道一起算）。
fn window_rms_dbfs(samples: &[f32], channels: usize, start_frame: usize, frames: usize) -> f64 {
    let available = samples.len() / channels;
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for frame in start_frame..(start_frame + frames).min(available) {
        for channel in 0..channels {
            let value = f64::from(samples[frame * channels + channel]);
            sum += value * value;
            count += 1;
        }
    }
    if count == 0 {
        return f64::NEG_INFINITY;
    }
    let mean = sum / count as f64;
    if mean <= 0.0 {
        return f64::NEG_INFINITY;
    }
    20.0 * mean.sqrt().log10()
}

/// 两份**等长**母带的样本差 RMS（dBFS）。
fn difference_rms_dbfs(left: &[f32], right: &[f32]) -> f64 {
    assert_eq!(left.len(), right.len(), "两份母带必须等长才能逐样本比较");
    let sum: f64 = left
        .iter()
        .zip(right)
        .map(|(a, b)| {
            let delta = f64::from(*a) - f64::from(*b);
            delta * delta
        })
        .sum();
    if left.is_empty() {
        return f64::NEG_INFINITY;
    }
    let mean = sum / left.len() as f64;
    if mean <= 0.0 {
        return f64::NEG_INFINITY;
    }
    20.0 * mean.sqrt().log10()
}

/// 左声道切片（判据只量左声道：母带是立体声，主音与打击在两声道都有）。
fn left_channel(samples: &[f32], channels: usize) -> Vec<f32> {
    samples
        .iter()
        .step_by(channels)
        .copied()
        .collect::<Vec<f32>>()
}

/// 单 bin 的 Hann 窗 DFT 幅度（与 `render_master.rs` 的既有判据同一实现口径）。
fn magnitude(samples: &[f32], freq_hz: f64, sample_rate: f64) -> f64 {
    let n = samples.len();
    let mut re = 0.0f64;
    let mut im = 0.0f64;
    for (index, sample) in samples.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let position = index as f64;
        let hann = 0.5 - 0.5 * (core::f64::consts::TAU * position / n as f64).cos();
        let phase = core::f64::consts::TAU * freq_hz * position / sample_rate;
        re += hann * f64::from(*sample) * phase.cos();
        im += hann * f64::from(*sample) * phase.sin();
    }
    #[allow(clippy::cast_precision_loss)]
    let norm = (re * re + im * im).sqrt() / n as f64 * 2.0;
    norm
}

/// 在 `[lo, hi]` 里按 1 Hz 步长找幅度最大的频率（Hz）。
fn dominant(samples: &[f32], lo: f64, hi: f64) -> f64 {
    let mut best = lo;
    let mut best_magnitude = f64::NEG_INFINITY;
    let mut freq = lo;
    while freq <= hi {
        let value = magnitude(samples, freq, 48_000.0);
        if value > best_magnitude {
            best_magnitude = value;
            best = freq;
        }
        freq += 1.0;
    }
    best
}

// ---------------------------------------------------------------------------
// 身份（26 字符 Crockford Base32 ULID；全部由本文件确定性派生）
// ---------------------------------------------------------------------------

/// 一个合法的 26 字符 ULID 文本。`seed` 只占末 4 位，因此同一测试里逐个唯一。
fn eid(seed: u32) -> String {
    let text = format!("01J8ZR{:0>16}{seed:04}", 0);
    assert_eq!(text.len(), 26, "ULID 必须 26 字符");
    text
}

// 片段 / 摆放 / 音符 / 设备 / 段落身份（与蓝图一一对应）。
const CLIP_BASS: u32 = 1001;
const CLIP_HARMONY: u32 = 1002;
const CLIP_LEAD: u32 = 1003;
const PLACEMENT_PERCUSSION: u32 = 1100;
const PLACEMENT_BASS: u32 = 1101;
const PLACEMENT_HARMONY: u32 = 1102;
const PLACEMENT_LEAD: u32 = 1103;
const DEVICE_REVERB: u32 = 3000;
const SECTION_INTRO: u32 = 4001;
const SECTION_DROP: u32 = 4002;

// ---------------------------------------------------------------------------
// 曲子的构造
// ---------------------------------------------------------------------------

/// 一条 `add` op（音符载荷）。
fn add_op(note: Value) -> Value {
    json!({"kind": "add", "note": note})
}

/// 一个音符载荷（`startTick` / `pitch` / `durationTicks` / `velocity`）。
fn note(id: Option<u32>, start_tick: u64, pitch: u8, duration_ticks: u64, velocity: u8) -> Value {
    let mut value = json!({
        "startTick": start_tick,
        "pitch": pitch,
        "durationTicks": duration_ticks,
        "velocity": velocity,
    });
    if let Some(seed) = id {
        value["id"] = Value::from(eid(seed));
    }
    value
}

/// 主音的颤音曲线：每个 960-tick 周期里，+60 音分平台占 240 tick、−60 音分平台占
/// 240 tick，两者之间线性过渡 240 tick、回到 0 再过渡 240 tick。
///
/// 8 个周期 = 7_680 tick；主音音符 A 只有 5_760 tick ⇒ 前 6 个周期完整、最后半个
/// 周期被音符末端截断（模型允许曲线点落在鸣响区间之外，`ADR-0012` D6）。
fn vibrato_curve() -> Vec<[i64; 2]> {
    #[allow(clippy::cast_possible_truncation)]
    let cents = VIBRATO_CENTS as i64;
    let mut points: Vec<[i64; 2]> = vec![[0, 0]];
    for cycle in 0..8u64 {
        #[allow(clippy::cast_possible_wrap)]
        let base = cycle.saturating_mul(VIBRATO_PERIOD) as i64;
        points.push([base + 240, cents]);
        points.push([base + 480, cents]);
        points.push([base + 720, -cents]);
        points.push([base + 960, -cents]);
    }
    points
}

/// 从 `yeban_query_project` 的顶层视图里按名字找一条音轨身份。
fn track_named(project: &Value, name: &str) -> String {
    project["tracks"]
        .as_object()
        .unwrap_or_else(|| panic!("工程必须有 tracks: {project}"))
        .iter()
        .find_map(|(id, track)| (track["name"] == name).then(|| id.clone()))
        .unwrap_or_else(|| panic!("找不到音轨 {name}: {project}"))
}

/// 一条音轨的 `(placementId, clipId)`（创建工程时种子给的那一条）。
fn first_placement(project: &Value, track_id: &str) -> (String, String) {
    let placements = project["tracks"][track_id]["clips"]
        .as_object()
        .unwrap_or_else(|| panic!("音轨 {track_id} 必须有摆放: {project}"));
    let (placement_id, placement) = placements
        .iter()
        .next()
        .unwrap_or_else(|| panic!("音轨 {track_id} 的摆放不能为空"));
    let clip_id = placement["clip_id"]
        .as_str()
        .unwrap_or_else(|| panic!("摆放必须带 clip_id: {placement}"))
        .to_owned();
    (placement_id.clone(), clip_id)
}

/// 全部音符 id（某个片段里的）。
fn note_ids(project: &Value, clip_id: &str) -> Vec<String> {
    project["clip_pool"][clip_id]["content"]["Midi"]["notes"]
        .as_object()
        .map(|notes| notes.keys().cloned().collect())
        .unwrap_or_default()
}

/// 曲子搭好之后要用到的身份。
struct Piece {
    project: Value,
    percussion: String,
    bass: String,
    harmony: String,
    lead: String,
}

/// **只用 MCP 工具调用**把蓝图搭出来，返回最终文档视图与各轨身份。
fn build_piece(dispatcher: &mut Dispatcher, auth: &str, project_path: &Path) -> Piece {
    // ---- 1. 建工程：4 条 MIDI 轨 + 主总线；3/4；124 BPM；种子片段留空（我们要自己写） ----
    let created = call(
        dispatcher,
        auth,
        "yeban_open_project",
        json!({
            "path": project_path.display().to_string(),
            "create": true,
            "title": "#335 acceptance piece",
            "bpm": 124.0,
            "timeSignature": "3/4",
            "seed": {"trackCount": 4, "clipName": "Percussion", "notes": []},
        }),
    );
    assert_eq!(created["status"], "success", "建工程必须成功: {created}");
    assert_eq!(created["data"]["created"], true, "{created}");

    let project = query_project(dispatcher, auth);
    let percussion = track_named(&project, "Track 1");
    let bass = track_named(&project, "Track 2");
    let harmony = track_named(&project, "Track 3");
    let lead = track_named(&project, "Track 4");
    let (seed_placement, percussion_clip) = first_placement(&project, &percussion);
    assert!(
        note_ids(&project, &percussion_clip).is_empty(),
        "种子片段必须是空的（我们不用它的 4 个种子音）"
    );

    // ---- 2. 打击轨：改名、写 1 小节图案、把它摆成 4 小节并循环 4 遍 -------------
    //
    // 先摘掉种子的 1 小节摆放，再用带 `loopEnabled` 的摆放盖到整首曲子上。
    let mut percussion_ops = vec![
        json!({"kind": "setTrackName", "trackId": percussion, "name": "Percussion"}),
        json!({"kind": "setParam", "lane": "TrackVolume", "value": -9.0}),
        json!({"kind": "setTempo", "bpm": 120.0}),
        json!({"kind": "setSection", "section": {
            "create": true,
            "sectionId": eid(SECTION_INTRO),
            "name": "Intro",
            "startTick": 0,
            "endTick": 2 * BAR_TICKS,
        }}),
        json!({"kind": "setSection", "section": {
            "create": true,
            "sectionId": eid(SECTION_DROP),
            "name": "Drop",
            "startTick": 2 * BAR_TICKS,
            "endTick": TOTAL_TICKS,
        }}),
    ];
    // 3/4 的一小节：kick 在拍 1、snare 在拍 2、kick 在拍 3。
    percussion_ops.push(add_op(note(Some(2000), 0, 36, 120, 120)));
    percussion_ops.push(add_op(note(Some(2001), PPQ, 38, 120, 96)));
    percussion_ops.push(add_op(note(Some(2002), 2 * PPQ, 36, 120, 112)));
    edit_and_merge(
        dispatcher,
        auth,
        json!({
            "trackId": percussion,
            "clipId": percussion_clip,
            "ops": percussion_ops,
            "placement": {"kind": "remove", "placementId": seed_placement},
        }),
        "percussion material + doc facts",
    );
    edit_and_merge(
        dispatcher,
        auth,
        json!({
            "trackId": percussion,
            "clipId": percussion_clip,
            "ops": [],
            "placement": {
                "startTick": 0,
                "durationTicks": TOTAL_TICKS,
                "placementId": eid(PLACEMENT_PERCUSSION),
                "loopEnabled": true,
                "loopStartTick": 0,
                "loopEndTick": LOOP_END_TICKS,
            },
        }),
        "percussion loop placement",
    );

    // ---- 3. 贝斯轨：每小节 3 个四分音（根音、根音、五度） ----------------------
    let bass_clip = eid(CLIP_BASS);
    let mut bass_ops = Vec::new();
    for (bar, root) in [(0u64, 45u8), (1, 41), (2, 48), (3, 43)] {
        let base = bar.saturating_mul(BAR_TICKS);
        bass_ops.push(add_op(note(
            Some(2100 + bar as u32 * 3),
            base,
            root,
            PPQ,
            112,
        )));
        bass_ops.push(add_op(note(
            Some(2101 + bar as u32 * 3),
            base + PPQ,
            root,
            PPQ,
            96,
        )));
        bass_ops.push(add_op(note(
            Some(2102 + bar as u32 * 3),
            base + 2 * PPQ,
            root + 7,
            PPQ,
            104,
        )));
    }
    edit_and_merge(
        dispatcher,
        auth,
        json!({
            "trackId": bass,
            "clipId": bass_clip,
            "create": true,
            "clipName": "Bass",
            "ops": bass_ops,
        }),
        "bass clip",
    );
    edit_and_merge(
        dispatcher,
        auth,
        json!({
            "trackId": bass,
            "clipId": bass_clip,
            "ops": [
                {"kind": "setTrackName", "trackId": bass, "name": "Bass"},
                {"kind": "setParam", "lane": "TrackVolume", "value": -12.0},
            ],
            "placement": {
                "startTick": 0,
                "durationTicks": TOTAL_TICKS,
                "placementId": eid(PLACEMENT_BASS),
            },
        }),
        "bass placement",
    );

    // ---- 4. 和声轨：4 个三和弦各占 1 小节 + 混响 ---------------------------------
    let harmony_clip = eid(CLIP_HARMONY);
    let chords: [[u8; 3]; 4] = [[57, 60, 64], [53, 57, 60], [60, 64, 67], [55, 59, 62]];
    let mut harmony_ops = Vec::new();
    for (bar, chord) in chords.iter().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        let bar_index = bar as u64;
        let base = bar_index.saturating_mul(BAR_TICKS);
        for (voice, pitch) in chord.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let seed = 2200 + bar as u32 * 3 + voice as u32;
            harmony_ops.push(add_op(note(Some(seed), base, *pitch, BAR_TICKS, 88)));
        }
    }
    edit_and_merge(
        dispatcher,
        auth,
        json!({
            "trackId": harmony,
            "clipId": harmony_clip,
            "create": true,
            "clipName": "Harmony",
            "ops": harmony_ops,
        }),
        "harmony clip",
    );
    edit_and_merge(
        dispatcher,
        auth,
        json!({
            "trackId": harmony,
            "clipId": harmony_clip,
            "ops": [
                {"kind": "setTrackName", "trackId": harmony, "name": "Harmony"},
                {"kind": "setParam", "lane": "TrackVolume", "value": -15.0},
                {"kind": "insertDevice", "trackId": harmony, "slotIndex": 0, "device": {
                    "deviceId": eid(DEVICE_REVERB),
                    "name": "Hall",
                    "kind": "InternalEffect",
                    "bypassed": false,
                    "latencySamples": 0,
                    "params": [
                        {"name": "reverb_size", "value": 0.8},
                        {"name": "reverb_damp", "value": 0.25},
                        {"name": "reverb_wet", "value": 0.45},
                    ],
                }},
            ],
            "placement": {
                "startTick": 0,
                "durationTicks": TOTAL_TICKS,
                "placementId": eid(PLACEMENT_HARMONY),
            },
        }),
        "harmony placement + reverb",
    );

    // ---- 5. 主音轨：颤音音符 + 滑音音符 ------------------------------------------
    let lead_clip = eid(CLIP_LEAD);
    let mut vibrato_note = note(Some(2300), 0, 69, 2 * BAR_TICKS, 108);
    vibrato_note["pitchBendCurve"] = json!(vibrato_curve());
    let mut slide_note = note(
        Some(2301),
        SLIDE_START_TICK,
        69,
        TOTAL_TICKS - SLIDE_START_TICK,
        104,
    );
    slide_note["slide"] = json!({
        "durationTicks": SLIDE_DURATION_TICKS,
        "targetPitch": 81,
        "curve": "Linear",
    });
    edit_and_merge(
        dispatcher,
        auth,
        json!({
            "trackId": lead,
            "clipId": lead_clip,
            "create": true,
            "clipName": "Lead",
            "ops": [add_op(vibrato_note), add_op(slide_note)],
        }),
        "lead clip (vibrato + slide)",
    );
    edit_and_merge(
        dispatcher,
        auth,
        json!({
            "trackId": lead,
            "clipId": lead_clip,
            "ops": [
                {"kind": "setTrackName", "trackId": lead, "name": "Lead"},
                {"kind": "setParam", "lane": "TrackVolume", "value": -12.0},
            ],
            "placement": {
                "startTick": 0,
                "durationTicks": TOTAL_TICKS,
                "placementId": eid(PLACEMENT_LEAD),
            },
        }),
        "lead placement",
    );

    // ---- 6. 自动化：贝斯轨 TrackVolume 从 −46 dB 渐强到 −6 dB -------------------
    for (tick, value) in [(0u64, -46.0f64), (TOTAL_TICKS, -6.0)] {
        let written = call(
            dispatcher,
            auth,
            "yeban_edit_automation",
            json!({
                "trackId": bass,
                "lane": "TrackVolume",
                "point": {"tick": tick, "value": value},
                "ticks": [0, TOTAL_TICKS],
            }),
        );
        assert_eq!(
            written["status"], "success",
            "写自动化点必须成功: {written}"
        );
        assert_eq!(
            written["data"]["appliedInOfflineRender"], true,
            "TrackVolume 自动化必须在离线母带里真的被应用: {written}"
        );
    }

    let project = query_project(dispatcher, auth);
    Piece {
        project,
        percussion,
        bass,
        harmony,
        lead,
    }
}

/// 找一个源报告（按路由节点身份）。
fn source_of<'a>(render: &'a Value, track_id: &str) -> &'a Value {
    render["data"]["sources"]
        .as_array()
        .unwrap_or_else(|| panic!("母带响应必须有 sources: {render}"))
        .iter()
        .find(|source| source["node"] == track_id)
        .unwrap_or_else(|| panic!("找不到音轨 {track_id} 的源报告: {render}"))
}

// ---------------------------------------------------------------------------
// 判据（一条测试，顺序：文档事实 → 基线母带 → 逐轨 → 主音 → 循环 → 自动化 → 混响）
// ---------------------------------------------------------------------------

#[test]
fn a_four_bar_piece_built_only_from_mcp_tool_calls_renders_measurably() {
    let scratch = Scratch::new("piece");
    let (mut dispatcher, auth) = fresh_dispatcher();
    let project_path = scratch.join("acceptance-piece.yeban");
    let piece = build_piece(&mut dispatcher, &auth, &project_path);

    // =====================================================================
    // A. 文档事实：拍号 / 速度 / 段落 —— 全部经 `yeban_query_project` 读回
    // =====================================================================
    //
    // 量什么：`time_signature.numerator` / `.denominator`（无量纲拍号）、`bpm`（BPM）、
    // 两个段落的 `start_tick` / `end_tick`（tick，960 PPQ）。
    assert_eq!(
        piece.project["time_signature"]["numerator"], 3,
        "拍号分子必须是 3/4 的 3: {}",
        piece.project["time_signature"]
    );
    assert_eq!(
        piece.project["time_signature"]["denominator"], 4,
        "拍号分母必须是 3/4 的 4: {}",
        piece.project["time_signature"]
    );
    assert_eq!(
        piece.project["bpm"], 120.0,
        "`setTempo` 必须把 124 BPM 写成 120: {}",
        piece.project["bpm"]
    );
    let sections = &piece.project["sections"];
    let intro = &sections[eid(SECTION_INTRO)];
    let drop = &sections[eid(SECTION_DROP)];
    assert_eq!(intro["name"], "Intro", "段落名: {intro}");
    assert_eq!(intro["start_tick"], 0, "Intro 起点必须 0 tick: {intro}");
    assert_eq!(
        intro["end_tick"],
        2 * BAR_TICKS,
        "Intro 终点必须 5_760 tick（2 小节 × 2_880）: {intro}"
    );
    assert_eq!(drop["name"], "Drop", "段落名: {drop}");
    assert_eq!(
        drop["start_tick"],
        2 * BAR_TICKS,
        "Drop 起点必须 5_760 tick: {drop}"
    );
    assert_eq!(
        drop["end_tick"], TOTAL_TICKS,
        "Drop 终点必须 11_520 tick（4 小节）: {drop}"
    );

    // 主音的音高形状也必须逐点落进文档（音频判据在下面另有一条）。
    let stored_vibrato =
        &piece.project["clip_pool"][eid(CLIP_LEAD)]["content"]["Midi"]["notes"][eid(2300)];
    assert_eq!(
        stored_vibrato["pitch_bend_curve"],
        json!(vibrato_curve()),
        "颤音曲线必须逐点进文档: {stored_vibrato}"
    );
    let stored_slide =
        &piece.project["clip_pool"][eid(CLIP_LEAD)]["content"]["Midi"]["notes"][eid(2301)];
    assert_eq!(
        stored_slide["slide"],
        json!({"duration_ticks": SLIDE_DURATION_TICKS, "target_pitch": 81, "curve": "Linear"}),
        "滑音必须逐字段进文档: {stored_slide}"
    );

    // =====================================================================
    // B. 基线母带：时长 / 峰值（响度回归判据放到最后，见 H 之前）
    // =====================================================================
    let artifact_dir = artifact_dir();
    fs::create_dir_all(&artifact_dir).expect("建稳定产物目录");
    let master_path = artifact_dir.join("yeban-335-piece.wav");
    let baseline = render_master(&mut dispatcher, &auth, &master_path, "baseline");
    let (baseline_samples, channels) = read_master(&master_path);
    assert_eq!(channels, 2, "母带必须是立体声");

    // 时长：帧数先看响应，再看磁盘文件的真实帧数（不是同一个来源）。
    let reported_frames = baseline["data"]["frames"]
        .as_u64()
        .unwrap_or_else(|| panic!("响应必须带 frames: {baseline}"));
    assert_eq!(
        reported_frames, EXPECTED_FRAMES,
        "母带帧数必须等于蓝图的 11_520 tick × 25 帧/tick = 288_000 帧"
    );
    let file_frames = baseline_samples.len() / channels;
    assert_eq!(
        file_frames as u64, EXPECTED_FRAMES,
        "磁盘文件的真实帧数必须等于 288_000（响应与产物必须一致）"
    );
    let reported_seconds = baseline["data"]["durationSeconds"]
        .as_f64()
        .unwrap_or_else(|| panic!("响应必须带 durationSeconds: {baseline}"));
    assert!(
        (reported_seconds - EXPECTED_SECONDS).abs() < 1.0e-9,
        "时长必须精确等于 6.000 s（288_000 / 48_000），实测 {reported_seconds}"
    );
    assert!(
        (COMPUTED_SECONDS - EXPECTED_SECONDS).abs() < 1.0e-12,
        "蓝图算术自洽：288_000 帧 / 48_000 Hz 必须等于 6.000 s"
    );

    // 峰值：24-bit 文件样本峰值 ≤ 1.0（0 dBFS），且工具自己的削波计数为 0。
    let measured_peak = peak(&baseline_samples);
    let measured_peak_dbfs = 20.0 * f64::from(measured_peak).log10();
    let reported_peak_before = baseline["data"]["peak"]["before"]
        .as_f64()
        .unwrap_or_else(|| panic!("响应必须带 peak.before: {baseline}"));
    let clipped_samples = baseline["data"]["clippedSamples"]
        .as_u64()
        .unwrap_or_else(|| panic!("响应必须带 clippedSamples: {baseline}"));
    let true_peak_dbfs = baseline["data"]["truePeakDbfs"]
        .as_f64()
        .unwrap_or_else(|| panic!("响应必须带 truePeakDbfs: {baseline}"));
    println!(
        "### [335] 基线: frames={reported_frames} seconds={reported_seconds:.6} \
         filePeak={measured_peak:.6} ({measured_peak_dbfs:.3} dBFS) \
         toolPeakBefore={reported_peak_before:.6} clippedSamples={clipped_samples} \
         truePeak={true_peak_dbfs:.3} dBTP"
    );
    assert!(
        measured_peak <= 1.0,
        "24-bit 文件样本峰值必须 ≤ 1.0（0 dBFS），实测 {measured_peak}"
    );
    assert_eq!(
        clipped_samples, 0,
        "母带不许有样本超过满刻度（clippedSamples 必须为 0）"
    );
    assert!(
        true_peak_dbfs <= 0.0,
        "16× 过采样的真峰值必须 ≤ 0 dBTP，实测 {true_peak_dbfs}"
    );

    // =====================================================================
    // C. 逐轨可闻：每条轨拿掉（静音）或单独留下（独奏）后，母带必须真的变
    // =====================================================================
    //
    // 量什么：两次母带的**逐样本差**在整段上的 RMS（dBFS）。差为 −∞ 表示两次
    // 渲染逐样本相同 ⇒ 这条轨对母带没有任何贡献。
    let mut contributions: Vec<(String, f64, String)> = Vec::new();
    for (track_id, name) in [
        (&piece.percussion, "percussion"),
        (&piece.bass, "bass"),
        (&piece.harmony, "harmony"),
        (&piece.lead, "lead"),
    ] {
        // 基线里这条源必须可闻且有电平。
        let source = source_of(&baseline, track_id);
        assert_eq!(source["audible"], true, "{name} 在基线里必须可闻: {source}");
        let source_peak = source["peak"]
            .as_f64()
            .unwrap_or_else(|| panic!("{name} 必须有逐源峰值: {source}"));
        assert!(
            source_peak > 0.001,
            "{name} 的逐源峰值必须大于 0.001（满刻度线性），实测 {source_peak}"
        );

        edit_and_merge(
            &mut dispatcher,
            &auth,
            json!({
                "trackId": track_id,
                "ops": [{"kind": "setTrackMute", "value": true}],
            }),
            &format!("mute {name}"),
        );
        let muted_path = scratch.join(&format!("muted-{name}.wav"));
        let muted = render_master(
            &mut dispatcher,
            &auth,
            &muted_path,
            &format!("muted {name}"),
        );
        edit_and_merge(
            &mut dispatcher,
            &auth,
            json!({
                "trackId": track_id,
                "ops": [{"kind": "setTrackMute", "value": false}],
            }),
            &format!("unmute {name}"),
        );

        let (muted_samples, muted_channels) = read_master(&muted_path);
        assert_eq!(muted_channels, channels, "母带声道数必须一致");
        assert_eq!(
            muted_samples.len(),
            baseline_samples.len(),
            "静音一条轨不许改变母带总帧数（其余轨仍撑满 11_520 tick）"
        );
        let muted_sha = muted["data"]["sha256"]
            .as_str()
            .unwrap_or_else(|| panic!("母带响应必须带 sha256: {muted}"));
        let baseline_sha = baseline["data"]["sha256"]
            .as_str()
            .unwrap_or_else(|| panic!("母带响应必须带 sha256: {baseline}"));
        assert_ne!(
            muted_sha, baseline_sha,
            "静音 {name} 后母带字节必须变（否则这条轨对母带没有任何贡献）"
        );
        let diff = difference_rms_dbfs(&baseline_samples, &muted_samples);
        println!("### [335] 逐轨: {name} mute-diff={diff:.3} dBFS (sha {muted_sha})");
        assert!(
            diff > TRACK_DIFF_FLOOR_DBFS,
            "静音 {name} 与基线的逐样本差 RMS 必须 > {TRACK_DIFF_FLOOR_DBFS} dBFS，实测 {diff:.3}"
        );
        assert_eq!(
            source_of(&muted, track_id)["audible"],
            false,
            "静音后这条源必须被报成不可闻"
        );
        contributions.push((name.to_owned(), diff, muted_sha.to_owned()));
    }
    assert_eq!(contributions.len(), 4, "四条轨必须逐条被量过");

    // =====================================================================
    // D. 打击的循环：独奏打击，逐小节都必须是响的
    // =====================================================================
    //
    // 打击片段只有 1 小节的内容（3 个音），摆放写了 `loopEnabled` 覆盖 4 小节。
    // 量什么：打击独奏母带中每一小节窗口（2_880 tick = 72_000 帧）的 RMS（dBFS）。
    edit_and_merge(
        &mut dispatcher,
        &auth,
        json!({
            "trackId": piece.percussion,
            "ops": [{"kind": "setTrackSolo", "value": true}],
        }),
        "solo percussion",
    );
    let percussion_path = scratch.join("percussion-only.wav");
    render_master(&mut dispatcher, &auth, &percussion_path, "percussion solo");
    edit_and_merge(
        &mut dispatcher,
        &auth,
        json!({
            "trackId": piece.percussion,
            "ops": [{"kind": "setTrackSolo", "value": false}],
        }),
        "unsolo percussion",
    );
    let (percussion_samples, percussion_channels) = read_master(&percussion_path);
    let bar_frames = usize::try_from(BAR_TICKS * FRAMES_PER_TICK).expect("一小节帧数");
    let mut bar_rms = Vec::new();
    for bar in 0..usize::try_from(BARS).expect("小节数") {
        bar_rms.push(window_rms_dbfs(
            &percussion_samples,
            percussion_channels,
            bar * bar_frames,
            bar_frames,
        ));
    }
    println!("### [335] 循环: percussion 逐小节 RMS = {bar_rms:?} dBFS");
    for (bar, value) in bar_rms.iter().enumerate() {
        assert!(
            *value > LOOP_BAR_FLOOR_DBFS,
            "循环必须铺满 4 小节：第 {} 小节的打击 RMS 必须 > {LOOP_BAR_FLOOR_DBFS} dBFS，实测 {value:.3}",
            bar + 1
        );
    }

    // =====================================================================
    // E. 主音的音高：颤音两窗 + 滑音两端（在主音**独奏**的母带上量）
    // =====================================================================
    //
    // 为什么要独奏：`dominant` 找的是整段里最强的频率；只有把另外三条轨拿掉，
    // "主频"才**只**可能是主音。颤音的两窗取在 +60 / −60 音分平台的**中点附近**：
    // 高频窗 [tick 240, 403)、低频窗 [tick 720, 884)，各 4096 帧（≈85 ms）。
    edit_and_merge(
        &mut dispatcher,
        &auth,
        json!({
            "trackId": piece.lead,
            "ops": [{"kind": "setTrackSolo", "value": true}],
        }),
        "solo lead",
    );
    let lead_path = scratch.join("lead-only.wav");
    let lead_render = render_master(&mut dispatcher, &auth, &lead_path, "lead solo");
    edit_and_merge(
        &mut dispatcher,
        &auth,
        json!({
            "trackId": piece.lead,
            "ops": [{"kind": "setTrackSolo", "value": false}],
        }),
        "unsolo lead",
    );
    let (lead_samples, lead_channels) = read_master(&lead_path);
    let lead_left = left_channel(&lead_samples, lead_channels);
    assert!(
        peak(&lead_left) > 0.01,
        "主音独奏母带不能是静音（峰值 {}）",
        peak(&lead_left)
    );
    let high_start = usize::try_from(VIBRATO_HIGH_TICKS * FRAMES_PER_TICK).expect("窗口起点");
    let low_start = usize::try_from(VIBRATO_LOW_TICKS * FRAMES_PER_TICK).expect("窗口起点");
    let high_window = &lead_left[high_start..high_start + PITCH_WINDOW_FRAMES];
    let low_window = &lead_left[low_start..low_start + PITCH_WINDOW_FRAMES];
    let high_hz = dominant(high_window, 405.0, 480.0);
    let low_hz = dominant(low_window, 405.0, 480.0);
    println!(
        "### [335] 颤音: high-window={high_hz:.3} Hz (期望 {EXPECTED_HIGH_HZ:.3}), \
         low-window={low_hz:.3} Hz (期望 {EXPECTED_LOW_HZ:.3}), delta={:.3} Hz",
        high_hz - low_hz
    );
    assert!(
        (high_hz - EXPECTED_HIGH_HZ).abs() <= 2.0,
        "+60 音分平台的窗口主频必须 ≈{EXPECTED_HIGH_HZ:.1} Hz，实测 {high_hz} Hz"
    );
    assert!(
        (low_hz - EXPECTED_LOW_HZ).abs() <= 2.0,
        "−60 音分平台的窗口主频必须 ≈{EXPECTED_LOW_HZ:.1} Hz，实测 {low_hz} Hz"
    );
    assert!(
        high_hz - low_hz > 25.0,
        "两个窗口的主频必须真的不同（颤音幅度 ±60 音分 ⇒ 期望差 ≈30 Hz），实测差 {} Hz",
        high_hz - low_hz
    );

    // 滑音：起点窗 [5_760+80, 5_760+240) tick；末段窗在滑音走完后的最后一小节里。
    let slide_start_frame =
        usize::try_from(SLIDE_START_TICK * FRAMES_PER_TICK).expect("滑音起点窗");
    let slide_end_frame = usize::try_from(EXPECTED_FRAMES - 16_000).expect("滑音末段窗");
    let slide_start_hz = dominant(
        &lead_left[slide_start_frame..slide_start_frame + PITCH_WINDOW_FRAMES],
        420.0,
        470.0,
    );
    let slide_end_hz = dominant(
        &lead_left[slide_end_frame..slide_end_frame + 8_000],
        850.0,
        910.0,
    );
    println!(
        "### [335] 滑音: start={slide_start_hz:.3} Hz (期望 ≈440), \
         end={slide_end_hz:.3} Hz (期望 {SLIDE_TARGET_HZ:.3})"
    );
    assert!(
        (slide_start_hz - LEAD_HZ).abs() <= 10.0,
        "滑音起点窗主频必须 ≈440 Hz（±10 Hz；窗口覆盖线性滑音的头 85 ms），实测 {slide_start_hz} Hz"
    );
    assert!(
        slide_end_hz - slide_start_hz > 400.0,
        "滑音必须真的把音高抬上去（起点≈440、终点≈880），实测 {} → {} Hz",
        slide_start_hz,
        slide_end_hz
    );
    assert!(
        (slide_end_hz - SLIDE_TARGET_HZ).abs() <= 3.0,
        "滑音走完后最后一小节必须保持在 880 Hz，实测 {slide_end_hz} Hz"
    );
    assert_ne!(
        lead_render["data"]["sha256"], baseline["data"]["sha256"],
        "主音独奏与全曲母带必须不同（否则独奏没生效）"
    );

    // =====================================================================
    // F. 自动化：贝斯轨的 −46→−6 dB 渐强必须在母带上**有方向地**听得出来
    // =====================================================================
    //
    // A/B 的做法：把同一条泳道的 `readEnabled` 关掉（静态 −12 dB 生效），再渲染一次。
    // 量什么：两次母带在**第 1 小节**与**第 4 小节**的 RMS（dBFS）。
    // 方向：第 1 小节"开"更小（−46 dB 压住贝斯）；第 4 小节"开"更大（−6 dB 抬起来）。
    edit_and_merge(
        &mut dispatcher,
        &auth,
        json!({
            "trackId": piece.bass,
            "ops": [{"kind": "setAutomationLane", "lane": {
                "lane": "TrackVolume",
                "readEnabled": false,
            }}],
        }),
        "automation lane off",
    );
    let automation_off_path = scratch.join("automation-off.wav");
    render_master(
        &mut dispatcher,
        &auth,
        &automation_off_path,
        "automation off",
    );
    edit_and_merge(
        &mut dispatcher,
        &auth,
        json!({
            "trackId": piece.bass,
            "ops": [{"kind": "setAutomationLane", "lane": {
                "lane": "TrackVolume",
                "readEnabled": true,
            }}],
        }),
        "automation lane on",
    );
    let (automation_off_samples, automation_off_channels) = read_master(&automation_off_path);
    assert_eq!(automation_off_channels, channels);
    let bar1_on = window_rms_dbfs(&baseline_samples, channels, 0, bar_frames);
    let bar1_off = window_rms_dbfs(
        &automation_off_samples,
        automation_off_channels,
        0,
        bar_frames,
    );
    let bar4_start = 3 * bar_frames;
    let bar4_on = window_rms_dbfs(&baseline_samples, channels, bar4_start, bar_frames);
    let bar4_off = window_rms_dbfs(
        &automation_off_samples,
        automation_off_channels,
        bar4_start,
        bar_frames,
    );
    println!(
        "### [335] 自动化: bar1 on={bar1_on:.3} off={bar1_off:.3} dBFS; \
         bar4 on={bar4_on:.3} off={bar4_off:.3} dBFS"
    );
    assert!(
        bar1_on < bar1_off - AUTOMATION_RAMP_IN_DB,
        "自动化开启时第 1 小节必须比关闭时至少小 {AUTOMATION_RAMP_IN_DB} dB（−46 dB 起点），\
         实测 开 {bar1_on:.3} / 关 {bar1_off:.3} dBFS"
    );
    assert!(
        bar4_on > bar4_off + AUTOMATION_RAMP_OUT_DB,
        "自动化开启时第 4 小节必须比关闭时至少大 {AUTOMATION_RAMP_OUT_DB} dB（−6 dB 终点），\
         实测 开 {bar4_on:.3} / 关 {bar4_off:.3} dBFS"
    );

    // =====================================================================
    // G. 混响：把和声轨的混响旁通，母带必须真的变
    // =====================================================================
    //
    // 先确认设备链真的报告了一台**激活的** reverb（结构），再做 A/B（音频）。
    let engine_state = call(
        &mut dispatcher,
        &auth,
        "yeban_query_engine_state",
        json!({"trackId": piece.harmony}),
    );
    assert_eq!(engine_state["status"], "success", "{engine_state}");
    let track_state = &engine_state["data"]["track"];
    let inserts = track_state["inserts"]
        .as_array()
        .unwrap_or_else(|| panic!("设备链读数: {track_state}"));
    assert_eq!(inserts.len(), 1, "和声轨必须有 1 台设备: {track_state}");
    assert_eq!(inserts[0]["active"], true, "混响必须激活: {inserts:?}");
    assert!(
        inserts[0]["effects"]
            .as_array()
            .is_some_and(|effects| effects.iter().any(|effect| effect == "reverb")),
        "设备链读数必须认出 reverb: {inserts:?}"
    );
    edit_and_merge(
        &mut dispatcher,
        &auth,
        json!({
            "trackId": piece.harmony,
            "ops": [{"kind": "setDeviceBypassed", "trackId": piece.harmony,
                     "deviceId": eid(DEVICE_REVERB), "bypassed": true}],
        }),
        "bypass reverb",
    );
    let bypassed_path = scratch.join("reverb-bypassed.wav");
    let bypassed = render_master(&mut dispatcher, &auth, &bypassed_path, "reverb bypassed");
    edit_and_merge(
        &mut dispatcher,
        &auth,
        json!({
            "trackId": piece.harmony,
            "ops": [{"kind": "setDeviceBypassed", "trackId": piece.harmony,
                     "deviceId": eid(DEVICE_REVERB), "bypassed": false}],
        }),
        "restore reverb",
    );
    let (bypassed_samples, bypassed_channels) = read_master(&bypassed_path);
    assert_eq!(bypassed_channels, channels);
    let reverb_diff = difference_rms_dbfs(&baseline_samples, &bypassed_samples);
    println!(
        "### [335] 混响: bypass-diff={reverb_diff:.3} dBFS (sha {})",
        bypassed["data"]["sha256"]
    );
    assert_ne!(
        bypassed["data"]["sha256"], baseline["data"]["sha256"],
        "旁通混响后母带字节必须变"
    );
    assert!(
        reverb_diff > REVERB_DIFF_FLOOR_DBFS,
        "旁通混响与基线的逐样本差 RMS 必须 > {REVERB_DIFF_FLOOR_DBFS} dBFS，实测 {reverb_diff:.3}"
    );

    // 响度：ITU-R BS.1770-4 门限积分，单位 LUFS。
    let integrated_lufs = baseline["data"]["loudness"]["measuredIntegratedLufs"]
        .as_f64()
        .unwrap_or_else(|| panic!("响应必须带实测响度: {baseline}"));
    println!("### [335] 基线: integratedLufs={integrated_lufs:.4} LUFS");
    // 回归带：首次跑通后把实测值钉成 `EXPECTED_LUFS`（见文件末尾的读数表）。
    let expected_lufs = EXPECTED_LUFS;
    assert!(
        (integrated_lufs - expected_lufs).abs() <= LUFS_TOLERANCE_LU,
        "母带积分响度必须在钉住的回归带 {expected_lufs} ± {LUFS_TOLERANCE_LU} LU 内，\
         实测 {integrated_lufs} LUFS"
    );

    // =====================================================================
    // H. 稳定产物：把 WAV 的路径 / 字节 / sha256 / 峰值 / 响度写出来
    // =====================================================================
    let bytes = fs::metadata(&master_path).expect("产物必须存在").len();
    let sha256 = yeban_model::AssetHash::of_bytes(&fs::read(&master_path).expect("读产物"))
        .as_str()
        .to_owned();
    assert_eq!(
        bytes,
        baseline["data"]["bytes"].as_u64().expect("响应 bytes"),
        "产物字节数必须等于响应里的 bytes"
    );
    assert_eq!(
        sha256,
        baseline["data"]["sha256"].as_str().expect("响应 sha256"),
        "产物的 sha256 必须等于响应里的 sha256"
    );
    let mut report = serde_json::Map::new();
    report.insert(
        "path".to_owned(),
        Value::from(master_path.display().to_string()),
    );
    report.insert("bytes".to_owned(), Value::from(bytes));
    report.insert("sha256".to_owned(), Value::from(sha256.clone()));
    report.insert("frames".to_owned(), Value::from(EXPECTED_FRAMES));
    report.insert("seconds".to_owned(), json!(EXPECTED_SECONDS));
    report.insert("peakLinear".to_owned(), json!(measured_peak));
    report.insert("peakDbfs".to_owned(), json!(measured_peak_dbfs));
    report.insert("clippedSamples".to_owned(), json!(clipped_samples));
    report.insert("truePeakDbfs".to_owned(), json!(true_peak_dbfs));
    report.insert("integratedLufs".to_owned(), json!(integrated_lufs));
    report.insert("trackContributionsDbfs".to_owned(), json!(contributions));
    report.insert("vibratoHighHz".to_owned(), json!(high_hz));
    report.insert("vibratoLowHz".to_owned(), json!(low_hz));
    report.insert("slideStartHz".to_owned(), json!(slide_start_hz));
    report.insert("slideEndHz".to_owned(), json!(slide_end_hz));
    report.insert("loopBarRmsDbfs".to_owned(), json!(bar_rms));
    report.insert(
        "automationBar1Dbfs".to_owned(),
        json!({"on": bar1_on, "off": bar1_off}),
    );
    report.insert(
        "automationBar4Dbfs".to_owned(),
        json!({"on": bar4_on, "off": bar4_off}),
    );
    report.insert("reverbDiffDbfs".to_owned(), json!(reverb_diff));
    let report_path = artifact_dir.join("yeban-335-piece-report.json");
    fs::write(
        &report_path,
        serde_json::to_vec_pretty(&Value::Object(report)).expect("序列化读数"),
    )
    .expect("写读数文件");
    println!(
        "### [335] 产物: {} ({} bytes, sha256 {sha256})",
        master_path.display(),
        bytes
    );
    println!("### [335] 读数: {}", report_path.display());
    println!(
        "### [335] 逐轨贡献: {}",
        contributions
            .iter()
            .map(|(name, diff, _)| format!("{name}={diff:.3} dBFS"))
            .collect::<Vec<String>>()
            .join(", ")
    );

    // =====================================================================
    // W. 墙（工具面让这首曲子难做/做不了的地方）—— 编译期常量，供人读
    // =====================================================================
    println!("### [335] 墙清单（{} 条，逐条见 WALLS）", WALLS.len());
    for wall in WALLS {
        println!("### [335] WALL {wall}");
    }
}

/// **墙清单**：每一堵都是本次真的撞到的（工具名 / 实参 / 精确错误或观察 / 绕法），
/// 按"多疼"从高到低排。⛔ 不要为了让清单好看而删条目。
///
/// 这份清单是判据的一部分（`assert!(WALLS.len() >= 5)`）：工具面改善之后，作者应当
/// **主动**回来删掉对应的墙，而不是让它们烂在文档里。
pub const WALLS: [&str; 8] = [
    "① 创建片段（`create:true`）与摆放（`placement`）必须分两次调用 + 两次\
     `yeban_merge_proposal`：同给是 `INVALID_PARAMETER_RANGE` / `placementIsNotCreation`\
     （`domain/mod.rs` 的 `plan_edit_notes`）。4 条轨因此要 **8** 次提案合并才搭得完\
     （本文件实测：每轨 \"建材料\" + \"摆放\" 各一次，第一轨的种子摆放还要先 `remove`）。\
     绕法：严格按\"先建材料、再单独摆放\"拆开。",
    "② 一条 `yeban_edit_notes` 调用只有一个顶层 `trackId`：`setTrackMute` / `setParam` /\
     `setTrackSolo` 都只作用于它。想把三条轨一起静音（做\"主音独奏\"）没有一次调用能表达——\
     本文件用 `setTrackSolo` 绕开（solo 的语义是\"其余全部让路\"），否则要 3 次提案 + 3 次合并。",
    "③ `yeban_edit_notes` 的 `ops` 里**没有**改 `ClipPlacement.loop_config` 的 op：改时长\
     有 `setClipPlacementDuration`，但循环区间/开关只存在于 `placement` 参数的 `add` 形态。\
     本文件绕法：`placement:{kind:\"remove\"}` 摘掉种子摆放，再单独\
     `placement:{kind:\"add\", loopEnabled:true, loopStartTick:0, loopEndTick:2880}` 重摆。\
     想改一条**已有**摆放的循环，只能 remove + add 两步。",
    "④ 设备链**没有 CRUD 工具**：装/拆/改旁通只能经 `yeban_edit_notes` 的\
     `insertDevice` / `removeDevice` / `setDeviceBypassed` op；`yeban_query_engine_state`\
     是只读的。本文件按这条唯一通路装混响并用它做 A/B。",
    "⑤ 段落的建/改/删**没有专用工具**，全部挤在 `yeban_edit_notes` 的 `setSection` /\
     `removeSection` op 里；`create:true` 时必须由调用方自己编一个 26 字符 `sectionId`。\
     想按名字找段落只能拉全量 `yeban_query_project` 再自己遍历 `sections`（没有\"按名字\"\
     或\"按 tick 区间\"的查询）。本文件在第一次编辑批里连建两个段落，省掉两次往返。",
    "⑥ 主音的颤音曲线只能整体写（`add.note.pitchBendCurve` 或 `setNotePitchShape` 的\
     `newPitchBendCurve`），**没有**逐点增删的 op；改一个点要整条重写。本文件的曲线是\
     构造期生成的 33 点数组，一次写入；拍号/速度那类顶层字段则只能经 `yeban_open_project`\
     的 `timeSignature`（模型 `Op` 全集没有写拍号的变体——这是文档里承认的缺口）。",
    "⑦ 两种提交模型混用：`yeban_edit_notes` 是\"提案 + `yeban_merge_proposal`\"，而\
     `yeban_edit_automation` 是**直接提交**（`OpOrigin::McpEdit`，没有提案、没有 merge）。\
     本文件因此有 8 次 merge（搭曲子）和 2 次无 merge 的自动化写点；把一次编辑从 `ops`\
     挪到自动化工具，响应里就少一整套 `proposal` / `pendingMerge` 字段。",
    "⑧ `yeban_render_master` 的 `sources[].peak` 已经给了**一次渲染**里的逐源峰值\
     （本文件用它证明每轨在基线里可闻），但\"拿掉这条轨后母带变了多少\"没有读数：\
     逐轨**贡献**只能靠 N 次静音/独奏渲染 + 本地逐样本比较（本文件 4 次静音 + 2 次独奏）。\
     这不是工具错误，是测量成本；它说明工具面没有\"分轨贡献/差值\"的一眼读法。",
];

/// 首次跑通后钉住的母带积分响度（LUFS，ITU-R BS.1770-4 门限积分）。
///
/// ⚠ 这是**回归锚**，不是"音乐上正确的响度"：它由本工具链的这一次渲染实测后照抄，
/// 用来抓"1 dB 以上的混音/推子漂移"。见模块文档的阈值说明。
const EXPECTED_LUFS: f64 = -12.74;
