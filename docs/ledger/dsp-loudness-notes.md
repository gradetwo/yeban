# `yeban-dsp` 响度与真峰值台账：8× 真峰值、BS.1770 门限/窗口、其它采样率系数（`dsp-loudness` 工作线）

- **台账类型**：口径表 / 改建前后实测对照 / 推导记录 / 判据清单 / 注入记录 / 未决项（**不是规范**）
- **记录时刻**：2026-10-05（本机 Apple M2 / aarch64，workspace-write 沙箱 + 受限 rustup）
- **工作线**：`line/dsp-loudness`（worktree `yeban/.worktrees/dsp-loudness`，起点 main `42442af`）
- **所有者目录**：`crates/yeban-dsp/**`（本台账是唯一新增的文档文件）
- **规范 / 裁决来源**：`ARCH-UI-002`（真峰值与 RMS 电平）、`ARCH-RT-001`（实时零分配）、
  `ARCH-DET-001`（L1 确定性）、`ROAD-M2-008`；
  [`ADR-0001`](../adr/ADR-0001-workspace-topology-and-version-pinning.md) 的
  **D32**（跨架构位精确按运算类别分策）、**D43**（1.0.0 之前直接推翻，不留兼容）、
  **D19**（feature 切分）；[`human-decisions.md`](human-decisions.md) 的
  **`HD-26`**（真峰值提到 8×/16×）与 **`HD-27`**（补齐 LUFS 门限/窗口 + 其它采样率）
- **零新增依赖**：`crates/yeban-dsp/Cargo.toml` 的 `[dependencies]` 仍然是空的；
  根 `Cargo.toml` / `Cargo.lock` **一行未动**（`git diff --stat` 只有两个 `.rs` 与一个 `tests/*.rs`）。
  两个功能都只用 `std` 的 `f32`/`f64` 算术 + **离线预计算的字面量表**。

> 本文件回答：**真峰值从 4× 提到 8× 到底改善了多少（同一夹具的读数是几）**、
> **留下的欠读是哪一条、为什么不是 bug**、**要不要 16×**、
> **LUFS 的门限/窗口口径与实测数字**、**其它采样率的系数是怎么推导出来的、怎么验证的**、
> **哪些既有语义一位没改、哪些被显式改写了**、**每条判据怎么变红（3 条注入实测）**、
> **本机真跑 vs 交给 CI 的严格区分**、**明确没做什么**。

---

## 0. 这一线做了什么（一句话）

把 `yeban-dsp` 的两条**已被负责人裁决但尚未实现**的口径做出来：
**真峰值从 4× 多相过采样提到 8×**（`HD-26`，在 `f = 0.4·fs` 处把固有欠读从
**−0.436 dB** 收到 **−0.0005 dB**，并另留 16× 可选），
**LUFS 从"无门限最小子集"补到完整的门限积分 + 瞬时/短时窗口**
（`HD-27`：−70 LUFS 绝对门限、−10 LU 相对门限、400 ms 块 / 75% 重叠、3 s 短时），
并把 K 加权系数从"只有 48 kHz"扩到 **44.1 / 48 / 88.2 / 96 kHz 四档**
（同一解析原型经双线性变换推导，其中 48 kHz 一档能复现 BS.1770-4 正文的系数表）。

**关闭的 need**：`dsp-level-notes.md` §8 的 **N3**（真峰值倍数）、**N5**（LUFS 门限切片）、
**N6**（其它采样率的 K 加权系数）三条由本线落地。那份台账属于**另一条线**的所有者，
因此本线不改它 —— N3/N5/N6 的关闭需要在集成者侧补记（见 §10 的 L1）。

---

## 1. 裁决 → 落地对照

| 裁决 | 要求 | 本线落地 | 判据 |
| :--- | :--- | :--- | :--- |
| `HD-26` | 真峰值过采样 **4× → 8×/16×** | 默认 8×（`TRUE_PEAK_PHASES = 8`，`TRUE_PEAK_TAPS = 32`）：`0.4·fs` 处欠读 −0.4359 → **−0.0005 dB**、频率轴最坏值 −0.4359 → **−0.1330 dB**；16× 走 `with_oversampling(16)`（最坏 −0.0636 dB）；旧 4× 核表与旧判据按 D43 **一并替换**，公开面不留 4× 路径 | `eight_times_oversampling_fixes_the_four_tenths_nyquist_underread`、`true_peak_steady_state_underread_is_within_the_documented_budget`、`true_peak_preserves_the_sample_grid`、`true_peak_kernel_is_coherent`（§7） |
| `HD-27`（a） | LUFS **−70 LUFS 绝对门限 + −10 LU 相对门限**（400 ms 块、75% 重叠） | `GatedLoudness`：两遍滤波的**精确**门限积分；流式部分（瞬时/短时）30 个跳的固定环、零分配 | `the_absolute_gate_sits_at_minus_seventy_lufs`、`the_relative_gate_removes_the_quiet_section`、`silence_padding_pulls_ungated_loudness_down_but_not_the_gated_one` |
| `HD-27`（b） | **3 s 短时**窗口 | `GatedLoudness` 的 `short_term_lufs` / `max_short_term_lufs`（30 个 100 ms 跳的滑窗） | `short_term_window_is_three_seconds_wide`、`momentary_window_covers_four_hundred_milliseconds_with_seventy_five_percent_overlap` |
| `HD-27`（c） | **其它采样率的 K 加权系数** | 内置 44.1 / 48 / 88.2 / 96 kHz 四档，全部由同一条解析原型推导；其余仍然显式拒绝 | `derived_coefficients_reproduce_the_itu_48k_table`、`minus_twenty_dbfs_997hz_is_minus_twenty_lufs_at_every_supported_rate`、`k_weighting_is_rate_consistent_on_a_wideband_fixture` |
| `D43` | 1.0.0 之前**直接推翻**，不留 `deprecated` 兼容层 | 旧的 `TRUE_PEAK_KERNEL`（4×/16 抽头）从公开面**删除**；旧判据 `four_times_oversampling_underreads_at_four_tenths_nyquist_as_documented` 被替换（4× 核表只以**测试内对照仪器**的形式保留，理由见 §2.2） | §6 的"被显式改写的判据" |
| `D32` | 跨架构位精确**按运算类别分策** | 真峰值核 / K 加权递归 / 门限累加全部是 IEEE 精确类（逐位）；只有**测试里的系数推导**碰 `tan`/`powf`，给 1e-12 预算 | §6 |

---

## 2. 真峰值：4× → 8×（按需 16×）

### 2.1 核的生成口径（写清楚，免得后人猜）

```text
C = T/2 = 16                       // sinc 中心 = 延迟（基础采样率样本）
h_p[m] = sinc(m − C + p/L) · Kaiser(m; β = 8, 中心 (T−1)/2, 支撑 [0, T−1])
L = 8（相位数 = 过采样倍数）, T = 32（每相抽头数）, β = 8
逐相位归一化: h_p[m] ← h_p[m] / Σ_k h_p[k]        // 每相直流增益 = 1
相位 0 强制为 δ(m − C)                            // 过采样输出含原始样本
```

