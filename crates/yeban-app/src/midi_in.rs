//! `--midi-in <端口>`：把**物理 MIDI 输入**接到用户能触发的 CLI 入口 [`ADR-0006`]。
//!
//! # 边界（⭐ 如实）
//!
//! * ⭐ FFI 面**留在 `yeban-midi-io`** ⇒ 本模块**只**用它的安全 API
//!   （[`yeban_midi_io::enumerate_input_ports`]／[`open_input`]／`event_channel`）；
//! * ⛔ 本模块**不出现** `use midir`、⛔ 不出现 `unsafe`；
//! * ⭐ **时间戳只消费**（⭐ `MidiEvent::timestamp_us` 来自 `midir` 原始值）⇒
//!   ⛔ app 侧不自造时间戳。
//!
//! # 两条入口（⭐ 用途不同、⛔ 不是两条实现）
//!
//! | 入口 | 返回 | 谁用 |
//! | :--- | :--- | :--- |
//! | [`capture`] | [`MidiInReport`]（**给人看的报告**：前 8 条 + 抖动 + 真读数） | `--midi-in` 的诊断输出 |
//! | [`capture_messages`] | [`CapturedMessages`]（**全部**消息 + 真读数） | `--midi-in-write-notes` / `--midi-in-render`（把消息变成音乐） |
//!
//! ⭐ 两者共用**同一次**采集循环与**同一个**装配点（[`build_report`]）⇒ ⛔ 不存在"报告说的事件数"
//! 与"配对看到的消息数"两个事实源。
//!
//! # 端口匹配规则（⭐ 可判定，⛔ 不猜）
//!
//! 1. ⭐ `--midi-in` 的取值**全是十进制数字** ⇒ 按**端口序号**匹配（⭐ `0` 起）；
//! 2. ⭐ 否则按**端口名的子串**匹配（⭐ **大小写敏感**，⭐ 与 `midir` 给的原始名字比较）；
//! 3. ⭐ 命中 **0** 个 ⇒ [`MidiInError::PortNotFound`]（⭐ **列出可用端口**）；
//! 4. ⭐ 命中 **>1** 个 ⇒ [`MidiInError::AmbiguousPort`]（⭐ **列出候选**、⛔ 不取第一个）。
//!
//! [`ADR-0006`]: ../../docs/adr/ADR-0006-hardware-midi-input-via-midir.md

/// ⭐ 本模块**自己的**端口读数类型（⛔ 不把可选依赖的类型暴露到默认档）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortInfo {
    /// 端口序号（`0` 起）。
    pub index: usize,
    /// 端口名。
    pub name: String,
}

/// ⭐ 本模块**自己的**抖动读数类型（⚠ `RSK-22` 临时口径，**待裁**）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jitter {
    /// 样本量。
    pub n: usize,
    /// 中位偏差（Tick）。
    pub p50: u64,
    /// 99 分位偏差（Tick）。
    pub p99: u64,
    /// 最大偏差（Tick）。
    pub max: u64,
}

/// ⭐ 本地序统计量（⭐ 与 `yeban_midi_io::jitter_reading` **同口径**；⭐ `midi-input` 档下
/// 有一条判据断言两者**逐字段相同**）。
#[must_use]
pub fn jitter_reading(deviations_ticks: &[u64]) -> Jitter {
    if deviations_ticks.is_empty() {
        return Jitter {
            n: 0,
            p50: 0,
            p99: 0,
            max: 0,
        };
    }
    let mut sorted = deviations_ticks.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    let at = |numerator: usize, denominator: usize| -> u64 {
        let rank = (n * numerator).div_ceil(denominator).max(1);
        sorted[rank.min(n) - 1]
    };
    Jitter {
        n,
        p50: at(1, 2),
        p99: at(99, 100),
        max: sorted[n - 1],
    }
}

/// ⭐ 微秒**差** → Tick（960 PPQ；⭐ 与 `yeban_midi_io::ticks_from_origin_us` 同一算式）。
#[must_use]
pub fn ticks_from_gap_us(gap_us: u64) -> u64 {
    gap_us.saturating_mul(960) / 1_000_000
}

/// ⭐ `--midi-in` 的**默认**采集时长（秒）。
pub const DEFAULT_SECONDS: f32 = 3.0;

/// ⭐ 采集时长的上限（秒）—— ⭐ 防止误敲一个巨大的数让命令**永不退出**。
pub const MAX_SECONDS: f32 = 600.0;

