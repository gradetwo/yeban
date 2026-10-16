//! MIDI CC 的**会话运行态**表（任务单 `#107`）：事件面 → 实时 SFZ 通路的"喂值"一半。
//!
//! # 1. 这张表**是什么**（以及它**不是**什么）
//!
//! 它是 [`crate::ring::EngineEvent::ControlChange`] 的落点：一个**定长** `[u8; 128]`
//! （控制器号 → 当前值）＋ 一个 128 位的"收到过"位图（每个控制器一位）。
//! [`crate::rt::EngineRuntime`] 按值持有它（⛔ 无 `Vec`／⛔ 无 `HashMap`／⛔ 无 `Box`），
//! 事件在**量子边界**的批量出队里写它，逐块路径只在**旁路**读它。
//!
//! ⛔ 它**不是**持久化工程状态：[MODEL-ISO-001] 把"持久化 AST ／ 会话运行态 ／ 本机配置"
//! 三层物理隔离，而 CC 是**挥发性会话运行态**（与 [`crate::transport::Transport`] 的
//! 播放头同一层）⇒ ⛔ 不进 `EngineSnapshot`、⛔ 不进 `yeban-model`、⛔ 不进任何 schema。
//!
//! # 2. 口径（逐条，都是判据读得到的）
//!
//! 1. **未收到过 ⇒ `None`**：[`Self::received`] 对"这个控制器**一条事件都没有**"返回
//!    `None`（而不是 `Some(0)`）；[`Self::is_armed`] 是"**整张表**一条事件都没有"的读数
//!    ⇒ 实时探针**只在 armed 之后**才接进 [`crate::sfz_instrument`]
//!    （`Some(probe)`）。没有 CC 事件的工程因此与接线前**逐位相同**。
//! 2. **未设置的控制器读 `0`**：一旦 armed，探针就是一个**纯函数**
//!    `Fn(u8) -> u8`（`yeban-sfz` 的 [`yeban_sfz::instrument::RegionQuery::cc`] 形状），
//!    它对"没收到过的那一个控制器"只能给出**一个** `u8`——本表给 `0`（表初值）。
//!    ⚠ 这是**本表**的口径，⛔ **不是** `<control> set_ccN=` 的初值：
//!    "调用方在加载乐器时按 [`yeban_sfz::instrument::Instrument::cc_defaults`] 播种"
//!    仍归调用方（登记见 `crate::sfz_instrument` 模块头的 CC 一节）。本单 ⛔ 不播种。
//!
//!    ⭐ ⭐ **追加修订段（任务单 `#110`）—— 被修订的是上面这一句的
//!    "**本单 ⛔ 不播种**"那一支**（⛔ 原文一字未删，口径同台账第 278 条：保留原文 ＋
//!    追加修订段 ＋ 点名被修订的支）：**播种已经接线**，机制在 `yeban-engine` 的**快照边界**
//!    ——`rt.rs::render_block` 的重新武装分支按**摊平表**里的
//!    [`crate::sfz_instrument::CcSeedTable`]（`PreparedSfz::cc_seeds`）调
//!    [`ControlChangeState::seed`]。⚠ **被修订的只是"归谁在什么时候写"**，不是上面那条
//!    读值口径：未收到过、也没被播种的控制器**仍然读 `0`**；而**没有任何 `set_ccN`** 的
//!    快照 ⛔ 一次 `seed` 都不调 ⇒ 本文件 §2 第 1 条的 armed 口径**整条有效**
//!    （`armed == false` ⇒ 引擎传 `None` ⇒ 与接线前逐位相同）。
//! 3. **越界输入按 `& 0x7F` 掩码**：与 `yeban-app` 的 `midi_live::note_action`
//!    对数据字节的掩码同口径 ⇒ 一个越界字节**不会**写到表外，也**不会**变成幻觉控制器。
//! 4. **FIFO**：事件在出队顺序里逐条写入 ⇒ 同一量子里的多条同控制器事件按到达顺序覆盖，
//!    最后一条赢（与 `midi_live::pump_once_with` 的"到达顺序即弹出顺序"同一条链）。
//!
//! # 3. 实时安全（红线 7）
//!
//! [`Self::apply`] 与 [`Self::value`] 是**两个整数下标 + 一个位或**：零分配、零锁、
//! 零 I/O、零超越函数 ⇒ 可在音频回调内调用。整张表 `144 B`（`128 + 2 × 8`），
//! 全 `Copy`、⛔ 无 `Drop`。
//!
//! # 4. 尺寸（机械读数）
//!
//! `size_of::<ControlChangeState>() = 144`（`[u8; 128]` ＋ `[u64; 2]`，对齐 8）。
//! 它进 [`crate::rt::EngineRuntime`] ⇒ 那个实时对象 `180000 → 180144 B`
//! （门槛 192 KiB = `196608`，余量 `16608 → 16464`）；⛔ 它**不进** `SfzVoice`
//! （仍 `40 B`）、⛔ 不进 `SfzVoicePool`（仍 `1712 B`）。

