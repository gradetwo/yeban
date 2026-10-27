# ADR-0030 — 分轨导出（stem export）：一次调用完成逐轨 32-bit Float 落盘

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: 审计 `G9b`（`/tmp/yeban-issues-20261027/GAPS.md`）；规范 `ARCH-DSP-003`
  （音轨冻结与母带分轨：**32-bit Float** 广播级 WAV）、`MCP-TOOL-002`
  （`yeban_render_master` 既有单文件母带口径）、`MODEL-ISO-001`（工程/会话/本机配置三层隔离）、
  `ADR-0026`（生成声部分级电平，母带不再首听削波的既有基线）；
  代码事实源：`crates/yeban-mcp/src/domain/render.rs`（`render::build`、
  `render::track_contributions`、`track_is_audible` / `any_solo` / `solo_safe`、
  `guard_output_path`）、`crates/yeban-render/src/dither.rs`（`BitDepth::Float32`，
  整数满量程与 LSB 缩放对浮点容器是 `panic!`）、`crates/yeban-model/src/store.rs`
  （`write_project_atomic` 仅**单文件**原子）
- **日期**: 2026-10-27

## 背景

`G9b` 的实测缺口：工具面**只有母带**。想要"每条轨一个 WAV"（喂给外部混音、
交给合作者、或做 A/B 替换某个声部），今天必须由调用方自己完成
**N × (克隆工程 → 只留一条轨 → `yeban_render_master` → 换文件名)** 的往返，
且每一次往返都要自己保证 `.yebanproject` 落盘、路径不覆盖、命名不漂移。
这不是能力缺失（渲染器已能单轨出声），而是**一次调用**这件事没被做出来。

规范侧 `ARCH-DSP-003` 已把分轨口径写成 **32-bit Float**，与母带现行的
24-bit 整数 + TPDF 抖动（`ADR-0026` 以来的削波基线）**不同**。本 ADR 把两种口径
的分工写清楚，⛔ 不为了"统一"去动母带判据。

## 决定

### D1 — 新增扩展工具 `yeban_export_stems`，不重载 `yeban_render_master`

母带响应是**单文件**语义（`path` / `sha256` / `bytes` / `masterDigest`），
分轨是**N 文件**语义（逐文件回执 + 批次成本 + 部分成功状态）。塞进同一个工具会让
两侧的回执都要写成"另一半可能不存在"的形状，判据也随之翻倍。

- 注册为**扩展段**（`EXTENSION_NAMES` 追加），`spec_id = "MCP-TOOL-EXT-EXPORT-STEMS"`；
  `EXTENSION_TOOL_COUNT 14 → 15`、`TOOL_COUNT 24 → 25`。
- `schemas/mcp-tools.schema.json`（现读结构，⛔ 不凭记忆写键名）：根层
  `properties.name.enum` 追加工具名 + `allOf` 新分支（`const: "yeban_export_stems"`
  与 `$ref` 指向 `$defs`）+ `definitions.ExtensionToolArguments.$defs` 新条目。
  调用/响应**样本不在 `schemas/` 目录**（那里只有 4 个 `.schema.json`），而在
  `crates/yeban-mcp/src/samples.rs`：`export_all()` 按 `FILE_PREFIX = "mcp-tools"` /
  `CALL_FILE_PREFIX = "mcp-tools.call."` 生成，判据
  `exported_call_samples_are_contract_shaped_tool_calls`（`tests/contract.rs:250`）
  逐个读回并核对形状 ⇒ 新工具必须在**该模块**登记样本，⛔ 不是手写 JSON 文件。
  （扩展段的 `$defs` 与 Rust 注册表**逐字段**对测，见 `tests/contract.rs:597`；
  文档化的十个工具不许出现在扩展段 `$defs` 里）。
- `docs/ledger/feature-alignment.md` 增加一行，`crates/yeban-mcp/tests/props_176.rs`
  的 `TOOL_COUNT` 字面量同步。

### D2 — 位深：分轨 **32-bit Float**，母带口径不变

