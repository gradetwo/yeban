# ADR-0024 — SFZ 装载对缺样本发非阻塞警告；采样外取流程入文档

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: `MCP-TOOL-001..010` 家族下的扩展工具 `yeban_load_sfz`
  （spec `MCP-TOOL-EXT-SFZ-LOAD`）、`MODEL-ISO-001`（会话 registry 第 2 层）、
  `MUST-GATE-014`（登记式资产策略）；
  `crates/yeban-mcp/src/domain/sfz.rs`（`SfzLoad::samples_warning`）、
  `crates/yeban-mcp/src/domain/render.rs`（同型先例 `silent_sources_warning`）、
  `crates/yeban-mcp/tests/sfz_tools.rs`、`crates/yeban-mcp/tests/sfz_sample_files.rs`、
  `docs/SFZ_SAMPLE_ACQUISITION.md`
- **日期**: 2026-10-27
- **编号说明**: `ADR-0023` 已被并行工作线 `line/remove-track-routing` 占用（同名文件在
  其分支上）；本条按下一个空号取名，两线合入 main 的先后不改变编号语义。

## 背景

审计 `G4`（2026-10-27，origin/main `36843ca9` 实测）：`yeban_load_sfz` 载入仓库自带
二胡 SFZ 返回 `success`、360 个区域，但**每个** `sampleFileState` 都是
`unreadable(... entity not found)`，随后 `setTrackSampler` 绑轨渲染出
RMS −144.5 dBFS 的**绝对静音**。"加载成功"配"整轨静音"是最容易误导的组合——数据其实
都在回执里（`sampleFailures` / `sampleFilesFailedCount`），但要调用方自己数数组才看得懂。

根因分两层：

1. **策略层**：采样是登记式素材（`assets/samples/manifest.json` 的 `policy`：
   9.371 GiB 不随包分发），缺文件是**设计内状态**；
2. **回执层**：`status: "success"` 说的只是"解析 + 注册成功"，却没有一条**点名**
   "这没有声音"的警告；采样器轨（`yeban_render_master` 的 SFZ 支路）**没有**正弦回退
   （`domain/sfz.rs` 模块头第 1 条 ⚠），缺样本 ⇒ 静音。

## 裁决

1. **D1 — 警告而非失败。** 采样缺失**不**让装载失败：SFZ 文本合法、registry 注册成功
   是真实发生的事，且用户可能下一步才外取样本（外取后 `yeban_load_sfz` 会重新读盘——
   `sfz::load` 每次调用都新建 `SampleCache`，`reused: true` 也不短路）。改为在
   `data.warnings[]` 里点名 `sfzSamplesUnreadable`（`severity: warning`、
   `blocking: false`、`count`、`allSamplesUnreadable`、最多 5 条 `examples`、
   `hint` 说清后果与外取路径）。口径与 ADR-0022 的 `silentSources` 同型。
2. **D2 — `warnings` 键恒在**（无警告时为空数组），与 `yeban_render_master` 的
   `warnings` 呈现一致；`dryRun` 的 `preview` 同样带——预览不许假装不会静音。
3. **D3 — ⛔ 不加采样器轨的正弦回退。** 任务单的另一选项（"渲染器在无采样时给正弦
   回退并标注"）**被拒**：采样器轨的静音是 `#274`/`#275` 已登记的既有语义
   （`notesWithoutSample`），静默合成器音色会把"缺资产"伪装成"低保真成品"，
   与"绝不静默降级"的纪律相反。`yeban_render_sfz_wav` 那条**显式导出**路径的回退维持原样。
4. **D4 — 外取流程入文档。** 新增 `docs/SFZ_SAMPLE_ACQUISITION.md`：git 形态
   （`repo` + `pin` + `git archive`）与 archive 形态（`archive.url` + 摘要校验）各一段、
   逐文件 sha256 校验脚本、以及"重跑装载看警告消失"的完成判据。§needs N3
   （脚本提升到 `scripts/dev/` 需改 `scripts/**`，集成者独占）如实登记为未完成。

## 代价与边界

- 回执多一个键 ⇒ 依赖"精确键集合"的调用方会看到新增（本仓库测试为按键选取断言，
  已核 `sfz_tools.rs` / `sfz_sample_files.rs` / `sampler_path_roots.rs` 无键集合钉死；
  `schemas/mcp-tools.schema.json` 只钉入参）。
- 警告不阻断：调用方**可以**忽略它继续渲染出静音——这是刻意的（外取是异步动作），
  但渲染侧的 `silentSources`（G1/ADR-0022）与 `sources[].sampler.missingSamplePaths`
  构成第二道点名。
- 真声卡/听感验证仍属人类事项，本条只改回执与文档。

## 判据（测试）

- `tests/sfz_tools.rs::render_sfz_wav_writes_a_non_silent_wav_through_a_real_mcp_call`
  —— 夹具样本**全部**读不到 ⇒ 恰好一条警告、`allSamplesUnreadable: true`、
  `count` 与 `sampleFilesFailedCount` 同读数、`hint` 点名清单路径；
- `tests/sfz_sample_files.rs::the_load_response_reports_the_file_it_really_read`
  —— 真读到文件 ⇒ `warnings == []`（没问题不许假报警）。
