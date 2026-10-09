//! SFZ v2 的 `<control>set_ccN`：乐器**加载时**的 MIDI CC 初始值。
//!
//! 规范出处（<https://sfzformat.com/opcodes/set_ccN/>，整页只有两句话加一张表）：
//!
//! - 正文第 1 句："Sets a default initial value for MIDI CC number N, when the
//!   instrument is initially loaded."；
//! - 正文第 2 句："Used under the ‹`control`› header."；
//! - 表格：Name = `set_ccN`、Version = SFZ v2、Type = integer、Default = N/A、
//!   **Range = 0 to 127**、Unit = N/A；Category = Instrument Settings；
//! - 该页的两个例子：`set_cc40=127`、`set_cc100=30`。
//!
//! ## 为什么它不是"纯 GUI 元数据"
//!
//! [`crate::label`] 那一族的 `label_ccN` 只影响显示，本 crate 明确写它「不改变任何
//! region 选择、门控或渲染结果」。`set_ccN` 不同：本 crate 已经用调用方给的 CC 探针
//! 判定 `loccN` / `hiccN` 门控（[`crate::Region::cc_gates_ok`]）与
//! `xfin_loccN` / `xfout_*` 交叉淡化（[`crate::Region::crossfade_gain`]）。
//! 文件声明「加载时 CC7 = 100」而调用方从 0 起步 ⇒ 这些门控与淡化在**第一拍**就落在
//! 错的档位上。因此本切片把该声明带出来给调用方（引擎）用作 CC 初值，
//! [`crate::Instrument::cc_defaults`] 就是那个交接点。
//!
//! ## 契约
//!
//! - **不改变本 crate 的任何判定**：与 `label_ccN` 一样，`set_ccN` **不**参与 region
//!   选择、门控与渲染；它只是把文件声明的初值交给调用方。
//! - **零拷贝**：本体只搬运整数，取值不保留文本。
//! - **实时安全**：读取是 `BTreeMap` 查找 / 切片遍历，无分配、无锁、无阻塞 I/O、无日志，
//!   可在逐样本路径调用。
//! - **逐位可复现**：只有整数，不含浮点运算；表是 [`std::collections::BTreeMap`]，
//!   迭代顺序由 CC 下标唯一确定 [ARCH-DET-001]。
//!
//! ## 工程裁决（登记，不静默降级）
//!
//! 1. **下标 `N` 的域 = `label_ccN` 的域**（[`MAX_CC_DEFAULT_INDEX`]，即
//!    [`crate::label::MAX_CC_LABEL_INDEX`] = `u16::MAX`）。
//!    规范表把 Range 写成 `0 to 127`，但那说的是**取值**（见第 2 条），不是 `N`：
//!    登记语料里有 `set_cc400=63` / `set_cc401=63`，它们恰好出现在**同样 19 个文件**里
//!    （`assets/samples/karoryfer-big-rusty-drums/`），而那 19 个文件同时声明
//!    `label_cc400` / `label_cc401` —— 本 crate 已按 `u16` 容器界接受后者。
//!    若把 Range 读成 `N`，这 19 个文件会在同一族里出现两种口径（`label_ccN` 收、
//!    `set_ccN` 拒），且与 ARIA 扩展 CC 的登记事实冲突
//!    （<https://sfzformat.com/extensions/midi_ccs/>："Anything above 137 is not specified
//!    in the SFZ 2 standard and strictly engine-dependent"）。**越界是明确 `Err`**。
//! 2. **取值的两条口径不同，因为证据不同**：
//!    - **不是十进制整数 ⇒ 丢弃该声明 + [`crate::Warning::MalformedSetCc`] 告警**（**不**中断解析）。
//!      登记语料里真有这种写法：`set_cc32=63.5` 一类**共 42 处、落在 10 个文件**里
//!      （`assets/samples/hungarian-zither/`、`assets/samples/virtuosity-drums/`、
//!      `assets/samples/body-percussion/` —— 作者用 `63.5` 表示 7 位 CC 的中点）。
//!      规范表格写 Type = integer，所以它**没有**可表示的值；但若在这里硬 `Err`，
//!      那 10 个已登记乐器会整份无法加载 —— 这是对已登记素材的功能回退，
//!      而本 crate 对「无法表示的声明」的既有口径正是**丢弃 + 告警**
//!      （同 [`crate::Warning::RegionWithoutSample`]，绝不猜一个值）。
//!    - **是整数但不在 `0 to 127` ⇒ 明确 [`crate::SfzError::IntegerOutOfRange`]**。
//!      登记语料里这种取值**出现 0 次**，因此这里沿用本 crate 对所有规范 Range 的
//!      既有纪律（`read_xf_endpoint` 的 `0..=127`、`amp_velcurve_N` 的 `0..=1`
//!      都是明确 `Err`，不静默钳位）。
//!
//!    两种失败都**不**静默：一条有告警，一条有错误载荷。
//! 3. **只认 `<control>` 作用域**：规范正文第 2 句就是 "Used under the ‹`control`› header."。
//!    与 `default_path` 同一条口径（本 crate 也只从 `<control>` 读它）。写在
//!    `region` / `group` / `master` / `global` 里的 `set_ccN` 按未知 opcode 忽略。
//!    登记语料 1137 处出现**全部**落在 `<control>`，因此这条口径在语料上无副作用。
//! 4. **多个 `<control>` 段里的同一个 CC ⇒ 后者覆盖前者**，且**不**被新段头重置。
//!    理由：本条与 `label_ccN` 在同一作用域、同一形状（都是文件级声明），
//!    `label_ccN` 已是「后者覆盖前者」（见 [`crate::Instrument::cc_labels`]）；
//!    两个兄弟族用两条相反的口径会让同一文件里的 CC 元数据自相矛盾。
//!    登记语料里**没有**一个文件含两个以上 `<control>` 段（169 个文件有 1 个、
//!    1229 个没有），因此这条是纯工程裁决、语料未验证。
//!
//! ## 登记语料普查（本次测量，2026-10-09）
//!
//! 两把尺子，读数分开写，不要混用：
//!
//! - **原文尺**（`/tmp` 下的一次性 `python3 -B` 脚本：先剥 `//` 行注释、引号内不截断、
//!   **不模拟 `#define` 展开**）——量的是 `.sfz` 文件里的字面文本。
//! - **解析器尺**（`/tmp` 下的一次性 `rustc` 探针，链接本 crate 的 rlib：
//!   `IncludeResolver::resolve` + `parse_sources`，母体同样是 1398 个登记 `.sfz`）
//!   ——量的是解析器**真正看到**的东西。
//!
//! 母体 N = 1398 个文件（同 [`crate::label`] 与 `docs/ledger/sfz-core-notes.md` 第 11 节
//! 的口径）。两个探针都**未入库**。
//!
//! | 读数（尺子） | 值 |
//! | :--- | ---: |
//! | 含 `set_ccN` 的文件（原文） | 105 |
//! | `set_ccN` 出现次数（原文） | 1 137 |
//! | 其中落在 `<control>` 段的次数（原文） | 1 137（**全部**） |
//! | 不同 CC 下标数 / 下标范围（原文） | 82 / 1..=401 |
//! | 取值字面是 `0..=127` 整数的次数（原文） | 1 086 |
//! | 取值字面是整数但越界的次数（原文） | **0** |
//! | 取值**字面**不是十进制整数的次数 / 文件数（原文） | 51 / 19 |
//! | 其中字面 `63.5` | 42（10 个文件） |
//! | 其中字面 `$ht_lo_hi_init` | 9（9 个文件）—— **这是原文尺的假阳性**：那 9 个文件第 7 行写着 `#define $ht_lo_hi_init 127`，解析器把它展开成 `127`，因此**不是**不可表示的取值 |
//! | 空取值（`set_ccN=`）的次数（原文） | 0 |
//! | 解析器真正丢弃的声明数 / 文件数（解析器，`Warning::MalformedSetCc`） | 42 / 10（与「字面 `63.5`」逐数吻合） |
//! | 解析器暴露的 CC 初值条数 / 带初值的文件数（解析器） | 799 / 83 |
//! | 因 `set_ccN` 而整份解析失败的文件数（解析器） | **0** |
//! | 同一文件里含 `label_ccN` 的（原文，兄弟族对照读数） | 105 个里有 99 个 |
//! | 对照：`label_ccN` 文件数（原文；已建模的兄弟族） | 102 |
//! | 对照：`note_polyphony` 文件数（原文；已建模） | 32 |
//! | 对照：`off_time` 文件数（原文；已建模） | 15 |
//! | 对照：`polyphony_group` / `loprog` 文件数（原文；别名 / 未登记，出现 0 次） | 0 / 0 |
//!
//! 最后四行是**同一次扫描**的对照读数：同一探针在已建模成员上给出非零计数，
//! 证明「模式」确实在匹配，而不是整表恒 0。
//!
//! **解析口径的对照（解析器尺，同一探针在改动前后各跑一次）**：1398 个文件上
//! `parse_ok=1267` / `parse_err=24` / include 解析失败 `107`，
//! 错误 opcode 直方图 `{key: 20, lokey: 3, loop_mode: 1}` —— 两次输出 `cmp` 退出码 0，
//! 即本切片**没有**改变任何一个登记文件的解析结果。

