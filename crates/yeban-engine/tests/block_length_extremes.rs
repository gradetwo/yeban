//! `line/engine-25` 的判据：**块长度极值**（类别⑦）——「设备缓冲长度」与引擎的
//! 固定 128 帧处理量子、以及 PDC 延迟线之间的交互。
//! `[ARCH-DET-001, ARCH-PDC-001, ARCH-PDC-002, ARCH-RT-001]`
//!
//! 全部判据都**不需要** `device` feature ⇒ 在 `--no-default-features`（不编译 cpal）下
//! 即可运行：走的是产品路径
//! `YebanProjectV1` → `EngineSnapshot::from_project` → `SnapshotSlot` →
//! `EngineRuntime::process_quantum`，没有测试专用捷径、没有声卡。
//!
//! # 1. 量的是哪一件事（单位：帧）
//!
//! 宿主把**设备缓冲**（cpal 回调交给引擎的那一段交错样本）的帧数交给
//! [`EngineRuntime::process_quantum`]。本文件量的是：**同一个工程、同样的总帧数，
//! 只把"每次回调的帧数"换掉，渲染出来的 PCM 与读数是否逐位相同**。
//!
//! 为什么这仍是类别⑦（块长度／范围极值）的空白而不是重复劳动——**已提交判据的射程**
//! （逐条核过，不是印象）：
//!
//! | 已提交判据 | 块长覆盖面 | ⛔ 它**不**量什么 |
//! | :--- | :--- | :--- |
//! | `tests/rt_zero_alloc.rs` 判据 ⑬ | **1/2/3/127/128/129/1024/1025 帧 × 25 轮 + 非帧对齐缓冲**，跑 `filled_project`（**含非零 PDC 补偿**） | 它只断言**四元组全 0**（alloc/dealloc/lock/io）与"帧对齐缓冲无哨兵残留 / 量子数"⇒ **不比较不同块长之间的 PCM**，也不量 PDC 的**逐位对齐**、不量电平读数 |
//! | `tests/pdc_mix_path.rs`（P1~P4，本 crate 的 PDC 契约文件） | **只有 128 帧**（辅助函数写死 `vec![0.0f32; 128 * 2]`） | 块长 ≠ 128 时的对齐（实测：一条"只在整量子补偿"的注入能让它 **4 passed; 0 failed**，见 §5 I1） |
//! | `tests/mix_render.rs` / `synth_*.rs` / 各器件契约文件 | **只有 128 帧** | 同上 |
//!
//! 也就是说：`frames != 128` 这条路径**被走到过**（⑬ 走的就是它），但
//! **它的音频后果从未被比较过**——而 `frames` 恰恰是引擎里每一个逐块器件（PDC 延迟线、
//! 通道条、Freeverb、卷积混响、电平表、节拍器）的入参。本文件补的就是这一半。
//!
//! # 2. 判据表（每条都写清"怎么变红"）
//!
//! | 编号 | 判据 | 怎么变红（注入） | 实测 |
//! | :--- | :--- | :--- | :--- |
//! | ⑦-1 | 设备缓冲长度不改变渲染输出：六个夹具 × 九种块长（含**一次回调整段**与**每回调 1 帧**）对 128 帧基线**逐位**相同（左右两声道） | 把 PDC 的 `apply` 只放在"整量子"分支里（`frames == 128` 才补偿）⇒ 块长 1/64/100/127/129 全部变红 | 见 §5 注入 I1 |
//! | ⑦-2 | PDC 补偿在**任意**块长下逐位精确：`aligned[t] == reference[t − 400]`，且前 400 帧逐位 `+0.0` | 同上；另把延迟线的读写指针约定写反 | 见 §5 注入 I1/I2 |
//! | ⑦-2b | 并联两条支路在**任意**块长下仍然采样级同相：`aligned[t] == single[t − 400] + single[t − 400]`（逐位） | 同上；把 `set_delay` 的读写指针约定写反 | 见 §5 注入 I2 |
//! | ⑦-3 | 一次回调内部按 128 帧切成整量子；**短于 128 帧的尾部就是一块**（这条钉住 ⑦-4/⑦-5 的成因） | `process_quantum` 丢掉尾部块 / 把尾部补零成整量子 | 见 §4 发现 3 |
//! | ⑦-4 | **电平读数**（逐节点峰值/峰值保持/RMS/平滑 RMS）在"设备缓冲长度是 128 的整数倍"时逐位相同（128 / 256 / 384 / 一次回调整段） | 把 `quanta_per_second` 改成按**回调**数折算而不是按**块**数折算 | 见 §5 注入 I1（同一条 `frames` 依赖） |
//! | ⑦-5 | 卷积插入级在**块长是 128 的整数倍**时与 128 帧基线逐位相同 | 让 `ConvolutionReverb::process` 的子块切分锚在绝对时间之外的东西上 | 非整数倍块长的分歧见 §4 发现 1（⛔ 不判据） |
//!
//! ⛔ **本文件不冻结任何跨架构常量**（[裁决 R24]）：⑦-1/⑦-2/⑦-2b/⑦-4/⑦-5 比较的都是**同一台
//! 机器、同一个进程内两次运行**的结果（同一份代码、同一组输入，只有块长不同），
//! 因此不涉及"两个架构的超越函数相差若干 ulp"的问题。唯一与架构无关的硬断言是
//! ⑦-2 的"延迟线前 400 帧是 `+0.0`"——那是延迟线的**逐位初始状态**（纯位模式）。
//!
//! # 3. 机械枚举：块长在引擎里的全部入口
//!
//! `frames`（每次 [`EngineRuntime::process_quantum`] 内部一块的帧数）进入的**逐块**
//! 调用点（量法：读 `crates/yeban-engine/src/rt.rs` 的 `render_block`，`frames` 的
//! 使用点逐个列出；括号里的"现位于第 N 行"以本票的树为准）：
//!
//! | # | 调用点 | 块长依赖 | 本文件的见证 |
//! | :-: | :--- | :--- | :--- |
//! | 1 | 走带/参数/音符事件出队（每块一次） | **有**：同一批事件在哪一块生效由块边界决定 | ⛔ 不计入（见 §4 发现 4） |
//! | 2 | 逐轨参数平滑 `params.apply` | 无（逐样本状态） | ⑦-1 裸合成器夹具 |
//! | 3 | 主总线参数平滑 `params.apply_master` | 无（逐样本状态） | ⛔ 未单独见证：本文件不注入事件 ⇒ 主总线增益槽位未武装（见 §4 发现 4） |
//! | 4 | 声部池渲染 `synth.render_track` | 无（逐样本状态） | ⑦-1 裸合成器夹具 |
//! | 5 | 通道条 `strip.process_mono` | 无（逐样本状态；`frames` 只是切片长度） | ⑦-1 通道条夹具 |
//! | 6 | Freeverb `reverb.process` | 无（逐样本状态） | ⑦-1 混响夹具 |
//! | 7 | 卷积混响 `conv.process`（交错立体声） | **有**：子块网格锚在**每次调用**的起点 | ⑦-5（整数倍块长）＋ §4 发现 1（非整数倍） |
//! | 8 | 电平 `bank.measure` / `measure_bus_stereo` | **有**：峰值保持的释放是**每次调用**一个乘子 | ⑦-4（整数倍块长）＋ §4 发现 2 |
//! | 9 | PDC `pdc.apply` | 无（环形延迟线的块边界天然连续） | ⑦-2 / ⑦-2b 全部块长 |
//! | 10 | 声相与母线求和 `sum_into_bus` | 无（逐样本） | ⑦-1 |
//! | 11 | 节拍器 `render_metronome_quantum` | 无（逐样本） | ⑦-1 节拍器夹具 |
//! | 12 | 主总线推子 `scale_bus` | 无（逐样本） | ⑦-1（推子恒在链上；非单位增益见 §4 发现 4 的边界） |
//! | 13 | 母线限制器 `limiter.process_stereo` | 无（逐样本前瞻） | ⑦-1（限制器在链上恒在） |
//!
//! 每一项都有结论：1/7/8 是**真有**块长依赖的三处，其中 1 属**设计口径**（控制面事件
//! 在块边界生效，见 §4 发现 4），7/8 是 §4 的两条发现；3 未单独见证（理由见该行）。
//! 其余九处的块长无关性由 ⑦-1 的六个夹具实测覆盖。
//!
//! # 4. 本票发现（**不改**，只报告 + 请求裁决）
//!
//! **发现 1（卷积混响）：设备缓冲长度不是 128 的整数倍时，渲染输出改变。**
//! 器件自己的模块文档（`crates/yeban-dsp/src/convolution_reverb.rs` 的 §4，现位于
//! 第 118 行起）写明：子块网格锚在**每次调用**的起点，**调用方必须用固定块长**
//! （"引擎的量子恒为 128 帧"），块长抖动会让网格相对绝对时间漂移；短块按零补齐
//! 推进历史。而 `process_quantum` 把"非 128 整数倍的设备缓冲"的**尾部**直接作为
//! 一块交给器件 ⇒ 每次回调都出现一个短块。
//!
//! 量法：同一条 MIDI 轨（四个音符）挂一台全湿卷积混响
//! （`conv_dry = 0`、`conv_wet = 1`、`conv_ir_decay_s = 0.35`、`conv_ir_seed = 7`），
//! 总窗口 8192 帧，左声道逐位比对。这条量法已落在 ⑦-5（`--nocapture` 打印；
//! 整数倍那一半是**判据**，非整数倍那一半只是读数），命令：
//! `cargo test -p yeban-engine --no-default-features --test block_length_extremes convolution_insert -- --nocapture`。
//! **字面读数**（帧号从 0 起；值为 `f32` 十进制的 9 位有效数字）：
//!
//! ```text
//! 基线块长  128：非零样本 8159；块长 256 / 384 / 512 / 8192（一次回调）：**逐位相同**（判据）
//! [engine-blk/7-5-finding1] 块长  129: 首个差异 @138  8.53335738e-1 vs 5.68071604e-1；差异样本 8054 / 8192
//! [engine-blk/7-5-finding1] 块长  127: 首个差异 @ 33 −2.26539314e-7 vs −7.37569863e-8；差异样本 8147 / 8192
//! [engine-blk/7-5-finding1] 块长  100: 首个差异 @ 33 −2.26539314e-7 vs  8.42936956e-8；差异样本 8151 / 8192
//! [engine-blk/7-5-finding1] 块长   64: 首个差异 @ 33 −2.26539314e-7 vs  5.26835606e-8；差异样本 8157 / 8192
//! [engine-blk/7-5-finding1] 块长    1: 首个差异 @ 33 −2.26539314e-7 vs  0e0；       差异样本 8159 / 8192
//! ```
//!
//! ⇒ **整数倍块长逐位相同**（子块仍是 128 帧，网格不漂移），**非整数倍块长整段不同**。
//! 这是 [ARCH-DET-001]（L1 逐位确定性）与"同一工程在任何宿主缓冲设置下同声"之间
//! 的一条**真实缺口**，而它不在任何已提交判据的射程内（⑦-1 因此**刻意不含**卷积夹具：
//! 把一条已知会红的组合写进"必须逐位相同"的判据里等于提交一个红判据）。
//!
//! **发现 2（电平弹道）：同样的情形下，峰值保持的释放速率按 `128 ÷ 实际块长` 倍偏快。**
//! `LevelDetector` 的峰值保持是**每次 `analyze` 调用**乘一个释放因子，该因子由
//! `quanta_per_second` 折算（`crates/yeban-dsp/src/meter.rs` 的 `commit`，现位于第 347 行）；
//! 而引擎武装的 `quanta_per_second` 恒为 `sample_rate / DEFAULT_BLOCK_FRAMES`
//! （= 375），与**实际**每秒的 `analyze` 次数无关（`crates/yeban-engine/src/rt.rs`
//! 的快照边界分支，现位于第 1754 行）。块长为 128 时两者一致（375 次/秒）；块长为 100
//! 时实际是 480 次/秒 ⇒ 释放快 1.28 倍。
//!
//! 量法：一个 1 tick（= 25 帧）长的音符后接静音，渲染 48 000 帧（1.0 s），从电平队列里取
//! **第一条普通轨**的第一帧与最后一帧的 `peak_hold`，折算 `20·log10(首/末)` dB。
//! 这条量法已落在 ⑦-4 的末尾（**打印**，⛔ 不当判据），命令：
//! `cargo test -p yeban-engine --no-default-features --test block_length_extremes meter_readings -- --nocapture`。
//! **字面读数**（单位：dB / 1.0 s；契约值是 **20 dB/s**）：
//!
//! ```text
//! [engine-blk/7-4-finding2] 块长  128: 该节点峰值保持首 = 9.446515e-2，末 = 9.50476e-3  ⇒ 释放 19.947 dB
//! [engine-blk/7-4-finding2] 块长  100: 该节点峰值保持首 = 9.446515e-2，末 = 4.9881777e-3 ⇒ 释放 25.547 dB
//! [engine-blk/7-4-finding2] 块长   64: 该节点峰值保持首 = 9.446515e-2，末 = 9.50482e-4  ⇒ 释放 39.947 dB
//! ```
//!
//! 即 128 → 19.947（≈ 契约的 20）、100 → 25.547（1.28 倍 = 128/100）、64 → 39.947（2 倍 = 128/64）。
//!
//! 这与 `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` 现位于第 805 行的 HD-28 登记
//! **同一族**（那里修掉的是"把设备缓冲当量子"⇒ 10 dB/s 而不是 20 dB/s；判据是
//! `meter_ballistics_follow_the_processing_quantum`）。那条判据钉的是"武装进去的数
//! 恒为 375"，它**看不见**"每秒实际调用次数不等于 375"这一半。
//!
//! **发现 3（共同成因，供裁决）：`process_quantum` 的尾部块不是整量子。**
//! 规范原文（`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §HD-28，现位于第 805 行）
//! 说"引擎把任意长度的设备缓冲**按 128 切成整量子**"。实现是
//! `let frames = (total_frames - offset).min(DEFAULT_BLOCK_FRAMES);`：一次回调内部的
//! **切分**符合 128，但**最后一块可以短于 128**（200 帧 ⇒ 128 + 72，已由
//! `src/rt.rs` 的同名单元判据钉住 `last_block().frames() == 72`）。在"设备缓冲长度不是
//! 128 的整数倍"的宿主上（例如 100 帧），**每一次**回调的最后一块都短于 128
//! ⇒ 引擎的实际处理块长恒为 100，而不是契约的 128。发现 1 与发现 2 都是它的下游。
//!
//! **裁决请求（措辞建议）**：让引擎在回调之间**捎带不足一个量子的余量**
//! （每次 `render_block(DEFAULT_BLOCK_FRAMES)`，把多出来的输出样本留到下一次回调再发出），
//! 使处理量子恒为 128。代价与影响面必须一并授权：
//!
//! | 影响 | 内容 |
//! | :--- | :--- |
//! | 语义 | `EngineRuntime::last_block()` 的帧数不再等于"设备缓冲的余数"（`src/rt.rs` 的 `partial_and_multi_quantum_buffers_are_split_at_the_boundary` 断言 `72` 必须同步改写） |
//! | 读数 | `EngineStats::quanta` 变成"总帧数 ÷ 128 向上取整"，与设备缓冲长度无关 |
//! | 音频 | **只有**"设备缓冲长度不是 128 整数倍"的宿主会变声（既有判据全部用 128 帧回调 ⇒ 逐位不变） |
//! | 多出的状态 | 一块输出余量缓冲（`[f32; 128]` × 声道数，构造期分配）⇒ 仍满足 [MUST-GATE-001] |
//!
//! 在裁决落地之前，本文件**不改**渲染输出：⑦-1 的六个夹具、⑦-2/⑦-3/⑦-4 全部按
//! 当前实现钉住"成立的那一半"，发现 1/2 只登记读数（⛔ 不把缺陷写成期望值）。
//!
//! **发现 4（不是缺陷，口径澄清）：控制面事件在块边界生效。**
//! 同一个 `SetParam` 事件在"块长 128"与"块长 1"下会在不同时刻生效（前者最多晚 127 帧），
//! 这是 `src/rt.rs` 明说的语义（"命令在量子边界按 FIFO 顺序应用"），不是块长缺陷
//! ⇒ ⑦-1 **不注入任何事件**（否则判据会把设计口径当成缺陷）。
//!
//! # 5. 注入实测（证明判据有判别力；三条都已整体回退，`sha256` 证明逐字节回原）
//!
//! 命令统一为
//! `cargo test -p yeban-engine --no-default-features --test <目标>`（`CARGO_HOME` /
//! `RUSTUP_TOOLCHAIN=stable` 按本仓库的本机纪律）。
//!
//! **I1 —— 把 PDC 的补偿限制在整量子里**（改 `crates/yeban-engine/src/rt.rs` 的逐轨循环，
//! 现位于第 2241 行：`if frames == DEFAULT_BLOCK_FRAMES { self.pdc.apply(…) }`）。
//! 语义是"块长不足 128 就不补偿"。字面红行：
//!
//! ```text
//! test pdc_alignment_is_sample_exact_at_every_device_buffer_length ... FAILED
//!   assertion `left == right` failed: 块长 129 · 左声道 第 161 帧：PDC 延迟线前 400 帧
//!   必须是**逐位 `+0.0`**（延迟线的历史是零），实测 -1.0586074e-1
//! test pdc_diamond_stays_in_phase_at_every_device_buffer_length ... FAILED
//!   assertion `left == right` failed: 块长 129 第 161 帧：两条支路都还没到达求和节点，
//!   必须是逐位 `+0.0`
//! test device_buffer_length_never_changes_the_rendered_audio ... FAILED
//!   PDC 两条并联支路（D(fast) = 400）：设备缓冲长度 192 帧必须与 128 帧**逐位**相同；
//!   实测第一处差异（帧, 位模式）= 左 Some((161, 0, 3185102212)) / 右 Some((161, 0, 3185102212))
//! test result: FAILED. 0 passed; 3 failed; 0 ignored; 0 measured; 2 filtered out
//! （这次跑用了 `device_buffer` 过滤，命中的正是 ⑦-1/⑦-2/⑦-2b 三条；
//! ⑦-3 与 ⑦-4 与 PDC 无关，本来就不该被这条注入打红）
//! ```
//!
//! ⚠ 同一条注入下 `cargo test … --test pdc_mix_path` 是 **`4 passed; 0 failed`**：
//! 已提交的四条 PDC 判据（P1/P2/P3/P4）**全部看不见**它 —— 因为它们只跑 128 帧块。
//! 这就是 ⑦-1/⑦-2/⑦-2b 必须存在的机械理由（也是本票的形式 D 发现）。
//!
//! **I2 —— 整体删掉 `self.pdc.apply(…)` 调用**（同一行）。字面红行：
//!
//! ```text
//! test pdc_compensation_is_applied_on_the_real_mix_path_sample_exactly ... FAILED
//!   assertion `left == right` failed: 左声道 第 34 帧：PDC 延迟线前 400 帧必须是**逐位静音**
//!   （延迟线的历史是零）；实测 4.0652166e-4   left: 970269290  right: 0
//! test pdc_lines_up_a_parallel_diamond_sample_exactly_at_the_summing_node ... FAILED
//!   assertion `left == right` failed: 第 34 帧：两条支路都还没到达求和节点，必须是逐位静音…
//!   left: 970269290  right: 0
//! test result: FAILED. 2 passed; 2 failed
//! ```
//!
//! ⇒ `tests/pdc_mix_path.rs` 的 P1/P2 **有牙**（I2 变红），只是**只对"整量子"这一种块长有牙**
//! （I1 全绿）。本文件的 ⑦-* 在同一注射下同样变红（3 failed）。
//!
//! **I3 —— 在 `render_block` 的调用树里加一次堆分配**（`drop(core::hint::black_box(
//! Vec::<u8>::with_capacity(1)))`，同一文件、探针入口之后）。字面红行
//! （`cargo test -p yeban-engine --no-default-features --test synth_rt_zero_alloc`）：
//!
//! ```text
//! [engine-sound/J5] FAIL: 合成路径在实时窗口内分配了 10000 次堆内存 —— [MUST-GATE-001] 一票否决
//! [engine-sound/J5] FAIL: 合成路径在实时窗口内释放了 10000 次 —— [MUST-GATE-001] 一票否决
//! [engine-sound/J5] FAIL: 快照交换期间的实时路径分配了 1 次（revision=2）
//! ```
//!
//! ⇒ 同一批 `frames` 依赖路径上的零分配判据仍有判别力。
//!
//! 回退证据：`crates/yeban-engine/src/rt.rs` 的三次注入前后
//! `sha256` 恒为 `a004b7be196b8a8a37c1b590d75223a3e844b53bf3e8c9d38ddd499945a8b03b`，
//! `cmp` 逐字节相同。

