//! **每轨插入器件**的引擎侧接线：把 `TrackV3.devices` 里的内置效果器投影成
//! 音频线程可执行的**通道条**（`yeban_dsp::channel_strip`，内含 EQ ＋ 滤波 ＋ 压缩）。
//! [ARCH-RT-001, ARCH-DET-001]
//!
//! ## 1. 规范出处（原文引用）
//!
//! | 出处 | 原文 |
//! | :--- | :--- |
//! | `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:411` | `\| **内部 DSP 拓扑调度** \| - \| **1.00 ms** \| 声部合成、通道条 EQ/压缩与 PDC 延迟线插入计算 \|` |
//! | `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:109` | `- 通道条算法 ➔ crates/yeban-dsp/src/channel_strip.rs（EQ、滤波、动态旁通链）` |
//! | `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:108` | 压缩器算法的目的地 `crates/yeban-dsp/src/compressor.rs`（通道条的动态级复用同一份实现） |
//!
//! 器件**早就实现并自带判据**，缺的只有一件事：**引擎侧没有任何调用点**
//! （核实方式：`grep -rn 'use yeban_dsp' crates/yeban-engine/src` 在接线前只命中
//! `envelope` / `filter` / `math` / `oscillator` / `polysynth` / `limiter` / `meter`）。
//! 本模块补的就是那条调用点。
//!
//! ## 2. 与上一轮（`c792fdc`，每轨压缩器）的关系：**本文件是同一个模块的第二件器件**
//!
//! `c792fdc` 把**压缩器**接成每轨插入（一件器件、一条链、一个定长槽位表）。
//! 本票把**通道条**接进**同一条链**，做法是同一套形状的**扩展**（不是第二套接线）：
//!
//! 1. [`InsertParams`] 里原来只有 `compressor`；本票把它改成
//!    **一个** `Option<ChannelStripParams>`（`ChannelStrip` 内部**本来就含**动态级，
//!    见 `crates/yeban-dsp/src/channel_strip.rs` 的模块注释 §2 第 ④ 级）。
//!    ⇒ 一条轨**至多一件**插入器件，实时侧的"逐轨查找 + 整段跳过"骨架与
//!    槽位表形状**一个字没改**。
//! 2. 只有压缩器参数的设备仍然**只有动态级工作**：投影把它折成
//!    `eq_enabled = false` / `filter_enabled = false` 的通道条，其余字段取
//!    [`ChannelStripParams::DEFAULT`] ⇒ 输入/输出增益 `0 dB`、链路其余部分是旁通。
//!    ⇒ 这条路径的输出与 `c792fdc` 的裸压缩器**逐位相同**（判据
//!    `tests/compressor_insert.rs` 的 C1/C4/C5 原样保留并继续钉住它）。
//! 3. 出现 EQ / 滤波旋钮名的设备 ⇒ 那些级**才**被启用。
//!
//! ## 3. 为什么不是"器件一存在就生效"
//!
//! 模型层的 `DeviceDefinition` 只有 `kind`（`InternalEffect`）说明"这是一个内置效果器"，
//! **没有**"这是通道条"的类型级信息（该缺口是 `docs/ledger/engine-mix-notes.md` §8.2 的
//! **N5**：`DeviceDefinition::params` 没有参数名规范）。
//! 因此"有 `InternalEffect` ⇒ 挂通道条"会让**任何**内置效果器（例如只为上报延迟而存在的
//! 那一台）突然开始 EQ 与压限 —— 那是把未知器件猜成通道条。
//!
//! 本模块沿用**同一仓库已有的**投影规则（[`crate::synth::ToneParams::from_devices`] 的
//! 形状：只认 `InternalInstrument` 设备的 `cutoff_hz`/`resonance`/`drive` 三个约定名）：
//! **只有当设备的 `params` 里真的出现通道条参数名时，它才是通道条来源**。
//! 于是"没有参数的效果器"与"没有效果器"在音频上同解 ⇒ 既有工程逐位不变。
//!
//! ## 4. 投影规则（完全确定、无猜测）
//!
//! 1. 只看 [`DeviceKind::InternalEffect`] 的设备（外部效果器由插件宿主负责，
//!    `External*` 一律忽略；内置乐器是**音源**，由 [`crate::synth`] 处理）；
//! 2. `bypassed` 的设备整体忽略（旁通就是旁通，不"取默认参数偷偷接回来"）；
//! 3. 在**第一个**含**至少一个**已识别参数名的设备上取值；更早的、一个已识别参数都没有的
//!    效果器不是来源 ⇒ 继续往下找；
//! 4. 未出现的字段保持 `yeban_dsp` 的默认值：
//!    - 增益类（`input_gain_db` / `output_gain_db`）⇒ `0 dB`（数学恒等 `x · 1.0`）；
//!    - EQ 字段 ⇒ `EqParams::default()`（**平坦**，三个增益 `0 dB`）；
//!    - 滤波字段 ⇒ [`FilterParams::DEFAULT`]（截止 `20 kHz`、无谐振、无驱动）；
//!    - 动态字段 ⇒ [`CompressorParams::DEFAULT`]（与 `c792fdc` 的裸压缩器**同一个常数**）；
//! 5. 同一设备内同名参数**后者胜**（与 `ToneParams::from_devices` 同口径）；
//! 6. 参数名大小写不敏感（`to_ascii_lowercase` 后比较）；
//! 7. 全部取值经 [`ChannelStripParams::sanitised`] 钳到合法域
//!    ⇒ 快照里存的就是音频线程将要用的那一份数（`NaN` 也有定义）；
//! 8. **一个已识别参数都没有 ⇒ [`InsertParams::is_empty`]**，实时侧整段跳过。
//!
//! ### 4.1 三级的启用开关（默认口径 = "没写的级不执行"）
//!
//! 通道条的 EQ 与滤波级**不是逐位恒等**变换：
//! `crates/yeban-dsp/src/channel_strip.rs` 的模块注释 §3 实测"平坦 EQ 仍有
//! `4.566e-5` 的末位舍入"，§6 也说明 `20 kHz` 低通仍有增益。因此
//! **"沉默地启用一条平坦 EQ"会改变输出**（这是听得见与逐位的双重改变）。
//!
//! 本模块的口径是：**只有写出了该级旋钮的设备才启用该级**。
//!
//! | 设备写了什么 | `eq_enabled` | `filter_enabled` | `compressor_enabled` |
//! | :--- | :--- | :--- | :--- |
//! | 一个通道条参数都没有 | ——（整轨不武装，判据见规则 8） | | |
//! | 只有 EQ 旋钮或无（见下） | `true` | `false` | `false` |
//! | 只有滤波旋钮 | `false` | `true` | `false` |
//! | 只有动态旋钮（`c792fdc` 的既有形状） | `false` | `false` | `true` |
//! | EQ ／ 滤波 ＋ 动态 | `true` | `true` | `true` |
//! | `eq_enabled = true` 写死（无 EQ 增益旋钮） | `true` | —— | —— |
//!
//! ⚠ 由此得到一个**明说的**取舍：一条只写 `eq_low_gain` 的设备会让**平坦**的中/高频
//! 双二阶也开起来（滤波器是三级串联的，`ShapingEq` 只有整级开关）。这与
//! `yeban_dsp::channel_strip` 的 `eq_enabled` 语义一致（该级**整级**执行或整级跳过）。
//!
//! ### 4.2 已识别的名字（每项列出全部别名）
//!
//! | 目标字段 | 名字（全部别名） |
//! | :--- | :--- |
//! | `input_gain_db` | `input_gain_db`、`input_gain`、`trim_db` |
//! | `eq_enabled` | `eq_enabled` |
//! | `eq.low_gain` | `eq_low_gain`、`low_gain` |
//! | `eq.low_freq` | `eq_low_freq`、`low_freq` |
//! | `eq.mid_gain` | `eq_mid_gain`、`mid_gain` |
//! | `eq.mid_freq` | `eq_mid_freq`、`mid_freq` |
//! | `eq.mid_q` | `eq_mid_q`、`mid_q` |
//! | `eq.high_gain` | `eq_high_gain`、`high_gain` |
//! | `eq.high_freq` | `eq_high_freq`、`high_freq` |
//! | `filter_enabled` | `filter_enabled` |
//! | `filter.cutoff_hz` | `cutoff_hz`（⚠ **没有** `filter_cutoff_hz` 这个别名，理由见 §4.3） |
//! | `filter.resonance` | `resonance`、`filter_resonance` |
//! | `filter.drive` | `drive`、`filter_drive` |
//! | `compressor_enabled` | `compressor_enabled` |
//! | `compressor.*` | `c792fdc` 已登记的七个名字及其别名（`threshold_db`/`threshold`、`ratio`、`knee_db`/`knee`、`detector_s`/`detector`、`attack_s`/`attack`、`release_s`/`release`、`makeup_db`/`makeup`） |
//! | `output_gain_db` | `output_gain_db`、`output_gain`、`fader_db` |
//!
//! ### 4.3 为什么**不**认 `filter_cutoff_hz`
//!
//! `tests/compressor_insert.rs` 的 C0 用两台效果器表达"**弄不懂的**效果器"：
//! 一台 `params` 为空，另一台带 `("reverb_mix", 0.3)` 与 `("filter_cutoff_hz", 800.0)`
//! （`crates/yeban-engine/tests/compressor_insert.rs:122`）。C0 断言这类工程与
//! "根本没有设备链"的工程**逐字节相同** —— 它是 `c792fdc` 的**字面判据**，
//! 本票不修改它的期望值。
//!
//! **实测**：把 `filter_cutoff_hz` 收作 `filter.cutoff_hz` 的别名之后，
//! `cargo test -p yeban-engine --no-default-features --test compressor_insert` 的 C0
//! **变红**（`参数不认识的的效果器: 输出必须与没有设备链时逐位相同`，
//! 指纹 `11503627533076961125` ≠ `7952998718812303221`）—— 因为那台设备从"不认识"
//! 变成了"一条 800 Hz 低通的通道条"。
//!
//! 本票的裁决：**不收这个名字**。它换来的是"接线不改变既有听感"这条更高的约束
//! （brief 的 ⚠ 项），代价是"想用 `filter_cutoff_hz` 表达通道条低通"的调用方
//! 必须改写 `cutoff_hz`。⚠ 这条取舍**是明说的**，不是遗漏。
//!
//! ## 5. 这是**临时形状**，不是模型层的第二份定义
//!
//! 与 [`crate::synth::ToneParams`] 一样：`yeban-model` 补齐"效果器参数 → 音频线程"的
//! 投影（N5 的裁决）之后，本模块的投影部分应当**整体删除**，只保留 `pub use` 与
//! 参数类型的再导出。删除前它必须保持**唯一实现**：
//! 本文件不定义通道条算法、不定义压缩器算法、不定义混响算法、不定义卷积混响算法，
//! 也不定义第二份参数类型 —— 判据
//! `engine_insert_module_has_no_second_compressor_implementation`、
//! `engine_insert_module_has_no_second_channel_strip_implementation`、
//! `engine_insert_module_has_no_second_reverb_implementation`（三条既有）与
//! `engine_insert_module_has_no_second_convolution_reverb_implementation`（本票新增）
//! 用源码级检查钉住它。
//!
//! ## 6. 实时分类（[ADR-0001 D32]）
//!
//! - **构造期**（控制线程，[`InsertParams::from_devices`]）：字符串比较、`Vec` 扫描、
//!   `sanitised` 的钳制 —— 允许分配，允许超越函数。
//! - **快照边界**（音频线程，每个修订一次）：[`ChannelStrip::new`] /
//!   [`ChannelStrip::set_params`] / [`ChannelStrip::set_sample_rate`] 会重算 EQ 与
//!   滤波系数、以及压缩器的三个一阶低通系数（含 `exp`）⇒ **超越函数类**。
//!   它与引擎里既有的"武装"步骤同一条口径（`crate::rt` 在同一个分支里调
//!   [`crate::mixer::pan_gains`] 的 `cos`/`sin`）。**不在逐样本路径上**。
//! - **逐样本**（[`ChannelStrip::process_mono`]）：乘加、一阶递归、除法；
//!   `exp`/`cos`/`sin` 不在路径上 ⇒ IEEE 精确类；**零分配、零锁、零 I/O**。
//!   EQ 的中间缓冲是**栈数组** `[f32; 64]`（`crates/yeban-dsp/src/channel_strip.rs` §3）。
//!
//! ## 7. 与"接线会改变渲染输出"的关系（本模块的边界）
//!
//! 接线的**默认口径**是"没有已识别的效果器设备 ⇒ 不挂通道条"：
//! 实时侧对此**整段跳过**（不是"参数取成透明"）⇒ 这类工程的输出与接线前**逐位相同**。
//! 只有**显式**携带通道条或压缩器参数的工程会改变输出 —— 那正是本模块的目的。
//! 逐位一致的**实测**证据（原始样本落盘 + `shasum -a 256`）见
//! `crates/yeban-engine/tests/compressor_insert.rs` 的 C0（未武装）与
//! `crates/yeban-engine/tests/channel_strip_insert.rs` 的 C8（未武装的四条口径）。
//!
//! ## 8. 第二件器件（本票）：每轨**插入混响**（`yeban_dsp::reverb`）
//!
//! `crates/yeban-dsp/src/reverb.rs` 是 Freeverb 拓扑的立体声混响，**早已实现并自带判据**
//! （`docs/ledger/dsp-core-provenance.md:216` 列出其中 6 条）。接线前它在 `crates/yeban-engine`
//! 里**只作为参数字符串**出现（`crates/yeban-engine/tests/compressor_insert.rs:121` 的
//! `("reverb_mix", 0.3)`）—— 没有任何调用点。本模块补的就是那条调用点。
//!
//! ### 8.1 已识别的名字与**两个刻意的拒绝**
//!
//! | 目标字段 | 名字（全部别名） |
//! | :--- | :--- |
//! | `size` | `reverb_size` |
//! | `damp` | `reverb_damp` |
//! | `mix` | `reverb_wet`、`reverb_wet_mix` |
//! | `predelay`（秒） | `reverb_predelay`、`reverb_predelay_s` |
//!
//! ⛔ **不认** `reverb_mix`：`crates/yeban-engine/tests/compressor_insert.rs:121` 与
//! `crates/yeban-engine/tests/channel_strip_insert.rs:126` 用 `("reverb_mix", 0.3)` 表达
//! "**弄不懂的**效果器"，而它们各自带着一条"逐字节相同"的既有判据（C0 / C8）。
//! 把那台设备从"不认识"变成"一台 25 % 湿声的混响"会让那两条判据变红 ⇒ 那是接线在
//! 改变既有输出。本票的裁决与 `db1850f` 对 `filter_cutoff_hz` 的裁决（模块文档 §4.3）
//! **同款**：**不收这个名字**；湿/干旋钮叫 `reverb_wet`。
//!
//! ⛔ **不认** `reverb_width`：本模块的插入点是**单声道**（见 §8.3）⇒ 立体声展宽在那里
//! **没有作用面**。收下一个不能生效的旋钮 = 界面上有、音频上没有 —— 本仓库把那种形状
//! 叫"说假话"。宽度参数因此登记为**未接线缺口**（是明说的取舍，不是遗漏）。
//!
//! ### 8.2 基值与"装置即生效"
//!
//! 与通道条不同，混响**没有**逐级开关（`ReverbParams` 的五个字段都参与每次处理）。
//! 因此口径是：**出现任何一个已识别名字 ⇒ 这台设备就是一台混响**，未出现的字段取
//! `ReverbParams::default()`（`crates/yeban-dsp/src/reverb.rs:155`）：
//! `size = 0.45` / `damp = 0.35` / `mix = 0.25` / `width = 0.8` / `predelay = 0.012` 秒。
//! 一个已识别名字都没有 ⇒ [`InsertParams::reverb`] 为 `None` ⇒ 实时侧**整段跳过**
//! （不是"参数取成透明"）⇒ 那类工程与接线前**逐位相同**。
//!
//! 钳制由器件自己做：`Reverb::set_params`（`crates/yeban-dsp/src/reverb.rs:279`）把非有限值
//! 换成默认值、把每个字段钳进合法域 ⇒ 快照里存的数与音频线程将要用的数之间有定义
//! （与 `ChannelStripParams::sanitised` 同目的，只是这里由器件自己负责）。
//!
//! ### 8.3 插入点与单声道口径（`width` 为什么拒收）
//!
//! `Reverb::process` 是**立体声块**接口（`&mut [f32], &mut [f32]`），而引擎的逐轨信号在
//! 插入点是**单声道**（`crate::rt` 的 `track_scratch`；逐轨电平、PDC 与声相都在它之后）。
//! 本模块的处理：把该单声道信号**同时喂给左右两路**，取两路输出的**中值**
//! `(out_l + out_r) · 0.5`。
//!
//! 这不是近似，是恒等式：器件湿路径的输入是 `(dl + dr) · 0.5 · 0.015`
//! （`crates/yeban-dsp/src/reverb.rs:364`）——`dl = dr = m` 时它等于 `m · 0.015`，与单声道
//! 输入同解；宽度只进 `side` 项（`crates/yeban-dsp/src/reverb.rs:382`），而
//! `out_l + out_r = 2 · mid` ⇒ 中值恰好等于 `mid`，`width` 在其中**代数抵消**。
//! 这就是 §8.1 拒收 `reverb_width` 的全部理由。
//!
//! ### 8.4 实时分类（[ADR-0001 D32]）
//!
//! - **构造期**（控制线程，[`InsertParams::from_devices`]）：字符串比较、`Vec` 扫描。
//! - **构造期**（控制线程，`crate::rt::EngineRuntime::new`）：延迟线的**唯一**分配点
//!   （`Reverb::set_sample_rate`，`crates/yeban-dsp/src/reverb.rs:242`）。它**必须**在音频
//!   回调之外调用 —— 这是本器件与通道条最大的结构差别（通道条没有堆）。
//! - **快照边界**（音频线程，每个修订一次）：只调 `Reverb::set_params`（标量赋值，
//!   **零分配**）。快照的采样率若与武装时不同：**不**调 `set_sample_rate`，而是整段不武装
//!   并累加一个读数（`EngineStats::insert_reverb_rate_rejects`）。理由见 §8.5。
//! - **逐样本**（`Reverb::process`）：乘加与环形缓冲读写；零分配、零锁、零 I/O、零日志。
//!
//! ### 8.5 为什么"采样率变了就不武装"（明说的取舍）
//!
//! `Reverb::set_sample_rate` 按 `sr_scale` **重建**每条延迟线：缓冲区长度
//! `buf.len() != self.len` 时它做 `Vec` 重分配（`crates/yeban-dsp/src/reverb.rs:73`、
//! `:115`）。音频线程不允许分配/释放 [MUST-GATE-001] ⇒ 那个方法只能在
//! `EngineRuntime::new`（回调之前）调用一次，用**初始快照**的采样率。
//!
//! 引擎的其余部分**支持**运行期换采样率（`crates/yeban-engine/tests/rt_zero_alloc.rs`
//! 的场景⑭把 5 个采样率逐个发布到同一个运行时，断言四元组全 0）。混响是唯一一个
//! "换采样率就要重新分配"的器件 ⇒ 本票的裁决：**换采样率 ⇒ 这一份快照不武装混响**，
//! 并累加 `insert_reverb_rate_rejects`。宁可少一个器件，也不在回调里分配、也不拿旧采样率
//! 的延迟线去处理新采样率的信号（后者是听不出来的错，本仓库不收）。
//!
//! 可达性（**实测**）：`EngineHost::reload`（`crates/yeban-app/src/engine_host.rs:496`）
//! **重建**运行时并把新采样率带进构造期；运行期换采样率只有
//! `EngineHost::publish_project`（`:716`）一条路径。
//! `grep -rn 'audio_config' crates/yeban-app/src crates/yeban-mcp/src` 命中 **4** 行
//! （`crates/yeban-mcp/src/domain/engine_state.rs:10,146,335` 与
//! `crates/yeban-mcp/src/domain/view.rs:42`），**全部是只读**（JSON 字段名 / 视图白名单），
//! **没有**任何一行给 `audio_config.sample_rate` 赋值
//! （`grep -rn 'audio_config\.sample_rate\s*=' crates/*/src` 在非 model 侧零命中）
//! ⇒ 今天没有任何界面/工具写入者能改它。这条守卫因此是**安全网**，不是日常路径。
//!
//! ## 9. 第三件器件（本票）：每轨**卷积混响**（`yeban_dsp::convolution_reverb`）
//!
//! `crates/yeban-dsp/src/convolution.rs`（单声道核）、`convolution_stereo.rs`（四通路装配）、
//! `convolution_reverb.rs`（预延迟 ＋ 湿干混合 ＋ IR 增益的外壳）三层**早已实现并自带判据**，
//! 但接线前 `crates/yeban-engine` 里**一个调用点都没有**（核实方式：
//! `grep -rn 'convolution' crates/yeban-engine/src` 在接线前**零命中**）。本模块补的就是那条调用点。
//!
//! 本件器件与前两件的**结构差别只有一条**，但它决定了全部接线形状：它要吃**一份数据**（IR），
//! 不是只吃几个标量。
//!
//! ### 9.1 模型层没有放 IR 的地方 —— 这是**实测**，不是推测
//!
//! | 事实 | 出处 |
//! | :--- | :--- |
//! | 器件的 IR 入口收的是**已经就绪的 `&[f32]`** | `crates/yeban-dsp/src/convolution_reverb.rs` 的 `set_impulse_response`（现位于第 281 行） |
//! | 它**不是** IR 载入器（文件 I/O、采样率换算、首波对齐、淡出都不在 `yeban-dsp` 的边界内） | 同文件模块文档 §0 第 1 条 |
//! | 模型层的参数**唯一**载体只有三个字段：`name: String` / `value: f32` / `unit: Option<String>` | `crates/yeban-model/src/project.rs` 的 `ParameterValue`（现位于第 417 行） |
//!
//! ⇒ **没有任何字段能承载一段样本数组**。给 `ParameterValue` 加一个样本数组、或给
//! `DeviceDefinition` 加一个 `ir` 字段都能解决它，但那**越出本票的改动范围**
//! （本票只许改 `crates/yeban-engine`），而且会把"参数"与"数据"两种东西塞进同一个类型。
//!
//! ### 9.2 本票的裁决：**不扩模型形状**，改为在构造期**合成** IR
//!
//! 投影因此走一条与前两件器件不同的路：**三个标量旋钮 → 一条确定的合成 IR**。三个旋钮是
//! [`ConvProjection`] 认下的 `conv_ir_decay_s` 与 `conv_ir_seed`（外加"这台设备算不算一台
//! 卷积混响"的 `seen` 口径），合成发生在 [`InsertParams::from_devices`] 里 —— 那是**控制线程的
//! 构造期**，允许分配、允许任何运算。合成结果与四个标量器件参数一起进快照
//! （[`ConvolutionPlan`]）。
//!
//! ⚠ **这是一条合成的 IR，不是一段测量到的 IR**。它由 `yeban_dsp::noise::Rng`（xorshift32，
//! 显式播种、跨运行同序）取白噪声、乘一条指数衰减包络、再做峰值归一化得到。本票**不做**
//! 任何"对标 ReaVerb"的主张：真实空间的 IR 需要载入器（文件 I/O），而本 crate 与
//! `yeban-dsp` 都不做 I/O；模型层补齐"设备数据"的形状之前，合成是唯一能在**不扩模型**的
//! 前提下把这条链路接通的路。这条取舍**是明说的**，不是遗漏。
//!
//! **IR 的时长是引擎常量，不是旋钮**（[`convolution_ir_frames`]：帧数 = 采样率 ÷ 10 = 0.1 s）。
//! 两条理由：
//!
//! 1. **分配纪律**：器件的 `set_impulse_response` 只在 IR **长度改变**时重建缓冲
//!    （`crates/yeban-dsp/src/convolution.rs` 的 `set_impulse_response` 文档，
//!    现位于第 271 行）⇒ 固定长度之后，快照边界上的换 IR **零分配**。长度若做成旋钮，
//!    "换长度"就会在音频线程上分配 [MUST-GATE-001]。
//! 2. **池的形状**：引擎在构造期把**每一个**轨道槽位按这个固定长度建好（见 [`crate::rt`]），
//!    因此**任何**轨道在**任何**快照里出现卷积混响设备都能武装 —— 与混响延迟线
//!    "按上限预分配"是同一个手法（模块文档 §8.4）。
//!
//! ### 9.3 已识别的名字（每项列出全部别名）
//!
//! | 目标字段 | 名字（全部别名） |
//! | :--- | :--- |
//! | `pre_delay_s`（秒） | `conv_predelay_s`、`conv_pre_delay_s`、`conv_predelay` |
//! | `dry`（线性） | `conv_dry` |
//! | `wet`（线性） | `conv_wet`、`conv_wet_mix` |
//! | `ir_gain_db`（dB） | `conv_ir_gain_db`、`conv_ir_gain` |
//! | IR 的衰减时间常数（秒） | `conv_ir_decay_s`、`conv_ir_decay` |
//! | IR 的随机种子（`f32` → `u32`） | `conv_ir_seed` |
//!
//! 前缀 `conv_` 是**刻意**的：它与前两件器件认的名字**无一重合**，因此一份既有工程
//! （一个 `conv_` 名字都没写）投影出来的链与接线前**逐位相同** —— 实时侧对
//! [`InsertParams::convolution`] 为 `None` 的轨整段跳过（判据
//! `devices_without_recognised_convolution_params_do_not_arm_a_convolution_reverb`）。
//! ⚠ 本模块**不认** `reverb_*` 系列名字（那些属于 §8 的 Freeverb 器件），也不认任何
//! 不带 `conv_` 前缀的通用名（`irdata` 之类）：认下一个既有的、别处用来表达别的东西的名字，
//! 正是 §4.3 与 §8.1 两次实测变红的那类缺陷。
//!
//! ### 9.4 放置与单声道口径
//!
//! 位置：`crate::rt` 的插入链里**通道条之后、Freeverb 混响之后**的第三级。链的形状没变
//! （仍是"逐轨查找 ＋ 整段跳过"的定长槽位表），只是多了一级。
//!
//! 器件的 `process` 收的是**交错立体声**块（`[L, R, …]`），而引擎的插入点是**单声道**。
//! 本模块的口径与 §8.3 同款：把该单声道样本同时写进左右两路，取两路输出的**中值**
//! `(out_l + out_r) · 0.5`。这里它同样是恒等式：四条通路是线性的，中值等于
//! `conv((h_LL + h_LR + h_RL + h_RR) / 2, m)`，而本模块交进去的 IR 是
//! `h_LL = h_RR = ir`、`h_LR = h_RL = 0` ⇒ 中值恰好是 `conv(ir, m)`，正是"单声道 IR 居中"。
//!
//! ⚠ **代价是四条核全跑**（两条对角有效、两条乘的是一片零谱）—— 这是器件自己
//! 登记过的取舍（`crates/yeban-dsp/src/convolution_reverb.rs` 的
//! `set_stereo_impulse_response` 文档）：本模块**不**为了省那一半而自己写一份
//! "单声道 ＋ 湿干混合"的第二实现（那是把 §5 的"唯一实现"承诺打破）。
//!
//! ### 9.5 实时分类（[ADR-0001 D32]）
//!
//! - **构造期**（控制线程，[`InsertParams::from_devices`]）：字符串比较、IR 合成
//!   （每个样本一次 `libm::expf`）⇒ 超越函数类，**不在**音频路径上。
//! - **构造期**（控制线程，`crate::rt::EngineRuntime::new`）：IR **缓冲**的**唯一**分配点
//!   （每个槽位一次"占位 IR"），以及预延迟线（`ConvolutionReverb::set_sample_rate`）。
//! - **快照边界**（音频线程，每个修订一次）：只做**长度不变**的
//!   `set_impulse_response`（原地复用缓冲：零分配）与 `set_params`（标量 ＋ 一次
//!   `libm::expf` 折算 IR 增益）⇒ 与通道条同一条纪律。
//! - **逐样本**（`ConvolutionReverb::process`）：频域分块乘加与 256 点变换；
//!   **零分配、零锁、零 I/O、零日志** [MUST-GATE-001]。⚠ 它是**超越函数类**而不是
//!   IEEE 精确类：旋转因子表在构造期由宿主 libm 的 `f32::cos`/`f32::sin` 算出
//!   （`crates/yeban-dsp/src/convolution.rs`，现位于第 309 行）⇒ 跨架构只承诺
//!   [ADR-0001 D32] 第 2 条的 4096 ulp 预算，**不**承诺逐位相同。本票的判据因此
//!   凡涉及卷积**输出**都只在冻结架构（aarch64）上逐位断言。
//! - **快照边界**（第二个入口）：IR 长度由采样率决定 ⇒ 与混响同一条"换采样率就整段
//!   不武装并计数"的守卫（§8.5）。否则那条边界上会出现一次 `Vec` 重建。
//!
//! ### 9.6 与"接线会改变渲染输出"的关系
//!
//! 与前两件器件同款：**没有任何已识别的 `conv_` 参数 ⇒ 不武装 ⇒ 实时侧整段跳过**
//! （不是"参数取成透明"）⇒ 那类工程的输出与接线前**逐位相同**。只有**显式**写了
//! `conv_*` 参数的工程才会改变输出 —— 那正是本模块的目的。

