//! 实时侧**参数目标表**：`EngineEvent::SetParam` → 逐样本平滑增益。
//! [ARCH-DSP-001, ARCH-RT-001, ROAD-M2-007]
//!
//! ## 0. 本模块补的是哪一条已登记的缺口
//!
//! 在接入本模块之前，音频线程对 [`crate::ring::EngineEvent::SetParam`] **只计数**
//! （`EngineStats::events_applied`），**不改任何 DSP 状态**。那条缺口由引擎自己的
//! 台账登记为一条实现缺口（`docs/ledger/gate-rt-zero-alloc-notes.md` 的 needs N1：
//! "实时侧的参数目标表 …… `SetParam` 目前只计数、不改 DSP ⇒ '自动化真的改变了声音'
//! 未被覆盖"，建议处置逐字为"由后续线做'参数槽 → DSP 系数'映射；届时 ⑤ 的判据可加
//! '输出随自动化变化'的断言"）。同一件事也写在 `docs/ledger/engine-rt-notes.md`
//! 的边界清单里（`EngineEvent` 的 `SetParam`／`NoteOn`／`NoteOff`／`Transport`
//! 当时"只计数，未接渲染"；其中 `Transport` 已由 `line/transport-engine` 接通）。
//!
//! 本模块就是那张**参数目标表**：把"参数地址"映射到一件音频线程可执行的 DSP 系数，
//! 并让该系数按规范要求**平滑**过渡。
//!
//! ### 0.1 本票（`line/engine-9`）补的是同一张表的**第二个槽位**
//!
//! **先量（本票当场跑，命令与读数逐字）**：`grep -rn 'use yeban_dsp' crates/yeban-engine/src`
//! 给 **32** 行（其中多数是**注释里的**引用）；把量法收窄成"语句以 `use` 或
//! `pub use` 起首"之后是 **20** 行真实引用，覆盖 **15** 个 `yeban_dsp` 模块名。
//! ⚠ 本票的第一次提交信息把这一条写成"13 行" —— 那个数字**没有出处**，是错的；
//! 这里按"先量后做"补上正确的量法与读数（该提交未改写，见交付报告）。
//!
//! `line/engine-6` 只开了**逐轨输出增益乘子**；主总线上的同一条路径（`scale_bus`）
//! **没有**槽位，因此落在主总线上的 `SetParam` 一律记进 `unmapped` ——
//! 一条**被计数但不生效**的事件。规范原文要求的是"**所有**瞬变自动化事件经过单极点
//! 低通滤波"（§1 的第一条），没有把主总线排除在外 ⇒ 那是"自动化没有完整施加"。
//! 本票把主总线的增益槽位（[`MASTER_GAIN_SLOT`]，见 §2.3）补上；
//! 逐轨槽位的行为、地址空间与读数**一字未改**。
//!
//! ## 1. 规范出处（原文引用）
//!
//! | 出处 | 原文 |
//! | :--- | :--- |
//! | `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §4.1 第 2 条 | "**参数自动化平滑滤波**：所有瞬变自动化事件经过单极点低通滤波（$y[n] = (1 - \alpha) x[n] + \alpha y[n-1]$，时间常数 $\tau \approx 5\text{ms}$），根除阶跃断崖引起的咔嗒杂音" |
//! | `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3.2 | "**内部 DSP 拓扑调度**" 的 1.00 ms 预算；`SetParam` 是 `[ROAD-M2-007]` 的批量无锁通道 |
//!
//! 公式的**唯一实现**住在 `yeban-dsp`（`yeban_dsp::smoothing::ParamSmoother`，
//! 本 crate 之前**零消费者** —— `crate::lib` 的边界清单把它列为"无消费者"）。
//! 本模块只把那个器件接到实时路径上，**不**定义第二份单极点低通。
//!
//! ## 2. 参数槽的词汇（⚠ 这是**临时形状**，与 `ToneParams` / `insert` / `drums` 同族）
//!
//! [`crate::ring::ParamAddress`] 只有两个字段（`entity` = 实体、`slot` = 该实体内部的
//! 参数槽位），而"槽位"的词汇在模型层**不存在**
//! （`DeviceDefinition::params` 是字符串键值对；这是
//! `docs/ledger/engine-mix-notes.md` §8.2 的 **N5**）。因此本模块定义**两个**槽位：
//!
//! | 地址 | 含义 | 单位与值域 | 引入 |
//! | :--- | :--- | :--- | :--- |
//! | `entity` = 一条**音轨**（不是主总线）、`slot = ` [`TRACK_GAIN_SLOT`] | 该轨的**输出增益乘子**，作用在快照音量（构造期烧进音符增益的那一份）**之上** | 线性，`0.0 …` 有限值；默认 `1.0` = 恒等 | `line/engine-6` |
//! | `entity` = **主总线**、`slot = ` [`MASTER_GAIN_SLOT`] | **整条母线输出**的增益乘子，作用在快照主总线推子（`EngineSnapshot::master_gain`）**之上** | 线性，`0.0 …` 有限值；默认 `1.0` = 恒等 | 本票（见 §2.3） |
//!
//! 槽位号**与实体**必须同时匹配：一条轨只有 [`TRACK_GAIN_SLOT`]、主总线只有
//! [`MASTER_GAIN_SLOT`] ⇒ 两个号**不通用**（互相串用一律未映射，见下面的清单）。
//!
//! `1.0` 是**恒等**：既不放大也不衰减。因此"没有收到任何 `SetParam`"与
//! "收到 `1.0`"在音频上同解（后者走恒等快路径，一位不动）。
//!
//! ⛔ **本模块不接受**的地址（每一个都被**计数**，不静默）：
//!
//! 1. 槽位号与实体**不匹配**：主总线带 [`TRACK_GAIN_SLOT`]、或一条轨带
//!    [`MASTER_GAIN_SLOT`]。多定义几个"看起来像旋钮"的槽位而没有对应的 DSP 消费点，
//!    就是本仓库叫的"说假话"；把已定义的两个槽位互相串用是同一类错；
//! 2. 任何**别的**槽位号（`0` 与 `1` 之外）：本模块只定义了两个槽位；
//! 3. 值非有限（`NaN`／`±inf`）或**负数**：增益乘子为负是"反相"，
//!    而本槽位的语义里没有它 ⇒ **忽略该事件**（目标值保持不变）并计数。
//!    ⛔ 刻意**不**把非法值钳成 `0.0`：那会把一个坏参数变成一次**静音**，
//!    是"听得出、读不出"的行为反转。
//! 4. 槽位表已满：见 §4。
//!
//! ### 2.1 为什么是**乘子**而不是绝对音量
//!
//! 静态音量（`TrackParams::volume_db`）在**构造期**就已经折进每个音符的增益
//! （`crate::snapshot::project_schedules` 的 `track_gain`）。音频线程看到的逐轨信号
//! 已经带着那一份增益 ⇒ 运行期**不可能**在不改变既有输出的前提下把它"换掉"。
//! 因此本槽位的语义是**叠加在快照音量之上的乘子**：模型层负责把
//! [`yeban_model::AutomationTarget::TrackVolume`] 的绝对值折成它相对于**当前静态值**
//! 的比值（`crate::ring` 的既有契约原话是"已归一化 / 已是目标域值，
//! 由模型层负责换域"）。⇒ 该比值换算登记为 **needs**（见 §7），本模块不发明它。
//!
//! ### 2.2 施加位置：**声源之后、插入链之前**
//!
//! 位置与"在构造期把该轨音量设成另一个值"**同解**：静态音量也是在插入链**之前**
//! 起作用的（`Synthesizer`／鼓机把 `ScheduledNote::gain` 烧进声部）。
//! 放在插入链**之后**会让压缩器看到的是**未自动化**的电平，
//! 那是另一种（也不好听的）行为 ⇒ 本模块刻意选择前者。
//!
//! ### 2.3 主总线槽位（本票）：**推子之上的乘子**，位置与推子相同
//!
//! 主总线增益的施加点在 `crate::rt` 的步骤 3a''：逐轨汇流与节拍器**之后**、
//! 母线限制器**之前**，由 `scale_bus` 做一次乘。本槽位与它**同一个位置**，
//! 语义是**乘在它之上**（与逐轨槽位"乘在快照音量之上"同一条口径，见 §2.1）：
//! `EngineSnapshot::master_gain` 仍然是"静态推子"的唯一事实源，本槽位只表达
//! "自动化相对它的比值"。⇒ needs §7 的 **P1**（绝对值 → 乘子的换算）对两个槽位
//! 是同一件事，由控制侧做。
//!
//! 为什么与推子同一个位置，而不是另开一条路径：推子是**整条母线输出**的音量
//! （含节拍器 —— 位置说明在 `crate::rt` 的 3a'），自动化必须与它同口径，
//! 否则"总线音量"会有两种含义。乘法的顺序也因此是**固定**的：先静态推子
//! （`scale_bus`）、后本乘子 —— 与逐轨槽位"先构造期静态音量、后运行期乘子"一致。
//! ⚠ 浮点乘法不满足结合律 ⇒ 顺序不是无关紧要的细节，`crate::rt` 的两个调用点
//! 的先后就是契约。
//!
//! 施加形状与逐轨槽位**不同**（这是主总线的性质，不是实现偷懒）：主总线是**合成的**，
//! 它没有声源、也不进声相表（`crate::rt` 步骤 2c 的 `*id == master` 分支跳过它）
//! ⇒ 本槽位不参与逐轨循环，而是对整个**立体声块**每帧一次：一次单极点低通
//! （推进一个增益值）＋ 左右两条声道**各一次**乘。⚠ 立体声联动是**强制**的：
//! 两条声道用**同一个**增益值（`process()` 每帧只推进一次），不允许左右各自平滑 ——
//! 那会把一个单声道参数变成两个，并让母线出现左右不同的增益轨迹。
//!
//! ## 3. 实时分类（[ADR-0001 D32]）
//!
//! - **构造期**（[`ParamTable::new`]）：建 [`PARAM_SLOTS`] 个逐轨 [`ParamSmoother`]
//!   ＋ **一个**主总线平滑器；`ParamSmoother::new` 里的 `exp` 属**超越函数类**，
//!   只在这里跑一次；
//! - **快照边界**（每个修订一次，[`ParamTable::set_sample_rate`]）：采样率变化才重算
//!   `α`（同样含 `exp`）。与音轨声相表（`cos`／`sin`）是同一条纪律；
//! - **事件边界**（每个 `SetParam` 一次，[`ParamTable::accept`]）：整数比较、
//!   线性搜索（至多 [`PARAM_SLOTS`] 项）、标量赋值 —— **IEEE 精确类**，零分配；
//! - **逐样本**（[`ParamTable::apply`] 与 [`ParamTable::apply_master`]）：每个样本
//!   一次乘加（`process`），以及一次（逐轨）或两次（主总线两条声道）乘 ——
//!   **IEEE 精确类**，零分配、零锁、零阻塞 I/O、零日志 [MUST-GATE-001]。
//!
//! ## 4. 槽位表：惰性分配、只增不减、容量有读数
//!
//! 槽位在**第一个被接受的** `SetParam` 上按实体惰性分配（线性搜索；同一实体永远
//! 命中同一个槽位）。容量是 [`PARAM_SLOTS`]（= 声部池的轨道上限，同一个事实源）。
//! 表**不会**释放槽位：一条轨消失之后它的槽位留着（代价是至多 [`PARAM_SLOTS`] 个
//! `f32` 状态），换来的是"事件永远能命中它自己那一格"这条确定性。
//! 容量用尽时事件被**计数**（[`ParamTable::capacity_drops`]）而不是静默丢弃。
//!
//! ## 5. 恒等快路径（"接线不改变既有输出"的机械形式）
//!
//! [`ParamTable::apply`] 在"该实体没有槽位"或"该槽位已吸附到 `1.0`"时
//! **整段跳过**（一个样本都不碰）⇒ 没有 `SetParam` 的工程与接线前**逐位相同**；
//! [`ParamTable::apply_master`] 在"主总线槽位从未收到过被接受的目标"或
//! "它已吸附到 `1.0`"时**整段跳过**（同一个口径）。
//! 这不是近似：`x · 1.0` 对有限值、`±0.0` 与次正规数都逐位恒等，但跳过更强 ——
//! 它对 `NaN` 也恒等（`NaN · 1.0` 会静默 NaN，虽然仍是 `NaN`）。
//!
//! ## 6. 这一层**没有**做的事（与读数一起读）
//!
//! - **没有**参数曲线的插值：控制侧给什么值，本表就把目标设成什么值。
//!   [`ParamSmoother`] 只负责"从当前值平滑走到目标"，不负责"目标本身怎么随时间变"
//!   （那是 `yeban_model` 的 `automation_value_at` 的职责，`tests/rt_zero_alloc.rs`
//!   的场景 ⑤ 已经在控制侧调它）；
//! - **没有**把参数写回快照或模型：参数目标表是**挥发性**的音频线程状态
//!   （[MODEL-ISO-001] 的第二层语义）；
//! - **没有**声道声相（`pan`）与插入器件参数的槽位：本票只把**增益**这一条路径
//!   补到主总线（见 §7 的 P4）。
//!
//! ## 7. needs（交给模型线／集成者）
//!
//! | # | needs | 为什么 |
//! | :-- | :--- | :--- |
//! | P1 | **绝对值 → 乘子的换算**（`AutomationTarget::TrackVolume` 的 dB 值 → 本槽位的线性乘子）由谁做 | 本表刻意只收乘子（§2.1）；换算需要一个"当前静态值"的读点，那是模型／app 侧的事实。**两个槽位同一件事**（音轨与主总线） |
//! | P2 | **槽位词汇的模型层规范**（`DeviceDefinition::params` 的字符串键 → 稳定索引） | 与 `engine-mix-notes.md` §8.2 的 **N5** 同一件事；它落下之后本模块的槽位表应当改为直读模型字段 |
//! | P3 | ~~**主总线增益的自动化槽位**~~ ⇒ **已由本票关闭**（[`MASTER_GAIN_SLOT`]，见 §2.3）。**残留**：主总线的绝对值 → 乘子的换算仍是 P1 | 关闭的理由：`crate::rt` 已经有一条明确的总线增益路径（`scale_bus`），本槽位只是同位置的第二个乘子，语义不需要新裁决 |
//! | P4 | **声相（`pan`）与插入器件参数的自动化槽位** | 本票只开了两个**增益**槽位。声相的施加点在声相表（构造期 `cos`／`sin`），把它做成逐样本需要"声相律在事件边界求值"的新裁决（超越函数类能不能在事件边界跑）；器件参数的词汇又卡在 P2 |

