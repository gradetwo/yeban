# ADR-0040 — 生成和声的声部跳进上界：把 `VoicingConstraints.max_voice_jump` 开放为 `harmonyMaxVoiceJump`（缺省逐字节 = theory 出厂的 12）

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent，参考 Logic Pro / 通行和声学里"声部连接不得大跳"的口径自行裁决）
- **关联**: `MCP-TOOL-005`（`yeban_propose_section`）、`ADR-0016`（缺省逐字节兼容契约）、
  `ADR-0028`（和声声部的**音域** `SECTION_THREE_VOICES` —— 本票⛔ 动它，只动**跳进**）、
  `ADR-0038`（旋律音域）、`ADR-0039`（旋律跳进；其非范围 `N1` 逐字写着"和声各声部走
  `voice_leading` 的自己的 `max_leap`，本票只碰旋律线"⇒ 本票就是那条 `N1` 点名的另一半）、
  `ADR-0037`（"给了必须被消费"的耦合口径，本票取它那条**较松**的：**不**要求 `includeMelody`）、
  `ARCH-DET-001`
- **实现落点**: `crates/yeban-mcp/src/domain/section_build.rs`（`PlanOptions`、`harmony_leap`、
  `theory_contents`、`plan_with_options` 的消费校验）、`crates/yeban-mcp/src/domain/mod.rs`
  （`arg_u64` 读取）、`crates/yeban-mcp/src/tools.rs`、`schemas/mcp-tools.schema.json`；
  `crates/yeban-theory`（**只读其既有公开面，本票不改它**）
- **日期**: 2026-10-28

## 背景（先测后改，AGENTS.md §6）

旋律那两票（`ADR-0038`/`ADR-0039`）把 `MelodyConstraints` 的三个字段全部接成了入参；
和声侧的对应物 `VoicingConstraints`（`crates/yeban-theory/src/voice_leading.rs:62`
`{ ranges, max_voice_jump: i16, max_total_movement: i16 }`）在 MCP 侧仍有两个写死的数字：
3 声部路径的 `max_voice_jump: 12, max_total_movement: 24`（`section_build.rs:1615-1616`）
与 4 声部路径的 `VoicingConstraints::FOUR_VOICES`（出厂同样是 12，`voice_leading.rs:82`）。
⇒ 和声的**声部连接**是这条生成线上最后一处"理论层有杠杆、工具面碰不到"的地方。

本轮先量（本机一次性探针 crate，路径依赖 `yeban-theory`，⛔ 重依赖、⛔ 不提交；
音域固定成 MCP 生成路径实际用的 `SECTION_THREE_VOICES`＝`C2–C3 / G3–G5 / C5–C7`、
`max_total_movement` 固定 24，只换 `max_voice_jump`；主音 `C` 自然小调；
单位＝**和弦层数（个）/ 相邻两和弦同一声部的半音距离 / 不同音高（个）/ 与缺省逐层不同的层数（个）**）：

| 进行 | `max_voice_jump` | 层数 | 实测最大跳进 | 不同音高 | 与缺省不同的层 | 判定 |
|---|---|---|---|---|---|---|
| `i-VI-III-VII` | 12（出厂） | 4 | **3** | 7 | 0 | 成功 |
| `i-VI-III-VII` | 5 / 127 | 4 | 3 | 7 | 0 | 成功（与缺省逐层相同） |
| `i-VI-III-VII` | 2 | 4 | **2** | 6 | **2** | 成功（真的改了音高） |
| `i-VI-III-VII` | 1 / 0 | — | — | — | — | **`NoFeasibleVoicing`** |
| `i-vii-i-vii` | 12 / 5 / 2 / 127 | 4 | 2 | 6 | 0 | 成功 |
| `i-vii-i-vii` | 1 / 0 | — | — | — | — | **`NoFeasibleVoicing`** |
| `i-i-i-i` | 12 → 0（六档） | 4 | **0** | 3 | 0 | 全部成功，含 `0` |

