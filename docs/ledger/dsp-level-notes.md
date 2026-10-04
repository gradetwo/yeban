# `yeban-dsp` 电平资产台账：上移、口径、逐位不变证据与新增能力（`dsp-level` 工作线）

- **台账类型**：搬迁对照 / 口径 / 判据清单 / 实测证据 / 未决项（**不是规范**）
- **记录时刻**：2026-10-05（本机 Apple M2，workspace-write 沙箱 + 受限 rustup）
- **工作线**：`line/dsp-level`（worktree `yeban/.worktrees/dsp-level`，起点 main `b014e8f`）
- **所有者目录**：`crates/yeban-dsp/**`、`crates/yeban-engine/**`（本台账是唯一新增的文档文件）
- **落地范围**：
  - 新增 `crates/yeban-dsp/src/meter.rs`（电平口径上移 + 4× 真峰值）
  - 新增 `crates/yeban-dsp/src/loudness.rs`（K 加权 + 无门限 LUFS 最小子集）
  - 改写 `crates/yeban-engine/src/level.rs` 为**纯 `pub use`**
  - `crates/yeban-engine/Cargo.toml` 增加 `yeban-dsp` 依赖（`engine → dsp`，无环）
  - `docs/ledger/dependency-licenses.md` 由 `license_inventory.py` 重新生成（只有 `Cargo.lock` 哈希行变化）
- **规范来源**：`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` 的 `ARCH-UI-002`、
  `ARCH-RT-001`、`ARCH-DET-001`、`ARCH-DSP-001`；
  `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` 的 `ROAD-M2-007`、`ROAD-M2-008`；
  `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md` D19（feature 切分）、
  D21（内部 path 依赖豁免通配）；`MUST-GATE-001`

> 本文件回答：**哪个函数搬去了哪里、engine 还剩什么**、**数值语义为什么逐位不变（证据在哪）**、
> **新加的 DSP 能力口径是什么**、**怎么证明 engine 没有第二份实现**、
> **每条判据怎么变红（5 条注入实测）**、**本机真跑 vs 交给 CI 的严格区分**、
> **明确没做什么**。

---

## 0. 这一线做了什么（一句话）

`yeban-engine/src/level.rs`（653 行、上一轮的 `line/engine-meters` 落地）里的**纯计算**
整体搬到 `yeban-dsp`，engine 侧退化为一行 `pub use`；同时给 DSP 侧补上两件与电平/计量
直接相关、且本机零依赖可验证的能力：**4× 过采样真峰值**与**K 加权无门限 LUFS 最小子集**。

这条线的意义是**归属**：电平口径从"引擎内部实现"变成"可复用 DSP 资产"——
混音台、母带、导出都可以 `use yeban_dsp::meter::LevelDetector`，而不必拖入 cpal 声卡栈。

**关闭的 need**：`engine-meters-notes.md` 的 **MN1**（"把纯计算上移到 `yeban-dsp`"）由本线落地；
本线**没有**改动那份台账（它属于别的线的所有者），MN1 的关闭需要集成者在其上补记。
MN2/MN4/MN5 仍开放（见 §8）。

---

## 1. 搬迁对照表

### 1.1 逐项去向（实现 → 新家）

| 原 `yeban-engine/src/level.rs` | 现位置 | engine 侧 |
| :--- | :--- | :--- |
| `SILENCE_FLOOR_DBFS` / `MAX_LINEAR_MAGNITUDE` | `yeban_dsp::meter` | `pub use` |
| `DEFAULT_PEAK_DECAY_DB_PER_SEC` / `DEFAULT_RMS_TIME_CONSTANT_SEC` / `DEFAULT_QUANTA_PER_SECOND` | `yeban_dsp::meter` | `pub use` |
| `sanitize_sample` | `yeban_dsp::meter` | `pub use` |
| `dbfs` / `dbfs_clamped` | `yeban_dsp::meter` | `pub use` |
| `supersedes` | `yeban_dsp::meter` | `pub use` |
| `LevelReading`（含 `silence`/`is_sane`/三个 dBFS 访问器） | `yeban_dsp::meter` | `pub use` |
| `LevelDetector`（`new`/`silent`/`with_ballistics`/`set_ballistics`/`set_quanta_per_second`/`reset`/`peak_hold`/`mean_square`/`analyze`/`analyze_stereo`/`commit`） | `yeban_dsp::meter` | `pub use` |
| `mean_of_squares`（私有） | `yeban_dsp::meter`（私有） | 不导出 |
| 14 条单元判据 | `yeban_dsp::meter::tests` | **同一份判据也留在 engine**（见 §1.2） |

### 1.2 engine 侧现在还剩什么

