# `line/model-automation` — 自动化泳道 + 曲线的**规范形状**

> 工作线：`model-automation`（分支 `line/model-automation`，基线 `44a071a`）
> 地盘：`crates/yeban-model/**`（本文件与本线新增的 `tests/**` 亦属本线）
> 规范来源：`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §2（`MODEL-AST-002/003`）、§4（`ARCH-DSP-001`
> 的参数自动化平滑滤波）、§5（`ARCH-DET-001` L1/L2 确定性）、§6（`ARCH-OPS-001/002`）；
> 裁决依据：`docs/adr/ADR-0001-workspace-topology-and-version-pinning.md` **D10 / D12 / D27 / D28 / D32**；
> 风险：`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` **RSK-31**（自动化点密集轰炸）。

## 0. 这次交付的到底是什么

三条下游线各自把"自动化"记成缺口，而缺口是**同一个**：

| 线 | 症状（实测） |
| :--- | :--- |
| `line/mcp-render` | `crates/yeban-mcp/src/domain/render.rs` 的能力矩阵里 `automationLanes` 恒为 `unsupported`，注释写的理由是"**自动化曲线没有求值**（静态值也不代偿）" |
| `line/engine-sound` | 音符调度里没有自动化求值 ⇒ 参数只能取静态值，曲线无处可查 |
| 界面侧 | `TrackV3::automation_lanes` 有采样点，但没有"读开关 / 写模式 / 取值域 / 唯一求值入口" ⇒ 画不出、也点不动 |

`yeban-model` 里**已经有**（本线不重造，只补齐）：`AutomationTarget`（5 变体）、`AutomationPoint`
（`id/tick/value/curve`）、`AutomationLane`（`target` + `points: BTreeMap<EntityId, AutomationPoint>`）、
`TrackV3::automation_lanes: BTreeMap<AutomationTarget, AutomationLane>`（`BTreeMap`，红线 4）；
`CurveType` 四变体也已存在（与 `MidiNote.slide` 复用）。

本线补的是它们**缺的那一半**：

> ## ⚠ 本节的"三字段可缺省"表述已被 **ADR-0001 D43** 覆盖（集成者注，2026-10-05）
>
> 本线落地时把 `read_enabled` / `write_mode` / `domain` 写成"**为了旧工程可读**"的 `#[serde(default)]`；
> **D43 明确 1.0.0 之前没有兼容需求** ⇒ 前两者已改为**必需**（缺失即报错），
> `domain` 因为是 `Option<T>`（语义上天然可选）保留 default。详见 `docs/ledger/model-no-compat-notes.md`。
> 此外，本节引用的"**107+28+50+16 全绿**"是**当时的**读数；模型侧后来新增了判据
> （`no_compat` 8 条），现行读数是 **107+28+50+16+8**。
>
> 以下表格保留为**当时的记录**（历史原样）。

1. 泳道的**读/写模式**与**取值域覆盖**（新字段，全部 `#[serde(default)]` + 默认值不落盘）；
2. **唯一的求值入口**（`YebanProjectV1::automation_value_at` / `AutomationLane::value_at`）与
   **唯一的曲线口径**（`CurveType::ease`）；
3. 两个 `Op` 变体 `SetAutomationLane` / `RemoveAutomationLane`（真逆，逐字节）；
4. 目标的**对账**（`AutomationTarget::validate_against`）与**静态值**（`AutomationTarget::static_value`）；
5. 28 条判据（`crates/yeban-model/tests/automation.rs`）+ 3 次注入取证。

---

## 1. 数据形状表（字段 / 单位 / 来源）

### 1.1 目标寻址（既有，未改）

| 变体 | 载荷 | 固有单位 `nominal_unit()` | 固有取值域 `nominal_domain()` | 来源 |
| :--- | :--- | :--- | :--- | :--- |
| `TrackVolume` | `track_id` | `Decibels` | `[-60.0, +12.0] dB` | 规范 §2.2 的 `AutomationTarget`；区间是本线裁决（规范未给） |
| `TrackPan` | `track_id` | `Bipolar` | `[-1.0, +1.0]` | 规范明文 `pan` 合法域 `-1.0..=1.0`（`ModelError::PanOutOfRange`） |
| `SendGain` | `track_id`, `edge_id` | `Decibels` | `[-60.0, +12.0] dB` | 规范 §2.2（D3 前身"扩展 `AutomationTarget` 覆盖发送增益"）；`None` = 单位增益 |
| `DeviceParam` | `track_id`, `slot_index`, `param_index` | `Native` | **`None`（不可知）** | `ParameterValue` 只有自由文本 `unit`，模型没有任何参数区间声明 ⇒ 诚实返回"不可知" |
| `Macro` | `track_id`, `macro_index` | `Normalized` | `[0.0, 1.0]` | 规范明文宏值 `0.0..=1.0`（`ModelError::MacroValueOutOfRange`） |

### 1.2 泳道（本线扩展）

