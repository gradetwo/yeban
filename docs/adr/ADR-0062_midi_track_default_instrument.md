# ADR-0062 — 建 MIDI 轨即挂一台内置乐器（缺口 D-UI-4：只用鼠标/键盘的用户做不出能发声的工程）

- **状态**: **Proposed（待人类批准；⛔ Agent 不代签）**
  —— 本票不是零代码票：实现已落在工作线 `line/zimmer-ui-write-surface`（未合流）。
  理由是「模拟人类 UI 模式创作一首 3 分钟作品」这一半目标在 A 之下**物理上听不见**
  （见"为什么现在必须定"）。若人类否决 B，撤回的是 `bridge.rs` 里那一条
  `Op::InsertDevice` ＋三个新符号 ＋两条判据；模型/引擎侧一位未动，无需迁移。
- **决定者**: 待人类裁决
- **关联**: 缺口 `D-UI-4`（本线创作验收实测登记，见 `docs/DEVELOPMENT_LEDGER.md` 的 zimmer 段）；
  `crates/yeban-app/src/bridge.rs`（`track_create_ops` / `builtin_instrument_device` /
  `BUILTIN_INSTRUMENT_PARAMS`）、`crates/yeban-app/tests/track_routing_crud.rs`（判据 ① 与新增 ①b）、
  `crates/yeban-engine/src/synth.rs`（`CONSUMED_INSTRUMENT_PARAMS`、`is_consumed_instrument_param`、
  `DeviceTone::is_source`、`ToneParams` 文档里那份九键投影示例）、
  `crates/yeban-model/src/project.rs`（`DeviceKind::InternalInstrument`）、
  `crates/yeban-mcp/src/effect_params.rs`（`cutoff_hz` 的单位 `Hz`，自 `yeban-app` 以 `#[path]` 共享）
- **日期**: 2026-11-01

## 背景（全部现读，⛔ 不采信报告措辞）

1. **建轨批次里没有音源**：UI 的"新建 MIDI 轨"落到 `bridge::track_create_ops`，
   此前返回 `[AddTrack, AddRoutingNode, ConnectRouting]`，而轨体是 `TrackV3::default()`
   ⇒ `devices` 是**空 `Vec`**。路由边是建好的（轨确实连到主总线），所以电平表、
   静音/独奏、推子全都"看起来在工作"。
2. **引擎对空设备链的处置是静音，不是报错**：`yeban-engine/src/synth.rs` 的
   `DeviceTone` 只在设备链里存在**音源**时才建合成器；`is_source()` 认定音源的条件是
   `cutoff_hz` 或六个振荡器键（`osc1_wave`/`osc2_wave`/`osc1_level`/`osc1_detune_cents`/
   `osc2_level`/`osc2_detune_cents`）**至少一个**。一台只有 `resonance`/`drive` 的设备
   ⇒ `Self::bypass()` ⇒ **工程改了、音符画满了，依然一点声音都没有**。
3. **界面里唯一的"装乐器"入口在仓内是空的**：控制台设备架的乐器按钮
   `instrument-install-{i}-button` 的清单来自 `library_instruments()` /
   `installable_builtin_instruments()`，两者按**样本字节是否存在**过滤
   （`BUILTIN_INSTRUMENTS` 是两条 SFZ 条目）⇒ 仓内不提交大样本 ⇒ 清单为空 ⇒
   **0 个按钮**，原因串是 `INSTRUMENT_INSTALL_NO_BUILTIN_REASON`。
   实测读数（活窗口，Tier-2 控制面）：按钮数 0。
4. **"装 EQ / 装混响"不解决**：`device-insert-eq-button` / `device-insert-reverb-button`
   只会插入 `DeviceKind::InternalEffect`。效果器不是音源 —— 在一台哑轨上装 EQ，
   听感从"静音"变成"被均衡过的静音"。
5. **领域面有这条路，UI 面没有**：MCP 模式的第一首作品（`yeban_apply_ops` 直插
   `Op::InsertDevice{kind: InternalInstrument}` 一台带九键的音源）全程可听；
   演示工程 `demo_project()` 的轨也自带 `InternalInstrument`
   （`crates/yeban-model/src/samples.rs`）。⇒ 差别不是"音色好不好"，而是
   **鼠标/键盘这条路根本没有装音源的入口**。
6. **这不是"缺一个音色库"**：本票不引入任何样本、不引入新 `DeviceKind`、不碰 DSP。
   它只回答一个问题：**新建的 MIDI 轨默认挂不挂一台能直接发声的内置乐器**。

## 为什么现在必须定（D-UI-4 是硬阻塞，不是体验优化）

目标之一是**只用模拟人类的 UI 手势**（点击 / 拖拽 / 组合键，⛔ 不调任何领域工具）
做出第二首 3 分钟作品，并**用测量证明它真的出声**。在 A 之下：

- 建轨 → 画片段 → 画音符 → 混音 → 导出，每一步的控制面回执都会是 `applied`、
  模型读数都会跟着变，**但导出 WAV 是静音**；
- 唯一绕过方式是中途改用领域工具装音源 —— 那就不再是"UI 模式"，这一半目标作废；
- 把轨道换成"打开演示工程再改"也不行：那首歌的轨不是用户建的，等于换了题目。

⇒ 要么定下默认音源，要么承认"UI 模式做不出能听的作品"这一条能力缺口写进报告。

## 三个选项（⛔ Agent 不代选）

- **A 维持现状**：新建 MIDI 轨设备链为空，装音源留给"将来的乐器库 / 将来的 SFZ 素材"。
  成本：UI 创作路径不通（见上）；优点：零行为变化，既有夹具工程一个字节都不动。
