# SFZ opcode 缺口收口（`#331`）——工作线台账

> 上一份台账：`docs/ledger/sfz-offline-parity-notes.md`（`#310`／`#311`，离线渲染与实时同口径）。
> 本份记的是**那一轮登记、这一轮真的做掉**的四项缺口，以及**没做**的两项的裁决与理由。
> 纪律照旧：**保留原文 ＋ 追加修订段 ＋ 点名被修订的支**；⛔ 不删旧结论。

---

## §1 任务单与 ROI 排序

用户原始诉求（逐字）：

```text
接着做 --track-sampler 一次只能装一条轨 和 ampeg_dynamic、off_time/fileg_*、WAV smpl 块的循环点
```

按"用户听到的差别 × 改动面 × 回归风险"排序后的**实际执行顺序**：

| 序 | 项 | 用户听到的差别 | 改动面 | 本轮结论 |
| ---: | :--- | :--- | :--- | :--- |
| 1 | `--track-sampler` 可重复 | ⛔ 不是声音，是**工作流**：多乐器工程不用跑 N 次命令 | `cli.rs`（解析／配对／应用）＋ 帮助文本 | ✅ 完成（6 条判据） |
| 2 | `off_time` / `off_mode=time` | 有 `off_by` 的镲片/吉他扼音**淡出长度**变成作者写的那个值 | `PreparedRegion` ＋ `stop_off_by_victims` 一个分支 | ✅ 完成（5 条判据） |
| 3 | WAV `smpl` 块的循环点 | 采样库里**只写 `loop_mode`** 的音色从"只响一声"变成正常循环 | 引擎归约 ＋ `sfz_load` 扫 RIFF | ✅ 完成（5 ＋ 5 条判据） |
| 4 | `ampeg_dynamic` | ⛔ **今天听不出**（见 §5.1） | 需要先让实时路径读 CC 状态 | ⛔ 未做（裁决：前置未满足） |
| 5 | `fileg_*` | 音色随时间的明暗变化（filter envelope） | 需要 per-region 滤波链 ＋ 第二套包络 ＋ 声部状态 | ⛔ 未做（见 §5.2） |

**为什么 4／5 不做**：这两项都不是"接线"级别的活，而是**新子系统**（§5 逐条说明）。
把它们做一半（例如只解析不消费、或只做离线不做实时）会在本仓制造"第二条半成品路径"，
比不做更糟 —— 本仓对这种情况的既有裁决是**书面登记 ＋ 另开单**（`ADR-0008` 那套）。

---

## §2 `--track-sampler` 可重复（一次装多份乐器）

### §2.1 缺陷（`#330` 逐字）

```text
--track-sampler 一次只能装一条轨（多乐器工程要跑 N 次命令 / 或只能手工点 GUI）
```

### §2.2 契约

| 项 | 裁决 |
| :--- | :--- |
| 重复语义 | `--track-sampler` **可重复**，`--sampler-track` **可重复**，**按出现位置一一配对**（第 i 个配第 i 个） |
| 条数 | `0/0` ⇒ 不装；`1/0` ⇒ 旧口径（展示轨）；`N/N` ⇒ 位置配对；**其余一律用法错误**（退出码 2） |
| N 份 ＋ 0 个目标 | 用法错误（⛔ 不把 N 份装到同一条轨：后者覆盖前者，前几份静默不生效） |
| 同一目标轨两次 | 用法错误（同上理由） |
| 提交 | **整批一次** `commit_ops` ⇒ 一次撤销把这一批**全部**撤回 |
| 报告行 | **每份一行**，格式与 `#210` 逐字相同 ⇒ 只装 1 份时输出**逐字节不变** |
| 旧用法 | `--track-sampler p.sfz`（无选择器）**逐字节不变**（含报告行与目标轨） |

**为什么是位置配对**：一旦两个开关都能重复，只剩这一种配对不需要"猜"。⛔ 不按路径
字典序、⛔ 不按身份排序 —— 那会让"我写的顺序"与"它配的顺序"不一致，而这种不一致
表现为"乐器装错了轨"，用户极难发现（声音还在响，只是音色错）。

**为什么不用 `--track-sampler <id>:<path>`**：路径里允许出现 `:`（POSIX 文件名合法字符，
Windows 盘符是第二个例子）⇒ 那种形状会把一个**合法**的旧调用重新解释成 `id:path`，
破坏"旧用法逐字节不变"。这是 `#287` 已经裁决过的事，本轮沿用。

### §2.3 一个**测试夹具**层面的坑（值得记下来）

