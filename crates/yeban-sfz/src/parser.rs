//! 零拷贝 SFZ v2 词法 / 结构解析器与 `#include` 沙箱。
//!
//! 规范来源 (Normative):
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` ROAD-M2-005（零拷贝解析器 + 预分配声部池）
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3.2 ARCH-RT-001（RT 路径零分配）
//!
//! 格式事实来源 (Measured, 出处 URL 见 `docs/ledger/sfz-core-notes.md`):
//! - 段头集合与版本：<https://sfzformat.com/headers/>
//! - `#include` 引号语法、相对主 SFZ 路径、嵌套与递归禁令：<https://sfzformat.com/opcodes/include/>
//! - `#define $VAR value`：<https://sfzformat.com/opcodes/define/>
//! - `key` = `lokey` + `hikey` + `pitch_keycenter`，`key=c5` 等价 `key=72`：<https://sfzformat.com/opcodes/key/>
//!
//! # 设计约束
//!
//! 1. **单遍、可重入、无全局状态**：解析状态全部在 [`Parser`] 实例里，没有 `static mut` /
//!    `thread_local` / 全局缓存；同一个缓冲可以被任意多线程同时解析。
//! 2. **零拷贝**：无宏替换时 opcode 名与取值以 `Cow::Borrowed` 借用输入缓冲；
//!    只有真的发生 `$VAR` 文本替换时才产生拥有所有权的 `String`。
//! 3. **显式上限**：见 [`ParseLimits`]。超长行、超多 region、超多 opcode、深 / 多 `#include`、
//!    glob 遍历炸弹全部收敛成 [`SfzError`]，不会 OOM、不会 panic。
//! 4. **不可信输入边界**：解析路径上没有 `unwrap` / `expect` / 直接索引 panic。
//!
//! # `#include` 沙箱（安全红线）
//!
//! `#include` 的目标必须满足全部条件，否则返回 [`SfzError`]：
//! - 以双引号包围（规范要求）；
//! - **相对路径**：绝对路径（`/...`、`C:\...`）一律拒绝；
//! - **不含 `..` 组件**；
//! - 规范化（`fs::canonicalize`）后仍位于基准目录之内 —— 这条同时挡住**符号链接逃逸**；
//! - 扩展名必须是 `.sfz` / `.sfzh`；
//! - 递归展开时不得成环、不得超过深度 / 文件数上限。
//!
//! glob 形式（`*` / `?` / `**`）只从基准目录向下遍历，**不跟随符号链接**，并受
//! [`ParseLimits::max_glob_matches`] / [`ParseLimits::max_glob_scanned`] /
//! [`ParseLimits::max_glob_depth`] 三重上限约束。

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::curve::{Curve, CurvePoint};
use crate::effect::{Effect, EffectBuilder};
use crate::error::SfzError;
use crate::instrument::{Instrument, Region, build_region};

// ---------------------------------------------------------------------------
// 显式上限 (DoS 防线) —— 数值本身也是「口径」，镜像到 docs/ledger/sfz-core-notes.md
// ---------------------------------------------------------------------------

/// 单行最大字节数 (64 KiB)：一行 16 MiB 无换行是典型 DoS 手法。
pub const DEFAULT_MAX_LINE_BYTES: usize = 64 * 1024;
/// 单个 `<region>` 段最大数量 (65,536)。
pub const DEFAULT_MAX_REGIONS: usize = 65_536;
/// 单个作用域内不同 opcode 名最大数量 (4,096)。
pub const DEFAULT_MAX_OPCODES_PER_HEADER: usize = 4_096;
/// `#define` 变量最大数量 (4,096)。
pub const DEFAULT_MAX_DEFINES: usize = 4_096;
/// 单行宏替换次数上限 (64)：阻止 `#define` 文本炸弹。
pub const DEFAULT_MAX_MACRO_EXPANSIONS_PER_LINE: usize = 64;
/// 单个源文件最大字节数 (16 MiB)。
pub const DEFAULT_MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
/// `#include` 最大嵌套深度 (16)。
pub const DEFAULT_MAX_INCLUDE_DEPTH: usize = 16;
/// `#include` 展开后最多几个源文件 (1,024)。
pub const DEFAULT_MAX_INCLUDE_FILES: usize = 1_024;
/// glob 最多匹配几个文件 (4,096)。
pub const DEFAULT_MAX_GLOB_MATCHES: usize = 4_096;
/// glob 目录遍历最大深度 (16)。
pub const DEFAULT_MAX_GLOB_DEPTH: usize = 16;
/// glob 目录遍历最多扫描多少个目录项 (65,536)。
pub const DEFAULT_MAX_GLOB_SCANNED: usize = 65_536;
/// 解析警告最多记录多少条 (256)：超过后静默截断，避免恶意输入撑爆内存。
pub const DEFAULT_MAX_WARNINGS: usize = 256;
/// 最大 `<curve>` 数量 (4096)：每个 `<curve>` 段都是独立作用域，不受
/// [`DEFAULT_MAX_OPCODES_PER_HEADER`] 的跨段约束，因此需要自己的显式上限。
///
/// **needs（集成者）**：本上限由本切片新增，`docs/ledger/sfz-core-notes.md` 第 4 节的
/// 上限表需要补一行（该文件由集成者独占；本工作线的可改写范围只有 `crates/yeban-sfz/**`）。
pub const DEFAULT_MAX_CURVES: usize = 4_096;
/// 最大 `<effect>` 数量 (4096)：与 [`DEFAULT_MAX_CURVES`] 同理 —— 每个 `<effect>` 段都是
/// 独立作用域，段内行数受 [`DEFAULT_MAX_OPCODES_PER_HEADER`] 约束，但**段数**不受它约束。
///
/// **needs（集成者）**：本上限由本切片新增，`docs/ledger/sfz-core-notes.md` 第 4 节的
/// 上限表需要补一行（该文件由集成者独占）。
pub const DEFAULT_MAX_EFFECTS: usize = 4_096;

/// 解析器硬性上限。
///
/// 「上限值写进 notes」是任务要求：默认值镜像到 `docs/ledger/sfz-core-notes.md`，
/// 改动这里必须同步改 notes。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseLimits {
    /// 单行最大字节数，见 [`DEFAULT_MAX_LINE_BYTES`]。
    pub max_line_bytes: usize,
    /// 最大 `<region>` 数量，见 [`DEFAULT_MAX_REGIONS`]。
    pub max_regions: usize,
    /// 最大 `<curve>` 数量，见 [`DEFAULT_MAX_CURVES`]。
    pub max_curves: usize,
    /// 最大 `<effect>` 数量，见 [`DEFAULT_MAX_EFFECTS`]。
    pub max_effects: usize,
    /// 单作用域最大 opcode 数，见 [`DEFAULT_MAX_OPCODES_PER_HEADER`]。
    pub max_opcodes_per_header: usize,
    /// 最大 `#define` 变量数，见 [`DEFAULT_MAX_DEFINES`]。
    pub max_defines: usize,
    /// 单行最大宏替换次数，见 [`DEFAULT_MAX_MACRO_EXPANSIONS_PER_LINE`]。
    pub max_macro_expansions_per_line: usize,
    /// 单文件最大字节数，见 [`DEFAULT_MAX_SOURCE_BYTES`]。
    pub max_source_bytes: usize,
    /// `#include` 最大嵌套深度，见 [`DEFAULT_MAX_INCLUDE_DEPTH`]。
    pub max_include_depth: usize,
    /// `#include` 展开后最大文件数，见 [`DEFAULT_MAX_INCLUDE_FILES`]。
    pub max_include_files: usize,
    /// glob 最大匹配数，见 [`DEFAULT_MAX_GLOB_MATCHES`]。
    pub max_glob_matches: usize,
    /// glob 最大遍历深度，见 [`DEFAULT_MAX_GLOB_DEPTH`]。
    pub max_glob_depth: usize,
    /// glob 最大扫描目录项数，见 [`DEFAULT_MAX_GLOB_SCANNED`]。
    pub max_glob_scanned: usize,
    /// 最大警告条数，见 [`DEFAULT_MAX_WARNINGS`]。
    pub max_warnings: usize,
}

