//! MCP 往返**载荷统计** `[BASELINE-006]`：量什么、单位是什么、为什么只量 JSON 那一半。
//!
//! ## 规范原文（`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:361`）
//!
//! | 规范 ID | 指标 | 测法 | 判据 |
//! | :--- | :--- | :--- | :--- |
//! | `[BASELINE-006]` | **AI 交互效率** | 统计生成 16 小节段落的**完整 MCP 工具往返载荷** | **序列化 JSON 载荷 ≤ 4 KB，结构化字段传输**，Token 开销中位数 ≤ 600 Tokens |
//!
//! 本模块只实现**能测的那一半**：序列化 JSON 的**字节数**。
//!
//! ## 为什么 Token 那一半**不**在这里实现（如实登记，不是遗漏）
//!
//! "Token 开销中位数 ≤ 600" 需要一个**分词器口径**（用哪个 tokenizer、算不算
//! 请求与响应的封装）。该口径由人类裁决，且已被明示**延后**
//! （`docs/ledger/gate-status.md` 的 `BASELINE-006` 行、`docs/DEVELOPMENT_LEDGER.md`
//! 的 PENDING 清单）。本模块**不发明**一个"字节数 ÷ 4"之类的代理指标冒充 Token：
//! 那会把一条待裁决的口径伪装成已实现的判据（`AGENTS.md` §6.1 的测量纪律）。
//! 因此 Token 半边保持 PENDING，JSON 半边由本模块给出可复跑的读数。
//!
//! ## 计数对象与单位（先说指标，再跑命令）
//!
//! - **对象**：一次 `tools/call` 的**线上 JSON 文本行**；
//! - **单位**：UTF-8 **字节**（`str::len()`，不是字符数、不是值里的数组长度）；
//! - **请求**：客户端发出的那一行文本本身（**不含** stdio 的换行与 HTTP 的头）；
//! - **响应**：`Response` 序列化后的字节数；notification 没有响应 ⇒ `0`。
//!
//! 复跑读数：`cargo test -p yeban-mcp --test payload_budget -- --nocapture`。
//!
//! ## 实测读数（本 crate 的 16 小节场景，单位：字节）
//!
//! 场景行由 [`baseline_006_section_line`] 给出（`request` 一列就是它的长度）。
//!
//! | `yeban_propose_section`（16 小节，`filled_project`） | 请求 | 响应 | 合计 | 对 4 KB |
//! | :--- | ---: | ---: | ---: | :--- |
//! | 回传完整 op 载荷（`includeOps: true`） | 214 | 9,456 | **9,670** | **2.36 × 超限** |
//! | 只回结构化字段（缺省，`includeOps` 不写） | 196 | 2,747 | **2,943** | 0.72 × 限额 |
//! | `dryRun: true` 的预览（缺省形状） | 210 | 2,708 | 2,918 | 0.71 × 限额 |
//!
//! 两行的差全部落在 `data.proposal.ops`（数组本身 6,702 字节 + 键名 7 字节）上。
//! 因此"结构化字段传输"这条规范措辞在实现里的落点是：**缺省**回传 `willCreate`
//! （派生清单）+ 提案摘要（`opCount` / `opKinds` / 提交身份），完整 `Op` 载荷改为
//! **显式可选**（[`crate::tools::INCLUDE_OPS_PARAM`]，默认 `false`）。

use std::fmt;

use crate::dispatch::{Dispatcher, Outcome};
use crate::jsonrpc::Response;
use crate::security::Channel;

/// `[BASELINE-006]` 对"生成 16 小节段落的完整 MCP 工具往返载荷"给出的 JSON 上限。
///
/// 单位：**字节**（序列化 JSON 的 UTF-8 长度，请求 + 响应）。
pub const BASELINE_006_JSON_LIMIT_BYTES: usize = 4096;

/// `[BASELINE-006]` 场景的小节数：规范原文写"**16 小节**段落"。
pub const BASELINE_006_SECTION_BARS: u32 = 16;

/// `[BASELINE-006]` 场景的风格标识。
///
/// 取自 `yeban-theory` 的流派库（`GenreLibrary::ids()` 里有它），因此这个场景是
/// **可成功执行**的真实调用，而不是一个必然报 `STYLE_NOT_FOUND` 的空壳。
pub const BASELINE_006_STYLE_PRESET: &str = "lo_fi_hip_hop";

/// 一次 MCP 工具往返的**序列化 JSON 载荷**读数（单位：UTF-8 字节）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoundTripPayload {
    /// 请求行（客户端发出的 JSON-RPC 文本）的字节数。
    pub request: usize,
    /// 响应行（服务端发出的 JSON-RPC 文本）的字节数；notification 为 `0`。
    pub response: usize,
}

