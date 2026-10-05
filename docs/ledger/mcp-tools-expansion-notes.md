# mcp-tools-expansion —— 三类扩展工具（自动化泳道 / 设备与引擎 / 音频导入）

> 工作线：`line/mcp-tools-expansion`（worktree `.worktrees/mcp-tools-expansion`，基线 `c53b7f3`）。
> 裁决依据：`ADR-0001` **D46**（十工具是起点不是上限；扩时必须同步
> `schemas/mcp-tools.schema.json`）、**D25**（错误码联集，不发明新码）、**D45/D47/D49**、
> `MODEL-ISO-001`（三层状态）。
> 本文件由本线维护；`docs/ledger/feature-alignment.md` **不归本线**（集成者维护，见 §7）。

---

## 1. 交付了什么（三个新工具，全部**真做事**）

| 工具 | specId | 能力 | 副作用 | 落点 |
| :--- | :--- | :--- | :--- | :--- |
| `yeban_edit_automation` | `MCP-TOOL-EXT-AUTOMATION` | 读某条自动化泳道的点与值（**唯一求值入口**）+ 写一个点 | `project-state`（按最坏情况） | `Op::SetAutomationPoint`（可逆） |
| `yeban_query_engine_state` | `MCP-TOOL-EXT-ENGINE-STATE` | 某轨设备链 + 引擎/会话读数 | `read-only` | 三份状态各有唯一来源 |
| `yeban_import_audio` | `MCP-TOOL-EXT-IMPORT-AUDIO` | 资产池哈希 / 磁盘音频 → `clip_pool` 条目 | `project-state` | `yeban-decode` + `Op::AddClip`（可逆） |

注册表 `TOOLS` 从 12 → **15**（`TOOL_COUNT = DOCUMENTED_TOOL_COUNT(10) + EXTENSION_TOOL_COUNT(5)`）。
新增的扩展段清单是**常量**（`tools::EXTENSION_NAMES`），判据、契约对账与样本导出全部从它派生。

### 1.1 `yeban_edit_automation` 的实参

| 实参 | 必填 | 语义 |
| :--- | :--- | :--- |
| `trackId` | ✅ | 目标音轨 `EntityId` |
| `lane` | ✅ | `TrackVolume` / `TrackPan` / `SendGain` / `DeviceParam` / `Macro`（**与 `project.json` 逐字相同**，不接受 `trackVolume` 这类别名） |
| `edgeId` | `SendGain` 必填 | 路由边身份 |
| `slotIndex` / `paramIndex` | `DeviceParam`（默认 0） | 设备链插槽 / 参数下标 |
| `macroIndex` | `Macro`（默认 0） | 宏下标 |
| `ticks` | 可选（≤256） | 要读取自动化值的 tick（**保持给定顺序**） |
| `point` | 可选 | `{tick, value, curve?}`；缺省 = **只读调用**（一位都不改） |
| `pointId` | 可选 | 显式点身份；缺省由 `(目标, tick)` **确定性派生** ⇒ 同一 tick 重复写 = 更新同一点 |

### 1.2 `yeban_query_engine_state` 的实参

`trackId`（可选：给了就展开该轨的设备链）。响应里的 `stateSources` 明写四个字段各自的唯一来源：

| 字段 | 唯一来源 |
| :--- | :--- |
| `engine.sampleRate` | `project.audio_config.sample_rate`（顶层刻意没有冗余副本） |
| `engine.bufferFrames` | 宿主注入的 **`Domain::set_engine_readings`**（`EngineReadings` 只读快照；没有注入时 `null` + `bufferSource: "unavailable"`） |
| `session.playheadTicks` / `isPlaying` | `yeban_model::SessionRuntimeState`（第 2 层，**不实现 `Serialize`**） |
| `session.undoCursor` | `undo_session::UndoState`（`Domain::apply` 结束时单向同步进会话态） |
| `track.devices[]` / `totalLatencySamples` | `TrackV3::devices`（含 `latency_samples`，`ARCH-PDC-001`） |

### 1.3 `yeban_import_audio` 的实参

