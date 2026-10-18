//! MCP 往返**载荷统计** `[BASELINE-006]`：量什么、单位是什么、token 半边为什么是近似。
//!
//! ## 规范原文（`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:361`）
//!
//! | 规范 ID | 指标 | 测法 | 判据 |
//! | :--- | :--- | :--- | :--- |
//! | `[BASELINE-006]` | **AI 交互效率** | 统计生成 16 小节段落的**完整 MCP 工具往返载荷** | **序列化 JSON 载荷 ≤ 4 KB，结构化字段传输**，Token 开销中位数 ≤ 600 Tokens |
//!
//! 本模块量**序列化 JSON 的字节数**（可复跑的真读数），并给出**明确标注为近似**的
//! token 估算。
//!
//! ## Token 那一半：**近似**，而且**明说**是近似（`HD-48` 选项 B）
//!
//! "Token 开销中位数 ≤ 600" 需要一个**分词器口径**。本仓**没有任何分词器依赖**
//! （`grep -nE "tiktoken|tokenizer|bpe|sentencepiece" Cargo.toml crates/*/Cargo.toml`
//! ⇒ 无命中），引入一个会触及**红线 2（许可合规）**与 `MUST-GATE-005` 的 vendor 集合。
//! 人类裁决 **`HD-48` 选项 B**：**不新增任何依赖**，用**字节数 / 4** 估算 token 量级，
//! 并在结果里**明说这是近似**。
//!
//! 因此本模块给出的是 [`TokenEstimate`]（`tokens = bytes / 4`，整数除法、向下取整）
//! ＋ 口径说明 [`TOKEN_ESTIMATOR`]（逐字"按字节/4 估算、非真实分词"）。**数字与
//! 说明必须一起出现**：单独一个整数会被读成真实分词读数，那就把近似伪装成了判据。
//! `[BASELINE-006]` 的"≤ 600 Tokens"若要用**真值**判定，仍需另一单引入分词器口径
//! （`HD-48` 明确没做这件事）。
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
//! | `yeban_propose_section`（16 小节，`filled_project`） | 请求 | 响应 | 合计 | 对 4 KB | token ≈（字节/4，非真实分词） |
//! | :--- | ---: | ---: | ---: | :--- | ---: |
//! | 回传完整 op 载荷（`includeOps: true`） | 214 | 9,525 | **9,739** | **2.38 × 超限** | 2,434 |
//! | 只回结构化字段（缺省，`includeOps` 不写） | 196 | 2,816 | **3,012** | 0.74 × 限额 | 753 |
//! | `dryRun: true` 的预览（缺省形状） | 210 | 2,777 | 2,987 | 0.73 × 限额 | 746 |
//!
//! ⚠ 三行都比 #174 之前多 **69 字节**：#175 给提案响应加了 `pendingMerge` /
//! `mergeTool` / `warnings` 三个字段（"提案必须 merge 才落盘"的显式语义）。
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

/// `HD-48` 选项 B 的估算口径：**每个 token 按 4 个 UTF-8 字节**折算。
///
/// 这是**量级近似**，不是分词器的真实读数：本 crate **零**第三方分词器依赖。
pub const TOKEN_ESTIMATOR_BYTES_PER_TOKEN: usize = 4;

/// token 读数**旁边**逐字出现的口径说明（`HD-48` 选项 B）。
///
/// ⚠ 任何报 token 数的读数都必须**同时**给出这句话；单独一个整数会被读成真实分词。
pub const TOKEN_ESTIMATOR: &str = "按字节/4 估算、非真实分词";

/// 一个**估算**的 token 读数：数字与口径说明**成对**出现，不允许只带数字。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenEstimate {
    /// 估算值：`bytes / 4`（整数除法、向下取整）。
    pub tokens: usize,
    /// 口径说明（逐字），永远是 [`TOKEN_ESTIMATOR`]。
    pub estimator: &'static str,
}

impl TokenEstimate {
    /// 由**已知的字节数**折算：`bytes / 4`（整数除法、向下取整）。
    ///
    /// 取整关系逐字是 `tokens * 4 ≤ bytes < (tokens + 1) * 4`。
    #[must_use]
    pub const fn from_bytes(bytes: usize) -> Self {
        Self {
            tokens: bytes / TOKEN_ESTIMATOR_BYTES_PER_TOKEN,
            estimator: TOKEN_ESTIMATOR,
        }
    }
}

impl fmt::Display for TokenEstimate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "token ≈ {}（{}）", self.tokens, self.estimator)
    }
}

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

    /// 合计的 token 量级**估算**：`total() / 4`（整数除法、向下取整）。
    ///
    /// ⚠ 这不是真实分词：本仓没有分词器依赖（`HD-48` 选项 B）。返回值里带着
    /// [`TOKEN_ESTIMATOR`] 这句逐字说明，显示时也不会只剩一个数字。
    #[must_use]
    pub const fn estimated_tokens(self) -> TokenEstimate {
        TokenEstimate::from_bytes(self.total())
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
            "request {} B + response {} B = {} B · {}",
            self.request,
            self.response,
            self.total(),
            self.estimated_tokens()
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
    fn display_names_the_unit_and_the_estimator() {
        // 合成读数（不是实测值）：这条判据只钉 `Display` 的形状、单位后缀 `B`
        // 与 token 估算**必须带着口径说明**一起出现（`HD-48` 选项 B）。
        let reading = RoundTripPayload {
            request: 100,
            response: 250,
        };
        assert_eq!(
            reading.to_string(),
            "request 100 B + response 250 B = 350 B · token ≈ 87（按字节/4 估算、非真实分词）"
        );
    }

    #[test]
    fn the_token_estimate_is_bytes_divided_by_four() {
        // 口径逐字：tokens = bytes / 4（整数除法、向下取整），
        // 等价于 tokens * 4 ≤ bytes < (tokens + 1) * 4。
        for bytes in [0, 1, 3, 4, 5, 196, 3012, 4096] {
            let estimate = TokenEstimate::from_bytes(bytes);
            assert_eq!(estimate.tokens, bytes / 4, "bytes={bytes}");
            assert!(
                estimate.tokens * 4 <= bytes,
                "token * 4 不得大于字节数: bytes={bytes}, tokens={}",
                estimate.tokens
            );
            assert!(
                bytes - estimate.tokens * 4 < 4,
                "向下取整的余数必须 < 4: bytes={bytes}, tokens={}",
                estimate.tokens
            );
        }
    }

    #[test]
    fn every_token_estimate_carries_the_verbatim_caveat() {
        let estimate = RoundTripPayload {
            request: 196,
            response: 2816,
        }
        .estimated_tokens();
        assert_eq!(estimate.tokens, 753);
        assert_eq!(estimate.estimator, TOKEN_ESTIMATOR);
        assert_eq!(TOKEN_ESTIMATOR, "按字节/4 估算、非真实分词");
        assert_eq!(
            estimate.to_string(),
            "token ≈ 753（按字节/4 估算、非真实分词）"
        );
    }
}
