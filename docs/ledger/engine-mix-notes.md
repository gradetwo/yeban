# `yeban-engine` 混音链台账：滤波 + 包络形状 + 声相 + 母线限制器 + 窃取淡出

- **台账类型**：信号路径图 / 口径表 / 判据与注入实测 / 未实现项（**不是规范**）
- **记录时刻**：2026-10-05（第 1 轮；本机 `workspace-write` 沙箱 + 受限 rustup）
- **工作线**：`line/engine-mix`（worktree `yeban/.worktrees/engine-mix`，基线 `44a071a`）
- **所有者目录**：`crates/yeban-engine/**`（本台账是唯一新增的文档文件）
- **落地范围**：
  - **新增** `src/mixer.rs`（声相定律 + 母线前瞻峰值限制器）、
    `tests/mix_render.rs`、`tests/limiter_contract.rs`、`tests/steal_fade.rs`、
    `tests/synth_filter.rs`；
  - **改写** `src/synth.rs`（`ToneParams` + 声部低通 + 3 ms 窃取淡出）、
    `src/snapshot.rs`（`pan_law` / `tones` 投影）、`src/rt.rs`（声相增益 + 母线限制器 + 2 个新统计）、
    `src/lib.rs`（模块地图）、`tests/support/mod.rs`（`MixSpec` / `tuned_project` / `SynthRig`）、
    `tests/synth_rt_zero_alloc.rs`（新增"整条混音链"零分配场景）
- **规范来源**：`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` —
  `ARCH-DSP-001`（纯 Rust 去爆音 / 安全限幅）、`ARCH-RT-001`（实时零分配）、
  `ARCH-RT-004`（声部窃取 3 ms 淡出）、`ARCH-DET-001`（L1 位精确）、
  `ARCH-PDC-001`（延迟上报）；`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md`
  `ROAD-M2-005/006`；`MODEL-AST-002`（`PanLaw` 枚举的唯一来源）
- **裁决来源**：`docs/adr/ADR-0001` 的 **D19**（feature 切分）、**D32**（跨架构按运算类别分策）、
  **D27**（`Op` 全集）、红线 7（实时零分配）

> 本文件回答六个问题：**混音链的每一级是什么、口径怎么定**、
> **哪些运算属于 D32 的哪一类**、**每条判据怎么变红（5 条注入实测）**、
> **零分配在整条混音链上是否仍然成立**、**哪些东西明确没做 / 需要谁裁决什么**。

---

## 0. 这一线改了什么（一句话）

改之前：逐轨合成已经出声，但母线是**等增益复制**（`sum_into_bus` 把单声道同时加到 L/R），
`TrackParams::pan` 与 `audio_config.pan_law` **不影响输出**；母线没有任何峰值约束
（实测峰值 **1.0058**）；声部没有滤波器、没有音色参数；声部池满时是**硬窃取**。

改之后：混音链的每一级都接上了，并且每一级都有**可机械验证**的判据：

```text
YebanProjectV1
  │  (控制线程投影, snapshot::from_project)
  ├─ schedules: BTreeMap<EntityId, NoteSchedule>     ← line/engine-sound
  ├─ tones:     BTreeMap<EntityId, ToneParams>       ← 本线（引擎侧临时形状）
  └─ pan_law:   PanLaw                               ← 本线
  ▼
RT: render_block
  │  逐轨: SynthEngine::render_track
  │         整数相位波表 + ADSR + **声部四级低通**（旁通时一次也不调用）
  │         ↓ track_scratch（声相之前、单声道）
  │        MeterBank::measure（口径不变）
  │         ↓
  │       sum_into_bus(cos θ, sin θ)   ← 构造期算好的声相增益
  │         ↓ 母线 L/R
  │       BusLimiter::apply            ← 前瞻 33 帧 + 软膝天花板
  │         ↓
  │       MeterBank::measure_bus_stereo（**限制之后**的读数）
  ▼
AudioBlock<128> ──► cpal / NullBackend
```

---

## 1. 声相定律（口径 + 规范依据 + 实测）

### 1.1 口径表

| `pan` | `gain_l` | `gain_r` | 每声道电平 | 备注 |
| :--- | :--- | :--- | :--- | :--- |
| −1.0 | `cos 0 = 1.0` | `sin 0 = 0.0` | 0 dB / −∞ | 全左：右声道**逐位**静音 |
| 0.0 | `cos(π/4) = √2/2` | `√2/2` | **−3.01 dB** | 居中 = 等功率 −3 dB |
| +1.0 | `cos(π/2) = −4.37e-8` | `sin(π/2) = 1.0` | −∞ / 0 dB | 全右（左声道只剩浮点残差） |

公式：`θ = (pan + 1) · π/4`、`(gain_l, gain_r) = (cos θ, sin θ)`；
`pan` 先钳到 `[−1, 1]`，非有限值按 `0`（居中）。

**规范依据**：`PanLaw::ConstantPowerMinus3dB` 是模型层的 `#[default]`
（`yeban-model::project::PanLaw`），本线的曲线与 `yeban-mcp` 离线渲染
（`src/domain/render.rs::pan_gains`，见 `docs/ledger/mcp-render-notes.md`）
**逐字同口径** —— 引擎与离线渲染不能各有一套声相定义。

**立体声行为（必须写清）**：本切片**没有**立体声源语义 —— 每条轨的渲染结果都是
**单声道**（波表声部本来就是单声道），因此"声相"是"把一个单声道信号按 `(cos θ, sin θ)`
分配给 L/R"。立体声片段的**立体声像**（stereo image）不属于本切片，见 §8 未实现项。

