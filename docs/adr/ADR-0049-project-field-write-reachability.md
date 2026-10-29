# ADR-0049 — 四个**读得到、写不了**的工程字段（`audio_config`／`rng_seed`／`metadata.description`+`tags`／`solo_safe`）：要不要开放为可写，四份产品边界各一条裁决

- **状态**: **Proposed**（待人类批准；⛔ 集成者代签 —— 本票不改任何代码，只把 S3 第一片实测出的
  零触点面、每个字段的**消费者读数**、以及"开放写入会撞到哪条既有保证"登记成可批可驳的裁决）
- **决定者**: 待人类裁决（集成者提出）
- **关联**: `ADR-0001`（D43：这些字段的 serde 严格性，现读注释就在字段上方）、
  `ADR-0023:79-82`（`folder_id` 那条"工具面没有通路 = 诚实的边界"的**写法先例**）、
  `ADR-0030:10,107-108`（`solo_safe` 的**渲染可读性语义**，⚠ 本票问的不是语义而是"能不能写"）、
  `ADR-0037`（"给了必须被消费"的耦合规则 —— `metadata.description` 这一条正卡在它上面）、
  `ADR-0042`／`ADR-0047`／`ADR-0048`（同类"Proposed 待批、本票不改代码"的写法先例）、
  `MODEL-AST-002`（`audio_config` 是采样率/pan law 唯一来源）、`MODEL-AST-005`（`rng_seed` 的跨机确定性）、
  `ARCH-DSP-002`（"设备 44.1/96 ↔ 工程 48k"由重采样兜住的那段前提）、`MCP-TOOL-001`
- **实现落点**: ⛔ 本票无落点。若批任何一条，落点在 `crates/yeban-model/src/ops.rs`（新增 `Op` 变体）
  ＋ `crates/yeban-mcp/src/op_schema.rs`（`kind(...)` 描述）＋ `schemas/mcp-tools.schema.json`
  ＋ `crates/yeban-model/tests/field_write_reachability.rs`（**S3 常驻判据会按设计变红**，见"闭环"一节）。

## 背景（先测后改，AGENTS.md §6）

**这个数字在数什么**：单位＝**次**。对 `crates/yeban-model/src/ops.rs` 的**产品区**
（`#[cfg(test)]` 之前、去注释）数 `.<字段名>` 出现次数 —— 即"操作日志层有没有写过这个字段"。

这条测量已经被**机械化**成常驻判据（S3 第一片，`crates/yeban-model/tests/field_write_reachability.rs`，
落地于主干 merge `be8e1492`）：五个字段 **0** 触点钉死，另以 `routing_graph` **12**／`macros` **5**／
`master_bus_track_id` **1** 作探针表（证明扫描器不是瞎的，并设棘轮下界）。

⚠ **口径边界**（写在该文件头，接手⛔ 误读）：这是**词法**判据，只能钉"零触点"这一半；
宽口径"这个字段到底能不能改"**不可**用词法构造（三条假阴性机理：链式换行 `ops.rs:1703`／
helper 调用 `ops.rs:3043` `doc.insert_track(...)`／局部绑定后赋值 `SetMacro`）。
要判"能不能改"得走**语义差分**（施加每个 `Op` 变体后逐字段比对序列化），那是另一片的设计活。

**读面是通的**：`crates/yeban-mcp/src/domain/view.rs:36` 的 `SELECTORS: [&str; 28]` 现读**含**
`rng_seed`／`audio_config`／`metadata`，且 `project_view` 是整档序列化后按根键挑选（`view.rs:229`）
⇒ 这四个字段**调用方读得到**，写不了。

**写面上"工程头/轨道级"已经开放了什么**（现读 `op_schema.rs` 的 `kind("…")` 去重集，共 **53** 个）：

| 层 | 已有可写通路（`kind` 名） | 同层⛔ 没有通路的字段 |
| --- | --- | --- |
| 工程头 | `setTitle`／`setAuthor`／`setTempo`／`setTimeSignature`／`setKeySignature`／`setTransportConfig` | `audio_config`、`rng_seed`、`metadata.description`、`metadata.tags` |
| 轨道（混音三开关族） | `setTrackMute`／`setTrackSolo` | `solo_safe`（同族第三档） |
| 轨道（增益族） | 走自动化泳道 `AutomationTarget::TrackVolume`/`TrackPan`（`ops.rs:3388`/`:3392` 实写） | ⛔ 无"一次性设音量"的 kind（只有自动化目标这条形态） |

⇒ 本票的四个问题**不是**"整套工程属性都不可写"，而是**四个具体的同族不对称**。

## 逐字段实测（每条都带现读位置；单位＝命中行数，统计面 `crates/*/src`）

