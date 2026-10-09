//! 鼓机（4 个经典音色的**合成配方**）[ARCH-RT-001, ARCH-RT-004, ARCH-DSP-001, ARCH-DET-001]。
//!
//! # 0. ⚠ 本模块是什么、**不是**什么
//!
//! 规范原文（`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:110`）写的是
//! **"鼓机电路建模 ➔ `crates/yeban-dsp/src/drums/`（808/909 模拟电路物理建模）"**。
//! **本模块不是那个。** 本模块是**经典鼓机音色的合成配方实现**：4 个音色
//! （底鼓 / 军鼓 / 踩镲 / 拍手），用波形 + 包络 + 噪声 + 带通直接合成。
//! 它**不是**任何硬件的电路模型，也**不是**逐级模拟电路建模。
//!
//! ## 0.1 未实现清单（**本模块最重要的一条**）
//!
//! ⛔ **"drums 做了"不等于"808/909 物理建模完成了"**。下面逐条列出**没有**建模的
//! 东西。"本模块实际做的"一列是实情，不是近似。
//!
//! | # | **没有**建模的东西 | 本模块实际做的 |
//! | :--- | :--- | :--- |
//! | 1 | TR-808 / TR-909 的**逐级模拟电路**（VCO → VCA → VCF → VCA 的拓扑、节点阻抗） | 按配方直接合成波形；没有晶体管级、运放级或节点级模型 |
//! | 2 | **元件容差与器件离散性**（同型号两台机器音色不同） | 固定参数；同一参数永远给同一结果（这是 [ARCH-DET-001] 要求的） |
//! | 3 | **真实包络形状**（808 的包络来自 RC 放电与晶体管非线性，不是理想指数） | 一极点指数包络（复用 [`crate::envelope::Adsr`]，6 个时间常数口径） |
//! | 4 | **非线性器件的具体模型**（`Vbe` 对数、运放压摆率、电容吸收） | 无专用模型；只有 [`crate::filter::LadderFilter`] 的通用有界饱和（仅在带通路径上） |
//! | 5 | 808 底鼓的**脉冲整形器（pulse shaper）／咔哒起音** | 只有每个音色一个线性起振（默认 0.5 ms）；起振不是脉冲波形 |
//! | 6 | 808 底鼓旋钮的**真实标度**（Tuning / Decay / Tone / Level 到电路量的映射） | 参数是本模块的（`tune_hz` / `glide_hz` / `glide_s` / `decay_s` / `level`），不是硬件旋钮标度 |
//! | 7 | 808 军鼓的**两个桥接 T（bridged-T）振荡器**及其耦合与 Q | 两个**固定频率、互不耦合**的正弦相加 |
//! | 8 | 808 军鼓／拍手的**带通噪声电路**（具体 Q、中心频率、两级带通、噪声门） | 四级 `x − LP(拐点)` 级联（高通的近似）＋ 一级 `LP` 低通；谐振固定 0。**没有**专用的带通/状态变量滤波器 |
//! | 9 | 808 踩镲的**金属振荡器**（6 位二进制计数器 + 移位寄存器近似正弦） | 6 条**连续方波**，比值为本模块选定的非谐组 |
//! | 10 | 808 踩镲的**采样保持／真值表**与 808 拍手的**噪声门控时序电路** | 连续方波、连续白噪声；定时只有整数帧计数 |
//! | 11 | **909 的一切**（909 是采样 + 模拟混合，其底鼓/军鼓/踩镲电路与 808 不同） | 没有任何 909 专用配方；本模块只有一套配方 |
//! | 12 | 808 的其它音色：**手鼓 / 桶鼓 / 康加 / 铃鼓 / 牛铃 / 镲（Cymbal）/ 边击 / 雷声 / 沙锤** | 只有 4 个音色 |
//! | 13 | **每音色独立输出与独立声像**（808 每路一个独立输出插孔） | 单声道求和（`render` 是单声道 `&mut [f32]`） |
//! | 14 | **重音（accent）总线**及其对每个音色的非线性影响 | 只有 [`DrumHit::gain`] 的线性增益 |
//! | 15 | 音色参数的**自动化曲线与工程持久化**（`yeban-model` 的设备定义 / `DeviceDefinition`） | 只有 [`DrumKitParams`]（静态参数）；没有自动化、没有持久化 |
//! | 16 | 与**引擎 / 界面 / MCP** 的接线（设备注册、PDC 延迟上报、混音路由） | 无。接线是引擎侧的独立裁决（本票不接） |
//! | 17 | **立体声**、每音色声像、并行压缩（New York Compression）发送 | 无 |
//! | 18 | **采样回放**（909 的 crash/ride/clap 等本来就用采样） | 无采样器；本模块只做合成 |
//! | 19 | **抗混叠**：真实 808 的金属振荡器是**移位寄存器伪随机序列**（谱近似平坦），本模块是 6 条**连续方波**（谱按 `1/n` 衰减） | 方波不做限带 ⇒ 高次谐波**会折返**。后果是本机实测的谱重心只有 **3289 Hz**（真 808 踩镲要亮得多）。这条是"设计选择 + 已知缺陷"，不是"近似 808" |
//! | 20 | **专用的带通／状态变量滤波器**（多模、可调 Q） | 带通由既有 [`crate::filter::LadderFilter`] 组合：`LP(hi) ∘ (x − LP(lo))⁴`。⚠ [`crate::shaping::Biquad`]（`shaping.rs:252`）**是私有类型**，本 crate 的公共面里没有带通 |
//! | 21 | **鼓机的独立效果链**（每音色的压缩/失真/滤波，808 每路都有独立的电平与音色整形） | 无；每个音色只有参数与一次带通 |
//! | 22 | **重触发时的相位重置语义的可配置性**（有些鼓机让同一音色的重触发从相位 0 起，有些从当前相位继续） | 每次触发都从相位 0、包络从 0 起（新槽位或窃取后的挂起一击） |
//!
//! # 1. 规范原文（本模块的依据）
//!
//! ⚠ 先量再写：下面每一行都是 `docs/**` 里与鼓机有关的**全部**命中
//! （测法：`grep -rniE '鼓|drum|kick|snare|hi-?hat|clap|808|909|tom|cymbal' docs/`，
//! 逐条剔除数字巧合 —— `0.008084789`、`0.984808`、`0.996884`、`37245680897`、
//! `4.42f32` 这类浮点/计数巧合，以及示例轨名 `"909 鼓组"`）。
//!
//! | `file:line` | 原文 |
//! | :--- | :--- |
//! | `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:110` | 鼓机电路建模 ➔ `crates/yeban-dsp/src/drums/`（808/909 模拟电路物理建模） |
//! | `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:347` | 参考工程 A …包含 16 个 PolySynth 减法合成器、8 轨 TR-808/909 模拟鼓机… |
//! | `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:445` | 内置 323 款 SFZ 原声乐器引擎 + 纯 Rust 建模合成器 (PolySynth, GS-1, 808/909) |
//! | `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:710` | `yeban-dsp/` …纯数学 DSP 库 (真峰值限制器, SSL 压缩, 通道条, 808/909) |
//!
//! ## ⇒ **规范未定义细节**（这是核实过的结论，不是省略）
//!
//! 上面 4 处里**没有**：音色清单、音色数量、任何参数表、任何频率/时间/比值的数字、
//! 任何电路级要求（哪些级、什么元件）、808 与 909 的差别、输出通道数。
//! ⚠ "808/909 模拟电路物理建模"是**目标陈述**，不是可实现的规格书；
//! 把它当成规格书会得到一个**无法对账**的实现。
//!
//! ## ⇒ 因此本模块的**音色选择与配方结构**是本模块声明的
//!
//! 结构选择的依据是**经典模拟鼓机合成法**（业界公开常识的 4 条结构）：
//!
//! | 音色 | 本模块用的结构 | 为什么这个结构是"经典"的 |
//! | :--- | :--- | :--- |
//! | 底鼓 | 一条**下滑正弦**（音高从 `glide_hz` 指数滑到 `tune_hz`）+ 指数幅度包络 | 模拟底鼓的"音高包络"是 RC 放电驱动的 VCO；正弦是因为下滑段听感就是一条低频正弦 |
//! | 军鼓 | **两个固定频率正弦**相加 + **带通白噪声**，两条包络各自衰减 | 军鼓 = 鼓皮的**音调**（两个共振模式）与响弦的**噪声**两件事；两条包络的衰减时间不同才是军鼓 |
//! | 踩镲 | **6 条非谐方波**相加 → 带通 → 极快衰减 | 方波的奇次谐波密集；6 条非谐比值让谐波互相错开 ⇒ 无音高感的金属噪声 |
//! | 拍手 | **带通白噪声** × 多次短促 onset 的包络，最后一个 onset 走长尾巴 | 拍手 = 多只手的噪声在 10 ms 级上错开；"多次 onset + 尾巴"就是这个错开 |
//!
//! ⛔ **具体数字没有外部出处**：`HAT_RATIOS` 的 6 个比值、各默认频率、衰减时间、
//! onset 间隔都是**本模块选的**（每个默认值旁边的注释写了选它的理由），
//! **不是**从硬件测出来的，也**没有**引用任何外部文献的数字。
//! 可对账的是**读数与结构的自洽**（`tests/drums_render.rs`），不是"像 808"。
//!
//! ### 1.1 两处"参数名不等于物理量"的地方（明说，免得误读）
//!
//! 1. **带通不是带通**：本 crate 的公共面里没有带通滤波器
//!    （[`crate::shaping::Biquad`] 是私有类型）。本模块的带通是
//!    `LP(lowpass_hz) ∘ (x − LP(highpass_hz))⁴` —— 两个既有
//!    [`crate::filter::LadderFilter`] 的组合。⚠ `x − LP(fc)` 是"四极低通的
//!    **补**"，它的阻带斜率是 **6 dB/oct**（不是 24），所以级联 4 次才近似
//!    24 dB/oct。⇒ **`highpass_hz` 是那一级的低通拐点，不是带通下沿**：
//!    实测拐点 6 kHz 时 452 Hz 才被压到 0.0107、2 kHz 已到 0.99
//!    ⇒ 有效通带约 **2–9 kHz**。完整实测表在 `drums/voice.rs` 的
//!    `HIGHPASS_STAGES` 与 `Band` 文档里（含第一版 `LP(hi) − LP(lo)` 被否掉的
//!    读数：452 Hz 处只有 −14 dB，导致踩镲谱重心掉到 1585.7 Hz）。
//! 2. **`level` 不是 RMS 之比**：军鼓的 `noise_level` 与 `tone_level` 是两个
//!    分量的电平旋钮，噪声还要过带通 ⇒ 电平之比 ≠ RMS 之比。默认参数下实测
//!    音调 RMS 0.02556 / 噪声 RMS 0.04400（**+4.72 dB**，判据 S3 钉住）。
//!
//! ## ⇒ 外部检索的结果（如实登记）
//!
//! 本机检索到了二手资料（硬件手册、复刻 wiki），但：
//!
//! - `https://midibox.org/dokuwiki/doku.php?id=mb-808re`（HTTP 200，2026-10-08 读）
//!   正文里**只有原理图图片**（BD/SD、Toms/Congas、Global），**没有**文字级的
//!   电路描述；该页脚注声明 **CC BY-NC-SA 4.0** ⇒ 本模块**未复制它的任何内容**；
//! - 检索命中的其余条目是 PDF 手册（`Tiptop_Audio_HATS808_ns.pdf` 等）与德语手册，
//!   本机工具无法取回其正文（`application/pdf` 不被支持）⇒ **未读**，不引用。
//!
//! ⇒ 本模块只依赖"结构"这条公开常识 + 本仓库自己的判据，不依赖任何外部数字。
//!
//! # 2. 器件形状（与既有器件对齐）
//!
//! | 器件 | 参数 | 构造/设置 | 触发 | 处理 | 读取器 |
//! | :--- | :--- | :--- | :--- | :--- | :--- |
//! | [`DrumMachine`] | [`DrumKitParams`] | [`DrumMachine::new`] / [`DrumMachine::set_params`] / [`DrumMachine::reset`] / [`DrumMachine::set_sample_rate`] | [`DrumMachine::trigger`] | [`DrumMachine::render`] | `slots` / `active_slots` / `triggers` / `voice_steals` / `hat_chokes` / `steal_fade_frames` / `params` / [`DrumMachine::latency_samples`] |
//!
//! 与 `compressor.rs` / `channel_strip.rs` / `limiter.rs` / `polysynth.rs` 同形。
//! 延迟上报见 §7。
//!
//! ## 2.1 为什么是 `drums/mod.rs`（而不是 `drums.rs`）
//!
//! 规范 `:110` 给的路径**以 `/` 结尾**（`crates/yeban-dsp/src/drums/`）⇒ 目录形态
//! 是对原文的忠实读法。本模块用 `drums/mod.rs` + `drums/voice.rs`：
//! `mod.rs` 是**器件与调度**（槽位、窃取、淡出），`voice.rs` 是**信号发生**
//! （波形、包络、带通、burst 定时）。两者分开的理由是它们变红的判据不同
//! （调度侧是窃取/确定性，发生侧是读数/数值安全）。
//!
//! # 3. 实时纪律（[ARCH-RT-001] / `MUST-GATE-001`，最高约束）
//!
//! | 禁令 | 本模块的做法 |
//! | :--- | :--- |
//! | 堆分配 | 槽位池是 `[Slot; SLOTS]`（定长数组）；没有 `Vec`/`Box`/`String` |
//! | 堆释放 | `Slot` 是 `Copy` 值类型；淡出完成时是**赋值**成 [`Slot::IDLE`]，不是 `drop` |
//! | 锁 | 器件自身无锁，也不调用会加锁的东西 |
//! | 阻塞 I/O | 器件没有 I/O |
//! | 日志 | `trigger` / `render` / `set_params` 里没有 `println!`/`eprintln!`/`format!` |
//!
//! 运行期由 `crates/yeban-dsp/tests/drums_rt_zero_alloc.rs`（计数型全局分配器）
//! 钉住：10 000 个量子 × 每个量子 128 帧的窗口内 `allocations == 0`
//! 且 `deallocations == 0`。
//!
//! # 4. 窃取与淡出口径：**沿用 3 ms 指数**（[ARCH-RT-004]），**不**用 5 ms 升余弦
//!
//! 树里有**两套**互相矛盾的原文：
//!
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:383`（[ARCH-RT-004]）：
//!   "窃取瞬间对被终止声部强制应用 **3ms 快速指数衰减**微淡出包络"；
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:426`（[ARCH-DSP-001]）：
//!   "分配器对被偷取声部强制施加 **5.0ms 升余弦窗**…或五次多项式平滑窗快速淡出"。
//!
//! 本模块选 **3 ms 指数**，三条理由（都是可核实的）：
//!
//! 1. [ARCH-RT-004] 是**专门讲声部窃取**的那一条，[ARCH-DSP-001] 那一句是
//!    去爆音一族的列举（同一段里还有参数平滑与 A/B 交叉淡化）；
//! 2. **常量已经在树里且是唯一事实源**：[`crate::envelope::STEAL_RELEASE_SECONDS`]
//!    （`envelope.rs:31`）；`Adsr::start_steal_fade`（`envelope.rs:188`）就是
//!    "先覆盖 release 再 gate_off"的唯一入口，本模块直接调它；
//! 3. **与刚落地的姊妹器件一致**：`polysynth.rs`（`4f6e2eb`）用的就是 3 ms，
//!    并把它写成登记项（`polysynth.rs:80-87`）。
//!
//! ⇒ 5 ms 升余弦窗**未做**，登记在未实现清单之外的本节（`loop_window.rs` 的
//! 升余弦窗是 **64 点循环点微平滑窗**，不是 5 ms 淡出：5 ms @48 kHz = 240 帧，
//! 长度不同 ⇒ 不能复用 ⇒ 本模块不会偷偷拿它冒充）。
//! ⚠ **既有判据一个字都没改**：`crates/yeban-engine/tests/steal_fade.rs` 的
//! S4（`144 帧 = 3 ms @48 kHz`）仍指 `polysynth`/引擎，本模块只加新文件。
//!
//! ## 4.1 鼓机特有的一处：**闭镲 choke 开镲**
//!
//! 触发闭镲时，所有在响的开镲槽位立刻进入同一条 3 ms 淡出
//! （计数器 [`DrumMachine::hat_chokes`]）。理由是**音乐性**：没有它，开镲的
//! 长尾巴（默认 0.32 s）会盖住后面的每一记闭镲，鼓组不可用。
//! ⚠ 这是本模块**声明**的行为，规范没有要求；它复用同一条 [ARCH-RT-004] 淡出。
//! 其它 choke 组（吊镲 choke 等）**未做**。
//!
//! # 5. 确定性（[ARCH-DET-001]）
//!
//! 1. **状态少且只前向依赖**：每个槽位的状态是 `u32` 相位、若干 `f32` 包络电平、
//!    两个四极滤波器的 4 个状态变量、一个 `u32` RNG 状态、几个整数。
//!    逐样本只读"上一帧的自己" ⇒ 与块切分**构造性无关**
//!    （判据：`tests/drums_render.rs` 的 R10，6 种切分逐位比对）；
//! 2. **相位用整数推进**：`wrapping_add` 无累积误差、不受 FTZ/DAZ 影响；
//! 3. **噪声显式播种**：种子由 `(音色, 触发序号)` 推出（[`DrumMachine::trigger`]）
//!    ⇒ 没有熵源、跨运行/跨平台同序。**同一串触发**给同一段噪声；
//! 4. **clap 的 onset 定时是整数帧**（`frame` 与 `spacing_frames`），不是浮点秒；
//! 5. **窃取选择是全序**：`(是否低于 −60 dBFS, start_sample, 下标)` 三级比较，
//!    任何平台同解（[ARCH-RT-004] 的"振幅能量最低 **或** 最早被触发"）。
//!
//! # 6. 与既有判据的关系：**影响面为零**
//!
//! 本模块只**新增**文件（`src/drums/**`、`tests/drums_*.rs`）并往 `lib.rs` 加
//! `pub mod drums;` 与一行模块地图。既有判据里提到 `dsp` 的只有
//! `crates/yeban-engine/tests/limiter_contract.rs`（限制器）与
//! `synth_filter.rs`（`LadderFilter`）—— 两者都不碰鼓机
//! （测法：`grep -rn 'dsp' crates/yeban-engine/tests/*.rs`）。
//! ⚠ 本 crate 的库目标仍是 `#![forbid(unsafe_code)]`（`lib.rs:70`）；
//! 计数型全局分配器只存在于**集成测试目标**（树里已有 4 个同款先例）。
//!
//! # 7. 延迟上报（[ARCH-PDC-001]）
//!
//! [`DrumMachine::latency_samples`] 恒为 **0 帧**。理由不是"没有观察到延迟"，
//! 而是本器件是**声源**、不是输入信号的处理器：
//!
//! 1. **没有前视缓冲**：`render` 只在 `now >= DrumHit::start_sample` 时发声，
//!    触发落在哪一帧就从那一帧起音；
//! 2. **没有延迟线**：本模块不引用 `crate::delay`；
//! 3. **没有过采样往返**：本模块不引用 `crate::oversample`
//!    （波形在基础采样率上直接合成）。
//!
//! 构造性依据（可对账，不是断言）：把同一击的 `start_sample` 从 `0` 移到 `P`，
//! 渲染结果恰是原结果**整体后移 `P` 帧**，且 `[0, P)` 全为 `0.0`。
//! 判据 `tests/drums_rt_zero_alloc.rs` 的
//! `a_delayed_hit_shifts_the_waveform_by_exactly_the_hit_offset` 钉住这条，
//! 同文件的 `the_reported_latency_is_the_zero_constant` 钉住读数。
//! ⚠ 上报口径的**唯一事实源**仍是模型层的 `DeviceDefinition::latency_samples`
//!（本成员是它的构造性依据，不改变任何输出）。

