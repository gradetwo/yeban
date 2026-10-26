//! **`#335` 验收：只用 MCP 工具面做一首曲子，然后量它。**
//!
//! ⭐ `#341` 在本文件上补了**内容判据**（贝斯每小节根音 / 和声每小节三和弦 /
//! 打击每次起音的位置）—— `#335` 只对主音做过音高内容检查，其余三轨只被量过
//! "有没有声、在哪几小节有声"。见下方 `#341` 一节。
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
//! | 贝斯内容 | 独奏贝斯母带每小节**拍 1** 窗的 DFT 主频 | Hz → 音分 | 期望 `440×2^((p−69)/12)`（MIDI 45/41/48/43 = A2/F2/C3/G2），**±30 音分**；见 `#341` 一节 |
//! | 和声内容 | 独奏和声母带（**混响保持开启**）每小节整小节窗的单频幅度 | dBFS（等效正弦幅度） | 三个和弦音各 `≥ −40 dBFS` 且比同窗非和弦探针高 `≥20 dB`；探针 `≤ −50 dBFS` |
//! | 打击起音 | 独奏打击母带每小节三次击打的起音帧 | 帧 / tick（1 tick = 25 帧） | 5 ms RMS 窗、5 ms 跳步、局部峰值 −20 dB 的首个越限窗，误差 `≤ ±240 帧 = ±5 ms` |
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
//! - **贝斯内容**的 ±30 音分不是"调音精度"判据：`dominant` 的 1 Hz 网格在 F2 上
//!   就是 ±9.9 音分，30 音分 ≈ 3 个量化格。它抓得住任何 ≥30 音分的移调（半音
//!   100 / 五度 700 / 八度 1200），抓不住 <30 音分的走音（F2 上 <1.5 Hz），也不
//!   评价这个音在音乐上是否是"对的贝斯音"。
//! - **和声内容**的探针音是每小节根音 + 1 个半音（A#3 / F#3 / C#4 / G#3），与三个
//!   和弦音的基频和奇次谐波都不重合。混响**不旁通**（内容判据量的是这条轨在曲子里
//!   实际发出的声）；"存在"由绝对下限 + 与探针的分离度两条一起判，"不存在"由探针
//!   的绝对上限单独判 —— 否则宽带混响就能满足"存在"。探针只是**每小节一个频点**的
//!   抽查，不是"所有非和弦音都为零"的全称判据。
//! - **打击起音**只判位置，不判音色：36/38 只差 2 个半音（65.4/73.4 Hz），5 ms 检测窗
//!   分辨不了，所以"这一击是 kick 还是 snare"落在"仍不能抓"里；力度与音色也不判。
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
//! ## `#341` 内容判据：为什么能证明"出的是该出的声"，以及它抓不到什么
//!
//! `#335` 的缺口（它自己在"阈值会漏掉什么"里就写了）：除了主音的颤音/滑音，**没有
//! 任何一条判据检查音高内容**。贝斯被移调一个八度、和声换掉和弦、打击挪半拍，
//! 时长 / 峰值 / 响度 / 逐轨静音差 / 循环逐小节 RMS / 段落 / 自动化 / 混响**全都
//! 照样绿**。本单补的就是这三条 —— 主音的音高 `#335` 已经量过（颤音两窗、滑音两端），
//! 它既是当时**唯一**的内容判据，也正是另外三条成为缺口的原因：先例已经证明"这件事
//! 能测"，只是没有推广到其它轨。
//!
//! 方法（曲子是合成的、渲染是确定性的 ⇒ 加窗 DFT + 峰值/幅度比较是合法的）：
//!
//! 1. **隔离**：贝斯与和声各自 `setTrackSolo` 后重新渲染；打击直接复用 D 节已有的
//!    独奏母带（不多渲染一次）。⛔ 不尝试用频谱分离四条轨 —— 那会在"哪条轨的哪个
//!    泛音"上引入歧义。
//! 2. **贝斯**：每小节拍 1 的音符内部取 16_384 帧窗（跳过 50 ms 起振，长 341 ms），
//!    以 1 Hz 步长找 DFT 主频，再换算成相对 `440×2^((p−69)/12)` 的音分差。搜索带是
//!    期望频率的 0.45×…2.2×：升/降八度都落在带内，而 3 次谐波（3×）被排除在外
//!    （HOLLOW 是奇次谐波波表、没有 2 次谐波 ⇒ 带内最强的峰只能是基频）。
//! 3. **和声**：每小节整小节 72_000 帧窗，在三个期望和弦音的**精确频率**上量单 bin
//!    幅度；同窗再量一个非和弦探针（根音 + 1 个半音）。混响保持开启，探针就是这条轨
//!    的"混响/泄漏地板"。
//! 4. **打击**：5 ms RMS 窗、5 ms 跳步扫描；局部峰值 −20 dB 的首个越限窗左沿即起音。
//!    期望起音都在 5 ms 网格上，所以检测没有网格误差；允差 ±5 ms 只是"哪个窗先越限"。
//!
//! ### 注入证明（判据真的会咬）
//!
//! 每条判据都做过"**只改 `build_piece` 的蓝图、不改测量**"的注入，先红后还原
//! （还原后 `sha256` 与 `git show HEAD:<path>` 相同；本轮实际哈希见本单报告）：
//!
//! | 注入（只改蓝图） | 红在哪 |
//! | :--- | :--- |
//! | 贝斯根音全部 +12 半音（八度） | 贝斯内容：主频翻倍（≈220 / 174.6 / 261.6 / 196 Hz）⇒ Δ ≈ +1200 音分 |
//! | 和声 bar1 三和弦整体 +12 半音（`[69,72,76]`） | 和声内容：期望的 A3/C4/E4 频点消失（"存在"那条） |
//! | 和声 bar1 额外加一个 A#3（探针音） | 和声内容：探针幅度 > −50 dBFS（"不存在"那条） |
//! | 打击 snare 从 tick 960 挪到 1200 | 打击起音：起音帧 30_000 ≠ 24_000（误差 6_000 帧 = 125 ms） |
//!
//! 上表四条**不会**让 `#335` 的任何旧断言变红：它们只改内容，不改时长 / 峰值 /
//! 响度 / 逐轨差 / 循环 RMS / 段落 / 自动化 / 混响。这就是"绿 ≠ 对"的机械版本。
//!
//! ### 本机实测读数（`#341`，stable，48 kHz / 24-bit；由判据自己打印）
//!
//! | 量 | 实测 | 期望 / 阈值 |
//! | :--- | :--- | :--- |
//! | 贝斯根音 | **110.5 / 87.3 / 130.9 / 98.1 Hz**（+7.9 / −0.4 / +0.7 / +1.8 音分） | 110.000 / 87.307 / 130.813 / 97.999 Hz，±30 音分 |
//! | 和声和弦音 | bar1 −25.18 / −24.77 / −25.48；bar2 −26.46 / −25.18 / −24.76；bar3 −24.76 / −25.48 / −25.85；bar4 −25.56 / −25.89 / −25.65 dBFS | 各 ≥ −40 dBFS 且比探针高 ≥20 dB |
//! | 和声探针 | bar1 −101.87 / bar2 −86.31 / bar3 −78.05 / bar4 −81.59 dBFS（分离 ≥76.4 / 59.8 / 52.2 / 55.7 dB） | ≤ −50 dBFS |
//! | 打击起音 | 12 次全部**误差 0 帧**（绝对 tick 0/960/1920/2880/…/10560） | 每小节 0/960/1920 tick，±240 帧（±5 ms） |
//!
//! 贝斯在 bar1 的 +7.9 音分是**测量网格**造成的：`dominant` 从 `110×0.45 = 49.5`
//! 起按 1 Hz 步进，110.000 不在网格上，最近的 110.5 胜出；它没有 1 Hz 以上的
//! 判据含义（真实基频由引擎的 `note_to_hz` 决定）。其余三格都在 ±2 音分内。
//!
//! ### 这些新判据仍不能抓（⛔ 别把绿当成品味判决）
//!
//! - **节奏感 / 律动**：起音判据只证明三次击打落在 tick 0 / 960 / 1920。它不证明这个
//!   节奏型好听，也不判 <5 ms 的微时序（蓝图没有，判据也没有）。
//! - **音色 / 配器**：打击的 36/38 在 5 ms 检测窗里不可分；判据不证明它是 kick/snare，
//!   更不证明音色合适。贝斯/和声也只判基频，不判泛音、力度、连奏。
//! - **声部排列（voicing）与额外音**：和声判据只检查"这三个频点在不在"、探针那一个
//!   频点干不干净。它抓不住"在期望频点之外又叠了一个八度"或"另加一个恰好避开探针的
//!   和弦外音"—— 那些仍会绿。它也完全不评价声部间距/转位是否音乐上合适。
//! - **和弦是否是"对的"和弦**：判据对的是**蓝图**（Am/F/C/G），不是**品味**。一个在
//!   音乐上更妙的进行，只要不是蓝图里的那四个，判据就会红；反过来，蓝图本身平庸时
//!   判据全绿。它验证的是"实现忠实于意图"，⛔ 不是"意图本身正确"。
//! - **混音 / 平衡**：内容判据只看单轨独奏里的频率与位置，不看四条轨之间的平衡、
//!   声场、动态 —— 那仍然要靠响度回归带与听感。
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
// `#341` 内容判据：**期望表**与阈值
//
// ⚠ 期望表是**独立于 `build_piece` 的字面量**，不是从构造代码里 import 的常量。
// 理由就是本单的注入实验：把蓝图的贝斯移调一个八度、把某个和弦换掉、把打击挪 240
// tick，期望表必须**不动**，判据才会红。若两边共用一个常量，改蓝图会同时改期望，
// 判据就是空转的。
// ---------------------------------------------------------------------------

