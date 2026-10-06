//! `yeban_export_midi` 的实现 —— **只读**地把活跃工程经 `yeban-midi` 的**共享映射**
//! 投影成 SMF 字节，再以 base64 回传。
//!
//! ## 唯一实现（本模块不写第二份映射 / 编码器）
//!
//! ```text
//! YebanProjectV1
//!   └─ yeban_midi::export::export_from_project   // 共享映射层 (与 app 的 --export-midi 同一份)
//!        └─ MidiExport::to_smf_bytes()           // 共享 SMF 编码器 (midly + 独立 VLQ 核对)
//!             └─ 本模块只做 base64 + JSON 信封
//! ```
//!
//! 判据 `crates/yeban-mcp/tests/extension_tools.rs` 的
//! `export_midi_round_trips_in_memory_through_the_shared_mapping` 断言
//! "工具回传的字节 **逐字节等于** 共享映射直接编出来的字节"，并把字节喂回
//! `yeban_midi::midi::parse_smf` 数音符 —— 因此本模块不可能成为第二份会漂移的实现。
//!
//! ## 为什么不落盘、为什么是 base64
//!
//! `ADR-0001` **D47** 把 MIDI 导出的**落盘出口**定为 app CLI
//! （`yeban-app --export-midi <path>`）。本工具是**只读**的：它把字节作为工具结果回传，
//! 由调用方决定写到哪里 —— 因此 `SideEffect::ReadOnly`，`dryRun` 与真调用在工程状态上等价。
//!
//! base64 是**手写**的（RFC 4648 §4）：本 crate 的规则是"不拖音频栈进 MCP"
//! （见 `crates/yeban-mcp/Cargo.toml`），而为一个 25 行的编码再引一个第三方 crate
//! 既没必要、也会动许可清单。单元判据用 RFC 4648 §10 的**官方测试向量**
//! （`""` / `"f"` / `"fo"` / `"foo"` / `"foob"` / `"fooba"` / `"foobar"`）把它钉住。
//!
//! ## 边界（如实登记）
//!
//! - 响应的 `content` 是**完整** SMF 字节的 base64；大工程会得到一个很大的字符串，
//!   调用方自己要控制上下文占用（本模块不做分页 —— 分页会把 SMF 切成不可独立解析的碎片）。
//! - 映射的全部边界（静音摆放跳过、音频片段不导出、主总线不导出、`micro_timing_ticks`
//!   并入起点、`probability` / `ratchet` 不导出……）由 `yeban-midi` 的模块文档**唯一**登记，
//!   本模块一个字节都不重新解释。

use serde_json::{Value, json};

use yeban_midi::export::{MidiExportError, export_from_project};
use yeban_midi::midi::MidiFormat;
use yeban_model::AssetHash;
use yeban_model::project::YebanProjectV1;

use crate::tools::ErrorCode;

use super::error::Fault;

/// `data.encoding` 的取值（唯一字面量，判据从它派生）。
pub const BASE64_ENCODING: &str = "base64";

/// `data.source` 的取值：**唯一**映射入口的规范名字。
///
/// 把它写进响应而不是留给台账：调用方据此知道"这份字节与 app CLI 的 `--export-midi`
/// 是同一段代码产出的"，而不是两家各自实现。
pub const MAPPING_SOURCE: &str = "yeban_midi::export::export_from_project";

/// `data.encoder` 的取值：**唯一** SMF 编码器的规范名字。
pub const ENCODER_SOURCE: &str = "yeban_midi::midi::MidiExport::to_smf_bytes";

/// RFC 4648 §4 的标准 base64 字母表（`+` / `/`，带 `=` 填充）。
const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 一次 `yeban_export_midi` 的只读产物（`plan` 的产物；`apply` 原样回传）。
#[derive(Debug)]
pub struct MidiExportArtifact {
    /// SMF 字节（`MThd` + conductor `MTrk` + 每条导出轨道一条 `MTrk`）。
    pub bytes: Vec<u8>,
    /// 写进 `MThd` 的时间分度（恒为工程的 960 PPQ，[MODEL-AST-001]）。
    pub ppq: u16,
    /// 写进 `MThd` 的格式（恒为 SMF 1 / [`MidiFormat::Parallel`]）。
    pub format: MidiFormat,
    /// 导出的 `MTrk` 条数（**不含** conductor 轨）。
    pub tracks: usize,
    /// 导出的音符总数。
    pub notes: usize,
    /// tempo map 的事件条数。
    pub tempos: usize,
    /// 字节的 SHA-256（与容器 CAS 的 `AssetHash` 同一口径，调用方可直接做完整性核对）。
    pub sha256: String,
}

