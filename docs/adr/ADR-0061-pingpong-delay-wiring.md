# ADR-0061 — 乒乓延迟接线（`delay_*`）：投影结构、bpm 线程化、mix 门与实时缺口

- **状态**: **Proposed**（待人类批准；本票已按建议值实施，分支 `dsp-pingpong-wiring`，见落点）
- **决定者**: 待人类
- **关联**: `~/workspace/yeban-work/dsp-features-design-20261101.md` §3（乒乓延迟）＋ §0（共用裁决）；
  `crates/yeban-dsp/src/delay.rs`（器件，零改动）；`crates/yeban-dsp/src/device_param.rs`（共享拼写表）；
  `crates/yeban-render/src/inserts.rs`（离线投影与运行态）；`crates/yeban-engine/src/insert.rs`（引擎投影）；
  `crates/yeban-render/src/automation.rs`（自动化落点）；`crates/yeban-mcp/src/effect_params.rs`（MCP 描述）
- **落点（已实施）**: `device_param.rs`（`DelayStageParams` / `DelayField` / 换算）＋
  `inserts.rs`（`InsertChain.delay` / `InsertRuntime.delay` / `delay_source_slot` / `delay_source_params`）＋
  `insert.rs`（`InsertParams.delay`）＋ `automation.rs`（名单 55→70 拼写）＋
  `effect_params.rs`（`DELAY` 组，`EFFECT_GROUPS` 6→7）＋ 调用点（app `render_core.rs` / mcp `domain/render.rs` /
  engine `snapshot.rs` / `rt.rs` 测试辅助）＋ `tests/insert_chain.rs` 与 `tests/effect_contract.rs` 判据

## 背景

设计 §3 的路线是 **InternalEffect 参数名扩展**：不新设 `DeviceKind`，复用 `delay.rs` 的 `Delay`
器件（`DelayParams { time_s, feedback, mix, damp, ping_pong }`，`MAX_DELAY_SECONDS = 2.0`，
`process` 内部门控 time 0.001..max / feedback 0..0.95 / mix·damp 0..1 / 非有限样本归零）。
链位置是卷积之后：通道条 → Freeverb → 卷积混响 → 乒乓延迟。

器件**没有**音符同步字段（`delay.rs` 零改动是设计定值），因此投影期参数不能直接就是
`DelayParams`：同步四字段必须住在别处、换算后再进器件。

## 决定

### D1 投影结构：单设 `DelayStageParams`，器件零改动

`DelayStageParams`（`device_param.rs`，render 与 engine 的唯一共同下游）＝ 器件 5 字段 ＋
同步 4 字段（`sync_enabled` / `sync_num` / `sync_den` / `sync_dotted`）。
`delay_params_from_stage(stage, bpm)` 在投影期换算成 `DelayParams`。

舞台缺省遵设计 §3.3 表（与器件缺省**在两处不同**，⛔ 不可照抄 `DelayParams::new`）：
`time 0.3` / `feedback 0.3` / **`mix 0.0`**（级缺省关闭）/ `damp 0.0` /
**`ping_pong true`**（功能缺省即乒乓）/ `sync_enabled false` / `sync_num 3` / `sync_den 8` /
`sync_dotted true`。

### D2 bpm 线程化：构造期参数，不进 `RenderPlan`

`InsertChain::from_devices`、`project_inserts`、`project_inserts_with_forced_instances`、
`project_inserts_with_forced_strips`、引擎 `InsertParams::from_devices` 均新增 `bpm: f64`。
生产调用点都有工程可传（app `render_core.rs`、mcp `domain/render.rs`、engine `snapshot.rs`
均传 `project.bpm`；`rt.rs` 测试辅助传 120.0）。

换算式（投影期、控制线程做）：`time_s = 60/bpm × 4 × num/den × (附点 ? 1.5 : 1)`，
结果钳到器件上限 2.0 s。84 BPM 换算表：8n = 0.3571、8n. = 0.5357、4n = 0.7143、
4n. = 1.0714、2n = 1.4286、**2n. = 2.1429 → 钳 2.0**（已在判据里钉住）。

### D3 bpm 无效回落：不编造速度

