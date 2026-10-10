//! **引擎宿主**：把 `YebanProjectV1` 变成一个真的在跑的 `EngineRuntime` + 一条新的电平队列
//! （`ui/reload_engine` 的落地；`[ARCH-RT-002]` / `[ROAD-M2-002]` / `[ARCH-UI-002]`）。
//!
//! ## 规范来源 (Normative)
//!
//! - `[ARCH-RT-002]` / `[ROAD-M2-002]`：`EngineSnapshot` 投影 + `SnapshotSlot` 原子交换 +
//!   退役回收队列（旧快照由**非实时线程**释放）。
//! - `[ARCH-UI-002]`：电平经独立 SPSC 解耦 —— 本模块建队列、把**消费端**交给 UI 线程
//!   （[`crate::meters::MeterRuntime`]），生产端留在引擎运行时里。
//! - `[ARCH-TOP-002]`：线程拓扑。**实时线程**跑 `EngineRuntime::process_quantum`；
//!   本模块的 `reload` 是**控制面/测试线程**的动作（允许分配、允许建队列）。
//! - ADR-0001 **D19**：本文件只用 `yeban-engine` 的公开面，**一个 `cpal::*` 都不出现**
//!   （`snapshot` / `rt` / `ring` / `meter` / **`device`**）；声卡宿主本身仍是引擎侧的事
//!   —— 本 crate 不写一行 cpal 调用，只调 [`yeban_engine::device::open_output`]。
//!
//! ## 为什么这不是"假装重建"
//!
//! `reload` 做的是**真的**四件事，每一步都有可观测的后果：
//!
//! 1. `EngineSnapshot::from_project(project, revision)` —— 真的把工程投影成引擎快照
//!    （失败就**报错**，例如工程没有主总线 ⇒ `SnapshotError::NoMasterBus`）；
//! 2. `SnapshotSlot::new(snapshot)` + `retire_channel` + `event_channel` + `meter_channel`
//!    —— 四样东西全是**新**的，旧的一代整体丢弃（不是"改改参数继续用"）；
//! 3. `EngineRuntime::new(...)` 后**真的推 `quanta` 个量子**：
//!    `meter_bulk_publishes == quanta`、`meter_frames == quanta × (非母线轨数 + 1)`
//!    （这两个数是引擎侧的结构性契约，见 `docs/ledger/engine-meters-notes.md` §2）；
//! 4. 新队列的**消费端**交还给 UI 线程 ⇒ 电平面板被清空后重新收到引擎发布的新帧
//!    （界面上的可观测副作用：混音台电平条回到下限）。
//!
//! ## 诚实边界（不要误读）
//!
//! - **引擎在合成**（2026-10-08 就地改正）：轨道渲染**不再**是占位静音 ——
//!   `project_schedules` 真的按摆放生成调度表、`synth.render_track` 真的渲染，
//!   因此 `process_quantum` 发布的电平**不是**恒为静音。
//!   实测（**CI 作业 `rust (yeban-app)` 的 `test` 步，run `38037793136`**；判据
//!   `production_meter_leg::production_loop_start_adopts_the_engine_meter_consumer` 按**生产形态**
//!   传工程 ⇒ **自动化生效**）：0 号轨峰值
//!   `-17.3 / -7.1 / -6.8 / -7.4 / -7.7 / -7.9 / -8.4 / -8.6` dBFS，
//!   主总线 `-20.4 / -13.1 / -9.8 / -10.0 / -10.7 / -10.9 / -11.2 / -11.6` dBFS。
//!   ⚠ 这 8 对数值**取代**了 R68 之前的 `-17.3 / -7.1 / -6.8 / -7.3 / -7.6 / -7.9 / -8.3 / -8.5`
//!   与 `-20.4 / -13.1 / -9.8 / -10.0 / -10.6 / -10.9 / -11.1 / -11.5`：后者测于判据把
//!   **工程参数传成 `None`** 时 ⇒ 那一跳 `publish_automation` 整段不跑（自动化泳道被静默丢弃）。
//!   这是**施加曲线**这个语义本身（裁决 R55/R68），⛔ 不是漂移；前 3 跳未动是因为平滑器
//!   （τ ≈ 5 ms）在这几跳里还没走起来。
//!   出处 = `cargo test -p yeban-app --test production_meter_leg production_loop_start -- --nocapture`
//!   的 `[meter-leg] tick #0‥#7` 行（测试写 stderr ⇒ libtest 不吞 ⇒ CI 日志里读得到）。
//!   同 crate 的判据 `production_loop_start_adopts_the_engine_meter_consumer` 正是断言
//!   "至少一条轨的读数必须离开显示下限"。
//!   ⚠ 本行原文写「**引擎仍然不发声**：轨道渲染是占位静音，因此引擎实际发布的电平恒为静音
//!   （`docs/ledger/engine-meters-notes.md` §0.1）」—— 那句话与上面那条判据**矛盾**，
//!   也与这份实测矛盾，因此**就地改正**。残留（本票不许碰其它 `docs/**`）：
//!   `docs/ledger/engine-meters-notes.md` §0.1 至今仍写同一条旧结论，需要它自己的所有者更正。
//!   "重建是真的"仍然指快照/队列/量子驱动是真的 —— 这一半没有变。
//! - **开声卡了（2026-10-08 就地改正）**：本切片现在调用 `yeban_engine::device`
//!   （[`EngineHost::open_device`] 走 `device::open_output` + `play()`）。原文写
//!   「本切片不调用 `yeban_engine::device`（红线 6 与 D19 都要求设备 I/O 单独裁决）」
//!   —— 复核两条裁决的**原文**后，这个理由是**不成立**的：
//!   `AGENTS.md` §2 红线 6 是「**发行特性安全红线**：官方默认 release 构建中严禁默认开启
//!   `mcp-http`、`ui-mcp`、`asio`、`experimental-vst3`、`experimental-als-export` 或
//!   `experimental-logic-export`」；ADR-0001 **D19** 是「实时引擎与离线渲染**共用 PDC 算法**，
//!   但离线侧不得被迫拖入 cpal」（裁决 = cpal 走 `yeban-engine` 的 `device` feature、
//!   `yeban-render` 用 `default-features = false`）。**两条都没有禁止开声卡**：
//!   红线 6 管的是"哪些 feature 不能默认开"，D19 管的是"离线渲染器不许被迫编译 cpal"。
//!   事实上 `yeban-engine` 的 `default = ["device"]` ⇒ `cpal 0.18.2` **早就在**
//!   `cargo tree -p yeban-app -e normal --locked` 的默认依赖图里，本票**没有**改任何
//!   feature、没有加任何依赖。D19 的字面要求（app 里不出现 `cpal::*`）继续成立：
//!   本文件只用 [`yeban_engine::device`] 的具名类型，裸 `cpal` 一个字都没有。
//! - **今天怎么驱动**：三种形态，同一时刻只有一种（[`EngineHost::open_device`] 成功之后
//!   控制面**不再**推进量子 —— `pump` / `drive_audio` 返回 0）：
//!
//!   | 形态 | 谁推进 `process_quantum` | 位置前进的驱动 |
//!   | :--- | :--- | :--- |
//!   | **设备腿**（有声卡） | cpal 回调线程（`device::open_output` 的闭包 → `device::render_callback`） | **真实时钟**（设备缓冲节拍） |
//!   | **控制驱动**（无声卡 / `open_device` 失败） | 控制面显式 `pump(n)` / `drive_audio(n)` | 每次走带动作 **1** 个量子（既有形态，一位没变） |
//!   | 无引擎（快照投影失败） | 没有 | 没有 |
//!
//!   ⚠ **两处仍然存在的缺口**（如实登记，本票**不实现**）：
//!   ① `[ROAD-M2-001]` **实时线程优先级未实现** —— cpal 0.18 的 `realtime` feature 只覆盖
//!   WASAPI / AAudio / PipeWire / JACK，macOS CoreAudio 与 Linux-ALSA 没有任何开关；
//!   自实现要 `pthread_setschedparam`（= 新的 `libc` 依赖）⇒ **依赖图裁决**，不由本工作线
//!   单独决定（`crates/yeban-engine/src/device.rs` 的模块文档与
//!   `docs/ledger/engine-rt-notes.md` §4 同一条）。
//!   ② **独占模式**：cpal 0.18 没有 WASAPI Exclusive 的 API ⇒
//!   `ShareMode::PreferExclusive` 静默降级为共享、`ShareMode::RequireExclusive` 返回
//!   `DeviceError::ExclusiveModeUnsupported`（宁可明确失败也不假装独占成功）。
//!   ⚠ 还有一处**本票引入的新边界**：设备腿活跃时 `EngineRuntime` 归 cpal 回调线程所有
//!   ⇒ [`EngineHost::engine_stats`] 返回 `None`（引擎累计统计不再可读）。走带读数仍可读
//!   （`TransportMirror` 是 `Arc`），电平仍可读（独立 SPSC），快照槽计数仍可读
//!   （`snapshot_counts`）。需要一个"跨线程只读的 `EngineStats` 镜像"才能补上 ——
//!   **本票不做**（登记为 needs，不发明第二份统计来源）。
//!
//! ## 编辑 ⇒ 发声：增量发布 ＋ 退役回收心跳
//!
//! `reload` **整体换代**（新槽 / 新队列 / 新 `EngineRuntime`，并推 `quanta` 个量子），
//! 因此它只适合 `ui/reload_engine` 那种"重建"动作：工程**改了一个参数**之后要换的只是
//! **快照**，用 `reload` 会连带改掉 `quanta` 契约。编辑路径因此走另一条：
//!
//! | 动作 | 谁做 | 何时 | 线程 |
//! | :--- | :--- | :--- | :--- |
//! | 增量发布（[`EngineHost::publish_project`]） | 控制线程 | 工程改动的那一跳（[`EditMark`] 变了） | UI / 主线程 |
//! | 写者侧回收（`SnapshotSlot::prune`） | [`EngineHost::heartbeat`] | 每跳（60Hz） | UI / 主线程 |
//! | 退役队列排空（`RetireQueue::drain`） | [`EngineHost::heartbeat`] | 每跳（60Hz） | UI / 主线程 |
//!
//! 生产接线是 `src/main.rs` 的 `run_gui`：一个 `slint::Timer`（`TimerMode::Repeated`，
//! 16 ms ⇒ 62.5 Hz）在**创建该槽的那一个线程**上跑"按需发布 + 回收"。窗口关闭 ⇒
//! `run_gui` 返回 ⇒ 定时器被 drop（停止）；控制面会话被关掉 ⇒ 权威工程读不到 ⇒
//! 发布自然停止，回收心跳继续跑到窗口关闭为止。
//!
//! ⚠ **没有留下"只发布、不回收"的路径**：发布（[`EngineHost::publish_project`]）与回收
//! （[`EngineHost::heartbeat`]）是**同一个调用序列**里的相邻两步，并且两者都在
//! **非实时线程**上 —— 音频回调那条读路径（`SnapshotReader::begin_block`）一位没动
//! [ARCH-RT-001 / MUST-GATE-001]。

