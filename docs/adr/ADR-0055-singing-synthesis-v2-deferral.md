# ADR-0055 — G3（无歌声合成）：SVS 在规范里排在 v2.0.0，本票只登记路线，⛔ 不提前实现

- **状态**: **Proposed（待人类批准；⛔ 集成者代签 —— 本票不改代码）**
- **决定者**: 待人类裁决
- **关联**: Zimmer 2026-10-31 创作评估 §5.2（`G3` 仍存在）；`MCP-TOOL-006`（`yeban_edit_notes`
  的 `note.syllable` / `note.phonemes`）；`MCP-TOOL-005`（`yeban_render_master` 的
  `unsupported.noteLyrics` 登记）；`ARCH-DET-001`（确定性读数）；
  `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:478`（v2.0.0 那条「本地神经歌声合成
  (SVS)」）；`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:342-343`（`syllable` / `phonemes`
  两个字段的规范位置）；`crates/yeban-mcp/src/domain/render.rs:56,675,4423`（登记点）；
  `crates/yeban-dsp/src/oscillator.rs`（`VOCAL` 波表）；`AGENTS.md §2` 红线 7 / 8 / 9
- **日期**: 2026-10-31

## 背景（全部现读，⛔ 不采信报告的措辞）

评估把 `G3`（"人声线做不出来"）判为**仍存在**。仓库侧的真相分三层：

1. **载体在，且在规范里**：`MidiNote.syllable: Option<String>` 与 `phonemes: Vec<String>`
   是 `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:342-343` 逐字声明的字段；工具面
   `yeban_edit_notes` 也**真的收**这两个键（`crates/yeban-mcp/src/tools.rs:930` 的
   `NoteOp` 清单）。现读 `syllable|phoneme` 的分布：`yeban-model` 5 个文件、
   `yeban-mcp` 10 个、`yeban-midi` 4 个、`yeban-engine` 2 个、`yeban-render` 2 个、
   `yeban-app` 1 个 —— 不是"没接线"。
2. **能唱出去的部分已经能唱**：`syllable`（非空）已写成 SMF `FF 05` 歌词元事件（`#312`），
   导入侧按"唯一才归属"读回（`#317`）；`phonemes` ⛔ 没有 SMF 载体（SMF 只有一个文本槽，
   把 SVS 对齐数据写进歌词槽等于伪装），所以导出侧另报一个键。
3. **母带⛔ 不发声，且这是**明说的**：`yeban_render_master` 对"任一音符带 `syllable`
   或非空 `phonemes`"登记 `unsupported: ["noteLyrics"]` ＋ `unsupportedCounts`
   （`render.rs:675` 的名册、`:4423` 的触发点、`:56` 的口径表）。评估的实测与之一致
   —— 音高腿有声、人声腿无声，回执没有撒过谎。

**关键读法**：`ROADMAP:478` 把「本地神经歌声合成 (SVS)：本地打字即唱，支持真实发音、
滑音与微表情包络推理」放在 **v2.0.0**（与商业插件宿主、`yeban-vst` 同一格），
而本项目当前的推进面是 Phase 4 / `ROAD-M4-*`。⇒ `G3` 不是"漏做的 Phase 4 项"，
是**规范上明确推迟的里程碑**。而 `DESKTOP_UI_UX` §9.3（`:372-373`，2026-10-06 经负责人
裁决**降级为 Advisory**）里那句"底层音素集合直接绑定 SVS 引擎定义的声学集合"是非绑定参考。

## 为什么本票不直接补

* **神经路线撞三条红线**：本地 SVS 模型必然引入权重与依赖 —— `AGENTS.md §2` 红线 9
  要求任何神经网络权重在 `assets/models/MANIFEST.json` 登记许可证与 SHA-256，红线 7
  要求实时回调零分配（推理引擎在回调里分配／锁是硬否决），红线 8 让 `unsafe` 需
  `// SAFETY:` 审计。这三条的处置是**项目级决定**，不是本切片能顺手带的。
