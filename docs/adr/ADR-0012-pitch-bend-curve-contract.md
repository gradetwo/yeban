# ADR-0012 — 弯音曲线（`MidiNote::pitch_bend_curve`）的坐标、形状与校验

- **状态**: **Accepted（2026-10-25 裁决；D4 改为「首点之前 = 0」，见「裁决记录」）**
- **决定者**: Agent（`#311a` 起草，2026-10-25 由集成线**依 Logic Pro 裁决**，人类授权「自己参考 Logic Pro 来裁决」）
- **关联**: `MODEL-AST-005`（`MidiNote` 结构）；`#311b`（DSP 内核）／`#311c`（门面 ＋ 判据）；
  `crates/yeban-model/src/music.rs` 的 `PITCH_BEND_CENTS_MAX_ABS` 与
  `MidiNote::validate_pitch_bend_curve`（本 ADR 的机械载体）；
  `#311` 的勘察（`KNOWN_UNSUPPORTED_KEYS` 里 `notePitchBend` 今天只登记、不求值）
- **日期**: 2026-10-24

## 背景

`MidiNote::pitch_bend_curve: Vec<(u64, i16)>` 自 `MODEL-AST-005` 起就在持久化 AST 里，
而它**只有一句注释**：`(tick, cents)` 序列，按 tick 升序；空表示无弯音。

四处缺口（逐条实测，不是推断）：

1. `MidiNote::validate()` **一条都没查**它 —— `pitch` / `velocity` / `probability` / `ratchet` /
   `micro_timing_ticks` / `duration_ticks` / `slide` 都有各自的值域检查，弯音一条没有
   （`crates/yeban-model/src/music.rs` 的 `validate()`）；
2. `schemas/project.schema.json` 里它只是裸整数对（`minItems/maxItems: 2`，无界、无序）；
3. `docs/adr/` 里**没有**任何一篇提到弯音（`grep -rln "pitch_bend\|弯音" docs/adr/` = 0 命中）；
4. 渲染 / 导出**没有**任何求值点：`yeban_render_master` 只把它登记进 `unsupported`
   （`crates/yeban-mcp/src/domain/render.rs`），SMF 导出显式"不导出"（`crates/yeban-midi/src/export.rs`），
   Logic / `.als` 导出把它记进"有损"表。

⇒ 今天写进去的任何曲线都是**语义未立法的字节**。`#311b` 要在 DSP 里真的求值它，
因此**必须先有契约**：否则 DSP 一落地就等于把某一种口味固化成"事实"。

**现在收紧是免费的**（先核代码，再下结论）：

- 工具面**没有任何写入通路**：`yeban_edit_notes` 的音符键只有 8 个
  （`id` / `startTick` / `pitch` / `durationTicks` / `velocity` / `probability` / `ratchet` /
  `microTimingTicks`），`pitchBendCurve` 不在其中 —— `crates/yeban-mcp/src/tools.rs` 逐字：
  "模型另有 `slide`/`pitchBendCurve`/`syllable`/`phonemes` 四个表现力字段, 工具面**还没有**通路"；
- `yeban-midi` 里 `grep -rni "bend"`（排除 `pitch_bend_curve` 本身）**0 命中**：
  导入侧不解析弯音，导出侧登记"不导出"；
- 仓内**唯一**的曲线是 3 处夹具：`crates/yeban-model/src/music.rs` 与
  `crates/yeban-mcp/tests/render_master.rs` 的 `[(0, 0), (240, 200)]`、
  `crates/yeban-midi/src/midi.rs` 的 `[(0, 0), (240, 100)]` —— 全部满足下面 D2–D4。

⇒ 今天不收紧，等工具面开了写入之后再收紧就是**破坏性**的。

## 决定

### D1 `tick` 是**音符局部**坐标

`tick = 0` 是**本音符的起点**（不是工程坐标、不是摆放坐标）。`note.start_tick` 的平移、
`micro_timing_ticks`、循环重复（`ClipPlacement::loop_repeat` 的每一遍）**都不改曲线**：
曲线是"音符自己的形状"，跟着音符走。

