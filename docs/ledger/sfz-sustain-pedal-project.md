# 立项：延音踏板（CC64）驱动 `trigger=release` 的时机（Proposed 设计稿）

> 状态：**已落地（2026-11-16）** —— 引擎面、池、合成器与 `EngineEvent::ControlChange`
> 的 CC64 跃变接线全部就绪；判据 P1～P4。在线 MIDI 的 CC64 经既有 `midi_live` →
> `EngineEvent::ControlChange` 通路到达引擎（通道→轨路由仍是既有的全局口径）。上一片
> （`docs/ledger/sfz-release-samples-notes.md`）把释放族接进了三条路，但**引擎没有踏板状态**
> ⇒ `trigger=release` 在踏板踩下时也会跟着键松开发声；规范要求等踏板松开。
> 本文件是范围、落点与判据；§4 已实现并判据化（`sfz_sustain_pedal.rs` P1～P3），
> **§7 的"CC64 跃变 → 逐轨 plan"接线仍未做**（产品路径的实时踏板今天还不起作用）。

---

## §1 规范事实（`yeban-sfz` 已建模的部分）

`TriggerEvent::NoteOff { pedal_down: bool }` 的文档逐字（`crates/yeban-sfz/src/instrument.rs`）：

> 规范里 `trigger=release` 的 region "will play on note-off **or** sustain pedal off"，
> 因此踏板踩下时它**不**发声（等踏板松开再报一次本事件）；`release_key` 则完全忽略踏板。

> 出处：<https://sfzformat.com/opcodes/trigger/>。

`Trigger::responds_to` 已实现：`release` ⇒ 只认 `NoteOff { pedal_down: false }`；
`release_key` ⇒ 认任意 `NoteOff`。⇒ **谓词层已就绪**，缺的只是引擎侧的踏板状态与延迟释放。

## §2 现状（本仓）

| 件 | 现状 |
| :--- | :--- |
| `yeban-sfz` | ✅ `TriggerEvent` / `responds_to` / 踏板语义全部建模并有判据 |
| `SfzVoicePool::trigger_release` | ⚠ 固定传 `NoteOff { pedal_down: false }` ⇒ 踏板未踩时也按"已松开"处理 |
| `SynthEngine::note_off_with_sources` | ⚠ 立刻 `release_note` ＋ `trigger_release`，没有"延迟到踏板松开" |
| `EngineEvent` | ✅ 已有 `ControlChange` 变体（`ring.rs`）可承载 CC64 |
| 离线导出 | ⚠ `TriggerEvent` 只在 `NoteInputs.event` 里；导出侧没有 CC64 时间线 |

## §3 建议口径（待人类批准 / 可被推翻）

1. **踏板状态是每轨（每槽）一位**：`SfzVoicePool::set_sustain_pedal(bool)`，`silence()` 复位。
2. **键松开 + 踏板踩下**：
   - `release_key` 区域**立刻**触发（`NoteOff { pedal_down: true }`）；
   - `release` 区域**不**触发；
   - **不**调 `release_note`（attack 声部继续响）。
3. **踏板松开**：对所有"键已松开但被踏板挂住"的音高：`release_note` ＋ 触发**只有**
   `release` 的区域。⚠ 需要一个**只**认 `release` 的事件（现 `NoteOff{pedal_down:false}`
   同时命中 `release_key` ⇒ 直接复用会在踏板松开时**重复**播 `release_key` 采样）。
   ⇒ 建议在 `yeban-sfz` 加 `TriggerEvent::PedalUp`（**纯加法**，`responds_to` 只让
   `Trigger::Release` 命中；`region_matches` / 摊平门控同步），代价是枚举多一档。
4. **每键挂起表**：`[bool; 128]`（键已松开、等踏板）+ 力度/起点沿用既有的
   `key_velocity` / `key_on_frame`。代价 **128 B / 池**。
5. **重踩踏板**（踏板松开前又按下同一键）：清掉该键的挂起位（新声部覆盖），与实时
   `PolySynth` 的同音重触发同一条口径。

## §4 判据（下一片的验收）

