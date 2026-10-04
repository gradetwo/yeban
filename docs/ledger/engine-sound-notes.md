# `yeban-engine` 合成路径台账：模型 → 快照 → 实时 → 输出（**引擎真的出声了**）

- **台账类型**：信号路径图 / 确定性论证 / 判据与注入实测 / 未实现项（**不是规范**）
- **记录时刻**：2026-10-05（第 1 轮；本机 `workspace-write` 沙箱 + 受限 rustup）
- **工作线**：`line/engine-sound`（worktree `yeban/.worktrees/engine-sound`，基线 `b8fe21e`）
- **所有者目录**：`crates/yeban-engine/**`、`crates/yeban-sfz/**`（本台账是唯一新增的文档文件）
- **落地范围**：新增 `crates/yeban-engine/src/synth.rs`；改写 `src/snapshot.rs` 的投影、
  `src/rt.rs` 的 `render_block`、`src/lib.rs` 的模块地图与边界；
  新增 `tests/synth_render.rs`、`tests/synth_rt_zero_alloc.rs`、`tests/support/mod.rs`；
  **修正** `tests/meter_rt_contract.rs` 的 S3 夹具（见 §7）
- **规范来源**：`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` `ROAD-M2-005` / `ROAD-M2-006`
  （静态预分配声部池 + 单轨音符触发稳定发声）；`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md`
  §3 的 `ARCH-RT-001`（零分配）、`ARCH-RT-004`（声部窃取）、`ARCH-DET-001`（L1 位精确）、
  `ARCH-DSP-001`（去爆音/限幅）
- **裁决来源**：`docs/adr/ADR-0001-workspace-topology-and-version-pinning.md`
  **D19**（feature 切分）、**D21**（通配路径依赖）、**D32**（跨架构按运算类别分策）

> 本文件回答六个问题：**信号从工程文件到样本走的是哪条路**、
> **哪些运算是位精确的（D32 分类）**、**每条判据怎么变红（5 条注入实测）**、
> **零分配在"音符铺满窗口"时是否仍然成立**、**哪些东西明确没做**、**需要谁裁决什么**。

---

## 0. 这一线改了什么（一句话）

改之前：`render_track_into(out, _track)` 就是 `out.fill(0.0)` —— **占位静音**。
电平计量、母线汇流、退役回收、FTZ/DAZ 全部是真的，但没有声源；`README` 里那句
"Nothing is playable yet" 说的就是它。

改之后：**工程的 MIDI 音符真的变成样本**。

```text
YebanProjectV1 ──(控制线程, src/snapshot.rs::project_schedules)──► BTreeMap<EntityId, NoteSchedule>
                                                                        │
   TrackV3.clips ─► ClipPlacement ─► clip_pool[clip_id] (Midi) ─► MidiNote
        │  start_tick = placement.start_tick + note.start_tick + micro_timing_ticks
        │  ratchet 把时值等分成 N 个脉冲（整数除法）
        │  probability → MidiNote::triggers(project.rng_seed)（确定性）
        ▼  tick → sample（一次 f64 换算 = IEEE 精确类）
   ScheduledNote { start_sample, end_sample, pitch, velocity, freq_hz, phase_inc, gain }
                                                                        │
YebanProjectV1 ─► EngineSnapshot（不可变, Arc 交换）────────────────────┘
                                                                        │
   RT: render_block ─► SynthEngine::render_track(track, schedule, out)
         │  游标只增不减地触发音符 → 定长声部池 [TrackSlot; 16] × [Voice; 16]
         │  逐样本：整数相位递推 → 波表 mip 查表 + 12 位线性插值 × ADSR × 增益
         ▼
   track_scratch [f32; 128] ─► MeterBank::measure（逐轨电平）
                            ─► sum_into_bus（等增益写 L/R）──► AudioBlock<128> ─► cpal / NullBackend
```

