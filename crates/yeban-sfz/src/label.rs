//! SFZ / ARIA 的 **标签（label）族**：把一个可读名字挂到 keyswitch、MIDI CC 与作用域上。
//!
//! 规范出处（全部是 ARIA 扩展，Type = string，Default = N/A，Range 列为空）：
//!
//! - `sw_label`：<https://sfzformat.com/opcodes/sw_label/> —— "Label for activated
//!   keyswitch on GUI."；同页 "Practical Considerations" 写明它的用途：
//!   "`sw_label` causes ARIA/Sforzando to display the most recent selected keyswitch
//!   label appear on its interface."。
//! - `label_ccN`：<https://sfzformat.com/opcodes/label_ccN/> —— "Creates a label for
//!   the MIDI CC."。
//! - `group_label`：<https://sfzformat.com/opcodes/group_label/> —— "An ARIA extension
//!   which sets what is displayed in the default info tab of Sforzando."；同页
//!   "It can be set anywhere, not just under the `<group>` header."。
//! - `master_label` / `global_label` / `region_label`：opcode 目录
//!   <https://sfzformat.com/opcodes/> 把它们与 `group_label` 逐字并列登记，
//!   四者都是同一条 "An ARIA extension which sets what is displayed in the default
//!   info tab of Sforzando."。
//!
//! ## 本模块负责什么
//!
//! 只有两件事：**识别 `label_ccN` 的名字**（[`parse_cc_label_name`]），以及承载归约结果的
//! [`Labels`]。作用域读取发生在 [`crate::instrument`]（`region → group → master → global`
//! 四级链），与其它 opcode 共用同一条路径。
//!
//! ## 契约
//!
//! - **零拷贝**：[`Labels`] 的每个字段都是 [`Cow`]，无 `$VAR` 宏替换时是
//!   [`Cow::Borrowed`]，只有发生文本替换的那一行降级为 [`Cow::Owned`]
//!   （与 [`crate::Region::sample`] 同一条口径）。
//! - **实时安全**：读取全是字段 / `BTreeMap` 查找，无分配、无锁、无 I/O、无日志，
//!   可在实时路径调用（构造期的 `BTreeMap` 分配只在解析时发生）。
//! - **逐位可复现**：只有字符串与整数，不含任何浮点运算，没有 `4096 ulp` 预算问题。
//! - **不解释、不发明**：标签**不改变**任何 region 选择、门控或渲染结果。
//!   本 crate 不发明「标签应该长什么样」的规则，也不翻译标签文本。
//!
//! ## 工程裁决（登记，不静默降级）
//!
//! 1. **`label_ccN` 的 `N` 上界是 [`MAX_CC_LABEL_INDEX`]**（`u16` 的容器界，65535）。
//!    规范表该行 Range 为空，正文只说 "for the MIDI CC"；登记的 1398 个 `.sfz` 里
//!    `label_ccN` 的最大下标是 401（`assets/samples/karoryfer-big-rusty-drums/`）。
//!    ARIA 的扩展 CC 编号到 155 为止（<https://sfzformat.com/extensions/midi_ccs/>：
//!    "Anything above 137 is not specified in the SFZ 2 standard and strictly
//!    engine-dependent"）。取 `u16` 只受容器限制；**越界是明确 `Err`，不静默丢弃**。
//! 2. **`Labels::scope_label` 的优先序是 `region → group → master → global`**，
//!    即与 opcode 继承链同向（越具体的段头越优先）。规范没有定义「同时给出多个作用域
//!    标签时显示哪一个」，这里选用与其它 opcode 完全一致的一条链。

use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::error::SfzError;

/// `label_ccN` 里 `N` 的上界（[`u16`] 的容器界）。
///
/// 出处与理由见模块文档的「工程裁决」第 1 条：规范 Range 为空，登记语料的最大下标是
/// 401，本常量只是容器限制。越过它的 `label_ccN` 是明确
/// [`crate::SfzError::IntegerOutOfRange`]，不静默丢弃。
pub const MAX_CC_LABEL_INDEX: u16 = u16::MAX;

