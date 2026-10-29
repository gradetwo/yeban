//! **常驻判据**：实时插入链（`src/insert.rs`）认的参数名，必须与
//! [`yeban_dsp::device_param`] 的三张 `SPELLINGS` **逐字同集**。
//!
//! ## 为什么要有它
//!
//! 设备参数落点表下移到 `yeban-dsp` 之后，仓库里仍有**两张**表：
//!
//! * `yeban_dsp::device_param::{StripField, ReverbField, ConvolutionField}::SPELLINGS`
//!   —— 离线渲染链（`yeban-render`）与工具面（`yeban-mcp`）读的那张；
//! * `src/insert.rs` 三个投影结构各自的 `fn apply` 的 **`match` 字面臂** —— 实时链读的那张。
//!
//! ⛔ 第二张暂时不能并进第一张：实时侧的投影**不是**直接写 `ChannelStripParams`，它还要记
//! "这一级有没有出现过"（`eq.seen` / `filter_seen` / `compressor_seen`）与**逐旋钮的部分覆盖**
//! （见 `src/insert.rs` 模块文档 §4.1／§8.2）—— 那是共享表的 `store` 表达不了的中间态。
//! ⇒ 本判据把"两张表**不许漂移**"这件事**机械化**：任何人只在其中一侧加/删一个拼写，
//! 另一侧不动 ⇒ 本判据立刻变红。（⛔ 一次性的人工对账不算判据。）
//!
//! ## 口径（数字在说什么）
//!
//! * **被解析对象**：`src/insert.rs` 的**源码文本**（`include_str!`，编译期固定；
//!   ⛔ 不用 `read_dir`／运行期读文件，夹具缺失会变真空）。
//! * **一个"拼写"**＝ `fn apply` 的 `match` 里，一行**以 `"` 起始**且含 `=>` 的臂模式里
//!   `=>` **之前**出现的每个双引号字符串。臂体（`=>` 之后）里的字符串字面量**不**算拼写。
//! * **归属**按 `impl` 块名判定（`impl StripProjection`／`impl ReverbProjection`／
//!   `impl ConvProjection`），⛔ 不按"集合恰好等于哪张表"反推 —— 后者会让断言自我确认。
//! * **唯一允许的已知差集**：实时侧额外认 **3** 个 IR 形状键（[`IR_SHAPE_NAMES`]）。
//!   它们是**构造期**烘焙参数、⛔ 不是自动化落点，故刻意不在 dsp 的 `ConvolutionField::SPELLINGS`
//!   里（口径同 `yeban_render::inserts::is_supported_param`）。
//!
//! ## 非真空（R93／R100）
//!
//! 每族解析出的拼写数**必须**等于冻结读数（41／6／11），且三族解析各**必须**找到
//! 唯一一个 `fn apply` ⇒ 解析器退化（读不到臂）只会变红，不会变"空集即通过"。

use std::collections::BTreeSet;

use yeban_dsp::device_param::{ConvolutionField, ReverbField, StripField};

/// 被扫文件：**编译期固定**（⛔ 运行期读盘）。
const SOURCE: &str = include_str!("../src/insert.rs");

/// 实时侧额外认、但⛔ 不是自动化落点的 **IR 形状**键（构造期烘焙）。
const IR_SHAPE_NAMES: [&str; 3] = ["conv_ir_decay_s", "conv_ir_decay", "conv_ir_seed"];

/// 冻结读数（与 `yeban-render` 的 `automation.rs` 同一条口径：41 ＋ 6 ＋ 8 ＝ 55 个拼写；
/// 实时侧再＋ 3 个 IR 形状键 ⇒ 卷积族在**本判据**里是 11）。
const FROZEN_COUNTS: (usize, usize, usize) = (41, 6, 11);

/// 取 `impl <header>` 块内 `fn apply(&mut self, name: &str, value: f32)` 的**臂名**列表。
///
/// 找不到 `fn apply` ⇒ `panic!`（⛔ 不返回空集合：空集合会让下面的等集断言真空通过）。
fn apply_arm_names(impl_header: &str) -> Vec<String> {
    let block = impl_block(impl_header);
    let marker = "fn apply(&mut self, name: &str, value: f32) {";
    let starts: Vec<usize> = block.match_indices(marker).map(|(i, _)| i).collect();
    assert_eq!(
        starts.len(),
        1,
        "{impl_header} 里必须**恰好一个** `fn apply`（实读 {} 个）⇒ 签名变了要先改本判据的口径",
        starts.len()
    );
    let body = balanced(&block, starts[0] + marker.len() - 1);
    let mut names = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim_start();
        // 只认「以引号起始、且含 =>」的臂模式行；⛔ 不认臂体里的字符串字面量。
        if !trimmed.starts_with('"') {
            continue;
        }
        let Some((pattern, _)) = trimmed.split_once("=>") else {
            continue;
        };
        let mut rest = pattern;
        while let Some(open) = rest.find('"') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('"') else {
                break;
            };
            names.push(after[..close].to_string());
            rest = &after[close + 1..];
        }
    }
    assert!(
        !names.is_empty(),
        "{impl_header} 的 `fn apply` 没解析出任何臂名"
    );
    names
}