use std::collections::BTreeMap;

use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_engine::meter::{MeterFrame, meter_channel};
use yeban_engine::ring::event_channel;
use yeban_engine::rt::{EngineRuntime, EngineStats};
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition, DeviceKind, EntityId, LoopConfig,
    MidiNote, ParameterValue, RoutingEdge, RoutingGraph, RoutingKind, TrackKind, TrackV3,
    YebanProjectV1,
};

/// 判据窗口（帧）：64 个整量子。
///
/// 取 64 个量子而不是"一个量子"是刻意的：块长 1 与块长 8192（一次回调整段）两个极值
/// 都必须跨越**多个**量子，否则"跨回调的块长依赖"观察不到。
const TOTAL_FRAMES: usize = 64 * DEFAULT_BLOCK_FRAMES;

/// 被测的块长（单位：帧）。
///
/// - `TOTAL_FRAMES`：**一次回调整段**（引擎内部切成 64 个量子）；
/// - `384` / `256`：128 的整数倍（`3×`、`2×`）；
/// - `192 = 128 + 64`：一次回调内部一个整量子 + 一个短块；
/// - `129 = 128 + 1`：同上，短块为 1 帧；
/// - `127` / `100` / `64`：每次回调都**短于**一个量子；
/// - `1`：极值，每次回调 1 帧。
const CHUNKS: [usize; 9] = [TOTAL_FRAMES, 384, 256, 192, 129, 127, 100, 64, 1];