/// 控制器个数（MIDI 1.0 的 CC 号域就是 `0..=127`）。
pub const CONTROL_CHANGE_COUNT: usize = 128;

/// "收到过"位图的字数（`128 / 64 = 2`）。
const CONTROL_CHANGE_WORDS: usize = CONTROL_CHANGE_COUNT / 64;

/// ⭐ **MIDI CC 的会话运行态表**（任务单 `#107`）。
///
/// 形状与口径见模块文档；它是 `Copy`（`144 B`），⛔ 没有堆、⛔ 没有 `Drop`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlChangeState {
    /// 控制器号 → 当前值（`0..=127`）。初值全 `0`。
    values: [u8; CONTROL_CHANGE_COUNT],
    /// "这个控制器**收到过**事件"的位图（字 0 = `0..=63`，字 1 = `64..=127`）。
    ///
    /// 它存在的理由只有一个：把"从未收到"与"收到 0"**分开**（[`Self::received`]），
    /// 而这条区分正是"没有 CC 事件的工程逐位不变"的机械防线
    /// （[`Self::is_armed`] 为假 ⇒ 实时探针不接进来）。
    received: [u64; CONTROL_CHANGE_WORDS],
}

impl ControlChangeState {
    /// 空表（全 `0`、一个控制器都没收到过）。`const` ⇒ 可在构造期/常量里用。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            values: [0; CONTROL_CHANGE_COUNT],
            received: [0; CONTROL_CHANGE_WORDS],
        }
    }

    /// 空表（与 [`Self::new`] 同值；常量形态便于判据与字段初值直接引用）。
    pub const EMPTY: Self = Self::new();

    /// ⭐ 写入一条 CC（**事件边界**；越界按 `& 0x7F` 掩码）。
    ///
    /// 返回**写入后的值**（＝ 掩码后的 `value`）⇒ 调用方不需要再读一次表。
    ///
    /// 零分配、零锁、零 I/O、一次位或 ⇒ 可在音频回调内调用 [MUST-GATE-001]。
    pub fn apply(&mut self, controller: u8, value: u8) -> u8 {
        let controller = usize::from(controller & 0x7F);
        let value = value & 0x7F;
        self.values[controller] = value;
        self.received[controller / 64] |= 1u64 << (controller % 64);
        value
    }

    /// ⭐ **播种**一条 `<control> set_ccN=` 的加载时初值（任务单 `#110`；**快照边界**）。
    ///
    /// 口径（三条，都可机械判定）：
    ///
    /// 1. **只在"这个控制器从未收到过"时写**：已经有值（真 MIDI CC 或更早的播种）
    ///    ⇒ **跳过**并返回 `false`。这条规则正是规范逐字 "**default initial value** …
    ///    when the instrument is initially loaded" 的机械形态 —— 初值只在没有真值时才成立；
    ///    ⭐ 它同时让播种**幂等**：每次快照重新武装（任何编辑都会发新修订）再播一遍，
    ///    ⛔ **不会**覆盖演奏中已经收到的真 CC。
    /// 2. **播种会计进"收到过"**：否则 [`Self::received`] 读 `None`、
    ///    [`Self::is_armed`] 读 `false` ⇒ 探针拿不到那个值（播种就白做了）。
    ///    ⚠ 这是**与真 MIDI CC 的唯一区别**：写入的表状态**故意**同形，
    ///    区分点在**时机**（加载/快照边界 vs 事件边界）与**先后**（播种⛔ 不覆盖真值）。
    /// 3. 越界按 `& 0x7F` 掩码（与 [`Self::apply`] 同口径）—— ⚠ 调用方
    ///    （[`crate::sfz_instrument::CcSeedTable`]）**已经**把 `N > 127` 的声明挡在外面
    ///    并计数，⛔ 不靠这次掩码去"别名"它们。
    ///
    /// 零分配、零锁、零 I/O、一次位或 ⇒ 可在快照边界调用 [MUST-GATE-001]。
    pub fn seed(&mut self, controller: u8, value: u8) -> bool {
        let controller = usize::from(controller & 0x7F);
        let bit = 1u64 << (controller % 64);
        if self.received[controller / 64] & bit != 0 {
            return false;
        }
        self.values[controller] = value & 0x7F;
        self.received[controller / 64] |= bit;
        true
    }

    /// 某个控制器**此刻的值**（未收到过也返回表里的 `0`；越界按 `& 0x7F` 掩码）。
    ///
    /// ⚠ 这就是喂给 `yeban-sfz` 探针（`Fn(u8) -> u8`）的那一个函数体：
    /// 它是**全定义**的（对 128 个控制器都有答案），口径见模块文档 §2.2。
    #[must_use]
    pub const fn value(&self, controller: u8) -> u8 {
        self.values[(controller & 0x7F) as usize]
    }

    /// 某个控制器是否**收到过**至少一条事件。
    #[must_use]
    pub const fn is_received(&self, controller: u8) -> bool {
        let controller = (controller & 0x7F) as usize;
        self.received[controller / 64] & (1u64 << (controller % 64)) != 0
    }

    /// ⭐ **读数**：收到过 ⇒ `Some(值)`；**从未收到** ⇒ `None`。
    ///
    /// 与 [`Self::value`] 的分工：后者是喂给探针的**全定义**读数，本方法是给控制面/判据
    /// 的**可区分**读数（"没收到" 与 "收到 0" 在它上面不同）。
    #[must_use]
    pub const fn received(&self, controller: u8) -> Option<u8> {
        if self.is_received(controller) {
            Some(self.value(controller))
        } else {
            None
        }
    }

    /// ⭐ **整张表是否已 armed**（＝ 至少收到过一条 CC 事件）。
    ///
    /// 实时探针的**唯一**开关：`false` ⇒ 传 `None`（与接线前逐位相同）；
    /// `true` ⇒ 传 `Some(probe)`（未设置的控制器读 `0`，见模块文档 §2.2）。
    #[must_use]
    pub const fn is_armed(&self) -> bool {
        let mut index = 0;
        while index < CONTROL_CHANGE_WORDS {
            if self.received[index] != 0 {
                return true;
            }
            index += 1;
        }
        false
    }

    /// 收到过事件的**控制器个数**（读数：分母 128）。
    #[must_use]
    pub const fn received_count(&self) -> u32 {
        let mut total = 0;
        let mut index = 0;
        while index < CONTROL_CHANGE_WORDS {
            total += self.received[index].count_ones();
            index += 1;
        }
        total
    }
}