use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_engine::device::{DeviceError, EngineConfig, OutputStreamHandle, ShareMode};
use yeban_engine::level::db_to_gain;
use yeban_engine::meter::{DEFAULT_METER_CAPACITY, MeterCollector, meter_channel};
use yeban_engine::mixer::{PanLaw, pan_gains};
use yeban_engine::param::{
    MASTER_GAIN_SLOT, PARAM_SLOTS, TRACK_GAIN_SLOT, TRACK_PAN_LEFT_SLOT, TRACK_PAN_RIGHT_SLOT,
};
use yeban_engine::ring::{EngineEvent, EventSender, ParamAddress, TransportCommand, event_channel};
use yeban_engine::rt::{EngineRuntime, EngineStats};
use yeban_engine::snapshot::{
    EngineSnapshot, RetireQueue, SnapshotError, SnapshotSlot, retire_channel,
};
use yeban_engine::transport::{TransportMirror, TransportReading, TransportState};
use yeban_model::AutomationTarget;
use yeban_model::EntityId;
use yeban_model::project::YebanProjectV1;

/// 走带命令等待设备回调确认的上界。
///
/// 设备腿活跃时命令只能**在下一个回调的块边界**生效（`render_block` 第 1 步出队）。
/// 控制面因此要等引擎自己的读数确认（`TransportReading::commands_applied` 前进），
/// 否则走带按钮会把**命令之前**的状态画回界面 —— 那是 UI 的假读数。
/// 一台正常的声卡回调周期是 3–10 ms，250 ms 是 25–80 倍余量；超时**不**换来源，
/// 只如实返回引擎此刻的读数（`quanta_pumped == 0`）。
const DEVICE_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

/// 设备腿轮询确认的间隔（控制线程；不是实时路径）。
const DEVICE_COMMAND_POLL: std::time::Duration = std::time::Duration::from_millis(1);

/// 退役队列容量（条）。
///
/// `reload` 同一次内最多发生一次快照交换；增量发布每一跳**至多**一次（[`EditMark`]
/// 变了才发），而心跳每一跳都会排空队列 ⇒ 32 条是 32 倍余量。真的排不上时读者会走
/// `stash`（寄存）并记 `stash_events`，**不会**在音频线程释放任何东西 [ARCH-RT-002]。
const RETIRE_CAPACITY: usize = 32;

/// UI → 引擎事件通道容量（条）。通道必须**在打开设备之前**建立
/// （`EngineRuntime::new` 的文档要求），所以它在这里就位。
///
/// 走带命令经这条通道进引擎（[`EngineHost::send_transport`]）：它是**既有的**
/// 无锁 SPSC 批量通道（`[ARCH-RT-001]` / `[ROAD-M2-007]`），不是为走带新造的第二条。
/// 容量 256 远超实际（一次走带动作只发 1–2 条），所以通道**永不成为瓶颈**。
const EVENT_CAPACITY: usize = 256;

/// 引擎重建失败的原因。
#[derive(Debug)]
pub enum EngineHostError {
    /// 工程无法投影成引擎快照（没有主总线 / 路由图成环 / 模型校验失败）。
    Snapshot(SnapshotError),
    /// 还没有一代引擎（`reload` 从未成功过）⇒ 没有槽可以发布。
    NoEngine,
    /// **设备腿**打开失败（[`EngineHost::open_device`]）：设备不存在 / 采样率不支持 /
    /// 格式不是 `f32` / 要求独占模式 / 后端错误。
    ///
    /// 变体直接承载引擎侧的 [`DeviceError`]（**不**翻译、**不**吞掉）—— 引擎是设备
    /// 错误码的**唯一**权威（与 D25 同一条纪律：不发明第二套错误码）。
    /// 打开失败时**既有引擎一位不动**（控制驱动形态可继续用）。
    Device(DeviceError),
}

impl core::fmt::Display for EngineHostError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Snapshot(error) => write!(formatter, "引擎快照投影失败: {error}"),
            Self::NoEngine => write!(formatter, "还没有一代引擎 (reload 从未成功过)"),
            Self::Device(error) => write!(formatter, "音频设备打开失败: {error}"),
        }
    }
}

impl core::error::Error for EngineHostError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Snapshot(error) => Some(error),
            Self::NoEngine => None,
            Self::Device(error) => Some(error),
        }
    }
}

impl From<SnapshotError> for EngineHostError {
    fn from(value: SnapshotError) -> Self {
        Self::Snapshot(value)
    }
}

/// 工程改动的**轻量标记**：这一跳要不要重新发布快照。
///
/// 它取自撤销端口的**显示态**（[`crate::undo::UndoPort::display`] 的模型读数），只保留两个
/// 会随编辑变化的字段：
///
/// | 编辑 | 变化 |
/// | :--- | :--- |
/// | 一次提交（推子松手 / 静音 / 加音符 / 删音符 / AI 的工具调用） | `head` 换成新提交身份 |
/// | 一次撤销 / 重做 | `undone` 变，而 `head` **不变** |
///
/// 为什么不逐跳算 `UndoPort::fingerprint`：那是整份容器字节的 SHA-256，逐跳算它是浪费。
/// 标记只用来回答"要不要重新投影"，**不是**工程指纹 —— 指纹的唯一用途仍是判据的逐字节证据。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditMark {
    /// 活跃分支头（一次提交 ⇒ 新身份）。
    pub head: Option<EntityId>,
    /// 已撤销步数（一次撤销 / 重做 ⇒ 变化）。
    pub undone: usize,
}

impl EditMark {
    /// 由两个字段构造。
    #[must_use]
    pub const fn new(head: Option<EntityId>, undone: usize) -> Self {
        Self { head, undone }
    }

    /// 从撤销端口的显示态取标记（**唯一**来源；不要另算一份）。
    #[must_use]
    pub fn from_display(display: &crate::undo::UndoDisplay) -> Self {
        Self::new(display.head, display.undone)
    }
}

/// 一次心跳（[`EngineHost::heartbeat`]）的**字面读数**。
///
/// 单位：`published` / `pruned` 是**累计条数**（快照数，单调不减），`pending_len` /
/// `released` / `retired` / `retire_pending` 是**条数**（`released` / `retired` 是**本次**
/// 释放 / 排空的条数，其余是**此刻**的长度）。它们全部来自
/// `yeban-engine` 的快照槽与退役队列，不是从工程推算的。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeartbeatReadings {
    /// 累计发布次数（`SnapshotSlot::published()`）。
    pub published: u64,
    /// 心跳**之后**写者侧待回收清单的长度（`SnapshotSlot::pending_len()`）。
    pub pending_len: usize,
    /// 累计由 `SnapshotSlot::prune()` 释放的条数。
    pub pruned: u64,
    /// **本次** `prune` 释放的条数。
    pub released: usize,
    /// **本次** `drain` 真的 `Drop` 掉的旧快照条数。
    pub retired: usize,
    /// 心跳**之后**退役队列里还排着几条。
    pub retire_pending: usize,
}

/// 快照槽的计数读数（[`EngineHost::snapshot_counts`]，没有引擎时是 `None`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotCounts {
    /// 累计发布次数。
    pub published: u64,
    /// 写者侧待回收清单长度。
    pub pending_len: usize,
    /// 累计由 `prune` 释放的条数。
    pub pruned: u64,
    /// 纪元（每次发布 +1）。
    pub epoch: u64,
    /// 当前快照的版本号（`EngineSnapshot::revision`）。
    pub revision: u64,
    /// 退役队列里排着几条。
    pub retire_pending: usize,
    /// 累计真的 `Drop` 掉的旧快照条数。
    pub retire_dropped: u64,
}

/// 一次引擎重建的**读数**（判据与 `ui/reload_engine` 的回执都用它）。
///
/// 每个数字都来自 `EngineRuntime::stats()` 或快照本身，**没有一个是从工程里推算出来的**
/// ——"报告的数字"与"引擎实际做的事"必须是同一份事实。
///
/// 刻意**不** derive `Clone` / `PartialEq`：它是"一次动作的读数"，不是可以随便复制比较的值
/// 对象，而且它携带的 [`MeterCollector`] 是**所有权**（消费端只能被采纳一次）。
#[derive(Debug)]
pub struct EngineRebuild {
    /// 第几代引擎（1 起，单调递增）。
    pub generation: u64,
    /// 快照的模型层版本号（`EngineSnapshot::from_project` 的 `revision`）。
    pub revision: u64,
    /// 快照里的节点数（含主总线）。
    pub tracks: usize,
    /// 本次真的推了多少个量子。
    pub quanta: u64,
    /// 引擎累计的批量发布次数（**结构性契约**：应等于 `quanta`）。
    pub meter_bulk_publishes: u64,
    /// 引擎累计发布的电平帧数（应等于 `quanta × (非母线轨数 + 1)`）。
    pub meter_frames: u64,
    /// 容量耗尽导致未计量/被淘汰的次数（应为 0）。
    pub meter_capacity_drops: u64,
    /// 重建之后引擎上报的**走带读数**。
    ///
    /// `reload` **不改变**走带状态（见它的第 5 步说明）：新引擎沿用"自由跑"默认值
    /// ⇒ 这里通常是 `Playing` / tick 0。"加载即停住"由控制面显式调 [`EngineHost::stop`]。
    pub transport: TransportReading,
    /// 新引擎的电平队列**消费端** —— UI 线程必须采纳它（[`crate::meters::MeterRuntime::adopt`]）。
    pub collector: MeterCollector,
}

impl EngineRebuild {
    /// 非母线轨数（发布帧数的分母；快照里含主总线）。
    #[must_use]
    pub const fn non_bus_tracks(&self) -> usize {
        self.tracks.saturating_sub(1)
    }
}

/// 一次**设备腿打开**的读数（[`EngineHost::open_device`]）。
///
/// 每个数字都来自**引擎侧**：设备名来自 `device::OutputStreamHandle::device_name()`，
/// 采样率 / 通道数 / 块长接受与否来自 `device::NegotiatedConfig`，`carryover_quanta`
/// 来自新运行时自己的 `EngineStats::quanta`（不是"推算"的）。
///
/// 刻意**不**携带任何 `cpal::*` 类型（D19）：`sample_format` 恒为 `f32`（协商不成就返回
/// `DeviceError::UnsupportedSampleFormat`），因此这里不重复登记它；`buffer_size` 的
/// 原始形态（`BufferSize::Fixed/Default`）由 `fixed_block_accepted` + `block_frames`
/// 两个**纯量**表达。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceOpening {
    /// 设备的显示名（`device::OutputStreamHandle::device_name()` 直读）。
    pub device_name: String,
    /// 通道数（协商结果）。
    pub channels: u16,
    /// 采样率（Hz，协商结果）。
    pub sample_rate: u32,
    /// **实际**共享模式：当前实现恒为 [`ShareMode::Shared`]（独占不可用，见模块文档缺口 ②）。
    pub share_mode: ShareMode,
    /// 请求的每回调帧数是否被后端接受为 `BufferSize::Fixed`。
    pub fixed_block_accepted: bool,
    /// 请求的每回调帧数（`EngineConfig::block_frames`）。
    pub block_frames: u32,
    /// 把新运行时**带到当前走带位置**所推的量子数（交设备之前，控制面自己推的）。
    pub carryover_quanta: u64,
    /// 交接时引擎上报的走带位置（960 PPQ tick）—— 设备腿的起点。
    pub carryover_position_ticks: u64,
    /// 交接时引擎上报的走带状态（`Playing` ⇒ 已入队一条 `Play`，由设备回调生效）。
    pub carryover_state: TransportState,
}