impl Default for ParseLimits {
    fn default() -> Self {
        Self {
            max_line_bytes: DEFAULT_MAX_LINE_BYTES,
            max_regions: DEFAULT_MAX_REGIONS,
            max_curves: DEFAULT_MAX_CURVES,
            max_effects: DEFAULT_MAX_EFFECTS,
            max_opcodes_per_header: DEFAULT_MAX_OPCODES_PER_HEADER,
            max_defines: DEFAULT_MAX_DEFINES,
            max_macro_expansions_per_line: DEFAULT_MAX_MACRO_EXPANSIONS_PER_LINE,
            max_source_bytes: DEFAULT_MAX_SOURCE_BYTES,
            max_include_depth: DEFAULT_MAX_INCLUDE_DEPTH,
            max_include_files: DEFAULT_MAX_INCLUDE_FILES,
            max_glob_matches: DEFAULT_MAX_GLOB_MATCHES,
            max_glob_depth: DEFAULT_MAX_GLOB_DEPTH,
            max_glob_scanned: DEFAULT_MAX_GLOB_SCANNED,
            max_warnings: DEFAULT_MAX_WARNINGS,
        }
    }
}

impl ParseLimits {
    /// 无限制上限，**仅供测试**（故意让某条上限变红时用）。
    ///
    /// 生产代码不应使用：`usize::MAX` 意味着放弃 DoS 防线。
    #[must_use]
    pub fn unlimited() -> Self {
        Self {
            max_line_bytes: usize::MAX,
            max_regions: usize::MAX,
            max_curves: usize::MAX,
            max_effects: usize::MAX,
            max_opcodes_per_header: usize::MAX,
            max_defines: usize::MAX,
            max_macro_expansions_per_line: usize::MAX,
            max_source_bytes: usize::MAX,
            max_include_depth: usize::MAX,
            max_include_files: usize::MAX,
            max_glob_matches: usize::MAX,
            max_glob_depth: usize::MAX,
            max_glob_scanned: usize::MAX,
            max_warnings: usize::MAX,
        }
    }
}

// ---------------------------------------------------------------------------
// 段头
// ---------------------------------------------------------------------------

/// 本解析器识别的 SFZ 段头。
///
/// 其余规范段头（`<midi>` / `<sample>`）
/// 会被识别为「忽略」：产生 [`Warning::IgnoredHeader`] 并跳过其 opcode，
/// 而不是静默当作 region 处理。见 <https://sfzformat.com/headers/>。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Header {
    /// `<control>`（SFZ v2）：文件级设置，本实现只取 `default_path`。
    Control,
    /// `<global>`（SFZ v2）：对所有 region 生效。
    Global,
    /// `<master>`（ARIA）：`global` 与 `group` 之间的中间层。
    ///
    /// 作用域链是 `region → group → master → global`。规范原文：
    /// "The master header is an extra level added inbetween group and global for the
    /// ARIA player. So, the global/group/region or global/master/group/region hierarchy…"
    /// <https://sfzformat.com/headers/>
    Master,
    /// `<group>`（SFZ v1）：对组内 region 生效。
    Group,
    /// `<region>`（SFZ v1）：最基本的可播放单位。
    Region,
    /// `<curve>`（SFZ v2）：定义一条 MIDI CC 调制曲线。
    ///
    /// 它是**定义段**，不属于 `region → group → master → global` 继承链：段内的 opcode
    /// 只进 [`crate::Curve`]（`curve_index` + `v000..v127`），绝不写进任何一个继承作用域，
    /// 也不会清空任何继承作用域。规范原文见 <https://sfzformat.com/headers/curve/>。
    Curve,
    /// `<effect>`（SFZ v2）：定义一条效果器总线声明。
    ///
    /// 与 [`Header::Curve`] 同为**定义段**：段内的 opcode 只进 [`crate::Effect`]
    /// （`bus` / `type` / `param_offset` / `dsp_order` / `effect1..4`），
    /// 既不写进继承链、也不清空继承链。规范原文见
    /// <https://sfzformat.com/headers/effect/>。
    Effect,
}

impl Header {
    /// 从段头名（不含尖括号）解析；大小写不敏感。
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        if name.eq_ignore_ascii_case("control") {
            Some(Self::Control)
        } else if name.eq_ignore_ascii_case("global") {
            Some(Self::Global)
        } else if name.eq_ignore_ascii_case("master") {
            Some(Self::Master)
        } else if name.eq_ignore_ascii_case("group") {
            Some(Self::Group)
        } else if name.eq_ignore_ascii_case("region") {
            Some(Self::Region)
        } else if name.eq_ignore_ascii_case("curve") {
            Some(Self::Curve)
        } else if name.eq_ignore_ascii_case("effect") {
            Some(Self::Effect)
        } else {
            None
        }
    }

    /// 段头的规范名（小写）。
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Global => "global",
            Self::Master => "master",
            Self::Group => "group",
            Self::Region => "region",
            Self::Curve => "curve",
            Self::Effect => "effect",
        }
    }
}

// ---------------------------------------------------------------------------
// 警告（拥有所有权：警告条数被上限约束，且可能来自宏替换产生的临时缓冲）
// ---------------------------------------------------------------------------

/// 非致命的解析异常。**不**中断解析，只是告诉调用方「这段输入被降级处理了」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// 未知 / 未实现的段头被跳过（`<midi>` / `<sample>` …）。
    IgnoredHeader {
        /// 1-based 行号（折算回原文件）。
        line: usize,
        /// 段头名（不含尖括号）。
        name: String,
    },
    /// 未定义的 `$VAR`：原样保留（多数播放器行为），但记录一条告警。
    UndefinedMacro {
        /// 1-based 行号。
        line: usize,
        /// 变量名（不含 `$`）。
        name: String,
    },
    /// `parse_text` 模式遇到 `#include`：纯文本解析器不做 I/O，因此忽略它。
    IncludeIgnored {
        /// 1-based 行号。
        line: usize,
    },
    /// `<region>` 段没有 `sample`（例如纯 keyswitch 映射）：该段被丢弃。
    RegionWithoutSample {
        /// 1-based 行号（`<region>` 段头所在行）。
        line: usize,
    },
    /// 无法识别的 `#` 指令。
    UnknownDirective {
        /// 1-based 行号。
        line: usize,
        /// 指令名。
        text: String,
    },
    /// `#define` 语法错误（缺 `$NAME` 或缺取值）。
    MalformedDefine {
        /// 1-based 行号。
        line: usize,
    },
    /// 警告数达到 [`ParseLimits::max_warnings`] 后的截断标志。
    Truncated,
}

// ---------------------------------------------------------------------------
// 源文件
// ---------------------------------------------------------------------------

/// 一段已展开的 SFZ 源文本。
///
/// [`IncludeResolver`] 会把 `#include` 在**出现位置**原地展开成片段序列，因此
/// `<region>` 可以跨片段延续（与「被包含文件在该点被粘贴进来」的规范语义一致）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SfzSource {
    /// 展示用路径（相对基准目录，`/` 分隔）。
    pub path: String,
    /// 该片段的文本。
    pub text: String,
    /// 该片段第一行在原文件中的 1-based 行号（错误定位用）。
    pub first_line: usize,
}

// ---------------------------------------------------------------------------
// 值类型读取
// ---------------------------------------------------------------------------

/// 一个 `opcode=value` 及其所在行号，提供类型化读取。
///
/// 解析器不猜类型：由调用方按 opcode 语义选择 `as_*`，失败一律返回 [`SfzError`]。
#[derive(Debug, Clone, PartialEq)]
pub struct OpcodeValue<'a> {
    /// opcode 名（如 `lokey`）。
    pub opcode: &'a str,
    /// 原始取值（已去引号、去首尾空白；已做宏替换）。
    pub value: Cow<'a, str>,
    /// 1-based 行号（已折算回原文件行号）。
    pub line: usize,
}

impl<'a> OpcodeValue<'a> {
    /// 构造一个类型化读取上下文。
    #[must_use]
    pub fn new(opcode: &'a str, value: Cow<'a, str>, line: usize) -> Self {
        Self {
            opcode,
            value,
            line,
        }
    }

    /// 作为字符串读取。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// 作为整数读取并做闭区间范围检查（范围取规范 `Range` 列）。
    pub fn as_int(&self, min: i64, max: i64) -> Result<i64, SfzError> {
        let parsed = parse_int(self.as_str()).ok_or_else(|| SfzError::InvalidInteger {
            line: self.line,
            opcode: self.opcode.to_string(),
            value: self.value.to_string(),
        })?;
        if parsed < min || parsed > max {
            return Err(SfzError::IntegerOutOfRange {
                line: self.line,
                opcode: self.opcode.to_string(),
                value: parsed,
                min,
                max,
            });
        }
        Ok(parsed)
    }