use crate::envelope::STEAL_RELEASE_SECONDS;
use crate::math::sanitise_sample_rate;

mod voice;

use voice::Generator;

/// 一个器件实例的**槽位容量**（同时发声的鼓击上限），定长数组的长度。
///
/// ⚠ 规范**没有**给出鼓机的复音数（见 §1）。`16` 与本仓库的
/// `polysynth::VOICES_PER_SLOT` 同值，理由是它已经在树里、且与本仓库的
/// 固定处理块长 `128` 无关（容量由音乐密度决定，不由块长决定）。
pub const DRUM_SLOTS: usize = 16;

/// 一个音色同时占用一个槽位；槽位号与音色号是**两件事**（同音色可以重叠）。
/// `DrumVoice` 是**音色身份**，[`DrumHit`] 携带它。
///
/// 5 个取值里有 4 套**独立配方**：`ClosedHat` 与 `OpenHat` 共用踩镲配方、
/// 只有衰减时间不同（[`HiHatParams::closed_decay_s`] / [`HiHatParams::open_decay_s`]）。
/// 把开/闭镲分成两个取值（而不是一个"踩镲 + 布尔"）的理由：鼓机的"音色"就是
/// 鼓件，调用方与界面按鼓件寻址；且二者是**不同的 choke 角色**
/// （闭镲 choke 开镲，反之不成立）。
///
/// ⚠ 4 个音色只是"经典鼓机里最常见的 4 件"，**不是**"808/909 的完整鼓组"
/// （见 §0.1 第 11、12 条：909 专用配方与其余 808 音色都没做）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum DrumVoice {
    /// 底鼓（下滑正弦）。
    Kick = 0,
    /// 军鼓（双音调 + 带通噪声）。
    Snare = 1,
    /// 闭镲（6 条非谐方波 → 带通 → 快衰减）。
    ClosedHat = 2,
    /// 开镲（同闭镲配方，衰减更长）。
    OpenHat = 3,
    /// 拍手（带通噪声 × 多次 onset + 长尾巴）。
    Clap = 4,
}