- **B 建轨即挂一台内置乐器（本票实现所采）**：`track_create_ops` 对 `NewTrackKind::Midi`
  在 `AddTrack` 之后**追加一条** `Op::InsertDevice{slot_index: 0, kind: InternalInstrument}`，
  参数取自引擎文档里那份九键投影示例、键名逐字对账 `is_consumed_instrument_param`。
  理由：① 这是**同一条 Op 批次**里的第 5 条 ⇒ 一次点击 = 一次 `commit_ops` ⇒ Undo/Redo
  仍是一步，撤销即"轨不存在"，⛔ 不会留下"删不掉的默认设备"；② 不新增任何控件；
  ③ 移除它用的是**既有**的"删设备"入口（`device-{i}-remove-button`）。
  代价：**改变了建轨这一既有操作的产物形状**（任何数设备数量的既有判据、
  以及"新建轨 = 3 条 Op"的假设都要跟着核对 —— 见"改动面"）。
- **C 让 SFZ 库按钮在仓内可用**（打包一份小样本、或放开按字节过滤）：能给出**可见**的
  乐器入口，但要动资产登记（`assets/manifest.json` ＋ SHA-256，AGENTS.md §2.9），
  且"没有素材就一个入口都没有"这个结构性问题仍在。本票否：它是**另一张票**的规模。

## 采纳 B 时的实现边界（已按此实现，供批准时核对）

1. **只加 MIDI 轨**：`if kind == NewTrackKind::Midi`。音频轨的音源是录进来的素材，
   给它挂合成器是错的（判据 ①b 钉"音频轨设备链仍为空"）。
2. **键名不许自造**：九个参数名必须逐字是 `CONSUMED_INSTRUMENT_PARAMS` 的成员。
   判据把这一条**交给引擎的权威函数**（`yeban_engine::synth::is_consumed_instrument_param`）
   逐条断言 ⇒ 引擎改名/删键时本表立刻红，而不是静默变成哑设备。
   ⚠ 承认残留风险：`bridge.rs` 这张表是引擎清单之外的**第二份值表**（缺省值没有契约位）。
   把它上提到共享契约属账本 **S6**（参数契约的家今天还在 `yeban-mcp`），本票不做。
3. **单位不发明**：只在共享契约里已有该键时逐字照抄（`cutoff_hz` → `Hz`）；
   振荡器电平/失谐在契约里没有单位 ⇒ 写 `None`。
4. **`latency_samples` 写 0**：与既有 `builtin_effect_device` 同一条纪律 ——
   没有公布的延迟就报零，⛔ 不编数字（PDC 会把这台设备的延迟当真）。
5. **身份新分配、槽位 0**：`EntityId::new()`；`slot_index: 0` 对空链等价于追加。
6. **不动方法面**：`ui/*` 与 `yeban_*` 的方法签名一字未改 ⇒ `METHOD_COUNT`、
   `Operation::ALL`、dryRun 计数三处计数器判据不受影响。

## 改动面（B 会碰到的既有假设）

| 位置 | 原假设 | 现在 |
| --- | --- | --- |
| `bridge::track_create_ops` | 返回 3~4 条 `Op` | 返回 4~5 条（MIDI 轨 +1 条 `InsertDevice`），`Vec::with_capacity(5)` |
| 判据 ① `creating_a_midi_track_commits_once_and_wires_it_to_the_master` | 只断言轨＋路由 | 追加断言：链上恰 1 台设备、`kind == InternalInstrument`、`!bypassed`、`latency_samples == 0`、含 `cutoff_hz` 或振荡器键、每个键名被引擎认 |
| 判据 ①b `creating_an_audio_track_leaves_the_device_chain_empty` | — | 新增：音频轨仍 `devices.is_empty()` |
| 宿主 `host.rs` 的建轨回调 | 一次点击 = 一次 `commit_ops` | **不变**（同一批次多一条 Op，⛔ 不是第二次提交） |

## 判据（怎么证明它真的做了事，⛔ 不靠"看起来接上了"）

`crates/yeban-app/tests/track_routing_crud.rs`：

| 判据 | 断言的是 |
| --- | --- |
| ① `creating_a_midi_track_commits_once_and_wires_it_to_the_master`（扩格） | 一次提交后：新轨存在、连到 master、**链上有一台被引擎判为音源的内置乐器**，键名逐个过 `is_consumed_instrument_param` |
| ①b `creating_an_audio_track_leaves_the_device_chain_empty` | 音频轨没被顺手挂上合成器 |

⚠ 注入实测：删掉 `InsertDevice` 那一条 ⇒ ① 红；把某个键名改成引擎不认的名字 ⇒ ① 红；
把 `kind` 改成 `InternalEffect` ⇒ ① 红。

**可听性另有一测**：本线第二首作品的 `sound_probe` 阶段用 UI 建轨、UI 画音符后导出，
以 WAV 头与响度读数验收（⛔ 不以判据的"结构对了"替代"听得到"）。

## 若批准 A（否决 B），需要一并处理

撤回 `track_create_ops` 里的 `InsertDevice` 分支 ＋ `BUILTIN_INSTRUMENT_NAME` /
`BUILTIN_INSTRUMENT_PARAMS` / `builtin_instrument_device` ＋判据 ① 的设备段与 ①b，
并把「模拟人类 UI 模式创作一首能听的 3 分钟作品」改为登记为**能力缺口**
（口径：UI 只能建哑轨，音源需领域 Ops 面补齐），台账 D-UI-4 行随之关闭为"维持现状"。
