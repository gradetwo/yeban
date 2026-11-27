//! 电平的**消费侧**：`MeterCollector` → `MeterBoard` → `.slint` 电平条
//! （`[ARCH-UI-002]` 的 UI 一半；生产侧见 `docs/ledger/engine-meters-notes.md`）。
//!
//! ## 规范来源 (Normative)
//!
//! - `[ARCH-UI-002]`（架构 §1.2 原文）：**"UI 线程绝不直接读取音频实时上下文。实时线程以
//!   60Hz 速度向无锁 Meter SPSC 压入真峰值与 RMS 电平，UI 线程定时器批量出队更新
//!   Slint Properties。"** —— 本模块就是"UI 线程定时器批量出队"的落地：
//!   [`MeterRuntime::poll`] 是定时器腿，[`MeterSnapshot`] 是"更新 Slint Properties"的载荷。
//! - `[ARCH-TOP-002]` / `[ARCH-RT-001]`：线程边界。**实时线程**持 `MeterPublisher`
//!   （由 `yeban_engine::meter::meter_channel` 建立），**UI 线程**持 [`MeterRuntime`]。
//!   两者之间只有那条 SPSC，**没有** `Mutex`、**没有**第二份窗口状态。
//! - `[ROAD-M2-008]`（VU/峰值电平独立 SPSC + 60Hz 抽干）、`[ROAD-M2-007]`（批量无锁环形队列）。
//!
//! ## 为什么这里**没有**第二条队列
//!
//! "实时线程发布、UI 取最新"这件事已经由引擎做完了：
//!
//! | 能力 | 归属 | 语义 |
//! | :--- | :--- | :--- |
//! | `meter_channel(capacity)` | `yeban-engine` | 建 SPSC（容量、丢弃行为、零分配全在那边） |
//! | `MeterCollector::drain_latest` | `yeban-engine` | **抽干积压**，每节点只留 `quantum` 最大的一帧 |
//! | `level::supersedes` / `MeterBoard::ingest` | `yeban-engine` | 迟到的旧帧永远顶不回新值 |
//!
//! 本模块只做三件事：**按时抽**、**把帧对上轨道**、**换算成界面要的形态**（0–1 柱高 +
//! dBFS 文本）。任何"再写一个 `VecDeque` 缓存最新帧"的做法都会造出第二份事实源
//! （`[MODEL-AST-002]` 只允许单向投影）—— 那条路被明确否决。
//!
//! ## 为什么必须"抽干"而不是"每 tick 取一批"
//!
//! `MeterCollector::tick`（FIFO 前缀）在"生产快于消费"时会长期积压，而 `rtrb` 满时丢的是
//! **最新**帧 ⇒ UI 永远看不到最新（`docs/ledger/engine-meters-notes.md` §3.1 的实测）。
//! 因此这里**只用** [`MeterCollector::drain_latest`]，并且把它当成本模块唯一的抽帧入口
//! （有判据 `poll_uses_the_drain_latest_semantics_not_the_fifo_one` 钉住）。
//!
//! ## 数值口径（判据钉住的就是这张表）
//!
//! | 量 | 定义 | 备注 |
//! | :--- | :--- | :--- |
//! | 输入幅度 | `MeterFrame::peak` / `rms_smoothed`（线性，1.0 = 0 dBFS） | 引擎已钳位 |
//! | dBFS | `20·log10(幅度)`，`幅度 ≤ 0` ⇒ `-∞` | 复用 `yeban_engine::level::dbfs` |
//! | 显示下限 | [`METER_FLOOR_DBFS`] = **-120.0** | 与引擎的 `SILENCE_FLOOR_DBFS` **同一个常量**（re-export） |
//! | 显示文本 | `{:.1}`；`> 0.0`（过载）**原值保留**成 `"+3.5"` 这种带号文本 | `-∞`/`NaN` 落到下限，`+∞` 落到 [`METER_CEIL_DBFS`] |
//! | 柱高 | `(clamped_dbfs - 下限) / (0 - 下限)`，再夹到 `0..=1` | 纯线性（对数已在 dBFS 里）；过载只能顶格，因此另有 `over` 锁存 |
//! | 过载 | `peak_dbfs > 0.0` ⇒ `ChannelMeter::over`（在 `MeterRuntime` 里**锁存**，直到清零） | 柱高夹到 1.0 会把 +3.5 dBFS 压成 0.0 dBFS 的观感，`over` 就是补这一格 |
//! | LUFS | 帧的 `momentary_lufs`（引擎 BS.1770 瞬时读数），`−∞` ⇒ 下限 | **不再由 RMS 推算**（旧 `rms − 0.691` 公式已删） |
//!
//! **"输入 NaN 不产生 NaN 显示"是两条独立的防线**：引擎侧的 `sanitize_sample` 保证实时
//! 路径不发布 `NaN`；本模块的 [`sanitize_dbfs`] 保证**即使**收到 `NaN`（测试注入、
//! 上游回归）也只会显示下限。判据 `nan_frames_never_render_as_nan` 钉住后者。

use yeban_engine::level::{SILENCE_FLOOR_DBFS, dbfs_clamped};
use yeban_engine::meter::{MeterBoard, MeterCollector, MeterFrame};

use crate::bridge::ViewState;

/// UI 侧显示下限（dBFS）—— **就是**引擎的 `SILENCE_FLOOR_DBFS`。
///
/// 这里 re-export 而不是重写一个字面量：两处各写一个 `-120.0` 迟早在调整观感时漂移，
/// 而"UI 显示下限"与"引擎钳位下限"必须是同一个数（判据
/// `the_display_floor_is_the_engine_floor`）。
pub const METER_FLOOR_DBFS: f32 = SILENCE_FLOOR_DBFS;

/// 连续多少跳没有新量子就把电平判为 **stale**（界面柱置灰、文本 `"--"`）。
///
/// 取 **12 跳**：UI 心跳是 16 ms（`src/main.rs` 的 `slint::Timer`，62.5 Hz），
/// 12 × 16 ms = **192 ms**（约 1/5 秒）；引擎默认量子是 128 帧 / 48 kHz ≈ 2.67 ms，
/// 这段窗口约 72 个量子。它足以吸收偶发的一两帧 UI 抖动，又能在设备真正断流时
/// 于 1/5 秒内把"定格"与"静音"分开，不必等用户先怀疑界面卡死。
pub const METER_STALE_AFTER_TICKS: u32 = 12;
/// 非有限**上界**（dBFS）：只有 `+∞` 会落到这里。
///
/// 它**不是**显示上限：有限值一律原样保留（`peak=1.5` ⇒ `+3.52 dBFS`，文本 `"+3.5"`）。
/// 旧实现把有限值也 `clamp(下限, 0.0)`，于是削波在混音台上显示成 `"0.0"` —— 看不见。
pub const METER_CEIL_DBFS: f32 = 12.0;