`EngineSnapshot` 新增**不可变**字段 `schedules: BTreeMap<EntityId, NoteSchedule>`：
它在控制线程构造（允许分配、允许超越函数），实时侧**只读**。
播放头（绝对样本位置）**不在快照里** —— 由 `EngineRuntime` 自己持有，因为
`MODEL-ISO-001` 明确禁止把挥发性走带状态放进模型投影。

---

## 1. 确定性论证（[ARCH-DET-001] 的 L1）

### 1.1 为什么"音符 → 样本位置"必须在构造期算完

tick → sample 的换算含 `bpm`（`f64`）与 960 PPQ。如果放在实时侧逐音符算，
"音频线程的浮点除法顺序"就会变成输出的一部分。现在这一步在投影里做**一次**，
实时侧只看整数样本位置：

```text
samples_per_tick = 60 × sample_rate / (bpm × PPQ)      // f64, 构造期一次
start_sample      = round(start_tick × samples_per_tick)  // IEEE 精确类
```

同一 `f64` 输入在任何架构上给出**同一**样本位置（乘/除/`round` 都是 IEEE-754 精确类）。

### 1.2 D32 分类表（这是本线的核心裁决落地）

| 层 | 运算 | 类别 | 何时发生 |
| :--- | :--- | :--- | :--- |
| 控制线程投影 | `note_to_hz`（`2^x`） | **超越函数类** | 每个音符一次 |
| 控制线程投影 | `db_to_gain`（`2^x`） | **超越函数类** | 每轨一次 |
| 控制线程投影 | tick→sample（乘/除/`round`） | IEEE 精确类 | 每个音符一次 |
| 控制线程投影 | `probability`（splitmix64 + f64 除法） | 整数 + 精确类 | 每个音符一次 |
| 快照边界 | `Adsr::set_params` 的 `exp` | **超越函数类** | 每修订一次（采样率变化时） |
| **逐样本** | `phase.wrapping_add(inc)` | 整数精确 | 每样本 |
| **逐样本** | `u64::from(phase) * len`、`>> 32` | 整数精确 | 每样本 |
| **逐样本** | `a + (b - a) * frac` | IEEE 精确类 | 每样本 |
| **逐样本** | `Adsr::process(gate)`（只有加/乘） | IEEE 精确类 | 每样本 |
| **逐样本** | `× envelope × gain` | IEEE 精确类 | 每样本 |

按 D32：
1. **IEEE 精确类**（钳位、比较、加减乘除、`sqrt`）⇒ 跨架构零容差，本线的逐样本路径全部属于这一类；
2. **超越函数类**（`exp2`/`exp`）⇒ 只出现在构造期与快照边界，给 D32 的 4096 ulp 预算；
3. 因此本线的 L1 强度是：**同一架构同一工具链下整条合成链逐位相同**
   （判据 J2 + 单测 `same_input_renders_byte_identical`）；
   跨架构下逐样本运算位精确，只有"音高、增益、包络系数"三个标量落在预算内。

### 1.3 相位为什么用整数（而不是浮点累加）

`phase: u32` 是"一个周期 = 2³²"的定点相位，`inc = round(freq_hz / sample_rate × 2³²)`。
整数回绕加法**没有累积误差**，也不受 FTZ/DAZ 影响。浮点相位累加会在长音符上缓慢漂移，
而漂移量取决于**块切分与累加次数** —— 那正是非确定性的来源。
`inc == 0`（极低频）被钳到 1，最低可表达频率是 `sample_rate / 2³² ≈ 1.1e-5 Hz @48k`。

查表用 `u64::from(phase) * len as u64 >> 32`（整数乘 + 右移，代替除法/取模），
末端回绕用一次比较；小数部分取低 32 位的高 12 位（`FRAC_BITS = 12`，插值抖动 ≈ −72 dBFS）。

---

## 2. 判据表（J1–J10）与本机实测证据

判据跑在 **`--no-default-features`（不编译 cpal）** 下 —— 本机与 CI 的主路径，不需要声卡。