- **口径不是新发明的**：同一条公式（β = 8、**窗中心 (T−1)/2**、逐相位归一化）
  能复现本仓库**旧的 4× 核表**到 **1.278e-8** 以内（残余是 f32 最短往返表示的舍入）——
  实测方法：用该公式在 `f64` 里重算 4×/16 抽头表（`L = 4, T = 16`）再按最短往返
  转 `f32`，与 `meter.rs` 里冻结的 16 个字面量逐项比较，最坏偏差 1.278e-8
  （对 `~0.9` 的量级是 ~1 ulp 的 1/5）。本线只是把 `L` 从 4 提到 8。
- 详细推导的**另一个实证**：`L = 16` 表的**偶数相位**与 `L = 8` 表**逐位相同**
  （`sinc(m − 16 + 2k/16) = sinc(m − 16 + k/8)`，且窗与归一化完全相同）——
  判据 `sixteen_times_contains_eight_times_phases_bit_for_bit` 钉住这一点。
- 系数按**最短往返表示**写成 `f32` 字面量（与 `crate::oversample` 同一纪律），
  `#[rustfmt::skip]` 保持表形状可读；表在源码里是**数据**，运行时零计算 [ARCH-RT-001]。
- **每个相位的直流增益**是"逐相位归一化"的直接结果（实测四档表的最大偏差 6.96e-8）。

### 2.2 改建前后实测对照（**同一夹具、同一段测量代码**）

夹具：`x[i] = sin(2π·0.4·i)`，8192 个样本（`f64` 相位算完再转 `f32`），
**先喂 512 个样本把窗填满**（理由见 §2.4），再在余下样本上取峰值。

| 计量器 | `f = 0.4·fs` 的读数 | dBFS | 相对 1.0 的欠读 |
| :--- | ---: | ---: | ---: |
| **改建前 4×**（冻结核表） | `0.951056540` | −0.435873 | **−0.435873 dB** |
| **改建后 8×**（默认） | `0.999942541` | −0.000499 | **−0.000499 dB** |
| 16×（可选） | `0.999942541` | −0.000499 | −0.000499 dB |

**改善 = +0.435374 dB**（判据 `eight_times_oversampling_fixes_the_four_tenths_nyquist_underread`
断言 8× 的读数落在 1.0 的 ±0.01 内、且改善 > 0.4 dB）。

物理原因：`f = 0.4·fs` 的连续峰值落在 `t = 5/8` 样本上。4× 的网格是 `1/4` 的倍数
⇒ 最近的网格点差 `1/8` 样本 ⇒ 读到 `cos(π/10) = 0.9511`。
8× 的网格是 `1/8` 的倍数，`5/8` **正好落在网格上** ⇒ 残余只剩核本身的带边误差。

> **关于"对照仪器"**：4× 的核表仍以 `FROZEN_FOUR_TIMES_KERNEL`（测试模块私有常量）
> 的形式存在，它**不是兼容层**（公开面没有任何 4× 路径），而是让上面这张表
> 在 **CI 里可复跑**的测量仪器：判据在同一次运行里同时算出 4× 与 8× 的读数，
> 而不是把"改建前"写成一个只有作者见过的数字。这是 D43 允许的（推翻的是公开语义，
> 保留的是证据）。

### 2.3 稳态扫频：**相称频率**才是频率轴上的最坏点

同一段测量代码（4096 个样本、前 512 个样本填窗）。频点分两类，这是本方案**实测**出来的结构：

- **一般频率**（有理数但分母大 ⇒ 真峰与 `1/L` 网格的相对位置遍历整周）：网格几乎总能落在
  真峰附近，读数 ≈ 1；
- **相称频率**（分母小 ⇒ 相对位置**固定**）：欠读表现为**网格梳**，最坏值出现在这里。

| `f/fs` | 类别 | 4× [dB] | 8× [dB] | 16× [dB] |
| ---: | :--- | ---: | ---: | ---: |
| 0.02 | 一般 | −0.0010 | −0.0011 | −0.0011 |
| 0.05 / 0.15 / 0.25 / 0.35 / 0.375 / 0.45 / 0.47 | 一般 | 0.0000 | +0.0000 | +0.0000 |
| 0.10 | 一般 | −0.0020 | −0.0017 | −0.0017 |
| 0.20 | 一般 | −0.0030 | −0.0000 | −0.0000 |
| 0.30 | 一般 | −0.0027 | −0.0014 | −0.0014 |
| 1/3 | 一般 | −0.0050 | −0.0005 | −0.0005 |
| 0.4375 | 相称（7/16） | — | +0.0000 | +0.0000 |
| **0.40** | 相称（2/5） | **−0.4359** | **−0.0005** | −0.0005 |
| 0.42 | 一般 | −0.0172 | −0.0026 | −0.0026 |
| 13/30 = 0.4333 | 相称 | —（未单独测） | −0.0227 | −0.0227 |
| 0.44 | 相称（11/25） | −0.0172 | −0.0172 | −0.0172 |
| 7/15 = 0.4667 | 相称 | — | −0.0477 | −0.0477 |
| 0.48 | 相称（12/25） | −0.0172 | −0.0172 | −0.0172 |
| 6/13 = 0.4615 | 相称 | — | −0.0636 | **−0.0636** |
| 5/11 = 0.4545 | 相称 | −0.0889 | −0.0889 | −0.0625 |
| 4/11 = 0.3636 | 相称 | −0.0889 | −0.0889 | −0.0002 |
| **4/9 = 0.4444** | 相称 | −0.1330 | **−0.1330** | −0.0271 |
| **最坏值** | — | **−0.4359 @ 0.40** | **−0.1330 @ 4/9** | **−0.0636 @ 6/13** |

（"未单独测"表示该频点是分析时发现的次级点，只在 8× 列上补了读数；
所有给出数字的格子都是 **Rust 检测器**的实测值，与 `f64` 解析模型一致到 ≤0.005 dB。）

**如实钉住的残余**（`HD-26` 要求"如果 8× 在某个频点仍有固有欠读，如实钉住并量化"）：

1. **8× 的频率轴最坏点是 `4/9·fs`（≈0.4444·fs），读 `0.984808` ⇒ −0.1330 dB**。
   它与 4× 在同一点**持平**（4× 的真峰也落在网格的同一距离上），
   但 4× 在更常见的 `0.4·fs` 上是 −0.4359 dB ⇒ **最坏值整体收紧 0.303 dB**。
   残余的成因是"相称频率下真峰与 `1/8` 网格的距离固定为 `1/36` 周期"这类几何关系，
   不是核的缺陷：解析值 `20·log10(sin(4π/9)) = −0.1330 dB` 与实测逐位吻合。
2. **一般频率上几乎无欠读**：实测最坏 −0.0017 dB（`f = 0.1·fs`），
   `0.05 / 0.15 / 0.25 / …` 上恰为 0。
