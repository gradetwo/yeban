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

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_engine::device::{DeviceError, EngineConfig, OutputStreamHandle, ShareMode};
use yeban_engine::insert::strip_source_index;
use yeban_engine::level::db_to_gain;
use yeban_engine::meter::{DEFAULT_METER_CAPACITY, MeterCollector, meter_channel};
use yeban_engine::mixer::{PanLaw, centered_pan_gains};
use yeban_engine::param::{
    MASTER_GAIN_SLOT, PARAM_SLOTS, SEND_GAIN_SLOT, SEND_SLOTS, STRIP_OUTPUT_GAIN_SLOT,
    TRACK_GAIN_SLOT, TRACK_PAN_LEFT_SLOT, TRACK_PAN_RIGHT_SLOT,
};
use yeban_engine::ring::{EngineEvent, EventSender, ParamAddress, TransportCommand, event_channel};
use yeban_engine::routing::edge_carries_audio;
use yeban_engine::rt::{EngineRuntime, EngineStats};
use yeban_engine::sfz_instrument::SfzPlan;
use yeban_engine::snapshot::{
    EngineSnapshot, RetireQueue, SnapshotError, SnapshotSlot, retire_channel,
};
use yeban_engine::transport::{TransportMirror, TransportReading, TransportState};
use yeban_model::AutomationTarget;
use yeban_model::EntityId;
use yeban_model::project::YebanProjectV1;
use yeban_model::{AssetHash, ClipContent};
use yeban_model::{LaunchQuantization, TimeSignature};
// ⭐ S1 第一片：`output_gain_db` 的**离线落点表**（拼写与钳制都由 render 那份定义）。
// 控制侧读它 ⇒ 监听与导出的"这个名字能不能落"不可能各答一套（`StripField::SPELLINGS`
// 是唯一一张名字表，`sanitised_output_gain_db` 走的是器件同一次钳制）。
use yeban_render::inserts::{StripField, sanitised_output_gain_db};

use crate::audio_inject::{
    AudioAssetSource, AudioCacheReadings, AudioClipInjection, AudioClipReport, AudioClipSkip,
    AudioPcmCache,
};
use crate::sfz_cache::{SfzCacheReadings, SfzPlanCache};
use crate::sfz_load::SfzLoadError;

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
    /// ⭐ **SFZ 摊平失败**（任务单 `#80`）：某条待摊平项的 SFZ 文件读不到 / 文本非法 /
    /// 区域表摊不平。
    ///
    /// ⚠ 它让 [`EngineHost::reload`] / [`EngineHost::publish_project`] **明确失败**
    /// （⛔ 不静默回退到复音合成器）：模型说"这条轨用这份文件"，而文件读不到时仍出一版
    /// **不是它**的声音，是用户听不出、报告里也看不见的内容错 —— 与 `#79` 的离线路径
    /// （`WavExportError::Sfz`）同一条裁决。
    ///
    /// 失败时**既有的一代引擎一位不动**（发布点在这一步之前没有动过任何状态）。
    Sfz(SfzLoadError),
}

impl core::fmt::Display for EngineHostError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Snapshot(error) => write!(formatter, "引擎快照投影失败: {error}"),
            Self::NoEngine => write!(formatter, "还没有一代引擎 (reload 从未成功过)"),
            Self::Device(error) => write!(formatter, "音频设备打开失败: {error}"),
            Self::Sfz(error) => write!(formatter, "SFZ 摊平失败: {error}"),
        }
    }
}

impl core::error::Error for EngineHostError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Snapshot(error) => Some(error),
            Self::NoEngine => None,
            Self::Device(error) => Some(error),
            Self::Sfz(error) => Some(error),
        }
    }
}

impl From<SnapshotError> for EngineHostError {
    fn from(value: SnapshotError) -> Self {
        Self::Snapshot(value)
    }
}

impl From<SfzLoadError> for EngineHostError {
    fn from(value: SfzLoadError) -> Self {
        Self::Sfz(value)
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
    /// ⭐ **装箱**：`size_of::<EngineRuntime>()` 本机 = `180144 B`（⭐ `#107` 后；前 `180000 B`）。按值持有 ⇒ `fresh_runtime`
    /// 自己的帧里要留一份返回值位，而这条路会**多层**叠在 GUI 链上（`#72`／台账 R725）。
    /// 装箱后本结构的值尺寸与 `EngineRuntime` 无关，搬动恒为常数级。
    runtime: Box<EngineRuntime>,
    retire: RetireQueue,
    events: EventSender,
    mirror: std::sync::Arc<TransportMirror>,
    collector: MeterCollector,
}

/// 用**当前快照槽**造一套全新端点（[`EngineRuntime::new_boxed`] 要求全部通道在开设备之前建立）。
///
/// ⚠ 构造入口是 [`EngineRuntime::new_boxed`]（⛔ 不是按值返回的 `new`）：在堆上就地构造，
/// 栈上只有一个 `Box` 指针 —— 这条链在 Windows 主线程默认栈（≈ 1 MiB）下才装得下。
fn fresh_runtime(slot: &std::sync::Arc<SnapshotSlot>) -> FreshRuntime {
    let (retire_producer, retire) = retire_channel(RETIRE_CAPACITY);
    let (events, event_receiver) = event_channel(EVENT_CAPACITY);
    let (publisher, collector) = meter_channel(DEFAULT_METER_CAPACITY);
    let runtime = EngineRuntime::new_boxed(slot, retire_producer, event_receiver, publisher);
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

/// **实时引擎不执行的自动化泳道**（按目标分类的读数；任务单 `#259`）。
///
/// `SendGain` 泳道**画得出来、写得进去**（`crate::automation` 的
/// `track-{i}-automation-send-{edge_id}-lane` 与 MCP 的 `yeban_edit_automation`），
/// 但实时引擎**没有发送量槽位**：`yeban_engine::param` 只有
/// `TRACK_GAIN_SLOT` / `MASTER_GAIN_SLOT` / `TRACK_PAN_LEFT_SLOT` /
/// `TRACK_PAN_RIGHT_SLOT`，`4..=65535` 一律 `Unmapped` ⇒ 监听时它**一条都不执行**，
/// 而离线导出（`--export-wav`）与 MCP 渲染走
/// `yeban_render::automation::SendGainAutomation` ⇒ **只在导出里生效**。
/// ⇒ 这种"沉默"必须变成**读数**，⛔ 不能只在代码里静默跳过。
///
/// 只数 `read_enabled == true` 的泳道：关掉的泳道**本来就不该生效**，把它算成缺口
/// 只会制造假读数。`TrackVolume` / `TrackPan` **不**计入 —— 它们已经被
/// [`EngineHost::publish_automation`] 真的落到引擎里了。
///
/// `DeviceParam` / `Macro` 与 `SendGain` **不是**同一类：实时引擎没有槽位。
/// ⚠ ⭐ **任务单 `#266` 修订了这段话的"连离线也没有落点"那一半，`#267`/`#293`/`#304` 再扩**：
/// `DeviceParam` 里**插入链能改的标量字段**（`yeban_render::automation::
/// APPLIED_DEVICE_PARAM_NAMES`，55 个拼写 / 30 个字段：通道条 22（14 增益/EQ/滤波
/// ＋ 8 压缩）＋ Freeverb 4 ＋ 卷积标量 4）现在有离线落点（`yeban_render` 的
/// `DeviceParamAutomation` → `InsertRuntime::apply_device_params`，只在导出/MCP 渲染里）；
/// **卷积 IR 形状**（`conv_ir_decay*` / `conv_ir_seed`）/ 未知名字与 `Macro`
/// **仍然**没有落点。⇒ 读数里**分开列**，⛔ 不混进 `send-gain`。
///
/// ⚠ **这是一个 gauge（每跳重采的当前值），不是累计计数器**：它与
/// [`EngineHost::automation_counts`] 的两个数（累计事件条数）形状不同 ——
/// "没落地的泳道"是工程的状态，60 Hz 累加会每秒 +60，读不出"有几条"。
///
/// ⚠ ⭐ **任务单 `#261` 对上面这段的修订（⭐ 只改 `send_gain` 那一列的口径）**：
/// 上面写"实时引擎**没有发送量槽位**"这句话**当时是真的**，现在**不再是** ——
/// `yeban_engine::param::SEND_GAIN_SLOT = 4` 已存在，`crate::rt` 有一条逐**边**的
/// 发送量平滑表（容量 `SEND_SLOTS = 64`），[`EngineHost::publish_automation`] 也会为
/// 有落点的边下发 `(edge_id, 4, db_to_gain(dB))`。⇒ `send_gain` 这一列现在数的是
/// **边在实时侧没有落点**的那些 `SendGain` 泳道（`RoutingGraph` 里查不到 / 侧链 /
/// 源是母线 / 源不是可渲染音轨）；有落点的泳道**不再计入**（它们真的落地了）。
/// ⛔ 原文保留（不删、不弱化）；这份读数的"不静默"职责不变。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnappliedAutomationLanes {
    /// 开着的 `SendGain` 泳道数**且它的边在实时侧没有落点**。
    ///
    /// `#261` 之后有落点的 `SendGain` 泳道**真的**在监听里生效（逐边槽位）
    /// ⇒ 它们不计入本列；本列只剩"边根本不在可搬运音频的图里"的那一类
    /// （离线导出 / MCP 渲染按边求值，但对这类边同样没有落点）。
    pub send_gain: u64,
    /// 开着的 `DeviceParam` 泳道数**且它在实时侧没有落点**。
    ///
    /// ⭐ **S1 第一片之后这一列的口径变了**：`output_gain_db`（拼写 `output_gain` /
    /// `fader_db` 同）在**有通道条来源的那台设备**上已经真落到实时引擎的通道条
    /// （槽位 [`STRIP_OUTPUT_GAIN_SLOT`]）⇒ 它们**不计入**本列。剩下的仍是没落点的：
    /// 其余通道条字段（EQ／滤波／压缩，`#267` / `#304`）、Freeverb（`#293`）、
    /// 卷积标量（`#293`）—— 这些离线有落点（见任务单 `#266` / `#267` / `#293` / `#304`）、
    /// 实时没有；以及"`slot_index` 指的不是该轨的通道条来源"与"该轨没有通道条来源"
    /// （离线会强制实例化恒等通道条，实时不会 —— 谓词见 [`strip_output_gain_lands`]）。
    pub device_param: u64,
    /// 开着的 `Macro` 泳道数（实时与离线**都**没有落点）。
    pub macro_lanes: u64,
}

impl UnappliedAutomationLanes {
    /// 总数 = 实时引擎不执行的泳道条数。
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.send_gain
            .saturating_add(self.device_param)
            .saturating_add(self.macro_lanes)
    }

    /// 一条都没有（实时引擎能执行本工程里的**全部**自动化泳道）。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.total() == 0
    }
}