| 模块 | 内容 | 归属理由 |
| :--- | :--- | :--- |
| `src/level.rs` | **只有** `pub use yeban_dsp::meter::{...}`（+ 判据） | 保留路径，让"搬家"对调用方透明 |
| `src/meter.rs` | `MeterFrame` / `MeterBank` / `MeterPublisher` / `MeterCollector` / `MeterBoard` / `meter_channel` | **实时侧状态 + 发布**：`rtrb` SPSC、每节点定长槽、LRU 淘汰、UI 60Hz 抽干。依赖 `rtrb` 与 `yeban_model::EntityId`，**不是**纯数学 |
| `src/rt.rs` | `EngineRuntime::process_quantum`：每量子**恰好一次**批量发布 | 实时侧调度契约 |
| `tests/rt_zero_alloc.rs`、`tests/meter_rt_contract.rs` | 零分配窗口与每量子一次发布的运行期判据 | 实时侧契约 |

**判断口径**：凡是"给定样本 → 给定读数、无跨调用外部状态、不引用队列/线程/模型类型"的，
都判为纯计算并上移；凡是"持有 SPSC 端点、持有跨量子节点状态、做容量淘汰/发布次数记账"的，
留在 engine。`LevelDetector` 虽然有弹道状态，但它的状态是**纯数学的弹道**（与线程/队列无关），
因此上移；`MeterBank` 的槽位、淘汰与 `MeterFrame` 的 `EntityId` 是实时侧资产，因此留下。

### 1.3 调用方改动量

**零**。`crate::level::*`、`yeban_engine::level::*` 与所有既有判据一行未改；
`src/meter.rs`、`src/rt.rs`、`tests/meter_rt_contract.rs` 全部按原样编译通过。

---

## 2. 口径表（搬迁后不变；判据钉住的就是这张表）

| 量 | 定义 | 单位 / 备注 |
| :--- | :--- | :--- |
| 输入钳位 | `NaN → 0.0`；`±∞` 与越界有限值 `→ ±16.0` | 4× 满量程 = **+24.08 dBFS** |
| 峰值 `peak` | 本量子内 `max abs(x)`（钳位后） | 线性，`1.0` = 0 dBFS |
| 峰值保持 `peak_hold` | `max(peak, peak_hold × release)` | 默认释放 **20 dB/s** |
| RMS `rms` | 本量子 `sqrt(mean(x²))`，不跨量子 | 满幅方波 ⇒ **恰好 1.0** |
| 平滑 RMS `rms_smoothed` | 对**均方**做一阶低通：`ms ← c·ms + (1−c)·mean(x²)`，再开方 | 默认 **τ = 300 ms** |
| dBFS | `20·log10(幅度)`；`幅度 ≤ 0`（含 `NaN`）⇒ **负无穷** | |
| 静音下限 | `SILENCE_FLOOR_DBFS = −120.0` | UI 柱高用有限值 |
| 取最新 | `supersedes(candidate, held) = candidate >= held` | `>=` 而非 `>`（同量子重复投递幂等） |
| 母线口径 | **立体声联动**：峰值取 L/R 最大绝对值；均方 = `(Σl² + Σr²)/(2n)` | 长度不等按较短者 |
| **真峰值**（新） | 4× 多相插值后的 `max abs`（含原始样本） | [`meter.rs`](../../crates/yeban-dsp/src/meter.rs) 的 `TruePeakDetector` |
| **LUFS**（新） | `−0.691 + 10·log10(z)`，`z` = K 加权双声道能量均方 | **无门限**；只支持 48 kHz |

### 2.1 弹道系数的折算（不变）

```text
quanta_per_second = sample_rate / block_frames      // 规范默认 48000 / 128 = 375
peak_release      = 10^(−decay_dB_per_s / 20 / qps) // 20 dB/s @375 ⇒ 0.993880
rms_coeff         = exp(−1 / (τ · qps))             // τ = 300 ms @375 ⇒ 0.991151
```

---

## 3. 逐位不变的证据（"搬家不是改行为"）

### 3.1 方法

搬迁**前**（main `b014e8f` + 未改动的 `yeban-engine/src/level.rs`）跑一段**确定性输入脚本**，
用 `f32::to_bits()` 打印每一个中间读数与最终读数的位模式；搬迁后同一段脚本必须给出**同一位模式**。

- 脚本覆盖：钳位 20 例、dBFS 15 例、`dbfs_clamped` 5 例、`supersedes` 6 例、
  正弦/方波/静音/空块/敌对输入（`NaN`/`±∞`/1e30）/375 量子释放/τ 阶跃/立体声联动/
  8 个量子的确定性 xorshift 序列/无弹道配置/非法弹道参数/不释放/750 量子/`reset`。
- 冻结条目：**285 条** `(名字, u32 位模式)`。
- 判据：`yeban_dsp::meter::tests::frozen_pre_hoist_table_is_reproduced_bit_for_bit`
  （dsp 侧全量 285 条）与
  `yeban_engine::level::tests::frozen_level_table_through_the_engine_path`
  （engine 侧经再导出路径的关键子集）。

**为什么用位模式而不是容差**：容差会掩盖"顺手改了行为"。把 20 dB/s 改成 10 dB/s、
把钳位删掉、把 `NaN` 原样返回，都会在这张表上逐条变红（§6 的注入 1–3 实测）。