四条读数各自的含义（前三条**推翻**了"照抄旋律判据"这条路）：

1. **层数恒 4** ⇒ 跳进上界⛔ 不筛和弦、⛔ 不改时值 —— 与旋律两票同一条形状，判据照抄。
2. **缺省 12 在真实进行上只打到 2–3**（⛔ 旋律那条"缺省就打满"）⇒ 本票的判据⛔ 写
   "实测跳进 == 入参"，只能写 **≤ 入参** ＋ "足够紧的值确实改变输出"（`2` 那行：两层不同）。
   这条差异是**实测定下来的**，不是措辞偏好。
3. **可行性边界随进行移动**：`1` 与 `0` 在 `i-VI-III-VII`、`i-vii-i-vii` 上红，`0` 在
   `i-i-i-i` 上**绿**（同和弦反复 ⇒ 零移动合法）⇒ ⛔ 把"某个值必红/必绿"钉成常量，
   也⛔ 在本层抄一张"进行 × 跳进上界"可行性表 —— 那是 theory 的 `NoFeasibleVoicing`。
4. **比实际所需更松的取值不改变输出**（`5`、`127` 与缺省逐层相同）⇒ 这是"上界"的
   正确语义（⛔ "设了不一定生效"的缺陷），文档串要把它写成语义而⛔ 写成警告。

## 决定

- **D1 — 新增可选键 `harmonyMaxVoiceJump: integer`，取值 `0..=127`，缺省 = theory 出厂值**
  ⇒ 键缺席与键给 `12` 必须**逐字节**产出同一份 `SectionPlan`（`ADR-0016`）。
  ⚠ **缺省不写数字常量**：`None` 时本层**不覆盖** `VoicingConstraints` 的该字段，
  于是 3 声部取 `THREE_VOICES`、4 声部取 `FOUR_VOICES` 各自的出厂值 ⇒ 界只有**一个写者**
  （theory），MCP 侧⛔ 再抄一个 `12`。单位 = **相邻两个和弦之间、同一声部的半音距离**
  （⛔ MIDI 音高、⛔ 八度号、⛔ 旋律线里相邻两音 —— 那一个是 `melodyMaxLeap`）。
- **D2 — 上界取 `127`**：单个声部的两个音都在 `0..=127` 内 ⇒ 半音距离在任何输入下都
  ⛔ 可能超过 127（`ADR-0039` D2 同一条理由）。⚠ 与旋律那一票不同的是这里**实测就有一档打不到**
  （读数 2）——"打不满"是**上界**这个概念的正常性质，⛔ 因此收紧到 12/24：那会把 theory
  的出厂语义换成第二张表。`0` 合法（同和弦反复，读数 3）。
- **D3 — 消费校验取 `ADR-0037` 那条**（较松的一支）：和声声部在 theory 路径上**恒**在场、
  恒被消费，因此只要求 `materialSource:"theory"`，⛔ **不**要求 `includeMelody` / `includeDrums`
  （那两条与 `melodyMaxLeap` 的"必须有旋律声部"不同口径，理由是消费点不同 —— 见 `ADR-0038` D3
  对照）。⛔ 把三个旋律键的 `register_given` 条件与这个键折成**同一个** `is_some()` 表达式：
  它们的要求不同（旋律三键多一条 `includeMelody`），折在一起就会把旋律那条更严的要求
  扩散到和声键上，产生一个⛔ 文档里没写的拒绝。
- **D4 — 界只判一次**：`0..=127` 复用 `ADR-0039` D4 提到模块级的 `midi_pitch_bound`
  （⛔ 第四个写者）；"这个上界在当前进行里可不可行"⛔ 由本层预判 —— 命中时由
  `voice_leading::realize` 返回 `NoFeasibleVoicing`，既有转发链 `theory_invalid(style,
  "voice_leading::realize", error)` 原样带出（读数 3：边界随进行移动，本层没有那张表）。
