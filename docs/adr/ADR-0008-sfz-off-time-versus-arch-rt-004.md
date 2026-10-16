# ADR-0008 — SFZ `off_time`（6 ms 淡出）与 `ARCH-RT-004`（3 ms 窃取淡出）的冲突

- **状态**: **Proposed（待人类裁决）**
- **决定者**: 集成者（Agent）提出；口径归负责人
- **关联**: `ARCH-RT-004`（Normative —— 窃取淡出 3 ms）；SFZ v2 的 `off_mode=time` / `off_time` / `off_shape` / `off_curve`；
  `crates/yeban-sfz/src/instrument.rs`（`:215` 缺省常量、`:315-319`/`:331` 的 `OffMode::Time` 契约、`:485-497` `Region::off_time`）；
  `crates/yeban-engine/src/sfz_instrument.rs`（`:259-262`/`:391`/`:1046-1049` 的书面偏离）；
  `docs/ledger/sfz-core-notes.md:117-121` 与 §6.4（本条冲突的既有登记）
- **日期**: 2026-10-16

## 背景（这是一次规范与规范的冲突，不是实现缺口）

`ARCH-RT-004` 规定**窃取淡出 3 ms**；而 SFZ v2（ARIA 扩展）规定 `off_mode=time` 用一段
**长度 ＝ `off_time`** 的淡出来关断声部，`off_time` **缺省 6 ms**，曲线由 `off_shape` / `off_curve` 控制。
规范页逐字：

> `off_mode=time` … can be used to specify **a time independent of the sample release time** using
> `off_time`, and also `off_shape` and `off_curve` to control the **curve of the fadeout**. In ARIA,
> `time` is actually the default value with a default `off_time` of 6 ms. **6 ms is also the fadeout
> time for `off_mode=fast` so these are equivalent in end result**, but `time` is the true default
> value 'under the hood'.

⇒ ⚠ **两个「3 ms / 6 ms」不是同一件事的两种说法**：`ARCH-RT-004` 管的是**窃取路**（池满抢占），
SFZ 的 `off_time` 管的是**`off_by` 独占组路 / `off_mode=time` 路**（见 ADR-0008 的上下文：
`#94` 已把 `off_mode` 的作用面裁成「只管 `off_by` 关断」）。本 ADR 只问后者。

## 事实（本仓现状，逐条有据）

1. ⭐ **解析层早已完整**：`OFF_TIME_DEFAULT_SECONDS = 0.006`（`instrument.rs:215`）、字段 `off_time: Option<f32>`（`:498`）、
   生效值 `effective_off_time()`（`:784`）、解析 `:1402-1412`（负/NaN/inf ⇒ `Err`，见 `error.rs:279-289`）。
2. ⭐ **播放层一个字节都没带它**：`crates/yeban-engine/src/` 里 `off_time` **只出现在注释**（`:259-262`/`:391`/`:1046-1049`）。
   今天的实现是：**不读 per-region `off_time`**，`off_mode=time` **按 `normal` 处理**（`#94` 登记的书面偏离）。
3. ⭐ ⭐ **那条「按 `normal`」的偏离依赖一个**不合规范的措辞****：台账与模块头把 `off_time` 写成"`off_mode=time` 的**延迟**"
   （`sfz-core-notes.md:452`、`sfz_instrument.rs:260-261`/`:391`/`:1046-1049`），而规范说的是**一段淡出**。
   ⇒ 按规范实现等于**用一条新的、仍不合规范的偏离换掉旧的那条**。
4. ⭐ **`off_shape` / `off_curve` 本仓未建模** ⇒ 按规范做也只是**半条**（曲线部分要动 `yeban-sfz` 的解析面）。
5. ⭐ **本仓既有契约与规范冲突**：`instrument.rs:315-319`（`OffMode::Time` 逐字"用一段**与采样 release 无关**的时间关断声部"）
   ＋ `:331`（"**在 `off_time` 秒后结束声部**"）＋ `:485-497`（"它**不**改写 `ARCH-RT-004` 的 3 ms、也**不**是 `normal` 的包络 release"）。