#### 3.1.1 两类比较（跨架构的诚实处理）

这张表是 **aarch64 本机**冻结的，而 CI 是 **x86_64**。第一版判据要求 285 条**全部**逐位相同，
**CI 实测抓到 1 条 1 ulp 差异**（run `37238420778`，`rust (workspace 全量)` → `test --workspace`）：
`dbfs(√½)`，aarch64 `0xc040a8c2`（−3.0103002）vs x86_64 `0xc040a8c3`（−3.0103004）。
原因是 `f32::log10` 这类**超越函数不要求正确舍入**，两个架构的标准库实现可以差 1 ulp。

因此判据按**运算类别**分开（不是简单放宽容差）：

| 类别 | 判据 | 依据 |
| :--- | :--- | :--- |
| **IEEE 精确类**：`san.*`、`sup.*`、`const.*`，以及读数的 `.peak`（`abs`/`max`）、`.rms`（乘加/除/`sqrt`）、`.is_sane` | **跨架构逐位相同，零容差** | 这些运算 IEEE-754 要求**正确舍入**，任何架构都必须给出同一位模式 |
| **超越函数类**：`db.*`、`dbc.*`、`.peak_hold*`、`.rms_smoothed`、`.`*`_dbfs`、`release.one_quantum`、`smooth.*`、`decay.*`、`reset.*` | 非冻结架构给 **4096 ulp** 预算；**冻结架构（aarch64）上仍然逐位相同** | `log10`/`exp`/`powf` 不要求正确舍入；误差沿弹道递归累积上界 ≈ `750 · 2^-24 ≈ 4.5e-5`（≈ 750 ulp），4096 ulp ≈ 2.4e-4 相对留 5× 余量 |

**预算足够紧吗**：最小的一次真实漂移是注入 1（20 → 10 dB/s 改动 `peak_release` 0.3%
≈ **50000 ulp**），仍然远超 4096 ulp 的预算 ⇒ 行为漂移照样变红（§7.2 实测重跑确认）。
换句话说：**同类内逐位、跨架构限 ulp、行为漂移照抓**。

> 诚实边界：**"285 条逐位相同"这个结论只在 aarch64 本机上被验证过**。
> CI（x86_64）验证的是"IEEE 精确类逐位 + 超越函数类 ≤ 4096 ulp"。
> 不得把后者说成跨架构逐位 —— 见 §8 的 N4（现已由本设计**收敛**，不再是悬空风险）。

### 3.2 关键数值表（搬迁前实测，搬迁后仍逐位相同）

| 输入 / 场景 | 量 | 位模式 | 十进制 |
| :--- | :--- | :--- | :--- |
| `sanitize_sample(NaN)` | — | `0x00000000` | `0.0` |
| `sanitize_sample(+∞)` | — | `0x41800000` | `16.0` |
| `sanitize_sample(−∞)` | — | `0xc1800000` | `-16.0` |
| `sanitize_sample(1e30)` | — | `0x41800000` | `16.0` |
| `sanitize_sample(−0.0)` | — | `0x80000000` | `-0.0` |
| `dbfs(1.0)` | — | `0x00000000` | `0.0` |
| `dbfs(0.5)` | — | `0xc0c0a8c2` | `-6.0206` |
| `dbfs(0.0)` / `dbfs(NaN)` | — | `0xff800000` | `-∞` |
| `dbfs_clamped(1e-9, −120)` | — | `0xc2f00000` | `-120.0` |
| 满幅正弦 10 周期 / 4800 点 | `peak` | `0x3f800000` | `1.0` |
| 同上 | `rms` | `0x3f3504f3` | `0.70710677` |
| 同上 | `rms_smoothed` | `0x3d883b02` | `0.0664555` |
| 同上 | `rms_dbfs` | `0xc1bc5432` | `-23.541` |
| `[1.0;64]` 之后 1 个静音量子 | `peak_hold` | `0x3f7e6ed4` | `0.993880`（= 每量子释放乘子） |
| `[1.0;64]` 首量子 | `mean_square` | `0x3c10fd80` | `0.008849`（= `1 − rms_coeff`） |
| 满幅方波 512 点 | `rms` | `0x3f800000` | `1.0`（解析值） |

### 3.3 交叉核对

- dsp 侧 285 条**全部**命中（aarch64：逐位；见 §3.1.1 的两类规则）；
- engine 侧经 `yeban_engine::level` 再导出的关键子集命中；
- 搬迁前后 engine 的结构性读数**完全一致**：
  `[meter-rt] S1 汇总: quanta=10242 publishes=10242 frames=40968 capacity_drops=0`
  （与 `engine-meters-notes.md` §5.3 的上一轮实测逐字相同）；
  105 行 `allocations=0 deallocations=0`；`MUST-GATE-001 ok`。

---

## 4. 新增的 DSP 能力

### 4.1 4× 过采样真峰值 `yeban_dsp::meter::TruePeakDetector`