/// 贝斯每小节**拍 1** 的根音（MIDI 音号；蓝图：A2 / F2 / C3 / G2）。
const EXPECTED_BASS_ROOTS: [u8; 4] = [45, 41, 48, 43];

/// 和声每小节的**三和弦**（MIDI 音号；蓝图：Am / F / C / G）。
const EXPECTED_HARMONY_CHORDS: [[u8; 3]; 4] =
    [[57, 60, 64], [53, 57, 60], [60, 64, 67], [55, 59, 62]];

/// 打击每小节的**三次击打**：`(小节内 tick, MIDI 音号)`（kick 36 / snare 38 / kick 36）。
const EXPECTED_PERCUSSION_HITS: [(u64, u8); 3] = [(0, 36), (960, 38), (1920, 36)];

/// 贝斯根音的测量窗起点（相对小节起点，帧）：96 tick = 50 ms，跳过 5 ms 起振。
const BASS_WINDOW_OFFSET_FRAMES: usize = 2_400;

/// 贝斯根音的测量窗长度（帧）：16_384 帧 ≈ 341 ms；F2（87.31 Hz）上约 29.7 个周期。
///
/// 窗在**拍 1 的音符内部**（该音符长 960 tick = 24_000 帧）：2_400 + 16_384 = 18_784 < 24_000。
const BASS_WINDOW_FRAMES: usize = 16_384;