| 实参 | 必填 | 语义 |
| :--- | :--- | :--- |
| `name` | ✅ | 片段显示名 |
| `assetHash` | 与 `path` **恰好一个** | 已在会话 CAS 池（`assets/{sha256}`）里的哈希 |
| `path` | 与 `assetHash` **恰好一个** | 磁盘音频（WAV/FLAC/OGG…） |
| `gainDb` | 可选（默认 0.0） | 片段增益 |
| `clipId` | 可选 | 显式片段身份；缺省由 `(来源, 名字, 增益)` 确定性派生 |

两条来源都会**真的解码一遍**（`DecodeOptions::default()`，与 `yeban_render_master` 的音频片段
路径同一个预算、同一个 `render::AssetStore` 池读法），随后**丢弃 PCM** ——登记进 CAS 池的是
**容器字节**（渲染路径自己会解码 `ClipContent::Audio` 的字节）。

---

## 2. 判据与证据（①..⑧）

判据全在 `crates/yeban-mcp/tests/extension_tools.rs`（12 条 + 既有 6 个测试文件的口径不变）：

| # | 判据 | 用例 | 证据（本机可跑的那一半） |
| :--- | :--- | :--- | :--- |
| ① | 读类：改模型 ⇒ 读数跟着变 | `automation_read_follows_the_model_through_the_unique_entry`、`engine_state_reads_three_single_sources` | 同一 tick 从 `-3.0` 变 `-20.0`（换工程）；采样率 `48000 → 44100`、设备链 `1 → 0` |
| ① | 写类：工程真的变了 + **逆操作逐字节回退** | `automation_write_lands_in_the_project_and_undo_restores_byte_for_byte`、`import_audio_registers_a_clip_and_undo_restores_byte_for_byte` | `canonical_project_bytes` 前后不等 → `yeban_undo` 之后**逐字节相等**；提交数 +1 → 撤销后回到原值；`yeban_redo` 再逐字节复原 |
| ② | `dryRun` 状态一位不变 + 预览与真做一致 | `dry_run_leaves_every_state_bit_identical`、`dry_run_preview_equals_the_real_call_for_all_three_tools` | 工程字节 / 摘要 / 提交数 / 会话态 / CAS 池 / 撤销游标**逐项**不变 + 幂等缓存不增长；预览的每一件事实与真做逐字段相同，且**真做后的实测摘要 == 预览里的预测** |
| ③ | 坏参数只用既有错误码 | `bad_and_unknown_parameters_only_use_codes_inside_d25` | `INVALID_PARAMETER_RANGE`（未知 lane / 缺 edgeId / 未知曲线 / 来源形状 / 坏哈希）、`OUT_OF_RANGE`（越出泳道取值域）、`TRACK_NOT_FOUND`、`INDEX_OUT_OF_BOUNDS`、`ENTITY_NOT_FOUND`、`FILE_NOT_FOUND`、`RENDER_FAILED`、`-32602`（类型错 / 拼错键） |
| ④ | 幂等键不重复生效 | `the_same_idempotency_key_never_applies_twice` | 第二次 `replayed: true` + 主体逐字节相同 + 摘要/提交数/CAS 池一位不动 |
| ⑤ | `tools/list` 与契约样本可发现 | `the_three_new_tools_are_discoverable` | 三个描述符带 `specId` / `dryRunSupported` / `idempotent` / `inputSchema`；样本目录里各有 `mcp-tools.call.<tool>.json` |
| ⑥ | 无第二份实现 | `no_second_automation_evaluation_in_production_sources`、`the_write_paths_commit_through_the_single_undo_entry`、`dry_run_entry_points_take_shared_references_only`、`production_region_agrees_across_the_three_copies` | 见 §4（本机真跑过，含注入） |
| ⑦ | 与既有十工具不冲突 | `the_documented_ten_tools_are_untouched` + `tests/contract.rs` 的"十工具不得出现在扩展节" | 既有 6 个测试文件的口径**没有**放宽；`tools_e2e.rs` 的逐工具表补了 3 条 |
| ⑧ | 门禁 | `cargo fmt --all --check` + `run-gates.sh light` | 见 §5（本机 vs CI 严格区分） |

另外两条**不变量**判据：