第一版 harness 是"spawn → `try_wait` 循环 → `wait_with_output`"。而用法错误的 stderr 是
**整份用法文本（约 37 KB）**：父进程在 `try_wait` 循环里不读管道，子进程写满管道缓冲
（容器里可能只有 8–16 KB）就阻塞在 `write`，父进程在等它退出 ⇒ **双向死锁**，
判据以"120 秒没退出"的形式**假红**。

同样的命令手动重定向到文件却立刻退出 2 —— 差别就在"管道没人读"。
修法：**两个 `read_to_end` 线程一 spawn 就开读**，主线程只等退出再 join
（见 `tests/track_sampler_multi.rs::invoke` 的文档）。

⚠ 既有 `#287` 的判据用的是旧形状：它输出小（几千字节），碰不到这条死锁 ——
本轮**不**去改它（没必要动一个绿的判据），但新判据必须用对的形状。

---

## §3 `off_time` / `off_mode=time`

### §3.1 关闭的书面偏离

`ADR-0008` 裁决 **C**（以及 `#94` 的登记）逐字：

> `off_mode=time` 按 `normal` 处理（规范说的是**一段长度 ＝ `off_time` 的淡出**、
> **independent of the sample release time**；本仓**不读 per-region `off_time`**）。

**本轮撤销这条偏离**：`time` 不再按 `normal` 处理。

### §3.2 规范事实（两页一起读）

* <https://sfzformat.com/opcodes/off_mode/>：ARIA 的 `time` "can be used to specify a time
  **independent of the sample release time** using `off_time`"；并且"time is actually the
  default value with a default off_time of 6 ms"；
* <https://sfzformat.com/opcodes/off_time/>："When `off_mode` is set to time, this specifies
  the fadeout time for regions being muted by voice-stealing."，Default `0.006`。

⭐ **作用面**：`off_mode` 只管 **`off_by` 关断**（"how a region is turned off by an `off_by`
opcode"）⇒ `off_time` **不参与 MIDI note-off**（那条路仍是 `ampeg_release`）。
这一点台账里反复确认过（`#94` 的"作用面"段），本轮不改。

### §3.3 实现（三处，全部复用既有原语）

| 位置 | 改动 |
| :--- | :--- |
| `PreparedRegion` | 新增 `off_time: f32`（**秒**，取 `Region::effective_off_time()`，缺省 6 ms） |
| `seconds_to_frames` | 从 `ampeg_run_from` 里**提取**出来的"秒 → 帧"唯一换算（`round`、非有限⇒0、饱和） |
| `SfzVoicePool::stop_off_by_victims` | 新增 `OffMode::Time` 分支：铺一段 `FadeRamp::new(off_time_frames)`，**⛔ 不碰 `end_sample`**、⛔ 不碰包络段位 |

**为什么用 `FadeRamp`（而不是"改 `AmpegRun` 的 release 段"）**：

1. 规范说的是**独立于** sample release time ⇒ 改 `AmpegRun` 的释放段就是"把 `off_time`
   当 `ampeg_release` 用"，语义上正是错的；
2. `SfzVoice` 必须保持 **40 B**（判据 `S9`；加 8 B 会把 `EngineRuntime` 顶出 196608 B
   的实时内存门槛）。用**池级**既有 `fade[index]` 数组 ⇒ **零新增声部字节**；
3. `mix_span_into` 在 `fade` 走完时 `break` ⇒ `render_slot` 看到 `written < remaining`
   ⇒ `silence_slot` 退休 —— 声部的生命周期完全复用既有那条路，⛔ 没有第二套终点口径。

**⚠ 曲线形状**：用的是**窃取路那条指数**（`STEAL_FADE_FLOOR = 1e-3`），
`off_shape` / `off_curve`（ARIA 的淡出曲线控制）**仍未做** —— 登记在 §5.3。

**`off_time == 0`**：`FadeRamp::new(0)` 返回 `NONE`（不淡出、也不退休）⇒ 必须**显式硬停**。
判据 `a_zero_off_time_hard_stops` 钉的就是这条（一个"0 秒淡出等于永远响"的实现在这里会红）。

**⚠ 尺寸代价（如实记）**：`PreparedRegion` **6168 → 6176 B（＋8）**。它按**区域数**计、
摊平期定，⛔ 不进实时对象；`SfzVoice` 仍 **40 B**、池仍 **2240 B**、`EngineRuntime` 仍 **45680 B**
（判据 `S9` 现读，`EngineRuntime` 门槛 204800 B，余量 159120）。
⇒ **13 条硬编码尺寸基线的判据因此变红**，处理见 §6.1。