3. **核的带边下垂**：`T = 32` 的窗化 sinc 在 `f ≥ 0.44·fs` 时单个相位的幅度开始明显偏离 1
   （实测：`f = 0.45` 最坏相位 −7.4e-2，`f = 0.49` 最坏相位 **−0.734**）。
   但检测器取的是"相位 × 网格"的最大值，所以**读数**仍然紧；
   单相位下垂不是 bug，是 `T = 32` 的物理极限 —— 判据 `true_peak_kernel_is_coherent`
   把"带边至少有一个相位掉到 0.6 以下"机械地记下来，免得后人误以为表坏了。

判据 `true_peak_steady_state_underread_is_within_the_documented_budget` 用一张
**逐频点预算表**（`TRUE_PEAK_STEADY_FLOOR`，23 个频点，含 `4/9`、`5/11`、`4/11`、
`6/13`、`7/15`、`13/30` 这些相称频率）把上表钉死，并额外钉住"最坏点必须仍是 `4/9·fs`、
最坏值仍在 −0.13 dB 量级"。**同一批预算下 4× 会大面积变红**（`0.4·fs` 一项就跌到 0.9511）。

### 2.4 冷启动瞬态（**诚实交代，且方向是安全侧**）

FIR 从零状态起步时，窗化 sinc 的暂态会**过读**：同一段 `f = 0.4·fs`、8192 样本的
夹具，**不填窗**直接测：

| 计量器 | 整块读数（含冷启动） | dBFS |
| :--- | ---: | ---: |
| 4× | `0.951056540` | −0.435873（= 网格欠读，暂态没超过它） |
| **8×** | `1.056788564` | **+0.479781（过读）** |

原因：新核更长（最坏相位的 `Σ|h|`：**2.3692** vs 旧核 **1.9053**），"头一个窗还没填满"的暂态更大。
**这不是稳态失准**，处置方式与上一线对直流阶跃 Gibbs 过冲的处置完全一致：
判据先喂满一个窗（`TAPS` 个样本，实际给 128～512 个样本的余量）再测量。
过读方向在**安全侧**（限制器不会因此漏拦），但它**不能**被当成稳态值 ——
这一点写进了 `TruePeakDetector` 的类型文档第 2 条与判据的注释。

### 2.5 要不要 16×？（结论：**默认 8×；母带母线/导出用 16×**）

**先纠正一版错误结论（留痕）**：本线第一版判据只覆盖 `0.02…0.48` 的 16 个频点，
恰好**漏掉了最坏的相称频率**（`4/9`、`5/11`），于是得出"16× 收益 < 0.01 dB、纯属浪费"
的结论。把相称频率补进频点表之后结论被推翻，实测如下（Rust 检测器，4096 样本 / 填窗 512）：

| 量 | 8× | 16× |
| :--- | ---: | ---: |
| 最坏读数所在频点 | **`4/9·fs`** | **`6/13·fs`** |
| 最坏读数 | 0.984807730（**−0.1330 dB**） | 0.992708862（**−0.0636 dB**） |
| `4/9·fs` 处的读数 | 0.984807730 | 0.996883929（**+0.1059 dB**） |
| `4/11·fs` 处的读数 | 0.989821434 | 0.999981701（+0.0913 dB） |
| 一般频率（≥0.9998） | 相同 | 相同 |

- **最坏值收紧 +0.0694 dB**，**最坏点处收紧 +0.1059 dB** ⇒ 16× **确实**有收益；
- **结构性保证**：16× 表的偶数相位与 8× 表逐位相同，而 16× 的网格是 8× 的**超集**
  ⇒ 同一输入下 `16× ≥ 8×`（逐位成立）。判据
  `sixteen_times_tightens_the_worst_case_grid_comb_by_the_documented_margin` 同时钉住
  "不小于"、"最坏点各自是谁"、"两项收益落在 0.05–0.10 / 0.08–0.13 dB 之间"；
- **成本**：2× 乘加（512 vs 256 MAC/样本）与 2× 表体积（512 vs 256 个 `f32`）；
- **结论**：**默认 8×** —— 逐通道计量（大工程里每个声部都挂表）时成本敏感，
  而最坏 −0.133 dB 已经比 4× 的 −0.436 dB 收紧 0.303 dB；
  **母带母线 / 导出天花板用 16×** —— "只付一次"的场景，把最坏欠读压到 −0.064 dB，
  这对 `-1 dBTP` 这类硬天花板是有意义的余量。
- 再往上要不要 32×：只对"更细的网格"有效，且一旦 `1/32` 网格比 `T = 32` 的核还细，
  卡住精度的就变成**核长度**（带边下垂）与冷启动瞬态。那条路的正确做法是
  **同时加长 `T`**（48/64 抽头）并重新口径化瞬态 —— 记为 §10 的 N1。

### 2.6 接口形状的变化（D43：旧的替换掉，不留兼容层）

| 旧（main `42442af`） | 新 | 说明 |
| :--- | :--- | :--- |
| `TRUE_PEAK_PHASES = 4` | `TRUE_PEAK_PHASES = 8` + `TRUE_PEAK_PHASES_HIGH = 16` | 相位数成为显式口径 |
| `TRUE_PEAK_TAPS = 16` | `TRUE_PEAK_TAPS = 32` | 每相抽头数加长（抑制带边下垂） |
| `TRUE_PEAK_LATENCY_SAMPLES = 8` | `= TRUE_PEAK_TAPS / 2 = 16` | 延迟随窗长走（幅度量不受影响，但调用方要能查到） |
| `TRUE_PEAK_KERNEL: [[f32;16];4]` | `TRUE_PEAK_KERNEL_8X` / `TRUE_PEAK_KERNEL_16X`（私有） | 核表仍然私有，倍数由构造器选 |
| `TruePeakDetector::new()` | `new()`（8×）+ `with_oversampling(n) -> Option<Self>` + `oversampling()` | 多一个"选倍数"的入口；无 `deprecated` 别名 |
| 判据 `four_times_oversampling_underreads_...`（钉住 4× 的欠读） | `eight_times_oversampling_fixes_the_four_tenths_nyquist_underread`（同一夹具上给**两个**读数） | §6 的"被显式改写的判据" |

`yeban-engine` 侧的 `pub use ... TRUE_PEAK_PHASES, TRUE_PEAK_TAPS,
TRUE_PEAK_LATENCY_SAMPLES, TruePeakDetector` **一个字都没改**，仍然编译通过
（本机实测：`yeban-engine --no-default-features --all-targets` 全绿，§9）。

---

## 3. LUFS：门限积分 + 瞬时/短时窗口

### 3.1 口径表