| 字段 | 类型 | serde | 默认值 | 单位 / 语义 | 来源 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `target` | `AutomationTarget` | 必填 | — | 同时是**泳道身份**（`BTreeMap` 键，必须与键一致，否则 `AutomationLaneTargetMismatch`） | 既有（规范 §2.2 类型清单） |
| `points` | `BTreeMap<EntityId, AutomationPoint>` | `default` | `{}` | 键 = 点身份，必须与 `point.id` 一致（否则 `EntityKeyMismatch`） | 既有（`MODEL-AST-003`） |
| `read_enabled` | `bool` | `default = true`，`skip_serializing_if = is_true` | `true` | **读**：走带/离线渲染是否应用本泳道 | 本线裁决（规范未定义，见 §4） |
| `write_mode` | `AutomationWriteMode` | `default`，`skip_serializing_if = is_off` | `Off` | **写**：录制时如何写入（`Off/Write/Touch/Latch`） | 本线裁决（规范未定义，见 §4） |
| `domain` | `Option<AutomationValueDomain>` | `default`，`skip_serializing_if = Option::is_none` | `None` | 取值域**覆盖**；`None` ⇒ 用目标的固有取值域 | 本线裁决（规范未定义，见 §4） |

### 1.3 取值域与单位（本线新增类型）

| 类型 | 形状 | JSON | 说明 |
| :--- | :--- | :--- | :--- |
| `AutomationValueDomain` | 私有字段 `{ min, max }` + 访问器 | `{"min":-60.0,"max":12.0}` | **端点按定义排序** ⇒ `min <= max` 是类型不变量；非有限端点被拒（`NonFiniteValue`） |
| `AutomationUnit` | `Decibels / Normalized / Bipolar / Native` | 字符串 | **不存储**：由目标派生（`AutomationLane::unit()`），避免两份事实源漂移（D28 的同一理由） |
| `AutomationWriteMode` | `Off / Write / Touch / Latch` | 字符串 | 默认 `Off`（唯一安全默认：默认"录制"会凭空产生写入） |

### 1.4 采样点与曲线（既有，未改）

| 字段 | 类型 | 语义 | 来源 |
| :--- | :--- | :--- | :--- |
| `AutomationPoint::tick` | `u64` | 960 PPQ 整数时钟上的位置 | `MODEL-AST-001` |
| `AutomationPoint::value` | `f32` | 该点的值（必须有限，否则 `NonFiniteValue`） | 既有 |
| `AutomationPoint::curve` | `CurveType` | **到下一个点**的曲线形状（`default = Linear`） | 既有；口径见 §2 |

### 1.5 本线**刻意不做**的两件事（连同理由）

| 没做 | 理由 |
| :--- | :--- |
| **不给泳道加 `id: EntityId`** | 泳道身份**就是** `AutomationTarget`（`BTreeMap` 键 + 内嵌 `target` 一致性判据）。再加一个身份字段 = 两份会漂移的事实源（D28 已被同类问题咬过一次）；而且旧文档读到的新 `id` 全是 nil，会立刻撞上"重复身份"校验 |
| **不给 `CurveType` 加 `Step`(阶梯) 变体** | 规范把 `curve` 定义为"**到下一个点**的曲线形状" ⇒ 采样点之间是**分段插值（曲线段）**，不是阶梯，四个变体就是全集。`CurveType` 还被 `MidiNote.slide` 复用，加变体会同时改动滑音语义。"阶梯"只出现在边界（区间外/单点 = **保持**），见 §2 |

---

## 2. 求值口径（唯一入口 + 边界 + 实测数值）

### 2.1 调用面

```rust
// 工程级（渲染 / 引擎 / 界面 都应当只调这一个）
let value: Option<f32> = project.automation_value_at(&target, tick)?;   // Err = 目标不存在
let used = value.unwrap_or(target.static_value(&project)?);             // 无自动化 ⇒ 静态值

// 已经拿到泳道时的纯计算版本（无 Result）
let value: Option<f32> = lane.value_at(tick);

// 曲线形状（唯一口径；滑音/弯音也用它）
let u: f32 = curve.ease(t);
```

三条语义（**下游必须按此分支**）：

| 返回 | 含义 | 下游该做什么 |
| :--- | :--- | :--- |
| `Err(_)` | 目标在文档里**不存在**（具体到哪个音轨/插槽/参数/宏/边） | 报能力缺口或参数错误，**不要**静默取静态值 |
| `Ok(None)` | 目标存在，但**没有可用自动化值**（无泳道 / 空泳道 / `read_enabled == false`） | 退回 `target.static_value(&project)` |
| `Ok(Some(v))` | 该 tick 的自动化值 | 直接用；引擎侧再按 `ARCH-DSP-001` 做 5 ms 单极点平滑 |

### 2.2 算法（确定性）

```text
① 取点集 → 按 (tick, point_id) 升序    // 与 BTreeMap 键序、插入序无关；同 tick 由 point_id 决定胜者
② 空                 → None
③ tick <  首点.tick  → 首点.value      // 保持（不外推）
④ tick >= 末点.tick  → 末点.value      // 保持（不外推；命中末点也走这条）
⑤ 否则 low = 最后一个 tick <= T 的点，high = 第一个 tick > T 的点：
      t = (T - low.tick) / (high.tick - low.tick)        // f32 除法
      u = low.curve.ease(t)                              // ← 左端点的曲线决定这一段
      value = low.value + (high.value - low.value) * u   // 只用 + - *
```

`CurveType::ease(t)`（`t` 先钳到 `[0,1]`；`NaN` 原样传出，绝不"修成 0"）：

| 变体 | 公式 | `u(0.25)` | `u(0.5)` | `u(0.75)` |
| :--- | :--- | :--- | :--- | :--- |
| `Linear` | `t` | `0.25` | `0.5` | `0.75` |
| `Exponential` | `t²` | `0.0625` | `0.25` | `0.5625` |
| `Logarithmic` | `t·(2−t)` | `0.4375` | `0.75` | `0.9375` |
| `SCurve` | `t²·(3−2t)` | `0.15625` | `0.5` | `0.84375` |

