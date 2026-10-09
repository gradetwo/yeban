//! SFZ `<midi>` 头：ARIA 的 MIDI 预处理器声明（**定义段**）。
//!
//! 规范出处 <https://sfzformat.com/headers/midi/>。本 crate 能引用的该页原文只有
//! [`crate::EffectBus::Midi`] 已记下的这一句（出处同页）：
//!
//! > From ARIA v1.0.8.0+ an `<effect>` section with a `bus=midi` can be used instead.
//!
//! 也就是说：`<midi>` 段与 `<effect>bus=midi</effect>` 在 ARIA 上是**同一件事的两种写法**
//! （机械判据见 [`crate::Instrument::midi_preprocessor_declared`]）。
//!
//! ## 为什么本模块只登记 opcode、不解释它
//!
//! `<midi>` 段内的 opcode 词汇跨播放器不一致。规范对 `<effect>` 段有同一句告诫
//! （见 [`crate::effect`] 的模块文档原文）："The specifics of what can be used under this
//! header differ widely between SFZ players."。本切片执行时**本机网络中断**，无法取回
//! `<midi>` 页的 opcode 表，因此按 [`crate::Curve`] 内建曲线 4..=6 的同一条口径处理：
//! **不发明** opcode 名字、缺省值与范围。段内每个 `opcode=value` 原样登记成
//! [`MidiOpcode`]（名字、取值、行号），留给能引用规范表的后续切片消费。
//!
//! ## 契约
//!
//! - **定义段**：`<midi>` 的 opcode 只进 [`MidiSection`]，**绝不**写进
//!   `region → group → master → global` 继承链，也**不清空**任何继承作用域
//!   （与 [`crate::Curve`] / [`crate::Effect`] 同一条口径）。
//! - **空段也登记**：与 `<curve>` / `<effect>` 不同 —— 那两者的数据都挂在 opcode 上，
//!   而 `<midi>` 段本身就是声明（规范原文把 `bus=midi` 的 `<effect>` 说成它的替代写法），
//!   所以没有 opcode 的 `<midi>` 段仍然产生一个条目。这条差异是**有意**的，不是漏判。
//! - **`MidiOpcode::line` 是 opcode 自己的行号**，不是段头行：原样登记口径下这是可用信息。
//!   `<curve>` / `<effect>` 的错误定位口径不变（仍用段头行）。
//! - **实时安全**：读取全是字段 / 切片访问，无分配、无锁、无 I/O、无日志，
//!   可在实时路径调用。
//! - **逐位可复现**：只有整数与字符串，不含任何浮点运算，没有 `4096 ulp` 预算问题。
//!
//! ## 未建模的部分（登记，不静默降级）
//!
//! - opcode 的**语义**（哪个 CC、哪条曲线、作用到哪个参数）：本 crate 不解释（见上）。
//! - `<sample>` 段头仍未建模：识别到即 [`crate::Warning::IgnoredHeader`] 并丢弃其 opcode。

use std::borrow::Cow;

/// `<midi>` 段里的一个 opcode，**原样**登记（本 crate 不解释它的语义）。
///
/// `name` / `value` 与其它 opcode 同口径：取值已去引号、去首尾空白，两者都做过宏替换；
/// 没有宏替换时是 [`Cow::Borrowed`]（零拷贝）。
///
/// **大小写保留**：名字不做任何大小写归并（与继承链里其它 opcode 的读取口径一致，
/// 只有段头名是大小写不敏感的）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiOpcode<'a> {
    name: Cow<'a, str>,
    value: Cow<'a, str>,
    line: usize,
}

impl<'a> MidiOpcode<'a> {
    /// 登记一个 opcode（解析器内部使用）。
    pub(crate) fn new(name: Cow<'a, str>, value: Cow<'a, str>, line: usize) -> Self {
        Self { name, value, line }
    }

    /// opcode 名（原样，大小写保留；已做宏替换）。
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// opcode 取值（已去引号、去首尾空白；已做宏替换）。
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// 该 opcode 所在行的 1-based 行号（**不是**段头行）。
    #[must_use]
    pub fn line(&self) -> usize {
        self.line
    }
}