/// 夹具栅格：120 BPM / 48 kHz / 960 PPQ ⇒ 1 tick = 25 样本（与 `tests/pdc_mix_path.rs` 同源）。
const SAMPLES_PER_TICK: u64 = 25;

/// 慢支路设备链上报的自身延迟（采样点）。400 = 16 tick，正好落在夹具栅格上。
const SLOW_LATENCY: u32 = 400;

/// 慢支路的乐器被触发的 tick（把设备链延迟平移到音符起点上，见 `tests/pdc_mix_path.rs`
/// 的边界登记第 2 条）。
const SLOW_START_TICK: u64 = SLOW_LATENCY as u64 / SAMPLES_PER_TICK;

/// 音符时值（tick）。
const NOTE_TICKS: u64 = 960;

/// 力度刻意取小：两支路叠加后仍在母线限制器阈值之下 ⇒ 限制器是**纯 33 帧延迟**。
const VELOCITY: u8 = 40;

/// 一次渲染的全部输出（两条声道逐帧展开 + 累计读数 + 电平队列里的帧）。
struct Rendered {
    left: Vec<f32>,
    right: Vec<f32>,
    stats: EngineStats,
    meters: Vec<MeterFrame>,
    /// 最后一次 `render_block` 的帧数（= `EngineRuntime::last_block().frames()`）。
    last_block_frames: usize,
}

