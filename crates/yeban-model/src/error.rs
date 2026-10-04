//! 数据模型错误类型。
//!
//! 所有校验失败一律返回 `Result<_, ModelError>`，绝不在解析路径上 panic：
//! 工程文件来自磁盘、MCP 代理或第三方归档，全部属于不可信输入。

use thiserror::Error;

/// `yeban-model` 的统一错误类型。
///
/// 只派生 `PartialEq` 而不派生 `Eq`：`ProbabilityOutOfRange` 携带 `f32`，而 `f32`
/// 不满足 `Eq`。这里保留 `PartialEq` 是为了让测试可以直接对错误做断言。
#[derive(Debug, Error, PartialEq)]
pub enum ModelError {
    /// MIDI 音高越界 (允许 0..=127)。
    #[error("pitch {value} out of range 0..=127")]
    PitchOutOfRange {
        /// 实际收到的音高值。
        value: u16,
    },

    /// 力度越界 (允许 0..=127)。
    #[error("velocity {value} out of range 0..=127")]
    VelocityOutOfRange {
        /// 实际收到的力度值。
        value: u16,
    },

    /// 概率字段越界 (允许 0.0..=1.0，且必须有限)。
    #[error("probability {value} out of range 0.0..=1.0")]
    ProbabilityOutOfRange {
        /// 实际收到的概率值。
        value: f32,
    },

    /// ratchet 连击数越界 (允许 1..=16)。
    #[error("ratchet {value} out of range 1..=16")]
    RatchetOutOfRange {
        /// 实际收到的连击数。
        value: u8,
    },

    /// 微时值偏移越界 (允许 -240..=240 tick，即 ±1/16 音符 @960 PPQ)。
    #[error("micro timing {value} ticks out of range -240..=240")]
    MicroTimingOutOfRange {
        /// 实际收到的 tick 偏移。
        value: i32,
    },

    /// 时长为零：音符必须有正时长。
    #[error("duration must be non-zero")]
    ZeroDuration,

    /// 哈希字符串不是 64 位小写十六进制 SHA-256。
    #[error("`{value}` is not a 64-char lowercase hex SHA-256 digest")]
    InvalidHash {
        /// 实际收到的字符串。
        value: String,
    },

    /// `EntityId` 文本不是合法的 26 字符 Crockford Base32 ULID。
    #[error("`{value}` is not a valid 26-char Crockford Base32 ULID")]
    InvalidEntityId {
        /// 实际收到的字符串。
        value: String,
    },

    /// 文档 schema 版本高于本读取器支持的上限，必须拒绝而不是猜测。
    #[error("document schema_version {found} is newer than supported {supported}")]
    SchemaVersionTooNew {
        /// 文档声明的版本。
        found: u32,
        /// 本读取器支持的最大版本。
        supported: u32,
    },

    /// 文档 schema 版本低于 `min_reader_version`，读取器太旧。
    #[error("document requires reader >= {required}, this reader is {actual}")]
    ReaderTooOld {
        /// 文档要求的读取器版本。
        required: u32,
        /// 本读取器版本。
        actual: u32,
    },
}