use crate::error::SfzError;

/// `set_ccN` 的 `N` 上界。
///
/// 直接取 [`crate::label::MAX_CC_LABEL_INDEX`]（`u16::MAX`）而不是再写一遍 `65535`：
/// 两个族的下标域是**同一个事实**（ARIA 扩展 CC 编号），本仓库的纪律是同一事实只留一处。
/// 理由见模块文档的「工程裁决」第 1 条。
pub const MAX_CC_DEFAULT_INDEX: u16 = crate::label::MAX_CC_LABEL_INDEX;

/// `set_ccN` 取值的上界（规范表格 Range = `0 to 127`）。
///
/// 下界是 `0`。越过上界是明确 [`crate::SfzError::IntegerOutOfRange`]，
/// 不静默钳位、不静默丢弃（见模块文档「工程裁决」第 2 条）。
pub const MAX_CC_DEFAULT_VALUE: u8 = 127;

/// 识别 `set_ccN`，返回 `N` 的**原始数字串**。
///
/// 名字大小写敏感（与 [`crate::label::parse_cc_label_name`] 及 [`crate::instrument`] 里
/// 其它 opcode 的读取口径一致，只有段头名大小写不敏感）：`SET_CC7` 不是本族的 opcode。
///
/// 返回数字串而不是解析结果，是为了让调用方区分两种失败：
/// 「名字不像 `set_ccN`」（返回 [`None`]，与未知 opcode 同口径地忽略）与
/// 「名字像但下标越界」（调用方给出明确 `Err`）。
#[must_use]
pub fn parse_set_cc_name(name: &str) -> Option<&str> {
    let digits = name.strip_prefix("set_cc")?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(digits)
}

