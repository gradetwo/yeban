# ADR-0077 — SFZ `<effect>` 与效果总线：最小 DSP 集、发送语义、实时落点

- **状态**: **Proposed**（待人类批准；⛔ 本票**未实施任何 DSP**，只定方案与落点，理由见 §5）
- **决定者**: 待人类
- **关联**: `crates/yeban-sfz/src/effect.rs`（声明层，已登记但无 DSP）、
  `crates/yeban-sfz/src/instrument.rs`（`Region` / `FilterType` / 触发与混音面）、
  `crates/yeban-dsp/src/reverb.rs`（`Reverb` / `ReverbParams`，现成器件）、
  `crates/yeban-engine/src/sfz_instrument.rs`（`PreparedSfz` / `SfzVoicePool` / `mix_span_into`）、
  `docs/ledger/sfz-opcode-gaps-notes.md` §5.5（"声明了但不处理，现在会告警"的现状）

## 1. 背景（为什么这件事必须裁决，而不是"照规范抄"）

`<effect>` 头的规范原文自己写明：

> "The specifics of what can be used under this header **differ widely between SFZ players**."

即：**规范只给了"总线下有哪些 opcode"（`bus` / `type` / `dsp_order` / `effect1..4` /
`param_offset`），没有给"某个 `type` 该听成什么声音"**。同一条 `type=reverb`，在不同
播放器里可以是卷积、可以是 Schroeder、也可以完全忽略；`param1..paramN` 的编号在
ARIA 与厂商自定义之间也不通用（本仓的 `<effect>` 段**没有**建模 `paramN`）。

⇒ 这件事的正确形态是**自定一套可复现的最小语义并留痕**，⛔ 不是"补齐规范"。
本仓已有同类先例（ADR-0061 乒乓延迟：器件零改动、投影结构自定、缺省值逐条写死）。

现状（2026-11-19 实测）：
- `<effect>` 能解析、能读（`Instrument::effects()` 给 `bus` / `type` / `dsp_order` / `sends`）；
- **没有任何 DSP**（`grep -rn "reverb_\|chorus_" crates/yeban-sfz/src` = 0；引擎侧 0 消费）；
- region 级 `effect1..4`（v1 的发送量）**不读**；
- 本批已补的 `Warning::EffectNotProcessed` 让"声明了但不处理"**响亮**（不再静默），
  但它只是提示，⛔ 不是能力。

## 2. 决定

### D1 总线模型：4 条效果总线（`fx1..fx4`），`aux*`/`main`/`midi` 不建 DSP

- `<effect bus=fxN>`（N=1..4）＝ 第 N 条总线的效果器声明；`dsp_order` 升序串成链，
  同 `dsp_order` 保持文件出现顺序（确定性，与 `BTreeMap` 口径一致）。
- `bus=main`：**不建独立 DSP**（主输出就是干声混音；对主输出整体加效果属于"轨道插入链"
  的领域，见 `yeban-engine` 的 `insert.rs`，⛔ 不由 SFZ 声明越权创建）。
- `bus=auxN`：登记为"未实现但已告警"（规范里 aux 的用途是"辅助发送"，与本票的 fx 同构，
  但本仓 0 语料使用，先不建）。
- `bus=midi`：维持现状（= ARIA `<midi>` 段头的等价声明，与 DSP 无关）。

### D2 发送语义：region 级 `effect1..4` = 对 `fx1..fx4` 的发送量（0..100 %）

- 与规范 v1/v2 的表格一致（`effectN` Type float，Default 0，Range 0..100，Unit %）。
- **干声不被扣减**：发送是"额外复制一份到总线"，⛔ 不是"把干声按比例挪走" ——
  这一点必须写死，否则同一个 patch 在不同播放器里干湿比会差一倍（本仓取
  "send = additive" 口径，与多数玩家一致）。
- `send = 0` ⇒ **逐位不变**：逐样本路径上不加任何浮点运算（`if send > 0.0` 才进总线拷贝，
  且该分支在 `mix_span_into` 外层、不在逐样本循环内）。
- 总线输出电平 = 干声 + Σ(总线湿声)。总线湿声本身**不再**乘发送量（发送量已在入总线时乘过）。

### D3 最小 DSP 集：先只做 `reverb` 一种，其余 `type` 一律"旁路 + 告警"

