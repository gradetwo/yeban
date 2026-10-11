//! `yeban_midi_input_probe` 的实现：把**物理 MIDI 输入**接成一条 MCP 工具
//! （`ADR-0006`，feature `midi-input`）。
//!
//! ## 工具面（客户端看到的形状）
//!
//! ```text
//! yeban_midi_input_probe { "port": <十进制下标 或 名字子串>, "seconds": <可选, 缺省 3> }
//!   ⇒ { port: {index,name}, ports: [...], seconds, events,
//!        firstEvents: [{ timestamp_us, status, data1, data2 }],
//!        jitter: { n, p50, p99, max, caliber },
//!        unsupportedMessages, unsupportedMessagesCaliber,
//!        droppedEvents, droppedEventsCaliber, readOnly: true }
//! ```
//!
//! ## 五条诚实登记（⛔ 不假装）
//!
//! 1. **工具集固定，能力可不可用**：`midi-input` **默认关**（`Cargo.toml` 的
//!    `default = []`）⇒ 未启用时本工具**仍在注册表里**，但**执行**回**结构化不可用**
//!    （`CONFLICT` ＋ `data.reason = "midiInputFeatureDisabled"`）。
//! 2. **无端口 ⇒ 结构化不可用，⛔ 不是"收到 0 条事件"**：枚举回空表时回
//!    `CONFLICT` ＋ `data.reason = "midiInputNoPorts"` ⇒ 调用方**区分得出**
//!    "这台机器没有 MIDI 输入"与"有端口但这段窗口里静默"（后者才是 `events: 0`）。
//! 3. **两个计数器都是**真读数****（⛔ 既不是 `0` 的占位、⛔ 也不是 `null`）：
//!    `yeban-midi-io` 把回调内核的**两个**计数镜像到了接收侧/会话侧
//!    （⭐ `MidiEventReceiver::unsupported_messages()` / `dropped_events()`，
//!    经 `MidiInputSession` 同名方法与 `receiver()` 可达）⇒ 本工具报**数字**：
//!    `unsupportedMessages` 登记**不承载**的消息（系统专用 / 可变长 / 长度不符）；
//!    `droppedEvents` 是 ⭐ **环满丢弃**的事件条数（⭐ 采集很要紧的那个数）。
//!
//!    两者的口径都写在同名的 `…Caliber` 字段里；`droppedEvents` 的口径逐字含三件事：
//!    **单位 = 事件条数**、**累计量（读它不消费）**、**窗口 = 从建环到读它的那一刻**
//!    （即采集窗口结束时的值）。
//! 4. **`jitter` 的口径是"相邻事件到达间隔（微秒）"**：本工具**不知道**输入源的
//!    tempo/PPQ ⇒ ⛔ 不把 `ADR-0006` D4 的"相对会话起点的 960 PPQ 偏差"谎报成读数；
//!    字段名带 `caliber`，逐字写 `interArrivalUs`，分位数由
//!    `yeban_midi_io::jitter_reading` 算（它只做分位数，不解释单位）。
//! 5. **`seconds` 是真的等待**：`plan` 会阻塞到窗口结束（上限 [`PROBE_MAX_SECONDS`]），
//!    期间持续排空环；响应 `droppedByProbe` 如实报"超出存储上限而没进 `firstEvents`
//!    的条数"（`events` 仍是**收到的总数**）。
//!
//! ## 与 `yeban_query_engine_state` 的分工
//!
//! 后者报的是**本进程/引擎的会话读数**（设备链、时钟等，**不碰硬件端口**）；
//! 本工具**真的打开一个物理输入端口**并收一段事件流。两者都不做对方的事。
//!
//! [`MidiInputSession`]: ../../../yeban_midi_io/struct.MidiInputSession.html

use serde_json::{Map, Value, json};

use crate::domain::error::Fault;
use crate::tools::ErrorCode;