/// 一张效果器设备。
fn effect(name: &str, params: &[(&str, f32)]) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: name.to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
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

/// 一台内置乐器设备（鼓机用）。
fn instrument(params: &[(&str, f32)]) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: "Instrument".to_owned(),
        kind: DeviceKind::InternalInstrument,
        bypassed: false,
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

/// 一条 MIDI 轨 + 可选音符片段。`notes` 的每一项是 `(起点 tick, 音高, 力度)`，
/// 时值统一取 `duration_ticks`。
fn note_track(
    id: EntityId,
    name: &str,
    notes: &[(u64, u8, u8)],
    duration_ticks: u64,
) -> (TrackV3, Option<ClipPoolEntry>) {
    let mut track = TrackV3 {
        id,
        name: name.to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    if notes.is_empty() {
        return (track, None);
    }
    let clip = EntityId::new();
    let placement = EntityId::new();
    let mut note_map = BTreeMap::new();
    let mut end_tick = 0u64;
    for (start_tick, pitch, velocity) in notes {
        let note_id = EntityId::new();
        let mut note = MidiNote::new(note_id, *start_tick, *pitch, duration_ticks);
        note.velocity = *velocity;
        note_map.insert(note_id, note);
        end_tick = end_tick.max(start_tick + duration_ticks);
    }
    let entry = ClipPoolEntry {
        id: clip,
        name: format!("{name} clip"),
        content: ClipContent::Midi { notes: note_map },
    };
    track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: end_tick,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );
    (track, Some(entry))
}