| `type` 的取值（大小写不敏感，去空白） | 处理 |
| :--- | :--- |
| 含 `reverb` 的子串（`reverb` / `com.mda.Reverb` / `Reverb2` …） | 建一台 `yeban_dsp::reverb::Reverb`（参数走 D4 的缺省） |
| 其余一切（`chorus` / `delay` / `eq` / `filter` / 厂商自定义） | ⛔ 旁路，并产一条 `Warning::EffectTypeNotSupported { line, bus, type }`（新增变体） |

理由：`Reverb` 是本仓唯一**已经过判据**的成品器件（Freeverb 拓扑、`ReverbParams` 有
`size/damp/mix/width/predelay/kind/decay_s` 七个可复现参数）；`chorus` 需要"调制延迟线"
新器件（`delay.rs` 是固定延迟，不具 LFO 调制面），`eq`/`compressor` 在器件层有
（`shaping` / `compressor`），但它们在 SFZ 侧的参数映射（`paramN`）本仓未建模 ⇒
先做一台、跑通总线，再按同一形状加第二台。⛔ 一次铺四台 = 四套未验证的参数映射。

### D4 参数：**不读 vendor `paramN`**，`reverb` 用固定缺省，唯一可调量是发送量

- 本仓 `<effect>` 段未建模 `param1..paramN`（`effect.rs` 的登记表里没有它们），
  ⇒ 任何"按 param 编号映射到 `ReverbParams`"的方案都是**发明**，本票不做。
- 缺省值（写死，可复现）：`ReverbParams { size: 0.45, damp: 0.35, mix: 1.0, width: 0.8,
  predelay: 0.012, kind: Room, decay_s: 0.0 }`。⚠ 与 `ReverbParams::default()` 的差别只有
  `mix`：器件缺省 0.25 是"通道条里干湿混合"语义，总线上要的是**纯湿**（干声已在主输出），
  ⇒ `mix = 1.0`。其余六个字段与器件缺省逐字相同。
- 采样率：装载期按工程采样率调 `Reverb::set_sample_rate`（分配在装载期，⛔ 不在回调里）。

### D5 实时落点：`mix_span_into` 增加"可选总线目标"，`send=0` 时逐位不变

- `PreparedRegion` 新增 `sends: [f32; 4]`（`Region::effect_sends` 的拷贝；缺省全 0）。
- `SfzVoicePool` 新增 `buses: [BusState; 4]`：
  - `BusState { buffer: Vec<f32> /* 容量 = 最大量子帧 */, wet: Vec<f32>, reverb: Option<Reverb>, active: bool }`；
  - 缓冲与器件在**构造期**分配（`Vec` 可以，构造在控制线程），回调里只做 `clear` + 乘加；
  - 4 条总线各自独立，未使用的总线 `active=false` ⇒ 不分配、不进循环。
- 逐块序：① 清总线缓冲 → ② 逐声部：干声进 `out`（与今天**同一段**代码），若该声部
  `send > 0` 再把它**同一批采样**按 `send` 混入对应总线缓冲 → ③ 每条 `active` 总线跑
  `Reverb::process` → ④ 湿声加进 `out`。⚠ ②③④ 都在"声部全部渲完之后"，**不改变**逐声部
  的既有递推与退休判定。
- **逐位不变的证明义务**：任何 `sends` 全 0 的 plan，输出必须与接线前逐位相同
  （判据 `S-bus-0`：同一份夹具、同一块长度、`to_bits` 全等）。

### D6 离线等价性：同一份"总线归约"两处入口

`SfzInstrument::render_note_into_with_cc`（离线一次性）与 `SfzVoicePool::render_into_with_cc`
（实时）必须走**同一个** `apply_buses(...)` 函数（"⛔ 不许两份实现"是本仓机械纪律）。
离线面的一次性入口只处理"这一次 note"的 region 发送；总线状态在本次调用内自建自毁
（离线没有跨块延续，⛔ 不假装有）。

### D7 判据（落地时必须全绿）