- 理由：字段挂在 `MidiNote` 上并序列化在音符里；用工程坐标会让"复制音符到另一位置"带出一条错位的曲线。
- 代价：跨音符搬移曲线需要换算（今天没有这种操作）。

### D2 `cents` 相对 `note.pitch`，闭区间 `-1200..=1200`

常量 `PITCH_BEND_CENTS_MAX_ABS = 1200`（±1 个八度，端点含）。

四条理由：

1. `i16` 的原始范围（±32767 ≈ ±27 个八度）在音乐上无意义，不能当值域用；
2. ±1200 = ±1 个八度是"弯音"能到的最宽处；再宽就是在写另一个音符（应当改 `pitch` 或用 `slide`）；
3. MIDI 线格式的默认弯音域是 ±2 半音，但**模型不是线格式**：模型存作者意图，导出映射是另一件事
   （SMF 导出今天不导出弯音）；
4. 仓内既有夹具最大 200 cents ⇒ 留 6 倍余量，收紧不动任何现存文档。

- 代价：想写"±2 个八度的警报器"会被拒。放宽是**兼容**的，收紧才是破坏性的 ⇒ 先松后紧仍安全。

### D3 tick **严格**升序

相邻两点必须严格递增；**同一 tick 出现两次 = 一个时刻两个音高** ⇒ 拒绝。

- 理由：消费侧要对 `(tick, cents)` 做插值查找，重复 tick 让"该点的值"没有唯一答案。
- 代价：想写"瞬时跳变"必须写成相邻两个 tick（`(t, a), (t+1, b)`）⇒ 跳变永远是 1 tick 的线性斜坡
  （@960 PPQ、120 BPM 时 1 tick ≈ 0.52 ms，听感上就是跳变）。

### D4 非空 ⇒ **首点必须 `tick == 0`** ⚠ **已于 2026-10-25 撤销**

> ⚠ **本条结论不再成立。** 现行规则见「裁决记录 → D4」：首点可以是任意 `tick ≥ 0`，
> 首点之前 `cents` 一律取 `0`。下面保留的是**当时为什么要求显式 `(0, 0)`**（留痕用，
> ⛔ 不是现行契约）。

曲线必须从音符起点开始定义。⛔ 不做"首点之前"的外插。

- 理由：外插有两种口味（保持首值 / 视为 0），选任何一种都会让"音符一起音就弯了"变成隐式行为。
  要求显式写出 `(0, 0)` 就没有歧义，也不需要外插规则。
- 代价：想"第 240 tick 才开始弯"要写 `(0, 0), (240, x)` —— 多一个点，但语义唯一。

### D5 相邻点之间**线性**插值（单位 cents）；**末点之后保持末值**

这条是**消费侧**规则（模型不计算音频），写在这里是为了让 `#311b` 没有自由裁量：

- `t ∈ [t_i, t_{i+1}]`：`cents(t) = c_i + (c_{i+1} − c_i) · (t − t_i) / (t_{i+1} − t_i)`；
- `t > t_last`：`cents(t) = c_last`（保持，不外插）；
- 曲线为空：`cents(t) = 0`（等价于无弯音）。

### D6 落在鸣响区间之外的点**允许**，且永不采样

`tick > duration_ticks` 的点**不**算错误。

- 理由（这是一条**刻意的非规则**）：`Op::SetNoteTiming` 能把时值改短；若把"末点 ≤ 时值"写进
  `validate()`，一次合法的缩短就会让**原本合规**的工程变成非法（用户看到"改时值失败"，而错在别处）。
  宁可让"长出来的尾巴"静默无效 —— 它在鸣响区间之外，本来就没有声音可弯。
- 代价：写错的"长尾巴"不报错，只能从曲线本身看出来。

### D7 `validate()` 机械执行 D2／D3，顺序即优先级

`MidiNote::validate()` → `validate_pitch_bend_curve()`，检查顺序：**升序 → `cents`**。
两条各有一个 `ModelError` 变体：

| 变体 | 条件 | MCP 契约码 |
| :--- | :--- | :--- |
| `PitchBendTicksNotAscending` | `tick` 非严格升序（含重复） | `INVALID_PARAMETER_RANGE`（形状不合法） |
| `PitchBendCentsOutOfRange` | `cents` 越出 ±1200 | `OUT_OF_RANGE`（量越界） |