| 量 | 定义 | 常量 |
| :--- | :--- | :--- |
| K 加权 | BS.1770-4 两级：高架（Stage 1）+ RLB 高通（Stage 2），`f64` 状态 | 四档系数见 §4 |
| 跳 `hop` | 100 ms | `GATING_HOP_SECONDS = 0.1`（48 kHz ⇒ 4800 样本） |
| 瞬时 momentary | 最近 **4 个跳 = 400 ms** 的均方 → LUFS，每跳更新一次 | `MOMENTARY_SECONDS = 0.4` |
| 短时 short-term | 最近 **30 个跳 = 3 s** 的均方 → LUFS，每跳更新一次 | `SHORT_TERM_SECONDS = 3.0` |
| 门限块 | **400 ms 块 + 75% 重叠**（块 = 4 个跳，块间隔 = 1 个跳） | `GATING_BLOCK_SECONDS = 0.4` |
| 绝对门限 `Γa` | 块响度 ≤ Γa 的块**不参与** | `ABSOLUTE_GATE_LUFS = −70.0` |
| 相对门限 `Γr` | `Γr = (过 Γa 的块的平均响度) − 10 LU` | `RELATIVE_GATE_LU = −10.0` |
| 积分响度 | 对 `loudness > max(Γa, Γr)` 的块求能量平均 → LUFS | — |
| 读数 | `LUFS = −0.691 + 10·log10(z)`，`z = (Σy_L² + Σy_R²)/frames`；能量 0 ⇒ **负无穷** | `LUFS_OFFSET_DB = −0.691` |
| 不足一个块的尾巴 | **丢弃**（块必须完整，与参考实现一致） | — |

### 3.2 门限语义的实测（**这就是"静音段不被拉低"的证据**）

夹具：1 秒 −20 LUFS 的 997 Hz 立体声正弦 + 1 秒静音，48 kHz。

| 计量器 | 读数 | 解释 |
| :--- | ---: | :--- |
| `LoudnessMeter`（**无门限**，上一线语义） | **−23.010311** | 静音段的零能量进了分母（能量减半 ⇒ −3.01 dB） |
| `GatedLoudness`（**门限积分**） | **−20.705866** | 静音块被 Γa 排除；只剩三块"跨在切换点上"的 400 ms 块 |

残余的 −0.706 dB **全部来自块边界**：75% 重叠的块里有三块跨在静音上，
分别含 3/4、1/2、1/4 的信号能量 ⇒ `10·log10((7 + 0.75 + 0.5 + 0.25)/10) = −0.706 dB`。
这不是门限失效，而是**标准的分块口径**；判据把这两个数**同时**钉住。

**绝对门限两侧**（单条 997 Hz 正弦，2 秒）：

| 夹具 | 无门限读数 | 门限积分读数 |
| :--- | ---: | ---: |
| −75 LUFS（低于 `Γa`） | −75.00 | **−∞**（没有可积分的块） |
| −65 LUFS（高于 `Γa`） | −65.00 | −65.00 |

**相对门限**（10 秒 −20 LUFS + 10 秒 −40 LUFS）：

| 口径 | 读数 |
| :--- | ---: |
| 无门限 | −22.967075 |
| **只**过绝对门限 | −22.967075（两段都在 −70 之上，Γa 一个人都拦不住） |
| **门限积分**（Γa + Γr，`Γr ≈ −32.97`） | **−20.064968** |

⇒ "−20.065"这个读数**只有相对门限存在时才可能出现**：少了 Γr 会读到 −22.967。

### 3.3 瞬时/短时窗口的实测

夹具：4 个跳的 −20 LUFS 正弦 + 4 个跳的静音。
**窗口 = 最近 4 个跳**（400 ms），所以每个跳的读数是：

| 跳 | 窗口内容 | 理论值 | 实测（48 kHz） |
| :-: | :--- | ---: | ---: |
| 1–3 | 未满 | −∞ | −∞ |
| 4 | 响×4 | −20.000 | −19.9971 |
| 5 | 响×3 + 静 | −21.2494 | **−21.2453** |
| 6 | 响×2 + 静×2 | −23.0103 | −23.0061 |
| 7 | 响 + 静×3 | −26.0206 | −26.0160 |

第 5 个跳的 −21.245 **唯一对应 4 跳窗口**：若窗口是 3 跳 ⇒ −21.76、5 跳 ⇒ −20.97、
1 跳 ⇒ −40。判据因此能真正抓住"窗口长度/重叠写错"（实测注入见 §8）。

短时窗口：第 1–29 个跳恒为 **−∞**，第 30 个跳给出 **−19.9963**（3 s 窗口刚满）。
`max_momentary_lufs` / `max_short_term_lufs` 分别记录两者迄今的最大值
（RF64/BW64 的 `MaxMomentary` / `MaxShortTerm` 字段要的就是它们）。

### 3.4 为什么积分是**两遍**（设计取舍，写清楚免得被当成缺陷）

相对门限 `Γr` 的定义依赖"**过绝对门限的那些块**的平均响度"，而它只有看完整段信号
才知道；判断一个块是否过 Γr 又必须等 Γr 定下来。BS.1770-4 的参考实现同样是
"先收集再重算"。因此 `GatedLoudness::integrated_stereo` 走**两遍滤波**：

```text
第一遍: 过 Γa 的块 → 平均响度 → Γr = 平均响度 − 10 LU
第二遍: 对 loudness > max(Γa, Γr) 的块求能量平均 → 积分响度
```

- **好处**：逐位精确（不是直方图近似）、**零分配**（块 = 4 个跳 ⇒ 只用 4 个 `f64` 的环）、
  **零状态**（没有隐藏的"上次测量"），适合离线导出/母带。
- **代价**：2× 滤波成本（离线路径，可接受）。
- **流式部分是有界的**：`GatedLoudness` 的 30 个跳的固定环 + 4 个跳的窗口是**实时安全**的
  （`add_*` 里零分配、零锁、零 I/O [ARCH-RT-001]），UI 响度表用的正是这部分。
- **没做流式 integrated**：那需要把全部块的 `z` 存下来（无界）或做量化直方图（有损）。
  两条路都不划算 ⇒ 记为 §10 的 N2（若 UI 真要"边放边看 integrated"，再按需要选一条，
  并把量化误差写进口径）。

### 3.5 与"无门限"口径的差异（**显式改写**，不是悄悄变红）

上一线的 `LoudnessMeter` 与它的全部判据**一位都没改**（§5）。本线新增的是**第二种计量器**，
并把原来那条"把无门限语义钉住"的判据**显式改写**为同时钉住两种语义
（`silence_padding_pulls_ungated_loudness_down_but_not_the_gated_one`）。
这是 `dsp-level-notes.md` §8 N5 明确要求过的处置方式。

---

## 4. 其它采样率的 K 加权系数（44.1 / 48 / 88.2 / 96 kHz）

### 4.1 推导口径（**这就是"你是怎么得到的"**）

BS.1770-4 正文只给 48 kHz 的系数表（表 1 = 高架、表 2 = RLB 高通）。**其它采样率不是插值**，
而是把**同一个解析原型**用**预扭曲双线性变换**重新离散化：