/// 一个通道条的电平读数（UI 线程上的**值对象**，不是引擎状态）。
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelMeter {
    /// 视图轨道序号（0 起，**已排除**主总线）—— 与 `ViewState::tracks[i].index` 同一个数。
    ///
    /// 主总线不在这里：它单独放在 [`MeterSnapshot::master`]，因此不存在一个"索引没意义"的
    /// 通道条（那种字段迟早被人当下标用）。
    pub index: usize,
    /// 轨道身份（`EntityId` 的 26 字符规范文本；与 `ViewState::tracks[i].id` 同一串）。
    pub node_id: String,
    /// 显示名（工程里的 `TrackV3::name`）。
    pub name: String,
    /// 峰值 dBFS，**只按下限钳位**，恒有限；`> 0.0` 表示过载（原值保留，⛔ 不截成 `0.0`）。
    pub peak_dbfs: f32,
    /// 平滑 RMS dBFS，**已按下限钳位**，恒有限。
    pub rms_dbfs: f32,
    /// 瞬时响度 momentary（LUFS，BS.1770-4 真实读数，来自引擎帧）。
    ///
    /// 只有主总线帧带真实读数；轨道帧恒为负无穷（不计量响度）。负无穷在
    /// [`MeterSnapshot::master_lufs`] 里被钳到显示下限 —— "没有读数"与"静音"
    /// 在界面上是同一个有限值。
    pub lufs: f32,
    /// 峰值柱高（0.0–1.0，过载只能顶格），由 [`peak_fraction`] 从 `peak_dbfs` 得到。
    pub level: f32,
    /// 本帧是否过载（`peak_dbfs > 0.0`）。`MeterRuntime::snapshot` 会把它**锁存**
    /// （保持到 [`MeterRuntime::clear_over_latches`] 清零），界面因此有顶格过载态。
    pub over: bool,
    /// 左声道柱高（0.0–1.0），来自帧的 `peak_l`。
    pub level_l: f32,
    /// 右声道柱高（0.0–1.0），来自帧的 `peak_r`。
    pub level_r: f32,
    /// 峰值的显示文本（`{:.1}`，例如 `"-6.0"` / `"-120.0"`）。
    pub peak_label: String,
    /// 平滑 RMS 的显示文本。
    pub rms_label: String,
    /// 这一读数的量子序号（`None` = 该节点还没有任何帧）。
    pub quantum: Option<u64>,
}

impl ChannelMeter {
    /// 没有电平读数时的通道条（全静态：下限 + 0 柱高）。
    ///
    /// 它出现的地方比想象的多：工程刚换、引擎刚重建、混音台第一次显示。
    /// 让它有**唯一**一个构造点，是为了"没有读数"与"读数是静音"在界面上完全一致
    /// （都是有限值 `-120.0`，而不是 `""`/`NaN` 之类的第三种形态）。
    #[must_use]
    pub fn silent(index: usize, node_id: String, name: String) -> Self {
        Self {
            index,
            node_id,
            name,
            peak_dbfs: METER_FLOOR_DBFS,
            rms_dbfs: METER_FLOOR_DBFS,
            lufs: METER_FLOOR_DBFS,
            level: 0.0,
            level_l: 0.0,
            level_r: 0.0,
            over: false,
            peak_label: display_dbfs(METER_FLOOR_DBFS),
            rms_label: display_dbfs(METER_FLOOR_DBFS),
            quantum: None,
        }
    }

    /// 由一帧真实电平构造。
    #[must_use]
    pub fn from_frame(index: usize, name: String, frame: &MeterFrame) -> Self {
        let peak_dbfs = sanitize_dbfs(frame.peak_dbfs_clamped(METER_FLOOR_DBFS));
        let rms_dbfs = sanitize_dbfs(frame.rms_dbfs());
        Self {
            index,
            node_id: frame.node.to_canonical_string(),
            name,
            peak_dbfs,
            rms_dbfs,
            lufs: frame.momentary_lufs,
            level: peak_fraction(peak_dbfs),
            level_l: amplitude_fraction(frame.peak_l),
            level_r: amplitude_fraction(frame.peak_r),
            over: peak_dbfs > 0.0,
            peak_label: display_dbfs(peak_dbfs),
            rms_label: display_dbfs(rms_dbfs),
            quantum: Some(frame.quantum),
        }
    }
}

/// 一次 UI 定时器抽帧的**完整结果**：界面侧电平的唯一数据来源。
///
/// `tracks` 的顺序与 [`ViewState::tracks`] **逐个对齐**（混音台用下标取），
/// `master` 是主总线那一条（工程没有主总线时为 `None`）。
#[derive(Debug, Clone, PartialEq)]
pub struct MeterSnapshot {
    /// 非主总线轨道（顺序 = 投影顺序）。
    pub tracks: Vec<ChannelMeter>,
    /// 主总线通道条（`ViewState::master` 存在时才有）。
    pub master: Option<ChannelMeter>,
    /// 面板里最大的量子序号（`None` = 一条帧都没有）。
    pub quantum: Option<u64>,
    /// 面板里已知的节点数（诊断用：与 `tracks + 1` 比较就知道"引擎报的轨道集"跟不跟得上工程）。
    pub nodes_seen: usize,
    /// 电平是否已过期（有读数、但连续 [`METER_STALE_AFTER_TICKS`] 跳以上没有新量子）。
    ///
    /// 过期时界面把柱置灰、文本改 `"--"`；数值仍保留最后一帧（柱高不跳回 0），
    /// 因此"引擎断流"与"真的静音"在画面上不再同形。
    pub stale: bool,
}

impl MeterSnapshot {
    /// 每一轨的峰值显示文本（`track-meter-peaks` 的注入源）。
    ///
    /// stale 时统一给 `"--"`：数值仍是最后一帧，但界面不再把它冒充当前读数。
    #[must_use]
    pub fn peak_labels(&self) -> Vec<String> {
        if self.stale {
            return vec!["--".to_owned(); self.tracks.len()];
        }
        self.tracks
            .iter()
            .map(|track| track.peak_label.clone())
            .collect()
    }

    /// 每一轨的平滑 RMS 显示文本（`track-meter-rmss` 的注入源）。
    #[must_use]
    pub fn rms_labels(&self) -> Vec<String> {
        if self.stale {
            return vec!["--".to_owned(); self.tracks.len()];
        }
        self.tracks
            .iter()
            .map(|track| track.rms_label.clone())
            .collect()
    }