- **规范锚点**：`ARCH-UI-002` 的"真峰值"字面要求；`engine-meters-notes.md` §7 第 3 条
  （上一轮明确记为"未实现"）。
- **口径**：4 相 × 16 抽头多相插值核（总原型 64 抽头）。生成方式
  `sinc(m − 8 + phase/4) × Kaiser(β = 8, L = 16)`，**逐相位归一化到直流增益 1**。
  `phase 0` 恰好退化为 `δ(m − 8)` ⇒ 过采样输出天然含原始样本 ⇒ **真峰值 ≥ 采样峰值**恒成立。
  系数按最短往返表示写成 `f32`（与 `crate::oversample` 同一纪律）。
- **诚实边界**：4× 是**估计**。`f = 0.4·fs` 的满幅正弦，连续峰值在 `t = 5/8`（不在 1/4 网格上），
  4× 读到 `cos(π/10) = 0.9511`（−0.44 dB）。这是方案的**固有**欠读，
  判据 `four_times_oversampling_underreads_at_four_tenths_nyquist_as_documented` **把这个数字钉住**，
  免得后人误以为是 bug 或误以为 4× 是精确真峰值。8×/16× 属于后续切片（§8）。
- **判据**：`true_peak_sees_the_intersample_overshoot`（`fs/4` + 45° 相位：采样峰值 0.7071 ⇒ 真峰值 > 0.999）、
  `true_peak_is_transparent_at_dc_and_low_frequency`（稳态直流逐位透明）、
  `true_peak_dominates_sample_peak_and_sanitizes_hostile_input`、
  `true_peak_kernel_is_coherent`（核表自洽：直流增益 1、相位 0 是 delta、Nyquist 以下幅度 ≈ 1）、
  `true_peak_is_bit_deterministic`。
- **零分配 / 实时安全**：状态是 `[f32; 16] + f32`，`process` 只有乘加与移位 [ARCH-RT-001]。

### 4.2 K 加权无门限积分响度 `yeban_dsp::loudness`

- **规范锚点**：`ARCH-UI-002` 的响度/电平面；`engine-meters-notes.md` §7 第 3 条。
- **口径**：BS.1770-4 的两级 K 加权（高架 + RLB 高通，**48 kHz 系数**）→
  `z = (Σy_L² + Σy_R²)/frames` → `LUFS = −0.691 + 10·log10(z)`。
  标定点：**997 Hz、−20 dBFS 的双声道正弦 = −20.0 LUFS**（判据容差 0.05 dB）。
- **明说没做的**（写进模块文档，避免误读为完整 BS.1770）：**没有门限**
  （无 −70 LUFS 绝对门限、无 −10 LU 相对门限、无 400 ms 重叠块流程）、
  无 3 s 短时/400 ms 瞬时窗口、无真峰值模式、无环绕权重与 LFE、**只支持 48 kHz**
  （其他采样率由 `KWeighting::for_sample_rate` 明确 `None`，不硬套 48 kHz 系数）。
- **判据**：`minus_twenty_dbfs_997hz_stereo_is_minus_twenty_lufs`、
  `one_channel_only_is_three_db_quieter`（−23.01 LUFS，钉住"两通道能量相加"）、
  `loudness_tracks_amplitude_by_six_db_per_halving`、
  `dc_is_rejected_by_the_k_weighting_high_pass`（稳态 −100 LUFS 以下）、
  `silence_padding_pulls_ungated_loudness_down`（**把"无门限"这一语义显式钉住**，
  将来做门限时必须显式改写这条判据）、
  `silence_and_empty_input_are_negative_infinity_never_nan`、
  `hostile_samples_do_not_poison_the_reading`、`loudness_is_bit_deterministic`、
  `reset_restores_the_initial_state`、`only_the_verified_sample_rate_is_accepted`、
  `biquad_gains_match_the_itu_analytic_values`（997 Hz 级联增益 = +0.691 dB，即标定常数来源）。
- **数值纪律**：滤波器状态与累加器用 `f64`（RLB 高通极点 `a2 = 0.99007` 很靠近单位圆，
  `f32` 状态会漂移）；非有限样本按 `0.0` 清洗，一个 `NaN` 不会杀死计量器。

### 4.3 没有做的（明确不做，拒绝凑数）

`ARCH-DSP-002`（多相重采样）与 `ARCH-DSP-004`（弹性拉伸）需要 `rubato` / `signalsmith-stretch`
**重依赖**，本机禁止编译、且属于依赖图裁决 ⇒ 不碰（§8 的 pending）。
`ARCH-DSP-001` 的去爆音（5 ms 参数平滑、升余弦淡出）**已经在上一轮落地**
（`crate::smoothing` / `crate::loop_window`），本线不重复实现。

---

## 5. engine 侧"没有第二份实现"的机械证据

三层，全部可机械复跑：