```text
Stage 1（高架）:  f0 = 1681.974450955533 Hz,  G = 3.999843853973347 dB,
                  Q  = 0.7071752369554196,    Vb = Vh^0.4996667741545416,  Vh = 10^(G/20)
Stage 2（RLB 高通）: f0 = 38.13547087602444 Hz, Q = 0.5003270373238773
变换:  K = tan(π·f0/fs)
  a0 = 1 + K/Q + K²
  shelf     = [ (Vh + Vb·K/Q + K²)/a0,  2(K² − Vh)/a0,  (Vh − Vb·K/Q + K²)/a0,
                2(K² − 1)/a0,  (1 − K/Q + K²)/a0 ]
  high_pass = [ 1, −2, 1,  2(K² − 1)/a0,  (1 − K/Q + K²)/a0 ]     // 分子固定 (1 − z⁻¹)²
```

这份推导**不是自说自话**：把它在 48 kHz 上跑一遍，得到的就是 BS.1770-4 正文表 1/表 2 的数字
（实测最大偏差 **3.3e-16 相对**，即 1–2 ulp of `f64`，见 §4.2 的判据）。
四档的系数作为 `f64` 字面量**离线算好写进源码**（运行时零超越函数 ⇒ 跨架构逐位），
48 kHz 那一档**原样保留**上一线的字面量（逐位不变的契约靠它）。

> **为什么离线预计算而不是运行时算**：运行时算要用 `tan`/`powf`，那是**超越函数类**，
> 跨架构不保证位模式（D32）。写成字面量之后，整条计量路径只剩加/减/乘/除
> （IEEE 精确类，跨架构逐位相同）。判据里的推导是**验证**用的，不在热路径上。

### 4.2 验证（三层，全部可机械复跑）

1. **锚点**：推导 ⇒ **复现标准正文的 48 kHz 系数表**（容差 1e-12，实测 ≤3.3e-16）。
   判据 `derived_coefficients_reproduce_the_itu_48k_table`。
2. **表内自洽**：推导 ⇒ **内置的四档表逐项一致**（同一条判据的后半段）。
   改任何一个内置系数（哪怕 1e-9）都会变红（§8 注入 3 实测）。
3. **行为互校**：同一段**模拟信号**在四档采样率下的读数一致（§4.3），
   并且"拿 48 kHz 系数硬套 96 kHz"会偏 **0.822 dB**（判据里显式跑一遍这个反例）。

### 4.3 逐率实测

**997 Hz、−20 dBFS 立体声正弦（BS.1770 的标定点）**：

| 采样率 | 无门限读数 | 门限读数 | 相对 −20.0 的偏差 | 硬套 48 kHz 系数会读到 |
| ---: | ---: | ---: | ---: | ---: |
| 44.1 kHz | −19.997343 | −19.997141 | **+0.0027 dB** | −19.791124（偏 0.206 dB） |
| 48 kHz | −20.000105 | −19.999903 | +0.0001 dB | （就是它自己） |
| 88.2 kHz | −20.015779 | −20.015583 | **−0.0158 dB** | −20.622337（偏 0.607 dB） |
| 96 kHz | −20.017427 | −20.017231 | **−0.0174 dB** | −20.649679（偏 0.632 dB） |

**宽带夹具**（100 + 1000 + 5000 Hz 之和，5 秒）：

| 采样率 | 无门限读数 |
| ---: | ---: |
| 44.1 kHz | −17.711996 |
| 48 kHz | −17.715944 |
| 88.2 kHz | −17.736189 |
| 96 kHz | −17.738140 |
| **跨度** | **0.0261 dB**（容差 0.05 dB） |
| 96 kHz 用 48 kHz 系数 | **−18.538059**（偏 **0.822 dB**） |

### 4.4 残余偏差的来源，以及为什么**不**改 −0.691

−0.691 是 `−0.691 + 10·log10(z)` 里的一个**常数**，标准给的值对应 48 kHz 的数字滤波器
（它的 997 Hz 功率增益恰好是 `10^(0.691/10)`）。同一解析原型在不同采样率上离散化后，
997 Hz 的增益略有不同（双线性变换的频率扭曲）⇒ 其它三档的标定点有 ±0.017 dB 的残余。

- **本实现选择保留常数 −0.691**（标准的口径），代价是上面那张表里的 ±0.017 dB；
- **没有**改成"每率归一化"的偏置：那会让 997 Hz 标定点更漂亮，但**不再是 BS.1770 的公式**，
  与外部工具的读数会对不上；
- 0.017 dB 比任何实用的响度判据阈值（通常 0.1 LU）都小一个数量级，而"硬套系数"的
  后果是 0.2–0.8 dB —— 两者相差 10 倍以上，因此这个选择不会掩盖真错误。

### 4.5 仍然拒绝的采样率

22.05 / 32 / 192 kHz、0、负数、`NaN` 一律由 `for_sample_rate` 返回 `None`
（**不静默回落、不就近取档**）。判据 `sample_rate_support_covers_the_four_derived_tables`
把四档"接受"与八种"拒绝"逐条钉住。192 kHz 是常见的母带采样率，若要支持，
按 §4.1 的公式加一档即可（列在 §10 的 N3）。

---

## 5. 不改既有语义的证据

`HD-27` 的交付要求（4）说：凡是**保留**的函数，其数值必须**逐位不变**。本线的处置：

1. **上一线的 285 条冻结位模式判据（`frozen_pre_hoist_table_is_reproduced_bit_for_bit`）
   一条未改、全部通过**。它只覆盖 `sanitize_sample` / `dbfs` / `dbfs_clamped` /
   `supersedes` / `LevelDetector` 的读数，**不碰真峰值与 LUFS**
   （上一线的 notes §7.2 注入 5 曾声称 engine 侧冻结表含"真峰值段"，
   实测那份表里只有一条 `sample_peak < 0.72 && measured > 0.999` 的**不等式**断言 ——
   它在 8× 下仍然成立，因此本线没有改动 engine 的任何一个字节）。
2. **48 kHz 的系数 `SHELF_48K` / `HIGH_PASS_48K` 是原样字面量**（一位没动），
   `KWeighting::new_48k()`（`const fn`）、`LoudnessMeter::new_48k()`、
   `LoudnessMeter` 的全部方法（`add_mono`/`add_stereo`/`mean_square`/
   `loudness_lufs`/`integrated_*`/`reset`）语义与数值不变；
   `loudness_lufs` 只是把同一表达式抽成了 `lufs_of`（**逐位等价**，判据全绿）。
3. `yeban-engine/src/level.rs`（**不是本线的文件**）一行未改，`pub use` 的名单不变；
   本机跑 `yeban-engine --no-default-features --all-targets` 全绿（§9）。
4. **被显式改写的判据（只有这两条，均已在代码注释里写明理由）**：

| 旧判据 | 新判据 | 为什么必须改写 |
| :--- | :--- | :--- |
| `four_times_oversampling_underreads_at_four_tenths_nyquist_as_documented` | `eight_times_oversampling_fixes_the_four_tenths_nyquist_underread` | `HD-26` 要求推翻 4×（D43）：旧判据断言"4× 读到 cos(π/10)"，而新实现根本没有 4× 路径 |
| `silence_padding_pulls_ungated_loudness_down` | `silence_padding_pulls_ungated_loudness_down_but_not_the_gated_one` | `HD-27` 要求补门限；旧判据把"无门限"当成唯一语义（N5 明确要求"落地时必须显式改写"） |
| `only_the_verified_sample_rate_is_accepted` | `sample_rate_support_covers_the_four_derived_tables` | `HD-27` 要求补其它采样率；旧判据要求 44.1/88.2/96 kHz 返回 `None` |