    /// 作为有限浮点数读取：语法错误 → [`SfzError::InvalidFloat`]，
    /// `NaN` / `±Inf` → [`SfzError::NonFiniteFloat`]。
    pub fn as_f32(&self) -> Result<f32, SfzError> {
        match self.as_str().trim().parse::<f32>() {
            Err(_) => Err(SfzError::InvalidFloat {
                line: self.line,
                opcode: self.opcode.to_string(),
                value: self.value.to_string(),
            }),
            Ok(value) if !value.is_finite() => Err(SfzError::NonFiniteFloat {
                line: self.line,
                opcode: self.opcode.to_string(),
                value: self.value.to_string(),
            }),
            Ok(value) => Ok(value),
        }
    }

    /// 作为音名（`C4` / `c#4` / `Db4`，IPN 记法 `C4 = 60`）或 MIDI 号读取。
    pub fn as_note(&self, min: i32, max: i32) -> Result<i32, SfzError> {
        let parsed = parse_note(self.as_str()).ok_or_else(|| SfzError::InvalidNote {
            line: self.line,
            opcode: self.opcode.to_string(),
            value: self.value.to_string(),
        })?;
        if parsed < min || parsed > max {
            return Err(SfzError::IntegerOutOfRange {
                line: self.line,
                opcode: self.opcode.to_string(),
                value: i64::from(parsed),
                min: i64::from(min),
                max: i64::from(max),
            });
        }
        Ok(parsed)
    }

    /// 作为枚举（字符串白名单）读取；大小写不敏感。
    pub fn as_option<T: Copy>(
        &self,
        options: &[(&str, T)],
        allowed: &'static str,
    ) -> Result<T, SfzError> {
        let needle = self.as_str();
        options
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(needle))
            .map(|(_, value)| *value)
            .ok_or_else(|| SfzError::InvalidOption {
                line: self.line,
                opcode: self.opcode.to_string(),
                value: self.value.to_string(),
                allowed,
            })
    }
}

/// 解析十进制整数（允许前导 `+` / `-`）。溢出返回 `None`，不 panic。
#[must_use]
pub fn parse_int(raw: &str) -> Option<i64> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    text.parse::<i64>().ok()
}

/// 解析浮点数并拒绝 `NaN` / `±Inf`（它们无法确定性序列化）。
#[must_use]
pub fn parse_f32(raw: &str) -> Option<f32> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    match text.parse::<f32>() {
        Ok(value) if value.is_finite() => Some(value),
        _ => None,
    }
}

/// 解析音名或 MIDI 号。
///
/// 记法依据 sfzformat 的 `key` 页：`key=c5` 与 `key=72` 等价，因此 `C4 = 60`
/// （IPN / 科学音高记法，中央 C 为 C4）。大小写不敏感；升号接受 `#` / `s`，
/// 降号接受 `b` / `f`；省略八度时按八度 4 处理。
///
/// 注意：AKAI / 部分老音色库使用 `C3 = 60`（Yamaha 记法），本实现**不**采用；
/// 该歧义记录在 `docs/ledger/sfz-core-notes.md` 的「需要人类裁决」一节。
#[must_use]
pub fn parse_note(raw: &str) -> Option<i32> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    // 纯数字（含负数，`key=-1` 是规范里的「不被音符触发」哨兵）。
    if text.starts_with(|c: char| c == '-' || c == '+' || c.is_ascii_digit()) {
        return text.parse::<i32>().ok();
    }

    let bytes = text.as_bytes();
    let base = match bytes[0].to_ascii_uppercase() {
        b'C' => 0,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => return None,
    };

    let mut i = 1usize;
    let mut semitone = base;
    if let Some(&accidental) = bytes.get(i) {
        match accidental.to_ascii_lowercase() {
            b'#' | b's' => {
                semitone += 1;
                i += 1;
            }
            b'b' | b'f' => {
                semitone -= 1;
                i += 1;
            }
            _ => {}
        }
    }

    let octave: i32 = if i >= bytes.len() {
        4
    } else {
        text.get(i..)?.trim().parse::<i32>().ok()?
    };
    Some((octave + 1) * 12 + semitone)
}

// ---------------------------------------------------------------------------
// 词法工具
// ---------------------------------------------------------------------------

/// 去掉整行注释（`//`），但**不**删除双引号内的 `//`（保护 `#include "a//b.sfz"`）。
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_quotes = false;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => in_quotes = !in_quotes,
            b'/' if !in_quotes && bytes.get(i + 1) == Some(&b'/') => return &line[..i],
            _ => {}
        }
        i += 1;
    }
    line
}

/// 去掉取值两端成对的单层双引号（`sample="x.wav"`）。
fn unquote(value: &str) -> &str {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    }
}

/// 判断一行是否 `#` 指令。
fn is_directive(line: &str) -> bool {
    line.starts_with('#')
}

/// 把一个 `#define` / `#include` 行拆成 `(指令名, 参数原文)`。
fn split_directive(line: &str) -> (&str, &str) {
    let body = line.trim_start_matches('#').trim_start();
    match body.find(char::is_whitespace) {
        Some(index) => (&body[..index], body[index..].trim_start()),
        None => (body, ""),
    }
}

/// 找到「下一个 opcode 名」的起点：空白之后紧跟标识符再跟 `=`。
///
/// 这条规则让 `sample=My Drums/kick.wav key=36` 切成两个 opcode，
/// 同时保留取值里的空格（文件名含空格）。
fn opcode_boundary(value: &str) -> usize {
    let bytes = value.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if !bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let mut k = i;
        while k < bytes.len() && bytes[k].is_ascii_whitespace() {
            k += 1;
        }
        if k >= bytes.len() {
            return i;
        }
        let start = k;
        while k < bytes.len()
            && (bytes[k].is_ascii_alphanumeric() || bytes[k] == b'_' || bytes[k] == b'$')
        {
            k += 1;
        }
        if k > start && k < bytes.len() && bytes[k] == b'=' {
            return i;
        }
        i += 1;
    }
    bytes.len()
}

/// 一个 `opcode=value` 迭代器（零分配）。
///
/// 找不到 `=` 的裸词会被跳过；空名会被跳过。取值到行尾或到下一个 opcode 名为止。
struct OpcodeIter<'a> {
    rest: &'a str,
}

impl<'a> Iterator for OpcodeIter<'a> {
    type Item = (&'a str, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let text = self.rest.trim_start();
            if text.is_empty() {
                self.rest = "";
                return None;
            }
            let name_end = text
                .find(|c: char| c == '=' || c.is_whitespace())
                .unwrap_or(text.len());
            if text.as_bytes().get(name_end) != Some(&b'=') {
                self.rest = &text[name_end..];
                continue;
            }
            let name = text[..name_end].trim();
            let after_eq = &text[name_end + 1..];
            let end = opcode_boundary(after_eq);
            let value = unquote(&after_eq[..end]);
            self.rest = &after_eq[end..];
            if name.is_empty() {
                continue;
            }
            return Some((name, value));
        }
    }
}

/// 宏表：`$VAR` 名（不含 `$`）到替换文本。
type MacroTable<'a> = BTreeMap<&'a str, Cow<'a, str>>;

/// 单遍、**非递归**的 `$VAR` 文本替换。
///
/// 只做一层替换：替换进去的文本不会再被展开，因此 `#define $A $A` 之类的输入
/// 不会指数爆炸。链式宏请在 `#define` 处按定义顺序展开（本实现在定义时展开一层）。
/// 未定义的 `$VAR` 原样保留并触发一次回调。
fn expand_macros<'a>(
    line: &'a str,
    line_no: usize,
    limits: ParseLimits,
    mut lookup: impl FnMut(&str) -> Option<String>,
    mut on_undefined: impl FnMut(&'a str),
) -> Result<Cow<'a, str>, SfzError> {
    let bytes = line.as_bytes();
    let mut output: Option<String> = None;
    let mut last = 0usize;
    let mut substitutions = 0usize;
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] != b'$' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
            j += 1;
        }
        if j == i + 1 {
            i += 1;
            continue;
        }
        let name = &line[i + 1..j];
        match lookup(name) {
            Some(replacement) => {
                substitutions += 1;
                if substitutions > limits.max_macro_expansions_per_line {
                    return Err(SfzError::MacroExpansionExceeded {
                        line: line_no,
                        name: name.to_string(),
                        limit: limits.max_macro_expansions_per_line,
                    });
                }
                let buffer = output.get_or_insert_with(|| String::with_capacity(line.len()));
                buffer.push_str(&line[last..i]);
                buffer.push_str(&replacement);
                last = j;
            }
            None => on_undefined(name),
        }
        i = j;
    }

    match output {
        None => Ok(Cow::Borrowed(line)),
        Some(mut buffer) => {
            buffer.push_str(&line[last..]);
            if buffer.len() > limits.max_line_bytes {
                return Err(SfzError::MacroExpansionTooLong {
                    line: line_no,
                    limit: limits.max_line_bytes,
                });
            }
            Ok(Cow::Owned(buffer))
        }
    }
}