- 上述数值是**本机真跑**的实测输出（探针 `tests/__probe.rs`，用后已删；同一组值在判据 ①a/①b 里
  以 `to_bits()` **逐位**断言）。
- 四者都满足 `u(0)=0`、`u(1)=1`、单调不减、值域 `[0,1]`，因此曲线**精确穿过两个端点**，
  采样点上不会有跳变（判据 ①d）。
- **没有超越函数**（`exp/log/pow` 一律不用）⇒ 按 ADR-0001 **D32** 的分类属于"IEEE 精确类"，
  跨架构**零容差**；`MUST-GATE-003` 的 `1e-6` 预算对自动化求值不会成为瓶颈。

### 2.3 边界与实测数值（判据 ②）

| 情形 | 实测（本机真跑） |
| :--- | :--- |
| 空泳道 | `value_at(任意 tick) = None`（**不是** `0.0`；`0.0` 会被下游误当成"自动化到 0"） |
| 单点 `tick=1920, value=-7.5` | `tick=0 → -7.5`；`tick=1920 → -7.5`；`tick=u64::MAX → -7.5` |
| 两点 `[960, -6.0] → [3840, 0.0]` | `tick=0 → -6.0`；`tick=959 → -6.0`；`tick=960 → -6.0`（逐位命中）；`tick=2400 → -3.0`（SCurve 的中点）；`tick=3840 → 0.0`；`tick=u64::MAX → 0.0` |
| 音量 dB 泳道 `[0, -60.0] → [3840, +12.0]`（Linear） | `tick=0 → -60.0`；`960 → -42.0`；`1920 → -24.0`；`2880 → -6.0`；`3840 → +12.0`；`7680 → +12.0` |
| 非二进制精确（区间 `[0,3]`、值 `0→1`） | `Linear tick=1 → 0.33333334`；`Exponential → 0.11111112`；`Logarithmic → 0.5555556`；`SCurve → 0.25925928`（判据 ①c 用容差 `1e-6` 与公式对账） |
| 同 tick 两点 `(960,-20.0,id100)` / `(960,-10.0,id200)` | `tick=960 → -10.0`（`point_id` 更大者胜）；`tick=0 → -20.0` |

### 2.4 确定性（判据 ⑥）

- 同一泳道同一 tick 求值 1000 次/点 × 12 个 tick：`to_bits()` 全等。
- 两份独立构造的等价泳道（含**不同插入序**）给出同一 `to_bits()`；序列化也逐字节相同。
- 求值只用 `+ - * /` 与比较 ⇒ 同架构逐位可复现，跨架构按 D32 属零容差类。

---

## 3. `Op` 变体清单与真逆证据

| 变体 | 载荷 | 前置条件 | 逆操作 |
| :--- | :--- | :--- | :--- |
| `SetAutomationLane` | `target`, `old_lane: Option<AutomationLane>`, `new_lane: AutomationLane` | ① `new_lane.target == target`；② `new_lane` **不得**与隐式泳道逐位不可区分；③ 音轨存在；④ `new_lane.validate()`（点键/身份一致、取值有限）；⑤ 文档当前泳道 == `old_lane` | `old_lane = Some(x)` → 交换的 `SetAutomationLane`；`old_lane = None` → `RemoveAutomationLane { previous_lane: new_lane }` |
| `RemoveAutomationLane` | `target`, `previous_lane: AutomationLane` | ① 音轨存在；② `previous_lane` 非隐式形状；③ 文档当前泳道 == `previous_lane` | `SetAutomationLane { old_lane: None, new_lane: previous_lane }` |

**"隐式泳道"不变量**（本线的关键裁决，见 §4.3）：

- 隐式形状 = `points` 为空 **且** `read_enabled == true` **且** `write_mode == Off` **且** `domain == None`
  （= `AutomationLane::implicit(target)`）。
- `SetAutomationPoint` 首次触碰某目标时**自动创建**隐式泳道；
  `RemoveAutomationPoint` 把泳道移空后，**仅当它仍是隐式形状**时自动回收。
  ⇒ 自动建/自动收**精确互逆**（判据 ③c）。
- 带属性的泳道被移空后**保留**（否则"移空最后一个点 → 撤销"会丢掉读/写模式与取值域）。
- 两个新变体**拒绝**隐式形状（创建与删除两侧都拒）：该形状的存亡完全由采样点决定，
  允许显式写它就等于允许一个撤销无法判定存亡的状态（§4.3 有实测推导）。

**真逆证据**（逐字节，`serde_json::to_string` 比较）：

| 判据 | 场景 | 证据 |
| :--- | :--- | :--- |
| ③a | 新建（`old_lane = None`，带 `write_mode=Touch` + `domain`）→ 撤销 | 逐字节回到原状 |
| ③a | 修改（`old_lane = Some`，关 `read_enabled`）→ 撤销 | 精确还原上一条泳道，再撤销新建后逐字节回到原状 |
| ③b | 删除（含"空但带属性"的泳道）→ 撤销 | 逐字节还原（属性 + 采样点都在） |
| ③c | 隐式建/收、属性泳道移空/撤销 | 两种情形都逐字节 |
| ⑧b | `Batch[SetAutomationLane, SetAutomationPoint, RemoveAutomationPoint]` → 逆批次 | 逐字节回到原状 |
| crate 内 | `every_variant_applies_and_inverts_exactly`（showcase 覆盖 29 个变体） | 通过 |
| crate 内 | `state_tree_is_conserved_under_reverse_undo`（`MUST-GATE-010`，本机 256 步 / CI 10000 步，生成器已含两个新变体） | 通过 |

