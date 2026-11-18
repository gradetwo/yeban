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
//!
//!    ⭐ **追加修订段（2026-11-18，`line/drum-fx-layer`）** —— 上面第一条里
//!    「`63.5` 没有可表示的值 ⇒ 丢弃」的前提被实测推翻，修订如下（原文保留备查）：
//!    作者用 `63.5` 表示 7 位 CC 域的**精确中点**，且文件同时以双极曲线
//!    （`tune_ccN=1200` + `tune_curveccN=1`）消费这些 CC —— 种子缺席时 CC 落 0，
//!    双极曲线在 0 处给出 −满程偏移。2026-11-18 全库装载审计实测代价：
//!    `virtuosity-drums`（`set_cc72/76/81/86/90=63.5` 全丢弃 ⇒ 鼓整组低
//!    1200–2400 音分，kick 原样本 83 Hz 在渲染里落到约 25 Hz，即 boombap 耳验
//!    怪音色的根因）、`body-percussion`（`set_cc41` 丢弃 ⇒ 全库低 1200 音分）、
//!    `hungarian-zither`（`set_cc32` 丢弃 ⇒ 全库低 700 音分），共 2363 条 region。
//!    且本 crate 对 ARIA `set_hdccN` 早有先例：浮点 CC 取值按 `round` 换算进
//!    同一个 `u8` 域（见本文件末节）——「浮点不可表示」与该先例自相矛盾。
//!    **新口径**：取值是 `0..=127` 内的有限小数 ⇒ `round` 取整为种子
//!    （`63.5` → 64；残差至多半个 CC 步，在 1200 音分满程的 tune 旋钮上约
//!    ±9.4 音分，听感可忽略）；无法按数值解读（字母、未展开宏、全角数字、空）
//!    或小数越界 ⇒ 维持丢弃 + 告警；整数越界 ⇒ 维持明确 `Err`。
//!    语料表里「解析器真正丢弃的声明数 42 / 10」是修订前的读数，修订后这 42 处
//!    全部成为种子（值 64），丢弃数归零。
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
//!
//! ⭐ **追加修订段（任务单 `#206`）** —— 被修订的是上面那句里的「include 解析失败 `107`」
//! （⛔ 原文一字未删）：`#206` 让 `#include` 支持 **`.txt`** 目标与**行内**写法后重测，
//! include 解析失败 ＝ **97**（−10 条此前被扩展名挡住的 `.txt`）。同一探针的
//! `parse_ok` / `parse_err` 仍是 `#205` 的读数（⛔ 未在 `#206` 重测，⛔ 不冒充新读数）。
//!
//! ## ⭐ ARIA 的 `set_hdccN`（第五份报告 §6，本片）
//!
//! 规范逐字（<https://sfzformat.com/opcodes/set_hdccN/>）：
//!
//! > Like `set_ccN` but with floating point MIDI CCs. Sets a default initial value for
//! > MIDI CC number N using a floating point value, when the instrument is initially loaded.
//! > `set_realccN` is a deprecated alias for `set_hdccN`.
//!
//! 语料 10 处（`salamander-grand` 1 处 `set_hdcc10=0.5`；`ixox-flute` 9 处含宏形态）。
//! 本仓的 CC 域是 7 位 `u8` ⇒ **取值** `0.0..=1.0` 在解析期按 `round(v × 127)` 换算
//! （0.5 → 64）；**下标**域照抄 `set_ccN`（`u16`，不把 Range 错读成下标上界）。
//! 与 `set_ccN` 同时声明同一 CC 时 **`set_ccN` 赢**（精确整数 > 扩展换算值，
//! 与书写顺序无关）—— 在 [`crate::parser`] 的 `finish()` 里用 `entry().or_insert()` 合并。

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
    /// 名字像 `set_ccN`、**下标**也合法，但取值无法按数值解读（字母、未展开的
    /// `$VAR`、空、全角数字），或是一个越过 `0..=127` 的小数。
    ///
    /// 调用方**丢弃这一条声明**并产生 [`crate::Warning::MalformedSetCc`]，
    /// 文件继续解析（见模块文档「工程裁决」第 2 条）。注意 `0..=127` 内的小数
    /// **不**落这里：它按 `round` 取整为种子（2026-11-18 修订）。
    UnrepresentableValue,
}

