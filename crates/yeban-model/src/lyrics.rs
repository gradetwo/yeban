//! 歌词轨：工程里的歌词时间轴与 LRC 编解码。
//!
//! ## 它补的是哪一格
//!
//! `synthesize_singing` 的 score 早就按音符带词（`{lyric, midi, start_s, dur_s}`），
//! `MidiNote` 也有 `syllable`（单个音节）与 `phonemes`，但工程里**没有歌词时间轴**：
//! 词只活在某次 SVS 请求的 JSON 里，工程存不下“一行词在几时唱”，更导不出 LRC。
//! Wang 2026-11-10 实测：切片双版那两份 LRC（`winter.lrc` / `winter_metal.lrc`）
//! 是比对 ASR 音频 + 编曲事件硬算出来的。
//!
//! 本模块给工程补一条时间轴：[`YebanProjectV1::lyrics`](crate::project::YebanProjectV1::lyrics)
//! （`Vec<LyricLine>`，一行 = 一个 LRC 行：起始 tick + 整行文本），与 LRC 的双向编解码。
//! 导出格式与那两份参照 LRC 同一口径：`[ti/ar/by/length/offset]` 头 + `[mm:ss.cc]行`。
//!
//! ## 口径（与两份参照 LRC 逐项对过）
//!
//! * 行级时间戳：`[mm:ss.cc]`，厘秒（2 位小数），四舍五入（3.6363 秒 ⇒ `03.64`）；
//! * 头标签：`ti` 取工程标题（恒有），`ar` 取工程作者（空则不写），`by` 恒为 `Yeban`，
//!   `length` 只有调用方给了总时长才写（CLI 不渲染、不知道总时长 ⇒ 不写，⛔ 不编造），
//!   `offset` 恒为 `0`；工程里**没有专辑字段** ⇒ `al` 不写；
//! * 行文本原样写出，只把 `\r` / `\n` 换成空格（换行符会劈开 LRC 行结构）；
//! * 导入：`[offset:毫秒]` 会移全部时间戳；一行可带多个时间戳（同一行词在多处唱）；
//!   未知 `[key:value]` 头标签与 `[chorus]` 这类裸标签一律忽略；
//!   没有时间戳的非空行、格式不对的时间戳 ⇒ 响亮失败（⛔ 不静默丢行）。
//!
//! ## 与 `MidiNote::syllable` 的关系
//!
//! `syllable` 是**音符级**的单个音节（SVS 对齐输入）；本时间轴是**行级**的整行词
//! （LRC 的单位）。两者不互相推导：行边界（哪几个音节成一行）只存在于时间轴里，
//! 音符的音节拼不出行 —— 因此时间轴必须**另存**，不能从音符派生。

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::error::ModelError;
use crate::ids::PPQ;
use crate::project::YebanProjectV1;
use crate::time_math::TempoMap;

/// 歌词时间轴的一行：一个 LRC 行。
///
/// `start_tick` 是 960 PPQ 工程时钟上的起始 tick（与 `MidiNote::start_tick` 同一时钟）；
/// `text` 是整行词（如 `"昨日 你 信誓旦旦"`，音节间空格原样保留）。
/// 行按 `start_tick` 严格升序排列（见 [`validate_lyrics`]）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LyricLine {
    /// 起始 tick (960 PPQ 整数时钟，绝不用浮点)。
    pub start_tick: u64,
    /// 整行词；空串非法（见 [`validate_lyrics`]）。
    pub text: String,
}

