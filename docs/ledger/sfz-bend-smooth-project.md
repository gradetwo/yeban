# 立项：`bend_smooth`（弯音惯性 / 毫秒）

> 状态：**立项 / 待实现**。缺口普查表 §6 列的最后一条小项：语料 2 处
> （`aliexpress-erhu` 与 `cithara-barbarica` 的 `Programs/modules/controls/common.sfz:7`
> 都是 `bend_smooth=40`，即 40 ms），当前解析层静默忽略。

## §1 规范逐字（2026-11-16 现读 sfzformat.com/opcodes/bend_smooth/）

> Pitch bend smoothness. Adds "inertia" to pitch bends, so fast movements of the
> pitch bend wheel will have a delayed effect on the pitch change.

> Sets the smoothness for pitch bend in milliseconds. Adds inertia to the bends, so fast
> movements of the controller will have a delayed, smoothed effect, similar to `smoothccN`.

表行：Type = float、Default = 0、Range = `0 to ?`、Unit = **ms**、Version = SFZ v2。
⇒ **0 = 无平滑（逐位不变）**；40 ms = 对弯音目标值施加时间常数约 40 ms 的低通。

## §2 现状与落点

| 件 | 现状 |
| :--- | :--- |
| `yeban-sfz` 解析 | ⛔ `bend_smooth` 不识别（未知 opcode 静默忽略） |
| 引擎弯音 | 走**音符级** `pitchBendCurve`（模型 `MidiNote`），实时池把它存在每槽 `pitch_shape` ＋ `note_start`，逐块按局部时间求值 [ADR-0012] |
| 实时弯音轮 | ⛔ 本仓没有"弯音轮控制器"运行态（`EngineEvent` 没有 PitchBend 变体）；只有已排程的音符曲线 |

⇒ 规范说的"pitch bend wheel"在本仓的**对应物是音符的 `pitchBendCurve`**，
不能造出一条新的运行态通路（那是架构单）。

## §3 建议口径（待实现时逐条落地）

1. `bend_smooth` 是**乐器级/区级**标量（`<control>` 或 region 皆可，按 opcode 继承链），
   单位 ms，缺省 0；解析进 `Region::bend_smooth_ms`（f32，有限非负，越界明确 `Err`）。
2. 平滑对象是**弯音曲线求值出的 cents 目标**：逐块（量子）用单极点低通
   `y += (target − y) × (1 − exp(−dt / τ))`，`τ = ms / 1000`，`dt` = 块帧数 / 采样率。
   ⚠ 与 `smoothccN` / 参数目标表的平滑保持**同一条公式**（`ARCH-DSP-001`），
   ⛔ 不在本片新写一条曲线。
3. 每声部的平滑状态放**池级每槽数组**（与 `pitch_shape` / `note_start` 同款，
   ⛔ 不进 `SfzVoice` 的 40 B 红线）。缺省 0 ⇒ 状态不被读、求值逐位不变。
4. 离线导出：用同一份标量在同一处求值（`render_note_with` 的 `pitch_cents`），
   ⛔ 不做第二套平滑。

## §4 判据

| # | 判据 | 判定量 |
| :--- | :--- | :--- |
| S1 | 解析 `bend_smooth=40` ⇒ 40.0；缺省 0；越界（负/非有限）响亮 `Err` | 字段读数 / 错误码 |
| S2 | 0 ⇒ 与接线前**逐位相同**（缺省文件与既有曲线的音频字节不变） | FNV1a 峰值 / 逐位 |
| S3 | 40 ms ⇒ 弯音目标阶跃被平滑：第一块只走到目标的一部分，若干块后逼近 | 逐块 cents 序列 |
| S4 | 实时池与离线导出同值（同一条公式、同一个 τ） | 两者首块 cents 逐位相同 |
| S5 | 两份语料（erhu / cithara）可加载，读数 40.0 | 解析成功 + 字段读数 |

## §5 语料实读

```
assets/samples/aliexpress-erhu/Programs/modules/controls/common.sfz:7:bend_smooth=40
assets/samples/cithara-barbarica/Programs/modules/controls/common.sfz:7:bend_smooth=40
```

## §6 不在本片范围

- **真正的 MIDI 弯音轮运行态**（`EngineEvent::PitchBend`）：本仓没有该事件，
  不属于"把 opcode 接上"的范围；需要时另立架构单。
- **`smoothccN`**：另一族 opcode，已有参数目标表平滑；本片只复用其公式，不改它。