`ARCH-DSP-003` 直接给出答案，且 `yeban-render` 的编码器**早已支持**
`BitDepth::Float32`（`is_integer_pcm()` 对浮点返回 `false` ⇒ 不做抖动、
不做 LSB 缩放、不做整数满量程钳位）。因此：

- 分轨默认 **32-bit Float**（规范口径，也是下游二次混音唯一不掉精度的选择）。
- **现读的实现事实**（决定"要不要动 `yeban-render`"）：
  `quantize(input, BitDepth::Float32, rng)`（`dither.rs:303`）是**直通**分支
  —— 不做抖动、非有限样本归 `0.0`；`PcmBuffer::Float32::to_le_bytes()`
  已存在（`dither.rs:286`）；容器侧 `rf64.rs:241` 已有 IEEE 浮点 `WaveFormat`
  构造器与 `format_tag()`（`WAVE_FORMAT_IEEE_FLOAT`）。
  ⚠ 一条**必须遵守的容器规则**（`rf64.rs:17` 明确写着的）：
  **非 PCM（IEEE 浮点）容器必须带 `fact` chunk**（`u32` 帧数，位置在 `fmt ` 之后、
  `bext` 之前，结尾回填）—— 浮点分轨若漏掉它就是坏文件。
  ⇒ 本片**不需要**改 `yeban-render` 的编码器，改动集中在 mcp 侧的调用与头构造。
- **不**新增 `bitDepth` 实参：多一档就多一套判据与"整数档要不要抖动"的追问，
  而规范只登记了一种。要 16/24-bit 的调用方，母带工具与外部工具链都在。
- 母带的 24-bit + TPDF 逐字节不动（削波与 `masterDigest` 判据是既有回归锚）。
- 每条分轨回执各带 `headerBytes` / `payloadBytes` / `sha256` / `bytes`，
  与母带同名键**同形**，便于调用方复用同一套校验代码。

### D3 — 单轨渲染 = 既有 `render::build` 的克隆-改-重渲染，⛔ 不另起渲染器

沿用 `track_contributions` 已验证的形状：克隆工程、把目标轨之外的轨按既有
`mute` 机制处理、以 `render::build(project, path, &request, now_ms, assets)`
重渲染。**约束（照抄 `track_contributions` 的既有教训）**：

- `path` 必须**逐轨不同**（该先例的 `path: None` 会让产物互相覆盖）；
- 分轨走各自的 `path`，但**共用同一次 `now_ms`** ⇒ `creationTimestamp` 一致，
  批次内可比；
- 母带链上的插入**不参与**分轨输出（分轨是轨后输出，不是总线快照）。
  **实现口径**：`project_without_audition` 在克隆上把母带轨的 `devices` 清空
  （`master.devices.clear()`），母带轨的 `volume_db` / `pan` **仍生效**（`build` 在
  `render.rs:2860` 一带照旧施加 `master_gain` 与母带声像）—— 那是**整批共用的同一个
  增益**，不改变各条 stem 之间的电平关系，混回去仍然对齐。口径与 Logic
  "导出单独音轨时总线效果器不进文件"一致。

#### D3a — 批次公共帧数需要一次**基线渲染**（`N + 1` 次，不是 `N` 次）

现读事实：`build` 在 `request.frames == None` 时的帧数来自
`end_tick = max(逐源 report.end_tick)`（`render.rs:2676` / `:2693`），
而源是按**可闻门**筛出来的 ⇒ "只独听第 k 轨"的那次渲染会把时间轴**缩短到第 k 轨的末端**。
一套要混回去的 stem 必须等长，因此本片：

1. **先**做一次"全部轨去掉试听状态"的基线渲染，只取它的 `frames`（字节测出来即丢弃，
   ⛔ 不落盘、不进回执）；
2. 每条分轨的请求把 `frames` **钉成**这个基线值（与 `track_contributions` 钉
   `master.frames` 同一条理由）；
3. `stemsBasis.renders` 因此是 `N + 1`，⛔ 不报 `N`（多出来的那一次是真实成本）。

