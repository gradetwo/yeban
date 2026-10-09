//! SFZ `<effect>` 头：效果器总线声明（**定义段**）。
//!
//! 规范出处 <https://sfzformat.com/headers/effect/>，原文：
//!
//! > SFZ v2 header for effects controls.
//! > In SFZ v1 only `effect1` and `effect2` opcodes was available and only at `<region>`
//! > level. From SFZ v2 this header was added together with the addition of `effect3` and
//! > `effect4` opcodes also to modulate the related bus. Other opcodes listed in the book
//! > are `bus`, `type` and `dsp_order`.
//! > The specifics of what can be used under this header differ widely between SFZ players.
//!
//! 本模块只登记规范**白纸黑字给出类型 / 缺省 / 范围**的那一部分，不给任何效果器写 DSP：
//!
//! | opcode | 版本 | 类型 | 缺省 | 范围 / 选项 | 单位 | 出处 |
//! | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
//! | `bus` | SFZ v2 | string | `main` | `main`, `aux1`..`aux8`, `fx1`..`fx4`（ARIA 另有 `midi`） | | `/opcodes/bus/` |
//! | `type` | SFZ v2 | string | N/A | 15 个 SFZ 名 + 14 个 ARIA `com.mda.*` 名（**厂商自定义不受限**） | | `/opcodes/type/` |
//! | `dsp_order` | SFZ v2 | integer | N/A | `0` 到 `14` | N/A | `/opcodes/dsp_order/` |
//! | `effect1`..`effect4` | v1（1、2）/ v2（3、4） | float | `0` | `0` 到 `100` | `%` | `/opcodes/effect1/` |
//! | `param_offset` | ARIA | integer | N/A | （规范未给范围） | N/A | `/opcodes/param_offset/` |
//!
//! ## 契约
//!
//! - **`bus` 的未知取值解析成 [`EffectBus::Main`]**：规范原文
//!   "If not set, or any other value is set, this goes to the main output." ⇒ 未知取值
//!   **不是错误**（与 `type` 一样属于「厂商自由」），但可以用 [`EffectBus::is_option`]
//!   机械区分「文件写的就是 `main`」与「文件写了别的、被规范归约到 main」。
//! - **`type` 原样登记，不做任何解释**：规范自己写明
//!   "Effect type or vendor-specific effect name. Varies across SFZ players." ⇒ 本 crate
//!   不猜它对应哪个 DSP（不发明）。
//! - **定义段语义**：`<effect>` 的 opcode 只进 [`Effect`]，**绝不**写进
//!   `region → group → master → global` 继承链，也**不清空**任何继承作用域
//!   （与 [`crate::Curve`] 同一条口径）。
//! - **实时安全**：[`Effect`] 的读取全是字段访问，无分配、无锁、无 I/O、无日志。
//! - **逐位可复现**：本模块只有整数解析与字段存取，不含超越函数。
//!
//! ## 未建模的部分（登记，不静默降级）
//!
//! - `effect1` / `effect2` **在 `<region>` 段**（SFZ v1 用法）本 crate 不读：那是「region 的
//!   发送量」，与 `<effect>` 段的同名 opcode 语义不同，本切片只做 `<effect>` 段。
//! - `<sample>` 段头仍未建模（`<midi>` 段头见 [`crate::midi`]；两处的登记都在
//!   `docs/ledger/sfz-core-notes.md`，该文件由集成者独占）。

use std::borrow::Cow;

/// `bus` 的选项里 `aux` 总线的最大编号（规范选项表：`aux1` 到 `aux8`）。
pub const MAX_AUX_BUS: u8 = 8;

/// `bus` 的选项里 `fx` 总线的最大编号（规范选项表：`fx1` 到 `fx4`）。
pub const MAX_FX_BUS: u8 = 4;

/// `dsp_order` 的规范上界（规范范围 `0 to 14`）。
pub const MAX_DSP_ORDER: u8 = 14;

/// `<effect>` 段里发送量 opcode 的条数：`effect1` / `effect2` / `effect3` / `effect4`。
pub const SEND_COUNT: usize = 4;

