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