    /// 每一轨的柱高（0.0–1.0）。
    #[must_use]
    pub fn levels(&self) -> Vec<f32> {
        self.tracks.iter().map(|track| track.level).collect()
    }

    /// 主控峰值文本（无主总线时为下限文本；stale 时为 `"--"`）。
    #[must_use]
    pub fn master_peak_label(&self) -> String {
        if self.stale {
            return "--".to_owned();
        }
        self.master.as_ref().map_or_else(
            || display_dbfs(METER_FLOOR_DBFS),
            |master| master.peak_label.clone(),
        )
    }

    /// 主控 RMS 文本（stale 时为 `"--"`）。
    #[must_use]
    pub fn master_rms_label(&self) -> String {
        if self.stale {
            return "--".to_owned();
        }
        self.master.as_ref().map_or_else(
            || display_dbfs(METER_FLOOR_DBFS),
            |master| master.rms_label.clone(),
        )
    }

    /// 每一轨的过载锁存（`track-meter-overs` 的注入源）。
    #[must_use]
    pub fn overs(&self) -> Vec<bool> {
        self.tracks.iter().map(|track| track.over).collect()
    }

    /// 主控过载锁存（无主总线时为 `false`）。
    #[must_use]
    pub fn master_over(&self) -> bool {
        self.master.as_ref().is_some_and(|master| master.over)
    }

    /// 主控柱高。
    #[must_use]
    pub fn master_level(&self) -> f32 {
        self.master.as_ref().map_or(0.0, |master| master.level)
    }

    /// 主控**左**声道柱高（0.0–1.0）。
    ///
    /// 与 [`master_level`](Self::master_level)（两声道最大，契约面的单标量）
    /// 不同：这里读的是引擎帧里**左声道自己的峰值**。左强右弱时两者不相等——
    /// 这正是「真立体声」与「同一标量复制两遍」的判别点。
    #[must_use]
    pub fn master_level_l(&self) -> f32 {
        self.master.as_ref().map_or(0.0, |master| master.level_l)
    }

    /// 主控**右**声道柱高（0.0–1.0）。语义同 [`master_level_l`](Self::master_level_l)。
    #[must_use]
    pub fn master_level_r(&self) -> f32 {
        self.master.as_ref().map_or(0.0, |master| master.level_r)
    }

    /// 主控 LUFS 读数（LKFS / LUFS）—— **引擎帧里的 BS.1770 瞬时响度**。
    ///
    /// 读数由实时线程在主总线计量时产出：`GatedLoudness` 对限制之后的母线立体声
    /// 做 K 加权 + 400 ms 滑窗（[`yeban_engine::meter::MeterBank`]），经电平 SPSC
    /// 随帧送到这里。本函数**只读帧**，不再算 `rms_dbfs − 0.691`：那个公式
    /// 没有 K 加权/窗口/门限，在 997 Hz 正弦上会把 −20.0 LUFS 报成 −23.7 LUFS。
    /// 帧里还没有读数（窗口未满 / 静音 / 采样率不支持）或没有主总线时，读数为
    /// 显示下限 METER_FLOOR_DBFS（`sanitize_dbfs` 把负无穷钳到下限）。
    #[must_use]
    pub fn master_lufs(&self) -> f32 {
        self.master
            .as_ref()
            .map_or(METER_FLOOR_DBFS, |master| sanitize_dbfs(master.lufs))
    }

    /// 主控 LUFS 显示文本。**未测出**时不许把显示下限冒充真实读数：
    /// stale / 无主总线 / 该主总线还没有帧（`quantum == None`）/ 帧里响度非有限
    /// （窗口未满、静音、采样率不支持）⇒ `"--"`；只有真实帧给出有限 LUFS 时
    /// 才显示读数（如 `-20.0`）。
    ///
    /// 数值侧 [`Self::master_lufs`] 仍返回钳到下限的有限值供柱高/计算使用，
    /// ⛔ 不改 DSP 口径；本函数只决定界面把什么当成"已经测出来"的读数。
    #[must_use]
    pub fn master_lufs_label(&self) -> String {
        if self.stale {
            return "--".to_owned();
        }
        let measured = self
            .master
            .as_ref()
            .is_some_and(|master| master.quantum.is_some() && master.lufs.is_finite());
        if !measured {
            return "--".to_owned();
        }
        display_dbfs(self.master_lufs())
    }
}

/// 一个**没有引擎**的静音快照：每轨都是 [`ChannelMeter::silent`]。
///
/// 用它而不是"空数组"：`.slint` 的电平数组必须与 `tracks` **等长**（界面按下标取），
/// 否则换工程的那一瞬间会按下标取到别的轨道的值（或越界）。`host::apply_view` 就在用
/// 它把电平数组重置成"与当前工程等长的静音"。
#[must_use]
pub fn silent_snapshot(view: &ViewState) -> MeterSnapshot {
    let tracks = view
        .tracks
        .iter()
        .map(|track| ChannelMeter::silent(track.index, track.id.clone(), track.name.clone()))
        .collect();
    let master = view
        .master
        .as_ref()
        .map(|master| ChannelMeter::silent(master.index, master.id.clone(), master.name.clone()));
    MeterSnapshot {
        tracks,
        master,
        quantum: None,
        nodes_seen: 0,
        stale: false,
    }
}

/// 把面板上的最新电平对齐到投影出来的轨道上。
///
/// 对齐规则：**按轨道身份**（`EntityId` 规范文本），不按下标 —— 引擎的节点集合与视图的
/// 轨道集合可能暂时不一致（快照切换、轨道增删、淘汰），按下标对齐会把 A 轨的电平画到
/// B 轨上。找不到帧的轨道给"静音"（有限值），**不给**上一条轨道的残留。
#[must_use]
pub fn snapshot(view: &ViewState, board: &MeterBoard, stale: bool) -> MeterSnapshot {
    // `MeterBoard` 内部是 `BTreeMap`（红线 4 的确定性顺序），把"面板里有什么"抄成
    // `Vec<(规范id, &MeterFrame)>` —— 轨道数在那儿是 O(轨道数 × 节点数) 的字符串比较，
    // 对 60Hz × 百轨量级完全够用，而且**不需要**在 app 侧再引入第二份索引。
    let held: Vec<(String, &MeterFrame)> = board
        .iter()
        .map(|(node, frame)| (node.to_canonical_string(), frame))
        .collect();
    let find = |id: &str| held.iter().find(|(node, _)| node == id).map(|(_, f)| *f);

    let tracks = view
        .tracks
        .iter()
        .map(|track| match find(&track.id) {
            Some(frame) => ChannelMeter::from_frame(track.index, track.name.clone(), frame),
            None => ChannelMeter::silent(track.index, track.id.clone(), track.name.clone()),
        })
        .collect();
    let master = view.master.as_ref().map(|master| match find(&master.id) {
        Some(frame) => ChannelMeter::from_frame(master.index, master.name.clone(), frame),
        None => ChannelMeter::silent(master.index, master.id.clone(), master.name.clone()),
    });
    MeterSnapshot {
        tracks,
        master,
        quantum: board.latest_quantum(),
        nodes_seen: board.len(),
        stale,
    }
}