use yeban_dsp::smoothing::{DEFAULT_TIME_CONSTANT_S, ParamSmoother};
use yeban_model::EntityId;

use crate::ring::ParamAddress;

/// 音轨**输出增益乘子**的槽位号。
///
/// 见模块文档 §2：`entity` 必须是一条音轨（不是主总线），值必须有限且非负。
pub const TRACK_GAIN_SLOT: u16 = 0;

/// **主总线输出增益乘子**的槽位号（本票新增，见模块文档 §2.3）。
///
/// 见模块文档 §2：`entity` 必须是**主总线**（不是一条普通音轨），值必须有限且非负。
/// 它与 [`TRACK_GAIN_SLOT`] **不通用** —— 串用一律未映射。
pub const MASTER_GAIN_SLOT: u16 = 1;

/// **逐轨声相左增益**的槽位号（裁决 P4=(b)，自动化接线的最后一块）。
///
/// 语义：值是**线性增益**（不是声相位置）—— 控制侧用 [`crate::mixer::pan_gains`]
/// 把声相位置折成 `(左, 右)`，再把这两个**绝对**增益发进来（"替换"口径；
/// ⛔ 不做分量相除：硬左时右增益恰为 `0`，相除会出 `0/0`）。
///
/// ⚠ **本表（[`ParamTable`]）不消费这两个槽位**：[`ParamTable::accept`] 仍然只认
/// [`TRACK_GAIN_SLOT`]／[`MASTER_GAIN_SLOT`]，其余一律 [`ParamOutcome::Unmapped`]。
/// 声相事件由 `crate::rt` 的**事件循环先截走**，写进那条独立的逐轨声相平滑对
/// （`rt.rs` 的 `armed_pan_l`／`armed_pan_r`）。这条分流有判据钉住。
pub const TRACK_PAN_LEFT_SLOT: u16 = 2;