/// `seconds` 的缺省值（与工具文档一致）。
pub const PROBE_DEFAULT_SECONDS: f64 = 3.0;

/// `seconds` 的上限：超过它一律 `INVALID_PARAMETER_RANGE`（⛔ 不静默截断）。
///
/// 为什么要上限：本工具在**请求线程**上真的等这段窗口 ⇒ 一个拼错的
/// `3600000` 会把 RPC 连接挂住一小时。上限是**显式**的取舍，写在文档里。
pub const PROBE_MAX_SECONDS: f64 = 60.0;

/// `firstEvents` 最多回传几条（`events` 仍是收到的**总数**）。
pub const PROBE_FIRST_EVENTS: usize = 8;

/// 环容量（与 `yeban-midi-io` 的缺省一致；消费侧持续排空）。
pub const PROBE_EVENT_CAPACITY: usize = 1024;

/// 解析后的探针请求。
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeRequest {
    /// 端口选择：`None` = 未给（只有**恰好一个**端口时才合法）。
    pub port: Option<String>,
    /// 收取窗口（秒，已校验 `0 < seconds <= PROBE_MAX_SECONDS`）。
    pub seconds: f64,
}

/// 探针的产物（只读；`apply` 原样回传，一位都不改）。
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeReport {
    /// ⚠ **私有**字段：本 crate 之外**构造不出** `ProbeReport`（⛔ 不能自己编一份载荷）。
    data: Value,
}

impl ProbeReport {
    /// 由**纯读数**组装载荷（**唯一**的构造入口）。
    ///
    /// 把载荷组装收在这一个函数里是**结构性**的保证，不是措辞：
    /// [`ProbeReport::data`] 的字段私有 ⇒ 本模块之外无法构造本类型；
    /// 本模块内**只有这一处**给它赋值。于是"判据判的那条载荷路径"
    /// **必然**就是线上那条路径，不会出现第二份 `json!` 让判据空转。
    ///
    /// 判据 `the_payload_is_built_in_exactly_one_place` 用源码扫描把
    /// "生产区里 `unsupportedMessages` **与** `droppedEvents` 这两个键各**恰好出现一次**
    /// 且都落在 [`probe_payload`] 体内"钉住 —— 挡住"日后有人再加一处 `json!`"。
    #[must_use]
    pub fn from_reading(reading: &ProbeReading) -> Self {
        Self {
            data: probe_payload(reading),
        }
    }

    /// 工具结果的 JSON 载荷（`dryRun` 预览与真调用**共用**这一个函数）。
    #[must_use]
    pub fn data(&self) -> Value {
        self.data.clone()
    }
}

/// 结构化不可用（⛔ 不 panic、⛔ 不假装成功）。
///
/// ⚠ **载荷在调用点给出**（`json!({ "reason": "…", … })`），⛔ 不在这里拼
/// —— 本 crate 的 `tests/fault_vocabulary.rs` 冻结**生产区**里出现的每一个
/// `"reason"` 字面量；在助手里拼字符串会让这些 reason **游离在词表之外**
///（改了没有任何东西会红）。
fn unavailable(message: String, data: Value) -> Fault {
    Fault::domain_with_data(ErrorCode::Conflict, message, data)
}