impl DrumVoice {
    /// 全部音色（顺序 = 判别值顺序），供 UI/MCP 枚举与判据遍历。
    pub const ALL: [Self; 5] = [
        Self::Kick,
        Self::Snare,
        Self::ClosedHat,
        Self::OpenHat,
        Self::Clap,
    ];

    /// 判别值（与 [`Generator`] 内的 `u8` 同口径）。
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// 短名（诊断与读数用；不是本地化文案）。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Kick => "kick",
            Self::Snare => "snare",
            Self::ClosedHat => "closed_hat",
            Self::OpenHat => "open_hat",
            Self::Clap => "clap",
        }
    }

    /// 从判别值还原；越界返回 `None`（不 panic、不钳位）。
    #[must_use]
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Kick),
            1 => Some(Self::Snare),
            2 => Some(Self::ClosedHat),
            3 => Some(Self::OpenHat),
            4 => Some(Self::Clap),
            _ => None,
        }
    }
}

/// [`Generator`] 内部用的判别值常量（与 [`DrumVoice`] 的 `repr(u8)` 一致）。
pub(crate) const VOICE_KICK: u8 = DrumVoice::Kick as u8;
/// 军鼓的判别值。
pub(crate) const VOICE_SNARE: u8 = DrumVoice::Snare as u8;
/// 闭镲的判别值。
pub(crate) const VOICE_CLOSED_HAT: u8 = DrumVoice::ClosedHat as u8;
/// 开镲的判别值。
pub(crate) const VOICE_OPEN_HAT: u8 = DrumVoice::OpenHat as u8;
/// 拍手的判别值。
pub(crate) const VOICE_CLAP: u8 = DrumVoice::Clap as u8;

// ---------------------------------------------------------------------------
// 参数合法域（全部在 `sanitised` 里用；逐样本路径只读钳好的值）
// ---------------------------------------------------------------------------

/// 频率合法域下界（Hz）。
pub const MIN_FREQ_HZ: f32 = 20.0;
/// 频率合法域上界（Hz）。
///
/// 取 18 kHz：`LadderFilter::configure` 自己会把截止钳到 `sr × 0.45`
/// （`filter.rs:98`），18 kHz 在 44.1 kHz 的 `0.45×` = 19.845 kHz 之下
/// ⇒ 本上界不会在 44.1 kHz 上被滤波器再钳一次。
pub const MAX_FREQ_HZ: f32 = 18_000.0;
/// 时间合法域下界（秒）。
pub const MIN_TIME_S: f32 = 1.0e-4;
/// 时间合法域上界（秒）。8 s 远长于任何鼓件的尾巴。
pub const MAX_TIME_S: f32 = 8.0;
/// 包络时间（`glide_s` / 各 `decay_s`）的合法域下界（秒）。
///
/// 比 [`MIN_TIME_S`] 宽？不 —— 同值。单列出来是因为 `glide_s` 的语义不同
/// （它是音高包络，不是幅度包络），读者不该以为两者共用一个常量只是巧合。
pub const MIN_ENVELOPE_S: f32 = MIN_TIME_S;
/// 比值（`tone_ratio`）合法域下界。
pub const MIN_RATIO: f32 = 0.25;
/// 比值（`tone_ratio`）合法域上界。8× 已经越过 3 个八度。
pub const MAX_RATIO: f32 = 8.0;
/// clap 的 onset 个数合法域下界。
pub const MIN_BURSTS: u32 = 1;
/// clap 的 onset 个数合法域上界。8 个 onset 在 10 ms 间隔下是 80 ms 的"串"。
pub const MAX_BURSTS: u32 = 8;
/// clap 的 onset 间隔合法域下界（秒）。
pub const MIN_BURST_SPACING_S: f32 = 1.0e-3;
/// clap 的 onset 间隔合法域上界（秒）。
pub const MAX_BURST_SPACING_S: f32 = 0.2;
/// 带通两端的最小比值（`hi / lo`）。
///
/// `LP(hi) ∘ hp⁴` 只有在 `hi > lo` 时才是带通；两端相等时输出恒 0
/// （静音、有限）。本常量让"两端被钳到同一点"的情形恢复到一条有宽度的带。
pub const MIN_BAND_RATIO: f32 = 1.05;

/// 底鼓参数。
///
/// 默认值一列写明"选它的理由"；⚠ 这些数字**没有外部出处**（见模块文档 §1）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KickParams {
    /// **尾巴**基频（Hz），即音高包络走完后的静止频率。默认 `55.0`
    /// （A1 ≈ 55 Hz，是底鼓尾巴的常用落点；它不是从硬件测出来的）。
    pub tune_hz: f32,
    /// **起始**基频（Hz），即音高包络 = 1 时的频率。默认 `120.0`
    /// （比尾巴高约 1.13 个八度；下滑量决定"thump"的力度）。
    pub glide_hz: f32,
    /// 音高包络的时间（秒，一极点、6 个时间常数口径）。默认 `0.035`
    /// （下滑在约 40 ms 内走完 —— 比幅度尾巴快一个数量级，这是下滑型底鼓的定义）。
    pub glide_s: f32,
    /// 起振时间（秒）。默认 `0.0005`（0.5 ms ⇒ 24 帧 @48 kHz）。
    ///
    /// ⚠ 它**不是** 808 的脉冲整形器（见 §0.1 第 5 条）。它的唯一作用是
    /// 让每个音色都从 0 电平出发 —— 方波与白噪声在第 0 帧不是 0，
    /// 没有起振就是一次满量程阶跃（那是咔哒，不是鼓）。
    pub attack_s: f32,
    /// 幅度包络的时间（秒，6 个时间常数口径）。默认 `0.40`
    /// （−60 dB 落在约 0.46 s：一个"长的"底鼓，听得见尾巴）。
    pub decay_s: f32,
    /// 音色电平（`0..=1`）。默认 `1.0`。
    pub level: f32,
}

impl KickParams {
    /// 默认参数（每个字段的理由见字段注释）。
    pub const DEFAULT: Self = Self {
        tune_hz: 55.0,
        glide_hz: 120.0,
        glide_s: 0.035,
        attack_s: 0.0005,
        decay_s: 0.40,
        level: 1.0,
    };

    /// 把所有字段钳到合法域后的副本。**全定义**：任何输入（含 `NaN`/`∞`）都有返回。
    ///
    /// | 字段 | 合法域 | 低于下界 | 高于上界 | `NaN` |
    /// | :--- | :--- | :--- | :--- | :--- |
    /// | `tune_hz` / `glide_hz` | `[`[`MIN_FREQ_HZ`]`, `[`MAX_FREQ_HZ`]`]` | 下界 | 上界 | **下界** |
    /// | `glide_s` | `[`[`MIN_ENVELOPE_S`]`, `[`MAX_TIME_S`]`]` | 下界 | 上界 | **下界** |
    /// | `attack_s` / `decay_s` | `[`[`MIN_TIME_S`]`, `[`MAX_TIME_S`]`]` | 下界 | 上界 | **下界** |
    /// | `level` | `[0, 1]` | `0` | `1` | **`0`** |
    ///
    /// `NaN` 一律归**下界**：理由与 `compressor.rs:319` 的同一段注释相同 ——
    /// `f32::clamp` 对 `NaN` 是**恒等**（`NaN.clamp(a, b) == NaN`），
    /// 单靠 `clamp` 会漏一个 `NaN` 进实时路径。选下界是因为它在每个字段上都是
    /// **保守**的（最低频率、最短时间、静音）。
    #[must_use]
    pub fn sanitised(self) -> Self {
        Self {
            tune_hz: clamp_freq(self.tune_hz),
            glide_hz: clamp_freq(self.glide_hz),
            glide_s: clamp_envelope(self.glide_s),
            attack_s: clamp_time(self.attack_s),
            decay_s: clamp_time(self.decay_s),
            level: clamp_gain(self.level),
        }
    }
}

impl Default for KickParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 军鼓参数。默认值理由见字段注释；⚠ 数字**没有外部出处**（模块文档 §1）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnareParams {
    /// 第一个音调分量的频率（Hz）。默认 `185.0`（鼓皮的低声共振模式落点）。
    pub tone_hz: f32,
    /// 第二个音调分量相对第一个的比值。默认 `1.784`（≈ 330/185：
    /// 两个共振模式相差约 10 个半音 —— 它们不成简单整数比，因此有"鼓皮"感）。
    pub tone_ratio: f32,
    /// 音调分量的包络时间（秒，6 个时间常数口径）。默认 `0.12`。
    pub tone_decay_s: f32,
    /// 音调分量的电平（`0..=1`）。默认 `0.5`。
    pub tone_level: f32,
    /// 噪声**高通**的拐点（Hz）。默认 `3_000.0`。
    ///
    /// ⚠ 它是 `x − LP(拐点)` 这个**补**形式的拐点，**不是**带通下沿：
    /// 实测（`drums/voice.rs` 的四级级联表）拐点 `3 kHz` 时 452 Hz 只被压到
    /// `0.136`、1 kHz 已到 `1.0` ⇒ 有效通带约 **1–8 kHz**。
    pub noise_highpass_hz: f32,
    /// 噪声**低通**的拐点（Hz）。默认 `10_000.0`。
    pub noise_lowpass_hz: f32,
    /// 噪声分量的包络时间（秒）。默认 `0.10`
    /// （比音调分量**短**：响弦先散，鼓皮的音调留得久一点 —— 这是军鼓的定义性特征）。
    pub noise_decay_s: f32,
    /// 噪声分量的电平（`0..=1`）。默认 `1.0`。
    ///
    /// ⚠ 它与 `tone_level` **不是**两个分量的 RMS 之比：噪声要过带通，带通会
    /// 衰减它。本机实测（默认参数、1 s）：音调 RMS **0.02556**、噪声 RMS
    /// **0.04400** ⇒ 噪声/音调 = **+4.72 dB**（判据 S3 把这两个数钉住）。
    /// 取 `1.0`（顶格）就是为了让"噪声比音调响"这条性质有可测的余量。
    pub noise_level: f32,
    /// 起振时间（秒）。默认 `0.0005`（理由同 [`KickParams::attack_s`]）。
    pub attack_s: f32,
    /// 音色电平（`0..=1`）。默认 `1.0`。
    pub level: f32,
}