| # | 判据 | 落地位置 |
| :-: | :--- | :--- |
| J1 | 含音符的工程渲染后**不是全零** | `tests/synth_render.rs::project_with_notes_produces_audible_samples` |
| J2 | 同输入两次渲染**逐位相同** | 同上 `same_input_renders_byte_identical` |
| J3 | 音符起点/终点对应样本位置误差 ≤ 1 量子 | 同上 `note_onset_and_offset_land_within_one_quantum` |
| J4 | 力度 0 不发声；力度越大有效值越大（严格单调 + 线性比） | 同上 `velocity_gate_and_monotonic_loudness` |
| J5 | 零分配窗口在"音符铺满窗口"时仍成立 | `tests/synth_rt_zero_alloc.rs`（`harness = false`） |
| J6 | 空工程 / 无音符工程 / 纯音频片段 ⇒ 静音且不 panic | `tests/synth_render.rs::empty_and_noteless_projects_render_silence_without_panic` |
| J7 | 升八度让零交叉数翻倍（音高正确） | 同上 `octave_doubles_the_zero_crossing_rate` |
| J8 | 等价快照重新武装**不重触发**、不改变任何样本位 | 同上 `rearming_an_equivalent_snapshot_does_not_retrigger` |
| J9 | 模型 → 快照 → 采样位置的覆盖度（手算栅格） | 同上 `snapshot_projects_model_notes_onto_the_sample_grid` |
| J10 | 超过调度表容量时计数丢弃、不 panic | 同上 `note_capacity_is_bounded_and_counted` |

另有 11 条**单元**判据在 `crates/yeban-engine/src/synth.rs` 的 `mod tests`
（相位下界/饱和、tick 换算、衰减上界、可闻门、槽位耗尽、硬窃取、游标不重触发等）。

### 2.1 本机实测读数（`cargo-local.sh test -p yeban-engine --no-default-features --all-targets`）

```text
lib 单测            : 89 passed; 0 failed
tests/synth_render  :  9 passed; 0 failed
tests/rt_zero_alloc :  ok（10,064 量子 + 63 次快照交换，零分配零释放）
tests/meter_rt_contract : ok（S1..S7）
tests/synth_rt_zero_alloc: ok（见 §3）
```

**"引擎真的出声了"的机械证据**（`--nocapture` 原样摘录）：

```text
[engine-sound] J1 frames=51200 nonzero=102398 peak=1.005757 \
  fingerprint=0x6a1671b901b3b6b9 scheduled_notes=3 notes_triggered=3 rendered_samples=51200
[engine-sound] J4 velocity=[0, 1, 32, 64, 127] \
  rms=[0.0, 0.004689777397952929, 0.15007287673449374, 0.3001457534689875, 0.5956017319392646]
[engine-sound] J7 crossings A4=882 A5=1764
[engine-sound/J5] 汇总: quanta=10001 scheduled_notes=256 notes_triggered=214 voice_steals=0 \
  非零样本=2560000 峰值=1.523726 filled(nonzero=88486, scheduled=4, triggered=4)
```

怎么读这三行：

1. **J1**：夹具是"3 个连续四分音符（力度 100）"的工程，渲染 400 个量子（51,200 帧）。
   左右两声道共 102,400 个样本里有 **102,398 个非零**；指纹 `0x6a1671b901b3b6b9`
   是对**全部样本位模式**的 FNV-1a 64。
   峰值 1.0058 > 1.0 是**预期之内**的：三个音符首尾相接，但前一个的 50 ms 释放尾与
   后一个的起音**重叠** ⇒ 两个声部瞬时相加。本切片**没有母线限制器**（见 §5 pending），
   因此判据只断言"两个声部叠加的上界 2.0"，越界才是增益/包络量级写错。
2. **J4**：`rms(1)=0.0046898`、`rms(32)=0.150073`、`rms(64)=0.300146`、`rms(127)=0.595602`。
   比值 32.000 / 2.0000 / 127.00 —— 力度到增益是**线性**的，与 `velocity/127` 一致。
