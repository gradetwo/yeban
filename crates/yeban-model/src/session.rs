//! `SessionRuntimeState` —— **挥发性会话运行态**（`MODEL-ISO-001` 第 2 层）。
//!
//! 规范来源：`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §2 `[MODEL-ISO-001]`：
//!
//! > **`SessionRuntimeState`（挥发性运行时状态层）**：包含当前播放头 Tick、`isPlaying`、
//! > 任务进度、插件进程 PID、视窗打开状态等。**严禁持久化存入 Commit**。
//!
//! ## 这一层为什么必须与 [`crate::project::YebanProjectV1`] 物理隔离
//!
//! 本模块的每一个字段都是**进程生命周期的函数**，不是作品的函数：
//!
//! | 字段 | 为什么不能进工程文件 |
//! | :--- | :--- |
//! | [`SessionRuntimeState::playhead_ticks`] | 播放头是"我上次听到哪儿"，同一份作品在两台机器上必然不同 |
//! | [`SessionRuntimeState::is_playing`] | 打开工程时是否正在播放，是会话事实 |
//! | [`SessionRuntimeState::tasks`] | 任务进度是**当前进程**的 IO 进度，重启即失效 |
//! | [`SessionRuntimeState::plugin_processes`] | 插件进程 PID 是操作系统级运行时事实，跨机无意义 |
//! | [`SessionRuntimeState::open_windows`] | 视窗布局是显示会话状态 |
//! | [`SessionRuntimeState::undo_cursor`] | 撤销游标 [ADR-0001 `D45`] 明令属会话运行态 |
//!
//! 一旦其中任何一项被写进 `.yeban`，同一份作品就会因为**打开顺序**不同而产生不同的
//! Commit 字节 —— 这会直接击穿 `ARCH-DET-001`（写入确定性）与 `ARCH-OPS-*`（提交图谱）。
//!
//! ## "严禁持久化"是**可机械证明**的，不是一句注释
//!
//! 本模块刻意**不引用 `serde`**（`use` 里没有、derive 里没有、手写 impl 也没有）：
//!
//! 1. **类型层**：本类型没有 `Serialize` / `Deserialize` 实现 ⇒
//!    `assert_serializable::<SessionRuntimeState>()` **编译失败**。
//!    这条以 `compile_fail` doc-test 的形式常驻（见 [`SessionRuntimeState`] 的文档），
//!    并配一条**必须编译成功**的对照（`YebanProjectV1` 确实实现了 `Serialize`）——
//!    否则"编译失败"可能只是因为探针本身写错了。
//! 2. **字节层**：`crates/yeban-model/tests/model_isolation.rs` 冻结了
//!    `YebanProjectV1` 的**逐字节样本 SHA-256** 与**递归键路径集合**；
//!    往工程里加任何一个会话字段都会让那两条判据变红。
//! 3. **源码层**：同一判据扫描本文件，断言 `session.rs` 里没有 `serde` / `Serialize`
//!    字样，并对 `project.rs` / `local_config.rs` 做**正向对照**证明这条扫描有牙。
//!
//! ## 红线 4 / `MODEL-AST-003`
//!
//! 本模块虽然是**非**持久化层，仍然一律使用 `BTreeMap` / `BTreeSet`
//! （`scripts/guards/policy_check.py` 的 `G01` 对整个 `crates/yeban-model/src` 生效）：
//! 会话态的迭代顺序同样要求确定 —— 否则"任务列表 UI"与"插件 PID 列表"会随
//! 哈希种子抖动，把不确定性从持久化层赶到运行时层，等于没解决。

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::commit::UndoCursor;
use crate::ids::{EntityId, PPQ};

/// 会话内长任务的身份（单调递增的进程内计数器，**不是** [`EntityId`]）。
///
/// 刻意不用 ULID：任务进度**严禁**持久化，给它一个跨进程稳定的身份反而会诱导
/// 后续把它写进文件。字段私有 ⇒ 只能经 [`TaskId::new`] 构造。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskId(u64);

impl TaskId {
    /// 构造一个任务身份。
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// 取出原始计数器值。
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// 长任务的类别（纯会话态：描述"这一次运行在干什么"）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TaskKind {
    /// 打开工程容器。
    ProjectLoad,
    /// 保存工程容器。
    ProjectSave,
    /// 导入音频资产。
    AssetImport,
    /// 扫描插件。
    PluginScan,
    /// 离线渲染 / 母带。
    Render,
    /// 导出（分轨 / MIDI / 实验性 `.als`）。
    Export,
}

/// 单个长任务的进度快照。
///
/// `total_units == 0` 是**非法**状态（无法表达"进度"），由
/// [`SessionRuntimeState::validate`] 拒绝，而不是靠 `Option` 把它变成静默的 0。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskProgress {
    /// 任务类别。
    pub kind: TaskKind,
    /// 已完成的工作单元数。
    pub completed_units: u64,
    /// 总工作单元数（必须 `> 0`）。
    pub total_units: u64,
    /// 是否允许用户取消。
    pub cancellable: bool,
    /// 面向用户的短标签（**绝不**放进工程：它可能含本机路径）。
    pub label: String,
}