| # | 判据 | 判定量 |
| :--- | :--- | :--- |
| P1 | 踏板踩下时键松开：`release` 不响、`release_key` 响 | `created` 与首帧（两个样本常值可反解） |
| P2 | 踏板松开：`release` 响、`release_key` **不重复**响 | 同一 fixtures 的 `created` 与首帧 |
| P3 | `release_note` 的时机推迟到踏板松开 | attack 声部在第 `held` 帧之后仍非零 |
| P4 | 踏板抬起后复位 | 第二次踩下/松开的序列与第一次逐位相同 |
| P5 | 无释放区域的库：踏板事件一个新分支都不走 | `created == 0`、输出逐位不变 |

## §5 落地读数（2026-11-16）

| 件 | 状态 |
| :--- | :--- |
| `TriggerEvent::PedalUp` | ✅ 已加（`release` 认、`release_key` 不认）＋ 三条 responds_to 判例 |
| `SfzVoicePool` 踏板状态与挂起位 | ✅ `set_pedal` / `mark_sustained` / `sustained` / `clear_sustained`；池 6464 → **6600 B** |
| `trigger_release` 按踏板选事件 / `trigger_pedal_up` | ✅ 同一条 `trigger_event` 实现 |
| `SynthEngine::note_off_with_sources` | ✅ 踩下时不停声部、只响 `release_key`、标记挂起 |
| `SynthEngine::set_sustain_pedal` | ✅ 松开时逐个 `release_note` ＋ `trigger_pedal_up` |
| `EngineEvent::ControlChange` → `set_sustain_pedal` | ✅ **已接**：出队臂只记 `Option<bool>` 跃变，主快照块（`begin_block` 每量子返回 `held`）按 `current.sfz()` 逐轨应用 |
| 判据 | ✅ `tests/sfz_sustain_pedal.rs` P1～P4（本地 4/4；P4 用真 `SynthEngine`） |

## §6 不在本片范围

- **离线导出**的 CC64 时间线：模型层暂无"CC64 自动化 → 引擎"的通路（`ADR-0071` 只把
  CC 映射进自动化泳道）。导出侧要么按"踏板常开/常关"两档，要么等模型接线；
  ⛔ 不在本片私自造一条时间线。
- **半踏板（CC64 0..127 连续值）**：规范只定义"踩下/松开"两态；连续值属于 ARIA 扩展，
  未证语义不补齐。

## §7 CC64 跃变 → 逐轨 plan（已落地，保留为设计记录）

`EngineRuntime::render_block` 的 1) 段（`rt.rs` 的 `if let EngineEvent::ControlChange`）
只把值写进 `ControlChangeState`。要驱动 `SynthEngine::set_sustain_pedal(track, down, plan)`
需要**该轨当前摊平表**，而 `snapshot.begin_block()` 的守卫在这一步**之后**才借出。
两种落法（下一片二选一，⛔ 本片不擅自选定）：

1. **延后到快照借出之后重放**：在 1) 段只记"CC64 跃变"（定长 `Option<bool>` ＋ 一个
   计数器），快照借出后对每条已武装轨调 `set_sustain_pedal`；
2. **定长跃变队列**：`[Option<bool>; N]`（N ＝ 每量子最多跃变数），出队时逐条应用。

验收：实时路径上 `CC64=127 → 键松开`（`release` 不响、`release_key` 响）→ `CC64=0`
（`release` 响）的**音频读数**。⚠ 离线导出仍没有 CC64 时间线（§5）。

## §8 依赖与排期

- 依赖：`line/sfz-release-samples`（已推送、CI 验证中）—— 本片在它的 `trigger_event`
  与释放掩码之上加**踏板状态**与**挂起表**，⛔ 不改释放族的既有口径。
- 文件面：`yeban-sfz/src/instrument.rs`（新 `TriggerEvent` 变体）、
  `yeban-engine/src/sfz_instrument.rs`、`yeban-engine/src/synth.rs`、
  `yeban-engine/src/ring.rs`（`ControlChange` 消费）、`yeban-app` 的输入映射。
- 与 `line/sfz-release-samples` **同一批文件** ⇒ 必须等它落地后再起（一文件一写者）。
