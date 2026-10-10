//! SFZ 解析与声部池的统一错误类型。
//!
//! 规范来源 (Normative): `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` ROAD-M2-005
//! （零拷贝解析器）、`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` ARCH-RT-004（声部池）。
//!
//! **不可信输入边界**：`.sfz` 文件来自磁盘、下载的第三方音色库或用户工程，
//! 全部属于不可信输入。因此本 crate 的解析路径上**绝不 panic**（无 `unwrap` /
//! `expect` / 索引越界 / 整数溢出），一切失败都收敛成 [`SfzError`]。
//! 这条约束是 `MUST-GATE-011`（cargo-fuzz 千万次变异零崩溃）在类型层面的第一道防线。

use thiserror::Error;

/// `yeban-sfz` 的统一错误类型。
///
/// 派生 `PartialEq` 是为了让测试可以直接对错误变体做断言（`f32` 字段使其无法派生 `Eq`）；
/// 携带的文本都是人可读的定位信息，不参与相等性语义的判断。
#[derive(Debug, Clone, Error, PartialEq)]
pub enum SfzError {
    // ------------------------------------------------------------------
    // 来源与编码
    // ------------------------------------------------------------------
    /// 读取 SFZ 源文件失败（路径 + 底层错误文本；不携带 `io::Error` 以保留 `PartialEq`）。
    #[error("cannot read `{path}`: {detail}")]
    Io {
        /// 出错的路径（已做展示用规范化）。
        path: String,
        /// 底层 I/O 错误的可读描述。
        detail: String,
    },

    /// 源文件不是合法 UTF-8。SFZ 规范未定义编码，非 UTF-8 一律显式拒绝而不是猜测。
    #[error("`{path}` is not valid UTF-8")]
    NotUtf8 {
        /// 出错的路径。
        path: String,
    },

    /// 单个源文件超过 [`crate::parser::ParseLimits::max_source_bytes`]。
    #[error("`{path}` is {len} bytes, exceeding the {limit} byte source limit")]
    SourceTooLarge {
        /// 出错的路径。
        path: String,
        /// 实际字节数。
        len: usize,
        /// 上限。
        limit: usize,
    },

    // ------------------------------------------------------------------
    // #include 沙箱（安全红线）
    // ------------------------------------------------------------------
    /// `#include` 的路径没有被双引号包围（规范要求 `#include "path"`）。
    #[error("line {line}: #include path must be double-quoted, got `{text}`")]
    IncludeNotQuoted {
        /// 1-based 行号。
        line: usize,
        /// 该行的原文（截断后）。
        text: String,
    },

    /// `#include` 的引号没有闭合。
    #[error("line {line}: unterminated #include string")]
    IncludeUnterminated {
        /// 1-based 行号。
        line: usize,
    },

    /// `#include` 路径为空。
    #[error("line {line}: #include path is empty")]
    IncludeEmptyPath {
        /// 1-based 行号。
        line: usize,
    },

    /// **安全红线**：`#include` 使用了绝对路径。沙箱只允许基准目录内的相对路径。
    #[error("line {line}: absolute #include path `{path}` is rejected (sandbox)")]
    IncludeAbsolutePath {
        /// 1-based 行号。
        line: usize,
        /// 被拒绝的路径。
        path: String,
    },

    /// **安全红线**：`#include` 通过 `..` 或符号链接逃逸出基准目录。
    #[error("line {line}: #include path `{path}` escapes the base directory (sandbox)")]
    IncludeEscape {
        /// 1-based 行号。
        line: usize,
        /// 被拒绝的路径。
        path: String,
    },

    /// `#include` 路径含 NUL 字节或其它不能进入文件系统的字符。
    #[error("line {line}: #include path `{path}` is not a valid filesystem path")]
    IncludeInvalidPath {
        /// 1-based 行号。
        line: usize,
        /// 被拒绝的路径。
        path: String,
    },