/// 贝斯根音允差（音分）：**±30 音分**。
///
/// 频率换算 `f = 440×2^((p−69)/12)`：A2=110.000、F2=87.307、C3=130.813、G2=97.999 Hz。
/// ±30 音分 = A2 ±1.91 Hz、F2 ±1.51 Hz、C3 ±2.27 Hz、G2 ±1.70 Hz。`dominant` 的
/// 搜索步长是 1 Hz（网格量化最多 ±0.5 Hz = F2 上 ±9.9 音分），30 音分 ≈ 3 个量化格。
///
/// **会抓**：任何 ≥30 音分的移调 —— 八度（1200）、五度（700）、半音（100）都在内，
/// 也就是"贝斯被移调了"这一类缺陷。
/// **不会抓**：<30 音分的走音/漂移（F2 上 <1.5 Hz），例如合成器整体调音偏 20 音分；
/// 也不评价这个音在音乐上是否是"对的贝斯音"（见模块文档末的"仍不能抓"）。
const BASS_TOLERANCE_CENTS: f64 = 30.0;

/// 和声内容判据的窗长（帧）：**整小节** = 2_880 tick = 72_000 帧 = 1.5 s。
///
/// 窗从 bar 起点开始：三个和弦音都从 bar 起点起振并持续整小节。0.667 Hz 的 bin 宽
/// 使三个和弦音（相隔 ≥3 个半音）互不重叠，1 Hz 的探针也不会被相邻音的主瓣扫到。
const HARMONY_WINDOW_FRAMES: usize = 72_000;