/// ⭐ `--midi-in` 的失败面（⭐ 每一种都带**精确**原因，⛔ 不 panic）。
#[derive(Debug, Clone, PartialEq)]
pub enum MidiInError {
    /// ⭐ **本构建未启用 `midi-input` feature**（⭐ 默认档就是这样）。
    ///
    /// ⚠ ⭐ 这条**必须**说人话：⭐ 点名 feature ＋ ⭐ 一句怎么开 ⇒
    /// ⛔ 不许退化成"未知参数"。
    FeatureDisabled,
    /// ⭐ 采集时长非法（⭐ 必须 `0 < seconds <= MAX_SECONDS`）。
    InvalidSeconds {
        /// 调用方给的值。
        given: f32,
        /// 上限（秒）。
        max: f32,
    },
    /// ⭐ 端口**一个都没匹配上**（⭐ 带可用端口清单）。
    PortNotFound {
        /// 调用方给的取值。
        wanted: String,
        /// 当前**可用**的端口名（⭐ 可能为空 = 无端口）。
        available: Vec<String>,
    },
    /// ⭐ 端口名子串**命中多于一个**（⭐ 带候选清单、⛔ 不猜）。
    AmbiguousPort {
        /// 调用方给的取值。
        wanted: String,
        /// 命中的候选端口名。
        matches: Vec<String>,
    },
    /// ⭐ `midir` 宿主不可用（⭐ 例：⭐ 无声卡环境）。
    HostUnavailable(String),
    /// ⭐ 连接端口失败。
    ConnectFailed {
        /// 目标端口名。
        port: String,
        /// `midir` 的原因。
        detail: String,
    },
}

impl MidiInError {
    /// ⭐ 退出码：⭐ 与 `--open` 失败同一档（[`crate::cli::EXIT_OPEN`] = **3**）。
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        crate::cli::EXIT_OPEN
    }

    /// ⭐ 可用端口清单的渲染（⭐ 空清单也要**说出来**，⛔ 不留空）。
    fn render_available(available: &[String]) -> String {
        if available.is_empty() {
            "（当前**没有任何**可用 MIDI 输入端口）".to_owned()
        } else {
            format!("（可用端口: {}）", available.join(" | "))
        }
    }
}

impl core::fmt::Display for MidiInError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::FeatureDisabled => write!(
                formatter,
                "本构建未启用 `midi-input` feature ⇒ `--midi-in` 不可用。\
                 重新构建时加上它: `cargo build -p yeban-app --features midi-input`"
            ),
            Self::InvalidSeconds { given, max } => write!(
                formatter,
                "`--midi-in-seconds` 的取值 {given} 非法 (必须 > 0 且 <= {max})"
            ),
            Self::PortNotFound { wanted, available } => write!(
                formatter,
                "没有匹配 `{wanted}` 的 MIDI 输入端口 {}",
                Self::render_available(available)
            ),
            Self::AmbiguousPort { wanted, matches } => write!(
                formatter,
                "`{wanted}` 命中多于一个 MIDI 输入端口 ({} 个候选: {}) —— \
                 请给更长的子串或改用端口序号",
                matches.len(),
                matches.join(" | ")
            ),
            Self::HostUnavailable(reason) => {
                write!(formatter, "MIDI 宿主不可用: {reason}")
            }
            Self::ConnectFailed { port, detail } => {
                write!(formatter, "连接 MIDI 输入端口 `{port}` 失败: {detail}")
            }
        }
    }
}

impl std::error::Error for MidiInError {}

/// ⭐ 一次采集的**报告**（⭐ app 侧打印它）。
#[derive(Debug, Clone, PartialEq)]
pub struct MidiInReport {
    /// ⭐ 会话起点（⭐ **显式**记录，⛔ 不是回调到达时刻）。
    pub origin_us: u64,
    /// ⭐ 实际采集时长（秒）。
    pub seconds: f32,
    /// ⭐ 收到的**完整事件**条数。
    pub events: u64,
    /// ⭐ 前几条事件的 `(timestamp_us, status, data1, data2)`。
    pub first_events: Vec<(u64, u8, u8, u8)>,
    /// ⭐ **不承载**的消息条数（⭐ 单位 = **条**，⛔ 不是字节）：⭐ 系统专用（`F0`…`F7`）／
    /// 可变长／长度不符的消息**被计数登记**（⛔ 不静默丢弃）。
    ///
    /// ⚠ ⭐ **口径**：⭐ 由**生产者**（⭐ `midir` 回调体；⭐ 内核即
    /// `yeban_midi_io::MidiEventSender::handle_message` 的 `None` 分支）在**共享原子**
    /// 上计数，⭐ 接收侧读**同一个**原子 ⇒ ⭐ 本字段是**真读数**（⭐ 不是 app 自己重新数的、
    /// ⭐ 也不是占位常量）。
    ///
    /// ⚠ ⭐ 它是**累计量** ⇒ ⭐ 读它**不消费**任何东西（⭐ 多次读得到同一个值）。
    pub unsupported: u64,
    /// ⭐ **临时**抖动读数（⭐ `RSK-22` 口径**待裁**）；⭐ 样本不足时为 `None`。
    pub jitter: Option<Jitter>,
}