### 1. `audio_config`（`sample_rate`／`block_size`／`bit_depth`／`pan_law`）

- 结构体：`crates/yeban-model/src/project.rs:347-356`（四档，全部**规范枚举**，⛔ 自由数值）。
- 顶层无冗余副本：`project.rs:20-21` 的文档契约 + `:2479-2491` 的常驻测试（默认工程序列化后
  `audio_config.sample_rate` 必须是整数 `48_000`，且顶层⛔ 出现 `sample_rate`/`pan_law`）。
- **消费者**（`audio_config` 在 `crates/*/src` 命中行数）：`yeban-app` **26**／`yeban-engine` **24**／
  `yeban-model` **17**／`yeban-mcp` **6**／`yeban-render` **3** ⇒ 这是一份**被到处投影**的配置，
  ⛔ "改了没人看见"那一类。
- 关键落点：`crates/yeban-engine/src/device.rs:341-346` 把 `audio_config` **投影成设备参数**
  （`sample_rate` / `block_frames`），`snapshot.rs:835` 亦按 `block_size.frames()` 取块长。
  ⇒ 改 `block_size`/`sample_rate` **不是改一个数**，是**流重配置**（设备重启）。
- 规范侧前提：`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` `ARCH-DSP-002`（`:759-760`）写的是
  "**工程设定为 48kHz**、声卡工作在 44.1/96 时由 `rubato` 重采样兜住" ⇒ 该条把"工程速率"当**前提**在用，
  ⛔ 明文禁止改，但也没有为"改工程速率"设计过任何通路。
- ⚠ 另一件**同形但不同层**的事：**逐次导出**的采样率/位深**已经**有选项 —— UI 侧那条刚落地
  （`crates/yeban-app/ui/dialogs/export_wav_dialog.slint`，随主干 merge `83954451` 进来，
  线提交 `fdb17b75`／`b907e371` —— ⚠ 本会话外的并行写者），
  其回执字段闭集 `[&str; 13]`（`crates/yeban-app/tests/ui_export_dialog.rs:82-96`）现读含
  `sample_rate` 与 `bits`；而 **MCP 侧渲染走工程速率**（`domain/render.rs:3537`
  现读 `let sample_rate = project.audio_config.sample_rate.hz();`）。
  ⇒ "想要 44.1k 的 WAV"这件事在 UI 面**已经**有答案，本票问的是"要不要能改**工程本身**"。

### 2. `rng_seed`

- 字段：`project.rs:1658-1663`（`pub rng_seed: u64`，注释点名 [MODEL-AST-005] 与 **ADR-0001 D43 必需**，
  理由是"缺字段会让同一份乐谱在默认种子下静默换一套触发结果 —— 听得见、读文件时看不见"）。
- **它真的在决定音频**：`crates/yeban-mcp/src/domain/mod.rs:4622`、`domain/sfz.rs:1602`、
  `crates/yeban-app/src/export_wav.rs:714` 都用 `note.triggers(project.rng_seed)` 裁决
  `probability` 音符响不响；`crates/yeban-app/src/mcp_mount/embedded_digest.rs:1447`/`:1725` 用
  `dither_rng_for(project.rng_seed, master_bus_track_id)` 决定**抖动**；
  `domain/render.rs:2044` 把"这次渲染实际使用的种子"报进回执。
- `.als` 导出侧把它当**损失项**上报（`crates/yeban-render/src/als.rs:516-523`：
  "工程 `rng_seed` —— 它决定 probability 的触发判定"）。
- ⚠ **别与已开放的 `melodySeed` 混为一谈**（`ADR-0045`，已 Accepted）：那票开放的是
  **生成旋律的取音种子**（theory 内部 `melody_contents` 的第 5 实参），
  `rng_seed` 是**工程级的熵输入**（概率触发 + 抖动 + 摘要）。两者不同层，⛔ 互相替代。
- ⇒ 开放 `rng_seed` = 开放"**换一遍混音/触发结果**"，是音乐行为，⛔ 元数据编辑。

### 3. `metadata.description` ＋ `metadata.tags`

- 结构体：`project.rs:363-385`。⚠ 该结构**有意不含** `title`/`bpm`/`time_signature`
  （`:360-361`：顶层是唯一权威，重复一份就产生两个事实源）⇒ "工程头注记"这一族**已经被裁决过一层**。