/// **逐轨声相右增益**的槽位号（与 [`TRACK_PAN_LEFT_SLOT`] 成对，语义相同）。
pub const TRACK_PAN_RIGHT_SLOT: u16 = 3;

/// 参数目标表的槽位数：与声部池的轨道上限**同一个事实源**。
///
/// 理由与 `PDC_SLOTS` 相同：不是"刚好够用"，而是"不为同一件事造第二个上限"。
pub const PARAM_SLOTS: usize = crate::synth::MAX_TRACK_SLOTS;

/// 一条 `SetParam` 事件的裁决结果。
///
/// 它是"事件有没有真的作用到 DSP"的**可读归因**：调用方（`crate::rt`）按变体累加
/// 各自的读数，于是"事件到了但什么都没发生"永远是**可见**的。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamOutcome {
    /// 被接受：该轨的增益乘子目标已更新（值可能在平滑中）。
    Accepted,
    /// 值非法（非有限 / 负数）⇒ 目标值保持不变。
    Rejected,
    /// 地址在本表里没有映射（槽位号不是 [`TRACK_GAIN_SLOT`]／实体是主总线）。
    Unmapped,
    /// 槽位表已满，没能为该实体分配槽位。
    Full,
}

/// 实时侧的**参数目标表**：实体 → 一个逐样本平滑的增益乘子。
///
/// 全部状态都是定长数组与标量（构造期一次成型），
/// 因此 `accept` / `apply` / `apply_master` 在音频线程上
/// **零分配、零锁、零阻塞 I/O、零日志**。
pub struct ParamTable {
    /// 已分配的槽位：`(实体, 是否已收到过被接受的目标)`。
    ///
    /// 只有前 `len` 项是活的；`bool` 为假表示"槽位已建、还没有收到过目标"
    /// （此时增益恒为 `1.0`，[`Self::apply`] 走恒等快路径）。
    slots: [(EntityId, bool); PARAM_SLOTS],
    /// 每个槽位的平滑器。初值 `1.0`（恒等）。
    gains: [ParamSmoother; PARAM_SLOTS],
    /// 已分配的槽位数（只增不减，见模块文档 §4）。
    len: usize,
    /// **主总线**增益乘子的平滑器（与逐轨槽位分开的一格，见模块文档 §2.3）。
    master: ParamSmoother,
    /// 主总线槽位是否收到过**被接受的**目标（假 = [`Self::apply_master`] 走恒等快路径）。
    master_armed: bool,
    /// 当前采样率（Hz），惰性分配槽位时用来建平滑器。
    sample_rate: f32,
    /// 累计被增益乘过的帧数（**见证**：参数真的改变了音频）。
    gain_frames: u64,
    /// 累计被**主总线**增益乘子乘过的帧数（与 [`Self::gain_frames`] 分开记账）。
    master_frames: u64,
    /// 累计被忽略的非法值事件数。
    rejections: u64,
    /// 累计未映射的地址事件数。
    unmapped: u64,
    /// 累计因槽位表满而未分配的事件数。
    capacity_drops: u64,
}