/// 把一个 `<control>` 段里的 opcode 解析成 `set_ccN` 声明。
///
/// 结果刻意不合并（调用方需要区别对待），三类：
///
/// 1. [`SetCcOutcome::NotThisOpcode`]：名字不是 `set_ccN`；
/// 2. [`SetCcOutcome::Value`]：命中，`cc` 不越过 [`MAX_CC_DEFAULT_INDEX`]、
///    取值不越过 [`MAX_CC_DEFAULT_VALUE`]；`0..=127` 内的小数按 `round`
///    取整后同样落这一类（2026-11-18 修订）；
/// 3. [`SetCcOutcome::UnrepresentableValue`]：取值无法按数值解读、或是越界小数
///    （丢弃 + 告警）；
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
    let Some(parsed) = crate::parser::parse_int(value) else {
        // 不是十进制整数时再试小数（2026-11-18 修订，见模块文档裁决第 2 条的
        // 修订段）：`0..=127` 内的有限小数按 `round` 取整为种子——`63.5` 是
        // 7 位域的精确中点，作者意图无歧义，与 `set_hdccN` 的 `round` 换算同款。
        // 不是数值（字母、未展开的 `$VAR`、全角数字、空）或小数越界
        // ⇒ 丢弃 + 告警，**不**中断解析。
        let decimal = value.trim().parse::<f32>().ok().filter(|v| {
            v.is_finite() && (0.0..=f32::from(MAX_CC_DEFAULT_VALUE)).contains(v)
        });
        let Some(v) = decimal else {
            return Ok(SetCcOutcome::UnrepresentableValue);
        };
        // 取值已限定在 `0..=127`，`round` 后仍在域内 ⇒ `as u8` 不截断。
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let initial = v.round() as u8;
        return Ok(SetCcOutcome::Value { cc, initial });
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

/// `set_hdccN`（ARIA 浮点 CC 初值）的名字解析：`set_hdccN` 本体与已废弃别名
/// `set_realccN`（规范页逐字 "set_realccN is a deprecated alias for set_hdccN"）。
///
/// 下标域与 `set_ccN` 完全相同（[`parse_set_cc_name`] / [`MAX_CC_DEFAULT_INDEX`]）：
/// 规范表把 `N` 写在名字里、Range 只约束**取值**（0..=1），语料里又有 `set_hdcc400`
/// 一类的 ARIA 扩展 CC —— ⛔ 不把 Range 错读成下标上界。
#[must_use]
pub fn parse_set_hdcc_name(name: &str) -> Option<&str> {
    let digits = name
        .strip_prefix("set_hdcc")
        .or_else(|| name.strip_prefix("set_realcc"))?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(digits)
}

/// 一个 `<control>` opcode 解析成 `set_hdccN` 声明的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SetHdccOutcome {
    /// 名字不是 `set_hdccN` / `set_realccN`。
    NotThisOpcode,
    /// 命中：CC `cc` 的初值是 `initial`（已按 `round(v × 127)` 换算到 7 位域）。
    Value { cc: u16, initial: u8 },
    /// 取值不是 `0..=1` 的浮点数（含非有限值）⇒ 丢弃该条声明 + 告警，不中断解析
    /// （与 `set_ccN` 的 `UnrepresentableValue` 同款口径：规范 Range 外的值不猜）。
    UnrepresentableValue,
}

