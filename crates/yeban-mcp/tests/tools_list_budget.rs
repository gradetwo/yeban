//! **`tools/list` 字节预算**（T-FIX-562）：MCP 工具清单的序列化 JSON 必须在上限之内。
//!
//! ## 这个文件量什么、单位是什么
//!
//! - **对象**：`tools/list` 的**响应体**，即 [`yeban_mcp::tools::catalog`] 序列化后的
//!   紧凑 JSON（`serde_json::to_string`，无空白）。分发层原样返回它
//!   （`dispatch.rs` 的 `tools/list` 分支），所以这里的字节数就是线上的字节数，
//!   只差一层定长的 JSON-RPC 信封；
//! - **单位**：UTF-8 **字节**；估算 token 用**字节 / 4**（与 `payload_budget.rs`
//!   同口径，**近似**，判据里明说）；
//! - **两条预算**：
//!   1. 清单**总字节** ≤ [`TOTAL_BYTES_MAX`]；
//!   2. 每条 `description`（工具级 + 参数级，含 `inputSchema` 里逐参数的说明与
//!      `op_schema` 的分支说明）≤ [`MAX_DESCRIPTION_CHARS`] 个**字符**
//!      （中文按字符计，⛔ 按字节计 —— 否则中文描述天然吃亏）。
//!
//! ## 为什么是这两条
//!
//! 工具与参数描述里长期塞着历史与 ADR 细节（`yeban_edit_notes` 一条摘要就
//! 6,639 字节）：每次 `tools/list` 都要把这些字原样发给 Agent，token 成本
//! 随工具数线性涨。总预算挡"清单整体又胖回去"，单条预算挡"某一条描述悄悄
//! 膨胀"—— 两条缺一不可：只钉总字节时，单条超长会被其它条的瘦身掩盖。
//!
//! ## 红对照（规则 30）
//!
//! 任一描述临时加长到超过 [`MAX_DESCRIPTION_CHARS`] 个字符 ⇒ 本判据变红；
//! 还原 ⇒ 变绿。复现：`cargo test -p yeban-mcp --test tools_list_budget -- --nocapture`。
//!
//! ## 描述该留什么（T-FIX-562 的瘦身口径）
//!
//! 只留用途、必填、关键约束、易错点；长度约束进 JSON Schema（`minLength` /
//! `maxLength` 已由注册表广播），其余约束留一句短子句。⛔ 删真实约束：
//! 下列逐字钉点由其它判据守护，瘦身时**必须原样保留**（删了就红）：
//! - `yeban_edit_notes` 摘要含"本调用永远写入隔离提案分支，必须 `yeban_merge_proposal` 才落到 main"
//!   （`trap_175.rs`）；`yeban_edit_automation` 摘要含"直接提交 / `McpEdit` / 不创建提案 /
//!   `data.applied`"（`extension_tools.rs`）；`yeban_list_proposals` 摘要含
//!   "未合并提案随工程容器落盘 / 会恢复它们"（`props_176.rs`）；
//! - `yeban_propose_section` 各参数说明含 ADR-00xx 出处与单位关键词
//!   （`section_*.rs` 一族），且 ⛔ 出现"随机"；
//! - `op_schema` 分支说明含"整片段 / 每个 / 展开 / 半音"与逐 kind 的"顶层 `clipId`"
//!   （`transpose_clip.rs` / `trap_177.rs`）。
//!
//! 复跑读数：`cargo test -p yeban-mcp --test tools_list_budget -- --nocapture`。

use serde_json::Value;

use yeban_mcp::tools;

/// `tools/list` 响应体总字节上限（T-FIX-562 瘦身后实测 301,513 字节，
/// 上限取 330,000 ≈ +9% 裕量，约够新增两个中等工具；读数见回执 §2/§3）。
///
/// ⚠ 改这个数之前先读回执：它是瘦身成果的**钉子**，不是随便调的旋钮。
/// 新增工具/参数要先瘦身再调预算，⛔ 反过来。
const TOTAL_BYTES_MAX: usize = 330_000;

/// 单条 `description` 的字符上限：1K 字符整（瘦身后最长一条 952 字符 =
/// `yeban_render_master` 的输出描述，字段名本身就长；其余全部 ≤ 717）。
///
/// 计**字符**（`chars().count()`），⛔ 计字节。
const MAX_DESCRIPTION_CHARS: usize = 1024;

/// 工具清单里一条 `description` 的出处（断言失败时点名）。
struct DescriptionRef {
    tool: String,
    param: Option<String>,
    chars: usize,
}

/// 递归收集 `value` 里全部 `description` 字符串（工具级、参数级、op 分支级）。
fn collect_descriptions(
    value: &Value,
    tool: &str,
    param: &Option<String>,
    out: &mut Vec<DescriptionRef>,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if key == "description" {
                    if let Some(text) = child.as_str() {
                        out.push(DescriptionRef {
                            tool: tool.to_owned(),
                            param: param.clone(),
                            chars: text.chars().count(),
                        });
                    }
                } else if key == "properties" {
                    if let Value::Object(props) = child {
                        for (name, prop) in props {
                            collect_descriptions(prop, tool, &Some(name.clone()), out);
                        }
                    }
                } else {
                    collect_descriptions(child, tool, param, out);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_descriptions(item, tool, param, out);
            }
        }
        _ => {}
    }
}