/// `<effect>` 的 `bus` opcode：这条 `<effect>` 把信号送往哪条总线。
///
/// 规范选项表（<https://sfzformat.com/opcodes/bus/>）：
///
/// | Name | Version | Type | Default | Options | Unit |
/// | --- | --- | --- | --- | --- | --- |
/// | bus | SFZ v2 | string | main | main, aux1..aux8, fx1..fx4 | |
/// | | ARIA | | | midi | |
///
/// 规范原文："If not set, or any other value is set, this goes to the main output.
/// Possibly `main` is the default value."
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectBus {
    /// `main`（缺省）：主输出。
    Main,
    /// `aux1`..=`aux8`：辅助发送总线。
    Aux(u8),
    /// `fx1`..=`fx4`：SFZ v1 的 `effect1` / `effect2` 总线在 v2 里的编号形式。
    Fx(u8),
    /// `midi`（ARIA 扩展）：MIDI 预处理器，等价于 ARIA 的 `<midi>` 段头
    /// （见 <https://sfzformat.com/headers/midi/>："From ARIA v1.0.8.0+ an `<effect>`
    /// section with a `bus=midi` can be used instead."）。
    Midi,
}

impl EffectBus {
    /// 规范选项表里**列名**是否包含该取值（大小写不敏感）。
    ///
    /// 用途：规范要求未知取值归约到主输出，于是 [`EffectBus::from_value`] 会把 `aux9`、
    /// `fx5`、`garbage` 全部映射成 [`EffectBus::Main`]；调用方若要区分「文件写的就是
    /// `main`」与「文件写了别的」，用本函数。
    ///
    /// 空白先 trim（与其余 opcode 取值口径一致）。
    #[must_use]
    pub fn is_option(text: &str) -> bool {
        let needle = text.trim();
        needle.eq_ignore_ascii_case("main")
            || needle.eq_ignore_ascii_case("midi")
            || numbered(needle, "aux").is_some_and(|number| (1..=MAX_AUX_BUS).contains(&number))
            || numbered(needle, "fx").is_some_and(|number| (1..=MAX_FX_BUS).contains(&number))
    }

    /// 把 `bus` 的取值解析成总线。大小写不敏感（与 [`crate::OpcodeValue::as_option`] 同口径）。
    ///
    /// 规范原文："If not set, or any other value is set, this goes to the main output."
    /// ⇒ 任何不在选项表里的取值（含空取值）都返回 [`EffectBus::Main`]，**不是**错误。
    #[must_use]
    pub fn from_value(text: &str) -> Self {
        let needle = text.trim();
        if needle.eq_ignore_ascii_case("main") {
            return Self::Main;
        }
        if needle.eq_ignore_ascii_case("midi") {
            return Self::Midi;
        }
        if let Some(number) = numbered(needle, "aux")
            && (1..=MAX_AUX_BUS).contains(&number)
        {
            return Self::Aux(number);
        }
        if let Some(number) = numbered(needle, "fx")
            && (1..=MAX_FX_BUS).contains(&number)
        {
            return Self::Fx(number);
        }
        Self::Main
    }

    /// 总线编号：`Aux(n)` / `Fx(n)` 返回 `Some(n)`；`Main` / `Midi` 没有编号 ⇒ `None`。
    ///
    /// 用途：引擎侧把 `Aux(n)` / `Fx(n)` 映射到总线数组下标。本 crate 不定义总线数组。
    #[must_use]
    pub fn index(self) -> Option<u8> {
        match self {
            Self::Aux(number) | Self::Fx(number) => Some(number),
            Self::Main | Self::Midi => None,
        }
    }
}

/// 把 `auxN` / `fxN` 里的 `N` 解析成数字；前缀大小写不敏感，`N` 只允许 ASCII 十进制数字。
///
/// 返回 `None` 表示形状不对（缺前缀、无数字、含非数字、或 `N` 超出 `u8`）。
/// 不 panic：UTF-8 边界用 [`str::get`] 取，切不开就是 `None`。
fn numbered(text: &str, prefix: &str) -> Option<u8> {
    let head = text.get(..prefix.len())?;
    if !head.eq_ignore_ascii_case(prefix) {
        return None;
    }
    let digits = text.get(prefix.len()..)?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u8>().ok()
}

/// 一条 `<effect>` 段头定义的效果器总线声明。
///
/// 只承载规范表格给出的那几个 opcode；**不含**任何 DSP、也不含效果器参数
/// （参数由 ARIA 的 `param_offset` + `set_ccN` 在 `<control>` 段里配置，本 crate 不读）。
#[derive(Debug, Clone, PartialEq)]
pub struct Effect<'a> {
    bus: EffectBus,
    type_name: Option<Cow<'a, str>>,
    param_offset: Option<i64>,
    dsp_order: Option<u8>,
    sends: [f32; SEND_COUNT],
}

impl<'a> Effect<'a> {
    /// 这条 `<effect>` 的目标总线（缺省 [`EffectBus::Main`]）。
    #[must_use]
    pub fn bus(&self) -> EffectBus {
        self.bus
    }