## 4. 规范依据与裁决留痕（D10 / D12 / D27 / D28 / D32）

### 4.1 规范**没有**给的东西（实测：对四份规范全文正则检索）

- "读/写模式"（`Read/Write/Touch/Latch/Off`、`read_mode`、`write_mode`、`自动化模式`）**零命中**；
- `CurveType` 只有四个**名字**（`Linear/Exponential/Logarithmic/SCurve`），**没有公式**；
- `AutomationLane` 只有类型名（规范 §2.2 的结构清单里写"保持不变"），**没有字段形状**；
- 与自动化相关的规范条文只有：`ARCH-DSP-001` 的"参数自动化平滑滤波（τ≈5 ms 单极点低通）"、
  `ARCH-OPS-001` 的"自动化**播放**是瞬态渲染流，不是 Op"、以及 `RSK-31`（点密集轰炸 ⇒
  Ramer-Douglas-Peucker 在线抽取、最低采样间隔 16 采样点）。

### 4.2 本线裁决（按 D10"规范没写的部分记录裁决"）

| # | 裁决 | 代价 / 反过来要改什么 |
| :--- | :--- | :--- |
| A1 | `CurveType::ease` 用**四项多项式**（`t` / `t²` / `t·(2−t)` / `t²(3−2t)`）定义形状，刻意不用超越函数 | 若人类要求"指数/对数"必须是真 `exp/log`，则要改这一处 + 判据 ①b/①d，并接受跨架构 ulp 预算（D32 超越函数类） |
| A2 | 泳道身份 = `AutomationTarget`（不加 `id`） | 若下游需要"跨重新寻址保持选中"，要加 `id` 并补一条与 `target` 一致性的判据 |
| A3 | 边界为**保持**（不外推）；采样点之间为**分段插值**（不是阶梯）；`Step` 不进 `CurveType` | 若界面要"阶梯"绘制，那是**渲染选项**，不该进持久化形状 |
| A4 | 推子类固有取值域取 `[-60, +12] dB`；设备参数固有取值域为 `None`（不可知） | 区间数字是工程约定（规范无）；设备参数的值域需要**设备描述符**补字段（另一条线的地盘） |
| A5 | 新增 `read_enabled` / `write_mode` / `domain`；默认 `true / Off / None`，且默认值不落盘 | 若人类给出别的字段名/默认值，旧文档不受影响（都是新增可选字段） |
| A6 | 隐式泳道不变量（§3）：`SetAutomationLane`/`RemoveAutomationLane` 拒绝隐式形状 | 若允许显式创建空默认泳道，`SetAutomationPoint`/`RemoveAutomationPoint` 的自动建/收就不再与其互逆（判据 ③c 会红） |
| A7 | `Op` 全集从 **27** 扩到 **29**（`SetAutomationLane` / `RemoveAutomationLane`） | 与 **D12/D27 同一族**：需要人类追认并回写规范 §6.1（见 §6 needs） |

### 4.3 A6 的实测推导（为什么必须拒绝隐式形状）

> 若允许 `SetAutomationLane { old_lane: None, new_lane: 隐式空泳道 }`：
> `[SetAutomationLane(隐式空), SetAutomationPoint(p), RemoveAutomationPoint(p)]` 的逆批次是
> `[SetAutomationPoint(p), RemoveAutomationPoint(p), RemoveAutomationLane(隐式空)]`。
> 应用时第二步会把**变空后仍是隐式形状**的泳道自动回收，于是第三步
> `RemoveAutomationLane` 的前置条件（泳道存在且逐位相同）**必然失败**。
> 拒绝该形状后，隐式泳道的存亡完全由采样点决定，两个方向都精确（判据 ③c/⑧a 覆盖）。

### 4.4 D27 式"判据盲区"的同一处理

本线新增的两个变体**改不动** `schemas/ops.schema.json`（`schemas/**` 由集成者与契约线共同拥有）。
为了让"枚举 ⊃ 契约"这件事**不能静默腐烂**，`op_variants_match_ops_schema_exactly` 与
`every_op_variant_is_declared_in_the_contract` 现在断言：

```text
enum − contract  ==  PENDING_CONTRACT_OPS   // 显式欠账，恰好相等
enum ⊇ contract                             // 契约的分支一个都不能少
contract ∩ PENDING_CONTRACT_OPS == ∅        // 契约补齐后必须清空清单
```

实测（本机）：把 `PENDING_CONTRACT_OPS` 从 2 个改成 1 个 ⇒ 两条判据立刻红并指名
`枚举里多出来的变体必须**恰好**是 PENDING_CONTRACT_OPS; 若契约已补齐, 请把该清单清空`。
契约补上这两个分支后，**必须同时**把该常量清成空数组，否则判据会红（这正是棘轮的作用）。

---

## 5. 下游接入点（渲染 / 引擎 / 界面 各一句"该调什么"）

