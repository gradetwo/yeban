# SFZ 离线渲染与实时同口径（note-off / 轮替 / region 级样本 / 循环缺省 / 静默失败）—— 工作线 notes

> 工作线: `line/sfz-offline-parity`（基线 `main` `da37619c`，已 `git pull` 到该提交）
> 拥有: `crates/yeban-engine/src/sfz_instrument.rs`、`crates/yeban-app/src/sfz_load.rs`、
> `crates/yeban-app/src/render_core.rs`、`crates/yeban-engine/tests/sfz_offline_note_off.rs`、本文件
> 触发: **一次真实的编曲事故** —— 用 CLI `--sfz <音色> --export-wav` 渲了一首 188 s 的歌，
> 成品里弦乐"断气"、吉他硬切、鼓的 `kick` 尾巴消失，而**同一份工程的 GUI 试听是正常的**。

---

## 0. 一句话结论

离线渲染（`--export-wav` 走的那条）比实时声部（`SfzVoicePool`）**少三件语义**：
**note-off**、**轮替互斥**、**region 级样本**。三者都不是"新功能"，是**两条路的口径分叉**：

| # | 缺口 | 用户听到的 | 修前代码位置 |
| :--- | :--- | :--- | :--- |
| ① | 离线**没有 note-off** ⇒ `ampeg_release` 永不生效 | 音符在时值处被硬切：断气、爆音、鼓尾巴消失 | `EnvRun::new(.., u64::MAX, ..)` |
| ② | 离线**没有轮替**（`seq_*`）⇒ 变体同相叠加 | 电平 +6 dB、机关枪效应、轮替本身消失 | `for region in regions()` 无 occurrence 过滤 |
| ③ | 离线**只有一份样本** ⇒ 所有匹配区域都用它 | 力度分层/多音区共用第一个匹配区域的采样 | `render_note_into(source: &InMemorySample)` |
| ④ | 每条轨各自解码**全部**采样 | （不是听感，是 OOM：`exit=137`） | `new_from_sfz_path` 逐轨自建样本表 |
| ⑤ | `loop_mode` 不写窗口 ⇒ **完全不循环** | 长音/弦乐在样本尾被硬切（"断气"的第二因） | `reduce_loop_window` 的 `loop_end <= loop_start ⇒ None`（缺省 `loop_end` 读成 `0`） |
| ⑥ | 采样读不到 ⇒ **静默**回退 440 Hz 正弦 | "音色完全不对"，而 stdout 一切正常 | `build_region_samples` 的 `Err(_) => sine(440.0, ..)` |
| ⑦ | 素材全挂在主总线轨 ⇒ **静默**导出空带子 | 拿到一条全静音的文件，日志全是 OK | `renderable` 排掉 `master_bus_track_id` 且**无任何提示** |

第 ④ 条是修 ③ 时**自己踩出来的**：第一版让 79 轨各建一份表 ⇒ 212 个鼓采样解码 79 遍
⇒ 进程被 OOM killer 干掉。修法是全工程共享一张 `Arc<BTreeMap<..>>`（只加引用计数）。

第 ⑤⑥⑦ 条是**另一类**：不是两条路分叉，而是**"规范写了缺省值，本仓没实现"**与
**"失败时不说话"**。它们的共同后果是同一个 —— 用户听到不对，但所有日志都正常。

---

## 1. 缺口 ①：`ampeg_release` 在离线渲染里是死的（**逐字节证据**）

### 1.1 怎么发现的

编曲用的音色全部写了 `ampeg_release`：

```
strings.sfz    ampeg_release=0.8      piano.sfz   ampeg_release=0.6
viola.sfz      ampeg_release=1        harp.sfz    ampeg_release=12
glock.sfz      ampeg_release=7        kit_funk3   ampeg_release=0.3 / 3 / 6 / 8
```

但成品里一个释放段都听不到。**对照实验**（同工程、同音色，只改 `ampeg_release`）：

| 版本 | `ampeg_release=0.8` | `ampeg_release=0` | 结论 |
| :--- | :--- | :--- | :--- |
| **修前**（`main` `da37619c` 编译的 `release` 二进制） | md5 `7a52931e…` | md5 `7a52931e…` | **逐字节相同** ⇒ 这个 opcode 一个字节都没读 |
| **修后** | md5 `7ce94f53…` | md5 `7a52931e…` | 有差异 ⇒ 生效 |

导出摘要里的 `digest` 也一样：修前两者都是 `72b81f32…`。