`bpm` 非有限或 ≤ 0 时同步**不换算**，保留写值。`sync_num` / `sync_den` 先钳 1..16 再取整
（拍数是整数）；`sanitise` 里非有限回落到缺省 3 / 8。

### D4 mix 门与 `-0.0`：整级跳过，不乘零

建级门与处理门都用**钳制后** mix（`DelayField::Mix.sanitise(mix) != 0.0`）：
非有限 ⇒ 0 ⇒ 不建（⛔ 不用原始值：`NaN != 0.0` 为真会误建恒 0 器件）。
`mix = 0` 时级不建；强制位为真时建器件但 `process` 整级跳过。
⛔ 不能靠乘零混过去：`x + 0·w` 在 `x = -0.0` 时符号翻转，判据钉住 `-0.0` 逐位保留。

### D5 投影口径与混响/卷积一致

`delay_projection(device) -> Option<DelayStageParams>`（字段级，供来源判定与镜像判据）与
`delay_of(device, bpm) -> Option<DelayParams>`（换算后器件参数）分两层，仿卷积模式。
`mix = 0` 时投影仍返回 `Some`（运行态再按建级门丢弃，强制位为真时保留实例），与
`reverb_of` 同口径。来源规则同三件：首个含延迟名字的非旁通 `InternalEffect` 胜，
同一台设备可兼四族来源。

### D6 自动化与 MCP：同一张表，不抄第二份

自动化名单追加延迟 15 拼写（55 → **70 个拼写 = 39 个字段**），强制扫描加 Delay 族，
静态值走 `delay_source_params` ＋ `sanitise(read)`。MCP `DELAY` 组（id `delay`，9 参数及别名、
单位、范围、缺省与中文 doc），`EFFECT_GROUPS` 6 → 7。
⛔ 引擎不抄第二份拼写表：引擎 `delay_of` 直接调 `DelayField::from_name`；
mcp `effect_contract` 判据 ② 因此同时扫 `insert.rs` 与共享拼写表（白名单住在两处）。

### D7 块边界同步字段：显式空操作（已登记的口径，不是静默缺失）

块边界没有 bpm 可用，`RenderPlan` 亦不携带 bpm；把 bpm 线程化进 `RenderPlan` 属超范围改动。
故 `apply_device_params` 的 Delay 分支只写 5 个器件字段进运行参数，同步四字段在块边界是
**显式空操作**（同步泳道仍报 applied、仍强制建器件）。设计 §3.7 的测试要求也只是
"自动化泳道 applied"，不是块边界同步重定时。

## 已知缺口（待人类裁决，⛔ 本票不私自发明）

### G1 实时运行态未接线

`rt.rs` 的器件池、`render_branch` 阶段与 snapshot armed 表**不在本票范围**：
设计附录落点表未列 `rt.rs`，且实时侧是单声道 scratch ＋ 取中值口径，乒乓回声在单声道
的语义属未决规范缺口。实时投影（`InsertParams.delay`）照做，但 `rt.rs` 不消费它 ——
这是**已登记的差异**，不是静默缺失。待裁决：单声道下乒乓回声的落点语义（取中值？
只走左线？实时侧另立一票？）。

### G2 192 kHz 下的器件行上限

`DelayField::TimeS.sanitise` 按文档常量 `MAX_DELAY_SECONDS = 2.0` 钳制；192 kHz 下
器件行上限会使实际 `max_seconds` 小于 2.0。此为器件级细节，恒等判定用文档常量，
不另展开。

## 后果

- 旧工程逐位不变：无延迟名字的工程投影不变；`process` 重构后旧链路径操作顺序与值不变
  （卷积后多一次平面↔交错拷贝，纯复制不改值）。
- 新工程：单写 `delay_time_s` 的设备投影为 `Some` 但运行态丢弃（除非强制），与设计 §3.2 一致；
  单写 `delay_mix` 即武装延迟（mix ≠ 0 时建器件）。
- 批 G1 后实时侧才能真正出声；批之前，延迟只在离线渲染生效，实时监听无延迟 ——
  该差异须在发布说明里写明，⛔ 不可只写"已接线"。
