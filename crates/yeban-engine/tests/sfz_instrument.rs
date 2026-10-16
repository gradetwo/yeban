//! SFZ 缺口 1 的**步 2／步 3**：引擎侧声部（[`SfzVoicePool`]）＋ 快照形状（[`SfzPlan`]）
//! ＋ ⭐ **多采样 ＋ 绑定注入 API**。
//!
//! 设计裁决（`docs/ledger/phase-status.md` 的 R698「甲-1」）：`TrackSlot` 增
//! `sfz: SfzVoicePool`（`[SfzVoice; 16]`、全 `Copy`）＋ 快照侧只读 [`PreparedSfz`]
//! ＋ `render_track` 加第 3 参 `sfz: Option<&PreparedSfz>`（与既有
//! `Option<&NoteSchedule>` 对称）。分派次序 ＝ **鼓机 ＞ SFZ ＞ 复音**。
//!
//! # 量什么（对象 ＋ 单位，先说再跑）
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | S1 | 引擎声部路径 == 摊平表路径 | **逐样本最大绝对差**（`max_i |flat_i − engine_i|`）＝ **0**，并配"两侧都真的出声"的反向对照 |
//! | S1b | ⭐ **逐块拼接的整音符**（`#66` 补做 2） | 8 × 128 帧拼接 vs 摊平表**一次性 1024 帧**：逐样本最大绝对差 ＝ **0**，且"有声块 ≥ 2"（跨块见证；⛔ 挡住"提前退休"注入） |
//! | S2 | 终点口径 ＝ 样本自然结束 | 播完那一块之后 `sfz_active_voices == 0`（退休）＋ 下一块**逐位静音**，而摊平表**每一块都从源 0 重播**（≠ 0） |
//! | S3 | ⭐ **显式 `span` 终点**（步 3 后两侧同口径） | 引擎与摊平表**逐样本 0**、都写在 `end` 处停（**写入帧数 == 190/200**）；反向对照：改 `end` ⇒ 帧数变 |
//! | S4 | ⭐ `end=-1`（`SampleEnd::Silent`） | 引擎**一帧不写**、池子 0 声部；摊平表**也**一帧不写（两侧同口径）；反向对照：去掉 `end=-1` ⇒ 摊平表出声 |
//! | S5 | ⭐ **反向对照（关键）** | **不给 `sfz`** 时端到端输出的 **FNV-1a 指纹**（逐位）等于**接线前**实测值 |
//! | S6 | 端到端非静音 ＋ 真的换了音源 | 非零样本数 `> 0` ＋ 指纹与"不给 SFZ"**不同** |
//! | S7 | 分派次序 | 鼓机 ＋ SFZ 同轨的输出对"只有鼓机"**逐位相同**；而无鼓机时 SFZ 支路**真的出声** |
//! | S8 | 池满（> 16 条匹配区域） | **窃取**（⛔ 不丢弃）：`created == 16` ＋ `stolen == 1` ＋ `dropped == 0`；33 条 ⇒ 挂起者被换掉 ＝ **真丢弃**（`stolen == 17` ＋ `dropped == 16`）；未满 ⇒ 两个计数都 0 |
//! | S9 | 尺寸 | `size_of` 读数 ＋ `EngineRuntime < 192 KiB`（与 `synth_rt_zero_alloc.rs:2570` 同一门槛） |
//! | S10 | ⭐ **换音源 / 定位必须释放 SFZ 池**（`#66` 补做 2） | `sfz_active_voices`：触发后 **2** → 换音源后/`seek` 后 **0**；换音源那一块**复音非零 > 0**；切回 SFZ 那一块**非零 ＝ 0** |
//! | ⭐ S11 | **多采样各播自己**（步 3） | 两条区域两条不同 `.wav` ⇒ 两条各自与"单独一条 + 它自己那份样本"的**逐位相同**，且两条之间**指纹不同**；反向对照：**对调**提供表 ⇒ 指纹变；广播版在 key=64 上播的是 a 的内容（**内容错**的机械证据） |
//! | ⭐ S12 | **同一样本引两次只解码一次**（步 3） | `sample_count() == 1` ＋ `pcm_bytes() == 1 × 480 × 4`；反向对照：两条**不同**路径 ⇒ `2` / `2 × 1920`；提供表同一路径两份 ⇒ 首个为准（内容仍是第一份） |
//! | ⭐ S13 | **`start`/`end` 真正应用**（步 3） | 离线 vs 摊平**逐样本 0**；`offset=10` ⇒ 输出第 0 帧 **逐位等于** `源[10] × gain`；写入帧数 **190**；反向对照：`offset=50` ⇒ 指纹与波形都变 |
//! | ⭐ S14 | **单样本重载的兼容性**（步 3） | `prepare`（广播）在 span-free 夹具上的 FNV-1a 指纹 ＝ **步 2 实测** `0xa933ddab286bdf5e`；`is_broadcast()` / `sample_content_label() == "broadcast(1)"` |
//! | ⭐ S15 | **缺样本 ⇒ 静音 ＋ 计数**（⭐ 本单改写；归属裁决 `R711` 的 `needs-7`） | 缺 `missing.wav`（提供表 `[a.wav]`）⇒ `silent_region_count() == 1`、`missing_sample_paths() == ["missing.wav"]`、`sample_content = distinct(1)+silent(1)`、`sample_index == NO_SAMPLE`、该键**一帧不写**（⛔ 不再回退到 `samples[0]`）；反向对照①：补上 ⇒ 该键真的出声且**逐位**是自己的内容；反向对照②：key=60 那条区域**照常出声**（静音作用域只到缺样本的区域）；空提供表 ⇒ `silent = 2`、写入 **0** 帧、`missing = ["a.wav","missing.wav"]`（排序去重） |
//! | ⭐ S16 | **绑定注入 API 经公开路径可用（多采样）**（步 3） | `SfzPlan::prepare_with_samples` → `EngineSnapshot::with_sfz` → `render_snapshot`：非零 > 0、指纹 ≠ 不给 SFZ；反向对照：广播 plan 的端到端指纹与多采样版**不同** |
//! | ⭐ S17 | ⭐ **轨推子变化 N dB ⇒ SFZ 与鼓机输出同比例变**（本单新增；`needs-1`） | 两臂（同一份工程、只差音源）`peak(−6 dB)/peak(0 dB)` 都等于 `track_gain(−6.0)`（相对误差 ≤ 1e-5）且**互相**相等；`mute = true` ⇒ 两臂**逐位** 0；另一条轨独奏 ⇒ 两臂**逐位** 0。⚠ 量的是 `render_track` 输出（母线限制器前）|
//! | ⭐ S18 | ⭐ **引擎层⛔ 不乘第二次力度**（本单新增；`needs-1` 的防双重计腿） | `v ∈ {0,32,64,100,127}`：引擎声部路径与**乐器层**一次性渲染**逐位相同**（手工调度表的 `gain` 显式写成 `velocity_gain(v)`）；反向对照：`velocity_gain(127) == 1.0`（恒等）⇒ S1 的 127 那条腿**抓不住**双重计 |
//! | ⭐ S19 | ⭐ **池满 ⇒ 窃取 ＋ 3 ms 淡出**（本单新增；`sfz-needs-2`） | 池满后再触发：`stolen == 1`、`dropped == 0`、`fade_remaining == 144`（3 ms @ 48 kHz）；淡出窗口里槽位仍宣称**被窃取者**、走完 144 帧后就地换成新声部（`note` 变）；输出读数：淡出臂的第 143 帧 / 第 0 帧 ＝ `FLOOR^(143/144)`；**硬窃取对照臂**（`steal_fade_frames = 0`）⇒ 即时换人、指纹不同 |
//! | ⭐ S20 | ⭐ **三件同轨的可读信号**（本单新增；`sfz-needs-3`） | 鼓机 ＋ SFZ ＋ 复音同轨 ⇒ `armed_sources == 3` ＋ `ignored_sources == 2`（输出仍对"只有鼓机"**逐位相同**）；反向对照：鼓机＋复音 ⇒ `2 / 1`、只有复音 ⇒ `1 / 0` |
//! | ⭐ S21 | ⭐ **反向对照（任务单 `#78` 的核心）** | `TrackV3.sampler = None` ⇒ 端到端 **FNV-1a 指纹**仍逐位等于**接线前** [`BASELINE_PLAIN`]／[`BASELINE_TUNED`]，且 `sfz_pending()` **空** |
//! | ⭐ S22 | ⭐ **正向（`#78`）** | `sampler = Some(路径)` ⇒ `from_project` 的快照里出现**该轨**的待摊平项 `PendingSfz`（路径**逐字**相同、条数 1）＋ ⛔ **摊平表 `sfz()` 仍空**（引擎⛔ 不凭空摊平） |
//! | ⭐ S23 | ⭐ **待摊平项是纯事实（`#78`）** | 路径**磁盘上不存在**也不报错、⛔ 不改渲染（同夹具 `None`／`Some` 指纹**逐位相同**）；`with_sfz` 注入摊平表 ⇒ 待摊平项被**取代**（表清空、`sfz()` 1 条、指纹变） |
//! | ⭐ S24 | ⭐ **缺样本口径不变，且经新接线可达（`#78`）** | 由 `sfz_pending()` 给出的路径 → 控制侧解码（**只给 `a.wav`**）⇒ `silent_region_count() == 1`、`missing_sample_paths() == ["missing.wav"]`、`sample_index == NO_SAMPLE`、`render_track` 该键**不建声部**（`sfz_active_voices == 0`、逐位 0） |
//! | ⭐ S25 | ⭐ **摊平表字节读数拆开 ＋ 预算判据**（本单新增；`sfz-needs-8` 的"可判"那一半） | **字节**：区域表 ＝ `size_of::<PreparedRegion>() × 3`、PCM ＝ `4 × 2400`、路径文本 ＝ `10`、合计 ＝ `memory_bytes()`；预算 `N = SfzByteBudget::for_layout(3, 2400, 10)` ⇒ 三项**各自** `bytes ≤ N`、超出读数**全 0**；⭐ **内置反向腿**：三个维度**各自** −1 字节 ⇒ 该维度恰好报 **1** 字节超出、`is_within() == false` |
//! | ⭐ S26 | ⭐ **超预算会怎样：引擎只报数**（本单新增；`sfz-needs-8` 的口径） | 真的超预算的臂（预算按"只给 `a.wav`"声明 ⇒ PCM 超 **7680** 字节）：`prepare_with_samples` 仍 **`Ok`**（⛔ 不报错）、读数逐项（区域表 0 ／ PCM 7680 ／ 路径 0）；⭐ **摊平结果逐位不变**：`budget_overrun` 前后的 `byte_usage`／`sample_count`／`pcm_bytes`／`region_count`／**渲染输出**全部相同（零预算 ⇒ 三项全报超，仍不改表） |
//! | ⭐ S27 | ⭐ **1 GiB 级音色库今天可判**（本单新增；`sfz-needs-8` 的动机） | ⛔ **不真的分配 1 GiB**：`SfzByteUsage::new(区域表, 1 GiB, 路径)` 对 `SfzByteBudget::new(区域表, 512 MiB, 路径)` 报出**恰好** `536 870 912` 字节超出（`is_within() == false`）；反向腿：上界给**恰好 1 GiB** ⇒ **通过**（闭区间，⛔ 判据不是"永远红"） |
//! | ⭐ S28 | ⭐ **循环周期逐样本精确**（任务 `#91` 新增；`sfz-needs-6` 的"循环"那一半） | **源采样下标序列**（斜坡夹具 `pcm[i] = (i+1)/1024` ⇒ 输出可**逐帧反解下标**）：`loop_continuous`、窗口 `[100,164)` ⇒ 下标序列**逐帧**等于 `t<100 ? t : 100 + (t−100) mod 64`（**0 处不符**）；⭐ **实测最小周期**（首个窗口段重复位置）**恰好** `64 ＝ loop_end − loop_start`；反向腿：窗口段必须 ≥ 2 个不同位模式（⛔ 防空转） |
//! | ⭐ S29 | ⭐ **`no_loop` / `one_shot` 都不循环 ＋ `one_shot` 忽略 note-off**（任务 `#91` 新增） | 两份夹具**都写了** `loop_start/loop_end`：`no_loop` ⇒ 非零 **399**（播到样本尾后**逐位静音**，⛔ 不重复）；`one_shot` ⇒ 非零 **399**（同样不循环）；⭐ `one_shot` ＋ `end_sample=100` ⇒ 仍 **399**（**忽略 note-off**）；⭐ **反向腿**：`no_loop` ＋ `end_sample=100` ⇒ 恰好 **100**（证明 `end_sample` 真的在切，⛔ 空绿） |
//! | ⭐ S30 | ⭐ **字段真的被播放层读到**（任务 `#91` 新增；§4.3 的常驻腿） | 两份**只差 `loop_end` 一个数**的夹具（`164` / `200`）⇒ `PreparedRegion::loop_end()` 与 `loop_window()` 逐字回读 ＋ **实测周期分别为 64 / 100**、下标序列各自**逐帧精确** ⇒ 字段错一位（或只"搬了个字段"而没接线）**必红** |
//! | ⭐ S31 | ⭐ **输入政策不变：非法取值仍响亮 `Err`**（任务 `#91` 新增；第 259 条） | 引擎路径上：集合外 `loop_mode` ⇒ `SfzError::InvalidOption`；越界 `loop_start` ⇒ `SfzError::IntegerOutOfRange`（⛔ **不是**"降级 ＋ 记录"）；⭐ 唯一例外臂：`loop_end <= loop_start`（两值各自合法、组合退化）⇒ `load` **`Ok`** ＋ `loop_window() == None` ＝ **按不循环渲染**（既有语义，⛔ 不加新机制） |
//! | ⭐ S32 | ⭐ **`loop_sustain` ≡ `loop_continuous` 的登记可判**（任务 `#91` 新增） | `include_str!("../src/sfz_instrument.rs")` 必须含字面量"`note-off 后两者的差别（走完当前循环再进释放）属 ADSR 片、本片不声称已实现`"；反向腿：文档里**没有**的那句 ⇒ `contains` 必须 `false`（⛔ 判据不是恒真） |
//! | ⭐ S33 | ⭐ **不带任何 `loop_*` 的既有夹具逐位不变**（任务 `#91` 新增；§4.4） | 既有夹具 [`TWO_MATCH_ONE_MISS`] 经 `render_track` 实时路径 512 帧 @ `(60,127)` 的 **FNV-1a 指纹** ＝ **改动前实测** `0xa933ddab286bdf5e`（基线探针：`flat` 与 `engine` 两条路同值、`max\|Δ\| = 0`）；⭐ **反向腿**：同一输出乘 `1.0000001` ⇒ 指纹**必须变**（证明它是逐位判据、⛔ 不是"有输出就算过"） |
//! | ⭐ S34 | ⭐ **五个 `ampeg_*` 的解析层五条腿 ＋ `Err` 政策 ＋ 缺失 ⇒ 缺省 ＋ 帧换算**（任务 `#92` 新增） | 秒 / 百分比读数逐字回读（`1.5s` / `250ms` / 裸值 / `50%` / `2s`）；`engine_hz == 48 kHz`；⭐ `0.01 s × 48 kHz` ⇒ **480 帧**（四舍五入，⛔ 不是截断 479）；非法三类 ⇒ `InvalidFloat` / `InvalidDuration` / `FloatOutOfRange`（＋ 同批数据里的合法样例**必须被接受**）；缺失 ⇒ 五个读数全在透明缺省（`release == 0`，⛔ 不是规范的 `0.001`）＋ `ampeg_is_transparent()` ＋ `ampeg_run() == None` |
//! | ⭐ S35 | ⭐ **包络形状逐样本可判**（任务 `#92` 新增） | 恒定 `1.0` 样本 ⇒ 输出**恰好**是包络电平；容差口径 ＝ **线性段每一点 `\|实测 − 理论\| ≤ 1e-6`**（实测最大 **5.96e-8**）；起音**恰好**在第 `A = 480` 帧到峰、衰减终点 ＝ `sustain = 0.5`、note-off 后**恰好** `R = 960` 帧归零且之后逐位 0；⭐ 段位读数逐块 `Attack → Hold → Decay → Sustain → None`（状态真的在推进，⛔ 不是假读数） |
//! | ⭐ S36 | ⭐ **`loop_sustain` 与 `loop_continuous` 在 note-off 之后输出不同**（任务 `#92` 新增；⭐ **任务 `#94` 按规范改写期望**） | 两臂**各配一条**"它自己该怎样"：`loop_continuous` ⇒ note-off 就地**进释放**且**释放期照旧回绕** ⇒ 非零**恰好 680**（200 ＋ 480；全程逐点 ＝「循环 × 斜坡」）；`loop_sustain` ⇒ 进释放、释放期**⛔ 不循环**（源下标逐帧 +1、永不回绕）⇒ 非零**恰好 463**（"释放走完"与"到样本尾"**谁先到谁结束** ⇒ 样本尾先到）；反向腿：两条腿的非零数**不同**（680 ≠ 463）＋ 输出越过 `end_sample` ＋ 理论值由 `pcm` ＋ 斜坡**独立算出** |
//! | ⭐ S37 | ⭐ **无 `ampeg_*` 的既有夹具逐位不变**（任务 `#92` 新增；§4.4） | 两臂（`loop_continuous` ＋ `end_sample = 200`、`no_loop` ＋ `end_sample = u64::MAX`）经实时池 512 帧的 **FNV-1a 指纹** ＝ **改动前实测** `0xa9dec8a87f69106c` / `0xd3c2a82076bd8947`（基线探针在 base `1beffe2a`、工作树干净时测得，测后删除）；⭐ **反向腿**：同一输出乘 `1.0000001` ⇒ 指纹**必须变** |
//! | ⭐ S38 | ⭐ **"取线性段"那条登记可判**（任务 `#92` 新增；§3.5） | `include_str!("../src/sfz_instrument.rs")` 必须含字面量"`本仓取线性段；参考实现的指数／对数选项不在本片范围内`"＋ note-off 四档语义表里那一条；反向腿：文档里**没有**的句子 ⇒ `contains` 必须 `false`（⛔ 判据不是恒真） |
//!
//! # S5 的指纹是**接线前**的实测值
//!
//! 三个 u64 由本线在**改动任何源码之前**于 HEAD `3ad1bda7`（`aarch64/macos`、
//! 同一个夹具、同一条产品路径）测得：
//!
//! | 夹具 | 帧数 | 非零样本 | 峰值 | 指纹（FNV-1a 64，逐位） |
//! | :--- | :--- | :--- | :--- | :--- |
//! | 两个音符、无设备链 | 2560 | 5052 | 0.5478619 | `0x71dca4dc55b44df5` |
//! | 两个音符、`cutoff_hz = 800` / `resonance = 0.3` | 2560 | 5052 | 0.32756245 | `0x6eee378072730521` |
//!
//! ⚠ 这两个字面值**不是**规范常数，而是本机 libm/LLVM 的一次实现读数：它们的作用是
//! **跨提交对账**（接线前 vs 接线后同一个夹具给出同一个数），⛔ 不是"跨架构必须相等"
//! （跨架构对账是 `ARCH-DET-002` 的独立票）。形态照
//! `tests/drums_instrument.rs` 的 `check_fingerprint_on`：**同平台硬红、异平台如实跳过
//! 并点名原因**（跳过 **不是** 通过）。

mod support;

use support::{MixSpec, NoteSpec, note_project, render, render_snapshot, tuned_project};
use yeban_engine::drums::DrumsParams;
use yeban_engine::sfz_instrument::{
    InMemorySample, PendingSfz, PreparedRegion, PreparedSfz, SFZ_VOICES_PER_TRACK, SfzByteBudget,
    SfzByteUsage, SfzInstrument, SfzPlan, SfzVoice, SfzVoicePool,
};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_engine::synth::{NoteSchedule, ScheduledNote, SynthEngine, track_gain, velocity_gain};
use yeban_model::{
    DeviceDefinition, DeviceKind, EntityId, ParameterValue, SamplerBinding, TrackKind, TrackV3,
    YebanProjectV1,
};
// ⭐ 任务 `#91`：循环判据要读**生效循环模式**与 `yeban-sfz` 的两类**响亮 `Err`**
// （⛔ 判据不复制它们的取值，只引用唯一权威事实源）。
use yeban_sfz::error::SfzError;
use yeban_sfz::instrument::{LoopMode, OffMode};

/// ⭐ 步 2 **实测**（本机 `aarch64/macos`、HEAD `6ae70ef8` 的 `prepare` 广播路径）：
/// span-free 夹具 [`TWO_MATCH_ONE_MISS`] 在 `(60, 127)`、512 帧上的 FNV-1a 指纹。
/// ⚠ 它的作用是**跨提交对账**（步 2 vs 步 3 的同一条广播路径逐位不变），⛔ 不是规范常数。
const BROADCAST_STEP2_FINGERPRINT: u64 = 0xa933_ddab_286b_df5e;

/// ⭐ **改动前实测**（任务 `#91` 的 §4.4）：既有夹具 [`TWO_MATCH_ONE_MISS`]（**不带任何
/// `loop_*`**）经 `render_track` **实时路径**渲染 512 帧 @ `(60,127)` 的 FNV-1a 指纹。
///
/// 测法与测后处置见 [`s33_a_fixture_without_any_loop_fields_renders_bit_identically`]
/// 的文档：在 HEAD `5ac570eb`、**工作树干净**时用一次性探针测得，探针随后删除。
/// ⚠ 与 [`BROADCAST_STEP2_FINGERPRINT`]（摊平路径）同值 —— 两条独立来路的交叉印证
/// （`S1` 曾把"引擎逐样本 == 摊平逐样本"钉成 `max|Δ| = 0`）。
const NO_LOOP_FIXTURE_FINGERPRINT: u64 = 0xa933_ddab_286b_df5e;

/// FNV-1a 64（逐位；把每个 `f32` 的 4 个字节按小端卷进哈希）。
fn fnv1a_64(samples: &[f32]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for sample in samples {
        for byte in sample.to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// S5 夹具：两个音符（1 tick = 25 样本）。⭐ 本夹具与**接线前**测指纹时**一字不差**。
const SFZ_NOTES: [NoteSpec; 2] = [
    NoteSpec::at(0, 480, 60, 100),
    NoteSpec::at(960, 480, 64, 90),
];

/// S6（端到端）夹具：两个起音都落在 20 个量子的窗口内（⇒ 两条区域都会发声）。
const E2E_NOTES: [NoteSpec; 2] = [NoteSpec::at(0, 480, 60, 100), NoteSpec::at(48, 480, 64, 90)];

/// S6 的 SFZ：两条区域各管一个音高（⇒ 两个起音各触发一条）。
const E2E_SFZ: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n\
                       <region> sample=b.wav key=64 pitch_keycenter=60 volume=-6\n";

/// S5 夹具的渲染量子数（20 × 128 = 2560 帧，覆盖两个起音）。
const SFZ_QUANTA: usize = 20;

/// **接线前**实测：无设备链（见模块头）。
const BASELINE_PLAIN: u64 = 8_204_613_887_399_841_269;
/// **接线前**实测：`cutoff_hz = 800` / `resonance = 0.3`（见模块头）。
const BASELINE_TUNED: u64 = 7_993_387_413_444_691_233;

/// 指纹的**测量平台**（`[ARCH-DET-001]` 的 L1 只承诺"相同 OS ＋ CPU 架构 ＋
/// 纯 Rust libm"下的逐位一致）。
const MEASURED_PLATFORM: &str = "aarch64/macos";

/// 本机（**编译期事实**）是否就是那个测量平台。
const ON_MEASURED_PLATFORM: bool = cfg!(target_arch = "aarch64") && cfg!(target_os = "macos");

/// 同平台**硬红** / 异平台**如实跳过并点名原因**（跳过 **不是** 通过）。
///
/// 形态照 `tests/drums_instrument.rs` 的 `check_fingerprint_on`（那条判据的
/// 同一条理由：字面值是某台机器上的实现读数，不是规范常数）。
fn check_fingerprint(label: &str, actual: u64, expected: u64) -> bool {
    if ON_MEASURED_PLATFORM {
        assert_eq!(
            actual, expected,
            "{label}: 同平台（本机 == {MEASURED_PLATFORM}）⇒ 指纹必须逐位等于**接线前**\
             实测值 0x{expected:016x}（实得 0x{actual:016x}）；不同就是真缺陷"
        );
        return true;
    }
    println!(
        "[engine-sfz/S5] SKIP（未判定，不是通过）：{label} 的字面值 0x{expected:016x} \
         实测于 {MEASURED_PLATFORM}；跨架构的位级差异属 L2（max|Δ| < 1e-6），不是本判据的失败"
    );
    false
}

/// 步 1 的"多匹配"夹具：**两条 `key=60` 的区域都匹配**（第二条另有 `volume=-6`），
/// 外加一条 `key=72` 的不命中区域（反向对照）。
const TWO_MATCH_ONE_MISS: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n\
                                  <region> sample=b.wav key=60 pitch_keycenter=60 volume=-6\n\
                                  <region> sample=c.wav key=72 pitch_keycenter=60\n";

/// 摊平用的内存样本长度（帧）：极短 ⇒ 一块 512 帧内就播完。
const SAMPLE_FRAMES: usize = 480;

/// 块长（帧）：**比样本长** ⇒ 一块之内就能观察到"样本自然结束 ⇒ 退休"。
const BLOCK: usize = 512;

/// ⭐ S1 第二条腿用的**量子长**（帧；= `block::DEFAULT_BLOCK_FRAMES`）。
const QUANTUM: usize = 128;

/// ⭐ S1 第二条腿用的**量子数**：`8 × 128 = 1024` 帧 > 样本 480 帧 ⇒ 真的跨块。
const MULTI_BLOCKS: usize = 8;

/// 摊平一条 SFZ 文本（控制侧；**允许分配**）。
fn prepare(text: &str) -> PreparedSfz {
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let sample = InMemorySample::sine(440.0, 48_000.0, SAMPLE_FRAMES);
    PreparedSfz::prepare(&sfz, &sample, 48_000.0).expect("摊平必须成功")
}

/// 造一台已经武装好 `track` 的合成器（48 kHz）。
fn engine_with_track(track: EntityId) -> SynthEngine {
    let mut engine = SynthEngine::new(48_000);
    engine.begin_snapshot(48_000, &[track], [], []);
    engine
}

/// 两个声道的位模式（"逐位相同"的判定量）。
fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|sample| sample.to_bits()).collect()
}