/// 取出 `impl <header>` 的整个块（含大括号配对）。
fn impl_block(header: &str) -> String {
    let Some(at) = SOURCE.find(header) else {
        panic!("src/insert.rs 里找不到 {header} ⇒ 本判据的归属口径已陈旧");
    };
    let open = at + header.len() - 1;
    balanced(&SOURCE[open..], 0)
}

/// 从 `src[start] == '{'` 起返回配对到闭合 `}` 的子串（含两端）。
///
/// ⚠ 只按**字符**数括号：源码里的 `'{'`／`'}'` 字符字面量与本仓库 `insert.rs` 的
/// 字符串字面量都不含裸括号（实测：三处 `fn apply` 的臂体里没有括号字符），故不处理
/// 转义。若将来出现，本判据会因括号计数失衡而**响亮失败**，不会静默给出错位文本。
fn balanced(src: &str, start: usize) -> String {
    assert!(
        src[start..].starts_with('{'),
        "期望从 `{{` 起算，实读 {:?}",
        &src[start..(start + 8).min(src.len())]
    );
    let mut depth = 0usize;
    for (i, ch) in src[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return src[start..=start + i].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("大括号未配对 ⇒ 解析器退化（⛔ 不当作空集通过）");
}

fn names_to_set(names: &[String]) -> BTreeSet<String> {
    names.iter().cloned().collect()
}

fn dsp_names<T>(spellings: &[(&'static str, T)]) -> BTreeSet<String> {
    spellings
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect()
}

/// **通道条 41 拼写**：实时臂集 == [`StripField::SPELLINGS`]，且非真空。
#[test]
fn the_realtime_strip_arms_are_the_dsp_strip_spellings() {
    let arms = apply_arm_names("impl StripProjection {");
    let expected = dsp_names(StripField::SPELLINGS);
    let got = names_to_set(&arms);
    assert_eq!(arms.len(), got.len(), "实时通道条臂里有重复拼写：{arms:?}");
    assert_eq!(
        got.len(),
        FROZEN_COUNTS.0,
        "实时通道条解析出 {} 个拼写，冻结读数是 {} ⇒ 先确认是不是有人单侧改了表",
        got.len(),
        FROZEN_COUNTS.0
    );
    assert_eq!(got, expected, "实时链与 dsp 的通道条拼写表不一致");
}

/// **Freeverb 6 拼写**：实时臂集 == [`ReverbField::SPELLINGS`]。
#[test]
fn the_realtime_reverb_arms_are_the_dsp_reverb_spellings() {
    let arms = apply_arm_names("impl ReverbProjection {");
    let expected = dsp_names(ReverbField::SPELLINGS);
    let got = names_to_set(&arms);
    assert_eq!(arms.len(), got.len(), "实时混响臂里有重复拼写：{arms:?}");
    assert_eq!(
        got.len(),
        FROZEN_COUNTS.1,
        "实时混响解析出 {} 个拼写，冻结读数是 {}",
        got.len(),
        FROZEN_COUNTS.1
    );
    assert_eq!(got, expected, "实时链与 dsp 的混响拼写表不一致");
}

/// **卷积 8 ＋ IR 形状 3**：实时臂集 == [`ConvolutionField::SPELLINGS`] ∪ [`IR_SHAPE_NAMES`]，
/// 且这三键确实**不在** dsp 表里（否则本条登记的差集就是空话）。
#[test]
fn the_realtime_convolution_arms_add_only_the_registered_ir_shapes() {
    let arms = apply_arm_names("impl ConvProjection {");
    let mut expected = dsp_names(ConvolutionField::SPELLINGS);
    let conv_spellings = expected.clone();
    for name in IR_SHAPE_NAMES {
        assert!(
            !conv_spellings.contains(name),
            "{name} 已经进了 dsp 的 `ConvolutionField::SPELLINGS` ⇒ 本判据登记的差集陈旧了"
        );
        expected.insert(name.to_string());
    }
    let got = names_to_set(&arms);
    assert_eq!(arms.len(), got.len(), "实时卷积臂里有重复拼写：{arms:?}");
    assert_eq!(
        got.len(),
        FROZEN_COUNTS.2,
        "实时卷积解析出 {} 个拼写（8 可自动化 ＋ 3 IR 形状 = 11）",
        got.len()
    );
    assert_eq!(
        got, expected,
        "实时链与 dsp 的卷积拼写表不一致（唯一允许的差集是 {IR_SHAPE_NAMES:?}）"
    );
}
