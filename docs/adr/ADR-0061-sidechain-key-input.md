# ADR-0061 — 侧链键输入消费面（v1）：键信号如何进压缩器检波器

- **状态**: **Proposed**（待人类批准；本票代码已按设计文档实施，裁决点是下面"决定"一节的 v1 口径）
- **决定者**: 待人类
- **关联**: 设计文档 `~/workspace/yeban-work/dsp-features-design-20261101.md` §2（侧链）＋ §0 共用裁决；
  `#315`（侧链读数）／`#337`（侧链边不再当普通音频边求和）；`#303`／`#309`（压缩器武装口径与 pre-pan tap）；
  `ADR-0011`（快照投影）／架构规范 §8（引擎 crate 不进 GUI 依赖）
- **落点（本票）**: `crates/yeban-dsp/src/compressor.rs`（`process_gain_with_key`）＋
  `crates/yeban-dsp/src/channel_strip.rs`（`render_frame_with_key`／`process_stereo_with_key`／`process_mono_with_key`）＋
  `crates/yeban-render/src/render.rs`（`RenderPlan::attach_key_armed_nodes`／`key_evaluated_edge_ids`）＋
  `crates/yeban-render/src/inserts.rs`（`InsertRuntime::process_with_key`）＋
  `crates/yeban-engine/src/routing.rs`（`RoutingPlan::key_routes`／`key_sources`）＋
  `crates/yeban-engine/src/rt.rs`（键抽头池＋`process_insert_chain` 的 `key_slot`）＋
  `crates/yeban-mcp/src/domain/render.rs`（收据 `sidechainEdgesDeclaredWithoutKeyInput` 的求值后口径）＋
  `crates/yeban-app/src/render_core.rs`（同一挂接点）

## 背景（先测后改，`AGENTS.md` §6）

**这个数字在数什么**：单位＝**条**，对象＝路由图里 `kind == Sidechain` 的边；
单位＝**样本**，对象＝键信号与节目信号的对齐帧。

现读（2026-11-01，`main` b0d19e8b）：

| # | 事实 | 出处（符号级） |
| :--- | :--- | :--- |
| 1 | 侧链边**不搬音频**（`#337`）：`RenderPlan` 不把它放进 `incoming`，求和读数恒 `0`；但**也没有键输入路径**：压缩器检波器只读自己正在压缩的那一路，收据 `sidechainEdgesDeclaredWithoutKeyInput` 恒等于声明条数 | `yeban-render/src/render.rs`（`edge_ids()` 不含 `Sidechain`）、`yeban-mcp/src/domain/render.rs`（旧等式） |
| 2 | 压缩器检波链（电平跟随 → 阈值／比率／拐点 → 起音／释放平滑 → 增益）与节目信号耦合在 `process_gain` 一个方法里，换检波输入需要拆步骤 | `yeban-dsp/src/compressor.rs`（旧 `process_gain`） |
| 3 | 压缩器"已武装"的口径已有两处现成实现，⛔ 不许手抄第三份：静态投影 ∪ 非恒等 `compressor_enabled` 泳道 | `yeban-render/src/automation.rs`（`compressing_nodes_with_lanes`）、`yeban-render/src/inserts.rs`（`statically_compressing_nodes`） |
| 4 | 离线渲染按层分段执行（同层节点并行），实时引擎按实体序逐轨循环＋转发阶段跑处理型总线链 —— 两条执行模型对"源什么时候可用"的回答不同，因此键路由规则必须分两套写 | `yeban-render/src/render.rs`（`layers`）、`yeban-engine/src/rt.rs`（`render_branch`） |

## 决定（v1 口径）