impl ParamTable {
    /// 构造：按给定采样率建满 [`PARAM_SLOTS`] 个逐轨平滑器 ＋ 一个主总线平滑器
    /// （**构造期**，允许 `exp`）。
    ///
    /// 全部槽位的初值是 `1.0`／目标 `1.0` ⇒ 本表在收到任何事件之前对音频**零影响**。
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let mut gains = [ParamSmoother::new(sample_rate, DEFAULT_TIME_CONSTANT_S); PARAM_SLOTS];
        for gain in &mut gains {
            gain.snap_to(1.0);
        }
        let mut master = ParamSmoother::new(sample_rate, DEFAULT_TIME_CONSTANT_S);
        master.snap_to(1.0);
        Self {
            slots: [(EntityId::default(), false); PARAM_SLOTS],
            gains,
            len: 0,
            master,
            master_armed: false,
            sample_rate,
            gain_frames: 0,
            master_frames: 0,
            rejections: 0,
            unmapped: 0,
            capacity_drops: 0,
        }
    }

    /// 快照边界：把采样率同步给**全部**平滑器（`α` 随之重算，含 `exp`）。
    ///
    /// 当前值与目标都**不动** ⇒ 换采样率不会让一个正在平滑的增益跳变。
    ///
    /// ⚠ **主总线那一格也必须跟随**，哪怕它还没有被武装过（判据 ⑧）。曾经的写法是
    /// "只在 `master_armed` 时跟随"，理由是"没收到过目标时当前值与目标都恒为 `1.0`，
    /// 没有东西会因为 `α` 变旧而漂移" —— 那条理由**只对此刻**成立：`α` 是**持久状态**，
    /// 武装发生在**快照边界之后**的事件边界上，而 `set_sample_rate` 在采样率相同时
    /// 直接返回 ⇒ 旧 `α` 从此再也没有被修正的入口，"先换采样率、后武装"会得到一条
    /// 按 `旧率 ÷ 新率` 缩放过的斜坡（48 kHz → 96 kHz 时是 10 ms 而不是 5 ms）。
    /// 逐轨槽位从来没有这个不对称：它们无条件全量转发。现在两者同款。
    ///
    /// 代价是每次**真的**换采样率时多一次 `exp`。它落在快照边界（每个修订至多一次），
    /// 不在逐样本路径上，且 `ParamSmoother::set_sample_rate` 在采样率相同时直接返回
    /// ⇒ 稳态下**一次都不跑**（[ADR-0001 D32] 的超越函数类纪律）。
    /// 零分配、零锁、零阻塞 I/O、零日志 [MUST-GATE-001]。
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        if sample_rate == self.sample_rate {
            return;
        }
        self.sample_rate = sample_rate;
        for gain in &mut self.gains[..self.len] {
            gain.set_sample_rate(sample_rate);
        }
        self.master.set_sample_rate(sample_rate);
    }

    /// 事件边界：处理一条 `SetParam`（见模块文档 §2 的裁决表）。
    ///
    /// `master` 是当前快照的主总线身份：`entity` 等于它 ⇒ 走**主总线**槽位
    /// （[`MASTER_GAIN_SLOT`]），否则走**逐轨**槽位（[`TRACK_GAIN_SLOT`]）。
    pub fn accept(&mut self, address: ParamAddress, value: f32, master: EntityId) -> ParamOutcome {
        let expected_slot = if address.entity == master {
            MASTER_GAIN_SLOT
        } else {
            TRACK_GAIN_SLOT
        };
        if address.slot != expected_slot {
            self.unmapped = self.unmapped.wrapping_add(1);
            return ParamOutcome::Unmapped;
        }
        if !value.is_finite() || value < 0.0 {
            self.rejections = self.rejections.wrapping_add(1);
            return ParamOutcome::Rejected;
        }
        if address.entity == master {
            // 主总线只有**一格**，不是逐轨表里的一项 ⇒ 不占逐轨槽位、
            // 也不受 §4 的容量约束（它永远装得下）。
            self.master_armed = true;
            self.master.set_target(value);
            return ParamOutcome::Accepted;
        }
        let index = match self.index_of(address.entity) {
            Some(index) => index,
            None => {
                if self.len >= PARAM_SLOTS {
                    self.capacity_drops = self.capacity_drops.wrapping_add(1);
                    return ParamOutcome::Full;
                }
                let index = self.len;
                self.slots[index] = (address.entity, true);
                self.gains[index] = ParamSmoother::new(self.sample_rate, DEFAULT_TIME_CONSTANT_S);
                // 新槽位从**恒等**起步：第一个目标值因此是一段平滑的进入，
                // 而不是一次从 `0.0` 到目标值的淡入（那会是一次听得见的误衰减）。
                self.gains[index].snap_to(1.0);
                self.len += 1;
                index
            }
        };
        self.slots[index].1 = true;
        self.gains[index].set_target(value);
        ParamOutcome::Accepted
    }

    /// 逐样本：把本实体的增益乘子乘进 `out`，返回**被乘过的帧数**。
    ///
    /// 没有槽位、或槽位已吸附到 `1.0` ⇒ 一个样本都不碰（返回 `0`），
    /// 见模块文档 §5 的恒等快路径。
    pub fn apply(&mut self, entity: EntityId, out: &mut [f32]) -> u64 {
        let Some(index) = self.index_of(entity) else {
            return 0;
        };
        let gain = &mut self.gains[index];
        if gain.is_settled() && gain.value() == 1.0 {
            return 0;
        }
        for sample in out.iter_mut() {
            *sample *= gain.process();
        }
        let frames = out.len() as u64;
        self.gain_frames = self.gain_frames.wrapping_add(frames);
        frames
    }

    /// 逐样本：把**主总线**增益乘子乘进左右两条声道，返回**被乘过的帧数**。
    ///
    /// 立体声联动：两条声道用**同一个**增益值（`process` 每帧只推进一次），
    /// 见模块文档 §2.3。长度取两条切片的**较短者**（防御式；本 crate 的音频块
    /// 两条声道恒等长）。
    ///
    /// 主总线槽位从未收到过被接受的目标、或它已吸附到 `1.0` ⇒ 一个样本都不碰
    /// （返回 `0`），见模块文档 §5 的恒等快路径。
    pub fn apply_master(&mut self, left: &mut [f32], right: &mut [f32]) -> u64 {
        if !self.master_armed {
            return 0;
        }
        let gain = &mut self.master;
        if gain.is_settled() && gain.value() == 1.0 {
            return 0;
        }
        let frames = left.len().min(right.len());
        for (l, r) in left[..frames].iter_mut().zip(right[..frames].iter_mut()) {
            let value = gain.process();
            *l *= value;
            *r *= value;
        }
        self.master_frames = self.master_frames.wrapping_add(frames as u64);
        frames as u64
    }

    /// 线性搜索实体的槽位（至多 [`PARAM_SLOTS`] 项，无分配）。
    fn index_of(&self, entity: EntityId) -> Option<usize> {
        self.slots[..self.len]
            .iter()
            .position(|(id, _)| *id == entity)
    }

    /// 已分配的槽位数（判据读它来证明"事件真的建了槽位"）。
    #[must_use]
    pub const fn slot_count(&self) -> usize {
        self.len
    }

    /// 某实体当前的目标值（`None` = 本表里没有它的槽位）。
    #[must_use]
    pub fn target(&self, entity: EntityId) -> Option<f32> {
        self.index_of(entity)
            .map(|index| self.gains[index].target())
    }

    /// 某实体当前的平滑输出值（`None` = 本表里没有它的槽位）。
    #[must_use]
    pub fn gain(&self, entity: EntityId) -> Option<f32> {
        self.index_of(entity).map(|index| self.gains[index].value())
    }

    /// 累计被增益乘过的帧数（见模块文档 §3 的**见证**口径）。
    #[must_use]
    pub const fn gain_frames(&self) -> u64 {
        self.gain_frames
    }

    /// 累计被忽略的非法值事件数（非有限 / 负数）。
    #[must_use]
    pub const fn rejections(&self) -> u64 {
        self.rejections
    }

    /// 累计未映射的地址事件数（槽位号与实体不匹配，见模块文档 §2）。
    #[must_use]
    pub const fn unmapped(&self) -> u64 {
        self.unmapped
    }

    /// 累计因槽位表满而未分配的事件数。
    #[must_use]
    pub const fn capacity_drops(&self) -> u64 {
        self.capacity_drops
    }

    /// 主总线槽位是否收到过**被接受的**目标（见模块文档 §2.3）。
    ///
    /// `false` ⇒ [`Self::apply_master`] 走恒等快路径（一个样本都不碰）。
    #[must_use]
    pub const fn master_armed(&self) -> bool {
        self.master_armed
    }

    /// 主总线增益乘子的**目标值**（没收到过目标时是恒等值 `1.0`）。
    #[must_use]
    pub const fn master_target(&self) -> f32 {
        self.master.target()
    }

    /// 主总线增益乘子的**当前平滑输出值**（同上口径）。
    #[must_use]
    pub const fn master_gain(&self) -> f32 {
        self.master.value()
    }

    /// 累计被**主总线**增益乘子乘过的帧数（**见证**：主总线上的参数真的改变了音频）。
    ///
    /// 与 [`Self::gain_frames`] **分开**记账：两者回答两个不同的问题
    /// （"有音轨被乘过吗" vs "母线被乘过吗"），合并会让判据失去区分力。
    #[must_use]
    pub const fn master_gain_frames(&self) -> u64 {
        self.master_frames
    }
}