| 下游 | 该调什么 | 不该做什么 |
| :--- | :--- | :--- |
| **渲染**（`yeban-mcp/domain/render.rs`、`yeban-render`、离线母带） | 每轨每个被自动化的目标：`project.automation_value_at(&target, tick)?`，`None` 时退回 `target.static_value(&project)?`；随后该把 `automationLanes` 从 `unsupported` 矩阵里移出（**这就是那条线的接入动作**） | 不要自己按 `points` 手写插值；不要跳过 `read_enabled`；不要用 `points.values().next()` 当"当前值" |
| **引擎**（`yeban-engine` 的音符调度/参数平滑） | 先把 `lane.points_in_tick_order()` 排一次序并缓存成 tick 索引（`RSK-31`：>10 万点），每个处理量子只按当前 tick 调一次求值口径并沿用 `CurveType::ease` 的语义；对结果做 `ARCH-DSP-001` 的 5 ms 单极点平滑 | 不要在 RT 回调里调用 `AutomationLane::value_at`（它是 O(n log n) 的模型层实现，为"唯一口径"而不是性能）；不要在回调里分配（红线 7） |
| **界面** | `lane.unit()` 画轴标签、`lane.effective_domain()` 定纵轴量程（`None` ⇒ 用曲线最值自适应）、`lane.points_in_tick_order()` 画折线（相邻两点之间按左端点的 `lane.points[i].curve` 采样 `ease`）、`lane.read_enabled`/`lane.write_mode` 画开关与录制臂 | 不要把 `domain` 当成"合法值钳位"（它只是取值域/显示域）；不要另写一份曲线公式（D28：两份必然漂移） |

**接口清单**（`yeban_model` 公开面，本线新增/明确）：

```text
YebanProjectV1::automation_value_at(&target, tick) -> Result<Option<f32>, ModelError>
YebanProjectV1::automation_lane(&target)            -> Option<&AutomationLane>
AutomationLane::value_at(tick)                      -> Option<f32>
AutomationLane::points_in_tick_order()              -> Vec<AutomationPoint>
AutomationLane::unit()                              -> AutomationUnit
AutomationLane::effective_domain()                  -> Option<AutomationValueDomain>
AutomationLane::implicit(target) / is_implicit()    -> AutomationLane / bool
AutomationTarget::track_id() / nominal_unit() / nominal_domain()
AutomationTarget::validate_against(&doc)            -> Result<(), ModelError>
AutomationTarget::static_value(&doc)                -> Result<f32, ModelError>
CurveType::ease(t)                                  -> f32
AutomationValueDomain::new(a, b) / min() / max() / span()
NOMINAL_GAIN_MIN_DB / NOMINAL_GAIN_MAX_DB
```

---

## 6. 判据清单（`crates/yeban-model/tests/automation.rs`，28 条）

| # | 判据 | 覆盖 |
| :--- | :--- | :--- |
| ①a | `linear_interpolation_is_bitwise_equal_to_the_documented_formula` | 线性实测（逐位） |
| ①b | `all_four_curve_shapes_match_the_documented_formulas` | 四形状公式（逐位）+ 泳道确实复用它 |
| ①c | `interpolation_on_a_non_dyadic_span_matches_the_formula_within_1e_6` | 非精确 t，容差 1e-6 |
| ①d | `curve_shapes_hit_both_endpoints_and_are_monotone` | 端点精确 / 单调 / 值域 / 越界钳位 |
| ②a | `empty_lane_yields_no_value` | 空泳道 = `None`（工程级同样 `Ok(None)`） |
| ②b | `single_point_lane_holds_its_value_everywhere` | 单点保持 |
| ②c | `ticks_outside_the_point_range_hold_the_boundary_value` | 首前/末后保持；区间内仍是插值 |
| ②d | `exact_point_hits_return_the_point_value_bitwise` | 命中采样点逐位 |
| ②e | `same_tick_points_resolve_deterministically_by_point_id` | 同 tick 的确定胜者 |
| ②f | `points_in_tick_order_is_deterministic_and_ordered` | `(tick, id)` 序 |
| ③a | `set_automation_lane_is_a_true_inverse_byte_for_byte` | 新建 + 修改的真逆 |
| ③b | `remove_automation_lane_is_a_true_inverse_byte_for_byte` | 删除的真逆 |
| ③c | `implicit_lane_lifecycle_is_exactly_reversible` | 隐式建/收 + 属性泳道保留 |
| ④a | `serde_round_trip_of_the_full_shape` | 文档级往返 + 两次序列化逐字节 |
| ④b | `value_domain_and_write_mode_json_shape_is_stable` | 取值域排序/默认值不落盘/非法输入被拒 |
| ⑤a | `legacy_lane_json_reads_with_defaults_and_reserializes_byte_identically` | 旧泳道 JSON 可读 + 默认值 + 再导出不变 |
| ⑤b | `legacy_project_document_without_the_new_fields_is_still_readable` | 整个旧工程可读/可校验/可求值 |
| ⑥a | `evaluation_is_bitwise_deterministic_across_repeated_calls` | 逐位确定性 |
| ⑥b | `evaluation_and_serialization_are_independent_of_insertion_order` | 与插入序无关 |
| ⑦a | `nominal_units_are_derived_from_the_target` | 单位派生表 |
| ⑦b | `nominal_domains_are_derived_from_the_target` | 取值域派生表 + 覆盖优先 |
| ⑦c | `automation_value_at_reconciles_the_target_and_reports_specific_errors` | 目标不存在 → 具体错误 |
| ⑦d | `static_value_covers_every_target_including_send_gain` | 静态值（含 `None` = 0 dB） |
| ⑦e | `read_disabled_lane_yields_no_value_but_keeps_its_points` | 读开关语义分层 |
| ⑧a | `lane_ops_reject_inconsistent_payloads_without_mutating` | 8 条拒绝路径且文档不变 |
| ⑧b | `lane_ops_inside_a_batch_are_reversible` | 批量真逆 |
| ⑧c | `documents_with_the_new_lane_fields_stay_valid` | 工程校验覆盖新字段 |
| ⑧d | `new_op_names_match_their_json_tags` | 新变体名 == JSON 标签 |