use yeban_model::{DeviceDefinition, DeviceKind};

// 通道条的**唯一实现**住在 `yeban-dsp`（`crates/yeban-dsp/src/channel_strip.rs`，
// 规范出处见 `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:109`）。本模块只做转发：
// 类型 + 参数类型。⚠ 引擎侧的既有命名先例是 `mixer.rs` 的 `Limiter as BusLimiter`
// —— 那是为了不改动既有调用点而保留旧名；本模块是**新**调用点，故直接用 dsp 的名字。
pub use yeban_dsp::channel_strip::{
    ChannelStrip, ChannelStripParams, FilterParams, MAX_CUTOFF_HZ, MAX_GAIN_DB, MIN_CUTOFF_HZ,
    MIN_GAIN_DB,
};
// 通道条的动态级复用 `yeban_dsp::compressor`（本模块**不**定义第二份）。
//
// ⚠ `Compressor` 与 `CompressorParams` **必须继续再导出**：`c792fdc` 的判据
// （`tests/compressor_insert.rs` 的 C6）按这两个名字断言"引擎的压缩器**就是** dsp 的
// 压缩器"（类型同一性 + `process_mono` / `new` / `set_params` 的函数地址同一性）。
// 本票把槽位里的器件换成通道条，但**没有**让那条判据失去对象：通道条的动态级仍是
// 同一个类型。删除这两个再导出会让 C6 编译失败（那是判据被削弱，不是被满足）。
pub use yeban_dsp::compressor::{Compressor, CompressorParams};
// EQ 参数类型同样转发（`ChannelStripParams::eq` 的字段类型）。
pub use yeban_dsp::shaping::EqParams;
// 判据要断言"引擎的通道条**就是** dsp 的通道条"（类型同一性 + 函数地址同一性）。
// `ChannelStrip` 这个名字已经被本模块上面的再导出占用，故再给一个**别名**让判据能写出
// "dsp 类型 ← engine 路径构造"的赋值（与 `mixer.rs` 的 `Limiter as BusLimiter` 同款）。
pub use yeban_dsp::channel_strip::ChannelStrip as DspChannelStrip;
// 混响的**唯一实现**住在 `yeban-dsp`（`crates/yeban-dsp/src/reverb.rs`）。本模块只转发：
// 类型 + 参数类型。判据（`tests/reverb_insert.rs` 的 R7）按这两个名字断言
// "引擎的混响**就是** dsp 的混响"（类型同一性 + `process`/`set_params` 的函数地址同一性）。
pub use yeban_dsp::reverb::{Reverb, ReverbParams};
// 与 `DspChannelStrip` 同一个理由：让判据能写出"dsp 类型 ← engine 路径"的赋值。
pub use yeban_dsp::reverb::Reverb as DspReverb;
// 卷积混响的**唯一实现**住在 `yeban-dsp`（`crates/yeban-dsp/src/convolution_reverb.rs`，
// 它下面还有 `convolution` 与 `convolution_stereo` 两层）。本模块只转发：类型 + 参数类型。
// 判据（`tests/convolution_insert.rs` 的 V7）按这两个名字断言"引擎的卷积混响**就是**
// dsp 的卷积混响"（类型同一性 + `process`/`set_params`/`set_impulse_response` 的函数地址同一性）。
pub use yeban_dsp::convolution_reverb::{ConvolutionReverb, ConvolutionReverbParams};
// 与 `DspChannelStrip` 同一个理由：让判据能写出"dsp 类型 ← engine 路径"的赋值。
pub use yeban_dsp::convolution_reverb::ConvolutionReverb as DspConvolutionReverb;
// IR 合成本身**不是**本模块的发明：白噪声源是 `yeban_dsp::noise::Rng`（xorshift32，
// 显式播种、跨运行同序），本模块**不**定义第二份 PRNG。
use yeban_dsp::noise::Rng;

