# SFZ 触发期随机族（`offset_random` / `amp_random`）与缺口审计——工作线台账

> 基线 `9b4240aa`（`main`）。本文件两件事：**①** 把这一片真正做掉的
> `offset_random` / `amp_random` 的口径、出处、落地与判据写下来；**②** 记下
> 2026-11-16 那次**可复算的语料普查**与**仍未实现**的族（供下一片排序用）。
> 纪律照旧：数字必须给出**口径与命令**，⛔ 不抄旧台账的手抄值。

---

## §1 语料普查（2026-11-16 实测）

**指标**：`git ls-files -z` 里 `.sfz` 结尾的文件。⚠ 分母有两档：**1 399** 是仓库内
全部追踪的 `.sfz`，其中 **1 398** 份在 `assets/samples/**`（`find` 读数），另 **1** 份是
测试夹具 `crates/yeban-app/tests/fixtures/sampler_c1.sfz`（本文件的下表都用 1 398 的语料面
口径，夹具不含本表任何 opcode）。逐文件剥 `//` 行注释与 `/* */` 块后，用
`(?:^|[\s])([A-Za-z_][A-Za-z_0-9]*)\s*=` 取 opcode 名；命令是**一次性探针**
（`python3 -B`，未入库）。单位：**文件数**（至少出现一次）与**出现次数**。

| opcode | 文件数 | 出现次数 | 本仓状态（2026-11-16 现读） |
| :--- | ---: | ---: | :--- |
| `offset_random` | **79** | **4 901** | ⭐ **本片落地**（解析 → 摊平 → 两条有触发身份的路） |
| `amp_random` | **2** | **19** | ⭐ **本片落地**（同上） |
| `rt_decay` | 5 | 22 | ⛔ 未实现（需要"按键按住时长"的会话状态） |
| `eq1_freq` / `eq1_bw` | 33 / 33 | 33 / 33 | ⛔ 未实现（三段 EQ，需要 biquad 链） |
| `eq1_gain` | 11 | 11 | ⛔ 未实现 |
| `fil2_type` | 24 | 47 | ⛔ 未实现（第二滤波器） |
| `eg06_*`（`eg06_time1_oncc109` 为首） | 23 | — | ⛔ 未实现（ARIA 多段 EG #6） |
| `var01_*` | 6 | — | ⛔ 未实现（ARIA 变量） |
| `width` | 11 | 19 | ⛔ 未实现（立体声宽度；⚠ 引擎 SFZ 支路是**单声道**累加，见 §6） |
| `set_hdcc10` | 10 | 10 | ⛔ 未实现（14 位 CC；需要 >127 的 CC 域） |
| `off_shape` | 8 | 8 | ⛔ 未实现（关断曲线） |
| `bend_smooth` | 2 | 2 | ⛔ 未实现 |

> ⚠ 这张表**只列本片实测过的名字**。`lfo0N_*` / `fil_type` / `ampeg_*` /
> `fileg_*` / `pitcheg_*` 等族**不在**表里：它们在 `yeban-sfz` + 引擎里已有实现
> （各自有判据），⛔ 不要因为本表没写就当成缺口。

---

## §2 规范逐字（2026-11-16 现读 sfzformat.com）

`offset_random`（<https://sfzformat.com/opcodes/offset_random/>）：

> Random offset added to the region offset.

> Computed when the note is triggered. **Unipolar in ARIA, Cakewalk and rcg sfz.**

表行：Type = integer、**Default = 0**、Range = `0 to 4294967296`、Unit = **sample units**。

`amp_random`（<https://sfzformat.com/opcodes/amp_random/>）：

> Random volume for the region, in decibels.

> Computed when the note is triggered, remains the same for that region for as long as
> the region plays.

> In ARIA and Cakewalk, `amp_random` is **unipolar**. To create variation which can be
> either negative or positive, this needs to be combined with a fixed `volume` offset …

表行：Type = float、**Default = 0**、Range = `0 to 24`、Unit = **dB**。