impl MidiExportArtifact {
    /// 工具结果的 JSON 载荷。
    ///
    /// `dryRun` 预览与真调用**共用**这一个函数（`mod.rs` 的 `describe` 把它的键并进预览），
    /// 因此"预览说的"与"真做的"逐字段相同 —— 这是 `tests/extension_tools.rs` 的 ② 号口径。
    #[must_use]
    pub fn data(&self) -> Value {
        json!({
            // 读数（与 `yeban_midi::export::MidiExportReport` 同名的三个计数口径一致）。
            "format": format_name(self.format),
            "formatNumber": self.format.number(),
            "ppq": self.ppq,
            "tracks": self.tracks,
            "notes": self.notes,
            "tempos": self.tempos,
            // 载荷：字节长度 + base64 正文 + SHA-256（调用方三样都要，缺一都要再问一次）。
            "bytes": self.bytes.len(),
            "encoding": BASE64_ENCODING,
            "content": base64_encode(&self.bytes),
            "sha256": self.sha256,
            // 事实来源：调用方据此确认这不是第二份实现。
            "source": MAPPING_SOURCE,
            "encoder": ENCODER_SOURCE,
            // 只读：工程状态一位都没改（`mod.rs` 的 `project_after` 对它返回 `None`）。
            "readOnly": true,
        })
    }
}

/// 只读规划：把活跃工程投影成 SMF 字节（**不碰磁盘**、不改任何状态）。
///
/// # Errors
///
/// - `NO_ACTIVE_PROJECT`：由调用方 [`super::require_active`] 判（本函数只收 `&YebanProjectV1`）；
/// - `CLIP_NOT_FOUND`：摆放引用了不存在的片段；
/// - `INVALID_PARAMETER_RANGE`：拍号分母不是 2 的幂 / PPQ 装不进 SMF 头；
/// - `CONFLICT`：工程 PPQ 与编码器默认 PPQ 漂移（拒绝偷偷换算）；
/// - `RENDER_FAILED`：没有任何可导出的 MIDI 内容，或编码器拒绝。
pub fn plan(project: &YebanProjectV1) -> Result<MidiExportArtifact, Fault> {
    let export = export_from_project(project).map_err(|error| map_export_error(&error))?;
    let bytes = export.to_smf_bytes().map_err(|error| {
        Fault::domain(ErrorCode::RenderFailed, format!("SMF 编码被拒绝: {error}"))
    })?;
    Ok(MidiExportArtifact {
        ppq: export.ppq,
        format: export.format,
        tracks: export.tracks.len(),
        notes: export.tracks.iter().map(|track| track.notes.len()).sum(),
        tempos: export.tempos.len(),
        sha256: AssetHash::of_bytes(&bytes).as_str().to_owned(),
        bytes,
    })
}

/// [`MidiExportError`] → 契约错误码的映射（**只用 `ADR-0001` D25 的 20 值联集**，不发明新码）。
///
/// | 映射失败 | 契约码 | 为什么是它 |
/// | :--- | :--- | :--- |
/// | `DanglingClip` | `CLIP_NOT_FOUND` | 摆放引用的片段不存在 —— 语义逐字相同 |
/// | `UnsupportedTimeSignature` / `PpqUnrepresentable` | `INVALID_PARAMETER_RANGE` | 工程参数装不进 SMF 的字段 |
/// | `PpqMismatch` | `CONFLICT` | 两个事实源（模型常量 / 编码器默认）互相矛盾 |
/// | `NoMidiContent` / `Encode` | `RENDER_FAILED` | 导出这个动作没能产出产物 |
fn map_export_error(error: &MidiExportError) -> Fault {
    match error {
        MidiExportError::NoMidiContent => Fault::domain(
            ErrorCode::RenderFailed,
            "工程里没有任何可导出的 MIDI 音符 (没有非主总线轨道含非静音 MIDI 摆放)",
        ),
        MidiExportError::DanglingClip { track, clip } => Fault::domain_with_data(
            ErrorCode::ClipNotFound,
            format!("轨道 {track} 的摆放引用了不存在的片段 {clip} (工程不合法)"),
            json!({
                "track": track.to_canonical_string(),
                "clip": clip.to_canonical_string(),
            }),
        ),
        MidiExportError::UnsupportedTimeSignature { denominator } => Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("拍号分母 {denominator} 不是 2 的幂, SMF 的拍号元事件表达不出来"),
            json!({ "field": "timeSignature.denominator", "denominator": denominator }),
        ),
        MidiExportError::PpqUnrepresentable { ppq } => Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("工程的 PPQ {ppq} 装不进 SMF 的时间分度字段"),
            json!({ "field": "ppq", "ppq": ppq }),
        ),
        MidiExportError::PpqMismatch { project, encoder } => Fault::domain_with_data(
            ErrorCode::Conflict,
            format!(
                "工程 PPQ {project} 与编码器默认 PPQ {encoder} 不一致 —— 拒绝导出 (不许偷偷换算)"
            ),
            json!({ "projectPpq": project, "encoderPpq": encoder }),
        ),
        MidiExportError::Encode(error) => {
            Fault::domain(ErrorCode::RenderFailed, format!("SMF 编码被拒绝: {error}"))
        }
    }
}