（`PitchBendFirstTickNotZero` 已随 D4 撤销一并**删除** —— 见「裁决记录 → D4」。）

D5／D6 **不可**机械校验：它们是消费侧语义与刻意的非规则。

判据：`crates/yeban-model/src/music.rs` 的 6 条 —— 合法形状正对照、首点**可晚于**起点、
升序（含重复）、`cents` 闭区间端点、**错误顺序**、常量与两条报文逐字；外加 `yeban-mcp` 的
`model_errors_never_escape_the_contract_enum`（变体必须落在契约错误码内，且分组被钉住）。

### D8 破坏面：今天为零，明天为一次

- 工具面没有写入通路、仓内 3 条夹具合规（见背景）⇒ 不破坏任何现存文档；
- `project.schema.json` 仍是裸整数对（见"未决 1"）⇒ "schema 允许、模型拒绝"的文档在读取时被
  `validate()` **响亮**拒绝（而不是静默接受一段没语义的字节），这正是本 ADR 想要的方向。

## 未决

1. **JSON Schema 的 `cents` 界**：`schemas/project.schema.json` 的 `pitch_bend_curve` 现在是
   `items: { type: integer }`（两个元素同一个约束）。只约束第二个元素要改成 `prefixItems`，属结构性改动；
   本切片不动 `schemas/**`（任务单 `#311` §3 的边界）⇒ 留给集成者。
   ✅ **已落地**（2026-10-26）：已在 `schemas/project.schema.json` 实施 `prefixItems`
   （元素 0 为非负 tick，元素 1 为 `-1200..=1200` cents，`items: false, minItems: 2, maxItems: 2`；
   `tests/schema_ratchet.rs` 遍历器已支持并以专用单元测试钉住边界）。
2. **`#311b` 的求值口径**：逐块重算 `phase_inc` 的块长、ramp 的插值形式（线性 / 指数），
   以及 `ratchet` 子脉冲与循环重复在 D1 的局部坐标下是否各自从 0 重放曲线。
   ✅ **已裁决**（2026-10-25，见 **D10** 与 **D11**）。
3. **工具面写入**（`pitchBendCurve` 键）：本 ADR 只立法形状，不给入口；入口要先过 D2–D4 的拒绝面。
4. **SMF 导出的弯音映射**（MIDI pitch-bend 事件 / RPN 弯音域）：`yeban-midi` 今天显式不导出；
   要不要导出是另一票（要处理线格式 ±2 半音默认域与 D2 的 ±1200 的映射）。
5. **`slide` 与 `pitch_bend_curve` 同时存在**时的优先级：两者都能改音高，本 ADR 不裁决（`#311b`）。
   ✅ **已裁决**（2026-10-25，见 **D9**）：**相加**，不是二选一。

---

## 裁决记录（2026-10-25）

负责人授权：「ADR-0012 你自己参考 logic pro 来裁决」。以下三条以 Logic Pro 的实际行为为准绳。
⚠ **可核范围**：本机无 Logic，故"Logic Pro 怎么做的"来自**通行的 MIDI 与 Logic 语义**，
不是逐屏截图；凡不能由仓内代码证明的，下文都写明它是**依据**而非**读数**。

### D2 —— 维持 `-1200..=1200`

**Logic Pro 的行为（依据）**：弯音在 Logic 里是**乐器属性**而非音符属性 —— 每个软件乐器有
「Pitch Bend Range」参数，出厂/通用 MIDI 默认 **±2 半音**，而 Logic 自家的合成器
（ES2／Retro Synth／Sampler 等）常被设成 **±12 半音**，少数到 ±24。

**裁决**：`±1200`（＝ ±12 半音）**保留**。理由：

1. 它**覆盖** Logic 的默认 `±2` 与最常用的 `±12` 两档；
2. `±24` 确实存在，但"超过一个八度的弯音"在音乐上更接近**另一个音高**而不是"弯" ——
   模型里那件事有专门的字段（`slide.target_pitch`），不该让弯音去兼任；