`cargo test -p yeban-model` 全绿：**107**（lib）+ **28**（automation）+ **50**（container_adversarial）
+ **16**（container_roundtrip）。

## 6.1 规范样本与**契约**的对账（本机真跑，跨实现）

`crates/yeban-model/src/samples.rs` 的 `filled_project()` 现在**显式**给出新形状（`write_mode: "Touch"`、
`domain: {"min":-60.0,"max":12.0}`；`read_enabled` 是默认值 `true` ⇒ 不落盘）。用导出的样本对
`schemas/project.schema.json` 做 **Rust serde ↔ Python jsonschema** 的独立对账：

```text
$ cargo run -p yeban-model --example export_schema_samples -- --out /tmp/schema-samples
导出 4 份样本到 /tmp/schema-samples
$ python3 scripts/gates/validate_schemas.py --samples-dir /tmp/schema-samples
[ok] project.filled.json: 通过 project.schema.json
[ok] ops.filled.json: 通过 ops.schema.json
契约校验通过 (4 份 schema)。
```

导出的泳道片段（实测）：

```json
{ "target": { "TrackVolume": { "track_id": "01J8ZQ00000000000000000002" } },
  "points": { "...40": {"id":"...40","tick":0,"value":-6.0,"curve":"Linear"},
              "...41": {"id":"...41","tick":3840,"value":0.0,"curve":"SCurve"} },
  "write_mode": "Touch",
  "domain": { "min": -60.0, "max": 12.0 } }
```

⇒ 新增字段**不需要**改 `schemas/project.schema.json`（它没有描述 `automation_lanes`，且对音轨对象
没有 `additionalProperties: false`），因此本线在 `schemas/**` 上**零改动**是有依据的，而不是"跳过了"。

## 7. 注入 ▸ 变红 ▸ 还原（3 次，本机真跑原始记录）

> 纪律：注入后**逐条**看是哪些判据变红、红的理由是否**指向被改坏的那件事**；随后**原样还原**并复跑全绿。

### 注入 1：求值忽略 tick（永远取第一个采样点）

改动（`src/automation.rs` 的 `value_at`：删掉 `partition_point` 插值分支，直接 `Some(ordered[0].value)`）：

```text
test result: FAILED. 22 passed; 6 failed
FAILED: all_four_curve_shapes_match_the_documented_formulas
        → 失败原因: assertion `left == right` failed: Linear 在 t=0.25 (tick=250) 处的求值必须等于 0.25
          left: 0 / right: 1048576000            （f32 位模式：0 与 0.25）
FAILED: exact_point_hits_return_the_point_value_bitwise
        → tick=480 命中采样点必须逐位等于 3.25   left: Some(3241672704) / right: Some(1078984704)
FAILED: interpolation_on_a_non_dyadic_span_matches_the_formula_within_1e_6
        → Linear tick=1: 实测 0, 公式 0.33333334
FAILED: linear_interpolation_is_bitwise_equal_to_the_documented_formula
        → tick=250: 线性插值必须逐位等于 0.25
FAILED: legacy_project_document_without_the_new_fields_is_still_readable
        → 线性中点必须是 -6.0，实测 -9
FAILED: ticks_outside_the_point_range_hold_the_boundary_value
        → 区间内必须是插值（SCurve 在 t=0.5 处恰为中点）: -6
```

（当时的 22 条通过里有 `empty_lane`/`single_point`/`same_tick`/`确定性`：它们被 `tick < 首点`/`tick >= 末点`
两个保持分支覆盖，注入 1 恰好没碰那两条 —— 这也说明三组判据各管一段，没有互相顶替。）

还原：`cp /tmp/inj_automation.rs.bak src/automation.rs` ⇒ `28 passed; 0 failed`。

### 注入 2：两个泳道 Op 的 `structural_inverse` 改成恒等

```text
test result: FAILED. 105 passed; 2 failed          （lib）
FAILED: ops::tests::every_variant_applies_and_inverts_exactly
        → SetAutomationLane 求逆失败: `SetAutomationLane` does not match the document state
FAILED: ops::tests::state_tree_is_conserved_under_reverse_undo   （MUST-GATE-010）
        → 撤销第 0/4/5/11/18/44 步 (SetAutomationLane / RemoveAutomationLane) 时求逆失败

（单独跑集成判据）test result: FAILED. 25 passed; 3 failed
FAILED: set_automation_lane_is_a_true_inverse_byte_for_byte        → 撤销新建泳道: OpStateMismatch
FAILED: remove_automation_lane_is_a_true_inverse_byte_for_byte     → 撤销删除泳道: OpStateMismatch
FAILED: lane_ops_inside_a_batch_are_reversible                    → 批量撤销: OpStateMismatch
```

还原：`cp /tmp/inj_ops.rs.bak src/ops.rs` ⇒ lib `107 passed` + automation `28 passed`。

### 注入 3：删掉三个新字段的 `#[serde(default)]`

```text
test result: FAILED. 25 passed; 3 failed
FAILED: legacy_lane_json_reads_with_defaults_and_reserializes_byte_identically
        → 旧 JSON 必须可读: Error("missing field `read_enabled`", line: 0, column: 0)
FAILED: legacy_project_document_without_the_new_fields_is_still_readable
        → 旧工程必须可读: Error("missing field `read_enabled`", line: 0, column: 0)
FAILED: value_domain_and_write_mode_json_shape_is_stable
        → 默认 true 不落盘
```