impl Default for ParamTable {
    /// 缺省采样率 `48 kHz`（与引擎其它"还没有快照时"的初值同口径）。
    fn default() -> Self {
        Self::new(48_000.0)
    }
}

impl core::fmt::Debug for ParamTable {
    /// 只打印**可读的读数**（不为 [`PARAM_SLOTS`] 个平滑器刷屏）。
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ParamTable")
            .field("slots", &self.len)
            .field("gain_frames", &self.gain_frames)
            .field("master_armed", &self.master_armed)
            .field("master_gain", &self.master.value())
            .field("master_gain_frames", &self.master_frames)
            .field("rejections", &self.rejections)
            .field("unmapped", &self.unmapped)
            .field("capacity_drops", &self.capacity_drops)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn address(entity: EntityId, slot: u16) -> ParamAddress {
        ParamAddress::new(entity, slot)
    }

    /// 判据 ①：没有事件 ⇒ 表对音频零影响（恒等快路径）。
    #[test]
    fn an_idle_table_never_touches_the_signal() {
        let mut table = ParamTable::new(SR);
        let mut block = [0.25f32, -0.5, 0.0, f32::MIN_POSITIVE];
        let before = block;
        assert_eq!(table.apply(EntityId::new(), &mut block), 0);
        assert_eq!(block, before, "没有槽位时一个样本都不许碰");
        assert_eq!(table.gain_frames(), 0);
        assert_eq!(table.slot_count(), 0);
    }