### §3.4 与 `ARCH-RT-004` 的 3 ms 不冲突

两条**不同的路**：3 ms 是**池满窃取**（`steal`，谁都没要求），`off_time` 是**作者要求的
`off_by` 关断**（缺省 6 ms）。⛔ 不共用常量、⛔ 不互相覆盖。

---

## §4 WAV `smpl` 块的循环点

### §4.1 缺陷

采样库的常态是"loop 画在音频编辑器里、写进 WAV 的 `smpl` 块，SFZ 里只写
`loop_mode=loop_continuous`"。本仓此前**只认 SFZ 写的点** ⇒ 这类音色整段不循环，
听感是"一个音只响一声"。上一轮的 `#311` 修的是"`loop_end` 缺省取样本尾"，**不是**这条。

### §4.2 规范事实

<https://sfzformat.com/opcodes/loop_start/>：

> If `loop_start` is not specified and the sample has a loop defined, the SFZ player will
> use **the start point of the first loop defined in the file**. … For files which have
> multiple loops defined, this is the only way to get anything other than the first defined
> loop to play.

⇒ "SFZ 没写点" ≠ "不循环"，而是"用样本的第一个 loop"。

### §4.3 落地（两段，各自有判据）

| 段 | 位置 | 做法 |
| :--- | :--- | :--- |
| 字节 → `SampleLoop` | `sfz_load::read_wav_smpl_loop` | **流式**扫 RIFF chunk（`data` 直接 seek 跳过）；只认 `RIFF`；取第 0 条 loop |
| `SampleLoop` → 窗口 | `reduce_loop_window` 的 `sample_loop` 参数 | `loop_end == 0`（SFZ 没写点）时替换成样本的 `[start, end)` |

**⚠ `dwEnd` 是含的**：`smpl` 的 `dwStart`/`dwEnd` 是"第一个/最后一个被循环的样本"，
本仓窗口是**半开** `[start, end)` ⇒ 读盘时 **＋1**。差一不补的后果是每次循环丢一个样本
（听感是"循环缝里有周期性咔哒"，低频长音上尤其明显）。
判据 `the_smpl_loop_is_read_and_its_end_becomes_exclusive` 钉的就是这个 `+1`。

### §4.4 ⚠ 书面偏离：**整体**回退，不是**逐点**回退

规范是逐点的（只写 `loop_end` 时 `loop_start` 仍取样本的）。本仓改成**整体**：

```text
loop_end == 0（一个点都没写）  ⇒ 用样本的 (start, end)
否则                          ⇒ 完全用 SFZ 的（loop_start, loop_end）
```

理由（这是**取舍**，不是遗漏）：`Region::loop_start` 是 `u32`、缺省值 `0`，
"显式写 0"与"没写"在**解析后的数据结构里不可区分**。逐点回退会把"想从 0 起循环"的作者
（语料里常见写法）判成"用样本的 4503 起" ⇒ 窗口退化 ⇒ **不循环**，比不读 `smpl` 更糟。

要真正逐点，需要给 `yeban_sfz::instrument::Region` 加两个"写了没"的布尔
（公共面改动，波及 3 处构造点与全部下游判据）⇒ 另开单。

**代价如实登记**：只写 `loop_end` 不写 `loop_start` 的文件仍从 0 起（＝ 接线前，
⛔ 不是本轮引入的新问题）。

### §4.5 尺寸代价

`PreparedSample` **40 → 48 B（＋8）**：`loop_points: Option<SampleLoop>`（两个 `u32`）
在 40 B 的尾部对齐之后净占 8 B。它按**样本数**计、⛔ 不进实时对象（`#279` 的定性未变）。
判据 `S12b` 的绝对值基线已按"保留原文 ＋ 追加修订段"更新（见 §6.1）。

### §4.6 坏元数据不该毁掉音频

`smpl` 被截断 / 尺寸撒谎 ⇒ `read_exact` 失败 ⇒ `None` ⇒ 退回接线前的窗口。
判据 `a_broken_smpl_block_does_not_break_decoding` 钉住"解码照常成功、`loop_points = None`"。

⚠ 这条与"解码失败"**不同**：解码失败有 `unreadable_samples` 报告行（会回退到内存正弦，
用户听得出来）；`smpl` 只是**可选元数据**，它的缺失不改变"这份样本能不能播"。

---

## §5 未做的两项（明确裁决）

### §5.1 `ampeg_dynamic` —— **前置条件不满足**