- `description`：`#[serde(default)]` 是**语义**（空串＝定义上的"没有描述"），⛔ 工程行为输入。
- `tags`：同族语义默认（空集＝没有标签）。
- **消费者读数**（这才是本票的关键实测）：
  - `metadata.created_at_unix_ms` / `modified_at_unix_ms` 有人读
    （`crates/yeban-render/src/als.rs:483-484`、`crates/yeban-mcp/src/domain/render.rs:3343` 的 BWF bext 戳、
    `crates/yeban-app/src/cli.rs:5535` 等）；
  - `metadata.tags` 的读取面 = **恰好 1 处**（`als.rs:485` 的 `project.metadata.tags.len()`，进 `.als` 损失表文本）；
  - `metadata.description` 在 `crates/*/src` 的读取面 = **0 处**（`grep -rn "metadata\.description" crates/ docs/ schemas/`
    在 `src` 下**零**命中；命中的 `description:` 全是**别的**结构体的同名字段 —— 操作/宏/提案的 `description`。
    工程侧唯一**写入**者是 `crates/yeban-app/src/import_logic.rs:941`（Logic 导入时**填**它）；
    ⚠ 同名字符串在仓库里另有两处，都⛔ 是消费者：`crates/yeban-model/tests/schema_ratchet.rs:85,112`
    —— 那是 JSON schema 的**棘轮登记行**（钉"这个键总是写出、读取器接受缺失"），读的是 schema⛔ 字段值）。
- ⇒ 这一条与 `ADR-0037` 的"给了必须被消费"直接相关：⚠ **开放写入并不补足缺口**，
  因为缺的是**消费面**（UI 显示／库检索／导出元数据袋），⛔ 写入端。

### 4. `solo_safe`

- 字段：`project.rs:1229`（`pub solo_safe: bool`，注释 [ADR-0001 D43]：**必需**，理由原文是
  "它改变 solo 的混音结果，且与同族字段 `mute`/`solo`（都无默认值）对称 —— 三个混音开关要么都写，要么都别写"）。
- **它有行为重量**：`ADR-0030:10,107-108` 的渲染可读性公式
  `track_is_audible = !mute && (!any_solo || solo || solo_safe)`；
  消费者行数 `yeban-engine` **19**／`yeban-model` **14**／`yeban-app` **14**／`yeban-mcp` **11**／`yeban-render` **9**。
- 同族另两档**都可写**（`setTrackMute`／`setTrackSolo`）⇒ 不对称是**具体的**，不是笼统的"轨道属性不可写"。
- ⚠ 另外：`ops.rs:525` 把 `bypassed` 归为"行为开关（与 `TrackV3::solo_safe` 同类）"，
  而 `setDeviceBypassed` 已存在 ⇒ 仓库里**已有**"行为开关应当有写通路"的先例，本票的问句因此是
  "**solo_safe 是不是也走这条路**"，⛔ "`solo_safe` 是不是根本不该可写"。

## 为什么⛔ 集成者自行发明答案（四条各自的硬约束）

1. `audio_config`：撞**设备层**。批"可改"必须先回答"改的时候引擎在跑怎么办" ——
   `device.rs:341-346` 的投影意味着它不是脏标记问题，是流重配置问题；
   还要回答 `block_size` 变更对 `ARCH-RT-*` 零分配回调与 `ARCH-PDC-*` 延迟预算的连带影响。
   ⛔ 这些都在实时契约面上，一次产品裁决⛔ 兜不完。
2. `rng_seed`：撞**确定性保证**。`MODEL-AST-005`／`TEST-SPEC-*` 那一族的立身之本是
   "同一份工程在任何机器上触发判定相同"；开放写入本身不破坏它（写进去之后仍确定），
   但"谁被允许改、改完是否要让既有 `.als`/回执里的种子读数作废"是产品面。
3. `metadata.description`+`tags`：撞 `ADR-0037`。**只开写入 = 给了不被消费**；
   要开就得同时决定消费面（UI 哪一处显示？导出容器带不带？），那是功能立项⛔ 补洞。
4. `solo_safe`：撞**"三个开关要么都写"的既有裁决本身**（`ADR-0001 D43` 用对称性论证了 serde 严格性）。
   把它开放为可写 = 在 `mute`/`solo` 那条 kind 家族上**新增第四条例外或第三条例外**，
   属于对该 ADR 语义的扩面，需人类点头。

## 五条可选路线（每张票独立可批／可驳）

> ⛔ 本票不建议任何一条。列出是为了让裁决有一致的选项名，且每条都写了"批了要跟着做什么"。

- **D1｜维持现状 + 明文登记为边界**（最小成本，`ADR-0023:79-82` 的既有写法）：
  只做文档 —— 在 `schemas/mcp-tools.schema.json` 相应工具描述与 `docs/DEV_WORKFLOW.md`
  的边界清单里点名"这四个字段可读不可写"，S3 判据继续钉零触点。代码 0 行。