1. **源码级（grep 判据，写在判据里）**：
   `yeban_engine::level::tests::engine_level_module_has_no_second_implementation`
   用 `include_str!("level.rs")` 读出自身源码，断言其中**不出现**实现记号
   （`struct {LevelDetector,LevelReading,TruePeakDetector} {`、`impl {…}`、
   `fn {sanitize_sample,dbfs,dbfs_clamped,supersedes,mean_of_squares}(`、`const TRUE_PEAK_KERNEL`），
   并断言存在 `pub use yeban_dsp::meter::`。记号用 `concat!` 拼接，避免判据命中自己。
   实测：`grep -rn INJECT crates/` = 0 命中（L24）。
2. **类型归属（编译期）**：`engine_level_is_literally_the_dsp_type` 把
   `yeban_engine::level::LevelDetector` 赋给 `yeban_dsp::meter::LevelDetector` 变量，
   并用 `core::ptr::fn_addr_eq` 比较 4 个函数指针的**地址**。若 engine 私藏同构实现，
   类型赋值与地址比较双双失败（注入 4 实测）。
3. **行为级**：285 条冻结位模式 + 77 条 engine 单测；任何"悄悄留下第二份"都会在
   `frozen_level_table_through_the_engine_path` 上现形。

---

## 6. 判据清单（怎么变红）

### 6.1 新增/搬迁的判据规模

| 位置 | `#[test]` 条数 |
| :--- | ---: |
| `crates/yeban-dsp/src/meter.rs` | 23（14 条搬迁 + 9 条新增） |
| `crates/yeban-dsp/src/loudness.rs` | 11（全部新增） |
| `crates/yeban-engine/src/level.rs` | 17（14 条搬迁 + 3 条归属/冻结） |
| 既有 `meter.rs` / `rt.rs` / `snapshot.rs` / `ring.rs` 等 | 不变 |

### 6.2 判据 → 变红方式

| # | 判据 | 测试名 | 注入什么会变红 |
| :-: | :--- | :--- | :--- |
| d1 | 搬迁前后 285 条读数一致（IEEE 精确类逐位；超越函数类 ≤ 4096 ulp，冻结架构上逐位） | `frozen_pre_hoist_table_is_reproduced_bit_for_bit`（dsp） | 任何行为漂移（实测注入 1/2/3/5） |
| d2 | engine 路径下关键子集一致（同一分策） | `frozen_level_table_through_the_engine_path`（engine） | 同上（实测注入 1/2/3/4/5） |
| d3 | engine 只有再导出、无第二份实现 | `engine_level_module_has_no_second_implementation` | 在 engine 加回任何实现记号（实测注入 4） |
| d4 | engine 与 dsp **是同一个类型/函数** | `engine_level_is_literally_the_dsp_type` | 同上（实测注入 4） |
| d5 | 峰值保持 1 秒恰好回落 **20 dB**（375 量子/s，写死） | `peak_hold_decays_by_the_documented_rate` | 释放率改成 10 dB/s（实测注入 1） |
| d6 | 平滑 RMS 在 τ=300 ms 达到 `1−1/e`、长期收敛 | `smoothed_rms_follows_the_documented_time_constant` | `rms_coeff = 0`；τ 改成 0.15（常量写死可抓） |
| d7 | 满幅方波 RMS **恰好 1.0** | `full_scale_square_wave_rms_is_exactly_one` | RMS 不除以 n / 用 `min` |
| d8 | dBFS 定义与边界（`≤0 ⇒ −∞`、下限 −120、`NaN ⇒ −∞`） | `dbfs_is_the_exact_inverse_of_the_documented_definition` | 漏掉 `20·log10`；把 `NaN` 传下去 |
| d9 | `NaN`/`±∞`/1e30 钳位且读数有限 | `sanitize_clamps_and_never_returns_nan`、`nan_and_infinity_inputs_never_produce_nan_levels` | 删钳位（实测注入 2）；`NaN` 原样返回（实测注入 3） |
| d10 | 满幅/半幅正弦峰值 dBFS | `full_scale_sine_peaks_at_zero_dbfs`、`half_scale_sine_is_minus_six_dbfs` | 峰值取 `sum`；漏掉 `20·` |
| d11 | 幅度单调 | `levels_are_monotonic_in_amplitude` | `max` 改 `min` |
| d12 | 立体声联动（单声道满幅 = 0 dBFS；均方按两声道平均） | `stereo_reading_is_channel_linked` | 只看左声道 |
| d13 | `supersedes` 取最新（同量子/更新接受，更旧拒绝） | `supersedes_accepts_equal_and_newer_but_not_older` | 改成 `>` 或恒真 |
| d14 | 空块/静音仍让保持值衰减、RMS 有限 | `empty_block_still_decays_hold_and_keeps_finite_rms`、`zero_input_is_negative_infinity_or_floor_never_nan` | 空块 `0/0`；保持值不衰减 |
| d15 | 无弹道配置 / 非法弹道参数 / reset 语义 | `silent_detector_has_no_ballistics`、`ballistics_reject_invalid_parameters_without_nan`、`reset_clears_state_but_keeps_ballistics` | 非法参数不退回默认 |
| t1 | **真峰值抓到采样点之间的过冲** | `true_peak_sees_the_intersample_overshoot` | `process` 只返回样本峰值（实测注入 5） |
| t2 | 真峰值 ≥ 采样峰值、敌对输入有限 | `true_peak_dominates_sample_peak_and_sanitizes_hostile_input` | 同上（实测注入 5） |
| t3 | 真峰值核表自洽（直流增益 1 / delta / 频响） | `true_peak_kernel_is_coherent` | 改表里任意一个系数 |
| t4 | 直流/低频稳态透明 | `true_peak_is_transparent_at_dc_and_low_frequency` | 核不归一化 |
| t5 | 4× 在 `0.4·fs` 的固有欠读**如实** | `four_times_oversampling_underreads_at_four_tenths_nyquist_as_documented` | 宣称 4× 精确 ⇒ 该判据会指出欠读 |
| t6 | 真峰值确定性 | `true_peak_is_bit_deterministic` | 引入无序/时间相关状态 |
| l1 | **BS.1770 标定点**：997 Hz −20 dBFS 双声道 = −20.0 LUFS | `minus_twenty_dbfs_997hz_stereo_is_minus_twenty_lufs` | 偏置改 0；漏高通级；丢掉一个通道 |
| l2 | 单通道 ⇒ −23.01 LUFS（能量相加） | `one_channel_only_is_three_db_quieter` | 通道能量取平均而不是求和 |
| l3 | 幅度减半 ⇒ −6.0206 dB | `loudness_tracks_amplitude_by_six_db_per_halving` | 用幅度而不是均方取对数 |
| l4 | 高通级拒绝稳态直流 | `dc_is_rejected_by_the_k_weighting_high_pass` | 丢掉两级中的高通 |
| l5 | **无门限语义**（静音段拉低读数） | `silence_padding_pulls_ungated_loudness_down` | 加门限而不改判据 |
| l6 | 静音/空输入 ⇒ −∞ 而非 NaN | `silence_and_empty_input_are_negative_infinity_never_nan` | `0/0` 不改 |
| l7 | 敌对样本不毒化读数 | `hostile_samples_do_not_poison_the_reading` | 删 `clean` |
| l8 | 确定性 + 分块一致 | `loudness_is_bit_deterministic` | 累计顺序依赖块大小 |
| l9 | 只接受被核验的 48 kHz | `only_the_verified_sample_rate_is_accepted` | 拿 48 kHz 系数硬套其它采样率 |
| l10 | 双二阶解析增益（含 997 Hz +0.691 dB） | `biquad_gains_match_the_itu_analytic_values` | 改系数一位小数 |