规范逐字（<https://sfzformat.com/opcodes/ampeg_dynamic/>）：

> When 1, causes envelope segment durations and sustain level to be **recalculated when a
> MIDI CC message modulating those envelopes is received**. When 0, envelope segment
> durations and sustain level are calculated only at the start of the particular envelope
> segment.

⇒ 它的**全部语义**是"收到 CC 时重算"。而本仓的实时路径**不读 CC 状态**
（`sfz_instrument.rs` 模块头的"CC 调制"一节逐字："实时通路⛔ 仍不读 CC 状态"），
离线路径的 CC 是**常量探针**（没有"消息流"这个概念）。

**结论**：今天把 `ampeg_dynamic` 接上，**产不出任何可观测差异** ——
它不会改变一个样本、一个读数。写一个"读了但不生效"的字段正是本仓最忌讳的形态。

**下一步（真正的前置）**：让实时通路读会话 CC 状态（SPSC 环 ＋ 每块快照），
届时 `ampeg_dynamic=1` 才有消费者。那是**另一个单**，而且它的收益远大于
`ampeg_dynamic` 本身（`ampeg_*ccN`／`cutoff_oncc*`／`pan_oncc*` 全部依赖它）。

### §5.2 `fileg_*` —— **需要新子系统**

`fileg_attack/decay/sustain/release` ＋ `fileg_depth` ＋ `fil_type`/`cutoff`/`resonance`
意味着：**per-region 滤波器**（今天引擎里**一个都没有**）＋ **第二套包络** ＋
**每声部的滤波状态**。

最后一条是硬约束：`SfzVoice` 必须 40 B（判据 `S9`），`EngineRuntime` 必须 ≤ 196608 B
（实测 202592 B 就越界）。一个 biquad 状态 ≈ 4–8 字节 × 2（双二阶）⇒ 16 个声部 × 8 轨
就要另开**池级**定长数组，且要重算内存门槛。

⇒ 工作量是 §2／§3／§4 三项之和的 2–3 倍，且必须一次做对（半成品＝第二条音源路径）。
**另开单**，⛔ 不在本轮硬塞。

### §5.3 其余登记（本轮新增的缺口）

| 缺口 | 状态 |
| :--- | :--- |
| `off_shape` / `off_curve`（`off_mode=time` 的淡出曲线） | 未做（用既有指数淡出） |
| `smpl` 的**多 loop**（只取第 0 条） | 按规范"只有第 0 条可播"，非缺口 |
| `cue ` / `inst` / AIFF `MARK` 的 loop 来源 | 未做（本仓只解 WAV 腿的 `smpl`） |
| 逐点回退（只写 `loop_end` 时 `loop_start` 取样本的） | 未做（§4.4 的取舍） |

---

## §6 判据清单

| 文件 | 条数 | 钉住什么 |
| :--- | ---: | :--- |
| `crates/yeban-app/tests/track_sampler_multi.rs` | 6 | 多份乐器：位置配对／一次提交／条数不等／重复目标／单份回归 |
| `crates/yeban-app/tests/track_sampler_track_selector.rs` | 6（既有，改 1 处构造） | `#287` 的目标轨选择（含"单独给选择器"的原文案） |
| `crates/yeban-engine/tests/sfz_off_time.rs` | 5 | `off_time` 决定长度／独立于 release／缺省 6 ms／0 秒硬停／`normal` 负对照 |
| `crates/yeban-engine/tests/sfz_sample_loop.rs` | 5 | 样本 loop 成窗口／真回绕（斜坡反解）／SFZ 优先／无 loop 不变／退化判 `None` |
| `crates/yeban-app/tests/sfz_wav_smpl_loop.rs` | 5 | `smpl` 读出 `+1`／无块／零 loop／退化／截断不毁解码 |

合计 **27 条**（新增 21）。

### §6.1 被修订的 13 条既有判据（⛔ 不是"改个数字"，全部保留原文）

第一遍跑 `yeban-engine --test sfz_instrument` 是 **13 failed / 98 passed**，13 条全是
**硬编码尺寸基线**与**一条被落地的登记**：