    /// `#include` 指向的文件不存在。
    #[error("line {line}: #include target `{path}` does not exist")]
    IncludeNotFound {
        /// 1-based 行号。
        line: usize,
        /// 缺失的路径。
        path: String,
    },

    /// `#include` 的目标存在但不是普通文件（例如目录）。
    #[error("line {line}: #include target `{path}` is not a regular file")]
    IncludeNotAFile {
        /// 1-based 行号。
        line: usize,
        /// 被拒绝的路径。
        path: String,
    },

    /// `#include` 目标扩展名不是规范允许的 `.sfz` / `.sfzh`。
    #[error("line {line}: #include target `{path}` must end in .sfz or .sfzh")]
    IncludeUnsupportedExtension {
        /// 1-based 行号。
        line: usize,
        /// 被拒绝的路径。
        path: String,
    },

    /// glob 形式的 `#include` 没有匹配到任何文件。
    #[error("line {line}: #include pattern `{pattern}` matched no file")]
    IncludeNoMatch {
        /// 1-based 行号。
        line: usize,
        /// 原始 glob 模式。
        pattern: String,
    },

    /// `#include` 形成环（A include B include A）。
    #[error("include cycle detected at `{path}`")]
    IncludeCycle {
        /// 形成环的路径。
        path: String,
    },

    /// `#include` 嵌套深度超过 [`crate::parser::ParseLimits::max_include_depth`]。
    #[error("#include nesting exceeds the {limit} level limit")]
    IncludeDepthExceeded {
        /// 上限。
        limit: usize,
    },

    /// 展开后的源文件总数超过 [`crate::parser::ParseLimits::max_include_files`]。
    #[error("#include expansion produced more than {limit} files")]
    IncludeCountExceeded {
        /// 上限。
        limit: usize,
    },

    /// glob 匹配数超过 [`crate::parser::ParseLimits::max_glob_matches`]。
    #[error("include pattern `{pattern}` matched more than {limit} files")]
    GlobMatchesExceeded {
        /// 原始 glob 模式。
        pattern: String,
        /// 上限。
        limit: usize,
    },

    /// glob 目录遍历步数超过 [`crate::parser::ParseLimits::max_glob_scanned`]（防遍历炸弹）。
    #[error("include pattern `{pattern}` required scanning more than {limit} directory entries")]
    GlobScanExceeded {
        /// 原始 glob 模式。
        pattern: String,
        /// 上限。
        limit: usize,
    },

    // ------------------------------------------------------------------
    // 结构
    // ------------------------------------------------------------------
    /// 段头 `<...` 没有闭合的 `>`。
    #[error("line {line}: unterminated header in `{text}`")]
    UnterminatedHeader {
        /// 1-based 行号。
        line: usize,
        /// 该行的原文（截断后）。
        text: String,
    },

    /// 段头 `< >` 为空（既不是已知段头也不是可忽略的扩展段头）。
    #[error("line {line}: empty header name")]
    EmptyHeaderName {
        /// 1-based 行号。
        line: usize,
    },

    /// `opcode=value` 的取值无法解析成该 opcode 期望的整数。
    #[error("line {line}: `{opcode}` expects an integer, got `{value}`")]
    InvalidInteger {
        /// 1-based 行号。
        line: usize,
        /// opcode 名。
        opcode: String,
        /// 原始取值。
        value: String,
    },

    /// 整数取值越界。
    #[error("line {line}: `{opcode}` value {value} out of range {min}..={max}")]
    IntegerOutOfRange {
        /// 1-based 行号。
        line: usize,
        /// opcode 名。
        opcode: String,
        /// 实际取值。
        value: i64,
        /// 允许下界（含）。
        min: i64,
        /// 允许上界（含）。
        max: i64,
    },