impl RoundTripPayload {
    /// 往返载荷合计（字节）。
    ///
    /// 用饱和加法：读数异常（[`response_wire_bytes`] 的 `usize::MAX`）必须**变大**，
    /// 绝不因回绕而变小。
    #[must_use]
    pub const fn total(self) -> usize {
        self.request.saturating_add(self.response)
    }

    /// 合计是否满足 `[BASELINE-006]` 的"≤ 4 KB"判据。
    #[must_use]
    pub const fn within_baseline_006(self) -> bool {
        self.total() <= BASELINE_006_JSON_LIMIT_BYTES
    }
}

impl fmt::Display for RoundTripPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "request {} B + response {} B = {} B",
            self.request,
            self.response,
            self.total()
        )
    }
}

/// 响应对象的**线上字节数**（与传输层 `serde_json::to_string` 同口径）。
///
/// 序列化失败（`Response` 的 `Serialize` 是对 `serde_json::Value` 的包装，
/// 实测不可失败）返回 `usize::MAX`：读数异常必须让判据**变红**，
/// 绝不静默地记成一个更小的数。
#[must_use]
pub fn response_wire_bytes(response: &Response) -> usize {
    serde_json::to_string(response).map_or(usize::MAX, |text| text.len())
}

/// 量一次**真实**的工具往返：把 `line` 交给 `dispatcher`，返回读数与 [`Outcome`]。
///
/// 它不改变分发语义：这次调用**真的发生**（`dryRun` 短路、幂等缓存、作用域判定
/// 与线上完全一致）。要量"什么都没发生"的预览，就在 `line` 里给 `dryRun: true`。
pub fn measure(
    dispatcher: &mut Dispatcher,
    channel: Channel,
    authorization: Option<&str>,
    line: &str,
) -> (RoundTripPayload, Outcome) {
    let outcome = dispatcher.handle_line(channel, authorization, line);
    let response = outcome.response.as_ref().map_or(0, response_wire_bytes);
    (
        RoundTripPayload {
            request: line.len(),
            response,
        },
        outcome,
    )
}

/// `[BASELINE-006]` 的**场景请求行**：在已打开的工程上生成 `bars` 小节段落。
///
/// 单位和形状：返回客户端原样发出去的那一行 JSON 文本（无换行）。
/// 它是"统计生成 16 小节段落的完整 MCP 工具往返载荷"这条规范措辞的机器可复跑形态 ——
/// 判据与将来的 CI 步骤共用同一个场景定义，因此不存在"文档里量的是一回事、
/// 脚本里量的是另一回事"。
#[must_use]
pub fn baseline_006_section_line(bars: u32) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": "baseline-006",
        "method": "tools/call",
        "params": {
            "name": "yeban_propose_section",
            "arguments": {
                "sectionName": "Chorus",
                "stylePreset": BASELINE_006_STYLE_PRESET,
                "bars": bars,
                "scale": "C minor",
            }
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limit_is_the_published_four_kilobytes() {
        assert_eq!(BASELINE_006_JSON_LIMIT_BYTES, 4096);
        assert_eq!(BASELINE_006_SECTION_BARS, 16);
    }

    #[test]
    fn total_saturates_instead_of_wrapping() {
        let reading = RoundTripPayload {
            request: usize::MAX,
            response: 1,
        };
        assert_eq!(reading.total(), usize::MAX);
        assert!(!reading.within_baseline_006());
    }

    #[test]
    fn the_boundary_is_inclusive() {
        let at_limit = RoundTripPayload {
            request: 4096,
            response: 0,
        };
        let over_limit = RoundTripPayload {
            request: 4097,
            response: 0,
        };
        assert!(at_limit.within_baseline_006());
        assert!(!over_limit.within_baseline_006());
    }

    #[test]
    fn the_scenario_line_pins_the_sixteen_bar_section_call() {
        let line = baseline_006_section_line(BASELINE_006_SECTION_BARS);
        let parsed: serde_json::Value = serde_json::from_str(&line).expect("场景行是 JSON");
        assert_eq!(parsed["method"], "tools/call");
        assert_eq!(parsed["params"]["name"], "yeban_propose_section");
        assert_eq!(parsed["params"]["arguments"]["bars"], 16);
        assert_eq!(
            parsed["params"]["arguments"]["stylePreset"],
            BASELINE_006_STYLE_PRESET
        );
        // 场景行**不**声明 `includeOps`：量的是缺省形状（结构化字段）。
        assert!(
            parsed["params"]["arguments"].get("includeOps").is_none(),
            "场景行不得隐式打开完整 op 载荷"
        );
    }

    #[test]
    fn display_names_the_unit() {
        // 合成读数（不是实测值）：这条判据只钉 `Display` 的形状与单位后缀 `B`。
        let reading = RoundTripPayload {
            request: 100,
            response: 250,
        };
        assert_eq!(
            reading.to_string(),
            "request 100 B + response 250 B = 350 B"
        );
    }
}