// ---------------------------------------------------------------------------
// 解析器
// ---------------------------------------------------------------------------

/// 段头作用域内的 opcode 表。
///
/// key / value 都用 `Cow`：无宏替换时全部是 `Borrowed`（零拷贝），
/// 宏替换后那一行降级为 `Owned`。
pub(crate) type OpcodeMap<'a> = BTreeMap<Cow<'a, str>, Cow<'a, str>>;

struct Parser<'a> {
    limits: ParseLimits,
    macros: MacroTable<'a>,
    control: OpcodeMap<'a>,
    global: OpcodeMap<'a>,
    master: OpcodeMap<'a>,
    group: OpcodeMap<'a>,
    region: OpcodeMap<'a>,
    /// 当前 opcode 应该写进哪个作用域。
    scope: Scope,
    regions: Vec<Region<'a>>,
    /// 已归约的 `<curve>` 段（保持文件出现顺序，确定性）。
    curves: Vec<Curve>,
    /// 当前 `<curve>` 段头所在行（归约时用于错误定位）。
    curve_line: usize,
    /// 当前 `<curve>` 的 `curve_index`（同一段里重复给出时后者胜，与其它 opcode 的
    /// 「`BTreeMap` 后者覆盖」口径一致）。
    curve_index: Option<u8>,
    /// 当前 `<curve>` 的点（同一段里重复给出同一个 `vNNN` 时后者胜）。
    curve_points: Vec<CurvePoint>,
    /// 当前 `<curve>` 段里已消费的 opcode 数（DoS 上限，见 [`ParseLimits::max_opcodes_per_header`]）。
    curve_opcodes: usize,
    /// 已归约的 `<effect>` 段（保持文件出现顺序，确定性）。
    effects: Vec<Effect<'a>>,
    /// 当前 `<effect>` 段头所在行（归约时用于错误定位）。
    effect_line: usize,
    /// 当前 `<effect>` 段累积到的规范 opcode。
    effect: EffectBuilder<'a>,
    /// 当前 `<effect>` 段里已消费的 opcode 数（DoS 上限，与 `<curve>` 同口径）。
    effect_opcodes: usize,
    /// 当前 `<region>` 段头所在行（归约时用于错误定位）。
    region_line: usize,
    warnings: Vec<Warning>,
    warnings_truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Control,
    Global,
    Master,
    Group,
    Region,
    /// `<curve>`：opcode 进 [`Parser::curve_points`] / `curve_index`，不进继承链。
    Curve,
    /// `<effect>`：opcode 进 [`Parser::effect`]，不进继承链（同为定义段）。
    Effect,
    /// 空行 / 注释 / 未知段头之后：丢弃 opcode，但 `#define` 仍然生效。
    Ignored,
}

impl<'a> Parser<'a> {
    fn new(limits: ParseLimits) -> Self {
        Self {
            limits,
            macros: MacroTable::new(),
            control: OpcodeMap::new(),
            global: OpcodeMap::new(),
            master: OpcodeMap::new(),
            group: OpcodeMap::new(),
            region: OpcodeMap::new(),
            // 规范要求显式段头；没有段头的裸 opcode 按 `<global>` 处理（宽松但无损）。
            scope: Scope::Global,
            regions: Vec::new(),
            curves: Vec::new(),
            curve_line: 1,
            curve_index: None,
            curve_points: Vec::new(),
            curve_opcodes: 0,
            effects: Vec::new(),
            effect_line: 1,
            effect: EffectBuilder::new(),
            effect_opcodes: 0,
            region_line: 1,
            warnings: Vec::new(),
            warnings_truncated: false,
        }
    }

    fn warn(&mut self, warning: Warning) {
        if self.warnings.len() < self.limits.max_warnings {
            self.warnings.push(warning);
        } else {
            self.warnings_truncated = true;
        }
    }

    fn map_for(&mut self, scope: Scope) -> Option<(&mut OpcodeMap<'a>, &'static str)> {
        match scope {
            Scope::Control => Some((&mut self.control, "control")),
            Scope::Global => Some((&mut self.global, "global")),
            Scope::Master => Some((&mut self.master, "master")),
            Scope::Group => Some((&mut self.group, "group")),
            Scope::Region => Some((&mut self.region, "region")),
            Scope::Curve | Scope::Effect | Scope::Ignored => None,
        }
    }

    fn insert_opcode(&mut self, name: Cow<'a, str>, value: Cow<'a, str>) -> Result<(), SfzError> {
        if self.scope == Scope::Curve {
            return self.insert_curve_opcode(name, value);
        }
        if self.scope == Scope::Effect {
            return self.insert_effect_opcode(name, value);
        }
        let limit = self.limits.max_opcodes_per_header;
        let scope = self.scope;
        let Some((map, scope_name)) = self.map_for(scope) else {
            return Ok(());
        };
        if !map.contains_key(name.as_ref()) && map.len() >= limit {
            return Err(SfzError::TooManyOpcodes {
                scope: scope_name,
                limit,
            });
        }
        map.insert(name, value);
        Ok(())
    }

    /// 把 `<curve>` 段里的一个 opcode 写进当前曲线。
    ///
    /// 只认 `curve_index` 与 `v000..v127`（名字大小写敏感，与其它 opcode 的读取口径一致）；
    /// 其余 opcode 与全文件口径一致地被忽略（未知 opcode 不报错）。
    ///
    /// [`ParseLimits::max_opcodes_per_header`] 在这里按**行数**计（不是「不同名字数」，
    /// 因为曲线不进 `BTreeMap`）：合法曲线最多 129 个 opcode，所以这个更严的口径
    /// 不会误伤真实文件，却仍然挡住「无限 opcode 行」的 DoS。
    ///
    /// 错误定位用**段头行**（与 [`build_region`] 用 `<region>` 段头行的口径一致）。
    fn insert_curve_opcode(
        &mut self,
        name: Cow<'a, str>,
        value: Cow<'a, str>,
    ) -> Result<(), SfzError> {
        let limit = self.limits.max_opcodes_per_header;
        self.curve_opcodes += 1;
        if self.curve_opcodes > limit {
            return Err(SfzError::TooManyOpcodes {
                scope: "curve",
                limit,
            });
        }
        let text: &str = name.as_ref();
        let line = self.curve_line;
        if text == "curve_index" {
            let parsed = OpcodeValue::new("curve_index", value, line)
                .as_int(0, i64::from(crate::curve::MAX_CURVE_INDEX))?;
            let index = parsed as u8;
            if index <= crate::curve::MAX_BUILT_IN_CURVE_INDEX {
                // 规范原文："These cannot be overwritten. Use `curve_index` numbers of 7
                // and above for custom curves." ⇒ 明确 Err，不静默丢弃也不静默覆盖内建曲线。
                return Err(SfzError::ReservedCurveIndex { line, index });
            }
            self.curve_index = Some(index);
            return Ok(());
        }
        let Some(digits) = text
            .strip_prefix('v')
            .filter(|digits| digits.len() == 3 && digits.bytes().all(|byte| byte.is_ascii_digit()))
        else {
            // 不是 `vNNN`（例如 `v5` / `v0000` / 任何未知 opcode）：与全文件口径一致地忽略。
            return Ok(());
        };
        let at = digits
            .bytes()
            .fold(0u32, |acc, byte| acc * 10 + u32::from(byte - b'0'));
        if at > 127 {
            return Err(SfzError::IntegerOutOfRange {
                line,
                opcode: text.to_string(),
                value: i64::from(at),
                min: 0,
                max: 127,
            });
        }
        let at = at as u8;
        let value = OpcodeValue::new(text, value, line).as_f32()?;
        match self.curve_points.iter_mut().find(|point| point.at == at) {
            // 同一段里重复给出同一个点：后者胜（与 `BTreeMap` 作用域的覆盖口径一致）。
            Some(existing) => existing.value = value,
            None => self.curve_points.push(CurvePoint { at, value }),
        }
        Ok(())
    }