    /// 浮点取值越界（规范表格给出 Range 的浮点 opcode，拒绝而不静默钳位）。
    ///
    /// 目前只有两个来源：`amp_veltrack`（<https://sfzformat.com/opcodes/amp_veltrack/>，
    /// Range = -100 to 100）与 `amp_velcurve_N`
    /// （<https://sfzformat.com/opcodes/amp_velcurve_N/>，Range = 0 to 1）。
    /// 与 [`SfzError::IntegerOutOfRange`] 的分工：那个用于整数语法（`parse_int`），
    /// 这个用于浮点语法（`as_f32`）。
    #[error("line {line}: `{opcode}` value {value} out of range {min}..={max}")]
    FloatOutOfRange {
        /// 1-based 行号。
        line: usize,
        /// opcode 名。
        opcode: String,
        /// 实际取值。
        value: f32,
        /// 允许下界（含）。
        min: f32,
        /// 允许上界（含）。
        max: f32,
    },

    /// `amp_velcurve_N` 的下标 `N` 不是 `0..=127` 的十进制整数。
    ///
    /// 出处 <https://sfzformat.com/opcodes/amp_velcurve_N/> 正文："N can be from 0 to 127"。
    /// 名字形如 `amp_velcurve_` 后跟**全数字**时按下标处理；数字解析溢出或大于 127 时
    /// 报本错误（不静默丢弃该点，那样会让力度曲线无声地变错）。
    #[error("line {line}: `{opcode}` velocity index `{index}` is not in 0..=127")]
    VelocityCurveIndexOutOfRange {
        /// 1-based 行号。
        line: usize,
        /// opcode 名。
        opcode: String,
        /// 原始下标文本。
        index: String,
    },

    /// `opcode=value` 的取值无法解析成浮点数。
    #[error("line {line}: `{opcode}` expects a number, got `{value}`")]
    InvalidFloat {
        /// 1-based 行号。
        line: usize,
        /// opcode 名。
        opcode: String,
        /// 原始取值。
        value: String,
    },

    /// 浮点取值不是有限值（`NaN` / `±Inf` 一律拒绝：它们无法确定性序列化）。
    #[error("line {line}: `{opcode}` must be finite, got `{value}`")]
    NonFiniteFloat {
        /// 1-based 行号。
        line: usize,
        /// opcode 名。
        opcode: String,
        /// 原始取值。
        value: String,
    },

    /// 时长类浮点 opcode 取到负数（时长的取反没有定义，拒绝而不静默钳位）。
    ///
    /// 目前只有 `off_time`（<https://sfzformat.com/opcodes/off_time/>）：规范表格的
    /// Range 为空、Default 为 0.006 秒，因此本 crate 只额外要求**非负**。
    #[error("line {line}: `{opcode}` must not be negative, got {value}")]
    InvalidDuration {
        /// 1-based 行号。
        line: usize,
        /// opcode 名。
        opcode: String,
        /// 实际取值。
        value: f32,
    },

    /// 音名 / MIDI 号无法解析。音名遵循 sfzformat 的 IPN 记法（`C4` = 60）。
    #[error("line {line}: `{opcode}` expects a note name or MIDI number 0..=127, got `{value}`")]
    InvalidNote {
        /// 1-based 行号。
        line: usize,
        /// opcode 名。
        opcode: String,
        /// 原始取值。
        value: String,
    },

    /// 枚举型 opcode 的取值不在允许集合内。
    #[error("line {line}: `{opcode}` value `{value}` is not one of {allowed}")]
    InvalidOption {
        /// 1-based 行号。
        line: usize,
        /// opcode 名。
        opcode: String,
        /// 原始取值。
        value: String,
        /// 允许的白名单（逗号分隔）。
        allowed: &'static str,
    },

    // ------------------------------------------------------------------
    // 显式上限（防 DoS）
    // ------------------------------------------------------------------
    /// 单行超过 [`crate::parser::ParseLimits::max_line_bytes`]。
    #[error("line {line}: line is {len} bytes, exceeding the {limit} byte line limit")]
    LineTooLong {
        /// 1-based 行号。
        line: usize,
        /// 实际字节数。
        len: usize,
        /// 上限。
        limit: usize,
    },