/// `NaN`/`±∞` 安全的 dBFS 钳位（**显示路径的最后一道防线**）。
///
/// 与引擎的 `dbfs_clamped` 的分工：那个函数处理"线性幅度 → dBFS"（生产侧），
/// 这个函数处理"已经是 dBFS 的数"（消费侧，可能来自测试注入或上游回归）。
#[must_use]
pub fn sanitize_dbfs(value: f32) -> f32 {
    if value.is_nan() {
        return METER_FLOOR_DBFS;
    }
    if value == f32::INFINITY {
        return METER_CEIL_DBFS;
    }
    if value == f32::NEG_INFINITY {
        return METER_FLOOR_DBFS;
    }
    // ⭐ T-FIX-521：**只钳下限**。有限值里的 `+3.52`（幅度 1.5）是真实过载，
    // 截成 `0.0` 会让削波在混音台上彻底看不见（柱高仍由 `peak_fraction` 夹到 1.0）。
    value.max(METER_FLOOR_DBFS)
}

/// dBFS → 显示文本（`{:.1}`，过载带 `+` 号）。非有限输入先经 [`sanitize_dbfs`]。
///
/// ⭐ T-FIX-521：`> 0.0` 显式带 `+`（`"+3.5"`）—— 混音台上"0.0"与"过载"必须一眼分得开。
#[must_use]
pub fn display_dbfs(value: f32) -> String {
    let value = sanitize_dbfs(value);
    if value > 0.0 {
        format!("+{value:.1}")
    } else {
        format!("{value:.1}")
    }
}

/// 峰值 dBFS → 柱高（0.0–1.0，纯线性映射到 [`METER_FLOOR_DBFS`]..`0.0`）。
#[must_use]
pub fn peak_fraction(peak_dbfs: f32) -> f32 {
    let clamped = sanitize_dbfs(peak_dbfs);
    let span = 0.0 - METER_FLOOR_DBFS;
    ((clamped - METER_FLOOR_DBFS) / span).clamp(0.0, 1.0)
}

/// 线性峰值幅度 → 柱高（判据与接线方常用的便捷形态）。
#[must_use]
pub fn amplitude_fraction(amplitude: f32) -> f32 {
    peak_fraction(dbfs_clamped(amplitude, METER_FLOOR_DBFS))
}

/// UI 线程持有的电平消费端（**60Hz 定时器腿**）。
///
/// 它**持有** [`MeterCollector`]（SPSC 的消费端）与 [`MeterBoard`]（每节点最新值），
/// 并复用一块栈外的 `scratch` 搬运缓冲。三样东西的生命周期绑定在一起，因为
/// "抽干 → 覆盖 → 面板"这三步必须在同一个线程上、按同一个节奏发生。
///
/// ## 为什么 `collector` 是 `Option`
///
/// 引擎可以在两种时刻不存在：app 刚起来还没建引擎；引擎重建的**中间态**。
/// 用 `Option` 而不是"建一条空队列"，是为了让"没有引擎"这件事在类型上可见 ——
/// 一条永远没有生产者的队列会让 `ui/tree` 显示"静音"，与"真的有引擎且在静音"无法区分。
#[derive(Debug)]
pub struct MeterRuntime {
    collector: Option<MeterCollector>,
    board: MeterBoard,
    scratch: Vec<MeterFrame>,
    /// 连续"有读数但最大量子没有前进"的 UI 跳数。
    stale_ticks: u32,
    /// 本轮长度不一致是否已经发过可见回执（一次失配只响一次，恢复后重新武装）。
    length_mismatch_reported: bool,
    /// 过载**锁存**（节点身份 → 是否曾经过载）。`MeterBoard` 每帧都是"当下"，
    /// 60Hz 下 +3.5 dBFS 的一帧转瞬即逝；锁存让顶格过载态留到用户清零为止。
    over_latched: std::collections::BTreeMap<String, bool>,
}

impl Default for MeterRuntime {
    fn default() -> Self {
        Self::empty()
    }
}

impl MeterRuntime {
    /// 还没有引擎的消费端。
    #[must_use]
    pub fn empty() -> Self {
        Self {
            collector: None,
            board: MeterBoard::new(),
            scratch: Vec::new(),
            stale_ticks: 0,
            length_mismatch_reported: false,
            over_latched: std::collections::BTreeMap::new(),
        }
    }

    /// 有引擎的消费端。
    #[must_use]
    pub fn with_collector(collector: MeterCollector) -> Self {
        let mut runtime = Self::empty();
        runtime.adopt(collector);
        runtime
    }

    /// **采纳**一条新的电平队列（引擎首次就绪 / 引擎重建）。
    ///
    /// 语义上等于"引擎换代"：旧的电平读数不再代表任何东西，因此面板被**清空**
    /// （不是保留到新帧到达）。这条语义使 `ui/reload_engine` 有一个可被界面观测到的副作用
    /// ——混音台电平条回到下限（判据 `reload_engine_resets_the_meter_tap`）。
    pub fn adopt(&mut self, collector: MeterCollector) {
        self.collector = Some(collector);
        self.board = MeterBoard::new();
        self.scratch.clear();
        self.stale_ticks = 0;
        self.length_mismatch_reported = false;
    }

    /// 是否有引擎在供电平。
    #[must_use]
    pub const fn has_collector(&self) -> bool {
        self.collector.is_some()
    }

    /// 当前面板（每节点最新值）。
    #[must_use]
    pub const fn board(&self) -> &MeterBoard {
        &self.board
    }