- `the_session_mirror_never_drifts_from_the_undo_authority`：一串混合工具调用之后，
  `SessionRuntimeState::undo_cursor` 必须与 `undo_session::UndoState` 的权威游标一致；
- `engine_state_is_read_only_and_never_touches_the_session_or_the_mirror`：只读工具调用前后
  工程摘要 / 提交数 / `SessionRuntimeState` / 引擎镜像逐位相同。

### 2.1 `dryRun` 的"预览 = 真做"是怎么做到**结构性**成立的

三个计划都把响应体收敛成**一个**函数（`AutomationEdit::data` / `AudioImport::data` /
`engine_state::snapshot`），`Plan::describe`（`dryRun`）与 `apply`（真做）调的是同一个；
而 `automation::apply` / `import_audio::apply` 在提交之后会**重新**算一次摘要（并在自动化那条
路径上重新求值一次读数）与只读规划的预测比对，不等就报 `CONFLICT` ——
"预览会撒谎"这件事因此是**上报的错误**，不是沉默的漂移。

---

## 3. 错误码对照 `ADR-0001` **D25**（不发明新码）

三个新工具声明的码**全部**落在 20 值联集内：

| 工具 | 声明的码 |
| :--- | :--- |
| `yeban_edit_automation` | `NO_ACTIVE_PROJECT` · `TRACK_NOT_FOUND` · `ENTITY_NOT_FOUND` · `INDEX_OUT_OF_BOUNDS` · `OUT_OF_RANGE` · `INVALID_PARAMETER_RANGE` · `CONFLICT` |
| `yeban_query_engine_state` | `NO_ACTIVE_PROJECT` · `TRACK_NOT_FOUND` |
| `yeban_import_audio` | `NO_ACTIVE_PROJECT` · `ENTITY_NOT_FOUND` · `FILE_NOT_FOUND` · `IO_ERROR` · `INVALID_PARAMETER_RANGE` · `RENDER_FAILED` · `CONFLICT` |

映射口径（**不新造表**，能复用就复用）：

- 模型错误 → 契约码：复用 `domain/error.rs::code_for_model`（穷举，无 `_` 兜底）；
- I/O → `error::code_for_io`（`NotFound` ⇒ `FILE_NOT_FOUND`，`ENOSPC`/`StorageFull` ⇒ `DISK_FULL`，其余 `IO_ERROR`）；
- 解码失败 → **复用** `render.rs::decode_error_class` 的分类名（把它提成 `pub(crate)`，
  判据 `error_code_vocabulary_is_the_contract_enum_exactly` 钉住两侧同词）：
  `Budget(_)` ⇒ `INVALID_PARAMETER_RANGE` + `data.budget = true`（**PcmBudget 是请求侧上限闸门**，
  不是解码器故障）；其余 ⇒ `RENDER_FAILED` + `data.decodeError`；
- "没有可撤销历史"沿用既有口径 `INDEX_OUT_OF_BOUNDS`（D25 联集里没有 `NO_HISTORY`）。

`tools.rs` 里原来的"逐工具错误码 == 表格 16 个"判据**收窄到文档十工具**（口径就是"表格里列了
什么"），并新增一条更强的：**每一个工具**声明的码都必须在 D25 联集内（含文档十工具）。

---

## 4. ⑥ "没有第二份实现" 的机械证据（**本机真跑**）

`yeban-mcp` 传递依赖 `yeban-render`/`yeban-decode`（rayon/symphonia/rubato）⇒ **本机不编译它**
（`AGENTS.md` §5.2；`scripts/dev/heavy-deps.py` 判它含重依赖）。因此把三条**能从源码文本判定**
的性质抽成**零依赖**（只用 `std`）的纯函数，本机用裸 `rustc` 真跑：