    /// `<curve>` 段定义了点却没有 `curve_index`：无法知道这条曲线属于哪个编号。
    #[error("line {line}: <curve> defines points but no curve_index")]
    CurveWithoutIndex {
        /// 1-based 行号（`<curve>` 段头所在行）。
        line: usize,
    },

    /// `<curve>` 的 `curve_index` 落在 ARIA 内建曲线 `0..=6` 上；规范明文这些不可覆写。
    #[error(
        "line {line}: curve_index {index} is reserved for an ARIA built-in curve (use 7..=254)"
    )]
    ReservedCurveIndex {
        /// 1-based 行号（`<curve>` 段头所在行）。
        line: usize,
        /// 被拒绝的曲线编号。
        index: u8,
    },

    /// 同一个 `curve_index` 在文件里定义了两次：哪一条生效没有规范依据，不猜。
    #[error("line {line}: curve_index {index} is already defined")]
    DuplicateCurveIndex {
        /// 1-based 行号（重复定义所在的 `<curve>` 段头行）。
        line: usize,
        /// 重复的曲线编号。
        index: u8,
    },

    /// `<region>` 数量超过 [`crate::parser::ParseLimits::max_regions`]。
    #[error("more than {limit} <region> sections")]
    TooManyRegions {
        /// 上限。
        limit: usize,
    },

    /// `<curve>` 数量超过 [`crate::parser::ParseLimits::max_curves`]。
    #[error("more than {limit} <curve> sections")]
    TooManyCurves {
        /// 上限。
        limit: usize,
    },

    /// `<effect>` 数量超过 [`crate::parser::ParseLimits::max_effects`]。
    #[error("more than {limit} <effect> sections")]
    TooManyEffects {
        /// 上限。
        limit: usize,
    },

    /// `<midi>` 数量超过 [`crate::parser::ParseLimits::max_midi_sections`]。
    #[error("more than {limit} <midi> sections")]
    TooManyMidiSections {
        /// 上限。
        limit: usize,
    },

    /// 全文登记的 `<midi>` opcode 总数超过 [`crate::parser::ParseLimits::max_midi_opcodes`]。
    ///
    /// 与 `<curve>` / `<effect>` 不同：`<midi>` 的 opcode 是**原样**登记的，段内条目会
    /// 全部留到解析结束，因此需要一条**总**预算（段数 × 单段条目数会相乘）。
    #[error("more than {limit} <midi> opcodes are registered in total")]
    TooManyMidiOpcodes {
        /// 上限。
        limit: usize,
    },

    /// 单个作用域内的 opcode 数量超过 [`crate::parser::ParseLimits::max_opcodes_per_header`]。
    #[error("`{scope}` scope declares more than {limit} distinct opcodes")]
    TooManyOpcodes {
        /// 作用域名（`<control>` / `<global>` / `<group>` / `<region>`）。
        scope: &'static str,
        /// 上限。
        limit: usize,
    },

    /// `#define` 变量数超过 [`crate::parser::ParseLimits::max_defines`]。
    #[error("more than {limit} #define variables")]
    TooManyDefines {
        /// 上限。
        limit: usize,
    },

    /// 单行宏替换次数超过 [`crate::parser::ParseLimits::max_macro_expansions_per_line`]。
    #[error("line {line}: macro expansion of `{name}` exceeded {limit} substitutions")]
    MacroExpansionExceeded {
        /// 1-based 行号。
        line: usize,
        /// 触发上限时正在替换的宏名。
        name: String,
        /// 上限。
        limit: usize,
    },

    /// 宏替换后的行超过 [`crate::parser::ParseLimits::max_line_bytes`]。
    #[error("line {line}: macro expansion grew the line beyond {limit} bytes")]
    MacroExpansionTooLong {
        /// 1-based 行号。
        line: usize,
        /// 上限。
        limit: usize,
    },

    // ------------------------------------------------------------------
    // 声部池
    // ------------------------------------------------------------------
    /// 请求的声部池容量为 0 或超过 [`crate::voice_pool::MAX_VOICE_CAPACITY`]。
    #[error("voice pool capacity {requested} out of range 1..={max}")]
    InvalidVoiceCapacity {
        /// 请求的容量。
        requested: usize,
        /// 允许的最大容量。
        max: usize,
    },

    /// 用一个无效（已失效 / 不属于本池）的声部句柄调用池操作。
    #[error("voice handle is stale or does not belong to this pool")]
    StaleVoiceHandle,
}