/// S1：**引擎声部路径与摊平表逐样本差 ＝ 0**，且两侧都真的出声。
///
/// ⚠ 这条 ==0 的**强度**在于接线（触发哪些区域／速率／增益／终点），⛔ 不在于"两段
/// 逐样本循环"：那一段是**同一个函数**（`sfz_instrument::mix_span_into`），
/// 因为本仓的机械纪律是"全仓不许两份实现"（两份在 binary32 下无法维持逐位相同）。
#[test]
fn s1_engine_voices_match_the_flattened_table_sample_by_sample() {
    let prepared = prepare(TWO_MATCH_ONE_MISS);
    assert_eq!(prepared.region_count(), 3, "三条区域都要摊平进来");
    let track = EntityId::new();
    let mut engine = engine_with_track(track);
    // 力度 127（`support::scheduled` 的口径）⇒ 与下面摊平表的 `(60, 127)` 同参数。
    let schedule =
        NoteSchedule::from_sorted(vec![support::scheduled(0, 12_000, 60, 1.0, 48_000.0)]);

    let mut engine_out = vec![0.0f32; BLOCK];
    engine.render_track(track, Some(&schedule), Some(&prepared), &mut engine_out);

    let mut flat = vec![0.0f32; BLOCK];
    let flat_written = prepared.render_block_into(60, 127, &mut flat);

    let max_abs_diff = engine_out
        .iter()
        .zip(flat.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    println!(
        "[engine-sfz/S1] 逐样本最大绝对差 = {max_abs_diff:e}；摊平写入 {flat_written} 帧；\
         引擎非零 = {}；摊平非零 = {}；引擎活声部 = {:?}；丢弃 = {}",
        engine_out.iter().filter(|s| **s != 0.0).count(),
        flat.iter().filter(|s| **s != 0.0).count(),
        engine.sfz_active_voices(track),
        engine.sfz_voice_drops(),
    );
    assert_eq!(
        max_abs_diff, 0.0,
        "⭐ 引擎声部路径必须与 `PreparedSfz::render_block_into` **逐样本相同**\
         （两条 `key=60` 的区域都匹配 ⇒ 两侧都必须叠加两条）"
    );
    // 反向对照：两侧都必须真的出声（⛔ 不能"两边都全零"也算一致）。
    assert!(
        engine_out.iter().any(|s| *s != 0.0),
        "引擎侧必须真的出声（否则上面的等号是空转）"
    );
    assert!(flat.iter().any(|s| *s != 0.0), "摊平侧必须真的出声");

    // --- ⭐ S1 第二条腿（任务单 `#66` 补做 2）：**逐块拼接的整音符** ---
    //
    // 为什么必须补（注入实测）：上面那条腿只跨**一块**（512 帧 ≥ 样本 480 帧）
    // ⇒ `written == remaining` 永不发生 ⇒ "声部是否跨量子延续"**没有被量到**。
    // 实测：把 `written < remaining` 改成 `<=`（提前退休）时上一条腿**逐位不变**，
    // 只有把尺度**跨过"样本长"**才看得见 —— 那种注入会让输出腰斩
    // （端到端非零 1716 → 508、㉙ 总能量 2999.164 → 780.073）而**无人知**。
    //
    // 判定量：8 个 128 帧量子（= 1024 帧）拼起来的整段，与摊平表**一次性 1024 帧**
    // 调用的**逐样本最大绝对差**；外加"真的有 ≥ 2 个块出声"（跨块见证，⛔ 防空转）。
    let mut engine_multi = engine_with_track(track);
    let mut joined = vec![0.0f32; QUANTUM * MULTI_BLOCKS];
    let mut blocks_with_sound = 0usize;
    for block in 0..MULTI_BLOCKS {
        let mut out = vec![0.0f32; QUANTUM];
        engine_multi.render_track(track, Some(&schedule), Some(&prepared), &mut out);
        if out.iter().any(|s| *s != 0.0) {
            blocks_with_sound += 1;
        }
        joined[block * QUANTUM..(block + 1) * QUANTUM].copy_from_slice(&out);
        engine_multi.advance(QUANTUM);
    }
    let mut flat_joined = vec![0.0f32; QUANTUM * MULTI_BLOCKS];
    let flat_joined_written = prepared.render_block_into(60, 127, &mut flat_joined);
    let joined_diff = joined
        .iter()
        .zip(flat_joined.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    println!(
        "[engine-sfz/S1b] 逐块拼接（{MULTI_BLOCKS} × {QUANTUM} = {} 帧）：逐样本最大绝对差 = \
         {joined_diff:e}；有声块 = {blocks_with_sound}；摊平一次性写入 {flat_joined_written} 帧；\
         引擎非零 = {}；摊平非零 = {}",
        QUANTUM * MULTI_BLOCKS,
        joined.iter().filter(|s| **s != 0.0).count(),
        flat_joined.iter().filter(|s| **s != 0.0).count(),
    );
    assert_eq!(
        joined_diff, 0.0,
        "⭐ 逐块拼接的整音符必须与摊平表的一次性调用**逐样本相同**\
         （声部跨量子延续、且样本只播**一遍**）"
    );
    assert!(
        blocks_with_sound >= 2,
        "⭐ 夹具必须真的**跨块**（有声块 = {blocks_with_sound}）—— 否则这条腿量不到'跨量子延续'\
         （进程内自检：注入'提前退休'时必须在这里变红）"
    );
    assert!(
        flat_joined_written > 0,
        "摊平表的整段调用必须真的写入（否则等号是空转）"
    );
}

/// S2：**终点口径 ＝ 样本自然结束**（⛔ 无包络）。
///
/// 一块 512 帧 > 样本 480 帧 ⇒ 这一块内源采样就播完了：声部**退休**
/// （`active == 0`），下一块**逐位静音**。
///
/// 反向对照（关键）：摊平表的 `render_block_into` 是**无状态**的，它每一块都从源
/// 位置 `0.0` 重播 ⇒ 第二块**不是**静音。这条对照把"第二块静音"从"整条链没接线"
/// 里区分出来。
#[test]
fn s2_the_voice_retires_when_the_sample_runs_out() {
    let prepared = prepare(TWO_MATCH_ONE_MISS);
    let track = EntityId::new();
    let mut engine = engine_with_track(track);
    let schedule =
        NoteSchedule::from_sorted(vec![support::scheduled(0, 12_000, 60, 1.0, 48_000.0)]);

    let mut first = vec![0.0f32; BLOCK];
    engine.render_track(track, Some(&schedule), Some(&prepared), &mut first);
    let after_first = engine.sfz_active_voices(track);

    engine.advance(BLOCK);
    let mut second = vec![0.0f32; BLOCK];
    engine.render_track(track, Some(&schedule), Some(&prepared), &mut second);

    let mut flat_again = vec![0.0f32; BLOCK];
    let flat_written = prepared.render_block_into(60, 127, &mut flat_again);

    println!(
        "[engine-sfz/S2] 第一块非零 = {}；播完后活声部 = {after_first:?}；第二块非零 = {}；\
         摊平表第二块写入 = {flat_written}（非零 = {}）",
        first.iter().filter(|s| **s != 0.0).count(),
        second.iter().filter(|s| **s != 0.0).count(),
        flat_again.iter().filter(|s| **s != 0.0).count(),
    );
    assert!(first.iter().any(|s| *s != 0.0), "第一块必须出声");
    assert_eq!(
        after_first,
        Some(0),
        "⭐ 样本播完 ⇒ 声部必须退休（`active == 0`）；`min(end_sample, 样本播完)` 的\
         诚实占位就是这一条"
    );
    assert!(
        second.iter().all(|s| *s == 0.0),
        "退休之后下一块必须**逐位静音**"
    );
    assert!(
        flat_again.iter().any(|s| *s != 0.0),
        "反向对照：摊平表是无状态的一次性调用 ⇒ 第二块**仍然出声**（否则'第二块静音'\
         可能只是整条链没接线）"
    );
}

/// S3：⭐ **显式 `span` 终点两侧同口径**（步 3 把离线/摊平侧补齐 ⇒ sfz-needs-4 **闭合**）。
///
/// `end=200`（含端点）⇒ 半开区间 `[0, 201)` ⇒ 三条路都写到 `floor(pos) + 1 >= 201`
/// 为止 ＝ **200 帧**；摊平表**不再**继续到样本末尾（步 2 时它继续 ⇒ 那是已登记的口径分叉）。
///
/// 反向对照：把 `end` 改成 `400` ⇒ **写入帧数变**（证明不是"恒定截断在 200"这种假口径）。
#[test]
fn s3_an_explicit_span_end_stops_the_voice_at_the_same_frame_on_both_sides() {
    let prepared = prepare("<region> sample=a.wav key=60 pitch_keycenter=60 end=200\n");
    let span = prepared.regions()[0].span().expect("span 必须已摊平");
    assert_eq!(
        prepared.regions()[0].source_end(SAMPLE_FRAMES),
        Some(201),
        "`end=200`（含端点）⇒ 半开 [0, 201)"
    );
    assert_eq!(span.end, Some(201), "摊平表的口径（步 1）");

    let track = EntityId::new();
    let mut engine = engine_with_track(track);
    let schedule =
        NoteSchedule::from_sorted(vec![support::scheduled(0, 12_000, 60, 1.0, 48_000.0)]);
    let mut engine_out = vec![0.0f32; BLOCK];
    engine.render_track(track, Some(&schedule), Some(&prepared), &mut engine_out);

    let mut flat = vec![0.0f32; BLOCK];
    let flat_frames = prepared.render_block_into(60, 127, &mut flat);

    let engine_written = engine_out.iter().filter(|s| **s != 0.0).count();
    let flat_written = flat.iter().filter(|s| **s != 0.0).count();
    // ⭐ 判定量是**逐样本**而不是"数非零"：正弦在 0 相位处就是 0.0（夹具的第 0 个样本），
    // 数非零会把"写了 200 帧"读成 199。
    let max_abs_diff = engine_out
        .iter()
        .zip(flat.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    println!(
        "[engine-sfz/S3] 引擎非零 = {engine_written}；摊平非零 = {flat_written}；\
         摊平写入帧数 = {flat_frames}；整块逐样本最大绝对差 = {max_abs_diff:e}"
    );
    assert_eq!(
        max_abs_diff, 0.0,
        "⭐ 步 3 之后：`span` 起点/终点两侧**逐样本相同**（不再是'只覆盖触发量子'）"
    );
    assert_eq!(
        flat_frames, 200,
        "⭐ `end=200`（半开 [0,201)）⇒ 摊平表必须也停在 **200** 帧（线性插值要两个样本）"
    );
    assert!(
        engine_out[..200].iter().any(|s| *s != 0.0),
        "两侧在 span 内必须真的出声"
    );
    assert!(
        engine_out[200..].iter().all(|s| *s == 0.0),
        "⭐ 引擎必须在 span 终点（第 200 帧）停：之后**一帧不出**"
    );
    assert!(
        flat[200..].iter().all(|s| *s == 0.0),
        "⭐ 摊平表**也**必须停在同一帧（步 2 时它继续出声 ⇒ 口径分叉已闭合）"
    );

    // --- 反向对照：改 `end` ⇒ 帧数必须跟着变（⛔ 挡"恒定截断"的假口径） ---
    let longer = prepare("<region> sample=a.wav key=60 pitch_keycenter=60 end=400\n");
    let mut longer_out = vec![0.0f32; BLOCK];
    let longer_frames = longer.render_block_into(60, 127, &mut longer_out);
    println!("[engine-sfz/S3] 反向对照 `end=400`：摊平写入帧数 = {longer_frames}");
    assert_eq!(longer_frames, 400, "`end=400` ⇒ 写 400 帧（半开 [0,401)）");
    assert_ne!(
        longer_frames, flat_frames,
        "⭐ 两个不同的 `end` 必须给出不同的写入帧数（否则上面的 200 可能是常数）"
    );
}

/// S4：⭐ `end=-1`（`SampleEnd::Silent`）的区域**不产生采样输出**（三条路同口径）。
///
/// 引擎⛔ 不建声部、一帧不写；⭐ **摊平表也一帧不写**（步 3 补齐；步 2 时它仍出声 —— 那是
/// 已登记的口径分叉）。
///
/// 反向对照（关键）：去掉 `end=-1` 之后**同一份夹具**必须出声 ⇒ 上面的静音不是"夹具坏了"。
#[test]
fn s4_a_silent_span_never_gets_a_voice_nor_any_sample() {
    let prepared = prepare("<region> sample=a.wav key=60 pitch_keycenter=60 end=-1\n");
    assert_eq!(
        prepared.regions()[0].span(),
        None,
        "`end=-1` ⇒ `SampleEnd::Silent` ⇒ 不产生采样输出"
    );
    assert_eq!(prepared.regions()[0].source_end(SAMPLE_FRAMES), None);
    assert_eq!(prepared.regions()[0].source_start(SAMPLE_FRAMES), None);

    let track = EntityId::new();
    let mut engine = engine_with_track(track);
    let schedule =
        NoteSchedule::from_sorted(vec![support::scheduled(0, 12_000, 60, 1.0, 48_000.0)]);
    let mut engine_out = vec![0.0f32; BLOCK];
    engine.render_track(track, Some(&schedule), Some(&prepared), &mut engine_out);

    let mut flat = vec![0.0f32; BLOCK];
    let flat_frames = prepared.render_block_into(60, 127, &mut flat);

    println!(
        "[engine-sfz/S4] 引擎非零 = {}；活声部 = {:?}；摊平非零 = {}；摊平写入 = {flat_frames}",
        engine_out.iter().filter(|s| **s != 0.0).count(),
        engine.sfz_active_voices(track),
        flat.iter().filter(|s| **s != 0.0).count(),
    );
    assert!(
        engine_out.iter().all(|s| *s == 0.0),
        "⭐ `end=-1` 的区域不得产生任何样本"
    );
    assert_eq!(
        engine.sfz_active_voices(track),
        Some(0),
        "⭐ 一条声部都不该建出来"
    );
    assert!(
        flat.iter().all(|s| *s == 0.0),
        "⭐ 摊平表**也**必须静音（步 3：三条路共用 `reduce_span` 的 `None` 口径）"
    );
    assert_eq!(flat_frames, 0, "摊平表写入帧数必须是 0");

    // --- 反向对照：去掉 `end=-1` ⇒ 同一夹具必须出声 ---
    let normal = prepare("<region> sample=a.wav key=60 pitch_keycenter=60\n");
    let mut normal_out = vec![0.0f32; BLOCK];
    let normal_frames = normal.render_block_into(60, 127, &mut normal_out);
    println!("[engine-sfz/S4] 反向对照（无 `end=-1`）：摊平写入帧数 = {normal_frames}");
    assert!(
        normal_frames > 0 && normal_out.iter().any(|s| *s != 0.0),
        "⭐ 反向对照必须出声（否则上面的静音可能只是夹具坏了）"
    );
}

/// S5：⭐ **反向对照（关键）**：**不给 `sfz`** 时端到端输出与**接线前**逐位相同。
///
/// 两条臂（无设备链 / 带音色）的指纹都必须等于**改动源码之前**实测的字面值。
/// 同平台硬红、异平台如实跳过（见 [`check_fingerprint`]）。
#[test]
fn s5_without_sfz_the_output_is_bit_identical_to_before_the_wiring() {
    let plain = note_project(&SFZ_NOTES);
    let rendered = render(&plain.project, SFZ_QUANTA);
    println!(
        "[engine-sfz/S5] 无设备链: 帧={} 非零={} 峰值={} 指纹={} (0x{:016x})",
        rendered.frames(),
        rendered.nonzero(),
        rendered.peak(),
        rendered.fingerprint(),
        rendered.fingerprint(),
    );
    assert!(rendered.nonzero() > 0, "夹具必须真的出声（否则等号是空转）");
    let plain_checked = check_fingerprint("无设备链", rendered.fingerprint(), BASELINE_PLAIN);

    let tuned = tuned_project(&SFZ_NOTES, MixSpec::tone(800.0, 0.3));
    let rendered_tuned = render(&tuned.project, SFZ_QUANTA);
    println!(
        "[engine-sfz/S5] 音色(800, 0.3): 帧={} 非零={} 峰值={} 指纹={} (0x{:016x})",
        rendered_tuned.frames(),
        rendered_tuned.nonzero(),
        rendered_tuned.peak(),
        rendered_tuned.fingerprint(),
        rendered_tuned.fingerprint(),
    );
    let tuned_checked = check_fingerprint(
        "音色(800, 0.3)",
        rendered_tuned.fingerprint(),
        BASELINE_TUNED,
    );

    // ⚠ 两条臂必须在**同一平台判定**下：不能一条硬红一条跳过（那说明平台判定漂了）。
    assert_eq!(
        plain_checked, tuned_checked,
        "两条臂的平台判定必须一致（否则本判据的读数不可比）"
    );
}

/// S6：`with_sfz` 是**唯一**的绑定入口（模型⛔ 不给），且端到端真的非静音、
/// 真的换了音源（指纹与"不给 SFZ"不同）。
#[test]
fn s6_the_snapshot_only_carries_the_sfz_a_caller_injected_and_it_is_audible() {
    let fixture = note_project(&E2E_NOTES);
    let track = fixture.track;

    // ① `from_project` **不**绑 SFZ（模型层今天没有轨↔SFZ 绑定）。
    let bare = EngineSnapshot::from_project(&fixture.project, 1).expect("快照");
    assert!(
        bare.sfz().is_empty(),
        "⛔ `from_project` 不得凭空绑一份 SFZ（模型语义不由引擎发明）"
    );
    assert!(bare.sfz_plan(&track).is_none());

    // ② 同一份快照 + 显式注入 ⇒ 表里正好一条，且只有那条轨。
    let plan = SfzPlan::prepare(
        &SfzInstrument::load(E2E_SFZ.to_owned(), &Default::default()).expect("SFZ 必须解析成功"),
        &InMemorySample::sine(440.0, 48_000.0, SAMPLE_FRAMES),
        48_000.0,
    )
    .expect("摊平必须成功");
    println!(
        "[engine-sfz/S6] 摊平表：区域 {}、内存 {} 字节",
        plan.region_count(),
        plan.memory_bytes()
    );
    let armed = EngineSnapshot::from_project(&fixture.project, 1)
        .expect("快照")
        .with_sfz(track, plan);
    assert_eq!(armed.sfz().len(), 1, "注入一次 ⇒ 表里一条");
    assert!(armed.sfz_plan(&track).is_some(), "注入的那条轨必须在表里");
    assert!(
        armed.sfz_plan(&fixture.master).is_none(),
        "⛔ 母线上不该有 SFZ"
    );

    // ③ 端到端（产品路径：快照槽 → 渲染量子 → 声相 → 母线限制器）：
    //    SFZ 臂必须非静音，且指纹与"不给 SFZ"的**不同**（真的换了音源）。
    let with_sfz = render_snapshot(armed, SFZ_QUANTA);
    let without = render(&fixture.project, SFZ_QUANTA);
    println!(
        "[engine-sfz/S6] SFZ 臂: 非零={} 峰值={} 指纹=0x{:016x}；对照臂: 非零={} 峰值={} 指纹=0x{:016x}",
        with_sfz.nonzero(),
        with_sfz.peak(),
        with_sfz.fingerprint(),
        without.nonzero(),
        without.peak(),
        without.fingerprint(),
    );
    assert!(
        with_sfz.nonzero() > 0 && with_sfz.peak() > 0.0,
        "⭐ 端到端必须**非静音**（非零 = {}）",
        with_sfz.nonzero()
    );
    assert_ne!(
        with_sfz.fingerprint(),
        without.fingerprint(),
        "⭐ 注入 SFZ 之后指纹必须与对照臂不同（否则 SFZ 支路根本没接上）"
    );
}

/// S7：分派次序 ＝ **鼓机 ＞ SFZ ＞ 复音**（一条轨至多一件音源）。
///
/// ① 鼓机 ＋ SFZ 同轨 ⇒ 输出对"只有鼓机"**逐位相同**（SFZ 池被释放、⛔ 不叠加）；
/// ② 反向对照：同一份 SFZ 在没有鼓机的轨上**真的出声**（⇒ ① 的等号不是"两边都哑"）。
/// ③ "SFZ ＞ 复音"由 S1 覆盖：那台槽的复音合成器本来会出声，而 S1 的逐样本差为 0
///    ⇒ 复音**没有**被叠加进去。
#[test]
fn s7_drums_win_over_sfz_which_wins_over_polyphony() {
    let prepared = prepare(TWO_MATCH_ONE_MISS);
    let track = EntityId::new();
    // 键位映射里**包含** key 60 ⇒ 同一条音符在鼓机分支里也有鼓件可打。
    let device = DeviceDefinition {
        id: EntityId::new(),
        name: "Yeban Drums".to_owned(),
        kind: DeviceKind::InternalInstrument,
        bypassed: false,
        params: [
            ("kick_note", 60.0),
            ("snare_note", 38.0),
            ("closed_hat_note", 42.0),
            ("open_hat_note", 46.0),
            ("clap_note", 39.0),
        ]
        .iter()
        .map(|(name, value)| ParameterValue {
            name: (*name).to_owned(),
            value: *value,
            unit: None,
        })
        .collect(),
        latency_samples: 0,
    };
    let params = DrumsParams::from_devices(&[device]).expect("五个键位名写全 ⇒ 识别为鼓机");
    let schedule =
        NoteSchedule::from_sorted(vec![support::scheduled(0, 12_000, 60, 1.0, 48_000.0)]);

    let mut with_sfz = SynthEngine::new(48_000);
    let mut only_drums = SynthEngine::new(48_000);
    let mut only_sfz = SynthEngine::new(48_000);
    with_sfz.begin_snapshot(48_000, &[track], [], [(&track, &params)]);
    only_drums.begin_snapshot(48_000, &[track], [], [(&track, &params)]);
    only_sfz.begin_snapshot(48_000, &[track], [], []);

    let mut a = vec![0.0f32; 128];
    let mut b = vec![0.0f32; 128];
    let mut c = vec![0.0f32; 128];
    with_sfz.render_track(track, Some(&schedule), Some(&prepared), &mut a);
    only_drums.render_track(track, Some(&schedule), None, &mut b);
    only_sfz.render_track(track, Some(&schedule), Some(&prepared), &mut c);

    println!(
        "[engine-sfz/S7] 鼓机+SFZ 非零 = {}；只有鼓机 非零 = {}；只有 SFZ 非零 = {}；\
         鼓机臂的 SFZ 活声部 = {:?}",
        a.iter().filter(|s| **s != 0.0).count(),
        b.iter().filter(|s| **s != 0.0).count(),
        c.iter().filter(|s| **s != 0.0).count(),
        with_sfz.sfz_active_voices(track),
    );
    assert!(b.iter().any(|s| *s != 0.0), "鼓机臂必须真的出声");
    assert_eq!(
        bits(&a),
        bits(&b),
        "⭐ 鼓机 ＞ SFZ：同轨两件音源时，输出必须与'只有鼓机'**逐位相同**\
         （⛔ SFZ 不得叠加进去）"
    );
    assert_eq!(
        with_sfz.sfz_active_voices(track),
        Some(0),
        "鼓机分支必须**释放** SFZ 池（⛔ 不留上一件音源的尾巴）"
    );
    assert!(
        c.iter().any(|s| *s != 0.0),
        "反向对照：没有鼓机的同一份 SFZ 必须真的出声（否则上面的等号可能只是两边都哑）"
    );
}

/// S8：池满（> [`SFZ_VOICES_PER_TRACK`] 条区域同时匹配）⇒ ⭐ **窃取**（⛔ 不是丢弃）。
///
/// # 判定量（`sfz-needs-2` 的落地）
///
/// * 17 条**全部匹配** ⇒ `created == 16`（窃取占的是**别人**的槽，⛔ 不计入"新建"）、
///   `stolen == 1`、`dropped == 0`（⭐ 池满不再是丢弃）；
/// * 33 条 ⇒ 挂起者还没轮到发声就被**再一次**换掉 ⇒ 那是**真丢弃**：
///   `stolen == 17` ＋ `dropped == 16`（⛔ 不静默 —— 这条腿保住"丢弃计数"的语义）；
/// * 反向对照：15 条（未满）⇒ `stolen == 0` ＋ `dropped == 0`。
///
/// ⚠ 本判据的**旧期望**是 `dropped == 1`（丢弃口径）；本单把它改成窃取口径
/// —— 注入"改回丢弃"时它当场变红（读数见提交报告）。
#[test]
fn s8_a_full_pool_steals_instead_of_silently_dropping_the_overflow() {
    let mut text = String::new();
    for _ in 0..=SFZ_VOICES_PER_TRACK {
        text.push_str("<region> sample=a.wav key=60 pitch_keycenter=60\n");
    }
    let prepared = prepare(&text);
    assert_eq!(
        prepared.region_count(),
        SFZ_VOICES_PER_TRACK + 1,
        "夹具必须是 16 + 1 条**全部匹配**的区域"
    );

    let mut pool = SfzVoicePool::empty();
    // ⭐ 轨增益门 = 1.0（单位）：本判据量的是**池容量**，⛔ 不掺轨增益。
    let created = pool.trigger_note(&prepared, 60, 100, 12_000, 1.0);
    println!(
        "[engine-sfz/S8] 17 条：建出声部 = {created}；活声部 = {}；窃取 = {}；真丢弃 = {}；挂起 = {}",
        pool.active_count(),
        pool.stolen(),
        pool.dropped(),
        pool.pending_count(),
    );
    assert_eq!(created, SFZ_VOICES_PER_TRACK, "池只能装 16 个空槽");
    assert_eq!(pool.active_count(), SFZ_VOICES_PER_TRACK);
    assert_eq!(
        pool.stolen(),
        1,
        "⭐ 第 17 条区域必须**窃取**一个槽（⛔ 不是丢弃）"
    );
    assert_eq!(pool.dropped(), 0, "⭐ 窃取**不是**丢弃");
    assert_eq!(pool.pending_count(), 1, "⭐ 新声部挂在 victim 的槽里等接管");

    // ⭐ 33 条：挂起者被**再一次**窃取换掉 ⇒ 真丢弃（计数仍在，⛔ 不静默）。
    let mut many = String::new();
    for _ in 0..(2 * SFZ_VOICES_PER_TRACK + 1) {
        many.push_str("<region> sample=a.wav key=60 pitch_keycenter=60\n");
    }
    let many_prepared = prepare(&many);
    let mut many_pool = SfzVoicePool::empty();
    let many_created = many_pool.trigger_note(&many_prepared, 60, 100, 12_000, 1.0);
    println!(
        "[engine-sfz/S8] 33 条：建出声部 = {many_created}；窃取 = {}；真丢弃 = {}；挂起 = {}",
        many_pool.stolen(),
        many_pool.dropped(),
        many_pool.pending_count(),
    );
    assert_eq!(many_created, SFZ_VOICES_PER_TRACK);
    assert_eq!(many_pool.active_count(), SFZ_VOICES_PER_TRACK);
    assert_eq!(
        many_pool.stolen(),
        17,
        "⭐ 33 条 ⇒ 17 次窃取（第 17..33 条各一次）"
    );
    assert_eq!(
        many_pool.dropped(),
        16,
        "⭐ 被换掉的挂起者（第 17..32 条）是真丢弃 ⇒ 必须计数（⛔ 不静默）"
    );

    // 反向对照：15 条区域时**不**该有窃取或丢弃（否则上面那些数可能是常数）。
    let mut under = String::new();
    for _ in 0..(SFZ_VOICES_PER_TRACK - 1) {
        under.push_str("<region> sample=a.wav key=60 pitch_keycenter=60\n");
    }
    let under_prepared = prepare(&under);
    let mut under_pool = SfzVoicePool::empty();
    assert_eq!(
        under_pool.trigger_note(&under_prepared, 60, 100, 12_000, 1.0),
        SFZ_VOICES_PER_TRACK - 1
    );
    assert_eq!(under_pool.dropped(), 0, "未满时不得计数");
    assert_eq!(under_pool.stolen(), 0, "未满时不得窃取");
}

/// S9：**尺寸照打** ＋ `EngineRuntime` 的 192 KiB 门槛（与
/// `tests/synth_rt_zero_alloc.rs:2570` 同一口径）。
///
/// 定长声部池的代价必须落在尺寸读数上：`SynthEngine` / `EngineRuntime` 都是
/// **按值搬进回调**的对象 ⇒ 这道门槛是"有没有把大件塞进实时对象"的机械防线。
#[test]
fn s9_sizes_are_printed_and_the_runtime_stays_under_192_kib() {
    let voice = core::mem::size_of::<SfzVoice>();
    let pool = core::mem::size_of::<SfzVoicePool>();
    let runtime = core::mem::size_of::<yeban_engine::rt::EngineRuntime>();
    println!(
        "[engine-sfz/S9] size_of: SfzVoice={voice} SfzVoicePool={pool} \
         SfzPlan={} PreparedSfz={} SynthEngine={} EngineSnapshot={} EngineRuntime={runtime} \
         （门槛 192 KiB = {}）",
        core::mem::size_of::<SfzPlan>(),
        core::mem::size_of::<PreparedSfz>(),
        core::mem::size_of::<SynthEngine>(),
        core::mem::size_of::<EngineSnapshot>(),
        192 * 1024,
    );
    // ⚠ 钉住声部的**定长**形状：`SfzVoice` 只含 `f32`/`u8`/`bool`/`u16`/`u64`，
    // ⛔ 无指针、⛔ 无 `Vec`。
    // ⭐ 步 3 的代价：`sample_index: u16` 进了声部 ⇒ 32 B → **40 B**（对齐到 8）。
    // ⭐ **本单（`sfz-needs-2`）的代价**：池里多了 16 槽 ×（挂起声部 40 ＋ 淡出 16 ＋
    // 触发序号 8）＋ 挂起标志 16 ＋ 两个计数器（丢弃/窃取/序号舵）⇒ 池 648 → **1712**、
    // `SynthEngine` 91984 → **109152**、`EngineRuntime` 162832 → **180000**
    // （仍 < 192 KiB，余量 16608 字节）。
    // ⭐ **本单（循环接线 `#91`）的代价 ＝ 0 字节**：声部只多一个 `region_index: u16`
    // （循环窗口由**区域**拥有，见 [`SfzVoice::region_index`]），它落在**既有尾部填充**里
    // ⇒ `SfzVoice` 仍 **40 B**、池仍 **1712**、`EngineRuntime` 仍 **180000**（余量仍 16608）。
    // ⚠ 涨价的是**摊平表**：`PreparedRegion` 2624 → **2632 B**（+8；三个循环字段），
    // 它按**区域数**计（`S25` 的预算读数跟着走），⛔ 不进实时对象。
    assert_eq!(voice, 40, "一个 SFZ 声部 = 40 字节（定长、全 `Copy`）");
    assert_eq!(
        pool, 1712,
        "声部池 = 16 槽 ×（声部 40 ＋ 挂起 40 ＋ 淡出 16 ＋ 序号 8）＋ 挂起标志 16 \
         ＋ 计数器 28（序号舵 8 ＋ 丢弃 8 ＋ 窃取 8 ＋ 淡出帧数 4）⇒ 对齐到 1712"
    );
    assert!(
        runtime < 192 * 1024,
        "`EngineRuntime` = {runtime} 字节 ≥ 192 KiB（{})—— 定长声部池把实时对象撑爆了",
        192 * 1024
    );
}

/// S10（任务单 `#66` **补做 2**）：**换音源 / 定位必须释放 SFZ 池**。
///
/// # 为什么补它（注入实测的缺口，逐字读数见补做报告）
///
/// `slot.sfz.silence()` 的**三处调用**（`synth.rs:1483` 的 `seek`／`:1831` 的鼓机分支／
/// `:1841` 的复音分支）在 S1~S9 里**零覆盖**：把三处全删掉之后，`S1~S9` 与
/// `rt_zero_alloc` ㉙ **全绿**（9 passed / 73 通过），而**临时探针**量到注入确实活着 ——
/// 换音源后 `sfz_active_voices` 仍是 `Some(1)`、`seek` 后仍是 `Some(1)`，
/// 且**切回 SFZ 时陈旧声部从旧源位置复活、写出 128 个非零样本**（还原态三处全是
/// `Some(0)` / `0`）。那是**用户可见的错**：换掉音源（或定位）之后仍听到上一件音源的尾巴。
///
/// # 判定量
///
/// * `SynthEngine::sfz_active_voices`（声部数）：触发后必须 **2**（前置见证）、
///   换音源后与 `seek` 后必须 **0**；
/// * 换音源那一个量子的**复音非零样本数** > 0（证明那个量子**真的**走了复音分支，
///   ⛔ 不是"整条链没跑"）；
/// * 切回 SFZ 那一个量子的**非零样本数** ＝ **0**（逐位静音，⛔ 不得复活）。
#[test]
fn s10_changing_the_source_or_seeking_releases_the_sfz_pool() {
    let prepared = prepare(TWO_MATCH_ONE_MISS);
    let track = EntityId::new();
    // 音符 A：key 60（两条区域都匹配）⇒ 第一量子建 **2** 个 SFZ 声部；
    // 音符 B：在**第二量子**起音、音高 67（三条区域都不匹配）⇒ 换音源那一个量子由
    // **复音**发声 ⇒ 那个量子是"真的走了复音分支"的见证。
    let schedule = NoteSchedule::from_sorted(vec![
        support::scheduled(0, 12_000, 60, 1.0, 48_000.0),
        support::scheduled(QUANTUM as u64, 3_000, 67, 1.0, 48_000.0),
    ]);

    // --- 臂 A：换音源（`sfz = None` ⇒ 复音分支）必须释放池 ---
    let mut engine = engine_with_track(track);
    let mut out = vec![0.0f32; QUANTUM];
    engine.render_track(track, Some(&schedule), Some(&prepared), &mut out);
    let after_sfz = engine.sfz_active_voices(track);
    engine.advance(QUANTUM);
    engine.render_track(track, Some(&schedule), None, &mut out);
    let poly_nonzero = out.iter().filter(|s| **s != 0.0).count();
    let after_swap = engine.sfz_active_voices(track);
    engine.advance(QUANTUM);
    let mut back = vec![0.0f32; QUANTUM];
    engine.render_track(track, Some(&schedule), Some(&prepared), &mut back);
    let back_nonzero = back.iter().filter(|s| **s != 0.0).count();

    // --- 臂 B：`seek`（定位）也必须释放池 ---
    let mut engine_seek = engine_with_track(track);
    engine_seek.render_track(track, Some(&schedule), Some(&prepared), &mut out);
    let before_seek = engine_seek.sfz_active_voices(track);
    engine_seek.seek(0);
    let after_seek = engine_seek.sfz_active_voices(track);

    println!(
        "[engine-sfz/S10] 触发后活声部 = {after_sfz:?}；换音源那一块复音非零 = {poly_nonzero}；\
         换音源后活声部 = {after_swap:?}；切回 SFZ 的非零 = {back_nonzero}；\
         seek 前 = {before_seek:?}、seek 后 = {after_seek:?}"
    );
    assert_eq!(
        after_sfz,
        Some(2),
        "⭐ 前置见证：两条 `key=60` 的区域都匹配 ⇒ 触发后必须有 **2** 个声部在飞\
         （否则下面的 `Some(0)` 是空转）"
    );
    assert!(
        poly_nonzero > 0,
        "⭐ 换音源那一个量子必须由**复音**发声（非零 = {poly_nonzero}）—— 这是'真的换了音源'的见证"
    );
    assert_eq!(
        after_swap,
        Some(0),
        "⭐ 换音源（`sfz = None`）⇒ SFZ 池必须被**释放**（`slot.sfz.silence()`）"
    );
    assert_eq!(
        back_nonzero, 0,
        "⭐ 切回 SFZ 时陈旧声部⛔ 不得复活（该量子必须逐位静音）—— 注入实测：不释放时会\
         从旧源位置写出 128 个非零样本"
    );
    assert_eq!(
        before_seek,
        Some(2),
        "⭐ 前置见证：`seek` 前必须也有 2 个声部在飞"
    );
    assert_eq!(
        after_seek,
        Some(0),
        "⭐ `seek`（定位）⇒ SFZ 池必须被释放（⛔ 不留上一段采样）"
    );
}

/// 用**一条区域**的 SFZ 离线渲染 `note`（参照臂：证明"某区域 + 某份样本"该出什么）。
///
/// ⚠ 它**解析**（⇒ 分配）⇒ 只用于判据的离线参照臂（与 `PreparedSfz` 的构造期同一条边）。
fn offline_one(path: &str, key: u8, sample: &InMemorySample, note: u8, out: &mut [f32]) -> usize {
    let sfz = SfzInstrument::load(
        format!("<region> sample={path} key={key} pitch_keycenter=60\n"),
        &Default::default(),
    )
    .expect("单区域 SFZ 必须解析成功");
    sfz.render_note_into(sample, note, 127, out, 48_000.0)
        .expect("离线渲染不得报错")
}

/// S11（步 3）：⭐ **多采样 —— 两条区域各播自己那份内容**。
///
/// # 判定量
///
/// * 每条区域的摊平输出与"**单独一条 SFZ ＋ 它自己那份样本**"的离线渲染**逐位相同**；
/// * 两条之间**指纹不同**（⛔ 不是只断言"非静音"）；
/// * 反向对照 1：把提供表里两份样本**对调** ⇒ 指纹变（证明真的按 `sample_path` 取内容）；
/// * 反向对照 2：**广播**重载（`prepare`）在 key=64 上播的是 `a.wav` 的内容
///   （与"`a.wav` 的 key=64 离线渲染"逐位相同）⇒ 这就是 `sample_content=broadcast(1)` 的
///   **音色内容错**的机械证据。
///
/// ⚠ 本判据里的 "WAV" 是**已解码**的 [`InMemorySample`]（两条不同频率的正弦 ⇒ 内容不同）：
/// 真读 PCM WAV 文件的那一步在控制侧（`crates/yeban-app/src/sfz_load.rs`），⛔ 引擎不做 I/O。
#[test]
fn s11_two_regions_with_two_samples_each_play_their_own_content() {
    const TWO_KEYS: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n\
                            <region> sample=b.wav key=64 pitch_keycenter=60\n";
    let sfz = SfzInstrument::load(TWO_KEYS.to_owned(), &Default::default()).expect("解析必须成功");
    let a = InMemorySample::sine(440.0, 48_000.0, SAMPLE_FRAMES);
    let b = InMemorySample::sine(660.0, 48_000.0, SAMPLE_FRAMES);
    let provided = vec![
        ("a.wav".to_owned(), a.clone()),
        ("b.wav".to_owned(), b.clone()),
    ];
    let prepared =
        PreparedSfz::prepare_with_samples(&sfz, &provided, 48_000.0).expect("多采样摊平必须成功");

    assert_eq!(prepared.region_count(), 2, "两条区域都要摊平进来");
    assert!(!prepared.is_broadcast(), "多采样路径⛔ 不是广播");
    assert_eq!(prepared.sample_count(), 2, "两条不同路径 ⇒ 两份样本");
    assert_eq!(prepared.regions()[0].sample_index(), 0, "`a.wav` ⇒ 第 0 份");
    assert_eq!(prepared.regions()[1].sample_index(), 1, "`b.wav` ⇒ 第 1 份");

    // ① 各播自己：摊平输出 == "单独一条 + 它自己那份样本"的离线渲染（逐位）
    let mut flat60 = vec![0.0f32; BLOCK];
    prepared.render_block_into(60, 127, &mut flat60);
    let mut ref60 = vec![0.0f32; BLOCK];
    let ref60_frames = offline_one("a.wav", 60, &a, 60, &mut ref60);

    let mut flat64 = vec![0.0f32; BLOCK];
    prepared.render_block_into(64, 127, &mut flat64);
    let mut ref64 = vec![0.0f32; BLOCK];
    let ref64_frames = offline_one("b.wav", 64, &b, 64, &mut ref64);

    println!(
        "[engine-sfz/S11] key=60: 摊平指纹 = 0x{:016x}／离线(a) 指纹 = 0x{:016x}（帧 {ref60_frames}）；\
         key=64: 摊平指纹 = 0x{:016x}／离线(b) 指纹 = 0x{:016x}（帧 {ref64_frames}）",
        fnv1a_64(&flat60),
        fnv1a_64(&ref60),
        fnv1a_64(&flat64),
        fnv1a_64(&ref64),
    );
    assert_eq!(
        bits(&flat60),
        bits(&ref60),
        "⭐ `key=60` 的区域必须播 `a.wav` 那份内容（与离线参照**逐位相同**）"
    );
    assert_eq!(
        bits(&flat64),
        bits(&ref64),
        "⭐ `key=64` 的区域必须播 `b.wav` 那份内容（与离线参照**逐位相同**）"
    );
    assert!(flat60.iter().any(|s| *s != 0.0), "key=60 必须真的出声");
    assert!(flat64.iter().any(|s| *s != 0.0), "key=64 必须真的出声");
    assert_ne!(
        fnv1a_64(&flat60),
        fnv1a_64(&flat64),
        "⭐ 两份不同内容 ⇒ 两条区域的指纹必须**不同**（⛔ 不是只断言'非静音'）"
    );

    // ② 反向对照 1：对调提供表 ⇒ 内容跟着换（指纹变），且等于对调后的离线参照
    let swapped = vec![
        ("a.wav".to_owned(), b.clone()),
        ("b.wav".to_owned(), a.clone()),
    ];
    let swapped_plan = PreparedSfz::prepare_with_samples(&sfz, &swapped, 48_000.0).expect("摊平");
    let mut swapped60 = vec![0.0f32; BLOCK];
    swapped_plan.render_block_into(60, 127, &mut swapped60);
    let mut ref_swapped60 = vec![0.0f32; BLOCK];
    offline_one("a.wav", 60, &b, 60, &mut ref_swapped60);
    println!(
        "[engine-sfz/S11] 对调提供表：key=60 指纹 = 0x{:016x}（原 = 0x{:016x}）",
        fnv1a_64(&swapped60),
        fnv1a_64(&flat60),
    );
    assert_eq!(
        bits(&swapped60),
        bits(&ref_swapped60),
        "对调之后 `a.wav` 那份槽位里放的是 `b` 的内容 ⇒ 输出必须跟着换"
    );
    assert_ne!(
        fnv1a_64(&swapped60),
        fnv1a_64(&flat60),
        "⭐ 对调提供表必须改变输出（否则'各播自己'可能是空转）"
    );

    // ③ 反向对照 2：**广播**重载在 key=64 上播的是 `a.wav` 的内容（内容错的机械证据）
    let broadcast = PreparedSfz::prepare(&sfz, &a, 48_000.0).expect("广播摊平");
    assert!(broadcast.is_broadcast());
    assert_eq!(broadcast.sample_content_label(), "broadcast(1)");
    assert_eq!(
        broadcast.regions()[1].sample_index(),
        0,
        "广播 ⇒ 全指第 0 份"
    );
    let mut bcast64 = vec![0.0f32; BLOCK];
    broadcast.render_block_into(64, 127, &mut bcast64);
    let mut ref_a_at_64 = vec![0.0f32; BLOCK];
    offline_one("a.wav", 64, &a, 64, &mut ref_a_at_64);
    println!(
        "[engine-sfz/S11] 广播在 key=64：指纹 = 0x{:016x}（多采样 = 0x{:016x}）",
        fnv1a_64(&bcast64),
        fnv1a_64(&flat64),
    );
    assert_eq!(
        bits(&bcast64),
        bits(&ref_a_at_64),
        "⚠ 广播路径在 key=64 上播的是 `a.wav`（**音色内容错**）—— 这正是已登记的局限"
    );
    assert_ne!(
        fnv1a_64(&bcast64),
        fnv1a_64(&flat64),
        "⭐ 广播与多采样在 key=64 上必须**不同**（否则多采样没生效）"
    );
}

/// S12（步 3）：⭐ **同一样本引两次只 materialize／解码一次**。
///
/// # 判定量
///
/// * `sample_count()`（＝摊平期的样本 clone 次数，"解码次数"的机械读数）与
///   `pcm_bytes()`（＝ PCM 字节数）；
/// * 反向对照：两条**不同**路径 ⇒ 计数与字节都翻倍；
/// * 提供表里同一路径出现**两份** ⇒ 首个为准（内容仍是第一份）。
#[test]
fn s12_the_same_sample_referenced_twice_is_materialized_once() {
    const SAME_TWICE: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n\
                              <region> sample=a.wav key=64 pitch_keycenter=60\n";
    const TWO_PATHS: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n\
                             <region> sample=c.wav key=64 pitch_keycenter=60\n";
    let a = InMemorySample::sine(440.0, 48_000.0, SAMPLE_FRAMES);
    let c = InMemorySample::sine(660.0, 48_000.0, SAMPLE_FRAMES);
    let provided = vec![("a.wav".to_owned(), a.clone())];
    let once = SfzInstrument::load(SAME_TWICE.to_owned(), &Default::default()).expect("解析");
    let twice = SfzInstrument::load(TWO_PATHS.to_owned(), &Default::default()).expect("解析");

    let deduped =
        PreparedSfz::prepare_with_samples(&once, &provided, 48_000.0).expect("摊平必须成功");
    let two_provided = vec![
        ("a.wav".to_owned(), a.clone()),
        ("c.wav".to_owned(), c.clone()),
    ];
    let doubled =
        PreparedSfz::prepare_with_samples(&twice, &two_provided, 48_000.0).expect("摊平必须成功");

    let one_copy = SAMPLE_FRAMES * core::mem::size_of::<f32>();
    println!(
        "[engine-sfz/S12] 同一路径引两次：sample_count = {}、pcm_bytes = {}（一份 = {one_copy}）；\
         两条不同路径：sample_count = {}、pcm_bytes = {}",
        deduped.sample_count(),
        deduped.pcm_bytes(),
        doubled.sample_count(),
        doubled.pcm_bytes(),
    );
    assert_eq!(
        deduped.region_count(),
        2,
        "两条区域都在（去重的是**样本**，⛔ 不是区域）"
    );
    assert_eq!(deduped.regions()[0].sample_index(), 0);
    assert_eq!(
        deduped.regions()[1].sample_index(),
        0,
        "⭐ 同一路径引两次 ⇒ 两条区域都指第 0 份（**只解码一次**）"
    );
    assert_eq!(deduped.sample_count(), 1, "⭐ 解码次数 ＝ 1");
    assert_eq!(
        deduped.pcm_bytes(),
        one_copy,
        "⭐ PCM 字节 ＝ **一份**（480 × 4 = {one_copy}）"
    );
    assert_eq!(doubled.sample_count(), 2, "反向对照：两条不同路径 ⇒ 2");
    assert_eq!(doubled.pcm_bytes(), 2 * one_copy, "反向对照：字节翻倍");
    assert_ne!(doubled.sample_count(), deduped.sample_count());

    // 提供表里同一路径两份（**内容不同**）⇒ 首个为准（⛔ 不静默合并内容）
    let second_a = InMemorySample::sine(880.0, 48_000.0, SAMPLE_FRAMES);
    let duplicated = vec![
        ("a.wav".to_owned(), a.clone()),
        ("a.wav".to_owned(), second_a),
    ];
    let first_wins =
        PreparedSfz::prepare_with_samples(&once, &duplicated, 48_000.0).expect("摊平必须成功");
    assert_eq!(
        first_wins.sample_count(),
        1,
        "同一路径两份 ⇒ 仍只 materialize 一份"
    );
    let mut out = vec![0.0f32; BLOCK];
    first_wins.render_block_into(60, 127, &mut out);
    let mut reference = vec![0.0f32; BLOCK];
    offline_one("a.wav", 60, &a, 60, &mut reference);
    assert_eq!(
        bits(&out),
        bits(&reference),
        "⭐ 提供表里同一路径出现两份时**首个为准**（内容不是第二份）"
    );
    // ⭐ 反向对照：第二份内容**确实不同**（否则上面的"首个为准"是空转）
    let mut second_reference = vec![0.0f32; BLOCK];
    offline_one(
        "a.wav",
        60,
        &InMemorySample::sine(880.0, 48_000.0, SAMPLE_FRAMES),
        60,
        &mut second_reference,
    );
    assert_ne!(
        fnv1a_64(&reference),
        fnv1a_64(&second_reference),
        "两份提供样本的内容必须真的不同（否则本条无鉴别力）"
    );
}

/// S13（步 3）：⭐ **`start`（`offset`）与 `end` 真正应用** —— 离线/摊平/实时三条路逐样本 0。
///
/// # 判定量
///
/// * 离线 vs 摊平**整块逐样本最大绝对差 ＝ 0**；
/// * `offset=10 end=200` ⇒ 写入帧数 **190**（半开 [10,201)），且输出第 0 帧
///   **逐位等于** `源[10] × gain_at(60,127)`；
/// * 反向对照：`offset=50` ⇒ 第 0 帧变成 `源[50] × gain` 且指纹不同（⛔ 证明起点真的进了循环）。
#[test]
fn s13_offset_and_end_are_applied_on_all_three_paths() {
    let a = InMemorySample::sine(440.0, 48_000.0, SAMPLE_FRAMES);
    let provided = vec![("a.wav".to_owned(), a.clone())];
    let span_sfz = SfzInstrument::load(
        "<region> sample=a.wav key=60 pitch_keycenter=60 offset=10 end=200\n".to_owned(),
        &Default::default(),
    )
    .expect("解析必须成功");
    let prepared =
        PreparedSfz::prepare_with_samples(&span_sfz, &provided, 48_000.0).expect("摊平必须成功");
    let region = &prepared.regions()[0];
    let gain = region.gain_at(60, 127);

    assert_eq!(
        region.source_start(SAMPLE_FRAMES),
        Some(10.0),
        "起点＝`offset`"
    );
    assert_eq!(
        region.source_end(SAMPLE_FRAMES),
        Some(201),
        "终点＝半开 [10,201)"
    );

    // ① 离线 vs 摊平：整块逐样本 0
    let mut offline = vec![0.0f32; BLOCK];
    span_sfz
        .render_note_into(&a, 60, 127, &mut offline, 48_000.0)
        .expect("离线渲染不得报错");
    let mut flat = vec![0.0f32; BLOCK];
    let flat_frames = prepared.render_block_into(60, 127, &mut flat);
    let diff = offline
        .iter()
        .zip(flat.iter())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max);

    // ② 实时声部也要在同一帧停、且与摊平逐样本 0
    let track = EntityId::new();
    let mut engine = engine_with_track(track);
    let schedule =
        NoteSchedule::from_sorted(vec![support::scheduled(0, 12_000, 60, 1.0, 48_000.0)]);
    let mut engine_out = vec![0.0f32; BLOCK];
    engine.render_track(track, Some(&schedule), Some(&prepared), &mut engine_out);
    let engine_diff = engine_out
        .iter()
        .zip(flat.iter())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max);

    let expected_first = a.pcm[10] * gain;
    println!(
        "[engine-sfz/S13] offset=10 end=200：摊平写入 = {flat_frames} 帧；离线 vs 摊平差 = {diff:e}；\
         引擎 vs 摊平差 = {engine_diff:e}；输出[0] = {}（源[10]×gain = {expected_first}）",
        flat[0]
    );
    assert_eq!(diff, 0.0, "⭐ 离线 vs 摊平必须逐样本相同（`span` 同口径）");
    assert_eq!(engine_diff, 0.0, "⭐ 引擎声部 vs 摊平必须逐样本相同");
    assert_eq!(flat_frames, 190, "半开 [10,201) ⇒ 190 帧");
    assert_eq!(
        flat[0].to_bits(),
        expected_first.to_bits(),
        "⭐ **起点真正应用**：输出第 0 帧必须逐位等于 `源[10] × gain`"
    );
    assert!(
        flat[190..].iter().all(|s| *s == 0.0),
        "⭐ `end` 真正应用：第 190 帧之后一帧不出"
    );

    // ③ 反向对照：换一个 `offset` ⇒ 起点与指纹都变
    let moved = SfzInstrument::load(
        "<region> sample=a.wav key=60 pitch_keycenter=60 offset=50 end=200\n".to_owned(),
        &Default::default(),
    )
    .expect("解析必须成功");
    let moved_plan =
        PreparedSfz::prepare_with_samples(&moved, &provided, 48_000.0).expect("摊平必须成功");
    let mut moved_out = vec![0.0f32; BLOCK];
    let moved_frames = moved_plan.render_block_into(60, 127, &mut moved_out);
    let expected_moved = a.pcm[50] * moved_plan.regions()[0].gain_at(60, 127);
    println!(
        "[engine-sfz/S13] 反向对照 offset=50：写入 = {moved_frames} 帧、输出[0] = {}（期望 {expected_moved}）",
        moved_out[0]
    );
    assert_eq!(
        moved_frames, 150,
        "半开 [50,201) ⇒ 151 个源采样 − 1（线性插值要两个样本）＝ 150 帧"
    );
    assert_eq!(
        moved_out[0].to_bits(),
        expected_moved.to_bits(),
        "⭐ `offset=50` ⇒ 起点必须跟着换"
    );
    assert_ne!(
        fnv1a_64(&moved_out),
        fnv1a_64(&flat),
        "⭐ 两个不同的 `offset` 必须给出不同波形（否则起点没进循环）"
    );
}

/// S14（步 3）：⭐ **单样本重载（广播）与步 2 逐位相同**。
///
/// 判定量：span-free 夹具 [`TWO_MATCH_ONE_MISS`] 的 `prepare`（广播）输出在
/// `(60, 127)`、512 帧上的 **FNV-1a 指纹** ＝ **步 2 实测** [`BROADCAST_STEP2_FINGERPRINT`]
/// （本机 `aarch64/macos` 上硬判；异平台如实跳过并点名原因）。
///
/// ⭐ 另附读数：`is_broadcast()` / `sample_content_label() == "broadcast(1)"`
/// ⇒ 报告里的 `sample_content=broadcast(1)` 有机械根据。
#[test]
fn s14_the_broadcast_overload_is_bit_identical_to_step_two() {
    let prepared = prepare(TWO_MATCH_ONE_MISS);
    let mut out = vec![0.0f32; BLOCK];
    let frames = prepared.render_block_into(60, 127, &mut out);
    println!(
        "[engine-sfz/S14] 广播路径 512 帧 @ (60,127)：写入 = {frames}、非零 = {}、\
         指纹 = 0x{:016x}（步 2 实测 = 0x{:016x}）、sample_content = {}",
        out.iter().filter(|s| **s != 0.0).count(),
        fnv1a_64(&out),
        BROADCAST_STEP2_FINGERPRINT,
        prepared.sample_content_label(),
    );
    assert!(out.iter().any(|s| *s != 0.0), "夹具必须真的出声");
    let _checked = check_fingerprint(
        "步 3 的广播重载 vs 步 2",
        fnv1a_64(&out),
        BROADCAST_STEP2_FINGERPRINT,
    );
    assert!(prepared.is_broadcast(), "`prepare` 必须标为广播");
    assert_eq!(
        prepared.sample_content_label(),
        "broadcast(1)",
        "⭐ 报告里的 `sample_content=broadcast(1)` 必须来自机械读数"
    );
    assert_eq!(prepared.sample_count(), 1);
    assert_eq!(prepared.silent_region_count(), 0);
    assert!(prepared.missing_sample_paths().is_empty());
    assert!(prepared.regions().iter().all(|r| r.sample_index() == 0));
}

/// S15（⭐ 本单改写，归属裁决见台账 R711）：⭐ **缺样本 ⇒ 静音 ＋ 计数**（⛔ 不静默、⛔ 不发明内容）。
///
/// # ⚠ 旧期望固化了**错内容**（如实说明）
///
/// 步 3 的 S15 断言的是"缺 `missing.wav` ⇒ 回退到 `samples[0]`，输出与'用 `a.wav` 播那个键'
/// **逐位相同**" ⇒ ⭐ 它把"缺样本的区域去播**别人的采样**"这条**内容错**钉成了期望值。
/// R711 裁决：**引擎出静音并计数**、**合成替代内容是控制侧的决定** ⇒ 本判据改为断言
/// **零输出 ＋ 计数正确**（新期望比旧期望**强**：旧的在"错内容"与"静音"之间分不出来）。
///
/// # 判定量（三条）
///
/// * 提供表**非空**但缺 `missing.wav` ⇒ 该区域 `NO_SAMPLE`：`silent_region_count() == 1`、
///   `missing_sample_paths() == ["missing.wav"]`、`sample_content_label() == "distinct(1)+silent(1)"`，
///   ⭐ 该键**一帧不写**（`render_block_into(64, 127)` 返回 **0** 且整块逐位 0）；
/// * 反向对照 ①：**同一份表**把 `missing.wav` 补上 ⇒ 同一个键**真的出声**且指纹 ≠ 0
///   （⇒ 上面那个 0 不是"这条区域本来就不响"）；
/// * 反向对照 ②：缺少的**只有** `missing.wav` ⇒ `a.wav` 那条区域（key=60）**照常出声**
///   （⇒ 静音的作用域**只到缺样本的那条区域**，⛔ 不是整条轨哑掉）；
/// * 提供表**为空** ⇒ `silent_region_count() == 2`、全部 `NO_SAMPLE`、**一帧不写**。
#[test]
fn s15_a_missing_sample_is_silent_and_is_counted() {
    const ONE_MISSING: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n\
                               <region> sample=missing.wav key=64 pitch_keycenter=60\n";
    let sfz = SfzInstrument::load(ONE_MISSING.to_owned(), &Default::default()).expect("解析");
    let a = InMemorySample::sine(440.0, 48_000.0, SAMPLE_FRAMES);
    let b = InMemorySample::sine(660.0, 48_000.0, SAMPLE_FRAMES);

    // ① 非空提供表 + 缺一份 ⇒ **静音** ＋ 计数（⛔ 不去播 `samples[0]`）
    let missing =
        PreparedSfz::prepare_with_samples(&sfz, &[("a.wav".to_owned(), a.clone())], 48_000.0)
            .expect("摊平必须成功");
    assert_eq!(missing.silent_region_count(), 1, "⭐ 缺样本的区域计数");
    assert_eq!(
        missing.missing_sample_paths().to_vec(),
        vec!["missing.wav".to_owned()],
        "⭐ 缺失路径必须**具名**登记（⛔ 不静默）"
    );
    assert_eq!(
        missing.sample_content_label(),
        "distinct(1)+silent(1)",
        "读数必须如实写出静音（⛔ 再没有 `+fallback(...)` 这一档）"
    );
    assert_eq!(
        missing.regions()[1].sample_index(),
        PreparedRegion::NO_SAMPLE,
        "⭐ 缺样本 ⇒ `NO_SAMPLE`（⛔ 不是回退到 `samples[0]`）"
    );
    assert!(
        !missing.regions()[1].has_sample(),
        "缺样本的区域必须**没有**样本可播"
    );
    let mut missing_out = vec![0.0f32; BLOCK];
    let missing_frames = missing.render_block_into(64, 127, &mut missing_out);
    // 反向对照 ② 的前置：**只有**缺样本的那条区域哑掉，key=60 那条照常出声。
    let mut sibling = vec![0.0f32; BLOCK];
    let sibling_frames = missing.render_block_into(60, 127, &mut sibling);
    println!(
        "[engine-sfz/S15] 缺样本：silent = {}、missing = {:?}、sample_content = {}、\
         key=64 写入 = {missing_frames}（整块非零 = {}）、key=60 写入 = {sibling_frames}",
        missing.silent_region_count(),
        missing.missing_sample_paths(),
        missing.sample_content_label(),
        missing_out.iter().filter(|s| **s != 0.0).count(),
    );
    assert_eq!(missing_frames, 0, "⭐ 缺样本 ⇒ **一帧不写**（引擎出静音）");
    assert!(
        missing_out.iter().all(|s| *s == 0.0),
        "⭐ 缺样本的区域必须**逐位静音**（⛔ 不许播别人的采样）"
    );
    assert!(
        sibling_frames > 0 && sibling.iter().any(|s| *s != 0.0),
        "反向对照②：缺的**只有** `missing.wav` ⇒ `a.wav` 那条区域必须照常出声"
    );

    // 反向对照 ①：把缺失那份补上 ⇒ 同一个键**真的出声**、指纹 ≠ 0
    let complete = PreparedSfz::prepare_with_samples(
        &sfz,
        &[
            ("a.wav".to_owned(), a.clone()),
            ("missing.wav".to_owned(), b.clone()),
        ],
        48_000.0,
    )
    .expect("摊平必须成功");
    let mut complete_out = vec![0.0f32; BLOCK];
    let complete_frames = complete.render_block_into(64, 127, &mut complete_out);
    assert!(
        complete.silent_region_count() == 0,
        "补上之后不得再有缺样本的区域"
    );
    assert!(complete.missing_sample_paths().is_empty());
    assert_ne!(
        complete_frames, 0,
        "⭐ 补上内容之后同一个键必须真的出声（否则上面那个 0 是空转）"
    );
    let mut reference = vec![0.0f32; BLOCK];
    offline_one("missing.wav", 64, &b, 64, &mut reference);
    assert_eq!(
        bits(&complete_out),
        bits(&reference),
        "补上的那一份必须**逐位**是它自己的内容（`b`，⛔ 不是 `a`）"
    );

    // ③ 空提供表 ⇒ 全部 NO_SAMPLE ＋ 计数 ＋ 一帧不写
    let empty = PreparedSfz::prepare_with_samples(&sfz, &[], 48_000.0).expect("摊平必须成功");
    let mut empty_out = vec![0.0f32; BLOCK];
    let empty_frames = empty.render_block_into(64, 127, &mut empty_out);
    println!(
        "[engine-sfz/S15] 空提供表：silent = {}、missing = {:?}、写入帧数 = {empty_frames}、\
         sample_content = {}",
        empty.silent_region_count(),
        empty.missing_sample_paths(),
        empty.sample_content_label(),
    );
    assert_eq!(empty.silent_region_count(), 2);
    assert!(empty.regions().iter().all(|r| !r.has_sample()));
    assert_eq!(empty_frames, 0, "没有样本 ⇒ 一帧不写");
    assert!(empty_out.iter().all(|s| *s == 0.0));
    assert_eq!(
        empty.missing_sample_paths().to_vec(),
        vec!["a.wav".to_owned(), "missing.wav".to_owned()],
        "缺失路径排序 ＋ 去重（确定性顺序）"
    );
}

/// S16（步 3）：⭐ **绑定注入 API 经公开路径可用（多采样）**。
///
/// 路径（全在集成测试里 ⇒ 只用 `pub` API）：
/// `SfzInstrument::referenced_sample_paths` →（控制侧解码 ⇒ 调用方）→
/// [`SfzPlan::prepare_with_samples`] → [`EngineSnapshot::with_sfz`] →
/// `render_snapshot`（快照槽 → 量子 → 声相 → 母线）。
///
/// 判定量：SFZ 臂非静音 ＋ 指纹 ≠ "不给 SFZ"；反向对照：**广播** plan 的端到端指纹与
/// 多采样版**不同**（⇒ 注入的多采样内容真的进了音频线程）。
#[test]
fn s16_the_multi_sample_binding_api_is_reachable_through_the_public_path() {
    let fixture = note_project(&E2E_NOTES);
    let track = fixture.track;
    let sfz =
        SfzInstrument::load(E2E_SFZ.to_owned(), &Default::default()).expect("SFZ 必须解析成功");

    // ① 控制侧：先查引用路径，再"解码"（判据里用内存样本；真读文件在 app 侧）
    let referenced = sfz.referenced_sample_paths().expect("路径查询");
    println!("[engine-sfz/S16] 引用路径 = {referenced:?}");
    assert_eq!(referenced, ["a.wav".to_owned(), "b.wav".to_owned()]);
    let a = InMemorySample::sine(440.0, 48_000.0, SAMPLE_FRAMES);
    let b = InMemorySample::sine(660.0, 48_000.0, SAMPLE_FRAMES);
    let plan = SfzPlan::prepare_with_samples(
        &sfz,
        &[
            ("a.wav".to_owned(), a.clone()),
            ("b.wav".to_owned(), b.clone()),
        ],
        48_000.0,
    )
    .expect("多采样摊平必须成功");
    assert_eq!(plan.prepared().sample_count(), 2);
    assert!(!plan.prepared().is_broadcast());

    // ② 注入（唯一公开绑定路径）＋ 端到端渲染
    let bare = EngineSnapshot::from_project(&fixture.project, 1).expect("快照");
    assert!(bare.sfz().is_empty(), "⛔ `from_project` 不得凭空绑 SFZ");
    let armed = EngineSnapshot::from_project(&fixture.project, 1)
        .expect("快照")
        .with_sfz(track, plan);
    let with_sfz = render_snapshot(armed, SFZ_QUANTA);
    let without = render(&fixture.project, SFZ_QUANTA);
    println!(
        "[engine-sfz/S16] 多采样臂: 非零={} 峰值={} 指纹=0x{:016x}；对照臂（不给 SFZ）: \
         非零={} 指纹=0x{:016x}",
        with_sfz.nonzero(),
        with_sfz.peak(),
        with_sfz.fingerprint(),
        without.nonzero(),
        without.fingerprint(),
    );
    assert!(with_sfz.nonzero() > 0, "⭐ 端到端必须非静音");
    assert_ne!(
        with_sfz.fingerprint(),
        without.fingerprint(),
        "⭐ 注入之后指纹必须不同（否则 SFZ 支路没接上）"
    );

    // ③ 反向对照：**广播** plan（只给 `a.wav`）⇒ 端到端指纹必须不同
    let broadcast_plan = SfzPlan::prepare(&sfz, &a, 48_000.0).expect("广播摊平");
    assert_eq!(
        broadcast_plan.prepared().sample_content_label(),
        "broadcast(1)"
    );
    let armed_broadcast = EngineSnapshot::from_project(&fixture.project, 1)
        .expect("快照")
        .with_sfz(track, broadcast_plan);
    let broadcast = render_snapshot(armed_broadcast, SFZ_QUANTA);
    println!(
        "[engine-sfz/S16] 广播臂: 非零={} 峰值={} 指纹=0x{:016x}",
        broadcast.nonzero(),
        broadcast.peak(),
        broadcast.fingerprint(),
    );
    assert!(
        broadcast.nonzero() > 0,
        "广播臂也必须出声（否则②的比较是空转）"
    );
    assert_ne!(
        broadcast.fingerprint(),
        with_sfz.fingerprint(),
        "⭐ 多采样 vs 广播的端到端指纹必须不同（`key=64` 那条区域换了内容）"
    );
}

// =====================================================================================
// ⭐ 归属裁决 `R711` 的两条落地判据（本单新增）：S17 = 轨推子（`needs-1`）、S18 = 防双重计。
// =====================================================================================

/// ⭐ S17／S18 用的**鼓机设备**：五个键位名写全 ⇒ `DrumsParams::from_devices` 认它是鼓机
/// （口径与 S7 的那个设备、与 `crates/yeban-engine/src/drums.rs` 的识别一致）。
fn drum_kit_device() -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: "Yeban Drums".to_owned(),
        kind: DeviceKind::InternalInstrument,
        bypassed: false,
        params: [
            ("kick_note", 60.0),
            ("snare_note", 38.0),
            ("closed_hat_note", 42.0),
            ("open_hat_note", 46.0),
            ("clap_note", 39.0),
        ]
        .iter()
        .map(|(name, value)| ParameterValue {
            name: (*name).to_owned(),
            value: *value,
            unit: None,
        })
        .collect(),
        latency_samples: 0,
    }
}