（旧判据的**意图**都没有丢：4× 的读数由测试内的对照仪器继续提供；
无门限语义由 `LoudnessMeter` 继续提供；"拒绝未核验采样率"由新判据的拒绝清单继续提供。）

---

## 6. D32：跨架构位精确怎么给预算

| 链路 | 运算类别 | 判据怎么比较 | 依据 |
| :--- | :--- | :--- | :--- |
| 真峰值核的重排/乘加（`process`） | **IEEE 精确类**（加/乘/比较/`abs`） | 判据用**位模式**（`to_bits`）比较冻结值，零容差 | IEEE-754 完全规定这些运算的舍入 |
| K 加权双二阶的递归、门限累加、窗口均方 | **IEEE 精确类** | 同上（本线的判据用**精确数值断言**与逐位比较） | 同上 |
| `true_peak_kernel_is_coherent` 的频响计算 | 超越函数类（`cos`/`sin`，**在测试里**） | 容差 2e-3（幅度）/ 1e-6（直流增益） | 这是**验证**不是热路径；容差远大于 ulp 级差异、远小于真实错误 |
| `derived_coefficients_reproduce_the_itu_48k_table` 的系数推导 | 超越函数类（`tan`/`powf`，**在测试里**） | 容差 **1e-12**（≈4500 ulp of f64） | 实测跨实现差异 ≤3.3e-16；而任何真实的推导错误（漏预扭曲、错 `f0`）都 ≫1e-3 ⇒ 判别力保留 |
| LUFS 的 `log10`（读数本身） | 超越函数类（`log10`） | 本线的 LUFS 判据用 **0.025 / 0.05 / 0.1 dB 级容差**（≈1e4～1e5 ulp），远大于 1 ulp | 与上一线一致：`log10` 不要求正确舍入 |
| 真峰值 ≥ 采样峰值、门限"两侧"语义 | **不等式**（D32 明确不放宽） | 逐位/严格不等，**零容差** | 它们是插值核与门限定义的**硬性质** |

> 因此本线**没有**任何"跨架构逐位"的过度声明：真峰值与 K 加权的**热路径**是精确类
> （在冻结架构 aarch64 上逐位；x86_64 上同样逐位，因为这些运算由 IEEE 规定），
> 而**测试里的推导/频响**按超越函数类给预算。

---

## 7. 判据清单（怎么变红）

### 7.1 规模

| 位置 | 判据条数 | 说明 |
| :--- | ---: | :--- |
| `crates/yeban-dsp/src/meter.rs` | 28 | 搬迁 14 + 真峰值 11 + 其余（数值卫生/弹道/冻结表） |
| `crates/yeban-dsp/src/loudness.rs` | 21 | 上一线 9（其中 2 条被显式改写）+ 本线 12 |
| `crates/yeban-dsp/tests/loudness_true_peak.rs` | 3 | **只走公开面**的端到端口径 |
| `yeban-dsp` 合计 | **157 单测 + 3 集成 + 4 文档测** | `run-gates.sh crate yeban-dsp` 真跑 |

### 7.2 判据 → 变红方式（本线新增 23 条 + 端到端 3 条；其中 3 条是对既有判据的显式改写）

| # | 判据 | 测试名 | 注入什么会变红 |
| :-: | :--- | :--- | :--- |
| p1 | 真峰值抓到采样点之间的过冲（fs/4 + 45° 相位） | `true_peak_sees_the_intersample_overshoot` | `process` 只返回样本峰值 |
| p2 | **4× → 8× 在 `0.4·fs` 的欠读显著改善**（同一夹具两个读数） | `eight_times_oversampling_fixes_the_four_tenths_nyquist_underread` | 回到 4× 网格（实测注入 1：读到 0.95105654） |
| p3 | **逐频点预算**（23 个频点，含相称频率 `4/9`、`5/11`…）；最坏点必须是 `4/9·fs` 且 −0.13 dB 量级；`0.25/0.40·fs` ≥ 0.999 | `true_peak_steady_state_underread_is_within_the_documented_budget` | 同上（实测注入 1：`0.4·fs` 跌到 0.95105654） |
| p4 | **真峰值 ≥ 采样峰值**（块内 + 全局，零容差） | `true_peak_preserves_the_sample_grid` | 相位 0 不再是 δ、或延迟与核里的 δ 位置不一致 |
| p5 | 只接受 8× / 16×，其它倍数响亮失败 | `true_peak_oversampling_is_either_eight_or_sixteen` | `with_oversampling` 静默回落 |
| p6 | 16× 的偶数相位与 8× **逐位相同** | `sixteen_times_contains_eight_times_phases_bit_for_bit` | 改 16× 表里任何一个偶数相位 |
| p7 | 16× 从不更差；最坏值收紧 0.069 dB、`4/9·fs` 处收紧 0.106 dB（要不要 16× 的实测依据） | `sixteen_times_tightens_the_worst_case_grid_comb_by_the_documented_margin` | 16× 表写错、或 8× 退化成 4× |
| p8 | 直流/低频透明（稳态直流 = 0.5 ± 1e-6） | `true_peak_is_transparent_at_dc_and_low_frequency` | 核不归一化 |
| p9 | 真峰值 ≥ 采样峰值 + 敌对输入有限 | `true_peak_dominates_sample_peak_and_sanitizes_hostile_input` | 删钳位 |
| p10 | 核表自洽（直流增益 1 / δ / 带内 ±2e-3 / 带边如实下垂） | `true_peak_kernel_is_coherent` | 改表里任意一个系数 |
| p11 | 真峰值确定性（含分块一致 + reset） | `true_peak_is_bit_deterministic` | 引入时间相关状态 |
| l1 | **门限常量写死**（Γa/Γr/块/跳/窗口） | `gate_thresholds_and_window_lengths_are_the_documented_values` | 改任一常量（实测注入 2） |
| l2 | **绝对门限两侧**（−75 ⇒ −∞；−65 ⇒ −65） | `the_absolute_gate_sits_at_minus_seventy_lufs` | Γa 改成 0（实测注入 2） |
| l3 | **相对门限真的在起作用**（−20+−40 ⇒ −20.065 而不是 −22.967） | `the_relative_gate_removes_the_quiet_section` | Γa 改成 0（注入 2）；Γr 改成 0 |
| l4 | **瞬时窗口 = 400 ms / 75% 重叠**（第 5 跳 = −21.245） | `momentary_window_covers_four_hundred_milliseconds_with_seventy_five_percent_overlap` | 窗口改成 3 跳或 5 跳 |
| l5 | 短时窗口 = 3 s（第 29 跳仍是 −∞，第 30 跳才给） | `short_term_window_is_three_seconds_wide` | 短时窗口改成 20 跳 |
| l6 | **门限计量确定性 + 分块不变 + 太短 ⇒ −∞** | `gated_loudness_is_bit_deterministic_and_blocking_invariant` | Γa 改成 0（注入 2）；累计顺序依赖块长 |
| l7 | **无门限 vs 门限积分的差异**（−23.010 vs −20.706） | `silence_padding_pulls_ungated_loudness_down_but_not_the_gated_one` | Γa 改成 0（注入 2） |
| l8 | 四档采样率上的 997 Hz 标定点都在 ±0.025 dB | `minus_twenty_dbfs_997hz_is_minus_twenty_lufs_at_every_supported_rate` | 某档系数被换成 48 kHz（注入 2/3 实测） |
| l9 | **系数推导复现标准正文的 48 kHz 表**，且与内置四档逐项一致 | `derived_coefficients_reproduce_the_itu_48k_table` | 改内置系数一位（实测注入 3） |
| l10 | 同一模拟信号在四档采样率下读数一致（跨度 < 0.05 dB），并显式跑"硬套 48 kHz"的反例 | `k_weighting_is_rate_consistent_on_a_wideband_fixture` | 换错某档系数（实测注入 3：偏 0.822 dB） |
| l11 | 采样率支持是显式枚举（4 档接受 + 8 种拒绝） | `sample_rate_support_covers_the_four_derived_tables` | 让未核验的采样率也返回 `Some` |
| l13 | **单声道口径 = 单通道能量**（`add_mono(x)` ≡ `add_stereo(x, 静音)` 逐位；`integrated_mono` 同） | `gated_loudness_mono_matches_one_silent_channel` | 单声道喂两遍（+3.01 dB）—— 实测注入 4 |
| l12 | 上一线 9 条判据（BS.1770 标定点、−23.01、6 dB/半、直流抑制、−∞ 不是 NaN、敌对输入、确定性、reset、解析增益） | 见 `loudness.rs` 的 `tests` | 逐个对应（**数值一位未改**） |
| e1–e3 | **公开面**端到端口径：文档里的数字（−23.0103 / −20.7059 / 真峰值 0.1 / −∞）与常量契约 | `crates/yeban-dsp/tests/loudness_true_peak.rs` | 公开面接错（例如 `integrated_stereo` 没走门限） |