| 模块 | 守卫 | 性质 |
| :--- | :--- | :--- |
| `domain/automation_audit.rs` | `scan_second_automation_evaluations` | 生产代码里出现 `value_at(` 但不是 `automation_value_at` ⇒ 违规；出现 `.ease(` / `interpolate` ⇒ 违规（**刻意不扫** `points_in_tick_order()`：它不含插值，界面画折线用它） |
| `domain/extension_audit.rs` | `scan_write_paths` | 两个写类扩展工具的 `apply` **必须**出现 `undo_session::commit(`，且不得出现直接改权威工程的形状 |
| 同上 | `scan_dry_run_entry_points` | 计划入口的签名必须是共享引用（`fn plan_edit_automation(domain: &Domain, …)` 等三条 + 两个领域计划函数的 `project: &YebanProjectV1`） |
| 同上 | `scan_error_code_vocabulary` | 实现写出的错误码集合 **==** 契约 `ToolResponse.error.code.enum`（文本级读契约文件，**唯一权威**） |

本机真跑的读数（命令与输出原文）：

```text
# 三个零依赖模块（--test，-D warnings）
rustc --edition 2024 --test -D warnings crates/yeban-mcp/src/domain/extension_pure.rs -o /tmp/p1 && /tmp/p1
   → test result: ok. 9 passed; 0 failed
rustc --edition 2024 --test -D warnings crates/yeban-mcp/src/domain/automation_audit.rs -o /tmp/p2 && /tmp/p2
   → test result: ok. 9 passed; 0 failed
rustc --edition 2024 --test -D warnings crates/yeban-mcp/src/domain/extension_audit.rs -o /tmp/p3 && /tmp/p3
   → test result: ok. 7 passed; 0 failed

# 同一份守卫跑在**真实仓库源码**上（裸 rustc 脚手架，逐字复用上面两个模块）
rustc --edition 2024 -D warnings /tmp/yeban-audit/local_audit.rs -o /tmp/yeban-audit/local_audit
/tmp/yeban-audit/local_audit <worktree>
   → 扫描: yeban-mcp/src 31 份 + yeban-app/src 18 份
   → CLEAN: 4 条守卫, 0 违规
```

### 4.1 注入 → 变红 → 还原（**四条都真做过**，md5 逐字节还原）

方法：`/tmp/yeban-audit/inject.py` 备份 → 注入 → 跑上面的脚手架（期望红且关键词命中）→
从备份还原（**断言 md5 与注入前相同**）→ 再跑一遍（期望 CLEAN）。四条全部 exit=0（脚本在
任何一条不符合预期时 `sys.exit(1)`）。

| # | 注入 | 变红的读数（原文） |
| :--- | :--- | :--- |
| 1 | `automation.rs` 把 `ops` 清空 + 直接 `write.op.apply(&mut active.project)`（只改内存不落 Op） | `VIOLATION: …/automation.rs:N 直接改权威工程 (apply(&mut active.project)) —— 必须走 undo_session::commit(` … `RED: 1 条违规` |
| 2 | `domain/mod.rs` 把 `fn plan_edit_automation(domain: &Domain, …)` 改成 `&mut Domain` | `VIOLATION: … 里找不到计划入口的共享引用签名 …` + `VIOLATION: …:N 计划入口拿到了**可变**领域引用` → `RED: 2 条违规` |
| 3 | `tools.rs` 把 `Self::Conflict => "CONFLICT"` 改成 `"STATE_CONFLICT"`（发明新码） | `VIOLATION: 契约 enum 里的 CONFLICT 没有被实现覆盖` + `VIOLATION: 实现写出了契约 enum 之外的错误码 STATE_CONFLICT` → `RED: 2 条违规` |
| 4 | 新建 `domain/injected_rogue.rs`，里面自己写 `lane.value_at(tick)`（第二份求值） | `VIOLATION: …/injected_rogue.rs:9 [secondEvaluation] 生产代码绕开唯一求值入口 automation_value_at 直接读泳道值: lane.value_at(tick)` → `RED: 1 条违规` |

注入 1–3 的"写类/只读/错误码"三条性质在 CI 上还有**运行期**判据（`tests/extension_tools.rs` 的
①②③）再证明一遍；注入 4 对应的运行期判据是 `no_second_automation_evaluation_in_production_sources`。

### 4.2 三份 `production_region` 拷贝不会变成三种口径

`undo_session` / `automation_audit` / `extension_audit` 各有一份（后两份为了"裸 rustc 独立跑"而
不能依赖 crate）。判据 `production_region_agrees_across_the_three_copies` 在**真实源码**上
逐文件断言三份实现输出**逐字节相同**；本机脚手架也顺带比对了两份。