    /// 一轮 60Hz 抽帧：**抽干积压**、每节点只留最新、喂进面板。
    ///
    /// `expected_nodes` 是调用方预期的节点数（`轨道数 + 1`）。搬运缓冲按它扩容 ——
    /// 缓冲装不下时 `drain_latest` 仍然抽干队列（防积压）并丢弃装不下的节点，
    /// 因此这里给足容量是"不丢节点"的唯一条件。
    ///
    /// 返回本轮**见到的最新节点数**（= 面板里的节点数）。
    pub fn poll(&mut self, expected_nodes: usize) -> usize {
        let Some(collector) = self.collector.as_mut() else {
            return 0;
        };
        let before = self.board.latest_quantum();
        let want = expected_nodes.max(1);
        if self.scratch.len() < want {
            // UI 线程允许分配（红线 7 约束的是音频线程）。
            self.scratch.resize(want, MeterFrame::default());
        }
        let filled = collector.drain_latest(&mut self.scratch);
        self.board.ingest(&self.scratch[..filled]);
        let after = self.board.latest_quantum();
        if after.is_some() && after == before {
            self.stale_ticks = self.stale_ticks.saturating_add(1);
        } else {
            self.stale_ticks = 0;
        }
        self.board.len()
    }

    /// 当前读数是否 stale（语义见 [`MeterSnapshot::stale`]）。
    #[must_use]
    pub fn is_stale(&self) -> bool {
        self.board.latest_quantum().is_some() && self.stale_ticks > METER_STALE_AFTER_TICKS
    }

    /// 记录一次"界面行数 ≠ 投影轨道数"；返回 `true` 表示**本次失配第一次**出现，
    /// 调用方应发一次可见回执。连续失配只响一次，长度恢复后重新武装。
    pub(crate) fn note_length_mismatch(&mut self) -> bool {
        if self.length_mismatch_reported {
            false
        } else {
            self.length_mismatch_reported = true;
            true
        }
    }

    /// 长度契约恢复 ⇒ 下一次再失配时允许重新发声。
    pub(crate) fn clear_length_mismatch(&mut self) {
        self.length_mismatch_reported = false;
    }

    /// 当前面板对齐到工程后的读数（界面要的形态）。
    #[must_use]
    /// ⭐ T-FIX-521：这里顺带**锁存过载** —— 任一阵发过 `peak_dbfs > 0.0`，该节点的
    /// `over` 就一直为真，直到 [`clear_over_latches`](Self::clear_over_latches) 清零
    /// （工程切换、用户点击主控电平条都会清）。
    pub fn snapshot(&mut self, view: &ViewState) -> MeterSnapshot {
        let mut shot = snapshot(view, &self.board, self.is_stale());
        for meter in shot.tracks.iter_mut().chain(shot.master.iter_mut()) {
            let latched = self
                .over_latched
                .entry(meter.node_id.clone())
                .or_insert(false);
            *latched |= meter.over;
            meter.over = *latched;
        }
        shot
    }