/// 一条轨的**插入链**投影（三件器件：通道条、混响与卷积混响）。
///
/// 结构体保留一个"链"的形状而不是裸 `Option<ChannelStripParams>`：后续器件接线时
/// 在这里**加字段**即可，实时侧的"逐轨查找 + 整段跳过"骨架不必改。
///
/// 本票按这个形状加了第三个字段 [`Self::convolution`]（`yeban_dsp::convolution_reverb`，
/// 见模块文档 §9）。⚠ 它同时是**本类型失去 `Copy` 的原因**：那一件器件必须携带一份 IR
/// （`Vec<f32>`），见 [`ConvolutionPlan`]。
///
/// ⚠ **本链的第四个器件仍然是"没有"**：`drums` 是**音源**（触发式），不是插入器件
/// ⇒ 它住在 [`crate::synth`] 里，位置在本链**之前**。理由（插入链没有事件输入、
/// `DrumMachine` 没有 `process_*`）见 [`crate::drums`] 模块文档 §2。
/// `grep -rn 'use yeban_dsp' crates/yeban-engine/src` 现在有真实引用分布在
/// `synth` / `insert` / `mixer` / `drums` 等模块（判据见
/// `crates/yeban-engine/tests/drums_instrument.rs` 的 D2）。
#[derive(Clone, Debug, PartialEq)]
pub struct InsertParams {
    /// 本轨的通道条参数：`None` = 本轨**没有**通道条（实时侧那段跳过）。
    strip: Option<ChannelStripParams>,
    /// 本轨的混响参数：`None` = 本轨**没有**混响（实时侧那段跳过）。
    ///
    /// ⚠ 与通道条不同，这里**没有**"级开关"：`ReverbParams` 的五个字段全参与处理，
    /// 未出现的字段取 `ReverbParams::default()`（模块文档 §8.2）。
    reverb: Option<ReverbParams>,
    /// 本轨的**卷积混响计划**：`None` = 本轨**没有**卷积混响（实时侧那段跳过）。
    ///
    /// ⚠ 与前两件不同，这里装的**不只是参数**：它带着一份**已经在构造期合成好**的 IR
    /// （见 [`ConvolutionPlan`] 与模块文档 §9.1／§9.2）。这是模型层放不下 IR 的直接后果。
    convolution: Option<ConvolutionPlan>,
}