### 1.2 实测（M1/M2）

```text
[engine-mix] M1 rms centre_l=0.210861 centre_r=0.210861 hard_l=0.298202 hard_r=0.298202 ratio=0.707107
[engine-mix] M2 worst |centre - hard_left*√2/2| = 1.9686534180607396e-8
[engine-mix] M2 hard-right left residual rms=1.3237348078564565e-8 (nonzero=7646)
```

读法：

1. **居中 = 全左的 √2/2**：实测比值 `0.707107`（声明值 `0.70710678…`），
   且"居中样本 = 全左样本 × √2/2"的**逐样本最大偏差 1.97e-8**（f32 量级）；
2. **全左 ⇒ 右声道逐位 0**（`x * 0.0f32` 对有限 `x` 恒为 `+0.0`）；
3. **全右 ⇒ 左声道只剩 `cos(π/2)` 的浮点残差**：rms 1.32e-8，是"全左"的 2.2e-8 倍。
   这是数学事实（`cos(π/2)` 在 f32 里不是 0），判据因此断言**残差量级**而不是逐位相等。

### 1.3 ⚠ 规范缺口（不猜，登记为 needs）

`PanLaw` 的另三个变体在规范里**只有枚举名**：

| 变体 | 缺什么 | 当前处置 |
| :--- | :--- | :--- |
| `Linear` | "居中给 `(0.5, 0.5)`（−6.02 dB）"还是"不衰减 `(1, 1)`" | 与默认律同曲线（`match` 里显式列出，不是通配） |
| `ConstantPowerMinus4_5dB` | "−4.5 dB"指**居中额外衰减**（⇒ 居中 `(0.75, 0.75)`）还是另一族曲线 | 同上 |
| `ConstantPowerMinus6dB` | 同上（⇒ 居中 `(0.5, 0.5)`） | 同上 |

**为什么这样处置**：任何一种读法都会**改变居中电平**，而默认工程（夹具与
`filled_project`）都是居中的 ⇒ 猜错会让所有既有电平读数整体偏移 3 dB。
`yeban-engine::mixer::PanLaw::from_model` 是**穷举 match**（没有 `_` 兜底分支）
⇒ 模型层将来新增变体时这里会**编译失败**，而不是悄悄当成默认律。

---

## 2. 母线限制器（确定性 + 无分配 + 峰值保证）

### 2.1 为什么是**前瞻式**（look-ahead）而不是反馈式

| 方案 | 峰值保证 | 代价 |
| :--- | :--- | :--- |
| 反馈式（用过去样本/过去段的电平反馈增益） | **不能**：瞬态的第一个样本已经以未压缩的幅度写进输出 | 无延迟 |
| **前瞻式（本实现）** | **能**：增益由"窗口峰值"决定，且被延迟输出的样本就在窗口里 | 33 帧延迟 |

### 2.2 几何（这条是"实测踩出来的"，不是美学）

```text
环长 L = LOOKAHEAD_SAMPLES      = 33 帧
窗口 = [read ..= write]         = L 个样本 = "被延迟的样本 + 它之后的 16 帧"
输出 = 写位置上的**旧值**（= 33 帧前写进去的那个样本）× 本次算出的增益
不变量：L == 2 · LIMITER_LATENCY_FRAMES + 1   （编译期 assert!）
```

**为什么 L 必须是奇数**：窗口要同时覆盖 `x[n-L]`（被延迟的样本，上界证明需要它）
与"未来 16 帧" ⇒ `L - d = d + 1` ⇒ `L = 2d + 1`。第一版用 `L = 32`（偶数）配
`write + 17` 的读位置，结果 `x[n]` 自己跑到了输出端（**延迟实际为 0**），
"未超阈值逐位不变"的判据立刻变红（见 §6 的注入 I5 邻域记录）。

**输出延迟 = 33 帧 ≈ 0.688 ms @48k**。它计入 [ARCH-PDC-001] 的监听回路预算
（规范给"内部 DSP 拓扑调度"的总预算是 **1.00 ms**），占去约三分之二 ——
这也是"窗口不能再长"的实际约束。**尚未回填 `LatencyTable`**（见 §8 needs）。

### 2.3 弹道（非对称，这是判据的一部分）

```text
peak   = max(|x|) over 窗口（L 个样本，含被延迟的那个）
target = peak > T ? T / peak : 1.0            // T = LIMITER_THRESHOLD = 0.9
gain   = target <= previous ? target           // 攻击：一步到位（不受限）
                             : min(previous + release, target)   // 释放：每样本 ≤ 5e-5
```

| 量 | 值 | 含义 |
| :--- | :--- | :--- |
| `LIMITER_THRESHOLD` | `0.9`（≈ −0.92 dBFS） | 超过即开始压 |
| `LIMITER_CEILING` | `0.95` | 软膝渐近上界；输出**严格** ≤ 它 |
| `LIMITER_RELEASE_PER_SAMPLE` | `5e-5` | 从 0 回满约 0.42 s（20,000 帧） |
| 攻击 | 一步到位 | 实测 `gain[31]=1.0 → gain[32]=0.45`（同一帧让位） |

### 2.4 输出边界：**软膝天花板**（不是硬钳到阈值）

```text
|v| <= T          ⇒ 逐位恒等（一次乘法都不做）
|v| >  T          ⇒ T + (K − T) · (1 − exp(−(|v| − T)/(K − T)))，K = 0.95
```