    /// 判据 ②：被接受的目标会**平滑**逼近（不是瞬时跳变），并且最终逐位相等。
    #[test]
    fn an_accepted_target_is_approached_smoothly_and_settles() {
        let entity = EntityId::new();
        let master = EntityId::new();
        let mut table = ParamTable::new(SR);
        assert_eq!(
            table.accept(address(entity, TRACK_GAIN_SLOT), 0.25, master),
            ParamOutcome::Accepted
        );
        assert_eq!(table.slot_count(), 1);
        // `set_target` 只改目标：**还没有**推进任何样本 ⇒ 输出值仍是 `1.0`。
        assert_eq!(table.target(entity), Some(0.25));
        assert_eq!(table.gain(entity), Some(1.0));
        // 5 ms @48 kHz ≈ 240 样本；吸附门限是 `1e-5`（相对量）⇒ 走完 0.75 的差
        // 需要 `τ · ln(0.75 / 1e-5) ≈ 11.2 τ ≈ 2 700` 个样本。这里跑 12 000 帧。
        let mut block = [1.0f32; 12_000];
        assert_eq!(table.apply(entity, &mut block), 12_000);
        assert_eq!(table.gain(entity), Some(0.25), "平滑器必须吸附到目标");
        assert!(
            block[0] < 1.0 && block[0] > 0.99,
            "第一个样本只许走一小步（实得 {}）",
            block[0]
        );
        assert_eq!(block[11_999], 0.25, "末样本必须逐位等于目标");
    }

    /// 判据 ③：非法值被忽略（目标不变）且被计数；未映射的地址同样被计数。
    #[test]
    fn illegal_values_and_unmapped_addresses_are_counted_not_applied() {
        let entity = EntityId::new();
        let master = EntityId::new();
        let mut table = ParamTable::new(SR);
        assert_eq!(
            table.accept(address(entity, TRACK_GAIN_SLOT), f32::NAN, master),
            ParamOutcome::Rejected
        );
        assert_eq!(
            table.accept(address(entity, TRACK_GAIN_SLOT), -1.0, master),
            ParamOutcome::Rejected
        );
        assert_eq!(table.slot_count(), 0, "被拒的事件不许建槽位");
        assert_eq!(table.rejections(), 2);
        assert_eq!(
            table.accept(address(entity, 1), 0.5, master),
            ParamOutcome::Unmapped
        );
        assert_eq!(
            table.accept(address(master, TRACK_GAIN_SLOT), 0.5, master),
            ParamOutcome::Unmapped
        );
        assert_eq!(table.unmapped(), 2);
        assert_eq!(table.target(entity), None);
    }

    /// 判据 ④：槽位表满时不静默 —— 第 [`PARAM_SLOTS`] + 1 个实体被计数。
    #[test]
    fn a_full_table_counts_instead_of_dropping_silently() {
        let master = EntityId::new();
        let mut table = ParamTable::new(SR);
        for _ in 0..PARAM_SLOTS {
            assert_eq!(
                table.accept(address(EntityId::new(), TRACK_GAIN_SLOT), 0.5, master),
                ParamOutcome::Accepted
            );
        }
        assert_eq!(table.slot_count(), PARAM_SLOTS);
        assert_eq!(
            table.accept(address(EntityId::new(), TRACK_GAIN_SLOT), 0.5, master),
            ParamOutcome::Full
        );
        assert_eq!(table.capacity_drops(), 1);
    }

    /// 判据 ⑤：恒等目标（`1.0`）不碰样本；非恒等目标**逐位**改变样本。
    #[test]
    fn an_identity_target_is_a_bit_exact_skip() {
        let entity = EntityId::new();
        let master = EntityId::new();
        let mut table = ParamTable::new(SR);
        table.accept(address(entity, TRACK_GAIN_SLOT), 1.0, master);
        let mut block = [0.25f32, -0.0, 1.5];
        let before = block;
        assert_eq!(table.apply(entity, &mut block), 0, "1.0 必须整段跳过");
        assert_eq!(block, before);
        table.accept(address(entity, TRACK_GAIN_SLOT), 0.5, master);
        assert_eq!(table.apply(entity, &mut [1.0f32; 4]), 4);
    }

    /// 判据 ⑥：主总线槽位只认 [`MASTER_GAIN_SLOT`] + 主总线实体；
    /// 互相串用与非法值都被**计数**且不建逐轨槽位（模块文档 §2.3）。
    #[test]
    fn the_master_slot_accepts_only_the_master_entity_and_its_own_slot() {
        let track = EntityId::new();
        let master = EntityId::new();
        let mut table = ParamTable::new(SR);
        assert!(!table.master_armed(), "没有事件时主总线槽位未武装");
        assert_eq!(table.master_gain(), 1.0, "未武装时是恒等值");

        // 串用：一条轨带 MASTER_GAIN_SLOT、主总线带 TRACK_GAIN_SLOT。
        assert_eq!(
            table.accept(address(track, MASTER_GAIN_SLOT), 0.5, master),
            ParamOutcome::Unmapped
        );
        assert_eq!(
            table.accept(address(master, TRACK_GAIN_SLOT), 0.5, master),
            ParamOutcome::Unmapped
        );
        // 别的槽位号（`0` 与 `1` 之外）。
        assert_eq!(
            table.accept(address(master, 2), 0.5, master),
            ParamOutcome::Unmapped
        );
        assert_eq!(table.unmapped(), 3);
        assert_eq!(table.slot_count(), 0, "未映射的事件不许建逐轨槽位");
        assert!(!table.master_armed());

        // 主总线自己的地址：被接受，但**不占**逐轨槽位。
        assert_eq!(
            table.accept(address(master, MASTER_GAIN_SLOT), 0.25, master),
            ParamOutcome::Accepted
        );
        assert!(table.master_armed());
        assert_eq!(table.master_target(), 0.25);
        assert_eq!(table.master_gain(), 1.0, "set_target 不许瞬时跳到目标");
        assert_eq!(table.slot_count(), 0, "主总线不占逐轨槽位");

        // 非法值与逐轨槽位同一个口径：忽略、计数、目标不动。
        assert_eq!(
            table.accept(address(master, MASTER_GAIN_SLOT), f32::NAN, master),
            ParamOutcome::Rejected
        );
        assert_eq!(
            table.accept(address(master, MASTER_GAIN_SLOT), -0.5, master),
            ParamOutcome::Rejected
        );
        assert_eq!(table.rejections(), 2);
        assert_eq!(table.master_target(), 0.25);
    }