impl SnareParams {
    /// 默认参数（每个字段的理由见字段注释）。
    pub const DEFAULT: Self = Self {
        tone_hz: 185.0,
        tone_ratio: 1.784,
        tone_decay_s: 0.12,
        tone_level: 0.5,
        noise_highpass_hz: 3_000.0,
        noise_lowpass_hz: 10_000.0,
        noise_decay_s: 0.10,
        noise_level: 1.0,
        attack_s: 0.0005,
        level: 1.0,
    };

    /// 把所有字段钳到合法域后的副本。**全定义**（`NaN` 归下界，理由同
    /// [`KickParams::sanitised`]）。
    ///
    /// | 字段 | 合法域 | 低于下界 | 高于上界 | `NaN` |
    /// | :--- | :--- | :--- | :--- | :--- |
    /// | `tone_hz` | `[`[`MIN_FREQ_HZ`]`, `[`MAX_FREQ_HZ`]`]` | 下界 | 上界 | 下界 |
    /// | `tone_ratio` | `[`[`MIN_RATIO`]`, `[`MAX_RATIO`]`]` | 下界 | 上界 | **`1.0`** |
    /// | `tone_decay_s` / `noise_decay_s` / `attack_s` | `[`[`MIN_TIME_S`]`, `[`MAX_TIME_S`]`]` | 下界 | 上界 | 下界 |
    /// | `noise_highpass_hz` / `noise_lowpass_hz` | 先各钳到频率域，再保证 `hi ≥ lo × `[`MIN_BAND_RATIO`] | — | — | 下界 |
    /// | `tone_level` / `noise_level` / `level` | `[0, 1]` | `0` | `1` | `0` |
    #[must_use]
    pub fn sanitised(self) -> Self {
        let (noise_highpass_hz, noise_lowpass_hz) =
            sanitise_band(self.noise_highpass_hz, self.noise_lowpass_hz);
        Self {
            tone_hz: clamp_freq(self.tone_hz),
            tone_ratio: clamp_ratio(self.tone_ratio, 1.0),
            tone_decay_s: clamp_time(self.tone_decay_s),
            tone_level: clamp_gain(self.tone_level),
            noise_highpass_hz,
            noise_lowpass_hz,
            noise_decay_s: clamp_time(self.noise_decay_s),
            noise_level: clamp_gain(self.noise_level),
            attack_s: clamp_time(self.attack_s),
            level: clamp_gain(self.level),
        }
    }
}

impl Default for SnareParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 踩镲参数（闭镲与开镲共用；只有衰减时间不同）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HiHatParams {
    /// 6 条方波的**基频**（Hz）；第 `k` 条的频率是 `base_hz × HAT_RATIOS[k]`。
    /// 默认 `320.0`。⚠ 它**不是** 808 金属振荡器的时钟频率（§0.1 第 9 条）。
    pub base_hz: f32,
    /// **高通**的拐点（Hz）。默认 `6_000.0`。
    ///
    /// ⚠ 不是带通下沿：它是 `x − LP(拐点)` 的补形式，实测（四级级联）
    /// 拐点 `6 kHz` 时 452 Hz 压到 **0.0107**（−39 dB）、2 kHz 到 `0.99`
    /// ⇒ 有效通带约 **2–9 kHz**。
    pub highpass_hz: f32,
    /// **低通**的拐点（Hz）。默认 `14_000.0`。
    pub lowpass_hz: f32,
    /// 起振时间（秒）。默认 `0.0005`（理由同 [`KickParams::attack_s`]；
    /// 对方波尤其必要：第 0 帧 6 条方波都是 `+1`）。
    pub attack_s: f32,
    /// 闭镲的幅度包络时间（秒）。默认 `0.045`（−60 dB 落在约 52 ms）。
    pub closed_decay_s: f32,
    /// 开镲的幅度包络时间（秒）。默认 `0.32`（−60 dB 落在约 0.37 s）。
    pub open_decay_s: f32,
    /// 音色电平（`0..=1`）。默认 `0.7`（踩镲在鼓组里通常比底鼓轻）。
    pub level: f32,
}

impl HiHatParams {
    /// 默认参数（每个字段的理由见字段注释）。
    pub const DEFAULT: Self = Self {
        base_hz: 320.0,
        highpass_hz: 6_000.0,
        lowpass_hz: 14_000.0,
        attack_s: 0.0005,
        closed_decay_s: 0.045,
        open_decay_s: 0.32,
        level: 0.7,
    };

    /// 把所有字段钳到合法域后的副本。**全定义**（`NaN` 归下界，理由同
    /// [`KickParams::sanitised`]）。
    ///
    /// | 字段 | 合法域 | 低于下界 | 高于上界 | `NaN` |
    /// | :--- | :--- | :--- | :--- | :--- |
    /// | `base_hz` | `[`[`MIN_FREQ_HZ`]`, `[`MAX_FREQ_HZ`]`]` | 下界 | 上界 | 下界 |
    /// | `highpass_hz` / `lowpass_hz` | 同 [`SnareParams::sanitised`] 的带通口径 | — | — | 下界 |
    /// | `attack_s` / `closed_decay_s` / `open_decay_s` | `[`[`MIN_TIME_S`]`, `[`MAX_TIME_S`]`]` | 下界 | 上界 | 下界 |
    /// | `level` | `[0, 1]` | `0` | `1` | `0` |
    #[must_use]
    pub fn sanitised(self) -> Self {
        let (highpass_hz, lowpass_hz) = sanitise_band(self.highpass_hz, self.lowpass_hz);
        Self {
            base_hz: clamp_freq(self.base_hz),
            highpass_hz,
            lowpass_hz,
            attack_s: clamp_time(self.attack_s),
            closed_decay_s: clamp_time(self.closed_decay_s),
            open_decay_s: clamp_time(self.open_decay_s),
            level: clamp_gain(self.level),
        }
    }
}

impl Default for HiHatParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 拍手参数。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClapParams {
    /// **高通**的拐点（Hz）。默认 `2_000.0`。
    ///
    /// ⚠ 不是带通下沿（同 [`HiHatParams::highpass_hz`] 的口径）。
    pub highpass_hz: f32,
    /// **低通**的拐点（Hz）。默认 `5_000.0`。
    ///
    /// 本机实测：这一组给出的谱重心是 **1424 Hz**（拍手的主带在 1–2 kHz）。
    pub lowpass_hz: f32,
    /// **onset 个数**（`1..=8`）。默认 `4` = 3 次短促 + 1 次带长尾巴。
    ///
    /// 语义是"包络从 0 起跳的次数"，不是"短促次数"。判据
    /// （`tests/drums_render.rs` 的 C1）从**渲染出的音频**数 onset，必须等于它。
    pub bursts: u32,
    /// 短促 onset 的包络时间（秒，6 个时间常数口径）。默认 `0.012`。
    pub burst_decay_s: f32,
    /// onset 之间的间隔（秒）。默认 `0.010`（10 ms：多只手落下的错开量）。
    pub burst_spacing_s: f32,
    /// 起振时间（秒）。默认 `0.0005`（理由同 [`KickParams::attack_s`]）。
    pub attack_s: f32,
    /// **最后一个 onset** 的长尾巴包络时间（秒）。默认 `0.22`。
    pub decay_s: f32,
    /// 音色电平（`0..=1`）。默认 `0.8`。
    pub level: f32,
}

impl ClapParams {
    /// 默认参数（每个字段的理由见字段注释）。
    pub const DEFAULT: Self = Self {
        highpass_hz: 2_000.0,
        lowpass_hz: 5_000.0,
        bursts: 4,
        burst_decay_s: 0.012,
        burst_spacing_s: 0.010,
        attack_s: 0.0005,
        decay_s: 0.22,
        level: 0.8,
    };

    /// 把所有字段钳到合法域后的副本。**全定义**（`NaN` 归下界，理由同
    /// [`KickParams::sanitised`]）。
    ///
    /// | 字段 | 合法域 | 低于下界 | 高于上界 | `NaN` |
    /// | :--- | :--- | :--- | :--- | :--- |
    /// | `highpass_hz` / `lowpass_hz` | 同 [`SnareParams::sanitised`] 的带通口径 | — | — | 下界 |
    /// | `bursts` | `[`[`MIN_BURSTS`]`, `[`[`MAX_BURSTS`]`]` | 下界 | 上界 | 不适用（`u32`） |
    /// | `burst_decay_s` / `attack_s` / `decay_s` | `[`[`MIN_TIME_S`]`, `[`MAX_TIME_S`]`]` | 下界 | 上界 | 下界 |
    /// | `burst_spacing_s` | `[`[`MIN_BURST_SPACING_S`]`, `[`[`MAX_BURST_SPACING_S`]`]` | 下界 | 上界 | 下界 |
    /// | `level` | `[0, 1]` | `0` | `1` | `0` |
    #[must_use]
    pub fn sanitised(self) -> Self {
        let (highpass_hz, lowpass_hz) = sanitise_band(self.highpass_hz, self.lowpass_hz);
        Self {
            highpass_hz,
            lowpass_hz,
            bursts: self.bursts.clamp(MIN_BURSTS, MAX_BURSTS),
            burst_decay_s: clamp_time(self.burst_decay_s),
            burst_spacing_s: clamp_spacing(self.burst_spacing_s),
            attack_s: clamp_time(self.attack_s),
            decay_s: clamp_time(self.decay_s),
            level: clamp_gain(self.level),
        }
    }
}

impl Default for ClapParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 整套鼓组的参数（4 个音色 + 总线电平）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrumKitParams {
    /// 底鼓。
    pub kick: KickParams,
    /// 军鼓。
    pub snare: SnareParams,
    /// 踩镲（闭镲与开镲）。
    pub hihat: HiHatParams,
    /// 拍手。
    pub clap: ClapParams,
    /// 总线电平（`0..=1`）。默认 `1.0`。
    ///
    /// ⚠ 它是**线性增益**，不是 dB（本 crate 的口径：参数一律 `f32`、
    /// 频率一律 Hz、时间一律秒；dB 只出现在明确叫 `*_db` 的字段里）。
    pub master_level: f32,
}