> 命令（修前，20 s 夹具）：
> `yeban-app --open proj7/guitar.yeban --sfz strings.sfz --export-wav A.wav --wav-seconds 20 …`
> `yeban-app --open proj7/guitar.yeban --sfz strings_rel0.sfz --export-wav B.wav …`
> `md5sum A.wav B.wav` ⇒ 同一行。

### 1.2 根因

`SfzInstrument::render_note_into_with_cc` 里，`EnvRun` 的 note-off 帧被写死成 `u64::MAX`：

```rust
.map(|run| EnvRun::new(run, ENV_STAGE_IDLE, 0, u64::MAX, 0, false));
```

`EnvRun::advance` 只在 `abs_frame >= frame` 时打 `ENV_RELEASING` 位 ⇒ `u64::MAX` 永远到不了
⇒ 释放段永远不开始 ⇒ `ampeg_release` 读进 `AmpegRun` 了，但**没有任何代码路径会用到它**。
注释当时也如实写着"这条路**没有 note-off**"，只是没人把"导出走的就是这条路"连起来。

### 1.3 修法

新增 `SfzInstrument::render_note_into_from_samples(samples, note, velocity, held_frames, occurrence, out, hz)`：
`held_frames = Some(n)` ⇒ `EnvRun::frame = n` ⇒ 第 `n` 帧进释放段，`ampeg_release` 决定释放多久。
`held_frames = None` ⇒ `u64::MAX` ⇒ **与接线前逐位相同**。

`SfzTrackSource::ensure_rendered` 改走这个入口，`held_frames = 音符时值`，并且**缓冲按需扩容**：
只要这次把缓冲写满了（说明释放段/采样还没完）就翻倍重来，上限 8 s。
⚠ 扩容是**重渲染**而不是拼接 —— 包络的"第几帧"以 `out` 起点为零点，分段渲染会在接缝处重置。

### 1.4 同时修掉的两个同源问题

| 现象 | 原因 | 修后语义 |
| :--- | :--- | :--- |
| `loop_mode=one_shot` 的鼓/镲被时值切断 | 没有 note-off 概念，一切都按时值停 | `one_shot` **忽略 note-off**（规范逐字："The entire sample will always play back in full."）⇒ 播满采样 |
| **没有** `ampeg_*` 的音色在 note-off 后停不下来 | `ampeg_run_from` 对"五值全透明"短路成 `None` ⇒ `mix_span_into` 的 `env_gain` 恒 `1.0` | `held = Some` 时显式补一个**零长释放** run（`f32::EPSILON` 绕开短路，换帧仍是 0）⇒ 在 note-off 当帧停 |

第二条**必须**修：否则"加了 note-off"对绝大多数音色（没写 `ampeg_*`）等于没加。

---

## 2. 缺口 ②：轮替（`seq_*`）在离线路径上全部叠加

### 2.1 现象

两条 `seq_length=2` ＋ `seq_position=1|2` 的 region、样本常值 0.5 / 0.25，
同一音符渲染一次：

| `occurrence` | 修前 | 修后 |
| :--- | ---: | ---: |
| `None`（旧入口） | **0.465**（＝ 0.310 ＋ 0.155，两个变体叠加） | 0.465（后向兼容，保留） |
| `Some(0)` | 不支持 | **0.310**（只出 a） |
| `Some(1)` | 不支持 | **0.155**（只出 b） |

实时路径（`SfzVoicePool::trigger_note`）早在 round-robin 那一单就做了互斥，判据在
`tests/sfz_round_robin.rs` 的 R1～R6；离线路径**从来没接**。

### 2.2 修法

`render_note_with` 里按 `PreparedRegion::selects_occurrence` 的**同一条判据**
（`seq_position == occurrence % seq_length + 1`）过滤；`seq_length <= 1` 不参与（后向兼容的全部）。

`SfzTrackSource` 构造期算 `occurrences_of(notes)`：同一音高按 `notes` 顺序计"第几次触发"，
逐音符喂进去。

---

## 3. 缺口 ③：一个音符只有一份样本

### 3.1 现象

修前 `region_sample_for` 只取**第一个**匹配区域的 `sample=`，而 `render_note_into` 会把
**所有**匹配区域都用这份 PCM 渲染 ⇒ 三种错法同时成立：

* 力度分层（`lovel`/`hivel` 重叠）⇒ 同一份采样播 N 遍；
* round-robin ⇒ 与 ② 叠加成"同一份样本叠 N 遍"，比"两个不同变体叠加"更糟（完全同相）；
* `key=` 重叠的多音区写法 ⇒ 高音区播低音区的采样。