**为什么用软膝而不是"硬钳到 T"**：
1. 硬钳在 `|v| ≈ T` 处留下一个**一阶折点**（增益曲线从"平滑释放"切到"硬上界"），
   听感上是阈值附近的轻微削波；软膝把那一小段过冲换成指数曲线 ⇒ 一阶连续；
2. 代价是**允许阈值之上有一小段过冲**，上界是 `K`：实测最大 `0.90015`
   （3.0 的尖峰，阈值之上 0.017%）。

**⚠ 这条容差必须写进判据，不能假装"输出 ≤ 0.9"**：第一版判据就是这么写的，
实测 `0.900029 / 0.900100 / 0.900150` 把它打红 —— 那是**判据写错了**，不是实现错了。

### 2.5 峰值上界证明（`limiter_contract` 的 L1 判据依据）

```text
设 W(n) = 窗口峰值，g(n) = 本次增益被钳到的上界，T = 阈值。
实现保证  g(n) <= T / W(n)      （target 的定义）
且        W(n) >= |x[n-L]|      （被延迟的样本就在窗口里，L ≥ 1）
⇒ |y[n]| = |x[n-L]| · g(n) <= |x[n-L]| · T / W(n) <= T
```

两种退化情形同样成立：`x[n-L] == 0 ⇒ |y| = 0`；`W(n) == 0 ⇒ target = 1`。
**实现层面的后果**：`y` 还要过一遍 §2.4 的软膝 ⇒ 实际上界是 `K`（比 `T` 宽 0.05）。

### 2.6 实测（M4/M5 + L1/L1b/L3）

```text
[engine-mix] M4 peak=0.900070 reductions=15201 ceiling=0.950
[engine-mix] M5 peak=0.137900 reductions=0        ceiling=0.950
[engine-mix] L1 peak=0.900100 ceiling=0.95 threshold=0.9 reductions=8188 engaged=true
[engine-mix] L1b square 1.5:        peak=0.900000 reductions=4096
[engine-mix] L1b impulse train 3.0: peak=0.900150 reductions=4096
[engine-mix] L1b dc 1.2:            peak=0.900000 reductions=4096
[engine-mix] L1b saw 2.0:           peak=0.900100 reductions=4096
[engine-mix] L2 compared=2015 samples bit-identically
[engine-mix] L3 left peak with/without right-channel transient: 0.252225 / 0.500000 = 0.504451
[engine-mix] L4 release: saw_release=true violations=0 final_gain=0.80687755
[engine-mix] L4 attack: gain[31]=1.000000 gain[32]=0.450000
```

**1.0058 那个夹具现在怎么样了（诚实回答）**：
`1.0058` 是**等增益复制**的产物 —— 单声道 `× 1.0` 写进两个声道，
因此"两个声部叠加"直接进母线。接上声相定律之后，同一个夹具（居中 = −3 dB）
的峰值变成 **0.711177**（实测 J1），**低于阈值**，限制器不介入。
也就是说："把 1.0058 压到阈值内"这件事，本线是**由声相定律 + 限制器共同**做到的；
真正的过阈值夹具（M4：+6 dB ⇒ 峰值约 1.42）由限制器压到 0.900070。

---

## 3. 声部滤波器 + 音色参数（D32 分类见 §4）

### 3.1 器件

`yeban_dsp::filter::LadderFilter`（四极 TPT / 零延迟反馈梯形低通，24 dB/oct，
`SOFT_KNEE = 0.7` 有界饱和，`DENORMAL_FLOOR = 1e-30` 软件冲刷）——
**直接用 dsp 的既有原语，没有改它的语义**（`line/dsp-level` 的 285 条冻结位模式判据仍然绿：
本线没有碰 `meter.rs`/`loudness.rs`，只在 `yeban-engine` 侧调用 `filter.rs`）。

### 3.2 参数来源：**引擎侧临时形状**（`yeban-engine::synth::ToneParams`）

`yeban-model` 目前**没有**"乐器参数 → 音频线程"的形状（`DeviceDefinition::params`
是 `Vec<ParameterValue>`：字符串名 + `f32` 值 + 可选单位；`DeviceKind` 只有
`Internal{Instrument,Effect}` / `External{Instrument,Effect}`，没有"合成器"这一档）。
因此本线在**自己拥有的目录**里定义了一层最小投影，**没有改模型**：

```text
现在（引擎侧临时形状）:                    等模型线补齐后:
TrackV3.devices[?]                          TrackV3.instrument: Option<InstrumentDefinition>
  kind == InternalInstrument                  ├ cutoff_hz: f32
  params: [ {name:"cutoff_hz", value: 1200}   ├ resonance: f32
            {name:"resonance", value: 0.2} ]  └ drive: f32
                                            ⇒ 本投影整体删除，改成直读字段
```

**投影规则**（完全确定，无猜测；实现见 `ToneParams::from_devices`）：

1. 只看 `DeviceKind::InternalInstrument` 的设备；`bypassed` 的设备整体忽略；
   `External*` 一律忽略（外部乐器归插件宿主）；
2. 在**第一个**含 `cutoff_hz`/`cutoff` 参数的设备上取值；
3. 三个参数名（大小写不敏感）：`cutoff_hz`|`cutoff`、`resonance`|`res`、`drive`；
4. **一个截止频率都没有 ⇒ 旁通**（`ToneParams::bypass()`）。这是刻意选的默认：
   "模型层没给参数"与"用户把滤波器拧到旁通"在音频上同解 ⇒ 既有的无设备链工程
   **逐位不变**；