6. ⭐ **语料有真实用法**：`off_time` 出现在 **15 个文件 / 16 次**；`off_mode=time` **4 个文件 / 4 次**
   （`off_mode` 取值分布 normal 818 / fast 87 / time 4）⇒ **不是无人使用的 opcode**（对照 `direction` = 0 次）。
7. ⭐ **尺寸代价实测**：`SfzVoice` ＝ **40 B 且零填充**（`align_of == 8` 由两个 `u64` 保证）
   ⇒ **任何 per-voice 新字段 ⇒ 48 B ＝ ＋8 B**（`#91`～`#97` 四单都做到了 ＋0）。

## 选项

| # | 方案 | 代价 | 后果 |
| :--- | :--- | :--- | :--- |
| **A** | **照规范**：`off_mode=time` ＝ **一段长度 `off_time` 的淡出**（与 `ampeg_release` **无关**） | 需 **per-voice 淡出状态 ⇒ ＋8 B**（`SfzVoice` 40 → 48），**或**改 `trigger_note` 签名引入绝对帧；且 `off_shape`/`off_curve` 仍缺 ⇒ **半条** | 与规范同向；但**要移动** `SfzVoice` 字面基线 ＋ 一个**新偏离**（`fast` 在本仓是硬停、缺省 6 ms 下与 `time` **不** "equivalent"） |
| **B** | **照"延迟 `off_time` 后进释放"**（＝我 `#99` 的裁决草稿） | 要把绝对帧引进 `trigger_note`（现签名无 `position`）⇒ 动 **25 个既有测试调用点** ＋ `synth.rs:1928`；并**改写**已登记的"延迟"契约 | ⛔ **仍不合规范**（规范说"independent of the sample release time"）⇒ **换一条偏离** |
| **C** | **维持现状**（不读 per-region `off_time`；`off_mode=time` ≡ `normal`）＋ **把措辞改成规范口径的偏离**（把"延迟"改成"未实现 per-region `off_time`，`time` 按 `normal`"） | **0 代码、0 尺寸、0 基线移动**；只改**文档**（`sfz-core-notes.md:452` ＋ 模块头三处） | 与 `ARCH-RT-004` **不冲突**（那条管窃取路）；per-region `off_time` **有意不读**并**说清**；**留下"曲线（`off_shape`/`off_curve`）也不做"的登记** |
| **D** | 为 SFZ 路径**接上** per-region `off_time`，**并**裁定它与 `ARCH-RT-004` 的关系（例如：窃取路仍 3 ms、独占组路用 `off_time`） | A 的全部代价 ＋ 一次 Normative 澄清 | 需要人类先回答"3 ms 与 6 ms 各管哪条路" |

## 建议

**C**（集成者建议，**不是裁决**）：① 现实现与 `ARCH-RT-004` **不冲突**（两条路不同路，见背景）；
② **A / B 都要动既有字面基线或既有测试调用点**，而**本片收益只是"4 个文件里的 `time` 语义"**；
③ **C 把"未实现"从"静默"变成"有文档 ＋ 有登记"**，⭐ 并且**修掉一句不合规范的措辞**（"延迟" ⇒ "未实现 per-region `off_time`"）——
⭐ 即 C **不是"什么都不做"**，而是**把现状说准 ＋ 把缺口登记清楚**。
⚠ 若负责人要**真支持** `off_mode=time` ⇒ 选 **A ＋ D**（并把 `off_shape`/`off_curve` 一并排期），
⚠ 且必须**同时**回答"缺省 6 ms 是否要 'equivalent to `fast` in end result'"（规范说等价、而本仓 `fast` 是硬停）。

## 不决定的后果

`off_mode=time` 继续按 `normal` 处理、per-region `off_time` 继续不被读取；**4 个文件的 `time` 语义与规范不同**，
但已**逐条登记**在 `sfz-core-notes.md` 与模块头（⛔ 不是静默偏离）。⇒ **不构成阻塞**：功能可继续推进，
本条只决定"要不要为 4 个文件引入 per-voice 淡出状态 ＋ 改测试签名"。
