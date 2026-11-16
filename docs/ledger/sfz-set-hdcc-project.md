# 立项：`set_hdccN`（ARIA 浮点 CC 初值）

> 状态：**立项 / 待实现**。语料 10 处（`ixox-flute` 9 处宏形态 ＋ `salamander-grand` 1 处 `set_hdcc10=0.5`），
> 规范页可用（<https://sfzformat.com/opcodes/set_hdccN/>），语义清晰：`<control>` 段的**浮点** CC 初值。

## §1 规范逐字（2026-11-16 现读）

> Like `set_ccN` but with floating point MIDI CCs.

> Sets a default initial value for MIDI CC number N using a floating point value, when the
> instrument is initially loaded. Used under the ‹`control`› header.

> `set_realccN` is a deprecated alias for `set_hdccN`.

表行：Version = ARIA、Type = float、Default = N/A、Range = 见页面（示例 `0.5`、`1`）。

## §2 现状与落点

| 件 | 现状 |
| :--- | :--- |
| `yeban-sfz` `<control>` | ✅ `set_ccN`（整数 0..=127、非整数告警丢弃）已在 `control.rs` |
| `yeban-sfz` 缝 | ⛔ `set_hdccN` / `set_realccN` 不识别（未知 opcode 静默忽略） |
| 引擎 `CcSeedTable` | ✅ 按 `Instrument::cc_defaults()` 播种（`#110`）；初值域是 `u8` |

## §3 需要裁决的两点

1. **浮点 → 7 位整数**：本仓 CC 探针域是 `u8`（`loccN`/`hiccN`、`xfin_*` 都是 0..=127）⇒
   `set_hdccN=0.5` 必须换算。建议 **`round(v × 127)`**（0.5 → 64；⛔ 不截断成 63 ——
   `round` 与"中点"的语义一致，且与 `set_ccN` 的"整数即所写值"不冲突）。
2. **与 `set_ccN` 同时出现**：同一 CC 两个初值 ⇒ 建议 **`set_ccN` 优先**（它是规范 SFZ v2
   的精确整数声明；`set_hdccN` 是 ARIA 扩展、且带一次换算舍入）。登记为工程裁决。

## §4 判据（验收）

| # | 判据 | 判定量 |
| :--- | :--- | :--- |
| H1 | 解析：`set_hdcc10=0.5` → 初值 64 | `cc_defaults()[10] == 64` |
| H2 | 值域：`0` / `1` / 小数 与越界（`>1`、负数、非有限） | 端点可读；越界按 `set_ccN` 同款（告警丢弃或响亮 `Err`，实现时二选一并登记） |
| H3 | 别名 `set_realccN` 与 `set_hdccN` 等价 | 两份文本给出同一张表 |
| H4 | 与 `set_ccN` 同时出现 ⇒ `set_ccN` 赢 | 同一 CC 上两种声明，读数是整数那份 |
| H5 | 引擎播种：`0.5` ⇒ `ControlChangeState` 读到 64 | `seed` 后 `value(cc) == 64` |
| H6 | 语料回归：`ixox-flute` / `salamander-grand` 仍可加载且初值正确 | 解析成功 + 逐 CC 读数 |

## §5 语料实读（2026-11-16）

- `assets/samples/salamander-grand/Salamander Grand Piano V3.sfz:44`：`set_hdcc10=0.5`
- `assets/samples/ixox-flute/Ixox Flute.sfz:74-84`：`set_hdcc10=0.5` ＋ 9 处宏形态
  （`set_hdcc$EXP=1`、`set_hdcc$F1_VOL=0.5` …，宏展开后是 `set_hdcc<数字>`）
- ⚠ 宏展开依赖 `#define`，探针必须在**展开后**的文本上数（与 `#123` 的口径同源）。

## §6 不在本片范围

- 引擎侧 CC 探针的域改成 14 位（真正的"high-definition"）：`loccN`/`hiccN`/`xfin_*`
  全部是 7 位语义，改域是**架构单**；本片只把加载初值换算到既有域。
- `set_hdccN` 的**运行时** CC（MIDI 14 位）——同样属于架构单。