/// 一条轨的**卷积混响计划**：四个器件标量参数 ＋ 一份**已经就绪**的 IR。
///
/// 它存在的唯一理由：器件的 IR 入口收的是**已经就绪的 `&[f32]`**，而模型层没有承载
/// 样本数组的字段（模块文档 §9.1 给了两处出处）。因此"标量 → IR"的合成必须发生在
/// **控制线程的构造期**，合成结果必须随快照一起送到音频线程。
///
/// ⚠ 它**不是**模型的第二份定义，也**不是**卷积算法：卷积全部在
/// [`ConvolutionReverb`] 里，这里只有一段样本数据 + 四个标量。
#[derive(Clone, Debug, PartialEq)]
pub struct ConvolutionPlan {
    /// 器件的四个标量参数（预延迟／干／湿／IR 增益）。器件在 `set_params` 里再净化一次。
    params: ConvolutionReverbParams,
    /// 单声道 IR：`h_LL = h_RR = ir`（两条**交叉**通路见 [`Self::silence`]）。
    ir: Vec<f32>,
    /// 与 [`Self::ir`] **等长**的全零切片（`h_LR = h_RL`）。
    ///
    /// ⚠ 它**必须**预先算好，不能留给音频线程：器件要求四条 IR 等长
    /// （`crates/yeban-dsp/src/convolution_stereo.rs` 的 `set_impulse_response`，
    /// 现位于第 137 行），而 `set_mono_impulse_response` 会在**方法内**分配这条零切片
    /// （`crates/yeban-dsp/src/convolution_reverb.rs`，现位于第 321 行）—— 那条路径
    /// **不能**在快照边界上走。把它搬到构造期之后，边界上只剩一次"长度不变"的
    /// `set_impulse_response`，实测零分配（`crates/yeban-dsp/tests/convolution_rt_zero_alloc.rs`）。
    silence: Vec<f32>,
    /// [`Self::ir`] 的**内容标识**（FNV-1a 64，逐样本位模式）。
    ///
    /// 用途**只有一个**：让快照边界能判断"这一份 IR 与器件里已经装着的那一份是不是同一个"
    /// ⇒ 是同一个就**只**调 `set_params`（器件的时间状态 —— 频域延迟线与重叠相加尾 ——
    /// 因此保留），不是同一个才调 `set_impulse_response`（那会把状态复位）。
    ///
    /// ⚠ 为什么需要它：应用在**每一次工程编辑**时都会发布新快照。若那条边界上无条件重设 IR，
    /// 每一次编辑都会切断所有轨的卷积尾巴，而且要为每条轨重算 `分区数` 次 256 点变换
    /// （1 秒 IR 是 375 次）—— 一个听得出、也算得出的缺陷。
    ///
    /// ⚠ 为什么是哈希而不是"存下 `(种子, 衰减)` 再逐个比较"：哈希覆盖的是**实际产出的样本**，
    /// 因此将来给合成器加一个旋钮时它自动跟着变（比较法会漏）。代价是理论上存在碰撞
    /// （FNV-1a 64，本夹具 4 800 个样本）⇒ 后果只是"这一份新 IR 没生效"，不是内存安全问题；
    /// 判据 `a_changed_ir_knob_reaches_the_device` 钉住"改了旋钮 IR 必须换"。
    ir_hash: u64,
}

/// FNV-1a 64：对一串 `f32` 的**位模式**求内容标识（与 `tests/support` 的 `fingerprint`
/// 同一个算法，但只覆盖 IR 本身）。**构造期**调用一次。
fn ir_fingerprint(samples: &[f32]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for sample in samples {
        for byte in sample.to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

impl ConvolutionPlan {
    /// 器件的四个标量参数（**只**被音频线程用来调 `ConvolutionReverb::set_params`）。
    #[must_use]
    pub const fn params(&self) -> ConvolutionReverbParams {
        self.params
    }

    /// 单声道 IR（`h_LL` 与 `h_RR` 两条对角通路都用它）。
    ///
    /// 存在的理由与前两件器件的 `*_of` 读数一样：把"将要交给器件的那个数组"变成**可读**的，
    /// 判据就不必从音频输出反推（并且能对 IR 本身做哈希对账）。
    #[must_use]
    pub fn ir(&self) -> &[f32] {
        &self.ir
    }

    /// 两条交叉通路的全零切片（与 [`Self::ir`] 等长）。
    #[must_use]
    pub fn silence(&self) -> &[f32] {
        &self.silence
    }

    /// [`Self::ir`] 的**内容标识**（FNV-1a 64）。**只**被音频线程用来判断"IR 换没换"：
    /// 相同 ⇒ 只调 `set_params`（保留器件状态），不同 ⇒ 才调 `set_impulse_response`。
    #[must_use]
    pub const fn ir_hash(&self) -> u64 {
        self.ir_hash
    }

    /// IR 的帧数（单位：帧）。它与 [`convolution_ir_frames`] 必须相等 —— 那条等式
    /// 正是"快照边界上的换 IR 零分配"的前提。**只**被音频线程用来做这条检查。
    #[must_use]
    pub fn ir_frames(&self) -> usize {
        self.ir.len()
    }
}

/// 引擎固定给每轨卷积混响合成的 IR 时长，以"采样率的分母"表达：帧数 = `采样率 ÷ 10`。
///
/// 整数除法 ⇒ `0.1 s` 的换算**不经过浮点**，因此逐位确定、跨平台相同
/// （`48_000 ÷ 10 = 4_800` 帧、`44_100 ÷ 10 = 4_410` 帧）。上界 `10` 与器件的代价表
/// 第一行同值（`crates/yeban-dsp/src/convolution.rs` 模块文档 §8 的 `0.10 s（4 800 帧）`）。
///
/// ⚠ 它**是常量而不是旋钮**：理由（"换长度会分配"＋"池按固定长度预建"）见模块文档 §9.2。
const CONV_IR_SAMPLE_RATE_DIVISOR: usize = 10;

/// 引擎为某采样率合成的 IR 帧数（单位：帧）。
///
/// `sample_rate ÷ 10`，至少 1 帧（退化采样率也不会产出空 IR —— 空 IR 在器件里是
/// "拒绝"，不是"静音"）。**构造期**两次用到它：投影（[`InsertParams::from_devices`]）
/// 与实时侧的池预建（[`crate::rt`]）；两处必须同值，否则快照边界上的换 IR 会**改长度**、
/// 于是分配。判据 `the_plan_and_the_pool_agree_on_the_ir_length` 钉住这条等式。
#[must_use]
pub const fn convolution_ir_frames(sample_rate: u32) -> usize {
    let frames = sample_rate as usize / CONV_IR_SAMPLE_RATE_DIVISOR;
    if frames == 0 { 1 } else { frames }
}

/// IR 衰减时间常数的缺省值（秒）＝ `1 / e` 处。
const CONV_IR_DEFAULT_DECAY_S: f32 = 0.35;

/// IR 衰减时间常数的合法域（秒）。上界远长于 IR 本身 ⇒ 实际得到的是"几乎不衰减"。
const CONV_IR_MIN_DECAY_S: f32 = 0.005;
const CONV_IR_MAX_DECAY_S: f32 = 5.0;

/// IR 随机种子的缺省值（`f32` 形态，写进投影的基值）。
///
/// ⚠ 取一个非零常数而不是 `0`：`Rng::new(0)` 自己会把 0 换成内部常数，那样"缺省"与
/// "显式写 0"就**不可区分**了。这里让缺省是一个**可复述**的数。
const CONV_IR_DEFAULT_SEED: f32 = 1.0;

/// 构造期**合成**一条单声道 IR（模块文档 §9.2）：白噪声 × 指数衰减包络，再峰值归一化。
///
/// **量什么／单位**：返回值是 `frames` 个 `f32` 的 IR 样本（无量纲）。
///
/// 为什么是这三个输入：`frames` 由 [`convolution_ir_frames`] 给（引擎常量），`decay_s` 与
/// `seed` 是投影**唯一**能携带的两个标量旋钮。**没有**别的可调项 —— 每多一项都是一处
/// 没有模型字段支撑的发明。
///
/// 确定性（[ARCH-DET-001]）：
///
/// - 随机源是 `yeban_dsp::noise::Rng`（xorshift32 ＋ `u32 as f32` 的两次算术）⇒ 全部是
///   IEEE 精确类运算，跨架构逐位相同；
/// - 指数走**纯 Rust** `libm::expf`，**不用** `f32::exp`：后者的实现来自宿主 libm，
///   换架构/OS 末位可能不同（`crates/yeban-dsp/src/envelope.rs` 的 `time_coefficient`
///   已按这条纪律办，现位于第 266 行）。全函数的取值因此落在 [ADR-0001 D32] 的
///   **IEEE 精确类**（跨架构逐位相同），而不是超越函数类；
/// - 峰值归一化只用 `abs`/`max`/除法（IEEE 精确类）。
fn synthesise_impulse_response(
    frames: usize,
    decay_s: f32,
    sample_rate: u32,
    seed: u32,
) -> Vec<f32> {
    // 时间常数换算成帧。`max(1.0)` 防止"极短衰减"变成除以 0（那会产出 `inf` 包络，
    // 而 `NaN`/`inf` 会被器件的 IR 校验**拒绝**，整台回到未配置直通）。
    let decay_frames = (decay_s * sample_rate as f32).max(1.0);
    let mut rng = Rng::new(seed);
    let mut ir = Vec::with_capacity(frames);
    for index in 0..frames {
        let envelope = libm::expf(-(index as f32) / decay_frames);
        ir.push(rng.next_bipolar() * envelope);
    }
    // 峰值归一化到 1.0：让 `wet` 旋钮的读数与实际电平一一对应，而不是"看这条 IR 有多响"。
    // 器件不替调用方做这件事（模块文档 §0 第 1 条：长度归一化属于上层）。
    let peak = ir.iter().fold(0.0f32, |acc, value| acc.max(value.abs()));
    if peak > 0.0 && peak.is_finite() {
        for value in &mut ir {
            *value /= peak;
        }
    }
    ir
}

/// 一条轨的**卷积混响投影中间态**（`insert.rs` 私有）。
///
/// 与 [`ReverbProjection`] 同款：没有逐级开关，只有"这台设备到底算不算一台卷积混响"
/// （`seen`）与若干可覆盖的字段。`seen` 不能由"字段 != 默认值"推出来 —— 写一个**恰好
/// 等于默认值**的旋钮同样是"这是一台卷积混响"（与 §8.2 同口径）。
#[derive(Clone, Copy, Debug)]
struct ConvProjection {
    /// 出现过的 `conv_` 旋钮。
    seen: bool,
    /// 四个器件标量的基值 = 器件默认值（`ConvolutionReverbParams::default()`）。
    params: ConvolutionReverbParams,
    /// IR 的时间常数（秒）的基值。
    decay_s: f32,
    /// IR 种子的基值。
    seed: u32,
}

impl Default for ConvProjection {
    fn default() -> Self {
        Self {
            seen: false,
            params: ConvolutionReverbParams::default(),
            decay_s: CONV_IR_DEFAULT_DECAY_S,
            seed: CONV_IR_DEFAULT_SEED as u32,
        }
    }
}

impl ConvProjection {
    /// 把一条参数写进投影（**构造期**；名字已转小写）。
    fn apply(&mut self, name: &str, value: f32) {
        match name {
            "conv_predelay_s" | "conv_pre_delay_s" | "conv_predelay" => {
                self.params.pre_delay_s = value;
                self.seen = true;
            }
            "conv_dry" => {
                self.params.dry = value;
                self.seen = true;
            }
            "conv_wet" | "conv_wet_mix" => {
                self.params.wet = value;
                self.seen = true;
            }
            "conv_ir_gain_db" | "conv_ir_gain" => {
                self.params.ir_gain_db = value;
                self.seen = true;
            }
            "conv_ir_decay_s" | "conv_ir_decay" => {
                self.decay_s = value;
                self.seen = true;
            }
            // `f32 → u32` 是**饱和**转换（Rust 的定义：负数 → 0、超界 → `u32::MAX`、
            // `NaN` → 0），因此退化输入有定义、不 panic。0 由 `Rng::new` 换成内部常数。
            "conv_ir_seed" => {
                self.seed = value as u32;
                self.seen = true;
            }
            _ => {}
        }
    }

    /// 收尾：`None` = 一个已识别的 `conv_` 名字都没有 ⇒ 本设备不是卷积混响来源；
    /// 否则合成 IR 并落成一份 [`ConvolutionPlan`]（**构造期**，允许分配）。
    fn into_plan(self, sample_rate: u32) -> Option<ConvolutionPlan> {
        if !self.seen {
            return None;
        }
        // 非有限值回落缺省，再钳到合法域（与器件的 `sanitise` 同一条纪律）：
        // 快照里存的就是音频线程将要用的那一份数。
        let decay_s = if self.decay_s.is_finite() {
            self.decay_s.clamp(CONV_IR_MIN_DECAY_S, CONV_IR_MAX_DECAY_S)
        } else {
            CONV_IR_DEFAULT_DECAY_S
        };
        let frames = convolution_ir_frames(sample_rate);
        let ir = synthesise_impulse_response(frames, decay_s, sample_rate, self.seed);
        let ir_hash = ir_fingerprint(&ir);
        let silence = vec![0.0f32; frames];
        Some(ConvolutionPlan {
            params: self.params,
            ir,
            silence,
            ir_hash,
        })
    }
}

impl InsertParams {
    /// **空链**：实时侧对这条轨整段跳过 ⇒ 输出逐位不变。
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            strip: None,
            reverb: None,
            convolution: None,
        }
    }

    /// 从模型层的设备链投影（**构造期**；见模块文档 §4、§8 与 §9 的规则表）。
    ///
    /// 三件器件**各自独立**取来源：通道条取第一个含通道条参数名的设备，混响取第一个含
    /// 混响参数名的设备，卷积混响取第一个含 `conv_` 参数名的设备（同一台设备可以同时是
    /// 三者的来源）。这与"第一条含已识别参数的设备是唯一来源"是同一个口径 —— 已识别的
    /// 名字集合变大了，但每一件的**首个**来源仍然是首个（先命中者不被后来的覆盖）。
    ///
    /// `sample_rate` 只在卷积混响那一支用到（IR 的帧数 = 采样率 ÷ 10，
    /// 见 [`convolution_ir_frames`]）：**构造期**把 IR 合成出来，是模型层放不下样本数组
    /// 的直接后果（模块文档 §9.1／§9.2）。
    #[must_use]
    pub fn from_devices(devices: &[DeviceDefinition], sample_rate: u32) -> Self {
        let mut strip: Option<ChannelStripParams> = None;
        let mut reverb: Option<ReverbParams> = None;
        let mut convolution: Option<ConvolutionPlan> = None;
        for device in devices {
            // 规则 1 + 2：只看非旁通的内置效果器。
            if device.bypassed || device.kind != DeviceKind::InternalEffect {
                continue;
            }
            // 规则 3：`None` = 本设备一个已识别参数都没有 ⇒ 它不是那件器件的来源。
            if strip.is_none() {
                strip = Self::strip_of(device);
            }
            if reverb.is_none() {
                reverb = Self::reverb_of(device);
            }
            if convolution.is_none() {
                convolution = Self::convolution_of(device, sample_rate);
            }
            if strip.is_some() && reverb.is_some() && convolution.is_some() {
                break;
            }
        }
        // 规则 7：快照里存的就是音频线程将要用的那一份数。
        Self {
            strip: strip.map(ChannelStripParams::sanitised),
            reverb,
            convolution,
        }
    }

    /// 单台设备 → 通道条参数（`None` = 一个已识别名字都没有）。
    ///
    /// 基值是 `yeban_dsp` 的设备默认值；只有**出现**的名字才覆盖它（规则 4）。
    /// 三级的启用开关按模块文档 §4.1 的表推导：**没写旋钮的级不执行**。
    fn strip_of(device: &DeviceDefinition) -> Option<ChannelStripParams> {
        let mut resolved = StripProjection::default();
        for param in &device.params {
            let name = param.name.to_ascii_lowercase();
            let value = param.value;
            // 规则 5：同名后者胜（逐个赋值）。规则 6：名字已转小写。
            resolved.apply(&name, value);
        }
        resolved.into_params()
    }

    /// 单台设备 → 混响参数（`None` = 一个已识别的混响名字都没有）。
    ///
    /// 基值是 `yeban_dsp` 的设备默认值；只有**出现**的名字才覆盖它（模块文档 §8.2）。
    fn reverb_of(device: &DeviceDefinition) -> Option<ReverbParams> {
        let mut resolved = ReverbProjection::default();
        for param in &device.params {
            let name = param.name.to_ascii_lowercase();
            resolved.apply(&name, param.value);
        }
        resolved.into_params()
    }

    /// 本轨的**通道条参数**；`None` = 整段跳过。
    #[must_use]
    pub const fn strip(&self) -> Option<ChannelStripParams> {
        self.strip
    }

    /// 本轨的**混响参数**；`None` = 整段跳过。
    ///
    /// 存在的理由与 [`Self::strip`] 相同：把"投影出来的那个数"变成可读的，
    /// 判据就不必从音频输出反推。**只**被音频线程用来调 `Reverb::set_params`。
    #[must_use]
    pub const fn reverb(&self) -> Option<ReverbParams> {
        self.reverb
    }

    /// 单台设备 → 卷积混响计划（`None` = 一个已识别的 `conv_` 名字都没有）。
    ///
    /// 基值是器件的默认值；合成 IR 是**构造期**的事（模块文档 §9.2）。
    fn convolution_of(device: &DeviceDefinition, sample_rate: u32) -> Option<ConvolutionPlan> {
        let mut resolved = ConvProjection::default();
        for param in &device.params {
            let name = param.name.to_ascii_lowercase();
            resolved.apply(&name, param.value);
        }
        resolved.into_plan(sample_rate)
    }

    /// 本轨的**卷积混响计划**；`None` = 整段跳过。
    ///
    /// ⚠ 与前两件的 `strip()` / `reverb()` 有一个形状差别：这里返回的是**借用**
    /// （计划里有 `Vec<f32>`，不是 `Copy`）。理由与 [`ConvolutionPlan`] 的文档同一句：
    /// 器件的 IR 入口收的是**已经就绪的 `&[f32]`**。
    ///
    /// **只**被音频线程用来做两件事：长度不变的 `ConvolutionReverb::set_impulse_response`
    /// 与 `ConvolutionReverb::set_params`。
    #[must_use]
    pub const fn convolution(&self) -> Option<&ConvolutionPlan> {
        self.convolution.as_ref()
    }

    /// 本轨通道条的**动态级**参数（`ChannelStripParams::compressor` 的直读）。
    ///
    /// 存在的理由与 `c792fdc` 的同类读数一样：把"投影出来的那个数"变成可读的，
    /// 判据就不必从音频输出反推。**只**压缩器参数的设备经模块文档 §2 第 2 条折成
    /// 只开动态级的通道条 ⇒ 这里的读数是那条既有路径的**权威**参数。
    #[must_use]
    pub fn compressor(&self) -> Option<CompressorParams> {
        self.strip.map(|strip| strip.compressor)
    }

    /// 本链是否为空（没有任何激活的器件）。
    ///
    /// 快照只收录**非空**的链 ⇒ "不在表里"与"表里是空链"在实时侧同解。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.strip.is_none() && self.reverb.is_none() && self.convolution.is_none()
    }
}