    /// 清空过载锁存（下一次 [`snapshot`](Self::snapshot) 从"当下"重新开始）。
    pub fn clear_over_latches(&mut self) {
        self.over_latched.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_engine::meter::meter_channel;
    use yeban_model::EntityId;
    use yeban_model::project::YebanProjectV1;

    use crate::bridge::{ViewState, demo_project};

    /// 工程里第一条轨道的身份（`EntityId`）。
    fn first_track_id(project: &YebanProjectV1) -> EntityId {
        project
            .tracks
            .keys()
            .find(|id| **id != project.master_bus_track_id)
            .copied()
            .expect("工程必须有非主总线轨道")
    }

    /// 判据 1：显示下限与引擎的静音下限**是同一个数**（不是各写一份字面量）。
    #[test]
    fn the_display_floor_is_the_engine_floor() {
        assert_eq!(METER_FLOOR_DBFS, SILENCE_FLOOR_DBFS);
        assert_eq!(METER_FLOOR_DBFS, -120.0);
        assert_eq!(display_dbfs(METER_FLOOR_DBFS), "-120.0");
        assert_eq!(peak_fraction(METER_FLOOR_DBFS), 0.0);
        assert_eq!(peak_fraction(0.0), 1.0);
        // 半幅满量程 ≈ -6.02 dBFS ⇒ 柱高 ≈ 0.9498（线性映射，不做任何"好看"的曲线）。
        let half = amplitude_fraction(0.5);
        assert!((half - (1.0 - 6.0206 / 120.0)).abs() < 1e-3, "实际 {half}");
    }

    /// 判据 2：`NaN` / `±∞` 输入 ⇒ 显示有限、且**永远不是** `"NaN"`。
    #[test]
    fn nan_frames_never_render_as_nan() {
        assert_eq!(sanitize_dbfs(f32::NAN), METER_FLOOR_DBFS);
        assert_eq!(sanitize_dbfs(f32::NEG_INFINITY), METER_FLOOR_DBFS);
        // ⭐ T-FIX-521：`+∞` 落到非有限上界；**有限的正值原样保留**（旧实现夹成 0.0 ⇒ 削波看不见）。
        assert_eq!(sanitize_dbfs(f32::INFINITY), METER_CEIL_DBFS);
        assert_eq!(sanitize_dbfs(6.0), 6.0, "有限的正 dBFS 不再被截成 0.0");
        assert_eq!(sanitize_dbfs(1e30), 1e30, "有限值只受下限约束");
        assert_eq!(sanitize_dbfs(-999.0), METER_FLOOR_DBFS);
        for value in [f32::NAN, f32::NEG_INFINITY, f32::INFINITY, 1e30, -1e30] {
            let text = display_dbfs(value);
            assert!(!text.contains("NaN"), "`{text}` 里不许出现 NaN");
            assert!(text.parse::<f32>().expect("可解析").is_finite());
            assert!(peak_fraction(value).is_finite());
        }

        // 手工构造的 `NaN` 帧（绕过引擎的生产侧钳位）也不得污染界面形态。
        let hostile = MeterFrame {
            node: EntityId::default(),
            quantum: 7,
            peak: f32::NAN,
            peak_l: f32::NAN,
            peak_r: f32::NAN,
            peak_hold: f32::NAN,
            rms: f32::NAN,
            rms_smoothed: f32::NAN,
            momentary_lufs: f32::NAN,
            short_term_lufs: f32::NAN,
        };
        let meter = ChannelMeter::from_frame(0, "敌意轨道".to_owned(), &hostile);
        assert_eq!(meter.peak_label, "-120.0");
        assert_eq!(meter.rms_label, "-120.0");
        assert_eq!(meter.level, 0.0);
    }

    /// 判据 3：面板 → 快照的**对齐是按身份**（换序不换值，缺帧给静音）。
    #[test]
    fn snapshot_joins_frames_by_identity_not_by_index() {
        let project = demo_project();
        let view = ViewState::from_project(&project).expect("投影");
        let (mut publisher, collector) = meter_channel(64);
        let mut runtime = MeterRuntime::with_collector(collector);

        let first = first_track_id(&project);
        // 满幅 ⇒ 0.0 dBFS；半幅 ⇒ -6.0 dBFS 左右。
        publisher.publish(&[
            MeterFrame::new(first, 3, 1.0, 1.0),
            MeterFrame::new(project.master_bus_track_id, 3, 0.5, 0.25),
        ]);
        let seen = runtime.poll(view.tracks.len() + 1);
        assert_eq!(seen, 2, "两个节点都要在面板里");

        let shot = runtime.snapshot(&view);
        assert_eq!(shot.tracks.len(), view.tracks.len());
        assert_eq!(shot.quantum, Some(3));
        assert_eq!(shot.nodes_seen, 2);
        assert!(shot.master_level() > 0.9, "半幅 ⇒ 柱高 > 0.9");
        let hit = shot
            .tracks
            .iter()
            .find(|track| track.node_id == first.to_canonical_string())
            .expect("第一轨在快照里");
        assert_eq!(hit.peak_label, "0.0");
        assert_eq!(hit.level, 1.0);
        assert_eq!(hit.name, view.tracks[hit.index].name, "名字来自投影");
        // 其余轨道**没有**帧 ⇒ 静音（不是复用了第一轨的值）。
        let others: Vec<&ChannelMeter> = shot
            .tracks
            .iter()
            .filter(|track| track.node_id != hit.node_id)
            .collect();
        assert!(!others.is_empty());
        for other in others {
            assert_eq!(other.quantum, None);
            assert_eq!(other.peak_label, "-120.0");
            assert_eq!(other.level, 0.0);
        }
    }

    /// T-FIX-114：主总线快照必须把**左右声道各自的峰值**投影成两根柱高。
    ///
    /// 注入一帧 `peak_l = 0.5` / `peak_r = 0.125` 的主总线读数（左强右弱）：
    /// 两柱高必须不同，且分别等于 `amplitude_fraction` 的换算值；契约面
    /// 单标量 `master_level` 仍是两声道最大（保持既有消费方语义）。
    #[test]
    fn master_snapshot_projects_each_channel_level_separately() {
        let project = demo_project();
        let view = ViewState::from_project(&project).expect("投影");
        let (mut publisher, collector) = meter_channel(8);
        let mut runtime = MeterRuntime::with_collector(collector);

        publisher.publish(&[MeterFrame {
            node: project.master_bus_track_id,
            quantum: 1,
            peak: 0.5,
            peak_l: 0.5,
            peak_r: 0.125,
            peak_hold: 0.5,
            rms: 0.25,
            rms_smoothed: 0.25,
            momentary_lufs: f32::NEG_INFINITY,
            short_term_lufs: f32::NEG_INFINITY,
        }]);
        runtime.poll(1);
        let shot = runtime.snapshot(&view);

        let left = shot.master_level_l();
        let right = shot.master_level_r();
        assert!(
            (left - amplitude_fraction(0.5)).abs() < 1e-5,
            "左柱高应来自 peak_l: 实际 {left}"
        );
        assert!(
            (right - amplitude_fraction(0.125)).abs() < 1e-5,
            "右柱高应来自 peak_r: 实际 {right}"
        );
        assert!(
            left > right,
            "左强右弱必须体现为不同柱高: l={left} r={right}"
        );
        assert!(
            (shot.master_level() - left).abs() < 1e-5,
            "契约面单标量仍是两声道最大: {}",
            shot.master_level()
        );
    }

    /// ⭐ **T-FIX-521**：幅度 1.5（+3.52 dBFS）的过载帧 ⇒ 文本 `"+3.5"`、`over == true`，
    /// 柱高仍顶格 1.0；锁存要在 `MeterRuntime` 里留住，`clear_over_latches` 才清。
    #[test]
    fn an_over_peak_keeps_its_positive_dbfs_and_latches() {
        let project = demo_project();
        let view = ViewState::from_project(&project).expect("投影");
        let track = first_track_id(&project);
        let (mut publisher, collector) = meter_channel(8);
        let mut runtime = MeterRuntime::with_collector(collector);

        publisher.publish(&[MeterFrame::new(track, 3, 1.5, 0.5)]);
        runtime.poll(view.tracks.len() + 1);
        let shot = runtime.snapshot(&view);
        let meter = shot
            .tracks
            .iter()
            .find(|meter| meter.node_id == track.to_canonical_string())
            .expect("该轨道必须有读数");
        assert_eq!(
            meter.peak_dbfs,
            sanitize_dbfs(20.0 * 1.5_f32.log10()),
            "幅度 1.5 ⇒ +3.52 dBFS"
        );
        assert_eq!(
            meter.peak_label, "+3.5",
            "削波文本必须带号原值，⛔ 不是 \"0.0\""
        );
        assert!(meter.over, "峰值 > 0 dBFS ⇒ over 锁存为真");
        assert_eq!(meter.level, 1.0, "柱高仍夹到 1.0（顶格）");
        assert!(shot.overs().iter().any(|over| *over));

        // 锁存：下一拍没有过载帧，`over` 仍然为真（`MeterBoard` 保留上一帧也是 +3.5 那一帧）。
        publisher.publish(&[MeterFrame::new(track, 4, 0.25, 0.25)]);
        runtime.poll(view.tracks.len() + 1);
        let latched = runtime.snapshot(&view);
        let meter = latched
            .tracks
            .iter()
            .find(|meter| meter.node_id == track.to_canonical_string())
            .expect("该轨道必须有读数");
        assert!(meter.over, "过载必须锁存到清零为止");
        assert_eq!(
            meter.peak_label, "-12.0",
            "当下读数必须是新帧（锁存只锁 `over`）"
        );

        runtime.clear_over_latches();
        let cleared = runtime.snapshot(&view);
        let meter = cleared
            .tracks
            .iter()
            .find(|meter| meter.node_id == track.to_canonical_string())
            .expect("该轨道必须有读数");
        assert!(!meter.over, "清零之后下一拍必须回到当下（此时不过载）");
    }

    /// 判据 4：抽帧走的是 `drain_latest`（取最新），**不是** FIFO 前缀。
    ///
    /// 注入形态：同一个节点连发 5 个量子，一轮 `poll` 之后面板里必须是 `quantum = 9`
    /// 的那一帧。把 `drain_latest` 换成 `tick`（或把覆盖判据改成按顺序取）立刻变红。
    #[test]
    fn poll_uses_the_drain_latest_semantics_not_the_fifo_one() {
        let project = demo_project();
        let view = ViewState::from_project(&project).expect("投影");
        let track = first_track_id(&project);
        let (mut publisher, collector) = meter_channel(64);
        let mut runtime = MeterRuntime::with_collector(collector);

        for quantum in 5..=9 {
            publisher.publish(&[MeterFrame::new(track, quantum, 1.0, 1.0)]);
        }
        assert_eq!(runtime.poll(view.tracks.len() + 1), 1, "每节点只留一帧");
        let shot = runtime.snapshot(&view);
        assert_eq!(shot.quantum, Some(9), "必须是最新的那个量子");
        assert_eq!(
            publisher.dropped(),
            0,
            "队列容量足够时不该丢帧（丢帧是另一条可观测的健康指标）"
        );
    }

    /// 判据 5：迟到的旧帧**永远**顶不回新值（`supersedes` 的消费侧形态）。
    ///
    /// 注入形态：先发 `quantum = 9`，再发 `quantum = 2`；面板必须仍是 9。
    #[test]
    fn lagging_frames_never_regress_the_board() {
        let project = demo_project();
        let view = ViewState::from_project(&project).expect("投影");
        let track = first_track_id(&project);
        let (mut publisher, collector) = meter_channel(64);
        let mut runtime = MeterRuntime::with_collector(collector);

        publisher.publish(&[MeterFrame::new(track, 9, 1.0, 1.0)]);
        assert_eq!(runtime.poll(view.tracks.len() + 1), 1);
        publisher.publish(&[MeterFrame::new(track, 2, 0.001, 0.001)]);
        runtime.poll(view.tracks.len() + 1);

        // `drain_latest` 自己就会丢掉迟到的旧帧；这里再断言面板值也没变。
        let shot = runtime.snapshot(&view);
        assert_eq!(shot.quantum, Some(9), "旧帧不得回退面板");
        let hit = shot
            .tracks
            .iter()
            .find(|track_| track_.node_id == track.to_canonical_string())
            .expect("在快照里");
        assert_eq!(hit.peak_label, "0.0", "旧帧的 -60 dB 不许覆盖 0 dB");

        // 直接喂 `MeterBoard`（绕过 collector）也必须拒绝回退 —— 这一层是引擎的判据，
        // 这里从消费侧再钉一次"两个入口语义一致"。
        let mut board = MeterBoard::new();
        board.ingest(&[MeterFrame::new(track, 9, 1.0, 1.0)]);
        let updated = board.ingest(&[MeterFrame::new(track, 2, 0.001, 0.001)]);
        assert_eq!(updated, 0, "`ingest` 必须拒绝更旧的量子");
        assert_eq!(board.latest_quantum(), Some(9));
    }

    /// 判据 6：**通道条数 == 工程轨道数**，且换工程 ⇒ 名字与数值都变。
    #[test]
    fn strip_count_and_names_follow_the_projected_project() {
        let demo = ViewState::from_project(&demo_project()).expect("演示投影");
        let filled =
            ViewState::from_project(&yeban_model::samples::filled_project()).expect("样本投影");
        assert_ne!(
            demo.tracks.len(),
            filled.tracks.len(),
            "两个样本的轨道数必须不同, 否则这条判据没有分辨力"
        );

        let silent_demo = silent_snapshot(&demo);
        let silent_filled = silent_snapshot(&filled);
        assert_eq!(silent_demo.tracks.len(), demo.tracks.len());
        assert_eq!(silent_filled.tracks.len(), filled.tracks.len());
        assert_eq!(
            silent_demo.peak_labels(),
            vec!["-120.0".to_owned(); demo.tracks.len()]
        );
        let demo_names: Vec<&str> = silent_demo.tracks.iter().map(|t| t.name.as_str()).collect();
        let filled_names: Vec<&str> = silent_filled
            .tracks
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert_ne!(demo_names, filled_names, "换工程必须换通道条名字");
        assert_eq!(demo_names[0], "鼓");
        assert_eq!(filled_names[0], "Lead");
        assert!(silent_demo.master.is_some() && silent_filled.master.is_some());
    }

    /// 判据 7：`adopt`（引擎换代）清空面板 ⇒ 界面可观测到"电平回到下限"。
    #[test]
    fn adopt_resets_the_board_so_a_new_engine_starts_silent() {
        let project = demo_project();
        let view = ViewState::from_project(&project).expect("投影");
        let track = first_track_id(&project);
        let (mut publisher, collector) = meter_channel(16);
        let mut runtime = MeterRuntime::with_collector(collector);
        publisher.publish(&[MeterFrame::new(track, 4, 1.0, 1.0)]);
        assert_eq!(runtime.poll(view.tracks.len() + 1), 1);
        assert_eq!(runtime.snapshot(&view).quantum, Some(4));

        let (_new_publisher, new_collector) = meter_channel(16);
        runtime.adopt(new_collector);
        let after = runtime.snapshot(&view);
        assert_eq!(after.quantum, None, "换代之后旧读数必须消失");
        assert_eq!(after.nodes_seen, 0);
        assert_eq!(
            after.peak_labels(),
            vec!["-120.0".to_owned(); view.tracks.len()]
        );
        assert!(runtime.has_collector());
    }

    /// 判据 8：**没有引擎**时不 panic、不假装有值（`poll` 返回 0、快照全是下限）。
    #[test]
    fn no_engine_yields_silence_instead_of_pretending() {
        let view = ViewState::from_project(&demo_project()).expect("投影");
        let mut runtime = MeterRuntime::empty();
        assert!(!runtime.has_collector());
        assert_eq!(runtime.poll(view.tracks.len() + 1), 0);
        let shot = runtime.snapshot(&view);
        assert_eq!(shot.quantum, None);
        assert_eq!(shot.tracks.len(), view.tracks.len());
        assert!(shot.tracks.iter().all(|track| track.level == 0.0));
    }

    /// 判据 9：容量不足时**仍然抽干**（下一轮拿到的是最新帧，而不是队列里的陈货）。
    #[test]
    fn a_small_scratch_still_drains_the_backlog() {
        let project = demo_project();
        let view = ViewState::from_project(&project).expect("投影");
        let track = first_track_id(&project);
        let (mut publisher, collector) = meter_channel(64);
        let mut runtime = MeterRuntime::with_collector(collector);
        for quantum in 0..10 {
            publisher.publish(&[MeterFrame::new(track, quantum, 1.0, 1.0)]);
        }
        // `expected_nodes = 1` ⇒ scratch 只有一格；`drain_latest` 仍然抽干整个积压。
        assert_eq!(runtime.poll(1), 1);
        assert_eq!(runtime.snapshot(&view).quantum, Some(9));
    }

    /// 判据 10：主控 LUFS **读引擎帧的 BS.1770 瞬时读数**，不再由 RMS 推算。
    ///
    /// 旧公式 `rms_dbfs − 0.691` 在这里会给出 **−0.691**（满幅 RMS ⇒ 0.0 dBFS）；
    /// 新口径下同一帧的 LUFS 是帧里带的 −20.0（997 Hz、−20 dBFS 的标定信号
    /// 在引擎侧的真实读数，见 `yeban_engine::meter` 的参考向量判据）。
    /// 这条判据在**旧公式下是红的**（−0.691 ≠ −20.0），正是"假公式"的判别点。
    #[test]
    fn master_lufs_reads_the_engine_bs1770_momentary_reading() {
        let project = demo_project();
        let view = ViewState::from_project(&project).expect("投影");
        let silent = silent_snapshot(&view);
        assert_eq!(silent.master_lufs(), METER_FLOOR_DBFS);
        assert_eq!(
            silent.master_lufs_label(),
            "--",
            "静音快照没有真实帧 ⇒ 未测出，不许显示 -120.0"
        );

        let master_entity = project.master_bus_track_id;

        let (mut publisher, collector) = meter_channel(16);
        let mut runtime = MeterRuntime::with_collector(collector);
        // 满幅 RMS（0.0 dBFS）但帧里带的真实响度是 −20.0 LUFS：新口径读 −20.0，
        // 旧公式会读 −0.691 —— 两者差 19.3 dB，绝不是"标定偏移"能解释的。
        let mut frame = MeterFrame::new(master_entity, 1, 1.0, 1.0);
        frame.momentary_lufs = -20.0;
        frame.short_term_lufs = -20.0;
        publisher.publish(&[frame]);
        assert_eq!(runtime.poll(view.tracks.len() + 1), 1);
        let shot = runtime.snapshot(&view);

        assert!((shot.master_lufs() - (-20.0)).abs() < 1e-4);
        assert_eq!(shot.master_lufs_label(), "-20.0");
    }

    /// 判据 10b：帧里**还没有**响度读数（负无穷：窗口未满 / 静音 / 采样率不支持）
    /// 时，主控 LUFS 显示下限 —— 绝不拿 RMS 拼一个假读数顶上去。
    #[test]
    fn master_lufs_without_a_frame_reading_stays_at_the_floor() {
        let project = demo_project();
        let view = ViewState::from_project(&project).expect("投影");
        let master_entity = project.master_bus_track_id;

        let (mut publisher, collector) = meter_channel(16);
        let mut runtime = MeterRuntime::with_collector(collector);
        // 满幅 RMS 但响度字段是负无穷：旧公式会报 −0.691，新口径必须报下限。
        publisher.publish(&[MeterFrame::new(master_entity, 1, 1.0, 1.0)]);
        assert_eq!(runtime.poll(view.tracks.len() + 1), 1);
        let shot = runtime.snapshot(&view);

        assert_eq!(shot.master_lufs(), METER_FLOOR_DBFS);
        assert_eq!(
            shot.master_lufs_label(),
            "--",
            "帧里响度非有限 ⇒ 未测出，不许把显示下限冒充读数"
        );
    }

    /// 判据 11：引擎断流后**推进 N 跳** ⇒ stale 为真、文本 `"--"`，数值仍停在最后一帧；
    /// 新量子一到立刻复鲜（不是永久判死）。
    #[test]
    fn meter_goes_stale_after_n_ticks_without_a_new_quantum_and_recovers() {
        let project = demo_project();
        let view = ViewState::from_project(&project).expect("投影");
        let track = first_track_id(&project);
        let (mut publisher, collector) = meter_channel(16);
        let mut runtime = MeterRuntime::with_collector(collector);

        // 注入 5 个量子，每量子之后走一跳（与生产 60Hz 心跳同形）。
        for quantum in 1..=5 {
            publisher.publish(&[MeterFrame::new(track, quantum, 1.0, 1.0)]);
            assert_eq!(runtime.poll(view.tracks.len() + 1), 1);
        }
        let fresh = runtime.snapshot(&view);
        assert!(!fresh.stale, "量子在前进时不得 stale");
        assert!(
            !fresh.peak_labels().iter().any(|label| label == "--"),
            "fresh 时文本仍是 dBFS：{:?}",
            fresh.peak_labels()
        );

        // 停发之后推进 N 跳：恰好 N 跳还不 stale（窗口吸收 UI 抖动）。
        for _ in 0..METER_STALE_AFTER_TICKS {
            runtime.poll(view.tracks.len() + 1);
        }
        assert!(
            !runtime.is_stale(),
            "恰好 {METER_STALE_AFTER_TICKS} 跳边界内不得 stale"
        );

        // 第 N+1 跳才 stale（"差超 N 跳"）。
        runtime.poll(view.tracks.len() + 1);
        let stale = runtime.snapshot(&view);
        assert!(
            stale.stale,
            "第 {} 跳必须 stale",
            METER_STALE_AFTER_TICKS + 1
        );
        assert_eq!(stale.quantum, Some(5), "过期不改变最后一帧的量子读数");
        assert!(
            stale.peak_labels().iter().all(|label| label == "--"),
            "stale 时每轨峰值文本必须是 --：{:?}",
            stale.peak_labels()
        );
        assert!(
            stale.rms_labels().iter().all(|label| label == "--"),
            "stale 时每轨 RMS 文本必须是 --"
        );
        assert_eq!(stale.master_peak_label(), "--");
        assert_eq!(stale.master_rms_label(), "--");
        assert_eq!(stale.master_lufs_label(), "--");
        assert!(
            stale.levels().iter().any(|level| *level > 0.0),
            "柱高保留最后一帧（置灰由界面完成，不把读数抹成 0）"
        );

        // 新量子到达 ⇒ 立即复鲜。
        publisher.publish(&[MeterFrame::new(track, 6, 0.5, 0.5)]);
        runtime.poll(view.tracks.len() + 1);
        let recovered = runtime.snapshot(&view);
        assert!(!recovered.stale, "有新量子必须复鲜");
        assert!(
            !recovered.peak_labels().iter().any(|label| label == "--"),
            "复鲜后文本回到 dBFS：{:?}",
            recovered.peak_labels()
        );
    }

    /// 判据 12：长度失配的可见回执**一次失配只响一次**，恢复后重新武装。
    #[test]
    fn length_mismatch_notice_is_armed_once_until_lengths_recover() {
        let mut runtime = MeterRuntime::empty();
        assert!(runtime.note_length_mismatch(), "首次失配必须发声");
        assert!(!runtime.note_length_mismatch(), "连续失配不得刷屏");
        runtime.clear_length_mismatch();
        assert!(
            runtime.note_length_mismatch(),
            "长度恢复后再次失配必须重新发声"
        );
    }
}
