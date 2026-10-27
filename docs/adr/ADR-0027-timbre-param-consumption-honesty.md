# ADR-0027 — 音色参数：撤掉 `deviceChainDsp` 的谎，注册口径改为"引擎是否真的消费"

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: 审计 `G5`（`/tmp/yeban-issues-20261027/GAPS.md`）；spec `MCP-TOOL-006`
  （`yeban_edit_notes`）；`ARCH-DET-001`（确定性读数）；
  `crates/yeban-engine/src/synth.rs`（`CONSUMED_INSTRUMENT_PARAMS` /
  `is_consumed_instrument_param`）、`crates/yeban-engine/src/drums.rs`
  （`consumes_drum_param`）、`crates/yeban-render/src/inserts.rs`
  （`track_has_unhandled_dsp`）、`crates/yeban-mcp/src/domain/render.rs`
  （`deviceChainDsp` 注册点）、`crates/yeban-mcp/src/tools.rs`（工具摘要）
- **日期**: 2026-10-27
- **编号说明**: `ADR-0025` / `ADR-0026` 已被并行工作线 `routing-batch-ordering` 与
  `generation-gain-staging` 占用（同名文件在各自分支上）；本条按下一个空号取名。

## 背景

审计 `G5` 的原始诉求是"工具面选不了音色"。实测（origin/main `36843ca9`）证明**引擎
早已消费**乐器参数：自 `#268` 起 `EngineSnapshot::from_project` → `ToneParams::from_devices`
（`synth.rs:510` 一带）逐条读 `cutoff_hz`/`cutoff`、`resonance`/`res`、`drive`、
`osc1_wave`/`osc2_wave`（值为 `ENGINE_WAVE_RECIPES` 下标：0=hollow / 1=organ / 2=vocal /
3=metallic / 4=glass）、`osc1_level`/`osc2_level`、`osc1_detune_cents`/`osc2_detune_cents`；
鼓组支路另有 5 个 `*_note` 与 kit 旋钮（`DrumKitParams::from_devices`）。
`yeban_edit_notes` 的 `insertDevice` 也**接受** `kind:"InternalInstrument"`。
真正的缺口只有一个：**回执在撒谎**。

`yeban_render_master` 对带设备链的轨一律注册 `unsupported.deviceChainDsp`
（"这条轨上的器件效果离线渲染**没有**施加"）。但 `track_has_unhandled_dsp` 的旧规则把
`InternalInstrument` 的参数也按"效果器参数白名单"衡量——乐器参数根本不在那个语义域里，
于是**引擎明明逐样本在用**的 `osc1_wave`，回执却说"未处理"。调用方据此会得出
"在工具面调波形是白费的"这一**错误**结论，恰恰把唯一存在的创作缺口（发现性）放大成
"能力不存在"。

## 裁决

1. **D1 — 注册口径 = 引擎是否真的消费，不再按 DeviceKind 粗判。**
   `track_has_unhandled_dsp` 改为逐 kind 判定：
   - `InternalEffect`：旧规则原样（空参数、或任一名字不在离线效果白名单 ⇒ 未处理）；
   - `InternalInstrument`：任一参数名（小写后）不落在
     `yeban_engine::synth::is_consumed_instrument_param` ⇒ 未处理；**空参数 = 已处理**
     （默认音色是引擎真的给出的东西）；
   - `ExternalInstrument` / `ExternalEffect`：照旧**一律**未处理（离线渲染没有插件宿主）。
2. **D2 — ⛔ 不手抄第二张表。** `is_consumed_instrument_param` 的旋律支路读
   `CONSUMED_INSTRUMENT_PARAMS`（`ToneParams::from_devices` 实际读取的名字清单），
   鼓支路**复用** `note_slot` / `apply_kit_knob` 这两个 `from_devices` 内部自己的
   查找函数作为探针（`consumes_drum_param`）。防漂移由测试承担：
   `every_consumed_instrument_param_reaches_the_engine_projection` 对每个登记名字构造
   "基线设备 + 该名字探测值"，断言引擎投影**确实**随探测值改变——名字登记了而引擎不读 ⇒ 红。
3. **D3 — 采样器绑定的轨维持诚实例外。** 轨绑了 SFZ 采样器时**发声源是采样**，
   乐器音色参数**不**被求值；此时若该轨有未旁通的 `InternalInstrument` 且参数非空，
   `yeban_render_master` **仍**注册 `deviceChainDsp`（`sampler_drops_instrument_params`
   分支）。没有这条，"绑了采样器还调 cutoff"会重演本单起因的镜像谎言。
4. **D4 — 发现性进工具摘要。** `yeban_edit_notes` 的 summary 补 G5 段：波形下标表、
   全部引擎消费名、鼓组键名、"非采样器轨的音色编辑不再报 `deviceChainDsp`"。
   `schemas/mcp-tools.schema.json` 不镜像 summary 文本（实测该 schema 无 summary 副本），
   故无 schema 改动。

## 代价与边界

- 旧回执里"报了 `deviceChainDsp`"的一部分场景（纯合成器轨 + 引擎已消费参数）现在**不报**
  ⇒ 按"必须出现该键"写断言的外部调用方会看到消失。仓库内核验：受影响断言全部在
  `tests/render_master.rs` / `tests/insert_chain.rs`，随本单同批改正。
- `is_source()`（决定"这轨有没有可发声内容"）口径**不变**：`resonance`/`drive` 单独
  存在不构成源——本单只动"处理/未处理"的注册口径，不动"有/无声"的判定。
- 音色**预设**（一键换 whole timbre）仍不在本单：引擎消费的是逐参数值，
  预设属 `G8` 一类的生成控制粒度问题。
- 听感层面"哪个波形好听"属人类事项；本单判据只到"波形改变 ⇒ 母带 `masterDigest` 改变"。

## 判据（测试）

- `crates/yeban-engine`：`every_consumed_instrument_param_reaches_the_engine_projection`
  —— 登记名逐条对引擎投影生效；`definitely_not_a_real_knob` 逐条**不**生效（反向防漂移）。
- `crates/yeban-render/tests/insert_chain.rs`：`osc1_wave`+`osc2_wave` ⇒ 已处理；
  `kick_note`+`kick_tune_hz` ⇒ 已处理；空参数乐器 ⇒ 已处理；`osc1_wave`+`mystery` ⇒
  未处理；低增益效果器 ⇒ 未处理（旧规则原样）。
- `crates/yeban-mcp/tests/render_master.rs`:
  `an_instrument_wave_choice_changes_the_master_and_stops_registering_device_chain_dsp`
  —— 同一段演奏 `osc1_wave` 0(hollow) vs 4(glass) 的 `masterDigest` **不同**（工具面
  经 `insertDevice` + `setParam` 写参数真的改变母带），且两次回执都**不含**
  `deviceChainDsp`；掺入未知名字 `mystery` 后**必须**重新注册。