/// ⭐ S17 的轨参数夹具：`volume_db` ＋ `mute` ＋ "**另一条轨在独奏**"（可闻门的三个自变量）。
///
/// `drums = true` ⇒ 本轨挂鼓机设备（分派次序 ＝ 鼓机 ＞ SFZ ⇒ 该臂走鼓机）；
/// `drums = false` ⇒ 不挂任何设备（该臂可由调用方注入 SFZ 摊平表）。
/// ⚠ 两个臂的**音符、推子、静音/独奏**逐字相同 ⇒ 差异**只有音源**。
fn fader_fixture(
    volume_db: f32,
    mute: bool,
    solo_elsewhere: bool,
    drums: bool,
) -> support::Fixture {
    let mut fixture = tuned_project(&E2E_NOTES, MixSpec::volume(volume_db));
    let track = fixture.track;
    {
        let entry = fixture
            .project
            .tracks
            .get_mut(&track)
            .expect("夹具必须有那条 MIDI 轨");
        entry.mute = mute;
        if drums {
            entry.devices = vec![drum_kit_device()];
        }
    }
    if solo_elsewhere {
        let other = EntityId::new();
        fixture.project.tracks.insert(
            other,
            TrackV3 {
                id: other,
                name: "Solo".to_owned(),
                kind: TrackKind::Midi,
                solo: true,
                ..TrackV3::default()
            },
        );
        fixture.project.routing_graph.nodes.push(other);
    }
    fixture
}