⚠ **语料反例（实测）**：`assets/samples/freepats-button-accordion-hn/PRESET Button
Accordion HN tuned.sfz:51` 的 `<group>` 段写 `amp_random=-6`（1 处 / 1 文件）。

---

## §3 工程裁决（⛔ 规范没说死的地方）

| # | 裁决 | 依据 |
| :--- | :--- | :--- |
| D1 | 位移取 **`floor(rand × N)` ∈ `[0, N)`** | 页面的 "Unipolar" 只给方向；闭开区间让 `N = 0` 与"不写"**逐位等价**。算术用 `f64`（`N` 可到 `u32::MAX`，`f32` 24 位尾数会丢低位）。 |
| D2 | `amp_random` 取 **单极衰减**：`10 ^ (−(rand × dB) / 20)` | 页面明写 ARIA / Cakewalk 单极；双极是 rgc 的口径，本仓不取。 |
| D2b | `amp_random` 的取值域放宽为 **−24 ..= 24**（表格是 `0 to 24`） | 语料里真有 `amp_random=-6`；拒收会让整份 34 区域乐器不可加载（与 `#206` 容忍 `-1.6.` 同一条"保可加载"纪律）。负值走**同一条公式** ⇒ 提升 `[0, +|v|]` dB，⛔ 不新造语义、⛔ 不当 0。 |
| D3 | 两族与 `lorand` / `hirand` **共用同一个** `trigger_rand(occurrence, note, 1)` | 规范对三者都写 "Computed when the note is triggered"；同次触发一个值让"同一次触发的多个 region"仍然相关（与随机轮替的分段区间语义同源）。 |
| D4 | `offset_random` 只动**起点**，`end` 不动；位移把起点推过终点 ⇒ 该 region **不发声** | 与 `end <= offset` 那条既有归约**同一条**（⛔ 不夹回起点、⛔ 不发明反向区间）。 |
| D5 | 取值域按 **`u32` 全域**（`0 ..= 4294967295`），越界响亮 `IntegerOutOfRange` | 表行上界 `4294967296` 比 `u32::MAX` 大 1，装不进 `u32`；⛔ 不静默钳位。 |
| D6 | **哪些路应用**：有触发身份的两条（实时声部触发、离线 `occurrence` 入口）。一次性回调口 `PreparedSfz::render_block_into_with_cc` **没有** occurrence 概念 ⇒ 按 `rand = 0` 渲染 | 该路**本来就**不做 `seq_*` 轮替与 `lorand` 随机门选择（现读 `sfz_instrument.rs` 的循环体：只有 `accepts` ＋ CC 门控）⇒ 随机族与它们**同一条**边界，⛔ 不是本片新造的例外。 |

---

## §4 落地（四处，各一处实现）

| 件 | 位置 | 说明 |
| :--- | :--- | :--- |
| 解析字段 | `yeban-sfz/src/instrument.rs` 的 `Region::offset_random` / `Region::amp_random` | 缺省 0；越界 `IntegerOutOfRange` / `FloatOutOfRange` |
| 唯一算术 | `yeban_sfz::instrument::offset_random_shift` / `amp_random_gain` | 纯函数、零分配；引擎⛔ 不另抄 |
| 区间 | `Region::playback_span_with_rand`（`playback_span` ＝ `rand = 0` 的同一条实现） | 位移 → 既有 `end` 归约 |
| 摊平表 | `PreparedRegion::offset_random` / `amp_random_db` ＋ `span_bounds_with_rand` / `amp_random_gain` | 触发期求值；实时路径把随机音量**折进** `SfzVoice::track_gain`（⛔ 不新增逐声部字段，`SfzVoice` 仍 **40 B**） |

**尺寸**：`PreparedRegion` `6248 → 6256`（＋8 ＝ `offset_random: u32` ＋ `amp_random: f32`，
摊平期、⛔ 不进实时对象）。`SfzVoice` 40 / `SfzVoicePool` 4800 / `EngineRuntime` 45888
**三条全部 ＋0**（判据 `sfz_instrument::s*_sizes` 的绝对值与 `bus_routing::r6` /
`bus_insert::b5` 的绝对值已按新基线更新）。

