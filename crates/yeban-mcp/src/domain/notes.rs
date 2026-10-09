//! `yeban_edit_notes` 的 `NoteOp` → [`Op`] 编译 [MCP-TOOL-006]。
//!
//! ## 逆操作**不在这里**
//!
//! 本模块只负责把 JSON 编译成 `yeban-model` 的 `Op` 变体。逆操作一律由
//! [`Op::invert`] 提供 —— 在 MCP 层再写一套 `invert` 就会有两份会漂移的真相
//! （判据 `note_ops_are_reversible_through_the_model_inverse` 直接复用模型侧的判据思路：
//! `apply` 之后 `invert` 回去，序列化字节必须回到原样）。
//!
//! ## 规范缺口（`NoteOp` 的形状没有契约）
//!
//! `schemas/mcp-tools.schema.json` 把 `ops` 声明为无约束数组，
//! `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §7.2 只写 `ops: Vec<NoteOp>`。
//! 本模块因此**定义了**四种 `kind`（`add` / `delete` / `move` / `velocity`）作为
//! 本地决策，并把它登记为待裁决项（见 `docs/ledger/tools-domain-notes.md`）。
//!
//! ## `add.note.probability`：让概率触发从工具面**可达**
//!
//! `MidiNote::probability` 是模型既有字段，但改动之前**没有任何工具**能设置它
//! （`parse_note` 不读它）⇒ 模型的概率触发能力在 MCP 工具面上不可达。本模块新增
//! 一个**可选**字段 [`PROBABILITY_FIELD`]（缺省 = 不写 = 必然触发 = 逐字节等于旧行为），
//! 把字面值搬进 `MidiNote`；"这一遍响不响"的裁决**不在本层**，而在
//! `MidiNote::triggers(rng_seed)`（`MODEL-AST-005`，实时引擎与离线母带共用）。
//!
//! 取值的权威判定也在模型层（[`MidiNote::validate`]）：越界 → `ProbabilityOutOfRange`
//! → 契约码 `OUT_OF_RANGE`。本层只额外拦下"不是数字"这类 JSON 形状错误。
//!
//! ## `add.note.ratchet` / `add.note.microTimingTicks`：让**已实现**的渲染能力可达
//!
//! 同一类缺口的第二个实例。离线母带渲染器**已经**按模型语义消费这两个字段
//! （`crate::domain::render` 的连击一节用 `step = (duration_ticks / ratchet).max(1)`
//! 逐脉冲排程；微时序经 `crate::domain::render_math::note_frame_span` 并入起点），
//! 但在这个字段接线之前，17 个工具的**任何一个**都写不了它们 ⇒ 能力已实现、工具面
//! 不可达，而且把 `ratchet` 写进 `ops[].note` 会被**静默丢弃**（`parse_note` 不读它）。
//!
//! 现在：可选字段（缺省 = `None` = 等价于 1 / 无偏移 = 逐字节等于接线之前的行为），
//! 字面值搬进 [`MidiNote`]，区间由模型层把关（`1..=16` / `-240..=240`）。
//!
//! ## `note` 的未知键：响亮拒绝，不静默丢弃
//!
//! 顶层实参已经是这个口径 —— `ToolCall::from_params` 对不在参数表里的键返回
//! `UnknownParam`（"拼错的参数必须被拒绝, 不能静默忽略"）。这条纪律此前**只守了顶层**：
//! `note` 对象是自由形状，多写的键被原样吞掉。现在 [`reject_unknown_note_fields`] 把
//! 同一口径下沉一层，`data.supportedNoteFields` 逐条列出支持集合。
//!
//! **登记边界（本线未接线，绝不静默）**：模型里还有四个表现力字段没有工具面通路 ——
//! `slide` / `pitchBendCurve` / `syllable` / `phonemes`。渲染器对它们一律如实登记进
//! 响应的 `unsupported`（`noteSlide` / `notePitchBend` / `noteLyrics`），而工具面
//! 现在会**响亮拒绝**这四个键（`data.unsupportedNoteFields`），不再吞掉。
//!
//! ## 材料创建形态（`arguments.create: true`）—— 关闭 needs-8 的 MIDI 那一半
//!
//! 台账 `docs/ledger/tools-domain-notes.md:283` 的 **needs-8** 记的事实是：
//! `yeban_propose_section` 要求 `clip_pool` 里至少有一条"MIDI 且至少一个音符"的材料
//! （`section_build.rs` 的 `usable_materials`，缺了报 `CLIP_NOT_FOUND`），
//! 而"没有任何 MCP 工具能让 Agent 把片段放进池子"⇒ 空池工程做不了配器。
//!
//! 那一半缺口由 [`compile_create`] 关闭：`create: true` 时 `clipId` 是**将要新建的**
//! 片段身份（§7.2 的参数表因此**一字不动** —— `clipId` 仍然是必填的"目标片段"），
//! `ops` 里的 `add` 折成一条 `Op::AddClip` 的**初始内容**。
//!
//! 为什么是"扩 `yeban_edit_notes` 的参数"而不是新增工具：
//! `ADR-0001` **D46** 的扩张原则是"先扩既有工具的参数，只有确实不合适才新增工具"，
//! 而新增工具必须同步 `schemas/mcp-tools.schema.json` 的
//! `properties.name.enum` + `ExtensionToolArguments.$defs` + `allOf` 三处
//!（本线禁改 `schemas/**`）。已有的先例是同一条原则下的
//! `yeban_open_project` 的 `create`/`seed`（`domain/project_create.rs`）。
//!
//! 三条刻意设成**响亮失败**的口径（绝不静默降级）：
//!
//! | 情形 | 结果 |
//! | :--- | :--- |
//! | `create: true` 且池里**已有** `clipId` | `CONFLICT`（`reason = clipAlreadyExists`）—— 与 `yeban_open_project` 的 `create` 同款：绝不覆盖 |
//! | `create: true` 且 `ops` 里有 `delete`/`move`/`velocity` | `INVALID_PARAMETER_RANGE`（`reason = createRequiresAddOps`）—— 新片段里还没有音符可以被它们指向 |
//! | `create: true` 且两个 `add` 抢同一个音符身份 | `INVALID_PARAMETER_RANGE`（`reason = duplicateNoteId`）—— 不静默去重 |
//!
//! 创建出来的片段**只在池子里**（本工具不摆放；摆放是 `Op::AddClipPlacement` 的事，
//! 而"配器"只要求池里有材料）。这一点如实写在 [`compile_create`] 的文档与响应里。
//!
//! ## 摆放形态（`arguments.placement`）—— 关闭 needs-6 的"放置/引用片段"那一半
//!
//! 台账 `docs/ledger/mcp-tools-expansion-notes.md` §6 的 **needs-6** 记的事实是：
//! "没有『放置/引用片段』的工具" —— `Op::AddClipPlacement` 在 MCP 侧只有三个写者
//! （`yeban_open_project` 的 `seed`、`yeban_propose_section` 自建的声部、以及
//! `yeban_import_audio` 的**音频**片段），而**已经躺在 `clip_pool` 里的片段**
//! （例如上面 `create: true` 刚建出来的 MIDI 材料）**没有任何工具**能摆到时间轴上
//! ⇒ 渲染器只遍历 `track.clips`，因此那些材料一帧都不出声。
//!
//! [`parse_placement`] 补上这一半：`placement` 在场时，除音符编辑之外再产出一条
//! [`Op::AddClipPlacement`]，把 `clipId` 摆到 `trackId` 的 `startTick` 上。
//!
//! 三条刻意设成**响亮失败**的口径（绝不静默降级）：
//!
//! | 情形 | 结果 |
//! | :--- | :--- |
//! | `placement` 与 `create: true` 同给 | `INVALID_PARAMETER_RANGE`（`reason = "placementIsNotCreation"`）—— 先 `create` 再 `place`，两步各自成一个提案 |
//! | `placement` 里的键不在 [`PLACEMENT_FIELDS`] | `INVALID_PARAMETER_RANGE`（`reason = "unknownPlacementField"`） |
//! | 片段推不出长度（非 MIDI / 空 MIDI）且没给 `durationTicks` | `INVALID_PARAMETER_RANGE`（`reason = "durationNotDerivable"`）—— 不猜一个假长度 |
//!
//! 空 `ops` 只在**摆放在场**时被接受：那时"这次调用要做什么"由 `placement` 承载，
//! 音符那一半就是"一个音符都不动"（[`parse_ops`] 自己的空数组守卫**没有**放松）。
//!
//! ## 静态混音值形态（`ops[].kind == "setParam"`）—— 关闭"工具面写不了静态混音值"
//!
//! 模型早就有 [`Op::SetParam`]（目标 [`AutomationTarget::TrackVolume`] / [`AutomationTarget::TrackPan`]，
//! `read_param` / `write_param` 直接读写 `TrackV3::volume_db` / `TrackV3::pan`），
//! 而在这个 kind 之前，**17** 个工具里**没有**任何一个能写它们：`yeban_edit_automation`
//! 写的是 [`Op::SetAutomationPoint`]（自动化**点**），读侧能报 `staticValue` 却没有写侧；
//! `yeban_import_audio` 的 `gainDb` 是**片段**增益，不是音轨静态值。
//!
//! 为什么落在本工具的 `ops[]` 上（而不是新增工具、也不改 `yeban_edit_automation` 的实参）：
//!
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §7.2 给本工具的行是
//!   `ops: Vec<NoteOp>` —— `NoteOp` 的 JSON 形状**没有契约**
//!   （`schemas/mcp-tools.schema.json` 只把本工具列在 `name.enum` 里，
//!   实参由 `crate::tools::ToolSpec::input_schema` 逐条派生），
//!   因此**加一个 kind** 不改 `schemas/**` 一个字节；
//! - 本工具是**扩展工具**名单之外的那十个之一，而扩展工具（含 `yeban_edit_automation`）
//!   的实参集合被 `schemas/mcp-tools.schema.json` 的
//!   `definitions.ExtensionToolArguments.$defs` 逐字段钉住
//!   （判据 `tests/contract.rs::extension_argument_constraints_match_the_registry`）
//!   ⇒ 往它们身上加实参会**必须**同步 `schemas/**`（本线禁改）；
//! - `ADR-0001` **D46** 的扩张原则是"先扩既有工具的参数，只有确实不合适才新增工具"。
//!
//! 语义与模型**同源**，本层不另立第二份：
//!
//! - 目标名用 `yeban_edit_automation` 的同一份词汇表（[`LaneKind`]），只放行
//!   [`StaticLane::NAMES`] 这两个"有静态值"的目标；另外三个仍是**响亮失败**；
//! - `old_val`（撤销载荷）从**当前文档**读，走 [`AutomationTarget::static_value`]
//!   （与 `yeban_edit_automation` 的 `staticValue` 读数**同一个**入口），
//!   因此撤销是模型自己的 [`Op::invert`]，本层不写逆操作；
//! - 值域判定（声相 `-1.0..=1.0`、非有限值）**不在本层**：`Op::SetParam` 的
//!   `apply` 会调模型自己的 `validate_param_value`，本层只拦"不是数字"这类 JSON 形状错误。
//!
//! 落地路径与本工具既有的音符编辑**完全相同**：先提案、再 `yeban_merge_proposal`。
//!
//! ## 音轨开关形态（`ops[].kind == "setTrackMute"` / `"setTrackSolo"`）
//! —— 关闭"工具面写不了静音 / 独奏"这一半
//!
//! 上一票让 `setParam` 能写音轨的**静态**音量与声相，但同一排通道条上另外两个开关
//! 仍然够不着：模型有 [`Op::SetTrackMute`] / [`Op::SetTrackSolo`]（载荷是 `bool`，
//! 各自带自包含的 `old_mute` / `old_solo` 撤销载荷），母带渲染器**真的**读它们
//! （`super::render` 的 `audible` 判定），而这两个变体在整个 `crates/yeban-mcp` 里
//! **一次都没有被构造过** ⇒ 也就是"渲染器会静音"这件已实现的能力在 17 个工具的
//! 面上**不可达**（与 `setParam` 之前那一票同型的缺口）。
//!
//! 为什么**不能**把它们塞进 `setParam` 的两个目标：`AutomationTarget` 没有静音 /
//! 独奏变体，而 `SetParam` 的两个载荷都是 `f32`（`TrackV3::mute` / `solo` 是 `bool`）
//! ⇒ 布尔开关在 `SetParam` 里**不可表达**。因此本文件加两个 `kind`，名字是模型 `Op`
//! 变体名的小驼峰（与 [`SET_PARAM_KIND`] 同一条命名规则）。
//!
//! 两条刻意设成**响亮失败**的口径（绝不静默降级）：
//!
//! | 情形 | 结果 |
//! | :--- | :--- |
//! | `value` 不是 JSON 布尔（`1` / `"true"` / 缺字段） | `INVALID_PARAMETER_RANGE`（`reason = "valueMustBeBoolean"`）—— 不做真假值强转 |
//! | 开关对象里有 `kind` / `value` 之外的键（含嵌套 `trackId`） | `INVALID_PARAMETER_RANGE`（`reason = "unknownFlagField"`）—— 目标音轨是**顶层** `trackId`，嵌套写它只会被静默忽略 |
//!
//! `old_mute` / `old_solo` 从**当前文档**读（不是调用方的声明），因此模型的
//! `OpStateMismatch` 前置条件天然成立，撤销仍是模型自己的 [`Op::invert`]。
//!
//! ## 自动化泳道形态（`ops[].kind == "setAutomationLane"`）
//! —— 关闭"工具面写不了泳道属性"这一半
//!
//! 上两票让工具面能写音轨的静态音量 / 声相 / 静音 / 独奏，但**自动化泳道自己的属性**
//! 仍然够不着。模型有 [`Op::SetAutomationLane`] / [`Op::RemoveAutomationLane`]
//! （各自带 `old_lane` / `previous_lane` 自包含撤销载荷），`AutomationLane::read_enabled`
//! 与 `write_mode` **真的**是运行期开关：
//!
//! - `YebanProjectV1::automation_value_at`（**唯一**求值入口）在 `read_enabled == false`
//!   时返回 `Ok(None)` ⇒ 走带/宿主求值退回 [`AutomationTarget::static_value`]；
//! - `yeban-app` 的自动化界面按它画读开关与录制臂，`yeban-render` 的 Live 导出把它
//!   连同 `write_mode` 一起写进"未映射"账；
//! - `yeban_edit_automation` 的**读**侧早就把 `lane.readEnabled` / `lane.writeMode` /
//!   `lane.domain` 报给 Agent 了，而**写**侧只写一个采样点 ⇒ 报得出、改不了。
//!
//! 而在本形态之前，这两个变体在整个 `crates/yeban-mcp` 里**一次都没有被构造过**
//! ⇒ "读开关真的能关掉一条自动化"这件已实现的能力在 17 个工具的面上**不可达**，
//! 且一条被关掉的泳道在工具面上**永远回不到开**（与 `setParam` / `setTrackMute`
//! 之前那两票同型的缺口）。
//!
//! 为什么**不能**把它们塞进 `setParam`：`AutomationTarget` 里没有"泳道"这个对象，
//! 而 `SetParam` 的载荷是 `f32`（`read_enabled` 是 `bool`、`write_mode` 是四值枚举、
//! `domain` 是一对端点）⇒ 三者在 `SetParam` 里**不可表达**。
//!
//! 形态：`{"kind":"setAutomationLane","lane":{…}}`。`lane` 是一个**自包含**的对象，
//! 目标与载荷都在里面（`lane` 的寻址字段与 `yeban_edit_automation` 的同名实参
//! 逐字同词、同一份 [`LaneKind`] 词表 —— `ADR-0001` D48）：
//!
//! ```json
//! {"kind":"setAutomationLane","lane":{"lane":"TrackVolume","readEnabled":false}}
//! {"kind":"setAutomationLane","lane":{"lane":"DeviceParam","slotIndex":0,
//!   "paramIndex":1,"writeMode":"Touch","domain":{"min":0.0,"max":1.0}}}
//! {"kind":"setAutomationLane","lane":{"lane":"TrackPan","remove":true}}
//! ```
//!
//! **合并语义**（不是整体替换）：`readEnabled` / `writeMode` / `domain` 三个键
//! **各自可选**，缺省 = 保留泳道现值（没有泳道时 = 模型的隐式默认
//! `true` / `Off` / 无覆盖）⇒ 只给一个键就只改那一个。`domain` 明写 `null`
//! 是"清掉显式覆盖"（回到"派生自目标"），不是"不改"。
//!
//! 三条刻意设成**响亮失败**的口径（绝不静默降级）：
//!
//! | 情形 | 结果 |
//! | :--- | :--- |
//! | `lane` 对象里有 [`SET_AUTOMATION_LANE_FIELDS`] 之外的键 | `INVALID_PARAMETER_RANGE`（`reason = "unknownLaneField"`） |
//! | `remove: true` 与任何属性键同给 | `INVALID_PARAMETER_RANGE`（`reason = "removeTakesNoProperties"`）—— "取走"与"改成什么"是两件事 |
//! | 同一个目标在一次调用里出现两次 | `INVALID_PARAMETER_RANGE`（`reason = "duplicateLaneTarget"`）—— 批内第二条的 `old_lane` 会与文档现值不符 |
//!
//! `old_lane` / `previous_lane` **从当前文档读**（不是调用方的声明），因此模型的
//! 前置条件天然成立，撤销仍是模型自己的 [`Op::invert`]。模型自己的不变量原样生效：
//! `SetAutomationLane` 拒绝把泳道写成**隐式形状**（无点 + 读开 + `Off` + 无覆盖），
//! 因此"把一条空泳道的读开关打开"是模型的响亮失败，本层不替它决定。
//!
//! ## 自动化点取走形态（`ops[].kind == "removeAutomationPoint"`）
//! —— 关闭"工具面取不走一个自动化点"这一半
//!
//! 模型有 [`Op::RemoveAutomationPoint`]（载荷 `target` / `point_id` / `previous_point`
//! 全部自包含），而在这个形态之前，这个变体在整个 `crates/yeban-mcp/src` 里
//! **一次都没有被构造过**（实测：`git grep -c 'RemoveAutomationPoint' HEAD --
//! crates/yeban-mcp/src` 命中 **0** 个文件，`git log --all -S'RemoveAutomationPoint'
//! -- crates/yeban-mcp` 命中 **0** 次提交）。
//!
//! 缺口的样子与前几票同型：`yeban_edit_automation` 的**写**侧只能
//! [`Op::SetAutomationPoint`]（同一个 `(目标, tick)` 是**更新**），**读**侧却早就在报
//! `points[].id` ⇒ 工具面"报得出、改得动、**拿不走**"。`yeban_undo` 补不上它：
//! 撤销是操作日志的栈顶回退，**不能**只取走一个旧点而保住之后的编辑。
//!
//! 为什么落在本工具的 `ops[]` 上（而不是新增工具、也不改 `yeban_edit_automation` 的实参）：
//! 与 `setParam` / `setAutomationLane` 那两票**逐条同因** —— `NoteOp` 的 JSON 形状
//! 没有契约（§7.2 只写 `ops: Vec<NoteOp>`，`schemas/mcp-tools.schema.json` 把 `ops`
//! 声明为无约束数组），而扩展工具（含 `yeban_edit_automation`）的实参集合被
//! `definitions.ExtensionToolArguments.$defs` 逐字段钉住 ⇒ 往它们身上加实参会**必须**
//! 同步 `schemas/**`（本线禁改）；`ADR-0001` **D46** 的扩张原则是"先扩既有工具的
//! 参数，只有确实不合适才新增工具"。
//!
//! 形态（`point` 是一个**自包含**对象：泳道寻址 + 点寻址）：
//!
//! ```json
//! {"kind":"removeAutomationPoint","point":{"lane":"TrackVolume","tick":3840}}
//! {"kind":"removeAutomationPoint","point":{"lane":"Macro","macroIndex":0,
//!   "pointId":"01J8Z0000000000000000000AB"}}
//! ```
//!
//! `tick` 与 `pointId` **恰好给一个**。为什么**不**照抄写侧的"两个都给、身份赢"：
//! 写侧的 `tick` 是**载荷**（点落在哪一拍），而这里是**寻址**（取走哪一个点）；
//! 两个寻址同给会让"删的是哪个点"取决于一条隐含优先级。恰好二选一是本仓既有的口径
//! （`yeban_import_audio` 的 `assetHash` / `path` 同款）。
//!
//! 两条寻址的语义**都在文档上**判定，本层不发明第二套身份规则：
//!
//! - `pointId` = 模型自己的点身份（`yeban_edit_automation` 读侧 `lane.points[].id`
//!   报的就是它）；不存在 ⇒ `ENTITY_NOT_FOUND`；
//! - `tick` = "文档里**恰好一个**落在该 tick 上的点"：0 个 ⇒ `ENTITY_NOT_FOUND`
//!   （`automationPointNotFound`），≥2 个 ⇒ **响亮拒绝**
//!   （`INVALID_PARAMETER_RANGE`，`reason = "automationTickAmbiguous"`，
//!   `data.candidatePointIds` 列出全部候选）。模型只保证 `point.id` 唯一
//!   （`AutomationLane::validate` 查键/身份一致与取值有限），**不保证**同一泳道内
//!   tick 唯一 —— 显式 `pointId` 的写入能在同一 tick 上留下两个点，因此"取第一个"
//!   会删掉调用方**没指名**的那个点，而撤销载荷只记被删的那个。
//!
//! `previous_point` **从当前文档读**（不是调用方的声明），因此模型的
//! 前置条件天然成立，撤销仍是模型自己的 [`Op::invert`]。
//!
//! 五条刻意设成**响亮失败**的口径（绝不静默降级）：
//!
//! | 情形 | 结果 |
//! | :--- | :--- |
//! | `tick` 与 `pointId` 同给 | `INVALID_PARAMETER_RANGE`（`reason = "pointAddressIsAmbiguous"`） |
//! | 两个都不给 | `INVALID_PARAMETER_RANGE`（`reason = "pointAddressRequired"`） |
//! | 操作对象里有 `kind` / `point` 之外的键（顶层 `tick` 是最像"写对了"的错法） | `INVALID_PARAMETER_RANGE`（`reason = "unknownPointRemovalField"`） |
//! | `point` 对象里有 `readEnabled` / `writeMode` / `domain` / `remove`（**别处合法**、此形态不适用） | `INVALID_PARAMETER_RANGE`（`reason = "lanePropertiesNotApplicableToPointRemoval"`） |
//! | 同一个 tick 上有多个点（`tick` 寻址） | `INVALID_PARAMETER_RANGE`（`reason = "automationTickAmbiguous"`） |
//!
//! 泳道不存在、或泳道上没有那个身份 / 那个 tick 的点 ⇒ `ENTITY_NOT_FOUND`
//! （`reason` 分别是 `automationLaneNotFound` / `automationPointNotFound`），
//! **不**静默成功。
//!
//! ⚠ 词表登记（`ADR-0001` D48）：本形态的 `point` 键与 `yeban_edit_automation` 的
//! `point` **实参**同名，但形状不同 —— 后者是"要写入的点的**载荷**"
//! （`{tick, value, curve?}`），本形态是"要取走的点的**寻址**"
//! （`{lane, …, tick | pointId}`）。同名的理由是两者都指"那一个自动化点"，
//! 且本对象**自带**泳道寻址（工具顶层只有一个 `trackId`，音轨之外的分量无处可放）。
//!
//! ## 片段池取走形态（`ops[].kind == "removeClip"`）
//! —— 关闭"工具面取不走一个片段池条目"这一半
//!
//! 模型有 [`Op::RemoveClip`]（载荷 `clip_id` / `previous_clip` 自包含），而在这个形态
//! 之前，这个变体在整个 `crates/yeban-mcp/src` 里**一次都没有被构造过**（实测：
//! `git grep -c 'RemoveClip {' HEAD -- crates/yeban-mcp/src` 命中 **0** 个文件）。
//!
//! 缺口与材料创建（`create: true`）那一票**互为镜像**：本工具能用 `create: true`
//! 把一条 MIDI 材料放进 `clip_pool`（[`Op::AddClip`]），`placement` 能把已有材料摆上
//! 时间轴、也能把摆放取走（[`Op::RemoveClipPlacement`]），而池子里的条目**没有任何
//! 工具**能取走 —— `yeban_query_project` 的实体索引早就在报每一条
//! `{"kind":"clip","id":…}`，于是工具面"看得到、建得出、**取不走**"。`yeban_undo`
//! 补不上它：撤销是操作日志的栈顶回退，不能只取走一条旧材料而保住之后的编辑。
//!
//! 为什么落在本工具的 `ops[]` 上（而不是新增工具、也不改别的扩展工具的实参）：
//! 与 `setParam` / `setAutomationLane` / `removeAutomationPoint` 那三票**逐条同因** ——
//! `NoteOp` 的 JSON 形状没有契约（架构 §7.2 只写 `ops: Vec<NoteOp>`，
//! `schemas/mcp-tools.schema.json` 把 `ops` 声明为无约束数组），而扩展工具的实参集合被
//! `definitions.ExtensionToolArguments.$defs` 逐字段钉住 ⇒ 往它们身上加实参会**必须**
//! 同步 `schemas/**`（本线禁改）；`ADR-0001` **D46** 的扩张原则是"先扩既有工具的参数，
//! 只有确实不合适才新增工具"。
//!
//! 形态：`{"kind":"removeClip"}` —— **空载荷**。目标片段就是工具**顶层**的 `clipId`
//! （那正是本工具必填的"目标片段"），因此这个操作对象只认 `kind` 一个键：
//! 多写任何键（尤其嵌套的 `clipId`）都是**响亮失败**，不静默丢弃。
//!
//! 五条刻意设成**响亮失败**或**如实上报**的口径（绝不静默降级）：
//!
//! | 情形 | 结果 |
//! | :--- | :--- |
//! | 操作对象里有 `kind` 之外的键 | `INVALID_PARAMETER_RANGE`（`reason = "unknownRemoveClipField"`） |
//! | 与 `ops` 里的其它任何形态同给 | `INVALID_PARAMETER_RANGE`（`reason = "removeClipTakesNoOtherOps"`）—— 不能在同一次调用里既编辑一条片段又把它取走 |
//! | 与 `placement` 同给 | `INVALID_PARAMETER_RANGE`（`reason = "removeClipIsNotPlacement"`） |
//! | 池子里没有这条片段 | `CLIP_NOT_FOUND` |
//! | 还有**任何**摆放引用它 | `CONFLICT`（模型自己的 `ClipInUse`，带 `placement_count`）—— 先取走摆放，再取走材料 |
//!
//! `previous_clip` **从当前文档读**（不是调用方的声明），因此模型 `RemoveClip` 的
//! 前置条件天然成立，撤销仍是模型自己的 [`Op::invert`]。
//!
//! 这个形态**不要求片段是 MIDI**：它一个音符都不读，因此 `yeban_import_audio`
//! 放进池子的**音频**片段同样取得走（那是本形态唯一的池回收通路）。
//! ⚠ 如实登记的边界：取走池条目**不回收 CAS 资产字节** —— 模型 `Op` 全集里没有任何
//! 资产变体（见 `docs/ledger/mcp-tools-expansion-notes.md` §6 的 needs-2），
//! 因此"孤儿字节"仍然要等模型侧一个资产声明/回收 `Op`。
//!
//! ## 路由边增益形态（`ops[].kind == "setRoutingGain"`）
//! —— 关闭"工具面写不了发送增益"这一半
//!
//! 模型有 [`Op::SetRoutingGain`]（载荷 `old_gain_db` / `new_gain_db` 都是
//! `Option<f32>`：`None` = **单位增益**），而在这个形态之前，这个变体在整个
//! `crates/yeban-mcp/src` 里**一次都没有被构造过**。实测（可复跑）：
//! `git grep -n 'Op::SetRoutingGain' origin/main -- crates/yeban-mcp/src` 只命中
//! **2 行**，而且两行**都是文字** —— 一条在 [`StaticLane`] 的文档里，一条在
//! `setParam` 拒绝 `SendGain` 的错误消息里，两条都在说"必须走
//! `Op::SetRoutingGain`"；把口径收紧到**构造点**
//! （`git grep -hoE 'Op::SetRoutingGain \{' origin/main -- crates/yeban-mcp/src | wc -l`）
//! 读数是 **0**。工具面把这条通路**指了出来**，却没有把它接上。
//!
//! 缺口的形状是"**读得出、写不了**"，与 `setParam` 那一票**互为镜像**：
//!
//! | 事实 | 依据 |
//! | :--- | :--- |
//! | 读侧报得出静态值 | `yeban_edit_automation` 的 `lane.staticValue` 走 `AutomationTarget::static_value`，`SendGain` 那一支返回 `edge.gain_db.unwrap_or(0.0)` |
//! | 渲染器真的消费它 | 路由边的 `gain_db` 在母带混音路径上（不是装饰字段） |
//! | 模型指定唯一写者 | `read_param` / `write_param` **明文拒绝** `SendGain`（`AutomationTargetNotApplicable`），理由是 `SetParam` 的载荷是裸 `f32`、`None` 与 `Some(0.0)` 会不可区分 |
//! | 工具面却没有写者 | 17 个工具里**没有**任何一个构造过 `Op::SetRoutingGain` |
//!
//! 于是 AI Agent 读得到发送增益、听得见它、却**改不动**它 —— 与
//! "音轨静态音量 / 声相只能读不能写"（`setParam` 那一票关掉的）同一族。
//!
//! 为什么是**新形态**而不是给 `setParam` 的 `lane` 再加一个目标名：
//! `setParam` 编译成 [`Op::SetParam`]，而模型**明文拒绝**它写 `SendGain`。
//! 把 `SendGain` 塞进那个形态等于在工具面说一套、在模型层做另一套
//! （[`StaticLane`] 用二值枚举把"哪三个不可写"做成**不可表达**，正是为了这件事）。
//! 本形态的名字与 [`Op::SetRoutingGain`] 同词（变体名的小驼峰），与其余 kind 同一规则。
//!
//! 形态：`{"kind":"setRoutingGain","edgeId":"<ULID>","value":<数字 | null>}` ——
//! `null` 就是模型的 `None`（单位增益），**不是** `0.0`（`Some(0.0)` 是另一件事）。
//!
//! 五条刻意设成**响亮失败**或**从文档读**的口径（绝不静默降级）：
//!
//! | 情形 | 结果 |
//! | :--- | :--- |
//! | 操作对象里有 `kind` / `edgeId` / `value` 之外的键 | `INVALID_PARAMETER_RANGE`（`reason = "unknownRoutingGainField"`） |
//! | `value` 既不是数字也不是 `null`（缺字段 / 布尔 / 字符串 / 对象） | `INVALID_PARAMETER_RANGE` |
//! | `value` 收窄到 `f32` 后不是有限数 | `INVALID_PARAMETER_RANGE`（`reason = "nonFiniteValue"`，与 `setParam` 同一口径：**先收窄再判**） |
//! | 文档里没有这条路由边 | `ENTITY_NOT_FOUND`（`reason = "routingEdgeNotFound"`） |
//! | `old_gain_db` | **从当前文档读**，不是调用方声明；且读 `RoutingGraph::edge` 的**原样** `Option<f32>` |
//!
//! ⚠ 为什么撤销载荷**不能**走 `AutomationTarget::static_value`（`setParam` 走的那个
//! 唯一静态值入口）：那个入口把 `None` **折算**成 `0.0`，而模型的 `same_gain` 是
//! **逐位**比较 —— `None` 与 `Some(0.0)` 不是同一个值。用 `static_value` 读出的
//! `0.0` 会让 [`Op::SetRoutingGain`] 的前置条件在"文档里是单位增益"的边上失败
//! （`OpStateMismatch` ⇒ `CONFLICT`），也就是**改不了**最常见的那一类边。
//!
//! 目标**不在**顶层 `trackId` 上：本形态自带寻址（`edgeId`），与
//! `setAutomationLane` 的 `lane` / `removeAutomationPoint` 的 `point` 同一纪律。
//! `SendGain` 的自动化**点**仍走 `yeban_edit_automation` 与 [`NoteOp::SetLane`]
//! （那是泳道的事）；本形态只管**静态**增益这一个字段。
//!
//! ## 路由节点取走形态（`ops[].kind == "removeRoutingNode"`）
//! —— 关闭"工具面造得出的节点取不走"这条缺口
//!
//! 模型有 [`Op::RemoveRoutingNode`]（载荷只有 `node` 一个身份，**没有撤销载荷** ——
//! 它与 `AddRoutingNode` 互为逆操作），而在这个形态之前，这个变体在整个
//! `crates/yeban-mcp/src` 里**一次都没有被构造过**。实测（可复跑，单位 = "匹配到的构造点个数"）：
//! `git grep -hoE '(^|[^A-Za-z0-9_])Op::RemoveRoutingNode \{' origin/main -- crates/yeban-mcp/src | wc -l`
//! 读数是 **0**（同一个模式对 `Op::SetRoutingGain` 读数是 **5** ⇒ 模式本身**有效**，
//! 0 不是"模式写坏了"）；`git grep -c 'Op::RemoveRoutingNode' origin/main -- crates/yeban-mcp/src`
//! 只命中 **2 行**，而且两行**都是文字** —— 一条是 [`NoteOp::DisconnectRouting`] 的文档，
//! 一条是 `yeban_edit_notes` 的工具说明，两条都在说"取走节点是 `Op::RemoveRoutingNode` 的事"。
//! 工具说明甚至**许了愿**：原文说"先断开每一条引用边, 再单独一次调用取走节点"，
//! 而那次调用**不存在**。
//!
//! 缺口的形状与 `setRoutingGain` / `disconnectRouting` 两票**逐条同因**：
//!
//! | 事实 | 依据 |
//! | :--- | :--- |
//! | 读侧报得出节点身份 | `yeban_query_project` 的 `routing_graph.nodes` 数组 |
//! | 渲染器真的消费它 | `yeban-engine` 的 `PdcPlan::compute` 以"主总线在 `nodes` 里"为前置条件（否则 `UnknownMaster`，一个块都不渲染） |
//! | 模型指定唯一写者 | 17 个工具里**没有**任何一个构造过 `Op::RemoveRoutingNode` |
//!
//! 于是 AI Agent 断得开边、看得见节点、却**取不走**它：一次声部连接回滚到最后会
//! 留下一个只能在读侧看见的孤节点。
//!
//! 为什么是**新形态**而不是给 [`DISCONNECT_ROUTING_KIND`] 加开关：取走节点是**另一个**
//! 模型变体，`Op::DisconnectRouting` 的前置条件（`previous_edge` 逐字段等于文档现值）
//! 与它无关；把它并进那个形态就是让那个形态的名字说谎
//! （与 [`SET_ROUTING_GAIN_KIND`] 单列的理由相同）。
//!
//! 形态：`{"kind":"removeRoutingNode","nodeId":"<ULID>"}` —— 载荷是**空**的（只有寻址）。
//!
//! 五条刻意设成**响亮失败**的口径（绝不静默降级）：
//!
//! | 情形 | 结果 |
//! | :--- | :--- |
//! | 操作对象里有 `kind` / `nodeId` 之外的键 | `INVALID_PARAMETER_RANGE`（`reason = "unknownRemoveRoutingNodeField"`） |
//! | `nodeId` 缺失 / 不是字符串 / 不是合法 ULID | `INVALID_PARAMETER_RANGE`（缺字段走统一的缺字段错误） |
//! | `routing_graph.nodes` 里没有这个身份 | `ENTITY_NOT_FOUND`（`reason = "routingNodeNotFound"`） |
//! | 这个身份是**主总线** | `CONFLICT`（`reason = "masterBusNodeCannotBeRemoved"`） |
//! | 还有**任何**路由边引用它 | `CONFLICT`（模型自己的 `RoutingNodeInUse`，带 `edge_count`） |
//!
//! ⚠ 为什么"主总线节点"是一条**本层**的提前拒绝：模型把它放在
//! `YebanProjectV1::validate` 里（有音轨时主总线必须在 `nodes` 里），那条校验只在
//! **提案模拟**那一步跑；它复用的错误变体是 `RoutingNodeNotFound`（模型侧注释写明了
//! 理由：不扩 `ModelError`），于是调用方会收到"节点不存在" —— 而那个节点**刚刚被
//! 自己取走**。同一个结论（响亮失败）在这里用一个说得通的契约码表达。
//!
//! ⚠ 本层**不**复制"没有任何边引用它"那条前置条件：那是 `Op::validate` 的事，
//! 它的载荷（`edge_count`）比本层能给的更具体 —— 与 `disconnectRouting` 让
//! `RoutingNodeInUse` 由模型报出同一条纪律。
//!
//! 目标**不在**顶层 `trackId` / `clipId` 上：本形态自带寻址（`nodeId`），与
//! `setRoutingGain` / `disconnectRouting` 同一纪律。
//!
//! ## 段落取走形态（`ops[].kind == "removeSection"`）
//! —— 关闭"工具面造得出的段落取不走"这条缺口
//!
//! 模型有 [`Op::RemoveSection`]（载荷 `section_id` / `previous_section`），而在这个形态
//! 之前，这个变体在整个 `crates/yeban-mcp/src` 里**一次都没有被构造过**。实测
//! （可复跑，单位 = "匹配到的构造点个数"）：
//! `git grep -hoE '(^|[^A-Za-z0-9_])Op::RemoveSection \{' origin/main -- crates/yeban-mcp/src | wc -l`
//! 读数是 **0**（同一个模式对 `Op::SetSection` 读数是 **2** ⇒ 模式本身**有效**，
//! 0 不是"模式写坏了"）；`git grep -c 'Op::RemoveSection' origin/main -- crates/yeban-mcp/src`
//! 命中 **0 行** —— 连一句文字都没有。
//!
//! 缺口形状与 `removeClip` / `removeRoutingNode` 两票**逐条同因**：
//!
//! | 事实 | 依据 |
//! | :--- | :--- |
//! | 读侧报得出段落身份 | `yeban_query_project` 的 `entities[]` 里 `kind == "section"` 的条目 |
//! | 写侧已经造得出段落 | `yeban_propose_section` 的建批写出 [`Op::SetSection`]（配器骨架） |
//! | 模型指定唯一写者 | 17 个工具里**没有**任何一个构造过 [`Op::RemoveSection`] |
//!
//! 于是 AI Agent 建得出一个曲式段落、看得见它的身份、却**取不走**它。
//! `RemoveSection` 这个变体存在的理由本身就是**不可逆性**（`docs/adr/ADR-0001` 的
//! D12）：`SetSection { old_section: None }` 表示"新建"，它的逆操作必须是删除，
//! 而 `Op` 全集里**没有**第二个变体能表达删除 ⇒ 它是"新建段落"这一步的**唯一**逆操作。
//! 工具面能走前半步、不能走后半步，正是本会话连修的那一类缺口。
//!
//! 为什么落在 [`compile`] 所在的 `yeban_edit_notes` 而不给 `yeban_propose_section`
//! 加开关：后者的语义是"在隔离分支上**建**一个配器骨架"，把"取走一个**已有**段落"
//! 塞进去就是让那个工具的名字说谎；`ADR-0001` D46 的扩张原则是"先扩既有工具的参数，
//! 只有确实不合适才新增工具"，而新增工具要同步 `schemas/mcp-tools.schema.json` 的
//! `name.enum` / `$defs` / `allOf` 三处（本线禁改 `schemas/**`）。`yeban_edit_notes`
//! 已经是本工具面的操作日志编辑器（`removeClip` / `setRoutingGain` /
//! `disconnectRouting` / `removeRoutingNode` 都住在它里面），段落取走因此落在同一处。
//!
//! 形态：`{"kind":"removeSection","sectionId":"<ULID>"}` —— 载荷是**空**的（只有寻址）；
//! 撤销载荷 `previous_section` 由 [`compile`] 从**当前文档**读（模型的前置条件要求它
//! 逐字段等于文档现值，因此本层不采信调用方声明的旧状态）。
//!
//! 三条刻意设成**响亮失败**的口径（绝不静默降级）：
//!
//! | 情形 | 结果 |
//! | :--- | :--- |
//! | 操作对象里有 `kind` / `sectionId` 之外的键 | `INVALID_PARAMETER_RANGE`（`reason = "unknownRemoveSectionField"`） |
//! | `sectionId` 缺失 / 不是字符串 / 不是合法 ULID | `INVALID_PARAMETER_RANGE`（缺字段走统一的缺字段错误） |
//! | `project.sections` 里没有这个身份 | `ENTITY_NOT_FOUND`（`reason = "sectionNotFound"`） |
//!
//! ⚠ 本层**不**把"段落存在"这条前置条件交给模型去报：`Op::validate` 报的是
//! `SectionNotFound`（契约码相同），但那条路径只在提案模拟那一步跑，消息里没有本层的
//! `reason` / `hint`；"现值等于 `previous_section`"那一条**不**复制（撤销载荷本来就是
//! 本层从文档读的，因此它自动成立 —— 与 `disconnectRouting` 同一条纪律）。
//!
//! 目标**不在**顶层 `trackId` / `clipId` 上：本形态自带寻址（`sectionId`），与三个
//! 路由级形态同一纪律。⚠ 但工具签名的 `trackId` / `clipId` 依旧是**必填**的
//! （`compile` 的入口先查音轨与片段池 —— 这是本工具既有的口径，三个路由级形态处在
//! 同一处境）；段落形态一个音符都不读，因此**不要求**片段是 MIDI。
//!
//! ## 场景取走形态（`ops[].kind == "removeScene"`）
//! —— 关闭"读侧报得出场景、工具面一个字都写不了"这条缺口
//!
//! 模型有 [`Op::RemoveScene`]（载荷 `scene_id` / `previous_scene`，语义由
//! `SetScene { old_scene: None }` 的逆定义 —— `docs/adr/ADR-0001` 的 D12 明文写着
//! "无自由度"），而在这个形态之前，`Op::SetScene` 与 `Op::RemoveScene` 在
//! `crates/yeban-mcp/src` 里的构造点**都是 0**。实测（可复跑）：
//! 作用域 = `crates/yeban-mcp/src`，ref = `origin/main`，单位 = "匹配到的个数"，
//! 两个口径 = ① `git grep -hoE '(^|[^A-Za-z0-9_])Op::<V>[[:space:]]*\{'`（构造点）
//! 与 ② `git grep -hoE '(^|[^A-Za-z0-9_])Op::<V>([^A-Za-z0-9_]|$)'`（任何提及）：
//!
//! | 变体 | ① 构造点 | ② 任何提及 | 对照读数（同一模式 / 同一作用域 / 同一 ref） |
//! | :--- | ---: | ---: | :--- |
//! | `Op::SetScene` | **0** | **0** | `Op::SetSection` 2 / 10 |
//! | `Op::RemoveScene` | **0** | **0** | `Op::SetMacro` 2 / 3 |
//! | `Op::RemoveTrack` | **0** | **0** | `Op::AddTrack` 3 / 5 |
//! | `Op::InsertDevice` | **0** | **0** | `Op::SetParam` 2 / 17 |
//! | `Op::RemoveDevice` | **0** | **0** | `Op::SetRoutingGain` 5 / 21 |
//!
//! 读法：同一份扫描模式在同一次运行里对其它成员读出**非零**（右列），因此左列的 0
//! 是"那里真的没有"，不是"模式把一切都扫成 0"。本形态落地之后，同一模式在**工作树**上
//! 对 `Op::RemoveScene` 读出 ① **3** / ② **20**（`compile` 的构造点 + 本节的变体定义
//! 与两条判据里的同名模式）—— 这正是"新写下的构造点会被这个模式看见"的正向对照。
//! ⚠ 诚实边界：本节的行文本身也会提到那几个仍然为 0 的变体名，因此在**工作树**上它们的
//! ② 不再是 0（全部来自本节）；上表的 0 一律是 **`origin/main`** 上的读数。
//! （`Op::SetScene` 那一行在**本票**之后也离开了这张表 —— 见下一节。）
//! 缺口的形状是：
//!
//! | 事实 | 依据 |
//! | :--- | :--- |
//! | 读侧报得出场景身份 | `yeban_query_project` 的 `entities[]` 里 `kind == "scene"` 的条目（`domain/view.rs` 一直在推它） |
//! | 写侧**没有**任何形态 | 17 个工具里没有任何一个构造过 `Op::SetScene` 或 `Op::RemoveScene` |
//! | 模型侧早已实现 | `Op::validate` 报 `SceneNotFound`、`Op::apply` 真的 `scenes.remove`、`Op::invert` 把它换成 `SetScene`；`domain/error.rs` 的 `code_for_model` 也早已把 `SceneNotFound` 映到 `ENTITY_NOT_FOUND` |
//!
//! 于是 AI Agent 打开一份带场景的工程、看得见每个场景的身份，却**一个都动不了**。
//! 本形态关的是其中**无自由度**的那一半：删除（D12：逆操作由 `SetScene` 定义）。
//! ⚠ 本票**不**做创建/更新那一半（`Op::SetScene`）：它要决定"`sceneId` 是可选还是
//! 必填""`name` 是否必填""`tempo` / `color` 是合并还是整体替换"，这些都是**有**自由度
//! 的设计选择，该由自己的一票（连同它的判据）来定 —— 登记在此，不冒充已完成。
//!
//! 形态：`{"kind":"removeScene","sceneId":"<ULID>"}` —— 载荷是**空**的（只有寻址）；
//! 撤销载荷 `previous_scene` 由 [`compile`] 从**当前文档**读。
//!
//! 三条刻意设成**响亮失败**的口径（绝不静默降级）：
//!
//! | 情形 | 结果 |
//! | :--- | :--- |
//! | 操作对象里有 `kind` / `sceneId` 之外的键 | `INVALID_PARAMETER_RANGE`（`reason = "unknownRemoveSceneField"`） |
//! | `sceneId` 缺失 / 不是字符串 / 不是合法 ULID | `INVALID_PARAMETER_RANGE`（缺字段走统一的缺字段错误） |
//! | `project.scenes` 里没有这个身份 | `ENTITY_NOT_FOUND`（`reason = "sceneNotFound"`） |
//!
//! ⚠ 本层**不**把"场景存在"这条前置条件交给模型去报：理由与段落形态逐条相同
//! （`Op::validate` 的 `SceneNotFound` 只在提案模拟那一步跑，消息里没有本层的
//! `reason` / `hint`）。⚠ 也**不**查"还有谁引用这个场景"：`SceneV3` 不与别的实体
//! 交叉引用（`git grep -nE '(scene_id|sceneId)' -- 'crates/**/*.rs'` 的命中只落在
//! `yeban-model` 的 `ops.rs`、`yeban-render` 的 `als.rs` 的本地导出变量与本节所在
//! 文件里），模型因此也没有给 `RemoveScene` 任何"仍被引用"型前置条件。
//!
//! 目标**不在**顶层 `trackId` / `clipId` 上：本形态自带寻址（`sceneId`），与段落级、
//! 三个路由级形态同一纪律；场景形态一个音符都不读，因此**不要求**片段是 MIDI。
//!
//! ## 场景写入形态（`ops[].kind == "setScene"`）
//! —— 关闭"读侧报得出场景、写侧只能删不能建"这条缺口
//!
//! 上一节把场景实体上**无自由度**的那一半（取走）接上了工具面，并明文登记"创建/更新那一半
//! 要决定 `sceneId` / `name` / `tempo` / `color` 的形状，留在它自己的一票"。本票就是那一票：
//! 模型 [`Op::SetScene`] 的构造点在 `crates/yeban-mcp/src` 里此前是 **0**（上一节的表在它自己
//! 标注的 ref `8474e4b^` 上读 `Op::SetScene` ① **0** / ② **0**，同表 `Op::SetSection` 读
//! 2 / 10、`Op::SetMacro` 读 2 / 3 —— 模式有效）。本形态把它接上。
//!
//! 落地之后，同一份 git-grep 模式在**本票提交的那个工作树**上的读数（单位 = "匹配到的个数"，
//! 作用域 = `crates/yeban-mcp/src`，与上一节逐字同一对模式；复跑同一对命令在这份文件上
//! 必须读出同一组数）：
//!
//! | 变体 | ① 构造点 | ② 任何提及 | 对照读数（同一模式 / 同一作用域 / 同一工作树） |
//! | :--- | ---: | ---: | :--- |
//! | `Op::SetScene` | **4** | **29** | `Op::SetSection` 2 / 17 |
//! | `Op::RemoveScene` | 3 | 23 | `Op::SetMacro` 2 / 6 |
//! | `Op::RemoveTrack` | 0 | 2 | `Op::SetSection` 2 / 17 |
//! | `Op::InsertDevice` | 0 | 2 | `Op::SetSection` 2 / 17 |
//! | `Op::RemoveDevice` | 0 | 2 | `Op::SetSection` 2 / 17 |
//!
//! 读法：正向对照（`Op::SetScene` 由 0 变成非零）证明"新写下的构造点会被这个模式看见"；
//! 右列证明模式在同一次运行里对别的成员读出非零，因此**仍然为 0 的那三个**
//! （`RemoveTrack` / `InsertDevice` / `RemoveDevice`）是"那里真的没有"，不是"模式把一切都
//! 扫成 0"。⚠ 诚实边界（两条）：① 那三个的 ② 不是 0（各 2 次），因为上一节的表与本节的行文
//! 都提到了它们的名字 —— ② 的 0 只在**未提到**它们的 ref 上成立；② ②列的读数**把本表自己的
//! 行文也算在内**（每一行都写了变体名），因此 ② 是"这份文件此刻的命中数"，不是"功能代码里的
//! 引用数"。① 列不受行文影响（模式要求变体名后跟 `{`），因此它是**只由构造点决定**的读数。
//!
//! 本票对上一节登记的**三个自由度**逐条裁决（每条都给理由，不冒充规范）：
//!
//! | 自由度 | 本票的决定 | 理由 |
//! | :--- | :--- | :--- |
//! | `sceneId` 可选还是必填 | **必填**（新建与更新都要） | 本工具面的身份一律由调用方给出（`create: true` 的 `clipId` 是 §7.2 的必填实参；`removeScene` 的 `sceneId` 同样必填）。`ops[]` 信封**没有**回传"服务端替你生成的身份"的通道，确定性派生又会让两个同名场景撞成一个 ⇒ 派生不是可选项 |
//! | `name` 是否必填 | **新建时必填**；更新时可省（合并） | `SceneV3::name` 是普通 `String`，模型对它**没有任何校验**（`project.rs` 的 `SceneV3::validate` 只查 `tempo`）⇒ 空名会一路静默落盘。`yeban_import_audio` 建材料时同样要求 `name` |
//! | `tempo` / `color` 合并还是整体替换 | **合并**（缺省 = 保留文档现值；显式 `null` = 清空） | 与本工具里 [`LanePatch`] 的口径逐条相同（`{"kind":"setAutomationLane"}` 的 `domain` 也是"`null` 清掉"）。三态**不折叠**：`None`（没提）与 `Some(None)`（明写 `null`）不是同一件事 |
//!
//! 新建与更新由载荷里的 `create` 布尔**显式**区分（缺省 `false`），不靠"文档里没有这个身份
//! 就当作新建"：后者会让一个打错的 `sceneId` **静默建出一个新场景**，而那正是本仓库
//! "响亮失败、绝不静默降级"纪律要拦的形状。`create` 与工具顶层的 `create` 同词同义
//! （`ADR-0001` D48）—— "目标不存在才新建，已存在就响亮拒绝"；两者不可能同时为真
//! （顶层 `create: true` 只放行 `add`）。
//!
//! 形态：`{"kind":"setScene","scene":{...}}` —— 载荷是**自包含**的 `scene` 对象
//! （与 `setAutomationLane` 的 `lane`、`removeAutomationPoint` 的 `point` 同一形状）：
//!
//! ```json
//! {"kind":"setScene","scene":{"create":true,"sceneId":"<ULID>","name":"Verse",
//!                             "tempo":128.0,"color":"#22AA88"}}
//! {"kind":"setScene","scene":{"sceneId":"<ULID>","name":"Verse 2","tempo":null}}
//! ```
//!
//! 撤销载荷 `old_scene` 由 [`compile`] 从**当前文档**读（新建时是 `None`，更新时是整条现值
//! 的克隆）：模型的前置条件要求它逐字等于文档现值，因此本层不采信调用方声明的旧状态。
//! `SetScene` 的逆操作是 [`Op::invert`] 定义的（更新 → 反向 `SetScene`；新建 →
//! `RemoveScene`），因此**新建一次 + 合并**之后，一次 `yeban_undo` 就能把这个场景整条取走。
//!
//! 六条刻意设成**响亮失败**的口径（绝不静默降级）：
//!
//! | 情形 | 结果 |
//! | :--- | :--- |
//! | 操作对象里有 `kind` / `scene` 之外的键 | `INVALID_PARAMETER_RANGE`（`reason = "unknownSetSceneOpField"`） |
//! | `scene` 对象里有 [`SET_SCENE_FIELDS`] 之外的键 | `INVALID_PARAMETER_RANGE`（`reason = "unknownSetSceneField"`） |
//! | `sceneId` 缺失 / 不是字符串 / 不是合法 ULID | `INVALID_PARAMETER_RANGE`（缺字段走统一的缺字段错误） |
//! | `create: true` 且 `project.scenes` 里**已有**该身份 | `CONFLICT`（`reason = "sceneAlreadyExists"`，与 `create: true` 的 `clipAlreadyExists` 同口径） |
//! | `create` 非真（缺省）且 `project.scenes` 里**没有**该身份 | `ENTITY_NOT_FOUND`（`reason = "sceneNotFound"`，与 `removeScene` 同一个词同一个意思） |
//! | `name` 出现但是空串 / `color` 出现但是空串 / `name` 或 `color` 给了 `null` | `INVALID_PARAMETER_RANGE`（`reason = "sceneNameMustBeNonEmptyString"` / `"sceneColorMustBeNonEmptyString"` / `"sceneNameMustBeString"`） |
//!
//! `tempo` 的**值域与有限性不在本层判**（与 `setParam` 的"值域由模型判"同一纪律）：
//! `SceneV3::validate` 报的 `NonFiniteValue` / `BpmOutOfRange` 经由
//! `domain/error.rs` 的 `code_for_model` 落到 `INVALID_PARAMETER_RANGE` / `OUT_OF_RANGE`，
//! 那两条在**提案模拟**那一步就会跑（`propose_draft` 的整批 `apply`），因此越界的 `tempo`
//! 连提案都建不出来。⚠ 同一场景在**一次调用**里被写两次由
//! [`reject_duplicate_scene_targets`] 在建提案之前响亮拒绝（批内每条的撤销载荷都从调用前的
//! 文档读，第二条必然对不上状态 —— 与 [`reject_duplicate_lane_targets`] 同因）。
//!
//! 目标**不在**顶层 `trackId` / `clipId` 上：本形态自带寻址（`sceneId`），与场景取走形态
//! 同一纪律；它一个音符都不读，因此**不要求**片段是 MIDI，也不碰音轨、片段池、路由图
//! 与曲式段落。

use std::collections::BTreeMap;
use std::str::FromStr as _;

use serde_json::{Map, Value};

use yeban_model::music::{MICRO_TIMING_MAX_ABS, RATCHET_MAX, RATCHET_MIN};
use yeban_model::{
    AutomationLane, AutomationTarget, AutomationValueDomain, AutomationWriteMode, ClipContent,
    ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, Op, SceneV3, TrackV3,
    YebanProjectV1,
};

use super::error::{Fault, from_model};
use super::extension_pure::LaneKind;
use super::ids::deterministic_id;
use crate::tools::ErrorCode;

/// `create: true` 且没给 `clipName` 时的片段名（**不是**身份，只是给人看的标签）。
pub const DEFAULT_NEW_CLIP_NAME: &str = "Clip";

/// 材料创建形态的开关实参名（`arguments.create`，缺省 `false` = 旧行为）。
///
/// 名字与 `yeban_open_project` 的 `create` **同词同义**（`ADR-0001` D48 的口径：
/// 同一个词必须同一个意思）—— "目标不存在才新建，已存在就响亮拒绝"。
pub const CREATE_PARAM: &str = "create";

/// 材料创建形态的片段名实参（`arguments.clipName`，可选）。
pub const CLIP_NAME_PARAM: &str = "clipName";

/// `add` 音符对象里的**概率触发**字段名（`ops[].note.probability`，可选）。
///
/// 语义与判定入口都在**模型层**，本层只负责搬运字面值：
///
/// - 取值域 `0.0..=1.0`（含端点），由 [`MidiNote::validate`] 把关 ⇒ 越界是
///   `ProbabilityOutOfRange` ⇒ 契约码 `OUT_OF_RANGE`（见 [`super::error::code_for_model`]）；
/// - 缺省（不写这个字段）= `None` = **必然触发**，与加这个字段之前逐字节相同；
/// - "这一遍响不响"由 `MidiNote::triggers(rng_seed)` **确定性**裁决
///   （`MODEL-AST-005`），实时引擎与离线母带用的是同一个入口。
pub const PROBABILITY_FIELD: &str = "probability";

/// `add` 音符对象里的**连击**字段名（`ops[].note.ratchet`，可选）。
///
/// 语义与判定入口都在**模型层**（[`MidiNote::ratchet`]）：`None` 等价于 1；
/// 取值域 `1..=16` 由 [`MidiNote::validate`] 把关（`RATCHET_MIN`/`RATCHET_MAX`）
/// ⇒ 越界是 `RatchetOutOfRange` ⇒ 契约码 `OUT_OF_RANGE`。
///
/// 为什么本字段值得一条工具面通路：离线母带渲染器**已经**按模型语义展开它
/// （`crate::domain::render` 的连击一节：`step = (duration_ticks / ratchet).max(1)`，
/// 与实时引擎同一条公式），但在这个字段接线之前**没有任何工具**能写它 ——
/// 于是"渲染器会展开连击"这件已实现的能力在 17 个工具的面上**不可达**。
/// 交付形态：只搬字面值进 `MidiNote`，"怎么分"不在这层另立第二份规则。
pub const RATCHET_FIELD: &str = "ratchet";

/// `add` 音符对象里的**微时序**字段名（`ops[].note.microTimingTicks`，可选，单位：tick）。
///
/// 语义与判定入口都在**模型层**（[`MidiNote::micro_timing_ticks`]）：`None` 等价于 0；
/// 取值域 `-240..=240` 由 [`MidiNote::validate`] 把关（`MICRO_TIMING_MAX_ABS`）
/// ⇒ 越界是 `MicroTimingOutOfRange` ⇒ 契约码 `OUT_OF_RANGE`。
///
/// 渲染器同样**已经**把它并入起点（`crate::domain::render` 的排程：
/// `placement.start_tick + note.start_tick + micro_timing_ticks`，经
/// `crate::domain::render_math::note_frame_span`）。
pub const MICRO_TIMING_FIELD: &str = "microTimingTicks";

/// `add.note` 对象**允许**出现的全部键。
///
/// 与 [`parse_note`] 真正读取的键**同源**（判据 `expressive_note_field_names_are_pinned`
/// 钉住"不多报"）：集合之外的键一律**响亮拒绝**（[`reject_unknown_note_fields`]），
/// 绝不静默丢弃 —— 顶层实参已经是这个口径（`ToolCall` 的 `UnknownParam`：拼错的参数
/// 必须被拒绝、不能静默忽略），同一条纪律不许只守一层。
pub const NOTE_FIELDS: &[&str] = &[
    "id",
    "startTick",
    "pitch",
    "durationTicks",
    "velocity",
    PROBABILITY_FIELD,
    RATCHET_FIELD,
    MICRO_TIMING_FIELD,
];

/// `yeban_edit_notes` 的**摆放**实参名（`arguments.placement`，可选）。
///
/// 语义：把**已经在 `clip_pool` 里**的片段摆到 `trackId` 的时间轴上
/// （模型 `Op::AddClipPlacement`，渲染器**真的**消费它 —— `crate::domain::render`
/// 只遍历 `track.clips`，池子里没被摆放的片段一帧都不出声）。
///
/// 为什么扩本工具而不新增工具：台账 `docs/ledger/mcp-tools-expansion-notes.md` §6 的
/// needs-6 记的事实是"没有『放置/引用片段』的工具"，并给出两条出路 —— 新增
/// `yeban_place_clip`，**或扩展 `yeban_edit_notes`**。`ADR-0001` **D46** 的扩张原则是
/// "先扩既有工具的参数，只有确实不合适才新增工具"，而新增工具必须同步
/// `schemas/mcp-tools.schema.json` 的 `name.enum` + `ExtensionToolArguments` + `allOf`
/// 三处（本线禁改 `schemas/**`）。
///
/// 本工具此前已经有 `create: true`（**建**材料，`f1098e2`）这一形态；本参数补上它的
/// 下一半（**摆**材料）。`§7.2` 的参数表因此**一字未动**：`ops` 仍是必填实参
/// （只摆放的调用给空数组，见 [`parse_ops`] 的空数组口径）。
pub const PLACEMENT_FIELD: &str = "placement";

/// `placement` 对象里 `add` 形态**允许**出现的全部键。
///
/// 与 [`parse_placement`] 真正读取的键**同源**（判据 `placement_field_names_are_pinned`
/// 钉住"不多报"）：集合之外的键一律**响亮拒绝**
/// （[`reject_placement_fields`]），绝不静默丢弃 —— 与 [`NOTE_FIELDS`] 同一口径。
///
/// 这个词表**只**管 `add` 形态的**内容键**：`kind` 是形态判别键，不在本表里。
pub const PLACEMENT_FIELDS: &[&str] = &["startTick", "durationTicks", "placementId", "muted"];

/// `placement.kind` 的字段名（形态判别键，可选；缺省 = [`PLACEMENT_KIND_ADD`]）。
///
/// 与 `ops[].kind` 同一风格：`kind` 说的是"这一次摆放编辑是哪个动词"，
/// 其余键是那个动词的载荷。
pub const PLACEMENT_KIND_FIELD: &str = "kind";

/// `placement.kind` 的**新增**形态：把**已在池子里**的片段摆到时间轴上
/// （[`Op::AddClipPlacement`]）。也是 `kind` 缺省时的形态 ⇒ 缺省路径逐字节不变。
pub const PLACEMENT_KIND_ADD: &str = "add";

/// `placement.kind` 的**平移**形态：改动一条**已经存在**的摆放的起点。
pub const PLACEMENT_KIND_MOVE: &str = "move";

/// `placement.kind` 的**取走**形态：把一条**已经存在**的摆放从时间轴上移除。
pub const PLACEMENT_KIND_REMOVE: &str = "remove";

/// `placement.kind` 的合法取值集合（错误信息与判据共用同一份真相）。
pub const PLACEMENT_KINDS: [&str; 3] = [
    PLACEMENT_KIND_ADD,
    PLACEMENT_KIND_MOVE,
    PLACEMENT_KIND_REMOVE,
];

/// `add` 形态允许的键 = [`PLACEMENT_FIELDS`] **加上**判别键。
///
/// 单列一个常量是为了不动 [`PLACEMENT_FIELDS`]：后者是 `add` 形态的内容键表，
/// 由判据 `placement_field_names_are_pinned` 逐个钉住；扩展形态时**不改**那张表。
pub const PLACEMENT_ADD_FIELDS: &[&str] = &[
    PLACEMENT_KIND_FIELD,
    "startTick",
    "durationTicks",
    "placementId",
    "muted",
];

/// `move` 形态允许的键：判别键 + 被平移的摆放身份 + **新的**起点。
///
/// `durationTicks` / `muted` 不在表里：[`Op::MoveClipPlacement`] 的载荷**只有**
/// `old_start_tick` / `new_start_tick`，模型层没有"改时值 / 改静音"的变体
/// ⇒ 给出这两个键是**已知但此形态不适用**，[`reject_placement_fields`] 会响亮拒绝
/// （`placementFieldNotApplicable`），绝不静默丢弃。
pub const PLACEMENT_MOVE_FIELDS: &[&str] = &[PLACEMENT_KIND_FIELD, "startTick", "placementId"];

/// `remove` 形态允许的键：判别键 + 被取走的摆放身份。
pub const PLACEMENT_REMOVE_FIELDS: &[&str] = &[PLACEMENT_KIND_FIELD, "placementId"];

/// `ops[].kind` 的**静态混音值**形态名（写 [`Op::SetParam`]）。
///
/// 与模型 `Op` 变体名同词（`SetParam` 的小驼峰），与其余四个 kind
/// （`add` / `delete` / `move` / `velocity`）同一风格。
pub const SET_PARAM_KIND: &str = "setParam";

/// `setParam` 的**目标**字段名（`ops[].lane`，必填）。
///
/// 借用 `yeban_edit_automation` 的同名实参与 [`LaneKind`] 的同一份词汇表
/// （`ADR-0001` D48：同一个词必须同一个意思）——目标名逐字等于 `project.json` 的变体名。
pub const SET_PARAM_LANE_FIELD: &str = "lane";

/// `setParam` 的**新值**字段名（`ops[].value`，必填，数字）。
pub const SET_PARAM_VALUE_FIELD: &str = "value";

/// `ops[].kind` 的**音轨静音**形态名（写 [`Op::SetTrackMute`]）。
///
/// 与模型 `Op` 变体名同词（`SetTrackMute` 的小驼峰），与 [`SET_PARAM_KIND`] 同一条规则。
pub const SET_TRACK_MUTE_KIND: &str = "setTrackMute";

/// `ops[].kind` 的**音轨独奏**形态名（写 [`Op::SetTrackSolo`]）。
pub const SET_TRACK_SOLO_KIND: &str = "setTrackSolo";

/// 音轨开关形态的**新值**字段名（`ops[].value`，必填，布尔）。
///
/// 与 [`SET_PARAM_VALUE_FIELD`] 逐字同词（`ADR-0001` D48：同一个词必须同一个意思 ——
/// "这次要写进去的值"），但**类型不同**：开关只收 JSON 布尔，不做真假值强转。
pub const TRACK_FLAG_VALUE_FIELD: &str = "value";

/// 音轨开关形态允许出现的**全部**键（判别键 + 新值键）。
///
/// 目标音轨**不在**这里：它是工具顶层的 `trackId`（与 `setParam` 同一条口径）。
/// 多写一个键（尤其是嵌套的 `trackId`）是**响亮失败**，不静默丢弃。
pub const TRACK_FLAG_FIELDS: [&str; 2] = ["kind", TRACK_FLAG_VALUE_FIELD];

/// `ops[].kind` 的**全集**（规范顺序：四个音符 / 池级 / 摆放形态在前，
/// 音轨级、路由级、段落级与场景级形态在后）。
///
/// 错误信息（[`parse_one`] 的未知 `kind`）与判据共用这一份真相。
pub const OP_KINDS: [&str; 16] = [
    "add",
    "delete",
    "move",
    "velocity",
    REMOVE_CLIP_KIND,
    SET_PARAM_KIND,
    SET_TRACK_MUTE_KIND,
    SET_TRACK_SOLO_KIND,
    SET_AUTOMATION_LANE_KIND,
    REMOVE_AUTOMATION_POINT_KIND,
    SET_ROUTING_GAIN_KIND,
    DISCONNECT_ROUTING_KIND,
    REMOVE_ROUTING_NODE_KIND,
    REMOVE_SECTION_KIND,
    REMOVE_SCENE_KIND,
    SET_SCENE_KIND,
];

/// `setParam` 能写的**静态目标**（[`Op::SetParam`] 里"有静态值可写"的那两个）。
///
/// 为什么单列一个二值枚举而不是直接收 [`LaneKind`]：`SendGain` 的模型语义是
/// `Option<f32>`（`None` = 单位增益），`SetParam` **明文拒绝**它（必须走
/// `Op::SetRoutingGain`）；`DeviceParam` / `Macro` 的静态写入在本工具面**没有**
/// 通路。用二值类型把"哪三个不可写"变成**不可表达**，比在 `compile` 里补一条
/// 不可达分支更诚实。
///
/// `SendGain` 的静态增益由 [`SET_ROUTING_GAIN_KIND`] 承载（那是**另一个**模型变体，
/// 因此仍然**不**属于本枚举 —— 把它并进来就是让本枚举的名字说谎）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticLane {
    /// 音轨静态音量（`TrackV3::volume_db`，单位 dB，有限值）。
    TrackVolume,
    /// 音轨静态声相（`TrackV3::pan`，-1.0..=1.0）。
    TrackPan,
}

impl StaticLane {
    /// 允许的目标名，**规范顺序**（错误信息的 `allowed` 与判据共用）。
    ///
    /// 从 [`LaneKind`] 自己的 `as_str` 派生 ⇒ 词汇表**只有一份**（不是第二张手写表）。
    pub const NAMES: [&'static str; 2] =
        [LaneKind::TrackVolume.as_str(), LaneKind::TrackPan.as_str()];

    /// 把目标落到具体音轨上（[`Op::SetParam`] 的载荷）。
    #[must_use]
    pub fn target(self, track_id: EntityId) -> AutomationTarget {
        match self {
            Self::TrackVolume => AutomationTarget::TrackVolume { track_id },
            Self::TrackPan => AutomationTarget::TrackPan { track_id },
        }
    }
}

/// 通道条上的一个**布尔开关**（[`Op::SetTrackMute`] / [`Op::SetTrackSolo`]）。
///
/// 为什么单列一个二值枚举：两个 `kind` 的**载荷完全相同**（一个 `bool`），
/// 差异只在目标字段与模型变体上。用类型把这条差异收成一处，
/// [`parse_one`] 与 [`compile`] 各自只有**一个**开关分支（不是两份会漂移的复制）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackFlag {
    /// 音轨静音（`TrackV3::mute`，[`Op::SetTrackMute`]）。
    Mute,
    /// 音轨独奏（`TrackV3::solo`，[`Op::SetTrackSolo`]）。
    Solo,
}

impl TrackFlag {
    /// 两个形态名，**规范顺序**（错误信息的 `allowed` 与判据共用）。
    pub const NAMES: [&'static str; 2] = [SET_TRACK_MUTE_KIND, SET_TRACK_SOLO_KIND];

    /// 该开关在 `arguments.ops[].kind` 里的字面名字（错误信息与判据共用同一份真相）。
    #[must_use]
    pub const fn kind_name(self) -> &'static str {
        match self {
            Self::Mute => SET_TRACK_MUTE_KIND,
            Self::Solo => SET_TRACK_SOLO_KIND,
        }
    }

    /// 撤销载荷要读的**当前**开关态（`TrackV3::mute` / `TrackV3::solo`）。
    ///
    /// 与模型 `apply` 的前置条件读的是**同一个字段**：本层不复制那份判定，
    /// 只是把文档现值搬进 `Op` 的 `old_*`。
    #[must_use]
    pub const fn read(self, track: &TrackV3) -> bool {
        match self {
            Self::Mute => track.mute,
            Self::Solo => track.solo,
        }
    }

    /// 编译成模型变体（`old_*` 由调用方从文档读入）。
    #[must_use]
    pub const fn compile(self, track_id: EntityId, old: bool, new: bool) -> Op {
        match self {
            Self::Mute => Op::SetTrackMute {
                track_id,
                old_mute: old,
                new_mute: new,
            },
            Self::Solo => Op::SetTrackSolo {
                track_id,
                old_solo: old,
                new_solo: new,
            },
        }
    }
}

/// 一次 `placement` 实参要做的**摆放编辑**（三种形态的编译结果）。
///
/// 三个变体逐一对应模型层的三个 `Op`：`Add` → [`Op::AddClipPlacement`]、
/// `Move` → [`Op::MoveClipPlacement`]、`Remove` → [`Op::RemoveClipPlacement`]。
/// 撤销仍然只有**一份**事实源：本层只把字面值搬进 `Op`，逆操作一律由
/// `Op::invert` 提供（与 [`super::compile`] 同一纪律）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacementEdit {
    /// 把池子里的材料摆到 `trackId` 上。
    Add(ClipPlacement),
    /// 把 `placement_id` 这条已有摆放的起点从 `previous_start_tick` 挪到
    /// `new_start_tick`（两者都取自 / 写回**文档**，不是调用方的声明）。
    Move {
        /// 被平移的摆放身份。
        placement_id: EntityId,
        /// 文档里的现值（模型层据此判 `OpStateMismatch`）。
        previous_start_tick: u64,
        /// 目标起点。
        new_start_tick: u64,
    },
    /// 把 `placement_id` 这条已有摆放从时间轴上取走。
    Remove {
        /// 被取走的摆放身份。
        placement_id: EntityId,
        /// 文档里的现值（模型层的撤销载荷，必须逐字段等于文档现值）。
        previous_placement: ClipPlacement,
    },
}

impl PlacementEdit {
    /// 该形态在 `placement.kind` 里的字面名字（错误信息与判据共用同一份真相）。
    #[must_use]
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::Add(_) => PLACEMENT_KIND_ADD,
            Self::Move { .. } => PLACEMENT_KIND_MOVE,
            Self::Remove { .. } => PLACEMENT_KIND_REMOVE,
        }
    }
}

/// 单个片段的**发声数**上限（同时发声的音符数）。
///
/// §7.2 要求 `yeban_edit_notes` "自动进行音域与发声数合法性校验"；
/// 音域（`0..=127`）由模型层把关，发声数这一半由本常量把关。
/// 32 是"一个片段内同时 32 个音符"的保守上限（超过它多半是批量生成的产物，
/// 而不是编曲意图）；超过即 `OUT_OF_RANGE` 并回报**峰值重叠**与位置。
pub const MAX_POLYPHONY: usize = 32;

/// `ops[].kind` 的**自动化泳道属性**形态名（写 [`Op::SetAutomationLane`] /
/// [`Op::RemoveAutomationLane`]）。
///
/// 与模型 `Op` 变体名同词（`SetAutomationLane` 的小驼峰），与 [`SET_PARAM_KIND`] /
/// [`SET_TRACK_MUTE_KIND`] 同一条命名规则。
pub const SET_AUTOMATION_LANE_KIND: &str = "setAutomationLane";

/// 泳道形态的**载荷对象**字段名（`ops[].lane`，必填）。
///
/// 目标与属性都在这个对象里（自包含）：`lane` 的寻址字段
/// （`lane` / `edgeId` / `slotIndex` / `paramIndex` / `macroIndex`）与
/// `yeban_edit_automation` 的**同名实参**逐字同词、共用 [`LaneKind`] 那一份词表
/// （`ADR-0001` D48）——寻址词表本身来自 [`LaneKind::parse`]，
/// 额外的分量名逐字照 `yeban_edit_automation` 的实参名。
///
/// 为什么不是把寻址放到顶层实参：本工具的顶层 `trackId` 是**音轨**身份，
/// 而泳道目标的额外分量（边 / 槽 / 参数 / 宏下标）**只有本形态**需要；
/// 塞进顶层就会让别的 `kind` 也凭空多出四个无关实参。
///
/// 为什么**不**直接复用 `super::automation::parse_target`：那个函数在**解析期**
/// 就构造 [`AutomationTarget`]，而音轨身份在解析期**还不知道** —— 它由
/// `arguments.trackId` 在 [`compile`] 里给出。把占位身份塞进去再事后修补，
/// 会造出一个"看起来是身份、其实不是"的中间值。本类型因此只在解析期保留
/// **分量**，目标在 [`LaneTargetSpec::target`] 里、拿到真实音轨身份之后才构造。
pub const SET_AUTOMATION_LANE_FIELD: &str = "lane";

/// 泳道属性的**读开关**字段名（`ops[].lane.readEnabled`，可选布尔）。
///
/// 与 `yeban_edit_automation` 响应里的 `lane.readEnabled` 逐字同词（D48）。
pub const LANE_READ_ENABLED_FIELD: &str = "readEnabled";

/// 泳道属性的**写模式**字段名（`ops[].lane.writeMode`，可选字符串）。
///
/// 取值 = `AutomationWriteMode` 的规范变体名（`Off` / `Write` / `Touch` / `Latch`），
/// 由该枚举自己的 serde 名字派生 ⇒ 词表**只有一份**。
pub const LANE_WRITE_MODE_FIELD: &str = "writeMode";

/// 泳道属性的**取值域覆盖**字段名（`ops[].lane.domain`，可选）。
///
/// `{"min":…,"max":…}` = 设显式覆盖（两端点由 [`AutomationValueDomain::new`] 排序，
/// 与模型同一份构造）；明写 `null` = 清掉覆盖（回到 `None` = 派生自目标）；
/// **缺省** = 保留现值。三者是三件不同的事，因此不做"null 与缺省同义"的折叠。
pub const LANE_DOMAIN_FIELD: &str = "domain";

/// 泳道属性的**取走**字段名（`ops[].lane.remove`，可选布尔，缺省 `false`）。
///
/// `true` = 用 [`Op::RemoveAutomationLane`] 把这条泳道从文档里取走；
/// 与任何属性键同给是**响亮失败**（见模块头的三条口径）。为什么必须有这条路：
/// 一条被关掉读开关（或带写模式）的空泳道**不会被**模型自动回收
/// （`AutomationLane::is_implicit` 为 `false`），没有它就会被永久卡住。
pub const LANE_REMOVE_FIELD: &str = "remove";

/// `ops[].lane` 对象**允许**出现的全部键。
///
/// 与 [`parse_lane_edit`] 真正读取的键**同源**（判据 `lane_field_names_are_pinned`
/// 钉住"不多报"）：集合之外的键一律**响亮拒绝**（`unknownLaneField`），
/// 绝不静默丢弃 —— 与 [`NOTE_FIELDS`] / [`TRACK_FLAG_FIELDS`] 同一口径。
pub const SET_AUTOMATION_LANE_FIELDS: [&str; 9] = [
    SET_AUTOMATION_LANE_FIELD,
    "edgeId",
    "slotIndex",
    "paramIndex",
    "macroIndex",
    LANE_READ_ENABLED_FIELD,
    LANE_WRITE_MODE_FIELD,
    LANE_DOMAIN_FIELD,
    LANE_REMOVE_FIELD,
];

/// `AutomationWriteMode` 的规范变体名（错误信息的 `allowed` 与判据共用）。
///
/// 顺序 = 该枚举的声明顺序；名字与序列化名同源（[`write_mode_name`]）。
pub const LANE_WRITE_MODES: [&str; 4] = ["Off", "Write", "Touch", "Latch"];

/// `ops[].kind` 的**自动化点取走**形态名（写 [`Op::RemoveAutomationPoint`]）。
///
/// 与模型 `Op` 变体名同词（`RemoveAutomationPoint` 的小驼峰），与 [`SET_PARAM_KIND`] /
/// [`SET_TRACK_MUTE_KIND`] / [`SET_AUTOMATION_LANE_KIND`] 同一条命名规则。
pub const REMOVE_AUTOMATION_POINT_KIND: &str = "removeAutomationPoint";

/// 点形态的**载荷对象**字段名（`ops[].point`，必填）。
///
/// 对象**自包含**：泳道寻址（`lane` / `edgeId` / `slotIndex` / `paramIndex` /
/// `macroIndex`，与 [`SET_AUTOMATION_LANE_FIELD`] 那一个对象同词同源）+ 点寻址
/// （`tick` 或 `pointId`）。为什么不是两个平铺的键：工具顶层只有 `trackId`（音轨），
/// 泳道的其余分量（边 / 槽 / 参数 / 宏下标）无处可放。
pub const REMOVE_POINT_FIELD: &str = "point";

/// 点寻址的 **tick** 字段名（`ops[].point.tick`，与 [`REMOVE_POINT_ID_FIELD`] 恰好给一个）。
///
/// 语义 = "文档里**恰好一个**落在该 tick 上的点"（0 个 / ≥2 个都是响亮失败，
/// 见 [`unique_point_at`]）。与 `yeban_edit_automation` 的 `point.tick` 同义：
/// 都是"这个自动化点的时间位置"。
pub const REMOVE_POINT_TICK_FIELD: &str = "tick";

/// 点寻址的**显式身份**字段名（`ops[].point.pointId`，与 [`REMOVE_POINT_TICK_FIELD`] 恰好给一个）。
///
/// 名字与 `yeban_edit_automation` 的同名实参逐字同词（`ADR-0001` D48）——
/// `yeban_edit_automation` 的读侧 `lane.points[].id` 报的就是这个身份。
pub const REMOVE_POINT_ID_FIELD: &str = "pointId";

/// `ops[].point` 对象**允许**出现的全部键。
///
/// 前五个是**泳道寻址**（与 [`LANE_TARGET_FIELDS`] 同源，由判据钉住），后两个是
/// **点寻址**。集合之外的键一律**响亮拒绝**，绝不静默丢弃 ——
/// 与 [`NOTE_FIELDS`] / [`TRACK_FLAG_FIELDS`] / [`SET_AUTOMATION_LANE_FIELDS`] 同一口径。
pub const REMOVE_POINT_FIELDS: [&str; 7] = [
    SET_AUTOMATION_LANE_FIELD,
    "edgeId",
    "slotIndex",
    "paramIndex",
    "macroIndex",
    REMOVE_POINT_TICK_FIELD,
    REMOVE_POINT_ID_FIELD,
];

/// 泳道**寻址**字段（[`SET_AUTOMATION_LANE_FIELDS`] 的前五个）。
///
/// 单列一个常量是为了让"`point` 对象允许的泳道键"与"`lane` 对象允许的泳道键"
/// **只有一份**真相：[`REMOVE_POINT_FIELDS`] 的前五项**就是**本表，判据
/// `point_removal_field_names_are_pinned` 机械钉住这条关系（不靠人去比对）。
pub const LANE_TARGET_FIELDS: [&str; 5] = [
    SET_AUTOMATION_LANE_FIELD,
    "edgeId",
    "slotIndex",
    "paramIndex",
    "macroIndex",
];

/// `removeAutomationPoint` 的**操作对象**允许出现的全部键（判别键 + 载荷对象）。
///
/// 点寻址**不在**这里：它在 `point` 对象里（顶层写 `tick` 是"看起来对"的错法，
/// 因此被 [`reject_point_removal_op_fields`] 点名拒绝，而不是静默忽略）。
pub const REMOVE_POINT_OP_FIELDS: [&str; 2] = ["kind", REMOVE_POINT_FIELD];

/// `ops[].kind` 的**片段池取走**形态名（写 [`Op::RemoveClip`]）。
///
/// 与模型 `Op` 变体名同词（`RemoveClip` 的小驼峰），与 [`SET_PARAM_KIND`] /
/// [`REMOVE_AUTOMATION_POINT_KIND`] 同一条命名规则。
pub const REMOVE_CLIP_KIND: &str = "removeClip";

/// 池级形态的**操作对象**允许出现的全部键（只有判别键 —— 载荷是**空**的）。
///
/// 目标片段**不在**这里：它是工具顶层的 `clipId`（与 `create: true` 的目标同一条口径）。
/// 多写一个键（尤其是嵌套的 `clipId`）是**响亮失败**，不静默丢弃 ——
/// 与 [`TRACK_FLAG_FIELDS`] / [`REMOVE_POINT_OP_FIELDS`] 同一纪律。
pub const REMOVE_CLIP_FIELDS: [&str; 1] = ["kind"];

/// `ops[].kind` 的**路由边增益**形态名（写 [`Op::SetRoutingGain`]）。
///
/// 与模型 `Op` 变体名同词（`SetRoutingGain` 的小驼峰），与 [`SET_PARAM_KIND`] /
/// [`REMOVE_CLIP_KIND`] 同一条命名规则。
pub const SET_ROUTING_GAIN_KIND: &str = "setRoutingGain";

/// **路由边**的寻址字段名（`ops[].edgeId`，必填）。
///
/// 两条路由级形态（[`SET_ROUTING_GAIN_KIND`] 与 [`DISCONNECT_ROUTING_KIND`]）寻址的是
/// **同一种实体**，因此共用这一个字面量 —— 词汇表只有一份（不是两张会各自漂移的表）。
///
/// 与 `yeban_edit_automation` 的同名实参逐字同词（`ADR-0001` D48：同一个词必须
/// 同一个意思 —— "那条路由边"），而 `SendGain` 泳道的寻址在 `setAutomationLane` 里
/// 住在嵌套的 `lane.edgeId`（那边还带着泳道属性，两条路由级形态都只有这一个字段）。
pub const ROUTING_EDGE_FIELD: &str = "edgeId";

/// **路由节点**的寻址字段名（`ops[].nodeId`，必填）。
///
/// 与 [`ROUTING_EDGE_FIELD`] **不同一个实体**（节点是 `routing_graph.nodes` 里的身份，
/// 边是 `routing_graph.edges` 里的身份），因此刻意**不共用**那个字面量 ——
/// 共用一个词会让"边"与"节点"在工具面上无法区分。
///
/// 与 `yeban_query_project` 的 `routing_graph.nodes` 数组里的身份同一个东西
/// （`ADR-0001` D48：同一个词必须同一个意思），命名与 `edgeId` / `trackId` / `noteId`
/// 同一规则（"被寻址的实体" + `Id`）。
pub const ROUTING_NODE_FIELD: &str = "nodeId";

/// 路由边增益形态的**寻址**字段名（= [`ROUTING_EDGE_FIELD`]，同一个字面量）。
pub const SET_ROUTING_GAIN_EDGE_FIELD: &str = ROUTING_EDGE_FIELD;

/// 路由边增益形态的**新值**字段名（`ops[].value`，必填，数字**或** `null`）。
///
/// 与 [`SET_PARAM_VALUE_FIELD`] / [`TRACK_FLAG_VALUE_FIELD`] 同词（"这次要写进去的
/// 值"），但**类型不同**：本字段是 `Option<f32>` 的字面形态 —— `null` = 模型的
/// `None` = **单位增益**，数字 = `Some(f32)`。两者**必须可区分**（见模块头）。
pub const SET_ROUTING_GAIN_VALUE_FIELD: &str = "value";

/// 路由边增益形态允许出现的**全部**键（判别键 + 寻址键 + 新值键）。
///
/// 目标音轨与目标片段**都不在**这里：本形态自带寻址（`edgeId`），
/// 与 [`REMOVE_CLIP_FIELDS`] 同一纪律（多写一个键是**响亮失败**，不静默丢弃）。
pub const SET_ROUTING_GAIN_FIELDS: [&str; 3] = [
    "kind",
    SET_ROUTING_GAIN_EDGE_FIELD,
    SET_ROUTING_GAIN_VALUE_FIELD,
];

/// `ops[].kind` 的**断开路由边**形态名（写 [`Op::DisconnectRouting`]）。
///
/// 与模型 `Op` 变体名同词（`DisconnectRouting` 的小驼峰），与 [`SET_ROUTING_GAIN_KIND`] /
/// [`REMOVE_CLIP_KIND`] 同一条命名规则。
pub const DISCONNECT_ROUTING_KIND: &str = "disconnectRouting";

/// 断开路由边形态允许出现的**全部**键（判别键 + 寻址键）。
///
/// 目标音轨、目标片段与边的旧增益**都不在**这里：本形态自带寻址
/// （[`ROUTING_EDGE_FIELD`]），撤销载荷 `previous_edge` 由 [`compile`] 从**当前文档**
/// 读（与 [`SET_ROUTING_GAIN_KIND`] 的 `old_gain_db` 同一条纪律）。
/// 多写一个键是**响亮失败**，不静默丢弃。
pub const DISCONNECT_ROUTING_FIELDS: [&str; 2] = ["kind", ROUTING_EDGE_FIELD];

/// `ops[].kind` 的**取走路由节点**形态名（写 [`Op::RemoveRoutingNode`]）。
///
/// 与模型 `Op` 变体名同词（`RemoveRoutingNode` 的小驼峰），与 [`DISCONNECT_ROUTING_KIND`] /
/// [`REMOVE_CLIP_KIND`] 同一条命名规则。
pub const REMOVE_ROUTING_NODE_KIND: &str = "removeRoutingNode";

/// 取走路由节点形态允许出现的**全部**键（判别键 + 寻址键）。
///
/// 目标音轨、目标片段与节点的旧状态**都不在**这里：本形态自带寻址
/// （[`ROUTING_NODE_FIELD`]），而 `RemoveRoutingNode` 的模型载荷**只有**节点身份
/// —— 它没有撤销载荷（与 `AddRoutingNode` 互为逆操作，两份载荷都不需要）。
/// 多写一个键是**响亮失败**，不静默丢弃。
pub const REMOVE_ROUTING_NODE_FIELDS: [&str; 2] = ["kind", ROUTING_NODE_FIELD];

/// `ops[].kind` 的**取走曲式段落**形态名（写 [`Op::RemoveSection`]）。
///
/// 与模型 `Op` 变体名同词（`RemoveSection` 的小驼峰），与 [`REMOVE_ROUTING_NODE_KIND`] /
/// [`REMOVE_CLIP_KIND`] 同一条命名规则。
pub const REMOVE_SECTION_KIND: &str = "removeSection";

/// 取走段落形态的**目标**字段名（`ops[].sectionId`，必填）。
///
/// 段落身份与路由节点身份（[`ROUTING_NODE_FIELD`]）、片段身份（顶层 `clipId`）**不是**
/// 同一个字面量：三者是三种实体，共用一个词会让"取走的是哪一个"从形状上无法区分。
pub const SECTION_FIELD: &str = "sectionId";

/// 取走段落形态允许出现的**全部**键（判别键 + 寻址键）。
///
/// 目标音轨、目标片段与段落的旧状态**都不在**这里：本形态自带寻址（[`SECTION_FIELD`]），
/// 而撤销载荷 `previous_section` 由 [`compile`] 从**当前文档**读。
/// 多写一个键是**响亮失败**，不静默丢弃。
pub const REMOVE_SECTION_FIELDS: [&str; 2] = ["kind", SECTION_FIELD];

/// `ops[].kind` 的**取走场景**形态名（写 [`Op::RemoveScene`]）。
///
/// 与模型 `Op` 变体名同词（`RemoveScene` 的小驼峰），与 [`REMOVE_SECTION_KIND`] /
/// [`REMOVE_ROUTING_NODE_KIND`] 同一条命名规则。
pub const REMOVE_SCENE_KIND: &str = "removeScene";

/// 取走场景形态的**目标**字段名（`ops[].sceneId`，必填）。
///
/// 场景身份与段落身份（[`SECTION_FIELD`]）、路由节点身份（[`ROUTING_NODE_FIELD`]）、
/// 片段身份（顶层 `clipId`）**不是**同一个字面量：四者是四种实体，共用一个词会让
/// "取走的是哪一个"从形状上无法区分。
pub const SCENE_FIELD: &str = "sceneId";

/// 取走场景形态允许出现的**全部**键（判别键 + 寻址键）。
///
/// 目标音轨、目标片段与场景的旧状态**都不在**这里：本形态自带寻址（[`SCENE_FIELD`]），
/// 而撤销载荷 `previous_scene` 由 [`compile`] 从**当前文档**读。
/// 多写一个键是**响亮失败**，不静默丢弃。
pub const REMOVE_SCENE_FIELDS: [&str; 2] = ["kind", SCENE_FIELD];

/// `ops[].kind` 的**写入场景**形态名（写 [`Op::SetScene`]）。
///
/// 与模型 `Op` 变体名同词（`SetScene` 的小驼峰），与 [`REMOVE_SCENE_KIND`] /
/// [`SET_AUTOMATION_LANE_KIND`] 同一条命名规则。
pub const SET_SCENE_KIND: &str = "setScene";

/// 写入场景形态的**载荷**字段名（`ops[].scene`，必填，对象）。
///
/// 与 `setAutomationLane` 的 [`SET_AUTOMATION_LANE_FIELD`]（`lane`）/
/// `removeAutomationPoint` 的 [`REMOVE_POINT_FIELD`]（`point`）同一形状：
/// 目标与属性都装在这个**自包含**的对象里，工具顶层的 `trackId` / `clipId`
/// 与本形态无关。
pub const SCENE_PAYLOAD_FIELD: &str = "scene";

/// 写入场景形态的**新建**开关字段名（`ops[].scene.create`，可选，缺省 `false`）。
///
/// 与 [`CREATE_PARAM`]（工具顶层）同词同义（`ADR-0001` D48）："目标不存在才新建，
/// 已存在就响亮拒绝"。为什么**必须**有它：靠"文档里没有这个身份"推断新建，会让一个
/// 打错的 `sceneId` 静默建出一个新场景。
pub const SCENE_CREATE_FIELD: &str = "create";

/// 写入场景形态的**场景名**字段名（`ops[].scene.name`）。
///
/// 新建时**必填**，更新时可省（缺省 = 保留现值）。`SceneV3::name` 是普通 `String`，
/// 模型对它没有任何校验 ⇒ 空名在这里就被响亮拒绝（绝不静默落盘一个没有名字的场景）。
pub const SCENE_NAME_FIELD: &str = "name";

/// 写入场景形态的**速度覆盖**字段名（`ops[].scene.tempo`，可选）。
///
/// 三态：缺省 = 保留现值；数字 = `Some(f64)`；`null` = 清空（跟随工程速度）。
/// 值域（20.0..=999.0）与有限性由模型 `SceneV3::validate` 判。
pub const SCENE_TEMPO_FIELD: &str = "tempo";

/// 写入场景形态的**界面色标**字段名（`ops[].scene.color`，可选）。
///
/// 三态与 [`SCENE_TEMPO_FIELD`] 相同（缺省 = 保留现值；字符串 = 覆盖；`null` = 清空）。
pub const SCENE_COLOR_FIELD: &str = "color";

/// 写入场景形态的 `scene` 对象允许出现的**全部**键（判别键 + 寻址键 + 三个属性键）。
///
/// 场景的**旧状态**不在里面：撤销载荷 `old_scene` 由 [`compile`] 从**当前文档**读。
/// 多写一个键是**响亮失败**，不静默丢弃。
pub const SET_SCENE_FIELDS: [&str; 5] = [
    SCENE_CREATE_FIELD,
    SCENE_FIELD,
    SCENE_NAME_FIELD,
    SCENE_TEMPO_FIELD,
    SCENE_COLOR_FIELD,
];

/// 写入场景形态的**操作对象**允许出现的全部键（判别键 + 载荷键）。
///
/// 与 [`REMOVE_POINT_OP_FIELDS`] 同一条口径：寻址与属性都在载荷对象里，
/// 操作对象顶层只认这两个键（嵌套的 `sceneId` 直接放在顶层是拼写错误）。
pub const SET_SCENE_OP_FIELDS: [&str; 2] = ["kind", SCENE_PAYLOAD_FIELD];

/// 泳道目标在**解析期**的形态：变体 + 额外分量（**不含**音轨身份）。
///
/// 目标名逐字等于 `project.json` 的变体名（[`LaneKind::parse`] 那一份词表）；
/// `edgeId` / `slotIndex` / `paramIndex` / `macroIndex` 逐字等于
/// `yeban_edit_automation` 的同名实参。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaneTargetSpec {
    /// 目标变体。
    pub kind: LaneKind,
    /// `SendGain` 的路由边身份。
    pub edge_id: Option<EntityId>,
    /// `DeviceParam` 的设备链下标（其余变体不用，恒 0）。
    pub slot_index: usize,
    /// `DeviceParam` 的参数下标（其余变体不用，恒 0）。
    pub param_index: usize,
    /// `Macro` 的宏下标（其余变体不用，恒 0）。
    pub macro_index: usize,
}

impl LaneTargetSpec {
    /// 解析 `lane` 对象里的**寻址**部分（属性键由 [`parse_lane_patch`] 读）。
    ///
    /// 目标名走 [`LaneKind::parse`]（与 `yeban_edit_automation` 同一份词表）；
    /// 额外分量按该变体**是否真的需要**读取 —— 不需要的分量恒 `0`/`None`，
    /// 因此"同一个目标用两种写法传实参"不会派生出两个不同的目标
    /// （与 `yeban_edit_automation` 的 `needs_*` 口径逐条一致）。
    ///
    /// # Errors
    ///
    /// - `lane` 缺失 / 不是字符串 → `INVALID_PARAMETER_RANGE`；
    /// - 目标名不在 [`LANE_NAMES`](super::extension_pure::LANE_NAMES) 里（含别名）
    ///   → `unknownLaneTarget`（带 `allowed` 全集）；
    /// - `SendGain` 缺 `edgeId`（或不是合法 ULID）→ `INVALID_PARAMETER_RANGE`；
    /// - `slotIndex` / `paramIndex` / `macroIndex` 不是非负整数或超出 `usize`。
    fn parse(lane: &Map<String, Value>) -> Result<Self, Fault> {
        let raw = lane
            .get(SET_AUTOMATION_LANE_FIELD)
            .ok_or_else(|| missing(SET_AUTOMATION_LANE_FIELD, "字符串"))?;
        let text = raw.as_str().ok_or_else(|| {
            Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!("`{SET_AUTOMATION_LANE_FIELD}` 必须是字符串, 实际收到 {raw}"),
                serde_json::json!({
                    "field": SET_AUTOMATION_LANE_FIELD,
                    "reason": "laneMustBeString",
                    "allowed": super::extension_pure::LANE_NAMES,
                }),
            )
        })?;
        let Some(kind) = LaneKind::parse(text) else {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!("未知自动化目标 `{text}`"),
                serde_json::json!({
                    "field": SET_AUTOMATION_LANE_FIELD,
                    "reason": "unknownLaneTarget",
                    "received": text,
                    "allowed": super::extension_pure::LANE_NAMES,
                    "note": "只接受 project.json 里的规范变体名 (不接受 trackVolume 这类别名)",
                }),
            ));
        };
        let edge_id = if kind.needs_edge_id() {
            let value = lane.get("edgeId").ok_or_else(|| {
                Fault::domain_with_data(
                    ErrorCode::InvalidParameterRange,
                    "`SendGain` 目标必须给定 `edgeId` (路由边身份)",
                    serde_json::json!({ "field": "edgeId", "reason": "edgeIdRequired" }),
                )
            })?;
            let text = value.as_str().ok_or_else(|| {
                Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    format!("`edgeId` 必须是字符串, 实际收到 {value}"),
                )
            })?;
            Some(EntityId::from_str(text).map_err(|error| {
                Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    format!("`edgeId` 不是合法 ULID: {error}"),
                )
            })?)
        } else {
            None
        };
        let slot_index = if kind.needs_device_slot() {
            read_usize(lane, "slotIndex")?
        } else {
            0
        };
        let param_index = if kind.needs_device_slot() {
            read_usize(lane, "paramIndex")?
        } else {
            0
        };
        let macro_index = if kind.needs_macro_index() {
            read_usize(lane, "macroIndex")?
        } else {
            0
        };
        Ok(Self {
            kind,
            edge_id,
            slot_index,
            param_index,
            macro_index,
        })
    }

    /// 落到具体音轨身份上，构造模型的 [`AutomationTarget`]。
    ///
    /// `SendGain` 到这里时 `edge_id` 必然已经是 `Some`（[`Self::parse`] 强制），
    /// 因此这里用 `expect` 而不是再造一个不可达的错误分支 —— 但**不**静默
    /// 回退到"某个默认边"（那会把发送增益写到别的通路上）。
    #[must_use]
    pub fn target(self, track_id: EntityId) -> AutomationTarget {
        match self.kind {
            LaneKind::TrackVolume => AutomationTarget::TrackVolume { track_id },
            LaneKind::TrackPan => AutomationTarget::TrackPan { track_id },
            LaneKind::SendGain => AutomationTarget::SendGain {
                track_id,
                edge_id: self
                    .edge_id
                    .expect("`SendGain` 的 `edgeId` 由 `parse` 强制存在"),
            },
            LaneKind::DeviceParam => AutomationTarget::DeviceParam {
                track_id,
                slot_index: self.slot_index,
                param_index: self.param_index,
            },
            LaneKind::Macro => AutomationTarget::Macro {
                track_id,
                macro_index: self.macro_index,
            },
        }
    }
}

/// 一个自动化点的**寻址**（`ops[].point` 的寻址那一半，恰好二选一）。
///
/// 两个变体逐一对应"怎么指名一个点"的两条路：文档里的**时间位置**（`tick`），
/// 或模型自己的**身份**（`pointId`，`yeban_edit_automation` 读侧 `lane.points[].id`
/// 报的就是它）。本形态**不允许两个同给**（见模块头的理由）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointAddress {
    /// 按 `tick` 寻址：**文档里恰好一个**落在该 tick 上的点
    /// （0 个 → `ENTITY_NOT_FOUND`；≥2 个 → 响亮拒绝，绝不挑一个）。
    Tick(u64),
    /// 按**显式身份**寻址（点不存在 → `ENTITY_NOT_FOUND`）。
    Id(EntityId),
}

/// 一次 `setAutomationLane` 的编译结果（目标的分量 + **替换后**的整条泳道，或"取走"）。
///
/// 单列一个类型（而不是在 [`NoteOp`] 里散着放）是为了让"取走"与"改成什么"
/// 在类型上互斥：`change` 一次只可能是其中之一。
#[derive(Clone, Debug, PartialEq)]
pub struct LaneEdit {
    /// 目标的分量（音轨身份由 [`compile`] 补上）。
    pub spec: LaneTargetSpec,
    /// 调用方给出的**属性覆盖**（`None` = 该键缺省 = 保留现值）。
    pub change: LaneChange,
}

/// `setAutomationLane` 真正要改的东西。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LaneChange {
    /// 把泳道替换成给定的属性组合（未给的键保留文档现值）。
    Set(LanePatch),
    /// 把泳道从文档里取走（[`Op::RemoveAutomationLane`]）。
    Remove,
}

/// 泳道属性的**逐键覆盖**（`None` = 调用方没提这个键 ⇒ 保留现值）。
///
/// 三态刻意不折叠：`domain` 的 `Some(None)`（明写 `null` = 清掉覆盖）与 `None`
/// （缺省 = 保留现值）是两件不同的事。
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct LanePatch {
    /// 读开关。
    pub read_enabled: Option<bool>,
    /// 写模式。
    pub write_mode: Option<AutomationWriteMode>,
    /// 取值域覆盖（外层 `Option` = 调用方提没提；内层 = 覆盖值还是"清掉"）。
    pub domain: Option<Option<AutomationValueDomain>>,
}

impl LanePatch {
    /// 该覆盖是否一个键都没提（`true` ⇒ 替换结果必然等于文档现值，是一次无操作）。
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.read_enabled.is_none() && self.write_mode.is_none() && self.domain.is_none()
    }

    /// 把覆盖施加到一条**基础**泳道上（基础 = 文档现值，或没有泳道时的隐式默认）。
    ///
    /// 返回替换后的整条泳道。`points` **不在**覆盖范围内（采样点是
    /// [`Op::SetAutomationPoint`] 的载荷）：这里原样保留，因此本形态不可能
    /// 顺手丢掉既有采样点。
    #[must_use]
    pub fn apply_to(self, base: &AutomationLane) -> AutomationLane {
        let mut lane = base.clone();
        if let Some(read_enabled) = self.read_enabled {
            lane.read_enabled = read_enabled;
        }
        if let Some(write_mode) = self.write_mode {
            lane.write_mode = write_mode;
        }
        if let Some(domain) = self.domain {
            lane.domain = domain;
        }
        lane
    }
}

/// 一个场景的**逐键覆盖**（`None` = 调用方没提这个键 ⇒ 保留现值）。
///
/// 三态刻意不折叠（与 [`LanePatch`] 逐条同口径）：`tempo` / `color` 的 `Some(None)`
/// （明写 `null` = 清掉覆盖）与 `None`（缺省 = 保留现值）是两件不同的事。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ScenePatch {
    /// 场景名（`None` = 保留现值）。
    pub name: Option<String>,
    /// 速度覆盖（外层 `Option` = 调用方提没提；内层 = 覆盖值还是"清掉"）。
    pub tempo: Option<Option<f64>>,
    /// 界面色标（外层 `Option` = 调用方提没提；内层 = 覆盖值还是"清掉"）。
    pub color: Option<Option<String>>,
}

impl ScenePatch {
    /// 该覆盖是否一个键都没提（`true` ⇒ 结果必然等于基础，是一次无操作）。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.name.is_none() && self.tempo.is_none() && self.color.is_none()
    }

    /// 把覆盖施加到一条**基础**场景上（基础 = 文档现值，或新建时的空形状）。
    ///
    /// 返回替换后的整条场景。`id` **不在**覆盖范围内（它是寻址，由
    /// [`NoteOp::SetScene`] 的 `scene_id` 给出）：这里原样保留，因此本形态不可能
    /// 顺手改掉一个场景的身份。
    #[must_use]
    pub fn apply_to(&self, base: &SceneV3) -> SceneV3 {
        let mut scene = base.clone();
        if let Some(name) = &self.name {
            scene.name.clone_from(name);
        }
        if let Some(tempo) = self.tempo {
            scene.tempo = tempo;
        }
        if let Some(color) = &self.color {
            scene.color.clone_from(color);
        }
        scene
    }
}

/// 一个音符编辑操作。
#[derive(Clone, Debug, PartialEq)]
pub enum NoteOp {
    /// 插入音符。
    Add {
        /// 完整音符。
        note: Box<MidiNote>,
    },
    /// 删除音符。
    Delete {
        /// 音符身份。
        note_id: EntityId,
    },
    /// 平移音符。
    Move {
        /// 音符身份。
        note_id: EntityId,
        /// tick 增量。
        delta_tick: i64,
        /// 半音增量。
        delta_pitch: i8,
    },
    /// 修改力度。
    Velocity {
        /// 音符身份。
        note_id: EntityId,
        /// 新力度 `0..=127`。
        velocity: u8,
    },
    /// 写**静态混音值**（[`Op::SetParam`]）：音轨音量或声相。
    ///
    /// 这是本枚举里第一个**音轨级**（而非音符级）的形态：它不读、不写任何音符，
    /// 目标音轨由调用方的 `trackId` 给出（见 [`compile`]）。值域判定在模型层
    /// （[`Op::SetParam`] 的 `apply` → `validate_param_value`）。
    SetParam {
        /// 目标（只有"有静态值"的两个变体）。
        lane: StaticLane,
        /// 目标值；`TrackVolume` 单位 dB（有限值），`TrackPan` ∈ -1.0..=1.0。
        value: f32,
    },
    /// 写一个**音轨开关**（[`Op::SetTrackMute`] / [`Op::SetTrackSolo`]）。
    ///
    /// 与 [`Self::SetParam`] 同族（音轨级、目标由顶层 `trackId` 给出），
    /// 但载荷是**布尔**：模型把 `mute` / `solo` 从 `SetParam` 里分出去的理由
    /// （`f32` 写不了 `bool`）在工具面这一侧同样成立。撤销载荷 `old_*` 由
    /// [`TrackFlag::read`] 从**当前文档**读，不是调用方声明。
    SetTrackFlag {
        /// 哪个开关。
        flag: TrackFlag,
        /// 目标态。
        value: bool,
    },
    /// 写一条**自动化泳道自己的属性**（[`Op::SetAutomationLane`] /
    /// [`Op::RemoveAutomationLane`]）。
    ///
    /// 与 [`Self::SetParam`] / [`Self::SetTrackFlag`] 同族（音轨级、不读不写音符），
    /// 但目标不是整条音轨而是一条**泳道**，载荷也不是一个标量。目标与载荷都由
    /// [`LaneEdit`] 承载（自包含的 `lane` 对象）。
    SetLane {
        /// 目标与属性覆盖（或"取走"）。
        edit: LaneEdit,
    },
    /// 取走**一个自动化点**（[`Op::RemoveAutomationPoint`]）。
    ///
    /// 与 [`Self::SetLane`] 同族（音轨级、目标是一条泳道），但改的不是泳道自己的
    /// 属性而是**泳道里的一个点**：写侧只有 [`Op::SetAutomationPoint`]（同一
    /// `(目标, tick)` 是更新）⇒ 没有本形态，工具面写进去的点就**取不走**。
    RemovePoint {
        /// 目标的分量（音轨身份由 [`compile`] 补上）。
        spec: LaneTargetSpec,
        /// 要取走的那一个点（文档上的 `tick` 或点的显式身份，恰好一个）。
        address: PointAddress,
    },
    /// 取走**一个片段池条目**（[`Op::RemoveClip`]）。
    ///
    /// 这是本枚举里唯一的**池级**形态：它不读不写任何音符，也不碰任何摆放，目标就是
    /// 工具**顶层**的 `clipId`（与 [`Self::Add`] 之外的形态不同，它连音轨都不需要）。
    /// 载荷是**空**的：撤销载荷 `previous_clip` 由 [`compile`] 从**当前文档**读。
    ///
    /// 与 [`Self::Add`] 的镜像关系：`create: true` 走的是另一条入口（整批折成一条
    /// `Op::AddClip`），而"把池里那条材料取走"此前**没有任何形态**能表达。
    RemoveClip,
    /// 写一条**路由边**的静态增益（[`Op::SetRoutingGain`]）。
    ///
    /// 这是本枚举里唯一的**路由级**形态：它不读不写任何音符，也不碰音轨与片段，
    /// 目标由**自带的** `edgeId` 给出（顶层 `trackId` / `clipId` 与本形态无关）。
    ///
    /// 载荷是 `Option<f32>` 的**字面**形态（[`Option`] 这一层不可省）：
    /// `None` = **单位增益**，`Some(0.0)` = 0 dB —— 模型明文要求两者可区分，
    /// 因此撤销载荷 `old_gain_db` 由 [`compile`] 从**当前文档**读**原样**的
    /// `Option<f32>`（**不走**会把 `None` 折算成 `0.0` 的静态值入口）。
    SetRoutingGain {
        /// 路由边身份（本形态自带寻址）。
        edge_id: EntityId,
        /// 目标增益（dB）；`None` = 单位增益。
        gain_db: Option<f32>,
    },
    /// 断开**一条路由边**（[`Op::DisconnectRouting`]，即把这条边从
    /// `routing_graph.edges` 取走）。
    ///
    /// 与 [`Self::SetRoutingGain`] **同族**（路由级、目标由自带的 `edgeId` 给出、
    /// 与顶层 `trackId` / `clipId` 无关），但取走的不是边上的一个值而是**边本身**：
    /// 写侧只有 [`Op::ConnectRouting`]（本工具面只在 `yeban_propose_section` 的建批里
    /// 构造它）⇒ 没有本形态，工具面**造得出**的边**取不走**（`yeban_query_project`
    /// 的实体索引与 `routing_graph` 字段却一直在报它们的身份）。
    ///
    /// 载荷是**空**的：撤销载荷 `previous_edge` 由 [`compile`] 从**当前文档**读
    /// （模型的前置条件要求它逐字段等于文档现值，因此本层不采信调用方声明的旧状态）。
    /// 断开一条边**不会**动 `routing_graph.nodes`：模型 `YebanProjectV1::validate`
    /// 只要求主总线出现在 `nodes` 里（那条要求与本形态无关），而
    /// `RemoveRoutingNode` 有"没有任何边引用它"的前置条件 —— 想取走节点必须先断开
    /// 引用它的每一条边，两步各自成一次可审查的调用。
    DisconnectRouting {
        /// 路由边身份（本形态自带寻址）。
        edge_id: EntityId,
    },
    /// 取走**一个路由节点**（[`Op::RemoveRoutingNode`]，即把这个身份从
    /// `routing_graph.nodes` 取走）。
    ///
    /// 与 [`Self::DisconnectRouting`] **同族**（路由级、目标由自带的 `nodeId` 给出、
    /// 与顶层 `trackId` / `clipId` 无关），但取走的不是一条边而是**一个节点**：
    /// 写侧只有 `AddRoutingNode`（本工具面只在 `yeban_propose_section` 的建批里
    /// 构造它）⇒ 没有本形态，工具面**造得出**的节点**取不走**
    /// （`yeban_query_project` 的 `routing_graph.nodes` 数组却一直在报它们的身份，
    /// 而 `disconnectRouting` 的名字说了它不动节点）。
    ///
    /// 载荷是**空**的：只有寻址。`RemoveRoutingNode` 在模型里**没有撤销载荷**
    /// （[`Op::invert`] 把它换成 `AddRoutingNode`，两条载荷都只有节点身份），
    /// 因此本形态既不读文档现值，也不接受调用方送来的旧状态
    /// （[`reject_remove_routing_node_fields`] 只认 `kind` 与 `nodeId`）。
    ///
    /// 模型的两条前置条件中，"没有任何边引用它"由 `Op::validate` 报
    /// （`RoutingNodeInUse`，带 `edge_count`）；"节点存在"由 [`compile`] 提前报
    /// （`routingNodeNotFound`），"这个节点是主总线"同样由 [`compile`] 提前报
    /// （`masterBusNodeCannotBeRemoved`）—— 那两条在模型里分别是
    /// `RoutingNodeNotFound` 与 `YebanProjectV1::validate` 的主总线不变量。
    RemoveRoutingNode {
        /// 路由节点身份（音轨或总线；本形态自带寻址）。
        node_id: EntityId,
    },
    /// 取走**一个曲式段落**（[`Op::RemoveSection`]，即把这个身份从
    /// `project.sections` 取走）。
    ///
    /// 这是本枚举里唯一的**段落级**形态：它不读不写任何音符，也不碰音轨、片段池与
    /// 路由图，目标由**自带的** `sectionId` 给出（顶层 `trackId` / `clipId` 与本形态
    /// 无关）。
    ///
    /// 写侧只有 [`Op::SetSection`]（本工具面只在 `yeban_propose_section` 的建批里
    /// 构造它）⇒ 没有本形态，工具面**造得出**的段落**取不走**
    /// （`yeban_query_project` 的 `entities[]` 里 `kind == "section"` 的条目却一直在
    /// 报它们的身份）。`RemoveSection` 与 `SetSection { old_section: None }` 互为逆
    /// 操作（见 `docs/adr/ADR-0001` 的 D12），因此它是"新建段落"的**唯一**逆操作。
    ///
    /// 载荷是**空**的：只有寻址。撤销载荷 `previous_section` 由 [`compile`] 从
    /// **当前文档**读（模型的前置条件要求它逐字段等于文档现值，因此本层不采信
    /// 调用方声明的旧状态），[`reject_remove_section_fields`] 只认 `kind` 与
    /// `sectionId`。
    RemoveSection {
        /// 曲式段落身份（本形态自带寻址）。
        section_id: EntityId,
    },
    /// 取走**一个场景**（[`Op::RemoveScene`]，即把这个身份从 `project.scenes` 取走）。
    ///
    /// 这是本枚举里唯一的**场景级**形态：它不读不写任何音符，也不碰音轨、片段池、
    /// 路由图与曲式段落，目标由**自带的** `sceneId` 给出（顶层 `trackId` / `clipId`
    /// 与本形态无关）。
    ///
    /// ⚠ 写侧在本形态之前**一处都没有**：`Op::SetScene` 与 `Op::RemoveScene` 在
    /// `crates/yeban-mcp/src` 里的构造点实测都是 0（口径与读数见模块头"场景取走形态"
    /// 一节）⇒ 工具面**既建不出**场景、也取不走场景，而
    /// `yeban_query_project` 的 `entities[]` 一直把 `kind == "scene"` 的身份报给客户端。
    /// 本形态关闭的是"看得见、取不走"的那一半。`RemoveScene` 与
    /// `SetScene { old_scene: None }` 互为逆操作（见 `docs/adr/ADR-0001` 的 D12），
    /// 语义由那一步的逆定义，**没有自由度**。
    ///
    /// 载荷是**空**的：只有寻址。撤销载荷 `previous_scene` 由 [`compile`] 从
    /// **当前文档**读（模型的前置条件要求它逐字段等于文档现值，因此本层不采信
    /// 调用方声明的旧状态），[`reject_remove_scene_fields`] 只认 `kind` 与
    /// `sceneId`。
    RemoveScene {
        /// 场景身份（本形态自带寻址）。
        scene_id: EntityId,
    },
    /// **写入一个场景**（[`Op::SetScene`]）：新建（`create` 为真）或更新（合并）。
    ///
    /// 与 [`Self::RemoveScene`] **同族**（场景级、目标由自带的 `sceneId` 给出、
    /// 与顶层 `trackId` / `clipId` 无关），但动作相反：它是"建/改"的那一半。
    /// ⚠ 在它之前，工具面**既建不出**场景、也取不走场景；上一个形态关掉了"取走"
    /// 那一半，本形态关掉"建/改"那一半 —— 于是 [`Op::SetScene`] 与
    /// [`Op::RemoveScene`] 这一对互逆操作在工具面上都可达。
    ///
    /// 载荷（除寻址外）是一个 [`ScenePatch`]：三个属性键**各自可选**，缺省 = 保留
    /// 文档现值（是**合并**不是整体替换），显式 `null` = 清空可空的那两个。
    /// 撤销载荷 `old_scene` 由 [`compile`] 从**当前文档**读（新建时 `None`，
    /// 更新时整条现值的克隆）—— 模型的前置条件要求它逐字等于文档现值，因此本层
    /// 不采信调用方声明的旧状态。
    ///
    /// `create` 显式区分新建与更新（缺省 `false` = 更新，身份必须已在
    /// `project.scenes` 里，否则 `ENTITY_NOT_FOUND`）：靠"文档里没有就当作新建"
    /// 会让打错的 `sceneId` 静默建出一个新场景。
    SetScene {
        /// 场景身份（本形态自带寻址；新建与更新都必须给出）。
        scene_id: EntityId,
        /// `true` = 新建（身份必须**不在**文档里，否则 `CONFLICT`）；
        /// `false` = 更新（身份必须**在**文档里，否则 `ENTITY_NOT_FOUND`）。
        create: bool,
        /// 属性覆盖（合并语义）。
        patch: ScenePatch,
    },
}

impl NoteOp {
    /// 该操作在 `arguments.ops[].kind` 里的字面名字（错误信息与判据共用同一份真相）。
    #[must_use]
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::Add { .. } => "add",
            Self::Delete { .. } => "delete",
            Self::Move { .. } => "move",
            Self::Velocity { .. } => "velocity",
            Self::SetParam { .. } => SET_PARAM_KIND,
            Self::SetTrackFlag { flag, .. } => flag.kind_name(),
            Self::SetLane { .. } => SET_AUTOMATION_LANE_KIND,
            Self::RemovePoint { .. } => REMOVE_AUTOMATION_POINT_KIND,
            Self::RemoveClip => REMOVE_CLIP_KIND,
            Self::SetRoutingGain { .. } => SET_ROUTING_GAIN_KIND,
            Self::DisconnectRouting { .. } => DISCONNECT_ROUTING_KIND,
            Self::RemoveRoutingNode { .. } => REMOVE_ROUTING_NODE_KIND,
            Self::RemoveSection { .. } => REMOVE_SECTION_KIND,
            Self::RemoveScene { .. } => REMOVE_SCENE_KIND,
            Self::SetScene { .. } => SET_SCENE_KIND,
        }
    }

    /// 该形态是否**读/写音符**（即是否必须在一条 MIDI 片段上施加）。
    ///
    /// [`Self::SetParam`] / [`Self::SetTrackFlag`] / [`Self::SetLane`] / [`Self::RemovePoint`]
    /// 都是**音轨级**的、[`Self::RemoveClip`] 是**池级**的、
    /// [`Self::SetRoutingGain`] / [`Self::DisconnectRouting`] / [`Self::RemoveRoutingNode`]
    /// 是**路由级**的、[`Self::RemoveSection`] 是**段落级**的、[`Self::RemoveScene`]
    /// 是**场景级**的：它们跟片段内容无关。
    /// 这条区分让 [`compile`] 的"必须是 MIDI 片段"断言只在真的有音符操作时成立
    /// （旧行为逐字节不变：四个音符形态的调用仍然要求 MIDI 材料）。
    #[must_use]
    pub const fn is_note_level(&self) -> bool {
        !matches!(
            self,
            Self::SetParam { .. }
                | Self::SetTrackFlag { .. }
                | Self::SetLane { .. }
                | Self::RemovePoint { .. }
                | Self::RemoveClip
                | Self::SetRoutingGain { .. }
                | Self::DisconnectRouting { .. }
                | Self::RemoveRoutingNode { .. }
                | Self::RemoveSection { .. }
                | Self::RemoveScene { .. }
                | Self::SetScene { .. }
        )
    }

    /// 该形态改的是**路由图**（而不是音符 / 音轨 / 泳道 / 片段池 / 段落）。
    ///
    /// 只用于把提案标题写成**实际内容**（`domain::plan_edit_notes` 的分类）：
    /// 一次纯 `setRoutingGain` / 纯 `disconnectRouting` / 纯 `removeRoutingNode`
    /// 的调用不能被报成"音轨级编辑"（那是不同的对象）。
    #[must_use]
    pub const fn is_routing_level(&self) -> bool {
        matches!(
            self,
            Self::SetRoutingGain { .. }
                | Self::DisconnectRouting { .. }
                | Self::RemoveRoutingNode { .. }
        )
    }

    /// 该形态改的是**曲式段落**（而不是音符 / 音轨 / 泳道 / 片段池 / 路由图）。
    ///
    /// 与 [`Self::is_routing_level`] 同因（`domain::plan_edit_notes` 的分类）：
    /// 段落不是音轨，一次纯 `removeSection` 的调用不能被报成"音轨级编辑"
    /// —— 本形态**不是**音轨级（[`Self::is_note_level`] 为 `false`，
    /// [`Self::is_routing_level`] 也为 `false`），因此必须有自己的桶。
    #[must_use]
    pub const fn is_section_level(&self) -> bool {
        matches!(self, Self::RemoveSection { .. })
    }

    /// 该形态改的是**场景**（而不是音符 / 音轨 / 泳道 / 片段池 / 路由图 / 曲式段落）。
    ///
    /// 与 [`Self::is_section_level`] 同因（`domain::plan_edit_notes` 的分类）：
    /// 场景不是段落，一次纯 `removeScene` / 纯 `setScene` 的调用不能被报成
    /// "段落级编辑" —— 两个形态都**不是**音轨级（[`Self::is_note_level`] 为 `false`），
    /// 也不是路由级、不是段落级，因此共用同一个桶。
    #[must_use]
    pub const fn is_scene_level(&self) -> bool {
        matches!(self, Self::RemoveScene { .. } | Self::SetScene { .. })
    }
}

/// 解析 `arguments.ops`。
///
/// 支持的 `kind`（本地定义，见模块头）：
///
/// ```json
/// {"kind":"add","note":{"id":"<可选 26 字符 ULID>","startTick":0,"pitch":60,
///                       "durationTicks":480,"velocity":100,"probability":0.5,
///                       "ratchet":4,"microTimingTicks":-12}}
/// {"kind":"delete","noteId":"<ULID>"}
/// {"kind":"move","noteId":"<ULID>","deltaTick":960,"deltaPitch":12}
/// {"kind":"velocity","noteId":"<ULID>","velocity":80}
/// {"kind":"setParam","lane":"TrackVolume","value":-6.0}
/// {"kind":"setParam","lane":"TrackPan","value":-0.25}
/// {"kind":"setTrackMute","value":true}
/// {"kind":"setTrackSolo","value":false}
/// {"kind":"setAutomationLane","lane":{"lane":"TrackVolume","readEnabled":false}}
/// {"kind":"setAutomationLane","lane":{"lane":"TrackPan","remove":true}}
/// {"kind":"removeAutomationPoint","point":{"lane":"TrackVolume","tick":3840}}
/// {"kind":"removeAutomationPoint","point":{"lane":"Macro","macroIndex":0,
///                                          "pointId":"<ULID>"}}
/// {"kind":"removeClip"}
/// {"kind":"setRoutingGain","edgeId":"<ULID>","value":-6.0}
/// {"kind":"setRoutingGain","edgeId":"<ULID>","value":null}
/// {"kind":"disconnectRouting","edgeId":"<ULID>"}
/// {"kind":"removeRoutingNode","nodeId":"<ULID>"}
/// {"kind":"removeSection","sectionId":"<ULID>"}
/// {"kind":"removeScene","sceneId":"<ULID>"}
/// {"kind":"setScene","scene":{"create":true,"sceneId":"<ULID>","name":"Verse",
///                             "tempo":128.0,"color":"#22AA88"}}
/// {"kind":"setScene","scene":{"sceneId":"<ULID>","name":"Verse 2","tempo":null}}
/// ```
///
/// `note.probability` / `note.ratchet` / `note.microTimingTicks` 是**可选**字段
/// （缺省逐字节等于旧行为）：给了就是 [`MidiNote`] 对应字段的字面值，语义与判定入口
/// 见 [`PROBABILITY_FIELD`] / [`RATCHET_FIELD`] / [`MICRO_TIMING_FIELD`]。
/// `note` 里 [`NOTE_FIELDS`] 之外的键一律**响亮拒绝**，不静默丢弃。
///
/// `setParam` / `setTrackMute` / `setTrackSolo` / `setAutomationLane` /
/// `removeAutomationPoint` 是**音轨级**形态
/// （见 [`NoteOp::is_note_level`]）：`setParam` 的 `lane` 只认 [`StaticLane::NAMES`]，
/// 其余三个自动化目标名（`SendGain` / `DeviceParam` / `Macro`）与未知名字都是
/// **响亮失败**（`INVALID_PARAMETER_RANGE`，`data.allowed` 给出全集）。
/// 值的范围判定**不在本层**（见 [`compile`]）；两个开关形态的 `value` 只收 JSON 布尔。
/// `setAutomationLane` 的**载荷**是 `lane` 对象（见 [`parse_lane_edit`]），
/// 三个属性键各自可选（缺省 = 保留文档现值）；`removeAutomationPoint` 的**载荷**是
/// `point` 对象（见 [`parse_point_removal`]），`tick` 与 `pointId` 恰好给一个。
///
/// `removeClip` 是唯一的**池级**形态（见 [`NoteOp::RemoveClip`]）：载荷是**空**的，
/// 目标片段是工具顶层的 `clipId`；对象里 [`REMOVE_CLIP_FIELDS`] 之外的键一律响亮拒绝。
///
/// `setRoutingGain` 是**写路由边上一个值**的路由级形态（见 [`NoteOp::SetRoutingGain`]）：
/// 目标由对象里**自带的** `edgeId` 给出（顶层 `trackId` / `clipId` 都与它无关），
/// `value` 是 `Option<f32>` 的字面形态（`null` = 单位增益 = 模型的 `None`），
/// 对象里 [`SET_ROUTING_GAIN_FIELDS`] 之外的键一律响亮拒绝。
///
/// `setRoutingGain` 与 `disconnectRouting` 是**两个**路由级形态
/// （见 [`NoteOp::is_routing_level`]）：前者写边上的一个值，后者把边**本身**取走
/// （见 [`NoteOp::DisconnectRouting`]）。两者都用 [`ROUTING_EDGE_FIELD`] 寻址，
/// 因此 `disconnectRouting` 对象里 [`DISCONNECT_ROUTING_FIELDS`] 之外的键一律响亮拒绝。
///
/// `removeRoutingNode` 是**第三个**路由级形态
/// （见 [`NoteOp::RemoveRoutingNode`]）：它把 `nodeId` 那个**节点**从
/// `routing_graph.nodes` 取走，用 [`ROUTING_NODE_FIELD`] 寻址（与边**不同的**实体），
/// 对象里 [`REMOVE_ROUTING_NODE_FIELDS`] 之外的键一律响亮拒绝。
///
/// `removeSection` 是**唯一的段落级**形态（见 [`NoteOp::RemoveSection`]，
/// [`NoteOp::is_section_level`]）：它把 `sectionId` 那个**曲式段落**从
/// `project.sections` 取走（[`Op::RemoveSection`]），用 [`SECTION_FIELD`] 寻址
/// （与节点 / 边 / 片段都是**不同的**实体），撤销载荷 `previous_section` 从当前文档读，
/// 对象里 [`REMOVE_SECTION_FIELDS`] 之外的键一律响亮拒绝。
///
/// `removeScene` 是**唯一的场景级取走**形态（见 [`NoteOp::RemoveScene`]，
/// [`NoteOp::is_scene_level`]）：它把 `sceneId` 那个**场景**从 `project.scenes` 取走
/// （[`Op::RemoveScene`]），用 [`SCENE_FIELD`] 寻址（与段落 / 节点 / 边 / 片段都是
/// **不同的**实体），撤销载荷 `previous_scene` 从当前文档读，
/// 对象里 [`REMOVE_SCENE_FIELDS`] 之外的键一律响亮拒绝。
///
/// `setScene` 是**场景级写入**形态（见 [`NoteOp::SetScene`]，
/// [`NoteOp::is_scene_level`]）：载荷是自包含的 `scene` 对象（[`SET_SCENE_FIELDS`]），
/// `create: true` 表示新建（身份必须**不在**文档里，且 `name` 必填），缺省表示更新
/// （身份必须**在**文档里，三个属性按**合并**语义施加，显式 `null` 清空可空的那两个），
/// 撤销载荷 `old_scene` 从当前文档读，操作对象里 [`SET_SCENE_OP_FIELDS`] 之外的键
/// 一律响亮拒绝。
///
/// # Errors
///
/// - `ops` 不是数组 / 元素不是对象 / 缺字段 / 字段类型不对 / `note` 里有未知键 /
///   开关对象里有 [`TRACK_FLAG_FIELDS`] 之外的键 / `lane` 对象里有
///   [`SET_AUTOMATION_LANE_FIELDS`] 之外的键 / 路由边增益对象里有
///   [`SET_ROUTING_GAIN_FIELDS`] 之外的键 / 断开路由边对象里有
///   [`DISCONNECT_ROUTING_FIELDS`] 之外的键 / 取走路由节点对象里有
///   [`REMOVE_ROUTING_NODE_FIELDS`] 之外的键 / 取走段落对象里有
///   [`REMOVE_SECTION_FIELDS`] 之外的键 / 取走场景对象里有
///   [`REMOVE_SCENE_FIELDS`] 之外的键 / 写入场景的操作对象里有
///   [`SET_SCENE_OP_FIELDS`] 之外的键 / 写入场景的 `scene` 对象里有
///   [`SET_SCENE_FIELDS`] 之外的键 / `name` 或 `color` 是空串 / `create: true` 而没给
///   `name` →
///   `INVALID_PARAMETER_RANGE`（含未知 `kind`、未知 `lane`、不可写 `lane`、
///   非布尔开关值、未知写模式、既不是数字也不是 `null` 的增益值或速度）；
/// - 音高、力度、时值、概率、连击、微时序越界 → `OUT_OF_RANGE`；
/// - 身份文本不是合法 ULID → `INVALID_PARAMETER_RANGE`。
pub fn parse_ops(value: &Value) -> Result<Vec<NoteOp>, Fault> {
    let Value::Array(items) = value else {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`ops` 必须是数组",
        ));
    };
    if items.is_empty() {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`ops` 不得为空数组（空操作不是一次编辑请求）",
        ));
    }
    items.iter().map(parse_one).collect()
}

/// 解析单个操作。
fn parse_one(item: &Value) -> Result<NoteOp, Fault> {
    let object = item.as_object().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`ops` 的元素必须是对象, 实际收到 {item}"),
        )
    })?;
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| missing("kind", "字符串"))?;
    match kind {
        "add" => {
            let note = object
                .get("note")
                .and_then(Value::as_object)
                .ok_or_else(|| missing("note", "对象"))?;
            Ok(NoteOp::Add {
                note: Box::new(parse_note(note)?),
            })
        }
        "delete" => Ok(NoteOp::Delete {
            note_id: read_id(object, "noteId")?,
        }),
        "move" => Ok(NoteOp::Move {
            note_id: read_id(object, "noteId")?,
            delta_tick: read_i64(object, "deltaTick")?,
            delta_pitch: read_i8(object, "deltaPitch")?,
        }),
        "velocity" => Ok(NoteOp::Velocity {
            note_id: read_id(object, "noteId")?,
            velocity: read_range(object, "velocity", 0, 127)?,
        }),
        SET_PARAM_KIND => Ok(NoteOp::SetParam {
            lane: parse_static_lane(object)?,
            value: read_number(object, SET_PARAM_VALUE_FIELD)?,
        }),
        SET_TRACK_MUTE_KIND => Ok(NoteOp::SetTrackFlag {
            flag: TrackFlag::Mute,
            value: parse_track_flag_value(object)?,
        }),
        SET_TRACK_SOLO_KIND => Ok(NoteOp::SetTrackFlag {
            flag: TrackFlag::Solo,
            value: parse_track_flag_value(object)?,
        }),
        SET_AUTOMATION_LANE_KIND => Ok(NoteOp::SetLane {
            edit: parse_lane_edit(object)?,
        }),
        REMOVE_AUTOMATION_POINT_KIND => {
            let (spec, address) = parse_point_removal(object)?;
            Ok(NoteOp::RemovePoint { spec, address })
        }
        REMOVE_CLIP_KIND => {
            reject_remove_clip_fields(object)?;
            Ok(NoteOp::RemoveClip)
        }
        SET_ROUTING_GAIN_KIND => {
            reject_routing_gain_fields(object)?;
            Ok(NoteOp::SetRoutingGain {
                edge_id: read_id(object, SET_ROUTING_GAIN_EDGE_FIELD)?,
                gain_db: read_routing_gain(object)?,
            })
        }
        DISCONNECT_ROUTING_KIND => {
            reject_disconnect_routing_fields(object)?;
            Ok(NoteOp::DisconnectRouting {
                edge_id: read_id(object, ROUTING_EDGE_FIELD)?,
            })
        }
        REMOVE_ROUTING_NODE_KIND => {
            reject_remove_routing_node_fields(object)?;
            Ok(NoteOp::RemoveRoutingNode {
                node_id: read_id(object, ROUTING_NODE_FIELD)?,
            })
        }
        REMOVE_SECTION_KIND => {
            reject_remove_section_fields(object)?;
            Ok(NoteOp::RemoveSection {
                section_id: read_id(object, SECTION_FIELD)?,
            })
        }
        REMOVE_SCENE_KIND => {
            reject_remove_scene_fields(object)?;
            Ok(NoteOp::RemoveScene {
                scene_id: read_id(object, SCENE_FIELD)?,
            })
        }
        SET_SCENE_KIND => {
            reject_set_scene_op_fields(object)?;
            let payload = object
                .get(SCENE_PAYLOAD_FIELD)
                .and_then(Value::as_object)
                .ok_or_else(|| missing(SCENE_PAYLOAD_FIELD, "对象"))?;
            reject_set_scene_fields(payload)?;
            let scene_id = read_id(payload, SCENE_FIELD)?;
            let create = read_optional_scene_bool(payload, SCENE_CREATE_FIELD)?.unwrap_or(false);
            let patch = parse_scene_patch(payload)?;
            if create && patch.name.is_none() {
                return Err(Fault::domain_with_data(
                    ErrorCode::InvalidParameterRange,
                    format!(
                        "`{SET_SCENE_KIND}` 且 `{SCENE_CREATE_FIELD}: true` 时必须给出 \
                         `{SCENE_NAME_FIELD}`: 模型的 `SceneV3::name` 没有校验, \
                         空名会静默落盘"
                    ),
                    serde_json::json!({
                        "field": format!("{SCENE_PAYLOAD_FIELD}.{SCENE_NAME_FIELD}"),
                        "reason": "sceneNameRequiredWhenCreating",
                        "hint": "新建场景必须有名 (`name` 是非空字符串); \
                                 只想改名字/速度/色标就不要给 `create: true`",
                    }),
                ));
            }
            Ok(NoteOp::SetScene {
                scene_id,
                create,
                patch,
            })
        }
        other => Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("未知 `kind`: `{other}`"),
            serde_json::json!({ "supportedKinds": OP_KINDS }),
        )),
    }
}

/// 拒绝 `setRoutingGain` 操作对象里 [`SET_ROUTING_GAIN_FIELDS`] 之外的键。
///
/// 与 [`reject_remove_clip_fields`] / [`reject_track_flag_fields`] 同一口径
/// （"拼错的键必须被拒绝, 不能静默忽略"）：最像"写对了"的错法是把目标写成
/// 工具顶层的 `trackId` 或把增益写成 `gainDb` —— 两者都会被静默忽略，
/// 而调用方以为发送增益已经改了。
///
/// # Errors
///
/// 出现 `kind` / `edgeId` / `value` 之外的键 → `INVALID_PARAMETER_RANGE`
/// （`data.reason = "unknownRoutingGainField"`）。
fn reject_routing_gain_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !SET_ROUTING_GAIN_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{SET_ROUTING_GAIN_KIND}` 操作里有不支持的键: {} \
             (支持集合只有 {SET_ROUTING_GAIN_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownRoutingGainField",
            "unsupportedFields": unknown,
            "supportedRoutingGainFields": SET_ROUTING_GAIN_FIELDS,
            "hint": "增益字段是 `value` (不是 `gainDb`); 路由边由操作对象自带的 \
                     `edgeId` 寻址, 嵌套的 `trackId` 不会被读取",
        }),
    ))
}

/// 拒绝 `disconnectRouting` 操作对象里 [`DISCONNECT_ROUTING_FIELDS`] 之外的键。
///
/// 与 [`reject_routing_gain_fields`] / [`reject_remove_clip_fields`] 同一口径
/// （"拼错的键必须被拒绝, 不能静默忽略"）：最像"写对了"的错法是**把上一条形态的
/// 键搬过来** —— 把目标写成工具顶层的 `trackId`，或以为要报告"断开前的值"而多写
/// `value` / `gainDb`。三种都会被静默忽略，而调用方以为边已经断了。
///
/// # Errors
///
/// 出现 `kind` / `edgeId` 之外的键 → `INVALID_PARAMETER_RANGE`
/// （`data.reason = "unknownDisconnectRoutingField"`）。
fn reject_disconnect_routing_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !DISCONNECT_ROUTING_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{DISCONNECT_ROUTING_KIND}` 操作里有不支持的键: {} \
             (支持集合只有 {DISCONNECT_ROUTING_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownDisconnectRoutingField",
            "unsupportedFields": unknown,
            "supportedDisconnectRoutingFields": DISCONNECT_ROUTING_FIELDS,
            "hint": "本形态的载荷是空的 (只认 `kind` 与 `edgeId`); 断开前的整条边由 \
                     `compile` 从当前文档读, 不需要 (也不接受) 调用方声明",
        }),
    ))
}

/// 拒绝 `removeRoutingNode` 操作对象里 [`REMOVE_ROUTING_NODE_FIELDS`] 之外的键。
///
/// 与 [`reject_disconnect_routing_fields`] / [`reject_routing_gain_fields`] 同一口径
/// （"拼错的键必须被拒绝, 不能静默忽略"）：最像"写对了"的两种错法是把**边的**寻址
/// （`edgeId`）搬过来，或把目标写成工具顶层的 `trackId` —— 两种都会被静默忽略，
/// 而调用方以为节点已经取走。
///
/// # Errors
///
/// 出现 `kind` / `nodeId` 之外的键 → `INVALID_PARAMETER_RANGE`
/// （`data.reason = "unknownRemoveRoutingNodeField"`）。
fn reject_remove_routing_node_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !REMOVE_ROUTING_NODE_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{REMOVE_ROUTING_NODE_KIND}` 操作里有不支持的键: {} \
             (支持集合只有 {REMOVE_ROUTING_NODE_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownRemoveRoutingNodeField",
            "unsupportedFields": unknown,
            "supportedRemoveRoutingNodeFields": REMOVE_ROUTING_NODE_FIELDS,
            "hint": "本形态的载荷是空的 (只认 `kind` 与 `nodeId`); 节点身份取自 \
                     `yeban_query_project` 的 `routing_graph.nodes`, 边由 `edgeId` 寻址 \
                     (那个键属于 `setRoutingGain` / `disconnectRouting`)",
        }),
    ))
}

/// 拒绝 `removeSection` 操作对象里 [`REMOVE_SECTION_FIELDS`] 之外的键。
///
/// 与 [`reject_remove_routing_node_fields`] / [`reject_disconnect_routing_fields`]
/// 同一口径（"拼错的键必须被拒绝, 不能静默忽略"）：最像"写对了"的三种错法是把
/// **别的实体的**寻址搬过来（`nodeId` / `edgeId` / 嵌套的 `clipId`）、把目标写成
/// 工具顶层的 `trackId`、或以为要报告"段落取走前的状态"而多写 `previousSection`
/// —— 三种都会被静默忽略，而调用方以为段落已经取走。
///
/// # Errors
///
/// 出现 `kind` / `sectionId` 之外的键 → `INVALID_PARAMETER_RANGE`
/// （`data.reason = "unknownRemoveSectionField"`）。
fn reject_remove_section_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !REMOVE_SECTION_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{REMOVE_SECTION_KIND}` 操作里有不支持的键: {} \
             (支持集合只有 {REMOVE_SECTION_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownRemoveSectionField",
            "unsupportedFields": unknown,
            "supportedRemoveSectionFields": REMOVE_SECTION_FIELDS,
            "hint": "本形态的载荷是空的 (只认 `kind` 与 `sectionId`); 段落身份取自 \
                     `yeban_query_project` 的 `entities[]` 里 `kind == \"section\"` 的条目, \
                     撤销载荷 `previousSection` 由服务端从当前文档读 (不接受调用方声明)",
        }),
    ))
}

/// 拒绝 `removeScene` 操作对象里 [`REMOVE_SCENE_FIELDS`] 之外的键。
///
/// 与 [`reject_remove_section_fields`] / [`reject_remove_routing_node_fields`]
/// 同一口径（"拼错的键必须被拒绝, 不能静默忽略"）：最像"写对了"的三种错法是把
/// **别的实体的**寻址搬过来（`sectionId` / `nodeId` / `edgeId` / 嵌套的 `clipId`）、
/// 把目标写成工具顶层的 `trackId`、或以为要报告"场景取走前的状态"而多写
/// `previousScene` —— 三种都会被静默忽略，而调用方以为场景已经取走。
///
/// # Errors
///
/// 出现 `kind` / `sceneId` 之外的键 → `INVALID_PARAMETER_RANGE`
/// （`data.reason = "unknownRemoveSceneField"`）。
fn reject_remove_scene_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !REMOVE_SCENE_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{REMOVE_SCENE_KIND}` 操作里有不支持的键: {} \
             (支持集合只有 {REMOVE_SCENE_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownRemoveSceneField",
            "unsupportedFields": unknown,
            "supportedRemoveSceneFields": REMOVE_SCENE_FIELDS,
            "hint": "本形态的载荷是空的 (只认 `kind` 与 `sceneId`); 场景身份取自 \
                     `yeban_query_project` 的 `entities[]` 里 `kind == \"scene\"` 的条目, \
                     撤销载荷 `previousScene` 由服务端从当前文档读 (不接受调用方声明)",
        }),
    ))
}

/// 拒绝 `setScene` **操作对象**里 [`SET_SCENE_OP_FIELDS`] 之外的键。
///
/// 与 [`reject_point_removal_op_fields`] 同一口径：寻址与属性都在载荷对象里，
/// 因此操作对象顶层只认 `kind` 与 `scene`。最像"写对了"的错法是把 `sceneId`
/// 直接放在顶层（那是 `removeScene` 的形状）—— 那会被静默忽略，
/// 而调用方以为场景已经改好了。
///
/// # Errors
///
/// 出现 `kind` / `scene` 之外的键 → `INVALID_PARAMETER_RANGE`
/// （`data.reason = "unknownSetSceneOpField"`）。
fn reject_set_scene_op_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !SET_SCENE_OP_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{SET_SCENE_KIND}` 操作里有不支持的键: {} \
             (支持集合只有 {SET_SCENE_OP_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownSetSceneOpField",
            "unsupportedFields": unknown,
            "supportedSetSceneOpFields": SET_SCENE_OP_FIELDS,
            "hint": "寻址与属性都在 `scene` 对象里 (与 `removeScene` 不同: 那个把 \
                     `sceneId` 放在操作对象顶层)",
        }),
    ))
}

/// 拒绝 `setScene` 的 `scene` **载荷对象**里 [`SET_SCENE_FIELDS`] 之外的键。
///
/// 与 [`reject_remove_scene_fields`] / [`reject_track_flag_fields`] 同一口径
/// （"拼错的键必须被拒绝, 不能静默忽略"）：最像"写对了"的错法是把别的实体的
/// 寻址（`sectionId` / `nodeId` / `edgeId`）或工具顶层的 `trackId` 搬进来 ——
/// 全都会被静默忽略。场景的**旧状态**（`oldScene` / `previousScene`）同样不在
/// 支持集合里：撤销载荷由 [`compile`] 从当前文档读。
///
/// # Errors
///
/// 出现 [`SET_SCENE_FIELDS`] 之外的键 → `INVALID_PARAMETER_RANGE`
/// （`data.reason = "unknownSetSceneField"`）。
fn reject_set_scene_fields(payload: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = payload
        .keys()
        .map(String::as_str)
        .filter(|key| !SET_SCENE_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{SET_SCENE_KIND}` 的 `{SCENE_PAYLOAD_FIELD}` 对象里有不支持的键: {} \
             (支持集合只有 {SET_SCENE_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownSetSceneField",
            "unsupportedFields": unknown,
            "supportedSetSceneFields": SET_SCENE_FIELDS,
            "hint": "场景身份取自 `yeban_query_project` 的 `entities[]` 里 \
                     `kind == \"scene\"` 的条目; 撤销载荷 `oldScene` 由服务端从当前文档读 \
                     (不接受调用方声明)",
        }),
    ))
}

/// 解析 `setScene` 的 `scene` 对象里的三个**属性**键（寻址与 `create` 由调用处读）。
///
/// 三态**不折叠**（与 [`parse_lane_patch`] 逐条同口径）：键**缺省** = 保留文档现值；
/// 显式 `null` = 清空（只对两个可空字段 `tempo` / `color` 合法）；给了值 = 覆盖。
/// `name` 在模型里**不是**可空字段（普通 `String`），因此 `null` 是响亮失败而不是"清空"。
///
/// # Errors
///
/// - `name` / `color` 出现但不是**非空**字符串 → `INVALID_PARAMETER_RANGE`
///   （`reason = "sceneNameMustBeNonEmptyString"` / `"sceneColorMustBeNonEmptyString"`）；
/// - `tempo` 出现但不是数字也不是 `null` → `INVALID_PARAMETER_RANGE`
///   （`reason = "sceneTempoMustBeNumberOrNull"`）。
fn parse_scene_patch(payload: &Map<String, Value>) -> Result<ScenePatch, Fault> {
    let name = match payload.get(SCENE_NAME_FIELD) {
        None => None,
        Some(value) => Some(read_non_empty_string(
            value,
            SCENE_NAME_FIELD,
            "sceneNameMustBeNonEmptyString",
        )?),
    };
    // `null` = 清空覆盖（跟随工程速度）; 数字 = `Some(f64)`。
    // 值域 (20.0..=999.0) 与有限性**不在这里**判: 模型 `SceneV3::validate` 是
    // 唯一事实源, 它的 `NonFiniteValue` / `BpmOutOfRange` 在提案模拟那一步就报。
    let tempo = match payload.get(SCENE_TEMPO_FIELD) {
        None => None,
        Some(Value::Null) => Some(None),
        Some(value) => Some(Some(value.as_f64().ok_or_else(|| {
            Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!(
                    "`{SCENE_PAYLOAD_FIELD}.{SCENE_TEMPO_FIELD}` 必须是数字或 null, \
                     实际收到 {value}"
                ),
                serde_json::json!({
                    "field": format!("{SCENE_PAYLOAD_FIELD}.{SCENE_TEMPO_FIELD}"),
                    "reason": "sceneTempoMustBeNumberOrNull",
                    "received": value,
                }),
            )
        })?)),
    };
    let color = match payload.get(SCENE_COLOR_FIELD) {
        None => None,
        Some(Value::Null) => Some(None),
        Some(value) => Some(Some(read_non_empty_string(
            value,
            SCENE_COLOR_FIELD,
            "sceneColorMustBeNonEmptyString",
        )?)),
    };
    Ok(ScenePatch { name, tempo, color })
}

/// 读一个**必须是非空字符串**的文本键。
///
/// `name` / `color` 都是给人看的标签：模型的 `SceneV3` 对它们**没有任何校验**，
/// 空串因此会一路静默落盘成一个看不见名字（或没有颜色）的场景。本层在解析期
/// 就把它拦下 —— "响亮失败，绝不静默降级"。
///
/// # Errors
///
/// 不是字符串 → `INVALID_PARAMETER_RANGE`（`data.reason = "{reason}MustBeString"`）；
/// 是空串 → `INVALID_PARAMETER_RANGE`（`data.reason = reason`）。
fn read_non_empty_string(value: &Value, field: &str, reason: &str) -> Result<String, Fault> {
    let text = value.as_str().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{SCENE_PAYLOAD_FIELD}.{field}` 必须是字符串, 实际收到 {value} \
                 (模型的 `SceneV3` 对文本字段没有校验, 因此本层不接受 null 或别的类型)"
            ),
            serde_json::json!({
                "field": format!("{SCENE_PAYLOAD_FIELD}.{field}"),
                "reason": format!("{reason}MustBeString"),
                "received": value,
            }),
        )
    })?;
    if text.is_empty() {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{SCENE_PAYLOAD_FIELD}.{field}` 不得为空串"),
            serde_json::json!({
                "field": format!("{SCENE_PAYLOAD_FIELD}.{field}"),
                "reason": reason,
            }),
        ));
    }
    Ok(text.to_owned())
}

/// 读一个**可选**布尔键（`None` = 缺省；给出 `null` / 数字 / 字符串一律响亮失败）。
///
/// 与 [`read_optional_lane_bool`] 同口径：不做真假值强转。
fn read_optional_scene_bool(
    payload: &Map<String, Value>,
    field: &str,
) -> Result<Option<bool>, Fault> {
    match payload.get(field) {
        None => Ok(None),
        Some(value) => value.as_bool().map(Some).ok_or_else(|| {
            Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!("`{SCENE_PAYLOAD_FIELD}.{field}` 必须是布尔, 实际收到 {value}"),
                serde_json::json!({
                    "field": format!("{SCENE_PAYLOAD_FIELD}.{field}"),
                    "reason": "valueMustBeBoolean",
                    "received": value,
                }),
            )
        }),
    }
}

/// 读 `setRoutingGain` 的目标增益（`ops[].value`，数字**或** `null`）。
///
/// 三态**不折叠**：JSON `null` 就是模型的 `None`（单位增益），数字才是 `Some(f32)`。
/// 缺字段、布尔、字符串、对象都是**响亮失败**（[`read_number`] 的 `valueMustBeNumber`
/// 只在"给了但不是数字"时报，缺字段走统一的 [`missing`]）。
///
/// 收窄与有限性判定**复用一个入口**（[`read_number`]）：`f64 → f32` 的舍入是模型
/// 载荷类型本身要求的，而"收窄之后不是有限数"（例如 `1e39`）必须**在收窄之后**判 ——
/// 否则一个 JSON 里有限、`f32` 里是 `inf` 的值会一路走到模型层才被拒
/// （与 `setParam` 那一票修正过的口径逐条相同）。
///
/// # Errors
///
/// - 缺 `value` → 统一的缺字段错误；
/// - `value` 既不是数字也不是 `null` → `INVALID_PARAMETER_RANGE`
///   （`reason = "valueMustBeNumber"`）；
/// - `value` 收窄到 `f32` 后不是有限数 → `INVALID_PARAMETER_RANGE`
///   （`reason = "nonFiniteValue"`）。
fn read_routing_gain(object: &Map<String, Value>) -> Result<Option<f32>, Fault> {
    let raw = object
        .get(SET_ROUTING_GAIN_VALUE_FIELD)
        .ok_or_else(|| missing(SET_ROUTING_GAIN_VALUE_FIELD, "数字或 null"))?;
    if raw.is_null() {
        return Ok(None);
    }
    read_number(object, SET_ROUTING_GAIN_VALUE_FIELD).map(Some)
}

/// 解析 `setParam` 的 `lane`（只认 [`StaticLane::NAMES`]）。
///
/// 词汇表与 `yeban_edit_automation` **同一份**（[`LaneKind`]）：已知但不可写的三个目标是
/// **响亮失败**，未知名字（别名 / 拼错）是另一条响亮失败，两条都不静默回退到默认值。
///
/// | 情形 | `data.reason` |
/// | :--- | :--- |
/// | `lane` 不是字符串 | `laneMustBeString` |
/// | 名字是另外三个自动化目标（`SendGain` / `DeviceParam` / `Macro`） | `staticLaneNotApplicable` |
/// | 名字不在 [`LaneKind`] 的词汇表里（别名 / 拼错） | `unknownStaticLane` |
/// | `lane` 缺失 | 统一的缺字段错误（无 `data`） |
fn parse_static_lane(object: &Map<String, Value>) -> Result<StaticLane, Fault> {
    let raw = object
        .get(SET_PARAM_LANE_FIELD)
        .ok_or_else(|| missing(SET_PARAM_LANE_FIELD, "字符串"))?;
    let text = raw.as_str().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{SET_PARAM_LANE_FIELD}` 必须是字符串, 实际收到 {raw}"),
            serde_json::json!({
                "field": SET_PARAM_LANE_FIELD,
                "reason": "laneMustBeString",
                "allowed": StaticLane::NAMES,
            }),
        )
    })?;
    let Some(kind) = LaneKind::parse(text) else {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("未知静态目标 `{text}`"),
            serde_json::json!({
                "field": SET_PARAM_LANE_FIELD,
                "reason": "unknownStaticLane",
                "received": text,
                "allowed": StaticLane::NAMES,
                "note": "只接受 project.json 的规范变体名 (不接受 trackVolume 这类别名)",
            }),
        ));
    };
    // 另外三个目标是**已知但此形态不适用**：说清楚它们各自为什么不适用，
    // 而不是笼统地报"未知名字"。
    match kind {
        LaneKind::TrackVolume => Ok(StaticLane::TrackVolume),
        LaneKind::TrackPan => Ok(StaticLane::TrackPan),
        LaneKind::SendGain => Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "`setParam` 不接受目标 `SendGain`: `SendGain` 的取值是 `Option<f32>` \
             (`None` = 单位增益), `Op::SetParam` 明文拒绝它; 发送增益必须走 `Op::SetRoutingGain`",
            serde_json::json!({
                "field": SET_PARAM_LANE_FIELD,
                "reason": "staticLaneNotApplicable",
                "received": kind.as_str(),
                "allowed": StaticLane::NAMES,
            }),
        )),
        LaneKind::DeviceParam | LaneKind::Macro => Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{SET_PARAM_KIND}` 不接受目标 `{}`: 本形态只写**音轨**的静态音量/声相; \
                 设备参数与宏的静态写入在工具面没有通路",
                kind.as_str()
            ),
            serde_json::json!({
                "field": SET_PARAM_LANE_FIELD,
                "reason": "staticLaneNotApplicable",
                "received": kind.as_str(),
                "allowed": StaticLane::NAMES,
            }),
        )),
    }
}

/// 读一个**音轨开关**的目标态（`ops[].value`，只收 JSON 布尔）。
///
/// 先把开关对象上**不该出现**的键拒掉（[`reject_track_flag_fields`]），再读值：
/// 顺序是刻意的 —— 嵌套的 `trackId` 是最危险的错键（写错音轨却静默成功），
/// 它必须在任何"值看起来没问题"的路径之前就被点名。
///
/// `1` / `0` / `"true"` / `null` 一律**响亮失败**，不做真假值强转：模型 `TrackV3::mute`
/// 是 `bool`，一次"猜调用方意思"的强转就是第二份语义。
fn parse_track_flag_value(object: &Map<String, Value>) -> Result<bool, Fault> {
    reject_track_flag_fields(object)?;
    let raw = object
        .get(TRACK_FLAG_VALUE_FIELD)
        .ok_or_else(|| missing(TRACK_FLAG_VALUE_FIELD, "布尔"))?;
    raw.as_bool().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{TRACK_FLAG_VALUE_FIELD}` 必须是布尔, 实际收到 {raw} \
                 (开关不做真假值强转: `1` / `\"true\"` 都不是布尔)"
            ),
            serde_json::json!({
                "field": TRACK_FLAG_VALUE_FIELD,
                "reason": "valueMustBeBoolean",
                "received": raw,
            }),
        )
    })
}

/// 拒绝开关对象里 [`TRACK_FLAG_FIELDS`] 之外的键。
///
/// 与 [`reject_unknown_note_fields`] 同一口径（"拼错的键必须被拒绝, 不能静默忽略"），
/// 只是对象更小。`data.hint` 明确写出目标音轨的正确位置（顶层 `trackId`）——
/// 最常见的错法是把它嵌套进操作对象，那一写会被静默忽略、开关落到**别的**音轨上。
fn reject_track_flag_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !TRACK_FLAG_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "音轨开关操作里有不支持的键: {} (支持集合只有 {TRACK_FLAG_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownFlagField",
            "unsupportedFields": unknown,
            "supportedFlagFields": TRACK_FLAG_FIELDS,
            "hint": "目标音轨是工具顶层的 `trackId`; 嵌套在操作对象里的 `trackId` 不会被读取",
        }),
    ))
}

/// 解析 `setAutomationLane` 的载荷对象（`ops[].lane`）。
///
/// 顺序是刻意的：先拒绝未知键（`unknownLaneField`），再判"取走"与属性互斥
/// （`removeTakesNoProperties`），最后才解析。这样最危险的错键（把 `lane`
/// 写成 `target`、把 `readEnabled` 写成 `read_enabled`）在任何"值看起来没问题"
/// 的路径之前就被点名。
///
/// # Errors
///
/// - `lane` 不是对象 / 缺 `lane.lane` / 未知目标名 / 缺 `edgeId`
///   （`SendGain`） → `INVALID_PARAMETER_RANGE`；
/// - `lane` 里有 [`SET_AUTOMATION_LANE_FIELDS`] 之外的键 → `unknownLaneField`；
/// - `remove: true` 与属性键同给 → `removeTakesNoProperties`；
/// - `readEnabled` 不是布尔 / `writeMode` 不是规范名 / `domain` 形状非法 →
///   各自的 `reason`，绝不静默回退到默认值。
fn parse_lane_edit(object: &Map<String, Value>) -> Result<LaneEdit, Fault> {
    let lane = object
        .get(SET_AUTOMATION_LANE_FIELD)
        .and_then(Value::as_object)
        .ok_or_else(|| missing(SET_AUTOMATION_LANE_FIELD, "对象"))?;
    reject_unknown_lane_fields(lane)?;
    let remove = read_optional_lane_bool(lane, LANE_REMOVE_FIELD)?.unwrap_or(false);
    let patch = parse_lane_patch(lane)?;
    if remove && !patch.is_empty() {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "`remove: true` 与属性键 (readEnabled/writeMode/domain) 不能同给: \
             「把这条泳道取走」与「把它改成什么」是两件事",
            serde_json::json!({
                "field": LANE_REMOVE_FIELD,
                "reason": "removeTakesNoProperties",
                "properties": [
                    LANE_READ_ENABLED_FIELD,
                    LANE_WRITE_MODE_FIELD,
                    LANE_DOMAIN_FIELD,
                ],
            }),
        ));
    }
    let spec = LaneTargetSpec::parse(lane)?;
    Ok(LaneEdit {
        spec,
        change: if remove {
            LaneChange::Remove
        } else {
            LaneChange::Set(patch)
        },
    })
}
/// 拒绝 `ops[].lane` 对象里 [`SET_AUTOMATION_LANE_FIELDS`] 之外的键。
///
/// 与 [`reject_track_flag_fields`] 同一口径（"拼错的键必须被拒绝, 不能静默忽略"）：
/// 一条被静默忽略的 `read_enabled`（下划线写法）会让调用方以为读开关已经关掉，
/// 而它其实**没关** —— 那正是本形态要修掉的那类"报得出、改不了"。
fn reject_unknown_lane_fields(lane: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = lane
        .keys()
        .map(String::as_str)
        .filter(|key| !SET_AUTOMATION_LANE_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{SET_AUTOMATION_LANE_FIELD}` 里有不支持的键: {} \
             (支持集合: {SET_AUTOMATION_LANE_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownLaneField",
            "unsupportedFields": unknown,
            "supportedLaneFields": SET_AUTOMATION_LANE_FIELDS,
            "hint": "属性名与 `yeban_edit_automation` 响应里的同名 \
                     (`readEnabled` / `writeMode` / `domain`), 不是 project.json 的 \
                     下划线写法",
        }),
    ))
}

/// 解析 `removeAutomationPoint` 的载荷（`ops[].point`）。
///
/// 顺序是刻意的：先拒绝**操作对象**上不该出现的键（顶层写 `tick` 是最像"写对了"
/// 的错法：它会被静默忽略 ⇒ 调用方以为删了 tick 1920 的点），再拒绝**载荷对象**上的键，
/// 最后才解析寻址 —— 与 [`parse_lane_edit`] / [`parse_track_flag_value`] 同一纪律。
///
/// `tick` 与 `pointId` **恰好给一个**（[`PointAddress`] 的两个变体）：
/// 两个同给是 `pointAddressIsAmbiguous`，都不给是 `pointAddressRequired`。
///
/// # Errors
///
/// - `point` 缺失 / 不是对象 → 统一的缺字段错误；
/// - 操作对象或 `point` 对象里有不支持的键 → `INVALID_PARAMETER_RANGE`
///   （`reason` = `unknownPointRemovalField` / `unknownPointField` /
///   `lanePropertiesNotApplicableToPointRemoval`）；
/// - 寻址不是恰好一个 → `pointAddressIsAmbiguous` / `pointAddressRequired`；
/// - `tick` 不是非负整数 / `pointId` 不是合法 ULID → `INVALID_PARAMETER_RANGE`；
/// - 泳道寻址本身非法（未知目标名 / `SendGain` 缺 `edgeId` / 下标形状错）→
///   [`LaneTargetSpec::parse`] 给出的错误。
fn parse_point_removal(
    object: &Map<String, Value>,
) -> Result<(LaneTargetSpec, PointAddress), Fault> {
    reject_point_removal_op_fields(object)?;
    let point = object
        .get(REMOVE_POINT_FIELD)
        .and_then(Value::as_object)
        .ok_or_else(|| missing(REMOVE_POINT_FIELD, "对象"))?;
    reject_point_fields(point)?;
    let spec = LaneTargetSpec::parse(point)?;
    let address = match (
        point.get(REMOVE_POINT_TICK_FIELD),
        point.get(REMOVE_POINT_ID_FIELD),
    ) {
        (Some(_), Some(_)) => {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!(
                    "`{REMOVE_POINT_TICK_FIELD}` 与 `{REMOVE_POINT_ID_FIELD}` 不能同给: \
                     要取走的是哪一个点必须**恰好**由其中一个决定"
                ),
                serde_json::json!({
                    "field": format!("{REMOVE_POINT_FIELD}.{REMOVE_POINT_ID_FIELD}"),
                    "reason": "pointAddressIsAmbiguous",
                    "addressFields": [REMOVE_POINT_TICK_FIELD, REMOVE_POINT_ID_FIELD],
                }),
            ));
        }
        (Some(_), None) => PointAddress::Tick(read_u64(point, REMOVE_POINT_TICK_FIELD)?),
        (None, Some(_)) => PointAddress::Id(read_id(point, REMOVE_POINT_ID_FIELD)?),
        (None, None) => {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!(
                    "必须给定 `{REMOVE_POINT_TICK_FIELD}` (文档里落在该 tick 的那个点) 或 \
                     `{REMOVE_POINT_ID_FIELD}` (点的显式身份), 二者恰好一个"
                ),
                serde_json::json!({
                    "field": REMOVE_POINT_FIELD,
                    "reason": "pointAddressRequired",
                    "addressFields": [REMOVE_POINT_TICK_FIELD, REMOVE_POINT_ID_FIELD],
                }),
            ));
        }
    };
    Ok((spec, address))
}

/// 拒绝 `removeAutomationPoint` 操作对象里 [`REMOVE_POINT_OP_FIELDS`] 之外的键。
///
/// 与 [`reject_track_flag_fields`] 同一口径（"拼错的键必须被拒绝, 不能静默忽略"）：
/// 顶层写 `tick` / `pointId`（正确的位置是 `point` 对象里）若被静默忽略，
/// 调用方会以为点已经取走。`data.hint` 明确写出两个正确位置。
fn reject_point_removal_op_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !REMOVE_POINT_OP_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{REMOVE_AUTOMATION_POINT_KIND}` 操作里有不支持的键: {} \
             (支持集合只有 {REMOVE_POINT_OP_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownPointRemovalField",
            "unsupportedFields": unknown,
            "supportedPointRemovalFields": REMOVE_POINT_OP_FIELDS,
            "hint": format!(
                "点寻址在 `{REMOVE_POINT_FIELD}` 对象里 (`{REMOVE_POINT_TICK_FIELD}` 或 \
                 `{REMOVE_POINT_ID_FIELD}`); 目标音轨是工具顶层的 `trackId`"
            ),
        }),
    ))
}

/// 拒绝 `removeClip` 操作对象里 [`REMOVE_CLIP_FIELDS`] 之外的键。
///
/// 与 [`reject_point_removal_op_fields`] 同一口径（"拼错的键必须被拒绝, 不能静默忽略"）：
/// 本形态的载荷是**空**的，目标片段是工具顶层的 `clipId`；嵌套写一个 `clipId`
/// （或任何别的键）若被静默忽略，调用方会以为取走的是另一个片段。
fn reject_remove_clip_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !REMOVE_CLIP_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{REMOVE_CLIP_KIND}` 操作里有不支持的键: {} \
             (支持集合只有 {REMOVE_CLIP_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownRemoveClipField",
            "unsupportedFields": unknown,
            "supportedRemoveClipFields": REMOVE_CLIP_FIELDS,
            "hint": "要取走的片段池条目是工具顶层的 `clipId` (这个形态自带空载荷)",
        }),
    ))
}

/// 拒绝 `ops[].point` 对象里 [`REMOVE_POINT_FIELDS`] 之外的键。
///
/// 两档**分开点名**（不混成一句"未知键"）：四个泳道**属性**键
/// （`readEnabled` / `writeMode` / `domain` / `remove`）在 `setAutomationLane` 里是合法的，
/// 只是**本形态不适用**（取走一个点不改泳道的属性）—— 与摆放形态的
/// `placementFieldNotApplicable` 同款口径；其余才是真的未知键。
fn reject_point_fields(point: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = point
        .keys()
        .map(String::as_str)
        .filter(|key| !REMOVE_POINT_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    let not_applicable: Vec<&str> = unknown
        .iter()
        .copied()
        .filter(|key| SET_AUTOMATION_LANE_FIELDS.contains(key))
        .collect();
    if !not_applicable.is_empty() {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{REMOVE_POINT_FIELD}` 里有本形态不适用的泳道属性键: {} \
                 (取走一个点不改泳道的读开关 / 写模式 / 取值域; 那些走 `{SET_AUTOMATION_LANE_KIND}`)",
                not_applicable.join(", ")
            ),
            serde_json::json!({
                "reason": "lanePropertiesNotApplicableToPointRemoval",
                "unsupportedFields": not_applicable,
                "supportedPointFields": REMOVE_POINT_FIELDS,
            }),
        ));
    }
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{REMOVE_POINT_FIELD}` 里有不支持的键: {} (支持集合: {REMOVE_POINT_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownPointField",
            "unsupportedFields": unknown,
            "supportedPointFields": REMOVE_POINT_FIELDS,
        }),
    ))
}

/// 解析三个**可选**属性键（缺省 = 保留现值）。
fn parse_lane_patch(lane: &Map<String, Value>) -> Result<LanePatch, Fault> {
    let domain = match lane.get(LANE_DOMAIN_FIELD) {
        // 缺省 = 调用方没提这个键 ⇒ 保留现值（与"清掉覆盖"不是同一件事）。
        None => None,
        // 明写 `null` = 清掉显式覆盖（回到"派生自目标"）。
        Some(Value::Null) => Some(None),
        Some(value) => Some(Some(parse_lane_domain(value)?)),
    };
    Ok(LanePatch {
        read_enabled: read_optional_lane_bool(lane, LANE_READ_ENABLED_FIELD)?,
        write_mode: read_optional_write_mode(lane)?,
        domain,
    })
}

/// 读一个**可选**布尔键（`None` = 缺省；给出 `null` / 数字 / 字符串一律响亮失败）。
///
/// 与 [`parse_track_flag_value`] 同口径：不做真假值强转。
fn read_optional_lane_bool(lane: &Map<String, Value>, field: &str) -> Result<Option<bool>, Fault> {
    match lane.get(field) {
        None => Ok(None),
        Some(value) => value.as_bool().map(Some).ok_or_else(|| {
            Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!("`{field}` 必须是布尔, 实际收到 {value}"),
                serde_json::json!({
                    "field": format!("{SET_AUTOMATION_LANE_FIELD}.{field}"),
                    "reason": "valueMustBeBoolean",
                    "received": value,
                }),
            )
        }),
    }
}

/// 读**可选**写模式（`None` = 缺省）。只认 [`LANE_WRITE_MODES`] 里的规范名。
fn read_optional_write_mode(
    lane: &Map<String, Value>,
) -> Result<Option<AutomationWriteMode>, Fault> {
    let Some(raw) = lane.get(LANE_WRITE_MODE_FIELD) else {
        return Ok(None);
    };
    let text = raw.as_str().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{LANE_WRITE_MODE_FIELD}` 必须是字符串, 实际收到 {raw}"),
            serde_json::json!({
                "field": format!("{SET_AUTOMATION_LANE_FIELD}.{LANE_WRITE_MODE_FIELD}"),
                "reason": "writeModeMustBeString",
                "allowed": LANE_WRITE_MODES,
            }),
        )
    })?;
    // 词表与 `AutomationWriteMode` 的 serde 名字**同源**：判据
    // `lane_write_modes_match_the_model_serde_names` 逐个钉住。
    let mode = match text {
        "Off" => AutomationWriteMode::Off,
        "Write" => AutomationWriteMode::Write,
        "Touch" => AutomationWriteMode::Touch,
        "Latch" => AutomationWriteMode::Latch,
        other => {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!("未知写模式 `{other}`"),
                serde_json::json!({
                    "field": format!("{SET_AUTOMATION_LANE_FIELD}.{LANE_WRITE_MODE_FIELD}"),
                    "reason": "unknownWriteMode",
                    "received": other,
                    "allowed": LANE_WRITE_MODES,
                }),
            ));
        }
    };
    Ok(Some(mode))
}

/// 读取值域覆盖（`{"min":…,"max":…}`）。
///
/// 两端点都走 [`read_number`]（`f64` → `f32` 收窄**之后**判有限性：`1e300` 是有限
/// `f64` 而是无限 `f32`），再交给模型自己的 [`AutomationValueDomain::new`] ——
/// "端点按定义排序"这条不变量**只有一份**实现，本层不自己 `min`/`max`。
fn parse_lane_domain(value: &Value) -> Result<AutomationValueDomain, Fault> {
    let object = value.as_object().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{LANE_DOMAIN_FIELD}` 必须是对象 {{\"min\":…,\"max\":…}} 或 null, \
                 实际收到 {value}"
            ),
            serde_json::json!({
                "field": format!("{SET_AUTOMATION_LANE_FIELD}.{LANE_DOMAIN_FIELD}"),
                "reason": "domainMustBeObjectOrNull",
            }),
        )
    })?;
    let min = read_number(object, "min")?;
    let max = read_number(object, "max")?;
    AutomationValueDomain::new(min, max).map_err(|error| from_model("自动化取值域", &error))
}

/// 写模式的**规范名**（与 [`read_optional_write_mode`] 的词表同源）。
///
/// 走 `AutomationWriteMode` 自己的 serde 名字 ⇒ 模型加一个变体时，这里要么跟着
/// 编译失败（如果改成穷举 `match`），要么由判据 `lane_write_modes_match_the_model_serde_names`
/// 抓住漂移。绝不手写第二张会漂移的表。
#[must_use]
pub fn write_mode_name(mode: AutomationWriteMode) -> String {
    serde_json::to_value(mode)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// 读一个**有限**的 JSON 数字（`f64` → `f32`，与 `value` 的模型类型同宽）。
///
/// 两条防线各管一段，**没有一条是摆设**：
///
/// - JSON 解析器本身不会产出非有限的 `f64`（`serde_json` 对溢出成 `±inf` 的字面量
///   返回 `NumberOutOfRange`），所以"输入是 `NaN` / `±inf`"这条路走不通；
/// - **但收窄到 `f32` 会**：`1e300` 是有限 `f64`，`as f32` 之后是 `inf`。因此有限性
///   检查放在**收窄之后**，判的是模型真正收到的那个 `f32`。
///
/// 越界时本层报 `INVALID_PARAMETER_RANGE`（`data.reason = "nonFiniteValue"`）；
/// 模型 [`Op::SetParam`] 的 `validate_param_value` 对同一个 `f32` 也判 `NonFiniteValue`
/// ⇒ 契约码一致（`domain/error.rs` 的映射），不是两份口径。
/// 范围（声相 `-1.0..=1.0`）**不在这里**：那是模型的事。
fn read_number(object: &Map<String, Value>, field: &str) -> Result<f32, Fault> {
    let raw = object.get(field).ok_or_else(|| missing(field, "数字"))?;
    let number = raw.as_f64().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是数字, 实际收到 {raw}"),
            serde_json::json!({ "field": field, "reason": "valueMustBeNumber" }),
        )
    })?;
    // `Op::SetParam` 的载荷类型就是 `f32`, 所以这一步的 f64 → f32 舍入是模型类型本身
    // 要求的 (与 `read_probability` 同型): IEEE 最近偶数, 确定性。
    #[allow(clippy::cast_possible_truncation)]
    let value = number as f32;
    if !value.is_finite() {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 收窄到 f32 后不是有限数: 输入 {number}, f32 {value}"),
            serde_json::json!({
                "field": field,
                "value": number,
                "narrowedToF32": value.to_string(),
                "reason": "nonFiniteValue",
            }),
        ));
    }
    Ok(value)
}

/// 解析 `note` 对象。
fn parse_note(object: &Map<String, Value>) -> Result<MidiNote, Fault> {
    reject_unknown_note_fields(object)?;
    let id = match object.get("id") {
        None | Some(Value::Null) => deterministic_id(&format!(
            "note:{start}:{pitch}:{duration}",
            start = object.get("startTick").and_then(Value::as_u64).unwrap_or(0),
            pitch = object.get("pitch").and_then(Value::as_u64).unwrap_or(0),
            duration = object
                .get("durationTicks")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        )),
        Some(value) => {
            let text = value.as_str().ok_or_else(|| {
                Fault::domain(ErrorCode::InvalidParameterRange, "`note.id` 必须是字符串")
            })?;
            EntityId::from_str(text).map_err(|error| {
                Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    format!("`note.id` 不是合法 ULID: {error}"),
                )
            })?
        }
    };
    let start_tick = read_u64(object, "startTick")?;
    let pitch = read_range(object, "pitch", 0, 127)?;
    let duration_ticks = read_u64(object, "durationTicks")?;
    let velocity = match object.get("velocity") {
        None => yeban_model::music::DEFAULT_VELOCITY,
        Some(_) => read_range(object, "velocity", 0, 127)?,
    };
    let mut note = MidiNote::new(id, start_tick, pitch, duration_ticks);
    note.velocity = velocity;
    note.probability = read_probability(object)?;
    note.ratchet = read_ratchet(object)?;
    note.micro_timing_ticks = read_micro_timing(object)?;
    note.validate()
        .map_err(|error| from_model("音符校验", &error))?;
    Ok(note)
}

/// 拒绝 `note` 对象里 [`NOTE_FIELDS`] 之外的键。
///
/// 键序是确定性的（`serde_json::Map` 在本 crate 的 feature 集合下是 `BTreeMap`），
/// 因此同一个非法载荷每次报的是**同一个** `field` —— 判据可以逐字钉住它。
///
/// # Errors
///
/// 出现未知键 ⇒ `INVALID_PARAMETER_RANGE`，`data` 带 `field`（第一个未知键）、
/// `supportedNoteFields`（[`NOTE_FIELDS`]）与 `hint`。
fn reject_unknown_note_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let Some(unknown) = object
        .keys()
        .find(|key| !NOTE_FIELDS.contains(&key.as_str()))
    else {
        return Ok(());
    };
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!("`note` 不接受字段 `{unknown}`（不是可选项缺失, 而是拼写/不支持）"),
        serde_json::json!({
            "field": unknown,
            "supportedNoteFields": NOTE_FIELDS,
            // 模型里有、但工具面**还没有**通路的四个表现力字段: 说出来, 不要吞掉。
            "unsupportedNoteFields": ["slide", "pitchBendCurve", "syllable", "phonemes"],
            "hint": "未知键不静默忽略: 去掉它, 或改用 supportedNoteFields 里的字段",
        }),
    ))
}

/// 读可选的 `note.ratchet`（缺省 = `None` = 等价于 1）。
///
/// 取值的**权威**判定在模型层（[`MidiNote::validate`] 的 `RATCHET_MIN..=RATCHET_MAX`）；
/// 这里额外拦一次同类区间，好让越界带上 `field` / `value` / `min` / `max` 的 `data`
/// 载荷（与 `pitch` / `velocity` / `probability` 的既有口径一致），并把"不是整数"
/// 这类 JSON 形状错误与区间错误分成两个契约码。
fn read_ratchet(object: &Map<String, Value>) -> Result<Option<u8>, Fault> {
    let Some(value) = object.get(RATCHET_FIELD) else {
        return Ok(None);
    };
    let number = value.as_i64().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{RATCHET_FIELD}` 必须是整数, 实际收到 {value}"),
        )
    })?;
    if !(i64::from(RATCHET_MIN)..=i64::from(RATCHET_MAX)).contains(&number) {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("`{RATCHET_FIELD}` 越界: {number} 不在 {RATCHET_MIN}..={RATCHET_MAX}"),
            serde_json::json!({
                "field": RATCHET_FIELD,
                "value": number,
                "min": RATCHET_MIN,
                "max": RATCHET_MAX,
            }),
        ));
    }
    Ok(Some(u8::try_from(number).unwrap_or(RATCHET_MAX)))
}

/// 读可选的 `note.microTimingTicks`（缺省 = `None` = 等价于 0）。
///
/// 与 [`read_ratchet`] 同口径：区间 `-MICRO_TIMING_MAX_ABS..=MICRO_TIMING_MAX_ABS`
/// 的权威判定在模型层，这里补 `field` / `value` / `min` / `max` 的 `data` 并区分
/// 形状错误与区间错误。
fn read_micro_timing(object: &Map<String, Value>) -> Result<Option<i16>, Fault> {
    let Some(value) = object.get(MICRO_TIMING_FIELD) else {
        return Ok(None);
    };
    let number = value.as_i64().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{MICRO_TIMING_FIELD}` 必须是整数, 实际收到 {value}"),
        )
    })?;
    let bound = i64::from(MICRO_TIMING_MAX_ABS);
    if !(-bound..=bound).contains(&number) {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!(
                "`{MICRO_TIMING_FIELD}` 越界: {number} 不在 {}..={}",
                -bound, bound
            ),
            serde_json::json!({
                "field": MICRO_TIMING_FIELD,
                "value": number,
                "min": -bound,
                "max": bound,
            }),
        ));
    }
    Ok(Some(i16::try_from(number).unwrap_or(MICRO_TIMING_MAX_ABS)))
}

/// 读可选的 `note.probability`（缺省 = `None` = 必然触发）。
///
/// 取值的**权威**判定在模型层（[`MidiNote::validate`] 的 `0.0..=1.0` 与有限性）；
/// 这里额外拦一次同类区间，好让越界带上 `field` / `value` / `min` / `max` 的 `data`
/// 载荷（与 `pitch` / `velocity` 的既有口径一致），并把"不是数字"这类 JSON 形状
/// 错误与区间错误分成两个契约码。
fn read_probability(object: &Map<String, Value>) -> Result<Option<f32>, Fault> {
    let Some(value) = object.get(PROBABILITY_FIELD) else {
        return Ok(None);
    };
    let raw = value.as_f64().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{PROBABILITY_FIELD}` 必须是数字, 实际收到 {value}"),
        )
    })?;
    if raw.is_nan() || raw < 0.0 || raw > 1.0 {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("`{PROBABILITY_FIELD}` 越界: {raw} 不在 0.0..=1.0"),
            serde_json::json!({
                "field": PROBABILITY_FIELD,
                "value": raw,
                "min": 0.0,
                "max": 1.0,
            }),
        ));
    }
    // `MidiNote::probability` 的类型就是 `f32`，所以这一步的 f64 → f32 舍入是**模型
    // 类型本身**要求的（不是本层多加的一次精度损失）：JSON 数字先按 f64 读出、判完区间
    // 再落到 f32，舍入是 IEEE 最近偶数（确定性），存进工程的就是判定用的那个值。
    #[allow(clippy::cast_possible_truncation)]
    Ok(Some(raw as f32))
}

/// 把解析过的操作编译成 [`Op`]（**读文档**补齐撤销载荷，但绝不改文档）。
///
/// `Delete` / `Move` / `Velocity` 需要当前音符状态：`DeleteNote` 自带
/// `previous_note`，`MoveNote` 与 `ModifyNoteVelocity` 的前置条件会核对
/// "音符此刻确实存在且内容一致"。
///
/// `SetParam` 是**音轨级**的：目标由 `track_id` 给出，`old_val` 从当前文档读
/// （[`AutomationTarget::static_value`]，与 `yeban_edit_automation` 的 `staticValue`
/// 读数同一个入口），因此模型的 `OpStateMismatch` 前置条件天然成立。
///
/// [`NoteOp::SetTrackFlag`] 同样是**音轨级**的：`old_mute` / `old_solo` 由
/// [`TrackFlag::read`] 从当前文档读（模型 `apply` 的前置条件读的是同一个字段）。
///
/// [`NoteOp::RemovePoint`] 也是**音轨级**的：目标是一条泳道上的**一个点**；
/// `tick` 寻址在**文档**上解析（恰好一个落在该 tick 的点），`pointId` 寻址用模型身份；
/// `previous_point` 从当前文档读（模型 `RemoveAutomationPoint` 的前置条件读的是同一个字段）。
///
/// 三个**路由级**形态都自带寻址（与 `track_id` / `clip_id` 无关）：
/// [`NoteOp::SetRoutingGain`] 与 [`NoteOp::DisconnectRouting`] 从当前文档读**边**的现值
/// （`old_gain_db` / `previous_edge`），[`NoteOp::RemoveRoutingNode`] 的载荷是空的
/// —— 它只查"节点在 `nodes` 里"与"不是主总线"两条（见该分支的注释）。
///
/// **段落级**形态 [`NoteOp::RemoveSection`] 同样自带寻址（`sectionId`，与
/// `track_id` / `clip_id` 无关）：它从当前文档读**整条**段落当撤销载荷
/// （`previous_section`），并且**先**查"段落真的在 `project.sections` 里"
/// —— 那一条模型也会报（`SectionNotFound`），但那条路径只在提案模拟里跑。
///
/// **场景级**形态 [`NoteOp::RemoveScene`] 与它同形：自带寻址（`sceneId`），
/// 从当前文档读**整条**场景当撤销载荷（`previous_scene`），并且**先**查"场景真的在
/// `project.scenes` 里"（模型报的是 `SceneNotFound`，同样只在提案模拟那一步跑）。
///
/// # Errors
///
/// - 音轨不存在 → `TRACK_NOT_FOUND`；
/// - 片段不存在 → `CLIP_NOT_FOUND`；**有音符操作**且片段不是 MIDI → `CLIP_NOT_FOUND`
///   （纯 `setParam` / 纯开关 / 纯路由 / 纯段落 / 纯场景调用不要求片段是 MIDI：
///   它们不读片段内容）；
/// - 音符不存在 → `ENTITY_NOT_FOUND`；自动化泳道或点不存在 → `ENTITY_NOT_FOUND`；
///   路由边或路由节点不存在 → `ENTITY_NOT_FOUND`（`routingEdgeNotFound` /
///   `routingNodeNotFound`）；曲式段落不存在 → `ENTITY_NOT_FOUND`（`sectionNotFound`）；
///   场景不存在 → `ENTITY_NOT_FOUND`（`sceneNotFound`）；
/// - 目标是主总线（`removeRoutingNode`）→ `CONFLICT`（`masterBusNodeCannotBeRemoved`）；
/// - 模型层校验失败 → [`super::error::code_for_model`] 给出的契约码。
pub fn compile(
    project: &YebanProjectV1,
    track_id: &EntityId,
    clip_id: &EntityId,
    ops: &[NoteOp],
) -> Result<Vec<Op>, Fault> {
    let track = project
        .track(track_id)
        .map_err(|error| from_model("音轨查找", &error))?;
    let entry = project
        .clip_pool
        .get(clip_id)
        .ok_or_else(|| Fault::domain(ErrorCode::ClipNotFound, format!("片段不存在: {clip_id}")))?;
    // "必须是 MIDI 片段"这条断言只在**真的有音符操作**时成立：`setParam` 一个音符都不读。
    // 四个音符形态的调用因此逐字节等于旧行为（它们总是走到这条断言）。
    if ops.iter().any(NoteOp::is_note_level) && entry.content.notes().is_none() {
        return Err(Fault::domain(
            ErrorCode::ClipNotFound,
            format!("片段 {clip_id} 不是 MIDI 片段, 没有音符集合"),
        ));
    }

    let mut compiled = Vec::with_capacity(ops.len());
    for op in ops {
        compiled.push(match op {
            NoteOp::Add { note } => Op::AddNote {
                track_id: *track_id,
                clip_id: *clip_id,
                note: (**note).clone(),
            },
            NoteOp::Delete { note_id } => {
                let previous_note = project
                    .note(clip_id, note_id)
                    .map_err(|error| from_model("音符查找", &error))?
                    .clone();
                Op::DeleteNote {
                    track_id: *track_id,
                    clip_id: *clip_id,
                    note_id: *note_id,
                    previous_note,
                }
            }
            NoteOp::Move {
                note_id,
                delta_tick,
                delta_pitch,
            } => {
                let current = project
                    .note(clip_id, note_id)
                    .map_err(|error| from_model("音符查找", &error))?;
                // 音域与时间轴越界在**这里**就报 OUT_OF_RANGE（§7.2 给本工具声明的码），
                // 而不是让模型层的 OpStateMismatch 冒泡成 CONFLICT。
                let shifted_pitch = i16::from(current.pitch) + i16::from(*delta_pitch);
                if !(0..=127).contains(&shifted_pitch) {
                    return Err(Fault::domain_with_data(
                        ErrorCode::OutOfRange,
                        format!(
                            "平移后音高越界: {} + {} = {shifted_pitch}",
                            current.pitch, delta_pitch
                        ),
                        serde_json::json!({ "noteId": note_id.to_canonical_string(), "pitch": current.pitch, "deltaPitch": delta_pitch }),
                    ));
                }
                let shifted_tick = i128::from(current.start_tick) + i128::from(*delta_tick);
                if !(0..=i128::from(u64::MAX)).contains(&shifted_tick) {
                    return Err(Fault::domain_with_data(
                        ErrorCode::OutOfRange,
                        format!(
                            "平移后 tick 越界: {} + {} = {shifted_tick}",
                            current.start_tick, delta_tick
                        ),
                        serde_json::json!({ "noteId": note_id.to_canonical_string(), "startTick": current.start_tick, "deltaTick": delta_tick }),
                    ));
                }
                Op::MoveNote {
                    track_id: *track_id,
                    clip_id: *clip_id,
                    note_id: *note_id,
                    delta_tick: *delta_tick,
                    delta_pitch: *delta_pitch,
                }
            }
            NoteOp::Velocity { note_id, velocity } => {
                let current = project
                    .note(clip_id, note_id)
                    .map_err(|error| from_model("音符查找", &error))?;
                Op::ModifyNoteVelocity {
                    track_id: *track_id,
                    clip_id: *clip_id,
                    note_id: *note_id,
                    old_vel: current.velocity,
                    new_vel: *velocity,
                }
            }
            NoteOp::SetParam { lane, value } => {
                let target = lane.target(*track_id);
                // 撤销载荷来自**唯一**的静态值入口 (`AutomationTarget::static_value`)：
                // 本层不自己读 `track.volume_db` / `track.pan`（那会是第二份真相）。
                let old_val = target
                    .static_value(project)
                    .map_err(|error| from_model("静态值读取", &error))?;
                Op::SetParam {
                    target,
                    old_val,
                    new_val: *value,
                }
            }
            NoteOp::SetTrackFlag { flag, value } => {
                // 撤销载荷来自**当前文档**（模型 `apply` 的前置条件读同一个字段）；
                // 本层不自己写 `Op::invert`（那是模型的唯一事实源）。
                flag.compile(*track_id, flag.read(track), *value)
            }
            NoteOp::SetLane { edit } => {
                let target = edit.spec.target(*track_id);
                // 撤销载荷来自**当前文档**：模型 `SetAutomationLane` 的前置条件要求
                // `old_lane` 等于文档现值（`RemoveAutomationLane` 同理要求
                // `previous_lane`），因此这里不采信调用方声明的"旧状态"。
                let current = track.automation_lanes.get(&target).cloned();
                match edit.change {
                    LaneChange::Remove => {
                        let previous_lane = current.ok_or_else(|| {
                            Fault::domain_with_data(
                                ErrorCode::EntityNotFound,
                                format!("这条自动化泳道不存在, 没有东西可以取走: {target:?}"),
                                serde_json::json!({
                                    "lane": edit.spec.kind.as_str(),
                                    "reason": "automationLaneNotFound",
                                }),
                            )
                        })?;
                        Op::RemoveAutomationLane {
                            target,
                            previous_lane,
                        }
                    }
                    LaneChange::Set(patch) => {
                        // 基础 = 文档现值；没有泳道时用模型的**隐式**默认形状
                        // （`read_enabled = true` / `write_mode = Off` / `domain = None`）
                        // —— 那是"这条泳道还不存在"的规范含义，不是本层发明的默认值。
                        let (old_lane, base) = match current {
                            Some(lane) => (Some(lane.clone()), lane),
                            None => (None, AutomationLane::implicit(target)),
                        };
                        Op::SetAutomationLane {
                            target,
                            old_lane,
                            new_lane: patch.apply_to(&base),
                        }
                    }
                }
            }
            NoteOp::RemovePoint { spec, address } => {
                let target = spec.target(*track_id);
                let lane = track.automation_lanes.get(&target).ok_or_else(|| {
                    Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!("这条自动化泳道不存在, 没有点可以取走: {target:?}"),
                        serde_json::json!({
                            "lane": spec.kind.as_str(),
                            "reason": "automationLaneNotFound",
                        }),
                    )
                })?;
                // 按 tick 寻址 = "文档里那一个**恰好**落在该 tick 上的点"：
                // 0 个是 `ENTITY_NOT_FOUND`，≥2 个是**响亮拒绝**（不挑一个）。
                // 模型只保证 `point.id` 唯一，**不保证** tick 唯一
                // （`AutomationLane::validate` 只查键/身份一致与取值有限），
                // 因此"tick 上恰好一个点"必须在**这里**判，不能假定。
                let point_id = match address {
                    PointAddress::Id(id) => *id,
                    PointAddress::Tick(tick) => unique_point_at(lane, *tick, spec.kind)?,
                };
                let previous_point = lane.points.get(&point_id).copied().ok_or_else(|| {
                    Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!(
                            "这条泳道上没有身份 {point_id} 的点 (取走必须指向一个**存在**的点)"
                        ),
                        serde_json::json!({
                            "lane": spec.kind.as_str(),
                            "pointId": point_id.to_canonical_string(),
                            "reason": "automationPointNotFound",
                        }),
                    )
                })?;
                // 撤销载荷来自**当前文档**（模型 `RemoveAutomationPoint` 的前置条件要求
                // `previous_point` 等于文档现值），因此本层不采信调用方声明的旧状态。
                Op::RemoveAutomationPoint {
                    target,
                    point_id,
                    previous_point,
                }
            }
            NoteOp::RemoveClip => {
                // 撤销载荷来自**当前文档**：`entry` 就是池子里那一条（上面已经查过，
                // 因此"池里没有这条片段"在这一行之前就已经是 `CLIP_NOT_FOUND`）。
                // 模型 `RemoveClip` 的 `validate` 要求 `previous_clip` **逐字段**等于
                // 文档现值，并且**没有任何摆放引用它**（否则 `ClipInUse`）—— 那两条
                // 都不在本层复制，模型是唯一事实源。
                Op::RemoveClip {
                    clip_id: *clip_id,
                    previous_clip: entry.clone(),
                }
            }
            NoteOp::SetRoutingGain { edge_id, gain_db } => {
                // 撤销载荷来自**当前文档**，而且读的是 `RoutingGraph::edge` 的
                // **原样** `Option<f32>`（**不**走 `AutomationTarget::static_value`：
                // 那个唯一静态值入口把 `None` 折算成 `0.0`，而模型的 `same_gain`
                // 逐位比较 —— `None` 与 `Some(0.0)` 不是同一个值）。用折算过的值会让
                // 模型的前置条件在"文档里是单位增益"的边上失败（`OpStateMismatch`
                // ⇒ `CONFLICT`），也就是改不了最常见的那一类边。
                let edge = project.routing_graph.edge(edge_id).ok_or_else(|| {
                    Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!("工程里没有身份 {edge_id} 的路由边, 没有增益可以写"),
                        serde_json::json!({
                            "edgeId": edge_id.to_canonical_string(),
                            "reason": "routingEdgeNotFound",
                            "hint": "路由边的身份由 `yeban_query_project` 的 \
                                     `routing_graph` 字段报出",
                        }),
                    )
                })?;
                Op::SetRoutingGain {
                    edge_id: *edge_id,
                    old_gain_db: edge.gain_db,
                    new_gain_db: *gain_db,
                }
            }
            NoteOp::DisconnectRouting { edge_id } => {
                // 撤销载荷来自**当前文档**的**整条**边：模型 `DisconnectRouting` 的
                // 前置条件要求 `previous_edge` 逐字段等于文档现值（`RoutingEdge` 是
                // `Copy`，因此这里是值搬运），因此本层不采信调用方声明的旧状态，
                // 也不接受调用方送来的载荷（`reject_disconnect_routing_fields` 只认
                // `kind` 与 `edgeId`）。`nodes` 一个字都不动 —— 取走节点是
                // `Op::RemoveRoutingNode` 的事，它有"没有任何边引用它"的前置条件。
                let previous_edge = project
                    .routing_graph
                    .edge(edge_id)
                    .copied()
                    .ok_or_else(|| {
                        Fault::domain_with_data(
                            ErrorCode::EntityNotFound,
                            format!("工程里没有身份 {edge_id} 的路由边, 没有边可以断开"),
                            serde_json::json!({
                                "edgeId": edge_id.to_canonical_string(),
                                "reason": "routingEdgeNotFound",
                                "hint": "路由边的身份由 `yeban_query_project` 的 \
                                         `routing_graph` 字段报出",
                            }),
                        )
                    })?;
                Op::DisconnectRouting {
                    edge_id: *edge_id,
                    previous_edge,
                }
            }
            NoteOp::RemoveRoutingNode { node_id } => {
                // 载荷是**空**的（模型 `RemoveRoutingNode` 只有节点身份，且它与
                // `AddRoutingNode` 互为逆操作），因此这里没有撤销载荷可读。
                //
                // 两条提前拒绝（与 `setRoutingGain` / `disconnectRouting` 的
                // `routingEdgeNotFound` 同一纪律 —— 让"身份打错"在**编译期**就带上
                // `reason` 与 `hint`，而不是在提案模拟那一步冒出一个泛化消息）：
                //
                // 1. 节点必须已经在 `routing_graph.nodes` 里（模型 `Op::validate`
                //    报的是 `RoutingNodeNotFound`，契约码相同, 但那条路径只在提案
                //    模拟里跑，消息里没有本层的 `reason` / `hint`）。
                // 2. 主总线**不能**被取走：`YebanProjectV1::validate` 要求有音轨时
                //    主总线在 `nodes` 里（`yeban-engine` 的 `PdcPlan::compute` 以它
                //    为前置条件）。模型把这条不变量复用了 `RoutingNodeNotFound`
                //    （见 `project.rs` 里那段注释的理由），于是"取走主总线"会以
                //    "节点不存在"告终 —— 而节点刚刚被自己取走。这里用一个说得通的
                //    契约码（`CONFLICT`）报同一个结论。
                //
                // "没有任何边引用它"**不在这里**复制：那是 `Op::validate` 的
                // `RoutingNodeInUse`（带 `edge_count`），由提案模拟报出。
                if !project.routing_graph.nodes.contains(node_id) {
                    return Err(Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!("工程的 routing_graph.nodes 里没有身份 {node_id} 的节点"),
                        serde_json::json!({
                            "nodeId": node_id.to_canonical_string(),
                            "reason": "routingNodeNotFound",
                            "hint": "节点的身份由 `yeban_query_project` 的 \
                                     `routing_graph.nodes` 数组报出",
                        }),
                    ));
                }
                if *node_id == project.master_bus_track_id {
                    return Err(Fault::domain_with_data(
                        ErrorCode::Conflict,
                        format!(
                            "身份 {node_id} 是工程的主总线, 模型要求主总线留在 \
                             routing_graph.nodes 里, 不能取走"
                        ),
                        serde_json::json!({
                            "nodeId": node_id.to_canonical_string(),
                            "masterBusTrackId": project.master_bus_track_id.to_canonical_string(),
                            "reason": "masterBusNodeCannotBeRemoved",
                            "hint": "主总线是唯一的声学出口 (`PdcPlan::compute` 以\
                                     「主总线在 nodes 里」为前置条件); 本形态只取走**非**\
                                     主总线节点, 并且在取走之前必须先断开引用它的每一条边",
                        }),
                    ));
                }
                Op::RemoveRoutingNode { node: *node_id }
            }
            NoteOp::RemoveSection { section_id } => {
                // 撤销载荷来自**当前文档**的**整条**段落：模型 `RemoveSection` 的前置条件
                // 要求 `previous_section` 逐字段等于文档现值，因此本层不采信调用方声明的
                // 旧状态，也不接受调用方送来的载荷（`reject_remove_section_fields` 只认
                // `kind` 与 `sectionId`）。`SectionV3` 不与别的实体交叉引用，因此这里
                // **没有**"还被谁引用"这一类前置条件要查（与 `RemoveClip` /
                // `RemoveRoutingNode` 不同 —— 那两条的模型前置条件真的存在，本层不复制）。
                let previous_section = project.sections.get(section_id).cloned().ok_or_else(|| {
                    Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!("工程里没有身份 {section_id} 的曲式段落, 没有段落可以取走"),
                        serde_json::json!({
                            "sectionId": section_id.to_canonical_string(),
                            "reason": "sectionNotFound",
                            "hint": "段落的身份由 `yeban_query_project` 的 `entities[]` 里 \
                                     `kind == \"section\"` 的条目报出",
                        }),
                    )
                })?;
                Op::RemoveSection {
                    section_id: *section_id,
                    previous_section,
                }
            }
            NoteOp::RemoveScene { scene_id } => {
                // 撤销载荷来自**当前文档**的**整条**场景：模型 `RemoveScene` 的前置条件
                // 要求 `previous_scene` 逐字段等于文档现值，因此本层不采信调用方声明的
                // 旧状态，也不接受调用方送来的载荷（`reject_remove_scene_fields` 只认
                // `kind` 与 `sceneId`）。`SceneV3` 不与别的实体交叉引用：全仓
                // `git grep -nE '(scene_id|sceneId)' -- 'crates/**/*.rs'` 的命中只落在
                // 三个文件 —— `yeban-model` 的 `ops.rs`（`Op` 自己的载荷与校验/应用/取反）、
                // `yeban-render` 的 `als.rs`（导出 .als 时**本地**生成 XML 场景号的变量）、
                // 以及本文件；没有任何**实体**持有场景身份。因此这里**没有**
                // "还被谁引用"这一类前置条件要查（与 `RemoveClip` / `RemoveRoutingNode`
                // 不同 —— 那两条的模型前置条件真的存在，本层不复制）。
                let previous_scene = project.scenes.get(scene_id).cloned().ok_or_else(|| {
                    Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!("工程里没有身份 {scene_id} 的场景, 没有场景可以取走"),
                        serde_json::json!({
                            "sceneId": scene_id.to_canonical_string(),
                            "reason": "sceneNotFound",
                            "hint": "场景的身份由 `yeban_query_project` 的 `entities[]` 里 \
                                     `kind == \"scene\"` 的条目报出",
                        }),
                    )
                })?;
                Op::RemoveScene {
                    scene_id: *scene_id,
                    previous_scene,
                }
            }
            NoteOp::SetScene {
                scene_id,
                create,
                patch,
            } => {
                // 新建与更新由**显式**的 `create` 区分（不靠"文档里没有这个身份就当作
                // 新建" —— 那会让一个打错的 `sceneId` 静默建出一个新场景）。
                // 两条前置条件都在这里提前报（模型 `Op::validate` 只在提案模拟那一步
                // 跑，消息里没有本层的 `reason` / `hint`）。
                let current = project.scenes.get(scene_id).cloned();
                if *create {
                    if current.is_some() {
                        return Err(Fault::domain_with_data(
                            ErrorCode::Conflict,
                            format!(
                                "工程里已经有身份 {scene_id} 的场景, `{SCENE_CREATE_FIELD}: true` \
                                 不覆盖既有场景"
                            ),
                            serde_json::json!({
                                "sceneId": scene_id.to_canonical_string(),
                                "reason": "sceneAlreadyExists",
                                "hint": "把 `create` 去掉就是一次普通更新; 要新建请换一个 `sceneId`",
                            }),
                        ));
                    }
                } else if current.is_none() {
                    return Err(Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!(
                            "工程里没有身份 {scene_id} 的场景, 没有场景可以更新 \
                             (新建请给 `{SCENE_CREATE_FIELD}: true`)"
                        ),
                        serde_json::json!({
                            "sceneId": scene_id.to_canonical_string(),
                            "reason": "sceneNotFound",
                            "hint": "场景的身份由 `yeban_query_project` 的 `entities[]` 里 \
                                     `kind == \"scene\"` 的条目报出",
                        }),
                    ));
                }
                // 撤销载荷来自**当前文档**：模型 `SetScene` 的前置条件要求
                // `old_scene` **逐字段**等于文档现值（`None` = 新建），因此本层不采信
                // 调用方声明的旧状态。基础 = 文档现值；新建时用 `SceneV3` 的**空形状**
                // （`name` 由 `create` 分支强制要求，因此不可能落下一个空名）。
                let base = current.clone().unwrap_or(SceneV3 {
                    id: *scene_id,
                    name: String::new(),
                    tempo: None,
                    color: None,
                });
                Op::SetScene {
                    scene_id: *scene_id,
                    old_scene: current,
                    new_scene: patch.apply_to(&base),
                }
            }
        });
    }
    Ok(compiled)
}

/// 文档里**恰好一个**落在 `tick` 上的点，返回它的身份（[`PointAddress::Tick`] 的解析）。
///
/// 三个分支都是**响亮**的：0 个 → `ENTITY_NOT_FOUND`（`automationPointNotFound`）；
/// 1 个 → 该身份；≥2 个 → `INVALID_PARAMETER_RANGE`（`automationTickAmbiguous`，
/// `data.candidatePointIds` 列出全部候选）。
///
/// 为什么 ≥2 个不是"取第一个"：模型只保证 `point.id` 唯一
/// （[`AutomationLane::validate`] 查键/身份一致与取值有限），**不保证**同一泳道内
/// tick 唯一 —— 显式 `pointId` 的写入可以在同一 tick 上留下两个点。
/// 静默挑一个会删掉调用方**没指名**的那个点，而撤销载荷（`previous_point`）
/// 只记被删的那个 ⇒ 调用方无从发现。
fn unique_point_at(lane: &AutomationLane, tick: u64, kind: LaneKind) -> Result<EntityId, Fault> {
    let candidates: Vec<EntityId> = lane
        .points
        .values()
        .filter(|point| point.tick == tick)
        .map(|point| point.id)
        .collect();
    match candidates.as_slice() {
        [only] => Ok(*only),
        [] => Err(Fault::domain_with_data(
            ErrorCode::EntityNotFound,
            format!("这条泳道上没有落在 tick {tick} 上的点"),
            serde_json::json!({
                "lane": kind.as_str(),
                "tick": tick,
                "reason": "automationPointNotFound",
            }),
        )),
        many => Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "这条泳道上有 {} 个点落在 tick {tick} 上, `tick` 不足以指名一个点 \
                 (请改用 `{REMOVE_POINT_ID_FIELD}`)",
                many.len()
            ),
            serde_json::json!({
                "lane": kind.as_str(),
                "tick": tick,
                "reason": "automationTickAmbiguous",
                "candidatePointIds": many
                    .iter()
                    .map(EntityId::to_canonical_string)
                    .collect::<Vec<_>>(),
            }),
        )),
    }
}

/// 拒绝"同一次调用里把同一条泳道写了两次"。
///
/// 为什么这是**响亮失败**而不是"后一条赢"：批里的每一条 `setAutomationLane` 的
/// `old_lane` 都从**调用前**的文档读，因此第二条的前置条件必然与第一条施加后的
/// 状态不符（模型报 `OpStateMismatch` ⇒ `CONFLICT`，一个说不清是哪条 op 的错）。
/// 在**建提案之前**用 `INVALID_PARAMETER_RANGE` 点名重复的目标，调用方才知道
/// 要拆成两次调用。
///
/// 只在真的出现 `SetLane` 时才做（其余形态逐字节等于接线之前的行为）。
///
/// # Errors
///
/// 同一目标的 `SetLane` 出现两次以上 → `INVALID_PARAMETER_RANGE`
/// （`data.reason = "duplicateLaneTarget"`，`data.lane` = 目标变体名）。
pub fn reject_duplicate_lane_targets(track_id: &EntityId, ops: &[NoteOp]) -> Result<(), Fault> {
    let mut seen: Vec<AutomationTarget> = Vec::new();
    for op in ops {
        let NoteOp::SetLane { edit } = op else {
            continue;
        };
        let target = edit.spec.target(*track_id);
        if seen.contains(&target) {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!(
                    "同一次调用里同一条自动化泳道被写了两次: {} \
                     (批内每条的撤销载荷都从调用前的文档读, 第二条必然对不上状态)",
                    edit.spec.kind.as_str()
                ),
                serde_json::json!({
                    "reason": "duplicateLaneTarget",
                    "lane": edit.spec.kind.as_str(),
                }),
            ));
        }
        seen.push(target);
    }
    Ok(())
}

/// 拒绝**同一次调用里同一个场景被写两次**（`setScene`）。
///
/// 与 [`reject_duplicate_lane_targets`] **逐条同因**：批内每一条的撤销载荷
/// （`old_scene`）都是 [`compile`] 从**调用前**的文档读的，而批是顺序施加的 ⇒
/// 第二条的 `old_scene` 必然与那一刻的文档现值不符，模型会报
/// `OpStateMismatch`（契约码 `CONFLICT`）。那条消息说不清是哪一条 op 的错，
/// 因此在**建提案之前**就响亮拒绝。
///
/// 只在真的出现 `setScene` 时才做（其余形态逐字节等于接线之前的行为）。
///
/// # Errors
///
/// 同一场景身份的 `setScene` 出现两次以上 → `INVALID_PARAMETER_RANGE`
/// （`data.reason = "duplicateSceneTarget"`，`data.sceneId` = 那个身份）。
pub fn reject_duplicate_scene_targets(ops: &[NoteOp]) -> Result<(), Fault> {
    let mut seen: Vec<EntityId> = Vec::new();
    for op in ops {
        let NoteOp::SetScene { scene_id, .. } = op else {
            continue;
        };
        if seen.contains(scene_id) {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!(
                    "同一次调用里同一个场景被写了两次: {scene_id} \
                     (批内每条的撤销载荷都从调用前的文档读, 第二条必然对不上状态)"
                ),
                serde_json::json!({
                    "reason": "duplicateSceneTarget",
                    "sceneId": scene_id.to_canonical_string(),
                    "hint": "同一个场景的多处修改请折成**一条** `setScene` \
                             (三个属性键可以一起给), 而不是两条",
                }),
            ));
        }
        seen.push(*scene_id);
    }
    Ok(())
}

/// 拒绝**池级形态**（`removeClip`）与另外三路混用。
///
/// 池级取走把顶层 `clipId` 那条**片段池条目**取走：它既不读不写音符，也不碰摆放，
/// 因此它必须**单独**出现在 `ops` 里，也不能和 `placement`（动时间轴上的摆放）
/// 同给。两条规则都在**建提案之前**响亮拒绝 —— 让一个自相矛盾的批走到模型层，
/// 撞到的会是一个说不清是哪条 op 的错。
///
/// 与 [`reject_duplicate_lane_targets`] 同一条纪律：只在真的出现 `removeClip` 时才做事
/// （其余形态逐字节等于接线之前的行为）。
///
/// # Errors
///
/// - `removeClip` 与别的 `kind` 同给 → `INVALID_PARAMETER_RANGE`
///   （`data.reason = "removeClipTakesNoOtherOps"`，`data.opKinds` = 本次真给的 `kind` 列表）；
/// - `removeClip` 与 `placement` 同给 → `INVALID_PARAMETER_RANGE`
///   （`data.reason = "removeClipIsNotPlacement"`）。
pub fn reject_remove_clip_conflicts(ops: &[NoteOp], placement_present: bool) -> Result<(), Fault> {
    let removing_clip = matches!(ops, [NoteOp::RemoveClip]);
    if ops.iter().any(|op| matches!(op, NoteOp::RemoveClip)) && !removing_clip {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "`removeClip` 只能**单独**出现: 同一次调用不能既编辑一条片段又把它取走",
            serde_json::json!({
                "reason": "removeClipTakesNoOtherOps",
                "opKinds": ops.iter().map(NoteOp::kind_name).collect::<Vec<_>>(),
                "hint": "先做音符 / 音轨级编辑, 再单独一次调用取走池里那条材料",
            }),
        ));
    }
    if removing_clip && placement_present {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "`removeClip` 与 `placement` 不能同给: 前者取走池里的材料, 后者动时间轴上的摆放",
            serde_json::json!({
                "reason": "removeClipIsNotPlacement",
                "hint": "先 `placement.kind: \"remove\"` 取走摆放, 再单独一次调用取走池里的材料",
            }),
        ));
    }
    Ok(())
}

/// **材料创建**形态的编译（`arguments.create: true`）：把一组 `add` 折成**一条**
/// [`Op::AddClip`]。
///
/// 与 [`compile`] 的分工：`compile` 改**已存在**的片段（每条 `NoteOp` 一条 `Op`），
/// 本函数建**新**片段（`ops` 全部折进 `AddClip` 的初始内容，因此产物恰好一条 `Op`）。
/// 两者共用同一个 `NoteOp` 解析器与同一个发声数上限常量。
///
/// ⚠ 本函数**不摆放**：新片段只在 `clip_pool` 里。渲染与 `yeban_export_midi` 只遍历
/// `track.clips`，因此未摆放的片段不出声 —— 这是刻意的（"配器材料"只要求池里有材料），
/// 并且如实写在响应 `willCreate.clipPoolEntries` 里，不假装它已经上了时间轴。
///
/// # Errors
///
/// - 音轨不存在 → `TRACK_NOT_FOUND`；
/// - 池里已有该 `clipId` → `CONFLICT`（`data.reason = "clipAlreadyExists"`）；
/// - `ops` 里出现 `add` 之外的操作 → `INVALID_PARAMETER_RANGE`
///   （`data.reason = "createRequiresAddOps"`）；
/// - 两个 `add` 用同一个音符身份 → `INVALID_PARAMETER_RANGE`
///   （`data.reason = "duplicateNoteId"`）。
pub fn compile_create(
    project: &YebanProjectV1,
    track_id: &EntityId,
    clip_id: &EntityId,
    clip_name: &str,
    ops: &[NoteOp],
) -> Result<Vec<Op>, Fault> {
    // 与 `compile` 同口径的入口校验：`trackId` 必须是工程里真实存在的音轨。
    project
        .track(track_id)
        .map_err(|error| from_model("音轨查找", &error))?;
    if project.clip_pool.contains_key(clip_id) {
        return Err(Fault::domain_with_data(
            ErrorCode::Conflict,
            format!("片段池里已经有身份 {clip_id}, `create: true` 不覆盖既有片段"),
            serde_json::json!({
                "clipId": clip_id.to_canonical_string(),
                "reason": "clipAlreadyExists",
                "hint": "把 `create` 去掉就是一次普通编辑; 要新建请换一个 `clipId`",
            }),
        ));
    }
    let mut notes: BTreeMap<EntityId, MidiNote> = BTreeMap::new();
    for op in ops {
        match op {
            NoteOp::Add { note } => {
                if notes.insert(note.id, (**note).clone()).is_some() {
                    return Err(Fault::domain_with_data(
                        ErrorCode::InvalidParameterRange,
                        format!("两个 `add` 用了同一个音符身份 {}", note.id),
                        serde_json::json!({
                            "reason": "duplicateNoteId",
                            "noteId": note.id.to_canonical_string(),
                        }),
                    ));
                }
            }
            other => {
                return Err(Fault::domain_with_data(
                    ErrorCode::InvalidParameterRange,
                    format!(
                        "`create: true` 时 `ops` 只允许 `add` (新片段里还没有音符可以被 \
                         `delete`/`move`/`velocity` 指向; 音轨级的 \
                         `setParam`/`setTrackMute`/`setTrackSolo`/`setAutomationLane`/\
                         `removeAutomationPoint` 与建材料无关; 池级的 `removeClip` 更是 \
                         与「建」相反的一步), 实际收到 `{}`",
                        other.kind_name()
                    ),
                    serde_json::json!({
                        "reason": "createRequiresAddOps",
                        "supportedKindsWhenCreating": ["add"],
                        "received": other.kind_name(),
                    }),
                ));
            }
        }
    }
    // `parse_ops` 已经拒绝空数组, 且上面只放行 `add` ⇒ `notes` 至少一条。
    // 仍显式断言: "空 MIDI 片段"不是可用材料 (`section_build` 的 `usable_materials`),
    // 建出它等于把 needs-8 的死角换一个地方。
    if notes.is_empty() {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`create: true` 至少需要一个 `add` 音符: 空片段不是可用材料",
        ));
    }
    Ok(vec![Op::AddClip {
        clip: ClipPoolEntry {
            id: *clip_id,
            name: clip_name.to_owned(),
            content: ClipContent::Midi { notes },
        },
    }])
}

/// 一次**摆放**的确定性标签：`(片段身份, 音轨身份, 起始 tick)`。
///
/// 与 `extension_pure::placement_label`（音频导入那一侧）**分开**一个前缀：
/// 两条路径的片段不是同一类材料，标签也不该长得一样（同一份 `deterministic_id`
/// 输入不同的标签 ⇒ 不同的摆放身份）。
///
/// 时值**不在**标签里：同一片段、同一音轨、同一起点是**同一次摆放**，改时值是
/// "改这一次摆放的长度"，不是凭空多出第二条摆放（重复提交同一起点但不同时值由
/// [`parse_placement`] 判成 `CONFLICT`，与"同身份不同内容的片段"同一口径）。
#[must_use]
pub fn placement_label(clip_id: &str, track_id: &str, start_tick: u64) -> String {
    format!("midi-placement:{clip_id}:{track_id}:{start_tick}")
}

/// 解析 `arguments.placement`，按 `placement.kind` 分发到三种**摆放编辑**。
///
/// | `kind` | 载荷 | 编译成 |
/// | :--- | :--- | :--- |
/// | 缺省 / `add` | `startTick?` / `durationTicks?` / `placementId?` / `muted?` | [`Op::AddClipPlacement`] |
/// | `move` | `placementId`（必填）+ `startTick`（必填 = **新**起点） | [`Op::MoveClipPlacement`] |
/// | `remove` | `placementId`（必填） | [`Op::RemoveClipPlacement`] |
///
/// 为什么有 `move` / `remove`：`f1098e2` 让 `create: true` 把材料**建**进池子，
/// `ff23302` 让 `add` 把材料**摆**上时间轴；到此为止工具面能**加**一条摆放，
/// 却**没有任何**工具能挪动或取走它 —— `Op::MoveClipPlacement` 与
/// `Op::RemoveClipPlacement` 在模型层早已实现（各自带自包含撤销载荷），
/// 渲染器也**真的**按 `track.clips` 出片，因此那两个能力在 17 个工具的面上
/// **不可达**：摆错位置只剩"整次调用撤销"一条路，而撤到那一步之前的编辑会一起丢。
/// 同族缺口的先例是 `probability`（`314d2fc`）与 `ratchet`/`microTimingTicks`
/// （`791a571`）—— 都是"模型/渲染器已经做到、工具面够不着"。
///
/// `placementId` 缺省派生**只**属于 `add`：派生一条新身份不可能命中一条已有摆放，
/// 因此 `move` / `remove` 缺它就是 [`require_placement_id`] 的响亮失败。
///
/// # Errors
///
/// - `placement` 不是对象 ⇒ `INVALID_PARAMETER_RANGE`；
/// - `kind` 不是字符串 / 不在 [`PLACEMENT_KINDS`] 里 ⇒ `INVALID_PARAMETER_RANGE`
///   （`reason = "unknownPlacementKind"`，`data` 列出支持集合）；
/// - 键不在本形态的词表里 ⇒ `INVALID_PARAMETER_RANGE`（见 [`reject_placement_fields`]）；
/// - `add` 形态的一切失败 ⇒ 见 [`parse_placement`]；
/// - `move` / `remove` 找不到那条摆放 ⇒ `ENTITY_NOT_FOUND`（`placementNotFound`）；
/// - `clipId` 与文档里那条摆放引用不一致 ⇒ `INVALID_PARAMETER_RANGE`
///   （`placementClipMismatch`）；
/// - `move` 的目标起点等于现值 ⇒ `CONFLICT`（`placementAlreadyAtStartTick`）——
///   没有可提交的改动，不制造一条空提案。
pub fn parse_placement_edit(
    project: &YebanProjectV1,
    track_id: &EntityId,
    clip_id: &EntityId,
    arguments: &Map<String, Value>,
) -> Result<Option<PlacementEdit>, Fault> {
    let Some(raw) = arguments.get(PLACEMENT_FIELD) else {
        return Ok(None);
    };
    let object = raw.as_object().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{PLACEMENT_FIELD}` 必须是对象, 实际收到 {raw}"),
        )
    })?;
    let kind = match object.get(PLACEMENT_KIND_FIELD) {
        None => PLACEMENT_KIND_ADD,
        Some(value) => value.as_str().ok_or_else(|| {
            Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!(
                    "`{PLACEMENT_FIELD}.{PLACEMENT_KIND_FIELD}` 必须是字符串, 实际收到 {value}"
                ),
                serde_json::json!({
                    "field": format!("{PLACEMENT_FIELD}.{PLACEMENT_KIND_FIELD}"),
                    "supportedPlacementKinds": PLACEMENT_KINDS,
                }),
            )
        })?,
    };
    match kind {
        PLACEMENT_KIND_ADD => {
            Ok(parse_placement(project, track_id, clip_id, arguments)?.map(PlacementEdit::Add))
        }
        PLACEMENT_KIND_MOVE => {
            reject_placement_fields(object, PLACEMENT_MOVE_FIELDS, PLACEMENT_KIND_MOVE)?;
            let placement_id = require_placement_id(object, PLACEMENT_KIND_MOVE)?;
            let Some(new_start_tick) = read_optional_u64(object, "startTick")? else {
                return Err(Fault::domain_with_data(
                    ErrorCode::InvalidParameterRange,
                    format!(
                        "`{PLACEMENT_FIELD}` 的 `{PLACEMENT_KIND_MOVE}` 形态必须给出 `startTick` \
                         (它是**目标**起点)"
                    ),
                    serde_json::json!({
                        "reason": "moveRequiresStartTick",
                        "field": format!("{PLACEMENT_FIELD}.startTick"),
                        "placementKind": PLACEMENT_KIND_MOVE,
                    }),
                ));
            };
            let found = existing_placement(
                project,
                track_id,
                &placement_id,
                clip_id,
                PLACEMENT_KIND_MOVE,
            )?;
            if found.start_tick == new_start_tick {
                return Err(Fault::domain_with_data(
                    ErrorCode::Conflict,
                    format!("摆放 {placement_id} 的起点已经是 {new_start_tick}, 没有可提交的改动"),
                    serde_json::json!({
                        "reason": "placementAlreadyAtStartTick",
                        "placementKind": PLACEMENT_KIND_MOVE,
                        "trackId": track_id.to_canonical_string(),
                        "placementId": placement_id.to_canonical_string(),
                        "startTick": new_start_tick,
                        "hint": "幂等重放请用 `idempotencyKey`; 要挪到别处就给不同的 `startTick`",
                    }),
                ));
            }
            Ok(Some(PlacementEdit::Move {
                placement_id,
                previous_start_tick: found.start_tick,
                new_start_tick,
            }))
        }
        PLACEMENT_KIND_REMOVE => {
            reject_placement_fields(object, PLACEMENT_REMOVE_FIELDS, PLACEMENT_KIND_REMOVE)?;
            let placement_id = require_placement_id(object, PLACEMENT_KIND_REMOVE)?;
            let found = existing_placement(
                project,
                track_id,
                &placement_id,
                clip_id,
                PLACEMENT_KIND_REMOVE,
            )?;
            Ok(Some(PlacementEdit::Remove {
                placement_id,
                previous_placement: found,
            }))
        }
        other => Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{PLACEMENT_FIELD}.{PLACEMENT_KIND_FIELD}` 不支持 `{other}`"),
            serde_json::json!({
                "reason": "unknownPlacementKind",
                "field": format!("{PLACEMENT_FIELD}.{PLACEMENT_KIND_FIELD}"),
                "value": other,
                "supportedPlacementKinds": PLACEMENT_KINDS,
                "hint": "缺省 `kind` 等价于 `add`; 未知形态响亮拒绝, 不静默按 add 处理",
            }),
        )),
    }
}

/// 解析 `placement` 的 **`add` 形态**（缺省 = 对象不在场 ⇒ `None` ⇒ 不摆放 =
/// 逐字节等于旧行为）。
///
/// ```json
/// {"placement": {"startTick": 0, "durationTicks": 3840,
///                "placementId": "<可选 26 字符 ULID>", "muted": false}}
/// ```
///
/// 四个内容键全部**可选**：`startTick` 缺省 0；`durationTicks` 缺省由片段内容推导；
/// `placementId` 缺省由 [`placement_label`] 确定性派生；`muted` 缺省 `false`。
/// `kind` 可写可不写；写了必须是 [`PLACEMENT_KIND_ADD`]（其余形态由
/// [`parse_placement_edit`] 分发，不走本函数）。
///
/// 三条刻意设成**响亮失败**的口径（绝不静默降级）：
///
/// | 情形 | 结果 |
/// | :--- | :--- |
/// | `placement` 对象里有 [`PLACEMENT_ADD_FIELDS`] 之外的键 | `INVALID_PARAMETER_RANGE`（`reason = "unknownPlacementField"`，列出支持集合） |
/// | `durationTicks` 缺省、而片段**推不出**长度（非 MIDI 片段 / 空 MIDI 片段） | `INVALID_PARAMETER_RANGE`（`reason = "durationNotDerivable"`）—— 不猜一个假长度 |
/// | 目标音轨上**已经有**这个摆放身份 | `CONFLICT`（逐字段相同 ⇒ `reason = "placementAlreadyExists"`；内容不同 ⇒ `reason = "placementIdConflict"`） |
///
/// # Errors
///
/// - `placement` 不是对象 / 键类型不对 / `placementId` 不是 ULID → `INVALID_PARAMETER_RANGE`；
/// - 音轨不存在 → `TRACK_NOT_FOUND`；片段不存在 → `CLIP_NOT_FOUND`；
/// - `durationTicks == 0` → `INVALID_PARAMETER_RANGE`（模型拒绝零时值的摆放）；
/// - 摆放身份已被占用 → `CONFLICT`；
/// - 模型层 [`ClipPlacement::validate`] 失败 → [`from_model`] 给出的契约码。
pub fn parse_placement(
    project: &YebanProjectV1,
    track_id: &EntityId,
    clip_id: &EntityId,
    arguments: &Map<String, Value>,
) -> Result<Option<ClipPlacement>, Fault> {
    let Some(raw) = arguments.get(PLACEMENT_FIELD) else {
        return Ok(None);
    };
    let object = raw.as_object().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{PLACEMENT_FIELD}` 必须是对象, 实际收到 {raw}"),
        )
    })?;
    reject_placement_fields(object, PLACEMENT_ADD_FIELDS, PLACEMENT_KIND_ADD)?;
    // 目标音轨与片段都必须**真的存在**：摆放是"把已有材料放到已有轨道上"，
    // 两个端点缺一个都不是一次摆放（`compile` 的编辑路径有同一对前置条件）。
    //
    // ⚠ 音轨句柄**只查一次**并留到函数末尾（摆放身份的占用判定要读它的 `clips`）：
    // 同一处检查写两遍时，删掉前一处**没有任何判据会变红**（实测：注入后全绿）
    // —— 那种守卫是"看起来在守"的装饰，不留。
    let track = project
        .track(track_id)
        .map_err(|error| from_model("摆放的目标音轨", &error))?;
    let entry = project
        .clip_pool
        .get(clip_id)
        .ok_or_else(|| Fault::domain(ErrorCode::ClipNotFound, format!("片段不存在: {clip_id}")))?;

    let start_tick = read_optional_u64(object, "startTick")?.unwrap_or(0);
    let duration_ticks = match read_optional_u64(object, "durationTicks")? {
        Some(0) => {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                "`placement.durationTicks` 必须 >= 1 (模型层拒绝零时值的摆放)",
                serde_json::json!({
                    "field": format!("{PLACEMENT_FIELD}.durationTicks"),
                    "value": 0,
                }),
            ));
        }
        Some(value) => value,
        // 缺省 = 片段内容自己的长度（MIDI 片段 = 最后一个音符的结束 tick）。
        // **推不出就不猜**：非 MIDI 片段（音频片段由 `yeban_import_audio` 的
        // `durationTicks` 承载，那里的时值来自素材帧数）与空 MIDI 片段都必须显式给。
        None => match entry.content.notes().and_then(|notes| {
            notes
                .values()
                .map(|note| note.start_tick.saturating_add(note.duration_ticks))
                .max()
        }) {
            Some(extent) if extent > 0 => extent,
            _ => {
                return Err(Fault::domain_with_data(
                    ErrorCode::InvalidParameterRange,
                    "这个片段推不出长度 (非 MIDI 片段或空 MIDI 片段) ⇒ 必须显式给出 \
                     `placement.durationTicks`",
                    serde_json::json!({
                        "reason": "durationNotDerivable",
                        "field": format!("{PLACEMENT_FIELD}.durationTicks"),
                        "clipId": clip_id.to_canonical_string(),
                        "isMidi": entry.content.notes().is_some(),
                    }),
                ));
            }
        },
    };
    let placement_id = match object.get("placementId") {
        Some(value) => {
            let text = value.as_str().ok_or_else(|| {
                Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    "`placement.placementId` 必须是 26 字符 ULID 字符串",
                )
            })?;
            EntityId::from_str(text).map_err(|error| {
                Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    format!("`placement.placementId` 不是合法 ULID: {error}"),
                )
            })?
        }
        None => deterministic_id(&placement_label(
            &clip_id.to_canonical_string(),
            &track_id.to_canonical_string(),
            start_tick,
        )),
    };
    let placement = ClipPlacement {
        id: placement_id,
        clip_id: *clip_id,
        start_tick,
        duration_ticks,
        // 循环配置是模型的**必需**子结构 [ADR-0001 D43]：缺省 = 关闭（不重复）。
        // "循环重复"不在渲染的已支持面里（响应 `unsupported: clipLoopRepetition`），
        // 因此这里刻意不暴露 `loopEnabled` —— 那会给出一个渲染不了的旋钮
        // （与 `yeban_import_audio` 同一个理由）。
        loop_config: LoopConfig::default(),
        muted: read_optional_bool(object, "muted")?.unwrap_or(false),
    };
    placement
        .validate()
        .map_err(|error| from_model("摆放校验", &error))?;
    // 摆放身份在**目标音轨**的 `clips` 里必须还没有被占用：`AddClipPlacement`
    // 的前置条件拒绝重复身份，在这里先判一次才能给出 `field`/`reason` 的结构化 `data`
    // （与 `compile_create` 的 `clipAlreadyExists` 同一口径）。
    match track.clips.get(&placement.id) {
        None => {}
        Some(existing) if *existing == placement => {
            return Err(Fault::domain_with_data(
                ErrorCode::Conflict,
                format!(
                    "音轨 {track_id} 上已经有这条摆放 {} (逐字段相同), 没有可提交的改动",
                    placement.id
                ),
                serde_json::json!({
                    "reason": "placementAlreadyExists",
                    "trackId": track_id.to_canonical_string(),
                    "placementId": placement.id.to_canonical_string(),
                    "hint": "幂等重放请用 `idempotencyKey`; 要挪位置就换 `placement.startTick`",
                }),
            ));
        }
        Some(existing) => {
            return Err(Fault::domain_with_data(
                ErrorCode::Conflict,
                format!(
                    "音轨 {track_id} 上已有摆放身份 {}, 但内容不同",
                    placement.id
                ),
                serde_json::json!({
                    "reason": "placementIdConflict",
                    "trackId": track_id.to_canonical_string(),
                    "placementId": placement.id.to_canonical_string(),
                    "existing": serde_json::to_value(existing).unwrap_or(Value::Null),
                    "requested": serde_json::to_value(placement).unwrap_or(Value::Null),
                    "hint": "换一个 `placement.placementId`/`startTick`, 或给出逐字段相同的载荷",
                }),
            ));
        }
    }
    Ok(Some(placement))
}

/// 拒绝 `placement` 对象里 `allowed` 之外的键（与 [`reject_unknown_note_fields`]
/// 同一口径：拼错/不支持的键一律响亮拒绝，绝不静默丢弃）。
///
/// 两种"不在 `allowed` 里"被**分开**报（`kind` 由 `kind` 参数如实带出）：
///
/// | 情形 | `reason` |
/// | :--- | :--- |
/// | 键不在**任何**形态的词表里（拼错 / 根本不支持，例如 `loopEnabled`） | `unknownPlacementField` |
/// | 键在别的形态里合法、但**本**形态不适用（例如 `move` 里的 `muted`） | `placementFieldNotApplicable` |
///
/// 分开的理由：把"这个形态改不了静音"报成"静音不是一个键"会让调用方去猜一个不存在的
/// 替代写法。
///
/// 键序是确定性的（`serde_json::Map` 在本 crate 的 feature 集合下是 `BTreeMap`），
/// 因此同一个非法载荷每次报的是**同一个** `field` —— 判据可以逐字钉住它。
///
/// # Errors
///
/// 出现不属于 `allowed` 的键 ⇒ `INVALID_PARAMETER_RANGE`，`data` 带 `field`
/// （第一个这样的键）、`reason`、`placementKind`、`supportedPlacementFields`
/// （本形态的 `allowed`）与 `hint`。
fn reject_placement_fields(
    object: &Map<String, Value>,
    allowed: &[&str],
    kind: &str,
) -> Result<(), Fault> {
    let Some(unknown) = object.keys().find(|key| !allowed.contains(&key.as_str())) else {
        return Ok(());
    };
    let known_in_another_shape =
        !allowed.contains(&unknown.as_str()) && PLACEMENT_ADD_FIELDS.contains(&unknown.as_str());
    let reason = if known_in_another_shape {
        "placementFieldNotApplicable"
    } else {
        "unknownPlacementField"
    };
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{PLACEMENT_FIELD}` 的 `{kind}` 形态不接受字段 `{unknown}` \
             (不是可选项缺失, 而是拼写/不支持/本形态不适用)"
        ),
        serde_json::json!({
            "reason": reason,
            "field": unknown,
            "placementKind": kind,
            "supportedPlacementFields": allowed,
            "hint": "未知键不静默忽略: 去掉它, 或改用 supportedPlacementFields 里的字段",
        }),
    ))
}

/// 读 `placement.placementId`（**必填** ULID；缺失 / 非字符串 / 不是 ULID 都拒绝）。
///
/// `move` / `remove` 两个形态都要指名一条**已经存在**的摆放，因此身份是必填的 ——
/// 缺省派生（[`placement_label`]）**只**属于 `add` 形态：派生一条新身份不可能命中
/// 一条已有摆放。
///
/// # Errors
///
/// 缺失 ⇒ `INVALID_PARAMETER_RANGE`（`reason = "{kind}RequiresPlacementId"`）；
/// 形状不对 ⇒ `INVALID_PARAMETER_RANGE`。
fn require_placement_id(object: &Map<String, Value>, kind: &str) -> Result<EntityId, Fault> {
    let Some(value) = object.get("placementId") else {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{PLACEMENT_FIELD}` 的 `{kind}` 形态必须给出 `placementId`"),
            serde_json::json!({
                "reason": format!("{kind}RequiresPlacementId"),
                "field": format!("{PLACEMENT_FIELD}.placementId"),
                "placementKind": kind,
                "hint": "`placementId` 缺省派生只属于 `add` 形态; `move`/`remove` 必须指名已有摆放",
            }),
        ));
    };
    let text = value.as_str().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`placement.placementId` 必须是 26 字符 ULID 字符串",
        )
    })?;
    EntityId::from_str(text).map_err(|error| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`placement.placementId` 不是合法 ULID: {error}"),
        )
    })
}

/// 在目标音轨上找出 `placement_id` 这条**已经存在**的摆放，并核对调用方报的 `clip_id`。
///
/// `clipId` 是 `yeban_edit_notes` 的**必填**实参（`§7.2` 的参数表），因此 `move` /
/// `remove` 也要求调用方把它写出来；它与文档里那条摆放的 `clip_id` **必须一致** ——
/// 不一致说明调用方手上的摆放和文档里的不是同一条，这时**不**猜、也**不**静默改用文档
/// 里的那一条，而是响亮失败（`reason = "placementClipMismatch"`）。
///
/// # Errors
///
/// - 音轨不存在 ⇒ `TRACK_NOT_FOUND`；
/// - 该音轨上没有这条摆放 ⇒ `ENTITY_NOT_FOUND`（`reason = "placementNotFound"`）；
/// - `clipId` 与文档不一致 ⇒ `INVALID_PARAMETER_RANGE`（`reason = "placementClipMismatch"`）。
fn existing_placement(
    project: &YebanProjectV1,
    track_id: &EntityId,
    placement_id: &EntityId,
    clip_id: &EntityId,
    kind: &str,
) -> Result<ClipPlacement, Fault> {
    let track = project
        .track(track_id)
        .map_err(|error| from_model("摆放编辑的目标音轨", &error))?;
    let Some(found) = track.clips.get(placement_id) else {
        return Err(Fault::domain_with_data(
            ErrorCode::EntityNotFound,
            format!("音轨 {track_id} 上没有摆放 {placement_id}"),
            serde_json::json!({
                "reason": "placementNotFound",
                "placementKind": kind,
                "trackId": track_id.to_canonical_string(),
                "placementId": placement_id.to_canonical_string(),
                "placementCount": track.clips.len(),
                "hint": "`add` 形态才是新建; `move`/`remove` 只能作用在**已有**的摆放上",
            }),
        ));
    };
    if found.clip_id != *clip_id {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "摆放 {placement_id} 引用的是片段 {}, 不是调用方给出的 `clipId` {clip_id}",
                found.clip_id
            ),
            serde_json::json!({
                "reason": "placementClipMismatch",
                "placementKind": kind,
                "trackId": track_id.to_canonical_string(),
                "placementId": placement_id.to_canonical_string(),
                "clipId": clip_id.to_canonical_string(),
                "placementClipId": found.clip_id.to_canonical_string(),
            }),
        ));
    }
    Ok(*found)
}

/// 读一个**可选**的非负整数键（缺省 = `None`；负数、非整数、溢出都拒绝）。
///
/// 与 `import_audio` 的 `parse_optional_u64` 同一条口径（那里读的是顶层实参，名字不带
/// `placement.` 前缀，因此错误的措辞不同）。它只做 **JSON 形状**读取，不携带领域语义 ——
/// "时值必须非零""音轨必须存在"这类裁决分别在 [`parse_placement`] 与模型层。
fn read_optional_u64(object: &Map<String, Value>, field: &str) -> Result<Option<u64>, Fault> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    value.as_u64().map(Some).ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{PLACEMENT_FIELD}.{field}` 必须是非负整数, 实际收到 {value}"),
        )
    })
}

/// 读一个**可选**的布尔键（缺省 = `None`；非布尔拒绝）。
fn read_optional_bool(object: &Map<String, Value>, field: &str) -> Result<Option<bool>, Fault> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    value.as_bool().map(Some).ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{PLACEMENT_FIELD}.{field}` 必须是布尔值, 实际收到 {value}"),
        )
    })
}

/// 峰值同时发声数（在 `[start, start + duration)` 上的最大重叠）。
#[must_use]
pub fn peak_polyphony(notes: impl IntoIterator<Item = (u64, u64)>) -> usize {
    let mut events: Vec<(u64, i64)> = Vec::new();
    for (start, duration) in notes {
        events.push((start, 1));
        events.push((start.saturating_add(duration), -1));
    }
    // 同一 tick 上"结束"排在"开始"之前: 首尾相接的两个音符不算重叠。
    events.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
    let mut current = 0_i64;
    let mut peak = 0_i64;
    for (_, delta) in events {
        current += delta;
        peak = peak.max(current);
    }
    usize::try_from(peak).unwrap_or(usize::MAX)
}

/// 检查某片段在施加 `ops` **之后**的发声数（在克隆体上模拟，不改原文档）。
///
/// 这条路径**同时覆盖**两个形态：`ops` 是编辑操作时它读的是既有片段的新状态；
/// `ops` 是 [`compile_create`] 的那一条 `Op::AddClip` 时，模拟里新片段已经存在
/// ⇒ 读到的就是**新片段**的峰值。因此材料创建不需要第二份发声数检查。
///
/// # Errors
///
/// - 模拟时模型层失败 → [`from_model`] 给出的契约码；
/// - 超过 [`MAX_POLYPHONY`] → `OUT_OF_RANGE`（带 `peak` / `limit` / `clipId`）。
pub fn check_polyphony(
    project: &YebanProjectV1,
    clip_id: &EntityId,
    ops: &[Op],
) -> Result<usize, Fault> {
    let mut simulated = project.clone();
    for op in ops {
        op.apply(&mut simulated)
            .map_err(|error| from_model("音符操作模拟", &error))?;
    }
    let notes = simulated
        .clip_pool
        .get(clip_id)
        .and_then(|entry| entry.content.notes())
        .map(|notes| {
            notes
                .values()
                .map(|note| (note.start_tick, note.duration_ticks))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let peak = peak_polyphony(notes);
    if peak > MAX_POLYPHONY {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("片段 {clip_id} 的发声数峰值 {peak} 超过上限 {MAX_POLYPHONY}"),
            serde_json::json!({ "clipId": clip_id.to_canonical_string(), "peak": peak, "limit": MAX_POLYPHONY }),
        ));
    }
    Ok(peak)
}

/// 缺字段的统一错误。
fn missing(field: &str, expected: &str) -> Fault {
    Fault::domain(
        ErrorCode::InvalidParameterRange,
        format!("缺少 `{field}`（期望 {expected}）"),
    )
}

/// 读 `0..=max` 的整数。
fn read_range(object: &Map<String, Value>, field: &str, min: u8, max: u8) -> Result<u8, Fault> {
    let value = object.get(field).ok_or_else(|| missing(field, "整数"))?;
    let number = value.as_i64().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是整数, 实际收到 {value}"),
        )
    })?;
    if !(i64::from(min)..=i64::from(max)).contains(&number) {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("`{field}` 越界: {number} 不在 {min}..={max}"),
            serde_json::json!({ "field": field, "value": number, "min": min, "max": max }),
        ));
    }
    Ok(u8::try_from(number).unwrap_or(max))
}

/// 读非负整数。
fn read_u64(object: &Map<String, Value>, field: &str) -> Result<u64, Fault> {
    let value = object
        .get(field)
        .ok_or_else(|| missing(field, "非负整数"))?;
    value.as_u64().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是非负整数, 实际收到 {value}"),
        )
    })
}

/// 读一个非负整数并收窄到 `usize`（下标类字段专用）。
///
/// 与 [`read_u64`] 的差别只有收窄那一步：`u64` 在 32 位平台上装不进 `usize`，
/// 静默截断会把 `slotIndex: 4294967296` 变成槽 0（写到**另一台设备**上）。
/// 越界因此是响亮失败（`INVALID_PARAMETER_RANGE`，`reason = "indexTooLarge"`）。
fn read_usize(object: &Map<String, Value>, field: &str) -> Result<usize, Fault> {
    let value = read_u64(object, field)?;
    usize::try_from(value).map_err(|_| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 超出本机 usize 表示范围: {value}"),
            serde_json::json!({
                "field": field,
                "reason": "indexTooLarge",
                "value": value,
            }),
        )
    })
}

/// 读 `i64`。
fn read_i64(object: &Map<String, Value>, field: &str) -> Result<i64, Fault> {
    let value = object.get(field).ok_or_else(|| missing(field, "整数"))?;
    value.as_i64().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是整数, 实际收到 {value}"),
        )
    })
}

/// 读 `i8`。
fn read_i8(object: &Map<String, Value>, field: &str) -> Result<i8, Fault> {
    let number = read_i64(object, field)?;
    i8::try_from(number).map_err(|_| {
        Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("`{field}` 越界: {number} 不在 -128..=127"),
            serde_json::json!({ "field": field, "value": number }),
        )
    })
}

/// 读 `EntityId`。
fn read_id(object: &Map<String, Value>, field: &str) -> Result<EntityId, Fault> {
    let value = object
        .get(field)
        .ok_or_else(|| missing(field, "26 字符 ULID 字符串"))?;
    let text = value.as_str().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是字符串, 实际收到 {value}"),
        )
    })?;
    EntityId::from_str(text).map_err(|error| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 不是合法 ULID: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_model::AutomationPoint;
    use yeban_model::samples::filled_project;

    fn lead_clip(project: &YebanProjectV1) -> (EntityId, EntityId) {
        let clip = project
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_some())
            .expect("样本里必须有 MIDI 片段");
        let track = project
            .tracks
            .values()
            .find(|track| track.kind == yeban_model::TrackKind::Midi)
            .expect("样本里必须有 MIDI 音轨");
        (track.id, clip.id)
    }

    /// 取一个领域失败的 `data`（判据只关心结构化补充，不关心人话信息）。
    fn lane_fault_data(fault: &Fault) -> &Value {
        match fault {
            Fault::Domain { data, .. } => data.as_ref().expect("本形态的失败必须带 data"),
            Fault::Impl { error } => panic!("应当是领域失败, 实际是 {error:?}"),
        }
    }

    #[test]
    fn parse_rejects_unknown_kind_and_bad_shapes() {
        for broken in [
            serde_json::json!([]),
            serde_json::json!([{"kind": "explode"}]),
            serde_json::json!([{"kind": "delete"}]),
            serde_json::json!([{"kind": "move", "noteId": "x", "deltaTick": 1, "deltaPitch": 0}]),
            serde_json::json!("not an array"),
        ] {
            let fault = parse_ops(&broken).expect_err("必须被拒");
            assert!(
                matches!(
                    fault.domain_code(),
                    Some(ErrorCode::InvalidParameterRange | ErrorCode::OutOfRange)
                ),
                "{broken} 得到了 {:?}",
                fault.domain_code()
            );
        }
    }

    #[test]
    fn parse_rejects_out_of_range_pitch_and_velocity() {
        let fault = parse_ops(&serde_json::json!([{
            "kind": "add",
            "note": {"startTick": 0, "pitch": 128, "durationTicks": 480}
        }]))
        .expect_err("音高越界");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));

        let fault = parse_ops(&serde_json::json!([{
            "kind": "velocity", "noteId": "01J8ZQ00000000000000000001", "velocity": 200
        }]))
        .expect_err("力度越界");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));

        let fault = parse_ops(&serde_json::json!([{
            "kind": "add",
            "note": {"startTick": 0, "pitch": 60, "durationTicks": 0}
        }]))
        .expect_err("零时值");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
    }

    #[test]
    fn compile_reads_the_undo_payload_from_the_document() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let note_id = project.clip_pool[&clip_id]
            .content
            .notes()
            .expect("MIDI")
            .keys()
            .next()
            .copied()
            .expect("至少一个音符");
        let ops =
            compile(&project, &track_id, &clip_id, &[NoteOp::Delete { note_id }]).expect("编译");
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            Op::DeleteNote { previous_note, .. } => {
                assert_eq!(
                    previous_note,
                    project.note(&clip_id, &note_id).expect("音符"),
                    "撤销载荷必须来自当前文档"
                );
            }
            other => panic!("应当是 DeleteNote: {other:?}"),
        }
    }

    #[test]
    fn compile_reports_missing_note_as_entity_not_found() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let ghost = deterministic_id("ghost-note");
        let fault = compile(
            &project,
            &track_id,
            &clip_id,
            &[NoteOp::Velocity {
                note_id: ghost,
                velocity: 1,
            }],
        )
        .expect_err("音符不存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::EntityNotFound));
    }

    /// `ops[].kind == "setParam"` 的**规范**形状：解析 → 编译 → 真的改工程 → 逆操作回原。
    ///
    /// 这一条是"工具面写不了静态混音值"缺口的**字面**判据：它钉住
    /// `old_val` 来自当前文档（不是调用方声明）、`new_val` 是 `Op::SetParam` 的载荷、
    /// 且 `Op::invert` 能逐字节回退。
    #[test]
    fn set_param_compiles_against_the_document_and_inverts_byte_for_byte() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let volume_before = project.track(&track_id).expect("音轨").volume_db;
        let pan_before = project.track(&track_id).expect("音轨").pan;

        let ops = parse_ops(&serde_json::json!([
            {"kind": "setParam", "lane": "TrackVolume", "value": -9.5},
            {"kind": "setParam", "lane": "TrackPan", "value": 0.75}
        ]))
        .expect("规范形状必须被接受");
        assert!(ops.iter().all(|op| !op.is_note_level()), "两条都是音轨级");
        assert_eq!(ops[0].kind_name(), SET_PARAM_KIND);

        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 2);
        match &compiled[0] {
            Op::SetParam {
                target:
                    AutomationTarget::TrackVolume {
                        track_id: target_track,
                    },
                old_val,
                new_val,
            } => {
                assert_eq!(*target_track, track_id);
                assert_eq!(*old_val, volume_before, "撤销载荷必须来自当前文档");
                assert_eq!(*new_val, -9.5);
            }
            other => panic!("应当是 TrackVolume 的 SetParam: {other:?}"),
        }

        Op::Batch {
            ops: compiled.clone(),
            description: "setParam".to_owned(),
        }
        .apply(&mut project)
        .expect("施加");
        assert_eq!(project.track(&track_id).expect("音轨").volume_db, -9.5);
        assert_eq!(project.track(&track_id).expect("音轨").pan, 0.75);

        for op in compiled.iter().rev() {
            op.apply_inverse(&mut project).expect("逆操作");
        }
        let restored = project.track(&track_id).expect("音轨");
        assert_eq!(restored.volume_db, volume_before, "音量必须逐字节回原值");
        assert_eq!(restored.pan, pan_before, "声相必须逐字节回原值");
    }

    /// 声相值域**不在本层**：越界的 `value` 在模型自己的 `validate_param_value` 处
    /// 变成 `PanOutOfRange`（契约码 `OUT_OF_RANGE`），本层只搬字面值。
    #[test]
    fn set_param_pan_range_is_judged_by_the_model_not_by_this_layer() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        // 本层必须**接受**这个形状（-1.0..=1.0 的判定不属于它）。
        let ops = parse_ops(&serde_json::json!([
            {"kind": "setParam", "lane": "TrackPan", "value": 2.0}
        ]))
        .expect("本层不做值域判定");
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        let mut simulated = project.clone();
        let failure = compiled[0]
            .apply(&mut simulated)
            .expect_err("模型必须拒绝越界声相");
        assert_eq!(
            super::super::error::code_for_model(&failure),
            ErrorCode::OutOfRange
        );
    }

    /// 另外三个自动化目标名与别名都是**响亮失败**：不静默回退、不猜。
    #[test]
    fn set_param_rejects_non_static_lanes_and_aliases() {
        for (payload, expected_reason) in [
            (
                serde_json::json!([{"kind": "setParam", "lane": "SendGain", "value": 0.0}]),
                Some("staticLaneNotApplicable"),
            ),
            (
                serde_json::json!([{"kind": "setParam", "lane": "DeviceParam", "value": 0.0}]),
                Some("staticLaneNotApplicable"),
            ),
            (
                serde_json::json!([{"kind": "setParam", "lane": "Macro", "value": 0.0}]),
                Some("staticLaneNotApplicable"),
            ),
            (
                serde_json::json!([{"kind": "setParam", "lane": "trackVolume", "value": 0.0}]),
                Some("unknownStaticLane"),
            ),
            // 缺 `value` / `value` 不是数字 / `lane` 不是字符串 / 缺 `lane`。
            (
                serde_json::json!([{"kind": "setParam", "lane": "TrackVolume"}]),
                None,
            ),
            (
                serde_json::json!([{"kind": "setParam", "lane": "TrackVolume", "value": "loud"}]),
                Some("valueMustBeNumber"),
            ),
            (
                serde_json::json!([{"kind": "setParam", "lane": 3, "value": 0.0}]),
                Some("laneMustBeString"),
            ),
            (
                serde_json::json!([{"kind": "setParam", "value": 0.0}]),
                None,
            ),
            // 有限 `f64` 收窄到 `f32` 会溢出成 `inf` ⇒ 这一条**可达**（不是摆设）。
            (
                serde_json::json!([{"kind": "setParam", "lane": "TrackVolume", "value": 1e300}]),
                Some("nonFiniteValue"),
            ),
        ] {
            let fault = parse_ops(&payload).expect_err(&format!("必须被拒: {payload}"));
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{payload}"
            );
            let Fault::Domain { data, .. } = &fault else {
                panic!("必须是领域失败: {payload}");
            };
            let Some(reason) = expected_reason else {
                assert!(data.is_none(), "缺字段错误不该带 data: {payload}");
                continue;
            };
            let data = data.clone().expect("形状错误必须带 data");
            assert_eq!(data["reason"], reason, "{payload} 的 data: {data}");
            if reason.contains("Lane") {
                assert_eq!(
                    data["allowed"],
                    serde_json::json!(StaticLane::NAMES),
                    "{payload} 必须报出允许集合"
                );
            }
        }
    }

    /// 纯 `setParam` 调用**不要求**片段是 MIDI（它一个音符都不读）；
    /// 同一片段上的音符操作**仍然**要求 MIDI（旧行为逐字节不变）。
    #[test]
    fn set_param_alone_does_not_require_a_midi_clip() {
        let project = filled_project();
        let audio_clip = project
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_none())
            .expect("样本里必须有非 MIDI 片段");
        let track_id = project
            .tracks
            .values()
            .find(|track| track.kind == yeban_model::TrackKind::Audio)
            .expect("样本里必须有音频轨")
            .id;
        let clip_id = audio_clip.id;

        let ops = parse_ops(&serde_json::json!([
            {"kind": "setParam", "lane": "TrackVolume", "value": -12.0}
        ]))
        .expect("解析");
        compile(&project, &track_id, &clip_id, &ops).expect("纯静态写入不要求 MIDI 材料");

        let note_ops = parse_ops(&serde_json::json!([
            {"kind": "add", "note": {"startTick": 0, "pitch": 60, "durationTicks": 480}}
        ]))
        .expect("解析");
        let fault = compile(&project, &track_id, &clip_id, &note_ops)
            .expect_err("音符操作仍然要求 MIDI 材料");
        assert_eq!(fault.domain_code(), Some(ErrorCode::ClipNotFound));
    }

    /// `ops[].kind == "setTrackMute"` / `"setTrackSolo"` 的**规范**形状：
    /// 解析 → 编译 → 真的改工程 → 逆操作回原。
    ///
    /// 这一条是"工具面写不了静音 / 独奏"缺口的**字面**判据：它钉住
    /// `old_mute` / `old_solo` 来自当前文档（不是调用方声明）、`new_*` 是模型变体的载荷、
    /// 且 `Op::invert` 能逐字节回退。
    ///
    /// 文档先被推到"两个开关**已经打开**"再写 `false`：这条安排是判据的**牙齿** ——
    /// 文档值恰好等于注入常量时，"把 `old_*` 写死成常量"的注入会全绿（实测过一次）。
    ///
    /// 注入（实测红）：删掉 `parse_one` 的两个分支 ⇒ 未知 `kind`；把
    /// [`TrackFlag::read`] 写死成常量 ⇒ 这里报 `old_mute` 不是 `true`。
    #[test]
    fn track_flags_compile_against_the_document_and_invert_byte_for_byte() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        {
            let track = project.track_mut(&track_id).expect("音轨");
            track.mute = true;
            track.solo = true;
        }
        let track_before = project.track(&track_id).expect("音轨").clone();
        let mute_before = track_before.mute;
        let solo_before = track_before.solo;
        assert!(mute_before && solo_before, "夹具前提: 两个开关先是打开的");

        let ops = parse_ops(&serde_json::json!([
            {"kind": "setTrackMute", "value": false},
            {"kind": "setTrackSolo", "value": false}
        ]))
        .expect("规范形状必须被接受");
        assert!(ops.iter().all(|op| !op.is_note_level()), "两条都是音轨级");
        assert_eq!(ops[0].kind_name(), SET_TRACK_MUTE_KIND);
        assert_eq!(ops[1].kind_name(), SET_TRACK_SOLO_KIND);

        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 2);
        match &compiled[0] {
            Op::SetTrackMute {
                track_id: target,
                old_mute,
                new_mute,
            } => {
                assert_eq!(*target, track_id);
                assert_eq!(*old_mute, mute_before, "撤销载荷必须来自当前文档");
                assert!(!*new_mute, "目标态是调用方给的 false");
            }
            other => panic!("应当是 SetTrackMute: {other:?}"),
        }
        match &compiled[1] {
            Op::SetTrackSolo {
                track_id: target,
                old_solo,
                new_solo,
            } => {
                assert_eq!(*target, track_id);
                assert_eq!(*old_solo, solo_before, "撤销载荷必须来自当前文档");
                assert!(!*new_solo);
            }
            other => panic!("应当是 SetTrackSolo: {other:?}"),
        }

        Op::Batch {
            ops: compiled.clone(),
            description: "track flags".to_owned(),
        }
        .apply(&mut project)
        .expect("施加");
        let muted = project.track(&track_id).expect("音轨");
        assert!(!muted.mute, "合并后静音必须真的关掉");
        assert!(!muted.solo, "合并后独奏必须真的关掉");
        assert_eq!(muted.volume_db, track_before.volume_db, "开关不碰音量");
        assert_eq!(muted.pan, track_before.pan, "开关不碰声相");
        assert_eq!(
            muted.solo_safe, track_before.solo_safe,
            "`Op::SetTrackSolo` 不顺手改 `solo_safe`"
        );

        for op in compiled.iter().rev() {
            op.apply_inverse(&mut project).expect("逆操作");
        }
        assert_eq!(
            project.track(&track_id).expect("音轨"),
            &track_before,
            "逆操作必须逐字段回到原音轨"
        );
    }

    /// 开关的**形状**错误全部响亮失败：非布尔值、缺字段、多写的键（含嵌套 `trackId`）。
    ///
    /// 注入：把 `raw.as_bool()` 换成 `raw.as_bool().unwrap_or(false)` ⇒ 前两条不再红。
    #[test]
    fn track_flags_reject_non_boolean_values_and_unknown_keys() {
        // 非布尔值。
        for payload in [
            serde_json::json!([{"kind": "setTrackMute", "value": 1}]),
            serde_json::json!([{"kind": "setTrackSolo", "value": "true"}]),
            serde_json::json!([{"kind": "setTrackMute", "value": null}]),
            serde_json::json!([{"kind": "setTrackSolo", "value": ["yes"]}]),
        ] {
            let fault = parse_ops(&payload).expect_err(&format!("必须被拒: {payload}"));
            assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
            let value = fault.into_result().expect("带内");
            assert_eq!(
                value["error"]["data"]["reason"], "valueMustBeBoolean",
                "{payload}"
            );
        }

        // 缺 `value`：统一的缺字段错误（不带 data）。
        let fault = parse_ops(&serde_json::json!([{"kind": "setTrackMute"}])).expect_err("缺字段");
        let Fault::Domain { data, .. } = &fault else {
            panic!("必须是领域失败");
        };
        assert!(data.is_none(), "缺字段错误不该带 data: {data:?}");

        // 开关对象里 `kind` / `value` 之外的键：**响亮失败**，并指出目标音轨的
        // 正确位置是顶层 `trackId`（嵌套写它会被静默忽略 ⇒ 开关落到别的音轨上）。
        for payload in [
            serde_json::json!([{"kind": "setTrackMute", "value": true,
                               "trackId": "01ARZ3NDEKTSV4RRFFQ69G5FAV"}]),
            serde_json::json!([{"kind": "setTrackSolo", "value": false, "lane": "TrackVolume"}]),
        ] {
            let fault = parse_ops(&payload).expect_err(&format!("必须被拒: {payload}"));
            assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
            let value = fault.into_result().expect("带内");
            assert_eq!(
                value["error"]["data"]["reason"], "unknownFlagField",
                "{payload}"
            );
            assert_eq!(
                value["error"]["data"]["supportedFlagFields"],
                serde_json::json!(TRACK_FLAG_FIELDS),
                "{payload}"
            );
            assert!(
                value["error"]["data"]["hint"]
                    .as_str()
                    .is_some_and(|hint| hint.contains("trackId")),
                "必须指出目标音轨在顶层: {payload}"
            );
        }

        // 猜一个更短的名字（`setMute`）不是别名，而是**未知 kind**：错误里给出全集。
        let fault = parse_ops(&serde_json::json!([{"kind": "setMute", "value": true}]))
            .expect_err("`setMute` 不是本工具的形态名");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(
            value["error"]["data"]["supportedKinds"],
            serde_json::json!(OP_KINDS),
            "未知 kind 必须报出全集 (含两个新开关)"
        );
    }

    /// 两个开关是**音轨级**的：纯开关调用不要求片段是 MIDI（一个音符都不读），
    /// 而 `create: true` 的形状里它们仍然被响亮拒绝。
    ///
    /// 注入：把 `NoteOp::is_note_level` 改回"只有 `SetParam` 是音轨级" ⇒ 第一条红。
    #[test]
    fn track_flags_alone_do_not_require_a_midi_clip() {
        let project = filled_project();
        let audio_clip = project
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_none())
            .expect("样本里必须有非 MIDI 片段");
        let track_id = project
            .tracks
            .values()
            .find(|track| track.kind == yeban_model::TrackKind::Audio)
            .expect("样本里必须有音频轨")
            .id;

        let ops = parse_ops(&serde_json::json!([
            {"kind": "setTrackMute", "value": true},
            {"kind": "setTrackSolo", "value": false}
        ]))
        .expect("解析");
        let compiled =
            compile(&project, &track_id, &audio_clip.id, &ops).expect("纯开关写入不要求 MIDI 材料");
        assert_eq!(compiled.len(), 2);

        // `kind` 的全集必须真的登记这四个音轨级名字 + 一个池级名字 + 两个路由级名字
        // + 一个段落级名字 + 一个场景级名字（错误信息的 `supportedKinds` 与判据共用
        // 同一份真相）。
        // 2026-10-09：新增 `disconnectRouting` 后全集为 12（裁决 R22，性质不变）；
        // 同日新增 `removeRoutingNode`（第三个路由级形态）后为 13；同日再新增
        // `removeSection`（唯一的段落级形态）后为 14；同日再新增 `removeScene`
        // （唯一的场景级形态）后为 15；本票再新增 `setScene`（场景级**写入**形态，
        // 与 `removeScene` 共用场景级那个桶）后为 16 —— 这是**同步**计数
        // （多了一个真存在的 `kind`），不是弱化判据。
        assert_eq!(OP_KINDS.len(), 16);
        assert_eq!(TrackFlag::NAMES, [SET_TRACK_MUTE_KIND, SET_TRACK_SOLO_KIND]);
        assert!(OP_KINDS.contains(&SET_TRACK_MUTE_KIND));
        assert!(OP_KINDS.contains(&SET_TRACK_SOLO_KIND));
        assert!(OP_KINDS.contains(&SET_AUTOMATION_LANE_KIND));
        assert!(OP_KINDS.contains(&REMOVE_AUTOMATION_POINT_KIND));
        assert!(OP_KINDS.contains(&REMOVE_CLIP_KIND));
        assert!(OP_KINDS.contains(&SET_ROUTING_GAIN_KIND));
        assert!(OP_KINDS.contains(&DISCONNECT_ROUTING_KIND));
        assert!(OP_KINDS.contains(&REMOVE_ROUTING_NODE_KIND));
        assert!(OP_KINDS.contains(&REMOVE_SECTION_KIND));
        assert!(OP_KINDS.contains(&REMOVE_SCENE_KIND));
        assert!(OP_KINDS.contains(&SET_SCENE_KIND));
        assert_eq!(LANE_WRITE_MODES.len(), 4);
    }

    /// 泳道形态的**词表**判据：键名、写模式名、目标名与模型同源。
    ///
    /// 注入（实测红）：往 [`LANE_WRITE_MODES`] 里加一个模型没有的名字 ⇒ 这条红。
    #[test]
    fn lane_field_names_are_pinned() {
        assert_eq!(SET_AUTOMATION_LANE_KIND, "setAutomationLane");
        assert_eq!(SET_AUTOMATION_LANE_FIELD, "lane");
        assert_eq!(
            SET_AUTOMATION_LANE_FIELDS,
            [
                "lane",
                "edgeId",
                "slotIndex",
                "paramIndex",
                "macroIndex",
                "readEnabled",
                "writeMode",
                "domain",
                "remove",
            ]
        );
        // 词表与模型自己的 serde 名字同源（不手写第二张会漂移的表）。
        assert_eq!(LANE_WRITE_MODES, ["Off", "Write", "Touch", "Latch"]);
        for (mode, name) in [
            (AutomationWriteMode::Off, "Off"),
            (AutomationWriteMode::Write, "Write"),
            (AutomationWriteMode::Touch, "Touch"),
            (AutomationWriteMode::Latch, "Latch"),
        ] {
            assert_eq!(write_mode_name(mode), name, "写模式名必须与 serde 名字同源");
        }
        // 目标名借用 `yeban_edit_automation` 的同一份词表。
        assert_eq!(
            super::super::extension_pure::LANE_NAMES,
            [
                "TrackVolume",
                "TrackPan",
                "SendGain",
                "DeviceParam",
                "Macro"
            ]
        );
    }

    /// 泳道形态的**规范**形状：解析 → 编译 → 真的改工程 → 逆操作逐字节回原。
    ///
    /// 这一条是"工具面写不了泳道属性"缺口的**字面**判据。夹具刻意让
    /// `readEnabled = true` / `writeMode = Touch` / `domain = Some(...)`，而调用写的是
    /// `false` / `Latch` / `null`：两边**不相等**，因此"把三个属性写死成常量"或
    /// "把 `old_lane` 写死"的注入都会在这里红。
    ///
    /// 注入（实测红）：删掉 `parse_one` 的 `setAutomationLane` 分支 ⇒ 未知 `kind`；
    /// 把 `LaneChange::Set` 的 `old_lane` 换成 `None` ⇒ 撤销载荷不是文档现值；
    /// 把 `patch.apply_to` 换成"直接用调用方的值"⇒ 保留语义消失（未给的键被重置）。
    #[test]
    fn lane_edits_compile_against_the_document_and_invert_byte_for_byte() {
        let mut project = filled_project();
        let (track_id, _clip_id) = lead_clip(&project);
        let target = AutomationTarget::TrackVolume { track_id };
        let before = project
            .track(&track_id)
            .expect("音轨")
            .automation_lanes
            .get(&target)
            .expect("样本里必须有 TrackVolume 泳道")
            .clone();
        assert!(before.read_enabled, "夹具前提: 读开关先是打开的");
        assert_eq!(before.write_mode, AutomationWriteMode::Touch);
        assert!(before.domain.is_some(), "夹具前提: 有显式取值域覆盖");
        let bytes_before = serde_json::to_string(&project).expect("序列化");

        // (1) 关闭读开关 + 改写模式 + 清掉取值域覆盖；**不**提 `domain` 的兄弟键。
        let ops = parse_ops(&serde_json::json!([
            {"kind": "setAutomationLane", "lane": {
                "lane": "TrackVolume", "readEnabled": false,
                "writeMode": "Latch", "domain": null
            }}
        ]))
        .expect("规范形状必须被接受");
        assert_eq!(ops[0].kind_name(), SET_AUTOMATION_LANE_KIND);
        assert!(!ops[0].is_note_level(), "泳道形态是音轨级");
        let compiled = compile(&project, &track_id, &_clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::SetAutomationLane {
                target: written,
                old_lane,
                new_lane,
            } => {
                assert_eq!(*written, target);
                assert_eq!(old_lane.as_ref(), Some(&before), "撤销载荷必须来自当前文档");
                assert!(!new_lane.read_enabled);
                assert_eq!(new_lane.write_mode, AutomationWriteMode::Latch);
                assert_eq!(new_lane.domain, None, "`domain: null` 是清掉覆盖");
                assert_eq!(new_lane.points, before.points, "属性写入不许碰采样点");
            }
            other => panic!("应当是 SetAutomationLane: {other:?}"),
        }

        // (2) **合并**语义：只给 `readEnabled`，其余两个属性必须保持文档现值。
        let merge = parse_ops(&serde_json::json!([
            {"kind": "setAutomationLane", "lane": {"lane": "TrackVolume", "readEnabled": false}}
        ]))
        .expect("解析");
        let merged = compile(&project, &track_id, &_clip_id, &merge).expect("编译");
        match &merged[0] {
            Op::SetAutomationLane { new_lane, .. } => {
                assert_eq!(
                    new_lane.write_mode, before.write_mode,
                    "没提 `writeMode` 就必须保留文档现值"
                );
                assert_eq!(new_lane.domain, before.domain, "没提 `domain` 就必须保留");
            }
            other => panic!("应当是 SetAutomationLane: {other:?}"),
        }

        Op::Batch {
            ops: compiled.clone(),
            description: "lane edit".to_owned(),
        }
        .apply(&mut project)
        .expect("施加");
        let after = project
            .track(&track_id)
            .expect("音轨")
            .automation_lanes
            .get(&target)
            .expect("泳道仍在")
            .clone();
        assert!(!after.read_enabled, "读开关必须真的关掉");
        // 唯一求值入口真的遵守它（走带/宿主求值因此在这一点退回静态值）。
        assert_eq!(
            project.automation_value_at(&target, 0).expect("求值"),
            None,
            "读关的泳道在唯一求值入口上必须返回「无自动化值」"
        );
        // (3) 写模式与取值域在**同一次调用**里的替换也要能被撤销。
        let latch = compile(
            &project,
            &track_id,
            &_clip_id,
            &parse_ops(&serde_json::json!([
                {"kind": "setAutomationLane", "lane": {"lane": "TrackVolume", "writeMode": "Off"}}
            ]))
            .expect("解析"),
        )
        .expect("编译");
        latch[0].apply(&mut project).expect("施加");
        assert_eq!(
            project
                .track(&track_id)
                .expect("音轨")
                .automation_lanes
                .get(&target)
                .expect("泳道")
                .write_mode,
            AutomationWriteMode::Off
        );
        latch[0].apply_inverse(&mut project).expect("逆操作");

        // (4) 取走整条泳道（`remove: true`）。
        let remove = compile(
            &project,
            &track_id,
            &_clip_id,
            &parse_ops(&serde_json::json!([
                {"kind": "setAutomationLane", "lane": {"lane": "TrackVolume", "remove": true}}
            ]))
            .expect("解析"),
        )
        .expect("编译");
        match &remove[0] {
            Op::RemoveAutomationLane {
                target: written,
                previous_lane,
            } => {
                assert_eq!(*written, target);
                assert_eq!(previous_lane, &after, "撤销载荷必须是文档里的整条泳道");
            }
            other => panic!("应当是 RemoveAutomationLane: {other:?}"),
        }
        remove[0].apply(&mut project).expect("取走");
        assert!(
            !project
                .track(&track_id)
                .expect("音轨")
                .automation_lanes
                .contains_key(&target),
            "取走后泳道必须真的不在文档里"
        );
        remove[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            project
                .track(&track_id)
                .expect("音轨")
                .automation_lanes
                .get(&target),
            Some(&after),
            "逆操作必须把整条泳道（含读关状态）装回来"
        );

        // 全部逆回去（(1) 的读关 + 写模式 + 清覆盖）。
        for op in compiled.iter().rev() {
            op.apply_inverse(&mut project).expect("逆操作");
        }
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "逆操作必须逐字节回到原状"
        );
    }

    /// 泳道形态的**形状**错误全部响亮失败：未知键、未知目标名、非布尔读开关、
    /// 未知写模式、`remove` 与属性同给、同一目标写两次。
    ///
    /// 注入（实测红）：把 [`reject_unknown_lane_fields`] 的过滤结果改成恒空 ⇒
    /// 前两条不再红；把 `remove && !patch.is_empty()` 去掉 ⇒ `removeTakesNoProperties` 不再红。
    #[test]
    fn lane_edits_reject_bad_shapes_loudly() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);

        // (a) 未知键（下划线写法是最常见的错法）。
        let fault = parse_ops(&serde_json::json!([
            {"kind": "setAutomationLane", "lane": {"lane": "TrackVolume", "read_enabled": false}}
        ]))
        .expect_err("未知键必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "unknownLaneField",
            "{fault:?}"
        );

        // (b) 未知目标名（别名）与缺 `edgeId`。
        for payload in [
            serde_json::json!([{"kind": "setAutomationLane", "lane": {"lane": "trackVolume"}}]),
            serde_json::json!([{"kind": "setAutomationLane", "lane": {"lane": "SendGain"}}]),
            serde_json::json!([{"kind": "setAutomationLane"}]),
        ] {
            let fault = parse_ops(&payload).expect_err("必须被拒");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{payload} -> {fault:?}"
            );
        }

        // (c) 非布尔读开关 / 未知写模式 / `domain` 形状非法。
        for payload in [
            serde_json::json!([{"kind": "setAutomationLane",
                "lane": {"lane": "TrackVolume", "readEnabled": 1}}]),
            serde_json::json!([{"kind": "setAutomationLane",
                "lane": {"lane": "TrackVolume", "writeMode": "touch"}}]),
            serde_json::json!([{"kind": "setAutomationLane",
                "lane": {"lane": "TrackVolume", "domain": [0.0, 1.0]}}]),
            serde_json::json!([{"kind": "setAutomationLane",
                "lane": {"lane": "TrackVolume", "domain": {"min": 1e300, "max": 1.0}}}]),
        ] {
            let fault = parse_ops(&payload).expect_err("必须被拒");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{payload} -> {fault:?}"
            );
        }

        // (d) `remove` 与属性键同给。
        let fault = parse_ops(&serde_json::json!([
            {"kind": "setAutomationLane",
             "lane": {"lane": "TrackVolume", "remove": true, "readEnabled": false}}
        ]))
        .expect_err("必须被拒");
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "removeTakesNoProperties",
            "{fault:?}"
        );

        // (e) 取走一条**不存在**的泳道：`ENTITY_NOT_FOUND`，不静默成功。
        let fault = parse_ops(&serde_json::json!([
            {"kind": "setAutomationLane", "lane": {"lane": "TrackPan", "remove": true}}
        ]))
        .and_then(|ops| compile(&project, &track_id, &clip_id, &ops))
        .expect_err("不存在的泳道不能取走");
        assert_eq!(fault.domain_code(), Some(ErrorCode::EntityNotFound));

        // (f) 同一目标写两次：在建提案之前就被点名。
        let twice = parse_ops(&serde_json::json!([
            {"kind": "setAutomationLane", "lane": {"lane": "TrackVolume", "readEnabled": false}},
            {"kind": "setAutomationLane", "lane": {"lane": "TrackVolume", "readEnabled": true}}
        ]))
        .expect("解析层不管重复");
        let fault = reject_duplicate_lane_targets(&track_id, &twice).expect_err("重复目标必须被拒");
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "duplicateLaneTarget",
            "{fault:?}"
        );
        // 不同目标不受影响。
        let distinct = parse_ops(&serde_json::json!([
            {"kind": "setAutomationLane", "lane": {"lane": "TrackVolume", "readEnabled": false}},
            {"kind": "setAutomationLane", "lane": {"lane": "TrackPan", "writeMode": "Off"}}
        ]))
        .expect("解析");
        reject_duplicate_lane_targets(&track_id, &distinct).expect("两个不同目标必须放行");
    }

    // -----------------------------------------------------------------------
    // 自动化点取走形态（`removeAutomationPoint`）
    // —— 写侧早就有 `SetAutomationPoint`, 取走侧此前在工具面不可达
    // -----------------------------------------------------------------------

    /// 词表判据：形态名、两个寻址键、`point` 对象允许的键。
    ///
    /// `point` 对象的**泳道寻址**那五项必须与 `lane` 对象的**前五项**是同一份真相
    /// （本判据机械钉住这条关系，不靠人去比对两张手写表）。
    ///
    /// 注入（实测红）：把 [`REMOVE_POINT_FIELDS`] 的第 6 项改成别的键 ⇒ 末段红。
    #[test]
    fn point_removal_field_names_are_pinned() {
        assert_eq!(REMOVE_AUTOMATION_POINT_KIND, "removeAutomationPoint");
        assert_eq!(REMOVE_POINT_FIELD, "point");
        assert_eq!(REMOVE_POINT_TICK_FIELD, "tick");
        assert_eq!(REMOVE_POINT_ID_FIELD, "pointId");
        assert_eq!(REMOVE_POINT_FIELDS.len(), 7);
        assert_eq!(REMOVE_POINT_OP_FIELDS, ["kind", REMOVE_POINT_FIELD]);
        // 前五项 = 泳道寻址 = `lane` 对象的前五项（一份真相）。
        assert_eq!(LANE_TARGET_FIELDS, SET_AUTOMATION_LANE_FIELDS[..5]);
        assert_eq!(REMOVE_POINT_FIELDS[..5], LANE_TARGET_FIELDS);
        // 后两项 = 点寻址（恰好二选一）。
        assert_eq!(
            REMOVE_POINT_FIELDS[5..],
            [REMOVE_POINT_TICK_FIELD, REMOVE_POINT_ID_FIELD]
        );
        assert!(OP_KINDS.contains(&REMOVE_AUTOMATION_POINT_KIND));
    }

    /// 夹具里 `TrackVolume` 泳道在 `tick` 上的那个点（样本前提的**唯一**来源）。
    fn fixture_point(project: &YebanProjectV1, track_id: EntityId, tick: u64) -> AutomationPoint {
        let target = AutomationTarget::TrackVolume { track_id };
        *project
            .track(&track_id)
            .expect("音轨")
            .automation_lanes
            .get(&target)
            .expect("样本里必须有 TrackVolume 泳道")
            .points
            .values()
            .find(|point| point.tick == tick)
            .unwrap_or_else(|| panic!("夹具前提: tick {tick} 上必须有一个点"))
    }

    /// **字面**判据：按 `tick` 取走一个点 ⇒ 文档里真的少了那个点 ⇒ 逆操作逐字节回原。
    ///
    /// 夹具前提（样本 `TrackVolume` 泳道）：两个点（tick 0 / tick 3840），
    /// `readEnabled = true` / `writeMode = Touch` / 有显式取值域 ⇒ 泳道**不是**隐式形状，
    /// 因此取走一个点之后泳道仍在（本形态不顺手回收它）。
    ///
    /// 注入（实测红）：把 `previous_point` 换成常量 ⇒ 模型前置条件报 `OpStateMismatch`；
    /// 把按 tick 的派生换成别的输入 ⇒ 该 tick 上找不到点（`ENTITY_NOT_FOUND`）。
    #[test]
    fn a_point_is_removed_by_tick_and_the_inverse_restores_it_byte_for_byte() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let target = AutomationTarget::TrackVolume { track_id };
        let lane_before = project
            .track(&track_id)
            .expect("音轨")
            .automation_lanes
            .get(&target)
            .expect("样本里必须有 TrackVolume 泳道")
            .clone();
        assert_eq!(lane_before.points.len(), 2, "夹具前提: 两个点");
        let victim = fixture_point(&project, track_id, 3840);
        let bytes_before = serde_json::to_string(&project).expect("序列化");

        let ops = parse_ops(&serde_json::json!([
            {"kind": "removeAutomationPoint", "point": {"lane": "TrackVolume", "tick": 3840}}
        ]))
        .expect("规范形状必须被接受");
        assert_eq!(ops[0].kind_name(), REMOVE_AUTOMATION_POINT_KIND);
        assert!(!ops[0].is_note_level(), "点形态是音轨级");
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::RemoveAutomationPoint {
                target: written,
                point_id,
                previous_point,
            } => {
                assert_eq!(*written, target);
                assert_eq!(*point_id, victim.id, "按 tick 寻址必须派生出文档里那个身份");
                assert_eq!(previous_point, &victim, "撤销载荷必须是文档现值");
            }
            other => panic!("应当是 RemoveAutomationPoint: {other:?}"),
        }

        compiled[0].apply(&mut project).expect("施加");
        let lane_after = project
            .track(&track_id)
            .expect("音轨")
            .automation_lanes
            .get(&target)
            .expect("泳道仍在（夹具的泳道是显式形状, 不随点回收）")
            .clone();
        assert_eq!(lane_after.points.len(), 1, "被取走的点必须真的不在文档里");
        assert!(!lane_after.points.contains_key(&victim.id));
        // 唯一求值入口真的跟着变：tick 3840 不再是那个被取走的值。
        assert_ne!(
            project.automation_value_at(&target, 3840).expect("求值"),
            Some(victim.value),
            "取走之后那一点的值必须不再由自动化给出"
        );

        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "逆操作必须逐字节回到原状"
        );
    }

    /// 显式身份寻址与六条响亮失败（形状错误绝不静默降级）。
    ///
    /// 注入（实测红）：把"恰好二选一"改成"tick 优先" ⇒ 前两条不再红；
    /// 去掉 [`reject_point_removal_op_fields`] ⇒ 顶层 `tick` 那条不再红。
    #[test]
    fn point_removal_shapes_fail_loudly() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let victim = fixture_point(&project, track_id, 3840);
        let victim_id = victim.id.to_canonical_string();

        // (a) 显式身份：与按 tick 寻址指向**同一个**点。
        let by_id = parse_ops(&serde_json::json!([
            {"kind": "removeAutomationPoint",
             "point": {"lane": "TrackVolume", "pointId": victim_id.clone()}}
        ]))
        .expect("解析");
        let compiled = compile(&project, &track_id, &clip_id, &by_id).expect("编译");
        match &compiled[0] {
            Op::RemoveAutomationPoint {
                point_id,
                previous_point,
                ..
            } => {
                assert_eq!(*point_id, victim.id);
                assert_eq!(previous_point, &victim);
            }
            other => panic!("应当是 RemoveAutomationPoint: {other:?}"),
        }

        // (b) 两个寻址同给 / 都不给。
        for (payload, reason) in [
            (
                serde_json::json!([{"kind": "removeAutomationPoint",
                    "point": {"lane": "TrackVolume", "tick": 3840, "pointId": victim_id}}]),
                "pointAddressIsAmbiguous",
            ),
            (
                serde_json::json!([{"kind": "removeAutomationPoint",
                    "point": {"lane": "TrackVolume"}}]),
                "pointAddressRequired",
            ),
        ] {
            let fault = parse_ops(&payload).expect_err("必须被拒");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{payload}"
            );
            assert_eq!(lane_fault_data(&fault)["reason"], reason, "{payload}");
        }

        // (c) 顶层写 `tick`（最像"写对了"的错法：寻址在 `point` 对象里）。
        let fault = parse_ops(&serde_json::json!([
            {"kind": "removeAutomationPoint", "tick": 3840, "point": {"lane": "TrackVolume"}}
        ]))
        .expect_err("顶层 tick 必须被拒");
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "unknownPointRemovalField",
            "{fault:?}"
        );

        // (d) `point` 对象里的泳道属性键（别处合法、此形态不适用）与真的未知键。
        let fault = parse_ops(&serde_json::json!([
            {"kind": "removeAutomationPoint",
             "point": {"lane": "TrackVolume", "tick": 3840, "readEnabled": false}}
        ]))
        .expect_err("必须被拒");
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "lanePropertiesNotApplicableToPointRemoval",
            "{fault:?}"
        );
        let fault = parse_ops(&serde_json::json!([
            {"kind": "removeAutomationPoint",
             "point": {"lane": "TrackVolume", "tick": 3840, "tickk": 1}}
        ]))
        .expect_err("必须被拒");
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "unknownPointField",
            "{fault:?}"
        );

        // (e) 不存在的点 / 不存在的泳道 ⇒ `ENTITY_NOT_FOUND`（不静默成功）。
        let missing_point = parse_ops(&serde_json::json!([
            {"kind": "removeAutomationPoint",
             "point": {"lane": "TrackVolume",
                       "pointId": deterministic_id("no-such-point").to_canonical_string()}}
        ]))
        .expect("解析");
        let fault = compile(&project, &track_id, &clip_id, &missing_point).expect_err("点不存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::EntityNotFound));
        assert_eq!(lane_fault_data(&fault)["reason"], "automationPointNotFound");
        let missing_lane = parse_ops(&serde_json::json!([
            {"kind": "removeAutomationPoint",
             "point": {"lane": "DeviceParam", "slotIndex": 99, "paramIndex": 99, "tick": 0}}
        ]))
        .expect("解析");
        let fault = compile(&project, &track_id, &clip_id, &missing_lane).expect_err("泳道不存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::EntityNotFound));
        assert_eq!(lane_fault_data(&fault)["reason"], "automationLaneNotFound");

        // (f) 同一 tick 上**两个**点：`tick` 寻址不足以指名一个，必须响亮拒绝
        //     （静默挑一个会删掉调用方没指名的那个点）。模型允许这种文档：
        //     `AutomationLane::validate` 只查键/身份一致与取值有限，不查 tick 唯一。
        let mut ambiguous = project.clone();
        let target = AutomationTarget::TrackVolume { track_id };
        let twin = deterministic_id("twin-point-at-3840");
        Op::SetAutomationPoint {
            target,
            point_id: twin,
            old_point: None,
            new_point: AutomationPoint {
                id: twin,
                tick: 3840,
                value: -3.0,
                curve: yeban_model::CurveType::Linear,
            },
        }
        .apply(&mut ambiguous)
        .expect("同一 tick 的第二个点在模型层是合法的");
        ambiguous.validate().expect("文档仍然合法");
        let by_tick = parse_ops(&serde_json::json!([
            {"kind": "removeAutomationPoint", "point": {"lane": "TrackVolume", "tick": 3840}}
        ]))
        .expect("解析");
        let fault = compile(&ambiguous, &track_id, &clip_id, &by_tick).expect_err("必须被拒");
        assert_eq!(
            fault.domain_code(),
            Some(ErrorCode::InvalidParameterRange),
            "{fault:?}"
        );
        let data = lane_fault_data(&fault);
        assert_eq!(data["reason"], "automationTickAmbiguous", "{fault:?}");
        assert_eq!(
            data["candidatePointIds"].as_array().map(Vec::len),
            Some(2),
            "两个候选都必须被点名: {data}"
        );
        // 同一条文档上，显式身份仍然能**精确**取走其中一个。
        let by_id = parse_ops(&serde_json::json!([
            {"kind": "removeAutomationPoint",
             "point": {"lane": "TrackVolume", "pointId": twin.to_canonical_string()}}
        ]))
        .expect("解析");
        let compiled =
            compile(&ambiguous, &track_id, &clip_id, &by_id).expect("显式身份不受歧义影响");
        match &compiled[0] {
            Op::RemoveAutomationPoint { point_id, .. } => assert_eq!(*point_id, twin),
            other => panic!("应当是 RemoveAutomationPoint: {other:?}"),
        }
    }

    /// **跨工具同源**判据：`yeban_edit_automation` 写进去的点，必须能被
    /// `yeban_edit_notes` 按**同一个 tick** 取走。
    ///
    /// 两处对"tick 1920 上是哪一个点"必须给出同一个身份：写侧把它写进文档，
    /// 取走侧从**同一个文档**读回来。这条把 `yeban_edit_automation` 的
    /// `written.pointId` 与取走 op 的 `point_id` 钉在同一个字面值上。
    #[test]
    fn a_point_written_by_the_automation_tool_can_be_removed_by_tick() {
        use super::super::automation;

        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let mut arguments = Map::new();
        arguments.insert("lane".to_owned(), Value::from("TrackVolume"));
        arguments.insert(
            "point".to_owned(),
            serde_json::json!({"tick": 2880, "value": -12.0, "curve": "SCurve"}),
        );
        let planned = automation::plan(&project, &arguments, track_id).expect("写侧规划");
        let write = planned.write.expect("给了 `point` 就必须有点要写");
        write.op.apply(&mut project).expect("施加写侧 op");
        let target = AutomationTarget::TrackVolume { track_id };
        assert!(
            project
                .track(&track_id)
                .expect("音轨")
                .automation_lanes
                .get(&target)
                .expect("泳道")
                .points
                .contains_key(&write.point_id),
            "夹具前提: 写侧的点真的进了文档"
        );

        let ops = parse_ops(&serde_json::json!([
            {"kind": "removeAutomationPoint", "point": {"lane": "TrackVolume", "tick": 2880}}
        ]))
        .expect("解析");
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        match &compiled[0] {
            Op::RemoveAutomationPoint {
                point_id,
                previous_point,
                ..
            } => {
                assert_eq!(
                    *point_id, write.point_id,
                    "按 tick 寻址必须命中写侧那个身份"
                );
                assert_eq!(previous_point.id, write.point_id);
            }
            other => panic!("应当是 RemoveAutomationPoint: {other:?}"),
        }
        compiled[0].apply(&mut project).expect("取走");
        assert!(
            !project
                .track(&track_id)
                .expect("音轨")
                .automation_lanes
                .get(&target)
                .expect("泳道")
                .points
                .contains_key(&write.point_id),
            "取走必须真的把写侧那个点从文档里拿掉"
        );
    }

    /// 池级形态的**字段名与支持集合**被钉住（不多报一个键，也不少报一个）。
    ///
    /// 注入（实测红）：把 [`REMOVE_CLIP_FIELDS`] 改成两个键 ⇒ 本判据红；
    /// 把 [`REMOVE_CLIP_KIND`] 改成模型里没有的名字 ⇒ 本判据红。
    #[test]
    fn remove_clip_field_names_are_pinned() {
        assert_eq!(REMOVE_CLIP_KIND, "removeClip");
        assert_eq!(REMOVE_CLIP_FIELDS, ["kind"]);
        assert!(OP_KINDS.contains(&REMOVE_CLIP_KIND));
    }

    /// **字面**判据：池里那条**没被摆放**的片段取得走 ⇒ 文档里真的少了那条条目 ⇒
    /// 逆操作逐字节回原；并且它的撤销载荷是**文档现值**（不是调用方声明）。
    ///
    /// 夹具前提：`unplaced_midi_clip` 把每条音轨的摆放都清掉，因此池里的 MIDI 材料
    /// 一条都没上时间轴 —— 这正是 [`Op::RemoveClip`] 的前置条件（没有任何摆放引用它）。
    ///
    /// 注入（实测红）：把 `previous_clip` 换成常量 / 另一条条目 ⇒ 模型前置条件报
    /// `OpStateMismatch`；把 `clip_id` 换成派生身份 ⇒ `ClipNotFound`；把 `is_note_level`
    /// 漏掉 `RemoveClip` 且在音频条目上编译 ⇒ `CLIP_NOT_FOUND`（见下一条判据）。
    #[test]
    fn a_pool_entry_is_removed_and_the_inverse_restores_it_byte_for_byte() {
        let (mut project, track_id, clip_id) = unplaced_midi_clip();
        let entry_before = project
            .clip_pool
            .get(&clip_id)
            .expect("夹具前提: 材料在池子里")
            .clone();
        let bytes_before = serde_json::to_string(&project).expect("序列化");

        let ops =
            parse_ops(&serde_json::json!([{"kind": "removeClip"}])).expect("规范形状必须被接受");
        assert_eq!(ops[0].kind_name(), REMOVE_CLIP_KIND);
        assert!(!ops[0].is_note_level(), "池级形态不读不写音符");
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::RemoveClip {
                clip_id: written,
                previous_clip,
            } => {
                assert_eq!(*written, clip_id, "目标必须是顶层 `clipId`");
                assert_eq!(
                    previous_clip, &entry_before,
                    "撤销载荷必须是文档现值 (不是调用方声明)"
                );
            }
            other => panic!("应当是 RemoveClip: {other:?}"),
        }

        compiled[0].apply(&mut project).expect("施加");
        assert!(
            !project.clip_pool.contains_key(&clip_id),
            "被取走的池条目必须真的不在文档里"
        );
        assert_eq!(
            project.clip_pool.len(),
            filled_project().clip_pool.len() - 1,
            "池子必须有且只有一条被取走"
        );

        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "逆操作必须逐字节回到原状"
        );
    }

    /// 池级形态**不要求片段是 MIDI**：它一个音符都不读，因此音频池条目也取得走。
    ///
    /// 阴性对照在同一判据里：那条音频片段**仍然被摆放引用** ⇒ 模型报 `ClipInUse`
    /// （先取走摆放，再取走材料）。这条同时证明本形态把模型的前置条件**原样**留给模型，
    /// 本层不自己复制一份。
    ///
    /// 注入（实测红）：把 `RemoveClip` 从 `is_note_level` 的对照里去掉 ⇒ 音频条目
    /// 编译时撞上"必须是 MIDI 片段"这条断言（`CLIP_NOT_FOUND`），前一段红。
    #[test]
    fn a_pool_entry_is_removed_without_midi_and_in_use_is_a_model_conflict() {
        let mut project = filled_project();
        let (track_id, _midi_clip) = lead_clip(&project);
        let audio_clip_id = project
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_none())
            .expect("样本里必须有非 MIDI 片段")
            .id;
        let ops = parse_ops(&serde_json::json!([{"kind": "removeClip"}])).expect("解析");

        // 阴性对照：音频片段还在 `bass` 的摆放里 ⇒ 编译本身成功，但**施加**被模型拒。
        let compiled = compile(&project, &track_id, &audio_clip_id, &ops)
            .expect("音频条目同样可编译 (本形态不要求 MIDI)");
        assert_eq!(compiled.len(), 1);
        let error = compiled[0]
            .apply(&mut project)
            .expect_err("还有摆放引用它 ⇒ 模型必须拒绝");
        assert!(
            matches!(error, yeban_model::ModelError::ClipInUse { .. }),
            "必须是模型自己的 ClipInUse, 实际是 {error:?}"
        );
        assert!(
            project.clip_pool.contains_key(&audio_clip_id),
            "被拒绝的批次不得改动文档"
        );

        // 取走引用它的那条摆放之后，同一条 op 就成立了。
        let placements: Vec<(EntityId, EntityId)> = project
            .tracks
            .values()
            .flat_map(|track| {
                track
                    .clips
                    .values()
                    .filter(|placement| placement.clip_id == audio_clip_id)
                    .map(|placement| (track.id, placement.id))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(!placements.is_empty(), "夹具前提: 音频片段确实被摆放引用");
        for (owner, placement_id) in placements {
            let placement = *project
                .track(&owner)
                .expect("音轨")
                .clips
                .get(&placement_id)
                .expect("摆放");
            Op::RemoveClipPlacement {
                track_id: owner,
                placement_id,
                previous_placement: placement,
            }
            .apply(&mut project)
            .expect("取走摆放");
        }
        compiled[0]
            .apply(&mut project)
            .expect("没有摆放引用它之后, 池条目取得走");
        assert!(!project.clip_pool.contains_key(&audio_clip_id));
    }

    /// 池级形态的形状错误**响亮失败**（绝不静默丢弃）。
    ///
    /// 注入（实测红）：去掉 [`reject_remove_clip_fields`] ⇒ 嵌套 `clipId` 那条不再红
    /// （调用方会以为取走的是另一个片段）。
    #[test]
    fn remove_clip_shapes_fail_loudly() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);

        for broken in [
            serde_json::json!([{"kind": "removeClip", "clipId": clip_id.to_canonical_string()}]),
            serde_json::json!([{"kind": "removeClip", "trackId": track_id.to_canonical_string()}]),
            serde_json::json!([{"kind": "removeClip", "remove": true}]),
        ] {
            let fault = parse_ops(&broken).expect_err("必须被拒");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{broken}"
            );
            assert_eq!(
                lane_fault_data(&fault)["reason"],
                "unknownRemoveClipField",
                "{broken}"
            );
            assert_eq!(
                lane_fault_data(&fault)["supportedRemoveClipFields"],
                serde_json::json!(["kind"]),
                "{broken}"
            );
        }

        // 恰好一个键的规范形状必须被接受（阴性对照：上面三条红的不是"全都拒"）。
        assert_eq!(
            parse_ops(&serde_json::json!([{"kind": "removeClip"}])).expect("规范形状"),
            vec![NoteOp::RemoveClip]
        );
    }

    /// 样本里的一条路由边：`(身份, 文档现值)`，按"现值是不是单位增益"挑。
    ///
    /// `filled_project` 恰好两种都有（两条 `None`、一条 `Some(-12.0)`），
    /// 因此两种情形都能被**真的**量到，而不是靠构造一个假的文档。
    fn an_edge(project: &YebanProjectV1, unit_gain: bool) -> (EntityId, Option<f32>) {
        let edge = project
            .routing_graph
            .edges
            .values()
            .find(|edge| edge.gain_db.is_none() == unit_gain)
            .expect("样本里必须有这两种路由边");
        (edge.id, edge.gain_db)
    }

    /// 路由级形态的**字面**判据：`old_gain_db` 来自当前文档的**原样** `Option<f32>`
    /// （`None` 不是 `0.0`）、`new_gain_db` 是 `Option<f32>` 的字面载荷、
    /// 且 `Op::invert` 能逐字节回退。
    ///
    /// 这一条对着"工具面写不了发送增益"的缺口：模型指定 `Op::SetRoutingGain` 是它
    /// 唯一的写者（`read_param` / `write_param` 明文拒绝 `SendGain`），而本形态出现
    /// 之前没有任何工具构造过这个变体。
    ///
    /// 注入（实测红）：把 `old_gain_db` 换成 `AutomationTarget::static_value` 的读数
    /// ⇒ 单位增益那条边的 `old_gain_db` 变成 `Some(0.0)`（`left: Some(0.0)` /
    /// `right: None`），且**施加**时模型的前置条件直接报 `OpStateMismatch`；
    /// 把 `new_gain_db` 由 `*gain_db` 改成 `Some(gain_db.unwrap_or(0.0))` ⇒ 第二条
    /// （写 `null`）不再是 `None`。
    #[test]
    fn set_routing_gain_compiles_against_the_document_and_inverts_byte_for_byte() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let (unit_edge, unit_before) = an_edge(&project, true);
        let (gain_edge, gain_before) = an_edge(&project, false);
        assert_eq!(unit_before, None, "夹具前提: 单位增益那条边是 `None`");
        assert!(gain_before.is_some(), "夹具前提: 另一条边有具体增益");
        let bytes_before = serde_json::to_string(&project).expect("序列化");

        let ops = parse_ops(&serde_json::json!([
            {"kind": SET_ROUTING_GAIN_KIND,
             "edgeId": unit_edge.to_canonical_string(), "value": -4.5},
            {"kind": SET_ROUTING_GAIN_KIND,
             "edgeId": gain_edge.to_canonical_string(), "value": null}
        ]))
        .expect("规范形状必须被接受");
        assert_eq!(ops[0].kind_name(), SET_ROUTING_GAIN_KIND);
        assert!(
            ops.iter().all(|op| !op.is_note_level()),
            "路由级不读不写音符"
        );
        assert!(ops.iter().all(NoteOp::is_routing_level));
        assert_eq!(
            ops[0],
            NoteOp::SetRoutingGain {
                edge_id: unit_edge,
                gain_db: Some(-4.5),
            }
        );
        assert_eq!(
            ops[1],
            NoteOp::SetRoutingGain {
                edge_id: gain_edge,
                gain_db: None,
            },
            "JSON `null` 必须解析成模型的 `None` (单位增益), 不是 `Some(0.0)`"
        );

        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 2);
        match &compiled[0] {
            Op::SetRoutingGain {
                edge_id,
                old_gain_db,
                new_gain_db,
            } => {
                assert_eq!(*edge_id, unit_edge);
                assert_eq!(
                    *old_gain_db, unit_before,
                    "撤销载荷必须是文档的**原样** `Option<f32>` (单位增益 ≠ 0.0 dB)"
                );
                assert_eq!(*new_gain_db, Some(-4.5));
            }
            other => panic!("应当是 SetRoutingGain: {other:?}"),
        }
        match &compiled[1] {
            Op::SetRoutingGain {
                old_gain_db,
                new_gain_db,
                ..
            } => {
                assert_eq!(*old_gain_db, gain_before, "撤销载荷必须来自当前文档");
                assert_eq!(*new_gain_db, None, "`null` 写进去的是单位增益");
            }
            other => panic!("应当是 SetRoutingGain: {other:?}"),
        }

        Op::Batch {
            ops: compiled.clone(),
            description: "setRoutingGain".to_owned(),
        }
        .apply(&mut project)
        .expect("施加");
        assert_eq!(
            project.routing_graph.edge(&unit_edge).expect("边").gain_db,
            Some(-4.5)
        );
        assert_eq!(
            project.routing_graph.edge(&gain_edge).expect("边").gain_db,
            None,
            "写 `null` 必须真的把该边变回单位增益"
        );

        for op in compiled.iter().rev() {
            op.apply_inverse(&mut project).expect("逆操作");
        }
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "逆操作必须逐字节回到原状"
        );
    }

    /// `-0.0` 与 `0.0` 在模型里是**两个**值（`same_gain` 逐位比较）：
    /// 本形态必须原样搬运，不"顺手归一化"。
    ///
    /// 注入（实测红）：把 `new_gain_db` 由 `*gain_db` 改成 `gain_db.map(|v| v + 0.0)`
    /// （`-0.0 + 0.0` 是 `0.0`）⇒ 第一条断言红（`left: 0` / `right: 2147483648`）；
    /// 把 `old_gain_db` 折成裸 `f32` 的读数（`None` → `0.0`）⇒ 第二条断言红。
    #[test]
    fn set_routing_gain_keeps_minus_zero_and_none_apart() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let (edge_id, before) = an_edge(&project, true);
        assert_eq!(before, None);

        let ops = parse_ops(&serde_json::json!([
            {"kind": SET_ROUTING_GAIN_KIND, "edgeId": edge_id.to_canonical_string(), "value": -0.0}
        ]))
        .expect("解析");
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        match &compiled[0] {
            Op::SetRoutingGain { new_gain_db, .. } => {
                let written = new_gain_db.expect("必须是 `Some`");
                assert_eq!(
                    written.to_bits(),
                    (-0.0_f32).to_bits(),
                    "`-0.0` 必须逐位原样搬进去 (不是 `0.0`)"
                );
                assert_ne!(written.to_bits(), 0.0_f32.to_bits());
            }
            other => panic!("应当是 SetRoutingGain: {other:?}"),
        }

        compiled[0].apply(&mut project).expect("施加");
        assert_eq!(
            project
                .routing_graph
                .edge(&edge_id)
                .expect("边")
                .gain_db
                .expect("`Some(-0.0)`")
                .to_bits(),
            (-0.0_f32).to_bits()
        );
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            project.routing_graph.edge(&edge_id).expect("边").gain_db,
            None,
            "逆操作必须回到 `None` (不是 `Some(0.0)`)"
        );
    }

    /// 路由级形态的**字段名与支持集合**被钉住（不多报一个键，也不少报一个）。
    ///
    /// 注入（实测红）：把 [`SET_ROUTING_GAIN_FIELDS`] 少写一个键 ⇒ 本判据红；
    /// 把 [`SET_ROUTING_GAIN_KIND`] 改成模型里没有的名字 ⇒ 本判据红。
    #[test]
    fn set_routing_gain_field_names_are_pinned() {
        assert_eq!(SET_ROUTING_GAIN_KIND, "setRoutingGain");
        assert_eq!(SET_ROUTING_GAIN_EDGE_FIELD, "edgeId");
        assert_eq!(SET_ROUTING_GAIN_VALUE_FIELD, "value");
        assert_eq!(SET_ROUTING_GAIN_FIELDS, ["kind", "edgeId", "value"]);
        assert!(OP_KINDS.contains(&SET_ROUTING_GAIN_KIND));
        // 与模型自己的变体名同词（不手写第二张会漂移的表）。
        assert_eq!(
            Op::SetRoutingGain {
                edge_id: EntityId::from_str("01J8ZQ00000000000000000060").expect("ULID"),
                old_gain_db: None,
                new_gain_db: None,
            }
            .name(),
            "SetRoutingGain"
        );
    }

    /// 路由级形态的形状错误**响亮失败**（绝不静默丢弃），而规范形状放行。
    ///
    /// 注入（实测红）：去掉 [`reject_routing_gain_fields`] 的调用 ⇒ 第一条（把目标
    /// 写成工具顶层的 `trackId`）被**静默接受**（`left: SetRoutingGain { … }`），本判据红。
    #[test]
    fn set_routing_gain_shapes_fail_loudly() {
        let project = filled_project();
        let (edge_id, _) = an_edge(&project, true);
        let (track_id, clip_id) = lead_clip(&project);
        let edge_text = edge_id.to_canonical_string();

        for broken in [
            // 目标写在顶层 `trackId` 上（那是别的形态的寻址，本形态不读它）。
            serde_json::json!([{"kind": SET_ROUTING_GAIN_KIND, "edgeId": edge_text,
                                "value": -3.0, "trackId": track_id.to_canonical_string()}]),
            // 增益键写成模型字段名 `gainDb`：`value` 缺失、`gainDb` 未知，两条都要报。
            serde_json::json!([{"kind": SET_ROUTING_GAIN_KIND, "edgeId": edge_text,
                                "gainDb": -3.0}]),
            serde_json::json!([{"kind": SET_ROUTING_GAIN_KIND, "edgeId": edge_text,
                                "value": -3.0, "null": true}]),
        ] {
            let fault = parse_ops(&broken).expect_err("必须被拒");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{broken}"
            );
            assert_eq!(
                lane_fault_data(&fault)["reason"],
                "unknownRoutingGainField",
                "{broken}"
            );
            assert_eq!(
                lane_fault_data(&fault)["supportedRoutingGainFields"],
                serde_json::json!(["kind", "edgeId", "value"]),
                "{broken}"
            );
        }

        for (broken, expected_reason) in [
            // 缺 `value`：统一的缺字段错误（没有 `data`）。
            (
                serde_json::json!([{"kind": SET_ROUTING_GAIN_KIND, "edgeId": edge_text}]),
                None,
            ),
            // 缺 `edgeId`。
            (
                serde_json::json!([{"kind": SET_ROUTING_GAIN_KIND, "value": -3.0}]),
                None,
            ),
            // `value` 是布尔 / 字符串 / 对象 ⇒ 既不是数字也不是 `null`。
            (
                serde_json::json!([{"kind": SET_ROUTING_GAIN_KIND, "edgeId": edge_text,
                                    "value": true}]),
                Some("valueMustBeNumber"),
            ),
            (
                serde_json::json!([{"kind": SET_ROUTING_GAIN_KIND, "edgeId": edge_text,
                                    "value": "quiet"}]),
                Some("valueMustBeNumber"),
            ),
            (
                serde_json::json!([{"kind": SET_ROUTING_GAIN_KIND, "edgeId": edge_text,
                                    "value": {}}]),
                Some("valueMustBeNumber"),
            ),
            // 有限 `f64` 收窄到 `f32` 会溢出成 `inf` ⇒ 这一条**可达**（不是摆设）。
            (
                serde_json::json!([{"kind": SET_ROUTING_GAIN_KIND, "edgeId": edge_text,
                                    "value": 1e300}]),
                Some("nonFiniteValue"),
            ),
            // `edgeId` 不是合法 ULID。
            (
                serde_json::json!([{"kind": SET_ROUTING_GAIN_KIND, "edgeId": "not-a-ulid",
                                    "value": -3.0}]),
                None,
            ),
        ] {
            let fault = parse_ops(&broken).expect_err(&format!("必须被拒: {broken}"));
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{broken}"
            );
            match expected_reason {
                Some(reason) => assert_eq!(lane_fault_data(&fault)["reason"], reason, "{broken}"),
                None => assert!(
                    matches!(&fault, Fault::Domain { data: None, .. }),
                    "缺字段 / 非法身份的失败不该带 `data`: {broken} / {fault:?}"
                ),
            }
        }

        // 阴性对照: 规范形状(数字 / `null` / `-0.0`)必须被接受 —— 上面红的不是"全都拒"。
        for value in [
            serde_json::json!(-3.0),
            serde_json::json!(null),
            serde_json::json!(-0.0),
        ] {
            let ops = parse_ops(&serde_json::json!([
                {"kind": SET_ROUTING_GAIN_KIND, "edgeId": edge_text, "value": value}
            ]))
            .unwrap_or_else(|fault| panic!("规范形状必须被接受: {value} / {fault:?}"));
            assert_eq!(ops.len(), 1);
            assert!(ops[0].is_routing_level());
            // 顺带证明"路由级 -> 编译"这一跳也能走通（不是只解析得动）。
            compile(&project, &track_id, &clip_id, &ops).expect("编译");
        }
    }

    /// 路由级形态**不要求片段是 MIDI**（一个音符都不读），而指向**不存在**的边时
    /// 报 `ENTITY_NOT_FOUND`（与模型 `RoutingEdgeNotFound` 同一个契约码）。
    ///
    /// 注入（实测红）：把 `SetRoutingGain` 从 `is_note_level` 的对照里去掉 ⇒
    /// 音频片段那条编译时撞上"必须是 MIDI 片段"（`CLIP_NOT_FOUND`）；
    /// 把缺边的错误码换成 `CONFLICT` ⇒ 第二条红。
    #[test]
    fn set_routing_gain_does_not_need_midi_and_a_missing_edge_is_entity_not_found() {
        let project = filled_project();
        let audio_track = project
            .tracks
            .values()
            .find(|track| track.kind == yeban_model::TrackKind::Audio)
            .expect("样本里必须有音频轨")
            .id;
        let audio_clip = project
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_none())
            .expect("样本里必须有非 MIDI 片段")
            .id;
        let (edge_id, _) = an_edge(&project, true);

        let ops = parse_ops(&serde_json::json!([
            {"kind": SET_ROUTING_GAIN_KIND, "edgeId": edge_id.to_canonical_string(), "value": -6.0}
        ]))
        .expect("解析");
        let compiled = compile(&project, &audio_track, &audio_clip, &ops)
            .expect("路由级写入不读片段内容, 非 MIDI 片段也必须被接受");
        assert_eq!(compiled.len(), 1);

        // 指向一条**不存在**的边: 本层在编译期就报 `ENTITY_NOT_FOUND`。
        let missing = EntityId::from_str("01J8ZQ00000000000000000999").expect("ULID");
        assert!(project.routing_graph.edge(&missing).is_none(), "夹具前提");
        let ops = parse_ops(&serde_json::json!([
            {"kind": SET_ROUTING_GAIN_KIND, "edgeId": missing.to_canonical_string(), "value": -6.0}
        ]))
        .expect("解析");
        let fault = compile(&project, &audio_track, &audio_clip, &ops).expect_err("边不存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::EntityNotFound));
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "routingEdgeNotFound",
            "缺边的 `reason` 必须点名是路由边"
        );
        assert_eq!(
            lane_fault_data(&fault)["edgeId"],
            serde_json::json!(missing.to_canonical_string())
        );
    }

    /// 样本里一条**非**主总线节点，以及当前引用它的每条边。
    ///
    /// 模型的 `RemoveRoutingNode` 有"没有任何边引用它"的前置条件，而 `filled_project`
    /// 的四个节点**全部**被引用 ⇒ "取走节点"的判据必须先走一遍工具面自己的两步
    /// （`disconnectRouting` 每一条引用边 → `removeRoutingNode`）。挑**引用边最少**的
    /// 那个节点，让两步序列最短；`nodes` 是 `Vec`，因此这个选择是确定性的。
    fn a_non_master_node(project: &YebanProjectV1) -> (EntityId, Vec<EntityId>) {
        let referencing = |node: EntityId| -> Vec<EntityId> {
            project
                .routing_graph
                .edges
                .values()
                .filter(|edge| edge.source_node == node || edge.destination_node == node)
                .map(|edge| edge.id)
                .collect()
        };
        project
            .routing_graph
            .nodes
            .iter()
            .copied()
            .filter(|node| *node != project.master_bus_track_id)
            .map(|node| (node, referencing(node)))
            .min_by_key(|(_, edges)| edges.len())
            .expect("样本里必须有非主总线节点")
    }

    /// 路由节点形态的**字面**判据：载荷只有节点身份（模型**没有**撤销载荷），
    /// 两步序列（断开每一条引用边 → 取走节点）能被 `Op::invert` 逐步逐字节回退。
    ///
    /// 这一条对着"工具面造得出的节点取不走"的缺口：`yeban_propose_section` 的建批
    /// 会构造 `Op::AddRoutingNode`，而本形态之前**没有任何工具**构造过
    /// `Op::RemoveRoutingNode`（工具说明却已经写了"再单独一次调用取走节点"）。
    ///
    /// 注入（实测红）：把本形态编译出的那个模型变体由 `Op::RemoveRoutingNode` 换成
    /// `Op::AddRoutingNode` ⇒ 节点数不减反增（`nodes.len() - 1`
    /// 那条断言红），逆操作也不再回到原状；把 [`ROUTING_NODE_FIELD`] 由 `nodeId`
    /// 改成 `edgeId` ⇒ 解析那一步就红。
    #[test]
    fn remove_routing_node_compiles_an_unreferenced_node_and_inverts_byte_for_byte() {
        let mut project = filled_project();
        let (node, edges) = a_non_master_node(&project);
        assert_eq!(
            edges.len(),
            1,
            "夹具前提: 引用边最少的非主总线节点只有一条边"
        );
        let nodes_before = project.routing_graph.nodes.clone();
        let bytes_before = serde_json::to_string(&project).expect("序列化");
        let (track_id, clip_id) = lead_clip(&project);

        // 第一步: 按 `disconnectRouting` 的纪律断开引用它的每一条边。
        let raw: Vec<Value> = edges
            .iter()
            .map(|edge| {
                serde_json::json!({
                    "kind": DISCONNECT_ROUTING_KIND,
                    "edgeId": edge.to_canonical_string(),
                })
            })
            .collect();
        let disconnected = parse_ops(&Value::Array(raw)).expect("解析");
        let compiled_disconnected =
            compile(&project, &track_id, &clip_id, &disconnected).expect("编译");
        assert_eq!(compiled_disconnected.len(), edges.len());
        for op in &compiled_disconnected {
            op.apply(&mut project).expect("断开");
        }
        assert!(
            project.routing_graph.nodes.contains(&node),
            "断开边一个字都不动 `nodes` (取走节点是本形态的事)"
        );
        let bytes_after_disconnect = serde_json::to_string(&project).expect("序列化");

        // 第二步: 取走那个节点。
        let ops = parse_ops(&serde_json::json!([
            {"kind": REMOVE_ROUTING_NODE_KIND, "nodeId": node.to_canonical_string()}
        ]))
        .expect("规范形状必须被接受");
        assert_eq!(ops[0].kind_name(), REMOVE_ROUTING_NODE_KIND);
        assert!(!ops[0].is_note_level(), "路由级不读不写音符");
        assert!(ops[0].is_routing_level());
        assert_eq!(ops[0], NoteOp::RemoveRoutingNode { node_id: node });
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::RemoveRoutingNode { node: target } => assert_eq!(*target, node),
            other => panic!("应当是 RemoveRoutingNode: {other:?}"),
        }
        compiled[0].apply(&mut project).expect("取走");
        assert!(
            !project.routing_graph.nodes.contains(&node),
            "节点必须真的从 `nodes` 里消失"
        );
        assert_eq!(project.routing_graph.nodes.len(), nodes_before.len() - 1);
        assert!(project.validate().is_ok(), "取走之后工程必须仍然合法");

        // 逆操作逐步回退: 先回退"取走", 再回退"断开"。
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            project.routing_graph.nodes, nodes_before,
            "逆操作必须把节点放回原来的位置"
        );
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_after_disconnect,
            "逆操作必须逐字节回到断开之后的状态"
        );
        for op in compiled_disconnected.iter().rev() {
            op.apply_inverse(&mut project).expect("逆操作");
        }
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "两步都回退之后必须逐字节回到原状"
        );
    }

    /// 路由节点形态的**字段名与支持集合**被钉住（不多报一个键，也不少报一个键）。
    ///
    /// 注入（实测红）：把 [`ROUTING_NODE_FIELD`] 改成 `edgeId` ⇒ 本判据红 ——
    /// 节点与边是两种实体，共用一个词会让"取走的是哪一个"从形状上无法区分。
    #[test]
    fn remove_routing_node_field_names_are_pinned() {
        assert_eq!(REMOVE_ROUTING_NODE_KIND, "removeRoutingNode");
        assert_eq!(ROUTING_NODE_FIELD, "nodeId");
        assert_ne!(
            ROUTING_NODE_FIELD, ROUTING_EDGE_FIELD,
            "节点与边不是同一个字面量"
        );
        assert_eq!(REMOVE_ROUTING_NODE_FIELDS, ["kind", "nodeId"]);
        assert!(OP_KINDS.contains(&REMOVE_ROUTING_NODE_KIND));
        // 与模型自己的变体名同词（不手写第二张会漂移的表）。
        assert_eq!(
            Op::RemoveRoutingNode {
                node: EntityId::from_str("01J8ZQ00000000000000000060").expect("ULID"),
            }
            .name(),
            "RemoveRoutingNode"
        );
    }

    /// 路由节点形态的形状错误**响亮失败**（绝不静默丢弃），而规范形状放行。
    ///
    /// 注入（实测红）：去掉 [`reject_remove_routing_node_fields`] 的调用 ⇒ 第一条
    /// （把**边**的寻址 `edgeId` 搬过来）与第二条（把工具顶层的 `trackId` 搬过来）
    /// 被**静默接受**，本判据红。
    #[test]
    fn remove_routing_node_shapes_fail_loudly() {
        let project = filled_project();
        let (node, _) = a_non_master_node(&project);
        let node_text = node.to_canonical_string();
        let (track_id, clip_id) = lead_clip(&project);

        for broken in [
            // 把**边**的寻址搬过来: `nodeId` 缺失, 而且 `edgeId` 不是本形态的键。
            serde_json::json!([{"kind": REMOVE_ROUTING_NODE_KIND, "edgeId": node_text}]),
            // 把工具顶层的 `trackId` 搬过来（那是别的形态的寻址）。
            serde_json::json!([{"kind": REMOVE_ROUTING_NODE_KIND, "nodeId": node_text,
                                "trackId": track_id.to_canonical_string()}]),
            serde_json::json!([{"kind": REMOVE_ROUTING_NODE_KIND, "nodeId": node_text,
                                "value": null}]),
        ] {
            let fault = parse_ops(&broken).expect_err("必须被拒");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{broken}"
            );
            assert_eq!(
                lane_fault_data(&fault)["reason"],
                "unknownRemoveRoutingNodeField",
                "{broken}"
            );
            assert_eq!(
                lane_fault_data(&fault)["supportedRemoveRoutingNodeFields"],
                serde_json::json!(["kind", "nodeId"]),
                "{broken}"
            );
        }

        for broken in [
            // 缺 `nodeId`。
            serde_json::json!([{"kind": REMOVE_ROUTING_NODE_KIND}]),
            // `nodeId` 不是字符串。
            serde_json::json!([{"kind": REMOVE_ROUTING_NODE_KIND, "nodeId": 7}]),
            // `nodeId` 不是合法 ULID。
            serde_json::json!([{"kind": REMOVE_ROUTING_NODE_KIND, "nodeId": "not-a-ulid"}]),
        ] {
            let fault = parse_ops(&broken).expect_err(&format!("必须被拒: {broken}"));
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{broken}"
            );
        }

        // 阴性对照: 规范形状必须被接受 —— 上面红的不是"全都拒"。
        let ops = parse_ops(&serde_json::json!([
            {"kind": REMOVE_ROUTING_NODE_KIND, "nodeId": node_text}
        ]))
        .expect("规范形状必须被接受");
        assert!(ops[0].is_routing_level());
        compile(&project, &track_id, &clip_id, &ops).expect("编译");
    }

    /// 三条口径：节点不存在 → 本层 `ENTITY_NOT_FOUND`；节点是主总线 → 本层
    /// `CONFLICT`；还有边引用它 → 本层**放行**、由模型报 `RoutingNodeInUse`。
    ///
    /// 注入（实测红）：删掉主总线那条分支 ⇒ 第二条红（它会走到模型，而模型对
    /// "有音轨但主总线不在 `nodes` 里"复用 `RoutingNodeNotFound` ⇒ 契约码变成
    /// `ENTITY_NOT_FOUND`，与"节点刚刚被自己取走"的事实不符）；删掉存在性分支
    /// ⇒ 第一条红（失败会来自提案模拟，没有本层的 `reason` / `nodeId`）。
    #[test]
    fn remove_routing_node_refuses_missing_and_master_and_defers_in_use_to_the_model() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let compile_one = |node: EntityId| -> Result<Vec<Op>, Fault> {
            let ops = parse_ops(&serde_json::json!([
                {"kind": REMOVE_ROUTING_NODE_KIND, "nodeId": node.to_canonical_string()}
            ]))
            .expect("解析");
            compile(&project, &track_id, &clip_id, &ops)
        };

        // 1. 节点不存在: 本层在编译期就报, 且带上 `reason` 与 `nodeId`。
        let missing = EntityId::from_str("01J8ZQ00000000000000000999").expect("ULID");
        assert!(
            !project.routing_graph.nodes.contains(&missing),
            "夹具前提: 这个身份不在 `nodes` 里"
        );
        let fault = compile_one(missing).expect_err("节点不存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::EntityNotFound));
        assert_eq!(lane_fault_data(&fault)["reason"], "routingNodeNotFound");
        assert_eq!(
            lane_fault_data(&fault)["nodeId"],
            serde_json::json!(missing.to_canonical_string())
        );

        // 2. 主总线节点: 模型要求它留在 `nodes` 里 ⇒ 本层用一个说得通的码 (CONFLICT)。
        let master = project.master_bus_track_id;
        assert!(
            project.routing_graph.nodes.contains(&master),
            "夹具前提: 主总线在 `nodes` 里"
        );
        let fault = compile_one(master).expect_err("主总线不能取走");
        assert_eq!(fault.domain_code(), Some(ErrorCode::Conflict));
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "masterBusNodeCannotBeRemoved"
        );
        assert_eq!(
            lane_fault_data(&fault)["masterBusTrackId"],
            serde_json::json!(master.to_canonical_string())
        );

        // 3. 还有边引用它: 本层**不**复制那条前置条件 (模型有 `edge_count`)。
        let (referenced, edges) = a_non_master_node(&project);
        assert!(!edges.is_empty(), "夹具前提: 这个节点还有引用边");
        let compiled = compile_one(referenced).expect("本层不复制模型的前置条件");
        assert_eq!(compiled.len(), 1);
        let mut probe = project.clone();
        let error = compiled[0].apply(&mut probe).expect_err("还有边引用它");
        assert!(
            matches!(
                error,
                yeban_model::ModelError::RoutingNodeInUse { edge_count, .. }
                    if edge_count == edges.len()
            ),
            "模型必须报 RoutingNodeInUse 并带上边数: {error:?}"
        );
        assert_eq!(
            probe.routing_graph.nodes, project.routing_graph.nodes,
            "失败的前置条件不得改文档"
        );
    }

    /// 路由节点形态**不要求片段是 MIDI**（一个音符都不读），而且它可以在**同一个批**
    /// 里跟在 `disconnectRouting` 后面（`Op::Batch` 在演化中的文档上逐条校验）。
    ///
    /// 注入（实测红）：把 `RemoveRoutingNode` 从 `is_note_level` 的对照里去掉 ⇒
    /// 音频片段那条编译时撞上"必须是 MIDI 片段"（`CLIP_NOT_FOUND`）。
    #[test]
    fn remove_routing_node_does_not_need_midi_and_can_follow_a_disconnect_in_one_batch() {
        let project = filled_project();
        let audio_track = project
            .tracks
            .values()
            .find(|track| track.kind == yeban_model::TrackKind::Audio)
            .expect("样本里必须有音频轨")
            .id;
        let audio_clip = project
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_none())
            .expect("样本里必须有非 MIDI 片段")
            .id;
        let (node, edges) = a_non_master_node(&project);
        assert_eq!(edges.len(), 1, "夹具前提: 只有一条引用边");

        let ops = parse_ops(&serde_json::json!([
            {"kind": DISCONNECT_ROUTING_KIND, "edgeId": edges[0].to_canonical_string()},
            {"kind": REMOVE_ROUTING_NODE_KIND, "nodeId": node.to_canonical_string()}
        ]))
        .expect("解析");
        let compiled = compile(&project, &audio_track, &audio_clip, &ops)
            .expect("路由级写入不读片段内容, 非 MIDI 片段也必须被接受");
        assert_eq!(compiled.len(), 2);

        let mut probe = project.clone();
        Op::Batch {
            ops: compiled.clone(),
            description: "先断开引用边, 再取走节点".to_owned(),
        }
        .apply(&mut probe)
        .expect("一个批里先断开再取走 (批在演化中的文档上逐条校验)");
        assert!(!probe.routing_graph.nodes.contains(&node));
        assert!(probe.validate().is_ok(), "取走之后工程必须仍然合法");
    }

    /// 段落形态的**字面**判据：载荷只有段落身份（撤销载荷从**当前文档**读整条段落），
    /// 一步就能被 `Op::invert` 逐字节回退到原状。
    ///
    /// 这一条对着"工具面造得出的段落取不走"的缺口：`yeban_propose_section` 的建批
    /// 会构造 `Op::SetSection`，而本形态之前**没有任何工具**构造过 `Op::RemoveSection`
    /// （读侧 `yeban_query_project` 的 `entities[]` 却一直在报 `kind == "section"` 的身份）。
    ///
    /// 注入（实测红）：把编译出的模型变体由 `Op::RemoveSection` 换成
    /// `Op::SetSection`（一次什么都不删的写）⇒ 变体匹配那条红；把撤销载荷换成**另一条**
    /// 段落的克隆（而不是文档里那一条）⇒ "撤销载荷必须是文档里那一条"红。
    #[test]
    fn remove_section_compiles_and_inverts_byte_for_byte() {
        let mut project = filled_project();
        let section_id = *project
            .sections
            .keys()
            .next()
            .expect("样本里必须有曲式段落");
        let expected = project.sections[&section_id].clone();
        let bytes_before = serde_json::to_string(&project).expect("序列化");
        let (track_id, clip_id) = lead_clip(&project);

        let ops = parse_ops(&serde_json::json!([
            {"kind": REMOVE_SECTION_KIND, "sectionId": section_id.to_canonical_string()}
        ]))
        .expect("规范形状必须被接受");
        assert_eq!(ops[0].kind_name(), REMOVE_SECTION_KIND);
        assert!(!ops[0].is_note_level(), "段落级不读不写音符");
        assert!(!ops[0].is_routing_level(), "段落不是路由图的一部分");
        assert!(ops[0].is_section_level());
        assert_eq!(ops[0], NoteOp::RemoveSection { section_id });

        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::RemoveSection {
                section_id: target,
                previous_section,
            } => {
                assert_eq!(*target, section_id);
                assert_eq!(
                    previous_section, &expected,
                    "撤销载荷必须是**文档里那一条**段落 (不是调用方声明的)"
                );
            }
            other => panic!("应当是 RemoveSection: {other:?}"),
        }

        compiled[0].apply(&mut project).expect("取走");
        assert!(
            !project.sections.contains_key(&section_id),
            "段落必须真的从 `sections` 里消失"
        );
        assert!(project.validate().is_ok(), "取走之后工程必须仍然合法");

        // 逆操作: `RemoveSection` 的逆是 `SetSection { old_section: None }`
        // （`docs/adr/ADR-0001` 的 D12），因此必须把段落原样放回去。
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            project.sections.get(&section_id),
            Some(&expected),
            "逆操作必须把段落放回原来的形状"
        );
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "逆操作必须逐字节回到取走之前的文档"
        );
    }

    /// 段落形态的**字段名与支持集合**被钉住（不多报一个键，也不少报一个键）。
    ///
    /// 注入（实测红）：把 [`SECTION_FIELD`] 改成 `nodeId` ⇒ 本判据红 ——
    /// 段落与路由节点是两种实体，共用一个词会让"取走的是哪一个"从形状上无法区分。
    #[test]
    fn remove_section_field_names_are_pinned() {
        assert_eq!(REMOVE_SECTION_KIND, "removeSection");
        assert_eq!(SECTION_FIELD, "sectionId");
        assert_ne!(
            SECTION_FIELD, ROUTING_NODE_FIELD,
            "段落与路由节点不是同一个字面量"
        );
        assert_ne!(SECTION_FIELD, ROUTING_EDGE_FIELD, "段落不是路由边");
        assert_eq!(REMOVE_SECTION_FIELDS, ["kind", "sectionId"]);
        assert!(OP_KINDS.contains(&REMOVE_SECTION_KIND));
        // 与模型自己的变体名同词（不手写第二张会漂移的表）。
        let project = filled_project();
        let section_id = *project
            .sections
            .keys()
            .next()
            .expect("样本里必须有曲式段落");
        assert_eq!(
            Op::RemoveSection {
                section_id,
                previous_section: project.sections[&section_id].clone(),
            }
            .name(),
            "RemoveSection"
        );
    }

    /// 段落形态的形状错误**响亮失败**（绝不静默丢弃），而规范形状放行。
    ///
    /// 注入（实测红）：去掉 [`reject_remove_section_fields`] 的调用 ⇒ 前三条
    /// （把别的实体的寻址 `nodeId` / `edgeId` / 工具顶层的 `trackId` 搬过来）
    /// 被**静默接受**，本判据红。
    #[test]
    fn remove_section_shapes_fail_loudly() {
        let project = filled_project();
        let section_id = *project
            .sections
            .keys()
            .next()
            .expect("样本里必须有曲式段落");
        let section_text = section_id.to_canonical_string();
        let (track_id, clip_id) = lead_clip(&project);
        let (node, _) = a_non_master_node(&project);

        for broken in [
            // 把**路由节点**的寻址搬过来: `sectionId` 缺失, 而且 `nodeId` 不是本形态的键。
            serde_json::json!([{"kind": REMOVE_SECTION_KIND, "nodeId": section_text}]),
            // 把**路由边**的寻址搬过来。
            serde_json::json!([{"kind": REMOVE_SECTION_KIND, "edgeId": section_text}]),
            // 把工具顶层的 `trackId` / `clipId` 搬过来（那是别的形态的寻址）。
            serde_json::json!([{"kind": REMOVE_SECTION_KIND, "sectionId": section_text,
                                "trackId": track_id.to_canonical_string()}]),
            serde_json::json!([{"kind": REMOVE_SECTION_KIND, "sectionId": section_text,
                                "clipId": clip_id.to_canonical_string()}]),
            // 以为要报告"段落删除前的状态"而多写 `previousSection`。
            serde_json::json!([{"kind": REMOVE_SECTION_KIND, "sectionId": section_text,
                                "previousSection": null}]),
        ] {
            let fault = parse_ops(&broken).expect_err(&format!("必须被拒: {broken}"));
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{broken}"
            );
            assert_eq!(
                lane_fault_data(&fault)["reason"],
                "unknownRemoveSectionField",
                "{broken}"
            );
            assert_eq!(
                lane_fault_data(&fault)["supportedRemoveSectionFields"],
                serde_json::json!(["kind", "sectionId"]),
                "{broken}"
            );
        }

        for broken in [
            // 缺 `sectionId`。
            serde_json::json!([{"kind": REMOVE_SECTION_KIND}]),
            // `sectionId` 不是字符串。
            serde_json::json!([{"kind": REMOVE_SECTION_KIND, "sectionId": 70}]),
            // `sectionId` 不是合法 ULID。
            serde_json::json!([{"kind": REMOVE_SECTION_KIND, "sectionId": "not-a-ulid"}]),
        ] {
            let fault = parse_ops(&broken).expect_err(&format!("必须被拒: {broken}"));
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{broken}"
            );
        }

        // 阴性对照: 规范形状必须被接受 —— 上面红的不是"全都拒"。
        let ops = parse_ops(&serde_json::json!([
            {"kind": REMOVE_SECTION_KIND, "sectionId": section_text}
        ]))
        .expect("规范形状必须被接受");
        assert!(ops[0].is_section_level());
        compile(&project, &track_id, &clip_id, &ops).expect("编译");
        // 路由节点的寻址字面量在本形态里**不是**合法的键, 但它自己仍然有效。
        assert!(project.routing_graph.nodes.contains(&node), "夹具前提");
    }

    /// 段落不存在 ⇒ 本层 `ENTITY_NOT_FOUND`（带上 `reason` 与 `sectionId`），
    /// 而且失败**不改文档**（编译期只读）。
    ///
    /// 注入（实测红）：把存在性检查换成一条**兜底**（取文档里任意一条段落当撤销载荷，
    /// 而不是报错）⇒ 本判据红，诊断里 `previousSection` 是**另一条**段落的身份
    /// （`Intro` 配上一个不存在的 `sectionId`）—— 那正是"静默取错载荷"的形状。
    #[test]
    fn remove_section_refuses_a_missing_section() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let missing = EntityId::from_str("01J8ZQ00000000000000000999").expect("ULID");
        assert!(!project.sections.contains_key(&missing), "夹具前提");

        let ops = parse_ops(&serde_json::json!([
            {"kind": REMOVE_SECTION_KIND, "sectionId": missing.to_canonical_string()}
        ]))
        .expect("解析");
        let fault = compile(&project, &track_id, &clip_id, &ops).expect_err("段落不存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::EntityNotFound));
        assert_eq!(lane_fault_data(&fault)["reason"], "sectionNotFound");
        assert_eq!(
            lane_fault_data(&fault)["sectionId"],
            serde_json::json!(missing.to_canonical_string())
        );
    }

    /// 段落形态**不要求片段是 MIDI**（一个音符都不读），而且它既不是音符级也不是路由级
    /// —— 三个谓词必须**互斥地**说真话（提案标题的分类靠它们）。
    ///
    /// 注入（实测红）：把 `RemoveSection` 从 `is_note_level` 的对照里去掉 ⇒
    /// 音频片段那条编译时撞上"必须是 MIDI 片段"（`CLIP_NOT_FOUND`）。
    #[test]
    fn remove_section_does_not_need_midi_and_is_its_own_level() {
        let project = filled_project();
        let audio_track = project
            .tracks
            .values()
            .find(|track| track.kind == yeban_model::TrackKind::Audio)
            .expect("样本里必须有音频轨")
            .id;
        let audio_clip = project
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_none())
            .expect("样本里必须有非 MIDI 片段")
            .id;
        let section_id = *project
            .sections
            .keys()
            .next()
            .expect("样本里必须有曲式段落");

        let ops = parse_ops(&serde_json::json!([
            {"kind": REMOVE_SECTION_KIND, "sectionId": section_id.to_canonical_string()}
        ]))
        .expect("解析");
        let compiled = compile(&project, &audio_track, &audio_clip, &ops)
            .expect("段落级写入不读片段内容, 非 MIDI 片段也必须被接受");
        assert_eq!(compiled.len(), 1);

        // 三个谓词在**一条**操作上不能同时说真话 (分类靠它们, 说两遍会让标题多算一步)。
        for op in &ops {
            let buckets = [
                op.is_note_level(),
                op.is_routing_level(),
                op.is_section_level(),
            ];
            assert_eq!(
                buckets.iter().filter(|flag| **flag).count(),
                1,
                "每个形态必须恰好落在一个桶里: {op:?}"
            );
        }
    }

    /// 场景形态的**字面**判据：载荷只有场景身份（撤销载荷从**当前文档**读整条场景），
    /// 一步就能被 `Op::invert` 逐字节回退到原状。
    ///
    /// 这一条对着"读侧报得出场景、工具面一个字都写不了"的缺口：`yeban_query_project`
    /// 的 `entities[]` 一直在报 `kind == "scene"` 的身份，而本形态之前**没有任何工具**
    /// 构造过 `Op::SetScene` 或 `Op::RemoveScene`（两个口径在 `crates/yeban-mcp/src`
    /// 的实测读数都是 0）。
    ///
    /// 注入（实测红）：把编译出的模型变体由 `Op::RemoveScene` 换成
    /// `Op::SetScene`（一次什么都不删的写）⇒ 变体匹配那条红；把撤销载荷换成**另一条**
    /// 场景的克隆（而不是文档里那一条）⇒ "撤销载荷必须是文档里那一条"红。
    #[test]
    fn remove_scene_compiles_and_inverts_byte_for_byte() {
        let mut project = filled_project();
        let scene_id = *project.scenes.keys().next().expect("样本里必须有场景");
        let expected = project.scenes[&scene_id].clone();
        let bytes_before = serde_json::to_string(&project).expect("序列化");
        let (track_id, clip_id) = lead_clip(&project);

        let ops = parse_ops(&serde_json::json!([
            {"kind": REMOVE_SCENE_KIND, "sceneId": scene_id.to_canonical_string()}
        ]))
        .expect("规范形状必须被接受");
        assert_eq!(ops[0].kind_name(), REMOVE_SCENE_KIND);
        assert!(!ops[0].is_note_level(), "场景级不读不写音符");
        assert!(!ops[0].is_routing_level(), "场景不是路由图的一部分");
        assert!(!ops[0].is_section_level(), "场景不是曲式段落");
        assert!(ops[0].is_scene_level());
        assert_eq!(ops[0], NoteOp::RemoveScene { scene_id });

        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::RemoveScene {
                scene_id: target,
                previous_scene,
            } => {
                assert_eq!(*target, scene_id);
                assert_eq!(
                    previous_scene, &expected,
                    "撤销载荷必须是**文档里那一条**场景 (不是调用方声明的)"
                );
            }
            other => panic!("应当是 RemoveScene: {other:?}"),
        }

        compiled[0].apply(&mut project).expect("取走");
        assert!(
            !project.scenes.contains_key(&scene_id),
            "场景必须真的从 `scenes` 里消失"
        );
        assert!(project.validate().is_ok(), "取走之后工程必须仍然合法");

        // 逆操作: `RemoveScene` 的逆是 `SetScene { old_scene: None }`
        // （`docs/adr/ADR-0001` 的 D12），因此必须把场景原样放回去。
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            project.scenes.get(&scene_id),
            Some(&expected),
            "逆操作必须把场景放回原来的形状"
        );
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "逆操作必须逐字节回到取走之前的文档"
        );
    }

    /// 场景形态的**字段名与支持集合**被钉住（不多报一个键，也不少报一个键）。
    ///
    /// 注入（实测红）：把 [`SCENE_FIELD`] 改成 `sectionId` ⇒ 本判据红 ——
    /// 场景与曲式段落是两种实体，共用一个词会让"取走的是哪一个"从形状上无法区分。
    #[test]
    fn remove_scene_field_names_are_pinned() {
        assert_eq!(REMOVE_SCENE_KIND, "removeScene");
        assert_eq!(SCENE_FIELD, "sceneId");
        assert_ne!(SCENE_FIELD, SECTION_FIELD, "场景与曲式段落不是同一个字面量");
        assert_ne!(SCENE_FIELD, ROUTING_NODE_FIELD, "场景不是路由节点");
        assert_ne!(SCENE_FIELD, ROUTING_EDGE_FIELD, "场景不是路由边");
        assert_eq!(REMOVE_SCENE_FIELDS, ["kind", "sceneId"]);
        assert!(OP_KINDS.contains(&REMOVE_SCENE_KIND));
        // 与模型自己的变体名同词（不手写第二张会漂移的表）。
        let project = filled_project();
        let scene_id = *project.scenes.keys().next().expect("样本里必须有场景");
        assert_eq!(
            Op::RemoveScene {
                scene_id,
                previous_scene: project.scenes[&scene_id].clone(),
            }
            .name(),
            "RemoveScene"
        );
    }

    /// 场景形态的形状错误**响亮失败**（绝不静默丢弃），而规范形状放行。
    ///
    /// 注入（实测红）：去掉 [`reject_remove_scene_fields`] 的调用 ⇒ 前四条
    /// （把别的实体的寻址 `sectionId` / `nodeId` / `edgeId` / 工具顶层的 `trackId`
    /// 搬过来）被**静默接受**，本判据红。
    #[test]
    fn remove_scene_shapes_fail_loudly() {
        let project = filled_project();
        let scene_id = *project.scenes.keys().next().expect("样本里必须有场景");
        let scene_text = scene_id.to_canonical_string();
        let (track_id, clip_id) = lead_clip(&project);
        let section_id = *project
            .sections
            .keys()
            .next()
            .expect("样本里必须有曲式段落");
        let (node, _) = a_non_master_node(&project);

        for broken in [
            // 把**曲式段落**的寻址搬过来: `sceneId` 缺失, 而且 `sectionId` 不是本形态的键。
            serde_json::json!([{"kind": REMOVE_SCENE_KIND, "sectionId": scene_text}]),
            // 把**路由节点**的寻址搬过来。
            serde_json::json!([{"kind": REMOVE_SCENE_KIND, "nodeId": scene_text}]),
            // 把**路由边**的寻址搬过来。
            serde_json::json!([{"kind": REMOVE_SCENE_KIND, "edgeId": scene_text}]),
            // 把工具顶层的 `trackId` / `clipId` 搬过来（那是别的形态的寻址）。
            serde_json::json!([{"kind": REMOVE_SCENE_KIND, "sceneId": scene_text,
                                "trackId": track_id.to_canonical_string()}]),
            serde_json::json!([{"kind": REMOVE_SCENE_KIND, "sceneId": scene_text,
                                "clipId": clip_id.to_canonical_string()}]),
            // 以为要报告"场景删除前的状态"而多写 `previousScene`。
            serde_json::json!([{"kind": REMOVE_SCENE_KIND, "sceneId": scene_text,
                                "previousScene": null}]),
        ] {
            let fault = parse_ops(&broken).expect_err(&format!("必须被拒: {broken}"));
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{broken}"
            );
            assert_eq!(
                lane_fault_data(&fault)["reason"],
                "unknownRemoveSceneField",
                "{broken}"
            );
            assert_eq!(
                lane_fault_data(&fault)["supportedRemoveSceneFields"],
                serde_json::json!(["kind", "sceneId"]),
                "{broken}"
            );
        }

        for broken in [
            // 缺 `sceneId`。
            serde_json::json!([{"kind": REMOVE_SCENE_KIND}]),
            // `sceneId` 不是字符串。
            serde_json::json!([{"kind": REMOVE_SCENE_KIND, "sceneId": 70}]),
            // `sceneId` 不是合法 ULID。
            serde_json::json!([{"kind": REMOVE_SCENE_KIND, "sceneId": "not-a-ulid"}]),
        ] {
            let fault = parse_ops(&broken).expect_err(&format!("必须被拒: {broken}"));
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{broken}"
            );
        }

        // 阴性对照: 规范形状必须被接受 —— 上面红的不是"全都拒"。
        let ops = parse_ops(&serde_json::json!([
            {"kind": REMOVE_SCENE_KIND, "sceneId": scene_text}
        ]))
        .expect("规范形状必须被接受");
        assert!(ops[0].is_scene_level());
        compile(&project, &track_id, &clip_id, &ops).expect("编译");
        // 别的实体的寻址字面量在本形态里**不是**合法的键, 但它们自己仍然有效。
        assert!(project.sections.contains_key(&section_id), "夹具前提");
        assert!(project.routing_graph.nodes.contains(&node), "夹具前提");
    }

    /// 场景不存在 ⇒ 本层 `ENTITY_NOT_FOUND`（带上 `reason` 与 `sceneId`），
    /// 而且失败**不改文档**（编译期只读）。
    ///
    /// 注入（实测红）：把存在性检查换成一条**兜底**（取文档里任意一条场景当撤销载荷，
    /// 而不是报错）⇒ 本判据红，诊断里 `previousScene` 是**另一条**场景的身份
    /// （`Intro` 配上一个不存在的 `sceneId`）—— 那正是"静默取错载荷"的形状。
    #[test]
    fn remove_scene_refuses_a_missing_scene() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let missing = EntityId::from_str("01J8ZQ00000000000000000999").expect("ULID");
        assert!(!project.scenes.contains_key(&missing), "夹具前提");

        let ops = parse_ops(&serde_json::json!([
            {"kind": REMOVE_SCENE_KIND, "sceneId": missing.to_canonical_string()}
        ]))
        .expect("解析");
        let fault = compile(&project, &track_id, &clip_id, &ops).expect_err("场景不存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::EntityNotFound));
        assert_eq!(lane_fault_data(&fault)["reason"], "sceneNotFound");
        assert_eq!(
            lane_fault_data(&fault)["sceneId"],
            serde_json::json!(missing.to_canonical_string())
        );
    }

    /// 场景形态**不要求片段是 MIDI**（一个音符都不读），而且它既不是音符级、也不是
    /// 路由级、也不是段落级 —— 四个谓词必须**互斥地**说真话（提案标题的分类靠它们）。
    ///
    /// 注入（实测红）：把 `RemoveScene` 从 `is_note_level` 的对照里去掉 ⇒
    /// 音频片段那条编译时撞上"必须是 MIDI 片段"（`CLIP_NOT_FOUND`）。
    #[test]
    fn remove_scene_does_not_need_midi_and_is_its_own_level() {
        let project = filled_project();
        let audio_track = project
            .tracks
            .values()
            .find(|track| track.kind == yeban_model::TrackKind::Audio)
            .expect("样本里必须有音频轨")
            .id;
        let audio_clip = project
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_none())
            .expect("样本里必须有非 MIDI 片段")
            .id;
        let scene_id = *project.scenes.keys().next().expect("样本里必须有场景");

        let ops = parse_ops(&serde_json::json!([
            {"kind": REMOVE_SCENE_KIND, "sceneId": scene_id.to_canonical_string()}
        ]))
        .expect("解析");
        let compiled = compile(&project, &audio_track, &audio_clip, &ops)
            .expect("场景级写入不读片段内容, 非 MIDI 片段也必须被接受");
        assert_eq!(compiled.len(), 1);

        // 四个谓词在**一条**操作上不能同时说真话 (分类靠它们, 说两遍会让标题多算一步)。
        for op in &ops {
            let buckets = [
                op.is_note_level(),
                op.is_routing_level(),
                op.is_section_level(),
                op.is_scene_level(),
            ];
            assert_eq!(
                buckets.iter().filter(|flag| **flag).count(),
                1,
                "每个形态必须恰好落在一个桶里: {op:?}"
            );
        }
    }

    /// 场景**写入**形态（`setScene`）的两条路：新建（`create: true`，撤销载荷为空）
    /// 与更新（缺省，撤销载荷是**文档现值**整条），两条都能被 `Op::invert` 逐字节回退。
    ///
    /// 这一条对着"读侧报得出场景、写侧只能删不能建"的缺口：上个形态关掉了取走那一半，
    /// 而 `Op::SetScene` 的构造点在 `crates/yeban-mcp/src` 里仍是 **0**（见模块头
    /// "场景写入形态"一节的对照读数）。
    ///
    /// 注入（实测红）：把 `old_scene` 从 `current` 换成 `None`（把更新当成新建）⇒
    /// 更新那条的 `old_scene` 断言红，而且模型的前置条件会在 `apply` 时拒绝它。
    #[test]
    fn set_scene_creates_and_updates_and_inverts_byte_for_byte() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);

        // ---- 新建：身份**不在**文档里，三个属性一起给 ----
        let fresh = EntityId::from_str("01J8ZQ00000000000000000777").expect("ULID");
        assert!(!project.scenes.contains_key(&fresh), "夹具前提");
        let bytes_before_create = serde_json::to_string(&project).expect("序列化");
        let ops = parse_ops(&serde_json::json!([{
            "kind": SET_SCENE_KIND,
            "scene": {
                "create": true,
                "sceneId": fresh.to_canonical_string(),
                "name": "Bridge",
                "tempo": 96.5,
                "color": "#22AA88",
            }
        }]))
        .expect("规范形状必须被接受");
        assert_eq!(ops[0].kind_name(), SET_SCENE_KIND);
        assert!(!ops[0].is_note_level(), "场景级不读不写音符");
        assert!(!ops[0].is_routing_level(), "场景不是路由图的一部分");
        assert!(!ops[0].is_section_level(), "场景不是曲式段落");
        assert!(ops[0].is_scene_level());
        assert_eq!(
            ops[0],
            NoteOp::SetScene {
                scene_id: fresh,
                create: true,
                patch: ScenePatch {
                    name: Some("Bridge".to_owned()),
                    tempo: Some(Some(96.5)),
                    color: Some(Some("#22AA88".to_owned())),
                },
            }
        );

        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::SetScene {
                scene_id,
                old_scene,
                new_scene,
            } => {
                assert_eq!(*scene_id, fresh);
                assert_eq!(
                    *old_scene, None,
                    "新建的撤销载荷必须是空的 (不是调用方声明的)"
                );
                assert_eq!(new_scene.id, fresh, "身份由寻址给出, 覆盖不可能改它");
                assert_eq!(new_scene.name, "Bridge");
                assert_eq!(new_scene.tempo, Some(96.5));
                assert_eq!(new_scene.color.as_deref(), Some("#22AA88"));
            }
            other => panic!("应当是 SetScene: {other:?}"),
        }

        compiled[0].apply(&mut project).expect("新建");
        assert!(
            project.scenes.contains_key(&fresh),
            "场景必须真的落进 `scenes`"
        );
        assert!(project.validate().is_ok(), "新建之后工程必须仍然合法");
        assert_ne!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before_create,
            "新建必须真的改了文档"
        );

        // 逆操作: `SetScene { old_scene: None }` 的逆是 `RemoveScene`（D12），
        // 因此一次 `yeban_undo` 就应当把这个新场景整条取走。
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert!(
            !project.scenes.contains_key(&fresh),
            "逆操作必须取走新建的场景"
        );
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before_create,
            "逆操作必须逐字节回到新建之前的文档"
        );

        // ---- 更新：身份**在**文档里，合并语义（只改给出来的那两个键）----
        let existing = *project.scenes.keys().next().expect("样本里必须有场景");
        let expected_before = project.scenes[&existing].clone();
        let bytes_before_update = serde_json::to_string(&project).expect("序列化");
        let ops = parse_ops(&serde_json::json!([{
            "kind": SET_SCENE_KIND,
            "scene": {
                "sceneId": existing.to_canonical_string(),
                "name": "Scene 1 (renamed)",
                "tempo": null,
            }
        }]))
        .expect("规范形状必须被接受");
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        match &compiled[0] {
            Op::SetScene {
                old_scene,
                new_scene,
                ..
            } => {
                assert_eq!(
                    old_scene.as_ref(),
                    Some(&expected_before),
                    "撤销载荷必须是**文档里那一条**场景 (不是调用方声明的)"
                );
                assert_eq!(new_scene.name, "Scene 1 (renamed)");
                assert_eq!(new_scene.tempo, None, "显式 null 必须清空速度覆盖");
                assert_eq!(
                    new_scene.color, expected_before.color,
                    "没给出来的键必须保留现值 (合并, 不是整体替换)"
                );
                assert_eq!(new_scene.id, existing);
            }
            other => panic!("应当是 SetScene: {other:?}"),
        }
        compiled[0].apply(&mut project).expect("更新");
        assert_eq!(project.scenes[&existing].name, "Scene 1 (renamed)");
        assert_eq!(project.scenes[&existing].tempo, None);
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before_update,
            "更新形态的逆操作必须逐字节回到更新之前的文档"
        );
    }

    /// 场景写入形态的**字段名与支持集合**被钉住（不多报一个键，也不少报一个键）。
    ///
    /// 注入（实测红）：把 [`SET_SCENE_KIND`] 改成 `removeScene` ⇒ 本判据红 ——
    /// 写与取走是两个相反的形态，同名会让"这次是建还是删"从字面上无法区分。
    #[test]
    fn set_scene_field_names_are_pinned() {
        assert_eq!(SET_SCENE_KIND, "setScene");
        assert_eq!(SCENE_PAYLOAD_FIELD, "scene");
        assert_eq!(SCENE_CREATE_FIELD, "create");
        assert_eq!(SCENE_NAME_FIELD, "name");
        assert_eq!(SCENE_TEMPO_FIELD, "tempo");
        assert_eq!(SCENE_COLOR_FIELD, "color");
        assert_eq!(SCENE_FIELD, "sceneId", "寻址与取走形态同词同义");
        assert_eq!(
            SET_SCENE_FIELDS,
            ["create", "sceneId", "name", "tempo", "color"]
        );
        assert_eq!(SET_SCENE_OP_FIELDS, ["kind", "scene"]);
        assert_ne!(SET_SCENE_KIND, REMOVE_SCENE_KIND, "写与取走不是同一个词");
        assert!(OP_KINDS.contains(&SET_SCENE_KIND));
        // 与模型自己的变体名同词（不手写第二张会漂移的表）。
        let project = filled_project();
        let scene_id = *project.scenes.keys().next().expect("样本里必须有场景");
        assert_eq!(
            Op::SetScene {
                scene_id,
                old_scene: None,
                new_scene: project.scenes[&scene_id].clone(),
            }
            .name(),
            "SetScene"
        );
    }

    /// 场景写入形态的形状错误**响亮失败**（绝不静默丢弃），而规范形状放行。
    ///
    /// 注入（实测红）：去掉 [`reject_set_scene_op_fields`] 的调用 ⇒ 前两条
    /// （把 `removeScene` 的形状搬过来：`sceneId` 直接放在操作对象顶层）被**静默接受**
    /// 成一个"没给 sceneId"的更新，本判据红。
    #[test]
    fn set_scene_shapes_fail_loudly() {
        let project = filled_project();
        let scene_id = *project.scenes.keys().next().expect("样本里必须有场景");
        let scene_text = scene_id.to_canonical_string();
        let (track_id, clip_id) = lead_clip(&project);
        let section_id = *project
            .sections
            .keys()
            .next()
            .expect("样本里必须有曲式段落");
        let (node, _) = a_non_master_node(&project);

        // 操作对象顶层的键：只认 `kind` 与 `scene`。
        for broken in [
            // 把**取走**形态的形状搬过来: `sceneId` 在顶层, 而载荷键缺失。
            serde_json::json!([{"kind": SET_SCENE_KIND, "sceneId": scene_text}]),
            // 顶层多写一个别的实体的寻址。
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_text,
                                "name": "x"}, "sectionId": scene_text}]),
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_text,
                                "name": "x"}, "trackId": track_id.to_canonical_string()}]),
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_text,
                                "name": "x"}, "clipId": clip_id.to_canonical_string()}]),
        ] {
            let fault = parse_ops(&broken).expect_err(&format!("必须被拒: {broken}"));
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{broken}"
            );
            assert_eq!(
                lane_fault_data(&fault)["reason"],
                "unknownSetSceneOpField",
                "{broken}"
            );
        }

        // `scene` 载荷对象里的键：只认那五个。
        for broken in [
            // 把别的实体的寻址搬进载荷对象。
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_text,
                                "name": "x", "sectionId": section_id.to_canonical_string()}}]),
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_text,
                                "name": "x", "nodeId": node.to_canonical_string()}}]),
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_text,
                                "name": "x", "edgeId": scene_text}}]),
            // 以为要报告"场景写入前的状态"而多写 `oldScene` / `previousScene`。
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_text,
                                "name": "x", "oldScene": null}}]),
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_text,
                                "name": "x", "previousScene": null}}]),
            // 拼错的属性名（模型里没有这个字段）。
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_text,
                                "name": "x", "bpm": 120.0}}]),
        ] {
            let fault = parse_ops(&broken).expect_err(&format!("必须被拒: {broken}"));
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{broken}"
            );
            assert_eq!(
                lane_fault_data(&fault)["reason"],
                "unknownSetSceneField",
                "{broken}"
            );
            assert_eq!(
                lane_fault_data(&fault)["supportedSetSceneFields"],
                serde_json::json!(["create", "sceneId", "name", "tempo", "color"]),
                "{broken}"
            );
        }

        // 载荷缺失 / 不是对象 / 寻址与文本属性形状不对。
        for broken in [
            serde_json::json!([{"kind": SET_SCENE_KIND}]),
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": 7}]),
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {}}]),
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"name": "x"}}]),
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": 70, "name": "x"}}]),
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": "not-a-ulid",
                                "name": "x"}}]),
            // `create` 不是布尔（不做真假值强转）。
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_text,
                                "create": "true", "name": "x"}}]),
            // `create: true` 而没给 `name`（模型对 name 没有校验 ⇒ 本层必拦）。
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_text,
                                "create": true}}]),
            // `name` 是空串 / 不是字符串 / 给了 null（模型里 name 不是可空字段）。
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_text,
                                "create": true, "name": ""}}]),
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_text,
                                "create": true, "name": 7}}]),
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_text,
                                "name": null}}]),
            // `color` 是空串 / 不是字符串 / 给了契约外的形状。
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_text,
                                "name": "x", "color": ""}}]),
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_text,
                                "name": "x", "color": 7}}]),
            // `tempo` 既不是数字也不是 null（布尔/字符串/对象都不折叠）。
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_text,
                                "name": "x", "tempo": "128"}}]),
            serde_json::json!([{"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_text,
                                "name": "x", "tempo": true}}]),
        ] {
            let fault = parse_ops(&broken).expect_err(&format!("必须被拒: {broken}"));
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{broken}"
            );
        }

        // 阴性对照: 一个只改 `color` 的最小更新必须被接受 —— 上面红的不是"全都拒"。
        let ops = parse_ops(&serde_json::json!([
            {"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_text, "color": "#123456"}}
        ]))
        .expect("规范形状必须被接受");
        assert!(ops[0].is_scene_level());
        compile(&project, &track_id, &clip_id, &ops).expect("编译");
    }

    /// 场景写入的两条**存在性**规则（响亮、且各自带 `reason`）：
    /// 更新一个不存在的场景 / 新建一个已存在的场景。
    ///
    /// 注入（实测红）：把 `create` 分支删掉（一律当更新）⇒ 第一条的
    /// `sceneAlreadyExists` 断言红（它会掉进"没有场景可以更新"那条）。
    #[test]
    fn set_scene_refuses_missing_on_update_and_existing_on_create() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let existing = *project.scenes.keys().next().expect("样本里必须有场景");
        let missing = EntityId::from_str("01J8ZQ00000000000000000999").expect("ULID");
        assert!(!project.scenes.contains_key(&missing), "夹具前提");

        // 更新一个不存在的场景 ⇒ ENTITY_NOT_FOUND (与 removeScene 同一个 reason)。
        let ops = parse_ops(&serde_json::json!([
            {"kind": SET_SCENE_KIND, "scene": {"sceneId": missing.to_canonical_string(),
                                               "name": "x"}}
        ]))
        .expect("解析");
        let fault = compile(&project, &track_id, &clip_id, &ops).expect_err("场景不存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::EntityNotFound));
        assert_eq!(lane_fault_data(&fault)["reason"], "sceneNotFound");
        assert_eq!(
            lane_fault_data(&fault)["sceneId"],
            serde_json::json!(missing.to_canonical_string())
        );

        // 新建一个**已存在**的场景 ⇒ CONFLICT (与 `create: true` 的 clipAlreadyExists 同口径)。
        let ops = parse_ops(&serde_json::json!([
            {"kind": SET_SCENE_KIND, "scene": {"create": true,
                                               "sceneId": existing.to_canonical_string(),
                                               "name": "x"}}
        ]))
        .expect("解析");
        let fault = compile(&project, &track_id, &clip_id, &ops).expect_err("场景已存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::Conflict));
        assert_eq!(lane_fault_data(&fault)["reason"], "sceneAlreadyExists");

        // 编译期只读: 两次失败都不许改文档。
        assert_eq!(project, filled_project(), "编译期失败不得改文档");
    }

    /// 场景写入形态**不要求片段是 MIDI**（一个音符都不读），落**场景级**那个桶，
    /// 而且同一个场景在一次调用里写两次被 [`reject_duplicate_scene_targets`] 响亮拒绝。
    ///
    /// 注入（实测红）：把 `SetScene` 从 `is_note_level` 的对照里去掉 ⇒ 音频片段那条
    /// 编译时撞上"必须是 MIDI 片段"（`CLIP_NOT_FOUND`）；把 `reject_duplicate_scene_targets`
    /// 的循环体去掉 ⇒ 重复那条不再报错。
    #[test]
    fn set_scene_does_not_need_midi_and_refuses_a_duplicate_target() {
        let project = filled_project();
        let audio_track = project
            .tracks
            .values()
            .find(|track| track.kind == yeban_model::TrackKind::Audio)
            .expect("样本里必须有音频轨")
            .id;
        let audio_clip = project
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_none())
            .expect("样本里必须有非 MIDI 片段")
            .id;
        let scene_id = *project.scenes.keys().next().expect("样本里必须有场景");

        let ops = parse_ops(&serde_json::json!([
            {"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_id.to_canonical_string(),
                                               "name": "Scene 1 (again)"}}
        ]))
        .expect("解析");
        let compiled = compile(&project, &audio_track, &audio_clip, &ops)
            .expect("场景级写入不读片段内容, 非 MIDI 片段也必须被接受");
        assert_eq!(compiled.len(), 1);

        // 四个谓词在**一条**操作上不能同时说真话 (分类靠它们, 说两遍会让标题多算一步)。
        for op in &ops {
            let buckets = [
                op.is_note_level(),
                op.is_routing_level(),
                op.is_section_level(),
                op.is_scene_level(),
            ];
            assert_eq!(
                buckets.iter().filter(|flag| **flag).count(),
                1,
                "每个形态必须恰好落在一个桶里: {op:?}"
            );
        }

        // 同一个场景写两次: 第二条的 `old_scene` 与那一刻的文档现值必然不符 ⇒ 提前拒。
        let duplicate = parse_ops(&serde_json::json!([
            {"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_id.to_canonical_string(),
                                               "name": "a"}},
            {"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_id.to_canonical_string(),
                                               "tempo": 100.0}}
        ]))
        .expect("解析");
        let fault = reject_duplicate_scene_targets(&duplicate).expect_err("同一场景写两次");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        assert_eq!(lane_fault_data(&fault)["reason"], "duplicateSceneTarget");
        assert_eq!(
            lane_fault_data(&fault)["sceneId"],
            serde_json::json!(scene_id.to_canonical_string())
        );
        // 阴性对照: 单条 / 不同身份的两条都必须放行 (防"全都拒")。
        assert!(reject_duplicate_scene_targets(&ops).is_ok());
        let other = EntityId::from_str("01J8ZQ00000000000000000778").expect("ULID");
        let two = parse_ops(&serde_json::json!([
            {"kind": SET_SCENE_KIND, "scene": {"create": true,
                                               "sceneId": other.to_canonical_string(),
                                               "name": "a"}},
            {"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_id.to_canonical_string(),
                                               "name": "b"}}
        ]))
        .expect("解析");
        assert!(reject_duplicate_scene_targets(&two).is_ok());
    }

    /// 场景写入的 `tempo` **值域**由模型判（本层不复制第二份真相），
    /// 而失败**不改文档**。
    ///
    /// 注入（实测红）：把 `crates/yeban-mcp/src/domain/error.rs` 的
    /// `ModelError::BpmOutOfRange` 从 `OUT_OF_RANGE` 挪走 ⇒ 本判据红。
    #[test]
    fn set_scene_tempo_range_is_judged_by_the_model() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let scene_id = *project.scenes.keys().next().expect("样本里必须有场景");

        let ops = parse_ops(&serde_json::json!([
            {"kind": SET_SCENE_KIND, "scene": {"sceneId": scene_id.to_canonical_string(),
                                               "tempo": 100_000.0}}
        ]))
        .expect("解析: 值域不在本层判");
        // 本层只编译, 值域那条由模型在 `apply` 里报（`Op::SetScene` 的 precondition
        // 调 `SceneV3::validate`）—— 契约码经 `code_for_model` 落到 OUT_OF_RANGE。
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        let mut probe = project.clone();
        let failure = compiled[0]
            .apply(&mut probe)
            .expect_err("模型必须拒绝越界的速度");
        assert_eq!(
            super::super::error::code_for_model(&failure),
            ErrorCode::OutOfRange,
            "{failure:?}"
        );
        assert_eq!(probe, project, "失败不得改文档");
    }

    ///
    /// 注入（实测红）：删掉 mixed 那条分支 ⇒ 第一条红；删掉 `placement_present` 那条分支
    /// ⇒ 第二条红；把"没有 `removeClip` 时什么都不做"去掉 ⇒ 末条红。
    #[test]
    fn remove_clip_conflicts_fail_loudly() {
        let single = parse_ops(&serde_json::json!([{"kind": "removeClip"}])).expect("解析");
        assert!(
            reject_remove_clip_conflicts(&single, false).is_ok(),
            "阴性对照: 单独出现且没有摆放 ⇒ 放行"
        );

        let mixed = parse_ops(&serde_json::json!([
            {"kind": "removeClip"},
            {"kind": "velocity", "noteId": "01J8Z0000000000000000000AB", "velocity": 40}
        ]))
        .expect("解析");
        let fault = reject_remove_clip_conflicts(&mixed, false).expect_err("必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "removeClipTakesNoOtherOps"
        );
        assert_eq!(
            lane_fault_data(&fault)["opKinds"],
            serde_json::json!(["removeClip", "velocity"]),
            "报出的必须是本次真给的 kind 列表"
        );

        let fault = reject_remove_clip_conflicts(&single, true).expect_err("必须被拒");
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "removeClipIsNotPlacement"
        );

        // 没有 `removeClip` 的调用逐字节等于旧行为（摆放形态也照旧）。
        let note_ops = parse_ops(&serde_json::json!([
            {"kind": "add", "note": {"startTick": 0, "pitch": 60, "durationTicks": 480}}
        ]))
        .expect("解析");
        assert!(
            reject_remove_clip_conflicts(&note_ops, true).is_ok(),
            "别的形态不受这两条规则影响"
        );
    }

    #[test]
    fn peak_polyphony_ignores_abutting_notes() {
        assert_eq!(peak_polyphony([] as [(u64, u64); 0]), 0);
        assert_eq!(peak_polyphony([(0, 100)]), 1);
        // 首尾相接不算重叠。
        assert_eq!(peak_polyphony([(0, 100), (100, 100)]), 1);
        // 真正重叠。
        assert_eq!(peak_polyphony([(0, 200), (100, 100)]), 2);
        assert_eq!(peak_polyphony([(0, 1000), (0, 1000), (0, 1000)]), 3);
    }

    #[test]
    fn polyphony_guard_is_reachable_and_bounded() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let mut ops = Vec::new();
        for index in 0..(MAX_POLYPHONY + 1) {
            let id = deterministic_id(&format!("flood-{index}"));
            let mut note = MidiNote::new(id, 0, 60, 960);
            note.velocity = 100;
            ops.push(NoteOp::Add {
                note: Box::new(note),
            });
        }
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        let fault = check_polyphony(&project, &clip_id, &compiled).expect_err("必须越界");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
        // 样本片段在 tick 0 已经有一个音符, 而注入的音符都在 tick 0 ⇒ 峰值 = 注入数 + 1。
        // 因此去掉 2 个（33 - 2 + 1 = 32）刚好落在上限之内。
        let ok = compile(&project, &track_id, &clip_id, &ops[2..]).expect("编译");
        assert_eq!(
            check_polyphony(&project, &clip_id, &ok).expect("上限之内"),
            MAX_POLYPHONY
        );
    }

    // -----------------------------------------------------------------------
    // 概率触发字段（`add.note.probability`）—— 让模型能力在工具面可达
    // -----------------------------------------------------------------------

    /// 缺省不写 ⇒ `None`（= 必然触发 = 加这个字段之前的逐字节行为）。
    #[test]
    fn add_without_probability_stays_none() {
        let ops = parse_ops(&Value::Array(vec![add_json(0, 60)])).expect("解析");
        let NoteOp::Add { note } = &ops[0] else {
            panic!("必须是 add");
        };
        assert_eq!(note.probability, None, "缺省必须是不写这个字段");
    }

    /// 给了就**逐值**搬进 `MidiNote`（判定不在这一层）。
    #[test]
    fn add_carries_the_probability_verbatim() {
        for (literal, expected) in [(0.0f64, 0.0f32), (0.5, 0.5), (1.0, 1.0), (0.25, 0.25)] {
            let mut item = add_json(0, 60);
            item["note"][PROBABILITY_FIELD] = serde_json::json!(literal);
            let ops = parse_ops(&Value::Array(vec![item])).expect("解析");
            let NoteOp::Add { note } = &ops[0] else {
                panic!("必须是 add");
            };
            assert_eq!(note.probability, Some(expected), "字面值 {literal}");
        }
    }

    /// 越界 ⇒ `OUT_OF_RANGE`（带 `field` / `value` / `min` / `max`），不是静默夹紧。
    #[test]
    fn probability_out_of_range_is_out_of_range() {
        for bad in [-0.001f64, 1.001, 2.0, 1e9] {
            let mut item = add_json(0, 60);
            item["note"][PROBABILITY_FIELD] = serde_json::json!(bad);
            let fault = parse_ops(&Value::Array(vec![item])).expect_err("必须拒绝");
            assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange), "值 {bad}");
            let Fault::Domain { data, .. } = &fault else {
                panic!("必须是领域失败");
            };
            let data = data.as_ref().expect("必须带 data");
            assert_eq!(data["field"], PROBABILITY_FIELD);
            assert_eq!(data["min"], 0.0);
            assert_eq!(data["max"], 1.0);
        }
    }

    /// 形状错（不是数字）⇒ `INVALID_PARAMETER_RANGE`，而不是被当成 0 或 1。
    #[test]
    fn probability_must_be_a_number() {
        for bad in [
            serde_json::json!("0.5"),
            serde_json::json!(true),
            serde_json::json!([0.5]),
        ] {
            let mut item = add_json(0, 60);
            item["note"][PROBABILITY_FIELD] = bad.clone();
            let fault = parse_ops(&Value::Array(vec![item])).expect_err("必须拒绝");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "值 {bad}"
            );
        }
    }

    /// 概率随 `add` 一路进 `Op::AddNote`（`create: true` 的新片段也一样），
    /// 因此它**真的**进工程、也**真的**可回退。
    #[test]
    fn probability_reaches_the_add_note_op_and_the_create_form() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let mut item = add_json(0, 72);
        item["note"][PROBABILITY_FIELD] = serde_json::json!(0.5);
        let ops = parse_ops(&Value::Array(vec![item])).expect("解析");
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        match &compiled[0] {
            Op::AddNote { note, .. } => assert_eq!(note.probability, Some(0.5)),
            other => panic!("必须是 AddNote, 实际 {other:?}"),
        }
        // `create: true` 走同一个 [`parse_note`] ⇒ 新片段里的音符也带着它。
        let fresh = project_without_midi_clips();
        let seed_id = deterministic_id("probability-create");
        let created = compile_create(&fresh, &track_id, &seed_id, "Seed", &ops).expect("创建");
        let Op::AddClip { clip } = &created[0] else {
            panic!("必须是 AddClip");
        };
        let note = clip
            .content
            .notes()
            .expect("MIDI")
            .values()
            .next()
            .expect("至少一个音符");
        assert_eq!(note.probability, Some(0.5));
    }

    // -----------------------------------------------------------------------
    // 连击 / 微时序字段（`add.note.ratchet` / `add.note.microTimingTicks`）
    // —— 渲染器**已经**按模型语义实现这两个字段，工具面此前写不进去
    // -----------------------------------------------------------------------

    /// 缺省不写 ⇒ `None`（= `ratchet` 等价于 1、微时序等价于 0 = 加这两个字段之前的
    /// 逐字节行为）。
    #[test]
    fn add_without_ratchet_or_micro_timing_stays_none() {
        let ops = parse_ops(&Value::Array(vec![add_json(0, 60)])).expect("解析");
        let NoteOp::Add { note } = &ops[0] else {
            panic!("必须是 add");
        };
        assert_eq!(note.ratchet, None, "缺省必须是不写这个字段");
        assert_eq!(note.micro_timing_ticks, None, "缺省必须是不写这个字段");
    }

    /// 字段名的字面拼写被钉住（工具面的实参名是契约的一部分，改名会让既有 Agent 静默降级），
    /// 且支持集合与 `parse_note` 真正读的键**同源**（多报一个键就是假话）。
    #[test]
    fn expressive_note_field_names_are_pinned() {
        assert_eq!(RATCHET_FIELD, "ratchet");
        assert_eq!(MICRO_TIMING_FIELD, "microTimingTicks");
        for expected in [
            "id",
            "startTick",
            "pitch",
            "durationTicks",
            "velocity",
            "probability",
            "ratchet",
            "microTimingTicks",
        ] {
            assert!(
                NOTE_FIELDS.contains(&expected),
                "支持集合必须含 {expected}: {NOTE_FIELDS:?}"
            );
        }
        assert_eq!(
            NOTE_FIELDS.len(),
            8,
            "支持集合不得多报未读的键: {NOTE_FIELDS:?}"
        );
    }

    /// 给了就**逐值**搬进 `MidiNote`，并一路进 `Op::AddNote`。
    #[test]
    fn ratchet_and_micro_timing_reach_the_add_note_op() {
        for (ratchet, micro) in [(1u8, 0i16), (2, -12), (16, 240), (4, -240)] {
            let mut item = add_json(0, 64);
            item["note"]["ratchet"] = serde_json::json!(ratchet);
            item["note"]["microTimingTicks"] = serde_json::json!(micro);
            let ops = parse_ops(&Value::Array(vec![item])).expect("解析");
            let NoteOp::Add { note } = &ops[0] else {
                panic!("必须是 add");
            };
            assert_eq!(note.ratchet, Some(ratchet), "ratchet 字面值");
            assert_eq!(note.micro_timing_ticks, Some(micro), "微时序字面值");

            let project = filled_project();
            let (track_id, clip_id) = lead_clip(&project);
            let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
            match &compiled[0] {
                Op::AddNote { note, .. } => {
                    assert_eq!(note.ratchet, Some(ratchet));
                    assert_eq!(note.micro_timing_ticks, Some(micro));
                }
                other => panic!("必须是 AddNote, 实际 {other:?}"),
            }
        }
    }

    /// 越界 ⇒ `OUT_OF_RANGE`（带 `field` / `value` / `min` / `max`），不是静默夹紧。
    #[test]
    fn ratchet_and_micro_timing_out_of_range_are_out_of_range() {
        for bad in [0i64, 17, -1, 200] {
            let mut item = add_json(0, 60);
            item["note"]["ratchet"] = serde_json::json!(bad);
            let fault = parse_ops(&Value::Array(vec![item])).expect_err("必须拒绝");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::OutOfRange),
                "ratchet {bad}"
            );
            let Fault::Domain { data, .. } = &fault else {
                panic!("必须是领域失败");
            };
            let data = data.as_ref().expect("必须带 data");
            assert_eq!(data["field"], "ratchet");
            assert_eq!(data["min"], 1);
            assert_eq!(data["max"], 16);
        }
        for bad in [241i64, -241, 1000] {
            let mut item = add_json(0, 60);
            item["note"]["microTimingTicks"] = serde_json::json!(bad);
            let fault = parse_ops(&Value::Array(vec![item])).expect_err("必须拒绝");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::OutOfRange),
                "微时序 {bad}"
            );
            let Fault::Domain { data, .. } = &fault else {
                panic!("必须是领域失败");
            };
            let data = data.as_ref().expect("必须带 data");
            assert_eq!(data["field"], "microTimingTicks");
            assert_eq!(data["min"], -240);
            assert_eq!(data["max"], 240);
        }
    }

    /// 形状错（不是整数）⇒ `INVALID_PARAMETER_RANGE`，而不是被截断、取整或当成缺省。
    #[test]
    fn ratchet_and_micro_timing_must_be_integers() {
        for bad in [
            serde_json::json!(2.5),
            serde_json::json!("4"),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!([4]),
        ] {
            for field in ["ratchet", "microTimingTicks"] {
                let mut item = add_json(0, 60);
                item["note"][field] = bad.clone();
                let fault = parse_ops(&Value::Array(vec![item])).expect_err("必须拒绝");
                assert_eq!(
                    fault.domain_code(),
                    Some(ErrorCode::InvalidParameterRange),
                    "{field} = {bad}"
                );
            }
        }
    }

    /// `note` 里的未知键 ⇒ 响亮拒绝（列出支持集合），**绝不**静默丢弃。
    ///
    /// 与顶层实参口径同源（`tools.rs` 的 `UnknownParam`：拼错的参数必须被拒绝、不能静默
    /// 忽略）—— 同一条纪律不许只守一层。`slide` / `pitchBendCurve` / `syllable` /
    /// `phonemes` 四个模型字段**仍然**没有工具面通路，它们必须**响亮地**说出来，
    /// 而不是原样吞掉。
    #[test]
    fn unknown_note_fields_are_rejected_with_the_supported_set() {
        for unknown in ["slyde", "ratchett", "microtimingticks", "noteId", "slide"] {
            let mut item = add_json(0, 60);
            item["note"][unknown] = serde_json::json!(1);
            let fault = parse_ops(&Value::Array(vec![item])).expect_err("必须拒绝");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{unknown}"
            );
            let Fault::Domain { data, .. } = &fault else {
                panic!("必须是领域失败");
            };
            let data = data.as_ref().expect("必须带 data");
            assert_eq!(data["field"], unknown);
            let supported = data["supportedNoteFields"].as_array().expect("必须是数组");
            for expected in ["startTick", "pitch", "ratchet", "microTimingTicks"] {
                assert!(
                    supported.iter().any(|value| value == expected),
                    "支持集合必须含 {expected}: {data}"
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // 材料创建形态（`create: true`）—— 关闭 needs-8 的 MIDI 那一半
    // -----------------------------------------------------------------------

    /// 一条 `add` 的 JSON（身份缺省 ⇒ 由 `parse_note` 确定性派生）。
    fn add_json(start: u64, pitch: u8) -> Value {
        serde_json::json!({
            "kind": "add",
            "note": {"startTick": start, "pitch": pitch, "durationTicks": 480},
        })
    }

    /// 池子里**没有** MIDI 片段的工程（needs-8 的负样本：只剩音频条目）。
    fn project_without_midi_clips() -> YebanProjectV1 {
        let mut project = filled_project();
        project
            .clip_pool
            .retain(|_, entry| entry.content.notes().is_none());
        project
    }

    /// `create: true` ⇒ 恰好**一条** `Op::AddClip`，内容 = 全部 `add`，名字/身份逐字段可控。
    #[test]
    fn create_compiles_adds_into_one_add_clip_op() {
        let project = project_without_midi_clips();
        assert!(
            project
                .clip_pool
                .values()
                .all(|entry| entry.content.notes().is_none()),
            "负样本里不得有 MIDI 片段"
        );
        let (track_id, _) = lead_clip(&filled_project());
        let clip_id = deterministic_id("clip:needs-8:material");
        let ops =
            parse_ops(&serde_json::json!([add_json(0, 60), add_json(480, 64)])).expect("解析");

        let compiled = compile_create(&project, &track_id, &clip_id, "Seed", &ops).expect("创建");
        assert_eq!(compiled.len(), 1, "整批 `add` 只折成一条 Op");
        let Op::AddClip { clip } = &compiled[0] else {
            panic!("必须是 Op::AddClip, 实际 {}", compiled[0].name());
        };
        assert_eq!(clip.id, clip_id);
        assert_eq!(clip.name, "Seed");
        let notes = clip.content.notes().expect("必须是 MIDI 内容");
        assert_eq!(notes.len(), 2);
        // `notes` 是 `BTreeMap<EntityId, _>` ⇒ 迭代序是**身份序**，不是插入序。
        let mut pitches: Vec<u8> = notes.values().map(|note| note.pitch).collect();
        pitches.sort_unstable();
        assert_eq!(pitches, vec![60, 64]);

        // 确定性：同一请求 ⇒ 同一份载荷（`dryRun` 预览才能等于真做）。
        let again = compile_create(&project, &track_id, &clip_id, "Seed", &ops).expect("创建");
        assert_eq!(compiled, again, "同一请求必须产出逐字段相同的 op");

        // 结果真的进得了池子，且**这就是** `propose_section` 要的材料口径。
        let mut after = project.clone();
        Op::Batch {
            ops: compiled,
            description: "判据".to_owned(),
        }
        .apply(&mut after)
        .expect("施加");
        let entry = after.clip_pool.get(&clip_id).expect("池里必须有新条目");
        assert!(
            entry.content.notes().is_some_and(|notes| !notes.is_empty()),
            "新条目必须是 `usable_materials` 认的形态 (MIDI 且至少一个音符)"
        );
    }

    /// `create: true` 且池里已有该身份 ⇒ `CONFLICT`（绝不覆盖别人的片段）。
    #[test]
    fn create_refuses_an_existing_clip_identity() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let ops = parse_ops(&serde_json::json!([add_json(0, 60)])).expect("解析");
        let fault =
            compile_create(&project, &track_id, &clip_id, "Seed", &ops).expect_err("必须拒绝");
        assert_eq!(fault.domain_code(), Some(ErrorCode::Conflict));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["code"], "CONFLICT");
        assert_eq!(value["error"]["data"]["reason"], "clipAlreadyExists");
    }

    /// `create: true` 只允许 `add`；`delete`/`move`/`velocity` 必须响亮失败。
    #[test]
    fn create_refuses_every_non_add_operation() {
        let project = project_without_midi_clips();
        let (track_id, _) = lead_clip(&filled_project());
        let clip_id = deterministic_id("clip:needs-8:kinds");
        let note = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        for (kind, op) in [
            (
                "delete",
                serde_json::json!({"kind": "delete", "noteId": note}),
            ),
            (
                "move",
                serde_json::json!({"kind": "move", "noteId": note, "deltaTick": 1, "deltaPitch": 0}),
            ),
            (
                "velocity",
                serde_json::json!({"kind": "velocity", "noteId": note, "velocity": 1}),
            ),
            // 两个音轨级开关与建材料**无关**：`create: true` 只收 `add`。
            (
                "setTrackMute",
                serde_json::json!({"kind": "setTrackMute", "value": true}),
            ),
            (
                "setTrackSolo",
                serde_json::json!({"kind": "setTrackSolo", "value": true}),
            ),
        ] {
            let mut items = vec![add_json(0, 60)];
            items.push(op);
            let ops = parse_ops(&Value::Array(items)).expect("解析");
            let fault =
                compile_create(&project, &track_id, &clip_id, "Seed", &ops).expect_err("必须拒绝");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{kind}"
            );
            let value = fault.into_result().expect("带内");
            assert_eq!(value["error"]["data"]["reason"], "createRequiresAddOps");
            assert_eq!(value["error"]["data"]["received"], kind);
        }
    }

    /// 两个 `add` 抢同一个音符身份 ⇒ 响亮失败（不静默去重）。
    #[test]
    fn create_refuses_duplicate_note_identities() {
        let project = project_without_midi_clips();
        let (track_id, _) = lead_clip(&filled_project());
        let clip_id = deterministic_id("clip:needs-8:dupes");
        let shared = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        let ops = parse_ops(&serde_json::json!([
            {"kind": "add", "note": {"id": shared, "startTick": 0, "pitch": 60, "durationTicks": 480}},
            {"kind": "add", "note": {"id": shared, "startTick": 0, "pitch": 64, "durationTicks": 480}},
        ]))
        .expect("解析");
        let fault =
            compile_create(&project, &track_id, &clip_id, "Seed", &ops).expect_err("必须拒绝");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["reason"], "duplicateNoteId");
    }

    /// 空音符集不是可用材料 —— 直接调 [`compile_create`]（**绕过** `parse_ops` 的
    /// "空数组不是一次编辑请求"那条规则）才能碰到这个分支。
    ///
    /// 为什么值得一条判据：`parse_ops` 今天恰好拦住了空数组，于是这个守卫从工具面
    /// **不可达**；不可达的守卫没有任何判据能证明它还在（注入证明：删掉它，全绿）。
    /// 这条判据让它可达一次，代价是一行 `Vec::new()`。
    #[test]
    fn create_refuses_an_empty_note_set() {
        let project = project_without_midi_clips();
        let (track_id, _) = lead_clip(&filled_project());
        let clip_id = deterministic_id("clip:needs-8:empty");
        let fault = compile_create(&project, &track_id, &clip_id, "Seed", &[])
            .expect_err("空音符集必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["code"], "INVALID_PARAMETER_RANGE");
    }

    /// 发声数上限在**材料创建**这条路上也有牙：`check_polyphony` 先施加 `ops`
    /// 再读池子，因此 `compile_create` 的 `Op::AddClip` 一进模拟体，读到的
    /// 就是**新片段**的峰值 —— 换句话说，创建形态不需要第二份发声数检查，
    /// 但它继承了同一份上限。判据钉住这一点（把 `check_polyphony` 从创建路径上
    /// 摘掉，这条会红）。
    #[test]
    fn create_enforces_the_polyphony_limit() {
        let project = project_without_midi_clips();
        let (track_id, _) = lead_clip(&filled_project());
        let clip_id = deterministic_id("clip:needs-8:polyphony");
        let flood: Vec<Value> = (0..=MAX_POLYPHONY)
            .map(|index| {
                // ⚠ 音高必须**逐条不同**：`parse_note` 在缺 `id` 时按
                // `(start, pitch, duration)` 派生身份，重复的音高会撞成 duplicateNoteId。
                serde_json::json!({
                    "kind": "add",
                    "note": {
                        "startTick": 0,
                        "pitch": u8::try_from(60 + index).expect("音高"),
                        "durationTicks": 960,
                    },
                })
            })
            .collect();
        let ops = parse_ops(&Value::Array(flood)).expect("解析");
        let compiled = compile_create(&project, &track_id, &clip_id, "Seed", &ops).expect("建");
        let fault = check_polyphony(&project, &clip_id, &compiled).expect_err("必须越界");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["peak"], MAX_POLYPHONY + 1);
        assert_eq!(value["error"]["data"]["limit"], MAX_POLYPHONY);
        // 去掉一个 ⇒ 峰值 = 上限 ⇒ 通过（上限本身合法）。
        let ok_ops = parse_ops(&Value::Array(
            (1..=MAX_POLYPHONY)
                .map(|index| {
                    serde_json::json!({
                        "kind": "add",
                        "note": {
                            "startTick": 0,
                            "pitch": u8::try_from(60 + index).expect("音高"),
                            "durationTicks": 960,
                        },
                    })
                })
                .collect(),
        ))
        .expect("解析");
        let ok = compile_create(&project, &track_id, &clip_id, "Seed", &ok_ops).expect("建");
        assert_eq!(
            check_polyphony(&project, &clip_id, &ok).expect("上限之内"),
            MAX_POLYPHONY
        );
    }

    // -----------------------------------------------------------------------
    // 摆放形态（`arguments.placement`）—— 关闭 needs-6 的"放置/引用片段"那一半
    // -----------------------------------------------------------------------

    /// 一份 `placement` 实参（键序稳定：判据要逐字钉住错误载荷）。
    fn placement_args(value: Value) -> Map<String, Value> {
        let mut arguments = Map::new();
        arguments.insert(PLACEMENT_FIELD.to_owned(), value);
        arguments
    }

    /// 把一组 op 当成**一次提交**来施加/回退（与 `propose_draft` 的封装同一形状）。
    fn batch(ops: &[Op]) -> Op {
        Op::Batch {
            ops: ops.to_vec(),
            description: "判据".to_owned(),
        }
    }

    /// 一个**只有一条 MIDI 片段、且该片段还没被摆放**的工程
    /// （阴性前提：摆放判据必须能看到"池里有、时间轴上没有"这个真实状态）。
    fn unplaced_midi_clip() -> (YebanProjectV1, EntityId, EntityId) {
        let mut project = filled_project();
        // 把每一条音轨上的摆放全部清掉（池子不动）—— 于是池里的 MIDI 片段
        // 一条都没上时间轴，正是 needs-6 描述的状态。
        for track in project.tracks.values_mut() {
            track.clips.clear();
        }
        let (track_id, clip_id) = lead_clip(&project);
        assert!(
            project.tracks.values().all(|track| track.clips.is_empty()),
            "阴性前提: 时间轴上必须没有任何摆放"
        );
        assert!(project.clip_pool.contains_key(&clip_id), "材料必须在池子里");
        (project, track_id, clip_id)
    }

    /// 字段名与支持集合被钉住（不多报一个键，也不少报一个）。
    #[test]
    fn placement_field_names_are_pinned() {
        assert_eq!(PLACEMENT_FIELD, "placement");
        assert_eq!(
            PLACEMENT_FIELDS,
            ["startTick", "durationTicks", "placementId", "muted"]
        );
    }

    /// 缺省（不给 `placement`）= `None` = 不摆放：接线之前的行为逐字节不变。
    #[test]
    fn no_placement_argument_means_no_placement() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        let parsed = parse_placement(&project, &track_id, &clip_id, &Map::new()).expect("缺省");
        assert_eq!(parsed, None);
    }

    /// 四个键全部缺省时：起点 0、时值 = 片段内容长度、身份由标签确定性派生、不静音。
    #[test]
    fn placement_defaults_come_from_the_clip_content() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        let placement = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({})),
        )
        .expect("解析")
        .expect("在场");
        assert_eq!(placement.clip_id, clip_id);
        assert_eq!(placement.start_tick, 0);
        assert!(!placement.muted);
        assert_eq!(
            placement.loop_config,
            LoopConfig::default(),
            "刻意不暴露循环旋钮 (渲染没有展开它)"
        );
        // 时值 = 片段里最后一个音符的结束 tick（不猜、不夹紧）。
        let expected = project.clip_pool[&clip_id]
            .content
            .notes()
            .expect("MIDI")
            .values()
            .map(|note| note.start_tick + note.duration_ticks)
            .max()
            .expect("至少一个音符");
        assert_eq!(placement.duration_ticks, expected);
        // 身份 = `placement_label` 的确定性派生（同一请求 ⇒ 同一身份）。
        assert_eq!(
            placement.id,
            deterministic_id(&placement_label(
                &clip_id.to_canonical_string(),
                &track_id.to_canonical_string(),
                0
            ))
        );
        // 两条独立事实：同一个标签两次派生必须同值；不同起点必须不同值。
        assert_eq!(
            deterministic_id(&placement_label("c", "t", 0)),
            deterministic_id(&placement_label("c", "t", 0))
        );
        assert_ne!(
            deterministic_id(&placement_label("c", "t", 0)),
            deterministic_id(&placement_label("c", "t", 1))
        );
    }

    /// 显式给的两个键**逐字**落进载荷（起点进标签 ⇒ 换起点换身份）。
    #[test]
    fn explicit_start_and_duration_reach_the_placement() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        let explicit = deterministic_id("placement:explicit");
        let placement = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({
                "startTick": 7680,
                "durationTicks": 960,
                "placementId": explicit.to_canonical_string(),
                "muted": true,
            })),
        )
        .expect("解析")
        .expect("在场");
        assert_eq!(placement.start_tick, 7680);
        assert_eq!(placement.duration_ticks, 960);
        assert_eq!(placement.id, explicit);
        assert!(placement.muted);
    }

    /// `durationTicks` 推不出来的两种片段都必须**响亮**要求显式给（不猜假长度）。
    #[test]
    fn duration_must_be_explicit_when_the_clip_cannot_derive_it() {
        let (mut project, track_id, clip_id) = unplaced_midi_clip();
        // ① 空 MIDI 片段：把音符清空。
        let notes = project
            .clip_pool
            .get_mut(&clip_id)
            .and_then(|entry| entry.content.notes_mut())
            .expect("MIDI 片段");
        notes.clear();
        let fault = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({})),
        )
        .expect_err("空片段推不出长度");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["reason"], "durationNotDerivable");
        assert_eq!(value["error"]["data"]["isMidi"], true);
        // 显式给时值 ⇒ 空 MIDI 片段也能摆（诚实: 它只是不出声）。
        let placed = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"durationTicks": 960})),
        )
        .expect("显式时值")
        .expect("在场");
        assert_eq!(placed.duration_ticks, 960);
        // ② 非 MIDI 片段（音频条目）同理。
        let project = filled_project();
        let audio = project
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_none())
            .expect("样本里必须有音频条目")
            .id;
        let fault = parse_placement(
            &project,
            &track_id,
            &audio,
            &placement_args(serde_json::json!({})),
        )
        .expect_err("音频片段推不出长度");
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["reason"], "durationNotDerivable");
        assert_eq!(value["error"]["data"]["isMidi"], false);
    }

    /// 时值 0 在**这一层**就被拒（模型层同样拒绝；这里多给 `field`/`value` 的结构化载荷）。
    #[test]
    fn zero_duration_is_refused_with_the_field() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        let fault = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"durationTicks": 0})),
        )
        .expect_err("零时值必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["field"], "placement.durationTicks");
        assert_eq!(value["error"]["data"]["value"], 0);
    }

    /// 未知键**响亮拒绝**并列出支持集合（与 `note` 的未知键同一口径，绝不静默丢弃）。
    #[test]
    fn unknown_placement_fields_are_rejected_with_the_supported_set() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        for unknown in ["start", "starttick", "loopEnabled", "clipId"] {
            let mut payload = serde_json::json!({"startTick": 0});
            payload[unknown] = serde_json::json!(1);
            let fault = parse_placement(&project, &track_id, &clip_id, &placement_args(payload))
                .expect_err("必须拒绝");
            assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
            let value = fault.into_result().expect("带内");
            assert_eq!(value["error"]["data"]["reason"], "unknownPlacementField");
            assert_eq!(value["error"]["data"]["field"], unknown);
            let supported = value["error"]["data"]["supportedPlacementFields"]
                .as_array()
                .expect("必须是数组");
            for expected in PLACEMENT_FIELDS {
                assert!(
                    supported.iter().any(|item| item == expected),
                    "支持集合必须含 {expected}"
                );
            }
        }
    }

    /// 形状错（不是对象 / 键类型不对 / 身份不是 ULID / 负数）都是参数错误，不是静默缺省。
    #[test]
    fn bad_placement_shapes_are_parameter_errors() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        let cases = [
            serde_json::json!("place it"),
            serde_json::json!({"startTick": -1}),
            serde_json::json!({"startTick": 1.5}),
            serde_json::json!({"durationTicks": "960"}),
            serde_json::json!({"muted": "yes"}),
            serde_json::json!({"placementId": "not-a-ulid"}),
            serde_json::json!({"placementId": 42}),
        ];
        for case in cases {
            let fault =
                parse_placement(&project, &track_id, &clip_id, &placement_args(case.clone()))
                    .expect_err("必须拒绝");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{case}"
            );
        }
    }

    /// 两个端点必须真的存在：音轨不存在 ⇒ `TRACK_NOT_FOUND`；片段不存在 ⇒ `CLIP_NOT_FOUND`。
    #[test]
    fn both_endpoints_must_exist() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        let ghost = deterministic_id("ghost");
        let fault = parse_placement(
            &project,
            &ghost,
            &clip_id,
            &placement_args(serde_json::json!({})),
        )
        .expect_err("幽灵音轨");
        assert_eq!(fault.domain_code(), Some(ErrorCode::TrackNotFound));
        let fault = parse_placement(
            &project,
            &track_id,
            &ghost,
            &placement_args(serde_json::json!({})),
        )
        .expect_err("幽灵片段");
        assert_eq!(fault.domain_code(), Some(ErrorCode::ClipNotFound));
    }

    /// 同一条摆放重复提交 ⇒ `CONFLICT`（逐字段相同的重放请走 `idempotencyKey`），
    /// 同一身份不同内容 ⇒ 同样是 `CONFLICT`，但 `reason` 不同。
    #[test]
    fn an_existing_placement_identity_is_a_conflict() {
        let (mut project, track_id, clip_id) = unplaced_midi_clip();
        let first = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"startTick": 0})),
        )
        .expect("解析")
        .expect("在场");
        // 先把这条摆放"做出来"（直接施加到克隆体，模拟上一次调用已经合并）。
        Op::AddClipPlacement {
            track_id,
            placement: first,
        }
        .apply(&mut project)
        .expect("施加");
        // ① 逐字段相同 ⇒ placementAlreadyExists。
        let fault = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"startTick": 0})),
        )
        .expect_err("已存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::Conflict));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["reason"], "placementAlreadyExists");
        assert_eq!(
            value["error"]["data"]["placementId"],
            first.id.to_canonical_string()
        );
        // ② 同身份、不同时值 ⇒ placementIdConflict。
        let fault = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({
                "startTick": 0,
                "placementId": first.id.to_canonical_string(),
                "durationTicks": 480,
            })),
        )
        .expect_err("身份被占用");
        assert_eq!(fault.domain_code(), Some(ErrorCode::Conflict));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["reason"], "placementIdConflict");
    }

    /// 摆放编译成**一条** `Op::AddClipPlacement`，端点与载荷逐字段等于解析结果。
    #[test]
    fn placement_compiles_into_one_add_clip_placement_op() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        let placement = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"startTick": 1920, "muted": true})),
        )
        .expect("解析")
        .expect("在场");
        let ops = vec![Op::AddClipPlacement {
            track_id,
            placement,
        }];
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].name(), "AddClipPlacement");
        // 施加到克隆体 ⇒ 时间轴上真的多出这一条（渲染器读的就是 `track.clips`）。
        let mut after = project.clone();
        batch(&ops).apply(&mut after).expect("整批施加");
        assert_eq!(after.tracks[&track_id].clips[&placement.id], placement);
        assert_eq!(after.clip_pool[&clip_id], project.clip_pool[&clip_id]);
        // 逆操作逐字节回到原位（`Batch` 的逆 = 逆序取逆）。
        let mut back = after;
        batch(&ops).apply_inverse(&mut back).expect("回退");
        assert_eq!(
            serde_json::to_value(&back).expect("序列化"),
            serde_json::to_value(&project).expect("序列化"),
            "撤销必须逐字段复原"
        );
    }

    // -----------------------------------------------------------------------
    // 摆放**编辑**形态（`placement.kind` = `move` / `remove`）
    //
    // 这一组钉住的是"模型层早已实现、工具面此前够不着"的那两个 `Op`：
    // `Op::MoveClipPlacement` 与 `Op::RemoveClipPlacement`（渲染器**真的**按
    // `track.clips` 出片 ⇒ 能不能挪 / 能不能取走是可听的能力）。
    // -----------------------------------------------------------------------

    /// 形态名与**每个形态**的词表被钉住：扩展形态不得发明内容键。
    #[test]
    fn placement_edit_kinds_and_field_sets_are_pinned() {
        assert_eq!(PLACEMENT_KIND_FIELD, "kind");
        assert_eq!(
            PLACEMENT_KINDS,
            [
                PLACEMENT_KIND_ADD,
                PLACEMENT_KIND_MOVE,
                PLACEMENT_KIND_REMOVE
            ]
        );
        assert_eq!(
            PLACEMENT_ADD_FIELDS,
            ["kind", "startTick", "durationTicks", "placementId", "muted"]
        );
        assert_eq!(PLACEMENT_MOVE_FIELDS, ["kind", "startTick", "placementId"]);
        assert_eq!(PLACEMENT_REMOVE_FIELDS, ["kind", "placementId"]);
        // `add` 的**内容键**表没有被这次扩展改动（判据 `placement_field_names_are_pinned`
        // 仍逐字成立）。
        assert_eq!(
            PLACEMENT_FIELDS,
            ["startTick", "durationTicks", "placementId", "muted"]
        );
        for fields in [PLACEMENT_MOVE_FIELDS, PLACEMENT_REMOVE_FIELDS] {
            assert!(
                fields.contains(&PLACEMENT_KIND_FIELD),
                "每个形态都要能写 `kind`"
            );
            for field in fields
                .iter()
                .filter(|field| **field != PLACEMENT_KIND_FIELD)
            {
                assert!(
                    PLACEMENT_FIELDS.contains(field),
                    "扩展形态不得发明新的内容键: {field}"
                );
            }
        }
    }

    /// 一份**已经摆好**一条 MIDI 片段的工程，外加那条摆放本身（`move`/`remove` 的夹具）。
    fn placed_midi_clip() -> (YebanProjectV1, EntityId, EntityId, ClipPlacement) {
        let (mut project, track_id, clip_id) = unplaced_midi_clip();
        let placement = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"startTick": 960, "durationTicks": 480})),
        )
        .expect("解析")
        .expect("在场");
        Op::AddClipPlacement {
            track_id,
            placement,
        }
        .apply(&mut project)
        .expect("施加");
        (project, track_id, clip_id, placement)
    }

    /// `move` 的**旧**起点取自文档、**新**起点取自实参 —— 两者都不是调用方声明的。
    #[test]
    fn move_edit_takes_the_old_tick_from_the_document_and_the_new_tick_from_the_arguments() {
        let (project, track_id, clip_id, existing) = placed_midi_clip();
        let edit = parse_placement_edit(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({
                "kind": PLACEMENT_KIND_MOVE,
                "placementId": existing.id.to_canonical_string(),
                "startTick": 4321,
            })),
        )
        .expect("解析")
        .expect("在场");
        assert_eq!(
            edit,
            PlacementEdit::Move {
                placement_id: existing.id,
                previous_start_tick: 960,
                new_start_tick: 4321,
            }
        );
        assert_eq!(edit.kind_name(), PLACEMENT_KIND_MOVE);
    }

    /// `remove` 的撤销载荷是**文档里那一条摆放本身**（逐字段相等）。
    #[test]
    fn remove_edit_carries_the_document_placement_as_the_undo_payload() {
        let (project, track_id, clip_id, existing) = placed_midi_clip();
        let edit = parse_placement_edit(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({
                "kind": PLACEMENT_KIND_REMOVE,
                "placementId": existing.id.to_canonical_string(),
            })),
        )
        .expect("解析")
        .expect("在场");
        assert_eq!(
            edit,
            PlacementEdit::Remove {
                placement_id: existing.id,
                previous_placement: existing,
            }
        );
        assert_eq!(edit.kind_name(), PLACEMENT_KIND_REMOVE);
    }

    /// 两个新形态都**真的**编译成模型里对应的那一个 `Op`，并且**可逆**
    /// （逆操作只有一份事实源：`Op::invert`）。
    #[test]
    fn move_and_remove_edits_are_reversible_through_the_model_inverse() {
        let (project, track_id, clip_id, existing) = placed_midi_clip();
        let before = serde_json::to_value(&project).expect("序列化");

        // ---- move: 960 → 4321, 施加后真的挪了, 逆操作逐字节回到原位 ----
        let moved = parse_placement_edit(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({
                "kind": PLACEMENT_KIND_MOVE,
                "placementId": existing.id.to_canonical_string(),
                "startTick": 4321,
            })),
        )
        .expect("解析")
        .expect("在场");
        let PlacementEdit::Move {
            placement_id,
            previous_start_tick,
            new_start_tick,
        } = moved
        else {
            panic!("必须是 move 形态");
        };
        let move_op = Op::MoveClipPlacement {
            track_id,
            placement_id,
            old_start_tick: previous_start_tick,
            new_start_tick,
        };
        assert_eq!(move_op.name(), "MoveClipPlacement");
        let mut after_move = project.clone();
        batch(std::slice::from_ref(&move_op))
            .apply(&mut after_move)
            .expect("施加");
        assert_eq!(
            after_move.tracks[&track_id].clips[&existing.id].start_tick,
            4321
        );
        assert_ne!(
            serde_json::to_value(&after_move).expect("序列化"),
            before,
            "move 必须真的改变文档, 否则下面那条回退断言什么也没证明"
        );
        let mut back_from_move = after_move;
        batch(&[move_op])
            .apply_inverse(&mut back_from_move)
            .expect("回退");
        assert_eq!(
            serde_json::to_value(&back_from_move).expect("序列化"),
            before,
            "move 的逆操作必须逐字段复原"
        );

        // ---- remove: 时间轴上真的空了一条, 逆操作逐字节回到原位 ----
        let removed = parse_placement_edit(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({
                "kind": PLACEMENT_KIND_REMOVE,
                "placementId": existing.id.to_canonical_string(),
            })),
        )
        .expect("解析")
        .expect("在场");
        let PlacementEdit::Remove {
            placement_id,
            previous_placement,
        } = removed
        else {
            panic!("必须是 remove 形态");
        };
        let remove_op = Op::RemoveClipPlacement {
            track_id,
            placement_id,
            previous_placement,
        };
        assert_eq!(remove_op.name(), "RemoveClipPlacement");
        let mut after_remove = project.clone();
        batch(std::slice::from_ref(&remove_op))
            .apply(&mut after_remove)
            .expect("施加");
        assert!(
            after_remove.tracks[&track_id].clips.is_empty(),
            "remove 必须真的把这条摆放从时间轴上取走"
        );
        assert!(
            after_remove.clip_pool.contains_key(&clip_id),
            "remove **只**取走摆放, 池子里的材料不动"
        );
        let mut back_from_remove = after_remove;
        batch(&[remove_op])
            .apply_inverse(&mut back_from_remove)
            .expect("回退");
        assert_eq!(
            serde_json::to_value(&back_from_remove).expect("序列化"),
            before,
            "remove 的逆操作必须逐字段复原"
        );
    }

    /// 缺省的 `kind` 与显式 `add` 逐字段等价，且等于 `add` 解析器的结果
    /// （缺省路径逐字节不变）。
    #[test]
    fn absent_or_explicit_add_kind_parses_to_the_same_edit() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        let absent = parse_placement_edit(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"startTick": 480})),
        )
        .expect("解析")
        .expect("在场");
        let explicit = parse_placement_edit(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"kind": PLACEMENT_KIND_ADD, "startTick": 480})),
        )
        .expect("解析")
        .expect("在场");
        assert_eq!(absent, explicit);
        assert_eq!(absent.kind_name(), PLACEMENT_KIND_ADD);
        let direct = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"startTick": 480})),
        )
        .expect("解析")
        .expect("在场");
        assert_eq!(absent, PlacementEdit::Add(direct));
        // 对象不在场 ⇒ `None`（不摆放）。
        assert_eq!(
            parse_placement_edit(&project, &track_id, &clip_id, &Map::new()).expect("缺省"),
            None
        );
    }

    /// 形态判别键的坏形状与未知形态都**响亮拒绝**，并列出支持集合（不静默按 `add` 处理）。
    #[test]
    fn unknown_placement_kinds_are_rejected_with_the_supported_set() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        for payload in [
            serde_json::json!({"kind": "adds"}),
            serde_json::json!({"kind": "delete"}),
            serde_json::json!({"kind": ""}),
        ] {
            let fault =
                parse_placement_edit(&project, &track_id, &clip_id, &placement_args(payload))
                    .expect_err("未知形态必须被拒");
            assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
            let value = fault.into_result().expect("带内");
            assert_eq!(value["error"]["data"]["reason"], "unknownPlacementKind");
            let supported = value["error"]["data"]["supportedPlacementKinds"]
                .as_array()
                .expect("必须是数组");
            for expected in PLACEMENT_KINDS {
                assert!(
                    supported.iter().any(|item| item == expected),
                    "支持集合必须含 {expected}"
                );
            }
        }
        // `kind` 不是字符串 ⇒ 形状错误, 不是"未知形态"。
        let fault = parse_placement_edit(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"kind": 3})),
        )
        .expect_err("`kind` 必须是字符串");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(
            value["error"]["data"]["field"],
            format!("{PLACEMENT_FIELD}.{PLACEMENT_KIND_FIELD}")
        );
    }

    /// `move` / `remove` 的**必填**载荷缺了就响亮拒绝（派生身份只属于 `add`）。
    #[test]
    fn move_and_remove_require_their_own_payload() {
        let (project, track_id, clip_id, existing) = placed_midi_clip();
        let id = existing.id.to_canonical_string();
        let cases = [
            (
                serde_json::json!({"kind": PLACEMENT_KIND_MOVE, "startTick": 10}),
                "moveRequiresPlacementId",
            ),
            (
                serde_json::json!({"kind": PLACEMENT_KIND_MOVE, "placementId": id}),
                "moveRequiresStartTick",
            ),
            (
                serde_json::json!({"kind": PLACEMENT_KIND_REMOVE}),
                "removeRequiresPlacementId",
            ),
        ];
        for (payload, reason) in cases {
            let fault =
                parse_placement_edit(&project, &track_id, &clip_id, &placement_args(payload))
                    .expect_err("缺必填载荷必须被拒");
            assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
            let value = fault.into_result().expect("带内");
            assert_eq!(value["error"]["data"]["reason"], reason);
        }
    }

    /// 已知但**本形态不适用**的键报 `placementFieldNotApplicable`，真拼错的键仍报
    /// `unknownPlacementField` —— 两者不混为一谈，也不静默丢弃。
    #[test]
    fn inapplicable_and_misspelled_placement_fields_are_told_apart() {
        let (project, track_id, clip_id, existing) = placed_midi_clip();
        let id = existing.id.to_canonical_string();
        let inapplicable = [
            serde_json::json!({"kind": PLACEMENT_KIND_MOVE, "placementId": id, "startTick": 1, "muted": true}),
            serde_json::json!({"kind": PLACEMENT_KIND_MOVE, "placementId": id, "startTick": 1, "durationTicks": 480}),
            serde_json::json!({"kind": PLACEMENT_KIND_REMOVE, "placementId": id, "startTick": 1}),
            serde_json::json!({"kind": PLACEMENT_KIND_REMOVE, "placementId": id, "muted": false}),
        ];
        for payload in inapplicable {
            let fault = parse_placement_edit(
                &project,
                &track_id,
                &clip_id,
                &placement_args(payload.clone()),
            )
            .expect_err("本形态不适用的键必须被拒");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{payload}"
            );
            let value = fault.into_result().expect("带内");
            assert_eq!(
                value["error"]["data"]["reason"], "placementFieldNotApplicable",
                "{payload}"
            );
            let supported = value["error"]["data"]["supportedPlacementFields"]
                .as_array()
                .expect("必须是数组");
            assert!(
                !supported
                    .iter()
                    .any(|item| *item == value["error"]["data"]["field"]),
                "报出来的字段不得出现在本形态的支持集合里: {payload}"
            );
        }
        // 真拼错 / 根本不支持的键（`mutedd` / `loopEnabled`）仍报 `unknownPlacementField`。
        for unknown in ["mutedd", "loopEnabled"] {
            let mut payload = serde_json::json!({
                "kind": PLACEMENT_KIND_MOVE,
                "placementId": id,
                "startTick": 1,
            });
            payload[unknown] = serde_json::json!(true);
            let fault =
                parse_placement_edit(&project, &track_id, &clip_id, &placement_args(payload))
                    .expect_err("未知键必须被拒");
            assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
            let value = fault.into_result().expect("带内");
            assert_eq!(value["error"]["data"]["reason"], "unknownPlacementField");
            assert_eq!(value["error"]["data"]["field"], unknown);
        }
    }

    /// 指名的摆放不在目标音轨上 ⇒ `ENTITY_NOT_FOUND`（不是静默新建一条）。
    #[test]
    fn move_and_remove_refuse_a_placement_that_is_not_on_the_track() {
        let (project, track_id, clip_id, _) = placed_midi_clip();
        let ghost = deterministic_id("placement:not-on-this-track").to_canonical_string();
        for payload in [
            serde_json::json!({"kind": PLACEMENT_KIND_MOVE, "placementId": ghost, "startTick": 10}),
            serde_json::json!({"kind": PLACEMENT_KIND_REMOVE, "placementId": ghost}),
        ] {
            let kind = payload["kind"].as_str().expect("kind").to_owned();
            let fault =
                parse_placement_edit(&project, &track_id, &clip_id, &placement_args(payload))
                    .expect_err("幽灵摆放必须被拒");
            assert_eq!(fault.domain_code(), Some(ErrorCode::EntityNotFound));
            let value = fault.into_result().expect("带内");
            assert_eq!(value["error"]["data"]["reason"], "placementNotFound");
            assert_eq!(value["error"]["data"]["placementKind"], kind.as_str());
        }
    }

    /// `clipId` 与文档里那条摆放引用的片段不一致 ⇒ 响亮失败（不静默改用文档那一条）。
    #[test]
    fn move_and_remove_refuse_a_clip_id_that_is_not_the_placed_one() {
        let (project, track_id, clip_id, existing) = placed_midi_clip();
        let other = project
            .clip_pool
            .keys()
            .find(|id| **id != clip_id)
            .copied()
            .expect("样本里必须还有第二条片段池条目");
        let id = existing.id.to_canonical_string();
        for payload in [
            serde_json::json!({"kind": PLACEMENT_KIND_MOVE, "placementId": id, "startTick": 10}),
            serde_json::json!({"kind": PLACEMENT_KIND_REMOVE, "placementId": id}),
        ] {
            let fault = parse_placement_edit(&project, &track_id, &other, &placement_args(payload))
                .expect_err("片段不一致必须被拒");
            assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
            let value = fault.into_result().expect("带内");
            assert_eq!(value["error"]["data"]["reason"], "placementClipMismatch");
            assert_eq!(
                value["error"]["data"]["placementClipId"],
                clip_id.to_canonical_string()
            );
            assert_eq!(
                value["error"]["data"]["clipId"],
                other.to_canonical_string()
            );
        }
    }

    /// `move` 到**原起点** ⇒ `CONFLICT`（没有可提交的改动；不制造一条空提案）。
    #[test]
    fn move_to_the_current_start_tick_is_a_loud_conflict() {
        let (project, track_id, clip_id, existing) = placed_midi_clip();
        let fault = parse_placement_edit(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({
                "kind": PLACEMENT_KIND_MOVE,
                "placementId": existing.id.to_canonical_string(),
                "startTick": existing.start_tick,
            })),
        )
        .expect_err("零位移必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::Conflict));
        let value = fault.into_result().expect("带内");
        assert_eq!(
            value["error"]["data"]["reason"],
            "placementAlreadyAtStartTick"
        );
        assert_eq!(value["error"]["data"]["startTick"], existing.start_tick);
    }
}