### 4.3 复用而不是复制（"第二份实现"的正面清单）

| 事实 | 唯一来源 | 谁复用它 |
| :--- | :--- | :--- |
| 自动化求值 | `YebanProjectV1::automation_value_at` | `yeban_edit_automation`（本线） |
| 静态值回退 | `AutomationTarget::static_value` | 同上（不自己读 `track.volume_db`） |
| 采样点顺序 | `AutomationLane::points_in_tick_order` | 同上 |
| 逆操作 | `Op::invert` / `Op::apply_inverse` | 本线**一行都没写**逆操作 |
| 提交（改工程 + 写 Op 日志） | `undo_session::commit` | 两个写类工具 |
| 资产池读法 | `render::AssetStore`（`Domain` 实现它） | `yeban_import_audio` |
| 解码预算 | `yeban_decode::DecodeOptions::default().budget` | 同上（响应 `data.budget` 直接读它，不抄数字） |
| 解码错误分类名 | `render::decode_error_class`（提为 `pub(crate)`） | 同上 |
| PCM 格式短名 | `render::pcm_format_name`（提为 `pub(crate)`） | 同上 |
| 会话运行态 | `yeban_model::SessionRuntimeState` | `Domain.session`（注入 + 读数） |

---

## 5. 本机 vs CI（**严格区分**）

| 项目 | 本机（M2） | CI |
| :--- | :--- | :--- |
| `cargo fmt --all --check` | ✅ 真跑过（`run-gates.sh light` 的第一档） | ✅ |
| 机械红线守卫 14 条 + 文档契约 | ✅ 真跑过（唯一红点见 §7） | ✅ |
| `rustc --test` × 三个零依赖模块（25 条） | ✅ **真跑过** | ✅（同一条判据也在 crate 内） |
| 裸 rustc 脚手架跑 4 条文本守卫（真实源码） | ✅ **真跑过**（CLEAN） | ✅（`tests/extension_tools.rs` 的 ④ 组） |
| 四条注入 → 红 → 还原 | ✅ **真跑过**（§4.1） | —（注入是实验，不进仓库） |
| 契约 ↔ 注册表逐字段对账（枚举顺序 / `$defs` 属性集 / 类型 / 必填 / if-then 接线 / D25 集合） | ✅ 用 Python 预演过一遍（与我写的 Rust 判据同口径） | ✅ `tests/contract.rs` |
| `cargo clippy -p yeban-mcp` / `cargo test -p yeban-mcp` | ❌ **本机不做**（`heavy-deps.py` 判含重依赖；`run-gates.sh crate yeban-mcp` 会 SKIP 并指向 CI） | ✅ 全部运行期判据 |
| 跨语言 schema 对账（`validate_schemas.py --samples-dir`） | ⚠ 未跑（需要 `python3 -c "import jsonschema"`；本机缺该依赖时 Rust 判据会打印 SKIP，不伪造绿） | ✅ checks job |

⇒ **本轮的运行期判据（`tests/extension_tools.rs` 的 12 条）只有 CI 判决算数**。本机的绿只覆盖
"格式 + 机械守卫 + 零依赖纯逻辑 + 文本级守卫 + 契约静态对账"。

---

## 6. needs（交集成者 / 人类裁决，本线**不**自己开）

1. **`OpOrigin` 缺"AI 直接编辑"这一档**。模型 `OpOrigin` 七个变体里没有 `McpEdit`/`McpAgent`；
   本线借用 `AutomationRecord`（自动化写入落盘）作自动化编辑的来源、`Import` 作音频导入的来源，
   **作者字段**仍是 `yeban-mcp`（`UndoState.author`），因此没有伪装成用户操作。
   建议：模型侧补 `OpOrigin::McpEdit { agent_name }`（或裁决"借用即可"）。响应里 `origin.note` 已如实写出。
2. **CAS 池的字节不是 `Op` 的载荷**。模型 `Op` 全集（实测 29 个变体）没有任何资产变体 ⇒
   `yeban_import_audio` 的 `Op::AddClip` **逆操作不会回收**池里的字节：撤销后保存会把一份
   未被引用的资产写进容器。内容寻址让重复导入幂等（可收敛），但"孤儿字节"需要模型侧一个
   资产声明/回收 `Op`。本工具在响应 `notes` 里**如实**写出这一点。