/// 校验一条歌词时间轴（结构不变量，与 `MidiNote::validate` 同一条纪律）。
///
/// * 行文本非空（空行在 LRC 里没有落点，是退化数据）；
///
/// * `start_tick` 严格升序（同一 tick 两行词在 LRC 里是同一时间戳的两行，
///
///   播放器只会显示其中一行 —— 与 `pitch_bend_curve` 的 tick 严格升序同一条理由）。
///
/// # Errors
///
/// 空文本 ⇒ [`ModelError::LyricTextEmpty`]；tick 不升序 ⇒
/// [`ModelError::LyricStartNotAscending`]。
pub fn validate_lyrics(lines: &[LyricLine]) -> Result<(), ModelError> {
    let mut previous: Option<u64> = None;
    for line in lines {
        if line.text.is_empty() {
            return Err(ModelError::LyricTextEmpty {
                start_tick: line.start_tick,
            });
        }
        if let Some(previous) = previous
            && line.start_tick <= previous
        {
            return Err(ModelError::LyricStartNotAscending {
                previous,
                start_tick: line.start_tick,
            });
        }
        previous = Some(line.start_tick);
    }
    Ok(())
}

/// 秒数 ⇒ LRC 时间戳 `mm:ss.cc`（厘秒四舍五入）。
///
/// 非法输入（非有限、负数、大到放不进 `u64` 厘秒）⇒ `None`（⛔ 不发明一个 `00:00.00`）。
fn format_lrc_timestamp(seconds: f64) -> Option<String> {
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    let total_centis = (seconds * 100.0).round();
    if total_centis > u64::MAX as f64 {
        return None;
    }
    let total_centis = total_centis as u64;
    Some(format!(
        "{:02}:{:02}.{:02}",
        total_centis / 6000,
        (total_centis % 6000) / 100,
        total_centis % 100
    ))
}

/// 头标签与行文本里的换行符会劈开 LRC 行结构 ⇒ 换成空格（其余字符原样保留）。
fn sanitize_lrc_text(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

/// 工程的歌词时间轴 ⇒ LRC 文本。
///
/// * `total_seconds`：总时长（秒），只有调用方**真的知道**才给 —— 给了就写
///   `[length:mm:ss.cc]`，不给就不写（⛔ 不从最后一行词推一个假长度：参照 LRC 的
///   长度比最后一行晚 3.63 秒，那是音频尾巴，不是词的结束）；
/// * tick ⇒ 秒走工程自己的变速映射（[`YebanProjectV1::tempo`]），与渲染同一条换算。
///
/// # Errors
///
/// `total_seconds` 非有限或为负 ⇒ [`ModelError::NonFiniteValue`]；某行 tick
/// 换算不出秒（病态 BPM —— 已校验的工程不会走到这里）⇒
/// [`ModelError::LyricTimeConversionFailed`]。
pub fn lyrics_to_lrc(
    project: &YebanProjectV1,
    total_seconds: Option<f64>,
) -> Result<String, ModelError> {
    if let Some(seconds) = total_seconds
        && (!seconds.is_finite() || seconds < 0.0)
    {
        return Err(ModelError::NonFiniteValue {
            field: "lyrics.total_seconds",
            value: seconds,
        });
    }
    let tempo = project.tempo();
    let sample_rate = project.audio_config.sample_rate.hz();
    let mut out = String::new();
    out.push_str(&format!("[ti:{}]\n", sanitize_lrc_text(&project.title)));
    if !project.author.is_empty() {
        out.push_str(&format!("[ar:{}]\n", sanitize_lrc_text(&project.author)));
    }
    out.push_str("[by:Yeban]\n");
    if let Some(seconds) = total_seconds {
        let stamp = format_lrc_timestamp(seconds)
            .ok_or(ModelError::LyricTimeConversionFailed { tick: 0 })?;
        out.push_str(&format!("[length:{stamp}]\n"));
    }
    out.push_str("[offset:0]\n");
    out.push('\n');
    for line in &project.lyrics {
        let frames = tempo
            .frames_at_tick(line.start_tick, PPQ, sample_rate)
            .ok_or(ModelError::LyricTimeConversionFailed {
                tick: line.start_tick,
            })?;
        #[allow(clippy::cast_precision_loss)]
        let seconds = frames as f64 / sample_rate as f64;
        let stamp = format_lrc_timestamp(seconds).ok_or(ModelError::LyricTimeConversionFailed {
            tick: line.start_tick,
        })?;
        out.push_str(&format!("[{stamp}]{}\n", sanitize_lrc_text(&line.text)));
    }
    Ok(out)
}

/// LRC 解析失败（输入文件属于不可信输入，一律返回 `Result`，绝不 panic）。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LyricParseError {
    /// 时间戳标签格式不对（应为 `[mm:ss.cc]`，秒 < 60、厘秒恰好 2 位）。
    #[error("第 {line} 行时间戳格式不对（应为 [mm:ss.cc]），原文: {text}")]
    BadTimestamp {
        /// 行号（1 起）。
        line: usize,
        /// 该行原文。
        text: String,
    },
    /// 非空行没有任何时间戳标签（词没有落点，放不进时间轴）。
    #[error("第 {line} 行没有时间戳，歌词行必须带时间: {text}")]
    LineWithoutTimestamp {
        /// 行号（1 起）。
        line: usize,
        /// 该行原文。
        text: String,
    },
    /// `[offset:毫秒]` 把某行时间戳移成了负数。
    #[error("第 {line} 行时间戳被 [offset:{offset_ms}] 移成负数")]
    NegativeTimestamp {
        /// 行号（1 起）。
        line: usize,
        /// 该文件声明的 offset（毫秒）。
        offset_ms: i64,
    },
    /// 秒数换算不成 tick（病态 BPM / 采样率，或时间大到溢出）。
    #[error("第 {line} 行秒数换算成 tick 失败")]
    TimeConversionFailed {
        /// 行号（1 起）。
        line: usize,
    },
}