/// 一个 `<region>` 归约出的标签集合（`*_label` 头族）。
///
/// 缺省值全部是 [`None`] / 空表：规范表里这一族的 Default 列就是 `N/A`，
/// 「没说」与「显式给空串」是两件事 —— 前者是 [`None`]，后者是 `Some("")`。
///
/// 字段全部 `pub`，因此调用方既能读 [`Cow`] 的形态（借用 / 拥有），
/// 也能用 [`Labels::scope_label`] / [`Labels::cc_label`] 这类读取助手。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Labels<'a> {
    /// `region_label`：`<region>` 段自己的标签。
    pub region_label: Option<Cow<'a, str>>,
    /// `group_label`：`<group>` 段的标签（规范允许写在任何作用域，见模块文档）。
    pub group_label: Option<Cow<'a, str>>,
    /// `master_label`：`<master>` 段的标签。
    pub master_label: Option<Cow<'a, str>>,
    /// `global_label`：`<global>` 段的标签。
    pub global_label: Option<Cow<'a, str>>,
    /// `sw_label`：本 region 要求的 keyswitch（[`crate::Region::sw_last`]）的名字。
    ///
    /// 规范把它定义成「GUI 上显示的、当前选中的 keyswitch 名字」
    /// （<https://sfzformat.com/opcodes/sw_label/>）。本 crate 不猜「当前选中」：
    /// 该判定见 [`crate::Instrument::keyswitch_label`]。
    pub keyswitch_label: Option<Cow<'a, str>>,
    /// `label_ccN`：按 CC 下标升序的标签表（确定性；同一 CC 重复给出时后者覆盖前者）。
    ///
    /// 只含 `region → group → master → global` 四级链里的值。写在 `<control>` 段里的
    /// `label_ccN` 是**文件级**声明，见 [`crate::Instrument::cc_labels`]。
    pub cc_labels: BTreeMap<u16, Cow<'a, str>>,
}

impl<'a> Labels<'a> {
    /// 生效的**作用域**标签：`region_label` 优先，其次 `group_label`、`master_label`、
    /// `global_label`（工程裁决第 2 条，见模块文档）。
    ///
    /// 返回 [`None`] 表示四个作用域标签都没给出。
    #[must_use]
    pub fn scope_label(&self) -> Option<&str> {
        [
            self.region_label.as_deref(),
            self.group_label.as_deref(),
            self.master_label.as_deref(),
            self.global_label.as_deref(),
        ]
        .into_iter()
        .flatten()
        .next()
    }

    /// 取某个 CC 下标的标签（[`None`] 表示该 CC 没有标签）。
    ///
    /// `BTreeMap` 查找，零分配。
    #[must_use]
    pub fn cc_label(&self, cc: u16) -> Option<&str> {
        self.cc_labels.get(&cc).map(Cow::as_ref)
    }

    /// CC 标签条数。
    #[must_use]
    pub fn cc_label_count(&self) -> usize {
        self.cc_labels.len()
    }

    /// 是否一个标签都没有（五个单值字段都是 [`None`]、CC 表为空）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.region_label.is_none()
            && self.group_label.is_none()
            && self.master_label.is_none()
            && self.global_label.is_none()
            && self.keyswitch_label.is_none()
            && self.cc_labels.is_empty()
    }
}

/// 识别 `label_ccN`，返回 `N` 的**原始数字串**。
///
/// 名字大小写敏感（与 [`crate::instrument`] 里其它 opcode 的读取口径一致，只有段头名
/// 大小写不敏感）：`LABEL_CC7` 不是本族的 opcode。
///
/// 返回数字串而不是解析结果，是为了让调用方区分两种失败：
/// 「名字不像 `label_ccN`」（返回 [`None`]，与未知 opcode 同口径地忽略）与
/// 「名字像但下标越界」（调用方给出明确 `Err`）。
#[must_use]
pub fn parse_cc_label_name(name: &str) -> Option<&str> {
    let digits = name.strip_prefix("label_cc")?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(digits)
}