    /// 判据 ⑦：主总线乘子**逐位**作用到两条声道（立体声联动），并单独记账；
    /// 恒等目标一个样本都不碰。
    #[test]
    fn the_master_multiplier_is_stereo_linked_and_bit_exact() {
        let master = EntityId::new();
        let mut table = ParamTable::new(SR);
        // 未武装 ⇒ 恒等快路径。
        let mut left = [0.5f32, -0.25];
        let mut right = [-0.125f32, 0.75];
        let before = (left, right);
        assert_eq!(table.apply_master(&mut left, &mut right), 0);
        assert_eq!((left, right), before, "未武装时一个样本都不许碰");
        assert_eq!(table.master_gain_frames(), 0);

        // 恒等目标 ⇒ 仍然整段跳过（但槽位已武装）。
        assert_eq!(
            table.accept(address(master, MASTER_GAIN_SLOT), 1.0, master),
            ParamOutcome::Accepted
        );
        assert_eq!(table.apply_master(&mut left, &mut right), 0);
        assert_eq!((left, right), before);
        assert_eq!(table.master_gain_frames(), 0);

        // `0.25` 是 2 的幂 ⇒ 吸附之后逐位精确；两条声道用**同一个**增益。
        assert_eq!(
            table.accept(address(master, MASTER_GAIN_SLOT), 0.25, master),
            ParamOutcome::Accepted
        );
        let mut left = [1.0f32; 12_000];
        let mut right = [1.0f32; 12_000];
        assert_eq!(table.apply_master(&mut left, &mut right), 12_000);
        assert_eq!(table.master_gain(), 0.25, "平滑器必须吸附到目标");
        assert_eq!(table.master_gain_frames(), 12_000);
        assert!(
            left[0] < 1.0 && left[0] > 0.99,
            "第一个帧只许走一小步（实得 {}）",
            left[0]
        );
        assert_eq!(left[11_999], 0.25);
        assert_eq!(right[11_999], 0.25);
        for (index, (l, r)) in left.iter().zip(right.iter()).enumerate() {
            assert_eq!(l, r, "第 {index} 帧：两条声道必须用同一个增益");
        }
    }

    /// 判据 ⑧：**换采样率发生在主总线槽位第一次武装之前**时，那一格也必须用
    /// **新**采样率的 `α` —— 也就是与"一开始就构造在该采样率"的同类表**逐位相同**。
    ///
    /// 它钉住一个**顺序**上的不对称：逐轨槽位在 [`ParamTable::set_sample_rate`] 里
    /// **无条件**跟随（`self.gains[..self.len]` 全量转发），而主总线那一格曾经只在
    /// "**已经**武装过"时才跟随。于是「先换采样率、后武装主总线」会留下一个按
    /// **旧**采样率算出的 `α` ⇒ 同一条 5 ms 的斜坡按 `旧率 ÷ 新率` 被拉长或压短
    /// （48 kHz → 96 kHz 时变成 10 ms），与模块文档 §3 第 2 条"采样率变化才重算 `α`"
    /// 的契约不符 —— 那个 `α` 从此**再也没有**被修正的入口：`set_sample_rate` 在
    /// 采样率相同时直接返回，武装发生在快照边界**之后**。
    ///
    /// 判据是**逐位等号**（两个表在同一批样本上的输出数组），不是容差：
    /// `α` 不同 ⇒ 第一个样本就不同。
    #[test]
    fn a_rate_change_before_the_master_slot_is_armed_leaves_no_stale_alpha() {
        let master = EntityId::new();

        // A：先构造在 48 kHz，**换到** 96 kHz，然后才第一次武装主总线槽位。
        let mut late = ParamTable::new(48_000.0);
        late.set_sample_rate(96_000.0);
        assert!(!late.master_armed(), "还没有事件 ⇒ 主总线槽位未武装");
        assert_eq!(
            late.accept(address(master, MASTER_GAIN_SLOT), 0.25, master),
            ParamOutcome::Accepted
        );

        // B：一开始就构造在 96 kHz（唯一的事实源）。
        let mut fresh = ParamTable::new(96_000.0);
        assert_eq!(
            fresh.accept(address(master, MASTER_GAIN_SLOT), 0.25, master),
            ParamOutcome::Accepted
        );

        // 4 000 帧：96 kHz 上的 5 ms 斜坡（≈ 5 386 样本才吸附）**还没**走完，
        // 而按 48 kHz 的 `α` 走（≈ 2 694 样本）已经吸附 ⇒ 两种情形在这一窗内可分。
        let mut late_left = [1.0f32; 4_000];
        let mut late_right = [1.0f32; 4_000];
        let mut fresh_left = [1.0f32; 4_000];
        let mut fresh_right = [1.0f32; 4_000];
        assert_eq!(late.apply_master(&mut late_left, &mut late_right), 4_000);
        assert_eq!(fresh.apply_master(&mut fresh_left, &mut fresh_right), 4_000);

        assert_eq!(
            late.master_gain(),
            fresh.master_gain(),
            "先换采样率再武装主总线，平滑器的输出值必须与一开始就在该采样率逐位相同"
        );
        assert_eq!(
            late_left, fresh_left,
            "左声道的逐样本轨迹必须逐位相同（α 必须是新采样率的）"
        );
        assert_eq!(late_right, fresh_right, "右声道同上（立体声联动）");
        // 反向见证：两条轨迹**真的**被推进过（否则上面两条等号是空转）。
        assert!(
            fresh_left[3_999] < 1.0 && fresh_left[3_999] > 0.25,
            "96 kHz 的第一窗必须仍在平滑中（实得 {}）",
            fresh_left[3_999]
        );
    }