/// **一代引擎的四个端点 + 渲染驱动**（[`EngineHost::reload`] 与
/// [`EngineHost::open_device`] 用**同一份**构造）。
///
/// 抽出来是为了让"设备腿"能先造一个**全新**的运行时，只有在
/// `device::open_output` **和** `play()` 都成功之后才换掉手里那一代 ——
/// 这样"打开设备失败"绝不会把正在用的引擎弄丢（优雅降级的结构性保证）。
struct FreshRuntime {
    runtime: EngineRuntime,
    retire: RetireQueue,
    events: EventSender,
    mirror: std::sync::Arc<TransportMirror>,
    collector: MeterCollector,
}

/// 用**当前快照槽**造一套全新端点（`EngineRuntime::new` 要求全部通道在开设备之前建立）。
fn fresh_runtime(slot: &std::sync::Arc<SnapshotSlot>) -> FreshRuntime {
    let (retire_producer, retire) = retire_channel(RETIRE_CAPACITY);
    let (events, event_receiver) = event_channel(EVENT_CAPACITY);
    let (publisher, collector) = meter_channel(DEFAULT_METER_CAPACITY);
    let runtime = EngineRuntime::new(slot, retire_producer, event_receiver, publisher);
    let mirror = std::sync::Arc::clone(runtime.transport_mirror());
    FreshRuntime {
        runtime,
        retire,
        events,
        mirror,
        collector,
    }
}

/// 一次走带动作的**记录**（走带接线判据的注入点）。
///
/// 它存在的唯一理由是"控件树里有这个元素"**不能**当作"回调接线了"的证据
/// （`docs/ledger/feature-alignment.md` 错位 7 登记过这个误判）。记录里每一个字段
/// 都来自**引擎**：命令本身、命令应用之后的状态与位置、为了让它生效推了几个量子。
///
/// 它是控制线程的私有缓冲（允许分配），不进实时路径。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportActionRecord {
    /// 发出的命令（`Stop`/`Pause` 会展开成一条记录；`stop_and_rewind` 是两条）。
    pub command: TransportCommand,
    /// 命令**应用之后**引擎上报的状态。
    pub state_after: TransportState,
    /// 命令应用之后引擎上报的位置（960 PPQ tick）。
    pub position_ticks_after: u64,
    /// 为了让命令在量子边界生效而推进的量子数（0 = 没有推进）。
    pub quanta_pumped: u64,
}

/// 引擎宿主：持有"当前这一代"的全部引擎侧对象。
///
/// 字段全部私有 + 只有 [`EngineHost::reload`] 能换掉它们 —— "换引擎"这条路径只有一条，
/// 不存在"改了快照但没换队列"这类半更新的形态。
#[derive(Default)]
pub struct EngineHost {
    generation: u64,
    /// **快照版本号**（`EngineSnapshot::revision` 的唯一来源）：每次
    /// [`EngineHost::reload`] 或 [`EngineHost::publish_project`] `+1`。
    ///
    /// 它与 `generation`（换代计数）**不是**同一个量：增量发布不换代，因此版本号会比
    /// 代数跑得快。既有判据只约束"连续两次 `reload` 的 `revision` 是 1、2"，在没有发布的
    /// 情况下两者逐步相等。
    snapshot_revision: u64,
    slot: Option<std::sync::Arc<SnapshotSlot>>,
    runtime: Option<EngineRuntime>,
    retire: Option<RetireQueue>,
    /// 最近一次发布所依据的工程标记（[`EngineHost::should_publish`] 的比较基准）。
    ///
    /// `None` = 还没有发布过（或刚 `reload` 过 ⇒ 由调用方用
    /// [`EngineHost::mark_applied`] 记下"当前快照已经反映哪个标记"）。
    last_mark: Option<EditMark>,
    /// UI → 引擎的**唯一**命令生产端（既有无锁 SPSC）。
    ///
    /// 它是 `Option` 只因为 `EngineHost::default()` 必须先存在（还没有引擎时没有通道）；
    /// 一旦 [`EngineHost::reload`] 成功，它就一定在（"有引擎必有通道"）。
    events: Option<EventSender>,
    /// RT → UI 的走带读数镜面（与 `EngineRuntime` 里那一份是**同一个** `Arc`）。
    transport_mirror: Option<std::sync::Arc<TransportMirror>>,
    /// **R55 自动化下发的读数**：累计写进 SPSC 的 `SetParam` 条数。
    automation_published: u64,
    /// **R55 自动化下发的读数**：因"装不下"（SPSC 满 / 批次上限）而没有写进去的条数。
    ///
    /// ⛔ 不许静默丢：没有写进去的值会在**下一跳**重算并重发（采样是幂等的：同一个
    /// `position_ticks` 给出同一批值），同时这里逐条计数 —— 控制面能看出"最近有没有丢"。
    automation_dropped: u64,
    /// **最近一份工程**（R68 修的缓存）：自动化采样在"标记没变"的那些跳也要进行。
    ///
    /// 生产路径（`src/main.rs` 的 16 ms 定时器）只在**标记变了**的那一跳才
    /// `try_project()`（那一跳的成本契约是"没改就不克隆工程"），其余各跳传给
    /// [`Self::publish_automation`] 的是 `None`。因此采样不能挂在入参上 ——
    /// 否则自动化**只在工程改动的那一跳生效**（静默丢失）。这里保留最近一份克隆，
    /// 克隆次数与既有成本契约**完全相同**（还是只在那一跳）。
    automation_project: Option<YebanProjectV1>,
    /// **设备腿**：已经交给真实声卡的那条流（[`EngineHost::open_device`]）。
    ///
    /// `Some` ⇒ `runtime` 是 `None`（`EngineRuntime` 已经 **move 进** cpal 的回调闭包，
    /// 它必须在**音频线程**上活着）；同一时刻只有一种驱动形态，见模块文档。
    /// 丢掉这个句柄 = cpal 的 `Drop` 停流 ⇒ 引擎回到"没有运行时"的形态。
    device: Option<OutputStreamHandle>,
    /// 走带动作日志（控制线程私有；判据的注入点，见 [`TransportActionRecord`]）。
    journal: Vec<TransportActionRecord>,
}

/// 手写 `Debug`：`EngineRuntime` / `SnapshotSlot` 刻意没有 `Debug`（它们持有裸指针与
/// 互斥量，打印它们只会误导），因此这里只报告"第几代、活没活着"。
impl core::fmt::Debug for EngineHost {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("EngineHost")
            .field("generation", &self.generation)
            .field("has_engine", &self.has_engine())
            .finish_non_exhaustive()
    }
}

impl EngineHost {
    /// 还没有引擎的宿主。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前是第几代（`0` = 从未成功重建过）。
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// 是否有一代活的引擎。
    ///
    /// **两种驱动形态都算"活着"**：控制驱动（`runtime` 在手里）与设备腿
    /// （`runtime` 在 cpal 回调线程上，见 [`EngineHost::open_device`]）。
    #[must_use]
    pub const fn has_engine(&self) -> bool {
        self.runtime.is_some() || self.device.is_some()
    }

    /// **设备腿**是否活跃（`open_device` 成功且流已 `play()`）。
    #[must_use]
    pub const fn device_active(&self) -> bool {
        self.device.is_some()
    }

    /// 设备腿那台设备的显示名（没有设备腿 ⇒ `None`）。
    #[must_use]
    pub fn device_name(&self) -> Option<&str> {
        self.device.as_ref().map(OutputStreamHandle::device_name)
    }

    /// 设备腿的**后端错误计数**（cpal 错误回调只做一次原子自增 [红线 7]；
    /// 没有设备腿 ⇒ `None`）。它由 `OutputStreamHandle::backend_errors()` 直读。
    #[must_use]
    pub fn device_backend_errors(&self) -> Option<u64> {
        self.device.as_ref().map(OutputStreamHandle::backend_errors)
    }

    /// 关掉设备腿（停流并丢掉那一代运行时）。返回"之前是否活跃"。
    ///
    /// 调用点：① [`EngineHost::reload`] 的开头（换代必然换驱动形态，同一时刻只允许
    /// 一个驱动）；② 窗口关闭时宿主整体 `Drop`（`OutputStreamHandle` 的 `Drop` 停流）。
    pub fn close_device(&mut self) -> bool {
        self.device.take().is_some()
    }

    /// **重建引擎**：投影新快照 → 建新队列 → 推 `quanta` 个量子 → 交还消费端。
    ///
    /// `quanta = 0` 是合法的（只重建、不推进）；但**失败必须被上报**：快照投影失败时
    /// 旧的一代**原样保留**（不做半更新），调用方拿到 `Err` 后界面上的引擎仍是上一代 ——
    /// 这比"清空成空引擎"诚实得多。
    ///
    /// ⚠ **换代必然先关掉设备腿**（同一时刻只有一个驱动形态）：新的一代是**控制驱动**的
    /// 一代，要重新交回设备请再调一次 [`EngineHost::open_device`]。关闭发生在投影**之前**
    /// 还是之后有区别吗？发生在**成功投影之后、装机之前**没有意义（失败要保留旧的一代，
    /// 而旧的一代可能就是设备腿）—— 因此关设备腿放在**投影成功之后**。
    ///
    /// # Errors
    ///
    /// [`EngineHostError::Snapshot`]：工程没有主总线 / 路由图成环 / 模型校验失败。
    pub fn reload(
        &mut self,
        project: &YebanProjectV1,
        quanta: u64,
    ) -> Result<EngineRebuild, EngineHostError> {
        let generation = self.generation.saturating_add(1);
        let revision = self.snapshot_revision.saturating_add(1);
        // 第 1 步：真的投影快照（失败在这里就返回，旧的一代不动 —— 包括设备腿）。
        let snapshot = EngineSnapshot::from_project(project, revision)?;
        let tracks = snapshot.tracks().len();
        let channels = snapshot.channels().max(1);

        // 第 1.5 步：投影已经成功 ⇒ 现在可以安全地放下旧驱动（旧设备腿在这里停流）。
        self.close_device();

        // 第 2 步：四样东西全是新的（与设备腿走**同一份**构造 [`fresh_runtime`]）。
        // 事件**生产端**保留下来（走带命令从这里进引擎）。
        let slot = SnapshotSlot::new(snapshot);
        let FreshRuntime {
            mut runtime,
            retire,
            events,
            mirror,
            collector,
        } = fresh_runtime(&slot);

        // 第 3 步：真的推量子。
        //
        // ⚠ **输出缓冲必须恰好一个"运行时量子"**（`[ARCH-DET-001]` 的 `DEFAULT_BLOCK_FRAMES`
        // = 128 帧 × 声道数）。`EngineRuntime::process_quantum` 会把**任意长度**的缓冲按
        // `DEFAULT_BLOCK_FRAMES` 切片，每片一个量子（每片一次批量发布）——
        // 而快照里 `ProjectAudioConfig::block_size` 是**模型层的声明值**（演示工程 = 256），
        // 两者是两个概念。第一版按"快照声明的块长"分配缓冲 ⇒ 一次调用推进了 **2** 个量子
        // （实测 `meter_bulk_publishes = 2 × quanta`），本机探针当场抓到。
        // 详见 `docs/ledger/app-mixer-notes.md` §6 的实测记录与 needs。
        let mut output = vec![0.0_f32; DEFAULT_BLOCK_FRAMES * usize::from(channels)];
        for _ in 0..quanta {
            runtime.process_quantum(&mut output, channels);
        }
        let stats = runtime.stats();

        // 第 4 步：装机（旧的一代在这里整体被替换；它持有的快照由 `SnapshotReader`
        // 的退役队列回收 —— 本线程内不会有积压，因此 `drain` 一次即可）。
        self.slot = Some(slot);
        self.runtime = Some(runtime);
        self.retire = Some(retire);
        self.events = Some(events);
        self.transport_mirror = Some(mirror);
        self.generation = generation;
        self.snapshot_revision = revision;
        // 新快照**就是** `project` 的投影 ⇒ "已经反映的标记"未知，由调用方用
        // [`EngineHost::mark_applied`] 记下（生产路径与判据都这么做）；在此之前
        // [`EngineHost::should_publish`] 恒为 `true`（宁多发一次，不漏发）。
        self.last_mark = None;
        self.drain_retired();

        // ⚠ 第 5 步**刻意不做**：`reload` 不碰走带状态。
        //
        // 为什么不在这里顺手发一条 `Stop`（本线第一版就是这么做的，被既有判据打红）：
        // `ui/reload_engine` 的契约是"推 `engine_quanta` 个量子"，而让命令在量子边界
        // 生效必须**再推一个量子**（命令是在 `render_block` 第 1 步出队的）——
        // 那会让报告里的 `quanta` 比调用方要的多 1（实测红在
        // `crates/yeban-app/tests/live_ui_mcp.rs` 的 `admin_reload_engine_...`）。
        // 因此"加载即停住"是**控制面的显式动作**：GUI 路径在 `reload` 之后调
        // [`EngineHost::stop`]，读回引擎读数再注入界面（见 `src/main.rs`）。
        let transport = self.transport();

        let rebuild = EngineRebuild {
            generation: self.generation,
            revision,
            tracks,
            quanta: stats.quanta,
            meter_bulk_publishes: stats.meter_bulk_publishes,
            meter_frames: stats.meter_frames,
            meter_capacity_drops: stats.meter_capacity_drops,
            transport,
            collector,
        };
        debug_assert_eq!(
            rebuild.meter_bulk_publishes, rebuild.quanta,
            "引擎的结构性契约: 每量子恰好一次电平批量发布 [ROAD-M2-007]"
        );
        Ok(rebuild)
    }

