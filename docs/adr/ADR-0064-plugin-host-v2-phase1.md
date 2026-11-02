# ADR-0064 — 插件宿主 v2 一期：进程内 VST3 乐器接入（离线渲染）

- **状态**: **Proposed**（待人类批准；本票代码已按本 ADR 实施，裁决点是下面“决定”一节的口径）
- **决定者**: 待人类
- **关联**: 架构规范 §9（`ARCH-PLUG-001`，v2.0.0 跨进程崩溃隔离宿主是远期形态）；
  Wang 2026-11-02 决定（插件宿主部分提前开工：乐器接入优先、VST3 优先；跨进程隔离＋shm＋看门狗、
  RT 回调插件处理、效果器链接入、AU 宿主均不在本期）；`ADR-0011`（快照投影）；
  `ADR-0063`（行为变更声明先例）；Logic Pro 的 `auval` 准入（扫描与验证类比）。
- **落点（本票）**: `docs/adr/ADR-0064-plugin-host-v2-phase1.md`（本文件）＋
  `docs/adr/README.md`（索引行）＋
  `crates/yeban-plugin-host/src/lib.rs`（范围文档重写）＋
  `crates/yeban-plugin-host/src/error.rs`（`HostError`）＋
  `crates/yeban-plugin-host/src/types.rs`（`PluginInstance` trait、事件、参数、状态容器）＋
  `crates/yeban-plugin-host/src/dylib.rs`（动态库装载）＋
  `crates/yeban-plugin-host/src/vst3.rs`（VST3 COM 宿主）＋
  `crates/yeban-plugin-host/src/stub.rs`（进程内测试桩乐器）＋
  `crates/yeban-plugin-host/src/host.rs`（扫描／加载）＋
  `crates/yeban-plugin-host/tests/vst3_loader.rs`（现场编译的最小 VST3 测试插件夹具）＋
  `crates/yeban-render/src/plugin_source.rs`（`PluginTrackSource`）＋
  `crates/yeban-render/Cargo.toml`（`yeban-plugin-host` 路径依赖）＋
  `crates/yeban-model/src/project.rs`（`DeviceDefinition.plugin` 绑定字段）＋
  `crates/yeban-mcp/src/tools.rs`（`yeban_scan_plugins`／`yeban_load_plugin`）＋
  `crates/yeban-mcp/src/domain/mod.rs`（分发臂＋两个 `plan_*`）＋
  `crates/yeban-mcp/src/domain/render.rs`（宿主乐器解析＋`plugin-instrument` 命名＋
  `externalPlugins` 排除）＋
  `crates/yeban-mcp/Cargo.toml`（`yeban-plugin-host` 路径依赖）＋
  `schemas/mcp-tools.schema.json`（名册＋`allOf`＋两个 `$defs`）＋
  `crates/yeban-mcp/tests/param_bounds_guard.rs`（新字符串参数的界登记）

## 背景（先测后改，`AGENTS.md` §6）

现读（2026-11-02，`main` 613fb511）：

| # | 事实 | 出处（符号级） |
| :--- | :--- | :--- |
| 1 | `yeban-plugin-host` 只有 12 行占位骨架，依赖为空，没有任何 crate 依赖它；`yeban-vst` 是 11 行反向（输出方向）占位，与本任务无关 | `crates/yeban-plugin-host/src/lib.rs`、`crates/yeban-vst/src/lib.rs` |
| 2 | 模型有 `ExternalInstrument`／`ExternalEffect`，但引擎与离线渲染都不消费它们，只报 `externalPlugins` 警告；`KNOWN_UNSUPPORTED_KEYS` 现读 10 键 | `crates/yeban-mcp/src/domain/render.rs`（`:4558` 条件、`:687` 名册） |
| 3 | `DeviceDefinition.params` 是 **f32 数值对**（`name`／`value: f32`／`unit`），路径与类名**放不进**参数 ⇒ 插件绑定必须进模型字段，⛔ 不能复用“参数约定” | `crates/yeban-model/src/project.rs:452`（`ParameterValue`） |
| 4 | `ScheduledNote` 已有 `start_sample`／`end_sample`／`pitch`／`velocity` 的 getter ⇒ 插件源读调度表**不需要改引擎** | `crates/yeban-engine/src/synth.rs`（`ScheduledNote` 的 `impl`） |
| 5 | `EngineTrackSource` 的契约可直接复用：单轨武装、一台引擎一个源、播放头 `first_frame` 校验、每块恰好推进一次、声相（居中 `(1.0, 1.0)` 铺到所有声道，否则左进 0 声道、右进 1 声道） | `crates/yeban-render/src/engine_source.rs`（`render_block`） |
| 6 | VST3 COM 常量已对 Steinberg `vst3_pluginterfaces`（MIT）原文核对：`IComponent`（`0xE831FF31…`）、`IAudioProcessor`（`0x42043F99…`）、`IEditController`（`0xDCD7BBE3…`）三枚 IID 与三张虚表顺序；TUID 字节序＝4 个 u32 大端串联（JUCE 的换算代码同口径） | `vst/ivstcomponent.h`、`vst/ivstaudioprocessor.h`、`vst/ivsteditcontroller.h` |
| 7 | MCP 工具数现读 **26**（规范 10＋扩展 16）；本票 ＋2 ⇒ **28**／扩展 **18**。⚠ SVS 支也走到 28 但工具不同源，合入时注意 | `crates/yeban-mcp/src/tools.rs`（`TOOL_COUNT`／`EXTENSION_TOOL_COUNT`） |
| 8 | VST3 **没有通用 MIDI CC 输入事件**：CC 必须经 `IMidiMapping` 映射成参数（SDK 原文：“MIDI controllers are not transmitted directly to a VST component”）⇒ 一期不做 CC 映射 | `vst/ivsteditcontroller.h`（`IMidiMapping` 文档） |
| 9 | 进程内宿主崩溃会拖垮宿主进程（与 Logic 在 Intel 时代同款风险）；一期以加载前验证＋失败隔离降概率，⛔ 不消除 | 本票的已知缺口（见 D2） |
| 10 | 真实商业插件兼容性**未经验证**：本仓无 Kontakt 可测；测试插件与宿主同作者 ⇒ 自洽测试不能证明对真实插件的 ABI 正确 | 本票的已知缺口（见 D17） |