/// `MidiFormat` → 规范短名（响应里的 `format` 字段）。
#[must_use]
const fn format_name(format: MidiFormat) -> &'static str {
    match format {
        MidiFormat::SingleTrack => "smf0",
        MidiFormat::Parallel => "smf1",
    }
}

/// RFC 4648 §4 的标准 base64 编码（带 `=` 填充；零第三方依赖）。
///
/// # Panics
///
/// 不 panic：索引上界恒为 63 < 64（掩码 `0x3F`）。
#[must_use]
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = u32::from(chunk[0]);
        let second = chunk.get(1).copied().map_or(0, u32::from);
        let third = chunk.get(2).copied().map_or(0, u32::from);
        let triple = (first << 16) | (second << 8) | third;
        out.push(char::from(
            BASE64_ALPHABET[((triple >> 18) & 0x3F) as usize],
        ));
        out.push(char::from(
            BASE64_ALPHABET[((triple >> 12) & 0x3F) as usize],
        ));
        if chunk.len() > 1 {
            out.push(char::from(BASE64_ALPHABET[((triple >> 6) & 0x3F) as usize]));
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(char::from(BASE64_ALPHABET[(triple & 0x3F) as usize]));
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_midi::midi::parse_smf;
    use yeban_model::samples::filled_project;

    /// RFC 4648 §10 的官方测试向量（**独立**事实源，不是从实现里导出的）。
    const RFC4648_VECTORS: [(&[u8], &str); 7] = [
        (b"", ""),
        (b"f", "Zg=="),
        (b"fo", "Zm8="),
        (b"foo", "Zm9v"),
        (b"foob", "Zm9vYg=="),
        (b"fooba", "Zm9vYmE="),
        (b"foobar", "Zm9vYmFy"),
    ];

    /// 测试侧的独立解码器（与被测编码器分开写；只覆盖标准字母表 + 填充）。
    fn decode(text: &str) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buffer = 0_u32;
        let mut bits = 0_u32;
        for byte in text.bytes() {
            if byte == b'=' {
                break;
            }
            let value = match byte {
                b'A'..=b'Z' => byte - b'A',
                b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                other => panic!("非法的 base64 字符: {other}"),
            };
            buffer = (buffer << 6) | u32::from(value);
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push(((buffer >> bits) & 0xFF) as u8);
            }
        }
        out
    }

    #[test]
    fn base64_matches_the_rfc4648_test_vectors() {
        for (bytes, expected) in RFC4648_VECTORS {
            assert_eq!(base64_encode(bytes), expected, "编码 {bytes:?}");
            assert_eq!(decode(expected), bytes, "解码 {expected}");
        }
    }

    #[test]
    fn the_artifact_counts_match_the_parsed_smf() {
        let artifact = plan(&filled_project()).expect("filled 样本必须可导出");
        let parsed = parse_smf(&artifact.bytes).expect("回读");
        assert_eq!(parsed.notes.len(), artifact.notes, "音符数");
        assert_eq!(parsed.tempos.len(), artifact.tempos, "tempo 事件数");
        assert_eq!(parsed.ppq, artifact.ppq, "PPQ");
        assert_eq!(parsed.format, artifact.format, "格式");
        assert_eq!(
            artifact.sha256,
            AssetHash::of_bytes(&artifact.bytes).as_str()
        );
        assert_eq!(
            artifact.bytes.len(),
            artifact.data()["bytes"].as_u64().expect("bytes") as usize
        );
    }

    #[test]
    fn an_empty_project_is_refused_with_a_contract_code() {
        let error = plan(&YebanProjectV1::default()).expect_err("空工程没有可导出的内容");
        assert_eq!(error.domain_code(), Some(ErrorCode::RenderFailed));
    }
}