还原：`cp /tmp/inj_project.rs.bak src/project.rs` ⇒ `28 passed; 0 failed`。

### 附加（契约棘轮）：把 `PENDING_CONTRACT_OPS` 少写一个

```text
FAILED: every_op_variant_is_declared_in_the_contract / op_variants_match_ops_schema_exactly
→ assertion `left == right` failed: 枚举里多出来的变体必须**恰好**是 PENDING_CONTRACT_OPS;
  若契约已补齐, 请把该清单清空;
  left: {"RemoveAutomationLane", "SetAutomationLane"} / right: {"SetAutomationLane"}
```

还原后全绿。

---

## 8. 本机真跑 vs CI（严格区分）

| 项 | 本机（Apple M2，`scripts/dev/cargo-local.sh`） | CI |
| :--- | :--- | :--- |
| `cargo clippy -p yeban-model --all-targets -- -D warnings` | **真跑，绿** | 跑 |
| `cargo test -p yeban-model` | **真跑，绿**（107 + 28 + 50 + 16） | 跑 |
| `run-gates.sh crate yeban-model` | **真跑，绿**（fmt + 13 条守卫 + 文档 + 许可清单 + clippy + test） | — |
| `run-gates.sh light` | **真跑，绿**（许可清单 679 行与依赖图一致、`Cargo.lock` 未变） | 跑 |
| 规范样本 ↔ 契约对账（`export_schema_samples` + `validate_schemas.py --samples-dir`） | **真跑，绿**（4 份样本全过；见 §6.1） | 跑 |
| `MUST-GATE-010` 的 10,000 步属性测试 | 只跑本机默认 **256** 步（`YEBAN_PROPTEST_CASES` 可控） | 跑 **10,000** 步 |
| 跨架构 L2 确定性对账（`MUST-GATE-003`） | **不跑**（本机纪律；D7/D32 记 PENDING） | ARM/x86 腿 |
| 其它 crate 的编译（`yeban-mcp` / `yeban-app` / `yeban-store`） | **不跑**（重依赖，本机纪律） | 跑（本线只碰 `yeban-model`；已 grep 确认下游无 `Op` 穷举 match、无 `AutomationLane` 结构体字面量） |
| 判决 | **不算数** | `scripts/dev/ci-verdict.sh line/model-automation` |

### 8.1 已读回的 CI 判决（代码提交 `2a94a5f`，run **37244333377**）

| job | 结论 |
| :--- | :--- |
| `plan` / `checks`（fmt + 13 守卫 + schema）/ `lockfile` / `deny` | ✓ |
| `windows (yeban-mcp / yeban-model)` | ✓ |
| `rust (yeban-model)` / `rust (yeban-mcp)` / `rust (yeban-render)` / `rust (yeban-decode)` / `rust (yeban-ui-mcp)` / `rust (yeban-engine)` | ✓ |
| `rust (yeban-app)` | ✗ —— **唯一红点，且是 main 上的预存在红点**（见下） |

### 8.2 预存在红点（**不是本线回归**，两条 run 逐字相同）

| 证据 | 值 |
| :--- | :--- |
| 本线 run | `37244333377`（`line/model-automation`，sha `2a94a5f`），失败 job `rust (yeban-app)` |
| main 自己的 run | `37244018311`（`main`，sha `fb7f80d6`），失败 job `rust (workspace 全量)` |
| 两条 run 的失败判据 | `crates/yeban-app/tests/live_ui_mcp.rs:892` `admin_reload_engine_rebuilds_and_resets_the_meter_tap` |
| 两条 run 的断言文本 | `引擎换代之后旧读数必须作废（电平回到下限）`，`left: "轨道 鼓 电平表 峰值 -6.0 RMS -23.4 dBFS"`，`right: … -120.0 …`（**逐字相同**） |
| 与本源的关系 | `git merge-base --is-ancestor 2a94a5f origin/main` ⇒ **否**（`origin/main = fb7f80d`，本线提交不在其中） |

根因（推理，可复核）：`40c371f merge(engine-sound)` 把 `render_block` 从占位静音换成真实合成之后，
`ui/reload_engine` 之后**新引擎对"鼓"轨立刻渲染出真实电平（-6.0 dBFS）**，而该判据仍写死
"换代后电平必须回到下限 -120.0" ⇒ 断言的前提被上游改动**作废**。修法在 `crates/yeban-app`
（判据或 `ui/reload_engine` 的读数作废语义），**不在本线地盘**（本线只拥有 `crates/yeban-model/**`）
⇒ 按处置纪律上报为 needs（N7），不改别人的 crate。

**判决的诚实表述**：本线代码的 **6 条 rust 腿 + checks + windows + deny + lockfile 全绿**；
唯一红点在 main 上以**完全相同**的形态复现 ⇒ 本线**没有引入回归**，但本线的 CI 判决**不能写成"通过"**
（未通过就是未通过；根因归属是另一件事）。

### 8.3 文档提交的判决（`21566c9`，run **37244665346**）= success，**但它不验证代码**

`docs-only` 的改动被 `plan` 判定为无受影响 crate ⇒ **六条 rust 腿全部 `skipped`**
（`rust (${{ matrix.crate }}) in 0s -`、`rust (workspace 全量) in 0s -`、`windows ... -`）。
⇒ 这一轮只证明"文档不破坏 fmt/守卫/schema/许可清单"，**不能**用来声称代码被验证过
（这正是 `docs/DEVELOPMENT_LEDGER.md` **L23/L26** 记的坑：被取消/被跳过的腿不是判决）。
代码的验证依据只有 §8.1/§8.2 那一轮（`37244333377`，sha `2a94a5f`）。

