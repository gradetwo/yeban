# ADR-0031 — 设备链 CRUD **不**另开工具：继续走 `yeban_edit_notes` 的 op 词汇；摩擦由 `knownParams` 补偿

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: `MCP-TOOL-006`（`yeban_edit_notes`）、`GAPS.md` G14（WALL ③）、
  `crates/yeban-model/src/ops.rs`（`InsertDevice` / `RemoveDevice` / `SetDeviceBypassed` / `SetDeviceName`）、
  `crates/yeban-mcp/src/op_schema.rs`（`insertDevice` / `removeDevice` / `setDeviceBypassed` / `setDeviceName`）、
  `crates/yeban-mcp/src/effect_params.rs`（`recognised_names()`，58 个拼写的唯一供给点）、
  `crates/yeban-mcp/src/domain/mod.rs`（`effect_param_warnings`）、`crates/yeban-mcp/src/output_schema.rs`
- **日期**: 2026-10-27

## 背景

`GAPS.md` G14 实测（外部 agent 试跑）：插混响 ×8、Master 压缩都**成功**，但手段是
`yeban_edit_notes` 的 `ops[].kind = insertDevice / removeDevice / setDeviceBypassed`；
登记的**影响**是"能用，但设备链操作混在音符编辑工具里"。同一份报告还登记了第二个摩擦：
参数名须精确（`reverb_size` / `reverb_damp` / `reverb_wet`、`threshold_db` / `ratio` /
`attack_s` / `release_s` / `makeup_db`），而 `reverb_mix` / `reverb_width` **不认**。

于是 G14 有两个可分的候选答案：① 新增一台专用工具（第 26 台）；② 把"该写什么拼写"
送到调用方手里。

## 决定

### D1 不开设备专用工具（本 ADR 的核心）

设备链的增删旁通继续住在 `yeban_edit_notes` 的 op 词汇里。理由，按绑定强度排：

1. **op 词汇是唯一权威面**。`yeban-model::Op` 已经是这四个变体的**唯一**载体
   （`ops.rs:730 InsertDevice` / `:739 RemoveDevice` / `:499 SetDeviceBypassed` /
   `:530 SetDeviceName`），任何"设备工具"要么重复发射同一批 `Op`
   （= 第二套写入路径，撤销/提案/合并/多父语义要重新对账一遍），要么发明 `Op` 之外的
   直接改写（违反 [ARCH-OPS-001] 的操作日志契约）。
2. **批语义正是设备链需要的**。8 台混响 + 1 台压缩 + 改名是一次 `ops[]` 数组、
   一次提案、一次合并；拆成逐设备工具反而把一次意图切成 N 次往返（G13 刚花掉一票
   把这些往返压回去，见 `ADR-0029` 的 `autoMerge`）。
3. **提案/合并路径只有一条**。`edit_notes` 自带 `pendingMerge` / `autoMerge` /
   `warnings[]` / CAS 校验。新工具要复制这套控制面，才有可用的等价物。
4. 规范侧没有授权：`MCP-TOOL-001..010` 是文档化十工具，扩展工具按 ADR 逐个登记
   （现 15 台，`ADR-0030` 刚加过分轨导出）。**"混在音符编辑工具里"是命名给人的
   错觉，不是能力缺失** —— `edit_notes` 的 `ops` 覆盖的是模型层全部编辑动作，
   设备链本来就和音符同属 `Op` 家族。

⛔ 本决定**不**引入新工具、不改 `TOOL_COUNT`、不动 `schemas/mcp-tools.schema.json`
的工具枚举。

### D2 真正的摩擦是"拼写公告的时机"，就地补 `knownParams`

不认一个未知拼写时，警告此前只说"不认 + 去哪儿读回"，把"那该写什么"留给调用方
自己 `tools/list` 捞 ⇒ 一次往返变两次。现在每条 `unrecognisedEffectParam` 逐字带
`knownParams` = `effect_params::recognised_names()`（32 规范名 + 26 别名，
规范名在前、别名紧随）。

为什么给**整张表**而不是"最像的几个"：能走到这条警告的前提就是
`similar_spellings` 为空（编辑距离全 > 2）—— 此刻**没有**最近邻可给
（`reverb_mix` 与 `reverb_wet` 差 3）。表由 `recognised_names()` 单点供给，
⛔ 不允许第四份手抄（另三份：`op_schema` 的 `oneOf`、读回的 `recognisedParams` 口径、
本警告）。

### D3 如实登记的边界（本 ADR 不解决，只点名）

- **参数值插入后没有 `Op` 可改**：`ops.rs:479` / `:512` 的注释写明设备参数唯一入口是
  `InsertDevice`（整台 `DeviceDefinition` 一次性给全）；要改一个旋钮的**曲线**只有
  `yeban_edit_automation` 的 `DeviceParam` 目标。想"改一个标量值"Today 的做法是
  `removeDevice` + `insertDevice` 同批重放。这不是本票引入的，也不由本票修改。
- `reverb_mix` / `reverb_width` 被引擎**刻意**不认（`effect_params.rs` 模块文档
  与 `insert.rs` 的模块文档都写明），本表的 `knownParams` 里也不许出现它们 ——
  判据把这条反面钉住。

## 判据与验收

- `crates/yeban-mcp/tests/trap_175.rs::the_warning_hands_back_the_exact_accepted_spellings`：
  单位 = 58 个拼写 + 1 个键集合（`code` / `deviceName` / `paramName` / `knownParams` / `hint`）
  + 2 个"不许广告成可用"的反面名。表内每个名字都必须被 `recognise` 真的认下。
- 既有判据不动：`tests/effect_contract.rs`（58 个拼写逐个对照 `insert.rs` 的带引号字面量）、
  `tests/trap_175.rs` 判据 ①②③④（错字拒绝 / 正确拼写读回干净 / 刻意不认仍接受 / 58 个数）。
- `output_schema.rs` 的 `yeban_edit_notes` 描述与 `warnings.items.properties` 同步登记
  `knownParams`（该 schema 无 `additionalProperties:false`，但键位仍要公告）。

## 影响

G14 的**拼写发现**一半当场消除；**"没有设备专用工具"**一半被裁决为**有意保持现状**
（状态词沿用 `docs/ledger/feature-alignment.md` 的 `有意不做`，加注见落地后的台账轮）。
若将来出现"必须逐设备寻址才能表达"的真实需求（例如改单条曲线的锚点之外的东西），
那是一次新的规范缺口，需另开 ADR，不由本 ADR 预留。