    /// 把 `<effect>` 段里的一个 opcode 写进当前效果器声明。
    ///
    /// 只认规范表格里的 `bus` / `type` / `param_offset` / `dsp_order` / `effect1`..=`effect4`
    /// （名字大小写敏感，与其它 opcode 的读取口径一致）；其余 opcode 与全文件口径一致地被
    /// 忽略（未知 opcode 不报错）。
    ///
    /// [`ParseLimits::max_opcodes_per_header`] 与 `<curve>` 同口径按**行数**计
    /// （效果器声明不进 `BTreeMap`）。错误定位用**段头行**。
    fn insert_effect_opcode(
        &mut self,
        name: Cow<'a, str>,
        value: Cow<'a, str>,
    ) -> Result<(), SfzError> {
        let limit = self.limits.max_opcodes_per_header;
        self.effect_opcodes += 1;
        if self.effect_opcodes > limit {
            return Err(SfzError::TooManyOpcodes {
                scope: "effect",
                limit,
            });
        }
        let text: &str = name.as_ref();
        let line = self.effect_line;
        match text {
            "bus" => self.effect.set_bus(value.as_ref()),
            "type" => self.effect.set_type(value),
            "param_offset" => {
                // 规范未给范围（<https://sfzformat.com/opcodes/param_offset/>）；
                // 「非负整数」的口径与 `offset` / `loop_start` 一致。
                let parsed =
                    OpcodeValue::new("param_offset", value, line).as_int(0, i64::from(u32::MAX))?;
                self.effect.set_param_offset(parsed);
            }
            "dsp_order" => {
                // 规范范围 `0 to 14`：越界是明确 Err，不静默钳位。
                let parsed = OpcodeValue::new("dsp_order", value, line)
                    .as_int(0, i64::from(crate::effect::MAX_DSP_ORDER))?;
                self.effect.set_dsp_order(parsed as u8);
            }
            _ => {
                let Some(index) = send_index(text) else {
                    // 非规范 opcode（例如 Rapture 的厂商私有 opcode）：与全文件口径一致地忽略。
                    return Ok(());
                };
                let parsed = OpcodeValue::new(text, value, line).as_f32()?;
                self.effect.set_send(index, parsed);
            }
        }
        Ok(())
    }

    /// 结束当前 `<effect>`：归约成 [`Effect`] 并压栈。
    ///
    /// 规则（规范出处 <https://sfzformat.com/headers/effect/>）：
    /// - **空段**（没有任何规范 opcode）不产生效果器、也不报错：没有数据可丢；
    /// - 段数超过 [`ParseLimits::max_effects`] ⇒ [`SfzError::TooManyEffects`]。
    ///
    /// 与 `<curve>` 不同，`<effect>` 段没有「重复定义」这一说：同一条总线上可以串多级效果
    /// （`dsp_order` 就是排序用的），因此**不**做去重。
    fn finalize_effect(&mut self) -> Result<(), SfzError> {
        if self.scope != Scope::Effect {
            return Ok(());
        }
        self.effect_opcodes = 0;
        let builder = core::mem::replace(&mut self.effect, EffectBuilder::new());
        if !builder.has_data() {
            return Ok(());
        }
        if self.effects.len() >= self.limits.max_effects {
            return Err(SfzError::TooManyEffects {
                limit: self.limits.max_effects,
            });
        }
        self.effects.push(builder.build());
        Ok(())
    }

    /// 结束当前 `<region>`：归约成 [`Region`] 并压栈。
    fn finalize_region(&mut self) -> Result<(), SfzError> {
        if self.scope != Scope::Region {
            return Ok(());
        }
        let line = self.region_line;
        let default_path = self.control.get("default_path").cloned();
        let built = build_region(
            &self.region,
            &self.group,
            &self.master,
            &self.global,
            default_path,
            line,
        )?;
        match built {
            Some(region) => self.regions.push(region),
            None => self.warn(Warning::RegionWithoutSample { line }),
        }
        self.region.clear();
        Ok(())
    }

    /// 结束当前 `<curve>`：归约成 [`Curve`] 并压栈。
    ///
    /// 规则（规范出处 <https://sfzformat.com/headers/curve/>）：
    /// - 规范缺省 `v000=0` / `v127=1` 在没被显式给出时补上，因此每条曲线都覆盖 `0..=127`
    ///   （`Curve::value_at` 永远不需要外推）；
    /// - **空段**（既无 `curve_index` 也无任何点）不产生曲线、也不报错：没有数据可丢；
    /// - 有数据却没有 `curve_index` ⇒ [`SfzError::CurveWithoutIndex`]（明确 Err，不静默丢弃）；
    /// - 同一个 `curve_index` 定义两次 ⇒ [`SfzError::DuplicateCurveIndex`]（含义歧义，不猜）；
    /// - 曲线条数超过 [`ParseLimits::max_curves`] ⇒ [`SfzError::TooManyCurves`]。
    fn finalize_curve(&mut self) -> Result<(), SfzError> {
        if self.scope != Scope::Curve {
            return Ok(());
        }
        let line = self.curve_line;
        let index = self.curve_index.take();
        let mut points = core::mem::take(&mut self.curve_points);
        self.curve_opcodes = 0;
        if index.is_none() && points.is_empty() {
            return Ok(());
        }
        let Some(index) = index else {
            return Err(SfzError::CurveWithoutIndex { line });
        };
        if self.curves.iter().any(|curve| curve.index() == index) {
            return Err(SfzError::DuplicateCurveIndex { line, index });
        }
        if self.curves.len() >= self.limits.max_curves {
            return Err(SfzError::TooManyCurves {
                limit: self.limits.max_curves,
            });
        }
        if !points.iter().any(|point| point.at == 0) {
            points.push(CurvePoint { at: 0, value: 0.0 });
        }
        if !points.iter().any(|point| point.at == 127) {
            points.push(CurvePoint {
                at: 127,
                value: 1.0,
            });
        }
        points.sort_by_key(|point| point.at);
        self.curves.push(Curve::from_points(index, points));
        Ok(())
    }