    /// `type` 的原始取值（去引号、去首尾空白、已做宏替换）；未给出为 `None`。
    ///
    /// **不做解释**：规范写明它是「效果类型或厂商自定义效果名」，跨播放器不一致。
    /// 这里的 `Cow` 在文件没写宏时是借用的（零拷贝）。
    #[must_use]
    pub fn type_name(&self) -> Option<&str> {
        self.type_name.as_deref()
    }

    /// ARIA 的 `param_offset`：给内置 / 厂商效果器的参数编号加一个偏移量；未给出为 `None`。
    #[must_use]
    pub fn param_offset(&self) -> Option<i64> {
        self.param_offset
    }

    /// `dsp_order`：Rapture DSP 块的信号流类型（规范范围 `0` 到 `14`）；未给出为 `None`。
    #[must_use]
    pub fn dsp_order(&self) -> Option<u8> {
        self.dsp_order
    }

    /// `effect1`..=`effect4` 的取值（规范缺省 `0`，单位 `%`），按编号顺序。
    ///
    /// 取值**不钳位**：规范范围是 `0` 到 `100`，但本 crate 与 `volume` / `pan` 同口径，
    /// 只保证读到的是有限 `f32`（`NaN` / `±Inf` 在解析期就是 [`crate::SfzError`]）。
    #[must_use]
    pub fn sends(&self) -> &[f32; SEND_COUNT] {
        &self.sends
    }

    /// 第 `index` 个发送量（`0` 是 `effect1`，`3` 是 `effect4`）；越界返回 `None`。
    #[must_use]
    pub fn send(&self, index: usize) -> Option<f32> {
        self.sends.get(index).copied()
    }
}

/// `<effect>` 段的构造器（解析器内部使用）：逐 opcode 累积，段结束时归约成 [`Effect`]。
#[derive(Debug, Clone)]
pub(crate) struct EffectBuilder<'a> {
    bus: Option<EffectBus>,
    type_name: Option<Cow<'a, str>>,
    param_offset: Option<i64>,
    dsp_order: Option<u8>,
    sends: [Option<f32>; SEND_COUNT],
}

impl<'a> EffectBuilder<'a> {
    /// 空的段构造器（`<effect>` 段头出现时调用）。
    pub(crate) fn new() -> Self {
        Self {
            bus: None,
            type_name: None,
            param_offset: None,
            dsp_order: None,
            sends: [None; SEND_COUNT],
        }
    }

    /// 记录 `bus`（规范缺省 `main`；未知取值归约成 `main`）。
    pub(crate) fn set_bus(&mut self, value: &str) {
        self.bus = Some(EffectBus::from_value(value));
    }

    /// 记录 `type`（原样，不解释）。
    pub(crate) fn set_type(&mut self, value: Cow<'a, str>) {
        self.type_name = Some(value);
    }

    /// 记录 `param_offset`。
    pub(crate) fn set_param_offset(&mut self, value: i64) {
        self.param_offset = Some(value);
    }

    /// 记录 `dsp_order`（范围检查在解析器里做）。
    pub(crate) fn set_dsp_order(&mut self, value: u8) {
        self.dsp_order = Some(value);
    }

    /// 记录第 `index` 个发送量（`index >= SEND_COUNT` 时忽略：调用方只喂 1..=4）。
    pub(crate) fn set_send(&mut self, index: usize, value: f32) {
        if let Some(slot) = self.sends.get_mut(index) {
            *slot = Some(value);
        }
    }

    /// 这一段是否登记到了任何规范 opcode。
    ///
    /// 空段（含「只写了非规范 opcode」的段）没有数据可丢 ⇒ 不产生 [`Effect`] 也不报错，
    /// 与 `<curve>` 的空段口径一致。
    pub(crate) fn has_data(&self) -> bool {
        self.bus.is_some()
            || self.type_name.is_some()
            || self.param_offset.is_some()
            || self.dsp_order.is_some()
            || self.sends.iter().any(Option::is_some)
    }