/// 判据窗口（帧）：`E2E_NOTES` 的两个起音（0 与 48 tick = **1200** 帧）都落在窗口内。
const FADER_FRAMES: usize = 2560;

/// 本轨的**单声道时域输出**（`render_track` ⇒ **声相与母线之前**）。
///
/// # ⚠ 量什么（先说再跑）
///
/// 量的是**本轨的样本序列**（单位：线性幅度，48 kHz 单声道，2560 帧）。
///
/// ⚠ **为什么⛔ 不量端到端**：母线限制器是**非线性**器件 ⇒ 它会给出一个与轨增益无关的
/// 因子，把"轨增益的线性"测糊。本判据要证的是"轨增益乘了一次、且两件音源乘的是**同一个**值"
/// ⇒ 必须在这一级量。
///
/// 路径：`EngineSnapshot::from_project` → `SynthEngine::arm_from_snapshot`（**产品路径**，
/// 含 `set_track_gain_scales`）→ `render_track`。
fn track_output(fixture: &support::Fixture, sfz: Option<&PreparedSfz>) -> Vec<f32> {
    let track = fixture.track;
    let snapshot = EngineSnapshot::from_project(&fixture.project, 1).expect("夹具工程必须成快照");
    let mut engine = SynthEngine::new(48_000);
    engine
        .arm_from_snapshot(&snapshot)
        .expect("武装最小工程不得报错");
    let schedule = snapshot
        .schedules()
        .get(&track)
        .cloned()
        .expect("本轨必须有调度表");
    let mut out = vec![0.0f32; FADER_FRAMES];
    engine.render_track(track, Some(&schedule), sfz, &mut out);
    out
}

/// ⭐ 一个档的**两臂峰值** ＋ 两臂的时域缓冲：(SFZ 臂, 鼓机臂)。
fn two_arm_outputs(
    volume_db: f32,
    mute: bool,
    solo_elsewhere: bool,
    prepared: &PreparedSfz,
) -> (Vec<f32>, Vec<f32>) {
    let sfz_arm = track_output(
        &fader_fixture(volume_db, mute, solo_elsewhere, false),
        Some(prepared),
    );
    let drum_arm = track_output(&fader_fixture(volume_db, mute, solo_elsewhere, true), None);
    (sfz_arm, drum_arm)
}

/// 峰值（绝对值最大；单位：线性幅度）。
fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs()))
}

/// S17（⭐ 本单新增；归属裁决 `R711` 的 `needs-1` 落地）：⭐ **轨推子变化 N dB ⇒
/// SFZ 与鼓机输出同比例变**。
///
/// # 判定量（三条腿，都可机械判定）
///
/// 1. **推子 −6 dB**：`peak(−6) / peak(0)` 在**两臂**上都等于 [`track_gain`]`(−6.0)`
///    （相对误差 ≤ `1e-5`），且两臂的比值**互相**相等 ⇒ ⭐ "同径、同一个算子"；
/// 2. **静音**（`mute = true`）：两臂**逐位**为 `0.0`（⇒ 可闻门真的到了声部上）；
/// 3. **另一条轨独奏**（`any_solo = true` 且本轨既不 solo 也不 solo_safe）：两臂**逐位** `0.0`
///    （⇒ 独奏与静音走的是**同一个** `track_is_audible` 门）。
///
/// # 怎么变红
///
/// 让 SFZ 那条支路不再乘轨增益（把 `trigger_note` 的 `region.gain_at(...) * track_gain`
/// 改回 `region.gain_at(...)`）⇒ 腿 ① 的 SFZ 比值变回 `1.0`、腿 ②/③ 的 SFZ 臂**不再全零**。
/// ⚠ 腿 ① 本身⛔ 不能证明"鼓机也吃了推子"：那由**同一个** `track_gain_scale` 在
/// `snapshot::project_schedules` 里的一次调用保证（鼓机读含轨门的 `note.gain`）。
#[test]
fn s17_a_track_fader_scales_the_sfz_and_the_drums_arm_by_the_same_ratio() {
    let prepared = prepare(E2E_SFZ);

    // ③ 前置：**两臂都必须真的出声**（否则"同比例"可能是 0/0 那种空话）。
    let (sfz_0, drum_0) = two_arm_outputs(0.0, false, false, &prepared);
    let (sfz_6, drum_6) = two_arm_outputs(-6.0, false, false, &prepared);
    let (sfz_mute, drum_mute) = two_arm_outputs(0.0, true, false, &prepared);
    let (sfz_solo, drum_solo) = two_arm_outputs(0.0, false, true, &prepared);

    let expected = track_gain(-6.0);
    let sfz_ratio = peak(&sfz_6) / peak(&sfz_0);
    let drum_ratio = peak(&drum_6) / peak(&drum_0);
    println!(
        "[engine-sfz/S17] 峰值：SFZ 臂 0 dB = {:.9} / −6 dB = {:.9} / 静音 = {:.9} / 别轨独奏 = {:.9}；\
         鼓机臂 0 dB = {:.9} / −6 dB = {:.9} / 静音 = {:.9} / 别轨独奏 = {:.9}；\
         比值 SFZ = {sfz_ratio:.9} vs 鼓机 = {drum_ratio:.9}（期望 track_gain(−6 dB) = {expected:.9}）",
        peak(&sfz_0),
        peak(&sfz_6),
        peak(&sfz_mute),
        peak(&sfz_solo),
        peak(&drum_0),
        peak(&drum_6),
        peak(&drum_mute),
        peak(&drum_solo),
    );

    assert!(
        peak(&sfz_0) > 0.0 && peak(&drum_0) > 0.0,
        "0 dB 档两臂都必须真的出声（SFZ = {} / 鼓机 = {}）",
        peak(&sfz_0),
        peak(&drum_0)
    );
    assert!(
        peak(&sfz_6) > 0.0 && peak(&drum_6) > 0.0,
        "−6 dB 档两臂都必须仍然出声（⇒ 比值不是 0/0）"
    );
    // ① 推子比例：两臂各自 == `track_gain(-6)`，且两臂**互相**相等。
    let rel = |a: f32, b: f32| (a - b).abs() / b.abs();
    assert!(
        rel(sfz_ratio, expected) <= 1e-5,
        "⭐ SFZ 臂的推子比例必须 ＝ `track_gain(-6 dB)`：实得 {sfz_ratio} vs {expected}"
    );
    assert!(
        rel(drum_ratio, expected) <= 1e-5,
        "⭐ 鼓机臂的推子比例必须 ＝ `track_gain(-6 dB)`：实得 {drum_ratio} vs {expected}"
    );
    assert!(
        rel(sfz_ratio, drum_ratio) <= 1e-5,
        "⭐ ⭐ **同径**：SFZ 与鼓机的推子比例必须一致（{sfz_ratio} vs {drum_ratio}）"
    );
    // ② 静音档：两臂**逐位** 0（⛔ 不是"很小"）。
    assert!(
        sfz_mute.iter().all(|s| *s == 0.0) && drum_mute.iter().all(|s| *s == 0.0),
        "⭐ 静音轨 ⇒ 两臂都必须**逐位**静音（SFZ 峰值 = {}、鼓机峰值 = {}）",
        peak(&sfz_mute),
        peak(&drum_mute)
    );
    // ③ 独奏档：同一道可闻门。
    assert!(
        sfz_solo.iter().all(|s| *s == 0.0) && drum_solo.iter().all(|s| *s == 0.0),
        "⭐ 另一条轨独奏 ⇒ 本轨两臂都必须**逐位**静音（SFZ 峰值 = {}、鼓机峰值 = {}）",
        peak(&sfz_solo),
        peak(&drum_solo)
    );
}

/// S18（⭐ 本单新增；归属裁决 `R711` 的**防双重计**腿）：⭐ **引擎层只乘轨增益，
/// ⛔ 绝不替乐器层再乘一次力度**。
///
/// # 判定量
///
/// 每个力度 `v ∈ {0, 32, 64, 100, 127}`：**引擎声部路径**与**摊平表（乐器层）一次性渲染**
/// **逐位相同**。手工调度表里 `ScheduledNote::gain` 显式写成 `velocity_gain(v)`
/// （＝"如果引擎把力度也交给 SFZ，就会多乘一次"的那一份）⇒ 只要两侧逐位相同，
/// 就证明引擎**没有**碰力度、只乘了轨增益（本档轨增益 ＝ 默认 `1.0`）。
///
/// # ⚠ 反向对照：为什么必须用**中段**力度
///
/// `velocity_gain(127) == 1.0` 是**恒等** ⇒ 在 127 上"多乘一次力度"看不出来。
/// S1 那条腿正是 127 ⇒ ⭐ **S1 抓不住双重计**，本判据补的正是这个盲区（下面两条断言钉住它）。
///
/// # 怎么变红
///
/// 在 `SfzVoicePool::trigger_note` 里把增益改成
/// `region.gain_at(note, velocity) * track_gain * velocity_gain(velocity)` ⇒
/// `v ∈ {32, 64, 100}` 三条腿当场**逐位不等**（`v = 127` 那条仍绿 ⇒ 盲区真实存在）。
#[test]
fn s18_the_engine_never_applies_a_second_velocity_gain() {
    let prepared = prepare(TWO_MATCH_ONE_MISS);
    let mut rows = Vec::new();
    for velocity in [0u8, 32, 64, 100, 127] {
        let track = EntityId::new();
        let mut engine = engine_with_track(track);
        // ⚠ 手工调度表：轨增益门 ＝ `NoteSchedule::from_sorted` 的默认 **1.0**（单位）；
        //    逐音符 `gain` 显式写成 `velocity_gain(v)`（见判据文档）。
        let schedule = NoteSchedule::from_sorted(vec![ScheduledNote::new(
            0,
            12_000,
            60,
            velocity,
            support::yeban_dsp_note_to_hz(60),
            velocity_gain(velocity),
            48_000.0,
        )]);
        let mut engine_out = vec![0.0f32; BLOCK];
        engine.render_track(track, Some(&schedule), Some(&prepared), &mut engine_out);
        let mut flat = vec![0.0f32; BLOCK];
        prepared.render_block_into(60, velocity, &mut flat);
        rows.push(format!(
            "v={velocity}: 引擎 = {:#010x} / 摊平 = {:#010x}",
            fnv1a_64(&engine_out),
            fnv1a_64(&flat)
        ));
        assert_eq!(
            bits(&engine_out),
            bits(&flat),
            "⭐ 力度 {velocity}：引擎声部路径必须与**乐器层**一次性渲染**逐位相同**             （⇒ 引擎⛔ 没有替乐器层再乘一次力度）"
        );
    }
    println!("[engine-sfz/S18] 逐位对账 ⇒ {}", rows.join("；"));

    // 反向对照：`velocity_gain(127) == 1.0`（恒等）⇒ 127 那条腿对"双重计"**没有牙**；
    // 中段力度才有（下面这条断言就是 S18 必须存在、而 S1 不够的理由）。
    assert_eq!(velocity_gain(127), 1.0, "127 的力度增益必须是**恒等**");
    assert!(
        velocity_gain(64) > 0.0 && velocity_gain(64) < 1.0,
        "中段力度必须既非 0 也非恒等（实得 {}）",
        velocity_gain(64)
    );
    assert_ne!(
        velocity_gain(64),
        1.0,
        "⭐ 中段力度是**恒等**的话，本判据就没有鉴别力了"
    );
}

/// ⭐ S19 的夹具：`regions` 条 `key` 的区域 ＋ 一份**常数 PCM**（全 `1.0`）。
///
/// 常数 PCM 的理由：淡出是**逐帧增益**，常数源 ⇒ 输出**就是那条增益曲线**
/// （`out[k] == gain × fade_gain(k)`），读数可以直接与 `FLOOR^(n/N)` 对账。
fn constant_plan(regions: usize, key: u8) -> PreparedSfz {
    let mut text = String::new();
    for _ in 0..regions {
        text.push_str(&format!(
            "<region> sample=a.wav key={key} pitch_keycenter=60\n"
        ));
    }
    let sfz = SfzInstrument::load(text, &Default::default()).expect("SFZ 必须解析成功");
    let sample = InMemorySample {
        pcm: vec![1.0f32; SAMPLE_FRAMES],
        sample_hz: 48_000.0,
    };
    PreparedSfz::prepare(&sfz, &sample, 48_000.0).expect("摊平必须成功")
}

/// ⭐ S19（本单新增；`sfz-needs-2`）：⭐ **池满 ⇒ 窃取最旧的声部 ＋ 3 ms 指数淡出**。
///
/// # 夹具（为什么要三次触发）
///
/// * 第 1 次：**只**触发 1 条区域（`key=60`，终点 `LATE`）⇒ 槽 0、触发序号最小；
/// * 第 2 次：触发 15 条区域（`key=60`，终点 `EARLY = 64`）⇒ 填满槽 1..15；
/// * 第 3 次：触发 1 条区域（`key=64`）⇒ 池满 ⇒ **窃取最旧的**（槽 0）。
///
/// 从 `position = 64` 起只有槽 0 还在响（其余 15 个的引擎终点已过 ⇒ 渲染时如实退休）
/// ⇒ 本判据量的**就是被窃取者那条淡出曲线**。
///
/// # 判定量
///
/// * 窃取读数：`created == 0`（没有空槽）、`stolen == 1`、`dropped == 0`；
/// * `fade_remaining_at(0) == 144`（3 ms @ 48 kHz，与复音**同一把舵**）；
/// * 淡出窗口里槽 0 仍宣称**被窃取者**（`note == 60`）；走完 144 帧后**就地换成**
///   新声部（`note == 64`）且仍是活声部 ⇒ "被换掉"可读；
/// * **输出读数**：`out[143] / out[0] == FLOOR^(143/144)`（逐位对账）；
/// * **硬窃取对照臂**（`set_steal_fade_frames(0)`）：同一次触发 ⇒ 槽 0 **立刻**是
///   `note == 64`、输出**无衰减**（`out[143] == out[0]` 逐位）⇒ 两条臂指纹不同
///   （⇒ ②"淡出确实生效"不是空转）。
///
/// ⚠ 本判据的**旧行为**是"第 3 次直接丢弃"⇒ 池仍是 16 个 `note == 60` 的声部、
/// `dropped == 1`、`stolen == 0` ⇒ 本判据当场变红（注入读数见提交报告）。
#[test]
fn s19_a_full_pool_steals_the_oldest_voice_behind_a_three_millisecond_fade() {
    const LATE: u64 = 12_000;
    const EARLY: u64 = 64;
    let one = constant_plan(1, 60);
    let fifteen = constant_plan(SFZ_VOICES_PER_TRACK - 1, 60);
    let stealer = constant_plan(1, 64);

    let mut pool = SfzVoicePool::empty();
    assert_eq!(pool.steal_fade_frames(), 144, "3 ms @ 48 kHz");
    assert_eq!(pool.trigger_note(&one, 60, 100, LATE, 1.0), 1, "槽 0");
    assert_eq!(
        pool.trigger_note(&fifteen, 60, 100, EARLY, 1.0),
        SFZ_VOICES_PER_TRACK - 1,
        "填满槽 1..15"
    );
    assert_eq!(pool.active_count(), SFZ_VOICES_PER_TRACK);

    // ③ 池满 ⇒ 窃取最旧的（槽 0 的触发序号最小）。
    let created = pool.trigger_note(&stealer, 64, 100, LATE, 1.0);
    println!(
        "[engine-sfz/S19] 池满触发：新建 = {created}；窃取 = {}；丢弃 = {}；挂起 = {}；\
         槽 0 淡出剩余 = {}；槽 0 在场音高 = {}",
        pool.stolen(),
        pool.dropped(),
        pool.pending_count(),
        pool.fade_remaining_at(0),
        pool.voices()[0].note(),
    );
    assert_eq!(created, 0, "池满 ⇒ 没有空槽可占（窃取占的是别人的槽）");
    assert_eq!(pool.stolen(), 1);
    assert_eq!(pool.dropped(), 0, "⭐ 窃取⛔ 不是丢弃");
    assert_eq!(pool.pending_count(), 1, "新声部挂在槽 0 等接管");
    assert_eq!(pool.fade_remaining_at(0), 144, "⭐ 3 ms ＝ 144 帧 @ 48 kHz");
    assert_eq!(
        pool.voices()[0].note(),
        60,
        "淡出窗口里槽位必须仍宣称**被窃取者**（口径与 PolySynth 同款）"
    );
    assert_eq!(pool.active_count(), SFZ_VOICES_PER_TRACK, "16 个槽一个不空");

    // ④ 输出读数：从 position = 64 起只有槽 0 出声，144 帧正好走完淡出。
    let mut out = vec![0.0f32; 144];
    pool.render_into(&one, EARLY, &mut out);
    // ⚠ 期望值写成**字面量** `1e-3`（−60 dBFS）而不是去读实现里的常量：
    //    判据用实现自己的常量做期望值是自我确认（实现改错了也一起改"对"）。
    let decay = 1.0e-3f64.powf(1.0 / 144.0) as f32;
    let mut expected = 1.0f32;
    for _ in 0..143 {
        expected *= decay;
    }
    println!(
        "[engine-sfz/S19] 淡出：out[0] = {:.9}；out[143] = {:.9}（期望 FLOOR^(143/144)·out[0] = {:.9}）；\
         比值 = {:.9}",
        out[0],
        out[143],
        out[0] * expected,
        out[143] / out[0],
    );
    assert!(out[0] > 0.0, "淡出首帧必须出声（否则比值是 0/0）");
    assert_eq!(
        out[143].to_bits(),
        (out[0] * expected).to_bits(),
        "⭐ 第 143 帧必须逐位等于 `out[0] × FLOOR^(143/144)`（StealFade 的指数口径）"
    );
    assert!(
        out[143] / out[0] < 0.002,
        "⭐ 淡出末尾必须落到 −60 dBFS 一带（实得 {}）",
        out[143] / out[0]
    );

    // ⑤ 走完 144 帧 ⇒ 挂起者**就地接管**（同一个槽），仍是活声部。
    assert_eq!(
        pool.voices()[0].note(),
        64,
        "⭐ 淡出走完 ⇒ 最旧的声部被换掉（槽 0 变成新声部）"
    );
    assert_eq!(pool.pending_count(), 0, "挂起者已经接管");
    assert_eq!(pool.fade_remaining_at(0), 0);
    assert!(pool.voices()[0].is_active(), "接管者必须开始发声");
    let mut after = vec![0.0f32; 64];
    pool.render_into(&one, EARLY + 144, &mut after);
    assert!(
        after.iter().any(|s| *s != 0.0),
        "⭐ 接管者必须真的出声（⛔ 不是把槽弄哑）"
    );

    // ⑥ 硬窃取对照臂：同样的触发序列，淡出帧数 = 0 ⇒ 立刻换人、输出无衰减。
    let mut hard = SfzVoicePool::empty();
    hard.set_steal_fade_frames(0);
    hard.trigger_note(&one, 60, 100, LATE, 1.0);
    hard.trigger_note(&fifteen, 60, 100, EARLY, 1.0);
    let hard_created = hard.trigger_note(&stealer, 64, 100, LATE, 1.0);
    let mut hard_out = vec![0.0f32; 144];
    hard.render_into(&one, EARLY, &mut hard_out);
    println!(
        "[engine-sfz/S19] 硬窃取对照：新建 = {hard_created}；槽 0 音高 = {}；淡出剩余 = {}；\
         指纹 软 = 0x{:016x} / 硬 = 0x{:016x}",
        hard.voices()[0].note(),
        hard.fade_remaining_at(0),
        fnv1a_64(&out),
        fnv1a_64(&hard_out),
    );
    assert_eq!(hard.stolen(), 1, "硬窃取同样计一次窃取");
    assert_eq!(hard.dropped(), 0);
    assert_eq!(
        hard.voices()[0].note(),
        64,
        "⭐ 硬窃取（帧数 0）⇒ **立刻**接管，没有淡出窗口"
    );
    assert_eq!(hard.fade_remaining_at(0), 0);
    assert_eq!(
        hard_out[143].to_bits(),
        hard_out[0].to_bits(),
        "硬窃取没有衰减（逐位相同）"
    );
    assert_ne!(
        fnv1a_64(&out),
        fnv1a_64(&hard_out),
        "⭐ 软/硬两条臂的指纹必须不同（否则'淡出生效'可能是空转）"
    );
}

/// ⭐ S20（本单新增；`sfz-needs-3`）：⭐ **三件同轨的可读信号**（⛔ 不改分派次序）。
///
/// # 裁决（本单）
///
/// 次序**维持** `鼓机 ＞ SFZ ＞ 复音`（与 `crates/yeban-engine/src/drums.rs` §4.1
/// "一条轨至多一件音源"的既有裁决一致）⇒ ⛔ **不改成叠加**。真正缺的是
/// "用户看得出被忽略了" ⇒ 新增**投影读数** [`SynthEngine::armed_sources`]
/// ／ [`SynthEngine::ignored_sources`]（`render_track` 每量子写一个 `u8`）。
///
/// # 判定量
///
/// * 三件同轨（鼓机设备 ＋ 注入 SFZ ＋ **恒在场**的复音）⇒ `armed_sources == 3`
///   ＋ `ignored_sources == 2`（⭐ 本单要求的"被忽略的音源计数 ＝ 2"）；
/// * 输出仍对"只有鼓机"**逐位相同**（S7 的复核腿 ⇒ 信号⛔ 不改变输出）；
/// * 反向对照：鼓机＋复音 ⇒ `2 / 1`；只有复音 ⇒ `1 / 0`；**没渲染过**的轨 ⇒ `None`。
#[test]
fn s20_three_sources_on_one_track_report_two_ignored() {
    let prepared = prepare(TWO_MATCH_ONE_MISS);
    let track = EntityId::new();
    let params = DrumsParams::from_devices(&[drum_kit_device()]).expect("鼓机设备必须命中");
    let schedule =
        NoteSchedule::from_sorted(vec![support::scheduled(0, 12_000, 60, 1.0, 48_000.0)]);

    let mut three = SynthEngine::new(48_000);
    let mut drums_only = SynthEngine::new(48_000);
    let mut poly_only = SynthEngine::new(48_000);
    let mut never_rendered = SynthEngine::new(48_000);
    three.begin_snapshot(48_000, &[track], [], [(&track, &params)]);
    drums_only.begin_snapshot(48_000, &[track], [], [(&track, &params)]);
    poly_only.begin_snapshot(48_000, &[track], [], []);
    never_rendered.begin_snapshot(48_000, &[track], [], []);

    let mut a = vec![0.0f32; 128];
    let mut b = vec![0.0f32; 128];
    let mut c = vec![0.0f32; 128];
    three.render_track(track, Some(&schedule), Some(&prepared), &mut a);
    drums_only.render_track(track, Some(&schedule), None, &mut b);
    poly_only.render_track(track, Some(&schedule), None, &mut c);

    println!(
        "[engine-sfz/S20] 三件同轨：武装 = {:?}；被忽略 = {:?}（鼓机＋SFZ 非零 = {}）；\
         鼓机＋复音：武装 = {:?}／被忽略 = {:?}；只有复音：武装 = {:?}／被忽略 = {:?}；\
         没渲染过：武装 = {:?}",
        three.armed_sources(track),
        three.ignored_sources(track),
        a.iter().filter(|s| **s != 0.0).count(),
        drums_only.armed_sources(track),
        drums_only.ignored_sources(track),
        poly_only.armed_sources(track),
        poly_only.ignored_sources(track),
        never_rendered.armed_sources(track),
    );
    assert_eq!(
        three.armed_sources(track),
        Some(3),
        "⭐ 鼓机 ＋ SFZ ＋ 复音 ⇒ 武装 3 件"
    );
    assert_eq!(
        three.ignored_sources(track),
        Some(2),
        "⭐ 三件同轨 ⇒ **被忽略的音源计数 ＝ 2**（本单的可读信号）"
    );
    assert!(
        b.iter().any(|s| *s != 0.0),
        "反向对照：鼓机臂必须真的出声（否则下面的等号可能只是两边都哑）"
    );
    assert_eq!(
        bits(&a),
        bits(&b),
        "⭐ 信号⛔ 不改变输出：三件同轨仍对'只有鼓机'**逐位相同**（S7 的复核腿）"
    );
    assert_eq!(
        drums_only.armed_sources(track),
        Some(2),
        "反向对照：鼓机 ＋ 复音 ⇒ 武装 2 件"
    );
    assert_eq!(drums_only.ignored_sources(track), Some(1));
    assert_eq!(
        poly_only.armed_sources(track),
        Some(1),
        "反向对照：只有复音 ⇒ 武装 1 件、没有取舍"
    );
    assert_eq!(poly_only.ignored_sources(track), Some(0));
    assert_eq!(
        never_rendered.armed_sources(track),
        None,
        "⛔ 没渲染过的槽必须读 `None`（不是'0 件'）"
    );
}

// ---------------------------------------------------------------------------
// ⭐ 任务单 `#78`：引擎侧**消费** `TrackV3.sampler`（⭐ 只接线、⛔ 不读盘）
// ---------------------------------------------------------------------------

/// S21／S22／S23 的路径夹具：`instruments/lead.sfz`。
///
/// ⚠ 它**刻意在磁盘上不存在**（判据不碰文件系统）：引擎侧⛔ 不读盘 ⇒ 这个路径无论
/// 真假都⛔ 不该改变任何东西（⛔ 不报错、⛔ 不建声部、⛔ 不改渲染）—— 那正是
/// "接线 ⛔ 不是读盘"的机械见证。
const SAMPLER_PATH: &str = "instruments/lead.sfz";

/// S24 的块长：**小于**样本帧数（`SAMPLE_FRAMES = 480`）⇒ 有样本的声部在这一次
/// 渲染里**不会播完退休** ⇒ `sfz_active_voices` 才有区分度（`BLOCK = 512` 会让
/// 声部在同一块里播完，两条腿都会读成 `0` —— 那会让"不建声部"这条腿失去牙齿）。
const S24_BLOCK: usize = 128;

/// 把夹具工程里那条 MIDI 轨的 `sampler` 设成给定值（`None` ⇒ 清空），其余一字不动。
///
/// ⭐ "只差这一个字段"是 S21／S23 反向对照成立的**前提**：两条臂的工程除 `sampler`
/// 之外完全相同（同一份夹具、同一批音符、同一条路由）。
fn project_with_sampler(fixture: &support::Fixture, sampler: Option<&str>) -> YebanProjectV1 {
    let mut project = fixture.project.clone();
    project
        .tracks
        .get_mut(&fixture.track)
        .expect("夹具里必须有那条 MIDI 轨")
        .sampler = sampler.map(|path| SamplerBinding {
        path: path.to_owned(),
    });
    project
}

/// ⭐ S21（任务单 `#78`；⭐ 本单核心的**反向对照**）：`sampler` 缺席 ⇒ 快照与渲染与
/// **接线前**逐位相同。
///
/// # 判定量
///
/// 两条臂（无设备链 / 带音色）的端到端 **FNV-1a 指纹**必须等于 [`BASELINE_PLAIN`]／
/// [`BASELINE_TUNED`] —— 那两个字面值是**接线前**（HEAD `720db9f1`，即本单的第一条
/// 命令之后）在同一夹具、同一量子数、同一条产品路径上实测的（同 S5 的口径）。
/// 另加一条**结构腿**：`sampler = None` ⇒ `sfz_pending()` 必须**空**。
///
/// # 怎么变红
///
/// 在 `from_project_with_latencies` 里把 `track.sampler` 也塞进**摊平表**（或让
/// 实时侧读 `sfz_pending`）⇒ 第二条腿（`sfz_pending()` 空）与第一条腿（指纹）当场红。
#[test]
fn s21_an_absent_sampler_is_bit_identical_to_before_the_wiring() {
    let plain = note_project(&SFZ_NOTES);
    let plain_snapshot = EngineSnapshot::from_project(&plain.project, 1).expect("快照");
    assert!(
        plain_snapshot.sfz_pending().is_empty(),
        "⛔ `sampler = None` ⇒ 待摊平表必须为空（实得 {} 条）",
        plain_snapshot.sfz_pending().len()
    );
    assert!(plain_snapshot.pending_sfz(&plain.track).is_none());

    let rendered = render(&plain.project, SFZ_QUANTA);
    assert!(rendered.nonzero() > 0, "夹具必须真的出声（否则等号是空转）");
    println!(
        "[engine-sfz/S21] `sampler = None` 无设备链: 帧={} 非零={} 峰值={} 指纹={} (0x{:016x})",
        rendered.frames(),
        rendered.nonzero(),
        rendered.peak(),
        rendered.fingerprint(),
        rendered.fingerprint(),
    );
    let plain_checked = check_fingerprint(
        "`sampler = None` 无设备链",
        rendered.fingerprint(),
        BASELINE_PLAIN,
    );

    let tuned = tuned_project(&SFZ_NOTES, MixSpec::tone(800.0, 0.3));
    let tuned_snapshot = EngineSnapshot::from_project(&tuned.project, 1).expect("快照");
    assert!(tuned_snapshot.sfz_pending().is_empty());
    let rendered_tuned = render(&tuned.project, SFZ_QUANTA);
    println!(
        "[engine-sfz/S21] `sampler = None` 音色(800, 0.3): 帧={} 非零={} 峰值={} 指纹={} (0x{:016x})",
        rendered_tuned.frames(),
        rendered_tuned.nonzero(),
        rendered_tuned.peak(),
        rendered_tuned.fingerprint(),
        rendered_tuned.fingerprint(),
    );
    let tuned_checked = check_fingerprint(
        "`sampler = None` 音色(800, 0.3)",
        rendered_tuned.fingerprint(),
        BASELINE_TUNED,
    );

    assert_eq!(
        plain_checked, tuned_checked,
        "两条臂必须在**同一平台判定**下（否则本判据的读数不可比）"
    );
}