/// 一个期望和弦音**存在**的绝对下限：等效正弦幅度 ≥ −40 dBFS。
///
/// 实测和弦音在 −24.8…−26.5 dBFS（见读数表），−40 留了 ≥13.5 dB 余量；它排除了
/// "只有混响底噪"这一档，但单靠它还不够（见下一条）。
const HARMONY_TONE_PRESENCE_DBFS: f64 = -40.0;

/// 期望和弦音必须比**同小节同长度窗的探针音**（根音 + 1 个半音）高这么多 dB。
///
/// 混响会把和弦能量抹到邻频；"某频点有能量"因此不能证明该和弦音存在。探针频点
/// 与三个和弦音的**基频和奇次谐波**都不重合（HOLLOW 波表只有奇次谐波：1/3/5/7/9/11），
/// 所以它上面的能量只可能来自混响/泄漏。实测分离度 52…77 dB，取 20 dB 作门限。
const HARMONY_TONE_ABOVE_PROBE_DB: f64 = 20.0;

/// 探针音**不存在**的绝对上限：等效正弦幅度 ≤ −50 dBFS。
///
/// 实测探针在 −78…−102 dBFS；−50 留了 ≥28 dB 余量。该门限与上一条一起，才使
/// "和弦音存在"不能靠宽带混响满足。
const HARMONY_PROBE_ABSENCE_DBFS: f64 = -50.0;

/// 打击起音检测的 RMS 窗长（帧）：240 = 5 ms。
const ONSET_WINDOW_FRAMES: usize = 240;

/// 打击起音检测的跳步（帧）：240 = 5 ms。所有期望起音（960 tick 的倍数 ⇒ 24_000 帧
/// 的倍数）都落在跳步网格上，因此检测值可以精确等于期望值。
const ONSET_HOP_FRAMES: usize = 240;

/// 打击起音搜索半径（帧）：12_000 = 480 tick = 半拍。半拍内不会碰到相邻击打。
const ONSET_SEARCH_FRAMES: usize = 12_000;

/// 起音判定门限：局部峰值 RMS − **20 dB**。5 ms 线性 attack 使起音后第一个 5 ms
/// 窗已经比稳态低不超过约 6 dB（实测见读数表），20 dB 既能在 attack 内触发，又不会被
/// 静音段的数值噪声触发。
const ONSET_RELATIVE_DB: f64 = 20.0;

/// 允许的起音误差（帧）：±240 帧 = ±5 ms = ±9.6 tick。
///
/// 检测窗的量化误差是 0（起音都在 5 ms 网格上）；±5 ms 是给"第几个窗越过门限"留的
/// 一个窗宽。它抓得住"挪 240 tick"（=6_000 帧，是允差的 25 倍），抓不住 <5 ms 的
/// 微时序（那本来也不在蓝图里）。
const ONSET_TOLERANCE_FRAMES: usize = 240;

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

/// MIDI 音号 → 频率（Hz）：`f = 440 × 2^((p − 69) / 12)`（十二平均律；A4 = MIDI 69 = 440 Hz）。
///
/// 与引擎 `yeban_dsp::math::note_to_hz` 同一个公式；本文件不 import 模型/引擎，
/// 只按蓝图算术独立算一遍。
fn midi_hz(pitch: u8) -> f64 {
    440.0 * 2f64.powf((f64::from(pitch) - 69.0) / 12.0)
}

/// 两个频率的音分差：`1200 × log2(measured / expected)`；正 = 实测偏高。
fn cents(measured_hz: f64, expected_hz: f64) -> f64 {
    1_200.0 * (measured_hz / expected_hz).log2()
}

/// 单 bin 的 Hann 窗 DFT 幅度 → **等效正弦幅度**（dBFS；1.0 = 满刻度）。
///
/// `magnitude` 对幅度 `A`、正好落在 bin 中心的正弦返回 `A/2`（Hann 窗均值 ≈1/2，
/// 归一化系数是 `2/N`），因此这里乘 2 才回到"若这个频点只有一个正弦，它的幅度是多少"。
/// 单位口径：这一频点的等效正弦幅度 / 满刻度，取 20·log10。
fn tone_dbfs(samples: &[f32], freq_hz: f64) -> f64 {
    let amplitude = 2.0 * magnitude(samples, freq_hz, 48_000.0);
    if amplitude <= 0.0 {
        f64::NEG_INFINITY
    } else {
        20.0 * amplitude.log10()
    }
}