    // -----------------------------------------------------------------------
    // 设备腿：把这一代引擎交给真实声卡（真实时钟驱动音频线程）
    // -----------------------------------------------------------------------

    /// **把这一代引擎交给真实声卡**（`yeban_engine::device`；`ROAD-M2-001` 的接线那一半）。
    ///
    /// 顺序是契约 —— 每一步失败都留下一个**仍然可用**的产品：
    ///
    /// 1. **先造一套新端点**（[`fresh_runtime`]）：手里那一代（可能是控制驱动，也可能是
    ///    已经在跑的设备腿）**一位不动**；
    /// 2. **把新运行时带到当前走带位置**：往**新**事件通道发 `Stop` + `SeekTicks(位置)`，
    ///    再在新运行时上推**一个**量子让命令在块边界生效。少了这一步，新运行时是
    ///    "自由跑在 tick 0"的默认值 ⇒ 开设备会把播放位置重置成 0 —— 那等于**另造一条
    ///    未初始化的音频路径**，本票不许有。交接时在播放的话再补一条 `Play`：
    ///    它由**设备回调**在下一个块边界生效，因此**不丢帧**；
    /// 3. `device::open_output(&config, 新运行时)`：失败 ⇒ 丢掉新端点、返回
    ///    [`EngineHostError::Device`]，手里那一代继续按**控制驱动**形态工作；
    /// 4. `handle.play()`：失败 ⇒ 流被 `Drop`（停流），**仍然**不动手里那一代；
    /// 5. 只有 1–4 全成功才装机：换掉 `events` / `transport_mirror` / `retire` /
    ///    `collector`，`runtime` 置 `None`（它已经 **move 进** cpal 的回调闭包，必须在
    ///    音频线程上活着），`device = Some(handle)`。
    ///
    /// 返回 `(读数, 电平消费端)`：消费端**必须**被 UI 线程采纳（与
    /// [`EngineRebuild::collector`] 同一条纪律），否则设备腿发布的电平没有人抽。
    ///
    /// ⚠ 已经有一个活跃设备腿时调用它：旧腿在**装机那一步**才停（`close_device`）。
    /// ⚠ 之后 [`EngineHost::engine_stats`] 返回 `None`（运行时在音频线程上，见模块文档）。
    ///
    /// # Errors
    ///
    /// - [`EngineHostError::NoEngine`]：还没有一代引擎（没有快照槽可挂）；
    /// - [`EngineHostError::Device`]：设备不存在 / 采样率不支持 / 格式不是 `f32` /
    ///   要求独占模式 / 后端错误 —— **全部是返回值，绝不 panic**（`device.rs` 硬要求 #1）。
    pub fn open_device(
        &mut self,
        config: EngineConfig,
    ) -> Result<(DeviceOpening, MeterCollector), EngineHostError> {
        // 1) 新端点（旧的一代一位不动）。
        let Some(slot) = self.slot.as_ref().map(std::sync::Arc::clone) else {
            return Err(EngineHostError::NoEngine);
        };
        let channels = slot.current().channels().max(1);
        let before = self.transport();
        let FreshRuntime {
            mut runtime,
            retire,
            mut events,
            mirror,
            collector,
        } = fresh_runtime(&slot);

        // 2) 交接走带状态（只在**新**运行时上推量子）。
        let mut scratch = vec![0.0_f32; DEFAULT_BLOCK_FRAMES * usize::from(channels)];
        let carryover = [
            EngineEvent::Transport {
                command: TransportCommand::Stop,
            },
            EngineEvent::Transport {
                command: TransportCommand::SeekTicks(before.position_ticks),
            },
        ];
        let written = events.publish(&carryover);
        debug_assert_eq!(written, carryover.len(), "交接命令必须全部被通道接受");
        runtime.process_quantum(&mut scratch, channels);
        let carryover_stats = runtime.stats();
        if before.state.is_running() {
            let resume = [EngineEvent::Transport {
                command: TransportCommand::Play,
            }];
            let written = events.publish(&resume);
            debug_assert_eq!(written, 1, "交接的 Play 必须被通道接受");
        }

        // 3) 开设备（失败 ⇒ 手里那一代一位不动）。
        let handle = match yeban_engine::device::open_output(&config, runtime) {
            Ok(handle) => handle,
            Err(error) => return Err(EngineHostError::Device(error)),
        };
        // 4) 启流（失败 ⇒ 流被 Drop、手里那一代一位不动）。
        if let Err(error) = handle.play() {
            return Err(EngineHostError::Device(error));
        }
        let negotiated = *handle.negotiated();
        let device_name = handle.device_name().to_owned();

        // 5) 装机（只有到这里，旧的一代才被替换）。
        self.close_device();
        self.runtime = None;
        self.events = Some(events);
        self.transport_mirror = Some(mirror);
        self.retire = Some(retire);
        self.device = Some(handle);
        Ok((
            DeviceOpening {
                device_name,
                channels: negotiated.channels,
                sample_rate: negotiated.sample_rate,
                share_mode: negotiated.share_mode,
                fixed_block_accepted: negotiated.fixed_block_accepted,
                block_frames: config.block_frames,
                carryover_quanta: carryover_stats.quanta,
                carryover_position_ticks: before.position_ticks,
                carryover_state: before.state,
            },
            collector,
        ))
    }

    // -----------------------------------------------------------------------
    // 编辑 ⇒ 发声（增量发布 + 退役回收心跳）
    // -----------------------------------------------------------------------

    /// **增量发布**：把 `project` 投影成一份新快照，经 `SnapshotSlot::publish_arc`
    /// 原子换掉当前快照。
    ///
    /// 这是"编辑 ⇒ 发声"链路的发布点。它与 [`EngineHost::reload`] 的分工是刻意的：
    ///
    /// | | `reload` | `publish_project` |
    /// | :--- | :--- | :--- |
    /// | 槽 / 队列 / `EngineRuntime` | **全部重建** | 一位不动 |
    /// | 推进量子 | 推 `quanta` 个 | 不推 |
    /// | `quanta` / `meter_*` 契约 | 由它定义（`ui/reload_engine` 的判据钉住） | **不影响** |
    /// | 走带状态 | 不动（见 `reload` 第 5 步） | 不动 |
    ///
    /// 音频线程在**下一个块边界**（`SnapshotReader::begin_block`）就会看到新快照；
    /// 本函数自己跑在**控制线程**上（写者路径允许 `Mutex`，见 `snapshot.rs` 的
    /// `publish_arc` 文档），因此不触碰实时红线 [ARCH-RT-001 / MUST-GATE-001]。
    ///
    /// 旧快照进入写者侧待回收清单 ⇒ 调用方**必须**紧接着跑 [`EngineHost::heartbeat`]
    /// （60Hz），否则清单只涨不回收。
    ///
    /// # Errors
    ///
    /// [`EngineHostError::NoEngine`]（还没 `reload` 过）或
    /// [`EngineHostError::Snapshot`]（投影失败 ⇒ **当前快照一位不动**）。
    pub fn publish_project(
        &mut self,
        project: &YebanProjectV1,
        mark: EditMark,
    ) -> Result<u64, EngineHostError> {
        let Some(slot) = self.slot.as_ref() else {
            return Err(EngineHostError::NoEngine);
        };
        let revision = self.snapshot_revision.saturating_add(1);
        let snapshot = EngineSnapshot::from_project(project, revision)?;
        // 复用槽自己的写者路径（`publish` 就是本函数的包装）。
        slot.publish_arc(std::sync::Arc::new(snapshot));
        self.snapshot_revision = revision;
        self.last_mark = Some(mark);
        Ok(revision)
    }

    /// 这一跳要不要发布：`true` = 标记与"当前快照已经反映的"不同。
    ///
    /// `reload` 之后到第一次 `publish_project` / [`EngineHost::mark_applied`] 之前恒为 `true`。
    #[must_use]
    pub fn should_publish(&self, mark: EditMark) -> bool {
        self.last_mark != Some(mark)
    }

    /// 记下"当前快照已经反映这个标记"（`reload` 之后调用一次，避免开局多发一份）。
    pub fn mark_applied(&mut self, mark: EditMark) {
        self.last_mark = Some(mark);
    }

    /// **心跳**（控制线程 / 主线程，60Hz）：释放写者侧待回收清单，并排空退役队列。
    ///
    /// 两条回收路径都必须在这里有着落，否则"队列空了"可能只是"写者侧清单在涨"
    /// （`snapshot.rs` 的 `pruned` 文档原话）：
    ///
    /// 1. `SnapshotSlot::prune()` —— 释放"被 `ptr` 淘汰、且读者已确认不再引用"的写者侧强引用；
    /// 2. `RetireQueue::drain(RETIRE_CAPACITY)` —— 出队并 **Drop** 读者持有的旧快照。
    ///
    /// 两者都在**非实时线程**上做真正的 `dealloc`：音频线程只 `push`，从不 `drop`
    /// [ARCH-RT-002 / MUST-GATE-012]。没有引擎时返回全 0（不 panic）。
    pub fn heartbeat(&mut self) -> HeartbeatReadings {
        let Some(slot) = self.slot.as_ref() else {
            return HeartbeatReadings {
                published: 0,
                pending_len: 0,
                pruned: 0,
                released: 0,
                retired: 0,
                retire_pending: 0,
            };
        };
        let released = slot.prune();
        let retired = self
            .retire
            .as_mut()
            .map_or(0, |queue| queue.drain(RETIRE_CAPACITY));
        HeartbeatReadings {
            published: slot.published(),
            pending_len: slot.pending_len(),
            pruned: slot.pruned(),
            released,
            retired,
            retire_pending: self.retire.as_ref().map_or(0, RetireQueue::pending),
        }
    }

