//! 文档化的**可选实参**必须至少被一条判据给过**非缺省值**（`ADR-0028` 的教训，规则 32 的机械化）。
//!
//! 由来（2026-10-28，`drumDensity`）：`ADR-0028` 落地后很长一段时间里，
//! `crates/yeban-mcp/tests/*.rs` 的 **14** 处 `drum_density` **全部**是 `None`（管线占位）——
//! 也就是说 `section_build.rs` 里那个稀疏筛点**没有任何判据看着**，而**看目录**完全看不出来：
//! 同族的 `swingPermille` / `drumStyle` / `drumBackbeat` / `beatGrouping` / `melodySeed`
//! 各有自己的判据文件（缺口补在 `tests/section_drum_density.rs`）。
//!
//! 单位 = **文档化可选键**（取自 `schemas/mcp-tools.schema.json` 的
//! `definitions.DocumentedToolArguments.$defs.<tool>.properties`，⛔ 本文件另抄一份键名表）。
//!
//! ## 判据力（先说清楚它**证明**什么、⛔ 证明什么）
//!
//! - 证明：每个登记的可选键都在测试里出现过一次**非缺省取值**（`k: <非 None>` 或 `"k": <非 null>`）。
//! - ⛔ 证明那个值真的到了消费点：`k: given.k` 这类建造器管线也只是"某个非 `None` 的值从这里过"。
//!   ⇒ 本判据挡的是"**整键零覆盖**"（14/14 全是 `None` 那一类），⛔ 挡"值语义写错"。
//! - ⛔ 数 `__` 字面量 `Some(`：同目录判据大多经建造器给值，构造点写的是 `k: given.k` ⇒
//!   `Some(` 在有覆盖的键上同样是 0（实测：本判据初版就是这么错的，见台账 16:03 的处方修正）。
//!
//! ## 敏感性（规则 30：先把真缺陷的预映像喂给它）
//!
//! 预映像（`section_drum_density.rs` 落地**之前**的测试树）上，本判据的等价算法在 21 个键里
//! **恰好红 1 个**：`drumDensity`（0 处非缺省）；其余 20 个键各 ≥ 1 ⇒ ⛔ 误伤面。
//! 现读最小读数 = **1**（`drumDensity` / `drumStyle` / `drumBackbeat` / `beatGrouping` 四个键）。
//!
//! ## 扫描面（⛔ 本判据**不**覆盖扩展工具；且扩展侧的词面读数会**假绿**）
//!
//! 只扫 `DocumentedToolArguments`（3 个工具 / 21 个键）。`ExtensionToolArguments.$defs`
//! （15 个工具 / 81 个属性）未纳入，两条**实测**理由：
//!
//! 1. `src` 单元测试也是判据（`yeban_query_engine_state.since` 的断言在
//!    `src/domain/engine_state.rs:649`）⇒ 只读 tests 会假红；
//! 2. 换成 tests ＋ src 全扫更糟：**跨工具同名键碰撞**把别的工具命中算给本工具
//!    （`yeban_render_sfz_wav.sampleRate` 的 199 处命中全在 `yeban_render_master`
//!    一族的调用里），**非法值**（拒绝用例）也算命中（`toneHz` 唯一一处是
//!    `yeban_record_wav` 的 `-5`）⇒ 2026-10-28 的手工全扫给出"零覆盖 0"的**假绿**。
//!    真相是：`yeban_render_sfz_wav.velocity` 那时**根本没到达渲染器**（解析＋回显的
//!    空操作，R142），`toneHz` 从未被给过有效值，`sampleHz` 只有输出被断言。
//!
//! 三处缺口现由 `tests/sfz_tools.rs` 的语义判据兜住
//! （`the_sample_hz_argument_is_honoured_and_bad_values_are_rejected` /
//! `the_render_velocity_override_selects_regions_and_scales_the_peak` /
//! `the_render_sample_rate_and_tone_are_honoured`）。
//!
//! ⚠ 结论：扩展参数轴**不能靠词面扫描**（规则 32 的假阳性一侧）。要机械化必须按
//! "**哪个调用的**实参"归类（解析调用点），本文件不做 —— 这项 rot 风险如实登记。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// 文档化可选键的登记处（`ToolCall` 的特有实参走注册表，这一节登记**新增**键的形状）。
const SCHEMA: &str = "schemas/mcp-tools.schema.json";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/yeban-mcp 之上应当有仓库根")
        .to_path_buf()
}