/// 一条 `<midi>` 段头定义的 MIDI 预处理器声明。
///
/// 只承载「这一段出现过」与段内 opcode 的**原文**；不含任何被解释过的语义
/// （见模块文档：opcode 词汇跨播放器不一致，且本切片无法核验规范表）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiSection<'a> {
    line: usize,
    opcodes: Vec<MidiOpcode<'a>>,
}

impl<'a> MidiSection<'a> {
    /// 用段头行号与已收集的 opcode 构造一条声明（解析器内部使用）。
    pub(crate) fn new(line: usize, opcodes: Vec<MidiOpcode<'a>>) -> Self {
        Self { line, opcodes }
    }

    /// `<midi>` 段头所在行的 1-based 行号。
    #[must_use]
    pub fn line(&self) -> usize {
        self.line
    }

    /// 段内 opcode，按文件出现顺序（确定性；重复名字**全部保留**，不做「后者胜」归并：
    /// 原样登记不丢数据）。
    #[must_use]
    pub fn opcodes(&self) -> &[MidiOpcode<'a>] {
        &self.opcodes
    }

    /// 段内 opcode 条数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.opcodes.len()
    }

    /// 这一段是否没有任何 opcode（**仍然是**一条已登记的声明，见模块文档）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.opcodes.is_empty()
    }

    /// 按名字取**第一个**匹配的取值（线性扫描，零分配、大小写敏感）。
    ///
    /// 返回 `None` 表示这一段没写这个名字。名字是原样登记的，所以调用方必须自己
    /// 用规范表里的确切拼写查询（本 crate 不猜别名、不做大小写归并）。
    #[must_use]
    pub fn opcode(&self, name: &str) -> Option<&str> {
        self.opcodes
            .iter()
            .find(|opcode| opcode.name() == name)
            .map(MidiOpcode::value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opcode<'a>(name: &'a str, value: &'a str, line: usize) -> MidiOpcode<'a> {
        MidiOpcode::new(Cow::Borrowed(name), Cow::Borrowed(value), line)
    }

    #[test]
    fn an_opcode_keeps_the_raw_name_case_and_its_own_line() {
        // 本类型原样承载调用方给的三元组：解析器传进来的取值已经去过引号与首尾空白
        // （与其它 opcode 同口径），但**名字的大小写不做任何归并**。
        let entry = opcode("midi_CC1", "64", 12);
        assert_eq!(entry.name(), "midi_CC1", "name case is preserved verbatim");
        assert_eq!(entry.value(), "64");
        assert_eq!(entry.line(), 12, "the line is the opcode's own line");
    }

    #[test]
    fn lookup_is_case_sensitive_and_takes_the_first_match() {
        let section = MidiSection::new(
            3,
            vec![
                opcode("cc1", "64", 4),
                opcode("CC1", "1", 5),
                opcode("cc1", "7", 6),
            ],
        );
        assert_eq!(section.line(), 3);
        assert_eq!(section.len(), 3);
        assert!(!section.is_empty());
        assert_eq!(section.opcode("cc1"), Some("64"), "first match wins");
        assert_eq!(
            section.opcode("CC1"),
            Some("1"),
            "lookup is case sensitive: the exact spelling matches"
        );
        assert_eq!(
            section.opcode("Cc1"),
            None,
            "no case folding, no alias guess"
        );
        assert_eq!(section.opcode("curve_index"), None);
        assert_eq!(
            section.opcodes()[2].value(),
            "7",
            "duplicates are all kept, in file order"
        );
    }

    #[test]
    fn an_empty_section_is_still_a_section() {
        let section: MidiSection<'_> = MidiSection::new(9, Vec::new());
        assert_eq!(section.line(), 9);
        assert_eq!(section.len(), 0);
        assert!(section.is_empty());
        assert_eq!(section.opcode("anything"), None);
    }

    #[test]
    fn lookup_never_panics_on_arbitrary_query_text() {
        let section = MidiSection::new(1, vec![opcode("cc1", "1", 2)]);
        for query in ["", "\u{00e9}", "cc1 ", " cc1", "\u{0000}"] {
            assert_eq!(section.opcode(query), None, "query {query:?}");
        }
    }
}