    fn run(
        &mut self,
        text: &'a str,
        first_line: usize,
        include_is_unresolved: bool,
    ) -> Result<(), SfzError> {
        let limits = self.limits;
        for (index, raw_line) in text.split_inclusive('\n').enumerate() {
            let line_no = first_line + index;
            if raw_line.len() > limits.max_line_bytes {
                return Err(SfzError::LineTooLong {
                    line: line_no,
                    len: raw_line.len(),
                    limit: limits.max_line_bytes,
                });
            }
            let line = strip_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }

            // ---- `#` 指令（在 Ignored 作用域里也必须生效） ----
            if is_directive(line) {
                let (directive, argument) = split_directive(line);
                if directive.eq_ignore_ascii_case("define") {
                    self.handle_define(argument, line_no)?;
                } else if directive.eq_ignore_ascii_case("include") {
                    if include_is_unresolved {
                        // 纯文本模式不做 I/O：明确告警，绝不静默半解析。
                        self.warn(Warning::IncludeIgnored { line: line_no });
                    }
                } else {
                    self.warn(Warning::UnknownDirective {
                        line: line_no,
                        text: directive.to_string(),
                    });
                }
                continue;
            }

            // ---- 宏替换（对整行做：`$VAR` 也可能出现在 opcode 名里） ----
            let expanded = {
                let macros = &self.macros;
                let mut undefined: Vec<&'a str> = Vec::new();
                let result = expand_macros(
                    line,
                    line_no,
                    limits,
                    |name| macros.get(name).map(|value| value.as_ref().to_string()),
                    |name| undefined.push(name),
                );
                for name in undefined {
                    self.warn(Warning::UndefinedMacro {
                        line: line_no,
                        name: name.to_string(),
                    });
                }
                result?
            };

            // 借用路径保持零拷贝；只有发生宏替换时才走拥有路径。
            match expanded {
                Cow::Borrowed(borrowed) => {
                    self.process_line(borrowed, line_no, Cow::Borrowed)?;
                }
                Cow::Owned(owned) => {
                    self.process_line(&owned, line_no, |slice| Cow::Owned(slice.to_string()))?;
                }
            }
        }
        Ok(())
    }

    /// 处理一行（可能以段头开头）：切换作用域并写入所有 `opcode=value`。
    ///
    /// `convert` 负责把借用自本行的切片变成作用域表要求的 `Cow<'a, str>`：
    /// 借用路径直接 `Cow::Borrowed`（零拷贝），拥有路径分配一个 `String`。
    fn process_line<'b>(
        &mut self,
        line: &'b str,
        line_no: usize,
        convert: impl Fn(&'b str) -> Cow<'a, str>,
    ) -> Result<(), SfzError> {
        let mut rest = line.trim_start();
        if let Some(after_bracket) = rest.strip_prefix('<') {
            let Some(close) = after_bracket.find('>') else {
                return Err(SfzError::UnterminatedHeader {
                    line: line_no,
                    text: truncate_for_error(rest),
                });
            };
            let name = after_bracket[..close].trim();
            if name.is_empty() {
                return Err(SfzError::EmptyHeaderName { line: line_no });
            }
            rest = &after_bracket[close + 1..];
            self.switch_scope(name, line_no)?;
        }

        for (name, value) in (OpcodeIter { rest }) {
            self.insert_opcode(convert(name), convert(value))?;
        }
        Ok(())
    }

    /// 处理 `#define $NAME value`（取值在定义时展开一层）。
    fn handle_define(&mut self, argument: &'a str, line_no: usize) -> Result<(), SfzError> {
        let Some(rest) = argument.trim_start().strip_prefix('$') else {
            self.warn(Warning::MalformedDefine { line: line_no });
            return Ok(());
        };
        let name_end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        let name = &rest[..name_end];
        let value_text = rest[name_end..].trim();
        if name.is_empty() || value_text.is_empty() {
            self.warn(Warning::MalformedDefine { line: line_no });
            return Ok(());
        }
        if !self.macros.contains_key(name) && self.macros.len() >= self.limits.max_defines {
            return Err(SfzError::TooManyDefines {
                limit: self.limits.max_defines,
            });
        }
        let limits = self.limits;
        let macros = &self.macros;
        let expanded = expand_macros(
            value_text,
            line_no,
            limits,
            |needle| macros.get(needle).map(|value| value.as_ref().to_string()),
            |_| {},
        )?;
        self.macros.insert(name, expanded);
        Ok(())
    }

    /// 切换当前作用域。`name` 可能来自宏替换后的临时缓冲，因此只读不存。
    fn switch_scope(&mut self, name: &str, line_no: usize) -> Result<(), SfzError> {
        // 关键顺序：**先**离开 region 作用域（归约尚未完成的 `<region>`），再切换。
        // 否则后续的 `<group>` / `<global>` / `<control>` 会清空继承表，
        // 让尚未归约的 region 要么丢掉继承值、要么被整段丢弃（回归判据：
        // `region_inherits_group_values_even_when_a_later_group_header_intervenes`）。
        self.finalize_region()?;
        // `<curve>` 是定义段：先归约上一条曲线，再开新段。它**不**动继承表。
        self.finalize_curve()?;
        // `<effect>` 同为定义段：口径与 `<curve>` 一致。
        self.finalize_effect()?;
        match Header::from_name(name) {
            Some(Header::Control) => {
                // ARIA 语义：新的 `<control>` 会重置 `default_path`。
                self.control.clear();
                self.scope = Scope::Control;
            }
            Some(Header::Global) => {
                self.global.clear();
                self.master.clear();
                self.group.clear();
                self.scope = Scope::Global;
            }
            Some(Header::Master) => {
                // 工程裁决（见 notes「需要人类裁决」）：新 `<master>` 同时清空 `<group>`，
                // 与上面新 `<global>` 清空 `<master>`+`<group>` 的做法一致。
                // 理由：`<master>` 是 `global` 与 `group` 之间的层，若不清空，上一个
                // master 段的 group 取值会泄漏到下一个 master 段（规范未明文规定，
                // 但泄漏会让 `sw_last` 之类的分组映射串味）。
                self.master.clear();
                self.group.clear();
                self.scope = Scope::Master;
            }
            Some(Header::Group) => {
                self.group.clear();
                self.scope = Scope::Group;
            }
            Some(Header::Region) => {
                if self.regions.len() >= self.limits.max_regions {
                    return Err(SfzError::TooManyRegions {
                        limit: self.limits.max_regions,
                    });
                }
                self.region.clear();
                self.region_line = line_no;
                self.scope = Scope::Region;
            }
            Some(Header::Curve) => {
                // 定义段：只重置曲线寄存器，**不**动 `global` / `master` / `group`。
                self.curve_line = line_no;
                self.curve_index = None;
                self.curve_points.clear();
                self.curve_opcodes = 0;
                self.scope = Scope::Curve;
            }
            Some(Header::Effect) => {
                // 定义段：只重置效果器寄存器，**不**动 `global` / `master` / `group`
                // （与 `<curve>` 同一条口径）。
                self.effect_line = line_no;
                self.effect = EffectBuilder::new();
                self.effect_opcodes = 0;
                self.scope = Scope::Effect;
            }
            None => {
                self.warn(Warning::IgnoredHeader {
                    line: line_no,
                    name: name.to_string(),
                });
                self.scope = Scope::Ignored;
            }
        }
        Ok(())
    }

    fn finish(self) -> Instrument<'a> {
        Instrument::new(self.regions, self.curves, self.effects, self.warnings)
    }
}

/// 识别 `effect1`..=`effect4`，返回 0-based 发送量下标；其它名字返回 `None`。
///
/// 只接受整整一位数字（`effect0` / `effect5` / `effect10` / `effect` 都不是规范 opcode）。
fn send_index(name: &str) -> Option<usize> {
    let digits = name.strip_prefix("effect")?;
    let [digit] = digits.as_bytes() else {
        return None;
    };
    match *digit {
        b'1'..=b'4' => Some(usize::from(*digit - b'1')),
        _ => None,
    }
}

fn truncate_for_error(text: &str) -> String {
    const MAX: usize = 96;
    if text.len() <= MAX {
        return text.to_string();
    }
    let mut end = MAX;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// 把一段文本解析成 [`Instrument`]（**不**解析 `#include`，遇到时产生
/// [`Warning::IncludeIgnored`]）。
///
/// 纯函数：无 I/O、无全局状态、可重入，也是 fuzz 目标的主入口。
pub fn parse_text<'a>(text: &'a str, limits: &ParseLimits) -> Result<Instrument<'a>, SfzError> {
    let mut parser = Parser::new(*limits);
    parser.run(text, 1, true)?;
    parser.finalize_region()?;
    parser.finalize_curve()?;
    parser.finalize_effect()?;
    Ok(parser.finish())
}

/// 把 [`IncludeResolver`] 展开好的片段序列解析成 [`Instrument`]。
///
/// 片段按粘贴顺序遍历，解析状态跨片段延续，因此 `<region>` 可以跨 `#include` 边界。
pub fn parse_sources<'a>(
    sources: &'a [SfzSource],
    limits: &ParseLimits,
) -> Result<Instrument<'a>, SfzError> {
    let mut parser = Parser::new(*limits);
    for source in sources {
        parser.run(&source.text, source.first_line, false)?;
    }
    parser.finalize_region()?;
    parser.finalize_curve()?;
    parser.finalize_effect()?;
    Ok(parser.finish())
}

// ---------------------------------------------------------------------------
// #include 沙箱与 glob 展开
// ---------------------------------------------------------------------------

/// 在**基准目录沙箱**内展开 `#include`。
///
/// 见模块头部的安全规则。`new` 会把基准目录规范化（解析符号链接），后续每次 include
/// 都会把候选路径规范化并断言 `starts_with(base)`，因此符号链接无法逃逸。
#[derive(Debug, Clone)]
pub struct IncludeResolver {
    base: PathBuf,
    limits: ParseLimits,
}

impl IncludeResolver {
    /// 以 `base_dir` 为沙箱根构造解析器。`base_dir` 必须存在且是目录。
    pub fn new(base_dir: impl AsRef<Path>, limits: ParseLimits) -> Result<Self, SfzError> {
        let requested = base_dir.as_ref();
        let base = fs::canonicalize(requested).map_err(|error| SfzError::Io {
            path: requested.display().to_string(),
            detail: error.to_string(),
        })?;
        if !base.is_dir() {
            return Err(SfzError::Io {
                path: base.display().to_string(),
                detail: "include base is not a directory".to_string(),
            });
        }
        Ok(Self { base, limits })
    }

    /// 沙箱基准目录的规范绝对路径。
    #[must_use]
    pub fn base_dir(&self) -> &Path {
        &self.base
    }