// ---------------------------------------------------------------------------
// 诊断文案（公开面：`Display`）的黄金表
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个 `SfzError` 臂的 `Display` 文案**逐字**钉住。
    ///
    /// 这条判据覆盖 44 个 `thiserror` 臂的全部公开诊断面：文案改动（哪怕是删一个词）
    /// 都会在这里变红。同时断言 `source()` 恒为 `None` —— 本 crate 刻意不携带底层
    /// `io::Error`（见 `Io` 的文档：保留 `PartialEq`），因此错误链只有一层。
    /// 黄金表的唯一来源：每个 `SfzError` 臂一条 `(错误值, 期望 Display 文案)`。
    ///
    /// R48：这张表**必须**与 `sfz_error_arm` 的臂编号一一对应
    /// （由 `the_sfz_error_golden_table_covers_every_arm_number` 守住）。
    fn display_cases() -> Vec<(SfzError, &'static str)> {
        vec![
            (
                SfzError::Io {
                    path: String::from("s"),
                    detail: String::from("s"),
                },
                "cannot read `s`: s",
            ),
            (
                SfzError::NotUtf8 {
                    path: String::from("s"),
                },
                "`s` is not valid UTF-8",
            ),
            (
                SfzError::SourceTooLarge {
                    path: String::from("s"),
                    len: 3,
                    limit: 3,
                },
                "`s` is 3 bytes, exceeding the 3 byte source limit",
            ),
            (
                SfzError::IncludeNotQuoted {
                    line: 3,
                    text: String::from("s"),
                },
                "line 3: #include path must be double-quoted, got `s`",
            ),
            (
                SfzError::IncludeUnterminated { line: 3 },
                "line 3: unterminated #include string",
            ),
            (
                SfzError::IncludeEmptyPath { line: 3 },
                "line 3: #include path is empty",
            ),
            (
                SfzError::IncludeAbsolutePath {
                    line: 3,
                    path: String::from("s"),
                },
                "line 3: absolute #include path `s` is rejected (sandbox)",
            ),
            (
                SfzError::IncludeEscape {
                    line: 3,
                    path: String::from("s"),
                },
                "line 3: #include path `s` escapes the base directory (sandbox)",
            ),
            (
                SfzError::IncludeInvalidPath {
                    line: 3,
                    path: String::from("s"),
                },
                "line 3: #include path `s` is not a valid filesystem path",
            ),
            (
                SfzError::IncludeNotFound {
                    line: 3,
                    path: String::from("s"),
                },
                "line 3: #include target `s` does not exist",
            ),
            (
                SfzError::IncludeNotAFile {
                    line: 3,
                    path: String::from("s"),
                },
                "line 3: #include target `s` is not a regular file",
            ),
            (
                SfzError::IncludeUnsupportedExtension {
                    line: 3,
                    path: String::from("s"),
                },
                "line 3: #include target `s` must end in .sfz or .sfzh",
            ),
            (
                SfzError::IncludeNoMatch {
                    line: 3,
                    pattern: String::from("s"),
                },
                "line 3: #include pattern `s` matched no file",
            ),
            (
                SfzError::IncludeCycle {
                    path: String::from("s"),
                },
                "include cycle detected at `s`",
            ),
            (
                SfzError::IncludeDepthExceeded { limit: 3 },
                "#include nesting exceeds the 3 level limit",
            ),
            (
                SfzError::IncludeCountExceeded { limit: 3 },
                "#include expansion produced more than 3 files",
            ),
            (
                SfzError::GlobMatchesExceeded {
                    pattern: String::from("s"),
                    limit: 3,
                },
                "include pattern `s` matched more than 3 files",
            ),
            (
                SfzError::GlobScanExceeded {
                    pattern: String::from("s"),
                    limit: 3,
                },
                "include pattern `s` required scanning more than 3 directory entries",
            ),
            (
                SfzError::UnterminatedHeader {
                    line: 3,
                    text: String::from("s"),
                },
                "line 3: unterminated header in `s`",
            ),
            (
                SfzError::EmptyHeaderName { line: 3 },
                "line 3: empty header name",
            ),
            (
                SfzError::InvalidInteger {
                    line: 3,
                    opcode: String::from("s"),
                    value: String::from("s"),
                },
                "line 3: `s` expects an integer, got `s`",
            ),
            (
                SfzError::IntegerOutOfRange {
                    line: 3,
                    opcode: String::from("s"),
                    value: -2,
                    min: -2,
                    max: -2,
                },
                "line 3: `s` value -2 out of range -2..=-2",
            ),
            (
                SfzError::FloatOutOfRange {
                    line: 3,
                    opcode: String::from("s"),
                    value: 1.5,
                    min: 1.5,
                    max: 1.5,
                },
                "line 3: `s` value 1.5 out of range 1.5..=1.5",
            ),
            (
                SfzError::VelocityCurveIndexOutOfRange {
                    line: 3,
                    opcode: String::from("s"),
                    index: String::from("s"),
                },
                "line 3: `s` velocity index `s` is not in 0..=127",
            ),
            (
                SfzError::InvalidFloat {
                    line: 3,
                    opcode: String::from("s"),
                    value: String::from("s"),
                },
                "line 3: `s` expects a number, got `s`",
            ),
            (
                SfzError::NonFiniteFloat {
                    line: 3,
                    opcode: String::from("s"),
                    value: String::from("s"),
                },
                "line 3: `s` must be finite, got `s`",
            ),
            (
                SfzError::InvalidDuration {
                    line: 3,
                    opcode: String::from("s"),
                    value: 1.5,
                },
                "line 3: `s` must not be negative, got 1.5",
            ),
            (
                SfzError::InvalidNote {
                    line: 3,
                    opcode: String::from("s"),
                    value: String::from("s"),
                },
                "line 3: `s` expects a note name or MIDI number 0..=127, got `s`",
            ),
            (
                SfzError::InvalidOption {
                    line: 3,
                    opcode: String::from("s"),
                    value: String::from("s"),
                    allowed: "opt",
                },
                "line 3: `s` value `s` is not one of opt",
            ),
            (
                SfzError::LineTooLong {
                    line: 3,
                    len: 3,
                    limit: 3,
                },
                "line 3: line is 3 bytes, exceeding the 3 byte line limit",
            ),
            (
                SfzError::CurveWithoutIndex { line: 3 },
                "line 3: <curve> defines points but no curve_index",
            ),
            (
                SfzError::ReservedCurveIndex { line: 3, index: 5 },
                "line 3: curve_index 5 is reserved for an ARIA built-in curve (use 7..=254)",
            ),
            (
                SfzError::DuplicateCurveIndex { line: 3, index: 5 },
                "line 3: curve_index 5 is already defined",
            ),
            (
                SfzError::TooManyRegions { limit: 3 },
                "more than 3 <region> sections",
            ),
            (
                SfzError::TooManyCurves { limit: 3 },
                "more than 3 <curve> sections",
            ),
            (
                SfzError::TooManyEffects { limit: 3 },
                "more than 3 <effect> sections",
            ),
            (
                SfzError::TooManyMidiSections { limit: 3 },
                "more than 3 <midi> sections",
            ),
            (
                SfzError::TooManyMidiOpcodes { limit: 3 },
                "more than 3 <midi> opcodes are registered in total",
            ),
            (
                SfzError::TooManyOpcodes {
                    scope: "opt",
                    limit: 3,
                },
                "`opt` scope declares more than 3 distinct opcodes",
            ),
            (
                SfzError::TooManyDefines { limit: 3 },
                "more than 3 #define variables",
            ),
            (
                SfzError::MacroExpansionExceeded {
                    line: 3,
                    name: String::from("s"),
                    limit: 3,
                },
                "line 3: macro expansion of `s` exceeded 3 substitutions",
            ),
            (
                SfzError::MacroExpansionTooLong { line: 3, limit: 3 },
                "line 3: macro expansion grew the line beyond 3 bytes",
            ),
            (
                SfzError::InvalidVoiceCapacity {
                    requested: 3,
                    max: 3,
                },
                "voice pool capacity 3 out of range 1..=3",
            ),
            (
                SfzError::StaleVoiceHandle,
                "voice handle is stale or does not belong to this pool",
            ),
        ]
    }

    #[test]
    fn every_sfz_error_display_arm_is_pinned() {
        let cases = display_cases();
        assert_eq!(cases.len(), 44);
        for (error, expected) in cases {
            assert_eq!(format!("{error}"), expected, "Display for {error:?}");
            assert!(
                std::error::Error::source(&error).is_none(),
                "{error:?} must not carry a source (no io::Error is chained)"
            );
        }
    }

    /// R48／R51 穷举探针：返回**臂编号**（`0..N`），**无通配符分支**。
    ///
    /// ⚠️ 本 `match` 必须保持**无通配符**：加一个 `_` 就会让 R48 失效
    ///（第十批 L6 实测「给探针加通配符 ⇒ 所有判据仍全绿」，见裁决 R51）。
    /// 枚举新增变体时这里会**编译失败**（`error[E0004]`）—— 这是黄金表的
    /// `cases.len()` 断言**给不了**的保证（那一行数的是**表**，读不到枚举）。
    fn sfz_error_arm(error: &SfzError) -> u8 {
        match error {
            SfzError::Io { .. } => 0,
            SfzError::NotUtf8 { .. } => 1,
            SfzError::SourceTooLarge { .. } => 2,
            SfzError::IncludeNotQuoted { .. } => 3,
            SfzError::IncludeUnterminated { .. } => 4,
            SfzError::IncludeEmptyPath { .. } => 5,
            SfzError::IncludeAbsolutePath { .. } => 6,
            SfzError::IncludeEscape { .. } => 7,
            SfzError::IncludeInvalidPath { .. } => 8,
            SfzError::IncludeNotFound { .. } => 9,
            SfzError::IncludeNotAFile { .. } => 10,
            SfzError::IncludeUnsupportedExtension { .. } => 11,
            SfzError::IncludeNoMatch { .. } => 12,
            SfzError::IncludeCycle { .. } => 13,
            SfzError::IncludeDepthExceeded { .. } => 14,
            SfzError::IncludeCountExceeded { .. } => 15,
            SfzError::GlobMatchesExceeded { .. } => 16,
            SfzError::GlobScanExceeded { .. } => 17,
            SfzError::UnterminatedHeader { .. } => 18,
            SfzError::EmptyHeaderName { .. } => 19,
            SfzError::InvalidInteger { .. } => 20,
            SfzError::IntegerOutOfRange { .. } => 21,
            SfzError::FloatOutOfRange { .. } => 22,
            SfzError::VelocityCurveIndexOutOfRange { .. } => 23,
            SfzError::InvalidFloat { .. } => 24,
            SfzError::NonFiniteFloat { .. } => 25,
            SfzError::InvalidDuration { .. } => 26,
            SfzError::InvalidNote { .. } => 27,
            SfzError::InvalidOption { .. } => 28,
            SfzError::LineTooLong { .. } => 29,
            SfzError::CurveWithoutIndex { .. } => 30,
            SfzError::ReservedCurveIndex { .. } => 31,
            SfzError::DuplicateCurveIndex { .. } => 32,
            SfzError::TooManyRegions { .. } => 33,
            SfzError::TooManyCurves { .. } => 34,
            SfzError::TooManyEffects { .. } => 35,
            SfzError::TooManyMidiSections { .. } => 36,
            SfzError::TooManyMidiOpcodes { .. } => 37,
            SfzError::TooManyOpcodes { .. } => 38,
            SfzError::TooManyDefines { .. } => 39,
            SfzError::MacroExpansionExceeded { .. } => 40,
            SfzError::MacroExpansionTooLong { .. } => 41,
            SfzError::InvalidVoiceCapacity { .. } => 42,
            SfzError::StaleVoiceHandle => 43,
        }
    }

    #[test]
    fn the_sfz_error_golden_table_covers_every_arm_number() {
        // 把黄金表逐行映射成**臂编号**并断言序列恰为 `0..N`：
        // 表缺一臂 ⇒ 序列变短 ⇒ 红；表里出现重复臂 ⇒ 序列有重复 ⇒ 红。
        // （枚举新增变体由 `sfz_error_arm` 的非穷举 `match` 在**编译期**拦下。）
        let cases = display_cases();
        let arms: Vec<u8> = cases
            .iter()
            .map(|(error, _)| sfz_error_arm(error))
            .collect();
        assert_eq!(arms, (0..cases.len() as u8).collect::<Vec<u8>>());
        // ⭐ R183 判据内红臂：**内部**挖掉一臂 ⇒ 臂序列不再连续 ⇒ 同一个比较**必须拒绝**。
        let mut holed = cases.clone();
        holed.remove(1);
        let holed_arms: Vec<u8> = holed
            .iter()
            .map(|(error, _)| sfz_error_arm(error))
            .collect();
        assert_ne!(
            holed_arms,
            (0..holed_arms.len() as u8).collect::<Vec<u8>>(),
            "red arm: an interior missing arm must be rejected"
        );
        // 登记（如实）：⛔ 去掉**最后一臂**时，连续性检查**检测不到**（序列仍连续）
        // —— 这条由 `the_error_arm_numbers_are_pinned` 之类的臂编号表判据兜住。
        let mut truncated = cases.clone();
        truncated.pop();
        let truncated_arms: Vec<u8> = truncated
            .iter()
            .map(|(error, _)| sfz_error_arm(error))
            .collect();
        assert_eq!(
            truncated_arms,
            (0..truncated_arms.len() as u8).collect::<Vec<u8>>(),
            "documented limitation: dropping the LAST arm is not detectable by contiguity"
        );
    }

    #[test]
    fn sfz_error_equality_discriminates_different_values() {
        // R58：黄金表与 `matches!` 判据依赖 `SfzError` 的相等性／模式匹配；
        // 同一个 `==` 上必须有反向断言（同变体、不同载荷也必须不等）。
        let not_quoted = SfzError::IncludeNotQuoted {
            line: 1,
            text: String::from("a"),
        };
        assert_eq!(not_quoted.clone(), not_quoted);
        assert_ne!(
            not_quoted,
            SfzError::IncludeNotQuoted {
                line: 1,
                text: String::from("b")
            },
            "the payload participates in equality"
        );
        // ⚠️ **判别式探针（假探针）**：只证明「不同变体不等」，**不**证明载荷参与比较
        // （R58：`assert_ne!(Ok(()), Err(_))` 同形）。
        assert_ne!(
            SfzError::IncludeNotQuoted {
                line: 1,
                text: String::from("a")
            },
            SfzError::IncludeUnterminated { line: 1 },
            "different variants differ (discriminant probe, not a payload probe)"
        );
        // ✅ 真探针（**同一变体、不同载荷**）：
        assert_ne!(
            SfzError::TooManyOpcodes {
                scope: "region",
                limit: 1
            },
            SfzError::TooManyOpcodes {
                scope: "region",
                limit: 2
            },
            "the limit payload participates in equality"
        );
        assert_ne!(
            SfzError::LineTooLong {
                line: 1,
                len: 2,
                limit: 3
            },
            SfzError::LineTooLong {
                line: 1,
                len: 2,
                limit: 4
            },
            "the limit participates in equality"
        );
    }
}