/// ⭐ 一次采集的**全部原始消息** ＋ 真读数（⭐ 音符配对与引擎调度的输入）。
///
/// ⚠ ⭐ 为什么需要它（⭐ 而不是把 `MidiInReport` 撑大）：`MidiInReport` 是**给人看的报告**
/// （只留前 [`MidiInReport::FIRST_EVENTS`] 条 + 抖动读数），而"把这次采到的东西变成音乐"
/// 需要**每一条**消息。两个消费者要的东西不同 ⇒ 两个类型，⛔ 不用一个类型兼职
/// （兼职的那一天，"报告"与"素材"就会开始互相将就）。
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedMessages {
    /// ⭐ 会话起点（微秒）—— tick 换算的零点参照。
    pub origin_us: u64,
    /// ⭐ 实际采集时长（秒）。
    pub seconds: f32,
    /// ⭐ **全部**承载的消息 `(timestamp_us, status, data1, data2)`（按到达顺序）。
    pub messages: Vec<(u64, u8, u8, u8)>,
    /// ⭐ **不承载**的条数（⭐ 真读数：生产者计数、接收侧读同一个共享原子）。
    pub unsupported: u64,
}

impl CapturedMessages {
    /// ⭐ 装配成给人看的报告（⭐ **唯一**装配点是 [`build_report`]）。
    #[must_use]
    pub fn report(&self) -> MidiInReport {
        let timestamps_us: Vec<u64> = self.messages.iter().map(|row| row.0).collect();
        build_report(
            self.origin_us,
            self.seconds,
            u64::try_from(self.messages.len()).unwrap_or(u64::MAX),
            self.messages
                .iter()
                .take(MidiInReport::FIRST_EVENTS)
                .copied()
                .collect(),
            self.unsupported,
            &timestamps_us,
        )
    }
}

impl MidiInReport {
    /// ⭐ 打印前几条事件的条数上限。
    pub const FIRST_EVENTS: usize = 8;

    /// ⭐ 单行摘要（⭐ 供 `lines.push`）。
    #[must_use]
    pub fn summary(&self) -> String {
        let jitter = match self.jitter {
            Some(reading) => format!(
                "jitter(n={} p50={} p99={} max={}) 单位=Tick 口径=临时待裁",
                reading.n, reading.p50, reading.p99, reading.max
            ),
            None => "jitter(n=0 样本不足)".to_owned(),
        };
        let head = self
            .first_events
            .iter()
            .map(|(t, s, d1, d2)| format!("[{t} {s:#04X} {d1} {d2}]"))
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "midi-in: port_origin_us={} seconds={} events={} unsupported={} {jitter} first={head} \
             负载=CLI 单进程采集(⛔ 无并发重活)",
            self.origin_us, self.seconds, self.events, self.unsupported
        )
    }
}

/// ⭐ **装配报告**（⭐ 纯函数：⭐ [`capture`] 与判据**共用**它）。
///
/// ⚠ ⭐ 为什么它是一个独立函数（⭐ 不是为了测试而测试）：⭐ `unsupported` 的**真读数**
/// 只有在 `capture()` 里才拿得到，而 `capture()` 必须**先连上一个真端口**才会走到那里
/// ⇒ ⭐ 若把这段装配写在 `capture()` 体内，⭐ 判据就**永远断言不到报告字段**，⭐ 只能
/// 断言计数器本身。⭐ 抽出来之后，⭐ 判据可以喂一个**真的**接收侧
/// （⭐ `yeban_midi_io::MidiEventReceiver`，⭐ 用 crate 的**模拟回调体** `handle_message`
/// 推消息 ⇒ ⭐ 与真 `midir` 回调走同一段内核）＋ ⭐ 真的计数值，⭐ 从而断言**报告字段**。
///
/// ⭐ 这也是"⭐ 无声卡也能判"的关键：⭐ 本函数零 I/O、零硬件、零时间依赖。
#[must_use]
pub fn build_report(
    origin_us: u64,
    seconds: f32,
    events: u64,
    first_events: Vec<(u64, u8, u8, u8)>,
    unsupported: u64,
    timestamps_us: &[u64],
) -> MidiInReport {
    MidiInReport {
        origin_us,
        seconds,
        events,
        first_events,
        unsupported,
        jitter: Some(inter_onset_jitter_ticks(timestamps_us, origin_us)),
    }
}

/// ⭐ 校验采集时长（⭐ 纯函数）。
fn validate_seconds(seconds: f32) -> Result<f32, MidiInError> {
    if seconds.is_finite() && seconds > 0.0 && seconds <= MAX_SECONDS {
        Ok(seconds)
    } else {
        Err(MidiInError::InvalidSeconds {
            given: seconds,
            max: MAX_SECONDS,
        })
    }
}