    /// 从 `entry`（相对基准目录）开始，按粘贴顺序展开全部 `#include`。
    ///
    /// 返回值交给 [`parse_sources`] 得到 [`Instrument`]：
    ///
    /// ```no_run
    /// use yeban_sfz::{IncludeResolver, ParseLimits, parse_sources};
    ///
    /// let limits = ParseLimits::default();
    /// let resolver = IncludeResolver::new("/path/to/instrument", limits)?;
    /// let sources = resolver.resolve("instrument.sfz")?;
    /// let instrument = parse_sources(&sources, &limits)?;
    /// assert!(instrument.len() >= 1);
    /// # Ok::<(), yeban_sfz::SfzError>(())
    /// ```
    pub fn resolve(&self, entry: &str) -> Result<Vec<SfzSource>, SfzError> {
        let mut state = ResolveState {
            sources: Vec::new(),
            macros: BTreeMap::new(),
            stack: Vec::new(),
            files: 0,
        };
        let relative = self.check_relative(entry, 0)?;
        let canonical = self.canonical_include(&relative, 0)?;
        self.expand_file(&canonical, &relative, 0, &mut state)?;
        Ok(state.sources)
    }

    /// 词法检查：拒绝绝对路径、`..`、NUL、非法扩展名。
    fn check_relative(&self, raw: &str, line: usize) -> Result<String, SfzError> {
        if raw.is_empty() {
            return Err(SfzError::IncludeEmptyPath { line });
        }
        if raw.contains('\0') {
            return Err(SfzError::IncludeInvalidPath {
                line,
                path: truncate_for_error(raw),
            });
        }
        // Windows 盘符绝对路径（`C:\...`）。
        let bytes = raw.as_bytes();
        if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            return Err(SfzError::IncludeAbsolutePath {
                line,
                path: truncate_for_error(raw),
            });
        }
        let normalized = raw.replace('\\', "/");
        if normalized.starts_with('/') {
            return Err(SfzError::IncludeAbsolutePath {
                line,
                path: truncate_for_error(raw),
            });
        }
        for component in normalized.split('/') {
            if component == ".." {
                return Err(SfzError::IncludeEscape {
                    line,
                    path: truncate_for_error(raw),
                });
            }
        }
        if !has_sfz_extension(&normalized) {
            return Err(SfzError::IncludeUnsupportedExtension {
                line,
                path: truncate_for_error(raw),
            });
        }
        Ok(normalized)
    }

    /// 把相对路径规范化并断言仍在沙箱内。
    fn canonical_include(&self, relative: &str, line: usize) -> Result<PathBuf, SfzError> {
        let candidate = self.base.join(relative);
        let canonical = fs::canonicalize(&candidate).map_err(|_| SfzError::IncludeNotFound {
            line,
            path: relative.to_string(),
        })?;
        if !canonical.starts_with(&self.base) {
            return Err(SfzError::IncludeEscape {
                line,
                path: relative.to_string(),
            });
        }
        if !canonical.is_file() {
            return Err(SfzError::IncludeNotAFile {
                line,
                path: relative.to_string(),
            });
        }
        Ok(canonical)
    }

    /// 读取并扫描一个文件；`#include` 在原地递归展开。
    fn expand_file(
        &self,
        canonical: &Path,
        display: &str,
        depth: usize,
        state: &mut ResolveState,
    ) -> Result<(), SfzError> {
        if depth >= self.limits.max_include_depth {
            return Err(SfzError::IncludeDepthExceeded {
                limit: self.limits.max_include_depth,
            });
        }
        if state.stack.iter().any(|item| item == canonical) {
            return Err(SfzError::IncludeCycle {
                path: display.to_string(),
            });
        }
        state.files += 1;
        if state.files > self.limits.max_include_files {
            return Err(SfzError::IncludeCountExceeded {
                limit: self.limits.max_include_files,
            });
        }

        let bytes = fs::read(canonical).map_err(|error| SfzError::Io {
            path: display.to_string(),
            detail: error.to_string(),
        })?;
        if bytes.len() > self.limits.max_source_bytes {
            return Err(SfzError::SourceTooLarge {
                path: display.to_string(),
                len: bytes.len(),
                limit: self.limits.max_source_bytes,
            });
        }
        let text = String::from_utf8(bytes).map_err(|_| SfzError::NotUtf8 {
            path: display.to_string(),
        })?;

        state.stack.push(canonical.to_path_buf());
        let result = self.scan_and_expand(&text, display, depth, state);
        state.stack.pop();
        result
    }

    /// 扫描一个文件的文本，按行处理 `#define` 与 `#include`，并把非 include 行切成片段。
    fn scan_and_expand(
        &self,
        text: &str,
        display: &str,
        depth: usize,
        state: &mut ResolveState,
    ) -> Result<(), SfzError> {
        let limits = self.limits;
        let mut segment_start = 0usize;
        let mut segment_first_line = 1usize;
        let mut offset = 0usize;

        for (index, raw_line) in text.split_inclusive('\n').enumerate() {
            let line_no = index + 1;
            if raw_line.len() > limits.max_line_bytes {
                return Err(SfzError::LineTooLong {
                    line: line_no,
                    len: raw_line.len(),
                    limit: limits.max_line_bytes,
                });
            }
            let line = strip_comment(raw_line).trim();
            if is_directive(line) {
                let (directive, argument) = split_directive(line);
                if directive.eq_ignore_ascii_case("include") {
                    // include 行之前的文本先落成片段（保证粘贴顺序）。
                    if offset > segment_start {
                        state.sources.push(SfzSource {
                            path: display.to_string(),
                            text: text[segment_start..offset].to_string(),
                            first_line: segment_first_line,
                        });
                    }
                    self.handle_include(argument, line_no, depth, state)?;
                    segment_start = offset + raw_line.len();
                    segment_first_line = line_no + 1;
                } else if directive.eq_ignore_ascii_case("define") {
                    handle_resolver_define(argument, line_no, limits, state)?;
                }
            }
            offset += raw_line.len();
        }

        if offset > segment_start {
            state.sources.push(SfzSource {
                path: display.to_string(),
                text: text[segment_start..offset].to_string(),
                first_line: segment_first_line,
            });
        }
        Ok(())
    }

    /// 处理一个 `#include "path"`：glob 展开 / 单文件递归。
    fn handle_include(
        &self,
        argument: &str,
        line: usize,
        depth: usize,
        state: &mut ResolveState,
    ) -> Result<(), SfzError> {
        let trimmed = argument.trim();
        let Some(after_quote) = trimmed.strip_prefix('"') else {
            return Err(SfzError::IncludeNotQuoted {
                line,
                text: truncate_for_error(trimmed),
            });
        };
        let Some(end) = after_quote.find('"') else {
            return Err(SfzError::IncludeUnterminated { line });
        };
        let raw_path = &after_quote[..end];
        if raw_path.is_empty() {
            return Err(SfzError::IncludeEmptyPath { line });
        }
        // include 路径里的宏：单遍展开；未定义则原样保留（通常随后变成 NotFound）。
        let macros = &state.macros;
        let expanded = expand_macros(
            raw_path,
            line,
            self.limits,
            |name| macros.get(name).cloned(),
            |_| {},
        )?;
        let relative = self.check_relative(expanded.as_ref(), line)?;

        if relative.contains('*') || relative.contains('?') {
            let matches = self.expand_glob(&relative, line)?;
            if matches.is_empty() {
                return Err(SfzError::IncludeNoMatch {
                    line,
                    pattern: relative,
                });
            }
            for matched in matches {
                let canonical = self.canonical_include(&matched, line)?;
                self.expand_file(&canonical, &matched, depth + 1, state)?;
            }
        } else {
            let canonical = self.canonical_include(&relative, line)?;
            self.expand_file(&canonical, &relative, depth + 1, state)?;
        }
        Ok(())
    }

    /// 从基准目录向下匹配 glob（`*` / `?` / `**`），不跟随符号链接。
    fn expand_glob(&self, pattern: &str, line: usize) -> Result<Vec<String>, SfzError> {
        let segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
        let mut walker = GlobWalker {
            resolver: self,
            pattern,
            line,
            matches: Vec::new(),
            scanned: 0,
        };
        walker.walk(&self.base, "", &segments, 0)?;
        let mut matches = walker.matches;
        // 确定性顺序：绝不依赖文件系统枚举顺序。
        matches.sort();
        matches.dedup();
        Ok(matches)
    }
}