/// ⭐ S22（任务单 `#78`；**正向**）：`sampler = Some(路径)` ⇒ 快照里出现**该轨**的
/// 待摊平项，路径**逐字**相同。
///
/// # 判定量
///
/// * `sfz_pending()` **恰好 1 条**、键就是那条 MIDI 轨（母线⛔ 不在表里）；
/// * `pending_sfz(track).path()` **逐字**等于模型里那个路径（⛔ 不规范化、⛔ 不改写）；
/// * ⛔ **摊平表 `sfz()` 仍空** —— 引擎没有凭空摊平（那需要读盘）；
/// * 反向对照：同一夹具把 `sampler` 清回 `None` ⇒ 待摊平表**空**。
#[test]
fn s22_a_sampler_path_lands_in_the_snapshot_as_a_pending_item() {
    let fixture = note_project(&SFZ_NOTES);
    let project = project_with_sampler(&fixture, Some(SAMPLER_PATH));
    let snapshot = EngineSnapshot::from_project(&project, 1).expect("快照");

    let paths: Vec<&str> = snapshot
        .sfz_pending()
        .values()
        .map(PendingSfz::path)
        .collect();
    println!(
        "[engine-sfz/S22] 待摊平条数 = {}；路径 = {paths:?}；摊平表条数 = {}；\
         母线在待摊平表里 = {}",
        snapshot.sfz_pending().len(),
        snapshot.sfz().len(),
        snapshot.sfz_pending().contains_key(&fixture.master),
    );

    assert_eq!(
        snapshot.sfz_pending().len(),
        1,
        "⭐ `sampler = Some(…)` ⇒ 恰好一条待摊平项"
    );
    let pending = snapshot
        .pending_sfz(&fixture.track)
        .expect("⭐ 正向：那条 MIDI 轨必须出现在待摊平表里");
    assert_eq!(
        pending.path(),
        SAMPLER_PATH,
        "⭐ 路径必须**逐字**来自模型（⛔ 不规范化、⛔ 不查存在性）"
    );
    assert!(
        !snapshot.sfz_pending().contains_key(&fixture.master),
        "没绑定 `sampler` 的轨（母线）⛔ 不得出现在待摊平表里"
    );
    assert!(
        snapshot.sfz().is_empty(),
        "⛔ 引擎不读盘 ⇒ 摊平表必须仍为空（只有控制侧解码后 `with_sfz` 才填它）"
    );

    // 反向对照：同一夹具、只把 `sampler` 清回 `None` ⇒ 待摊平表空。
    let cleared = project_with_sampler(&fixture, None);
    let cleared_snapshot = EngineSnapshot::from_project(&cleared, 1).expect("快照");
    assert!(
        cleared_snapshot.sfz_pending().is_empty(),
        "反向对照：`sampler = None` ⇒ 待摊平表必须为空"
    );
}

/// ⭐ S23（任务单 `#78`）：**待摊平项是纯事实** —— 它⛔ 不改渲染、⛔ 不报错；
/// 而 `with_sfz` 注入摊平表时**取代**它。
///
/// # 判定量
///
/// 1. 同夹具 `None`／`Some(不存在的路径)` 两条臂的端到端 FNV-1a 指纹**逐位相同**
///    （且都真的出声）⇒ ⭐ 引擎⛔ 没有第二份音源事实源；
/// 2. 那个路径**磁盘上不存在**，但 `from_project` 仍 `Ok` ⇒ ⭐ 引擎⛔ 不读盘
///    （真读盘会 `Err` 或至少要碰文件系统）；
/// 3. 控制侧注入摊平表（`with_sfz`）⇒ 待摊平项被**取代**（表清空、`sfz()` 一条），
///    且指纹**变**（⇒ 摊平表真的接上了实时侧）。
///
/// # 怎么变红
///
/// 把 `with_sfz` 里那句 `self.sfz_pending.remove(&track)` 删掉 ⇒ 腿 ③ 的"表清空"红；
/// 让 `from_project` 去读那个路径 ⇒ 腿 ② 红（不存在的文件必然 `Err`）。
#[test]
fn s23_a_pending_path_changes_nothing_audible_and_is_superseded_by_with_sfz() {
    let fixture = note_project(&SFZ_NOTES);
    let bare_project = project_with_sampler(&fixture, None);
    let pending_project = project_with_sampler(&fixture, Some(SAMPLER_PATH));

    let bare = render(&bare_project, SFZ_QUANTA);
    let with_pending = render(&pending_project, SFZ_QUANTA);
    println!(
        "[engine-sfz/S23] `sampler = None`: 非零={} 指纹={} (0x{:016x})；\
         `sampler = Some({SAMPLER_PATH})`（文件不存在）: 非零={} 指纹={} (0x{:016x})",
        bare.nonzero(),
        bare.fingerprint(),
        bare.fingerprint(),
        with_pending.nonzero(),
        with_pending.fingerprint(),
        with_pending.fingerprint(),
    );
    assert!(
        bare.nonzero() > 0 && with_pending.nonzero() > 0,
        "两条臂都必须真的出声（否则'逐位相同'可能只是两边都哑）"
    );
    assert_eq!(
        with_pending.fingerprint(),
        bare.fingerprint(),
        "⭐ 待摊平项⛔ 不改渲染：`None` 与 `Some(…)` 的输出必须**逐位相同**"
    );
    assert_eq!(
        bits(&with_pending.left),
        bits(&bare.left),
        "⭐ 左声道逐位相同（指纹之外的独立见证）"
    );

    // ② 不存在的路径仍能编译成快照 ⇒ ⭐ 引擎⛔ 不读盘。
    let snapshot = EngineSnapshot::from_project(&pending_project, 1)
        .expect("⭐ 引擎不读盘 ⇒ 磁盘上不存在的 SFZ 路径也必须能编译成快照");
    assert!(snapshot.sfz().is_empty(), "⛔ 没有摊平表（引擎没读盘）");

    // ③ 控制侧（允许 I/O）解码后注入 ⇒ 摊平表**取代**待摊平项。
    let sfz = SfzInstrument::load(
        "<region> sample=a.wav key=60 pitch_keycenter=60\n".to_owned(),
        &Default::default(),
    )
    .expect("SFZ 必须解析成功");
    let sample = InMemorySample::sine(440.0, 48_000.0, SAMPLE_FRAMES);
    let plan = SfzPlan::prepare_with_samples(&sfz, &[("a.wav".to_owned(), sample)], 48_000.0)
        .expect("多采样摊平必须成功");
    let armed = snapshot.with_sfz(fixture.track, plan);
    println!(
        "[engine-sfz/S23] 注入摊平表之后：待摊平条数 = {}；摊平表条数 = {}",
        armed.sfz_pending().len(),
        armed.sfz().len()
    );
    assert!(
        armed.sfz_pending().is_empty(),
        "⭐ 摊平表**取代**待摊平项（否则控制侧会重复解码同一条轨）"
    );
    assert_eq!(armed.sfz().len(), 1, "注入一次 ⇒ 摊平表一条");
    let armed_audio = render_snapshot(armed, SFZ_QUANTA);
    assert_ne!(
        armed_audio.fingerprint(),
        bare.fingerprint(),
        "⭐ 摊平表必须真的接上实时侧（否则'取代'只是账面动作）"
    );
}

/// ⭐ S24（任务单 `#78`）：**缺样本口径不变**，且它能**经新接线**走到。
///
/// 前三条判据只证明"路径进快照"；本条证明⭐ **新接线与控制侧既有分工衔接得上**：
/// 由 `sfz_pending()` 里的路径 →（控制侧"读盘"／解析）→ 只解码 `a.wav` →
/// `prepare_with_samples` → `render_track`：缺 `missing.wav` 的那条区域
/// **`NO_SAMPLE` ⇒ 不建声部 ＋ 计数**（既有口径，`R711`／`needs-7`）。
///
/// # 判定量
///
/// * `silent_region_count() == 1`、`missing_sample_paths() == ["missing.wav"]`、
///   该区域 `sample_index == PreparedRegion::NO_SAMPLE`；
/// * 引擎侧：key=64（缺样本）那次 `render_track` ⇒ `sfz_active_voices == Some(0)`
///   ＋ 输出**逐位 0**（⛔ 不建声部、⛔ 不回退播别人的采样）；
/// * 反向对照：同一份 plan 上 key=60（有样本）**真的出声** ⇒ 上一条不是"两边都哑"。
#[test]
fn s24_a_missing_sample_is_still_silent_and_counted_through_the_new_wiring() {
    const ONE_MISSING: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n\
                               <region> sample=missing.wav key=64 pitch_keycenter=60\n";
    let fixture = note_project(&SFZ_NOTES);
    let project = project_with_sampler(&fixture, Some(SAMPLER_PATH));
    let snapshot = EngineSnapshot::from_project(&project, 1).expect("快照");

    // 控制侧（模拟"读盘"）：路径必须**先**从待摊平项取到。
    let pending = snapshot
        .pending_sfz(&fixture.track)
        .expect("⭐ 待摊平项是控制侧的唯一入口");
    assert_eq!(pending.path(), SAMPLER_PATH);
    let sfz = SfzInstrument::load(ONE_MISSING.to_owned(), &Default::default()).expect("解析");
    assert_eq!(
        sfz.referenced_sample_paths().expect("路径全集"),
        ["a.wav".to_owned(), "missing.wav".to_owned()],
        "控制侧据此知道该解码哪两份（`missing.wav` 读不到 ⇒ ⛔ 不放进提供表）"
    );
    let a = InMemorySample::sine(440.0, 48_000.0, SAMPLE_FRAMES);
    let plan = SfzPlan::prepare_with_samples(&sfz, &[("a.wav".to_owned(), a)], 48_000.0)
        .expect("多采样摊平必须成功");
    let prepared = plan.prepared();
    println!(
        "[engine-sfz/S24] silent = {}、missing = {:?}、sample_content = {}、\
         key=64 的 sample_index = {}",
        prepared.silent_region_count(),
        prepared.missing_sample_paths(),
        prepared.sample_content_label(),
        prepared.regions()[1].sample_index(),
    );
    assert_eq!(prepared.silent_region_count(), 1, "⭐ 缺样本的区域计数");
    assert_eq!(
        prepared.missing_sample_paths().to_vec(),
        vec!["missing.wav".to_owned()],
        "⭐ 缺失路径必须**具名**登记（⛔ 不静默）"
    );
    assert_eq!(
        prepared.regions()[1].sample_index(),
        PreparedRegion::NO_SAMPLE,
        "⭐ 缺样本 ⇒ `NO_SAMPLE`（⛔ 不是回退到 `samples[0]`）"
    );

    // 引擎侧：缺样本那个键**不建声部**（`sfz_active_voices == Some(0)` ＋ 逐位 0）。
    let engine_track = EntityId::new();
    let mut engine = engine_with_track(engine_track);
    let missing_note = NoteSchedule::from_sorted(vec![ScheduledNote::new(
        0,
        12_000,
        64,
        127,
        support::yeban_dsp_note_to_hz(64),
        1.0,
        48_000.0,
    )]);
    let mut missing_out = vec![0.0f32; S24_BLOCK];
    engine.render_track(
        engine_track,
        Some(&missing_note),
        Some(prepared),
        &mut missing_out,
    );
    assert_eq!(
        engine.sfz_active_voices(engine_track),
        Some(0),
        "⭐ 缺样本 ⇒ **不建声部**（池里 0 个活声部）"
    );
    assert!(
        missing_out.iter().all(|s| *s == 0.0),
        "⭐ 缺样本的区域必须**逐位静音**"
    );

    // 反向对照：同一份 plan，key=60（有样本）必须真的出声。
    // ⚠ 用一条**新的**引擎：`SynthEngine` 的每槽 `cursor` 单调前进（同 S18 每条腿
    // 各建一台的理由）⇒ 在同一个引擎上换一份调度表不会重新触发。
    let sibling_note = NoteSchedule::from_sorted(vec![ScheduledNote::new(
        0,
        12_000,
        60,
        127,
        support::yeban_dsp_note_to_hz(60),
        1.0,
        48_000.0,
    )]);
    let mut sibling_engine = engine_with_track(engine_track);
    let mut sibling_out = vec![0.0f32; S24_BLOCK];
    sibling_engine.render_track(
        engine_track,
        Some(&sibling_note),
        Some(prepared),
        &mut sibling_out,
    );
    println!(
        "[engine-sfz/S24] 反向对照 key=60：活声部 = {:?}、非零 = {}",
        sibling_engine.sfz_active_voices(engine_track),
        sibling_out.iter().filter(|s| **s != 0.0).count(),
    );
    assert!(
        sibling_engine.sfz_active_voices(engine_track).unwrap_or(0) > 0
            && sibling_out.iter().any(|s| *s != 0.0),
        "反向对照：缺的**只有** `missing.wav` ⇒ key=60 那条区域必须照常出声"
    );
}

// ---------------------------------------------------------------------------
// ⭐ S25／S26／S27：摊平表的**字节预算**（任务单 `#84`；`sfz-needs-8` 的"可判"那一半）
//
// 背景（`sfz-needs-8` 逐字）："摊平表无字节预算／无流式（样本 PCM 整段 clone）"。
// ⭐ 本片要的**不是**流式，而是"让它可判"：一条**读数**（拆成区域表／PCM／路径文本）
// ＋ 一条**判据**（`bytes ≤ N`，`N` 的来源可复算）＋ 一条**超预算口径**
// （引擎只报数、控制侧决定 —— 理由见 `PreparedSfz::budget_overrun`）。
// ---------------------------------------------------------------------------

/// ⭐ 字节预算夹具的**声明形状**（这三个常量就是"这份音色库有多大"的声明）。
///
/// * `BUDGET_FIXTURE_REGIONS = 3` 条区域；
/// * 去重后 **2** 份样本：`a.wav` 480 帧、`b.wav` 1920 帧 ⇒ PCM ＝ `(480 + 1920) × 4`；
/// * 路径文本 ＝ `"a.wav" + "b.wav"`。
const BUDGET_FIXTURE_REGIONS: usize = 3;
const BUDGET_FIXTURE_A_FRAMES: usize = 480;
const BUDGET_FIXTURE_B_FRAMES: usize = 1920;
const BUDGET_FIXTURE_PCM_FRAMES: usize = BUDGET_FIXTURE_A_FRAMES + BUDGET_FIXTURE_B_FRAMES;
const BUDGET_FIXTURE_PATH_BYTES: usize = "a.wav".len() + "b.wav".len();

/// ⭐ 字节预算夹具的 SFZ 文本：两条区域引 `a.wav`（不同键）、一条引 `b.wav`
/// ⇒ **去重后 2 份样本**（去重口径本身由 S12 钉住，这里只借它造出"样本 < 区域"的形状）。
const BUDGET_FIXTURE_SFZ: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n\
                                  <region> sample=a.wav key=62 pitch_keycenter=62\n\
                                  <region> sample=b.wav key=64 pitch_keycenter=64\n";

/// ⭐ 建一份**声明形状已知**的摊平表（S25／S26／S27 共用）。
fn budget_fixture_plan() -> PreparedSfz {
    let instrument = SfzInstrument::load(BUDGET_FIXTURE_SFZ.to_owned(), &Default::default())
        .expect("字节预算夹具的 SFZ 必须解析成功");
    PreparedSfz::prepare_with_samples(
        &instrument,
        &[
            (
                "a.wav".to_owned(),
                InMemorySample::sine(440.0, 48_000.0, BUDGET_FIXTURE_A_FRAMES),
            ),
            (
                "b.wav".to_owned(),
                InMemorySample::sine(660.0, 48_000.0, BUDGET_FIXTURE_B_FRAMES),
            ),
        ],
        48_000.0,
    )
    .expect("字节预算夹具的摊平必须成功")
}

/// ⭐ S25（本单新增；`sfz-needs-8`）：摊平表的字节占用**拆成三项** ＋ 一条**可判的预算口径**。
///
/// # 量什么（对象 ＋ 单位，先说再跑）
///
/// 对象 ＝ 一份 [`PreparedSfz`]（[`budget_fixture_plan`]：3 条区域、2 份样本、2400 帧 PCM）；
/// 单位 ＝ **字节**。三项拆开：
///
/// | 项 | 换算 | 夹具读数（字节） |
/// | :--- | :--- | ---: |
/// | 区域表 | `size_of::<PreparedRegion>() × 3` | 由 `size_of` 现读（⛔ 不手抄） |
/// | PCM | `4 × 2400` | **9 600** |
/// | 路径文本 | `"a.wav" + "b.wav"` | **10** |
///
/// # `N` 的来源（⭐ 为什么是这个数）
///
/// `N = SfzByteBudget::for_layout(3, 2400, 10)`：由夹具**自己声明的形状**按
/// [`PreparedSfz::region_bytes`] ／ [`PreparedSfz::pcm_bytes`] ／ [`PreparedSfz::path_bytes`]
/// 那**同一份**换算反推 ⇒ 上界**恰好**是该形状的精确字节数。于是判据 "`bytes ≤ N`"
/// 钉住的是"**读数里没有隐藏的第四份开销**"（例如 `Vec` 的容量余量、或有人往
/// `PreparedRegion` 里加了大字段却没进读数），⛔ **不是**"某个数比某个拍出来的阈值小"。
/// 口径与 `yeban-decode` 的 `PcmBudget::for_layout` 同一形状（那边由 `pcm_bytes_for`
/// 一条换算给出，可被判据复算）。
///
/// # 判定量
///
/// * 三项**各自的精确读数**（⛔ 不是 `> 0` 那种地板断言）＋ 合计 ＝ 既有 `memory_bytes()`；
/// * `budget_overrun(&N).is_within() == true`（三项超出读数全 0）；
/// * ⭐ **内置反向腿（⛔ 让"绿"不能是空的）**：三个维度**各自** −1 字节 ⇒
///   该维度恰好报 **1** 字节超出、`is_within() == false`、且**只有**一个维度非 0。
#[test]
fn s25_the_flat_table_byte_reading_is_split_and_has_a_judgeable_budget() {
    let plan = budget_fixture_plan();
    let region_struct = core::mem::size_of::<PreparedRegion>();
    assert_eq!(
        plan.region_count(),
        BUDGET_FIXTURE_REGIONS,
        "⭐ 声明与夹具必须一致（否则下面的 N 就不是这份夹具的形状）"
    );

    // ① 现状读数（拆开）：区域表 ／ PCM ／ 路径文本 ／ 合计。
    let usage = plan.byte_usage();
    println!(
        "[engine-sfz/S25] 摊平表字节读数（拆开）：区域表 = size_of::<PreparedRegion>() {region_struct} B × \
         {BUDGET_FIXTURE_REGIONS} = {}；PCM = 4 B × {BUDGET_FIXTURE_PCM_FRAMES} = {}；\
         路径文本 = {}；合计 = {}（＝ memory_bytes() = {}）",
        usage.region_bytes(),
        usage.pcm_bytes(),
        usage.path_bytes(),
        usage.total_bytes(),
        plan.memory_bytes(),
    );

    // ② 三项各自的**精确**读数（⛔ 不是地板断言）。
    assert_eq!(
        usage.region_bytes(),
        region_struct * BUDGET_FIXTURE_REGIONS,
        "区域表 ＝ 结构体字节数 × 区域数"
    );
    assert_eq!(
        usage.pcm_bytes(),
        4 * BUDGET_FIXTURE_PCM_FRAMES,
        "PCM ＝ 4 B × 2400 帧（去重后 2 份样本）"
    );
    assert_eq!(
        usage.pcm_bytes(),
        plan.pcm_bytes(),
        "⭐ 与既有 `pcm_bytes()` **同一份求和**（⛔ 不是第二套口径）"
    );
    assert_eq!(
        usage.path_bytes(),
        BUDGET_FIXTURE_PATH_BYTES,
        "路径文本 ＝ `\"a.wav\" + \"b.wav\"`（⛔ 不含 `String` 容量余量）"
    );
    // ③ 合计 ＝ 既有 `memory_bytes()`（⛔ 不是第二套口径）。
    assert_eq!(
        usage.total_bytes(),
        plan.memory_bytes(),
        "⭐ 三项之和必须**逐字节等于**既有的 memory_bytes()"
    );
    assert_eq!(
        usage.total_bytes(),
        usage.region_bytes() + usage.pcm_bytes() + usage.path_bytes(),
        "合计 ＝ 三项之和"
    );

    // ④ 预算判据：bytes ≤ N（N 由夹具声明的形状反推；闭区间 ⇒ 恰好等于上界通过）。
    let declared = SfzByteBudget::for_layout(
        BUDGET_FIXTURE_REGIONS,
        BUDGET_FIXTURE_PCM_FRAMES,
        BUDGET_FIXTURE_PATH_BYTES,
    );
    println!(
        "[engine-sfz/S25] 预算 N = for_layout({BUDGET_FIXTURE_REGIONS}, {BUDGET_FIXTURE_PCM_FRAMES}, \
         {BUDGET_FIXTURE_PATH_BYTES}) = {{区域表 ≤ {}，PCM ≤ {}，路径 ≤ {}}}；\
         实际 = {{区域表 {}，PCM {}，路径 {}}} ⇒ 超出 = {{区域表 {}，PCM {}，路径 {}}}（is_within = {}）",
        declared.max_region_bytes(),
        declared.max_pcm_bytes(),
        declared.max_path_bytes(),
        usage.region_bytes(),
        usage.pcm_bytes(),
        usage.path_bytes(),
        plan.budget_overrun(&declared).region_bytes(),
        plan.budget_overrun(&declared).pcm_bytes(),
        plan.budget_overrun(&declared).path_bytes(),
        plan.budget_overrun(&declared).is_within(),
    );
    assert_eq!(
        declared.max_region_bytes(),
        usage.region_bytes(),
        "N 的区域表那一维必须恰好等于夹具的实际区域表字节数（同一份换算）"
    );
    assert_eq!(
        declared.max_pcm_bytes(),
        usage.pcm_bytes(),
        "N 的 PCM 那一维必须恰好等于夹具的实际 PCM 字节数"
    );
    assert_eq!(
        declared.max_path_bytes(),
        usage.path_bytes(),
        "N 的路径那一维必须恰好等于夹具的实际路径文本字节数"
    );
    let within = plan.budget_overrun(&declared);
    assert!(
        within.is_within(),
        "⭐ 夹具的实际占用必须 ≤ 声明形状反推的 N（超出读数 = {within:?}）"
    );
    assert_eq!(within.total_bytes(), 0, "在预算内 ⇒ 三项超出量全 0");

    // ⑤ ⭐ 内置反向腿：每个维度各 −1 字节 ⇒ 该维度恰好报 1 字节超出（⛔ 绿不是空的）。
    let tight = [
        (
            "区域表",
            SfzByteBudget::new(
                declared.max_region_bytes() - 1,
                declared.max_pcm_bytes(),
                declared.max_path_bytes(),
            ),
        ),
        (
            "PCM",
            SfzByteBudget::new(
                declared.max_region_bytes(),
                declared.max_pcm_bytes() - 1,
                declared.max_path_bytes(),
            ),
        ),
        (
            "路径文本",
            SfzByteBudget::new(
                declared.max_region_bytes(),
                declared.max_pcm_bytes(),
                declared.max_path_bytes() - 1,
            ),
        ),
    ];
    for (name, budget) in tight {
        let over = plan.budget_overrun(&budget);
        let nonzero = [over.region_bytes(), over.pcm_bytes(), over.path_bytes()]
            .iter()
            .filter(|bytes| **bytes != 0)
            .count();
        println!(
            "[engine-sfz/S25] 反向腿（{name} −1 B）⇒ 超出 = {{区域表 {}，PCM {}，路径 {}}}、\
             is_within = {}",
            over.region_bytes(),
            over.pcm_bytes(),
            over.path_bytes(),
            over.is_within(),
        );
        assert!(
            !over.is_within(),
            "⭐ {name} 预算 −1 字节 ⇒ 判据必须红（实得 {over:?}）"
        );
        assert_eq!(over.total_bytes(), 1, "⭐ {name}：恰好超出 1 字节");
        assert_eq!(
            nonzero, 1,
            "⭐ {name}：**只有**该维度非 0（其余两项在预算内）"
        );
    }
}

/// ⭐ S26（本单新增；`sfz-needs-8`）：**超预算会怎样** —— 引擎**只报数**。
///
/// 构造一份**真的超预算**的表：同一份夹具（PCM 9 600 字节），但预算按"只给 `a.wav`
/// 那一份"声明 ＝ `for_layout(3, 480, 10)` ⇒ PCM 超出 `9600 − 1920` ＝ **7 680** 字节。
///
/// # 三条腿（对应"超预算怎么办"的候选 ①②③）
///
/// 1. **⛔ 不报错**：`prepare_with_samples` 仍然返回 `Ok`（本测试能走到断言就是证据）；
/// 2. **⛔ 不截断 / 不改摊平结果**：`budget_overrun` 前后 —— `byte_usage` ／ `sample_count`
///    ／ `pcm_bytes` ／ `region_count` ／ **渲染输出**全部**相同**；
///    ⭐ 零预算（`new(0, 0, 0)`）也**只**报数（三项全报超），表一个字不变；
/// 3. **只报数**：读数**逐项**给出超出量（区域表 0 ／ PCM 7 680 ／ 路径 0）——
///    于是"超了多少、超在哪一项"是机械可读的（归属裁决与三条理由见
///    `PreparedSfz::budget_overrun`）。
#[test]
fn s26_over_budget_is_reported_not_refused() {
    let plan = budget_fixture_plan();
    let before = plan.byte_usage();
    let before_samples = plan.sample_count();
    let before_pcm = plan.pcm_bytes();
    let before_regions = plan.region_count();

    // 渲染参照：`budget_overrun` 不得改变任何渲染读数。
    let render = |plan: &PreparedSfz| {
        let mut out = vec![0.0f32; 512];
        let written = plan.render_block_into(64, 100, &mut out);
        (written, out)
    };
    let reference = render(&plan);

    // ① 真的超预算：预算按"只给 `a.wav` 那一份"声明。
    let only_a = SfzByteBudget::for_layout(
        BUDGET_FIXTURE_REGIONS,
        BUDGET_FIXTURE_A_FRAMES,
        BUDGET_FIXTURE_PATH_BYTES,
    );
    let over = plan.budget_overrun(&only_a);
    println!(
        "[engine-sfz/S26] 超预算读数：N = for_layout({BUDGET_FIXTURE_REGIONS}, {BUDGET_FIXTURE_A_FRAMES}, \
         {BUDGET_FIXTURE_PATH_BYTES}) ⇒ 超出 = {{区域表 {}，PCM {}，路径 {}}}、合计 {} 字节、is_within = {}；\
         ⭐ prepare_with_samples 仍 Ok（⛔ 不报错），样本数 = {}、PCM = {} 字节、区域数 = {}",
        over.region_bytes(),
        over.pcm_bytes(),
        over.path_bytes(),
        over.total_bytes(),
        over.is_within(),
        plan.sample_count(),
        plan.pcm_bytes(),
        plan.region_count(),
    );
    assert!(
        !over.is_within(),
        "⭐ 这份夹具对「只给 `a.wav`」的预算必须**超**（否则本判据无话可说）"
    );
    assert_eq!(over.region_bytes(), 0, "区域表那一维仍在预算内");
    assert_eq!(
        over.pcm_bytes(),
        4 * BUDGET_FIXTURE_B_FRAMES,
        "⭐ PCM 恰好超出 `b.wav` 那一份（1920 帧 × 4 = 7680 字节）"
    );
    assert_eq!(over.path_bytes(), 0, "路径文本那一维仍在预算内");
    assert_eq!(over.total_bytes(), 7_680, "合计超出 = 7680 字节");

    // ② ⭐ 零预算：三项全报超 ⇒ 仍然**只报数**。
    let zero = plan.budget_overrun(&SfzByteBudget::new(0, 0, 0));
    println!(
        "[engine-sfz/S26] 零预算读数：超出 = {{区域表 {}，PCM {}，路径 {}}}、合计 {} 字节（⭐ 仍不改表）",
        zero.region_bytes(),
        zero.pcm_bytes(),
        zero.path_bytes(),
        zero.total_bytes(),
    );
    assert!(!zero.is_within(), "零预算 ⇒ 必然超");
    assert_eq!(
        zero.region_bytes(),
        before.region_bytes(),
        "零预算 ⇒ 区域表超出量 ＝ 它的全部占用（⛔ 不是被截断后的值）"
    );

    // ③ ⛔ 摊平结果**逐位不变**（"只报数"的机械证据）。
    let after = plan.byte_usage();
    let rendered = render(&plan);
    println!(
        "[engine-sfz/S26] 摊平结果逐位不变：byte_usage {before:?} → {after:?}；\
         sample_count {before_samples} → {}；pcm_bytes {before_pcm} → {}；region_count {before_regions} → {}；\
         渲染 (写入 {}, 非零 {}) → (写入 {}, 非零 {})",
        plan.sample_count(),
        plan.pcm_bytes(),
        plan.region_count(),
        reference.0,
        reference.1.iter().filter(|s| **s != 0.0).count(),
        rendered.0,
        rendered.1.iter().filter(|s| **s != 0.0).count(),
    );
    assert_eq!(after, before, "⭐ 预算判定不得改变字节读数");
    assert_eq!(plan.sample_count(), before_samples, "⛔ 不丢样本");
    assert_eq!(plan.pcm_bytes(), before_pcm, "⛔ 不裁 PCM");
    assert_eq!(plan.region_count(), before_regions, "⛔ 不丢区域");
    assert_eq!(rendered.0, reference.0, "⛔ 不改变写入帧数");
    assert_eq!(
        rendered.1, reference.1,
        "⭐ 超预算判定之后，渲染输出必须**逐位相同**（只报数 ⇒ 表没被改）"
    );
    assert!(
        rendered.1.iter().any(|sample| *sample != 0.0),
        "⭐ 见证：这条渲染路径**真的出声**（⛔ 挡住「什么都没渲染」的假绿）"
    );
}

/// ⭐ S27（本单新增；`sfz-needs-8` 的**动机**）："巨大音色库"这件事今天**可判**。
///
/// # 为什么要这条（逐字背景）
///
/// `sfz-needs-8` ＝ "摊平表无字节预算／无流式（样本 PCM 整段 clone）" ⇒ 1 GB PCM 的
/// 音色库会被**整段读进内存并 clone 进摊平表**。本判据给出的是**那个库会不会被看出来**。
///
/// # 量什么（对象 ＋ 单位）
///
/// 对象 ＝ 一份**参数化**的 [`SfzByteUsage`]（128 条区域 ＋ **1 GiB** PCM ＋ 128 × 12 字节
/// 路径文本）；单位 ＝ **字节**。⭐ 判定是**纯算术**（[`SfzByteUsage::overrun`]）⇒
/// **⛔ 不需要真的分配 1 GiB**（那正是"可判"与"实现流式"的区别）。
///
/// # 判定量
///
/// * 对 **512 MiB** 的 PCM 上界：`pcm` 超出 **恰好** `1 GiB − 512 MiB` ＝ **536 870 912** 字节、
///   `is_within() == false`、区域表与路径两维**为 0**；
/// * ⭐ **反向腿**（闭区间）：上界给**恰好 1 GiB** ⇒ **通过**（⛔ 判据不是"永远红"）。
#[test]
fn s27_a_gib_scale_library_is_visible_to_the_budget() {
    const GIB: usize = 1 << 30;
    const HALF_GIB: usize = 1 << 29;
    let region_bytes = core::mem::size_of::<PreparedRegion>() * 128;
    let path_bytes = 128 * 12;
    let usage = SfzByteUsage::new(region_bytes, GIB, path_bytes);
    let budget = SfzByteBudget::new(region_bytes, HALF_GIB, path_bytes);
    let over = usage.overrun(&budget);
    println!(
        "[engine-sfz/S27] 1 GiB 级 PCM（128 条区域）：读数 = {{区域表 {region_bytes}，PCM {GIB}，路径 {path_bytes}}}、\
         预算 = {{区域表 {region_bytes}，PCM {HALF_GIB}，路径 {path_bytes}}} ⇒ \
         超出 = {{区域表 {}，PCM {}，路径 {}}}、合计 {} 字节、is_within = {}",
        over.region_bytes(),
        over.pcm_bytes(),
        over.path_bytes(),
        over.total_bytes(),
        over.is_within(),
    );
    assert_eq!(
        usage.total_bytes(),
        region_bytes + GIB + path_bytes,
        "读数合计"
    );
    assert!(
        !over.is_within(),
        "⭐ 1 GiB PCM 对 512 MiB 的预算必须超（否则「巨大音色库不可判」）"
    );
    assert_eq!(
        over.pcm_bytes(),
        GIB - HALF_GIB,
        "⭐ 恰好超出 512 MiB ＝ 536 870 912 字节"
    );
    assert_eq!(
        over.total_bytes(),
        536_870_912,
        "合计超出 ＝ 536 870 912 字节"
    );
    assert_eq!(
        (over.region_bytes(), over.path_bytes()),
        (0, 0),
        "区域表与路径两维在预算内（⇒ 超预算**指向** PCM 这一项）"
    );
    // 反向腿：闭区间 ⇒ 上界恰好等于占用时**通过**（⛔ 判据不是"永远红"）。
    let exact = SfzByteBudget::new(region_bytes, GIB, path_bytes);
    assert!(
        usage.overrun(&exact).is_within(),
        "⭐ 上界恰好等于占用时必须**通过**（闭区间，与 `PcmBudget` 同口径）"
    );
}