/// 一个 `<control>` opcode 的解析结果（见 [`set_cc_declaration`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SetCcOutcome {
    /// 名字不是 `set_ccN` ⇒ 与其它未知 opcode 同口径地忽略。
    NotThisOpcode,
    /// 命中：CC `cc` 的初始值是 `initial`。
    Value {
        /// MIDI CC 编号。
        cc: u16,
        /// 规范表格 Range = `0 to 127` 的初始值。
        initial: u8,
    },
    /// 名字像 `set_ccN`、**下标**也合法，但取值不是十进制整数（登记语料里有 `63.5`）。
    ///
    /// 调用方**丢弃这一条声明**并产生 [`crate::Warning::MalformedSetCc`]，
    /// 文件继续解析（见模块文档「工程裁决」第 2 条）。
    UnrepresentableValue,
}

/// 把一个 `<control>` 段里的 opcode 解析成 `set_ccN` 声明。
///
/// 结果刻意不合并（调用方需要区别对待），三类：
///
/// 1. [`SetCcOutcome::NotThisOpcode`]：名字不是 `set_ccN`；
/// 2. [`SetCcOutcome::Value`]：命中，`cc` 不越过 [`MAX_CC_DEFAULT_INDEX`]、
///    取值不越过 [`MAX_CC_DEFAULT_VALUE`]；
/// 3. [`SetCcOutcome::UnrepresentableValue`]：取值不是十进制整数（丢弃 + 告警）；
/// 4. `Err(`[`crate::SfzError::IntegerOutOfRange`]`)`：**下标**越
///    [`MAX_CC_DEFAULT_INDEX`]（与 `label_ccN` 同一条容器界），或**取值是整数但不在**
///    `0..=127`（此时 `min` / `max` 是 `0` / `127`）。
///
/// 第 3 类**不**是错误（否则 10 个已登记文件会整份无法加载），第 4 类**是**明确 `Err`；
/// 两类都**不**静默（见模块文档「工程裁决」第 1、2 条）。
///
/// 该函数只在**解析期**调用；错误与告警载荷里的 `String` 由调用方分配，
/// 因此它**不在**实时路径上。
pub(crate) fn set_cc_declaration(
    name: &str,
    value: &str,
    line: usize,
) -> Result<SetCcOutcome, SfzError> {
    let Some(digits) = parse_set_cc_name(name) else {
        return Ok(SetCcOutcome::NotThisOpcode);
    };
    let cc = match digits.parse::<u16>() {
        Ok(cc) => cc,
        // `digits` 已保证是全 ASCII 数字，因此这里只有「超过 u16」这一种失败。
        // 错误载荷按 i64 饱和取值（更长的数字串仍能报出行号与 opcode 名）。
        Err(_) => {
            return Err(SfzError::IntegerOutOfRange {
                line,
                opcode: name.to_string(),
                value: digits.parse::<i64>().unwrap_or(i64::MAX),
                min: 0,
                max: i64::from(MAX_CC_DEFAULT_INDEX),
            });
        }
    };
    // 不是十进制整数（含没展开的 `$VAR` 文本）⇒ 丢弃 + 告警，**不**中断解析。
    let Some(parsed) = crate::parser::parse_int(value) else {
        return Ok(SetCcOutcome::UnrepresentableValue);
    };
    if !(0..=i64::from(MAX_CC_DEFAULT_VALUE)).contains(&parsed) {
        return Err(SfzError::IntegerOutOfRange {
            line,
            opcode: name.to_string(),
            value: parsed,
            min: 0,
            max: i64::from(MAX_CC_DEFAULT_VALUE),
        });
    }
    // 取值范围已在上一行限定在 `0..=127`，因此这里的 `as u8` 不截断。
    Ok(SetCcOutcome::Value {
        cc,
        initial: parsed as u8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_cc_name_needs_a_nonempty_all_digit_suffix() {
        assert_eq!(parse_set_cc_name("set_cc0"), Some("0"));
        assert_eq!(parse_set_cc_name("set_cc7"), Some("7"));
        assert_eq!(parse_set_cc_name("set_cc127"), Some("127"));
        // 登记语料里的扩展下标（`assets/samples/karoryfer-big-rusty-drums/`，
        // 与 `label_cc400` / `label_cc401` 出现在同样 19 个文件里）。
        assert_eq!(parse_set_cc_name("set_cc400"), Some("400"));
        assert_eq!(parse_set_cc_name("set_cc401"), Some("401"));
        // 前导零原样带出：解析由调用方按十进制做，`007` 与 `7` 是同一个下标。
        assert_eq!(parse_set_cc_name("set_cc007"), Some("007"));

        for bad in [
            "set_cc",
            "set_c",
            "set_ccX",
            "set_cc7x",
            "set_cc-1",
            "set_cc7 ",
            " set_cc7",
            "SET_CC7",
            "Set_cc7",
            "set_cc 7",
            "set_ccx7",
            "xset_cc7",
            "label_cc7",
            "set_cc₁",
        ] {
            assert_eq!(parse_set_cc_name(bad), None, "{bad:?} must not match");
        }
    }

    #[test]
    fn the_index_domain_is_the_shared_aria_cc_domain() {
        // 同一个事实只留一处：本常量就是 `label_ccN` 的常量。
        assert_eq!(MAX_CC_DEFAULT_INDEX, crate::label::MAX_CC_LABEL_INDEX);
        assert_eq!(MAX_CC_DEFAULT_INDEX, 65535);

        // 上界本身可解析；再大一位就落到下标的越界分支。
        assert_eq!(
            set_cc_declaration("set_cc65535", "1", 1).expect("in range"),
            SetCcOutcome::Value {
                cc: 65535,
                initial: 1
            }
        );
        assert!(matches!(
            set_cc_declaration("set_cc65536", "1", 7),
            Err(SfzError::IntegerOutOfRange {
                line: 7,
                opcode,
                value: 65536,
                min: 0,
                max: 65535,
            }) if opcode == "set_cc65536"
        ));
    }

    #[test]
    fn the_value_domain_is_the_specification_table_range() {
        assert_eq!(MAX_CC_DEFAULT_VALUE, 127);
        // 两个端点都是合法取值（规范表格 Range = `0 to 127`）。
        assert_eq!(
            set_cc_declaration("set_cc7", "0", 1).expect("low end"),
            SetCcOutcome::Value { cc: 7, initial: 0 }
        );
        assert_eq!(
            set_cc_declaration("set_cc7", "127", 1).expect("high end"),
            SetCcOutcome::Value {
                cc: 7,
                initial: 127
            }
        );
        // 整数但越界 ⇒ 明确 Err（规范 Range；登记语料里出现 0 次）。
        for (text, reported) in [("128", 128i64), ("-1", -1), ("1000", 1000)] {
            assert!(
                matches!(
                    set_cc_declaration("set_cc7", text, 3),
                    Err(SfzError::IntegerOutOfRange {
                        line: 3,
                        value,
                        min: 0,
                        max: 127,
                        ..
                    }) if value == reported
                ),
                "{text:?} must be out of range"
            );
        }
    }

    #[test]
    fn a_non_integer_value_is_dropped_loudly_and_not_an_error() {
        // 登记语料里 42 处 `63.5`（10 个文件）：硬 `Err` 会让那些乐器整份无法加载，
        // 因此这里是「丢弃 + 告警」而不是错误（见模块文档「工程裁决」第 2 条）。
        // 没展开的 `$VAR` 文本也会落到这里（未定义宏按原样保留，见 `Warning::UndefinedMacro`）。
        for text in ["63.5", "abc", "", "1.5", "12x", "$undefined_macro", "１２"] {
            assert_eq!(
                set_cc_declaration("set_cc7", text, 5).expect("not an error"),
                SetCcOutcome::UnrepresentableValue,
                "{text:?} must be dropped, not errored"
            );
        }
        // 前导 `+` 与本 crate 的 `parse_int` 同口径：是合法整数语法。
        assert_eq!(
            set_cc_declaration("set_cc7", "+64", 1).expect("plus sign"),
            SetCcOutcome::Value { cc: 7, initial: 64 }
        );
    }

    #[test]
    fn a_name_that_is_not_set_cc_is_ignored_not_an_error() {
        for other in ["set_cc", "set_ccX", "SET_CC7", "label_cc7", "default_path"] {
            assert_eq!(
                set_cc_declaration(other, "not an integer", 9).expect("ignored"),
                SetCcOutcome::NotThisOpcode,
                "{other:?} must not be treated as set_ccN"
            );
        }
    }

    #[test]
    fn an_overflowing_index_reports_the_saturated_value() {
        // 与 `label_ccN`（`crate::label::cc_label_index`）同一条形状：名字已保证是
        // 全 ASCII 数字，因此 `parse::<u16>` 只有「超过 u16」这一种失败，错误载荷按
        // `i64` 饱和取值（更长的数字串仍能报出行号与 opcode 名）。
        // 上面那条判据只核对 65536 这个恰好可表示的值 ⇒ 把饱和值改成 0 也全绿。
        let error = set_cc_declaration("set_cc99999999999999999999999", "1", 7)
            .expect_err("above the u16 container");
        assert!(
            matches!(
                &error,
                SfzError::IntegerOutOfRange { line: 7, opcode, value, min: 0, max, .. }
                    if opcode == "set_cc99999999999999999999999"
                        && *value == i64::MAX
                        && *max == i64::from(MAX_CC_DEFAULT_INDEX)
            ),
            "unexpected error: {error:?}"
        );
    }
}