/// ⭐ **端口匹配规则**（⭐ 纯函数 ⇒ 无声卡也能判；⭐ 规则见模块文档）。
///
/// # Errors
///
/// ⭐ 命中 0 个 ⇒ [`MidiInError::PortNotFound`]；
/// ⭐ 命中 >1 个 ⇒ [`MidiInError::AmbiguousPort`]。
pub fn resolve_port(ports: &[PortInfo], wanted: &str) -> Result<usize, MidiInError> {
    let available = || {
        ports
            .iter()
            .map(|port| port.name.clone())
            .collect::<Vec<_>>()
    };
    let trimmed = wanted.trim();
    // ⭐ 规则 ①：⭐ 全十进制 ⇒ 按序号。
    if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
        // ⭐ 用 `filter` 一步写（⛔ 不用嵌套 `if` —— clippy `collapsible_if`）；
        // ⭐ 且**只解析一次**（⭐ 避免"解析两次"这种会漂移的写法）。
        if let Some(index) = trimmed
            .parse::<usize>()
            .ok()
            .filter(|index| ports.iter().any(|port| port.index == *index))
        {
            return Ok(index);
        }
        return Err(MidiInError::PortNotFound {
            wanted: wanted.to_owned(),
            available: available(),
        });
    }
    // ⭐ 规则 ②：⭐ 名字**子串**（⭐ 大小写敏感）。
    let matches: Vec<usize> = ports
        .iter()
        .filter(|port| port.name.contains(trimmed))
        .map(|port| port.index)
        .collect();
    match matches.as_slice() {
        [] => Err(MidiInError::PortNotFound {
            wanted: wanted.to_owned(),
            available: available(),
        }),
        [only] => Ok(*only),
        many => Err(MidiInError::AmbiguousPort {
            wanted: wanted.to_owned(),
            matches: many
                .iter()
                .filter_map(|index| ports.iter().find(|port| port.index == *index))
                .map(|port| port.name.clone())
                .collect(),
        }),
    }
}

/// ⭐ **`midi-input` 关**：⭐ 立刻给出结构化错误（⭐ 因此**默认档**也有证据）。
///
/// # Errors
///
/// ⭐ 恒 [`MidiInError::FeatureDisabled`]（⭐ 但**先校验秒数**，⭐ 让参数错误在两种档里一致）。
#[cfg(not(feature = "midi-input"))]
pub fn capture_messages(wanted: &str, seconds: f32) -> Result<CapturedMessages, MidiInError> {
    let _ = wanted;
    validate_seconds(seconds)?;
    Err(MidiInError::FeatureDisabled)
}

/// ⭐ **`midi-input` 关**：⭐ 立刻给出结构化错误（⭐ 因此**默认档**也有证据）。
///
/// # Errors
///
/// ⭐ 恒 [`MidiInError::FeatureDisabled`]（⭐ 但**先校验秒数**，⭐ 让参数错误在两种档里一致）。
#[cfg(not(feature = "midi-input"))]
pub fn capture(wanted: &str, seconds: f32) -> Result<MidiInReport, MidiInError> {
    let _ = wanted;
    validate_seconds(seconds)?;
    Err(MidiInError::FeatureDisabled)
}