5. 取值在构造期钳制：截止 `[20 Hz, 0.45·fs]`、共振/驱动 `[0, 1]`；
6. 非有限值按该参数的默认取（截止 12 kHz、共振/驱动 0），绝不 `NaN`。

**旁通的实现方式是"不调用 `process`"**，不是"把系数取成透明" ——
梯形滤波器的四级状态即使系数透明也会吸收瞬态并在起音处染色（判据 F1 钉住这一点）。

### 3.3 系数在**构造期**算（快照边界），逐样本只做乘加

`LadderFilter::configure` 含 `tan(π·fc/fs)`（超越函数类）；它在
`SynthEngine::begin_snapshot` 里对**每个轨道槽**算一次，触发时按值拷进声部
（`LadderFilter` 是 `Copy` 的四个 `f32` + 四级状态）。逐样本路径**只有**
`process`：乘加 + 一次 Padé 除法 + 比较 ⇒ IEEE 精确类。

### 3.4 实测（F1–F7）

```text
[engine-mix] F2 dc steady peak reference=0.700000 filtered=0.697824 ratio=0.996892
[engine-mix] F3 1760 Hz through 200 Hz LP: reference_rms=0.597536 filtered_rms=0.000112 attenuation=-74.55 dB
[engine-mix] F4 window peak=0.825159 max_step=0.008163
[engine-mix] F4 overshoot ratio=1.2767 (tail peak=0.646343)
[engine-mix] F5 resonance=1.0 peak=0.484982
```

| # | 判据 | 实测 |
| :-: | :--- | :--- |
| F1 | 旁通逐位恒等；接滤波器后必须不同 | 通过（`ToneParams::bypass()` ≡ `ToneParams::default()`） |
| F2 | 准直流通过 2 kHz 低通的稳态增益 ≈ 1 | **0.996892**（容差 5%） |
| F3 | 1760 Hz 过 200 Hz 低通的衰减 | **−74.55 dB**（远深于 −20 dB 门槛） |
| F4 | 阶跃响应无振铃、无过冲 | 最大相邻位移 **0.008163**（峰值 0.825）；过冲 **1.2767** 倍（容差 1.35） |
| F5 | 满共振 + 满驱动仍然有界、无次正规数尾巴 | 峰值 **0.484982**（< 8.0），次正规数 0 个 |
| F6 | `NaN`/越界参数被钳制 | 输出全有限、非静音 |
| F7 | 每声部各自的滤波器状态 | 第二音符起音前，第一音符的轨迹**逐位不变** |

---

## 4. D32 分类：混音链逐条归类

[ADR-0001 D32] 要求**按运算类别分策**（IEEE 精确类零容差；超越函数类 4096 ulp 预算）。
**滤波器与限制器的分类不同，必须分开说**：

| 层 | 运算 | 类别 | 何时发生 |
| :--- | :--- | :--- | :--- |
| 控制线程投影 | `note_to_hz`（`2^x`）、`db_to_gain`（`2^x`） | **超越函数类** | 每音符 / 每轨一次 |
| 控制线程投影 | tick→sample（乘/除/`round`） | IEEE 精确类 | 每音符一次 |
| 控制线程投影 | **`pan_gains` 的 `cos`/`sin`** | **超越函数类** | 每轨一次（修订变化时） |
| 构造期（快照边界） | **`ToneParams::filter` 的 `tan(π·fc/fs)`** | **超越函数类** | 每轨一次（修订/采样率变化时） |
| 构造期（快照边界） | `Adsr::set_params` 的 `exp` | **超越函数类** | 每修订一次 |
| **逐样本** | `phase.wrapping_add(inc)`、`u64::from(phase)*len >> 32` | 整数精确 | 每样本 |
| **逐样本** | 波表线性插值 `a + (b−a)*frac` | IEEE 精确类 | 每样本 |
| **逐样本** | `Adsr::process`（加/乘） | IEEE 精确类 | 每样本 |
| **逐样本** | **`LadderFilter::process`**（乘加 + Padé 除法 + 比较 + `abs`） | **IEEE 精确类** | 每样本 |
| **逐样本** | **声相乘加** `*l += *m * gain_l` | **IEEE 精确类** | 每样本 |
| **逐样本** | **限制器**：`max`/`abs`/除法/`min`/乘 | **IEEE 精确类** | 每样本 |
| **逐样本** | **软膝天花板**：`exp` | ⚠ **超越函数类** | 每样本（**只在过阈值时**） |
| 构造期 | 窃取淡出的 `exp`（`time_coefficient(0.003, fs)`） | 超越函数类 | 每次 `start_steal_fade` |

### 4.1 哪一条判据会在跨架构预算下放宽

1. **母线限制器的"未超阈值逐位不变"（L2/M5）**：链路里只有一次 `x * 1.0f32`，
   而乘法是 IEEE 精确类 ⇒ **跨架构仍逐位相同**，不放宽。
2. **限制器的"峰值 ≤ 天花板"（L1/M4）**：同样全是精确类 ⇒ 不放宽。
   但**软膝那一步含 `exp`** ⇒ 过阈值样本的**具体数值**落在超越函数类，
   跨架构只保证 4096 ulp 预算内；「≤ 天花板」这条**不等式**仍然严格成立
   （因为 `1 − exp(−t) < 1` 对任何浮点实现都成立，除非 `exp` 精度差到把
   `1 − exp(−t)` 算成 ≥ 1 —— 那需要 4000+ ulp 的误差，远超任何实现）。