### 3.2 修法

新增 `build_region_samples(instrument, sfz_path) -> (Arc<BTreeMap<String, InMemorySample>>, String)`：
把这份 SFZ 里**所有** `sample=` 各读一次，键用 `Region::sample_path()` 的**原样字符串**。
三种区域的处理：

| 区域 | 表里放什么 |
| :--- | :--- |
| `sample=*silence`（`#217`） | **空 PCM** ⇒ 渲染时一帧不写 |
| 正常 `sample=xxx.wav` | 真读出来的 PCM |
| 读不到 | 内存正弦 ＋ 来源文案写明原因（**保留**旧语义，⛔ 不在本单改成静音） |

表里**没有**的区域 ⇒ 该区域不发声（⛔ 不借用别的样本、⛔ 不回退正弦）。
理由：错样本比静音难查 —— 静音至少会在判据里红。

---

## 4. 缺口 ④：逐轨重复解码（OOM）

修 ③ 的第一版让 **每条轨**在 `new_from_sfz_path` 里自建样本表。79 轨的工程：

```
$ yeban-app --open proj7/kit.yeban --sfz kit_funk3.sfz --export-wav kit.wav --wav-seconds 12
exit=137                      # SIGKILL
```

212 个鼓采样 × 79 轨 × 每个 `read_sample_file` 都是一份**新的** `Arc<Vec<f32>>`。

修法：`render_core` 在**循环外**建一次表，`SfzTrackSource::with_shared_samples(Arc, describe)` 注入；
`BTreeMap` 的克隆只加引用计数 ⇒ PCM 全进程一份。修后最大驻留 **152 MB**，`exit=0`。

> ⚠ 顺带说明：修前**也**不省 —— `region_sample_for` 是**逐音符**读盘的（780 个音符 = 780 次）。
> 现在是"全工程一次"，比修前**更省**。

---

## 4.5 缺口 ⑤：`loop_mode` 写了也不能延音（规范缺省没实现）

### 4.5.1 现象

编曲用的弦乐 `.sfz` 长这样（`vsco2ce` 全套都是这个形状）：

```
<region> sample=VlnEns_susVib_v1_rr1.wav pitch_keycenter=67 lovel=1 hivel=64
         loop_mode=loop_continuous ampeg_release=0.8
```

**没有 `loop_start` / `loop_end`**。而 `sfzformat.com/opcodes/loop_end/` 逐字写着
**"Default: `-1` (end of sample)"** —— 语义是**循环整个样本**。

本仓把未写的 `loop_end` 读成 `0`（`read_u32(&scopes, "loop_end", .., 0)`），
`reduce_loop_window` 又按 `loop_end <= loop_start ⇒ None` 判死 ⇒ **一个本该延音的长音
在样本尾被硬切**。

实测（斜坡样本 0.2 s、输出 0.5 s、`key` 与 `pitch_keycenter` 同为 60 ⇒ `rate = 1`）：

| 夹具 | 修前 `written` | 修后 `written` |
| :--- | ---: | ---: |
| `loop_mode=no_loop` | 9599 | 9599（不变） |
| `loop_mode=loop_continuous`（**不写窗口**） | **9599** | **24000** |
| `loop_mode=loop_continuous loop_start=2400 loop_end=9600` | 24000 | 24000（不变） |
| `loop_mode=one_shot` | 9599 | 9599（不变） |

> 9599 而不是 9600：`mix_span_into` 的停止条件是"插值要读的**下一格**仍在 PCM 内"，
> 最后一帧的右邻不存在 —— 这条是**既有语义**，⛔ 不是本次引入的（判据 L3/L4 按 9599 钉）。

### 4.5.2 修法（`reduce_loop_window`，**唯一归约**，三条路同时受益）

| # | 修前 | 修后 | 依据 |
| :--- | :--- | :--- | :--- |
| ① | `loop_end == 0` ⇒ 与 `loop_start == 0` 组合成退化窗口 ⇒ **不循环** | `loop_end == 0` **＝ 未指定** ⇒ 窗口尾取 `source_end` | 规范缺省 `-1` |
| ② | `loop_end > source_end` ⇒ **整条判死** | **钳到** `source_end`（再钳到 `pcm_len`） | `end=-1` 时 `source_end = pcm_len`，与手写 `loop_end = 总帧数` 本就该同解 |
| ③ | `loop_start` 越界 ⇒ 未检查 | `loop_start >= pcm_len` ⇒ `None`（不循环） | 防回绕第一帧就越界读 |