3. **工程 `assets` 索引（许可 / 原路径 / 字节数）没有 `Op`** ⇒ 本工具只登记 `clip_pool` 条目、
   **不写索引**（`declaredInIndex` 如实上报）。要写索引必须先在模型侧补一个可逆的资产 `Op`。
4. **响度目标（`BASELINE-006`）本轮未做**：人类已延后口径 ⇒ 本线**不做**"能测的那一半"，
   因为"MCP 侧可测"依赖 Token 口径（`LUFS` 的 K-weighting 与门限）先定。
   **Token 口径未定**是这一条的全部原因。
5. **MIDI 导出不在本线**（`ADR-0001` **D47**：唯一出口是 app CLI `--export-midi`）。
   本线**没有**新增任何 MIDI 导出工具，也不打算加。
6. **没有"放置/引用片段"的工具**：`Op::AddClipPlacement` 在 MCP 侧仍未接线，因此
   `yeban_import_audio` 导入的片段**无法**被任何工具引用到音轨上（只能被 `yeban_render_master`
   在渲染时看到池里的资产）。需要一条 `yeban_place_clip`（或扩展 `yeban_edit_notes`）才能闭环。
7. **引擎读数镜像的注入点**：本线只提供 `Domain::set_engine_readings`（形态 A 的宿主调用）。
   形态 B（stdio 二进制）没有引擎，因此 `bufferFrames` 恒为 `null` + `source: "unavailable"`。
   若要让形态 B 也报缓冲，需要一条从 `yeban-engine` 到 MCP 的镜像边（**本线零新增依赖**，没有做）。

---

## 7. 本机门禁的**唯一**红点：`feature-alignment`（集成者的文件）

```text
bash scripts/gates/run-gates.sh light
==> fmt / guards / spec-ids / id-dictionary / decisions / gate-status / phase-status 全部 ok
三方对齐矩阵校验未通过:
  - `yeban_edit_automation` 在 `schemas/mcp-tools.schema.json` 里但**表里没有点名** —— 有工具却没人管它暴露没有
  - `yeban_import_audio` 在 `schemas/mcp-tools.schema.json` 里但**表里没有点名** —— 有工具却没人管它暴露没有
  - `yeban_query_engine_state` 在 `schemas/mcp-tools.schema.json` 里但**表里没有点名** —— 有工具却没人管它暴露没有
FAIL feature-alignment (exit=1)
```

这是**预期**的：`docs/ledger/feature-alignment.md` 由集成者维护（本线**禁改**，任务书明文），
新工具必须在表里点名。给集成者的三侧状态（供补表）：

| 工具 | 系统 | UI | MCP | 缺口 |
| :--- | :--- | :--- | :--- | :--- |
| `yeban_edit_automation` | 已实现｜`yeban-model` 的 `automation_value_at`（唯一求值入口）+ `Op::SetAutomationPoint` / `SetAutomationLane` | 有｜`crates/yeban-app/src/automation.rs`（泳道 → 折线投影，含 `automation_value_at` 细采样） | 有｜本线（读 + 写一个点，可逆） | 无 |
| `yeban_query_engine_state` | 已实现｜`SessionRuntimeState`（第 2 层）+ `project.audio_config.sample_rate` + `TrackV3::devices`；引擎侧的缓冲帧数住在 `yeban-engine` | 有｜`crates/yeban-app/src/engine_host.rs`（走带/seek/play）+ `meters.rs` | 有｜本线（**只读**；缓冲帧数走宿主注入的镜像） | 原因：形态 B（stdio 二进制）没有引擎进程，`bufferFrames` 只能为 `null`；计划：要覆盖它需要一条 `yeban-engine → MCP` 的镜像边（本线零新增依赖，未做）；状态：部分 |
| `yeban_import_audio` | 已实现｜`yeban-decode`（解码 + `PcmBudget`）+ `Op::AddClip` | **无**｜`crates/yeban-app/src` 里没有任何音频导入路径（在 `crates/yeban-app/src` 里分别 grep `decode_path` 与 `yeban_decode` 均零命中；`bridge.rs` 的音频片段是夹具里手写的假哈希） | 有｜本线 | 原因：UI 侧从来没有"导入音频文件"的入口，`yeban-decode` 此前只被 `yeban-mcp` 的渲染片段路径消费；计划：UI 侧接一条导入动作（走同一个 `Op::AddClip` + 同一份 CAS 池）；状态：无 |