/// 一条 `SendGain` 泳道的边在**实时侧有没有落点**（任务单 `#261`）。
///
/// 判据是 [`edge_carries_audio`]（引擎 `RoutingPlan::compile` 用的**同一份**过滤条件）：
/// 只有能搬运音频的出边才会进实时侧的**逐边发送量武装表** ⇒ 只有那些边的
/// `SendGain` 自动化才可能生效。
///
/// ⛔ 容量（[`SEND_SLOTS`]）**不在**本谓词里：表满时是"保留静态边增益 + 计数"，
/// 由引擎的 `send_capacity_drops` 如实报告（⛔ 控制侧不复制一份"能装几条"的算术）。
///
/// 成本：一次 `BTreeMap` 查找 ＋ 一个 3 行的谓词 —— 控制面纯函数，⛔ 不碰音频线程。
fn send_gain_lands(project: &YebanProjectV1, edge_id: EntityId) -> bool {
    project
        .routing_graph
        .edges
        .get(&edge_id)
        .is_some_and(|edge| {
            edge_carries_audio(edge, project.master_bus_track_id, |node| {
                project.tracks.contains_key(&node)
            })
        })
}

/// ⭐ **`output_gain_db` 在实时侧有没有落点**（S1 第一片的谓词，采样与 gauge 共用）。
///
/// 两个条件**都要**成立（缺一就不发、也不落地 ⇒ 由 [`unapplied_automation_lanes`]
/// 计数，⛔ 不静默）：
///
/// 1. **这个名字能被落点**：`StripField::from_name`（离线渲染用的**同一张**拼写表，
///    `output_gain_db` / `output_gain` / `fader_db` 三个拼写）解析到
///    `OutputGainDb` —— 本单**只接这一组**，其余通道条字段（EQ／滤波／压缩）与
///    混响／卷积标量仍然只在导出里有落点（那是 S2 以后的事，读数照旧）；
/// 2. **这台设备就是该轨通道条的来源**：`slot_index ==` [`strip_source_index`]
///    （引擎 `InsertParams::from_devices` 的规则 1＋3，同一个函数）⇒ 否则实时侧改的是
///    **别的**器件的输出增益，那是"落错了地方"比"没落"更坏的结果。
///
/// ⚠ 本谓词**不**回答"引擎的通道条武装表里此刻有没有这条轨的槽位"：
/// 那要读 RT 状态，超出控制侧的边界。打不到 ⇒ 事件原样交给 `ParamTable`
/// 判 `Unmapped` 并计数（引擎侧读数 `param_unmapped_events`），⛔ 不是静默。
///
/// ⚠ **已知残留（如实登记）**：离线渲染在"本轨没有通道条来源器件"时会强制实例化一条
/// 恒等通道条把值落下去（`yeban_render::inserts::forced_identity_strip`），实时侧
/// ⛔ 不做 ⇒ 那类泳道计入 [`unapplied_automation_lanes`] 的 `device_param`。
#[must_use]
fn strip_output_gain_lands(
    project: &YebanProjectV1,
    track_id: EntityId,
    slot_index: usize,
    param_index: usize,
) -> bool {
    let Some(track) = project.tracks.get(&track_id) else {
        return false;
    };
    if strip_source_index(&track.devices) != Some(slot_index) {
        return false;
    }
    let Some(name) = track
        .devices
        .get(slot_index)
        .and_then(|device| device.params.get(param_index))
        .map(|param| param.name.as_str())
    else {
        return false;
    };
    // `from_name` 自身**大小写不敏感** ⇒ ⛔ 不再复制一份 `to_ascii_lowercase`。
    StripField::from_name(name) == Some(StripField::OutputGainDb)
}

/// 扫描工程里的自动化泳道，数出**实时引擎没有执行落点**的那些（任务单 `#259`）。
///
/// 这是**唯一**的分类处：[`EngineHost::publish_automation`] 每跳调用它更新读数，
/// CLI 的 `automation-engine:` 行（[`automation_unapplied_line`]）也调用它
/// ⇒ 两处不可能各数一套。
///
/// ⚠ ⭐ **`#261` 之后 `send_gain` 的口径变了**：`SendGain` 现在**真的**落到实时引擎
/// （逐边发送量槽位）⇒ 只有"边在实时侧**没有落点**"的泳道才计入本读数
/// （谓词：[`send_gain_lands`]）。以前这里无条件计数 —— 那在当时是实话，
/// 现在会变成假读数。⛔ 不是"删掉一个计数"：落点判断与 [`Self::publish_automation`]
/// 的采样用的是**同一个**谓词。
///
/// 成本：O(轨道数 × 每条轨的泳道数)，**无分配**；与 `publish_automation` 自己的
/// 采样循环同阶。⚠ 它是控制面 / UI 线程的纯函数，⛔ 不碰音频线程。
#[must_use]
pub fn unapplied_automation_lanes(project: &YebanProjectV1) -> UnappliedAutomationLanes {
    let mut lanes = UnappliedAutomationLanes::default();
    for track in project.tracks.values() {
        for lane in track.automation_lanes.values() {
            if !lane.read_enabled {
                continue;
            }
            match lane.target {
                AutomationTarget::SendGain { edge_id, .. } => {
                    if !send_gain_lands(project, edge_id) {
                        lanes.send_gain = lanes.send_gain.saturating_add(1);
                    }
                }
                AutomationTarget::DeviceParam {
                    track_id,
                    slot_index,
                    param_index,
                } => {
                    // ⭐ **S1 第一片**：`output_gain_db`（含 `output_gain` / `fader_db` 拼写）
                    // 且**这台设备就是该轨通道条的来源** ⇒ 实时侧有落点 ⇒ **不再**计入本读数。
                    // 谓词与 [`EngineHost::publish_automation`] 的第四块**同一个函数**
                    // （[`strip_output_gain_lands`]）⇒ 采样与读数不可能各答一套。
                    // 其余名字（EQ／滤波／压缩、混响／卷积标量）与"不是通道条来源"的泳道
                    // 照旧计数 —— 那仍是实话，⛔ 不是删掉一个计数。
                    if !strip_output_gain_lands(project, track_id, slot_index, param_index) {
                        lanes.device_param = lanes.device_param.saturating_add(1);
                    }
                }
                AutomationTarget::Macro { .. } => {
                    lanes.macro_lanes = lanes.macro_lanes.saturating_add(1);
                }
                AutomationTarget::TrackVolume { .. } | AutomationTarget::TrackPan { .. } => {}
            }
        }
    }
    lanes
}

/// `automation-engine:` 诊断行（CLI 的 `--headless` 报告；任务单 `#259`）。
///
/// 它**逐字**说出缺口：有几条泳道在监听里没落地、其中哪些只在导出里生效、
/// 哪些在任何渲染器里都没有落点。⛔ 不是"加个计数就完了"。
///
/// ⚠ ⭐ **`#261` 之后 `send-gain` 那一列的措辞改了**：有落点的 `SendGain` 已经
/// 在监听里生效 ⇒ 这一列只剩"**边在实时侧没有落点**"的泳道，所以后半句从
/// "SendGain 只在离线导出/MCP 渲染里生效"改成"SendGain 的边在监听里没有落点"。
/// ⛔ 字段名与行形状（`unapplied-lanes=` 打头、四个 `key=value`）一字未动
/// （既有 CLI 判据按 `field(...)` 读，不按整行位置读）。
#[must_use]
pub fn automation_unapplied_line(lanes: UnappliedAutomationLanes) -> String {
    format!(
        "automation-engine: unapplied-lanes={} send-gain={} device-param={} macro={} \
         note=\"有 {} 条自动化泳道在监听里没落地；SendGain 的边在监听里没有落点\
         （离线导出/MCP 按边求值），DeviceParam 的 output_gain_db 在有通道条来源的轨上有实时落点\
         （拼写 output_gain / fader_db 同一条），其余通道条（含压缩）/混响/卷积标量名字在离线导出/MCP \
         渲染里有落点（卷积 IR 形状没有），Macro 在监听里没有落点\
         （离线导出/MCP 只在存在 Macro 泳道时按映射级联）\"",
        lanes.total(),
        lanes.send_gain,
        lanes.device_param,
        lanes.macro_lanes,
        lanes.total(),
    )
}

/// ⭐ 任务单 `#283`：产品路径的一次**场景启动请求**（Session View 的场景按钮 → 引擎）。
///
/// 字段与 [`yeban_engine::rt::EngineRuntime::queue_scene_launch`] 的入参一一对应
/// （⛔ 不在这里另算生效帧：那是引擎在**量子边界**用
/// [`yeban_engine::transport`] 的同一套整数算术算的，见
/// `yeban_engine::rt::scene_launch_switch_frame`）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneLaunchRequest {
    /// 目标场景身份（引擎只作读数）。
    pub scene_id: Option<EntityId>,
    /// 该场景的 tempo 覆盖（BPM）。
    pub tempo_bpm: f64,
    /// 工程配置的量化档。
    pub quantization: LaunchQuantization,
    /// 工程拍号（引擎快照里没有 ⇒ 由控制面给）。
    pub time_signature: TimeSignature,
}

/// ⭐ 任务单 `#283`：请求**真的落到哪条腿上** —— 如实报告，⛔ 不假装成功。
///
/// 两种驱动形态（见本文件的"驱动形态表"）各自有唯一通路：
///
/// | 形态 | 通路 | 变体 |
/// | :--- | :--- | :--- |
/// | 控制驱动（没有声卡 / 开设备失败） | 直接 `&mut EngineRuntime::queue_scene_launch` | [`Self::Direct`] |
/// | 设备腿（`EngineRuntime` 在 cpal 回调线程上） | 既有的事件环 `EngineEvent::SceneLaunch` | [`Self::QueuedToDevice`] |
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneLaunchDisposition {
    /// 控制驱动：请求已武装在实时运行时上（下一次量子边界开始倒计时）。
    Direct,
    /// 设备腿：请求已写进**既有**无锁 SPSC，cpal 回调在量子边界武装。
    QueuedToDevice,
    /// 事件环满（极罕见）：⛔ 一条都没写进去 ⇒ 调用方必须如实告诉用户。
    EventQueueFull,
    /// 没有引擎 / 没有命令通道：⛔ 一位都没动。
    NoEngine,
}