- **D2｜只开纯注记族**（`metadata.description` + `tags`）：⛔ 单独开写入（违反 `ADR-0037`），
  必须同时批一个消费面（建议形态：`yeban_query_project` 的 `metadata` 视图已经能读到 ⇒
  真正要补的是 UI 显示或导出容器写入）。票面要一并裁决消费点。
- **D3｜开 `solo_safe`**（沿 `setTrackMute`/`setTrackSolo` 同形新增 `setTrackSoloSafe`）：
  技术上与那两个变体同构（一个 `bool`、一次 `track_mut`、`old/new` 进回执），
  但要一并裁决 `ADR-0001 D43` 那句"要么都写"是否被扩面。
- **D4｜开 `rng_seed`**（工程级换熵输入）：需同时裁决三件事 —— ①是否需要 `validate()` 侧任何约束
  （`u64` 全域，界由类型面给，同 `ADR-0045` 写法）；②与 `melodySeed`（`ADR-0045`）的**层级关系**要不要写进契约文本；
  ③改种子后既有渲染回执里的 `seed` 读数是否算**作废**。
- **D5｜`audio_config`**（单独一票，⛔ 与 D2/D3/D4 挤在一起）：批之前需要一份"设备生命周期"裁决
  （改速率/块长是**仅在引擎未启动时允许**，还是运行中允许并触发重配置）。这一条最像产品边界，
  也最贵，建议**最后**裁。

## 闭环：这批判据会怎么配合

S3 的常驻判据 `crates/yeban-model/tests/field_write_reachability.rs` 现在**钉着五个**字段的零触点
（第五个 `folder_id` 已由 `ADR-0004` + `ADR-0023:79-82` 裁过，属"已登记待补实现"⛔ 待裁，
见本会话台账"2026-10-30（凌晨）· S4 第一片"一节）。⇒ 若人类批 `D2`/`D3`/`D4`/`D5` 任一条，
**该判据会按设计变红**，实现片的第一个动作就是把对应字段从"零触点名单"移到"探针表"
（探针表要求 ≥1 触点 ⇒ 移过去即是被机械化盯住的扩面，⛔ 悄悄放宽）。

## ⭐ 补账（2026-10-30）：本票的"零写者"现在有两种口径

S5 差分片（线 `line/s5-op-write-differential`，tip `0c450809`，判决 **pending**）在
`crates/yeban-model/src/ops.rs` 的测试模块里对**每个 `Op` 变体**（现读 **55** 条，⛔ 手抄）
各施加一次，比较施加前后的 `serde_json` 树并取**最小差异路径集**，判"值到底变了没"。
⇒ 这是「逐字段实测」那套**词法零触点**断言之外**互相独立**的第二半；批任一条路线时两片会**同时**红。

差分口径额外机械给出两件事（原来只有推断）：

1. **顶层无写者字段全集＝8**（判据 `the_never_written_top_level_fields_match_frozen_dead_set`）：
   `audio_config`／`rng_seed`／`metadata`（本票三张子票）＋ `id`／`schema_version`／
   `min_reader_version`／`writer_version`／`master_bus_track_id`。
   - 前四个是**结构上不可能有写面**：改 `id`＝换一份工程，三个版本字段由**写入器**产出而⛔
     由领域操作改写（这里只陈述机制，⛔ 替人类裁什么）。
   - ⭐ `master_bus_track_id` 是差分口径**新暴露**的一项，本票原来⛔ 列进待裁名单：
     它有 **1** 处读触点（S3 探针表钉着）而 **0** 处写点 ⇒ "换主总线"这件事在 `Op` 面
     不存在。⛔ 本票代人类裁定它该不该做；只登记为**要不要另开一张票**的问题。
2. **每个变体的顶层写面表（55 行棘轮）**（判据
   `op_write_differential_root_keys_match_frozen_table`）＋一条**检测器自证**
   （`a_sole_change_of_each_registered_field_is_attributed_to_that_field`：人造写者逐字段只改一个，
   差异必须**只**归到它，`folder_id` 走"键新增"、`solo_safe` 走"值改变"两条分支各测一次）。
   ⇒ 实现片落地时要**同时**动三处：棘轮表、死集、`field_write_reachability.rs` 的零触点名单。
   三处一起红＝设计意图；⛔ 只删其中一处来让它绿。

## 影响面

- 代码：⛔ 本票 0 行（纯文档）。
- 契约：若批任一条 ⇒ `schemas/mcp-tools.schema.json`（新增 `kind` 描述与字段）
  ＋ `AGENTS.md` §4.2 的工具计数句需**现读**更新（该节已两次因手抄数字而陈旧，见其自带警告）。
- 判据：`field_write_reachability.rs`（S3 常驻）＋ 各票实现片自带的语义判据。