impl Default for InsertParams {
    fn default() -> Self {
        Self::empty()
    }
}

/// 一台设备的**逐字段投影中间态**（`insert.rs` 私有）。
///
/// 职责：把"参数名 → 字段"的匹配与"哪一级被启用"的推导集中在一处，
/// 让 [`InsertParams::strip_of`] 只负责遍历参数。
///
/// 为什么需要中间态而不是直接改 `ChannelStripParams`：三级的启用开关**不能**用
/// 设备默认值（`eq_enabled` / `filter_enabled` 在 dsp 里是 `true`，而"没写旋钮"必须
/// 得到 `false`，见模块文档 §4.1）。`None` 在这里的语义是"设备没写这个开关"。
#[derive(Clone, Copy, Debug)]
struct StripProjection {
    /// 出现过的 EQ 旋钮。
    eq: StripEq,
    /// 出现过滤波旋钮。
    filter_seen: bool,
    /// 出现过动态旋钮（或 `compressor_enabled`）。
    compressor_seen: bool,
    /// 设备写过的开关（`eq_enabled` / `filter_enabled` / `compressor_enabled`）。
    eq_enabled: Option<bool>,
    filter_enabled: Option<bool>,
    compressor_enabled: Option<bool>,
    /// 非旋钮字段（增益 / 截止频率 / 动态参数 / EQ 频率与 Q）。
    params: ChannelStripParams,
}

#[derive(Clone, Copy, Debug, Default)]
struct StripEq {
    seen: bool,
    low_gain: Option<f32>,
    low_freq: Option<f32>,
    mid_gain: Option<f32>,
    mid_freq: Option<f32>,
    mid_q: Option<f32>,
    high_gain: Option<f32>,
    high_freq: Option<f32>,
}

impl Default for StripProjection {
    fn default() -> Self {
        Self {
            eq: StripEq::default(),
            filter_seen: false,
            compressor_seen: false,
            eq_enabled: None,
            filter_enabled: None,
            compressor_enabled: None,
            // 基值 = dsp 的设备默认值（规则 4）。
            params: ChannelStripParams::DEFAULT,
        }
    }
}

/// 布尔参数的取值口径：非零即真（`0.0` 是假，其余值是真）。
///
/// 模型层的参数值是 `f32`（`ParameterValue::value`），没有布尔类型 ⇒ 这里给出一条
/// 机械且确定的读法，而不是"大于 0.5 算真"这类需要解释的阈值。
fn flag(value: f32) -> bool {
    value != 0.0
}

impl StripProjection {
    /// 把一条参数写进投影（**构造期**；名字已转小写）。
    fn apply(&mut self, name: &str, value: f32) {
        match name {
            // --- 增益级（逐样本乘一个常数）---
            "input_gain_db" | "input_gain" | "trim_db" => self.params.input_gain_db = value,
            "output_gain_db" | "output_gain" | "fader_db" => self.params.output_gain_db = value,

            // --- 级开关 ---
            "eq_enabled" => self.eq_enabled = Some(flag(value)),
            "filter_enabled" => self.filter_enabled = Some(flag(value)),
            "compressor_enabled" => {
                self.compressor_enabled = Some(flag(value));
                self.compressor_seen = true;
            }

            // --- EQ 旋钮 ---
            "eq_low_gain" | "low_gain" => {
                self.eq.low_gain = Some(value);
                self.eq.seen = true;
            }
            "eq_low_freq" | "low_freq" => {
                self.eq.low_freq = Some(value);
                self.eq.seen = true;
            }
            "eq_mid_gain" | "mid_gain" => {
                self.eq.mid_gain = Some(value);
                self.eq.seen = true;
            }
            "eq_mid_freq" | "mid_freq" => {
                self.eq.mid_freq = Some(value);
                self.eq.seen = true;
            }
            "eq_mid_q" | "mid_q" => {
                self.eq.mid_q = Some(value);
                self.eq.seen = true;
            }
            "eq_high_gain" | "high_gain" => {
                self.eq.high_gain = Some(value);
                self.eq.seen = true;
            }
            "eq_high_freq" | "high_freq" => {
                self.eq.high_freq = Some(value);
                self.eq.seen = true;
            }

            // --- 滤波旋钮 ---
            // ⚠ 只有 `cutoff_hz`：`filter_cutoff_hz` **刻意不认**，理由见模块文档 §4.3。
            "cutoff_hz" => {
                self.params.filter.cutoff_hz = value;
                self.filter_seen = true;
            }
            "resonance" | "filter_resonance" => {
                self.params.filter.resonance = value;
                self.filter_seen = true;
            }
            "drive" | "filter_drive" => {
                self.params.filter.drive = value;
                self.filter_seen = true;
            }

            // --- 动态旋钮（`c792fdc` 已登记的七个名字及其别名）---
            "threshold_db" | "threshold" => {
                self.params.compressor.threshold_db = value;
                self.compressor_seen = true;
            }
            "ratio" => {
                self.params.compressor.ratio = value;
                self.compressor_seen = true;
            }
            "knee_db" | "knee" => {
                self.params.compressor.knee_db = value;
                self.compressor_seen = true;
            }
            "detector_s" | "detector" => {
                self.params.compressor.detector_s = value;
                self.compressor_seen = true;
            }
            "attack_s" | "attack" => {
                self.params.compressor.attack_s = value;
                self.compressor_seen = true;
            }
            "release_s" | "release" => {
                self.params.compressor.release_s = value;
                self.compressor_seen = true;
            }
            "makeup_db" | "makeup" => {
                self.params.compressor.makeup_db = value;
                self.compressor_seen = true;
            }

            _ => {}
        }
    }

