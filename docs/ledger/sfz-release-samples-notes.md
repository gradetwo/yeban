# SFZ 释放采样（`trigger=release` / `release_key`）与 `rt_decay`——工作线台账

> 基线 `391a2748`（`main`，含随机起点/音量片）。本片把**松键采样**从"永远不发声"
> 变成三条路都发声，并接上 `rt_decay` 的按住时长衰减。

---

## §1 缺口与语料（2026-11-16 实测）

**指标**：`git ls-files -z` 的 1 398 份 `assets/samples/**/*.sfz`（剥 `//` 与 `/* */`），
按 `(?:^|[\s])([A-Za-z_][A-Za-z_0-9]*)\s*=` 取 opcode 名；单位＝文件数／次数。

| opcode | 文件数 | 次数 | 修前行为 |
| :--- | ---: | ---: | :--- |
| `trigger=release` | **29** | **117** | ⛔ 键掩码在摊平期被清零（`#94` 防"按下时打出来"），而**没有**任何 note-off 触发路径 ⇒ 整族**永不发声** |
| `trigger=release_key` | **10** | **20** | 同上 |
| `rt_decay` | **5** | **22** | 解析层不认（未知 opcode 静默忽略） |

**用户听到的**：钢琴的 key-up 噪声、吉他的松弦声、手风琴的松键机械声全部消失；
写入 `trigger=release` 的乐器在夜半里只剩"按下"那一半。

---

## §2 规范逐字（2026-11-16 现读 sfzformat.com）

`trigger`（<https://sfzformat.com/opcodes/trigger/>）：

> `release` — Region will play after a key is released …

> `release_key` — same as release, but is not affected by sustain pedal …

> …will play as if `loop_mode` was set to `one_shot`.

`rt_decay`（<https://sfzformat.com/opcodes/rt_decay/>）：

> Applies only to regions that triggered through `trigger=release`. The volume decrease
> (in decibels) **per seconds** after the corresponding attack region was triggered.

> The sample `keyup_noise.wav` will play 3db quieter for every second the key has been on.

表行：Type = float、**Default = 0**、Range = `0 to 200`、Unit = **dB**。

---

## §3 工程裁决（规范没写死的四处）

| # | 裁决 | 依据 / 代价 |
| :--- | :--- | :--- |
| D1 | **释放声部的力度 = 该键最近一次 note-on 的力度** | 规范的 `trigger` 页对释放声部的力度来源**没有**规定；MIDI note-off 的力度在多数 DAW 里是 0（用它 ⇒ 释放采样全静音）。池里记 `key_velocity[128]`（128 B）。 |
| D2 | **两张掩码分开**：note-on 掩码 / 释放掩码 | 一条区域的"可播范围"在两侧本来就不同（事件族是静态事实）；共用一格必然有一侧错（修前正是 note-on 掩码把释放族清零）。代价 `＋32 B/区域`。 |
| D3 | **释放族用独立的轮替计数**（`release_occurrence[128]`） | 同一条区域表上两族可以各有自己的 `seq_position` 组；共用一格会让两族的"第 n 次"互相推进。代价 `＋512 B/池`。 |
| D4 | **延音踏板不在引擎建模**：note-off 一律按 `pedal_down = false` 处理 | 引擎没有踏板状态（现读 `synth.rs` 零处）；`release` 与 `release_key` 因此都在**键松开**时发声。⚠ 与 `ADR-0071`（踏板进自动化泳道）不冲突：那是控制面。**登记为待接**（见 §6）。 |
| D5 | 释放声部 **`end_sample = u64::MAX`**（没有后续 note-off 停它） | 规范"as if `loop_mode=one_shot`" ⇒ 播到样本尾；`release_note` 对 `one_shot` 本来就忽略。 |
| D6 | 离线 `rt_decay` 用**调用方给的 `held_frames`**；实时用 `off_frame − key_on_frame` | 两条路都有"按住时长"的原始事实 ⇒ ⛔ 不引入第二套时间基准。 |

---

## §4 落地（四条路，各一处实现）