/// 解析实参（**纯函数**：⛔ 不碰硬件、⛔ 不读时钟）。
///
/// 这里只做**与硬件无关**的校验（空端口名、`seconds` 区间）⇒ 因此它在
/// "没有 MIDI 硬件"的机器上（含 CI）也能被完整判据覆盖。
///
/// # Errors
///
/// `port` 是空串 ⇒ `INVALID_PARAMETER_RANGE`（空串"匹配一切"是歧义，不是"随便挑一个"）；
/// `seconds` ≤ 0、> [`PROBE_MAX_SECONDS`] 或不是有限数 ⇒ `INVALID_PARAMETER_RANGE`。
pub fn parse_arguments(arguments: &Map<String, Value>) -> Result<ProbeRequest, Fault> {
    let port = match arguments.get("port") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Err(Fault::domain_with_data(
                    ErrorCode::InvalidParameterRange,
                    "`port` 不得是空串: 空串会匹配**任意**端口名, 属于歧义而不是\"随便挑一个\""
                        .to_owned(),
                    json!({
                        "reason": "midiInputEmptyPortName",
                        "field": "port",
                        "hint": "给十进制端口下标 (例如 \"0\") 或端口名的非空子串",
                    }),
                ));
            }
            Some(trimmed.to_owned())
        }
        Some(other) => {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!("`port` 必须是字符串 (十进制下标或端口名子串), 实际收到 {other}"),
                json!({ "reason": "midiInputPortTypeMismatch", "field": "port" }),
            ));
        }
    };
    let seconds = match arguments.get("seconds") {
        None | Some(Value::Null) => PROBE_DEFAULT_SECONDS,
        Some(Value::Number(number)) => number.as_f64().unwrap_or(f64::NAN),
        Some(other) => {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!("`seconds` 必须是数字, 实际收到 {other}"),
                json!({ "reason": "midiInputSecondsTypeMismatch", "field": "seconds" }),
            ));
        }
    };
    if !seconds.is_finite() || seconds <= 0.0 || seconds > PROBE_MAX_SECONDS {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`seconds` 必须在 (0, {PROBE_MAX_SECONDS}] 秒内, 实际收到 {seconds}"),
            json!({
                "reason": "midiInputSecondsOutOfRange",
                "field": "seconds",
                "minExclusive": 0.0,
                "maxInclusive": PROBE_MAX_SECONDS,
                "default": PROBE_DEFAULT_SECONDS,
            }),
        ));
    }
    Ok(ProbeRequest { port, seconds })
}

// ---------------------------------------------------------------------------
// 纯读数 → 载荷：**与硬件完全解耦**的那一层
// ---------------------------------------------------------------------------

/// `unsupportedMessages` 的**口径文案**（单一事实源：载荷与判据都读它）。
///
/// ⭐ 口径 = "会话连接之后到本工具读它的那一刻，回调内核登记的**不承载**消息条数"
/// （系统专用 / 可变长 / 长度不符 ⇒ 计数登记，⛔ 不静默丢弃）。
/// ⇒ 它**不是**"本机一共遇到过多少条"，也**不是**"本次窗口内有多少条"。
pub const UNSUPPORTED_MESSAGES_CALIBER: &str = "unsupported_messages: 会话连接之后到本工具读它的那一刻, 回调内核登记的不承载消息条数 (系统专用/可变长/长度不符 ⇒ 不静默丢弃)";

/// `droppedEvents` 的**口径文案**（单一事实源：载荷与判据都读它）。
///
/// ⭐ 三件事逐字写在这里，⛔ 不留给调用方猜：
/// **① 单位 = 事件条数**；**② 累计量**（⭐ 读它**不消费** —— 与 `pop()` 不同，
/// 读一次不会让下一次读数变小）；**③ 窗口 = 从建环那一刻到本工具读它的那一刻**
/// （即本次采集窗口结束时的值）。
///
/// ⚠ 与 `droppedByProbe` 的**分工**（两个数不是一回事）：
/// `droppedByProbe` 是**本工具自己**为了限制回传条数而没放进 `firstEvents` 的条数
/// （探针侧，窗口内）；`droppedEvents` 是**环**因为满而丢弃的条数
/// （生产侧，建环起累计）⇒ 前者恒 ≤ `events`，后者与 `events` 无固定大小关系。
pub const DROPPED_EVENTS_CALIBER: &str = "droppedEvents: 单位 = 事件条数; 累计量 (读它不消费); 窗口 = 从建环那一刻到本工具读它的那一刻 (即采集窗口结束时的值) —— 环满丢弃的事件条数, 与 droppedByProbe (本工具为限制回传条数而没放进 firstEvents 的条数) 不是一回事";