3. 放宽是**兼容**的、收紧是破坏性的 ⇒ 将来真要 `±2400`，改常量 ＋ 三条报文即可（判据会红）。

### D3 —— 维持「tick 严格升序」

**Logic Pro 的行为（依据）**：Logic 的自动化/弯音曲线**不能在同一时间位置存在两个点** ——
把一点拖到另一点上会**替换**而不是并存。

**裁决**：维持严格升序、同 tick 两条**拒**。理由：与 Logic 一致；且"同一时刻两个音高"没有
可判定的解释。代价：瞬时跳变要写成相邻两 tick（960 PPQ/120 BPM ≈ 0.52 ms 斜坡）——那是
**听不出来的**，可接受。

### D4 —— ⚠ **改**：首点**不必**是 `tick == 0`；**首点之前 `cents = 0`**

**Logic Pro 的行为（依据）**：Logic 的弯音是**绝对状态**，不是"从某个点开始的曲线" ——
在第一条弯音事件之前，弯音处于**默认值（中心＝0 音分）**。所以"从第 240 tick 才开始弯"
在 Logic 里就是"前 240 tick 保持 0"。

**裁决**：`D4` 由「非空 ⇒ 首点必须 `tick == 0`」改为：

> **首点可以任意 `tick ≥ 0`；`tick` 小于首点的部分，`cents` 一律取 `0`。**

理由：

1. 与 Logic 的**绝对状态 ＋ 默认值**语义一致（原 D4 要求显式写 `(0,0)`，比 Logic **更严**）；
2. 少一条校验、少一个"为什么要我写一个 0"的问题；
3. `(0, x)` 依然可表达"音符一起就弯"，表达力不减。

**代价（如实记）**：`#311a` 已落地的 `PitchBendFirstTickNotZero` 变体、报文与两条判据要**撤掉**，
并新增一条"首点之前取 0"的判据。这是本裁决唯一要改代码的地方，记在 `#332`。
⚠ **实际由 `#311b` 一并做完**（撤变体＋撤两条判据＋新增"首点之前取 0"），`#332` 未单独开单。

### 由此产生的下游

- `#311b`（DSP 内核）**解锁**：三条已定，内核可以写。
- `project.schema.json` 的 `prefixItems`（原"未决 1"）：`cents` 上界随 D2 定为 `±1200`，
  首点约束随 D4 改为**无**。✅ **已落地**（2026-10-26）：已在 `schemas/project.schema.json` 用 `prefixItems` 实施。

### 仍未决（不在本裁决范围）

- **D5 的播放侧细节**：逐块重算 `phase_inc` 的块长、ramp 形式、ratchet／循环是否重放（`#311b`）。
  ⚠ **当时未决，现已决** → 见下方「裁决记录之二」的 **D10**／**D11**。
- **`slide` × `pitch_bend_curve` 同时存在时的优先级**（`#311b`；两者都能改音高）。
  ⚠ **当时未决，现已决** → 见下方「裁决记录之二」的 **D9**。

---

## 裁决记录之二（2026-10-25，`#311b` 已落地）

授权同上（「遇到需要裁决的，参考 logic pro 自己裁决」）。下面三条是 `#311b` 写内核前必须先回答的
三个问题；它们现在**已由代码实现并由判据钉住**，故从"未决"移入"决定"。

### D9 `slide` 与 `pitch_bend_curve` 同时存在 ⇒ **相加**，不是二选一

**Logic Pro 的行为（依据）**：Logic 的弯音是**乐器级的频率偏移**，滑音（Portamento）是
**音符级的目标音高过渡** —— 两者作用在链条的不同环节，因此同时打开时是**叠加的**：
弯音把"已经滑到目标音的那个声部"再整体推偏。Logic 没有"弯音优先于滑音"这类仲裁规则。

**裁决**：`cents(local) = bend_cents(local) + slide_cents(local)`；两者单位同为音分，直接相加。
滑音的形状用**模型层的** `CurveType` 表示（`Linear`／`Exponential`／`Logarithmic`／`SCurve`
四档，默认 `Linear`），而曲线的数学只有 `CurveType::ease` **一份**实现 —— `#311b` **不**在 DSP
里复制一份；滑音的值由引擎的 `SlideSpan::cents_at` 算好再相加。