## 决定（一期口径）

| # | 决定 | 理由／边界 |
| :--- | :--- | :--- |
| D1 | 一期＝**进程内宿主＋离线渲染链路＋乐器接入＋扫描／验证／加载＋参数＋状态往返**；VST3 先行，CLAP 随后（CLAP 宿主是后续票） | 先打通“轨 MIDI → 宿主乐器 → 音频”全链路与音质验证，再谈隔离与第二格式 |
| D2 | **进程内**，不做跨进程隔离＋shm＋看门狗＋热重启（那是 phase 2） | 隔离是独立工程量；一期先证明链路本身是对的。代价见背景 9 |
| D3 | **零新增第三方依赖**：VST3 加载用 std＋手写 FFI（`dlopen`／`LoadLibrary`＋COM 虚表定义），不用 `vst3-sys`／`clack` | 无 C++ 工具链、无新依赖评审、一期表面小。代价：COM ABI 由本仓手写定义负责（背景 6 已核对原文；背景 10 是已知缺口） |
| D4 | 绑定**进模型**：`DeviceDefinition` 新增 `plugin: Option<PluginBinding>`（`path`＋可选 `class`；缺省 `None` ⇒ 旧文档逐字可读）。只有 `ExternalInstrument`／`ExternalEffect` 可带绑定，其余 kind 带绑定 ⇒ 模型校验拒绝。`path` 约定：`.vst3` 包路径／动态库路径／`stub:` 测试桩（仅测试夹具）。`device.name`＝显示名（插件名），`latency_samples`＝装载时实例上报值 | 背景 3 排除了参数载体；绑定必须随工程落盘，渲染才能只凭工程解析（会话注册表在进程重启／换机器后丢绑定，⛔ 不能当事实源） |
| D5 | 音源优先级：非旁通 `ExternalInstrument`（带绑定且加载成功）⇒ 该轨音源，**胜过**引擎合成器与采样器绑定 | 外部设备是显式音源选择；采样器轨＋外部设备并存时外部胜出（⛔ 不叠两份声音） |
| D6 | 失败口径：带绑定但加载／实例化失败 ⇒ 渲染构造期 **`RENDER_FAILED` 响亮失败**，⛔ 不静默、不出声；无绑定的外部设备 ⇒ 保留旧口径（`externalPlugins` 警告＋静音） | 显式指向坏插件必须响亮；未指定路径的沿用旧口径，不破坏旧工程 |
| D7 | MIDI：只发音符 on／off 事件（帧精确，`noteId`＝调度下标；同块同偏移先 off 后 on）。CC／弯音／滑音一期**不生效**（背景 8），文档明示 | VST3 事件模型里 CC 不是事件；映射是后续票 |
| D8 | 参数：一期用构造期＋渲染前直接 `setParamNormalized`；逐点参数队列（sample-accurate 自动化）后续 | 直接设值覆盖“默认音色／增益”一期用例；逐点队列是自动化票 |
| D9 | 状态：自有容器（magic＋版本＋处理器 blob＋控制器 blob）；保存＝两段 `getState`，恢复＝处理器 `setState` → 控制器 `setComponentState`（处理器态，同步参数）→ 控制器 `setState`（控制器态）；控制器态为空时跳过控制器 `setState`（个别控制器对空流返回失败）。**一期往返在进程内证明**；状态 blob 进工程文件（CAS 资产）＝phase 2 | 容器格式现在定死，落盘位置后定：blob 是不透明字节，值得 CAS 设计而不是塞进工程 JSON |
| D10 | 声相与音量：插件立体声输出 × `schedule.track_gain()`（静态推子，与引擎路径同一口径）→ 声相增益逐声道；居中 `(1.0, 1.0)` 逐位不变；单声道导出 `(L+R)*0.5`（声相无意义）；＞2 声道 L／R 进前两声道、其余 0 | 与引擎源同序（增益在声相之前）；节点级 `TrackGainAutomation` 仍在源输出之外另乘 |
| D11 | 延迟：实例 `getLatencySamples` 上报给实例；PDC 集成＝phase 2；`device.latency_samples`＝装载时上报值（声明值） | 上报与补偿分两步；一期只做到“报得出” |
| D12 | 扫描与验证（Logic `auval` 类比）：发现候选（`.vst3` 包内平台动态库＋散动态库）＋**逐个尝试加载与枚举类**；失败逐个记原因、⛔ 不中断扫描、⛔ 不进可用列表；列表含失败项＋原因（Plug-in Manager Compatibility 列类比）。路径：`YEBAN_VST3_PATHS` 环境变量＋平台标准路径 | 加载前验证是失败隔离的唯一手段；扫描是显式动作，⛔ 不在每次渲染时重扫 |
| D13 | MCP：两个扩展工具（⛔ 不新增工具族）。`yeban_scan_plugins`（只读枚举，`Plan::Query`）；`yeban_load_plugin`（验证加载后经**提案路径**挂设备：缺省建提案，`autoMerge: true` 当场落 main，与 `yeban_edit_notes` 同一条提案纪律，⛔ 不另设直写路） | 提案纪律是本仓写工程的唯一正门（`ADR-0057`）；直写会绕过撤销与模拟校验 |
| D14 | RT 不在本期：`rt.rs` 不动；实时监听仍无插件声（文档明示，与插入链实时单声道下混同类已知缺口） | RT 回调里跑第三方插件要另做零分配／零锁证明 |
| D15 | 效果器链（`ExternalEffect`）不在本期，等 slotlists 折中合入后再接（链模型先定） | 链模型未定前接效果器会接错槽位 |
| D16 | AU 不在范围（夜半无 AU） | —— |
| D17 | 测试夹具：进程内 stub 乐器（`PluginInstance` trait 实现）＋`rustc` 现场编译的最小 VST3 测试插件（纯 std `cdylib`，布局定义独立手写、⛔ 不 import 宿主代码）。⛔ 不依赖用户机器上的商业插件。真实商业插件兼容性未经验证（背景 10），phase 2 前需真插件实测 | 同作者的自洽测试只能证明“宿主与夹具对同一份 ABI 理解一致”，不能证明对真实插件正确 |
| D18 | 依赖与 deny：零新增第三方依赖 ⇒ 锁文件第三方集合不变 ⇒ deny 表面不变（deny 本机未装；以锁文件不变为凭据，⛔ 不为此单装 deny） |  brief 的“先跑 deny”针对的是**引入**新依赖；未引入时锁文件就是证据 |
| D19 | 三路位一致：插件源不进引擎的三路比对（引擎合成器路径无插件）；无插件工程走旧分支**逐字同**（插件轨是新增分支） | 既有位一致测试的保护面不变 |