3. **J7**：A4（440 Hz）在 1.0017 s 窗口里 882 次零交叉，A5 **恰好** 1764 = 2 × 882。
4. **J5**：10,000 个量子（1,280,000 帧）里 **2,560,000 个样本全部非零**
   （交叠音符铺满整个窗口），触发 214 个音符、零次声部窃取、**零分配零释放**。

---

## 3. 零分配：为什么单开一个 `harness = false` 目标

既有的 `tests/rt_zero_alloc.rs` 用 `filled_project()`，它的 4 个音符只覆盖前 1.5 秒
—— 10,000 个量子的窗口里**绝大多数时间没有声部在跑**，于是"零分配"可能只是
"什么都没做"。`tests/synth_rt_zero_alloc.rs` 用**256 个交叠音符**（每 240 tick 起音、
时值 480 tick ⇒ 覆盖 1,548,000 样本 > 窗口的 1,280,000 样本）复测，
并**同时**断言窗口里确实有非零样本（覆盖度自检，防假绿）：

```text
[engine-sound/J5] 10_000 quanta (saturated notes): allocations=0 deallocations=0
[engine-sound/J5] snapshot swap + quantum: allocations=0 deallocations=0   × 63
[engine-sound/J5] ok: 10,000 量子（音符铺满窗口）+ 63 次快照交换 + filled_project 4,000 量子，
                   实时窗口内零分配零释放
```

场景 3 再用真实的 `filled_project()` 复测 4,000 个量子（4 个音符全部触发），仍然零分配。

---

## 4. 注入 → 变红 → 还原（5 条，本机真跑）

方法：改源码 → `cargo-local.sh test -p yeban-engine --no-default-features --all-targets --no-fail-fast`
→ 记录红点 → 从备份还原 → `md5` 复核与注入前**逐字节相同**（L24/L26：补丁会静默不生效）。

| # | 注入 | 变红的判据（实测） | 还原复核 |
| :-: | :--- | :--- | :--- |
| I1 | `SynthEngine::render_track` 开头 `return;`（回到占位静音） | J1 `project_with_notes_produces_audible_samples`、J2、J3、J7、J10、**J5 覆盖度自检**（`没有任何非零样本`）+ 6 条单测 | md5 `ec42e6e6…` 一致 |
| I2 | `velocity_gain` 恒返回 `1.0`（忽略力度） | **只有** J4（`velocity_zero_is_silent_and_loudness_is_monotonic` + `velocity_gate_and_monotonic_loudness`），其余全绿 | md5 `ec42e6e6…` 一致 |
| I3 | `render_track` 每个量子把在鸣声部的相位重置为 0 | **只有** J7：`A4=752 A5=1504`（期望 ≈882.3）—— 绝对频率与倍频关系同时失效 | md5 `ec42e6e6…` 一致 |
| I4 | `align_cursors` 里把 `slot.cursor = 0`（切换时重触发） | **只有** J8（`rearming_an_equivalent_snapshot_does_not_retrigger` + 单测 `snapshot_realign_never_retriggers_consumed_notes`） | md5 `ec42e6e6…` 一致 |
| I5 | `render_block` 在汇流之后 `block.silence()`（母线级占位静音） | J1、J3、J4、J7、J10、J5 覆盖度自检 | md5 `95e85486…`（rt.rs）一致 |

**两条值得单独记下的读数**：

1. **I1 让 J2 变红，是因为 J2 里那句覆盖度护栏**（"对照渲染必须真的出声"）。
   如果没有那句话，"两次渲染逐位相同"在**全零**时也成立 —— 判据会变成永真。
   这是"判据的判据"，本线把它写进了 J2 本身。
2. **I3 让"倍频关系"也失效**，说明 J7 的两条断言（绝对值 + 倍率）都有判别力：
   若只写 `high == low * 2`，把 `inc` 整体除以 2（整体降八度）这种注入**不会**变红
   —— 绝对频率断言补上了这个盲区。

---

## 5. 明确没做 / pending / TODO

### 5.1 本切片明确**没有**做（都在 `src/synth.rs` 模块文档 §4 有对应说明）