/// `jitter` 的单位口径文案（单一事实源）。
pub const JITTER_CALIBER: &str = "interArrivalUs";

/// 一个 MIDI 输入端口的**纯数据**形状（⛔ 不含 `yeban_midi_io` 类型）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortEntry {
    /// `midir` 端口列表里的下标。
    pub index: usize,
    /// 端口名。
    pub name: String,
}

/// 一条 MIDI 事件的**纯数据**形状（⛔ 不含 `yeban_midi_io::MidiEvent`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeEvent {
    /// `midir` 的原始时间戳（微秒）。
    pub timestamp_us: u64,
    /// 状态字节。
    pub status: u8,
    /// 第一个数据字节。
    pub data1: u8,
    /// 第二个数据字节。
    pub data2: u8,
}

/// 抖动读数的**纯数据**形状（⛔ 不含 `yeban_midi_io::JitterReading`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JitterNumbers {
    /// 样本量（条）。
    pub n: usize,
    /// 中位偏差（微秒）。
    pub p50: u64,
    /// 99 分位偏差（微秒）。
    pub p99: u64,
    /// 最大偏差（微秒）。
    pub max: u64,
}

/// 一次探针的**纯读数**：全部字段都是"**已经量到的数**"。
///
/// ## 为什么要有这个类型（这是本轮的核心结构决定）
///
/// 原先"量数"与"拼载荷"是**同一个函数**（`plan()` 里的一个巨型 `json!`），
/// 而那个函数要 `open_input` ⇒ **没有 MIDI 硬件的机器（含 CI）根本走不到**，
/// 于是载荷这一层（含 `unsupportedMessages` 的**类型**与**取值口径**）
/// **从未被任何判据碰过** —— 一处"继承来的零覆盖"。
///
/// 把读数组装抽成 [`ProbeReading`] + [`probe_payload`] 之后：
///
/// - 本类型与 [`probe_payload`] **⛔ 不挂任何 feature**（始终编译）⇒ 默认档的 CI 腿
///   也会编译并执行针对它的判据（⛔ 不会"绿着跳过"）；
/// - 它们**⛔ 不含句柄、⛔ 不含 `Instant`、⛔ 不含任何设备类型** ⇒ 判据可**直接构造**
///   任意读数，含"计数器等于 `u64::MAX`"这种真机上要跑几百年才可能出现的输入；
/// - 设备那一层（枚举 / 连接 / 收窗口）**留在 [`plan`] 里**，它"无硬件不可判"这件事
///   照旧**如实登记**（见本模块头与 `tests/midi_input_probe.rs`）。
///
/// ⚠ 这**不是**"为了测试而造假"：`plan()` 量完数之后**只**通过本类型过一手，
/// 由 [`ProbeReport::from_reading`] 组装载荷 ⇒ 判据判的就是线上那条路径。
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeReading {
    /// 被选中的端口下标。
    pub port_index: usize,
    /// 被选中的端口名。
    pub port_name: String,
    /// 枚举到的全部端口（顺序 = 枚举顺序）。
    pub ports: Vec<PortEntry>,
    /// 收取窗口（秒）。
    pub seconds: f64,
    /// 收到的事件**总数**（⛔ 不是 `firstEvents` 的长度）。
    pub events: u64,
    /// 回传的前几条事件（最多 [`PROBE_FIRST_EVENTS`] 条）。
    pub first_events: Vec<ProbeEvent>,
    /// 因超出回传上限而**只计数不回传**的条数。
    pub dropped_by_probe: u64,
    /// 相邻到达间隔的分位数读数。
    pub jitter: JitterNumbers,
    /// ⭐ 回调内核登记的**不承载消息条数**（与生产端**同一个**共享计数）。
    ///
    /// ⚠ 类型是 `u64`：它是 `AtomicU64` 的累加值，**不会**为负、**不截断**。
    pub unsupported_messages: u64,
    /// ⭐ **环满丢弃**的事件条数（与生产端**同一个**共享计数，⭐ 读它不消费）。
    ///
    /// ⚠ 类型同样是 `u64`（`AtomicU64` 的累加值）；口径见
    /// [`DROPPED_EVENTS_CALIBER`]（单位 = 事件条数／累计量／窗口 = 建环→读它的那一刻）。
    pub dropped_events: u64,
}