impl DrumKitParams {
    /// 默认鼓组（= 4 个音色各自的 [`Default`]）。
    pub const DEFAULT: Self = Self {
        kick: KickParams::DEFAULT,
        snare: SnareParams::DEFAULT,
        hihat: HiHatParams::DEFAULT,
        clap: ClapParams::DEFAULT,
        master_level: 1.0,
    };

    /// 把 4 个音色与总线电平全部钳到合法域后的副本（`NaN` 归下界）。
    #[must_use]
    pub fn sanitised(self) -> Self {
        Self {
            kick: self.kick.sanitised(),
            snare: self.snare.sanitised(),
            hihat: self.hihat.sanitised(),
            clap: self.clap.sanitised(),
            master_level: clamp_gain(self.master_level),
        }
    }
}

impl Default for DrumKitParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 频率钳制：`NaN` ⇒ 下界。
#[must_use]
fn clamp_freq(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(MIN_FREQ_HZ, MAX_FREQ_HZ)
    } else {
        MIN_FREQ_HZ
    }
}

/// 时间钳制：`NaN` ⇒ 下界。
#[must_use]
fn clamp_time(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(MIN_TIME_S, MAX_TIME_S)
    } else {
        MIN_TIME_S
    }
}

/// 包络时间钳制：`NaN` ⇒ 下界。
#[must_use]
fn clamp_envelope(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(MIN_ENVELOPE_S, MAX_TIME_S)
    } else {
        MIN_ENVELOPE_S
    }
}

/// onset 间隔钳制：`NaN` ⇒ 下界。
#[must_use]
fn clamp_spacing(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(MIN_BURST_SPACING_S, MAX_BURST_SPACING_S)
    } else {
        MIN_BURST_SPACING_S
    }
}

/// 增益钳制：`NaN` ⇒ `0`（静音，最保守）。
#[must_use]
fn clamp_gain(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// 比值钳制：`NaN` ⇒ `nan_fallback`（调用方给出该字段的中性值）。
#[must_use]
fn clamp_ratio(value: f32, nan_fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(MIN_RATIO, MAX_RATIO)
    } else {
        nan_fallback
    }
}

/// 带通两端：各自钳到频率域，排序，再保证 `hi ≥ lo × MIN_BAND_RATIO`。
///
/// 退化情形（两端都顶到 [`MAX_FREQ_HZ`]）**不会被强行撑开**：此时
/// `LP(hi)(hp⁴(x))` 的输出恒 0（静音且有限）。这是**有意的**：
/// 与其发明一条规则，不如让不可表达的请求退化为静音。
#[must_use]
fn sanitise_band(first: f32, second: f32) -> (f32, f32) {
    let a = clamp_freq(first);
    let b = clamp_freq(second);
    let (low, high) = if a <= b { (a, b) } else { (b, a) };
    let high = if high >= low * MIN_BAND_RATIO {
        high
    } else {
        (low * MIN_BAND_RATIO).min(MAX_FREQ_HZ)
    };
    (low, high)
}

/// 窃取淡出的帧数：`STEAL_RELEASE_SECONDS`（3 ms）× 采样率。
///
/// 上限 0.5 s（采样率异常大时也不会把槽位卡死），下限 1 帧。
/// 与 `polysynth::steal_fade_frames_for` 同口径。
#[must_use]
pub fn steal_fade_frames_for(sample_rate: f32) -> u32 {
    let frames = STEAL_RELEASE_SECONDS * sanitise_sample_rate(sample_rate);
    if !frames.is_finite() {
        return 1;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let clamped = frames.round().clamp(1.0, 24_000.0) as u32;
    clamped
}

/// 一次**鼓击触发请求**（控制侧组装；实时侧只读）。
///
/// 与 `polysynth::NoteEvent` 的区别是**没有终点**：鼓击是一击即走
/// （one-shot），结束由包络自己决定（[`Generator::is_finished`]）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrumHit {
    /// 音色身份。
    voice: DrumVoice,
    /// 起始的绝对样本位置（支持提前排程）。
    start_sample: u64,
    /// 这一击的增益（`0..=1`，构造期钳制；非有限 ⇒ `0`）。
    gain: f32,
}

impl DrumHit {
    /// 构造。`gain` 非有限 ⇒ `0`（静音），越界 ⇒ 钳到 `[0, 1]`。
    #[must_use]
    pub fn new(voice: DrumVoice, start_sample: u64, gain: f32) -> Self {
        Self {
            voice,
            start_sample,
            gain: clamp_gain(gain),
        }
    }

    /// 音色身份。
    #[must_use]
    pub const fn voice(&self) -> DrumVoice {
        self.voice
    }

    /// 起始绝对样本位置。
    #[must_use]
    pub const fn start_sample(&self) -> u64 {
        self.start_sample
    }

    /// 这一击的增益（`0..=1`）。
    #[must_use]
    pub const fn gain(&self) -> f32 {
        self.gain
    }
}

/// `u32` 采样率 → 钳好的 `f32` 采样率（Hz）。
///
/// `u32 → f32` 在 `u32::MAX` 附近会丢低位，但那个量级（4.29 GHz）本来就不是
/// 合法采样率，钳制会把它压到合法域；真正的采样率由
/// [`DrumMachine::set_sample_rate`] 校准。与 `polysynth::new` 的口径一致。
#[must_use]
fn sample_rate_from_u32(sample_rate: u32) -> f32 {
    sanitise_sample_rate(sample_rate as f32)
}

/// 一个槽位的调度状态。
#[derive(Clone, Copy, Debug)]
struct Slot {
    /// 是否被占用（占用中但 `now < start_sample` 表示还没开始发声）。
    active: bool,
    /// 这一击的增益。
    gain: f32,
    /// 起始绝对样本位置。
    start_sample: u64,
    /// 还剩多少帧的窃取/闭镲淡出（`> 0` 表示正在淡出）。
    fade_remaining: u32,
    /// 淡出走完后立刻接管的新一击（`None` = 淡出完就回收）。
    pending: Option<PendingHit>,
    /// 信号发生器。
    generator: Generator,
}

/// 淡出期间**挂起**的一击（[ARCH-RT-004] 的"旧声部淡出、新声部起音"两件事
/// 永不同时发声 ⇒ 不可能叠加爆音）。
#[derive(Clone, Copy, Debug)]
struct PendingHit {
    generator: Generator,
    gain: f32,
    start_sample: u64,
}

impl Slot {
    /// 空槽位。
    const IDLE: Self = Self {
        active: false,
        gain: 0.0,
        start_sample: 0,
        fade_remaining: 0,
        pending: None,
        generator: Generator::IDLE,
    };

    /// 让这一击占用本槽位（不改 `fade_remaining` 的调用点语义：调用方先清零）。
    fn arm(&mut self, pending: PendingHit) {
        self.active = true;
        self.gain = pending.gain;
        self.start_sample = pending.start_sample;
        self.fade_remaining = 0;
        self.pending = None;
        self.generator = pending.generator;
    }
}

/// 窃取优先级的第一级：`0` = 已经低于 −60 dBFS，`1` = 仍在响。
/// 数字小者优先被窃取。
///
/// `0.001` 就是 −60 dBFS（[ARCH-RT-004] 原文的阈值），与
/// `polysynth::steal_priority`（`polysynth.rs:662`）同口径。
#[must_use]
fn steal_priority(slot: &Slot) -> u8 {
    if slot.generator.level() < 0.001 { 0 } else { 1 }
}

/// 4 音色鼓机：**定容预分配槽位池** + 确定性窃取 + 3 ms 指数淡出。
///
/// 容量 `SLOTS` 是编译期常量（默认 [`DRUM_SLOTS`]）⇒ 槽位池是定长数组，
/// [`DrumMachine::trigger`] / [`DrumMachine::render`] / [`DrumMachine::set_params`]
/// 全部**零分配、零锁、零 I/O**（模块文档 §3）。
///
/// 形状参照 `crates/yeban-sfz/src/voice_pool.rs`（定容池 + 确定性窃取 + 3 ms
/// 指数淡出）与 [`crate::polysynth::PolySynth`]，但**不依赖** `yeban-sfz`
/// （那会给 `yeban-dsp` 加一条内部依赖）。
#[derive(Clone, Copy, Debug)]
pub struct DrumMachine<const SLOTS: usize = DRUM_SLOTS> {
    slots: [Slot; SLOTS],
    params: DrumKitParams,
    sample_rate: f32,
    /// 窃取淡出帧数（默认 3 ms ⇒ @48 kHz = 144 帧）。判据可覆盖成 0（硬窃取）。
    steal_fade_frames: u32,
    /// 白噪声播种计数（每触发一次 +1；与音色号混合成种子）。
    ///
    /// 它是**音频状态**而不是诊断计数器：它决定每一击听到的是哪一段噪声，
    /// 因此 [`DrumMachine::reset`] 把它归零（见那里的契约）。
    trigger_seed: u32,
    triggers: u64,
    voice_steals: u64,
    hat_chokes: u64,
    /// 累计的**发声槽位帧**（一个槽位渲染出一个非零样本记 1）。
    ///
    /// 它是判据的**逐槽位**覆盖度读数（不是"这一帧里有没有声音"）。
    /// 见 [`DrumMachine::sounding_slot_frames`] 的文档。
    sounding_slot_frames: u64,
}