| 项 | 为什么没做 | 落地位置（接入点） |
| :--- | :--- | :--- |
| **滤波器 / 音色参数** | `yeban-model` 里还没有"乐器参数"到音频线程的形状（`DeviceDefinition::params` 是字符串键值对，未投影进快照） | `SynthEngine::trigger` 里给声部加 `Filter` |
| **3 ms 声部窃取淡出** [ARCH-RT-004] | 硬窃取只需复用槽位；软淡出需要额外的"淡出声部"槽位（否则新音符会被淡出曲线污染） | `trigger` 里的池满分支 + `Adsr::start_steal_fade` |
| **母线限制器** [ARCH-DSP-001] | 本切片只做"出声"，压限属于母带切片；J1 的峰值 1.0058 就是它缺席的实测后果 | `sum_into_bus` 之后 |
| **声相定律** | `TrackParams::pan` 与 `audio_config.pan_law` **不进入快照**，母线汇流是等增益复制（等价声相居中） | `sum_into_bus` |
| **循环片段展开** | `ClipPlacement::loop_config` 被**忽略**（一个摆放只播一遍）。坐标语义（clip 局部 vs 时间轴）在规范里没有定义 | `project_schedules`（见 needs N1） |
| **滑音 / 弯音 / 歌词 / 音素** | `MidiNote::slide`/`pitch_bend_curve`/`phonemes` 不参与合成 | `project_schedules` |
| **采样播放 / SFZ** | 需要采样数据加载 + 声部池 + 重采样；`assets/samples/` 目前**只有** `ATTRIBUTION.md`（没有样本），且没有采样解码器 | 见 §5.3 |
| **走带控制（播放/暂停/定位）** | 播放头由引擎自己持有并**从 tick 0 起滚**；`MODEL-ISO-001` 禁止把它放进快照 | `EngineRuntime` 的 `position_samples()` / `SynthEngine::seek` |

### 5.2 已知口径（需要用判据读的人知道）

1. **每个渲染量子都从 tick 0 的绝对位置播放**：`synth.position()` 从 0 起每量子 +`frames`。
   因此"含 tick 0 音符的工程"在**第一个量子**就会出声 —— 这一点改变了
   `meter_rt_contract.rs` 的 S3 夹具（见 §7）。
2. **力度 0 的音符仍然进调度表**，由 `velocity_gain(0) == 0.0` 让它静音。
   这样"力度 0 不发声"是**增益路径**的判据，而不是"被调度器丢掉"的巧合。
3. **不可闻（静音/非独奏）轨仍然触发声部**，只是增益恒为 0 ⇒ 输出逐位 0。
   代价是白跑声部；收益是"静音 ⇒ 逐位静音"可以被端到端判据直接钉住。
4. **音符时值被裁剪到摆放区间** `[start_tick, start_tick + duration_ticks)`。
5. **`ratchet` 用整数除法等分时值**（余数不补）；`duration/ratchet` 为 0 时取 1 tick。
6. **快照切换不追溯**：切换前已经过去的音符不会被补触发（游标只增不减）。
7. **在鸣的音符不被切换切断**：仍在释放段的声部保留，已静音且过终点的声部回收。

### 5.3 SFZ 接口预留（**明确 pending，不在本切片交付**）

`yeban-sfz` 已经能解析乐器（`parse_text` / `parse_sources` → `Instrument::region_for`）
并持有预分配声部池（`VoicePool`，默认 512 / 上限 1024，含 `StealFade` 3 ms 淡出）。
把它们接到音源上需要**三个当前不存在的前提**：

```text
engine 侧需要的接口（预留形状, 尚未实现）:
  trait RegionSampleSource {              // 采样数据从哪里来
      fn sample(&self, region: &Region) -> Option<SampleRef>;   // ← 缺失: 无解码器/无数据
  }
  1) 采样数据加载 + 解码 (WAV/FLAC) —— 需要依赖裁决（symphonia/hound）或自研解码器；
  2) 重采样 (region.pitch_keycenter / tune / 采样率 ≠ 工程采样率) —— 需要 rubato 或自研；
  3) 数据本身: assets/samples/ 目前只有 ATTRIBUTION.md，**没有一个样本**
     （而且 AGENTS.md §2 红线 9 要求样本必须在 assets/manifest.json 登记许可证 + SHA-256）。
```

