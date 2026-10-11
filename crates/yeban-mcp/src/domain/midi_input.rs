//! `yeban_midi_input_probe` 的实现：把**物理 MIDI 输入**接成一条 MCP 工具
//! （`ADR-0006`，feature `midi-input`）。
//!
//! ## 工具面（客户端看到的形状）
//!
//! ```text
//! yeban_midi_input_probe { "port": <十进制下标 或 名字子串>, "seconds": <可选, 缺省 3> }
//!   ⇒ { port: {index,name}, ports: [...], seconds, events,
//!        firstEvents: [{ timestamp_us, status, data1, data2 }],
//!        jitter: { n, p50, p99, max, caliber }, unsupportedMessages, readOnly: true }
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
//! 3. **`unsupportedMessages` 是**真读数****：`yeban-midi-io` 的
//!    `MidiEventReceiver::unsupported_messages()`（⭐ 经 `MidiInputSession::receiver()` 可达）
//!    暴露了**与生产端同一个**共享计数（⭐ 回调内核里一次 `fetch_add`）⇒ 本字段报**数字**，
//!    ⛔ 既不是 `0` 的占位、⛔ 也不是 `null`。口径写在 `unsupportedMessagesCaliber` 里
//!    （⭐ 读的是"连接之后到读它的那一刻"这段窗口内的登记数）。
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
    data: Value,
}

impl ProbeReport {
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
    let first: Vec<Value> = events
        .iter()
        .map(|event| {
            json!({
                "timestamp_us": event.timestamp_us,
                "status": event.status,
                "data1": event.data1,
                "data2": event.data2,
            })
        })
        .collect();
    Ok(ProbeReport {
        data: json!({
            "port": { "index": resolved.index, "name": resolved.name },
            "ports": ports
                .iter()
                .map(|port| json!({ "index": port.index, "name": port.name }))
                .collect::<Vec<_>>(),
            "seconds": request.seconds,
            "events": total,
            "firstEvents": first,
            "droppedByProbe": dropped_by_probe,
            "jitter": {
                "n": jitter.n,
                "p50": jitter.p50,
                "p99": jitter.p99,
                "max": jitter.max,
                "caliber": "interArrivalUs",
            },
            // ⚠ 诚实登记：会话句柄没暴露生产端的未承载计数 ⇒ `null`，⛔ 不假装 0。
            "unsupportedMessages": session.receiver().unsupported_messages(),
            "unsupportedMessagesCaliber":
                "unsupported_messages: 会话连接之后到本工具读它的那一刻, 回调内核登记的不承载消息条数 (系统专用/可变长/长度不符 ⇒ 不静默丢弃)",
            "readOnly": true,
        }),
    })
}