impl<const SLOTS: usize> DrumMachine<SLOTS> {
    /// 构造：默认鼓组、默认 3 ms 淡出、槽位全部空闲。
    ///
    /// ⚠ 规范**没有**规定鼓机的默认音色；[`DrumKitParams::DEFAULT`] 是本模块
    /// 声明的（每个默认值旁边写了理由）。`sample_rate` 非法（`0`/非有限）时
    /// 按 [`crate::MIN_SAMPLE_RATE`] 处理。
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        let sample_rate = sample_rate_from_u32(sample_rate);
        Self {
            slots: [Slot::IDLE; SLOTS],
            params: DrumKitParams::DEFAULT,
            sample_rate,
            steal_fade_frames: steal_fade_frames_for(sample_rate),
            trigger_seed: 0,
            triggers: 0,
            voice_steals: 0,
            hat_chokes: 0,
            sounding_slot_frames: 0,
        }
    }

    /// 当前采样率（Hz）。
    #[must_use]
    pub const fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// 槽位容量（同时发声的鼓击上限）。
    #[must_use]
    pub const fn slots(&self) -> usize {
        SLOTS
    }

    /// 当前被占用的槽位数。
    #[must_use]
    pub fn active_slots(&self) -> usize {
        self.slots.iter().filter(|slot| slot.active).count()
    }

    /// 累计触发的鼓击数（含被窃取的）。
    #[must_use]
    pub const fn triggers(&self) -> u64 {
        self.triggers
    }

    /// 累计的窃取次数（池满时替换了一个槽位）。
    #[must_use]
    pub const fn voice_steals(&self) -> u64 {
        self.voice_steals
    }

    /// 累计的闭镲 choke 开镲次数。
    #[must_use]
    pub const fn hat_chokes(&self) -> u64 {
        self.hat_chokes
    }

    /// 累计的**发声槽位帧**（单位：槽位×帧）。
    ///
    /// 一个槽位在一次 `render` 的某一帧里产出非零样本，就记 1。除以
    /// `渲染的帧数` 就得到"平均同时发声的槽位数"。
    ///
    /// ⚠ 它是**覆盖度仪器**，不是电平表：非零只说明"这个槽位在算东西"，
    /// 不说明它多响（一个 −120 dB 的尾音也算）。**不改变音频输出**。
    #[must_use]
    pub const fn sounding_slot_frames(&self) -> u64 {
        self.sounding_slot_frames
    }

    /// 当前窃取淡出帧数。
    #[must_use]
    pub const fn steal_fade_frames(&self) -> u32 {
        self.steal_fade_frames
    }

    /// 覆盖窃取淡出帧数（`0` = 硬窃取）。判据用它注入"没有淡出"的情形。
    pub fn set_steal_fade_frames(&mut self, frames: u32) {
        self.steal_fade_frames = frames;
    }

    /// 当前参数。
    #[must_use]
    pub const fn params(&self) -> DrumKitParams {
        self.params
    }

    /// 换采样率并**重算所有在响槽位的系数**（相位与包络电平保留）。
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        let sample_rate = sample_rate_from_u32(sample_rate);
        if sample_rate == self.sample_rate {
            return;
        }
        self.sample_rate = sample_rate;
        self.steal_fade_frames = steal_fade_frames_for(sample_rate);
        for slot in &mut self.slots {
            if slot.active && slot.fade_remaining == 0 {
                slot.generator.reconfigure(&self.params, sample_rate);
            }
        }
    }

    /// 换参数：钳到合法域，并对**没有在淡出**的在响槽位重算系数
    /// （相位、包络电平、RNG 状态、滤波器状态、clap 的 `frame` 都保留）。
    ///
    /// 正在淡出的槽位**跳过**：它的 3 ms release 是窃取的实现本体，
    /// 重算会把它覆盖回正常释放 ⇒ 等于取消淡出。代价是那一击的其余系数
    /// 停在旧参数上（最多 3 ms）。
    pub fn set_params(&mut self, params: DrumKitParams) {
        self.params = params.sanitised();
        for slot in &mut self.slots {
            if slot.active && slot.fade_remaining == 0 {
                slot.generator.reconfigure(&self.params, self.sample_rate);
            }
        }
    }

    /// 全部槽位回到空闲，并把**白噪声播种计数**归零
    /// （不改参数、不改诊断计数器、不改采样率）。
    ///
    /// 契约（由判据
    /// `drums::tests::reset_reproduces_a_freshly_built_machine_bit_for_bit` 钉住）：
    /// `reset()` 之后再处理，与一台**用同样参数新建**的实例在同一串触发、同一段
    /// 位置上**逐位相同**。
    ///
    /// ⚠ 播种计数（`trigger_seed`）是**音频状态**，不是诊断计数器：每一击的白噪声
    /// 种子由"音色号 × 播种计数"推出，它决定这一击听到的是**哪一段**噪声。只清槽位池
    /// 的话，一次定位（引擎 `seek` 的接入点就是本方法）之后重放同一串鼓击会得到
    /// **与从头渲染不同**的噪声 ⇒ 同一份工程两次渲染不逐位相同 [ARCH-DET-001]。
    /// 四个诊断计数器（`triggers` / `voice_steals` / `hat_chokes` /
    /// `sounding_slot_frames`）**照旧只增不减** —— 它们不改变音频输出。
    ///
    /// **逐样本零分配**（一个标量写），可以在实时线程上调用 [ARCH-RT-001]。
    pub fn reset(&mut self) {
        self.slots = [Slot::IDLE; SLOTS];
        self.trigger_seed = 0;
    }

    /// 本器件引入的处理延迟：**恒为 `0` 帧** [ARCH-PDC-001]。
    ///
    /// 本器件是**声源**，不是输入信号的处理器：没有前视缓冲、没有延迟线、
    /// 没有过采样往返。`render` 只在 `now >= DrumHit::start_sample` 时发声，
    /// 触发帧就是发声的首帧。
    ///
    /// 构造性依据与判据见模块文档 §7。**实时路径**：`const fn`，零分配、零锁。
    #[must_use]
    pub const fn latency_samples(&self) -> usize {
        0
    }

    /// 回收"包络已走完且不在淡出中"的槽位。
    pub fn retire_finished(&mut self, position: u64) {
        for slot in &mut self.slots {
            if slot.active
                && slot.fade_remaining == 0
                && slot.start_sample <= position
                && slot.generator.is_finished()
            {
                *slot = Slot::IDLE;
            }
        }
    }

    /// 单个槽位的诊断快照：`(占用, 音色判别值, 起始样本, 淡出剩余帧)`。
    ///
    /// 它不分配、不改状态（同 `polysynth::debug_voice` 的理由：不按
    /// `debug_assertions` 门控，否则 `cargo test --release` 编译不过）。
    #[must_use]
    pub fn debug_slot(&self, index: usize) -> Option<(bool, u8, u64, u32)> {
        self.slots.get(index).map(|slot| {
            (
                slot.active,
                slot.generator.voice(),
                slot.start_sample,
                slot.fade_remaining,
            )
        })
    }

    /// 触发一次鼓击（**实时路径**）。
    ///
    /// 分配顺序：
    ///
    /// 1. 有空槽 ⇒ 直接占用；
    /// 2. 池满 ⇒ 按 [ARCH-RT-004] 选一个被终止者：**已经低于 −60 dBFS 的优先**，
    ///    同档内 **`start_sample` 最小（最早触发）** 的优先，仍然并列取
    ///    **下标最小**者。三级比较合起来是全序 ⇒ 任何平台同解；
    /// 3. 被窃取的槽位进入 `steal_fade_frames` 帧（默认 3 ms）的指数淡出，
    ///    新一击**挂起**在同一槽位上，淡出走完的那一帧才起音
    ///    ⇒ 旧的不响时新的才响，二者不可能相加；
    /// 4. `steal_fade_frames == 0` 退化为**硬窃取**（判据注入用）：直接覆盖，
    ///    输出会在样本间出现跃变。
    ///
    /// ⚠ 闭镲 choke 开镲：触发 [`DrumVoice::ClosedHat`] 时，所有在响的开镲槽位
    /// 立刻进入同一条淡出（模块文档 §4.1）。反向（开镲 choke 闭镲）**不做**。
    ///
    /// **实时路径**：零分配、零释放、零锁、零阻塞 I/O、零日志。
    pub fn trigger(&mut self, hit: DrumHit) {
        let voice = hit.voice();
        let voice_id = voice.as_u8();
        self.trigger_seed = self.trigger_seed.wrapping_add(1);
        let seed = u32::from(voice_id)
            .wrapping_mul(0x9E37_79B9)
            .wrapping_add(self.trigger_seed);
        let pending = PendingHit {
            generator: Generator::new(voice_id, &self.params, self.sample_rate, seed),
            gain: hit.gain(),
            start_sample: hit.start_sample(),
        };
        self.triggers = self.triggers.saturating_add(1);

        if voice == DrumVoice::ClosedHat {
            let fade_frames = self.steal_fade_frames;
            let mut chokes = 0u64;
            for slot in &mut self.slots {
                if slot.active
                    && slot.generator.voice() == VOICE_OPEN_HAT
                    && slot.fade_remaining == 0
                {
                    if fade_frames == 0 {
                        *slot = Slot::IDLE;
                    } else {
                        slot.generator.begin_fade();
                        slot.fade_remaining = fade_frames;
                        slot.pending = None;
                    }
                    chokes = chokes.saturating_add(1);
                }
            }
            self.hat_chokes = self.hat_chokes.saturating_add(chokes);
        }

        if let Some(index) = self.slots.iter().position(|slot| !slot.active) {
            self.slots[index].arm(pending);
            return;
        }

        // 池满：全序三级比较选被终止者（见本方法文档 §2）。
        let mut best = 0usize;
        let mut best_key = (
            steal_priority(&self.slots[0]),
            self.slots[0].start_sample,
            0usize,
        );
        for index in 1..SLOTS {
            let key = (
                steal_priority(&self.slots[index]),
                self.slots[index].start_sample,
                index,
            );
            if key < best_key {
                best = index;
                best_key = key;
            }
        }
        self.voice_steals = self.voice_steals.saturating_add(1);

        if self.steal_fade_frames == 0 || !self.slots[best].active {
            self.slots[best].arm(pending);
            return;
        }
        let victim = &mut self.slots[best];
        if victim.fade_remaining == 0 {
            victim.generator.begin_fade();
            victim.fade_remaining = self.steal_fade_frames;
        }
        victim.pending = Some(pending);
    }

    /// 渲染本器件的下一个块（**实时路径**）。`out.len()` 就是本块的有效帧数。
    ///
    /// `position` 是 `out[0]` 对应的**绝对样本位置**（由调用方的播放头给出）：
    /// 槽位只在 `now >= start_sample` 时发声。
    ///
    /// ⚠ 输出**不做**软限幅、不做归一化：它是所有在响槽位的原始和乘
    /// [`DrumKitParams::master_level`]（与 `polysynth::render` 同口径 ——
    /// 总线限幅是 `limiter` 的职责，器件内再限一次会让读数不可对账）。
    /// 16 个槽位同时满电平相加因此**可以超过 ±1**；那是调用方的余量问题，
    /// 不是本器件的数值缺陷（输出仍**恒为有限数**，见 `voice.rs` §3）。
    ///
    /// **实时路径**：零分配、零释放、零锁、零阻塞 I/O、零日志
    /// [ARCH-RT-001 / `MUST-GATE-001`]。
    pub fn render(&mut self, position: u64, out: &mut [f32]) {
        out.fill(0.0);
        if !self.slots.iter().any(|slot| slot.active) {
            return;
        }
        let master = self.params.master_level;
        let mut sounding: u64 = 0;
        for (frame, output) in out.iter_mut().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let now = position + frame as u64;
            let mut accumulator = 0.0f32;
            for slot in &mut self.slots {
                if !slot.active || now < slot.start_sample {
                    continue;
                }
                // --- 淡出：旧一击指数衰减；期满换成挂起的新一击 ---
                if slot.fade_remaining > 0 {
                    slot.fade_remaining -= 1;
                    let value = slot.generator.next();
                    if slot.fade_remaining == 0 {
                        match slot.pending.take() {
                            Some(pending) => {
                                // 新一击从这一帧开始（相位从 0、包络从 0 起振）。
                                // 旧一击在上一帧已衰减到 ≈0 ⇒ 中间没有"两个波形的和"。
                                let pending = PendingHit {
                                    start_sample: pending.start_sample.max(now),
                                    ..pending
                                };
                                slot.arm(pending);
                            }
                            None => {
                                *slot = Slot::IDLE;
                                continue;
                            }
                        }
                    } else {
                        if value != 0.0 {
                            sounding = sounding.saturating_add(1);
                        }
                        accumulator += value * slot.gain;
                        continue;
                    }
                }
                if slot.generator.is_finished() {
                    *slot = Slot::IDLE;
                    continue;
                }
                let sample = slot.generator.next();
                if sample != 0.0 {
                    sounding = sounding.saturating_add(1);
                }
                accumulator += sample * slot.gain;
            }
            *output = accumulator * master;
        }
        self.sounding_slot_frames = self.sounding_slot_frames.saturating_add(sounding);
    }
}