struct ResolveState {
    sources: Vec<SfzSource>,
    macros: BTreeMap<String, String>,
    stack: Vec<PathBuf>,
    files: usize,
}

/// resolver 侧的 `#define`（宏表拥有所有权，避免自引用借用）。
fn handle_resolver_define(
    argument: &str,
    line: usize,
    limits: ParseLimits,
    state: &mut ResolveState,
) -> Result<(), SfzError> {
    let Some(rest) = argument.trim_start().strip_prefix('$') else {
        return Ok(());
    };
    let name_end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let name = &rest[..name_end];
    let value_text = rest[name_end..].trim();
    if name.is_empty() || value_text.is_empty() {
        return Ok(());
    }
    if !state.macros.contains_key(name) && state.macros.len() >= limits.max_defines {
        return Err(SfzError::TooManyDefines {
            limit: limits.max_defines,
        });
    }
    let macros = &state.macros;
    let expanded = expand_macros(
        value_text,
        line,
        limits,
        |needle| macros.get(needle).cloned(),
        |_| {},
    )?;
    state.macros.insert(name.to_string(), expanded.into_owned());
    Ok(())
}

/// glob 遍历器：把「上限检查 + 确定性排序」集中在一处。
struct GlobWalker<'r> {
    resolver: &'r IncludeResolver,
    pattern: &'r str,
    line: usize,
    matches: Vec<String>,
    scanned: usize,
}

impl GlobWalker<'_> {
    fn walk(
        &mut self,
        dir: &Path,
        prefix: &str,
        segments: &[&str],
        depth: usize,
    ) -> Result<(), SfzError> {
        if depth > self.resolver.limits.max_glob_depth {
            return Ok(());
        }
        let Some((segment, rest)) = segments.split_first() else {
            return Ok(());
        };

        if *segment == "**" {
            // `**` 匹配零层或多层目录。
            self.walk(dir, prefix, rest, depth)?;
            for name in self.entries(dir)? {
                let child = dir.join(&name);
                if child.is_dir() && !is_symlink(&child) {
                    let child_prefix = join_prefix(prefix, &name);
                    self.walk(&child, &child_prefix, segments, depth + 1)?;
                }
            }
            return Ok(());
        }

        let is_last = rest.is_empty();
        if segment.contains('*') || segment.contains('?') {
            for name in self.entries(dir)? {
                if !glob_match(segment, &name) {
                    continue;
                }
                let child = dir.join(&name);
                let child_prefix = join_prefix(prefix, &name);
                if is_last {
                    if child.is_file() && has_sfz_extension(&name) {
                        self.push(child_prefix)?;
                    }
                } else if child.is_dir() && !is_symlink(&child) {
                    self.walk(&child, &child_prefix, rest, depth + 1)?;
                }
            }
        } else {
            let child = dir.join(segment);
            let child_prefix = join_prefix(prefix, segment);
            if is_last {
                if child.is_file() && has_sfz_extension(segment) {
                    self.push(child_prefix)?;
                }
            } else if child.is_dir() && !is_symlink(&child) {
                self.walk(&child, &child_prefix, rest, depth + 1)?;
            }
        }
        Ok(())
    }

    /// 列出一个目录下的可见项（跳过点开头），并计入扫描配额。
    fn entries(&mut self, dir: &Path) -> Result<Vec<String>, SfzError> {
        let mut names: Vec<String> = Vec::new();
        let entries = fs::read_dir(dir).map_err(|error| SfzError::Io {
            path: dir.display().to_string(),
            detail: error.to_string(),
        })?;
        for entry in entries {
            self.scanned += 1;
            if self.scanned > self.resolver.limits.max_glob_scanned {
                return Err(SfzError::GlobScanExceeded {
                    pattern: self.pattern.to_string(),
                    limit: self.resolver.limits.max_glob_scanned,
                });
            }
            let Ok(entry) = entry else { continue };
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') {
                continue;
            }
            names.push(name.to_string());
        }
        names.sort();
        Ok(names)
    }

    fn push(&mut self, candidate: String) -> Result<(), SfzError> {
        if self.matches.len() >= self.resolver.limits.max_glob_matches {
            return Err(SfzError::GlobMatchesExceeded {
                pattern: self.pattern.to_string(),
                limit: self.resolver.limits.max_glob_matches,
            });
        }
        let _ = self.line;
        self.matches.push(candidate);
        Ok(())
    }
}

fn join_prefix(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

fn has_sfz_extension(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("sfz") || ext.eq_ignore_ascii_case("sfzh"))
}

/// 单段 glob 匹配：`*` 匹配任意（含空）字符序列，`?` 匹配一个字符。
///
/// 迭代实现（无递归），因此 `*****` 这类输入不会栈溢出。
fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut star_match = 0usize;

    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            star_match = ni;
            pi += 1;
        } else if let Some(star_index) = star {
            pi = star_index + 1;
            star_match += 1;
            ni = star_match;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_note_names_per_ipn() {
        assert_eq!(parse_note("C4"), Some(60));
        assert_eq!(parse_note("c5"), Some(72));
        assert_eq!(parse_note("c-1"), Some(0));
        assert_eq!(parse_note("C#4"), Some(61));
        assert_eq!(parse_note("Db4"), Some(61));
        assert_eq!(parse_note("cs4"), Some(61));
        assert_eq!(parse_note("Bb4"), Some(70));
        assert_eq!(parse_note("-1"), Some(-1));
        assert_eq!(parse_note("127"), Some(127));
        assert_eq!(parse_note(""), None);
        assert_eq!(parse_note("H4"), None);
        assert_eq!(parse_note("C99999999999"), None);
    }

    #[test]
    fn strips_comments_outside_quotes() {
        assert_eq!(strip_comment("tune=-1200 // comment"), "tune=-1200 ");
        assert_eq!(
            strip_comment("#include \"a//b.sfz\""),
            "#include \"a//b.sfz\""
        );
    }

    #[test]
    fn splits_multiple_opcodes_preserving_spaced_values() {
        let line = "sample=My Drums/kick 01.wav key=36 // c";
        let pairs: Vec<(&str, &str)> = (OpcodeIter {
            rest: strip_comment(line),
        })
        .collect();
        assert_eq!(
            pairs,
            vec![("sample", "My Drums/kick 01.wav"), ("key", "36")]
        );
    }

    #[test]
    fn glob_matcher_handles_edges() {
        assert!(glob_match("*.sfz", "kick.sfz"));
        assert!(!glob_match("*.sfz", "kick.wav"));
        assert!(glob_match("k?ck.sfz", "kick.sfz"));
        assert!(glob_match("**", "anything"));
        assert!(glob_match("*", ""));
    }

    #[test]
    fn macro_expansion_is_single_pass_and_capped() {
        let limits = ParseLimits::default();
        let mut table: MacroTable<'_> = MacroTable::new();
        table.insert("A", Cow::Borrowed("36"));
        let expanded = expand_macros(
            "key=$A",
            1,
            limits,
            |name| table.get(name).map(|v| v.as_ref().to_string()),
            |_| {},
        )
        .expect("expansion succeeds");
        assert_eq!(expanded, "key=36");

        let capped = ParseLimits {
            max_macro_expansions_per_line: 1,
            ..ParseLimits::default()
        };
        let error = expand_macros(
            "$A$A",
            7,
            capped,
            |name| table.get(name).map(|v| v.as_ref().to_string()),
            |_| {},
        )
        .expect_err("second substitution must hit the cap");
        assert!(matches!(
            error,
            SfzError::MacroExpansionExceeded { line: 7, .. }
        ));
    }

    #[test]
    fn oversized_line_is_rejected() {
        let limits = ParseLimits {
            max_line_bytes: 8,
            ..ParseLimits::default()
        };
        let error = parse_text("<region>sample=a.wav", &limits).expect_err("line too long");
        assert!(matches!(error, SfzError::LineTooLong { limit: 8, .. }));
    }

    #[test]
    fn absolute_and_escaping_includes_are_rejected() {
        let dir = std::env::temp_dir();
        let resolver = IncludeResolver::new(&dir, ParseLimits::default()).expect("base dir");
        for bad in ["/etc/passwd.sfz", "../../etc/passwd.sfz", "C:\\x.sfz"] {
            let error = resolver.resolve(bad).expect_err("must be rejected");
            assert!(
                matches!(
                    error,
                    SfzError::IncludeAbsolutePath { .. } | SfzError::IncludeEscape { .. }
                ),
                "unexpected verdict for {bad}: {error:?}"
            );
        }
    }
}