### 8.4 红点的归属与处置（集成者已接手）

集成者已确认该红点是 `main` 自己在 `40c371f` 之后带进去的（合并后未读那一轮判决 = L23/L26 的复发），
并在 `crates/yeban-app` 修（`main` 的 `c210eb4`，run `37244705178`）。**本线不碰别人的 crate**，
只把它记为 needs N7；等 `main` 转绿后，本线的代码验证结论可以直接复用（本源未触碰 `crates/yeban-app`）。

## 9. needs（交给集成者 / 契约线 / 人类）

| # | 需要谁 | 具体动作 | 不做会怎样 |
| :--- | :--- | :--- | :--- |
| N1 | **集成者 + 契约线** | 把 `SetAutomationLane` / `RemoveAutomationLane` 加进 `schemas/ops.schema.json` 的 `op.oneOf`（27 → 29 个分支，每个分支照现有格式 `required: [<名>]` + `properties`），并**同时**把 `crates/yeban-model/src/ops.rs` 里的 `PENDING_CONTRACT_OPS` 清成 `[]` | 契约缺两个分支；判据里的显式欠账会一直挂着（`run-gates.sh` 会提醒"若契约已补齐, 请把该清单清空"） |
| N2 | **人类** | 追认 §4.2 的 A1–A7（尤其 **A7：`Op` 全集 27 → 29**），按 D12/D27 的方式回写规范 §6.1；并在 `docs/ledger/human-decisions.md` 里补一行 `HD-42`（本线**不得**改该文件） | 与 `HD-01`/`HD-12` 同族：规范正文与实际 `Op` 全集继续分叉，下一条线仍会以为"只有 27 个" |
| N3 | **人类 / 规范线** | 若要求 `Exponential`/`Logarithmic` 是**真** `exp`/`log`，需修订规范给出公式（并接受 D32 的 ulp 预算）；否则把 §4.2 A1 的四项多项式写进规范正文 | 公式只活在代码注释与判据里 |
| N4 | **设备/插件线** | `ParameterValue` 需要一个"合法区间 + 量纲"字段，`AutomationTarget::DeviceParam::nominal_domain()` 才能从 `None` 变成真值；在此之前界面必须按曲线最值自适应 | 设备参数泳道的纵轴刻度只能自适应 |
| N5 | **`yeban-mcp` 线** | 本线**没有**新增 `ModelError` 变体（刻意复用 `TrackNotFound` / `RoutingEdgeNotFound` / `DeviceSlotOutOfRange` / `ParamIndexOutOfRange` / `MacroIndexOutOfRange` / `OpStateMismatch` / `NonFiniteValue`）⇒ `code_for_model` **无需改动**。若将来要一条"区间反了/缺字段"的专用码，需要新增变体**并同步**它的穷举 match | 目前无反了区间这种状态（类型不变量），无需动作 |
| N6 | **`line/mcp-render`** | 接 `automation_value_at` 后，把 `render.rs` 的 `automationLanes` 从 `unsupported` 移出（该注释的前提"没有求值"已被本线消除） | 能力矩阵会继续谎报"不支持" |
| N7 | **`yeban-app` / `yeban-engine` 线的所有者（集成者指派）** | 修 `crates/yeban-app/tests/live_ui_mcp.rs:892` 的 `admin_reload_engine_rebuilds_and_resets_the_meter_tap`：真实合成接上后，"换代后电平必须回到下限"的前提已不成立（**main 上就以 `left: -6.0 dBFS` 逐字红着**，run `37244018311`）。要么改断言为"新引擎的真实读数取代注入帧"，要么修 `ui/reload_engine` 的读数作废语义 | **所有**工作线的 CI 判决都会带着这个红点，合入判断会被污染 |

## 10. pending / TODO(hoist)

| 项 | 说明 |
| :--- | :--- |
| `PENDING_CONTRACT_OPS`（`crates/yeban-model/src/ops.rs`） | 已知欠账（N1）；它是**机器校验**的，不会腐烂 |
| `TODO(hoist)`：`AutomationLane::value_at` 是 O(n log n) | 热路径需要 tick 索引 —— 已给 `points_in_tick_order()`，索引实现属于**引擎线**（并受 `RSK-31` 的 Ramer-Douglas-Peucker / 16 采样点约束） |
| `TODO(hoist)`：`ARCH-DSP-001` 的 5 ms 单极点平滑 | 模型层**不做**平滑（模型不该有时间常数），由引擎/DSP 在自动化值之后施加 |
| `PENDING`：跨架构逐位 | 本机只有单架构读数；按 D7/D32 记 PENDING，等 `MUST-GATE-003` 的 ARM 腿 |
| `PENDING`：`HD-42` 行 | 本线不得改 `docs/ledger/human-decisions.md`（见 N2） |
| `PENDING`：CI 判决 | 代码提交 `2a94a5f` 的 run `37244333377` = **failure**，唯一红点是 main 上逐字复现的预存在红（§8.2 / N7）；本线不把它算作通过，也不擅自改别人的 crate。文档提交 `21566c9` 的 run `37244665346` = success 但**六条 rust 腿被 `plan` 跳过**（§8.3）⇒ 不构成代码验证 |