**接入点已经就位**：`SynthEngine::trigger` 是"选择一个声部并初始化它"的唯一位置；
把 `NoteSchedule` 的 `(pitch, velocity)` 换成"region → 采样播放"只需在那里分派，
`render_track` 的逐样本循环、电平、母线、零分配约束都不需要改。

---

## 6. 需要裁决 / 需要别人做的事（needs）

| # | 事项 | 为什么需要裁决 |
| :-: | :--- | :--- |
| **N1** | `ClipPlacement::loop_config` 的 `start_tick`/`end_tick` 到底是 **clip 局部**坐标还是**时间轴**坐标？循环与 `placement.duration_ticks` 的关系是什么？ | 规范只写"循环配置"，未定义坐标语义。本切片选择**不展开**（一个摆放只播一遍）以免发明语义 —— 但 `filled_project()` 的 `loop_config.enabled = true` 因此被忽略。需要一条裁决（写进 ADR）才能实现。 |
| **N2** | 引擎的**走带语义**：`process_quantum` 现在默认"从 tick 0 起滚"。播放/暂停/定位需要一条事件通道（`ring::EngineEvent` 的新变体？）与"未播放时输出静音"的契约 | 决定 UI 的播放按钮能不能真的控制引擎；也决定"工程一发布就出声"是否符合产品预期 |
| **N3** | `ARCH-RT-004` 的 3 ms 声部窃取淡出需要**第二个声部槽位**（淡出声部）还是"就地降级"？ | 涉及实时侧内存（每槽 16 声部 × 每声部 ≈ 90 B）与 polyphony 口径 |
| **N4** | 声相定律（`audio_config.pan_law`）与发送/辅助汇流：是否把 `pan_law` 投影进快照？ | `TrackParams` 已有 `pan`，但快照没有 `pan_law` ⇒ 无法按工程口径实现声相 |
| **N5** | 乐器/音色参数的形状：`DeviceDefinition::params`（字符串键值对）如何投影成音频线程可读的定长参数集？ | 没有它，合成器只能有一个内置波表音色（本切片即如此） |
| **N6** | SFZ 采样解码依赖（symphonia/hound）与重采样（rubato）的依赖裁决 + 样本资产的许可证/SHA-256 登记 | AGENTS.md §2 红线 2 与红线 9；见 §5.3 |
| **N7** | 跨架构 L1 对账：本线的逐样本路径全部是 IEEE 精确类，建议在 CI 上补"aarch64 vs x86_64 逐位对账"（现有 `frozen_level_table` 是电平线的同类判据） | 需要 CI 改动（**本线不动 `ci.yml`**，按纪律写进 needs） |
| **N8** | **CI 的"受影响集合"计划器认不出 `workspace = true` 形式的下游依赖** ⇒ 公共 API 的破坏性改动可能不被拦下 | 见下方 §6.1：本线实测复现，属于 `scripts/dev/changed-crates.py` **与 `.github/**`**（红线：本线禁改），必须由集成者处置 |

### 6.1 N8 的实测复现（`line/engine-sound` 推送后立刻发现）

推送后 CI 的 `plan (受影响集合)` 只选出 `["yeban-engine", "yeban-sfz"]`
（本地重跑 `python3 scripts/dev/changed-crates.py --base origin/main --head HEAD`
得到同一结果：`reason = "11 个文件改动, 命中 2 个 crate, 含下游共 2 个"`），
于是 `rust (yeban-app)` **没有进 matrix**，`rust (workspace 全量)` 也被 `workspace_wide=false` 跳过。

根因（读代码得出，不是猜）：

```python
# scripts/dev/changed-crates.py::dependents_of
if f'"{parent}/{crate}"' in text or f'path = "{parent}/{crate}"' in text:
```