/// ⭐ **`midi-input` 开**：⭐ 打开端口、采集 `seconds` 秒、**到点必然返回**。
///
/// ⭐ 返回**全部**消息（⛔ 不再只留前 8 条）—— 音符配对（[`crate::midi_notes`]）要看到每一条；
/// 给人看的报告由 [`CapturedMessages::report`] 从同一批数据装配（⛔ 不二次采集、⛔ 不双份解析）。
///
/// ⭐ **`origin_us` = 0** 是**显式**选择：⭐ 会话起点就是 `midir` 时间戳的零点参照
/// ⇒ ⭐ 记进 [`CapturedMessages::origin_us`]（⛔ 不用"回调到达时刻"当基准）。
///
/// # Errors
///
/// ⭐ 见 [`MidiInError`] 的六个变体（⭐ 端口不存在时**立刻返回**、⛔ 不进入采集循环）。
#[cfg(feature = "midi-input")]
pub fn capture_messages(wanted: &str, seconds: f32) -> Result<CapturedMessages, MidiInError> {
    let seconds = validate_seconds(seconds)?;
    let crate_ports = yeban_midi_io::enumerate_input_ports()
        .map_err(|error| MidiInError::HostUnavailable(error.to_string()))?;
    // ⭐ 只在这里出现可选依赖的类型 ⇒ 默认档完全不引用它。
    let ports: Vec<PortInfo> = crate_ports
        .iter()
        .map(|port| PortInfo {
            index: port.index,
            name: port.name.clone(),
        })
        .collect();
    let index = resolve_port(&ports, wanted)?;
    let port_name = ports
        .iter()
        .find(|port| port.index == index)
        .map_or_else(|| format!("#{index}"), |port| port.name.clone());
    let origin_us = 0_u64;
    let mut session =
        yeban_midi_io::open_input(index, origin_us, yeban_midi_io::DEFAULT_EVENT_CAPACITY)
            .map_err(|error| MidiInError::ConnectFailed {
                port: port_name,
                detail: error.to_string(),
            })?;

    // ⭐ 采集循环：⭐ **到点必然退出**（⛔ 不无限阻塞）。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f32(seconds);
    let mut events: u64 = 0;
    let mut first_events: Vec<(u64, u8, u8, u8)> = Vec::new();
    let mut timestamps_us: Vec<u64> = Vec::new();
    // ⭐ **全部**消息（⭐ 不再只留前 8 条）：⭐ 音符配对（`crate::midi_notes`）要看到**每一条**。
    // ⚠ 这是"内存里的消息表"，⭐ 采集期分配不违反红线 7（⭐ 本函数是控制侧，不是回调体）。
    let mut messages: Vec<(u64, u8, u8, u8)> = Vec::new();
    while std::time::Instant::now() < deadline {
        while let Some(event) = session.pop() {
            events = events.saturating_add(1);
            let row = (event.timestamp_us, event.status, event.data1, event.data2);
            if first_events.len() < MidiInReport::FIRST_EVENTS {
                first_events.push(row);
            }
            timestamps_us.push(event.timestamp_us);
            messages.push(row);
        }
        // ⭐ 控制侧（⭐ 非实时）⇒ 短睡不违反红线 7。
        std::thread::sleep(std::time::Duration::from_millis(2));
    }

    // ⭐ **不承载的条数**：⭐ 生产侧（⭐ `midir` 回调体，⭐ 内核 = `MidiEventSender::handle_message`
    // 的 `None` 分支）在**与接收侧共享**的 `Arc<AtomicU64>` 上计数 ⇒ 这里从**接收侧**
    // 读**同一个**原子 ⇒ ⭐ 拿到的就是真读数。
    //
    // ⚠ ⭐ 为什么写成 `session.receiver().unsupported_messages()`（⭐ 而不是更短的
    // `session.unsupported_messages()` 转发）：⭐ 判据要用**同一个方法**证明"报告里的值
    // = 生产侧数到的值"，而会话类型只能由**真硬件**构造（`open_input`）⇒ 判据构造不出它。
    // 走接收侧之后，判据喂一个 `event_channel` 造的真接收侧，⭐ 调用的就是**这一行调用的
    // 那个方法**（⭐ `MidiInputSession::unsupported_messages()` 本身也只是转发到它）。
    let unsupported = session.receiver().unsupported_messages();

    let _ = (events, first_events);
    Ok(CapturedMessages {
        origin_us,
        seconds,
        messages,
        unsupported,
    })
}

/// ⭐ **`midi-input` 开**：⭐ 打开端口、采集 `seconds` 秒、**到点必然返回**（⭐ 报告形态）。
///
/// ⭐ 本函数是 [`capture_messages`] ＋ [`MidiInReport`] 装配（⭐ 装配只在 [`build_report`] 一处）。
///
/// # Errors
///
/// ⭐ 见 [`MidiInError`] 的六个变体（⭐ 端口不存在时**立刻返回**、⛔ 不进入采集循环）。
#[cfg(feature = "midi-input")]
pub fn capture(wanted: &str, seconds: f32) -> Result<MidiInReport, MidiInError> {
    let captured = capture_messages(wanted, seconds)?;
    Ok(captured.report())
}