---

## 8. 改动文件与行数

| 文件 | 增 / 删 | 说明 |
| :--- | ---: | :--- |
| `crates/yeban-mcp/src/domain/automation.rs` | +1029 / −0 | 自动化工具（读 + 写一个点） |
| `crates/yeban-mcp/src/domain/import_audio.rs` | +810 / −0 | 音频导入（decode + `PcmBudget` + `Op::AddClip`） |
| `crates/yeban-mcp/src/domain/engine_state.rs` | +300 / −0 | 设备链 + 引擎/会话读数（只读） |
| `crates/yeban-mcp/src/domain/extension_pure.rs` | +373 / −0 | **零依赖**纯逻辑（lane/curve 词表、来源二选一、确定性标签） |
| `crates/yeban-mcp/src/domain/automation_audit.rs` | +319 / −0 | **零依赖**审计：无第二份自动化求值 |
| `crates/yeban-mcp/src/domain/extension_audit.rs` | +552 / −0 | **零依赖**审计：写路径 / `dryRun` 入口 / 错误码词表 |
| `crates/yeban-mcp/tests/extension_tools.rs` | +1260 / −0 | 12 条运行期判据（①..⑧ 的主体） |
| `crates/yeban-mcp/src/domain/mod.rs` | +227 / −5 | `Domain.session` + 引擎镜像 + 3 个 `Plan` 变体 + plan/apply 接线 |
| `crates/yeban-mcp/src/tools.rs` | +208 / −15 | 注册表 15 个工具 + 扩展清单常量 + 判据口径收窄 |
| `crates/yeban-mcp/tests/contract.rs` | +129 / −35 | 扩展工具逐字段对账（含"十工具不得出现在扩展节"） |
| `schemas/mcp-tools.schema.json` | +188 / −2 | enum + 3 条 if/then + 3 个 `$defs`（**只新增**，十工具定义一字未改） |
| `crates/yeban-mcp/src/samples.rs` | +15 / −8 | 新实参的确定性夹具 + 文档口径 |
| `crates/yeban-mcp/tests/tools_e2e.rs` | +17 / −0 | 逐工具表补 3 条（`TOOLS.len()` 断言要求） |
| `crates/yeban-mcp/src/undo_session.rs` | +7 / −1 | `production_region` 提为 `pub`（供跨模块判据比对） |
| `crates/yeban-mcp/src/domain/render.rs` | +8 / −2 | `decode_error_class` / `pcm_format_name` 提为 `pub(crate)`（复用而非复制） |

**净变化：+5442 / −68 = +5374 行**（其中 `schemas/` 之外的源码与判据 +5254/−66）。

已核对的纪律：

- `git add -A` 之前看过 `git diff --cached --stat`（只动了本线拥有的路径）；
- 契约文件**只新增**：`git diff` 的删除行只有 2 处（`"yeban_redo"` 的逗号、`ExtensionToolArguments`
  的说明段），十工具的 `$defs`/参数**一个字节都没改**；
- 根 `Cargo.toml` / `Cargo.lock` **未动**（零新增依赖）；`scripts/**`、`.github/**`、
  `docs/adr/**`、`docs/YEBAN_*.md`、README、法务文件、其它 `crates/**` 均未动。

---

## 9. CI 判决留痕

### 9.1 第 1 轮：`run 37274474454`（commit `0958f4b`）—— **红**

| job | 结果 | 红在哪一步 |
| :--- | :--- | :--- |
| `plan` / `deny` / `lockfile` | ✅ | — |
| `checks (fmt / 红线守卫 / schema)` | ❌ | **三方对齐矩阵**（§7：集成者的 `feature-alignment.md` 还没点名三个新工具） |
| `rust (workspace 全量)` | ❌ | `clippy --workspace --all-targets -- -D warnings` |
| `windows (yeban-mcp / yeban-model 的平台分支)` | ❌ | `clippy -p yeban-model -p yeban-mcp --all-targets` |