/// 把一个 `<control>` opcode 解析成 `set_hdccN` / `set_realccN` 声明。
///
/// * **下标**越界（> [`MAX_CC_DEFAULT_INDEX`]）⇒ 明确 `IntegerOutOfRange`（与 `set_ccN` 同款）；
/// * **取值**不是 `0.0..=1.0` 的有限浮点 ⇒ [`SetHdccOutcome::UnrepresentableValue`]（丢弃 + 告警）；
/// * 命中 ⇒ `round(v × 127)`（0.5 → 64，1 → 127，0 → 0）。
///
/// ⚠ 与 `set_ccN` 的优先级在 [`crate::parser`] 的 `finish()` 合并：`set_ccN`
/// 是 SFZ v2 的精确整数声明，**赢**过 ARIA 扩展的浮点换算值。
pub(crate) fn set_hdcc_declaration(
    name: &str,
    value: &str,
    line: usize,
) -> Result<SetHdccOutcome, SfzError> {
    let Some(digits) = parse_set_hdcc_name(name) else {
        return Ok(SetHdccOutcome::NotThisOpcode);
    };
    let cc = match digits.parse::<u16>() {
        Ok(cc) => cc,
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
    let parsed = value.trim().parse::<f32>().ok();
    let Some(v) = parsed.filter(|v| v.is_finite() && (0.0..=1.0).contains(v)) else {
        return Ok(SetHdccOutcome::UnrepresentableValue);
    };
    // 0.0..=1.0 的有限值 ×127 再 round 必落 0..=127 ⇒ `as u8` 不截断。
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let initial = (v * f32::from(MAX_CC_DEFAULT_VALUE)).round() as u8;
    Ok(SetHdccOutcome::Value { cc, initial })
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
    fn a_non_numeric_value_is_dropped_loudly_and_not_an_error() {
        // 无法按数值解读的取值：硬 `Err` 会让整份文件无法加载，
        // 因此这里是「丢弃 + 告警」而不是错误（见模块文档「工程裁决」第 2 条）。
        // 没展开的 `$VAR` 文本也会落到这里（未定义宏按原样保留，见 `Warning::UndefinedMacro`）。
        // 越界小数（`200.5` / `-1.5`）与非有限值同款：与 `set_hdccN` 的口径一致，
        // 规范 Range 外的浮点不猜。
        for text in [
            "abc",
            "",
            "12x",
            "$undefined_macro",
            "１２",
            "200.5",
            "-1.5",
            "NaN",
            "inf",
        ] {
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
    fn a_decimal_value_in_range_rounds_to_the_nearest_seed() {
        // 2026-11-18 修订（模块文档裁决第 2 条修订段）：登记语料里 42 处 `63.5`
        // （10 个文件）是作者写的 7 位域精确中点。丢弃它曾让双极 tune_cc 在
        // 种子 0 处把 3 款乐器整库拉低 700–2400 音分；按 `round` 取整后
        // 种子落在中点 ±半步内（`63.5` → 64，与 `set_hdccN` 的换算同款）。
        for (text, initial) in [
            ("63.5", 64u8),
            ("63.4", 63),
            ("1.5", 2),
            ("0.5", 1),
            ("0.4", 0),
            ("0.0", 0),
            ("126.5", 127),
            ("127.0", 127),
        ] {
            assert_eq!(
                set_cc_declaration("set_cc7", text, 5).expect("decimal in range"),
                SetCcOutcome::Value { cc: 7, initial },
                "set_cc7={text} must seed {initial}"
            );
        }
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

    #[test]
    fn the_set_cc_index_ends_are_real_declarations() {
        // 索引域的两端（`0` 与 `127`）都是**真实声明**，不是「没写索引」的别名。
        for (name, cc) in [("set_cc0", 0u16), ("set_cc127", 127u16)] {
            assert_eq!(
                set_cc_declaration(name, "7", 3).expect("in range"),
                SetCcOutcome::Value { cc, initial: 7 },
                "{name}"
            );
        }
        // 前导零是同一个数值、**不同**的名字。
        assert_eq!(
            set_cc_declaration("set_cc00", "7", 3).expect("in range"),
            SetCcOutcome::Value { cc: 0, initial: 7 }
        );
        // 解析器**不**去重：同一索引声明两次各自成立（「后者胜」是调用方的口径）。
        assert_eq!(
            set_cc_declaration("set_cc0", "1", 1).expect("ok"),
            SetCcOutcome::Value { cc: 0, initial: 1 }
        );
        assert_eq!(
            set_cc_declaration("set_cc0", "2", 2).expect("ok"),
            SetCcOutcome::Value { cc: 0, initial: 2 }
        );
    }

    #[test]
    fn set_hdcc_names_cover_the_aria_spelling_and_its_deprecated_alias() {
        for name in ["set_hdcc10", "set_realcc10", "set_hdcc0", "set_hdcc65535"] {
            assert!(parse_set_hdcc_name(name).is_some(), "{name}");
        }
        for bad in [
            "set_hdcc",
            "set_hdccX",
            "set_hdcc7x",
            "SET_HDCC7",
            "set_cc7",
        ] {
            assert_eq!(parse_set_hdcc_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn set_hdcc_values_are_rounded_from_zero_to_one_to_seven_bits() {
        for (text, initial) in [
            ("0", 0u8),
            ("0.5", 64),
            ("1", 127),
            ("0.08", 10),
            ("0.1", 13),
            (" +0.5 ", 64),
        ] {
            assert_eq!(
                set_hdcc_declaration("set_hdcc10", text, 1).expect("ok"),
                SetHdccOutcome::Value { cc: 10, initial },
                "set_hdcc10={text}"
            );
            assert_eq!(
                set_hdcc_declaration("set_realcc10", text, 1).expect("alias"),
                SetHdccOutcome::Value { cc: 10, initial },
                "set_realcc10={text}"
            );
        }
    }

    #[test]
    fn set_hdcc_rejects_out_of_range_values_and_index_overflow() {
        for bad in ["-0.1", "1.001", "2", "NaN", "inf", "abc"] {
            assert_eq!(
                set_hdcc_declaration("set_hdcc10", bad, 7).expect("no error"),
                SetHdccOutcome::UnrepresentableValue,
                "value {bad}"
            );
        }
        let error = set_hdcc_declaration("set_hdcc65536", "0.5", 9).expect_err("index");
        assert!(
            matches!(error, SfzError::IntegerOutOfRange { .. }),
            "{error:?}"
        );
        assert_eq!(
            set_hdcc_declaration("set_cc7", "0.5", 1).expect("not hdcc"),
            SetHdccOutcome::NotThisOpcode
        );
    }
}