impl SceneLaunchDisposition {
    /// 这次请求**是否真的送到了引擎**（两个成功变体之一）。
    #[must_use]
    pub const fn is_delivered(self) -> bool {
        matches!(self, Self::Direct | Self::QueuedToDevice)
    }
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
    /// ⭐ **装箱**：`EngineHost` **按值**活在调用者栈上（`ProductionLoop::start` 里先建它、
    /// 再搬进 `Rc<RefCell<..>>`），按值持有 `EngineRuntime` 会让 `size_of::<EngineHost>()`
    /// 跟着 `180144 B` 走 ⇒ GUI 链每一层都多背一份（台账 R725 逐帧实测）。
    runtime: Option<Box<EngineRuntime>>,
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
    /// **`#259` 的读数**：最近一跳"**实时引擎不执行**"的自动化泳道（gauge，不是累计）。
    ///
    /// 由 [`unapplied_automation_lanes`] 在 [`Self::publish_automation`] 的每一跳重采；
    /// 读口见 [`Self::automation_unapplied`] / [`Self::automation_unapplied_breakdown`]。
    automation_unapplied: UnappliedAutomationLanes,
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
    /// ⭐ ⭐ **路径 ⇒ 已摊平 SFZ 的控制侧缓存**（任务单 `#80`）。
    ///
    /// 为什么住在这里（⛔ 不是全局）：生命周期就是"这一代宿主"、写者只有 `reload` /
    /// `publish_project` 两条（都是 `&mut self` ⇒ 不需要 `Mutex`）、判据可隔离。
    /// 完整的设计与三条失效条件见 [`crate::sfz_cache`] 的模块文档。
    sfz_cache: SfzPlanCache,
    /// ⭐ ⭐ **工程文件所在目录**（任务单 `#168`）：相对 SFZ 路径的**第一个**候选根。
    ///
    /// 为什么住在这里（⛔ 不放进 `EngineSnapshot`）：快照是**实时侧**读的不可变投影，
    /// 而"工程从哪个文件打开"是**控制侧**的装配知识（引擎册 `#78` 的快照只带路径字符串）。
    /// 装配方在**打开工程 / 另存为**时经 [`Self::set_project_dir`] 写下它；`reload` /
    /// `publish_project` 用它调 [`crate::sfz_load::resolve_sampler_path`]。
    ///
    /// `None` = 工程不是从文件打开的（内置样本 / 判据装配）⇒ 解析从**仓库根**开始，
    /// 与 `#164`／`#165` 的旧口径一致。
    project_dir: Option<PathBuf>,
    /// ⭐ ⭐ **音频片段字节的只读来源**（任务单 `#136`）：`AssetHash ⇒ 会话 CAS 池里的字节`。
    ///
    /// 为什么住在这里：`reload` / `publish_project` 只拿得到 `&YebanProjectV1`，而 **PCM
    /// 不在工程模型里**（模型只有 `AssetHash`）⇒ 字节必须由装配方交进来。产品把它接成
    /// 会话池（`UndoPort::asset_bytes`，⛔ 不复制第二份事实源）；判据接自己的夹具表。
    ///
    /// ⚠ 只在**控制线程**、且**缓存未命中**时被调用（读字节/解码都允许分配）。
    audio_asset_source: Option<AudioAssetSource>,
    /// ⭐ **按 `AssetHash` 键控的解码缓存**（任务单 `#136`）：PCM 不在工程模型里 ⇒ 每个
    /// 修订都要重新注入 ⇒ 没有它就会每修订重解码。失效/上限见 [`crate::audio_inject`]。
    audio_cache: AudioPcmCache,
    /// ⭐ **最近一次注入的读数**（注入几个素材 / 每个的 `frames` / 跳过原因）。
    audio_report: AudioClipReport,
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

    /// ⭐ **写下工程文件所在目录**（任务单 `#168`）：相对 SFZ 路径的第一个候选根。
    ///
    /// 装配方在**打开工程**（`--open` / 运行期打开）之后、下一次 `reload` 之前调用。
    /// 传相对目录时按**进程 cwd** 先转绝对（`std::path::absolute`）—— 否则"之后改 cwd"
    /// 会把同一个工程目录解释成另一个目录。
    ///
    /// `None` = 工程不是从文件打开的 ⇒ 解析从仓库根开始（`#164`／`#165` 的旧口径）。
    pub fn set_project_dir(&mut self, project_dir: Option<PathBuf>) {
        self.project_dir =
            project_dir.map(|dir| std::path::absolute(&dir).unwrap_or_else(|_| dir.clone()));
    }