// ---------------------------------------------------------------------------
// ⭐ S28～S33：**SFZ 循环点接进实时播放**（任务单 `#91` 的"循环那一半"）
//
// ⚠ **本单是接线，⛔ 不是新实现**：`loop_start` / `loop_end` / `loop_mode` 的解析
// 与"哪些模式才循环 / 退化窗口不循环"的裁决**早就存在**于 `yeban-sfz`
// （`instrument.rs` 的 `Region` 三字段与 `LoopMode` 白名单、`playback.rs` 的
// `Region::loop_window`）。缺的只是**播放层装不下** —— 摊平表此前不携带这三个字段。
// 本片把它们搬进 `PreparedRegion`、在唯一的逐样本循环 `mix_span_into` 里回绕，
// 并按 `region_index` 从逐块路径读回。
//
// 判据的分工：
//   * S28 —— 周期**逐样本精确**（下标序列，⛔ 不是"波形看起来在重复"）；
//   * S29 —— `no_loop` / `one_shot` **不循环** ＋ `one_shot` 忽略 note-off；
//   * S30 —— 字段**真的**被播放层读到（两份只差一个数的夹具）；
//   * S31 —— 输入政策不变（非法取值仍响亮 `Err`）；
//   * S32 —— `loop_sustain ≡ loop_continuous` 的登记**可判**；
//   * S33 —— 不带 `loop_*` 的既有夹具**逐位不变**（改动前实测指纹）。
// ---------------------------------------------------------------------------

/// ⭐ 循环夹具的 PCM 长度（帧；`rate` 恒 1.0 ⇒ 源下标与输出帧一一对应）。
const LOOP_PCM_FRAMES: usize = 400;

/// ⭐ 循环窗口（源采样点；半开 `[LOOP_START, LOOP_END)`）。
const LOOP_START: usize = 100;
const LOOP_END: usize = 164;

/// ⭐ 循环周期 ＝ `loop_end − loop_start`（帧）。
const LOOP_PERIOD: usize = LOOP_END - LOOP_START;

/// ⭐ 渲染帧数：跨过 **3 整圈**（⇒ "周期"不是从一段里猜的）。
const LOOP_RENDER_FRAMES: usize = LOOP_END + 3 * LOOP_PERIOD;

/// ⭐ **可解码的斜坡样本**：`pcm[i] = (i + 1) / 1024`。
///
/// 分母是 2 的幂 ⇒ 在 binary32 下**逐位精确**（`i < 2²⁴`）。
/// ⚠ 从 `i + 1` 起（⛔ 不是 `i`）：`pcm[0]` **必须非零**，否则"非零帧数"会把第 0 帧
/// 误记成静音（本单实测过一次 off-by-one）。
/// `gain == 1.0` 时输出**恰好**等于源采样值 ⇒ 判据可以把每个输出帧**反解成源采样
/// 下标**，从而把"循环周期"量成**下标的精确序列**，⛔ 不是"波形看起来像在重复"。
fn ramp_sample(frames: usize) -> InMemorySample {
    InMemorySample {
        pcm: (0..frames)
            .map(|frame| (frame + 1) as f32 / 1024.0)
            .collect(),
        sample_hz: 48_000.0,
    }
}

/// ⭐ 摊平一份**自己给样本**的 SFZ（循环夹具用）。
fn prepare_with(text: &str, sample: &InMemorySample) -> PreparedSfz {
    let instrument = SfzInstrument::load(text.to_owned(), &Default::default())
        .expect("循环夹具的 SFZ 必须解析成功");
    PreparedSfz::prepare(&instrument, sample, 48_000.0).expect("摊平必须成功")
}

/// ⭐ 单区域循环夹具的 SFZ 文本（`key=60`、零移调 ⇒ `rate == 1.0`）。
fn loop_sfz(mode: &str, loop_start: usize, loop_end: usize) -> String {
    format!(
        "<region> sample=a.wav key=60 pitch_keycenter=60 loop_mode={mode} \
         loop_start={loop_start} loop_end={loop_end}\n"
    )
}

/// ⭐ 从输出帧**反解源采样下标**（斜坡夹具专用；见 [`ramp_sample`]）。
fn source_index_at(out: &[f32], frame: usize) -> usize {
    (out[frame] * 1024.0) as usize - 1
}

/// ⭐ 触发**一个**声部并渲染 `frames` 帧（实时池路径；⭐ 本单的目标路径）。
fn render_pool(plan: &PreparedSfz, end_sample: u64, frames: usize) -> Vec<f32> {
    let mut pool = SfzVoicePool::new(48_000.0);
    let created = pool.trigger_note(plan, 60, 127, end_sample, 1.0);
    assert_eq!(created, 1, "单区域夹具 ⇒ 恰好建 1 个声部");
    let mut out = vec![0.0f32; frames];
    pool.render_into(plan, 0, &mut out);
    out
}

/// ⭐ **实测最小周期**：窗口段 `out[start..start+8)` 第一次**逐位重复**的位置减去 `start`。
///
/// ⛔ 不是"数非零点"：斜坡夹具下任何小于真周期的平移都会**逐位不同** ⇒ 首个重复位置
/// 就是真周期。
fn detect_period(out: &[f32], start: usize, search_to: usize) -> usize {
    const WINDOW: usize = 8;
    let head = &out[start..start + WINDOW];
    for candidate in (start + 1)..=search_to {
        if out[candidate..candidate + WINDOW] == *head {
            return candidate - start;
        }
    }
    panic!(
        "⭐ 在 [{}..{}] 内找不到周期（说明逐样本路径根本没回绕）",
        start + 1,
        search_to
    );
}

/// ⭐ **循环段下标序列的逐帧对账**：`t < start` ⇒ `t`；否则 `start + (t−start) mod period`。
fn loop_index_mismatches(out: &[f32], start: usize, period: usize) -> usize {
    out.iter()
        .enumerate()
        .filter(|(frame, _)| {
            let expected = if *frame < start {
                *frame
            } else {
                start + (*frame - start) % period
            };
            source_index_at(out, *frame) != expected
        })
        .count()
}

/// ⭐ S28（任务单 `#91`；`sfz-needs-6` 的"循环"那一半）：**循环周期逐样本精确**。
///
/// # 量什么（对象 ＋ 单位，先说再跑）
///
/// 对象 ＝ 一个实时 SFZ 声部（[`SfzVoicePool`]）渲染出的一段输出；单位 ＝ **源采样下标**
/// （由斜坡夹具**逐帧反解**，见 [`ramp_sample`]）。
///
/// # 判定量
///
/// * 摊平表**真的**带出了三个循环字段（`loop_start` / `loop_end` / `loop_mode`）——
///   ⛔ 否则这条判据量到的只是 `PlaybackSpec` 里的读数；
/// * 下标序列**逐帧**等于 `t < 100 ? t : 100 + (t−100) mod 64`（**0 处不符**）；
/// * ⭐ **实测最小周期**（首个窗口段逐位重复的位置）**恰好** `64 ＝ loop_end − loop_start`；
/// * ⭐ 反向腿（⛔ 防空转）：窗口段 `[100,164)` 必须 ≥ 2 个**不同位模式**。
#[test]
fn s28_the_loop_cycle_is_exactly_loop_end_minus_loop_start() {
    let plan = prepare_with(
        &loop_sfz("loop_continuous", LOOP_START, LOOP_END),
        &ramp_sample(LOOP_PCM_FRAMES),
    );
    let region = &plan.regions()[0];
    assert_eq!(
        (region.loop_start(), region.loop_end()),
        (LOOP_START as u32, LOOP_END as u32),
        "⭐ 摊平表必须**真的**带出 `loop_start` / `loop_end`（⛔ 不是只落在 `PlaybackSpec` 里）"
    );
    assert_eq!(
        region.loop_mode(),
        LoopMode::LoopContinuous,
        "⭐ 生效循环模式必须进摊平表"
    );
    let window = region
        .loop_window(LOOP_PCM_FRAMES)
        .expect("loop_continuous ＋ 合法窗口 ⇒ 必须有窗口");
    assert_eq!(
        (window.start as usize, window.end as usize),
        (LOOP_START, LOOP_END),
        "归约后的窗口必须逐字等于文件里的两个数"
    );

    let out = render_pool(&plan, u64::MAX, LOOP_RENDER_FRAMES);
    let nonzero = out.iter().filter(|sample| **sample != 0.0).count();
    let mismatches = loop_index_mismatches(&out, LOOP_START, LOOP_PERIOD);
    let measured = detect_period(&out, LOOP_START, LOOP_END);
    let distinct = out[LOOP_START..LOOP_END]
        .iter()
        .map(|sample| sample.to_bits())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    println!(
        "[engine-sfz/S28] 窗口 = [{LOOP_START},{LOOP_END}) ⇒ 声明周期 = {LOOP_PERIOD}；\
         {LOOP_RENDER_FRAMES} 帧里非零 = {nonzero}；⭐ **实测最小周期** = {measured}；\
         下标序列不符 = {mismatches}；窗口段不同位模式 = {distinct}；\
         前 8 帧下标 = {:?}、回绕点前后（163,164,165）= {:?}",
        (0..8)
            .map(|frame| source_index_at(&out, frame))
            .collect::<Vec<_>>(),
        (163..166)
            .map(|frame| source_index_at(&out, frame))
            .collect::<Vec<_>>(),
    );
    assert_eq!(
        nonzero, LOOP_RENDER_FRAMES,
        "循环声部必须写满整段（⛔ 不因'样本播完'退休）"
    );
    assert_eq!(
        mismatches, 0,
        "⭐ 下标序列必须**逐帧精确**（⛔ 这不是'波形看起来在重复'）"
    );
    assert_eq!(
        measured, LOOP_PERIOD,
        "⭐ **实测周期必须恰好等于 `loop_end − loop_start`**（实得 {measured}）"
    );
    assert!(
        distinct >= 2,
        "⭐ 反向腿：窗口段必须 ≥ 2 个不同位模式（否则'周期'可能是常数段的假象）"
    );
}

/// ⭐ S29（任务单 `#91`）：**`no_loop` / `one_shot` 都不循环**，且 **`one_shot` 忽略 note-off**。
///
/// # 量什么（对象 ＋ 单位，先说再跑）
///
/// 对象 ＝ 实时池的输出（斜坡夹具 400 帧 ⇒ 源下标可反解）；单位 ＝ **非零帧数**
/// （⛔ 不是"有没有声音"：`no_loop` 与 `one_shot` 都**必须**出声，只是**不重复**）。
///
/// # 为什么四条腿都要
///
/// * 两份夹具都**写了** `loop_start=100 loop_end=164` ⇒ 若播放层不看 `loop_mode`
///   就会循环（非零会远超 399）⇒ 这两条腿把"模式被读到"钉住；
/// * `one_shot` ＋ `end_sample=100` ⇒ 非零仍 **399** ＝ **忽略 note-off**；
/// * ⭐ **反向腿**：`no_loop` ＋ `end_sample=100` ⇒ 恰好 **100**
///   （证明 `end_sample` **真的在切**，否则"忽略 note-off"那一条是空绿）。
#[test]
fn s29_no_loop_and_one_shot_do_not_loop_and_one_shot_ignores_note_off() {
    // 样本 400 帧、从源 0 起播：会写下的**最大**帧数（线性插值要读 `index+1`）。
    const MAX_WRITTEN: usize = LOOP_PCM_FRAMES - 1;
    let frames = 512usize;

    let no_loop = prepare_with(
        &loop_sfz("no_loop", LOOP_START, LOOP_END),
        &ramp_sample(LOOP_PCM_FRAMES),
    );
    let one_shot = prepare_with(
        &loop_sfz("one_shot", LOOP_START, LOOP_END),
        &ramp_sample(LOOP_PCM_FRAMES),
    );
    assert!(
        no_loop.regions()[0].loop_window(LOOP_PCM_FRAMES).is_none(),
        "`no_loop` 即使写了 `loop_start`/`loop_end` 也⛔ 不产生窗口"
    );
    assert!(
        one_shot.regions()[0].loop_window(LOOP_PCM_FRAMES).is_none(),
        "`one_shot` 同样⛔ 不产生窗口"
    );

    let no_loop_out = render_pool(&no_loop, u64::MAX, frames);
    let one_shot_out = render_pool(&one_shot, u64::MAX, frames);
    let one_shot_held = render_pool(&one_shot, 100, frames);
    let no_loop_cut = render_pool(&no_loop, 100, frames);

    let count = |out: &[f32]| out.iter().filter(|sample| **sample != 0.0).count();
    println!(
        "[engine-sfz/S29] no_loop ⇒ 非零 = {}；one_shot ⇒ 非零 = {}；\
         one_shot ＋ end_sample=100 ⇒ 非零 = {}；⭐ no_loop ＋ end_sample=100（反向腿）⇒ 非零 = {}；\
         （样本 {LOOP_PCM_FRAMES} 帧 ⇒ 不循环时上限 {MAX_WRITTEN}）",
        count(&no_loop_out),
        count(&one_shot_out),
        count(&one_shot_held),
        count(&no_loop_cut),
    );
    for (label, out) in [("no_loop", &no_loop_out), ("one_shot", &one_shot_out)] {
        assert_eq!(
            count(out),
            MAX_WRITTEN,
            "⭐ `{label}` **不循环**：播到样本尾就停（⛔ 不是从 `loop_start` 重播）"
        );
        assert!(
            out[MAX_WRITTEN..].iter().all(|sample| *sample == 0.0),
            "⭐ `{label}`：样本尾之后必须是**逐位静音**（⛔ 不是重复）"
        );
    }
    assert_eq!(
        count(&one_shot_held),
        MAX_WRITTEN,
        "⭐ `one_shot` **忽略 note-off**：`end_sample=100` 到点⛔ 不切它"
    );
    assert_eq!(
        count(&no_loop_cut),
        100,
        "⭐ 反向腿：`no_loop` 在 `end_sample=100` 处**恰好**停 100 帧\
         （证明 `end_sample` 真的在切，⛔ 否则上一条是空绿）"
    );
}

/// ⭐ S30（任务单 `#91`；§4.3 的**常驻腿**）：字段**真的**被逐样本路径读到。
///
/// # 为什么需要它（把"接线"与"只是搬了个字段"分开）
///
/// 一份"搬了字段但没接线"的实现，让 `PreparedRegion` 带上 `loop_start` / `loop_end`
/// 也能通过类型检查、也能让 S9 的尺寸读数变化 —— 但⛔ **不会**让周期跟着字段走。
/// 本判据用两份**只差 `loop_end` 一个数**的夹具（`164` / `200`），要求：
///
/// * 回读的字段与归约窗口**逐字**等于文件里的数；
/// * **实测最小周期**分别为 `64` / `100`，且下标序列各自**逐帧精确**。
///
/// ⇒ 字段错一位（或没接线）**必红**。
#[test]
fn s30_the_loop_window_really_feeds_the_per_sample_path() {
    for (loop_end, period) in [(164usize, 64usize), (200usize, 100usize)] {
        let plan = prepare_with(
            &loop_sfz("loop_continuous", LOOP_START, loop_end),
            &ramp_sample(LOOP_PCM_FRAMES),
        );
        let region = &plan.regions()[0];
        assert_eq!(
            (region.loop_start(), region.loop_end()),
            (LOOP_START as u32, loop_end as u32),
            "回读的循环字段必须逐字等于文件"
        );
        let window = region.loop_window(LOOP_PCM_FRAMES).expect("有效窗口");
        assert_eq!(
            (
                window.start as usize,
                window.end as usize,
                window.len() as usize
            ),
            (LOOP_START, loop_end, period),
            "归约窗口与长度"
        );

        let frames = loop_end + 3 * period;
        let out = render_pool(&plan, u64::MAX, frames);
        let measured = detect_period(&out, LOOP_START, loop_end);
        let mismatches = loop_index_mismatches(&out, LOOP_START, period);
        println!(
            "[engine-sfz/S30] loop_end = {loop_end} ⇒ 声明周期 = {period}；\
             实测周期 = {measured}；下标序列不符 = {mismatches}（帧数 {frames}）"
        );
        assert_eq!(
            measured, period,
            "⭐ 实测周期必须跟着 `loop_end` 走（⛔ 只搬字段不接线会在这里红）"
        );
        assert_eq!(mismatches, 0, "⭐ 下标序列逐帧精确");
    }
}

/// ⭐ S31（任务单 `#91`；第 259 条）：**非法循环输入仍响亮 `Err`**，⛔ 不静默降级。
///
/// # 判定量（逐条）
///
/// * 集合外 `loop_mode` ⇒ `SfzError::InvalidOption`（`yeban-sfz` 的 `as_option` 白名单）；
/// * 越界 `loop_start` ⇒ `SfzError::IntegerOutOfRange`（`read_u32`）；
/// * ⭐ **唯一保留的一档**：`loop_end <= loop_start`（两个值**各自合法**、只有**组合**
///   退化）⇒ `load` **`Ok`**、`loop_window() == None` ＝ **按不循环渲染**
///   ——既有语义，本片⛔ 不加新机制、⛔ 不报错。
#[test]
fn s31_illegal_loop_input_still_fails_loudly_on_the_engine_path() {
    let load = |text: &str| SfzInstrument::load(text.to_owned(), &Default::default());

    let bad_mode = load("<region> sample=a.wav loop_mode=bogus\n")
        .expect_err("⭐ 集合外的 `loop_mode` 必须**响亮报错**（⛔ 不许降级 ＋ 记录）");
    assert!(
        matches!(bad_mode, SfzError::InvalidOption { .. }),
        "⭐ 必须是 `InvalidOption`（白名单拒绝），实得 {bad_mode:?}"
    );

    let bad_start = load(
        "<region> sample=a.wav loop_mode=loop_continuous loop_start=99999999999999 loop_end=20\n",
    )
    .expect_err("⭐ 越界的 `loop_start` 必须**响亮报错**（⛔ 不许降级）");
    assert!(
        matches!(bad_start, SfzError::IntegerOutOfRange { .. }),
        "⭐ 必须是 `IntegerOutOfRange`，实得 {bad_start:?}"
    );

    // ⭐ 唯一保留的一档：组合退化 ⇒ `Ok` ＋ 不循环（既有语义）。
    let degenerate = load(
        "<region> sample=a.wav key=60 pitch_keycenter=60 loop_mode=loop_continuous loop_start=100 loop_end=100\n",
    )
    .expect("⭐ 两个值各自合法 ⇒ `load` 必须 `Ok`（⛔ 不新增降级机制）");
    let plan = PreparedSfz::prepare(&degenerate, &ramp_sample(LOOP_PCM_FRAMES), 48_000.0)
        .expect("摊平必须成功");
    let window = plan.regions()[0].loop_window(LOOP_PCM_FRAMES);
    println!(
        "[engine-sfz/S31] `loop_mode=bogus` ⇒ {bad_mode:?}；越界 `loop_start` ⇒ {bad_start:?}；\
         退化窗口（`loop_end == loop_start`）⇒ `load` Ok、`loop_window` = {window:?}"
    );
    assert!(
        window.is_none(),
        "⭐ `loop_end <= loop_start` ⇒ 既有语义是**按不循环渲染**（⛔ 不是死循环窗口）"
    );
}

/// ⭐ S32（任务单 `#91`）：**`loop_sustain ≡ loop_continuous` 的登记可判**。
///
/// # 判定量
///
/// `include_str!("../src/sfz_instrument.rs")`（模块头**源码**）必须含那条**逐字**登记；
/// ⭐ 反向腿：文档里**没有**的那一句 ⇒ `contains` 必须 `false`（⛔ 判据不是恒真）。
///
/// ⚠ 形态照本仓"文本即判据"的既有做法（如 `yeban-engine/src/level.rs` /
/// `yeban-app/tests/engine_device_leg.rs` 的 `include_str!` 断言）。
#[test]
fn s32_the_loop_sustain_equivalence_is_registered_in_the_module_doc() {
    const MODULE_SOURCE: &str = include_str!("../src/sfz_instrument.rs");
    const NEEDLE: &str =
        "note-off 后两者的差别（走完当前循环再进释放）属 ADSR 片、本片不声称已实现";
    println!(
        "[engine-sfz/S32] 模块头含登记 = {}；反向腿（文档里没有的句子）= {}",
        MODULE_SOURCE.contains(NEEDLE),
        MODULE_SOURCE.contains("note-off 后两者的差别属本片已实现"),
    );
    assert!(
        MODULE_SOURCE.contains(NEEDLE),
        "⭐ 模块头必须逐字登记：{NEEDLE}"
    );
    assert!(
        !MODULE_SOURCE.contains("note-off 后两者的差别属本片已实现"),
        "⭐ 反向腿：文档里**没有**的句子必须 `contains == false`（否则判据恒真）"
    );
}

/// ⭐ S33（任务单 `#91`；§4.4）：**不带任何 `loop_*` 的既有夹具逐位不变**。
///
/// # 量什么（对象 ＋ 单位，先说再跑）
///
/// 对象 ＝ 既有夹具 [`TWO_MATCH_ONE_MISS`]（**一个 `loop_*` 都没有**）经
/// `SynthEngine::render_track` 实时路径渲染的 512 帧；单位 ＝ **FNV-1a 64 指纹**（逐位）。
///
/// # 字面值的来源（⭐ 改动前实测）
///
/// `0xa933ddab286bdf5e` 是**基线探针**（`git checkout` 回 HEAD `5ac570eb`、工作树干净时
/// 运行、测后删除）在本机 `aarch64/macos` 上量到的**同一个夹具、同一段 512 帧**的读数：
/// 摊平路径与**引擎实时路径**两条路**同值**、`max|engine − flat| = 0`。
/// ⭐ 它同时等于既有 [`BROADCAST_STEP2_FINGERPRINT`]（步 2 实测）——两条独立来路的
/// 交叉印证。
///
/// # 反向腿（⛔ 让"绿"不能是空的）
///
/// 把同一段输出**乘 `1.0000001`** ⇒ 指纹**必须变**（证明它是**逐位**判据，
/// ⛔ 不是"有输出就算过"）。
#[test]
fn s33_a_fixture_without_any_loop_fields_renders_bit_identically() {
    let prepared = prepare(TWO_MATCH_ONE_MISS);
    let track = EntityId::new();
    let mut engine = engine_with_track(track);
    let schedule =
        NoteSchedule::from_sorted(vec![support::scheduled(0, 12_000, 60, 1.0, 48_000.0)]);
    let mut out = vec![0.0f32; BLOCK];
    engine.render_track(track, Some(&schedule), Some(&prepared), &mut out);
    let actual = fnv1a_64(&out);
    // ⭐ 反向腿：逐位判据的敏感性见证。
    let scaled: Vec<f32> = out.iter().map(|sample| sample * 1.000_000_1).collect();
    let scaled_hash = fnv1a_64(&scaled);
    println!(
        "[engine-sfz/S33] 无 `loop_*` 夹具（512 帧 @60,127）：非零 = {}、指纹 = 0x{actual:016x}\
         （改动前实测 = 0x{NO_LOOP_FIXTURE_FINGERPRINT:016x}）；\
         反向腿（× 1.0000001）指纹 = 0x{scaled_hash:016x}（必须不同）",
        out.iter().filter(|sample| **sample != 0.0).count(),
    );
    assert!(
        out.iter().any(|sample| *sample != 0.0),
        "夹具必须真的出声（否则指纹比对是空转）"
    );
    let _checked = check_fingerprint(
        "S33 无 loop 夹具逐位不变",
        actual,
        NO_LOOP_FIXTURE_FINGERPRINT,
    );
    assert_ne!(
        scaled_hash, actual,
        "⭐ 反向腿：输出乘 1.0000001 ⇒ 指纹**必须**不同（否则判据不是逐位的）"
    );
}

// ---------------------------------------------------------------------------
// ⭐ S34～S38：**SFZ `ampeg` ADSR 接进实时播放**（任务单 `#92` 的 ADSR 那一半）
//
// 判据的分工：
//   * S34 —— 解析层五条腿（含单位后缀）＋ `Err` 政策 ＋ 缺失 ⇒ 缺省 ＋ 帧换算；
//   * S35 —— ⭐ **包络形状逐样本可判**（到峰帧数 / sustain 电平 / 释放归零 ＋ 段位真的推进）；
//   * S36 —— ⭐ **`loop_sustain` 与 `loop_continuous` 在 note-off 之后输出不同**
//             （各配一条"它自己该怎样"的断言）；
//   * S37 —— ⭐ **无 `ampeg_*` 的既有夹具逐位不变**（改动前实测指纹 ＋ 乘 `1.0000001` 的反向腿）；
//   * S38 —— ⭐ 模块文档里"取线性段 / 指数对数不在本片范围"那条登记**可判**（＋ 反向腿）。
// ---------------------------------------------------------------------------

/// ⭐ S35 夹具的形状（**秒**；48 kHz ⇒ 帧数都是整数，四舍五入与截断**不可区分**的
/// 边界之外还刻意留了一条 `.01` 的腿）。
const AMPEG_ATTACK_SECONDS: f32 = 0.010; // 480 帧
const AMPEG_HOLD_SECONDS: f32 = 0.005; // 240 帧
const AMPEG_DECAY_SECONDS: f32 = 0.005; // 240 帧
const AMPEG_SUSTAIN_PERCENT: f32 = 50.0;
const AMPEG_RELEASE_SECONDS: f32 = 0.020; // 960 帧

/// 与上面四段对应的**引擎帧**（48 kHz；判据里⛔ 不手抄进断言，全部由这两条常量推）。
const AMPEG_ATTACK_FRAMES: usize = 480;
const AMPEG_HOLD_FRAMES: usize = 240;
const AMPEG_DECAY_FRAMES: usize = 240;
const AMPEG_RELEASE_FRAMES: usize = 960;
/// 进入 sustain 的帧（＝ 前 3 段之和）。
const AMPEG_SUSTAIN_START: usize = AMPEG_ATTACK_FRAMES + AMPEG_HOLD_FRAMES + AMPEG_DECAY_FRAMES;
/// note-off 落在 sustain 里、且**离段边界有余量**（这样"释放段"]的起点电平是 `sustain`）。
const AMPEG_NOTE_OFF: usize = AMPEG_SUSTAIN_START + 240;

/// ⭐ **包络判据的夹具样本**：恒定 `1.0`。
///
/// 为什么必须是常量：`gain == 1.0`（`volume=0`、`velocity=127` ⇒ 两条恒等）时
/// 输出**恰好**等于包络电平 ⇒ 判据可以把每个输出帧**直接当成包络读数**对账
/// （⛔ 不是"波形看起来像包络"）。
/// ⚠ 速率必须是 `1.0`（`pitch_keycenter == note`、`sample_hz == engine_hz`）：
/// 否则 `frac ≠ 0` 会让 `1.0 * (1 − frac) + 1.0 * frac` 出现 1 ulp 的偏差
/// （本片实测：`frac = 0.1` 时该式**不等于** 1.0）。
fn flat_sample(frames: usize) -> InMemorySample {
    InMemorySample {
        pcm: vec![1.0f32; frames],
        sample_hz: 48_000.0,
    }
}

/// ⭐ S35/S36 的 SFZ 文本（单区域、`key=60`、零移调 ⇒ `rate == 1.0`）。
fn ampeg_sfz(mode: &str) -> String {
    format!(
        "<region> sample=a.wav key=60 pitch_keycenter=60 loop_mode={mode} \
         ampeg_attack={AMPEG_ATTACK_SECONDS} ampeg_hold={AMPEG_HOLD_SECONDS} \
         ampeg_decay={AMPEG_DECAY_SECONDS} ampeg_sustain={AMPEG_SUSTAIN_PERCENT} \
         ampeg_release={AMPEG_RELEASE_SECONDS}\n"
    )
}