**`has_rand` 的语义扩写**：从"有随机门"扩到"**本次触发需要一个随机数**"
（三族任一在场）⇒ 触发期照旧推进每键触发计数并派生 `rand`；三族都不在的库仍为
`false`（分支整段跳过，逐位不变）。

---

## §5 判据

`crates/yeban-engine/tests/sfz_random_start.rs`：

| # | 判据 | 判定量 |
| :--- | :--- | :--- |
| R1 | 摊平表真的带上两族读数 | `offset_random()` / `amp_random_db()`（含缺省） |
| R2 | 实时起点按 `trigger_rand` 位移 | 首帧 == `pcm[100 + floor(rand × 800)] × 同一增益链` |
| R3 | 离线 `occurrence = Some(0)` 与实时同一身份 ⇒ 同起点 | 首帧逐位相同；16 个身份至少两种起点；`None` ⇒ 与不写逐位相同 |
| R4 | `amp_random` 单极衰减、两条路同一乘子 | 比值 == `10 ^ (−(rand × 24) / 20)` 且 ∈ (0, 1] |
| R5 | 两族都不写 ⇒ 逐位不变 | 首帧 == `pcm[0]`；实时 == 离线；显式写 0 也逐位相同 |
| R6 | `has_rand` 三族各自为真、都没有为假 | `has_rand()` |
| R7 | 位移进循环窗口后**回绕**（规范页把"loop 样本从随机起点起播"列为用途） | 首帧 == 回绕点的采样值，离线同身份逐位相同 |

`yeban-sfz` 侧另有**常驻语料判据** `tests/corpus_random_families.rs`（C1：1 398 份语料里
**没有任何**文件因为这两个 opcode 解析失败；C2：accordion 文件的组级 `amp_random=-6`
与 `offset_random=10` 真的继承到末区域），以及四条单元判据（`playback.rs`）：单极区间与 `rand < 1` 的边界
（`0.999999 × 1000 ⇒ 999`）、`NaN` 退化为 0、位移越界 ⇒ `None`、`amp_random` 的
`0 dB ⇒ 恰 1.0` 与越界响亮报错。

---

## §6 仍未做（下一片的候选，按语料面排序）

1. `rt_decay`（5 文件 / 22 处）：`trigger=release` 的音量随"按键按住时长"每秒钟衰减
   `rt_decay` dB。需要**每键 note-on 帧**这份会话状态（池里现无；`note_start[16]` 是
   逐声部的弯音原点，⛔ 不能顶替）。
2. `eq1_*` / `eq2_*` / `eq3_*`（33 / 2 / 2 文件）：三段 EQ。`yeban-dsp` 有 biquad
   原语（`voice_filter` / `shaping`），但 SFZ 声部的滤波器链**只有一只** `VoiceFilter`
   ⇒ 需要每声部的 EQ 状态（会动 `SfzVoicePool` 的尺寸基线）。
3. `fil2_*`（24 文件 / 47 处）：第二滤波器 —— 同上，需要第二只滤波器状态。
4. `eg06_*`（23 文件）：ARIA 的 `egN` 多段包络；本仓现只有 `ampeg` / `fileg` /
   `pitcheg` 三套，`egN` 的段数与电平表是**新子系统**。
5. `width` / `width_onccN`（11 / 5 文件）：⚠ 引擎的 SFZ 支路把声部**累加成单声道**
   （`render_track(..., out: &mut [f32])`）⇒ 宽度没有落点；接入前需要一次立体声
   声部路径的裁决，⛔ 不要先做半个宽度。
6. `set_hdccN`（10 文件）：14 位 CC；本仓 CC 探针的域是 `u8`（`Fn(u8) -> u8`）⇒
   需要新的 CC 域，属于架构单。
7. `off_shape`（8 文件）：`off_mode` 关断的曲线；现有实现是**指数**淡出（`STEAL_FADE_FLOOR`），
   与规范可选的曲线词汇要逐条对表。
8. `var01_*`（6 文件）/ `bend_smooth`（2 文件）：面小、语义跨播放器不一致，先登记。