⚠ **退化窗口仍然返回 `None`**（判据 `S31` 钉的那一条：`loop_start=100 loop_end=100`
依旧不循环）—— 本修订⛔ 不碰它。

---

## 4.6 缺口 ⑥⑦：静默失败（采样缺失、素材挂错轨）

这两条不是"渲染语义错了"，是"**错了但不说**"。修法都是**只加报告、⛔ 不改渲染行为**。

### 4.6.1 ⑥ 采样读不到

修前 `build_region_samples` 的失败分支：

```rust
Err(_) => InMemorySample::sine(440.0, 48_000.0, 48_000),
```

回退本身是**对的**（少一个采样的工程也要能出带子），但它**没有任何信号**：
`loaded-sfz:` 行照常打印，`exported-wav:` 照常成功，用户只听到"音色完全不对"。

修后：失败清单进 `RegionSampleTable::missing`（`(sfz 里的原文, 原因)`），
`render_core` 打成：

```
sample-missing: 2 个区域的采样读不到，已**回退为 440 Hz 正弦**（音色会完全不对）：
  - 真的不存在.wav: 样本读取失败: `/tmp/e2e/真的不存在.wav`: hound 失败: No such file or directory (os error 2)
  - 也不存在2.wav: 样本读取失败: …
```

上限 10 条（坏掉一百个采样时不刷屏，剩下的只报条数）。

### 4.6.2 ⑦ 素材挂在主总线轨上

`renderable` 排掉 `master_bus_track_id` —— 这是**对的**（总线不产生素材）。但如果工程的
素材**恰好全在**主总线轨上（导入的工程、手改过的 `.yeban`、把 clip 拖错轨），导出会
**成功**，拿到一条**全静音**的带子，而每一行日志都正常。

修后（只在"可渲染轨上真的什么都没有"时报，⛔ 不出假警）：

```
render-empty: 可渲染轨上没有任何 clip 或采样器（主总线轨上有 1 个 clip，按设计**不参与渲染**）
              —— 导出会是全静音。请检查素材是否挂在了主总线轨上。
```

⚠ 判据是 `!clips.is_empty() || sampler.is_some()`：**采样器轨也算内容**
（`--track-sampler` 走的是引擎分支，没有 clip）⇒ 不会把正常的采样器工程报成空。

---

## 4.7 一次自查出来的回归（来源文案也是契约）

改 `--sfz` 走共享表之后，`SfzTrackSource::sample_source()` 的兜底从 `per_note`
（`region_sample_for` 填的）换成了 `describe`（`build_region_samples` 填的）。
但 `describe` 原来只对"**真的读到**的样本"赋值，**内建静音那条留空** ⇒ `*silence`
工程的 `loaded-sfz:` 行从

```
sample_source=builtin-silence(*silence)
```

退成

```
sample_source=in-memory-sine(440Hz)
```

判据 `sfz_silence_pseudo_sample::the_cli_export_of_a_silence_only_instrument_is_really_silent`
当场红（它钉的正是 `source=sfz(builtin-silence(*silence))` 逐字）。

修法：`silence` 分支也写 `line = BUILTIN_SILENCE_SOURCE`。

⚠ 记下来的理由：这是"**换了数据来源，却忘了来源文案也是契约**"这一类错误的样本。
`sample_source=` 被判据逐字钉住（任务 `#217`）⇒ 它和函数返回值一样是 API，
不是可以随手改的日志。

---

## 5. 判据（`crates/yeban-engine/tests/sfz_offline_note_off.rs`，8 条，全绿）

| # | 判据 | 判定量 |
| :--- | :--- | :--- |
| O1 | `ampeg_release` **真的生效** | `held=2400`、`release=0.1 s`（4800 帧）⇒ `written == 7200` 整；`t=2399` 仍满值、`t=3600` 已衰减、`t=7200` 后逐位 0 |
| O2 | 旧入口**逐位不变** | 写了 `ampeg_release=0.1` 的 SFZ，`render_note_into` 仍然整段恒定 |
| O3 | `held = None` ⇒ **与旧入口逐位相同** | 两条路输出**逐 `f32` 位相等**（带 `attack/decay/sustain` 的夹具） |
| O4 | **无 `ampeg_*` 也响应 note-off** | `written == held`（⛔ 不是播到样本尾） |
| O5 | `one_shot` **忽略 note-off** | `written ≈ pcm_len`（播满 4096 帧） |
| O6 | 轮替**互斥** | `Some(0)=0.310`、`Some(1)=0.155`、`None=0.465`（＝两者之和，旧行为） |
| O7 | **region 级样本** | 同音符两条 `key=` 各取自己的样本，值成 2:1 |
| O8 | 表里缺该区域 ⇒ **不发声** | `written == 0`，且整段逐位 0 |

