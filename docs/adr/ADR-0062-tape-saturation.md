# ADR-0062 — 磁带饱和（链级饱和器）：参数、位置与 PDC 口径

- **状态**: **Proposed**（待人类批准；本票已含实现，批准后按乒乓 → 侧链 → 饱和的合入顺序走 CI／合入）
- **决定者**: 待人类
- **关联**: `~/workspace/yeban-work/dsp-features-design-20261101.md` §1（设计文档；本 ADR 是它的落地裁决）；
  `ADR-0060`（编号顺延）；`ARCH-PDC-*`（延迟补偿）；`ARCH-DET-*`（确定性）
- **落点**: `crates/yeban-dsp/src/tape.rs`（新器件）＋ `device_param.rs`（`TapeField` 族）＋
  `crates/yeban-render/src/inserts.rs`（离线链）＋ `crates/yeban-engine/src/insert.rs`（实时投影）＋
  `crates/yeban-render/src/automation.rs`（自动化落点）＋ `crates/yeban-mcp/src/effect_params.rs`（MCP 描述）＋
  `crates/yeban-engine/tests/device_param_spellings_pinned.rs`（镜像判据）

## 背景

Nightfall 采样版复盘暴露：采样器轨没有可用的饱和手段（通道条的 `drive` 是滤波器驱动，
只走引擎音源投影，采样器轨不经过；`drive` 也不进链级）。设计文档 §1 给出链级饱和器方案，
本票按建议值实施。

## 决定

### 1. 器件与参数（`§1.2` 表原样落地）

| 参数 | 拼写（规范／别名） | 范围 | 缺省 | 含义 |
| :--- | :--- | :--- | :--- | :--- |
| 饱和驱动 | `tape_drive` / `saturation_drive` | 0..1 | 0.0 | 曲线 `pade_tanh(g·x)/g`，`g = 1 + 9·drive` |
| 干湿比 | `tape_mix` / `saturation_mix` | 0..1 | 0.0 | 0 = 干路直通 |
| 输出补偿 | `tape_trim_db` / `saturation_trim_db` | −24..+24 dB | 0.0 | 湿路后的补偿增益 |
| 湿路低通 | `tape_hf_hz` / `saturation_hf_hz` | 2000..20000 Hz | 20000 | 一阶低通；≥ 20000 时旁通 |

- 裸 `mix` / `drive` **不是**饱和名字（那是通道条的名字；歧义消解由
  `tape_projection_recognises_only_prefixed_spellings` 钉住）。
- 钳制规则**只有一份**：`TapeParams::sanitised()`（`TapeField::sanitise` 与两处投影都读它，
  ⛔ 不复制上下界）。非有限值归**下界**（与 `CompressorParams::sanitised` 同一条裁决，
  因 `f32::clamp` 对 NaN 是恒等）。
- 曲线复用 `math.rs` 的 `pade_tanh`（⛔ 不另写饱和函数）；小信号增益 ≈ 1（与 drive 无关），
  由 `small_signal_gain_is_approximately_one` 钉住。
- 映射系数 `g = 1 + 9·drive` 是设计文档的**建议值**（设计附录的未验证项之一），
  本票按它实施并由测试钉住；若人类要改系数，改一处（`tape.rs`）＋改测试期望。

### 2. 链位置与建成条件

- 位置：**通道条之后、Freeverb 之前**（离线链与实时投影同一条链；实时执行见 §5 后续项）。
- 级建成条件：`tape_drive > 0` **且** `tape_mix > 0`；任一缺省 ⇒ **不建器件**，
  ⛔ 不另设 `enabled` 开关。静态 `tape_mix: 0` 时强制实例化位（`force_saturation`，
  与 `#293` 的 `force_reverb` 同一条缝）让块边界泳道有落点。
- 2x 过采样往返固定：`Oversampler2x`，往返延迟 `OS_LATENCY = 31` 采样（`tape.rs` 的
  `latency_samples()`；冲激峰位判据钉住）。
- 湿路一阶低通只作用于湿路（干路不受 `tape_hf_hz` 影响）；磁滞模型 **v2 不做**
  （设计 §1 明确排除）。

### 3. PDC 口径（唯一事实源）

- 器件的 `latency_samples`（= 31）是延迟的**唯一事实源**：离线 `track_latencies`
  与实时 PDC 都读它，⛔ 不从链结构自动推导（设计文档的裁决；本票不发明推导规则）。
- MCP 描述（`effect_params.rs` 的 TAPE 组 summary）必须写明 31 采样这一条，
  与器件实现同值（⛔ 描述与实现各写一套会漂移）。

### 4. 旧工程逐位不变

- 新级缺省恒等：`TapeParams::DEFAULT`（drive 0 / mix 0）⇒ `is_active()` 为假 ⇒
  旧工程的链里**不出现**饱和级；已写饱和参数的旧工程（本票之前不可能存在，
  因名字是新加的）不在此列。