| 编号 | 判据 | 钉住什么 |
| :--- | :--- | :--- |
| `SFX-1` | `bus_send_zero_is_bit_exact` | `sends` 全 0 ⇒ 输出 `to_bits` 与接线前全等 |
| `SFX-2` | `reverb_bus_audible` | `send=100` 的脉冲：总线输出在 note 结束后仍有拖尾（能量 > 干声） |
| `SFX-3` | `unsupported_type_is_loud` | `type=chorus` ⇒ 旁路 + `EffectTypeNotSupported` 告警，⛔ 静默 |
| `SFX-4` | `dsp_order_sorts_chain` | 两条 `fx1` 声明按 `dsp_order` 排序（同序保持文件顺序） |
| `SFX-5` | `engine_runtime_memory_budget` | 追加后 `EngineRuntime` 仍 ≤ 196608 B（重测并更新钉数） |
| `SFX-6` | `buses_do_not_leak_across_blocks` | 上一块的总线残留不清零 ⇒ 红（`clear` 必须在每块开头） |

### D8 内存与性能预算（必须实测后回填）

- 每轨 4 条总线 × 128 帧 × 2 个 `Vec<f32>` ≈ 4 KB；`Reverb` 内部缓冲（8 梳 + 4 全通 × 2 声道，
48 kHz 下约 100–200 KB/台）**是堆分配**，计进"每轨常驻"。
- ⇒ 8 轨 × 1 台 reverb ≈ 0.8–1.6 MB 常驻；`EngineRuntime` 的**内联**尺寸只增
  `Option<Reverb>` 的指针 + 两个 `Vec` 头 ≈ 每轨 4×(24+24)+… ≈ 数百字节（**不**是 KB 级）。
- ⚠ 必须重测 `size_of::<EngineRuntime>()` 与堆常驻，并把读数写回本节（⛔ 不写"应该没问题"）。

## 3. 落点清单（按依赖顺序，实施时逐条打勾）

1. `yeban-sfz/src/instrument.rs`：`Region::effect_sends: [f32; 4]`（解析 `effect1..4`，0..100，
   `read_f32` 同族；`effect1` 在 v1 语义下是 region 级 —— 与本仓"`<effect>` 段不写继承链"的
   既有裁决**不冲突**：段级 opcode 与 region 级同名 opcode 语义不同，`effect.rs` 模块头已登记）。
2. `yeban-sfz/src/playback.rs`：`PlaybackSpec::sends`（离线面同源）。
3. `yeban-sfz/src/parser.rs`：新增 `Warning::EffectTypeNotSupported { line, bus, kw_type }`；
   `finalize_effect` 里按 D3 分类：可处理 ⇒ 只留 `EffectNotProcessed` 的旧文案改成"will be
   processed in this build"不再成立 ⇒ 改为**仅对不支持的类型**告警（旧变体改名或改义，
  需在同一提交里改两处判据的期望，见补丁 `0002` 的同类做法）。
4. `yeban-engine/src/sfz_instrument.rs`：`PreparedEffect { bus: u8, order: i64, kind: ReverbKind }`、
   `PreparedRegion::sends`、`PreparedSfz::buses`、`SfzVoicePool::buses`、`apply_buses()`、
   `mix_span_into` 增加 `bus: Option<&mut [f32]>` + `send: f32` 两个参数（`None / 0.0` 时行为逐位不变）。
5. 判据：`crates/yeban-engine/tests/sfz_effect_bus.rs`（SFX-1..6）＋ `yeban-sfz` 的解析判据。

## 4. 备选方案（明确否决的理由）

| 备选 | 否决理由 |
| :--- | :--- |
| A. 把 `<effect>` 映射到轨道插入链（`insert.rs`） | 一条 SFZ 文件不该改写工程结构；插入链是**工程状态**（可撤销、可序列化），SFZ 是**音源内部**。混在一起会让"打开一个 sfz"产生 Undo 步 |
| B. 只对 `<effect bus=main>` 加效果（跳过发送） | `bus=main` 是"主输出"不是"效果链"，给主输出整体加混响等于给整个工程加混响，语义错误 |
| C. 一次铺 `reverb`/`chorus`/`eq`/`compressor` 四台 | 四套未验证的 `paramN` 映射（本仓未建模）＝四份发明；且 `chorus` 需新器件。先一台跑通总线，按同一形状加第二台 |
| D. 读 `param1..paramN` 映射到 `ReverbParams` | 编号与含义跨播放器不一致，本仓无该表 ⇒ 属"凭空发明"，违反 `AGENTS.md` §5.7 |