/// **纯函数**：把一次探针的读数组装成工具载荷。
///
/// ⛔ 不碰 `open_input`、⛔ 不碰 `midir`、⛔ 不读时钟、⛔ 不读环境变量、
/// ⛔ 不分配端口 ⇒ 因此它在**没有 MIDI 硬件**的机器上也能被完整判据覆盖。
///
/// 这是本工具"可判的那一层"：**序列化**本身是一个能被独立验证的真实单元。
/// 反过来，"端口枚举 / 连接 / 收窗口"那一层无硬件不可判，**如实登记**、⛔ 不假装。
#[must_use]
pub fn probe_payload(reading: &ProbeReading) -> Value {
    json!({
        "port": { "index": reading.port_index, "name": reading.port_name },
        "ports": reading
            .ports
            .iter()
            .map(|port| json!({ "index": port.index, "name": port.name }))
            .collect::<Vec<_>>(),
        "seconds": reading.seconds,
        "events": reading.events,
        "firstEvents": reading
            .first_events
            .iter()
            .map(|event| {
                json!({
                    "timestamp_us": event.timestamp_us,
                    "status": event.status,
                    "data1": event.data1,
                    "data2": event.data2,
                })
            })
            .collect::<Vec<_>>(),
        "droppedByProbe": reading.dropped_by_probe,
        "jitter": {
            "n": reading.jitter.n,
            "p50": reading.jitter.p50,
            "p99": reading.jitter.p99,
            "max": reading.jitter.max,
            "caliber": JITTER_CALIBER,
        },
        // ⭐ **真读数**：与生产端同一个共享计数（回调内核里一次 `fetch_add`）。
        // ⛔ 既不是 `0` 的占位、⛔ 也不是 `null` —— 判据
        // `unsupported_messages_is_a_number_equal_to_the_input` 钉住这两条。
        "unsupportedMessages": reading.unsupported_messages,
        "unsupportedMessagesCaliber": UNSUPPORTED_MESSAGES_CALIBER,
        // ⭐ **真读数**：环满丢弃的条数（同样是与生产端共享的那个计数）。
        // ⛔ 不是 `null`、⛔ 不是 `0` 的占位 —— 判据
        // `dropped_events_is_a_number_and_never_null` /
        // `dropped_events_equals_the_input_value` 钉住这两条。
        "droppedEvents": reading.dropped_events,
        "droppedEventsCaliber": DROPPED_EVENTS_CALIBER,
        "readOnly": true,
    })
}