## 行为变更声明（必须显式）

1. **以前静音＋警告的外部乐器轨接通后真的出声**：带绑定且加载成功的 `ExternalInstrument` 轨，导出字节会变（仅这类轨；其余轨逐位不变）。⛔ 不是修复回归，是 D5 的既定行为变化。
2. **模型字段新增**：`DeviceDefinition.plugin` 缺省 `None`，旧文档逐字可读（向前兼容）；带绑定的新文档被旧版工具读到时绑定不可见（向后不兼容，写明）。
3. **工具数 26 → 28**（扩展 16 → 18）；`KNOWN_UNSUPPORTED_KEYS` 的 `externalPlugins` 口径收窄（已宿主的乐器不再登记）。

## 待办（phase 2，⛔ 不在本票）

- 跨进程隔离＋shm 桥＋看门狗＋热重启（架构规范 §9 的远期形态）
- CLAP 宿主；RT 回调里的插件处理；`ExternalEffect` 效果器链接入（等 slotlists 折中）
- PDC 集成；CC 经 `IMidiMapping`；逐点参数队列；状态 blob 进工程文件（CAS 资产）
- 真实商业插件（Kontakt）实测；`moduleinfo.json` 解析（扫描优化，非验证必需）
- 扫描缓存（现每次扫描都逐个加载验证）