> 与上一线的 `t1–t6` / `l1–l10` 编号不冲突：本表是本线**新增/改写**部分的清单；
> 上一线那份完整清单在 `dsp-level-notes.md` §6.2，其中 t5 与 l5/l9 已被本线显式替换（§5）。

---

## 8. 注入 → 变红 → 还原（4 条实测）

方法：改一处源码 ⇒ `cargo-local.sh test -p yeban-dsp` ⇒ 记录红点 ⇒
从注入前快照 `cp` 还原 ⇒ 重跑确认全绿。**注入标记一律写 `INJECT-n` 注释并随还原一起消失**
（还原后 `grep -rn "INJECT" crates/` = 0 命中）。

| # | 注入 | 实测红点 | 关键读数 |
| :-: | :--- | :--- | :--- |
| 1 | 真峰值相位循环改成 `step_by(2)`（**等价于回到 4× 网格**） | **3 FAILED**：`eight_times_oversampling_fixes_...`、`true_peak_steady_state_underread_...`、`sixteen_times_tightens_the_worst_case_...` | `8× 在 f=0.4·fs 应读到 1.0 附近, 实际 0.95105654 (-0.4359 dBFS)`；`16× 的最坏频点异常`（`step_by(2)` 后 16× 的实际网格变成 8×，最坏点从 `6/13` 挪走） |
| 2 | `ABSOLUTE_GATE_LUFS` `−70.0 → 0.0`（等于"没有块能过绝对门限"） | **7 FAILED**：`gate_thresholds_...`、`minus_twenty_dbfs_997hz_...`、`silence_padding_...`、`the_absolute_gate_...`、`gated_loudness_is_bit_deterministic_...`、`k_weighting_is_rate_consistent_...`、`the_relative_gate_...` | `门限积分应读到 −20.706, 实际 -inf`；`高于 Γa 的信号应读到 -65, 实际 -inf`；`44100 Hz 的门限读数 -inf 偏离 −20.0` |
| 4 | `push_frame(y, None)` → `Some(y)`（单声道被当成「同一个信号喂两个通道」） | **1 FAILED**：`gated_loudness_mono_matches_one_silent_channel` | `单声道流式读数必须与'另一路静音'的立体声读数逐位相同` —— 这是本线**代码审查**发现并修掉的真实语义缺陷（单声道会平白多 3.01 dB） |
| 3 | `SHELF_96K` + `HIGH_PASS_96K` 换成 48 kHz 的系数 | **3 FAILED**：`derived_coefficients_reproduce_the_itu_48k_table`、`minus_twenty_dbfs_997hz_...`、`k_weighting_is_rate_consistent_...` | `96000 Hz 的 shelf[0]: 内置 1.53512485958697 ≠ 推导 1.5597142289757966`；`96000 Hz 的无门限读数 -20.64968 偏离 −20.0`；`96000 Hz 的宽带读数是 -18.537706, 与 48 kHz 的 -17.715538 相差过大` |

**还原判据**：注入后 `cp` 回快照；`grep -rn "INJECT" crates/` = 0 命中；
重跑 `cargo-local.sh test -p yeban-dsp` = **157 + 3 + 4 全绿**；
`clippy --all-targets -D warnings` 0 告警。

> **诚实交代**：注入 2 之所以红了 7 条（比"门限语义"的三条多），是因为
> `k_weighting_is_rate_consistent_...` 与 `minus_twenty_dbfs_...` 走的是**门限积分**
> 路径，Γa = 0 会让它们的读数直接变成 −∞。这不是判据设计缺陷，而是"门限是计量链的
> 必经环节"的必然结果 —— 但读数上看，它确实让"系数一致性"那条判据在门限坏掉时
> 也变红。若将来要把两类失效分开，可给系数一致性判据加一条"用无门限计量器"的孪生判据。

---

## 9. 本机真跑 vs 交给 CI（**严格区分**）

### 9.1 本机（M2 / aarch64）**真的跑过**的

| 命令 | 结果 |
| :--- | :--- |
| `bash scripts/dev/cargo-local.sh test -p yeban-dsp` | ✅ **157 passed** + 集成 **3 passed** + 文档测 **4 passed**（`INJECT` 清空后重跑确认） |
| `bash scripts/dev/cargo-local.sh clippy -p yeban-dsp --all-targets -- -D warnings` | ✅ 0 告警 |
| `bash scripts/dev/cargo-local.sh fmt --all --check` | ✅ |
| `bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --all-targets` | ✅ **全绿**：101 单测 + 6 个集成/契约目标（`limiter_contract` 7、`mix_render` 6、`steal_fade` 4、`synth_filter` 7、`synth_render` 9，外加 `meter_rt_contract` / `rt_zero_alloc` / `synth_rt_zero_alloc` 三个 `harness=false` 目标）；电平/真峰值经 engine 的 `pub use` 路径仍然可用 |
| `bash scripts/gates/run-gates.sh crate yeban-dsp` | ✅ 通过（**真编译真跑**：`clippy[yeban-dsp] ok` + `test[yeban-dsp]` 157+3+4，**不是 SKIP**） |
| `bash scripts/gates/run-gates.sh light` | ✅ fmt + 13 条守卫 + 文档门禁 + 许可清单（见 §9.3） |
| **4 条**注入 → 变红 → 还原 | ✅ 见 §8 |