/// 单条 MIDI 轨（可挂设备链、可开节拍器）直连母线的工程。
fn chain_project(
    devices: Vec<DeviceDefinition>,
    notes: &[(u64, u8, u8)],
    duration_ticks: u64,
    metronome: bool,
) -> YebanProjectV1 {
    let master = EntityId::new();
    let track = EntityId::new();
    let (mut track_def, clip) = note_track(track, "Track", notes, duration_ticks);
    track_def.devices = devices;

    let mut tracks = BTreeMap::new();
    tracks.insert(track, track_def);
    tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    let mut clip_pool = BTreeMap::new();
    if let Some(clip) = clip {
        clip_pool.insert(clip.id, clip);
    }
    let mut routing = RoutingGraph {
        nodes: vec![track, master],
        ..RoutingGraph::default()
    };
    let id = EntityId::new();
    routing.edges.insert(
        id,
        RoutingEdge {
            id,
            source_node: track,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
    let mut project = YebanProjectV1 {
        master_bus_track_id: master,
        tracks,
        routing_graph: routing,
        clip_pool,
        ..YebanProjectV1::default()
    };
    project.transport.metronome_enabled = metronome;
    project
}

/// PDC 夹具的实体身份。
///
/// 参考渲染与被测渲染必须共用同一组身份（`SynthEngine::begin_snapshot` 按键序分配声部槽）
/// —— 与 `tests/pdc_mix_path.rs` 同款纪律。
#[derive(Clone, Copy)]
struct Ids {
    master: EntityId,
    fast: EntityId,
    slow: EntityId,
}

fn ids() -> Ids {
    Ids {
        master: EntityId::new(),
        fast: EntityId::new(),
        slow: EntityId::new(),
    }
}

/// "两条并联支路各自直连 master"的工程：`slow` 上报 `slow_latency` 帧自身延迟。
///
/// 图：`fast → master`、`slow(自身 slow_latency) → master` ⇒ `L_max = slow_latency`、
/// `D(fast) = slow_latency`、`D(slow) = 0`。只有 `fast` 需要补偿。
fn pdc_project(
    ids: Ids,
    slow_latency: u32,
    fast_start: Option<u64>,
    slow_start: Option<u64>,
) -> YebanProjectV1 {
    let fast_notes: Vec<(u64, u8, u8)> = fast_start
        .map(|start| vec![(start, 60, VELOCITY)])
        .unwrap_or_default();
    // ⚠ 两条支路的音高与力度必须**逐位同源**（`single + single` 才是逐位等式）。
    // `tests/pdc_mix_path.rs` 的夹具两条支路都写死音高 60、力度 `VELOCITY`。
    let slow_notes: Vec<(u64, u8, u8)> = slow_start
        .map(|start| vec![(start, 60, VELOCITY)])
        .unwrap_or_default();
    let (fast, fast_clip) = note_track(ids.fast, "Fast", &fast_notes, NOTE_TICKS);
    let (mut slow, slow_clip) = note_track(ids.slow, "Slow", &slow_notes, NOTE_TICKS);
    slow.devices = vec![DeviceDefinition {
        id: EntityId::new(),
        name: "Reported".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: Vec::new(),
        latency_samples: slow_latency,
    }];

    let mut tracks = BTreeMap::new();
    tracks.insert(ids.fast, fast);
    tracks.insert(ids.slow, slow);
    tracks.insert(
        ids.master,
        TrackV3 {
            id: ids.master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    let mut clip_pool = BTreeMap::new();
    if let Some(clip) = fast_clip {
        clip_pool.insert(clip.id, clip);
    }
    if let Some(clip) = slow_clip {
        clip_pool.insert(clip.id, clip);
    }
    let mut routing = RoutingGraph {
        nodes: vec![ids.fast, ids.slow, ids.master],
        ..RoutingGraph::default()
    };
    for source in [ids.fast, ids.slow] {
        let id = EntityId::new();
        routing.edges.insert(
            id,
            RoutingEdge {
                id,
                source_node: source,
                destination_node: ids.master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
    }
    YebanProjectV1 {
        master_bus_track_id: ids.master,
        tracks,
        routing_graph: routing,
        clip_pool,
        ..YebanProjectV1::default()
    }
}

/// 按**固定的设备缓冲长度** `chunk` 帧渲染 `total` 帧。
///
/// 每条回调调用 `process_quantum` 一次，缓冲长度就是 `min(chunk, 剩余)` × 2 声道
/// —— 与 cpal 把设备缓冲整段交给引擎的形状相同。电平队列容量按"每个量子两个节点"
/// 上限给足（块长 1 时量子数最多 `total`）。
fn render_chunked(project: &YebanProjectV1, chunk: usize, total: usize) -> Rendered {
    assert!(chunk > 0 && total > 0);
    let snapshot = EngineSnapshot::from_project(project, 1).expect("夹具工程必须合法");
    let slot = SnapshotSlot::new(snapshot);
    let (retire, _queue) = retire_channel(16);
    let (_sender, receiver) = event_channel(64);
    let (publisher, mut collector) = meter_channel(2 * total + 32);
    let mut runtime = EngineRuntime::new(&slot, retire, receiver, publisher);

    let mut left = Vec::with_capacity(total);
    let mut right = Vec::with_capacity(total);
    let mut out = vec![0.0f32; chunk * 2];
    let mut done = 0usize;
    while done < total {
        let frames = chunk.min(total - done);
        let slice = &mut out[..frames * 2];
        slice.fill(0.0);
        runtime.process_quantum(slice, 2);
        for frame in 0..frames {
            left.push(slice[frame * 2]);
            right.push(slice[frame * 2 + 1]);
        }
        done += frames;
    }

    let stats = runtime.stats();
    let last_block_frames = runtime.last_block().frames();
    let mut frames = vec![MeterFrame::default(); 2 * total + 32];
    let drained = collector.tick(&mut frames);
    frames.truncate(drained);
    Rendered {
        left,
        right,
        stats,
        meters: frames,
        last_block_frames,
    }
}

/// 逐位比较两条声道，返回第一处差异的 `(帧, 左位, 右位)`。
fn first_bit_difference(a: &[f32], b: &[f32]) -> Option<(usize, u32, u32)> {
    assert_eq!(a.len(), b.len(), "两条渲染必须等长");
    a.iter()
        .zip(b.iter())
        .enumerate()
        .find(|(_, (x, y))| x.to_bits() != y.to_bits())
        .map(|(index, (x, y))| (index, x.to_bits(), y.to_bits()))
}

/// 两个电平队列逐帧逐字段的逐位比较，返回第一处差异的描述。
fn first_meter_difference(a: &[MeterFrame], b: &[MeterFrame]) -> Option<String> {
    if a.len() != b.len() {
        return Some(format!("队列长度 {} vs {}", a.len(), b.len()));
    }
    for (index, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        for (field, xs, ys) in [
            ("peak", x.peak.to_bits(), y.peak.to_bits()),
            ("peak_hold", x.peak_hold.to_bits(), y.peak_hold.to_bits()),
            ("rms", x.rms.to_bits(), y.rms.to_bits()),
            (
                "rms_smoothed",
                x.rms_smoothed.to_bits(),
                y.rms_smoothed.to_bits(),
            ),
        ] {
            if xs != ys {
                return Some(format!("第 {index} 帧 {field}: {xs:#010x} vs {ys:#010x}"));
            }
        }
        if x.node != y.node || x.quantum != y.quantum {
            return Some(format!(
                "第 {index} 帧的节点/量子序号不同: {:?}@{} vs {:?}@{}",
                x.node, x.quantum, y.node, y.quantum
            ));
        }
    }
    None
}

fn nonzero(samples: &[f32]) -> usize {
    samples.iter().filter(|sample| **sample != 0.0).count()
}

/// ⑦-1 的一个夹具：工程 + **覆盖度自检**（判据必须真的走到那条调用点）。
struct Case {
    label: &'static str,
    project: YebanProjectV1,
    /// 自检项的可读名字（失败信息里点名）。
    witness_name: &'static str,
    witness: fn(&EngineStats) -> bool,
}

/// ⑦-1：**设备缓冲长度不改变渲染输出**（六个夹具 × 九种块长，逐位）。
///
/// 六个夹具各自见证一条块长敏感的调用点（见模块文档 §3）：裸合成器（参数平滑 + 声部池
/// + 声相 + 限制器）、通道条 + 压缩器、Freeverb、鼓机、节拍器、PDC 两条并联支路。
///
/// ⚠ 每个夹具都有一条**覆盖度自检**（`witness`）：夹具必须真的走到那条调用点，
/// 否则"逐位相同"可能是"两边都没跑"的空转假绿。
#[test]
fn device_buffer_length_never_changes_the_rendered_audio() {
    let notes = [
        (0u64, 60u8, 127u8),
        (480, 64, 110),
        (960, 67, 100),
        (1920, 72, 90),
    ];
    let drum_notes = [
        (0u64, 36u8, 120u8),
        (480, 38, 120),
        (960, 42, 120),
        (1200, 46, 120),
        (1440, 39, 120),
    ];
    let drum_kit = [
        ("kick_note", 36.0f32),
        ("snare_note", 38.0),
        ("closed_hat_note", 42.0),
        ("open_hat_note", 46.0),
        ("clap_note", 39.0),
        ("master_level", 0.8),
    ];

    let ids = ids();
    let cases = [
        Case {
            label: "裸合成器（参数平滑 + 声部池 + 声相 + 母线限制器）",
            project: chain_project(Vec::new(), &notes, NOTE_TICKS, false),
            witness_name: "rendered_samples > 0",
            witness: |stats| stats.rendered_samples > 0,
        },
        Case {
            label: "通道条 + 压缩器",
            project: chain_project(
                vec![effect(
                    "Strip",
                    &[
                        ("input_gain_db", -3.0),
                        ("eq_low_gain", 8.0),
                        ("cutoff_hz", 900.0),
                        ("resonance", 0.3),
                        ("threshold_db", -30.0),
                        ("ratio", 8.0),
                    ],
                )],
                &notes,
                NOTE_TICKS,
                false,
            ),
            witness_name: "insert_strip_frames > 0",
            witness: |stats| stats.insert_strip_frames > 0,
        },
        Case {
            label: "Freeverb 混响",
            project: chain_project(
                vec![effect(
                    "Reverb",
                    &[
                        ("reverb_size", 1.0),
                        ("reverb_wet", 1.0),
                        ("reverb_predelay", 0.02),
                    ],
                )],
                &notes,
                NOTE_TICKS,
                false,
            ),
            witness_name: "insert_reverb_frames > 0",
            witness: |stats| stats.insert_reverb_frames > 0,
        },
        Case {
            label: "鼓机",
            project: chain_project(vec![instrument(&drum_kit)], &drum_notes, NOTE_TICKS, false),
            witness_name: "drum_hits > 0",
            witness: |stats| stats.drum_hits > 0,
        },
        Case {
            label: "节拍器",
            project: chain_project(Vec::new(), &notes, NOTE_TICKS, true),
            witness_name: "metronome_clicks > 0",
            witness: |stats| stats.metronome_clicks > 0,
        },
        Case {
            label: "PDC 两条并联支路（D(fast) = 400）",
            project: pdc_project(ids, SLOW_LATENCY, Some(0), None),
            witness_name: "pdc_processed_blocks > 0",
            witness: |stats| stats.pdc_processed_blocks > 0,
        },
    ];

    for Case {
        label,
        project,
        witness_name,
        witness,
    } in cases
    {
        let baseline = render_chunked(&project, DEFAULT_BLOCK_FRAMES, TOTAL_FRAMES);
        assert!(
            nonzero(&baseline.left) > 0,
            "{label}：128 帧块长的基线必须是**有声**的，否则下面的逐位比较是空转假绿"
        );
        assert!(
            witness(&baseline.stats),
            "{label}：覆盖度自检失败 —— 夹具没有走到 `{witness_name}`，\
             本夹具对该调用点没有判别力"
        );
        for chunk in CHUNKS {
            let other = render_chunked(&project, chunk, TOTAL_FRAMES);
            let left = first_bit_difference(&baseline.left, &other.left);
            let right = first_bit_difference(&baseline.right, &other.right);
            assert!(
                left.is_none() && right.is_none(),
                "{label}：设备缓冲长度 {chunk} 帧必须与 {DEFAULT_BLOCK_FRAMES} 帧**逐位**相同；\
                 实测第一处差异（帧, 位模式）= 左 {left:?} / 右 {right:?}"
            );
        }
        println!(
            "[engine-blk/7-1] {label}: {} 种块长（含 1 帧与一次回调整段）对 128 帧基线逐位相同；\
             非零样本 {} / {}",
            CHUNKS.len(),
            nonzero(&baseline.left),
            TOTAL_FRAMES
        );
    }
    let _ = ids;
}

/// ⑦-2：PDC 补偿在**任意**设备缓冲长度下逐位精确。
///
/// 与 `tests/pdc_mix_path.rs` 的 P1 是**同一条**断言，唯一区别是块长：P1 只跑 128 帧块，
/// 这里跑 129 / 127 / 100 / 64 / 1（以及 P1 的 128 作对照）。
///
/// `aligned[t] == reference[t − 400]` 必须**逐位**成立，且前 400 帧必须**逐位 `+0.0`**
/// （延迟线的初始环里是零）。参考与被测**用同一个块长**渲染 ⇒ 其它逐块器件的行为差异
/// 在两侧同时出现，剩下的只能是 PDC 自己的块长依赖。
#[test]
fn pdc_alignment_is_sample_exact_at_every_device_buffer_length() {
    let ids = ids();
    let delay = SLOW_LATENCY as usize;
    // P1 的两个前提：计划本身与块长无关，零延迟工程的 L_max 为 0。
    let reference_project = pdc_project(ids, 0, Some(0), None);
    let pdc_project = pdc_project(ids, SLOW_LATENCY, Some(0), None);
    let reference_snapshot = EngineSnapshot::from_project(&reference_project, 1).expect("参考工程");
    let pdc_snapshot = EngineSnapshot::from_project(&pdc_project, 1).expect("被测工程");
    assert_eq!(reference_snapshot.pdc().total_latency(), 0);
    assert_eq!(pdc_snapshot.pdc().total_latency(), SLOW_LATENCY);
    assert_eq!(
        pdc_snapshot.pdc().compensation(&ids.fast),
        Some(SLOW_LATENCY)
    );
    assert_eq!(pdc_snapshot.pdc().compensation(&ids.slow), Some(0));

    for chunk in [DEFAULT_BLOCK_FRAMES, 129, 127, 100, 64, 1] {
        let reference = render_chunked(&reference_project, chunk, TOTAL_FRAMES);
        let aligned = render_chunked(&pdc_project, chunk, TOTAL_FRAMES);
        assert!(
            nonzero(&reference.left) > 0,
            "块长 {chunk}：参考渲染必须真的出声，否则逐位相等是空转假绿"
        );
        assert!(
            reference.left[..delay].iter().any(|sample| *sample != 0.0),
            "块长 {chunk}：参考渲染在前 {delay} 帧里必须有信号，否则移位观察不到"
        );

        for (channel, reference_channel, aligned_channel) in [
            ("左", &reference.left, &aligned.left),
            ("右", &reference.right, &aligned.right),
        ] {
            for (index, sample) in aligned_channel[..delay].iter().enumerate() {
                assert_eq!(
                    sample.to_bits(),
                    0.0f32.to_bits(),
                    "块长 {chunk} · {channel}声道 第 {index} 帧：PDC 延迟线前 {delay} 帧必须是\
                     **逐位 `+0.0`**（延迟线的历史是零），实测 {sample:e}"
                );
            }
            let mut mismatches: Vec<String> = Vec::new();
            let mut matched_nonzero = 0usize;
            for index in delay..aligned_channel.len() {
                let expected = reference_channel[index - delay];
                if aligned_channel[index].to_bits() == expected.to_bits() {
                    if expected != 0.0 {
                        matched_nonzero += 1;
                    }
                } else if mismatches.len() < 3 {
                    mismatches.push(format!(
                        "第 {index} 帧: 实测 {:#010x} ({:e}) vs 期望 {:#010x} ({:e})",
                        aligned_channel[index].to_bits(),
                        aligned_channel[index],
                        expected.to_bits(),
                        expected
                    ));
                }
            }
            assert!(
                mismatches.is_empty(),
                "块长 {chunk} · {channel}声道：PDC 后移 {delay} 帧必须逐位精确，\
                 实测差异（最多 3 处）：{mismatches:?}"
            );
            assert!(
                matched_nonzero > 1_000,
                "块长 {chunk} · {channel}声道：逐位相同的**非零**样本只有 {matched_nonzero} 个，\
                 判据疑似空转"
            );
        }
        println!(
            "[engine-blk/7-2] 块长 {chunk:4}: 逐位后移 {delay} 帧成立；\
             前 {delay} 帧逐位 +0.0；参考非零 {} / {}",
            nonzero(&reference.left),
            TOTAL_FRAMES
        );
    }
}

/// ⑦-2b：并联两条支路在**任意**设备缓冲长度下仍然采样级同相。
///
/// 这是 `tests/pdc_mix_path.rs` 的 P2 的块长推广：`fast` 自身 0 ⇒ `D = 400`；
/// `slow` 自身 400 ⇒ `D = 0`，其"设备链延迟"由夹具把乐器触发时刻后移 400 帧模拟。
/// 补偿正确时两条支路在**同一采样点**到达求和节点 ⇒
/// `aligned[t] == single[t − 400] + single[t − 400]`（逐位，两条贡献逐位相同）。
///
/// 参考与被测**用同一个块长**渲染 ⇒ 逐块器件的块长差异在两侧同时出现；
/// 剩下能变红的只有 PDC 的对齐本身。对照组（零延迟计划、同样的摆放）在前 400 帧里
/// **不是**静音 ⇒ 对齐不是音符摆放自动带来的。
#[test]
fn pdc_diamond_stays_in_phase_at_every_device_buffer_length() {
    let delay = SLOW_LATENCY as usize;
    assert_eq!(
        SLOW_START_TICK,
        SLOW_LATENCY as u64 / SAMPLES_PER_TICK,
        "慢支路的触发 tick 必须正好把 400 帧落在夹具栅格上"
    );
    for chunk in [DEFAULT_BLOCK_FRAMES, 129, 127, 100, 64, 1] {
        let ids = ids();
        let single = render_chunked(&pdc_project(ids, 0, Some(0), None), chunk, TOTAL_FRAMES);
        let aligned = render_chunked(
            &pdc_project(ids, SLOW_LATENCY, Some(0), Some(SLOW_START_TICK)),
            chunk,
            TOTAL_FRAMES,
        );
        let uncompensated = render_chunked(
            &pdc_project(ids, 0, Some(0), Some(SLOW_START_TICK)),
            chunk,
            TOTAL_FRAMES,
        );

        assert!(
            nonzero(&single.left) > 0,
            "块长 {chunk}：单支路参考必须真的出声"
        );
        let peak = single.left.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(
            peak * 2.0 < yeban_engine::mixer::LIMITER_THRESHOLD,
            "块长 {chunk}：两支路叠加后的峰值 {} 必须仍在母线限制器阈值 {} 之下，\
             否则下面的 `2 ×` 断言经过的是非线性软膝、不再是逐位等式",
            peak * 2.0,
            yeban_engine::mixer::LIMITER_THRESHOLD
        );
        assert!(
            uncompensated.left[..delay]
                .iter()
                .any(|sample| *sample != 0.0),
            "块长 {chunk}：零延迟计划下两支路必须错开 {delay} 帧（对照组前 {delay} 帧非静音），\
             否则本判据测的不是 PDC"
        );

        for (index, sample) in aligned.left[..delay].iter().enumerate() {
            assert_eq!(
                sample.to_bits(),
                0.0f32.to_bits(),
                "块长 {chunk} 第 {index} 帧：两条支路都还没到达求和节点，必须是逐位 `+0.0`"
            );
        }
        let mut mismatches: Vec<String> = Vec::new();
        let mut matched = 0usize;
        for index in delay..aligned.left.len() {
            let contribution = single.left[index - delay];
            let expected = contribution + contribution;
            if aligned.left[index].to_bits() == expected.to_bits() {
                if expected != 0.0 {
                    matched += 1;
                }
            } else if mismatches.len() < 3 {
                mismatches.push(format!(
                    "t={index}: 实测 {:#010x} ({:e}) vs 期望 {:#010x} ({:e})",
                    aligned.left[index].to_bits(),
                    aligned.left[index],
                    expected.to_bits(),
                    expected
                ));
            }
        }
        assert!(
            mismatches.is_empty(),
            "块长 {chunk}：并联支路必须在同一条采样线上到达求和节点；前 3 处差异：{mismatches:?}"
        );
        assert!(
            matched > 1_000,
            "块长 {chunk}：逐位相同的**非零**样本只有 {matched} 个，判据疑似空转"
        );
        println!("[engine-blk/7-2b] 块长 {chunk:4}: 两支路在第 {delay} 帧对齐；非零匹配={matched}");
    }
}

/// ⑦-5：**卷积插入级在"块长是 128 的整数倍"时与 128 帧基线逐位相同**；
/// 不是整数倍时的分歧作为 §4 发现 1 的**可复现读数**打印（⛔ 不把缺陷写成期望值）。
///
/// 为什么只断言整数倍的一半：器件（`crates/yeban-dsp/src/convolution_reverb.rs` 的 §4）
/// 把子块网格锚在**每次调用**的起点，因此"短块"这件事在器件内部按零补齐推进历史
/// ⇒ 非整数倍块长下输出确实不同（发现 1）。把那条组合写成"必须逐位相同"等于提交一个
/// 红判据；写成"必须不同"等于把缺陷写成期望值。本仓库的既有先例（`engine-22` 的单声道
/// 映射、`engine-21` 的 `SeekTicks` 口径）都是在模块文档里登记读数、在判据里钉住
/// **成立的那一半**。
#[test]
fn convolution_insert_is_block_invariant_for_whole_quantum_buffer_lengths() {
    let notes = [
        (0u64, 60u8, 127u8),
        (480, 64, 110),
        (960, 67, 100),
        (1920, 72, 90),
    ];
    let project = chain_project(
        vec![effect(
            "Convolution",
            &[
                ("conv_dry", 0.0),
                ("conv_wet", 1.0),
                ("conv_ir_decay_s", 0.35),
                ("conv_ir_seed", 7.0),
            ],
        )],
        &notes,
        NOTE_TICKS,
        false,
    );
    let baseline = render_chunked(&project, DEFAULT_BLOCK_FRAMES, TOTAL_FRAMES);
    assert!(
        nonzero(&baseline.left) > 0,
        "128 帧块长的基线必须真的出声（全湿卷积会替换掉干声，静音说明夹具没武装）"
    );
    assert!(
        baseline.stats.insert_convolution_frames > 0,
        "覆盖度自检：卷积级必须真的处理过帧（否则本判据测的不是卷积）"
    );

    for chunk in [
        2 * DEFAULT_BLOCK_FRAMES,
        3 * DEFAULT_BLOCK_FRAMES,
        4 * DEFAULT_BLOCK_FRAMES,
        TOTAL_FRAMES,
    ] {
        let other = render_chunked(&project, chunk, TOTAL_FRAMES);
        let difference = first_bit_difference(&baseline.left, &other.left);
        assert!(
            difference.is_none(),
            "块长 {chunk} 帧是 128 的整数倍 ⇒ 子块仍是 128 帧、网格不漂移，\
             必须与 128 帧基线逐位相同；实测第一处差异（帧, 位模式）= {difference:?}"
        );
    }

    // ---- §4 发现 1 的复现读数（⛔ 不判据）----
    for chunk in [129usize, 127, 100, 64, 1] {
        let other = render_chunked(&project, chunk, TOTAL_FRAMES);
        let differing = baseline
            .left
            .iter()
            .zip(other.left.iter())
            .filter(|(a, b)| a.to_bits() != b.to_bits())
            .count();
        println!(
            "[engine-blk/7-5-finding1] 块长 {chunk:4}: 首个差异（帧, 基线位, 该块长位）= {:?}；\
             差异样本 {differing} / {TOTAL_FRAMES}",
            first_bit_difference(&baseline.left, &other.left)
        );
    }
}

/// ⑦-3：一次回调内部按 128 帧切成整量子，**短于 128 帧的尾部就是一块**。
///
/// 这条判据钉住的正是 §4 发现 1/2 的**成因**：引擎的处理块长等于"调用内部按 128 切分
/// 之后的最后一块"，因此设备缓冲长度不是 128 的整数倍时，**每一次**回调都会产生一个
/// 短块。⚠ 若 §4 的裁决改成"跨回调捎带余量"，本判据的期望值必须同步改写（这是刻意的：
/// 语义变更必须留下痕迹）。
#[test]
fn a_call_is_split_into_whole_quanta_and_a_short_tail() {
    let project = chain_project(Vec::new(), &[(0, 60, 127)], NOTE_TICKS, false);

    // 一次回调 256 帧 = 2 个整量子。
    let doubled = render_chunked(&project, 2 * DEFAULT_BLOCK_FRAMES, 4 * DEFAULT_BLOCK_FRAMES);
    assert_eq!(
        doubled.stats.quanta, 4,
        "512 帧、每次回调 256 帧 ⇒ 4 个整量子"
    );
    assert_eq!(doubled.last_block_frames, DEFAULT_BLOCK_FRAMES);

    // 一次回调 129 帧 = 128 + 1 ⇒ 两个量子，最后一块 1 帧。
    let odd = render_chunked(
        &project,
        DEFAULT_BLOCK_FRAMES + 1,
        2 * (DEFAULT_BLOCK_FRAMES + 1),
    );
    assert_eq!(odd.stats.quanta, 4, "258 帧、每次回调 129 帧 ⇒ 4 个量子");
    assert_eq!(
        odd.last_block_frames, 1,
        "129 = 128 + 1 ⇒ 尾部块是 1 帧，不是整量子"
    );

    // 一次回调 200 帧 = 128 + 72（`src/rt.rs` 的同名单元判据同款）。
    let partial = render_chunked(&project, 200, 400);
    assert_eq!(
        partial.stats.quanta, 4,
        "400 帧、每次回调 200 帧 ⇒ 4 个量子"
    );
    assert_eq!(partial.last_block_frames, 72, "200 = 128 + 72");

    // 一次回调 100 帧 ⇒ **每一次**回调都只有一块，且短于 128。
    let short = render_chunked(&project, 100, 400);
    assert_eq!(short.stats.quanta, 4, "400 帧、每次回调 100 帧 ⇒ 4 个量子");
    assert_eq!(
        short.last_block_frames, 100,
        "设备缓冲长度 100 帧 ⇒ 处理块长恒为 100（不是契约的 128）—— 这正是发现 1/2 的成因"
    );
    println!(
        "[engine-blk/7-3] 256帧回调 quanta={} last={}；129 quanta={} last={}；\
         200 quanta={} last={}；100 quanta={} last={}",
        doubled.stats.quanta,
        doubled.last_block_frames,
        odd.stats.quanta,
        odd.last_block_frames,
        partial.stats.quanta,
        partial.last_block_frames,
        short.stats.quanta,
        short.last_block_frames
    );
}

/// ⑦-4：**电平读数**在"设备缓冲长度是 128 的整数倍"时逐位相同。
///
/// 为什么需要单独一条：⑦-1 只比 PCM，而电平队列是**另一条 SPSC 输出**，它的内容由
/// "每秒 `analyze` 调用次数"决定（模块文档 §3 第 7 行）。块长 256 / 384 / 一次回调整段
/// 都分解成 128 帧的整量子 ⇒ 调用次数与块长 128 完全相同 ⇒ 读数必须**逐位**相同。
///
/// 观测方式：一个 1 tick 长的音符后接静音，渲染 1.0 s（48 000 帧 = 375 个整量子），
/// 把整个电平队列逐帧逐字段逐位比对。读数里含**峰值保持的释放**（弹道）⇒ 这条判据同时
/// 钉住"整数倍块长下弹道按 20 dB/s 释放"（读数直接对比，不折算 dB，因此不引入超越函数）。
///
/// ⛔ 块长 100 / 64 / 1 **不在**本判据里：实测它们的读数与 128 不同（§4 发现 2），
/// 把已知会红的组合写成"必须逐位相同"等于提交一个红判据。
#[test]
fn meter_readings_are_bit_identical_for_whole_quantum_buffer_lengths() {
    const ONE_SECOND: usize = 48_000;
    // 1 tick = 25 帧的短音符：起音之后整段是静音 ⇒ 只能在峰值保持的释放上观察。
    let project = chain_project(Vec::new(), &[(0, 60, 127)], NOTE_TICKS, false);
    let baseline = render_chunked(&project, DEFAULT_BLOCK_FRAMES, ONE_SECOND);
    assert!(
        !baseline.meters.is_empty(),
        "电平队列必须是空的以外的东西，否则本判据是空转"
    );
    let decaying = baseline
        .meters
        .iter()
        .any(|frame| frame.peak_hold > 0.0 && frame.peak == 0.0);
    assert!(
        decaying,
        "窗口里必须出现「峰值已落、峰值保持还在」的帧，否则弹道没有被覆盖"
    );

    for chunk in [
        2 * DEFAULT_BLOCK_FRAMES,
        3 * DEFAULT_BLOCK_FRAMES,
        ONE_SECOND,
    ] {
        let other = render_chunked(&project, chunk, ONE_SECOND);
        assert_eq!(
            first_meter_difference(&baseline.meters, &other.meters),
            None,
            "设备缓冲长度 {chunk} 帧（128 的整数倍）的电平读数必须与 128 帧逐位相同"
        );
    }
    println!(
        "[engine-blk/7-4] 128/256/384/一次回调整段四种块长的电平读数逐位相同；帧数 {}；\
         末帧 peak_hold = {:e}",
        baseline.meters.len(),
        baseline
            .meters
            .iter()
            .rev()
            .find(|frame| frame.peak_hold > 0.0)
            .map_or(0.0, |frame| frame.peak_hold)
    );

    // ---- §4 发现 2 的**复现读数**（⛔ 不判据：把已知会红的组合写成期望值等于提交一个红判据）----
    //
    // 量法（单位：dB / 1.0 s；契约值 20 dB/s）：同一个 1 秒窗口里，取该轨电平队列的
    // **第一帧**与**最后一帧**的 `peak_hold`，折算 `20·log10(首/末)`。块长是 128 的整数倍
    // 时等于 128 帧的读数（上面已逐位证明）；不是整数倍时，实际 `analyze` 次数按
    // `128 ÷ 块长` 倍上升 ⇒ 释放同倍数加快。
    //
    // ⚠ 读数用**另一个**夹具：1 tick（= 25 帧）的短音符接整段静音。用上面那个
    // 960 tick 的长音符量不到释放速率（音符在整段窗口里都在发声，峰值保持被持续顶住）。
    let finding_project = chain_project(Vec::new(), &[(0, 60, 127)], 1, false);
    for chunk in [DEFAULT_BLOCK_FRAMES, 100, 64] {
        let rendered = render_chunked(&finding_project, chunk, ONE_SECOND);
        // 只看**一个节点**（第一条普通轨 —— 母线的帧也在同一个队列里，混在一起会把
        // "首帧/末帧"的位置换成另一个节点，读数就不再是释放速率）。
        let node = rendered.meters.first().map(|frame| frame.node);
        let track_frames: Vec<&MeterFrame> = rendered
            .meters
            .iter()
            .filter(|frame| Some(frame.node) == node && frame.peak_hold > 0.0)
            .collect();
        if let (Some(first), Some(last)) = (track_frames.first(), track_frames.last()) {
            println!(
                "[engine-blk/7-4-finding2] 块长 {chunk:4}: 电平帧 {}，该节点峰值保持首 = {:e}，\
                 末 = {:e} ⇒ 1.0 s 内释放 {:.3} dB（契约 20 dB/s）",
                rendered.meters.len(),
                first.peak_hold,
                last.peak_hold,
                20.0 * (first.peak_hold / last.peak_hold).log10()
            );
        }
    }
}