---

## 7. 本机真跑 vs 交给 CI（严格区分）

### 7.1 本机（M2）**真的跑过**的

| 命令 | 结果 |
| :--- | :--- |
| `bash scripts/dev/cargo-local.sh test -p yeban-dsp` | ✅ **142 passed**（+ 3 doc-test） |
| `bash scripts/dev/cargo-local.sh clippy -p yeban-dsp --all-targets -- -D warnings` | ✅ 0 告警 |
| `bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --all-targets` | ✅ **77 passed** + 2 个 `harness=false` 目标 ok（`rt_zero_alloc`、`meter_rt_contract`）；CI 默认 feature 实测 **87 passed**（见 §7.4） |
| `bash scripts/dev/cargo-local.sh clippy -p yeban-engine --no-default-features --all-targets -- -D warnings` | ✅ 0 告警 |
| `bash scripts/gates/run-gates.sh crate yeban-dsp` | ✅ 通过（**真编译真跑**：`clippy[yeban-dsp] ok` + `test[yeban-dsp]` 142+3，**不是 SKIP**） |
| `bash scripts/gates/run-gates.sh light` | ✅ fmt + 13 条守卫 + 文档门禁（50 个 md / 136 链接）+ 许可清单 |
| `bash scripts/dev/cargo-local.sh fmt --all --check` | ✅ |
| 5 条注入 → 变红 → 还原 | ✅ 见 §7.2 |

> ⚠ **本机没有编译默认 feature（`device` = cpal）** [ADR-0001 D19]。
> engine 的 `src/device.rs` 与 `rt.rs` 的 `NullBackend: Send` 断言、以及
> `cargo test -p yeban-engine`（默认 feature，**含 10 条 device 门控判据**）**只由 CI 判定**。
> 预期 CI 上 engine 单测 = **87**（77 + 10）；本线没有改 `device.rs`，`rt.rs` 也只因电平路径上移而"零改动"。
>
> 另外：`run-gates.sh crate yeban-engine` **不能在本机跑**——它的重依赖正则认不出
> `cpal = { workspace = true, optional = true }`（`engine-rt-notes.md` §5「门禁缺口 1」），
> 会真的去编译 cpal。因此 engine 侧一律用 `cargo-local.sh ... --no-default-features`。

### 7.2 注入 → 变红 → 还原（5 条）