/// `materialSource` → `material_source`（`PlanOptions` 的字段名形态）。
fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (index, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// 本键在**给定文本**里被给过非缺省值的次数。
///
/// 两种形态各数一遍：Rust 字段（`drum_density: Some("sparse")` / `drum_density: density`）
/// 与 JSON 实参（`"drumDensity": "sparse"`）。`None` / `null` ⛔ 算数（那正是缺口的样子）。
fn non_default_hits(text: &str, camel: &str, snake: &str) -> usize {
    let mut hits = 0usize;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        // ① Rust 字段形态：`<snake>:` 之后的值⛔ 是 `None`。
        if let Some(at) = line.find(&format!("{snake}:")) {
            let rest = line[at + snake.len() + 1..]
                .trim()
                .trim_end_matches(',')
                .trim();
            if !rest.is_empty() && rest != "None" {
                hits += 1;
            }
        }
        // ② JSON 实参形态：`"<camel>":` 之后的值⛔ 是 `null`。
        if let Some(at) = line.find(&format!("\"{camel}\":")) {
            let rest = line[at + camel.len() + 3..]
                .trim()
                .trim_end_matches(',')
                .trim();
            if !rest.is_empty() && rest != "null" {
                hits += 1;
            }
        }
    }
    hits
}

#[test]
fn every_documented_optional_argument_is_given_a_non_default_value_somewhere() {
    let root = repo_root();
    let schema: Value =
        serde_json::from_str(&fs::read_to_string(root.join(SCHEMA)).expect("schema 必须可读"))
            .expect("schema 必须是合法 JSON");
    let tools = schema["definitions"]["DocumentedToolArguments"]["$defs"]
        .as_object()
        .expect("`DocumentedToolArguments.$defs` 必须是对象");

    let mut keys: Vec<(String, String, String)> = Vec::new();
    for (tool, spec) in tools {
        let props = spec["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("`{tool}` 必须登记 properties"));
        for (camel, _) in props {
            keys.push((tool.clone(), camel.clone(), snake_case(camel)));
        }
    }
    // R93：被扫集合是**动态**的（schema 派生）⇒ 必须有下界，否则 schema 清空后本判据恒绿。
    assert!(
        !tools.is_empty(),
        "`DocumentedToolArguments.$defs` 为空 —— 扫描面异常"
    );
    assert!(
        keys.len() >= 20,
        "文档化可选键只剩 {} 个 —— 扫描面异常（现读 21）",
        keys.len()
    );

    let tests_dir = root.join("crates/yeban-mcp/tests");
    let mut sources: Vec<(PathBuf, String)> = Vec::new();
    for entry in fs::read_dir(&tests_dir).expect("tests 目录必须可读") {
        let path = entry.expect("目录项必须可读").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        sources.push((
            path.clone(),
            fs::read_to_string(&path).expect("测试源必须可读"),
        ));
    }
    // R93 同上：文件集合也是动态的。
    assert!(
        sources.len() >= 20,
        "扫到的测试源只剩 {} 个 —— 扫描面异常",
        sources.len()
    );

    let mut uncovered: Vec<String> = Vec::new();
    let mut readings: Vec<(String, usize)> = Vec::new();
    for (tool, camel, snake) in &keys {
        let hits: usize = sources
            .iter()
            .map(|(_, text)| non_default_hits(text, camel, snake))
            .sum();
        readings.push((camel.clone(), hits));
        if hits == 0 {
            uncovered.push(format!("{tool}.{camel}"));
        }
    }

    assert!(
        uncovered.is_empty(),
        "这些文档化的可选键在 `crates/yeban-mcp/tests/*.rs` 里**一次非缺省值都没有**给过 \
         （＝整键零覆盖：`ADR-0028` 的 `drumDensity` 就是这样漏了整整一轮）: {:?}\n\
         逐个读数（键, 非缺省出现次数）: {:?}",
        uncovered,
        readings
    );
}
