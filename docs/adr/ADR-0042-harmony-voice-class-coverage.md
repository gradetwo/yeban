# ADR-0042 — 和声生成的**音级覆盖性**：`voice_leading::realize` 是否保证"和弦每个构成音都被发出"（⚠ 当下⛔ 保证，本票把它变成人类裁决）

- **状态**: **Proposed**（待人类批准；⛔ 集成者代签 —— 本票不改动任何代码，只把实测的缺口与三条可选路线登记成可批可驳的裁决）
- **决定者**: 待人类裁决（集成者提出）
- **关联**: `MCP-TOOL-005`（`yeban_propose_section`）、`ADR-0028`（和声寄存器 `SECTION_THREE_VOICES`）、
  `ADR-0040`（和声声部跳进；其 `N1` 只碰跳进⛔ 碰音域与声部数）、`ADR-0039`（旋律跳进）、
  `ARCH-DET-001`、`MODEL-AST-001..005`
- **影响面（等这张票的两张现成缺口票）**: `harmonyVoices`（和声声部数）与 `harmonyLower`／`harmonyUpper`
  （和声音域）。二者⛔ 在覆盖性确立前上线 —— 见下面的实测。
- **实现落点**: ⛔ 本票**无**代码落点。若批准 D1／D2，落点在 `crates/yeban-theory/src/voice_leading.rs`
  （`candidate_voicings` / `recurse`）；若批准 D3，则⛔ 有落点（明确不做，只登记）。

## 背景（先测后改，AGENTS.md §6）

**这个数字在数什么**：`voice_leading::realize` 每层返回的**声部数**、该层实际覆盖的**音级**
（`PitchClass::semitones()`，单位 `0..=11`）、以及相对该层和弦音级集的**缺失音级**。
量法＝仓库外一次性 probe（⛔ 不入库），链接本机已编好的 `libyeban_theory` rlib。

现读 `crates/yeban-theory/src/voice_leading.rs` 的机制：

1. `realize` **没有**独立的"声部数"参数 ⇒ 声部数 ≡ `constraints.ranges.len()`，
   出厂只有 `THREE_VOICES`／`FOUR_VOICES` 两个 const（各带 `DEFAULT_{THREE,FOUR}_VOICES`）。
2. `candidate_voicings()` 只要求 `tones.len() >= ranges.len()`，而 `tones` 是和弦的
   **绝对音高**去重集 ⇒ 三和弦配 4 声部**不会**触发 `TooManyVoices`（八度重复即可满足）。
3. `recurse()` 要求每个声部在**自己音域内**取一个严格上行的和弦音 ⇒ "每个声部都是和弦音"
   是**由构造成立**的（取值集合就是和弦的绝对音高集 `tones`）。
   ⚠ 本仓唯一的**相关测试**是 `voice_leading.rs:413` 的 `every_voice_stays_on_a_chord_tone`
   —— 它在 `mod tests`（起始 `:371`）里、⛔ 不是产品码强制，且只跑 **3 声部 × `ii-V-I-vi`**。
   ⭐ 而**全仓库没有任何一条断言"和弦的每个音级都被某个声部发出"**（覆盖性）—— 这是实测的第二条结论。

实测（调 C minor，`i-v-i-VI` 三和弦与 `i7-v7-i7-VI7` 七和弦，各 4 小节）：

| 输入 | 结果 |
| --- | --- |
| 三和弦 × `THREE_VOICES` | 层 0..3 全 `缺=[]` |
| 三和弦 × `FOUR_VOICES` | 层0 `音=[60,67,79,84]` 类=`{0,7}` **缺=[3]**；层2 同；层3 类=`{0,8}` **缺=[3]** |
| 七和弦 × `THREE_VOICES` | 层0 **缺=[4]**、层1 **缺=[5]**、层2 **缺=[4]**、层3 **缺=[3]** |

⇒ 症状不是报错而是**静默丢音级**（三音／七音被八度重复的根音与五音挤掉）。这比"会报错"更坏：
任何"靠错误兜底"的设想都不成立，调用方⛔ 从返回值里看出被丢的是什么。

## 为什么这就是那两张票的阻塞

`crates/yeban-mcp/src/domain/section_build.rs` 的 `genre_voice_count()` 现从
`chord.pitch_classes().len()` **派生**声部数 —— 这条对齐就是上表第一行 `缺=[]` 的原因，
也是本仓库**唯一**的覆盖性防线（其策略注释明文"⛔ 本地表"）。

- `harmonyVoices`（把声部数开放为入参）＝**拆掉这条防线**：调用方一旦给出与和弦音数不同的声部数，
  拿到的就是上表那些丢三音的和声，而 MCP 侧⛔ 表达"这次丢的是三音"。
- `harmonyLower`／`harmonyUpper`（把和声音域开放为入参）另有一道钉：
  `ranges: &'static [VoiceRange]` ⇒ MCP 侧只能在**编译期**备表；逐 N 的变长音域＝又一张本地表，
  撞 `genre_voice_count()` 既有口径。

## 待裁的三个选项（⛔ 集成者代签）

- **D1 — 给 `realize` 加"音级覆盖性"硬性约束**：候选声位必须在**至少一个八度**里覆盖和弦的
  每个音级，否则按既有词汇报 `NoFeasibleVoicing`。
  代价＝可行性面收窄（实测三和弦 × 4 声部里层0/2/3 那类解会被淘汰），可能让某些既有的
  默认进行从"能出"变"不出"；收益＝`harmonyVoices` 才可能安全接线。
- **D2 — 覆盖性作为`可选`约束**（`VoicingConstraints` 加一个开关，出厂保持关闭）：
  逐字节兼容（`ADR-0016`）不破，入参选择权交给 MCP 侧。代价＝"同一个和弦有两种口径"，
  账本与判据都要翻倍。
- **D3 — 明确不做**：把"和声声部数与音域"永久钉在 `genre_voice_count()` 的派生口径上，
  `harmonyVoices`／`harmonyLower`／`harmonyUpper` 三张票登记为**有意不做**（同 `ADR-0041`
  的 `N3` 那条处理法）。收益＝缺口清单收敛为"已裁"，代价＝和声的"厚度/音域"这项创作自由度
  ⛔ 交付。

⚠ 本票⛔ 替人类选 D3 之外的任何一项，也⛔ 为了"有活干"把 D1 拆成 MCP 侧的补丁表。

## 判据（若批准 D1／D2，落点与形状）

`crates/yeban-theory/tests/`（⛔ 覆盖性判据**现在一条也没有** —— 这是实测的第二条结论：
现有的 `voice_leading.rs:413` 那条测试钉的是"每个声部**属于**和弦音"＝**成员归属**，⛔ 不是覆盖性）：

1. `每个声部都是和弦音`（已有，钉住不退）。
2. `三和弦 × FOUR_VOICES 的每层覆盖 {根,三,五}` —— 现读为**红**（层0/2/3 缺=[3]）⇒ 本票的**主判据**。
3. `七和弦 × THREE_VOICES 的可行性口径`：要么每层覆盖三个音级，要么报 `NoFeasibleVoicing`，
   ⛔ 静默丢音（现读为**红**）。
4. `缺省逐字节不变`（若批准 D2）：出厂开关关闭时，与今天的输出逐字节相同（`ADR-0016` 那条）。