/// ⭐ **临时**抖动口径（⭐ `RSK-22` **待裁**）：⭐ 以**相邻事件间隔的中位数**为名义网格，
/// ⭐ 偏差 = |实际间隔 − 中位间隔|（⭐ 单位 = Tick）⇒ ⭐ 交给
/// [`yeban_midi_io::jitter_reading`] 取 p50／p99／max。
///
/// ⚠ ⭐ 样本 `< 3` ⇒ `n = 0` 的读数（⭐ 分母不足时**如实**给空）。
#[must_use]
pub fn inter_onset_jitter_ticks(timestamps_us: &[u64], _origin_us: u64) -> Jitter {
    let deltas: Vec<u64> = timestamps_us
        .windows(2)
        .map(|pair| pair[1].saturating_sub(pair[0]))
        .collect();
    if deltas.len() < 2 {
        return jitter_reading(&[]);
    }
    let mut sorted = deltas.clone();
    sorted.sort_unstable();
    let median = sorted[sorted.len() / 2];
    let deviations: Vec<u64> = deltas
        .iter()
        .map(|delta| {
            let gap_us = delta.abs_diff(median);
            // ⭐ 用**同一个**换算函数 ⇒ 口径与 crate 一致（⭐ 单位 = Tick）。
            ticks_from_gap_us(gap_us)
        })
        .collect();
    jitter_reading(&deviations)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ports(names: &[&str]) -> Vec<PortInfo> {
        names
            .iter()
            .enumerate()
            .map(|(index, name)| PortInfo {
                index,
                name: (*name).to_owned(),
            })
            .collect()
    }

    /// 判据 ①：⭐ 纯十进制取值 ⇒ ⭐ 按**序号**匹配（⭐ ⛔ 不按名字）。
    #[test]
    fn a_numeric_selector_matches_by_index() {
        let list = ports(&["IAC Bus 1", "IAC Bus 2"]);
        assert_eq!(resolve_port(&list, "0"), Ok(0));
        assert_eq!(resolve_port(&list, "1"), Ok(1));
        assert_eq!(resolve_port(&list, " 1 "), Ok(1), "两侧空白必须被裁掉");
    }

    /// 判据 ②：⭐ 非十进制取值 ⇒ ⭐ 按**名字子串**（⭐ 大小写敏感）。
    #[test]
    fn a_text_selector_matches_by_name_substring() {
        let list = ports(&["IAC Bus 1", "Launchpad MK3"]);
        assert_eq!(resolve_port(&list, "Launch"), Ok(1));
        assert_eq!(resolve_port(&list, "pad MK"), Ok(1), "子串可以在中间");
        // ⚠ ⭐ 大小写敏感 ⇒ ⭐ `launch` 命中 0 个（⭐ 判据把这条口径钉住）。
        assert!(matches!(
            resolve_port(&list, "launch"),
            Err(MidiInError::PortNotFound { .. })
        ));
    }

    /// 判据 ③：⭐ 命中 0 个 ⇒ `PortNotFound` ＋ ⭐ **文本里带可用端口清单**；⭐ 无端口时
    /// **明说"没有任何可用端口"**（⭐ ⛔ 不留空、⛔ 不 panic）。
    #[test]
    fn a_missing_port_names_the_available_ones() {
        let list = ports(&["IAC Bus 1"]);
        let error = resolve_port(&list, "nope").expect_err("必须失败");
        let text = error.to_string();
        assert!(text.contains("IAC Bus 1"), "必须列出可用端口: {text}");
        assert_eq!(error.exit_code(), crate::cli::EXIT_OPEN);
        // ⭐ 空端口清单也要说出来。
        let empty = resolve_port(&[], "anything").expect_err("必须失败");
        assert!(
            empty.to_string().contains("没有任何"),
            "无端口时必须明说: {empty}"
        );
    }

    /// 判据 ④：⭐ 命中多于一个 ⇒ `AmbiguousPort` ＋ ⭐ **列出全部候选**（⭐ ⛔ 不取第一个）。
    #[test]
    fn an_ambiguous_substring_lists_every_candidate() {
        let list = ports(&["IAC Bus 1", "IAC Bus 2", "Launchpad"]);
        let error = resolve_port(&list, "IAC").expect_err("必须失败");
        match &error {
            MidiInError::AmbiguousPort { matches, .. } => {
                assert_eq!(
                    matches,
                    &vec!["IAC Bus 1".to_owned(), "IAC Bus 2".to_owned()]
                );
            }
            other => panic!("必须报歧义，得到 {other:?}"),
        }
        let text = error.to_string();
        assert!(
            text.contains("IAC Bus 1") && text.contains("IAC Bus 2"),
            "{text}"
        );
    }

    /// 判据 ⑤：⭐ 秒数校验（⭐ 0／负数／NaN／超过上限 ⇒ 结构化错误）。
    #[test]
    fn the_duration_is_validated() {
        for bad in [0.0_f32, -1.0, f32::NAN, MAX_SECONDS + 1.0] {
            assert!(
                matches!(
                    validate_seconds(bad),
                    Err(MidiInError::InvalidSeconds { .. })
                ),
                "{bad} 必须被拒"
            );
        }
        assert_eq!(validate_seconds(DEFAULT_SECONDS), Ok(DEFAULT_SECONDS));
        assert_eq!(validate_seconds(MAX_SECONDS), Ok(MAX_SECONDS));
    }

    /// 判据 ⑥：⭐ **短样本 ⇒ 空读数**（⭐ 分母不足时 ⛔ 不编造）。
    #[test]
    fn the_provisional_jitter_is_empty_when_the_sample_is_too_small() {
        assert_eq!(inter_onset_jitter_ticks(&[], 0).n, 0);
        assert_eq!(inter_onset_jitter_ticks(&[1], 0).n, 0);
        assert_eq!(inter_onset_jitter_ticks(&[1, 2], 0).n, 0, "一个间隔不够");
        let steady: Vec<u64> = (0..10).map(|step| step * 62_500).collect();
        let reading = inter_onset_jitter_ticks(&steady, 0);
        assert_eq!(reading.n, 9, "10 个时间戳 ⇒ 9 个间隔");
        assert_eq!(reading.max, 0, "等间隔 ⇒ 偏差恒 0");
    }

    /// 判据 ⑦：⭐ **默认档**：⭐ `capture` 必须报 [`MidiInError::FeatureDisabled`]，
    /// ⭐ 文本**点名 `midi-input`** ＋ ⭐ 给出怎么开，⭐ 且 ⛔ **不 panic**。
    ///
    /// ⚠ ⭐ 本判据在**默认档**编译并运行（⭐ 故 CI 的默认腿就有它的证据）；
    /// ⭐ 在 `midi-input` 档下**不存在**（⭐ 两条 `capture` 是互斥的 `cfg`）。
    #[cfg(not(feature = "midi-input"))]
    #[test]
    fn without_the_feature_the_cli_says_exactly_that() {
        let error = capture("IAC", DEFAULT_SECONDS).expect_err("默认档必须失败");
        assert_eq!(error, MidiInError::FeatureDisabled);
        let text = error.to_string();
        assert!(text.contains("midi-input"), "必须点名 feature: {text}");
        assert!(text.contains("--features"), "必须给出怎么开: {text}");
        assert_eq!(error.exit_code(), crate::cli::EXIT_OPEN);
    }

    /// 判据 ⑧：⭐ **默认档**：⭐ 秒数错误**先于** feature 判定（⭐ 两档行为一致）。
    #[cfg(not(feature = "midi-input"))]
    #[test]
    fn the_duration_error_precedes_the_feature_error() {
        assert!(matches!(
            capture("IAC", 0.0),
            Err(MidiInError::InvalidSeconds { .. })
        ));
    }

    /// 判据 ⑨：⭐ **feature 档**：⭐ 端口不存在 ⇒ ⭐ **立刻**结构化错误
    /// （⭐ ⛔ 不进入采集循环）＋ ⭐ 文本带候选/无端口 ＋ ⛔ 不 panic。
    ///
    /// ⭐ "立刻"用**墙钟上界**判：⭐ 远小于采集时长本身。
    #[cfg(feature = "midi-input")]
    #[test]
    fn a_missing_port_fails_immediately_without_entering_the_capture_loop() {
        let wanted = "\u{1F6AB}no-such-port\u{1F6AB}";
        let started = std::time::Instant::now();
        let error = capture(wanted, 30.0).expect_err("不存在的端口必须失败");
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "必须立刻返回（实测 {elapsed:?}）⇒ ⛔ 没有进入采集循环"
        );
        assert!(
            matches!(
                error,
                MidiInError::PortNotFound { .. } | MidiInError::HostUnavailable(_)
            ),
            "必须是结构化错误，得到 {error:?}"
        );
        let text = error.to_string();
        assert!(
            text.contains("可用端口") || text.contains("没有任何") || text.contains("宿主不可用"),
            "文本必须点出可用端口或宿主不可用: {text}"
        );
        assert_eq!(error.exit_code(), crate::cli::EXIT_OPEN);
        eprintln!("DIAGNOSTIC midi_in 无端口路径: elapsed={elapsed:?} 结论={text}");
    }

    /// 判据 ⑩（⭐ **同规格交叉对账**，R217②）：⭐ `midi-input` 档下，⭐ 本模块的本地
    /// 序统计量与 Tick 换算必须与 `yeban_midi_io` 的**逐字段/逐值相同** ⇒
    /// ⭐ 防止"⭐ 计数器与被计数者各写一份规格"。
    #[cfg(feature = "midi-input")]
    #[test]
    fn the_local_statistics_agree_with_the_crate_under_the_feature() {
        let samples: Vec<u64> = (0..257).map(|step| (step * 7) % 23).collect();
        let mine = jitter_reading(&samples);
        let theirs = yeban_midi_io::jitter_reading(&samples);
        assert_eq!(
            (mine.n, mine.p50, mine.p99, mine.max),
            (theirs.n, theirs.p50, theirs.p99, theirs.max),
            "本地序统计量必须与 crate 同口径"
        );
        for gap in [0_u64, 1, 999, 1_000, 62_500, 1_000_000, u64::MAX / 2] {
            assert_eq!(
                ticks_from_gap_us(gap),
                yeban_midi_io::ticks_from_origin_us(gap, 0),
                "Tick 换算必须与 crate 同算式：gap={gap}"
            );
        }
    }

    /// 判据 ⑪（⭐ feature 档）：⭐ 采集时长校验与端口解析在**两种档**下行为一致。
    #[cfg(feature = "midi-input")]
    #[test]
    fn the_feature_configuration_also_validates_the_duration_first() {
        assert!(matches!(
            capture("IAC", -1.0),
            Err(MidiInError::InvalidSeconds { .. })
        ));
    }

    /// 判据 ⑫（⭐ **feature 档**，⭐ 行为判据）：⭐ **生产侧数到 N 条不承载消息 ⇒ 报告字段就是 N**
    /// （⭐ ⛔ 不是 0）。
    ///
    /// ⭐ 消息由 crate 的**模拟回调体**推进（⭐ `MidiEventSender::handle_message` 与真
    /// `midir` 回调**走同一段内核**）⇒ ⭐ 本判据**不需要任何 MIDI 硬件**。
    #[cfg(feature = "midi-input")]
    #[test]
    fn the_report_carries_the_unsupported_count_the_producer_saw() {
        let (mut sender, receiver) = yeban_midi_io::event_channel(8);
        // ⭐ 三条**系统专用**消息（`F0` … `F7`，可变长）⇒ 生产者按**条**计数 3。
        for _ in 0..3 {
            sender.handle_message(0, &[0xF0, 0x7E, 0x7F, 0x09, 0x01, 0xF7]);
        }
        // ⭐ 一条**承载**的消息（NoteOn）⇒ 它进环、⛔ 不进"不承载"计数。
        sender.handle_message(0, &[0x90, 60, 100]);
        assert_eq!(receiver.pending(), 1, "承载的那一条必须进环");
        assert_eq!(
            receiver.unsupported_messages(),
            3,
            "生产者只数不承载的那三条"
        );

        // ⭐ 报告字段：⭐ 走 `capture()` **同一个**装配函数 ＋ **同一个**接收侧读取。
        let report = build_report(0, 1.0, 1, Vec::new(), receiver.unsupported_messages(), &[0]);
        assert_eq!(report.unsupported, 3, "报告里必须是真读数（⛔ 不是 0）");
        assert!(
            report.summary().contains("unsupported=3"),
            "摘要必须打印真读数: {}",
            report.summary()
        );
        // ⭐ 读它**不消费**：⭐ 累计量 ⇒ 再读还是 3（⛔ 不是"读一次清零"的队列）。
        assert_eq!(receiver.unsupported_messages(), 3, "读它不消费任何东西");
    }

    /// 判据 ⑬（⭐ **feature 档**，⭐ 行为判据）：⭐ **没有任何不承载消息 ⇒ 确实是 0**
    /// （⭐ 与判据 ⑫ 成对 ⇒ ⭐ ⚠ **两条一起**才证明"不是把写死换成了另一种写死"）。
    #[cfg(feature = "midi-input")]
    #[test]
    fn the_report_is_zero_when_the_producer_saw_nothing_unsupported() {
        let (mut sender, receiver) = yeban_midi_io::event_channel(8);
        sender.handle_message(0, &[0x90, 60, 100]);
        sender.handle_message(1, &[0x80, 60, 0]);
        assert_eq!(receiver.unsupported_messages(), 0, "两条都承载 ⇒ 计数为 0");

        let report = build_report(
            0,
            1.0,
            2,
            Vec::new(),
            receiver.unsupported_messages(),
            &[0, 1],
        );
        assert_eq!(report.unsupported, 0, "无消息时必须是 0");
        assert!(
            report.summary().contains("unsupported=0"),
            "{}",
            report.summary()
        );
    }

    /// 判据 ⑭（⭐ **结构判据**，⭐ feature 档）：⭐ 生产路径**确实**从接收侧读那个共享计数，
    /// ⭐ 而文件里**不存在**写死的 `unsupported` 常量。
    ///
    /// ⚠ ⭐ 为什么需要它（⭐ 而不是纯行为判据就够了）：⭐ 判据 ⑫/⑬ 覆盖"装配 + 接收侧读取"，
    /// 但**覆盖不到** `capture()` 体内那一行（⭐ 它只有连上真端口才会执行，⭐ CI 无 MIDI 硬件）
    /// ⇒ ⭐ 若有人把那行改回常量，⭐ ⑫/⑬ 仍会全绿。⭐ 本判据把那一行的**文本**钉住
    /// （⭐ 手法沿用 `elements.rs` 对 `.slint` / `host.rs` 的文本契约判据）。
    #[cfg(feature = "midi-input")]
    #[test]
    fn the_capture_path_reads_the_shared_counter_and_has_no_placeholder() {
        let source = include_str!("midi_in.rs");
        assert!(
            source.contains("session.receiver().unsupported_messages()"),
            "生产路径必须从接收侧读共享计数"
        );
        // ⚠ ⭐ 自指陷阱：⭐ 本断言想要的那个字符串**不能**以字面形式出现在本文件里
        // （⭐ 否则它自己就会命中）⇒ ⭐ 用 `concat!` 拆开拼出来。
        let placeholder = concat!("unsupported", ": 0");
        assert!(
            !source.contains(placeholder),
            "⛔ 报告里不许再有写死的 {placeholder}"
        );
        // ⭐ 装配只有一个入口：⭐ feature 档的 `capture()` **经** `build_report` 装配
        // （⛔ 不许在别处再造一份报告）。
        assert!(
            source.contains("Ok(build_report("),
            "capture 必须经 build_report 装配报告"
        );
        assert_eq!(
            source.matches(concat!("pub fn ", "build_report(")).count(),
            1,
            "build_report 只能有一个定义"
        );
    }
}