| 判据 | 期望 → 实得 | 触发原因 | 处理 |
| :--- | :--- | :--- | :--- |
| `S9`（区间上界） | `5768 ＋ 384 …= 6168` → 6176 越界 8 B | `off_time` ＋8 | 上界 ＋16 → **＋24**，报文写明"下界未动、⛔ 不是放宽判据" |
| `S91` `S98` | `6152 ＋ 16` → 6176 | 同上 | 写成 **`6152 + 16 + 8`**（原文式子留在式子里） |
| `S100` `S106` `S111` `S113` `S114` | `6168` → 6176 | 同上 | 绝对值 ＋8，报文追加修订句 |
| `S115` `S116` `S117` | 四元组 `(6168, 40, 2240, …)` → `(6176, …)` | 同上 | 同上（②③④ 仍 ＋0，写明） |
| `S118`（四元组 + 巡检第 6 条） | 四元组同上；巡检"`off_time` 必须不在引擎代码里" → 在 | `#331` 把 `off_time` 落地 | 巡检行 **`NotInCode` → `InCode` 翻转**（⛔ 不删行） |
| `S12b` | `PreparedSample` 40 → **48** | `loop_points` ＋8 | 绝对值更新 ＋ 修订句 |

**纪律**：每条的原句**逐字保留**，后面接 `→ ⭐ `#331` 修订：…`；**点名被修订的是哪一支**
（"绝对值那一条"，⛔ 不是整条判据作废）。`S118` 那一行尤其重要 —— 它登记的就是
"引擎未应用 `off_time`"，落地之后**见证要翻转而不是消失**。

引擎模块头的登记（`off_time` 不在本片范围 / `off_mode=time` 按 `normal` 处理 /
`ADR-0008` 裁决 C）也补了修订段，并**明写裁决 C 撤销**（不是"暂缓"）。

---

## §7 回归

### §7.1 实际拿到的读数（⛔ 不写"全量跑过"，只写真跑过的）

| 命令 | 结果 |
| :--- | :--- |
| `cargo test -p yeban-engine --lib` | 322 passed / 0 failed |
| `sfz_off_time` / `sfz_sample_loop` / `sfz_loop_defaults` / `sfz_offline_note_off` | 5 / 5 / 4 / 8 passed |
| `track_sampler_multi` / `track_sampler_track_selector` / `sfz_wav_smpl_loop` | 6 / 6 / 5 passed |
| `yeban-engine --test sfz_instrument` | **111 passed / 0 failed**（第一遍 13 failed，见 §6.1） |
| `yeban-app --lib` | **399 passed / 0 failed**（第一遍 1 failed，磁盘假红，见 §7.3） |
| `cargo check --workspace --all-features` | 0 error ／ 0 warning |
| `cargo fmt --all --check` | 通过 |
| 干净 HEAD 上 `git apply --check` | 19 个文件全过，退出码 0 |

⇒ **本轮所有目标的读数都拿到了**（⛔ 没有"没跑完"这一项了）。

### §7.2 一个必须记住的教训（文案即契约）

`#310` 那次已经踩过一次，这一轮又踩到形态相同的坑："换了数据来源，报告文案也是契约"。
本轮把 `off_mode=time` 从 `normal` 支里拆出来之后，既有判据 `S39`（`normal` 关断）
**必须仍绿** —— 它绿，说明拆得干净。

### §7.3 环境坑：并发链接会假红

三个 app target 一起跑 ⇒ `error: linking with cc failed: exit status: 1`，三条全红；
同一份代码逐条 `-j 2` ⇒ 全绿。测试二进制约 400 MB，磁盘当时还有 20 GB（不是 `ENOSPC`）。
**本仓验证请加 `-j 2`**，否则会把"链接失败"误读成"判据失败"。
上一轮"`sfz_instrument` 90 分钟没跑完"也是同一回事：它在反复重试链接，不是在编译。

还有第二层：`collect2: fatal error: ld terminated with signal 7 [Bus error]` ——
不是参数错，是 **mmap 页分配不出来**（盘写不进去）。这个沙箱的 `df` 读数在接近写满时
**不可信**（显示 17–26 GB 可用时 `rename` 已在报 `ENOSPC`）。元凶是
`target/debug/deps` 里 **1004 个测试二进制、129 GB** 的历史残留；清掉无关的之后
**157 GB 可用**，链接一次通过。

### §7.4 教训：**尺寸基线是硬编码契约，不是现读读数**

我在交付说明里先写过"判据现读 `size_of`，加字段不影响断言"——**跑出来 13 条红，这句话是错的**。
`S91`／`S98`／`S100`／`S106`／… 每一条都把绝对字节数**钉死**在里面，这是**故意**的：
加字段的人必须一行一行看过去、在报文里写清"这 8 B 是谁"。
⇒ 以后凡是给 `PreparedRegion` / `PreparedSample` / `SfzVoice` 加字段，
**先假设 13 条会红**，并把"更新基线"算进工作量，别拿"判据现读"当挡箭牌。