/// 各段的序列化字节：描述文本、inputSchema、outputSchema、annotations。
struct Breakdown {
    description_bytes: usize,
    input_schema_bytes: usize,
    output_schema_bytes: usize,
    annotations_bytes: usize,
}

fn breakdown(descriptor: &Value) -> Breakdown {
    let mut out = Breakdown {
        description_bytes: 0,
        input_schema_bytes: 0,
        output_schema_bytes: 0,
        annotations_bytes: 0,
    };
    // 直接按顶层键分段（inputSchema 内的 description 字节归入 inputSchema 段）。
    if let Some(text) = descriptor.get("description").and_then(Value::as_str) {
        out.description_bytes += text.len();
    }
    if let Some(schema) = descriptor.get("inputSchema") {
        out.input_schema_bytes += serde_json::to_string(schema).map(|s| s.len()).unwrap_or(0);
    }
    if let Some(schema) = descriptor.get("outputSchema") {
        out.output_schema_bytes += serde_json::to_string(schema).map(|s| s.len()).unwrap_or(0);
    }
    if let Some(annotations) = descriptor.get("annotations") {
        out.annotations_bytes += serde_json::to_string(annotations)
            .map(|s| s.len())
            .unwrap_or(0);
    }
    out
}

#[test]
fn tools_list_stays_within_byte_and_description_budgets() {
    let catalog = tools::catalog();
    let json = serde_json::to_string(&catalog).expect("清单必须可序列化");
    let total_bytes = json.len();
    let total_tokens_approx = total_bytes / 4;

    let descriptors = catalog["tools"].as_array().expect("tools 必须是数组");
    assert_eq!(
        descriptors.len(),
        tools::TOOL_COUNT,
        "清单工具数必须与注册表一致"
    );

    // --- 分段读数（eprintln：--nocapture 下可见，回执的 before/after 即取自这里）---
    let mut descriptions = Vec::new();
    let mut sum_desc = Breakdown {
        description_bytes: 0,
        input_schema_bytes: 0,
        output_schema_bytes: 0,
        annotations_bytes: 0,
    };
    let mut longest: Vec<(usize, String, Option<String>)> = Vec::new();
    for descriptor in descriptors {
        let name = descriptor["name"].as_str().expect("工具名");
        let seg = breakdown(descriptor);
        sum_desc.description_bytes += seg.description_bytes;
        sum_desc.input_schema_bytes += seg.input_schema_bytes;
        sum_desc.output_schema_bytes += seg.output_schema_bytes;
        sum_desc.annotations_bytes += seg.annotations_bytes;
        collect_descriptions(descriptor, name, &None, &mut descriptions);
    }
    for d in &descriptions {
        longest.push((d.chars, d.tool.clone(), d.param.clone()));
    }
    longest.sort_by_key(|e| std::cmp::Reverse(e.0));
    eprintln!("### tools/list 字节读数");
    eprintln!(
        "total_bytes={total_bytes} | tokens~{total_tokens_approx} | tools={}",
        descriptors.len()
    );
    eprintln!(
        "segments: tool_description={} inputSchema={} outputSchema={} annotations={}",
        sum_desc.description_bytes,
        sum_desc.input_schema_bytes,
        sum_desc.output_schema_bytes,
        sum_desc.annotations_bytes
    );
    eprintln!("descriptions={} | longest 5:", descriptions.len());
    for (chars, tool, param) in longest.iter().take(5) {
        eprintln!("  {chars} chars | {tool} | param={param:?}");
    }

    // --- 预算 ①：总字节 ---
    assert!(
        total_bytes <= TOTAL_BYTES_MAX,
        "tools/list 总字节 {total_bytes} 超过预算 {TOTAL_BYTES_MAX}（~{total_tokens_approx} tokens）：先瘦身描述再调预算，⛔ 反过来"
    );

    // --- 预算 ②：单条描述字符数 ---
    let mut overlong: Vec<&DescriptionRef> = descriptions
        .iter()
        .filter(|d| d.chars > MAX_DESCRIPTION_CHARS)
        .collect();
    overlong.sort_by_key(|d| std::cmp::Reverse(d.chars));
    assert!(
        overlong.is_empty(),
        "有 {} 条 description 超过 {} 字符（最长 {} 字符，{} {:?}）：把历史/ADR 细节移出描述，约束进 schema 或留一句短子句",
        overlong.len(),
        MAX_DESCRIPTION_CHARS,
        overlong.first().map(|d| d.chars).unwrap_or(0),
        overlong.first().map(|d| d.tool.as_str()).unwrap_or(""),
        overlong.first().and_then(|d| d.param.as_deref()),
    );
}