## 5. 为什么本票**只到 ADR**（⛔ 没写代码）

`AGENTS.md` §5.7：**规范缺口不得私自发明答案，裁决写进 ADR 标 `Proposed`，由人类批准**。
本票的 D2/D3/D4 都是"规范没给、必须自定"的量（发送是否扣干声、缺省 mix、支持哪些 type），
⛔ 不是照抄能得到的。因此先把裁决写死，再由实施票按 D1–D8 落地 —— 这与 ADR-0061 的
"先把缺省值写死再接线"同一条路径，只是本票把范围收得更小（一台器件、四类总线里的两类）。

⚠ 已落地的**无争议**部分（上一批）：`<effect>` 声明现在会产 `Warning::EffectNotProcessed`
（响亮、可读、进 `yeban_load_sfz` 的 `warnings`）—— 它把"静默"这一条先关掉，
本票负责"有声音"那一条。

## 6. 未决点（需人类一并裁决）

1. **发送是否扣干声**（D2 取 additive）：若是 subtractive（干声 = 1 − send），老 patch 的
   干声电平会整体变小，属**可听的行为变更**。
2. **`mix = 1.0` 的纯湿总线**是否合适（D4）：另一条路是让总线 `mix = 0.5` 并**不**加干声，
   等价但少一次拷贝；取纯湿是为了"发送量 = 唯一可调量"这条口径干净。
3. **`aux*` 要不要一并建**（D1 仅 fx）：语料 0 使用，但规范列了 8 条。
4. **是否允许未来的 `type` 白名单扩张**（D3）：每加一台器件都要新增一条判据与 ADR 附注。

## 7. 实施记录（2026-11-19）

### 7.1 已落地

* `effect1..4` → [`Region::effect_sends`] / [`PlaybackSpec::effect_sends`]（判据
  `crates/yeban-sfz/tests/effect_sends.rs` 5 条）。
* `Unsupported vs supported` 的判据函数：`type` 含 `reverb`（大小写不敏感）为支持集。

### 7.2 D5/D6 首轮尝试：三条硬约束（实测，不是推测）

把 `BusBank`（4 条 `Vec` 累积 ＋ 4 台 `Reverb`）放进 `SfzVoicePool` 时，编译期依次
撞到三堵墙：

1. **`Copy` 派生链**：`SfzVoicePool` 现在是 `#[derive(Clone, Copy, Debug, PartialEq)]`，
   而 `Reverb` 持有 `Vec`（无任何派生）、`BusBank` 因此不是 `Copy` ⇒ 池的 `Copy`
   必须去掉 ⇒ `synth.rs` 的 `TrackSlot`（同样 `Copy`）跟着断，`TrackSlot` 在 `synth.rs`
   里按值取用的地方成片改引用。
2. **分配点**：`ADR-0077` D8 原写"构造期分配"，但池构造时**拿不到** `PreparedSfz`
   （表是每次渲染传进来的），且 `SfzVoicePool::new` 在 `TrackSlot::empty`（控制线程、
   无表）里跑 ⇒ 构造期分配不可行；改"首次渲染分配"就把一次 `Vec` ＋
   `Reverb::set_sample_rate` 的分配放进音频回调，违 [MUST-GATE-001]。
3. **`EngineRuntime ≤ 196608 B`**：`rt.rs` 的尺寸判据钉死；把总线池（`Vec<Option<Box<BusBank>>>`
   或内联缓冲）塞进运行时，要么触发重钉，要么重钉后立刻怀疑"这个数是量出来的还是让出来的"。

### 7.3 结论：D5/D6 需要一次**结构性改动**，不能顺手加

两条可行路线（都需要单独一批、单独评审）：

* **方案甲（推荐）**：去掉 `SfzVoicePool` / `TrackSlot` 的 `Copy`，把 `BusBank` 放池里，
  构造改在 `TrackSlot::bind`（拿到表之后、控制线程）而不是 `empty`。
  代价：`synth.rs` 的按值取用改引用（机械但面积大），`EngineRuntime` 尺寸重钉。