    /// **R55：把已开启的 `TrackVolume` 自动化泳道采样成 `SetParam` 并下发**。
    ///
    /// 返回**实际写进 SPSC 的条数**。设计口径（裁决 R55 / R61，逐条都有机械依据）：
    ///
    /// 1. **采样时点 = 音频时钟**：`self.transport().position_ticks`，也就是
    ///    [`TransportMirror`] 的读数（RT → UI 的原子 seqlock）⇒ ⛔ 一行墙钟都不用；
    /// 2. **每一跳（60 Hz）至多一批**：`≤ PARAM_SLOTS` 条装进定长数组、**一次**
    ///    `publish`。机械理由：渲染量子长度固定（[`DEFAULT_BLOCK_FRAMES`] = 128 帧 @
    ///    48 kHz ⇒ 375 量子/秒），而采样时点由 `position_ticks` **唯一确定** ⇒
    ///    同一条泳道在同一跳里只可能有一个值；
    /// 3. **不绕过平滑**：音频线程照旧走既有的 `yeban_engine::param` 目标表
    ///    （τ ≈ 5 ms 单极点）⇒ 本函数**不碰**引擎侧任何产线代码；
    /// 4. **换域**（槽位收的是**线性乘子**，见 `EngineEvent::SetParam` 的契约）：
    ///    `db_to_gain(自动化 dB) / db_to_gain(静态 dB)` —— 绝对值 ÷ 当前静态值。
    ///    `static_db` 非有限 ⇒ 静态增益为 0 ⇒ **直接取自动化绝对值**（R61-1）；
    ///    ⛔ 本函数绝不产出 `NaN`/`inf`（非有限或负的乘子一律**计数丢弃**）；
    /// 5. **不许静默丢**：`publish` 返回实际写入数，差额计进 `automation_dropped`，
    ///    并在下一跳自然重发（采样幂等）。读数见 [`Self::automation_counts`]。
    ///
    /// **为什么 60 Hz 够**（⚠ 改采样点的人先读这一段）：平滑时间常数 τ = 5 ms
    /// （`yeban_dsp::smoothing::DEFAULT_TIME_CONSTANT_S`），心跳 16.67 ms = **3.33 τ**
    /// ⇒ 每个新目标在下一跳之前收敛 `1 − e^(−3.33) ≈ 96.4 %` ⇒ 渲染参数以
    /// **≈5 ms 滞后 / 16.7 ms 阶梯**跟随曲线。60 Hz 的 Nyquist 是 30 Hz，而 5 ms
    /// 单极点拐点 ≈ 32 Hz ⇒ **保真上限 ≈ 10–15 Hz**：够推子曲线与渐强，
    /// **不够**音频速率调制（要更快就得移动采样点，那是另一条裁决）。
    ///
    /// 只处理 [`AutomationTarget::TrackVolume`]（音轨与主总线两个槽位）；
    /// `TrackPan` 由 P4 单独裁决（另票），这里**不**碰。
    /// 一跳最多下发的自动化事件数：**每条轨最多 3 条**（音量 + 声相左 + 声相右）。
    ///
    /// ⛔ 不用 [`PARAM_SLOTS`] 当上限：那个数是**引擎参数表的槽位数**（每条轨一个增益槽位）。
    /// 声相走引擎里**另一条独立通路**（`rt.rs` 的逐轨声相平滑对），不占参数表的槽位
    /// ⇒ 批次可以更大。**每跳仍然只有一次 `publish`（一条批）**，这条纪律不变。
    const fn automation_batch_capacity() -> usize {
        PARAM_SLOTS * 3
    }

    /// **把工程里的自动化泳道采样成 `SetParam` 事件并下发**（裁决 R55／R68／P4=(b)）。
    ///
    /// * **采样时点** = [`Self::transport`] 的 `position_ticks`（RT→UI 镜面 = **音频时钟**）
    ///   ⇒ 没有任何墙钟输入，同一个时点给出同一批值（幂等）；
    /// * **工程缓存**：`Some(project)` 更新缓存，`None` 跳用缓存 —— 生产的 16 ms 定时器
    ///   只在"标记变了"的那一跳才 `try_project()`（成本契约），⛔ 采样**不得**挂在入参上；
    /// * 一条轨最多 **3** 条事件：音量 1 条 + 声相 2 条（左/右**绝对**增益，语义是**替换**）；
    /// * 声相值由引擎公开的 [`pan_gains`] 与 [`PanLaw::from_model`] 在**控制侧**算好
    ///   （超越函数⛔ 不进音频线程），并 `max(0.0)` 钳位（硬右的左增益实测是负的）；
    /// * 返回**实际写入**的事件条数（没写进去的记进 [`Self::automation_counts`] 的第二个数）。
    ///   ⛔ **不加 `#[must_use]`**：生产调用点（`host.rs` 的 `tick`）刻意忽略返回值，
    ///   加了会在 `-D warnings` 下把整条腿弄红。
    pub fn publish_automation(&mut self, project: Option<&YebanProjectV1>) -> usize {
        // **工程缓存（R68 修的那一处）**：生产路径只在"标记变了"的那一跳才 `try_project()`
        // ⇒ 其余各跳传的是 `None`。若采样直接挂在入参上，自动化就**只在工程改动的那一跳
        // 生效** —— 那不是"没采集"，是**静默丢失**。因此：`Some(..)` 时更新缓存，
        // 之后每一跳都用缓存里的那一份采样（克隆仍只发生在本来就克隆的那一跳）。
        if let Some(project) = project {
            self.automation_project = Some(project.clone());
        }
        let Some(project) = self.automation_project.as_ref() else {
            return 0;
        };
        let tick = self.transport().position_ticks;
        let batch_capacity = Self::automation_batch_capacity();
        let mut batch = [EngineEvent::Idle; Self::automation_batch_capacity()];
        let mut len = 0usize;
        let mut dropped = 0u64;
        for track in project.tracks.values() {
            let target = AutomationTarget::TrackVolume { track_id: track.id };
            let Some(lane) = project.automation_lane(&target) else {
                continue;
            };
            if !lane.read_enabled {
                continue;
            }
            let Ok(Some(automated_db)) = project.automation_value_at(&target, tick) else {
                continue;
            };
            let static_gain = db_to_gain(track.volume_db);
            let gain = if static_gain.is_finite() && static_gain > 0.0 {
                db_to_gain(automated_db) / static_gain
            } else {
                // R61-1：静态不可闻（非有限 / 0）⇒ 直接用自动化的绝对值。
                db_to_gain(automated_db)
            };
            if !gain.is_finite() || gain < 0.0 {
                dropped += 1;
                continue;
            }
            let slot = if track.id == project.master_bus_track_id {
                MASTER_GAIN_SLOT
            } else {
                TRACK_GAIN_SLOT
            };
            if len == batch_capacity {
                dropped += 1;
                continue;
            }
            batch[len] = EngineEvent::SetParam {
                target: ParamAddress::new(track.id, slot),
                value: gain,
            };
            len += 1;

            // --- 声相（裁决 P4=(b)）：一条轨**两条**事件（左 / 右**绝对**增益）---
            //
            // 语义是**替换**：泳道给的是声相位置，这里的 `pan_gains` 就是引擎在快照边界
            // 用的同一个公开函数（同一个衰减律，`PanLaw::from_model(工程)`）⇒ 值域与
            // 引擎内部一致，⛔ 不做任何分量相除。
            // ⚠ 母线**不**进声相表（引擎的 `armed_pan_gains` 明确排除它）⇒ 这里也跳过，
            // 否则两条事件会被判未映射（计数），读数上像缺陷。
            if track.id != project.master_bus_track_id {
                let pan_target = AutomationTarget::TrackPan { track_id: track.id };
                if let Some(lane) = project.automation_lane(&pan_target)
                    && lane.read_enabled
                    && let Ok(Some(pan)) = project.automation_value_at(&pan_target, tick)
                {
                    let (left, right) =
                        pan_gains(pan, PanLaw::from_model(project.audio_config.pan_law));
                    // ⚠ **必须 clamp 到 `>= 0`**：硬右时 `pan_gains(1.0).0` 实测是
                    // `-4.371139e-8`（f32 的 `π/2` 不精确，见 `mixer` 的端点判据）⇒
                    // 不 clamp 会被引擎"有限且非负"的值规则拒掉（静默丢失）。
                    for (slot, value) in [
                        (TRACK_PAN_LEFT_SLOT, left.max(0.0)),
                        (TRACK_PAN_RIGHT_SLOT, right.max(0.0)),
                    ] {
                        if !value.is_finite() || len == batch_capacity {
                            dropped += 1;
                            continue;
                        }
                        batch[len] = EngineEvent::SetParam {
                            target: ParamAddress::new(track.id, slot),
                            value,
                        };
                        len += 1;
                    }
                }
            }
        }
        let written = if len == 0 {
            0
        } else {
            self.events
                .as_mut()
                .map_or(0, |sender| sender.publish(&batch[..len]))
        };
        self.automation_published = self.automation_published.saturating_add(written as u64);
        self.automation_dropped = self
            .automation_dropped
            .saturating_add(dropped + (len - written) as u64);
        written
    }

    /// **R55 的读数**：`(累计写进 SPSC 的条数, 累计因装不下而丢弃的条数)`。
    #[must_use]
    pub const fn automation_counts(&self) -> (u64, u64) {
        (self.automation_published, self.automation_dropped)
    }

    /// 当前快照槽的计数（[`HeartbeatReadings`] 之外的静止读数；没有引擎时 `None`）。
    #[must_use]
    pub fn snapshot_counts(&self) -> Option<SnapshotCounts> {
        self.slot.as_ref().map(|slot| SnapshotCounts {
            published: slot.published(),
            pending_len: slot.pending_len(),
            pruned: slot.pruned(),
            epoch: slot.epoch(),
            revision: self.snapshot_revision,
            retire_pending: self.retire.as_ref().map_or(0, RetireQueue::pending),
            retire_dropped: self.retire.as_ref().map_or(0, RetireQueue::dropped),
        })
    }

    /// 当前快照的一份 `Arc`（**控制线程**读：`SnapshotSlot::current()` 会在写者锁上短暂等待）。
    ///
    /// 判据与诊断的读口。音频线程走的仍然是
    /// [`yeban_engine::snapshot::SnapshotReader::begin_block`] —— 本函数不参与那条路径。
    #[must_use]
    pub fn current_snapshot(&self) -> Option<std::sync::Arc<EngineSnapshot>> {
        self.slot.as_ref().map(|slot| slot.current())
    }