/// `[mm:ss.cc]` ⇒ 秒数（严格口径：分任意位数字、秒恰好 2 位且 < 60、厘秒恰好 2 位）。
///
/// 不合口径 ⇒ `None`（调用方再按“像时间戳的尝试”判响亮失败，还是按头标签忽略）。
fn parse_lrc_timestamp(tag: &str) -> Option<f64> {
    let (minutes, rest) = tag.split_once(':')?;
    if minutes.is_empty() || !minutes.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let (seconds, centis) = rest.split_once('.')?;
    if seconds.len() != 2
        || centis.len() != 2
        || !seconds.bytes().all(|byte| byte.is_ascii_digit())
        || !centis.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let minutes: f64 = minutes.parse().ok()?;
    let seconds: f64 = seconds.parse().ok()?;
    let centis: f64 = centis.parse().ok()?;
    if seconds >= 60.0 {
        return None;
    }
    let total = minutes * 60.0 + seconds + centis / 100.0;
    total.is_finite().then_some(total)
}

/// LRC 文本 ⇒ 歌词时间轴（[`lyrics_to_lrc`] 的逆方向）。
///
/// * 空行跳过；只有头标签的行跳过（`ti`/`ar`/`al`/`by`/`length` 与未知标签都不产生词行）；
///
/// * `[offset:毫秒]`（可负）移**全部**时间戳，移成负数 ⇒ 响亮失败；
///
/// * 一行多个时间戳 ⇒ 同一行词在多处唱（每戳一行）；
///
/// * 只有时间戳、没有词的行不产生条目（空行没有信息）；
///
/// * 返回按 `start_tick` 升序排好的行（稳定排序：同一 tick 的多行保持文件顺序）。
///
/// # Errors
///
/// 格式不对的时间戳 ⇒ [`LyricParseError::BadTimestamp`]；非空行无时间戳 ⇒
/// [`LyricParseError::LineWithoutTimestamp`]；offset 移成负数 ⇒
/// [`LyricParseError::NegativeTimestamp`]；秒数换算不成 tick ⇒
/// [`LyricParseError::TimeConversionFailed`]。
pub fn parse_lrc(
    text: &str,
    tempo: &TempoMap,
    sample_rate: u32,
) -> Result<Vec<LyricLine>, LyricParseError> {
    let mut offset_ms: i64 = 0;
    let mut lines: Vec<LyricLine> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        let mut rest = raw.trim();
        if rest.is_empty() {
            continue;
        }
        let mut stamps: Vec<f64> = Vec::new();
        let mut saw_tag = false;
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(close) = stripped.find(']') else {
                return Err(LyricParseError::BadTimestamp {
                    line: line_no,
                    text: raw.to_owned(),
                });
            };
            let tag = &stripped[..close];
            saw_tag = true;
            rest = stripped[close + 1..].trim_start();
            if let Some(seconds) = parse_lrc_timestamp(tag) {
                stamps.push(seconds);
            } else if tag.bytes().next().is_some_and(|byte| byte.is_ascii_digit()) {
                // 像时间戳的尝试（数字开头）却不合口径 ⇒ 响亮失败，⛔ 不当成未知标签忽略。
                return Err(LyricParseError::BadTimestamp {
                    line: line_no,
                    text: raw.to_owned(),
                });
            } else if let Some((key, value)) = tag.split_once(':')
                && key == "offset"
            {
                offset_ms =
                    value
                        .trim()
                        .parse::<i64>()
                        .map_err(|_| LyricParseError::BadTimestamp {
                            line: line_no,
                            text: raw.to_owned(),
                        })?;
            }
            // `[key:value]` 的其它头标签（`ti`/`ar`/`al`/`by`/`length` 与未知标签）与
            // 无冒号的裸标签（`[chorus]` 这类段落标记）一律忽略。
        }
        if !saw_tag {
            return Err(LyricParseError::LineWithoutTimestamp {
                line: line_no,
                text: raw.to_owned(),
            });
        }
        if stamps.is_empty() {
            continue;
        }
        let lyric = rest.trim();
        if lyric.is_empty() {
            continue;
        }
        for seconds in stamps {
            #[allow(clippy::cast_precision_loss)]
            let shifted = seconds + offset_ms as f64 / 1000.0;
            if shifted < 0.0 {
                return Err(LyricParseError::NegativeTimestamp {
                    line: line_no,
                    offset_ms,
                });
            }
            #[allow(clippy::cast_precision_loss)]
            let frames = (shifted * sample_rate as f64).round();
            if !frames.is_finite() || frames < 0.0 || frames > u64::MAX as f64 {
                return Err(LyricParseError::TimeConversionFailed { line: line_no });
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let tick = tempo
                .ticks_at_frame(frames as u64, PPQ, sample_rate)
                .ok_or(LyricParseError::TimeConversionFailed { line: line_no })?;
            lines.push(LyricLine {
                start_tick: tick,
                text: lyric.to_owned(),
            });
        }
    }
    lines.sort_by_key(|line| line.start_tick);
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_rounds_to_centiseconds() {
        // 参照 LRC 的实测读数：3.6363 秒写成 03.64，7.2727 秒写成 07.27。
        assert_eq!(
            format_lrc_timestamp(3.636_363_6),
            Some("00:03.64".to_owned())
        );
        assert_eq!(format_lrc_timestamp(0.0), Some("00:00.00".to_owned()));
        assert_eq!(format_lrc_timestamp(3.2), Some("00:03.20".to_owned()));
        assert_eq!(
            format_lrc_timestamp(14.545_454),
            Some("00:14.55".to_owned())
        );
        assert_eq!(format_lrc_timestamp(203.63), Some("03:23.63".to_owned()));
        assert_eq!(format_lrc_timestamp(-1.0), None);
        assert_eq!(format_lrc_timestamp(f64::NAN), None);
        assert_eq!(format_lrc_timestamp(f64::INFINITY), None);
    }

    #[test]
    fn timestamp_parser_is_strict() {
        assert_eq!(parse_lrc_timestamp("00:03.64"), Some(3.64));
        assert_eq!(parse_lrc_timestamp("03:23.63"), Some(203.63));
        assert_eq!(parse_lrc_timestamp("00:60.00"), None);
        assert_eq!(parse_lrc_timestamp("00:03.6"), None);
        assert_eq!(parse_lrc_timestamp("00:03.645"), None);
        assert_eq!(parse_lrc_timestamp("0:03.64"), Some(3.64));
        assert_eq!(parse_lrc_timestamp("ti:xxx"), None);
        assert_eq!(parse_lrc_timestamp(""), None);
    }
}