3. **声部滤波器（F2/F3/F4）**：系数含 `tan` ⇒ **系数本身**是超越函数类。
   因此本线的声明是"**系数冻结后逐样本位精确**"，**不是**"整条滤波器链跨架构位精确"。
4. **声相定律（M1/M2/M3）**：`cos/sin` 在构造期 ⇒ 增益落在 4096 ulp 预算内；
   逐样本的两次乘法是精确类。M2 的"居中 = 全左 × √2/2"用的是**同一个** `gain_l`
   ⇒ 那条判据即使在跨架构下也成立（它比较的是同一进程内的两个渲染）。
5. **窃取淡出（S2–S6）**：`exp` 只在**构造期**（算 `release_coef`），
   逐样本只有乘 ⇒ 精确类；只有"淡出的具体衰减曲线"受 4096 ulp 预算影响。

**一句话**：本线的**逐样本**路径里唯一的超越函数是软膝天花板的 `exp`
（且只在过阈值时执行）；它影响的判据只有"过阈值样本的具体数值"，
"≤ 天花板"这条**不等式**判据不放宽。

---

## 5. 3 ms 声部窃取淡出 [ARCH-RT-004]

### 5.1 语义（本实现的取舍）

```text
池满 → 选一个被终止者（优先级见下）→ 标记 pending
      ├ 旧音符继续按旧增益输出，包络进入 3 ms 指数 release（覆盖 release 系数）
      └ fade_remaining = 144 倒数；归零的那一帧换成新音符（phase=0、env reset+gate_on）
新音符的起音因此最多推迟 3 ms；代价换来"旧音符淡出与新音符起音**永不同时发声**"。
```

**窃取优先级**（[ARCH-RT-004] 原文："优先窃取处于 Release 阶段尾部、振幅能量最低
（< −60 dBFS）或最早被触发的声音"）：包络在 `Release` **或**电平 < 0.001 者优先；
同档取 `start_sample` 最小者；再并列取下标最小者 ⇒ **全序**，跨平台确定。

### 5.2 事故记录（判据抓到的两个**实现 bug**）

1. **淡出被"顺手"改成了新音符的参数**：第一版在 `trigger` 里把被窃取声部的
   `gain`/`inc`/`end_sample` 直接换成新音符的，并且 `env.reset()` 把包络电平清零
   ⇒ 被窃取声部**瞬间静音**，"3 ms 淡出"等于没做。实测：软窃取与硬窃取的输出
   只差 1.97，且在触发当帧（1001）就分叉。**修法**：新音符的值寄存在 `note_*`
   字段里，淡出走完那一帧才启用。
2. **前瞻窗口几何（`L` 偶数）**：见 §2.2。

### 5.3 实测（S1–S6）

```text
[engine-mix] S4 steal_fade_frames=144 (3ms @48k = 144) steals=1
[engine-mix] S2/S3 difference peak hard=1.908356 soft=1.908358 | step@steal hard=0.555190 soft=0.000031 ratio=0.0001
[engine-mix] S5 soft difference peak=0.230575 max_step=0.031631 bound=0.037739
```

读数口径：**差分** `fixture() − reference_fixture()`。两个夹具只差"第 17 个音符的音高"
（被测 = 音高 108，参照 = 音高 45），窃取仍然发生 ⇒ 差分**恰好隔离出被窃取声部**。

| 臂 | 步骤 | 读数 |
| :--- | :--- | :--- |
| 硬窃取（`fade_frames = 0`） | 旧音符被瞬断 | 台阶 **0.555190** |
| 软窃取（3 ms = 144 帧） | 旧音符 3 ms 指数淡出 | 台阶 **0.000031**（**17910 倍**小） |
| 软窃取窗口内 | 每样本位移 ≤ 指数衰减上界 | **0.031631** ≤ 0.037739 |

### 5.4 ⚠ 规范冲突（登记为 needs，不在本线发明答案）

| 规范处 | 措辞 | 换算 |
| :--- | :--- | :--- |
| `ARCH-RT-004`（§3.2 第 4 条） | "**3 ms** 快速指数衰减微淡出" | 144 帧 @48k ⇒ **本线实现** |
| `ARCH-DSP-001`（§4.1） | "**5.0 ms** 升余弦窗或五次多项式平滑窗" | 240 帧 + 另一种曲线 |

两处是**同一个动作**（窃取时的淡出）的两种措辞。本线按任务书选择 `ARCH-RT-004` 的
3 ms 指数（`yeban-dsp::envelope::STEAL_RELEASE_SECONDS` 已经是这个值，
`Adsr::start_steal_fade` 也已经是这个 API），并把冲突登记为 needs（§8 N4）。

---

## 6. 判据表与注入（5 条，本机真跑）

### 6.1 判据清单