⛔ 不自己算"工程有多长"：那条公式的**唯一**实现就在 `build` 里，复制一份必然漂移
（`G1` / `WALL ⑦` 的既有教训）。

### D4 — 试听状态（`mute` / `solo`）不参与分轨选取，但必须如实回报

`track_is_audible(track, any_solo)` 的语义是 `!mute && (!any_solo || solo || solo_safe)`：
一旦工程里存在任何 `solo`，非 `solo_safe` 的轨就**不发声**。分轨是给下游用的素材，
不是试听快照 ⇒ 若照搬这条门，批次内容会随用户上一次按了哪个独听键而漂移。

- 默认批次 = 工程里**全部**轨（`trackIds` 可筛选）；被 `mute` 的轨照样出文件，
  其内容按 D3 的克隆口径就是**它自己**的信号（静音只影响总线，不影响自身渲染）。
- 回执逐轨附 `sourceMute` / `sourceSolo` 读数 + 批次级 `auditionStateIgnored: true`，
  让调用方能判"这条本来在工程里是静音的"。⛔ 不静默丢轨（`G1` 的教训：
  静默丢源就是假话回执）。

### D5 — 部分成功是**一等状态**，不是异常尾巴

`store::write_project_atomic` 只保证**单文件**原子，跨 N 个文件没有事务。
第 k 条失败时，前 k−1 条已经在盘上。任何"要么全有要么全无"的措辞都是假话。

- 成功：`stems: [{trackId, name, path, sha256, bytes, headerBytes, payloadBytes}, …]`、
  `partial: false`。
- 中途失败：`code: IO_ERROR`（⭐ **原稿写的 `STEM_WRITE_FAILED` 已作废**：`ADR-0001` D25
  的 20 值联集是**封闭**的，判据
  `tests/contract.rs::extension_tools_only_declare_codes_inside_the_d25_union` 会直接拒绝
  发明新码；本片的失败**就是** I/O 失败，不需要新名字），`data` 必带
  `errorKind: "stemWriteFailed"` + `completedStems`（已成功落盘、**保留不删**）、
  `failedTrackId` / `failedReason`（⭐ 键名刻意**不是** `reason` —— 不新增 `"reason"`
  字面量站点，免得动 `fault_vocabulary` 的冻结摘要）、
  `orphanedFiles`（盘上存在但本批未完成的文件路径，含目标文件被中断时的残留）。
  回执同时给 `resumeFrom`：下一次调用可以只补 `trackIds` 里的剩余项。
- ⛔ 不做"自动回滚删除"：删用户盘上的文件比留一个如实的 `orphanedFiles` 危险。

### D6 — 成本读数 `stemsBasis`，与 `contributionsBasis` 同口径

分轨的代价是 **`N + 1` 次整工程长度渲染**（多出的那一次是 D3a 的基线帧数探针），
这必须自己说出来（`WALL ⑦` 的先例：
逐轨贡献上线时同时给 `contributionsBasis`，⛔ 不给"免费的错觉"）。

- `stemsBasis: {renders: N + 1, basisRender: "projectWide", audioFramesPerRender: F, sampleRate: R, basis: "per-track re-render of the full project window"}`；
- `bytesTotal`（本批产物字节合计，单位：字节）与 `maxFileNameBytes`。

### D7 — 文件名：确定性清洗 + 撞车即整批拒绝，⛔ 不猜也不改名

轨名可以带 `/`、空格、`..`、全角字符，直接当文件名是路径穿越与覆盖的门。

- 每个 stem 文件名 = `<index>.<sanitized-track-name>.wav`，`index` 是**工程内轨顺序**
  的十进制（`0001` 起，宽度按总轨数），保证同一工程两次导出**逐字节同名**；
- 清洗规则：保留 `[0-9A-Za-z._-]`，其余一律 `_`，首尾的 `.`/`_` 去掉，
  空结果回退成 `track`（回退是**确定的**，不是随机）；