    /// 收尾：把出现过（或一个都没出现）的字段落成 [`ChannelStripParams`]。
    ///
    /// `None` = 一个已识别参数都没有 ⇒ 本设备不是通道条来源（规则 8）。
    fn into_params(self) -> Option<ChannelStripParams> {
        // "已识别"的三个来源：EQ 旋钮 / 滤波旋钮 / 动态旋钮（含 `compressor_enabled`）、
        // 三个级开关、以及任何非旋钮字段（增益等）被写过。
        // ⚠ `params != DEFAULT` 是**兜底**：任何一条被 `apply` 认下的名字都至少改到一处，
        // 因此它只可能漏掉"写了一个与默认值逐位相同的值"——那种写法由三个 `*_seen`
        // 标志覆盖（`compressor_enabled` 自己也设 `compressor_seen`）。
        // `eq_enabled` / `filter_enabled` 单独列出：它们是**其他**参数都不写时唯一的痕迹。
        let recognised = self.eq.seen
            || self.filter_seen
            || self.compressor_seen
            || self.eq_enabled.is_some()
            || self.filter_enabled.is_some()
            || self.params != ChannelStripParams::DEFAULT;
        if !recognised {
            return None;
        }
        let mut params = self.params;
        // EQ 字段：写过的覆盖 `EqParams::default()`（未写的保持平坦）。
        let eq = &mut params.eq;
        if let Some(value) = self.eq.low_gain {
            eq.low_gain = value;
        }
        if let Some(value) = self.eq.low_freq {
            eq.low_freq = value;
        }
        if let Some(value) = self.eq.mid_gain {
            eq.mid_gain = value;
        }
        if let Some(value) = self.eq.mid_freq {
            eq.mid_freq = value;
        }
        if let Some(value) = self.eq.mid_q {
            eq.mid_q = value;
        }
        if let Some(value) = self.eq.high_gain {
            eq.high_gain = value;
        }
        if let Some(value) = self.eq.high_freq {
            eq.high_freq = value;
        }
        // 启用开关：写过的优先，否则按"没写的级不执行"推导（模块文档 §4.1）。
        params.eq_enabled = self.eq_enabled.unwrap_or(self.eq.seen);
        params.filter_enabled = self.filter_enabled.unwrap_or(self.filter_seen);
        params.compressor_enabled = self.compressor_enabled.unwrap_or(self.compressor_seen);
        Some(params)
    }
}

/// 一台设备的**混响投影中间态**（`insert.rs` 私有）。
///
/// 比通道条那一份简单得多：混响没有逐级开关，只有"这台设备到底算不算一台混响"
/// （`seen`）与四个可覆盖的字段。`seen` 不能由"字段 != 默认值"推出来 ——
/// 写一个**恰好等于默认值**的旋钮同样是"这是一台混响"（模块文档 §8.2）。
#[derive(Clone, Copy, Debug, Default)]
struct ReverbProjection {
    /// 出现过的混响旋钮（或 `reverb_wet` 等）。
    seen: bool,
    /// 基值 = 器件默认值（模块文档 §8.2）。
    params: ReverbParams,
}

impl ReverbProjection {
    /// 把一条参数写进投影（**构造期**；名字已转小写）。
    ///
    /// ⚠ `reverb_mix` 与 `reverb_width` **刻意不在**这个 `match` 里：
    /// 理由（C0/C8 的逐位一致判据、单声道插入点）见模块文档 §8.1。
    fn apply(&mut self, name: &str, value: f32) {
        match name {
            "reverb_size" => {
                self.params.size = value;
                self.seen = true;
            }
            "reverb_damp" => {
                self.params.damp = value;
                self.seen = true;
            }
            "reverb_wet" | "reverb_wet_mix" => {
                self.params.mix = value;
                self.seen = true;
            }
            "reverb_predelay" | "reverb_predelay_s" => {
                self.params.predelay = value;
                self.seen = true;
            }
            _ => {}
        }
    }