impl<const SLOTS: usize> Default for DrumMachine<SLOTS> {
    fn default() -> Self {
        Self::new(48_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 量什么：`sanitised()` 的每个字段在**敌意输入**（`NaN`/`+∞`/`−∞`/越界）下
    /// 是否都落进模块文档写明的合法域（单位：各字段自己的单位）。
    #[test]
    fn sanitised_clamps_every_field_into_its_documented_domain() {
        let hostile = DrumKitParams {
            kick: KickParams {
                tune_hz: f32::NAN,
                glide_hz: f32::INFINITY,
                glide_s: -5.0,
                attack_s: f32::NAN,
                decay_s: 1.0e9,
                level: f32::NEG_INFINITY,
            },
            snare: SnareParams {
                tone_hz: -1.0,
                tone_ratio: f32::NAN,
                tone_decay_s: 0.0,
                tone_level: 2.0,
                noise_highpass_hz: 9_000.0,
                noise_lowpass_hz: 100.0,
                noise_decay_s: f32::INFINITY,
                noise_level: -3.0,
                attack_s: f32::NAN,
                level: 42.0,
            },
            hihat: HiHatParams {
                base_hz: f32::NAN,
                highpass_hz: 1.0,
                lowpass_hz: 1.0,
                attack_s: -1.0,
                closed_decay_s: f32::NAN,
                open_decay_s: 1.0e9,
                level: 1.5,
            },
            clap: ClapParams {
                highpass_hz: f32::NAN,
                lowpass_hz: f32::INFINITY,
                bursts: 0,
                burst_decay_s: -1.0,
                burst_spacing_s: f32::NAN,
                attack_s: f32::NAN,
                decay_s: 1.0e9,
                level: f32::NAN,
            },
            master_level: f32::NAN,
        };
        let kit = hostile.sanitised();

        let finite = |value: f32| value.is_finite();
        assert!(finite(kit.kick.tune_hz) && finite(kit.kick.glide_hz));
        assert!((MIN_FREQ_HZ..=MAX_FREQ_HZ).contains(&kit.kick.tune_hz));
        assert!((MIN_FREQ_HZ..=MAX_FREQ_HZ).contains(&kit.kick.glide_hz));
        assert!((MIN_ENVELOPE_S..=MAX_TIME_S).contains(&kit.kick.glide_s));
        assert!((MIN_TIME_S..=MAX_TIME_S).contains(&kit.kick.attack_s));
        assert!((MIN_TIME_S..=MAX_TIME_S).contains(&kit.kick.decay_s));
        assert_eq!(kit.kick.level, 0.0, "NaN 音量必须退化为静音");
        assert!((MIN_RATIO..=MAX_RATIO).contains(&kit.snare.tone_ratio));
        assert!(
            kit.snare.noise_lowpass_hz >= kit.snare.noise_highpass_hz * MIN_BAND_RATIO,
            "带通两端必须有序：{} / {}",
            kit.snare.noise_highpass_hz,
            kit.snare.noise_lowpass_hz
        );
        assert!(kit.snare.noise_highpass_hz < kit.snare.noise_lowpass_hz);
        assert_eq!(kit.snare.tone_level, 1.0);
        assert_eq!(kit.snare.noise_level, 0.0);
        assert_eq!(kit.hihat.level, 1.0);
        assert!(kit.hihat.lowpass_hz > kit.hihat.highpass_hz);
        assert_eq!(kit.clap.bursts, MIN_BURSTS);
        assert!((MIN_BURST_SPACING_S..=MAX_BURST_SPACING_S).contains(&kit.clap.burst_spacing_s));
        assert!(kit.clap.lowpass_hz > kit.clap.highpass_hz);
        assert_eq!(kit.master_level, 0.0);

        // 全部字段有限（逐个 walk，不靠上面逐条的抽样）。
        for value in [
            kit.kick.tune_hz,
            kit.kick.glide_hz,
            kit.kick.glide_s,
            kit.kick.attack_s,
            kit.kick.decay_s,
            kit.kick.level,
            kit.snare.tone_hz,
            kit.snare.tone_ratio,
            kit.snare.tone_decay_s,
            kit.snare.tone_level,
            kit.snare.noise_highpass_hz,
            kit.snare.noise_lowpass_hz,
            kit.snare.noise_decay_s,
            kit.snare.noise_level,
            kit.snare.attack_s,
            kit.snare.level,
            kit.hihat.base_hz,
            kit.hihat.highpass_hz,
            kit.hihat.lowpass_hz,
            kit.hihat.attack_s,
            kit.hihat.closed_decay_s,
            kit.hihat.open_decay_s,
            kit.hihat.level,
            kit.clap.highpass_hz,
            kit.clap.lowpass_hz,
            kit.clap.burst_decay_s,
            kit.clap.burst_spacing_s,
            kit.clap.attack_s,
            kit.clap.decay_s,
            kit.clap.level,
            kit.master_level,
        ] {
            assert!(value.is_finite(), "sanitised 漏了非有限值：{value}");
        }
    }

    /// 量什么：**敌意参数**下每个音色的渲染输出是否**恒为有限数**
    /// （`NaN`/`Inf` 计数，单位：个样本）。
    #[test]
    fn hostile_parameters_never_produce_a_non_finite_sample() {
        let kit = DrumKitParams {
            kick: KickParams {
                tune_hz: f32::NAN,
                glide_hz: f32::NEG_INFINITY,
                glide_s: f32::NAN,
                attack_s: f32::INFINITY,
                decay_s: f32::NAN,
                level: f32::NAN,
            },
            snare: SnareParams {
                tone_hz: f32::INFINITY,
                tone_ratio: f32::NAN,
                tone_decay_s: f32::NAN,
                tone_level: f32::NAN,
                noise_highpass_hz: f32::NAN,
                noise_lowpass_hz: f32::NAN,
                noise_decay_s: f32::NAN,
                noise_level: f32::NAN,
                attack_s: f32::NAN,
                level: f32::NAN,
            },
            hihat: HiHatParams {
                base_hz: f32::INFINITY,
                highpass_hz: f32::NAN,
                lowpass_hz: f32::NAN,
                attack_s: f32::NAN,
                closed_decay_s: f32::NAN,
                open_decay_s: f32::NAN,
                level: f32::NAN,
            },
            clap: ClapParams {
                highpass_hz: f32::NAN,
                lowpass_hz: f32::NAN,
                bursts: u32::MAX,
                burst_decay_s: f32::NAN,
                burst_spacing_s: f32::NAN,
                attack_s: f32::NAN,
                decay_s: f32::NAN,
                level: f32::NAN,
            },
            master_level: f32::INFINITY,
        };
        for voice in DrumVoice::ALL {
            let mut machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
            machine.set_params(kit);
            machine.trigger(DrumHit::new(voice, 0, f32::NAN));
            let mut out = vec![0.0f32; 4_800];
            machine.render(0, &mut out);
            let bad = out.iter().filter(|sample| !sample.is_finite()).count();
            assert_eq!(bad, 0, "{} 产出 {bad} 个非有限样本", voice.name());
        }
    }

    /// 量什么：**未被触发过的**器件渲染一整块是否是**逐位全 0**
    /// （单位：非零样本数）。
    #[test]
    fn an_untouched_machine_renders_bit_exact_silence() {
        let mut machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
        let mut out = vec![7.0f32; 512];
        machine.render(0, &mut out);
        assert!(
            out.iter().all(|sample| *sample == 0.0),
            "空闲器件必须输出逐位 0"
        );
        assert_eq!(machine.active_slots(), 0);
        assert_eq!(machine.triggers(), 0);
        assert_eq!(machine.voice_steals(), 0);
    }

    /// 量什么：池满之后继续触发，`voice_steals` 是否**恰好等于**超出的次数
    /// （单位：次），以及被窃取的那个槽位是否就是全序三级比较选出的那一个。
    ///
    /// 夹具刻意让**电平**成为区分项：8 记底鼓（尾巴 0.40 s）＋ 8 记闭镲
    /// （尾巴 0.045 s），都在样本 0；渲染 3 000 帧（62.5 ms）之后闭镲已落到
    /// −60 dBFS 以下（`Adsr` 实测值 2.6e−4），底鼓仍在 0.39。
    /// ⇒ 第 17 记**必须**落在最早触发的那一记闭镲（槽 8）上，而不是"下标最小"
    /// 的槽 0（那是底鼓）。
    #[test]
    fn more_hits_than_slots_steals_the_lowest_level_slot_deterministically() {
        let mut machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
        for _ in 0..8 {
            machine.trigger(DrumHit::new(DrumVoice::Kick, 0, 1.0));
        }
        for _ in 0..8 {
            machine.trigger(DrumHit::new(DrumVoice::ClosedHat, 0, 1.0));
        }
        assert_eq!(machine.active_slots(), DRUM_SLOTS);
        let mut warmup = vec![0.0f32; 3_000];
        machine.render(0, &mut warmup);

        machine.trigger(DrumHit::new(DrumVoice::Kick, 3_000, 1.0));
        assert_eq!(machine.triggers(), (DRUM_SLOTS + 1) as u64);
        assert_eq!(machine.voice_steals(), 1);
        assert_eq!(machine.active_slots(), DRUM_SLOTS);

        let victim = (0..DRUM_SLOTS)
            .find(|index| machine.debug_slot(*index).is_some_and(|slot| slot.3 > 0))
            .expect("池满的一记必须造成一个淡出中的槽位");
        assert_eq!(
            victim, 8,
            "被窃取的必须是电平最低且最早触发的槽位（槽 8 = 第一记闭镲）"
        );
        let victim_slot = machine.debug_slot(victim).expect("刚查过");
        assert_eq!(
            victim_slot.1, VOICE_CLOSED_HAT,
            "淡出中的槽位仍在发旧音色（闭镲）"
        );
    }

    /// 量什么：`steal_fade_frames` 的默认值是否等于 [ARCH-RT-004] 的
    /// 3 ms @48 kHz（单位：帧）。
    #[test]
    fn the_default_steal_fade_is_the_three_millisecond_constant() {
        assert_eq!(steal_fade_frames_for(48_000.0), 144);
        assert_eq!(steal_fade_frames_for(96_000.0), 288);
        assert_eq!(steal_fade_frames_for(0.0), 3); // 钳到 MIN_SAMPLE_RATE = 1000 ⇒ 3 帧
        assert_eq!(steal_fade_frames_for(f32::NAN), 3);
        assert_eq!(steal_fade_frames_for(f32::INFINITY), 3);
        let machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
        assert_eq!(machine.steal_fade_frames(), 144);
    }

    /// 量什么：触发闭镲时在响的开镲是否进入淡出（单位：choke 次数 + 枚举值）。
    #[test]
    fn a_closed_hat_chokes_every_ringing_open_hat() {
        let mut machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
        machine.trigger(DrumHit::new(DrumVoice::OpenHat, 0, 1.0));
        machine.trigger(DrumHit::new(DrumVoice::OpenHat, 0, 1.0));
        assert_eq!(machine.active_slots(), 2);
        assert_eq!(machine.hat_chokes(), 0);
        machine.trigger(DrumHit::new(DrumVoice::ClosedHat, 0, 1.0));
        assert_eq!(machine.hat_chokes(), 2, "两记开镲都必须被 choke");
        let open_still_ringing = (0..DRUM_SLOTS)
            .filter(|index| {
                machine
                    .debug_slot(*index)
                    .is_some_and(|slot| slot.0 && slot.1 == VOICE_OPEN_HAT && slot.3 == 0)
            })
            .count();
        assert_eq!(open_still_ringing, 0, "choke 之后不该有还在自由衰减的开镲");
        // 反向不成立：开镲不 choke 闭镲。
        let mut machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
        machine.trigger(DrumHit::new(DrumVoice::ClosedHat, 0, 1.0));
        machine.trigger(DrumHit::new(DrumVoice::OpenHat, 0, 1.0));
        assert_eq!(machine.hat_chokes(), 0);
    }

    /// 量什么：一击走完之后 `retire_finished` 是否释放槽位（单位：占用槽位数）。
    #[test]
    fn a_finished_hit_frees_its_slot() {
        let mut machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
        machine.trigger(DrumHit::new(DrumVoice::ClosedHat, 0, 1.0));
        assert_eq!(machine.active_slots(), 1);
        // 闭镲默认 45 ms；跑到 1 s 之后必定走完（−80 dB 由 Adsr 归零）。
        let mut out = vec![0.0f32; 48_000];
        machine.render(0, &mut out);
        machine.retire_finished(48_000);
        assert_eq!(machine.active_slots(), 0, "走完的一击必须被回收");
        assert_eq!(machine.triggers(), 1, "回收不改触发计数");
    }

    /// 量什么：`HAT_RATIOS` 的 6 个比值是否满足模块文档声明的两条性质
    /// （单位：无量纲比值）。
    ///
    /// 1. 两两不同（容差 `1e-3`）；
    /// 2. **任意两个比值的商都不是 `1..=8` 的整数**（容差 `5e-3`）⇒ 6 条方波
    ///    没有共同基频。断言消息里打印实测的**最小**整数偏离，供读者判断余量。
    #[test]
    fn the_hat_ratios_are_mutually_inharmonic() {
        let ratios = voice::HAT_RATIOS;
        assert_eq!(ratios.len(), voice::HAT_PARTIALS);
        let mut worst = f32::INFINITY;
        for (i, first) in ratios.iter().enumerate() {
            for (j, second) in ratios.iter().enumerate().skip(i + 1) {
                assert!(
                    (first - second).abs() > 1.0e-3,
                    "比值 {i} 与 {j} 相同：{first} / {second}"
                );
                let quotient = first / second;
                for multiple in 1..=8u32 {
                    #[allow(clippy::cast_precision_loss)]
                    let gap = (quotient - multiple as f32).abs();
                    worst = worst.min(gap);
                    assert!(
                        gap > 5.0e-3,
                        "比值 {i}:{j} 的商 {quotient} 落在整数 {multiple} 上（差 {gap}）\
                         ⇒ 两条方波有共同基频"
                    );
                }
            }
        }
        assert!(
            worst > 0.05,
            "实测最小整数偏离只有 {worst} —— 余量比声明的小，文档要改"
        );
    }

    /// 量什么：`set_params` 在淡出中的槽位上**不得**取消淡出
    /// （单位：淡出剩余帧数）。
    ///
    /// 这条钉住的是一个曾经的实现陷阱：`reconfigure` 会把 `release_s` 覆盖回
    /// 正常释放 ⇒ 淡出被取消 ⇒ 窃取处出现样本跃变。
    #[test]
    fn set_params_during_a_fade_does_not_cancel_the_fade() {
        let mut machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
        for index in 0..=DRUM_SLOTS {
            machine.trigger(DrumHit::new(DrumVoice::Snare, index as u64, 1.0));
        }
        let fading_before = (0..DRUM_SLOTS)
            .filter_map(|index| machine.debug_slot(index))
            .filter(|slot| slot.3 > 0)
            .count();
        assert!(fading_before > 0, "夹具必须至少造出一个淡出中的槽位");
        machine.set_params(DrumKitParams::DEFAULT);
        let fading_after = (0..DRUM_SLOTS)
            .filter_map(|index| machine.debug_slot(index))
            .filter(|slot| slot.3 > 0)
            .count();
        assert_eq!(fading_before, fading_after, "set_params 取消了淡出");
    }

    /// 量什么：`reset()` **之后**再处理一段的输出，与一台"用同样参数新建"的实例在
    /// 同一串触发、同一段位置上是否**逐位相同**（单位：样本值的位模式，逐位比较）。
    ///
    /// 为什么噪声播种计数是**音频状态**：每一击的白噪声种子由"音色号 × 播种计数"
    /// 推出（[`DrumMachine::trigger`] 的 `trigger_seed`），它决定这一击听到的是
    /// **哪一段**噪声。它没有 getter、也不是诊断读数 —— 诊断计数器
    /// （`triggers` / `voice_steals` / `hat_chokes` / `sounding_slot_frames`）
    /// 不改变音频，本字段改变音频。
    ///
    /// 缺口（本判据在修复前**实测变红**）：`reset()` 只清槽位池、不清播种计数
    /// ⇒ 引擎在定位（`seek` 的接入点就是 `reset`）之后重放同一串鼓击，得到的是
    /// **与从头渲染不同**的噪声 ⇒ 同一份工程两次渲染不逐位相同 [ARCH-DET-001]。
    ///
    /// 怎么变红：把 `reset` 改回"只清槽位池"（即撤掉播种计数的归零）。
    #[test]
    fn reset_reproduces_a_freshly_built_machine_bit_for_bit() {
        const FRAMES: usize = 512;
        // 覆盖四条音色：底鼓（音高包络、无噪声）、军鼓（噪声）、闭镲/开镲（噪声）、
        // 拍手（多 onset）。开镲与闭镲相邻 ⇒ choke 路径也在窗口里。
        const HITS: [(DrumVoice, u64); 6] = [
            (DrumVoice::Kick, 0),
            (DrumVoice::Snare, 300),
            (DrumVoice::ClosedHat, 500),
            (DrumVoice::OpenHat, 700),
            (DrumVoice::Clap, 900),
            (DrumVoice::Kick, 1_100),
        ];

        /// 逐击触发并渲染，把全部块拼起来（渲染位置与触发位置一致）。
        fn play(machine: &mut DrumMachine<DRUM_SLOTS>, hits: [(DrumVoice, u64); 6]) -> Vec<f32> {
            let mut out = Vec::new();
            for (voice, start) in hits {
                machine.trigger(DrumHit::new(voice, start, 0.9));
                let mut block = vec![0.0f32; FRAMES];
                machine.render(start, &mut block);
                out.extend_from_slice(&block);
            }
            out
        }

        let mut fresh = DrumMachine::<DRUM_SLOTS>::new(48_000);
        fresh.set_params(DrumKitParams::DEFAULT);
        let expected = play(&mut fresh, HITS);

        // 用过一段历史（播种计数因此前进），再复位。
        let mut used = DrumMachine::<DRUM_SLOTS>::new(48_000);
        used.set_params(DrumKitParams::DEFAULT);
        let _ = play(&mut used, HITS);
        used.reset();
        let actual = play(&mut used, HITS);

        assert_eq!(actual.len(), expected.len(), "夹具长度");
        assert!(
            actual.iter().any(|sample| *sample != 0.0),
            "夹具必须真的出声（否则逐位相等是空转）"
        );
        for (index, (got, want)) in actual.iter().zip(expected.iter()).enumerate() {
            assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "复位后的输出与全新实例在第 {index} 个样本上不同：{got} vs {want}"
            );
        }
    }

    /// 量什么：`DrumVoice::from_u8` 与 `as_u8` 是否互为逆（越界返回 `None`）。
    #[test]
    fn the_voice_discriminants_round_trip() {
        for (index, voice) in DrumVoice::ALL.iter().enumerate() {
            assert_eq!(usize::from(voice.as_u8()), index);
            assert_eq!(DrumVoice::from_u8(voice.as_u8()), Some(*voice));
            assert!(!voice.name().is_empty());
        }
        assert_eq!(DrumVoice::from_u8(DrumVoice::ALL.len() as u8), None);
        assert_eq!(DrumVoice::from_u8(u8::MAX), None);
    }
}