它只在**成员自己的** `Cargo.toml` 里找内联的 `path = "crates/<name>"` 字面量。
而 ADR-0001 **D21** 之后，跨成员依赖的推荐写法就是

```toml
# crates/yeban-app/Cargo.toml
yeban-engine = { workspace = true }      # ← 路径住在根 [workspace.dependencies]
```

路径因此不在成员清单里，计划器**看不见这条边**。实测受影响的边至少有
`yeban-app → yeban-engine`、`yeban-app → yeban-model`、`yeban-engine → yeban-dsp`、
`yeban-engine → yeban-model`、`yeban-mcp/yeban-decode → yeban-model` 等等
（`grep -rn 'yeban-.*\.workspace = true' crates/*/Cargo.toml` 可复现清单）。

**本轮为什么没有出事**：本切片的公共 API 改动是**纯增量**的
（新增 `EngineSnapshot::with_schedules`/`schedules`/`schedule`/`scheduled_notes`/
`note_schedule_drops`、新增 `pub mod synth`、`EngineStats` 只**增加**字段、
`from_parts` 签名不变、删除的 `render_track_into` 是私有函数），
因此"没被 CI 编译到的下游"仍然是源码兼容的 —— 这一点由人工逐条核对
`crates/yeban-app/src/engine_host.rs` 与 `src/live_surface.rs` / `src/meters.rs`
的调用点确认（只用到 `from_project` / `tracks()` / `channels()` /
`block_size_matches_enum()` / `EngineRuntime::new` / `process_quantum` / `stats()`，
全部未变）。

**为什么必须修**：下一个改 `yeban-model` 或 `yeban-engine` 公共 API 的切片，
只要不是"纯增量"，就会在 **`rust (workspace 全量)` 被跳过**的情况下拿到一个全绿的判决 ——
这正是"本地绿/CI 绿不等于没坏"的另一种形态。处置选项（由集成者裁决）：
(a) `dependents_of` 直接读 `cargo metadata` 的 `resolve` 图；
(b) 或退一步：把 `workspace_wide` 改成"任一 engine/model 公共 crate 被改动即为真"。
本线**没有**动 `.github/**` 与 `scripts/**`（红线），只登记在此。

---

## 7. ⚠ 跨线影响：`meter_rt_contract.rs` 的 S3 夹具必须改（已改）

**发现**：`tests/meter_rt_contract.rs` 的 S3（"静音 ⇒ 峰值 0 / dBFS 负无穷 / 无 NaN"）
用 `engine_rig()` = `yeban_model::samples::filled_project()` 当"静音输入"。
那个前提**建立在渲染占位静音之上**：`render_track_into` 曾写 `out.fill(0.0)`，
所以"夹具工程的前 128 帧"恒为静音。真实合成接上之后，`filled_project` 的第 0 个音符合
好落在 tick 0 ⇒ 第一个量子即出声 ⇒ **S3 立刻变红，而红的原因不是电平口径坏了**。

**处置**：把 S3 的输入换成显式的 `silent_project()`（`filled_project` 去掉全部 MIDI 音符，
保留轨道/路由/片段）。判据的**意图**（静音输入 ⇒ 峰值 0、有限、无 NaN、dBFS 负无穷）
一个字没改，判别力反而更强：旧版测的是"占位渲染恰好静音"，新版测的是"真实合成链上的静音"。
S1/S2/S4/S5/S6/S7 全部原样保持绿。

**给电平线的提醒**：这条是"夹具隐含依赖了另一个模块的实现细节"的典型形态
（P9.7 同族）。凡是拿"填充工程"当静音/fixture 的判据，都要在语义变化后复核一次。

---

## 8. 边界声明（本线**没有**证明的东西）

1. **"位精确"只在同一架构同一工具链下**：含 `exp2`/`exp` 的构造期标量拒绝跨架构位精确
   （[ADR-0001 D32]），本线的逐样本路径虽然是 IEEE 精确类，但输入里含这些标量。
   跨架构对账（N7）**没有做**，也没有在 CI 上跑过。