* **方案乙**：`EngineRuntime` 持有**每轨总线池**，`render_into_*` 增加
   `bus: Option<&mut BusBank>` 参数（池不持有 DSP 状态，`Copy` 保全）；
  代价：渲染签名穿过 `synth.rs` 的 4 个包装，实时侧每块多一次字段借出。

⇒ 本票仍为 **Proposed**：装载半边已落地，DSP 半边等上面二选一（`HD-73` 的 A
   分支里也请一并选甲/乙）。**2026-11-19 负责人指示按方案甲继续实施 ⇒ 见 §7.4。**

### 7.4 D5/D6 按方案甲落地（2026-11-19 第二轮）

#### 7.4.1 改了什么（逐处）

* **去掉 `Copy`**：`SfzVoicePool` 改 `#[derive(Debug)]`；`synth.rs::TrackSlot` 改
  `#[derive(Debug)]`；`vec![TrackSlot::empty(sr); n]` 改
  `(0..n).map(|_| TrackSlot::empty(sr)).collect()`（`slots` 与 `group_slots` 两处）。
  ⇒ `EngineRuntime` 尺寸**未变**（读数见下），只是"按值取用"改成"原地借用"。
* **`BusBank`**（`sfz_instrument.rs` 文件末尾）：`frames` ＋ `dry[4]`/`wet[4]`
  两条缓冲 ＋ `verbs: [Option<Box<Reverb>>; 4]` ＋ `orders: [Option<u8>; 4]`。
  每台混响 `Reverb::new() → set_sample_rate(hz) → set_params(ReverbParams{mix:1.0,
  ..Default})`（全湿口径：干声由发送侧另行叠加，⛔ 不扣干）。
  `apply`：按 `orders` 升序，`wet.copy_from_slice(dry)` 后 `reverb.process`，
  `out += (L + R) * 0.5` —— **单声道口径**与 `insert.rs §8.3` 同款（本仓不做第二份归约）。
* **加性发送**：`mix_span_into` 新增第 17 参 `bus: Option<&mut BusSend<'_>>`；
  逐样本 `voice_out` 先写干路，再对四条总线 `target[written] += voice_out * send`
  （`send` 已在摊平期折成 `0..=1`）。`send == 0` ⇒ 该分支一次都不进 ⇒ **逐位不变**。
* **发送源**：`PreparedRegion.effect_sends: [f32; 4]`（摊平期从 `Region` 搬）。
* **器件表**：`PreparedSfz.buses: [Option<BusEffect>; 4]`，由 `bus_effects()` 从
  `<effect bus=fx1..fx4 type=…reverb…>` 收集（`dsp_order` 最小者胜）。
* **预占 ＋ 预热**：`SynthEngine::reserve_track_slot(sample_rate, track)` 先在控制线程
  占好槽（与快照边界同一套槽位口径，但 ⛔ 不动 `present`／音色／鼓机），
  `SfzVoicePool::warm_buses(plan, frames)`（控制线程）；
  `EngineRuntime::warm_sfz_buses(track, plan, frames)` 把它转给某轨的槽；
  **宿主已接线**：`EngineHost::reload` 与 `publish_project` 都在发布快照后、
  播放前逐轨预热（`engine_host.rs`）。`synth.rs` 渲染入口另有兜底
  （`frames`／器件表未变时**严格空操作**，不会逐块重建）。
* **告警口径收紧**（D3 修订）：`fx1..fx4` ＋ `type` 含 `reverb` ⇒ **不再**
  `Warning::EffectNotProcessed`；其余（`main`/`aux*`/`midi`、非 reverb）**仍然响亮告警**
  （判据 `sfz_effect_bus.rs::an_unsupported_type_stays_dry_and_still_warns` 钉住）。

#### 7.4.2 判据与实测读数（`crates/yeban-engine/tests/sfz_effect_bus.rs`，4/4 绿）

| # | 判据 | 读数 |
| :--- | :--- | :--- |
| SFX-1 | 全 0 发送 vs 无 `<effect>` | 逐样本 `to_bits` **完全相同** |
| SFX-2 | `effect1=100` ＋ `fx1 type=reverb` | 干声 3 块后 RMS `< 1e-6`；湿声 7..12 块最大 RMS `> 1e-4` |
| SFX-3 | `fx1 type=chorus`（不支持） | 输出逐位等于干声 ＋ `EffectNotProcessed` 仍在 |
| SFX-6 | 缓冲每块清零 | 80 块内：20..40 块峰值 `> 1e-4`，72..80 块 `< 0.5 × 峰值` |