- 实现落点：`crates/yeban-engine/src/synth.rs` 的 `PitchShape::cents_at`
  （`bend_cents_at + slide_cents_at`）。
- 判据：`crates/yeban-engine/tests/pitch_bend.rs` 的 `slide_and_bend_add_up`（`[B4]`）——
  弯音 `+1200` 与滑音 `+1200` 同时开 ⇒ 末段主频 **`1760.000 Hz`**（440 × 2^(2400/1200)），
  同段 880 Hz 幅度 ≈ `2e-9`；`a_slide_reaches_the_target_pitch`（`[B3]`）单独给 `880.000 Hz`。
- 代价（如实记）：一个"弯音推到顶、滑音也到顶"的音符能到 **±2400 音分（两个八度）**，
  这是 D2 的 `±1200` 上界**管不到**的 —— D2 约束的是曲线上的 `cents` 值，不是总和。
  没有为总音分再设一个上界：它是两条合法输入的合法叠加，再设界会变成"两条都合法却合起来非法"。

### D10 `ratchet` 子脉冲与循环副本 ⇒ **各自从 `local = 0` 重放曲线**

**Logic Pro 的行为（依据）**：Logic 的 ratchet（音符重复／滚奏）是**重新触发**同一个音符，
弯音曲线跟着音符走 ⇒ 每次重新触发都从曲线头部开始。循环片段里的每个副本也是独立的音符实例。

**裁决**：曲线的横轴是**音符局部**样本数（D1），而局部原点是**声部自己的 `start_sample`**：

```rust
let local = now - voice.start_sample;   // crates/yeban-dsp/src/polysynth.rs
```

⇒ 任何重新触发（ratchet 子脉冲、循环副本、声部被窃取后提升的新音符）都自然从 `local = 0`
开始 ⇒ **重放曲线**，不需要额外分支。这**不是一条新规则，是 D1 的机械后果**；写在这里
是因为它是 D1 唯一一处会让听感差异显形的地方。

- 判据：投影层读数（`NoteSchedule::cents_at(shape, 0)` 对每个副本／子脉冲都落在"首点之前"）。
- ⚠ **如实登记未证的部分**：`#311b` **没有**为 ratchet 做音频级 A/B —— `[B1]`–`[B4]` 全部用
  单副本。所以"每个子脉冲真的各自从 0 重放"目前是**代码结构上的结论 ＋ 投影层读数**，
  不是母带频谱上的读数。

### D11 控制率 = **音符局部 64 样本网格**，⛔ 不是渲染调用块

**这条不是 Logic 的语义，是离线确定性的硬约束**，所以它的依据在仓内：
`crates/yeban-render/src/engine_source.rs` 已提交的判据要求**同一工程在 64／128／256 帧三种
切法下渲染出逐位相同的字节**。若把 `phase_inc` 的重算点钉在"每个渲染调用块的开头"，
块长一变结果就变 ⇒ 那条判据必然红。

**裁决**：`inc` 只在 `local % 64 == 0` 的帧重算（`PITCH_UPDATE_SAMPLES`），相位保持连续。

- 代价（如实记，这是本内核**最弱的一处**）：控制率 **0.75 kHz**（48 kHz ÷ 64）⇒ 极快的弯音是
  **3.2 音分/步**的频率阶梯，理论上可能有可闻"拉链"。`#311b` 只有**稳态**读数
  （`末段主频 880.000 Hz`），**没有**对渐变段的时频／听感判据 ⇒ 登记在案、未证。
- 逐位无损的三条（判据钉住）：曲线为空或声部无形状 ⇒ **一次**浮点运算都不加
  （空曲线母带 `sha256` / `masterDigest` 与加这个内核之前**逐字节相同**）；窃取淡出窗口内不更新；
  `voice.freq_hz` 永不改写（`note_off` 的配对键与 `retune` 都还是 base 频率）。
- ⚠ 另一个未做判据的窗口：**换采样率时 `retune` 会先按未弯的 base 频率算一次**，
  到下一个网格点才纠正 ⇒ 一段 ≤64 样本（≈1.33 ms @48 kHz）的未弯窗口。已登记、未证。