2. **没有声卡**：全部判据都在 `--no-default-features`（不编译 cpal）下跑。
   默认 feature（含 cpal）在本机**禁止编译**（`yeban-engine` 含重依赖，`run-gates.sh crate` 会跳过），
   因此 `device.rs` / `NullBackend` 侧的改动**只由 CI 覆盖**。本切片没有改 `device.rs`。
3. **性能没有打点**：本线**没有**跑 benchmark（纪律禁止），因此
   "每量子 16 槽 × 16 声部的内层循环"的真实开销**未知**，只有"10,000 量子在本机能跑完"这一条
   （debug 构建，未计时）。BASELINE 打点属于 CI。
4. **声部池容量上限**（16 轨 × 16 声部）是**拍出来的**，没有按目标工程规模推导过；
   超过上限的轨道会被 `track_drops` 计数（判据只在单元测试里覆盖）。
5. **`unsafe`**：本 crate 不属于 `forbid(unsafe_code)` 名单；本切片新增的 `synth.rs`
   **没有引入任何 `unsafe`**（可 grep 复核）。
6. **依赖图未变**：本切片只用了已有的 `yeban-dsp`（`Wavetable`/`Adsr`/`math`）与
   `yeban-model`，**没有**新增任何依赖 ⇒ 不需要重跑 `license_inventory.py`（已跑，见 §9）。
7. **没有改 `yeban-dsp`**：任务书允许"缺什么加到 dsp 里"，但 dsp 属于别的工作线；
   本线选择把声部池与定点相位读数留在 `yeban-engine`（自己拥有的目录），
   只用 dsp 的**只读**原语。若后续三方（render / sfz）都要用，可把
   `SynthEngine` 的"整数相位 + 12 位插值"读数上移到 dsp，判据可以整块搬过去。

---

## 9. 本机跑了什么 / 没跑什么（严格区分）

| 命令 | 结果 |
| :--- | :--- |
| `bash scripts/dev/cargo-local.sh fmt --all` | ok |
| `bash scripts/dev/cargo-local.sh check -p yeban-engine --no-default-features --all-targets` | ok |
| `bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --all-targets` | **全绿**（89 单测 + 9 集成 + 3 个 `harness=false` 目标） |
| 5 次注入 + 还原（§4） | 全部按要求变红，还原后 md5 与注入前一致 |
| `bash scripts/gates/run-gates.sh light` | 绿（fmt + 13 条红线守卫 + 文档链接 + 依赖许可清单） |
| `run-gates.sh crate yeban-engine` | **未跑**：本机禁止（cpal 重依赖），交给 CI |
| `cargo test --workspace` / benchmark / fuzz | **未跑**（纪律禁止） |

### 9.1 CI 判决（**已读回**，不是 pending）

```text
run id  : 37243099566   （line/engine-sound，push 触发）
结论    : completed / success —— 所有被调度到的 job 全绿
  ✓ lockfile (确定性 Cargo.lock)           17s
  ✓ checks (fmt / 红线守卫 / schema)        28s
  ✓ plan (受影响集合)                        6s
  ✓ deny (cargo-deny 开源合规)              59s
  ✓ rust (yeban-engine)                    48s   ← clippy -D warnings + test，**默认 feature（含 cpal）**
  ✓ rust (yeban-sfz)                       31s
  - rust (workspace 全量)                        ← 被 plan 跳过（见 N8）
  - windows (yeban-mcp / yeban-model 平台分支)    ← 与本改动无关
```

读法：`rust (yeban-engine)` 用的是 `cargo clippy -p yeban-engine --all-targets --locked -- -D warnings`
与 `cargo test -p yeban-engine --all-targets`（工作流原文，**默认 feature**），
所以这一条同时给出了"默认 feature（cpal 在编）下 clippy 零告警 + 全部目标测试通过"的
平台判决 —— 本机无法编译的那一侧由它覆盖。**`workspace 全量` 被跳过**是 N8 的直接后果。