impl Default for ControlChangeState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 尺寸：`144 B`（判据与报告里的"定长小表"读数就是它）。
    #[test]
    fn the_table_is_fixed_size_and_copy() {
        assert_eq!(core::mem::size_of::<ControlChangeState>(), 144);
        assert_eq!(core::mem::align_of::<ControlChangeState>(), 8);
        fn assert_copy<T: Copy>() {}
        assert_copy::<ControlChangeState>();
    }

    /// 口径 ①②③：未收到 ⇒ `None`；收到 0 ⇒ `Some(0)`；越界掩码；最后一条赢。
    #[test]
    fn the_registration_readings_are_distinguishable() {
        let mut state = ControlChangeState::new();
        assert!(!state.is_armed(), "空表没有 armed");
        assert_eq!(state.received(7), None, "没收到 ⇒ None（⛔ 不是 Some(0)）");
        assert_eq!(state.value(7), 0, "探针读数仍是全定义的 0");
        assert_eq!(state.received_count(), 0);

        assert_eq!(state.apply(7, 100), 100);
        assert!(state.is_armed());
        assert_eq!(state.received(7), Some(100));
        assert_eq!(state.value(7), 100);
        assert_eq!(state.received_count(), 1);

        // 收到 0 与从未收到**可区分**。
        state.apply(8, 0);
        assert_eq!(state.received(8), Some(0));
        assert_eq!(state.received(9), None);
        assert_eq!(state.received_count(), 2);

        // 越界按 `& 0x7F` 掩码：`0x87` ⇒ 控制器 7、值 `0x80` ⇒ 0。
        assert_eq!(state.apply(0x87, 0x80), 0);
        assert_eq!(state.received(7), Some(0), "最后一条赢（FIFO）");
        assert_eq!(state.received_count(), 2, "掩码后的控制器仍落在 128 个之内");
        assert_eq!(
            state.value(0xFF),
            0,
            "0xFF & 0x7F = 127（表内，未收到 ⇒ 0）"
        );
    }

    /// ⭐ 任务单 `#110`：**播种**的三条口径（只在未收到过时写 ＋ 会计进 armed ＋ 幂等）。
    #[test]
    fn seeding_only_fills_slots_that_never_received_a_real_cc() {
        let mut state = ControlChangeState::new();
        assert!(!state.is_armed(), "空表没有 armed");

        // ① 空槽 ⇒ 真的写进去，并且**读得到**（`received` 是 `Some`、表 armed）。
        assert!(state.seed(1, 64), "未收到过 ⇒ 播种成功");
        assert_eq!(state.received(1), Some(64), "播种后探针读得到那个初值");
        assert!(state.is_armed(), "播种会计进 armed（否则探针拿不到值）");
        assert_eq!(state.received_count(), 1);

        // ② 幂等：同一个槽再播一次 ⇒ 跳过（返回 false），值不变。
        assert!(!state.seed(1, 0), "已经收到过/播种过 ⇒ 跳过");
        assert_eq!(state.received(1), Some(64), "幂等：⛔ 不覆盖");

        // ③ 真 CC **赢**：先播种再 `apply` ⇒ 后者覆盖；再来一次播种仍跳过。
        assert_eq!(state.apply(1, 7), 7);
        assert_eq!(state.received(1), Some(7), "真 CC 覆盖初值");
        assert!(!state.seed(1, 64), "真 CC 之后播种照样跳过");
        assert_eq!(state.received(1), Some(7));

        // ④ 越界播种按 `& 0x7F` 掩码（调用方另有域检查，见 `CcSeedTable`）。
        assert!(
            !state.seed(0x81, 0x40),
            "0x81 ⇒ 控制器 1（已被真 CC 占）⇒ 跳过"
        );
        assert!(state.seed(0x82, 3), "0x82 ⇒ 控制器 2（空槽）⇒ 写得进");
        assert_eq!(state.received(2), Some(3));
    }
}