| # | 注入 | 实测红点 |
| :-: | :--- | :--- |
| 1 | `DEFAULT_PEAK_DECAY_DB_PER_SEC` `20.0 → 10.0` | dsp **3 FAILED**（`peak_hold_decays_by_the_documented_rate`、`empty_block_still_decays_hold_and_keeps_finite_rms`、`frozen_pre_hoist_table…`）；engine **2 FAILED** |
| 2 | `sanitize_sample` 开头 `return sample;`（删钳位） | dsp **3 FAILED**；engine **4 FAILED**（含 `meter::tests::meter_bank_sanitizes_hostile_input_to_finite_levels`） |
| 3 | `NaN` 分支 `0.0 → sample`（原样返回） | dsp **2 FAILED**（`sanitize_clamps_and_never_returns_nan`、`frozen_pre_hoist_table…`）；engine **2 FAILED**。**诚实说明**：`nan_and_infinity_inputs_never_produce_nan_levels` **没有**变红 —— 因为 `LevelDetector` 自己的 `is_finite()` 兜底已经把 `NaN` 吸收成 0，该判据抓的是"读数不脏"，抓不到"单样本钳位被改"。抓它的是 dsp 的直接钳位判据与冻结表 |
| 4 | 在 engine 里偷偷留一份 `pub fn sanitize_sample`（从 `pub use` 里摘掉它） | engine **4 FAILED**：`engine_level_module_has_no_second_implementation`（源码判据命中 `fn sanitize_sample(`）、`engine_level_is_literally_the_dsp_type`（`fn_addr_eq` 失败）、`frozen_level_table_through_the_engine_path`、`sanitize_clamps_and_never_returns_nan`。**另有一层**：不加文档时连 `#![deny(missing_docs)]` 都过不去 |
| 5 | `TruePeakDetector::process` 去掉多相插值（只取样本峰值） | dsp **2 FAILED**（`true_peak_sees_the_intersample_overshoot`、`true_peak_dominates_sample_peak_and_sanitizes_hostile_input`）；engine **1 FAILED**（冻结表里的真峰值段） |

**还原判据**：每条注入后 `cp` 回注入前快照；`grep -rn "INJECT" crates/` = **0 命中**（L24）；
还原后 `cargo-local.sh test -p yeban-dsp` = 142 passed、engine = 77 passed。
⚠ 诚实交代一处**流程瑕疵**：注入 1 第一次实测时判据 `peak_hold_decays_by_the_documented_rate`
**没有变红** —— 因为它拿 `DEFAULT_PEAK_DECAY_DB_PER_SEC` 当期望值（判据与常量同源）。
本线随即把该判据的 375 / 20 / 0.3 **写死**并重跑注入 1，才得到上表的红点。
这正是"判据必须钉住口径本身、不能复读常量"的活例子（同族教训 L12/L18/L20）。

### 7.3 交给 CI 的（本机未跑）

- `cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo test --workspace --all-targets`
  的**默认 feature** 形态（含 cpal / Slint / symphonia）；
- `cargo deny check`（本机跑了 light 档的许可清单对账，**不是** deny 全量）；
- 跨平台（x86_64 Linux runner）的数值复现 —— **真的抓到了东西**，见 §7.4。

### 7.4 CI 判决与"CI 抓到的真问题"

| 轮 | commit | run id | 结论 | 关键读数 |
| :-: | :--- | :--- | :--- | :--- |
| 1 | `79c9c37` | `37238420778` | ❌ **红**：`checks`/`lockfile`/`deny`/`plan` 全绿，`rust (workspace 全量)` 的 `clippy --workspace -D warnings` **绿**，但 `test --workspace` **红** | `crates/yeban-dsp/src/meter.rs:1621`：`db.fract_1_sqrt2 漂移: 实测 0xc040a8c3 (-3.0103004), 搬迁前 0xc040a8c2 (-3.0103002)`；`test result: FAILED. 141 passed; 1 failed`。**285 条里只有 1 条**、且只差 **1 ulp** —— 这正是 `log10` 跨架构不保证正确舍入 |
| 2 | `a5b9c7d` | `37238844191` | ✅ **全绿** | `checks` ✓ 36s、`lockfile` ✓ 18s、`deny` ✓ 48s、`plan` ✓ 6s、**`rust (yeban-dsp)` ✓ 33s**、**`rust (yeban-engine)` ✓ 44s**（默认 feature，含 cpal/device）；`rust (workspace 全量)` 被 `plan` 按受影响集合跳过。判据改为按运算类别分策（§3.1.1）；本地重跑 5 条注入确认仍然全红 |

CI（x86_64 Linux）上与代码同源的关键读数（从 job 日志抓取）：

```text
rust (yeban-dsp)     test result: ok. 142 passed; 0 failed     ← 含 285 条冻结表（分策）
rust (yeban-engine)  test result: ok.  87 passed; 0 failed     ← 默认 feature: 77 + 10 条 device 门控
[meter-rt] S1 汇总: quanta=10242 publishes=10242 frames=40968 capacity_drops=0
[ARCH-UI-002] ok: 真峰值/RMS 计量 + 每量子一次批量发布 + UI 取最新 + 10,000 量子零分配
[MUST-GATE-001] ok: 10,000 量子 + 63 次快照交换，实时窗口内零分配零释放
（allocations=0 deallocations=0 共 105 行，与本机一致）
```