⚠ **SFX-2 的关键读数不是"接线失败"**：混响的尾巴有 ~1800 帧物理延迟
（预延迟 `0.012 s` ＝ 576 帧 ＋ 最短梳状线 ≈ 1200 帧）⇒ 只看前 6 块必然读到 0。
判据已按真实延迟取窗口，并在注释里写清这条，⛔ 不是把阈值调软。

#### 7.4.3 尺寸读数（`size_of`，48 kHz）

`PreparedSfz` **320 B**（＋8：四槽器件表）、`PreparedRegion` **6328 B**（＋16：
四路发送）、`SfzVoicePool` **10520 B**（＋8：`Option<Box<BusBank>>`；＋8：判据仪器 `bus_builds: u64`）、
`EngineRuntime` **47888 B**（未变，余量 156912 ≥ 8192）。
`bus_insert.rs` 与 `sfz_instrument.rs` 里 7 处尺寸钉已同步（每处都写明＋B 的来源）。

#### 7.4.9 D12 运行期半边：CC 每块热改旋钮（不重建）

装载期只读 `<control>` 初值；**运行期**每块按 `cc` 探针读 `offset+0/1/3` 并热改
`ReverbParams`：⛔ 不重建 `BusBank`、⛔ 不换缓冲、⛔ 不分配。判据：

| # | 判定量 |
| :--- | :--- |
| A01 | 16 块后把 `size` 从 0 推到 127 ⇒ 26..32 块尾巴 `> 1e-4`（用户能听到变化） |
| A02 | 同一轮里 `bus_builds` **保持 1**（零重建） |
| A03 | CC 抖动 100↔101（一个步长内）⇒ `bus_param_writes ≤ 1`（迟滞） |
| A04 | `fx1` 基址 0、`fx2` 基址 4 ⇒ 只动 CC4 时 `fx2` 独自响应 |
| A05 | CC 值 255 ⇒ 钳到 1.0，输出全有限、峰值 ≤ 1.0 |
| A09 | 两条同 `dsp_order` 声明 ⇒ 先出现者胜（`offset()` 可读） |

⚠ **仍是 Proposed**：`bus_param_writes` 这类计数器是判据仪器（⛔ 不参与音频路径），
上线时若要"零仪器"构建可加 `cfg(test)` 收口。

#### 7.4.8 D12（**Proposed**，待人类确认）：`set_ccN` → 三个混响旋钮

**口径**（本片按此实现）：`param_offset` 是旋钮基址，`<control>` 的 `set_ccN`
初值 `0..127` 归一成 `0..1`：

| 槽 | CC 下标 | 落到 | 范围 |
| :--- | :--- | :--- | :--- |
| 0 | `offset+0` | `size`（房间/衰减） | `0..1` |
| 1 | `offset+1` | `damp` | `0..1` |
| 2 | `offset+2` | `width` | **惰性**（单声道总线，听不出，只登记不生效） |
| 3 | `offset+3` | `predelay` | 归一后乘 `0.1 s` |

**`mix` 不开放**：总线必须纯湿（`mix=1.0`），否则处理后的缓冲里含干声，
`apply` 会把干声再加回主路 = 双重干声。`param_offset` 缺省 `0`。
没配过的槽 ⇒ 既有固定缺省（与会话前的输出**逐位一致**，判据 SFX-25 的对照腿）。

⚠ **这是设计裁决（映射表是本项目拟的，非 ARIA 官方表），标 Proposed**：
若人类裁决要求对齐 sfizz 的 `param_offset` 语义或换槽位，只需改
`bus_effects` 的三行 `knob(..)` 与 `BusBank::new` 的赋值，判据 SFX-22..27 一并改写。

#### 7.4.7 D11（已实现，告警裁决）：写了发送量但没有那条总线