- **D5 — 3 与 4 声部两条路径都接线**（`VoicingConstraints` 的 struct-update 只换
  `max_voice_jump`，各自的 `ranges` 与 `max_total_movement` 保持出厂）：只给一条路径接线
  会让**同一个键**在两种风格预设下有不同语义（一种收紧、一种静默用出厂值），那是比
  "没做"更糟的形态。⚠ 顺带把 3 声部那处的字面量 `max_voice_jump: 12, max_total_movement: 24`
  改成 `..VoicingConstraints::THREE_VOICES`（实测二者逐字段相同 ⇒ 构造结果不变，
  而"12/24 这两个数字"从此只有 theory 一个写者）。
- **D6 — `max_total_movement` 不在本票**：它是**软约束**（theory 文档："用于报告未达标
  而⛔ 直接失败"），把它接成入参会造出一个"给了可能不生效且⛔ 报错"的键 —— 与读数 4
  那种"上界打不满"是两类事情，需要自己的裁决与判据。已登记为非范围 `N2`。
- **D7 — ⛔ 新增故障词**：拒绝载荷⛔ 带 `reason` 键，形状与力度/音域/跳进三族逐字同族 ⇒
  `fault_vocabulary` 的词表与规范面摘要不变。

## 非范围（登记，⛔ 补齐）

- `N1` **和声声部的音域**（`SECTION_THREE_VOICES`）—— `ADR-0028`/`ADR-0038` 已两次裁过：
  那是本层为生成路径**特意**偏离 theory 出厂音域的一份常量，入参化它等于把那次裁决
  交给调用方，需要新的裁决。
- `N2` **`max_total_movement`（总移动量上界）** —— 见 D6，软约束需要自己的语义与判据。
- `N3` **逐声部的跳进表**（本票一个数管所有声部，`voice_leading` 的字段本身也是单个 `i16`）。
- `N4` **旋律动机** —— 沿 `ADR-0039` N4，仍等人类裁决。

## 判据（`crates/yeban-mcp/tests/section_harmony_jump.rs`，6 条）

1. 键缺席 ≡ 给 `12` ⇒ `SectionPlan` 逐字节相同；⛔ 断言"缺省实测跳进 == 12"（读数 2 会把它打红）。
2. 收紧到 `2` ⇒ **层数不变**、旋律与鼓**逐字节**相同、和声声部 id/起点/时值逐条对齐、
   同一声部相邻两和弦的半音距离 ≤ 2、**至少一层音高被改**（防空判据：读数 2 说缺省只打到 3，
   所以"≤ 入参"这一条单独存在时可能永远绿）、力度仍取阶梯两档。
3. 越界（`128`）⇒ `INVALID_PARAMETER_RANGE`，载荷点名 `field:"harmonyMaxVoiceJump"` + `value` +
   `range:[0,127]`，⛔ `reason` 键；端点 `0` 与 `127` ⛔ 被本层拒（`0` 在缺省进行上会转成
   判据 5 那一支，⛔ 参数层的拒绝 —— 这条断言只看码）。
4. 耦合：⛔ `materialSource:"theory"` ⇒ 拒且 `hint` 点名 `materialSource`；
   ⚠ **反钉**：`includeMelody:false` 且 `includeDrums:false` 时给这个键**必须放行**
   （D3 的较松口径，钉住"⛔ 把旋律那条更严的要求扩散过来"）。
5. 极紧上界 ⇒ 由 theory 判定、本层**原样转发**（`theoryStep:"voice_leading::realize"` +
   `stylePreset` + `theoryError` 字符串），⛔ 本层抄可行性表、⛔ 静默放宽或产出空和声。
6. 两个声明面（`tools.rs` 的 `param` 与 schema 的 property）同源：类型 `integer`、
   `minimum 0`、`maximum 127`；且 schema 的 `default` 与 theory 出厂值 `THREE_VOICES.max_voice_jump`
   是**同一组数字**（⛔ 各写一份）。