`cargo test -p yeban-engine --test sfz_offline_note_off` ⇒ `8 passed`。

### 5.1 循环缺省（`crates/yeban-engine/tests/sfz_loop_defaults.rs`，4 条，全绿）

| # | 判据 | 判定量 |
| :--- | :--- | :--- |
| L1 | **只写 `loop_mode`** ⇒ 循环整个样本 | `written == 24000`（0.5 s），且 `out[t]` 与 `out[t − 9600]` **逐位相同**（斜坡夹具可逐帧反解下标） |
| L2 | `loop_end` 越界 ⇒ **钳位后仍循环** | 与 L1 **逐位相同**（最大差 < 1e-6） |
| L3 | **反向腿**：`no_loop` 不循环 | `written == 9599`（＝ `pcm_len − 1`） |
| L4 | **退化窗口不变** | `loop_start=100 loop_end=100` ⇒ `written == 9599`（与 `S31` 同口径） |

### 5.2 采样缺失报告（`crates/yeban-app/tests/sfz_missing_sample_report.rs`，4 条，全绿）

| # | 判据 | 判定量 |
| :--- | :--- | :--- |
| M1 | 缺一个采样 ⇒ **恰有一条** `missing` | `len == 1`、原文 `gone.wav`、原因里含路径 |
| M2 | 全部读到 ⇒ **不报警** | `missing.is_empty()` 且 `describe` 非空 |
| M3 | `sample=*silence` **不是**缺采样 | `missing` 为空、表里那条 PCM 为空 |
| M4 | 坏采样**仍然回退**（行为不变） | 回退样本 48 000 帧 / 48 kHz / 非直流 |

### 5.3 端到端 release（`crates/yeban-app/tests/sfz_offline_release_export.rs`，1 条，绿）

走**产品路径**（`render_project_f32_with_assets` ＋ `--sfz` 旁路）钉住"导出的字节里能听见释放段"：

| # | 判据 | 判定量 |
| :--- | :--- | :--- |
| E1 | 只差 `ampeg_release` 的两份 SFZ ⇒ 导出**必须不同** | `digest` 不等 |
| E2 | 有 release ⇒ 释放段中点仍有信号，**且≈按住段的一半** | 窗口峰值比 ∈ [0.35, 0.65]（线性释放 `(R−k)/R = 0.5`） |
| E3 | 没有 release ⇒ note-off 之后**逐位 0** | 窗口峰值 `to_bits() == 0` |
| E4 | 两份在 note-off **之前**同值 | 0.25 s ±5 ms 逐样本最大差 < 1e-6 |

> ⚠ E2/E3 用**窗口峰值**而不是单点：夹具样本是周期信号，单点会落在过零点上；
> 更糟的是渲染链有 DC blocker，**直流样本会被它变成 Nyquist 振荡**（逐样本 ±交替）
> —— 第一版判据就是这么红的（`t=0.65 s` 读到 4.8e-6，而同一时刻的包络其实是 0.25）。
> 这条坑记在这里，避免下一个人重踩。

---

## 6. 兼容性（实测，不是推断）

| 场景 | 结果 |
| :--- | :--- |
| 新引擎、`ampeg_release=0` 的 SFZ | 与**旧引擎**产物**逐样本最大差 `0.000e+00`**（md5 `7a52931e…` 两边一致） |
| 旧入口 `render_note_into` / `render_note_into_with_cc` | 代码路径一字未改（只多一层委派），判据 O2/O3 钉住 |
| 没有 `seq_*` 的音色 | `occurrence` 拿到也不生效（判据条件 `seq_length > 1`） |
| 有 `seq_*` 的音色 | **会变**（这正是修复目标），旧行为只能由 `occurrence = None` 复现 |
| 只写 `loop_mode`、不写窗口的音色 | **会变**：从"播完样本就停"变成"循环整个样本"（这正是修复目标） |
| 手写 `loop_end > 样本长` 的音色 | **会变**：从"完全不循环"变成"循环到样本尾"（钳位） |
| 写了显式窗口（`loop_start`/`loop_end` 都在界内）的音色 | **逐位不变**（判据 `S28`/`S30`/`S46`/`S47` 全绿） |
| `sample=` 读不到 | 渲染**逐位不变**（仍回退正弦），只多一行 `sample-missing:` |