* **规则形态（非神经）也⛔ 无规范依据**：共振峰／音素→包络的映射表在四份规范里
  `formant` 0 命中、`vocal` 0 命中（只有 `yeban-dsp::oscillator::VOCAL` 这个**波表名**，
  它是音色配方、不是发音器）。⛔ 私自发明一张音素表就是发明规范。

## 路线（**本票不另立一套名册** —— 路线的权威在 `docs/ledger/human-decisions.md` 的 `HD-67`；本票只补读数与一条新选项）

`HD-67` 原有的四条与下表一一对应：**D1 ≡ A（先勘察）之上收口**、**D4 ≡ B（登记式人声样本）**、
**D3 ≡ C（纯 DSP 共振峰）的神经放大版**（`ROADMAP:478` 那条）、**D2 是 `HD-67` 之外的新选项**。
下表⛔ 不是第二份裁决，是把 `HD-67` 的四个选项配上 2026-10-31 的现读代价。

| 路线 | 内容 | 代价 / 风险 |
| :--- | :--- | :--- |
| **D1 关掉本票（推荐默认）** | 承认 `G3` ＝ v2.0.0 推迟项，⛔ 不列当前缺口；把评估的"仍存在"改读成"按规范尚未开工"。仓库侧仅补一处**可发现性**：`yeban_render_master` 的 `noteLyrics` 说明里加一句"歌声合成按 `ROADMAP:478` 属 v2.0.0"，让下一次创作评估不再把它算成缺陷 | 一行文档；不改行为、不改 digest |
| **D2 借既有波表做"哑唱"** | 在 `InternalInstrument` 支路上按音符 `syllable` 的元音把 `osc1_wave` 临时切到既有 `VOCAL`（下标 2，ADR-0027 已证明这条路**真的**改变母带）。⛔ 不新增音素表，只做"元音→已有配方"的映射 | 需要**新的裁决**：映射表谁定义？会改变所有带歌词工程的渲染 digest ⇒ 5 张基准与既有 pin 全要重录 |
| **D3 正式 SVS 切片** | 按 `ROADMAP:478` 提前开工：模型层音素时间轴 ＋ `Op` ＋ 迁移 ＋ 推理后端 ＋ 权重登记 ＋ 实时零分配判据 | 一条工作线量级（红线 7/8/9 三条同时吃），且推翻 v2.0.0 的排期承诺 |
| **D4 采样路线** | 用真人合唱采样（SFZ）充当合唱垫 —— 这条路今天就在工具面上（`setTrackSampler`），⛔ 被 `G4` 的素材缺口挡住：现读 `assets/samples` 共 **1 586** 个文件 = `.sfz` 1 398 ＋ `.txt` 170 ＋ `.md` 11 ＋ `.xml` 6 ＋ `.json` 1，**音频样本 0 个**（清单 `policy` 逐字写明登记式外取，总素材 9.371 GiB 不随包分发） | 素材获取与许可是**人类**决定（撞过的坑：>10 MiB 二进制还过得了文件守卫，见 `AGENTS.md §2` 红线 9 与 `.gitignore` 里那条注释） |

## 代价与边界

* D1 之外任何一条都会改变既有工程的渲染结果或模型形态 ⇒ 必须先有 `ADR` 批准，再谈切片。
* 本票**不**新增 `propose_section` 键、**不**新增 `DeviceKind` 变体、**不**动
  `unsupported` 名册（那条名册由常驻判据钉住，增删都要同批改判据）。
* 与 `G5` 的边界：音色入口**已存在**（`insertDevice` + `InternalInstrument` +
  `osc1_wave`/`osc2_wave`，判据
  `an_instrument_wave_choice_changes_the_master_and_stops_registering_device_chain_dsp`
  钉住"换波表 ⇒ 母带 digest 必变"）。`G3` ⛔ 不是"选不了 `vocal` 音色"，是"不会按歌词发音"。