/// 一次打击的**起音帧**（帧号；1 tick = 25 帧）。
///
/// 规则（先写成字，再写成代码）：
/// 1. 在 `[expected − ONSET_SEARCH_FRAMES, expected + ONSET_SEARCH_FRAMES]` 里以
///    `ONSET_HOP_FRAMES`（5 ms）为跳步、`ONSET_WINDOW_FRAMES`（5 ms）为窗算 RMS（dBFS）；
/// 2. 先取区间内**最大**窗 RMS 作为这次击打的局部峰值（击打是自己的，不跨击打）；
/// 3. 再取**第一个** RMS ≥ 局部峰值 − `ONSET_RELATIVE_DB`（20 dB）的窗**左沿**作为起音。
///
/// 搜索区间从静音开始（上一击的 50 ms release 早已走完），所以"第一个过门限的窗"
/// 不会落在上一击的尾巴上。区间也到不了下一击（间距 24_000 帧，半径 12_000 帧）。
fn detect_onset_frame(samples: &[f32], channels: usize, expected_frame: usize) -> usize {
    let start = expected_frame.saturating_sub(ONSET_SEARCH_FRAMES);
    let end = expected_frame + ONSET_SEARCH_FRAMES;
    let mut local_peak = f64::NEG_INFINITY;
    let mut hop = start;
    while hop + ONSET_WINDOW_FRAMES <= end {
        local_peak = local_peak.max(window_rms_dbfs(samples, channels, hop, ONSET_WINDOW_FRAMES));
        hop += ONSET_HOP_FRAMES;
    }
    assert!(
        local_peak.is_finite() && local_peak > f64::NEG_INFINITY,
        "期望帧 {expected_frame} ± {ONSET_SEARCH_FRAMES} 内没有能量（局部峰值 {local_peak} dBFS）"
    );
    let threshold = local_peak - ONSET_RELATIVE_DB;
    let mut hop = start;
    while hop + ONSET_WINDOW_FRAMES <= end {
        let rms = window_rms_dbfs(samples, channels, hop, ONSET_WINDOW_FRAMES);
        if rms >= threshold {
            return hop;
        }
        hop += ONSET_HOP_FRAMES;
    }
    panic!("期望帧 {expected_frame} ± {ONSET_SEARCH_FRAMES} 内没有窗越过 {threshold:.3} dBFS");
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
    // 一小节的帧数：2_880 tick × 25 帧/tick = 72_000 帧（后面的内容判据与小节 RMS 共用）。
    let bar_frames = usize::try_from(BAR_TICKS * FRAMES_PER_TICK).expect("一小节帧数");

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
    // C2. `#341` 内容：贝斯每小节的根音（独奏贝斯母带）
    // =====================================================================
    //
    // 量什么：每小节**拍 1** 的音符中部 16_384 帧窗里的 DFT 主频（Hz），换算成相对
    // `EXPECTED_BASS_ROOTS` 期望频率的音分差。为什么只取拍 1：第 3 拍是根音 +7（五度），
    // 整小节窗的主频可能在根音与五度之间摆；拍 1 的窗里只有根音。为什么独奏：`dominant`
    // 找整段最强频率，只有把另外三条轨拿掉，"主频"才只可能是贝斯（与 C/E 同一做法）。
    edit_and_merge(
        &mut dispatcher,
        &auth,
        json!({
            "trackId": piece.bass,
            "ops": [{"kind": "setTrackSolo", "value": true}],
        }),
        "solo bass (content)",
    );
    let bass_content_path = scratch.join("bass-content.wav");
    render_master(&mut dispatcher, &auth, &bass_content_path, "bass content");
    edit_and_merge(
        &mut dispatcher,
        &auth,
        json!({
            "trackId": piece.bass,
            "ops": [{"kind": "setTrackSolo", "value": false}],
        }),
        "unsolo bass (content)",
    );
    let (bass_content_samples, bass_content_channels) = read_master(&bass_content_path);
    let bass_left = left_channel(&bass_content_samples, bass_content_channels);
    assert!(
        peak(&bass_left) > 0.001,
        "贝斯独奏母带不能是静音（峰值 {}）",
        peak(&bass_left)
    );
    let mut bass_bar_hz = Vec::new();
    let mut bass_bar_cents = Vec::new();
    for (bar, root) in EXPECTED_BASS_ROOTS.iter().enumerate() {
        let expected_hz = midi_hz(*root);
        let window_start = bar * bar_frames + BASS_WINDOW_OFFSET_FRAMES;
        let window = &bass_left[window_start..window_start + BASS_WINDOW_FRAMES];
        // 搜索带覆盖期望频率的 ±1 个多八度（低到 0.45×、高到 2.2×）：升/降八度、
        // 换和弦音都能落在带内，测出来的是**实际主频**，不是"期望附近的噪声"。
        let measured_hz = dominant(window, expected_hz * 0.45, expected_hz * 2.2);
        let delta_cents = cents(measured_hz, expected_hz);
        println!(
            "### [341] 贝斯: bar{} root MIDI {root} = 期望 {expected_hz:.3} Hz，\
             实测主频 {measured_hz:.1} Hz（Δ {delta_cents:+.1} 音分，允差 ±{BASS_TOLERANCE_CENTS}）",
            bar + 1
        );
        assert!(
            delta_cents.abs() <= BASS_TOLERANCE_CENTS,
            "第 {} 小节的贝斯根音必须是 MIDI {root} = {expected_hz:.3} Hz（±{BASS_TOLERANCE_CENTS} 音分），\
             实测主频 {measured_hz:.1} Hz（Δ {delta_cents:+.1} 音分）",
            bar + 1
        );
        bass_bar_hz.push(measured_hz);
        bass_bar_cents.push(delta_cents);
    }

    // =====================================================================
    // C3. `#341` 内容：和声每小节的三和弦（独奏和声母带，混响**保持开启**）
    // =====================================================================
    //
    // 怎么处理混响：蓝图给和声轨装了一台 `reverb_wet = 0.45` 的 Freeverb，且本判据
    // 不旁通它 —— 内容判据要量的是"这条轨在曲子里实际发出的声"。混响会把和弦能量
    // 抹宽，于是"某频点有能量"本身不能证明和弦音存在。判据因此同时量一个**同小节、
    // 同长度窗、不在和弦里**的探针音（根音 + 1 个半音）：该频点与三个和弦音的基频和
    // **奇次谐波**（HOLLOW 波表只有 1/3/5/7/9/11 次）都不重合，上面的能量只可能来自
    // 混响/泄漏。存在的判据 = 和弦音绝对幅度 ≥ −40 dBFS 且比探针高 ≥20 dB；不存在的
    // 判据 = 探针 ≤ −50 dBFS。两条一起，宽带混响无法满足"存在"。
    assert_eq!(
        HARMONY_WINDOW_FRAMES, bar_frames,
        "和声内容窗必须与一小节等长（帧）"
    );
    edit_and_merge(
        &mut dispatcher,
        &auth,
        json!({
            "trackId": piece.harmony,
            "ops": [{"kind": "setTrackSolo", "value": true}],
        }),
        "solo harmony (content)",
    );
    let harmony_content_path = scratch.join("harmony-content.wav");
    render_master(
        &mut dispatcher,
        &auth,
        &harmony_content_path,
        "harmony content",
    );
    edit_and_merge(
        &mut dispatcher,
        &auth,
        json!({
            "trackId": piece.harmony,
            "ops": [{"kind": "setTrackSolo", "value": false}],
        }),
        "unsolo harmony (content)",
    );
    let (harmony_content_samples, harmony_content_channels) = read_master(&harmony_content_path);
    let harmony_left = left_channel(&harmony_content_samples, harmony_content_channels);
    assert!(
        peak(&harmony_left) > 0.001,
        "和声独奏母带不能是静音（峰值 {}）",
        peak(&harmony_left)
    );
    let mut harmony_tones_dbfs: Vec<Vec<f64>> = Vec::new();
    let mut harmony_probes_dbfs: Vec<f64> = Vec::new();
    for (bar, chord) in EXPECTED_HARMONY_CHORDS.iter().enumerate() {
        let window = &harmony_left[bar * HARMONY_WINDOW_FRAMES..(bar + 1) * HARMONY_WINDOW_FRAMES];
        let probe_pitch = chord[0] + 1;
        let probe_hz = midi_hz(probe_pitch);
        let probe_dbfs = tone_dbfs(window, probe_hz);
        // "不存在"先判：这是"存在"不能靠宽带混响满足的那一半。
        assert!(
            probe_dbfs <= HARMONY_PROBE_ABSENCE_DBFS,
            "第 {} 小节的非和弦探针 MIDI {probe_pitch}（{probe_hz:.3} Hz）必须不存在：\
             等效正弦幅度必须 ≤ {HARMONY_PROBE_ABSENCE_DBFS} dBFS，实测 {probe_dbfs:.2} dBFS",
            bar + 1
        );
        let mut tones = Vec::new();
        for pitch in chord {
            let expected_hz = midi_hz(*pitch);
            let dbfs = tone_dbfs(window, expected_hz);
            let separation_db = dbfs - probe_dbfs;
            println!(
                "### [341] 和声: bar{} MIDI {pitch} = {expected_hz:.3} Hz，实测 {dbfs:.2} dBFS；\
                 探针 MIDI {probe_pitch} = {probe_hz:.3} Hz 处 {probe_dbfs:.2} dBFS（分离 {separation_db:.1} dB）",
                bar + 1
            );
            assert!(
                dbfs >= HARMONY_TONE_PRESENCE_DBFS,
                "第 {} 小节的和弦音 MIDI {pitch}（{expected_hz:.3} Hz）必须存在：\
                 等效正弦幅度必须 ≥ {HARMONY_TONE_PRESENCE_DBFS} dBFS，实测 {dbfs:.2} dBFS",
                bar + 1
            );
            assert!(
                separation_db >= HARMONY_TONE_ABOVE_PROBE_DB,
                "第 {} 小节的和弦音 MIDI {pitch} 必须比非和弦探针高 ≥{HARMONY_TONE_ABOVE_PROBE_DB} dB：\
                 实测和弦音 {dbfs:.2} dBFS / 探针 {probe_dbfs:.2} dBFS（分离 {separation_db:.1} dB）",
                bar + 1
            );
            tones.push(dbfs);
        }
        println!(
            "### [341] 和声: bar{} 和弦音 dBFS = {tones:?}，探针 {probe_dbfs:.2} dBFS",
            bar + 1
        );
        harmony_tones_dbfs.push(tones);
        harmony_probes_dbfs.push(probe_dbfs);
    }

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

    // ---- 起音位置（`#341` 内容判据，复用上面这份打击独奏母带，不再多渲染一次）----
    //
    // 量什么：每次击打的**起音帧**（帧号；1 tick = 25 帧，1 帧 = 1/48 ms）。
    // 期望：蓝图每小节的 (0, kick 36) / (960, snare 38) / (1920, kick 36)，循环 4 遍。
    // 检测规则见 `detect_onset_frame`（5 ms RMS 窗、5 ms 跳步、局部峰值 −20 dB 的首个
    // 越限窗）。MIDI 36/38 只用于读数：5 ms 检测窗分辨不了相差 2 个半音的 65.4/73.4 Hz，
    // 所以本判据只判**位置**，不判"这一击是 kick 还是 snare"（见模块文档"仍不能抓"）。
    let mut percussion_onset_ticks = Vec::new();
    for bar in 0..usize::try_from(BARS).expect("小节数") {
        for (local_tick, pitch) in EXPECTED_PERCUSSION_HITS {
            let bar_tick = u64::try_from(bar).expect("小节下标") * BAR_TICKS + local_tick;
            let expected_frame = usize::try_from(bar_tick * FRAMES_PER_TICK).expect("期望起音帧");
            let detected_frame =
                detect_onset_frame(&percussion_samples, percussion_channels, expected_frame);
            let error_frames = detected_frame.abs_diff(expected_frame);
            let detected_tick = detected_frame as f64 / FRAMES_PER_TICK as f64;
            let error_ms = error_frames as f64 / 48.0;
            println!(
                "### [341] 打击: bar{} tick {local_tick}（MIDI {pitch}）→ 起音帧 \
                 {detected_frame} = tick {detected_tick:.1}（误差 {error_frames} 帧 = {error_ms:.2} ms）",
                bar + 1
            );
            assert!(
                error_frames <= ONSET_TOLERANCE_FRAMES,
                "第 {} 小节 tick {local_tick} 的击打必须落在这里（±{ONSET_TOLERANCE_FRAMES} 帧 = ±5 ms），\
                 实测起音帧 {detected_frame}（误差 {error_frames} 帧 = {error_ms:.2} ms）",
                bar + 1
            );
            percussion_onset_ticks.push(detected_tick);
        }
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
    // `#341` 内容判据的读数（单位：Hz / 音分 / dBFS / tick）。
    report.insert("bassBarHz".to_owned(), json!(bass_bar_hz));
    report.insert("bassBarCents".to_owned(), json!(bass_bar_cents));
    report.insert("harmonyBarTonesDbfs".to_owned(), json!(harmony_tones_dbfs));
    report.insert("harmonyProbeDbfs".to_owned(), json!(harmony_probes_dbfs));
    report.insert(
        "percussionOnsetTicks".to_owned(),
        json!(percussion_onset_ticks),
    );
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
/// 这份清单是判据的一部分（数组长度即契约）：工具面改善之后，作者应当
/// **主动**回来删掉对应的墙，而不是让它们烂在文档里。
///
/// ⭐ 原 WALL ①（`create:true` 与 `placement` 必须分两次调用，同给报
/// `placementIsNotCreation`）已由 `ADR-0018` 消除：单次提案即可原子生成
/// `Op::AddClip` + `Op::AddClipPlacement`。⭐ 原 WALL ②（一次调用只有一个顶层
/// `trackId`）已由可选的 `ops[].trackId` 消除（判据
/// `edit_notes_per_op_track_id_batches_three_tracks_in_one_proposal`）。
/// ⭐ 原 WALL ⑤（弯音曲线只能整体写）已由 `ADR-0019` 消除：`setPitchBendPoint`
/// op 可逐点插入 / 改值 / 删除（判据
/// `edit_notes_set_pitch_bend_point_inserts_updates_and_deletes_one_point`）。
/// ⭐ 原 WALL ⑦（`yeban_render_master` 没有"分轨贡献/差值"读数）已由 `ADR-0020`
/// 消除：`contributions: true` 让响应带 `data.contributions[]`（两次**未归一化**
/// 母带逐样本差的 RMS / 峰值，dBFS）。⚠ 本文件 C 节**继续**用 4 次静音 + 2 次独奏
/// 独立测量 —— 有意为之：工具自己的读数不能当自己的 oracle；两者的一致性由
/// `render_master.rs` 的 `contributions_match_an_independent_mute_render` 钉住。
/// 四条均从清单移除（编号保持历史原样）。
pub const WALLS: [&str; 3] = [
    "③ 设备链**没有 CRUD 工具**：装/拆/改旁通只能经 `yeban_edit_notes` 的\
     `insertDevice` / `removeDevice` / `setDeviceBypassed` op；`yeban_query_engine_state`\
     是只读的。本文件按这条唯一通路装混响并用它做 A/B（注：原先的'改已有摆放循环'\
     已在 `#336` 由 `setClipPlacementLoop` 彻底消除）。",
    "④ 段落的建/改/删**没有专用工具**，全部挤在 `yeban_edit_notes` 的 `setSection` /\
     `removeSection` op 里；`create:true` 时必须由调用方自己编一个 26 字符 `sectionId`。\
     想按名字找段落只能拉全量 `yeban_query_project` 再自己遍历 `sections`（没有\"按名字\"\
     或\"按 tick 区间\"的查询）。本文件在第一次编辑批里连建两个段落，省掉两次往返。",
    "⑥ 两种提交模型混用：`yeban_edit_notes` 是\"提案 + `yeban_merge_proposal`\"，而\
     `yeban_edit_automation` 是**直接提交**（`OpOrigin::McpEdit`，没有提案、没有 merge）。\
     本文件因此有 8 次 merge（搭曲子）和 2 次无 merge 的自动化写点；把一次编辑从 `ops`\
     挪到自动化工具，响应里就少一整套 `proposal` / `pendingMerge` 字段。",
];

/// 首次跑通后钉住的母带积分响度（LUFS，ITU-R BS.1770-4 门限积分）。
///
/// ⚠ 这是**回归锚**，不是"音乐上正确的响度"：它由本工具链的这一次渲染实测后照抄，
/// 用来抓"1 dB 以上的混音/推子漂移"。见模块文档的阈值说明。
const EXPECTED_LUFS: f64 = -12.74;