/// 端口选择（**只用枚举结果**，⛔ 不打开端口 ⇒ 纯函数，可判据）。
///
/// 规则（按此顺序，任一不合 ⇒ 结构化 `INVALID_PARAMETER_RANGE`）：
///
/// 1. 未给 `port` ⇒ 必须**恰好一个**端口，否则报"必须点名"（⛔ 不默认挑第一个：
///    两台同型号设备时"挑第一个"会让调用方以为自己在跟另一台说话）；
/// 2. 全十进制 ⇒ 当下标用，越界 ⇒ `midiInputNoSuchPortIndex`；
/// 3. 否则按**大小写不敏感的子串**匹配：0 个 ⇒ `midiInputUnknownPort`，
///    多个 ⇒ `midiInputAmbiguousPort`（`data.candidates` 列出全部候选）。
///
/// # Errors
///
/// 上述任一条被违反。
#[cfg(feature = "midi-input")]
fn resolve_port_index(
    ports: &[yeban_midi_io::MidiPortInfo],
    requested: Option<&str>,
) -> Result<usize, Fault> {
    let candidates = || {
        Value::Array(
            ports
                .iter()
                .map(|port| json!({ "index": port.index, "name": port.name }))
                .collect(),
        )
    };
    let Some(requested) = requested else {
        if ports.len() == 1 {
            return Ok(ports[0].index);
        }
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "枚举到 {} 个 MIDI 输入端口 ⇒ 必须用 `port` 点名 (下标或名字子串)",
                ports.len()
            ),
            json!({
                "reason": "midiInputPortRequired",
                "field": "port",
                "ports": candidates(),
            }),
        ));
    };
    if requested
        .chars()
        .all(|character| character.is_ascii_digit())
    {
        if let Ok(index) = requested.parse::<usize>()
            && ports.iter().any(|port| port.index == index)
        {
            return Ok(index);
        }
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("没有下标为 `{requested}` 的 MIDI 输入端口"),
            json!({
                "reason": "midiInputNoSuchPortIndex",
                "field": "port",
                "requested": requested,
                "ports": candidates(),
            }),
        ));
    }
    let needle = requested.to_lowercase();
    let matched: Vec<&yeban_midi_io::MidiPortInfo> = ports
        .iter()
        .filter(|port| port.name.to_lowercase().contains(&needle))
        .collect();
    match matched.as_slice() {
        [only] => Ok(only.index),
        [] => Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("没有端口名含 `{requested}` 的 MIDI 输入端口"),
            json!({
                "reason": "midiInputUnknownPort",
                "field": "port",
                "requested": requested,
                "ports": candidates(),
            }),
        )),
        many => Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{requested}` 匹配到 {} 个 MIDI 输入端口 ⇒ 换一个更长的子串或给下标",
                many.len()
            ),
            json!({
                "reason": "midiInputAmbiguousPort",
                "field": "port",
                "requested": requested,
                "candidates": many
                    .iter()
                    .map(|port| json!({ "index": port.index, "name": port.name }))
                    .collect::<Vec<_>>(),
            }),
        )),
    }
}

/// **未启用 `midi-input` 的构建**：结构化不可用（工具仍在注册表里）。
///
/// # Errors
///
/// 恒为 `CONFLICT` ＋ `data.reason = "midiInputFeatureDisabled"`。
#[cfg(not(feature = "midi-input"))]
pub fn plan(_request: &ProbeRequest) -> Result<ProbeReport, Fault> {
    Err(unavailable(
        "本构建未启用 `midi-input` feature ⇒ 没有物理 MIDI 输入后端".to_owned(),
        json!({
            "reason": "midiInputFeatureDisabled",
            "feature": "midi-input",
            "hint": "以 `--features midi-input` 构建 (`default` 不含它: release 默认不拖入平台后端)",
            "port": Value::Null,
        }),
    ))
}

/// **启用 `midi-input`**：枚举 → 选端口 → 打开 → 收一段窗口。
///
/// # Errors
///
/// ① 宿主不可用／连接失败 ⇒ `CONFLICT`（`midiInputHostUnavailable` / `midiInputConnectFailed`）；
/// ② 枚举到 0 个端口 ⇒ `CONFLICT` ＋ `midiInputNoPorts`（⛔ **不是** `events: 0` 的成功响应）；
/// ③ 端口名歧义／下标越界／未点名 ⇒ `INVALID_PARAMETER_RANGE`（见 [`resolve_port_index`]）。
#[cfg(feature = "midi-input")]
pub fn plan(request: &ProbeRequest) -> Result<ProbeReport, Fault> {
    let ports = yeban_midi_io::enumerate_input_ports().map_err(|error| {
        unavailable(
            format!("MIDI 宿主不可用: {error}"),
            json!({ "reason": "midiInputHostUnavailable", "port": Value::Null }),
        )
    })?;
    if ports.is_empty() {
        return Err(unavailable(
            "无可用 MIDI 输入端口 (枚举到 0 个)".to_owned(),
            json!({ "reason": "midiInputNoPorts", "ports": [] }),
        ));
    }
    let index = resolve_port_index(&ports, request.port.as_deref())?;
    let resolved = ports
        .iter()
        .find(|port| port.index == index)
        .expect("resolve_port_index 只返回枚举里的下标");
    // ⚠ `origin_us` 只影响 `MidiEvent::ticks_from`；本工具的**载荷不报 tick**
    //（⛔ 不谎报 `ADR-0006` D4 的"相对会话起点"口径）⇒ 这里给 0 并**不使用**它。
    let mut session =
        yeban_midi_io::open_input(index, 0, PROBE_EVENT_CAPACITY).map_err(|error| {
            unavailable(
                format!("连接 MIDI 输入端口 `{}` 失败: {error}", resolved.name),
                json!({
                    "reason": "midiInputConnectFailed",
                    "port": { "index": resolved.index, "name": resolved.name },
                }),
            )
        })?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(request.seconds);
    let mut events: Vec<yeban_midi_io::MidiEvent> = Vec::new();
    let mut timestamps_us: Vec<u64> = Vec::new();
    while std::time::Instant::now() < deadline {
        let mut drained_any = false;
        while let Some(event) = session.pop() {
            drained_any = true;
            timestamps_us.push(event.timestamp_us);
            if events.len() < PROBE_FIRST_EVENTS {
                events.push(event);
            }
        }
        if !drained_any {
            // 2 ms 轮询：控制面线程的一次探测，⛔ 不在实时路径上。
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    let total = timestamps_us.len();
    let dropped_by_probe = total.saturating_sub(events.len());
    let deltas: Vec<u64> = timestamps_us
        .windows(2)
        .map(|pair| pair[1].saturating_sub(pair[0]))
        .collect();
    let jitter = yeban_midi_io::jitter_reading(&deltas);
    // ⭐ 本函数**只负责"量数"**：把量到的东西装进【纯读数】。
    // ⭐ 载荷组装在 `ProbeReport::from_reading` → `probe_payload`（纯函数、无硬件可判）。
    // ⛔ 这里**不再**出现任何 `json!` —— 否则就会出现第二份载荷让判据空转。
    Ok(ProbeReport::from_reading(&ProbeReading {
        port_index: resolved.index,
        port_name: resolved.name.clone(),
        ports: ports
            .iter()
            .map(|port| PortEntry {
                index: port.index,
                name: port.name.clone(),
            })
            .collect(),
        seconds: request.seconds,
        events: total as u64,
        first_events: events
            .iter()
            .map(|event| ProbeEvent {
                timestamp_us: event.timestamp_us,
                status: event.status,
                data1: event.data1,
                data2: event.data2,
            })
            .collect(),
        dropped_by_probe: dropped_by_probe as u64,
        jitter: JitterNumbers {
            n: jitter.n,
            p50: jitter.p50,
            p99: jitter.p99,
            max: jitter.max,
        },
        // ⭐ 真读数：会话句柄暴露的、与**生产端同一个**共享计数
        //（`MidiInputSession::unsupported_messages()` → `MidiEventReceiver` 的 `AtomicU64`）。
        // ⛔ 不是 `null`、⛔ 不是 `0` 的占位。
        unsupported_messages: session.unsupported_messages(),
        // ⭐ 环满丢弃数（`MidiInputSession::dropped_events()` → `MidiEventReceiver` 的
        // `AtomicU64`）⇒ 与 `unsupportedMessages` **同一条**镜像路径、同一个组装点。
        dropped_events: session.dropped_events(),
    }))
}