    /// 引擎累计统计（没有引擎时 `None`）。
    ///
    /// `EngineStats::snapshot_switches` 是"音频读路径真的换了快照"的见证
    /// （它是 `SnapshotReader::begin_block` 里计的数）。
    ///
    /// ⚠ **设备腿活跃时返回 `None`**：那一代的 `EngineRuntime` 已经 move 进 cpal 回调，
    /// 控制线程拿不到它。走带读数（`TransportMirror`）、电平（独立 SPSC）与快照槽计数
    /// （[`EngineHost::snapshot_counts`]）不受影响。要在这里也读到统计，需要一份
    /// "跨线程只读的 `EngineStats` 镜像" —— **本票不做**（登记为 needs，不发明第二份
    /// 统计来源）。
    #[must_use]
    pub fn engine_stats(&self) -> Option<EngineStats> {
        self.runtime.as_ref().map(EngineRuntime::stats)
    }

    // -----------------------------------------------------------------------
    // 走带（`line/transport-engine`）
    // -----------------------------------------------------------------------

    /// 当前走带读数（**来自引擎**；还没有引擎时是中性冷值）。
    ///
    /// 它读的是 [`TransportMirror`]（原子 seqlock），不是界面自己的状态 ——
    /// "播放中"这个显示值只有一个事实源。
    #[must_use]
    pub fn transport(&self) -> TransportReading {
        self.transport_mirror
            .as_ref()
            .map_or_else(TransportReading::cold, |mirror| mirror.read())
    }

    /// 走带动作日志（判据的注入点；见 [`TransportActionRecord`]）。
    #[must_use]
    pub fn transport_journal(&self) -> &[TransportActionRecord] {
        &self.journal
    }

    /// 是否有一代活着的引擎（走带命令只有在这种情况下才发得出去）。
    ///
    /// ⚠ 设备腿活跃时 `runtime` 是 `None`（它在 cpal 回调线程上），因此这里的判据
    /// **不能**只看 `runtime` —— 否则"接了声卡"会让走带整体失能。
    #[must_use]
    pub fn transport_ready(&self) -> bool {
        self.has_engine() && self.events.is_some()
    }

    /// 发一批走带命令，返回**命令应用之后**的读数。
    ///
    /// 为什么"发完还要让边界到来"：命令是在**量子边界**由实时侧出队应用的
    /// （`EngineRuntime::render_block` 第 1 步）。没有设备回调时，唯一让边界到来的
    /// 方式就是控制面显式推量子。
    ///
    /// 两种驱动形态在这里**分开**（本票新增；见模块文档的形态表）：
    ///
    /// | 形态 | 让命令生效的方式 | `quanta_pumped` |
    /// | :--- | :--- | :--- |
    /// | 控制驱动（无声卡 / 开设备失败） | 控制面 `pump(1)`（既有形态，一位没变） | `1` |
    /// | 设备腿（有声卡） | **设备回调**在下一个块边界出队（等 `commands_applied` 前进） | `0` |
    ///
    /// 设备腿那一支**必须等确认**：不等就会把"命令之前"的状态画回界面（UI 的假读数）。
    /// 等待是**有界的**（[`DEVICE_COMMAND_TIMEOUT`]，250 ms ≈ 回调周期的 25–80 倍），
    /// 超时**不换来源**，如实返回引擎此刻的读数。
    ///
    /// 控制驱动的**已知代价**：走带位置会因此前进一个运行时量子（128 帧）—— 那是
    /// "没有声卡"形态的代价，不是走带的语义。设备腿形态没有这个代价（推进由真实时钟给出）。
    pub fn send_transport(&mut self, commands: &[TransportCommand]) -> TransportReading {
        if !self.transport_ready() {
            return TransportReading::cold();
        }
        let events: Vec<EngineEvent> = commands
            .iter()
            .map(|command| EngineEvent::Transport { command: *command })
            .collect();
        let expected = commands.len() as u64;
        if let Some(sender) = self.events.as_mut() {
            sender.publish(&events);
        }
        let pumped = if self.runtime.is_some() {
            self.pump(1)
        } else {
            // 设备腿：等引擎自己的读数确认（`TransportReading::commands_applied`）。
            let _confirmed = self.await_device_commands(expected);
            0
        };
        let reading = self.transport();
        for command in commands {
            self.journal.push(TransportActionRecord {
                command: *command,
                state_after: reading.state,
                position_ticks_after: reading.position_ticks,
                quanta_pumped: pumped,
            });
        }
        reading
    }