impl TaskProgress {
    /// 进度分数（整数百分比，`0..=100`，不引入浮点）。
    #[must_use]
    pub fn percent(&self) -> u64 {
        if self.total_units == 0 {
            return 0;
        }
        // 先乘后除保证整数语义；`total_units > 0` 已保证不会除零。
        (self.completed_units.min(self.total_units) * 100) / self.total_units
    }

    /// 是否已完成（`completed_units >= total_units`）。
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.total_units > 0 && self.completed_units >= self.total_units
    }
}

/// 插件宿主进程的运行时快照（沙箱隔离出的子进程）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginProcess {
    /// 操作系统进程 ID（PID 只在**本机本会话**内有意义）。
    pub pid: u32,
    /// 该进程是否运行在沙箱 / 带外隔离中（`ARCH-PLUG-*`）。
    pub sandboxed: bool,
}

/// 已打开的视窗身份。
///
/// 用 `enum` 而不是字符串，是为了让"哪些视窗可以打开"成为**编译期**已知的集合；
/// 插件编辑器视窗按插件实例区分（同一个插件可以开多个编辑器窗口）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WindowId {
    /// 编曲视图。
    Arrangement,
    /// 钢琴卷帘。
    PianoRoll,
    /// 混音台。
    Mixer,
    /// 素材浏览器。
    Browser,
    /// 某个插件实例的编辑器窗口。
    PluginEditor(EntityId),
}

/// 会话运行态的内部一致性错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionStateError {
    /// 任务进度越界（`completed_units > total_units`）。
    #[error("task {task} reports {completed} of {total} units (completed exceeds total)")]
    TaskProgressOutOfRange {
        /// 任务身份。
        task: u64,
        /// 已报告完成数。
        completed: u64,
        /// 已报告总数。
        total: u64,
    },
    /// 任务总单元数为零（进度不可表达）。
    #[error("task {task} declares zero total units; progress would be undefined")]
    TaskTotalZero {
        /// 任务身份。
        task: u64,
    },
}

/// **第 2 层：会话运行态**（`MODEL-ISO-001`）。
///
/// 本类型**没有** `Serialize` / `Deserialize`，这是设计而非疏漏 ——
/// 详见模块文档。下面这条 `compile_fail` doc-test 就是"严禁持久化"的常驻机械证据：
///
/// ```compile_fail
/// // 类型层证据 [MODEL-ISO-001 第 2 层]：会话态没有 Serialize ⇒ 这一行必须编译失败。
/// fn assert_serializable<T: serde::Serialize>() {}
/// assert_serializable::<yeban_model::SessionRuntimeState>();
/// ```
///
/// 对照（**必须编译成功**）证明上面那条不是"探针写错了"造成的假绿：
///
/// ```
/// fn assert_serializable<T: serde::Serialize>() {}
/// assert_serializable::<yeban_model::YebanProjectV1>();
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRuntimeState {
    /// 播放头位置，单位 **tick**（[`PPQ`] = 960 tick / 四分音符，整数语义）。
    pub playhead_ticks: u64,
    /// 走带是否正在播放。
    pub is_playing: bool,
    /// 长任务进度（任务身份 → 进度）。
    pub tasks: BTreeMap<TaskId, TaskProgress>,
    /// 插件宿主子进程（插件实例 → 进程快照）。
    pub plugin_processes: BTreeMap<EntityId, PluginProcess>,
    /// 已打开的视窗集合（`BTreeSet`：迭代顺序确定，红线 4）。
    pub open_windows: BTreeSet<WindowId>,
    /// 撤销游标（[ADR-0001 `D45`]：属会话运行态，见 [`UndoCursor`]）。
    pub undo_cursor: UndoCursor,
}

impl Default for SessionRuntimeState {
    /// 一个"刚打开工程、还没动过"的干净会话。
    fn default() -> Self {
        Self {
            playhead_ticks: 0,
            is_playing: false,
            tasks: BTreeMap::new(),
            plugin_processes: BTreeMap::new(),
            open_windows: BTreeSet::new(),
            undo_cursor: UndoCursor::new(),
        }
    }
}

impl SessionRuntimeState {
    /// 全新的会话（等价于 [`Default`]）。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 每四分音符的 tick 数（960 PPQ 的**唯一**语义入口）。
    #[must_use]
    pub const fn ticks_per_beat() -> u64 {
        PPQ
    }

    /// 一小节的 tick 数；参数与 `PPQ` 不整除时返回 `None`（拒绝而不是取整）。
    ///
    /// `PPQ = 960` 对 1/2/4/8/16/32/64 分音符全部整除，因此常规拍号都落在整数 tick 上。
    #[must_use]
    pub const fn ticks_per_bar(numerator: u8, denominator: u8) -> Option<u64> {
        if numerator == 0 || denominator == 0 {
            return None;
        }
        // 一小节 = numerator × 全音符/denominator = numerator × (4 × PPQ) / denominator。
        let whole_note_ticks = 4 * PPQ;
        let denominator = denominator as u64;
        if !whole_note_ticks.is_multiple_of(denominator) {
            return None;
        }
        Some((numerator as u64) * (whole_note_ticks / denominator))
    }

