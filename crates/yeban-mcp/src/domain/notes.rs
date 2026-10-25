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
//! **登记边界（绝不静默）**：模型里还有**两个**表现力字段没有工具面通路 ——
//! `syllable` / `phonemes`（没有歌声合成）。工具面会**响亮拒绝**这两个键
//! （`data.unsupportedNoteFields`），不再吞掉。
//!
//! ⚠ ⭐ **`#311b` 之后渲染器不再登记 `noteSlide` / `notePitchBend`**：这两个字段
//! 已经由引擎**真的求值**（滑音末频落在目标音高、弯音曲线按音符局部 64 样本网格
//! 重算 `phase_inc`）⇒ 母带里听得到。仍然为真的只有 `noteLyrics`（没有歌声合成）。
//!
//! ⭐ **`#311c` 把写入通路接上了**：`add.note` 的 [`PITCH_BEND_CURVE_FIELD`] /
//! [`SLIDE_FIELD`] 现在真的进 [`MidiNote`]，`data.supportedNoteFields` 从 8 条变 10 条。
//! 工具面拒绝的面因此**只剩** `syllable` / `phonemes` —— 再说 `slide` / `pitchBendCurve`
//! "没有通路"就是假话（[`reject_unknown_note_fields`] 的 `unsupportedNoteFields` 同口径）。
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
//!   [`StaticLane::NAMES`] 这三个"有静态值"的目标（音轨音量 / 声相 / 设备参数）；
//!   `SendGain`（走 `setRoutingGain`）与 `Macro`（仍没有通路）是**响亮失败**；
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
//! （[`StaticLane`] 把 `SendGain` 与 `Macro` 都排除在外 —— 前者由本形态承载，
//! 后者在工具面仍没有通路，正是为了这件事）。
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
//! ⚠ **就地更正（设备 CRUD 那一票之后）**：上表的 `Op::InsertDevice` / `Op::RemoveDevice`
//! 两行是**那一票之前**的读数，**不改写**（带 ref 的历史读数只标注不改写）。设备 CRUD
//! 落地之后，那两个变体各有了一个真的构造点（[`compile`] 里的两条 match 臂），
//! 因此 ① 列不再是 0；那一票的判据是
//! `device_field_names_are_pinned`／`device_kinds_match_the_model_variant_names`／
//! `insert_device_appends_or_lands_at_the_given_slot_and_is_reversible`／
//! `insert_device_refuses_an_out_of_range_slot_instead_of_clamping`／
//! `insert_device_refuses_a_duplicate_id_and_a_missing_track`／
//! `remove_device_addresses_by_identity_and_reads_the_document`／
//! `device_payload_shapes_fail_loudly`／`device_ops_do_not_need_the_envelope_ids_or_midi`
//! （模块级）与 `edit_notes_device_crud_reaches_the_engine_state_reading_and_undo_restores_it`／
//! `edit_notes_insert_device_refuses_an_out_of_range_slot`（端到端）。
//! 于是 [`OP_KINDS`] 从 17 同步到 19；`Op::RemoveTrack` 那一行**仍然**是 0（上一票之后
//! 它已由另一个形态接上，见该形态自己的小节）。
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
//!
//! ## 音轨实体取走形态（`ops[].kind == "removeTrack"`）
//! —— 关闭"工具面建得出音轨、取不走音轨"这条缺口
//!
//! 与上两节**同型**：模型早就有 [`Op::RemoveTrack`]（载荷 `track_id` + 整条
//! `previous_track` 撤销载荷，`Op::validate` 还拒绝取走主总线），而它在
//! `crates/yeban-mcp` 里的**构造点**实测是 0 —— 与此同时**写侧造得出音轨**：
//! `domain/project_create.rs` 的 `create: true` 分支与 `domain/section_build.rs`
//! 的配器骨架都真的构造 [`Op::AddTrack`]。⇒ 一个 Agent 打开工程后能建音轨、
//! 能改它的音量 / 开关 / 泳道，却**取不走**它；唯一出路是 `yeban_undo`，
//! 而那会连带回退同一提交里的其它一切。读侧一直在报身份
//! （`yeban_query_project` 的 `tracks`）。
//!
//! 本形态关的是"取走**整条音轨实体**"这一半，而且它**自成一层**：它不是"写音轨属性"
//! （那五个音轨属性级形态的目标是**工具顶层**的 `trackId`），因此分类上单列
//! （[`NoteOp::is_track_entity_level`]；`domain::plan_edit_notes` 的提案标题靠它，
//! 否则一次"取走音轨"会被报成"音轨级编辑"）。
//!
//! 形态：`{"kind":"removeTrack","trackId":"<ULID>"}` —— 载荷是**空**的（只有寻址）；
//! 撤销载荷 `previous_track` 由 [`compile`] 从**当前文档**读（模型的前置条件要求它
//! 逐字段等于文档现值，因此本层不采信调用方声明的旧状态）。
//!
//! 三条刻意设成**响亮失败**的口径（绝不静默降级）：
//!
//! | 情形 | 结果 |
//! | :--- | :--- |
//! | 操作对象里有 `kind` / `trackId` 之外的键 | `INVALID_PARAMETER_RANGE`（`reason = "unknownRemoveTrackField"`） |
//! | `trackId` 缺失 / 不是字符串 / 不是合法 ULID | `INVALID_PARAMETER_RANGE`（缺字段走统一的缺字段错误） |
//! | `project.tracks` 里没有这个身份 | `ENTITY_NOT_FOUND`（`reason = "trackNotFound"`） |
//! | 这个身份是**主总线** | `CONFLICT`（`reason = "masterBusTrackCannotBeRemoved"`） |
//! | 与别的 `kind` 同给 | `INVALID_PARAMETER_RANGE`（`reason = "removeTrackTakesNoOtherOps"`） |
//! | 与 `placement` 同给 | `INVALID_PARAMETER_RANGE`（`reason = "removeTrackIsNotPlacement"`） |
//!
//! ⚠ **本形态的寻址不用工具顶层的 `trackId`**：那个实参的语义是"目标片段所在的音轨"
//! （[`compile`] 的入口拿它查音轨与片段池），而本形态的寻址在操作对象**自带**的
//! `trackId` 上 —— 与三个路由级形态、段落形态、场景形态同一纪律。顶层 `trackId` /
//! `clipId` 在本形态单独出现时因此**不被读取**（可以给任意合法 ULID）：`removeTrack`
//! 一个音符都不读，也**不要求**片段是 MIDI。
//!
//! 不碰的东西（逐条给理由，不冒充已完成）：**片段池不回收**（池里的材料可能仍被别的
//! 音轨引用；模型 `Op` 全集里也没有资产 / 池级回收变体）；**路由图不清理**（图只存
//! 身份、不存音轨对象，取走一条仍在 `routing_graph.nodes` 里的音轨不会让图失效 ——
//! 想清节点请走 `disconnectRouting` + `removeRoutingNode` 两步）；**不查别的音轨的
//! `folder_id`**（`YebanProjectV1::validate` 自己会报悬空引用，而且工具面没有任何形态
//! 能设置 `folder_id`）。

//! ## 设备链形态（`ops[].kind == "insertDevice"` / `"removeDevice"`）
//! —— 关闭"工具面看得见设备链、一台都动不了"这条缺口
//!
//! 与前面几节**同型**，但缺口的后果更重：`Op::InsertDevice` / `Op::RemoveDevice` 在
//! `crates/yeban-mcp/src` 里的**构造点**实测都是 **0**（两口径普查见上一节的表），
//! 而读侧一直在报设备链（`yeban_query_engine_state` 的 `tracks[].devices[]`，逐槽带
//! `slotIndex` / `deviceId` / `latencySamples` / `params`）。`DeviceDefinition::
//! latency_samples` 是 PDC 的**唯一**延迟来源（`ARCH-PDC-001`，见 `domain/render.rs`
//! 的 `track_latencies`）⇒ 一台延迟非零的外部设备此前只能**存在于文档里**，
//! 在 MCP 会话里无法被表达。
//!
//! 两个形态都**自带寻址**（操作对象自己的 `trackId`），因此与 [`NoteOp::RemoveTrack`]
//! 同一条纪律：[`NoteOp::is_self_addressed`] 为真时 [`compile`] 与
//! `domain::plan_edit_notes` 不读工具顶层的 `trackId` / `clipId`，也不要求片段是 MIDI。
//! 两条设备形态**允许**同批（换一台设备本来就是两条操作），取走按**身份**（`deviceId`）
//! 而不是按下标（下标会随同批的其它插入 / 取走漂移，身份不会）。
//!
//! `device` 载荷的字段名取**工具面**的 camelCase（[`DEVICE_FIELDS`]），而 `.yeban`
//! 容器里的 `project.json` 用模型 serde 的 snake_case（`latency_samples`）——
//! 这是一处**有自由度**的裁决，理由写在 [`parse_device`] 的文档上，不静默。
//! `device.kind` 的取值则逐字等于模型 serde 的变体名（[`DEVICE_KINDS`]）。
//!
//! 六条刻意设成**响亮失败**的口径（绝不静默降级）：
//!
//! | 情形 | 结果 |
//! | :--- | :--- |
//! | 操作对象里有支持集合之外的键 | `INVALID_PARAMETER_RANGE`（`unknownInsertDeviceField` / `unknownRemoveDeviceField`） |
//! | `device` 里有支持集合之外的键（含 snake_case 拼写） | `INVALID_PARAMETER_RANGE`（`unknownDeviceField`） |
//! | `device.params[]` 里有支持集合之外的键 | `INVALID_PARAMETER_RANGE`（`unknownDeviceParamField`） |
//! | ⭐ `device.params[].name` 与某个已识别拼写只差 ≤ 2 个字符（拼错） | `INVALID_PARAMETER_RANGE`（`unknownEffectParam`，带最近邻建议 — #175） |
//! | `device.kind` 不是 [`DEVICE_KINDS`] 里的名字 | `INVALID_PARAMETER_RANGE`（`unknownDeviceKind`） |
//! | `device.latencySamples` 超出 `u32` | `OUT_OF_RANGE`（`latencyOutOfRange`） |
//! | 参数值在收窄到 `f32` **之后**非有限 | `INVALID_PARAMETER_RANGE`（`nonFiniteValue`） |
//! | `device.name` 是空串 | `INVALID_PARAMETER_RANGE`（`deviceNameRequired`） |
//! | `slotIndex` 越界（`> devices.len()`） | `OUT_OF_RANGE`（`deviceSlotOutOfRange`，不夹紧） |
//! | 设备身份在同一条音轨上已存在 | `CONFLICT`（`deviceIdAlreadyInTrack`） |
//! | 目标音轨不存在 | `ENTITY_NOT_FOUND`（`deviceTrackNotFound`） |
//! | `removeDevice` 的身份不在链上 | `ENTITY_NOT_FOUND`（`deviceNotFound`） |
//!
//! 不碰的东西（逐条给理由，不冒充已完成）：**本形态只增删整台设备**，不夹带参数修改 ——
//! 改现有设备的静态参数走 `setParam` 的 `DeviceParam` 目标（那个目标会先过
//! `yeban_render::inserts::resolve_device_param` 的白名单）；**不校验设备链的 DSP 语义**
//! （`yeban-render/src/inserts.rs` 对通道条 / 混响 / 卷积混响**真的**求值，但压缩与
//! 未知旋钮仍不执行 —— 那是 `domain/render.rs` 的 `deviceChainDsp` 登记面）；**不允许插入重复身份**（模型 `Op::validate` 的
//! `DuplicateEntityId`，本层提前报 `deviceIdAlreadyInTrack`）。

use std::collections::BTreeMap;
use std::str::FromStr as _;

use serde_json::{Map, Value};

use yeban_model::music::{
    MICRO_TIMING_MAX_ABS, MIDI_PITCH_MAX, MIDI_VELOCITY_MAX, PITCH_BEND_CENTS_MAX_ABS,
    PROBABILITY_MAX, PROBABILITY_MIN, RATCHET_MAX, RATCHET_MIN,
};
use yeban_model::{
    AssetHash, AutomationLane, AutomationTarget, AutomationValueDomain, AutomationWriteMode,
    ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition, DeviceKind, EntityId,
    LaunchQuantization, LoopConfig, MidiNote, Op, ParameterValue, RoutingEdge, RoutingKind,
    SamplerBinding, SceneV3, SectionV3, SlideConfig, TrackKind, TrackV3, TransportConfig,
    YebanProjectV1,
};
// 设备参数的**序数 → 稳定身份**解析与**名字白名单**只有一份：`#263` 落在 render 里。
// ⛔ 本层不复制第二份（否则 `paramNotSupported` 会有两个字面量来源）。
use yeban_render::inserts::{UnresolvedReason, resolve_device_param};

use super::error::{Fault, from_model};
use super::extension_pure::LaneKind;
// ⚠ `setAsset` 的许可判定**复用** `import_audio` 的那一份（⛔ 不复制第二套）。
use super::ids::deterministic_id;
use super::import_audio::require_allowed_licence;
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

/// `add` 音符对象里的**弯音曲线**字段名（`ops[].note.pitchBendCurve`，可选）。
///
/// 形状是 `[tick, cents]` 二元组的数组；坐标与值域的**唯一**权威是
/// `docs/adr/ADR-0012-pitch-bend-curve-contract.md`：
///
/// - `tick` 是**音符局部**坐标（D1），首点可以是任意 `tick >= 0`（D4：首点之前
///   `cents` 取 `0`，因此 `[[240, 200]]` 合法）；
/// - `cents` 相对 `note.pitch`，闭区间
///   `-PITCH_BEND_CENTS_MAX_ABS..=PITCH_BEND_CENTS_MAX_ABS`（D2，±1200）；
/// - tick **严格**升序（D3，重复 tick 即错）；点落在鸣响区间之外**允许**（D6，
///   那些点永不采样）；
/// - 检查顺序**照抄**模型（升序先于 `cents`，D7）⇒ 同一份输入在两层报同一个错误。
///
/// 取值的权威判定仍在模型层（[`MidiNote::validate`] → `validate_pitch_bend_curve`）；
/// 本层只额外把 `field` / `index` 放进拒绝载荷里。
///
/// ⚠ `#311c` 之前这个键会被 [`reject_unknown_note_fields`] **响亮拒绝**（工具面
/// 没有写入通路）；现在它是 [`NOTE_FIELDS`] 的一员，母带渲染器与引擎**真的**求值它
/// （`#311b`：音符局部 64 样本网格重算 `phase_inc`）。
pub const PITCH_BEND_CURVE_FIELD: &str = "pitchBendCurve";

/// `add` 音符对象里的**滑音**字段名（`ops[].note.slide`，可选）。
///
/// 形状：`{ "durationTicks": <非零 tick>, "targetPitch": <0..=127>,
/// "curve": <四个规范名之一，可选，缺省 Linear> }`。
///
/// 曲线词表**只有一份**：`crate::domain::automation::parse_curve`（名字表
/// `extension_pure::CURVE_NAMES`，与模型 `CurveType` 的双射由判据
/// `every_name_table_agrees_with_the_model_enum` 钉住）—— 本层⛔ 不复制第二套拼写。
///
/// 语义与求值在模型/引擎：`slide` 与 `pitch_bend_curve` 同时存在时**相加**（`ADR-0012`
/// D9），曲线的数学只有 `CurveType::ease` 一份实现。
pub const SLIDE_FIELD: &str = "slide";

/// `slide` 对象里的**时值**字段名（可选字段 `slide` 的必填子字段，单位 tick）。
pub const SLIDE_DURATION_FIELD: &str = "durationTicks";

/// `slide` 对象里的**目标音高**字段名（必填，MIDI `0..=127`）。
pub const SLIDE_TARGET_PITCH_FIELD: &str = "targetPitch";

/// `slide` 对象里的**曲线形状**字段名（可选，缺省 `Linear`）。
pub const SLIDE_CURVE_FIELD: &str = "curve";

/// `slide` 对象**允许**出现的全部键。
///
/// 与 `note` 顶层同一口径：集合之外的键**响亮拒绝**并列出这一份支持集合，
/// 绝不静默丢弃。
pub const SLIDE_FIELDS: [&str; 3] = [
    SLIDE_DURATION_FIELD,
    SLIDE_TARGET_PITCH_FIELD,
    SLIDE_CURVE_FIELD,
];

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
    PITCH_BEND_CURVE_FIELD,
    SLIDE_FIELD,
];

/// `ops[].kind` 的**音符时间位置**形态名（写 [`Op::SetNoteTiming`]）。
///
/// ⚠ 与模型 `Op` 变体名同词（`SetNoteTiming` 的小驼峰），与四个既有音符形态
/// （`add` / `delete` / `move` / `velocity`）同一条命名规则。
///
/// ⚠ **本形态补上了什么**：模型里 `start_tick` / `duration_ticks` 此前**只在 `add`
/// 时**给定（`move` 只平移起点、⛔ 改不了时值）⇒ 改时值只能"删 + 加"，而那要求
/// 调用方把概率 / 连击 / 微时序 / 滑音 / 弯音曲线等表现力字段全部抄一遍
/// （漏抄一个就是**静默的表现力丢失**），且操作日志里留下**两条** op。
pub const SET_NOTE_TIMING_KIND: &str = "setNoteTiming";

/// 音符时间位置形态的**目标音符**字段名（`ops[].noteId`，必填，ULID）。
pub const SET_NOTE_TIMING_NOTE_FIELD: &str = "noteId";

/// 音符时间位置形态的**起点**字段名（`ops[].startTick`，必填，`u64` tick）。
pub const SET_NOTE_TIMING_START_FIELD: &str = "startTick";

/// 音符时间位置形态的**时值**字段名（`ops[].durationTicks`，必填，正整数 tick）。
///
/// ⚠ `0` 由模型 `MidiNote::validate` 的 `ZeroDuration` 响亮拒绝（契约码 `OUT_OF_RANGE`）
/// —— 本层⛔ 不自造第二条口径（与 [`SET_CLIP_PLACEMENT_DURATION_DURATION_FIELD`] 同一纪律）。
pub const SET_NOTE_TIMING_DURATION_FIELD: &str = "durationTicks";

/// 音符时间位置形态允许出现的**全部**键（判别键 + 三个必填字段）。
///
/// ⚠ 两个旧值**不在**这里：它们由 [`compile`] 从**当前文档**读（⛔ 不信调用方声明），
/// 与 `delete` / `velocity` 的撤销载荷同一条纪律。多写一个键是**响亮失败**。
pub const SET_NOTE_TIMING_FIELDS: [&str; 4] = [
    "kind",
    SET_NOTE_TIMING_NOTE_FIELD,
    SET_NOTE_TIMING_START_FIELD,
    SET_NOTE_TIMING_DURATION_FIELD,
];

/// `ops[].kind` 的**音符表现力**形态名（写 [`Op::SetNoteExpression`]）。
///
/// ⚠ 与模型 `Op` 变体名同词（`SetNoteExpression` 的小驼峰）。
///
/// ⚠ **本形态补上了什么**：`probability` / `ratchet` / `micro_timing_ticks` 此前**只在
/// `add` 时**由 `note` 对象给定，之后**没有任何通路** ⇒ ⭐ **音符建得出、表情改不了**：
/// 改一个概率只能删 + 加，而那会**换掉音符身份** —— `MidiNote::triggers` 的确定性哈希
/// 只用 `id` ⇒ 触发结果会跟着变。三个字段都有**行为重量**（`yeban_render_master` 真的
/// 消费它们：概率触发、连击细分、起点偏移）。
///
/// ⚠ **整体替换而不是逐字段**（照 [`SET_TRANSPORT_CONFIG_KIND`]）：三个键**全部必填**，
/// `null` = 清空 = 模型的 `None`（必然触发／等价于 1／无偏移）—— ⛔ 缺键**不等于** `null`
/// （缺键是"没陈述这个字段"，而本形态没有"保留现值"的合并语义）。
pub const SET_NOTE_EXPRESSION_KIND: &str = "setNoteExpression";

/// 音符表现力形态的**目标音符**字段名（`ops[].noteId`，必填，ULID）。
///
/// 与 [`SET_NOTE_TIMING_NOTE_FIELD`] **同一个字面量**（同一个词 = 同一个意思）。
pub const SET_NOTE_EXPRESSION_NOTE_FIELD: &str = SET_NOTE_TIMING_NOTE_FIELD;

/// 音符表现力形态的**概率**字段名（`ops[].probability`，必填，`0.0..=1.0` 的数字或 `null`）。
///
/// ⚠ 与 `add.note.probability` **同一个字面量**；值域照抄 `MidiNote::validate`
/// （有限 ∧ `0.0..=1.0`），越界报既有的 `ProbabilityOutOfRange` ⇒ 契约码 `OUT_OF_RANGE`
/// （⛔ 本层不另造区间）。
pub const SET_NOTE_EXPRESSION_PROBABILITY_FIELD: &str = PROBABILITY_FIELD;

/// 音符表现力形态的**连击**字段名（`ops[].ratchet`，必填，`1..=16` 的整数或 `null`）。
pub const SET_NOTE_EXPRESSION_RATCHET_FIELD: &str = RATCHET_FIELD;

/// 音符表现力形态的**微时序**字段名（`ops[].microTimingTicks`，必填，
/// `-240..=240` 的整数或 `null`）。
pub const SET_NOTE_EXPRESSION_MICRO_TIMING_FIELD: &str = MICRO_TIMING_FIELD;

/// 音符表现力形态允许出现的**全部**键（判别键 + 一个身份 + 三个必填值）。
///
/// ⚠ 三个旧值**不在**这里：它们由 [`compile`] 从**当前文档**读（⛔ 不信调用方声明），
/// 与 `delete` / `velocity` / `setNoteTiming` 的撤销载荷同一条纪律。
pub const SET_NOTE_EXPRESSION_FIELDS: [&str; 5] = [
    "kind",
    SET_NOTE_EXPRESSION_NOTE_FIELD,
    SET_NOTE_EXPRESSION_PROBABILITY_FIELD,
    SET_NOTE_EXPRESSION_RATCHET_FIELD,
    SET_NOTE_EXPRESSION_MICRO_TIMING_FIELD,
];

/// `ops[].kind` 的**音符音高形状**形态名（写 [`Op::SetNotePitchShape`]）。
///
/// ⚠ 与模型 `Op` 变体名同词（`SetNotePitchShape` 的小驼峰）。
///
/// ⚠ **本形态补上了什么**：`pitchBendCurve` / `slide` 此前**只在 `add` 的 `note`
/// 对象里**能给一次（`#311c`），之后**没有任何通路** ⇒ ⭐ **音符建得出、音高形状改不了**：
/// 改一条弯音曲线只能删 + 加，而那会**换掉音符身份** —— `MidiNote::triggers` 的
/// 确定性哈希只用 `id` ⇒ 触发结果会跟着变。两个字段都有**听觉重量**
/// （`yeban_render_master` 真的求值它们，且同时存在时**相加**，`ADR-0012` D9）。
///
/// ⚠ **整体替换而不是逐字段**：弯音与滑音是**一个作者选择**（照
/// [`SET_TRANSPORT_CONFIG_KIND`] 的同一条裁决）⇒ 两个新值一起移动，一条 `Op` 一步撤销。
///
/// ⚠ **载荷只带新值**（与 [`SET_NOTE_EXPRESSION_KIND`] 同一条口径）：两个旧值由
/// [`compile`] 从**当前文档**读后填进模型 `Op`，⛔ 不采信调用方声明。把
/// `oldPitchBendCurve` / `oldSlide` 写进载荷是**未知键**，响亮拒绝 —— 那正是
/// `oldProbability` 对 `setNoteExpression` 的待遇（同一个教训：最像"写对了"的错法
/// 就是把旧值也写进来，而它会被静默忽略）。
pub const SET_NOTE_PITCH_SHAPE_KIND: &str = "setNotePitchShape";

/// 音符音高形状形态的**目标音符**字段名（`ops[].noteId`，必填，ULID）。
///
/// 与 [`SET_NOTE_EXPRESSION_NOTE_FIELD`] **同一个字面量**。
pub const SET_NOTE_PITCH_SHAPE_NOTE_FIELD: &str = SET_NOTE_EXPRESSION_NOTE_FIELD;

/// 音符音高形状形态的**新弯音曲线**字段名（必填，`[[tick, cents], ...]`）。
///
/// ⚠ 名字刻意保留 `add` 路径的 `pitchBendCurve` 词面（同一个概念 = 同一个词），
/// 只加 `new*` 前缀。形状与值域由**同一个**读取器把关
/// （[`read_pitch_bend_curve_value`]），⛔ 不复制第二套。
///
/// ⚠ "无弯音"**只有一种拼法 —— 空数组 `[]`**；`null` 这里是**形状错误**
/// （模型的 `pitch_bend_curve` 是 `Vec`，⛔ 不是 `Option`）。
pub const SET_NOTE_PITCH_SHAPE_NEW_BEND_FIELD: &str = "newPitchBendCurve";

/// 音符音高形状形态的**新滑音**字段名（必填，对象或 `null` = 清空）。
pub const SET_NOTE_PITCH_SHAPE_NEW_SLIDE_FIELD: &str = "newSlide";

/// 音符音高形状形态允许出现的**全部**键（判别键 + 一个身份 + 两个必填新值）。
///
/// ⚠ 两个值**全部必填**（照 `setNoteExpression` 的三个新值）：`null` 只对 `slide`
/// 有意义（`null` = 清空 = 模型的 `None`）；弯音用**空数组**表示无弯音。
/// ⚠ **旧值不在载荷里**：它们由 [`compile`] 从**当前文档**读（与 `setNoteExpression`
/// 的三个旧值同一条纪律）。把 `oldPitchBendCurve` / `oldSlide` 写进来是**未知键**，
/// 走 [`reject_set_note_pitch_shape_fields`]（reuse 同一条拒绝路径），响亮拒绝并列出
/// 本集合 —— ⛔ 不静默忽略。
pub const SET_NOTE_PITCH_SHAPE_FIELDS: [&str; 4] = [
    "kind",
    SET_NOTE_PITCH_SHAPE_NOTE_FIELD,
    SET_NOTE_PITCH_SHAPE_NEW_BEND_FIELD,
    SET_NOTE_PITCH_SHAPE_NEW_SLIDE_FIELD,
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
pub const PLACEMENT_FIELDS: &[&str] = &[
    "startTick",
    "durationTicks",
    "placementId",
    "muted",
    LOOP_ENABLED_FIELD,
    LOOP_START_TICK_FIELD,
    LOOP_END_TICK_FIELD,
];

/// ⭐ `#241`：`add` 摆放的循环开关（`placement.loopEnabled`，可选；缺省 `false`）。
///
/// 语义与展开规则**只有一份**（`yeban-model` 的 `ClipPlacement::loop_repeat` / `note_copies`，[ADR-0010 D5]）：
/// 启用且 `loopStartTick < loopEndTick < durationTicks` 时，区间按
/// `loopEndTick - loopStartTick` 为周期重复、铺满摆放（`N = ceil((duration - start) / L)`）。
///
/// 为什么这个旋钮值得工具面：渲染器**已经**能展开循环（`#241`），但在这个字段
/// 之前**没有任何工具**能写 `ClipPlacement::loop_config` —— 一个 4 小节 loop
/// 只能让 agent 把音符手写 N 遍。交付形态：只搬字面值进 `LoopConfig`，
/// "怎么展开"不在这层另立第二份规则（与 `ratchet`/`probability` 同一纪律）。
pub const LOOP_ENABLED_FIELD: &str = "loopEnabled";

/// ⭐ `#241`：循环区间起点（`placement.loopStartTick`，可选；缺省 0，摆放相对 tick）。
///
/// 给出 `loopStartTick`/`loopEndTick` 时必须同时 `loopEnabled: true`：
/// 关闭循环却给区间是**自相矛盾**的载荷，响亮拒绝（不静默丢弃）。
pub const LOOP_START_TICK_FIELD: &str = "loopStartTick";

/// ⭐ `#241`：循环区间终点（`placement.loopEndTick`，可选；启用时缺省 = 片段内容长度，
/// 推不出内容长度时 = `durationTicks`）。
pub const LOOP_END_TICK_FIELD: &str = "loopEndTick";

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
    LOOP_ENABLED_FIELD,
    LOOP_START_TICK_FIELD,
    LOOP_END_TICK_FIELD,
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

/// `setParam` 写**设备参数**时的**槽位下标**字段名（`ops[].slotIndex`，`lane == "DeviceParam"` 时必填）。
///
/// 与 `yeban_edit_automation` / `insertDevice` 的同一字段逐字同词：它们是同一份
/// `track.devices` 的同一个序数（`ADR-0001` D48）。
pub const SET_PARAM_SLOT_FIELD: &str = "slotIndex";

/// `setParam` 写**设备参数**时的**参数下标**字段名（`ops[].paramIndex`，`lane == "DeviceParam"` 时必填）。
///
/// 与 `yeban_edit_automation` 的同一字段逐字同词：它们是同一份 `device.params` 的同一个序数。
pub const SET_PARAM_PARAM_FIELD: &str = "paramIndex";

/// `ops[].kind` 的**音轨静音**形态名（写 [`Op::SetTrackMute`]）。
///
/// 与模型 `Op` 变体名同词（`SetTrackMute` 的小驼峰），与 [`SET_PARAM_KIND`] 同一条规则。
pub const SET_TRACK_MUTE_KIND: &str = "setTrackMute";

/// `ops[].kind` 的**音轨独奏**形态名（写 [`Op::SetTrackSolo`]）。
pub const SET_TRACK_SOLO_KIND: &str = "setTrackSolo";

/// `ops[].kind` 的**走带配置**形态名（写 [`Op::SetTransportConfig`]）。
///
/// 与模型 `Op` 变体名同词（`SetTransportConfig` 的小驼峰），与 [`SET_TRACK_MUTE_KIND`]
/// 同一条规则。
pub const SET_TRANSPORT_CONFIG_KIND: &str = "setTransportConfig";

/// `setTransportConfig` 的**节拍器开关**字段名（`ops[].metronomeEnabled`，必填，布尔）。
pub const TRANSPORT_METRONOME_FIELD: &str = "metronomeEnabled";

/// `setTransportConfig` 的**预备小节数**字段名（`ops[].countInBars`，必填，整数 `0..=255`）。
pub const TRANSPORT_COUNT_IN_FIELD: &str = "countInBars";

/// `setTransportConfig` 的**启动量化**字段名（`ops[].launchQuantization`，必填，枚举串）。
///
/// ⚠ 取值与线协议**逐字相同**（`Off` / `Bar` / `TwoBars` / `FourBars` / `EightBars`）
/// —— 那是 `schemas/project.schema.json` 的 `transport.launch_quantization.enum`
/// 与模型 `LaunchQuantization` 的 `serde` 名，因此本层**不另发明**一套拼写。
pub const TRANSPORT_LAUNCH_FIELD: &str = "launchQuantization";

/// 走带配置形态允许出现的**全部**键（判别键 `kind` ＋ 三个字段，字段全部必填）。
///
/// ⚠ 与音轨开关形态同一条口径（`ADR-0001 D43` 的"必须被显式陈述"）：三个字段
/// **一个都不能省**，多写一个键是**响亮失败**（⛔ 不静默丢弃、⛔ 不用 `Default` 补）。
pub const TRANSPORT_CONFIG_FIELDS: [&str; 4] = [
    "kind",
    TRANSPORT_METRONOME_FIELD,
    TRANSPORT_COUNT_IN_FIELD,
    TRANSPORT_LAUNCH_FIELD,
];

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

/// `ops[].kind` 的**全集**（规范顺序：四个音符 / 池级 / 音轨级 / 设备级 / 摆放形态在前，
/// 音轨属性级、路由级、段落级与场景级形态在后）。
///
/// 错误信息（[`parse_one`] 的未知 `kind`）与判据共用这一份真相。
/// ⚠ 两个"音轨"词面的键分属两个不同的桶：`REMOVE_TRACK_KIND`（`removeTrack`）
/// 取走**整条音轨**，`SET_PARAM_KIND` / `SET_TRACK_MUTE_KIND` / `SET_TRACK_SOLO_KIND`
/// 写一条音轨的**属性** —— 它们不是一个层级（见 [`NoteOp::is_track_entity_level`]）。
/// ⚠ 两个设备键同理自成一层（见 [`NoteOp::is_device_level`]）。
/// ⚠ ⭐ **类型是切片、⛔ 不是定长数组** —— 原因**实测**：本常量会被 `json!` 序列化
/// （`supportedKinds`），⭐ 而 `serde` 对 `[T; N]` 的 impl **只到 N ≤ 32** ⇒
/// ⭐ **加到第 33 个 kind 时会以 `E0277: [&str; 33]: Serialize` 报错**。
/// ⭐ 故这里用 `&[&str]`（`&[T]` 的 impl 没有长度上限）⇒ 加 kind 时⛔ 不会撞墙。
pub const OP_KINDS: &[&str] = &[
    "add",
    "delete",
    "move",
    "velocity",
    SET_NOTE_TIMING_KIND,
    SET_NOTE_EXPRESSION_KIND,
    SET_NOTE_PITCH_SHAPE_KIND,
    REMOVE_CLIP_KIND,
    REMOVE_ASSET_KIND,
    SET_ASSET_KIND,
    REMOVE_TRACK_KIND,
    INSERT_DEVICE_KIND,
    REMOVE_DEVICE_KIND,
    SET_PARAM_KIND,
    SET_TRACK_MUTE_KIND,
    SET_TRACK_SOLO_KIND,
    SET_TRANSPORT_CONFIG_KIND,
    SET_TEMPO_KIND,
    SET_AUTHOR_KIND,
    SET_TITLE_KIND,
    SET_TRACK_NAME_KIND,
    SET_CLIP_NAME_KIND,
    SET_CLIP_GAIN_KIND,
    SET_DEVICE_NAME_KIND,
    SET_DEVICE_BYPASSED_KIND,
    SET_CLIP_PLACEMENT_MUTED_KIND,
    SET_CLIP_PLACEMENT_LOOP_KIND,
    SET_TRACK_COLOR_KIND,
    SET_TRACK_SAMPLER_KIND,
    SET_CLIP_PLACEMENT_DURATION_KIND,
    SET_AUTOMATION_LANE_KIND,
    REMOVE_AUTOMATION_POINT_KIND,
    SET_ROUTING_GAIN_KIND,
    DISCONNECT_ROUTING_KIND,
    CONNECT_ROUTING_KIND,
    REMOVE_ROUTING_NODE_KIND,
    ADD_ROUTING_NODE_KIND,
    ADD_TRACK_KIND,
    REMOVE_SECTION_KIND,
    SET_SECTION_KIND,
    REMOVE_SCENE_KIND,
    SET_SCENE_KIND,
];

/// **读**工具顶层 `clipId` 的 kind 清单（`#177` 坑④：`clipId` 从无条件必填改成条件必需）。
///
/// 这就是 [`NoteOp::reads_envelope_clip`] 的**发布面**：`crate::op_schema::ops_items`
/// 用它给每个 `oneOf` 分支打 `x-envelope.clipId`（并写进分支描述），
/// `domain::plan_edit_notes` 用它决定"缺参时要不要响亮拒绝"。
///
/// ⚠ 两份真相（本清单 vs 谓词的 `match`）由判据
/// `every_catalog_kind_parses_and_round_trips` 在**全部 42 个 kind** 上对账
/// （`OP_KINDS` 里每个 kind 的最小合法形状 → 解析 → 与本清单比对），
/// 因此"加了一个 kind 却忘了决定它读不读 `clipId`"会立刻变红。
pub const ENVELOPE_CLIP_KINDS: &[&str] = &[
    "add",
    "delete",
    "move",
    "velocity",
    SET_NOTE_TIMING_KIND,
    SET_NOTE_EXPRESSION_KIND,
    SET_NOTE_PITCH_SHAPE_KIND,
    REMOVE_CLIP_KIND,
];

/// **读/写音符**的 kind 清单 —— [`NoteOp::is_note_level`] 的**唯一发布面**。
///
/// ⭐ `#179`：本集合收窄到音符号形态之后，现在是**七个**（`add` / `delete` /
/// `move` / `velocity` / `setNoteTiming` / `setNoteExpression` /
/// `setNotePitchShape`），也就是
/// [`ENVELOPE_CLIP_KINDS`] 里除池级 `removeClip` 之外的七个。其余 33 个形态
/// （音轨属性级 / 设备级 / 摆放属性级 / 路由级 / 段落级 / 场景级 / 资产级 /
/// 文档级 / 音轨实体级 / 池级）**一个音符都不读**，因此不要求片段是 MIDI。
///
/// ⚠ 与 [`ENVELOPE_CLIP_KINDS`] **不是**同一个清单：`removeClip` 读顶层 `clipId`
/// 却**不**读音符（它取走整条池条目，音频条目同样取得走）；反过来，本清单里的
/// 七个形态必然是顶层 `clipId` 的消费者 —— 因此本清单是
/// [`ENVELOPE_CLIP_KINDS`] 去掉 `removeClip` 之后的子集，且这条包含关系由判据钉住。
///
/// 两份真相（本清单 vs 谓词的 `match`）由判据
/// `every_catalog_kind_compiles_to_exactly_one_op_variant` 在**全部 42 个 kind** 上
/// 对账（`OP_KINDS` 里每个 kind 的规范形状 → 解析 → 与本清单比对），因此"加了一个
/// kind 却忘了决定它读不读音符"会立刻变红。
pub const NOTE_LEVEL_KINDS: &[&str] = &[
    "add",
    "delete",
    "move",
    "velocity",
    SET_NOTE_TIMING_KIND,
    SET_NOTE_EXPRESSION_KIND,
    SET_NOTE_PITCH_SHAPE_KIND,
];

/// `setParam` 能写的**静态目标**（[`Op::SetParam`] 里"有静态值可写"的那三个）。
///
/// 为什么单列一个枚举而不是直接收 [`LaneKind`]：`SendGain` 的模型语义是
/// `Option<f32>`（`None` = 单位增益），`SetParam` **明文拒绝**它（必须走
/// `Op::SetRoutingGain`）；`Macro` 的静态写入在本工具面**没有**通路。用类型把
/// "哪两个不可写"变成**不可表达**，比在 `compile` 里补一条不可达分支更诚实。
///
/// `DeviceParam` 是**设备级**目标：寻址是两个下标（`slotIndex` / `paramIndex`），
/// 因此变体自带这两个字段；参数名是否被离线渲染器消费由
/// `yeban_render::inserts::resolve_device_param` 回答（⛔ 本层不复制白名单）。
///
/// `SendGain` 的静态增益由 [`SET_ROUTING_GAIN_KIND`] 承载（那是**另一个**模型变体，
/// 因此仍然**不**属于本枚举 —— 把它并进来就是让本枚举的名字说谎）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticLane {
    /// 音轨静态音量（`TrackV3::volume_db`，单位 dB，有限值）。
    TrackVolume,
    /// 音轨静态声相（`TrackV3::pan`，-1.0..=1.0）。
    TrackPan,
    /// 设备静态参数（`track.devices[slotIndex].params[paramIndex].value`）。
    DeviceParam {
        /// `track.devices` 里的槽位下标。
        slot_index: usize,
        /// 该设备 `params` 里的参数下标。
        param_index: usize,
    },
}

impl StaticLane {
    /// 允许的目标名，**规范顺序**（错误信息的 `allowed` 与判据共用）。
    ///
    /// 从 [`LaneKind`] 自己的 `as_str` 派生 ⇒ 词汇表**只有一份**（不是第二张手写表）。
    pub const NAMES: [&'static str; 3] = [
        LaneKind::TrackVolume.as_str(),
        LaneKind::TrackPan.as_str(),
        LaneKind::DeviceParam.as_str(),
    ];

    /// 把目标落到具体音轨上（[`Op::SetParam`] 的载荷）。
    #[must_use]
    pub fn target(self, track_id: EntityId) -> AutomationTarget {
        match self {
            Self::TrackVolume => AutomationTarget::TrackVolume { track_id },
            Self::TrackPan => AutomationTarget::TrackPan { track_id },
            Self::DeviceParam {
                slot_index,
                param_index,
            } => AutomationTarget::DeviceParam {
                track_id,
                slot_index,
                param_index,
            },
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
/// `ops[].kind` 的**取走资产索引项**形态名（写 [`Op::RemoveAsset`]）。
///
/// 与模型 `Op` 变体名同词（`RemoveAsset` 的小驼峰），与 [`REMOVE_CLIP_KIND`] 同一条
/// 命名规则。
///
/// ⚠⭐ **它是资产唯一的删除通路**：在它之前 `Op::RemoveAsset` 在 `yeban-mcp` 里
/// **零构造点**（只由模型的 `structural_inverse` 产出 ⇒ 用户只能靠**撤销那次导入**
/// 取走它）。本仓同族实体（音轨/场景/段落/片段/设备/路由节点/自动化点）**都有**
/// 显式 `remove*` 形态 ⇒ 本形态补齐资产那一格。
pub const REMOVE_ASSET_KIND: &str = "removeAsset";

/// 取走资产索引项形态的**寻址**字段名（`ops[].assetHash`）。
///
/// ⚠ 与 `yeban_import_audio` 的实参同名，但**不是**同一条通路：本形态在
/// `yeban_edit_notes` 上，该工具**没有**顶层 `assetHash` 实参 ⇒ ⛔ 必须**自带寻址**
/// （与 [`REMOVE_SCENE_KIND`] / [`REMOVE_SECTION_KIND`] 的 `sceneId` / `sectionId` 同型）。
pub const REMOVE_ASSET_HASH_FIELD: &str = "assetHash";

/// 取走资产索引项形态允许出现的**全部**键（判别键 + 一个寻址）。
pub const REMOVE_ASSET_FIELDS: [&str; 2] = ["kind", REMOVE_ASSET_HASH_FIELD];

/// `ops[].kind` 的**改资产索引项**形态名（写 [`Op::SetAsset`]，**更新**语义）。
///
/// ⚠ 与 [`REMOVE_ASSET_KIND`] 成对：那个删、这个改。⭐ 它补的是"**建得出、改不了**"
/// —— 在它之前 `Op::SetAsset` 的唯一构造点写死 `old_asset: None`（创建），
/// 而"同哈希换个 `license` 重导"被 `yeban_import_audio` 当**冲突**拒
/// （`assetDeclaredDifferent`，那是**有意**的：⛔ 不许静默改写许可留痕）。
/// 本形态给出一条**正门**：用户**明说**要改。
pub const SET_ASSET_KIND: &str = "setAsset";

/// 改资产形态的**可改字段**：许可标识（`ops[].license`）。
///
/// ⚠ 与 [`SET_ASSET_ORIGINAL_PATH_FIELD`] 同属**可改**；其余三字段**不可改**，
/// 且有明文依据（模型 `Op::SetAsset` 的文档逐字："之后同哈希再声明就是**更新
/// （改许可/原路径）**"）：
/// - `hash` **是键**（内容身份）⇒ 改它等于换一个资产；
/// - `byte_len` 是**内容事实**（容器字节数）⇒ 改它会与 CAS 池里的字节不符；
/// - `media_kind` 由 `yeban-decode` 的 `asset_index` 按**内容**判定。
pub const SET_ASSET_LICENSE_FIELD: &str = "license";

/// 改资产形态的**可改字段**：原路径留痕（`ops[].originalPath`）。
///
/// ⚠ 它是**留痕**（只有归档用，不参与寻址）⇒ 可改；改它不会让寻址失效。
/// 名字用线上的 camelCase（与 `project.schema.json` 的 `original_path` 同一字段）。
pub const SET_ASSET_ORIGINAL_PATH_FIELD: &str = "originalPath";

/// 改资产形态允许出现的**全部**键（判别键 + 寻址 + 两个可改字段）。
pub const SET_ASSET_FIELDS: [&str; 4] = [
    "kind",
    REMOVE_ASSET_HASH_FIELD,
    SET_ASSET_LICENSE_FIELD,
    SET_ASSET_ORIGINAL_PATH_FIELD,
];

/// 一个资产索引项的**逐键覆盖**（`None` = 调用方没提这个键 ⇒ 保留现值）。
///
/// ⚠ 只覆盖**可改**的两个字段（见 [`SET_ASSET_LICENSE_FIELD`]）：
/// `hash` / `byte_len` / `media_kind` **不在**覆盖范围内，因此本形态不可能
/// 把一条索引项改成"另一个资产"。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct AssetPatch {
    /// 许可标识（`None` = 保留现值）。
    pub license: Option<String>,
    /// 原路径留痕（`None` = 保留现值）。
    pub original_path: Option<String>,
}

impl AssetPatch {
    /// 该覆盖是否一个键都没提（`true` ⇒ 结果必然等于基础，是一次无操作）。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.license.is_none() && self.original_path.is_none()
    }

    /// 把覆盖施加到一条**基础**索引项上。
    ///
    /// `hash` / `byte_len` / `media_kind` **原样保留**（⛔ 不可改）。
    #[must_use]
    pub fn apply_to(&self, base: &yeban_model::AssetMetadata) -> yeban_model::AssetMetadata {
        let mut asset = base.clone();
        if let Some(license) = &self.license {
            asset.license.clone_from(license);
        }
        if let Some(path) = &self.original_path {
            asset.original_path.clone_from(path);
        }
        asset
    }
}

/// `ops[].kind` 的**取走片段池条目**形态名（写 [`Op::RemoveClip`]）。
///
/// 与模型 `Op` 变体名同词（`RemoveClip` 的小驼峰）⇒ 三个名字（工具 `kind`、
/// 模型变体、契约分支）逐字一致。
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

/// `ops[].kind` 的**设置速度**形态名（写 [`Op::SetTempo`]）。
///
/// 与模型 `Op` 变体名同词（`SetTempo` 的小驼峰）。
///
/// ⚠ 它是**顶层文档字段**形态（与 [`SET_TRANSPORT_CONFIG_KIND`] / [`SET_AUTHOR_KIND`]
/// 同族）：不读不写音符，也不寻址音轨。
pub const SET_TEMPO_KIND: &str = "setTempo";

/// 设置速度形态的**新速度**字段名（`ops[].bpm`，必填，数字）。
///
/// ⚠ 区间与有限性由 `Op::SetTempo` 的前置条件拒（`MIN_BPM..=MAX_BPM` =
/// `20.0..=999.0`）—— 本层**提前**报同一个区间（`bpmOutOfRange`），⛔ 不靠 apply 后的
/// `YebanProjectV1::validate` 兜底。
pub const SET_TEMPO_BPM_FIELD: &str = "bpm";

/// 设置速度形态允许出现的**全部**键（判别键 + 一个必填）。
pub const SET_TEMPO_FIELDS: [&str; 2] = ["kind", SET_TEMPO_BPM_FIELD];

/// `ops[].kind` 的**设置署名**形态名（写 [`Op::SetAuthor`]）。
pub const SET_AUTHOR_KIND: &str = "setAuthor";

/// 设置署名形态的**新署名**字段名（`ops[].author`，必填，字符串）。
///
/// ⚠ **⛔ 本层不发明**"非空"或"长度上限"：模型 `Op::SetAuthor` 的校验**只有**
/// "文档现值必须逐字等于 `old_author`"（陈旧载荷 ⇒ `OpStateMismatch`），而
/// `schemas/project.schema.json` 的 `author` 是**裸 `{"type":"string"}`**（非必需、
/// 无 `minLength`、无 `pattern`）⇒ 空串是一个**合法的明示值**（"显式清空署名"）。
/// 故本形态**照抄模型口径**：收任意字符串。
pub const SET_AUTHOR_AUTHOR_FIELD: &str = "author";

/// 设置署名形态允许出现的**全部**键（判别键 + 一个必填）。
pub const SET_AUTHOR_FIELDS: [&str; 2] = ["kind", SET_AUTHOR_AUTHOR_FIELD];

/// `ops[].kind` 的**设置标题**形态名（写 [`Op::SetTitle`]）。
///
/// 与模型 `Op` 变体名同词（`SetTitle` 的小驼峰），与 [`SET_AUTHOR_KIND`] 同一条规则。
///
/// ⚠ 它是**顶层文档字段**形态（与 [`SET_TRANSPORT_CONFIG_KIND`] / [`SET_TEMPO_KIND`] /
/// [`SET_AUTHOR_KIND`] 同族）：不读不写音符，也不寻址音轨。
pub const SET_TITLE_KIND: &str = "setTitle";

/// 设置标题形态的**新标题**字段名（`ops[].title`，必填，字符串）。
///
/// ⚠ **⛔ 本层不发明**"非空"或"长度上限"：模型 `Op::SetTitle` 的校验**只有**
/// "文档现值必须逐字节等于 `old_title`"（陈旧载荷 ⇒ `OpStateMismatch`），而
/// `schemas/project.schema.json` 的 `title` 是**裸 `{"type":"string"}`**（必需，
/// 但无 `minLength`、无 `maxLength`、无 `pattern`）⇒ 空串是一个**合法的明示值**
/// （"显式清空标题"）。故本形态**照抄模型口径**：收任意字符串。
pub const SET_TITLE_TITLE_FIELD: &str = "title";

/// 设置标题形态允许出现的**全部**键（判别键 + 一个必填）。
pub const SET_TITLE_FIELDS: [&str; 2] = ["kind", SET_TITLE_TITLE_FIELD];

/// `ops[].kind` 的**设置音轨名**形态名（写 [`Op::SetTrackName`]）。
pub const SET_TRACK_NAME_KIND: &str = "setTrackName";

/// 设置音轨名形态的**目标音轨**字段名（`ops[].trackId`，必填，ULID）。
pub const SET_TRACK_NAME_TRACK_FIELD: &str = "trackId";

/// 设置音轨名形态的**新名字**字段名（`ops[].name`，必填，字符串）。
///
/// ⚠ **⛔ 本层不发明**"非空"或长度上限：模型 [`Op::SetTrackName`] 的校验**只有**
/// "文档现值必须逐字等于 `old_name`"（陈旧载荷 ⇒ `OpStateMismatch`），而
/// `TrackV3::validate` **不查**空名（同 [`Op::AddTrack`] 那处的口径）⇒ 空串合法
/// （"显式清空名字"）。⛔ 而 [`Op::AddTrack`] 的 `name` 走 `read_required_non_empty_string`
/// 是因为**建轨**需要一个可寻址的展示名 —— 改名与建名不是同一条口径。
pub const SET_TRACK_NAME_NAME_FIELD: &str = "name";

/// 设置音轨名形态允许出现的**全部**键（判别键 + 一个必填）。
pub const SET_TRACK_NAME_FIELDS: [&str; 3] = [
    "kind",
    SET_TRACK_NAME_TRACK_FIELD,
    SET_TRACK_NAME_NAME_FIELD,
];

/// `ops[].kind` 的**设置片段名**形态名（写 [`Op::SetClipName`]）。
pub const SET_CLIP_NAME_KIND: &str = "setClipName";

/// 设置片段名形态的**目标片段**字段名（`ops[].clipId`，必填，ULID）。
pub const SET_CLIP_NAME_CLIP_FIELD: &str = "clipId";

/// 设置片段名形态的**新名字**字段名（`ops[].name`，必填，字符串）。
///
/// ⚠ **⛔ 本层不发明**"非空"或长度上限：模型 [`Op::SetClipName`] 的校验**只有**
/// "文档现值必须逐字等于 `old_name`"（陈旧载荷 ⇒ `OpStateMismatch`），而
/// `ClipPoolEntry` 的校验**不查**空名 ⇒ 空串合法（"显式清空名字"）。
/// ⚠ 与 [`SET_TRACK_NAME_NAME_FIELD`] 同一口径，⛔ 与 `addClip` 的"必需标签"不是同一条。
pub const SET_CLIP_NAME_NAME_FIELD: &str = "name";

/// 设置片段名形态允许出现的**全部**键（判别键 + 两个必填）。
pub const SET_CLIP_NAME_FIELDS: [&str; 3] =
    ["kind", SET_CLIP_NAME_CLIP_FIELD, SET_CLIP_NAME_NAME_FIELD];

/// `ops[].kind` 的**设置音频片段增益**形态名（写 [`Op::SetClipGain`]）。
///
/// 与模型 `Op` 变体名同词（`SetClipGain` 的小驼峰），与 [`SET_CLIP_NAME_KIND`]
/// 同一条规则。⭐ 本形态补的是「音频片段建得出、增益改不了」—— `gain_db` 此前只在
/// `yeban_import_audio` 创建时由 `gainDb` 写一次。
///
/// ⚠ 它是**池级、自带片段寻址**的形态：`clipId` 在操作对象里（同
/// [`SET_CLIP_NAME_KIND`]），⛔ 不是工具顶层的 `clipId`（那个由 `#177` 的
/// `x-envelope.clipId` 管）。
pub const SET_CLIP_GAIN_KIND: &str = "setClipGain";

/// 设置片段增益形态的**目标片段**字段名（`ops[].clipId`，必填，ULID）。
///
/// 与 [`SET_CLIP_NAME_CLIP_FIELD`] **同一个字面量**（同一个词 = 同一个意思）——
/// 目标都是 `clip_pool` 的键。
pub const SET_CLIP_GAIN_CLIP_FIELD: &str = SET_CLIP_NAME_CLIP_FIELD;

/// 设置片段增益形态的**新增益**字段名（`ops[].gainDb`，必填，数字，单位 dB）。
///
/// ⚠ **⛔ 本层不发明** dB 范围：模型的 `ClipContent::validate` 对 `gain_db`
/// **只**查有限性（字段名 `clip.audio.gain_db`，`crates/yeban-model/src/project.rs:1040-1047`），
/// **没有** min/max；契约里 `gain_db` 也是裸 `{"type":"number"}`
/// （`schemas/project.schema.json`）⇒ 本形态收**任意有限 `f32`**，非有限由
/// `read_number` 在**收窄之后**拒（`data.reason = "nonFiniteValue"`），
/// 与 `setParam` / `setRoutingGain` 同一条口径。
pub const SET_CLIP_GAIN_GAIN_FIELD: &str = "gainDb";

/// 设置片段增益形态允许出现的**全部**键（判别键 + 两个必填）。
pub const SET_CLIP_GAIN_FIELDS: [&str; 3] =
    ["kind", SET_CLIP_GAIN_CLIP_FIELD, SET_CLIP_GAIN_GAIN_FIELD];

/// `ops[].kind` 的**设置设备名**形态名（写 [`Op::SetDeviceName`]）。
pub const SET_DEVICE_NAME_KIND: &str = "setDeviceName";

/// 设置设备名形态的**新名字**字段名（`ops[].name`，必填，字符串）。
///
/// ⚠ **⛔ 本层不发明**"非空"或长度上限：模型 [`Op::SetDeviceName`] 的校验**只有**
/// "文档现值必须逐字等于 `old_name`"（陈旧载荷 ⇒ `OpStateMismatch`）⇒ 空串合法
/// （"显式清空名字"），与 [`SET_CLIP_NAME_NAME_FIELD`] / [`SET_TRACK_NAME_NAME_FIELD`]
/// 同一口径。⚠ 目标音轨与设备复用既有的 [`DEVICE_TRACK_FIELD`] / [`DEVICE_ID_FIELD`]，
/// ⛔ 不另立一套拼写。
pub const SET_DEVICE_NAME_NAME_FIELD: &str = "name";

/// 设置设备名形态允许出现的**全部**键（判别键 + 目标音轨 + 目标设备 + 新名字）。
pub const SET_DEVICE_NAME_FIELDS: [&str; 4] = [
    "kind",
    DEVICE_TRACK_FIELD,
    DEVICE_ID_FIELD,
    SET_DEVICE_NAME_NAME_FIELD,
];

/// `ops[].kind` 的**切换设备旁通**形态名（写 [`Op::SetDeviceBypassed`]）。
pub const SET_DEVICE_BYPASSED_KIND: &str = "setDeviceBypassed";

/// 切换设备旁通形态的**目标旁通态**字段名（`ops[].bypassed`，必填，布尔）。
///
/// ⚠ **只收 JSON 布尔**（照 `setTrackMute`／`setTrackSolo` 的 `value` 口径：
/// 不做真假值强转）⇒ 非布尔一律 `INVALID_PARAMETER_RANGE`。
/// ⚠ 目标音轨与设备复用既有的 [`DEVICE_TRACK_FIELD`] / [`DEVICE_ID_FIELD`]。
pub const SET_DEVICE_BYPASSED_BYPASSED_FIELD: &str = "bypassed";

/// 切换设备旁通形态允许出现的**全部**键（判别键 + 目标音轨 + 目标设备 + 旁通态）。
pub const SET_DEVICE_BYPASSED_FIELDS: [&str; 4] = [
    "kind",
    DEVICE_TRACK_FIELD,
    DEVICE_ID_FIELD,
    SET_DEVICE_BYPASSED_BYPASSED_FIELD,
];

/// `ops[].kind` 的**切换摆放静音**形态名（写 [`Op::SetClipPlacementMuted`]）。
pub const SET_CLIP_PLACEMENT_MUTED_KIND: &str = "setClipPlacementMuted";

/// 切换摆放静音形态的**目标音轨**字段名（`ops[].trackId`，必填，ULID）。
pub const SET_CLIP_PLACEMENT_MUTED_TRACK_FIELD: &str = "trackId";

/// 切换摆放静音形态的**目标摆放**字段名（`ops[].placementId`，必填，ULID）。
///
/// ⚠ **平铺**（⛔ 不嵌套 `placement` 对象）—— 与 `setDeviceBypassed` 的 `deviceId` 同形，
/// 因为 `set*` 家族一律是**平铺**的（嵌套 `placement` 对象属于 `add`/`move`/`remove`
/// 那三个走顶层实参的形态）。
pub const SET_CLIP_PLACEMENT_MUTED_PLACEMENT_FIELD: &str = "placementId";

/// 切换摆放静音形态的**目标静音态**字段名（`ops[].muted`，必填，布尔）。
///
/// ⚠ **只收 JSON 布尔**（照 `setTrackMute`／`setDeviceBypassed` 口径：⛔ 不做真假值强转）。
pub const SET_CLIP_PLACEMENT_MUTED_MUTED_FIELD: &str = "muted";

/// 切换摆放静音形态允许出现的**全部**键（判别键 + 目标音轨 + 目标摆放 + 静音态）。
pub const SET_CLIP_PLACEMENT_MUTED_FIELDS: [&str; 4] = [
    "kind",
    SET_CLIP_PLACEMENT_MUTED_TRACK_FIELD,
    SET_CLIP_PLACEMENT_MUTED_PLACEMENT_FIELD,
    SET_CLIP_PLACEMENT_MUTED_MUTED_FIELD,
];

/// `ops[].kind` 的**设置摆放循环**形态名（写 [`Op::SetClipPlacementLoop`]，`#336`）。
///
/// ⭐ **本形态补上了什么**：⭐ 在它之前 [`ClipPlacement::loop_config`] **只在创建时**
/// 能给（`placement` 的 `add` 形态带 `loopEnabled` / `loopStartTick` / `loopEndTick`）
/// ⇒ ⭐ **创建之后改不了循环**；⭐ 而 `setClipPlacementDuration` 只改长度、重建摆放会
/// **换掉摆放身份**。这是 [`SET_CLIP_PLACEMENT_MUTED_KIND`] 文档里"`loopConfig` 是
/// 结构体要另议"那句的另议结果。
pub const SET_CLIP_PLACEMENT_LOOP_KIND: &str = "setClipPlacementLoop";

/// 设置摆放循环形态的**目标音轨**字段名（`ops[].trackId`，必填，ULID）。
pub const SET_CLIP_PLACEMENT_LOOP_TRACK_FIELD: &str = "trackId";

/// 设置摆放循环形态的**目标摆放**字段名（`ops[].placementId`，必填，ULID）。
pub const SET_CLIP_PLACEMENT_LOOP_PLACEMENT_FIELD: &str = "placementId";

/// 设置摆放循环形态允许出现的**全部**键（判别键 + 目标音轨 + 目标摆放 + 三项循环配置）。
///
/// ⚠ **三项循环配置的字面名复用 `add` 形态的 `placement.*` 常量**
/// （[`LOOP_ENABLED_FIELD`] / [`LOOP_START_TICK_FIELD`] / [`LOOP_END_TICK_FIELD`]）：
/// "同一个词必须同一个意思"（`ADR-0001` D48）—— 本形态只是把**同一组字段**从
/// **创建**挪到**整体替换**，⛔ 不另造三个同义拼写。三个都**必填**：
/// ⭐ 本形态是**整体替换**（与 `setNoteExpression` / `setNotePitchShape` 同形），
/// ⛔ 不是"合并式 patch"（后者是 `setScene` / `setSection` 的形状）⇒ 缺一个就是
/// 伪造作者的选择（`ADR-0001` D43：`enabled` 缺省会把打开的循环静默关掉、
/// 区间缺省会把循环静默搬回 tick 0）。
///
/// ⚠ **⛔ 没有 `oldLoopConfig` / `oldEnabled` / `oldStartTick` / `oldEndTick`，
/// 也没有任何别的旧值键**：载荷**只带新值**，旧值由 [`compile`] 从**当前文档**读
/// （与 `setNoteExpression` / `setNotePitchShape` 同一条纪律，见 `#334`）。
/// 写旧值键 ⇒ 未知键，[`reject_set_clip_placement_loop_fields`] **响亮拒绝**并列出
/// 本支持集合（⛔ 绝不静默忽略）。
pub const SET_CLIP_PLACEMENT_LOOP_FIELDS: [&str; 6] = [
    "kind",
    SET_CLIP_PLACEMENT_LOOP_TRACK_FIELD,
    SET_CLIP_PLACEMENT_LOOP_PLACEMENT_FIELD,
    LOOP_ENABLED_FIELD,
    LOOP_START_TICK_FIELD,
    LOOP_END_TICK_FIELD,
];

/// `ops[].kind` 的**设置音轨色标**形态名（写 [`Op::SetTrackColor`]）。
pub const SET_TRACK_COLOR_KIND: &str = "setTrackColor";

/// 设置音轨色标形态的**目标音轨**字段名（`ops[].trackId`，必填，ULID）。
pub const SET_TRACK_COLOR_TRACK_FIELD: &str = "trackId";

/// 设置音轨色标形态的**色标**字段名（`ops[].color`，**必填**，非空字符串或 `null`）。
///
/// ⚠ **`null` = 清除**（⭐ 回到主题默认色 —— ⭐ 模型的 `None` 是**一等状态**，⭐ 见
/// `TrackV3::color` 的文档）。⭐ **缺键 ⛔ 不等于清除**：⭐ 本形态是**单字段设值**、
/// ⛔ 不是"合并式 patch"（⭐ 后者是 `setScene`／`setSection` 的形状）⇒ ⭐ 缺键**响亮拒绝**。
/// ⚠ **非空**：⭐ 照 [`SCENE_COLOR_FIELD`] 的既有口径（`read_non_empty_string`，
/// reason `sceneColorMustBeNonEmptyString`）⇒ 本源用 `trackColorMustBeNonEmptyString`。
/// ⚠ 模型层**没有**校验色值格式（`parse_hex_color` 在 `yeban-app`、非法即回退默认）⇒
/// 本层也⛔ **不自造格式判断**。
pub const SET_TRACK_COLOR_COLOR_FIELD: &str = "color";

/// 设置音轨色标形态允许出现的**全部**键（判别键 + 目标音轨 + 色标）。
pub const SET_TRACK_COLOR_FIELDS: [&str; 3] = [
    "kind",
    SET_TRACK_COLOR_TRACK_FIELD,
    SET_TRACK_COLOR_COLOR_FIELD,
];

/// `ops[].kind` 的**绑定采样器**形态名（写 [`Op::SetTrackSampler`]，⭐ 任务单 `#221`）。
///
/// ⚠ ⭐ **它补上的缺口是 `#220` 的缺陷 ①**：在它之前 `TrackV3::sampler` 在**工具面**
/// 完全不可达 —— `tools/list` 的 22 个工具与 `yeban_edit_notes` 当时的 **35** 个
/// kind 里一个 `sampler` 字段都没有，于是 `setTrackSampler` 只能报
/// `INVALID_PARAMETER_RANGE` / `未知 kind: setTrackSampler`，而**模型里
/// `Op::SetTrackSampler` 早就存在**（`crates/yeban-model/src/ops.rs`）。
pub const SET_TRACK_SAMPLER_KIND: &str = "setTrackSampler";

/// 绑定采样器形态的**目标音轨**字段名（`ops[].trackId`，必填，ULID）。
pub const SET_TRACK_SAMPLER_TRACK_FIELD: &str = "trackId";

/// 绑定采样器形态的**采样器路径**字段名（`ops[].sampler`，**必填**，非空字符串或 `null`）。
///
/// ⚠ `null` = **解绑**（回到"这条轨不用采样器"）—— 模型的 `None` 是**一等状态**；
/// ⚠ **缺键 ⛔ 不等于解绑**（本形态是单字段设值、⛔ 不是合并式 patch）。
///
/// ⚠ ⭐ **本层不发明路径语义**：不查存在性、不查扩展名、不规范化 —— 与
/// [`SamplerBinding`] 的字段文档逐字同口径（模型把路径当留痕字符串）。
/// "这个路径读不到"由**渲染**那一侧响亮拒绝（`RENDER_FAILED` / `samplerUnreadable`），
/// ⛔ 不在解析期猜。
pub const SET_TRACK_SAMPLER_SAMPLER_FIELD: &str = "sampler";

/// 绑定采样器形态允许出现的**全部**键（判别键 + 目标音轨 + 采样器路径）。
pub const SET_TRACK_SAMPLER_FIELDS: [&str; 3] = [
    "kind",
    SET_TRACK_SAMPLER_TRACK_FIELD,
    SET_TRACK_SAMPLER_SAMPLER_FIELD,
];

/// `ops[].kind` 的**设置摆放长度**形态名（写 [`Op::SetClipPlacementDuration`]）。
pub const SET_CLIP_PLACEMENT_DURATION_KIND: &str = "setClipPlacementDuration";

/// 设置摆放长度形态的**目标音轨**字段名（`ops[].trackId`，必填，ULID）。
pub const SET_CLIP_PLACEMENT_DURATION_TRACK_FIELD: &str = "trackId";

/// 设置摆放长度形态的**目标摆放**字段名（`ops[].placementId`，必填，ULID）。
pub const SET_CLIP_PLACEMENT_DURATION_PLACEMENT_FIELD: &str = "placementId";

/// 设置摆放长度形态的**长度**字段名（`ops[].durationTicks`，**必填**，非零整数）。
///
/// ⚠ 按第 200 条：⭐ 本形态是**单字段设值**、⛔ 不是合并式 patch ⇒ ⭐ **必填**，
/// 且 **`null` ⛔ 不适用**（`u64` 不可空）⇒ ⭐ **缺键／非数／非整数一律响亮拒绝**、
/// ⛔ **不做真假值强转**（⭐ 与 `setTrackMute`／`setDeviceBypassed` 同一纪律）。
/// ⚠ **必须 ≥ 1**：⭐ 模型 `ClipPlacement::validate` 对 `0` 报 `ZeroDuration`
/// ⇒ ⭐ 本层⛔ 不自造第二条口径，⭐ 但会在**取值时**要求正整数（⭐ `0` 由模型层响亮拒绝）。
pub const SET_CLIP_PLACEMENT_DURATION_DURATION_FIELD: &str = "durationTicks";

/// 设置摆放长度形态允许出现的**全部**键（判别键 + 目标音轨 + 目标摆放 + 长度）。
pub const SET_CLIP_PLACEMENT_DURATION_FIELDS: [&str; 4] = [
    "kind",
    SET_CLIP_PLACEMENT_DURATION_TRACK_FIELD,
    SET_CLIP_PLACEMENT_DURATION_PLACEMENT_FIELD,
    SET_CLIP_PLACEMENT_DURATION_DURATION_FIELD,
];

/// `ops[].kind` 的**新增音轨**形态名（写 [`Op::AddTrack`]）。
///
/// 与模型 `Op` 变体名同词（`AddTrack` 的小驼峰），与 [`REMOVE_TRACK_KIND`] 同一条
/// 命名规则。
///
/// ⚠ **它与 [`REMOVE_TRACK_KIND`] 是一对**：在那个形态之前，工具面**删得到**音轨却
/// ⛔ **加不到**（加音轨此前只在 `yeban_create_project` 的建批与 `yeban_propose_section`
/// 的配器骨架里发生）⇒ 一个**已存在的**工程无法被逐步扩展。
pub const ADD_TRACK_KIND: &str = "addTrack";

/// 新增音轨形态的目标字段名（`ops[].trackId`，必填）。
///
/// ⚠ 这个名字与工具**顶层**的 `trackId` 逐字相同，但**不是**同一个东西：
/// 顶层那个是"目标片段所在的音轨"，本形态这个是"**要新建**的那条音轨的身份"
/// （与 [`REMOVE_TRACK_KIND`] 的自带 `trackId` 同一条口径）。
pub const ADD_TRACK_TRACK_FIELD: &str = "trackId";

/// 新增音轨形态的**显示名**字段名（`ops[].name`，必填，非空字符串）。
pub const ADD_TRACK_NAME_FIELD: &str = "name";

/// 新增音轨形态的**音轨类型**字段名（`ops[].trackKind`，必填，规范枚举串）。
///
/// ⚠ 取值与线协议**逐字相同**（`Midi` / `Audio` / `AuxReturn` / `Master`）——
/// 那是 `schemas/project.schema.json` 的 `track.kind.enum` 与模型 `TrackKind` 的
/// `serde` 名，因此本层**不另发明**一套拼写。
pub const ADD_TRACK_KIND_FIELD: &str = "trackKind";

/// 新增音轨形态允许出现的**全部**键（判别键 + 三个必填字段）。
///
/// ⚠⭐ **四个集合与 `folderId`/`color` 以及五个混音标量都**不在**这里** —— 那是
/// **决定**，理由 = **新实体无内容可清**：`ADR-0001 D43` 的必需性立法理由是
/// "缺键 = 静默清空**既有**内容"，而创建时没有既有内容。本层照抄同族创建先例
/// （`project_create.rs:381` 的 `TrackV3 { id, name, kind, ..Default::default() }`
/// 与 `setScene` 的 create 分支）⇒ 编译期用**空形状**。多写一个键是**响亮失败**。
pub const ADD_TRACK_FIELDS: [&str; 4] = [
    "kind",
    ADD_TRACK_TRACK_FIELD,
    ADD_TRACK_NAME_FIELD,
    ADD_TRACK_KIND_FIELD,
];

/// `ops[].kind` 的**新建路由节点**形态名（写 [`Op::AddRoutingNode`]）。
///
/// 与模型 `Op` 变体名同词（`AddRoutingNode` 的小驼峰），与 [`REMOVE_ROUTING_NODE_KIND`]
/// 同一条命名规则。
///
/// ⚠ 两个节点形态**互为逆操作**（`Op::invert` 把它们互相换成对方），且模型的两份载荷
/// **都只有节点身份**（⛔ 没有撤销载荷）⇒ 因此它们共用同一个寻址字段
/// [`ROUTING_NODE_FIELD`]。
pub const ADD_ROUTING_NODE_KIND: &str = "addRoutingNode";

/// 新建路由节点形态允许出现的**全部**键（判别键 + 寻址键）。
///
/// 目标音轨、目标片段与节点的旧状态**都不在**这里：本形态自带寻址
/// （[`ROUTING_NODE_FIELD`]），而 `AddRoutingNode` 的模型载荷**只有**节点身份
/// —— 它没有撤销载荷（与 [`REMOVE_ROUTING_NODE_KIND`] 互为逆操作）。
/// 多写一个键是**响亮失败**，不静默丢弃。
pub const ADD_ROUTING_NODE_FIELDS: [&str; 2] = ["kind", ROUTING_NODE_FIELD];

/// `ops[].kind` 的**建立路由边**形态名（写 [`Op::ConnectRouting`]）。
///
/// 与模型 `Op` 变体名同词（`ConnectRouting` 的小驼峰），与
/// [`DISCONNECT_ROUTING_KIND`] 同一条命名规则。
///
/// ⚠ **它是 [`DISCONNECT_ROUTING_KIND`] 的逆**，也是路由图可用性的**最后一环**：
/// 在它之前工具面**建得起节点、连不起边**（边只在 `yeban_create_project` 的建批与
/// `yeban_propose_section` 的声部连接里发生）。
pub const CONNECT_ROUTING_KIND: &str = "connectRouting";

/// 建立路由边形态的**边身份**字段名（`ops[].edgeId`，必填）。
///
/// 与 [`SET_ROUTING_GAIN_KIND`] / [`DISCONNECT_ROUTING_KIND`] 的 `edgeId` **逐字同词**
/// （`ADR-0001` D48：同一个词必须同一个意思 —— 被寻址的那条**边**）。
/// 建立路由边形态的**边身份**字段名（= [`ROUTING_EDGE_FIELD`]，同一个字面量）。
///
/// ⚠ 与 [`SET_ROUTING_GAIN_EDGE_FIELD`] 同一条做法：**不复制**那个字面量，而是
/// 引用它 —— 两条形态寻址的是**同一种实体**（`routing_graph.edges` 里的那条边）。
pub const CONNECT_EDGE_FIELD: &str = ROUTING_EDGE_FIELD;

/// 建立路由边形态的**源节点**字段名（`ops[].sourceNode`，必填）。
///
/// ⚠ 两个端点都必须是 `routing_graph.nodes` 里**已有**的身份（模型 `ConnectRouting`
/// 的前置条件）—— 因此正常的用法是**先**用 [`ADD_ROUTING_NODE_KIND`] 把节点加进图，
/// **再**用本形态连边。
pub const CONNECT_SOURCE_FIELD: &str = "sourceNode";

/// 建立路由边形态的**目标节点**字段名（`ops[].destinationNode`，必填）。
pub const CONNECT_DESTINATION_FIELD: &str = "destinationNode";

/// 建立路由边形态的**路由类型**字段名（`ops[].edgeKind`，必填，规范枚举串）。
///
/// ⚠ 取值与线协议**逐字相同**（`TrackToBus` / `BusToMaster` / `SendToAux` /
/// `Sidechain`）—— 那是模型 `RoutingKind` 的 `serde` 名，因此本层**不另发明**拼写。
/// ⚠ 字段名**不叫** `kind`：那个词在本层是**判别键**（`ops[].kind`），复用会让
/// "这一项是哪一形态"与"这条边是什么路由类型"混成一个词。
pub const CONNECT_EDGE_KIND_FIELD: &str = "edgeKind";

/// 建立路由边形态的**增益**字段名（`ops[].gainDb`，**可选**）。
///
/// ⚠ 它是 `Option<f32>` 的平铺写法，与 [`SET_ROUTING_GAIN_KIND`] 的 `value` **同口径**
/// （⛔ 不造第二套）：`null` = `None` = **单位增益**；数字 = `Some`（必须有限）；
/// **整键省略** = `None`（与 `null` 同义 —— 因为 `RoutingEdge::gain_db` 带
/// `#[serde(default)]`，模型的"缺键"就是单位增益）。
pub const CONNECT_GAIN_FIELD: &str = "gainDb";

/// 建立路由边形态允许出现的**全部**键（判别键 + 四个必填 + 一个可选）。
///
/// ⚠ **平铺**：⛔ 不嵌套一个 `edge: {...}` 对象 —— 与 [`ADD_ROUTING_NODE_KIND`] /
/// [`DISCONNECT_ROUTING_KIND`] 同一条形状（同族形态都平铺）。
/// 多写一个键是**响亮失败**，不静默丢弃。
pub const CONNECT_ROUTING_FIELDS: [&str; 6] = [
    "kind",
    CONNECT_EDGE_FIELD,
    CONNECT_SOURCE_FIELD,
    CONNECT_DESTINATION_FIELD,
    CONNECT_EDGE_KIND_FIELD,
    CONNECT_GAIN_FIELD,
];

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
pub const SET_SECTION_KIND: &str = "setSection";

/// `setSection` 的**自包含载荷**字段名（`ops[].section`，必填对象）。
///
/// ⚠ 与 [`SET_SCENE_KIND`] 同一条形状：四个属性都放在这个**嵌套对象**里，
/// 操作对象顶层只认 `kind` 与它。
pub const SECTION_PAYLOAD_FIELD: &str = "section";

/// `setSection` 载荷里的**新建**判别键（`section.create`，可选布尔，缺省 = 更新）。
///
/// ⚠ 与 [`SET_SCENE_KIND`] 的 `create` **逐字同词**：新建与更新由**显式**的它区分，
/// ⛔ 不靠「文档里没有这个身份就当作新建」—— 那会让一个打错的 `sectionId`
/// 静默建出一个新段落。
pub const SECTION_CREATE_FIELD: &str = "create";

/// `setSection` 载荷里的**段落名**（`section.name`，非空字符串）。
pub const SECTION_NAME_FIELD: &str = "name";

/// `setSection` 载荷里的**起始 tick**（`section.startTick`，非负整数）。
///
/// ⚠ 与 [`SECTION_END_FIELD`] 一起构成段落的**区间**：模型 `SectionV3::validate`
/// 要求 `end_tick > start_tick`（否则 `ZeroDuration`）⇒ 工具面照抄那条约束、⛔ 不发明第二套。
pub const SECTION_START_FIELD: &str = "startTick";

/// `setSection` 载荷里的**结束 tick**（`section.endTick`，非负整数）。
pub const SECTION_END_FIELD: &str = "endTick";

/// `setSection` 载荷里的**界面色标**（`section.color`，缺省 = 保留；字符串 = 覆盖；`null` = 清空）。
///
/// 三态与 [`SET_SCENE_KIND`] 的 `color` **逐条同口径**。
pub const SECTION_COLOR_FIELD: &str = "color";

/// `setSection` 载荷允许出现的**全部**键（新建判别 + 四个属性）。
pub const SET_SECTION_FIELDS: [&str; 6] = [
    SECTION_CREATE_FIELD,
    SECTION_FIELD,
    SECTION_START_FIELD,
    SECTION_END_FIELD,
    SECTION_NAME_FIELD,
    SECTION_COLOR_FIELD,
];

/// `setSection` 操作对象顶层允许的**全部**键（判别键 + 载荷键）。
pub const SET_SECTION_OP_FIELDS: [&str; 2] = ["kind", SECTION_PAYLOAD_FIELD];

/// `ops[].kind` 的**设置场景**形态名（写 [`Op::SetScene`]）。
///
/// 与模型 `Op` 变体名同词（`SetScene` 的小驼峰），与 [`SET_SECTION_KIND`] 同一条命名规则。
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

/// `ops[].kind` 的**取走音轨**形态名（写 [`Op::RemoveTrack`]）。
///
/// 与模型 `Op` 变体名同词（`RemoveTrack` 的小驼峰），与 [`REMOVE_SECTION_KIND`] /
/// [`REMOVE_SCENE_KIND`] / [`REMOVE_CLIP_KIND`] 同一条命名规则。
pub const REMOVE_TRACK_KIND: &str = "removeTrack";

/// 取走音轨形态的**目标**字段名（`ops[].trackId`，必填）。
///
/// 音轨身份与路由节点身份（[`ROUTING_NODE_FIELD`]）、段落身份（[`SECTION_FIELD`]）、
/// 场景身份（[`SCENE_FIELD`]）、片段身份（顶层 `clipId`）**不是**同一个字面量：
/// 五者是五种实体，共用一个词会让"取走的是哪一个"从形状上无法区分。
///
/// ⚠ 本形态的寻址**不用**工具顶层的 `trackId`：那个实参是"目标片段所在的音轨"
/// （[`compile`] 的入口拿它查音轨与片段池），而本形态自带寻址 —— 与三个路由级形态、
/// 段落形态、场景形态同一纪律。顶层的 `trackId` / `clipId` 对本形态无意义。
pub const REMOVE_TRACK_FIELD: &str = "trackId";

/// 取走音轨形态允许出现的**全部**键（判别键 + 寻址键）。
///
/// 目标片段与音轨的旧状态**都不在**这里：本形态自带寻址（[`REMOVE_TRACK_FIELD`]），
/// 而撤销载荷 `previous_track` 由 [`compile`] 从**当前文档**读。
/// 多写一个键是**响亮失败**，不静默丢弃。
pub const REMOVE_TRACK_FIELDS: [&str; 2] = ["kind", REMOVE_TRACK_FIELD];

/// `ops[].kind` 的**插入设备**形态名（写 [`Op::InsertDevice`]）。
///
/// 与模型 `Op` 变体名同词（`InsertDevice` 的小驼峰），与 [`REMOVE_DEVICE_KIND`]
/// 同一条命名规则。
///
/// 为什么需要它：`Op::InsertDevice` / `Op::RemoveDevice` 在 `crates/yeban-mcp/src` 里的
/// **构造点**实测是 **0**（两口径普查见本文件的模块头表）⇒ 设备链在工具面上
/// **只读**（`yeban_query_engine_state` 的 `tracks[].devices[]`）而**不可写**，
/// 可是设备链的 `latency_samples` 是 PDC 的**唯一**延迟来源（`ARCH-PDC-001`）：
/// 一台外部设备装了却报不出来，延迟补偿就没有输入。
pub const INSERT_DEVICE_KIND: &str = "insertDevice";

/// `ops[].kind` 的**取走设备**形态名（写 [`Op::RemoveDevice`]）。
pub const REMOVE_DEVICE_KIND: &str = "removeDevice";

/// 设备形态的**目标音轨**字段名（`ops[].trackId`，两个设备形态都必填）。
///
/// 与 [`REMOVE_TRACK_FIELD`] 逐字同词：那个实参的语义就是"哪一条音轨"，而设备链
/// 挂在音轨上 —— 这是本文件里**唯一**两处共用同一个寻址字段名的形态，因为两处指的
/// 是同一个对象（音轨），而 [`ROUTING_NODE_FIELD`] / [`SECTION_FIELD`] /
/// [`SCENE_FIELD`] 指的是**别的**实体（因此各有各的字面量）。
///
/// ⚠ 它**不是**工具顶层的 `trackId`（那是"目标片段所在的音轨"，[`compile`] 的入口
/// 用它查音轨与片段池）：设备形态**自带寻址**，与 [`REMOVE_TRACK_KIND`] 同一纪律。
pub const DEVICE_TRACK_FIELD: &str = "trackId";

/// 插入设备形态的**插槽下标**字段名（`ops[].slotIndex`，可选）。
///
/// 缺省 = **追加到链尾**（模型允许 `slot_index == devices.len()`）。给出的值必须
/// `<= devices.len()`；`> len` 是响亮失败（`OUT_OF_RANGE`，`deviceSlotOutOfRange`），
/// 不静默夹紧 —— 夹紧会让"插到第 7 槽"变成"插到链尾"，而调用方以为插在中间
/// （PDC 的关键路径按链上位置算，位置错了相位就错了）。
pub const DEVICE_SLOT_FIELD: &str = "slotIndex";

/// 插入设备形态的**设备载荷**字段名（`ops[].device`，必填对象）。
pub const DEVICE_PAYLOAD_FIELD: &str = "device";

/// 取走设备形态的**目标设备身份**字段名（`ops[].deviceId`，必填）。
///
/// 取走按**身份**寻址（与 `removeClip` / `removeScene` / `removeRoutingNode` 同一
/// 纪律），不按下标：模型在**同一条音轨上**拒绝重复的设备身份
/// （`Op::validate` 的 `DuplicateEntityId`），因此 `(trackId, deviceId)` 唯一确定一格。
/// 下标会随同批的其它插入 / 取走而漂移，身份不会。
pub const DEVICE_ID_FIELD: &str = "deviceId";

/// 插入设备形态允许出现的**全部**键（判别键 + 寻址键 + 载荷键 + 可选的槽位键）。
pub const INSERT_DEVICE_FIELDS: [&str; 4] = [
    "kind",
    DEVICE_TRACK_FIELD,
    DEVICE_PAYLOAD_FIELD,
    DEVICE_SLOT_FIELD,
];

/// 取走设备形态允许出现的**全部**键（判别键 + 寻址键）。
///
/// 被取走设备的旧状态**不在**这里：撤销载荷 `previous_device` 由 [`compile`] 从
/// **当前文档**读。多写一个键是**响亮失败**，不静默丢弃。
pub const REMOVE_DEVICE_FIELDS: [&str; 3] = ["kind", DEVICE_TRACK_FIELD, DEVICE_ID_FIELD];

/// `device` 载荷对象允许出现的**全部**键（camelCase，与本 crate 的工具体一致）。
///
/// ⚠ 这是**有自由度**的一处裁决，理由逐条写在 [`parse_device`]：`.yeban` 容器里的
/// `project.json` 用的是模型 serde 的 snake_case（`latency_samples`），而 MCP 工具面
/// 通篇 camelCase（`clipId` / `startTick` / `microTimingTicks`）。本形态取工具面的口径。
pub const DEVICE_FIELDS: [&str; 6] = [
    DEVICE_ID_FIELD,
    "name",
    "kind",
    "bypassed",
    "latencySamples",
    "params",
];

/// `device.params[]` 每个元素允许出现的**全部**键。
///
/// `unit` 是 [`ParameterValue::unit`] 的对应物，**可省**（`None` 是一等状态
/// "无量纲/未标注"，模型对 `Option<T>` 有明确豁免，见 `ADR-0001` D43 第 2 条）。
pub const DEVICE_PARAM_FIELDS: [&str; 3] = ["name", "value", "unit"];

/// [`DeviceKind`] 在 `device.kind` 位置接受的**全部**字面量。
///
/// 逐字等于模型 serde 的变体名（`DeviceKind` 没有 `rename_all`），因此同一个字面量
/// 在 `.yeban` 的 `project.json` 里与在工具体里是**同一个词**（`ADR-0001` D48）。
pub const DEVICE_KINDS: [&str; 4] = [
    "InternalInstrument",
    "InternalEffect",
    "ExternalInstrument",
    "ExternalEffect",
];

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

/// 一个段落的**逐键覆盖**（`None` = 调用方没提这个键 ⇒ 保留现值）。
///
/// 三态刻意不折叠（与 [`ScenePatch`] **逐条同口径**）：`color` 的 `Some(None)`
/// （明写 `null` = 清掉色标）与 `None`（缺省 = 保留现值）是两件不同的事。
/// `start_tick` / `end_tick` 同形 —— 它们**各自**可分，因此一次合并可以只移动起点。
///
/// ⚠ 区间约束（`end_tick > start_tick`）**不在这里**判：模型 `SectionV3::validate`
/// 是唯一事实源（`ZeroDuration`），它由提案模拟那一步报。本层只负责把四个属性
/// **如实合并**到基础上。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SectionPatch {
    /// 段落名（`None` = 保留现值）。
    pub name: Option<String>,
    /// 起始 tick（外层 `Option` = 调用方提没提；内层恒为 `Some` ⇒ 非空字段）。
    pub start_tick: Option<u64>,
    /// 结束 tick（同上）。
    pub end_tick: Option<u64>,
    /// 界面色标（外层 `Option` = 调用方提没提；内层 = 覆盖值还是"清掉"）。
    pub color: Option<Option<String>>,
}

impl SectionPatch {
    /// 该覆盖是否一个键都没提（`true` ⇒ 结果必然等于基础，是一次无操作）。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.start_tick.is_none()
            && self.end_tick.is_none()
            && self.color.is_none()
    }

    /// 把覆盖施加到一条**基础**段落上（基础 = 文档现值，或新建时的空形状）。
    ///
    /// 返回替换后的整条段落。`id` **不在**覆盖范围内（它是寻址，由
    /// [`NoteOp::SetSection`] 的 `section_id` 给出）：这里原样保留，因此本形态不可能
    /// 顺手改掉一个段落的身份。
    #[must_use]
    pub fn apply_to(&self, base: &SectionV3) -> SectionV3 {
        let mut section = base.clone();
        if let Some(name) = &self.name {
            section.name.clone_from(name);
        }
        if let Some(start) = self.start_tick {
            section.start_tick = start;
        }
        if let Some(end) = self.end_tick {
            section.end_tick = end;
        }
        if let Some(color) = &self.color {
            section.color.clone_from(color);
        }
        section
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
    /// 设置音符的**时间位置**（起点与时值），不改任何其他字段。
    ///
    /// 载荷只有**新值**：两个旧值由 [`compile`] 从当前文档读 —— 与
    /// [`Self::Delete`] / [`Self::Velocity`] 的撤销载荷同一条纪律（⛔ 不信调用方）。
    /// 模型层 [`Op::SetNoteTiming`] 的前置条件要求旧值**逐字段**对上，因此同一批里
    /// 对同一音符连写两次 `setNoteTiming` 会在模拟阶段被 `CONFLICT` 拒绝
    /// （与连写两次 `velocity` 同口径）。
    SetTiming {
        /// 音符身份。
        note_id: EntityId,
        /// 新起点（tick）。
        start_tick: u64,
        /// 新时值（tick，必须 ≥ 1；`0` 由模型拒绝）。
        duration_ticks: u64,
    },
    /// 设置音符的**三个表现力字段**（[`Op::SetNoteExpression`]）：触发概率 / 连击 /
    /// 微时序。
    ///
    /// ⭐ **本形态补上了什么**：三个字段此前**只在 `add` 的 `note` 对象里**能给一次，
    /// 之后**没有任何通路** ⇒ ⭐ **音符建得出、表情改不了**。改一个概率只能删 + 加，
    /// 而那会换掉音符身份（`MidiNote::triggers` 的确定性哈希只用 `id`）。
    ///
    /// ⚠ **整体替换而不是逐字段**（照 [`Self::SetTransportConfig`]）：三个值**全部必填**，
    /// `null` = 清空 = 模型的 `None`；缺键会被 [`reject_set_note_expression_fields`] 响亮
    /// 拒绝（⛔ 不把"没陈述"静默当成"清空"）。
    ///
    /// 载荷只有**新值**：三个旧值由 [`compile`] 从**当前文档**读（⛔ 不信调用方声明）。
    /// 值域照抄模型（`0.0..=1.0` 且有限 / `1..=16` / `-240..=240`）—— 越界在解析期就报
    /// `OUT_OF_RANGE`（与 `add.note.*` 同一份读数），模型侧再判一次同一区间。
    SetNoteExpression {
        /// 音符身份。
        note_id: EntityId,
        /// 新触发概率（`None` = 清空 = 必然触发）。
        new_probability: Option<f32>,
        /// 新连击次数（`None` = 清空 = 等价于 1）。
        new_ratchet: Option<u8>,
        /// 新微时值偏移（`None` = 清空 = 无偏移）。
        new_micro_timing_ticks: Option<i16>,
    },
    /// 设置音符的**音高形状**（[`Op::SetNotePitchShape`]）：弯音曲线 ＋ 滑音。
    ///
    /// ⭐ **本形态补上了什么**：`pitchBendCurve` / `slide` 此前**只在 `add` 的 `note`
    /// 对象里**能给一次，之后**没有任何通路** ⇒ 改形状只能删 + 加，而那会**换掉音符
    /// 身份**（`MidiNote::triggers` 的确定性哈希只用 `id`）。
    ///
    /// ⚠ **整体替换而不是逐字段**（照 [`Self::SetNoteExpression`] / [`Self::SetTransportConfig`]）：
    /// 两个新值**全部必填**；`slide` 的 `null` = 清空 = 模型的 `None`；弯音的"无"**只有**
    /// 空数组一种拼法（模型的 `pitch_bend_curve` 是 `Vec`，⛔ 不是 `Option`）。
    ///
    /// ⚠ **载荷只带新值**：两个旧值由 [`compile`] 从**当前文档**读（与
    /// [`Self::SetNoteExpression`] 同一条纪律），⛔ 不采信调用方声明。值域照抄模型
    /// （`slide.targetPitch` 0..=127、`slide.durationTicks` ≠ 0、弯音 tick 严格升序
    /// 先于 `cents` ±1200），模型侧再判一次同一份实现。
    SetNotePitchShape {
        /// 音符身份。
        note_id: EntityId,
        /// 新弯音曲线（空 `Vec` = 清空 = 无弯音）。
        new_bend: Vec<(u64, i16)>,
        /// 新滑音（`None` = 清空 = 无滑音）。
        new_slide: Option<SlideConfig>,
    },
    /// 写**静态混音值**（[`Op::SetParam`]）：音轨音量 / 声相，或设备参数。
    ///
    /// 这是本枚举里第一个**音轨级**（而非音符级）的形态：它不读、不写任何音符，
    /// 目标音轨由调用方的 `trackId` 给出（见 [`compile`]）。值域判定在模型层
    /// （[`Op::SetParam`] 的 `apply` → `validate_param_value`）；设备参数的**寻址**
    /// 与**名字白名单**在 [`compile`] 里交给 `yeban_render::inserts::resolve_device_param`。
    SetParam {
        /// 目标（"有静态值"的三个变体）。
        lane: StaticLane,
        /// 目标值；`TrackVolume` 单位 dB（有限值），`TrackPan` ∈ -1.0..=1.0，
        /// `DeviceParam` 由该设备参数的既有值域决定（模型 `SetParam` 不另设范围）。
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
    /// 设置**走带配置**（[`Op::SetTransportConfig`]）。
    ///
    /// 与 [`Self::SetTrackFlag`] 同族（不读不写音符），但载荷是**三个字段的整体**
    /// —— 模型那一条 `Op` 也是整体替换（理由见 `Op::SetTransportConfig` 的文档）。
    /// 撤销载荷 `old_config` 由 [`Self::read_transport`] 从**当前文档**读，
    /// 不是调用方声明。
    ///
    /// ⚠ **目标不是音轨**：本形态不查顶层 `trackId`（与 `removeTrack` 同类，
    /// 见 [`Self::is_track_entity_level`]）。
    /// 设置**速度**（[`Op::SetTempo`]，顶层 `bpm`）。
    ///
    /// 与 [`Self::SetTransportConfig`] / [`Self::SetAuthor`] 同族（**顶层文档字段**、
    /// 不读不写音符），载荷是一个 `f64`。
    ///
    /// ⚠ 模型的两条前置条件：① `new_bpm` **有限且在** `MIN_BPM..=MAX_BPM`
    /// （`20.0..=999.0`，否则 `BpmOutOfRange`）；② 文档现值**逐位**等于 `old_bpm`
    /// （否则 `OpStateMismatch`）。撤销载荷 `old_bpm` 由 [`compile`] 从**当前文档**读
    /// （`project.bpm`），⛔ 不信调用方声明。
    SetTempo {
        /// 修改前的速度（BPM，来自当前文档）。
        old_bpm: f64,
        /// 修改后的速度（BPM）。
        new_bpm: f64,
    },
    /// 设置**作者署名**（[`Op::SetAuthor`]，顶层 `author`）。
    ///
    /// 与 [`Self::SetTransportConfig`] / [`Self::SetTempo`] 同族（顶层文档字段），
    /// 但载荷是 **`String`**（⛔ 不是 `Copy`）。
    ///
    /// ⚠ 模型**只有**一条前置条件：文档现值必须**逐字节**等于 `old_author`
    /// （否则 `OpStateMismatch`）⇒ ⛔ **没有**空串/长度校验 ⇒ 本层也**不发明**它们
    /// （见 [`SET_AUTHOR_AUTHOR_FIELD`] 的文档）。撤销载荷 `old_author` 由 [`compile`]
    /// 从**当前文档**读（`project.author`）。
    SetAuthor {
        /// 修改前的署名（来自当前文档）。
        old_author: String,
        /// 修改后的署名。
        new_author: String,
    },
    /// 设置**作品标题**（[`Op::SetTitle`]，顶层 `title`）。
    ///
    /// 与 [`Self::SetTransportConfig`] / [`Self::SetTempo`] / [`Self::SetAuthor`] 同族
    /// （顶层文档字段），载荷是 **`String`**（⛔ 不是 `Copy`）。
    ///
    /// ⭐ **本形态补上了什么**：`title` 是**必需**的顶层字段，而工具面此前**没有任何
    /// 通路能改它** —— 它只在 `yeban_create_project` 里被写一次
    /// （`domain/project_create.rs:370`）⇒ ⭐ **工程建得出、标题改不了**。
    ///
    /// ⚠ 模型**只有**一条前置条件：文档现值必须**逐字节**等于 `old_title`
    /// （否则 `OpStateMismatch`）⇒ ⛔ **没有**空串/长度校验 ⇒ 本层也**不发明**它们
    /// （见 [`SET_TITLE_TITLE_FIELD`] 的文档）。撤销载荷 `old_title` 由 [`compile`]
    /// 从**当前文档**读（`project.title`）。
    SetTitle {
        /// 修改前的标题（来自当前文档）。
        old_title: String,
        /// 修改后的标题。
        new_title: String,
    },
    /// 设置**音轨名字**（[`Op::SetTrackName`]，寻址到一条音轨）。
    ///
    /// ⭐ **本形态补上了什么**：⭐ 在它之前工具面**只有 [建轨][Self::AddTrack] 时能给名字**
    /// ⇒ 改名只能"删轨 + 重建"，而那会**换掉 `track_id`** 并连带丢掉设备链 / 自动化 /
    /// 路由边。⚠ `name` 是**纯展示注记**（身份是 `track_id`）⇒ 改它不影响任何不变式。
    ///
    /// ⚠ 模型**只有**一条前置条件：文档现值必须**逐字节**等于 `old_name` ⇒ ⛔ 本层
    /// **不发明**空串/长度校验（见 [`SET_TRACK_NAME_NAME_FIELD`] 的文档）。
    /// 撤销载荷 `old_name` 由 [`compile`] 从**当前文档**读（`project.track(...)?.name`）。
    SetTrackName {
        /// 目标音轨。
        track_id: EntityId,
        /// 修改前的名字（来自当前文档）。
        old_name: String,
        /// 修改后的名字。
        new_name: String,
    },
    /// 设置**走带配置**（[`Op::SetTransportConfig`]）。
    ///
    /// 与 [`Self::SetTrackFlag`] 同族（不读不写音符），但载荷是**三个字段的整体**
    /// —— 模型那一条 `Op` 也是整体替换（理由见 `Op::SetTransportConfig` 的文档）。
    /// 撤销载荷 `old_config` 由 [`Self::read_transport`] 从**当前文档**读，
    /// 不是调用方声明。
    ///
    /// 设置**片段池条目的显示名**（[`Op::SetClipName`]，寻址到一个 `clip_id`）。
    ///
    /// ⭐ **本形态补上了什么**：⭐ 在它之前 `ClipPoolEntry::name` **没有**任何字段级通路
    /// ⇒ 改名只能"删 + 加"，⭐ 而模型 [`Op::RemoveClip`] 还要求**没有任何摆放引用它**
    /// ⇒ 调用方得先把该片段的每一处摆放拆掉、改完名再摆回去。
    ///
    /// ⚠ **为何安全**：⭐ 身份是 `ClipPoolEntry::id`（即 `clip_pool` 的**键**）⇒ 改 `name`
    /// **不动键**、⛔ 不让任何摆放悬空。撤销载荷 `old_name` 由 [`compile`] 从**当前文档**读
    /// （`project.clip_pool[clip_id].name`）⇒ ⛔ 不信调用方声明。⭐ 与 [`Self::SetTrackName`]
    /// 同口径：模型**只有**"陈旧载荷"一条校验 ⇒ ⛔ 本层不发明非空/长度校验。
    SetClipName {
        /// 目标片段身份。
        clip_id: EntityId,
        /// 修改前的显示名（来自当前文档）。
        old_name: String,
        /// 修改后的显示名。
        new_name: String,
    },
    /// 设置**音频片段增益**（[`Op::SetClipGain`]，寻址到一个 `clip_id`）。
    ///
    /// ⭐ **本形态补上了什么**：`ClipContent::Audio::gain_db` 此前**只在创建时**由
    /// `yeban_import_audio` 的 `gainDb` 写一次（`domain/import_audio.rs:396` / `:501`）
    /// ⇒ ⭐ **音频片段建得出、增益改不了**。改增益只能删片段 + 重新导入，而那会换掉
    /// `clip_id`、并让每一处摆放悬空。
    ///
    /// ⚠ **内容类型门是硬前置，不是礼貌**：`old_gain_db` 必须从**当前文档**读，而
    /// MIDI 片段**没有**这个字段 ⇒ 目标必须是 [`ClipContent::Audio`]，否则
    /// [`compile`] 在构造 `Op` **之前**就响亮拒绝（模型侧同一道门报
    /// `ClipContentKindMismatch`）。⛔ 不 panic、⛔ 不静默当作 0 dB。
    ///
    /// ⚠ **值域照抄现状**：模型只查有限性（`clip.audio.gain_db`），**没有** dB 范围
    /// ⇒ 本层也⛔ 不发明；非有限由 `read_number` 在**收窄到 `f32` 之后**拒
    /// （`nonFiniteValue`）。撤销载荷 `old_gain_db` 由 [`compile`] 从当前文档读。
    SetClipGain {
        /// 目标片段身份。
        clip_id: EntityId,
        /// 修改前的增益 (dB，来自当前文档)。
        old_gain_db: f32,
        /// 修改后的增益 (dB)。
        new_gain_db: f32,
    },
    /// 设置摆放**长度**（[`Op::SetClipPlacementDuration`]，寻址到一条音轨上的一个摆放）。
    ///
    /// ⭐ **本形态补上了什么**：⭐ 在它之前 `ClipPlacement::duration_ticks` **只在创建时**能给
    /// ⇒ ⭐ **创建之后改不了长度**（修剪/拉伸都做不了）；⭐ `move` 形态**只写 `startTick`**。
    /// ⚠ ⭐ **与 `loopConfig` 的关系（已核实）**：⭐ **校验层解耦**（`loopConfig` 用自己的
    /// 摆放相对 tick 区间、⛔ 无"必须与长度一致"的校验）⇒ ⭐ 本形态⛔ 不动 `loopConfig`；
    /// ⚠ 但渲染器 `#241` 之后**真的展开**循环：`N = ceil((duration_ticks - start) / (end - start))`
    /// ⇒ ⭐ 改长度会改变重复遍数（⭐ 那是**长度本来就该决定的量**；区间落到新末端之外时
    /// 按"不重复"播放 —— 见 `yeban_model::ClipPlacement::loop_repeat`）。
    /// ⚠ **值域**：⭐ **必须 ≥ 1**（⭐ `0` 由模型 `ZeroDuration` 响亮拒绝）。
    /// ⚠ **为何完全可逆**：⭐ 长度只是**播放窗口**；⭐ 音符住在片段内容里，⭐ 引擎的裁剪发生在
    /// **快照构建期**、⛔ 不是在应用时截断 ⇒ ⭐ **改短再改长⛔ 不丢音符**。
    /// 撤销载荷 `old_duration_ticks` 由 [`compile`] 从**当前文档**读 ⇒ ⛔ 不信调用方。
    /// ⛔ 不碰 `clipId` / `startTick` / `muted` / `loopConfig`。
    SetClipPlacementDuration {
        /// 目标音轨（摆放的容器）。
        track_id: EntityId,
        /// 目标摆放身份。
        placement_id: EntityId,
        /// 修改前的长度（tick，来自当前文档）。
        old_duration_ticks: u64,
        /// 修改后的长度（tick，必须 ≥ 1）。
        new_duration_ticks: u64,
    },
    /// 设置**音轨色标**（[`Op::SetTrackColor`]，寻址到一条音轨）。
    ///
    /// ⭐ **本形态补上了什么**：⭐ 在它之前 `TrackV3::color` **只能在建轨时给**
    /// （`addTrack` 的载荷）⇒ ⭐ 改色标只能删轨重建（⭐ 会换掉 `trackId`、丢掉设备/自动化/路由）。
    /// ⚠ ⭐ **`section` 与 `scene` 的色标 **已有通路****（⭐ 走它们各自的 patch）⇒
    /// ⭐ **只有音轨这一个是缺口**，⛔ 故本单只加这一个形态（⭐ ⛔ 不给另两个加重复通路）。
    ///
    /// ⚠ **`color` 必填**：⭐ `null` = **清除**（回主题默认色）、⭐ 非空字符串 = 设值
    /// ⇒ ⭐ **缺键 ⛔ 不等于清除**（⭐ 本形态是**单字段设值**、⛔ 不是合并式 patch）。
    /// ⚠ **为何安全**：色标是**纯展示注记**、身份是 `trackId`；⭐ `None` 是**一等状态**。
    /// 撤销载荷 `old_color` 由 [`compile`] 从**当前文档**读（`project.tracks[trackId].color`）
    /// ⇒ ⛔ 不信调用方。⛔ 不碰 `id` / `name` / `kind` / 集合 / 混音标量。
    SetTrackColor {
        /// 目标音轨。
        track_id: EntityId,
        /// 修改前的色标（来自当前文档；`None` = 主题默认色）。
        old_color: Option<String>,
        /// 修改后的色标（`None` = 清除 = 回到主题默认色）。
        new_color: Option<String>,
    },
    /// 给一条音轨**绑定采样器**（[`Op::SetTrackSampler`]，寻址到一条音轨）。
    ///
    /// ⭐ **本形态补上了什么**（`#221`，验收 `#220` 的缺陷 ①）：⭐ 在它之前
    /// `TrackV3::sampler` 在**工具面完全不可达** —— 模型里 `Op::SetTrackSampler` 早就
    /// 存在（`#76` / `crates/yeban-model/src/ops.rs`），`yeban-app` 侧也有构造先例
    /// （`bridge.rs` 的 `instrument_install_ops_on` ＋ `#210` 的 `--track-sampler`），
    /// 但 22 个工具 / 36 个 kind 里**一个 `sampler` 字段都没有** ⇒ 唯一能"设采样器"的
    /// 办法是删轨重建（会换掉 `EntityId`，连带丢掉设备链 / 自动化 / 路由边）。
    ///
    /// ⚠ **载荷是路径字符串 + 可空**：⭐ `null` = **解绑**（回"这条轨不用采样器"）；
    /// ⭐ **缺键 ⛔ 不等于解绑**（单字段设值、⛔ 不是合并式 patch）。
    /// ⚠ 本层⛔ **不发明路径语义**（不查存在性 / 不查扩展名 / 不规范化）—— 与
    /// [`SamplerBinding`] 的字段文档同口径；"这个路径读不到"由**渲染**响亮拒绝。
    /// 撤销载荷 `old_sampler` 由 [`compile`] 从**当前文档**读 ⇒ ⛔ 不信调用方。
    /// ⛔ 不碰 `id` / `name` / `kind` / `color` / `devices` / 混音标量 / 其它轨。
    SetTrackSampler {
        /// 目标音轨。
        track_id: EntityId,
        /// 修改前的绑定（来自当前文档；`None` = 该轨本来不用采样器）。
        old_sampler: Option<SamplerBinding>,
        /// 修改后的绑定（`None` = 解绑）。
        new_sampler: Option<SamplerBinding>,
    },
    /// 切换**摆放静音**（[`Op::SetClipPlacementMuted`]，寻址到一条音轨上的一个摆放）。
    ///
    /// ⭐ **本形态补上了什么**：⭐ 在它之前 `ClipPlacement::muted` **只在创建时**能给
    /// （`add` 形态的 `placement` 对象带它）⇒ ⭐ **创建之后改不了静音**；⭐ 而重建摆放会
    /// **换掉摆放身份**。⚠ `move` 形态**只写 `startTick`**。
    ///
    /// ⚠ **寻址用 `trackId` + `placementId`（两个身份）**：⭐ 摆放住在音轨里；⚠ 但本条
    /// **不必防下标移位** —— ⭐ `TrackV3::clips` 是 `BTreeMap`（键 = 摆放身份，稳定），
    /// ⭐ 与 `devices`（`Vec`）那处**不同**。
    /// ⚠ **为何安全（可逆）**：静音是布尔开关 ⇒ 切回去即复原；⚠ **但有行为重量** ——
    /// 四处下游按它**不发声/不导出**（引擎快照、`als`、`logic`、离线渲染）。
    /// ⛔ 不碰 `clipId` / `startTick` / `durationTicks` / `loopConfig`（⭐ 后两个是
    /// **有意不做**：改长度与"静音"不同类、`loopConfig` 是结构体要另议）。
    /// 撤销载荷 `old_muted` 由 [`compile`] 从**当前文档**读 ⇒ ⛔ 不信调用方。
    SetClipPlacementMuted {
        /// 目标音轨（摆放的容器）。
        track_id: EntityId,
        /// 目标摆放身份。
        placement_id: EntityId,
        /// 修改前的静音状态（来自当前文档）。
        old_muted: bool,
        /// 修改后的静音状态。
        new_muted: bool,
    },
    /// **整体替换**一条摆放的循环配置（[`Op::SetClipPlacementLoop`]，`#336`）。
    ///
    /// ⭐ **本形态补上了什么**：⭐ 在它之前 [`ClipPlacement::loop_config`] **只在创建时**
    /// 能给（`placement` 的 `add` 形态带 `loopEnabled` / `loopStartTick` /
    /// `loopEndTick`）⇒ ⭐ **创建之后改不了循环**；⭐ 改一个已有摆放的循环此前只能
    /// "`placement.kind: "remove"` + 新的 `add`"，⭐ 而那会**换掉摆放身份**。
    /// `setClipPlacementDuration` 只写长度（文档逐字登记了它**有意**不碰
    /// `loop_config`），本形态就是那处"另议"。
    ///
    /// ⚠ **载荷只带新值**（三项全部必填）：⛔ 没有 `oldLoopConfig` / `oldEnabled` /
    /// 任何旧值键 —— 旧值由 [`compile`] 从**当前文档**读（与 `setNoteExpression` /
    /// `setNotePitchShape` 同一条纪律，见 `#334`）。⭐ 整体替换、⛔ 不是逐字段合并。
    ///
    /// ⚠ **合法性口径镜像 `add`**：模型侧调用的是 [`Op::AddClipPlacement`] 所调用的
    /// **同一个** [`LoopConfig::validate`] ⇒ ⛔ 本层不另立"什么是一个合法循环"，
    /// 也⛔ 不规范化任何值。`enabled == false` / `end_tick >= duration_ticks` 等
    /// 退化形态原样写进文档，由唯一一份展开语义（`ClipPlacement::loop_repeat`）
    /// 按"恰好一遍"处理。
    /// ⛔ 不碰 `clipId` / `startTick` / `durationTicks` / `muted`。
    SetClipPlacementLoop {
        /// 目标音轨（摆放的容器）。
        track_id: EntityId,
        /// 目标摆放身份。
        placement_id: EntityId,
        /// 修改前的**整份**循环配置（来自当前文档）。
        old_loop_config: LoopConfig,
        /// 修改后的**整份**循环配置（整体替换）。
        new_loop_config: LoopConfig,
    },
    /// 切换**设备旁通**（[`Op::SetDeviceBypassed`]，寻址到一条音轨上的一台设备）。
    ///
    /// ⭐ **本形态补上了什么**：⭐ 在它之前 `DeviceDefinition::bypassed` **只能**在
    /// [`Self::InsertDevice`] 时由调用方给值 ⇒ ⭐ 今天**连一次"旁通这台设备"都做不出来**。
    ///
    /// ⚠ **为何安全（可逆）**：⭐ 旁通是**布尔开关** ⇒ 切回去即完全复原。
    /// ⚠ ⭐ **但它有真正的行为重量** —— 字段文档逐字写着「缺了它，一个被旁通的设备会被
    /// **静默接回信号链**」；下游确实读它（`yeban-engine` 的延迟汇总**跳过**旁通设备、
    /// `drums` **整体忽略**它们）。⭐ ⚠ **PDC 无需联动**：延迟表是从当前文档推导的**投影**
    /// ⇒ 改完文档后下次推导自然反映新旁通位。
    ///
    /// ⚠ **寻址用 `trackId` + `deviceId` 两个身份、⛔ 不用 `slotIndex`**（`devices` 是 `Vec`）。
    /// ⛔ 不碰 `id` / `name` / `kind` / `params` / `latencySamples`（延迟值照旧保留，
    /// 只是旁通期间不计入相位）。撤销载荷 `old_bypassed` 由 [`compile`] 从**当前文档**读
    /// （按 id 查设备）⇒ ⛔ 不信调用方。⚠ 设备缺失复用既有 `deviceNotFound` 形态。
    SetDeviceBypassed {
        /// 目标音轨（设备的容器）。
        track_id: EntityId,
        /// 目标设备身份。
        device_id: EntityId,
        /// 修改前的旁通状态（来自当前文档）。
        old_bypassed: bool,
        /// 修改后的旁通状态。
        new_bypassed: bool,
    },
    /// 设置**设备显示名**（[`Op::SetDeviceName`]，寻址到一条音轨上的一台设备）。
    ///
    /// ⭐ **本形态补上了什么**：⭐ 在它之前 `DeviceDefinition::name` **没有**任何字段级通路
    /// ⇒ 改名只能"删 + 加"，⭐ 而 `TrackV3::devices` 是 `Vec` ⇒ 既有 `insertDevice` /
    /// `removeDevice` 用 `slotIndex` 寻址、增删还会**移位下标**，⭐ 调用方得自己重算。
    ///
    /// ⚠ **寻址用 `trackId` + `deviceId`（两个身份）、⛔ 不用下标** —— 正是为了不受移位影响。
    /// ⚠ **为何安全**：身份是 `DeviceDefinition::id` ⇒ 改 `name` ⛔ 不动身份/下标。
    /// ⛔ 不碰 `kind` / `params` / `latencySamples`，⭐ **也⛔ 不碰 `bypassed`** ——
    /// ⚠ **那是有意不做的**（它是**行为开关**，与 `soloSafe` 同类，⛔ 不属于"显示名"这一票）。
    /// 撤销载荷 `old_name` 由 [`compile`] 从**当前文档**读（按 id 查设备）⇒ ⛔ 不信调用方。
    /// ⚠ 设备缺失复用既有 `deviceNotFound` 形态（与 [`Self::RemoveDevice`] 同一条）。
    SetDeviceName {
        /// 目标音轨（设备的容器）。
        track_id: EntityId,
        /// 目标设备身份。
        device_id: EntityId,
        /// 修改前的显示名（来自当前文档）。
        old_name: String,
        /// 修改后的显示名。
        new_name: String,
    },
    /// ⚠ **目标不是音轨**：本形态不查顶层 `trackId`（与 `removeTrack` 同类，
    /// 见 [`Self::is_track_entity_level`]）。
    SetTransportConfig {
        /// 目标态的三个字段。
        new_config: TransportConfig,
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
    /// 新增**一条音轨**（[`Op::AddTrack`]，即把这条音轨加进 `project.tracks`）。
    ///
    /// ⚠ **本形态是"造音轨"的那一半**：此前工具面**删得到**音轨
    /// （[`Self::RemoveTrack`]）却 ⛔ **加不到** —— 加音轨只在 `yeban_create_project`
    /// 的建批与 `yeban_propose_section` 的配器骨架里发生 ⇒ 一个**已存在的**工程
    /// 无法被**逐步**扩展。本形态把那条通路补成"同一件事两种粒度"。
    ///
    /// ⚠⭐ **四个集合（`devices`/`macros`/`automation_lanes`/`clips`）与
    /// `folder_id`/`color` 以及五个混音标量都**不在**载荷里** —— 那是**决定**，
    /// ⛔ 不是疏漏：`ADR-0001 D43` 的必需性立法理由是"**缺键 = 静默清空既有内容**"，
    /// 而**创建**时没有内容可清 ⇒ 本层用**空形状**
    /// （`TrackV3 { id, name, kind, ..TrackV3::default() }`），与
    /// `project_create.rs:381` 和 `setScene` 的 create 分支**同一口径**。判据
    /// `add_track_lands_an_empty_track_and_leaves_the_mix_alone` 钉住这一点。
    ///
    /// 模型的前置条件是"音轨自校验通过 + `id` 不重复"：`compile` 提前报"这个身份
    /// 已经在 `tracks` 里"（`projectAlreadyExists`），与 [`Self::RemoveTrack`] 提前报
    /// `trackNotFound` 同一条口径。
    /// ⚠ 模型 `apply` 只 `insert_track` —— 它**不**自动把身份加进
    /// `routing_graph.nodes`、也**不**建边（那是 [`Self::AddRoutingNode`] 的事）。
    AddTrack {
        /// 要新建的音轨（四个集合与两个可选字段已是空形状）。
        track: TrackV3,
    },
    /// 新建**一个路由节点**（[`Op::AddRoutingNode`]，即把这个身份加进
    /// `routing_graph.nodes`）。
    ///
    /// ⚠ **本形态是造节点的那一半**：此前工具面**造得出**节点的唯一通路是
    /// `yeban_propose_section` 的建批（它构造 `Op::AddRoutingNode`），而
    /// `yeban_edit_notes` 的目录里**没有**对应的 `kind` ⇒ 一张**空白**工程
    /// （还没有任何路由节点）无法被**逐步**搭起来。本形态把那条通路补成"同一件事
    /// 两种粒度"，⛔ 不去掉 propose 的建批。
    ///
    /// 与 [`Self::RemoveRoutingNode`] **严格互为逆**（`Op::invert` 把两者互换），
    /// 因此载荷也**只有**节点身份：模型那两条 `Op` **都没有撤销载荷**
    /// ⇒ 本形态既不读文档现值，也不接受调用方送来的旧状态
    /// （[`reject_add_routing_node_fields`] 只认 `kind` 与 `nodeId`）。
    ///
    /// ⚠ **本形态⛔ 不要求这个身份是一条真实音轨** —— 那是**决定**不是疏忽：
    /// 模型只做**数据层**的校验（[`RoutingGraph::validate`] 查节点重复、以及每条边的
    /// 两端**都在** `nodes` 里），它⛔ **不**要求节点能对应到 `project.tracks` 的条目。
    /// 同一族的先例: `addTrack` 同样只把一条新音轨加进 `tracks`、**不**自动把它的身份
    /// 加进 `nodes`（两份集合各有自己的 `Op`）。本层照抄模型口径 ⇒ 一个**拼错**的
    /// `nodeId` 会被接受并落地，且引擎侧看不到它所属的音轨 —— 这条边界**如实登记在
    /// 本变体的文档里**，⛔ 不在此处发明一条模型没有的前置条件。
    ///
    /// 模型的前置条件是"这个身份**不在** `nodes` 里"（`DuplicateEntityId`）：
    /// [`compile`] 提前报它（`routingNodeAlreadyExists`），与
    /// [`Self::RemoveRoutingNode`] 提前报 `routingNodeNotFound` 同一条口径。
    AddRoutingNode {
        /// 路由节点身份（本形态自带寻址）。
        node_id: EntityId,
    },
    /// 建立**一条路由边**（[`Op::ConnectRouting`]，即把这条边加进
    /// `routing_graph.edges`）。
    ///
    /// ⚠⭐ **本形态是路由图可用性的最后一环**：在它之前工具面**建得起节点、连不起边**
    /// —— `Op::ConnectRouting` 在 MCP 侧只在 `yeban_create_project` 的建批与
    /// `yeban_propose_section` 的声部连接里被构造 ⇒ 一个**已存在的**工程无法自己接线。
    /// 它与 [`Self::DisconnectRouting`] **严格互为逆**。
    ///
    /// ⚠ **平铺形状**（⛔ 不嵌套 `edge: {...}`）：与 [`Self::AddRoutingNode`] /
    /// [`Self::DisconnectRouting`] 同一条形状（同族形态都平铺）。
    ///
    /// 模型那条 `Op` **自带全部载荷**（撤销靠 `invert` 换成 `DisconnectRouting`，
    /// 而后者也自带 `previous_edge`）⇒ 本形态**既不读文档现值、也不接受调用方声明的
    /// 旧状态**（与 `removeSection` 那类"撤销载荷从当前文档读"的形态**不同**）。
    ///
    /// 模型的三条前置条件：① `edge.validate()`（增益必须有限）；② **两端都必须在
    /// `routing_graph.nodes` 里**（`RoutingNodeNotFound`）；③ `edge.id` **不在**
    /// `routing_graph.edges` 里（`DuplicateEntityId`）。
    /// ⚠ 模型**没有**"节点类型与 `edgeKind` 必须匹配"这条约束（例如 `TrackToBus`
    /// 的两端可以是任意身份）⇒ 本层⛔ **不发明**它，只提前报 ②③
    /// （`routingNodeNotFound` / `routingEdgeAlreadyExists`），与
    /// [`Self::RemoveRoutingNode`] 提前报 `routingNodeNotFound` 同一条纪律。
    ConnectRouting {
        /// 边身份（本形态自带寻址）。
        edge_id: EntityId,
        /// 源节点（必须已在 `routing_graph.nodes` 里）。
        source_node: EntityId,
        /// 目标节点（必须已在 `routing_graph.nodes` 里）。
        destination_node: EntityId,
        /// 路由类型。
        kind: RoutingKind,
        /// 该边的增益（dB）；`None` = 单位增益。
        gain_db: Option<f32>,
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
    /// **写入一个曲式段落**（[`Op::SetSection`]，`project.sections`）：新建
    /// （`create` 为真）或更新（合并）。
    ///
    /// ⚠⭐ **本形态补的是"段落建得出、删得掉、⛔ 改不了"** —— 在它之前
    /// [`Op::SetSection`] 在 MCP 侧的**唯一**构造点是 `yeban_propose_section` 的
    /// 内部建批（`section_build.rs`），而工具面只有 [`Self::RemoveSection`]
    /// ⇒ 一个**已存在的**工程无法改段落名 / 区间 / 色标，也无法**逐个**加段落。
    ///
    /// 与 [`Self::SetScene`] **逐条同口径**：`create` 显式区分新建与更新
    /// （⛔ 不靠"文档里没有这个身份就当作新建"）；`old_section` 由 [`compile`] 从
    /// **当前文档**读（新建时 `None`），⛔ 不信调用方声明；载荷是一个
    /// [`SectionPatch`]，四个属性键**各自可选**、按**合并**语义施加。
    ///
    /// ⚠ 区间约束（`end_tick > start_tick`）**不在这里**复制：模型
    /// `SectionV3::validate` 是唯一事实源（`ZeroDuration`）。
    SetSection {
        /// 段落身份（本形态自带寻址；新建与更新都必须给出）。
        section_id: EntityId,
        /// `true` = 新建（身份必须**不在**文档里，否则 `CONFLICT`）；
        /// `false` = 更新（身份必须**在**文档里，否则 `ENTITY_NOT_FOUND`）。
        create: bool,
        /// 属性覆盖（合并语义）。
        patch: SectionPatch,
    },
    /// **改一条资产索引项**（[`Op::SetAsset`]，**更新**语义；`old_asset` 为 `Some`）。
    ///
    /// ⚠⭐ 本形态补的是"**建得出、改不了**"：在它之前 [`Op::SetAsset`] 在 MCP 侧的
    /// 唯一构造点写死 `old_asset: None`（创建），而"同哈希换个 `license` 重导"被
    /// `yeban_import_audio` 当**冲突**拒（那是**有意**的 —— ⛔ 不许靠重导**静默**改写
    /// 许可留痕）。本形态给出一条**正门**：用户**明说**要改。
    ///
    /// ⚠ **可改字段有明文**（模型 `Op::SetAsset` 的文档逐字："之后同哈希再声明就是
    /// **更新（改许可/原路径）**"）：只有 `license` 与 `original_path`。
    /// `hash`（内容身份）／`byte_len`（内容事实）／`media_kind`（按内容判定）
    /// **不可改** —— 因此本形态⛔ 不可能把一条索引项改成"另一个资产"。
    ///
    /// ⚠ 自带寻址（`ops[].assetHash`，与 [`Self::RemoveAsset`] 同因）。
    /// 撤销载荷 `old_asset` 由 [`compile`] 从**当前文档**读（整条），⛔ 不信调用方声明。
    /// 至少给出一个可改字段（否则是一次无操作 ⇒ 响亮拒绝）。
    SetAsset {
        /// 资产身份（内容寻址的 CAS 键，本形态自带寻址）。
        asset_hash: AssetHash,
        /// 逐键覆盖（只含可改字段）。
        patch: AssetPatch,
    },
    /// **取走一条资产索引项**（[`Op::RemoveAsset`]，`project.assets` 里那一格）。
    ///
    /// ⚠⭐ **资产唯一的显式删除通路**。在它之前 [`Op::RemoveAsset`] 只由模型的
    /// `structural_inverse` 产出（`SetAsset { old_asset: None }` 的逆）⇒ 用户**只能靠
    /// 撤销那次导入**取走索引项，没有一个"删掉这个资产"的形态 —— 与 [`Self::RemoveScene`]
    /// 补齐之前"建得出、删不掉"的形状同族。
    ///
    /// ⚠ **自带寻址**（`ops[].assetHash`）：`yeban_edit_notes` **没有**顶层 `assetHash`
    /// 实参，因此本形态⛔ 不能像 [`Self::RemoveTrack`] 那样借顶层实参（那个形态的
    /// `trackId` 是工具签名的一部分）。
    ///
    /// ⚠ 撤销载荷 `previous_asset` 由 [`compile`] 从**当前文档**读（整条索引项，
    /// 逐字段比较），⛔ 不信调用方声明。
    ///
    /// ⚠ **被引用则拒**：`project.assets` 是 `clip_pool` 里 `ClipContent::Audio` 的
    /// **许可留痕** ⇒ 还有片段引用该哈希时取走它，会留下**指向不存在资产的片段**
    /// （`YebanProjectV1::validate` **不查**这条 ⇒ 模型⛔ 不会替我们拦）⇒ 本层自己拒
    /// （`assetInUse`，与 `removeClip` 报模型 `ClipInUse` 同一条纪律）。
    RemoveAsset {
        /// 资产身份（内容寻址的 CAS 键，本形态自带寻址）。
        asset_hash: AssetHash,
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
    /// 取走**一条音轨**（[`Op::RemoveTrack`]，即把这个身份从 `project.tracks` 取走）。
    ///
    /// 这是本枚举里唯一的**音轨实体级**形态：它取走的是音轨**本身**（连带它自己的
    /// `clips` 摆放与 `automation_lanes`），而不是音轨上的某个属性
    /// —— 属性由 [`Self::SetParam`] / [`Self::SetTrackFlag`] / [`Self::SetLane`] 写
    /// （见 [`Self::is_track_entity_level`] 与 [`Self::is_note_level`] 的分工）。
    ///
    /// ⚠ 写侧在它之前**一处都没有**：`Op::AddTrack` 在工具面上由
    /// `yeban_open_project` 的 `create: true` 与 `yeban_propose_section` 的骨架**真的**
    /// 构造（`domain/project_create.rs` / `domain/section_build.rs`），而
    /// `Op::RemoveTrack` 在 `crates/yeban-mcp/src` 里的构造点实测是 0 ⇒ 工具面
    /// **建得出**音轨、**取不走**音轨（唯一出路是 `yeban_undo`，而那会连带回退
    /// 同一提交里的其它一切）。读侧却一直在报音轨身份
    /// （`yeban_query_project` 的 `tracks`）。
    ///
    /// 目标由**自带的** `trackId` 给出（[`REMOVE_TRACK_FIELD`]），与顶层 `trackId` /
    /// `clipId` 无关 —— 后者是"目标片段所在的音轨"（[`compile`] 的入口用它查音轨与
    /// 片段池）。与三个路由级形态、段落形态、场景形态同一纪律。
    ///
    /// 载荷是**空**的：只有寻址。撤销载荷 `previous_track` 由 [`compile`] 从
    /// **当前文档**读（模型的前置条件要求它逐字段等于文档现值，因此本层不采信
    /// 调用方声明的旧状态），[`reject_remove_track_fields`] 只认 `kind` 与 `trackId`。
    ///
    /// 模型的两条前置条件：主总线**不能**被取走（`Op::validate` 报
    /// `OpStateMismatch`）—— 那一条由 [`compile`] 提前报（`masterBusTrackCannotBeRemoved`，
    /// 与 `removeRoutingNode` 的主总线拒绝同一口径）；音轨必须**存在**
    /// （`TrackNotFound`）—— 同样提前报（`trackNotFound`）。
    RemoveTrack {
        /// 音轨身份（本形态自带寻址）。
        track_id: EntityId,
    },
    /// 在一条音轨的设备链里**插入**一台设备（[`Op::InsertDevice`]）。
    ///
    /// 这是本枚举里第一个**设备级**形态。写侧在它之前**一处都没有**：`Op::InsertDevice`
    /// 与 `Op::RemoveDevice` 在 `crates/yeban-mcp/src` 里的**构造点**实测是 **0**
    /// （两口径普查见本文件的模块头表）⇒ 工具面能**看见**设备链
    /// （`yeban_query_engine_state` 的 `tracks[].devices[]`，含 `slotIndex` /
    /// `deviceId` / `latencySamples`）却**造不出、也取不走**任何一台设备。
    ///
    /// 为什么这不是"可有可无的补充"：`DeviceDefinition::latency_samples` 是 PDC 的
    /// **唯一**延迟来源（`ARCH-PDC-001`，见 `domain/render.rs` 的 `track_latencies`），
    /// 而一台延迟非零的外部设备只能在文档里表示、不能在工具面里表示 ⇒ 外部设备的
    /// 相位对齐在 MCP 会话里**无法被表达**。
    ///
    /// 目标由**自带的** [`DEVICE_TRACK_FIELD`] 给出，与顶层 `trackId` / `clipId` 无关
    /// （后者是"目标片段所在的音轨"）。与 [`Self::RemoveTrack`] 同一纪律：本形态
    /// 一个音符都不读，也不要求片段是 MIDI。
    ///
    /// 载荷是 [`DeviceDefinition`]，字段与 [`DEVICE_FIELDS`] 逐条对应；`slot_index`
    /// 是 [`Option`]：`None` = **追加到链尾**（模型允许 `slot_index == devices.len()`），
    /// `Some(n)` 的 `n > devices.len()` 是响亮失败（不夹紧）。
    InsertDevice {
        /// 目标音轨身份（本形态自带寻址）。
        track_id: EntityId,
        /// 插槽下标；`None` = 追加到链尾。
        slot_index: Option<usize>,
        /// 被插入的完整设备定义。
        device: DeviceDefinition,
    },
    /// 从一条音轨的设备链里**取走**一台设备（[`Op::RemoveDevice`]）。
    ///
    /// 与 [`Self::InsertDevice`] 同族。按**身份**寻址（[`DEVICE_ID_FIELD`]）而不是按
    /// 下标：模型在**同一条音轨上**拒绝重复的设备身份，因此 `(trackId, deviceId)`
    /// 唯一确定一格；下标会随同批的其它插入 / 取走而漂移，身份不会。
    ///
    /// 撤销载荷 `previous_device` 由 [`compile`] 从**当前文档**读（模型的前置条件
    /// 要求它逐字段等于文档现值，因此本层不采信调用方声明的旧状态）。
    RemoveDevice {
        /// 目标音轨身份（本形态自带寻址）。
        track_id: EntityId,
        /// 被取走的设备身份。
        device_id: EntityId,
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
            Self::SetTiming { .. } => SET_NOTE_TIMING_KIND,
            Self::SetNoteExpression { .. } => SET_NOTE_EXPRESSION_KIND,
            Self::SetNotePitchShape { .. } => SET_NOTE_PITCH_SHAPE_KIND,
            Self::SetParam { .. } => SET_PARAM_KIND,
            Self::SetTrackFlag { flag, .. } => flag.kind_name(),
            Self::SetTransportConfig { .. } => SET_TRANSPORT_CONFIG_KIND,
            Self::SetTempo { .. } => SET_TEMPO_KIND,
            Self::SetAuthor { .. } => SET_AUTHOR_KIND,
            Self::SetTitle { .. } => SET_TITLE_KIND,
            Self::SetTrackName { .. } => SET_TRACK_NAME_KIND,
            Self::SetClipName { .. } => SET_CLIP_NAME_KIND,
            Self::SetClipGain { .. } => SET_CLIP_GAIN_KIND,
            Self::SetDeviceName { .. } => SET_DEVICE_NAME_KIND,
            Self::SetDeviceBypassed { .. } => SET_DEVICE_BYPASSED_KIND,
            Self::SetClipPlacementMuted { .. } => SET_CLIP_PLACEMENT_MUTED_KIND,
            Self::SetClipPlacementLoop { .. } => SET_CLIP_PLACEMENT_LOOP_KIND,
            Self::SetTrackColor { .. } => SET_TRACK_COLOR_KIND,
            Self::SetTrackSampler { .. } => SET_TRACK_SAMPLER_KIND,
            Self::SetClipPlacementDuration { .. } => SET_CLIP_PLACEMENT_DURATION_KIND,
            Self::SetLane { .. } => SET_AUTOMATION_LANE_KIND,
            Self::RemovePoint { .. } => REMOVE_AUTOMATION_POINT_KIND,
            Self::RemoveClip => REMOVE_CLIP_KIND,
            Self::SetRoutingGain { .. } => SET_ROUTING_GAIN_KIND,
            Self::ConnectRouting { .. } => CONNECT_ROUTING_KIND,
            Self::DisconnectRouting { .. } => DISCONNECT_ROUTING_KIND,
            Self::AddTrack { .. } => ADD_TRACK_KIND,
            Self::AddRoutingNode { .. } => ADD_ROUTING_NODE_KIND,
            Self::RemoveRoutingNode { .. } => REMOVE_ROUTING_NODE_KIND,
            Self::RemoveSection { .. } => REMOVE_SECTION_KIND,
            Self::SetAsset { .. } => SET_ASSET_KIND,
            Self::RemoveAsset { .. } => REMOVE_ASSET_KIND,
            Self::RemoveScene { .. } => REMOVE_SCENE_KIND,
            Self::SetSection { .. } => SET_SECTION_KIND,
            Self::SetScene { .. } => SET_SCENE_KIND,
            Self::RemoveTrack { .. } => REMOVE_TRACK_KIND,
            Self::InsertDevice { .. } => INSERT_DEVICE_KIND,
            Self::RemoveDevice { .. } => REMOVE_DEVICE_KIND,
        }
    }

    /// 该形态是否**读/写音符**（即是否必须真的施加在一条带音符的 MIDI 片段上）。
    ///
    /// ⭐ `#179`：本谓词收窄到**音符号形态**（`add` / `delete` / `move` /
    /// `velocity` / `setNoteTiming`）—— 它们恰好是 [`ENVELOPE_CLIP_KINDS`] 去掉
    /// 池级 `removeClip` 之后的五个（与 [`NOTE_LEVEL_KINDS`] 同一份真相）。
    /// 其余 **32** 个形态一个音符
    /// 都不读：音轨属性级（`setParam` / `setTrackFlag` / `setLane` / `removePoint`）、
    /// 设备级（`insertDevice` / `removeDevice` / `setDeviceName` /
    /// `setDeviceBypassed`）、摆放属性级（`setClipPlacementMuted` /
    /// `setClipPlacementDuration`）、片段名 / 音轨名 / 音轨色（`setClipName` /
    /// `setTrackName` / `setTrackColor`）、池级（`removeClip`）、路由级、段落级、
    /// 场景级、资产级、文档级与音轨实体级。
    ///
    /// ⚠ `#177` 之后 [`compile`] 只在批里真有形态读顶层 `clipId` 时才查片段池，而
    /// "必须是 MIDI 片段"那条断言挂在 `entry` 上 ⇒ 那 32 个形态里**读 `clipId` 的
    /// 只有 `removeClip`**，其余 31 个此前虽然 `is_note_level() == true`（反选清单
    /// 漏掉了它们），断言也**不可达**。本票把谓词改成与**语义**一致：它们从来不该
    /// 被算作音符级 ⇒ `domain::plan_edit_notes` 的提案描述也不再把这些属性写入报成
    /// "音符编辑"（它们落到"音轨级编辑"桶里，与 `setParam` 同口径）。
    ///
    /// 旧行为逐字节不变：五个音符形态在 MIDI 片段上的编译产物仍是
    /// `Op::{AddNote, DeleteNote, MoveNote, ModifyNoteVelocity, SetNoteTiming}`。
    #[must_use]
    pub const fn is_note_level(&self) -> bool {
        matches!(
            self,
            Self::Add { .. }
                | Self::Delete { .. }
                | Self::Move { .. }
                | Self::Velocity { .. }
                | Self::SetTiming { .. }
                | Self::SetNoteExpression { .. }
                | Self::SetNotePitchShape { .. }
        )
    }

    /// 该形态取走的是**一整条音轨实体**（而不是音轨上的一个属性 / 音符 / 摆放）。
    ///
    /// 与**音轨属性级**形态（`setParam` / `setTrackMute` / `setTrackSolo` /
    /// `setAutomationLane` / `removeAutomationPoint`）**不是**同一件事：那五个写的是
    /// 一条音轨**属性**，目标由工具顶层的 `trackId` 给出；本形态取走音轨**本身**
    /// （连带它自己的摆放与泳道），目标由操作对象**自带**的 `trackId` 给出。
    /// 两者必须在提案标题里分得开 —— 一次"取走音轨"不能被报成"音轨级编辑"
    /// （见 `domain::plan_edit_notes`）。
    #[must_use]
    pub const fn is_track_entity_level(&self) -> bool {
        matches!(self, Self::RemoveTrack { .. })
    }

    /// 该形态改的是**一条音轨的设备链**（而不是音符 / 片段池 / 摆放 / 路由图 / 段落 /
    /// 场景 / 音轨属性）。
    ///
    /// 与 [`Self::is_routing_level`] 同因（`domain::plan_edit_notes` 的分类）：
    /// 设备链不是音轨属性，一次纯 `insertDevice` 的调用不能被报成"音轨级编辑"
    /// （那是不同的对象 —— 属性级形态的目标是工具顶层的 `trackId`，本形态的目标是
    /// 操作对象**自带**的 `trackId`）。
    #[must_use]
    pub const fn is_device_level(&self) -> bool {
        matches!(self, Self::InsertDevice { .. } | Self::RemoveDevice { .. })
    }

    /// 该形态的寻址**全部**在操作对象自己身上（不读工具顶层的 `trackId` / `clipId`）。
    ///
    /// 两种形态：音轨实体级（[`Self::RemoveTrack`]）与设备级
    /// （[`Self::InsertDevice`] / [`Self::RemoveDevice`]）。它们的寻址都在操作对象里
    /// 自带的 `trackId`（设备形态还有 `deviceId` / `slotIndex`），因此 [`compile`]
    /// 与 `domain::plan_edit_notes` **不必**先取顶层那两个身份，也不必要求片段是 MIDI。
    ///
    /// ⚠ 与 [`Self::is_self_addressed`] 的**区别**要写清：路由级 / 段落级 / 场景级形态
    /// 的寻址**也**在操作对象里（`edgeId` / `nodeId` / `sectionId` / `sceneId`），但它们
    /// **仍然**要求顶层 `trackId` / `clipId` 是合法身份 —— 那是本工具既有的入口口径，
    /// 本方法**不**替它们放松（放松会改变既有调用的可接受集合）。
    #[must_use]
    pub const fn is_self_addressed(&self) -> bool {
        self.is_track_entity_level() || self.is_device_level()
    }

    /// 批内**每一条**都是音轨实体级形态（[`Op::RemoveTrack`] 的写侧）。
    ///
    /// 这一问只用于[`compile`]（以及它上游的 `domain::plan_edit_notes`）决定
    /// "要不要先取顶层 `trackId` / `clipId`"：本形态一个音符都不读、也不碰片段池，
    /// 因此它的寻址是**自带**的 `trackId`。**空批不算**（`false`）—— 空 `ops` 本来
    /// 就被 [`parse_ops`] 挡掉，把它算成"全是"会让一条错误路径变成另一条。
    ///
    /// ⚠ 混批（取走音轨 + 别的形态）在 [`reject_remove_track_conflicts`] 里**响亮拒绝**，
    /// 因此本方法在合法输入上的取值与"批里有没有 `removeTrack`"是同一个答案。
    #[must_use]
    pub fn all_track_entity_level(ops: &[Self]) -> bool {
        !ops.is_empty() && ops.iter().all(Self::is_track_entity_level)
    }

    /// 批内**每一条**的寻址都在操作对象自己身上（[`Self::is_self_addressed`]）。
    ///
    /// 这一问只用于 [`compile`]（以及它上游的 `domain::plan_edit_notes`）决定
    /// "要不要先取顶层 `trackId` / `clipId`"。它对**两种**形态为真：取走音轨，以及
    /// 设备链形态（`insertDevice` / `removeDevice` —— 它们的寻址是自带的 `trackId`）。
    /// **空批不算**（`false`）—— 与 [`Self::all_track_entity_level`] 同一理由：
    /// 空 `ops` 本来就被 [`parse_ops`] 挡掉，把它算成"全是"会让一条错误路径变成另一条。
    ///
    /// ⚠ 混批（取走音轨 + 别的形态）在 [`reject_remove_track_conflicts`] 里**响亮拒绝**，
    /// 因此"全是音轨实体级"与"批里只有一条 `removeTrack`"是同一个答案；设备形态之间
    /// （以及 `insertDevice` + `removeDevice` 的混批）**允许**同批 —— 换一台设备本来
    /// 就是两条操作。
    #[must_use]
    pub fn all_self_addressed(ops: &[Self]) -> bool {
        !ops.is_empty() && ops.iter().all(Self::is_self_addressed)
    }

    /// 该形态**读**工具顶层的 `clipId`（`#177` 坑④：`clipId` 从无条件必填改成条件必需）。
    ///
    /// 只有五个音符形态与池级取走读它 —— 它们正是 [`compile`] 里 `clip_id` 的
    /// **全部**消费者（`Op::AddNote` / `DeleteNote` / `MoveNote` /
    /// `ModifyNoteVelocity` / `SetNoteTiming` / `RemoveClip` 的载荷，以及"片段池里
    /// 必须有它"那次查找）。
    /// ⚠ 与 [`Self::is_self_addressed`] **不是**同一问：音轨属性级 / 路由级 / 段落级 /
    /// 场景级 / 资产级 / 文档级形态的寻址也在操作对象里，但它们只读顶层 `trackId`
    /// （或什么都不读），**不读** `clipId`（`addTrack` 连 `trackId` 都不读）。
    ///
    /// 本谓词是"顶层 `clipId` 是否必需"的**唯一**真相：
    /// `domain::plan_edit_notes` 的缺参守卫、[`compile`] 的片段池查找、以及
    /// `crate::op_schema::ops_items` 广播给 `tools/list` 的逐 kind 标记都从它派生。
    #[must_use]
    pub const fn reads_envelope_clip(&self) -> bool {
        matches!(
            self,
            Self::Add { .. }
                | Self::Delete { .. }
                | Self::Move { .. }
                | Self::Velocity { .. }
                | Self::SetTiming { .. }
                | Self::SetNoteExpression { .. }
                | Self::SetNotePitchShape { .. }
                | Self::RemoveClip
        )
    }

    /// 批内**有没有**形态读顶层 `clipId`。
    #[must_use]
    pub fn envelope_clip_required(ops: &[Self]) -> bool {
        ops.iter().any(Self::reads_envelope_clip)
    }

    /// 本批里**读**顶层 `clipId` 的 kind（按出现顺序、去重）——
    /// 缺参报文用 `requiredByKinds` 点名"到底哪一条需要"。
    #[must_use]
    pub fn envelope_clip_kinds(ops: &[Self]) -> Vec<&'static str> {
        let mut kinds: Vec<&'static str> = Vec::new();
        for kind in ops
            .iter()
            .filter(|op| op.reads_envelope_clip())
            .map(Self::kind_name)
        {
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
        kinds
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
                | Self::ConnectRouting { .. }
                | Self::DisconnectRouting { .. }
                | Self::AddRoutingNode { .. }
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
        matches!(self, Self::RemoveSection { .. } | Self::SetSection { .. })
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

    /// 该形态改的是**工程资产索引**（而不是音符 / 音轨 / 泳道 / 片段池 / 路由图 /
    /// 曲式段落 / 场景）。
    ///
    /// 与 [`Self::is_scene_level`] / [`Self::is_section_level`] 同因
    /// （`domain::plan_edit_notes` 的分类）：资产不是上面任何一个 ⇒ 它需要自己的桶，
    /// ⛔ 否则一次纯 `removeAsset` 的调用会被报成别的层级的编辑。
    #[must_use]
    pub const fn is_asset_level(&self) -> bool {
        matches!(self, Self::RemoveAsset { .. } | Self::SetAsset { .. })
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
/// {"kind":"removeTrack","trackId":"<ULID>"}
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
/// 另外两个自动化目标名（`SendGain` / `Macro`）与未知名字都是**响亮失败**
/// （`INVALID_PARAMETER_RANGE`，`data.allowed` 给出全集）；`lane == "DeviceParam"`
/// 时 `slotIndex` / `paramIndex` 必填，越界与名字不在白名单同样响亮失败。
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
/// `removeTrack` 是**唯一的音轨实体级**形态（见 [`NoteOp::RemoveTrack`]，
/// [`NoteOp::is_track_entity_level`]）：它把 `trackId` 那条**音轨本身**从
/// `project.tracks` 取走（[`Op::RemoveTrack`]，连带它自己的 `clips` 摆放与
/// `automation_lanes`），用 [`REMOVE_TRACK_FIELD`] 寻址（与节点 / 边 / 段落 / 场景 /
/// 片段都是**不同的**实体），撤销载荷 `previous_track` 从当前文档读，
/// 对象里 [`REMOVE_TRACK_FIELDS`] 之外的键一律响亮拒绝。⚠ 它与五个**音轨属性级**
/// 形态不是一层：那五个写一条音轨的属性，目标由工具顶层的 `trackId` 给出；
/// 本形态取走音轨本身，目标由本对象自带的 `trackId` 给出。
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
///   [`REMOVE_SCENE_FIELDS`] 之外的键 / 取走音轨对象里有
///   [`REMOVE_TRACK_FIELDS`] 之外的键 / 写入场景的操作对象里有
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
            velocity: read_range(object, "velocity", 0, MIDI_VELOCITY_MAX)?,
        }),
        SET_NOTE_TIMING_KIND => {
            reject_set_note_timing_fields(object)?;
            Ok(NoteOp::SetTiming {
                note_id: read_id(object, SET_NOTE_TIMING_NOTE_FIELD)?,
                start_tick: read_required_u64(object, SET_NOTE_TIMING_START_FIELD)?,
                duration_ticks: read_required_u64(object, SET_NOTE_TIMING_DURATION_FIELD)?,
            })
        }
        SET_NOTE_EXPRESSION_KIND => {
            reject_set_note_expression_fields(object)?;
            Ok(NoteOp::SetNoteExpression {
                note_id: read_id(object, SET_NOTE_EXPRESSION_NOTE_FIELD)?,
                // ⚠ 三个值**必填、可为 null**（`null` = 清空 = 模型的 `None`）；
                // 缺键是响亮失败（整体替换没有"保留现值"语义）。
                new_probability: read_required_nullable(
                    object,
                    SET_NOTE_EXPRESSION_PROBABILITY_FIELD,
                    read_probability,
                )?,
                new_ratchet: read_required_nullable(
                    object,
                    SET_NOTE_EXPRESSION_RATCHET_FIELD,
                    read_ratchet,
                )?,
                new_micro_timing_ticks: read_required_nullable(
                    object,
                    SET_NOTE_EXPRESSION_MICRO_TIMING_FIELD,
                    read_micro_timing,
                )?,
            })
        }
        SET_NOTE_PITCH_SHAPE_KIND => {
            // ⚠ 未知键拒绝**先**跑：旧值（`oldPitchBendCurve` / `oldSlide`）不在
            // 载荷里 —— 它们由 `compile` 从当前文档读（与 `setNoteExpression` 同口径）。
            reject_set_note_pitch_shape_fields(object)?;
            Ok(NoteOp::SetNotePitchShape {
                note_id: read_id(object, SET_NOTE_PITCH_SHAPE_NOTE_FIELD)?,
                // 弯音在滑音之前（解析顺序，与模型"先滑音后弯音"的值域顺序是两件事）。
                new_bend: read_required_pitch_bend_curve(
                    object,
                    SET_NOTE_PITCH_SHAPE_NEW_BEND_FIELD,
                )?,
                new_slide: read_required_nullable_slide(
                    object,
                    SET_NOTE_PITCH_SHAPE_NEW_SLIDE_FIELD,
                )?,
            })
        }
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
        SET_TEMPO_KIND => {
            reject_set_tempo_fields(object)?;
            Ok(NoteOp::SetTempo {
                // ⚠ `old_bpm` 是**占位**：真值由 `compile` 从当前文档读（⛔ 不信调用方）。
                old_bpm: 0.0,
                new_bpm: read_required_finite_f64(object, SET_TEMPO_BPM_FIELD)?,
            })
        }
        SET_CLIP_PLACEMENT_DURATION_KIND => {
            reject_set_clip_placement_duration_fields(object)?;
            Ok(NoteOp::SetClipPlacementDuration {
                track_id: read_id(object, SET_CLIP_PLACEMENT_DURATION_TRACK_FIELD)?,
                placement_id: read_id(object, SET_CLIP_PLACEMENT_DURATION_PLACEMENT_FIELD)?,
                // ⚠ `old_duration_ticks` 是**占位**：真值由 `compile` 从当前文档读。
                old_duration_ticks: 0,
                new_duration_ticks: read_required_u64(
                    object,
                    SET_CLIP_PLACEMENT_DURATION_DURATION_FIELD,
                )?,
            })
        }
        SET_TRACK_COLOR_KIND => {
            reject_set_track_color_fields(object)?;
            Ok(NoteOp::SetTrackColor {
                track_id: read_id(object, SET_TRACK_COLOR_TRACK_FIELD)?,
                // ⚠ `old_color` 是**占位**：真值由 `compile` 从当前文档读。
                old_color: None,
                new_color: match object.get(SET_TRACK_COLOR_COLOR_FIELD) {
                    // ⚠ **缺键 ≠ 清除**：本形态是单字段设值 ⇒ 缺键响亮拒绝。
                    None => {
                        return Err(missing(
                            SET_TRACK_COLOR_COLOR_FIELD,
                            "非空字符串或 null（null 表示清除、回到主题默认色）",
                        ));
                    }
                    Some(Value::Null) => None,
                    Some(value) => Some(read_non_empty_string(
                        SET_TRACK_COLOR_KIND,
                        value,
                        SET_TRACK_COLOR_COLOR_FIELD,
                        "trackColorMustBeNonEmptyString",
                    )?),
                },
            })
        }
        SET_TRACK_SAMPLER_KIND => {
            reject_set_track_sampler_fields(object)?;
            Ok(NoteOp::SetTrackSampler {
                track_id: read_id(object, SET_TRACK_SAMPLER_TRACK_FIELD)?,
                // ⚠ `old_sampler` 是**占位**：真值由 `compile` 从当前文档读。
                old_sampler: None,
                new_sampler: match object.get(SET_TRACK_SAMPLER_SAMPLER_FIELD) {
                    // ⚠ **缺键 ≠ 解绑**：本形态是单字段设值 ⇒ 缺键响亮拒绝。
                    None => {
                        return Err(missing(
                            SET_TRACK_SAMPLER_SAMPLER_FIELD,
                            "非空字符串或 null（null 表示解绑、这条轨回到不用采样器）",
                        ));
                    }
                    Some(Value::Null) => None,
                    Some(value) => Some(SamplerBinding {
                        path: read_non_empty_string(
                            SET_TRACK_SAMPLER_KIND,
                            value,
                            SET_TRACK_SAMPLER_SAMPLER_FIELD,
                            "trackSamplerMustBeNonEmptyString",
                        )?,
                    }),
                },
            })
        }
        SET_CLIP_PLACEMENT_MUTED_KIND => {
            reject_set_clip_placement_muted_fields(object)?;
            Ok(NoteOp::SetClipPlacementMuted {
                track_id: read_id(object, SET_CLIP_PLACEMENT_MUTED_TRACK_FIELD)?,
                placement_id: read_id(object, SET_CLIP_PLACEMENT_MUTED_PLACEMENT_FIELD)?,
                // ⚠ `old_muted` 是**占位**：真值由 `compile` 从当前文档读。
                old_muted: false,
                new_muted: read_required_bool(object, SET_CLIP_PLACEMENT_MUTED_MUTED_FIELD)?,
            })
        }
        SET_CLIP_PLACEMENT_LOOP_KIND => {
            reject_set_clip_placement_loop_fields(object)?;
            Ok(NoteOp::SetClipPlacementLoop {
                track_id: read_id(object, SET_CLIP_PLACEMENT_LOOP_TRACK_FIELD)?,
                placement_id: read_id(object, SET_CLIP_PLACEMENT_LOOP_PLACEMENT_FIELD)?,
                // ⚠ `old_loop_config` 是**占位**：真值由 `compile` 从当前文档读
                // （⛔ 载荷只带新值；旧值键会被 `reject_set_clip_placement_loop_fields`
                // 当作未知键响亮拒绝）。
                old_loop_config: LoopConfig::default(),
                new_loop_config: LoopConfig {
                    // ⚠ 三个键**全部必填**（整体替换，⛔ 不是合并式 patch）。
                    enabled: read_required_bool(object, LOOP_ENABLED_FIELD)?,
                    start_tick: read_required_u64(object, LOOP_START_TICK_FIELD)?,
                    end_tick: read_required_u64(object, LOOP_END_TICK_FIELD)?,
                },
            })
        }
        SET_DEVICE_BYPASSED_KIND => {
            reject_set_device_bypassed_fields(object)?;
            Ok(NoteOp::SetDeviceBypassed {
                track_id: read_id(object, DEVICE_TRACK_FIELD)?,
                device_id: read_id(object, DEVICE_ID_FIELD)?,
                // ⚠ `old_bypassed` 是**占位**：真值由 `compile` 从当前文档读。
                old_bypassed: false,
                new_bypassed: read_required_bool(object, SET_DEVICE_BYPASSED_BYPASSED_FIELD)?,
            })
        }
        SET_DEVICE_NAME_KIND => {
            reject_set_device_name_fields(object)?;
            Ok(NoteOp::SetDeviceName {
                track_id: read_id(object, DEVICE_TRACK_FIELD)?,
                device_id: read_id(object, DEVICE_ID_FIELD)?,
                // ⚠ `old_name` 是**占位**：真值由 `compile` 从当前文档读。
                old_name: String::new(),
                new_name: read_required_string(object, SET_DEVICE_NAME_NAME_FIELD)?,
            })
        }
        SET_CLIP_NAME_KIND => {
            reject_set_clip_name_fields(object)?;
            Ok(NoteOp::SetClipName {
                clip_id: read_id(object, SET_CLIP_NAME_CLIP_FIELD)?,
                // ⚠ `old_name` 是**占位**：真值由 `compile` 从当前文档读。
                old_name: String::new(),
                new_name: read_required_string(object, SET_CLIP_NAME_NAME_FIELD)?,
            })
        }
        SET_CLIP_GAIN_KIND => {
            reject_set_clip_gain_fields(object)?;
            Ok(NoteOp::SetClipGain {
                clip_id: read_id(object, SET_CLIP_GAIN_CLIP_FIELD)?,
                // ⚠ `old_gain_db` 是**占位**：真值由 `compile` 从当前文档读
                // （MIDI 片段读不到它 ⇒ compile 必须**先**做内容类型门）。
                old_gain_db: 0.0,
                // `read_number` 在**收窄到 f32 之后**判有限性（`1e300 ⇒ inf` 也被拒）。
                new_gain_db: read_number(object, SET_CLIP_GAIN_GAIN_FIELD)?,
            })
        }
        SET_TRACK_NAME_KIND => {
            reject_set_track_name_fields(object)?;
            Ok(NoteOp::SetTrackName {
                track_id: read_id(object, SET_TRACK_NAME_TRACK_FIELD)?,
                // ⚠ `old_name` 是**占位**：真值由 `compile` 从当前文档读。
                old_name: String::new(),
                new_name: read_required_string(object, SET_TRACK_NAME_NAME_FIELD)?,
            })
        }
        SET_AUTHOR_KIND => {
            reject_set_author_fields(object)?;
            Ok(NoteOp::SetAuthor {
                // ⚠ `old_author` 是**占位**：真值由 `compile` 从当前文档读。
                old_author: String::new(),
                new_author: read_required_string(object, SET_AUTHOR_AUTHOR_FIELD)?,
            })
        }
        SET_TITLE_KIND => {
            reject_set_title_fields(object)?;
            Ok(NoteOp::SetTitle {
                // ⚠ `old_title` 是**占位**：真值由 `compile` 从当前文档读。
                old_title: String::new(),
                new_title: read_required_string(object, SET_TITLE_TITLE_FIELD)?,
            })
        }
        SET_TRANSPORT_CONFIG_KIND => Ok(NoteOp::SetTransportConfig {
            new_config: parse_transport_config(object)?,
        }),
        SET_AUTOMATION_LANE_KIND => Ok(NoteOp::SetLane {
            edit: parse_lane_edit(object)?,
        }),
        REMOVE_AUTOMATION_POINT_KIND => {
            let (spec, address) = parse_point_removal(object)?;
            Ok(NoteOp::RemovePoint { spec, address })
        }
        SET_ASSET_KIND => {
            reject_set_asset_fields(object)?;
            let asset_hash = AssetHash::parse(
                object
                    .get(REMOVE_ASSET_HASH_FIELD)
                    .and_then(Value::as_str)
                    .ok_or_else(|| missing(REMOVE_ASSET_HASH_FIELD, "SHA-256 十六进制"))?,
            )
            .map_err(|error| {
                Fault::domain_with_data(
                    ErrorCode::InvalidParameterRange,
                    format!("`{REMOVE_ASSET_HASH_FIELD}` 不是合法 SHA-256: {error}"),
                    serde_json::json!({ "field": REMOVE_ASSET_HASH_FIELD }),
                )
            })?;
            let license = match object.get(SET_ASSET_LICENSE_FIELD) {
                None => None,
                Some(value) => Some(read_non_empty_string(
                    SET_ASSET_KIND,
                    value,
                    SET_ASSET_LICENSE_FIELD,
                    "assetLicenseMustBeNonEmptyString",
                )?),
            };
            let original_path = match object.get(SET_ASSET_ORIGINAL_PATH_FIELD) {
                None => None,
                Some(value) => Some(read_non_empty_string(
                    SET_ASSET_KIND,
                    value,
                    SET_ASSET_ORIGINAL_PATH_FIELD,
                    "assetOriginalPathMustBeNonEmptyString",
                )?),
            };
            let patch = AssetPatch {
                license,
                original_path,
            };
            // ⚠ 至少给一个可改字段：一个字段都不给是一次**无操作** ⇒ 响亮拒绝
            // （与 `setScene` 的 `sceneNameRequiredWhenCreating` 同一条纪律：
            //  让"我以为我改了"在编译期就变成错误）。
            if patch.is_empty() {
                return Err(Fault::domain_with_data(
                    ErrorCode::InvalidParameterRange,
                    format!(
                        "`{SET_ASSET_KIND}` 至少要给一个可改字段 \
                         (`{SET_ASSET_LICENSE_FIELD}` 或 `{SET_ASSET_ORIGINAL_PATH_FIELD}`)"
                    ),
                    serde_json::json!({
                        "reason": "assetPatchIsEmpty",
                        "mutableFields": [
                            SET_ASSET_LICENSE_FIELD,
                            SET_ASSET_ORIGINAL_PATH_FIELD,
                        ],
                        "hint": "`hash`/`byteLen`/`mediaKind` 是**内容身份或内容事实**, \
                                 ⛔ 不可改; 只想改许可就给 `license`",
                    }),
                ));
            }
            Ok(NoteOp::SetAsset { asset_hash, patch })
        }
        REMOVE_ASSET_KIND => {
            reject_remove_asset_fields(object)?;
            Ok(NoteOp::RemoveAsset {
                asset_hash: AssetHash::parse(
                    object
                        .get(REMOVE_ASSET_HASH_FIELD)
                        .and_then(Value::as_str)
                        .ok_or_else(|| missing(REMOVE_ASSET_HASH_FIELD, "SHA-256 十六进制"))?,
                )
                .map_err(|error| {
                    Fault::domain_with_data(
                        ErrorCode::InvalidParameterRange,
                        format!("`{REMOVE_ASSET_HASH_FIELD}` 不是合法 SHA-256: {error}"),
                        serde_json::json!({ "field": REMOVE_ASSET_HASH_FIELD }),
                    )
                })?,
            })
        }
        REMOVE_CLIP_KIND => {
            reject_remove_clip_fields(object)?;
            Ok(NoteOp::RemoveClip)
        }
        REMOVE_TRACK_KIND => {
            reject_remove_track_fields(object)?;
            Ok(NoteOp::RemoveTrack {
                track_id: read_id(object, REMOVE_TRACK_FIELD)?,
            })
        }
        INSERT_DEVICE_KIND => {
            reject_insert_device_fields(object)?;
            let payload = object
                .get(DEVICE_PAYLOAD_FIELD)
                .and_then(Value::as_object)
                .ok_or_else(|| missing(DEVICE_PAYLOAD_FIELD, "对象"))?;
            let device = parse_device(payload)?;
            let slot_index = match object.get(DEVICE_SLOT_FIELD) {
                None => None,
                Some(_) => Some(read_usize(object, DEVICE_SLOT_FIELD)?),
            };
            Ok(NoteOp::InsertDevice {
                track_id: read_id(object, DEVICE_TRACK_FIELD)?,
                slot_index,
                device,
            })
        }
        REMOVE_DEVICE_KIND => {
            reject_remove_device_fields(object)?;
            Ok(NoteOp::RemoveDevice {
                track_id: read_id(object, DEVICE_TRACK_FIELD)?,
                device_id: read_id(object, DEVICE_ID_FIELD)?,
            })
        }
        SET_ROUTING_GAIN_KIND => {
            reject_routing_gain_fields(object)?;
            Ok(NoteOp::SetRoutingGain {
                edge_id: read_id(object, SET_ROUTING_GAIN_EDGE_FIELD)?,
                gain_db: read_routing_gain(object)?,
            })
        }
        CONNECT_ROUTING_KIND => {
            reject_connect_routing_fields(object)?;
            Ok(NoteOp::ConnectRouting {
                edge_id: read_id(object, CONNECT_EDGE_FIELD)?,
                source_node: read_id(object, CONNECT_SOURCE_FIELD)?,
                destination_node: read_id(object, CONNECT_DESTINATION_FIELD)?,
                kind: read_required_routing_kind(object, CONNECT_EDGE_KIND_FIELD)?,
                gain_db: read_optional_finite_f32(object, CONNECT_GAIN_FIELD)?,
            })
        }
        DISCONNECT_ROUTING_KIND => {
            reject_disconnect_routing_fields(object)?;
            Ok(NoteOp::DisconnectRouting {
                edge_id: read_id(object, ROUTING_EDGE_FIELD)?,
            })
        }
        ADD_TRACK_KIND => Ok(NoteOp::AddTrack {
            track: parse_new_track(object)?,
        }),
        ADD_ROUTING_NODE_KIND => {
            reject_add_routing_node_fields(object)?;
            Ok(NoteOp::AddRoutingNode {
                node_id: read_id(object, ROUTING_NODE_FIELD)?,
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
        SET_SECTION_KIND => {
            reject_set_section_op_fields(object)?;
            let payload = object
                .get(SECTION_PAYLOAD_FIELD)
                .and_then(Value::as_object)
                .ok_or_else(|| missing(SECTION_PAYLOAD_FIELD, "对象"))?;
            reject_set_section_fields(payload)?;
            let section_id = read_id(payload, SECTION_FIELD)?;
            let create = read_optional_scene_bool(payload, SECTION_CREATE_FIELD)?.unwrap_or(false);
            let patch = parse_section_patch(payload)?;
            // 与 `setScene` 的 `create` 分支**同一条理由**：模型的 `SectionV3::name`
            // 没有校验, 空名会静默落盘 (而段落名是用户在编曲视图里唯一能读到的标签)。
            if create && patch.name.is_none() {
                return Err(Fault::domain_with_data(
                    ErrorCode::InvalidParameterRange,
                    format!(
                        "`{SET_SECTION_KIND}` 且 `{SECTION_CREATE_FIELD}: true` 时必须给出 \
                         `{SECTION_NAME_FIELD}`: 模型的 `SectionV3::name` 没有校验, \
                         空名会静默落盘"
                    ),
                    serde_json::json!({
                        "field": format!("{SECTION_PAYLOAD_FIELD}.{SECTION_NAME_FIELD}"),
                        "reason": "sectionNameRequiredWhenCreating",
                        "hint": "新建段落必须有名 (`name` 是非空字符串); \
                                 只想改名字/区间/色标就不要给 `create: true`",
                    }),
                ));
            }
            Ok(NoteOp::SetSection {
                section_id,
                create,
                patch,
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
/// 拒绝 `setTempo` 里 [`SET_TEMPO_FIELDS`] 之外的键。
fn reject_set_tempo_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    reject_unknown_keys(
        object,
        &SET_TEMPO_FIELDS,
        SET_TEMPO_KIND,
        "unknownSetTempoField",
    )
}

/// 拒绝 `setAuthor` 里 [`SET_AUTHOR_FIELDS`] 之外的键。
fn reject_set_clip_placement_duration_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    reject_unknown_keys(
        object,
        &SET_CLIP_PLACEMENT_DURATION_FIELDS,
        SET_CLIP_PLACEMENT_DURATION_KIND,
        "unknownSetClipPlacementDurationField",
    )
}

/// 拒绝 `setNoteTiming` 里 [`SET_NOTE_TIMING_FIELDS`] 之外的键。
///
/// 与同族 `reject_*_fields` 同一口径（拼错的键必须被拒绝、⛔ 不能静默忽略）：
/// 最像"写对了"的错法是把旧值（`oldStartTick` / `oldDurationTicks`）也写进来 ——
/// 那会被静默忽略，而调用方以为前置条件由自己声明。
fn reject_set_note_timing_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    reject_unknown_keys(
        object,
        &SET_NOTE_TIMING_FIELDS,
        SET_NOTE_TIMING_KIND,
        "unknownSetNoteTimingField",
    )
}

/// 拒绝 `setNoteExpression` 里 [`SET_NOTE_EXPRESSION_FIELDS`] 之外的键。
///
/// 与同族 `reject_*_fields` 同一口径（拼错的键必须被拒绝、⛔ 不能静默忽略）：
/// 最像"写对了"的错法是把三个旧值（`oldProbability` 等）也写进来 —— 那会被静默忽略，
/// 而调用方以为前置条件由自己声明。
fn reject_set_note_expression_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    reject_unknown_keys(
        object,
        &SET_NOTE_EXPRESSION_FIELDS,
        SET_NOTE_EXPRESSION_KIND,
        "unknownSetNoteExpressionField",
    )
}

/// 拒绝 `setNotePitchShape` 里 [`SET_NOTE_PITCH_SHAPE_FIELDS`] 之外的键。
///
/// 与同族 `reject_*_fields` 同一口径（拼错的键必须被拒绝、⛔ 不能静默忽略）：
/// 最像"写对了"的错法是把**旧值**（`oldPitchBendCurve` / `oldSlide`）写进载荷 ——
/// 它们由 [`compile`] 从当前文档读，写在这里只会被静默忽略，而调用方以为前置条件
/// 由自己声明（与 `setNoteExpression` 的 `oldProbability` 是同一条教训）；
/// 其次是 `add` 的内层键名（`pitchBendCurve` / `slide`）或简称（`newBend`）。
fn reject_set_note_pitch_shape_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    reject_unknown_keys(
        object,
        &SET_NOTE_PITCH_SHAPE_FIELDS,
        SET_NOTE_PITCH_SHAPE_KIND,
        "unknownSetNotePitchShapeField",
    )
}

fn reject_set_track_color_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    reject_unknown_keys(
        object,
        &SET_TRACK_COLOR_FIELDS,
        SET_TRACK_COLOR_KIND,
        "unknownSetTrackColorField",
    )
}

/// 拒绝 `setTrackSampler` 里 [`SET_TRACK_SAMPLER_FIELDS`] 之外的键。
///
/// 与 [`reject_set_track_color_fields`] 同一口径（"拼错的键必须被拒绝，⛔ 不能静默
/// 忽略"）：最像"写对了"的两种错法是把工具**顶层**的 `trackId` 当成载荷的一部分
/// 再写一遍，或把路径写成 `path` / `sfzPath` —— 两种都会被静默忽略，而调用方以为
/// 采样器已经绑上（这正是 `#220` 缺陷 ① 的形状：**静默不生效**）。
fn reject_set_track_sampler_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    reject_unknown_keys(
        object,
        &SET_TRACK_SAMPLER_FIELDS,
        SET_TRACK_SAMPLER_KIND,
        "unknownSetTrackSamplerField",
    )
}

fn reject_set_clip_placement_muted_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    reject_unknown_keys(
        object,
        &SET_CLIP_PLACEMENT_MUTED_FIELDS,
        SET_CLIP_PLACEMENT_MUTED_KIND,
        "unknownSetClipPlacementMutedField",
    )
}

/// 拒绝 `setClipPlacementLoop` 里 [`SET_CLIP_PLACEMENT_LOOP_FIELDS`] 之外的键。
///
/// 与同族 `reject_*_fields` 同一口径（拼错的键必须被拒绝、⛔ 不能静默忽略）：
/// 最像"写对了"的错法是把**旧值**写进载荷（`oldLoopConfig` / `oldEnabled` /
/// `oldStartTick` / `oldEndTick`）—— 它们由 [`compile`] 从当前文档读，写在这里只会被
/// 静默忽略，而调用方以为前置条件由自己声明（`#334` 的 `oldPitchBendCurve` /
/// `oldSlide` 与 `setNoteExpression` 的 `oldProbability` 是同一条教训）。
fn reject_set_clip_placement_loop_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    reject_unknown_keys(
        object,
        &SET_CLIP_PLACEMENT_LOOP_FIELDS,
        SET_CLIP_PLACEMENT_LOOP_KIND,
        "unknownSetClipPlacementLoopField",
    )
}

fn reject_set_device_bypassed_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    reject_unknown_keys(
        object,
        &SET_DEVICE_BYPASSED_FIELDS,
        SET_DEVICE_BYPASSED_KIND,
        "unknownSetDeviceBypassedField",
    )
}

fn reject_set_device_name_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    reject_unknown_keys(
        object,
        &SET_DEVICE_NAME_FIELDS,
        SET_DEVICE_NAME_KIND,
        "unknownSetDeviceNameField",
    )
}

fn reject_set_clip_name_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    reject_unknown_keys(
        object,
        &SET_CLIP_NAME_FIELDS,
        SET_CLIP_NAME_KIND,
        "unknownSetClipNameField",
    )
}

/// 拒绝 `setClipGain` 里 [`SET_CLIP_GAIN_FIELDS`] 之外的键。
fn reject_set_clip_gain_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    reject_unknown_keys(
        object,
        &SET_CLIP_GAIN_FIELDS,
        SET_CLIP_GAIN_KIND,
        "unknownSetClipGainField",
    )
}

fn reject_set_track_name_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    reject_unknown_keys(
        object,
        &SET_TRACK_NAME_FIELDS,
        SET_TRACK_NAME_KIND,
        "unknownSetTrackNameField",
    )
}

fn reject_set_author_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    reject_unknown_keys(
        object,
        &SET_AUTHOR_FIELDS,
        SET_AUTHOR_KIND,
        "unknownSetAuthorField",
    )
}

/// 拒绝 `setTitle` 里 [`SET_TITLE_FIELDS`] 之外的键。
fn reject_set_title_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    reject_unknown_keys(
        object,
        &SET_TITLE_FIELDS,
        SET_TITLE_KIND,
        "unknownSetTitleField",
    )
}

/// 通用的"只认这几个键"检查（⛔ 多写一个键不静默丢弃）。
fn reject_unknown_keys(
    object: &Map<String, Value>,
    allowed: &[&str],
    kind: &str,
    reason: &str,
) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !allowed.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{kind}` 操作里有不支持的键: {} (支持集合只有 {allowed:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": reason,
            "unsupportedFields": unknown,
            "supportedFields": allowed,
        }),
    ))
}

/// 读一个**必填**的有限 `f64`（必须是 JSON 数字）。
fn read_required_finite_f64(object: &Map<String, Value>, field: &str) -> Result<f64, Fault> {
    let raw = object.get(field).ok_or_else(|| missing(field, "数字"))?;
    let number = raw.as_f64().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是数字, 实际收到 {raw}"),
            serde_json::json!({ "field": field, "reason": "valueMustBeNumber" }),
        )
    })?;
    if !number.is_finite() {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须有限, 实际收到 {number}"),
            serde_json::json!({
                "field": field,
                "reason": "nonFiniteValue",
                "value": number,
                "narrowedToF32": false,
            }),
        ));
    }
    Ok(number)
}

/// 读一个**必填**字符串（⛔ 不查空串、⛔ 不查长度 —— 模型与 schema 都没有这两条约束）。
fn read_required_string(object: &Map<String, Value>, field: &str) -> Result<String, Fault> {
    let raw = object.get(field).ok_or_else(|| missing(field, "字符串"))?;
    raw.as_str().map(str::to_owned).ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是字符串, 实际收到 {raw}"),
            serde_json::json!({
                "field": field,
                "reason": "valueMustBeString",
                "received": raw,
            }),
        )
    })
}

/// 拒绝 `connectRouting` 里 [`CONNECT_ROUTING_FIELDS`] 之外的键。
fn reject_connect_routing_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !CONNECT_ROUTING_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{CONNECT_ROUTING_KIND}` 操作里有不支持的键: {} \
             (支持集合只有 {CONNECT_ROUTING_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownConnectRoutingField",
            "unsupportedFields": unknown,
            "supportedConnectRoutingFields": CONNECT_ROUTING_FIELDS,
            "hint": "本形态是**平铺**的 (⛔ 不嵌套 `edge: {{...}}` 对象): \
                     `edgeId`/`sourceNode`/`destinationNode`/`edgeKind` 四个必填, \
                     `gainDb` 可选 (`null` 或省略都 = 单位增益)",
        }),
    ))
}

/// 读一个**必填**的路由类型名（取值与线协议逐字相同，⛔ 不发明拼写）。
fn read_required_routing_kind(
    object: &Map<String, Value>,
    field: &str,
) -> Result<RoutingKind, Fault> {
    let raw = object
        .get(field)
        .ok_or_else(|| missing(field, "路由类型名"))?;
    let text = raw.as_str().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是字符串, 实际收到 {raw}"),
            serde_json::json!({
                "field": field,
                "reason": "valueMustBeString",
                "received": raw,
            }),
        )
    })?;
    match text {
        "TrackToBus" => Ok(RoutingKind::TrackToBus),
        "BusToMaster" => Ok(RoutingKind::BusToMaster),
        "SendToAux" => Ok(RoutingKind::SendToAux),
        "Sidechain" => Ok(RoutingKind::Sidechain),
        other => Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{field}` 不是规范的路由类型: {other} \
                 (规范名与线协议逐字相同: TrackToBus / BusToMaster / SendToAux / Sidechain)"
            ),
            serde_json::json!({
                "field": field,
                "reason": "unknownRoutingKind",
                "received": other,
                "allowed": ["TrackToBus", "BusToMaster", "SendToAux", "Sidechain"],
            }),
        )),
    }
}

/// 读一个**可选**的有限 `f32`（键**省略** 与 `null` **同义** ⇒ 都返回 `None`）。
///
/// ⚠ 与 [`SET_ROUTING_GAIN_KIND`] 的 `value` **同口径**（⛔ 不造第二套）：
/// `null` = `None` = **单位增益**；数字 = `Some`（必须有限，否则 `nonFiniteValue`）。
/// 省略整键 ⇒ `None`（因为 `RoutingEdge::gain_db` 带 `#[serde(default)]`，
/// 模型里"缺键"就是单位增益）。
fn read_optional_finite_f32(
    object: &Map<String, Value>,
    field: &str,
) -> Result<Option<f32>, Fault> {
    let Some(raw) = object.get(field) else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    let number = raw.as_f64().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是数字或 null, 实际收到 {raw}"),
            serde_json::json!({
                "field": field,
                "reason": "valueMustBeNumberOrNull",
                "received": raw,
            }),
        )
    })?;
    let narrowed = number as f32;
    if !narrowed.is_finite() {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 的增益必须有限, 实际收到 {number}"),
            serde_json::json!({
                "field": field,
                "reason": "nonFiniteValue",
                "value": number,
                "narrowedToF32": true,
            }),
        ));
    }
    Ok(Some(narrowed))
}

/// 拒绝 `addTrack` 里 [`ADD_TRACK_FIELDS`] 之外的键。
///
/// ⚠ **四个集合与 `folderId`/`color`/五个混音标量都**不在**支持集合里** —— 那是
/// **决定**（理由见 [`ADD_TRACK_FIELDS`] 与 [`NoteOp::AddTrack`] 的文档）：新实体
/// 无内容可清。多写一个键是**响亮失败**，⛔ 不静默丢弃。
fn reject_add_track_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !ADD_TRACK_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{ADD_TRACK_KIND}` 操作里有不支持的键: {} \
             (支持集合只有 {ADD_TRACK_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownAddTrackField",
            "unsupportedFields": unknown,
            "supportedAddTrackFields": ADD_TRACK_FIELDS,
            "hint": "本形态只收 `kind`/`trackId`/`name`/`trackKind` 四个键; 四个集合 \
                     (devices/macros/automationLanes/clips) 与 folderId/color 以及五个 \
                     混音标量**不在**载荷里 —— 新实体用**空形状** (见本形态的文档)",
        }),
    ))
}

/// 读一个**必填**的非空字符串（⛔ 空串不做真值强转，直接拒绝）。
fn read_required_non_empty_string(
    object: &Map<String, Value>,
    field: &str,
) -> Result<String, Fault> {
    let raw = object
        .get(field)
        .ok_or_else(|| missing(field, "非空字符串"))?;
    let text = raw.as_str().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是非空字符串, 实际收到 {raw}"),
            serde_json::json!({
                "field": field,
                "reason": "valueMustBeString",
                "received": raw,
            }),
        )
    })?;
    if text.is_empty() {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{field}` 不能是空字符串 (模型的 `TrackV3::validate` 不查空名 ⇒ 空名会静默落盘)"
            ),
            serde_json::json!({
                "field": field,
                "reason": "valueMustBeNonEmptyString",
                "received": text,
            }),
        ));
    }
    Ok(text.to_owned())
}

/// 读一个**必填**的音轨类型名（取值与线协议逐字相同，⛔ 不发明拼写）。
fn read_required_track_kind(object: &Map<String, Value>, field: &str) -> Result<TrackKind, Fault> {
    let raw = object
        .get(field)
        .ok_or_else(|| missing(field, "音轨类型名"))?;
    let text = raw.as_str().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是字符串, 实际收到 {raw}"),
            serde_json::json!({
                "field": field,
                "reason": "valueMustBeString",
                "received": raw,
            }),
        )
    })?;
    match text {
        "Midi" => Ok(TrackKind::Midi),
        "Audio" => Ok(TrackKind::Audio),
        "AuxReturn" => Ok(TrackKind::AuxReturn),
        "Master" => Ok(TrackKind::Master),
        other => Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{field}` 不是规范的音轨类型: {other} \
                 (规范名与线协议逐字相同: Midi / Audio / AuxReturn / Master)"
            ),
            serde_json::json!({
                "field": field,
                "reason": "unknownTrackKind",
                "received": other,
                "allowed": ["Midi", "Audio", "AuxReturn", "Master"],
            }),
        )),
    }
}

/// 解析 `addTrack` 的载荷（三个字段**全部必填**）⇒ 一条**空形状**的音轨。
///
/// ⚠ **四个集合与 `folderId`/`color`/五个混音标量都取 `TrackV3::default()`** ——
/// 那是**决定**（见 [`NoteOp::AddTrack`]）。因此一条"新建空轨"**不会**改动混音：
/// `volume_db = 0.0`、`pan = 0.0`、`mute = solo = solo_safe = false`。
fn parse_new_track(object: &Map<String, Value>) -> Result<TrackV3, Fault> {
    reject_add_track_fields(object)?;
    Ok(TrackV3 {
        id: read_id(object, ADD_TRACK_TRACK_FIELD)?,
        name: read_required_non_empty_string(object, ADD_TRACK_NAME_FIELD)?,
        kind: read_required_track_kind(object, ADD_TRACK_KIND_FIELD)?,
        // ⚠ 空形状：理由见本函数与 `NoteOp::AddTrack` 的文档。
        ..TrackV3::default()
    })
}

/// 拒绝 `addRoutingNode` 里 [`ADD_ROUTING_NODE_FIELDS`] 之外的键。
///
/// 与 [`reject_remove_routing_node_fields`] 同形，但 `reason` **不同名**
/// （`unknownAddRoutingNodeField`）：两条形态虽然载荷同型，但它们做的是**相反**的事，
/// 共用一个 `reason` 会让"是哪一次调用被拒"从 `reason` 上读不出来。
fn reject_add_routing_node_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !ADD_ROUTING_NODE_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{ADD_ROUTING_NODE_KIND}` 操作里有不支持的键: {} \
             (支持集合只有 {ADD_ROUTING_NODE_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownAddRoutingNodeField",
            "unsupportedFields": unknown,
            "supportedAddRoutingNodeFields": ADD_ROUTING_NODE_FIELDS,
            "hint": "本形态的载荷是空的 (只认 `kind` 与 `nodeId`); 节点身份由 \
                     `yeban_query_project` 的 `routing_graph.nodes` 报出 —— ⚠ 模型**不要求**\
                     它对应一条真实音轨 (只查节点不重复、且每条边的两端都在 `nodes` 里), \
                     所以新音轨要先 `addRoutingNode` 才成为节点; 边由 `edgeId` \
                     寻址 (节点与边是两种实体)",
        }),
    ))
}

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

/// 拒绝 `removeTrack` 操作对象里 [`REMOVE_TRACK_FIELDS`] 之外的键。
///
/// 与 [`reject_remove_scene_fields`] / [`reject_remove_section_fields`] /
/// [`reject_remove_routing_node_fields`] 同一口径（"拼错的键必须被拒绝, 不能静默忽略"）：
/// 最像"写对了"的几种错法是给一个**别的实体**的寻址（`sceneId` / `sectionId` /
/// `nodeId` / `edgeId`）、给一个**工具顶层**的 `clipId`（那是"目标片段"，不是音轨）、
/// 或以为要报告"音轨取走前的状态"而多写 `previousTrack` —— 三种都会被静默忽略，
/// 而调用方以为音轨已经取走。
///
/// ⚠ 顶层 `trackId` 与操作对象自带的 `trackId` **不是同一个东西**：后者是本形态的
/// 寻址（被取走的那条音轨），前者是"目标片段所在的音轨"（[`compile`] 的入口拿它查
/// 音轨与片段池）。两者都叫 `trackId`，但一个在信封里、一个在操作对象里，
/// 因此本函数不会把顶层那个键当成"未知键" —— 它根本不在操作对象里。
///
/// # Errors
///
/// 出现 `kind` / `trackId` 之外的键 → `INVALID_PARAMETER_RANGE`
/// （`data.reason = "unknownRemoveTrackField"`）。
fn reject_remove_track_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !REMOVE_TRACK_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{REMOVE_TRACK_KIND}` 操作里有不支持的键: {} \
             (支持集合只有 {REMOVE_TRACK_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownRemoveTrackField",
            "unsupportedFields": unknown,
            "supportedRemoveTrackFields": REMOVE_TRACK_FIELDS,
            "hint": "本形态的载荷是空的 (只认 `kind` 与 `trackId`); 音轨身份取自 \
                     `yeban_query_project` 的 `tracks` 字段, 撤销载荷 `previousTrack` \
                     由服务端从当前文档读 (不接受调用方声明)",
        }),
    ))
}

/// 拒绝 `insertDevice` 操作对象里 [`INSERT_DEVICE_FIELDS`] 之外的键。
///
/// 与 [`reject_remove_track_fields`] 同一口径（"拼错的键必须被拒绝, 不能静默忽略"）：
/// 最像"写对了"的几种错法是给一个**别的实体**的寻址（`nodeId` / `edgeId` / `sectionId`）、
/// 把设备身份直接写在顶层（`deviceId` —— 它在 `device` 载荷里）、或把 `slot` / `index`
/// 当成 [`DEVICE_SLOT_FIELD`] 的别名 —— 都会被静默忽略，而调用方以为设备已经插上。
///
/// # Errors
///
/// 出现 `kind` / `trackId` / `device` / `slotIndex` 之外的键 →
/// `INVALID_PARAMETER_RANGE`（`data.reason = "unknownInsertDeviceField"`）。
fn reject_insert_device_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !INSERT_DEVICE_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{INSERT_DEVICE_KIND}` 操作里有不支持的键: {} \
             (支持集合只有 {INSERT_DEVICE_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownInsertDeviceField",
            "unsupportedFields": unknown,
            "supportedInsertDeviceFields": INSERT_DEVICE_FIELDS,
            "hint": "目标音轨在操作对象自带的 `trackId` 上; 设备定义在 `device` 对象里 \
                     (字段集合见 `supportedDeviceFields`); `slotIndex` 可省 (缺省 = 追加到链尾)",
            "supportedDeviceFields": DEVICE_FIELDS,
        }),
    ))
}

/// 拒绝 `removeDevice` 操作对象里 [`REMOVE_DEVICE_FIELDS`] 之外的键。
///
/// 与 [`reject_remove_track_fields`] 同一口径：最像"写对了"的错法是把设备定义整份
/// 搬过来（`device` —— 那是 `insertDevice` 的形状）、或以为要报告"取走前的状态"而多写
/// `previousDevice` —— 两者都会被静默忽略，而调用方以为设备已经取走。
///
/// # Errors
///
/// 出现 `kind` / `trackId` / `deviceId` 之外的键 → `INVALID_PARAMETER_RANGE`
/// （`data.reason = "unknownRemoveDeviceField"`）。
fn reject_remove_device_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !REMOVE_DEVICE_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{REMOVE_DEVICE_KIND}` 操作里有不支持的键: {} \
             (支持集合只有 {REMOVE_DEVICE_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownRemoveDeviceField",
            "unsupportedFields": unknown,
            "supportedRemoveDeviceFields": REMOVE_DEVICE_FIELDS,
            "hint": "本形态只认 `kind` / `trackId` / `deviceId`; 设备身份取自 \
                     `yeban_query_engine_state` 的 `tracks[].devices[].deviceId`, \
                     撤销载荷 `previousDevice` 由服务端从当前文档读 (不接受调用方声明)",
        }),
    ))
}

/// 把 `device` 载荷对象解析成 [`DeviceDefinition`]。
///
/// **有自由度的一处裁决（逐条给理由，不冒充规范）**：`.yeban` 容器里的 `project.json`
/// 用的是模型 serde 的字段名（`DeviceDefinition` 没有 `rename_all` ⇒ `latency_samples`
/// 是 snake_case），而 MCP 工具面通篇 camelCase（`clipId` / `startTick` /
/// `microTimingTicks` / 读侧的 `latencySamples`）。本形态取**工具面**的口径：
/// `deviceId` / `name` / `kind` / `bypassed` / `latencySamples` / `params`，
/// 其中 `kind` 的取值逐字等于模型 serde 的变体名（[`DEVICE_KINDS`]）。
/// 于是同一个词在工具面与容器里对**枚举**是同一个字面量，对**字段名**差一个大写分界
/// —— 这条差异写在 [`DEVICE_FIELDS`] 的文档上，不静默。
///
/// 为什么用具名读取而不是 `serde_json::from_value::<DeviceDefinition>`：
/// serde 会**静默忽略**未知键（拼错的 `latencySample` 会被当成"零延迟设备"读进来，
/// 而 PDC 的正确性完全依赖这个数字），因此"响亮拒绝未知键"这条本 crate 的一贯口径
/// 必须由本函数自己执行。
///
/// # Errors
///
/// - 出现 [`DEVICE_FIELDS`] 之外的键 → `INVALID_PARAMETER_RANGE`
///   （`data.reason = "unknownDeviceField"`）；
/// - 缺字段 / 类型不对 → `INVALID_PARAMETER_RANGE`（缺字段走统一的 [`missing`]）；
/// - `kind` 不在 [`DEVICE_KINDS`] 里 → `INVALID_PARAMETER_RANGE`
///   （`data.reason = "unknownDeviceKind"`）；
/// - `latencySamples` 不是 `u32` → `INVALID_PARAMETER_RANGE`
///   （`data.reason = "latencyOutOfRange"`）；
/// - `params[]` 的 `value` 在收窄到 `f32` 之后不是有限数 →
///   `INVALID_PARAMETER_RANGE`（`data.reason = "nonFiniteValue"`）；
/// - ⭐ `params[].name` 与某个已识别拼写**只差一两个字符**（拼错） →
///   `INVALID_PARAMETER_RANGE`（`data.reason = "unknownEffectParam"`，带最近邻）——
///   见 [`reject_misspelled_effect_params`]；
/// - `name` 是空串 → `INVALID_PARAMETER_RANGE`（`data.reason = "deviceNameRequired"`）。
fn parse_device(payload: &Map<String, Value>) -> Result<DeviceDefinition, Fault> {
    let mut unknown: Vec<&str> = payload
        .keys()
        .map(String::as_str)
        .filter(|key| !DEVICE_FIELDS.contains(key))
        .collect();
    if !unknown.is_empty() {
        unknown.sort_unstable();
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{DEVICE_PAYLOAD_FIELD}` 里有不支持的键: {} \
                 (支持集合只有 {DEVICE_FIELDS:?})",
                unknown.join(", ")
            ),
            serde_json::json!({
                "reason": "unknownDeviceField",
                "unsupportedFields": unknown,
                "supportedDeviceFields": DEVICE_FIELDS,
                "hint": "字段名用工具面的 camelCase (`deviceId` / `latencySamples`) —— \
                         `.yeban` 容器里的 `project.json` 用模型 serde 的 snake_case \
                         (`latency_samples`), 两者不是同一份拼写",
            }),
        ));
    }
    let id = read_id(payload, DEVICE_ID_FIELD)?;
    let name = match payload.get("name") {
        Some(Value::String(text)) if !text.is_empty() => text.clone(),
        Some(Value::String(_)) => {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                "`device.name` 不得为空串".to_owned(),
                serde_json::json!({
                    "field": format!("{DEVICE_PAYLOAD_FIELD}.name"),
                    "reason": "deviceNameRequired",
                }),
            ));
        }
        Some(other) => {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("`device.name` 必须是字符串, 实际收到 {other}"),
            ));
        }
        None => return Err(missing("device.name", "非空字符串")),
    };
    let kind = {
        let raw = payload
            .get("kind")
            .ok_or_else(|| missing("device.kind", "设备类型字符串"))?;
        let text = raw.as_str().ok_or_else(|| {
            Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("`device.kind` 必须是字符串, 实际收到 {raw}"),
            )
        })?;
        match text {
            "InternalInstrument" => DeviceKind::InternalInstrument,
            "InternalEffect" => DeviceKind::InternalEffect,
            "ExternalInstrument" => DeviceKind::ExternalInstrument,
            "ExternalEffect" => DeviceKind::ExternalEffect,
            other => {
                return Err(Fault::domain_with_data(
                    ErrorCode::InvalidParameterRange,
                    format!("未知 `device.kind`: `{other}`"),
                    serde_json::json!({
                        "reason": "unknownDeviceKind",
                        "supportedDeviceKinds": DEVICE_KINDS,
                    }),
                ));
            }
        }
    };
    let bypassed = payload.get("bypassed").map_or_else(
        || Err(missing("device.bypassed", "布尔值")),
        |raw| {
            raw.as_bool().ok_or_else(|| {
                Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    format!("`device.bypassed` 必须是布尔值, 实际收到 {raw}"),
                )
            })
        },
    )?;
    let latency_samples = {
        let raw = payload
            .get("latencySamples")
            .ok_or_else(|| missing("device.latencySamples", "u32 整数"))?;
        let number = raw.as_u64().ok_or_else(|| {
            Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("`device.latencySamples` 必须是非负整数, 实际收到 {raw}"),
            )
        })?;
        u32::try_from(number).map_err(|_| {
            Fault::domain_with_data(
                ErrorCode::OutOfRange,
                format!("`device.latencySamples` 越界: {number} 不在 0..=4294967295"),
                serde_json::json!({
                    "field": "device.latencySamples",
                    "reason": "latencyOutOfRange",
                    "value": number,
                    "max": u32::MAX,
                }),
            )
        })?
    };
    let params = {
        let raw = payload
            .get("params")
            .ok_or_else(|| missing("device.params", "数组"))?;
        let items = raw.as_array().ok_or_else(|| {
            Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("`device.params` 必须是数组, 实际收到 {raw}"),
            )
        })?;
        let mut parsed = Vec::with_capacity(items.len());
        for item in items {
            parsed.push(parse_device_param(item)?);
        }
        parsed
    };
    // ⭐ #175：拼错的参数名在**写入时**就响亮失败（⛔ 不再"存下来、静默不生效"）。
    reject_misspelled_effect_params(&name, &params)?;
    Ok(DeviceDefinition {
        id,
        name,
        kind,
        bypassed,
        params,
        latency_samples,
    })
}

/// 拒绝"与某个已识别拼写只差一两个字符"的设备参数名（**#175 坑⑤**）。
///
/// ## 为什么需要它（实测，⛔ 不是假想）
///
/// #174 的外部 agent 试跑写下 `{"name":"eq_low_gian","value":9.0}`，读到
/// `status:"success"` —— 而引擎不认这个名字，设备链一台效果器都没武装
/// （`effects:[]`、`recognisedParams:[]`、`unrecognisedParams:["eq_low_gian"]`）。
/// agent **以为写成了**。根因不是"读回不够"，是**写入侧没有声音**。
///
/// ## 判据：够近才拒绝，⛔ 不是"未知就拒绝"
///
/// [`crate::effect_params::similar_spellings`] 只在编辑距离 ≤
/// [`crate::effect_params::TYPO_MAX_DISTANCE`]（2）时给建议。那时我们**能确定**
/// 调用方想写哪个名字 ⇒ 拒绝并把最近邻逐字交还，比"成功但没生效"诚实。
///
/// 距离更远的未知名字（`reverb_mix` / 外部插件自己的旋钮名）**照旧接受**：
/// `#156` 的验收面要求它们在 `inserts[].unrecognisedParams` 里可见，而且
/// 工具面没有外部插件参数词表，替调用方拒绝会误伤。它们**不静默** —— 提案响应的
/// `warnings[]` 会逐条报出（见 `domain::mod::effect_param_warnings`）。
///
/// # Errors
///
/// 有至少一个 `params[].name` 命中"够近的错字" ⇒ `INVALID_PARAMETER_RANGE`
/// （`data.reason = "unknownEffectParam"`；`data.suggestions[]` 逐条给
/// `requested` / `spelling` / `canonicalName` / `effect` / `distance`）。
fn reject_misspelled_effect_params(
    device_name: &str,
    params: &[ParameterValue],
) -> Result<(), Fault> {
    let mut misspelled: Vec<(&str, Vec<crate::effect_params::SimilarSpelling>)> = Vec::new();
    for param in params {
        // 已识别的拼写（规范名或别名）一个字都不动。
        if crate::effect_params::recognise(&param.name).is_some() {
            continue;
        }
        let similar = crate::effect_params::similar_spellings(&param.name);
        if !similar.is_empty() {
            misspelled.push((param.name.as_str(), similar));
        }
    }
    if misspelled.is_empty() {
        return Ok(());
    }
    let requested: Vec<&str> = misspelled.iter().map(|(name, _)| *name).collect();
    // 报文逐字点名"你是指谁"：只写"未知参数"仍然要靠调用方自己再猜一次。
    let advice: Vec<String> = misspelled
        .iter()
        .map(|(name, similar)| {
            let best = similar[0];
            format!(
                "你是指 `{}`？（你写的是 `{name}`，编辑距离 {}）",
                best.canonical, best.distance
            )
        })
        .collect();
    let suggestions: Vec<Value> = misspelled
        .iter()
        .flat_map(|(name, similar)| similar.iter().map(move |spelling| spelling.to_value(name)))
        .collect();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`device.params` 里有引擎不认的**拼错**名字（设备 `{device_name}`）: {}。{} \
             引擎不认的拼写写进设备也**不会生效**, 因此本次调用响亮失败 (⛔ 不再静默通过)。\
             全部可用拼写见 `tools/list` 的 `insertDevice.device.params.items`。",
            requested.join(", "),
            advice.join("; "),
        ),
        serde_json::json!({
            "reason": "unknownEffectParam",
            "deviceName": device_name,
            "unrecognisedParams": requested,
            "suggestions": suggestions,
            "hint": "拼写与某个已识别名字的编辑距离 <= 2 ⇒ 写入侧直接拒绝并给出最近邻; \
                     距离更远的未知名字仍然接受, 但会出现在提案响应的 warnings[] 与读回的 \
                     tracks[].inserts[].unrecognisedParams 里",
        }),
    ))
}

/// 解析 `device.params[]` 的一个元素。
///
/// 与 [`parse_device`] 同一口径（用具名读取而不是 serde，未知键响亮拒绝）。
/// `value` 走 [`read_number`]：它在 f64 → f32 收窄之后判有限性，
/// 与 `setParam` / `probability` 逐字同一口径（收窄**之前**有限、之后溢出成 `inf`
/// 的输入也必须被拒 —— 那正是 `ef459f9` 修掉的那类缺陷）。
///
/// # Errors
///
/// - 元素不是对象 / 出现 [`DEVICE_PARAM_FIELDS`] 之外的键 →
///   `INVALID_PARAMETER_RANGE`（`data.reason = "unknownDeviceParamField"`）；
/// - `name` 缺 / 不是字符串 → `INVALID_PARAMETER_RANGE`；
/// - `value` 缺 / 不是有限数 → `INVALID_PARAMETER_RANGE`
///   （非有限走 `data.reason = "nonFiniteValue"`）；
/// - `unit` 给了但不是字符串也不是 `null` → `INVALID_PARAMETER_RANGE`。
fn parse_device_param(item: &Value) -> Result<ParameterValue, Fault> {
    let object = item.as_object().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`device.params` 的元素必须是对象, 实际收到 {item}"),
        )
    })?;
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !DEVICE_PARAM_FIELDS.contains(key))
        .collect();
    if !unknown.is_empty() {
        unknown.sort_unstable();
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`device.params` 的元素里有不支持的键: {} \
                 (支持集合只有 {DEVICE_PARAM_FIELDS:?})",
                unknown.join(", ")
            ),
            serde_json::json!({
                "reason": "unknownDeviceParamField",
                "unsupportedFields": unknown,
                "supportedDeviceParamFields": DEVICE_PARAM_FIELDS,
                "hint": "参数只认 `name` / `value` / `unit`; `unit` 可省 (缺省 = 无量纲)",
            }),
        ));
    }
    let name = {
        let raw = object
            .get("name")
            .ok_or_else(|| missing("device.params[].name", "字符串"))?;
        raw.as_str()
            .ok_or_else(|| {
                Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    format!("`device.params[].name` 必须是字符串, 实际收到 {raw}"),
                )
            })?
            .to_owned()
    };
    let value = read_number(object, "value")?;
    let unit = match object.get("unit") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text.clone()),
        Some(other) => {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("`device.params[].unit` 必须是字符串或 null, 实际收到 {other}"),
            ));
        }
    };
    Ok(ParameterValue { name, value, unit })
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
fn reject_set_section_op_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    reject_unknown_keys(
        object,
        &SET_SECTION_OP_FIELDS,
        SET_SECTION_KIND,
        "unknownSetSectionOpField",
    )
}

/// 拒绝 `setSection` 载荷里 [`SET_SECTION_FIELDS`] 之外的键。
fn reject_set_section_fields(payload: &Map<String, Value>) -> Result<(), Fault> {
    reject_unknown_keys(
        payload,
        &SET_SECTION_FIELDS,
        SET_SECTION_KIND,
        "unknownSetSectionField",
    )
}

/// 读一个**可选**的非负整数（缺省 = 保留现值）。
fn read_optional_index(value: &Value, field: &str) -> Result<u64, Fault> {
    value.as_u64().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{SECTION_PAYLOAD_FIELD}.{field}` 必须是非负整数, 实际收到 {value}"),
            serde_json::json!({
                "field": format!("{SECTION_PAYLOAD_FIELD}.{field}"),
                "reason": "sectionTickMustBeUnsignedInteger",
                "received": value,
            }),
        )
    })
}

/// 解析 `setSection` 载荷的四个属性覆盖（照 [`parse_scene_patch`] 的形状）。
fn parse_section_patch(payload: &Map<String, Value>) -> Result<SectionPatch, Fault> {
    let name = match payload.get(SECTION_NAME_FIELD) {
        None => None,
        Some(value) => Some(read_non_empty_string(
            SECTION_PAYLOAD_FIELD,
            value,
            SECTION_NAME_FIELD,
            "sectionNameMustBeNonEmptyString",
        )?),
    };
    let start_tick = match payload.get(SECTION_START_FIELD) {
        None => None,
        Some(value) => Some(read_optional_index(value, SECTION_START_FIELD)?),
    };
    let end_tick = match payload.get(SECTION_END_FIELD) {
        None => None,
        Some(value) => Some(read_optional_index(value, SECTION_END_FIELD)?),
    };
    // ⚠ 区间约束 (`end_tick > start_tick`) 与 `ScenePatch` 的速度值域同一条纪律:
    // **不在这里**判 —— 模型 `SectionV3::validate` 是唯一事实源 (`ZeroDuration`)。
    let color = match payload.get(SECTION_COLOR_FIELD) {
        None => None,
        Some(Value::Null) => Some(None),
        Some(value) => Some(Some(read_non_empty_string(
            SECTION_PAYLOAD_FIELD,
            value,
            SECTION_COLOR_FIELD,
            "sectionColorMustBeNonEmptyString",
        )?)),
    };
    Ok(SectionPatch {
        name,
        start_tick,
        end_tick,
        color,
    })
}

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
            SCENE_PAYLOAD_FIELD,
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
            SCENE_PAYLOAD_FIELD,
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
fn read_non_empty_string(
    payload_field: &str,
    value: &Value,
    field: &str,
    reason: &str,
) -> Result<String, Fault> {
    let text = value.as_str().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{payload_field}.{field}` 必须是字符串, 实际收到 {value} \
                 (模型的 `SceneV3` / `SectionV3` 对文本字段没有校验, \
                 因此本层不接受 null 或别的类型)"
            ),
            serde_json::json!({
                "field": format!("{payload_field}.{field}"),
                "reason": format!("{reason}MustBeString"),
                "received": value,
            }),
        )
    })?;
    if text.is_empty() {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{payload_field}.{field}` 不得为空串"),
            serde_json::json!({
                "field": format!("{payload_field}.{field}"),
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
/// 词汇表与 `yeban_edit_automation` **同一份**（[`LaneKind`]）：已知但不可写的两个目标是
/// **响亮失败**，未知名字（别名 / 拼错）是另一条响亮失败，两条都不静默回退到默认值。
///
/// | 情形 | `data.reason` |
/// | :--- | :--- |
/// | `lane` 不是字符串 | `laneMustBeString` |
/// | 名字是另外两个自动化目标（`SendGain` / `Macro`） | `staticLaneNotApplicable` |
/// | 名字不在 [`LaneKind`] 的词汇表里（别名 / 拼错） | `unknownStaticLane` |
/// | `lane` 缺失 | 统一的缺字段错误（无 `data`） |
/// | `lane == "DeviceParam"` 而 `slotIndex` / `paramIndex` 缺失或不是非负整数 | 统一的缺字段 / 非负整数错误 |
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
    // 另外两个目标是**已知但此形态不适用**：说清楚它们各自为什么不适用，
    // 而不是笼统地报"未知名字"。
    match kind {
        LaneKind::TrackVolume => Ok(StaticLane::TrackVolume),
        LaneKind::TrackPan => Ok(StaticLane::TrackPan),
        LaneKind::DeviceParam => Ok(StaticLane::DeviceParam {
            // ⚠ 两个下标都**必填**（缺省不是 0）：缺一个却静默写第 0 槽 / 第 0 个参数
            // 会改到调用方没点名的东西。
            slot_index: read_usize(object, SET_PARAM_SLOT_FIELD)?,
            param_index: read_usize(object, SET_PARAM_PARAM_FIELD)?,
        }),
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
        LaneKind::Macro => Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{SET_PARAM_KIND}` 不接受目标 `{}`: 本形态写音轨静态音量/声相与设备参数; \
                 宏的静态写入在工具面没有通路",
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

/// 把 `setParam` 的设备参数寻址**交给 render 的唯一解析入口**，并把它回答的三种
/// 失败原样抬成领域错误。
///
/// - 越界 / 音轨不存在：复用 [`UnresolvedReason`] 的**逐字原因串**（⛔ 不新造词）；
/// - 名字不在离线渲染器消费的白名单里：复用 `ResolvedDeviceParam::unresolved_reason`
///   的**唯一字面量** `paramNotSupported` —— 写完导出不会变声的参数，必须在**提案期**
///   就响亮拒绝，而不是让调用方以为写进去了。
///
/// 返回值永远是**已确认可写**的 `StaticLane::DeviceParam`；`device_id` 只用于错误读数
/// （模型 `Op::SetParam` 的寻址仍是两个序数，与 `#263` 的解析口径一致）。
fn resolve_set_param_device(
    project: &YebanProjectV1,
    track_id: EntityId,
    slot_index: usize,
    param_index: usize,
) -> Result<StaticLane, Fault> {
    let track_text = track_id.to_canonical_string();
    let resolved = match resolve_device_param(project, track_id, slot_index, param_index) {
        Ok(resolved) => resolved,
        Err(reason) => {
            let (code, field, detail) = match reason {
                UnresolvedReason::TrackNotFound => (
                    ErrorCode::EntityNotFound,
                    "trackId",
                    format!("`{SET_PARAM_KIND}` 的设备参数寻址失败: 音轨 {track_text} 不存在"),
                ),
                UnresolvedReason::SlotOutOfRange => (
                    ErrorCode::OutOfRange,
                    SET_PARAM_SLOT_FIELD,
                    format!(
                        "`{SET_PARAM_KIND}` 的设备参数寻址越界: 音轨 {track_text} 的第 \
                         {slot_index} 槽不存在"
                    ),
                ),
                UnresolvedReason::ParamIndexOutOfRange => (
                    ErrorCode::OutOfRange,
                    SET_PARAM_PARAM_FIELD,
                    format!(
                        "`{SET_PARAM_KIND}` 的设备参数寻址越界: 音轨 {track_text} 第 \
                         {slot_index} 槽的设备没有第 {param_index} 个参数"
                    ),
                ),
            };
            return Err(Fault::domain_with_data(
                code,
                detail,
                serde_json::json!({
                    "field": field,
                    "reason": reason.as_str(),
                    "trackId": track_text,
                    "slotIndex": slot_index,
                    "paramIndex": param_index,
                }),
            ));
        }
    };
    if let Some(reason) = resolved.unresolved_reason() {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{SET_PARAM_KIND}` 不接受设备参数 `{}` (设备 {}): 该名字不在离线渲染器\
                 消费的参数名白名单里, 写进去导出的声音不会变",
                resolved.param_name,
                resolved.device_id.to_canonical_string()
            ),
            serde_json::json!({
                "field": SET_PARAM_PARAM_FIELD,
                "reason": reason,
                "trackId": track_text,
                "slotIndex": slot_index,
                "paramIndex": param_index,
                "deviceId": resolved.device_id.to_canonical_string(),
                "paramName": resolved.param_name,
                "hint": "参数名与单位见 `insertDevice` 的 `device.params.items`; \
                         大小写不敏感, 但必须是渲染器真的消费的名字",
            }),
        ));
    }
    Ok(StaticLane::DeviceParam {
        slot_index,
        param_index,
    })
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

/// 拒绝 `setTransportConfig` 里 [`TRANSPORT_CONFIG_FIELDS`] 之外的键。
///
/// 与 [`reject_track_flag_fields`] 同一条口径，但这里的三个字段**一个都不能省**：
/// `ADR-0001 D43` 规定走带配置的三个字段都是必需陈述 —— 缺一个就会"静默伪造一个
/// 作者从未做过的选择"（缺 `metronomeEnabled` 会静默关掉节拍器、缺 `countInBars`
/// 会静默取消预备拍、缺 `launchQuantization` 会静默换成一小节量化）。
fn reject_transport_config_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !TRANSPORT_CONFIG_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "走带配置操作里有不支持的键: {} (支持集合只有 {TRANSPORT_CONFIG_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownTransportConfigField",
            "unsupportedFields": unknown,
            "supportedTransportConfigFields": TRANSPORT_CONFIG_FIELDS,
        }),
    ))
}

/// 读一个**必填**布尔（⛔ 不做真假值强转：`1` / `"true"` 都不是布尔）。
fn read_required_bool(object: &Map<String, Value>, field: &str) -> Result<bool, Fault> {
    let raw = object.get(field).ok_or_else(|| missing(field, "布尔"))?;
    raw.as_bool().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是布尔, 实际收到 {raw} (不做真假值强转)"),
            serde_json::json!({
                "field": field,
                "reason": "valueMustBeBoolean",
                "received": raw,
            }),
        )
    })
}

/// 读一个**必填**的无符号字节（缺省/负/超 `255`/非整数一律响亮失败）。
/// 读一个**必填**的非负整数（`u64` 全域）。
///
/// ⚠ **为什么新增它**：⭐ 摆放长度是 `u64`，⭐ 而本文件此前只有 `read_required_u8`；
/// ⚠ ⭐ **它复用既有的通用 reason 名 `valueMustBeUnsignedInteger`**（⭐ 与
/// [`read_required_u8`] 同一条）⇒ ⭐ **⛔ 不新增 `reason` 名**、⭐ 故 `reason` 面的
/// 规范形式（⭐ 冻结摘要 ＋ 两张黄金表）**不受影响**。
/// ⚠ **只收 JSON 无符号整数**（⛔ 不做真假值强转：`"480"` 与 `480.0` 都不是整数）。
fn read_required_u64(object: &Map<String, Value>, field: &str) -> Result<u64, Fault> {
    let raw = object
        .get(field)
        .ok_or_else(|| missing(field, "非负整数"))?;
    raw.as_u64().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是非负整数, 实际收到 {raw}"),
            serde_json::json!({
                "field": field,
                "reason": "valueMustBeUnsignedInteger",
                "received": raw,
            }),
        )
    })
}

fn read_required_u8(object: &Map<String, Value>, field: &str) -> Result<u8, Fault> {
    let raw = object
        .get(field)
        .ok_or_else(|| missing(field, "0..=255 的整数"))?;
    let value = raw.as_u64().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是 0..=255 的整数, 实际收到 {raw}"),
            serde_json::json!({
                "field": field,
                "reason": "valueMustBeUnsignedInteger",
                "received": raw,
            }),
        )
    })?;
    u8::try_from(value).map_err(|_| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 超出 0..=255, 实际收到 {value}"),
            serde_json::json!({
                "field": field,
                "reason": "valueOutOfRange",
                "received": value,
                "max": u8::MAX,
            }),
        )
    })
}

/// 读一个**必填**的启动量化名（取值与线协议逐字相同，⛔ 不发明拼写）。
fn read_required_launch_quantization(
    object: &Map<String, Value>,
    field: &str,
) -> Result<LaunchQuantization, Fault> {
    let raw = object
        .get(field)
        .ok_or_else(|| missing(field, "启动量化名"))?;
    let text = raw.as_str().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是字符串, 实际收到 {raw}"),
            serde_json::json!({
                "field": field,
                "reason": "valueMustBeString",
                "received": raw,
            }),
        )
    })?;
    match text {
        "Off" => Ok(LaunchQuantization::Off),
        "Bar" => Ok(LaunchQuantization::Bar),
        "TwoBars" => Ok(LaunchQuantization::TwoBars),
        "FourBars" => Ok(LaunchQuantization::FourBars),
        "EightBars" => Ok(LaunchQuantization::EightBars),
        other => Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{field}` 不是规范的启动量化名: {other}                  (规范名与线协议逐字相同: Off / Bar / TwoBars / FourBars / EightBars)"
            ),
            serde_json::json!({
                "field": field,
                "reason": "unknownLaunchQuantization",
                "received": other,
                "allowed": ["Off", "Bar", "TwoBars", "FourBars", "EightBars"],
            }),
        )),
    }
}

/// 解析 `setTransportConfig` 的载荷对象（三个字段**全部必填**）。
///
/// 顺序刻意：**先拒绝未知键**，再逐个读必填字段 ⇒ 最危险的错键
/// （`metronome_enabled` / `countInBars` 拼错）在任何"值看起来没问题"的路径之前点名。
///
/// # Errors
///
/// - 未知键 → `unknownTransportConfigField`；
/// - 任一字段缺失 → `MISSING_PARAMETER`；
/// - `metronomeEnabled` 非布尔 / `countInBars` 非 `0..=255` 整数 /
///   `launchQuantization` 非规范名 → 各自的 `reason`，⛔ 绝不静默回退到默认值。
fn parse_transport_config(object: &Map<String, Value>) -> Result<TransportConfig, Fault> {
    reject_transport_config_fields(object)?;
    Ok(TransportConfig {
        metronome_enabled: read_required_bool(object, TRANSPORT_METRONOME_FIELD)?,
        count_in_bars: read_required_u8(object, TRANSPORT_COUNT_IN_FIELD)?,
        launch_quantization: read_required_launch_quantization(object, TRANSPORT_LAUNCH_FIELD)?,
    })
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
/// 拒绝 `setAsset` 操作对象里 [`SET_ASSET_FIELDS`] 之外的键。
///
/// ⚠ 最像"写对了"的错法是给 `byteLen` / `mediaKind` / `hash` —— ⛔ 那三个**不可改**，
/// 因此必须**响亮拒绝**（而不是静默忽略，让调用方以为改成了）。
///
/// # Errors
///
/// 出现 [`SET_ASSET_FIELDS`] 之外的键 → `INVALID_PARAMETER_RANGE`
/// （`data.reason = "unknownSetAssetField"`）。
fn reject_set_asset_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !SET_ASSET_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{SET_ASSET_KIND}` 操作里有不支持的键: {} (支持集合只有 {SET_ASSET_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownSetAssetField",
            "unsupportedFields": unknown,
            "supportedSetAssetFields": SET_ASSET_FIELDS,
            "hint": "可改的只有 `license` / `originalPath`; \
                     `hash`/`byteLen`/`mediaKind` 是**内容身份或内容事实**, ⛔ 不可改",
        }),
    ))
}

/// 拒绝 `removeAsset` 操作对象里 [`REMOVE_ASSET_FIELDS`] 之外的键。
///
/// 与 [`reject_remove_clip_fields`] 同一口径：最像"写对了"的错法是把寻址写成
/// 工具顶层的 `trackId`／`clipId`（那是别的形态的实参）—— 那会被静默忽略，
/// 而调用方以为资产已经取走了。
///
/// # Errors
///
/// 出现 `kind` / `assetHash` 之外的键 → `INVALID_PARAMETER_RANGE`
/// （`data.reason = "unknownRemoveAssetField"`）。
fn reject_remove_asset_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !REMOVE_ASSET_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{REMOVE_ASSET_KIND}` 操作里有不支持的键: {} \
             (支持集合只有 {REMOVE_ASSET_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownRemoveAssetField",
            "unsupportedFields": unknown,
            "supportedRemoveAssetFields": REMOVE_ASSET_FIELDS,
            "hint": "本形态**自带寻址**: 资产身份写在操作对象的 `assetHash` 上 \
                     (⛔ 不是工具顶层的 `trackId`/`clipId`)",
        }),
    ))
}

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
    let pitch = read_range(object, "pitch", 0, MIDI_PITCH_MAX)?;
    let duration_ticks = read_u64(object, "durationTicks")?;
    let velocity = match object.get("velocity") {
        None => yeban_model::music::DEFAULT_VELOCITY,
        Some(_) => read_range(object, "velocity", 0, MIDI_VELOCITY_MAX)?,
    };
    let mut note = MidiNote::new(id, start_tick, pitch, duration_ticks);
    note.velocity = velocity;
    note.probability = read_probability(object)?;
    note.ratchet = read_ratchet(object)?;
    note.micro_timing_ticks = read_micro_timing(object)?;
    // 先按**模型 `validate` 的前半段**判一次（音高/力度/概率/连击/微时序/时值）。
    // 模型 `validate` 的顺序是：… → 时值 → 滑音 → 弯音，而滑音/弯音的读取在下面 ⇒
    // 这一句让"时值早于滑音/弯音"这条优先级在两层一致（例如 `durationTicks:0` +
    // 一条非法曲线，模型报 `ZeroDuration`，本层也报它，⛔ 不是曲线错误）。
    note.validate()
        .map_err(|error| from_model("音符校验", &error))?;
    // 顺序照抄模型：滑音先、弯音后（`ADR-0012` D7）。
    note.slide = read_slide(object)?;
    note.pitch_bend_curve = read_pitch_bend_curve(object)?;
    // 读取器与模型同口径；再校验一次是让"`parse_note` 只产出合法音符"这条不变式
    // 不依赖两份实现永远同步（成本是一次 O(曲线点数) 的扫描，不在实时路径上）。
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
            // 模型里有、但工具面**仍然没有**写入通路的两个表现力字段: 说出来, 不要吞掉。
            // ⚠ `slide` / `pitchBendCurve` 已由 `#311c` 接上通路 ⇒ ⛔ 不许再列在这里
            // （把它们说成"不支持"就是假话）。
            "unsupportedNoteFields": ["syllable", "phonemes"],
            "hint": "未知键不静默忽略: 去掉它, 或改用 supportedNoteFields 里的字段",
        }),
    ))
}

/// 读一个**必填但可为 `null`** 的表现力字段（`setNoteExpression` 专用）。
///
/// ⚠ **缺键不是 `null`**：本形态是**整体替换**（照 `setTransportConfig`）⇒ 三个键都必须
/// 被**显式陈述**；"保留现值"没有可表达的形态（那需要三态载荷 `Option<Option<T>>`）。
/// `null` = 清空 = 模型的 `None`（必然触发／等价于 1／无偏移）。
fn read_required_nullable<T>(
    object: &Map<String, Value>,
    field: &str,
    read: impl FnOnce(&Map<String, Value>) -> Result<Option<T>, Fault>,
) -> Result<Option<T>, Fault> {
    match object.get(field) {
        None => Err(missing(field, "该字段的值或 null（缺键 ⛔ 不等于 null）")),
        Some(Value::Null) => Ok(None),
        Some(_) => read(object),
    }
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

/// 曲线形状错误的统一出口：`INVALID_PARAMETER_RANGE` + `field` / `index`。
///
/// 所有形状错误都是"载荷不是 `[tick, cents]`"这一族（⛔ 不是量越界），
/// 与 `ADR-0012` D7 的契约码分组一致。与 `read_probability` / `read_ratchet` 的
/// 形状错误同口径：报文说清"哪个字段、第几项、收到了什么"，`data` 只放可寻址的
/// 坐标（⛔ 不发明一个新的 `reason` 词 —— 那会把 `fault_vocabulary` 的冻结面
/// 为一条普通形状错误扩一次）。
fn pitch_bend_shape_error(field: &str, index: usize, detail: String) -> Fault {
    Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        detail,
        serde_json::json!({
            "field": field,
            "index": index,
        }),
    )
}

/// 读 `add.note.pitchBendCurve`（缺省 = `Vec::new()` = 无弯音）。
///
/// ⚠ 这是**可选**那一半：缺键 = 无弯音（`add` 的既有语义）。`setNotePitchShape` 的
/// **一个**弯音字段（`newPitchBendCurve`）是**必填**的，它走
/// [`read_required_pitch_bend_curve`] —— 两者共用同一个核心
/// [`read_pitch_bend_curve_value`]（⛔ 不写第二份解析器）。
fn read_pitch_bend_curve(object: &Map<String, Value>) -> Result<Vec<(u64, i16)>, Fault> {
    let Some(value) = object.get(PITCH_BEND_CURVE_FIELD) else {
        return Ok(Vec::new());
    };
    read_pitch_bend_curve_value(value, PITCH_BEND_CURVE_FIELD)
}

/// 读一个**必填**的弯音曲线字段（`field` 必须是 [`SET_NOTE_PITCH_SHAPE_FIELDS`] 的一员）。
///
/// ⚠ **缺键 ⛔ 不等于空数组**：本形态是整体替换（照 `setNoteExpression`）⇒ 每个值都
/// 必须被**显式陈述**。"无弯音"只有一种拼法 —— **空数组 `[]`**（模型的
/// `pitch_bend_curve` 是 `Vec`，⛔ 不是 `Option`）；`null` 在这里是**形状错误**，
/// 不是"清空"。
fn read_required_pitch_bend_curve(
    object: &Map<String, Value>,
    field: &str,
) -> Result<Vec<(u64, i16)>, Fault> {
    let value = object.get(field).ok_or_else(|| {
        missing(
            field,
            "[[tick, cents], ...]（空数组 = 无弯音；⛔ 不收 null）",
        )
    })?;
    read_pitch_bend_curve_value(value, field)
}

/// 弯音曲线解析的**唯一**核心：结构 → tick 升序 → `cents` 闭区间。
///
/// 形状是 `[tick, cents]` 二元组数组；语义与边界照抄
/// `docs/adr/ADR-0012-pitch-bend-curve-contract.md`（模型
/// [`crate::domain::notes`] 的 `MidiNote::validate` 是权威）：
///
/// 1. **结构**：数组、每项恰好两个整数、`tick >= 0` —— 不满足 ⇒
///    `INVALID_PARAMETER_RANGE`，报文与 `data.index` 指向出错的项；
/// 2. **tick 严格升序**（D3）—— 重复或倒退 ⇒ `INVALID_PARAMETER_RANGE`；
/// 3. **`cents` ∈ ±1200**（D2）—— 越界 ⇒ `OUT_OF_RANGE`。
///
/// ⚠ 2 与 3 是**两遍**扫描，不是逐点一起判：模型先扫全部窗口再扫全部 `cents`
/// （`ADR-0012` D7），逐点判会让 `[[0, 99999], [0, 0]]` 在本层报 `cents`、在模型
/// 报"不升序"—— 同一份输入两层说法不同。这里的顺序是**模型顺序**。
///
/// ⚠ **不对点数设上界**（决定 + 理由，刻意写在这里）：
/// - 解析不在实时路径上（这里跑在工具调用的调用方线程，音频回调用的是模型里那份
///   `Vec`），所以一次 O(n) 扫描不会造成 xrun；
/// - 传输层已经对**整个请求体**设了 1 MiB 上限（`transport::http::MAX_REQUEST_BODY_BYTES`）
///   ⇒ 曲线点数天然被它携带的请求大小夹住；
/// - 模型 `MidiNote::pitch_bend_curve` 自己**没有**长度上界；在门面层发明一个音乐上
///   的数字（例如"最多 4096 点"）会拒绝一条模型认为合法、作者也可能真的要的密集曲线，
///   且与 `ops` 数组本身"没有条数上限"的既有口径不一致。
fn read_pitch_bend_curve_value(value: &Value, field: &str) -> Result<Vec<(u64, i16)>, Fault> {
    let Some(pairs) = value.as_array() else {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是 [tick, cents] 数组, 实际收到 {value}"),
            serde_json::json!({
                "field": field,
            }),
        ));
    };
    // 第一遍：结构。`tick` 用 `as_u64` ⇒ 负数/小数/字符串在这里就落成"不是非负整数"。
    let mut raw: Vec<(u64, i64)> = Vec::with_capacity(pairs.len());
    for (index, pair) in pairs.iter().enumerate() {
        let Some(items) = pair.as_array() else {
            return Err(pitch_bend_shape_error(
                field,
                index,
                format!("`{field}[{index}]` 必须是 [tick, cents] 二元组, 实际收到 {pair}"),
            ));
        };
        if items.len() != 2 {
            return Err(pitch_bend_shape_error(
                field,
                index,
                format!(
                    "`{field}[{index}]` 必须恰好 2 个元素 [tick, cents], 实际 {} 个",
                    items.len()
                ),
            ));
        }
        let Some(tick) = items[0].as_u64() else {
            return Err(pitch_bend_shape_error(
                field,
                index,
                format!(
                    "`{field}[{index}][0]` 的 tick 必须是非负整数, 实际收到 {}",
                    items[0]
                ),
            ));
        };
        let Some(cents) = items[1].as_i64() else {
            return Err(pitch_bend_shape_error(
                field,
                index,
                format!(
                    "`{field}[{index}][1]` 的 cents 必须是整数, 实际收到 {}",
                    items[1]
                ),
            ));
        };
        raw.push((tick, cents));
    }
    // 第二遍：tick 严格升序（D3；顺序照抄模型 D7：升序先于 cents）。
    for index in 1..raw.len() {
        let (previous, _) = raw[index - 1];
        let (tick, _) = raw[index];
        if tick <= previous {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!(
                    "`{field}[{index}]` 的 tick 必须严格大于前一个 tick: {tick} 不大于 {previous}"
                ),
                serde_json::json!({
                    "field": field,
                    "index": index,
                    "previousTick": previous,
                    "tick": tick,
                }),
            ));
        }
    }
    // 第三遍：`cents` 闭区间 ±`PITCH_BEND_CENTS_MAX_ABS`（D2）。
    let bound = i64::from(PITCH_BEND_CENTS_MAX_ABS);
    let mut curve = Vec::with_capacity(raw.len());
    for (index, (tick, cents)) in raw.into_iter().enumerate() {
        if !(-bound..=bound).contains(&cents) {
            return Err(Fault::domain_with_data(
                ErrorCode::OutOfRange,
                format!(
                    "`{field}[{index}][1]` 的 cents 越界: {cents} 不在 {}..={}",
                    -bound, bound
                ),
                serde_json::json!({
                    "field": field,
                    "index": index,
                    "tick": tick,
                    "value": cents,
                    "min": -bound,
                    "max": bound,
                }),
            ));
        }
        // 区间已判（±1200 必然落进 i16），这里的 `unwrap_or` 只是不 panic 的构造。
        curve.push((
            tick,
            i16::try_from(cents).unwrap_or(PITCH_BEND_CENTS_MAX_ABS),
        ));
    }
    Ok(curve)
}

/// 读可选的 `note.slide`（缺省 = `None` = 无滑音）。
///
/// ⚠ 这是**可选**那一半（`add` 的既有语义）。`setNotePitchShape` 的**一个**滑音字段
/// （`newSlide`）是**必填**的，它走 [`read_required_nullable_slide`]；两者共用同一个
/// 核心 [`read_slide_value`]（⛔ 不写第二份解析器）。
fn read_slide(object: &Map<String, Value>) -> Result<Option<SlideConfig>, Fault> {
    let Some(value) = object.get(SLIDE_FIELD) else {
        return Ok(None);
    };
    read_slide_value(value, SLIDE_FIELD).map(Some)
}

/// 读一个**必填但可为 `null`** 的滑音字段（`null` = 清空 = 模型的 `None`）。
fn read_required_nullable_slide(
    object: &Map<String, Value>,
    field: &str,
) -> Result<Option<SlideConfig>, Fault> {
    match object.get(field) {
        None => Err(missing(
            field,
            "滑音对象或 null（null = 清空 = 无滑音；缺键 ⛔ 不等于 null）",
        )),
        Some(Value::Null) => Ok(None),
        Some(value) => read_slide_value(value, field).map(Some),
    }
}

/// 滑音对象解析的**唯一**核心。
///
/// 形状：`{ "durationTicks": <非零 tick>, "targetPitch": <0..=127>, "curve": <规范名?> }`。
///
/// - `durationTicks` / `targetPitch` **必填**（决定 + 理由）：没有目标的滑音不是滑音，
///   没有时值也无从排程；缺键 ⛔ 不折算成模型默认（`SlideConfig::default()` 的
///   `target_pitch = 0` 是"由调用方随后覆盖"的占位值，不是作者选择）。
/// - `curve` 可选，缺省 `Linear`（模型 `CurveType::default`）。
/// - 未知键**响亮拒绝**并列出 [`SLIDE_FIELDS`]（与 `note` 顶层同一口径）。
/// - 值域照抄模型 `SlideConfig::validate`，且**检查顺序一致**：先 `targetPitch`、
///   后 `durationTicks`（`ADR-0012` D9 之下 `SlideConfig` 是唯一语义源）。
/// - `curve` 的拼写/别名判定**复用** `crate::domain::automation::parse_curve`
///   （名字表 `extension_pure::CURVE_NAMES`），⛔ 不复制第二套。
fn read_slide_value(value: &Value, field: &str) -> Result<SlideConfig, Fault> {
    let Some(slide) = value.as_object() else {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{field}` 必须是对象 {{durationTicks, targetPitch, curve?}}, 实际收到 {value}"
            ),
            serde_json::json!({
                "field": field,
                "supportedSlideFields": SLIDE_FIELDS,
            }),
        ));
    };
    if let Some(unknown) = slide
        .keys()
        .find(|key| !SLIDE_FIELDS.contains(&key.as_str()))
    {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 不接受字段 `{unknown}`（不是可选项缺失, 而是拼写/不支持）"),
            serde_json::json!({
                "field": format!("{field}.{unknown}"),
                "supportedSlideFields": SLIDE_FIELDS,
                "hint": "未知键不静默忽略: 去掉它, 或改用 supportedSlideFields 里的字段",
            }),
        ));
    }
    let target_raw = slide.get(SLIDE_TARGET_PITCH_FIELD).ok_or_else(|| {
        missing(
            &format!("{field}.{SLIDE_TARGET_PITCH_FIELD}"),
            "整数 MIDI 音高 0..=127（滑音没有目标就不是滑音）",
        )
    })?;
    let duration_raw = slide
        .get(SLIDE_DURATION_FIELD)
        .ok_or_else(|| missing(&format!("{field}.{SLIDE_DURATION_FIELD}"), "非零整数 tick"))?;
    let target = target_raw.as_i64().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}.{SLIDE_TARGET_PITCH_FIELD}` 必须是整数, 实际收到 {target_raw}"),
        )
    })?;
    let duration = duration_raw.as_u64().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{field}.{SLIDE_DURATION_FIELD}` 必须是非负整数 tick, 实际收到 {duration_raw}"
            ),
        )
    })?;
    // 顺序照抄 `SlideConfig::validate`：先 targetPitch，再 durationTicks。
    if !(0..=i64::from(MIDI_PITCH_MAX)).contains(&target) {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!(
                "`{field}.{SLIDE_TARGET_PITCH_FIELD}` 越界: {target} 不在 0..={MIDI_PITCH_MAX}"
            ),
            serde_json::json!({
                "field": format!("{field}.{SLIDE_TARGET_PITCH_FIELD}"),
                "value": target,
                "min": 0,
                "max": MIDI_PITCH_MAX,
            }),
        ));
    }
    if duration == 0 {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("`{field}.{SLIDE_DURATION_FIELD}` 越界: 0 不在 1..=u64::MAX"),
            serde_json::json!({
                "field": format!("{field}.{SLIDE_DURATION_FIELD}"),
                "value": 0,
                "min": 1,
            }),
        ));
    }
    let curve = super::automation::parse_curve(slide.get(SLIDE_CURVE_FIELD), field)?;
    Ok(SlideConfig {
        duration_ticks: duration,
        // 区间已判（0..=127），`unwrap_or` 只是不 panic 的构造。
        target_pitch: u8::try_from(target).unwrap_or(MIDI_PITCH_MAX),
        curve,
    })
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
    if raw.is_nan() || raw < f64::from(PROBABILITY_MIN) || raw > f64::from(PROBABILITY_MAX) {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            // `{:?}` 对 f32 打印 `0.0` / `1.0`（Display 会打印 `0` / `1`）⇒ 报文逐字不变。
            format!(
                "`{PROBABILITY_FIELD}` 越界: {raw} 不在 {PROBABILITY_MIN:?}..={PROBABILITY_MAX:?}"
            ),
            serde_json::json!({
                "field": PROBABILITY_FIELD,
                "value": raw,
                "min": PROBABILITY_MIN,
                "max": PROBABILITY_MAX,
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
    // 音轨实体级形态（`removeTrack`）自带寻址，而且一个音符都不读：它不需要顶层
    // `trackId` / `clipId`（那两个是"目标片段所在的音轨"的寻址）。判据
    // `remove_track_does_not_need_midi_and_ignores_the_envelope` 钉住这条。
    // 空批不算"全是"（`parse_ops` 本来就挡空数组）。
    // 寻址自带的形态（`removeTrack` 与两个设备形态）不读工具顶层的 `trackId` /
    // `clipId`：它们的寻址在操作对象自己身上。判据
    // `remove_track_does_not_need_midi_and_ignores_the_envelope` 与
    // `device_ops_do_not_need_the_envelope_ids_or_midi` 钉住这条。
    // 空批不算"全是"（`parse_ops` 本来就挡空数组）。
    let self_addressed_only = NoteOp::all_self_addressed(ops);
    // ⭐ `#177` 坑④：片段池查找**只**在批里真的有形态读顶层 `clipId` 时发生。
    // 这正是"`clipId` 按 `ops[].kind` 条件必需"在编译层的另一半：不读它的形态
    // （`addTrack` / 路由级 / 段落级 / 场景级 / 资产级 / 文档级 / 音轨属性级）拿到的是
    // 占位身份 ⇒ 这里**不查池**，因此"省略 `clipId`"不会变成 `CLIP_NOT_FOUND`。
    // ⚠ 调用方（`domain::plan_edit_notes`）用**同一个**谓词决定要不要响亮拒绝缺参，
    // 因此"谁需要它"只有一份真相。
    let reads_clip = NoteOp::envelope_clip_required(ops);
    let track = if self_addressed_only {
        None
    } else {
        Some(
            project
                .track(track_id)
                .map_err(|error| from_model("音轨查找", &error))?,
        )
    };
    let entry = if self_addressed_only || !reads_clip {
        None
    } else {
        Some(project.clip_pool.get(clip_id).ok_or_else(|| {
            Fault::domain(ErrorCode::ClipNotFound, format!("片段不存在: {clip_id}"))
        })?)
    };
    // "必须是 MIDI 片段"这条断言只在**真的有音符操作**时成立：`setParam` 一个音符都不读。
    // 五个音符形态的调用因此逐字节等于旧行为（它们总是走到这条断言）。
    // ⚠ `#177` 之后 `entry` 只在读 `clipId` 的批里是 `Some`，而五个音符形态是它的
    // 子集 ⇒ 这条断言的可达集合逐条不变（`removeClip` 不读音符，故不受影响）。
    // ⭐ `#179` 把 `is_note_level` 收窄到**恰好**这四个（见 `NOTE_LEVEL_KINDS`）：
    // 六个属性级形态（名字 / 旁通 / 静音 / 颜色 / 时长）与 `setTrackName` 不再被算作
    // 音符级 —— 它们在 `#177` 之后本来就走不到这里（`entry == None`），本票只是让
    // 谓词与语义对齐（判据 `note_level_is_exactly_the_four_note_forms` 钉住）。
    if ops.iter().any(NoteOp::is_note_level)
        && entry.is_some_and(|entry| entry.content.notes().is_none())
    {
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
                if !(0..=i16::from(MIDI_PITCH_MAX)).contains(&shifted_pitch) {
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
            NoteOp::SetTiming {
                note_id,
                start_tick,
                duration_ticks,
            } => {
                // 撤销载荷来自**当前文档**（模型前置条件逐字段比较旧值）⇒ 本层
                // ⛔ 不采信调用方声明的旧状态。`0` 时值也交给模型拒绝，好让
                // 错误码只有一份真相（`ZeroDuration` ⇒ `OUT_OF_RANGE`）。
                let current = project
                    .note(clip_id, note_id)
                    .map_err(|error| from_model("音符查找", &error))?;
                Op::SetNoteTiming {
                    track_id: *track_id,
                    clip_id: *clip_id,
                    note_id: *note_id,
                    old_start_tick: current.start_tick,
                    old_duration_ticks: current.duration_ticks,
                    new_start_tick: *start_tick,
                    new_duration_ticks: *duration_ticks,
                }
            }
            NoteOp::SetNoteExpression {
                note_id,
                new_probability,
                new_ratchet,
                new_micro_timing_ticks,
            } => {
                // 三个旧值都来自**当前文档**（模型前置条件逐位/逐值比较）⇒ 本层
                // ⛔ 不采信调用方声明的旧状态。缺失的音符由 `project.note` 报
                // `NoteNotFound` ⇒ `ENTITY_NOT_FOUND`（既有错误，⛔ 新码）。
                let current = project
                    .note(clip_id, note_id)
                    .map_err(|error| from_model("音符查找", &error))?;
                Op::SetNoteExpression {
                    track_id: *track_id,
                    clip_id: *clip_id,
                    note_id: *note_id,
                    old_probability: current.probability,
                    old_ratchet: current.ratchet,
                    old_micro_timing_ticks: current.micro_timing_ticks,
                    new_probability: *new_probability,
                    new_ratchet: *new_ratchet,
                    new_micro_timing_ticks: *new_micro_timing_ticks,
                }
            }
            NoteOp::SetNotePitchShape {
                note_id,
                new_bend,
                new_slide,
            } => {
                // ⚠ 两个旧值**从当前文档读**（与 `setNoteExpression` 的三个旧值同一条
                // 纪律）：载荷里没有旧值字段，所以本层没有"调用方声明"可采信 ——
                // 模型 `OpStateMismatch` 仍然守着撤销/重放时的陈旧载荷（同一批里对同一
                // 音符连写两次时，第二次的前置条件会对不上）。
                let current = project
                    .note(clip_id, note_id)
                    .map_err(|error| from_model("音符查找", &error))?;
                Op::SetNotePitchShape {
                    track_id: *track_id,
                    clip_id: *clip_id,
                    note_id: *note_id,
                    old_bend: current.pitch_bend_curve.clone(),
                    old_slide: current.slide,
                    new_bend: new_bend.clone(),
                    new_slide: *new_slide,
                }
            }
            NoteOp::SetParam { lane, value } => {
                // 设备参数先过 render 的**唯一**解析入口（越界 / 名字不在白名单都在这里
                // 变成带 `reason` 的领域错误），再落到模型目标上。
                let lane = match *lane {
                    StaticLane::DeviceParam {
                        slot_index,
                        param_index,
                    } => resolve_set_param_device(project, *track_id, slot_index, param_index)?,
                    other => other,
                };
                let target = lane.target(*track_id);
                // 撤销载荷来自**唯一**的静态值入口 (`AutomationTarget::static_value`)：
                // 本层不自己读 `track.volume_db` / `track.pan` / 设备参数（那会是第二份真相）。
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
                // `track` 在这里必然是 `Some`：本形态**不是**音轨实体级，因此上面那条
                // "要不要查顶层 `trackId`"的分支一定取了音轨（见 `track_entity_only`）。
                flag.compile(*track_id, flag.read(track.expect("非音轨实体级必查音轨")), *value)
            }
            NoteOp::SetTempo { new_bpm, .. } => {
                // 撤销载荷来自**当前文档**（模型的前提条件读同一个字段、且是**逐位**比较）
                // ⇒ 本层不采信调用方声明的旧状态。
                //
                // ⚠ 区间与有限性**在这里提前拒**（模型自己也会拒同一区间）：让越界在
                // **编译期**就带上 `reason`，而不是在提案模拟那一步冒出一个泛化消息。
                if !new_bpm.is_finite() || !(yeban_model::project::MIN_BPM..=yeban_model::project::MAX_BPM).contains(new_bpm) {
                    return Err(Fault::domain_with_data(
                        ErrorCode::InvalidParameterRange,
                        format!(
                            "速度必须在 {}..={} BPM 且有限, 实际收到 {new_bpm}",
                            yeban_model::project::MIN_BPM,
                            yeban_model::project::MAX_BPM
                        ),
                        serde_json::json!({
                            "reason": "bpmOutOfRange",
                            "received": new_bpm,
                            "min": yeban_model::project::MIN_BPM,
                            "max": yeban_model::project::MAX_BPM,
                        }),
                    ));
                }
                // ⚠ 当前速度已是**合法文档**的一部分（`YebanProjectV1::validate` 早已
                // 钉住同一区间）⇒ 这里只搬现值，⛔ 不再重复校验它。
                Op::SetTempo {
                    old_bpm: project.bpm,
                    new_bpm: *new_bpm,
                }
            }
            NoteOp::SetTrackName {
                track_id, new_name, ..
            } => {
                // 撤销载荷来自**当前文档**（模型只查"陈旧载荷"这一条，逐字节比较）。
                let track = project.track(track_id).map_err(|_| {
                    Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!("工程里没有身份 {track_id} 的音轨, 没有名字可以改"),
                        serde_json::json!({
                            "trackId": track_id.to_canonical_string(),
                            "reason": "trackNotFound",
                            "hint": "音轨的身份由 `yeban_query_project` 的 `tracks` 字段报出",
                        }),
                    )
                })?;
                Op::SetTrackName {
                    track_id: *track_id,
                    old_name: track.name.clone(),
                    new_name: new_name.clone(),
                }
            }
            NoteOp::SetClipPlacementDuration {
                track_id,
                placement_id,
                new_duration_ticks,
                ..
            } => {
                // 撤销载荷来自**当前文档**（模型只查"陈旧载荷"这一条，值比较）。
                let track = project.track(track_id).map_err(|_| {
                    Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!("工程里没有身份 {track_id} 的音轨, 没有摆放可以改长度"),
                        serde_json::json!({
                            "trackId": track_id.to_canonical_string(),
                            "reason": "placementTrackNotFound",
                            "hint": "音轨的身份由 `yeban_query_project` 的 `tracks` 字段报出",
                        }),
                    )
                })?;
                let placement = track.clips.get(placement_id).ok_or_else(|| {
                    Fault::domain_with_data(
                        ErrorCode::ClipNotFound,
                        format!("音轨 {track_id} 上没有身份 {placement_id} 的摆放"),
                        serde_json::json!({
                            "trackId": track_id.to_canonical_string(),
                            "placementId": placement_id.to_canonical_string(),
                            "reason": "clipPlacementNotFound",
                            "hint": "摆放身份由 `yeban_query_project` 的 `tracks[].clips` 报出",
                        }),
                    )
                })?;
                Op::SetClipPlacementDuration {
                    track_id: *track_id,
                    placement_id: *placement_id,
                    old_duration_ticks: placement.duration_ticks,
                    new_duration_ticks: *new_duration_ticks,
                }
            }
            NoteOp::SetTrackColor {
                track_id,
                new_color,
                ..
            } => {
                // 撤销载荷来自**当前文档**（模型只查"陈旧载荷"这一条，`Option` 比对）。
                let track = project.track(track_id).map_err(|_| {
                    Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!("工程里没有身份 {track_id} 的音轨, 没有色标可以改"),
                        serde_json::json!({
                            "trackId": track_id.to_canonical_string(),
                            "reason": "trackNotFound",
                            "hint": "音轨的身份由 `yeban_query_project` 的 `tracks` 字段报出",
                        }),
                    )
                })?;
                Op::SetTrackColor {
                    track_id: *track_id,
                    old_color: track.color.clone(),
                    new_color: new_color.clone(),
                }
            }
            NoteOp::SetTrackSampler {
                track_id,
                new_sampler,
                ..
            } => {
                // 撤销载荷来自**当前文档**（模型的前置条件只查"陈旧载荷"这一条，
                // `Option<SamplerBinding>` 逐字节比对）。
                let track = project.track(track_id).map_err(|_| {
                    Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!("工程里没有身份 {track_id} 的音轨, 没有采样器可以绑"),
                        serde_json::json!({
                            "trackId": track_id.to_canonical_string(),
                            "reason": "trackNotFound",
                            "hint": "音轨的身份由 `yeban_query_project` 的 `tracks` 字段报出",
                        }),
                    )
                })?;
                Op::SetTrackSampler {
                    track_id: *track_id,
                    old_sampler: track.sampler.clone(),
                    new_sampler: new_sampler.clone(),
                }
            }
            NoteOp::SetClipPlacementMuted {
                track_id,
                placement_id,
                new_muted,
                ..
            } => {
                // 撤销载荷来自**当前文档**（模型只查"陈旧载荷"这一条，值比较）。
                // ⚠ 模型本实体有**专用**错误 `ClipPlacementNotFound`（与 `MoveClipPlacement` 同一条）。
                let track = project.track(track_id).map_err(|_| {
                    Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!("工程里没有身份 {track_id} 的音轨, 没有摆放可以切换静音"),
                        serde_json::json!({
                            "trackId": track_id.to_canonical_string(),
                            "reason": "placementTrackNotFound",
                            "hint": "音轨的身份由 `yeban_query_project` 的 `tracks` 字段报出",
                        }),
                    )
                })?;
                let placement = track.clips.get(placement_id).ok_or_else(|| {
                    Fault::domain_with_data(
                        ErrorCode::ClipNotFound,
                        format!("音轨 {track_id} 上没有身份 {placement_id} 的摆放"),
                        serde_json::json!({
                            "trackId": track_id.to_canonical_string(),
                            "placementId": placement_id.to_canonical_string(),
                            "reason": "clipPlacementNotFound",
                            "hint": "摆放身份由 `yeban_query_project` 的 `tracks[].clips` 报出",
                        }),
                    )
                })?;
                Op::SetClipPlacementMuted {
                    track_id: *track_id,
                    placement_id: *placement_id,
                    old_muted: placement.muted,
                    new_muted: *new_muted,
                }
            }
            NoteOp::SetClipPlacementLoop {
                track_id,
                placement_id,
                new_loop_config,
                ..
            } => {
                // 撤销载荷来自**当前文档**（模型只查"陈旧载荷"这一条，`LoopConfig`
                // 是 `PartialEq + Eq` ⇒ 逐字段值比较）。⛔ 不信调用方 —— 线上没有
                // 旧值键可给（`reject_set_clip_placement_loop_fields` 会拒）。
                let track = project.track(track_id).map_err(|_| {
                    Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!("工程里没有身份 {track_id} 的音轨, 没有摆放可以改循环"),
                        serde_json::json!({
                            "trackId": track_id.to_canonical_string(),
                            "reason": "placementTrackNotFound",
                            "hint": "音轨的身份由 `yeban_query_project` 的 `tracks` 字段报出",
                        }),
                    )
                })?;
                let placement = track.clips.get(placement_id).ok_or_else(|| {
                    Fault::domain_with_data(
                        ErrorCode::ClipNotFound,
                        format!("音轨 {track_id} 上没有身份 {placement_id} 的摆放"),
                        serde_json::json!({
                            "trackId": track_id.to_canonical_string(),
                            "placementId": placement_id.to_canonical_string(),
                            "reason": "clipPlacementNotFound",
                            "hint": "摆放身份由 `yeban_query_project` 的 `tracks[].clips` 报出",
                        }),
                    )
                })?;
                Op::SetClipPlacementLoop {
                    track_id: *track_id,
                    placement_id: *placement_id,
                    old_loop_config: placement.loop_config,
                    new_loop_config: *new_loop_config,
                }
            }
            NoteOp::SetDeviceBypassed {
                track_id,
                device_id,
                new_bypassed,
                ..
            } => {
                // 撤销载荷来自**当前文档**（模型只查"陈旧载荷"这一条，值比较）。
                // ⚠ 与 [`Self::RemoveDevice`] / [`Self::SetDeviceName`] **同一形状**。
                let track = project.track(track_id).map_err(|_| {
                    Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!("工程里没有身份 {track_id} 的音轨, 没有设备链可以切换旁通"),
                        serde_json::json!({
                            "trackId": track_id.to_canonical_string(),
                            "reason": "deviceTrackNotFound",
                            "hint": "音轨的身份由 `yeban_query_project` 的 `tracks` 字段报出",
                        }),
                    )
                })?;
                let device = track
                    .devices
                    .iter()
                    .find(|device| device.id == *device_id)
                    .ok_or_else(|| {
                        Fault::domain_with_data(
                            ErrorCode::EntityNotFound,
                            format!("音轨 {track_id} 的设备链里没有身份 {device_id} 的设备"),
                            serde_json::json!({
                                "trackId": track_id.to_canonical_string(),
                                "deviceId": device_id.to_canonical_string(),
                                "reason": "deviceNotFound",
                                "hint": "`yeban_query_engine_state` 的 \
                                         `tracks[].devices[]` 逐槽报出 `deviceId`",
                            }),
                        )
                    })?;
                Op::SetDeviceBypassed {
                    track_id: *track_id,
                    device_id: *device_id,
                    old_bypassed: device.bypassed,
                    new_bypassed: *new_bypassed,
                }
            }
            NoteOp::SetDeviceName {
                track_id,
                device_id,
                new_name,
                ..
            } => {
                // 撤销载荷来自**当前文档**（模型只查"陈旧载荷"这一条，逐字节比较）。
                // ⚠ 与 [`Self::RemoveDevice`] **同一形状**：按 id 找、缺失报 `deviceNotFound`。
                let track = project.track(track_id).map_err(|_| {
                    Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!("工程里没有身份 {track_id} 的音轨, 没有设备链可以改名"),
                        serde_json::json!({
                            "trackId": track_id.to_canonical_string(),
                            "reason": "deviceTrackNotFound",
                            "hint": "音轨的身份由 `yeban_query_project` 的 `tracks` 字段报出",
                        }),
                    )
                })?;
                let device = track
                    .devices
                    .iter()
                    .find(|device| device.id == *device_id)
                    .ok_or_else(|| {
                        Fault::domain_with_data(
                            ErrorCode::EntityNotFound,
                            format!("音轨 {track_id} 的设备链里没有身份 {device_id} 的设备"),
                            serde_json::json!({
                                "trackId": track_id.to_canonical_string(),
                                "deviceId": device_id.to_canonical_string(),
                                "reason": "deviceNotFound",
                                "hint": "`yeban_query_engine_state` 的 \
                                         `tracks[].devices[]` 逐槽报出 `deviceId`",
                            }),
                        )
                    })?;
                Op::SetDeviceName {
                    track_id: *track_id,
                    device_id: *device_id,
                    old_name: device.name.clone(),
                    new_name: new_name.clone(),
                }
            }
            NoteOp::SetClipName {
                clip_id, new_name, ..
            } => {
                // 撤销载荷来自**当前文档**（模型只查"陈旧载荷"这一条，逐字节比较）。
                let clip = project.clip_pool.get(clip_id).ok_or_else(|| {
                    Fault::domain(ErrorCode::ClipNotFound, format!("片段不存在: {clip_id}"))
                })?;
                Op::SetClipName {
                    clip_id: *clip_id,
                    old_name: clip.name.clone(),
                    new_name: new_name.clone(),
                }
            }
            NoteOp::SetClipGain {
                clip_id,
                new_gain_db,
                ..
            } => {
                // 撤销载荷来自**当前文档**，⛔ 不信调用方声明。
                let clip = project.clip_pool.get(clip_id).ok_or_else(|| {
                    Fault::domain(ErrorCode::ClipNotFound, format!("片段不存在: {clip_id}"))
                })?;
                // ⚠ 内容类型门必须**早于**构造 `Op`：MIDI 片段**没有** `gain_db` 可读，
                // `old_gain_db` 无从取得 ⇒ ⛔ 用占位 0.0 冒充（那会让撤销把 MIDI 片段
                // 的"假旧值"写回一个不存在的字段）；⭐ 这是「必须早」的机械理由，
                // 不是礼貌。模型侧同一道门报 `ClipContentKindMismatch`。
                let ClipContent::Audio { gain_db, .. } = &clip.content else {
                    return Err(Fault::domain(
                        ErrorCode::ClipNotFound,
                        format!("片段 {clip_id} 不是音频片段, 没有 gain_db 可改"),
                    ));
                };
                Op::SetClipGain {
                    clip_id: *clip_id,
                    old_gain_db: *gain_db,
                    new_gain_db: *new_gain_db,
                }
            }
            NoteOp::SetAuthor { new_author, .. } => {
                // 撤销载荷来自**当前文档**（模型只查"陈旧载荷"这一条，且是**逐字节**比较）。
                // ⚠ 模型**没有**空串/长度校验 ⇒ 本层也⛔ 不发明（`new_author` 原样收下）。
                Op::SetAuthor {
                    old_author: project.author.clone(),
                    new_author: new_author.clone(),
                }
            }
            NoteOp::SetTitle { new_title, .. } => {
                // 撤销载荷来自**当前文档**（照 `setAsset` 的 `old_asset` 口径，⛔ 不信调用方）。
                // 模型只查"陈旧载荷"这一条，且是**逐字节**比较。
                Op::SetTitle {
                    old_title: project.title.clone(),
                    new_title: new_title.clone(),
                }
            }
            NoteOp::SetTransportConfig { new_config } => {
                // 撤销载荷来自**当前文档**（模型 `apply` 的前置条件读同一个字段）；
                // 本层不自己写 `Op::invert`（那是模型的唯一事实源）。
                // ⚠ 本形态**不是**音轨实体级、也不寻址音轨 ⇒ 只用 `project.transport`。
                Op::SetTransportConfig {
                    old_config: project.transport,
                    new_config: *new_config,
                }
            }
            NoteOp::SetLane { edit } => {
                let target = edit.spec.target(*track_id);
                // 撤销载荷来自**当前文档**：模型 `SetAutomationLane` 的前置条件要求
                // `old_lane` 等于文档现值（`RemoveAutomationLane` 同理要求
                // `previous_lane`），因此这里不采信调用方声明的"旧状态"。
                let current = track
                    .expect("非音轨实体级必查音轨")
                    .automation_lanes
                    .get(&target)
                    .cloned();
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
                let lane = track
                    .expect("非音轨实体级必查音轨")
                    .automation_lanes
                    .get(&target)
                    .ok_or_else(|| {
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
                    previous_clip: entry.expect("非音轨实体级必查片段池").clone(),
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
            NoteOp::ConnectRouting {
                edge_id,
                source_node,
                destination_node,
                kind,
                gain_db,
            } => {
                // 载荷**自带**：模型 `ConnectRouting` 有全部字段，逆操作是
                // `DisconnectRouting`（它也自带 `previous_edge`）⇒ 本层不读文档现值。
                //
                // 两条提前拒绝（与 `removeRoutingNode` 的 `routingNodeNotFound` 同一
                // 纪律）：① 两端都必须在 `nodes` 里；② `edge.id` 不在 `edges` 里。
                // ⚠ "增益有限"由模型 `edge.validate()` 报；"节点类型与 edgeKind 匹配"
                // **模型没有**这条 ⇒ 本层不发明。
                for endpoint in [*source_node, *destination_node] {
                    if !project.routing_graph.nodes.contains(&endpoint) {
                        return Err(Fault::domain_with_data(
                            ErrorCode::EntityNotFound,
                            format!(
                                "工程的 routing_graph.nodes 里没有身份 {endpoint} 的节点, \
                                 不能连边"
                            ),
                            serde_json::json!({
                                "nodeId": endpoint.to_canonical_string(),
                                "reason": "routingNodeNotFound",
                                "hint": "连边之前两端都必须已经在 `nodes` 里 —— 先调 \
                                         `addRoutingNode` 把节点加进图; 节点身份由 \
                                         `yeban_query_project` 的 `routing_graph.nodes` 报出",
                            }),
                        ));
                    }
                }
                if project.routing_graph.edges.contains_key(edge_id) {
                    return Err(Fault::domain_with_data(
                        ErrorCode::Conflict,
                        format!("工程的 routing_graph.edges 里已经有身份 {edge_id} 的边"),
                        serde_json::json!({
                            "edgeId": edge_id.to_canonical_string(),
                            "reason": "routingEdgeAlreadyExists",
                            "hint": "建边是**幂等拒绝**的 (模型 `ConnectRouting` 的前置条件\
                                     是 `edge.id` 不在 `edges` 里); 要改一条已有边的增益用 \
                                     `setRoutingGain`, 要取走它用 `disconnectRouting`",
                        }),
                    ));
                }
                Op::ConnectRouting {
                    edge: RoutingEdge {
                        id: *edge_id,
                        source_node: *source_node,
                        destination_node: *destination_node,
                        kind: *kind,
                        gain_db: *gain_db,
                    },
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
            NoteOp::AddTrack { track } => {
                // 载荷就是整条 `TrackV3`（四个集合与两个可选字段已是空形状），
                // 因此这里**没有**撤销载荷要读：`AddTrack` 的逆操作是 `RemoveTrack`
                // （它自带整条 `previous_track`），模型那两条互为逆。
                //
                // 一条提前拒绝（与 `removeTrack` 的 `trackNotFound` 同一纪律 ——
                // 让"身份打错 / 重复"在**编译期**就带上 `reason` 与 `hint`，而不是在
                // 提案模拟那一步冒出一个泛化消息）：这个身份**必须不在** `tracks` 里。
                if project.tracks.contains_key(&track.id) {
                    return Err(Fault::domain_with_data(
                        ErrorCode::Conflict,
                        format!("工程的 tracks 里已经有身份 {} 的音轨", track.id),
                        serde_json::json!({
                            "trackId": track.id.to_canonical_string(),
                            "reason": "trackAlreadyExists",
                            "hint": "新建音轨是**幂等拒绝**的 (模型 `AddTrack` 的前置条件\
                                     是身份不在 `tracks` 里); 已有的音轨身份由 \
                                     `yeban_query_project` 的 `tracks` 报出。若你想改一条\
                                     已有音轨, 用 `setParam` (音量/声相) / `setTrackMute` / \
                                     `setTrackSolo` / `setTrackName` / `setTrackColor` / \
                                     `setTrackSampler`",
                        }),
                    ));
                }
                Op::AddTrack {
                    track: track.clone(),
                }
            }
            NoteOp::AddRoutingNode { node_id } => {
                // 载荷是**空**的（模型 `AddRoutingNode` 只有节点身份，且它与
                // `RemoveRoutingNode` 互为逆操作），因此这里没有撤销载荷可读。
                //
                // 一条提前拒绝（与 `removeRoutingNode` 的 `routingNodeNotFound` 同一
                // 纪律 —— 让"身份打错"在**编译期**就带上 `reason` 与 `hint`，而不是在
                // 提案模拟那一步冒出一个泛化消息）：这个身份**必须不在** `nodes` 里。
                // 模型那条前置条件在 `Op::validate` 里报 `DuplicateEntityId`
                // （`error.rs` 把它映射到 `CONFLICT`），但那条路径只在提案模拟里跑、
                // 消息里没有本层的 `reason` / `hint`。
                //
                // ⚠ **⛔ 不**在这里要求"这个身份是一条真实音轨"：模型只做数据层校验
                // （`RoutingGraph::validate` 查节点重复、以及每条边的两端都在 `nodes`
                // 里），它⛔ 不要求节点能对应到 `project.tracks` —— 而同一族的先例是
                // `addTrack` 只把音轨加进 `tracks`、**不**自动把身份加进 `nodes`。
                // 本层照抄模型口径（见 `NoteOp::AddRoutingNode` 的文档）。
                if project.routing_graph.nodes.contains(node_id) {
                    return Err(Fault::domain_with_data(
                        ErrorCode::Conflict,
                        format!("工程的 routing_graph.nodes 里已经有身份 {node_id} 的节点"),
                        serde_json::json!({
                            "nodeId": node_id.to_canonical_string(),
                            "reason": "routingNodeAlreadyExists",
                            "hint": "新建节点是**幂等拒绝**的 (模型 `AddRoutingNode` 的前置\
                                     条件是身份不在 `nodes` 里); 节点身份由 \
                                     `yeban_query_project` 的 `routing_graph.nodes` 报出。\
                                     若你以为它是一条还没进图的音轨, 请先确认 `tracks` 里\
                                     确实有它 —— 模型**不**要求节点对应真实音轨",
                        }),
                    ));
                }
                Op::AddRoutingNode { node: *node_id }
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
            NoteOp::SetAsset { asset_hash, patch } => {
                // 撤销载荷来自**当前文档**（模型 `SetAsset` 的前置条件要求
                // `old_asset` 逐字段等于文档现值）⇒ 本层不采信调用方声明。
                //
                // ⚠⭐ **步 ③ 的许可判定必须同样适用于这条通路**：否则新增的"改"通路
                // 会**绕过**刚建的工程层判定 ⇒ 一个非商用许可可以经 `setAsset` 写进去。
                if let Some(license) = &patch.license {
                    require_allowed_licence(license)?;
                }
                let current = project.assets.get(asset_hash).cloned().ok_or_else(|| {
                    Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!("工程的 assets 索引里没有身份 {} 的资产", asset_hash.as_str()),
                        serde_json::json!({
                            "assetHash": asset_hash.as_str(),
                            "reason": "assetNotFound",
                            "hint": "资产身份由 `yeban_query_project` 的 `assets` 字段报出; \
                                     要**声明**一条新资产请用 `yeban_import_audio`",
                        }),
                    )
                })?;
                Op::SetAsset {
                    asset_hash: asset_hash.clone(),
                    old_asset: Some(current.clone()),
                    new_asset: patch.apply_to(&current),
                }
            }
            NoteOp::RemoveAsset { asset_hash } => {
                // 撤销载荷来自**当前文档**（模型 `RemoveAsset` 的 `validate` 要求
                // `previous_asset` 逐字段等于文档现值）⇒ 本层不采信调用方声明。
                //
                // 两条提前拒绝（与 `removeClip` 的 `CLIP_NOT_FOUND` / 模型
                // `ClipInUse` 同一纪律 —— 让"哈希打错"与"资产还在被引用"在**编译期**
                // 就带上 `reason`，而不是在提案模拟那一步冒出一个泛化消息）：
                //
                // 1. 该索引项必须在 `project.assets` 里（否则 `assetNotFound`）。
                // 2. ⚠ **没有任何 `clip_pool` 条目引用它**（否则 `assetInUse`）。
                //    `project.assets` 是 `ClipContent::Audio` 的许可留痕 ⇒ 取走一个
                //    仍被引用的哈希会留下**指向不存在资产的片段**，而
                //    `YebanProjectV1::validate` **不查**这条（实测：`project.rs` 里
                //    `assets.get` / `assets.contains_key` 零命中）⇒ ⛔ 模型不会替我们拦。
                let current = project.assets.get(asset_hash).cloned().ok_or_else(|| {
                    Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!("工程的 assets 索引里没有身份 {} 的资产", asset_hash.as_str()),
                        serde_json::json!({
                            "assetHash": asset_hash.as_str(),
                            "reason": "assetNotFound",
                            "hint": "资产身份由 `yeban_query_project` 的 `assets` 字段报出",
                        }),
                    )
                })?;
                let referencing: Vec<String> = project
                    .clip_pool
                    .values()
                    .filter(|entry| match &entry.content {
                        ClipContent::Audio { asset, .. } => asset == asset_hash,
                        ClipContent::Midi { .. } => false,
                    })
                    .map(|entry| entry.id.to_canonical_string())
                    .collect();
                if !referencing.is_empty() {
                    return Err(Fault::domain_with_data(
                        ErrorCode::Conflict,
                        format!(
                            "资产 {} 还被 {} 条片段引用, 不能取走它的索引项",
                            asset_hash.as_str(),
                            referencing.len()
                        ),
                        serde_json::json!({
                            "assetHash": asset_hash.as_str(),
                            "reason": "assetInUse",
                            "clipCount": referencing.len(),
                            "clipIds": referencing,
                            "hint": "先取走引用它的片段 (`removeClip`), 再取走这条索引项                                      (与 `removeClip` 要求先取走摆放是同一条顺序纪律)",
                        }),
                    ));
                }
                Op::RemoveAsset {
                    asset_hash: asset_hash.clone(),
                    previous_asset: current,
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
            NoteOp::SetSection {
                section_id,
                create,
                patch,
            } => {
                // 与 `setScene` **逐条同口径**: 新建与更新由**显式**的 `create` 区分
                // (⛔ 不靠「文档里没有这个身份就当作新建」—— 那会让一个打错的
                // `sectionId` 静默建出一个新段落); 两条前置条件都在这里提前报
                // (模型 `Op::validate` 只在提案模拟那一步跑, 消息里没有本层的
                // `reason` / `hint`)。
                let current = project.sections.get(section_id).cloned();
                if *create {
                    if current.is_some() {
                        return Err(Fault::domain_with_data(
                            ErrorCode::Conflict,
                            format!(
                                "工程里已经有身份 {section_id} 的段落, \
                                 `{SECTION_CREATE_FIELD}: true` 不覆盖既有段落"
                            ),
                            serde_json::json!({
                                "sectionId": section_id.to_canonical_string(),
                                "reason": "sectionAlreadyExists",
                                "hint": "把 `create` 去掉就是一次普通更新; \
                                         要新建请换一个 `sectionId`",
                            }),
                        ));
                    }
                } else if current.is_none() {
                    return Err(Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!(
                            "工程里没有身份 {section_id} 的段落, 没有段落可以更新 \
                             (新建请给 `{SECTION_CREATE_FIELD}: true`)"
                        ),
                        serde_json::json!({
                            "sectionId": section_id.to_canonical_string(),
                            "reason": "sectionNotFound",
                            "hint": "段落的身份由 `yeban_query_project` 的 `sections` \
                                     字段报出",
                        }),
                    ));
                }
                // 撤销载荷来自**当前文档**: 模型 `SetSection` 的前置条件要求
                // `old_section` **逐字段**等于文档现值 (`None` = 新建), 因此本层不采信
                // 调用方声明的旧状态。基础 = 文档现值; 新建时用 `SectionV3` 的**空形状**
                // (区间两端都是 0 ⇒ 不给合法区间时模型的 `ZeroDuration` 会拒;
                // `name` 由 `create` 分支强制要求, 因此不可能落下一个空名)。
                let base = current.clone().unwrap_or(SectionV3 {
                    id: *section_id,
                    name: String::new(),
                    start_tick: 0,
                    end_tick: 0,
                    color: None,
                });
                Op::SetSection {
                    section_id: *section_id,
                    old_section: current,
                    new_section: patch.apply_to(&base),
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
            NoteOp::RemoveTrack { track_id: target } => {
                // 撤销载荷来自**当前文档**的**整条**音轨：模型 `RemoveTrack` 的前置条件
                // 要求 `previous_track` 逐字段等于文档现值（`TrackV3` 不是 `Copy`，
                // 因此这里是克隆），本层因此不采信调用方声明的旧状态，也不接受调用方
                // 送来的载荷（`reject_remove_track_fields` 只认 `kind` 与 `trackId`）。
                //
                // 两条提前拒绝（与 `removeRoutingNode` 的主总线拒绝同一纪律 —— 让身份
                // 打错 / 取走主总线在**编译期**就带上 `reason` 与 `hint`，而不是在提案
                // 模拟那一步冒出一个泛化消息）：
                //
                // 1. 主总线**不能**被取走：`Op::validate` 用它自己的错误词
                //    （`OpStateMismatch`）报这条模型不变量，而 `YebanProjectV1::validate`
                //    要求"有音轨时主总线在 `tracks` 里且 `kind == Master`" —— 主总线被
                //    取走之后工程立刻不合法，且 `master_bus_track_id` 会悬空。
                //    这里用一个说得通的契约码（`CONFLICT`）报同一个结论。
                // 2. 音轨必须**存在**（模型报 `TrackNotFound`）。
                //
                // "还有谁来引用它"（别的音轨的 `folder_id` / 路由图里的边与节点）
                // **不在这里**复制：`YebanProjectV1::validate` 会自己报
                // `TrackNotFound`（悬空的 `folder_id`）。工具面没有任何形态能设置
                // `folder_id`（模型 `Op` 全集里没有这个变体），而路由图只存身份、
                // 不存音轨对象 —— 取走一条仍在图里的音轨**不会**让图失效。
                let current = project.tracks.get(target);
                if let Some(master) = current
                    && *target == project.master_bus_track_id
                    && !target.is_nil()
                {
                    return Err(Fault::domain_with_data(
                        ErrorCode::Conflict,
                        format!(
                            "身份 {target} 是工程的主总线 (名字 `{}`), 模型要求主总线留在 \
                             `tracks` 里, 不能取走",
                            master.name
                        ),
                        serde_json::json!({
                            "trackId": target.to_canonical_string(),
                            "masterBusTrackId": project.master_bus_track_id.to_canonical_string(),
                            "reason": "masterBusTrackCannotBeRemoved",
                            "hint": "主总线是唯一的声学出口 (`PdcPlan::compute` 以\
                                     「主总线在 tracks 里且 kind == Master」为前置条件); \
                                     本形态只取走**非**主总线音轨",
                        }),
                    ));
                }
                let previous_track = current.cloned().ok_or_else(|| {
                    Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!("工程里没有身份 {target} 的音轨, 没有音轨可以取走"),
                        serde_json::json!({
                            "trackId": target.to_canonical_string(),
                            "reason": "trackNotFound",
                            "hint": "音轨的身份由 `yeban_query_project` 的 `tracks` 字段报出",
                        }),
                    )
                })?;
                Op::RemoveTrack {
                    track_id: *target,
                    previous_track,
                }
            }
            NoteOp::InsertDevice {
                track_id: target,
                slot_index,
                device,
            } => {
                // 撤销载荷由模型从**这一条** op 自己反转（`Op::invert` 把 `InsertDevice`
                // 换成 `RemoveDevice`，两个载荷本来就都在插入这一条上），因此本层
                // 不需要从文档读任何"旧状态"；要读的是**前**置条件。
                let track = project.track(target).map_err(|_| {
                    Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!("工程里没有身份 {target} 的音轨, 没有设备链可以插入"),
                        serde_json::json!({
                            "trackId": target.to_canonical_string(),
                            "reason": "deviceTrackNotFound",
                            "hint": "音轨的身份由 `yeban_query_project` 的 `tracks` 字段报出",
                        }),
                    )
                })?;
                // 槽位：缺省 = 追加到链尾。给出的值 `> len` 是**响亮**失败，不夹紧
                // —— 夹紧会让"插到第 7 槽"变成"插到链尾"，而 PDC 的关键路径按链上
                // 位置算，位置错了相位就错了。
                let resolved = slot_index.unwrap_or(track.devices.len());
                if resolved > track.devices.len() {
                    return Err(Fault::domain_with_data(
                        ErrorCode::OutOfRange,
                        format!(
                            "`{DEVICE_SLOT_FIELD}` 越界: {resolved} 不在 0..={} (链长)",
                            track.devices.len()
                        ),
                        serde_json::json!({
                            "field": DEVICE_SLOT_FIELD,
                            "reason": "deviceSlotOutOfRange",
                            "value": resolved,
                            "len": track.devices.len(),
                            "hint": "缺省 (不给 `slotIndex`) 就等于追加到链尾 (= len)",
                        }),
                    ));
                }
                // 同一条音轨上设备身份不得重复（模型 `Op::validate` 的
                // `DuplicateEntityId`）。提前报，让调用方拿到 `reason` 与现成的位置。
                if let Some(existing) = track.devices.iter().position(|it| it.id == device.id) {
                    return Err(Fault::domain_with_data(
                        ErrorCode::Conflict,
                        format!(
                            "音轨 {target} 的第 {existing} 槽已经有身份 {} 的设备",
                            device.id
                        ),
                        serde_json::json!({
                            "trackId": target.to_canonical_string(),
                            "deviceId": device.id.to_canonical_string(),
                            "existingSlotIndex": existing,
                            "reason": "deviceIdAlreadyInTrack",
                            "hint": "⭐ `setParam` 现在可以写目标 `DeviceParam`: \
                                     `{\"kind\":\"setParam\",\"lane\":\"DeviceParam\",\
                                     \"slotIndex\":<槽位>,\"paramIndex\":<参数下标>,\"value\":<数字>}`; \
                                     要改现有设备的静态参数就用它, ⛔ 不必先 `removeDevice` 再重插; \
                                     想**换一台设备**才需要先 `removeDevice`(那会丢掉整台设备的其它参数)",
                        }),
                    ));
                }
                // 参数值的有限性（模型 `DeviceDefinition::validate` 的
                // `NonFiniteValue`）。解析期已经逐条判过收窄后的 f32，
                // 这里再判一次是因为**设备对象也可能由别的路径构造**，
                // 而这条闸门是"进提案之前"的最后一道。
                device.validate().map_err(|error| {
                    Fault::domain_with_data(
                        ErrorCode::InvalidParameterRange,
                        format!("设备定义不合法: {error}"),
                        serde_json::json!({
                            "reason": "invalidDeviceDefinition",
                            "deviceId": device.id.to_canonical_string(),
                        }),
                    )
                })?;
                Op::InsertDevice {
                    track_id: *target,
                    slot_index: resolved,
                    device: device.clone(),
                }
            }
            NoteOp::RemoveDevice {
                track_id: target,
                device_id,
            } => {
                let track = project.track(target).map_err(|_| {
                    Fault::domain_with_data(
                        ErrorCode::EntityNotFound,
                        format!("工程里没有身份 {target} 的音轨, 没有设备链可以取走"),
                        serde_json::json!({
                            "trackId": target.to_canonical_string(),
                            "reason": "deviceTrackNotFound",
                            "hint": "音轨的身份由 `yeban_query_project` 的 `tracks` 字段报出",
                        }),
                    )
                })?;
                // 撤销载荷来自**当前文档**的那一台设备：模型 `RemoveDevice` 的前置条件
                // 要求 `previous_device` 逐字段等于文档现值，因此本层不采信调用方
                // 声明的旧状态（`reject_remove_device_fields` 也不接受 `device` 键）。
                let (slot_index, previous_device) = track
                    .devices
                    .iter()
                    .enumerate()
                    .find(|(_slot, device)| device.id == *device_id)
                    .map(|(slot, device)| (slot, device.clone()))
                    .ok_or_else(|| {
                        Fault::domain_with_data(
                            ErrorCode::EntityNotFound,
                            format!("音轨 {target} 的设备链里没有身份 {device_id} 的设备"),
                            serde_json::json!({
                                "trackId": target.to_canonical_string(),
                                "deviceId": device_id.to_canonical_string(),
                                "reason": "deviceNotFound",
                                "hint": "`yeban_query_engine_state` 的 \
                                         `tracks[].devices[]` 逐槽报出 `deviceId`",
                            }),
                        )
                    })?;
                Op::RemoveDevice {
                    track_id: *target,
                    slot_index,
                    previous_device,
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

/// 拒绝**音轨实体级形态**（`removeTrack`）与另外几路混用。
///
/// 音轨实体级取走把一条**音轨本身**（连带它自己的 `clips` 摆放与 `automation_lanes`）
/// 取走。它必须**单独**出现在 `ops` 里，也不能和 `placement` 同给 —— 那两件事都在
/// 同一个对象上做相反的动作（摘掉它 / 在它上面摆片段），让它们同时在场等于让调用方
/// 猜哪一个先发生。所有规则都在**建提案之前**响亮拒绝：一个自相矛盾的批走到模型层，
/// 撞到的会是一个说不清是哪条 op 的错（第一条 `RemoveTrack` 真的摘掉音轨之后，
/// 第二条对同一条轨的 `old_*` 必然与"调用前的文档"不符 ⇒ `OpStateMismatch`）。
///
/// 与 [`reject_remove_clip_conflicts`] 同一条纪律：只在真的出现 `removeTrack` 时才做事
/// （其余形态逐字节等于接线之前的行为）。
///
/// # Errors
///
/// - `removeTrack` 与别的 `kind` 同给 → `INVALID_PARAMETER_RANGE`
///   （`data.reason = "removeTrackTakesNoOtherOps"`，`data.opKinds` = 本次真给的 `kind`
///   列表，`data.trackId` = 被取走的那条音轨）；
/// - `removeTrack` 与 `placement` 同给 → `INVALID_PARAMETER_RANGE`
///   （`data.reason = "removeTrackIsNotPlacement"`）。
pub fn reject_remove_track_conflicts(ops: &[NoteOp], placement_present: bool) -> Result<(), Fault> {
    let removed = ops.iter().find_map(|op| match op {
        NoteOp::RemoveTrack { track_id } => Some(*track_id),
        _ => None,
    });
    let Some(track_id) = removed else {
        return Ok(());
    };
    if ops.len() > 1 {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "`removeTrack` 只能**单独**出现: 同一次调用不能既取走一条音轨、又对它\
             (或对别的东西) 做别的编辑",
            serde_json::json!({
                "reason": "removeTrackTakesNoOtherOps",
                "trackId": track_id.to_canonical_string(),
                "opKinds": ops.iter().map(NoteOp::kind_name).collect::<Vec<_>>(),
                "hint": "取走音轨请单独一次调用; 要先把这条轨上的摆放/泳道清掉, 也请各自\
                         单独成一次调用 (批内每条的撤销载荷都从调用前的文档读)",
            }),
        ));
    }
    if placement_present {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "`removeTrack` 与 `placement` 不能同给: 前者取走整条音轨, 后者在音轨上动摆放",
            serde_json::json!({
                "reason": "removeTrackIsNotPlacement",
                "trackId": track_id.to_canonical_string(),
                "hint": "先 `placement.kind: \"remove\"` 取走摆放, 再单独一次调用取走音轨",
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
///                "placementId": "<可选 26 字符 ULID>", "muted": false,
///                "loopEnabled": true, "loopStartTick": 0, "loopEndTick": 960}}
/// ```
///
/// 七个内容键全部**可选**：`startTick` 缺省 0；`durationTicks` 缺省由片段内容推导；
/// `placementId` 缺省由 [`placement_label`] 确定性派生；`muted` 缺省 `false`；
/// `loopEnabled` 缺省 `false`；`loopStartTick` 缺省 0；`loopEndTick` 在启用时
/// 缺省 = 片段内容长度（推不出时 = `durationTicks`）。
///
/// ⭐ `#241`：`loopEnabled: true` + `durationTicks = N * (loopEndTick - loopStartTick)`
/// ⇒ 渲染器把循环区间**真的展开 N 遍**（语义唯一一份在 `crate::domain::render`
/// 的 `ClipPlacement::loop_repeat`）。给区间却不给 `loopEnabled: true` 是自相矛盾的载荷
/// （`placementFieldNotApplicable`：字段已知、但在这个形态下不适用），响亮拒绝。
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
    // 片段内容自己的长度（MIDI = 最后一个音符的结束 tick）；音频片段/空片段 = `None`。
    // 它有两个用途：`durationTicks` 的缺省、以及 `loopEndTick` 的缺省。
    let content_extent = entry.content.notes().and_then(|notes| {
        notes
            .values()
            .map(|note| note.start_tick.saturating_add(note.duration_ticks))
            .max()
    });
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
        None => match content_extent {
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
    // ⭐ `#241`：循环旋钮。只搬字面值进 `LoopConfig`；展开语义在渲染层唯一一份。
    let loop_enabled = read_optional_bool(object, LOOP_ENABLED_FIELD)?.unwrap_or(false);
    let loop_start_tick = read_optional_u64(object, LOOP_START_TICK_FIELD)?.unwrap_or(0);
    let loop_end_tick = read_optional_u64(object, LOOP_END_TICK_FIELD)?;
    if !loop_enabled
        && (object.contains_key(LOOP_START_TICK_FIELD) || object.contains_key(LOOP_END_TICK_FIELD))
    {
        // 与 `reject_placement_fields` 同一句：字段**已知**，但在"循环关闭"这个
        // 形态下不适用。复用既有 `reason`（不新增一个词表外的值）。
        let reason = "placementFieldNotApplicable";
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{PLACEMENT_FIELD}.{LOOP_START_TICK_FIELD}` / `{LOOP_END_TICK_FIELD}` 只在 \
                 `{LOOP_ENABLED_FIELD}: true` 时有意义"
            ),
            serde_json::json!({
                "reason": reason,
                "field": format!("{PLACEMENT_FIELD}.{LOOP_START_TICK_FIELD}"),
                "placementKind": PLACEMENT_KIND_ADD,
                "supportedPlacementFields": [
                    "startTick",
                    "durationTicks",
                    "placementId",
                    "muted",
                ],
                "hint": "给 `loopEnabled: true` 与循环区间; 关闭循环就别给区间（不静默丢弃）",
            }),
        ));
    }
    let loop_end_tick = match loop_end_tick {
        Some(value) => value,
        // 启用但没给终点 ⇒ 缺省为片段内容长度（"整段循环"）；推不出就取摆放跨度
        // （此时区间恰好覆盖 ⇒ 单遍，除非调用方另外把 `durationTicks` 拉长）。
        None if loop_enabled => content_extent.unwrap_or(duration_ticks),
        None => 0,
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
        // ⭐ `#241`：渲染器**真的**展开它（`#246` 起由 `yeban_model::ClipPlacement::loop_repeat` 决定），所以这个旋钮可以暴露。
        loop_config: LoopConfig {
            enabled: loop_enabled,
            start_tick: loop_start_tick,
            end_tick: loop_end_tick,
        },
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
/// | 键不在**任何**形态的词表里（拼错 / 根本不支持，例如 `repeatCount`） | `unknownPlacementField` |
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
    use yeban_model::project::{MAX_BPM, MIN_BPM};
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

    /// 取故障的 `reason`（公开的 `to_tool_response` 面 ⇒ ⛔ 不碰内部字段）。
    fn reason_of(fault: &Fault) -> Option<String> {
        let response = fault.to_tool_response()?;
        // 响应形状：`{"error":{"data":{"reason": ...}}}`。
        response
            .pointer("/error/data/reason")
            .and_then(Value::as_str)
            .map(str::to_owned)
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

    /// 另外两个自动化目标名、别名与设备参数缺失下标都是**响亮失败**：不静默回退、不猜。
    #[test]
    fn set_param_rejects_non_static_lanes_and_aliases() {
        for (payload, expected_reason) in [
            (
                serde_json::json!([{"kind": "setParam", "lane": "SendGain", "value": 0.0}]),
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
            // `DeviceParam` 的两个下标都**必填**（缺省不是 0）。
            (
                serde_json::json!([{"kind": "setParam", "lane": "DeviceParam", "value": 0.0}]),
                None,
            ),
            (
                serde_json::json!([{"kind": "setParam", "lane": "DeviceParam",
                                    "slotIndex": 0, "value": 0.0}]),
                None,
            ),
            (
                serde_json::json!([{"kind": "setParam", "lane": "DeviceParam",
                                    "slotIndex": "0", "paramIndex": 0, "value": 0.0}]),
                None,
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

    /// ⭐ `#265`：`setParam` 的 `DeviceParam` 目标真的**写进模型**，且：
    ///
    /// - 撤销载荷 `old_val` 来自当前文档（不是调用方声明）；
    /// - 名字不在 `yeban_render::inserts::resolve_device_param` 的白名单里 ⇒
    ///   `INVALID_PARAMETER_RANGE` + `reason == "paramNotSupported"`（唯一字面量在 render）；
    /// - `slotIndex` / `paramIndex` 越界 ⇒ `OUT_OF_RANGE` + render 的
    ///   `slotOutOfRange` / `paramIndexOutOfRange`（⛔ 本层不新造词）。
    ///
    /// 夹具：`filled_project` 的 lead 第 0 槽是 `InternalInstrument`，
    /// `params[0] = cutoff`（不在白名单）、`params[1] = resonance`（在白名单）。
    ///
    /// 注入（实测红）：删掉 `compile` 里对 `resolve_set_param_device` 的调用 ⇒
    /// 不支持的名字也会被接受（`paramNotSupported` 断言红）；把 `old_val` 改成
    /// 调用方声明 ⇒ 撤销载荷断言红；把 `slot_index` / `param_index` 的解析写死成 0
    /// ⇒ 越界断言与写后读数红。
    #[test]
    fn set_param_writes_a_whitelisted_device_parameter_and_rejects_the_rest() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let device_id = project.track(&track_id).expect("音轨").devices[0].id;
        let before = project.track(&track_id).expect("音轨").devices[0].params[1].value;
        assert_eq!(before, 0.35, "夹具的 resonance 初值");

        let ops = parse_ops(&serde_json::json!([
            {"kind": "setParam", "lane": "DeviceParam",
             "slotIndex": 0, "paramIndex": 1, "value": 0.8}
        ]))
        .expect("规范形状必须被接受");
        assert!(!ops[0].is_note_level(), "设备参数写入不读音符");
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("白名单内的名字");
        match &compiled[0] {
            Op::SetParam {
                target:
                    AutomationTarget::DeviceParam {
                        track_id: target_track,
                        slot_index,
                        param_index,
                    },
                old_val,
                new_val,
            } => {
                assert_eq!(*target_track, track_id);
                assert_eq!((*slot_index, *param_index), (0, 1));
                assert_eq!(*old_val, before, "撤销载荷必须来自当前文档");
                assert_eq!(*new_val, 0.8);
            }
            other => panic!("应当是 DeviceParam 的 SetParam: {other:?}"),
        }
        assert_eq!(
            project.track(&track_id).expect("音轨").devices[0].id,
            device_id,
            "目标仍是同一台设备"
        );

        Op::Batch {
            ops: compiled.clone(),
            description: "setParam(DeviceParam)".to_owned(),
        }
        .apply(&mut project)
        .expect("施加");
        assert_eq!(
            project.track(&track_id).expect("音轨").devices[0].params[1].value,
            0.8,
            "模型里的设备参数必须真的变了"
        );
        for op in compiled.iter().rev() {
            op.apply_inverse(&mut project).expect("逆操作");
        }
        assert_eq!(
            project.track(&track_id).expect("音轨").devices[0].params[1].value,
            before,
            "逆操作必须逐字节回原值"
        );

        // ① 名字不在白名单（`cutoff` 是乐器旋钮，不是渲染器消费的 `cutoff_hz`）。
        let ops = parse_ops(&serde_json::json!([
            {"kind": "setParam", "lane": "DeviceParam",
             "slotIndex": 0, "paramIndex": 0, "value": 0.5}
        ]))
        .expect("形状合法");
        let fault =
            compile(&project, &track_id, &clip_id, &ops).expect_err("不在白名单必须响亮拒绝");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let data = lane_fault_data(&fault);
        assert_eq!(data["reason"], "paramNotSupported", "{data}");
        assert_eq!(data["paramName"], "cutoff", "{data}");
        assert_eq!(data["deviceId"], device_id.to_canonical_string(), "{data}");

        // ② `slotIndex` 越界（render 的 `UnresolvedReason` 原样抬上来）。
        let ops = parse_ops(&serde_json::json!([
            {"kind": "setParam", "lane": "DeviceParam",
             "slotIndex": 99, "paramIndex": 0, "value": 0.5}
        ]))
        .expect("形状合法");
        let fault = compile(&project, &track_id, &clip_id, &ops).expect_err("槽位越界必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
        assert_eq!(lane_fault_data(&fault)["reason"], "slotOutOfRange");

        // ③ `paramIndex` 越界。
        let ops = parse_ops(&serde_json::json!([
            {"kind": "setParam", "lane": "DeviceParam",
             "slotIndex": 0, "paramIndex": 99, "value": 0.5}
        ]))
        .expect("形状合法");
        let fault = compile(&project, &track_id, &clip_id, &ops).expect_err("参数下标越界必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
        assert_eq!(lane_fault_data(&fault)["reason"], "paramIndexOutOfRange");
    }

    /// ⭐ `#179`：`is_note_level()` **恰好**是七个音符形态（`#334` 起）。
    ///
    /// 六个**属性级**形态（`setClipName` / `setDeviceName` / `setDeviceBypassed` /
    /// `setClipPlacementMuted` / `setTrackColor` / `setClipPlacementDuration`）改的是
    /// 名字 / 旁通 / 静音 / 颜色 / 时长，**一个音符都不读**；`setTrackName` 同理。
    /// 它们此前落在反选清单的**漏网**一侧（`is_note_level() == true`），本票改成与
    /// `NOTE_LEVEL_KINDS` 同一份真相：**只有**音符形态为真。
    ///
    /// 注入（实测红）：把谓词改回"反选清单"（即删掉本票的收窄）⇒ 七条 `!is_note_level`
    /// 全红；把 `Add`/`Delete`/`Move`/`Velocity`/`SetTiming` 任一从正选里去掉 ⇒ 对应的
    /// `is_note_level` 断言红。
    #[test]
    fn note_level_is_exactly_the_seven_note_forms() {
        let ulid = "01J8ZQ00000000000000000999";
        // 七个音符形态：解析后 `is_note_level()` 必须为真。
        let note_forms = [
            (
                "add",
                serde_json::json!({"kind": "add", "note": {"startTick": 0, "pitch": 60, "durationTicks": 480}}),
            ),
            (
                "delete",
                serde_json::json!({"kind": "delete", "noteId": ulid}),
            ),
            (
                "move",
                serde_json::json!({"kind": "move", "noteId": ulid, "deltaTick": 0, "deltaPitch": 0}),
            ),
            (
                "velocity",
                serde_json::json!({"kind": "velocity", "noteId": ulid, "velocity": 64}),
            ),
            (
                "setNoteTiming",
                serde_json::json!({"kind": "setNoteTiming", "noteId": ulid, "startTick": 0, "durationTicks": 480}),
            ),
            (
                "setNoteExpression",
                serde_json::json!({"kind": "setNoteExpression", "noteId": ulid, "probability": null, "ratchet": null, "microTimingTicks": null}),
            ),
            (
                "setNotePitchShape",
                serde_json::json!({"kind": "setNotePitchShape", "noteId": ulid, "newPitchBendCurve": [], "newSlide": null}),
            ),
        ];
        for (kind, payload) in note_forms {
            let ops = parse_ops(&serde_json::json!([payload]))
                .unwrap_or_else(|fault| panic!("`{kind}` 必须能解析: {fault:?}"));
            assert_eq!(ops.len(), 1);
            assert_eq!(ops[0].kind_name(), kind);
            assert!(ops[0].is_note_level(), "`{kind}` 必须**仍然**是音符级");
        }
        // 六个属性级形态 + `setTrackName`：`is_note_level()` 必须为假。
        let not_note = [
            (
                "setClipName",
                serde_json::json!({"kind": "setClipName", "clipId": ulid, "name": "A"}),
            ),
            (
                "setDeviceName",
                serde_json::json!({"kind": "setDeviceName", "trackId": ulid, "deviceId": ulid, "name": "A"}),
            ),
            (
                "setDeviceBypassed",
                serde_json::json!({"kind": "setDeviceBypassed", "trackId": ulid, "deviceId": ulid, "bypassed": true}),
            ),
            (
                "setClipPlacementMuted",
                serde_json::json!({"kind": "setClipPlacementMuted", "trackId": ulid, "placementId": ulid, "muted": true}),
            ),
            (
                "setTrackColor",
                serde_json::json!({"kind": "setTrackColor", "trackId": ulid, "color": "#22AA88"}),
            ),
            (
                "setClipPlacementDuration",
                serde_json::json!({"kind": "setClipPlacementDuration", "trackId": ulid, "placementId": ulid, "durationTicks": 960}),
            ),
            (
                "setTrackName",
                serde_json::json!({"kind": "setTrackName", "trackId": ulid, "name": "A"}),
            ),
        ];
        for (kind, payload) in not_note {
            let ops = parse_ops(&serde_json::json!([payload]))
                .unwrap_or_else(|fault| panic!("`{kind}` 必须能解析: {fault:?}"));
            assert_eq!(ops.len(), 1);
            assert_eq!(ops[0].kind_name(), kind);
            assert!(
                !ops[0].is_note_level(),
                "`{kind}` 改的是属性（不读音符）, ⛔ 不得算作音符级"
            );
        }
        // 发布清单本身：逐字 + 恰好七个（⛔ 不是常量自比）。
        assert_eq!(
            NOTE_LEVEL_KINDS,
            [
                "add",
                "delete",
                "move",
                "velocity",
                "setNoteTiming",
                "setNoteExpression",
                "setNotePitchShape"
            ],
            "音符级清单必须逐字是这七个"
        );
        assert_eq!(NOTE_LEVEL_KINDS.len(), 7);
        // 七个音符形态必然是顶层 `clipId` 的消费者（清单之间的包含关系）。
        for kind in NOTE_LEVEL_KINDS {
            assert!(
                ENVELOPE_CLIP_KINDS.contains(kind),
                "`{kind}` 在音符级清单里 ⇒ 必然也在 ENVELOPE_CLIP_KINDS 里"
            );
        }
    }

    /// `setNoteTiming`：解析新值、拒绝未知键，并从**当前文档**读旧值。
    ///
    /// 注入（实测红）：把 `compile` 的 `old_start_tick` 换成调用方可声明的值（或 `0`）
    /// ⇒ ③ 的逐字段断言红；删掉 `reject_set_note_timing_fields` ⇒ ② 红。
    #[test]
    fn set_note_timing_reads_the_old_values_from_the_document() {
        let project = filled_project();
        let (track, clip) = lead_clip(&project);
        let note = *project.clip_pool[&clip]
            .content
            .notes()
            .expect("MIDI 片段")
            .keys()
            .next()
            .expect("样本音符");
        let current = project.note(&clip, &note).expect("音符存在").clone();

        // ① 规范形状能解析，并回环到同一个 kind。
        let ops = parse_ops(&serde_json::json!([{
            "kind": "setNoteTiming",
            "noteId": note,
            "startTick": current.start_tick + 960,
            "durationTicks": current.duration_ticks + 240,
        }]))
        .expect("规范形状必须被接受");
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].kind_name(), SET_NOTE_TIMING_KIND);
        assert!(ops[0].is_note_level());
        assert!(ops[0].reads_envelope_clip());

        // ② 旧值不是调用方声明的一部分：写进来就是响亮失败。
        let fault = parse_ops(&serde_json::json!([{
            "kind": "setNoteTiming",
            "noteId": note,
            "startTick": 0,
            "durationTicks": 480,
            "oldStartTick": current.start_tick,
        }]))
        .expect_err("未知键必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "unknownSetNoteTimingField"
        );

        // ③ 编译出的旧值来自当前文档（⛔ 不是调用方声明）。
        let compiled = compile(&project, &track, &clip, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::SetNoteTiming {
                old_start_tick,
                old_duration_ticks,
                new_start_tick,
                new_duration_ticks,
                ..
            } => {
                assert_eq!(*old_start_tick, current.start_tick);
                assert_eq!(*old_duration_ticks, current.duration_ticks);
                assert_eq!(*new_start_tick, current.start_tick + 960);
                assert_eq!(*new_duration_ticks, current.duration_ticks + 240);
            }
            other => panic!("必须编译成 `Op::SetNoteTiming`, 实际 {other:?}"),
        }
    }

    /// `setNoteExpression`：解析 → 编译到**正是** `Op::SetNoteExpression` → 三个旧值来自
    /// **当前文档** → 一步撤销逐字节复原（`#329`）。
    ///
    /// 判据钉七件事：① 三个键**全必填**、`null` = 清空（模型的一等 `None`）；
    /// ② 未知键（含把旧值写进载荷的 `oldProbability`）响亮拒绝；③ 缺键 ⛔ 不等于 `null`
    /// （整体替换没有"保留现值"语义）；④ 三个旧值都来自当前文档 —— 先把文档推到
    /// **任何占位常量都不同**的状态，作为这条判据的牙；⑤ 越界在**解析期**报
    /// `OUT_OF_RANGE` 并逐字点名 `field`；⑥ 音符不存在 ⇒ `ENTITY_NOT_FOUND`（既有码）；
    /// ⑦ 撤销逐字节复原。
    ///
    /// 注入（实测红）：① 把 `old_probability` 写死成 `None` ⇒ ④ 的断言红；
    /// ② 删掉 `reject_set_note_expression_fields` 的调用 ⇒ ② 的未知键判据红。
    #[test]
    fn set_note_expression_reads_old_values_from_the_document_and_inverts() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let note_id = *project.clip_pool[&clip_id]
            .content
            .notes()
            .expect("MIDI 片段")
            .keys()
            .next()
            .expect("样本音符");
        // 先把文档推到**任何占位常量都不同**的状态 ⇒ "旧值真从文档读"才有牙。
        {
            let notes = project
                .clip_pool
                .get_mut(&clip_id)
                .expect("样本片段")
                .content
                .notes_mut()
                .expect("MIDI 片段");
            let note = notes.get_mut(&note_id).expect("样本音符");
            note.probability = Some(0.25);
            note.ratchet = Some(3);
            note.micro_timing_ticks = Some(-17);
        }
        let bytes_before = serde_json::to_vec(&project).expect("序列化");

        // ① 规范形状：三个键全给。
        let ops = parse_ops(&serde_json::json!([{
            "kind": SET_NOTE_EXPRESSION_KIND,
            "noteId": note_id,
            "probability": 0.75,
            "ratchet": 2,
            "microTimingTicks": 12,
        }]))
        .expect("规范形状必须被接受");
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].kind_name(), SET_NOTE_EXPRESSION_KIND);
        assert!(ops[0].is_note_level(), "它读/写一个音符");
        assert!(ops[0].reads_envelope_clip(), "它用顶层 `clipId` 定位片段");

        // ② 旧值不是载荷的一部分：写进来就响亮失败。
        let fault = parse_ops(&serde_json::json!([{
            "kind": SET_NOTE_EXPRESSION_KIND,
            "noteId": note_id,
            "probability": 0.75,
            "ratchet": 2,
            "microTimingTicks": 12,
            "oldProbability": 0.25,
        }]))
        .expect_err("未知键必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "unknownSetNoteExpressionField"
        );

        // ③ 缺键 ⛔ 不等于 null。
        let missing = parse_ops(&serde_json::json!([{
            "kind": SET_NOTE_EXPRESSION_KIND,
            "noteId": note_id,
            "probability": null,
            "ratchet": null,
        }]))
        .expect_err("缺键必须被拒");
        assert_eq!(
            missing.domain_code(),
            Some(ErrorCode::InvalidParameterRange)
        );

        // ④ 编译到**正是**该变体，三个旧值来自当前文档。
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::SetNoteExpression {
                old_probability,
                old_ratchet,
                old_micro_timing_ticks,
                new_probability,
                new_ratchet,
                new_micro_timing_ticks,
                ..
            } => {
                assert_eq!(
                    old_probability.map(f32::to_bits),
                    Some(0.25_f32.to_bits()),
                    "old 必须等于**文档现值**（⛔ 不信调用方声明）"
                );
                assert_eq!(*old_ratchet, Some(3));
                assert_eq!(*old_micro_timing_ticks, Some(-17));
                assert_eq!(new_probability.map(f32::to_bits), Some(0.75_f32.to_bits()));
                assert_eq!(*new_ratchet, Some(2));
                assert_eq!(*new_micro_timing_ticks, Some(12));
            }
            other => panic!("必须编译成 `Op::SetNoteExpression`, 实际 {other:?}"),
        }

        // ⑤ 应用 → 读回逐位 → 逆操作逐字节复原。
        compiled[0].apply(&mut project).expect("应用");
        let note = project.note(&clip_id, &note_id).expect("音符");
        assert_eq!(note.probability.map(f32::to_bits), Some(0.75_f32.to_bits()));
        assert_eq!(note.ratchet, Some(2));
        assert_eq!(note.micro_timing_ticks, Some(12));
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            serde_json::to_vec(&project).expect("序列化"),
            bytes_before,
            "一步撤销必须逐字节回到原状"
        );

        // ⑥ `null` = 清空（三个键都显式陈述）。
        let clear = parse_ops(&serde_json::json!([{
            "kind": SET_NOTE_EXPRESSION_KIND,
            "noteId": note_id,
            "probability": null,
            "ratchet": null,
            "microTimingTicks": null,
        }]))
        .expect("null 必须被接受");
        let compiled = compile(&project, &track_id, &clip_id, &clear).expect("编译");
        compiled[0].apply(&mut project).expect("清空");
        let note = project.note(&clip_id, &note_id).expect("音符");
        assert_eq!(note.probability, None, "null = 清空 = 必然触发");
        assert_eq!(note.ratchet, None, "null = 清空 = 等价于 1");
        assert_eq!(note.micro_timing_ticks, None, "null = 清空 = 无偏移");
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            serde_json::to_vec(&project).expect("序列化"),
            bytes_before,
            "清空一步撤销也必须逐字节回到原状"
        );

        // ⑦ 越界在**解析期**报 `OUT_OF_RANGE` 并逐字点名 field。
        for (field, payload) in [
            (
                "probability",
                serde_json::json!({"probability": 1.5, "ratchet": 2, "microTimingTicks": 0}),
            ),
            (
                "probability",
                serde_json::json!({"probability": -0.5, "ratchet": 2, "microTimingTicks": 0}),
            ),
            (
                "ratchet",
                serde_json::json!({"probability": 0.5, "ratchet": 0, "microTimingTicks": 0}),
            ),
            (
                "ratchet",
                serde_json::json!({"probability": 0.5, "ratchet": 17, "microTimingTicks": 0}),
            ),
            (
                "microTimingTicks",
                serde_json::json!({"probability": 0.5, "ratchet": 2, "microTimingTicks": -241}),
            ),
            (
                "microTimingTicks",
                serde_json::json!({"probability": 0.5, "ratchet": 2, "microTimingTicks": 241}),
            ),
        ] {
            let mut object = payload.as_object().expect("对象").clone();
            object.insert("kind".to_owned(), Value::from(SET_NOTE_EXPRESSION_KIND));
            object.insert("noteId".to_owned(), serde_json::json!(note_id));
            let fault =
                parse_ops(&serde_json::json!([Value::Object(object)])).expect_err("越界值必须被拒");
            assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange), "{field}");
            assert_eq!(lane_fault_data(&fault)["field"], field);
        }

        // ⑧ 音符不存在 ⇒ 既有错误 `ENTITY_NOT_FOUND`（⛔ 新码）。
        let ghost = deterministic_id("ghost-note");
        let fault = compile(
            &project,
            &track_id,
            &clip_id,
            &[NoteOp::SetNoteExpression {
                note_id: ghost,
                new_probability: None,
                new_ratchet: None,
                new_micro_timing_ticks: None,
            }],
        )
        .expect_err("音符不存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::EntityNotFound));
    }

    /// `setNotePitchShape`：载荷**只带新值**（旧值从当前文档读）、未知键响亮拒绝、
    /// 清空与不变可区分（`#334`）。
    ///
    /// 判据钉七件事：① 规范形状（只有 `kind`/`noteId`/两个 `new*`）解析 → 编译到
    /// **正是** `Op::SetNotePitchShape`，且 `old_bend` / `old_slide` 等于**文档现值**
    /// （⛔ 不是把调用方声明抄进 `Op`，因为载荷里根本没有旧值）；
    /// ② 把旧值（`oldPitchBendCurve` / `oldSlide`）写进载荷是**未知键** ⇒
    /// `INVALID_PARAMETER_RANGE`，`reason = unknownSetNotePitchShapeField`，
    /// `supportedFields` 如实是四键且报文点名字段（与 `oldProbability` 同一条纪律）；
    /// ③ 该 op 在**没有旧值声明**时是 truthful 的：第二次编译读到的 `old_*` 是**第一次
    /// 写后**的文档值（旧值随文档走，不靠调用方）；④ 缺键 ⛔ 不等于 `null`；
    /// ⑤ 弯音的 `null` 是**形状错误**（"无"只有 `[]` 一种拼法）；
    /// ⑥ 值域错误复用模型的变体（`OUT_OF_RANGE` / `INVALID_PARAMETER_RANGE`）；
    /// ⑦ 清空（`[]` / `null`）合法、与"原值写回"可区分，且一步撤销逐字节复原。
    ///
    /// 注入（实测红）：把 `old_bend` 写成 `Vec::new()` 常量（而不是读文档）⇒ ①/③ 红；
    /// 删掉 [`reject_set_note_pitch_shape_fields`] 调用 ⇒ ② 红（旧值会被静默忽略）。
    #[test]
    fn set_note_pitch_shape_compiles_new_values_only_and_rejects_old_keys() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let note_id = *project.clip_pool[&clip_id]
            .content
            .notes()
            .expect("MIDI 片段")
            .keys()
            .next()
            .expect("样本音符");
        // 先把文档推到**非空形状** ⇒ "旧值真从文档读"才有牙（占位常量会全绿）。
        {
            let notes = project
                .clip_pool
                .get_mut(&clip_id)
                .expect("样本片段")
                .content
                .notes_mut()
                .expect("MIDI 片段");
            let note = notes.get_mut(&note_id).expect("样本音符");
            note.pitch_bend_curve = vec![(0, 0), (240, 200)];
            note.slide = Some(SlideConfig {
                duration_ticks: 240,
                target_pitch: 72,
                curve: yeban_model::music::CurveType::SCurve,
            });
        }
        let bytes_before = serde_json::to_vec(&project).expect("序列化");

        // ① 规范形状（**只有**新值）→ 解析 → 编译。
        let canonical = serde_json::json!({
            "kind": SET_NOTE_PITCH_SHAPE_KIND,
            "noteId": note_id,
            "newPitchBendCurve": [[0, 0], [480, 1_200]],
            "newSlide": null,
        });
        let ops = parse_ops(&serde_json::json!([canonical])).expect("规范形状必须被接受");
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].kind_name(), SET_NOTE_PITCH_SHAPE_KIND);
        assert!(ops[0].is_note_level(), "它读/写一个音符");
        assert!(ops[0].reads_envelope_clip(), "它用顶层 `clipId` 定位片段");

        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::SetNotePitchShape {
                old_bend,
                old_slide,
                new_bend,
                new_slide,
                ..
            } => {
                assert_eq!(
                    old_bend,
                    &vec![(0u64, 0i16), (240, 200)],
                    "old 必须等于**文档现值**（载荷里没有旧值）"
                );
                assert_eq!(
                    *old_slide,
                    Some(SlideConfig {
                        duration_ticks: 240,
                        target_pitch: 72,
                        curve: yeban_model::music::CurveType::SCurve,
                    }),
                    "old_slide 必须等于文档现值"
                );
                assert_eq!(new_bend, &vec![(0u64, 0i16), (480, 1_200)]);
                assert_eq!(*new_slide, None, "`null` = 清空滑音");
            }
            other => panic!("必须编译成 `Op::SetNotePitchShape`, 实际 {other:?}"),
        }
        assert_eq!(
            SET_NOTE_PITCH_SHAPE_FIELDS,
            ["kind", "noteId", "newPitchBendCurve", "newSlide"],
            "线上契约只有这四个键（⛔ 没有 old*）"
        );

        // ② 旧值写进载荷 ⇒ 未知键（与 `oldProbability` 同一条纪律）。
        for key in ["oldPitchBendCurve", "oldSlide"] {
            let mut object = canonical.as_object().expect("对象").clone();
            object.insert(key.to_owned(), Value::Null);
            let fault =
                parse_ops(&serde_json::json!([Value::Object(object)])).expect_err("旧值必须被拒");
            assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
            {
                let data = lane_fault_data(&fault);
                assert_eq!(data["reason"], "unknownSetNotePitchShapeField");
                assert_eq!(data["unsupportedFields"], serde_json::json!([key]));
                assert_eq!(
                    data["supportedFields"],
                    serde_json::json!(SET_NOTE_PITCH_SHAPE_FIELDS)
                );
            }
            let value = fault.into_result().expect("带内");
            assert!(
                value["error"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains(key)),
                "报文必须点名 `{key}`: {value}"
            );
        }
        // 同族的其它错拼也响亮拒绝（`add` 的内层键名 / 简称）。
        for key in ["pitchBendCurve", "slide", "oldBend", "newBend"] {
            let mut object = canonical.as_object().expect("对象").clone();
            object.insert(key.to_owned(), Value::Null);
            let fault =
                parse_ops(&serde_json::json!([Value::Object(object)])).expect_err("未知键必须被拒");
            assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
            assert_eq!(
                lane_fault_data(&fault)["reason"],
                "unknownSetNotePitchShapeField",
                "{key}: 未知键 reason"
            );
        }

        // ③ truthful 不需要旧值：写一次，再从**写后的**文档编译第二个 op —— 它的
        // `old_*` 必须等于第一个 op 的**新值**（旧值跟着文档走）。
        compiled[0].apply(&mut project).expect("应用");
        let second = parse_ops(&serde_json::json!([{
            "kind": SET_NOTE_PITCH_SHAPE_KIND,
            "noteId": note_id,
            "newPitchBendCurve": [],
            "newSlide": null,
        }]))
        .expect("第二个 op 合法");
        let second = compile(&project, &track_id, &clip_id, &second).expect("编译");
        match &second[0] {
            Op::SetNotePitchShape {
                old_bend,
                old_slide,
                ..
            } => {
                assert_eq!(
                    old_bend,
                    &vec![(0u64, 0i16), (480, 1_200)],
                    "第二次编译的 old 必须是第一次写后的文档值"
                );
                assert_eq!(*old_slide, None);
            }
            other => panic!("第二个 op 必须仍是 `Op::SetNotePitchShape`, 实际 {other:?}"),
        }

        // ④ 缺键 ⛔ 不等于 null（本形态只有两个新值键）。
        for missing in ["newPitchBendCurve", "newSlide"] {
            let mut object = canonical.as_object().expect("对象").clone();
            object.remove(missing);
            let fault =
                parse_ops(&serde_json::json!([Value::Object(object)])).expect_err("缺键必须被拒");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{missing}: 缺键"
            );
        }

        // ⑤ 弯音的 `null` 是形状错误（"无弯音"只有空数组一种拼法）。
        let mut object = canonical.as_object().expect("对象").clone();
        object.insert("newPitchBendCurve".to_owned(), Value::Null);
        let fault = parse_ops(&serde_json::json!([Value::Object(object)]))
            .expect_err("弯音的 null 必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        assert_eq!(
            lane_fault_data(&fault)["field"],
            SET_NOTE_PITCH_SHAPE_NEW_BEND_FIELD
        );

        // ⑥ 值域错误复用模型既有变体。
        for (field, payload, code) in [
            (
                "newPitchBendCurve",
                serde_json::json!({"newPitchBendCurve": [[0, 1_201]]}),
                ErrorCode::OutOfRange,
            ),
            (
                "newPitchBendCurve",
                serde_json::json!({"newPitchBendCurve": [[480, 0], [480, 0]]}),
                ErrorCode::InvalidParameterRange,
            ),
            (
                "newSlide.targetPitch",
                serde_json::json!({"newSlide": {"durationTicks": 240, "targetPitch": 128}}),
                ErrorCode::OutOfRange,
            ),
            (
                "newSlide.durationTicks",
                serde_json::json!({"newSlide": {"durationTicks": 0, "targetPitch": 60}}),
                ErrorCode::OutOfRange,
            ),
        ] {
            let mut object = canonical.as_object().expect("对象").clone();
            for (key, value) in payload.as_object().expect("对象") {
                object.insert(key.clone(), value.clone());
            }
            let fault = parse_ops(&serde_json::json!([Value::Object(object)]))
                .expect_err("越界/形状错误必须被拒");
            assert_eq!(fault.domain_code(), Some(code), "{field}");
            assert_eq!(lane_fault_data(&fault)["field"], field);
        }

        // ⑦ 清空 vs 原值写回可区分 → 撤销逐字节复原。
        let after_write = serde_json::to_vec(&project).expect("序列化");
        // `new == old`（原值写回）合法且**不改文档** —— 载荷没有旧值，怎么证明？
        // 再写一次同一形状，文档字节必须不变。
        let unchanged = parse_ops(&serde_json::json!([{
            "kind": SET_NOTE_PITCH_SHAPE_KIND,
            "noteId": note_id,
            "newPitchBendCurve": [[0, 0], [480, 1_200]],
            "newSlide": null,
        }]))
        .expect("原值写回必须合法");
        let unchanged = compile(&project, &track_id, &clip_id, &unchanged).expect("编译");
        unchanged[0].apply(&mut project).expect("原值写回");
        assert_eq!(
            serde_json::to_vec(&project).expect("序列化"),
            after_write,
            "原值写回必须真的什么都不改"
        );

        // 清空：弯音 `[]`、滑音 `null`。
        let clear = parse_ops(&serde_json::json!([{
            "kind": SET_NOTE_PITCH_SHAPE_KIND,
            "noteId": note_id,
            "newPitchBendCurve": [],
            "newSlide": null,
        }]))
        .expect("清空形状合法");
        let clear = compile(&project, &track_id, &clip_id, &clear).expect("编译");
        assert!(matches!(
            &clear[0],
            Op::SetNotePitchShape {
                new_bend,
                new_slide: None,
                ..
            } if new_bend.is_empty()
        ));
        clear[0].apply(&mut project).expect("清空");
        let cleared = project.note(&clip_id, &note_id).expect("音符");
        assert!(cleared.pitch_bend_curve.is_empty(), "空 Vec = 无弯音");
        assert_eq!(cleared.slide, None);
        assert_ne!(
            serde_json::to_vec(&project).expect("序列化"),
            after_write,
            "清空必须与\"原值写回\"可区分"
        );
        clear[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            serde_json::to_vec(&project).expect("序列化"),
            after_write,
            "清空的一步撤销必须逐字节回到写后的状态"
        );
        // 撤销第一个 op（此时文档里是写后的状态）。
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            serde_json::to_vec(&project).expect("序列化"),
            bytes_before,
            "一步撤销必须逐字节回到最初的状态"
        );
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
        // （唯一的场景级形态）后为 15；`setScene`（场景级**写入**形态，
        // 与 `removeScene` 共用场景级那个桶）后为 16；本票再新增 `removeTrack`
        // （唯一的音轨实体级形态）后为 17 —— 这是**同步**计数
        // （多了一个真存在的 `kind`），不是弱化判据。
        // 本票（设备 CRUD）再新增两个**设备级**形态（`insertDevice` / `removeDevice`）
        // 后为 19 —— 同样是与真实的 `kind` 同步，字面值不是常量自比。
        // 本轮（走带配置）新增 `setTransportConfig`（**不是**音轨实体级、也不寻址音轨）
        // 后为 20 —— 同样与真实 `kind` 同步。
        // 本票（路由节点）新增 `addRoutingNode`（路由级；补上"造得出节点"的**逐步**通路）
        // 后为 21 —— 同样与真实 `kind` 同步。
        // 本票（音符时间位置）新增 `setNoteTiming`（第五个音符级形态）后为 37 —— 同样
        // 与真实 `kind` 同步。
        // 本票（`#327` 标题）新增 `setTitle`（顶层文档字段，与 `setAuthor` 同族）后为 38
        // —— 同样与真实 `kind` 同步。
        // 本票（`#328` 音频片段增益）新增 `setClipGain`（池级，池条目的 `gain_db`）后为 39
        // —— 同样与真实 `kind` 同步。
        // 本票（`#329` 音符表现力）新增 `setNoteExpression`（第六个音符级形态）后为 40
        // —— 同样与真实 `kind` 同步。
        // 本票（`#334` 音高形状）新增 `setNotePitchShape`（第七个音符级形态）后为 41
        // —— 同样与真实 `kind` 同步。
        // 本票（`#336` 摆放循环）新增 `setClipPlacementLoop`（摆放属性级，⛔ 不是
        // 音符级）后为 42 —— 同样与真实 `kind` 同步。
        assert_eq!(OP_KINDS.len(), 42);
        assert_eq!(TrackFlag::NAMES, [SET_TRACK_MUTE_KIND, SET_TRACK_SOLO_KIND]);
        assert!(OP_KINDS.contains(&SET_TRACK_MUTE_KIND));
        assert!(OP_KINDS.contains(&SET_TRACK_SOLO_KIND));
        assert!(OP_KINDS.contains(&SET_AUTOMATION_LANE_KIND));
        assert!(OP_KINDS.contains(&REMOVE_AUTOMATION_POINT_KIND));
        assert!(OP_KINDS.contains(&REMOVE_CLIP_KIND));
        assert!(OP_KINDS.contains(&REMOVE_TRACK_KIND));
        assert!(OP_KINDS.contains(&INSERT_DEVICE_KIND));
        assert!(OP_KINDS.contains(&REMOVE_DEVICE_KIND));
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

    /// 音轨实体级形态的**字面**判据：载荷只有音轨身份（撤销载荷从**当前文档**读整条
    /// 音轨），一步就能被 `Op::invert` 逐字节回退到原状。
    ///
    /// 这一条对着"读侧报得出音轨、工具面写得出音轨的属性却取不走音轨"的缺口：
    /// `yeban_query_project` 的 `tracks` 一直在报身份，`Op::AddTrack` 在 MCP 侧也**真的**
    /// 被 `yeban_open_project` 与 `yeban_propose_section` 构造，而 `Op::RemoveTrack` 在
    /// `crates/yeban-mcp/src` 的实测构造点是 0 ⇒ 建得出的音轨取不走（唯一出路是
    /// `yeban_undo`，那会连带回退同一提交里的其它一切）。
    ///
    /// 注入（实测红）：把编译出的模型变体由 `Op::RemoveTrack` 换成 `Op::AddTrack`
    /// 一个副本（一次什么都不删的写）⇒ 变体匹配那条红；把撤销载荷换成**另一条**音轨的
    /// 克隆（而不是文档里那一条）⇒ "撤销载荷必须是文档里那一条"红。
    #[test]
    fn remove_track_compiles_and_inverts_byte_for_byte() {
        let mut project = filled_project();
        let target = project
            .tracks
            .values()
            .find(|track| track.kind == yeban_model::TrackKind::Audio)
            .expect("样本里必须有非主总线音轨")
            .id;
        assert_ne!(target, project.master_bus_track_id, "夹具前提");
        let expected = project.tracks[&target].clone();
        assert!(
            !expected.clips.is_empty(),
            "夹具前提: 被取走的那条音轨自己带着摆放 (撤销载荷必须把子结构一起带回)"
        );
        let bytes_before = serde_json::to_string(&project).expect("序列化");
        let (track_id, clip_id) = lead_clip(&project);

        let ops = parse_ops(&serde_json::json!([
            {"kind": REMOVE_TRACK_KIND, "trackId": target.to_canonical_string()}
        ]))
        .expect("规范形状必须被接受");
        assert_eq!(ops[0].kind_name(), REMOVE_TRACK_KIND);
        assert!(!ops[0].is_note_level(), "音轨实体级不读不写音符");
        assert!(!ops[0].is_routing_level(), "音轨不是路由图的一部分");
        assert!(!ops[0].is_section_level(), "音轨不是曲式段落");
        assert!(!ops[0].is_scene_level(), "音轨不是场景");
        assert!(ops[0].is_track_entity_level());
        assert!(NoteOp::all_track_entity_level(&ops));
        assert_eq!(ops[0], NoteOp::RemoveTrack { track_id: target });

        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::RemoveTrack {
                track_id: addressed,
                previous_track,
            } => {
                assert_eq!(*addressed, target);
                assert_eq!(
                    previous_track, &expected,
                    "撤销载荷必须是**文档里那一条**音轨 (不是调用方声明的)"
                );
            }
            other => panic!("应当是 RemoveTrack: {other:?}"),
        }

        compiled[0].apply(&mut project).expect("取走");
        assert!(
            !project.tracks.contains_key(&target),
            "音轨必须真的从 `tracks` 里消失"
        );
        assert!(project.validate().is_ok(), "取走之后工程必须仍然合法");
        // 片段池**没有**被回收: 池里的材料可能仍被别的音轨引用。
        assert!(
            project.clip_pool.contains_key(&clip_id),
            "池级材料不随音轨一起消失"
        );

        // 逆操作: `RemoveTrack` 的逆是 `AddTrack`（两条载荷都只有音轨身份 +
        // 整条音轨），因此必须把音轨**连同它的摆放**原样放回去。
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            project.tracks.get(&target),
            Some(&expected),
            "逆操作必须把音轨 (含子结构) 放回原来的形状"
        );
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "逆操作必须逐字节回到取走之前的文档"
        );
    }

    /// 音轨实体级的**字段名与支持集合**被钉住（不多报一个键，也不少报一个键）。
    ///
    /// 注入（实测红）：把 [`REMOVE_TRACK_FIELD`] 改成 `sceneId` ⇒ 本判据红 ——
    /// 音轨与场景是两种实体，共用一个词会让"取走的是哪一个"从形状上无法区分。
    #[test]
    fn remove_track_field_names_are_pinned() {
        assert_eq!(REMOVE_TRACK_KIND, "removeTrack");
        assert_eq!(REMOVE_TRACK_FIELD, "trackId");
        assert_ne!(REMOVE_TRACK_FIELD, SCENE_FIELD, "音轨不是场景");
        assert_ne!(REMOVE_TRACK_FIELD, SECTION_FIELD, "音轨不是曲式段落");
        assert_ne!(REMOVE_TRACK_FIELD, ROUTING_NODE_FIELD, "音轨不是路由节点");
        assert_ne!(REMOVE_TRACK_FIELD, ROUTING_EDGE_FIELD, "音轨不是路由边");
        assert_eq!(REMOVE_TRACK_FIELDS, ["kind", "trackId"]);
        assert!(OP_KINDS.contains(&REMOVE_TRACK_KIND));
        // 与模型自己的变体名同词（不手写第二张会漂移的表）。
        let project = filled_project();
        let (track_id, _) = lead_clip(&project);
        assert_eq!(
            Op::RemoveTrack {
                track_id,
                previous_track: project.tracks[&track_id].clone(),
            }
            .name(),
            "RemoveTrack"
        );
    }

    /// 音轨实体级的形状错误**响亮失败**（绝不静默丢弃），而规范形状放行。
    ///
    /// 注入（实测红）：去掉 [`reject_remove_track_fields`] 的调用 ⇒ 前六条
    /// （把别的实体的寻址 `sceneId` / `sectionId` / `nodeId` / `edgeId` 与工具顶层的
    /// `clipId` 搬过来、以及多写 `previousTrack`）被**静默接受**，本判据红。
    #[test]
    fn remove_track_shapes_fail_loudly() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let target = project
            .tracks
            .values()
            .find(|track| track.kind == yeban_model::TrackKind::Audio)
            .expect("样本里必须有非主总线音轨")
            .id;
        let target_text = target.to_canonical_string();
        let scene_id = *project.scenes.keys().next().expect("样本里必须有场景");
        let section_id = *project
            .sections
            .keys()
            .next()
            .expect("样本里必须有曲式段落");
        let (node, _) = a_non_master_node(&project);

        for broken in [
            // 把**场景**的寻址搬过来: `trackId` 缺失, 而且 `sceneId` 不是本形态的键。
            serde_json::json!([{"kind": REMOVE_TRACK_KIND,
                                "sceneId": scene_id.to_canonical_string()}]),
            // 把**曲式段落**的寻址搬过来。
            serde_json::json!([{"kind": REMOVE_TRACK_KIND,
                                "sectionId": section_id.to_canonical_string()}]),
            // 把**路由节点**的寻址搬过来。
            serde_json::json!([{"kind": REMOVE_TRACK_KIND, "nodeId": node.to_canonical_string()}]),
            // 把**路由边**的寻址搬过来。
            serde_json::json!([{"kind": REMOVE_TRACK_KIND, "edgeId": node.to_canonical_string()}]),
            // 把工具顶层的 `clipId` 搬过来（那是别的实体的寻址）。
            serde_json::json!([{"kind": REMOVE_TRACK_KIND, "trackId": target_text,
                                "clipId": clip_id.to_canonical_string()}]),
            // 以为要报告"音轨删除前的状态"而多写 `previousTrack`。
            serde_json::json!([{"kind": REMOVE_TRACK_KIND, "trackId": target_text,
                                "previousTrack": null}]),
        ] {
            let fault = parse_ops(&broken).expect_err(&format!("必须被拒: {broken}"));
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{broken}"
            );
            assert_eq!(
                lane_fault_data(&fault)["reason"],
                "unknownRemoveTrackField",
                "{broken}"
            );
            assert_eq!(
                lane_fault_data(&fault)["supportedRemoveTrackFields"],
                serde_json::json!(["kind", "trackId"]),
                "{broken}"
            );
        }

        for broken in [
            // 缺 `trackId`。
            serde_json::json!([{"kind": REMOVE_TRACK_KIND}]),
            // `trackId` 不是字符串。
            serde_json::json!([{"kind": REMOVE_TRACK_KIND, "trackId": 70}]),
            // `trackId` 不是合法 ULID。
            serde_json::json!([{"kind": REMOVE_TRACK_KIND, "trackId": "not-a-ulid"}]),
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
            {"kind": REMOVE_TRACK_KIND, "trackId": target_text}
        ]))
        .expect("规范形状必须被接受");
        assert!(ops[0].is_track_entity_level());
        compile(&project, &track_id, &clip_id, &ops).expect("编译");
    }

    /// 音轨不存在 ⇒ 本层 `ENTITY_NOT_FOUND`（带上 `reason` 与 `trackId`）；
    /// 主总线 ⇒ `CONFLICT`；两者都**不改文档**（编译期只读）。
    ///
    /// 注入（实测红）：把存在性检查换成一条**兜底**（取文档里任意一条音轨当撤销载荷，
    /// 而不是报错）⇒ 第一条红，诊断里 `previousTrack` 是**另一条**音轨的身份
    /// —— 那正是"静默取错载荷"的形状。
    #[test]
    fn remove_track_refuses_a_missing_track_and_the_master_bus() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let missing = EntityId::from_str("01J8ZQ00000000000000000999").expect("ULID");
        assert!(!project.tracks.contains_key(&missing), "夹具前提");

        let ops = parse_ops(&serde_json::json!([
            {"kind": REMOVE_TRACK_KIND, "trackId": missing.to_canonical_string()}
        ]))
        .expect("解析");
        let fault = compile(&project, &track_id, &clip_id, &ops).expect_err("音轨不存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::EntityNotFound));
        assert_eq!(lane_fault_data(&fault)["reason"], "trackNotFound");
        assert_eq!(
            lane_fault_data(&fault)["trackId"],
            serde_json::json!(missing.to_canonical_string())
        );

        // 主总线: 模型的不变量在这里被提前报成 `CONFLICT` (模型自己会用
        // `OpStateMismatch` 报同一个结论, 但那条路径不带本层的 `reason` / `hint`)。
        let ops = parse_ops(&serde_json::json!([
            {"kind": REMOVE_TRACK_KIND,
             "trackId": project.master_bus_track_id.to_canonical_string()}
        ]))
        .expect("解析");
        let fault = compile(&project, &track_id, &clip_id, &ops).expect_err("主总线不能取走");
        assert_eq!(fault.domain_code(), Some(ErrorCode::Conflict));
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "masterBusTrackCannotBeRemoved"
        );
        assert_eq!(
            lane_fault_data(&fault)["masterBusTrackId"],
            serde_json::json!(project.master_bus_track_id.to_canonical_string())
        );
        // 而且这个身份**真的**在文档里 (主总线的拒绝不是"顺带报了不存在")。
        assert!(project.tracks.contains_key(&project.master_bus_track_id));

        // 阴性对照: 非主总线音轨走得通, 且失败路径一个字节都没改文档
        // (`compile` 只读; 上面两次 `expect_err` 的 `project` 未被移动)。
        let ops = parse_ops(&serde_json::json!([
            {"kind": REMOVE_TRACK_KIND,
             "trackId": project.master_bus_track_id.to_canonical_string()}
        ]))
        .expect("解析");
        assert!(compile(&project, &track_id, &clip_id, &ops).is_err());
    }

    /// 音轨实体级**不要求片段是 MIDI**、也**不读顶层身份**：`removeTrack` 自带寻址，
    /// 顶层 `trackId` / `clipId` 是占位的 nil 身份也照样编译。
    ///
    /// 注入（实测红）：把 [`compile`] 入口的 `track_entity_only` 分支去掉 ⇒ 本判据红
    /// （nil 顶层身份查不到音轨 ⇒ `TRACK_NOT_FOUND`），而三个路由级 / 段落 / 场景形态
    /// 的同类判据仍绿 —— 它们是**别的**形态，各自有自己的自带寻址。
    #[test]
    fn remove_track_does_not_need_the_envelope_ids_or_midi() {
        let project = filled_project();
        let audio_clip = project
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_none())
            .expect("样本里必须有非 MIDI 片段")
            .id;
        let audio_track = project
            .tracks
            .values()
            .find(|track| track.kind == yeban_model::TrackKind::Audio)
            .expect("样本里必须有音频轨")
            .id;

        // 顶层给一条**别的**音轨 + 一条**非 MIDI** 片段: 本形态一个都不读。
        let ops = parse_ops(&serde_json::json!([
            {"kind": REMOVE_TRACK_KIND, "trackId": audio_track.to_canonical_string()}
        ]))
        .expect("解析");
        let compiled = compile(&project, &audio_track, &audio_clip, &ops)
            .expect("音轨实体级写入不读片段内容, 非 MIDI 片段也必须被接受");
        assert_eq!(compiled.len(), 1);
        assert!(matches!(compiled[0], Op::RemoveTrack { .. }));

        // 顶层两个身份是 nil 占位 ⇒ 仍然编译（`all_track_entity_level` 走的那条路）。
        let nil = EntityId::default();
        let compiled = compile(&project, &nil, &nil, &ops)
            .expect("音轨实体级自带寻址, 顶层 trackId / clipId 不被读取");
        assert!(matches!(compiled[0], Op::RemoveTrack { .. }));

        // 阴性对照: 换一个**音符级**形态, 同样的 nil 顶层身份必须**响亮失败**
        // （否则本判据证明不了"顶层身份真的被跳过", 只证明了"有个兜底"）。
        let note_ops = parse_ops(&serde_json::json!([
            {"kind": "add", "note": {"startTick": 0, "pitch": 60, "durationTicks": 480}}
        ]))
        .expect("解析");
        let fault = compile(&project, &nil, &nil, &note_ops).expect_err("音符级必须要音轨");
        assert_eq!(fault.domain_code(), Some(ErrorCode::TrackNotFound));
    }

    /// 音轨实体级与别的形态 / 摆放**互斥**：三条规则各自红一次，且阴性对照放行。
    ///
    /// 注入（实测红）：删掉 mixed 那条分支 ⇒ 第一条红；删掉 `placement_present` 那条
    /// 分支 ⇒ 第二条红；把"没有 `removeTrack` 时什么都不做"去掉 ⇒ 末条红。
    #[test]
    fn remove_track_conflicts_fail_loudly() {
        let single = parse_ops(&serde_json::json!([
            {"kind": REMOVE_TRACK_KIND, "trackId": "01J8Z0000000000000000000AB"}
        ]))
        .expect("解析");
        assert!(
            reject_remove_track_conflicts(&single, false).is_ok(),
            "阴性对照: 单独出现且没有摆放 ⇒ 放行"
        );

        let mixed = parse_ops(&serde_json::json!([
            {"kind": REMOVE_TRACK_KIND, "trackId": "01J8Z0000000000000000000AB"},
            {"kind": "velocity", "noteId": "01J8Z0000000000000000000CD", "velocity": 40}
        ]))
        .expect("解析");
        let fault = reject_remove_track_conflicts(&mixed, false).expect_err("必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "removeTrackTakesNoOtherOps"
        );
        assert_eq!(
            lane_fault_data(&fault)["opKinds"],
            serde_json::json!(["removeTrack", "velocity"]),
            "报出的必须是本次真给的 kind 列表"
        );
        assert_eq!(
            lane_fault_data(&fault)["trackId"],
            serde_json::json!("01J8Z0000000000000000000AB"),
            "报出的必须是**被取走**的那条音轨"
        );

        let fault = reject_remove_track_conflicts(&single, true).expect_err("必须被拒");
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "removeTrackIsNotPlacement"
        );

        // 没有 `removeTrack` 的调用逐字节等于旧行为（摆放形态也照旧）。
        let note_ops = parse_ops(&serde_json::json!([
            {"kind": "add", "note": {"startTick": 0, "pitch": 60, "durationTicks": 480}}
        ]))
        .expect("解析");
        assert!(
            reject_remove_track_conflicts(&note_ops, true).is_ok(),
            "别的形态不受这两条规则影响"
        );
    }

    /// 一条**带设备链**的音轨（夹具里那条 `lead`）。
    ///
    /// 设备级判据的公共前置：没有设备的音轨量不出"插入 / 取走"，而用一条空链做夹具
    /// 会让"追加到链尾"与"插到第 0 槽"变成同一件事。
    fn device_track(project: &YebanProjectV1) -> EntityId {
        project
            .tracks
            .values()
            .find(|track| !track.devices.is_empty())
            .expect("样本里必须有带设备链的音轨")
            .id
    }

    /// 设备级形态的**字段名与支持集合**被钉住（不多报一个键，也不少报一个键）。
    ///
    /// 注入（实测红）：把 [`DEVICE_SLOT_FIELD`] 改成 `"slot"` ⇒ 本判据红；
    /// 把 `device.kind` 的取值表改成模型没有的名字（[`DEVICE_KINDS`] 少一项）⇒
    /// `device_kinds_match_the_model_variant_names` 红。
    #[test]
    fn device_field_names_are_pinned() {
        assert_eq!(INSERT_DEVICE_KIND, "insertDevice");
        assert_eq!(REMOVE_DEVICE_KIND, "removeDevice");
        assert_eq!(DEVICE_TRACK_FIELD, "trackId");
        assert_eq!(DEVICE_SLOT_FIELD, "slotIndex");
        assert_eq!(DEVICE_PAYLOAD_FIELD, "device");
        assert_eq!(DEVICE_ID_FIELD, "deviceId");
        assert_ne!(DEVICE_ID_FIELD, SCENE_FIELD, "设备不是场景");
        assert_ne!(DEVICE_ID_FIELD, SECTION_FIELD, "设备不是曲式段落");
        assert_ne!(DEVICE_ID_FIELD, ROUTING_NODE_FIELD, "设备不是路由节点");
        assert_ne!(DEVICE_ID_FIELD, ROUTING_EDGE_FIELD, "设备不是路由边");
        assert_eq!(
            INSERT_DEVICE_FIELDS,
            ["kind", "trackId", "device", "slotIndex"]
        );
        assert_eq!(REMOVE_DEVICE_FIELDS, ["kind", "trackId", "deviceId"]);
        assert_eq!(
            DEVICE_FIELDS,
            [
                "deviceId",
                "name",
                "kind",
                "bypassed",
                "latencySamples",
                "params"
            ]
        );
        assert_eq!(DEVICE_PARAM_FIELDS, ["name", "value", "unit"]);
        assert!(OP_KINDS.contains(&INSERT_DEVICE_KIND));
        assert!(OP_KINDS.contains(&REMOVE_DEVICE_KIND));
    }

    /// `device.kind` 的词表与**模型自己的变体名**同源（不手写第二张会漂移的表）。
    ///
    /// 注入（实测红）：把 [`DEVICE_KINDS`] 的任一元素改成模型没有的拼写 ⇒ 本判据红；
    /// 删掉一项 ⇒ 本判据红（模型有四个变体）。
    #[test]
    fn device_kinds_match_the_model_variant_names() {
        let model_names = [
            DeviceKind::InternalInstrument,
            DeviceKind::InternalEffect,
            DeviceKind::ExternalInstrument,
            DeviceKind::ExternalEffect,
        ];
        assert_eq!(DEVICE_KINDS.len(), model_names.len());
        for (index, kind) in model_names.iter().enumerate() {
            let name = serde_json::to_value(kind).expect("枚举序列化");
            assert_eq!(
                name,
                serde_json::json!(DEVICE_KINDS[index]),
                "工具面的 `device.kind` 必须逐字等于模型 serde 的变体名"
            );
        }
        // 四个名字互不相同（否则"词表四项"这个计数会骗人）。
        let mut sorted: Vec<&str> = DEVICE_KINDS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), DEVICE_KINDS.len());
    }

    /// `insertDevice` 缺省**追加到链尾**；给了 `slotIndex` 就落在那一格；
    /// 逆操作把文档逐字节还原。
    ///
    /// 注入（实测红）：把 `slot_index.unwrap_or(track.devices.len())` 改成
    /// `unwrap_or(0)` ⇒ 第一条红（缺省落到了 0 槽）；把 `Op::InsertDevice` 的
    /// `slot_index: resolved` 改成 `0` ⇒ 第二条红。
    #[test]
    fn insert_device_appends_or_lands_at_the_given_slot_and_is_reversible() {
        let mut project = filled_project();
        let track = device_track(&project);
        let length_before = project.tracks[&track].devices.len();
        let bytes_before = serde_json::to_string(&project).expect("序列化");
        let fresh = EntityId::from_str("01J8ZQ00000000000000000DEV").expect("ULID");
        let (_, clip_id) = lead_clip(&project);

        // ① 不给 `slotIndex` ⇒ 追加到链尾 (= 当前链长)。
        let ops = parse_ops(&serde_json::json!([{
            "kind": INSERT_DEVICE_KIND,
            "trackId": track.to_canonical_string(),
            "device": {
                "deviceId": fresh.to_canonical_string(),
                "name": "Probe Effect",
                "kind": "InternalEffect",
                "bypassed": true,
                "latencySamples": 64,
                "params": [
                    {"name": "mix", "value": 0.5},
                    {"name": "tone", "value": 2200.0, "unit": "Hz"}
                ]
            }
        }]))
        .expect("解析");
        let compiled = compile(&project, &track, &clip_id, &ops).expect("插入必须被接受");
        assert_eq!(compiled.len(), 1);
        let Op::InsertDevice {
            track_id,
            slot_index,
            device,
        } = &compiled[0]
        else {
            panic!("必须是 Op::InsertDevice, 实际 {:?}", compiled[0]);
        };
        assert_eq!(*track_id, track);
        assert_eq!(*slot_index, length_before, "缺省 = 追加到链尾");
        assert_eq!(device.id, fresh);
        assert_eq!(device.name, "Probe Effect");
        assert_eq!(device.kind, DeviceKind::InternalEffect);
        assert!(device.bypassed, "旁通位必须真的搬进模型 (ADR-0001 D43)");
        assert_eq!(device.latency_samples, 64, "PDC 的唯一延迟来源");
        assert_eq!(device.params.len(), 2);
        assert_eq!(device.params[0].unit, None, "缺 `unit` = 无量纲 (一等状态)");
        assert_eq!(device.params[1].unit.as_deref(), Some("Hz"));

        compiled[0].apply(&mut project).expect("施加");
        assert_eq!(
            project.tracks[&track].devices.len(),
            length_before + 1,
            "链上必须真的多了一台"
        );
        assert_eq!(
            project.tracks[&track].devices[length_before].id, fresh,
            "必须落在链尾"
        );
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "逆操作必须逐字节回到插入之前的文档"
        );

        // ② 给 `slotIndex: 0` ⇒ 落在第 0 格（而不是链尾）。
        let ops = parse_ops(&serde_json::json!([{
            "kind": INSERT_DEVICE_KIND,
            "trackId": track.to_canonical_string(),
            "slotIndex": 0,
            "device": {
                "deviceId": fresh.to_canonical_string(),
                "name": "Probe Effect",
                "kind": "ExternalEffect",
                "bypassed": false,
                "latencySamples": 0,
                "params": []
            }
        }]))
        .expect("解析");
        let compiled = compile(&project, &track, &clip_id, &ops).expect("插入必须被接受");
        let Op::InsertDevice { slot_index, .. } = &compiled[0] else {
            panic!("必须是 Op::InsertDevice");
        };
        assert_eq!(*slot_index, 0, "给了 `slotIndex` 就必须落在那一格");
        compiled[0].apply(&mut project).expect("施加");
        assert_eq!(project.tracks[&track].devices[0].id, fresh);
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "换一台设备插到第 0 槽之后, 逆操作同样必须逐字节还原"
        );
    }

    /// 槽位越界**响亮失败**（不静默夹紧），且 `slotIndex == 链长` 是放行的边界。
    ///
    /// 注入（实测红）：把 `if resolved > track.devices.len()` 改成 `>=` ⇒ 第一条红
    /// （合法的链尾追加被拒）；删掉这条闸门 ⇒ 第二条红（越界一路走到模型层，
    /// `reason` 不再是 `deviceSlotOutOfRange`）。
    #[test]
    fn insert_device_refuses_an_out_of_range_slot_instead_of_clamping() {
        let project = filled_project();
        let track = device_track(&project);
        let length = project.tracks[&track].devices.len();
        let (_, clip_id) = lead_clip(&project);
        let fresh = EntityId::from_str("01J8ZQ00000000000000000DEV").expect("ULID");
        let device = serde_json::json!({
            "deviceId": fresh.to_canonical_string(),
            "name": "Probe",
            "kind": "InternalEffect",
            "bypassed": false,
            "latencySamples": 0,
            "params": []
        });

        // 边界: `slotIndex == len` 就是追加, 必须放行。
        let ops = parse_ops(&serde_json::json!([{
            "kind": INSERT_DEVICE_KIND,
            "trackId": track.to_canonical_string(),
            "slotIndex": length,
            "device": device.clone()
        }]))
        .expect("解析");
        let compiled = compile(&project, &track, &clip_id, &ops).expect("链尾是合法边界");
        assert!(matches!(
            compiled[0],
            Op::InsertDevice { slot_index, .. } if slot_index == length
        ));

        // 越界一格: 响亮拒绝, 且报文里带上链长。
        let ops = parse_ops(&serde_json::json!([{
            "kind": INSERT_DEVICE_KIND,
            "trackId": track.to_canonical_string(),
            "slotIndex": length + 1,
            "device": device
        }]))
        .expect("解析");
        let fault = compile(&project, &track, &clip_id, &ops).expect_err("越界必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
        assert_eq!(lane_fault_data(&fault)["reason"], "deviceSlotOutOfRange");
        assert_eq!(
            lane_fault_data(&fault)["len"],
            serde_json::json!(length),
            "报文必须报出链长 (调用方才知道合法的上界)"
        );
    }

    /// 设备身份在**同一条音轨**上不得重复；音轨必须存在。两条都提前报 `reason`。
    ///
    /// 注入（实测红）：删掉 `track.devices.iter().position(...)` 那段 ⇒ 第一条红
    /// （重复身份被接受, 直到提案模拟才以泛化消息冒出）；把 `project.track(target)` 的
    /// 错误映射改掉 ⇒ 第二条红。
    #[test]
    fn insert_device_refuses_a_duplicate_id_and_a_missing_track() {
        let project = filled_project();
        let track = device_track(&project);
        let existing = project.tracks[&track].devices[0].id;
        let (_, clip_id) = lead_clip(&project);
        let device = serde_json::json!({
            "deviceId": existing.to_canonical_string(),
            "name": "Duplicate",
            "kind": "InternalEffect",
            "bypassed": false,
            "latencySamples": 0,
            "params": []
        });

        let ops = parse_ops(&serde_json::json!([{
            "kind": INSERT_DEVICE_KIND,
            "trackId": track.to_canonical_string(),
            "device": device.clone()
        }]))
        .expect("解析");
        let fault = compile(&project, &track, &clip_id, &ops).expect_err("重复身份必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::Conflict));
        assert_eq!(lane_fault_data(&fault)["reason"], "deviceIdAlreadyInTrack");
        assert_eq!(
            lane_fault_data(&fault)["existingSlotIndex"],
            serde_json::json!(0)
        );
        // ⭐ `#265`：`setParam` 已经能写 `DeviceParam`，因此 hint 必须把调用方指向
        // **真的可行**的那条路（`slotIndex` + `paramIndex`），⛔ 不得再说"没有通路"
        // 或"不接受"；`removeDevice` 只保留给"换一台设备"。
        let hint = lane_fault_data(&fault)["hint"]
            .as_str()
            .expect("hint 必须是字符串");
        assert!(
            hint.contains("现在可以") && hint.contains("`DeviceParam`"),
            "hint 必须说清 `setParam` 现在可以写 `DeviceParam`: {hint}"
        );
        assert!(
            hint.contains("slotIndex") && hint.contains("paramIndex"),
            "hint 必须给出 `slotIndex` / `paramIndex` 两个下标: {hint}"
        );
        assert!(
            !hint.contains("没有通路") && !hint.contains("不接受"),
            "hint 不得再声称 `setParam` 没有通路 / 不接受 `DeviceParam`: {hint}"
        );
        assert!(
            hint.contains("removeDevice"),
            "换一台设备仍要先 `removeDevice`: {hint}"
        );

        // 音轨不存在（自带寻址的那条 `trackId` 打错了）。
        let missing = EntityId::from_str("01J8ZQ00000000000000000999").expect("ULID");
        let ops = parse_ops(&serde_json::json!([{
            "kind": INSERT_DEVICE_KIND,
            "trackId": missing.to_canonical_string(),
            "device": device
        }]))
        .expect("解析");
        let fault = compile(&project, &track, &clip_id, &ops).expect_err("音轨不存在必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::EntityNotFound));
        assert_eq!(lane_fault_data(&fault)["reason"], "deviceTrackNotFound");
    }

    /// `removeDevice` 按**身份**寻址，撤销载荷从**当前文档**读；不存在则响亮失败。
    ///
    /// 注入（实测红）：把 `previous_device: device.clone()` 改成
    /// `DeviceDefinition::default()` ⇒ 第一条红（模型的 `OpStateMismatch` 前置条件
    /// 会在施加时报错, 而判据直接比字段）；把按身份查找改成按槽位 0 ⇒ 第二条红。
    #[test]
    fn remove_device_addresses_by_identity_and_reads_the_document() {
        let mut project = filled_project();
        let track = device_track(&project);
        let expected = project.tracks[&track].devices[0].clone();
        let bytes_before = serde_json::to_string(&project).expect("序列化");
        let (_, clip_id) = lead_clip(&project);

        let ops = parse_ops(&serde_json::json!([{
            "kind": REMOVE_DEVICE_KIND,
            "trackId": track.to_canonical_string(),
            "deviceId": expected.id.to_canonical_string()
        }]))
        .expect("解析");
        let compiled = compile(&project, &track, &clip_id, &ops).expect("取走必须被接受");
        let Op::RemoveDevice {
            track_id,
            slot_index,
            previous_device,
        } = &compiled[0]
        else {
            panic!("必须是 Op::RemoveDevice, 实际 {:?}", compiled[0]);
        };
        assert_eq!(*track_id, track);
        assert_eq!(*slot_index, 0, "身份必须被翻译成它**当前**所在的槽位");
        assert_eq!(
            previous_device, &expected,
            "撤销载荷必须是文档里那一台 (整份, 不是调用方声明的)"
        );

        compiled[0].apply(&mut project).expect("施加");
        assert!(
            !project.tracks[&track]
                .devices
                .iter()
                .any(|it| it.id == expected.id),
            "合并之后那台设备必须真的从设备链里消失"
        );
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "逆操作必须逐字节还原 (设备随 Op::RemoveDevice 的逆 InsertDevice 回来)"
        );

        // 身份不存在 ⇒ ENTITY_NOT_FOUND（`deviceNotFound`）。
        let missing = EntityId::from_str("01J8ZQ00000000000000000ZZZ").expect("ULID");
        let ops = parse_ops(&serde_json::json!([{
            "kind": REMOVE_DEVICE_KIND,
            "trackId": track.to_canonical_string(),
            "deviceId": missing.to_canonical_string()
        }]))
        .expect("解析");
        let fault = compile(&project, &track, &clip_id, &ops).expect_err("不存在的设备必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::EntityNotFound));
        assert_eq!(lane_fault_data(&fault)["reason"], "deviceNotFound");
    }

    /// `device` 载荷与两个操作对象的**形状错误**响亮失败（绝不静默丢弃）。
    ///
    /// 注入（实测红）：去掉 [`parse_device`] 里那段未知键检查 ⇒ 第一条红；去掉
    /// `read_number`（改用 `as_f64`）⇒ 非有限那一条红；把 `latencySamples` 的
    /// `u32::try_from` 换成 `as u32` ⇒ 溢出那一条红。
    #[test]
    fn device_payload_shapes_fail_loudly() {
        let project = filled_project();
        let track = device_track(&project);
        let (_, clip_id) = lead_clip(&project);
        let fresh = EntityId::from_str("01J8ZQ00000000000000000DEV").expect("ULID");
        let base = serde_json::json!({
            "deviceId": fresh.to_canonical_string(),
            "name": "Probe",
            "kind": "InternalEffect",
            "bypassed": false,
            "latencySamples": 0,
            "params": []
        });
        let with_device = |patch: &serde_json::Value| {
            let mut device = base.clone();
            if let (Some(target), Some(source)) = (device.as_object_mut(), patch.as_object()) {
                for (key, value) in source {
                    target.insert(key.clone(), value.clone());
                }
            }
            serde_json::json!([{
                "kind": INSERT_DEVICE_KIND,
                "trackId": track.to_canonical_string(),
                "device": device
            }])
        };
        // 形状错误可能在**解析期**（`parse_device` 自己的未知键 / 类型 / 值域闸门）
        // 或**编译期**（`compile` 的前置条件）报出。两者都是"本层响亮拒绝、带 `reason`"，
        // 因此本判据只看"最终报出来的那一条"，不钉它出自哪一步 —— 钉那一步会让判据
        // 在把闸门搬位置时无意义地变红。
        let expect_domain = |raw: &serde_json::Value, code: ErrorCode, reason: &str| {
            let fault = match parse_ops(raw) {
                Err(fault) => fault,
                Ok(ops) => compile(&project, &track, &clip_id, &ops).expect_err("必须被拒"),
            };
            assert_eq!(fault.domain_code(), Some(code), "报文 {fault:?}");
            assert_eq!(lane_fault_data(&fault)["reason"], serde_json::json!(reason));
        };

        // ① 操作对象里的未知键（把设备身份错写在顶层 —— 它属于 `device` 载荷）。
        // 解析期就拒（`reject_insert_device_fields`），因此这里读的是 `parse_ops` 的失败。
        let fault = parse_ops(&serde_json::json!([{
            "kind": INSERT_DEVICE_KIND,
            "trackId": track.to_canonical_string(),
            "deviceId": fresh.to_canonical_string(),
            "device": base.clone()
        }]))
        .expect_err("顶层多写 `deviceId` 必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "unknownInsertDeviceField"
        );

        // ② `device` 里的未知键（把 snake_case 的容器拼写搬进工具面）。
        expect_domain(
            &with_device(&serde_json::json!({"latency_samples": 32})),
            ErrorCode::InvalidParameterRange,
            "unknownDeviceField",
        );
        // ③ `params[]` 里的未知键。
        expect_domain(
            &with_device(
                &serde_json::json!({"params": [{"name": "a", "value": 1.0, "unitX": "dB"}]}),
            ),
            ErrorCode::InvalidParameterRange,
            "unknownDeviceParamField",
        );
        // ④ 未知的设备类型名。
        expect_domain(
            &with_device(&serde_json::json!({"kind": "SfzSampler"})),
            ErrorCode::InvalidParameterRange,
            "unknownDeviceKind",
        );
        // ⑤ `latencySamples` 超出 u32。
        expect_domain(
            &with_device(&serde_json::json!({"latencySamples": 4_294_967_296u64})),
            ErrorCode::OutOfRange,
            "latencyOutOfRange",
        );
        // ⑥ 空设备名（模型对 `DeviceDefinition::name` 没有校验 ⇒ 本层必须拦）。
        expect_domain(
            &with_device(&serde_json::json!({"name": ""})),
            ErrorCode::InvalidParameterRange,
            "deviceNameRequired",
        );
        // ⑦ 参数值在 f64 → f32 收窄**之后**非有限（`1e39` 在 f64 里有限、在 f32 里是
        // `inf` ⇒ 正是 `ef459f9` 修掉的那一类；`read_number` 判的是收窄之后的值）。
        expect_domain(
            &with_device(&serde_json::json!({"params": [{"name": "a", "value": 1e39}]})),
            ErrorCode::InvalidParameterRange,
            "nonFiniteValue",
        );
        // ⑧ `removeDevice` 上的 `device` 键（那是 `insertDevice` 的形状）。
        let fault = parse_ops(&serde_json::json!([{
            "kind": REMOVE_DEVICE_KIND,
            "trackId": track.to_canonical_string(),
            "deviceId": fresh.to_canonical_string(),
            "device": base
        }]))
        .expect_err("`removeDevice` 多写 `device` 必须被拒");
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "unknownRemoveDeviceField"
        );
    }

    /// 设备形态**自带寻址**：顶层 `trackId` / `clipId` 是 nil 占位也照样编译，
    /// 而且不要求片段是 MIDI；阴性对照是音符级形态必须响亮失败。
    ///
    /// 注入（实测红）：把 [`compile`] 入口的 `self_addressed_only` 分支换回
    /// `all_track_entity_level` ⇒ 本判据红（nil 顶层身份查不到音轨），
    /// 而 `remove_track_does_not_need_the_envelope_ids_or_midi` 仍绿。
    #[test]
    fn device_ops_do_not_need_the_envelope_ids_or_midi() {
        let project = filled_project();
        let track = device_track(&project);
        let existing = project.tracks[&track].devices[0].id;
        let fresh = EntityId::from_str("01J8ZQ00000000000000000DEV").expect("ULID");
        let audio_clip = project
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_none())
            .expect("样本里必须有非 MIDI 片段")
            .id;

        let ops = parse_ops(&serde_json::json!([
            {
                "kind": INSERT_DEVICE_KIND,
                "trackId": track.to_canonical_string(),
                "device": {
                    "deviceId": fresh.to_canonical_string(),
                    "name": "Probe",
                    "kind": "InternalEffect",
                    "bypassed": false,
                    "latencySamples": 0,
                    "params": []
                }
            },
            {
                "kind": REMOVE_DEVICE_KIND,
                "trackId": track.to_canonical_string(),
                "deviceId": existing.to_canonical_string()
            }
        ]))
        .expect("解析");
        // 两条设备形态**允许**同批（换一台设备本来就是两条操作）。
        let compiled = compile(&project, &track, &audio_clip, &ops)
            .expect("设备形态不读片段内容, 非 MIDI 片段也必须被接受");
        assert_eq!(compiled.len(), 2);

        // 顶层两个身份是 nil 占位 ⇒ 仍然编译（`all_self_addressed` 走的那条路）。
        let nil = EntityId::default();
        let compiled = compile(&project, &nil, &nil, &ops)
            .expect("设备形态自带寻址, 顶层 trackId / clipId 不被读取");
        assert_eq!(compiled.len(), 2);
        assert!(ops.iter().all(NoteOp::is_device_level));
        assert!(NoteOp::all_self_addressed(&ops));
        assert!(
            !NoteOp::all_track_entity_level(&ops),
            "设备形态**不是**音轨实体级 (那是取走整条音轨), 两个桶必须分得开"
        );

        // 阴性对照: 音符级形态 + nil 顶层身份必须响亮失败。
        let note_ops = parse_ops(&serde_json::json!([
            {"kind": "add", "note": {"startTick": 0, "pitch": 60, "durationTicks": 480}}
        ]))
        .expect("解析");
        let fault = compile(&project, &nil, &nil, &note_ops).expect_err("音符级必须要音轨");
        assert_eq!(fault.domain_code(), Some(ErrorCode::TrackNotFound));
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
    ///
    /// `#311c`：8 → **10**（`pitchBendCurve` / `slide`）。长度是硬读数的判据 ——
    /// 只 `contains` 不加长度不够：把某个旧键从表里删掉但把新键加上，集合仍是 10
    /// 条却少读一个字段（`unknown_note_fields...` 只测未知键，测不到这种"漏报"）。
    #[test]
    fn expressive_note_field_names_are_pinned() {
        assert_eq!(RATCHET_FIELD, "ratchet");
        assert_eq!(MICRO_TIMING_FIELD, "microTimingTicks");
        assert_eq!(PITCH_BEND_CURVE_FIELD, "pitchBendCurve");
        assert_eq!(SLIDE_FIELD, "slide");
        assert_eq!(SLIDE_DURATION_FIELD, "durationTicks");
        assert_eq!(SLIDE_TARGET_PITCH_FIELD, "targetPitch");
        assert_eq!(SLIDE_CURVE_FIELD, "curve");
        for expected in [
            "id",
            "startTick",
            "pitch",
            "durationTicks",
            "velocity",
            "probability",
            "ratchet",
            "microTimingTicks",
            "pitchBendCurve",
            "slide",
        ] {
            assert!(
                NOTE_FIELDS.contains(&expected),
                "支持集合必须含 {expected}: {NOTE_FIELDS:?}"
            );
        }
        assert_eq!(
            NOTE_FIELDS.len(),
            10,
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

    // -----------------------------------------------------------------------
    // 值域的"两份字面"：机械对账（#330）
    //
    // 三个表现力字段的区间**只允许**有一个来源：`yeban_model::music` 的常量。
    // 下面三条判据把"模型 / MCP 预判 / 广播 schema / 报文"四侧钉在一起：
    // (a) schema 的 min/max 与常量**数值**对账（⛔ 不比拼法：`add.probability`
    //     刻意保留整数 `0/1`，改拼法会让 `tools/list` 动字节）；
    // (b) 边界值穿四层逐值一致（含 MCP 单独的预判 —— 只测 `add` 解析会漏掉
    //     "MCP 放松、模型收紧"那种分叉，因为解析结果被模型校验再兜一次）；
    // (c) 报文（模型 Display 与 MCP 越界报文）与常量对账。
    // -----------------------------------------------------------------------

    /// `ops` item schema 里某个分支某字段的 `(minimum, maximum)`（**数值**口径，
    /// 与 JSON 里写的是 `16` 还是 `16.0` 无关）。
    fn schema_bounds(kind_name: &str, field: &str) -> (f64, f64) {
        let items = crate::op_schema::ops_items();
        let branch = items["oneOf"]
            .as_array()
            .expect("`oneOf` 必须是数组")
            .iter()
            .find(|branch| branch["properties"]["kind"]["const"].as_str() == Some(kind_name))
            .unwrap_or_else(|| panic!("schema 里必须有 `{kind_name}` 分支"));
        // `add` 把音符载荷嵌在 `properties.note.properties` 下；其余分支是平铺的。
        let direct = &branch["properties"][field];
        let property = if direct.is_object() {
            direct
        } else {
            &branch["properties"]["note"]["properties"][field]
        };
        assert!(
            !property.is_null(),
            "schema 的 `{kind_name}` 分支必须声明 `{field}`"
        );
        (
            property["minimum"].as_f64().expect("minimum 必须是数字"),
            property["maximum"].as_f64().expect("maximum 必须是数字"),
        )
    }

    /// 三个表现力字段（探针表的字段维）。
    #[derive(Clone, Copy, Debug)]
    enum ExpressionRange {
        Probability,
        Ratchet,
        MicroTiming,
    }

    impl ExpressionRange {
        const fn field(self) -> &'static str {
            match self {
                Self::Probability => PROBABILITY_FIELD,
                Self::Ratchet => RATCHET_FIELD,
                Self::MicroTiming => MICRO_TIMING_FIELD,
            }
        }

        /// 把探针值变成该字段的 JSON 形状（概率是小数，另外两个是整数）。
        fn json(self, value: f64) -> Value {
            match self {
                Self::Probability => serde_json::json!(value),
                Self::Ratchet | Self::MicroTiming => serde_json::json!(value as i64),
            }
        }
    }

    /// 探针表：票面那 11 个边界值 ＋ 两个补洞探针（`-0.001` 守概率下界、
    /// `0.5` 守中段），`bool` = 修前/修后都必须给出的结论。
    const EXPRESSIVE_PROBES: [(ExpressionRange, f64, bool); 13] = [
        (ExpressionRange::Probability, 0.0, true),
        (ExpressionRange::Probability, 1.0, true),
        (ExpressionRange::Probability, 0.999, true),
        (ExpressionRange::Probability, 1.001, false),
        (ExpressionRange::Probability, -0.001, false),
        (ExpressionRange::Probability, 0.5, true),
        (ExpressionRange::Ratchet, 1.0, true),
        (ExpressionRange::Ratchet, 16.0, true),
        (ExpressionRange::Ratchet, 17.0, false),
        (ExpressionRange::Ratchet, 0.0, false),
        (ExpressionRange::MicroTiming, -240.0, true),
        (ExpressionRange::MicroTiming, 240.0, true),
        (ExpressionRange::MicroTiming, 241.0, false),
    ];

    /// 第一层：MCP **自己的**预判（`read_*`），不含模型兜底。
    fn mcp_read_accepts(field: ExpressionRange, value: f64) -> bool {
        let mut object = Map::new();
        object.insert(field.field().to_owned(), field.json(value));
        match field {
            ExpressionRange::Probability => read_probability(&object).is_ok(),
            ExpressionRange::Ratchet => read_ratchet(&object).is_ok(),
            ExpressionRange::MicroTiming => read_micro_timing(&object).is_ok(),
        }
    }

    /// 第二层：`add` 的 JSON 解析（MCP 预判 ＋ 模型校验的合取）。
    fn add_accepts(field: ExpressionRange, value: f64) -> bool {
        let mut item = add_json(0, 60);
        item["note"][field.field()] = field.json(value);
        parse_ops(&Value::Array(vec![item])).is_ok()
    }

    /// 第三层：`setNoteExpression` 的 JSON 解析（同一份 MCP 预判的另一条入口）。
    fn set_note_expression_accepts(field: ExpressionRange, value: f64) -> bool {
        let mut item = serde_json::json!({
            "kind": SET_NOTE_EXPRESSION_KIND,
            "noteId": deterministic_id("expressive-range-probe"),
            PROBABILITY_FIELD: Value::Null,
            RATCHET_FIELD: Value::Null,
            MICRO_TIMING_FIELD: Value::Null,
        });
        item[field.field()] = field.json(value);
        parse_ops(&Value::Array(vec![item])).is_ok()
    }

    /// 第四层：模型自己的 `MidiNote::validate`。
    fn model_accepts(field: ExpressionRange, value: f64) -> bool {
        let mut note = MidiNote::new(deterministic_id("expressive-range-probe"), 0, 60, 480);
        note.velocity = 100;
        match field {
            ExpressionRange::Probability => note.probability = Some(value as f32),
            ExpressionRange::Ratchet => {
                note.ratchet = Some(u8::try_from(value as i64).expect("探针在 u8 内"));
            }
            ExpressionRange::MicroTiming => {
                note.micro_timing_ticks = Some(i16::try_from(value as i64).expect("探针在 i16 内"));
            }
        }
        note.validate().is_ok()
    }

    /// 第五层：模型 `Op::SetNoteExpression::precondition`（`#329` 新增的第二份
    /// 概率字面就在那里）。
    fn op_precondition_accepts(field: ExpressionRange, value: f64) -> bool {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let notes = project.clip_pool[&clip_id]
            .content
            .notes()
            .expect("样本片段必须是 MIDI");
        let note_id = *notes.keys().next().expect("样本片段必须至少一个音符");
        let current = notes.get(&note_id).expect("音符存在").clone();
        let (new_probability, new_ratchet, new_micro_timing_ticks) = match field {
            ExpressionRange::Probability => (
                Some(value as f32),
                current.ratchet,
                current.micro_timing_ticks,
            ),
            ExpressionRange::Ratchet => (
                current.probability,
                Some(u8::try_from(value as i64).expect("探针在 u8 内")),
                current.micro_timing_ticks,
            ),
            ExpressionRange::MicroTiming => (
                current.probability,
                current.ratchet,
                Some(i16::try_from(value as i64).expect("探针在 i16 内")),
            ),
        };
        let op = Op::SetNoteExpression {
            track_id,
            clip_id,
            note_id,
            old_probability: current.probability,
            old_ratchet: current.ratchet,
            old_micro_timing_ticks: current.micro_timing_ticks,
            new_probability,
            new_ratchet,
            new_micro_timing_ticks,
        };
        op.precondition(&project).is_ok()
    }

    /// (a) 广播 schema 的 8 组 min/max 与模型常量**数值**对账。
    ///
    /// 修前注入（实测，D）：只把 `add.ratchet.maximum` 从 `16` 改成 `15`
    /// ⇒ `cargo test -p yeban-mcp --locked --no-fail-fast` 的 39 个 test 二进制
    /// 全绿 ⇒ 本判据就是为那一条裸着的字面加的。
    #[test]
    fn expressive_schema_bounds_reconcile_with_the_model() {
        let pitch_max = f64::from(yeban_model::music::MIDI_PITCH_MAX);
        let velocity_max = f64::from(yeban_model::music::MIDI_VELOCITY_MAX);
        let probability_min = f64::from(PROBABILITY_MIN);
        let probability_max = f64::from(PROBABILITY_MAX);
        let ratchet_min = f64::from(RATCHET_MIN);
        let ratchet_max = f64::from(RATCHET_MAX);
        let micro_min = f64::from(-MICRO_TIMING_MAX_ABS);
        let micro_max = f64::from(MICRO_TIMING_MAX_ABS);
        for (kind_name, field, expected) in [
            ("add", PROBABILITY_FIELD, (probability_min, probability_max)),
            ("add", RATCHET_FIELD, (ratchet_min, ratchet_max)),
            ("add", MICRO_TIMING_FIELD, (micro_min, micro_max)),
            ("add", "pitch", (0.0, pitch_max)),
            ("add", "velocity", (0.0, velocity_max)),
            (
                "setNoteExpression",
                PROBABILITY_FIELD,
                (probability_min, probability_max),
            ),
            (
                "setNoteExpression",
                RATCHET_FIELD,
                (ratchet_min, ratchet_max),
            ),
            (
                "setNoteExpression",
                MICRO_TIMING_FIELD,
                (micro_min, micro_max),
            ),
            ("velocity", "velocity", (0.0, velocity_max)),
            ("setTempo", "bpm", (MIN_BPM, MAX_BPM)),
        ] {
            assert_eq!(
                schema_bounds(kind_name, field),
                expected,
                "`{kind_name}.{field}` 的广播区间必须与模型常量同源"
            );
        }
    }

    /// (b) 边界值穿**五层**逐值一致（票面的"四层"＋ MCP 单独的预判这一层）。
    ///
    /// 修前注入（实测，C1）：只把模型 `music.rs` 的概率上界从 `1.0` 改成 `1.2`
    /// ⇒ model 28/28、MCP 5/5 全绿；修后本判据在 `1.001` 处红（MCP 拒绝、
    /// `MidiNote::validate` 接受、`Op::precondition` 拒绝）。
    #[test]
    fn expressive_boundaries_agree_across_every_layer() {
        for (field, value, expected) in EXPRESSIVE_PROBES {
            let read = mcp_read_accepts(field, value);
            let add = add_accepts(field, value);
            let expression = set_note_expression_accepts(field, value);
            let model = model_accepts(field, value);
            let op = op_precondition_accepts(field, value);
            println!(
                "[range] {field:?} {value} => expected={expected} mcp={read} add={add} \
                 setNoteExpression={expression} model={model} op={op}"
            );
            assert_eq!(read, expected, "{field:?} {value}: MCP 预判");
            assert_eq!(add, expected, "{field:?} {value}: `add` 解析");
            assert_eq!(
                expression, expected,
                "{field:?} {value}: `setNoteExpression` 解析"
            );
            assert_eq!(model, expected, "{field:?} {value}: `MidiNote::validate`");
            assert_eq!(
                op, expected,
                "{field:?} {value}: `Op::SetNoteExpression::precondition`"
            );
        }
    }

    /// (c) 三条模型 `Display` 报文与 MCP 的 probability 越界报文：逐字且与常量同源。
    #[test]
    fn expressive_range_messages_speak_the_model_bounds() {
        assert_eq!(
            yeban_model::ModelError::ProbabilityOutOfRange { value: 1.5 }.to_string(),
            format!("probability 1.5 out of range {PROBABILITY_MIN:?}..={PROBABILITY_MAX:?}")
        );
        assert_eq!(
            yeban_model::ModelError::RatchetOutOfRange { value: 17 }.to_string(),
            format!("ratchet 17 out of range {RATCHET_MIN}..={RATCHET_MAX}")
        );
        assert_eq!(
            yeban_model::ModelError::MicroTimingOutOfRange { value: 241 }.to_string(),
            format!(
                "micro timing 241 ticks out of range -{MICRO_TIMING_MAX_ABS}..={MICRO_TIMING_MAX_ABS}"
            )
        );
        // 本轮把 MCP 的 probability 报文改成 `{:?}` 拼常量 ⇒ 字节必须不变。
        let mut object = Map::new();
        object.insert(PROBABILITY_FIELD.to_owned(), serde_json::json!(1.001));
        let fault = read_probability(&object).expect_err("1.001 必须被拒绝");
        let Fault::Domain { message, data, .. } = &fault else {
            panic!("必须是领域失败");
        };
        assert_eq!(message.as_str(), "`probability` 越界: 1.001 不在 0.0..=1.0");
        let data = data.as_ref().expect("必须带 data");
        assert_eq!(data["min"], 0.0);
        assert_eq!(data["max"], 1.0);
    }

    // -----------------------------------------------------------------------
    // 值域的"两份字面"：`pitch` / `velocity` / `bpm`（#331，照 #330 先例）
    //
    // 四个解析点（`add.pitch` / `add.velocity` / `velocity` / `move` 的平移前置）
    // 与一处 `scene_launch` 的 tempo 区间此前各写一份 `127` / `999.0`。
    // 下面三条判据把"模型常量 / MCP 预判 / 广播 schema / 报文"钉在一起。
    // -----------------------------------------------------------------------

    fn add_pitch_accepts(pitch: u8) -> bool {
        parse_ops(&Value::Array(vec![serde_json::json!({
            "kind": "add",
            "note": {"startTick": 0, "pitch": pitch, "durationTicks": 480},
        })]))
        .is_ok()
    }

    fn add_velocity_accepts(velocity: u8) -> bool {
        parse_ops(&Value::Array(vec![serde_json::json!({
            "kind": "add",
            "note": {
                "startTick": 0,
                "pitch": 60,
                "durationTicks": 480,
                "velocity": velocity,
            },
        })]))
        .is_ok()
    }

    fn velocity_op_accepts(velocity: u8) -> bool {
        parse_ops(&Value::Array(vec![serde_json::json!({
            "kind": "velocity",
            "noteId": deterministic_id("pitch-velocity-probe"),
            "velocity": velocity,
        })]))
        .is_ok()
    }

    fn model_pitch_accepts(pitch: u8) -> bool {
        let mut note = MidiNote::new(deterministic_id("pitch-velocity-probe"), 0, pitch, 480);
        note.velocity = 100;
        note.validate().is_ok()
    }

    fn model_velocity_accepts(velocity: u8) -> bool {
        let mut note = MidiNote::new(deterministic_id("pitch-velocity-probe"), 0, 60, 480);
        note.velocity = velocity;
        note.validate().is_ok()
    }

    /// `move` 的平移前置（`compile` 里的 `0..=MIDI_PITCH_MAX`）。
    fn move_pitch_accepts(base_pitch: u8, delta_pitch: i8) -> bool {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let note_id = *project.clip_pool[&clip_id]
            .content
            .notes()
            .expect("样本片段必须是 MIDI")
            .keys()
            .next()
            .expect("样本片段必须至少一个音符");
        project
            .clip_pool
            .get_mut(&clip_id)
            .expect("片段存在")
            .content
            .notes_mut()
            .expect("样本片段必须是 MIDI")
            .get_mut(&note_id)
            .expect("音符存在")
            .pitch = base_pitch;
        let ops = parse_ops(&Value::Array(vec![serde_json::json!({
            "kind": "move",
            "noteId": note_id.to_canonical_string(),
            "deltaTick": 0,
            "deltaPitch": delta_pitch,
        })]))
        .expect("解析");
        compile(&project, &track_id, &clip_id, &ops).is_ok()
    }

    /// 3 个值 × 3 个 MCP 入口 ＋ 模型校验：逐值同结论（127 含、128 拒）。
    ///
    /// 修前注入（实测，本单）：只把 `notes.rs:3792`/`6550`/`6554` 的
    /// `0, 127` 之一改成 `0, 126` ⇒ 全套 39 个 MCP test 二进制
    /// **989 passed / 0 failed 全绿** ⇒ 本判据就是为那三处裸字面加的。
    #[test]
    fn pitch_and_velocity_boundaries_agree_across_every_layer() {
        for (value, expected) in [(0_u8, true), (127, true), (128, false)] {
            assert_eq!(add_pitch_accepts(value), expected, "`add.pitch` = {value}");
            assert_eq!(
                add_velocity_accepts(value),
                expected,
                "`add.velocity` = {value}"
            );
            assert_eq!(
                velocity_op_accepts(value),
                expected,
                "`velocity.velocity` = {value}"
            );
            assert_eq!(model_pitch_accepts(value), expected, "模型 pitch = {value}");
            assert_eq!(
                model_velocity_accepts(value),
                expected,
                "模型 velocity = {value}"
            );
        }

        // 越界报文与 `data.max` 也必须指向模型常量（不是第二个字面）。
        let pitch_fault = parse_ops(&Value::Array(vec![serde_json::json!({
            "kind": "add",
            "note": {"startTick": 0, "pitch": 128, "durationTicks": 480},
        })]))
        .expect_err("音高越界");
        let Fault::Domain { message, data, .. } = &pitch_fault else {
            panic!("必须是领域失败");
        };
        assert_eq!(
            message.as_str(),
            format!("`pitch` 越界: 128 不在 0..={MIDI_PITCH_MAX}")
        );
        assert_eq!(
            data.as_ref().expect("必须带 data")["max"].as_u64(),
            Some(u64::from(MIDI_PITCH_MAX))
        );

        let velocity_fault = parse_ops(&Value::Array(vec![serde_json::json!({
            "kind": "velocity",
            "noteId": deterministic_id("pitch-velocity-probe"),
            "velocity": 128,
        })]))
        .expect_err("力度越界");
        let Fault::Domain { message, data, .. } = &velocity_fault else {
            panic!("必须是领域失败");
        };
        assert_eq!(
            message.as_str(),
            format!("`velocity` 越界: 128 不在 0..={MIDI_VELOCITY_MAX}")
        );
        assert_eq!(
            data.as_ref().expect("必须带 data")["max"].as_u64(),
            Some(u64::from(MIDI_VELOCITY_MAX))
        );
    }

    /// `move` 的两个端点都**含**：127+0 收、127+1 拒、0+0 收、0−1 拒。
    ///
    /// 修前注入（实测，本单）：只把 `notes.rs:6845` 的 `0..=127` 改成
    /// `0..=126` ⇒ 同一套 39 个 test 二进制全绿。
    #[test]
    fn move_pitch_boundaries_are_inclusive_at_both_ends() {
        for (base, delta, expected) in [
            (127_u8, 0_i8, true),
            (127, 1, false),
            (0, 0, true),
            (0, -1, false),
        ] {
            assert_eq!(
                move_pitch_accepts(base, delta),
                expected,
                "move {base} + {delta}"
            );
        }
    }

    /// `bpm` 四值穿过 `setTempo` 的 `compile` 前置（模型常量是唯一区间来源）。
    #[test]
    fn bpm_boundaries_agree_between_schema_and_compile() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        for (bpm, expected) in [
            (19.0_f64, false),
            (20.0, true),
            (999.0, true),
            (1000.0, false),
        ] {
            let ops = parse_ops(&Value::Array(vec![serde_json::json!({
                "kind": SET_TEMPO_KIND,
                "bpm": bpm,
            })]))
            .expect("解析不查区间");
            assert_eq!(
                compile(&project, &track_id, &clip_id, &ops).is_ok(),
                expected,
                "bpm = {bpm}"
            );
        }
    }

    /// (c) `bpm` 的 schema 描述与发布端点逐字＝模型常量。
    ///
    /// ⚠ `setScene.tempo` 的 schema **没有** `minimum`/`maximum`（只有描述）——
    /// 给它加数字字段会动 `tools/list` 字节，因此 (a) 只对账 `setTempo.bpm`。
    #[test]
    fn bpm_schema_and_messages_speak_the_model_bounds() {
        let items = crate::op_schema::ops_items();
        let branches = items["oneOf"].as_array().expect("`oneOf` 必须是数组");
        let branch = |name: &str| {
            branches
                .iter()
                .find(|branch| branch["properties"]["kind"]["const"].as_str() == Some(name))
                .unwrap_or_else(|| panic!("schema 里必须有 `{name}` 分支"))
        };

        let set_tempo = branch("setTempo");
        let expected = format!("写工程速度（BPM {MIN_BPM:?}..={MAX_BPM:?}）");
        assert!(
            set_tempo["description"]
                .as_str()
                .expect("description")
                .starts_with(&expected),
            "`setTempo` 描述必须拼常量: {set_tempo}"
        );
        let set_scene = branch("setScene");
        let expected = format!("速度覆盖（BPM {MIN_BPM}..={MAX_BPM}）；null = 清空 = 跟随工程速度");
        assert_eq!(
            set_scene["properties"]["scene"]["properties"]["tempo"]["description"].as_str(),
            Some(expected.as_str()),
            "`setScene.tempo` 描述必须拼常量"
        );
        // 已发布端点：20.0 / 999.0（`#330` 的 `project_create` 判据同口径）。
        assert_eq!((MIN_BPM, MAX_BPM), (20.0, 999.0));
    }

    /// `note` 里的未知键 ⇒ 响亮拒绝（列出支持集合），**绝不**静默丢弃。
    ///
    /// 与顶层实参口径同源（`tools.rs` 的 `UnknownParam`：拼错的参数必须被拒绝、不能静默
    /// 忽略）—— 同一条纪律不许只守一层。`#311c` 之后只剩 `syllable` / `phonemes`
    /// 两个模型字段**仍然**没有工具面通路；`slide` / `pitchBendCurve` 已经接线，
    /// 因此 ⛔ 不许再出现在 `unsupportedNoteFields` 里（把它们说成"不支持"就是假话）。
    #[test]
    fn unknown_note_fields_are_rejected_with_the_supported_set() {
        // ⚠ `"slide"` 已在 `#311c` 从"未知键"样本里移除 —— 它现在是**合法字段**。
        for unknown in [
            "slyde",
            "ratchett",
            "microtimingticks",
            "noteId",
            "pitchbend",
        ] {
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
            assert_eq!(
                supported.len(),
                10,
                "支持集合必须如实报 10 条（8 → +pitchBendCurve +slide）: {data}"
            );
            for expected in [
                "startTick",
                "pitch",
                "ratchet",
                "microTimingTicks",
                "pitchBendCurve",
                "slide",
            ] {
                assert!(
                    supported.iter().any(|value| value == expected),
                    "支持集合必须含 {expected}: {data}"
                );
            }
            assert_eq!(
                data["unsupportedNoteFields"],
                serde_json::json!(["syllable", "phonemes"]),
                "仍然没有通路的只有 syllable / phonemes: {data}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // 弯音曲线 / 滑音字段（`add.note.pitchBendCurve` / `add.note.slide`）
    // —— `#311b` 已让引擎真的求值，`#311c` 补上工具面写入通路
    // -----------------------------------------------------------------------

    /// 一条带曲线/滑音的 `add` 音符的完整读回：先解析、再编译、再 `apply`，
    /// 最后从**工程文档**里读回那个音符（⛔ 不只看解析结果）。
    ///
    /// `note_id` 显式给出，才能从 `BTreeMap` 里定位"这一次 add 的那一个音符"。
    fn apply_bent_note(curve: Value, slide: Value) -> (YebanProjectV1, EntityId, Value, Vec<Op>) {
        let note_id = EntityId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").expect("ULID");
        let mut item = add_json(0, 69);
        item["note"]["id"] = serde_json::json!(note_id.to_canonical_string());
        item["note"]["pitchBendCurve"] = curve.clone();
        item["note"]["slide"] = slide.clone();
        item["note"]["durationTicks"] = serde_json::json!(1_920);
        let ops = parse_ops(&Value::Array(vec![item])).expect("解析");
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        let mut after = project.clone();
        Op::Batch {
            ops: compiled.clone(),
            description: "bend-slide 判据".to_owned(),
        }
        .apply(&mut after)
        .expect("施加");
        (after, note_id, curve, compiled)
    }

    /// 判据 1（往返 + D4）：`add` 带着**首点 tick = 240** 的曲线与一条滑音，
    /// 落地后工程里的音符逐字段等于请求里的值。
    ///
    /// 首点 240 是 `ADR-0012` D4 的故意样本：它在 2026-10-25 之前会被模型拒绝
    /// （`PitchBendFirstTickNotZero`），今天是**合法**的，前 240 tick 由消费侧读作 0。
    #[test]
    fn bend_curve_and_slide_round_trip_into_the_project() {
        let curve = serde_json::json!([[240, 200], [480, -300], [960, 0]]);
        let slide = serde_json::json!({
            "durationTicks": 480,
            "targetPitch": 81,
            "curve": "SCurve",
        });
        let (after, note_id, curve_json, compiled) = apply_bent_note(curve, slide);
        let note = after
            .clip_pool
            .values()
            .find_map(|entry| entry.content.notes().and_then(|notes| notes.get(&note_id)))
            .expect("工程里必须有这次 add 的音符");
        assert_eq!(
            note.pitch_bend_curve,
            vec![(240u64, 200i16), (480, -300), (960, 0)],
            "曲线必须逐点落进工程（首点 240 是 D4 的合法形态）"
        );
        assert_eq!(
            note.slide,
            Some(SlideConfig {
                duration_ticks: 480,
                target_pitch: 81,
                curve: yeban_model::CurveType::SCurve,
            }),
            "滑音必须逐字段落进工程（curve 走 canonical 名字表）"
        );
        // 编译出来的 `Op::AddNote` 也逐字段相等（不是"只在解析结果里有"）。
        match &compiled[0] {
            Op::AddNote { note, .. } => {
                assert_eq!(
                    note.pitch_bend_curve,
                    vec![(240, 200), (480, -300), (960, 0)]
                );
                assert_eq!(
                    note.slide
                        .map(|slide| (slide.duration_ticks, slide.target_pitch)),
                    Some((480, 81))
                );
            }
            other => panic!("必须是 AddNote, 实际 {other:?}"),
        }
        // 曲线/滑音的 JSON 字面量原样（防止 helper 自己改写了样本）。
        assert_eq!(
            curve_json,
            serde_json::json!([[240, 200], [480, -300], [960, 0]])
        );
    }

    /// 判据 2a（`cents` 闭区间）：-1200 / 0 / +1200 收，-1201 / +1201 拒，
    /// 且拒绝码是 `OUT_OF_RANGE`、`data` 报出错的下标。
    #[test]
    fn bend_curve_cents_boundaries_are_inclusive() {
        let note_id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        for (cents, accepted) in [
            (-1_200i64, true),
            (0, true),
            (1_200, true),
            (-1_201, false),
            (1_201, false),
        ] {
            let mut item = add_json(0, 60);
            item["note"]["id"] = serde_json::json!(note_id);
            item["note"]["pitchBendCurve"] = serde_json::json!([[0, 0], [480, cents]]);
            let parsed = parse_ops(&Value::Array(vec![item]));
            assert_eq!(
                parsed.is_ok(),
                accepted,
                "cents = {cents} 的接受性必须与 D2 一致"
            );
            if let Err(fault) = parsed {
                assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange), "{cents}");
                let Fault::Domain { data, .. } = &fault else {
                    panic!("必须是领域失败");
                };
                let data = data.as_ref().expect("必须带 data");
                assert_eq!(data["index"], 1, "必须报出错的下标: {data}");
                assert_eq!(data["field"], PITCH_BEND_CURVE_FIELD);
                assert_eq!(data["min"], -1_200);
                assert_eq!(data["max"], 1_200);
            }
        }
    }

    /// 判据 2b（tick 严格升序）：`[0, 480]` 收、`[480, 480]` 拒、`[480, 240]` 拒；
    /// 拒绝码是 `INVALID_PARAMETER_RANGE`（形状，不是量），报文点名字段与下标。
    #[test]
    fn bend_curve_ticks_must_be_strictly_ascending() {
        for (ticks, accepted) in [
            ([0u64, 480], true),
            ([480, 480], false),
            ([480, 240], false),
        ] {
            let mut item = add_json(0, 60);
            item["note"]["pitchBendCurve"] = serde_json::json!([[ticks[0], 10], [ticks[1], 20]]);
            let parsed = parse_ops(&Value::Array(vec![item]));
            assert_eq!(parsed.is_ok(), accepted, "ticks = {ticks:?}");
            if let Err(fault) = parsed {
                assert_eq!(
                    fault.domain_code(),
                    Some(ErrorCode::InvalidParameterRange),
                    "升序是形状错误（ADR-0012 D7）"
                );
                let Fault::Domain { message, .. } = &fault else {
                    panic!("必须是领域失败");
                };
                assert!(
                    message.contains(PITCH_BEND_CURVE_FIELD) && message.contains("[1]"),
                    "报文必须点名字段与出错下标: {message}"
                );
            }
        }
    }

    /// 判据 2c（结构）：不是数组 / 不是二元组 / 长度不是 2 / tick 不是非负整数 /
    /// cents 不是整数 ⇒ 全部 `INVALID_PARAMETER_RANGE`。
    #[test]
    fn bend_curve_shape_errors_are_loud() {
        for bad in [
            serde_json::json!(1),
            serde_json::json!("[[0,0]]"),
            serde_json::json!([[0]]),
            serde_json::json!([[0, 0, 0]]),
            serde_json::json!([["0", 0]]),
            serde_json::json!([[-1, 0]]),
            serde_json::json!([[0, 1.5]]),
            serde_json::json!([[0, "x"]]),
            serde_json::json!([{}]),
        ] {
            let mut item = add_json(0, 60);
            item["note"]["pitchBendCurve"] = bad.clone();
            let fault = parse_ops(&Value::Array(vec![item])).expect_err("必须拒绝");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "输入 {bad}"
            );
        }
    }

    /// 判据 2d（错误优先级）：`[[0, 99999], [0, 0]]` 同时违反"升序"与"cents"，
    /// 两层都必须报**升序**（模型 `validate` 先扫窗口、再扫 cents，`ADR-0012` D7）。
    /// 修法注入：把 `read_pitch_bend_curve` 的两遍扫描合并成逐点判 ⇒ 本判据红。
    #[test]
    fn bend_curve_error_order_matches_the_model() {
        let mut item = add_json(0, 60);
        item["note"]["pitchBendCurve"] = serde_json::json!([[0, 99_999], [0, 0]]);
        let fault = parse_ops(&Value::Array(vec![item])).expect_err("必须拒绝");
        assert_eq!(
            fault.domain_code(),
            Some(ErrorCode::InvalidParameterRange),
            "先报升序 ⇒ INVALID_PARAMETER_RANGE，⛔ 不是 cents 的 OUT_OF_RANGE"
        );
        // 模型对**同一份**载荷报的也是升序（`cents` 取 `i16::MAX`，同样越界）。
        let model_error = MidiNote {
            pitch_bend_curve: vec![(0, i16::MAX), (0, 0)],
            ..MidiNote::new(
                EntityId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").expect("ULID"),
                0,
                60,
                480,
            )
        }
        .validate()
        .expect_err("模型必须拒绝");
        assert!(
            matches!(
                model_error,
                yeban_model::ModelError::PitchBendTicksNotAscending { .. }
            ),
            "模型必须报升序: {model_error:?}"
        );
    }

    /// 判据 2e / 长数组决定：**不设点数上界**。一条 512 点的合法曲线逐点保留
    /// （若日后有人加一个"最多 4096 点"之类的门面层上界，本判据只会在越界时红，
    /// 所以这里同时钉住"上界不存在"这份决定的可观察后果：大数组不被拒）。
    #[test]
    fn long_bend_curves_are_accepted_and_preserved() {
        let points: Vec<Value> = (0..512u64)
            .map(|index| {
                let cents = i64::try_from(index).unwrap_or(0) - 1_200;
                serde_json::json!([index * 2, cents])
            })
            .collect();
        let mut item = add_json(0, 60);
        item["note"]["pitchBendCurve"] = Value::Array(points);
        let ops = parse_ops(&Value::Array(vec![item])).expect("解析");
        let NoteOp::Add { note } = &ops[0] else {
            panic!("必须是 add");
        };
        assert_eq!(note.pitch_bend_curve.len(), 512, "512 点必须逐点保留");
        assert_eq!(note.pitch_bend_curve[511], (1_022, -689));
    }

    /// 判据 3（滑音边界）：`durationTicks = 0` 拒、`targetPitch = 127` 收 / 128 拒、
    /// 未知曲线名拒且列出允许名、非规范拼写（小写 `linear`）拒、缺必填键拒、
    /// 对象内未知键拒且列出 `supportedSlideFields`。
    #[test]
    fn slide_boundaries_and_curve_spelling_are_loud() {
        let base = serde_json::json!({"durationTicks": 480, "targetPitch": 81});
        // 正对照：完整滑音可解析。
        let mut item = add_json(0, 60);
        item["note"]["slide"] = base.clone();
        let ops = parse_ops(&Value::Array(vec![item])).expect("基线滑音必须可解析");
        let NoteOp::Add { note } = &ops[0] else {
            panic!("必须是 add");
        };
        assert_eq!(
            note.slide,
            Some(SlideConfig {
                duration_ticks: 480,
                target_pitch: 81,
                curve: yeban_model::CurveType::Linear,
            }),
            "缺省 curve = Linear"
        );

        // 边界：targetPitch 127 收 / 128 拒；durationTicks 0 拒。
        for (slide, accepted, code) in [
            (
                serde_json::json!({"durationTicks": 480, "targetPitch": 127}),
                true,
                None,
            ),
            (
                serde_json::json!({"durationTicks": 480, "targetPitch": 128}),
                false,
                Some(ErrorCode::OutOfRange),
            ),
            (
                serde_json::json!({"durationTicks": 0, "targetPitch": 81}),
                false,
                Some(ErrorCode::OutOfRange),
            ),
        ] {
            let mut item = add_json(0, 60);
            item["note"]["slide"] = slide.clone();
            let parsed = parse_ops(&Value::Array(vec![item]));
            assert_eq!(parsed.is_ok(), accepted, "slide = {slide}");
            if let Err(fault) = parsed {
                assert_eq!(fault.domain_code(), code, "slide = {slide}");
            }
        }

        // 未知曲线名 ⇒ 拒，且 `allowed` 列出四个规范名。
        let mut item = add_json(0, 60);
        item["note"]["slide"] = serde_json::json!({
            "durationTicks": 480, "targetPitch": 81, "curve": "Sine",
        });
        let fault = parse_ops(&Value::Array(vec![item])).expect_err("未知曲线必须拒绝");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let Fault::Domain { data, .. } = &fault else {
            panic!("必须是领域失败");
        };
        let data = data.as_ref().expect("必须带 data");
        assert_eq!(
            data["allowed"],
            serde_json::json!(["Linear", "Exponential", "Logarithmic", "SCurve"]),
            "必须列出与模型枚举双射的四个规范名: {data}"
        );
        assert_eq!(data["field"], "slide.curve");

        // 非规范拼写（小写）必须拒 —— 别名不留情面。
        for spelling in ["linear", "Linear ", "SCurve ", "s_curve"] {
            let mut item = add_json(0, 60);
            item["note"]["slide"] = serde_json::json!({
                "durationTicks": 480, "targetPitch": 81, "curve": spelling,
            });
            let fault = parse_ops(&Value::Array(vec![item])).expect_err("别名必须拒绝");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "拼写 {spelling:?}"
            );
        }

        // 缺必填键：没有目标的滑音不是滑音，没有时值也无从排程。
        for partial in [
            serde_json::json!({"durationTicks": 480}),
            serde_json::json!({"targetPitch": 81}),
            serde_json::json!({}),
        ] {
            let mut item = add_json(0, 60);
            item["note"]["slide"] = partial.clone();
            let fault = parse_ops(&Value::Array(vec![item])).expect_err("缺必填必须拒绝");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "slide = {partial}"
            );
        }

        // 对象内未知键 ⇒ 拒并列出支持集合。
        let mut item = add_json(0, 60);
        item["note"]["slide"] = serde_json::json!({
            "durationTicks": 480, "targetPitch": 81, "bend": 1,
        });
        let fault = parse_ops(&Value::Array(vec![item])).expect_err("未知键必须拒绝");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let Fault::Domain { data, .. } = &fault else {
            panic!("必须是领域失败");
        };
        assert_eq!(
            data.as_ref().expect("必须带 data")["supportedSlideFields"],
            serde_json::json!(["durationTicks", "targetPitch", "curve"])
        );
    }

    /// 判据：两个字段缺省时是模型的零值（`Vec::new()` / `None`），⛔ 不是"缺键报错"。
    #[test]
    fn add_without_bend_or_slide_keeps_the_model_defaults() {
        let ops = parse_ops(&Value::Array(vec![add_json(0, 60)])).expect("解析");
        let NoteOp::Add { note } = &ops[0] else {
            panic!("必须是 add");
        };
        assert!(note.pitch_bend_curve.is_empty(), "缺省 = 无弯音");
        assert_eq!(note.slide, None, "缺省 = 无滑音");
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
            [
                "startTick",
                "durationTicks",
                "placementId",
                "muted",
                "loopEnabled",
                "loopStartTick",
                "loopEndTick",
            ]
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
        // R75: 纯函数自比（同输入两次必须同值）—— 牙长在**确定性**上。
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
        for unknown in ["start", "starttick", "repeatCount", "clipId"] {
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

    /// ⭐ `#241`：三个循环键**逐字**落进 `LoopConfig`，且边界响亮拒绝。
    #[test]
    fn loop_fields_reach_the_placement() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        let looping = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({
                "durationTicks": 1_920,
                "loopEnabled": true,
                "loopStartTick": 0,
                "loopEndTick": 480,
            })),
        )
        .expect("解析")
        .expect("在场");
        assert_eq!(
            looping.loop_config,
            LoopConfig {
                enabled: true,
                start_tick: 0,
                end_tick: 480,
            }
        );

        // 启用但没给终点 ⇒ 缺省 = 片段内容长度（"整段循环"）。
        let extent = project.clip_pool[&clip_id]
            .content
            .notes()
            .expect("MIDI")
            .values()
            .map(|note| note.start_tick + note.duration_ticks)
            .max()
            .expect("至少一个音符");
        let defaulted = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({
                "durationTicks": 1_920,
                "loopEnabled": true,
            })),
        )
        .expect("解析")
        .expect("在场");
        assert_eq!(
            defaulted.loop_config,
            LoopConfig {
                enabled: true,
                start_tick: 0,
                end_tick: extent,
            }
        );

        // 给区间却不给 `loopEnabled: true` ⇒ 自相矛盾，响亮拒绝（不静默丢弃）。
        let fault = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"loopStartTick": 0, "loopEndTick": 480})),
        )
        .expect_err("区间缺开关必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(
            value["error"]["data"]["reason"], "placementFieldNotApplicable",
            "{value}"
        );

        // 启用但区间非正 ⇒ 模型层 `ZeroDuration`（不是本层另造一条规则）。
        let fault = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"loopEnabled": true, "loopEndTick": 0})),
        )
        .expect_err("非正区间必须被拒");
        // 模型层 `ZeroDuration` 走 `from_model` 映射到 `OUT_OF_RANGE`（不是本层另造码）。
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
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
            [
                "kind",
                "startTick",
                "durationTicks",
                "placementId",
                "muted",
                "loopEnabled",
                "loopStartTick",
                "loopEndTick",
            ]
        );
        assert_eq!(PLACEMENT_MOVE_FIELDS, ["kind", "startTick", "placementId"]);
        assert_eq!(PLACEMENT_REMOVE_FIELDS, ["kind", "placementId"]);
        // `add` 的**内容键**表没有被这次扩展改动（判据 `placement_field_names_are_pinned`
        // 仍逐字成立）。
        assert_eq!(
            PLACEMENT_FIELDS,
            [
                "startTick",
                "durationTicks",
                "placementId",
                "muted",
                "loopEnabled",
                "loopStartTick",
                "loopEndTick",
            ]
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
        // 真拼错 / 根本不支持的键（`mutedd` / `repeatCount`）仍报 `unknownPlacementField`。
        for unknown in ["mutedd", "repeatCount"] {
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
    /// `kind` 的**目录 ↔ 解析臂的双射**：`OP_KINDS` 里的每一个都必须被 `parse_one` 接受，
    /// 并**逐字回环**到同一个字面量（`kind_name()`）。
    ///
    /// 为什么必须有这一条：`OP_KINDS` 是一张**字面量表**，`parse_one` 的 `match` 是另一份
    /// 真相 —— `kind_name()` 的穷举 `match` 只保证"每个 `NoteOp` 变体都登记了一个名字"
    /// （漏一个变体 ⇒ E0004），它**不**保证"`OP_KINDS` 里的名字真的有解析臂"。
    /// 只把名字加进 `OP_KINDS` 而忘了加臂 ⇒ 错误报文会把它列进 `supportedKinds`，
    /// 而真调用它却报「未知 `kind`」。本判据把这条不对称变成红的。
    ///
    /// 注入（实测红）：从 `parse_one` 删掉 `REMOVE_SECTION_KIND` 臂 ⇒ 本判据红；
    /// 把 `OP_KINDS` 里的 `"setScene"` 改成 `"setScenes"` ⇒ 本判据红。
    #[test]
    fn every_catalog_kind_parses_and_round_trips() {
        let ulid = "01J8ZQ00000000000000000999";
        // 每个 kind 的**最小合法形状**（只到解析期为止；文档相关的前置条件在 `compile`）。
        let table: [(&str, serde_json::Value); 42] = [
            (
                "add",
                serde_json::json!({"note": {"startTick": 0, "pitch": 60, "durationTicks": 480}}),
            ),
            ("delete", serde_json::json!({"noteId": ulid})),
            (
                "move",
                serde_json::json!({"noteId": ulid, "deltaTick": 0, "deltaPitch": 0}),
            ),
            (
                "velocity",
                serde_json::json!({"noteId": ulid, "velocity": 64}),
            ),
            (
                "setNoteTiming",
                serde_json::json!({"noteId": ulid, "startTick": 0, "durationTicks": 480}),
            ),
            (
                "setNoteExpression",
                serde_json::json!({
                    "noteId": ulid,
                    "probability": 0.5,
                    "ratchet": 2,
                    "microTimingTicks": -12,
                }),
            ),
            (
                "setNotePitchShape",
                serde_json::json!({
                    "noteId": ulid,
                    "newPitchBendCurve": [[0, 0], [240, 200]],
                    "newSlide": null,
                }),
            ),
            ("removeClip", serde_json::json!({})),
            (
                "setAsset",
                serde_json::json!({"assetHash": "00".repeat(32), "license": "CC0-1.0"}),
            ),
            (
                "removeAsset",
                serde_json::json!({"assetHash": "00".repeat(32)}),
            ),
            ("removeTrack", serde_json::json!({"trackId": ulid})),
            (
                "insertDevice",
                serde_json::json!({
                    "trackId": ulid,
                    "device": {
                        "deviceId": ulid,
                        "name": "Probe",
                        "kind": "InternalEffect",
                        "bypassed": false,
                        "latencySamples": 0,
                        "params": [],
                    },
                }),
            ),
            (
                "removeDevice",
                serde_json::json!({"trackId": ulid, "deviceId": ulid}),
            ),
            (
                "setParam",
                serde_json::json!({"lane": "TrackVolume", "value": 0.0}),
            ),
            ("setTrackMute", serde_json::json!({"value": true})),
            ("setTrackSolo", serde_json::json!({"value": false})),
            (
                "setTransportConfig",
                serde_json::json!({
                    "metronomeEnabled": true,
                    "countInBars": 2,
                    "launchQuantization": "Bar",
                }),
            ),
            ("setTempo", serde_json::json!({"bpm": 140.0})),
            ("setAuthor", serde_json::json!({"author": "A"})),
            ("setTitle", serde_json::json!({"title": "T"})),
            (
                "setTrackName",
                serde_json::json!({"trackId": ulid, "name": "A"}),
            ),
            (
                "setClipName",
                serde_json::json!({"clipId": ulid, "name": "A"}),
            ),
            (
                "setClipGain",
                serde_json::json!({"clipId": ulid, "gainDb": -6.0}),
            ),
            (
                "setDeviceName",
                serde_json::json!({
                    "trackId": ulid,
                    "deviceId": ulid,
                    "name": "A",
                }),
            ),
            (
                "setDeviceBypassed",
                serde_json::json!({
                    "trackId": ulid,
                    "deviceId": ulid,
                    "bypassed": true,
                }),
            ),
            (
                "setClipPlacementMuted",
                serde_json::json!({
                    "trackId": ulid,
                    "placementId": ulid,
                    "muted": true,
                }),
            ),
            (
                "setClipPlacementLoop",
                serde_json::json!({
                    "trackId": ulid,
                    "placementId": ulid,
                    "loopEnabled": true,
                    "loopStartTick": 0,
                    "loopEndTick": 480,
                }),
            ),
            (
                "setTrackColor",
                serde_json::json!({"trackId": ulid, "color": "#22AA88"}),
            ),
            // ⭐ `#221`：采样器绑定形态（`null` 与字符串两种载荷都合法；这里取字符串）。
            (
                "setTrackSampler",
                serde_json::json!({"trackId": ulid, "sampler": "/x/inst.sfz"}),
            ),
            (
                "setClipPlacementDuration",
                serde_json::json!({
                    "trackId": ulid,
                    "placementId": ulid,
                    "durationTicks": 960,
                }),
            ),
            (
                "setAutomationLane",
                serde_json::json!({"lane": {"lane": "TrackVolume"}}),
            ),
            (
                "removeAutomationPoint",
                serde_json::json!({"point": {"lane": "TrackVolume", "tick": 0}}),
            ),
            (
                "setRoutingGain",
                serde_json::json!({"edgeId": ulid, "value": null}),
            ),
            ("disconnectRouting", serde_json::json!({"edgeId": ulid})),
            (
                "connectRouting",
                serde_json::json!({"edgeId": ulid, "sourceNode": ulid, "destinationNode": ulid, "edgeKind": "TrackToBus"}),
            ),
            ("removeRoutingNode", serde_json::json!({"nodeId": ulid})),
            ("addRoutingNode", serde_json::json!({"nodeId": ulid})),
            (
                "addTrack",
                serde_json::json!({"trackId": ulid, "name": "New Track", "trackKind": "Midi"}),
            ),
            ("removeSection", serde_json::json!({"sectionId": ulid})),
            (
                "setSection",
                serde_json::json!({"section": {"sectionId": ulid, "name": "Intro", "startTick": 0, "endTick": 960}}),
            ),
            ("removeScene", serde_json::json!({"sceneId": ulid})),
            ("setScene", serde_json::json!({"scene": {"sceneId": ulid}})),
        ];
        // The catalogue itself is pinned with **literals** (order included): most entries of
        // `OP_KINDS` are written as `*_KIND` constants, so asserting only "same length" or
        // "contains that constant" would stay true when the constant's *value* changes
        // (constant self-comparison is always true).
        assert_eq!(
            OP_KINDS,
            [
                "add",
                "delete",
                "move",
                "velocity",
                "setNoteTiming",
                "setNoteExpression",
                "setNotePitchShape",
                "removeClip",
                "removeAsset",
                "setAsset",
                "removeTrack",
                "insertDevice",
                "removeDevice",
                "setParam",
                "setTrackMute",
                "setTrackSolo",
                "setTransportConfig",
                "setTempo",
                "setAuthor",
                "setTitle",
                "setTrackName",
                "setClipName",
                "setClipGain",
                "setDeviceName",
                "setDeviceBypassed",
                "setClipPlacementMuted",
                "setClipPlacementLoop",
                "setTrackColor",
                "setTrackSampler",
                "setClipPlacementDuration",
                "setAutomationLane",
                "removeAutomationPoint",
                "setRoutingGain",
                "disconnectRouting",
                "connectRouting",
                "removeRoutingNode",
                "addRoutingNode",
                "addTrack",
                "removeSection",
                "setSection",
                "removeScene",
                "setScene",
            ],
            "`ops[].kind` catalog literals and their order"
        );
        assert_eq!(
            table.len(),
            OP_KINDS.len(),
            "the table must line up with the catalog"
        );
        for (kind, extra) in &table {
            assert!(OP_KINDS.contains(kind), "`{kind}` 必须在目录里");
            let mut object = extra.as_object().expect("对象").clone();
            object.insert("kind".to_owned(), Value::from(*kind));
            let ops = parse_ops(&serde_json::json!([Value::Object(object)]))
                .unwrap_or_else(|fault| panic!("`{kind}` 的规范形状必须能解析, 实际 {fault:?}"));
            assert_eq!(ops.len(), 1);
            assert_eq!(
                ops[0].kind_name(),
                *kind,
                "解析出来的 `NoteOp` 必须回环到同一个 `kind`"
            );
            // ⭐ `#177` 坑④：发布的"读顶层 `clipId` 的 kind 清单"必须与谓词**在全部
            // 36 个 kind 上**逐条一致（两份真相：本测试的 `table` 覆盖 `OP_KINDS`
            // 全集，谓词是 `compile` 真正用的那一份）。任一方向漂移都会在这里变红：
            // 清单里多一个 ⇒ `reads_envelope_clip` 为 `false`；谓词里多一个而清单
            // 没跟上 ⇒ 清单漏报（调用方会以为它不需要，而 `compile` 会读）。
            assert_eq!(
                ops[0].reads_envelope_clip(),
                ENVELOPE_CLIP_KINDS.contains(kind),
                "`{kind}` 的顶层 `clipId` 口径必须与发布的 ENVELOPE_CLIP_KINDS 一致"
            );
        }
        // 阴性对照：目录外的名字必须响亮拒绝，且报文里列出**整张**目录。
        let fault = parse_ops(&serde_json::json!([{"kind": "noSuchKind"}]))
            .expect_err("目录外的 kind 必须被拒");
        assert_eq!(
            lane_fault_data(&fault)["supportedKinds"],
            serde_json::json!(OP_KINDS),
            "拒绝报文必须列出与 `OP_KINDS` 同一份真相"
        );
    }

    /// `disconnectRouting` 的未知键**响亮失败**（`unknownDisconnectRoutingField`）。
    ///
    /// 注入（实测红）：去掉 [`reject_disconnect_routing_fields`] 的调用 ⇒
    /// 嵌套的 `trackId` / `value` 会被静默忽略，而调用方以为边已经断开。
    #[test]
    fn disconnect_routing_rejects_unknown_keys_loudly() {
        let ulid = "01J8ZQ00000000000000000999";
        for broken in [
            serde_json::json!([{"kind": DISCONNECT_ROUTING_KIND, "edgeId": ulid, "trackId": ulid}]),
            serde_json::json!([{"kind": DISCONNECT_ROUTING_KIND, "edgeId": ulid, "value": 0.0}]),
            serde_json::json!([{"kind": DISCONNECT_ROUTING_KIND, "edgeId": ulid, "gainDb": 1.0}]),
        ] {
            let fault = parse_ops(&broken).expect_err("未知键必须被拒");
            assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
            assert_eq!(
                lane_fault_data(&fault)["reason"],
                "unknownDisconnectRoutingField"
            );
            assert_eq!(
                lane_fault_data(&fault)["supportedDisconnectRoutingFields"],
                serde_json::json!(DISCONNECT_ROUTING_FIELDS)
            );
        }
        // 阴性对照：规范形状放行。
        assert!(
            parse_ops(&serde_json::json!([{"kind": DISCONNECT_ROUTING_KIND, "edgeId": ulid}]))
                .is_ok()
        );
    }

    /// `setAutomationLane` 的 `writeMode` 两种形状错误**各有各的 reason**。
    ///
    /// 注入（实测红）：把 `writeModeMustBeString` 那一段的 `as_str()` 改成 `as_str().unwrap_or("Off")`
    /// ⇒ 非字符串被静默当成 `Off`，第一条红；把 `unknownWriteMode` 那一段的 `other =>` 臂
    /// 改成落到 `Off` ⇒ 第二条红。
    #[test]
    fn lane_write_mode_shape_failures_use_distinct_reasons() {
        let ulid = "01J8ZQ00000000000000000999";
        let fault = parse_ops(&serde_json::json!([{
            "kind": SET_AUTOMATION_LANE_KIND,
            "lane": {"lane": "TrackVolume", "writeMode": 7},
        }]))
        .expect_err("数字写模式必须被拒");
        assert_eq!(lane_fault_data(&fault)["reason"], "writeModeMustBeString");
        assert_eq!(
            lane_fault_data(&fault)["allowed"],
            serde_json::json!(LANE_WRITE_MODES)
        );

        let fault = parse_ops(&serde_json::json!([{
            "kind": SET_AUTOMATION_LANE_KIND,
            "lane": {"lane": "TrackVolume", "writeMode": "Bogus"},
        }]))
        .expect_err("未知写模式必须被拒");
        assert_eq!(lane_fault_data(&fault)["reason"], "unknownWriteMode");
        assert_eq!(lane_fault_data(&fault)["received"], "Bogus");

        // 阴性对照：四个规范名逐个放行。
        for mode in LANE_WRITE_MODES {
            assert!(
                parse_ops(&serde_json::json!([{
                    "kind": SET_AUTOMATION_LANE_KIND,
                    "lane": {"lane": "TrackVolume", "writeMode": mode},
                }]))
                .is_ok(),
                "{mode} 必须放行"
            );
        }
        let _ = ulid;
    }

    /// `setAutomationLane` 的 `domain` 形状错误**响亮失败**（`domainMustBeObjectOrNull`）。
    #[test]
    fn lane_domain_shape_failure_is_a_parameter_error() {
        let fault = parse_ops(&serde_json::json!([{
            "kind": SET_AUTOMATION_LANE_KIND,
            "lane": {"lane": "TrackVolume", "domain": 5},
        }]))
        .expect_err("数字 domain 必须被拒");
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "domainMustBeObjectOrNull"
        );
        // 阴性对照：`null`（= 清掉覆盖）与规范对象都放行。
        assert!(
            parse_ops(&serde_json::json!([{
                "kind": SET_AUTOMATION_LANE_KIND,
                "lane": {"lane": "TrackVolume", "domain": null},
            }]))
            .is_ok()
        );
        assert!(
            parse_ops(&serde_json::json!([{
                "kind": SET_AUTOMATION_LANE_KIND,
                "lane": {"lane": "TrackVolume", "domain": {"min": -12.0, "max": 6.0}},
            }]))
            .is_ok()
        );
    }

    /// `SendGain` 目标缺 `edgeId` ⇒ **响亮失败**（`edgeIdRequired`）。
    ///
    /// 注入（实测红）：把那个 `ok_or_else` 换成 `unwrap_or(None)` ⇒ 缺 `edgeId` 的
    /// `SendGain` 目标会被当成"没有边"往下走，本判据红。
    #[test]
    fn send_gain_lane_without_an_edge_id_is_refused() {
        for raw in [
            serde_json::json!([{
                "kind": SET_AUTOMATION_LANE_KIND,
                "lane": {"lane": "SendGain"},
            }]),
            serde_json::json!([{
                "kind": REMOVE_AUTOMATION_POINT_KIND,
                "point": {"lane": "SendGain", "tick": 0},
            }]),
        ] {
            let fault = parse_ops(&raw).expect_err("SendGain 缺 edgeId 必须被拒");
            assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
            assert_eq!(lane_fault_data(&fault)["reason"], "edgeIdRequired");
            assert_eq!(lane_fault_data(&fault)["field"], "edgeId");
        }
    }

    /// `setScene` 的两处形状错误**各有各的 reason**：
    /// `create: true` 且没有 `name` ⇒ `sceneNameRequiredWhenCreating`；
    /// `tempo` 既不是数字也不是 `null` ⇒ `sceneTempoMustBeNumberOrNull`。
    ///
    /// 注入（实测红）：删掉 `create && patch.name.is_none()` 那条守卫 ⇒ 第一条红；
    /// 把 `tempo` 的 `as_f64()` 换成 `as_f64().unwrap_or(120.0)` ⇒ 第二条红。
    #[test]
    fn set_scene_name_and_tempo_shape_failures_use_distinct_reasons() {
        let ulid = "01J8ZQ00000000000000000999";
        let fault = parse_ops(&serde_json::json!([{
            "kind": SET_SCENE_KIND,
            "scene": {"sceneId": ulid, "create": true},
        }]))
        .expect_err("create 且没有 name 必须被拒");
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "sceneNameRequiredWhenCreating"
        );

        let fault = parse_ops(&serde_json::json!([{
            "kind": SET_SCENE_KIND,
            "scene": {"sceneId": ulid, "tempo": "fast"},
        }]))
        .expect_err("字符串 tempo 必须被拒");
        assert_eq!(
            lane_fault_data(&fault)["reason"],
            "sceneTempoMustBeNumberOrNull"
        );
        assert_eq!(lane_fault_data(&fault)["received"], "fast");

        // 阴性对照：`create: true` + `name` 放行；`tempo: null` 放行。
        assert!(
            parse_ops(&serde_json::json!([{
                "kind": SET_SCENE_KIND,
                "scene": {"sceneId": ulid, "create": true, "name": "Chorus"},
            }]))
            .is_ok()
        );
        assert!(
            parse_ops(&serde_json::json!([{
                "kind": SET_SCENE_KIND,
                "scene": {"sceneId": ulid, "tempo": null},
            }]))
            .is_ok()
        );
    }

    /// **程序化构造**的设备定义（参数非有限）在 `compile` 里被拒
    /// （`invalidDeviceDefinition`）。
    ///
    /// 这一道是**纵深防御**：工具面走 [`parse_device`]，非有限参数在**解析期**就被拒
    /// （判据 `device_payload_shapes_fail_loudly` 钉住），因此从 JSON 到不了这里。
    /// 但 [`compile`] 是公开函数、`NoteOp::InsertDevice` 可以直接构造 ⇒ 这道闸门
    /// 仍然必须真的拒（否则一条程序化构造的批会把 `NaN` 写进文档，而
    /// `YebanProjectV1::validate` 要到提案模拟那一步才报）。
    #[test]
    fn a_programmatically_built_device_with_a_non_finite_param_is_refused() {
        let project = filled_project();
        let track = device_track(&project);
        let (_, clip_id) = lead_clip(&project);
        let mut device = project.tracks[&track].devices[0].clone();
        device.id = EntityId::from_str("01J8ZQ00000000000000000DEV").expect("ULID");
        device.params.push(ParameterValue {
            name: "broken".to_owned(),
            value: f32::NAN,
            unit: None,
        });
        let ops = vec![NoteOp::InsertDevice {
            track_id: track,
            slot_index: None,
            device,
        }];
        let fault = compile(&project, &track, &clip_id, &ops).expect_err("NaN 参数必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        assert_eq!(lane_fault_data(&fault)["reason"], "invalidDeviceDefinition");
    }
    /// **方向 3：`kind`（＋载荷分支）→ 恰好一个 `Op` 变体**（第三口径的另一半）。
    ///
    /// 上一批证明了"31 个 `Op` 变体都有构造点"，也证明了"`OP_KINDS` 与 `parse_one` 的臂
    /// 逐字双射"，但**没有**证明"每个 `kind` 落到哪个 `Op` 变体"。本判据把这条补齐：
    /// 对 19 个 `kind`（其中 `setAutomationLane` 有**两个**载荷分支，因此共 20 行）
    /// 真的 `parse_ops` + `compile`，并断言 `Op::name()` **逐行等于登记值**。
    ///
    /// ⚠ 精确措辞：`kind → Op` 不是"一一对应" —— `setAutomationLane` 的 `remove` 分支产出
    /// **另一个**变体（`SetAutomationLane` vs `RemoveAutomationLane`）。因此登记的单位是
    /// **(kind, 载荷分支)**；20 行 → **20 个互不相同的变体**（末条断言钉住单射）。
    ///
    /// 注入（实测红）：把 `parse_one` 的某个臂换成产出另一个 `NoteOp` ⇒ 该行期望变体不符，红；
    /// 把两行登记成同一个变体 ⇒ 末条单射断言红。
    #[test]
    fn every_catalog_kind_compiles_to_exactly_one_op_variant() {
        let project = filled_project();
        let (track, clip) = lead_clip(&project);
        let sample_note = project.clip_pool[&clip]
            .content
            .notes()
            .expect("MIDI 片段")
            .values()
            .next()
            .expect("样本音符")
            .clone();
        let note = sample_note.id;
        let device_track_id = device_track(&project);
        let device = project.tracks[&device_track_id].devices[0].id;
        let (node, _referencing) = a_non_master_node(&project);
        let edge = *project
            .routing_graph
            .edges
            .keys()
            .next()
            .expect("样本路由边");
        let scene = *project.scenes.keys().next().expect("样本场景");
        let section = *project.sections.keys().next().expect("样本段落");
        let target_track = project
            .tracks
            .values()
            .find(|track| track.id != project.master_bus_track_id)
            .expect("非主总线音轨")
            .id;
        let (lane_name, lane_tick) = project
            .tracks
            .values()
            .flat_map(|track| track.automation_lanes.values())
            .find_map(|lane| {
                let name = match lane.target {
                    AutomationTarget::TrackVolume { .. } => "TrackVolume",
                    AutomationTarget::TrackPan { .. } => "TrackPan",
                    _ => return None,
                };
                let tick = lane.points.values().next()?.tick;
                let unique = lane
                    .points
                    .values()
                    .filter(|point| point.tick == tick)
                    .count()
                    == 1;
                unique.then_some((name, tick))
            })
            .expect("样本里必须有一条 TrackVolume/TrackPan 泳道且带唯一 tick 的采样点");
        // `removeClip` 的目标必须**没有任何摆放引用**，而样本里每条池条目都被引用 ⇒ 在克隆体上
        // 加一条新的池条目（内容抄自样本第一条），只给这一行用。
        let spare_entry = {
            let mut entry = project.clip_pool.values().next().expect("池条目").clone();
            entry.id = EntityId::from_str("01J8ZQ00000000000000000CP1").expect("ULID");
            entry
        };
        let unplaced = spare_entry.id;
        let ghost = EntityId::from_str("01J8ZQ00000000000000000999").expect("ULID");
        let new_scene = EntityId::from_str("01J8ZQ00000000000000000SC1").expect("ULID");
        // ⭐ `removeAsset` 需要一条**真的在** `project.assets` 里的哈希（否则它会响亮
        // 拒绝 `assetNotFound`），而它的前置还要求**没有片段引用**它 ⇒ 用夹具的音频片段
        // 所引用的那个哈希作目标，并在那一行**预置**该片段。
        let audio_entry = project
            .clip_pool
            .values()
            .find(|entry| matches!(&entry.content, ClipContent::Audio { .. }))
            .expect("夹具必须有音频片段")
            .clone();
        // ⚠ 目标**必须无引用**：`removeAsset` 的前置要求"没有任何 `clip_pool` 条目
        // 引用它" ⇒ 夹具里那条音频片段引用的哈希**不能**当目标（那会红 `assetInUse`，
        // 而那条拒绝路径由它自己的判据覆盖）。这里登记一条**没人引用**的合成资产。
        let orphan_asset = AssetHash::of_bytes(b"orphan-asset-for-removeAsset-row");
        let orphan_entry = yeban_model::AssetMetadata {
            hash: orphan_asset.clone(),
            original_path: "samples/orphan.wav".to_owned(),
            byte_len: 64,
            media_kind: yeban_model::MediaKind::Audio,
            license: "CC0-1.0".to_owned(),
        };

        // (标签, 顶层 clip 覆盖, 本行预置的池条目, 载荷, 期望的 `Op` 变体名)
        #[allow(clippy::type_complexity)]
        // ⭐ 摆放静音那一行需要**真的存在**的摆放（`compile` 按 trackId + placementId 查，
        // 缺失会响亮报 `clipPlacementNotFound`）⇒ 找一个**有摆放的**音轨与它的第一个摆放。
        let (placement_track, placed) = project
            .tracks
            .iter()
            .find(|(_, track)| !track.clips.is_empty())
            .map(|(id, track)| {
                (
                    *id,
                    *track.clips.keys().next().expect("非空轨必有第一个摆放"),
                )
            })
            .expect("样本里必须有带摆放的音轨");
        // ⚠ 类型别名：⭐ `clippy::type_complexity` 在这个宽度上会红（⭐ 5 元组 × 定长数组），
        // ⭐ 按 clippy 的建议**提取别名**（⛔ 不用 `#[allow]` 掩盖 —— 那只是把复杂度留下）。
        type Row<'a> = (
            &'a str,
            Option<EntityId>,
            Option<ClipPoolEntry>,
            Value,
            &'a str,
        );
        let rows: [Row<'_>; 43] = [
            (
                "add",
                None,
                None,
                serde_json::json!({"note": {"startTick": 0, "pitch": 60, "durationTicks": 480}}),
                "AddNote",
            ),
            (
                "delete",
                None,
                None,
                serde_json::json!({"noteId": note}),
                "DeleteNote",
            ),
            (
                "move",
                None,
                None,
                serde_json::json!({"noteId": note, "deltaTick": 0, "deltaPitch": 0}),
                "MoveNote",
            ),
            (
                "velocity",
                None,
                None,
                serde_json::json!({"noteId": note, "velocity": 64}),
                "ModifyNoteVelocity",
            ),
            (
                "setNoteTiming",
                None,
                None,
                serde_json::json!({
                    "noteId": note,
                    "startTick": sample_note.start_tick + 960,
                    "durationTicks": sample_note.duration_ticks + 240,
                }),
                "SetNoteTiming",
            ),
            (
                "setNoteExpression",
                None,
                None,
                serde_json::json!({
                    "noteId": note,
                    "probability": 0.5,
                    "ratchet": 2,
                    "microTimingTicks": -12,
                }),
                "SetNoteExpression",
            ),
            (
                "setNotePitchShape",
                None,
                None,
                serde_json::json!({
                    "noteId": note,
                    "newPitchBendCurve": [[0, 0], [240, 200]],
                    "newSlide": {"durationTicks": 240, "targetPitch": 72},
                }),
                "SetNotePitchShape",
            ),
            (
                "setAsset",
                None,
                Some({
                    let mut entry = audio_entry.clone();
                    entry.id = EntityId::from_str("01J8ZQ00000000000000000AS2").expect("ULID");
                    entry
                }),
                serde_json::json!({"assetHash": orphan_asset.as_str(), "license": "CC-BY-4.0"}),
                "SetAsset",
            ),
            (
                "removeAsset",
                None,
                Some({
                    let mut entry = audio_entry.clone();
                    entry.id = EntityId::from_str("01J8ZQ00000000000000000AS1").expect("ULID");
                    entry
                }),
                serde_json::json!({"assetHash": orphan_asset.as_str()}),
                "RemoveAsset",
            ),
            (
                "removeClip",
                Some(unplaced),
                Some(spare_entry),
                serde_json::json!({}),
                "RemoveClip",
            ),
            (
                "removeTrack",
                None,
                None,
                serde_json::json!({"trackId": target_track}),
                "RemoveTrack",
            ),
            (
                "insertDevice",
                None,
                None,
                serde_json::json!({"trackId": device_track_id,
                "device": {"deviceId": ghost, "name": "Probe", "kind": "InternalEffect",
                           "bypassed": false, "latencySamples": 0, "params": []}}),
                "InsertDevice",
            ),
            (
                "removeDevice",
                None,
                None,
                serde_json::json!({"trackId": device_track_id, "deviceId": device}),
                "RemoveDevice",
            ),
            (
                "setParam",
                None,
                None,
                serde_json::json!({"lane": "TrackVolume", "value": -3.0}),
                "SetParam",
            ),
            (
                "setTrackMute",
                None,
                None,
                serde_json::json!({"value": true}),
                "SetTrackMute",
            ),
            (
                "setTrackSolo",
                None,
                None,
                serde_json::json!({"value": false}),
                "SetTrackSolo",
            ),
            (
                "setTransportConfig",
                None,
                None,
                serde_json::json!({
                    "metronomeEnabled": true,
                    "countInBars": 2,
                    "launchQuantization": "Bar",
                }),
                "SetTransportConfig",
            ),
            (
                "setTempo",
                None,
                None,
                serde_json::json!({"bpm": 140.0}),
                "SetTempo",
            ),
            (
                "setAuthor",
                None,
                None,
                serde_json::json!({"author": "New Author"}),
                "SetAuthor",
            ),
            (
                "setTitle",
                None,
                None,
                serde_json::json!({"title": "夜半 标题"}),
                "SetTitle",
            ),
            (
                "setTrackName",
                None,
                None,
                serde_json::json!({"trackId": device_track_id, "name": "Renamed"}),
                "SetTrackName",
            ),
            (
                "setClipName",
                None,
                None,
                serde_json::json!({"clipId": clip.to_canonical_string(), "name": "Renamed"}),
                "SetClipName",
            ),
            (
                "setClipGain",
                None,
                // ⚠ 目标必须是一条**音频**条目：`clipId` 自带寻址（⛔ 不读顶层 `clipId`），
                // 而 `compile` 要**从文档**读 `old_gain_db` ⇒ 预置这条真实音频条目。
                Some(audio_entry.clone()),
                serde_json::json!({
                    "clipId": audio_entry.id.to_canonical_string(),
                    "gainDb": -6.0,
                }),
                "SetClipGain",
            ),
            (
                "setDeviceName",
                None,
                None,
                serde_json::json!({
                    // ⚠ 用**真实**设备链上的 id（`device` ＝ 某个带设备音轨的第 0 台），
                    // ⛔ 不是 `device_track_id` —— compile 按 id 查设备、缺失即 `deviceNotFound`。
                    "trackId": device_track_id.to_canonical_string(),
                    "deviceId": device.to_canonical_string(),
                    "name": "Renamed",
                }),
                "SetDeviceName",
            ),
            (
                "setDeviceBypassed",
                None,
                None,
                serde_json::json!({
                    "trackId": device_track_id.to_canonical_string(),
                    "deviceId": device.to_canonical_string(),
                    "bypassed": true,
                }),
                "SetDeviceBypassed",
            ),
            (
                "setClipPlacementMuted",
                None,
                None,
                serde_json::json!({
                    "trackId": placement_track.to_canonical_string(),
                    "placementId": placed.to_canonical_string(),
                    "muted": true,
                }),
                "SetClipPlacementMuted",
            ),
            (
                "setClipPlacementLoop",
                None,
                None,
                serde_json::json!({
                    "trackId": placement_track.to_canonical_string(),
                    "placementId": placed.to_canonical_string(),
                    "loopEnabled": true,
                    "loopStartTick": 0,
                    "loopEndTick": 480,
                }),
                "SetClipPlacementLoop",
            ),
            (
                "setTrackColor",
                None,
                None,
                serde_json::json!({"trackId": track.to_canonical_string(), "color": "#22AA88"}),
                "SetTrackColor",
            ),
            (
                "setClipPlacementDuration",
                None,
                None,
                serde_json::json!({
                    "trackId": placement_track.to_canonical_string(),
                    "placementId": placed.to_canonical_string(),
                    "durationTicks": 960,
                }),
                "SetClipPlacementDuration",
            ),
            (
                "setAutomationLane#patch",
                None,
                None,
                serde_json::json!({"lane": {"lane": lane_name, "readEnabled": true}}),
                "SetAutomationLane",
            ),
            (
                "setAutomationLane#remove",
                None,
                None,
                serde_json::json!({"lane": {"lane": lane_name, "remove": true}}),
                "RemoveAutomationLane",
            ),
            (
                "removeAutomationPoint",
                None,
                None,
                serde_json::json!({"point": {"lane": lane_name, "tick": lane_tick}}),
                "RemoveAutomationPoint",
            ),
            (
                "setRoutingGain",
                None,
                None,
                serde_json::json!({"edgeId": edge, "value": -3.0}),
                "SetRoutingGain",
            ),
            (
                "connectRouting",
                None,
                None,
                serde_json::json!({"edgeId": ghost, "sourceNode": node, "destinationNode": node, "edgeKind": "TrackToBus"}),
                "ConnectRouting",
            ),
            (
                "disconnectRouting",
                None,
                None,
                serde_json::json!({"edgeId": edge}),
                "DisconnectRouting",
            ),
            (
                "addTrack",
                None,
                None,
                serde_json::json!({"trackId": ghost, "name": "New Track", "trackKind": "Midi"}),
                "AddTrack",
            ),
            (
                "addRoutingNode",
                None,
                None,
                // ⚠ 必须用一个**不在** `routing_graph.nodes` 里的身份：`node` 那一行
                // 已经被 `removeRoutingNode` 用掉了（它是**已有**节点）。
                serde_json::json!({"nodeId": ghost}),
                "AddRoutingNode",
            ),
            (
                "removeRoutingNode",
                None,
                None,
                serde_json::json!({"nodeId": node}),
                "RemoveRoutingNode",
            ),
            (
                "setSection",
                None,
                None,
                serde_json::json!({"section": {"sectionId": section, "name": "Intro", "startTick": 0, "endTick": 960}}),
                "SetSection",
            ),
            (
                "removeSection",
                None,
                None,
                serde_json::json!({"sectionId": section}),
                "RemoveSection",
            ),
            (
                "removeScene",
                None,
                None,
                serde_json::json!({"sceneId": scene}),
                "RemoveScene",
            ),
            (
                "setScene",
                None,
                None,
                serde_json::json!({"scene": {"sceneId": new_scene, "create": true,
                                          "name": "Probe"}}),
                "SetScene",
            ),
            // ⭐ `#221`：`setTrackSampler` ⇒ `Op::SetTrackSampler`（这条行同时钉住
            // "工具面真的有采样器绑定通路"，而 `#220` 缺陷 ① 正是"没有"）。
            (
                "setTrackSampler",
                None,
                None,
                serde_json::json!({"trackId": track.to_canonical_string(),
                                   "sampler": "/samples/dsmolken-double-bass.sfz"}),
                "SetTrackSampler",
            ),
        ];
        assert_eq!(
            rows.len(),
            OP_KINDS.len() + 1,
            "42 个 kind ＋ 1 个额外载荷分支"
        );

        let mut seen: Vec<&str> = Vec::new();
        // ⭐ `#179`：在**全部 42 个 kind**上对账 `is_note_level()` 与发布的
        // `NOTE_LEVEL_KINDS`（⛔ 不是手抄的第二份清单，也不是"断言它非空"）。
        let mut note_level_seen: Vec<&str> = Vec::new();
        for (label, clip_override, preset, payload, expected) in rows {
            let mut object = payload.as_object().expect("对象").clone();
            let kind = label.split('#').next().expect("标签有 kind 前缀");
            object.insert("kind".to_owned(), Value::from(kind));
            let ops = parse_ops(&serde_json::json!([Value::Object(object)]))
                .unwrap_or_else(|fault| panic!("`{label}` 的规范形状必须能解析: {fault:?}"));
            assert_eq!(
                ops[0].is_note_level(),
                NOTE_LEVEL_KINDS.contains(&kind),
                "`{kind}` 的音符级口径必须与发布的 NOTE_LEVEL_KINDS 一致"
            );
            if ops[0].is_note_level() && !note_level_seen.contains(&kind) {
                note_level_seen.push(kind);
            }
            let use_clip = clip_override.unwrap_or(clip);
            // ⭐ 每一行都注入那条**没人引用**的合成资产（`removeAsset` 的目标）；
            // `preset` 再额外插一条片段池条目（有的行需要）。
            let mut document = project.clone();
            document
                .assets
                .insert(orphan_asset.clone(), orphan_entry.clone());
            if let Some(entry) = preset {
                document.clip_pool.insert(entry.id, entry);
            }
            let compiled = compile(&document, &track, &use_clip, &ops)
                .unwrap_or_else(|fault| panic!("`{label}` 必须能编译: {fault:?}"));
            assert_eq!(compiled.len(), 1, "`{label}` 应恰好产出一条 `Op`");
            assert_eq!(
                compiled[0].name(),
                expected,
                "`{label}` 必须落到登记的 `Op` 变体"
            );
            assert!(
                !seen.contains(&expected),
                "`{label}` 的变体 `{expected}` 与别的行重复 —— (kind, 分支) → 变体必须是单射"
            );
            seen.push(expected);
        }
        // ⚠ 本数是**去重后的变体数**（`add` 的三种形态共用 `AddNote`）⇒ 与
        // `rows.len()` 可以不同，也与 `OP_KINDS.len()` 不同。本轮新增
        // `setNoteTiming`（独占一个变体）⇒ 37 → **38**；`#327` 的 `setTitle`（独占一个
        // 变体）⇒ 38 → **39**；`#328` 的 `setClipGain`（独占一个变体）⇒ 39 → **40**；
        // `#329` 的 `setNoteExpression`（独占一个变体）⇒ 40 → **41**；
        // `#334` 的 `setNotePitchShape`（独占一个变体）⇒ 41 → **42**；
        // `#336` 的 `setClipPlacementLoop`（独占一个变体）⇒ 42 → **43**。
        assert_eq!(seen.len(), 43);
        // ⭐ `#179`：音符级分支集合**恰好是七个**，且逐字等于发布清单。
        assert_eq!(
            NOTE_LEVEL_KINDS,
            [
                "add",
                "delete",
                "move",
                "velocity",
                "setNoteTiming",
                "setNoteExpression",
                "setNotePitchShape"
            ],
            "音符级 kind 必须恰好是七个音符形态（字面表，⛔ 不是常量自比）"
        );
        assert_eq!(
            note_level_seen.len(),
            7,
            "全部 42 个 kind 里 `is_note_level()` 为真的必须**恰好七个**，实际: {note_level_seen:?}"
        );
        assert_eq!(
            note_level_seen,
            NOTE_LEVEL_KINDS.to_vec(),
            "逐个 kind 收集到的音符级集合必须逐项等于 NOTE_LEVEL_KINDS"
        );
    }

    /// `setTransportConfig`：解析三个字段、拒绝未知键/缺字段、并把 `old_config`
    /// 从**当前文档**读出来（⛔ 不是调用方声明）。
    #[test]
    fn set_transport_config_parses_and_reads_the_old_config_from_the_document() {
        // ① 规范形状能解析，且回环到同一个 kind。
        let ops = parse_ops(&serde_json::json!([{
            "kind": "setTransportConfig",
            "metronomeEnabled": true,
            "countInBars": 2,
            "launchQuantization": "TwoBars",
        }]))
        .expect("规范形状必须被接受");
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].kind_name(), SET_TRANSPORT_CONFIG_KIND);

        // ② 未知键响亮失败（⛔ 不静默丢弃）。
        let fault = parse_ops(&serde_json::json!([{
            "kind": "setTransportConfig",
            "metronomeEnabled": true,
            "countInBars": 2,
            "launchQuantization": "Bar",
            "tempo": 128,
        }]))
        .expect_err("未知键必须被拒");
        assert_eq!(
            reason_of(&fault).as_deref(),
            Some("unknownTransportConfigField")
        );

        // ③ 三个字段**一个都不能省**（ADR-0001 D43：缺一个就是伪造作者的选择）。
        for missing in [
            TRANSPORT_METRONOME_FIELD,
            TRANSPORT_COUNT_IN_FIELD,
            TRANSPORT_LAUNCH_FIELD,
        ] {
            let mut object = serde_json::json!({
                "kind": "setTransportConfig",
                "metronomeEnabled": true,
                "countInBars": 2,
                "launchQuantization": "Bar",
            });
            object.as_object_mut().expect("对象").remove(missing);
            assert!(
                parse_ops(&serde_json::json!([object])).is_err(),
                "缺 `{missing}` 必须响亮失败"
            );
        }

        // ④ `compile` 的 `old_config` 来自**当前工程**（`filled_project` 的走带配置）。
        let project = filled_project();
        let (track, clip) = lead_clip(&project);
        let compiled = compile(&project, &track, &clip, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::SetTransportConfig {
                old_config,
                new_config,
            } => {
                assert_eq!(*old_config, project.transport, "old 必须等于文档现值");
                assert!(new_config.metronome_enabled);
                assert_eq!(new_config.count_in_bars, 2);
                assert_eq!(new_config.launch_quantization, LaunchQuantization::TwoBars);
            }
            other => panic!("必须编译成 SetTransportConfig, 实际 {other:?}"),
        }

        // ⑤ 反向：非规范量化名响亮失败。
        assert!(
            parse_ops(&serde_json::json!([{
                "kind": "setTransportConfig",
                "metronomeEnabled": true,
                "countInBars": 2,
                "launchQuantization": "bar",
            }]))
            .is_err(),
            "❌ 拼错的大小写不许被静默接受"
        );
    }

    /// `addRoutingNode`：解析 → 编译到**正是** `Op::AddRoutingNode` → 应用 → 逆操作
    /// 逐字节回退；重复身份与未知键各自响亮失败。
    ///
    /// 注入（实测红）：① 把 [`ADD_ROUTING_NODE_KIND`] 指到 `Op::RemoveRoutingNode` ⇒
    /// 节点数不减反增（`+1` 那条断言红）、逆操作也回不到原状；② 把
    /// [`ROUTING_NODE_FIELD`] 由 `nodeId` 改成 `edgeId` ⇒ 解析那一步就红；
    /// ③ 去掉 `routingNodeAlreadyExists` 那条提前拒绝 ⇒ "重复身份"那次调用会走到
    /// 提案模拟才报错（`expect_err` 仍然成立、但 `reason` 断言红）。
    #[test]
    fn add_routing_node_adds_a_track_identity_and_inverts_byte_for_byte() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        // ⚠ 夹具前提：要用一个**还没在** `nodes` 里的身份 —— 样本的图很完整（每条音轨
        // 都已在 `nodes` 里），所以这里取一个**新建音轨**那种"还没进图"的身份。
        // 这正是本形态的**真实用途**：`addTrack` 只把音轨加进 `tracks`，⛔ **不**自动
        // 把它的身份加进 `nodes`（两份集合各有自己的 `Op`）⇒ 没有本形态，这样一条
        // 音轨**永远**不会成为路由节点。
        let node = EntityId::from_str("01J8ZQ0000000000000000FR5H").expect("ULID");
        assert!(
            !project.routing_graph.nodes.contains(&node),
            "夹具前提：这个身份不在 routing_graph.nodes 里"
        );
        let nodes_before = project.routing_graph.nodes.clone();
        let bytes_before = serde_json::to_string(&project).expect("序列化");

        // ---- ① 规范形状能解析，且分类正确（路由级、不读不写音符） ----
        let ops = parse_ops(&serde_json::json!([
            {"kind": ADD_ROUTING_NODE_KIND, "nodeId": node.to_canonical_string()}
        ]))
        .expect("规范形状必须被接受");
        assert_eq!(
            ops[0].kind_name(),
            ADD_ROUTING_NODE_KIND,
            "kind 必须回环一致"
        );
        assert!(!ops[0].is_note_level(), "路由级不读不写音符");
        assert!(ops[0].is_routing_level(), "必须落在路由级那个桶里");
        assert_eq!(ops[0], NoteOp::AddRoutingNode { node_id: node });

        // ---- ② 编译到**正是** `Op::AddRoutingNode` ----
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::AddRoutingNode { node: target } => assert_eq!(*target, node),
            other => panic!("应当是 AddRoutingNode: {other:?}"),
        }

        // ---- ③ 应用：节点真的进图，且工程仍然合法（那条不变式此前不可达） ----
        compiled[0].apply(&mut project).expect("新建");
        assert!(
            project.routing_graph.nodes.contains(&node),
            "节点必须真的进 `routing_graph.nodes`"
        );
        assert_eq!(project.routing_graph.nodes.len(), nodes_before.len() + 1);
        assert!(
            project.validate().is_ok(),
            "新建节点之后工程必须仍然合法 —— 图校验要求每条边的两端都在 nodes 里, \
             而那个不变式在工具面可达本形态之前**无法被建立**"
        );

        // ---- ④ 逆操作逐字节回退 ----
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(project.routing_graph.nodes, nodes_before, "节点必须被摘掉");
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "逆操作必须逐字节回到原状"
        );

        // ---- ⑤ 重复身份：响亮拒绝（模型那条前置条件是"身份不在 nodes 里"） ----
        let occupied = filled_project();
        let existing = occupied.routing_graph.nodes[0];
        let duplicate = parse_ops(&serde_json::json!([
            {"kind": ADD_ROUTING_NODE_KIND, "nodeId": existing.to_canonical_string()}
        ]))
        .expect("解析不查文档");
        let fault = compile(&occupied, &track_id, &clip_id, &duplicate)
            .expect_err("已经在 nodes 里的身份必须被拒");
        assert_eq!(
            reason_of(&fault).as_deref(),
            Some("routingNodeAlreadyExists"),
            "必须是**幂等拒绝**并报清楚理由"
        );

        // ---- ⑥ 未知键：响亮失败，⛔ 不静默丢弃 ----
        let bad = parse_ops(&serde_json::json!([
            {"kind": ADD_ROUTING_NODE_KIND, "nodeId": node.to_canonical_string(), "edgeId": node.to_canonical_string()}
        ]))
        .expect_err("嵌套的 edgeId 必须被拒不");
        assert_eq!(
            reason_of(&bad).as_deref(),
            Some("unknownAddRoutingNodeField"),
            "`nodeId` 之外的键必须点名"
        );
    }

    /// `addRoutingNode` 的**字面名与支持集合**被钉住。
    #[test]
    fn add_routing_node_field_names_are_pinned() {
        assert_eq!(ADD_ROUTING_NODE_KIND, "addRoutingNode");
        assert_eq!(ADD_ROUTING_NODE_FIELDS, ["kind", ROUTING_NODE_FIELD]);
        assert_eq!(ROUTING_NODE_FIELD, "nodeId");
        // 两个节点形态**互为逆操作** ⇒ 共用同一个寻址字段（不是两个不同的字面量）。
        assert_eq!(ADD_ROUTING_NODE_FIELDS, REMOVE_ROUTING_NODE_FIELDS);
        assert_ne!(
            ADD_ROUTING_NODE_KIND, REMOVE_ROUTING_NODE_KIND,
            "两个节点形态做的是**相反**的事, 共用一个 kind 会让调用方分不开它们"
        );
    }

    /// `addTrack`：解析 → 编译到**正是** `Op::AddTrack` → 撤销一步恢复原状；
    /// 并钉住"空形状是**决定**"与"新轨不改混音"。
    ///
    /// 注入（实测红）：① 编译成 `RemoveTrack` ⇒ 音轨数不减反增那条断言红；
    /// ② 把空形状换个非默认的 `volume_db`/`mute` ⇒ 第 ② 条判据红；
    /// ③ 去掉 `trackAlreadyExists` 那条提前拒绝 ⇒ 重复身份那次调用的 `reason` 断言红。
    #[test]
    fn add_track_lands_an_empty_track_and_leaves_the_mix_alone() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let new_id = EntityId::from_str("01J8ZQ000000000000000000TR").expect("ULID");
        assert!(
            !project.tracks.contains_key(&new_id),
            "夹具前提: 身份是新的"
        );
        let tracks_before = project.tracks.len();
        let bytes_before = serde_json::to_string(&project).expect("序列化");

        // ---- ① 规范形状能解析，kind 回环一致，分类正确 ----
        let ops = parse_ops(&serde_json::json!([
            {"kind": ADD_TRACK_KIND, "trackId": new_id.to_canonical_string(),
             "name": "New Track", "trackKind": "Audio"}
        ]))
        .expect("规范形状必须被接受");
        assert_eq!(ops[0].kind_name(), ADD_TRACK_KIND, "kind 必须回环一致");
        assert!(!ops[0].is_note_level(), "音轨实体级不读不写音符");
        assert!(!ops[0].is_routing_level(), "音轨**不是**路由图的一部分");
        match &ops[0] {
            NoteOp::AddTrack { track } => {
                assert_eq!(track.id, new_id);
                assert_eq!(track.name, "New Track");
                assert_eq!(track.kind, TrackKind::Audio, "`trackKind` 必须逐字映射");
            }
            other => panic!("应当是 AddTrack: {other:?}"),
        }

        // ---- ②⭐ 空形状是**决定**：四个集合为空 + 两个可选字段为 None ----
        let parsed_track = match &ops[0] {
            NoteOp::AddTrack { track } => track.clone(),
            other => panic!("{other:?}"),
        };
        assert!(parsed_track.devices.is_empty(), "devices 必须是空形状");
        assert!(parsed_track.macros.is_empty(), "macros 必须是空形状");
        assert!(
            parsed_track.automation_lanes.is_empty(),
            "automation_lanes 必须是空形状"
        );
        assert!(parsed_track.clips.is_empty(), "clips 必须是空形状");
        assert_eq!(parsed_track.folder_id, None, "folderId 不在载荷里 ⇒ None");
        assert_eq!(parsed_track.color, None, "color 不在载荷里 ⇒ None");

        // ---- ②⭐ 新建空轨**不改混音**：五个标量必须等于 `TrackV3::default()` ----
        let defaults = TrackV3::default();
        assert_eq!(
            parsed_track.volume_db, defaults.volume_db,
            "新建空轨的音量必须是默认值 (⛔ 不许静默改混音)"
        );
        assert_eq!(parsed_track.pan, defaults.pan, "声相必须是默认值");
        assert!(!parsed_track.mute, "新轨不许默认静音");
        assert!(!parsed_track.solo, "新轨不许默认独奏");
        assert_eq!(
            parsed_track.solo_safe, defaults.solo_safe,
            "solo_safe 必须是默认值"
        );

        // ---- ③ 编译到**正是** `Op::AddTrack` ----
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::AddTrack { track } => assert_eq!(track, &parsed_track),
            other => panic!("应当是 AddTrack: {other:?}"),
        }

        // ---- ④ 应用：音轨真的进 `tracks`，且工程仍然合法 ----
        compiled[0].apply(&mut project).expect("新建");
        assert!(
            project.tracks.contains_key(&new_id),
            "音轨必须真的进 tracks"
        );
        assert_eq!(project.tracks.len(), tracks_before + 1);
        assert!(project.validate().is_ok(), "新建之后工程必须仍然合法");
        // ⚠ 模型 `apply` **不**自动把身份加进 `nodes`（那是 addRoutingNode 的事）。
        assert!(
            !project.routing_graph.nodes.contains(&new_id),
            "`addTrack` ⛔ 不自动把身份加进 routing_graph.nodes"
        );

        // ---- ⑤ **撤销一步**必须恢复原状（逐字节） ----
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(project.tracks.len(), tracks_before, "音轨数必须回到原值");
        assert!(
            !project.tracks.contains_key(&new_id),
            "逆操作必须把这条音轨摘掉"
        );
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "一步撤销必须逐字节回到原状"
        );
    }

    /// 重复身份与未知键各自**响亮失败**（⛔ 不静默丢弃）。
    #[test]
    fn add_track_refuses_a_duplicate_identity_and_an_unknown_key() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let existing = *project.tracks.keys().next().expect("样本音轨");

        // ---- 重复身份 ⇒ CONFLICT / `trackAlreadyExists` ----
        let duplicate = parse_ops(&serde_json::json!([
            {"kind": ADD_TRACK_KIND, "trackId": existing.to_canonical_string(),
             "name": "Dup", "trackKind": "Midi"}
        ]))
        .expect("解析不查文档");
        let fault = compile(&project, &track_id, &clip_id, &duplicate)
            .expect_err("已在 tracks 里的身份必须被拒");
        assert_eq!(reason_of(&fault).as_deref(), Some("trackAlreadyExists"));

        // ---- 未知键 ⇒ `unknownAddTrackField`（四个集合不在支持集合里）----
        for extra in [
            "devices",
            "macros",
            "automationLanes",
            "clips",
            "volumeDb",
            "folderId",
        ] {
            let mut object = serde_json::json!({
                "kind": ADD_TRACK_KIND,
                "trackId": "01J8ZQ000000000000000000TR",
                "name": "N",
                "trackKind": "Midi",
            });
            object
                .as_object_mut()
                .expect("对象")
                .insert(extra.to_owned(), serde_json::json!([]));
            let fault =
                parse_ops(&serde_json::json!([object])).expect_err("集合/混音标量都不在支持集合里");
            assert_eq!(
                reason_of(&fault).as_deref(),
                Some("unknownAddTrackField"),
                "`{extra}` 必须被点名拒绝 (空形状是**决定**)"
            );
        }

        // ---- 空名 ⇒ 响亮拒绝（模型不查空名）----
        let empty_name = parse_ops(&serde_json::json!([
            {"kind": ADD_TRACK_KIND, "trackId": "01J8ZQ000000000000000000TR",
             "name": "", "trackKind": "Midi"}
        ]))
        .expect_err("空名必须被拒");
        assert_eq!(
            reason_of(&empty_name).as_deref(),
            Some("valueMustBeNonEmptyString")
        );

        // ---- 非规范类型名 ⇒ 响亮拒绝 ----
        let bad_kind = parse_ops(&serde_json::json!([
            {"kind": ADD_TRACK_KIND, "trackId": "01J8ZQ000000000000000000TR",
             "name": "N", "trackKind": "midi"}
        ]))
        .expect_err("大小写拼错必须被拒");
        assert_eq!(reason_of(&bad_kind).as_deref(), Some("unknownTrackKind"));
    }

    /// `addTrack` 的**字面名与支持集合**被钉住。
    #[test]
    fn add_track_field_names_are_pinned() {
        assert_eq!(ADD_TRACK_KIND, "addTrack");
        assert_eq!(ADD_TRACK_FIELDS, ["kind", "trackId", "name", "trackKind"]);
        assert_eq!(ADD_TRACK_TRACK_FIELD, "trackId");
        assert_eq!(ADD_TRACK_NAME_FIELD, "name");
        assert_eq!(ADD_TRACK_KIND_FIELD, "trackKind");
        assert_ne!(
            ADD_TRACK_KIND, REMOVE_TRACK_KIND,
            "新增与取走是**相反**的两件事, 共用一个 kind 会让调用方分不开"
        );
    }

    /// `connectRouting`：解析 → 编译到**正是** `Op::ConnectRouting` → 边真的进图 →
    /// **撤销一步**恢复原状；两端节点不存在 / 边身份重复各自**结构化拒绝**。
    ///
    /// 注入（实测红）：① 编译成 `DisconnectRouting` ⇒ "边必须真的进图"那条断言红；
    /// ② 去掉节点存在性检查 ⇒ 那条 `routingNodeNotFound` 断言红；
    /// ③ `edgeKind` 的四个规范名改成别的 ⇒ 解析那一步红。
    #[test]
    fn connect_routing_lands_an_edge_and_inverts_byte_for_byte() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        // 两端都取**已在** `nodes` 里的身份（夹具的图很完整）。
        let source = project.routing_graph.nodes[0];
        let destination = project
            .routing_graph
            .nodes
            .iter()
            .copied()
            .find(|node| *node != source)
            .expect("夹具至少两个节点");
        let edge_id = EntityId::from_str("01J8ZQ00000000000000000EDG").expect("ULID");
        assert!(
            !project.routing_graph.edges.contains_key(&edge_id),
            "夹具前提: 边身份是新的"
        );
        let edges_before = project.routing_graph.edges.len();
        let bytes_before = serde_json::to_string(&project).expect("序列化");

        // ---- ① 平铺形状能解析，分类正确 ----
        let ops = parse_ops(&serde_json::json!([{
            "kind": CONNECT_ROUTING_KIND,
            "edgeId": edge_id.to_canonical_string(),
            "sourceNode": source.to_canonical_string(),
            "destinationNode": destination.to_canonical_string(),
            "edgeKind": "TrackToBus",
            "gainDb": -3.0,
        }]))
        .expect("平铺形状必须被接受");
        assert_eq!(
            ops[0].kind_name(),
            CONNECT_ROUTING_KIND,
            "kind 必须回环一致"
        );
        assert!(ops[0].is_routing_level(), "必须落在路由级那个桶里");
        assert!(!ops[0].is_note_level(), "路由级不读不写音符");
        assert_eq!(
            ops[0],
            NoteOp::ConnectRouting {
                edge_id,
                source_node: source,
                destination_node: destination,
                kind: RoutingKind::TrackToBus,
                gain_db: Some(-3.0),
            }
        );

        // ---- ② 编译到**正是** `Op::ConnectRouting` ----
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::ConnectRouting { edge } => {
                assert_eq!(edge.id, edge_id);
                assert_eq!(edge.source_node, source);
                assert_eq!(edge.destination_node, destination);
                assert_eq!(edge.kind, RoutingKind::TrackToBus);
                assert_eq!(edge.gain_db, Some(-3.0));
            }
            other => panic!("应当是 ConnectRouting: {other:?}"),
        }

        // ---- ③ 应用：边真的进图，且工程仍然合法 ----
        compiled[0].apply(&mut project).expect("建边");
        assert!(
            project.routing_graph.edges.contains_key(&edge_id),
            "边必须真的进 `routing_graph.edges`"
        );
        assert_eq!(project.routing_graph.edges.len(), edges_before + 1);
        assert!(project.validate().is_ok(), "建边之后工程必须仍然合法");

        // ---- ④ **撤销一步**逐字节恢复 ----
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            project.routing_graph.edges.len(),
            edges_before,
            "边数回到原值"
        );
        assert!(
            !project.routing_graph.edges.contains_key(&edge_id),
            "边必须被摘掉"
        );
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "一步撤销必须逐字节回到原状"
        );
    }

    /// `connectRouting` 的两条前置条件各自**结构化拒绝**，且 `gainDb` 的
    /// "省略 vs `null`" 与 `setRoutingGain` **同口径**（都 = 单位增益）。
    #[test]
    fn connect_routing_refuses_a_missing_node_and_a_duplicate_edge() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let source = project.routing_graph.nodes[0];
        let existing_edge = *project.routing_graph.edges.keys().next().expect("样本边");

        // ---- ① 两端节点必须已在 `nodes` 里 ----
        let ghost = EntityId::from_str("01J8ZQ000000000000000000GH").expect("ULID");
        assert!(
            !project.routing_graph.nodes.contains(&ghost),
            "夹具前提: 这个身份不在图里"
        );
        let dangling = parse_ops(&serde_json::json!([{
            "kind": CONNECT_ROUTING_KIND,
            "edgeId": "01J8ZQ00000000000000000EDG",
            "sourceNode": source.to_canonical_string(),
            "destinationNode": ghost.to_canonical_string(),
            "edgeKind": "TrackToBus",
        }]))
        .expect("解析不查文档");
        let fault =
            compile(&project, &track_id, &clip_id, &dangling).expect_err("节点不在图里必须被拒");
        assert_eq!(reason_of(&fault).as_deref(), Some("routingNodeNotFound"));

        // ---- ② 边身份不重复 ----
        let duplicate = parse_ops(&serde_json::json!([{
            "kind": CONNECT_ROUTING_KIND,
            "edgeId": existing_edge.to_canonical_string(),
            "sourceNode": source.to_canonical_string(),
            "destinationNode": project.routing_graph.nodes[1].to_canonical_string(),
            "edgeKind": "TrackToBus",
        }]))
        .expect("解析不查文档");
        let fault = compile(&project, &track_id, &clip_id, &duplicate)
            .expect_err("已在 edges 里的身份必须被拒");
        assert_eq!(
            reason_of(&fault).as_deref(),
            Some("routingEdgeAlreadyExists")
        );

        // ---- ③ `gainDb` 省略与 `null` **同口径** = 单位增益 ----
        for payload in [
            serde_json::json!({"kind": CONNECT_ROUTING_KIND, "edgeId": "01J8ZQ00000000000000000EDG",
                "sourceNode": source.to_canonical_string(),
                "destinationNode": project.routing_graph.nodes[1].to_canonical_string(),
                "edgeKind": "TrackToBus"}),
            serde_json::json!({"kind": CONNECT_ROUTING_KIND, "edgeId": "01J8ZQ00000000000000000EDG",
                "sourceNode": source.to_canonical_string(),
                "destinationNode": project.routing_graph.nodes[1].to_canonical_string(),
                "edgeKind": "TrackToBus", "gainDb": null}),
        ] {
            let ops = parse_ops(&serde_json::json!([payload])).expect("解析");
            match &ops[0] {
                NoteOp::ConnectRouting { gain_db, .. } => {
                    assert_eq!(*gain_db, None, "省略与 null 都必须 = 单位增益 (None)");
                }
                other => panic!("{other:?}"),
            }
        }

        // ---- ④ 未知键 / 非规范类型名 / 非有限增益都响亮拒绝 ----
        let unknown = parse_ops(&serde_json::json!([{
            "kind": CONNECT_ROUTING_KIND, "edgeId": "01J8ZQ00000000000000000EDG",
            "sourceNode": source.to_canonical_string(),
            "destinationNode": source.to_canonical_string(),
            "edgeKind": "TrackToBus", "edge": {"id": "x"},
        }]))
        .expect_err("嵌套的 edge 对象必须被拒");
        assert_eq!(
            reason_of(&unknown).as_deref(),
            Some("unknownConnectRoutingField")
        );

        let bad_kind = parse_ops(&serde_json::json!([{
            "kind": CONNECT_ROUTING_KIND, "edgeId": "01J8ZQ00000000000000000EDG",
            "sourceNode": source.to_canonical_string(),
            "destinationNode": source.to_canonical_string(),
            "edgeKind": "trackToBus",
        }]))
        .expect_err("大小写拼错必须被拒");
        assert_eq!(reason_of(&bad_kind).as_deref(), Some("unknownRoutingKind"));

        let bad_gain = parse_ops(&serde_json::json!([{
            "kind": CONNECT_ROUTING_KIND, "edgeId": "01J8ZQ00000000000000000EDG",
            "sourceNode": source.to_canonical_string(),
            "destinationNode": source.to_canonical_string(),
            "edgeKind": "TrackToBus", "gainDb": 1.0e300,
        }]))
        .expect_err("增益必须有限（收窄到 f32 后）");
        assert_eq!(reason_of(&bad_gain).as_deref(), Some("nonFiniteValue"));
    }

    /// `connectRouting` 的**字面名与支持集合**被钉住。
    #[test]
    fn connect_routing_field_names_are_pinned() {
        assert_eq!(CONNECT_ROUTING_KIND, "connectRouting");
        assert_eq!(CONNECT_EDGE_FIELD, "edgeId");
        assert_eq!(
            CONNECT_EDGE_FIELD, ROUTING_EDGE_FIELD,
            "同一个字面量（同一种实体）"
        );
        assert_eq!(CONNECT_SOURCE_FIELD, "sourceNode");
        assert_eq!(CONNECT_DESTINATION_FIELD, "destinationNode");
        assert_eq!(CONNECT_EDGE_KIND_FIELD, "edgeKind");
        assert_eq!(CONNECT_GAIN_FIELD, "gainDb");
        assert_eq!(
            CONNECT_ROUTING_FIELDS,
            [
                "kind",
                "edgeId",
                "sourceNode",
                "destinationNode",
                "edgeKind",
                "gainDb"
            ]
        );
        assert_ne!(
            CONNECT_ROUTING_KIND, DISCONNECT_ROUTING_KIND,
            "建边与断开是**相反**的两件事, 共用一个 kind 会让调用方分不开"
        );
    }

    /// `setTempo`：解析 → 编译到**正是** `Op::SetTempo` → 撤销一步逐字节恢复；
    /// 越界 / 非有限 / 陈旧 `old_bpm` 各自结构化拒绝。
    ///
    /// 注入（实测红）：① 编译成 `SetAuthor` ⇒ 那条 `Op::SetTempo` 匹配断言红；
    /// ② 去掉区间前置条件 ⇒ 越界那条断言红；③ `old_bpm` 取自调用方而非文档 ⇒
    /// 陈旧载荷那条断言红。
    #[test]
    fn set_tempo_compiles_the_document_value_and_inverts_byte_for_byte() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let bpm_before = project.bpm;
        let bytes_before = serde_json::to_string(&project).expect("序列化");

        // ---- ① 解析：`old_bpm` 是占位，真值在编译期读文档 ----
        let ops = parse_ops(&serde_json::json!([
            {"kind": SET_TEMPO_KIND, "bpm": 140.0}
        ]))
        .expect("规范形状必须被接受");
        assert_eq!(ops[0].kind_name(), SET_TEMPO_KIND, "kind 必须回环一致");
        assert!(!ops[0].is_note_level(), "顶层文档字段不读不写音符");

        // ---- ② 编译到**正是** `Op::SetTempo`，且 `old_bpm` == 文档现值 ----
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::SetTempo { old_bpm, new_bpm } => {
                assert_eq!(
                    *old_bpm, bpm_before,
                    "old 必须等于**文档现值**（⛔ 非调用方声明）"
                );
                assert_eq!(*new_bpm, 140.0);
            }
            other => panic!("应当是 SetTempo: {other:?}"),
        }

        // ---- ③ 应用 + **撤销一步**逐字节恢复 ----
        compiled[0].apply(&mut project).expect("设置速度");
        assert_eq!(project.bpm, 140.0, "速度必须真的变了");
        assert!(project.validate().is_ok());
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(project.bpm, bpm_before, "逆操作必须回到原速度");
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "一步撤销必须逐字节回到原状"
        );
    }

    /// `setTempo` 的越界/非有限与陈旧载荷**分别**给出结构化理由。
    #[test]
    fn set_tempo_refuses_out_of_range_non_finite_and_stale_payloads() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);

        // ---- 区间外（含端点外）：**前置条件**拒（⛔ 不靠 apply 后的 validate 兜底）----
        for bad in [19.999_f64, 999.001] {
            let ops = parse_ops(&serde_json::json!([{"kind": SET_TEMPO_KIND, "bpm": bad}]))
                .expect("解析不查区间");
            let fault = compile(&project, &track_id, &clip_id, &ops).expect_err("区间外必须被拒");
            assert_eq!(
                reason_of(&fault).as_deref(),
                Some("bpmOutOfRange"),
                "{bad} 必须报区间理由"
            );
        }
        // ⚠ JSON **没有** NaN / Infinity 字面量 —— 它们在 `serde_json` 里是 `null`，
        //    因此非有限值的**可观测**入口是"`null`"与"极大数收窄"两条：
        //    ① `null` ⇒ 解析期 `valueMustBeNumber`；② 超出 `f64` 的巨数 ⇒
        //    `serde_json` 解析失败（在 `parse_ops` 之前）。这里钉住 ①，
        //    并把"非有限"这一半**如实**说清：它在本工具面**不可表达**。
        let from_null = parse_ops(&serde_json::json!([{"kind": SET_TEMPO_KIND, "bpm": null}]))
            .expect_err("null 不是数字");
        assert_eq!(
            reason_of(&from_null).as_deref(),
            Some("valueMustBeNumber"),
            "JSON 的 null 不是非有限值的替身: 它在解析期就被拒, 理由必须点名"
        );
        // ---- 端点**含**在内：20.0 与 999.0 必须被接受 ----
        for good in [20.0_f64, 999.0] {
            let ops = parse_ops(&serde_json::json!([{"kind": SET_TEMPO_KIND, "bpm": good}]))
                .expect("解析");
            compile(&project, &track_id, &clip_id, &ops)
                .unwrap_or_else(|e| panic!("{good} 在区间内必须被接受: {e:?}"));
        }

        // ---- 陈旧 `old_bpm`：模型 `validate` 逐位比较 ⇒ `OpStateMismatch` ----
        let stale = Op::SetTempo {
            old_bpm: project.bpm + 1.0,
            new_bpm: 140.0,
        };
        let mut doc = project.clone();
        assert!(
            stale.apply(&mut doc).is_err(),
            "陈旧 old_bpm 必须被模型拒 (OpStateMismatch)"
        );
        // 且**逐位**：`-0.0` 与 `0.0` 不是同一个值。
        let mut zero = project.clone();
        zero.bpm = 0.0;
        let negative_zero = Op::SetTempo {
            old_bpm: -0.0,
            new_bpm: 140.0,
        };
        assert!(
            negative_zero.apply(&mut zero).is_err(),
            "-0.0 与 0.0 的**位**不同 ⇒ 必须被拒（逐位比较）"
        );
    }

    /// `setAuthor`：解析 → 编译到**正是** `Op::SetAuthor` → 撤销一步逐字节恢复；
    /// 陈旧 `old_author` 被拒；⛔ **不发明**空串/长度校验（模型没有）。
    #[test]
    fn set_author_compiles_the_document_value_and_inverts_byte_for_byte() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let author_before = project.author.clone();
        let bytes_before = serde_json::to_string(&project).expect("序列化");

        let ops = parse_ops(&serde_json::json!([
            {"kind": SET_AUTHOR_KIND, "author": "New Author"}
        ]))
        .expect("规范形状必须被接受");
        assert_eq!(ops[0].kind_name(), SET_AUTHOR_KIND, "kind 必须回环一致");
        assert!(!ops[0].is_note_level(), "顶层文档字段不读不写音符");

        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::SetAuthor {
                old_author,
                new_author,
            } => {
                assert_eq!(old_author, &author_before, "old 必须等于**文档现值**");
                assert_eq!(new_author, "New Author");
            }
            other => panic!("应当是 SetAuthor: {other:?}"),
        }

        compiled[0].apply(&mut project).expect("设置署名");
        assert_eq!(project.author, "New Author");
        assert!(project.validate().is_ok());
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(project.author, author_before, "逆操作必须回到原署名");
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "一步撤销必须逐字节回到原状"
        );

        // ---- 空串是**合法的明示值**（模型与 schema 都没有"非空"约束）----
        let empty = parse_ops(&serde_json::json!([{"kind": SET_AUTHOR_KIND, "author": ""}]))
            .expect("空串必须被接受（⛔ 本层不发明模型没有的约束）");
        let compiled_empty = compile(&project, &track_id, &clip_id, &empty).expect("编译");
        let mut doc = project.clone();
        compiled_empty[0].apply(&mut doc).expect("清空署名");
        assert_eq!(doc.author, "", "空串必须如实落地");

        // ---- 陈旧 `old_author` ⇒ `OpStateMismatch`（逐字节比较）----
        let stale = Op::SetAuthor {
            old_author: "Somebody Else".to_owned(),
            new_author: "X".to_owned(),
        };
        let mut doc = project.clone();
        assert!(stale.apply(&mut doc).is_err(), "陈旧署名必须被拒");

        // ---- 未知键 / 非字符串 响亮拒绝 ----
        let unknown = parse_ops(&serde_json::json!([
            {"kind": SET_AUTHOR_KIND, "author": "A", "bpm": 140.0}
        ]))
        .expect_err("未知键必须被拒");
        assert_eq!(
            reason_of(&unknown).as_deref(),
            Some("unknownSetAuthorField")
        );

        let not_string = parse_ops(&serde_json::json!([{"kind": SET_AUTHOR_KIND, "author": 3}]))
            .expect_err("非字符串必须被拒");
        assert_eq!(reason_of(&not_string).as_deref(), Some("valueMustBeString"));
    }

    /// `setTitle`：解析 → 编译到**正是** `Op::SetTitle` → `old_title` 来自**当前文档**
    /// → 撤销一步逐字节恢复；陈旧 `old_title` 被拒；空串/非 ASCII 照收（模型无约束）。
    ///
    /// ⭐ 本判据对着「工程建得出、标题改不了」：`title` 此前唯一的结构写点是
    /// `domain/project_create.rs:370`（`yeban_create_project`）。
    ///
    /// 注入（实测红）：① 编译成 `SetAuthor` ⇒ 那条 `Op::SetTitle` 匹配断言红；
    /// ② `old_title` 用解析期的占位空串而不是文档现值 ⇒ 陈旧标题那条断言红。
    #[test]
    fn set_title_compiles_the_document_value_and_inverts_byte_for_byte() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let title_before = project.title.clone();
        let author_before = project.author.clone();
        let bpm_before = project.bpm;
        let bytes_before = serde_json::to_string(&project).expect("序列化");

        // ---- ① 解析：`old_title` 是占位，真值在编译期读文档 ----
        let ops = parse_ops(&serde_json::json!([
            {"kind": SET_TITLE_KIND, "title": "夜半 · Yeban"}
        ]))
        .expect("规范形状必须被接受");
        assert_eq!(ops[0].kind_name(), SET_TITLE_KIND, "kind 必须回环一致");
        assert!(!ops[0].is_note_level(), "顶层文档字段不读不写音符");

        // ---- ② 编译到**正是** `Op::SetTitle`，且 `old_title` == 文档现值 ----
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::SetTitle {
                old_title,
                new_title,
            } => {
                assert_eq!(
                    old_title, &title_before,
                    "old 必须等于**文档现值**（⛔ 不信调用方声明）"
                );
                assert_eq!(new_title, "夜半 · Yeban");
            }
            other => panic!("应当是 SetTitle: {other:?}"),
        }

        compiled[0].apply(&mut project).expect("设置标题");
        assert_eq!(project.title, "夜半 · Yeban");
        assert_eq!(project.author, author_before, "改标题 ⛔ 不碰 author");
        assert_eq!(project.bpm, bpm_before, "改标题 ⛔ 不碰 bpm");
        assert!(project.validate().is_ok());
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(project.title, title_before, "逆操作必须回到原标题");
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "一步撤销必须逐字节回到原状"
        );

        // ---- 空串是**合法的明示值**（模型与 schema 都没有"非空"约束）----
        let empty = parse_ops(&serde_json::json!([{"kind": SET_TITLE_KIND, "title": ""}]))
            .expect("空串必须被接受（⛔ 本层不发明模型没有的约束）");
        let compiled_empty = compile(&project, &track_id, &clip_id, &empty).expect("编译");
        let mut doc = project.clone();
        compiled_empty[0].apply(&mut doc).expect("清空标题");
        assert_eq!(doc.title, "", "空串必须如实落地");
        assert!(doc.validate().is_ok(), "空标题的工程必须仍然合法");

        // ---- 陈旧 `old_title` ⇒ `OpStateMismatch`（逐字节比较）----
        let stale = Op::SetTitle {
            old_title: "别的标题".to_owned(),
            new_title: "X".to_owned(),
        };
        let mut doc = project.clone();
        assert!(stale.apply(&mut doc).is_err(), "陈旧标题必须被拒");
        assert_eq!(doc.title, title_before, "被拒之后文档必须不动");

        // ---- 未知键 / 非字符串 响亮拒绝 ----
        let unknown = parse_ops(&serde_json::json!([
            {"kind": SET_TITLE_KIND, "title": "T", "author": "A"}
        ]))
        .expect_err("未知键必须被拒");
        assert_eq!(reason_of(&unknown).as_deref(), Some("unknownSetTitleField"));

        let not_string = parse_ops(&serde_json::json!([{"kind": SET_TITLE_KIND, "title": 3}]))
            .expect_err("非字符串必须被拒");
        assert_eq!(reason_of(&not_string).as_deref(), Some("valueMustBeString"));
    }

    /// `setClipGain`：解析 → 编译到**正是** `Op::SetClipGain` → `old_gain_db` 来自
    /// **当前文档** → 一步撤销逐字节复原；**MIDI 片段**被内容类型门响亮拒绝。
    ///
    /// ⭐ 本判据对着「音频片段建得出、增益改不了」：`ClipContent::Audio::gain_db`
    /// 此前只在 `yeban_import_audio` 创建时由 `gainDb` 写一次
    /// （`domain/import_audio.rs:396` / `:501`）。
    ///
    /// 注入（实测红）：① `compile` 用解析期的占位 `0.0` 而不是文档现值 ⇒
    /// `old_gain_db` 那条断言红；② 去掉 `compile` 的内容类型门 ⇒ MIDI 那条断言红
    /// （模型会报 `ClipContentKindMismatch`，而本层先拒的文案不是那一句）。
    #[test]
    fn set_clip_gain_compiles_the_document_value_and_gates_midi() {
        let mut project = filled_project();
        // 顶层 `trackId`/`clipId` 仍指向 MIDI 材料（工具的签名口径），但本形态**自带**
        // `clipId` ⇒ 目标音频片段与顶层那对身份无关（`reads_envelope_clip() == false`）。
        let (track_id, clip_id) = lead_clip(&project);
        let audio_clip = project
            .clip_pool
            .values()
            .find(|entry| matches!(&entry.content, ClipContent::Audio { .. }))
            .expect("夹具必须有音频片段")
            .clone();
        let audio_id = audio_clip.id;
        let gain_before = match &project.clip_pool[&audio_id].content {
            ClipContent::Audio { gain_db, .. } => *gain_db,
            ClipContent::Midi { .. } => panic!("夹具的音频片段必须是 Audio"),
        };
        let bytes_before = serde_json::to_string(&project).expect("序列化");

        assert_eq!(SET_CLIP_GAIN_KIND, "setClipGain");
        assert_eq!(SET_CLIP_GAIN_FIELDS, ["kind", "clipId", "gainDb"]);
        assert_eq!(SET_CLIP_GAIN_GAIN_FIELD, "gainDb");
        // ---- ① 解析 + ② 编译到**正是** `Op::SetClipGain`，`old_gain_db` 来自文档 ----
        let ops = parse_ops(&serde_json::json!([{
            "kind": SET_CLIP_GAIN_KIND,
            "clipId": audio_id.to_canonical_string(),
            "gainDb": -6.0,
        }]))
        .expect("规范形状必须被接受");
        assert_eq!(ops[0].kind_name(), SET_CLIP_GAIN_KIND);
        assert!(!ops[0].is_note_level(), "改增益不读不写音符");
        assert!(
            !ops[0].reads_envelope_clip(),
            "本形态的 `clipId` 在操作对象里（自带寻址，⛔ 不是工具顶层那个）"
        );

        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::SetClipGain {
                clip_id,
                old_gain_db,
                new_gain_db,
            } => {
                assert_eq!(*clip_id, audio_id);
                assert_eq!(
                    old_gain_db.to_bits(),
                    gain_before.to_bits(),
                    "old 必须等于**文档现值**（⛔ 不信调用方声明）"
                );
                assert_eq!(*new_gain_db, -6.0);
            }
            other => panic!("应当是 SetClipGain: {other:?}"),
        }

        // ---- ③ 施加 ⇒ 读回逐位；逆 ⇒ 逐字节回原状 ----
        compiled[0].apply(&mut project).expect("设置增益");
        match &project.clip_pool[&audio_id].content {
            ClipContent::Audio { gain_db, .. } => {
                assert_eq!(gain_db.to_bits(), (-6.0_f32).to_bits(), "读回必须逐位相同")
            }
            ClipContent::Midi { .. } => panic!("目标不得变成 MIDI"),
        }
        assert!(project.validate().is_ok(), "改增益后工程必须仍合法");
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "一步撤销必须逐字节回到原状"
        );

        // ---- ④ MIDI 片段 ⇒ 内容类型门（响亮、⛔ panic ⛔ 静默）----
        let midi = parse_ops(&serde_json::json!([{
            "kind": SET_CLIP_GAIN_KIND,
            "clipId": clip_id.to_canonical_string(),
            "gainDb": -6.0,
        }]))
        .expect("解析期不查文档");
        let fault = compile(&project, &track_id, &clip_id, &midi).expect_err("MIDI 片段必须被拒");
        assert_eq!(
            fault.domain_code(),
            Some(ErrorCode::ClipNotFound),
            "内容类型门复用既有的 CLIP_NOT_FOUND（模型同一个变体也这么映射）"
        );
        println!("### 逐字：MIDI 片段调 setClipGain ⇒ {fault:?}");
        let message = format!("{fault:?}");
        assert!(
            message.contains("不是音频片段, 没有 gain_db 可改"),
            "逐字错误必须点明内容类型门: {message}"
        );
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "被拒之后文档必须逐字节不动"
        );

        // ---- ⑤ 未知键 / 非数字 / 收窄成 inf 都响亮拒绝 ----
        let unknown = parse_ops(&serde_json::json!([{
            "kind": SET_CLIP_GAIN_KIND,
            "clipId": audio_id.to_canonical_string(),
            "gainDb": -3.0,
            "trackId": audio_id.to_canonical_string(),
        }]))
        .expect_err("未知键必须被拒");
        assert_eq!(
            reason_of(&unknown).as_deref(),
            Some("unknownSetClipGainField")
        );

        let not_number = parse_ops(&serde_json::json!([{
            "kind": SET_CLIP_GAIN_KIND,
            "clipId": audio_id.to_canonical_string(),
            "gainDb": "loud",
        }]))
        .expect_err("非数字必须被拒");
        assert_eq!(reason_of(&not_number).as_deref(), Some("valueMustBeNumber"));

        // `1e300` 是有限 f64，但收窄到 f32 后是 `inf` ⇒ 必须在**收窄之后**拒。
        let narrowed = parse_ops(&serde_json::json!([{
            "kind": SET_CLIP_GAIN_KIND,
            "clipId": audio_id.to_canonical_string(),
            "gainDb": 1e300,
        }]))
        .expect_err("收窄成 inf 必须被拒");
        assert_eq!(
            reason_of(&narrowed).as_deref(),
            Some("nonFiniteValue"),
            "与 setParam/setRoutingGain 同一条口径"
        );
    }

    /// 三个形态的**字面名与支持集合**被钉住。
    #[test]
    fn top_level_field_kinds_are_pinned() {
        assert_eq!(SET_TEMPO_KIND, "setTempo");
        assert_eq!(SET_TEMPO_FIELDS, ["kind", "bpm"]);
        assert_eq!(SET_TEMPO_BPM_FIELD, "bpm");
        assert_eq!(SET_AUTHOR_KIND, "setAuthor");
        assert_eq!(SET_AUTHOR_FIELDS, ["kind", "author"]);
        assert_eq!(SET_AUTHOR_AUTHOR_FIELD, "author");
        assert_eq!(SET_TITLE_KIND, "setTitle");
        assert_eq!(SET_TITLE_FIELDS, ["kind", "title"]);
        assert_eq!(SET_TITLE_TITLE_FIELD, "title");
        assert_ne!(
            SET_TEMPO_KIND, SET_AUTHOR_KIND,
            "速度与署名是**两个不同**的顶层字段, 共用一个 kind 会让调用方分不开"
        );
        assert_ne!(
            SET_AUTHOR_KIND, SET_TITLE_KIND,
            "署名与标题是**两个不同**的顶层字段, 共用一个 kind 会让调用方分不开"
        );
    }

    /// 段落**写入**形态（`setSection`）的两条路：新建（`create: true`）与更新
    /// （缺省，撤销载荷是**文档现值**整条），两条都能被 `Op::invert` **逐字节**回退。
    ///
    /// 这一条对着"段落建得出、删得掉、⛔ 改不了"的缺口：`Op::SetSection` 在 MCP 侧
    /// 此前**唯一**的构造点是 `yeban_propose_section` 的内部建批，而工具面只有
    /// `removeSection`。
    ///
    /// 注入（实测红）：把 `old_section` 从 `current` 换成 `None`（把更新当成新建）
    /// ⇒ 更新那条的 `old_section` 断言红，而且模型的前置条件会在 `apply` 时拒绝它。
    #[test]
    fn set_section_creates_and_updates_and_inverts_byte_for_byte() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);

        // ---- 新建：身份**不在**文档里，四个属性一起给 ----
        let fresh = EntityId::from_str("01J8ZQ00000000000000000SEC").expect("ULID");
        assert!(
            !project.sections.contains_key(&fresh),
            "夹具前提: 身份是新的"
        );
        let bytes_before_create = serde_json::to_string(&project).expect("序列化");
        let ops = parse_ops(&serde_json::json!([{
            "kind": SET_SECTION_KIND,
            "section": {
                "create": true,
                "sectionId": fresh.to_canonical_string(),
                "name": "Bridge",
                "startTick": 1920,
                "endTick": 3840,
                "color": "#22AA88",
            }
        }]))
        .expect("规范形状必须被接受");
        assert_eq!(ops[0].kind_name(), SET_SECTION_KIND, "kind 必须回环一致");
        assert!(ops[0].is_section_level(), "必须落在段落级那个桶里");
        assert!(!ops[0].is_note_level(), "段落级不读不写音符");
        assert!(!ops[0].is_routing_level(), "段落不是路由图的一部分");
        assert!(!ops[0].is_scene_level(), "段落不是场景");
        assert_eq!(
            ops[0],
            NoteOp::SetSection {
                section_id: fresh,
                create: true,
                patch: SectionPatch {
                    name: Some("Bridge".to_owned()),
                    start_tick: Some(1920),
                    end_tick: Some(3840),
                    color: Some(Some("#22AA88".to_owned())),
                },
            }
        );

        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::SetSection {
                section_id,
                old_section,
                new_section,
            } => {
                assert_eq!(*section_id, fresh);
                assert!(old_section.is_none(), "新建时 old 必须是 None");
                assert_eq!(new_section.id, fresh, "身份由寻址给出, 覆盖里没有它");
                assert_eq!(new_section.name, "Bridge");
                assert_eq!(new_section.start_tick, 1920);
                assert_eq!(new_section.end_tick, 3840);
                assert_eq!(new_section.color.as_deref(), Some("#22AA88"));
            }
            other => panic!("应当是 SetSection: {other:?}"),
        }
        compiled[0].apply(&mut project).expect("新建段落");
        assert!(
            project.sections.contains_key(&fresh),
            "段落必须真的进 sections"
        );
        assert!(project.validate().is_ok(), "新建之后工程必须仍然合法");
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert!(!project.sections.contains_key(&fresh), "逆操作必须摘掉它");
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before_create,
            "新建 + 一步撤销必须逐字节回到原状"
        );

        // ---- 更新：只改**区间与色标**（`name` 不给 ⇒ 保留现值, 是**合并**）----
        let existing = *project.sections.keys().next().expect("样本段落");
        let before = project.sections[&existing].clone();
        let bytes_before_update = serde_json::to_string(&project).expect("序列化");
        let ops = parse_ops(&serde_json::json!([{
            "kind": SET_SECTION_KIND,
            "section": {
                "sectionId": existing.to_canonical_string(),
                "startTick": 60,
                "endTick": 480,
            }
        }]))
        .expect("更新形状必须被接受");
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        match &compiled[0] {
            Op::SetSection {
                old_section,
                new_section,
                ..
            } => {
                assert_eq!(
                    old_section.as_ref(),
                    Some(&before),
                    "old 必须等于**文档现值**（⛔ 非调用方声明）"
                );
                assert_eq!(new_section.start_tick, 60, "提了的键被覆盖");
                assert_eq!(new_section.end_tick, 480);
                assert_eq!(new_section.name, before.name, "没提的键保留现值（合并）");
                assert_eq!(new_section.color, before.color, "没提的键保留现值（合并）");
            }
            other => panic!("应当是 SetSection: {other:?}"),
        }
        compiled[0].apply(&mut project).expect("更新段落");
        assert_eq!(project.sections[&existing].start_tick, 60);
        assert_eq!(
            project.sections[&existing].name, before.name,
            "合并语义必须成立"
        );
        assert!(project.validate().is_ok());
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before_update,
            "更新 + 一步撤销必须逐字节回到原状"
        );
    }

    /// `setSection` 的三条前置条件各自**结构化拒绝**：新建撞已有、更新找不到、
    /// 陈旧 `old_section` 被模型拒（`OpStateMismatch`）。
    ///
    /// 注入（实测红）：① 去掉新建的冲突拒绝 ⇒ `sectionAlreadyExists` 断言红；
    /// ② 去掉更新的存在性拒绝 ⇒ `sectionNotFound` 断言红；
    /// ③ 把 `old_section` 换成 `Some(伪造值)` ⇒ 陈旧载荷那条断言红。
    #[test]
    fn set_section_refuses_existing_on_create_missing_on_update_and_stale_payloads() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let existing = *project.sections.keys().next().expect("样本段落");

        // ---- 新建但身份已在 ⇒ CONFLICT / `sectionAlreadyExists` ----
        let dup = parse_ops(&serde_json::json!([{
            "kind": SET_SECTION_KIND,
            "section": {
                "create": true,
                "sectionId": existing.to_canonical_string(),
                "name": "Dup",
                "startTick": 0,
                "endTick": 960,
            }
        }]))
        .expect("解析不查文档");
        let fault =
            compile(&project, &track_id, &clip_id, &dup).expect_err("已在文档里的身份必须被拒");
        assert_eq!(reason_of(&fault).as_deref(), Some("sectionAlreadyExists"));

        // ---- 更新但身份不在 ⇒ ENTITY_NOT_FOUND / `sectionNotFound` ----
        let ghost = EntityId::from_str("01J8ZQ000000000000000000GH").expect("ULID");
        assert!(!project.sections.contains_key(&ghost), "夹具前提");
        let dangling = parse_ops(&serde_json::json!([{
            "kind": SET_SECTION_KIND,
            "section": {"sectionId": ghost.to_canonical_string(), "name": "Nope"}
        }]))
        .expect("解析不查文档");
        let fault = compile(&project, &track_id, &clip_id, &dangling)
            .expect_err("不在文档里的身份必须被拒");
        assert_eq!(reason_of(&fault).as_deref(), Some("sectionNotFound"));

        // ---- 新建却**没给** `name` ⇒ 响亮拒绝（模型不查空名）----
        let unnamed = parse_ops(&serde_json::json!([{
            "kind": SET_SECTION_KIND,
            "section": {
                "create": true,
                "sectionId": "01J8ZQ00000000000000000SEC",
                "startTick": 0,
                "endTick": 960,
            }
        }]))
        .expect_err("新建不给 name 必须被拒");
        assert_eq!(
            reason_of(&unnamed).as_deref(),
            Some("sectionNameRequiredWhenCreating")
        );

        // ---- 陈旧 `old_section`：模型逐字段比较 ⇒ `OpStateMismatch` ----
        let stale = Op::SetSection {
            section_id: existing,
            old_section: None, // 伪造: 文档里其实**有**这条段落
            new_section: SectionV3 {
                id: existing,
                name: "Forged".to_owned(),
                start_tick: 0,
                end_tick: 960,
                color: None,
            },
        };
        let mut doc = project.clone();
        assert!(
            stale.apply(&mut doc).is_err(),
            "陈旧 old_section 必须被模型拒 (OpStateMismatch)"
        );

        // ---- 区间反向（`endTick <= startTick`）由**模型**判（⛔ 本层不复制）----
        let zero = Op::SetSection {
            section_id: existing,
            old_section: project.sections.get(&existing).cloned(),
            new_section: SectionV3 {
                id: existing,
                name: "Zero".to_owned(),
                start_tick: 960,
                end_tick: 960,
                color: None,
            },
        };
        let mut doc = project.clone();
        assert!(
            zero.apply(&mut doc).is_err(),
            "零长度区间必须被模型拒 (ZeroDuration)"
        );
    }

    /// `setSection` 的形状拒绝：未知键、顶层嵌套错位、非整数 tick、空名、非字符串色标。
    #[test]
    fn set_section_shapes_fail_loudly() {
        let base = serde_json::json!({
            "create": true,
            "sectionId": "01J8ZQ00000000000000000SEC",
            "name": "Intro",
            "startTick": 0,
            "endTick": 960,
        });

        // 载荷里的未知键
        let mut bad = base.clone();
        bad["tempo"] = serde_json::json!(120.0);
        let fault = parse_ops(&serde_json::json!([{"kind": SET_SECTION_KIND, "section": bad}]))
            .expect_err("载荷里的未知键必须被拒");
        assert_eq!(reason_of(&fault).as_deref(), Some("unknownSetSectionField"));

        // 操作对象顶层的未知键（把 `sectionId` 放到顶层是最像"写对了"的错法）
        let fault = parse_ops(&serde_json::json!([{
            "kind": SET_SECTION_KIND,
            "sectionId": "01J8ZQ00000000000000000SEC",
            "section": base.clone(),
        }]))
        .expect_err("顶层多写键必须被拒");
        assert_eq!(
            reason_of(&fault).as_deref(),
            Some("unknownSetSectionOpField")
        );

        // 非整数 tick
        let mut bad = base.clone();
        bad["startTick"] = serde_json::json!("0");
        let fault = parse_ops(&serde_json::json!([{"kind": SET_SECTION_KIND, "section": bad}]))
            .expect_err("非整数 tick 必须被拒");
        assert_eq!(
            reason_of(&fault).as_deref(),
            Some("sectionTickMustBeUnsignedInteger")
        );

        // 空名
        let mut bad = base.clone();
        bad["name"] = serde_json::json!("");
        let fault = parse_ops(&serde_json::json!([{"kind": SET_SECTION_KIND, "section": bad}]))
            .expect_err("空名必须被拒");
        assert_eq!(
            reason_of(&fault).as_deref(),
            Some("sectionNameMustBeNonEmptyString")
        );

        // 非字符串色标
        let mut bad = base.clone();
        bad["color"] = serde_json::json!(3);
        let fault = parse_ops(&serde_json::json!([{"kind": SET_SECTION_KIND, "section": bad}]))
            .expect_err("非字符串色标必须被拒");
        // ⚠ `read_non_empty_string` 对"类型不对"与"空串"给**两个**理由
        // （与 `sceneColorMustBeNonEmptyString` / `…MustBeString` 同族）：
        // 这里给的是**非字符串**，因此理由带 `MustBeString` 后缀。
        assert_eq!(
            reason_of(&fault).as_deref(),
            Some("sectionColorMustBeNonEmptyStringMustBeString")
        );

        // 缺 `section` 载荷 —— ⚠ `missing()` 这个既有辅助**不带** `reason`
        // （与 `setScene` 缺载荷同一条路径），因此这里只断言"响亮失败"，
        // 并**走既有读法**确认契约码是 `INVALID_PARAMETER_RANGE`，⛔ 不发明新理由。
        let fault = parse_ops(&serde_json::json!([{"kind": SET_SECTION_KIND}]))
            .expect_err("缺载荷必须被拒");
        let response = fault.to_tool_response().expect("失败必须能变成工具响应");
        assert_eq!(
            response.pointer("/error/code").and_then(Value::as_str),
            Some("INVALID_PARAMETER_RANGE")
        );
        assert!(
            response
                .pointer("/error/message")
                .and_then(Value::as_str)
                .is_some_and(|message| message.contains(SECTION_PAYLOAD_FIELD)),
            "缺失消息必须点名缺的是哪个键: {response}"
        );
    }

    /// `setSection` 的**字面名与支持集合**被钉住。
    #[test]
    fn set_section_field_names_are_pinned() {
        assert_eq!(SET_SECTION_KIND, "setSection");
        assert_eq!(SECTION_PAYLOAD_FIELD, "section");
        assert_eq!(SECTION_CREATE_FIELD, "create");
        assert_eq!(SECTION_NAME_FIELD, "name");
        assert_eq!(SECTION_START_FIELD, "startTick");
        assert_eq!(SECTION_END_FIELD, "endTick");
        assert_eq!(SECTION_COLOR_FIELD, "color");
        assert_eq!(
            SET_SECTION_FIELDS,
            [
                "create",
                "sectionId",
                "startTick",
                "endTick",
                "name",
                "color"
            ]
        );
        assert_eq!(SET_SECTION_OP_FIELDS, ["kind", "section"]);
        assert_ne!(
            SET_SECTION_KIND, REMOVE_SECTION_KIND,
            "写入与取走是**相反**的两件事, 共用一个 kind 会让调用方分不开"
        );
    }

    /// ⭐ **`removeAsset` 主判据**：取走一条**未被引用**的资产索引项 ⇒ 逐字节可逆
    /// （**撤销一步**回原状）＋ 分类正确。
    ///
    /// ⚠ 为什么它非有不可：在它之前 `Op::RemoveAsset` 在 `yeban-mcp` 里**零构造点**
    /// （只由模型的 `structural_inverse` 产出）⇒ 用户**只能靠撤销那次导入**取走索引项，
    /// 没有一个"删掉这个资产"的形态 —— 与 `removeScene` 补齐前同族。
    ///
    /// 注入（实测红）：把 `previous_asset` 换成一份伪造值 ⇒ 撤销那条断言红
    /// （而且模型的前置条件会在 `apply` 时拒它）。
    #[test]
    fn remove_asset_takes_an_unreferenced_asset_and_inverts_byte_for_byte() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        // 目标：一条**没人引用**的合成资产（夹具里那条音频片段引用的哈希不能当目标）。
        let orphan = AssetHash::of_bytes(b"removeAsset-main-criterion");
        project.assets.insert(
            orphan.clone(),
            yeban_model::AssetMetadata {
                hash: orphan.clone(),
                original_path: "samples/orphan.wav".to_owned(),
                byte_len: 64,
                media_kind: yeban_model::MediaKind::Audio,
                license: "CC0-1.0".to_owned(),
            },
        );
        let before = project.assets.len();
        let bytes_before = serde_json::to_string(&project).expect("序列化");

        let ops = parse_ops(&serde_json::json!([{
            "kind": REMOVE_ASSET_KIND,
            "assetHash": orphan.as_str(),
        }]))
        .expect("规范形状必须被接受");
        assert_eq!(ops[0].kind_name(), REMOVE_ASSET_KIND, "kind 必须回环一致");
        assert!(ops[0].is_asset_level(), "必须落在资产级那个桶里");
        assert!(!ops[0].is_note_level(), "资产级不读不写音符");
        assert!(!ops[0].is_routing_level(), "资产不是路由图的一部分");
        assert!(!ops[0].is_section_level(), "资产不是曲段");
        assert!(!ops[0].is_scene_level(), "资产不是场景");
        assert_eq!(
            ops[0],
            NoteOp::RemoveAsset {
                asset_hash: orphan.clone()
            }
        );

        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::RemoveAsset {
                asset_hash,
                previous_asset,
            } => {
                assert_eq!(asset_hash, &orphan);
                assert_eq!(
                    previous_asset,
                    project.assets.get(&orphan).expect("现值"),
                    "撤销载荷必须**从当前文档读**（⛔ 非调用方声明）"
                );
            }
            other => panic!("应当是 RemoveAsset: {other:?}"),
        }
        compiled[0].apply(&mut project).expect("取走");
        assert!(!project.assets.contains_key(&orphan), "索引项必须真的没了");
        assert_eq!(project.assets.len(), before - 1);
        assert!(project.validate().is_ok(), "取走之后工程必须仍然合法");
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(project.assets.len(), before, "撤销必须把它放回来");
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "一步撤销必须逐字节回到原状"
        );
    }

    /// ⭐ **控制臂**（第 46／47 条）：取一条**仍被引用**的资产 ⇒ **必须被拒**、
    /// 且**文档一个字都不改**。此臂**独立变色**（只差目标哈希）。
    ///
    /// ⚠ 这条是本单的**关键安全属性**：`project.assets` 是 `ClipContent::Audio` 的
    /// 许可留痕 ⇒ 取走一个仍被引用的哈希会留下**指向不存在资产的片段**，
    /// 而 `YebanProjectV1::validate` **不查**这条（实测：`project.rs` 里
    /// `assets.get` / `assets.contains_key` 零命中）⇒ ⛔ 模型不会替我们拦。
    ///
    /// 注入（实测红）：删掉 `assetInUse` 那段守卫 ⇒ 本判据红（它会把资产真的取走）。
    #[test]
    fn remove_asset_refuses_a_referenced_asset_and_leaves_the_document_untouched() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let referenced = project
            .clip_pool
            .values()
            .find_map(|entry| match &entry.content {
                ClipContent::Audio { asset, .. } => Some(asset.clone()),
                ClipContent::Midi { .. } => None,
            })
            .expect("夹具必须有引用资产的音频片段");
        let bytes_before = serde_json::to_string(&project).expect("序列化");

        let ops = parse_ops(&serde_json::json!([{
            "kind": REMOVE_ASSET_KIND,
            "assetHash": referenced.as_str(),
        }]))
        .expect("解析不查文档");
        let fault =
            compile(&project, &track_id, &clip_id, &ops).expect_err("仍被引用的资产必须被拒");
        let Fault::Domain { data, .. } = &fault else {
            panic!("应当是领域失败, 实际 {fault:?}");
        };
        let data = data.as_ref().expect("拒绝必须带 data");
        assert_eq!(
            data.pointer("/reason").and_then(Value::as_str),
            Some("assetInUse"),
            "理由必须点名 assetInUse: {data}"
        );
        assert!(
            data.pointer("/clipIds")
                .and_then(Value::as_array)
                .is_some_and(|list| !list.is_empty()),
            "必须回显引用它的片段身份: {data}"
        );

        // ⚠ 控制臂要证明"文档不动"：拒绝发生在构造 `Op` **之前** ⇒ 无从施加。
        let untouched = project.clone();
        assert_eq!(
            serde_json::to_string(&untouched).expect("序列化"),
            bytes_before,
            "被拒的调用⛔ 不得改动文档"
        );
        // 并且同一份文档里那个哈希**仍然在**。
        assert!(untouched.assets.contains_key(&referenced));
    }

    /// `removeAsset` 的形状拒绝：未知键（含工具顶层实参搬进来）、非法哈希、
    /// 以及**不存在的哈希**（`assetNotFound`）。
    ///
    /// 注入（实测红）：把 `assetNotFound` 那条拒绝删掉 ⇒ 第二条断言红
    /// （模型侧只会报一个泛化的 `OpStateMismatch`）。
    #[test]
    fn remove_asset_shapes_and_a_missing_hash_fail_loudly() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let orphan = AssetHash::of_bytes(b"removeAsset-shapes");
        let exists = project.assets.contains_key(&orphan);

        // 未知键：把**工具顶层**的 `trackId` 搬进操作对象（最像"写对了"的错法）。
        let fault = parse_ops(&serde_json::json!([{
            "kind": REMOVE_ASSET_KIND,
            "assetHash": "00".repeat(32),
            "trackId": "01J8ZQ00000000000000000001",
        }]))
        .expect_err("未知键必须被拒");
        assert_eq!(
            reason_of(&fault).as_deref(),
            Some("unknownRemoveAssetField")
        );

        // 非法哈希（长度不对）。
        // ⚠ 哈希不合法 ⇒ 我的解析臂自己给 `INVALID_PARAMETER_RANGE`
        // （`AssetHash::parse` 的失败**不**带 `reason`，因此那条判在解析臂里）。
        let fault = parse_ops(&serde_json::json!([{
            "kind": REMOVE_ASSET_KIND,
            "assetHash": "not-a-hash",
        }]))
        .expect_err("非法哈希必须被拒");
        assert_eq!(
            fault.domain_code(),
            Some(ErrorCode::InvalidParameterRange),
            "非法哈希是参数错误: {fault:?}"
        );

        // 不存在的哈希 ⇒ `assetNotFound`（编译期，带 reason）。
        assert!(!exists, "夹具前提");
        let ops = parse_ops(&serde_json::json!([{
            "kind": REMOVE_ASSET_KIND,
            "assetHash": orphan.as_str(),
        }]))
        .expect("解析不查文档");
        let fault =
            compile(&project, &track_id, &clip_id, &ops).expect_err("文档里没有的哈希必须被拒");
        assert_eq!(reason_of(&fault).as_deref(), Some("assetNotFound"));
    }

    /// `removeAsset` 的**字面名与支持集合**被钉住。
    #[test]
    fn remove_asset_field_names_are_pinned() {
        assert_eq!(REMOVE_ASSET_KIND, "removeAsset");
        assert_eq!(REMOVE_ASSET_HASH_FIELD, "assetHash");
        assert_eq!(REMOVE_ASSET_FIELDS, ["kind", "assetHash"]);
        assert_ne!(
            REMOVE_ASSET_KIND, REMOVE_CLIP_KIND,
            "取走资产与取走片段是**两种不同的实体**, 共用一个 kind 会让调用方分不开"
        );
    }

    /// ⭐ **`setAsset` 主判据**：改一个**已存在**资产的 `license` ⇒ 逐字节可逆
    /// （**撤销一步**回原值）＋ 分类正确 ＋ 只改可改字段。
    ///
    /// ⚠ 为什么它非有不可：在它之前 `Op::SetAsset` 的唯一构造点写死 `old_asset: None`
    /// （创建）⇒ `old_asset: Some(..)` 的**更新**语义用户不可达 ⇒ "改不了 `license`"。
    ///
    /// 注入（实测红）：把 `old_asset` 换成 `None`（把更新当创建）⇒ 那条断言红，
    /// 而且模型的前置条件会在 `apply` 时拒它。
    #[test]
    fn set_asset_updates_a_licence_and_inverts_byte_for_byte() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let target = project
            .assets
            .keys()
            .next()
            .expect("夹具必须有资产")
            .clone();
        let before = project.assets[&target].clone();
        let total_before = project.assets.len();
        let bytes_before = serde_json::to_string(&project).expect("序列化");

        let ops = parse_ops(&serde_json::json!([{
            "kind": SET_ASSET_KIND,
            "assetHash": target.as_str(),
            "license": "CC-BY-4.0",
            "originalPath": "samples/renamed.wav",
        }]))
        .expect("规范形状必须被接受");
        assert_eq!(ops[0].kind_name(), SET_ASSET_KIND, "kind 必须回环一致");
        assert!(ops[0].is_asset_level(), "必须落在资产级那个桶里");
        assert!(!ops[0].is_note_level(), "资产级不读不写音符");
        assert_eq!(
            ops[0],
            NoteOp::SetAsset {
                asset_hash: target.clone(),
                patch: AssetPatch {
                    license: Some("CC-BY-4.0".to_owned()),
                    original_path: Some("samples/renamed.wav".to_owned()),
                },
            }
        );

        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 1);
        match &compiled[0] {
            Op::SetAsset {
                asset_hash,
                old_asset,
                new_asset,
            } => {
                assert_eq!(asset_hash, &target);
                assert_eq!(
                    old_asset.as_ref(),
                    Some(&before),
                    "`old_asset` 必须从**当前文档读**（⛔ 非调用方声明）且是 `Some`（更新语义）"
                );
                assert_eq!(new_asset.license, "CC-BY-4.0", "许可被改了");
                assert_eq!(
                    new_asset.original_path, "samples/renamed.wav",
                    "原路径被改了"
                );
                // ⚠ **不可改**的三字段必须**逐字保留**。
                assert_eq!(new_asset.hash, before.hash, "`hash` 不可改");
                assert_eq!(new_asset.byte_len, before.byte_len, "`byte_len` 不可改");
                assert_eq!(
                    new_asset.media_kind, before.media_kind,
                    "`media_kind` 不可改"
                );
            }
            other => panic!("应当是 SetAsset: {other:?}"),
        }
        compiled[0].apply(&mut project).expect("改");
        assert_eq!(project.assets[&target].license, "CC-BY-4.0");
        assert_eq!(
            project.assets.len(),
            total_before,
            "改一条**不该**增减条目数"
        );
        assert!(project.validate().is_ok());
        compiled[0].apply_inverse(&mut project).expect("逆操作");
        assert_eq!(
            project.assets[&target].license, before.license,
            "撤销必须回原许可"
        );
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            bytes_before,
            "一步撤销必须逐字节回到原状"
        );
    }

    /// ⭐ **控制臂（第 46／47 条）＋ 第 4 点的关键读数**：`setAsset` ⛔ **不绕过**
    /// 步 ③ 的工程层许可判定 —— 改成**白名单外**的值必须**被拦**。
    ///
    /// ⚠ 这条非有不可：`setAsset` 是**新增**的一条写 `project.assets` 的通路
    /// ⇒ 若它不判许可，一个非商用许可可以经它写进去 ⇒ ⭐ **刚建的判定被绕过**。
    ///
    /// 走**产品路径**（同一个 `compile`），两臂只差 `license` 一个值；此臂**独立变色**。
    ///
    /// 注入（实测红）：删掉 `compile` 里的 `require_allowed_licence` 调用 ⇒ 本判据红
    /// （`CC-BY-NC-SA` 会被放行）。
    #[test]
    fn set_asset_does_not_bypass_the_licence_whitelist() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let target = project
            .assets
            .keys()
            .next()
            .expect("夹具必须有资产")
            .clone();
        let make = |licence: &str| {
            parse_ops(&serde_json::json!([{
                "kind": SET_ASSET_KIND,
                "assetHash": target.as_str(),
                "license": licence,
            }]))
            .expect("解析不查白名单")
        };

        // 臂 A（正对照）：白名单内的新值被接受。
        let allowed = compile(&project, &track_id, &clip_id, &make("CC-BY-4.0"))
            .expect("白名单内的新许可必须被接受");
        assert_eq!(allowed.len(), 1);

        // 臂 B（控制臂）：白名单外的**新值**必须被拦 —— 这是"不绕过"的证明。
        for bad in ["CC-BY-NC-SA", "Garbage-1.0"] {
            let fault = compile(&project, &track_id, &clip_id, &make(bad))
                .expect_err("白名单外的新许可必须被拦");
            let Fault::Domain { data, .. } = &fault else {
                panic!("应当是领域失败, 实际 {fault:?}");
            };
            assert_eq!(
                data.as_ref()
                    .and_then(|value| value.pointer("/reason"))
                    .and_then(Value::as_str),
                Some("licenseNotAllowed"),
                "`{bad}` 必须报 licenseNotAllowed（证明不绕过步 ③ 的判定）"
            );
        }
    }

    /// `setAsset` 的形状拒绝：不可改字段、非法哈希、空 patch、以及不存在的哈希。
    ///
    /// 注入（实测红）：删掉 `assetPatchIsEmpty` 那条 ⇒ 第三条断言红。
    #[test]
    fn set_asset_shapes_and_a_missing_hash_fail_loudly() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let target = project
            .assets
            .keys()
            .next()
            .expect("夹具必须有资产")
            .clone();

        // ⚠ 最像"写对了"的错法：给**不可改**的字段。
        for forbidden in ["byteLen", "mediaKind", "hash"] {
            let mut object = serde_json::json!({
                "kind": SET_ASSET_KIND,
                "assetHash": target.as_str(),
                "license": "CC0-1.0",
            });
            object
                .as_object_mut()
                .expect("对象")
                .insert(forbidden.to_owned(), serde_json::json!("x"));
            let fault = parse_ops(&serde_json::json!([object])).expect_err("不可改字段必须被拒");
            assert_eq!(
                reason_of(&fault).as_deref(),
                Some("unknownSetAssetField"),
                "`{forbidden}` 必须被点名拒绝"
            );
        }

        // 一个可改字段都不给 ⇒ 无操作 ⇒ 响亮拒绝。
        let fault = parse_ops(&serde_json::json!([{
            "kind": SET_ASSET_KIND,
            "assetHash": target.as_str(),
        }]))
        .expect_err("空 patch 必须被拒");
        assert_eq!(reason_of(&fault).as_deref(), Some("assetPatchIsEmpty"));

        // 文档里没有的哈希 ⇒ `assetNotFound`（编译期，带 reason）。
        let ghost = AssetHash::of_bytes(b"setAsset-ghost");
        assert!(!project.assets.contains_key(&ghost), "夹具前提");
        let ops = parse_ops(&serde_json::json!([{
            "kind": SET_ASSET_KIND,
            "assetHash": ghost.as_str(),
            "license": "CC0-1.0",
        }]))
        .expect("解析不查文档");
        let fault =
            compile(&project, &track_id, &clip_id, &ops).expect_err("文档里没有的哈希必须被拒");
        assert_eq!(reason_of(&fault).as_deref(), Some("assetNotFound"));
    }

    /// `setAsset` 的**字面名与支持集合**被钉住。
    #[test]
    fn set_asset_field_names_are_pinned() {
        assert_eq!(SET_ASSET_KIND, "setAsset");
        assert_eq!(SET_ASSET_LICENSE_FIELD, "license");
        assert_eq!(SET_ASSET_ORIGINAL_PATH_FIELD, "originalPath");
        assert_eq!(
            SET_ASSET_FIELDS,
            ["kind", "assetHash", "license", "originalPath"]
        );
        assert_ne!(
            SET_ASSET_KIND, REMOVE_ASSET_KIND,
            "改与取走是**相反**的两件事, 共用一个 kind 会让调用方分不开"
        );
    }
}