/// ⭐ S34（任务单 `#92`）：**解析层五条腿 ＋ `Err` 政策 ＋ 缺失 ⇒ 缺省 ＋ 帧换算**。
///
/// # 量什么（对象 ＋ 单位，先说再跑）
///
/// 对象 ＝ 摊平表 [`PreparedRegion`] 的五个只读读数（**秒 / 百分比**）与
/// [`AmpegRun`] 的**帧数**；单位 ＝ 秒、百分比、引擎帧（48 kHz）。
///
/// # 判定量（逐条）
///
/// * 五个 opcode **各自**一条腿，且**混用单位后缀**（`1.5s` / `250ms` / 裸值 / `50%` / `2s`）
///   ⇒ 回读值**逐字**等于文件里的数（⛔ 不是"能解析就算过"）；
/// * `PreparedSfz::engine_hz()` ＝ 摊平期的 48 kHz（帧换算的唯一事实源）；
/// * 帧换算：`0.010 s × 48 kHz` ⇒ **480 帧**（⭐ 四舍五入：截断在 `.01` 上会得到 479）；
/// * ⭐ **非法 ⇒ 响亮 `Err`**（三类各一条：认不出 / 负 / 越界）＋ ⭐
///   **同一批数据里的合法样例必须被接受**（⛔ 防"输入永远被拒"的假绿）；
/// * ⭐ **缺失 ⇒ 缺省**（与"非法"**分开**断言）：五个读数全在透明缺省上、
///   `ampeg_is_transparent()` 为真、`ampeg_run()` 为 `None`（＝ ⛔ 不进包络路径）。
#[test]
fn s34_the_five_ampeg_opcodes_parse_and_the_illegal_ones_still_fail_loudly() {
    let load = |text: &str| SfzInstrument::load(text.to_owned(), &Default::default());

    // ---- ① 五个 opcode 各自一条腿（混用单位后缀写法）----
    const FIVE: &str = "<region> sample=a.wav key=60 pitch_keycenter=60 ampeg_attack=1.5s \
                        ampeg_hold=250ms ampeg_decay=0.25 ampeg_sustain=50% ampeg_release=2s\n";
    let instrument = load(FIVE).expect("⭐ 合法样例必须被接受");
    let plan = PreparedSfz::prepare(&instrument, &flat_sample(64), 48_000.0).expect("摊平");
    let region = &plan.regions()[0];
    println!(
        "[engine-sfz/S34] 五个 opcode（`1.5s` / `250ms` / `0.25` / `50%` / `2s`）⇒ \
         attack={} hold={} decay={} sustain={} release={}；engine_hz={}",
        region.ampeg_attack_seconds(),
        region.ampeg_hold_seconds(),
        region.ampeg_decay_seconds(),
        region.ampeg_sustain_percent(),
        region.ampeg_release_seconds(),
        plan.engine_hz(),
    );
    assert_eq!(region.ampeg_attack_seconds(), 1.5, "`1.5s` ⇒ 1.5 秒");
    assert_eq!(region.ampeg_hold_seconds(), 0.25, "`250ms` ⇒ 0.25 秒");
    assert_eq!(region.ampeg_decay_seconds(), 0.25, "裸值按秒读");
    assert_eq!(region.ampeg_sustain_percent(), 50.0, "`50%` ⇒ 50 个百分点");
    assert_eq!(region.ampeg_release_seconds(), 2.0, "`2s` ⇒ 2 秒");
    assert_eq!(plan.engine_hz(), 48_000.0, "帧换算的事实源＝摊平期采样率");

    // ---- ② 帧换算：四舍五入（⛔ 不是截断）----
    let rounded = PreparedSfz::prepare(
        &load("<region> sample=a.wav key=60 pitch_keycenter=60 ampeg_attack=0.01\n").expect("load"),
        &flat_sample(64),
        48_000.0,
    )
    .expect("摊平");
    let run = rounded.regions()[0]
        .ampeg_run(rounded.engine_hz())
        .expect("0.01 s ≠ 缺省 ⇒ 必须进包络路径");
    println!(
        "[engine-sfz/S34] `0.01 s @ 48 kHz` ⇒ attack_frames = {}（四舍五入；截断会给 479）",
        run.attack_frames()
    );
    assert_eq!(
        run.attack_frames(),
        480,
        "⭐ 0.01 s × 48 kHz 必须**四舍五入**到 480 帧（截断 479 ⇒ 到峰帧数差一）"
    );
    assert_eq!(run.hold_frames(), 0, "未给出的段 ⇒ 0 帧（零长段）");
    assert_eq!(run.decay_frames(), 0);
    assert_eq!(run.release_frames(), 0);
    assert_eq!(run.sustain_level(), 1.0, "未给出的 sustain ⇒ 100 % ⇒ 1.0");

    // ---- ③ 非法 ⇒ 响亮 `Err`（三类各一条）----
    let bad_float = load("<region> sample=a.wav ampeg_attack=abc\n")
        .expect_err("⭐ 认不出的数值必须**响亮报错**");
    let bad_duration =
        load("<region> sample=a.wav ampeg_release=-1s\n").expect_err("⭐ 负时长必须**响亮报错**");
    let bad_range = load("<region> sample=a.wav ampeg_sustain=150\n")
        .expect_err("⭐ 越界的 sustain 必须**响亮报错**");
    println!("[engine-sfz/S34] 非法输入 ⇒ {bad_float:?} / {bad_duration:?} / {bad_range:?}");
    assert!(
        matches!(bad_float, SfzError::InvalidFloat { .. }),
        "⛔ `ampeg_attack=abc` 不许静默降级，实得 {bad_float:?}"
    );
    assert!(
        matches!(bad_duration, SfzError::InvalidDuration { .. }),
        "⛔ `ampeg_release=-1s` 不许静默钳位，实得 {bad_duration:?}"
    );
    assert!(
        matches!(bad_range, SfzError::FloatOutOfRange { .. }),
        "⛔ `ampeg_sustain=150` 不许静默钳位，实得 {bad_range:?}"
    );
    // ⭐ 同一批数据里必须有一条**合法**样例被正确接受（⛔ 否则上面三条可能是"永远拒绝"的假绿）。
    let mixed = "<region> sample=a.wav ampeg_attack=abc\n\
                 <region> sample=b.wav ampeg_attack=0.5s\n";
    assert!(load(mixed).is_err(), "同一批里有一条非法 ⇒ 整份 `Err`");
    let legal = load("<region> sample=b.wav ampeg_attack=0.5s\n").expect("⭐ 合法样例必须被接受");
    let legal_value = legal
        .with_instrument(|parsed| parsed.regions()[0].ampeg_attack)
        .expect("查询");
    assert_eq!(legal_value, 0.5, "同批数据里的合法值照读");

    // ---- ④ 缺失 ⇒ 缺省（**与非法分开**断言）----
    let absent = PreparedSfz::prepare(
        &load("<region> sample=a.wav key=60 pitch_keycenter=60\n").expect("load"),
        &flat_sample(64),
        48_000.0,
    )
    .expect("摊平");
    let absent_region = &absent.regions()[0];
    println!(
        "[engine-sfz/S34] 缺失读数：attack={} hold={} decay={} sustain={} release={}、\
         transparent={}、ampeg_run={:?}",
        absent_region.ampeg_attack_seconds(),
        absent_region.ampeg_hold_seconds(),
        absent_region.ampeg_decay_seconds(),
        absent_region.ampeg_sustain_percent(),
        absent_region.ampeg_release_seconds(),
        absent_region.ampeg_is_transparent(),
        absent_region.ampeg_run(absent.engine_hz()).is_some(),
    );
    assert_eq!(absent_region.ampeg_attack_seconds(), 0.0);
    assert_eq!(absent_region.ampeg_hold_seconds(), 0.0);
    assert_eq!(absent_region.ampeg_decay_seconds(), 0.0);
    assert_eq!(absent_region.ampeg_sustain_percent(), 100.0);
    assert_eq!(
        absent_region.ampeg_release_seconds(),
        0.0,
        "⭐ 本仓 `ampeg_release` 缺省是 0（⛔ 不是规范的 0.001）—— 见 yeban-sfz 的常量文档"
    );
    assert!(
        absent_region.ampeg_is_transparent(),
        "五个值都在透明缺省上 ⇒ 快速通道"
    );
    assert!(
        absent_region.ampeg_run(absent.engine_hz()).is_none(),
        "⭐ 透明 ⇒ `None` ⇒ 逐样本路径一个浮点运算都不加"
    );
}

/// ⭐ S35（任务单 `#92`）：**包络形状逐样本可判**。
///
/// # 量什么（对象 ＋ 单位，先说再跑）
///
/// 对象 ＝ 实时池（[`SfzVoicePool`]）在**恒定 1.0 样本**上的输出；单位 ＝ **包络电平**
/// （`gain == 1.0` ⇒ 输出**恰好**是包络值，见 [`flat_sample`]）。
///
/// # ⭐ 容差口径（写死，⛔ 不用"大约"）
///
/// **线性段上每一点都在 `|实测 − 理论| ≤ 1e-6` 内**。理论上界（逐段）：
/// `t/A`、`1`、`1 − (1−S)(t−A−H)/D`、`S`、`L_off·(R−k)/R`。
/// ⚠ 实现只做「一次整数→浮点转换 ＋ 一两次乘加」（倒数在逐块时算好）⇒ 实测偏差
/// 应在 `1e-7` 量级（本判据会把实测最大偏差**打印出来**）。
///
/// # 判定量（逐条）
///
/// * `|out[t] − t/A| ≤ 1e-6`（`t ∈ 0..A`）＋ `out[A] == 1.0`（⭐ **恰好第 A 帧到峰**）；
/// * `out[t] == 1.0`（保持段）＋ `out[t] == 0.5`（衰减后的 sustain 电平）；
/// * ⭐ note-off 后按 `ampeg_release` **恰好 960 帧**线性归零：
///   `|out[T_off + k] − 0.5·(960 − k)/960| ≤ 1e-6`（`k ∈ 0..960`）、
///   `out[T_off + 960] == 0.0` 且**之后逐位为 0**（声部已退休）；
/// * ⭐ **段位真的在推进**（`SfzVoice::envelope_stage` 逐块读数：`Attack → Hold → Decay
///   → Sustain → Release`）—— ⛔ 这条把"电平恰好对"与"状态真的动了"分开钉住
///   （一个永远返回 `Attack` 的假读数会在这里红）。
#[test]
fn s35_the_envelope_shape_is_judgeable_sample_by_sample() {
    // 释放段需要写满 960 帧 ⇒ 总帧数留到 note-off ＋ 释放 ＋ 余量。
    const FRAMES: usize = AMPEG_NOTE_OFF + AMPEG_RELEASE_FRAMES + 64;
    let plan = prepare_with(&ampeg_sfz("no_loop"), &flat_sample(4096));
    let mut pool = SfzVoicePool::new(48_000.0);
    assert_eq!(
        pool.trigger_note(&plan, 60, 127, AMPEG_NOTE_OFF as u64, 1.0),
        1,
        "单区域夹具 ⇒ 恰好建 1 个声部"
    );

    // ⭐ 逐块渲染：每块之后读段位（状态真的在推进的机械见证）。
    let mut out = vec![0.0f32; FRAMES];
    let mut stages: Vec<(usize, String)> = Vec::new();
    let mut cursor = 0usize;
    for chunk in [
        1usize,
        AMPEG_ATTACK_FRAMES - 1,
        AMPEG_HOLD_FRAMES,
        AMPEG_DECAY_FRAMES,
    ] {
        pool.render_into(&plan, cursor as u64, &mut out[cursor..cursor + chunk]);
        cursor += chunk;
        stages.push((cursor, format!("{:?}", pool.voices()[0].envelope_stage())));
    }
    // 保持到 note-off 前一刻，再读一次（此时仍在 sustain）。
    let to_note_off = AMPEG_NOTE_OFF - cursor;
    pool.render_into(&plan, cursor as u64, &mut out[cursor..cursor + to_note_off]);
    cursor += to_note_off;
    stages.push((cursor, format!("{:?}", pool.voices()[0].envelope_stage())));
    // 释放段（含 note-off 那一帧）。
    let rest = FRAMES - cursor;
    pool.render_into(&plan, cursor as u64, &mut out[cursor..]);
    stages.push((
        cursor + rest,
        format!("{:?}", pool.voices()[0].envelope_stage()),
    ));

    // ---- ① 逐样本对账（线性段，容差 1e-6）----
    let mut worst = 0.0f32;
    let mut worst_at = 0usize;
    let check = |frame: usize, expected: f32, worst: &mut f32, worst_at: &mut usize| {
        let delta = (out[frame] - expected).abs();
        if delta > *worst {
            *worst = delta;
            *worst_at = frame;
        }
    };
    let sustain = AMPEG_SUSTAIN_PERCENT / 100.0;
    for t in 0..AMPEG_ATTACK_FRAMES {
        let expected = t as f32 / AMPEG_ATTACK_FRAMES as f32;
        check(t, expected, &mut worst, &mut worst_at);
    }
    // 保持段（只有 `A..A+H`；衰减段在下一个循环里单独对账）。
    for t in AMPEG_ATTACK_FRAMES..(AMPEG_ATTACK_FRAMES + AMPEG_HOLD_FRAMES) {
        check(t, 1.0, &mut worst, &mut worst_at);
    }
    // 衰减段：从 1.0 线性到 sustain。
    for t in (AMPEG_ATTACK_FRAMES + AMPEG_HOLD_FRAMES)..AMPEG_SUSTAIN_START {
        let k = t - AMPEG_ATTACK_FRAMES - AMPEG_HOLD_FRAMES;
        let expected = 1.0 - (1.0 - sustain) * (k as f32 / AMPEG_DECAY_FRAMES as f32);
        check(t, expected, &mut worst, &mut worst_at);
    }
    // sustain 段。
    for t in AMPEG_SUSTAIN_START..=AMPEG_NOTE_OFF {
        check(t, sustain, &mut worst, &mut worst_at);
    }
    // 释放段。
    for k in 0..AMPEG_RELEASE_FRAMES {
        let expected = sustain * (AMPEG_RELEASE_FRAMES - k) as f32 / AMPEG_RELEASE_FRAMES as f32;
        check(AMPEG_NOTE_OFF + k, expected, &mut worst, &mut worst_at);
    }
    println!(
        "[engine-sfz/S35] 恒定 1.0 样本、{FRAMES} 帧：⭐ 线性段最大偏差 = {worst:e}（在第 \
         {worst_at} 帧）；到峰帧 = {}（out[{}] = {}）；sustain = {}；\
         释放起点（第 {} 帧）= {}、释放末帧（第 {} 帧）= {}；第 {} 帧起 = {}；\
         段位读数 = {stages:?}",
        AMPEG_ATTACK_FRAMES,
        AMPEG_ATTACK_FRAMES,
        out[AMPEG_ATTACK_FRAMES],
        out[AMPEG_SUSTAIN_START],
        AMPEG_NOTE_OFF,
        out[AMPEG_NOTE_OFF],
        AMPEG_NOTE_OFF + AMPEG_RELEASE_FRAMES - 1,
        out[AMPEG_NOTE_OFF + AMPEG_RELEASE_FRAMES - 1],
        AMPEG_NOTE_OFF + AMPEG_RELEASE_FRAMES,
        out[AMPEG_NOTE_OFF + AMPEG_RELEASE_FRAMES],
    );
    assert!(
        worst <= 1.0e-6,
        "⭐ 容差口径：线性段每一点 |实测 − 理论| ≤ 1e-6（实测最大 {worst:e}，第 {worst_at} 帧）"
    );

    // ---- ② 到峰 / sustain / 归零三条**精确**腿 ----
    assert_eq!(
        out[AMPEG_ATTACK_FRAMES], 1.0,
        "⭐ 起音**恰好**在 A ＝ {AMPEG_ATTACK_FRAMES} 帧到峰（⛔ 不是 A±1）"
    );
    assert_eq!(
        out[AMPEG_SUSTAIN_START], sustain,
        "衰减段终点恰好是 sustain"
    );
    assert_eq!(
        out[AMPEG_NOTE_OFF + AMPEG_RELEASE_FRAMES],
        0.0,
        "⭐ 释放段**恰好** R ＝ {AMPEG_RELEASE_FRAMES} 帧归零"
    );
    assert!(
        out[AMPEG_NOTE_OFF + AMPEG_RELEASE_FRAMES..]
            .iter()
            .all(|sample| *sample == 0.0),
        "⭐ 归零之后必须**逐位**静音（声部已退休，⛔ 不是「接近 0」）"
    );
    assert_eq!(
        pool.active_count(),
        0,
        "⭐ 释放走完 ⇒ 声部退休（池内状态翻转）"
    );

    // ---- ③ 段位读数（状态真的在推进）----
    let labels: Vec<&str> = stages.iter().map(|(_, label)| label.as_str()).collect();
    // ⚠ 读数的语义：`envelope_stage()` 是**最后一帧所落的段**（状态是"处理完"的机器状态，
    // 不是"下一帧将要进"的段）⇒ 在第 480 帧处读到的仍是 Attack（最后一帧 = 第 479 帧）。
    // 这条**逐块**的序列本身就是"状态真的在按 A/H/D 推进"的机械见证。
    assert_eq!(
        labels,
        vec![
            "Some(Attack)",  // 第 1 帧之后（最后一帧 = 第 0 帧）
            "Some(Attack)",  // 480 帧之后（最后一帧 = 第 479 帧，仍在起音段）
            "Some(Hold)",    // 720 帧之后（最后一帧 = 第 719 帧，落在保持段）
            "Some(Decay)",   // 960 帧之后（最后一帧 = 第 959 帧，落在衰减段）
            "Some(Sustain)", // 1200 帧之后（最后一帧 = 第 1199 帧，落在 sustain）
            "None",          // 释放走完 ⇒ 声部退休
        ],
        "⭐ 段位必须逐块按 A/H/D 的边界推进，并在释放走完后回到 `None`（退休）"
    );
}

/// ⭐ S36（任务单 `#92` 新增；⭐ **任务单 `#94` 按规范改写期望**）：**`loop_sustain` 与
/// `loop_continuous` 在 note-off 之后的输出必须不同**，且**各配一条"它自己在 note-off
/// 之后该怎样"的断言**（⛔ 不是只断言"两者不同"）。
///
/// # ⚠ 本判据为什么被改写（沿革，⛔ 不改写历史）
///
/// `#92` 曾按"`loop_sustain` ⇒ **走完当前循环**再进释放；`loop_continuous` ⇒ note-off
/// **根本不进释放**（`end_sample` 硬停）"实现。任务单 `#94` 查了 `sfzformat` 的两份页面后
/// **撤掉了这两条口径**（逐字引文见下）⇒ 本判据的**期望**随之改写（**编号与判据名不变**）。
///
/// # 量什么（对象 ＋ 单位，先说再跑）
///
/// 对象 ＝ 实时池在**斜坡样本**（⇒ 输出可**逐帧反解源下标**）上的输出；单位 ＝
/// **非零帧数**（存在性）＋ **源采样下标序列**（形状）＋ **释放段逐帧对账**
/// （`|实测 − 理论| ≤ 1e-6`）。
///
/// # 规范逐字（全部来自 `sfzformat`，⭐ 三条）
///
/// 1. <https://sfzformat.com/opcodes/loop_mode/>：「loop_sustain: the player will play the
///    loop while the note is held … **During the release phase, there's no looping.**」；
/// 2. 同页：「Whether no_loop, **loop_continuous** or **loop_sustain** is set, **the duration
///    of the release phase is set using `ampeg_release`**. However, in loop_sustain or
///    no_loop mode, **the sound can be cut off before the release phase ends, if the end of
///    sample is reached**. In loop_continuous mode, **the loop will repeat if the loop end
///    is reached during the release phase**, including repeating multiple times if the
///    release time is longer than the loop length.」；
/// 3. <https://sfzformat.com/legacy/>：「loop_sustain: … **The rest of the sample will play
///    after note release.**」
///
/// # 判定量（两条腿**各自**成立）
///
/// * ⭐ **`loop_continuous`**：note-off（第 200 帧）**就地进释放**，且**释放期照旧回绕**
///   ⇒ 非零**恰好** `200 + 480 = 680`（释放走完才结束）；⭐ **整段**与**独立**理论轨迹
///   （源下标按 `[100,164)` 循环 × 释放斜坡）逐点相符 ⇒ 那一条同时钉住"释放期仍在循环"；
/// * ⭐ **`loop_sustain`**：note-off **就地进释放**、释放期**⛔ 不循环**（沿源位置往后播）
///   ⇒ 第 200 帧起源下标**逐帧 +1、永不回绕**，直到**样本播完**
///   （`source_end = 400` ⇒ 最后一个可写下标 `398`）⇒ 非零**恰好** `200 + 263 = 463`
///   （⚠ "释放走完（680）"与"到样本尾（463）"**谁先到谁结束** ⇒ 这里样本尾先到）。
/// * ⭐ 反向腿（⛔ 防空绿）：两条腿的**非零帧数必须不同**（680 ≠ 463）；
///   `loop_sustain` 的输出**确实越过** `end_sample`（第 200 帧必须非零）；
///   释放段的理论值由 `pcm` ＋ 斜坡**独立算出**（⛔ 不拿实测反解自己比自己）；
///   ⭐ 前置见证：两份夹具在 note-off **之前**都实测周期 ＝ `loop_end − loop_start`。
#[test]
fn s36_loop_sustain_and_loop_continuous_differ_after_note_off() {
    const END_SAMPLE: usize = 200;
    const RELEASE_FRAMES: usize = 480; // 0.01 s @ 48 kHz
    /// ⭐ note-off 那一帧的源位置：窗口 `[100,164)`、周期 64 ⇒ `100 + (200 − 164) mod 64`。
    const SUSTAIN_POSITION_AT_NOTE_OFF: usize = LOOP_START + (END_SAMPLE - LOOP_END) % LOOP_PERIOD;
    /// ⭐ `loop_sustain` 在释放期还能写的帧数：源下标从 136 一路 +1 到最后一个可写下标 398。
    const SUSTAIN_TAIL: usize = (LOOP_PCM_FRAMES - 1) - SUSTAIN_POSITION_AT_NOTE_OFF;
    const FRAMES: usize = END_SAMPLE + RELEASE_FRAMES + 8;

    let text = |mode: &str| {
        format!(
            "<region> sample=a.wav key=60 pitch_keycenter=60 loop_mode={mode} \
             loop_start={LOOP_START} loop_end={LOOP_END} ampeg_release=0.01\n"
        )
    };
    let pcm = ramp_sample(LOOP_PCM_FRAMES);
    let continuous = prepare_with(&text("loop_continuous"), &pcm);
    let sustain = prepare_with(&text("loop_sustain"), &pcm);

    let continuous_out = render_pool(&continuous, END_SAMPLE as u64, FRAMES);
    let sustain_out = render_pool(&sustain, END_SAMPLE as u64, FRAMES);
    let count = |out: &[f32]| out.iter().filter(|sample| **sample != 0.0).count();

    // ⭐ `loop_continuous` 的**独立**理论轨迹：源下标永远按 `[100,164)` 循环，
    // 第 200 帧起乘 `(R − k)/R`（k ≥ R ⇒ 0 ⇒ 声部已退休）。
    let continuous_index = |t: usize| {
        if t < LOOP_END {
            t
        } else {
            LOOP_START + (t - LOOP_START) % LOOP_PERIOD
        }
    };
    let continuous_mismatches = (0..=END_SAMPLE + RELEASE_FRAMES)
        .filter(|t| {
            let ramp = if *t < END_SAMPLE {
                1.0
            } else {
                (RELEASE_FRAMES - (*t - END_SAMPLE)) as f32 / RELEASE_FRAMES as f32
            };
            (continuous_out[*t] - pcm.pcm[continuous_index(*t)] * ramp).abs() > 1.0e-6
        })
        .count();
    // ⭐ `loop_sustain` 的**独立**理论轨迹：第 200 帧起源下标**逐帧 +1**（⛔ 不回绕），
    // 直到样本播完（最后一个可写下标 = `LOOP_PCM_FRAMES − 2`）。
    let sustain_mismatches = (0..SUSTAIN_TAIL)
        .filter(|k| {
            let ramp = (RELEASE_FRAMES - k) as f32 / RELEASE_FRAMES as f32;
            (sustain_out[END_SAMPLE + k] - pcm.pcm[SUSTAIN_POSITION_AT_NOTE_OFF + k] * ramp).abs()
                > 1.0e-6
        })
        .count();
    // ⭐ "释放期⛔ 不循环"的**形状**读数：除掉释放斜坡后反解的源下标必须**严格递增**。
    let recover = |k: usize| {
        sustain_out[END_SAMPLE + k] * RELEASE_FRAMES as f32 / (RELEASE_FRAMES - k) as f32
    };
    let sustain_indices_increase = (1..SUSTAIN_TAIL).all(|k| recover(k) > recover(k - 1));
    // ⭐ 前置见证：note-off **之前**两份夹具都真的按窗口循环（⛔ 防"根本没出声"的假绿）。
    let continuous_period = detect_period(&continuous_out, LOOP_START, LOOP_END + 3 * LOOP_PERIOD);
    let sustain_period = detect_period(&sustain_out, LOOP_START, LOOP_END + 3 * LOOP_PERIOD);
    println!(
        "[engine-sfz/S36] ⭐ `loop_continuous`（end_sample={END_SAMPLE}）⇒ 非零 = {}（＝ note-off \
         ＋ 释放 {RELEASE_FRAMES} 帧）；全程与「循环 × 斜坡」不符 = {continuous_mismatches}；\
         ⭐ `loop_sustain` ⇒ 非零 = {}（＝ note-off ＋ 往后播到样本尾 {SUSTAIN_TAIL} 帧）；\
         释放段与「源下标恒 +1 × 斜坡」不符 = {sustain_mismatches}；反解源下标严格递增 = \
         {sustain_indices_increase}；前置周期 = {continuous_period} / {sustain_period}",
        count(&continuous_out),
        count(&sustain_out),
    );

    // ---- 腿 ①：`loop_continuous` —— note-off 进释放、释放期照旧循环 ----
    assert_eq!(
        continuous_period, LOOP_PERIOD,
        "⭐ 前置见证：`loop_continuous` 在 note-off 之前必须按 `loop_end − loop_start` 循环"
    );
    assert_eq!(
        count(&continuous_out),
        END_SAMPLE + RELEASE_FRAMES,
        "⭐ `loop_continuous`：note-off **就地进释放**且**释放期照旧回绕** ⇒ 声部活到释放走完"
    );
    assert_eq!(
        continuous_mismatches, 0,
        "⭐ `loop_continuous`：整段必须逐点等于「源下标按 [100,164) 循环 × 释放斜坡」\
         （|实测 − 理论| ≤ 1e-6）"
    );
    assert!(
        continuous_out[END_SAMPLE + RELEASE_FRAMES..]
            .iter()
            .all(|sample| *sample == 0.0),
        "⭐ `loop_continuous`：释放走完 ⇒ 逐位静音"
    );

    // ---- 腿 ②：`loop_sustain` —— 释放期⛔ 不循环，沿源位置往后播到样本尾 ----
    assert_eq!(
        sustain_period, LOOP_PERIOD,
        "⭐ 前置见证：`loop_sustain` 在 note-off 之前照旧「按住才循环」"
    );
    assert_eq!(
        count(&sustain_out),
        END_SAMPLE + SUSTAIN_TAIL,
        "⭐ `loop_sustain`：非零 = note-off（{END_SAMPLE}）＋ 往后播到样本尾（{SUSTAIN_TAIL}）\
         —— 释放（{RELEASE_FRAMES}）比样本尾巴长 ⇒ **样本尾先到**"
    );
    assert_eq!(
        sustain_mismatches, 0,
        "⭐ `loop_sustain`：释放段必须逐点等于「源下标 **恒 +1**（⛔ 不回绕）× 释放斜坡」\
         （|实测 − 理论| ≤ 1e-6）"
    );
    assert!(
        sustain_indices_increase,
        "⭐ `loop_sustain`：释放期的源下标必须**严格递增**（⛔ 不回绕）\
         —— 这就是「During the release phase, there's no looping」的机械读数"
    );
    assert!(
        sustain_out[END_SAMPLE] != 0.0,
        "⭐ 反向腿：`loop_sustain` 的输出必须**越过** `end_sample`（否则这条腿是空绿）"
    );
    assert!(
        sustain_out[END_SAMPLE + SUSTAIN_TAIL..]
            .iter()
            .all(|sample| *sample == 0.0),
        "⭐ `loop_sustain`：样本播完 ⇒ 逐位静音"
    );
    // ⭐ 两条腿必须**不同**（本节的核心判据；680 ≠ 463）。
    assert_ne!(
        count(&continuous_out),
        count(&sustain_out),
        "⭐ 核心：note-off 之后两者输出必须不同（前一片搬家过来的判据）"
    );

    // ---- 腿 ③：`one_shot` ＋ 有包络 ⇒ **连释放段也不进**（"忽略 note-off"是完整裁决）----
    let one_shot = prepare_with(&text("one_shot"), &pcm);
    let one_shot_out = render_pool(&one_shot, END_SAMPLE as u64, FRAMES);
    // 参照臂：同一个 `one_shot` 夹具但**没有** note-off（`end_sample = u64::MAX`）
    // ⇒ 逐帧对照才能证明"note-off 那一帧没有被释放段改写"。
    let one_shot_free = render_pool(&one_shot, u64::MAX, FRAMES);
    println!(
        "[engine-sfz/S36] ⭐ `one_shot` ＋ `ampeg_release`（end_sample={END_SAMPLE}）⇒ 非零 = {}；\
         第 {END_SAMPLE} 帧 = {}（⛔ 不许就地进释放）",
        count(&one_shot_out),
        one_shot_out[END_SAMPLE],
    );
    assert!(
        one_shot.regions()[0].ignores_note_off(),
        "`one_shot` 的既有裁决：忽略 note-off"
    );
    assert_eq!(
        count(&one_shot_out),
        LOOP_PCM_FRAMES - 1,
        "⭐ `one_shot` ＋ 有包络：note-off（＝ {END_SAMPLE}）既不退休、**也不起释放** ⇒ \
         声部照旧播到样本尾（{LOOP_PCM_FRAMES} 帧样本 ⇒ 最多 {} 帧）",
        LOOP_PCM_FRAMES - 1
    );
    assert_eq!(
        one_shot_out[END_SAMPLE], one_shot_free[END_SAMPLE],
        "⭐ `one_shot`：note-off 那一帧的电平必须等于「无 note-off」（⛔ 不是释放段的第 0 帧）"
    );
    assert_eq!(
        fnv1a_64(&one_shot_out),
        fnv1a_64(&one_shot_free),
        "⭐ `one_shot` ＋ 有包络：整段输出与「无 note-off」**逐位相同**"
    );
}

/// ⭐ S37（任务单 `#92`；§4.4）：**无 `ampeg_*` 的既有夹具逐位不变**。
///
/// # 量什么（对象 ＋ 单位，先说再跑）
///
/// 对象 ＝ 两份**一个 `ampeg_*` 都没有**的既有形状夹具（① 循环 ＋ 有限 `end_sample`；
/// ② `no_loop` ＋ `end_sample = u64::MAX`）经**实时池**渲染的输出；单位 ＝
/// **FNV-1a 64 指纹**（逐位）＋ **非零帧数**。
///
/// # 字面值的来源（⭐ 改动前实测）
///
/// 两个 `u64` 由本线在**改动任何源码之前**于 base `1beffe2a`（`aarch64/macos`、
/// 同一份夹具、同一条产品路径、一次性探针，测后删除）测得：
///
/// | 夹具 | 帧数 | 非零 | 指纹 |
/// | :--- | :--- | :--- | :--- |
/// | `loop_continuous` ＋ `end_sample = 200` | 512 | 200 | `0xa9dec8a87f69106c` |
/// | `no_loop` ＋ `end_sample = u64::MAX` | 512 | 399 | `0xd3c2a82076bd8947` |
///
/// ⚠ 与既有 [`NO_LOOP_FIXTURE_FINGERPRINT`]（`S33`）同一条纪律：字面值是**跨提交对账**
/// 的读数，⛔ 不是"跨架构必须相等"的规范常数 ⇒ 异平台**如实跳过并点名原因**。
///
/// # 反向腿（⛔ 让"绿"不能是空的）
///
/// 把同一段输出**乘 `1.0000001`** ⇒ 指纹**必须变**（证明它是**逐位**判据）。
#[test]
fn s37_a_fixture_without_any_ampeg_fields_renders_bit_identically() {
    /// ⭐ 改动前实测（base `1beffe2a`、`aarch64/macos`）。
    const LOOP_FIXTURE_FINGERPRINT: u64 = 0xa9de_c8a8_7f69_106c;
    /// ⭐ 改动前实测（同上）。
    const PLAIN_FIXTURE_FINGERPRINT: u64 = 0xd3c2_a820_76bd_8947;

    let looped = prepare_with(
        "<region> sample=a.wav key=60 pitch_keycenter=60 loop_mode=loop_continuous \
         loop_start=100 loop_end=164\n",
        &ramp_sample(LOOP_PCM_FRAMES),
    );
    let plain = prepare_with(
        "<region> sample=a.wav key=60 pitch_keycenter=60\n",
        &ramp_sample(LOOP_PCM_FRAMES),
    );
    assert!(
        looped.regions()[0].ampeg_is_transparent(),
        "夹具必须**一个 `ampeg_*` 都没有**（否则下面的指纹比对不是「缺省路径」的判据）"
    );
    assert!(plain.regions()[0].ampeg_is_transparent());

    for (label, plan, end_sample, frames, nonzero, expected) in [
        (
            "loop_continuous ＋ end_sample = 200",
            &looped,
            200u64,
            512usize,
            200usize,
            LOOP_FIXTURE_FINGERPRINT,
        ),
        (
            "no_loop ＋ end_sample = u64::MAX",
            &plain,
            u64::MAX,
            512usize,
            399usize,
            PLAIN_FIXTURE_FINGERPRINT,
        ),
    ] {
        let out = render_pool(plan, end_sample, frames);
        let actual = fnv1a_64(&out);
        let nonzero_actual = out.iter().filter(|sample| **sample != 0.0).count();
        // ⭐ 反向腿：逐位判据的敏感性见证。
        let scaled: Vec<f32> = out.iter().map(|sample| sample * 1.000_000_1).collect();
        let scaled_hash = fnv1a_64(&scaled);
        println!(
            "[engine-sfz/S37] {label}：非零 = {nonzero_actual}（改动前 {nonzero}）、\
             指纹 = 0x{actual:016x}（改动前实测 0x{expected:016x}）；\
             反向腿（× 1.0000001）指纹 = 0x{scaled_hash:016x}（必须不同）"
        );
        assert_eq!(nonzero_actual, nonzero, "⭐ {label}：非零帧数也必须不变");
        let _checked = check_fingerprint(&format!("S37 {label}"), actual, expected);
        assert_ne!(
            scaled_hash, actual,
            "⭐ 反向腿（{label}）：输出乘 1.0000001 ⇒ 指纹**必须**不同"
        );
    }
}