| 件 | 位置 | 说明 |
| :--- | :--- | :--- |
| 解析 | `yeban-sfz` 的 `Region::rt_decay`（`0..=200`，越界响亮报错） | 只读不判：是否应用由引擎在**触发期**按事件族决定 |
| 唯一算术 | `yeban_sfz::instrument::rt_decay_gain` | `10 ^ (−(dB × 秒) / 20)`；`0` ⇒ 恰 `1.0` |
| 摊平 | `PreparedRegion::release_key_mask` / `release_vel_mask` / `rt_decay` | 释放掩码用**同一套**键/力度范围与静态门控，只把事件换成 `NoteOff{pedal_down:false}` |
| 实时 | `SfzVoicePool::trigger_release` → `trigger_event`（与 `trigger_note` **同一条实现**） | 池记 `key_on_frame` / `key_velocity` / `release_occurrence` / `sample_rate` |
| 合成器 | `SynthEngine::note_off_with_sources`：`release_note` ＋ `trigger_release` | 同一帧（`off_clock`）⇒ 缩短与新建同刻 |
| 离线 | `SfzInstrument::render_release_into_from_samples(.., held_frames, ..)` | 从 `out[held_frames]` 起写；`NoteInputs` 增 `event` / `frame_offset` / `rt_decay_held_frames` |
| 导出 | `yeban-app::sfz_load::SfzTrackSource::ensure_rendered` | note-on 段之后叠释放段，按需扩容（重渲染不拼接） |

**尺寸**：`PreparedRegion` `6256 → 6296`（＋40 ＝ 两张 `[u64;2]` 掩码 32 ＋ `rt_decay` 与对齐 8）；
`SfzVoicePool` `4800 → 6464`（＋1664 ＝ `key_on_frame` 1024 ＋ `release_occurrence` 512 ＋
`key_velocity` 128 ＋ 对齐）；`SfzVoice` 40 / `EngineRuntime` 45888 **＋0**。

---

## §5 判据

`crates/yeban-engine/tests/sfz_release_trigger.rs`：

| # | 判据 | 判定量 |
| :--- | :--- | :--- |
| T1 | 两张掩码各归各的 | 释放区域 `key_mask == [0,0]` 且 `release_key_mask != [0,0]`；起音相反 |
| T2 | note-on 不建释放声部、note-off 建 | 按下 `created == 0` 且全静音；松键 `created == 1` 且首帧 = 样本值 × 力度链 |
| T3 | `rt_decay` 按秒衰减 | 1 秒 / 6 dB/秒 ⇒ 首帧比值 ≈ 0.501187；缺省 `0` ⇒ 比值恰 1 |
| T4 | 离线从松键帧起写、与实时同值 | `out[..held]` 全 0；`out[held]` 与实时首帧**逐位相同** |
| T5 | 无释放区域 ⇒ 快路径 | `has_release_regions() == false`、`trigger_release == 0` |

`yeban-sfz` 侧另有三条单元判据：`rt_decay` 的默认值、按秒线性衰减（1 s/2 s）、
域外（`>200` / 负数）响亮报错。

---

## §6 仍未做 / 已知边界（如实登记）

1. **延音踏板**：引擎没有踏板状态 ⇒ `trigger=release` 在踏板踩下时也会跟着**键**松开
   发声（规范要求等踏板松开）。要接需要会话运行态的 CC64 与"延迟 note-off"语义；
   `TriggerEvent::NoteOff { pedal_down }` 这一层**已经就绪**（`yeban-sfz` 的谓词已判），
   只差引擎的踏板状态来源。
2. **`rt_decayN`**（ARIA 扩展，按时间区间选不同衰减率）：语料 **0** 处，未做。
3. **"对应的 attack region"判定**（ARIA / DropZone 要求存在对应起音区域才响）：
   规范自陈 "varies considerably between SFZ players"，本片**不发明**（与
   `Trigger` 的既有登记一致）—— 释放族只要有匹配键就响。
4. **释放声部与样本尾**：`ampeg_release` 对释放声部没有后续 note-off ⇒ 不会进入释放段；
   声部按 `one_shot` 播到样本尾。若作者的释放样本需要包络收尾，需要一次"样本尾即
   note-off"的裁决（⛔ 本片不发明）。
5. **轨增益/随机/rt_decay 的乘法次序**：`voice_gain = track × amp_random × rt_decay`，
   乐器层（`gain_at`）在逐块路径现算 —— 与 note-on 路径**同一处**，⛔ 不重复。