| # | 决定 | 理由／边界 |
| :--- | :--- | :--- |
| D1 | 检波输入换键信号，其余与 `process_gain` **逐字同**：`process_gain` 的步骤 ②–⑥原样抽成 `advance()`，`process_gain_with_key` 只换步骤 ① 的检波输入来源；键信号**不进混音**，返回值仍由调用方乘到节目上 | 键＝节目时必须与节目模式**逐位相同**（判据钉死）；键只改增益，不改音色与电平链 |
| D2 | 启用规则**隐式**：边 ＋ 已武装压缩器 ⇒ 键模式；压缩器未武装 ⇒ 边保持声明未求值。零新参数、零新边种、零新模型字段 | 启用状态由压缩器自身状态决定，不另设开关；未武装时边是纯数据，渲染逐字节不变 |
| D3 | 键抽头点 ＝ 源节点**插入链之后、声相之前、推子之前** | 与 `#303` 的 pre-pan tap 同口径：检波器读链后信号，不受声相与推子污染 |
| D4 | 离线：`attach_key_armed_nodes`（构造期调用，执行中不调用）。一条边被求值当且仅当：目的已武装、源在保留子图里、源层号**严格小于**目的层号、且是该目的边身份升序第一条合格边 | 同层源因并行不可用；自边与环边因此至少一个方向落选，**环永远不能完全求值**；v1 单键：每目的至多一条 |
| D5 | 实时：`RoutingPlan::compile` 在编译期算键路由（构造期分配，逐块零分配）。源必须**确定**在逐轨循环里跑链：无入边或零器件的源可求值；有入边且有器件的源可能在转发阶段跑链 ⇒ 排除（离线渲染覆盖这类源）。轨目的要求源实体序 ＜ 目的；master 目的不受序限；每目的首条合格边胜出，不合格边不占位 | 编译期没有武装信息，规则必须在"不知道武装"时恒正确；总线源是 v1 的已知缺口（见待办） |
| D6 | 抽头只在**循环里跑链**的轨写：处理型总线的链在转发阶段才跑，此时 `track_scratch` 还只是自身声源 ⇒ v1 不抽总线源。源轨因电平容量被跳过时，目的**优雅降级**为节目检波（音频正确，只是不压） | 不抽错信号比"尽量压"重要；降级是可观测的（增益恒 1 的那几块），不是静默错误 |
| D7 | 回执**不新增字段**：`sidechainEdgesDeclaredWithoutKeyInput` ＝ 声明条数 − 已求值条数。求值后归零是"全部求值"的读数，不是恒等于零的承诺 | `#337` 的旧等式（恒等于声明条数）失效，字段文档与 `outputSchema` 描述同步更新 |
| D8 | 音频表过滤与音频边计数**不动**：侧链边仍不进 `incoming`、不进 `edge_ids()`、不参与可达性／PDC／分层 | 键路由是检波器的第二输入，不是音频通路；`#337` 的修补面不回退 |

## 行为变更声明（⛔ 不是修复回归，必须写进发布说明）

有边且压缩器已武装的旧工程，**以前没压、接通后真的压**，导出字节会变。这是本票**有意**的行为变更：

- 以前：侧链边是纯声明，压缩器永远读节目 ⇒ 母带里没有 ducking。
- 接通后：同一工程的同一压缩器读键 ⇒ 增益包络变化 ⇒ 母带样本变化（变化量由键电平与阈值／比率决定，不是数值噪声）。
- 不受影响的工程：没有侧链边的工程（一个计划槽都不改，产物逐字节不变）；目的未武装的工程（边仍登记未求值）；源被剪枝／层序不满足／环边的工程（边仍登记未求值）。

## 推子不对称（tap 定义的精确口径，审阅时必读）

抽头取"插入链之后即刻"的值，两条执行路径对推子的处理**不对称**，是结构使然不是 bug：

- 离线：tap 写在输出级（推子／声相）之前 ⇒ **排除**总线推子。
- 实时：参数增益按 `#param` §2.2 设计本来就在链前 ⇒ **包含**在 tap 信号里（它是链前增益，不是输出级推子）。
- 声相：两边都排除（tap 在声相之前；实时 tap 取单声道 `track_scratch`，声相在汇流时才应用）。

## 待办（v2，不在本票）

- 实时总线源：转发阶段跑链的总线做键源（需转发阶段抽头或两阶段循环）。
- 多键：同一目的多条边同时生效（键混合口径未定：求和／取最大／加权）。
- 键滤波／键混合参数（零新参数是 v1 的约束，v2 需另立参数提案）。
- 泳道中途开关压缩器时：路由求值（构造期快照）与逐帧启用（条按帧自查）是两层语义 —— 路由已求值但某帧压缩器关 ⇒ 该帧不压；路由未求值但某帧压缩器开 ⇒ 该帧仍走节目检波（路由不重算）。文档须写明这一层区别。

## 后果

- 批准前，本票的 v1 口径是"**已实施、待批的提议**"：代码与判据先行，发布说明里的行为变更声明是批准的附带条件。
- 判据：`yeban-dsp` 键＝节目逐位相同／键响压轻节目／键静不压响节目／未启用时键被忽略；
  `yeban-render` 路由三条件＋单调重挂＋首边胜出＋源不在子图＋端到端 ducking；
  `yeban-engine` 循环相位源／实体序／总线源排除／首边胜出／master 目的＋端到端 ducking＋惰性边对照；
  `yeban-mcp` `#337` 旧断言（目的无武装时读数与字节全不变）继续绿。
- `EngineRuntime` 值尺寸 ＋32 B（指针 8 ＋ 填充标记 16 ＋ 对齐填充 8；堆上抽头池 8192 B 不计入），
  仍在 192 KiB 门槛内；`rt_new_boxed.rs` 的钉值同步为 186768 B。