    /// 归约成 [`Effect`]，补上规范缺省（`bus=main`、`effect1..4=0`）。
    pub(crate) fn build(self) -> Effect<'a> {
        Effect {
            bus: self.bus.unwrap_or(EffectBus::Main),
            type_name: self.type_name,
            param_offset: self.param_offset,
            dsp_order: self.dsp_order,
            sends: self.sends.map(|slot| slot.unwrap_or(0.0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bus_option_table_matches_the_specification() {
        // 出处 <https://sfzformat.com/opcodes/bus/> 的选项表。
        for (text, expected) in [
            ("main", EffectBus::Main),
            ("aux1", EffectBus::Aux(1)),
            ("aux8", EffectBus::Aux(8)),
            ("fx1", EffectBus::Fx(1)),
            ("fx4", EffectBus::Fx(4)),
            ("midi", EffectBus::Midi),
        ] {
            assert_eq!(EffectBus::from_value(text), expected, "bus={text}");
            assert!(EffectBus::is_option(text), "bus={text} is a listed option");
        }
        assert_eq!(EffectBus::Aux(1).index(), Some(1));
        assert_eq!(EffectBus::Fx(4).index(), Some(4));
        assert_eq!(EffectBus::Main.index(), None);
        assert_eq!(EffectBus::Midi.index(), None);
    }

    #[test]
    fn bus_is_case_insensitive_like_the_other_option_opcodes() {
        for (text, expected) in [
            ("MAIN", EffectBus::Main),
            ("Aux3", EffectBus::Aux(3)),
            ("  aux3  ", EffectBus::Aux(3)),
            ("FX2", EffectBus::Fx(2)),
            ("Midi", EffectBus::Midi),
        ] {
            assert_eq!(EffectBus::from_value(text), expected, "bus={text:?}");
        }
    }

    #[test]
    fn an_out_of_table_bus_value_falls_back_to_main_and_is_not_an_option() {
        // 规范原文："If not set, or any other value is set, this goes to the main output."
        // ⇒ 未知取值归约成 main，且 is_option 必须说「这不是表里的名字」。
        for text in [
            "", "  ", "aux0", "aux9", "fx0", "fx5", "garbage", "aux", "fx", "aux1x", "2",
        ] {
            assert_eq!(EffectBus::from_value(text), EffectBus::Main, "bus={text:?}");
            assert!(
                !EffectBus::is_option(text),
                "bus={text:?} is not a listed option"
            );
        }
    }

    #[test]
    fn bus_parsing_never_panics_on_arbitrary_text() {
        // 不可信输入：任意字节不得 panic（含 UTF-8 多字节被前缀切开的情形）。
        let cases: [&str; 8] = [
            "aux\u{00e9}",
            "fx\u{00e9}",
            "\u{00e9}",
            "aux999999999999999999999999",
            "-1",
            "aux-1",
            "aux 1",
            "aux\u{0000}",
        ];
        for text in cases {
            let parsed = EffectBus::from_value(text);
            assert_eq!(parsed, EffectBus::Main, "bus={text:?}");
            assert!(!EffectBus::is_option(text), "bus={text:?}");
        }
        // 前缀长于整个输入时 `str::get` 必须返回 None 而不是 panic。
        assert!(!EffectBus::is_option("\u{00e9}"));
    }

    #[test]
    fn builder_fills_the_specification_defaults() {
        let effect = EffectBuilder::new().build();
        assert_eq!(effect.bus(), EffectBus::Main, "bus default is main");
        assert_eq!(effect.type_name(), None);
        assert_eq!(effect.param_offset(), None);
        assert_eq!(effect.dsp_order(), None);
        assert_eq!(
            effect.sends(),
            &[0.0, 0.0, 0.0, 0.0],
            "effect1..4 default 0"
        );
        assert!(
            !EffectBuilder::new().has_data(),
            "empty section has no data"
        );
    }

    #[test]
    fn builder_records_every_opcode_of_the_table() {
        let mut builder = EffectBuilder::new();
        builder.set_bus("aux2");
        builder.set_type(Cow::Borrowed("com.mda.Limiter"));
        builder.set_param_offset(400);
        builder.set_dsp_order(3);
        builder.set_send(0, 12.5);
        builder.set_send(3, 100.0);
        assert!(builder.has_data());
        let effect = builder.build();
        assert_eq!(effect.bus(), EffectBus::Aux(2));
        assert_eq!(effect.type_name(), Some("com.mda.Limiter"));
        assert_eq!(effect.param_offset(), Some(400));
        assert_eq!(effect.dsp_order(), Some(3));
        assert_eq!(effect.send(0), Some(12.5));
        assert_eq!(effect.send(1), Some(0.0));
        assert_eq!(effect.send(2), Some(0.0));
        assert_eq!(effect.send(3), Some(100.0));
        assert_eq!(effect.send(4), None, "out of range send index");
    }

    #[test]
    fn a_section_with_only_a_non_normative_opcode_has_no_data() {
        // 非规范 opcode 由解析器忽略；这里确认构造器本身不会因此报告「有数据」。
        assert!(!EffectBuilder::new().has_data());
        assert_eq!(MAX_DSP_ORDER, 14, "spec range is 0 to 14");
        assert_eq!(MAX_AUX_BUS, 8);
        assert_eq!(MAX_FX_BUS, 4);
        assert_eq!(SEND_COUNT, 4);
    }
}