### 9.2 交给 CI 的（本机未跑）

- `cargo clippy/test --workspace` 的**默认 feature** 形态（含 Slint / cpal / symphonia）；
- `cargo deny check` 全量（本机只跑了 light 档的许可清单对账）；
- **跨架构（x86_64 Linux runner）的数值复现**：真峰值核与 K 加权热路径是 IEEE 精确类
  （理论上逐位），但这条结论**只有 CI 能判定** —— 本线**没有**在 x86_64 上跑过；
- `yeban-engine` 的默认 feature（10 条 `device` 门控判据）与其它 crate 的回归。

### 9.3 ⚠ 本机验证的边界（不要过度声明）

- 本机是 **aarch64**：所有读数都是这一架构的实测值；
- CI（x86_64）会重新跑同一批判据。按 D32 的分策，真峰值/K 加权（IEEE 精确类）应当逐位相同；
  若 CI 抓到差异，处置方式与上一线一致：**按运算类别**判断该不该给预算，而不是整体放宽容差；
- 本机**没有**跑 `--workspace`（AGENTS.md §5 硬性禁止），因此"全仓绿"这句话本线不会说。

---

## 10. pending / needs

| ID | 内容 | 归属 |
| :--- | :--- | :--- |
| L1 | **`dsp-level-notes.md` 的 N3 / N5 / N6 关闭记**：本线已落地，那份台账属于别的线的所有者，需要集成者补记（本线不改别人的文件） | 集成者 |
| L2 | **`docs/ledger/gate-status.md`**：`MUST-GATE-001` 的证据行可补一句"电平/真峰值口径仍在 `yeban-dsp`，engine 侧零分配窗口不变"（状态不变） | 集成者 |
| N1 | **真峰值的下一步**：若母带链要更紧的高频真峰值（`f > 0.44·fs`），正确的方向是**加长 `T`**（48/64 抽头）或上 **32×**（只对网格梳有效），而**不是**加相位数 —— `L = 8 → 16` 在 16 个频点上实测**零收益**（§2.5）。加长 `T` 会同时抬高冷启动瞬态（§2.4），需要配套口径 | 后续母带/计量线 |
| N2 | **流式 integrated 响度**（若 UI 要"边放边看"）：需要 (a) 有界直方图（量化误差要写进口径）或 (b) 由调用方持有块列表。当前实现是**两遍精确**的离线形式（§3.4） | 后续 UI/计量线 |
| N3 | **192 kHz（及其它采样率）的 K 加权系数**：按 §4.1 的公式加一档即可，需要补一条与 48 kHz 表同级的验证判据 | 后续线 |
| N4 | **LRA（响度范围）**：BS.1770-4/EBU R128 的 LRA 需要按 −20 LU 相对门限对 3 s 短时值做直方图（1 s 步进）。本线实现了 3 s 短时窗口，**没有**做 LRA | 后续母带线 |
| N5 | **环绕权重与 LFE**：>2 声道与 `G = 1.41` 的环绕权重、LFE 排除仍未实现（上一线 N7 仍开放） | 混音台切片 |
| N6 | **真峰值模式接进限制器**：[`engine-mix-notes.md`](engine-mix-notes.md) 的 N3 —— 母线限制器仍按**样本峰值**工作。本线只改进了（计量用）真峰值检测器；把它接到限制器需要 4/8× 过采样 + 前瞻延迟预算（PDC 回填），属于母带切片 | 母带线 |
| N7 | **`LoudnessMeter`（无门限）与 `GatedLoudness` 的取舍**：目前两者并存（前者逐位不变的合约、后者是 BS.1770 的正解）。若将来要"只留一个"，应当在 UI/导出的消费点统一后按 D43 删掉无门限那一半 | 架构（需要裁决） |

---

## 11. 复用来源（Reuse provenance）

- `meter.rs` 的电平口径与 `loudness.rs` 的框架**来自本仓库**（同一作者、同一
  GPL-3.0-only 许可，见 `dsp-level-notes.md`），不是第三方代码；
- **真峰值 8×/16× 核表是本线按 §2.1 的公式生成的**（Kaiser 窗 sinc + 逐相位归一化 +
  相位 0 强制 δ），生成脚本与逐项复现记录见 §2.1；未引入任何新依赖；
- **四个采样率的 K 加权系数是本线按 §4.1 的解析原型推导的**，推导口径写在代码注释与
  §4.1 里；48 kHz 一档与 BS.1770-4 正文的表 1/表 2 一致（1–2 ulp of f64）；
- 未引入任何新依赖（`crates/yeban-dsp/Cargo.toml` 的 `[dependencies]` 仍为空），
  `Cargo.lock` 与 `docs/ledger/dependency-licenses.md` **不需要改动**；
- 因此**不新增** `THIRD_PARTY_LICENSES.md` 条目、不触发 `deny.toml` 白名单。

---

## 12. 修改文件清单与净行数

| 文件 | 变化 | 说明 |
| :--- | :--- | :--- |
| [`crates/yeban-dsp/src/meter.rs`](../../crates/yeban-dsp/src/meter.rs) | 改写 | 真峰值 4× → 8×（+16× 可选）：核表替换、API 形状、11 条判据 |
| [`crates/yeban-dsp/src/loudness.rs`](../../crates/yeban-dsp/src/loudness.rs) | 改写 + 新增 | 四档 K 加权系数与推导记录、`GatedLoudness`（门限积分 + 瞬时/短时）、12 条判据、2 条判据显式改写 |
| [`crates/yeban-dsp/tests/loudness_true_peak.rs`](../../crates/yeban-dsp/tests/loudness_true_peak.rs) | **新增** | 公开面端到端口径（3 条） |
| `docs/ledger/dsp-loudness-notes.md` | **新增** | 本台账 |
| `crates/yeban-dsp/Cargo.toml` | **未改** | 零新增依赖 |
| `Cargo.toml` / `Cargo.lock` / `.github/**` / `scripts/**` / `deny.toml` / `docs/adr/**` / 其它 `crates/**` | **未改** | 纪律要求（本线不碰共享文件与别人的地盘） |

净行数（`git show --numstat HEAD`，代码提交 `0e074bf`）：

| 文件 | 净增 | 净删 |
| :--- | ---: | ---: |
| `crates/yeban-dsp/src/meter.rs` | +761 | −107 |
| `crates/yeban-dsp/src/loudness.rs` | +1029 | −64 |
| `crates/yeban-dsp/src/lib.rs` | +2 | −2 |
| `crates/yeban-dsp/tests/loudness_true_peak.rs` | +113 | 0 |
| **合计** | **+1905** | **−173** |