两个 Rust 腿红在**同一条**编译错误原文（本机编不出来 ⇒ 只有 CI 能抓到这一类）：

```text
error[E0382]: borrow of moved value: `hash`
  --> crates/yeban-mcp/src/domain/import_audio.rs:293:37
264 |             let hash = AssetHash::parse(text).map_err(|error| {
    |                 ---- move occurs because `hash` has type `yeban_model::AssetHash`,
    |                      which does not implement the `Copy` trait
292 |                 hash,
    |                 ---- value moved here
293 |                 format!("asset:{}", hash.as_str()),
    |                                     ^^^^^ value borrowed here after move
help: consider cloning the value if the performance cost is acceptable
```

处置：把来源标签**在**把 `hash` 移进元组**之前**算好（不 clone）——

```rust
let source_ref = format!("asset:{}", hash.as_str());   // ⚠ AssetHash 不是 Copy
(None, facts, hash, source_ref, declared)
```

并把这条教训写进该处的注释（下次有人"顺手"调整元组顺序会再看到它）。同一批里
`fmt` / 机械守卫 14 条 / 其余 5 条文档守卫全绿。

> 这一轮同时给出一个方法论读数：**"本机不能编译"的代价是真实的** ——
> 本机 25 条零依赖判据 + 4 条文本守卫 + 契约静态对账全绿，**仍然**漏掉一个
> `E0382`。所以"文本级守卫"能替代的只是"能从源码文本判定的性质"，**不能**替代类型检查；
> 类型错误只能在 CI 上被抓到（这也是 `AGENTS.md` §5 把重依赖 crate 交给 CI 的代价之一）。

### 9.2 第 2 轮：`run 37274788668`（commit `ab9738c`）—— **红**（两条 clippy，已修）

| job | 结果 | 红在哪一步 |
| :--- | :--- | :--- |
| `plan` / `deny` / `lockfile` | ✅ | — |
| `checks` | ❌ | 仍是三方对齐矩阵（集成者的表，§7） |
| `rust (yeban-mcp)` | ❌ | `cargo clippy -p yeban-mcp --all-targets -- -D warnings` |
| `windows` | ❌ | 同两条 clippy |
| `rust (yeban-ui-mcp)` | ❌ | **同两条**（`yeban-ui-mcp` 依赖 `yeban-mcp` ⇒ 它的 lib 被一起编） |
| `rust (workspace 全量)` | skipped | 依赖腿失败 |

原文（两条，都在 `clippy::all` 里）：

```text
error: the borrowed expression implements the required traits
   --> crates/yeban-mcp/src/domain/automation.rs:260:44
    |   "target": serde_json::to_value(&self.target)...
    |                                            ^^^^^^^^^^^^ help: change this to: `self.target`
    = help: ... clippy::needless_borrows_for_generic_args

error: useless use of `format!`
   --> crates/yeban-mcp/src/domain/extension_pure.rs:120:55
    |        LaneKind::TrackVolume | LaneKind::TrackPan => format!("{track_id}"),
    |                                                       ^^^^^^^^^^^^^^^^^^^^^ help: consider using `.to_string()`
    = help: ... clippy::useless_format
```

处置（两条都是"语义不变、写法更直白"）：`AutomationTarget` 是 `Copy` ⇒ `to_value(self.target)`；
`format!("{track_id}")` ⇒ `track_id.to_string()`。

> 方法论读数（第二轮）：这一轮红的是 **`clippy::all` 的写法级 lint** —— 本机的
> `cargo fmt --check` / 零依赖 `rustc -D warnings` / 契约静态对账**都覆盖不到**它，
> 因为本机根本没有 clippy 跑在这个 crate 上（含重依赖 ⇒ SKIP）。这也是把
> `yeban-mcp` 的 clippy 交给 CI 的代价，只能靠"少写会被 lint 的写法"来降低频率：
> 本线在这一轮之后把**新写的 `format!("{x}")` 与 `to_value(&copy)` 全部清掉**。