/// 把 opcode 名解析成 `label_ccN` 的 CC 下标。
///
/// 三种结果，刻意不合并（调用方需要区别对待）：
///
/// - `Ok(None)`：名字不是 `label_ccN` ⇒ 与其它未知 opcode 同口径地忽略；
/// - `Ok(Some(index))`：命中，`index <= MAX_CC_LABEL_INDEX`；
/// - `Err(`[`crate::SfzError::IntegerOutOfRange`]`)`：名字是 `label_ccN` 但下标越界
///   ⇒ **明确 `Err`**，不静默丢弃、不静默截断（见模块文档「工程裁决」第 1 条）。
pub(crate) fn cc_label_index(name: &str, line: usize) -> Result<Option<u16>, SfzError> {
    let Some(digits) = parse_cc_label_name(name) else {
        return Ok(None);
    };
    match digits.parse::<u16>() {
        Ok(index) => Ok(Some(index)),
        // `digits` 已保证是全 ASCII 数字，因此这里只有「超过 u16」这一种失败。
        // 错误载荷按 i64 饱和取值（更长的数字串仍能报出行号与 opcode 名）。
        Err(_) => Err(SfzError::IntegerOutOfRange {
            line,
            opcode: name.to_string(),
            value: digits.parse::<i64>().unwrap_or(i64::MAX),
            min: 0,
            max: i64::from(MAX_CC_LABEL_INDEX),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cc_label_name_needs_a_nonempty_all_digit_suffix() {
        assert_eq!(parse_cc_label_name("label_cc0"), Some("0"));
        assert_eq!(parse_cc_label_name("label_cc7"), Some("7"));
        assert_eq!(parse_cc_label_name("label_cc127"), Some("127"));
        // 登记语料里的扩展下标（`assets/samples/karoryfer-big-rusty-drums/`）。
        assert_eq!(parse_cc_label_name("label_cc400"), Some("400"));
        assert_eq!(parse_cc_label_name("label_cc401"), Some("401"));
        // 前导零原样带出：解析由调用方按十进制做，`007` 与 `7` 是同一个下标。
        assert_eq!(parse_cc_label_name("label_cc007"), Some("007"));

        for bad in [
            "label_cc",
            "label_c",
            "label_ccX",
            "label_cc7x",
            "label_cc-1",
            "label_cc7 ",
            " label_cc7",
            "LABEL_CC7",
            "Label_cc7",
            "label_cc 7",
            "label_ccx7",
            "xlabel_cc7",
            "label_cc₁",
        ] {
            assert_eq!(parse_cc_label_name(bad), None, "{bad:?} must not match");
        }
    }

    #[test]
    fn scope_label_follows_the_opcode_inheritance_direction() {
        let mut labels = Labels::default();
        assert_eq!(labels.scope_label(), None);

        labels.global_label = Some(Cow::Borrowed("g"));
        assert_eq!(labels.scope_label(), Some("g"));
        labels.master_label = Some(Cow::Borrowed("m"));
        assert_eq!(labels.scope_label(), Some("m"));
        labels.group_label = Some(Cow::Borrowed("grp"));
        assert_eq!(labels.scope_label(), Some("grp"));
        labels.region_label = Some(Cow::Borrowed("r"));
        assert_eq!(labels.scope_label(), Some("r"));
    }

    #[test]
    fn scope_label_skips_a_present_but_empty_most_specific_label() {
        // 「显式给空串」不是「没说」：空串是**存在**的取值，因此它胜出。
        let labels = Labels {
            region_label: Some(Cow::Borrowed("")),
            group_label: Some(Cow::Borrowed("grp")),
            ..Labels::default()
        };
        assert_eq!(labels.scope_label(), Some(""));
    }

    #[test]
    fn cc_label_is_an_exact_index_lookup() {
        let mut labels = Labels::default();
        assert!(labels.is_empty());
        assert_eq!(labels.cc_label(0), None);
        assert_eq!(labels.cc_label_count(), 0);

        labels.cc_labels.insert(7, Cow::Borrowed("Volume"));
        labels
            .cc_labels
            .insert(400, Cow::Borrowed("Limiter thresh"));
        assert_eq!(labels.cc_label(7), Some("Volume"));
        assert_eq!(labels.cc_label(400), Some("Limiter thresh"));
        assert_eq!(labels.cc_label(8), None);
        assert_eq!(labels.cc_label(40), None);
        assert_eq!(labels.cc_label_count(), 2);
        assert!(!labels.is_empty());
    }

    #[test]
    fn max_cc_label_index_matches_the_u16_container() {
        assert_eq!(MAX_CC_LABEL_INDEX, 65535);
        // 上界本身可解析；再大一位就落到调用方的越界分支。
        assert_eq!(parse_cc_label_name("label_cc65535"), Some("65535"));
        assert!("65535".parse::<u16>().is_ok());
        assert!("65536".parse::<u16>().is_err());
    }
}