`87 = 77 + 10`：本机（`--no-default-features`）跑不到的 10 条 `device` 门控判据由 CI 补齐，
本机预测与 CI 实测一致。

> **这是本线最有价值的一次 CI 反馈**：`ci.yml` 的 workspace 腿（不是 per-crate 腿）
> 跑了 x86_64，把"我在 aarch64 上冻结的位模式"与"跨架构可复现"之间的差距**具体化**成一条 1 ulp 的断言。
> 处理方式不是把判据调松到"随便近似"，而是**按运算的舍入性质分类**：
> 该逐位的仍然零容差，只有数学上不保证正确舍入的那一类才拿到 ulp 预算。
>
> ⚠ 同时它暴露了本机验证的边界：**本机（aarch64）无法发现跨架构的数值差异**，
> 这类判据的最终裁决只能来自 CI。

---

## 8. pending / needs

| ID | 内容 | 归属 |
| :--- | :--- | :--- |
| N1 | **`engine-meters-notes.md` 的 MN1 关闭**：本线上移已落地，那份台账需要集成者补记（本线不改别的线所有者的文件） | 集成者 |
| N2 | **`docs/ledger/gate-status.md` 的 `MUST-GATE-001` 证据行**：可补一句"电平口径已在 `yeban-dsp`，engine 侧零分配窗口不变"。状态**不变**（仍为"部分"） | 集成者 |
| N3 | **真峰值倍数**：4× 在 `0.4·fs` 欠读 0.44 dB。若母带/导出要更紧的真峰值，需要 8×/16× 或 BS.1770 Annex 2 的专用核——属于新切片与口径裁决 | 人类/架构 |
| N4 | ~~冻结位模式的跨架构稳定性~~ **已由设计收敛**：按运算类别分策（IEEE 精确类逐位 / 超越函数类 4096 ulp），已在 CI run `37238420778` 的实测反馈上落地。**仍存**的诚实边界：`aarch64` 上"285 条全逐位"是本地结论，CI 只验证"IEEE 精确类逐位" | 已处理（本线） |
| N5 | **LUFS 门限切片**（−70 LUFS 绝对门限 + −10 LU 相对门限 + 400 ms/75% 重叠块 + 3 s 短时窗口）。落地时必须**显式改写** `silence_padding_pulls_ungated_loudness_down` | 后续电平/母带线 |
| N6 | **其它采样率的 K 加权系数**（44.1/88.2/96 kHz）。本实现明确拒绝，需要核验过的系数表 | 后续线 |
| N7 | **多声道（>2）独立电平与环绕权重**（沿用上一轮的 pending） | 混音台切片 |
| N8 | **UI 侧消费**（`MeterBoard` → Slint Property、60 Hz 定时器）——本线未碰 | `yeban-app` 线 |
| N9 | **`ARCH-DSP-002` 重采样 / `ARCH-DSP-004` 弹性拉伸**：需要 `rubato` / `signalsmith-stretch`，属重依赖，留给 CI 能编译的线 | 后续 DSP 线 |
| N10 | **峰值保持的"钉住时间"（hold time）**：本线**没有**做（上一轮 notes §7 第 6 条仍有效）。做的话应当是独立的 `PeakHoldMeter`，**不得**改动 `LevelDetector`（那会破坏 §3 的逐位契约） | 后续 UI/计量线 |

## 9. CI 改动需求

**没有。** 本线**没有**往 `.github/workflows/ci.yml` 加任何步骤。新增的
`yeban-engine → yeban-dsp` 依赖由既有 per-crate / workspace 腿自动覆盖；
`Cargo.lock` 与 `docs/ledger/dependency-licenses.md` 已同步（外部依赖包数不变，仍是 618）。
唯一可能需要 CI 侧动作的是 §8 的 N4（跨架构位模式风险）——但那是**观测结果**驱动的裁决，不是预先加步骤。

## 10. 复用来源（Reuse provenance）

本线代码是**新写 + 同仓库搬迁**：

- `meter.rs` 的电平口径**原样来自本仓库** `crates/yeban-engine/src/level.rs`
  （同一作者、同一 GPL-3.0-only 许可），不是第三方代码，因此**不新增**
  `THIRD_PARTY_LICENSES.md` 条目；
- 4× 真峰值核是**本线按公式生成**的（Kaiser 窗 sinc，逐相位归一化），生成口径写在代码注释里；
- LUFS 的 K 加权系数是 **ITU-R BS.1770-4 的公开标准数值**（标准表格，非代码），
  实现（双二阶 + 能量累加）是本线新写；
- 未引入任何新依赖（`engine → dsp` 是 workspace 内部 path 依赖），
  因此**不需要 hoist**、也不触发 `deny.toml` 的任何白名单。
