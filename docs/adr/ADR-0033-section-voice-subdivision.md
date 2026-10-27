# ADR-0033 — 理论段和声声部的节奏切分（`voiceSubdivision`）

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: `MCP-TOOL-005`（`yeban_propose_section`）、`ARCH-DET-001`（L1 确定性）、
  `ADR-0016`（`materialSource` 缺省逐字节兼容契约）、`ADR-0017`（鼓/旋律追加）、
  `ADR-0028`（音域修正 + `drumDensity`，本票沿用其"缺省逐字节不变"口径）；
  `crates/yeban-mcp/src/domain/section_build.rs`（`theory_contents`、`PlanOptions`、
  `plan_with_options`）、`crates/yeban-mcp/src/tools.rs`、`schemas/mcp-tools.schema.json`
- **日期**: 2026-10-27

## 背景

2026-08-20 的 MCP 创作链路审计（G8）实测：`materialSource:"theory"` 的**和声声部**
（3/4 声部 voicing，含 bass）每个 `ChordSpan` 只生成**一条满跨度的持续音**。审计读数
（3 段 × 3 小节工程）：和声声部时值＝整跨（约 3840 tick @ 960 PPQ）、力度恒 96、
每小节每声部恰好 1 个击点。`ADR-0028` 修好了**音域**与**鼓密度**，但**和声的节奏身份**
仍然不可调：ostinato、定音鼓脉冲、BRAAAM 这类"有节奏的持续音"只能生成后逐音符
`edit_notes` 手写 —— 而 `edit_notes` 永远走提案，两轮往返。

代码事实（`section_build.rs`）：

- `theory_contents()`（`:1284-1348`）对每个 `ChordSpan` 建一条
  `MidiNote::new(id, span.start_tick, pitch, span.duration_ticks)`，力度恒
  `THEORY_VELOCITY`（`:162` = 96）；id 种子
  `"theory-note:{section_id}:{voice}:{chord}"`（`:1338`）。
- 鼓（`drum_contents()` `:1395-1444`）与旋律（`melody_contents()` `:1450-1483`）
  各自已有节奏表（`THEORY_ONSETS_PER_BAR` = 16 的网格），**本票不碰它们**。

## 决定

### D1 新可选整数参数 `voiceSubdivision`（缺省 1 = 逐字节不变）

`yeban_propose_section` 新增可选整数参数 `voiceSubdivision`，取值 **1..=16**，
缺省 `1`。它把每个 `ChordSpan` 均匀切成 `N` 条同音高音符：

- `step = span.duration_ticks / N`（整数 tick，⛔ 无浮点）；第 `i` 条起点
  `span.start_tick + i * step`；**最后一条吸收余数**（`duration - (N-1)*step`），
  ⇒ 跨度总时长逐字节守恒，不产生跨段缝隙或重叠。
- 力度全部沿用 `THEORY_VELOCITY`（96）。⛔ 本票不引入重音模式（见 D4）。
- 只作用于 `'theory'` 路径的和声声部；鼓/旋律/`'project'` 材料路径**不受影响**。

**为何是这个形状**：审计要的是"能生成有节奏身份的铺底"。均匀切分是 ostinato /
脉冲的最小可用形状，且完全确定性；而"节奏型查表"要新造一张表并定义它与和弦跨度、
拍号的映射 —— 那是另一个量级的规范缺口，不该混进本票（D4 登记）。

### D2 命名：⛔ `onsetsPerBar`

`onsetsPerBar` 这个键名**已被占用**——它是 `GenreRule::drum_pattern` 失败时错误载荷里
的键（`section_build.rs:1384`，值为内部 16-grid 常量 `THEORY_ONSETS_PER_BAR`）。
若把参数也叫 `onsetsPerBar`，同一个词在同一 crate 里就有了两种单位（每小节 vs 每跨度），
读错误码的人会算错。故取名 `voiceSubdivision`（单位：**每和弦跨度的切分次数**），
参数说明与判据都用"每跨度 N 条"的措辞。

### D3 缺省逐字节不变的机械口径（含 id 种子）

`ADR-0016` 的缺省兼容契约要求 `N` 缺省/`N=1` 时输出与今天**逐字节相同**。因此：

- id 种子**只在 `N > 1` 时**追加切分序号：`"theory-note:{section}:{voice}:{chord}:{part}"`；
  `N == 1` 沿用现行两段式种子。⚠ 这是**有意**的条件分支 —— 若无条件追加 `:0`，
  所有音符 id 会变，工程哈希变，`ADR-0016` 的缺省判据与一批 pin 住读数的测试全部无谓翻动。
- 参数**给了就必须被消费**：`voiceSubdivision` 与 `materialSource: "theory"` 不同给时
  响亮拒绝（`INVALID_PARAMETER_RANGE` + `data.hint`），与 `ADR-0028` 处理
  `drumDensity` 的同一口径（`plan_with_options:1573-1581`）。
- 越界（`0` 或 `>16`、非整数）同样 `INVALID_PARAMETER_RANGE`，`data` 带
  `{"voiceSubdivision": 实际值, "range": [1, 16]}`；整数上下界写在 domain 层
  （`ParamSpec` 只有字符串 min/max），与 `yeban_render_master` 的
  `TARGET_LUFS_RANGE`（`domain/render.rs:567`）同一先例。

### D4 不在本票范围（登记，不发明）

1. **每声部独立粒度**（例：bass 切 8、pad 持续）——需要"按声部名/序给值"的形状选择，
   留给后续票。
2. **非均匀节奏型（贝斯节奏型表）**——需要新数据表与规范口径。
3. **切分内的重音模式**——会改动力度面，与 `ADR-0026` 分级、`fault_vocabulary` 冻结面
   都要重新核对，故本票固定"力度恒 96"。

## 判据（落地时随代码一起提交）

1. `voice_subdivision_default_is_byte_identical`：同一工程同一参数两次调用
   （一次缺省、一次 `voiceSubdivision: 1`）生成的提案 payload **逐字节相同**，
   且与 `ADR-0028` 之后的既有缺省读数相同。
2. `voice_subdivision_partitions_the_span_without_loss`：对每个声部，
   `N ∈ {2, 3, 4, 6, 7, 16}` 时击点数 == `N × 跨度数`，且
   `Σ duration == 原跨度时值`、相邻音符 `start_{i+1} == start_i + duration_i`
   （无缝无叠）、音高与 `N=1` 时逐声部相同。
3. `voice_subdivision_bounds_are_loud`：`0` / `17` / 非整数 ⇒ `INVALID_PARAMETER_RANGE`；
   缺 `materialSource: "theory"` 时同码 + `hint`（给了不消费 ⇒ 拒绝）。
4. `voice_subdivision_does_not_touch_drums_or_melody`：`includeDrums` /
   `includeMelody` 的音符表在 `voiceSubdivision` 取 1 与取 8 时**逐字节相同**。
5. `ARCH-DET-001` 面：固定输入下两进程/两次调用的 id 与 payload 逐字节一致
   （既有确定性判据自动覆盖，本票只加一条针对新种子的读数）。

## 后果

- `payload_budget` BASELINE-006 的缺省读数**不变**（缺省逐字节不变）。
- 新增判据落在 `crates/yeban-mcp/tests/`（或 `section_build.rs` 的 `#[cfg(test)]` 模块），
  ⛔ 不改动 `verify/section_pure.rs` 已 pin 的缺省读数。