| 文件 | 编号 | 条数 | 覆盖 |
| :--- | :--- | :-: | :--- |
| `tests/mix_render.rs` | M1–M7 | 7 | 声相定律（居中/全左/全右/中右）、限制器端到端、透明、确定性、静音 |
| `tests/limiter_contract.rs` | L1–L6 | 7 | 器件上界（5 种过载形态）、透明、立体声联动、弹道、确定性、顶点 |
| `tests/steal_fade.rs` | S1–S6 | 4 | 窃取真的发生、淡出 vs 硬窃取的台阶、淡出窗口平滑、确定性 |
| `tests/synth_filter.rs` | F1–F7 | 7 | 旁通恒等、直流、频响、阶跃、有界、退化参数、每声部状态 |
| `tests/synth_rt_zero_alloc.rs` | J5（扩展） | 1 目标 | **整条混音链**的零分配（滤波器 + 声相 + 限制器 + 窃取） |
| `src/mixer.rs` 单测 | — | 11 | 声相曲线、等功率、退化输入、上界、透明、尖峰、弹道、零帧、`NaN` |
| `src/synth.rs` 单测 | — | 11 | 相位、tick、位精确、边界、力度、八度、槽位、窃取计数、重触发、可闻门 + **新增声部级** |

合计：**100 个 lib 单测 + 33 个集成判据 + 4 个 `harness = false` 目标**。

### 6.2 注入 → 变红 → 还原（本机真跑，`md5` 逐字节复核）

方法：改源码 → `cargo-local.sh test -p yeban-engine --no-default-features --all-targets --no-fail-fast`
→ 记录红点 → 从 `/tmp/inj/*.bak` 还原 → `md5` 与注入前**逐字节相同**（L24/L26）。

| # | 注入 | 变红的判据（实测） | 还原复核 |
| :-: | :--- | :--- | :--- |
| I1 | `LIMITER_THRESHOLD` 从 `0.9` 改成 `9.0`（等于摘掉限制器） | `mixer::tests::limited_peak_never_exceeds_threshold`、`limit_engages_in_a_single_sample`、`a_single_spike_…`、`overload_never_steps_…`、`gain_release_…`（5 条单测）；`limiter_contract` **6/7**；`mix_render::bus_limiter_caps_an_over_threshold_fixture` | `mixer.rs` md5 `e5c723b8…` 一致 |
| I2 | `pan_gains` 改回**等增益复制**（两侧都 1.0） | `mixer` 3 条单测（等功率/声明曲线/退化钳制）；`mix_render` **2** 条（M1/M2）；**并且** `synth_render::velocity_gate_and_monotonic_loudness` 变红 —— 见下面的读法 | `mixer.rs` md5 一致 |
| I3 | 声部滤波器**旁路**（`if false && !tone_bypass`） | 只有 `synth_filter::a_tone_above_the_cutoff_is_attenuated`、`bypass_is_bit_identical_to_no_filter_path` 变红 | `synth.rs` md5 `bb9ffa63…` 一致 |
| I4 | `steal_fade_frames` 恒返回 `0`（回到硬窃取） | `steal_fade` **3/4**：`fixture_really_steals_and_the_default_fade_is_three_milliseconds`（144 ≠ 0）、`steal_fade_bounds_…`（台阶 0.555 vs 0.000031）、`the_stolen_voice_leaves_without_a_step` | `synth.rs` md5 一致 |
| I5 | 声相**永不进母线**（引擎侧武装表恒为居中） | `mix_render` **2** 条（M1/M2）；**并且** J5 的混音链覆盖度自检变红：`[engine-mix/J5] 混音链: … 右声道非零=128` ⇒ `FAIL: 全左声相下右声道仍有 128 个非零样本` | `rt.rs` md5 `0742741b…` 一致 |

**I2 的额外读法（值得单独记下）**：把声相改回等增益复制之后，母线峰值从 0.711
回到 1.0058 ⇒ **限制器开始压那个力度 127 的音符**，于是
`J4 velocity_gate_and_monotonic_loudness` 的"力度线性比"从 127 掉到 **126.079** 变红。
这条链是：**声相定律 → 母线电平 → 限制器是否介入 → 力度线性判据**。
它同时说明了三件事：
1. 限制器**确实**接在母线上（否则 1.0058 不会被压）；
2. "力度线性"这条判据的**前提**是"整段不越阈值" —— 换句话说，它是
   **透明区**里的判据，越阈值之后它测的就是限制器的弹道了；
3. 一条判据的**前提**（夹具在透明区）必须与实现的口径一起维护 ——
   这属于"P9.7 同族：夹具隐含依赖了另一个模块的实现细节"。

---

## 7. 零分配：整条混音链（扩展 `harness = false` 目标）

`tests/synth_rt_zero_alloc.rs` 新增**场景 4**（**整条混音链**）：

```text
夹具：256 个交叠音符（铺满窗口）+ 40 个同时起音的长音符（逼出窃取）
      +6 dB 音量（峰值约 1.42 > 阈值 ⇒ 限制器真的在工作）
      pan = -1.0（声相真的在线）+ cutoff = 2 kHz / resonance = 0.4（低通真的被调用）
窗口：2,000 个量子（256,000 帧）
```

```text
[engine-sound/J5] mix chain (filter+pan+limiter+steal) 2_000 quanta: allocations=0 deallocations=0
[engine-mix/J5] 混音链: quanta=2001 reductions=256098 steals=27 最大压限=0.8878 右声道非零=0
```

**三重覆盖度自检**（防"什么都没跑"的假绿）：`reductions > 0`（限制器被驱动）、
`steals > 0`（淡出路径被走到）、`右声道非零 == 0`（声相真的在线）。
三者任一为 0 ⇒ 该场景**判失败**（而不是静默通过）。

前三个场景（10,000 量子饱和音符 + 63 次快照交换 + `filled_project` 4,000 量子）
原样保持绿。

---

## 8. 明确没做 / pending / needs / TODO(hoist)

### 8.1 本切片明确**没有**做