    /// 当前写下的工程文件所在目录（诊断/判据读数）。
    #[must_use]
    pub fn project_dir(&self) -> Option<&Path> {
        self.project_dir.as_deref()
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

    /// ⭐ ⭐ **把快照里的待摊平表 arm 成摊平表**（任务单 `#80`：`#79` 登记的
    /// "实时／GUI 播放未接线"那半边）。
    ///
    /// ## 它补的是哪一跳
    ///
    /// `#78` 让 `TrackV3.sampler` 在快照里只留一个**路径**（`PendingSfz`，引擎⛔ 不读盘）；
    /// `#79` 把"路径 ⇒ 摊平表"的六步接到了**导出／录音**的构造期。实时／GUI 那条路走的是
    /// [`EngineHost::reload`] 与 [`EngineHost::publish_project`] —— 本函数就是那两处的**同一个**
    /// 注入点（⛔ 不新建第三条引擎入口、⛔ 不另写一份解码）。
    ///
    /// ## 三步（每一步都复用 `#79` 的既有形状）
    ///
    /// ```text
    /// ① 待摊平项 ← EngineSnapshot::sfz_pending()          // 键升序 ⇒ 确定性 [MODEL-AST-003]
    /// ② 路径 ⇒ 摊平表 ← SfzPlanCache::get_or_prepare(…)    // ⭐ 本单的缓存；未命中才读盘
    ///        build = crate::sfz_load::decode_sfz_plan(path, engine_hz)   // #79 的同一条管线
    /// ③ 注入 ← crate::sfz_load::arm_pending_sfz(snapshot, hz, prebuilt, roots)  // #79 的同一条入口
    ///    （`roots` = 本宿主的 `project_dir` ⇒ 仓库根 ⇒ cwd，任务单 `#168`）
    /// ```
    ///
    /// ## 缓存（本单的核心设计点）
    ///
    /// 采样率取**快照自己的** `sample_rate()`（⛔ 不是界面/选项里的另一份声明）：
    /// 摊平表里的播放速率是相对**这个**采样率算的 ⇒ 它变了就必须重建（[`crate::sfz_cache`] ②）。
    ///
    /// ## 失败口径（⛔ 不静默）
    ///
    /// SFZ 文件读不到 / 文本非法 ⇒ [`EngineHostError::Sfz`] 原样透出；两个调用点都在
    /// "还没有动过任何状态"的位置调它 ⇒ **既有的一代引擎一位不动**。
    ///
    /// ## 红线 7（[MUST-GATE-001]）
    ///
    /// 读盘、解析、PCM 解码与 `SfzPlan` 的 clone **全部**在这里（控制线程）发生；
    /// 音频线程只读快照里已摊平的表。⛔ 本函数不在任何回调路径上。
    fn arm_pending_sfz_into_snapshot(
        &mut self,
        snapshot: EngineSnapshot,
    ) -> Result<EngineSnapshot, EngineHostError> {
        let engine_hz = snapshot.sample_rate() as f32;
        // ⭐ `#168`：相对路径的候选根（工程目录 ⇒ 仓库根 ⇒ cwd）。`project_dir` 先拷出来，
        //    否则下面 `self.sfz_cache` 的可变借用会与 `self.project_dir` 的不可变借用冲突。
        let project_dir = self.project_dir.clone();
        let roots = match project_dir.as_deref() {
            Some(dir) => crate::sfz_load::SfzRoots::under_project(dir),
            None => crate::sfz_load::SfzRoots::none(),
        };
        // ① 待摊平项（`BTreeMap` ⇒ 键升序；`Vec` 收集是因为 `with_sfz` 需要 `&mut self`）。
        let pending: Vec<(EntityId, String)> = snapshot
            .sfz_pending()
            .iter()
            .map(|(track, entry)| (*track, entry.path().to_owned()))
            .collect();
        // ⭐ 缓存只保留"此刻还有 SFZ 绑定的轨"：sampler 被清回 `None` 的轨在这里回收条目
        //    （失效条件 ③；⛔ 不回收的话旧摊平表会只涨不落）。即使 `pending` 为空也要做。
        let live: BTreeSet<EntityId> = pending.iter().map(|(track, _)| *track).collect();
        self.sfz_cache.retain_tracks(&live);
        if pending.is_empty() {
            return Ok(snapshot);
        }
        // ② 路径 ⇒ 绝对路径（⭐ `#168` 的唯一解析）⇒ 摊平表（命中复用；未命中走 `#79` 管线）。
        //
        // ⚠ 缓存键用**解析后**的路径：同一条模型路径在不同 cwd/工程目录下可能指向不同文件，
        //    键必须是"真的读了哪一份"，否则失效条件 ① 会被一个稳定但错误的字符串骗过。
        let mut prebuilt: BTreeMap<EntityId, SfzPlan> = BTreeMap::new();
        for (track, path) in pending {
            let resolved = crate::sfz_load::resolve_sampler_path(&path, roots)
                .map_err(|error| SfzLoadError::Read(error.to_string()))?;
            let key = resolved.to_string_lossy().into_owned();
            let plan = self
                .sfz_cache
                .get_or_prepare(track, &key, engine_hz, |_key, hz| {
                    crate::sfz_load::decode_sfz_plan(&resolved, hz).map(|(plan, _report)| plan)
                })?;
            // ⭐ `#279`：`PreparedSample.pcm` 已是 `Arc<Vec<f32>>` ⇒ 注入的 `SfzPlan::clone`
            //    只复制**区域表/路径**（控制侧的小对象），PCM 用 `Arc::clone` 共享 —— 缓存与
            //    快照是**同一份**样本字节，⛔ 不再 memcpy PCM（`sfz-cache-needs-1` 记的那次
            //    PCM 拷贝不再发生；台账状态由集成者登记）。
            prebuilt.insert(track, (*plan).clone());
        }
        // ③ 复用 `#79` 的注入入口：它先注入 `prebuilt`（并**取代**待摊平项），再解码剩余
        //    待摊平项（这里已经没有剩余 —— 全部由缓存供给 ⇒ ⛔ 不会重复读盘）。
        let (snapshot, _reports) =
            crate::sfz_load::arm_pending_sfz(snapshot, engine_hz, prebuilt, roots)?;
        Ok(snapshot)
    }

    // -----------------------------------------------------------------------
    // 音频片段的 PCM 注入（任务单 `#136`）
    // -----------------------------------------------------------------------

    /// ⭐ **接上资产字节的只读来源**（`AssetHash ⇒ 会话 CAS 池里的字节`；控制线程）。
    ///
    /// 为什么需要它：`ClipContent::Audio` 在工程模型里**只有 `AssetHash`**（PCM 不在模型里）
    /// ⇒ [`Self::reload`] / [`Self::publish_project`] 拿不到字节。产品把会话池
    /// （`UndoPort::asset_bytes`）接进来；判据接自己的夹具表。⛔ 引擎侧一个字不改：
    /// 注入走既有的 `EngineSnapshot::with_audio_asset`（任务单 `#135`）。
    ///
    /// 来源只在**缓存未命中**时被调用；返回 `None` 表示"池里没有这份字节"，会进
    /// [`AudioClipReport::skipped`]（⛔ 不静默）。
    ///
    /// ⚠ 接上之后**已经存在的快照**不会自动补 PCM —— 下一次
    /// [`Self::reload`] / [`Self::publish_project`] 才注入（"每修订重注入"是契约，
    /// ⛔ 不是增量补丁）。
    pub fn set_audio_asset_source(&mut self, source: AudioAssetSource) {
        self.audio_asset_source = Some(source);
    }

    /// 是否接上了资产字节来源（没有 ⇒ 音频片段会被逐条记为"跳过"）。
    #[must_use]
    pub const fn has_audio_asset_source(&self) -> bool {
        self.audio_asset_source.is_some()
    }

    /// ⭐ **最近一次注入的读数**（任务单 `#136` 的判据入口）。
    ///
    /// 没有音频片段 / 还没注入过 ⇒ 一份空报告（`revision = 0`、两个列表都空）。
    #[must_use]
    pub const fn audio_clip_report(&self) -> &AudioClipReport {
        &self.audio_report
    }

    /// 音频解码缓存的读数（`decodes` / `hits` / 回收与上限计数）。
    #[must_use]
    pub fn audio_cache_readings(&self) -> AudioCacheReadings {
        self.audio_cache.readings()
    }

    /// ⭐ ⭐ **第 1.3 步：枚举 `clip_pool` 的音频素材 ⇒ 注入快照**（任务单 `#136`／`#138`）。
    ///
    /// 口径（逐条与交付线一致）：
    ///
    /// 1. **枚举**：当前工程 `clip_pool` 里**所有** `ClipContent::Audio { asset, .. }` 的
    ///    `AssetHash`（`BTreeSet` ⇒ 键升序，确定性 [MODEL-AST-003]）；
    /// 2. **缓存失效**：先按这份集合调 [`AudioPcmCache::retain_live`] ⇒ 不再被引用的素材
    ///    在这一修订掉出缓存（快照自己持有的 `Arc` 由引擎退役队列回收）；
    /// 3. **取 PCM**：命中缓存 ⇒ `Arc` 克隆；未命中 ⇒ 向 [`AudioAssetSource`] 要字节 ⇒
    ///    **同一个** `crate::audio_clip::decode_asset_bytes`（`#134` 的解码器）⇒
    ///    **素材率 ≠ 快照率时**用唯一那份 `yeban_decode::resample_interleaved`（`#138`）
    ///    转到快照率（控制线程；输出帧数 = `ceil(原帧数 × 快照率 / 素材率)`）；
    /// 4. **注入**：`snapshot.with_audio_asset(&asset, pcm)`。传进去的 PCM **已经是快照率**
    ///    ⇒ 引擎的 `SampleRateMismatch` 不再触发；引擎仍保留那道拒绝（⛔ 不在 app 侧重写它）；
    /// 5. **报告**：每个素材的 `frames` / `channels` / `sample_rate`（注入后）＋
    ///    `source_sample_rate` / `source_frames` / `resampled`（重采样前），以及每个跳过的
    ///    **逐字原因**；非空时打到 stderr（`[yeban-app] …`）。
    ///
    /// ⚠ 这个方法**不返回 `Result`**：注入失败是"某一份素材不发声"，不是"整代引擎失败" ——
    /// 与投影失败（`reload` 必须返回 `Err`）是两种不同性质的事。⛔ 但失败一律**可见**。
    #[must_use]
    fn arm_audio_clips_into_snapshot(
        &mut self,
        project: &YebanProjectV1,
        snapshot: EngineSnapshot,
    ) -> EngineSnapshot {
        let live: BTreeSet<AssetHash> = project
            .clip_pool
            .values()
            .filter_map(|entry| match &entry.content {
                ClipContent::Audio { asset, .. } => Some(asset.clone()),
                _ => None,
            })
            .collect();
        self.audio_cache.begin_revision();
        self.audio_cache.retain_live(&live);
        // 目标采样率 = 快照自己的率（`project.audio_config.sample_rate` 的投影，唯一来源）。
        let target_rate = snapshot.sample_rate();
        let mut report = AudioClipReport {
            revision: snapshot.revision(),
            ..AudioClipReport::default()
        };
        let mut snapshot = snapshot;
        for asset in &live {
            let attached_before = snapshot.audio_pcm_spans();
            match self
                .audio_cache
                .lookup(asset, self.audio_asset_source.as_ref(), target_rate)
            {
                Ok(lookup) => {
                    let frames = lookup.pcm.frames();
                    let channels = lookup.pcm.channels();
                    let sample_rate = lookup.pcm.sample_rate();
                    let source_sample_rate = lookup.source_sample_rate;
                    let source_frames = lookup.source_frames;
                    let resampled = lookup.resampled;
                    let from_cache = lookup.from_cache;
                    // ⚠ `with_audio_asset` 消费快照，而它唯一的 `Err`（采样率不符）必须
                    //    让**已经接上的素材**保留 ⇒ 在副本上调用、成功才替换。
                    //    代价 = 每份音频素材一次 `EngineSnapshot` 深拷贝（控制线程；音频素材
                    //    通常个位数，且只在"工程改了"的那一跳发生）—— 换来的是引擎仍是
                    //    **唯一**的采样率判据（⛔ 不在 app 侧重写采样率比较与错误文本）。
                    match snapshot.clone().with_audio_asset(asset, lookup.pcm) {
                        Ok(next) => {
                            let attached = next.audio_pcm_spans().saturating_sub(attached_before);
                            snapshot = next;
                            report.injected_assets = report.injected_assets.saturating_add(1);
                            report.spans_attached = report.spans_attached.saturating_add(attached);
                            if resampled {
                                report.resampled_assets = report.resampled_assets.saturating_add(1);
                            }
                            report.assets.push(AudioClipInjection {
                                asset: asset.clone(),
                                frames,
                                channels,
                                sample_rate,
                                source_sample_rate,
                                source_frames,
                                resampled,
                                spans_attached: attached,
                                from_cache,
                            });
                        }
                        Err(error) => report.skipped.push(AudioClipSkip {
                            asset: asset.as_str().to_owned(),
                            reason: error.to_string(),
                        }),
                    }
                }
                Err(reason) => report.skipped.push(AudioClipSkip {
                    asset: asset.as_str().to_owned(),
                    reason,
                }),
            }
        }
        report.cache = self.audio_cache.readings();
        if !report.is_empty() {
            // ⛔ 不静默：一行摘要 ＋ 逐素材读数 ＋ 逐跳过原因（逐字文本）。
            eprintln!("[yeban-app] {}", report.summary());
            for line in report.asset_lines() {
                eprintln!("[yeban-app] {line}");
            }
            for skip in &report.skipped {
                eprintln!(
                    "[yeban-app] 音频片段注入跳过: asset {}：{}",
                    skip.asset, skip.reason
                );
            }
        }
        self.audio_report = report;
        snapshot
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
        // ⭐ ⭐ 第 1.2 步（任务单 `#80`）：待摊平表 ⇒ 摊平表（缓存 ⇒ 控制侧解码 ⇒ `with_sfz`）。
        //    它在关设备腿**之前**、动任何状态**之前**：失败 ⇒ 旧的一代一位不动
        //    （与投影失败同一条口径）。读盘/分配都发生在**控制线程**上 [红线 7]。
        let snapshot = self.arm_pending_sfz_into_snapshot(snapshot)?;
        // ⭐ ⭐ 第 1.3 步（任务单 `#136`）：`ClipContent::Audio` 的 PCM 注入（缓存 ⇒ 解码 ⇒
        //    `with_audio_asset`）。位置与第 1.2 步**同一条纪律**：投影/摊平之后、关设备腿
        //    之前；失败**不**换代（某份素材跳过 ＋ 报告，见 `arm_audio_clips_into_snapshot`）。
        //    ⚠ 没有接字节来源 / 没有音频片段 ⇒ 这一步是**空操作**，输出与接线前逐位相同。
        let snapshot = self.arm_audio_clips_into_snapshot(project, snapshot);
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
        //
        // ⚠ `open_output` 的入参是**按值**的 `EngineRuntime`（引擎侧 API，本单⛔ 不改）：
        // 这里必须把装箱的那一份**移出来**交给它，调用点因此仍有一个 `180144 B` 的
        // 实参位。这是设备腿的**下限**，不是本单能搬走的部分 —— 读数见
        // `tests/gui_stack_budget.rs` 的 `DeviceLegEntry` 一档。
        let handle = match yeban_engine::device::open_output(&config, *runtime) {
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
        // ⚠ 先把槽的 `Arc` 拷出来（而不是借 `self.slot`）：本函数下面要 `&mut self`
        //    走 SFZ 缓存 ⇒ 不能同时持有对 `self` 的不可变借用。
        let Some(slot) = self.slot.as_ref().map(std::sync::Arc::clone) else {
            return Err(EngineHostError::NoEngine);
        };
        let revision = self.snapshot_revision.saturating_add(1);
        let snapshot = EngineSnapshot::from_project(project, revision)?;
        // ⭐ ⭐ 任务单 `#80` 的**第二个**接线点：与 `reload` 走**同一个**注入函数
        //    （缓存 ＋ `#79` 的 `arm_pending_sfz`），因此"编辑 ⇒ 发声"这条生产路径
        //    （GUI 16 ms 一跳）只在**首次**读盘。失败 ⇒ 槽里当前快照一位不动。
        let snapshot = self.arm_pending_sfz_into_snapshot(snapshot)?;
        // ⭐ ⭐ 任务单 `#136` 的**第二个**接线点：与 `reload` 走**同一个**
        //    `arm_audio_clips_into_snapshot` ⇒ 编辑（加音符 / 再录一次）发布的新快照
        //    **仍然带 PCM**（PCM 不在工程模型里 ⇒ 每个修订都要重注入）。缓存保证同一素材
        //    只解码一次（`audio_cache_readings().decodes` 是判据读数）。
        let snapshot = self.arm_audio_clips_into_snapshot(project, snapshot);
        // 复用槽自己的写者路径（`publish` 就是本函数的包装）。
        slot.publish_arc(std::sync::Arc::new(snapshot));
        self.snapshot_revision = revision;
        self.last_mark = Some(mark);
        Ok(revision)
    }

    /// ⭐ **SFZ 摊平缓存的读数**（任务单 `#80` 的缓存判据；单位见 [`SfzCacheReadings`]）。
    ///
    /// 它与 `yeban_app::sfz_load::read_probe` 的读盘计数是**两个互补的观测量**：
    /// 前者是"缓存自己怎么看"（命中/未命中/回收），后者是"磁盘真的被碰了几次"。
    /// ⛔ 两者都不进实时线程。
    #[must_use]
    pub fn sfz_cache_readings(&self) -> SfzCacheReadings {
        self.sfz_cache.readings()
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
    /// ⭐ **判据用**：当前这一代的 `EngineRuntime` 可变借用（⭐ 只给判据，⛔ 不给产品）。
    ///
    /// ⚠ **为什么必须有它**：`[ARCH-RT-005]` 的试听准入（`attach_audition_branch` /
    /// `begin_audition`）住在 `EngineRuntime` 上，而宿主⛔ 不公开它的 runtime
    /// （那是"换引擎只有一条路"的纪律）。判据要证"**产品的那一个** runtime 上真的能准入"
    /// ⇒ 需要一个**显式标注为判据用**的借用点；⛔ 产品代码不得借它绕过本宿主的 API。
    ///
    /// ⚠ 没装引擎（`reload` 之前 / 失败之后）⇒ `None`（⛔ 不 panic）。
    pub fn runtime_for_criteria(&mut self) -> Option<&mut yeban_engine::rt::EngineRuntime> {
        self.runtime.as_deref_mut()
    }

    /// 心跳：把**写者侧**要做的事做完（电平读数 ＋ 快照退役回收）。
    ///
    /// ⚠ **只在 UI / 主线程上调用**（它读通道、回收快照，都允许分配）。
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
    ///
    /// ⚠ ⭐ **任务单 `#257` 对上面这条登记的修订（⭐ 只针对 `TrackPan` 那一支）**：
    /// ⭐ 原文**保留**（第 278 条：不删原文、不弱化原判据），但它已与代码**漂移** ——
    /// [`Self::publish_automation`] 的声相分支（`#250` 勘察点名的 `TrackPan` 采样段）
    /// **已经把 `TrackPan` 发布进引擎**：逐轨左/右**两条绝对增益**事件，值由同一个
    /// `centered_pan_gains` 与 `PanLaw::from_model(工程)` 算好（超越函数不进音频线程）。
    /// ⇒ “这里**不**碰 `TrackPan`”**不再成立**；`TrackVolume` 那一支没有改动。
    /// ⛔ 离线导出侧的声相自动化是**另一条**执行路径（`#257`，
    /// `yeban_render::automation::TrackPanAutomation`），两者共用同一条曲线、
    /// ⛔ 不共用执行。⚠ 母线仍被排除（引擎的声相表不含它）⇒ 原文的“主总线两个槽位”
    /// 只适用于 `TrackVolume`。
    ///
    /// ⚠ ⭐ **任务单 `#260` 对上面两条登记的修订（⭐ 只改"采样结构"，不改值）**：
    /// ⭐ `#257` 把 `TrackPan` 发布进了引擎，但它被**嵌在** `TrackVolume` 分支的
    /// `if let` **里面**（音量泳道缺失 ⇒ `continue` 把声相段一起跳过）⇒
    /// **只有声相泳道、没有音量泳道**的轨（demo 的贝斯轨就是）声相自动化**一条都不下发**。
    /// 本单把两块改成**并列的两个 `if let`**（见函数体注释 1 / 2）：
    /// * 声相不再依赖音量泳道，也不依赖"音量这一条有效"（无效音量只丢它自己）；
    /// * 两条都有泳道的轨：事件**条数、顺序、值**逐位不变（正对照）；
    /// * 每轨仍是**至多 3 条**（音量 1 ＋ 声相 2）⇒ [`Self::automation_batch_capacity`]
    ///   的 `PARAM_SLOTS * 3` = **48** 不需要变（48 = 16 条引擎槽位 × 3）；
    /// * ⛔ **没有第二份采样**：每条泳道仍然只调一次 `automation_value_at`，仍然只有
    ///   一个 `for track`、一次 `publish`、一份 `automation_project` 缓存；
    /// * `automation_unapplied`（`#259` 的 gauge）**一位不动**：它数的是"实时不执行"的
    ///   那几类目标，声相本来就不在其中。
    ///
    /// ⚠ ⭐ **任务单 `#261` 对上面三条登记的修订（⭐ 只加"第三块并列采样"）**：
    /// ⭐ `SendGain` 泳道**不再只在导出里生效** —— 实时引擎有了**逐边发送量槽位**
    /// （`yeban_engine::param::SEND_GAIN_SLOT = 4`，容量 `SEND_SLOTS = 64`）。
    /// 本函数为**每一对 (轨, 边)** 下发至多一条事件，地址的 `entity` 是**边身份**
    /// （⛔ 不是轨身份）⇒ 同一条轨的发送边与直连边各自独立。
    /// * 换域是**绝对值**（裁决 2）：`db_to_gain(自动化 dB)`，⛔ 不除以静态边增益
    ///   （与声相两条绝对增益同一条口径；音量那一支的"÷ 静态值"是针对**乘子**语义的）；
    /// * `read_enabled:false` ⇒ 不发（裁决 4）；
    /// * 边在实时侧没有落点（不存在 / 侧链 / 源是母线 / 源不是可渲染音轨）⇒ 不发，
    ///   由 [`unapplied_automation_lanes`] 计数（与声相跳过母线同一条先例）。
    ///
    /// 一跳最多下发的自动化事件数：**每条轨 3 条 ＋ 每条可寻址的 `SendGain` 边 1 条
    /// ＋ 每条轨的 `output_gain_db` 落点 1 条**（⭐ S1 第一片）。
    ///
    /// ⛔ 不用 [`PARAM_SLOTS`] 当上限：那个数是**引擎参数表的槽位数**（每条轨一个增益槽位）。
    /// 声相与发送量走引擎里**另外两条独立通路**（`rt.rs` 的逐轨声相平滑对 ／ 逐边发送量
    /// 武装表），不占参数表的槽位 ⇒ 批次可以更大。**每跳仍然只有一次 `publish`
    /// （一条批）**，这条纪律不变。
    ///
    /// 容量 = `PARAM_SLOTS × 3 ＋ SEND_SLOTS ＋ PARAM_SLOTS` = **128**：
    /// 前半是"音量 1 ＋ 声相 2"（每条引擎轨），中半是**引擎发送量表的行数上限**
    /// （`SendGain` 事件最多只对表里的边有落点），后 `PARAM_SLOTS` 是**通道条落点**
    /// （一条轨只有一个通道条来源 ⇒ 每轨至多一条）。⚠ 同一台设备上用**多个拼写**
    /// 各开一条泳道（`output_gain_db` 与 `fader_db` 并存）这类畸形工程会超出上限
    /// ⇒ 超过的部分逐条计进 `automation_dropped`（⛔ 不静默），与发送量那半条同口径。
    const fn automation_batch_capacity() -> usize {
        PARAM_SLOTS * 3 + SEND_SLOTS + PARAM_SLOTS
    }

    /// **把工程里的自动化泳道采样成 `SetParam` 事件并下发**（裁决 R55／R68／P4=(b)）。
    ///
    /// * **采样时点** = [`Self::transport`] 的 `position_ticks`（RT→UI 镜面 = **音频时钟**）
    ///   ⇒ 没有任何墙钟输入，同一个时点给出同一批值（幂等）；
    /// * **工程缓存**：`Some(project)` 更新缓存，`None` 跳用缓存 —— 生产的 16 ms 定时器
    ///   只在"标记变了"的那一跳才 `try_project()`（成本契约），⛔ 采样**不得**挂在入参上；
    /// * 一条轨最多 **3** 条事件：音量 1 条 + 声相 2 条（左/右**绝对**增益，语义是**替换**）；
    ///   ⭐ **`#261`**：另外每一条**有实时落点**的 `SendGain` 边再加 1 条（见下第三块）；
    ///   ⭐ **S1 第一片**：再加**至多 1 条** `output_gain_db`（见下第四块，谓词与
    ///   [`unapplied_automation_lanes`] 共用 ⇒ 没有落点的一条都不发）；
    /// * ⭐ **`#260`**：音量与声相是**两块并列的采样**（各自独立的 `if let`）⇒
    ///   只有声相泳道（或只有音量泳道）的轨也各自下发；⛔ 声相**不**因音量泳道缺失 /
    ///   关闭 / 这一跳没有值 / 值非法而被跳过（修前正是那样，`continue` 跳过了整段）；
    /// * 声相值由引擎公开的 [`centered_pan_gains`] 与 [`PanLaw::from_model`] 在**控制侧**
    ///   算好（超越函数⛔ 不进音频线程），并 `max(0.0)` 钳位（硬右的左增益实测是负的）；
    /// * 返回**实际写入**的事件条数（没写进去的记进 [`Self::automation_counts`] 的第二个数）。
    ///   ⛔ **不加 `#[must_use]`**：生产调用点（`host.rs` 的 `tick`）刻意忽略返回值，
    ///   加了会在 `-D warnings` 下把整条腿弄红。
    /// * ⭐ **`#259`**：每跳还重采一次**实时引擎不执行**的泳道数（本单只点名 `SendGain`，
    ///   顺带按目标分开数 `DeviceParam` / `Macro`），存进 [`Self::automation_unapplied`]；
    ///   它**不产生任何事件**、⛔ 不改一个音频字节。
    pub fn publish_automation(&mut self, project: Option<&YebanProjectV1>) -> usize {
        // **工程缓存（R68 修的那一处）**：生产路径只在"标记变了"的那一跳才 `try_project()`
        // ⇒ 其余各跳传的是 `None`。若采样直接挂在入参上，自动化就**只在工程改动的那一跳
        // 生效** —— 那不是"没采集"，是**静默丢失**。因此：`Some(..)` 时更新缓存，
        // 之后每一跳都用缓存里的那一份采样（克隆仍只发生在本来就克隆的那一跳）。
        if let Some(project) = project {
            self.automation_project = Some(project.clone());
        }
        let Some(project) = self.automation_project.as_ref() else {
            // 没有工程缓存 ⇒ 没有泳道可数（把读数归零，⛔ 不留下上一份工程的旧值）。
            self.automation_unapplied = UnappliedAutomationLanes::default();
            return 0;
        };
        // ⭐ `#259`：先数"这一份工程里实时引擎**不执行**的泳道"（gauge，每跳重采）。
        // 它与下面的采样循环是两个问题：采样循环只发**落地了的**两条目标；
        // 这里数的是**没有落点**的那几条 —— 数出来才有读数，⛔ 不是静默跳过。
        self.automation_unapplied = unapplied_automation_lanes(project);
        let tick = self.transport().position_ticks;
        let batch_capacity = Self::automation_batch_capacity();
        let mut batch = [EngineEvent::Idle; Self::automation_batch_capacity()];
        let mut len = 0usize;
        let mut dropped = 0u64;
        for track in project.tracks.values() {
            // ---- 1. 音量（`TrackVolume`，每条轨**至多 1 条**事件）----
            //
            // ⚠ ⭐ **`#260`：这是两个并列的块，不是一个嵌套的块。**
            // 修前声相段（下一段）写在这个块**里面**（在 `let Some(lane) = … else { continue }`
            // 之后）⇒ "只有声相泳道、没有音量泳道"的轨被 `continue` 整段跳过
            // ⇒ 声相自动化**一条都不下发**。两块现在各自独立 `if let`：互不依赖。
            // ⛔ 也**不**用 `continue` 表达"这一条无效"（那会顺带跳过声相）：
            // 音量这一条只丢它自己，声相照旧采样。每轨最多仍是 **3** 条
            // （音量 1 ＋ 声相 2）⇒ [`Self::automation_batch_capacity`] 的 48 不变。
            let volume_target = AutomationTarget::TrackVolume { track_id: track.id };
            if let Some(lane) = project.automation_lane(&volume_target)
                && lane.read_enabled
                && let Ok(Some(automated_db)) = project.automation_value_at(&volume_target, tick)
            {
                let static_gain = db_to_gain(track.volume_db);
                let gain = if static_gain.is_finite() && static_gain > 0.0 {
                    db_to_gain(automated_db) / static_gain
                } else {
                    // R61-1：静态不可闻（非有限 / 0）⇒ 直接用自动化的绝对值。
                    db_to_gain(automated_db)
                };
                // 无效值（非有限 / 负）与"批满了"都只丢这一条 ⇒ 合成一个条件
                // （⛔ 两个相同的分支会被 `clippy::if_same_then_else` 判红）。
                if !gain.is_finite() || gain < 0.0 || len == batch_capacity {
                    dropped += 1;
                } else {
                    let slot = if track.id == project.master_bus_track_id {
                        MASTER_GAIN_SLOT
                    } else {
                        TRACK_GAIN_SLOT
                    };
                    batch[len] = EngineEvent::SetParam {
                        target: ParamAddress::new(track.id, slot),
                        value: gain,
                    };
                    len += 1;
                }
            }

            // ---- 2. 声相（裁决 P4=(b)）：一条轨**两条**事件（左 / 右**绝对**增益）----
            //
            // 语义是**替换**：泳道给的是声相位置，这里的 `centered_pan_gains` 就是引擎在
            // 快照边界用的同一个公开函数（同一个衰减律，`PanLaw::from_model(工程)`
            // ⇒ `pan == 0` 逐位 `(1, 1)`，`pan != 0` 走曲线）⇒ 值域与引擎内部一致，
            // ⛔ 不做任何分量相除。
            // ⭐ `#300`：修前这里用**裸** `pan_gains` ⇒ 自动化经过 `pan == 0` 时发
            // `0.707/0.707`，而源侧静态已是 `(1, 1)` / 离线自动化也是 `(1, 1)`
            // ⇒ "自动化过中心"仍差 3 dB。现在两条腿共用同一条落点规则。
            // ⚠ 母线**不**进声相表（引擎的 `armed_pan_gains` 明确排除它）⇒ 这里也跳过，
            // 否则两条事件会被判未映射（计数），读数上像缺陷。
            // ⭐ `#260`：这一段**不再**嵌在音量块里（见上）—— 没有音量泳道也照样采样。
            if track.id != project.master_bus_track_id {
                let pan_target = AutomationTarget::TrackPan { track_id: track.id };
                if let Some(lane) = project.automation_lane(&pan_target)
                    && lane.read_enabled
                    && let Ok(Some(pan)) = project.automation_value_at(&pan_target, tick)
                {
                    let (left, right) =
                        centered_pan_gains(pan, PanLaw::from_model(project.audio_config.pan_law));
                    // ⚠ **必须 clamp 到 `>= 0`**：硬右时曲线左增益实测是
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

            // ---- 3. ⭐ 发送量（`SendGain`，任务单 `#261`）：**每对 (轨, 边) 一条** ----
            //
            // 地址的 `entity` 是**边身份**（⛔ 不是轨身份）—— 实时侧的武装表按边开槽，
            // 所以"同一条轨的发送边与直连边"是两条独立的自动化通路（`#258` 的否决判据）。
            //
            // **换域**（与音量那一支**不同**）：本槽位收的是**线性绝对增益**
            // ⇒ 直接 `db_to_gain(自动化 dB)`，⛔ **不除以**静态边增益。
            // 理由：静态边增益在实时侧是"武装时的吸附点"（`snap_to`），
            // 而 `SendGain` 泳道表达的就是"这条发送量应该是多少"（绝对值）
            // —— 与声相两条绝对增益同一条口径（裁决 2）。
            //
            // `read_enabled:false` ⇒ **不发**（等于没有这条泳道 ⇒ 引擎保持静态边增益，裁决 4）。
            //
            // ⚠ 与声相跳过母线**同一条先例**：边在实时侧没有落点（不存在 / 侧链 /
            // 源是母线 / 源不是可渲染音轨）时**不发** —— 否则每一跳都会产生一条被判
            // `Unmapped` 的事件（读数上像缺陷）。这类泳道由
            // [`unapplied_automation_lanes`] 的 `send_gain` 计数（**不静默**）。
            for lane in track.automation_lanes.values() {
                let AutomationTarget::SendGain { edge_id, .. } = lane.target else {
                    continue;
                };
                if !lane.read_enabled || !send_gain_lands(project, edge_id) {
                    continue;
                }
                let Ok(Some(automated_db)) = project.automation_value_at(&lane.target, tick) else {
                    continue;
                };
                // **绝对值**（裁决 2）：音频线程只乘、无除法。
                let value = db_to_gain(automated_db);
                if !value.is_finite() || value < 0.0 || len == batch_capacity {
                    dropped += 1;
                    continue;
                }
                batch[len] = EngineEvent::SetParam {
                    target: ParamAddress::new(edge_id, SEND_GAIN_SLOT),
                    value,
                };
                len += 1;
            }

            // ---- 4. ⭐ 设备参数 `output_gain_db`（S1 第一片）：**每轨至多一条** ----
            //
            // 地址的 `entity` 是**轨身份**（与音量／声相同；实时侧的通道条武装表
            // `armed_strips` 正是按轨查的），槽位 [`STRIP_OUTPUT_GAIN_SLOT`] = 5。
            //
            // **换域 = 绝对值**（与发送量 `#261` 同一条口径，⛔ 不是音量的"÷ 静态值"）：
            // `value = db_to_gain(sanitised_output_gain_db(自动化 dB))`。
            // 先钳制再换算，用的正是**离线渲染那一次**钳制
            // （`StripField::sanitise` → `ChannelStripParams::sanitised`）与**同一个**
            // `db_to_gain`（引擎转发的那份就是 `yeban_dsp::math::db_to_gain`，
            // 由 `the_forwarded_db_to_gain_is_literally_the_dsp_function` 钉住）
            // ⇒ 监听的 `10^(db/20)` 与导出的 `10^(db/20)` **逐位相同**。
            // ⚠ 换算与钳制都发生在**控制侧**：超越函数⛔ 不进音频线程
            // （[ADR-0001 D32]，与声相 `(左, 右)` 增益同一条纪律）。
            //
            // 落点条件（谓词与 [`unapplied_automation_lanes`] **同一个函数**）：
            // 名字被离线的拼写表认成 `OutputGainDb` ＋ `slot_index` 指的就是该轨
            // **通道条来源**那台设备 ⇒ 不满足就**不发**（不发＝没有落点＝计数，
            // ⛔ 不是发一条让引擎判 `Unmapped` 的假事件）。
            // `read_enabled:false` ⇒ 不发（与音量／声相／发送量同一条裁决 4）。
            for lane in track.automation_lanes.values() {
                let AutomationTarget::DeviceParam {
                    track_id: lane_track,
                    slot_index,
                    param_index,
                } = lane.target
                else {
                    continue;
                };
                if !lane.read_enabled
                    || !strip_output_gain_lands(project, lane_track, slot_index, param_index)
                {
                    continue;
                }
                let Ok(Some(automated_db)) = project.automation_value_at(&lane.target, tick) else {
                    continue;
                };
                // **先钳制后换算**（与离线同一次 `sanitised()`）⇒ 非有限 / 越界的值
                // 归到器件的合法域，永远不会把一个 `NaN` 送进音频线程。
                let value = db_to_gain(sanitised_output_gain_db(automated_db));
                if !value.is_finite() || value < 0.0 || len == batch_capacity {
                    // 前两个条件在钳制后**不该**成立（`db_to_gain` 有限域内产出正值）
                    // ⇒ 留作值规则的最后一道，与发送量逐字同形；第三个是批满。
                    dropped += 1;
                    continue;
                }
                batch[len] = EngineEvent::SetParam {
                    target: ParamAddress::new(lane_track, STRIP_OUTPUT_GAIN_SLOT),
                    value,
                };
                len += 1;
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

    /// ⭐ **`#259` 的读数**：最近一跳**实时引擎不执行**的自动化泳道**总数**（gauge）。
    ///
    /// ⚠ 与 [`Self::automation_counts`] 的**形状不同**：那两个是**累计事件条数**
    /// （写进 SPSC / 丢弃），这个是**当前工程里没落地的泳道条数** —— 每跳重采、
    /// ⛔ 不累加。理由见 [`UnappliedAutomationLanes`]：60 Hz 的累计计数读不出"有几条"。
    ///
    /// 没有工程缓存（还没有调用过 `publish_automation(Some(..))`）时是 `0`。
    #[must_use]
    pub const fn automation_unapplied(&self) -> u64 {
        self.automation_unapplied.total()
    }

    /// [`Self::automation_unapplied`] 的按目标分类形态（`send-gain` / `device-param` / `macro`）。
    #[must_use]
    pub const fn automation_unapplied_breakdown(&self) -> UnappliedAutomationLanes {
        self.automation_unapplied
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
        self.runtime.as_deref().map(EngineRuntime::stats)
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

    /// ⭐ 任务单 `#283`：**产品路径的场景启动**（Session View 的场景按钮 → 引擎）。
    ///
    /// 两种驱动形态各走**唯一**通路，⛔ 不新造第二条：
    ///
    /// * 控制驱动（`runtime` 在手里）⇒ 直接
    ///   [`yeban_engine::rt::EngineRuntime::queue_scene_launch`]：生效帧由引擎算，
    ///   本函数**不**推量子（与 [`Self::send_engine_events`] 一样只负责"送到"）；
    /// * 设备腿（`runtime == None`，它在 cpal 回调线程上）⇒ 把
    ///   [`EngineEvent::SceneLaunch`] 写进**既有**的事件环 —— 那是
    ///   [ARCH-RT-001] 允许的唯一跨线程通路，也是"接了声卡的机器上用户按一下真的会生效"
    ///   的关键（给按钮接一个只有控制驱动才工作的回调等于假接线）。
    ///
    /// 返回值如实报告落点（[`SceneLaunchDisposition`]）：没有引擎 / 环满都**不假装成功**。
    ///
    /// ⚠ 本函数不阻塞、**不**等命令生效（那要等下一次量子边界）；判据要观察"真的生效"
    /// 时自己推量子（控制驱动）或等设备回调（设备腿）。
    pub fn queue_scene_launch(&mut self, request: SceneLaunchRequest) -> SceneLaunchDisposition {
        let SceneLaunchRequest {
            scene_id,
            tempo_bpm,
            quantization,
            time_signature,
        } = request;
        if let Some(runtime) = self.runtime.as_deref_mut() {
            runtime.queue_scene_launch(scene_id, tempo_bpm, quantization, time_signature);
            return SceneLaunchDisposition::Direct;
        }
        let Some(sender) = self.events.as_mut() else {
            return SceneLaunchDisposition::NoEngine;
        };
        let written = sender.publish(&[EngineEvent::SceneLaunch {
            scene_id,
            tempo_bpm,
            quantization,
            time_signature,
        }]);
        if written == 1 {
            SceneLaunchDisposition::QueuedToDevice
        } else {
            SceneLaunchDisposition::EventQueueFull
        }
    }

    /// ⭐ 任务单 `#283`：引擎侧**有没有**排队的场景启动（判据读口）。
    ///
    /// ⚠ **设备腿活跃时返回 `None`**：那一代的 `EngineRuntime` 在 cpal 回调线程上
    /// （与 [`Self::engine_stats`] 同一条已知限制，⛔ 不是本函数新造的）。
    #[must_use]
    pub fn has_pending_scene_launch(&self) -> Option<bool> {
        self.runtime
            .as_deref()
            .map(EngineRuntime::has_pending_scene_launch)
    }

    /// ⭐ 任务单 `#283`：引擎累计**真的生效**过的场景启动次数（判据读口）。
    ///
    /// ⚠ 设备腿活跃时返回 `None`（同 [`Self::engine_stats`]）。
    #[must_use]
    pub fn scene_launches_applied(&self) -> Option<u64> {
        self.runtime
            .as_deref()
            .map(EngineRuntime::scene_launches_applied)
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

    /// ⭐ 把**任意引擎事件**（音符开/关、参数、走带）写进**既有的** UI→引擎通道。
    ///
    /// ⛔ 本函数**不新建通道**、⛔ 不新造第二条路径：[`Self::send_transport`] 用的就是这一条
    /// SPSC（[`ARCH-RT-001`] 的"UI/模型线程 → 音频线程"唯一通路）。音符走同一个环 ⇒
    /// "弹一下键盘"与"按一下播放"在引擎侧是**同一种东西**。
    ///
    /// ⭐ 引擎侧自 `eca718ab` 起**真的消费** `NoteOn` / `NoteOff`
    /// （`rt.rs` 的两条分支 + `SynthEngine::note_on/note_off`）⇒ 写进去就会**作用到合成**。
    ///
    /// 返回**真正写进环**的条数（环满时少写 ⇒ 调用方能看出"没全进去"，⛔ 不假装成功）。
    pub fn send_engine_events(&mut self, events: &[EngineEvent]) -> usize {
        match self.events.as_mut() {
            Some(sender) => sender.publish(events),
            None => 0,
        }
    }

    /// ⭐ **离线渲染**：把 `quanta` 个量子推进运行时，并把**真实输出**取回来。
    ///
    /// 与 [`Self::drive_audio`] 的关系：那一个把输出**丢掉**（它只回答"推了多少步"）；
    /// 本函数把输出交回调用方 ⇒ 控制侧（离线导出 / 判据）能听见引擎**真的**发出了什么。
    /// 两者走**同一个** `EngineRuntime::process_quantum`，⛔ 不存在第二条渲染路径。
    ///
    /// `out` 按交错 `channels` 声道解释；实际推进的量子数取
    /// `min(quanta, out.len() / (channels * DEFAULT_BLOCK_FRAMES))` ⇒ 缓冲不足时**少推**，
    /// ⛔ 不 panic、⛔ 不越界。
    ///
    /// ⚠ 只允许**控制侧**（离线导出 / 判据）调用；⛔ 设备腿活跃时恒返回 0
    /// （那一代的运行时在 cpal 回调线程手里，控制面推不了、也不许推）。
    pub fn render_into(&mut self, out: &mut [f32], channels: u16, quanta: u64) -> u64 {
        let channels = channels.max(1);
        let block = DEFAULT_BLOCK_FRAMES * usize::from(channels);
        let capacity = u64::try_from(out.len() / block).unwrap_or(0);
        let quanta = quanta.min(capacity);
        let Some(runtime) = self.runtime.as_mut() else {
            return 0;
        };
        for index in 0..quanta {
            let start = usize::try_from(index).unwrap_or(0) * block;
            let end = start + block;
            runtime.process_quantum(&mut out[start..end], channels);
        }
        quanta
    }

    /// 离线渲染时一个量子对应的**帧数**（`DEFAULT_BLOCK_FRAMES`）。
    ///
    /// 交给调用方（[`crate::midi_notes`]）用**同一把尺子**算"第几个量子"，⛔ 不让它猜常量。
    #[must_use]
    pub const fn quantum_frames() -> usize {
        DEFAULT_BLOCK_FRAMES
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

    /// ⭐ 任务单 `#283`：产品 API 的落点**如实报告**（没引擎 ⇒ `NoEngine`；控制驱动 ⇒
    /// `Direct` ＋ 请求真的武装在运行时上）。
    ///
    /// 设备腿那一格（`QueuedToDevice`）**不在本判据里**：它要求 `runtime == None`，而
    /// 那一形态只有在真声卡打开之后才存在（[`EngineHost::open_device`] 把运行时 move
    /// 进 cpal 回调）⇒ 本机 / CI 都造不出来。消费侧的等价性由 `yeban-engine` 的
    /// `a_scene_launch_sent_through_the_event_ring_fires_like_the_control_call` 钉住。
    #[test]
    fn queue_scene_launch_reports_where_the_request_landed() {
        let request = SceneLaunchRequest {
            scene_id: None,
            tempo_bpm: 140.0,
            quantization: LaunchQuantization::Bar,
            time_signature: TimeSignature {
                numerator: 4,
                denominator: 4,
            },
        };
        let mut host = EngineHost::new();
        assert_eq!(
            host.queue_scene_launch(request),
            SceneLaunchDisposition::NoEngine,
            "没有引擎 ⇒ ⛔ 不假装成功"
        );
        assert_eq!(host.has_pending_scene_launch(), None);
        assert_eq!(host.scene_launches_applied(), None);

        let project = demo_project();
        host.reload(&project, 0).expect("重建");
        assert_eq!(
            host.queue_scene_launch(request),
            SceneLaunchDisposition::Direct,
            "控制驱动 ⇒ 直接调运行时的 queue_scene_launch"
        );
        assert_eq!(
            host.has_pending_scene_launch(),
            Some(true),
            "请求必须真的武装在运行时上"
        );
        assert_eq!(host.scene_launches_applied(), Some(0), "⛔ 还没到点生效");
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

    // ---------------------------------------------------------------------
    // 任务单 `#260`：`TrackPan` 必须**独立于** `TrackVolume` 被采样
    // ---------------------------------------------------------------------

    /// 只有一条开着的 `TrackPan` 泳道、**没有**任何 `TrackVolume` 泳道的轨。
    ///
    /// demo 工程（`yeban_model::samples::demo_automation_lanes`）的**贝斯轨**（slot 1）
    /// 就是这种轨 ⇒ 它是 `#260` 的复现夹具，⛔ 不另造夹具（否则测的就不是产品工程）。
    fn pan_only_track(project: &YebanProjectV1) -> EntityId {
        let mut found: Vec<EntityId> = Vec::new();
        for (id, track) in &project.tracks {
            let has_enabled_pan = track.automation_lanes.values().any(|lane| {
                lane.read_enabled && matches!(lane.target, AutomationTarget::TrackPan { .. })
            });
            let has_volume = track
                .automation_lanes
                .keys()
                .any(|target| matches!(target, AutomationTarget::TrackVolume { .. }));
            if has_enabled_pan && !has_volume {
                found.push(*id);
            }
        }
        assert_eq!(
            found.len(),
            1,
            "夹具前提：demo 恰好一条「只有声相泳道」的轨（实得 {found:?}）"
        );
        found.remove(0)
    }

    /// 只有一条开着的 `TrackVolume` 泳道、**没有**任何 `TrackPan` 泳道的轨（demo slot 0）。
    fn volume_only_track(project: &YebanProjectV1) -> EntityId {
        let mut found: Vec<EntityId> = Vec::new();
        for (id, track) in &project.tracks {
            let has_enabled_volume = track.automation_lanes.values().any(|lane| {
                lane.read_enabled && matches!(lane.target, AutomationTarget::TrackVolume { .. })
            });
            let has_pan = track
                .automation_lanes
                .keys()
                .any(|target| matches!(target, AutomationTarget::TrackPan { .. }));
            if has_enabled_volume && !has_pan {
                found.push(*id);
            }
        }
        assert_eq!(
            found.len(),
            1,
            "夹具前提：demo 恰好一条「只有音量泳道」的轨（实得 {found:?}）"
        );
        found.remove(0)
    }

    /// 把宿主的命令通道换成判据自己的一条，返回这一跳**真正写进 SPSC 的**事件。
    ///
    /// 这是判据读 `publish_automation` **值**的唯一入口（返回值只有条数）；替换的只是
    /// 通道的消费端 —— `publish_automation` 的代码一位没动。
    fn captured_automation_events(
        host: &mut EngineHost,
        project: &YebanProjectV1,
    ) -> Vec<EngineEvent> {
        let (sender, mut receiver) = event_channel(EVENT_CAPACITY);
        host.events = Some(sender);
        let written = host.publish_automation(Some(project));
        let mut scratch = [EngineEvent::Idle; 8];
        let drained = receiver.drain_into(&mut scratch);
        assert_eq!(drained, written, "写进去的条数与读回来的必须一致");
        assert_eq!(receiver.pending(), 0, "一条判据只取这一跳，通道必须取空");
        scratch[..drained].to_vec()
    }

    /// 事件 → `(实体, 槽位, 值)`；⛔ 非 `SetParam` 一律 panic（不静默过滤）。
    fn param_events(events: &[EngineEvent]) -> Vec<(EntityId, u16, f32)> {
        events
            .iter()
            .map(|event| match event {
                EngineEvent::SetParam { target, value } => (target.entity, target.slot, *value),
                other => panic!("自动化只允许下发 SetParam，实得 {other:?}"),
            })
            .collect()
    }

    /// 一个自动化点（判据夹具）。
    fn automation_point(tick: u64, value: f32) -> AutomationPoint {
        AutomationPoint {
            id: EntityId::new(),
            tick,
            value,
            curve: CurveType::Linear,
        }
    }

    /// 一串点 → 泳道的 `points` 表。
    ///
    /// ⚠ **键必须是点自己的 `id`**（`points: BTreeMap<EntityId, AutomationPoint>`）——
    /// 键与内嵌 id 不一致会被 `project.validate()` 判 `EntityKeyMismatch`。
    fn automation_points(points: &[(u64, f32)]) -> BTreeMap<EntityId, AutomationPoint> {
        points
            .iter()
            .map(|(tick, value)| {
                let point = automation_point(*tick, *value);
                (point.id, point)
            })
            .collect()
    }

    /// ⭐ **`#260` 判据 1 + 2**：**只有声相泳道**的轨 ⇒ 修前 **0 条**、修后 **2 条**
    /// （`TRACK_PAN_LEFT_SLOT` ＋ `TRACK_PAN_RIGHT_SLOT`），值落在等功率曲线上。
    ///
    /// demo 贝斯轨在 tick 0 的声相是 **-1.0**（硬左）⇒ 等功率 `(cos 0, sin 0) = (1.0, 0.0)`。
    /// 采样时点 = `transport().position_ticks`；本判据不建引擎 ⇒ 它是 `cold()` 的 **0**
    /// （⛔ 不注入墙钟：`#260` 只改"哪些泳道被采样"，不改采样时点）。
    #[test]
    fn pan_automation_is_sampled_without_a_volume_lane() {
        let project = demo_project();
        let bass = pan_only_track(&project);
        let mut host = EngineHost::new();
        let events = captured_automation_events(&mut host, &project);
        let params = param_events(&events);
        println!("### #260 pan-only events = {params:?}");
        let bass_events: Vec<(EntityId, u16, f32)> = params
            .iter()
            .copied()
            .filter(|(entity, _, _)| *entity == bass)
            .collect();
        // ⚠ 总数里还有 demo **另一条轨**（鼓，slot 0）的音量 1 条 —— 那是既有行为，
        //    本判据只对贝斯轨这一格下断言（⛔ 不看总数就以为"多发"）。
        assert_eq!(
            bass_events.len(),
            2,
            "修前这条轨恒为 0 条（声相段嵌在 `TrackVolume` 分支的 `if let` 里面，\
             音量泳道缺失时 `continue` 把整段跳过了）"
        );
        assert_eq!(
            bass_events,
            vec![
                (bass, TRACK_PAN_LEFT_SLOT, 1.0),
                (bass, TRACK_PAN_RIGHT_SLOT, 0.0),
            ],
            "声相 = -1.0 ⇒ 等功率曲线 (cos 0, sin 0) = (1.0, 0.0)"
        );
        assert_eq!(
            params.len(),
            3,
            "同一次采样里 demo 鼓轨的音量 1 条 ＋ 贝斯声相 2 条"
        );
        assert_eq!(host.automation_counts(), (3, 0), "三条都写进去了、没有丢");
        assert_eq!(host.automation_unapplied(), 0, "声相泳道不是「未落地」");
    }

    /// ⭐ **`#260` 判据 2（值）**：没有音量泳道的轨，声相仍由**同一个** `pan_gains`
    /// 折成**绝对**左右增益（等功率 −3 dB：`L² + R² = 1`）。
    ///
    /// 值用**独立字面量**断言（`cos(π/8)` / `sin(π/8)`），⛔ 不在判据里调 `pan_gains`
    /// 自证（那等于用被测函数证明被测函数）。
    #[test]
    fn a_pan_only_lane_lands_on_the_equal_power_curve() {
        let mut project = demo_project();
        let bass = pan_only_track(&project);
        let target = AutomationTarget::TrackPan { track_id: bass };
        let lane = project
            .tracks
            .get_mut(&bass)
            .expect("贝斯轨必须在")
            .automation_lanes
            .get_mut(&target)
            .expect("声相泳道必须在");
        // 平直 -0.5 ⇒ tick 0 处没有插值歧义；θ = (pan + 1)·π/4 = π/8。
        lane.points = automation_points(&[(0, -0.5), (1_920, -0.5)]);
        let mut host = EngineHost::new();
        let params = param_events(&captured_automation_events(&mut host, &project));
        let bass_events: Vec<(EntityId, u16, f32)> = params
            .iter()
            .copied()
            .filter(|(entity, _, _)| *entity == bass)
            .collect();
        assert_eq!(bass_events.len(), 2, "左 + 右两条");
        let left = bass_events[0].2;
        let right = bass_events[1].2;
        println!("### #260 pan=-0.5 ⇒ L={left:.7} R={right:.7}");
        assert!(
            (left - 0.923_879_5).abs() < 1e-6,
            "左增益 {left} ≠ cos(π/8)"
        );
        assert!(
            (right - 0.382_683_43).abs() < 1e-6,
            "右增益 {right} ≠ sin(π/8)"
        );
        assert!(
            (left * left + right * right - 1.0).abs() < 1e-6,
            "等功率 −3 dB：L² + R² 必须 = 1（实得 {}）",
            left * left + right * right
        );
    }

    /// ⭐ **`#260` 判据 3（正对照）**：音量 ＋ 声相**两条都有**的轨 ⇒ **3 条**事件，
    /// 顺序 `[增益, 声相左, 声相右]`、值与修前**逐位相同**（修前它走的就是这条路）。
    ///
    /// 这里补上的那条音量泳道就是 `#259` 的**绕法**（`tests/automation_unapplied.rs` 的
    /// `insert_volume_lane`）在单测侧的等价物；本判据证明它触发的仍是同一批事件，
    /// 因此 `#260` 的修法对这组输入**零字节变化**。
    #[test]
    fn a_track_with_both_lanes_keeps_its_three_events() {
        let mut project = demo_project();
        let bass = pan_only_track(&project);
        let target = AutomationTarget::TrackVolume { track_id: bass };
        project
            .tracks
            .get_mut(&bass)
            .expect("贝斯轨必须在")
            .automation_lanes
            .insert(
                target,
                AutomationLane {
                    target,
                    points: automation_points(&[(0, -3.0), (1_920, -3.0)]),
                    read_enabled: true,
                    write_mode: AutomationWriteMode::Off,
                    domain: None,
                },
            );
        let mut host = EngineHost::new();
        let params = param_events(&captured_automation_events(&mut host, &project));
        println!("### #260 both-lanes events = {params:?}");
        let bass_events: Vec<(EntityId, u16, f32)> = params
            .iter()
            .copied()
            .filter(|(entity, _, _)| *entity == bass)
            .collect();
        assert_eq!(bass_events.len(), 3, "每条轨：音量 1 ＋ 声相 2");
        // demo 贝斯静态 -6 dB、自动化 -3 dB ⇒ 比率 = 10^(3/20) = 1.4125376。
        assert_eq!(bass_events[0].0, bass);
        assert_eq!(bass_events[0].1, TRACK_GAIN_SLOT);
        assert!(
            (bass_events[0].2 - 1.412_537_6).abs() < 1e-6,
            "3 dB 的线性比 = {}",
            bass_events[0].2
        );
        assert_eq!(
            bass_events[1],
            (bass, TRACK_PAN_LEFT_SLOT, 1.0),
            "顺序必须是 [增益, 声相左, 声相右]（修前修后逐位相同）"
        );
        assert_eq!(bass_events[2], (bass, TRACK_PAN_RIGHT_SLOT, 0.0));
        assert_eq!(params.len(), 4, "贝斯 3 条 ＋ demo 鼓轨的音量 1 条");
        assert_eq!(host.automation_counts(), (4, 0));
    }

    /// ⭐ **`#260` 判据 4**：**只有音量泳道**的轨 ⇒ 仍只发 **1** 条（⛔ 不多发）。
    #[test]
    fn a_track_with_only_a_volume_lane_still_publishes_one_event() {
        let project = demo_project();
        let drums = volume_only_track(&project);
        let mut host = EngineHost::new();
        let params = param_events(&captured_automation_events(&mut host, &project));
        println!("### #260 volume-only events = {params:?}");
        let drums_events: Vec<(EntityId, u16, f32)> = params
            .iter()
            .copied()
            .filter(|(entity, _, _)| *entity == drums)
            .collect();
        assert_eq!(
            drums_events.len(),
            1,
            "只有音量 ⇒ 一条，⛔ 不因为并列结构多发"
        );
        // demo slot 0 的自动化点 A1 与静态值同为 -3.2 dB ⇒ 比率恰为 1.0。
        assert_eq!(drums_events[0], (drums, TRACK_GAIN_SLOT, 1.0));
        assert_eq!(
            params.len(),
            3,
            "同一次采样里鼓 1 条 ＋ 贝斯声相 2 条（修前是 1 条：声相被跳过）"
        );
    }

    /// ⭐ **`#261` 判据**：一条开着的 `SendGain` 泳道（边在实时侧**有落点**）⇒
    /// ① 它**真的**下发一条 `SetParam { entity: <边>, slot: 4, value: db_to_gain(-6 dB) }`；
    /// ② `send-gain` 的 gauge **不**再数它（它落地了）；③ 声相照发 2 条。
    ///
    /// ⚠ 修前（`#259`/`#260`）这里断言的是"数 1、⛔ 不产生事件" —— 那在当时是实话；
    /// 本单把发送量接进实时侧 ⇒ 旧断言会变成假读数，故按新裁决改写（⛔ 不删判据）。
    #[test]
    fn a_send_gain_lane_lands_as_an_edge_address() {
        let mut project = demo_project();
        let bass = pan_only_track(&project);
        let edge = crate::bridge::demo_id("R1");
        let target = AutomationTarget::SendGain {
            track_id: bass,
            edge_id: edge,
        };
        project
            .tracks
            .get_mut(&bass)
            .expect("贝斯轨必须在")
            .automation_lanes
            .insert(
                target,
                AutomationLane {
                    target,
                    points: automation_points(&[(0, -6.0)]),
                    read_enabled: true,
                    write_mode: AutomationWriteMode::Off,
                    domain: None,
                },
            );
        assert_eq!(
            unapplied_automation_lanes(&project).send_gain,
            0,
            "#261：有落点的 SendGain 已落地 ⇒ 不再计入「未落地」"
        );
        let mut host = EngineHost::new();
        let events = captured_automation_events(&mut host, &project);
        let params = param_events(&events);
        // 地址的 `entity` 是**边身份**（⛔ 不是轨身份）—— 这就是跨边独立的寻址单位。
        assert_eq!(
            params
                .iter()
                .filter(|(entity, slot, _)| *entity == edge && *slot == SEND_GAIN_SLOT)
                .count(),
            1,
            "每条有落点的 SendGain 边恰好一条事件（地址 = 边身份）"
        );
        let value = params
            .iter()
            .find(|(entity, slot, _)| *entity == edge && *slot == SEND_GAIN_SLOT)
            .map(|(_, _, value)| *value)
            .expect("上面刚数过 1 条");
        assert_eq!(
            value,
            db_to_gain(-6.0),
            "**绝对值**语义：值 = db_to_gain(泳道 dB)，⛔ 不除以静态边增益"
        );
        let bass_events: Vec<(EntityId, u16, f32)> = params
            .iter()
            .copied()
            .filter(|(entity, _, _)| *entity == bass)
            .collect();
        assert_eq!(bass_events.len(), 2, "贝斯声相仍是 2 条（发送量不挤掉它）");
        assert_eq!(params.len(), 4, "贝斯声相 2 ＋ 鼓音量 1 ＋ 发送量 1");
        assert_eq!(host.automation_counts(), (4, 0), "四条都真的写进 SPSC");
        assert_eq!(host.automation_unapplied(), 0);
        assert_eq!(host.automation_unapplied_breakdown().send_gain, 0);
        assert_eq!(host.automation_unapplied_breakdown().device_param, 0);
        assert_eq!(host.automation_unapplied_breakdown().macro_lanes, 0);
    }

    /// ⭐ **`#261` 判据（负对照）**：泳道的边**没有实时落点**（这里用一个不存在的
    /// 边身份）⇒ ① 不下发事件；② `send-gain` 数 1（不静默）；③ 声相照旧。
    #[test]
    fn a_send_gain_lane_without_a_landing_is_counted_and_publishes_nothing() {
        let mut project = demo_project();
        let bass = pan_only_track(&project);
        let target = AutomationTarget::SendGain {
            track_id: bass,
            edge_id: EntityId::new(),
        };
        project
            .tracks
            .get_mut(&bass)
            .expect("贝斯轨必须在")
            .automation_lanes
            .insert(
                target,
                AutomationLane {
                    target,
                    points: automation_points(&[(0, -6.0)]),
                    read_enabled: true,
                    write_mode: AutomationWriteMode::Off,
                    domain: None,
                },
            );
        assert_eq!(unapplied_automation_lanes(&project).send_gain, 1);
        let mut host = EngineHost::new();
        let events = captured_automation_events(&mut host, &project);
        let params = param_events(&events);
        assert_eq!(
            params.len(),
            3,
            "没有落点的 SendGain 不产生事件（贝斯声相 2 ＋ 鼓音量 1）"
        );
        assert_eq!(host.automation_counts(), (3, 0));
        assert_eq!(host.automation_unapplied(), 1);
        assert_eq!(host.automation_unapplied_breakdown().send_gain, 1);
    }
}