/// ⭐ S38（任务单 `#92`；§3.5）：**模块文档里那条"取线性段"的登记可判**。
///
/// # 判定量
///
/// `include_str!("../src/sfz_instrument.rs")`（模块头**源码**）必须含那条**逐字**登记
/// 「本仓取线性段；参考实现的指数／对数选项不在本片范围内」；
/// ⭐ 反向腿：文档里**没有**的那一句 ⇒ `contains` 必须 `false`（⛔ 判据不是恒真）。
///
/// ⚠ 形态照本仓"文本即判据"的既有做法（`S32` 同款）。
#[test]
fn s38_the_linear_segment_registration_is_judgeable() {
    const MODULE_SOURCE: &str = include_str!("../src/sfz_instrument.rs");
    /// ⭐ 任务单 `#92` 裁决第 2 条要求逐字写明的那一句。
    const NEEDLE: &str = "本仓取线性段；参考实现的指数／对数选项不在本片范围内";
    println!(
        "[engine-sfz/S38] 模块头含「线性段 ＋ 指数/对数不在本片范围」登记 = {}；\
         反向腿（文档里没有的句子）= {}",
        MODULE_SOURCE.contains(NEEDLE),
        MODULE_SOURCE.contains("本仓取指数段；参考实现的指数选项已在本片范围内"),
    );
    assert!(
        MODULE_SOURCE.contains(NEEDLE),
        "⭐ 模块头必须逐字登记：{NEEDLE}"
    );
    assert!(
        !MODULE_SOURCE.contains("本仓取指数段；参考实现的指数选项已在本片范围内"),
        "⭐ 反向腿：文档里**没有**的句子必须 `contains == false`（否则判据恒真）"
    );
    // ⭐ 第二条登记（⭐ 任务单 `#94` **按规范改写**）：note-off 语义表必须与
    // "`loop_continuous` 释放期照旧循环"一起在案；⭐ **反向腿**：旧口径那一句必须
    // `contains == false`（⛔ 不许留一句与实现相反的文档）。
    const NOTE_OFF_NEEDLE: &str = "释放期照旧循环（可以重复多次）";
    assert!(
        MODULE_SOURCE.contains(NOTE_OFF_NEEDLE),
        "⭐ note-off 语义表必须逐字在案：{NOTE_OFF_NEEDLE}"
    );
    const OLD_NEEDLE: &str = "note-off **不影响循环**：照旧循环并在 `end_sample` 硬停";
    assert!(
        !MODULE_SOURCE.contains(OLD_NEEDLE),
        "⭐ 反向腿：被任务单 `#94` 撤掉的旧口径必须 `contains == false`"
    );
}

// ---------------------------------------------------------------------------
// ⭐ S39～S45：**`group` / `off_by` / `off_mode` 接进实时播放**（任务单 `#94`）
//
// 规范逐字（全部来自 `sfzformat`，⭐ 三条）：
// * <https://sfzformat.com/opcodes/group/>：「Exclusive group number for this region.」
//   ＋「In these cases, the `group` number will be equal to the `off_by` number.」
//   ＋ 镲片扼音例（"the crash sounds would be in one group, the choke samples in another"）；
// * <https://sfzformat.com/opcodes/off_by/>：「Region off group. When a new region with a
//   group number equal to `off_by` plays, this region will be turned off.」；
// * <https://sfzformat.com/opcodes/off_mode/>：「This opcode will determinate how a region
//   is turned off by an `off_by` opcode.」—— `fast`（缺省）＝「The voice will be turned off
//   immediately. Release settings will not have any effect.」；`normal` ＝「The region will
//   be set into release stage.」
//
// ⚠ **作用面**（任务单 `#94` 按规范字面裁）：`off_mode` **只管 `off_by` 关断**，
// ⛔ **不管** MIDI note-off —— 那条路由 `loop_mode` ＋ `ampeg_*` 决定（见 `S35`/`S36`）。
// ---------------------------------------------------------------------------

/// ⭐ S39（任务单 `#94`；§4.3）：**`off_by` 真的把"停"落到那一个声部上**，
/// 且 victim 按**它自己**的关断规则衰减（两个读数：**释放起点** ＋ **非零长度**）。
#[test]
fn s39_off_by_really_stops_the_named_group_member() {
    const NOTE_OFF_AT: usize = 200;
    const RELEASE: usize = 480; // 0.01 s @ 48 kHz
    const TAIL: usize = 64;
    const FRAMES: usize = NOTE_OFF_AT + RELEASE + TAIL;
    /// ⭐ 样本 PCM 长度（`flat_sample(PCM)` 下最多写 `PCM − 1` 帧，见 `mix_span_into`）。
    const PCM: usize = 4096;

    // A：出声、`group=1`、`off_mode=normal`（⇒ 被关断时**进释放**）。
    // B：规范登记的**静音关断区**（`end=-1`）—— 它自己⛔ 不建声部，但照常关断别人
    //    （<https://sfzformat.com/legacy/> 的 `end` 词条逐字："Marking a region end with -1
    //    can be used to use a silent region to turn off other regions by using the `group`
    //    and `off_by` opcodes."）。
    let text = |off_by: u32| {
        format!(
            "<region> sample=a.wav key=60 pitch_keycenter=60 group=1 off_mode=normal \
             ampeg_release=0.01\n\
             <region> sample=s.wav key=62 pitch_keycenter=62 end=-1 off_by={off_by}\n"
        )
    };
    let sample = flat_sample(PCM);
    let plan = prepare_with(&text(1), &sample);
    let control = prepare_with(&text(0), &sample);
    let mut pool = SfzVoicePool::new(48_000.0);
    let mut control_pool = SfzVoicePool::new(48_000.0);
    assert_eq!(
        pool.trigger_note(&plan, 60, 127, u64::MAX, 1.0),
        1,
        "A 必须建 1 个声部"
    );
    assert_eq!(
        control_pool.trigger_note(&control, 60, 127, u64::MAX, 1.0),
        1
    );
    let mut out = vec![0.0f32; FRAMES];
    let mut control_out = vec![0.0f32; FRAMES];
    pool.render_into(&plan, 0, &mut out[..NOTE_OFF_AT]);
    control_pool.render_into(&control, 0, &mut control_out[..NOTE_OFF_AT]);
    assert!(
        out[..NOTE_OFF_AT].iter().all(|sample| *sample == 1.0),
        "⭐ 前置见证：关断之前 A 恒 1.0（`ampeg_attack` 缺省 0、sustain 缺省 100 %）"
    );
    assert_eq!(
        pool.voices()[0].end_sample(),
        u64::MAX,
        "关断之前 A 的引擎终点仍是「永不」（夹具给的就是 u64::MAX）"
    );
    // ⭐ 触发 B（静音关断区）：它自己⛔ 不建声部，但 `off_by=1` 必须关掉 group=1 的 A。
    assert_eq!(
        pool.trigger_note(&plan, 62, 127, u64::MAX, 1.0),
        0,
        "⭐ 静音关断区（`end=-1`）⛔ 不建声部（既有 `span == None` 口径）"
    );
    assert_eq!(
        control_pool.trigger_note(&control, 62, 127, u64::MAX, 1.0),
        0
    );
    assert_eq!(
        pool.voices()[0].end_sample(),
        0,
        "⭐ `normal` 关断 ＝ 把 victim 的 note-off 挪到「现在」（既有 note-off 机接手）"
    );
    assert_eq!(
        control_pool.voices()[0].end_sample(),
        u64::MAX,
        "⭐ 负对照：`off_by=0` ⇒ 一个都不关（同一份夹具、只差那一个数）"
    );
    pool.render_into(&plan, NOTE_OFF_AT as u64, &mut out[NOTE_OFF_AT..]);
    control_pool.render_into(
        &control,
        NOTE_OFF_AT as u64,
        &mut control_out[NOTE_OFF_AT..],
    );
    let count = |samples: &[f32]| samples.iter().filter(|sample| **sample != 0.0).count();
    let ramp_mismatches = (0..RELEASE)
        .filter(|k| {
            let expected = (RELEASE - k) as f32 / RELEASE as f32;
            (out[NOTE_OFF_AT + k] - expected).abs() > 1.0e-6
        })
        .count();
    println!(
        "[engine-sfz/S39] ⭐ `normal` 关断（第 {NOTE_OFF_AT} 帧触发）⇒ 释放起点电平 = {}；\
         非零 = {}（＝ 关断前 {NOTE_OFF_AT} ＋ 释放 {RELEASE}）；释放段与 (R−k)/R 不符 = \
         {ramp_mismatches}；⭐ 负对照（`off_by=0`）在窗内非零 = {}",
        out[NOTE_OFF_AT],
        count(&out),
        count(&control_out),
    );
    assert_eq!(
        out[NOTE_OFF_AT], 1.0,
        "⭐ 释放起点：关断那一帧的电平必须＝关断前那一刻（就地进释放，⛔ 不跳变、⛔ 不硬切）"
    );
    assert_eq!(
        count(&out),
        NOTE_OFF_AT + RELEASE,
        "⭐ 非零长度：关断前 {NOTE_OFF_AT} 帧 ＋ 恰好 {RELEASE} 帧释放"
    );
    assert_eq!(
        ramp_mismatches, 0,
        "⭐ 释放段逐点 ＝ (R − k)/R（|Δ| ≤ 1e-6）"
    );
    assert!(
        out[NOTE_OFF_AT + RELEASE..]
            .iter()
            .all(|sample| *sample == 0.0),
        "⭐ 释放走完 ⇒ 逐位 0（声部已退休）"
    );
    assert_eq!(
        count(&control_out),
        FRAMES,
        "⭐ 负对照：`off_by=0` ⇒ A 一路有声（窗内 {FRAMES} 帧全非零）"
    );
    assert!(
        count(&out) < count(&control_out),
        "⭐ 正/负对照（第 42 条）：关断腿必须比不关断那条**更短**"
    );
}

/// ⭐ S40（任务单 `#94`；§4.4）：**组号不匹配的区域必须完全不受影响**（⛔ 不误伤）。
///
/// ⭐ **负判据的正对照**（第 42 条）：同一批数据里同时有"该被停的"（A，`group=1`）与
/// "不该被停的"（C，`group=4`）；`off_by` 取 `1`（关 A）／`0`（不关任何）／`5`（⛔ 没有
/// 区域属于组 5）三跑 ⇒ 后两跑**整段逐位相同**，而第一跑必须不同；C 的**声部状态**
/// 在"关 A"与"不关"两跑之间也必须**逐字段相同**。
#[test]
fn s40_off_by_never_touches_a_region_outside_the_named_group() {
    const HALF: usize = 200;
    let text = |off_by: u32| {
        format!(
            "<region> sample=a.wav key=60 pitch_keycenter=60 group=1 off_mode=normal ampeg_release=0.01\n\
             <region> sample=s.wav key=62 pitch_keycenter=62 end=-1 off_by={off_by}\n\
             <region> sample=c.wav key=62 pitch_keycenter=62 group=4\n"
        )
    };
    let sample = flat_sample(4096);
    let run = |off_by: u32| {
        let plan = prepare_with(&text(off_by), &sample);
        let mut pool = SfzVoicePool::new(48_000.0);
        assert_eq!(pool.trigger_note(&plan, 60, 127, u64::MAX, 1.0), 1);
        let mut out = vec![0.0f32; HALF * 2];
        pool.render_into(&plan, 0, &mut out[..HALF]);
        // ⭐ 关断区是**静音**的 ⇒ 本次触发只可能改变 A 的状态。
        assert_eq!(
            pool.trigger_note(&plan, 62, 127, u64::MAX, 1.0),
            1,
            "只有 C 建声部"
        );
        pool.render_into(&plan, HALF as u64, &mut out[HALF..]);
        (pool, out)
    };
    let (pool_stop, out_stop) = run(1);
    let (pool_none, out_none) = run(0);
    let (pool_other, out_other) = run(5);
    println!(
        "[engine-sfz/S40] 指纹：off_by=1 ⇒ {:#018x}；off_by=0 ⇒ {:#018x}；off_by=5 ⇒ {:#018x}",
        fnv1a_64(&out_stop),
        fnv1a_64(&out_none),
        fnv1a_64(&out_other),
    );
    assert_eq!(
        fnv1a_64(&out_none),
        fnv1a_64(&out_other),
        "⭐ 组号不匹配（off_by=5，⛔ 没有区域属于组 5）⇒ 与「不关任何」逐位相同"
    );
    assert_eq!(out_none, out_other, "⭐ 逐位相同（比整段，⛔ 不止比指纹）");
    assert_eq!(
        pool_other.voices()[1],
        pool_none.voices()[1],
        "⭐ 声部状态读数：`off_by=5`（无人属于组 5）与 `off_by=0` 下 C 逐字段相同"
    );
    assert_ne!(
        fnv1a_64(&out_stop),
        fnv1a_64(&out_none),
        "⭐ 正对照：`off_by=1` 真的关掉了 A ⇒ 输出必须不同"
    );
    // ⭐ C 的声部**状态**（位置／增益／包络计数器／终点…）在关 A 与不关 A 两跑之间相同。
    assert_eq!(
        pool_stop.voices()[1],
        pool_none.voices()[1],
        "⭐ C 必须逐字段不受影响（它不在被点名的组里）"
    );
    assert_eq!(
        pool_stop.active_count(),
        2,
        "关断腿：A 进释放（仍 active）＋ C 在响 ⇒ 活声部 2"
    );
    assert!(
        pool_stop.voices()[0].is_active(),
        "⭐ `normal` 关断⛔ 不是硬停：A 在释放段里仍然是活声部"
    );
}

/// ⭐ S41（任务单 `#94`；原 §4.1 的"fast vs normal 可判"**改在 `off_by` 关断路径上考**
/// —— 那是规范唯一赋予 `off_mode` 意义的路径）：两条腿**都必须不同**，
/// 且**各配一条"它自己该怎样"的断言**（⛔ 只断言"不同"不够）。
///
/// 规范逐字（<https://sfzformat.com/opcodes/off_mode/>）：
/// * `fast`（缺省）：「The voice will be turned off immediately. Release settings will not
///   have any effect.」⇒ 关断帧起**逐位 0**（`ampeg_release` 不生效）；
/// * `normal`：「The region will be set into release stage.」⇒ 就地进释放段
///   （`ampeg_release × engine_hz` 帧线性归零）。
#[test]
fn s41_off_by_fast_cuts_immediately_and_normal_enters_release() {
    const NOTE_OFF_AT: usize = 200;
    const RELEASE: usize = 480;
    const TAIL: usize = 64;
    const FRAMES: usize = NOTE_OFF_AT + RELEASE + TAIL;
    let text = |mode: &str| {
        format!(
            "<region> sample=a.wav key=60 pitch_keycenter=60 group=1 off_mode={mode} \
             ampeg_release=0.01\n\
             <region> sample=s.wav key=62 pitch_keycenter=62 end=-1 off_by=1\n"
        )
    };
    let sample = flat_sample(4096);
    let run = |mode: &str| {
        let plan = prepare_with(&text(mode), &sample);
        let mut pool = SfzVoicePool::new(48_000.0);
        assert_eq!(pool.trigger_note(&plan, 60, 127, u64::MAX, 1.0), 1);
        let mut out = vec![0.0f32; FRAMES];
        pool.render_into(&plan, 0, &mut out[..NOTE_OFF_AT]);
        pool.trigger_note(&plan, 62, 127, u64::MAX, 1.0);
        pool.render_into(&plan, NOTE_OFF_AT as u64, &mut out[NOTE_OFF_AT..]);
        (pool, out)
    };
    let (fast_pool, fast_out) = run("fast");
    let (normal_pool, normal_out) = run("normal");
    let count = |samples: &[f32]| samples.iter().filter(|sample| **sample != 0.0).count();
    let ramp_mismatches = (0..RELEASE)
        .filter(|k| {
            let expected = (RELEASE - k) as f32 / RELEASE as f32;
            (normal_out[NOTE_OFF_AT + k] - expected).abs() > 1.0e-6
        })
        .count();
    println!(
        "[engine-sfz/S41] ⭐ `fast` ⇒ 非零 = {}（关断帧起逐位 0）；⭐ `normal` ⇒ 非零 = {}；\
         释放起点电平 = {}；释放段与 (R−k)/R 不符 = {ramp_mismatches}",
        count(&fast_out),
        count(&normal_out),
        normal_out[NOTE_OFF_AT],
    );
    // ---- 腿 ①：`fast` ⇒ 立刻关断（release 设置无效）----
    assert_eq!(
        count(&fast_out),
        NOTE_OFF_AT,
        "⭐ `fast`：关断帧起逐位 0 ⇒ 非零恰好 {NOTE_OFF_AT}"
    );
    assert!(
        fast_out[NOTE_OFF_AT..].iter().all(|sample| *sample == 0.0),
        "⭐ `fast`：`ampeg_release` 设置**无效**"
    );
    assert!(
        !fast_pool.voices()[0].is_active(),
        "⭐ `fast` ＝ 声部**当场退休**（池内状态翻转）"
    );
    // ---- 腿 ②：`normal` ⇒ 进释放段 ----
    assert_eq!(
        normal_out[NOTE_OFF_AT], 1.0,
        "⭐ `normal`：关断那一帧 ＝ 释放起点电平（就地进释放，⛔ 不跳变）"
    );
    assert_eq!(
        count(&normal_out),
        NOTE_OFF_AT + RELEASE,
        "⭐ `normal`：非零 ＝ 关断前 {NOTE_OFF_AT} ＋ 恰好 {RELEASE} 帧释放"
    );
    assert_eq!(ramp_mismatches, 0, "⭐ `normal`：释放段逐点 ＝ (R − k)/R");
    assert!(
        normal_out[NOTE_OFF_AT + RELEASE - 1] > 0.0,
        "⭐ `normal`：释放的**最后一帧**仍非零（释放段真的走满 {RELEASE} 帧 ⇒ ⛔ 不是硬停）"
    );
    assert!(
        !normal_pool.voices()[0].is_active(),
        "⭐ `normal`：**释放走完才退休**（与 `fast` 的「立刻退休」分开钉）"
    );
    // ---- 反向腿：两者必须不同 ----
    assert_ne!(
        count(&fast_out),
        count(&normal_out),
        "⭐ 核心：`fast` ≠ `normal`（{NOTE_OFF_AT} ≠ {}）",
        NOTE_OFF_AT + RELEASE
    );
}

/// ⭐ S42（任务单 `#94` 裁决 4）：**`one_shot` 的 victim 也忽略 `off_by`**
/// （"忽略 note-off"是一条**完整**裁决 ⇒ `off_by` 这种停止请求同样被忽略），
/// ⭐ **并配正对照**：同一批数据里把 victim 换成 `no_loop` ⇒ 关断**必须**发生
/// （⛔ 否则上面那条"逐位相同"可能是"夹具根本没触发"的假绿）。
#[test]
fn s42_a_one_shot_victim_ignores_off_by() {
    const HALF: usize = 200;
    let text = |mode: &str, off_by: u32| {
        format!(
            "<region> sample=a.wav key=60 pitch_keycenter=60 group=1 off_mode=normal \
             loop_mode={mode} ampeg_release=0.01\n\
             <region> sample=s.wav key=62 pitch_keycenter=62 end=-1 off_by={off_by}\n"
        )
    };
    let sample = flat_sample(4096);
    let run = |mode: &str, off_by: u32| {
        let plan = prepare_with(&text(mode, off_by), &sample);
        assert!(
            plan.regions()[0].ignores_note_off() == (mode == "one_shot"),
            "夹具自检：`ignores_note_off` 必须与 loop_mode 一致"
        );
        let mut pool = SfzVoicePool::new(48_000.0);
        assert_eq!(pool.trigger_note(&plan, 60, 127, u64::MAX, 1.0), 1);
        let mut out = vec![0.0f32; HALF * 2];
        pool.render_into(&plan, 0, &mut out[..HALF]);
        pool.trigger_note(&plan, 62, 127, u64::MAX, 1.0);
        pool.render_into(&plan, HALF as u64, &mut out[HALF..]);
        out
    };
    let one_shot_stopped = run("one_shot", 1);
    let one_shot_free = run("one_shot", 0);
    let no_loop_stopped = run("no_loop", 1);
    let no_loop_free = run("no_loop", 0);
    println!(
        "[engine-sfz/S42] ⭐ `one_shot`：off_by=1 与 off_by=0 的指纹 = {:#018x} / {:#018x}；\
         ⭐ 正对照 `no_loop`：{:#018x} / {:#018x}",
        fnv1a_64(&one_shot_stopped),
        fnv1a_64(&one_shot_free),
        fnv1a_64(&no_loop_stopped),
        fnv1a_64(&no_loop_free),
    );
    assert_eq!(
        one_shot_stopped, one_shot_free,
        "⭐ `one_shot` ＋ `off_by=1` 必须与 `off_by=0` **逐位相同**（它忽略 `off_by`）"
    );
    assert_eq!(
        fnv1a_64(&one_shot_stopped),
        fnv1a_64(&one_shot_free),
        "⭐ 同一条的指纹读数"
    );
    assert_ne!(
        no_loop_stopped, no_loop_free,
        "⭐ 正对照：同批数据里 `no_loop` 的 victim **必须**被关掉（否则上面那条是空绿）"
    );
}

/// ⭐ S43（任务单 `#94` 裁决 5）：**触发者不自掐**。
///
/// 夹具就是登记语料 `assets/samples/karoryfer-272-merry-orks/ork_dialogue.sfz` 的写法
/// （`group=1` ＋ `off_by=1` ＝ 同组独占）：第二次触发**必须关掉第一个声部**，
/// 而**第二个声部自己**必须照常发声 —— ⭐ 按 ARIA 的自掐解释，新声部会被自己立刻掐掉，
/// 那 20 处会**发不出声** ⇒ 本仓取 rgc／sfizz 一侧（不自掐）。
#[test]
fn s43_the_triggering_region_does_not_choke_itself() {
    const FIRST: usize = 100;
    let sample = flat_sample(4096);
    let plan = prepare_with(
        "<region> sample=a.wav key=60 pitch_keycenter=60 group=1 off_by=1\n",
        &sample,
    );
    let mut pool = SfzVoicePool::new(48_000.0);
    assert_eq!(
        pool.trigger_note(&plan, 60, 127, u64::MAX, 1.0),
        1,
        "第一次触发建 1 个声部"
    );
    let mut out = vec![0.0f32; FIRST * 2];
    pool.render_into(&plan, 0, &mut out[..FIRST]);
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.voices()[0].source_position(), FIRST as f32);
    // ⭐ 第二次触发**同一条区域**：它自己就是自己的 `off_by` 目标。
    assert_eq!(
        pool.trigger_note(&plan, 60, 127, u64::MAX, 1.0),
        1,
        "第二次触发建 1 个声部"
    );
    assert_eq!(
        pool.active_count(),
        1,
        "⭐ 恰好 1 个活声部：旧声部被关掉、**新声部没有被自己掐掉**\
         （自掐 ⇒ 0；完全不关 ⇒ 2 ⇒ 两种都被这条抓住）"
    );
    assert_eq!(
        pool.voices()[0].source_position(),
        0.0,
        "⭐ 活下来的是**新**声部（源位置从 0 起）"
    );
    pool.render_into(&plan, FIRST as u64, &mut out[FIRST..]);
    assert!(
        out[FIRST..].iter().all(|sample| *sample == 1.0),
        "⭐ 新声部必须照常发声（整段恒 1.0）—— 自掐会让它一个样本都发不出来"
    );
}

/// ⭐ S44（任务单 `#94`；第 259 条）：**非法的 `group` / `off_by` 响亮 `Err`**，
/// ⛔ 不静默降级；⭐ 同批数据里的合法样例**必须被接受**；⭐ 缺省与别名逐字回读。
///
/// ⚠ 域是 `yeban-sfz` 的**既有**口径（`read_u32` ⇒ `0..=u32::MAX`，与
/// `tests/malformed_inputs.rs` 的登记一致）—— 本片⛔ 不改解析、⛔ 不发明边界。
#[test]
fn s44_illegal_group_and_off_by_are_errors_not_defaults() {
    let load = |text: &str| SfzInstrument::load(text.to_owned(), &Default::default());
    // ---- ① 合法 ⇒ 原样读出 ----
    let legal = load("<region> sample=a.wav group=7 off_by=9\n").expect("⭐ 合法样例必须被接受");
    let plan = PreparedSfz::prepare(&legal, &flat_sample(64), 48_000.0).expect("摊平");
    let region = &plan.regions()[0];
    assert_eq!(region.group(), 7, "`group` 原样");
    assert_eq!(region.off_by(), 9, "`off_by` 原样");
    assert_eq!(
        region.off_mode(),
        OffMode::Fast,
        "`off_mode` 缺省是 `fast`（规范缺省）"
    );
    // ---- ② 缺省：`group=0` ＋ `off_by=0`（⛔ 不关任何人）----
    let default = load("<region> sample=a.wav\n").expect("parses");
    let default_plan = PreparedSfz::prepare(&default, &flat_sample(64), 48_000.0).expect("摊平");
    let default_region = &default_plan.regions()[0];
    assert_eq!(
        (default_region.group(), default_region.off_by()),
        (0, 0),
        "⭐ 缺省：组 0、⛔ 不关任何区域"
    );
    // ---- ③ 非法 ⇒ `Err`（⛔ 不是"降级 ＋ 记录"）----
    for text in [
        "<region> sample=a.wav off_by=-1\n",
        "<region> sample=a.wav group=-1\n",
    ] {
        assert!(
            matches!(load(text), Err(SfzError::IntegerOutOfRange { .. })),
            "⛔ {text:?} 必须**越界报错**（⛔ 不许静默钳到 0）"
        );
    }
    for text in [
        "<region> sample=a.wav off_by=abc\n",
        "<region> sample=a.wav group=abc\n",
    ] {
        assert!(
            matches!(load(text), Err(SfzError::InvalidInteger { .. })),
            "⛔ {text:?} 必须**认不出就报错**（⛔ 不许落缺省）"
        );
    }
    // ---- ④ 两个别名与四级作用域链同径 ----
    let alias = load("<group>polyphony_group=3 offby=4\n<region> sample=a.wav\n").expect("parses");
    let alias_plan = PreparedSfz::prepare(&alias, &flat_sample(64), 48_000.0).expect("摊平");
    assert_eq!(
        (
            alias_plan.regions()[0].group(),
            alias_plan.regions()[0].off_by()
        ),
        (3, 4),
        "⭐ `polyphony_group` / `offby` 两个别名经 `<group>` 段继承下来"
    );
}

/// ⭐ S45（任务单 `#94` §3.2 的第二条）：**`off_by` 的硬停只动 `active` 那一个声部，
/// ⛔ 不丢掉同槽的挂起者（`pending`）**。
///
/// 场景：池被 `SFZ_VOICES_PER_TRACK` 条区域灌满 ⇒ 第 17 条区域走**窃取**，那个槽里
/// `active` ＝ 正在淡出的 victim、`pending` ＝ 等着接管的新声部；此时再触发一条 `off_by`
/// 静音关断区 ⇒ victim 被**硬停**，而 `pending` 必须**照常接管**
/// （读数：`pending_count()` ＋ 接管后那个声部的 `sample_index` ＋ 输出非零）。
#[test]
fn s45_an_off_by_hard_stop_keeps_the_pending_voice_in_the_same_slot() {
    let mut text = String::new();
    for _ in 0..SFZ_VOICES_PER_TRACK {
        text.push_str("<region> sample=a.wav key=60 pitch_keycenter=60 group=7\n");
    }
    // 第 17 条：走窃取 ⇒ 挂起（用另一份 PCM ⇒ 接管后能靠 `sample_index` 认出来）。
    text.push_str("<region> sample=b.wav key=60 pitch_keycenter=60 group=7\n");
    // 关断区（另一个音符 ⇒ ⛔ 不占池的槽）：静音、`off_by=7`。
    text.push_str("<region> sample=s.wav key=62 pitch_keycenter=62 end=-1 off_by=7\n");
    let instrument = SfzInstrument::load(text, &Default::default()).expect("夹具必须解析成功");
    let plan = PreparedSfz::prepare_with_samples(
        &instrument,
        &[
            ("a.wav".to_string(), flat_sample(4096)),
            ("b.wav".to_string(), flat_sample(4096)),
        ],
        48_000.0,
    )
    .expect("摊平必须成功");
    assert_eq!(plan.samples()[0].path(), "a.wav");
    assert_eq!(plan.samples()[1].path(), "b.wav");
    let mut pool = SfzVoicePool::new(48_000.0);
    let created = pool.trigger_note(&plan, 60, 127, u64::MAX, 1.0);
    assert_eq!(created, SFZ_VOICES_PER_TRACK, "⭐ 前 16 条占满池");
    assert_eq!(pool.stolen(), 1, "⭐ 第 17 条走窃取");
    assert_eq!(pool.pending_count(), 1, "⭐ 窃取 ⇒ 恰好 1 个挂起者");
    assert_eq!(
        pool.voices()[0].sample_index(),
        0,
        "victim 仍占着槽 0（它还在淡出）"
    );
    // ⭐ 触发关断区：槽 0 的 active victim 被**硬停**（缺省 `off_mode=fast`）。
    assert_eq!(
        pool.trigger_note(&plan, 62, 127, u64::MAX, 1.0),
        0,
        "静音关断区⛔ 不建声部"
    );
    assert_eq!(
        pool.active_count(),
        0,
        "⭐ 组 7 的 16 个 active 声部全被关掉"
    );
    assert_eq!(
        pool.pending_count(),
        1,
        "⭐ 挂起者**必须还在**（⛔ 不许被 `off_by` 的硬停连带丢掉）"
    );
    let mut out = vec![0.0f32; 512];
    pool.render_into(&plan, 0, &mut out);
    assert_eq!(
        pool.pending_count(),
        0,
        "⭐ 同一块内挂起者就地接管（`render_slot` 第 2 步）"
    );
    assert_eq!(pool.active_count(), 1, "⭐ 接管之后恰好 1 个活声部");
    assert_eq!(
        pool.voices()[0].sample_index(),
        1,
        "⭐ 接管的是**挂起者**（`b.wav`），⛔ 不是被硬停的 victim"
    );
    assert!(
        out.iter().any(|sample| *sample != 0.0),
        "⭐ 接管者必须真的发声（⛔ 不是「槽还在但永远不出声」）"
    );
}