    /// 收尾：`None` = 一个已识别的混响名字都没有 ⇒ 本设备不是混响来源。
    fn into_params(self) -> Option<ReverbParams> {
        if self.seen { Some(self.params) } else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_model::{EntityId, ParameterValue};

    /// 本模块判据用的采样率（Hz）。取 `48_000` ⇒ IR 恰好 `4_800` 帧，与
    /// `crates/yeban-dsp/src/convolution.rs` 模块文档 §8 代价表的第一行同值。
    const TEST_SR: u32 = 48_000;

    fn device(kind: DeviceKind, bypassed: bool, params: &[(&str, f32)]) -> DeviceDefinition {
        DeviceDefinition {
            id: EntityId::new(),
            name: "Device".to_owned(),
            kind,
            bypassed,
            params: params
                .iter()
                .map(|(name, value)| ParameterValue {
                    name: (*name).to_owned(),
                    value: *value,
                    unit: None,
                })
                .collect(),
            latency_samples: 0,
        }
    }

    /// 带采样率的投影（本模块判据统一用 [`TEST_SR`]）。
    ///
    /// 单独包一层的理由：三件器件里**只有**卷积混响需要采样率（IR 帧数 = 采样率 ÷ 10），
    /// 而既有判据关心的全是另外两件 ⇒ 一层转发让那些判据的正文一个字都不必改，
    /// 也就不会在"改接线"的同时改动"既有期望值"。
    fn from_devices(devices: &[DeviceDefinition]) -> InsertParams {
        InsertParams::from_devices(devices, TEST_SR)
    }

    /// **默认口径**：没有设备 / 没有已识别参数 ⇒ 空链（实时侧整段跳过）。
    #[test]
    fn devices_without_recognised_params_do_not_arm_a_channel_strip() {
        assert!(from_devices(&[]).is_empty());
        // 只上报延迟的效果器（既有夹具的形状）：参数为空。
        assert!(from_devices(&[device(DeviceKind::InternalEffect, false, &[])]).is_empty());
        // 参数名不认识的第三方效果器：不许猜成通道条。
        // ⚠ `filter_cutoff_hz` **不**是本模块认的名字（模块文档 §4.3 说明了理由：
        // 它正是 `tests/compressor_insert.rs` 的 C0 夹具用来表达"弄不懂的效果器"的名字，
        // 把它收下会让那条既有的逐位一致判据变红 ⇒ 那是接线在改变既有听感）。
        // `filter_slope` 是另一类**不认**的名字。
        assert!(
            from_devices(&[device(
                DeviceKind::InternalEffect,
                false,
                &[("reverb_mix", 0.3), ("filter_slope", 0.7)]
            )])
            .is_empty()
        );
        // 内置乐器（音源）不是插入器件 —— 同一个名字也不认。
        assert!(
            from_devices(&[device(
                DeviceKind::InternalInstrument,
                false,
                &[("ratio", 4.0)]
            )])
            .is_empty()
        );
        // 旁通的效果器就是旁通：不许"取默认参数偷偷接回来"。
        assert!(
            from_devices(&[device(
                DeviceKind::InternalEffect,
                true,
                &[("threshold_db", -20.0), ("ratio", 8.0)]
            )])
            .is_empty()
        );
    }

    /// **命中**：出现的名字覆盖默认值，未出现的字段保持 dsp 的默认值。
    #[test]
    fn recognised_params_override_the_defaults_field_by_field() {
        let insert = from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[("threshold", -20.0), ("RATIO", 8.0), ("release", 0.25)],
        )]);
        let params = insert.compressor().expect("必须武装动态级");
        assert_eq!(params.threshold_db.to_bits(), (-20.0f32).to_bits());
        assert_eq!(params.ratio.to_bits(), 8.0f32.to_bits());
        assert_eq!(params.release_s.to_bits(), 0.25f32.to_bits());
        // 未出现的字段 = 默认值（逐位）。
        let default = CompressorParams::DEFAULT;
        assert_eq!(params.knee_db.to_bits(), default.knee_db.to_bits());
        assert_eq!(params.detector_s.to_bits(), default.detector_s.to_bits());
        assert_eq!(params.attack_s.to_bits(), default.attack_s.to_bits());
        assert_eq!(params.makeup_db.to_bits(), default.makeup_db.to_bits());
        // 只有动态参数的设备 ⇒ EQ 与滤波级**不执行**（模块文档 §4.1）。
        let strip = insert.strip().expect("必须武装通道条");
        assert!(!strip.eq_enabled, "没写 EQ 旋钮 ⇒ EQ 级不许启用");
        assert!(!strip.filter_enabled, "没写滤波旋钮 ⇒ 滤波级不许启用");
        assert!(strip.compressor_enabled, "写了动态参数 ⇒ 动态级启用");
    }

    /// 规则 3：第一条**没有**已识别参数的效果器不是来源，继续找下一条。
    /// 规则 5：同一设备内同名参数**后者胜**。
    /// 规则 7：非有限值经 `sanitised` 有定义（不许 `NaN` 进快照）。
    #[test]
    fn source_selection_is_ordered_and_values_are_sanitised() {
        let chain = [
            device(DeviceKind::InternalEffect, false, &[("mix", 0.5)]),
            device(
                DeviceKind::InternalEffect,
                false,
                &[("ratio", 2.0), ("ratio", 3.0), ("threshold_db", f32::NAN)],
            ),
            device(DeviceKind::InternalEffect, false, &[("ratio", 99.0)]),
        ];
        let params = from_devices(&chain).compressor().expect("第二条必须命中");
        assert_eq!(params.ratio.to_bits(), 3.0f32.to_bits(), "同名后者胜");
        assert!(params.threshold_db.is_finite(), "非有限值必须被钳掉");
        assert_eq!(
            params.threshold_db.to_bits(),
            yeban_dsp::compressor::MIN_LEVEL_DB.to_bits(),
            "NaN 归到下界（与 sanitised 同口径）"
        );
    }

    /// **EQ ／ 滤波级只在写了对应旋钮时才启用**（模块文档 §4.1 的表）。
    ///
    /// 这条判据是"沉默地启用平坦 EQ"这类缺陷的机械判据：平坦 EQ 仍会改变低位
    /// （`crates/yeban-dsp/src/channel_strip.rs` §3 实测 `4.566e-5`），
    /// 因此"写了 `eq_low_gain`"与"没写"必须在**启用位**上可区分。
    #[test]
    fn stage_enable_flags_follow_the_written_knobs() {
        // 只有 EQ 旋钮。
        let eq_only = from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[("eq_low_gain", 6.0)],
        )])
        .strip()
        .expect("EQ 旋钮是已识别参数");
        assert!(eq_only.eq_enabled);
        assert!(!eq_only.filter_enabled);
        assert!(!eq_only.compressor_enabled);
        assert_eq!(eq_only.eq.low_gain.to_bits(), 6.0f32.to_bits());
        // 未写的 EQ 字段 = 平坦（`EqParams::default`）。
        assert_eq!(eq_only.eq.mid_gain.to_bits(), 0.0f32.to_bits());
        assert_eq!(eq_only.eq.high_gain.to_bits(), 0.0f32.to_bits());

        // 只有滤波旋钮。
        let filter_only = from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[("cutoff_hz", 800.0), ("resonance", 0.4)],
        )])
        .strip()
        .expect("滤波旋钮是已识别参数");
        assert!(!filter_only.eq_enabled);
        assert!(filter_only.filter_enabled);
        assert!(!filter_only.compressor_enabled);
        assert_eq!(filter_only.filter.cutoff_hz.to_bits(), 800.0f32.to_bits());

        // EQ ＋ 动态。
        let both = from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[("eq_high_gain", -12.0), ("threshold_db", -24.0)],
        )])
        .strip()
        .expect("两类旋钮都是已识别参数");
        assert!(both.eq_enabled);
        assert!(!both.filter_enabled);
        assert!(both.compressor_enabled);

        // 显式开关：写了 `eq_enabled = true` 而没有任何 EQ 增益旋钮 ⇒ EQ 级仍然启用
        // （"写过的优先"）。平坦 EQ 的舍入是**调用方明说要的**。
        let forced = from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[("eq_enabled", 1.0)],
        )])
        .strip()
        .expect("开关名也是已识别参数");
        assert!(forced.eq_enabled);
        assert!(!forced.filter_enabled);
        assert!(!forced.compressor_enabled);
    }

    /// 判据：**engine 侧没有第二份压缩器实现**（源码级机械检查）。
    ///
    /// 与 `mixer.rs` 的 `engine_mixer_module_has_no_second_limiter_implementation`
    /// 同款。记号用 `concat!` 拼出来，避免判据自己的字面量命中自己。
    ///
    /// 注入：把 `yeban_dsp::compressor` 的**类型定义**复制进本文件（`struct` + 类型名），
    /// 或在本文件里新增一个 `process_mono` 方法 ⇒ 本判据立即变红。
    /// ⚠ 源码级检查对**本判据自己的注释**也生效：注释里不许出现被禁的字面量
    /// （第一版就是这样自我命中，实测变红后改写了本条注释）。
    #[test]
    fn engine_insert_module_has_no_second_compressor_implementation() {
        let source = include_str!("insert.rs");
        let forbidden = [
            concat!("struct", " Compressor", " {"),
            concat!("impl", " Compressor"),
            concat!("struct", " CompressorParams", " {"),
            concat!("impl", " CompressorParams"),
            concat!("fn", " process_mono("),
            concat!("fn", " gain_db_for("),
            concat!("fn", " output_db_for("),
            concat!("const", " DEFAULT_DETECTOR_S"),
            concat!("const", " POWER_FLOOR"),
        ];
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "engine 的 insert.rs 里出现了实现记号 `{needle}` —— 这里只允许投影 + `pub use`"
            );
        }
        assert!(
            source.contains("pub use yeban_dsp::compressor::{Compressor, CompressorParams};"),
            "engine 的 insert.rs 必须把 dsp 压缩器的**类型与参数类型**都再导出（C6 的同一性判据靠它）"
        );
    }

    /// 判据：**engine 侧没有第二份通道条实现**（源码级机械检查）。
    ///
    /// 与本文件上一条同款、对准 `yeban_dsp::channel_strip`。注入：在本文件里复制
    /// `ChannelStrip` 的类型定义或它的任何一个逐样本内核 ⇒ 本判据立即变红。
    /// ⚠ 与上一条同样的自我命中风险：本条注释里也**不许**出现被禁字面量。
    #[test]
    fn engine_insert_module_has_no_second_channel_strip_implementation() {
        let source = include_str!("insert.rs");
        let forbidden = [
            concat!("struct", " ChannelStrip", " {"),
            concat!("impl", " ChannelStrip"),
            concat!("struct", " ChannelStripParams", " {"),
            concat!("impl", " ChannelStripParams"),
            concat!("struct", " FilterParams", " {"),
            concat!("impl", " FilterParams"),
            concat!("fn", " render_frame("),
            concat!("fn", " observe_input("),
            concat!("fn", " process_stereo("),
            concat!("const", " EQ_CHUNK_FRAMES"),
        ];
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "engine 的 insert.rs 里出现了实现记号 `{needle}` —— 这里只允许投影 + `pub use`"
            );
        }
        assert!(
            source.contains("pub use yeban_dsp::channel_strip::{"),
            "engine 的 insert.rs 必须是 dsp 通道条与其参数类型的再导出"
        );
    }

    /// 判据：**没有已识别混响参数**的设备不武装混响（模块文档 §8.1 / §8.2）。
    ///
    /// ⚠ 这里钉住两个**刻意的拒绝**：`reverb_mix`（C0/C8 用来表达"弄不懂的效果器"）
    /// 与 `reverb_width`（单声道插入点没有作用面）。把任一个收下 ⇒ 本判据变红。
    #[test]
    fn devices_without_recognised_reverb_params_do_not_arm_a_reverb() {
        assert!(from_devices(&[]).reverb().is_none());
        assert!(
            from_devices(&[device(DeviceKind::InternalEffect, false, &[])])
                .reverb()
                .is_none()
        );
        // ⛔ `reverb_mix` 与 `reverb_width` 都不认（模块文档 §8.1）。
        assert!(
            from_devices(&[device(
                DeviceKind::InternalEffect,
                false,
                &[("reverb_mix", 0.3), ("reverb_width", 0.9)]
            )])
            .is_empty(),
            "`reverb_mix` / `reverb_width` 不许让一台设备变成混响"
        );
        // 内置乐器（音源）不是插入器件 —— 同一个名字也不认。
        assert!(
            from_devices(&[device(
                DeviceKind::InternalInstrument,
                false,
                &[("reverb_size", 0.8)]
            )])
            .reverb()
            .is_none()
        );
        // 旁通的效果器就是旁通：不许"取默认参数偷偷接回来"。
        assert!(
            from_devices(&[device(
                DeviceKind::InternalEffect,
                true,
                &[("reverb_size", 0.8)]
            )])
            .reverb()
            .is_none()
        );
    }

    /// **命中**：出现的名字覆盖默认值，未出现的字段保持器件的默认值（模块文档 §8.2）。
    ///
    /// 同时钉住"写一个**恰好等于默认值**的旋钮仍然算一台混响"（`seen` 与
    /// "字段 != 默认值"不是同一件事）。
    #[test]
    fn recognised_reverb_params_override_the_defaults_field_by_field() {
        let insert = from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[
                ("REVERB_SIZE", 0.9),
                ("reverb_damp", 0.1),
                ("reverb_wet", 0.6),
            ],
        )]);
        let params = insert.reverb().expect("必须武装混响");
        assert_eq!(params.size.to_bits(), 0.9f32.to_bits(), "名字大小写不敏感");
        assert_eq!(params.damp.to_bits(), 0.1f32.to_bits());
        assert_eq!(params.mix.to_bits(), 0.6f32.to_bits());
        // 未出现的字段 = 器件默认值（逐位）。
        let default = ReverbParams::default();
        assert_eq!(params.width.to_bits(), default.width.to_bits());
        assert_eq!(params.predelay.to_bits(), default.predelay.to_bits());
        // 混响**没有**级开关：这台设备不产生通道条（一个通道条旋钮都没写）。
        assert!(insert.strip().is_none(), "混响旋钮不许顺带启用通道条");

        // 写一个**恰好等于默认值**的旋钮 ⇒ 仍然是一台混响（`seen` 的口径）。
        let defaulted = from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[("reverb_size", default.size)],
        )]);
        assert!(
            defaulted.reverb().is_some(),
            "写了一个等于默认值的旋钮仍然说明这是一台混响"
        );
    }

    /// 两件器件**各自独立**取来源：同一台设备可以同时是通道条与混响的来源；
    /// 两台设备可以各出一件（模块文档 §8 的 `from_devices` 文档）。
    #[test]
    fn the_two_devices_take_their_sources_independently() {
        // 同一台设备同时带通道条与混响旋钮。
        let both = from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[("threshold_db", -20.0), ("reverb_size", 0.7)],
        )]);
        assert!(both.strip().is_some(), "同一台设备要能同时出通道条");
        assert!(both.reverb().is_some(), "同一台设备要能同时出混响");
        assert!(!both.is_empty());

        // 两台设备各出一件：先出现的那一台是各自的首个来源。
        let split = from_devices(&[
            device(DeviceKind::InternalEffect, false, &[("reverb_size", 0.7)]),
            device(DeviceKind::InternalEffect, false, &[("ratio", 4.0)]),
        ]);
        assert!(split.strip().is_some(), "后一台设备要能补上通道条");
        assert!(split.reverb().is_some(), "前一台设备要能补上混响");
        assert_eq!(
            split.reverb().expect("混响").size.to_bits(),
            0.7f32.to_bits()
        );

        // 首个来源不被后来的覆盖。
        let ordered = from_devices(&[
            device(DeviceKind::InternalEffect, false, &[("reverb_size", 0.7)]),
            device(DeviceKind::InternalEffect, false, &[("reverb_size", 0.2)]),
        ]);
        assert_eq!(
            ordered.reverb().expect("混响").size.to_bits(),
            0.7f32.to_bits(),
            "首个来源不被后来的覆盖"
        );
    }

    /// 判据：**engine 侧没有第二份混响实现**（源码级机械检查）。
    ///
    /// 与前两条同款、对准 `yeban_dsp::reverb`。注入：在本文件里复制 `Reverb` 的
    /// 类型定义、延迟线结构或它的任何一个逐样本内核 ⇒ 本判据立即变红。
    /// ⚠ 与前两条同样的自我命中风险：本条注释里也**不许**出现被禁字面量。
    #[test]
    fn engine_insert_module_has_no_second_reverb_implementation() {
        let source = include_str!("insert.rs");
        let forbidden = [
            concat!("struct", " Reverb", " {"),
            // ⚠ 这一条的记号必须带空格与花括号：不带花括号的版本会把本文件的
            // `impl ReverbProjection {` 一起命中（第一版实测就是这样自我变红的）。
            concat!("impl", " Reverb", " {"),
            concat!("struct", " ReverbParams", " {"),
            concat!("struct", " Comb", " {"),
            concat!("impl", " Comb"),
            concat!("struct", " Allpass", " {"),
            concat!("fn", " setup("),
            concat!("const", " COMB_TUNING"),
            concat!("const", " WET_GAIN"),
        ];
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "engine 的 insert.rs 里出现了实现记号 `{needle}` —— 这里只允许投影 + `pub use`"
            );
        }
        assert!(
            source.contains("pub use yeban_dsp::reverb::{Reverb, ReverbParams};"),
            "engine 的 insert.rs 必须把 dsp 混响的**类型与参数类型**都再导出（R7 的同一性判据靠它）"
        );
    }

    // -----------------------------------------------------------------------
    // 第三件器件：卷积混响（模块文档 §9）
    // -----------------------------------------------------------------------

    /// 判据：**没有已识别 `conv_` 参数**的设备不武装卷积混响（模块文档 §9.3）。
    ///
    /// ⚠ 这条同时钉住"`conv_` 前缀是刻意的"：`reverb_*` 与 `filter_cutoff_hz` 这两族名字
    /// **都不许**让一台设备变成卷积混响 —— 它们各自已经被另外两件器件（或"弄不懂的效果器"
    /// 那条既有判据）占用。把任一个收进 [`ConvProjection::apply`] ⇒ 本判据变红。
    #[test]
    fn devices_without_recognised_convolution_params_do_not_arm_a_convolution_reverb() {
        assert!(from_devices(&[]).convolution().is_none());
        assert!(
            from_devices(&[device(DeviceKind::InternalEffect, false, &[])])
                .convolution()
                .is_none()
        );
        // 别处已占用的名字：`reverb_*`（§8 的 Freeverb）与 `filter_cutoff_hz`（§4.3 的拒绝）。
        // ⚠ `conv_ir_seed` **不在**这一组里 —— 它是本器件认的名字（见下面第二条判据）。
        assert!(
            from_devices(&[device(
                DeviceKind::InternalEffect,
                false,
                &[
                    ("reverb_size", 0.9),
                    ("reverb_wet", 0.5),
                    ("filter_cutoff_hz", 800.0),
                    ("ir_frames", 4_800.0),
                ]
            )])
            .convolution()
            .is_none(),
            "只有 `conv_` 前缀的名字才让一台设备变成卷积混响"
        );
        // 内置乐器（音源）不是插入器件 —— 同一个名字也不认。
        assert!(
            from_devices(&[device(
                DeviceKind::InternalInstrument,
                false,
                &[("conv_wet", 0.5)]
            )])
            .convolution()
            .is_none()
        );
        // 旁通的效果器就是旁通：不许"取默认参数偷偷接回来"。
        assert!(
            from_devices(&[device(
                DeviceKind::InternalEffect,
                true,
                &[("conv_wet", 0.5)]
            )])
            .convolution()
            .is_none()
        );
    }

    /// **命中**：出现的 `conv_` 名字覆盖四个器件标量的基值，未出现的字段保持器件默认值；
    /// 写一个**恰好等于默认值**的旋钮仍然算一台卷积混响（`seen` 的口径，与 §8.2 同款）。
    #[test]
    fn recognised_convolution_params_override_the_defaults_field_by_field() {
        let insert = from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[
                ("CONV_WET", 0.6),
                ("conv_dry", 0.4),
                ("conv_predelay_s", 0.03),
                ("conv_ir_gain_db", -6.0),
            ],
        )]);
        let plan = insert.convolution().expect("必须武装卷积混响");
        let params = plan.params();
        assert_eq!(params.wet.to_bits(), 0.6f32.to_bits(), "名字大小写不敏感");
        assert_eq!(params.dry.to_bits(), 0.4f32.to_bits());
        assert_eq!(params.pre_delay_s.to_bits(), 0.03f32.to_bits());
        assert_eq!(params.ir_gain_db.to_bits(), (-6.0f32).to_bits());
        // 未出现的字段 = 器件默认值（逐位）。
        let default = ConvolutionReverbParams::default();
        assert_eq!(plan.ir_frames(), 4_800, "IR 帧数 = 采样率 ÷ 10");
        assert!(default.wet != params.wet, "上面那几条断言确实改到了东西");
        // 这台设备不产生通道条，也不产生 Freeverb 混响。
        assert!(insert.strip().is_none(), "`conv_` 旋钮不许顺带启用通道条");
        assert!(
            insert.reverb().is_none(),
            "`conv_` 旋钮不许顺带启用 Freeverb 混响"
        );

        // 写一个**恰好等于默认值**的旋钮 ⇒ 仍然是一台卷积混响。
        let defaulted = from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[("conv_wet", default.wet)],
        )]);
        assert!(
            defaulted.convolution().is_some(),
            "写了一个等于默认值的旋钮仍然说明这是一台卷积混响"
        );
    }

    /// 判据：**合成 IR 是确定的**（[ARCH-DET-001]），并且三个输入各自真的起作用。
    ///
    /// 三条子判据：
    ///
    /// 1. **同输入 ⇒ 逐位相同**（`to_bits` 全等）：这是"跨运行/跨架构可复现"的机械形式。
    ///    该函数的全部运算都在 [ADR-0001 D32] 的 IEEE 精确类里（`Rng` 的整数移位与两次
    ///    算术 ＋ `libm::expf` ＋ `abs`/`max`/除法）⇒ 这条断言**不受**平台差异影响。
    /// 2. **不同种子 ⇒ 不同 IR**（否则"种子"就是一个假旋钮）。
    /// 3. **不同衰减 ⇒ 不同 IR**（否则"衰减"就是一个假旋钮）。
    #[test]
    fn the_synthesised_impulse_response_is_deterministic_and_its_knobs_bite() {
        let frames = convolution_ir_frames(TEST_SR);
        let a = synthesise_impulse_response(frames, 0.35, TEST_SR, 7);
        let b = synthesise_impulse_response(frames, 0.35, TEST_SR, 7);
        assert_eq!(a.len(), frames);
        assert_eq!(a.len(), 4_800, "48 kHz ⇒ 4 800 帧");
        assert!(
            a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()),
            "同一个 (frames, decay, sr, seed) 必须逐位可复现"
        );
        // 峰值归一化到 1.0（逐位）。
        let peak = a.iter().fold(0.0f32, |acc, v| acc.max(v.abs()));
        assert_eq!(peak.to_bits(), 1.0f32.to_bits(), "IR 必须峰值归一化到 1.0");
        // 每个样本都是有限值（器件的 IR 校验会拒绝非有限值 ⇒ 这台器件会整台不工作）。
        assert!(a.iter().all(|v| v.is_finite()));

        let other_seed = synthesise_impulse_response(frames, 0.35, TEST_SR, 8);
        assert!(
            a.iter()
                .zip(&other_seed)
                .any(|(x, y)| x.to_bits() != y.to_bits()),
            "换了种子 IR 必须变（否则 `conv_ir_seed` 是假旋钮）"
        );
        let other_decay = synthesise_impulse_response(frames, 0.05, TEST_SR, 7);
        assert!(
            a.iter()
                .zip(&other_decay)
                .any(|(x, y)| x.to_bits() != y.to_bits()),
            "换了衰减 IR 必须变（否则 `conv_ir_decay_s` 是假旋钮）"
        );
    }

    /// 判据：**IR 帧数只由采样率决定，且与实时侧池的预建长度同源**（模块文档 §9.2）。
    ///
    /// 这条等式是"快照边界上的换 IR 零分配"的**全部前提**：长度一旦不同，
    /// `Convolution::set_impulse_response` 就会重建缓冲（分配）。
    ///
    /// ⚠ 注入：把 [`convolution_ir_frames`] 的除数从 10 改成别的值 ⇒ 本判据的红行是
    /// `48_000 Hz ⇒ 4 800 帧`；把 `into_plan` 里的 `frames` 换成别的表达式 ⇒
    /// `计划里的 IR 长度必须等于 convolution_ir_frames` 那一条变红。
    #[test]
    fn the_plan_and_the_pool_agree_on_the_ir_length() {
        assert_eq!(convolution_ir_frames(48_000), 4_800);
        assert_eq!(convolution_ir_frames(44_100), 4_410);
        assert_eq!(convolution_ir_frames(96_000), 9_600);
        // 退化采样率也要给出 ≥ 1 帧：空 IR 在器件里是"拒绝"，不是"静音"。
        assert_eq!(convolution_ir_frames(0), 1);
        assert_eq!(convolution_ir_frames(9), 1);

        let plan_holder = from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[("conv_wet", 0.5)],
        )]);
        let plan = plan_holder.convolution().expect("必须武装卷积混响");
        assert_eq!(plan.ir_frames(), convolution_ir_frames(TEST_SR));
        assert_eq!(plan.ir().len(), plan.silence().len(), "四条通路必须等长");
        assert!(
            plan.silence().iter().all(|v| *v == 0.0),
            "交叉通路必须是全零（留空在器件里是**直通**，会把对侧信号漏过来）"
        );
    }

    /// 判据：**投影真的把两个 IR 旋钮交给了合成器**（`into_plan` 这条接线）。
    ///
    /// ⚠ 这条判据是**注入换来的**：`line/engine-7` 的注入 H 把 `into_plan` 里的
    /// `synthesise_impulse_response(frames, decay_s, sample_rate, self.seed)` 换成
    /// 三个常数，于是 `tests/convolution_insert.rs` 的三条判据变红，而
    /// [`the_synthesised_impulse_response_is_deterministic_and_its_knobs_bite`]
    /// **没有**变红 —— 因为那一条直接调**纯函数**，不经过 `into_plan`。
    /// 纯函数正确 ＋ 接线丢掉旋钮 = 一个假旋钮，而上面那条判据看不见它。
    /// 本条补的正是那一格：两条路径各钉一半。
    #[test]
    fn the_projection_really_hands_the_ir_knobs_to_the_generator() {
        let plan = |params: &[(&str, f32)]| {
            from_devices(&[device(DeviceKind::InternalEffect, false, params)])
                .convolution()
                .expect("`conv_` 名字出现过 ⇒ 必须武装")
                .ir_hash()
        };
        let short = plan(&[("conv_ir_decay_s", 0.05)]);
        let long = plan(&[("conv_ir_decay_s", 0.5)]);
        assert_ne!(short, long, "`conv_ir_decay_s` 必须真的改变 IR");
        let other_seed = plan(&[("conv_ir_seed", 9.0)]);
        assert_ne!(
            plan(&[("conv_ir_seed", 1.0)]),
            other_seed,
            "`conv_ir_seed` 必须真的改变 IR"
        );
        // 两个旋钮都写 ⇒ 与只写一个都不同（不是"某个旋钮被另一个覆盖"）。
        let both = plan(&[("conv_ir_decay_s", 0.05), ("conv_ir_seed", 9.0)]);
        assert_ne!(both, short);
        assert_ne!(both, other_seed);
    }

    /// 判据：**非法 IR 旋钮有定义**（非有限值回落缺省、超界钳制），并且整台器件仍能用。
    ///
    /// 理由：IR 是**配置期数据**，器件对非有限/溢出的 IR 只有一条出路 —— 拒绝并回到
    /// 未配置的直通。投影的职责是让"快照里存的那一份"本身合法，而不是把拒绝留给音频线程。
    #[test]
    fn degenerate_convolution_knobs_fall_back_to_defined_values() {
        let nan = from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[("conv_ir_decay_s", f32::NAN), ("conv_ir_seed", f32::NAN)],
        )]);
        let plan = nan.convolution().expect("`conv_` 名字出现过 ⇒ 必须武装");
        // ⭐ **R114／R125（本判据的实缺口）**：空 IR 是**缺陷**（器件没产出）⇒
        // 先钉**集合非空**，否则下面的 `all(..)` 在空 IR 上**恒真**、什么也没测。
        assert!(
            !plan.ir().is_empty(),
            "夹具前提：IR 必须非空（空 IR 会让下面的 all(..) 恒真）"
        );
        assert!(
            plan.ir().iter().all(|v| v.is_finite()),
            "非有限旋钮不许产出非有限 IR（那会让器件整台拒绝）"
        );
        assert_eq!(plan.ir_frames(), 4_800);

        // 极大衰减：钳到上界后仍是有限 IR（不是 `inf` 包络、不是全 1）。
        let huge = from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[("conv_ir_decay_s", 1e30)],
        )]);
        let huge_ir = huge.convolution().expect("必须武装").ir().to_vec();
        assert!(
            !huge_ir.is_empty(),
            "夹具前提：极大衰减的 IR 必须非空（空 IR 会让 all(..) 恒真）"
        );
        assert!(huge_ir.iter().all(|v| v.is_finite()));

        // 负种子：`f32 → u32` 是饱和转换 ⇒ 0 ⇒ `Rng::new` 换成内部常数。有定义、不 panic。
        let negative = from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[("conv_ir_seed", -123.0)],
        )]);
        let negative_ir = negative.convolution().expect("必须武装").ir().to_vec();
        assert!(
            !negative_ir.is_empty(),
            "夹具前提：负种子的 IR 必须非空（空 IR 会让 all(..) 恒真）"
        );
        assert!(negative_ir.iter().all(|v| v.is_finite()));
    }

    /// 三件器件**各自独立**取来源：同一台设备可以同时是三者的来源（模块文档 §9 的
    /// `from_devices` 文档）。
    #[test]
    fn the_three_devices_take_their_sources_independently() {
        let all_three = from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[
                ("threshold_db", -20.0),
                ("reverb_size", 0.7),
                ("conv_wet", 0.5),
            ],
        )]);
        assert!(all_three.strip().is_some(), "同一台设备要能同时出通道条");
        assert!(all_three.reverb().is_some(), "同一台设备要能同时出混响");
        assert!(
            all_three.convolution().is_some(),
            "同一台设备要能同时出卷积混响"
        );
        assert!(!all_three.is_empty());

        // 首个来源不被后来的覆盖（三件各一条）。
        let ordered = from_devices(&[
            device(DeviceKind::InternalEffect, false, &[("conv_wet", 0.5)]),
            device(DeviceKind::InternalEffect, false, &[("conv_wet", 0.1)]),
        ]);
        assert_eq!(
            ordered
                .convolution()
                .expect("卷积混响")
                .params()
                .wet
                .to_bits(),
            0.5f32.to_bits(),
            "首个来源不被后来的覆盖"
        );
    }

    /// 判据：**engine 侧没有第二份卷积混响实现**（源码级机械检查）。
    ///
    /// 与前三件同款、对准 `yeban_dsp::convolution_reverb`（它下面还有 `convolution` 与
    /// `convolution_stereo` 两层）。注入：在本文件里复制 `ConvolutionReverb` 的类型定义、
    /// 分块/FFT 内核或它的湿干混合公式 ⇒ 本判据立即变红。
    /// ⚠ 与前三件同样的自我命中风险：本条注释里也**不许**出现被禁字面量。
    #[test]
    fn engine_insert_module_has_no_second_convolution_reverb_implementation() {
        let source = include_str!("insert.rs");
        let forbidden = [
            concat!("struct", " ConvolutionReverb", " {"),
            concat!("struct", " ConvolutionReverbParams", " {"),
            concat!("struct", " Convolution", " {"),
            concat!("struct", " TrueStereoConvolution", " {"),
            concat!("fn", " fft_radix2("),
            concat!("fn", " process_chunk("),
            concat!("fn", " ensure_delay_lines("),
            concat!("const", " CONV_BLOCK_FRAMES"),
            concat!("const", " CONV_BINS"),
            concat!("const", " CONV_FFT_FRAMES"),
        ];
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "engine 的 insert.rs 里出现了实现记号 `{needle}` —— 这里只允许投影 + `pub use`"
            );
        }
        assert!(
            source.contains(
                "pub use yeban_dsp::convolution_reverb::{ConvolutionReverb, ConvolutionReverbParams};"
            ),
            "engine 的 insert.rs 必须把 dsp 卷积混响的**类型与参数类型**都再导出\
             （V7 的同一性判据靠它）"
        );
    }
}