| 项 | 为什么没做 | 接入点 |
| :--- | :--- | :--- |
| **PDC 回填限制器延迟** | 限制器给母线引入了 **33 帧（0.688 ms）** 延迟，但 `LatencyTable` 里没有这一项（`from_project` 只读设备的 `latency_samples`） | `graph::LatencyTable::from_project` + `EngineSnapshot::from_project` |
| **真峰值（inter-sample peak）限制** | 限制器按**样本峰值**工作；真峰值可以比样本峰值高 1–3 dB。`yeban_dsp::meter` 有 4× 真峰值检测器，但没有接到限制器上（需要 4× 过采样 + 前瞻缓冲，属母带切片） | `BusLimiter::apply` 之前串一个 4× 过采样级 |
| **`PanLaw` 的另三个变体** | 规范只有枚举名、没有曲线定义（见 §1.3） | `mixer::pan_gains` |
| **立体声源 / 多声道** | 每条轨只渲染单声道；`ClipContent::Audio` 与立体声像未接 | `synth::render_track` 的返回类型 |
| **发送 / 辅助汇流、路由边 `gain_db`** | `RoutingEdge::gain_db` 目前被忽略；母线汇流是"所有非母线轨直接进 master"的扁平模型 | `rt::render_block` 的逐轨循环 |
| **限制器的增益衰减表（GR）上报** | 只暴露了 `limiter_gain_reductions` 与 `limiter_max_reduction` 两个累计量，没有按量子发布"当前 GR"给 UI | `meter::MeterFrame` 的扩展 |
| **参数自动化平滑（τ≈5 ms）** | [ARCH-DSP-001] 要求"所有瞬变自动化事件经单极点低通"；本切片的参数全部来自**不可变快照**，没有"实时改参数"的路径 | `yeban_dsp::smoothing` + 事件通道 |
| **滤波器类型不止一种** | 只有四极梯形低通（`LadderFilter`）；高通/带通/EQ/压缩未接 | `ToneParams` 的扩展 |

### 8.2 needs（需要别人 / 需要裁决）

| # | 事项 | 为什么需要裁决 |
| :--- | :--- | :--- |
| **N1** | **`PanLaw` 的曲线定义**：`Linear` / `ConstantPowerMinus4_5dB` / `ConstantPowerMinus6dB` 各自在 `pan = 0` 处给什么？（见 §1.3） | 任何读法都会整体改变居中电平（3 dB 量级）；本线只实现了默认律 |
| **N2** | **限制器延迟的回填口径**：33 帧是记在**母线节点**的 `LatencyTable` 上，还是记在"引擎输出的固定延迟"上？ | 决定 PDC 补偿要不要给其它分支插 33 帧延迟线；也决定"引擎延迟"是否要在 UI 披露 |
| **N3** | **真峰值 vs 样本峰值**：母线限制器按哪个口径？（`HD-26` 已经裁决 4× 过采样，但那是**计量**口径） | 若要拦 inter-sample peak，需要 4× 过采样 + 更多延迟（预算从 0.688 ms 变成 > 1 ms） |
| **N4** | **窃取淡出：3 ms 指数（`ARCH-RT-004`）还是 5 ms 升余弦（`ARCH-DSP-001`）？** | 两处规范措辞冲突（见 §5.4）；本线按 `ARCH-RT-004` 实现，改写规范正文需要人类 |
| **N5** | **乐器参数的形状**：`DeviceDefinition::params`（字符串键值对）如何投影成音频线程可读的定长参数集？ | 没有它，音色只能用本线的**临时**字符串约定（`cutoff_hz`/`resonance`/`drive`）；模型线补齐后本投影应整体删除 |
| **N6** | **混音链的架构归属**：`mixer.rs` 现在住在 `yeban-engine`（本线拥有）；若 `yeban-render`（离线母带）也要用同一份声相/限制器，应当上移到 `yeban-dsp` | 上移是**别人的地盘**（`yeban-dsp` 刚整理过），需要裁决"谁拥有混音链" |
| **N7** | **跨架构 L1 对账**：本线的逐样本路径里唯一的超越函数是软膝的 `exp`；建议在 CI 的 `arm` 档里覆盖"过阈值样本的 ulp 预算" | 需要 CI 改动（本线不动 `.github/**`）；`line/l1-digest` 的收据机制可以直接复用 |

### 8.3 跨线提醒（给集成者）

1. **`EngineStats` 新增了两个字段**（`limiter_gain_reductions`、`limiter_max_reduction`）
   ⇒ 纯增量，构造点只有 `EngineRuntime::stats()` 一处；`yeban-app` 的结构体字面量
   若**穷举**了 `EngineStats` 的字段就会编译失败（`yeban-app/src/engine_host.rs`
   只用 `stats()` 的读接口，实测无影响）；
2. **`SynthEngine` 的新增公共方法**：`tone()`/`tones()`/`pan_law()` 在
   `EngineSnapshot` 上；`armed_pan_gain()`/`armed_pan_slot_count()` 在
   `EngineRuntime` 上；`set_steal_fade_frames()`/`steal_fade_frames()` 在 `SynthEngine` 上；
   `debug_voice_state()`/`debug_active_voices()` 只在 `debug_assertions` 下编译。
   **`SynthEngine::begin_snapshot` 的签名变了**（多了第三个参数 `tones`）——
   `line/engine-sound` 之后如果有人调用过它，需要一起改；
3. **`ToneParams` 是引擎侧临时形状**（§3.2）：模型线补齐"乐器参数"之后，
   应当把 `ToneParams::from_devices` 的投影规则整段删掉，改成直读模型字段。