- 每个目标路径都过 `guard_output_path`（既有防覆盖 / 防穿越门），再加一条**批内
  存在性预检**：目标文件**已在盘上**就整批拒绝（`INVALID_PARAMETER_RANGE` +
  `conflictingPaths` / `why` / `hint`）。理由是 `guard_output_path` 只保护**工程文件与其锁文件**，
  对目录里的其他同名 `.wav` 不管；而 `write_project_atomic` 那类落盘是**覆盖**语义。
  同一批内两个清洗后名字撞车由 `index` 前缀天然消歧（`index` 唯一），
  不做隐式 ` (1)` 之类的手写去重；
- **预检发生在 `export`（渲染）阶段, 写盘在其后的 `write` 阶段** ⇒ 因碰撞拒绝时
  一个字节都还没落下，不出现"半批新文件 + 半批旧文件"；
- `outDir` 必填且必须是**已存在**的目录，否则拒绝。⚠ **就地更正（2026-10-27，实测两层落点）**：「否则」在实现里分成两层 —— **缺失**由 `schemas/mcp-tools.schema.json` 的 `required` 在参数校验层先拒（JSON-RPC `-32602` + `data.detail` 点名 `outDir`，请求根本走不到领域层，口径同 `tests/effect_contract.rs` 判据 ⑤）；**给了但不是目录**（不存在 / 是文件）才走领域层 `INVALID_PARAMETER_RANGE`。原判据把两者写成同一条 ⇒ CI Run `40485603721` 实测为协议层错误，已按两层改写（`tests/export_stems.rs` 判据 7）。
  带 `missing` / `why`（不 `create_dir_all` 到任意深度：分轨是写一堆文件，
  目录写错位置比单文件更贵）。
  ⚠ 参数名**复用**已登记的 `outDir`（`param_bounds_guard` 的 `STRING_PARAM_BOUNDS`
  里已是 `Unbounded`），⛔ 不新造 `outputDir` —— 新名字要同步动那条由 G13 线在飞的
  29 元组，白白造一个跨线写者冲突。

## 探测证据（先测后改，测量纪律 §6）

| 读数对象 | 单位 | 修复前 | 本片判据 |
|---|---|---|---|
| 拿到 N 条分轨所需的 `tools/call` 次数 | 次 | 调用方自拼 `N × (克隆+渲染+改名)`，工具面**无**此能力 | `1` |
| 分轨容器位深 | bit | 不存在该工具 | `32` float（`ARCH-DSP-003`） |
| 批次内容随 `solo` 漂移 | 个（stem 文件数） | —— | 同一工程 `solo` 前后**文件数与字节数不变** |
| 第 k 条写失败后的盘上状态 | 个 | 未定义 | `completedStems = k-1` 且全部可读回 `sha256` |
| `render::build` 复用次数 | 次 | —— | `N + 1`（⛔ 不新写渲染器；多出的 1 次是 D3a 的**基线渲染**，真实成本） |

## 影响与不做的范围

- **不动**：`yeban_render_master` 的响应形状、位深、`masterDigest` 判据；
  `yeban_render_sfz_wav` / `yeban_record_wav` 的 24-bit 口径（它们不是分轨）。
- **不做**：`Stems by Output Path`（总线/输出路径分组）、时间戳分轨段
  （仅导出某小段落）、非 WAV 容器 —— 都登记为后续切片，⛔ 本片不发明。
- **实时线程**：分轨是离线后台路径，不触 `ARCH-RT-*` 红线；
  但⛔ 不得在音频回调内做（沿用 `yeban_render_master` 的调用位置）。
- 判据集中在 `crates/yeban-mcp/tests/export_stems.rs`：
  ① 一次调用出 N 文件且逐文件 `sha256` 可回读；② 位深 32f 的 RIFF `fmt`
  字段（`audioFormat = 3` / `bitsPerSample = 32`）；③ `solo` 不影响批次；
  ④ 第 2 条失败时 `completedStems` / `orphanedFiles` 如实；⑤ `stemsBasis.renders == N`。