    /// 判据 ⑨：`0.0` 是**合法**增益（静音），不是非法值。
    ///
    /// 量什么：`ParamTable::accept(…, 0.0, …)` 的返回值、`rejections`（次）与
    /// 目标值（线性幅度）。契约原文是"非有限 / **负**数"这两类才非法 ⇒
    /// `0.0` 必须被接受并真的走到目标。
    ///
    /// 为什么单独立一条：既有判据只喂过 `NaN` / `-0.5` / `1.0` / `0.25` / `-1.0`
    /// ⇒ 把 `value < 0.0` 改 `value <= 0.0` 没有任何判据变红
    /// （本票注入实测：全量 24 个目标全绿）。后果是"把一条轨拉到静音"的
    /// 自动化事件被整条丢弃，推子停在旧值。
    #[test]
    fn a_zero_gain_is_accepted_and_reaches_the_target() {
        let track = EntityId::new();
        let mut table = ParamTable::new(SR);
        assert_eq!(
            table.accept(address(track, TRACK_GAIN_SLOT), 0.0, EntityId::new()),
            ParamOutcome::Accepted,
            "0.0（静音）必须是合法增益 —— 非法的只有非有限与负数"
        );
        assert_eq!(table.rejections(), 0, "0.0 不得被计成非法值");
        assert_eq!(table.target(track), Some(0.0), "目标必须真的是 0.0");
        // 覆盖度：0.0 真的作用到样本上（不是"接受了但没接线"）。
        let mut out = [1.0f32; 12_000];
        assert_eq!(table.apply(track, &mut out), 12_000, "槽位必须真的乘过样本");
        assert_eq!(out[11_999], 0.0, "静音目标必须让样本吸附到 0.0");
    }

    /// 判据：手写 `Debug` 的**形状与读数**必须被钉住（它是诊断面的一部分）。
    ///
    /// `ParamTable` 的 `Debug` 是**手写**的（不是 derive）：它刻意只打印 8 个可读读数，
    /// 不为 [`PARAM_SLOTS`] 个平滑器刷屏。人工排障与 MCP 的引擎状态查询读到的就是这份
    /// 文本，因此**结构体名、字段名、字段值**三者都是契约。
    ///
    /// **量什么**：`format!("{table:?}")` 的文本（单位：字符）。
    /// 注入实测（第四批）：
    /// * `.debug_struct("ParamTable")` → `"ParamTableX"` ⇒ 本判据实测变红；
    /// * `.field("slots", &self.len)` → `&0` ⇒ 本判据实测变红（`slots: 0` vs `slots: 1`）。
    #[test]
    fn the_hand_written_debug_shape_reports_the_readouts() {
        let mut table = ParamTable::new(SR);
        let track = EntityId::new();
        table.accept(address(track, TRACK_GAIN_SLOT), 0.5, EntityId::new());
        let text = format!("{table:?}");
        assert!(
            text.starts_with("ParamTable {"),
            "结构体名是契约（实得 {text}）"
        );
        for field in [
            "slots",
            "gain_frames",
            "master_armed",
            "master_gain",
            "master_gain_frames",
            "rejections",
            "unmapped",
            "capacity_drops",
        ] {
            assert!(
                text.contains(&format!("{field}:")),
                "缺字段 `{field}`（实得 {text}）"
            );
        }
        assert!(
            text.contains("slots: 1"),
            "槽位数必须跟着状态走（1 个在册槽位 ⇒ `slots: 1`，实得 {text}）"
        );

        // R58：等号/包含式的判据必须另有一条 `assert_ne!` 落在**同一个**表达式上 ——
        // 证明 Debug 文本**真的随状态变**，而不是恒等于同一个常量。
        assert_ne!(
            text,
            format!("{:?}", ParamTable::new(SR)),
            "有 1 个在册槽位的 Debug 文本必须与空表不同（否则 `slots` 没被打印成读数）"
        );
    }
    /// 判据：声相槽位常量是 **2 / 3**（字面量钉住，⛔ 不是常量自比），
    /// 且 [`ParamTable`] **不消费**它们 —— 它把它们判成 `Unmapped` 并计数。
    ///
    /// 两条合起来钉住"分流发生在 `rt.rs` 的事件循环里"这个结构：常量被改动、
    /// 或有人把声相槽位接进 `accept`，本判据就变红。
    #[test]
    fn pan_slot_constants_are_two_and_three_and_the_param_table_rejects_them() {
        assert_eq!(TRACK_PAN_LEFT_SLOT, 2, "声相左槽位号是 2（字面量契约）");
        assert_eq!(TRACK_PAN_RIGHT_SLOT, 3, "声相右槽位号是 3（字面量契约）");
        // R58：等号判据要有一条落在**同一个**表达式上的 `assert_ne!`。
        assert_ne!(
            TRACK_PAN_LEFT_SLOT, TRACK_PAN_RIGHT_SLOT,
            "两个声相槽位必须不同"
        );
        let master = EntityId::new();
        let track = EntityId::new();
        let mut table = ParamTable::new(SR);
        for slot in [TRACK_PAN_LEFT_SLOT, TRACK_PAN_RIGHT_SLOT] {
            assert_eq!(
                table.accept(ParamAddress::new(track, slot), 0.5, master),
                ParamOutcome::Unmapped,
                "槽位 {slot} 必须**不**被参数表消费（分流在 rt.rs 的事件循环里）"
            );
        }
        assert_eq!(table.unmapped(), 2, "两次都必须被计数（不静默）");
        assert_eq!(table.slot_count(), 0, "不得为它们建槽位");
    }
}