---

## 9. 本机跑了什么 / 没跑什么（严格区分）

| 命令 | 结果 |
| :--- | :--- |
| `bash scripts/dev/cargo-local.sh fmt --all` | ok |
| `bash scripts/dev/cargo-local.sh check -p yeban-engine --no-default-features --all-targets` | ok |
| `bash scripts/dev/cargo-local.sh clippy -p yeban-engine --no-default-features --all-targets` | **零告警**（本机；CI 的 `-D warnings` 也覆盖默认 feature 一侧） |
| `bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --all-targets` | **全绿**：100 lib + 7 + 7 + 6 + 4 + 9 集成 + 4 个 `harness = false` 目标 |
| 5 次注入 + 还原（§6.2） | 全部按要求变红，还原后 4 个文件 md5 与注入前**逐字节相同** |
| `bash scripts/gates/run-gates.sh light` | 绿（见 §9.1） |
| `run-gates.sh crate yeban-engine` | **未跑**：本机禁止（cpal 重依赖），交给 CI |
| `cargo test --workspace` / benchmark / fuzz | **未跑**（纪律禁止） |

### 9.1 CI 判决

```text
run id  : 待读（见下方 §9.2 的读回记录）
```

**注意（L23/L26）**：本机绿不等于 CI 绿。本线的代码改动（含 `mixer.rs` 的 `exp`）
在 **CI 的 `rust (yeban-engine)` 腿（默认 feature = cpal 在编）** 上会再跑一次
clippy + test；`workspace 全量` 是否进 matrix 取决于 `scripts/dev/changed-crates.py`
的计划器（`line/engine-sound` 的 N8 已经记录它认不出 `workspace = true` 的依赖边）。

### 9.2 CI 判决（**已读回**，不是 pending）

```text
run id  : 37244879720   （line/engine-mix, push 触发, tip 9b03ca7）
结论    : failure —— 5 绿 1 红 2 跳过
  ✓ plan (受影响集合)                5s
  ✓ checks (fmt / 红线守卫 / schema) 32s
  ✓ deny (cargo-deny)                44s
  ✓ lockfile (确定性 Cargo.lock)     16s
  ✓ rust (yeban-engine)              54s   ← clippy -D warnings + test，**默认 feature（含 cpal）**
  ✗ rust (yeban-app)                 4m1s  ← clippy ✓ / test ✗（1 条红，11 条绿）
  - rust (workspace 全量)                  ← 被 plan 跳过
  - windows (yeban-mcp / yeban-model)      ← 与本改动无关
```

**红点原文**（`ci-verdict.sh --logs 37244879720` 摘录）：

```text
test admin_reload_engine_rebuilds_and_resets_the_meter_tap ... FAILED
panicked at crates/yeban-app/tests/live_ui_mcp.rs:892:
assertion `left == right` failed: 引擎换代之后旧读数必须作废（电平回到下限）
  left: "轨道 鼓 电平表 峰值 -6.0 RMS -23.4 dBFS"
 right: "轨道 鼓 电平表 峰值 -120.0 RMS -120.0 dBFS"
test result: FAILED. 11 passed; 1 failed
```

### 9.3 这个红点是什么（诊断，附证据）

**它不是本线引入的回归，而是 `44a071a` 修好"假绿生成器"之后**第一次**把所有下游拖进矩阵**，
于是暴露了一个**跨线潜伏失败**（与 `engine-sound-notes.md` §7 的 S3 **完全同族**）：

1. 该判据的**副作用链**是"注入一帧电平 ⇒ `ui/reload_engine` 重建引擎 ⇒
   **旧的注入帧必须作废** ⇒ 读回下限 −120 dBFS"；
2. 它隐含的前提是"**新引擎的前几帧没有任何声源**"——那在 `line/engine-sound`
   之前是**真的**（`render_track_into` 是 `out.fill(0.0)` 占位静音）；
3. `line/engine-sound` 让引擎**真的出声**（`demo_project` 的鼓轨有 MIDI 音符）
   ⇒ 重建后的引擎在第 4 个量子就有真实电平 −6.0 dBFS；
4. 而 `rust (yeban-app)` **从来没在 `line/engine-sound` 的推送里跑过** ——
   正是 `44a071a` 记录的 N8（计划器看不见 `{ workspace = true }` 依赖边）。
   所以这个红点在 `line/engine-mix` 之前的 `main` 上**已经潜伏**，只是没人编译到它。

**为什么本线不能修**：修复点在 `crates/yeban-app/tests/live_ui_mcp.rs`，
而任务书把 `其它 crates/**` 列为**禁改**。处置建议（给 `line/app-mixer` / 集成者）：

- **(a) 改判据意图**：把"读回 −120 dBFS"改成"读回的**不是**注入的那一帧"
  （例如断言 `quanta == 4`、`visibleQuantum == 4`，且读数来自引擎）。
  这条更贴近判据标题"重建引擎并重置电平抽头"的**真实意图**；
- **(b) 或者**把 `demo_project` 换成"无音符"的工程（与 `meter_rt_contract.rs::silent_project()`
  的做法一致）——那样"新引擎前几帧静音"重新成立，判据一字不改。

**本线已做的部分**：`line/engine-sound` 修 S3 时已经采取过 (b) 的做法；
`engine-sound-notes.md` §7 末尾那条"给电平线的提醒"说的就是这个形态
（**夹具隐含依赖了另一个模块的实现细节**，P9.7 同族）。