    /// 等设备回调把**至少** `expected` 条命令应用掉（控制线程，有界轮询）。
    ///
    /// 判据是引擎自己的读数（[`TransportReading::commands_applied`]）前进，不是猜意图：
    /// 每条命令（含幂等的 `Play`/`Stop`）都会让那个计数器 `+1`（`Transport::apply`）。
    /// 返回"是否在超时之前确认"。超时不是错误：调用方仍会读到引擎此刻的**真实**读数。
    fn await_device_commands(&self, expected: u64) -> bool {
        let Some(mirror) = self.transport_mirror.as_ref() else {
            return false;
        };
        let target = mirror.read().commands_applied.saturating_add(expected);
        let deadline = std::time::Instant::now() + DEVICE_COMMAND_TIMEOUT;
        loop {
            if mirror.read().commands_applied >= target {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(DEVICE_COMMAND_POLL);
        }
    }

    /// 推 `quanta` 个运行时量子（每个量子 [`DEFAULT_BLOCK_FRAMES`] 帧），返回实际推进数。
    ///
    /// 它是"走带路径的控制线程推量子"：推完顺带 `drain` 一次退役队列（历史形态，见
    /// [`Self::drive_audio`]）。没有引擎时返回 0（不 panic、不假装推进过）。
    ///
    /// ⚠ **设备腿活跃时恒返回 0**：那一代的 `EngineRuntime` 在 cpal 回调线程上，
    /// 控制面推不了、也**不许**推（推了就是第二条音频路径）。推进由真实时钟负责。
    pub fn pump(&mut self, quanta: u64) -> u64 {
        let driven = self.drive_audio(quanta);
        self.drain_retired();
        driven
    }

    /// 只推量子、**不**排空退役队列（返回实际推进数；没有引擎时 0）。
    ///
    /// 它模拟的是**设备回调**：回调在每个块边界只做"读者无锁换快照 + 把旧快照
    /// `push` 进退役队列"（[`crate::engine_host`] 模块文档的接线图），**释放是心跳的事**
    /// （[`Self::heartbeat`]）。生产 GUI 里音频侧由设备时钟驱动、控制线程不再调
    /// `pump`，因此"读者换了快照、但还没人回收"这个窗口是真实存在的 —— 判据用本函数
    /// 把它造出来，用来证明心跳的 `drain` 腿有牙。
    ///
    /// ⚠ 设备腿活跃时返回 0（运行时不在控制线程手里）。
    pub fn drive_audio(&mut self, quanta: u64) -> u64 {
        let Some(runtime) = self.runtime.as_mut() else {
            return 0;
        };
        let channels = self
            .slot
            .as_ref()
            .map_or(2, |slot| slot.current().channels().max(1));
        let mut output = vec![0.0_f32; DEFAULT_BLOCK_FRAMES * usize::from(channels)];
        for _ in 0..quanta {
            runtime.process_quantum(&mut output, channels);
        }
        quanta
    }

    /// 播放（**位置保留** ⇒ 停住之后从这里继续）。
    pub fn play(&mut self) -> TransportReading {
        self.send_transport(&[TransportCommand::Play])
    }

    /// 停住（**位置保留**）。"回到起始点"用 [`Self::stop_and_rewind`]。
    pub fn stop(&mut self) -> TransportReading {
        self.send_transport(&[TransportCommand::Stop])
    }

    /// 停止并回到 tick 0：**一条批量里的两条命令**（`Stop` + `SeekTicks(0)`），
    /// 因此它们在**同一个量子边界**按 FIFO 一起生效 —— 不存在"停住了但还在半路"的中间态。
    pub fn stop_and_rewind(&mut self) -> TransportReading {
        self.send_transport(&[TransportCommand::Stop, TransportCommand::SeekTicks(0)])
    }

    /// 播放 / 停住的切换（界面 `toggle-play` 的落点）。
    ///
    /// 判断依据是**引擎读数**（不是界面属性）：
    /// 读数说 Playing ⇒ 发 `Stop`；否则发 `Play`。
    pub fn toggle_play(&mut self) -> TransportReading {
        if self.transport().state.is_running() {
            self.stop()
        } else {
            self.play()
        }
    }

    /// 定位到 `tick`（960 PPQ），播放状态不变。
    pub fn seek(&mut self, tick: u64) -> TransportReading {
        self.send_transport(&[TransportCommand::SeekTicks(tick)])
    }

    /// 主线程腿：把退役队列里的旧快照真正 **Drop** 掉（[`RetireQueue::drain`]）。
    ///
    /// 返回本次释放的条数。生产路径的正规调用点是 60Hz 心跳（[`Self::heartbeat`]）；
    /// 保留本函数是因为 [`Self::pump`]（走带路径）与 [`Self::reload`] 末尾也要它。
    pub fn drain_retired(&mut self) -> usize {
        self.retire
            .as_mut()
            .map_or(0, |queue| queue.drain(RETIRE_CAPACITY))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_model::project::YebanProjectV1;
    use yeban_model::{
        AutomationLane, AutomationPoint, AutomationTarget, AutomationWriteMode, CurveType,
        EntityId, Op,
    };

    use crate::bridge::{ViewState, demo_project};
    use crate::undo::{UndoPort, UndoSession};

    /// 会话打开时刻（与 app / MCP 侧既有夹具同一个常量）。
    const NOW: u64 = 1_760_000_000_000;

    /// 判据 1：重建真的推量子，且**每量子恰好一次**批量发布、帧数 = 非母线轨 + 1。
    #[test]
    fn reload_runs_quanta_and_respects_the_publishing_contract() {
        let project = demo_project();
        let view = ViewState::from_project(&project).expect("投影");
        let mut host = EngineHost::new();
        assert_eq!(host.generation(), 0);
        assert!(!host.has_engine());

        let rebuild = host.reload(&project, 16).expect("重建");
        assert_eq!(rebuild.generation, 1);
        assert_eq!(rebuild.revision, 1);
        assert_eq!(rebuild.quanta, 16);
        assert_eq!(rebuild.meter_bulk_publishes, 16, "每量子恰好一次批量发布");
        // 非母线轨（投影里的 tracks）+ 母线 1 条。
        assert_eq!(rebuild.non_bus_tracks(), view.tracks.len());
        assert_eq!(
            rebuild.meter_frames,
            16 * (view.tracks.len() as u64 + 1),
            "帧数 = 量子数 × (非母线轨数 + 1)"
        );
        assert_eq!(rebuild.meter_capacity_drops, 0);
        assert!(host.has_engine());
        assert_eq!(host.generation(), 1);
    }

    /// 判据 2：连着重建两次 ⇒ 代数是 1、2，且每次都给一条**新**队列。
    #[test]
    fn a_second_reload_advances_the_generation_and_hands_out_a_new_queue() {
        let project = demo_project();
        let mut host = EngineHost::new();
        let first = host.reload(&project, 4).expect("第一代");
        assert_eq!(first.generation, 1);
        assert_eq!(first.revision, 1);
        assert_eq!(first.quanta, 4);

        let second = host.reload(&project, 2).expect("第二代");
        assert_eq!(second.generation, 2);
        assert_eq!(second.revision, 2);
        assert_eq!(second.quanta, 2, "统计是**这一代**的, 不是累加的");
        assert_eq!(second.meter_bulk_publishes, 2);
        assert_eq!(
            second.meter_frames,
            2 * (second.non_bus_tracks() as u64 + 1)
        );
    }

    /// 判据 3：**失败必须上报**，而且旧的一代原样保留（不做半更新）。
    #[test]
    fn a_project_without_a_master_bus_is_an_explicit_error_and_keeps_the_old_engine() {
        let good = demo_project();
        let mut host = EngineHost::new();
        host.reload(&good, 3).expect("先装一代好的");
        assert_eq!(host.generation(), 1);

        // `YebanProjectV1::default()` 的 `master_bus_track_id` 是 nil ⇒ NoMasterBus。
        let error = host
            .reload(&YebanProjectV1::default(), 3)
            .expect_err("没有主总线必须报错");
        assert!(
            matches!(error, EngineHostError::Snapshot(SnapshotError::NoMasterBus)),
            "实际错误: {error:?}"
        );
        assert_eq!(host.generation(), 1, "失败不得推进代数");
        assert!(host.has_engine(), "失败不得把旧引擎清空");
    }

    /// 判据 4：`quanta = 0` 是合法的（只重建、不推进）—— 报告里就是 0，不是 panic。
    #[test]
    fn zero_quanta_rebuilds_without_advancing() {
        let mut host = EngineHost::new();
        let rebuild = host.reload(&demo_project(), 0).expect("重建");
        assert_eq!(rebuild.quanta, 0);
        assert_eq!(rebuild.meter_bulk_publishes, 0);
        assert_eq!(rebuild.meter_frames, 0);
        assert_eq!(rebuild.generation, 1);
        assert!(host.has_engine());
    }

    /// 判据 5：**运行时量子长度 = `[ARCH-DET-001]` 的 `DEFAULT_BLOCK_FRAMES`**，
    /// 而快照声明的 `block_size` 是模型层的另一个概念（演示工程声明 256）。
    ///
    /// 这条判据把第一版的真错误钉住：按"快照声明的块长"分配输出缓冲 ⇒ 一次
    /// `process_quantum` 推进 2 个量子 ⇒ `meter_bulk_publishes` 翻倍。
    #[test]
    fn the_runtime_quantum_is_the_det_block_not_the_declared_block_size() {
        let project = demo_project();
        let snapshot = EngineSnapshot::from_project(&project, 1).expect("快照");
        assert!(snapshot.block_size_matches_enum(), "声明值必须是合法枚举");
        assert_eq!(snapshot.channels(), 2, "引擎快照恒以立体声投影");
        assert_eq!(
            yeban_engine::block::DEFAULT_BLOCK_FRAMES,
            128,
            "运行时量子长度是 [ARCH-DET-001] 的固定 128"
        );
        assert_eq!(
            snapshot.block_frames(),
            256,
            "演示工程声明的 block_size 是 256（与运行时量子不是同一个数）"
        );
        // 一次 `reload(.., 1)` 必须恰好推进 **1** 个量子（而不是 2）。
        let mut host = EngineHost::new();
        let rebuild = host.reload(&project, 1).expect("重建");
        assert_eq!(rebuild.quanta, 1);
        assert_eq!(rebuild.meter_bulk_publishes, 1);
        assert_eq!(rebuild.meter_frames, rebuild.non_bus_tracks() as u64 + 1);
    }

    /// 判据 6：退役队列被 drain（返回 0 而不是 panic；本线程内不产生退役项）。
    #[test]
    fn retire_queue_is_drained_without_panicking() {
        let mut host = EngineHost::new();
        assert_eq!(host.drain_retired(), 0, "没有引擎时是 0");
        host.reload(&demo_project(), 2).expect("重建");
        assert_eq!(host.drain_retired(), 0, "同线程内换快照不产生退役项");
    }

    // -----------------------------------------------------------------------
    // 编辑 ⇒ 发声（本票新增的判据 7–10）
    // -----------------------------------------------------------------------

    /// 判据 7–9 的夹具：一条真的撤销会话 + 一代真的引擎 + 一条真的轨道。
    ///
    /// 返回 `(端口, 引擎, 轨道身份, 起点音量 dB)`。端口是 `UndoPort`（**混音台写入面的
    /// 同一条路径**，`host::wire_mixer_edit` 用的就是它）；引擎是 `reload(.., 0)`
    /// 出来的一代（0 个量子 ⇒ 不空转），并且已经用 [`EngineHost::mark_applied`] 记下
    /// "初始快照已经反映开局标记"。
    fn mixer_fixture() -> (UndoPort, EngineHost, EntityId, f32) {
        let project = demo_project();
        let track_id = *project.tracks.keys().next().expect("演示工程至少一条轨道");
        let volume_db = project.tracks[&track_id].volume_db;
        let port = UndoPort::new(
            UndoSession::open("<判据>", "yeban-app", project.clone(), NOW).expect("打开撤销会话"),
        );
        let mut host = EngineHost::new();
        host.reload(&project, 0).expect("重建引擎");
        host.mark_applied(EditMark::from_display(&port.display()));
        (port, host, track_id, volume_db)
    }

    /// 经 `UndoPort::commit_ops` 改一条轨道的音量（**混音台推子松手的同一条路径**）。
    fn commit_volume(port: &UndoPort, track_id: EntityId, old_val: f32, new_val: f32, step: u64) {
        port.commit_ops(
            NOW + step + 1,
            "mixer: set track volume",
            vec![Op::SetParam {
                target: AutomationTarget::TrackVolume { track_id },
                old_val,
                new_val,
            }],
        )
        .expect("音量提交必须成功");
    }

    /// 判据 7（**编辑 ⇒ 发声**，本票的端到端）：经 `UndoPort::commit_ops` 改一个混音参数
    /// ⇒ 增量发布 ⇒ 快照里那**一个字段**真的换了，而且音频读路径在下一个块边界换了快照。
    ///
    /// 起点 / 终点都是**字面读数**：`EngineSnapshot::track(id).volume_db()`（dB，f32）。
    #[test]
    fn a_mixer_edit_reaches_the_engine_snapshot_and_the_reader_switches() {
        let (port, mut host, track_id, before_db) = mixer_fixture();
        let before = host
            .current_snapshot()
            .expect("有引擎必有快照")
            .track(&track_id)
            .expect("快照里有这条轨道")
            .volume_db();
        assert_eq!(before, before_db, "起点：快照里的音量 = 工程里的音量");

        let new_db = before_db - 6.0;
        commit_volume(&port, track_id, before_db, new_db, 0);
        assert_eq!(
            port.project().tracks[&track_id].volume_db,
            new_db,
            "工程真的改了（模型侧读数）"
        );

        // —— 控制线程的一跳：按需发布 + 回收（生产 60Hz 定时器调的就是这三步）——
        let mark = EditMark::from_display(&port.display());
        assert!(host.should_publish(mark), "工程改过 ⇒ 这一跳必须发布");
        let revision = host
            .publish_project(&port.project(), mark)
            .expect("增量发布");
        assert_eq!(revision, 2, "增量发布的版本号 = 快照版本号 + 1");
        assert_eq!(
            host.generation(),
            1,
            "增量发布**不**推进代数（不是 reload）"
        );
        let tick = host.heartbeat();
        assert_eq!(tick.published, 2, "槽里累计发布 2 次（reload 1 + 增量 1）");
        assert_eq!(
            tick.pending_len, 1,
            "读者还没走过这个纪元 ⇒ 那一份必须保持存活（`prune` 不得提前释放）"
        );
        assert_eq!(tick.released, 0, "读者没确认之前一条都不许释放");
        assert!(!host.should_publish(mark), "同一个标记不会再发第二次");

        let after = host
            .current_snapshot()
            .expect("有引擎必有快照")
            .track(&track_id)
            .expect("快照里有这条轨道")
            .volume_db();
        assert_ne!(
            after, before,
            "改音量之后快照里那一个字段必须不同 —— 这就是「编辑 ⇒ 发声」的断点"
        );
        assert_eq!(after, new_db, "快照读到的就是新的那一版");

        // 音频读路径（`begin_block`）真的换了快照：`snapshot_switches` 由它计数。
        let switches_before = host
            .engine_stats()
            .expect("有引擎必有统计")
            .snapshot_switches;
        assert_eq!(host.drive_audio(1), 1, "推一个量子");
        assert_eq!(
            host.engine_stats().expect("统计").snapshot_switches,
            switches_before + 1,
            "下一个块边界必须换到新快照"
        );
        let tick = host.heartbeat();
        assert_eq!(tick.retired, 1, "读者交出的旧快照由心跳排空并 Drop");
        assert_eq!(
            tick.released, 1,
            "读者走过之后，写者侧那一份也在同一跳被回收"
        );
        assert_eq!(tick.pending_len, 0, "回收之后清单回到基线 0");
        // 交付报告要的是**字面读数**：`--nocapture` 时这一行会打出来。
        println!(
            "[engine-snapshot] 音量 {before:.1} → {after:.1} dB; published={} pending_len={} \
             pruned={} switches={}",
            tick.published,
            tick.pending_len,
            tick.pruned,
            host.engine_stats().expect("统计").snapshot_switches,
        );
    }

    /// 判据 8（**发布了必须回收**，本票的回归基线）：连续 100 次编辑。
    ///
    /// 逐跳探针（不是只读末值）：每一跳都断言"这一跳发布了 1 份、释放了 1 份、
    /// 清单回到基线"。因此"某一跳不发布"或"某一跳不回收"都会在**那一跳**当场变红。
    ///
    /// 判据阈值与理由：`pending_len == 0`（**严格等于**基线 0，无容差）。本夹具在每次
    /// 发布之后、心跳之前恰好推**一个**量子 ⇒ 读者的 `reader_done` 覆盖该条目的
    /// `retired_at`（这正是 `prune` 的释放条件）⇒ 任何非零都意味着"发了没回收"。
    /// 生产形态的界是"设备时钟让读者落后多少"，不是本判据的容差。
    #[test]
    fn a_hundred_edits_leave_nothing_pending_on_the_writer_side() {
        const EDITS: u32 = 100;
        let (port, mut host, track_id, start_db) = mixer_fixture();
        let baseline = host.snapshot_counts().expect("有引擎必有计数");
        assert_eq!(baseline.published, 1, "reload 之后槽里只有那一份初始快照");
        assert_eq!(baseline.pending_len, 0, "基线：写者侧清单为空");
        assert_eq!(baseline.pruned, 0, "基线：还没回收过");
        assert_eq!(baseline.retire_dropped, 0, "基线：还没 Drop 过旧快照");

        let mut current_db = start_db;
        let mut max_pending = 0_usize;
        for step in 0..EDITS {
            let next_db = current_db - 0.5;
            commit_volume(&port, track_id, current_db, next_db, u64::from(step));
            current_db = next_db;

            let mark = EditMark::from_display(&port.display());
            assert!(
                host.should_publish(mark),
                "第 {step} 跳：提交之后标记必须变（否则这一跳不会发布）"
            );
            host.publish_project(&port.project(), mark)
                .expect("增量发布");
            // 设备回调：读者在块边界换快照（旧快照进退役队列），**不**回收。
            assert_eq!(host.drive_audio(1), 1, "第 {step} 跳：推一个量子");

            let tick = host.heartbeat();
            assert_eq!(
                tick.released, 1,
                "第 {step} 跳：心跳必须释放写者侧那一条（发布了不回收 = 这里红）"
            );
            assert_eq!(
                tick.pending_len, 0,
                "第 {step} 跳：写者侧清单必须回到基线 0"
            );
            assert_eq!(
                tick.retired, 1,
                "第 {step} 跳：心跳必须排空退役队列并 Drop（不排空 = 这里红）"
            );
            assert_eq!(tick.retire_pending, 0, "第 {step} 跳：退役队列必须为空");
            max_pending = max_pending.max(tick.pending_len);
        }

        let end = host.snapshot_counts().expect("有引擎必有计数");
        assert_eq!(
            end.published,
            1 + u64::from(EDITS),
            "发布次数 = 基线 1 + 编辑 {EDITS} 次"
        );
        assert_eq!(
            end.pruned,
            u64::from(EDITS),
            "每一次发布都在同一跳被回收（累计释放条数）"
        );
        assert_eq!(end.pending_len, 0, "编辑结束：写者侧清单回到基线");
        assert_eq!(
            end.retire_dropped,
            u64::from(EDITS),
            "读者交出的每一份旧快照都真的 Drop 了"
        );
        assert_eq!(end.retire_pending, 0, "编辑结束：退役队列为空");
        assert_eq!(max_pending, 0, "全程逐跳没有一次积压（不是只看末值）");
        // 音频读路径真的换了 100 次快照（不是"发了但读者看不见"）。
        assert_eq!(
            host.engine_stats().expect("统计").snapshot_switches,
            u64::from(EDITS),
            "每一次编辑之后的下一个块边界都换到了新快照"
        );
        // 末值探针：最后那一版仍在快照里（逐跳改的是不同的值）。
        assert_eq!(
            host.current_snapshot()
                .expect("快照")
                .track(&track_id)
                .expect("轨道")
                .volume_db(),
            current_db,
            "第 {EDITS} 次编辑的字段值必须留在当前快照里"
        );
        assert_eq!(current_db, start_db - 0.5 * EDITS as f32);
        // 交付报告要的是**字面读数**：`--nocapture` 时这一行会打出来。
        println!(
            "[snapshot-heartbeat] edits={EDITS} published={} pruned={} pending_len={} \
             retire_dropped={} retire_pending={} switches={} max_pending={max_pending}",
            end.published,
            end.pruned,
            end.pending_len,
            end.retire_dropped,
            end.retire_pending,
            host.engine_stats().expect("统计").snapshot_switches,
        );
    }

    /// 判据 9：投影失败时**当前快照一位不动**（版本号 / 发布计数都不推进）。
    #[test]
    fn a_failed_incremental_publish_keeps_the_current_snapshot() {
        let project = demo_project();
        let track_id = *project.tracks.keys().next().expect("轨道");
        let before_db = project.tracks[&track_id].volume_db;
        let mut host = EngineHost::new();
        host.reload(&project, 0).expect("重建");

        // `YebanProjectV1::default()` 的 `master_bus_track_id` 是 nil ⇒ NoMasterBus。
        let error = host
            .publish_project(&YebanProjectV1::default(), EditMark::new(None, 7))
            .expect_err("没有主总线的工程不可能发布");
        assert!(
            matches!(error, EngineHostError::Snapshot(SnapshotError::NoMasterBus)),
            "实际错误: {error:?}"
        );
        let counts = host.snapshot_counts().expect("有引擎必有计数");
        assert_eq!(counts.published, 1, "失败不得推进发布计数");
        assert_eq!(counts.revision, 1, "失败不得推进快照版本号");
        assert_eq!(counts.pending_len, 0, "失败不得留下待回收条目");
        assert_eq!(
            host.current_snapshot()
                .expect("快照")
                .track(&track_id)
                .expect("轨道")
                .volume_db(),
            before_db,
            "失败之后当前快照仍是老那一份"
        );
    }

    /// 判据 10：没有引擎时心跳是**中性**的（全 0，不 panic），发布如实报 `NoEngine`。
    #[test]
    fn heartbeat_without_an_engine_is_neutral_and_publish_is_refused() {
        let mut host = EngineHost::new();
        assert_eq!(
            host.heartbeat(),
            HeartbeatReadings {
                published: 0,
                pending_len: 0,
                pruned: 0,
                released: 0,
                retired: 0,
                retire_pending: 0,
            }
        );
        assert!(host.snapshot_counts().is_none());
        assert!(host.current_snapshot().is_none());
        assert!(host.engine_stats().is_none());
        let error = host
            .publish_project(&demo_project(), EditMark::new(None, 0))
            .expect_err("没有引擎时发布必须报错");
        assert!(matches!(error, EngineHostError::NoEngine), "{error:?}");
    }

    /// 判据（**R55**）：自动化泳道被采样成 `SetParam` 下发，且三条边界都成立。
    ///
    /// 1. **开着的** `TrackVolume` 泳道 ⇒ 至少下发 1 条；
    /// 2. 同一个采样时点再发一次 ⇒ **条数相同**（采样是 tick 的纯函数 ⇒ 幂等）；
    /// 3. `read_enabled = false` ⇒ **一条都不发**（关掉的泳道是"没有值"，不是"值等于 0"）。
    ///
    /// 采样时点是 `EngineHost::transport().position_ticks`（RT→UI 镜面 = 音频时钟）；
    /// 本判据**不注入任何时钟**（`reload` 推 4 个量子 ⇒ 手算得 20 tick）。
    /// ⚠ 本判据**只能由 CI 执行**（`yeban-app` 依赖 Slint，`AGENTS.md §5` 禁止本机编译）。
    #[test]
    fn automation_lanes_are_sampled_from_the_audio_clock_and_published() {
        let mut project = demo_project();
        // **机械找出**那条带着"开着的 TrackVolume 泳道"的轨（⛔ 不用 `keys().next()`：
        // `BTreeMap` 的首键可能是主总线，而 demo 夹具的 TrackVolume 泳道挂在 slot 0 那条轨上）。
        let mut enabled: Vec<EntityId> = project
            .tracks
            .iter()
            .filter(|(_, track)| {
                track.automation_lanes.values().any(|lane| {
                    lane.read_enabled && matches!(lane.target, AutomationTarget::TrackVolume { .. })
                })
            })
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(
            enabled.len(),
            1,
            "夹具前提：demo 工程恰好有一条**开着的** TrackVolume 泳道（实得 {}）",
            enabled.len()
        );
        let track = enabled.remove(0);
        let target = AutomationTarget::TrackVolume { track_id: track };
        let point = |tick: u64, value: f32| AutomationPoint {
            id: EntityId::new(),
            tick,
            value,
            curve: CurveType::Linear,
        };
        let lane = |read_enabled: bool| AutomationLane {
            target,
            points: std::collections::BTreeMap::from([
                (EntityId::new(), point(0, -12.0)),
                (EntityId::new(), point(960, 0.0)),
            ]),
            read_enabled,
            write_mode: AutomationWriteMode::Off,
            domain: None,
        };
        project
            .tracks
            .get_mut(&track)
            .expect("那条轨必须在")
            .automation_lanes
            .insert(target, lane(true));

        let mut host = EngineHost::new();
        host.reload(&project, 4).expect("重建");
        // 采样时点来自 **RT→UI 镜面**（音频时钟），不是墙钟：`reload` 推 4 个量子
        // = 512 帧；120 BPM / 960 PPQ / 48 kHz 下 **1 tick = 25 帧** ⇒ **20 tick**
        // （可手算、与机器速度无关）。这条断言同时是"时点真的来自音频时钟"的证据。
        let expected_tick = (4 * DEFAULT_BLOCK_FRAMES / 25) as u64;
        assert_eq!(
            host.transport().position_ticks,
            expected_tick,
            "采样时点必须由音频时钟（4 量子 = 512 帧 = 20 tick）给出"
        );
        // R58：等号判据要有一条 `assert_ne!` 落在**同一个**表达式上。这里它同时钉住一条
        // **实测教训**：`reload` 之后 `position_ticks` **不是 0**（曾经的错误假设在 CI 上红过）。
        assert_ne!(
            host.transport().position_ticks,
            0,
            "时点不得是「停住不动」的 0（`reload` 会推量子 ⇒ 它必须前进）"
        );
        // 第 1 跳传 `Some`（= 生产在"标记变了"那一跳的形态）⇒ 更新缓存并下发。
        let written = host.publish_automation(Some(&project));
        assert!(written >= 1, "开着的泳道必须至少下发一条（实得 {written}）");
        // ⭐ **R68：生产的常态是 `None`**（`src/main.rs` 的定时器只在标记变了时才
        // `try_project()`）。若采样挂在入参上，这里会**静默返回 0** —— 那正是本判据要抓的
        // 缺陷（自动化只在工程改动的那一跳生效）。有了工程缓存，`None` 跳必须给出**同一批**。
        let again = host.publish_automation(None);
        assert_eq!(
            again, written,
            "`None` 跳必须仍然从**缓存的工程**里采样并给出同一批条数（R68）"
        );
        // R58：同一条 `==` 上的 `assert_ne!` —— 同时钉住本票修的缺陷本身：
        // `None` 跳**绝不允许**返回 0（那正是"采样挂在入参上"时的行为）。
        assert_ne!(
            again, 0,
            "`None` 跳返回 0 就是 R68 修的静默丢失（自动化只在工程改动的那一跳生效）"
        );
        assert_eq!(
            host.automation_counts(),
            (written as u64 * 2, 0),
            "计数必须逐条对得上，且这一路没有丢"
        );

        // 关掉**所有**读开关（音量那条 + 声相那条）⇒ 一条都不发，且不产生任何计数。
        // （demo 夹具的 slot 0 是 TrackVolume、slot 1 是 TrackPan ⇒ 两条都要关，
        // 否则剩下的那条会照发，`written == 0` 就不成立。）
        let before = host.automation_counts();
        let enabled_before = project
            .tracks
            .values()
            .flat_map(|track| track.automation_lanes.values())
            .filter(|lane| lane.read_enabled)
            .count();
        for lane in project
            .tracks
            .values_mut()
            .flat_map(|track| track.automation_lanes.values_mut())
        {
            lane.read_enabled = false;
        }
        let enabled_after = project
            .tracks
            .values()
            .flat_map(|track| track.automation_lanes.values())
            .filter(|lane| lane.read_enabled)
            .count();
        assert_eq!(enabled_after, 0, "关灯之后不得还有开着的泳道");
        // R58：同一条 `==` 上的 `assert_ne!` 必须是**真探针**（R69）—— 用"关灯前 ≠ 关灯后"
        // 证明这次关灯真的改了状态。⛔ 写成 `count != 0` 是**假探针**（关灯后它恒为 0 ⇒ 必红，
        // 正是上一轮 CI 抓到的那条）。
        assert_ne!(
            enabled_before, enabled_after,
            "关灯必须真的把「开着」的条数从 {enabled_before} 变成 0"
        );
        assert_eq!(
            host.publish_automation(Some(&project)),
            0,
            "关掉的泳道不得下发"
        );
        assert_eq!(
            host.publish_automation(None),
            0,
            "关掉的泳道在 `None` 跳也不得下发"
        );
        assert_eq!(host.automation_counts(), before, "关掉的泳道不产生任何计数");
    }
}