`effectN > 0` 而整份文件没有 `bus=fxN` 的 `<effect>` 声明时，发送**永远到不了
任何器件**——干声照常响，用户听不出差别。这是"默认不能静默"规则的典型命中：
新增 `Warning::EffectSendWithoutBus { line, bus }`，按**总线号去重**（同一份文件里
缺 `fx2` 只报一条），行号取首次引用它的 region；`bus=fxN` 声明了但**类型不支持**
（如 `chorus`）走既有 `EffectNotProcessed`，⛔ 不叠加第二条。判据 SFX-12。

同总线多条声明的裁决（D11 的另一半）：**`dsp_order` 最小者胜**，同序取**先出现**者；
`PreparedSfz::bus_effect(i)` 把它变成可读数（SFX-13）。`dsp_order` 在"每条总线
只有一台器件"的现实下仍不可从音频听出差别 —— 这条只是把裁决钉住，不是功能。

#### 7.4.6 D10（已实现，行为裁决）：未认识的 `bus=` 不静默改写

`EffectBus::from_value` 把不认识的值（如 `fx5`、`aux9`）归约成 `main` —— 这是
**解析期**的容错口径（不拒载）。但告警文本必须用**原始** `bus=` 文本：否则
"作者写了 `fx5`"会被静默改写成"`main`"，用户按告警去改也改不对。
实现：`Effect::bus_raw()` 保留原文，`finalize_effect` 优先用它；判据 SFX-11。
⭐ 注意 `effect.bus()` 的归约行为**没有改**（`main` 仍是不支持侧），本片只修告警诚实性。

#### 7.4.5 D9（Proposed，待人类确认）：立体声腿的归约口径

**裁决内容**（本片按此实现，判据 SFX-8）：进总线 = `(L + R) / 2`（每声道各送
`0.5 × send`，累加即归约）；出总线**只加湿声**、同一份湿声**等量**加回左右；
干声一个样本不动。

**为什么是这个口径**（而不是 ①只取 L、②按能量相加、③两声道各自过一台混响）：

* ① 只取 L ⇒ 声相偏右的源，尾巴跟着偏左，听起来"混响跑到另一边"；
* ② `L + R` ⇒ 居中源会得到 2 倍激励，尾巴比单声道主路响 6 dB，同一工程切
  单/双声道输出会变响度；
* ③ 两套混响 ⇒ 尾巴自身带立体声、成本 ×2，且与单声道主路听感分叉。

`(L+R)/2` ＋ 等量加回 = 尾巴居中、与单声道主路**同一电平**，且 ⛔ 不动干声
⇒ `send = 0` 逐位不变（SFX-1 的立体声版仍然成立）。

⚠ **这是设计裁决，本片只实现、不自签**：若人类裁决改成别的口径，只需改
`apply_stereo` 与两处 `send_for_scaled(…, 0.5)`，判据 SFX-8 一并改写。

#### 7.4.4 明确**没做**的（登记，不静默）

1. **`dsp_order` 排序**：每条总线本片只接一台器件 ⇒ 排序位**不可观测**，⛔ 未落地判据。
2. **`aux*`/`main`/`midi` 总线**：D1 已否决，继续**只告警**。
3. **`paramN`**：D4 决定不读 ⇒ 混响参数是固定缺省。
4. ~~立体声腿不接总线~~ ⇒ **已接**（见 §7.4.5 / D9）。判据 SFX-8 已从
   "缺口钉"翻成**功能判据**：`effect1=0` 两声道逐位不变；`effect1=100` 两声道
   都在 7..12 块出现尾巴，且居中源的左右尾巴逐位相同。
   ⚠ 仍不接：**离线腿**（`mix_span_into` 的调用点仍传 `None`）——离线渲染
   没有"两声道"这个上下文，要接得先定离线的声道口径，属下一片。
5. ~~首次渲染块的兜底预热仍可能分配一次~~ ⇒ **已清零**（本片）：`EngineHost::reload`
   / `publish_project` 在发布后先 `reserve_sfx_slot`（每条轨）再 `warm_sfz_buses`；
   判据 SFX-9 钉住"渲染**之前**总线已建好、80 块零重建"，并有反腿（不预占 ⇒ `None`）
   证明仪器有判别力。
   ⚠ 残留事实（⛔ 不粉饰）：这只覆盖**轨槽**。组槽（`group_slots`）与 SFZ 无交集，
   不在本片射程；`begin_snapshot` 里**其它**一次性分配（若有）需另立判据才能声称清零。