- 链非空判定沿用既有口径：投影里有饱和参数即非空（⛔ 不按 `is_active()` 收窄，
  与混响/卷积同一条规则）；运行态丢不活跃级是运行态的事。

### 5. 明确的后续项（本票不含，⛔ 不是静默缺口）

**实时 `rt.rs` 池接线不在本票**：离线链（`yeban-render`）已接线并可用；
实时执行需要 `EngineRuntime`／`BranchShare` 加 `armed_tapes` 表＋`tape_pool`＋
`tape` 划痕缓冲＋快照武装＋一条"strip 之后、reverb 之前"的处理级，
与侧链 agent 的 `rt.rs` 改动（`key_routes`）同文件 —— 为避合入冲突，
本票刻意不碰 `rt.rs`，后续项由父 agent 另派（实施时以本 ADR 的链位置与建成条件为准）。
在该后续项落地前，实时监听路径**不执行**饱和（离线导出执行）。

## 后果

- 采样器轨获得链级饱和手段；`drive` 不再是"采样器轨上写了没反应"的名字
  （MCP 描述写明链级与音源级的区别）。
- 侧链接通后有边且已武装的旧工程导出会变（设计文档的声明义务）—— 本票不涉及侧链，
  该声明随侧链票走。
- 实时监听与离线导出的差异（饱和只在离线执行）在后续项落地前必须如实写进发布说明，
  ⛔ 不宣称"全链路已接线"。

## ⭐ 修订：`tape_asym` —— 把丢掉的偶次谐波补回来（本票之后）

> 口径同台账第 278 条：**上文原文一字未删**，本节只**追加**并点名被修订的那一条。

**被修订的是**「决定 · 1. 曲线与信号模型」里 `y_sat = pade_tanh(g · x) / g` 的那一条，
以及「后果」里对"奇次谐波"的隐含假设。

### 问题（如实登记的一处音色缺口）

`pade_tanh` 是**奇函数** ⇒ 展开只有奇次项（3f / 5f / …），**没有任何偶次分量**。
磁带"暖"的听感主要来自 **2f**；只有奇次的饱和听上去更"硬"、更像过载而不是磁带。
这一点在本票的 v1 里是明确登记的局限（"饱和从离线链搬进工程时丢掉了 5% 偶次"）。

### 决定

新增参数 **`tape_asym`**（别名 `saturation_asym`，域 `[0, 0.5]`，缺省 `0`），
曲线改成**沿 x 轴平移**的形式：

```text
y_sat = pade_tanh(g · (x + asym)) / g − pade_tanh(g · asym) / g
```

- **`asym = 0` ⇒ 逐位等于原公式**：`pade_tanh(0) == 0` ⇒ 补偿项恰 `+0.0`，
  而 binary32 下 `a − 0.0` 逐位等于 `a`（含 `a == −0.0`）⇒ 旧工程输出不变；
- **减掉常量是必须的**：不减则 `x = 0` 输出 `pade_tanh(g·asym)/g ≠ 0`，
  整条链带上 DC，下游限幅器要为它让余量（判据 `asym_keeps_silence_silent` 钉住）；
- **不加任何状态、不加延迟**：仍然一次 `pade_tanh` 调用 ＋ 一次减法，
  PDC 仍是 31 采样 ⇒ 不影响既有 `latency_samples` 声明。

### 代价（如实登记）

- **小信号增益下降** `pade_tanh'(g·asym)`（`drive = 0.06`、`asym = 0.3` 时约 **−1.3 dB**）
  ⇒ 用 `tape_trim_db` 补。**本票不自动补偿**：自动补偿会把"改音色"和"改电平"
  绑在一起，两个旋钮就不再正交。
- 奇次也会变（工作点离开零点的直接后果）：实测 `asym = 0.3` 时 3f 从 `3.37e-2`
  降到 `2.26e-2`（**−3.5 dB**），而 2f 从 `1.27e-6` 升到 `5.05e-2`（**＋92 dB**）
  ⇒ 判据钉的是「2f 至少高 20 dB」＋「2f **超过** 3f」（"暖"的机械特征），
  ⛔ 不是"奇次不许动"（那在数学上不成立）。
- `tape_asym` **只在级建成时生效**（`drive > 0` 且 `mix > 0`，见「级建成条件」）——
  曲线是 `tanh(g·x)` 而 `g` 只由 `drive` 决定 ⇒ `drive = 0` 时级本来就不建。
  这是既有建成条件的直接后果，本票**不为它开口子**。

### 判据

`crates/yeban-dsp/src/tape.rs` 的 `asym_creates_the_second_harmonic_that_pade_tanh_cannot`
／`asym_keeps_silence_silent`／`asym_zero_is_the_pre_change_curve`，
以及 `crates/yeban-engine/tests/device_param_spellings_pinned.rs` 的等集判据
（实时 `TapeProjection` 的臂集 == `TapeField::SPELLINGS`，现读 **10** 条）。