    /// 当前播放头 tick。
    #[must_use]
    pub const fn playhead_ticks(&self) -> u64 {
        self.playhead_ticks
    }

    /// 播放头所在小节序号（0 基）与小节内 tick，全部为整数运算。
    ///
    /// 拍号非法（[`Self::ticks_per_bar`] 返回 `None`）时返回 `None`。
    #[must_use]
    pub const fn playhead_bar_and_offset(
        &self,
        numerator: u8,
        denominator: u8,
    ) -> Option<(u64, u64)> {
        match Self::ticks_per_bar(numerator, denominator) {
            Some(bar_ticks) => Some((
                self.playhead_ticks / bar_ticks,
                self.playhead_ticks % bar_ticks,
            )),
            None => None,
        }
    }

    /// 走带是否正在播放。
    #[must_use]
    pub const fn is_playing(&self) -> bool {
        self.is_playing
    }

    /// 显式设置走带状态。
    pub const fn set_playing(&mut self, playing: bool) {
        self.is_playing = playing;
    }

    /// 起播（`is_playing = true`）。
    pub const fn play(&mut self) {
        self.is_playing = true;
    }

    /// 停播（`is_playing = false`）。**不动**播放头：停止后再次起播从原处继续。
    pub const fn stop(&mut self) {
        self.is_playing = false;
    }

    /// 定位播放头（绝对 tick）。
    pub const fn seek_ticks(&mut self, tick: u64) {
        self.playhead_ticks = tick;
    }

    /// 前进 `delta` tick，返回新播放头位置（饱和加法：不 wrap，不 panic）。
    pub const fn advance_ticks(&mut self, delta: u64) -> u64 {
        self.playhead_ticks = self.playhead_ticks.saturating_add(delta);
        self.playhead_ticks
    }

    /// 把播放头拉回零（回到曲首）。
    pub const fn rewind(&mut self) {
        self.playhead_ticks = 0;
    }

    /// 全部插件进程的 PID 集合（去重 + **升序**：`BTreeSet` 确定性，红线 4）。
    #[must_use]
    pub fn plugin_pids(&self) -> BTreeSet<u32> {
        self.plugin_processes.values().map(|p| p.pid).collect()
    }

    /// 登记 / 覆盖一个插件进程快照。
    pub fn track_plugin_process(&mut self, instance: EntityId, process: PluginProcess) {
        self.plugin_processes.insert(instance, process);
    }

    /// 注销一个插件进程（返回是否真的存在过）。
    pub fn forget_plugin_process(&mut self, instance: &EntityId) -> bool {
        self.plugin_processes.remove(instance).is_some()
    }

    /// 打开一个视窗（已在集合中则幂等）。
    pub fn open_window(&mut self, window: WindowId) -> bool {
        self.open_windows.insert(window)
    }

    /// 关闭一个视窗（返回是否真的关掉了）。
    pub fn close_window(&mut self, window: &WindowId) -> bool {
        self.open_windows.remove(window)
    }

    /// 视窗当前是否打开。
    #[must_use]
    pub fn is_window_open(&self, window: &WindowId) -> bool {
        self.open_windows.contains(window)
    }

    /// 视窗集合的**确定性**顺序快照（升序，`BTreeSet` 迭代序）。
    #[must_use]
    pub fn windows_in_order(&self) -> Vec<WindowId> {
        self.open_windows.iter().copied().collect()
    }

    /// 登记 / 覆盖一个任务进度。
    pub fn set_task(&mut self, task: TaskId, progress: TaskProgress) {
        self.tasks.insert(task, progress);
    }

    /// 撤销一个任务（返回是否真的存在过）。
    pub fn clear_task(&mut self, task: TaskId) -> bool {
        self.tasks.remove(&task).is_some()
    }

    /// 读一个任务的进度。
    #[must_use]
    pub fn task_progress(&self, task: TaskId) -> Option<&TaskProgress> {
        self.tasks.get(&task)
    }

    /// 会话态内部一致性校验（越界即 `Err`，绝不 panic）。
    ///
    /// # Errors
    ///
    /// - 某任务 `total_units == 0` ⇒ [`SessionStateError::TaskTotalZero`]；
    /// - 某任务 `completed_units > total_units` ⇒
    ///   [`SessionStateError::TaskProgressOutOfRange`]。
    pub fn validate(&self) -> Result<(), SessionStateError> {
        for (task, progress) in &self.tasks {
            if progress.total_units == 0 {
                return Err(SessionStateError::TaskTotalZero { task: task.get() });
            }
            if progress.completed_units > progress.total_units {
                return Err(SessionStateError::TaskProgressOutOfRange {
                    task: task.get(),
                    completed: progress.completed_units,
                    total: progress.total_units,
                });
            }
        }
        Ok(())
    }
}