---

## 7. 创作过程中遇到、但**本次未修**的项（如实登记）

| # | 项 | 现状 | 为什么不修 / 修的前提 |
| :--- | :--- | :--- | :--- |
| 1 | `PreparedSfz::render_block_into`（摊平路径）仍无 note-off | 与离线同源的缺口 | 它的调用方是判据与 MCP 的"一次性试听"，**产品导出不走它**。改它要同时改"逐样本差 = 0"那批判据的夹具前提（模块头已登记：实时侧取 `end_sample = u64::MAX` 才逐位相等） |
| 2 | `ampeg_dynamic`（语料 **60** 次）/ `ampeg_delay`（**8** 次）未解析 | 静默忽略 | 单独立项：`ampeg_vel2delay` 的靶子就是 `ampeg_delay`，要先补后者才能做前者（台账 §phase-status 已记） |
| 3 | `off_time` / `fileg_*` / `pitcheg_*` / `pan` | 未做（`direction` 已做） | 模块头"仍未做"清单，需要单独立项 |
| 4 | 扩展 CC（128–137）的取值语义 | 不声称支持 | `render_note_into_with_cc` 的文档已登记 |
| 5 | 缺样本 ⇒ 静默回退 440 Hz 正弦 | ✅ **本轮已修报告侧**（`sample-missing:` 行）；回退本身保留 | 改成**硬错误**仍属产品决策（会推翻既有工程的产物字节与一批判据）。正确做法是加 `--strict-samples` 开关，本轮未做 |
| 6 | `--sfz` 是**整工程一份音色** | 逐轨绑定已有 `--track-sampler <path>` ＋ `--sampler-track <id>`（任务 `#287`） | 不是缺陷，是**发现性**问题：编曲脚本原先不知道这两个开关，才拆成 5 个工程分别渲 |
| 6b | `--track-sampler` **一次只能装一条轨**（`Option` ＋ `set_once_string`） | 5 轨 5 音色 ⇒ 必须渲 5 次或分轨渲 | **真实缺口**（本轮未修）：改成可重复的 `Vec<(track, path)>` 会动 CLI 契约与一批判据，属产品决策。绕法：`--track-sampler` 装一条 ＋ 存工程，或逐轨单独导出 |
| 7 | 工程里 clip 挂在**主总线轨**上时会发生什么 | ✅ **已复现并修**：`renderable` filter 掉 master ⇒ 素材全在主总线轨时导出全静音且无提示 | 修法是 `render-empty:` 报告（判据：`!clips.is_empty() \|\| sampler.is_some()`，⛔ 不出假警） |
| 8 | CLI 无内建"分轨 → 混音 → 母带"链路 | 需外部工具（本次用 Python 自写） | 产品功能缺口，不属引擎 |

---

## 8. 改动清单

| 文件 | 行数 | 内容 |
| :--- | ---: | :--- |
| `crates/yeban-engine/src/sfz_instrument.rs` | +194 | `NoteInputs`、`render_note_with`、`render_note_into_from_samples`、轮替互斥、region 级样本、note-off、`one_shot` 忽略 note-off、无包络的零长释放 |
| `crates/yeban-app/src/sfz_load.rs` | +296 | `build_region_samples`（共享 `Arc` 表）、`occurrences_of`、`with_shared_samples`、`ensure_rendered` 的 note-off ＋ 按需扩容、`render_block` 的窗口上限放开 |
| `crates/yeban-app/src/render_core.rs` | +52 | 共享样本表在循环外建一次；`sample-missing:` 与 `render-empty:` 两条报告 |
| `crates/yeban-engine/tests/sfz_offline_note_off.rs` | +275（新） | 判据 O1～O8 |
| `crates/yeban-engine/tests/sfz_loop_defaults.rs` | +134（新） | 判据 L1～L4（循环缺省与钳位） |
| `crates/yeban-app/tests/sfz_offline_release_export.rs` | +278（新） | 判据 E1～E4（端到端：导出的字节里能听见释放段） |
| `crates/yeban-app/tests/sfz_missing_sample_report.rs` | +194（新） | 判据 M1～M4（`missing` 清单与回退行为） |
| `docs/ledger/sfz-offline-parity-notes.md` | 本文件 | 事故复盘、根因、修法、判据、未修项 |
